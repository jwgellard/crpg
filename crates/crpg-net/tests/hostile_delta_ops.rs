//! T042b2: the hostile-server delta suite on the event, fact and movement ops
//! (spec §16.2, T042b part 2). The four client-side lane-0 delta decoders fed
//! byte-exact hostile inputs on the v2 event op, the v3 fact ops and the v4
//! movement ops. No clock, no thread, no I/O, no unseeded RNG, no
//! `HashMap`/`HashSet`, no float.

use std::ops::Range;

use crpg_net::codec::{self, CodecError};
use crpg_net::codec_v2;
use crpg_net::codec_v3;
use crpg_net::codec_v4;
use crpg_net::protocol::{self, LANE_COMBAT};
use crpg_net::protocol_v2;
use crpg_net::protocol_v3;
use crpg_net::protocol_v4;

#[path = "support/hostile.rs"]
mod hostile;

use hostile::{row, Verdict, Want};

/// The row `Want`s, named once so each row stays one readable line.
const MAL: Want = Want::Refused(CodecError::Malformed);
const LIMIT: Want = Want::Refused(CodecError::LimitExceeded);
const UNKNOWN: Want = Want::Refused(CodecError::UnknownMessage);
const SAME: Want = Want::Same;
const ORIG: Want = Want::Orig;

// ---------------------------------------------------------------------------
// Wire table (WB1, reused from WA1).
// ---------------------------------------------------------------------------

/// One client-side lane-0 delta decoder with its own encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wire {
    V1,
    V2,
    V3,
    V4,
}

/// Table order; every "for each wire" loop uses it.
const WIRES: [Wire; 4] = [Wire::V1, Wire::V2, Wire::V3, Wire::V4];

impl Wire {
    /// 1, 2, 3, 4.
    fn first_byte(self) -> u8 {
        match self {
            Wire::V1 => protocol::PROTOCOL_VERSION,
            Wire::V2 => protocol_v2::PROTOCOL_VERSION,
            Wire::V3 => protocol_v3::PROTOCOL_VERSION,
            Wire::V4 => protocol_v4::PROTOCOL_VERSION,
        }
    }

    /// The lane byte at index 1: always `LANE_COMBAT` (0).
    fn lane(self) -> u8 {
        LANE_COMBAT
    }

    /// Decode with this wire's decoder; on `Ok(v)`, re-encode `v`. An encoder
    /// error on a decoded value panics with the wire and the bytes (WB10).
    fn verdict(self, bytes: &[u8]) -> Verdict {
        match self {
            Wire::V1 => {
                hostile::reencoded("V1", bytes, codec::decode_delta(bytes), codec::encode_delta)
            }
            Wire::V2 => hostile::reencoded(
                "V2",
                bytes,
                codec_v2::decode_delta(bytes),
                codec_v2::encode_delta,
            ),
            Wire::V3 => hostile::reencoded(
                "V3",
                bytes,
                codec_v3::decode_delta(bytes),
                codec_v3::encode_delta,
            ),
            Wire::V4 => hostile::reencoded(
                "V4",
                bytes,
                codec_v4::decode_delta(bytes),
                codec_v4::encode_delta,
            ),
        }
    }
}

/// One pinned valid frame.
struct Fixture {
    name: &'static str,
    wire: Wire,
    bytes: Vec<u8>,
    ulid_text: Vec<Range<usize>>,
}

fn fixture<'a>(all: &'a [Fixture], name: &str) -> &'a Fixture {
    all.iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no fixture {name}"))
}

/// Applies `r`'s edits to `f.bytes` (the `Row` stores owned byte vectors;
/// `hostile::edit` wants slices).
fn mutate(f: &Fixture, r: &hostile::Row) -> Vec<u8> {
    let edits: Vec<(usize, usize, &[u8])> = r
        .edits
        .iter()
        .map(|(at, remove, insert)| (*at, *remove, insert.as_slice()))
        .collect();
    hostile::edit(&f.bytes, &edits)
}

// ---------------------------------------------------------------------------
// Pinned literals and encoder-built fixtures (WB1).
// ---------------------------------------------------------------------------

/// `ABILITY` text (`014D2PF2DB0000SQQG28T5CY4T`), 26 bytes.
const ULID_TEXT: &[u8; 26] = b"014D2PF2DB0000SQQG28T5CY4T";
/// `ABILITY_B` text (`014D2PF2DB0000SQQG28T5CY4V`), 26 bytes.
const ULID_B_TEXT: &[u8; 26] = b"014D2PF2DB0000SQQG28T5CY4V";

