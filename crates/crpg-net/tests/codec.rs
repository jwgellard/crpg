//! T018a codec conformance: round-trip, rejection matrix, boundaries.
//!
//! Net-local only: hand-built [`Ulid`]s (no data/sim edges), fixed tables (no
//! `HashMap`, no clock, no threads, no I/O, no RNG in the codec path).

use crpg_core::Ulid;
use crpg_net::codec::{decode_delta, decode_intent, encode_delta, encode_intent, CodecError};
use crpg_net::protocol::{
    DeltaFrame, DeltaOp, IntentBody, IntentFrame, NetId, ReceiptStatus, RejectionCode,
    DELTA_TAG_DESPAWNED, DELTA_TAG_DIED, DELTA_TAG_ENTITY_ENTER, DELTA_TAG_ENTITY_LEAVE,
    DELTA_TAG_HEALTH, DELTA_TAG_RECEIPT, DELTA_TAG_SPAWNED, DELTA_TAG_TURN,
    INTENT_TAG_DECLARE_ACTION, INTENT_TAG_END_TURN, LANE_COMBAT, MAX_DELTA_FRAME_BYTES,
    MAX_DELTA_OPS, MAX_INTENT_FRAME_BYTES, MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_CHUNKS,
    MAX_VISIBLE_ENTITIES, MAX_WIRE_BYTES_FIELD, MAX_WIRE_COLLECTION, MAX_WIRE_STRING_BYTES,
    POLICY_EGRESS_BYTES_HOST, POLICY_EGRESS_BYTES_PER_PEER, POLICY_EGRESS_FRAMES_PER_PEER,
    POLICY_FAIL_BYTES_PER_SEC, POLICY_FAIL_BYTE_BURST, POLICY_FAIL_FRAMES_PER_SEC,
    POLICY_FAIL_FRAME_BURST, POLICY_INGRESS_BYTES_HOST, POLICY_INGRESS_BYTES_PER_PEER,
    POLICY_INGRESS_FRAMES_HOST, POLICY_INGRESS_FRAMES_PER_PEER, POLICY_INTENT_BYTES_PER_SEC,
    POLICY_INTENT_BYTE_BURST, POLICY_INTENT_FRAMES_PER_SEC, POLICY_INTENT_FRAME_BURST,
    POLICY_OBSERVED_TICK_WINDOW, POLICY_PROOF_PEERS, POLICY_RETRY_CACHE_BYTES_PER_PEER_LANE,
    POLICY_RETRY_CACHE_PER_PEER_LANE, POLICY_SNAPSHOT_IN_FLIGHT_PER_PEER,
    POLICY_SNAPSHOT_REASSEMBLY_SECS, PROTOCOL_VERSION, RECEIPT_TAG_APPLIED, RECEIPT_TAG_REJECTED,
    SEQ_FIRST, SEQ_LAST,
};

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

const EPOCH: [u8; 16] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
];

fn net(raw: u64) -> NetId {
    NetId::new(raw).expect("fixture id is nonzero")
}

fn ability() -> Ulid {
    // Fixed hand-built ULID; canonical text is pinned below.
    Ulid::from_parts(0x0123_4567_89AB, 0x0CDE_F012_3456_789A)
}

fn declare_frame() -> IntentFrame {
    IntentFrame {
        epoch: EPOCH,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 41,
        actor: net(7),
        body: IntentBody::DeclareAction {
            ability: ability(),
            target: net(9),
        },
    }
}

fn end_turn_frame() -> IntentFrame {
    IntentFrame {
        epoch: EPOCH,
        lane: LANE_COMBAT,
        seq: 2,
        observed_tick: 42,
        actor: net(7),
        body: IntentBody::EndTurn,
    }
}

