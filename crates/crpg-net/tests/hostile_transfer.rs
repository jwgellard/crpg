//! T042b3: the hostile-server transfer suite (spec §16.2, T042b part 3).
//! The `0x81`/`0x82` downstream frame decoders (Begin, Chunk, Cancel) and the
//! two snapshot bodies fed byte-exact hostile inputs. No clock, no thread, no
//! I/O, no unseeded RNG, no `HashMap`/`HashSet`, no float.

use std::ops::Range;

use crpg_net::codec::CodecError;
use crpg_net::codec_transfer;
use crpg_net::protocol::{MAX_SNAPSHOT_BYTES, MAX_VISIBLE_ENTITIES};
use crpg_net::protocol_transfer::{
    BeginTransfer, CancelReason, HealthFact, ResyncReason, SnapshotBody, SnapshotEntity,
    TransferChunk, TransferDown, TransferUp, TurnFact, LANE_SNAPSHOT, MAX_DOWN_FRAME_BYTES,
    TRANSFER_VERSION,
};
use crpg_net::protocol_v3::{EncounterFacts, FactSet, OrderFact, TimelineEntry};
use crpg_net::transfer_v3::{self, SnapshotBodyV3, SnapshotEntityV3, MAX_SNAPSHOT_BYTES_V3};

#[path = "support/hostile.rs"]
mod hostile;

use hostile::{row, Verdict, Want};

/// The `ABILITY` text, 26 canonical Crockford base32 bytes.
const ABILITY_TEXT: &[u8] = b"014D2PF2DB0000SQQG28T5CY4T";

// ---------------------------------------------------------------------------
// Subject table (WC1).
// ---------------------------------------------------------------------------

/// One client-side transfer decoder with its own encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Subject {
    T81,
    T82,
    B81,
    B82,
}

/// Table order; every "for each subject" loop uses it.
const SUBJECTS: [Subject; 4] = [Subject::T81, Subject::T82, Subject::B81, Subject::B82];

impl Subject {
    /// `0x81` or `0x82`; panics for a body, which has no version byte.
    fn first_byte(self) -> u8 {
        match self {
            Subject::T81 => TRANSFER_VERSION,
            Subject::T82 => transfer_v3::TRANSFER_VERSION_V3,
            Subject::B81 | Subject::B82 => panic!("{self:?} has no version byte"),
        }
    }

    /// The lane byte at index 1: always `LANE_SNAPSHOT` (2).
    fn lane(self) -> u8 {
        LANE_SNAPSHOT
    }

    /// Whether this subject is a frame (a version byte and a lane) rather
    /// than a bare body.
    fn is_frame(self) -> bool {
        matches!(self, Subject::T81 | Subject::T82)
    }

    /// Decode with this subject's decoder; on `Ok(v)`, re-encode `v`. An
    /// encoder error on a decoded value panics with the subject and the bytes
    /// (Reading 9).
    fn verdict(self, bytes: &[u8]) -> Verdict {
        match self {
            Subject::T81 => hostile::reencoded(
                "T81",
                bytes,
                codec_transfer::decode_down(bytes),
                codec_transfer::encode_down,
            ),
            Subject::T82 => hostile::reencoded(
                "T82",
                bytes,
                transfer_v3::decode_down(bytes),
                transfer_v3::encode_down,
            ),
            Subject::B81 => hostile::reencoded(
                "B81",
                bytes,
                codec_transfer::decode_body(bytes),
                codec_transfer::encode_body,
            ),
            Subject::B82 => hostile::reencoded(
                "B82",
                bytes,
                transfer_v3::decode_body(bytes),
                transfer_v3::encode_body,
            ),
        }
    }

    /// The size cap, checked before anything else (WC3).
    fn cap(self) -> usize {
        match self {
            Subject::T81 | Subject::T82 => MAX_DOWN_FRAME_BYTES,
            Subject::B81 => MAX_SNAPSHOT_BYTES,
            Subject::B82 => MAX_SNAPSHOT_BYTES_V3,
        }
    }
}

/// The `ulid_text` range of `Y82`: its one 26-byte ULID text (indices 12..38).
fn ability_text_range() -> Range<usize> {
    12..38
}