/// Header `H(v)` = `[v, 0x00, 0x05, 0x01]` (version, lane 0, server_tick 5,
/// event_seq 1; the op count follows at index 4).
fn h(version: u8) -> Vec<u8> {
    vec![version, LANE_COMBAT, 0x05, 0x01]
}

/// `Xv` literal: `H(v) ++ [0x01, 0x08, 0x07, 0x09, 0x1A] ++ ABILITY ++
/// [0x07] ++ b"success" ++ [0x06]` (44 bytes).
fn x_literal(version: u8) -> Vec<u8> {
    let head = h(version);
    hostile::cat(&[
        &head[..],
        &[0x01, 0x08, 0x07, 0x09, 0x1A][..],
        &ULID_TEXT[..],
        &[0x07][..],
        b"success",
        &[0x06][..],
    ])
}

/// The `ActionResolved` frame, built by `wire`'s encoder.
fn encode_x(wire: Wire) -> Vec<u8> {
    match wire {
        Wire::V1 => panic!("no X fixture on V1"),
        Wire::V2 => codec_v2::encode_delta(&protocol_v2::DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 5,
            event_seq: 1,
            ops: vec![protocol_v2::DeltaOp::ActionResolved {
                actor: hostile::net(7),
                target: hostile::net(9),
                ability: hostile::ability(),
                outcome: "success".to_owned(),
                damage: 6,
            }],
        })
        .unwrap(),
        Wire::V3 => codec_v3::encode_delta(&protocol_v3::DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 5,
            event_seq: 1,
            ops: vec![protocol_v3::DeltaOp::ActionResolved {
                actor: hostile::net(7),
                target: hostile::net(9),
                ability: hostile::ability(),
                outcome: "success".to_owned(),
                damage: 6,
            }],
        })
        .unwrap(),
        Wire::V4 => codec_v4::encode_delta(&protocol_v4::DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 5,
            event_seq: 1,
            ops: vec![protocol_v4::DeltaOp::V3(
                protocol_v3::DeltaOp::ActionResolved {
                    actor: hostile::net(7),
                    target: hostile::net(9),
                    ability: hostile::ability(),
                    outcome: "success".to_owned(),
                    damage: 6,
                },
            )],
        })
        .unwrap(),
    }
}

/// `E3` literal: `H(3) ++ [0x01, 0x0D, 0x09]` then the five fact families
/// (155 bytes).
fn e_literal() -> Vec<u8> {
    let head = h(3);
    hostile::cat(&[
        &head[..],
        &[0x01, 0x0D, 0x09][..],
        &[0x01, 0x01, 0x1A][..],
        &ULID_TEXT[..],
        &[0x03, 0x05][..],
        &[0x01, 0x02, 0x1A][..],
        &ULID_TEXT[..],
        &[0x04, 0x1A][..],
        &ULID_B_TEXT[..],
        &[0x09][..],
        &[0x01, 0x01, 0x1A][..],
        &ULID_TEXT[..],
        &[0x02, 0x01][..],
        &[0x01, 0x02, 0x03][..],
        b"str",
        &[0x00, 0x03, 0x03][..],
        b"agi",
        &[0x01, 0x80, 0x80, 0x08][..],
        &[0x01, 0x02, 0x04][..],
        b"ally",
        &[0x04][..],
        b"boss",
    ])
}

/// The `EntityFacts` frame, built by v3's encoder.
fn encode_e() -> Vec<u8> {
    codec_v3::encode_delta(&protocol_v3::DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 5,
        event_seq: 1,
        ops: vec![protocol_v3::DeltaOp::EntityFacts {
            entity: hostile::net(9),
            facts: protocol_v3::FactSet {
                pools: Some(vec![protocol_v3::PoolFact {
                    pool: hostile::ability(),
                    current: 3,
                    max: 5,
                }]),
                effects: Some(vec![
                    protocol_v3::EffectFact {
                        effect: hostile::ability(),
                        expires_round: 4,
                    },
                    protocol_v3::EffectFact {
                        effect: hostile::ability_b(),
                        expires_round: 9,
                    },
                ]),
                order: Some(Some(protocol_v3::OrderFact {
                    ability: hostile::ability(),
                    target: hostile::net(2),
                    repeat: true,
                })),
                stats: Some(vec![
                    protocol_v3::StatFact {
                        name: "str".to_owned(),
                        value: protocol_v3::StatValue::Int(-2),
                    },
                    protocol_v3::StatFact {
                        name: "agi".to_owned(),
                        value: protocol_v3::StatValue::Fixed(65536),
                    },
                ]),
                tags: Some(vec!["ally".to_owned(), "boss".to_owned()]),
            },
        }],
    })
    .unwrap()
}