fn all_ops_delta() -> DeltaFrame {
    DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 100,
        event_seq: 55,
        ops: vec![
            DeltaOp::EntityEnter { entity: net(1) },
            DeltaOp::EntityLeave { entity: net(2) },
            DeltaOp::Spawned { entity: net(3) },
            DeltaOp::Despawned { entity: net(4) },
            DeltaOp::Died { entity: net(5) },
            DeltaOp::Health {
                entity: net(6),
                health: 3,
                max_health: 12,
                dead: false,
            },
            DeltaOp::Turn {
                active: Some(net(6)),
                round: 2,
            },
            DeltaOp::Turn {
                active: None,
                round: 3,
            },
            DeltaOp::Receipt {
                epoch: EPOCH,
                lane: LANE_COMBAT,
                seq: 1,
                processed_tick: 99,
                status: ReceiptStatus::Applied,
            },
            DeltaOp::Receipt {
                epoch: EPOCH,
                lane: LANE_COMBAT,
                seq: 2,
                processed_tick: 100,
                status: ReceiptStatus::Rejected(RejectionCode::IllegalAction),
            },
        ],
    }
}

fn all_rejection_codes() -> [RejectionCode; 20] {
    [
        RejectionCode::FrameTooLarge,
        RejectionCode::QueueFull,
        RejectionCode::UnsupportedVersion,
        RejectionCode::WrongDirection,
        RejectionCode::UnknownMessage,
        RejectionCode::Malformed,
        RejectionCode::LimitExceeded,
        RejectionCode::RateLimited,
        RejectionCode::Unauthenticated,
        RejectionCode::SessionExpired,
        RejectionCode::SeqConflict,
        RejectionCode::SeqGap,
        RejectionCode::StaleSeq,
        RejectionCode::SeqExhausted,
        RejectionCode::StaleTick,
        RejectionCode::FutureTick,
        RejectionCode::NotAuthorized,
        RejectionCode::IllegalAction,
        RejectionCode::ServerBusy,
        RejectionCode::SnapshotExpired,
    ]
}

// ---------------------------------------------------------------------------
// 1. Round-trip + pinned wire values.
// ---------------------------------------------------------------------------

#[test]
fn intent_variants_round_trip() {
    for frame in [declare_frame(), end_turn_frame()] {
        let bytes = encode_intent(&frame).expect("valid intent encodes");
        assert!(bytes.len() <= MAX_INTENT_FRAME_BYTES);
        assert_eq!(decode_intent(&bytes).expect("valid intent decodes"), frame);
    }
}

#[test]
fn delta_all_ops_round_trip() {
    let frame = all_ops_delta();
    let bytes = encode_delta(&frame).expect("valid delta encodes");
    assert!(bytes.len() <= MAX_DELTA_FRAME_BYTES);
    assert_eq!(decode_delta(&bytes).expect("valid delta decodes"), frame);
}

#[test]
fn every_rejection_code_round_trips() {
    for (index, code) in all_rejection_codes().into_iter().enumerate() {
        let frame = DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 1,
            event_seq: index as u64,
            ops: vec![DeltaOp::Receipt {
                epoch: EPOCH,
                lane: LANE_COMBAT,
                seq: 1,
                processed_tick: 1,
                status: ReceiptStatus::Rejected(code),
            }],
        };
        let bytes = encode_delta(&frame).expect("receipt encodes");
        assert_eq!(decode_delta(&bytes).expect("receipt decodes"), frame);
    }
}

#[test]
fn frozen_wire_values_pinned() {
    assert_eq!(PROTOCOL_VERSION, 1);
    assert_eq!(LANE_COMBAT, 0);
    assert_eq!(SEQ_FIRST, 1);
    assert_eq!(SEQ_LAST, u64::MAX - 1);
    assert_eq!(INTENT_TAG_DECLARE_ACTION, 0);
    assert_eq!(INTENT_TAG_END_TURN, 1);
    assert_eq!(DELTA_TAG_ENTITY_ENTER, 0);
    assert_eq!(DELTA_TAG_ENTITY_LEAVE, 1);
    assert_eq!(DELTA_TAG_SPAWNED, 2);
    assert_eq!(DELTA_TAG_DESPAWNED, 3);
    assert_eq!(DELTA_TAG_DIED, 4);
    assert_eq!(DELTA_TAG_HEALTH, 5);
    assert_eq!(DELTA_TAG_TURN, 6);
    assert_eq!(DELTA_TAG_RECEIPT, 7);
    assert_eq!(RECEIPT_TAG_APPLIED, 0);
    assert_eq!(RECEIPT_TAG_REJECTED, 1);
    let codes: Vec<u8> = all_rejection_codes().iter().map(|c| c.as_u8()).collect();
    assert_eq!(
        codes,
        (1..=20).collect::<Vec<u8>>(),
        "frozen v1 code values 1..=20"
    );
    for raw in [0u8, 21, 100, 255] {
        assert_eq!(RejectionCode::from_u8(raw), None);
    }
    for code in all_rejection_codes() {
        assert_eq!(RejectionCode::from_u8(code.as_u8()), Some(code));
    }
}

