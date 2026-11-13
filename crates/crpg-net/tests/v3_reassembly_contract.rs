//! T140e two-keys suite: v3 receiver sizing, written from `tasks/T140.md`
//! (Behaviour E-1..E-5, Tests E1..E6) and the public API only.
//!
//! Expected values are contract literals or hand arithmetic, never output of
//! the code under test. Every refusal has an exact error and a passing control.

use crpg_core::Ulid;
use crpg_net::codec::CodecError::{LimitExceeded, Malformed};
use crpg_net::protocol::MAX_SNAPSHOT_BYTES;
use crpg_net::protocol_transfer::{
    BeginTransfer, HealthFact, TransferChunk, TransferDown, TransferId, TurnFact,
};
use crpg_net::protocol_v3::{
    ClockFact, EffectFact, EncounterFacts, EncounterMode, FactSet, NetId, OrderFact, PoolFact,
    StatFact, StatValue, TimelineEntry,
};
use crpg_net::reassembly::{
    AssemblyError, ChunkOutcome, LimitsError, TransferAssembly, TransferLimits,
};
use crpg_net::reassembly_v3::{
    TransferAssemblyV3, TransferLimitsV3, POLICY_SNAPSHOT_REASSEMBLY_MS_V3,
    POLICY_SNAPSHOT_STAGING_BYTES_HOST_V3, POLICY_SNAPSHOT_STAGING_BYTES_PER_PEER_V3,
};
use crpg_net::transfer_v3 as t3;
use crpg_net::transfer_v3::{SnapshotBodyV3, SnapshotEntityV3};

type Step = Result<ChunkOutcome, AssemblyError>;
const Z: [u8; 16] = [0; 16];
const STORED: Step = Ok(ChunkOutcome::Stored);
const COMPLETE: Step = Ok(ChunkOutcome::Complete);
const DUPLICATE: Step = Ok(ChunkOutcome::Duplicate);

fn tid(n: u64) -> TransferId {
    TransferId::new(n).expect("nonzero transfer id")
}

fn nid(n: u64) -> NetId {
    NetId::new(n).expect("nonzero net id")
}

fn begin(total: u32, spc: u8, chunks: u8) -> BeginTransfer {
    BeginTransfer {
        epoch: Z,
        transfer: tid(1),
        base_tick: 0,
        cursor: 0,
        total_bytes: total,
        entity_count: 0,
        segments_per_chunk: spc,
        chunk_count: chunks,
    }
}

/// `G` of the contract: 4 MiB, 8 segments, 128 chunks.
fn g() -> BeginTransfer {
    begin(4_194_304, 8, 128)
}

fn raw(epoch: [u8; 16], id: u64, index: u8, bytes: Vec<u8>) -> TransferChunk {
    TransferChunk {
        epoch,
        transfer: tid(id),
        index,
        bytes,
    }
}

/// `chunk(i)` of the contract: 32,768 bytes of `i`.
fn chunk(i: u8) -> TransferChunk {
    raw(Z, 1, i, vec![i; 32_768])
}

fn fresh() -> TransferAssemblyV3 {
    TransferAssemblyV3::new(&g()).expect("G is a valid v3 Begin")
}

/// Accept `c`, and when the call is an `Err` or `Duplicate`, require that the
/// assembly equals its pre-call clone.
fn refused(asm: &mut TransferAssemblyV3, c: &TransferChunk) -> Step {
    let before = asm.clone();
    let result = asm.accept(c);
    assert_eq!(*asm, before, "a refusal or duplicate must change nothing");
    result
}

fn inv(field: &'static str) -> Result<(), LimitsError> {
    Err(LimitsError::InvalidLimits { field })
}

fn loose(field: &'static str) -> Result<(), LimitsError> {
    Err(LimitsError::LoosensPolicy { field })
}

// E1 / E-1 / E-2: the full envelope.