/// `S3` literal: `H(3) ++ [0x04]` then Encounter, Timeline, Clock and
/// HostFacts (35 bytes).
fn s_literal() -> Vec<u8> {
    let head = h(3);
    hostile::cat(&[
        &head[..],
        &[0x04][..],
        &[0x0B, 0x01, 0x01, 0x01, 0x00, 0x01, 0x01, 0x14, 0x03][..],
        &[0x0C, 0x01, 0x02, 0x01, 0x09, 0x04, 0x07][..],
        &[0x13, 0x01, 0x01, 0x01, 0x01][..],
        &[0x14, 0x01, 0x00, 0x01, 0x05, 0x01, 0x01, 0xDC, 0x0B][..],
    ])
}

/// The Encounter + Timeline + Clock + HostFacts frame, built by v3's
/// encoder.
fn encode_s() -> Vec<u8> {
    codec_v3::encode_delta(&protocol_v3::DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 5,
        event_seq: 1,
        ops: vec![
            protocol_v3::DeltaOp::Encounter(protocol_v3::EncounterFacts {
                mode: Some(protocol_v3::EncounterMode::RealTime),
                pending_mode: Some(None),
                clock: Some(Some(protocol_v3::ClockFact {
                    round_ticks: 20,
                    elapsed: 3,
                })),
            }),
            protocol_v3::DeltaOp::Timeline {
                entries: Some(vec![
                    protocol_v3::TimelineEntry {
                        key: -1,
                        entity: hostile::net(9),
                    },
                    protocol_v3::TimelineEntry {
                        key: 2,
                        entity: hostile::net(7),
                    },
                ]),
            },
            protocol_v3::DeltaOp::Clock {
                paused: Some(Some(protocol_v3::PauseReason::Auto(
                    protocol_v3::AutoPauseTrigger::RoundStarted,
                ))),
            },
            protocol_v3::DeltaOp::HostFacts(protocol_v3::HostFacts {
                rights: protocol_v3::PauseRights {
                    pause: true,
                    unpause: false,
                },
                auto_pause: Some(protocol_v3::AutoPauseToggles::from_bits(5).unwrap()),
                turn_timer_ms: Some(Some(1500)),
            }),
        ],
    })
    .unwrap()
}

/// `P4` literal: `H(4) ++ [0x02]` then Moved and Position (21 bytes).
fn p_literal() -> Vec<u8> {
    let head = h(4);
    hostile::cat(&[
        &head[..],
        &[0x02][..],
        &[0x15, 0x09, 0x02, 0x02, 0x04, 0x06, 0x08, 0x04, 0x0C][..],
        &[0x16, 0x07, 0x01, 0x02, 0x04, 0x06, 0x01][..],
    ])
}

/// The Moved + Position frame, built by v4's encoder.
fn encode_p() -> Vec<u8> {
    codec_v4::encode_delta(&protocol_v4::DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 5,
        event_seq: 1,
        ops: vec![
            protocol_v4::DeltaOp::Moved {
                entity: hostile::net(9),
                points: vec![
                    protocol_v4::Point::new([1, 2, 3]).unwrap(),
                    protocol_v4::Point::new([4, 2, 6]).unwrap(),
                ],
            },
            protocol_v4::DeltaOp::Position {
                entity: hostile::net(7),
                position: Some(protocol_v4::PositionFact {
                    point: protocol_v4::Point::new([1, 2, 3]).unwrap(),
                    moving: true,
                }),
            },
        ],
    })
    .unwrap()
}

/// The 6 fixtures of WB1 in table order, each built by its encoder.
fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture {
            name: "X2",
            wire: Wire::V2,
            bytes: encode_x(Wire::V2),
            ulid_text: vec![Range { start: 9, end: 35 }],
        },
        Fixture {
            name: "X3",
            wire: Wire::V3,
            bytes: encode_x(Wire::V3),
            ulid_text: vec![Range { start: 9, end: 35 }],
        },
        Fixture {
            name: "X4",
            wire: Wire::V4,
            bytes: encode_x(Wire::V4),
            ulid_text: vec![Range { start: 9, end: 35 }],
        },
        Fixture {
            name: "E3",
            wire: Wire::V3,
            bytes: encode_e(),
            ulid_text: vec![10..36, 41..67, 69..95, 99..125],
        },
        Fixture {
            name: "S3",
            wire: Wire::V3,
            bytes: encode_s(),
            ulid_text: vec![],
        },
        Fixture {
            name: "P4",
            wire: Wire::V4,
            bytes: encode_p(),
            ulid_text: vec![],
        },
    ]
}

