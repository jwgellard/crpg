//! T042b4: the hostile-server session suite (spec §16.2, T042b part 4).
//! The client-side `0x91` resume and `0xA1` movement downstream decoders fed
//! byte-exact hostile inputs. No clock, no thread, no I/O, no unseeded RNG,
//! no `HashMap`/`HashSet`, no float.

use std::ops::Range;

use crpg_net::codec::CodecError;
use crpg_net::codec_resume;
use crpg_net::movement_v4::{
    self, MoveBody, MoveDown, MoveIntent, MoveOutcome, MoveReceipt, MoveSample, SampleEntry,
};
use crpg_net::protocol_resume::{
    self, ConnectionFence, LaneClaim, LaneProgress, ResumeAccepted, ResumeDown, ResumeProof,
    ResumeRejected, ResumeRejection, ResumeRequest, ResumeResult, ResumeTicket,
};
use crpg_net::protocol_v4::Point;

#[path = "support/hostile.rs"]
mod hostile;

use hostile::{row, Verdict, Want};

// ---------------------------------------------------------------------------
// Subject table (WD1).
// ---------------------------------------------------------------------------

/// One client-side downstream decoder with its own encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Subject {
    /// `0x91` resume downstream (lane 3).
    R91,
    /// `0xA1` movement downstream (lane 1).
    A1,
}

/// Table order; every "for each subject" loop uses it.
const SUBJECTS: [Subject; 2] = [Subject::R91, Subject::A1];

impl Subject {
    /// `0x91` for `R91`, `0xA1` for `A1`.
    fn first_byte(self) -> u8 {
        match self {
            Subject::R91 => protocol_resume::RESUME_VERSION,
            Subject::A1 => movement_v4::MOVEMENT_VERSION,
        }
    }

    /// The lane byte at index 1: 3 for `R91`, 1 for `A1`.
    fn lane(self) -> u8 {
        match self {
            Subject::R91 => protocol_resume::LANE_SESSION,
            Subject::A1 => movement_v4::LANE_MOVEMENT,
        }
    }

    /// Decode with this subject's decoder; on `Ok(v)`, re-encode `v`. An
    /// encoder error on a decoded value panics with the subject and bytes.
    fn verdict(self, bytes: &[u8]) -> Verdict {
        match self {
            Subject::R91 => hostile::reencoded(
                "R91",
                bytes,
                codec_resume::decode_down(bytes),
                codec_resume::encode_down,
            ),
            Subject::A1 => hostile::reencoded(
                "A1",
                bytes,
                movement_v4::decode_down(bytes),
                movement_v4::encode_down,
            ),
        }
    }
}

/// One pinned valid frame.
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
// Pinned literals and encoder-built fixtures (WD1).
// ---------------------------------------------------------------------------

/// `T91` literal: `[0x91, 0x03, 0x30] ++ EPOCH ++ [0x10] ++ [0xA5; 16]`.
fn t_literal() -> Vec<u8> {
    hostile::cat(&[
        &[0x91, 0x03, 0x30][..],
        &hostile::EPOCH[..],
        &[0x10][..],
        &[0xA5; 16][..],
    ])
}

/// `A91` literal: `[0x91, 0x03, 0x31] ++ EPOCH ++ [0x02, 0x01, 0x00, 0x07]`.
fn a_literal() -> Vec<u8> {
    hostile::cat(&[
        &[0x91, 0x03, 0x31][..],
        &hostile::EPOCH[..],
        &[0x02, 0x01, 0x00, 0x07][..],
    ])
}

/// `J91` literal: `[0x91, 0x03, 0x32] ++ EPOCH ++ [0x02]`.
fn j_literal() -> Vec<u8> {
    hostile::cat(&[&[0x91, 0x03, 0x32][..], &hostile::EPOCH[..], &[0x02][..]])
}

