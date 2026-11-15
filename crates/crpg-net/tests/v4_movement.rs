//! T026p2 protocol v4 lane-1 acceptance: movement intents, receipts, position
//! samples, their codec, the sample splitter and the policy constants
//! (`tasks/T026p.md`, tests M1..=M8).
//!
//! Public API only. Every expectation is a hand-written literal byte or value
//! oracle built from the contract's layout (postcard primitives: raw `u8` and
//! `bool`, LEB128 varints, zigzag `i32`), never from the codec under test.
//! Every bad-byte case carries a complete valid remainder and a control with
//! the good byte that decodes `Ok`, so truncation cannot mask a refusal.

use crpg_net::codec::{
    decode_delta as decode_delta_v1, decode_intent as decode_intent_v1, CodecError,
};
use crpg_net::codec_resume;
use crpg_net::codec_transfer;
use crpg_net::codec_v2::{decode_delta as decode_delta_v2, decode_intent as decode_intent_v2};
use crpg_net::codec_v3::{decode_delta as decode_delta_v3, decode_intent as decode_intent_v3};
use crpg_net::codec_v4;
use crpg_net::movement_v4::{
    check_sample, decode_down, decode_intent, encode_down, encode_intent, sample_datagrams,
    MoveBody, MoveDown, MoveIntent, MoveOutcome, MoveReceipt, MoveSample, SampleEntry,
    DOWN_KIND_RECEIPT, DOWN_KIND_SAMPLE, LANE_MOVEMENT, MAX_MOVE_DOWN_BYTES, MAX_SAMPLE_ENTRIES,
    MAX_SAMPLE_ENTRY_BYTES, MAX_SAMPLE_HEADER_BYTES, MOVEMENT_VERSION, MOVE_TAG_HALT,
    MOVE_TAG_MOVE_TO, OUTCOME_TAG_ACCEPTED, OUTCOME_TAG_REJECTED, OUTCOME_TAG_SUPERSEDED,
    POLICY_MOVE_INTENTS_PER_SEC, POLICY_MOVE_INTENT_BURST, POLICY_MOVE_IN_FLIGHT,
};
use crpg_net::protocol::{LANE_COMBAT, MAX_INTENT_FRAME_BYTES, SEQ_LAST};
use crpg_net::protocol_resume::LANE_SESSION;
use crpg_net::protocol_transfer::LANE_SNAPSHOT;
use crpg_net::protocol_v4::{NetId, Point, RejectionCode};

// ---------------------------------------------------------------------------
// Fixtures and byte helpers.
// ---------------------------------------------------------------------------

const Z16: [u8; 16] = [0; 16];
/// `L = 2^29`.
const L: i32 = 1 << 29;
/// `SEQ_LAST` as a varint: `FE`, seven `FF`, `01`.
const SEQ_LAST_VARINT: [u8; 10] = [0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];
/// `u64::MAX` as a varint: nine `FF`, `01`.
const U64_MAX_VARINT: [u8; 10] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];
/// `L` as a coordinate: `80 80 80 80 04`.
const COORD_L: [u8; 5] = [0x80, 0x80, 0x80, 0x80, 0x04];
/// `L + 1` as a coordinate.
const COORD_L_PLUS_1: [u8; 5] = [0x82, 0x80, 0x80, 0x80, 0x04];
/// `-L - 1` as a coordinate.
const COORD_MINUS_L_MINUS_1: [u8; 5] = [0x81, 0x80, 0x80, 0x80, 0x04];
/// `i32::MIN` as a coordinate.
const COORD_I32_MIN: [u8; 5] = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
/// A varint that overflows `i32`.
const COORD_OVERFLOW: [u8; 5] = [0xFF, 0xFF, 0xFF, 0xFF, 0x1F];

fn net(raw: u64) -> NetId {
    NetId::new(raw).expect("fixture id is nonzero")
}

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

/// The wire bytes of `B`: `80 80 08`, `00`, `FF FF 0F`.
const B_BYTES: [u8; 7] = [0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F];
/// The wire bytes of `A`.
const A_BYTES: [u8; 3] = [0, 0, 0];

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

/// The wire bytes of a point, from the zigzag oracle.
fn pt(point: Point) -> Vec<u8> {
    let [x, y, z] = point.raw();
    cat(&[&zz(x), &zz(y), &zz(z)])
}

/// `A1 Z16 <lane> <seq> <actor> <tail>`.
fn intent_hdr(version: u8, lane: u8, seq: &[u8], actor: &[u8], tail: &[u8]) -> Vec<u8> {
    cat(&[&[version], &Z16, &[lane], seq, actor, tail])
}

/// `A1 Z16 01 01 01 <tail>` (the defaults: `seq` 1, `actor` 1).
fn ibytes(tail: &[u8]) -> Vec<u8> {
    intent_hdr(0xA1, 1, &[1], &[1], tail)
}

fn halt_bytes() -> Vec<u8> {
    ibytes(&[0x01])
}

fn move_to_bytes(body: &[u8]) -> Vec<u8> {
    cat(&[&ibytes(&[0x00]), body])
}

fn intent(body: MoveBody) -> MoveIntent {
    MoveIntent {
        epoch: Z16,
        seq: 1,
        actor: net(1),
        body,
    }
}

fn move_to(destination: Point) -> MoveBody {
    MoveBody::MoveTo { destination }
}

/// `A1 01 00 Z16 <seq> <actor> <processed_tick = 100>` then `tail`.
fn receipt_bytes(seq: &[u8], actor: &[u8], tail: &[u8]) -> Vec<u8> {
    cat(&[&[0xA1, 0x01, 0x00], &Z16, seq, actor, &[0x64], tail])
}

fn receipt(outcome: MoveOutcome) -> MoveReceipt {
    MoveReceipt {
        epoch: Z16,
        seq: 1,
        actor: net(1),
        processed_tick: 100,
        outcome,
    }
}

fn entry(entity: u64, point: Point, moving: bool) -> SampleEntry {
    SampleEntry {
        entity: net(entity),
        point,
        moving,
    }
}

