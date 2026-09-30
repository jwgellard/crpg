//! T021 v2 event protocol acceptance (ADR-0019).
//!
//! Public-API only, with positive controls for every rejection. v1
//! ([`crpg_net::codec`]) is exercised directly — never a mock — to prove
//! both the version refusal and the separate unknown-tag refusal. All v2
//! wire expectations are independent hand-encoded byte oracles, not
//! rebaselines: every structural byte below was written from the frozen
//! layout (`version u8`, `lane u8`, LEB128 varints, length-prefixed
//! strings), so a codec/layout drift fails loudly. The history-driven
//! cases consume the real T020 [`HistoryWorld`](crpg_sim::HistoryWorld)
//! through the existing dev-only sim edge (no new runtime net→sim
//! dependency) with an explicit test-adapter mapping and a hand-authored
//! ordered oracle; dropping, duplicating, or swapping one same-tick event
//! fails it. No v1 fixture, replay, golden, transport, or host change.

use std::collections::BTreeMap;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_data::{
    Ability, Creature, DamageEntry, Effect, Encounter, OutcomeBandWire, OutcomeTable as DataTable,
    OutcomeWire, Placement, Ruleset,
};
use crpg_net::codec::{
    decode_delta as decode_delta_v1, decode_intent as decode_intent_v1,
    encode_delta as encode_delta_v1, encode_intent as encode_intent_v1, CodecError,
};
use crpg_net::codec_v2::{
    decode_delta as decode_delta_v2, decode_intent as decode_intent_v2,
    encode_delta as encode_delta_v2, encode_intent as encode_intent_v2,
};
use crpg_net::projection_v2::{project_events, EventCandidate, ProjectionError};
use crpg_net::protocol::{
    DeltaFrame as DeltaFrameV1, DeltaOp as DeltaOpV1, IntentBody, IntentFrame, NetId,
    ReceiptStatus, RejectionCode, LANE_COMBAT, MAX_DELTA_FRAME_BYTES, MAX_DELTA_OPS,
};
use crpg_net::protocol_v2::{
    DeltaFrame as DeltaFrameV2, DeltaOp as DeltaOpV2, DELTA_TAG_ACTION_RESOLVED,
    DELTA_TAG_ENCOUNTER_ENDED, DELTA_TAG_TURN_STARTED, PROTOCOL_VERSION,
};
use crpg_net::sim::{FaultSchedule, InMemoryTransport, QueueCaps, RateCaps, SimClock};
use crpg_sim::{
    history_hash, CombatAction, EncounterSpec, HistoryEvent, HistoryWorld, PlacementAndArea,
};
use serde_json::json;

// ---------------------------------------------------------------------------
// Shared fixtures: replica ids, identities, frames.
// ---------------------------------------------------------------------------

const EPOCH: [u8; 16] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
];

fn net(raw: u64) -> NetId {
    NetId::new(raw).expect("fixture id is nonzero")
}

/// Fixed hand-built ability identity (matches the v1 codec suite).
fn ability() -> Ulid {
    Ulid::from_parts(0x0123_4567_89AB, 0x0CDE_F012_3456_789A)
}

/// Fixed authored encounter identity for the history-driven cases.
fn encounter_id() -> Ulid {
    Ulid::from_u128(101)
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

/// Every frozen v1 delta shape, for the v1-still-passes and old-vocabulary
/// equivalence controls.
fn all_ops_delta_v1() -> DeltaFrameV1 {
    DeltaFrameV1 {
        lane: LANE_COMBAT,
        server_tick: 100,
        event_seq: 55,
        ops: vec![
            DeltaOpV1::EntityEnter { entity: net(1) },
            DeltaOpV1::EntityLeave { entity: net(2) },
            DeltaOpV1::Spawned { entity: net(3) },
            DeltaOpV1::Despawned { entity: net(4) },
            DeltaOpV1::Died { entity: net(5) },
            DeltaOpV1::Health {
                entity: net(6),
                health: 3,
                max_health: 12,
                dead: false,
            },
            DeltaOpV1::Turn {
                active: Some(net(6)),
                round: 2,
            },
            DeltaOpV1::Turn {
                active: None,
                round: 3,
            },
            DeltaOpV1::Receipt {
                epoch: EPOCH,
                lane: LANE_COMBAT,
                seq: 1,
                processed_tick: 99,
                status: ReceiptStatus::Applied,
            },
            DeltaOpV1::Receipt {
                epoch: EPOCH,
                lane: LANE_COMBAT,
                seq: 2,
                processed_tick: 100,
                status: ReceiptStatus::Rejected(RejectionCode::IllegalAction),
            },
        ],
    }
}

/// The same vocabulary wrapped for v2: [`DeltaOpV2::Legacy`] only.
fn all_ops_delta_v2_legacy(source: &DeltaFrameV1) -> DeltaFrameV2 {
    DeltaFrameV2 {
        lane: source.lane,
        server_tick: source.server_tick,
        event_seq: source.event_seq,
        ops: source.ops.iter().cloned().map(DeltaOpV2::Legacy).collect(),
    }
}

/// Unsigned LEB128, the `postcard` integer/string-length encoding the
/// oracles below are hand-checked against.
fn leb128(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            return out;
        }
    }
}

// ---------------------------------------------------------------------------
// v1_and_v2_are_explicit
// ---------------------------------------------------------------------------