/// The 6 fixtures' hand-written literals, in the same order.
fn pinned() -> Vec<Fixture> {
    vec![
        Fixture {
            name: "X2",
            wire: Wire::V2,
            bytes: x_literal(2),
            ulid_text: vec![Range { start: 9, end: 35 }],
        },
        Fixture {
            name: "X3",
            wire: Wire::V3,
            bytes: x_literal(3),
            ulid_text: vec![Range { start: 9, end: 35 }],
        },
        Fixture {
            name: "X4",
            wire: Wire::V4,
            bytes: x_literal(4),
            ulid_text: vec![Range { start: 9, end: 35 }],
        },
        Fixture {
            name: "E3",
            wire: Wire::V3,
            bytes: e_literal(),
            ulid_text: vec![10..36, 41..67, 69..95, 99..125],
        },
        Fixture {
            name: "S3",
            wire: Wire::V3,
            bytes: s_literal(),
            ulid_text: vec![],
        },
        Fixture {
            name: "P4",
            wire: Wire::V4,
            bytes: p_literal(),
            ulid_text: vec![],
        },
    ]
}

/// Pinned varint byte sequences (the "Row notation" of the contract).
const U32MAX: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
const U32OVF: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF, 0x10];

/// `x-out-custom255`: `0A "custom:255"`.
const CUSTOM_255: &[u8] = &[
    0x0A, 0x63, 0x75, 0x73, 0x74, 0x6F, 0x6D, 0x3A, 0x32, 0x35, 0x35,
];
/// `x-out-custom0`: `08 "custom:0"`.
const CUSTOM_0: &[u8] = &[0x08, 0x63, 0x75, 0x73, 0x74, 0x6F, 0x6D, 0x3A, 0x30];
/// `x-out-critical`: `10 "critical_success"`.
const CRITICAL_SUCCESS: &[u8] = &[
    0x10, 0x63, 0x72, 0x69, 0x74, 0x69, 0x63, 0x61, 0x6C, 0x5F, 0x73, 0x75, 0x63, 0x63, 0x65, 0x73,
    0x73,
];
/// `x-out-custom256`: `0A "custom:256"`.
const CUSTOM_256: &[u8] = &[
    0x0A, 0x63, 0x75, 0x73, 0x74, 0x6F, 0x6D, 0x3A, 0x32, 0x35, 0x36,
];
/// `x-out-custom07`: `09 "custom:07"`.
const CUSTOM_07: &[u8] = &[0x09, 0x63, 0x75, 0x73, 0x74, 0x6F, 0x6D, 0x3A, 0x30, 0x37];
/// `x-out-custom-empty`: `07 "custom:"`.
const CUSTOM_EMPTY: &[u8] = &[0x07, 0x63, 0x75, 0x73, 0x74, 0x6F, 0x6D, 0x3A];
/// `x-out-Success`: `07 "Success"`.
const SUCCESS_TITLE: &[u8] = &[0x07, 0x53, 0x75, 0x63, 0x63, 0x65, 0x73, 0x73];

/// `e-pool2-distinct`'s inserted second pool: `ABILITY_B`, 3/5.
const POOL_B_ADD: &[u8] = &[
    0x1A, 0x30, 0x31, 0x34, 0x44, 0x32, 0x50, 0x46, 0x32, 0x44, 0x42, 0x30, 0x30, 0x30, 0x30, 0x53,
    0x51, 0x51, 0x47, 0x32, 0x38, 0x54, 0x35, 0x43, 0x59, 0x34, 0x56, 0x03, 0x05,
];
/// `e-pool2-dup`'s inserted second pool: `ABILITY`, 3/5.
const POOL_DUP_ADD: &[u8] = &[
    0x1A, 0x30, 0x31, 0x34, 0x44, 0x32, 0x50, 0x46, 0x32, 0x44, 0x42, 0x30, 0x30, 0x30, 0x30, 0x53,
    0x51, 0x51, 0x47, 0x32, 0x38, 0x54, 0x35, 0x43, 0x59, 0x34, 0x54, 0x03, 0x05,
];
/// `e-stat-name-33`: a 33-byte name (`21` then 33 `61`s).
const STAT_NAME_33: &[u8] = &[
    0x21, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61,
    0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61,
    0x61, 0x61,
];

// ---------------------------------------------------------------------------
// HB1..HB11
// ---------------------------------------------------------------------------

