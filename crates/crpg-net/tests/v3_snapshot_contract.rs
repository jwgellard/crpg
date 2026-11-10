//! T140d two-keys suite: the `0x82` snapshot, written from `tasks/T140.md`
//! (Behaviour D-1..D-6, Tests D1..D10) and the public API only.
//!
//! Expected bytes are literals from the contract or hand arithmetic from the
//! D-2 layout, never produced by the code under test. Every rejection has an
//! exact error and a passing control.

use std::sync::OnceLock;

use crpg_core::{GenerationalArena, Ulid};
use crpg_net::codec::CodecError;
use crpg_net::codec::CodecError::{
    FrameTooLarge, LimitExceeded, Malformed, UnknownMessage, UnsupportedVersion, WrongDirection,
};
use crpg_net::codec_transfer as v1;
use crpg_net::codec_v3;
use crpg_net::facts_v3::{
    project_page, project_snapshot_v3, EncounterGrants, EncounterInput, FactContext, FactError,
    FactMask, FactRow, FactView, OrderInput, SnapshotProjectionV3,
};
use crpg_net::interest::{
    EntityFact, EntityKey, FieldMask, InterestError, InterestSet, InterestView, TurnState,
    ViewCursor, ViewerContext,
};
use crpg_net::protocol::{self, SEQ_LAST};
use crpg_net::protocol_transfer::{
    BeginTransfer, CancelReason, HealthFact, ResyncReason, TransferChunk, TransferDown, TransferId,
    TransferUp, TurnFact,
};
use crpg_net::protocol_v3::{
    ClockFact, DeltaOp, EffectFact, EncounterFacts, EncounterMode, FactSet, NetId, OrderFact,
    PoolFact, SessionEpoch, StatFact, StatValue, TimelineEntry,
};
use crpg_net::transfer_v3 as t3;
use crpg_net::transfer_v3::{SnapshotBodyV3, SnapshotEntityV3};

type R<T> = Result<T, CodecError>;
const Z: [u8; 16] = [0; 16];

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

/// Hex bytes; the token `Z16` is sixteen zero bytes.
fn hx(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .flat_map(|t| {
            if t == "Z16" {
                vec![0u8; 16]
            } else {
                vec![u8::from_str_radix(t, 16).expect("hex token")]
            }
        })
        .collect()
}

