//! T140e implementer suite: protocol v3 receiver sizing (`reassembly_v3`),
//! derived from `tasks/T140.md` Behaviour E-1..E-5 and the tests E1..E6,
//! through the public API only.
//!
//! Expectations are hand-written whole values. The v1 types are used only as
//! the contract's own oracle for "the same as v1 inside the v1 envelope" and
//! for the untouched v1 bounds. The D5 body builder is copied from
//! `tests/v3_snapshot.rs`, never imported. Each rejection has a passing
//! control, and every refusal is checked to leave the assembly equal to its
//! pre-call clone (compared with `assert!`, so a failure does not print
//! megabytes).

use std::sync::OnceLock;

use crpg_core::Ulid;
use crpg_net::codec::CodecError;
use crpg_net::protocol_transfer::{
    BeginTransfer, TransferChunk, TransferDown, TransferId, MAX_SEGMENT_BYTES,
};
use crpg_net::protocol_v3::{
    ClockFact, EffectFact, EncounterFacts, EncounterMode, FactSet, NetId, OrderFact, PoolFact,
    StatFact, StatValue, TimelineEntry,
};
use crpg_net::reassembly::{TransferAssembly, TransferLimits};
use crpg_net::reassembly_v3::{
    AssemblyError, ChunkOutcome, LimitsError, TransferAssemblyV3, TransferLimitsV3,
    POLICY_SNAPSHOT_REASSEMBLY_MS_V3, POLICY_SNAPSHOT_STAGING_BYTES_HOST_V3,
    POLICY_SNAPSHOT_STAGING_BYTES_PER_PEER_V3,
};
use crpg_net::transfer_v3::{
    decode_down, encode_body, plan_transfer, HealthFact, SnapshotBodyV3, SnapshotEntityV3,
    TurnFact, MAX_ENCODED_BODY_BYTES_V3,
};

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

const Z16: [u8; 16] = [0; 16];
const CHUNK: usize = 32_768;
const MIB: usize = 1_048_576;

fn tid(n: u64) -> TransferId {
    TransferId::new(n).expect("fixture transfer id is nonzero")
}

fn begin(total: u32, spc: u8, chunks: u8) -> BeginTransfer {
    BeginTransfer {
        epoch: Z16,
        transfer: tid(1),
        base_tick: 0,
        cursor: 0,
        total_bytes: total,
        entity_count: 0,
        segments_per_chunk: spc,
        chunk_count: chunks,
    }
}

/// The contract's `G`: 4,194,304 bytes, 8 segments, 128 chunks.
fn g() -> BeginTransfer {
    begin(4_194_304, 8, 128)
}

/// The contract's `chunk(i)`: `Z16`, transfer 1, 32,768 bytes of `i`.
fn chunk(i: u8) -> TransferChunk {
    chunk_of(i, vec![i; CHUNK])
}

fn chunk_of(index: u8, bytes: Vec<u8>) -> TransferChunk {
    TransferChunk {
        epoch: Z16,
        transfer: tid(1),
        index,
        bytes,
    }
}

fn asm_g() -> TransferAssemblyV3 {
    TransferAssemblyV3::new(&g()).expect("G is a valid v3 Begin")
}

/// Whole-assembly equality without printing the buffers on failure.
fn assert_same(left: &TransferAssemblyV3, right: &TransferAssemblyV3, what: &str) {
    assert!(left == right, "assemblies differ: {what}");
}

/// Calls `accept` and checks the result and, for a refusal or `Duplicate`,
/// that the assembly equals its pre-call clone.
fn expect(
    asm: &mut TransferAssemblyV3,
    chunk: &TransferChunk,
    expected: Result<ChunkOutcome, AssemblyError>,
    what: &str,
) {
    let before = asm.clone();
    let got = asm.accept(chunk);
    assert_eq!(got, expected, "{what}");
    if got != Ok(ChunkOutcome::Stored) && got != Ok(ChunkOutcome::Complete) {
        assert_same(asm, &before, what);
    }
}

// ---------------------------------------------------------------------------
// E1: the full envelope.
// ---------------------------------------------------------------------------