#[test]
fn v1_and_v2_are_explicit() {
    // Every frozen v1 shape still round-trips through v1 (positive control
    // that v1 is intact; the full frozen suite lives in tests/codec.rs).
    assert_eq!(
        decode_intent_v1(&encode_intent_v1(&declare_frame()).expect("v1 encodes")),
        Ok(declare_frame())
    );
    assert_eq!(
        decode_intent_v1(&encode_intent_v1(&end_turn_frame()).expect("v1 encodes")),
        Ok(end_turn_frame())
    );
    let v1_frame = all_ops_delta_v1();
    assert_eq!(
        decode_delta_v1(&encode_delta_v1(&v1_frame).expect("v1 encodes")),
        Ok(v1_frame.clone())
    );

    // Equivalent old-vocabulary v2 bytes differ from v1 only in the version
    // byte: Legacy contributes no extra tag.
    let v1_bytes = encode_delta_v1(&v1_frame).expect("v1 encodes");
    let v2_legacy = all_ops_delta_v2_legacy(&v1_frame);
    let v2_bytes = encode_delta_v2(&v2_legacy).expect("v2 encodes");
    assert_eq!(v1_bytes[0], 1);
    assert_eq!(v2_bytes[0], PROTOCOL_VERSION);
    assert_eq!(v2_bytes[0], 2);
    assert_eq!(v2_bytes.len(), v1_bytes.len(), "Legacy adds no wire bytes");
    assert_eq!(&v2_bytes[1..], &v1_bytes[1..]);
    assert_eq!(decode_delta_v2(&v2_bytes).expect("v2 decodes"), v2_legacy);
    let v1_intent = encode_intent_v1(&declare_frame()).expect("v1 encodes");
    let v2_intent = encode_intent_v2(&declare_frame()).expect("v2 encodes");
    assert_eq!(v2_intent[0], 2);
    assert_eq!(&v2_intent[1..], &v1_intent[1..]);

    // A v1 peer on valid v2 traffic reports UnsupportedVersion — for a real
    // event frame, not just old vocabulary.
    let event_frame = DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 1,
        event_seq: 1,
        ops: vec![DeltaOpV2::TurnStarted {
            actor: net(3),
            round: 0,
        }],
    };
    let event_bytes = encode_delta_v2(&event_frame).expect("v2 encodes");
    assert_eq!(
        decode_delta_v1(&event_bytes),
        Err(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        decode_intent_v1(&v2_intent),
        Err(CodecError::UnsupportedVersion)
    );

    // v2 refuses v1 traffic the same explicit way: no autodetect.
    assert_eq!(
        decode_delta_v2(&v1_bytes),
        Err(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        decode_intent_v2(&v1_intent),
        Err(CodecError::UnsupportedVersion)
    );

    // Each new tag inside an otherwise valid v1 envelope is UnknownMessage.
    // These are separate tests from the version refusal above: the version
    // is 1 and the header is well-formed, so only the tag dispatch fails.
    for tag in [
        DELTA_TAG_ACTION_RESOLVED,
        DELTA_TAG_TURN_STARTED,
        DELTA_TAG_ENCOUNTER_ENDED,
    ] {
        let mut envelope = vec![1u8, LANE_COMBAT];
        envelope.extend(leb128(1)); // server_tick
        envelope.extend(leb128(1)); // event_seq
        envelope.extend(leb128(1)); // op_count
        envelope.push(tag);
        assert_eq!(
            decode_delta_v1(&envelope),
            Err(CodecError::UnknownMessage),
            "v1 rejects new tag {tag}"
        );
    }

    // v2 refuses a tag past its own vocabulary (11), even with payload
    // bytes after it: the tag dispatch fires before any field read.
    let mut beyond = vec![2u8, LANE_COMBAT];
    beyond.extend(leb128(1));
    beyond.extend(leb128(1));
    beyond.extend(leb128(1));
    beyond.push(11);
    beyond.extend([0x00, 0x00]);
    assert_eq!(decode_delta_v2(&beyond), Err(CodecError::UnknownMessage));
}

// ---------------------------------------------------------------------------
// new_tags_have_literal_byte_oracles
// ---------------------------------------------------------------------------

/// The 26 ASCII `0` bytes of [`Ulid::NIL`], whose canonical text the v1
/// suite pins; the length prefix 26 (`0x1A`) is written literally.
fn nil_text(out: &mut Vec<u8>) {
    out.extend([b'0'; 26]);
}

#[test]
fn new_tags_have_literal_byte_oracles() {
    // Minimum ActionResolved: hand-encoded from the frozen layout.
    let action_min = DeltaOpV2::ActionResolved {
        actor: net(1),
        target: net(2),
        ability: Ulid::NIL,
        outcome: "success".to_owned(),
        damage: 0,
    };
    let frame_min = DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 1,
        event_seq: 1,
        ops: vec![action_min.clone()],
    };
    let mut expected = vec![2u8, 0, 1, 1, 1, 8, 1, 2, 0x1A];
    nil_text(&mut expected);
    expected.extend([0x07]);
    expected.extend(b"success");
    expected.push(0x00);
    assert_eq!(
        encode_delta_v2(&frame_min).expect("encodes"),
        expected,
        "minimum ActionResolved oracle"
    );
    assert_eq!(decode_delta_v2(&expected).expect("decodes"), frame_min);

    // Minimum TurnStarted.
    let turn_min = DeltaOpV2::TurnStarted {
        actor: net(1),
        round: 0,
    };
    let frame_turn_min = DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 0,
        event_seq: 1,
        ops: vec![turn_min.clone()],
    };
    let expected_turn_min = vec![2u8, 0, 0, 1, 1, 9, 1, 0];
    assert_eq!(
        encode_delta_v2(&frame_turn_min).expect("encodes"),
        expected_turn_min
    );
    assert_eq!(
        decode_delta_v2(&expected_turn_min).expect("decodes"),
        frame_turn_min
    );

    // TurnStarted varint boundary: actor 128 -> [0x80, 0x01], round 300 ->
    // [0xAC, 0x02] (300 = 2 + 44*128... low 7 bits 0x2C with continuation).
    assert_eq!(leb128(128), vec![0x80, 0x01]);
    assert_eq!(leb128(300), vec![0xAC, 0x02]);
    let turn_wide = DeltaOpV2::TurnStarted {
        actor: net(128),
        round: 300,
    };
    let frame_turn_wide = DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 3,
        event_seq: 7,
        ops: vec![turn_wide.clone()],
    };
    let expected_turn_wide = vec![2u8, 0, 3, 7, 1, 9, 0x80, 0x01, 0xAC, 0x02];
    assert_eq!(
        encode_delta_v2(&frame_turn_wide).expect("encodes"),
        expected_turn_wide,
        "TurnStarted varint-boundary oracle"
    );
    assert_eq!(
        decode_delta_v2(&expected_turn_wide).expect("decodes"),
        frame_turn_wide
    );

    // Minimum EncounterEnded.
    let end_min = DeltaOpV2::EncounterEnded {
        encounter: Ulid::NIL,
        round: 0,
    };
    let frame_end_min = DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 0,
        event_seq: 1,
        ops: vec![end_min.clone()],
    };
    let mut expected_end_min = vec![2u8, 0, 0, 1, 1, 10, 0x1A];
    nil_text(&mut expected_end_min);
    expected_end_min.push(0x00);
    assert_eq!(
        encode_delta_v2(&frame_end_min).expect("encodes"),
        expected_end_min
    );
    assert_eq!(
        decode_delta_v2(&expected_end_min).expect("decodes"),
        frame_end_min
    );

    // EncounterEnded boundary: identity from_u128(31) is 25 zeros then 'Z'
    // (base-32 value 31), round 16384 = 2^14 -> [0x80, 0x80, 0x01].
    assert_eq!(leb128(16384), vec![0x80, 0x80, 0x01]);
    assert_eq!(
        Ulid::from_u128(31).to_string(),
        "0000000000000000000000000Z"
    );
    let end_wide = DeltaOpV2::EncounterEnded {
        encounter: Ulid::from_u128(31),
        round: 16384,
    };
    let frame_end_wide = DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 5,
        event_seq: 9,
        ops: vec![end_wide.clone()],
    };
    let mut expected_end_wide = vec![2u8, 0, 5, 9, 1, 10, 0x1A];
    expected_end_wide.extend([b'0'; 25]);
    expected_end_wide.push(b'Z');
    expected_end_wide.extend([0x80, 0x80, 0x01]);
    assert_eq!(
        encode_delta_v2(&frame_end_wide).expect("encodes"),
        expected_end_wide,
        "EncounterEnded varint-boundary oracle"
    );
    assert_eq!(
        decode_delta_v2(&expected_end_wide).expect("decodes"),
        frame_end_wide
    );

    // ActionResolved boundary: actor 300, target 128, custom outcome,
    // damage 16383 = 2^14 - 1 -> [0xFF, 0x7F].
    assert_eq!(leb128(16383), vec![0xFF, 0x7F]);
    let action_wide = DeltaOpV2::ActionResolved {
        actor: net(300),
        target: net(128),
        ability: Ulid::from_u128(31),
        outcome: "custom:255".to_owned(),
        damage: 16383,
    };
    let frame_action_wide = DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 2,
        event_seq: 4,
        ops: vec![action_wide.clone()],
    };
    let mut expected_action_wide = vec![2u8, 0, 2, 4, 1, 8, 0xAC, 0x02, 0x80, 0x01, 0x1A];
    expected_action_wide.extend([b'0'; 25]);
    expected_action_wide.push(b'Z');
    expected_action_wide.push(0x0A);
    expected_action_wide.extend(b"custom:255");
    expected_action_wide.extend([0xFF, 0x7F]);
    assert_eq!(
        encode_delta_v2(&frame_action_wide).expect("encodes"),
        expected_action_wide,
        "ActionResolved varint-boundary oracle"
    );
    assert_eq!(
        decode_delta_v2(&expected_action_wide).expect("decodes"),
        frame_action_wide
    );

    // Core alias decoding: lowercase ULID text parses, re-encode is
    // canonical uppercase.
    let mut aliased = vec![2u8, 0, 2, 4, 1, 8, 0xAC, 0x02, 0x80, 0x01, 0x1A];
    aliased.extend([b'0'; 25]);
    aliased.push(b'z');
    aliased.push(0x0A);
    aliased.extend(b"custom:255");
    aliased.extend([0xFF, 0x7F]);
    assert_eq!(
        decode_delta_v2(&aliased).expect("alias decodes"),
        frame_action_wide,
        "lowercase alias parses to the same identity"
    );
    assert_eq!(
        encode_delta_v2(&frame_action_wide).expect("encodes"),
        expected_action_wide,
        "re-encode is canonical uppercase"
    );

    // Mixed old/new op order is preserved byte-exactly through the round
    // trip: Health, ActionResolved, Turn(None), TurnStarted, EncounterEnded.
    let mixed = DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 11,
        event_seq: 12,
        ops: vec![
            DeltaOpV2::Legacy(DeltaOpV1::Health {
                entity: net(3),
                health: 5,
                max_health: 10,
                dead: false,
            }),
            action_min.clone(),
            DeltaOpV2::Legacy(DeltaOpV1::Turn {
                active: None,
                round: 4,
            }),
            DeltaOpV2::TurnStarted {
                actor: net(5),
                round: 6,
            },
            DeltaOpV2::EncounterEnded {
                encounter: Ulid::NIL,
                round: 7,
            },
        ],
    };
    let mut expected_mixed = vec![2u8, 0, 11, 12, 5];
    expected_mixed.extend([5, 3, 5, 10, 0]); // Health
    expected_mixed.extend([8, 1, 2, 0x1A]); // ActionResolved head
    nil_text(&mut expected_mixed);
    expected_mixed.extend([0x07]);
    expected_mixed.extend(b"success");
    expected_mixed.push(0x00);
    expected_mixed.extend([6, 0, 4]); // Turn(None, 4)
    expected_mixed.extend([9, 5, 6]); // TurnStarted
    expected_mixed.extend([10, 0x1A]); // EncounterEnded head
    nil_text(&mut expected_mixed);
    expected_mixed.push(0x07);
    assert_eq!(
        encode_delta_v2(&mixed).expect("encodes"),
        expected_mixed,
        "mixed old/new order oracle"
    );
    let decoded = decode_delta_v2(&expected_mixed).expect("decodes");
    assert_eq!(decoded, mixed);
    assert_eq!(
        decoded.ops,
        vec![
            DeltaOpV2::Legacy(DeltaOpV1::Health {
                entity: net(3),
                health: 5,
                max_health: 10,
                dead: false,
            }),
            action_min,
            DeltaOpV2::Legacy(DeltaOpV1::Turn {
                active: None,
                round: 4,
            }),
            DeltaOpV2::TurnStarted {
                actor: net(5),
                round: 6,
            },
            DeltaOpV2::EncounterEnded {
                encounter: Ulid::NIL,
                round: 7,
            },
        ]
    );
}