/// One pinned valid input.
struct Fixture {
    name: &'static str,
    subject: Subject,
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
// Pinned literals and encoder-built fixtures (WC1).
// ---------------------------------------------------------------------------

/// `Gw` literal: `[w, 0x02, 0x10] ++ EPOCH ++ [0x01, 0x29, 0x03, 0x88, 0x27,
/// 0x02, 0x01, 0x02]` (27 bytes).
fn g_literal(version: u8) -> Vec<u8> {
    hostile::cat(&[
        &[version, LANE_SNAPSHOT, 0x10][..],
        &hostile::EPOCH[..],
        &[0x01, 0x29, 0x03, 0x88, 0x27, 0x02, 0x01, 0x02][..],
    ])
}

/// `Kw` literal: `[w, 0x02, 0x11] ++ EPOCH ++ [0x01, 0x00, 0x01, 0x05,
/// 0xAB×5]` (28 bytes).
fn k_literal(version: u8) -> Vec<u8> {
    hostile::cat(&[
        &[version, LANE_SNAPSHOT, 0x11][..],
        &hostile::EPOCH[..],
        &[0x01, 0x00, 0x01, 0x05, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB][..],
    ])
}

/// `Cw` literal: `[w, 0x02, 0x12] ++ EPOCH ++ [0x01, 0x02]` (21 bytes).
fn c_literal(version: u8) -> Vec<u8> {
    hostile::cat(&[
        &[version, LANE_SNAPSHOT, 0x12][..],
        &hostile::EPOCH[..],
        &[0x01, 0x02][..],
    ])
}

/// `Y81` literal (13 bytes).
fn y81_literal() -> Vec<u8> {
    vec![
        0x02, 0x01, 0x01, 0x64, 0xC8, 0x01, 0x00, 0x02, 0x00, 0x01, 0x01, 0x02, 0x03,
    ]
}

/// `Y82` literal (62 bytes).
fn y82_literal() -> Vec<u8> {
    hostile::cat(&[
        &[0x02, 0x01, 0x01, 0x64, 0xC8, 0x01, 0x00][..],
        &[0x00, 0x00, 0x01, 0x01, 0x1A][..],
        ABILITY_TEXT,
        &[0x02, 0x00, 0x00, 0x00][..],
        &[0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00][..],
        &[0x01, 0x01, 0x02, 0x03][..],
        &[0x00, 0x00, 0x00][..],
        &[0x01, 0x02, 0x00, 0x01, 0x0A, 0x02][..],
    ])
}

/// The `BeginTransfer` of `Gw`.
fn begin() -> BeginTransfer {
    BeginTransfer {
        epoch: hostile::EPOCH,
        transfer: hostile::tid(1),
        base_tick: 41,
        cursor: 3,
        total_bytes: 5000,
        entity_count: 2,
        segments_per_chunk: 1,
        chunk_count: 2,
    }
}

/// `Gw`, built by `subject`'s encoder.
fn encode_g(subject: Subject) -> Vec<u8> {
    let msg = TransferDown::Begin(begin());
    match subject {
        Subject::T81 => codec_transfer::encode_down(&msg).unwrap(),
        Subject::T82 => transfer_v3::encode_down(&msg).unwrap(),
        _ => panic!("{subject:?} is not a frame subject"),
    }
}

/// `Kw`, built by `subject`'s encoder.
fn encode_k(subject: Subject) -> Vec<u8> {
    let msg = TransferDown::Chunk(TransferChunk {
        epoch: hostile::EPOCH,
        transfer: hostile::tid(1),
        index: 0,
        bytes: vec![0xAB; 5],
    });
    match subject {
        Subject::T81 => codec_transfer::encode_down(&msg).unwrap(),
        Subject::T82 => transfer_v3::encode_down(&msg).unwrap(),
        _ => panic!("{subject:?} is not a frame subject"),
    }
}

/// `Cw`, built by `subject`'s encoder.
fn encode_c(subject: Subject) -> Vec<u8> {
    let msg = TransferDown::Cancel {
        epoch: hostile::EPOCH,
        transfer: hostile::tid(1),
        reason: CancelReason::Aborted,
    };
    match subject {
        Subject::T81 => codec_transfer::encode_down(&msg).unwrap(),
        Subject::T82 => transfer_v3::encode_down(&msg).unwrap(),
        _ => panic!("{subject:?} is not a frame subject"),
    }
}

/// `Y81`, built by its encoder.
fn encode_y81() -> Vec<u8> {
    codec_transfer::encode_body(&SnapshotBody {
        entities: vec![
            SnapshotEntity {
                entity: hostile::net(1),
                health: Some(HealthFact {
                    health: 100,
                    max_health: 200,
                    dead: false,
                }),
            },
            SnapshotEntity {
                entity: hostile::net(2),
                health: None,
            },
        ],
        turn: Some(TurnFact {
            active: Some(hostile::net(2)),
            round: 3,
        }),
    })
    .unwrap()
}

/// `Y82`, built by its encoder.
fn encode_y82() -> Vec<u8> {
    let facts = FactSet {
        order: Some(Some(OrderFact {
            ability: hostile::ability(),
            target: hostile::net(2),
            repeat: false,
        })),
        ..FactSet::default()
    };
    transfer_v3::encode_body(&SnapshotBodyV3 {
        entities: vec![
            SnapshotEntityV3 {
                entity: hostile::net(1),
                health: Some(HealthFact {
                    health: 100,
                    max_health: 200,
                    dead: false,
                }),
                facts,
            },
            SnapshotEntityV3 {
                entity: hostile::net(2),
                health: None,
                facts: FactSet::default(),
            },
        ],
        turn: Some(TurnFact {
            active: Some(hostile::net(2)),
            round: 3,
        }),
        encounter: EncounterFacts::default(),
        timeline: Some(vec![
            TimelineEntry {
                key: 0,
                entity: hostile::net(1),
            },
            TimelineEntry {
                key: 5,
                entity: hostile::net(2),
            },
        ]),
    })
    .unwrap()
}

/// The 8 fixtures of WC1 in table order, each built by its encoder.
fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture {
            name: "G81",
            subject: Subject::T81,
            bytes: encode_g(Subject::T81),
            ulid_text: vec![],
        },
        Fixture {
            name: "K81",
            subject: Subject::T81,
            bytes: encode_k(Subject::T81),
            ulid_text: vec![],
        },
        Fixture {
            name: "C81",
            subject: Subject::T81,
            bytes: encode_c(Subject::T81),
            ulid_text: vec![],
        },
        Fixture {
            name: "G82",
            subject: Subject::T82,
            bytes: encode_g(Subject::T82),
            ulid_text: vec![],
        },
        Fixture {
            name: "K82",
            subject: Subject::T82,
            bytes: encode_k(Subject::T82),
            ulid_text: vec![],
        },
        Fixture {
            name: "C82",
            subject: Subject::T82,
            bytes: encode_c(Subject::T82),
            ulid_text: vec![],
        },
        Fixture {
            name: "Y81",
            subject: Subject::B81,
            bytes: encode_y81(),
            ulid_text: vec![],
        },
        Fixture {
            name: "Y82",
            subject: Subject::B82,
            bytes: encode_y82(),
            ulid_text: vec![ability_text_range()],
        },
    ]
}

