//! T042b1: the hostile-server delta suite (spec §16.2, T042b part 1).
//! The four client-side lane-0 delta decoders fed byte-exact hostile inputs
//! on the common ops (EntityEnter, Health, Receipt). No clock, no thread, no
//! I/O, no unseeded RNG, no `HashMap`/`HashSet`, no float.

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

// ---------------------------------------------------------------------------
// Wire table (WA1).
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
    /// error on a decoded value panics with the wire and the bytes (WA12).
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

/// Applies `row`'s edits to `f.bytes` (the `Row` stores owned byte vectors;
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
// Pinned literals and encoder-built fixtures (WA1).
// ---------------------------------------------------------------------------

/// Header `H(v)` = `[v, 0x00, 0x05, 0x01]` (version, lane 0, server_tick 5,
/// event_seq 1; the op count follows at index 4).
fn h(version: u8) -> Vec<u8> {
    vec![version, LANE_COMBAT, 0x05, 0x01]
}

/// `Dv` literal: `H(v) ++ [0x01, 0x00, 0x09]` (7 bytes).
fn d_literal(version: u8) -> Vec<u8> {
    let head = h(version);
    hostile::cat(&[&head[..], &[0x01, 0x00, 0x09][..]])
}

/// `Rv` literal: the Health + Receipt ops after `H(v)` (33 bytes).
fn r_literal(version: u8) -> Vec<u8> {
    let head = h(version);
    hostile::cat(&[
        &head[..],
        &[0x02, 0x05, 0x09, 0x64, 0xC8, 0x01, 0x00, 0x07][..],
        &hostile::EPOCH[..],
        &[0x00, 0x01, 0x29, 0x01, 0x11][..],
    ])
}

/// The `EntityEnter { entity: net(9) }` frame, built by `wire`'s encoder.
fn encode_d(wire: Wire) -> Vec<u8> {
    match wire {
        Wire::V1 => codec::encode_delta(&protocol::DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 5,
            event_seq: 1,
            ops: vec![protocol::DeltaOp::EntityEnter {
                entity: hostile::net(9),
            }],
        })
        .unwrap(),
        Wire::V2 => codec_v2::encode_delta(&protocol_v2::DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 5,
            event_seq: 1,
            ops: vec![protocol_v2::DeltaOp::Legacy(
                protocol::DeltaOp::EntityEnter {
                    entity: hostile::net(9),
                },
            )],
        })
        .unwrap(),
        Wire::V3 => codec_v3::encode_delta(&protocol_v3::DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 5,
            event_seq: 1,
            ops: vec![protocol_v3::DeltaOp::EntityEnter {
                entity: hostile::net(9),
            }],
        })
        .unwrap(),
        Wire::V4 => codec_v4::encode_delta(&protocol_v4::DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 5,
            event_seq: 1,
            ops: vec![protocol_v4::DeltaOp::V3(
                protocol_v3::DeltaOp::EntityEnter {
                    entity: hostile::net(9),
                },
            )],
        })
        .unwrap(),
    }
}

/// The `Health` + `Receipt` frame, built by `wire`'s encoder.
fn encode_r(wire: Wire) -> Vec<u8> {
    let health_v1 = protocol::DeltaOp::Health {
        entity: hostile::net(9),
        health: 100,
        max_health: 200,
        dead: false,
    };
    let receipt_v1 = protocol::DeltaOp::Receipt {
        epoch: hostile::EPOCH,
        lane: LANE_COMBAT,
        seq: 1,
        processed_tick: 41,
        status: protocol::ReceiptStatus::Rejected(protocol::RejectionCode::NotAuthorized),
    };
    let health_v3 = protocol_v3::DeltaOp::Health {
        entity: hostile::net(9),
        health: 100,
        max_health: 200,
        dead: false,
    };
    let receipt_v3 = protocol_v3::DeltaOp::Receipt {
        epoch: hostile::EPOCH,
        lane: LANE_COMBAT,
        seq: 1,
        processed_tick: 41,
        status: protocol_v3::ReceiptStatus::Rejected(protocol_v3::RejectionCode::NotAuthorized),
    };
    match wire {
        Wire::V1 => codec::encode_delta(&protocol::DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 5,
            event_seq: 1,
            ops: vec![health_v1, receipt_v1],
        })
        .unwrap(),
        Wire::V2 => codec_v2::encode_delta(&protocol_v2::DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 5,
            event_seq: 1,
            ops: vec![
                protocol_v2::DeltaOp::Legacy(health_v1),
                protocol_v2::DeltaOp::Legacy(receipt_v1),
            ],
        })
        .unwrap(),
        Wire::V3 => codec_v3::encode_delta(&protocol_v3::DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 5,
            event_seq: 1,
            ops: vec![health_v3.clone(), receipt_v3.clone()],
        })
        .unwrap(),
        Wire::V4 => codec_v4::encode_delta(&protocol_v4::DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 5,
            event_seq: 1,
            ops: vec![
                protocol_v4::DeltaOp::V3(health_v3),
                protocol_v4::DeltaOp::V3(receipt_v3),
            ],
        })
        .unwrap(),
    }
}