/// `{ id(1), B, true }`, `{ id(2), A, false }`.
fn s1_entries() -> Vec<SampleEntry> {
    vec![entry(1, pb(), true), entry(2, pa(), false)]
}

/// `S1 = { 100, 1, [{ id(1), B, true }, { id(2), A, false }] }`.
fn s1() -> MoveSample {
    MoveSample {
        server_tick: 100,
        processed_seq: 1,
        entries: s1_entries(),
    }
}

/// The 20 bytes of `S1`.
fn s1_bytes() -> Vec<u8> {
    cat(&[
        &[0xA1, 0x01, 0x01, 0x64, 0x01, 0x02],
        &[0x01],
        &B_BYTES,
        &[0x01],
        &[0x02],
        &A_BYTES,
        &[0x00],
    ])
}

/// `A1 01 01 64 <processed_seq> <count>` then `entries`.
fn sample_bytes(processed_seq: &[u8], count: &[u8], entries: &[u8]) -> Vec<u8> {
    cat(&[&[0xA1, 0x01, 0x01, 0x64], processed_seq, count, entries])
}

/// One sample entry's bytes: `<entity> <point> <moving>`.
fn entry_bytes(entity: u64, point: &[u8], moving: u8) -> Vec<u8> {
    cat(&[&varint(entity), point, &[moving]])
}

/// A valid `{ id(9), A, false }` second entry for bad-first-entry cases.
fn valid_entry_9() -> Vec<u8> {
    entry_bytes(9, &A_BYTES, 0)
}

/// `n` ascending maxima entries ending at `u64::MAX`.
fn max_entries(n: u64) -> Vec<SampleEntry> {
    (0..n)
        .map(|i| entry(u64::MAX - (n - 1) + i, p(L, L, L), true))
        .collect()
}

/// Replaces byte 0 of `bytes`.
fn with_first(bytes: &[u8], first: u8) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out[0] = first;
    out
}

/// `bytes` padded with `00` to exactly `len`.
fn padded(bytes: &[u8], len: usize) -> Vec<u8> {
    let mut out = bytes.to_vec();
    assert!(out.len() <= len);
    out.resize(len, 0);
    out
}

fn dec_intent_err(bytes: &[u8]) -> CodecError {
    decode_intent(bytes).expect_err("intent must be refused")
}

fn dec_down_err(bytes: &[u8]) -> CodecError {
    decode_down(bytes).expect_err("downstream message must be refused")
}

fn receipt_msg(outcome: MoveOutcome) -> MoveDown {
    MoveDown::Receipt(receipt(outcome))
}

// ---------------------------------------------------------------------------
// M1.
// ---------------------------------------------------------------------------

/// M1: `MoveTo`, `Halt` and the largest intent, byte for byte, in both
/// directions; no `observed_tick`.
#[test]
fn move_intent_literals() {
    // The point oracle agrees with the contract's literal bytes.
    assert_eq!(pt(pb()), B_BYTES);
    assert_eq!(pt(p(L, L, L)), cat(&[&COORD_L, &COORD_L, &COORD_L]));
    assert_eq!(zz(-L), [0xFF, 0xFF, 0xFF, 0xFF, 0x03]);
    let move_to_b = intent(move_to(pb()));
    let expected = cat(&[
        &[0xA1],
        &Z16,
        &[0x01, 0x01, 0x01, 0x00],
        &[0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F],
    ]);
    assert_eq!(expected.len(), 28);
    assert_eq!(encode_intent(&move_to_b), Ok(expected.clone()));
    assert_eq!(decode_intent(&expected), Ok(move_to_b));

    let halt = intent(MoveBody::Halt);
    let expected = cat(&[&[0xA1], &Z16, &[0x01, 0x01, 0x01, 0x01]]);
    assert_eq!(expected.len(), 21);
    assert_eq!(encode_intent(&halt), Ok(expected.clone()));
    assert_eq!(decode_intent(&expected), Ok(halt));

    let largest = MoveIntent {
        epoch: Z16,
        seq: SEQ_LAST,
        actor: net(u64::MAX),
        body: move_to(p(L, L, L)),
    };
    let expected = cat(&[
        &[0xA1],
        &Z16,
        &[0x01],
        &SEQ_LAST_VARINT,
        &U64_MAX_VARINT,
        &[0x00],
        &COORD_L,
        &COORD_L,
        &COORD_L,
    ]);
    assert_eq!(expected.len(), 54);
    assert_eq!(encode_intent(&largest), Ok(expected.clone()));
    assert_eq!(decode_intent(&expected), Ok(largest));
}

/// M1: a non-zero epoch and negative coordinates round-trip, and the encoder
/// is deterministic (M-8 identity).
#[test]
fn move_intent_identity() {
    let intents = [
        MoveIntent {
            epoch: [7; 16],
            seq: 5,
            actor: net(300),
            body: move_to(p(-L, 0, L)),
        },
        MoveIntent {
            epoch: [0xFF; 16],
            seq: SEQ_LAST,
            actor: net(u64::MAX),
            body: MoveBody::Halt,
        },
        intent(move_to(pa())),
    ];
    for original in intents {
        let first = encode_intent(&original).expect("encodes");
        assert_eq!(encode_intent(&original).as_ref(), Ok(&first));
        assert_eq!(decode_intent(&first), Ok(original));
    }
    // The epoch and every field sit where the layout puts them.
    let bytes = encode_intent(&MoveIntent {
        epoch: [7; 16],
        seq: 5,
        actor: net(300),
        body: MoveBody::Halt,
    })
    .expect("encodes");
    assert_eq!(
        bytes,
        cat(&[&[0xA1], &[7; 16], &[0x01, 0x05], &[0xAC, 0x02], &[0x01]])
    );
}

// ---------------------------------------------------------------------------
// M2.
// ---------------------------------------------------------------------------