/// The 8 fixtures' hand-written literals, in the same order.
fn pinned() -> Vec<Fixture> {
    vec![
        Fixture {
            name: "G81",
            subject: Subject::T81,
            bytes: g_literal(0x81),
            ulid_text: vec![],
        },
        Fixture {
            name: "K81",
            subject: Subject::T81,
            bytes: k_literal(0x81),
            ulid_text: vec![],
        },
        Fixture {
            name: "C81",
            subject: Subject::T81,
            bytes: c_literal(0x81),
            ulid_text: vec![],
        },
        Fixture {
            name: "G82",
            subject: Subject::T82,
            bytes: g_literal(0x82),
            ulid_text: vec![],
        },
        Fixture {
            name: "K82",
            subject: Subject::T82,
            bytes: k_literal(0x82),
            ulid_text: vec![],
        },
        Fixture {
            name: "C82",
            subject: Subject::T82,
            bytes: c_literal(0x82),
            ulid_text: vec![],
        },
        Fixture {
            name: "Y81",
            subject: Subject::B81,
            bytes: y81_literal(),
            ulid_text: vec![],
        },
        Fixture {
            name: "Y82",
            subject: Subject::B82,
            bytes: y82_literal(),
            ulid_text: vec![ability_text_range()],
        },
    ]
}

/// Pinned varint byte sequences (the "Row notation" of the contract).
const U64MAX: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];
const U64LAST: &[u8] = &[0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];
const VAR_1024: &[u8] = &[0x80, 0x08];
const VAR_1025: &[u8] = &[0x81, 0x08];

