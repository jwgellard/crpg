//! T026p2 two-keys contract tests: lane-1 movement (`movement_v4`), written
//! from `tasks/T026p.md` alone (Behaviour M-1..M-9, Errors, Tests M1..M8),
//! through the public `crpg-net` API only.
//!
//! Every rejection case keeps a complete valid remainder after the bad byte
//! and has an accepted control with the good byte, so truncation cannot mask
//! the refusal.

use crpg_net::codec::{self, CodecError};
use crpg_net::codec_resume;
use crpg_net::codec_transfer;
use crpg_net::codec_v2;
use crpg_net::codec_v3;
use crpg_net::codec_v4;
use crpg_net::movement_v4::{
    check_sample, decode_down, decode_intent, encode_down, encode_intent, sample_datagrams,
    MoveBody, MoveDown, MoveIntent, MoveOutcome, MoveReceipt, MoveSample, SampleEntry,
    DOWN_KIND_RECEIPT, DOWN_KIND_SAMPLE, LANE_MOVEMENT, MAX_MOVE_DOWN_BYTES, MAX_SAMPLE_ENTRIES,
    MAX_SAMPLE_ENTRY_BYTES, MAX_SAMPLE_HEADER_BYTES, MOVEMENT_VERSION, MOVE_TAG_HALT,
    MOVE_TAG_MOVE_TO, OUTCOME_TAG_ACCEPTED, OUTCOME_TAG_REJECTED, OUTCOME_TAG_SUPERSEDED,
    POLICY_MOVE_INTENTS_PER_SEC, POLICY_MOVE_INTENT_BURST, POLICY_MOVE_IN_FLIGHT,
};
use crpg_net::protocol::{
    LANE_COMBAT, MAX_INTENT_FRAME_BYTES, POLICY_RETRY_CACHE_PER_PEER_LANE, SEQ_FIRST, SEQ_LAST,
};
use crpg_net::protocol_resume::LANE_SESSION;
use crpg_net::protocol_transfer::LANE_SNAPSHOT;
use crpg_net::protocol_v4::{NetId, Point, RejectionCode};

const L: i32 = 1 << 29;
const Z16: [u8; 16] = [0; 16];

// ---------------------------------------------------------------- helpers --

fn id(n: u64) -> NetId {
    NetId::new(n).unwrap()
}

fn p(x: i32, y: i32, z: i32) -> Point {
    Point::new([x, y, z]).unwrap()
}

