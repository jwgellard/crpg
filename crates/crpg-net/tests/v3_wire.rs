//! T140a protocol v3 acceptance: version gate, intents, native tags 0..=10,
//! codes 21..=30, scalar ops 14..=20 (ADR-0029 N1–N4, `tasks/T140.md`).
//!
//! Public API only. Every v3 expectation is a hand-written literal byte
//! oracle built from the contract's layout (postcard primitives: raw `u8`
//! and `bool`, LEB128 varints, length-prefixed text), never from the codec
//! under test; the v2 equivalence checks compare against the frozen v2 codec.
//! Every rejection has a passing control beside it.

use crpg_core::Ulid;
use crpg_net::codec::{
    decode_delta as decode_delta_v1, decode_intent as decode_intent_v1,
    encode_intent as encode_intent_v1, CodecError,
};
use crpg_net::codec_v2::{
    decode_delta as decode_delta_v2, decode_intent as decode_intent_v2,
    encode_delta as encode_delta_v2, encode_intent as encode_intent_v2,
};
use crpg_net::codec_v3::{decode_delta, decode_intent, encode_delta, encode_intent};
use crpg_net::protocol::{
    DeltaOp as DeltaOpV1, IntentBody as IntentBodyV1, IntentFrame as IntentFrameV1,
    ReceiptStatus as ReceiptStatusV1, RejectionCode as RejectionCodeV1, LANE_COMBAT,
    MAX_DELTA_FRAME_BYTES, MAX_INTENT_FRAME_BYTES, SEQ_LAST,
};
use crpg_net::protocol_v2::{DeltaFrame as DeltaFrameV2, DeltaOp as DeltaOpV2};
use crpg_net::protocol_v3::{
    AutoPauseToggles, AutoPauseTrigger, DeltaFrame, DeltaOp, EncounterMode, HostFacts, IntentBody,
    IntentFrame, NetId, PauseReason, PauseRights, ReceiptStatus, RejectionCode,
    AUTO_PAUSE_TOGGLE_MASK, DELTA_TAG_CLOCK, DELTA_TAG_ENCOUNTER, DELTA_TAG_ENTITY_FACTS,
    DELTA_TAG_HOST_FACTS, DELTA_TAG_JOINED, DELTA_TAG_LEFT, DELTA_TAG_MODE_CHANGED,
    DELTA_TAG_ORDER_ENDED, DELTA_TAG_ROUND_STARTED, DELTA_TAG_TIMELINE, INTENT_TAG_DECLARE_ACTION,
    INTENT_TAG_END_TURN, INTENT_TAG_PAUSE, INTENT_TAG_REQUEST_MODE, INTENT_TAG_STOP,
    INTENT_TAG_UNPAUSE, POLICY_CLOCK_HEARTBEAT_TICKS, PROTOCOL_VERSION,
};

// ---------------------------------------------------------------------------
// Fixtures and byte helpers.
// ---------------------------------------------------------------------------

const Z16: [u8; 16] = [0; 16];

fn net(raw: u64) -> NetId {
    NetId::new(raw).expect("fixture id is nonzero")
}