/// Encodes `value` as an unsigned LEB128 varint (the wire form `postcard`
/// uses), for hand-built chunk frames only.
fn varint(value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = value;
    loop {
        let byte = (rest & 0x7F) as u8;
        rest >>= 7;
        if rest == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

/// A hand-built chunk frame: `[w, 0x02, 0x11] ++ EPOCH ++ [0x01, 0x00,
/// <count>]`, then per segment its length as a varint and its bytes.
fn hand_chunk(version: u8, segments: &[usize]) -> Vec<u8> {
    let mut out = hostile::cat(&[
        &[version, LANE_SNAPSHOT, 0x11][..],
        &hostile::EPOCH[..],
        &[0x01, 0x00, segments.len() as u8][..],
    ]);
    for &len in segments {
        out.extend_from_slice(&varint(len as u64));
        out.extend(std::iter::repeat_n(0xCD, len));
    }
    out
}

/// A full chunk frame built by `subject`'s encoder from `payload` bytes of
/// `0xCD`.
fn encoded_chunk(subject: Subject, payload: usize) -> Vec<u8> {
    let msg = TransferDown::Chunk(TransferChunk {
        epoch: hostile::EPOCH,
        transfer: hostile::tid(1),
        index: 0,
        bytes: vec![0xCD; payload],
    });
    match subject {
        Subject::T81 => codec_transfer::encode_down(&msg).unwrap(),
        Subject::T82 => transfer_v3::encode_down(&msg).unwrap(),
        _ => panic!("{subject:?} is not a frame subject"),
    }
}

// ---------------------------------------------------------------------------
// HC1..HC13
// ---------------------------------------------------------------------------

#[test]
fn fixtures_are_pinned() {
    let fx = fixtures();
    let literal = pinned();
    assert_eq!(fx.len(), 8, "fixture count");
    assert_eq!(literal.len(), 8, "literal count");
    assert_eq!(hostile::ability().to_string().as_bytes(), ABILITY_TEXT);

    for (f, p) in fx.iter().zip(literal.iter()) {
        assert_eq!(f.name, p.name);
        assert_eq!(f.subject, p.subject, "fixture {} subject", f.name);
        assert_eq!(
            f.bytes, p.bytes,
            "fixture {} encoder bytes {:02X?} != literal {:02X?}",
            f.name, f.bytes, p.bytes
        );
        assert_eq!(
            f.subject.verdict(&f.bytes),
            Verdict::Accepted(f.bytes.clone()),
            "fixture {} is not canonical: {:02X?}",
            f.name,
            f.bytes
        );
    }

    let lengths = [27usize, 28, 21, 27, 28, 21, 13, 62];
    for (f, len) in fx.iter().zip(lengths) {
        assert_eq!(f.bytes.len(), len, "fixture {} length", f.name);
    }

    assert_eq!(Subject::T81.first_byte(), TRANSFER_VERSION);
    assert_eq!(Subject::T81.first_byte(), 0x81);
    assert_eq!(Subject::T82.first_byte(), transfer_v3::TRANSFER_VERSION_V3);
    assert_eq!(Subject::T82.first_byte(), 0x82);
    for subject in [Subject::T81, Subject::T82] {
        assert_eq!(subject.lane(), LANE_SNAPSHOT, "{subject:?} lane");
    }
    assert_eq!(LANE_SNAPSHOT, 2);
    assert_eq!(MAX_DOWN_FRAME_BYTES, 65_536);
    assert_eq!(MAX_SNAPSHOT_BYTES, 1_048_576);
    assert_eq!(MAX_SNAPSHOT_BYTES_V3, 4_194_304);

    // One-byte-off literal control of HA1: the chunk_count is 2, not 3.
    let mut off = g_literal(0x81);
    off[26] = 0x03;
    assert_ne!(
        off,
        encode_g(Subject::T81),
        "control: G81 with chunk_count 3 is not G81's encoding"
    );
}

#[test]
fn short_inputs_are_refused_exactly() {
    for subject in [Subject::T81, Subject::T82] {
        assert_eq!(
            subject.verdict(&[]),
            Verdict::Refused(CodecError::Malformed),
            "{subject:?} empty input"
        );
        for b in 0..=255u8 {
            let expected = if b == subject.first_byte() {
                CodecError::Malformed
            } else {
                CodecError::UnsupportedVersion
            };
            assert_eq!(
                subject.verdict(&[b]),
                Verdict::Refused(expected),
                "{subject:?} one-byte input {:02X?}",
                [b]
            );
            for c in 0..=255u8 {
                let expected = if b != subject.first_byte() {
                    CodecError::UnsupportedVersion
                } else if c != subject.lane() {
                    CodecError::WrongDirection
                } else {
                    CodecError::Malformed
                };
                assert_eq!(
                    subject.verdict(&[b, c]),
                    Verdict::Refused(expected),
                    "{subject:?} two-byte input {:02X?}",
                    [b, c]
                );
            }
        }
    }

    for subject in [Subject::B81, Subject::B82] {
        assert_eq!(
            subject.verdict(&[]),
            Verdict::Refused(CodecError::Malformed),
            "{subject:?} empty input"
        );
        for b in 0..=255u8 {
            assert_eq!(
                subject.verdict(&[b]),
                Verdict::Refused(CodecError::Malformed),
                "{subject:?} one-byte input {:02X?}",
                [b]
            );
            for c in 0..=255u8 {
                let expected = if subject == Subject::B81 && b == 0x00 && c == 0x00 {
                    Verdict::Accepted(vec![0x00, 0x00])
                } else if b >= 0x80 && c < 0x80 {
                    let count = u32::from(b & 0x7F) | (u32::from(c) << 7);
                    if count > MAX_VISIBLE_ENTITIES as u32 {
                        Verdict::Refused(CodecError::LimitExceeded)
                    } else {
                        Verdict::Refused(CodecError::Malformed)
                    }
                } else {
                    Verdict::Refused(CodecError::Malformed)
                };
                assert_eq!(
                    subject.verdict(&[b, c]),
                    expected,
                    "{subject:?} two-byte input {:02X?}",
                    [b, c]
                );
            }
        }
    }

    // The rule's own branches, as controls.
    assert_eq!(
        Subject::T81.verdict(&[0x81]),
        Verdict::Refused(CodecError::Malformed)
    );
    assert_eq!(
        Subject::T81.verdict(&[0x80]),
        Verdict::Refused(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        Subject::T81.verdict(&[0x81, 0x02]),
        Verdict::Refused(CodecError::Malformed)
    );
    assert_eq!(
        Subject::T81.verdict(&[0x81, 0x01]),
        Verdict::Refused(CodecError::WrongDirection)
    );
    assert_eq!(
        Subject::B81.verdict(&[0x00, 0x00]),
        Verdict::Accepted(vec![0x00, 0x00])
    );
    assert_eq!(
        Subject::B82.verdict(&[0x00, 0x00]),
        Verdict::Refused(CodecError::Malformed)
    );
}

#[test]
fn oversize_is_refused_first() {
    let fx = fixtures();

    for subject in [Subject::T81, Subject::T82] {
        assert_eq!(
            subject.verdict(&vec![0xFF; 65_537]),
            Verdict::Refused(CodecError::FrameTooLarge),
            "{subject:?} 65537 bytes"
        );
        assert_eq!(
            subject.verdict(&vec![0xFF; 65_536]),
            Verdict::Refused(CodecError::UnsupportedVersion),
            "{subject:?} 65536 bytes"
        );
        for f in fx.iter().filter(|f| f.subject == subject) {
            assert_eq!(
                subject.verdict(&hostile::padded(&f.bytes, 65_536)),
                Verdict::Refused(CodecError::Malformed),
                "{subject:?} fixture {} padded to 65536",
                f.name
            );
            assert_eq!(
                subject.verdict(&hostile::padded(&f.bytes, 65_537)),
                Verdict::Refused(CodecError::FrameTooLarge),
                "{subject:?} fixture {} padded to 65537",
                f.name
            );
        }
    }

    for subject in [Subject::B81, Subject::B82] {
        let cap = subject.cap();
        assert_eq!(
            subject.verdict(&vec![0xFF; cap + 1]),
            Verdict::Refused(CodecError::LimitExceeded),
            "{subject:?} {} bytes",
            cap + 1
        );
        assert_eq!(
            subject.verdict(&vec![0xFF; cap]),
            Verdict::Refused(CodecError::Malformed),
            "{subject:?} {} bytes",
            cap
        );
        let f = fx.iter().find(|f| f.subject == subject).unwrap();
        assert_eq!(
            subject.verdict(&hostile::padded(&f.bytes, cap)),
            Verdict::Refused(CodecError::Malformed),
            "{subject:?} fixture {} padded to {cap}",
            f.name
        );
        assert_eq!(
            subject.verdict(&hostile::padded(&f.bytes, cap + 1)),
            Verdict::Refused(CodecError::LimitExceeded),
            "{subject:?} fixture {} padded to {}",
            f.name,
            cap + 1
        );
    }

    let y81 = fixture(&fx, "Y81");
    assert_eq!(
        Subject::B81.verdict(&hostile::padded(&y81.bytes, 1_048_576)),
        Verdict::Refused(CodecError::Malformed)
    );
    assert_eq!(
        Subject::B81.verdict(&hostile::padded(&y81.bytes, 1_048_577)),
        Verdict::Refused(CodecError::LimitExceeded)
    );
}

#[test]
fn every_truncation_is_malformed() {
    let fx = fixtures();
    let mut count = 0usize;
    for f in &fx {
        for n in 0..f.bytes.len() {
            assert_eq!(
                f.subject.verdict(&f.bytes[..n]),
                Verdict::Refused(CodecError::Malformed),
                "{:?} fixture {} prefix of {n} bytes: {:02X?}",
                f.subject,
                f.name,
                &f.bytes[..n]
            );
            count += 1;
        }
        assert_eq!(
            f.subject.verdict(&f.bytes),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} full bytes",
            f.subject,
            f.name
        );
    }
    assert_eq!(count, 227);
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
                f.subject.verdict(&input),
                Verdict::Refused(CodecError::Malformed),
                "{:?} fixture {} plus {extra:02X}: {:02X?}",
                f.subject,
                f.name,
                input
            );
            count += 1;
        }
        assert_eq!(
            f.subject.verdict(&f.bytes),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} without the extra byte",
            f.subject,
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
        for subject in [Subject::T81, Subject::T82] {
            if !f.subject.is_frame() || subject == f.subject {
                continue;
            }
            assert_eq!(
                subject.verdict(&f.bytes),
                Verdict::Refused(CodecError::UnsupportedVersion),
                "{subject:?} reading fixture {} of {:?}: {:02X?}",
                f.name,
                f.subject,
                f.bytes
            );
            cross += 1;
        }
        if !f.subject.is_frame() {
            continue;
        }
        for b in 0..=255u8 {
            if b == f.bytes[0] {
                continue;
            }
            let input = hostile::with_byte(&f.bytes, 0, b);
            assert_eq!(
                f.subject.verdict(&input),
                Verdict::Refused(CodecError::UnsupportedVersion),
                "{:?} fixture {} with version byte {b:02X}: {:02X?}",
                f.subject,
                f.name,
                input
            );
        }
        assert_eq!(
            f.subject.verdict(&f.bytes),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} on its own subject",
            f.subject,
            f.name
        );
    }
    assert_eq!(cross, 6);
}