#[test]
fn fixtures_are_pinned() {
    let fx = fixtures();
    let literal = pinned();
    assert_eq!(fx.len(), 6, "fixture count");
    assert_eq!(literal.len(), 6, "literal count");

    for (f, p) in fx.iter().zip(literal.iter()) {
        assert_eq!(f.name, p.name);
        assert_eq!(f.wire, p.wire, "fixture {} wire", f.name);
        assert_eq!(
            f.bytes.len(),
            p.bytes.len(),
            "fixture {} length {} != literal length {}",
            f.name,
            f.bytes.len(),
            p.bytes.len()
        );
        assert_eq!(
            f.bytes, p.bytes,
            "fixture {} encoder bytes {:02X?} != literal {:02X?}",
            f.name, f.bytes, p.bytes
        );
        assert_eq!(
            f.wire.verdict(&f.bytes),
            Verdict::Accepted(f.bytes.clone()),
            "fixture {} is not canonical: {:02X?}",
            f.name,
            f.bytes
        );
    }

    assert_eq!(Wire::V1.first_byte(), protocol::PROTOCOL_VERSION);
    assert_eq!(Wire::V2.first_byte(), protocol_v2::PROTOCOL_VERSION);
    assert_eq!(Wire::V3.first_byte(), protocol_v3::PROTOCOL_VERSION);
    assert_eq!(Wire::V4.first_byte(), protocol_v4::PROTOCOL_VERSION);
    for wire in WIRES {
        assert_eq!(wire.lane(), LANE_COMBAT, "{:?} lane", wire);
    }

    let x2 = fixture(&fx, "X2");
    assert_ne!(
        hostile::with_byte(&x2.bytes, 5, 0x07),
        x2.bytes,
        "control: X2 with op tag 7 is not X2's encoding"
    );
    assert_eq!(x2.bytes.len(), 44, "X2 length");
    assert_eq!(fixture(&fx, "E3").bytes.len(), 155, "E3 length");
    assert_eq!(fixture(&fx, "S3").bytes.len(), 35, "S3 length");
    assert_eq!(fixture(&fx, "P4").bytes.len(), 21, "P4 length");
}

#[test]
fn every_truncation_is_malformed() {
    let fx = fixtures();
    let mut count = 0usize;
    for f in &fx {
        for n in 0..f.bytes.len() {
            assert_eq!(
                f.wire.verdict(&f.bytes[..n]),
                Verdict::Refused(CodecError::Malformed),
                "{:?} fixture {} prefix of {n} bytes: {:02X?}",
                f.wire,
                f.name,
                &f.bytes[..n]
            );
            count += 1;
        }
        assert_eq!(
            f.wire.verdict(&f.bytes),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} full bytes",
            f.wire,
            f.name
        );
    }
    assert_eq!(count, 343, "WB2 input count");
}

#[test]
fn trailing_bytes_are_malformed() {
    let fx = fixtures();
    let mut count = 0usize;
    for f in &fx {
        for extra in [0x00u8, 0x01, 0xFF] {
            let mut input = f.bytes.clone();
            input.push(extra);
            assert_eq!(
                f.wire.verdict(&input),
                Verdict::Refused(CodecError::Malformed),
                "{:?} fixture {} plus {:02X?}: {:02X?}",
                f.wire,
                f.name,
                extra,
                input
            );
            count += 1;
        }
        assert_eq!(
            f.wire.verdict(&f.bytes),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} without the extra byte",
            f.wire,
            f.name
        );
    }
    assert_eq!(count, 18, "WB3 input count");
}

#[test]
fn foreign_first_bytes_are_unsupported() {
    let fx = fixtures();
    let mut cross = 0usize;
    for f in &fx {
        for wire in WIRES {
            if wire == f.wire {
                continue;
            }
            assert_eq!(
                wire.verdict(&f.bytes),
                Verdict::Refused(CodecError::UnsupportedVersion),
                "{:?} reading fixture {} of {:?}: {:02X?}",
                wire,
                f.name,
                f.wire,
                f.bytes
            );
            cross += 1;
        }
        for b in 0..=255u8 {
            if b == f.bytes[0] {
                continue;
            }
            let input = hostile::with_byte(&f.bytes, 0, b);
            assert_eq!(
                f.wire.verdict(&input),
                Verdict::Refused(CodecError::UnsupportedVersion),
                "{:?} fixture {} with version byte {b:02X}: {:02X?}",
                f.wire,
                f.name,
                input
            );
        }
        assert_eq!(
            f.wire.verdict(&f.bytes),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} on its own wire",
            f.wire,
            f.name
        );
    }
    assert_eq!(cross, 18, "WB4 cross-wire count");
}

#[test]
fn foreign_lanes_are_wrong_direction() {
    let fx = fixtures();
    for f in &fx {
        for v in 0..=255u8 {
            if v == LANE_COMBAT {
                continue;
            }
            let input = hostile::with_byte(&f.bytes, 1, v);
            assert_eq!(
                f.wire.verdict(&input),
                Verdict::Refused(CodecError::WrongDirection),
                "{:?} fixture {} with lane byte {v:02X} at 1: {:02X?}",
                f.wire,
                f.name,
                input
            );
        }
        assert_eq!(
            f.wire
                .verdict(&hostile::with_byte(&f.bytes, 1, LANE_COMBAT)),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} with its own lane byte",
            f.wire,
            f.name
        );
    }
}