#[test]
fn wire_hard_maxima_pinned() {
    assert_eq!(MAX_INTENT_FRAME_BYTES, 4096);
    assert_eq!(MAX_DELTA_FRAME_BYTES, 65536);
    assert_eq!(MAX_DELTA_OPS, 256);
    assert_eq!(MAX_WIRE_COLLECTION, 256);
    assert_eq!(MAX_WIRE_STRING_BYTES, 256);
    assert_eq!(MAX_WIRE_BYTES_FIELD, 4096);
    assert_eq!(MAX_SNAPSHOT_BYTES, 1_048_576);
    assert_eq!(MAX_VISIBLE_ENTITIES, 1024);
    assert_eq!(MAX_SNAPSHOT_CHUNKS, 32);
}

#[test]
fn operational_policy_defaults_pinned() {
    assert_eq!(POLICY_INTENT_FRAMES_PER_SEC, 40);
    assert_eq!(POLICY_INTENT_FRAME_BURST, 80);
    assert_eq!(POLICY_INTENT_BYTES_PER_SEC, 64 * 1024);
    assert_eq!(POLICY_INTENT_BYTE_BURST, 128 * 1024);
    assert_eq!(POLICY_FAIL_FRAMES_PER_SEC, 10);
    assert_eq!(POLICY_FAIL_FRAME_BURST, 10);
    assert_eq!(POLICY_FAIL_BYTES_PER_SEC, 4 * 1024);
    assert_eq!(POLICY_FAIL_BYTE_BURST, 4 * 1024);
    assert_eq!(POLICY_INGRESS_FRAMES_PER_PEER, 128);
    assert_eq!(POLICY_INGRESS_BYTES_PER_PEER, 256 * 1024);
    assert_eq!(POLICY_INGRESS_FRAMES_HOST, 1024);
    assert_eq!(POLICY_INGRESS_BYTES_HOST, 2 * 1024 * 1024);
    assert_eq!(POLICY_EGRESS_FRAMES_PER_PEER, 128);
    assert_eq!(POLICY_EGRESS_BYTES_PER_PEER, 2 * 1024 * 1024);
    assert_eq!(POLICY_EGRESS_BYTES_HOST, 16 * 1024 * 1024);
    assert_eq!(POLICY_RETRY_CACHE_PER_PEER_LANE, 256);
    assert_eq!(POLICY_RETRY_CACHE_BYTES_PER_PEER_LANE, 2 * 1024 * 1024);
    assert_eq!(POLICY_PROOF_PEERS, 8);
    assert_eq!(POLICY_SNAPSHOT_IN_FLIGHT_PER_PEER, 1);
    assert_eq!(POLICY_SNAPSHOT_REASSEMBLY_SECS, 5);
    assert_eq!(POLICY_OBSERVED_TICK_WINDOW, 200);
}

#[test]
fn ulid_canonical_form_pinned() {
    assert_eq!(Ulid::NIL.to_string(), "00000000000000000000000000");
    let text = ability().to_string();
    assert_eq!(text.len(), 26);
    assert!(text
        .bytes()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()));
    // The wire carries exactly the canonical text: length prefix 26 + bytes.
    let bytes = encode_intent(&declare_frame()).expect("encodes");
    let needle = text.as_bytes();
    let pos = bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("canonical ULID text on the wire");
    assert_eq!(bytes[pos - 1], 26, "postcard string prefix is the length");
}