/// The 8 fixtures of WA1 in table order, each built by its encoder.
fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture {
            name: "D1",
            wire: Wire::V1,
            bytes: encode_d(Wire::V1),
            ulid_text: vec![],
        },
        Fixture {
            name: "R1",
            wire: Wire::V1,
            bytes: encode_r(Wire::V1),
            ulid_text: vec![],
        },
        Fixture {
            name: "D2",
            wire: Wire::V2,
            bytes: encode_d(Wire::V2),
            ulid_text: vec![],
        },
        Fixture {
            name: "R2",
            wire: Wire::V2,
            bytes: encode_r(Wire::V2),
            ulid_text: vec![],
        },
        Fixture {
            name: "D3",
            wire: Wire::V3,
            bytes: encode_d(Wire::V3),
            ulid_text: vec![],
        },
        Fixture {
            name: "R3",
            wire: Wire::V3,
            bytes: encode_r(Wire::V3),
            ulid_text: vec![],
        },
        Fixture {
            name: "D4",
            wire: Wire::V4,
            bytes: encode_d(Wire::V4),
            ulid_text: vec![],
        },
        Fixture {
            name: "R4",
            wire: Wire::V4,
            bytes: encode_r(Wire::V4),
            ulid_text: vec![],
        },
    ]
}

/// The 8 fixtures' hand-written literals, in the same order.
fn pinned() -> Vec<Fixture> {
    vec![
        Fixture {
            name: "D1",
            wire: Wire::V1,
            bytes: d_literal(1),
            ulid_text: vec![],
        },
        Fixture {
            name: "R1",
            wire: Wire::V1,
            bytes: r_literal(1),
            ulid_text: vec![],
        },
        Fixture {
            name: "D2",
            wire: Wire::V2,
            bytes: d_literal(2),
            ulid_text: vec![],
        },
        Fixture {
            name: "R2",
            wire: Wire::V2,
            bytes: r_literal(2),
            ulid_text: vec![],
        },
        Fixture {
            name: "D3",
            wire: Wire::V3,
            bytes: d_literal(3),
            ulid_text: vec![],
        },
        Fixture {
            name: "R3",
            wire: Wire::V3,
            bytes: r_literal(3),
            ulid_text: vec![],
        },
        Fixture {
            name: "D4",
            wire: Wire::V4,
            bytes: d_literal(4),
            ulid_text: vec![],
        },
        Fixture {
            name: "R4",
            wire: Wire::V4,
            bytes: r_literal(4),
            ulid_text: vec![],
        },
    ]
}

/// Pinned varint byte sequences (the "Row notation" of the contract).
const VAR_1: &[u8] = &[0x01];
const VAR_256: &[u8] = &[0x80, 0x02];
const VAR_257: &[u8] = &[0x81, 0x02];
const U64MAX: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];
const U64OVF: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x02];
const TEN_CONT: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
const U32MAX: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
const U32OVF: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF, 0x10];
const U64LAST: &[u8] = &[0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];

// ---------------------------------------------------------------------------
// HA1..HA13
// ---------------------------------------------------------------------------