#[test]
fn e1_assembly_full_envelope() {
    let mut asm = asm_g();
    assert_eq!(asm.begin(), &g());
    assert_eq!(asm.received_chunks(), 0);
    assert!(!asm.is_complete());

    // Reverse order: chunk 127 first, so an index past 31, 63 and 127 are all hit.
    let mut count = 0u8;
    for i in (1..=127u8).rev() {
        assert_eq!(asm.accept(&chunk(i)), Ok(ChunkOutcome::Stored), "chunk {i}");
        count += 1;
        assert_eq!(asm.received_chunks(), count, "after chunk {i}");
        assert!(!asm.is_complete(), "after chunk {i}");
    }
    assert_eq!(count, 127);

    // Before the last chunk, into_bytes hands back an equal assembly.
    let snapshot = asm.clone();
    match asm.clone().into_bytes() {
        Err(back) => assert_same(&back, &snapshot, "into_bytes before complete"),
        Ok(_) => panic!("into_bytes succeeded with 127 of 128 chunks"),
    }

    assert_eq!(asm.accept(&chunk(0)), Ok(ChunkOutcome::Complete));
    assert_eq!(asm.received_chunks(), 128);
    assert!(asm.is_complete());

    let expected: Vec<u8> = (0..128u8).flat_map(|i| vec![i; CHUNK]).collect();
    assert_eq!(expected.len(), 4_194_304);
    let bytes = asm.into_bytes().expect("complete");
    assert!(bytes == expected, "into_bytes is the 128 payloads in order");
}

#[test]
fn e1_bitmap_bits_past_31_and_63_do_not_alias() {
    let mut asm = asm_g();
    // Each of 0, 32, 64, 127 is Stored while the others are set: a bitmap
    // narrower than 128 bits, or a shift that wraps, reports Duplicate.
    for (n, i) in [0u8, 32, 64, 127].into_iter().enumerate() {
        assert_eq!(asm.accept(&chunk(i)), Ok(ChunkOutcome::Stored), "chunk {i}");
        assert_eq!(asm.received_chunks() as usize, n + 1);
    }
    // Neighbours of the stored bits are still unset.
    for i in [1u8, 31, 33, 63, 65, 126] {
        assert_eq!(asm.accept(&chunk(i)), Ok(ChunkOutcome::Stored), "chunk {i}");
    }
    assert_eq!(asm.received_chunks(), 10);
    // Every stored bit is remembered: identical is Duplicate, other is Conflict.
    for i in [0u8, 1, 31, 32, 33, 63, 64, 65, 126, 127] {
        expect(&mut asm, &chunk(i), Ok(ChunkOutcome::Duplicate), "dup");
        expect(
            &mut asm,
            &chunk_of(i, vec![0xEE; CHUNK]),
            Err(AssemblyError::Conflict),
            "conflict",
        );
    }
    assert_eq!(asm.received_chunks(), 10);
}

#[test]
fn e1_chunk_offsets_follow_segments_per_chunk() {
    // 3 segments per chunk: chunk size 12,288; the last chunk is partial.
    let size = 3 * MAX_SEGMENT_BYTES;
    let total = 127 * size + 100;
    let b = begin(total as u32, 3, 128);
    let mut asm = TransferAssemblyV3::new(&b).expect("valid v3 Begin");
    let mut expected = Vec::new();
    for i in 0..128u8 {
        let len = if i == 127 { 100 } else { size };
        expected.extend(std::iter::repeat_n(i, len));
    }
    assert_eq!(expected.len(), total);
    for i in (0..128u8).rev() {
        let len = if i == 127 { 100 } else { size };
        let outcome = if i == 0 {
            ChunkOutcome::Complete
        } else {
            ChunkOutcome::Stored
        };
        assert_eq!(
            asm.accept(&chunk_of(i, vec![i; len])),
            Ok(outcome),
            "chunk {i}"
        );
    }
    assert!(asm.is_complete());
    assert_eq!(asm.into_bytes(), Ok(expected));
}

// ---------------------------------------------------------------------------
// E2: new and accept refusals, with precedence.
// ---------------------------------------------------------------------------

#[test]
fn e2_new_refusals() {
    // Control: G itself, and one chunk under the envelope.
    assert!(TransferAssemblyV3::new(&g()).is_ok());
    assert!(TransferAssemblyV3::new(&begin(4_194_303, 8, 128)).is_ok());

    for (b, expected, what) in [
        (
            begin(4_194_305, 8, 128),
            CodecError::LimitExceeded,
            "total over 4 MiB",
        ),
        (
            begin(4_194_304, 8, 129),
            CodecError::LimitExceeded,
            "chunk count 129",
        ),
        (begin(0, 8, 128), CodecError::Malformed, "total 0"),
        (
            begin(4_194_304, 8, 127),
            CodecError::Malformed,
            "chunk count 127",
        ),
    ] {
        assert_eq!(TransferAssemblyV3::new(&b), Err(expected), "{what}");
    }
}