#[test]
fn net_id_zero_rejected_at_construction() {
    assert_eq!(NetId::new(0), None);
    assert_eq!(net(1).get(), 1);
    assert_eq!(net(u64::MAX).get(), u64::MAX);
}

// ---------------------------------------------------------------------------
// Golden bytes: exact layout of minimal frames.
// ---------------------------------------------------------------------------

#[test]
fn end_turn_golden_bytes() {
    let frame = IntentFrame {
        epoch: [0; 16],
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 7,
        actor: net(1),
        body: IntentBody::EndTurn,
    };
    let mut expected = vec![PROTOCOL_VERSION];
    expected.extend([0; 16]);
    expected.extend([LANE_COMBAT, 0x01, 0x07, 0x01, INTENT_TAG_END_TURN]);
    assert_eq!(encode_intent(&frame).expect("encodes"), expected);
    assert_eq!(decode_intent(&expected).expect("decodes"), frame);
}

#[test]
fn empty_delta_golden_bytes() {
    let frame = DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 0,
        event_seq: 0,
        ops: vec![],
    };
    let expected = vec![PROTOCOL_VERSION, LANE_COMBAT, 0x00, 0x00, 0x00];
    assert_eq!(encode_delta(&frame).expect("encodes"), expected);
    assert_eq!(decode_delta(&expected).expect("decodes"), frame);
}

// ---------------------------------------------------------------------------
// 2. Rejection matrix: each hostile input maps to its exact CodecError.
// ---------------------------------------------------------------------------

fn with_version(bytes: &[u8], version: u8) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out[0] = version;
    out
}

#[test]
fn unknown_version_rejected() {
    let intent = encode_intent(&declare_frame()).expect("encodes");
    let delta = encode_delta(&all_ops_delta()).expect("encodes");
    for version in [0u8, 2, 255] {
        assert_eq!(
            decode_intent(&with_version(&intent, version)),
            Err(CodecError::UnsupportedVersion),
            "intent version {version}"
        );
        assert_eq!(
            decode_delta(&with_version(&delta, version)),
            Err(CodecError::UnsupportedVersion),
            "delta version {version}"
        );
    }
}

#[test]
fn wrong_lane_rejected() {
    let mut intent = declare_frame();
    intent.lane = 1;
    assert_eq!(encode_intent(&intent), Err(CodecError::WrongDirection));
    let mut delta = all_ops_delta();
    delta.lane = 3;
    assert_eq!(encode_delta(&delta), Err(CodecError::WrongDirection));

    // Lane byte offsets on the wire: intent [1 + 16], delta [1].
    let good_intent = encode_intent(&declare_frame()).expect("encodes");
    let good_delta = encode_delta(&all_ops_delta()).expect("encodes");
    for lane in [1u8, 255] {
        let mut bad = good_intent.clone();
        bad[1 + 16] = lane;
        assert_eq!(decode_intent(&bad), Err(CodecError::WrongDirection));
        let mut bad = good_delta.clone();
        bad[1] = lane;
        assert_eq!(decode_delta(&bad), Err(CodecError::WrongDirection));
    }
}

#[test]
fn receipt_lane_and_seq_checked_both_directions() {
    let receipt = |lane: u8, seq: u64| DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 1,
        event_seq: 1,
        ops: vec![DeltaOp::Receipt {
            epoch: EPOCH,
            lane,
            seq,
            processed_tick: 1,
            status: ReceiptStatus::Applied,
        }],
    };
    // Encode side: raw struct fields are checkable (lane/seq are plain u8/u64).
    assert_eq!(
        encode_delta(&receipt(5, 1)),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(encode_delta(&receipt(0, 0)), Err(CodecError::Malformed));
    assert_eq!(
        encode_delta(&receipt(0, u64::MAX)),
        Err(CodecError::Malformed)
    );
    // Decode side: flip the receipt lane byte of a valid encoding.
    let mut bytes = encode_delta(&receipt(0, 1)).expect("encodes");
    // Layout: version(1) lane(1) tick(1) event_seq(1) count(1) tag(1)
    // epoch(16) then receipt lane.
    let lane_pos = 1 + 1 + 1 + 1 + 1 + 1 + 16;
    assert_eq!(bytes[lane_pos], LANE_COMBAT);
    bytes[lane_pos] = 2;
    assert_eq!(decode_delta(&bytes), Err(CodecError::WrongDirection));
}