/// `R1` literal: `[0xA1, 0x01, 0x00] ++ EPOCH ++
/// [0x01, 0x07, 0x29, 0x00, 0x2A]`.
fn r_literal() -> Vec<u8> {
    hostile::cat(&[
        &[0xA1, 0x01, 0x00][..],
        &hostile::EPOCH[..],
        &[0x01, 0x07, 0x29, 0x00, 0x2A][..],
    ])
}

/// `S1` literal: the sample header and its two entries.
fn s_literal() -> Vec<u8> {
    hostile::cat(&[
        &[0xA1, 0x01, 0x01, 0x05, 0x00, 0x02][..],
        &[0x07, 0x02, 0x04, 0x06, 0x00][..],
        &[0x09, 0x01, 0x00, 0x08, 0x01][..],
    ])
}

/// `T91`, built by `encode_down`.
fn encode_t() -> Vec<u8> {
    codec_resume::encode_down(&ResumeDown::Ticket(ResumeTicket {
        epoch: hostile::EPOCH,
        proof: ResumeProof::new(vec![0xA5; 16]).unwrap(),
    }))
    .unwrap()
}

/// `A91`, built by `encode_down`.
fn encode_a() -> Vec<u8> {
    codec_resume::encode_down(&ResumeDown::Answer(ResumeResult::Accepted(
        ResumeAccepted {
            epoch: hostile::EPOCH,
            granted: ConnectionFence::new(2).unwrap(),
            progress: vec![LaneProgress {
                lane: 0,
                finalized_through: 7,
            }],
        },
    )))
    .unwrap()
}

/// `J91`, built by `encode_down`.
fn encode_j() -> Vec<u8> {
    codec_resume::encode_down(&ResumeDown::Answer(ResumeResult::Rejected(
        ResumeRejected {
            epoch: hostile::EPOCH,
            reason: ResumeRejection::Busy,
        },
    )))
    .unwrap()
}

/// `R1`, built by `encode_down`.
fn encode_r() -> Vec<u8> {
    movement_v4::encode_down(&MoveDown::Receipt(MoveReceipt {
        epoch: hostile::EPOCH,
        seq: 1,
        actor: hostile::net(7),
        processed_tick: 41,
        outcome: MoveOutcome::Accepted { start_tick: 42 },
    }))
    .unwrap()
}

/// `S1`, built by `encode_down`.
fn encode_s() -> Vec<u8> {
    movement_v4::encode_down(&MoveDown::Sample(MoveSample {
        server_tick: 5,
        processed_seq: 0,
        entries: vec![
            SampleEntry {
                entity: hostile::net(7),
                point: Point::new([1, 2, 3]).unwrap(),
                moving: false,
            },
            SampleEntry {
                entity: hostile::net(9),
                point: Point::new([-1, 0, 4]).unwrap(),
                moving: true,
            },
        ],
    }))
    .unwrap()
}

/// The 5 fixtures of WD1 in table order, each built by its encoder.
fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture {
            name: "T91",
            subject: Subject::R91,
            bytes: encode_t(),
            ulid_text: vec![],
        },
        Fixture {
            name: "A91",
            subject: Subject::R91,
            bytes: encode_a(),
            ulid_text: vec![],
        },
        Fixture {
            name: "J91",
            subject: Subject::R91,
            bytes: encode_j(),
            ulid_text: vec![],
        },
        Fixture {
            name: "R1",
            subject: Subject::A1,
            bytes: encode_r(),
            ulid_text: vec![],
        },
        Fixture {
            name: "S1",
            subject: Subject::A1,
            bytes: encode_s(),
            ulid_text: vec![],
        },
    ]
}

/// The 5 fixtures' hand-written literals, in the same order.
fn pinned() -> Vec<Fixture> {
    vec![
        Fixture {
            name: "T91",
            subject: Subject::R91,
            bytes: t_literal(),
            ulid_text: vec![],
        },
        Fixture {
            name: "A91",
            subject: Subject::R91,
            bytes: a_literal(),
            ulid_text: vec![],
        },
        Fixture {
            name: "J91",
            subject: Subject::R91,
            bytes: j_literal(),
            ulid_text: vec![],
        },
        Fixture {
            name: "R1",
            subject: Subject::A1,
            bytes: r_literal(),
            ulid_text: vec![],
        },
        Fixture {
            name: "S1",
            subject: Subject::A1,
            bytes: s_literal(),
            ulid_text: vec![],
        },
    ]
}