/// M2: version, cap, lane, range and tag refusals, with controls.
#[test]
fn move_intent_refusals() {
    // Version: every other first byte before a valid Halt remainder.
    let halt = halt_bytes();
    assert!(decode_intent(&halt).is_ok());
    for first in [0x01, 0x02, 0x03, 0x04, 0x81, 0x82, 0x91] {
        assert_eq!(
            dec_intent_err(&with_first(&halt, first)),
            CodecError::UnsupportedVersion,
            "first byte {first:#04x}"
        );
    }
    // Empty and the cap.
    assert_eq!(dec_intent_err(&[]), CodecError::Malformed);
    let mut big = vec![0x02];
    big.resize(MAX_INTENT_FRAME_BYTES + 1, 0);
    assert_eq!(big.len(), 4_097);
    assert_eq!(dec_intent_err(&big), CodecError::FrameTooLarge);
    assert_eq!(
        dec_intent_err(&padded(&halt, MAX_INTENT_FRAME_BYTES)),
        CodecError::Malformed
    );
    // Lane 00 before a valid remainder; control lane 01 is `halt` above.
    assert_eq!(
        dec_intent_err(&intent_hdr(0xA1, 0, &[1], &[1], &[0x01])),
        CodecError::WrongDirection
    );
    for lane in [2u8, 3, 0xFF] {
        assert_eq!(
            dec_intent_err(&intent_hdr(0xA1, lane, &[1], &[1], &[0x01])),
            CodecError::WrongDirection
        );
    }
    // The v4 lane-0 decoder and this decoder refuse each other's bytes.
    assert_eq!(
        codec_v4::decode_intent(&halt),
        Err(CodecError::UnsupportedVersion)
    );
    let v4_end_turn = cat(&[&[0x04], &Z16, &[0x00, 0x01, 0x07, 0x01, 0x01]]);
    assert!(codec_v4::decode_intent(&v4_end_turn).is_ok());
    assert_eq!(dec_intent_err(&v4_end_turn), CodecError::UnsupportedVersion);
    // `seq` 0 and `u64::MAX` are Malformed; `SEQ_LAST` is accepted.
    assert_eq!(
        dec_intent_err(&intent_hdr(0xA1, 1, &[0], &[1], &[0x01])),
        CodecError::Malformed
    );
    assert_eq!(
        dec_intent_err(&intent_hdr(0xA1, 1, &U64_MAX_VARINT, &[1], &[0x01])),
        CodecError::Malformed
    );
    let last = decode_intent(&intent_hdr(0xA1, 1, &SEQ_LAST_VARINT, &[1], &[0x01]))
        .expect("SEQ_LAST is accepted");
    assert_eq!(last.seq, SEQ_LAST);
    // `actor` 0.
    assert_eq!(
        dec_intent_err(&intent_hdr(0xA1, 1, &[1], &[0], &[0x01])),
        CodecError::Malformed
    );
    // Tags.
    assert_eq!(dec_intent_err(&ibytes(&[0x02])), CodecError::UnknownMessage);
    assert_eq!(dec_intent_err(&ibytes(&[0xFF])), CodecError::UnknownMessage);
    // Malformed shapes.
    assert_eq!(
        dec_intent_err(&cat(&[&halt, &[0x00]])),
        CodecError::Malformed
    );
    assert_eq!(
        dec_intent_err(&cat(&[&move_to_bytes(&[0x00, 0x00]), &[]])),
        CodecError::Malformed,
        "MoveTo missing z"
    );
    let full = move_to_bytes(&cat(&[&B_BYTES]));
    assert!(decode_intent(&full).is_ok());
    assert_eq!(
        dec_intent_err(&cat(&[&full, &[0x00]])),
        CodecError::Malformed,
        "MoveTo with a trailing byte"
    );
    // Encode refuses `seq` 0 (and `u64::MAX`).
    for seq in [0, u64::MAX] {
        let bad = MoveIntent {
            seq,
            ..intent(MoveBody::Halt)
        };
        assert_eq!(encode_intent(&bad), Err(CodecError::Malformed));
    }
}

/// M2: every coordinate of a `MoveTo` point is checked against U3 when read.
#[test]
fn move_intent_point_bounds() {
    let zero = [0x00];
    for position in 0..3 {
        let with = |coord: &[u8]| {
            let mut parts: [&[u8]; 3] = [&zero, &zero, &zero];
            parts[position] = coord;
            move_to_bytes(&cat(&parts))
        };
        for (name, coord) in [("L", &COORD_L), ("-L", &[0xFF, 0xFF, 0xFF, 0xFF, 0x03])] {
            assert!(
                decode_intent(&with(coord)).is_ok(),
                "{name} accepted in position {position}"
            );
        }
        for (name, coord) in [
            ("L+1", &COORD_L_PLUS_1),
            ("-L-1", &COORD_MINUS_L_MINUS_1),
            ("i32::MIN", &COORD_I32_MIN),
            ("overflow", &COORD_OVERFLOW),
        ] {
            assert_eq!(
                dec_intent_err(&with(coord)),
                CodecError::Malformed,
                "{name} refused in position {position}"
            );
        }
    }
    // The contract's literal: `x` of `82 80 80 80 04` with a valid `y`, `z`.
    assert_eq!(
        dec_intent_err(&move_to_bytes(&cat(&[&COORD_L_PLUS_1, &[0x00, 0x00]]))),
        CodecError::Malformed
    );
    assert!(decode_intent(&move_to_bytes(&cat(&[&COORD_L, &[0x00, 0x00]]))).is_ok());
}