// ---------------------------------------------------------------------------
// new_fields_are_bounded_before_allocation
// ---------------------------------------------------------------------------

/// A v2 delta carrying exactly one [`DeltaOpV2::ActionResolved`] built from
/// raw payload bytes, for hostile-prefix probes.
fn action_frame_head() -> Vec<u8> {
    let mut head = vec![2u8, LANE_COMBAT];
    head.extend(leb128(1)); // server_tick
    head.extend(leb128(1)); // event_seq
    head.extend(leb128(1)); // op_count
    head.push(DELTA_TAG_ACTION_RESOLVED);
    head.extend(leb128(1)); // actor
    head.extend(leb128(2)); // target
    head
}

/// The standard 26-byte canonical ULID prefix plus text.
fn ulid_prefix(text: &str) -> Vec<u8> {
    assert_eq!(text.len(), 26);
    let mut out = leb128(26);
    out.extend(text.as_bytes());
    out
}

#[test]
fn new_fields_are_bounded_before_allocation() {
    // Every string-bearing path rejects overflowing/truncated length
    // varints, while representable oversized lengths hit the bound before
    // looking for a body. Overflowing prefixes would wrap to valid lengths.
    let mut outcome_head = action_frame_head();
    outcome_head.extend(ulid_prefix(&Ulid::NIL.to_string()));
    let intent_bytes = encode_intent_v2(&declare_frame()).expect("encodes");
    let intent_head = intent_bytes[..intent_bytes.len() - 28].to_vec();
    let nil = Ulid::NIL.to_string();
    for (head, is_intent, text, tail) in [
        (
            action_frame_head(),
            false,
            nil.as_str(),
            b"\x07success\x00".as_slice(),
        ),
        (outcome_head, false, "success", b"\x00".as_slice()),
        (
            vec![2, 0, 0, 1, 1, 10],
            false,
            nil.as_str(),
            b"\x00".as_slice(),
        ),
        (intent_head, true, nil.as_str(), b"\x09".as_slice()),
    ] {
        let mut valid = head.clone();
        valid.extend(leb128(text.len() as u64));
        valid.extend(text.as_bytes());
        valid.extend(tail);
        if is_intent {
            assert!(decode_intent_v2(&valid).is_ok());
        } else {
            assert!(decode_delta_v2(&valid).is_ok());
        }
        let mut overflow = vec![0x80; 10];
        overflow[0] |= text.len() as u8;
        overflow[9] = 2;
        for (length, expected) in [
            (overflow, CodecError::Malformed),
            (vec![0x80; 10], CodecError::Malformed),
            (vec![0x80], CodecError::Malformed),
            (leb128(u64::MAX), CodecError::LimitExceeded),
            (leb128(257), CodecError::LimitExceeded),
        ] {
            let mut bytes = head.clone();
            bytes.extend(&length);
            let decode_error = |bytes: &[u8]| {
                if is_intent {
                    decode_intent_v2(bytes).unwrap_err()
                } else {
                    decode_delta_v2(bytes).unwrap_err()
                }
            };
            assert_eq!(decode_error(&bytes), expected);
            if length.last() == Some(&2) && length.len() == 10 {
                bytes.extend(text.as_bytes());
                bytes.extend(tail);
                assert_eq!(decode_error(&bytes), CodecError::Malformed);
            }
        }
    }

    // 256-byte outcome bodies reach the length gate: encode reports the
    // bound only past 256, and a fitting-but-hostile body reaches
    // vocabulary instead. 257 bytes fail on both sides.
    let outcome_256 = "a".repeat(256);
    let outcome_257 = "a".repeat(257);
    let long_bad_vocab = DeltaOpV2::ActionResolved {
        actor: net(1),
        target: net(2),
        ability: Ulid::NIL,
        outcome: outcome_256.clone(),
        damage: 0,
    };
    // 256 ASCII bytes pass the length gate, then fail vocabulary as
    // Malformed (positive control that 256 is not the bound).
    assert_eq!(
        encode_delta_v2(&DeltaFrameV2 {
            lane: LANE_COMBAT,
            server_tick: 1,
            event_seq: 1,
            ops: vec![long_bad_vocab],
        }),
        Err(CodecError::Malformed)
    );
    let overlong = DeltaOpV2::ActionResolved {
        actor: net(1),
        target: net(2),
        ability: Ulid::NIL,
        outcome: outcome_257,
        damage: 0,
    };
    assert_eq!(
        encode_delta_v2(&DeltaFrameV2 {
            lane: LANE_COMBAT,
            server_tick: 1,
            event_seq: 1,
            ops: vec![overlong],
        }),
        Err(CodecError::LimitExceeded)
    );
    for (len, body_present) in [(257usize, true), (100_000, false)] {
        let mut bytes = action_frame_head();
        bytes.extend(ulid_prefix(&Ulid::NIL.to_string()));
        bytes.extend(leb128(len as u64));
        if body_present {
            bytes.extend(vec![b'a'; len.min(300)]);
        }
        // Even with the full body present, 257+ is LimitExceeded; the
        // hostile 100_000 length with an absent body is LimitExceeded too,
        // never Malformed truncation — the bound fires before the body is
        // inspected or copied.
        assert_eq!(
            decode_delta_v2(&bytes),
            Err(CodecError::LimitExceeded),
            "outcome length {len}"
        );
    }
    // A fitting 256-byte body that is not vocabulary reaches semantics:
    // Malformed, proving 256 passes the length gate on decode as well.
    let mut fitting = action_frame_head();
    fitting.extend(ulid_prefix(&Ulid::NIL.to_string()));
    fitting.extend(leb128(256));
    fitting.extend(vec![b'a'; 256]);
    fitting.extend(leb128(0)); // damage
    assert_eq!(decode_delta_v2(&fitting), Err(CodecError::Malformed));

    // Hostile ability length with an absent body: LimitExceeded, not
    // truncation. Positive control: the same shape with a real body
    // decodes.
    let mut hostile_ability = action_frame_head();
    hostile_ability.extend(leb128(16384));
    assert_eq!(
        decode_delta_v2(&hostile_ability),
        Err(CodecError::LimitExceeded)
    );
    let mut good_ability = action_frame_head();
    good_ability.extend(ulid_prefix(&Ulid::NIL.to_string()));
    good_ability.extend(leb128(7));
    good_ability.extend(b"success");
    good_ability.extend(leb128(0));
    assert!(
        decode_delta_v2(&good_ability).is_ok(),
        "positive control decodes"
    );

    // Op-count bound: 256 decodes and encodes, 257 fails on both sides
    // without allocating from the untrusted count.
    let ops_256: Vec<DeltaOpV2> = (0..256)
        .map(|_| DeltaOpV2::Legacy(DeltaOpV1::Died { entity: net(1) }))
        .collect();
    let frame_256 = DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 1,
        event_seq: 1,
        ops: ops_256,
    };
    let bytes_256 = encode_delta_v2(&frame_256).expect("256 ops encode");
    assert_eq!(
        decode_delta_v2(&bytes_256)
            .expect("256 ops decode")
            .ops
            .len(),
        256
    );
    let mut ops_257 = frame_256.ops.clone();
    ops_257.push(DeltaOpV2::Legacy(DeltaOpV1::Died { entity: net(1) }));
    assert_eq!(
        encode_delta_v2(&DeltaFrameV2 {
            lane: LANE_COMBAT,
            server_tick: 1,
            event_seq: 1,
            ops: ops_257,
        }),
        Err(CodecError::LimitExceeded)
    );
    let mut count_257 = vec![2u8, LANE_COMBAT];
    count_257.extend(leb128(1));
    count_257.extend(leb128(1));
    count_257.extend(leb128(257));
    assert_eq!(decode_delta_v2(&count_257), Err(CodecError::LimitExceeded));

    // Exact frame cap passes through to the trailing-byte check; cap + 1
    // is FrameTooLarge however broken the rest is.
    let small = encode_delta_v2(&DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 1,
        event_seq: 1,
        ops: vec![DeltaOpV2::TurnStarted {
            actor: net(1),
            round: 0,
        }],
    })
    .expect("encodes");
    assert!(small.len() < MAX_DELTA_FRAME_BYTES);
    let mut at_cap = small.clone();
    at_cap.extend(vec![0u8; MAX_DELTA_FRAME_BYTES - small.len()]);
    assert_eq!(at_cap.len(), MAX_DELTA_FRAME_BYTES);
    assert_eq!(
        decode_delta_v2(&at_cap),
        Err(CodecError::Malformed),
        "exact cap reaches the trailing-byte check"
    );
    let mut over_cap = small.clone();
    over_cap.extend(vec![0u8; MAX_DELTA_FRAME_BYTES - small.len() + 1]);
    assert_eq!(decode_delta_v2(&over_cap), Err(CodecError::FrameTooLarge));
    // Cap fires before version: a cap+1 frame with a bad version is still
    // FrameTooLarge (multifault precedence).
    let mut over_bad_version = over_cap.clone();
    over_bad_version[0] = 9;
    assert_eq!(
        decode_delta_v2(&over_bad_version),
        Err(CodecError::FrameTooLarge)
    );

    // Zero replica ids are Malformed wherever read (encode cannot express
    // them: NetId::new(0) is None, the positive construction control).
    assert_eq!(NetId::new(0), None);
    for op_bytes in [
        vec![DELTA_TAG_ACTION_RESOLVED, 0x00],
        vec![DELTA_TAG_TURN_STARTED, 0x00],
    ] {
        let mut bytes = vec![2u8, LANE_COMBAT];
        bytes.extend(leb128(1));
        bytes.extend(leb128(1));
        bytes.extend(leb128(1));
        bytes.extend(op_bytes);
        assert_eq!(decode_delta_v2(&bytes), Err(CodecError::Malformed));
    }
    // Zero target with a nonzero actor is Malformed too.
    let mut zero_target = vec![2u8, LANE_COMBAT];
    zero_target.extend(leb128(1));
    zero_target.extend(leb128(1));
    zero_target.extend(leb128(1));
    zero_target.push(DELTA_TAG_ACTION_RESOLVED);
    zero_target.extend(leb128(1));
    zero_target.extend(leb128(0));
    assert_eq!(decode_delta_v2(&zero_target), Err(CodecError::Malformed));

    // Intent and receipt sequences exhaust at 0 and u64::MAX on both
    // sides; the delta event_seq accepts the full u64 range instead.
    let mut intent = declare_frame();
    for seq in [0u64, u64::MAX] {
        intent.seq = seq;
        assert_eq!(encode_intent_v2(&intent), Err(CodecError::Malformed));
    }
    for seq_bytes in [leb128(0), leb128(u64::MAX)] {
        // epoch occupies bytes 1..17, lane is byte 17, seq follows;
        // rebuild the frame bytes around the hostile seq instead.
        let mut raw = vec![2u8];
        raw.extend(declare_frame().epoch);
        raw.push(LANE_COMBAT);
        raw.extend(seq_bytes);
        raw.extend(leb128(declare_frame().observed_tick));
        raw.extend(leb128(declare_frame().actor.get()));
        raw.push(0);
        raw.extend(ulid_prefix(&ability().to_string()));
        raw.extend(leb128(9));
        assert_eq!(decode_intent_v2(&raw), Err(CodecError::Malformed));
    }
    let receipt_frame = |seq: u64| DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 1,
        event_seq: 1,
        ops: vec![DeltaOpV2::Legacy(DeltaOpV1::Receipt {
            epoch: EPOCH,
            lane: LANE_COMBAT,
            seq,
            processed_tick: 1,
            status: ReceiptStatus::Applied,
        })],
    };
    for seq in [0u64, u64::MAX] {
        assert_eq!(
            encode_delta_v2(&receipt_frame(seq)),
            Err(CodecError::Malformed)
        );
    }
    // Delta event_seq 0 and u64::MAX are accepted: full u64 range.
    for event_seq in [0u64, u64::MAX] {
        let frame = DeltaFrameV2 {
            lane: LANE_COMBAT,
            server_tick: 1,
            event_seq,
            ops: vec![],
        };
        let bytes = encode_delta_v2(&frame).expect("full-range seq encodes");
        assert_eq!(
            decode_delta_v2(&bytes).expect("full-range seq decodes"),
            frame
        );
    }
    let mut max_seq = vec![2u8, LANE_COMBAT];
    max_seq.extend(leb128(1));
    max_seq.extend(leb128(u64::MAX));
    max_seq.extend(leb128(0));
    assert_eq!(
        decode_delta_v2(&max_seq)
            .expect("max event_seq decodes")
            .event_seq,
        u64::MAX
    );

    // Invalid custom outcomes are Malformed on both sides; the four words
    // and the decimal edges round-trip.
    for bad in [
        "custom:",
        "custom:256",
        "custom:00",
        "custom:01",
        "custom:-1",
        "custom:1a",
        "",
        "Success",
        "custom:255 ",
        "custom: 1",
    ] {
        let op = DeltaOpV2::ActionResolved {
            actor: net(1),
            target: net(2),
            ability: Ulid::NIL,
            outcome: bad.to_owned(),
            damage: 0,
        };
        assert_eq!(
            encode_delta_v2(&DeltaFrameV2 {
                lane: LANE_COMBAT,
                server_tick: 1,
                event_seq: 1,
                ops: vec![op],
            }),
            Err(CodecError::Malformed),
            "encode rejects {bad:?}"
        );
        let mut bytes = action_frame_head();
        bytes.extend(ulid_prefix(&Ulid::NIL.to_string()));
        bytes.extend(leb128(bad.len() as u64));
        bytes.extend(bad.as_bytes());
        bytes.extend(leb128(0));
        assert_eq!(
            decode_delta_v2(&bytes),
            Err(CodecError::Malformed),
            "decode rejects {bad:?}"
        );
    }
    for good in [
        "critical_success",
        "success",
        "failure",
        "critical_failure",
        "custom:0",
        "custom:9",
        "custom:10",
        "custom:255",
    ] {
        let frame = DeltaFrameV2 {
            lane: LANE_COMBAT,
            server_tick: 1,
            event_seq: 1,
            ops: vec![DeltaOpV2::ActionResolved {
                actor: net(1),
                target: net(2),
                ability: Ulid::NIL,
                outcome: good.to_owned(),
                damage: 1,
            }],
        };
        let bytes = encode_delta_v2(&frame).expect("valid outcome encodes");
        assert_eq!(
            decode_delta_v2(&bytes).expect("valid outcome decodes"),
            frame
        );
    }

    // Trailing bytes and truncation are Malformed; multifault inputs prove
    // first-failure-wins precedence.
    let valid = encode_delta_v2(&DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 1,
        event_seq: 1,
        ops: vec![DeltaOpV2::TurnStarted {
            actor: net(1),
            round: 0,
        }],
    })
    .expect("encodes");
    let mut trailing = valid.clone();
    trailing.push(0x00);
    assert_eq!(decode_delta_v2(&trailing), Err(CodecError::Malformed));
    assert_eq!(
        decode_delta_v2(&valid[..valid.len() - 1]),
        Err(CodecError::Malformed)
    );
    // Bad version beats bad lane beats unknown tag.
    let mut bad_version = trailing.clone();
    bad_version[0] = 9;
    bad_version[1] = 9;
    assert_eq!(
        decode_delta_v2(&bad_version),
        Err(CodecError::UnsupportedVersion)
    );
    let mut bad_lane = valid.clone();
    bad_lane[1] = 9;
    bad_lane.push(11);
    assert_eq!(decode_delta_v2(&bad_lane), Err(CodecError::WrongDirection));
    let mut unknown_tag = valid.clone();
    unknown_tag.pop();
    unknown_tag.pop();
    unknown_tag.pop();
    unknown_tag.extend([11, 0x00]);
    assert_eq!(
        decode_delta_v2(&unknown_tag),
        Err(CodecError::UnknownMessage)
    );

    // Receipt with a foreign lane is WrongDirection on both sides.
    let foreign = DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 1,
        event_seq: 1,
        ops: vec![DeltaOpV2::Legacy(DeltaOpV1::Receipt {
            epoch: EPOCH,
            lane: 1,
            seq: 1,
            processed_tick: 1,
            status: ReceiptStatus::Applied,
        })],
    };
    assert_eq!(encode_delta_v2(&foreign), Err(CodecError::WrongDirection));
    let good_receipt = encode_delta_v2(&receipt_frame(1)).expect("encodes");
    // Receipt payload layout: tag 7, epoch 16, lane, seq, tick, status.
    // The lane byte sits at offset 5 (header) + 1 (tag) + 16 (epoch).
    let mut foreign_wire = good_receipt.clone();
    foreign_wire[5 + 1 + 16] = 1;
    assert_eq!(
        decode_delta_v2(&foreign_wire),
        Err(CodecError::WrongDirection)
    );

    // Unknown receipt status and code are UnknownMessage.
    let mut bad_status = good_receipt.clone();
    let status_at = bad_status.len() - 1;
    bad_status[status_at] = 7;
    assert_eq!(
        decode_delta_v2(&bad_status),
        Err(CodecError::UnknownMessage)
    );
    let rejected = DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 1,
        event_seq: 1,
        ops: vec![DeltaOpV2::Legacy(DeltaOpV1::Receipt {
            epoch: EPOCH,
            lane: LANE_COMBAT,
            seq: 1,
            processed_tick: 1,
            status: ReceiptStatus::Rejected(RejectionCode::IllegalAction),
        })],
    };
    let mut bad_code = encode_delta_v2(&rejected).expect("encodes");
    let code_at = bad_code.len() - 1;
    bad_code[code_at] = 21;
    assert_eq!(decode_delta_v2(&bad_code), Err(CodecError::UnknownMessage));

    // Intent bound: oversized intent is FrameTooLarge; wrong lane and
    // unknown body tag map exactly.
    let mut big_intent = encode_intent_v2(&declare_frame()).expect("encodes");
    big_intent.extend(vec![0u8; 4096]);
    assert_eq!(
        decode_intent_v2(&big_intent),
        Err(CodecError::FrameTooLarge)
    );
    let mut wrong_lane = declare_frame();
    wrong_lane.lane = 1;
    assert_eq!(
        encode_intent_v2(&wrong_lane),
        Err(CodecError::WrongDirection)
    );
    let mut unknown_body = encode_intent_v2(&declare_frame()).expect("encodes");
    let tag_at = unknown_body.len() - 1 - 26 - 1 - 1;
    unknown_body[tag_at] = 9;
    assert_eq!(
        decode_intent_v2(&unknown_body),
        Err(CodecError::UnknownMessage)
    );
}

