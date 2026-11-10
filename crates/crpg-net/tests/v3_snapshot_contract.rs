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
        assert_eq!(
            t3::check_begin(&msg),
            expect,
            "check_begin {b:?}",
            b = b.total
        );
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