#[test]
fn unknown_discriminants_rejected() {
    // Intent body tag is the last byte of a minimal EndTurn frame.
    let mut end = encode_intent(&end_turn_frame()).expect("encodes");
    let tag_pos = end.len() - 1;
    for tag in [2u8, 100, 255] {
        end[tag_pos] = tag;
        assert_eq!(decode_intent(&end), Err(CodecError::UnknownMessage));
    }
    // Delta op tag: single-op delta, tag right after the 5-byte header.
    let op_frame = DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 0,
        event_seq: 0,
        ops: vec![DeltaOp::Died { entity: net(1) }],
    };
    let mut bytes = encode_delta(&op_frame).expect("encodes");
    assert_eq!(bytes.len(), 5 + 1 + 1);
    for tag in [8u8, 100, 255] {
        bytes[5] = tag;
        assert_eq!(decode_delta(&bytes), Err(CodecError::UnknownMessage));
    }
    // Receipt status tag + code bytes.
    let receipt_frame = DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 0,
        event_seq: 0,
        ops: vec![DeltaOp::Receipt {
            epoch: EPOCH,
            lane: LANE_COMBAT,
            seq: 1,
            processed_tick: 0,
            status: ReceiptStatus::Rejected(RejectionCode::StaleSeq),
        }],
    };
    let mut bytes = encode_delta(&receipt_frame).expect("encodes");
    let status_pos = bytes.len() - 2;
    assert_eq!(bytes[status_pos], RECEIPT_TAG_REJECTED);
    bytes[status_pos] = 7;
    assert_eq!(decode_delta(&bytes), Err(CodecError::UnknownMessage));
    bytes[status_pos] = RECEIPT_TAG_REJECTED;
    let code_pos = bytes.len() - 1;
    for code in [0u8, 21, 255] {
        bytes[code_pos] = code;
        assert_eq!(decode_delta(&bytes), Err(CodecError::UnknownMessage));
    }
}

#[test]
fn trailing_bytes_rejected() {
    let mut intent = encode_intent(&declare_frame()).expect("encodes");
    intent.push(0x00);
    assert_eq!(decode_intent(&intent), Err(CodecError::Malformed));
    let mut delta = encode_delta(&all_ops_delta()).expect("encodes");
    delta.extend([0x01, 0x02]);
    assert_eq!(decode_delta(&delta), Err(CodecError::Malformed));
}

#[test]
fn empty_and_truncated_inputs_malformed() {
    assert_eq!(decode_intent(&[]), Err(CodecError::Malformed));
    assert_eq!(decode_delta(&[]), Err(CodecError::Malformed));
    let intent = encode_intent(&declare_frame()).expect("encodes");
    for len in [1, 5, 17, intent.len() - 1] {
        assert_eq!(
            decode_intent(&intent[..len]),
            Err(CodecError::Malformed),
            "intent prefix len {len}"
        );
    }
    let delta = encode_delta(&all_ops_delta()).expect("encodes");
    for len in [1, 3, 10, delta.len() - 1] {
        assert_eq!(
            decode_delta(&delta[..len]),
            Err(CodecError::Malformed),
            "delta prefix len {len}"
        );
    }
}