#[test]
fn e1_assembly_full_envelope() {
    let mut asm = fresh();
    assert_eq!(asm.begin(), &g());
    assert_eq!(asm.received_chunks(), 0);
    assert!(!asm.is_complete());
    assert_eq!(asm.clone().into_bytes(), Err(asm.clone()));
    for i in (1..=127u8).rev() {
        assert_eq!(asm.accept(&chunk(i)), STORED, "chunk {i}");
        assert_eq!(asm.received_chunks(), 128 - i);
        assert!(!asm.is_complete());
    }
    assert_eq!(asm.clone().into_bytes(), Err(asm.clone()));
    assert_eq!(asm.accept(&chunk(0)), COMPLETE);
    assert_eq!(asm.received_chunks(), 128);
    assert!(asm.is_complete());
    assert_eq!(refused(&mut asm, &chunk(0)), DUPLICATE);
    assert_eq!(asm.received_chunks(), 128);
    let expected: Vec<u8> = (0..128u8).flat_map(|i| vec![i; 32_768]).collect();
    assert_eq!(expected.len(), 4_194_304);
    assert_eq!(asm.into_bytes(), Ok(expected));
}

#[test]
fn e1_every_bitmap_position_is_distinct() {
    // Indices on both sides of the u32, u64 and u128 word boundaries.
    let probes = [0u8, 1, 31, 32, 33, 63, 64, 65, 96, 100, 126, 127];
    let mut asm = fresh();
    for (n, &i) in probes.iter().enumerate() {
        assert_eq!(
            asm.accept(&chunk(i)),
            STORED,
            "chunk {i} aliases an earlier bit"
        );
        assert_eq!(usize::from(asm.received_chunks()), n + 1);
    }
    for &i in &probes {
        assert_eq!(refused(&mut asm, &chunk(i)), DUPLICATE, "chunk {i}");
        let mut other = chunk(i);
        other.bytes[32_767] ^= 1;
        assert_eq!(
            refused(&mut asm, &other),
            Err(AssemblyError::Conflict),
            "chunk {i}"
        );
        assert_eq!(usize::from(asm.received_chunks()), probes.len());
    }
    // A bit never set is not mistaken for one that is.
    assert_eq!(asm.accept(&chunk(2)), STORED);
    assert_eq!(asm.accept(&chunk(95)), STORED);
    assert_eq!(usize::from(asm.received_chunks()), probes.len() + 2);
    // Forward order completes too, with the last bit being 127.
    let mut fwd = fresh();
    for i in 0..127u8 {
        assert_eq!(fwd.accept(&chunk(i)), STORED);
    }
    assert!(!fwd.is_complete());
    assert_eq!(fwd.accept(&chunk(127)), COMPLETE);
    assert!(fwd.is_complete());
}

#[test]
fn e2_begin_refusals() {
    let cases = [
        (begin(4_194_305, 8, 128), LimitExceeded),
        (begin(4_194_304, 8, 129), LimitExceeded),
        (begin(0, 8, 128), Malformed),
        (begin(4_194_304, 8, 127), Malformed),
    ];
    for (bad, error) in cases {
        assert_eq!(t3::check_begin(&bad), Err(error), "{bad:?}");
        assert_eq!(TransferAssemblyV3::new(&bad).err(), Some(error), "{bad:?}");
    }
    assert_eq!(t3::check_begin(&g()), Ok(()));
    assert!(TransferAssemblyV3::new(&g()).is_ok());
}