fn vi(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

fn zz(v: i32) -> u64 {
    u64::from(((v << 1) ^ (v >> 31)) as u32)
}

fn tid(n: u64) -> TransferId {
    TransferId::new(n).expect("nonzero transfer id")
}

fn nid(n: u64) -> NetId {
    NetId::new(n).expect("nonzero net id")
}

fn u(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

/// ULID text as the wire holds it: length 26, then 26 ASCII bytes.
fn ut(n: u128) -> Vec<u8> {
    let text = u(n).to_string();
    assert_eq!(text.len(), 26);
    let mut out = vi(26);
    out.extend(text.bytes());
    out
}

fn string_bytes(s: &str) -> Vec<u8> {
    let mut out = vi(s.len() as u64);
    out.extend(s.bytes());
    out
}

fn as_v1(frame: &[u8]) -> Vec<u8> {
    let mut copy = frame.to_vec();
    copy[0] = 0x81;
    copy
}

fn begin(total: u32, ent: u16, spc: u8, chunks: u8) -> BeginTransfer {
    BeginTransfer {
        epoch: Z,
        transfer: tid(1),
        base_tick: 0,
        cursor: 0,
        total_bytes: total,
        entity_count: ent,
        segments_per_chunk: spc,
        chunk_count: chunks,
    }
}

fn chunk(index: u8, bytes: Vec<u8>) -> TransferChunk {
    TransferChunk {
        epoch: Z,
        transfer: tid(1),
        index,
        bytes,
    }
}

/// Begin fields as raw wire numbers, so a value past the struct's width or
/// past a cap can be written.
#[derive(Clone, Copy)]
struct B {
    transfer: u64,
    cursor: u64,
    total: u64,
    ent: u64,
    spc: u8,
    chunks: u8,
}

const OK: B = B {
    transfer: 1,
    cursor: 0,
    total: 6,
    ent: 0,
    spc: 1,
    chunks: 1,
};

fn bb(first: u8, b: B) -> Vec<u8> {
    let mut out = vec![first, 2, 0x10];
    out.extend(Z);
    out.extend(vi(b.transfer));
    out.extend(vi(0));
    out.extend(vi(b.cursor));
    out.extend(vi(b.total));
    out.extend(vi(b.ent));
    out.push(b.spc);
    out.push(b.chunks);
    out
}

/// A Chunk frame with a raw segment section.
fn cb(first: u8, transfer: u64, index: u8, count: u8, segs: &[u8]) -> Vec<u8> {
    let mut out = vec![first, 2, 0x11];
    out.extend(Z);
    out.extend(vi(transfer));
    out.push(index);
    out.push(count);
    out.extend(segs);
    out
}

/// One segment: length varint, then that many bytes.
fn seg(len: usize) -> Vec<u8> {
    let mut out = vi(len as u64);
    out.extend(vec![0xAB; len]);
    out
}

fn cancel_frame(first: u8, lane: u8, tag: u8) -> Vec<u8> {
    let mut out = vec![first, lane, tag];
    out.extend(Z);
    out.extend([1, 1]);
    out
}

fn big(first: u8, len: usize) -> Vec<u8> {
    let mut out = vec![first, 2, 0xFF];
    out.resize(len, 0);
    out
}

fn empty_body() -> SnapshotBodyV3 {
    SnapshotBodyV3 {
        entities: Vec::new(),
        turn: None,
        encounter: EncounterFacts::default(),
        timeline: None,
    }
}

// ---------------------------------------------------------------------------
// D1 / D2 / D10: frames.
// ---------------------------------------------------------------------------

#[test]
fn d1_frame_literals_0x82() {
    let begin_lit = hx("82 02 10 Z16 01 00 00 06 00 01 01");
    let chunk_lit = hx("82 02 11 Z16 01 00 01 06 00 00 00 00 00 00");
    let begin_msg = TransferDown::Begin(begin(6, 0, 1, 1));
    let chunk_msg = TransferDown::Chunk(chunk(0, vec![0; 6]));
    let cancel = |reason| TransferDown::Cancel {
        epoch: Z,
        transfer: tid(1),
        reason,
    };
    // (literal, message, length, control index, control byte)
    let down = [
        (begin_lit.clone(), begin_msg, 26, 20, 5),
        (chunk_lit.clone(), chunk_msg, 29, 28, 7),
        (
            hx("82 02 12 Z16 01 01"),
            cancel(CancelReason::Superseded),
            21,
            20,
            2,
        ),
        (
            hx("82 02 12 Z16 01 02"),
            cancel(CancelReason::Aborted),
            21,
            20,
            1,
        ),
    ];
    for (lit, msg, len, at, new) in down {
        assert_eq!(lit.len(), len);
        assert_eq!(t3::encode_down(&msg), Ok(lit.clone()));
        assert_eq!(t3::decode_down(&lit), Ok(msg.clone()));
        let v1_bytes = v1::encode_down(&msg).unwrap();
        assert_eq!(v1_bytes[0], 0x81);
        assert_eq!(v1_bytes[1..], lit[1..]);
        let mut ctrl = lit.clone();
        ctrl[at] = new;
        let got = t3::decode_down(&ctrl);
        assert!(got.is_ok() && got != Ok(msg), "control differs in one byte");
    }
    let up = [
        (
            hx("82 02 20 Z16 01"),
            TransferUp::Published {
                epoch: Z,
                transfer: tid(1),
            },
            20,
            19,
            2,
        ),
        (
            hx("82 02 21 Z16 00 01"),
            TransferUp::Resync {
                epoch: Z,
                after: None,
                reason: ResyncReason::Expired,
            },
            21,
            20,
            2,
        ),
        (
            hx("82 02 21 Z16 02 03"),
            TransferUp::Resync {
                epoch: Z,
                after: Some(tid(2)),
                reason: ResyncReason::Overflow,
            },
            21,
            19,
            3,
        ),
    ];
    for (lit, msg, len, at, new) in up {
        assert_eq!(lit.len(), len);
        assert_eq!(t3::encode_up(&msg), Ok(lit.clone()));
        assert_eq!(t3::decode_up(&lit), Ok(msg.clone()));
        assert_eq!(v1::encode_up(&msg).unwrap()[1..], lit[1..]);
        let mut ctrl = lit.clone();
        ctrl[at] = new;
        let got = t3::decode_up(&ctrl);
        assert!(got.is_ok() && got != Ok(msg), "control differs in one byte");
    }
    assert_eq!(
        t3::plan_transfer(Z, tid(1), 0, 0, 1, &empty_body()),
        Ok(vec![begin_lit, chunk_lit])
    );
}

fn sample_down() -> Vec<TransferDown> {
    let mut out = Vec::new();
    for spc in 1..=8u8 {
        for total in [1u32, 4096, 4097, 32_768, 32_769, 1_048_576] {
            let chunks = total.div_ceil(u32::from(spc) * 4096);
            if chunks <= 32 {
                for ent in [0u16, 1, 1024] {
                    out.push(TransferDown::Begin(begin(total, ent, spc, chunks as u8)));
                }
            }
        }
    }
    for index in [0u8, 1, 31] {
        for len in [1usize, 4095, 4096, 4097, 8192, 12_289, 32_767, 32_768] {
            out.push(TransferDown::Chunk(chunk(index, vec![0x5A; len])));
        }
    }
    for transfer in [1, u64::MAX] {
        for reason in [CancelReason::Superseded, CancelReason::Aborted] {
            out.push(TransferDown::Cancel {
                epoch: [9; 16],
                transfer: tid(transfer),
                reason,
            });
        }
    }
    out
}

#[test]
fn d1_encode_is_0x81_with_byte0_changed() {
    for msg in sample_down() {
        let old = v1::encode_down(&msg).expect("0x81 accepts");
        let new = t3::encode_down(&msg).expect("0x82 accepts what 0x81 accepts");
        assert_eq!(old[0], 0x81);
        assert_eq!(new[0], 0x82);
        assert_eq!(old[1..], new[1..]);
        assert_eq!(t3::decode_down(&new), Ok(msg.clone()));
        assert_eq!(t3::decode_down(&new), v1::decode_down(&as_v1(&new)));
    }
    for transfer in [1, 7, u64::MAX] {
        let mut ups = vec![TransferUp::Published {
            epoch: [3; 16],
            transfer: tid(transfer),
        }];
        for reason in [
            ResyncReason::Expired,
            ResyncReason::Invalid,
            ResyncReason::Overflow,
        ] {
            for after in [None, Some(tid(transfer))] {
                ups.push(TransferUp::Resync {
                    epoch: [3; 16],
                    after,
                    reason,
                });
            }
        }
        for msg in ups {
            let old = v1::encode_up(&msg).unwrap();
            let new = t3::encode_up(&msg).unwrap();
            assert_eq!(old[1..], new[1..]);
            assert_eq!(new[0], 0x82);
            assert_eq!(t3::decode_up(&new), Ok(msg.clone()));
            assert_eq!(t3::decode_up(&new), v1::decode_up(&as_v1(&new)));
        }
    }
    // What 0x81 refuses and 0x82 accepts is exactly the three wider bounds.
    let wide = TransferDown::Begin(begin(1_048_577, 0, 8, 33));
    assert_eq!(v1::encode_down(&wide), Err(LimitExceeded));
    assert!(t3::encode_down(&wide).is_ok());
    let wide = TransferDown::Chunk(chunk(32, vec![1]));
    assert_eq!(v1::encode_down(&wide), Err(LimitExceeded));
    assert!(t3::encode_down(&wide).is_ok());
}

/// A Begin row with the identity, base and cursor of `OK`.
fn bt(total: u64, ent: u64, spc: u8, chunks: u8) -> B {
    B {
        total,
        ent,
        spc,
        chunks,
        ..OK
    }
}

fn begin_table() -> Vec<(B, R<()>)> {
    let m = Err(Malformed);
    let l = Err(LimitExceeded);
    let wide = bt(4_194_304, 0, 8, 128);
    vec![
        (OK, Ok(())),
        (B { transfer: 0, ..OK }, m),
        (
            B {
                transfer: 0,
                cursor: u64::MAX,
                ..bt(4_194_305, 1025, 0, 0)
            },
            m,
        ),
        (
            B {
                cursor: SEQ_LAST,
                ..OK
            },
            Ok(()),
        ),
        (
            B {
                cursor: u64::MAX,
                ..OK
            },
            m,
        ),
        (
            B {
                cursor: u64::MAX,
                ..bt(4_194_305, 0, 1, 1)
            },
            m,
        ),
        (bt(0, 0, 1, 1), m),
        (bt(0, 1025, 0, 0), m),
        (wide, Ok(())),
        (bt(4_194_305, 0, 8, 128), l),
        (bt(4_194_305, 1025, 0, 0), l),
        (bt(1 << 32, 0, 8, 128), l),
        (bt(u64::MAX, 0, 8, 128), l),
        (bt(6, 1024, 1, 1), Ok(())),
        (bt(6, 1025, 1, 1), l),
        (bt(6, 70_000, 1, 1), l),
        (bt(6, 1025, 0, 0), l),
        (bt(6, 0, 0, 1), m),
        (bt(6, 0, 0, 0), m),
        (bt(6, 0, 8, 1), Ok(())),
        (bt(6, 0, 9, 1), l),
        (bt(6, 0, 9, 0), l),
        (bt(6, 0, 255, 1), l),
        (bt(6, 0, 1, 0), m),
        (bt(4_194_304, 0, 8, 129), l),
        (bt(6, 0, 1, 129), l),
        (bt(6, 0, 1, 255), l),
        (bt(6, 0, 1, 2), m),
        (bt(4097, 0, 1, 1), m),
        (bt(4097, 0, 1, 2), Ok(())),
        (bt(32_769, 0, 8, 2), Ok(())),
        (bt(32_768, 0, 8, 1), Ok(())),
        (bt(32_768, 0, 8, 2), m),
        (bt(8192, 0, 2, 1), Ok(())),
        (bt(8193, 0, 2, 1), m),
        (bt(1_048_577, 0, 8, 33), Ok(())),
        (bt(1_048_577, 0, 8, 32), m),
        (bt(3_145_728, 0, 8, 96), Ok(())),
        (bt(2_847_788, 0, 8, 87), Ok(())),
        (bt(2_847_788, 0, 8, 86), m),
        (bt(2_847_788, 0, 8, 88), m),
    ]
}

fn within_v1(b: B) -> bool {
    b.total <= 1_048_576 && b.chunks <= 32
}

#[test]
fn d1_begin_decode_check_order() {
    for (b, expect) in begin_table() {
        let frame = bb(0x82, b);
        let got = t3::decode_down(&frame);
        match expect {
            Ok(()) => assert_eq!(
                got,
                Ok(TransferDown::Begin(BeginTransfer {
                    epoch: Z,
                    transfer: tid(b.transfer),
                    base_tick: 0,
                    cursor: b.cursor,
                    total_bytes: b.total as u32,
                    entity_count: b.ent as u16,
                    segments_per_chunk: b.spc,
                    chunk_count: b.chunks,
                })),
                "{frame:02x?}"
            ),
            Err(e) => assert_eq!(got, Err(e), "{frame:02x?}"),
        }
        if within_v1(b) {
            assert_eq!(got, v1::decode_down(&as_v1(&frame)), "{frame:02x?}");
        }
        // A trailing byte is checked last: it never hides an earlier error.
        let mut longer = frame.clone();
        longer.push(0);
        let want = match expect {
            Ok(()) => Err(Malformed),
            Err(e) => Err(e),
        };
        assert_eq!(t3::decode_down(&longer), want, "{longer:02x?}");
    }
    // Every proper prefix of a valid Begin is Malformed (truncation).
    let valid = bb(0x82, OK);
    for cut in 0..valid.len() {
        assert_eq!(t3::decode_down(&valid[..cut]), Err(Malformed), "cut {cut}");
    }
    // Checked as soon as read: an over-cap value fails with nothing after it.
    let head = bb(
        0x82,
        B {
            total: 4_194_305,
            ..OK
        },
    );
    assert_eq!(t3::decode_down(&head[..head.len() - 3]), Err(LimitExceeded));
    let head = bb(0x82, B { ent: 1025, ..OK });
    assert_eq!(t3::decode_down(&head[..head.len() - 2]), Err(LimitExceeded));
    let head = bb(0x82, B { spc: 9, ..OK });
    assert_eq!(t3::decode_down(&head[..head.len() - 1]), Err(LimitExceeded));
}

#[test]
fn d1_begin_encode_and_check_begin_agree() {
    for (b, expect) in begin_table() {
        if b.transfer == 0 || b.total > u64::from(u32::MAX) || b.ent > u64::from(u16::MAX) {
            continue;
        }
        let msg = BeginTransfer {
            epoch: Z,
            transfer: tid(b.transfer),
            base_tick: 0,
            cursor: b.cursor,
            total_bytes: b.total as u32,
            entity_count: b.ent as u16,
            segments_per_chunk: b.spc,
            chunk_count: b.chunks,
        };
        assert_eq!(t3::check_begin(&msg), expect, "total {}", b.total);
        let enc = t3::encode_down(&TransferDown::Begin(msg.clone()));
        match expect {
            Ok(()) => assert_eq!(enc, Ok(bb(0x82, b))),
            Err(e) => assert_eq!(enc, Err(e)),
        }
        if within_v1(b) {
            assert_eq!(t3::check_begin(&msg), msg.validate());
        }
    }
}

type ChunkCase = (Vec<u8>, R<usize>);

fn chunk_table() -> Vec<ChunkCase> {
    let nine: Vec<u8> = [seg(4096).repeat(8), seg(1)].concat();
    vec![
        (cb(0x82, 1, 0, 1, &seg(1)), Ok(1)),
        (cb(0x82, 0, 128, 0, &[]), Err(Malformed)),
        (cb(0x82, 1, 127, 1, &seg(1)), Ok(1)),
        (cb(0x82, 1, 32, 1, &seg(1)), Ok(1)),
        (cb(0x82, 1, 128, 1, &seg(1)), Err(LimitExceeded)),
        (cb(0x82, 1, 128, 0, &[]), Err(LimitExceeded)),
        (cb(0x82, 1, 255, 1, &seg(1)), Err(LimitExceeded)),
        (cb(0x82, 1, 0, 0, &[]), Err(Malformed)),
        (cb(0x82, 1, 0, 0, &seg(1)), Err(Malformed)),
        (cb(0x82, 1, 0, 9, &nine), Err(LimitExceeded)),
        (cb(0x82, 1, 0, 9, &[]), Err(LimitExceeded)),
        (cb(0x82, 1, 0, 255, &[]), Err(LimitExceeded)),
        (cb(0x82, 1, 0, 1, &seg(4097)), Err(LimitExceeded)),
        (cb(0x82, 1, 0, 1, &vi(4097)), Err(LimitExceeded)),
        (cb(0x82, 1, 0, 1, &vi(5000)), Err(LimitExceeded)),
        (cb(0x82, 1, 0, 1, &vi(0)), Err(Malformed)),
        (cb(0x82, 1, 0, 1, &[0, 1, 2, 3]), Err(Malformed)),
        (
            cb(0x82, 1, 0, 2, &[seg(4095), seg(1)].concat()),
            Err(Malformed),
        ),
        (
            cb(0x82, 1, 0, 2, &[seg(100), seg(4097)].concat()),
            Err(Malformed),
        ),
        (
            cb(0x82, 1, 0, 2, &[seg(4096), vi(0)].concat()),
            Err(Malformed),
        ),
        (
            cb(0x82, 1, 0, 2, &[seg(4096), seg(4097)].concat()),
            Err(LimitExceeded),
        ),
        (
            cb(0x82, 1, 0, 1, &[vi(10), vec![0xAB; 9]].concat()),
            Err(Malformed),
        ),
        (
            cb(0x82, 1, 0, 1, &[seg(1), vec![0]].concat()),
            Err(Malformed),
        ),
        (
            cb(0x82, 1, 0, 2, &[seg(4096), seg(4096)].concat()),
            Ok(8192),
        ),
        (cb(0x82, 1, 0, 8, &seg(4096).repeat(8)), Ok(32_768)),
        (cb(0x82, 1, 0, 1, &seg(4096)), Ok(4096)),
    ]
}

#[test]
fn d1_chunk_decode_check_order() {
    for (frame, expect) in chunk_table() {
        let got = t3::decode_down(&frame);
        match expect {
            Ok(len) => match &got {
                Ok(TransferDown::Chunk(c)) => {
                    assert_eq!(c.bytes.len(), len);
                    assert_eq!(c.index, frame[20]);
                    assert!(c.bytes.iter().all(|b| *b == 0xAB));
                }
                other => panic!("{other:?} for {frame:02x?}"),
            },
            Err(e) => assert_eq!(got, Err(e), "{:02x?}", &frame[..frame.len().min(40)]),
        }
        if frame[20] < 32 {
            assert_eq!(got, v1::decode_down(&as_v1(&frame)));
        }
    }
    // Truncation anywhere in a valid chunk is Malformed.
    let valid = cb(0x82, 1, 0, 1, &seg(3));
    for cut in 0..valid.len() {
        assert_eq!(t3::decode_down(&valid[..cut]), Err(Malformed), "cut {cut}");
    }
    // The 0x81 decoder holds index 32..=127 to its own bound.
    for index in [32u8, 100, 127] {
        let frame = cb(0x82, 1, index, 1, &seg(1));
        assert!(t3::decode_down(&frame).is_ok());
        assert_eq!(v1::decode_down(&as_v1(&frame)), Err(LimitExceeded));
    }
}

#[test]
fn d1_chunk_encode_checks() {
    let msg = |index: u8, len: usize| TransferDown::Chunk(chunk(index, vec![0xAB; len]));
    assert_eq!(
        t3::encode_down(&msg(127, 1)),
        Ok(cb(0x82, 1, 127, 1, &seg(1)))
    );
    assert_eq!(t3::encode_down(&msg(128, 1)), Err(LimitExceeded));
    assert_eq!(t3::encode_down(&msg(255, 1)), Err(LimitExceeded));
    assert_eq!(t3::encode_down(&msg(128, 0)), Err(LimitExceeded));
    assert_eq!(t3::encode_down(&msg(0, 0)), Err(Malformed));
    assert_eq!(t3::encode_down(&msg(0, 32_769)), Err(LimitExceeded));
    assert_eq!(t3::encode_down(&msg(128, 32_769)), Err(LimitExceeded));
    assert_eq!(
        t3::encode_down(&msg(0, 32_768)),
        Ok(cb(0x82, 1, 0, 8, &seg(4096).repeat(8)))
    );
    assert_eq!(
        t3::encode_down(&msg(5, 4097)),
        Ok(cb(0x82, 1, 5, 2, &[seg(4096), seg(1)].concat()))
    );
    for index in [0u8, 31] {
        for len in [1, 4096, 4097, 32_768] {
            assert_eq!(
                t3::encode_down(&msg(index, len)).map(|b| b[1..].to_vec()),
                v1::encode_down(&msg(index, len)).map(|b| b[1..].to_vec())
            );
        }
    }
}

#[test]
fn d1_cancel_published_resync() {
    // Down: Cancel.
    let cancel = |reason: u8| {
        let mut f = hx("82 02 12 Z16 01");
        f.push(reason);
        f
    };
    for bad in [0u8, 3, 255] {
        assert_eq!(t3::decode_down(&cancel(bad)), Err(UnknownMessage));
    }
    let mut zero_id = hx("82 02 12 Z16 00 01");
    assert_eq!(t3::decode_down(&zero_id), Err(Malformed));
    zero_id.truncate(zero_id.len() - 1);
    assert_eq!(t3::decode_down(&zero_id), Err(Malformed));
    let mut trailing = cancel(1);
    trailing.push(0);
    assert_eq!(t3::decode_down(&trailing), Err(Malformed));
    assert_eq!(t3::decode_down(&cancel(1)[..20]), Err(Malformed));
    // Up: Published, Resync.
    let published = hx("82 02 20 Z16 01");
    assert!(t3::decode_up(&published).is_ok());
    assert_eq!(t3::decode_up(&hx("82 02 20 Z16 00")), Err(Malformed));
    assert_eq!(
        t3::decode_up(&[published.clone(), vec![0]].concat()),
        Err(Malformed)
    );
    assert_eq!(t3::decode_up(&published[..19]), Err(Malformed));
    for bad in [0u8, 4, 255] {
        let mut f = hx("82 02 21 Z16 00");
        f.push(bad);
        assert_eq!(t3::decode_up(&f), Err(UnknownMessage));
    }
    assert_eq!(
        t3::decode_up(&hx("82 02 21 Z16 00 02")),
        Ok(TransferUp::Resync {
            epoch: Z,
            after: None,
            reason: ResyncReason::Invalid
        })
    );
    assert_eq!(t3::decode_up(&hx("82 02 21 Z16 00 01 00")), Err(Malformed));
    assert_eq!(t3::decode_up(&hx("82 02 21 Z16 00")), Err(Malformed));
    // Encode refuses what decode refuses; delegation keeps the 0x81 bytes.
    let zero = tid(1);
    assert_eq!(
        t3::encode_up(&TransferUp::Published {
            epoch: [4; 16],
            transfer: zero
        })
        .map(|b| b[1..].to_vec()),
        v1::encode_up(&TransferUp::Published {
            epoch: [4; 16],
            transfer: zero
        })
        .map(|b| b[1..].to_vec())
    );
}

#[test]
fn d1_decode_preamble_order() {
    let ok_down = cancel_frame(0x82, 2, 0x12);
    let ok_up = {
        let mut f = hx("82 02 20 Z16 01");
        f.push(0);
        f.pop();
        f
    };
    assert!(t3::decode_down(&ok_down).is_ok());
    assert!(t3::decode_up(&ok_up).is_ok());
    // (1) size: 65,536 passes the check, 65,537 is FrameTooLarge, whatever byte 0.
    for first in [0x82u8, 0x81, 0x00] {
        assert_eq!(t3::decode_down(&big(first, 65_537)), Err(FrameTooLarge));
        assert_eq!(t3::decode_up(&big(first, 4_097)), Err(FrameTooLarge));
    }
    assert_eq!(t3::decode_down(&big(0x82, 65_536)), Err(UnknownMessage));
    assert_eq!(t3::decode_down(&big(0x81, 65_536)), Err(UnsupportedVersion));
    assert_eq!(t3::decode_up(&big(0x82, 4_096)), Err(UnknownMessage));
    assert_eq!(t3::decode_up(&big(0x81, 4_096)), Err(UnsupportedVersion));
    // The down limit is not the up limit.
    assert_eq!(t3::decode_down(&big(0x82, 4_097)), Err(UnknownMessage));
    // (2) empty is Malformed; a lone version byte and version+lane are too.
    assert_eq!(t3::decode_down(&[]), Err(Malformed));
    assert_eq!(t3::decode_up(&[]), Err(Malformed));
    for cut in [1usize, 2] {
        assert_eq!(t3::decode_down(&ok_down[..cut]), Err(Malformed));
        assert_eq!(t3::decode_up(&ok_up[..cut]), Err(Malformed));
    }
    // (3) version before lane before tag.
    for first in [0x81u8, 0x01, 0x03, 0x83, 0x00, 0xFF, 0x91] {
        assert_eq!(
            t3::decode_down(&cancel_frame(first, 2, 0x12)),
            Err(UnsupportedVersion)
        );
        assert_eq!(
            t3::decode_down(&cancel_frame(first, 3, 0x99)),
            Err(UnsupportedVersion)
        );
        assert_eq!(
            t3::decode_up(&cancel_frame(first, 3, 0x99)),
            Err(UnsupportedVersion)
        );
    }
    for lane in [0u8, 1, 3, 255] {
        assert_eq!(
            t3::decode_down(&cancel_frame(0x82, lane, 0x12)),
            Err(WrongDirection)
        );
        assert_eq!(
            t3::decode_up(&cancel_frame(0x82, lane, 0x20)),
            Err(WrongDirection)
        );
        // Lane is checked before the tag.
        assert_eq!(t3::decode_down(&[0x82, lane, 0x99]), Err(WrongDirection));
        assert_eq!(t3::decode_up(&[0x82, lane, 0x99]), Err(WrongDirection));
    }
    for tag in [0x00u8, 0x13, 0x22, 0xFF] {
        assert_eq!(t3::decode_down(&[0x82, 2, tag]), Err(UnknownMessage));
        assert_eq!(t3::decode_up(&[0x82, 2, tag]), Err(UnknownMessage));
    }
    // Direction: an up tag in the down decoder and the reverse.
    for tag in [0x20u8, 0x21] {
        assert_eq!(t3::decode_down(&[0x82, 2, tag]), Err(WrongDirection));
    }
    for tag in [0x10u8, 0x11, 0x12] {
        assert_eq!(t3::decode_up(&[0x82, 2, tag]), Err(WrongDirection));
    }
}

#[test]
fn d2_version_refusals() {
    let downs = [
        hx("82 02 10 Z16 01 00 00 06 00 01 01"),
        hx("82 02 11 Z16 01 00 01 06 00 00 00 00 00 00"),
        hx("82 02 12 Z16 01 01"),
    ];
    let ups = [hx("82 02 20 Z16 01"), hx("82 02 21 Z16 00 01")];
    for frame in &downs {
        assert_eq!(t3::decode_down(&as_v1(frame)), Err(UnsupportedVersion));
        assert_eq!(v1::decode_down(frame), Err(UnsupportedVersion));
        assert!(v1::decode_down(&as_v1(frame)).is_ok());
        assert!(t3::decode_down(frame).is_ok());
        assert_eq!(codec_v3::decode_delta(frame), Err(UnsupportedVersion));
        assert_eq!(codec_v3::decode_intent(frame), Err(UnsupportedVersion));
    }
    for frame in &ups {
        assert_eq!(t3::decode_up(&as_v1(frame)), Err(UnsupportedVersion));
        assert_eq!(v1::decode_up(frame), Err(UnsupportedVersion));
        assert!(v1::decode_up(&as_v1(frame)).is_ok());
        assert!(t3::decode_up(frame).is_ok());
        assert_eq!(codec_v3::decode_intent(frame), Err(UnsupportedVersion));
    }
    assert_eq!(t3::decode_down(&[]), Err(Malformed));
    assert_eq!(t3::decode_up(&[]), Err(Malformed));
    assert_eq!(v1::decode_down(&[]), Err(Malformed));
    assert_eq!(t3::decode_down(&big(0x81, 65_537)), Err(FrameTooLarge));
    assert_eq!(t3::decode_up(&big(0x81, 4_097)), Err(FrameTooLarge));
    assert_eq!(
        t3::decode_down(&hx("82 03 10 Z16 01 00 00 06 00 01 01")),
        Err(WrongDirection)
    );
    assert_eq!(t3::decode_up(&hx("82 03 20 Z16 01")), Err(WrongDirection));
    assert_eq!(t3::decode_down(&hx("82 02 20 Z16 01")), Err(WrongDirection));
    assert_eq!(
        t3::decode_up(&hx("82 02 12 Z16 01 01")),
        Err(WrongDirection)
    );
}

#[test]
fn d10_caps() {
    assert_eq!(t3::TRANSFER_VERSION_V3, 0x82);
    assert_eq!(t3::MAX_SNAPSHOT_BYTES_V3, 4_194_304);
    assert_eq!(t3::MAX_SNAPSHOT_CHUNKS_V3, 128);
    assert_eq!(t3::MAX_ENCODED_BODY_BYTES_V3, 2_847_788);
    assert_eq!(protocol::MAX_SNAPSHOT_BYTES, 1_048_576);
    assert_eq!(protocol::MAX_SNAPSHOT_CHUNKS, 32);
    assert_eq!(t3::LANE_SNAPSHOT, 2);
}

#[test]
fn d10_begin_bounds() {
    let max = hx("82 02 10 Z16 01 00 00 80 80 80 02 00 08 80");
    assert_eq!(max.len(), 29);
    let msg = begin(4_194_304, 0, 8, 128);
    assert_eq!(t3::decode_down(&max), Ok(TransferDown::Begin(msg.clone())));
    assert_eq!(
        t3::encode_down(&TransferDown::Begin(msg.clone())),
        Ok(max.clone())
    );
    assert_eq!(t3::check_begin(&msg), Ok(()));
    assert_eq!(msg.validate(), Err(LimitExceeded));
    assert_eq!(v1::decode_down(&as_v1(&max)), Err(LimitExceeded));
    assert_eq!(
        v1::encode_down(&TransferDown::Begin(msg.clone())),
        Err(LimitExceeded)
    );
    // total 4,194,305 and chunk count 129.
    let over_total = hx("82 02 10 Z16 01 00 00 81 80 80 02 00 08 80");
    assert_eq!(t3::decode_down(&over_total), Err(LimitExceeded));
    assert_eq!(
        t3::encode_down(&TransferDown::Begin(begin(4_194_305, 0, 8, 128))),
        Err(LimitExceeded)
    );
    assert_eq!(
        t3::check_begin(&begin(4_194_305, 0, 8, 128)),
        Err(LimitExceeded)
    );
    let over_chunks = hx("82 02 10 Z16 01 00 00 80 80 80 02 00 08 81");
    assert_eq!(t3::decode_down(&over_chunks), Err(LimitExceeded));
    assert_eq!(
        t3::encode_down(&TransferDown::Begin(begin(4_194_304, 0, 8, 129))),
        Err(LimitExceeded)
    );
    // total 1,048,577 with 33 chunks.
    let wide = hx("82 02 10 Z16 01 00 00 81 80 40 00 08 21");
    assert_eq!(wide.len(), 28);
    assert_eq!(
        t3::decode_down(&wide),
        Ok(TransferDown::Begin(begin(1_048_577, 0, 8, 33)))
    );
    assert_eq!(v1::decode_down(&as_v1(&wide)), Err(LimitExceeded));
    let short = hx("82 02 10 Z16 01 00 00 81 80 40 00 08 20");
    assert_eq!(t3::decode_down(&short), Err(Malformed));
    // chunk_len.
    assert_eq!(t3::chunk_len(&msg, 127), Some(32_768));
    assert_eq!(t3::chunk_len(&msg, 128), None);
    assert_eq!(t3::chunk_len(&msg, 0), Some(32_768));
    assert_eq!(msg.chunk_len(127), None);
    let w = begin(1_048_577, 0, 8, 33);
    assert_eq!(t3::chunk_len(&w, 31), Some(32_768));
    assert_eq!(t3::chunk_len(&w, 32), Some(1));
    assert_eq!(t3::chunk_len(&w, 33), None);
    // An invalid Begin has no chunk lengths.
    assert_eq!(t3::chunk_len(&begin(4_194_305, 0, 8, 128), 0), None);
    assert_eq!(t3::chunk_len(&begin(6, 0, 1, 2), 0), None);
    // v1 caps unchanged: its own pair still refuses one past.
    assert_eq!(begin(1_048_576, 0, 8, 32).validate(), Ok(()));
    assert_eq!(begin(1_048_577, 0, 8, 33).validate(), Err(LimitExceeded));
}

#[test]
fn d10_chunk_index_bounds() {
    let lit = hx("82 02 11 Z16 01 7F 01 01 00");
    assert_eq!(
        t3::decode_down(&lit),
        Ok(TransferDown::Chunk(chunk(127, vec![0])))
    );
    assert_eq!(
        t3::encode_down(&TransferDown::Chunk(chunk(127, vec![0]))),
        Ok(lit.clone())
    );
    assert_eq!(v1::decode_down(&as_v1(&lit)), Err(LimitExceeded));
    let over = hx("82 02 11 Z16 01 80");
    assert_eq!(t3::decode_down(&over), Err(LimitExceeded));
    let mut full = lit.clone();
    full[20] = 0x80;
    assert_eq!(t3::decode_down(&full), Err(LimitExceeded));
    assert_eq!(
        t3::encode_down(&TransferDown::Chunk(chunk(128, vec![0]))),
        Err(LimitExceeded)
    );
}

// ---------------------------------------------------------------------------
// Body helpers: hand-built bytes from the D-2 layout.
// ---------------------------------------------------------------------------

fn ent(n: u64) -> SnapshotEntityV3 {
    SnapshotEntityV3 {
        entity: nid(n),
        health: None,
        facts: FactSet::default(),
    }
}

fn body_of(ents: Vec<SnapshotEntityV3>) -> SnapshotBodyV3 {
    SnapshotBodyV3 {
        entities: ents,
        ..empty_body()
    }
}

fn tl(key: i32, entity: u64) -> TimelineEntry {
    TimelineEntry {
        key,
        entity: nid(entity),
    }
}

fn fam_list<T>(items: &[T], one: impl Fn(&T) -> Vec<u8>) -> Vec<u8> {
    let mut out = vec![1];
    out.extend(vi(items.len() as u64));
    for item in items {
        out.extend(one(item));
    }
    out
}

fn fam_pools(p: &[(u128, u32, u32)]) -> Vec<u8> {
    fam_list(p, |&(id, c, m)| {
        [ut(id), vi(c.into()), vi(m.into())].concat()
    })
}

fn fam_effects(e: &[(u128, u64)]) -> Vec<u8> {
    fam_list(e, |&(id, round)| [ut(id), vi(round)].concat())
}

fn fam_order(ability: u128, target: u64, repeat: bool) -> Vec<u8> {
    [vec![1, 1], ut(ability), vi(target), vec![u8::from(repeat)]].concat()
}

fn fam_stats(s: &[(&str, u8, i32)]) -> Vec<u8> {
    fam_list(s, |&(n, k, v)| {
        [string_bytes(n), vec![k], vi(zz(v))].concat()
    })
}

fn fam_tags(t: &[&str]) -> Vec<u8> {
    fam_list(t, |t| string_bytes(t))
}

/// Five families in wire order; each argument is raw family bytes.
fn facts_b(pools: &[u8], effects: &[u8], order: &[u8], stats: &[u8], tags: &[u8]) -> Vec<u8> {
    [pools, effects, order, stats, tags].concat()
}

const NONE5: [u8; 5] = [0; 5];

fn eb(net: u64, health: &[u8], facts: &[u8]) -> Vec<u8> {
    [vi(net).as_slice(), health, facts].concat()
}

fn e0(net: u64) -> Vec<u8> {
    eb(net, &[0], &NONE5)
}

fn rb(ents: &[Vec<u8>], turn: &[u8], enc: &[u8], tl: &[u8]) -> Vec<u8> {
    [
        vi(ents.len() as u64),
        ents.concat(),
        turn.to_vec(),
        enc.to_vec(),
        tl.to_vec(),
    ]
    .concat()
}

fn rb0(ents: &[Vec<u8>]) -> Vec<u8> {
    rb(ents, &[0], &[0, 0, 0], &[0])
}

fn pool(id: u128, current: u32, max: u32) -> PoolFact {
    PoolFact {
        pool: u(id),
        current,
        max,
    }
}

fn stat(name: &str, value: StatValue) -> StatFact {
    StatFact {
        name: name.to_owned(),
        value,
    }
}

fn with_facts(n: u64, facts: FactSet) -> SnapshotEntityV3 {
    SnapshotEntityV3 { facts, ..ent(n) }
}

// ---------------------------------------------------------------------------
// D3 / D4: body layout and decode order.
// ---------------------------------------------------------------------------

#[test]
fn d3_body_literals() {
    let empty = hx("00 00 00 00 00 00");
    assert_eq!(t3::encode_body(&empty_body()), Ok(empty.clone()));
    assert_eq!(t3::decode_body(&empty), Ok(empty_body()));
    let lit = hx("01 01 01 0A 0A 00 00 00 00 00 00 00 00 00 00 00");
    let health = |dead| {
        let mut e = ent(1);
        e.health = Some(HealthFact {
            health: 10,
            max_health: 10,
            dead,
        });
        body_of(vec![e])
    };
    assert_eq!(t3::encode_body(&health(false)), Ok(lit.clone()));
    assert_eq!(t3::decode_body(&lit), Ok(health(false)));
    // Control: one byte differs (`dead`), and the body differs with it.
    let mut ctrl = lit.clone();
    ctrl[5] = 1;
    assert_eq!(t3::decode_body(&ctrl), Ok(health(true)));
    assert_eq!(t3::encode_body(&health(true)), Ok(ctrl));
    // A flag byte of 02 is Malformed wherever it stands, with a valid remainder.
    for at in [2usize, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15] {
        let mut bad = lit.clone();
        bad[at] = 2;
        assert_eq!(t3::decode_body(&bad), Err(Malformed), "flag at {at}");
    }
    // Nested flags (order `some` and `repeat`, turn `active_present`, pending and
    // clock `some`), each with the remainder it would need if the flag were 1.
    let order = |some: u8, repeat: u8| {
        let fam = [vec![1, some], ut(7), vec![1, repeat]].concat();
        rb0(&[eb(1, &[0], &facts_b(&[0], &[0], &fam, &[0], &[0]))])
    };
    let bad = [
        order(2, 1),
        order(1, 2),
        rb(&[e0(1)], &[1, 2, 1, 5], &[0, 0, 0], &[0]),
        rb(&[e0(1)], &[0], &[0, 1, 2, 0, 0], &[0]),
        rb(&[e0(1)], &[0], &[0, 0, 1, 2, 120, 5], &[0]),
    ];
    assert!(t3::decode_body(&order(1, 1)).is_ok());
    for b in bad {
        assert_eq!(t3::decode_body(&b), Err(Malformed), "{b:02x?}");
    }
}

fn full_layout_body() -> SnapshotBodyV3 {
    let mut e2 = ent(2);
    e2.health = Some(HealthFact {
        health: 1,
        max_health: 2,
        dead: true,
    });
    e2.facts = FactSet {
        pools: Some(vec![pool(1, 2, 3)]),
        effects: Some(vec![EffectFact {
            effect: u(5),
            expires_round: 9,
        }]),
        order: Some(Some(OrderFact {
            ability: u(7),
            target: nid(1),
            repeat: true,
        })),
        stats: Some(vec![
            stat("might", StatValue::Int(4)),
            stat("grit", StatValue::Fixed(-65_536)),
        ]),
        tags: Some(vec!["a".to_owned(), "b".to_owned()]),
    };
    SnapshotBodyV3 {
        entities: vec![ent(1), e2],
        turn: Some(TurnFact {
            active: Some(nid(2)),
            round: 3,
        }),
        encounter: EncounterFacts {
            mode: Some(EncounterMode::RealTime),
            pending_mode: Some(None),
            clock: Some(Some(ClockFact {
                round_ticks: 120,
                elapsed: 240,
            })),
        },
        timeline: Some(vec![tl(-1, 2), tl(5, 1)]),
    }
}

fn full_layout_bytes() -> Vec<u8> {
    let facts = facts_b(
        &fam_pools(&[(1, 2, 3)]),
        &fam_effects(&[(5, 9)]),
        &fam_order(7, 1, true),
        &fam_stats(&[("might", 0, 4), ("grit", 1, -65_536)]),
        &fam_tags(&["a", "b"]),
    );
    rb(
        &[e0(1), eb(2, &[1, 1, 2, 1], &facts)],
        &[1, 1, 2, 3],
        &hx("01 01 01 00 01 01 78 F0 01"),
        &hx("01 02 01 02 0A 01"),
    )
}

#[test]
fn d3_body_full_layout_literal() {
    let bytes = full_layout_bytes();
    assert_eq!(t3::encode_body(&full_layout_body()), Ok(bytes.clone()));
    assert_eq!(t3::decode_body(&bytes), Ok(full_layout_body()));
    // zigzag(-65536) is 131071: `FF FF 07`.
    assert!(bytes.windows(3).any(|w| w == [0xFF, 0xFF, 0x07]));
    // Control: round 3 -> 4 is one byte and one field.
    let mut ctrl = bytes.clone();
    let turn_round = bytes.len() - hx("01 01 01 00 01 01 78 F0 01").len() - 6 - 1;
    assert_eq!(bytes[turn_round], 3);
    ctrl[turn_round] = 4;
    let mut want = full_layout_body();
    want.turn = Some(TurnFact {
        active: Some(nid(2)),
        round: 4,
    });
    assert_eq!(t3::decode_body(&ctrl), Ok(want));
    // Every proper prefix is Malformed.
    for cut in 0..bytes.len() {
        assert_eq!(t3::decode_body(&bytes[..cut]), Err(Malformed), "cut {cut}");
    }
}
type Refusal = (&'static str, SnapshotBodyV3, Option<Vec<u8>>, CodecError);

fn one_fam(index: usize, fam: Vec<u8>) -> Vec<u8> {
    let mut fams: Vec<Vec<u8>> = vec![vec![0]; 5];
    fams[index] = fam;
    rb0(&[eb(1, &[0], &fams.concat())])
}

fn only(facts: FactSet) -> SnapshotBodyV3 {
    body_of(vec![with_facts(1, facts)])
}

fn order_to(target: u64) -> FactSet {
    FactSet {
        order: Some(Some(OrderFact {
            ability: u(7),
            target: nid(target),
            repeat: false,
        })),
        ..FactSet::default()
    }
}

/// Entity 1 whose order targets entity 9.
fn order9_entity() -> Vec<u8> {
    let fam = [vec![1, 1], ut(7), vec![9, 0]].concat();
    eb(1, &[0], &facts_b(&[0], &[0], &fam, &[0], &[0]))
}

/// That entity alone, with a chosen timeline block.
fn dangling_order_bytes(tail: &[u8]) -> Vec<u8> {
    rb(&[order9_entity()], &[0], &[0, 0, 0], tail)
}

fn stats33() -> FactSet {
    FactSet {
        stats: Some(
            (0..33)
                .map(|i| stat(&format!("s{i}"), StatValue::Int(0)))
                .collect(),
        ),
        ..FactSet::default()
    }
}

fn bad_clock(round_ticks: u32, elapsed: u64) -> SnapshotBodyV3 {
    let mut b = body_of(vec![ent(1)]);
    b.encounter.clock = Some(Some(ClockFact {
        round_ticks,
        elapsed,
    }));
    b
}

fn active_9() -> Option<TurnFact> {
    Some(TurnFact {
        active: Some(nid(9)),
        round: 1,
    })
}

/// Each refused body, its hand-built bytes, and the one error both
/// `encode_body` and `decode_body` must give.
fn refusals() -> Vec<Refusal> {
    let (m, l) = (Malformed, LimitExceeded);
    let hostile = Some((1..=1025).map(|i| tl(0, i)).collect::<Vec<_>>());
    let h = [1, 0x81, 8];
    let one = || body_of(vec![ent(1)]);
    let two = || body_of(vec![ent(1), ent(2)]);
    let clock_bytes = hx("00 00 01 01 00 05");
    let mut rows: Vec<Refusal> = vec![
        (
            "dup entity",
            body_of(vec![ent(1), ent(1)]),
            Some(rb0(&[e0(1), e0(1)])),
            m,
        ),
        (
            "descending",
            body_of(vec![ent(2), ent(1)]),
            Some(rb0(&[e0(2), e0(1)])),
            m,
        ),
        (
            "1025 entities",
            body_of((1..=1025).map(ent).collect()),
            Some(hx("81 08")),
            l,
        ),
        (
            "order target",
            only(order_to(9)),
            Some(dangling_order_bytes(&[0])),
            m,
        ),
        (
            "active",
            SnapshotBodyV3 {
                turn: active_9(),
                ..one()
            },
            Some(rb(&[e0(1)], &[1, 1, 9, 1], &[0, 0, 0], &[0])),
            m,
        ),
        (
            "timeline entity",
            SnapshotBodyV3 {
                timeline: Some(vec![tl(0, 9)]),
                ..one()
            },
            Some(rb(&[e0(1)], &[0], &[0, 0, 0], &[1, 1, 0, 9])),
            m,
        ),
        (
            "keys 5,4",
            SnapshotBodyV3 {
                timeline: Some(vec![tl(5, 1), tl(4, 2)]),
                ..two()
            },
            Some(rb(
                &[e0(1), e0(2)],
                &[0],
                &[0, 0, 0],
                &hx("01 02 0A 01 08 02"),
            )),
            m,
        ),
        (
            "repeated timeline entity",
            SnapshotBodyV3 {
                timeline: Some(vec![tl(1, 1), tl(2, 1)]),
                ..two()
            },
            Some(rb(
                &[e0(1), e0(2)],
                &[0],
                &[0, 0, 0],
                &hx("01 02 02 01 04 01"),
            )),
            m,
        ),
        (
            "round_ticks 0",
            bad_clock(0, 5),
            Some(rb(&[e0(1)], &[0], &clock_bytes, &[0])),
            m,
        ),
        (
            "elapsed 2^31",
            bad_clock(120, 2_147_483_648),
            Some(rb(
                &[e0(1)],
                &[0],
                &hx("00 00 01 01 78 80 80 80 80 08"),
                &[0],
            )),
            m,
        ),
        (
            "timeline 1025",
            SnapshotBodyV3 {
                timeline: hostile.clone(),
                ..one()
            },
            Some(rb(&[e0(1)], &[0], &[0, 0, 0], &h)),
            l,
        ),
        (
            "[2 (33 stats), 1]",
            body_of(vec![with_facts(2, stats33()), ent(1)]),
            Some(hx("02 02 00 00 00 00 01 21")),
            l,
        ),
        (
            "[2, 1 (33 stats)]",
            body_of(vec![ent(2), with_facts(1, stats33())]),
            Some([hx("02"), e0(2), hx("01 00 00 00 00 01 21")].concat()),
            m,
        ),
        (
            "[1 (dangling order), 2 (33 stats)]",
            body_of(vec![with_facts(1, order_to(9)), with_facts(2, stats33())]),
            Some([hx("02"), order9_entity(), hx("02 00 00 00 00 01 21")].concat()),
            l,
        ),
        (
            "dangling order beats timeline 1025",
            SnapshotBodyV3 {
                timeline: hostile.clone(),
                ..only(order_to(9))
            },
            Some(dangling_order_bytes(&h)),
            m,
        ),
        (
            "dangling active beats timeline 1025",
            SnapshotBodyV3 {
                turn: active_9(),
                timeline: hostile.clone(),
                ..one()
            },
            Some(rb(&[e0(1)], &[1, 1, 9, 1], &[0, 0, 0], &h)),
            m,
        ),
        (
            "bad clock beats timeline 1025",
            SnapshotBodyV3 {
                timeline: hostile,
                ..bad_clock(0, 5)
            },
            Some(rb(&[e0(1)], &[0], &clock_bytes, &h)),
            m,
        ),
    ];
    let pools = |p: Vec<PoolFact>| {
        only(FactSet {
            pools: Some(p),
            ..FactSet::default()
        })
    };
    let effects = |e: Vec<EffectFact>| {
        only(FactSet {
            effects: Some(e),
            ..FactSet::default()
        })
    };
    let stats = |s: Vec<StatFact>| {
        only(FactSet {
            stats: Some(s),
            ..FactSet::default()
        })
    };
    let tags = |t: &[&str]| {
        let tags = t.iter().map(|s| (*s).to_owned()).collect();
        only(FactSet {
            tags: Some(tags),
            ..FactSet::default()
        })
    };
    let eff = |id: u128| EffectFact {
        effect: u(id),
        expires_round: 1,
    };
    let n = |s: &str| stat(s, StatValue::Int(0));
    let long = "a".repeat(33);
    let nine = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];
    rows.extend([
        (
            "17 pools",
            pools((1..=17).map(|i| pool(i, 1, 1)).collect()),
            Some(one_fam(0, vec![1, 0x11])),
            l,
        ),
        (
            "current > max",
            pools(vec![pool(1, 3, 2)]),
            Some(one_fam(0, fam_pools(&[(1, 3, 2)]))),
            m,
        ),
        (
            "dup pool",
            pools(vec![pool(1, 1, 1), pool(1, 2, 2)]),
            Some(one_fam(0, fam_pools(&[(1, 1, 1), (1, 2, 2)]))),
            m,
        ),
        (
            "17 effects",
            effects((1..=17).map(eff).collect()),
            Some(one_fam(1, vec![1, 0x11])),
            l,
        ),
        (
            "effects 5,4",
            effects(vec![eff(5), eff(4)]),
            Some(one_fam(1, fam_effects(&[(5, 1), (4, 1)]))),
            m,
        ),
        (
            "effects 5,5",
            effects(vec![eff(5), eff(5)]),
            Some(one_fam(1, fam_effects(&[(5, 1), (5, 1)]))),
            m,
        ),
        (
            "33 stats",
            stats((0..33).map(|i| n(&format!("s{i}"))).collect()),
            Some(one_fam(3, vec![1, 0x21])),
            l,
        ),
        (
            "33-byte stat name",
            stats(vec![n(&long)]),
            Some(one_fam(3, vec![1, 1, 0x21])),
            l,
        ),
        (
            "empty stat name",
            stats(vec![n("")]),
            Some(one_fam(3, fam_stats(&[("", 0, 0)]))),
            m,
        ),
        (
            "dup stat name",
            stats(vec![n("a"), stat("a", StatValue::Fixed(1))]),
            Some(one_fam(3, fam_stats(&[("a", 0, 0), ("a", 1, 1)]))),
            m,
        ),
        ("9 tags", tags(&nine), Some(one_fam(4, vec![1, 9])), l),
        (
            "33-byte tag",
            tags(&[&long]),
            Some(one_fam(4, vec![1, 1, 0x21])),
            l,
        ),
        (
            "empty tag",
            tags(&[""]),
            Some(one_fam(4, fam_tags(&[""]))),
            m,
        ),
        (
            "tags b,a",
            tags(&["b", "a"]),
            Some(one_fam(4, fam_tags(&["b", "a"]))),
            m,
        ),
        (
            "tags a,a",
            tags(&["a", "a"]),
            Some(one_fam(4, fam_tags(&["a", "a"]))),
            m,
        ),
    ]);
    rows
}

#[test]
fn d9_body_encode_checks() {
    for (name, body, _, err) in refusals() {
        assert_eq!(t3::encode_body(&body), Err(err), "{name}");
    }
    // Controls: each fault fixed gives bytes.
    let ok = |b: &SnapshotBodyV3| assert!(t3::encode_body(b).is_ok(), "{b:?}");
    ok(&body_of(vec![ent(1), ent(2)]));
    ok(&body_of((1..=1024).map(ent).collect()));
    let first = with_facts(1, order_to(2));
    let mut b = body_of(vec![first, ent(2)]);
    ok(&b);
    b.turn = Some(TurnFact {
        active: Some(nid(2)),
        round: 1,
    });
    b.timeline = Some(vec![tl(4, 2), tl(4, 1)]);
    ok(&b);
    b.encounter.clock = Some(Some(ClockFact {
        round_ticks: 1,
        elapsed: 2_147_483_647,
    }));
    ok(&b);
    b.timeline = Some(Vec::new());
    ok(&b);
}

#[test]
fn d9_encode_agrees_with_decode() {
    let rows = refusals();
    assert!(rows.iter().all(|r| r.2.is_some()));
    for (name, _, bytes, err) in rows {
        assert_eq!(t3::decode_body(&bytes.unwrap()), Err(err), "{name}");
    }
}

#[test]
fn d4_body_refusals() {
    assert_eq!(t3::decode_body(&hx("81 08")), Err(LimitExceeded));
    assert_eq!(t3::decode_body(&hx("80 08")), Err(Malformed));
    assert_eq!(t3::decode_body(&rb0(&[e0(2), e0(1)])), Err(Malformed));
    assert_eq!(
        t3::decode_body(&rb0(&[e0(1), e0(2)])),
        Ok(body_of(vec![ent(1), ent(2)]))
    );
    assert_eq!(t3::decode_body(&rb0(&[e0(0)])), Err(Malformed));
    // Entity 0 is refused before its health and facts are read.
    assert_eq!(
        t3::decode_body(&hx("01 00 00 00 00 00 01 21")),
        Err(Malformed)
    );
    // The 4 MiB input cap is checked first; the cap itself is not over.
    assert_eq!(t3::decode_body(&vec![0; 4_194_305]), Err(LimitExceeded));
    assert_eq!(t3::decode_body(&vec![0; 4_194_304]), Err(Malformed));
    let mut trailing = rb0(&[e0(1)]);
    assert!(t3::decode_body(&trailing).is_ok());
    trailing.push(0);
    assert_eq!(t3::decode_body(&trailing), Err(Malformed));
    for (name, _, bytes, err) in refusals() {
        if ["order target", "active", "timeline entity"].contains(&name) {
            assert_eq!(t3::decode_body(&bytes.unwrap()), Err(err), "{name}");
        }
    }
}

#[test]
fn d4_body_decode_order() {
    let order_ent = |target: u64| {
        let fam = [vec![1, 1], ut(7), vi(target), vec![0]].concat();
        eb(1, &[0], &facts_b(&[0], &[0], &fam, &[0], &[0]))
    };
    let dec = |ents: &[Vec<u8>], turn: &[u8], enc: &[u8], tl: &[u8]| {
        t3::decode_body(&rb(ents, turn, enc, tl))
    };
    // Order targets are checked after the last entity, so a forward target is fine.
    assert!(t3::decode_body(&rb0(&[order_ent(2), e0(2)])).is_ok());
    assert!(t3::decode_body(&rb0(&[order_ent(1)])).is_ok());
    assert_eq!(
        t3::decode_body(&rb0(&[order_ent(3), e0(2)])),
        Err(Malformed)
    );
    // Membership (order, then turn) comes before the encounter block.
    let bad_mode = hx("01 02");
    let order9 = [order_ent(9)];
    assert_eq!(dec(&order9, &[0], &bad_mode, &[0]), Err(Malformed));
    assert_eq!(
        dec(&[e0(1)], &[1, 1, 9, 1], &bad_mode, &[0]),
        Err(Malformed)
    );
    assert_eq!(
        dec(&[e0(1)], &[1, 1, 1, 1], &bad_mode, &[0]),
        Err(UnknownMessage)
    );
    // The encounter block comes before the timeline block.
    let h = [1, 0x81, 8];
    let e1 = [e0(1)];
    assert_eq!(dec(&e1, &[0], &bad_mode, &h), Err(UnknownMessage));
    assert_eq!(dec(&e1, &[0], &hx("00 01 01 02"), &h), Err(UnknownMessage));
    assert_eq!(dec(&e1, &[0], &hx("00 00 01 01 00 05"), &h), Err(Malformed));
    assert_eq!(dec(&e1, &[0], &[0, 0, 0], &h), Err(LimitExceeded));
    // Timeline entries: entity 0, duplicate; equal keys may come in any order.
    let e12 = [e0(1), e0(2)];
    assert_eq!(
        dec(&e12, &[0], &[0, 0, 0], &hx("01 01 00 00")),
        Err(Malformed)
    );
    assert_eq!(
        dec(&e12, &[0], &[0, 0, 0], &hx("01 02 0A 01 0A 01")),
        Err(Malformed)
    );
    let ties_bytes = rb(&e12, &[0], &[0, 0, 0], &hx("01 02 0A 02 0A 01"));
    let ties = t3::decode_body(&ties_bytes).expect("equal keys, any order");
    assert_eq!(ties.timeline, Some(vec![tl(5, 2), tl(5, 1)]));
    assert_eq!(t3::encode_body(&ties), Ok(ties_bytes));
    // Unknown bytes and bad values inside the fact set and encounter.
    let stat_bytes = |name: &[u8], kind: u8, value: Vec<u8>| {
        let fam = [vec![1, 1], name.to_vec(), vec![kind], value].concat();
        rb0(&[eb(1, &[0], &facts_b(&[0], &[0], &[0], &fam, &[0]))])
    };
    let n = string_bytes("a");
    assert!(t3::decode_body(&stat_bytes(&n, 0, vec![0])).is_ok());
    let kinds = [(2u8, UnknownMessage), (255, UnknownMessage)];
    for (kind, err) in kinds {
        assert_eq!(t3::decode_body(&stat_bytes(&n, kind, vec![0])), Err(err));
    }
    assert_eq!(
        t3::decode_body(&stat_bytes(&n, 0, vi(1 << 32))),
        Err(Malformed)
    );
    assert_eq!(
        t3::decode_body(&stat_bytes(&[1, 0xFF], 0, vec![0])),
        Err(Malformed)
    );
    assert_eq!(
        t3::decode_body(&stat_bytes(&[0], 0, vec![0])),
        Err(Malformed)
    );
    for mode in [2u8, 3, 255] {
        assert_eq!(dec(&[], &[0], &[1, mode, 0, 0], &[0]), Err(UnknownMessage));
        assert_eq!(
            dec(&[], &[0], &[0, 1, 1, mode, 0], &[0]),
            Err(UnknownMessage)
        );
    }
    assert!(dec(&[], &[0], &[1, 1, 1, 1, 1, 0], &[0]).is_ok());
}