#[test]
fn action_resolved_fields() {
    let fx = fixtures();
    let x_names = ["X2", "X3", "X4"];
    let rows = [
        row("x-actor0", &x_names, &[(6, 1, &[0x00])], MAL),
        row("x-target0", &x_names, &[(7, 1, &[0x00])], MAL),
        row("x-ulid-len-257", &x_names, &[(8, 1, &[0x81, 0x02])], LIMIT),
        row("x-ulid-len-256", &x_names, &[(8, 1, &[0x80, 0x02])], MAL),
        row("x-ulid-len-25", &x_names, &[(8, 1, &[0x19])], MAL),
        row("x-ulid-len-27", &x_names, &[(8, 1, &[0x1B])], MAL),
        row("x-ulid-first-8", &x_names, &[(9, 1, &[0x38])], MAL),
        row("x-ulid-first-7", &x_names, &[(9, 1, &[0x37])], SAME),
        row("x-ulid-U", &x_names, &[(12, 1, &[0x55])], MAL),
        row("x-ulid-alias-o", &x_names, &[(9, 1, &[0x6F])], ORIG),
        row("x-ulid-alias-L", &x_names, &[(10, 1, &[0x4C])], ORIG),
        row("x-ulid-alias-d", &x_names, &[(12, 1, &[0x64])], ORIG),
        row("x-out-len-257", &x_names, &[(35, 1, &[0x81, 0x02])], LIMIT),
        row("x-out-len-0", &x_names, &[(35, 8, &[0x00])], MAL),
        row("x-out-len-6", &x_names, &[(35, 1, &[0x06])], MAL),
        row("x-out-custom255", &x_names, &[(35, 8, CUSTOM_255)], SAME),
        row("x-out-custom0", &x_names, &[(35, 8, CUSTOM_0)], SAME),
        row(
            "x-out-critical",
            &x_names,
            &[(35, 8, CRITICAL_SUCCESS)],
            SAME,
        ),
        row("x-out-custom256", &x_names, &[(35, 8, CUSTOM_256)], MAL),
        row("x-out-custom07", &x_names, &[(35, 8, CUSTOM_07)], MAL),
        row(
            "x-out-custom-empty",
            &x_names,
            &[(35, 8, CUSTOM_EMPTY)],
            MAL,
        ),
        row("x-out-Success", &x_names, &[(35, 8, SUCCESS_TITLE)], MAL),
        row("x-out-badutf8", &x_names, &[(35, 8, &[0x01, 0xFF])], MAL),
        row("x-damage-u32max", &x_names, &[(43, 1, U32MAX)], SAME),
        row("x-damage-u32ovf", &x_names, &[(43, 1, U32OVF)], MAL),
    ];

    for r in &rows {
        for name in &r.fixtures {
            let f = fixture(&fx, name);
            assert_eq!(
                f.wire.verdict(&f.bytes),
                Verdict::Accepted(f.bytes.clone()),
                "control: {:?} fixture {} is not canonical",
                f.wire,
                f.name
            );
            let mutated = mutate(f, r);
            let got = f.wire.verdict(&mutated);
            hostile::assert_row(name, r, &f.bytes, &mutated, &got);
        }
    }
}