/// M2: two faults, the first check wins.
#[test]
fn move_intent_two_faults() {
    // First byte 04 with lane 00: UnsupportedVersion before WrongDirection.
    assert_eq!(
        dec_intent_err(&intent_hdr(0x04, 0, &[1], &[1], &[0x01])),
        CodecError::UnsupportedVersion
    );
    // Lane 00 with seq 0: WrongDirection before Malformed.
    assert_eq!(
        dec_intent_err(&intent_hdr(0xA1, 0, &[0], &[1], &[0x01])),
        CodecError::WrongDirection
    );
    // seq 0 with tag FF: Malformed before UnknownMessage.
    assert_eq!(
        dec_intent_err(&intent_hdr(0xA1, 1, &[0], &[1], &[0xFF])),
        CodecError::Malformed
    );
    // actor 0 with tag FF: Malformed before UnknownMessage.
    assert_eq!(
        dec_intent_err(&intent_hdr(0xA1, 1, &[1], &[0], &[0xFF])),
        CodecError::Malformed
    );
    // Tag FF with a trailing byte: UnknownMessage before the trailing check.
    assert_eq!(
        dec_intent_err(&ibytes(&[0xFF, 0x00])),
        CodecError::UnknownMessage
    );
    // Tag 02 likewise.
    assert_eq!(
        dec_intent_err(&ibytes(&[0x02, 0x00])),
        CodecError::UnknownMessage
    );
    // The cap is checked before the version.
    let mut big = vec![0x04];
    big.resize(MAX_INTENT_FRAME_BYTES + 1, 0);
    assert_eq!(dec_intent_err(&big), CodecError::FrameTooLarge);
}

// ---------------------------------------------------------------------------
// M3.
// ---------------------------------------------------------------------------

/// M3: receipt literals, the largest receipt and every code.
#[test]
fn receipt_literals() {
    let accepted = receipt_bytes(&[1], &[1], &[0x00, 0x65]);
    assert_eq!(accepted.len(), 24);
    assert_eq!(
        encode_down(&receipt_msg(MoveOutcome::Accepted { start_tick: 101 })),
        Ok(accepted.clone())
    );
    assert_eq!(
        decode_down(&accepted),
        Ok(receipt_msg(MoveOutcome::Accepted { start_tick: 101 }))
    );

    let superseded = receipt_bytes(&[1], &[1], &[0x01]);
    assert_eq!(superseded.len(), 23);
    assert_eq!(&superseded[21..], &[0x64, 0x01]);
    assert_eq!(
        encode_down(&receipt_msg(MoveOutcome::Superseded)),
        Ok(superseded.clone())
    );
    assert_eq!(
        decode_down(&superseded),
        Ok(receipt_msg(MoveOutcome::Superseded))
    );

    let rejected = receipt_bytes(&[1], &[1], &[0x02, 0x12]);
    assert_eq!(rejected.len(), 24);
    assert_eq!(&rejected[21..], &[0x64, 0x02, 0x12]);
    let msg = receipt_msg(MoveOutcome::Rejected(RejectionCode::IllegalAction));
    assert_eq!(encode_down(&msg), Ok(rejected.clone()));
    assert_eq!(decode_down(&rejected), Ok(msg));

    // The largest receipt: seq SEQ_LAST, every other number u64::MAX.
    let largest = MoveDown::Receipt(MoveReceipt {
        epoch: Z16,
        seq: SEQ_LAST,
        actor: net(u64::MAX),
        processed_tick: u64::MAX,
        outcome: MoveOutcome::Accepted {
            start_tick: u64::MAX,
        },
    });
    let expected = cat(&[
        &[0xA1, 0x01, 0x00],
        &Z16,
        &SEQ_LAST_VARINT,
        &U64_MAX_VARINT,
        &U64_MAX_VARINT,
        &[0x00],
        &U64_MAX_VARINT,
    ]);
    assert_eq!(expected.len(), 60);
    assert_eq!(encode_down(&largest), Ok(expected.clone()));
    assert_eq!(decode_down(&expected), Ok(largest));
}

/// M3: every code 1..=30 round-trips at its literal byte; 0 and 31..=255 are
/// refused with a valid remainder and a control.
#[test]
fn receipt_rejection_codes() {
    for code in 1u8..=30 {
        let parsed = RejectionCode::from_u8(code).expect("1..=30 is a code");
        let msg = receipt_msg(MoveOutcome::Rejected(parsed));
        let bytes = receipt_bytes(&[1], &[1], &[0x02, code]);
        assert_eq!(encode_down(&msg), Ok(bytes.clone()), "code {code}");
        assert_eq!(decode_down(&bytes), Ok(msg), "code {code}");
    }
    for bad in [0x00u8, 0x1F, 0x20, 0xFF] {
        assert_eq!(
            dec_down_err(&receipt_bytes(&[1], &[1], &[0x02, bad])),
            CodecError::UnknownMessage,
            "code byte {bad:#04x}"
        );
    }
}

/// M3: outcome tags, ranges and shapes.
#[test]
fn receipt_refusals() {
    // Outcome tag 03 followed by a valid start_tick; control tag 00.
    assert_eq!(
        dec_down_err(&receipt_bytes(&[1], &[1], &[0x03, 0x65])),
        CodecError::UnknownMessage
    );
    assert_eq!(
        dec_down_err(&receipt_bytes(&[1], &[1], &[0xFF, 0x65])),
        CodecError::UnknownMessage
    );
    assert!(decode_down(&receipt_bytes(&[1], &[1], &[0x00, 0x65])).is_ok());
    // seq 0, u64::MAX; actor 0, each with a complete valid remainder.
    let tail = [0x00, 0x65];
    assert_eq!(
        dec_down_err(&receipt_bytes(&[0], &[1], &tail)),
        CodecError::Malformed
    );
    assert_eq!(
        dec_down_err(&receipt_bytes(&U64_MAX_VARINT, &[1], &tail)),
        CodecError::Malformed
    );
    assert!(decode_down(&receipt_bytes(&SEQ_LAST_VARINT, &[1], &tail)).is_ok());
    assert_eq!(
        dec_down_err(&receipt_bytes(&[1], &[0], &tail)),
        CodecError::Malformed
    );
    // Accepted without start_tick; Superseded and Rejected with a trailing
    // byte; truncated rejected code.
    assert_eq!(
        dec_down_err(&receipt_bytes(&[1], &[1], &[0x00])),
        CodecError::Malformed
    );
    assert_eq!(
        dec_down_err(&receipt_bytes(&[1], &[1], &[0x01, 0x00])),
        CodecError::Malformed
    );
    assert_eq!(
        dec_down_err(&receipt_bytes(&[1], &[1], &[0x02, 0x12, 0x00])),
        CodecError::Malformed
    );
    assert_eq!(
        dec_down_err(&receipt_bytes(&[1], &[1], &[0x02])),
        CodecError::Malformed
    );
    assert_eq!(
        dec_down_err(&receipt_bytes(&[1], &[1], &[0x00, 0x65, 0x00])),
        CodecError::Malformed
    );
    // Encode refuses seq 0 and u64::MAX.
    for seq in [0, u64::MAX] {
        let bad = MoveDown::Receipt(MoveReceipt {
            seq,
            ..receipt(MoveOutcome::Superseded)
        });
        assert_eq!(encode_down(&bad), Err(CodecError::Malformed));
    }
    // Identity: every outcome encodes twice to equal bytes and round-trips.
    for outcome in [
        MoveOutcome::Accepted { start_tick: 0 },
        MoveOutcome::Accepted {
            start_tick: u64::MAX,
        },
        MoveOutcome::Superseded,
        MoveOutcome::Rejected(RejectionCode::NoPauseRight),
    ] {
        let msg = receipt_msg(outcome);
        let first = encode_down(&msg).expect("encodes");
        assert_eq!(encode_down(&msg).as_ref(), Ok(&first));
        assert_eq!(decode_down(&first), Ok(msg));
    }
}