/// Pinned varint byte sequences (the "Row notation" of the contract).
const U64MAX: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];
const U64LAST: &[u8] = &[0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];

/// `2^29` as a zigzag `i32` varint.
const ZIG_POS_LIMIT: &[u8] = &[0x80, 0x80, 0x80, 0x80, 0x04];
/// `2^29 + 1` as a zigzag `i32` varint.
const ZIG_POS_OVER: &[u8] = &[0x82, 0x80, 0x80, 0x80, 0x04];
/// `-2^29` as a zigzag `i32` varint.
const ZIG_NEG_LIMIT: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF, 0x03];
/// `-(2^29 + 1)` as a zigzag `i32` varint.
const ZIG_NEG_OVER: &[u8] = &[0x81, 0x80, 0x80, 0x80, 0x04];

// ---------------------------------------------------------------------------
// HD1..HD13
// ---------------------------------------------------------------------------

#[test]
fn fixtures_are_pinned() {
    let fx = fixtures();
    let literal = pinned();
    assert_eq!(fx.len(), 5, "fixture count");
    assert_eq!(literal.len(), 5, "literal count");

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

    assert_eq!(Subject::R91.first_byte(), protocol_resume::RESUME_VERSION);
    assert_eq!(Subject::R91.first_byte(), 0x91);
    assert_eq!(Subject::R91.lane(), protocol_resume::LANE_SESSION);
    assert_eq!(Subject::R91.lane(), 3);
    assert_eq!(Subject::A1.first_byte(), movement_v4::MOVEMENT_VERSION);
    assert_eq!(Subject::A1.first_byte(), 0xA1);
    assert_eq!(Subject::A1.lane(), movement_v4::LANE_MOVEMENT);
    assert_eq!(Subject::A1.lane(), 1);

    let t91 = fixture(&fx, "T91");
    assert_ne!(
        hostile::with_byte(&t91.bytes, 19, 0x11),
        t91.bytes,
        "control: T91 with proof length 17 is not T91's encoding"
    );
}

#[test]
fn short_inputs_are_refused_exactly() {
    for subject in SUBJECTS {
        assert_eq!(
            subject.verdict(&[]),
            Verdict::Refused(CodecError::Malformed),
            "{:?} empty input",
            subject
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
                "{:?} one-byte input {:02X?}",
                subject,
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
                    "{:?} two-byte input {:02X?}",
                    subject,
                    [b, c]
                );
            }
        }
        assert_eq!(
            subject.verdict(&[subject.first_byte()]),
            Verdict::Refused(CodecError::Malformed),
            "{:?} first byte alone",
            subject
        );
        assert_eq!(
            subject.verdict(&[subject.first_byte(), subject.lane()]),
            Verdict::Refused(CodecError::Malformed),
            "{:?} header alone",
            subject
        );
        assert_eq!(
            subject.verdict(&[subject.first_byte(), subject.lane().wrapping_add(1)]),
            Verdict::Refused(CodecError::WrongDirection),
            "{:?} header with a foreign lane",
            subject
        );
    }
}