#[test]
fn entity_facts_fields() {
    let fx = fixtures();
    let e3 = ["E3"];
    let rows = [
        row("e-pools-count-17", &e3, &[(8, 1, &[0x11])], LIMIT),
        row("e-pools-count-0", &e3, &[(8, 1, &[0x00])], MAL),
        row("e-pool-cur-gt-max", &e3, &[(36, 1, &[0x06])], MAL),
        row("e-pool-cur-eq-max", &e3, &[(36, 1, &[0x05])], SAME),
        row("e-pool-max-lt-cur", &e3, &[(37, 1, &[0x02])], MAL),
        row(
            "e-pool2-distinct",
            &e3,
            &[(8, 1, &[0x02]), (38, 0, POOL_B_ADD)],
            SAME,
        ),
        row(
            "e-pool2-dup",
            &e3,
            &[(8, 1, &[0x02]), (38, 0, POOL_DUP_ADD)],
            MAL,
        ),
        row("e-eff-count-17", &e3, &[(39, 1, &[0x11])], LIMIT),
        row("e-eff-equal", &e3, &[(94, 1, &[0x54])], MAL),
        row("e-eff-descending", &e3, &[(94, 1, &[0x53])], MAL),
        row("e-eff-ascending", &e3, &[(94, 1, &[0x57])], SAME),
        row("e-order-target0", &e3, &[(125, 1, &[0x00])], MAL),
        row("e-order-repeat2", &e3, &[(126, 1, &[0x02])], MAL),
        row("e-stats-count-33", &e3, &[(128, 1, &[0x21])], LIMIT),
        row("e-stat-name-0", &e3, &[(129, 4, &[0x00])], MAL),
        row("e-stat-name-33", &e3, &[(129, 4, STAT_NAME_33)], LIMIT),
        row("e-stat-kind2", &e3, &[(133, 1, &[0x02])], UNKNOWN),
        row("e-stat-dup", &e3, &[(136, 3, &[0x73, 0x74, 0x72])], MAL),
        row("e-tags-count-9", &e3, &[(144, 1, &[0x09])], LIMIT),
        row(
            "e-tag-descending",
            &e3,
            &[(151, 4, &[0x61, 0x62, 0x62, 0x61])],
            MAL,
        ),
        row(
            "e-tag-equal",
            &e3,
            &[(151, 4, &[0x61, 0x6C, 0x6C, 0x79])],
            MAL,
        ),
        row("e-tag-empty", &e3, &[(150, 5, &[0x00])], MAL),
    ];

    for r in &rows {
        for name in &r.fixtures {
            let f = fixture(&fx, name);
            assert_eq!(
                f.wire.verdict(&f.bytes),
                Verdict::Accepted(f.bytes.clone()),
                "control: {:?} fixture {} is not canonical",
                f.wire,
                f.name
            );
            let mutated = mutate(f, r);
            let got = f.wire.verdict(&mutated);
            hostile::assert_row(name, r, &f.bytes, &mutated, &got);
        }
    }
}

#[test]
fn encounter_timeline_clock_host_fields() {
    let fx = fixtures();
    let s3 = ["S3"];
    let rows = [
        row("s-mode2", &s3, &[(7, 1, &[0x02])], UNKNOWN),
        row("s-mode0", &s3, &[(7, 1, &[0x00])], SAME),
        row("s-pending-bool2", &s3, &[(9, 1, &[0x02])], MAL),
        row("s-roundticks0", &s3, &[(12, 1, &[0x00])], MAL),
        row(
            "s-elapsed-max",
            &s3,
            &[(13, 1, &[0xFF, 0xFF, 0xFF, 0xFF, 0x07])],
            SAME,
        ),
        row(
            "s-elapsed-over",
            &s3,
            &[(13, 1, &[0x80, 0x80, 0x80, 0x80, 0x08])],
            MAL,
        ),
        row("s-tl-count-1025", &s3, &[(16, 1, &[0x81, 0x08])], LIMIT),
        row("s-tl-key-desc", &s3, &[(17, 1, &[0x0A])], MAL),
        row("s-tl-key-equal", &s3, &[(19, 1, &[0x01])], SAME),
        row("s-tl-entity-dup", &s3, &[(20, 1, &[0x09])], MAL),
        row("s-tl-entity0", &s3, &[(20, 1, &[0x00])], MAL),
        row("s-clock-reason0", &s3, &[(24, 1, &[0x00])], MAL),
        row("s-clock-reason2", &s3, &[(24, 1, &[0x02])], MAL),
        row("s-clock-reason4", &s3, &[(24, 1, &[0x04])], UNKNOWN),
        row("s-clock-trigger4", &s3, &[(25, 1, &[0x04])], SAME),
        row("s-clock-trigger5", &s3, &[(25, 1, &[0x05])], UNKNOWN),
        row("s-host-bits1f", &s3, &[(30, 1, &[0x1F])], SAME),
        row("s-host-bits20", &s3, &[(30, 1, &[0x20])], MAL),
        row("s-host-running2", &s3, &[(32, 1, &[0x02])], MAL),
        row(
            "s-host-ms-u32ovf",
            &s3,
            &[(33, 2, &[0xFF, 0xFF, 0xFF, 0xFF, 0x10])],
            MAL,
        ),
    ];

    for r in &rows {
        for name in &r.fixtures {
            let f = fixture(&fx, name);
            assert_eq!(
                f.wire.verdict(&f.bytes),
                Verdict::Accepted(f.bytes.clone()),
                "control: {:?} fixture {} is not canonical",
                f.wire,
                f.name
            );
            let mutated = mutate(f, r);
            let got = f.wire.verdict(&mutated);
            hostile::assert_row(name, r, &f.bytes, &mutated, &got);
        }
    }
}