#[test]
fn e2_accept_refusals_and_precedence() {
    let mut asm = asm_g();
    let other_epoch = TransferChunk {
        epoch: [1; 16],
        ..chunk(5)
    };
    let other_id = TransferChunk {
        transfer: tid(2),
        ..chunk(5)
    };

    // Control: the same chunk 5 is accepted on a clone.
    assert_eq!(asm.clone().accept(&chunk(5)), Ok(ChunkOutcome::Stored));

    expect(
        &mut asm,
        &other_epoch,
        Err(AssemblyError::WrongTransfer),
        "epoch",
    );
    expect(&mut asm, &other_id, Err(AssemblyError::WrongTransfer), "id");
    expect(
        &mut asm,
        &chunk_of(128, vec![0; CHUNK]),
        Err(AssemblyError::OutOfRange),
        "index 128",
    );
    expect(
        &mut asm,
        &chunk_of(255, vec![0; CHUNK]),
        Err(AssemblyError::OutOfRange),
        "index 255",
    );
    for len in [0usize, 1, 32_767, 32_769] {
        expect(
            &mut asm,
            &chunk_of(5, vec![5; len]),
            Err(AssemblyError::WrongLength),
            "length",
        );
    }
    // Precedence: epoch before range, range before length.
    expect(
        &mut asm,
        &TransferChunk {
            epoch: [1; 16],
            ..chunk_of(200, vec![0; CHUNK])
        },
        Err(AssemblyError::WrongTransfer),
        "wrong epoch beats index 200",
    );
    expect(
        &mut asm,
        &TransferChunk {
            transfer: tid(2),
            ..chunk_of(200, vec![0; 7])
        },
        Err(AssemblyError::WrongTransfer),
        "wrong id beats index 200 and a wrong length",
    );
    expect(
        &mut asm,
        &chunk_of(200, vec![0; 7]),
        Err(AssemblyError::OutOfRange),
        "index 200 beats a wrong length",
    );
    expect(
        &mut asm,
        &TransferChunk {
            epoch: [1; 16],
            ..chunk_of(5, vec![0; 7])
        },
        Err(AssemblyError::WrongTransfer),
        "wrong epoch beats a wrong length",
    );

    // No refusal set a bit: chunk 5 is still unseen.
    assert_eq!(asm.received_chunks(), 0);
    assert_eq!(asm.accept(&chunk(5)), Ok(ChunkOutcome::Stored));
    assert_eq!(asm.received_chunks(), 1);

    // Stored: identical is Duplicate, other bytes are Conflict.
    expect(&mut asm, &chunk(5), Ok(ChunkOutcome::Duplicate), "repeat");
    expect(
        &mut asm,
        &chunk_of(5, vec![6; CHUNK]),
        Err(AssemblyError::Conflict),
        "other bytes",
    );
    let mut one_byte_off = vec![5u8; CHUNK];
    one_byte_off[CHUNK - 1] = 4;
    expect(
        &mut asm,
        &chunk_of(5, one_byte_off),
        Err(AssemblyError::Conflict),
        "last byte differs",
    );
    // Precedence on a stored index: the earlier checks still win.
    expect(
        &mut asm,
        &chunk_of(5, vec![5; 32_767]),
        Err(AssemblyError::WrongLength),
        "length beats Duplicate",
    );
    expect(
        &mut asm,
        &chunk_of(5, vec![6; 32_767]),
        Err(AssemblyError::WrongLength),
        "length beats Conflict",
    );
    expect(
        &mut asm,
        &other_epoch,
        Err(AssemblyError::WrongTransfer),
        "epoch beats Duplicate",
    );
    assert_eq!(asm.received_chunks(), 1);
    assert!(!asm.is_complete());
}

#[test]
fn e2_chunk_length_follows_the_begin_not_the_cap() {
    // 100,000 bytes at 8 segments: chunks of 32,768 x 3 and one of 1,696.
    let b = begin(100_000, 8, 4);
    let mut asm = TransferAssemblyV3::new(&b).expect("valid");
    // Index 4 is out of range for this Begin though under 128.
    expect(
        &mut asm,
        &chunk_of(4, vec![0; CHUNK]),
        Err(AssemblyError::OutOfRange),
        "index 4 of 4",
    );
    expect(
        &mut asm,
        &chunk_of(3, vec![3; CHUNK]),
        Err(AssemblyError::WrongLength),
        "a full chunk at the short tail",
    );
    expect(
        &mut asm,
        &chunk_of(3, vec![3; 1_695]),
        Err(AssemblyError::WrongLength),
        "one under the tail",
    );
    expect(
        &mut asm,
        &chunk_of(0, vec![0; 1_696]),
        Err(AssemblyError::WrongLength),
        "the tail length at a full index",
    );
    assert_eq!(
        asm.accept(&chunk_of(3, vec![3; 1_696])),
        Ok(ChunkOutcome::Stored)
    );
    for i in [2u8, 1] {
        assert_eq!(
            asm.accept(&chunk_of(i, vec![i; CHUNK])),
            Ok(ChunkOutcome::Stored)
        );
    }
    assert!(!asm.is_complete());
    assert_eq!(
        asm.accept(&chunk_of(0, vec![0; CHUNK])),
        Ok(ChunkOutcome::Complete)
    );
    assert!(asm.is_complete());
    let mut expected = Vec::new();
    for i in 0..3u8 {
        expected.extend(std::iter::repeat_n(i, CHUNK));
    }
    expected.extend(std::iter::repeat_n(3u8, 1_696));
    assert_eq!(asm.into_bytes(), Ok(expected));
}