#[test]
fn oversize_is_refused_first() {
    assert_eq!(protocol_resume::MAX_RESUME_FRAME_BYTES, 4096);
    assert_eq!(movement_v4::MAX_MOVE_DOWN_BYTES, 1024);

    assert_eq!(
        Subject::R91.verdict(&[0xFF; 4097]),
        Verdict::Refused(CodecError::FrameTooLarge),
        "R91 4097 bytes"
    );
    assert_eq!(
        Subject::R91.verdict(&[0xFF; 4096]),
        Verdict::Refused(CodecError::UnsupportedVersion),
        "R91 4096 bytes"
    );
    assert_eq!(
        Subject::A1.verdict(&[0xFF; 1025]),
        Verdict::Refused(CodecError::FrameTooLarge),
        "A1 1025 bytes"
    );
    assert_eq!(
        Subject::A1.verdict(&[0xFF; 1024]),
        Verdict::Refused(CodecError::UnsupportedVersion),
        "A1 1024 bytes"
    );

    let fx = fixtures();
    for f in &fx {
        let cap = match f.subject {
            Subject::R91 => 4096,
            Subject::A1 => 1024,
        };
        assert_eq!(
            f.subject.verdict(&hostile::padded(&f.bytes, cap)),
            Verdict::Refused(CodecError::Malformed),
            "{:?} fixture {} padded to {}: {:02X?}",
            f.subject,
            f.name,
            cap,
            f.bytes
        );
        assert_eq!(
            f.subject.verdict(&hostile::padded(&f.bytes, cap + 1)),
            Verdict::Refused(CodecError::FrameTooLarge),
            "{:?} fixture {} padded to {}: {:02X?}",
            f.subject,
            f.name,
            cap + 1,
            f.bytes
        );
    }
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
    assert_eq!(count, 119);
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
                "{:?} fixture {} plus {:02X?}: {:02X?}",
                f.subject,
                f.name,
                extra,
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
    assert_eq!(count, 15);
}

