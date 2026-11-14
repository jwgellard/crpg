//! T026p1 protocol v4 lane-0 acceptance: version gate, intents (the six v3
//! bodies plus `DeclareMove`), deltas (every v3 op by its v3 bytes plus
//! `Moved` and `Position`), the `Point` bound and the v4 `frame_spans`
//! (`tasks/T026p.md`, tests W1..=W11).
//!
//! Public API only. Every v4 expectation is a hand-written literal byte or
//! value oracle built from the contract's layout (postcard primitives: raw
//! `u8` and `bool`, LEB128 varints, zigzag `i32`), never from the codec under
//! test; the v3 identities are differentials against the frozen v3 codec.
//! Every bad-byte case carries a complete valid remainder and a control with
//! the good byte that decodes `Ok`, so truncation cannot mask a refusal.

use core::ops::Range;

use crpg_core::Ulid;
use crpg_net::codec::{
    decode_delta as decode_delta_v1, decode_intent as decode_intent_v1,
    encode_intent as encode_intent_v1, CodecError,
};
use crpg_net::codec_v2::{
    decode_delta as decode_delta_v2, decode_intent as decode_intent_v2,
    encode_intent as encode_intent_v2,
};
use crpg_net::codec_v3;
use crpg_net::codec_v4::{decode_delta, decode_intent, encode_delta, encode_intent, frame_spans};
use crpg_net::protocol::{
    IntentBody as IntentBodyV1, IntentFrame as IntentFrameV1, LANE_COMBAT, MAX_DELTA_FRAME_BYTES,
    MAX_DELTA_OPS, MAX_INTENT_FRAME_BYTES, SEQ_LAST,
};
use crpg_net::protocol_v3 as v3;
use crpg_net::protocol_v4::{
    check_points, DeltaFrame, DeltaOp, EncounterMode, IntentBody, IntentFrame, NetId, Point,
    PositionFact, ReceiptStatus, RejectionCode, COORD_LIMIT_RAW, DELTA_TAG_MOVED,
    DELTA_TAG_POSITION, INTENT_TAG_DECLARE_MOVE, MAX_MOVED_POINTS, PROTOCOL_VERSION,
};

// ---------------------------------------------------------------------------
// Fixtures and byte helpers.
// ---------------------------------------------------------------------------

const Z16: [u8; 16] = [0; 16];
/// `L = 2^29`.
const L: i32 = 1 << 29;

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

/// Zigzag `i32` as a varint, written independently of postcard.
fn zz(value: i32) -> Vec<u8> {
    varint(u64::from(((value << 1) ^ (value >> 31)) as u32))
}

/// `P(x, y, z)` of the contract.
fn p(x: i32, y: i32, z: i32) -> Point {
    Point::new([x, y, z]).expect("fixture point is within U3")
}

/// `A = P(0, 0, 0)`.
fn pa() -> Point {
    p(0, 0, 0)
}

/// `B = P(65536, 0, -131072)`.
fn pb() -> Point {
    p(65_536, 0, -131_072)
}

/// The wire bytes of a point, from the zigzag oracle.
fn pt(point: Point) -> Vec<u8> {
    let [x, y, z] = point.raw();
    cat(&[&zz(x), &zz(y), &zz(z)])
}

/// `P64`: `P(L - i, L, L)` for `i` in 0..64 (15 bytes each, no zero segment).
fn p64() -> Vec<Point> {
    (0..64).map(|i| p(L - i, L, L)).collect()
}

/// `P65`: the same for `i` in 0..65.
fn p65() -> Vec<Point> {
    (0..65).map(|i| p(L - i, L, L)).collect()
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

fn v3_intent(body: v3::IntentBody) -> v3::IntentFrame {
    v3::IntentFrame {
        epoch: Z16,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 7,
        actor: net(1),
        body,
    }
}

/// `<version> Z16 <lane> <seq> 7 <actor>` followed by `tail`.
fn hdr(version: u8, lane: u8, seq: &[u8], actor: &[u8], tail: &[u8]) -> Vec<u8> {
    cat(&[&[version], &Z16, &[lane], seq, &[7], actor, tail])
}

/// `04 Z16 00 01 07 01` followed by `tail` (the defaults).
fn intent_bytes(tail: &[u8]) -> Vec<u8> {
    hdr(4, 0, &[1], &[1], tail)
}

fn delta(ops: Vec<DeltaOp>) -> DeltaFrame {
    DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 100,
        event_seq: 1,
        ops,
    }
}

/// `04 00 64 01 <count>` followed by `ops`.
fn delta_bytes(count: u8, ops: &[u8]) -> Vec<u8> {
    cat(&[&[4, 0, 0x64, 1, count], ops])
}

fn one_op(op: DeltaOp) -> Vec<u8> {
    encode_delta(&delta(vec![op])).expect("fixture op encodes")
}

fn try_one_op(op: DeltaOp) -> Result<Vec<u8>, CodecError> {
    encode_delta(&delta(vec![op]))
}

/// The decode error of a one-op frame holding `tail`.
fn dec_err(tail: &[u8]) -> CodecError {
    decode_delta(&delta_bytes(1, tail)).expect_err("tail must be refused")
}

fn dec_ok(tail: &[u8]) -> DeltaOp {
    let mut frame = decode_delta(&delta_bytes(1, tail)).expect("tail must decode");
    assert_eq!(frame.ops.len(), 1);
    frame.ops.remove(0)
}

fn wrap(op: v3::DeltaOp) -> DeltaOp {
    DeltaOp::V3(op)
}

fn moved(entity: u64, points: &[Point]) -> DeltaOp {
    DeltaOp::Moved {
        entity: net(entity),
        points: points.to_vec(),
    }
}

fn position(entity: u64, fact: Option<(Point, bool)>) -> DeltaOp {
    DeltaOp::Position {
        entity: net(entity),
        position: fact.map(|(point, moving)| PositionFact { point, moving }),
    }
}

/// `15 <entity> <count> <points>`, the `Moved` oracle.
fn moved_oracle(entity: u64, points: &[Point]) -> Vec<u8> {
    let mut out = cat(&[&[0x15], &varint(entity), &varint(points.len() as u64)]);
    for point in points {
        out.extend(pt(*point));
    }
    out
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

/// `bytes` with its first byte replaced.
fn with_version(bytes: &[u8], version: u8) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out[0] = version;
    out
}

fn v3_receipt(seq: u64, status: v3::ReceiptStatus) -> v3::DeltaOp {
    v3::DeltaOp::Receipt {
        epoch: Z16,
        lane: 0,
        seq,
        processed_tick: 100,
        status,
    }
}

fn action_resolved(outcome: &str) -> v3::DeltaOp {
    v3::DeltaOp::ActionResolved {
        actor: net(1),
        target: net(2),
        ability: u(1),
        outcome: outcome.to_owned(),
        damage: 7,
    }
}

/// W-3's identity for one `03`-prefixed intent byte string: the v4 decoder on
/// the `04` form gives the v3 decoder's error, or the same frame converted.
fn assert_intent_matches_v3(three: &[u8]) {
    assert_eq!(three[0], 3);
    let four = with_version(three, 4);
    match codec_v3::decode_intent(three) {
        Ok(f) => assert_eq!(
            decode_intent(&four),
            Ok(IntentFrame {
                epoch: f.epoch,
                lane: f.lane,
                seq: f.seq,
                observed_tick: f.observed_tick,
                actor: f.actor,
                body: f.body.into(),
            }),
            "{three:?}"
        ),
        Err(e) => assert_eq!(decode_intent(&four), Err(e), "{three:?}"),
    }
}

/// W-8's identity for one `03`-prefixed delta byte string.
fn assert_delta_matches_v3(three: &[u8]) {
    assert_eq!(three[0], 3);
    let four = with_version(three, 4);
    match codec_v3::decode_delta(three) {
        Ok(f) => assert_eq!(
            decode_delta(&four),
            Ok(DeltaFrame {
                lane: f.lane,
                server_tick: f.server_tick,
                event_seq: f.event_seq,
                ops: f.ops.into_iter().map(DeltaOp::V3).collect(),
            }),
            "{three:?}"
        ),
        Err(e) => assert_eq!(decode_delta(&four), Err(e), "{three:?}"),
    }
}

/// W-8's encode half for one v3 frame: the v4 encoding of the wrapped ops is
/// the v3 encoding with byte 0 changed 3 -> 4, or the same error.
fn assert_encode_matches_v3(frame: &v3::DeltaFrame) {
    let wrapped = DeltaFrame {
        lane: frame.lane,
        server_tick: frame.server_tick,
        event_seq: frame.event_seq,
        ops: frame.ops.iter().cloned().map(DeltaOp::V3).collect(),
    };
    match (codec_v3::encode_delta(frame), encode_delta(&wrapped)) {
        (Ok(three), Ok(four)) => {
            assert_eq!(three[0], 3);
            assert_eq!(four[0], 4);
            assert_eq!(&three[1..], &four[1..]);
            assert_eq!(decode_delta(&four), Ok(wrapped));
        }
        (Err(e), got) => assert_eq!(got, Err(e)),
        (Ok(_), Err(e)) => panic!("v4 refused what v3 encodes: {e:?}"),
    }
}