#[test]
fn foreign_lanes_are_wrong_direction() {
    let fx = fixtures();
    for f in &fx {
        if !f.subject.is_frame() {
            continue;
        }
        for v in 0..=255u8 {
            if v == LANE_SNAPSHOT {
                continue;
            }
            let input = hostile::with_byte(&f.bytes, 1, v);
            assert_eq!(
                f.subject.verdict(&input),
                Verdict::Refused(CodecError::WrongDirection),
                "{:?} fixture {} with lane byte {v:02X} at 1: {:02X?}",
                f.subject,
                f.name,
                input
            );
        }
        assert_eq!(
            f.subject
                .verdict(&hostile::with_byte(&f.bytes, 1, LANE_SNAPSHOT)),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} with its own lane byte",
            f.subject,
            f.name
        );
    }
}

/// WC8's verdict for the frame fixtures' tag byte `t` (the mutated input is
/// its own canonical form wherever the verdict is `Accepted(m)`).
fn tag_expected(name: &str, t: u8, mutated: &[u8]) -> Verdict {
    match name {
        "G81" | "G82" => match t {
            0x10 => Verdict::Accepted(mutated.to_vec()),
            0x11 => Verdict::Refused(CodecError::LimitExceeded),
            0x12 => Verdict::Refused(CodecError::UnknownMessage),
            0x20 | 0x21 => Verdict::Refused(CodecError::WrongDirection),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        "K81" | "K82" => match t {
            0x10 => Verdict::Refused(CodecError::Malformed),
            0x11 => Verdict::Accepted(mutated.to_vec()),
            0x12 => Verdict::Refused(CodecError::UnknownMessage),
            0x20 | 0x21 => Verdict::Refused(CodecError::WrongDirection),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        "C81" | "C82" => match t {
            0x10 | 0x11 => Verdict::Refused(CodecError::Malformed),
            0x12 => Verdict::Accepted(mutated.to_vec()),
            0x20 | 0x21 => Verdict::Refused(CodecError::WrongDirection),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        _ => panic!("no WC8 tag row for {name}"),
    }
}

#[test]
fn tag_matrix() {
    let fx = fixtures();
    let names = ["G81", "K81", "C81", "G82", "K82", "C82"];
    let mut inputs = 0usize;
    for name in names {
        let f = fixture(&fx, name);
        for t in 0..=255u8 {
            let input = hostile::with_byte(&f.bytes, 2, t);
            let expected = tag_expected(name, t, &input);
            assert_eq!(
                f.subject.verdict(&input),
                expected,
                "{:?} fixture {} with tag {t:02X}: {:02X?}",
                f.subject,
                f.name,
                input
            );
            inputs += 1;
        }
        let control = hostile::with_byte(&f.bytes, 2, f.bytes[2]);
        assert_eq!(
            f.subject.verdict(&control),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} with its own tag",
            f.subject,
            f.name
        );
    }
    assert_eq!(inputs, 1_536);
}

#[test]
fn frame_fields_sweeps_and_built_chunks() {
    let fx = fixtures();
    let g = ["G81", "G82"];
    let k = ["K81", "K82"];
    let c = ["C81", "C82"];

    let rows = [
        row(
            "g-transfer0",
            &g,
            &[(19, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "g-cursor-u64max",
            &g,
            &[(21, 1, U64MAX)],
            Want::Refused(CodecError::Malformed),
        ),
        row("g-cursor-last", &g, &[(21, 1, U64LAST)], Want::Same),
        row(
            "g-total-0",
            &g,
            &[(22, 2, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "g-total-1048577-t81",
            &["G81"],
            &[(22, 2, &[0x81, 0x80, 0x40])],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "g-total-1048577-t82",
            &["G82"],
            &[(22, 2, &[0x81, 0x80, 0x40])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "g-total-4194305",
            &g,
            &[(22, 2, &[0x81, 0x80, 0x80, 0x02])],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row("g-entities-1024", &g, &[(24, 1, VAR_1024)], Want::Same),
        row(
            "g-entities-1025",
            &g,
            &[(24, 1, VAR_1025)],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "g-spc-0",
            &g,
            &[(25, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "g-spc-8",
            &g,
            &[(25, 1, &[0x08])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "g-spc-9",
            &g,
            &[(25, 1, &[0x09])],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "g-chunks-0",
            &g,
            &[(26, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "g-chunks-1",
            &g,
            &[(26, 1, &[0x01])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "g-chunks-3",
            &g,
            &[(26, 1, &[0x03])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "g-32chunks",
            &g,
            &[(22, 2, &[0x80, 0x80, 0x08]), (26, 1, &[0x20])],
            Want::Same,
        ),
        row(
            "g-33chunks-t81",
            &["G81"],
            &[(22, 2, &[0x81, 0x80, 0x08]), (26, 1, &[0x21])],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "g-33chunks-t82",
            &["G82"],
            &[(22, 2, &[0x81, 0x80, 0x08]), (26, 1, &[0x21])],
            Want::Same,
        ),
        row(
            "g-128chunks-t81",
            &["G81"],
            &[(22, 2, &[0x80, 0x80, 0x20]), (26, 1, &[0x80])],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "g-128chunks-t82",
            &["G82"],
            &[(22, 2, &[0x80, 0x80, 0x20]), (26, 1, &[0x80])],
            Want::Same,
        ),
        row(
            "g-129chunks",
            &g,
            &[(22, 2, &[0x81, 0x80, 0x20]), (26, 1, &[0x81])],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "g-8spc-1chunk",
            &g,
            &[
                (22, 2, &[0x80, 0x80, 0x02]),
                (25, 1, &[0x08]),
                (26, 1, &[0x01]),
            ],
            Want::Same,
        ),
        row(
            "k-transfer0",
            &k,
            &[(19, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "k-segcount-0",
            &k,
            &[(21, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "k-segcount-2",
            &k,
            &[(21, 1, &[0x02])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "k-segcount-9",
            &k,
            &[(21, 1, &[0x09])],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "k-len-0",
            &k,
            &[(22, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "k-len-4097",
            &k,
            &[(22, 1, &[0x81, 0x20])],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "k-len-4096",
            &k,
            &[(22, 1, &[0x80, 0x20])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "k-len-6",
            &k,
            &[(22, 1, &[0x06])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "k-len-4",
            &k,
            &[(22, 1, &[0x04])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "c-transfer0",
            &c,
            &[(19, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row("c-reason1", &c, &[(20, 1, &[0x01])], Want::Same),
        row(
            "c-reason0",
            &c,
            &[(20, 1, &[0x00])],
            Want::Refused(CodecError::UnknownMessage),
        ),
        row(
            "c-reason3",
            &c,
            &[(20, 1, &[0x03])],
            Want::Refused(CodecError::UnknownMessage),
        ),
    ];

    for r in &rows {
        for name in &r.fixtures {
            let f = fixture(&fx, name);
            let mutated = mutate(f, r);
            let got = f.subject.verdict(&mutated);
            hostile::assert_row(name, r, &f.bytes, &mutated, &got);
        }
    }

    // Sweeps: a chunk's index byte (index 20).
    for (name, cap) in [("K81", 32u8), ("K82", 128u8)] {
        let f = fixture(&fx, name);
        for t in 0..=255u8 {
            let input = hostile::with_byte(&f.bytes, 20, t);
            let expected = if t < cap {
                Verdict::Accepted(input.clone())
            } else {
                Verdict::Refused(CodecError::LimitExceeded)
            };
            assert_eq!(
                f.subject.verdict(&input),
                expected,
                "{:?} fixture {} chunk index {t:02X}: {:02X?}",
                f.subject,
                f.name,
                input
            );
        }
    }

    // Sweep: a Cancel's reason byte (index 20).
    for name in ["C81", "C82"] {
        let f = fixture(&fx, name);
        for t in 0..=255u8 {
            let input = hostile::with_byte(&f.bytes, 20, t);
            let expected = match t {
                0 => Verdict::Refused(CodecError::UnknownMessage),
                1 | 2 => Verdict::Accepted(input.clone()),
                _ => Verdict::Refused(CodecError::UnknownMessage),
            };
            assert_eq!(
                f.subject.verdict(&input),
                expected,
                "{:?} fixture {} cancel reason {t:02X}: {:02X?}",
                f.subject,
                f.name,
                input
            );
        }
    }

    // Built chunks: m1 and m4 are encoder-built, m2, m3 and m5 are written by
    // hand from the layout.
    for subject in [Subject::T81, Subject::T82] {
        let version = subject.first_byte();

        let m1 = encoded_chunk(subject, 4_099);
        assert_eq!(m1.len(), 4_124, "{subject:?} m1 length");
        assert_eq!(
            subject.verdict(&m1),
            Verdict::Accepted(m1.clone()),
            "{subject:?} m1"
        );

        let m2 = hand_chunk(version, &[4_095, 4]);
        assert_eq!(
            subject.verdict(&m2),
            Verdict::Refused(CodecError::Malformed),
            "{subject:?} m2 (a non-final segment shorter than 4096)"
        );

        let m3 = hand_chunk(version, &[4_096, 4_097]);
        assert_eq!(
            subject.verdict(&m3),
            Verdict::Refused(CodecError::LimitExceeded),
            "{subject:?} m3 (a segment length over 4096)"
        );

        let m4 = encoded_chunk(subject, 28_673);
        assert_eq!(m4.len(), 28_710, "{subject:?} m4 length");
        assert_eq!(
            subject.verdict(&m4),
            Verdict::Accepted(m4.clone()),
            "{subject:?} m4"
        );

        let m5 = hand_chunk(
            version,
            &[4_096, 4_096, 4_096, 4_096, 4_096, 4_096, 4_096, 4_096, 1],
        );
        assert_eq!(
            subject.verdict(&m5),
            Verdict::Refused(CodecError::LimitExceeded),
            "{subject:?} m5 (nine segments)"
        );
    }
}

#[test]
fn body_fields() {
    let fx = fixtures();
    let y = ["Y81"];
    let z = ["Y82"];

    let rows = [
        row(
            "y-count-1025",
            &y,
            &[(0, 1, VAR_1025)],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "y-count-1024",
            &y,
            &[(0, 1, VAR_1024)],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "y-count-0",
            &y,
            &[(0, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "y-count-1",
            &y,
            &[(0, 1, &[0x01])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "y-count-3",
            &y,
            &[(0, 1, &[0x03])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "y-e1-0",
            &y,
            &[(1, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "y-e2-equal",
            &y,
            &[(7, 1, &[0x01])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "y-e2-3",
            &y,
            &[(7, 1, &[0x03])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "y-e2-3-active3",
            &y,
            &[(7, 1, &[0x03]), (11, 1, &[0x03])],
            Want::Same,
        ),
        row(
            "y-present2",
            &y,
            &[(2, 1, &[0x02])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "y-dead2",
            &y,
            &[(6, 1, &[0x02])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "y-turn2",
            &y,
            &[(9, 1, &[0x02])],
            Want::Refused(CodecError::Malformed),
        ),
        row("y-active1", &y, &[(11, 1, &[0x01])], Want::Same),
        row(
            "y-active3",
            &y,
            &[(11, 1, &[0x03])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "y-active0",
            &y,
            &[(11, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "y-health-u32ovf",
            &y,
            &[(3, 1, &[0xFF, 0xFF, 0xFF, 0xFF, 0x10])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "z-count-1025",
            &z,
            &[(0, 1, VAR_1025)],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "z-order-target3",
            &z,
            &[(38, 1, &[0x03])],
            Want::Refused(CodecError::Malformed),
        ),
        row("z-order-target1", &z, &[(38, 1, &[0x01])], Want::Same),
        row(
            "z-active3",
            &z,
            &[(51, 1, &[0x03])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "z-tl-entity3",
            &z,
            &[(59, 1, &[0x03])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "z-tl-entity-dup",
            &z,
            &[(59, 1, &[0x02])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "z-tl-key-desc",
            &z,
            &[(58, 1, &[0x0C])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "z-tl-count-1025",
            &z,
            &[(57, 1, VAR_1025)],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row("z-tl-withheld", &z, &[(56, 6, &[0x00])], Want::Same),
        row(
            "z-enc-mode2",
            &z,
            &[(53, 3, &[0x01, 0x02, 0x00, 0x00])],
            Want::Refused(CodecError::UnknownMessage),
        ),
        row(
            "z-e2-facts-pools-17",
            &z,
            &[(44, 1, &[0x01, 0x11])],
            Want::Refused(CodecError::LimitExceeded),
        ),
    ];

    for r in &rows {
        for name in &r.fixtures {
            let f = fixture(&fx, name);
            let mutated = mutate(f, r);
            let got = f.subject.verdict(&mutated);
            hostile::assert_row(name, r, &f.bytes, &mutated, &got);
        }
    }

    // Controls: the unmodified fixtures and their one-field neighbours.
    for name in ["Y81", "Y82"] {
        let f = fixture(&fx, name);
        assert_eq!(
            f.subject.verdict(&f.bytes),
            Verdict::Accepted(f.bytes.clone()),
            "control: fixture {name}"
        );
    }
}

#[test]
fn upstream_frames_sent_down_are_refused() {
    let published = TransferUp::Published {
        epoch: hostile::EPOCH,
        transfer: hostile::tid(1),
    };
    let resync = TransferUp::Resync {
        epoch: hostile::EPOCH,
        after: None,
        reason: ResyncReason::Expired,
    };
    let frames = [
        (Subject::T81, codec_transfer::encode_up(&published).unwrap()),
        (Subject::T81, codec_transfer::encode_up(&resync).unwrap()),
        (Subject::T82, transfer_v3::encode_up(&published).unwrap()),
        (Subject::T82, transfer_v3::encode_up(&resync).unwrap()),
    ];

    for (index, (subject, bytes)) in frames.iter().enumerate() {
        let expected_len = if index % 2 == 0 { 20 } else { 21 };
        assert_eq!(bytes.len(), expected_len, "{subject:?} upstream length");
        assert_eq!(
            subject.verdict(bytes),
            Verdict::Refused(CodecError::WrongDirection),
            "{subject:?} reading its own upstream frame: {:02X?}",
            bytes
        );
        for other in [Subject::T81, Subject::T82] {
            if other == *subject {
                continue;
            }
            assert_eq!(
                other.verdict(bytes),
                Verdict::Refused(CodecError::UnsupportedVersion),
                "{other:?} reading {subject:?}'s upstream frame: {:02X?}",
                bytes
            );
        }
    }

    // Control: each upstream frame's matching decode_up is Ok and canonical.
    for (index, (subject, bytes)) in frames.iter().enumerate() {
        let canonical = match subject {
            Subject::T81 => codec_transfer::encode_up(&codec_transfer::decode_up(bytes).unwrap()),
            Subject::T82 => transfer_v3::encode_up(&transfer_v3::decode_up(bytes).unwrap()),
            _ => unreachable!(),
        }
        .unwrap();
        assert_eq!(canonical, *bytes, "control: upstream frame {index}");
    }
}

#[test]
fn single_byte_corruption_is_refused_or_exact() {
    let fx = fixtures();
    for f in &fx {
        let counts =
            hostile::check_corruption(f.name, &|b| f.subject.verdict(b), &f.bytes, &f.ulid_text);
        assert!(counts.refused >= 1, "{} has no refused corruption", f.name);
        assert!(counts.exact >= 1, "{} has no exact corruption", f.name);
        if f.name == "Y82" {
            assert!(counts.alias >= 1, "{} has no ULID alias corruption", f.name);
        }
    }
    let total_bytes: usize = fx.iter().map(|f| f.bytes.len()).sum();
    assert_eq!(total_bytes * 255, 57_885, "WC12 input count");
}

/// One full WC13 seeded run, driven by `rng`; returns the input count and the
/// `Accepted` count of passes (c) and (d).
fn seeded_run(rng: &mut hostile::XorShift64) -> (usize, usize) {
    let mut accepted = 0usize;
    let mut inputs = 0usize;

    for subject in [Subject::T81, Subject::T82] {
        for _ in 0..1024 {
            inputs += 1;
            let len = 1 + (rng.next() % 64) as usize;
            let mut input = Vec::new();
            for _ in 0..len {
                input.push(rng.next() as u8);
            }
            input[0] = subject.first_byte();
            if len > 1 {
                input[1] = subject.lane();
            }
            assert!(
                hostile::stable(&|b| subject.verdict(b), &input),
                "(a) {subject:?} is not stable on {:02X?}",
                input
            );
        }
    }

    for _ in 0..1024 {
        inputs += 1;
        let len = (rng.next() % 65) as usize;
        let mut input = Vec::new();
        for _ in 0..len {
            input.push(rng.next() as u8);
        }
        for subject in SUBJECTS {
            assert!(
                hostile::stable(&|b| subject.verdict(b), &input),
                "(b) {subject:?} is not stable on {:02X?}",
                input
            );
        }
    }

    let fx = fixtures();
    for name in ["G81", "K81", "C81", "G82", "K82", "C82"] {
        let f = fixture(&fx, name);
        for _ in 0..256 {
            inputs += 1;
            let i = 3 + (rng.next() % 16) as usize;
            let v = rng.next() as u8;
            let mutated = hostile::with_byte(&f.bytes, i, v);
            assert_eq!(
                f.subject.verdict(&mutated),
                Verdict::Accepted(mutated.clone()),
                "(c) {:?} fixture {} epoch byte {i} := {v:02X}: {:02X?}",
                f.subject,
                f.name,
                mutated
            );
            accepted += 1;
        }
    }

    for f in &fx {
        let from = if f.subject.is_frame() { 2 } else { 0 };
        let span = f.bytes.len() - from;
        for _ in 0..256 {
            inputs += 1;
            let i = from + (rng.next() % span as u64) as usize;
            let v = rng.next() as u8;
            let mutated = hostile::with_byte(&f.bytes, i, v);
            assert!(
                hostile::stable(&|b| f.subject.verdict(b), &mutated),
                "(d) {:?} fixture {} byte {i} := {v:02X} is not stable: {:02X?}",
                f.subject,
                f.name,
                mutated
            );
            if matches!(f.subject.verdict(&mutated), Verdict::Accepted(_)) {
                accepted += 1;
            }
        }
    }

    (inputs, accepted)
}

#[test]
fn seeded_noise_is_stable() {
    let mut rng = hostile::XorShift64(hostile::SEED);
    assert_eq!(rng.next(), 0x4B10_DA10_A18C_50C2);
    assert_eq!(rng.next(), 0x8788_B83B_1805_38E3);
    assert_eq!(rng.next(), 0x18B5_5443_1CC3_6A52);

    let (inputs, accepted) = seeded_run(&mut rng);
    assert_eq!(inputs, 6_656, "WC13 input count");
    assert!(
        accepted >= 1_536,
        "only {accepted} accepted verdicts in (c) and (d)"
    );
}