/// `U(n)` of the contract.
fn u(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

/// Wire text of `U(n)` for `n` in 1..=9: `1A`, then 25 x `30`, then the digit.
fn ulid_wire(digit: u8) -> Vec<u8> {
    let mut out = vec![0x1A];
    out.extend(std::iter::repeat_n(0x30, 25));
    out.push(0x30 + digit);
    out
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

/// LEB128 varint, written independently of postcard.
fn varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

fn intent(body: IntentBody) -> IntentFrame {
    IntentFrame {
        epoch: Z16,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 7,
        actor: net(1),
        body,
    }
}

/// `03 Z16 lane seq tick actor` followed by `tail`, with the defaults
/// `lane 0, seq 1, observed_tick 7, actor 1`.
fn intent_bytes(tail: &[u8]) -> Vec<u8> {
    cat(&[&[3], &Z16, &[0, 1, 7, 1], tail])
}

fn delta(ops: Vec<DeltaOp>) -> DeltaFrame {
    DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 100,
        event_seq: 1,
        ops,
    }
}

/// `03 00 64 01 <count>` followed by `ops`.
fn delta_bytes(count: u8, ops: &[u8]) -> Vec<u8> {
    cat(&[&[3, 0, 0x64, 1, count], ops])
}

fn one_op(op: DeltaOp) -> Vec<u8> {
    encode_delta(&delta(vec![op])).expect("fixture op encodes")
}

fn receipt(seq: u64, status: ReceiptStatus) -> DeltaOp {
    DeltaOp::Receipt {
        epoch: Z16,
        lane: 0,
        seq,
        processed_tick: 100,
        status,
    }
}

fn round_trip_op(op: DeltaOp) {
    let bytes = one_op(op.clone());
    assert_eq!(bytes, one_op(op.clone()), "encode is deterministic");
    let decoded = decode_delta(&bytes).expect("round trip decodes");
    assert_eq!(decoded, delta(vec![op]));
    assert_eq!(encode_delta(&decoded).expect("re-encodes"), bytes);
}

fn round_trip_intent(frame: IntentFrame) {
    let bytes = encode_intent(&frame).expect("fixture intent encodes");
    assert_eq!(bytes, encode_intent(&frame).expect("again"));
    let decoded = decode_intent(&bytes).expect("round trip decodes");
    assert_eq!(decoded, frame);
    assert_eq!(encode_intent(&decoded).expect("re-encodes"), bytes);
}

fn action_resolved(outcome: &str) -> DeltaOp {
    DeltaOp::ActionResolved {
        actor: net(1),
        target: net(2),
        ability: u(1),
        outcome: outcome.to_owned(),
        damage: 7,
    }
}

#[test]
fn fixture_ulid_text_matches_the_contract() {
    assert_eq!(u(1).to_string(), format!("{}1", "0".repeat(25)));
    assert_eq!(u(2).to_string(), format!("{}2", "0".repeat(25)));
    assert_eq!(u(3).to_string(), format!("{}3", "0".repeat(25)));
    assert_eq!(ulid_wire(1)[0], 26);
}

// ---------------------------------------------------------------------------
// A1.
// ---------------------------------------------------------------------------

#[test]
fn a1_v3_version_gate() {
    // EndTurn: 22 bytes, equal to v1 with byte 0 changed 1 -> 3.
    let end_turn = intent(IntentBody::EndTurn);
    let v3 = encode_intent(&end_turn).unwrap();
    assert_eq!(v3, cat(&[&[3], &Z16, &[0, 1, 7, 1, 1]]));
    assert_eq!(v3.len(), 22);
    let v1_frame = IntentFrameV1 {
        epoch: Z16,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 7,
        actor: net(1),
        body: IntentBodyV1::EndTurn,
    };
    let v1 = encode_intent_v1(&v1_frame).unwrap();
    let v2 = encode_intent_v2(&v1_frame).unwrap();
    assert_eq!((v1[0], v2[0]), (1, 2));
    assert_eq!(&v1[1..], &v3[1..]);
    assert_eq!(&v2[1..], &v3[1..]);

    // v3 refuses every other version byte.
    assert_eq!(decode_intent(&v1), Err(CodecError::UnsupportedVersion));
    assert_eq!(decode_intent(&v2), Err(CodecError::UnsupportedVersion));
    assert_eq!(decode_intent(&[0x81]), Err(CodecError::UnsupportedVersion));
    assert_eq!(decode_intent(&[0x91]), Err(CodecError::UnsupportedVersion));
    assert_eq!(decode_intent(&[]), Err(CodecError::Malformed));
    // Control: its own bytes decode.
    assert_eq!(decode_intent(&v3), Ok(end_turn));

    // v1 and v2 refuse v3 bytes.
    assert_eq!(decode_intent_v1(&v3), Err(CodecError::UnsupportedVersion));
    assert_eq!(decode_intent_v2(&v3), Err(CodecError::UnsupportedVersion));
    let empty_v3 = [3u8, 0, 0x64, 1, 0];
    assert_eq!(
        decode_delta_v1(&empty_v3),
        Err(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        decode_delta_v2(&empty_v3),
        Err(CodecError::UnsupportedVersion)
    );

    // Delta side of the gate.
    for first in [1u8, 2, 0x81, 0x91] {
        let mut bytes = empty_v3.to_vec();
        bytes[0] = first;
        assert_eq!(decode_delta(&bytes), Err(CodecError::UnsupportedVersion));
    }
    assert_eq!(decode_delta(&[]), Err(CodecError::Malformed));
    assert_eq!(decode_delta(&empty_v3).unwrap(), delta(vec![]));

    // The frame cap runs before the version check.
    let mut big_intent = vec![2u8];
    big_intent.resize(MAX_INTENT_FRAME_BYTES + 1, 0);
    assert_eq!(decode_intent(&big_intent), Err(CodecError::FrameTooLarge));
    let mut big_delta = vec![2u8];
    big_delta.resize(MAX_DELTA_FRAME_BYTES + 1, 0);
    assert_eq!(decode_delta(&big_delta), Err(CodecError::FrameTooLarge));
    // Control: exactly at the cap the version byte decides.
    let mut at_cap_intent = vec![2u8];
    at_cap_intent.resize(MAX_INTENT_FRAME_BYTES, 0);
    assert_eq!(
        decode_intent(&at_cap_intent),
        Err(CodecError::UnsupportedVersion)
    );
    let mut at_cap_delta = vec![2u8];
    at_cap_delta.resize(MAX_DELTA_FRAME_BYTES, 0);
    assert_eq!(
        decode_delta(&at_cap_delta),
        Err(CodecError::UnsupportedVersion)
    );
    // A valid-looking 4,096 / 65,536-byte v3 frame padded with zeros is
    // Malformed (trailing bytes), proving the cap is not the refusal.
    let mut padded_intent = v3.clone();
    padded_intent.resize(MAX_INTENT_FRAME_BYTES, 0);
    assert_eq!(decode_intent(&padded_intent), Err(CodecError::Malformed));
    let mut padded_delta = empty_v3.to_vec();
    padded_delta.resize(MAX_DELTA_FRAME_BYTES, 0);
    assert_eq!(decode_delta(&padded_delta), Err(CodecError::Malformed));
    // One byte over, with version 3, is FrameTooLarge.
    let mut over_intent = v3.clone();
    over_intent.resize(MAX_INTENT_FRAME_BYTES + 1, 0);
    assert_eq!(decode_intent(&over_intent), Err(CodecError::FrameTooLarge));
    let mut over_delta = empty_v3.to_vec();
    over_delta.resize(MAX_DELTA_FRAME_BYTES + 1, 0);
    assert_eq!(decode_delta(&over_delta), Err(CodecError::FrameTooLarge));
}

// ---------------------------------------------------------------------------
// A2.
// ---------------------------------------------------------------------------

#[test]
fn a2_v3_intent_literals() {
    for (body, tag) in [
        (IntentBody::Stop, 2u8),
        (IntentBody::Pause, 3),
        (IntentBody::Unpause, 4),
    ] {
        let bytes = encode_intent(&intent(body)).unwrap();
        assert_eq!(bytes, intent_bytes(&[tag]));
        assert_eq!(bytes.len(), 22);
        assert_eq!(&bytes[bytes.len() - 2..], &[1, tag]);
        round_trip_intent(intent(body));
    }
    let real_time = encode_intent(&intent(IntentBody::RequestMode {
        mode: EncounterMode::RealTime,
    }))
    .unwrap();
    assert_eq!(real_time, intent_bytes(&[5, 1]));
    assert_eq!(real_time.len(), 23);
    let turn_based = encode_intent(&intent(IntentBody::RequestMode {
        mode: EncounterMode::TurnBased,
    }))
    .unwrap();
    assert_eq!(turn_based, intent_bytes(&[5, 0]));
    assert_eq!(turn_based.len(), 23);
    round_trip_intent(intent(IntentBody::RequestMode {
        mode: EncounterMode::RealTime,
    }));
    round_trip_intent(intent(IntentBody::RequestMode {
        mode: EncounterMode::TurnBased,
    }));

    let declare = intent(IntentBody::DeclareAction {
        ability: u(1),
        target: net(2),
    });
    let bytes = encode_intent(&declare).unwrap();
    let mut expected = cat(&[&[3], &Z16, &[0, 1, 7, 1, 0]]);
    expected.extend(ulid_wire(1));
    expected.push(2);
    assert_eq!(bytes, expected);
    assert_eq!(bytes.len(), 50);
    round_trip_intent(declare);

    // EndTurn keeps its v1 body.
    assert_eq!(
        encode_intent(&intent(IntentBody::EndTurn)).unwrap(),
        intent_bytes(&[1])
    );
}

// ---------------------------------------------------------------------------
// A3.
// ---------------------------------------------------------------------------

#[test]
fn a3_v3_intent_refusals() {
    // Controls: the shapes the refusals below perturb.
    assert!(decode_intent(&intent_bytes(&[5, 1])).is_ok());
    assert!(decode_intent(&intent_bytes(&[2])).is_ok());

    // Body tag 6 and FF.
    for tag in [6u8, 0xFF] {
        assert_eq!(
            decode_intent(&intent_bytes(&[tag])),
            Err(CodecError::UnknownMessage)
        );
    }
    // Mode byte 2 and FF.
    for mode in [2u8, 0xFF] {
        assert_eq!(
            decode_intent(&intent_bytes(&[5, mode])),
            Err(CodecError::UnknownMessage)
        );
    }
    // Lane 1.
    let lane1 = cat(&[&[3], &Z16, &[1, 1, 7, 1, 1]]);
    assert_eq!(decode_intent(&lane1), Err(CodecError::WrongDirection));
    // seq 0 and u64::MAX are Malformed; SEQ_LAST is accepted.
    let with_seq = |seq: &[u8]| cat(&[&[3], &Z16, &[0], seq, &[7, 1, 1]]);
    assert_eq!(decode_intent(&with_seq(&[0])), Err(CodecError::Malformed));
    assert_eq!(
        decode_intent(&with_seq(&varint(u64::MAX))),
        Err(CodecError::Malformed)
    );
    assert_eq!(SEQ_LAST, u64::MAX - 1);
    let last = decode_intent(&with_seq(&varint(SEQ_LAST))).unwrap();
    assert_eq!(last.seq, SEQ_LAST);
    // actor 0 and DeclareAction target 0.
    assert_eq!(
        decode_intent(&cat(&[&[3], &Z16, &[0, 1, 7, 0, 1]])),
        Err(CodecError::Malformed)
    );
    let mut declare_zero_target = cat(&[&[3], &Z16, &[0, 1, 7, 1, 0]]);
    declare_zero_target.extend(ulid_wire(1));
    let mut declare_ok = declare_zero_target.clone();
    declare_zero_target.push(0);
    declare_ok.push(2);
    assert_eq!(
        decode_intent(&declare_zero_target),
        Err(CodecError::Malformed)
    );
    assert!(decode_intent(&declare_ok).is_ok());
    // ULID length prefix of 257 (81 02) with no body: LimitExceeded.
    let long_prefix = cat(&[&[3], &Z16, &[0, 1, 7, 1, 0], &[0x81, 0x02]]);
    assert_eq!(decode_intent(&long_prefix), Err(CodecError::LimitExceeded));
    // 256 bytes of 'A': within the bound, not a ULID.
    let mut text256 = cat(&[&[3], &Z16, &[0, 1, 7, 1, 0], &[0x80, 0x02]]);
    text256.extend(std::iter::repeat_n(0x41, 256));
    text256.push(2);
    assert_eq!(decode_intent(&text256), Err(CodecError::Malformed));
    // Prefix says 26 but the body is short: Malformed.
    let short = cat(&[&[3], &Z16, &[0, 1, 7, 1, 0], &[0x1A], &[0x30; 5]]);
    assert_eq!(decode_intent(&short), Err(CodecError::Malformed));
    // One trailing byte.
    let mut trailing = intent_bytes(&[1]);
    trailing.push(0);
    assert_eq!(decode_intent(&trailing), Err(CodecError::Malformed));
    let mut trailing_declare = declare_ok.clone();
    trailing_declare.push(0);
    assert_eq!(decode_intent(&trailing_declare), Err(CodecError::Malformed));
    // RequestMode missing its last byte.
    assert_eq!(
        decode_intent(&intent_bytes(&[5])),
        Err(CodecError::Malformed)
    );
    // Truncation at every prefix of a valid frame is Malformed, not a panic.
    let full = intent_bytes(&[5, 1]);
    for cut in 1..full.len() {
        assert_eq!(
            decode_intent(&full[..cut]),
            Err(CodecError::Malformed),
            "cut at {cut}"
        );
    }

    // Precedence: version, then lane, then seq, then actor, then body tag.
    let mut bad_version_and_lane = lane1.clone();
    bad_version_and_lane[0] = 2;
    assert_eq!(
        decode_intent(&bad_version_and_lane),
        Err(CodecError::UnsupportedVersion)
    );
    let lane1_bad_seq = cat(&[&[3], &Z16, &[1, 0, 7, 1, 1]]);
    assert_eq!(
        decode_intent(&lane1_bad_seq),
        Err(CodecError::WrongDirection)
    );
    let actor0_bad_tag = cat(&[&[3], &Z16, &[0, 1, 7, 0, 0xFF]]);
    assert_eq!(decode_intent(&actor0_bad_tag), Err(CodecError::Malformed));
    let bad_tag_trailing = intent_bytes(&[0xFF, 0]);
    assert_eq!(
        decode_intent(&bad_tag_trailing),
        Err(CodecError::UnknownMessage)
    );

    // Encode.
    let mut frame = intent(IntentBody::EndTurn);
    frame.lane = 1;
    assert_eq!(encode_intent(&frame), Err(CodecError::WrongDirection));
    frame.lane = 0;
    frame.seq = 0;
    assert_eq!(encode_intent(&frame), Err(CodecError::Malformed));
    frame.seq = u64::MAX;
    assert_eq!(encode_intent(&frame), Err(CodecError::Malformed));
    frame.seq = SEQ_LAST;
    assert!(encode_intent(&frame).is_ok());
    // Lane is checked before seq.
    frame.lane = 1;
    frame.seq = 0;
    assert_eq!(encode_intent(&frame), Err(CodecError::WrongDirection));
}

// ---------------------------------------------------------------------------
// A4.
// ---------------------------------------------------------------------------

fn v2_legacy_ops() -> Vec<DeltaOpV2> {
    let mut ops = vec![
        DeltaOpV2::Legacy(DeltaOpV1::EntityEnter { entity: net(1) }),
        DeltaOpV2::Legacy(DeltaOpV1::EntityLeave { entity: net(1) }),
        DeltaOpV2::Legacy(DeltaOpV1::Spawned { entity: net(1) }),
        DeltaOpV2::Legacy(DeltaOpV1::Despawned { entity: net(1) }),
        DeltaOpV2::Legacy(DeltaOpV1::Died { entity: net(1) }),
        DeltaOpV2::Legacy(DeltaOpV1::Health {
            entity: net(1),
            health: 10,
            max_health: 10,
            dead: false,
        }),
        DeltaOpV2::Legacy(DeltaOpV1::Turn {
            active: None,
            round: 5,
        }),
        DeltaOpV2::Legacy(DeltaOpV1::Turn {
            active: Some(net(1)),
            round: 5,
        }),
        DeltaOpV2::Legacy(DeltaOpV1::Receipt {
            epoch: Z16,
            lane: 0,
            seq: 1,
            processed_tick: 100,
            status: ReceiptStatusV1::Applied,
        }),
        DeltaOpV2::ActionResolved {
            actor: net(1),
            target: net(2),
            ability: u(1),
            outcome: "success".to_owned(),
            damage: 7,
        },
        DeltaOpV2::TurnStarted {
            actor: net(1),
            round: 3,
        },
        DeltaOpV2::EncounterEnded {
            encounter: u(1),
            round: 3,
        },
    ];
    for raw in 1..=20u8 {
        let code = RejectionCodeV1::from_u8(raw).expect("v1 code");
        ops.push(DeltaOpV2::Legacy(DeltaOpV1::Receipt {
            epoch: Z16,
            lane: 0,
            seq: 1,
            processed_tick: 100,
            status: ReceiptStatusV1::Rejected(code),
        }));
    }
    ops
}

#[test]
fn a4_v3_legacy_tags_identical() {
    for op in v2_legacy_ops() {
        let v2_bytes = encode_delta_v2(&DeltaFrameV2 {
            lane: 0,
            server_tick: 100,
            event_seq: 1,
            ops: vec![op.clone()],
        })
        .unwrap();
        let v3_op = DeltaOp::from(op.clone());
        let v3_bytes = one_op(v3_op.clone());
        assert_eq!(v2_bytes[0], 2);
        assert_eq!(v3_bytes[0], 3);
        assert_eq!(&v2_bytes[1..], &v3_bytes[1..], "{op:?}");
        assert_eq!(decode_delta(&v3_bytes).unwrap(), delta(vec![v3_op]));
    }

    // Hand-written literals for every native tag 0..=10.
    let mut action = vec![8u8, 1, 2];
    action.extend(ulid_wire(1));
    action.push(7);
    action.extend(b"success");
    action.push(7);
    let mut ended = vec![10u8];
    ended.extend(ulid_wire(1));
    ended.push(3);
    let cases: Vec<(DeltaOp, Vec<u8>)> = vec![
        (DeltaOp::EntityEnter { entity: net(1) }, vec![0, 1]),
        (DeltaOp::EntityLeave { entity: net(1) }, vec![1, 1]),
        (DeltaOp::Spawned { entity: net(1) }, vec![2, 1]),
        (DeltaOp::Despawned { entity: net(1) }, vec![3, 1]),
        (DeltaOp::Died { entity: net(1) }, vec![4, 1]),
        (
            DeltaOp::Health {
                entity: net(1),
                health: 10,
                max_health: 10,
                dead: false,
            },
            vec![5, 1, 10, 10, 0],
        ),
        (
            DeltaOp::Turn {
                active: None,
                round: 5,
            },
            vec![6, 0, 5],
        ),
        (
            DeltaOp::Turn {
                active: Some(net(1)),
                round: 5,
            },
            vec![6, 1, 1, 5],
        ),
        (action_resolved("success"), action),
        (
            DeltaOp::TurnStarted {
                actor: net(1),
                round: 3,
            },
            vec![9, 1, 3],
        ),
        (
            DeltaOp::EncounterEnded {
                encounter: u(1),
                round: 3,
            },
            ended,
        ),
    ];
    for (op, tail) in cases {
        assert_eq!(one_op(op.clone()), delta_bytes(1, &tail), "{op:?}");
        round_trip_op(op);
    }
    // The contract's named oracle.
    assert_eq!(
        one_op(DeltaOp::Turn {
            active: None,
            round: 5
        }),
        vec![0x03, 0x00, 0x64, 0x01, 0x01, 0x06, 0x00, 0x05]
    );

    // From conversions are variant for variant.
    assert_eq!(
        DeltaOp::from(DeltaOpV1::Health {
            entity: net(4),
            health: 1,
            max_health: 2,
            dead: true
        }),
        DeltaOp::Health {
            entity: net(4),
            health: 1,
            max_health: 2,
            dead: true
        }
    );
    assert_eq!(
        DeltaOp::from(DeltaOpV2::Legacy(DeltaOpV1::Died { entity: net(4) })),
        DeltaOp::Died { entity: net(4) }
    );
    assert_eq!(
        DeltaOp::from(DeltaOpV2::TurnStarted {
            actor: net(4),
            round: 9
        }),
        DeltaOp::TurnStarted {
            actor: net(4),
            round: 9
        }
    );
}

// ---------------------------------------------------------------------------
// A5.
// ---------------------------------------------------------------------------

fn receipt_tail(lane: u8, seq: u8, status: &[u8]) -> Vec<u8> {
    cat(&[&[7], &Z16, &[lane, seq, 0x64], status])
}

#[test]
fn a5_v3_receipt_codes() {
    let applied = one_op(receipt(1, ReceiptStatus::Applied));
    assert_eq!(applied.len(), 26);
    assert_eq!(applied, delta_bytes(1, &receipt_tail(0, 1, &[0])));
    assert_eq!(
        one_op(receipt(
            1,
            ReceiptStatus::Rejected(RejectionCode::ModeUnsupported)
        )),
        delta_bytes(1, &receipt_tail(0, 1, &[1, 0x15]))
    );
    assert_eq!(
        one_op(receipt(
            1,
            ReceiptStatus::Rejected(RejectionCode::NoPauseRight)
        )),
        delta_bytes(1, &receipt_tail(0, 1, &[1, 0x1E]))
    );
    // Codes 21..=30 round-trip with their literal byte.
    for raw in 21..=30u8 {
        let code = RejectionCode::from_u8(raw).expect("v3 code");
        assert_eq!(code.as_u8(), raw);
        let op = receipt(1, ReceiptStatus::Rejected(code));
        assert_eq!(
            one_op(op.clone()),
            delta_bytes(1, &receipt_tail(0, 1, &[1, raw]))
        );
        round_trip_op(op);
    }
    // Controls: code bytes inside the range decode; outside is Unknown.
    for raw in 1..=30u8 {
        assert!(decode_delta(&delta_bytes(1, &receipt_tail(0, 1, &[1, raw]))).is_ok());
    }
    for raw in [0x00u8, 0x1F, 0xFF] {
        assert_eq!(
            decode_delta(&delta_bytes(1, &receipt_tail(0, 1, &[1, raw]))),
            Err(CodecError::UnknownMessage),
            "code {raw:#x}"
        );
    }
    // Status tag 02.
    assert_eq!(
        decode_delta(&delta_bytes(1, &receipt_tail(0, 1, &[2]))),
        Err(CodecError::UnknownMessage)
    );
    // A rejected status missing its code byte is truncation.
    assert_eq!(
        decode_delta(&delta_bytes(1, &receipt_tail(0, 1, &[1]))),
        Err(CodecError::Malformed)
    );
    // Receipt lane 1 and seq 0.
    assert_eq!(
        decode_delta(&delta_bytes(1, &receipt_tail(1, 1, &[0]))),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        decode_delta(&delta_bytes(1, &receipt_tail(0, 0, &[0]))),
        Err(CodecError::Malformed)
    );
    // Encode side mirrors both.
    let mut lane1 = receipt(1, ReceiptStatus::Applied);
    if let DeltaOp::Receipt { lane, .. } = &mut lane1 {
        *lane = 1;
    }
    assert_eq!(
        encode_delta(&delta(vec![lane1])),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        encode_delta(&delta(vec![receipt(0, ReceiptStatus::Applied)])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        encode_delta(&delta(vec![receipt(u64::MAX, ReceiptStatus::Applied)])),
        Err(CodecError::Malformed)
    );
    assert!(encode_delta(&delta(vec![receipt(SEQ_LAST, ReceiptStatus::Applied)])).is_ok());

    // from_u8 / as_u8 agree on exactly 1..=30.
    assert_eq!(RejectionCode::from_u8(0), None);
    for raw in 1..=30u8 {
        assert_eq!(
            RejectionCode::from_u8(raw).map(RejectionCode::as_u8),
            Some(raw)
        );
    }
    for raw in 31..=255u8 {
        assert_eq!(RejectionCode::from_u8(raw), None, "{raw}");
    }
    // Names are pinned at their numbers.
    assert_eq!(RejectionCode::ModeUnsupported.as_u8(), 21);
    assert_eq!(RejectionCode::WrongMode.as_u8(), 22);
    assert_eq!(RejectionCode::StatEvaluation.as_u8(), 23);
    assert_eq!(RejectionCode::EncounterTerminal.as_u8(), 24);
    assert_eq!(RejectionCode::RulesetMismatch.as_u8(), 25);
    assert_eq!(RejectionCode::PlacementInUse.as_u8(), 26);
    assert_eq!(RejectionCode::NotCreature.as_u8(), 27);
    assert_eq!(RejectionCode::AlreadyParticipant.as_u8(), 28);
    assert_eq!(RejectionCode::NoEntrants.as_u8(), 29);
    assert_eq!(RejectionCode::NoPauseRight.as_u8(), 30);
    // v1 values stay put and 21..=30 are still unassigned there.
    for raw in 1..=20u8 {
        let v1 = RejectionCodeV1::from_u8(raw).expect("v1 code");
        assert_eq!(RejectionCode::from(v1).as_u8(), raw);
        assert_eq!(
            ReceiptStatus::from(ReceiptStatusV1::Rejected(v1)),
            ReceiptStatus::Rejected(RejectionCode::from_u8(raw).unwrap())
        );
    }
    assert_eq!(
        ReceiptStatus::from(ReceiptStatusV1::Applied),
        ReceiptStatus::Applied
    );
    for raw in 21..=30u8 {
        assert_eq!(RejectionCodeV1::from_u8(raw), None);
    }
}

// ---------------------------------------------------------------------------
// A6.
// ---------------------------------------------------------------------------

#[test]
fn a6_v3_event_literals() {
    let cases: Vec<(DeltaOp, Vec<u8>)> = vec![
        (DeltaOp::RoundStarted { round: 3 }, vec![0x0E, 3]),
        (
            DeltaOp::ModeChanged {
                mode: EncounterMode::RealTime,
                round: 2,
            },
            vec![0x0F, 1, 2],
        ),
        (
            DeltaOp::ModeChanged {
                mode: EncounterMode::TurnBased,
                round: 2,
            },
            vec![0x0F, 0, 2],
        ),
        (DeltaOp::OrderEnded { actor: net(2) }, vec![0x10, 2]),
        (DeltaOp::Joined { entity: net(2) }, vec![0x11, 2]),
        (DeltaOp::Left { entity: net(2) }, vec![0x12, 2]),
        (
            DeltaOp::RoundStarted { round: u64::MAX },
            vec![
                0x0E, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01,
            ],
        ),
    ];
    for (op, tail) in cases {
        assert_eq!(one_op(op.clone()), delta_bytes(1, &tail), "{op:?}");
        round_trip_op(op);
    }
    // Mode byte 2 and FF are UnknownMessage (control: 0 and 1 above).
    for mode in [2u8, 0xFF] {
        assert_eq!(
            decode_delta(&delta_bytes(1, &[0x0F, mode, 2])),
            Err(CodecError::UnknownMessage)
        );
    }
    // The mode is read before the round.
    assert_eq!(
        decode_delta(&delta_bytes(1, &[0x0F, 2])),
        Err(CodecError::UnknownMessage)
    );
    // Actor and entity 0 are Malformed (controls: id 1 decodes).
    for tag in [0x10u8, 0x11, 0x12] {
        assert_eq!(
            decode_delta(&delta_bytes(1, &[tag, 0])),
            Err(CodecError::Malformed),
            "tag {tag:#x}"
        );
        assert!(decode_delta(&delta_bytes(1, &[tag, 1])).is_ok());
    }
    // Truncated payloads.
    for tail in [&[0x0Eu8][..], &[0x0F, 1], &[0x10], &[0x11], &[0x12]] {
        assert_eq!(
            decode_delta(&delta_bytes(1, tail)),
            Err(CodecError::Malformed)
        );
    }
    // Several ops in order.
    let frame = delta(vec![
        DeltaOp::RoundStarted { round: 3 },
        DeltaOp::Joined { entity: net(2) },
        DeltaOp::Left { entity: net(2) },
    ]);
    assert_eq!(
        encode_delta(&frame).unwrap(),
        delta_bytes(3, &[0x0E, 3, 0x11, 2, 0x12, 2])
    );
}

// ---------------------------------------------------------------------------
// A7.
// ---------------------------------------------------------------------------

fn clock(paused: Option<Option<PauseReason>>) -> DeltaOp {
    DeltaOp::Clock { paused }
}

fn host(
    pause: bool,
    unpause: bool,
    auto_pause: Option<AutoPauseToggles>,
    turn_timer_ms: Option<Option<u32>>,
) -> DeltaOp {
    DeltaOp::HostFacts(HostFacts {
        rights: PauseRights { pause, unpause },
        auto_pause,
        turn_timer_ms,
    })
}

#[test]
fn a7_v3_clock_and_host_facts() {
    let clock_cases: Vec<(DeltaOp, Vec<u8>)> = vec![
        (clock(None), vec![0x13, 0]),
        (clock(Some(None)), vec![0x13, 1, 0]),
        (clock(Some(Some(PauseReason::Player))), vec![0x13, 1, 1, 0]),
        (
            clock(Some(Some(PauseReason::Auto(AutoPauseTrigger::OrderEnded)))),
            vec![0x13, 1, 1, 1, 3],
        ),
        (
            clock(Some(Some(PauseReason::Operator))),
            vec![0x13, 1, 1, 2],
        ),
        (clock(Some(Some(PauseReason::Restore))), vec![0x13, 1, 1, 3]),
        (
            clock(Some(Some(PauseReason::Auto(
                AutoPauseTrigger::EncounterStarted,
            )))),
            vec![0x13, 1, 1, 1, 0],
        ),
        (
            clock(Some(Some(PauseReason::Auto(
                AutoPauseTrigger::RoundStarted,
            )))),
            vec![0x13, 1, 1, 1, 1],
        ),
        (
            clock(Some(Some(PauseReason::Auto(AutoPauseTrigger::ModeChanged)))),
            vec![0x13, 1, 1, 1, 2],
        ),
        (
            clock(Some(Some(PauseReason::Auto(
                AutoPauseTrigger::PartyMemberDied,
            )))),
            vec![0x13, 1, 1, 1, 4],
        ),
    ];
    for (op, tail) in clock_cases {
        assert_eq!(one_op(op.clone()), delta_bytes(1, &tail), "{op:?}");
        round_trip_op(op);
    }
    // Reason 04 / FF and trigger 05 / FF are UnknownMessage (controls above).
    for reason in [4u8, 0xFF] {
        assert_eq!(
            decode_delta(&delta_bytes(1, &[0x13, 1, 1, reason])),
            Err(CodecError::UnknownMessage)
        );
    }
    for trigger in [5u8, 0xFF] {
        assert_eq!(
            decode_delta(&delta_bytes(1, &[0x13, 1, 1, 1, trigger])),
            Err(CodecError::UnknownMessage)
        );
    }
    // Flag bytes of 02 are Malformed (postcard bool).
    assert_eq!(
        decode_delta(&delta_bytes(1, &[0x13, 2])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_delta(&delta_bytes(1, &[0x13, 1, 2])),
        Err(CodecError::Malformed)
    );
    // A running or withheld clock has no further bytes: extra bytes trail.
    assert_eq!(
        decode_delta(&delta_bytes(1, &[0x13, 0, 0])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_delta(&delta_bytes(1, &[0x13, 1, 0, 0])),
        Err(CodecError::Malformed)
    );
    // A non-Auto reason has no trigger byte; an Auto reason requires one.
    assert_eq!(
        decode_delta(&delta_bytes(1, &[0x13, 1, 1, 0, 0])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_delta(&delta_bytes(1, &[0x13, 1, 1, 1])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_delta(&delta_bytes(1, &[0x13])),
        Err(CodecError::Malformed)
    );

    // HostFacts.
    let both_withheld = host(true, true, None, None);
    assert_eq!(
        one_op(both_withheld.clone()),
        delta_bytes(1, &[0x14, 1, 1, 0, 0])
    );
    round_trip_op(both_withheld);
    let full = host(
        true,
        true,
        Some(AutoPauseToggles::from_bits(0x1F).unwrap()),
        Some(Some(30000)),
    );
    assert_eq!(
        one_op(full.clone()),
        delta_bytes(1, &[0x14, 1, 1, 1, 0x1F, 1, 1, 0xB0, 0xEA, 0x01])
    );
    round_trip_op(full);
    let no_timer = host(true, true, None, Some(None));
    assert_eq!(
        one_op(no_timer.clone()),
        delta_bytes(1, &[0x14, 1, 1, 0, 1, 0])
    );
    round_trip_op(no_timer);
    // "Withheld" and "no timer" do not share bytes.
    assert_ne!(
        one_op(host(true, true, None, None)),
        one_op(host(true, true, None, Some(None)))
    );
    // Rights are independent bits in order (pause, unpause).
    assert_eq!(
        one_op(host(false, true, None, None)),
        delta_bytes(1, &[0x14, 0, 1, 0, 0])
    );
    assert_eq!(
        one_op(host(true, false, None, None)),
        delta_bytes(1, &[0x14, 1, 0, 0, 0])
    );
    round_trip_op(host(false, true, None, None));
    round_trip_op(host(true, false, None, None));
    // Zero toggles are disclosed, not withheld.
    assert_eq!(
        one_op(host(false, false, Some(AutoPauseToggles::default()), None)),
        delta_bytes(1, &[0x14, 0, 0, 1, 0, 0])
    );
    round_trip_op(host(false, false, Some(AutoPauseToggles::default()), None));
    // Timer 0 is disclosed and distinct from no timer.
    assert_eq!(
        one_op(host(true, true, None, Some(Some(0)))),
        delta_bytes(1, &[0x14, 1, 1, 0, 1, 1, 0])
    );
    round_trip_op(host(true, true, None, Some(Some(0))));

    // Toggle bits outside the mask are Malformed; the mask edge is accepted.
    for bits in [0x20u8, 0x40, 0x80, 0xE0, 0xFF] {
        assert_eq!(
            decode_delta(&delta_bytes(1, &[0x14, 1, 1, 1, bits, 0])),
            Err(CodecError::Malformed),
            "bits {bits:#x}"
        );
    }
    for bits in [0x00u8, 0x01, 0x10, 0x1F] {
        let decoded = decode_delta(&delta_bytes(1, &[0x14, 1, 1, 1, bits, 0])).unwrap();
        assert_eq!(
            decoded.ops,
            vec![host(
                true,
                true,
                Some(AutoPauseToggles::from_bits(bits).unwrap()),
                None
            )]
        );
    }
    // Flag bytes of 02 in each flag position.
    for tail in [
        &[0x14u8, 2, 1, 0, 0][..],
        &[0x14, 1, 2, 0, 0],
        &[0x14, 1, 1, 2, 0],
        &[0x14, 1, 1, 0, 2],
        &[0x14, 1, 1, 0, 1, 2],
    ] {
        assert_eq!(
            decode_delta(&delta_bytes(1, tail)),
            Err(CodecError::Malformed),
            "{tail:?}"
        );
    }
    // u32::MAX timer round-trips as five varint bytes.
    let max_timer = host(true, true, None, Some(Some(u32::MAX)));
    assert_eq!(
        one_op(max_timer.clone()),
        delta_bytes(1, &[0x14, 1, 1, 0, 1, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F])
    );
    round_trip_op(max_timer);
    // A timer varint above u32 is Malformed (postcard u32 overflow).
    assert_eq!(
        decode_delta(&delta_bytes(
            1,
            &[0x14, 1, 1, 0, 1, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0x1F]
        )),
        Err(CodecError::Malformed)
    );
    // Truncation inside the timer.
    assert_eq!(
        decode_delta(&delta_bytes(1, &[0x14, 1, 1, 0, 1, 1])),
        Err(CodecError::Malformed)
    );

    // Toggle value semantics.
    assert!(AutoPauseToggles::from_bits(0x1F).is_some());
    assert!(AutoPauseToggles::from_bits(0x20).is_none());
    assert_eq!(
        AutoPauseToggles::default()
            .with(AutoPauseTrigger::OrderEnded, true)
            .bits(),
        0x08
    );
}

// ---------------------------------------------------------------------------
// A8.
// ---------------------------------------------------------------------------

#[test]
fn a8_v3_tag_table_closed() {
    assert_eq!(PROTOCOL_VERSION, 3);
    assert_eq!(INTENT_TAG_DECLARE_ACTION, 0);
    assert_eq!(INTENT_TAG_END_TURN, 1);
    assert_eq!(INTENT_TAG_STOP, 2);
    assert_eq!(INTENT_TAG_PAUSE, 3);
    assert_eq!(INTENT_TAG_UNPAUSE, 4);
    assert_eq!(INTENT_TAG_REQUEST_MODE, 5);
    assert_eq!(DELTA_TAG_ENCOUNTER, 11);
    assert_eq!(DELTA_TAG_TIMELINE, 12);
    assert_eq!(DELTA_TAG_ENTITY_FACTS, 13);
    assert_eq!(DELTA_TAG_ROUND_STARTED, 14);
    assert_eq!(DELTA_TAG_MODE_CHANGED, 15);
    assert_eq!(DELTA_TAG_ORDER_ENDED, 16);
    assert_eq!(DELTA_TAG_JOINED, 17);
    assert_eq!(DELTA_TAG_LEFT, 18);
    assert_eq!(DELTA_TAG_CLOCK, 19);
    assert_eq!(DELTA_TAG_HOST_FACTS, 20);
    assert_eq!(AUTO_PAUSE_TOGGLE_MASK, 0x1F);
    assert_eq!(POLICY_CLOCK_HEARTBEAT_TICKS, 100);

    // Tag 21 (0x15) and FF are unknown; the reserved 11..=13 are unknown in
    // T140a, with and without a plausible payload. Control: tag 20 decodes.
    assert!(decode_delta(&delta_bytes(1, &[0x14, 1, 1, 0, 0])).is_ok());
    for tag in [0x15u8, 0xFF] {
        assert_eq!(
            decode_delta(&delta_bytes(1, &[tag])),
            Err(CodecError::UnknownMessage)
        );
    }
    for tag in [0x0Bu8, 0x0C, 0x0D] {
        assert_eq!(
            decode_delta(&delta_bytes(1, &[tag])),
            Err(CodecError::UnknownMessage),
            "tag {tag:#x}"
        );
        assert_eq!(
            decode_delta(&delta_bytes(1, &[tag, 0, 0, 0])),
            Err(CodecError::UnknownMessage),
            "tag {tag:#x} with payload"
        );
    }
    // Every byte outside 0..=10 and 14..=20 is unknown, with an empty tail.
    for tag in 0..=255u8 {
        let result = decode_delta(&delta_bytes(1, &[tag]));
        if (11..=13).contains(&tag) || tag >= 21 {
            assert_eq!(result, Err(CodecError::UnknownMessage), "tag {tag}");
        } else {
            assert_ne!(result, Err(CodecError::UnknownMessage), "tag {tag}");
        }
    }
    // Same for intent body tags: 6..=255 are unknown.
    for tag in 0..=255u8 {
        let result = decode_intent(&intent_bytes(&[tag]));
        if tag >= 6 {
            assert_eq!(result, Err(CodecError::UnknownMessage), "tag {tag}");
        } else {
            assert_ne!(result, Err(CodecError::UnknownMessage), "tag {tag}");
        }
    }
    // The v2 codec does not know tag 0E.
    assert_eq!(
        decode_delta_v2(&[2, 0, 0x64, 1, 1, 0x0E, 3]),
        Err(CodecError::UnknownMessage)
    );
    // Control: v3 reads the same tail.
    assert_eq!(
        decode_delta(&delta_bytes(1, &[0x0E, 3])).unwrap().ops,
        vec![DeltaOp::RoundStarted { round: 3 }]
    );
}

// ---------------------------------------------------------------------------
// A9.
// ---------------------------------------------------------------------------

#[test]
fn a9_v3_delta_bounds() {
    assert_eq!(
        encode_delta(&delta(vec![])).unwrap(),
        vec![0x03, 0x00, 0x64, 0x01, 0x00]
    );
    // 256 ops are accepted, both ways.
    let ops: Vec<DeltaOp> = (0..256)
        .map(|_| DeltaOp::RoundStarted { round: 0 })
        .collect();
    let bytes = encode_delta(&delta(ops.clone())).unwrap();
    let mut expected = vec![0x03, 0x00, 0x64, 0x01, 0x80, 0x02];
    for _ in 0..256 {
        expected.extend([0x0E, 0x00]);
    }
    assert_eq!(bytes, expected);
    assert_eq!(decode_delta(&bytes).unwrap().ops, ops);
    // Count 257 with no ops following is LimitExceeded (cap before ops).
    assert_eq!(
        decode_delta(&[0x03, 0x00, 0x64, 0x01, 0x81, 0x02]),
        Err(CodecError::LimitExceeded)
    );
    // Control: count 256 with no ops passes the cap and then truncates.
    assert_eq!(
        decode_delta(&[0x03, 0x00, 0x64, 0x01, 0x80, 0x02]),
        Err(CodecError::Malformed)
    );
    // Encoding 257 ops is LimitExceeded.
    let too_many: Vec<DeltaOp> = (0..257)
        .map(|_| DeltaOp::RoundStarted { round: 0 })
        .collect();
    assert_eq!(
        encode_delta(&delta(too_many)),
        Err(CodecError::LimitExceeded)
    );
    // Frame lane 1.
    assert_eq!(
        decode_delta(&[0x03, 0x01, 0x64, 0x01, 0x00]),
        Err(CodecError::WrongDirection)
    );
    let mut lane1 = delta(vec![]);
    lane1.lane = 1;
    assert_eq!(encode_delta(&lane1), Err(CodecError::WrongDirection));
    // One trailing byte.
    assert_eq!(
        decode_delta(&[0x03, 0x00, 0x64, 0x01, 0x00, 0x00]),
        Err(CodecError::Malformed)
    );
    // A truncated header.
    for cut in 1..5 {
        assert_eq!(
            decode_delta(&[0x03, 0x00, 0x64, 0x01, 0x00][..cut]),
            Err(CodecError::Malformed)
        );
    }
    // Header scalars take the full u64 range.
    let wide = DeltaFrame {
        lane: 0,
        server_tick: u64::MAX,
        event_seq: u64::MAX,
        ops: vec![],
    };
    let wide_bytes = encode_delta(&wide).unwrap();
    let max_varint = [0xFFu8, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];
    assert_eq!(wide_bytes, cat(&[&[3, 0], &max_varint, &max_varint, &[0]]));
    assert_eq!(decode_delta(&wide_bytes).unwrap(), wide);
    // Zero event_seq is also just a number at this layer.
    let zero = DeltaFrame {
        lane: 0,
        server_tick: 0,
        event_seq: 0,
        ops: vec![],
    };
    assert_eq!(decode_delta(&encode_delta(&zero).unwrap()).unwrap(), zero);
    // An overlong varint in the header is Malformed.
    let overlong = cat(&[&[3, 0], &[0xFF; 10], &[1, 0]]);
    assert_eq!(decode_delta(&overlong), Err(CodecError::Malformed));
}

// ---------------------------------------------------------------------------
// A10.
// ---------------------------------------------------------------------------

#[test]
fn a10_v3_round_trip_identity() {
    let ops = vec![
        DeltaOp::EntityEnter { entity: net(1) },
        DeltaOp::EntityLeave { entity: net(2) },
        DeltaOp::Spawned { entity: net(3) },
        DeltaOp::Despawned { entity: net(4) },
        DeltaOp::Died { entity: net(5) },
        DeltaOp::Health {
            entity: net(6),
            health: 0,
            max_health: u32::MAX,
            dead: true,
        },
        DeltaOp::Turn {
            active: None,
            round: 0,
        },
        DeltaOp::Turn {
            active: Some(net(u64::MAX)),
            round: u64::MAX,
        },
        receipt(1, ReceiptStatus::Applied),
        receipt(SEQ_LAST, ReceiptStatus::Rejected(RejectionCode::NoEntrants)),
        action_resolved("critical_success"),
        action_resolved("critical_failure"),
        action_resolved("failure"),
        action_resolved("custom:0"),
        action_resolved("custom:255"),
        DeltaOp::TurnStarted {
            actor: net(2),
            round: 1,
        },
        DeltaOp::EncounterEnded {
            encounter: u(3),
            round: 2,
        },
        DeltaOp::RoundStarted { round: 9 },
        DeltaOp::ModeChanged {
            mode: EncounterMode::TurnBased,
            round: 9,
        },
        DeltaOp::OrderEnded { actor: net(7) },
        DeltaOp::Joined { entity: net(8) },
        DeltaOp::Left { entity: net(9) },
        clock(None),
        clock(Some(None)),
        clock(Some(Some(PauseReason::Restore))),
        host(false, true, Some(AutoPauseToggles::default()), Some(None)),
    ];
    for op in &ops {
        round_trip_op(op.clone());
    }
    let whole = delta(ops);
    let bytes = encode_delta(&whole).unwrap();
    assert_eq!(bytes, encode_delta(&whole).unwrap());
    assert_eq!(decode_delta(&bytes).unwrap(), whole);

    for body in [
        IntentBody::DeclareAction {
            ability: u(3),
            target: net(u64::MAX),
        },
        IntentBody::EndTurn,
        IntentBody::Stop,
        IntentBody::Pause,
        IntentBody::Unpause,
        IntentBody::RequestMode {
            mode: EncounterMode::RealTime,
        },
    ] {
        round_trip_intent(intent(body));
    }

    // A lowercase ULID alias decodes and re-encodes canonical uppercase.
    let ability = Ulid::from_parts(0x0123_4567_89AB, 0x0CDE_F012_3456_789A);
    let canonical = ability.to_string();
    assert!(canonical.chars().any(|c| c.is_ascii_uppercase()));
    let lowered = canonical.to_ascii_lowercase();
    assert_ne!(canonical, lowered);
    let alias_bytes = cat(&[&[3], &Z16, &[0, 1, 7, 1, 0, 26], lowered.as_bytes(), &[2]]);
    let frame = decode_intent(&alias_bytes).unwrap();
    assert_eq!(
        frame.body,
        IntentBody::DeclareAction {
            ability,
            target: net(2)
        }
    );
    let canonical_bytes = encode_intent(&frame).unwrap();
    assert_eq!(
        canonical_bytes,
        cat(&[&[3], &Z16, &[0, 1, 7, 1, 0, 26], canonical.as_bytes(), &[2]])
    );
    assert_ne!(canonical_bytes, alias_bytes);
    // Same for an encounter id inside a delta.
    let alias_delta = delta_bytes(1, &cat(&[&[10, 26], lowered.as_bytes(), &[3]]));
    let decoded = decode_delta(&alias_delta).unwrap();
    assert_eq!(
        decoded.ops,
        vec![DeltaOp::EncounterEnded {
            encounter: ability,
            round: 3
        }]
    );
    assert_eq!(
        encode_delta(&decoded).unwrap(),
        delta_bytes(1, &cat(&[&[10, 26], canonical.as_bytes(), &[3]]))
    );
}

// ---------------------------------------------------------------------------
// A11.
// ---------------------------------------------------------------------------

#[test]
fn a11_v3_delta_encode_checks() {
    // Outcome vocabulary and bound.
    assert!(encode_delta(&delta(vec![action_resolved("custom:255")])).is_ok());
    assert_eq!(
        encode_delta(&delta(vec![action_resolved("bogus")])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        encode_delta(&delta(vec![action_resolved("custom:256")])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        encode_delta(&delta(vec![action_resolved(&"a".repeat(257))])),
        Err(CodecError::LimitExceeded)
    );
    let long_custom = format!("custom:{}", "9".repeat(250));
    assert_eq!(long_custom.len(), 257);
    assert_eq!(
        encode_delta(&delta(vec![action_resolved(&long_custom)])),
        Err(CodecError::LimitExceeded)
    );
    // 256 bytes is inside the bound, then fails the vocabulary.
    assert_eq!(
        encode_delta(&delta(vec![action_resolved(&"a".repeat(256))])),
        Err(CodecError::Malformed)
    );
    // Decode applies the same vocabulary.
    let mut bad_outcome = vec![8u8, 1, 2];
    bad_outcome.extend(ulid_wire(1));
    bad_outcome.push(5);
    bad_outcome.extend(b"bogus");
    bad_outcome.push(7);
    assert_eq!(
        decode_delta(&delta_bytes(1, &bad_outcome)),
        Err(CodecError::Malformed)
    );

    // Order of checks: frame lane, then op count, then ops.
    let mut lane1_many: Vec<DeltaOp> = (0..257)
        .map(|_| DeltaOp::RoundStarted { round: 0 })
        .collect();
    let mut frame = delta(lane1_many.clone());
    frame.lane = 1;
    assert_eq!(encode_delta(&frame), Err(CodecError::WrongDirection));
    lane1_many[0] = action_resolved("bogus");
    assert_eq!(
        encode_delta(&delta(lane1_many)),
        Err(CodecError::LimitExceeded)
    );
    // The first failing op wins.
    let mut bad_lane_receipt = receipt(1, ReceiptStatus::Applied);
    if let DeltaOp::Receipt { lane, .. } = &mut bad_lane_receipt {
        *lane = 1;
    }
    assert_eq!(
        encode_delta(&delta(vec![
            action_resolved("bogus"),
            bad_lane_receipt.clone()
        ])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        encode_delta(&delta(vec![bad_lane_receipt, action_resolved("bogus")])),
        Err(CodecError::WrongDirection)
    );
    // Control: the same ops, valid, encode.
    assert!(encode_delta(&delta(vec![
        action_resolved("success"),
        receipt(1, ReceiptStatus::Applied)
    ]))
    .is_ok());
}

// ---------------------------------------------------------------------------
// Scalar value types (A-9).
// ---------------------------------------------------------------------------

#[test]
fn a9_toggle_and_enum_value_semantics() {
    for bits in 0..=255u8 {
        assert_eq!(
            AutoPauseToggles::from_bits(bits).map(AutoPauseToggles::bits),
            if bits <= 0x1F { Some(bits) } else { None },
            "bits {bits:#x}"
        );
    }
    let triggers = [
        AutoPauseTrigger::EncounterStarted,
        AutoPauseTrigger::RoundStarted,
        AutoPauseTrigger::ModeChanged,
        AutoPauseTrigger::OrderEnded,
        AutoPauseTrigger::PartyMemberDied,
    ];
    for (n, trigger) in triggers.into_iter().enumerate() {
        assert_eq!(trigger.as_u8(), n as u8);
        assert_eq!(AutoPauseTrigger::from_u8(n as u8), Some(trigger));
        let on = AutoPauseToggles::default().with(trigger, true);
        assert_eq!(on.bits(), 1 << n);
        assert!(on.get(trigger));
        for other in triggers {
            if other != trigger {
                assert!(!on.get(other));
            }
        }
        // Switching off clears exactly that bit; switching on twice is idempotent.
        assert_eq!(on.with(trigger, true), on);
        assert_eq!(on.with(trigger, false), AutoPauseToggles::default());
    }
    let all = triggers
        .into_iter()
        .fold(AutoPauseToggles::default(), |t, tr| t.with(tr, true));
    assert_eq!(all.bits(), 0x1F);
    assert_eq!(all.with(AutoPauseTrigger::RoundStarted, false).bits(), 0x1D);
    for raw in 5..=255u8 {
        assert_eq!(AutoPauseTrigger::from_u8(raw), None);
    }

    assert_eq!(PauseReason::Player.tag(), 0);
    assert_eq!(
        PauseReason::Auto(AutoPauseTrigger::PartyMemberDied).tag(),
        1
    );
    assert_eq!(PauseReason::Operator.tag(), 2);
    assert_eq!(PauseReason::Restore.tag(), 3);

    assert_eq!(EncounterMode::TurnBased.as_u8(), 0);
    assert_eq!(EncounterMode::RealTime.as_u8(), 1);
    assert_eq!(EncounterMode::from_u8(0), Some(EncounterMode::TurnBased));
    assert_eq!(EncounterMode::from_u8(1), Some(EncounterMode::RealTime));
    for raw in 2..=255u8 {
        assert_eq!(EncounterMode::from_u8(raw), None);
    }
    assert!(EncounterMode::TurnBased < EncounterMode::RealTime);
    assert_eq!(
        PauseRights::default(),
        PauseRights {
            pause: false,
            unpause: false
        }
    );
}