// ---------------------------------------------------------------------------
// field_by_field_suppression
// ---------------------------------------------------------------------------

fn permitted_action() -> EventCandidate {
    EventCandidate::ActionResolved {
        actor: Some(net(1)),
        target: Some(net(2)),
        ability: Some(Ulid::NIL),
        outcome: Some("success".to_owned()),
        damage: Some(2),
    }
}

#[test]
fn field_by_field_suppression() {
    // All fields permitted: exactly one op.
    let expected_action = DeltaOpV2::ActionResolved {
        actor: net(1),
        target: net(2),
        ability: Ulid::NIL,
        outcome: "success".to_owned(),
        damage: 2,
    };
    assert_eq!(
        project_events(&[permitted_action()]).expect("projects"),
        vec![expected_action.clone()]
    );

    // Removing each permission individually suppresses the whole event.
    let removals: Vec<EventCandidate> = vec![
        EventCandidate::ActionResolved {
            actor: None,
            target: Some(net(2)),
            ability: Some(Ulid::NIL),
            outcome: Some("success".to_owned()),
            damage: Some(2),
        },
        EventCandidate::ActionResolved {
            actor: Some(net(1)),
            target: None,
            ability: Some(Ulid::NIL),
            outcome: Some("success".to_owned()),
            damage: Some(2),
        },
        EventCandidate::ActionResolved {
            actor: Some(net(1)),
            target: Some(net(2)),
            ability: None,
            outcome: Some("success".to_owned()),
            damage: Some(2),
        },
        EventCandidate::ActionResolved {
            actor: Some(net(1)),
            target: Some(net(2)),
            ability: Some(Ulid::NIL),
            outcome: None,
            damage: Some(2),
        },
        EventCandidate::ActionResolved {
            actor: Some(net(1)),
            target: Some(net(2)),
            ability: Some(Ulid::NIL),
            outcome: Some("success".to_owned()),
            damage: None,
        },
    ];
    for (index, removal) in removals.iter().enumerate() {
        assert_eq!(
            project_events(std::slice::from_ref(removal)).expect("projects"),
            Vec::<DeltaOpV2>::new(),
            "removal {index} suppresses the whole event"
        );
    }

    // Hidden payload values are never inspected: a forbidden actor plus a
    // non-vocabulary outcome still omits quietly instead of erroring.
    assert_eq!(
        project_events(&[EventCandidate::ActionResolved {
            actor: None,
            target: Some(net(2)),
            ability: Some(Ulid::NIL),
            outcome: Some("bogus-outcome!!!".to_owned()),
            damage: Some(2),
        }])
        .expect("projects"),
        Vec::<DeltaOpV2>::new()
    );
    // Same for an oversized hidden outcome: suppression, not StringLimit.
    assert_eq!(
        project_events(&[EventCandidate::ActionResolved {
            actor: Some(net(1)),
            target: None,
            ability: Some(Ulid::NIL),
            outcome: Some("a".repeat(300)),
            damage: Some(2),
        }])
        .expect("projects"),
        Vec::<DeltaOpV2>::new()
    );

    // TurnStarted: each of actor and round is required.
    assert_eq!(
        project_events(&[EventCandidate::TurnStarted {
            actor: Some(net(3)),
            round: Some(1),
        }])
        .expect("projects"),
        vec![DeltaOpV2::TurnStarted {
            actor: net(3),
            round: 1,
        }]
    );
    for partial in [
        EventCandidate::TurnStarted {
            actor: None,
            round: Some(1),
        },
        EventCandidate::TurnStarted {
            actor: Some(net(3)),
            round: None,
        },
    ] {
        assert_eq!(
            project_events(std::slice::from_ref(&partial)).expect("projects"),
            Vec::<DeltaOpV2>::new()
        );
    }

    // EncounterEnded: each of encounter and round is required.
    assert_eq!(
        project_events(&[EventCandidate::EncounterEnded {
            encounter: Some(encounter_id()),
            round: Some(0),
        }])
        .expect("projects"),
        vec![DeltaOpV2::EncounterEnded {
            encounter: encounter_id(),
            round: 0,
        }]
    );
    for partial in [
        EventCandidate::EncounterEnded {
            encounter: None,
            round: Some(0),
        },
        EventCandidate::EncounterEnded {
            encounter: Some(encounter_id()),
            round: None,
        },
    ] {
        assert_eq!(
            project_events(std::slice::from_ref(&partial)).expect("projects"),
            Vec::<DeltaOpV2>::new()
        );
    }

    // Structural notices need both the mapping and the cause grant.
    for (variant, op) in [
        (
            EventCandidate::Spawned {
                entity: Some(net(4)),
                disclose: true,
            },
            DeltaOpV2::Legacy(DeltaOpV1::Spawned { entity: net(4) }),
        ),
        (
            EventCandidate::Despawned {
                entity: Some(net(4)),
                disclose: true,
            },
            DeltaOpV2::Legacy(DeltaOpV1::Despawned { entity: net(4) }),
        ),
        (
            EventCandidate::Died {
                entity: Some(net(4)),
                disclose: true,
            },
            DeltaOpV2::Legacy(DeltaOpV1::Died { entity: net(4) }),
        ),
    ] {
        assert_eq!(
            project_events(std::slice::from_ref(&variant)).expect("projects"),
            vec![op]
        );
    }
    for hidden in [
        EventCandidate::Spawned {
            entity: None,
            disclose: true,
        },
        EventCandidate::Spawned {
            entity: Some(net(4)),
            disclose: false,
        },
        EventCandidate::Despawned {
            entity: Some(net(4)),
            disclose: false,
        },
        EventCandidate::Died {
            entity: None,
            disclose: false,
        },
    ] {
        assert_eq!(
            project_events(std::slice::from_ref(&hidden)).expect("projects"),
            Vec::<DeltaOpV2>::new()
        );
    }

    // An unrelated permitted state update stays independently deliverable
    // while the event is suppressed: the host assembles state ops itself.
    let suppressed = project_events(&[EventCandidate::ActionResolved {
        actor: Some(net(1)),
        target: None,
        ability: Some(Ulid::NIL),
        outcome: Some("success".to_owned()),
        damage: Some(2),
    }])
    .expect("projects");
    assert!(suppressed.is_empty());
    let state_frame = DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 9,
        event_seq: 3,
        ops: vec![DeltaOpV2::Legacy(DeltaOpV1::Health {
            entity: net(2),
            health: 8,
            max_health: 10,
            dead: false,
        })],
    };
    let state_bytes = encode_delta_v2(&state_frame).expect("state encodes");
    assert_eq!(
        decode_delta_v2(&state_bytes).expect("state decodes"),
        state_frame
    );

    // Two peers receive different filtered views of the same facts.
    let facts = vec![
        permitted_action(),
        EventCandidate::TurnStarted {
            actor: Some(net(1)),
            round: Some(0),
        },
    ];
    let peer_a = project_events(&facts).expect("peer A projects");
    assert_eq!(
        peer_a,
        vec![
            expected_action,
            DeltaOpV2::TurnStarted {
                actor: net(1),
                round: 0,
            },
        ]
    );
    // Peer B may see the turn fact but not the resolution target.
    let peer_b_facts = vec![
        EventCandidate::ActionResolved {
            actor: Some(net(1)),
            target: None,
            ability: Some(Ulid::NIL),
            outcome: Some("success".to_owned()),
            damage: Some(2),
        },
        EventCandidate::TurnStarted {
            actor: Some(net(1)),
            round: Some(0),
        },
    ];
    assert_eq!(
        project_events(&peer_b_facts).expect("peer B projects"),
        vec![DeltaOpV2::TurnStarted {
            actor: net(1),
            round: 0,
        }]
    );

    // A suppressed-only page projects to no ops: the host emits no frame
    // and consumes no delivery sequence.
    assert_eq!(
        project_events(&[
            EventCandidate::Died {
                entity: Some(net(5)),
                disclose: false,
            },
            EventCandidate::TurnStarted {
                actor: None,
                round: Some(2),
            },
        ])
        .expect("projects"),
        Vec::<DeltaOpV2>::new()
    );

    // Projection errors: too many inputs, oversized disclosed outcome,
    // non-vocabulary disclosed outcome. No partial output on error.
    let many: Vec<EventCandidate> = (0..MAX_DELTA_OPS + 1)
        .map(|_| EventCandidate::TurnStarted {
            actor: Some(net(1)),
            round: Some(0),
        })
        .collect();
    assert_eq!(project_events(&many), Err(ProjectionError::TooManyEvents));
    assert_eq!(
        project_events(&[EventCandidate::ActionResolved {
            actor: Some(net(1)),
            target: Some(net(2)),
            ability: Some(Ulid::NIL),
            outcome: Some("a".repeat(300)),
            damage: Some(0),
        }]),
        Err(ProjectionError::StringLimit)
    );
    assert_eq!(
        project_events(&[EventCandidate::ActionResolved {
            actor: Some(net(1)),
            target: Some(net(2)),
            ability: Some(Ulid::NIL),
            outcome: Some("bogus".to_owned()),
            damage: Some(0),
        }]),
        Err(ProjectionError::InvalidOutcome)
    );
    // A good event before a bad one still yields no partial output.
    assert!(project_events(&[
        permitted_action(),
        EventCandidate::ActionResolved {
            actor: Some(net(1)),
            target: Some(net(2)),
            ability: Some(Ulid::NIL),
            outcome: Some("bogus".to_owned()),
            damage: Some(0),
        },
    ])
    .is_err());
    // Error Display carries no payloads.
    assert_eq!(
        ProjectionError::TooManyEvents.to_string(),
        "TooManyEvents at projection/events"
    );
    assert_eq!(
        ProjectionError::InvalidOutcome.to_string(),
        "InvalidOutcome at projection/events"
    );
    assert_eq!(
        ProjectionError::StringLimit.to_string(),
        "StringLimit at projection/events"
    );
}