fn v3_frame(ops: Vec<v3::DeltaOp>) -> v3::DeltaFrame {
    v3::DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 100,
        event_seq: 1,
        ops,
    }
}

// ---------------------------------------------------------------------------
// The v3 op corpus (T140 A4, A6, A7, B1, B2, B3).
// ---------------------------------------------------------------------------

fn clock(paused: Option<Option<v3::PauseReason>>) -> v3::DeltaOp {
    v3::DeltaOp::Clock { paused }
}

fn host(
    pause: bool,
    unpause: bool,
    auto_pause: Option<v3::AutoPauseToggles>,
    turn_timer_ms: Option<Option<u32>>,
) -> v3::DeltaOp {
    v3::DeltaOp::HostFacts(v3::HostFacts {
        rights: v3::PauseRights { pause, unpause },
        auto_pause,
        turn_timer_ms,
    })
}

fn timeline(entries: &[(i32, u64)]) -> v3::DeltaOp {
    v3::DeltaOp::Timeline {
        entries: Some(
            entries
                .iter()
                .map(|&(key, entity)| v3::TimelineEntry {
                    key,
                    entity: net(entity),
                })
                .collect(),
        ),
    }
}

fn full_fact_set() -> v3::FactSet {
    v3::FactSet {
        pools: Some(vec![v3::PoolFact {
            pool: u(1),
            current: 2,
            max: 3,
        }]),
        effects: Some(vec![v3::EffectFact {
            effect: u(2),
            expires_round: 4,
        }]),
        order: Some(Some(v3::OrderFact {
            ability: u(3),
            target: net(1),
            repeat: true,
        })),
        stats: Some(vec![
            v3::StatFact {
                name: "might".to_owned(),
                value: v3::StatValue::Int(4),
            },
            v3::StatFact {
                name: "reach".to_owned(),
                value: v3::StatValue::Fixed(98_304),
            },
        ]),
        tags: Some(vec!["undead".to_owned()]),
    }
}

/// B3's 123-byte `EntityFacts` tail.
fn full_fact_bytes() -> Vec<u8> {
    cat(&[
        &[0x0D, 2],
        &[1, 1],
        &ulid_wire(1),
        &[2, 3],
        &[1, 1],
        &ulid_wire(2),
        &[4],
        &[1, 1],
        &ulid_wire(3),
        &[1, 1],
        &[1, 2, 5],
        b"might",
        &[0, 8],
        &[5],
        b"reach",
        &[1, 0x80, 0x80, 0x0C],
        &[1, 1, 6],
        b"undead",
    ])
}

fn encounter_full() -> v3::EncounterFacts {
    v3::EncounterFacts {
        mode: Some(EncounterMode::TurnBased),
        pending_mode: Some(Some(EncounterMode::RealTime)),
        clock: Some(Some(v3::ClockFact {
            round_ticks: 120,
            elapsed: 240,
        })),
    }
}