#[test]
fn zero_net_ids_rejected() {
    // Actor = 0: intent layout version(1) epoch(16) lane(1) seq(1) tick(1).
    let mut intent = encode_intent(&declare_frame()).expect("encodes");
    let actor_pos = 1 + 16 + 1 + 1 + 1;
    intent[actor_pos] = 0x00;
    assert_eq!(decode_intent(&intent), Err(CodecError::Malformed));

    // Target = 0 on the DeclareAction path.
    let mut intent = encode_intent(&declare_frame()).expect("encodes");
    let last = intent.len() - 1;
    intent[last] = 0x00;
    assert_eq!(decode_intent(&intent), Err(CodecError::Malformed));

    // Entity = 0 in every single-entity op.
    for op in [
        DeltaOp::EntityEnter { entity: net(1) },
        DeltaOp::EntityLeave { entity: net(1) },
        DeltaOp::Spawned { entity: net(1) },
        DeltaOp::Despawned { entity: net(1) },
        DeltaOp::Died { entity: net(1) },
    ] {
        let frame = DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 0,
            event_seq: 0,
            ops: vec![op],
        };
        let mut bytes = encode_delta(&frame).expect("encodes");
        let entity_pos = bytes.len() - 1;
        bytes[entity_pos] = 0x00;
        assert_eq!(decode_delta(&bytes), Err(CodecError::Malformed));
    }

    // Health entity = 0.
    let health = DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 0,
        event_seq: 0,
        ops: vec![DeltaOp::Health {
            entity: net(1),
            health: 5,
            max_health: 5,
            dead: false,
        }],
    };
    let mut bytes = encode_delta(&health).expect("encodes");
    bytes[5 + 1] = 0x00;
    assert_eq!(decode_delta(&bytes), Err(CodecError::Malformed));

    // Turn active = Some(0).
    let turn = DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 0,
        event_seq: 0,
        ops: vec![DeltaOp::Turn {
            active: Some(net(1)),
            round: 0,
        }],
    };
    let mut bytes = encode_delta(&turn).expect("encodes");
    let active_pos = bytes.len() - 2;
    bytes[active_pos] = 0x00;
    assert_eq!(decode_delta(&bytes), Err(CodecError::Malformed));
}

#[test]
fn seq_bounds_rejected_both_directions() {
    for seq in [0u64, u64::MAX] {
        let mut frame = declare_frame();
        frame.seq = seq;
        assert_eq!(encode_intent(&frame), Err(CodecError::Malformed));
    }
    // Boundary survivors still round-trip.
    for seq in [SEQ_FIRST, SEQ_LAST, 1, u64::MAX - 1] {
        let mut frame = declare_frame();
        frame.seq = seq;
        let bytes = encode_intent(&frame).expect("boundary seq encodes");
        assert_eq!(decode_intent(&bytes).expect("boundary seq decodes"), frame);
    }
    // Wire-level: seq varint zeroed in place (frame used seq 1, single byte).
    let mut bytes = encode_intent(&declare_frame()).expect("encodes");
    let seq_pos = 1 + 16 + 1;
    assert_eq!(bytes[seq_pos], 0x01);
    bytes[seq_pos] = 0x00;
    assert_eq!(decode_intent(&bytes), Err(CodecError::Malformed));
}

#[test]
fn oversized_frames_rejected_before_decode() {
    let big_intent = vec![0xAA; MAX_INTENT_FRAME_BYTES + 1];
    assert_eq!(decode_intent(&big_intent), Err(CodecError::FrameTooLarge));
    let big_delta = vec![0xAA; MAX_DELTA_FRAME_BYTES + 1];
    assert_eq!(decode_delta(&big_delta), Err(CodecError::FrameTooLarge));
    // Exact-cap inputs get past the cap gate into version dispatch.
    assert_eq!(
        decode_intent(&vec![0x09; MAX_INTENT_FRAME_BYTES]),
        Err(CodecError::UnsupportedVersion)
    );
}

#[test]
fn oversized_ops_count_rejected_without_allocation() {
    // Hostile count u64::MAX: valid varint, then nothing.
    // Must fail as LimitExceeded, not hang or reserve.
    let mut bytes = vec![PROTOCOL_VERSION, LANE_COMBAT, 0x00, 0x00];
    bytes.extend([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01]);
    assert_eq!(decode_delta(&bytes), Err(CodecError::LimitExceeded));
}