// ---------------------------------------------------------------------------
// History-driven cases: authored fixture through the real T020 API.
// ---------------------------------------------------------------------------

/// Minimal authored combat documents, parsed through the production data
/// shapes (mirrors the sim suite's standard fixture; fixture data only).
struct HistoryFixture {
    encounter: Encounter,
    ruleset: Ruleset,
    abilities: BTreeMap<Ulid, Ability>,
    tables: BTreeMap<Ulid, DataTable>,
    effects: BTreeMap<Ulid, Effect>,
    placements: BTreeMap<Ulid, (Placement, Ulid)>,
    creatures: BTreeMap<Ulid, Creature>,
    ability: Ulid,
}

fn fx_raw(value: i32) -> i32 {
    Fx16_16::from_int(value).to_raw()
}

fn uid(value: u128) -> String {
    Ulid::from_u128(value).to_string()
}

impl HistoryFixture {
    /// Two combatants, flat `{success: 2, failure: 0}` damage, two-band
    /// roll-under table; B (initiative 9) acts before A (initiative 10).
    fn standard() -> Self {
        let ability = Ulid::from_u128(103);
        let ruleset: Ruleset = serde_json::from_value(json!({
            "id": uid(102),
            "slug": "probe-ruleset",
            "name": "probe.ruleset",
            "package": "probe-pack",
            "version": "0.1.0",
            "stats": [
                {"name": "alpha", "kind": "int"},
                {"name": "beta", "kind": "int"},
                {"name": "gamma", "kind": "int"},
                {"name": "health", "kind": "int"}
            ],
            "health_stat": "health",
            "attributes": ["alpha", "beta", "gamma"],
            "pools": [
                {
                    "id": uid(105),
                    "max": 1,
                    "refresh": {"type": "on_turn_start"}
                }
            ],
            "abilities": [uid(103)]
        }))
        .expect("the standard ruleset is valid");
        let attack: Ability = serde_json::from_value(json!({
            "id": uid(103),
            "slug": "probe-attack",
            "name": "probe.attack",
            "dice": "2d6",
            "attribute": "alpha",
            "outcome_table": uid(104),
            "damage": [
                {"outcome": {"type": "success"}, "amount": 2},
                {"outcome": {"type": "failure"}, "amount": 0}
            ],
            "cost": 1,
            "extra_costs": [],
            "ends_turn": true,
            "defense": {"type": "actor_attribute"},
            "requires_target": true,
            "allow_self_target": false
        }))
        .expect("the standard ability is valid");
        let table: DataTable = serde_json::from_value(json!({
            "id": uid(104),
            "slug": "probe-table",
            "name": "probe.table",
            "bands": [
                {"min_margin": i64::MIN, "outcome": {"type": "success"}},
                {"min_margin": 1, "outcome": {"type": "failure"}}
            ],
            "natural_rules": []
        }))
        .expect("the standard table is valid");
        let encounter: Encounter = serde_json::from_value(json!({
            "id": uid(101),
            "slug": "probe-encounter",
            "name": "probe.encounter",
            "ruleset": uid(102),
            "participants": [
                {"placement": uid(111), "initiative": 10},
                {"placement": uid(112), "initiative": 9}
            ]
        }))
        .expect("the standard encounter is valid");
        let placement_a: Placement = serde_json::from_value(json!({
            "id": uid(111),
            "slug": "probe-a",
            "name": "probe.a",
            "prefab": uid(121),
            "transform": {
                "position": [0, 0, 0],
                "rotation": [0, 0, 0],
                "scale": [65536, 65536, 65536]
            },
            "overrides": {}
        }))
        .expect("placement A is valid");
        let placement_b: Placement = serde_json::from_value(json!({
            "id": uid(112),
            "slug": "probe-b",
            "name": "probe.b",
            "prefab": uid(122),
            "transform": {
                "position": [0, 0, 0],
                "rotation": [0, 0, 0],
                "scale": [65536, 65536, 65536]
            },
            "overrides": {}
        }))
        .expect("placement B is valid");
        let hero: Creature = serde_json::from_value(json!({
            "id": uid(121),
            "slug": "probe-creature-a",
            "name": "probe.creature-a",
            "stats": {
                "alpha": fx_raw(8),
                "beta": fx_raw(6),
                "gamma": fx_raw(7),
                "health": fx_raw(10)
            },
            "tags": [],
            "faction": null,
            "inventory": []
        }))
        .expect("creature A is valid");
        let rival: Creature = serde_json::from_value(json!({
            "id": uid(122),
            "slug": "probe-creature-b",
            "name": "probe.creature-b",
            "stats": {
                "alpha": fx_raw(6),
                "beta": fx_raw(7),
                "gamma": fx_raw(5),
                "health": fx_raw(6)
            },
            "tags": [],
            "faction": null,
            "inventory": []
        }))
        .expect("creature B is valid");
        let mut abilities = BTreeMap::new();
        abilities.insert(ability, attack);
        let mut tables = BTreeMap::new();
        tables.insert(Ulid::from_u128(104), table);
        let mut placements = BTreeMap::new();
        placements.insert(Ulid::from_u128(111), (placement_a, Ulid::from_u128(131)));
        placements.insert(Ulid::from_u128(112), (placement_b, Ulid::from_u128(131)));
        let mut creatures = BTreeMap::new();
        creatures.insert(Ulid::from_u128(121), hero);
        creatures.insert(Ulid::from_u128(122), rival);
        Self {
            encounter,
            ruleset,
            abilities,
            tables,
            effects: BTreeMap::new(),
            placements,
            creatures,
            ability,
        }
    }