/// Every v3 op that has a hand-written literal tail, with that tail.
fn v3_literal_ops() -> Vec<(v3::DeltaOp, Vec<u8>)> {
    let mut action = vec![8u8, 1, 2];
    action.extend(ulid_wire(1));
    action.push(7);
    action.extend(b"success");
    action.push(7);
    let mut ended = vec![10u8];
    ended.extend(ulid_wire(1));
    ended.push(3);
    let receipt_tail = |status: &[u8]| cat(&[&[7], &Z16, &[0, 1, 0x64], status]);
    vec![
        // A4: the tags 0..=10.
        (v3::DeltaOp::EntityEnter { entity: net(1) }, vec![0, 1]),
        (v3::DeltaOp::EntityLeave { entity: net(1) }, vec![1, 1]),
        (v3::DeltaOp::Spawned { entity: net(1) }, vec![2, 1]),
        (v3::DeltaOp::Despawned { entity: net(1) }, vec![3, 1]),
        (v3::DeltaOp::Died { entity: net(1) }, vec![4, 1]),
        (
            v3::DeltaOp::Health {
                entity: net(1),
                health: 10,
                max_health: 10,
                dead: false,
            },
            vec![5, 1, 10, 10, 0],
        ),
        (
            v3::DeltaOp::Turn {
                active: None,
                round: 5,
            },
            vec![6, 0, 5],
        ),
        (
            v3::DeltaOp::Turn {
                active: Some(net(1)),
                round: 5,
            },
            vec![6, 1, 1, 5],
        ),
        (
            v3_receipt(1, v3::ReceiptStatus::Applied),
            receipt_tail(&[0]),
        ),
        (
            v3_receipt(
                1,
                v3::ReceiptStatus::Rejected(v3::RejectionCode::ModeUnsupported),
            ),
            receipt_tail(&[1, 0x15]),
        ),
        (
            v3_receipt(
                1,
                v3::ReceiptStatus::Rejected(v3::RejectionCode::NoPauseRight),
            ),
            receipt_tail(&[1, 0x1E]),
        ),
        (action_resolved("success"), action),
        (
            v3::DeltaOp::TurnStarted {
                actor: net(1),
                round: 3,
            },
            vec![9, 1, 3],
        ),
        (
            v3::DeltaOp::EncounterEnded {
                encounter: u(1),
                round: 3,
            },
            ended,
        ),
        // A6.
        (v3::DeltaOp::RoundStarted { round: 3 }, vec![0x0E, 3]),
        (
            v3::DeltaOp::ModeChanged {
                mode: EncounterMode::RealTime,
                round: 2,
            },
            vec![0x0F, 1, 2],
        ),
        (v3::DeltaOp::OrderEnded { actor: net(2) }, vec![0x10, 2]),
        (v3::DeltaOp::Joined { entity: net(2) }, vec![0x11, 2]),
        (v3::DeltaOp::Left { entity: net(2) }, vec![0x12, 2]),
        (
            v3::DeltaOp::RoundStarted { round: u64::MAX },
            cat(&[&[0x0E], &[0xFF; 9], &[0x01]]),
        ),
        // A7.
        (clock(None), vec![0x13, 0]),
        (clock(Some(None)), vec![0x13, 1, 0]),
        (
            clock(Some(Some(v3::PauseReason::Player))),
            vec![0x13, 1, 1, 0],
        ),
        (
            clock(Some(Some(v3::PauseReason::Auto(
                v3::AutoPauseTrigger::OrderEnded,
            )))),
            vec![0x13, 1, 1, 1, 3],
        ),
        (
            clock(Some(Some(v3::PauseReason::Operator))),
            vec![0x13, 1, 1, 2],
        ),
        (
            clock(Some(Some(v3::PauseReason::Restore))),
            vec![0x13, 1, 1, 3],
        ),
        (host(true, true, None, None), vec![0x14, 1, 1, 0, 0]),
        (
            host(
                true,
                true,
                Some(v3::AutoPauseToggles::from_bits(0x1F).unwrap()),
                Some(Some(30_000)),
            ),
            vec![0x14, 1, 1, 1, 0x1F, 1, 1, 0xB0, 0xEA, 0x01],
        ),
        (
            host(true, true, None, Some(None)),
            vec![0x14, 1, 1, 0, 1, 0],
        ),
        (
            host(true, true, None, Some(Some(u32::MAX))),
            vec![0x14, 1, 1, 0, 1, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        ),
        // B1.
        (
            v3::DeltaOp::Encounter(v3::EncounterFacts::default()),
            vec![0x0B, 0, 0, 0],
        ),
        (
            v3::DeltaOp::Encounter(encounter_full()),
            vec![0x0B, 1, 0, 1, 1, 1, 1, 1, 0x78, 0xF0, 0x01],
        ),
        // B2.
        (v3::DeltaOp::Timeline { entries: None }, vec![0x0C, 0]),
        (
            v3::DeltaOp::Timeline {
                entries: Some(vec![]),
            },
            vec![0x0C, 1, 0],
        ),
        (
            timeline(&[(-1, 2), (-1, 1), (5, 3)]),
            vec![0x0C, 1, 3, 1, 2, 1, 1, 0x0A, 3],
        ),
        (
            timeline(&[(i32::MIN, 2), (i32::MAX, 3)]),
            vec![
                0x0C, 1, 2, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 2, 0xFE, 0xFF, 0xFF, 0xFF, 0x0F, 3,
            ],
        ),
        // B3.
        (
            v3::DeltaOp::EntityFacts {
                entity: net(2),
                facts: v3::FactSet::default(),
            },
            vec![0x0D, 2, 0, 0, 0, 0, 0],
        ),
        (
            v3::DeltaOp::EntityFacts {
                entity: net(2),
                facts: full_fact_set(),
            },
            full_fact_bytes(),
        ),
    ]
}

/// Every v3 tag, 0..=20, at least once, and every rejection code 1..=30.
fn v3_round_trip_ops() -> Vec<v3::DeltaOp> {
    let mut ops: Vec<v3::DeltaOp> = v3_literal_ops().into_iter().map(|(op, _)| op).collect();
    for raw in 1..=30u8 {
        let code = v3::RejectionCode::from_u8(raw).expect("v3 code");
        ops.push(v3_receipt(1, v3::ReceiptStatus::Rejected(code)));
    }
    ops.extend([
        v3::DeltaOp::Health {
            entity: net(6),
            health: 0,
            max_health: u32::MAX,
            dead: true,
        },
        v3::DeltaOp::Turn {
            active: Some(net(u64::MAX)),
            round: u64::MAX,
        },
        v3_receipt(
            SEQ_LAST,
            v3::ReceiptStatus::Rejected(v3::RejectionCode::NoEntrants),
        ),
        action_resolved("critical_success"),
        action_resolved("custom:255"),
        v3::DeltaOp::ModeChanged {
            mode: EncounterMode::TurnBased,
            round: 9,
        },
        host(
            false,
            true,
            Some(v3::AutoPauseToggles::default()),
            Some(None),
        ),
        v3::DeltaOp::Encounter(v3::EncounterFacts {
            mode: None,
            pending_mode: Some(None),
            clock: Some(None),
        }),
    ]);
    ops
}

// ---------------------------------------------------------------------------
// W1 v4_version_gate
// ---------------------------------------------------------------------------

#[test]
fn w1_v4_version_gate() {
    // EndTurn: 22 bytes, the v3 bytes with byte 0 changed 3 -> 4.
    let end_turn = intent(IntentBody::EndTurn);
    let v4 = encode_intent(&end_turn).unwrap();
    assert_eq!(v4, cat(&[&[4], &Z16, &[0, 1, 7, 1, 1]]));
    assert_eq!(v4.len(), 22);
    let v1_frame = IntentFrameV1 {
        epoch: Z16,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 7,
        actor: net(1),
        body: IntentBodyV1::EndTurn,
    };
    let one = encode_intent_v1(&v1_frame).unwrap();
    let two = encode_intent_v2(&v1_frame).unwrap();
    let three = codec_v3::encode_intent(&v3_intent(v3::IntentBody::EndTurn)).unwrap();
    assert_eq!((one[0], two[0], three[0]), (1, 2, 3));
    assert_eq!(&three[1..], &v4[1..]);

    // v4 refuses every other version byte (the cap and emptiness aside).
    assert_eq!(decode_intent(&one), Err(CodecError::UnsupportedVersion));
    assert_eq!(decode_intent(&two), Err(CodecError::UnsupportedVersion));
    assert_eq!(decode_intent(&three), Err(CodecError::UnsupportedVersion));
    for first in [0x81u8, 0x82, 0x91, 0xA1] {
        assert_eq!(decode_intent(&[first]), Err(CodecError::UnsupportedVersion));
    }
    assert_eq!(decode_intent(&[]), Err(CodecError::Malformed));
    // Control: its own bytes decode.
    assert_eq!(decode_intent(&v4), Ok(end_turn));

    // v1, v2 and v3 refuse v4 bytes.
    assert_eq!(decode_intent_v1(&v4), Err(CodecError::UnsupportedVersion));
    assert_eq!(decode_intent_v2(&v4), Err(CodecError::UnsupportedVersion));
    assert_eq!(
        codec_v3::decode_intent(&v4),
        Err(CodecError::UnsupportedVersion)
    );
    let empty_v4 = [4u8, 0, 0x64, 1, 0];
    assert_eq!(
        decode_delta_v1(&empty_v4),
        Err(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        decode_delta_v2(&empty_v4),
        Err(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        codec_v3::decode_delta(&empty_v4),
        Err(CodecError::UnsupportedVersion)
    );

    // Delta side of the gate, each foreign first byte with a valid remainder.
    for first in [1u8, 2, 3, 0x81, 0x82, 0x91, 0xA1] {
        assert_eq!(
            decode_delta(&with_version(&empty_v4, first)),
            Err(CodecError::UnsupportedVersion)
        );
    }
    assert_eq!(decode_delta(&[]), Err(CodecError::Malformed));
    assert_eq!(decode_delta(&empty_v4).unwrap(), delta(vec![]));

    // The frame cap runs before the version check: 4,097 / 65,537 bytes with
    // first byte 03 are FrameTooLarge, not UnsupportedVersion.
    let mut big_intent = vec![3u8];
    big_intent.resize(MAX_INTENT_FRAME_BYTES + 1, 0);
    assert_eq!(decode_intent(&big_intent), Err(CodecError::FrameTooLarge));
    let mut big_delta = vec![3u8];
    big_delta.resize(MAX_DELTA_FRAME_BYTES + 1, 0);
    assert_eq!(decode_delta(&big_delta), Err(CodecError::FrameTooLarge));
    // Controls: exactly at the cap the version byte decides.
    let mut at_cap_intent = vec![3u8];
    at_cap_intent.resize(MAX_INTENT_FRAME_BYTES, 0);
    assert_eq!(
        decode_intent(&at_cap_intent),
        Err(CodecError::UnsupportedVersion)
    );
    let mut at_cap_delta = vec![3u8];
    at_cap_delta.resize(MAX_DELTA_FRAME_BYTES, 0);
    assert_eq!(
        decode_delta(&at_cap_delta),
        Err(CodecError::UnsupportedVersion)
    );
    // A 4,096-byte v4 EndTurn padded with 00 is Malformed (trailing bytes),
    // so the cap is not what refused it; one byte more is FrameTooLarge.
    let mut padded_intent = v4.clone();
    padded_intent.resize(MAX_INTENT_FRAME_BYTES, 0);
    assert_eq!(decode_intent(&padded_intent), Err(CodecError::Malformed));
    padded_intent.push(0);
    assert_eq!(
        decode_intent(&padded_intent),
        Err(CodecError::FrameTooLarge)
    );
    let mut padded_delta = empty_v4.to_vec();
    padded_delta.resize(MAX_DELTA_FRAME_BYTES, 0);
    assert_eq!(decode_delta(&padded_delta), Err(CodecError::Malformed));
    padded_delta.push(0);
    assert_eq!(decode_delta(&padded_delta), Err(CodecError::FrameTooLarge));
}

// ---------------------------------------------------------------------------
// W2 v4_intent_v3_bodies
// ---------------------------------------------------------------------------

#[test]
fn w2_v4_intent_v3_bodies() {
    let bodies = [
        v3::IntentBody::EndTurn,
        v3::IntentBody::Stop,
        v3::IntentBody::Pause,
        v3::IntentBody::Unpause,
        v3::IntentBody::RequestMode {
            mode: EncounterMode::TurnBased,
        },
        v3::IntentBody::RequestMode {
            mode: EncounterMode::RealTime,
        },
        v3::IntentBody::DeclareAction {
            ability: u(1),
            target: net(2),
        },
    ];
    for body in bodies {
        let frame = v3_intent(body);
        let three = codec_v3::encode_intent(&frame).unwrap();
        let converted = IntentFrame {
            epoch: frame.epoch,
            lane: frame.lane,
            seq: frame.seq,
            observed_tick: frame.observed_tick,
            actor: frame.actor,
            body: IntentBody::from(body),
        };
        let four = encode_intent(&converted).unwrap();
        assert_eq!(four, with_version(&three, 4), "{body:?}");
        assert_eq!(decode_intent(&four), Ok(converted));
        assert_intent_matches_v3(&three);
    }
    // The literal bodies, by hand.
    for (body, tail) in [
        (IntentBody::EndTurn, vec![1u8]),
        (IntentBody::Stop, vec![2]),
        (IntentBody::Pause, vec![3]),
        (IntentBody::Unpause, vec![4]),
        (
            IntentBody::RequestMode {
                mode: EncounterMode::TurnBased,
            },
            vec![5, 0],
        ),
        (
            IntentBody::RequestMode {
                mode: EncounterMode::RealTime,
            },
            vec![5, 1],
        ),
    ] {
        assert_eq!(encode_intent(&intent(body)).unwrap(), intent_bytes(&tail));
    }
    let declare = encode_intent(&intent(IntentBody::DeclareAction {
        ability: u(1),
        target: net(2),
    }))
    .unwrap();
    assert_eq!(declare, intent_bytes(&cat(&[&[0], &ulid_wire(1), &[2]])));
    assert_eq!(declare.len(), 50);

    // The v3 refusals, each with the `04` form giving the v3 error and a
    // control with the good byte decoding Ok. (bad v3 bytes, control, error)
    let target_zero = hdr(3, 0, &[1], &[1], &cat(&[&[0], &ulid_wire(1), &[0]]));
    let target_two = hdr(3, 0, &[1], &[1], &cat(&[&[0], &ulid_wire(1), &[2]]));
    let mut trailing_declare = target_two.clone();
    trailing_declare.push(0);
    let max_seq = varint(u64::MAX);
    let last_seq = varint(SEQ_LAST);
    let refusals: Vec<(Vec<u8>, Vec<u8>, CodecError)> = vec![
        // Body tag 07 and FF.
        (
            hdr(3, 0, &[1], &[1], &[7]),
            hdr(3, 0, &[1], &[1], &[2]),
            CodecError::UnknownMessage,
        ),
        (
            hdr(3, 0, &[1], &[1], &[0xFF]),
            hdr(3, 0, &[1], &[1], &[2]),
            CodecError::UnknownMessage,
        ),
        // Mode byte 02 and FF.
        (
            hdr(3, 0, &[1], &[1], &[5, 2]),
            hdr(3, 0, &[1], &[1], &[5, 1]),
            CodecError::UnknownMessage,
        ),
        (
            hdr(3, 0, &[1], &[1], &[5, 0xFF]),
            hdr(3, 0, &[1], &[1], &[5, 0]),
            CodecError::UnknownMessage,
        ),
        // Lane 01, with a complete valid remainder.
        (
            hdr(3, 1, &[1], &[1], &[1]),
            hdr(3, 0, &[1], &[1], &[1]),
            CodecError::WrongDirection,
        ),
        // seq 0 and u64::MAX; SEQ_LAST is the control.
        (
            hdr(3, 0, &[0], &[1], &[1]),
            hdr(3, 0, &last_seq, &[1], &[1]),
            CodecError::Malformed,
        ),
        (
            hdr(3, 0, &max_seq, &[1], &[1]),
            hdr(3, 0, &last_seq, &[1], &[1]),
            CodecError::Malformed,
        ),
        // actor 0.
        (
            hdr(3, 0, &[1], &[0], &[1]),
            hdr(3, 0, &[1], &[1], &[1]),
            CodecError::Malformed,
        ),
        // DeclareAction target 0.
        (target_zero, target_two.clone(), CodecError::Malformed),
        // ULID length prefix of 257 (81 02) with no body.
        (
            hdr(3, 0, &[1], &[1], &[0, 0x81, 0x02]),
            target_two.clone(),
            CodecError::LimitExceeded,
        ),
        // One trailing byte.
        (
            hdr(3, 0, &[1], &[1], &[1, 0]),
            hdr(3, 0, &[1], &[1], &[1]),
            CodecError::Malformed,
        ),
        (trailing_declare, target_two, CodecError::Malformed),
    ];
    for (bad, control, expected) in refusals {
        assert_eq!(codec_v3::decode_intent(&bad), Err(expected), "v3 {bad:?}");
        assert_eq!(
            decode_intent(&with_version(&bad, 4)),
            Err(expected),
            "{bad:?}"
        );
        assert!(
            decode_intent(&with_version(&control, 4)).is_ok(),
            "control {control:?}"
        );
    }
    assert_eq!(SEQ_LAST, u64::MAX - 1);
    assert_eq!(
        decode_intent(&hdr(4, 0, &last_seq, &[1], &[1]))
            .unwrap()
            .seq,
        SEQ_LAST
    );

    // Two faults, first wins (v4 bytes). Each fault alone gives a different
    // error, so the order is observable.
    let two_faults: Vec<(Vec<u8>, CodecError)> = vec![
        (hdr(4, 1, &[0], &[1], &[1]), CodecError::WrongDirection),
        (hdr(4, 0, &[0], &[1], &[0xFF]), CodecError::Malformed),
        (hdr(4, 0, &[1], &[0], &[0xFF]), CodecError::Malformed),
        (
            hdr(4, 0, &[1], &[1], &[0xFF, 0]),
            CodecError::UnknownMessage,
        ),
        (
            hdr(4, 0, &[1], &[1], &[5, 2, 0]),
            CodecError::UnknownMessage,
        ),
    ];
    for (bytes, expected) in two_faults {
        assert_eq!(decode_intent(&bytes), Err(expected), "{bytes:?}");
    }
    // Lane is checked before seq on encode as well.
    let mut frame = intent(IntentBody::EndTurn);
    frame.lane = 1;
    frame.seq = 0;
    assert_eq!(encode_intent(&frame), Err(CodecError::WrongDirection));
    frame.lane = 0;
    assert_eq!(encode_intent(&frame), Err(CodecError::Malformed));
    frame.seq = u64::MAX;
    assert_eq!(encode_intent(&frame), Err(CodecError::Malformed));
    frame.seq = SEQ_LAST;
    assert!(encode_intent(&frame).is_ok());

    // W-3's second half over a corpus: every prefix, trailing bytes and
    // single-byte mutations of valid v3 frames. The mutation values and the
    // base frames avoid byte 6, so no decoder can reach body tag 6.
    let mut corpus: Vec<Vec<u8>> = Vec::new();
    for body in bodies {
        let base = codec_v3::encode_intent(&v3_intent(body)).unwrap();
        for cut in 0..=base.len() {
            if cut > 0 {
                corpus.push(base[..cut].to_vec());
            }
        }
        for extra in [0u8, 1, 6, 0xFF] {
            let mut longer = base.clone();
            longer.push(extra);
            corpus.push(longer);
        }
        for index in 1..base.len() {
            for value in [0x00u8, 0x01, 0x02, 0x05, 0x07, 0x1A, 0x7F, 0x80, 0xFF] {
                let mut mutated = base.clone();
                mutated[index] = value;
                corpus.push(mutated);
            }
        }
    }
    for three in &corpus {
        // A byte 6 is only ever appended after a complete frame, where it is
        // a trailing byte, never the body tag.
        assert_intent_matches_v3(three);
    }
}

// ---------------------------------------------------------------------------
// W3 declare_move_literals_and_bounds
// ---------------------------------------------------------------------------

#[test]
fn w3_declare_move_literals_and_bounds() {
    let declare = |point: Point| intent(IntentBody::DeclareMove { destination: point });
    let b_bytes = encode_intent(&declare(pb())).unwrap();
    assert_eq!(
        b_bytes,
        cat(&[
            &[4],
            &Z16,
            &[0, 1, 7, 1, 6],
            &[0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F],
        ])
    );
    assert_eq!(b_bytes.len(), 29);
    round_trip_intent(declare(pb()));
    // The components are in x, y, z order.
    assert_eq!(
        encode_intent(&declare(p(1, 2, 3))).unwrap(),
        intent_bytes(&[6, 0x02, 0x04, 0x06])
    );
    assert_eq!(
        encode_intent(&declare(p(-1, -2, -3))).unwrap(),
        intent_bytes(&[6, 0x01, 0x03, 0x05])
    );

    let plus_l: &[u8] = &[0x80, 0x80, 0x80, 0x80, 0x04];
    let minus_l: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF, 0x03];
    let over_plus: &[u8] = &[0x82, 0x80, 0x80, 0x80, 0x04];
    let over_minus: &[u8] = &[0x81, 0x80, 0x80, 0x80, 0x04];
    let min: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
    let overflow: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF, 0x1F];
    assert_eq!(zz(L), plus_l);
    assert_eq!(zz(-L), minus_l);
    assert_eq!(zz(L + 1), over_plus);
    assert_eq!(zz(-L - 1), over_minus);
    assert_eq!(zz(i32::MIN), min);
    // Each coordinate position, the other two `00`.
    let at = |slot: usize, bytes: &[u8]| -> Vec<u8> {
        let mut tail = vec![6u8];
        for position in 0..3 {
            if position == slot {
                tail.extend_from_slice(bytes);
            } else {
                tail.push(0);
            }
        }
        intent_bytes(&tail)
    };
    for slot in 0..3 {
        // Controls: both limits are accepted.
        for (bytes, value) in [(plus_l, L), (minus_l, -L)] {
            let mut raw = [0i32; 3];
            raw[slot] = value;
            let decoded = decode_intent(&at(slot, bytes)).expect("limit accepted");
            assert_eq!(
                decoded.body,
                IntentBody::DeclareMove {
                    destination: p(raw[0], raw[1], raw[2])
                }
            );
            assert_eq!(encode_intent(&decoded).unwrap(), at(slot, bytes));
        }
        // One past each limit, i32::MIN, and an i32 overflow are Malformed.
        for bad in [over_plus, over_minus, min, overflow] {
            assert_eq!(
                decode_intent(&at(slot, bad)),
                Err(CodecError::Malformed),
                "slot {slot} {bad:?}"
            );
        }
    }
    // A bad coordinate is Malformed before the next one is read: even a
    // valid remainder cannot rescue it, and a good one is the control.
    assert_eq!(decode_intent(&at(0, over_plus)), Err(CodecError::Malformed));
    // Missing z and one trailing byte are Malformed; the full form is the
    // control.
    assert!(decode_intent(&intent_bytes(&[6, 0, 0, 0])).is_ok());
    assert_eq!(
        decode_intent(&intent_bytes(&[6, 0, 0])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_intent(&intent_bytes(&[6])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_intent(&intent_bytes(&[6, 0, 0, 0, 0])),
        Err(CodecError::Malformed)
    );
    // Truncation anywhere is Malformed, never a panic.
    for cut in 0..b_bytes.len() {
        assert_eq!(
            decode_intent(&b_bytes[..cut]),
            Err(CodecError::Malformed),
            "cut {cut}"
        );
    }

    // Point::new and raw.
    assert_eq!(Point::new([L, -L, 0]).map(Point::raw), Some([L, -L, 0]));
    assert_eq!(Point::new([L + 1, 0, 0]), None);
    assert_eq!(Point::new([0, -L - 1, 0]), None);
    assert_eq!(Point::new([0, 0, i32::MIN]), None);
    assert_eq!(Point::new([i32::MAX, 0, 0]), None);
    assert_eq!(Point::new([-L, -L, -L]).map(Point::raw), Some([-L, -L, -L]));
    assert_eq!(pb().raw(), [65_536, 0, -131_072]);

    // Encode refuses lane 1 with WrongDirection and seq 0 with Malformed.
    let mut frame = declare(pb());
    frame.lane = 1;
    assert_eq!(encode_intent(&frame), Err(CodecError::WrongDirection));
    frame.lane = 0;
    frame.seq = 0;
    assert_eq!(encode_intent(&frame), Err(CodecError::Malformed));
    frame.seq = 1;
    assert!(encode_intent(&frame).is_ok());
}

// ---------------------------------------------------------------------------
// W4 v4_v3_ops_identical
// ---------------------------------------------------------------------------

#[test]
fn w4_v4_v3_ops_identical() {
    // Every literal op: the hand-written tail, the v3 differential, the
    // decode back to `V3(op)`.
    for (op, tail) in v3_literal_ops() {
        assert_eq!(one_op(wrap(op.clone())), delta_bytes(1, &tail), "{op:?}");
        assert_eq!(dec_ok(&tail), wrap(op.clone()));
        round_trip_op(wrap(op.clone()));
        assert_encode_matches_v3(&v3_frame(vec![op]));
    }
    // The contract's named oracle.
    assert_eq!(
        one_op(wrap(v3::DeltaOp::Turn {
            active: None,
            round: 5
        })),
        vec![0x04, 0x00, 0x64, 0x01, 0x01, 0x06, 0x00, 0x05]
    );
    assert_eq!(full_fact_bytes().len(), 123);

    // The whole corpus, every code 1..=30, as one frame and one op each.
    let corpus = v3_round_trip_ops();
    for op in &corpus {
        assert_encode_matches_v3(&v3_frame(vec![op.clone()]));
        round_trip_op(wrap(op.clone()));
    }
    for chunk in corpus.chunks(64) {
        assert_encode_matches_v3(&v3_frame(chunk.to_vec()));
    }
    // From wraps.
    assert_eq!(
        DeltaOp::from(v3::DeltaOp::Died { entity: net(4) }),
        DeltaOp::V3(v3::DeltaOp::Died { entity: net(4) })
    );

    // v3 encode refusals are the same refusals in v4.
    let bad_frames = vec![
        v3_frame(vec![v3_receipt(0, v3::ReceiptStatus::Applied)]),
        v3_frame(vec![v3::DeltaOp::Receipt {
            epoch: Z16,
            lane: 1,
            seq: 1,
            processed_tick: 100,
            status: v3::ReceiptStatus::Applied,
        }]),
        v3_frame(vec![action_resolved("bogus")]),
        v3_frame(vec![action_resolved("custom:256")]),
        v3_frame(vec![action_resolved(&"a".repeat(257))]),
        v3_frame(vec![v3::DeltaOp::Encounter(v3::EncounterFacts {
            clock: Some(Some(v3::ClockFact {
                round_ticks: 0,
                elapsed: 0,
            })),
            ..v3::EncounterFacts::default()
        })]),
        v3_frame(vec![timeline(&[(5, 1), (4, 2)])]),
        v3_frame(vec![timeline(&[(1, 1), (2, 1)])]),
        v3_frame(vec![v3::DeltaOp::EntityFacts {
            entity: net(2),
            facts: v3::FactSet {
                tags: Some(vec!["b".to_owned(), "a".to_owned()]),
                ..v3::FactSet::default()
            },
        }]),
        v3_frame(vec![v3::DeltaOp::EntityFacts {
            entity: net(2),
            facts: v3::FactSet {
                pools: Some(vec![v3::PoolFact {
                    pool: u(1),
                    current: 4,
                    max: 3,
                }]),
                ..v3::FactSet::default()
            },
        }]),
    ];
    for frame in &bad_frames {
        assert!(codec_v3::encode_delta(frame).is_err());
        assert_encode_matches_v3(frame);
    }

    // These v3 refusals, with byte 0 set to 04, give the v3 error. Each has a
    // control with the good byte, and the v3 form is checked too.
    let receipt = |lane: u8, code: u8| cat(&[&[7], &Z16, &[lane, 1, 0x64], &[1, code]]);
    let refusals: Vec<(Vec<u8>, Vec<u8>, CodecError)> = vec![
        (
            receipt(0, 0x1F),
            receipt(0, 0x1E),
            CodecError::UnknownMessage,
        ),
        (
            receipt(1, 0x1E),
            receipt(0, 0x1E),
            CodecError::WrongDirection,
        ),
        (
            vec![0x0B, 1, 2, 0, 0],
            vec![0x0B, 1, 1, 0, 0],
            CodecError::UnknownMessage,
        ),
        (
            vec![0x0C, 1, 0x81, 0x08],
            vec![0x0C, 1, 1, 0, 1],
            CodecError::LimitExceeded,
        ),
        (
            vec![0x0D, 2, 0, 0, 0, 1, 0x21],
            vec![0x0D, 2, 0, 0, 0, 1, 0, 0],
            CodecError::LimitExceeded,
        ),
        (
            vec![0x13, 1, 1, 4],
            vec![0x13, 1, 1, 3],
            CodecError::UnknownMessage,
        ),
        (
            vec![0x14, 1, 1, 1, 0x20, 0],
            vec![0x14, 1, 1, 1, 0x1F, 0],
            CodecError::Malformed,
        ),
    ];
    for (bad, control, expected) in refusals {
        let three = cat(&[&[3, 0, 0x64, 1, 1], &bad]);
        assert_eq!(codec_v3::decode_delta(&three), Err(expected), "v3 {bad:?}");
        assert_eq!(dec_err(&bad), expected, "{bad:?}");
        assert!(
            decode_delta(&delta_bytes(1, &control)).is_ok(),
            "control {control:?}"
        );
    }

    // W-8's decode half over a corpus of valid v3 frames: every prefix,
    // trailing bytes and single-byte mutations. The mutation values avoid
    // 0x15 and 0x16, and the base frames contain neither byte, so no decoder
    // can reach op tag 21 or 22.
    let base_ops = vec![
        v3::DeltaOp::EntityEnter { entity: net(1) },
        v3::DeltaOp::Health {
            entity: net(1),
            health: 10,
            max_health: 10,
            dead: false,
        },
        v3::DeltaOp::Turn {
            active: Some(net(1)),
            round: 5,
        },
        v3_receipt(1, v3::ReceiptStatus::Applied),
        action_resolved("success"),
        v3::DeltaOp::ModeChanged {
            mode: EncounterMode::RealTime,
            round: 2,
        },
        clock(Some(Some(v3::PauseReason::Auto(
            v3::AutoPauseTrigger::OrderEnded,
        )))),
        host(true, true, None, Some(Some(30_000))),
        v3::DeltaOp::Encounter(encounter_full()),
        timeline(&[(-1, 2), (-1, 1), (5, 3)]),
        v3::DeltaOp::EntityFacts {
            entity: net(2),
            facts: full_fact_set(),
        },
    ];
    let base = codec_v3::encode_delta(&v3_frame(base_ops)).unwrap();
    assert!(!base.contains(&0x15) && !base.contains(&0x16));
    let mut corpus: Vec<Vec<u8>> = Vec::new();
    for cut in 1..=base.len() {
        corpus.push(base[..cut].to_vec());
    }
    for extra in [0u8, 1, 0xFF] {
        let mut longer = base.clone();
        longer.push(extra);
        corpus.push(longer);
    }
    for index in 1..base.len() {
        for value in [0x00u8, 0x01, 0x02, 0x04, 0x05, 0x07, 0x1A, 0x7F, 0x80, 0xFF] {
            let mut mutated = base.clone();
            mutated[index] = value;
            corpus.push(mutated);
        }
    }
    for three in &corpus {
        assert_delta_matches_v3(three);
    }
    assert_delta_matches_v3(&base);
}

// ---------------------------------------------------------------------------
// W5 moved_literals_and_bounds
// ---------------------------------------------------------------------------

#[test]
fn w5_moved_literals_and_bounds() {
    let two = [pa(), p(65_536, 0, 0)];
    let literal = vec![
        0x15, 0x02, 0x02, 0x00, 0x00, 0x00, 0x80, 0x80, 0x08, 0x00, 0x00,
    ];
    assert_eq!(literal.len(), 11);
    assert_eq!(moved_oracle(2, &two), literal);
    assert_eq!(one_op(moved(2, &two)), delta_bytes(1, &literal));
    assert_eq!(dec_ok(&literal), moved(2, &two));
    round_trip_op(moved(2, &two));

    // P64 is accepted; for entity u64::MAX it is 972 bytes.
    let max_op = moved(u64::MAX, &p64());
    let max_bytes = one_op(max_op.clone());
    assert_eq!(max_bytes.len() - 5, 972);
    assert_eq!(&max_bytes[5..], moved_oracle(u64::MAX, &p64()).as_slice());
    round_trip_op(max_op);
    round_trip_op(moved(2, &p64()));

    // Count 65 (41) with no point following is LimitExceeded, before any
    // point; count 00 is Malformed; the controls are 64 (40) and 2.
    assert_eq!(dec_err(&[0x15, 0x02, 0x41]), CodecError::LimitExceeded);
    assert_eq!(dec_err(&[0x15, 0x02, 0x00]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x15, 0x02, 0x01]), CodecError::Malformed);
    assert_eq!(
        dec_err(&cat(&[&[0x15, 0x02], &[0x81, 0x01]])),
        CodecError::LimitExceeded
    );
    // A count varint that overflows u64 is Malformed; at 2^63 it is just big.
    let huge = |last: u8| cat(&[&[0x15, 0x02], &[0xFF; 9], &[last]]);
    assert_eq!(dec_err(&huge(0x02)), CodecError::Malformed);
    assert_eq!(dec_err(&huge(0x01)), CodecError::LimitExceeded);
    assert_eq!(dec_ok(&moved_oracle(2, &p64())).clone(), moved(2, &p64()));
    // Under 2 is refused before any point is read, so a valid point after
    // count 1 changes nothing; the two-op frame from the contract.
    assert_eq!(
        decode_delta(&cat(&[
            &[0x04, 0x00, 0x64, 0x01, 0x02],
            &[0x15, 0x02, 0x01, 0x00, 0x00, 0x00],
            &[0x0E, 0x03],
        ])),
        Err(CodecError::Malformed)
    );
    // Control: the same frame with count 2 and a second point decodes.
    assert_eq!(
        decode_delta(&cat(&[
            &[0x04, 0x00, 0x64, 0x01, 0x02],
            &[0x15, 0x02, 0x02, 0x00, 0x00, 0x00, 0x80, 0x80, 0x08, 0x00, 0x00],
            &[0x0E, 0x03],
        ])),
        Ok(delta(vec![
            moved(2, &two),
            wrap(v3::DeltaOp::RoundStarted { round: 3 }),
        ]))
    );

    // Zero horizontal segments: equal x and z is Malformed, y may differ.
    let vertical = [pa(), p(0, 65_536, 0)];
    assert_eq!(
        dec_err(&[0x15, 0x02, 0x02, 0, 0, 0, 0, 0x80, 0x80, 0x08, 0]),
        CodecError::Malformed
    );
    assert_eq!(dec_err(&moved_oracle(2, &vertical)), CodecError::Malformed);
    let near = [pa(), p(0, 0, 1)];
    assert_eq!(
        dec_ok(&[0x15, 0x02, 0x02, 0, 0, 0, 0, 0, 0x02]),
        moved(2, &near)
    );
    round_trip_op(moved(2, &near));
    let repeat_x = [pa(), p(1, 0, 0), p(1, 5, 0)];
    assert_eq!(dec_err(&moved_oracle(2, &repeat_x)), CodecError::Malformed);
    // Only consecutive points count: a route that returns to an earlier
    // point, or runs vertically then sideways, is fine.
    let returns = [pa(), p(1, 0, 0), pa()];
    round_trip_op(moved(2, &returns));
    let zigzag = [pa(), p(1, 9, 0), p(2, 9, 1), p(2, 3, 2)];
    round_trip_op(moved(2, &zigzag));
    // The check also fires at later positions.
    let late = [pa(), p(1, 0, 0), p(2, 0, 0), p(2, 7, 0)];
    assert_eq!(dec_err(&moved_oracle(2, &late)), CodecError::Malformed);
    assert_eq!(try_one_op(moved(2, &late)), Err(CodecError::Malformed));
    assert_eq!(try_one_op(moved(2, &vertical)), Err(CodecError::Malformed));
    assert_eq!(try_one_op(moved(2, &repeat_x)), Err(CodecError::Malformed));
    // Entity 0 is Malformed (control: entity 2) and a second point whose x is
    // L + 1 is Malformed (control: x = L).
    let mut entity_zero = literal.clone();
    entity_zero[1] = 0;
    assert_eq!(dec_err(&entity_zero), CodecError::Malformed);
    let bad_x = cat(&[
        &[0x15, 0x02, 0x02, 0, 0, 0],
        &[0x82, 0x80, 0x80, 0x80, 0x04, 0x00, 0x02],
    ]);
    let good_x = cat(&[
        &[0x15, 0x02, 0x02, 0, 0, 0],
        &[0x80, 0x80, 0x80, 0x80, 0x04, 0x00, 0x02],
    ]);
    assert_eq!(dec_err(&bad_x), CodecError::Malformed);
    assert_eq!(dec_ok(&good_x), moved(2, &[pa(), p(L, 0, 1)]));
    // Truncation anywhere is Malformed.
    let whole = delta_bytes(1, &literal);
    for cut in 1..whole.len() {
        assert_eq!(
            decode_delta(&whole[..cut]),
            Err(CodecError::Malformed),
            "cut {cut}"
        );
    }
    // One trailing byte after a Moved.
    let mut trailing = whole.clone();
    trailing.push(0);
    assert_eq!(decode_delta(&trailing), Err(CodecError::Malformed));

    // check_points: counts, then zero segments; each equal to encode_delta's
    // error for a Moved with those points.
    assert_eq!(MAX_MOVED_POINTS, 64);
    assert_eq!(check_points(&[]), Err(CodecError::Malformed));
    assert_eq!(check_points(&[pa()]), Err(CodecError::Malformed));
    assert_eq!(check_points(&p65()), Err(CodecError::LimitExceeded));
    assert_eq!(check_points(&vertical), Err(CodecError::Malformed));
    assert_eq!(check_points(&repeat_x), Err(CodecError::Malformed));
    assert_eq!(check_points(&p64()), Ok(()));
    assert_eq!(check_points(&two), Ok(()));
    assert_eq!(check_points(&returns), Ok(()));
    for points in [vec![], vec![pa()], p65(), vertical.to_vec()] {
        assert_eq!(
            try_one_op(moved(2, &points)),
            check_points(&points).map(|()| unreachable!()),
            "{} points",
            points.len()
        );
    }
    assert!(try_one_op(moved(2, &p64())).is_ok());
    // Count is checked before the zero-segment rule: 65 equal points is a
    // LimitExceeded, not a Malformed.
    let flat: Vec<Point> = (0..65).map(|_| pa()).collect();
    assert_eq!(check_points(&flat), Err(CodecError::LimitExceeded));
    assert_eq!(try_one_op(moved(2, &flat)), Err(CodecError::LimitExceeded));
    // And an under-2 list is Malformed whatever else is true of it.
    assert_eq!(check_points(&[pa()]), Err(CodecError::Malformed));
}

// ---------------------------------------------------------------------------
// W6 position_literals
// ---------------------------------------------------------------------------

#[test]
fn w6_position_literals() {
    let disclosed = position(2, Some((pb(), true)));
    let tail = vec![
        0x16, 0x02, 0x01, 0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F, 0x01,
    ];
    assert_eq!(one_op(disclosed.clone()), delta_bytes(1, &tail));
    assert_eq!(dec_ok(&tail), disclosed);
    round_trip_op(disclosed);
    let still = position(2, Some((pb(), false)));
    let mut still_tail = tail.clone();
    *still_tail.last_mut().unwrap() = 0x00;
    assert_eq!(one_op(still.clone()), delta_bytes(1, &still_tail));
    assert_eq!(dec_ok(&still_tail), still);
    round_trip_op(still);
    let withheld = position(2, None);
    assert_eq!(
        one_op(withheld.clone()),
        delta_bytes(1, &[0x16, 0x02, 0x00])
    );
    assert_eq!(dec_ok(&[0x16, 0x02, 0x00]), withheld);
    round_trip_op(withheld);
    // Withheld and disclosed do not share bytes.
    assert_ne!(
        one_op(position(2, None)),
        one_op(position(2, Some((pa(), false))))
    );

    // A disclosed flag of 02 before a full point is Malformed; 01 is the
    // control.
    let flag = |byte: u8| {
        cat(&[
            &[0x16, 0x02, byte],
            &[0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F, 0x01],
        ])
    };
    assert!(decode_delta(&delta_bytes(1, &flag(1))).is_ok());
    assert_eq!(dec_err(&flag(2)), CodecError::Malformed);
    assert_eq!(dec_err(&flag(0xFF)), CodecError::Malformed);
    // A moving byte of 02 is Malformed (controls: 00 and 01 above).
    let mut bad_moving = tail.clone();
    *bad_moving.last_mut().unwrap() = 0x02;
    assert_eq!(dec_err(&bad_moving), CodecError::Malformed);
    // The two-op frame: no bytes are read after a withheld flag.
    assert_eq!(
        decode_delta(&[0x04, 0x00, 0x64, 0x01, 0x02, 0x16, 0x02, 0x00, 0x0E, 0x03]),
        Ok(delta(vec![
            position(2, None),
            wrap(v3::DeltaOp::RoundStarted { round: 3 }),
        ]))
    );
    // Entity 0 is Malformed (control: entity 2) and a missing moving byte is
    // Malformed.
    let mut entity_zero = tail.clone();
    entity_zero[1] = 0;
    assert_eq!(dec_err(&entity_zero), CodecError::Malformed);
    assert_eq!(dec_err(&tail[..tail.len() - 1]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x16, 0x02]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x16]), CodecError::Malformed);
    // A coordinate beyond U3 inside a Position is Malformed, as in any point.
    let beyond = cat(&[
        &[0x16, 0x02, 0x01],
        &[0x82, 0x80, 0x80, 0x80, 0x04, 0x00, 0x00, 0x00],
    ]);
    assert_eq!(dec_err(&beyond), CodecError::Malformed);
    // A trailing byte after a withheld position is Malformed.
    assert_eq!(
        decode_delta(&delta_bytes(1, &[0x16, 0x02, 0x00, 0x00])),
        Err(CodecError::Malformed)
    );
    // The largest Position is 28 bytes: entity 10, flag 1, point 15, moving 1.
    let largest = position(u64::MAX, Some((p(-L, -L, -L), true)));
    assert_eq!(one_op(largest.clone()).len() - 5, 28);
    round_trip_op(largest);
}