// ---------------------------------------------------------------------------
// M4.
// ---------------------------------------------------------------------------

/// M4: `S1`, the empty sample and `processed_seq` bounds.
#[test]
fn sample_literals_and_bounds() {
    let expected = vec![
        0xA1, 0x01, 0x01, 0x64, 0x01, 0x02, 0x01, 0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F, 0x01,
        0x02, 0x00, 0x00, 0x00, 0x00,
    ];
    assert_eq!(expected.len(), 20);
    assert_eq!(s1_bytes(), expected);
    assert_eq!(encode_down(&MoveDown::Sample(s1())), Ok(expected.clone()));
    assert_eq!(decode_down(&expected), Ok(MoveDown::Sample(s1())));

    let empty = MoveSample {
        server_tick: 100,
        processed_seq: 0,
        entries: vec![],
    };
    let expected = vec![0xA1, 0x01, 0x01, 0x64, 0x00, 0x00];
    assert_eq!(
        encode_down(&MoveDown::Sample(empty.clone())),
        Ok(expected.clone())
    );
    assert_eq!(decode_down(&expected), Ok(MoveDown::Sample(empty)));

    // processed_seq SEQ_LAST is accepted; u64::MAX is Malformed.
    let at_last = sample_bytes(&SEQ_LAST_VARINT, &[0x00], &[]);
    assert_eq!(
        decode_down(&at_last),
        Ok(MoveDown::Sample(MoveSample {
            server_tick: 100,
            processed_seq: SEQ_LAST,
            entries: vec![],
        }))
    );
    assert_eq!(
        dec_down_err(&sample_bytes(&U64_MAX_VARINT, &[0x00], &[])),
        CodecError::Malformed
    );
    // A server_tick of u64::MAX is any value.
    let big_tick = cat(&[&[0xA1, 0x01, 0x01], &U64_MAX_VARINT, &[0x00, 0x00]]);
    assert_eq!(
        decode_down(&big_tick),
        Ok(MoveDown::Sample(MoveSample {
            server_tick: u64::MAX,
            processed_seq: 0,
            entries: vec![],
        }))
    );
}

/// M4: 38 maxima entries are 1,012 bytes; 39 is a LimitExceeded count.
#[test]
fn sample_max_and_count_cap() {
    let entries = max_entries(38);
    assert_eq!(entries[0].entity, net(u64::MAX - 37));
    assert_eq!(entries[37].entity, net(u64::MAX));
    let sample = MoveSample {
        server_tick: u64::MAX,
        processed_seq: SEQ_LAST,
        entries,
    };
    let mut expected = cat(&[
        &[0xA1, 0x01, 0x01],
        &U64_MAX_VARINT,
        &SEQ_LAST_VARINT,
        &[38],
    ]);
    for i in 0..38u64 {
        expected.extend(varint(u64::MAX - 37 + i));
        expected.extend(COORD_L);
        expected.extend(COORD_L);
        expected.extend(COORD_L);
        expected.push(0x01);
    }
    assert_eq!(expected.len(), 1_012);
    assert_eq!(
        MAX_SAMPLE_HEADER_BYTES + 38 * MAX_SAMPLE_ENTRY_BYTES,
        expected.len()
    );
    assert_eq!(
        encode_down(&MoveDown::Sample(sample.clone())),
        Ok(expected.clone())
    );
    assert_eq!(decode_down(&expected), Ok(MoveDown::Sample(sample)));

    // A count of 39 (`27`) with no entry following is LimitExceeded, before
    // any entry is read; the control with count 38 and no entries is
    // Malformed (truncation), so the two differ only by the count check.
    assert_eq!(
        dec_down_err(&sample_bytes(&[0x01], &[0x27], &[])),
        CodecError::LimitExceeded
    );
    assert_eq!(
        dec_down_err(&sample_bytes(&[0x01], &[0x26], &[])),
        CodecError::Malformed
    );
    // A count varint that overflows u64 is Malformed.
    let overflow = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x02];
    assert_eq!(
        dec_down_err(&sample_bytes(&[0x01], &overflow, &[])),
        CodecError::Malformed
    );
    // A multi-byte count above the cap (`81 02` = 257).
    assert_eq!(
        dec_down_err(&sample_bytes(&[0x01], &[0x81, 0x02], &[])),
        CodecError::LimitExceeded
    );
}