#[test]
fn oversized_ability_string_rejected() {
    // 257-byte ability text: passes postcard decode, fails the string bound.
    let long = "A".repeat(MAX_WIRE_STRING_BYTES + 1);
    // Rebuild by hand: header through tag, then the long string, then target.
    let mut bytes = Vec::new();
    bytes.push(PROTOCOL_VERSION);
    bytes.extend(EPOCH);
    bytes.push(LANE_COMBAT);
    bytes.extend([0x01, 0x2A, 0x07, INTENT_TAG_DECLARE_ACTION]);
    postcard_extend_str(&mut bytes, &long);
    bytes.push(0x09);
    assert!(bytes.len() <= MAX_INTENT_FRAME_BYTES);
    assert_eq!(decode_intent(&bytes), Err(CodecError::LimitExceeded));
}

#[test]
fn hostile_string_prefix_cannot_force_allocation() {
    // Claims a 100,000-byte string but supplies almost none of it.
    let mut bytes = Vec::new();
    bytes.push(PROTOCOL_VERSION);
    bytes.extend(EPOCH);
    bytes.extend([LANE_COMBAT, 0x01, 0x2A, 0x07, INTENT_TAG_DECLARE_ACTION]);
    bytes.extend([0xA0, 0x8D, 0x06]); // varint 100_000
    bytes.extend([0x41; 8]);
    assert_eq!(decode_intent(&bytes), Err(CodecError::Malformed));
}

#[test]
fn non_utf8_and_non_ulid_ability_malformed() {
    // Non-UTF8 ability bytes.
    let mut bytes = Vec::new();
    bytes.push(PROTOCOL_VERSION);
    bytes.extend(EPOCH);
    bytes.extend([LANE_COMBAT, 0x01, 0x2A, 0x07, INTENT_TAG_DECLARE_ACTION]);
    bytes.extend([0x03, 0xFF, 0xFE, 0x41]);
    bytes.push(0x09);
    assert_eq!(decode_intent(&bytes), Err(CodecError::Malformed));
    // Well-formed UTF-8, wrong length for a ULID.
    let mut bytes = Vec::new();
    bytes.push(PROTOCOL_VERSION);
    bytes.extend(EPOCH);
    bytes.extend([LANE_COMBAT, 0x01, 0x2A, 0x07, INTENT_TAG_DECLARE_ACTION]);
    postcard_extend_str(&mut bytes, "TOO-SHORT");
    bytes.push(0x09);
    assert_eq!(decode_intent(&bytes), Err(CodecError::Malformed));
}

#[test]
fn bad_bool_byte_malformed() {
    // Health op with dead = 2.
    let frame = DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 0,
        event_seq: 0,
        ops: vec![DeltaOp::Health {
            entity: net(1),
            health: 5,
            max_health: 5,
            dead: false,
        }],
    };
    let mut bytes = encode_delta(&frame).expect("encodes");
    let dead_pos = bytes.len() - 1;
    assert_eq!(bytes[dead_pos], 0x00);
    bytes[dead_pos] = 0x02;
    assert_eq!(decode_delta(&bytes), Err(CodecError::Malformed));
    // Turn presence flag = 2.
    let frame = DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 0,
        event_seq: 0,
        ops: vec![DeltaOp::Turn {
            active: None,
            round: 0,
        }],
    };
    let mut bytes = encode_delta(&frame).expect("encodes");
    bytes[5 + 1] = 0x02;
    assert_eq!(decode_delta(&bytes), Err(CodecError::Malformed));
}

/// Minimal postcard string writer for hand-built hostile frames.
fn postcard_extend_str(out: &mut Vec<u8>, text: &str) {
    let mut len = text.len() as u64;
    loop {
        let mut byte = (len & 0x7F) as u8;
        len >>= 7;
        if len != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if len == 0 {
            break;
        }
    }
    out.extend(text.as_bytes());
}

// ---------------------------------------------------------------------------
// 3. Boundaries: largest-valid / first-invalid.
// ---------------------------------------------------------------------------