/// LEB128 varint of a u64 (postcard's integer encoding).
fn leb(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

/// Zigzag varint of an i32 coordinate.
fn zz(v: i32) -> Vec<u8> {
    leb((((v << 1) ^ (v >> 31)) as u32) as u64)
}

fn pt(x: i32, y: i32, z: i32) -> Vec<u8> {
    let mut out = zz(x);
    out.extend(zz(y));
    out.extend(zz(z));
    out
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.iter().flat_map(|p| p.iter().copied()).collect()
}

fn max_varint() -> Vec<u8> {
    // u64::MAX: nine 0xFF bytes then 0x01.
    let mut v = vec![0xFF; 9];
    v.push(0x01);
    v
}

/// `A1 epoch lane seq actor` with pre-encoded seq/actor bytes.
fn ihead(epoch: [u8; 16], lane: u8, seq: &[u8], actor: &[u8]) -> Vec<u8> {
    cat(&[&[0xA1], &epoch, &[lane], seq, actor])
}

fn halt_bytes() -> Vec<u8> {
    cat(&[&ihead(Z16, 1, &[1], &[1]), &[1]])
}

fn move_to_bytes(point: &[u8]) -> Vec<u8> {
    cat(&[&ihead(Z16, 1, &[1], &[1]), &[0], point])
}

fn move_to_b() -> Vec<u8> {
    move_to_bytes(&[0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F])
}

/// `A1 01 00 epoch seq actor processed_tick tail`.
fn receipt_with(epoch: [u8; 16], seq: &[u8], actor: &[u8], tick: &[u8], tail: &[u8]) -> Vec<u8> {
    cat(&[&[0xA1, 0x01, 0x00], &epoch, seq, actor, tick, tail])
}

fn receipt_std(tail: &[u8]) -> Vec<u8> {
    receipt_with(Z16, &[1], &[1], &[0x64], tail)
}

fn entry_bytes(entity: &[u8], point: &[u8], moving: u8) -> Vec<u8> {
    cat(&[entity, point, &[moving]])
}

/// `A1 01 01 server_tick processed_seq count body`.
fn sample_raw(tick: &[u8], pseq: &[u8], count: &[u8], body: &[u8]) -> Vec<u8> {
    cat(&[&[0xA1, 0x01, 0x01], tick, pseq, count, body])
}

/// S1 = { 100, 1, [{ id(1), B, true }, { id(2), A, false }] }.
fn s1_bytes() -> Vec<u8> {
    vec![
        0xA1, 0x01, 0x01, 0x64, 0x01, 0x02, 0x01, 0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F, 0x01,
        0x02, 0x00, 0x00, 0x00, 0x00,
    ]
}

fn s1_value() -> MoveSample {
    MoveSample {
        server_tick: 100,
        processed_seq: 1,
        entries: vec![
            SampleEntry {
                entity: id(1),
                point: p(65536, 0, -131072),
                moving: true,
            },
            SampleEntry {
                entity: id(2),
                point: p(0, 0, 0),
                moving: false,
            },
        ],
    }
}

fn entry(entity: u64, k: i32, moving: bool) -> SampleEntry {
    SampleEntry {
        entity: id(entity),
        point: p(k, -k, 2 * k),
        moving,
    }
}

fn entries_of(ids: &[u64]) -> Vec<SampleEntry> {
    ids.iter()
        .enumerate()
        .map(|(i, &e)| entry(e, i as i32, i % 2 == 0))
        .collect()
}

fn maxima_entries(count: u64) -> Vec<SampleEntry> {
    (0..count)
        .map(|i| SampleEntry {
            entity: id(u64::MAX - (count - 1) + i),
            point: p(L, L, L),
            moving: true,
        })
        .collect()
}

fn sample_of(tick: u64, pseq: u64, entries: Vec<SampleEntry>) -> MoveSample {
    MoveSample {
        server_tick: tick,
        processed_seq: pseq,
        entries,
    }
}

fn dec_i(bytes: &[u8]) -> Result<MoveIntent, CodecError> {
    decode_intent(bytes)
}

fn dec_d(bytes: &[u8]) -> Result<MoveDown, CodecError> {
    decode_down(bytes)
}

fn intent(seq: u64, actor: u64, body: MoveBody) -> MoveIntent {
    MoveIntent {
        epoch: Z16,
        seq,
        actor: id(actor),
        body,
    }
}

fn receipt(seq: u64, actor: u64, tick: u64, outcome: MoveOutcome) -> MoveReceipt {
    MoveReceipt {
        epoch: Z16,
        seq,
        actor: id(actor),
        processed_tick: tick,
        outcome,
    }
}

fn assert_intent_round_trip(i: &MoveIntent, expected: &[u8]) {
    let bytes = encode_intent(i).unwrap();
    assert_eq!(bytes, expected);
    assert_eq!(encode_intent(i).unwrap(), bytes, "encode is deterministic");
    assert_eq!(dec_i(&bytes), Ok(*i));
}

fn assert_down_round_trip(m: &MoveDown, expected: &[u8]) {
    let bytes = encode_down(m).unwrap();
    assert_eq!(bytes, expected);
    assert_eq!(encode_down(m).unwrap(), bytes, "encode is deterministic");
    assert_eq!(dec_d(&bytes), Ok(m.clone()));
}

/// Every strict prefix of a valid message is `Malformed`; one trailing byte
/// is `Malformed`.
fn assert_intent_truncation_and_trailing(bytes: &[u8]) {
    assert!(dec_i(bytes).is_ok());
    for n in 0..bytes.len() {
        assert_eq!(
            dec_i(&bytes[..n]),
            Err(CodecError::Malformed),
            "prefix {n} of {}",
            bytes.len()
        );
    }
    let mut longer = bytes.to_vec();
    longer.push(0);
    assert_eq!(dec_i(&longer), Err(CodecError::Malformed));
}

fn assert_down_truncation_and_trailing(bytes: &[u8]) {
    assert!(dec_d(bytes).is_ok());
    for n in 0..bytes.len() {
        assert_eq!(
            dec_d(&bytes[..n]),
            Err(CodecError::Malformed),
            "prefix {n} of {}",
            bytes.len()
        );
    }
    let mut longer = bytes.to_vec();
    longer.push(0);
    assert_eq!(dec_d(&longer), Err(CodecError::Malformed));
}

/// Coordinate encodings that are out of U3 or overflow `i32`.
const COORD_L: [u8; 5] = [0x80, 0x80, 0x80, 0x80, 0x04];
const COORD_NEG_L: [u8; 5] = [0xFF, 0xFF, 0xFF, 0xFF, 0x03];
const COORD_L_PLUS_1: [u8; 5] = [0x82, 0x80, 0x80, 0x80, 0x04];
const COORD_NEG_L_MINUS_1: [u8; 5] = [0x81, 0x80, 0x80, 0x80, 0x04];
const COORD_I32_MIN: [u8; 5] = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
const COORD_OVERFLOW: [u8; 5] = [0xFF, 0xFF, 0xFF, 0xFF, 0x1F];

/// Point bytes with `coord` in position `pos` (0 x, 1 y, 2 z), others `00`.
fn point_with(pos: usize, coord: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..3 {
        if i == pos {
            out.extend_from_slice(coord);
        } else {
            out.push(0);
        }
    }
    out
}

// ============================================================== M1 =========

#[test]
fn m1_move_intent_literals() {
    // MoveTo { B }: A1 Z16 01 01 01 00 80 80 08 00 FF FF 0F, 28 bytes.
    let mut expected = vec![0xA1];
    expected.extend_from_slice(&Z16);
    expected.extend_from_slice(&[
        0x01, 0x01, 0x01, 0x00, 0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F,
    ]);
    assert_eq!(expected.len(), 28);
    assert_intent_round_trip(
        &intent(
            1,
            1,
            MoveBody::MoveTo {
                destination: p(65536, 0, -131072),
            },
        ),
        &expected,
    );
    assert_eq!(expected, move_to_b());

    // Halt: A1 Z16 01 01 01 01, 21 bytes, no observed_tick.
    let mut halt = vec![0xA1];
    halt.extend_from_slice(&Z16);
    halt.extend_from_slice(&[0x01, 0x01, 0x01, 0x01]);
    assert_eq!(halt.len(), 21);
    assert_intent_round_trip(&intent(1, 1, MoveBody::Halt), &halt);
}

#[test]
fn m1_largest_intent_is_54_bytes() {
    let i = intent(
        SEQ_LAST,
        u64::MAX,
        MoveBody::MoveTo {
            destination: p(L, L, L),
        },
    );
    let bytes = encode_intent(&i).unwrap();
    assert_eq!(bytes.len(), 54);
    let expected = cat(&[
        &ihead(Z16, 1, &leb(SEQ_LAST), &leb(u64::MAX)),
        &[0],
        &pt(L, L, L),
    ]);
    assert_eq!(bytes, expected);
    assert_eq!(dec_i(&bytes), Ok(i));
}

#[test]
fn m1_field_order_with_distinct_values() {
    let epoch: [u8; 16] = core::array::from_fn(|i| i as u8 + 1);
    let seq = 300u64;
    let actor = 70_000u64;
    // 300 is the two-byte varint AC 02.
    assert_eq!(leb(300), vec![0xAC, 0x02]);
    let i = MoveIntent {
        epoch,
        seq,
        actor: id(actor),
        body: MoveBody::MoveTo {
            destination: p(-5, 7, 123_456),
        },
    };
    let expected = cat(&[
        &ihead(epoch, 1, &leb(seq), &leb(actor)),
        &[0],
        &pt(-5, 7, 123_456),
    ]);
    assert_eq!(encode_intent(&i).unwrap(), expected);
    assert_eq!(dec_i(&expected), Ok(i));
    // epoch, seq, actor are all kept in a Halt too
    let h = MoveIntent {
        epoch,
        seq,
        actor: id(actor),
        body: MoveBody::Halt,
    };
    let expected_h = cat(&[&ihead(epoch, 1, &leb(seq), &leb(actor)), &[1]]);
    assert_eq!(encode_intent(&h).unwrap(), expected_h);
    assert_eq!(dec_i(&expected_h), Ok(h));
}

#[test]
fn m1_seq_range_endpoints_round_trip() {
    for seq in [SEQ_FIRST, 2, 127, 128, SEQ_LAST] {
        let h = intent(seq, 1, MoveBody::Halt);
        let expected = cat(&[&ihead(Z16, 1, &leb(seq), &[1]), &[1]]);
        assert_intent_round_trip(&h, &expected);
    }
}

#[test]
fn m1_tag_constants_on_the_wire() {
    assert_eq!(MOVE_TAG_MOVE_TO, 0);
    assert_eq!(MOVE_TAG_HALT, 1);
    let bytes = encode_intent(&intent(
        1,
        1,
        MoveBody::MoveTo {
            destination: p(0, 0, 0),
        },
    ))
    .unwrap();
    assert_eq!(bytes[0], MOVEMENT_VERSION);
    assert_eq!(bytes[17], LANE_MOVEMENT);
    assert_eq!(bytes[20], MOVE_TAG_MOVE_TO);
    let bytes = encode_intent(&intent(1, 1, MoveBody::Halt)).unwrap();
    assert_eq!(bytes[20], MOVE_TAG_HALT);
}

#[test]
fn m1_point_bounds_accept_limits_in_each_position() {
    for pos in 0..3 {
        for coord in [COORD_L, COORD_NEG_L] {
            let bytes = move_to_bytes(&point_with(pos, &coord));
            let got = dec_i(&bytes).unwrap_or_else(|e| panic!("pos {pos} {coord:?}: {e:?}"));
            let raw = {
                let mut r = [0i32; 3];
                r[pos] = if coord == COORD_L { L } else { -L };
                r
            };
            assert_eq!(
                got.body,
                MoveBody::MoveTo {
                    destination: Point::new(raw).unwrap()
                }
            );
            // and encodes back to the same bytes
            assert_eq!(encode_intent(&got).unwrap(), bytes);
        }
    }
}

// ============================================================== M2 =========

#[test]
fn m2_version_byte_gate() {
    for v in [0x01u8, 0x02, 0x03, 0x04, 0x81, 0x82, 0x91] {
        let mut bytes = halt_bytes();
        bytes[0] = v;
        assert_eq!(dec_i(&bytes), Err(CodecError::UnsupportedVersion), "{v:#x}");
    }
    // every first byte but 0xA1 is refused, nothing else is
    for v in 0..=255u8 {
        let mut bytes = halt_bytes();
        bytes[0] = v;
        if v == 0xA1 {
            assert!(dec_i(&bytes).is_ok());
        } else {
            assert_eq!(dec_i(&bytes), Err(CodecError::UnsupportedVersion), "{v:#x}");
        }
    }
    // the version byte is checked before truncation
    for v in [0x01u8, 0x03, 0x04, 0x81, 0xA0, 0xA2, 0xFF] {
        assert_eq!(dec_i(&[v]), Err(CodecError::UnsupportedVersion));
    }
}

#[test]
fn m2_empty_and_cap() {
    assert_eq!(dec_i(&[]), Err(CodecError::Malformed));
    assert_eq!(MAX_INTENT_FRAME_BYTES, 4096);
    let mut big = vec![0u8; MAX_INTENT_FRAME_BYTES + 1];
    big[0] = 0x02;
    assert_eq!(dec_i(&big), Err(CodecError::FrameTooLarge));
    // the cap is checked before the version, also for a good version byte
    big[0] = 0xA1;
    assert_eq!(dec_i(&big), Err(CodecError::FrameTooLarge));
    // and before trailing bytes
    let mut padded = halt_bytes();
    padded.resize(MAX_INTENT_FRAME_BYTES + 1, 0);
    assert_eq!(dec_i(&padded), Err(CodecError::FrameTooLarge));
    // a 4,096-byte Halt padded with 00 is under the cap: trailing bytes
    let mut padded = halt_bytes();
    padded.resize(MAX_INTENT_FRAME_BYTES, 0);
    assert_eq!(dec_i(&padded), Err(CodecError::Malformed));
    // a 4,096-byte first byte 02 is a version error, not a size error
    let mut v2 = vec![0u8; MAX_INTENT_FRAME_BYTES];
    v2[0] = 0x02;
    assert_eq!(dec_i(&v2), Err(CodecError::UnsupportedVersion));
}

#[test]
fn m2_lane_byte() {
    let mut bytes = halt_bytes();
    bytes[17] = 0x00;
    assert_eq!(dec_i(&bytes), Err(CodecError::WrongDirection));
    for lane in 0..=255u8 {
        let mut b = halt_bytes();
        b[17] = lane;
        if lane == 1 {
            assert!(dec_i(&b).is_ok());
        } else {
            assert_eq!(dec_i(&b), Err(CodecError::WrongDirection), "lane {lane}");
        }
    }
    // lane is checked before the rest is read (truncated after the lane)
    assert_eq!(
        dec_i(&cat(&[&[0xA1], &Z16, &[0x00]])),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        dec_i(&cat(&[&[0xA1], &Z16, &[LANE_SNAPSHOT]])),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        dec_i(&cat(&[&[0xA1], &Z16, &[LANE_SESSION]])),
        Err(CodecError::WrongDirection)
    );
}

#[test]
fn m2_seq_range() {
    let max = max_varint();
    for bad in [vec![0x00u8], max.clone()] {
        let bytes = cat(&[&ihead(Z16, 1, &bad, &[1]), &[1]]);
        assert_eq!(dec_i(&bytes), Err(CodecError::Malformed));
    }
    for good in [vec![0x01u8], leb(SEQ_LAST)] {
        let bytes = cat(&[&ihead(Z16, 1, &good, &[1]), &[1]]);
        assert!(dec_i(&bytes).is_ok());
    }
    // a seq varint overflowing u64
    let mut over = vec![0xFFu8; 9];
    over.push(0x02);
    let bytes = cat(&[&ihead(Z16, 1, &over, &[1]), &[1]]);
    assert_eq!(dec_i(&bytes), Err(CodecError::Malformed));
}

#[test]
fn m2_actor_zero() {
    let bytes = cat(&[&ihead(Z16, 1, &[1], &[0]), &[1]]);
    assert_eq!(dec_i(&bytes), Err(CodecError::Malformed));
    let bytes = cat(&[&ihead(Z16, 1, &[1], &[1]), &[1]]);
    assert!(dec_i(&bytes).is_ok());
    let bytes = cat(&[&ihead(Z16, 1, &[1], &max_varint()), &[1]]);
    assert_eq!(dec_i(&bytes).unwrap().actor, id(u64::MAX));
}

#[test]
fn m2_unknown_tags() {
    let remainder = pt(0, 0, 0);
    for tag in 2..=255u8 {
        let bytes = cat(&[&ihead(Z16, 1, &[1], &[1]), &[tag], &remainder]);
        assert_eq!(dec_i(&bytes), Err(CodecError::UnknownMessage), "tag {tag}");
    }
    // control: tag 00 with the same remainder decodes
    let bytes = cat(&[&ihead(Z16, 1, &[1], &[1]), &[0], &remainder]);
    assert!(dec_i(&bytes).is_ok());
    // the tag alone is enough to refuse it
    for tag in [2u8, 0xFF] {
        let bytes = cat(&[&ihead(Z16, 1, &[1], &[1]), &[tag]]);
        assert_eq!(dec_i(&bytes), Err(CodecError::UnknownMessage));
    }
}

#[test]
fn m2_move_to_point_bounds() {
    // x = L + 1 is Malformed, control L is accepted
    let bad = move_to_bytes(&cat(&[&COORD_L_PLUS_1, &[0x00, 0x00]]));
    assert_eq!(dec_i(&bad), Err(CodecError::Malformed));
    let good = move_to_bytes(&cat(&[&COORD_L, &[0x00, 0x00]]));
    assert!(dec_i(&good).is_ok());
    for pos in 0..3 {
        for coord in [
            COORD_L_PLUS_1,
            COORD_NEG_L_MINUS_1,
            COORD_I32_MIN,
            COORD_OVERFLOW,
        ] {
            let bytes = move_to_bytes(&point_with(pos, &coord));
            assert_eq!(
                dec_i(&bytes),
                Err(CodecError::Malformed),
                "pos {pos} coord {coord:?}"
            );
        }
        for coord in [COORD_L, COORD_NEG_L] {
            assert!(dec_i(&move_to_bytes(&point_with(pos, &coord))).is_ok());
        }
    }
}

#[test]
fn m2_move_to_missing_z_and_halt_trailing() {
    let missing_z = move_to_bytes(&cat(&[&zz(65536), &zz(0)]));
    assert_eq!(dec_i(&missing_z), Err(CodecError::Malformed));
    let with_z = move_to_bytes(&pt(65536, 0, 0));
    assert!(dec_i(&with_z).is_ok());
    let mut halt = halt_bytes();
    halt.push(0x00);
    assert_eq!(dec_i(&halt), Err(CodecError::Malformed));
    let mut move_to = move_to_b();
    move_to.push(0x00);
    assert_eq!(dec_i(&move_to), Err(CodecError::Malformed));
}

#[test]
fn m2_every_truncation_and_trailing_byte() {
    assert_intent_truncation_and_trailing(&halt_bytes());
    assert_intent_truncation_and_trailing(&move_to_b());
    assert_intent_truncation_and_trailing(
        &encode_intent(&intent(
            SEQ_LAST,
            u64::MAX,
            MoveBody::MoveTo {
                destination: p(-L, L, -L),
            },
        ))
        .unwrap(),
    );
}

#[test]
fn m2_truncated_after_each_check_point() {
    // After the version byte: truncated epoch.
    assert_eq!(dec_i(&[0xA1]), Err(CodecError::Malformed));
    assert_eq!(dec_i(&[0xA1, 0, 0, 0]), Err(CodecError::Malformed));
    // 15 epoch bytes.
    assert_eq!(
        dec_i(&cat(&[&[0xA1], &Z16[..15]])),
        Err(CodecError::Malformed)
    );
    // Epoch complete, no lane.
    assert_eq!(dec_i(&cat(&[&[0xA1], &Z16])), Err(CodecError::Malformed));
    // Lane good, no seq.
    assert_eq!(
        dec_i(&cat(&[&[0xA1], &Z16, &[1]])),
        Err(CodecError::Malformed)
    );
    // Seq good, no actor.
    assert_eq!(
        dec_i(&cat(&[&[0xA1], &Z16, &[1, 1]])),
        Err(CodecError::Malformed)
    );
    // Actor good, no tag.
    assert_eq!(
        dec_i(&cat(&[&[0xA1], &Z16, &[1, 1, 1]])),
        Err(CodecError::Malformed)
    );
    // Tag good (MoveTo), no point.
    assert_eq!(
        dec_i(&cat(&[&[0xA1], &Z16, &[1, 1, 1, 0]])),
        Err(CodecError::Malformed)
    );
    // Check order: a bad value is reported before the bytes after it run out.
    assert_eq!(
        dec_i(&cat(&[&[0xA1], &Z16, &[0]])),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        dec_i(&cat(&[&[0xA1], &Z16, &[1, 1, 1, 2]])),
        Err(CodecError::UnknownMessage)
    );
    assert_eq!(
        dec_i(&cat(&[&[0xA1], &Z16, &[1, 1, 1, 0xFF]])),
        Err(CodecError::UnknownMessage)
    );
}

#[test]
fn m2_two_faults_first_wins() {
    // first byte 04 with lane 00 is UnsupportedVersion
    let lane0 = cat(&[&[0x04], &Z16, &[0x00, 0x01, 0x01, 0x01]]);
    assert_eq!(dec_i(&lane0), Err(CodecError::UnsupportedVersion));
    // lane 00 with seq 0 is WrongDirection
    let bytes = cat(&[&[0xA1], &Z16, &[0x00, 0x00, 0x01, 0x01]]);
    assert_eq!(dec_i(&bytes), Err(CodecError::WrongDirection));
    // seq 0 with tag FF is Malformed
    let bytes = cat(&[&[0xA1], &Z16, &[0x01, 0x00, 0x01, 0xFF]]);
    assert_eq!(dec_i(&bytes), Err(CodecError::Malformed));
    // actor 0 with tag FF is Malformed
    let bytes = cat(&[&[0xA1], &Z16, &[0x01, 0x01, 0x00, 0xFF]]);
    assert_eq!(dec_i(&bytes), Err(CodecError::Malformed));
    // tag FF with a trailing byte is UnknownMessage
    let bytes = cat(&[&[0xA1], &Z16, &[0x01, 0x01, 0x01, 0xFF, 0x00]]);
    assert_eq!(dec_i(&bytes), Err(CodecError::UnknownMessage));
    // also: a bad coordinate with a trailing byte is Malformed either way, but
    // a bad version with an oversized frame is FrameTooLarge (cap first)
    let mut big = vec![0x04u8; MAX_INTENT_FRAME_BYTES + 1];
    big[0] = 0x04;
    assert_eq!(dec_i(&big), Err(CodecError::FrameTooLarge));
}

#[test]
fn m2_lane_zero_v4_and_lane_one_are_each_other_s_foreign_bytes() {
    // v4 lane-0 EndTurn: 04 Z16 00 01 07 01 01
    let end_turn = cat(&[&[0x04], &Z16, &[0x00, 0x01, 0x07, 0x01, 0x01]]);
    assert_eq!(end_turn.len(), 22);
    assert_eq!(dec_i(&end_turn), Err(CodecError::UnsupportedVersion));
    // codec_v4 refuses the lane-1 Halt bytes
    assert_eq!(
        codec_v4::decode_intent(&halt_bytes()).err(),
        Some(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        codec_v4::decode_intent(&move_to_b()).err(),
        Some(CodecError::UnsupportedVersion)
    );
    // this decoder refuses the v1/v2/v3 EndTurn too
    for v in [0x01u8, 0x02, 0x03] {
        let bytes = cat(&[&[v], &Z16, &[0x00, 0x01, 0x07, 0x01, 0x01]]);
        assert_eq!(dec_i(&bytes), Err(CodecError::UnsupportedVersion));
    }
}

#[test]
fn m2_encode_seq_checks() {
    assert_eq!(
        encode_intent(&intent(0, 1, MoveBody::Halt)),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        encode_intent(&intent(u64::MAX, 1, MoveBody::Halt)),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        encode_intent(&intent(
            0,
            1,
            MoveBody::MoveTo {
                destination: p(0, 0, 0)
            }
        )),
        Err(CodecError::Malformed)
    );
    assert!(encode_intent(&intent(SEQ_FIRST, 1, MoveBody::Halt)).is_ok());
    assert!(encode_intent(&intent(SEQ_LAST, 1, MoveBody::Halt)).is_ok());
}

// ============================================================== M3 =========

#[test]
fn m3_receipt_literals() {
    // Accepted { 101 }: A1 01 00 Z16 01 01 64 00 65
    let accepted = receipt_std(&[0x00, 0x65]);
    assert_eq!(accepted.len(), 24);
    assert_eq!(accepted[..3], [0xA1, 0x01, 0x00]);
    assert_down_round_trip(
        &MoveDown::Receipt(receipt(
            1,
            1,
            100,
            MoveOutcome::Accepted { start_tick: 101 },
        )),
        &accepted,
    );
    // Superseded: 23 bytes, ends 64 01
    let superseded = receipt_std(&[0x01]);
    assert_eq!(superseded.len(), 23);
    assert_eq!(superseded[21..], [0x64, 0x01]);
    assert_down_round_trip(
        &MoveDown::Receipt(receipt(1, 1, 100, MoveOutcome::Superseded)),
        &superseded,
    );
    // Rejected(IllegalAction): 24 bytes, ends 64 02 12
    let rejected = receipt_std(&[0x02, 0x12]);
    assert_eq!(rejected.len(), 24);
    assert_eq!(rejected[21..], [0x64, 0x02, 0x12]);
    assert_down_round_trip(
        &MoveDown::Receipt(receipt(
            1,
            1,
            100,
            MoveOutcome::Rejected(RejectionCode::IllegalAction),
        )),
        &rejected,
    );
}

#[test]
fn m3_constants_on_the_wire() {
    assert_eq!(DOWN_KIND_RECEIPT, 0);
    assert_eq!(DOWN_KIND_SAMPLE, 1);
    assert_eq!(OUTCOME_TAG_ACCEPTED, 0);
    assert_eq!(OUTCOME_TAG_SUPERSEDED, 1);
    assert_eq!(OUTCOME_TAG_REJECTED, 2);
    let bytes = encode_down(&MoveDown::Receipt(receipt(
        1,
        1,
        100,
        MoveOutcome::Superseded,
    )))
    .unwrap();
    assert_eq!(bytes[0], MOVEMENT_VERSION);
    assert_eq!(bytes[1], LANE_MOVEMENT);
    assert_eq!(bytes[2], DOWN_KIND_RECEIPT);
    assert_eq!(bytes[22], OUTCOME_TAG_SUPERSEDED);
    let bytes = encode_down(&MoveDown::Sample(s1_value())).unwrap();
    assert_eq!(bytes[2], DOWN_KIND_SAMPLE);
}

#[test]
fn m3_every_rejection_code_round_trips() {
    for raw in 1..=30u8 {
        let code = RejectionCode::from_u8(raw).unwrap();
        let m = MoveDown::Receipt(receipt(1, 1, 100, MoveOutcome::Rejected(code)));
        assert_down_round_trip(&m, &receipt_std(&[0x02, raw]));
    }
}

#[test]
fn m3_field_order_with_distinct_values() {
    let epoch: [u8; 16] = core::array::from_fn(|i| 0x10 + i as u8);
    let r = MoveReceipt {
        epoch,
        seq: 300,
        actor: id(70_000),
        processed_tick: 5_000,
        outcome: MoveOutcome::Accepted { start_tick: 9_000 },
    };
    let expected = receipt_with(
        epoch,
        &leb(300),
        &leb(70_000),
        &leb(5_000),
        &cat(&[&[0x00], &leb(9_000)]),
    );
    assert_down_round_trip(&MoveDown::Receipt(r), &expected);
}

#[test]
fn m3_largest_receipt_is_60_bytes() {
    let r = receipt(
        SEQ_LAST,
        u64::MAX,
        u64::MAX,
        MoveOutcome::Accepted {
            start_tick: u64::MAX,
        },
    );
    let bytes = encode_down(&MoveDown::Receipt(r)).unwrap();
    assert_eq!(bytes.len(), 60);
    let expected = receipt_with(
        Z16,
        &leb(SEQ_LAST),
        &leb(u64::MAX),
        &leb(u64::MAX),
        &cat(&[&[0x00], &leb(u64::MAX)]),
    );
    assert_eq!(bytes, expected);
    assert_eq!(dec_d(&bytes), Ok(MoveDown::Receipt(r)));
}

#[test]
fn m3_code_bytes_refused() {
    for code in [0x00u8, 0x1F, 0x20, 0x7F, 0x80, 0xFF] {
        assert_eq!(
            dec_d(&receipt_std(&[0x02, code])),
            Err(CodecError::UnknownMessage),
            "code {code:#x}"
        );
    }
    for code in 31..=255u8 {
        assert_eq!(
            dec_d(&receipt_std(&[0x02, code])),
            Err(CodecError::UnknownMessage)
        );
    }
    // control: 12 decodes
    assert!(dec_d(&receipt_std(&[0x02, 0x12])).is_ok());
    // code is checked before trailing bytes
    assert_eq!(
        dec_d(&receipt_std(&[0x02, 0x1F, 0x00])),
        Err(CodecError::UnknownMessage)
    );
    // a missing code is truncation
    assert_eq!(dec_d(&receipt_std(&[0x02])), Err(CodecError::Malformed));
}

#[test]
fn m3_outcome_tags() {
    // tag 03 followed by 65 is UnknownMessage; control 00 65 is accepted
    assert_eq!(
        dec_d(&receipt_std(&[0x03, 0x65])),
        Err(CodecError::UnknownMessage)
    );
    assert!(dec_d(&receipt_std(&[0x00, 0x65])).is_ok());
    for tag in 3..=255u8 {
        assert_eq!(
            dec_d(&receipt_std(&[tag, 0x65])),
            Err(CodecError::UnknownMessage),
            "tag {tag}"
        );
        // with nothing after it, the tag is still refused as unknown
        assert_eq!(
            dec_d(&receipt_std(&[tag])),
            Err(CodecError::UnknownMessage),
            "tag {tag} alone"
        );
    }
}

#[test]
fn m3_seq_and_actor() {
    let ok = receipt_with(Z16, &[1], &[1], &[0x64], &[0x01]);
    assert!(dec_d(&ok).is_ok());
    for bad_seq in [vec![0x00u8], max_varint()] {
        let bytes = receipt_with(Z16, &bad_seq, &[1], &[0x64], &[0x01]);
        assert_eq!(dec_d(&bytes), Err(CodecError::Malformed));
    }
    let bytes = receipt_with(Z16, &leb(SEQ_LAST), &[1], &[0x64], &[0x01]);
    assert!(dec_d(&bytes).is_ok());
    let bytes = receipt_with(Z16, &[1], &[0x00], &[0x64], &[0x01]);
    assert_eq!(dec_d(&bytes), Err(CodecError::Malformed));
    // processed_tick and start_tick take any u64
    let any = receipt_with(
        Z16,
        &[1],
        &[1],
        &leb(u64::MAX),
        &cat(&[&[0x00], &leb(u64::MAX)]),
    );
    assert!(dec_d(&any).is_ok());
    let zero = receipt_with(Z16, &[1], &[1], &[0x00], &[0x00, 0x00]);
    assert!(dec_d(&zero).is_ok());
}

#[test]
fn m3_truncation_and_trailing() {
    assert_eq!(
        dec_d(&receipt_std(&[0x00])),
        Err(CodecError::Malformed),
        "Accepted without start_tick"
    );
    let mut sup = receipt_std(&[0x01]);
    sup.push(0x00);
    assert_eq!(dec_d(&sup), Err(CodecError::Malformed));
    for tail in [&[0x00u8, 0x65][..], &[0x01][..], &[0x02, 0x12][..]] {
        assert_down_truncation_and_trailing(&receipt_std(tail));
    }
    let big = encode_down(&MoveDown::Receipt(receipt(
        SEQ_LAST,
        u64::MAX,
        u64::MAX,
        MoveOutcome::Accepted {
            start_tick: u64::MAX,
        },
    )))
    .unwrap();
    assert_down_truncation_and_trailing(&big);
}

#[test]
fn m3_encode_seq_check() {
    for seq in [0, u64::MAX] {
        for outcome in [
            MoveOutcome::Accepted { start_tick: 1 },
            MoveOutcome::Superseded,
            MoveOutcome::Rejected(RejectionCode::WrongMode),
        ] {
            assert_eq!(
                encode_down(&MoveDown::Receipt(receipt(seq, 1, 100, outcome))),
                Err(CodecError::Malformed)
            );
        }
    }
    for seq in [SEQ_FIRST, SEQ_LAST] {
        assert!(encode_down(&MoveDown::Receipt(receipt(
            seq,
            1,
            100,
            MoveOutcome::Superseded
        )))
        .is_ok());
    }
}

// ============================================================== M4 =========

#[test]
fn m4_sample_literals() {
    let s1 = s1_bytes();
    assert_eq!(s1.len(), 20);
    assert_down_round_trip(&MoveDown::Sample(s1_value()), &s1);
    // and the helper-built S1 agrees with the literal
    let built = sample_raw(
        &[0x64],
        &[0x01],
        &[0x02],
        &cat(&[
            &entry_bytes(&[1], &pt(65536, 0, -131072), 1),
            &entry_bytes(&[2], &pt(0, 0, 0), 0),
        ]),
    );
    assert_eq!(built, s1);

    let empty = vec![0xA1, 0x01, 0x01, 0x64, 0x00, 0x00];
    assert_down_round_trip(&MoveDown::Sample(sample_of(100, 0, vec![])), &empty);
}

#[test]
fn m4_field_order_with_distinct_values() {
    let s = sample_of(
        70_000,
        300,
        vec![
            SampleEntry {
                entity: id(5),
                point: p(-7, 8, 9),
                moving: false,
            },
            SampleEntry {
                entity: id(1_000),
                point: p(1, 2, 3),
                moving: true,
            },
        ],
    );
    let expected = sample_raw(
        &leb(70_000),
        &leb(300),
        &[0x02],
        &cat(&[
            &entry_bytes(&leb(5), &pt(-7, 8, 9), 0),
            &entry_bytes(&leb(1_000), &pt(1, 2, 3), 1),
        ]),
    );
    assert_down_round_trip(&MoveDown::Sample(s), &expected);
}

#[test]
fn m4_processed_seq_range() {
    let seq_last = sample_raw(&[0x64], &leb(SEQ_LAST), &[0x00], &[]);
    assert_eq!(
        dec_d(&seq_last),
        Ok(MoveDown::Sample(sample_of(100, SEQ_LAST, vec![])))
    );
    assert_down_round_trip(
        &MoveDown::Sample(sample_of(100, SEQ_LAST, vec![])),
        &seq_last,
    );
    let zero = sample_raw(&[0x64], &[0x00], &[0x00], &[]);
    assert_eq!(
        dec_d(&zero),
        Ok(MoveDown::Sample(sample_of(100, 0, vec![])))
    );
    let max = sample_raw(&[0x64], &max_varint(), &[0x00], &[]);
    assert_eq!(dec_d(&max), Err(CodecError::Malformed));
    // a varint overflowing u64
    let mut over = vec![0xFFu8; 9];
    over.push(0x02);
    assert_eq!(
        dec_d(&sample_raw(&[0x64], &over, &[0x00], &[])),
        Err(CodecError::Malformed)
    );
    // server_tick takes any u64
    let any = sample_raw(&max_varint(), &[0x01], &[0x00], &[]);
    assert_eq!(
        dec_d(&any),
        Ok(MoveDown::Sample(sample_of(u64::MAX, 1, vec![])))
    );
}

#[test]
fn m4_largest_sample_is_1012_bytes() {
    let s = sample_of(u64::MAX, SEQ_LAST, maxima_entries(38));
    assert_eq!(s.entries[0].entity, id(u64::MAX - 37));
    assert_eq!(s.entries[37].entity, id(u64::MAX));
    let bytes = encode_down(&MoveDown::Sample(s.clone())).unwrap();
    assert_eq!(bytes.len(), 1012);
    assert!(bytes.len() <= MAX_MOVE_DOWN_BYTES);
    let body: Vec<u8> = s
        .entries
        .iter()
        .flat_map(|e| entry_bytes(&leb(e.entity.get()), &pt(L, L, L), 1))
        .collect();
    let expected = sample_raw(&leb(u64::MAX), &leb(SEQ_LAST), &[38], &body);
    assert_eq!(bytes, expected);
    assert_eq!(dec_d(&bytes), Ok(MoveDown::Sample(s)));
}

#[test]
fn m4_count_bounds() {
    // 39 entries claimed, nothing following: LimitExceeded before any entry
    let bytes = vec![0xA1, 0x01, 0x01, 0x64, 0x01, 0x27];
    assert_eq!(dec_d(&bytes), Err(CodecError::LimitExceeded));
    // 38 claimed, nothing following: truncation
    let bytes = vec![0xA1, 0x01, 0x01, 0x64, 0x01, 0x26];
    assert_eq!(dec_d(&bytes), Err(CodecError::Malformed));
    // 39 claimed with a bad first entry (entity 0): the count wins
    let mut bytes = vec![0xA1, 0x01, 0x01, 0x64, 0x01, 0x27];
    bytes.extend(entry_bytes(&[0], &pt(0, 0, 0), 0));
    assert_eq!(dec_d(&bytes), Err(CodecError::LimitExceeded));
    // 39 well-formed ascending entries are still refused
    let body: Vec<u8> = (1..=39u64)
        .flat_map(|e| entry_bytes(&leb(e), &pt(0, 0, 0), 0))
        .collect();
    assert_eq!(
        dec_d(&sample_raw(&[0x64], &[0x01], &[0x27], &body)),
        Err(CodecError::LimitExceeded)
    );
    // a larger count, two-byte varint
    assert_eq!(
        dec_d(&sample_raw(&[0x64], &[0x01], &[0x80, 0x01], &[])),
        Err(CodecError::LimitExceeded)
    );
    // a count varint overflowing u64 is Malformed, not LimitExceeded
    let mut over = vec![0xFFu8; 9];
    over.push(0x02);
    assert_eq!(
        dec_d(&sample_raw(&[0x64], &[0x01], &over, &[])),
        Err(CodecError::Malformed)
    );
    // count 0 is valid; count 0 with an entry after it is trailing bytes
    assert!(dec_d(&sample_raw(&[0x64], &[0x01], &[0x00], &[])).is_ok());
    let tail = entry_bytes(&[1], &pt(0, 0, 0), 0);
    assert_eq!(
        dec_d(&sample_raw(&[0x64], &[0x01], &[0x00], &tail)),
        Err(CodecError::Malformed)
    );
    // the count is read before the entries: 1 claimed, 2 present
    let two = cat(&[
        &entry_bytes(&[1], &pt(0, 0, 0), 0),
        &entry_bytes(&[2], &pt(0, 0, 0), 0),
    ]);
    assert_eq!(
        dec_d(&sample_raw(&[0x64], &[0x01], &[0x01], &two)),
        Err(CodecError::Malformed)
    );
}

#[test]
fn m4_entity_order() {
    let enc = |ids: &[u64]| {
        let body: Vec<u8> = ids
            .iter()
            .flat_map(|&e| entry_bytes(&leb(e), &pt(0, 0, 0), 0))
            .collect();
        sample_raw(&[0x64], &[0x01], &[ids.len() as u8], &body)
    };
    assert!(dec_d(&enc(&[1, 2])).is_ok());
    assert_eq!(dec_d(&enc(&[2, 1])), Err(CodecError::Malformed));
    assert_eq!(dec_d(&enc(&[1, 1])), Err(CodecError::Malformed));
    assert!(dec_d(&enc(&[1, 3, 4])).is_ok());
    assert_eq!(dec_d(&enc(&[1, 3, 2])), Err(CodecError::Malformed));
    assert_eq!(dec_d(&enc(&[1, 3, 3])), Err(CodecError::Malformed));
    assert_eq!(dec_d(&enc(&[3, 2, 1])), Err(CodecError::Malformed));
    assert!(dec_d(&enc(&[u64::MAX - 1, u64::MAX])).is_ok());
    assert_eq!(
        dec_d(&enc(&[u64::MAX, u64::MAX])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        dec_d(&enc(&[u64::MAX, u64::MAX - 1])),
        Err(CodecError::Malformed)
    );
}

#[test]
fn m4_bad_entry_fields_before_a_valid_second_entry() {
    let second = entry_bytes(&[2], &pt(0, 0, 0), 0);
    let build = |first: Vec<u8>| sample_raw(&[0x64], &[0x01], &[0x02], &cat(&[&first, &second]));
    // control
    assert!(dec_d(&build(entry_bytes(&[1], &pt(0, 0, 0), 0))).is_ok());
    // entity 0
    assert_eq!(
        dec_d(&build(entry_bytes(&[0], &pt(0, 0, 0), 0))),
        Err(CodecError::Malformed)
    );
    // x = L + 1, control x = L
    assert_eq!(
        dec_d(&build(entry_bytes(
            &[1],
            &point_with(0, &COORD_L_PLUS_1),
            0
        ))),
        Err(CodecError::Malformed)
    );
    assert!(dec_d(&build(entry_bytes(&[1], &point_with(0, &COORD_L), 0))).is_ok());
    // moving byte 02, control 00 and 01
    assert_eq!(
        dec_d(&build(entry_bytes(&[1], &pt(0, 0, 0), 2))),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        dec_d(&build(entry_bytes(&[1], &pt(0, 0, 0), 0xFF))),
        Err(CodecError::Malformed)
    );
    assert!(dec_d(&build(entry_bytes(&[1], &pt(0, 0, 0), 1))).is_ok());
    // the same faults in the second entry, with a valid first one
    let first = entry_bytes(&[1], &pt(0, 0, 0), 0);
    for bad in [
        entry_bytes(&[0], &pt(0, 0, 0), 0),
        entry_bytes(&[2], &point_with(1, &COORD_L_PLUS_1), 0),
        entry_bytes(&[2], &point_with(2, &COORD_NEG_L_MINUS_1), 1),
        entry_bytes(&[2], &point_with(2, &COORD_OVERFLOW), 1),
        entry_bytes(&[2], &pt(0, 0, 0), 2),
    ] {
        let bytes = sample_raw(&[0x64], &[0x01], &[0x02], &cat(&[&first, &bad]));
        assert_eq!(dec_d(&bytes), Err(CodecError::Malformed));
    }
}

#[test]
fn m4_point_bounds_in_each_position() {
    for pos in 0..3 {
        for coord in [COORD_L, COORD_NEG_L] {
            let bytes = sample_raw(
                &[0x64],
                &[0x01],
                &[0x01],
                &entry_bytes(&[1], &point_with(pos, &coord), 1),
            );
            assert!(dec_d(&bytes).is_ok(), "pos {pos}");
        }
        for coord in [
            COORD_L_PLUS_1,
            COORD_NEG_L_MINUS_1,
            COORD_I32_MIN,
            COORD_OVERFLOW,
        ] {
            let bytes = sample_raw(
                &[0x64],
                &[0x01],
                &[0x01],
                &entry_bytes(&[1], &point_with(pos, &coord), 1),
            );
            assert_eq!(dec_d(&bytes), Err(CodecError::Malformed), "pos {pos}");
        }
    }
}

#[test]
fn m4_truncation_and_trailing() {
    assert_down_truncation_and_trailing(&s1_bytes());
    assert_down_truncation_and_trailing(&[0xA1, 0x01, 0x01, 0x64, 0x00, 0x00]);
    let max = encode_down(&MoveDown::Sample(sample_of(
        u64::MAX,
        SEQ_LAST,
        maxima_entries(38),
    )))
    .unwrap();
    assert_down_truncation_and_trailing(&max);
}

// ============================================================== M5 =========

#[test]
fn m5_kind_lane_version_refusals() {
    let s1 = s1_bytes();
    for kind in 2..=255u8 {
        let mut b = s1.clone();
        b[2] = kind;
        assert_eq!(dec_d(&b), Err(CodecError::UnknownMessage), "kind {kind}");
    }
    // control kind 01 decodes
    assert!(dec_d(&s1).is_ok());
    // lane 00 before S1's kind and body
    let mut b = s1.clone();
    b[1] = 0x00;
    assert_eq!(dec_d(&b), Err(CodecError::WrongDirection));
    for lane in 0..=255u8 {
        let mut b = s1.clone();
        b[1] = lane;
        if lane == 1 {
            assert!(dec_d(&b).is_ok());
        } else {
            assert_eq!(dec_d(&b), Err(CodecError::WrongDirection), "lane {lane}");
        }
    }
    // codec_v4::decode_delta(S1) is UnsupportedVersion
    assert_eq!(
        codec_v4::decode_delta(&s1).err(),
        Some(CodecError::UnsupportedVersion)
    );
    // every first byte but A1 is UnsupportedVersion
    for v in 0..=255u8 {
        let mut b = s1.clone();
        b[0] = v;
        if v == 0xA1 {
            assert!(dec_d(&b).is_ok());
        } else {
            assert_eq!(dec_d(&b), Err(CodecError::UnsupportedVersion), "{v:#x}");
        }
    }
    for v in [0x03u8, 0x04] {
        let mut b = s1.clone();
        b[0] = v;
        assert_eq!(dec_d(&b), Err(CodecError::UnsupportedVersion));
    }
    // a bare version byte is judged before truncation
    for v in [0x03u8, 0x04, 0x81, 0xA2] {
        assert_eq!(dec_d(&[v]), Err(CodecError::UnsupportedVersion));
    }
    // v3 and v4 empty-delta bytes
    assert_eq!(
        dec_d(&[0x04, 0x00, 0x64, 0x01, 0x00]),
        Err(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        dec_d(&[0x03, 0x00, 0x64, 0x01, 0x00]),
        Err(CodecError::UnsupportedVersion)
    );
}

#[test]
fn m5_empty_cap_and_padding() {
    assert_eq!(dec_d(&[]), Err(CodecError::Malformed));
    assert_eq!(MAX_MOVE_DOWN_BYTES, 1_024);
    let mut big = vec![0u8; 1_025];
    big[0] = 0x02;
    assert_eq!(dec_d(&big), Err(CodecError::FrameTooLarge));
    big[0] = 0xA1;
    assert_eq!(dec_d(&big), Err(CodecError::FrameTooLarge));
    // S1 padded to 1,025 is a size error; padded to 1,024 is trailing bytes
    let mut padded = s1_bytes();
    padded.resize(1_025, 0);
    assert_eq!(dec_d(&padded), Err(CodecError::FrameTooLarge));
    let mut padded = s1_bytes();
    padded.resize(1_024, 0);
    assert_eq!(dec_d(&padded), Err(CodecError::Malformed));
    // S1 plus one byte
    let mut plus = s1_bytes();
    plus.push(0x00);
    assert_eq!(dec_d(&plus), Err(CodecError::Malformed));
    // exactly 1,024 bytes starting 02 is a version error
    let mut v2 = vec![0u8; 1_024];
    v2[0] = 0x02;
    assert_eq!(dec_d(&v2), Err(CodecError::UnsupportedVersion));
}

#[test]
fn m5_truncated_after_each_check_point() {
    assert_eq!(dec_d(&[0xA1]), Err(CodecError::Malformed));
    assert_eq!(dec_d(&[0xA1, 0x01]), Err(CodecError::Malformed));
    assert_eq!(dec_d(&[0xA1, 0x01, 0x00]), Err(CodecError::Malformed));
    assert_eq!(dec_d(&[0xA1, 0x01, 0x01]), Err(CodecError::Malformed));
    // lane is judged before kind is read; kind before the body is read
    assert_eq!(dec_d(&[0xA1, 0x00]), Err(CodecError::WrongDirection));
    assert_eq!(dec_d(&[0xA1, 0x02]), Err(CodecError::WrongDirection));
    assert_eq!(dec_d(&[0xA1, 0x01, 0x02]), Err(CodecError::UnknownMessage));
    assert_eq!(dec_d(&[0xA1, 0x01, 0xFF]), Err(CodecError::UnknownMessage));
    // lane 02 (snapshot), lane 03 (session)
    assert_eq!(
        dec_d(&[0xA1, LANE_SNAPSHOT, 0x01]),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        dec_d(&[0xA1, LANE_SESSION, 0x00]),
        Err(CodecError::WrongDirection)
    );
    // receipt: truncated at each field
    let r = receipt_std(&[0x00, 0x65]);
    for n in 3..r.len() {
        assert_eq!(dec_d(&r[..n]), Err(CodecError::Malformed), "prefix {n}");
    }
}

#[test]
fn m5_two_faults() {
    // lane 00 with kind FF is WrongDirection
    assert_eq!(dec_d(&[0xA1, 0x00, 0xFF]), Err(CodecError::WrongDirection));
    let mut b = s1_bytes();
    b[1] = 0x00;
    b[2] = 0xFF;
    assert_eq!(dec_d(&b), Err(CodecError::WrongDirection));
    // first byte 04 with lane 00 is UnsupportedVersion
    let mut b = s1_bytes();
    b[0] = 0x04;
    b[1] = 0x00;
    assert_eq!(dec_d(&b), Err(CodecError::UnsupportedVersion));
    // kind FF with S1's body is UnknownMessage
    let mut b = s1_bytes();
    b[2] = 0xFF;
    assert_eq!(dec_d(&b), Err(CodecError::UnknownMessage));
    // receipt with seq 0 and code 1F is Malformed
    assert_eq!(
        dec_d(&receipt_with(Z16, &[0x00], &[1], &[0x64], &[0x02, 0x1F])),
        Err(CodecError::Malformed)
    );
    // receipt with seq 0 and outcome tag 03 is Malformed
    assert_eq!(
        dec_d(&receipt_with(Z16, &[0x00], &[1], &[0x64], &[0x03, 0x65])),
        Err(CodecError::Malformed)
    );
    // receipt with actor 0 and outcome tag 03 is Malformed
    assert_eq!(
        dec_d(&receipt_with(Z16, &[1], &[0x00], &[0x64], &[0x03, 0x65])),
        Err(CodecError::Malformed)
    );
    // sample with processed_seq u64::MAX and count 39 is Malformed;
    // control: count 39 alone is LimitExceeded
    assert_eq!(
        dec_d(&sample_raw(&[0x64], &max_varint(), &[0x27], &[])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        dec_d(&sample_raw(&[0x64], &[0x01], &[0x27], &[])),
        Err(CodecError::LimitExceeded)
    );
    // a 1,025-byte frame with a bad lane is a size error
    let mut big = vec![0u8; 1_025];
    big[0] = 0xA1;
    big[1] = 0x00;
    assert_eq!(dec_d(&big), Err(CodecError::FrameTooLarge));
}

#[test]
fn m5_other_codecs_refuse_the_movement_version() {
    for bytes in [s1_bytes(), halt_bytes(), move_to_b()] {
        let b = bytes.as_slice();
        let want = Some(CodecError::UnsupportedVersion);
        assert_eq!(codec::decode_intent(b).err(), want);
        assert_eq!(codec::decode_delta(b).err(), want);
        assert_eq!(codec_v2::decode_intent(b).err(), want);
        assert_eq!(codec_v2::decode_delta(b).err(), want);
        assert_eq!(codec_v3::decode_intent(b).err(), want);
        assert_eq!(codec_v3::decode_delta(b).err(), want);
        assert_eq!(codec_v4::decode_intent(b).err(), want);
        assert_eq!(codec_v4::decode_delta(b).err(), want);
        assert_eq!(codec_transfer::decode_down(b).err(), want);
        assert_eq!(codec_transfer::decode_up(b).err(), want);
        assert_eq!(codec_resume::decode_request(b).err(), want);
        assert_eq!(codec_resume::decode_down(b).err(), want);
    }
    // a receipt too
    let r = receipt_std(&[0x01]);
    assert_eq!(
        codec_v4::decode_delta(&r).err(),
        Some(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        codec_v3::decode_delta(&r).err(),
        Some(CodecError::UnsupportedVersion)
    );
}

// ============================================================== M6 =========

#[test]
fn m6_sample_encode_checks() {
    let asc = |n: u64| entries_of(&(1..=n).collect::<Vec<_>>());
    let desc = |n: u64| entries_of(&(1..=n).rev().collect::<Vec<_>>());
    let cases: Vec<(MoveSample, Result<(), CodecError>)> = vec![
        // valid
        (sample_of(100, 0, vec![]), Ok(())),
        (sample_of(100, SEQ_LAST, asc(38)), Ok(())),
        (sample_of(u64::MAX, 1, entries_of(&[1, 2])), Ok(())),
        // 39 entries: LimitExceeded
        (sample_of(100, 1, asc(39)), Err(CodecError::LimitExceeded)),
        (sample_of(100, 1, asc(100)), Err(CodecError::LimitExceeded)),
        // entities [2, 1]: Malformed
        (
            sample_of(100, 1, entries_of(&[2, 1])),
            Err(CodecError::Malformed),
        ),
        (
            sample_of(100, 1, entries_of(&[1, 1])),
            Err(CodecError::Malformed),
        ),
        (
            sample_of(100, 1, entries_of(&[1, 3, 2])),
            Err(CodecError::Malformed),
        ),
        // processed_seq u64::MAX: Malformed
        (sample_of(100, u64::MAX, vec![]), Err(CodecError::Malformed)),
        (sample_of(100, u64::MAX, asc(2)), Err(CodecError::Malformed)),
        // u64::MAX with 39 descending entries: Malformed (first check)
        (
            sample_of(100, u64::MAX, desc(39)),
            Err(CodecError::Malformed),
        ),
        // u64::MAX with 39 ascending entries: Malformed (before the count)
        (
            sample_of(100, u64::MAX, asc(39)),
            Err(CodecError::Malformed),
        ),
        // 39 descending with a valid processed_seq: count before order
        (sample_of(100, 1, desc(39)), Err(CodecError::LimitExceeded)),
    ];
    for (i, (sample, want)) in cases.into_iter().enumerate() {
        assert_eq!(check_sample(&sample), want, "case {i} check_sample");
        let enc = encode_down(&MoveDown::Sample(sample));
        match want {
            Ok(()) => {
                let bytes = enc.unwrap_or_else(|e| panic!("case {i}: {e:?}"));
                assert!(bytes.len() <= MAX_MOVE_DOWN_BYTES);
            }
            Err(e) => assert_eq!(enc, Err(e), "case {i} encode_down"),
        }
    }
}

#[test]
fn m6_receipt_seq_zero_is_malformed() {
    assert_eq!(
        encode_down(&MoveDown::Receipt(receipt(
            0,
            1,
            100,
            MoveOutcome::Superseded
        ))),
        Err(CodecError::Malformed)
    );
}

#[test]
fn m6_encoded_samples_round_trip() {
    for n in [0u64, 1, 2, 37, 38] {
        let ids: Vec<u64> = (0..n).map(|i| 1 + i * 3).collect();
        let s = sample_of(123_456, 99, entries_of(&ids));
        let bytes = encode_down(&MoveDown::Sample(s.clone())).unwrap();
        assert_eq!(encode_down(&MoveDown::Sample(s.clone())).unwrap(), bytes);
        assert_eq!(dec_d(&bytes), Ok(MoveDown::Sample(s)));
    }
}

// ============================================================== M7 =========

fn down_sample(bytes: &[u8]) -> MoveSample {
    match decode_down(bytes).unwrap() {
        MoveDown::Sample(s) => s,
        other => panic!("not a sample: {other:?}"),
    }
}

#[test]
fn m7_empty_input() {
    assert_eq!(sample_datagrams(100, 1, &[]), Ok(vec![]));
    assert_eq!(sample_datagrams(100, 0, &[]), Ok(vec![]));
    assert_eq!(sample_datagrams(u64::MAX, SEQ_LAST, &[]), Ok(vec![]));
    // empty input is checked first, even with an invalid processed_seq
    assert_eq!(sample_datagrams(100, u64::MAX, &[]), Ok(vec![]));
}

#[test]
fn m7_exact_chunk_boundaries() {
    let ids = |n: u64| (1..=n).collect::<Vec<_>>();
    // 1 and 38 entries: one datagram, equal to encode_down of the whole sample
    for n in [1u64, 2, 37, 38] {
        let entries = entries_of(&ids(n));
        let got = sample_datagrams(100, 1, &entries).unwrap();
        assert_eq!(got.len(), 1, "{n} entries");
        let whole = encode_down(&MoveDown::Sample(sample_of(100, 1, entries.clone()))).unwrap();
        assert_eq!(got[0], whole);
    }
    // 39: two datagrams (38, then 1); 76: (38, 38); 77: (38, 38, 1); 114: 3 full
    for (n, shape) in [
        (39u64, vec![38usize, 1]),
        (76, vec![38, 38]),
        (77, vec![38, 38, 1]),
        (114, vec![38, 38, 38]),
        (115, vec![38, 38, 38, 1]),
    ] {
        let entries = entries_of(&ids(n));
        let got = sample_datagrams(100, 7, &entries).unwrap();
        assert_eq!(got.len(), shape.len(), "{n} entries");
        let mut start = 0usize;
        for (d, &len) in got.iter().zip(&shape) {
            let chunk = entries[start..start + len].to_vec();
            let want = encode_down(&MoveDown::Sample(sample_of(100, 7, chunk.clone()))).unwrap();
            assert_eq!(d, &want, "{n} entries, chunk at {start}");
            let decoded = down_sample(d);
            assert_eq!(decoded.server_tick, 100);
            assert_eq!(decoded.processed_seq, 7);
            assert_eq!(decoded.entries, chunk);
            start += len;
        }
        assert_eq!(start, entries.len());
    }
}

#[test]
fn m7_thirty_nine_gives_38_then_1() {
    let entries = entries_of(&(1..=39).collect::<Vec<_>>());
    let got = sample_datagrams(5, 2, &entries).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(down_sample(&got[0]).entries, entries[..38].to_vec());
    assert_eq!(down_sample(&got[1]).entries, entries[38..].to_vec());
    assert_eq!(down_sample(&got[1]).entries.len(), 1);
}

#[test]
fn m7_full_tick_of_1024_maxima_entries() {
    let entries = maxima_entries(1_024);
    assert_eq!(entries[0].entity, id(u64::MAX - 1023));
    assert_eq!(entries[1023].entity, id(u64::MAX));
    let got = sample_datagrams(u64::MAX, SEQ_LAST, &entries).unwrap();
    assert_eq!(got.len(), 27);
    let mut all = Vec::new();
    for (i, d) in got.iter().enumerate() {
        assert!(d.len() <= 1_012, "datagram {i} is {} bytes", d.len());
        assert!(d.len() <= MAX_MOVE_DOWN_BYTES);
        let s = down_sample(d);
        assert_eq!(s.server_tick, u64::MAX);
        assert_eq!(s.processed_seq, SEQ_LAST);
        let want_len = if i < 26 { 38 } else { 36 };
        assert_eq!(s.entries.len(), want_len, "datagram {i}");
        all.extend(s.entries);
    }
    assert_eq!(all, entries);
    // 26 full datagrams are exactly 1,012 bytes
    for d in &got[..26] {
        assert_eq!(d.len(), 1_012);
    }
}

#[test]
fn m7_varied_values_keep_order_and_content() {
    let ids: Vec<u64> = (0..200u64).map(|i| 3 + i * 5).collect();
    let entries = entries_of(&ids);
    let got = sample_datagrams(777, 42, &entries).unwrap();
    assert_eq!(got.len(), 6); // 5 * 38 + 10
    let mut all = Vec::new();
    for d in &got {
        let s = down_sample(d);
        assert_eq!((s.server_tick, s.processed_seq), (777, 42));
        all.extend(s.entries);
    }
    assert_eq!(all, entries);
}

#[test]
fn m7_refusals_give_no_datagrams() {
    // entities [1, 3, 2] with processed_seq u64::MAX: Malformed
    assert_eq!(
        sample_datagrams(100, u64::MAX, &entries_of(&[1, 3, 2])),
        Err(CodecError::Malformed)
    );
    // processed_seq alone
    assert_eq!(
        sample_datagrams(100, u64::MAX, &entries_of(&[1, 2, 3])),
        Err(CodecError::Malformed)
    );
    // ordering alone, with a valid processed_seq
    assert_eq!(
        sample_datagrams(100, 1, &entries_of(&[1, 3, 2])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        sample_datagrams(100, 1, &entries_of(&[2, 2])),
        Err(CodecError::Malformed)
    );
    // controls
    assert!(sample_datagrams(100, 1, &entries_of(&[1, 2, 3])).is_ok());
    assert!(sample_datagrams(100, SEQ_LAST, &entries_of(&[1, 2, 3])).is_ok());
    assert!(sample_datagrams(100, 0, &entries_of(&[1, 2, 3])).is_ok());
    // the order is checked over the whole input, not per chunk: 38 ascending
    // entries then an entity below the 38th
    let mut ids: Vec<u64> = (1..=38).collect();
    ids.push(5);
    assert_eq!(
        sample_datagrams(100, 1, &entries_of(&ids)),
        Err(CodecError::Malformed)
    );
    // equal across the chunk boundary
    let mut ids: Vec<u64> = (1..=38).collect();
    ids.push(38);
    assert_eq!(
        sample_datagrams(100, 1, &entries_of(&ids)),
        Err(CodecError::Malformed)
    );
    // a descent in the last chunk of a full tick
    let mut ids: Vec<u64> = (1..=1_024).collect();
    ids.swap(1_022, 1_023);
    assert_eq!(
        sample_datagrams(100, 1, &entries_of(&ids)),
        Err(CodecError::Malformed)
    );
    // control for the above
    let ids: Vec<u64> = (1..=1_024).collect();
    assert_eq!(
        sample_datagrams(100, 1, &entries_of(&ids)).unwrap().len(),
        27
    );
    // more than 38 entries is not an error for the splitter
    assert_eq!(
        sample_datagrams(100, 1, &entries_of(&(1..=39).collect::<Vec<_>>()))
            .unwrap()
            .len(),
        2
    );
}

// ============================================================== M8 =========

#[test]
fn m8_movement_constants() {
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
    // 38 is the largest count that fits
    const {
        assert!(
            MAX_SAMPLE_HEADER_BYTES + MAX_SAMPLE_ENTRIES * MAX_SAMPLE_ENTRY_BYTES
                <= MAX_MOVE_DOWN_BYTES
        );
        assert!(
            MAX_SAMPLE_HEADER_BYTES + (MAX_SAMPLE_ENTRIES + 1) * MAX_SAMPLE_ENTRY_BYTES
                > MAX_MOVE_DOWN_BYTES
        );
    }
    assert_eq!(
        MAX_SAMPLE_HEADER_BYTES + MAX_SAMPLE_ENTRIES * MAX_SAMPLE_ENTRY_BYTES,
        1_012
    );
}

#[test]
fn m8_policy_constants() {
    let per_sec: u64 = POLICY_MOVE_INTENTS_PER_SEC;
    let burst: u64 = POLICY_MOVE_INTENT_BURST;
    let in_flight: usize = POLICY_MOVE_IN_FLIGHT;
    assert_eq!(per_sec, 20);
    assert_eq!(burst, 40);
    assert_eq!(in_flight, 8);
    assert!(in_flight <= POLICY_RETRY_CACHE_PER_PEER_LANE);
}

#[test]
fn m8_lane_numbers_are_distinct() {
    assert_eq!(LANE_COMBAT, 0);
    assert_eq!(LANE_MOVEMENT, 1);
    assert_eq!(LANE_SNAPSHOT, 2);
    assert_eq!(LANE_SESSION, 3);
    assert_ne!(LANE_MOVEMENT, LANE_COMBAT);
    assert_ne!(LANE_MOVEMENT, LANE_SNAPSHOT);
    assert_ne!(LANE_MOVEMENT, LANE_SESSION);
}