#[test]
fn foreign_first_bytes_are_unsupported() {
    let fx = fixtures();
    let mut cross = 0usize;
    for f in &fx {
        for subject in SUBJECTS {
            if subject == f.subject {
                continue;
            }
            assert_eq!(
                subject.verdict(&f.bytes),
                Verdict::Refused(CodecError::UnsupportedVersion),
                "{:?} reading fixture {} of {:?}: {:02X?}",
                subject,
                f.name,
                f.subject,
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
    assert_eq!(cross, 5);
}

#[test]
fn foreign_lanes_are_wrong_direction() {
    let fx = fixtures();
    for f in &fx {
        for v in 0..=255u8 {
            if v == f.subject.lane() {
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
                .verdict(&hostile::with_byte(&f.bytes, 1, f.subject.lane())),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} with its own lane byte",
            f.subject,
            f.name
        );
    }
}

/// WD8's verdict for the tag or kind byte `t` at index 2.
fn tag_expected(name: &str, t: u8, mutated: &[u8]) -> Verdict {
    match name {
        "T91" => match t {
            0x30 => Verdict::Accepted(mutated.to_vec()),
            0x31 => Verdict::Refused(CodecError::LimitExceeded),
            0x40 => Verdict::Refused(CodecError::WrongDirection),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        "A91" => match t {
            0x30 | 0x32 => Verdict::Refused(CodecError::Malformed),
            0x31 => Verdict::Accepted(mutated.to_vec()),
            0x40 => Verdict::Refused(CodecError::WrongDirection),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        "J91" => match t {
            0x30 | 0x31 => Verdict::Refused(CodecError::Malformed),
            0x32 => Verdict::Accepted(mutated.to_vec()),
            0x40 => Verdict::Refused(CodecError::WrongDirection),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        "R1" => match t {
            0 => Verdict::Accepted(mutated.to_vec()),
            1 => Verdict::Refused(CodecError::Malformed),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        "S1" => match t {
            0 => Verdict::Refused(CodecError::Malformed),
            1 => Verdict::Accepted(mutated.to_vec()),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        _ => panic!("no WD8 tag row for {name}"),
    }
}

#[test]
fn tag_and_kind_matrix() {
    let fx = fixtures();
    let mut inputs = 0usize;
    for name in ["T91", "A91", "J91", "R1", "S1"] {
        let f = fixture(&fx, name);
        for t in 0..=255u8 {
            let input = hostile::with_byte(&f.bytes, 2, t);
            let expected = tag_expected(name, t, &input);
            assert_eq!(
                f.subject.verdict(&input),
                expected,
                "{:?} fixture {} with tag/kind {t:02X}: {:02X?}",
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
            "control: {:?} fixture {} with its own tag/kind",
            f.subject,
            f.name
        );
    }
    assert_eq!(inputs, 1280);
}

#[test]
fn resume_fields() {
    let fx = fixtures();
    let rows = [
        row(
            "t-len-15",
            &["T91"],
            &[(19, 1, &[0x0F])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "t-len-0",
            &["T91"],
            &[(19, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "t-len-17",
            &["T91"],
            &[(19, 1, &[0x11])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "t-len-128",
            &["T91"],
            &[(19, 1, &[0x80])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "t-len-129",
            &["T91"],
            &[(19, 1, &[0x81])],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "t-len-255",
            &["T91"],
            &[(19, 1, &[0xFF])],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "t-proof-17",
            &["T91"],
            &[(19, 1, &[0x11]), (36, 0, &[0xA5])],
            Want::Same,
        ),
        row(
            "t-proof-128",
            &["T91"],
            &[(19, 1, &[0x80]), (36, 0, &[0xA5; 112])],
            Want::Same,
        ),
        row(
            "t-proof-129",
            &["T91"],
            &[(19, 1, &[0x81]), (36, 0, &[0xA5; 113])],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "a-granted-0",
            &["A91"],
            &[(19, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "a-granted-1",
            &["A91"],
            &[(19, 1, &[0x01])],
            Want::Refused(CodecError::Malformed),
        ),
        row("a-granted-3", &["A91"], &[(19, 1, &[0x03])], Want::Same),
        row("a-granted-u64max", &["A91"], &[(19, 1, U64MAX)], Want::Same),
        row(
            "a-count-0",
            &["A91"],
            &[(20, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "a-count-2",
            &["A91"],
            &[(20, 1, &[0x02])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "a-count-4",
            &["A91"],
            &[(20, 1, &[0x04])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "a-count-5",
            &["A91"],
            &[(20, 1, &[0x05])],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "a-lane-1",
            &["A91"],
            &[(21, 1, &[0x01])],
            Want::Refused(CodecError::UnknownMessage),
        ),
        row(
            "a-lane-255",
            &["A91"],
            &[(21, 1, &[0xFF])],
            Want::Refused(CodecError::UnknownMessage),
        ),
        row("a-final-last", &["A91"], &[(22, 1, U64LAST)], Want::Same),
        row(
            "a-final-u64max",
            &["A91"],
            &[(22, 1, U64MAX)],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "a-two-lanes",
            &["A91"],
            &[(20, 1, &[0x02]), (23, 0, &[0x00, 0x08])],
            Want::Refused(CodecError::Malformed),
        ),
        row("j-reason-1", &["J91"], &[(19, 1, &[0x01])], Want::Same),
        row(
            "j-reason-0",
            &["J91"],
            &[(19, 1, &[0x00])],
            Want::Refused(CodecError::UnknownMessage),
        ),
        row(
            "j-reason-3",
            &["J91"],
            &[(19, 1, &[0x03])],
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

    // Sweep: the Rejected reason (index 19).
    let j91 = fixture(&fx, "J91");
    for t in 0..=255u8 {
        let input = hostile::with_byte(&j91.bytes, 19, t);
        let expected = match t {
            1 | 2 => Verdict::Accepted(input.clone()),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        };
        assert_eq!(
            Subject::R91.verdict(&input),
            expected,
            "J91 reason {t:02X}: {:02X?}",
            input
        );
    }

    // Sweep: the Accepted `granted` fence (index 19).
    let a91 = fixture(&fx, "A91");
    for t in 0..=255u8 {
        let input = hostile::with_byte(&a91.bytes, 19, t);
        let expected = match t {
            0 | 1 => Verdict::Refused(CodecError::Malformed),
            2..=127 => Verdict::Accepted(input.clone()),
            _ => Verdict::Refused(CodecError::Malformed),
        };
        assert_eq!(
            Subject::R91.verdict(&input),
            expected,
            "A91 granted {t:02X}: {:02X?}",
            input
        );
    }

    // Sweep: the lane list's lane (index 21).
    for t in 0..=255u8 {
        let input = hostile::with_byte(&a91.bytes, 21, t);
        let expected = match t {
            0 => Verdict::Accepted(input.clone()),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        };
        assert_eq!(
            Subject::R91.verdict(&input),
            expected,
            "A91 lane {t:02X}: {:02X?}",
            input
        );
    }
}

#[test]
fn movement_fields() {
    let fx = fixtures();
    let rows = [
        row(
            "r-seq-0",
            &["R1"],
            &[(19, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "r-seq-u64max",
            &["R1"],
            &[(19, 1, U64MAX)],
            Want::Refused(CodecError::Malformed),
        ),
        row("r-seq-last", &["R1"], &[(19, 1, U64LAST)], Want::Same),
        row(
            "r-actor-0",
            &["R1"],
            &[(20, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "r-outcome-superseded",
            &["R1"],
            &[(22, 1, &[0x01])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "r-outcome-rejected",
            &["R1"],
            &[(22, 1, &[0x02])],
            Want::Refused(CodecError::UnknownMessage),
        ),
        row(
            "r-outcome-3",
            &["R1"],
            &[(22, 1, &[0x03])],
            Want::Refused(CodecError::UnknownMessage),
        ),
        row(
            "r-rejected-17",
            &["R1"],
            &[(22, 2, &[0x02, 0x11])],
            Want::Same,
        ),
        row(
            "r-rejected-30",
            &["R1"],
            &[(22, 2, &[0x02, 0x1E])],
            Want::Same,
        ),
        row(
            "r-rejected-31",
            &["R1"],
            &[(22, 2, &[0x02, 0x1F])],
            Want::Refused(CodecError::UnknownMessage),
        ),
        row(
            "r-rejected-0",
            &["R1"],
            &[(22, 2, &[0x02, 0x00])],
            Want::Refused(CodecError::UnknownMessage),
        ),
        row("r-superseded", &["R1"], &[(22, 2, &[0x01])], Want::Same),
        row(
            "s-count-39",
            &["S1"],
            &[(5, 1, &[0x27])],
            Want::Refused(CodecError::LimitExceeded),
        ),
        row(
            "s-count-38",
            &["S1"],
            &[(5, 1, &[0x26])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "s-count-0",
            &["S1"],
            &[(5, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "s-count-1",
            &["S1"],
            &[(5, 1, &[0x01])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "s-entity1-0",
            &["S1"],
            &[(6, 1, &[0x00])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "s-entity2-equal",
            &["S1"],
            &[(11, 1, &[0x07])],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "s-entity2-less",
            &["S1"],
            &[(11, 1, &[0x06])],
            Want::Refused(CodecError::Malformed),
        ),
        row("s-entity2-10", &["S1"], &[(11, 1, &[0x0A])], Want::Same),
        row(
            "s-pseq-u64max",
            &["S1"],
            &[(4, 1, U64MAX)],
            Want::Refused(CodecError::Malformed),
        ),
        row("s-pseq-last", &["S1"], &[(4, 1, U64LAST)], Want::Same),
        row("s-tick-u64max", &["S1"], &[(3, 1, U64MAX)], Want::Same),
        row("s-x-limit", &["S1"], &[(7, 1, ZIG_POS_LIMIT)], Want::Same),
        row(
            "s-x-over",
            &["S1"],
            &[(7, 1, ZIG_POS_OVER)],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "s-y-neglimit",
            &["S1"],
            &[(8, 1, ZIG_NEG_LIMIT)],
            Want::Same,
        ),
        row(
            "s-y-negover",
            &["S1"],
            &[(8, 1, ZIG_NEG_OVER)],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "s-z-over",
            &["S1"],
            &[(9, 1, ZIG_POS_OVER)],
            Want::Refused(CodecError::Malformed),
        ),
        row(
            "s-moving-2",
            &["S1"],
            &[(10, 1, &[0x02])],
            Want::Refused(CodecError::Malformed),
        ),
        row("s-empty", &["S1"], &[(5, 11, &[0x00])], Want::Same),
    ];

    for r in &rows {
        for name in &r.fixtures {
            let f = fixture(&fx, name);
            let mutated = mutate(f, r);
            let got = f.subject.verdict(&mutated);
            hostile::assert_row(name, r, &f.bytes, &mutated, &got);
        }
    }

    // Sweep: the receipt's outcome tag (index 22).
    let r1 = fixture(&fx, "R1");
    for t in 0..=255u8 {
        let input = hostile::with_byte(&r1.bytes, 22, t);
        let expected = match t {
            0 => Verdict::Accepted(input.clone()),
            1 => Verdict::Refused(CodecError::Malformed),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        };
        assert_eq!(
            Subject::A1.verdict(&input),
            expected,
            "R1 outcome tag {t:02X}: {:02X?}",
            input
        );
    }

    // Sweep: the sample's entry count (index 5).
    let s1 = fixture(&fx, "S1");
    for t in 0..=255u8 {
        let input = hostile::with_byte(&s1.bytes, 5, t);
        let expected = match t {
            0 | 1 => Verdict::Refused(CodecError::Malformed),
            2 => Verdict::Accepted(input.clone()),
            3..=38 => Verdict::Refused(CodecError::Malformed),
            _ => Verdict::Refused(CodecError::LimitExceeded),
        };
        assert_eq!(
            Subject::A1.verdict(&input),
            expected,
            "S1 count {t:02X}: {:02X?}",
            input
        );
    }
}

#[test]
fn upstream_frames_sent_down_are_refused() {
    let q91 = codec_resume::encode_request(&ResumeRequest {
        epoch: hostile::EPOCH,
        fence: ConnectionFence::FIRST,
        proof: ResumeProof::new(vec![0xA5; 16]).unwrap(),
        claims: vec![LaneClaim {
            lane: 0,
            receipted_through: 0,
        }],
    })
    .unwrap();
    let mt = movement_v4::encode_intent(&MoveIntent {
        epoch: hostile::EPOCH,
        seq: 1,
        actor: hostile::net(7),
        body: MoveBody::MoveTo {
            destination: Point::new([1, 2, 3]).unwrap(),
        },
    })
    .unwrap();
    let mh = movement_v4::encode_intent(&MoveIntent {
        epoch: hostile::EPOCH,
        seq: 1,
        actor: hostile::net(7),
        body: MoveBody::Halt,
    })
    .unwrap();

    assert_eq!(
        q91,
        hostile::cat(&[
            &[0x91, 0x03, 0x40][..],
            &hostile::EPOCH[..],
            &[0x01, 0x10][..],
            &[0xA5; 16][..],
            &[0x01, 0x00, 0x00][..],
        ]),
        "Q91 literal"
    );
    assert_eq!(
        mt,
        hostile::cat(&[
            &[0xA1][..],
            &hostile::EPOCH[..],
            &[0x01, 0x01, 0x07, 0x00, 0x02, 0x04, 0x06][..],
        ]),
        "MT literal"
    );
    assert_eq!(
        mh,
        hostile::cat(&[
            &[0xA1][..],
            &hostile::EPOCH[..],
            &[0x01, 0x01, 0x07, 0x01][..],
        ]),
        "MH literal"
    );
    assert_eq!(q91.len(), 40, "Q91 length");
    assert_eq!(mt.len(), 24, "MT length");
    assert_eq!(mh.len(), 21, "MH length");

    assert_eq!(
        Subject::R91.verdict(&q91),
        Verdict::Refused(CodecError::WrongDirection),
        "R91 reading its own upstream request: {:02X?}",
        q91
    );
    assert_eq!(
        Subject::A1.verdict(&mt),
        Verdict::Refused(CodecError::UnknownMessage),
        "A1 reading MT: {:02X?}",
        mt
    );
    assert_eq!(
        Subject::A1.verdict(&mh),
        Verdict::Refused(CodecError::UnknownMessage),
        "A1 reading MH: {:02X?}",
        mh
    );
    assert_eq!(
        Subject::A1.verdict(&q91),
        Verdict::Refused(CodecError::UnsupportedVersion),
        "A1 reading Q91: {:02X?}",
        q91
    );
    assert_eq!(
        Subject::R91.verdict(&mt),
        Verdict::Refused(CodecError::UnsupportedVersion),
        "R91 reading MT: {:02X?}",
        mt
    );
    assert_eq!(
        Subject::R91.verdict(&mh),
        Verdict::Refused(CodecError::UnsupportedVersion),
        "R91 reading MH: {:02X?}",
        mh
    );

    // Control: each upstream frame through its matching upstream decoder.
    assert_eq!(
        codec_resume::encode_request(&codec_resume::decode_request(&q91).unwrap()).unwrap(),
        q91
    );
    assert_eq!(
        movement_v4::encode_intent(&movement_v4::decode_intent(&mt).unwrap()).unwrap(),
        mt
    );
    assert_eq!(
        movement_v4::encode_intent(&movement_v4::decode_intent(&mh).unwrap()).unwrap(),
        mh
    );
}

#[test]
fn single_byte_corruption_is_refused_or_exact() {
    let fx = fixtures();
    for f in &fx {
        let counts =
            hostile::check_corruption(f.name, &|b| f.subject.verdict(b), &f.bytes, &f.ulid_text);
        assert!(counts.refused >= 1, "{} has no refused corruption", f.name);
        assert!(counts.exact >= 1, "{} has no accepted corruption", f.name);
    }
    let total_bytes: usize = fx.iter().map(|f| f.bytes.len()).sum();
    assert_eq!(total_bytes, 119, "WD12 byte count");
    assert_eq!(total_bytes * 255, 30_345, "WD12 input count");
}

/// One full WD13 run, driven by `rng`; returns the verdict sequence, the
/// `Accepted` count of passes (c) and (d), and the input count.
fn seeded_run(rng: &mut hostile::XorShift64) -> (Vec<Verdict>, usize, usize) {
    let mut verdicts = Vec::new();
    let mut accepted = 0usize;
    let mut inputs = 0usize;

    for subject in SUBJECTS {
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
                "(a) {:?} is not stable on {:02X?}",
                subject,
                input
            );
            verdicts.push(subject.verdict(&input));
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
                "(b) {:?} is not stable on {:02X?}",
                subject,
                input
            );
            verdicts.push(subject.verdict(&input));
        }
    }

    let fx = fixtures();
    for name in ["T91", "A91", "J91", "R1"] {
        let f = fixture(&fx, name);
        for _ in 0..256 {
            inputs += 1;
            let i = 3 + (rng.next() % 16) as usize;
            let v = rng.next() as u8;
            let mutated = hostile::with_byte(&f.bytes, i, v);
            let verdict = f.subject.verdict(&mutated);
            assert_eq!(
                verdict,
                Verdict::Accepted(mutated.clone()),
                "(c) {:?} fixture {} epoch byte {i} := {v:02X}: {:02X?}",
                f.subject,
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
            let span = f.bytes.len() - 2;
            let i = 2 + (rng.next() % span as u64) as usize;
            let v = rng.next() as u8;
            let mutated = hostile::with_byte(&f.bytes, i, v);
            assert!(
                hostile::stable(&|b| f.subject.verdict(b), &mutated),
                "(d) {:?} fixture {} byte {i} := {v:02X} is not stable: {:02X?}",
                f.subject,
                f.name,
                mutated
            );
            let verdict = f.subject.verdict(&mutated);
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
    assert_eq!(inputs, 5376, "WD13 input count");
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