    /// Lethal always-successful damage for the death/end case.
    fn lethal(mut self) -> Self {
        self.abilities
            .get_mut(&self.ability)
            .expect("ability")
            .damage = vec![
            DamageEntry {
                outcome: OutcomeWire::Success,
                amount: 99,
            },
            DamageEntry {
                outcome: OutcomeWire::Failure,
                amount: 0,
            },
        ];
        self.tables
            .get_mut(&Ulid::from_u128(104))
            .expect("table")
            .bands = vec![OutcomeBandWire {
            min_margin: i64::MIN,
            outcome: OutcomeWire::Success,
        }];
        self
    }
}

/// Reference maps assembled from one fixture for [`EncounterSpec`].
struct SpecBundle<'a> {
    abilities: BTreeMap<Ulid, &'a Ability>,
    tables: BTreeMap<Ulid, &'a DataTable>,
    placements: BTreeMap<Ulid, PlacementAndArea<'a>>,
    creatures: BTreeMap<Ulid, &'a Creature>,
    effects: BTreeMap<Ulid, &'a Effect>,
}

fn bundle_of(fixture: &HistoryFixture) -> SpecBundle<'_> {
    let mut abilities = BTreeMap::new();
    for (id, ability) in &fixture.abilities {
        abilities.insert(*id, ability);
    }
    let mut tables = BTreeMap::new();
    for (id, table) in &fixture.tables {
        tables.insert(*id, table);
    }
    let mut placements = BTreeMap::new();
    for (id, (placement, area)) in &fixture.placements {
        placements.insert(
            *id,
            PlacementAndArea {
                placement,
                area: *area,
            },
        );
    }
    let mut creatures = BTreeMap::new();
    for (id, creature) in &fixture.creatures {
        creatures.insert(*id, creature);
    }
    let mut effects = BTreeMap::new();
    for (id, effect) in &fixture.effects {
        effects.insert(*id, effect);
    }
    SpecBundle {
        abilities,
        tables,
        placements,
        creatures,
        effects,
    }
}