#[test]
fn e2_chunk_refusals_and_precedence() {
    let bad_epoch = [1u8; 16];
    let mut last_byte = Z;
    last_byte[15] = 1;
    let mut asm = fresh();
    let wrong = Err(AssemblyError::WrongTransfer);
    let range = Err(AssemblyError::OutOfRange);
    let length = Err(AssemblyError::WrongLength);
    // (1) epoch or transfer differs.
    assert_eq!(
        refused(&mut asm, &raw(bad_epoch, 1, 5, vec![5; 32_768])),
        wrong
    );
    assert_eq!(
        refused(&mut asm, &raw(last_byte, 1, 5, vec![5; 32_768])),
        wrong
    );
    assert_eq!(refused(&mut asm, &raw(Z, 2, 5, vec![5; 32_768])), wrong);
    // (2) index at or past the chunk count.
    assert_eq!(refused(&mut asm, &raw(Z, 1, 128, vec![0; 32_768])), range);
    assert_eq!(refused(&mut asm, &raw(Z, 1, 255, vec![0; 32_768])), range);
    // (3) byte length differs from `chunk_len`.
    for len in [0, 1, 32_767, 32_769, 65_536] {
        assert_eq!(
            refused(&mut asm, &raw(Z, 1, 5, vec![5; len])),
            length,
            "len {len}"
        );
    }
    assert_eq!(refused(&mut asm, &raw(Z, 1, 127, vec![7; 32_767])), length);
    // Precedence: 1 beats 2 and 3, 2 beats 3.
    assert_eq!(
        refused(&mut asm, &raw(bad_epoch, 1, 200, vec![0; 32_768])),
        wrong
    );
    assert_eq!(refused(&mut asm, &raw(Z, 2, 200, vec![0; 32_768])), wrong);
    assert_eq!(refused(&mut asm, &raw(bad_epoch, 1, 5, vec![5; 3])), wrong);
    assert_eq!(refused(&mut asm, &raw(Z, 1, 200, vec![0; 32_767])), range);
    assert_eq!(refused(&mut asm, &raw(Z, 1, 128, Vec::new())), range);
    // Nothing above stored anything, and the control is accepted.
    assert_eq!(asm.received_chunks(), 0);
    assert_eq!(asm.accept(&chunk(5)), STORED);
    assert_eq!(asm.received_chunks(), 1);
    // (4) a set bit: identical bytes are Duplicate, other bytes are Conflict.
    assert_eq!(refused(&mut asm, &chunk(5)), DUPLICATE);
    for at in [0, 16_384, 32_767] {
        let mut other = chunk(5);
        other.bytes[at] = 6;
        assert_eq!(
            refused(&mut asm, &other),
            Err(AssemblyError::Conflict),
            "byte {at}"
        );
    }
    assert_eq!(
        refused(&mut asm, &raw(Z, 1, 5, vec![6; 32_768])),
        Err(AssemblyError::Conflict)
    );
    // Checks 1 to 3 still run before check 4 on a stored index.
    assert_eq!(refused(&mut asm, &raw(Z, 1, 5, vec![5; 32_767])), length);
    assert_eq!(refused(&mut asm, &raw(Z, 2, 5, vec![5; 32_768])), wrong);
    assert_eq!(asm.received_chunks(), 1);
    assert_eq!(asm.accept(&chunk(6)), STORED);
    assert_eq!(asm.received_chunks(), 2);
    assert!(!asm.is_complete());
}

// E3 / E-3: v3 equals v1 inside the v1 envelope.

fn run_both(begin: &BeginTransfer, seq: &[TransferChunk], want: &[Step], bytes: Vec<u8>) {
    assert_eq!(seq.len(), want.len());
    let mut a3 = TransferAssemblyV3::new(begin).expect("v3 accepts a v1-valid Begin");
    let mut a1 = TransferAssembly::new(begin).expect("v1 accepts the Begin");
    assert_eq!(a3.begin(), a1.begin());
    for (n, (c, expected)) in seq.iter().zip(want).enumerate() {
        assert_eq!(a3.accept(c), *expected, "v3 call {n}");
        assert_eq!(a1.accept(c), *expected, "v1 call {n}");
        assert_eq!(a3.received_chunks(), a1.received_chunks(), "call {n}");
        assert_eq!(a3.is_complete(), a1.is_complete(), "call {n}");
        if n == 0 && want.len() > 5 {
            assert!(a3.clone().into_bytes().is_err());
            assert!(a1.clone().into_bytes().is_err());
        }
    }
    assert!(a3.is_complete());
    assert_eq!(a3.into_bytes().ok(), Some(bytes.clone()));
    assert_eq!(a1.into_bytes().ok(), Some(bytes));
}