#[test]
fn fixtures_are_pinned() {
    let fx = fixtures();
    let literal = pinned();
    assert_eq!(fx.len(), 8, "fixture count");
    assert_eq!(literal.len(), 8, "literal count");

    for (f, p) in fx.iter().zip(literal.iter()) {
        assert_eq!(f.name, p.name);
        assert_eq!(f.wire, p.wire, "fixture {} wire", f.name);
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

    let d1 = fixture(&fx, "D1");
    assert_ne!(
        hostile::with_byte(&d1.bytes, 5, 0x01),
        d1.bytes,
        "control: D1 with op tag 1 is not D1's encoding"
    );
}

#[test]
fn short_inputs_are_refused_exactly() {
    for wire in WIRES {
        assert_eq!(
            wire.verdict(&[]),
            Verdict::Refused(CodecError::Malformed),
            "{:?} empty input",
            wire
        );
        for b in 0..=255u8 {
            let expected = if b == wire.first_byte() {
                CodecError::Malformed
            } else {
                CodecError::UnsupportedVersion
            };
            assert_eq!(
                wire.verdict(&[b]),
                Verdict::Refused(expected),
                "{:?} one-byte input {:02X?}",
                wire,
                [b]
            );
            for c in 0..=255u8 {
                let expected = if b != wire.first_byte() {
                    CodecError::UnsupportedVersion
                } else if c != wire.lane() {
                    CodecError::WrongDirection
                } else {
                    CodecError::Malformed
                };
                assert_eq!(
                    wire.verdict(&[b, c]),
                    Verdict::Refused(expected),
                    "{:?} two-byte input {:02X?}",
                    wire,
                    [b, c]
                );
            }
        }
    }

    assert_eq!(
        Wire::V1.verdict(&[1]),
        Verdict::Refused(CodecError::Malformed)
    );
    assert_eq!(
        Wire::V1.verdict(&[2]),
        Verdict::Refused(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        Wire::V1.verdict(&[1, 0]),
        Verdict::Refused(CodecError::Malformed)
    );
    assert_eq!(
        Wire::V1.verdict(&[1, 1]),
        Verdict::Refused(CodecError::WrongDirection)
    );
}

#[test]
fn oversize_is_refused_first() {
    assert_eq!(protocol::MAX_DELTA_FRAME_BYTES, 65536);

    let fx = fixtures();
    for wire in WIRES {
        assert_eq!(
            wire.verdict(&[0xFF; 65537]),
            Verdict::Refused(CodecError::FrameTooLarge),
            "{:?} 65537 bytes",
            wire
        );
        assert_eq!(
            wire.verdict(&[0xFF; 65536]),
            Verdict::Refused(CodecError::UnsupportedVersion),
            "{:?} 65536 bytes",
            wire
        );
        for f in fx.iter().filter(|f| f.wire == wire) {
            assert_eq!(
                wire.verdict(&hostile::padded(&f.bytes, 65536)),
                Verdict::Refused(CodecError::Malformed),
                "{:?} fixture {} padded to 65536: {:02X?}",
                wire,
                f.name,
                f.bytes
            );
            assert_eq!(
                wire.verdict(&hostile::padded(&f.bytes, 65537)),
                Verdict::Refused(CodecError::FrameTooLarge),
                "{:?} fixture {} padded to 65537: {:02X?}",
                wire,
                f.name,
                f.bytes
            );
        }
    }

    let d1 = fixture(&fx, "D1");
    let at_cap = hostile::padded(&d1.bytes, 65536);
    let over_cap = hostile::padded(&d1.bytes, 65537);
    assert_eq!(over_cap.len(), at_cap.len() + 1);
    assert_eq!(
        Wire::V1.verdict(&at_cap),
        Verdict::Refused(CodecError::Malformed)
    );
    assert_eq!(
        Wire::V1.verdict(&over_cap),
        Verdict::Refused(CodecError::FrameTooLarge)
    );
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
    assert_eq!(count, 160);
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
    assert_eq!(count, 24);
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
    assert_eq!(cross, 24);
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

/// WA8's verdict for `D`'s op tag byte `t` (the mutated input is its own
/// canonical form wherever the verdict is `Accepted(m)`).
fn op_tag_expected(name: &str, t: u8, mutated: &[u8]) -> Verdict {
    match name {
        "D1" => match t {
            0..=4 => Verdict::Accepted(mutated.to_vec()),
            5..=7 => Verdict::Refused(CodecError::Malformed),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        "D2" => match t {
            0..=4 => Verdict::Accepted(mutated.to_vec()),
            5..=10 => Verdict::Refused(CodecError::Malformed),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        "D3" => match t {
            0..=4 | 14 | 16 | 17 | 18 => Verdict::Accepted(mutated.to_vec()),
            15 => Verdict::Refused(CodecError::UnknownMessage),
            5..=13 | 19 | 20 => Verdict::Refused(CodecError::Malformed),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        "D4" => match t {
            0..=4 | 14 | 16 | 17 | 18 => Verdict::Accepted(mutated.to_vec()),
            15 => Verdict::Refused(CodecError::UnknownMessage),
            5..=13 | 19..=22 => Verdict::Refused(CodecError::Malformed),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        _ => panic!("no WA8 op-tag row for {name}"),
    }
}

#[test]
fn op_tag_matrix() {
    let fx = fixtures();
    let names = ["D1", "D2", "D3", "D4"];
    for name in names {
        let f = fixture(&fx, name);
        for t in 0..=255u8 {
            let input = hostile::with_byte(&f.bytes, 5, t);
            let expected = op_tag_expected(name, t, &input);
            assert_eq!(
                f.wire.verdict(&input),
                expected,
                "{:?} fixture {} with op tag {t:02X}: {:02X?}",
                f.wire,
                f.name,
                input
            );
        }
        let control = hostile::with_byte(&f.bytes, 5, f.bytes[5]);
        assert_eq!(
            f.wire.verdict(&control),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} with its own op tag",
            f.wire,
            f.name
        );
    }
}

/// `H(v)` with its op count replaced by `count_varint` and then `ops` ops
/// `00 09` (WA9's `ops256`/`ops257`).
fn ops_frame(version: u8, count_varint: &[u8], ops: usize) -> Vec<u8> {
    let mut out = h(version);
    out.extend_from_slice(count_varint);
    for _ in 0..ops {
        out.extend_from_slice(&[0x00, 0x09]);
    }
    out
}

#[test]
fn header_scalars_and_op_count() {
    let fx = fixtures();
    let rows = [
        row(
            "c-1",
            &["D1", "D2", "D3", "D4"],
            &[(4, 1, VAR_1)],
            Want::Same,
        ),
        row(
            "c-0",
            &["D1", "D2", "D3", "D4"],
            &[(4, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "c-2",
            &["D1", "D2", "D3", "D4"],
            &[(4, 1, &[0x02])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "c-256",
            &["D1", "D2", "D3", "D4"],
            &[(4, 1, VAR_256)],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "c-257",
            &["D1", "D2", "D3", "D4"],
            &[(4, 1, VAR_257)],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "c-u64max",
            &["D1", "D2", "D3", "D4"],
            &[(4, 1, U64MAX)],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "c-ovf",
            &["D1", "D2", "D3", "D4"],
            &[(4, 1, U64OVF)],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "c-10cont",
            &["D1", "D2", "D3", "D4"],
            &[(4, 1, TEN_CONT)],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "tick-u64max",
            &["D1", "D2", "D3", "D4"],
            &[(2, 1, U64MAX)],
            Want::Same,
        ),
        row(
            "tick-ovf",
            &["D1", "D2", "D3", "D4"],
            &[(2, 1, U64OVF)],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "evseq-0",
            &["D1", "D2", "D3", "D4"],
            &[(3, 1, &[0x00])],
            Want::Same,
        ),
        row(
            "evseq-u64max",
            &["D1", "D2", "D3", "D4"],
            &[(3, 1, U64MAX)],
            Want::Same,
        ),
    ];

    for r in &rows {
        for name in &r.fixtures {
            let f = fixture(&fx, name);
            let mutated = mutate(f, r);
            let got = f.wire.verdict(&mutated);
            hostile::assert_row(name, r, &f.bytes, &mutated, &got);
        }
    }

    for wire in WIRES {
        let ops256 = ops_frame(wire.first_byte(), VAR_256, 256);
        assert_eq!(
            wire.verdict(&ops256),
            Verdict::Accepted(ops256.clone()),
            "{:?} ops256",
            wire
        );
        let ops257 = ops_frame(wire.first_byte(), VAR_257, 257);
        assert_eq!(
            wire.verdict(&ops257),
            Verdict::Refused(CodecError::LimitExceeded),
            "{:?} ops257",
            wire
        );
    }
}

#[test]
fn health_and_receipt_fields() {
    let fx = fixtures();
    let r_names = ["R1", "R2", "R3", "R4"];
    let rows = [
        row(
            "h-entity0",
            &r_names,
            &[(6, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row("h-dead1", &r_names, &[(10, 1, &[0x01])], Want::Same),
        row(
            "h-dead2",
            &r_names,
            &[(10, 1, &[0x02])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "h-dead255",
            &r_names,
            &[(10, 1, &[0xFF])],
            Want::Refused(CodecError::Malformed),
        ),
        row("h-health-u32max", &r_names, &[(7, 1, U32MAX)], Want::Same),
        row(
            "h-health-u32ovf",
            &r_names,
            &[(7, 1, U32OVF)],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "h-max-nonmin",
            &r_names,
            &[(8, 2, &[0xC8, 0x81, 0x00])],
            Want::Orig,
        ),
        row(
            "r-lane1",
            &r_names,
            &[(28, 1, &[0x01])],
            Want::Refused(CodecError::WrongDirection),
        ),
        row(
            "r-lane255",
            &r_names,
            &[(28, 1, &[0xFF])],
            Want::Refused(CodecError::WrongDirection),
        ),
        row(
            "r-seq0",
            &r_names,
            &[(29, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "r-seq-u64max",
            &r_names,
            &[(29, 1, U64MAX)],
            Want::Refused(CodecError::Malformed),
        ),
        row("r-seq-last", &r_names, &[(29, 1, U64LAST)], Want::Same),
        row(
            "r-status0",
            &r_names,
            &[(31, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "r-status2",
            &r_names,
            &[(31, 1, &[0x02])],
            Want::Refused(CodecError::UnknownMessage),
        ),
        row(
            "r-status255",
            &r_names,
            &[(31, 1, &[0xFF])],
            Want::Refused(CodecError::UnknownMessage),
        ),
        row(
            "r-code0",
            &r_names,
            &[(32, 1, &[0x00])],
            Want::Refused(CodecError::UnknownMessage),
        ),
        row("r-code20", &r_names, &[(32, 1, &[0x14])], Want::Same),
        row(
            "r-code21-v1",
            &["R1", "R2"],
            &[(32, 1, &[0x15])],
            Want::Refused(CodecError::UnknownMessage),
        ),
        row(
            "r-code21-v3",
            &["R3", "R4"],
            &[(32, 1, &[0x15])],
            Want::Same,
        ),
        row(
            "r-code30-v1",
            &["R1", "R2"],
            &[(32, 1, &[0x1E])],
            Want::Refused(CodecError::UnknownMessage),
        ),
        row(
            "r-code30-v3",
            &["R3", "R4"],
            &[(32, 1, &[0x1E])],
            Want::Same,
        ),
        row(
            "r-code31",
            &r_names,
            &[(32, 1, &[0x1F])],
            Want::Refused(CodecError::UnknownMessage),
        ),
    ];

    for r in &rows {
        for name in &r.fixtures {
            let f = fixture(&fx, name);
            let mutated = mutate(f, r);
            let got = f.wire.verdict(&mutated);
            hostile::assert_row(name, r, &f.bytes, &mutated, &got);
        }
    }

    // Sweep: the Receipt status tag (index 31), all four wires.
    for name in ["R1", "R2", "R3", "R4"] {
        let f = fixture(&fx, name);
        for t in 0..=255u8 {
            let input = hostile::with_byte(&f.bytes, 31, t);
            let expected = match t {
                0 => Verdict::Refused(CodecError::Malformed),
                1 => Verdict::Accepted(f.bytes.clone()),
                _ => Verdict::Refused(CodecError::UnknownMessage),
            };
            assert_eq!(
                f.wire.verdict(&input),
                expected,
                "{:?} fixture {} status tag {t:02X}: {:02X?}",
                f.wire,
                f.name,
                input
            );
        }
    }

    // Sweep: the rejection code (index 32), v1/v2 then v3/v4.
    for name in ["R1", "R2"] {
        let f = fixture(&fx, name);
        for t in 0..=255u8 {
            let input = hostile::with_byte(&f.bytes, 32, t);
            let expected = match t {
                1..=20 => Verdict::Accepted(input.clone()),
                _ => Verdict::Refused(CodecError::UnknownMessage),
            };
            assert_eq!(
                f.wire.verdict(&input),
                expected,
                "{:?} fixture {} code {t:02X}: {:02X?}",
                f.wire,
                f.name,
                input
            );
        }
    }
    for name in ["R3", "R4"] {
        let f = fixture(&fx, name);
        for t in 0..=255u8 {
            let input = hostile::with_byte(&f.bytes, 32, t);
            let expected = match t {
                1..=30 => Verdict::Accepted(input.clone()),
                _ => Verdict::Refused(CodecError::UnknownMessage),
            };
            assert_eq!(
                f.wire.verdict(&input),
                expected,
                "{:?} fixture {} code {t:02X}: {:02X?}",
                f.wire,
                f.name,
                input
            );
        }
    }
}

/// The four upstream `EndTurn` intents `U(v)` of WA11, each built by its
/// wire's `encode_intent`.
fn upstream_intents() -> [(Wire, Vec<u8>); 4] {
    let epoch = hostile::EPOCH;
    let v12 = protocol::IntentFrame {
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 41,
        actor: hostile::net(7),
        body: protocol::IntentBody::EndTurn,
    };
    let u1 = codec::encode_intent(&v12).unwrap();
    let u2 = codec_v2::encode_intent(&v12).unwrap();
    let v3 = protocol_v3::IntentFrame {
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 41,
        actor: hostile::net(7),
        body: protocol_v3::IntentBody::EndTurn,
    };
    let u3 = codec_v3::encode_intent(&v3).unwrap();
    let v4 = protocol_v4::IntentFrame {
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 41,
        actor: hostile::net(7),
        body: protocol_v4::IntentBody::EndTurn,
    };
    let u4 = codec_v4::encode_intent(&v4).unwrap();
    [
        (Wire::V1, u1),
        (Wire::V2, u2),
        (Wire::V3, u3),
        (Wire::V4, u4),
    ]
}

#[test]
fn upstream_intents_sent_down_are_refused() {
    let intents = upstream_intents();

    for (wire, bytes) in &intents {
        assert_eq!(bytes.len(), 22, "{:?} intent length", wire);
        assert_eq!(
            wire.verdict(bytes),
            Verdict::Refused(CodecError::WrongDirection),
            "{:?} reading its own upstream intent: {:02X?}",
            wire,
            bytes
        );
    }
    for (wire, bytes) in &intents {
        for other in WIRES {
            if other == *wire {
                continue;
            }
            assert_eq!(
                other.verdict(bytes),
                Verdict::Refused(CodecError::UnsupportedVersion),
                "{:?} reading {:?}'s upstream intent: {:02X?}",
                other,
                wire,
                bytes
            );
        }
    }

    // Control: each intent's matching decode_intent is Ok and canonical.
    assert_eq!(
        codec::encode_intent(&codec::decode_intent(&intents[0].1).unwrap()).unwrap(),
        intents[0].1
    );
    assert_eq!(
        codec_v2::encode_intent(&codec_v2::decode_intent(&intents[1].1).unwrap()).unwrap(),
        intents[1].1
    );
    assert_eq!(
        codec_v3::encode_intent(&codec_v3::decode_intent(&intents[2].1).unwrap()).unwrap(),
        intents[2].1
    );
    assert_eq!(
        codec_v4::encode_intent(&codec_v4::decode_intent(&intents[3].1).unwrap()).unwrap(),
        intents[3].1
    );
}

#[test]
fn single_byte_corruption_is_refused_or_exact() {
    let fx = fixtures();
    for f in &fx {
        let counts =
            hostile::check_corruption(f.name, &|b| f.wire.verdict(b), &f.bytes, &f.ulid_text);
        assert!(counts.refused >= 1, "{} has no refused corruption", f.name);
        assert!(counts.exact >= 1, "{} has no accepted corruption", f.name);
    }
    let total_bytes: usize = fx.iter().map(|f| f.bytes.len()).sum();
    assert_eq!(total_bytes * 255, 40_800, "WA12 input count");
}

/// One full WA13 run, driven by `rng`; returns the verdict sequence, the
/// `Accepted` count of passes (c) and (d), and the input count.
fn seeded_run(rng: &mut hostile::XorShift64) -> (Vec<Verdict>, usize, usize) {
    let mut verdicts = Vec::new();
    let mut accepted = 0usize;
    let mut inputs = 0usize;

    for wire in WIRES {
        for _ in 0..1024 {
            inputs += 1;
            let len = 1 + (rng.next() % 64) as usize;
            let mut input = Vec::new();
            for _ in 0..len {
                input.push(rng.next() as u8);
            }
            input[0] = wire.first_byte();
            if len > 1 {
                input[1] = 0;
            }
            assert!(
                hostile::stable(&|b| wire.verdict(b), &input),
                "(a) {:?} is not stable on {:02X?}",
                wire,
                input
            );
            verdicts.push(wire.verdict(&input));
        }
    }

    for _ in 0..1024 {
        inputs += 1;
        let len = (rng.next() % 65) as usize;
        let mut input = Vec::new();
        for _ in 0..len {
            input.push(rng.next() as u8);
        }
        for wire in WIRES {
            assert!(
                hostile::stable(&|b| wire.verdict(b), &input),
                "(b) {:?} is not stable on {:02X?}",
                wire,
                input
            );
            verdicts.push(wire.verdict(&input));
        }
    }

    let fx = fixtures();
    for name in ["R1", "R2", "R3", "R4"] {
        let f = fixture(&fx, name);
        for _ in 0..256 {
            inputs += 1;
            let i = 12 + (rng.next() % 16) as usize;
            let v = rng.next() as u8;
            let mutated = hostile::with_byte(&f.bytes, i, v);
            let verdict = f.wire.verdict(&mutated);
            assert_eq!(
                verdict,
                Verdict::Accepted(mutated.clone()),
                "(c) {:?} fixture {} epoch byte {i} := {v:02X}: {:02X?}",
                f.wire,
                f.name,
                mutated
            );
            accepted += 1;
            verdicts.push(verdict);
        }
    }

    for f in fixtures() {
        for _ in 0..256 {
            inputs += 1;
            let span = f.bytes.len() - 5;
            let i = 5 + (rng.next() % span as u64) as usize;
            let v = rng.next() as u8;
            let mutated = hostile::with_byte(&f.bytes, i, v);
            assert!(
                hostile::stable(&|b| f.wire.verdict(b), &mutated),
                "(d) {:?} fixture {} byte {i} := {v:02X} is not stable: {:02X?}",
                f.wire,
                f.name,
                mutated
            );
            let verdict = f.wire.verdict(&mutated);
            if matches!(verdict, Verdict::Accepted(_)) {
                accepted += 1;
            }
            verdicts.push(verdict);
        }
    }

    (verdicts, accepted, inputs)
}

#[test]
fn seeded_noise_is_stable() {
    let mut rng = hostile::XorShift64(hostile::SEED);
    assert_eq!(rng.next(), 0x4B10_DA10_A18C_50C2);
    assert_eq!(rng.next(), 0x8788_B83B_1805_38E3);
    assert_eq!(rng.next(), 0x18B5_5443_1CC3_6A52);

    let (first, accepted, inputs) = seeded_run(&mut rng);
    assert_eq!(inputs, 8192, "WA13 input count");
    assert!(
        accepted >= 1024,
        "only {accepted} accepted verdicts in (c) and (d)"
    );

    let mut again = hostile::XorShift64(hostile::SEED);
    assert_eq!(again.next(), 0x4B10_DA10_A18C_50C2);
    assert_eq!(again.next(), 0x8788_B83B_1805_38E3);
    assert_eq!(again.next(), 0x18B5_5443_1CC3_6A52);
    let (second, _, _) = seeded_run(&mut again);
    assert_eq!(first, second, "the seeded run is not deterministic");
}