/// Starts the fixture encounter in a fresh wrapper, returning combatant
/// entities in authored participant order with their replica mapping.
fn start_history(
    fixture: &HistoryFixture,
    seed: u64,
) -> (HistoryWorld, Vec<EntityId>, BTreeMap<EntityId, NetId>) {
    let bundle = bundle_of(fixture);
    let spec = EncounterSpec {
        encounter: &fixture.encounter,
        ruleset: &fixture.ruleset,
        abilities: &bundle.abilities,
        outcome_tables: &bundle.tables,
        placements: &bundle.placements,
        creatures: &bundle.creatures,
        effects: &bundle.effects,
    };
    let mut history = HistoryWorld::new(seed);
    history.start_encounter(&spec).expect("the fixture starts");
    assert!(
        history.world().events().is_empty(),
        "the inner queue stays empty at wrapper boundaries"
    );
    let ids: Vec<EntityId> = history
        .world()
        .combatants()
        .iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(ids.len(), 2, "two combatants in authored order");
    let mut mapping = BTreeMap::new();
    mapping.insert(ids[0], net(1));
    mapping.insert(ids[1], net(2));
    (history, ids, mapping)
}

/// Explicit test-adapter mapping: one history variant to one fully
/// disclosed projection candidate. These fixture grants stand in for
/// T027 interest facts; they are not a production auth implementation.
fn candidate_for(
    payload: &HistoryEvent,
    mapping: &BTreeMap<EntityId, NetId>,
    ability: Ulid,
    encounter: Ulid,
) -> EventCandidate {
    let id = |entity: &EntityId| mapping.get(entity).copied();
    match payload {
        HistoryEvent::Spawned { entity } => EventCandidate::Spawned {
            entity: id(entity),
            disclose: true,
        },
        HistoryEvent::Despawned { entity } => EventCandidate::Despawned {
            entity: id(entity),
            disclose: true,
        },
        HistoryEvent::Died { entity } => EventCandidate::Died {
            entity: id(entity),
            disclose: true,
        },
        HistoryEvent::ActionResolved {
            actor,
            target,
            ability: event_ability,
            outcome,
            damage,
        } => {
            assert_eq!(*event_ability, ability, "the fixture ability resolves");
            EventCandidate::ActionResolved {
                actor: id(actor),
                target: id(target),
                ability: Some(*event_ability),
                outcome: Some(outcome.clone()),
                damage: Some(*damage),
            }
        }
        HistoryEvent::TurnStarted { actor, round } => EventCandidate::TurnStarted {
            actor: id(actor),
            round: Some(*round),
        },
        HistoryEvent::EncounterEnded {
            encounter: event_encounter,
            round,
        } => {
            assert_eq!(*event_encounter, encounter, "the fixture encounter ends");
            EventCandidate::EncounterEnded {
                encounter: Some(*event_encounter),
                round: Some(*round),
            }
        }
    }
}

#[test]
fn real_history_order_survives_projection() {
    // World A: two non-lethal resolutions across a round rollover.
    let fixture = HistoryFixture::standard();
    let (mut history, ids, mapping) = start_history(&fixture, 3);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        history.world().combat().expect("combat").active,
        Some(b),
        "B holds the first turn"
    );
    let first = history
        .perform_action(&CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        })
        .expect("B attacks")
        .expect("attack");
    assert_eq!(first.damage, 2, "non-lethal fixture damage");
    history
        .perform_action(&CombatAction::EndTurn { actor: a })
        .expect("A ends its turn");
    let second = history
        .perform_action(&CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        })
        .expect("B attacks again")
        .expect("attack");
    assert_eq!(second.damage, 2, "non-lethal fixture damage");

    // Hand-authored ordered oracle: spawns in authored order, the initial
    // turn, then per T020 resolution, drained deaths (none here), and the
    // branch-observed trailing turn/round transition — never inferred from
    // the producer.
    let journal = history.read_after(0, 256).expect("a full read");
    assert_eq!(journal.len(), 8, "eight journaled events");
    for (index, envelope) in journal.iter().enumerate() {
        assert_eq!(envelope.seq, index as u64 + 1, "contiguous from 1");
    }
    let candidates: Vec<EventCandidate> = journal
        .iter()
        .map(|envelope| candidate_for(&envelope.payload, &mapping, fixture.ability, encounter_id()))
        .collect();
    let projected = project_events(&candidates).expect("all disclosed");
    let expected = vec![
        DeltaOpV2::Legacy(DeltaOpV1::Spawned { entity: net(1) }),
        DeltaOpV2::Legacy(DeltaOpV1::Spawned { entity: net(2) }),
        DeltaOpV2::TurnStarted {
            actor: net(2),
            round: 0,
        },
        DeltaOpV2::ActionResolved {
            actor: net(2),
            target: net(1),
            ability: fixture.ability,
            outcome: "success".to_owned(),
            damage: 2,
        },
        DeltaOpV2::TurnStarted {
            actor: net(1),
            round: 0,
        },
        DeltaOpV2::TurnStarted {
            actor: net(2),
            round: 1,
        },
        DeltaOpV2::ActionResolved {
            actor: net(2),
            target: net(1),
            ability: fixture.ability,
            outcome: "success".to_owned(),
            damage: 2,
        },
        DeltaOpV2::TurnStarted {
            actor: net(1),
            round: 1,
        },
    ];
    assert_eq!(projected, expected, "ordered wire events match the oracle");
    // The projected wire frame round-trips byte-identically.
    let frame = DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 0,
        event_seq: 1,
        ops: projected.clone(),
    };
    let bytes = encode_delta_v2(&frame).expect("encodes");
    assert_eq!(decode_delta_v2(&bytes).expect("decodes"), frame);

    // The oracle is order- and content-sensitive: no multiset comparison.
    let mut omitted = expected.clone();
    omitted.remove(3);
    assert_ne!(projected, omitted, "dropping an event must fail the oracle");
    let mut duplicated = expected.clone();
    duplicated.insert(3, expected[3].clone());
    assert_ne!(
        projected, duplicated,
        "duplicating a same-tick event must fail the oracle"
    );
    let mut swapped = expected.clone();
    swapped.swap(4, 5);
    assert_ne!(
        projected, swapped,
        "swapping same-tick turns must fail the oracle"
    );

    // World B: lethal resolution, natural end, then despawn.
    let lethal = HistoryFixture::standard().lethal();
    let (mut history_b, ids_b, mapping_b) = start_history(&lethal, 3);
    let (a_b, b_b) = (ids_b[0], ids_b[1]);
    let killing = history_b
        .perform_action(&CombatAction::UseAbility {
            actor: b_b,
            ability: lethal.ability,
            target: a_b,
        })
        .expect("the lethal attack is accepted")
        .expect("attack");
    assert_eq!(killing.damage, 99);
    assert!(killing.target_died);
    assert!(
        history_b.despawn(a_b).expect("despawn succeeds"),
        "the dead entity is still live until despawned"
    );
    let journal_b = history_b.read_after(0, 256).expect("a full read");
    assert_eq!(journal_b.len(), 7, "seven journaled events");
    let candidates_b: Vec<EventCandidate> = journal_b
        .iter()
        .map(|envelope| {
            candidate_for(
                &envelope.payload,
                &mapping_b,
                lethal.ability,
                encounter_id(),
            )
        })
        .collect();
    let projected_b = project_events(&candidates_b).expect("all disclosed");
    let expected_b = vec![
        DeltaOpV2::Legacy(DeltaOpV1::Spawned { entity: net(1) }),
        DeltaOpV2::Legacy(DeltaOpV1::Spawned { entity: net(2) }),
        DeltaOpV2::TurnStarted {
            actor: net(2),
            round: 0,
        },
        DeltaOpV2::ActionResolved {
            actor: net(2),
            target: net(1),
            ability: lethal.ability,
            outcome: "success".to_owned(),
            damage: 99,
        },
        DeltaOpV2::Legacy(DeltaOpV1::Died { entity: net(1) }),
        DeltaOpV2::EncounterEnded {
            encounter: encounter_id(),
            round: 0,
        },
        DeltaOpV2::Legacy(DeltaOpV1::Despawned { entity: net(1) }),
    ];
    assert_eq!(
        projected_b, expected_b,
        "death/end/despawn order matches the oracle"
    );
    let mut swapped_b = expected_b.clone();
    swapped_b.swap(4, 5);
    assert_ne!(
        projected_b, swapped_b,
        "death and end are ordered, not a multiset"
    );
}