#[test]
fn e3_assembly_matches_v1_inside_v1_caps() {
    // D1's empty-body Begin: id 1, total 6, 1 segment, 1 chunk.
    let empty = begin(6, 1, 1);
    let at0 = |bytes: Vec<u8>| raw(Z, 1, 0, bytes);
    let mut conflict = vec![0u8; 6];
    conflict[5] = 1;
    run_both(
        &empty,
        &[
            at0(vec![0; 6]),
            at0(vec![0; 6]),
            at0(conflict),
            at0(vec![0; 5]),
            raw(Z, 1, 32, vec![0; 6]),
        ],
        &[
            COMPLETE,
            DUPLICATE,
            Err(AssemblyError::Conflict),
            Err(AssemblyError::WrongLength),
            Err(AssemblyError::OutOfRange),
        ],
        vec![0; 6],
    );
    // 1 MiB, 8 segments, 32 chunks, in reverse order, then the four refusals.
    let mut seq: Vec<TransferChunk> = (0..32u8).rev().map(chunk).collect();
    let mut other = chunk(0);
    other.bytes[0] = 1;
    seq.extend([
        chunk(0),
        other,
        raw(Z, 1, 0, vec![0; 100]),
        raw(Z, 1, 32, vec![32; 32_768]),
    ]);
    let mut want = vec![STORED; 31];
    want.extend([
        COMPLETE,
        DUPLICATE,
        Err(AssemblyError::Conflict),
        Err(AssemblyError::WrongLength),
        Err(AssemblyError::OutOfRange),
    ]);
    let bytes: Vec<u8> = (0..32u8).flat_map(|i| vec![i; 32_768]).collect();
    run_both(&begin(1_048_576, 8, 32), &seq, &want, bytes);
}

// E4 / E-2: the largest D5 body rebuilt from its plan.

fn pool(id: u128, current: u32, max: u32) -> PoolFact {
    PoolFact {
        pool: Ulid::from_u128(id),
        current,
        max,
    }
}