#[test]
fn ops_boundary_256_valid_257_invalid() {
    let mut ops = Vec::new();
    for _ in 0..MAX_DELTA_OPS {
        ops.push(DeltaOp::Died { entity: net(1) });
    }
    let largest = DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 9,
        event_seq: 9,
        ops,
    };
    let bytes = encode_delta(&largest).expect("256 ops encode");
    assert!(bytes.len() <= MAX_DELTA_FRAME_BYTES);
    assert_eq!(decode_delta(&bytes).expect("256 ops decode"), largest);

    let mut ops = Vec::new();
    for _ in 0..MAX_DELTA_OPS + 1 {
        ops.push(DeltaOp::Died { entity: net(1) });
    }
    let first_invalid = DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 9,
        event_seq: 9,
        ops,
    };
    assert_eq!(encode_delta(&first_invalid), Err(CodecError::LimitExceeded));
    // Decode side: header + count 257, then one valid op (count fires first).
    let mut bytes = vec![PROTOCOL_VERSION, LANE_COMBAT, 0x09, 0x09];
    bytes.extend([0x81, 0x02]); // varint 257
    bytes.extend([DELTA_TAG_DIED, 0x01]);
    assert_eq!(decode_delta(&bytes), Err(CodecError::LimitExceeded));
}

#[test]
fn string_boundary_256_vs_257() {
    // 257 'A's: bound fires before ULID parsing.
    let long = "A".repeat(257);
    let mut bytes = Vec::new();
    bytes.push(PROTOCOL_VERSION);
    bytes.extend(EPOCH);
    bytes.extend([LANE_COMBAT, 0x01, 0x2A, 0x07, INTENT_TAG_DECLARE_ACTION]);
    postcard_extend_str(&mut bytes, &long);
    bytes.push(0x09);
    assert_eq!(decode_intent(&bytes), Err(CodecError::LimitExceeded));
    // 256 'A's: passes the length gate, fails ULID parsing instead.
    let edge = "A".repeat(256);
    let mut bytes = Vec::new();
    bytes.push(PROTOCOL_VERSION);
    bytes.extend(EPOCH);
    bytes.extend([LANE_COMBAT, 0x01, 0x2A, 0x07, INTENT_TAG_DECLARE_ACTION]);
    postcard_extend_str(&mut bytes, &edge);
    bytes.push(0x09);
    assert_eq!(decode_intent(&bytes), Err(CodecError::Malformed));
}

#[test]
fn largest_valid_intent_has_headroom() {
    // The largest encodable v1 intent (DeclareAction, max-range varints) must
    // clear the 4 KiB cap with room to spare; first-invalid is the cap + 1.
    let frame = IntentFrame {
        epoch: [0xFF; 16],
        lane: LANE_COMBAT,
        seq: SEQ_LAST,
        observed_tick: u64::MAX,
        actor: net(u64::MAX),
        body: IntentBody::DeclareAction {
            ability: Ulid::from_u128(u128::MAX),
            target: net(u64::MAX),
        },
    };
    let bytes = encode_intent(&frame).expect("largest intent encodes");
    assert!(bytes.len() < MAX_INTENT_FRAME_BYTES);
    assert_eq!(
        decode_intent(&bytes).expect("largest intent decodes"),
        frame
    );
    assert_eq!(
        decode_intent(&vec![0xAA; MAX_INTENT_FRAME_BYTES + 1]),
        Err(CodecError::FrameTooLarge)
    );
}

// ---------------------------------------------------------------------------
// 4. No-timing determinism: same frames in, same bytes out.
// ---------------------------------------------------------------------------

#[test]
fn codec_is_byte_deterministic() {
    for _ in 0..3 {
        assert_eq!(
            encode_intent(&declare_frame()).expect("encodes"),
            encode_intent(&declare_frame()).expect("encodes")
        );
        assert_eq!(
            encode_delta(&all_ops_delta()).expect("encodes"),
            encode_delta(&all_ops_delta()).expect("encodes")
        );
    }
    // Decode/encode round-trip is a fixed point, including every code.
    for code in all_rejection_codes() {
        let frame = DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 7,
            event_seq: 8,
            ops: vec![DeltaOp::Receipt {
                epoch: EPOCH,
                lane: LANE_COMBAT,
                seq: 3,
                processed_tick: 7,
                status: ReceiptStatus::Rejected(code),
            }],
        };
        let once = encode_delta(&frame).expect("encodes");
        let back = decode_delta(&once).expect("decodes");
        assert_eq!(encode_delta(&back).expect("re-encodes"), once);
    }
}