// ---------------------------------------------------------------------------
// W7 v4_tag_table_closed
// ---------------------------------------------------------------------------

#[test]
fn w7_v4_tag_table_closed() {
    assert_eq!(PROTOCOL_VERSION, 4);
    assert_eq!(INTENT_TAG_DECLARE_MOVE, 6);
    assert_eq!(DELTA_TAG_MOVED, 21);
    assert_eq!(DELTA_TAG_POSITION, 22);
    assert_eq!(COORD_LIMIT_RAW, 536_870_912);
    assert_eq!(MAX_MOVED_POINTS, 64);
    // As a bare one-op frame, tags 0..=22 are Malformed (each needs fields)
    // and 23..=255 are UnknownMessage: 256 assertions.
    for tag in 0..=255u8 {
        let expected = if tag <= 22 {
            CodecError::Malformed
        } else {
            CodecError::UnknownMessage
        };
        assert_eq!(
            decode_delta(&delta_bytes(1, &[tag])),
            Err(expected),
            "tag {tag}"
        );
    }
    // An unknown tag after valid ops is still UnknownMessage.
    assert_eq!(
        decode_delta(&delta_bytes(2, &[0x0E, 0x03, 0x17])),
        Err(CodecError::UnknownMessage)
    );
    // Intent body tags 07..=FF are UnknownMessage; 00..=06 are not.
    for tag in 7..=255u8 {
        assert_eq!(
            decode_intent(&intent_bytes(&[tag])),
            Err(CodecError::UnknownMessage),
            "tag {tag}"
        );
    }
    assert!(decode_intent(&intent_bytes(&[6, 0, 0, 0])).is_ok());
    // v3 does not learn tag 21 or the intent tag 6.
    assert_eq!(
        codec_v3::decode_delta(&[
            0x03, 0x00, 0x64, 0x01, 0x01, 0x15, 0x02, 0x02, 0x00, 0x00, 0x00, 0x80, 0x80, 0x08,
            0x00, 0x00
        ]),
        Err(CodecError::UnknownMessage)
    );
    assert_eq!(
        codec_v3::decode_delta(&[0x03, 0x00, 0x64, 0x01, 0x01, 0x16, 0x02, 0x00]),
        Err(CodecError::UnknownMessage)
    );
    assert_eq!(
        codec_v3::decode_intent(&hdr(3, 0, &[1], &[1], &[6, 0, 0, 0])),
        Err(CodecError::UnknownMessage)
    );
    // v3's tags and the v3 constants are unchanged.
    assert_eq!(v3::PROTOCOL_VERSION, 3);
    assert_eq!(v3::DELTA_TAG_HOST_FACTS, 20);
    assert_eq!(v3::INTENT_TAG_REQUEST_MODE, 5);
    // The re-exports are v3's types.
    let code: v3::RejectionCode = RejectionCode::NoPauseRight;
    let status: v3::ReceiptStatus = ReceiptStatus::Rejected(code);
    assert_eq!(
        status,
        v3::ReceiptStatus::Rejected(v3::RejectionCode::NoPauseRight)
    );
}