#[test]
fn moved_and_position_fields() {
    let fx = fixtures();
    let p4 = ["P4"];
    let rows = [
        row("p-moved-count-0", &p4, &[(7, 1, &[0x00])], MAL),
        row("p-moved-count-1", &p4, &[(7, 1, &[0x01])], MAL),
        row("p-moved-count-64", &p4, &[(7, 1, &[0x40])], MAL),
        row("p-moved-count-65", &p4, &[(7, 1, &[0x41])], LIMIT),
        row(
            "p-moved-zero-seg",
            &p4,
            &[(11, 3, &[0x02, 0x0A, 0x06])],
            MAL,
        ),
        row(
            "p-moved-z-differs",
            &p4,
            &[(11, 3, &[0x02, 0x04, 0x08])],
            SAME,
        ),
        row(
            "p-x-limit",
            &p4,
            &[(8, 1, &[0x80, 0x80, 0x80, 0x80, 0x04])],
            SAME,
        ),
        row(
            "p-x-over",
            &p4,
            &[(8, 1, &[0x82, 0x80, 0x80, 0x80, 0x04])],
            MAL,
        ),
        row(
            "p-x-neg-limit",
            &p4,
            &[(8, 1, &[0xFF, 0xFF, 0xFF, 0xFF, 0x03])],
            SAME,
        ),
        row(
            "p-x-neg-over",
            &p4,
            &[(8, 1, &[0x81, 0x80, 0x80, 0x80, 0x04])],
            MAL,
        ),
        row(
            "p-z-over",
            &p4,
            &[(10, 1, &[0x82, 0x80, 0x80, 0x80, 0x04])],
            MAL,
        ),
        row(
            "p-coord-ovf",
            &p4,
            &[(8, 1, &[0xFF, 0xFF, 0xFF, 0xFF, 0x10])],
            MAL,
        ),
        row("p-pos-disclosed2", &p4, &[(16, 1, &[0x02])], MAL),
        row("p-pos-disclosed0", &p4, &[(16, 1, &[0x00])], MAL),
        row("p-pos-moving2", &p4, &[(20, 1, &[0x02])], MAL),
        row("p-pos-entity0", &p4, &[(15, 1, &[0x00])], MAL),
        row("p-moved-entity0", &p4, &[(6, 1, &[0x00])], MAL),
    ];

    for r in &rows {
        for name in &r.fixtures {
            let f = fixture(&fx, name);
            assert_eq!(
                f.wire.verdict(&f.bytes),
                Verdict::Accepted(f.bytes.clone()),
                "control: {:?} fixture {} is not canonical",
                f.wire,
                f.name
            );
            let mutated = mutate(f, r);
            let got = f.wire.verdict(&mutated);
            hostile::assert_row(name, r, &f.bytes, &mutated, &got);
        }
    }
}

#[test]
fn single_byte_corruption_is_refused_or_exact() {
    let fx = fixtures();
    for f in &fx {
        let counts =
            hostile::check_corruption(f.name, &|b| f.wire.verdict(b), &f.bytes, &f.ulid_text);
        assert!(counts.refused >= 1, "{} has no refused corruption", f.name);
        assert!(counts.exact >= 1, "{} has no accepted corruption", f.name);
        if f.name != "S3" && f.name != "P4" {
            assert!(counts.alias >= 1, "{} has no ULID alias corruption", f.name);
        }
    }
    let total: usize = fx.iter().map(|f| f.bytes.len()).sum();
    assert_eq!(total * 255, 87_465, "WB10 input count");
}

#[test]
fn seeded_noise_is_stable() {
    let mut rng = hostile::XorShift64(hostile::SEED);
    assert_eq!(rng.next(), 0x4B10_DA10_A18C_50C2);
    assert_eq!(rng.next(), 0x8788_B83B_1805_38E3);
    assert_eq!(rng.next(), 0x18B5_5443_1CC3_6A52);

    let fx = fixtures();
    let mut accepted = 0usize;
    let mut inputs = 0usize;
    for f in &fx {
        let span = f.bytes.len() - 5;
        for _ in 0..512 {
            inputs += 1;
            let i = 5 + (rng.next() % span as u64) as usize;
            let v = rng.next() as u8;
            let mutated = hostile::with_byte(&f.bytes, i, v);
            assert!(
                hostile::stable(&|b| f.wire.verdict(b), &mutated),
                "(WB11) {:?} fixture {} byte {i} := {v:02X} is not stable: {:02X?}",
                f.wire,
                f.name,
                mutated
            );
            if matches!(f.wire.verdict(&mutated), Verdict::Accepted(_)) {
                accepted += 1;
            }
        }
    }
    assert_eq!(inputs, 3_072, "WB11 input count");
    assert!(
        accepted >= 300,
        "only {accepted} accepted verdicts in the WB11 noise run"
    );
}