/// M4: entity order, entity 0, point and `moving` refusals, each before a
/// valid second entry, with controls.
#[test]
fn sample_entry_refusals() {
    let two = |first: &[u8], second: &[u8]| sample_bytes(&[0x01], &[0x02], &cat(&[first, second]));
    let e = |entity: u64| entry_bytes(entity, &A_BYTES, 0);

    // Order: [2, 1] and [1, 1] are Malformed; control [1, 2] decodes.
    assert_eq!(dec_down_err(&two(&e(2), &e(1))), CodecError::Malformed);
    assert_eq!(dec_down_err(&two(&e(1), &e(1))), CodecError::Malformed);
    let ascending = decode_down(&two(&e(1), &e(2))).expect("[1, 2] decodes");
    assert_eq!(
        ascending,
        MoveDown::Sample(MoveSample {
            server_tick: 100,
            processed_seq: 1,
            entries: vec![entry(1, pa(), false), entry(2, pa(), false)],
        })
    );
    // A gap is fine.
    assert!(decode_down(&two(&e(1), &e(1 << 40))).is_ok());

    // Entity 0 as the first entry and as the second.
    assert_eq!(
        dec_down_err(&two(&e(0), &valid_entry_9())),
        CodecError::Malformed
    );
    assert_eq!(dec_down_err(&two(&e(1), &e(0))), CodecError::Malformed);
    // x of L+1 in the first entry before a valid second entry.
    let bad_x = cat(&[&[0x01], &COORD_L_PLUS_1, &[0x00, 0x00], &[0x00]]);
    assert_eq!(
        dec_down_err(&two(&bad_x, &valid_entry_9())),
        CodecError::Malformed
    );
    let good_x = cat(&[&[0x01], &COORD_L, &[0x00, 0x00], &[0x00]]);
    assert!(decode_down(&two(&good_x, &valid_entry_9())).is_ok());
    // A moving byte of 02 in the first entry before a valid second entry.
    let bad_moving = entry_bytes(1, &A_BYTES, 0x02);
    assert_eq!(
        dec_down_err(&two(&bad_moving, &valid_entry_9())),
        CodecError::Malformed
    );
    let good_moving = entry_bytes(1, &A_BYTES, 0x01);
    assert!(decode_down(&two(&good_moving, &valid_entry_9())).is_ok());
    // Truncation after the last entry's point and a trailing byte.
    assert_eq!(
        dec_down_err(&sample_bytes(&[0x01], &[0x01], &cat(&[&[0x01], &A_BYTES]))),
        CodecError::Malformed
    );
    assert_eq!(
        dec_down_err(&cat(&[&sample_bytes(&[0x01], &[0x01], &e(1)), &[0x00]])),
        CodecError::Malformed
    );
}

// ---------------------------------------------------------------------------
// M5.
// ---------------------------------------------------------------------------

/// M5: cap, version, lane and kind refusals, with controls.
#[test]
fn move_down_refusals() {
    let s1 = s1_bytes();
    assert!(decode_down(&s1).is_ok());
    // Kind 02 and FF before S1's body.
    for kind in [0x02u8, 0xFF] {
        let mut bytes = s1.clone();
        bytes[2] = kind;
        assert_eq!(
            dec_down_err(&bytes),
            CodecError::UnknownMessage,
            "kind {kind:#04x}"
        );
    }
    // Lane 00 before S1's kind and body.
    let mut bytes = s1.clone();
    bytes[1] = 0x00;
    assert_eq!(dec_down_err(&bytes), CodecError::WrongDirection);
    // The v4 lane-0 delta decoder refuses S1.
    assert_eq!(
        codec_v4::decode_delta(&s1),
        Err(CodecError::UnsupportedVersion)
    );
    // First bytes 03 and 04.
    for first in [0x03u8, 0x04] {
        assert_eq!(
            dec_down_err(&with_first(&s1, first)),
            CodecError::UnsupportedVersion
        );
    }
    // Empty and the cap.
    assert_eq!(dec_down_err(&[]), CodecError::Malformed);
    let mut big = vec![0x02];
    big.resize(MAX_MOVE_DOWN_BYTES + 1, 0);
    assert_eq!(big.len(), 1_025);
    assert_eq!(dec_down_err(&big), CodecError::FrameTooLarge);
    // S1 padded with 00 to 1,024 bytes, and S1 plus one byte.
    assert_eq!(
        dec_down_err(&padded(&s1, MAX_MOVE_DOWN_BYTES)),
        CodecError::Malformed
    );
    assert_eq!(dec_down_err(&cat(&[&s1, &[0x00]])), CodecError::Malformed);
    // A lane-1 intent is not a downstream message: its byte 1 is the epoch's
    // first byte (00), which is not lane 1.
    assert_eq!(dec_down_err(&halt_bytes()), CodecError::WrongDirection);
}

/// M5: two faults, the first check wins.
#[test]
fn move_down_two_faults() {
    let s1 = s1_bytes();
    // Lane 00 with kind FF: WrongDirection before UnknownMessage.
    let mut bytes = s1.clone();
    bytes[1] = 0x00;
    bytes[2] = 0xFF;
    assert_eq!(dec_down_err(&bytes), CodecError::WrongDirection);
    // First byte 04 with lane 00: UnsupportedVersion first.
    let mut bytes = s1.clone();
    bytes[0] = 0x04;
    bytes[1] = 0x00;
    assert_eq!(dec_down_err(&bytes), CodecError::UnsupportedVersion);
    // Kind FF with S1's body: UnknownMessage.
    let mut bytes = s1.clone();
    bytes[2] = 0xFF;
    assert_eq!(dec_down_err(&bytes), CodecError::UnknownMessage);
    // A receipt with seq 0 and code 1F: Malformed (seq before the code).
    assert_eq!(
        dec_down_err(&receipt_bytes(&[0], &[1], &[0x02, 0x1F])),
        CodecError::Malformed
    );
    assert_eq!(
        dec_down_err(&receipt_bytes(&[1], &[1], &[0x02, 0x1F])),
        CodecError::UnknownMessage
    );
    // A receipt with seq 0 and outcome tag 03: Malformed.
    assert_eq!(
        dec_down_err(&receipt_bytes(&[0], &[1], &[0x03, 0x65])),
        CodecError::Malformed
    );
    assert_eq!(
        dec_down_err(&receipt_bytes(&[1], &[1], &[0x03, 0x65])),
        CodecError::UnknownMessage
    );
    // A sample with processed_seq u64::MAX and count 39: Malformed first;
    // count 39 alone is LimitExceeded.
    assert_eq!(
        dec_down_err(&sample_bytes(&U64_MAX_VARINT, &[0x27], &[])),
        CodecError::Malformed
    );
    assert_eq!(
        dec_down_err(&sample_bytes(&[0x01], &[0x27], &[])),
        CodecError::LimitExceeded
    );
    // The cap beats the version.
    let mut big = vec![0x04];
    big.resize(MAX_MOVE_DOWN_BYTES + 1, 0);
    assert_eq!(dec_down_err(&big), CodecError::FrameTooLarge);
}