// ---------------------------------------------------------------------------
// W8 v4_delta_bounds
// ---------------------------------------------------------------------------

#[test]
fn w8_v4_delta_bounds() {
    assert_eq!(
        encode_delta(&delta(vec![])).unwrap(),
        vec![0x04, 0x00, 0x64, 0x01, 0x00]
    );
    assert_eq!(MAX_DELTA_OPS, 256);
    // 256 ops accepted; 257 are LimitExceeded on encode.
    let rounds = |n: usize| -> Vec<DeltaOp> {
        (0..n)
            .map(|_| wrap(v3::DeltaOp::RoundStarted { round: 0 }))
            .collect()
    };
    let full = delta(rounds(256));
    let bytes = encode_delta(&full).unwrap();
    assert_eq!(&bytes[..6], &[0x04, 0x00, 0x64, 0x01, 0x80, 0x02]);
    assert_eq!(decode_delta(&bytes), Ok(full));
    assert_eq!(
        encode_delta(&delta(rounds(257))),
        Err(CodecError::LimitExceeded)
    );
    // A count of 257 (81 02) with no op following is LimitExceeded, before
    // any op is read; the count of 256 with its ops is the control above.
    assert_eq!(
        decode_delta(&[0x04, 0x00, 0x64, 0x01, 0x81, 0x02]),
        Err(CodecError::LimitExceeded)
    );
    // Frame lane 01 is WrongDirection (control: lane 00 above).
    assert_eq!(
        decode_delta(&[0x04, 0x01, 0x64, 0x01, 0x00]),
        Err(CodecError::WrongDirection)
    );
    // One trailing byte is Malformed.
    assert_eq!(
        decode_delta(&[0x04, 0x00, 0x64, 0x01, 0x00, 0x00]),
        Err(CodecError::Malformed)
    );
    // event_seq and server_tick u64::MAX round-trip.
    let extreme = DeltaFrame {
        lane: 0,
        server_tick: u64::MAX,
        event_seq: u64::MAX,
        ops: vec![moved(2, &[pa(), p(1, 0, 0)])],
    };
    let extreme_bytes = encode_delta(&extreme).unwrap();
    assert_eq!(decode_delta(&extreme_bytes), Ok(extreme));
    // Header fields are read in order: a truncated header is Malformed.
    for cut in 1..5 {
        assert_eq!(
            decode_delta(&[0x04, 0x00, 0x64, 0x01, 0x00][..cut]),
            Err(CodecError::Malformed),
            "cut {cut}"
        );
    }

    // Encode order: lane, then count, then each op in order.
    let bad_moved = moved(2, &[pa()]);
    let bad_receipt = wrap(v3::DeltaOp::Receipt {
        epoch: Z16,
        lane: 1,
        seq: 1,
        processed_tick: 100,
        status: v3::ReceiptStatus::Applied,
    });
    let mut lane_one = delta(rounds(257));
    lane_one.lane = 1;
    assert_eq!(encode_delta(&lane_one), Err(CodecError::WrongDirection));
    let mut many = vec![bad_moved.clone()];
    many.extend(rounds(256));
    assert_eq!(many.len(), 257);
    assert_eq!(encode_delta(&delta(many)), Err(CodecError::LimitExceeded));
    assert_eq!(
        encode_delta(&delta(vec![bad_moved.clone(), bad_receipt.clone()])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        encode_delta(&delta(vec![bad_receipt, bad_moved])),
        Err(CodecError::WrongDirection)
    );
    // The same op checks run for a good-then-bad frame: the bad one wins.
    assert_eq!(
        encode_delta(&delta(vec![
            wrap(v3::DeltaOp::RoundStarted { round: 1 }),
            moved(2, &p65()),
        ])),
        Err(CodecError::LimitExceeded)
    );
}

// ---------------------------------------------------------------------------
// W9 v4_frame_spans_and_cap
// ---------------------------------------------------------------------------

fn r(round: u64) -> DeltaOp {
    wrap(v3::DeltaOp::RoundStarted { round })
}

fn big_moved() -> DeltaOp {
    moved(u64::MAX, &p64())
}

fn op_len(op: &DeltaOp) -> usize {
    one_op(op.clone()).len() - 5
}

/// `OA`: 67 x `M`, 28 x `R(u64::MAX)`, 40 x `R(0)`.
fn oa() -> Vec<DeltaOp> {
    let mut ops = Vec::new();
    ops.extend((0..67).map(|_| big_moved()));
    ops.extend((0..28).map(|_| r(u64::MAX)));
    ops.extend((0..40).map(|_| r(0)));
    ops
}

fn max_frame(ops: Vec<DeltaOp>) -> DeltaFrame {
    DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: u64::MAX,
        event_seq: u64::MAX,
        ops,
    }
}