// ---------------------------------------------------------------------------
// E3: call-by-call equality with the v1 assembly inside the v1 envelope.
// ---------------------------------------------------------------------------

fn compare_with_v1(b: &BeginTransfer, what: &str) {
    let mut v1 = TransferAssembly::new(b).unwrap_or_else(|e| panic!("{what}: v1 refuses: {e:?}"));
    let mut v3 = TransferAssemblyV3::new(b).unwrap_or_else(|e| panic!("{what}: v3 refuses: {e:?}"));
    assert_eq!(v1.begin(), v3.begin(), "{what}");

    let count = b.chunk_count;
    let size = b.segments_per_chunk as usize * MAX_SEGMENT_BYTES;
    let len = |i: u8| {
        let start = i as usize * size;
        (b.total_bytes as usize - start).min(size)
    };
    let mut calls: Vec<(String, TransferChunk)> = Vec::new();
    for i in (0..count).rev() {
        calls.push((format!("chunk {i}"), chunk_of(i, vec![i + 1; len(i)])));
    }
    calls.push((
        "identical duplicate".into(),
        chunk_of(count - 1, vec![count; len(count - 1)]),
    ));
    calls.push((
        "conflicting duplicate".into(),
        chunk_of(count - 1, vec![0xEE; len(count - 1)]),
    ));
    calls.push(("wrong length".into(), chunk_of(0, vec![1; len(0) + 1])));
    calls.push(("index 32".into(), chunk_of(32, vec![0; size])));
    calls.push((
        "wrong epoch".into(),
        TransferChunk {
            epoch: [1; 16],
            ..chunk_of(0, vec![1; len(0)])
        },
    ));
    calls.push((
        "wrong id".into(),
        TransferChunk {
            transfer: tid(2),
            ..chunk_of(0, vec![1; len(0)])
        },
    ));

    for (name, c) in &calls {
        let r1 = v1.accept(c);
        let r3 = v3.accept(c);
        assert_eq!(r3, r1, "{what}: {name}");
        assert_eq!(v3.received_chunks(), v1.received_chunks(), "{what}: {name}");
        assert_eq!(v3.is_complete(), v1.is_complete(), "{what}: {name}");
    }
    assert!(v3.is_complete(), "{what}");
    let expected: Vec<u8> = (0..count).flat_map(|i| vec![i + 1; len(i)]).collect();
    let b1 = v1
        .into_bytes()
        .unwrap_or_else(|_| panic!("{what}: v1 complete"));
    let b3 = v3
        .into_bytes()
        .unwrap_or_else(|_| panic!("{what}: v3 complete"));
    assert!(b1 == expected, "{what}: v1 bytes");
    assert!(b3 == b1, "{what}: v3 bytes equal v1 bytes");
}

#[test]
fn e3_assembly_matches_v1_inside_v1_caps() {
    // D1's empty-body Begin, then the 1 MiB / 32-chunk Begin, then a partial tail.
    compare_with_v1(&begin(6, 1, 1), "empty body");
    compare_with_v1(&begin(MIB as u32, 8, 32), "1 MiB");
    compare_with_v1(&begin(100_000, 8, 4), "partial tail");
}