/// M5: every other codec refuses `0xA1` with `UnsupportedVersion`.
#[test]
fn other_codecs_refuse_movement_version() {
    for bytes in [s1_bytes(), halt_bytes()] {
        let unsupported = CodecError::UnsupportedVersion;
        assert_eq!(decode_intent_v1(&bytes), Err(unsupported));
        assert_eq!(decode_delta_v1(&bytes), Err(unsupported));
        assert_eq!(decode_intent_v2(&bytes), Err(unsupported));
        assert_eq!(decode_delta_v2(&bytes), Err(unsupported));
        assert_eq!(decode_intent_v3(&bytes), Err(unsupported));
        assert_eq!(decode_delta_v3(&bytes), Err(unsupported));
        assert_eq!(codec_v4::decode_intent(&bytes), Err(unsupported));
        assert_eq!(codec_v4::decode_delta(&bytes), Err(unsupported));
        assert_eq!(
            codec_transfer::decode_down(&bytes).expect_err("refused"),
            unsupported
        );
        assert_eq!(
            codec_transfer::decode_up(&bytes).expect_err("refused"),
            unsupported
        );
        assert_eq!(
            codec_resume::decode_request(&bytes).expect_err("refused"),
            unsupported
        );
        assert_eq!(
            codec_resume::decode_down(&bytes).expect_err("refused"),
            unsupported
        );
    }
}

// ---------------------------------------------------------------------------
// M6.
// ---------------------------------------------------------------------------

fn sample_with(processed_seq: u64, entities: &[u64]) -> MoveSample {
    MoveSample {
        server_tick: 100,
        processed_seq,
        entries: entities
            .iter()
            .map(|&entity| entry(entity, pa(), false))
            .collect(),
    }
}

/// `check_sample` and `encode_down` agree on every refusal and on success.
fn assert_sample_refused(sample: &MoveSample, expected: CodecError) {
    assert_eq!(check_sample(sample), Err(expected));
    assert_eq!(
        encode_down(&MoveDown::Sample(sample.clone())),
        Err(expected)
    );
}

/// M6: the three encode checks, in order.
#[test]
fn sample_encode_checks() {
    let ascending: Vec<u64> = (1..=39).collect();
    let descending: Vec<u64> = (1..=39).rev().collect();

    // 39 entries.
    assert_sample_refused(&sample_with(1, &ascending), CodecError::LimitExceeded);
    // [2, 1], [1, 1].
    assert_sample_refused(&sample_with(1, &[2, 1]), CodecError::Malformed);
    assert_sample_refused(&sample_with(1, &[1, 1]), CodecError::Malformed);
    // processed_seq u64::MAX.
    assert_sample_refused(&sample_with(u64::MAX, &[1]), CodecError::Malformed);
    // u64::MAX with 39 descending entries: the first check wins.
    assert_sample_refused(&sample_with(u64::MAX, &descending), CodecError::Malformed);
    // 39 descending entries with a valid processed_seq: count before order.
    assert_sample_refused(&sample_with(1, &descending), CodecError::LimitExceeded);
    // 38 descending entries: now the order is what fails.
    assert_sample_refused(&sample_with(1, &descending[1..]), CodecError::Malformed);

    // Valid samples pass both, including the empty one, 38 entries and a
    // processed_seq of 0 and SEQ_LAST.
    let ok38: Vec<u64> = (1..=38).collect();
    for sample in [
        sample_with(0, &[]),
        sample_with(SEQ_LAST, &[1]),
        sample_with(1, &ok38),
    ] {
        assert_eq!(check_sample(&sample), Ok(()));
        assert!(encode_down(&MoveDown::Sample(sample)).is_ok());
    }

    // A receipt with seq 0 is Malformed.
    let bad = MoveDown::Receipt(MoveReceipt {
        seq: 0,
        ..receipt(MoveOutcome::Superseded)
    });
    assert_eq!(encode_down(&bad), Err(CodecError::Malformed));
}

// ---------------------------------------------------------------------------
// M7.
// ---------------------------------------------------------------------------

/// M7: empty input is checked first.
#[test]
fn sample_datagrams_empty() {
    assert_eq!(sample_datagrams(100, 1, &[]), Ok(vec![]));
    assert_eq!(sample_datagrams(100, u64::MAX, &[]), Ok(vec![]));
}

/// M7: 38 entries are one datagram; 39 are two (38, then 1).
#[test]
fn sample_datagrams_chunking() {
    let entries: Vec<SampleEntry> = (1..=38).map(|i| entry(i, pb(), i % 2 == 0)).collect();
    let whole = MoveSample {
        server_tick: 100,
        processed_seq: 1,
        entries: entries.clone(),
    };
    assert_eq!(
        sample_datagrams(100, 1, &entries),
        Ok(vec![encode_down(&MoveDown::Sample(whole)).expect("encodes")])
    );

    let mut entries = entries;
    entries.push(entry(39, pa(), true));
    let datagrams = sample_datagrams(100, 1, &entries).expect("splits");
    assert_eq!(datagrams.len(), 2);
    let expected_first = MoveSample {
        server_tick: 100,
        processed_seq: 1,
        entries: entries[..38].to_vec(),
    };
    let expected_second = MoveSample {
        server_tick: 100,
        processed_seq: 1,
        entries: entries[38..].to_vec(),
    };
    assert_eq!(
        datagrams[0],
        encode_down(&MoveDown::Sample(expected_first.clone())).expect("encodes")
    );
    assert_eq!(
        datagrams[1],
        encode_down(&MoveDown::Sample(expected_second.clone())).expect("encodes")
    );
    assert_eq!(
        decode_down(&datagrams[0]),
        Ok(MoveDown::Sample(expected_first))
    );
    assert_eq!(
        decode_down(&datagrams[1]),
        Ok(MoveDown::Sample(expected_second))
    );
    // Entries 0 (the empty tail) never produce a third datagram, and a
    // single entry is one datagram.
    assert_eq!(
        sample_datagrams(100, 1, &entries[..1])
            .expect("splits")
            .len(),
        1
    );
}