fn spans(list: &[(usize, usize)]) -> Vec<Range<usize>> {
    list.iter().map(|&(a, b)| a..b).collect()
}

#[test]
fn w9_v4_frame_spans_and_cap() {
    // The op sizes.
    assert_eq!(op_len(&big_moved()), 972);
    assert_eq!(op_len(&r(u64::MAX)), 11);
    assert_eq!(op_len(&r(0)), 2);
    assert_eq!(op_len(&r(128)), 3);

    // At the cap.
    let a = oa();
    assert_eq!(a.len(), 135);
    let total: usize = a.iter().map(op_len).sum();
    assert_eq!(total, 65_512);
    assert_eq!(frame_spans(&a), Ok(spans(&[(0, 135)])));
    let frame = max_frame(a);
    let bytes = encode_delta(&frame).expect("65,536 bytes is within the cap");
    assert_eq!(bytes.len(), 65_536);
    let mut header = vec![4u8, 0];
    header.extend([0xFF; 9]);
    header.push(0x01);
    header.extend([0xFF; 9]);
    header.push(0x01);
    header.extend([0x87, 0x01]);
    assert_eq!(header.len(), 24);
    assert_eq!(&bytes[..24], header.as_slice());
    assert_eq!(decode_delta(&bytes), Ok(frame));

    // One past: the last op becomes R(128), 65,513 op bytes.
    let mut b = oa();
    *b.last_mut().unwrap() = r(128);
    let total: usize = b.iter().map(op_len).sum();
    assert_eq!(total, 65_513);
    assert_eq!(frame_spans(&b), Ok(spans(&[(0, 134), (134, 135)])));
    assert_eq!(encode_delta(&max_frame(b)), Err(CodecError::FrameTooLarge));

    // Op cap.
    let rs = |n: usize| -> Vec<DeltaOp> { (0..n).map(|_| r(0)).collect() };
    assert_eq!(frame_spans(&rs(256)), Ok(spans(&[(0, 256)])));
    assert_eq!(frame_spans(&rs(257)), Ok(spans(&[(0, 256), (256, 257)])));
    assert_eq!(frame_spans(&rs(512)), Ok(spans(&[(0, 256), (256, 512)])));
    assert_eq!(frame_spans(&[]), Ok(vec![]));
    assert_eq!(frame_spans(&rs(1)), Ok(spans(&[(0, 1)])));

    // Error: the first failing op wins, with no spans.
    assert_eq!(
        frame_spans(&[r(0), moved(2, &[pa()])]),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        frame_spans(&[moved(2, &p65()), moved(2, &[pa()])]),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        frame_spans(&[
            moved(2, &[pa(), p(0, 5, 0)]),
            wrap(v3::DeltaOp::Receipt {
                epoch: Z16,
                lane: 1,
                seq: 1,
                processed_tick: 1,
                status: v3::ReceiptStatus::Applied,
            }),
        ]),
        Err(CodecError::Malformed)
    );
    // A big op closes the frame before it, not after it.
    let mut ops = vec![r(u64::MAX)];
    ops.extend((0..68).map(|_| big_moved()));
    // 11 + 67 * 972 = 65,135; the 68th Moved would reach 66,107.
    assert_eq!(frame_spans(&ops), Ok(spans(&[(0, 68), (68, 69)])));

    // Position ops count their bytes too: 28 bytes each at the maximum.
    let pos = position(u64::MAX, Some((p(-L, -L, -L), true)));
    assert_eq!(op_len(&pos), 28);
    let many_pos: Vec<DeltaOp> = (0..256).map(|_| pos.clone()).collect();
    assert_eq!(frame_spans(&many_pos), Ok(spans(&[(0, 256)])));

    // Same as v3: T140 B9's A and B, wrapped in V3.
    let t = |k: u64| -> v3::DeltaOp {
        let entries: Vec<(i32, u64)> = (0..k).map(|i| (0, (1u64 << 63) + i)).collect();
        timeline(&entries)
    };
    let rr = |round: u64| v3::DeltaOp::RoundStarted { round };
    let mut v3_a: Vec<v3::DeltaOp> = Vec::new();
    for _ in 0..5 {
        v3_a.push(t(1024));
    }
    v3_a.push(t(711));
    for _ in 0..122 {
        v3_a.push(rr(u64::MAX));
    }
    let mut v3_b = v3_a.clone();
    v3_a.push(rr(1 << 21));
    v3_b.push(rr(1 << 28));
    for (ops, expected) in [
        (v3_a, spans(&[(0, 129)])),
        (v3_b, spans(&[(0, 128), (128, 129)])),
    ] {
        let wrapped: Vec<DeltaOp> = ops.iter().cloned().map(DeltaOp::V3).collect();
        assert_eq!(codec_v3::frame_spans(&ops), Ok(expected.clone()));
        assert_eq!(frame_spans(&wrapped), Ok(expected));
    }
}