// ---------------------------------------------------------------------------
// redelivery_is_byte_identical
// ---------------------------------------------------------------------------

/// Single-command fixture driver with fixed grants, not production admission.
/// Retries enter the same path as new commands and hit the cache before sim.
struct RetryDriver {
    history: HistoryWorld,
    mapping: BTreeMap<EntityId, NetId>,
    ability: Ulid,
    cached: Option<(Vec<u8>, Vec<u8>)>,
    executions: usize,
}

impl RetryDriver {
    fn page(&self, after: u64, event_seq: u64) -> Vec<u8> {
        let candidates: Vec<_> = self
            .history
            .read_after(after, 256)
            .expect("read")
            .iter()
            .map(|event| candidate_for(&event.payload, &self.mapping, self.ability, encounter_id()))
            .collect();
        encode_delta_v2(&DeltaFrameV2 {
            lane: LANE_COMBAT,
            server_tick: 0,
            event_seq,
            ops: project_events(&candidates).expect("fixture grants"),
        })
        .expect("encodes")
    }

    fn admit(&mut self, bytes: &[u8]) -> Vec<u8> {
        let intent = decode_intent_v2(bytes).expect("valid fixture command");
        assert_eq!(intent.epoch, EPOCH);
        assert_eq!(intent.seq, 1);
        if let Some((request, response)) = &self.cached {
            assert_eq!(request, bytes, "retry must match the cached command");
            return response.clone();
        }
        let IntentBody::DeclareAction { ability, target } = intent.body else {
            panic!("fixture only admits the attack");
        };
        let resolve = |net_id| {
            *self
                .mapping
                .iter()
                .find(|(_, mapped)| **mapped == net_id)
                .expect("fixture mapping")
                .0
        };
        let action = CombatAction::UseAbility {
            actor: resolve(intent.actor),
            ability,
            target: resolve(target),
        };
        let after = self
            .history
            .read_after(0, 256)
            .expect("read")
            .last()
            .unwrap()
            .seq;
        self.executions += 1;
        self.history
            .perform_action(&action)
            .expect("attack succeeds");
        let response = self.page(after, 2);
        self.cached = Some((bytes.to_vec(), response.clone()));
        response
    }
}

#[derive(Default)]
struct EventReceiver {
    frames: BTreeMap<u64, Vec<u8>>,
    events: Vec<DeltaOpV2>,
}

impl EventReceiver {
    fn receive(&mut self, bytes: Vec<u8>) {
        let frame = decode_delta_v2(&bytes).expect("valid delivery");
        if let Some(previous) = self.frames.get(&frame.event_seq) {
            assert_eq!(previous, &bytes, "duplicate must be byte-identical");
            return;
        }
        assert_eq!(frame.event_seq, self.frames.len() as u64 + 1);
        self.events.extend(frame.ops);
        self.frames.insert(frame.event_seq, bytes);
    }
}

#[test]
fn redelivery_is_byte_identical() {
    // One authoritative mutation: the lethal resolution from a fresh
    // wrapper. The driver caches the encoded v2 frame; recovering loss and
    // duplicates must never re-execute the public sim action.
    let lethal = HistoryFixture::standard().lethal();
    let (history, _, mapping) = start_history(&lethal, 3);
    let mut driver = RetryDriver {
        history,
        mapping,
        ability: lethal.ability,
        cached: None,
        executions: 0,
    };
    let mut transport = InMemoryTransport::new(QueueCaps::v1_egress(), RateCaps::v1(), SimClock(0));
    let peer = transport.add_peer(EPOCH).expect("peer");
    let faults = FaultSchedule {
        loss_every: Some(2),
        dup_every: Some(1),
        reorder_depth: 0,
        seed: 3,
    };
    let mut receiver = EventReceiver::default();
    // Transmission 1 delivers the initial history (and a duplicate).
    transport.send_to(peer, driver.page(0, 1)).expect("send");
    transport.tick(&faults);
    let mut received = 0;
    while let Some(bytes) = transport.recv_from(peer) {
        receiver.receive(bytes);
        received += 1;
    }
    assert_eq!(received, 2);
    assert_eq!(receiver.events.len(), 3);

    let command = encode_intent_v2(&IntentFrame {
        epoch: EPOCH,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: net(2),
        body: IntentBody::DeclareAction {
            ability: lethal.ability,
            target: net(1),
        },
    })
    .expect("command");
    let before_action = history_hash(&driver.history);
    let cached = driver.admit(&command);
    let hash_after_action = history_hash(&driver.history);
    assert_ne!(before_action, hash_after_action);
    let journal_after_action = driver.history.read_after(0, 256).expect("read");
    assert_eq!(journal_after_action.len(), 6);

    // Transmission 2 is actually dropped by the fabric.
    transport.send_to(peer, cached.clone()).expect("send");
    transport.tick(&faults);
    assert!(transport.recv_from(peer).is_none());
    assert_eq!(receiver.events.len(), 3, "resolution has not arrived");

    // Retry enters admission again. Transmission 3 recovers the lost frame
    // with two copies; the receiver consumes its events exactly once.
    let retry = driver.admit(&command);
    assert_eq!(retry, cached);
    transport.send_to(peer, retry).expect("retry send");
    transport.tick(&faults);
    received = 0;
    while let Some(bytes) = transport.recv_from(peer) {
        assert_eq!(bytes, cached);
        receiver.receive(bytes);
        received += 1;
    }
    assert_eq!(received, 2);
    assert_eq!(receiver.frames.len(), 2);
    assert_eq!(
        receiver.events,
        vec![
            DeltaOpV2::Legacy(DeltaOpV1::Spawned { entity: net(1) }),
            DeltaOpV2::Legacy(DeltaOpV1::Spawned { entity: net(2) }),
            DeltaOpV2::TurnStarted {
                actor: net(2),
                round: 0
            },
            DeltaOpV2::ActionResolved {
                actor: net(2),
                target: net(1),
                ability: lethal.ability,
                outcome: "success".to_owned(),
                damage: 99,
            },
            DeltaOpV2::Legacy(DeltaOpV1::Died { entity: net(1) }),
            DeltaOpV2::EncounterEnded {
                encounter: encounter_id(),
                round: 0
            },
        ]
    );

    assert_eq!(driver.executions, 1, "retry must not re-execute gameplay");
    assert_eq!(history_hash(&driver.history), hash_after_action);
    assert_eq!(driver.history.pending_len(), 6);
    assert_eq!(
        driver.history.read_after(0, 256).expect("read"),
        journal_after_action
    );
}