/// M7: 1,024 maxima entries make 27 datagrams, none lost or reordered.
#[test]
fn sample_datagrams_full_tick() {
    let entries = max_entries(1_024);
    assert_eq!(entries[0].entity, net(u64::MAX - 1_023));
    let datagrams = sample_datagrams(u64::MAX, SEQ_LAST, &entries).expect("splits");
    assert_eq!(datagrams.len(), 27);
    let mut joined = Vec::new();
    for (index, datagram) in datagrams.iter().enumerate() {
        assert!(datagram.len() <= 1_012, "datagram {index}");
        assert!(datagram.len() <= MAX_MOVE_DOWN_BYTES);
        let expected_len = if index < 26 { 1_012 } else { 24 + 36 * 26 };
        assert_eq!(datagram.len(), expected_len, "datagram {index}");
        let MoveDown::Sample(sample) = decode_down(datagram).expect("decodes") else {
            panic!("datagram {index} is not a sample");
        };
        assert_eq!(sample.server_tick, u64::MAX);
        assert_eq!(sample.processed_seq, SEQ_LAST);
        assert_eq!(sample.entries.len(), if index < 26 { 38 } else { 36 });
        joined.extend(sample.entries);
    }
    assert_eq!(joined, entries);
}

/// M7: refusals check the whole input first and return no datagrams.
#[test]
fn sample_datagrams_refusals() {
    // [1, 3, 2] with processed_seq u64::MAX: Malformed (either check).
    let bad_order = [
        entry(1, pa(), false),
        entry(3, pa(), false),
        entry(2, pa(), false),
    ];
    assert_eq!(
        sample_datagrams(100, u64::MAX, &bad_order),
        Err(CodecError::Malformed)
    );
    // Each fault alone is Malformed too, with a control that splits.
    assert_eq!(
        sample_datagrams(100, 1, &bad_order),
        Err(CodecError::Malformed)
    );
    let ascending = [
        entry(1, pa(), false),
        entry(2, pa(), false),
        entry(3, pa(), false),
    ];
    assert_eq!(
        sample_datagrams(100, u64::MAX, &ascending),
        Err(CodecError::Malformed)
    );
    assert_eq!(sample_datagrams(100, 1, &ascending).map(|d| d.len()), Ok(1));
    // A descent that falls in the second chunk is found before any output.
    let mut entries: Vec<SampleEntry> = (1..=38).map(|i| entry(i, pa(), false)).collect();
    entries.push(entry(39, pa(), false));
    entries.push(entry(10, pa(), false));
    assert_eq!(
        sample_datagrams(100, 1, &entries),
        Err(CodecError::Malformed)
    );
    // A descent exactly across the chunk boundary (38th and 39th entries).
    let mut across: Vec<SampleEntry> = (1..=37).map(|i| entry(i, pa(), false)).collect();
    across.push(entry(100, pa(), false));
    across.push(entry(50, pa(), false));
    assert_eq!(
        sample_datagrams(100, 1, &across),
        Err(CodecError::Malformed)
    );
    // Equal neighbours are not strictly ascending.
    assert_eq!(
        sample_datagrams(100, 1, &[entry(1, pa(), false), entry(1, pa(), true)]),
        Err(CodecError::Malformed)
    );
    // SEQ_LAST is a valid processed_seq; 0 too.
    assert!(sample_datagrams(100, SEQ_LAST, &ascending).is_ok());
    assert!(sample_datagrams(100, 0, &ascending).is_ok());
}

// ---------------------------------------------------------------------------
// M8.
// ---------------------------------------------------------------------------

/// M8: every constant of the `movement_v4` Interface has its value, and the
/// lane differs from every other lane.
#[test]
fn movement_constants() {
    assert_eq!(MOVEMENT_VERSION, 0xA1);
    assert_eq!(LANE_MOVEMENT, 1);
    assert_eq!(MOVE_TAG_MOVE_TO, 0);
    assert_eq!(MOVE_TAG_HALT, 1);
    assert_eq!(DOWN_KIND_RECEIPT, 0);
    assert_eq!(DOWN_KIND_SAMPLE, 1);
    assert_eq!(OUTCOME_TAG_ACCEPTED, 0);
    assert_eq!(OUTCOME_TAG_SUPERSEDED, 1);
    assert_eq!(OUTCOME_TAG_REJECTED, 2);
    assert_eq!(MAX_MOVE_DOWN_BYTES, 1_024);
    assert_eq!(MAX_SAMPLE_HEADER_BYTES, 24);
    assert_eq!(MAX_SAMPLE_ENTRY_BYTES, 26);
    assert_eq!(MAX_SAMPLE_ENTRIES, 38);
    assert_eq!(POLICY_MOVE_INTENTS_PER_SEC, 20);
    assert_eq!(POLICY_MOVE_INTENT_BURST, 40);
    assert_eq!(POLICY_MOVE_IN_FLIGHT, 8);

    assert_ne!(LANE_MOVEMENT, LANE_COMBAT);
    assert_ne!(LANE_MOVEMENT, LANE_SNAPSHOT);
    assert_ne!(LANE_MOVEMENT, LANE_SESSION);
    assert_eq!(
        [LANE_COMBAT, LANE_MOVEMENT, LANE_SNAPSHOT, LANE_SESSION],
        [0, 1, 2, 3]
    );
    // MOVEMENT_VERSION is no lane-0 version and no other subprotocol's first
    // byte.
    for other in [1u8, 2, 3, 4, 0x81, 0x82, 0x91] {
        assert_ne!(MOVEMENT_VERSION, other);
    }
}