// ---------------------------------------------------------------------------
// W10 v4_round_trip_identity
// ---------------------------------------------------------------------------

#[test]
fn w10_v4_round_trip_identity() {
    // Every intent body.
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
        IntentBody::RequestMode {
            mode: EncounterMode::TurnBased,
        },
        IntentBody::DeclareMove { destination: pa() },
        IntentBody::DeclareMove {
            destination: p(L, -L, L),
        },
        IntentBody::DeclareMove {
            destination: p(-L, L, -L),
        },
    ] {
        round_trip_intent(intent(body));
    }
    // A V3 op of every v3 tag (and every rejection code), Moved, Position.
    for op in v3_round_trip_ops() {
        round_trip_op(wrap(op));
    }
    round_trip_op(moved(2, &[pa(), p(1, 0, 0)]));
    round_trip_op(moved(u64::MAX, &p64()));
    round_trip_op(position(2, Some((pb(), true))));
    round_trip_op(position(3, Some((p(-L, L, 0), false))));
    round_trip_op(position(4, None));
    // One frame holding a mix, encoded twice.
    let mix = delta(vec![
        position(2, None),
        wrap(v3::DeltaOp::EntityEnter { entity: net(2) }),
        moved(2, &[pa(), p(1, 0, 0), p(1, 0, 1)]),
        position(2, Some((p(1, 0, 1), true))),
        wrap(v3::DeltaOp::Left { entity: net(2) }),
    ]);
    let bytes = encode_delta(&mix).unwrap();
    assert_eq!(bytes, encode_delta(&mix).unwrap());
    assert_eq!(decode_delta(&bytes), Ok(mix));
    // Every prefix of the mix is refused as Malformed, never a panic.
    for cut in 0..bytes.len() {
        assert_eq!(
            decode_delta(&bytes[..cut]),
            Err(CodecError::Malformed),
            "cut {cut}"
        );
    }

    // Intent conversion is variant for variant.
    let ability = u(5);
    assert_eq!(
        IntentBody::from(v3::IntentBody::DeclareAction {
            ability,
            target: net(2)
        }),
        IntentBody::DeclareAction {
            ability,
            target: net(2)
        }
    );
    for (from, to) in [
        (v3::IntentBody::EndTurn, IntentBody::EndTurn),
        (v3::IntentBody::Stop, IntentBody::Stop),
        (v3::IntentBody::Pause, IntentBody::Pause),
        (v3::IntentBody::Unpause, IntentBody::Unpause),
        (
            v3::IntentBody::RequestMode {
                mode: EncounterMode::RealTime,
            },
            IntentBody::RequestMode {
                mode: EncounterMode::RealTime,
            },
        ),
    ] {
        assert_eq!(IntentBody::from(from), to);
    }

    // A lowercase ULID alias decodes and re-encodes canonical uppercase.
    let ability = Ulid::from_parts(0x0123_4567_89AB, 0x0CDE_F012_3456_789A);
    let canonical = ability.to_string();
    assert!(canonical.chars().any(|c| c.is_ascii_uppercase()));
    let lowered = canonical.to_ascii_lowercase();
    assert_ne!(canonical, lowered);
    let alias_bytes = intent_bytes(&cat(&[&[0, 26], lowered.as_bytes(), &[2]]));
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
        intent_bytes(&cat(&[&[0, 26], canonical.as_bytes(), &[2]]))
    );
    assert_ne!(canonical_bytes, alias_bytes);
    // Same for an encounter id inside a V3 op.
    let alias_delta = delta_bytes(1, &cat(&[&[10, 26], lowered.as_bytes(), &[3]]));
    let decoded = decode_delta(&alias_delta).unwrap();
    assert_eq!(
        decoded.ops,
        vec![wrap(v3::DeltaOp::EncounterEnded {
            encounter: ability,
            round: 3
        })]
    );
    assert_eq!(
        encode_delta(&decoded).unwrap(),
        delta_bytes(1, &cat(&[&[10, 26], canonical.as_bytes(), &[3]]))
    );

    // Point::new is Some exactly when every component is within U3.
    for raw in [
        [0, 0, 0],
        [L, L, L],
        [-L, -L, -L],
        [L, -L, 0],
        [L + 1, 0, 0],
        [0, L + 1, 0],
        [0, 0, L + 1],
        [-L - 1, 0, 0],
        [0, -L - 1, 0],
        [0, 0, -L - 1],
        [i32::MIN, 0, 0],
        [0, i32::MAX, 0],
    ] {
        let within = raw.iter().all(|c| c.unsigned_abs() <= L as u32);
        assert_eq!(Point::new(raw).is_some(), within, "{raw:?}");
        if let Some(point) = Point::new(raw) {
            assert_eq!(point.raw(), raw);
        }
    }
}

// ---------------------------------------------------------------------------
// W11 v4_constants_match_sim
// ---------------------------------------------------------------------------

#[test]
fn w11_v4_constants_match_sim() {
    assert_eq!(COORD_LIMIT_RAW, crpg_sim::COORD_LIMIT_RAW);
    assert_eq!(MAX_MOVED_POINTS, crpg_sim::MAX_ROUTE_POINTS);
}