/// The D5 body: 1,024 full entities, a turn, a full encounter, a timeline.
fn largest_body() -> SnapshotBodyV3 {
    let base: u64 = 1 << 63;
    let entities = (0..1024u64)
        .map(|i| SnapshotEntityV3 {
            entity: nid(base + i),
            health: Some(HealthFact {
                health: u32::MAX,
                max_health: u32::MAX,
                dead: true,
            }),
            facts: FactSet {
                pools: Some((0..16).map(|j| pool(100 + j, u32::MAX, u32::MAX)).collect()),
                effects: Some(
                    (0..16)
                        .map(|j| EffectFact {
                            effect: Ulid::from_u128(200 + j),
                            expires_round: u64::MAX,
                        })
                        .collect(),
                ),
                order: Some(Some(OrderFact {
                    ability: Ulid::from_u128(300),
                    target: nid(base),
                    repeat: true,
                })),
                stats: Some(
                    (0..32)
                        .map(|j| StatFact {
                            name: format!("s{j:031}"),
                            value: StatValue::Int(i32::MIN),
                        })
                        .collect(),
                ),
                tags: Some((0..8).map(|j| format!("t{j:031}")).collect()),
            },
        })
        .collect();
    SnapshotBodyV3 {
        entities,
        turn: Some(TurnFact {
            active: Some(nid(base)),
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
        timeline: Some(
            (0..1024u64)
                .map(|i| TimelineEntry {
                    key: i32::MIN,
                    entity: nid(base + i),
                })
                .collect(),
        ),
    }
}

#[test]
fn e4_assembly_rebuilds_largest_plan() {
    let body = largest_body();
    let encoded = t3::encode_body(&body).expect("the largest valid body encodes");
    assert_eq!(encoded.len(), 2_847_788);
    let plan = t3::plan_transfer([0; 16], tid(1), 0, 0, 8, &body).expect("plans at 8 segments");
    assert_eq!(plan.len(), 88);
    let Ok(TransferDown::Begin(b)) = t3::decode_down(&plan[0]) else {
        panic!("the first frame is a Begin");
    };
    assert_eq!((b.total_bytes, b.chunk_count), (2_847_788, 87));
    let chunks: Vec<TransferChunk> = plan[1..]
        .iter()
        .map(|frame| match t3::decode_down(frame) {
            Ok(TransferDown::Chunk(c)) => c,
            other => panic!("expected a Chunk, got {other:?}"),
        })
        .collect();
    assert_eq!(chunks.len(), 87);
    let mut asm = TransferAssemblyV3::new(&b).expect("the plan's Begin is valid");
    assert_eq!(asm.begin(), &b);
    // The last chunk (payload 29,740) must be refused at the full chunk size,
    // and an index at the chunk count is out of range.
    let last = chunks[86].clone();
    assert_eq!(last.bytes.len(), 29_740);
    let mut padded = last.clone();
    padded.bytes.resize(32_768, 0);
    assert_eq!(refused(&mut asm, &padded), Err(AssemblyError::WrongLength));
    assert_eq!(
        refused(&mut asm, &raw(Z, 1, 87, last.bytes.clone())),
        Err(AssemblyError::OutOfRange)
    );
    for (n, c) in chunks.iter().enumerate().rev() {
        let want = if n == 0 { COMPLETE } else { STORED };
        assert_eq!(asm.accept(c), want, "chunk {n}");
        assert_eq!(usize::from(asm.received_chunks()), 87 - n);
        assert_eq!(asm.is_complete(), n == 0);
    }
    assert_eq!(refused(&mut asm, &last), DUPLICATE);
    assert_eq!(asm.into_bytes(), Ok(encoded));
}

// E5 / E-4: TransferLimitsV3.

fn lim(f: impl FnOnce(&mut TransferLimitsV3)) -> TransferLimitsV3 {
    let mut l = TransferLimitsV3::v3();
    f(&mut l);
    l
}

#[test]
fn e5_limits_v3_values_and_bounds() {
    assert_eq!(
        TransferLimitsV3::v3(),
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
    assert_eq!(TransferLimitsV3::v3().validate(), Ok(()));
    assert_eq!(POLICY_SNAPSHOT_REASSEMBLY_MS_V3, 20_000);
    assert_eq!(POLICY_SNAPSHOT_STAGING_BYTES_PER_PEER_V3, 4_194_304);
    assert_eq!(POLICY_SNAPSHOT_STAGING_BYTES_HOST_V3, 33_554_432);
    let cases: Vec<(&str, TransferLimitsV3, Result<(), LimitsError>)> = vec![
        (
            "max_receivers 0",
            lim(|l| l.max_receivers = 0),
            inv("max_receivers"),
        ),
        ("max_receivers 1", lim(|l| l.max_receivers = 1), Ok(())),
        (
            "max_receivers 9",
            lim(|l| l.max_receivers = 9),
            loose("max_receivers"),
        ),
        ("deadline 0", lim(|l| l.deadline_ms = 0), inv("deadline_ms")),
        ("deadline 1", lim(|l| l.deadline_ms = 1), Ok(())),
        ("deadline 20000", lim(|l| l.deadline_ms = 20_000), Ok(())),
        (
            "deadline 20001",
            lim(|l| l.deadline_ms = 20_001),
            loose("deadline_ms"),
        ),
        (
            "per 0",
            lim(|l| l.staging_bytes_per_receiver = 0),
            inv("staging_bytes_per_receiver"),
        ),
        (
            "per 2847787",
            lim(|l| l.staging_bytes_per_receiver = 2_847_787),
            inv("staging_bytes_per_receiver"),
        ),
        (
            "per 2847788",
            lim(|l| l.staging_bytes_per_receiver = 2_847_788),
            Ok(()),
        ),
        (
            "per 4194305",
            lim(|l| l.staging_bytes_per_receiver = 4_194_305),
            loose("staging_bytes_per_receiver"),
        ),
        (
            "total 0",
            lim(|l| l.staging_bytes_total = 0),
            inv("staging_bytes_total"),
        ),
        (
            "total 4194303",
            lim(|l| l.staging_bytes_total = 4_194_303),
            inv("staging_bytes_total"),
        ),
        (
            "total 4194304",
            lim(|l| l.staging_bytes_total = 4_194_304),
            Ok(()),
        ),
        (
            "total 33554433",
            lim(|l| l.staging_bytes_total = 33_554_433),
            loose("staging_bytes_total"),
        ),
        // The total's floor follows the per-receiver value.
        (
            "per 2847788 total 2847787",
            lim(|l| {
                l.staging_bytes_per_receiver = 2_847_788;
                l.staging_bytes_total = 2_847_787;
            }),
            inv("staging_bytes_total"),
        ),
        (
            "per 2847788 total 2847788",
            lim(|l| {
                l.staging_bytes_per_receiver = 2_847_788;
                l.staging_bytes_total = 2_847_788;
            }),
            Ok(()),
        ),
        (
            "frames 0",
            lim(|l| l.delta_frames_per_receiver = 0),
            inv("delta_frames_per_receiver"),
        ),
        ("frames 1", lim(|l| l.delta_frames_per_receiver = 1), Ok(())),
        (
            "frames 128",
            lim(|l| l.delta_frames_per_receiver = 128),
            Ok(()),
        ),
        (
            "frames 129",
            lim(|l| l.delta_frames_per_receiver = 129),
            loose("delta_frames_per_receiver"),
        ),
        (
            "dbytes 0",
            lim(|l| l.delta_bytes_per_receiver = 0),
            inv("delta_bytes_per_receiver"),
        ),
        (
            "dbytes 65535",
            lim(|l| l.delta_bytes_per_receiver = 65_535),
            inv("delta_bytes_per_receiver"),
        ),
        (
            "dbytes 65536",
            lim(|l| l.delta_bytes_per_receiver = 65_536),
            Ok(()),
        ),
        (
            "dbytes 2097153",
            lim(|l| l.delta_bytes_per_receiver = 2_097_153),
            loose("delta_bytes_per_receiver"),
        ),
        (
            "dtotal 0",
            lim(|l| l.delta_bytes_total = 0),
            inv("delta_bytes_total"),
        ),
        (
            "dtotal 2097151",
            lim(|l| l.delta_bytes_total = 2_097_151),
            inv("delta_bytes_total"),
        ),
        (
            "dtotal 2097152",
            lim(|l| l.delta_bytes_total = 2_097_152),
            Ok(()),
        ),
        (
            "dtotal 16777217",
            lim(|l| l.delta_bytes_total = 16_777_217),
            loose("delta_bytes_total"),
        ),
        // The delta total's floor follows the per-receiver value.
        (
            "dbytes 65536 dtotal 65535",
            lim(|l| {
                l.delta_bytes_per_receiver = 65_536;
                l.delta_bytes_total = 65_535;
            }),
            inv("delta_bytes_total"),
        ),
        (
            "dbytes 65536 dtotal 65536",
            lim(|l| {
                l.delta_bytes_per_receiver = 65_536;
                l.delta_bytes_total = 65_536;
            }),
            Ok(()),
        ),
    ];
    for (label, limits, want) in cases {
        assert_eq!(limits.validate(), want, "{label}");
    }
}

#[test]
fn e5_limits_v3_precedence_and_display() {
    // Field order: the first failing field wins, whatever its kind.
    let a = lim(|l| {
        l.max_receivers = 0;
        l.deadline_ms = 0;
    });
    assert_eq!(a.validate(), inv("max_receivers"));
    let b = lim(|l| {
        l.deadline_ms = 20_001;
        l.staging_bytes_per_receiver = 0;
    });
    assert_eq!(b.validate(), loose("deadline_ms"));
    let c = lim(|l| {
        l.max_receivers = 9;
        l.deadline_ms = 0;
    });
    assert_eq!(c.validate(), loose("max_receivers"));
    let d = lim(|l| {
        l.staging_bytes_total = 4_194_303;
        l.delta_frames_per_receiver = 129;
    });
    assert_eq!(d.validate(), inv("staging_bytes_total"));
    // Display of the 20,001 case.
    let error = lim(|l| l.deadline_ms = 20_001).validate().unwrap_err();
    assert_eq!(
        error.to_string(),
        "LoosensPolicy at transfer/limits: deadline_ms"
    );
}

// E6 / E-5: v1 sizing is untouched.

#[test]
fn e6_v1_sizing_unchanged() {
    let v1 = TransferLimits::v1();
    assert_eq!(
        v1,
        TransferLimits {
            max_receivers: 8,
            deadline_ms: 5_000,
            staging_bytes_per_receiver: 1_048_576,
            staging_bytes_total: 8 * 1_048_576,
            delta_frames_per_receiver: 128,
            delta_bytes_per_receiver: 2 * 1_048_576,
            delta_bytes_total: 16 * 1_048_576,
        }
    );
    assert_eq!(v1.validate(), Ok(()));
    assert_eq!(MAX_SNAPSHOT_BYTES, 1_048_576);
    let wide = TransferLimits {
        staging_bytes_per_receiver: 4_194_304,
        ..TransferLimits::v1()
    };
    assert_eq!(wide.validate(), loose("staging_bytes_per_receiver"));
    let slow = TransferLimits {
        deadline_ms: 20_000,
        ..TransferLimits::v1()
    };
    assert_eq!(slow.validate(), loose("deadline_ms"));
    // The v1 assembly refuses G, and a 33-chunk Begin; v3 takes both.
    assert_eq!(TransferAssembly::new(&g()).err(), Some(LimitExceeded));
    let wider = begin(1_081_344, 8, 33);
    assert_eq!(TransferAssembly::new(&wider).err(), Some(LimitExceeded));
    assert!(TransferAssemblyV3::new(&wider).is_ok());
    // Controls: the v1 envelope's own limit is still accepted by v1.
    assert!(TransferAssembly::new(&begin(1_048_576, 8, 32)).is_ok());
}

// E-1 / E-2 (1): the stored Begin is a copy of every field of the given one.

#[test]
fn e1_stored_begin_keeps_its_own_fields() {
    let epoch: [u8; 16] = core::array::from_fn(|i| i as u8 + 1);
    let b = BeginTransfer {
        epoch,
        transfer: tid(7),
        base_tick: 12_345,
        cursor: 678,
        total_bytes: 6,
        entity_count: 3,
        segments_per_chunk: 1,
        chunk_count: 1,
    };
    assert_eq!(t3::check_begin(&b), Ok(()));
    let mut asm = TransferAssemblyV3::new(&b).expect("a valid v3 Begin");
    assert_eq!(asm.begin(), &b);
    let payload = vec![9, 8, 7, 6, 5, 4];
    // The all-zero epoch is not this assembly's epoch; nor is another id.
    let zero_epoch = raw(Z, 7, 0, payload.clone());
    assert_eq!(
        refused(&mut asm, &zero_epoch),
        Err(AssemblyError::WrongTransfer)
    );
    let other_id = raw(epoch, 1, 0, payload.clone());
    assert_eq!(
        refused(&mut asm, &other_id),
        Err(AssemblyError::WrongTransfer)
    );
    assert_eq!(asm.received_chunks(), 0);
    assert_eq!(asm.accept(&raw(epoch, 7, 0, payload.clone())), COMPLETE);
    assert_eq!(asm.begin(), &b);
    assert_eq!(asm.into_bytes(), Ok(payload));
}