#[test]
fn e3_incomplete_assemblies_agree() {
    let b = begin(MIB as u32, 8, 32);
    let mut v1 = TransferAssembly::new(&b).unwrap();
    let mut v3 = TransferAssemblyV3::new(&b).unwrap();
    for i in [31u8, 0, 17] {
        let c = chunk(i);
        assert_eq!(v3.accept(&c), v1.accept(&c));
    }
    assert_eq!(v3.received_chunks(), 3);
    assert_eq!(v1.received_chunks(), 3);
    assert!(!v3.is_complete() && !v1.is_complete());
    let r1 = v1.into_bytes().expect_err("v1 incomplete");
    let r3 = v3.into_bytes().expect_err("v3 incomplete");
    assert_eq!(r3.received_chunks(), r1.received_chunks());
    assert_eq!(r3.begin(), r1.begin());
    // New refusals agree too, where both are inside the v1 envelope.
    for b in [
        begin(0, 8, 32),
        begin(MIB as u32, 8, 31),
        begin(MIB as u32, 0, 32),
        begin(MIB as u32, 9, 32),
    ] {
        assert_eq!(
            TransferAssemblyV3::new(&b).map(|_| ()),
            TransferAssembly::new(&b).map(|_| ()),
            "{b:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// E4: the largest valid plan (the D5 body, copied from tests/v3_snapshot.rs).
// ---------------------------------------------------------------------------

fn u(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

fn net(n: u64) -> NetId {
    NetId::new(n).expect("fixture net id is nonzero")
}

fn pool(id: u128, current: u32, max: u32) -> PoolFact {
    PoolFact {
        pool: u(id),
        current,
        max,
    }
}

fn effect(id: u128, expires_round: u64) -> EffectFact {
    EffectFact {
        effect: u(id),
        expires_round,
    }
}

fn stat(name: &str, value: StatValue) -> StatFact {
    StatFact {
        name: name.to_owned(),
        value,
    }
}

fn tl(key: i32, entity: u64) -> TimelineEntry {
    TimelineEntry {
        key,
        entity: net(entity),
    }
}

fn largest_body() -> SnapshotBodyV3 {
    let base = 1u64 << 63;
    let entities = (0..1024u64)
        .map(|i| SnapshotEntityV3 {
            entity: net(base + i),
            health: Some(HealthFact {
                health: u32::MAX,
                max_health: u32::MAX,
                dead: true,
            }),
            facts: FactSet {
                pools: Some((1..=16).map(|n| pool(n, u32::MAX, u32::MAX)).collect()),
                effects: Some((1..=16).map(|n| effect(n, u64::MAX)).collect()),
                order: Some(Some(OrderFact {
                    ability: u(1),
                    target: net(base),
                    repeat: true,
                })),
                stats: Some(
                    (0..32)
                        .map(|n| stat(&format!("{n:032}"), StatValue::Int(i32::MIN)))
                        .collect(),
                ),
                tags: Some((0..8).map(|n| format!("{n:032}")).collect()),
            },
        })
        .collect();
    SnapshotBodyV3 {
        entities,
        turn: Some(TurnFact {
            active: Some(net(base)),
            round: u64::MAX,
        }),
        encounter: EncounterFacts {
            mode: Some(EncounterMode::RealTime),
            pending_mode: Some(Some(EncounterMode::TurnBased)),
            clock: Some(Some(ClockFact {
                round_ticks: u32::MAX,
                elapsed: 2_147_483_647,
            })),
        },
        timeline: Some((0..1024u64).map(|i| tl(i32::MIN, base + i)).collect()),
    }
}

/// The largest body and its encoding, built once.
fn largest() -> &'static (SnapshotBodyV3, Vec<u8>) {
    static CELL: OnceLock<(SnapshotBodyV3, Vec<u8>)> = OnceLock::new();
    CELL.get_or_init(|| {
        let body = largest_body();
        let bytes = encode_body(&body).expect("the largest valid body encodes");
        (body, bytes)
    })
}

#[test]
fn e4_assembly_rebuilds_largest_plan() {
    let (body, encoded) = largest();
    assert_eq!(encoded.len(), 2_847_788);
    let frames = plan_transfer(Z16, tid(1), 0, 0, 8, body).expect("the largest body plans");
    assert_eq!(frames.len(), 88);

    let Ok(TransferDown::Begin(b)) = decode_down(&frames[0]) else {
        panic!("the first frame is a Begin");
    };
    assert_eq!(b.total_bytes, 2_847_788);
    assert_eq!(b.chunk_count, 87);
    let mut asm = TransferAssemblyV3::new(&b).expect("the plan's Begin is a valid v3 Begin");

    // Control: the v1 assembly refuses this very Begin (over 1 MiB, 87 > 32).
    assert_eq!(
        TransferAssembly::new(&b).map(|_| ()),
        Err(CodecError::LimitExceeded)
    );

    let chunks: Vec<TransferChunk> = frames[1..]
        .iter()
        .enumerate()
        .map(|(position, frame)| {
            let Ok(TransferDown::Chunk(c)) = decode_down(frame) else {
                panic!("frame {} is a Chunk", position + 1);
            };
            assert_eq!(c.index as usize, position);
            c
        })
        .collect();
    assert_eq!(chunks.len(), 87);
    assert_eq!(chunks[86].bytes.len(), 29_740);

    let mut stored = 0u8;
    for c in chunks.iter().skip(1).rev() {
        assert_eq!(asm.accept(c), Ok(ChunkOutcome::Stored), "chunk {}", c.index);
        stored += 1;
        assert_eq!(asm.received_chunks(), stored);
        assert!(!asm.is_complete());
    }
    assert_eq!(stored, 86);
    assert_eq!(asm.accept(&chunks[0]), Ok(ChunkOutcome::Complete));
    assert_eq!(asm.received_chunks(), 87);
    assert!(asm.is_complete());
    let bytes = asm.into_bytes().expect("complete");
    assert_eq!(bytes.len(), 2_847_788);
    assert!(
        bytes == *encoded,
        "into_bytes equals encode_body of the body"
    );
}

// ---------------------------------------------------------------------------
// E5: TransferLimitsV3.
// ---------------------------------------------------------------------------

fn v3_with(edit: impl FnOnce(&mut TransferLimitsV3)) -> TransferLimitsV3 {
    let mut limits = TransferLimitsV3::v3();
    edit(&mut limits);
    limits
}

fn invalid(field: &'static str) -> Result<(), LimitsError> {
    Err(LimitsError::InvalidLimits { field })
}

fn loosens(field: &'static str) -> Result<(), LimitsError> {
    Err(LimitsError::LoosensPolicy { field })
}

#[test]
fn e5_limits_v3_values() {
    let v = TransferLimitsV3::v3();
    assert_eq!(
        v,
        TransferLimitsV3 {
            max_receivers: 8,
            deadline_ms: 20_000,
            staging_bytes_per_receiver: 4_194_304,
            staging_bytes_total: 33_554_432,
            delta_frames_per_receiver: 128,
            delta_bytes_per_receiver: 2_097_152,
            delta_bytes_total: 16_777_216,
        }
    );
    assert_eq!(v.validate(), Ok(()));
    assert_eq!(POLICY_SNAPSHOT_REASSEMBLY_MS_V3, 20_000);
    assert_eq!(POLICY_SNAPSHOT_STAGING_BYTES_PER_PEER_V3, 4_194_304);
    assert_eq!(POLICY_SNAPSHOT_STAGING_BYTES_HOST_V3, 33_554_432);
    assert_eq!(MAX_ENCODED_BODY_BYTES_V3, 2_847_788);
}

#[test]
fn e5_limits_v3_bounds() {
    // (label, edit, expected) for each field at its floor, one under, its
    // upper bound and one over.
    type Edit = fn(&mut TransferLimitsV3);
    let cases: Vec<(&str, Edit, Result<(), LimitsError>)> = vec![
        (
            "receivers 0",
            |l| l.max_receivers = 0,
            invalid("max_receivers"),
        ),
        ("receivers 1", |l| l.max_receivers = 1, Ok(())),
        ("receivers 8", |l| l.max_receivers = 8, Ok(())),
        (
            "receivers 9",
            |l| l.max_receivers = 9,
            loosens("max_receivers"),
        ),
        ("deadline 0", |l| l.deadline_ms = 0, invalid("deadline_ms")),
        ("deadline 1", |l| l.deadline_ms = 1, Ok(())),
        ("deadline 20000", |l| l.deadline_ms = 20_000, Ok(())),
        (
            "deadline 20001",
            |l| l.deadline_ms = 20_001,
            loosens("deadline_ms"),
        ),
        (
            "per receiver 0",
            |l| l.staging_bytes_per_receiver = 0,
            invalid("staging_bytes_per_receiver"),
        ),
        (
            "per receiver 22552 (the v1 floor)",
            |l| l.staging_bytes_per_receiver = 22_552,
            invalid("staging_bytes_per_receiver"),
        ),
        (
            "per receiver 2847787",
            |l| l.staging_bytes_per_receiver = 2_847_787,
            invalid("staging_bytes_per_receiver"),
        ),
        (
            "per receiver 2847788",
            |l| l.staging_bytes_per_receiver = 2_847_788,
            Ok(()),
        ),
        (
            "per receiver 4194304",
            |l| l.staging_bytes_per_receiver = 4_194_304,
            Ok(()),
        ),
        (
            "per receiver 4194305",
            |l| l.staging_bytes_per_receiver = 4_194_305,
            loosens("staging_bytes_per_receiver"),
        ),
        (
            "staging total 0",
            |l| l.staging_bytes_total = 0,
            invalid("staging_bytes_total"),
        ),
        (
            "staging total 4194303",
            |l| l.staging_bytes_total = 4_194_303,
            invalid("staging_bytes_total"),
        ),
        (
            "staging total 4194304",
            |l| l.staging_bytes_total = 4_194_304,
            Ok(()),
        ),
        (
            "staging total 33554432",
            |l| l.staging_bytes_total = 33_554_432,
            Ok(()),
        ),
        (
            "staging total 33554433",
            |l| l.staging_bytes_total = 33_554_433,
            loosens("staging_bytes_total"),
        ),
        (
            "frames 0",
            |l| l.delta_frames_per_receiver = 0,
            invalid("delta_frames_per_receiver"),
        ),
        ("frames 1", |l| l.delta_frames_per_receiver = 1, Ok(())),
        ("frames 128", |l| l.delta_frames_per_receiver = 128, Ok(())),
        (
            "frames 129",
            |l| l.delta_frames_per_receiver = 129,
            loosens("delta_frames_per_receiver"),
        ),
        (
            "delta per receiver 0",
            |l| l.delta_bytes_per_receiver = 0,
            invalid("delta_bytes_per_receiver"),
        ),
        (
            "delta per receiver 65535",
            |l| l.delta_bytes_per_receiver = 65_535,
            invalid("delta_bytes_per_receiver"),
        ),
        (
            "delta per receiver 65536",
            |l| l.delta_bytes_per_receiver = 65_536,
            Ok(()),
        ),
        (
            "delta per receiver 2097152",
            |l| l.delta_bytes_per_receiver = 2_097_152,
            Ok(()),
        ),
        (
            "delta per receiver 2097153",
            |l| l.delta_bytes_per_receiver = 2_097_153,
            loosens("delta_bytes_per_receiver"),
        ),
        (
            "delta total 0",
            |l| l.delta_bytes_total = 0,
            invalid("delta_bytes_total"),
        ),
        (
            "delta total 2097151",
            |l| l.delta_bytes_total = 2_097_151,
            invalid("delta_bytes_total"),
        ),
        (
            "delta total 2097152",
            |l| l.delta_bytes_total = 2_097_152,
            Ok(()),
        ),
        (
            "delta total 16777216",
            |l| l.delta_bytes_total = 16_777_216,
            Ok(()),
        ),
        (
            "delta total 16777217",
            |l| l.delta_bytes_total = 16_777_217,
            loosens("delta_bytes_total"),
        ),
    ];
    for (label, edit, expected) in cases {
        assert_eq!(v3_with(edit).validate(), expected, "{label}");
    }
}

#[test]
fn e5_limits_v3_cross_field_floors() {
    // A total is bounded below by its own per-receiver value, not by a constant.
    let low_per = |l: &mut TransferLimitsV3| l.staging_bytes_per_receiver = 2_847_788;
    assert_eq!(
        v3_with(|l| {
            low_per(l);
            l.staging_bytes_total = 2_847_788;
        })
        .validate(),
        Ok(())
    );
    assert_eq!(
        v3_with(|l| {
            low_per(l);
            l.staging_bytes_total = 2_847_787;
        })
        .validate(),
        invalid("staging_bytes_total")
    );
    let low_delta = |l: &mut TransferLimitsV3| l.delta_bytes_per_receiver = 65_536;
    assert_eq!(
        v3_with(|l| {
            low_delta(l);
            l.delta_bytes_total = 65_536;
        })
        .validate(),
        Ok(())
    );
    assert_eq!(
        v3_with(|l| {
            low_delta(l);
            l.delta_bytes_total = 65_535;
        })
        .validate(),
        invalid("delta_bytes_total")
    );
}

#[test]
fn e5_limits_v3_precedence() {
    // The contract's two cases.
    assert_eq!(
        v3_with(|l| {
            l.max_receivers = 0;
            l.deadline_ms = 0;
        })
        .validate(),
        invalid("max_receivers")
    );
    assert_eq!(
        v3_with(|l| {
            l.deadline_ms = 20_001;
            l.staging_bytes_per_receiver = 0;
        })
        .validate(),
        loosens("deadline_ms")
    );
    // A Loosens on an earlier field beats an Invalid on a later one, and the
    // reverse order of the same pair reports the earlier field again.
    assert_eq!(
        v3_with(|l| {
            l.max_receivers = 9;
            l.deadline_ms = 0;
        })
        .validate(),
        loosens("max_receivers")
    );
    assert_eq!(
        v3_with(|l| {
            l.max_receivers = 0;
            l.deadline_ms = 20_001;
        })
        .validate(),
        invalid("max_receivers")
    );

    // All seven fields bad (Invalid where a floor exists): repairing them in
    // declaration order moves the reported field down the declaration order.
    let fixed = TransferLimitsV3::v3();
    let invalid_chain = TransferLimitsV3 {
        max_receivers: 0,
        deadline_ms: 0,
        staging_bytes_per_receiver: 2_847_787,
        staging_bytes_total: 4_194_303,
        delta_frames_per_receiver: 0,
        delta_bytes_per_receiver: 65_535,
        delta_bytes_total: 2_097_151,
    };
    let loosen_chain = TransferLimitsV3 {
        max_receivers: 9,
        deadline_ms: 20_001,
        staging_bytes_per_receiver: 4_194_305,
        staging_bytes_total: 33_554_433,
        delta_frames_per_receiver: 129,
        delta_bytes_per_receiver: 2_097_153,
        delta_bytes_total: 16_777_217,
    };
    for (start, kind) in [(invalid_chain, "invalid"), (loosen_chain, "loosens")] {
        let report = |field: &'static str| {
            if kind == "invalid" {
                invalid(field)
            } else {
                loosens(field)
            }
        };
        let mut l = start;
        assert_eq!(l.validate(), report("max_receivers"), "{kind} 1");
        l.max_receivers = fixed.max_receivers;
        assert_eq!(l.validate(), report("deadline_ms"), "{kind} 2");
        l.deadline_ms = fixed.deadline_ms;
        assert_eq!(
            l.validate(),
            report("staging_bytes_per_receiver"),
            "{kind} 3"
        );
        l.staging_bytes_per_receiver = fixed.staging_bytes_per_receiver;
        assert_eq!(l.validate(), report("staging_bytes_total"), "{kind} 4");
        l.staging_bytes_total = fixed.staging_bytes_total;
        assert_eq!(
            l.validate(),
            report("delta_frames_per_receiver"),
            "{kind} 5"
        );
        l.delta_frames_per_receiver = fixed.delta_frames_per_receiver;
        assert_eq!(l.validate(), report("delta_bytes_per_receiver"), "{kind} 6");
        l.delta_bytes_per_receiver = fixed.delta_bytes_per_receiver;
        assert_eq!(l.validate(), report("delta_bytes_total"), "{kind} 7");
        l.delta_bytes_total = fixed.delta_bytes_total;
        assert_eq!(l.validate(), Ok(()), "{kind} fixed");
        assert_eq!(l, fixed);
    }
}

#[test]
fn e5_limits_v3_display() {
    assert_eq!(
        LimitsError::LoosensPolicy {
            field: "deadline_ms"
        }
        .to_string(),
        "LoosensPolicy at transfer/limits: deadline_ms"
    );
    let Err(err) = v3_with(|l| l.deadline_ms = 20_001).validate() else {
        panic!("20,001 loosens the policy");
    };
    assert_eq!(
        err.to_string(),
        "LoosensPolicy at transfer/limits: deadline_ms"
    );
    let Err(err) = v3_with(|l| l.max_receivers = 0).validate() else {
        panic!("0 receivers is invalid");
    };
    assert_eq!(
        err.to_string(),
        "InvalidLimits at transfer/limits: max_receivers"
    );
}

// ---------------------------------------------------------------------------
// E6: v1 sizing is untouched.
// ---------------------------------------------------------------------------

#[test]
fn e6_v1_sizing_unchanged() {
    let v1 = TransferLimits::v1();
    assert_eq!(
        v1,
        TransferLimits {
            max_receivers: 8,
            deadline_ms: 5_000,
            staging_bytes_per_receiver: 1_048_576,
            staging_bytes_total: 8_388_608,
            delta_frames_per_receiver: 128,
            delta_bytes_per_receiver: 2_097_152,
            delta_bytes_total: 16_777_216,
        }
    );
    assert_eq!(v1.validate(), Ok(()));

    // Controls: the v1 bounds themselves validate.
    assert_eq!(
        TransferLimits {
            deadline_ms: 5_000,
            staging_bytes_per_receiver: 1_048_576,
            ..TransferLimits::v1()
        }
        .validate(),
        Ok(())
    );
    // The v3 values loosen the v1 policy.
    assert_eq!(
        TransferLimits {
            staging_bytes_per_receiver: 4_194_304,
            ..TransferLimits::v1()
        }
        .validate(),
        loosens("staging_bytes_per_receiver")
    );
    assert_eq!(
        TransferLimits {
            deadline_ms: 20_000,
            ..TransferLimits::v1()
        }
        .validate(),
        loosens("deadline_ms")
    );
    assert_eq!(
        TransferLimits {
            staging_bytes_per_receiver: 1_048_577,
            ..TransferLimits::v1()
        }
        .validate(),
        loosens("staging_bytes_per_receiver")
    );
    assert_eq!(
        TransferLimits {
            deadline_ms: 5_001,
            ..TransferLimits::v1()
        }
        .validate(),
        loosens("deadline_ms")
    );
    assert_eq!(
        TransferLimits {
            staging_bytes_total: 33_554_432,
            ..TransferLimits::v1()
        }
        .validate(),
        loosens("staging_bytes_total")
    );
    // And v3's own limits accept what v1 refuses.
    assert_eq!(TransferLimitsV3::v3().validate(), Ok(()));

    // The v1 assembly still refuses the v3 envelope; the control is a Begin
    // that fits it.
    assert_eq!(
        TransferAssembly::new(&g()).map(|_| ()),
        Err(CodecError::LimitExceeded)
    );
    assert!(TransferAssembly::new(&begin(MIB as u32, 8, 32)).is_ok());
    assert_eq!(
        TransferAssembly::new(&begin(MIB as u32 + 1, 8, 33)).map(|_| ()),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(g().validate(), Err(CodecError::LimitExceeded));
    assert_eq!(begin(MIB as u32, 8, 32).validate(), Ok(()));
    // The same 33-chunk Begin is a valid v3 Begin.
    assert!(TransferAssemblyV3::new(&begin(MIB as u32 + 1, 8, 33)).is_ok());
}
