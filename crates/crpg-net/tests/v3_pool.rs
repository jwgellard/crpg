//! T140r1 implementer suite: the protocol v3 receiver gate (`pool_v3`),
//! derived from `tasks/T140r.md` Behaviour P-1..P-16 and the tests P1..P11,
//! through the public API only.
//!
//! Expectations are hand-written whole values: whole results, full status
//! structs and full `PublicationV3` bodies. Every refusal is checked to leave
//! the pool equal to its pre-call clone where P-14 says it must. The D5 body
//! builder is copied from `tests/v3_reassembly.rs`, never imported. The P8
//! mirror runs the same script on T024's v1 pool, whose types are used only
//! as that oracle.

use std::sync::OnceLock;

use crpg_core::Ulid;
use crpg_net::codec::CodecError;
use crpg_net::codec_transfer;
use crpg_net::codec_v3::encode_delta;
use crpg_net::pool_v3::{
    PublicationV3, ReassemblyPoolV3, ReceiverIdV3, ReceiverStatusV3, ReplicaUpdateV3,
};
use crpg_net::protocol::SEQ_LAST;
use crpg_net::protocol_transfer::{
    BeginTransfer, CancelReason, ResyncReason, SnapshotBody, TransferChunk, TransferDown,
    TransferId,
};
use crpg_net::protocol_v3::{
    AutoPauseToggles, AutoPauseTrigger, ClockFact, DeltaFrame, DeltaOp, EffectFact, EncounterFacts,
    EncounterMode, FactSet, HostFacts, NetId, OrderFact, PauseReason, PauseRights, PoolFact,
    StatFact, StatValue, TimelineEntry,
};
use crpg_net::reassembly::{
    DeltaOutcome, DeltaWire, FailReason, GateMode, GatedDelta, InFlightStatus, PublishedBase,
    ReassemblyPool, Rebound, ReceiverId, ReceiverStatus, ReplicaUpdate, TransferError,
    TransferLimits, TransferOutcome,
};
use crpg_net::reassembly_v3::{LimitsError, TransferLimitsV3};
use crpg_net::transfer_v3::{
    encode_down, plan_transfer, HealthFact, SnapshotBodyV3, SnapshotEntityV3, TurnFact,
};

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

const Z16: [u8; 16] = [0; 16];
const CHUNK: usize = 32_768;

fn tid(n: u64) -> TransferId {
    TransferId::new(n).expect("fixture transfer id is nonzero")
}

fn id(n: u64) -> NetId {
    NetId::new(n).expect("fixture net id is nonzero")
}

fn u(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

/// `L3`.
fn l3() -> TransferLimitsV3 {
    TransferLimitsV3::v3()
}

/// The contract's `d(n)`: an empty v3 delta with tick and seq `n`.
fn d(n: u64) -> Vec<u8> {
    encode_delta(&DeltaFrame {
        lane: 0,
        server_tick: n,
        event_seq: n,
        ops: vec![],
    })
    .expect("fixture delta encodes")
}

/// The decoded form of the contract's `d(n)`.
fn dframe(n: u64) -> DeltaFrame {
    DeltaFrame {
        lane: 0,
        server_tick: n,
        event_seq: n,
        ops: vec![],
    }
}

/// The contract's `E0`: the empty v3 body, 6 bytes.
fn e0() -> SnapshotBodyV3 {
    SnapshotBodyV3 {
        entities: vec![],
        turn: None,
        encounter: EncounterFacts::default(),
        timeline: None,
    }
}

/// The contract's `plan(t, base, cursor, segs, body)`.
fn plan(t: u64, base: u64, cursor: u64, segs: u8, body: &SnapshotBodyV3) -> Vec<Vec<u8>> {
    plan_transfer(Z16, tid(t), base, cursor, segs, body).expect("fixture transfer plans")
}

/// One `0x82` Begin, encoded.
fn begin_frame(begin: BeginTransfer) -> Vec<u8> {
    encode_down(&TransferDown::Begin(begin)).expect("fixture Begin encodes")
}

/// The contract's `G(t)`: T140e's 4 MiB Begin, encoded.
fn g(t: u64) -> Vec<u8> {
    begin_frame(BeginTransfer {
        epoch: Z16,
        transfer: tid(t),
        base_tick: 0,
        cursor: 0,
        total_bytes: 4_194_304,
        entity_count: 0,
        segments_per_chunk: 8,
        chunk_count: 128,
    })
}

/// The contract's `g(t, i)`: a Chunk of 32,768 bytes of `i`.
fn g_chunk(t: u64, i: u8) -> Vec<u8> {
    encode_down(&TransferDown::Chunk(TransferChunk {
        epoch: Z16,
        transfer: tid(t),
        index: i,
        bytes: vec![i; CHUNK],
    }))
    .expect("fixture chunk encodes")
}

/// The contract's `B(t, total)`: a Begin with 8 segments and the canonical
/// chunk count.
fn b_frame(t: u64, total: u32) -> Vec<u8> {
    let chunks = (total as usize).div_ceil(CHUNK) as u8;
    begin_frame(BeginTransfer {
        epoch: Z16,
        transfer: tid(t),
        base_tick: 0,
        cursor: 0,
        total_bytes: total,
        entity_count: 0,
        segments_per_chunk: 8,
        chunk_count: chunks,
    })
}

fn cancel(t: u64) -> Vec<u8> {
    encode_down(&TransferDown::Cancel {
        epoch: Z16,
        transfer: tid(t),
        reason: CancelReason::Superseded,
    })
    .expect("fixture Cancel encodes")
}

fn pool_with(limits: TransferLimitsV3) -> ReassemblyPoolV3 {
    ReassemblyPoolV3::new(limits).expect("fixture limits validate")
}

fn live(pool: &mut ReassemblyPoolV3) -> ReceiverIdV3 {
    pool.add_receiver(Z16, GateMode::Live)
        .expect("fixture receiver admitted")
}

fn awaiting(pool: &mut ReassemblyPoolV3) -> ReceiverIdV3 {
    pool.add_receiver(Z16, GateMode::AwaitSnapshot)
        .expect("fixture receiver admitted")
}

/// Asserts a non-`Failed` refusal and that the pool is unchanged (P-14).
fn refused_transfer(
    pool: &mut ReassemblyPoolV3,
    receiver: ReceiverIdV3,
    bytes: &[u8],
    now_ms: u64,
    expected: TransferError,
) {
    let before = pool.clone();
    assert_eq!(pool.offer_transfer(receiver, bytes, now_ms), Err(expected));
    assert_eq!(*pool, before, "a refusal changed the pool");
}

/// Asserts a non-`Failed` delta refusal and that the pool is unchanged (P-14).
fn refused_delta(
    pool: &mut ReassemblyPoolV3,
    receiver: ReceiverIdV3,
    bytes: &[u8],
    now_ms: u64,
    expected: TransferError,
) {
    let before = pool.clone();
    assert_eq!(pool.offer_delta(receiver, bytes, now_ms), Err(expected));
    assert_eq!(*pool, before, "a refusal changed the pool");
}

// ---------------------------------------------------------------------------
// P1: construction, receivers and queries.
// ---------------------------------------------------------------------------

#[test]
fn p1_pool_v3_surface_and_receivers() {
    let limits = l3();
    let mut pool = ReassemblyPoolV3::new(limits.clone()).expect("L3 validates");
    assert_eq!(pool.limits(), limits);
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(pool.held_delta_bytes(), 0);

    assert_eq!(
        ReassemblyPoolV3::new(TransferLimitsV3 {
            max_receivers: 0,
            ..limits.clone()
        })
        .map(|_| ()),
        Err(LimitsError::InvalidLimits {
            field: "max_receivers"
        })
    );
    assert_eq!(
        ReassemblyPoolV3::new(TransferLimitsV3 {
            deadline_ms: 20_001,
            ..limits.clone()
        })
        .map(|_| ()),
        Err(LimitsError::LoosensPolicy {
            field: "deadline_ms"
        })
    );

    let ids: Vec<ReceiverIdV3> = (0..8)
        .map(|_| pool.add_receiver(Z16, GateMode::Live).expect("admitted"))
        .collect();
    assert_eq!(
        ids.iter().map(|r| r.get()).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6, 7, 8]
    );

    let before = pool.clone();
    assert_eq!(
        pool.add_receiver(Z16, GateMode::Live),
        Err(TransferError::ReceiversFull)
    );
    assert_eq!(pool, before);

    // A fresh receiver's status is P-3's literal, with `Live`.
    assert_eq!(
        pool.status(ids[0]).expect("receiver exists"),
        ReceiverStatusV3 {
            epoch: Z16,
            mode: GateMode::Live,
            in_flight: None,
            retired_through: 0,
            published: None,
            next_event_seq: 1,
            held_frames: 0,
            held_bytes: 0,
            queued_updates: 0,
            resync: None,
        }
    );

    // Once with `AwaitSnapshot` too, on a second pool.
    let mut other = pool_with(l3());
    let first = other
        .add_receiver(Z16, GateMode::AwaitSnapshot)
        .expect("ok");
    assert_eq!(
        other.status(first).expect("receiver exists"),
        ReceiverStatusV3 {
            epoch: Z16,
            mode: GateMode::AwaitSnapshot,
            in_flight: None,
            retired_through: 0,
            published: None,
            next_event_seq: 1,
            held_frames: 0,
            held_bytes: 0,
            queued_updates: 0,
            resync: None,
        }
    );

    // Removing frees the slot; the id is not reused.
    pool.remove_receiver(ids[2]);
    assert_eq!(pool.status(ids[2]), Err(TransferError::UnknownReceiver));
    assert_eq!(
        pool.add_receiver(Z16, GateMode::Live)
            .expect("admitted")
            .get(),
        9
    );
    let before = pool.clone();
    pool.remove_receiver(ids[2]);
    assert_eq!(pool, before);
}

// ---------------------------------------------------------------------------
// P2: the largest body through the pool.
// ---------------------------------------------------------------------------

fn largest_body() -> SnapshotBodyV3 {
    let base = 1u64 << 63;
    let entities = (0..1024u64)
        .map(|i| SnapshotEntityV3 {
            entity: id(base + i),
            health: Some(HealthFact {
                health: u32::MAX,
                max_health: u32::MAX,
                dead: true,
            }),
            facts: FactSet {
                pools: Some(
                    (1..=16)
                        .map(|n| PoolFact {
                            pool: u(n),
                            current: u32::MAX,
                            max: u32::MAX,
                        })
                        .collect(),
                ),
                effects: Some(
                    (1..=16)
                        .map(|n| EffectFact {
                            effect: u(n),
                            expires_round: u64::MAX,
                        })
                        .collect(),
                ),
                order: Some(Some(OrderFact {
                    ability: u(1),
                    target: id(base),
                    repeat: true,
                })),
                stats: Some(
                    (0..32)
                        .map(|n| StatFact {
                            name: format!("{n:032}"),
                            value: StatValue::Int(i32::MIN),
                        })
                        .collect(),
                ),
                tags: Some((0..8).map(|n| format!("{n:032}")).collect()),
            },
        })
        .collect();
    SnapshotBodyV3 {
        entities,
        turn: Some(TurnFact {
            active: Some(id(base)),
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
                    entity: id(base + i),
                })
                .collect(),
        ),
    }
}

/// The largest body and its encoding, built once.
fn largest() -> &'static (SnapshotBodyV3, Vec<u8>) {
    static CELL: OnceLock<(SnapshotBodyV3, Vec<u8>)> = OnceLock::new();
    CELL.get_or_init(|| {
        let body = largest_body();
        let bytes = crpg_net::transfer_v3::encode_body(&body).expect("the largest body encodes");
        (body, bytes)
    })
}

#[test]
fn p2_largest_body_publishes_through_the_pool() {
    let (body, encoded) = largest();
    assert_eq!(encoded.len(), 2_847_788);
    let frames = plan(1, 0, 0, 8, body);
    assert_eq!(frames.len(), 88);

    let mut pool = pool_with(l3());
    let a = awaiting(&mut pool);
    assert_eq!(
        pool.offer_transfer(a, &frames[0], 1_000),
        Ok(TransferOutcome::Begun)
    );
    let mut received = 0u8;
    for index in (1..=86u8).rev() {
        received += 1;
        assert_eq!(
            pool.offer_transfer(a, &frames[1 + index as usize], 1_000),
            Ok(TransferOutcome::Stored {
                received,
                chunk_count: 87,
            }),
            "chunk {index}"
        );
    }
    assert_eq!(received, 86);

    let status = pool.status(a).expect("receiver exists");
    assert_eq!(
        status.in_flight,
        Some(InFlightStatus {
            transfer: tid(1),
            cursor: 0,
            base_tick: 0,
            received: 86,
            chunk_count: 87,
            deadline_ms: 21_000,
            reserved_bytes: 2_847_788,
        })
    );
    assert_eq!(pool.staging_bytes(), 2_847_788);
    assert_eq!(status.queued_updates, 0);
    assert_eq!(status.held_frames, 0);
    assert_eq!(status.held_bytes, 0);
    assert_eq!(status.mode, GateMode::AwaitSnapshot);
    assert_eq!(status.resync, None);

    assert_eq!(
        pool.offer_transfer(a, &frames[1], 1_000),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    assert_eq!(
        pool.pop_update(a),
        Some(ReplicaUpdateV3::Snapshot(PublicationV3 {
            transfer: tid(1),
            base_tick: 0,
            cursor: 0,
            body: body.clone(),
        }))
    );
    assert_eq!(pool.pop_update(a), None);

    let status = pool.status(a).expect("receiver exists");
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(status.retired_through, 1);
    assert_eq!(
        status.published,
        Some(PublishedBase {
            transfer: tid(1),
            base_tick: 0,
            cursor: 0,
        })
    );
    assert_eq!(status.next_event_seq, 1);
    assert_eq!(status.mode, GateMode::Live);
    assert_eq!(status.resync, None);
    assert_eq!(status.in_flight, None);
    assert_eq!(status.queued_updates, 0);
}

// ---------------------------------------------------------------------------
// P3: staging bounds.
// ---------------------------------------------------------------------------

#[test]
fn p3_full_envelope_staging_and_aggregate() {
    // Eight maximal transfers fill the pool's 32 MiB.
    let mut pool = pool_with(l3());
    for _ in 0..8 {
        let r = live(&mut pool);
        assert_eq!(
            pool.offer_transfer(r, &g(1), 1_000),
            Ok(TransferOutcome::Begun)
        );
    }
    assert_eq!(pool.staging_bytes(), 33_554_432);

    // Per-receiver staging.
    let mut per = pool_with(TransferLimitsV3 {
        staging_bytes_per_receiver: 2_847_788,
        ..l3()
    });
    let a = live(&mut per);
    assert_eq!(
        per.offer_transfer(a, &b_frame(1, 2_847_788), 1_000),
        Ok(TransferOutcome::Begun)
    );
    let b = live(&mut per);
    let before = per.clone();
    assert_eq!(
        per.offer_transfer(b, &b_frame(1, 2_847_789), 1_000),
        Err(TransferError::StagingFull)
    );
    assert_eq!(per, before);

    // Pool staging.
    let mut total = pool_with(TransferLimitsV3 {
        staging_bytes_total: 7_042_092,
        ..l3()
    });
    let a = live(&mut total);
    assert_eq!(
        total.offer_transfer(a, &g(1), 1_000),
        Ok(TransferOutcome::Begun)
    );
    let b = live(&mut total);
    let before = total.clone();
    assert_eq!(
        total.offer_transfer(b, &b_frame(1, 2_847_789), 1_000),
        Err(TransferError::AggregateFull)
    );
    assert_eq!(total, before);
    assert_eq!(
        total.offer_transfer(b, &b_frame(1, 2_847_788), 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(total.staging_bytes(), 7_042_092);
    assert_eq!(
        total.offer_transfer(a, &cancel(1), 1_000),
        Ok(TransferOutcome::Cancelled { discarded: true })
    );
    assert_eq!(total.staging_bytes(), 2_847_788);
    total.remove_receiver(b);
    assert_eq!(total.staging_bytes(), 0);
}

// ---------------------------------------------------------------------------
// P4: the deadline boundary.
// ---------------------------------------------------------------------------

#[test]
fn p4_deadline_boundary_19999_20000() {
    let (body, _) = largest();
    let frames = plan(1, 0, 0, 8, body);

    // 19,999 ms after the Begin is still live.
    let mut early = pool_with(l3());
    let a = awaiting(&mut early);
    assert_eq!(
        early.offer_transfer(a, &frames[0], 1_000),
        Ok(TransferOutcome::Begun)
    );
    for index in (1..=86u8).rev() {
        assert_eq!(
            early.offer_transfer(a, &frames[1 + index as usize], 1_000),
            Ok(TransferOutcome::Stored {
                received: 87 - index,
                chunk_count: 87,
            })
        );
    }
    assert_eq!(
        early.offer_transfer(a, &frames[1], 20_999),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );

    // 20,000 ms is expired, and the completing chunk is refused as such.
    let mut late = pool_with(l3());
    let a = awaiting(&mut late);
    assert_eq!(
        late.offer_transfer(a, &frames[0], 1_000),
        Ok(TransferOutcome::Begun)
    );
    for index in (1..=86u8).rev() {
        assert_eq!(
            late.offer_transfer(a, &frames[1 + index as usize], 1_000),
            Ok(TransferOutcome::Stored {
                received: 87 - index,
                chunk_count: 87,
            })
        );
    }
    assert_eq!(
        late.offer_transfer(a, &frames[1], 21_000),
        Err(TransferError::Failed(FailReason::Expired))
    );
    let status = late.status(a).expect("receiver exists");
    assert_eq!(status.in_flight, None);
    assert_eq!(late.staging_bytes(), 0);
    assert_eq!(status.retired_through, 1);
    assert_eq!(
        status.resync,
        Some(crpg_net::protocol_transfer::ResyncReason::Expired)
    );
    assert_eq!(status.mode, GateMode::AwaitSnapshot);
    assert_eq!(status.queued_updates, 0);

    assert_eq!(
        late.offer_transfer(a, &frames[1], 21_000),
        Err(TransferError::StaleTransfer)
    );
    // A smaller `now_ms` is clamped up, not backwards.
    let before = late.clone();
    assert_eq!(
        late.offer_transfer(a, &frames[2], 10),
        Err(TransferError::StaleTransfer)
    );
    assert_eq!(late, before);

    // Poll at the two boundaries.
    let mut polling = pool_with(l3());
    let a = awaiting(&mut polling);
    assert_eq!(
        polling.offer_transfer(a, &frames[0], 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(polling.poll(20_999), Vec::new());
    assert_eq!(polling.poll(21_000), vec![a]);

    // The deadline saturates.
    let mut saturating = pool_with(l3());
    let a = live(&mut saturating);
    let empty = plan(1, 0, 0, 1, &e0());
    assert_eq!(
        saturating.offer_transfer(a, &empty[0], u64::MAX - 10),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        saturating
            .status(a)
            .expect("receiver exists")
            .in_flight
            .expect("in flight")
            .deadline_ms,
        u64::MAX
    );
}

// ---------------------------------------------------------------------------
// P5: invalid bodies never publish.
// ---------------------------------------------------------------------------

#[test]
fn p5_invalid_bodies_never_publish() {
    let mut pool = pool_with(l3());
    let a = awaiting(&mut pool);

    // Control: the empty body publishes.
    let empty = plan(1, 0, 0, 1, &e0());
    assert_eq!(
        pool.offer_transfer(a, &empty[0], 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool.offer_transfer(a, &empty[1], 1_000),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );

    // Transfer 2: 4 MiB of `g` chunks, which decode as `E0` plus trailing
    // bytes.
    assert_eq!(
        pool.offer_transfer(a, &g(2), 1_000),
        Ok(TransferOutcome::Begun)
    );
    for index in 0..=126u8 {
        assert_eq!(
            pool.offer_transfer(a, &g_chunk(2, index), 1_000),
            Ok(TransferOutcome::Stored {
                received: index + 1,
                chunk_count: 128,
            }),
            "chunk {index}"
        );
    }
    assert_eq!(
        pool.offer_transfer(a, &g_chunk(2, 127), 1_000),
        Err(TransferError::Failed(FailReason::InvalidBody(
            CodecError::Malformed
        )))
    );
    let status = pool.status(a).expect("receiver exists");
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(status.retired_through, 2);
    assert_eq!(
        status.resync,
        Some(crpg_net::protocol_transfer::ResyncReason::Invalid)
    );
    assert_eq!(status.mode, GateMode::AwaitSnapshot);
    assert_eq!(status.queued_updates, 1);

    // Transfer 3: a valid body whose entity count disagrees with the Begin.
    let one_entity: Vec<u8> = vec![
        0x01, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ];
    assert_eq!(one_entity.len(), 16);
    let mut pool3 = pool.clone();
    let begin3 = begin_frame(BeginTransfer {
        epoch: Z16,
        transfer: tid(3),
        base_tick: 0,
        cursor: 0,
        total_bytes: 16,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    });
    let chunk3 = encode_down(&TransferDown::Chunk(TransferChunk {
        epoch: Z16,
        transfer: tid(3),
        index: 0,
        bytes: one_entity.clone(),
    }))
    .expect("chunk encodes");
    assert_eq!(
        pool3.offer_transfer(a, &begin3, 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool3.offer_transfer(a, &chunk3, 1_000),
        Err(TransferError::Failed(FailReason::InvalidBody(
            CodecError::Malformed
        )))
    );
    let status3 = pool3.status(a).expect("receiver exists");
    assert_eq!(pool3.staging_bytes(), 0);
    assert_eq!(status3.retired_through, 3);
    assert_eq!(
        status3.resync,
        Some(crpg_net::protocol_transfer::ResyncReason::Invalid)
    );

    // Transfer 4: one trailing byte after a valid `E0`.
    let begin4 = begin_frame(BeginTransfer {
        epoch: Z16,
        transfer: tid(4),
        base_tick: 0,
        cursor: 0,
        total_bytes: 7,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    });
    let chunk4 = encode_down(&TransferDown::Chunk(TransferChunk {
        epoch: Z16,
        transfer: tid(4),
        index: 0,
        bytes: vec![0; 7],
    }))
    .expect("chunk encodes");
    assert_eq!(
        pool3.offer_transfer(a, &begin4, 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool3.offer_transfer(a, &chunk4, 1_000),
        Err(TransferError::Failed(FailReason::InvalidBody(
            CodecError::Malformed
        )))
    );
    assert_eq!(pool3.staging_bytes(), 0);
    assert_eq!(pool3.status(a).expect("receiver exists").retired_through, 4);

    // Transfer 1's snapshot survived every failure.
    assert_eq!(
        pool3.pop_update(a),
        Some(ReplicaUpdateV3::Snapshot(PublicationV3 {
            transfer: tid(1),
            base_tick: 0,
            cursor: 0,
            body: e0(),
        }))
    );
    assert_eq!(pool3.pop_update(a), None);
}

// ---------------------------------------------------------------------------
// P6: version and wire refusals.
// ---------------------------------------------------------------------------

/// T140 D1's `0x82` Begin: `82 02 10 Z16 01 00 00 06 00 01 01`.
fn d1_begin(version: u8) -> Vec<u8> {
    let mut out = vec![version, 0x02, 0x10];
    out.extend_from_slice(&Z16);
    out.extend_from_slice(&[0x01, 0x00, 0x00, 0x06, 0x00, 0x01, 0x01]);
    out
}

#[test]
fn p6_version_and_wire_refusals() {
    let mut pool = pool_with(l3());
    let a = live(&mut pool);
    let removed = live(&mut pool);
    pool.remove_receiver(removed);
    // The pool's clock is already at 1,000, so every refusal below has no
    // sweep work and must leave the pool equal to its clone (P-14).
    assert_eq!(pool.poll(1_000), Vec::new());

    // A 0x81 transfer frame, a v2 and a v1 delta, and the two crossed wires.
    let mut wrong_version = d1_begin(0x82);
    wrong_version[0] = 0x81;
    refused_transfer(
        &mut pool,
        a,
        &wrong_version,
        1_000,
        TransferError::Codec(CodecError::UnsupportedVersion),
    );
    refused_delta(
        &mut pool,
        a,
        &[0x02, 0x00, 0x01, 0x01, 0x00],
        1_000,
        TransferError::Codec(CodecError::UnsupportedVersion),
    );
    refused_delta(
        &mut pool,
        a,
        &[0x01, 0x00, 0x01, 0x01, 0x00],
        1_000,
        TransferError::Codec(CodecError::UnsupportedVersion),
    );
    refused_transfer(
        &mut pool,
        a,
        &d(1),
        1_000,
        TransferError::Codec(CodecError::UnsupportedVersion),
    );
    refused_delta(
        &mut pool,
        a,
        &d1_begin(0x82),
        1_000,
        TransferError::Codec(CodecError::UnsupportedVersion),
    );

    // Sequence refusals.
    refused_delta(
        &mut pool,
        a,
        &[0x03, 0x00, 0x00, 0x00, 0x00],
        1_000,
        TransferError::DeltaSequence,
    );
    let too_high = encode_delta(&DeltaFrame {
        lane: 0,
        server_tick: 1,
        event_seq: u64::MAX,
        ops: vec![],
    })
    .expect("the fixture encodes");
    refused_delta(&mut pool, a, &too_high, 1_000, TransferError::DeltaSequence);

    // Unknown before decode.
    refused_transfer(
        &mut pool,
        removed,
        &[0xFF],
        1_000,
        TransferError::UnknownReceiver,
    );
    refused_delta(
        &mut pool,
        removed,
        &[0xFF],
        1_000,
        TransferError::UnknownReceiver,
    );

    // Positive control: the last legal sequence is buffered, not refused.
    let last = encode_delta(&DeltaFrame {
        lane: 0,
        server_tick: 1,
        event_seq: SEQ_LAST,
        ops: vec![],
    })
    .expect("the fixture encodes");
    assert_eq!(
        pool.offer_delta(a, &last, 1_000),
        Ok(DeltaOutcome::Buffered)
    );

    // The v1 pool still refuses both v3 wires.
    let mut v1 = ReassemblyPool::new(TransferLimits::v1()).expect("v1 limits");
    let r = v1
        .add_receiver(Z16, DeltaWire::V2, GateMode::Live)
        .expect("receiver admitted");
    assert_eq!(
        v1.offer_transfer(r, &d1_begin(0x82), 1_000),
        Err(TransferError::Codec(CodecError::UnsupportedVersion))
    );
    assert_eq!(
        v1.offer_delta(r, &d(1), 1_000),
        Err(TransferError::Codec(CodecError::UnsupportedVersion))
    );
}

// ---------------------------------------------------------------------------
// P7: every v3 op passes the gate unchanged.
// ---------------------------------------------------------------------------

fn all_ops_frame() -> DeltaFrame {
    DeltaFrame {
        lane: 0,
        server_tick: 100,
        event_seq: 1,
        ops: vec![
            DeltaOp::Encounter(EncounterFacts {
                mode: Some(EncounterMode::RealTime),
                pending_mode: Some(None),
                clock: Some(Some(ClockFact {
                    round_ticks: 60,
                    elapsed: 5,
                })),
            }),
            DeltaOp::Timeline {
                entries: Some(vec![TimelineEntry {
                    key: -1,
                    entity: id(1),
                }]),
            },
            DeltaOp::EntityFacts {
                entity: id(1),
                facts: FactSet {
                    pools: Some(vec![]),
                    effects: None,
                    order: Some(None),
                    stats: Some(vec![StatFact {
                        name: "might".to_owned(),
                        value: StatValue::Int(3),
                    }]),
                    tags: Some(vec!["elite".to_owned()]),
                },
            },
            DeltaOp::RoundStarted { round: 2 },
            DeltaOp::ModeChanged {
                mode: EncounterMode::TurnBased,
                round: 2,
            },
            DeltaOp::OrderEnded { actor: id(1) },
            DeltaOp::Joined { entity: id(1) },
            DeltaOp::Left { entity: id(1) },
            DeltaOp::Clock {
                paused: Some(Some(PauseReason::Auto(AutoPauseTrigger::PartyMemberDied))),
            },
            DeltaOp::HostFacts(HostFacts {
                rights: PauseRights {
                    pause: true,
                    unpause: false,
                },
                auto_pause: Some(AutoPauseToggles::from_bits(0x1F).expect("five bits")),
                turn_timer_ms: Some(Some(30_000)),
            }),
        ],
    }
}

#[test]
fn p7_v3_ops_pass_the_gate_unchanged() {
    let frame = all_ops_frame();
    let bytes = encode_delta(&frame).expect("the fixture encodes");

    let mut pool = pool_with(l3());
    let a = live(&mut pool);
    assert_eq!(
        pool.offer_delta(a, &bytes, 1_000),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    assert_eq!(pool.held_delta_bytes(), bytes.len());
    assert_eq!(
        pool.pop_update(a),
        Some(ReplicaUpdateV3::Delta(frame.clone()))
    );
    assert_eq!(pool.held_delta_bytes(), 0);
    assert_eq!(pool.pop_update(a), None);

    let mut waiting = pool_with(l3());
    let a = awaiting(&mut waiting);
    assert_eq!(
        waiting.offer_delta(a, &bytes, 1_000),
        Ok(DeltaOutcome::Buffered)
    );
    let status = waiting.status(a).expect("receiver exists");
    assert_eq!(status.held_frames, 1);
    assert_eq!(status.held_bytes, bytes.len());
    assert_eq!(waiting.held_delta_bytes(), bytes.len());
}

// ---------------------------------------------------------------------------
// P9: rebind discards the in-flight transfer and the held deltas.
// ---------------------------------------------------------------------------

/// The contract's `G(t)` with `cursor` in place of 0: a 4 MiB Begin.
fn g_at(t: u64, cursor: u64) -> Vec<u8> {
    begin_frame(BeginTransfer {
        epoch: Z16,
        transfer: tid(t),
        base_tick: 0,
        cursor,
        total_bytes: 4_194_304,
        entity_count: 0,
        segments_per_chunk: 8,
        chunk_count: 128,
    })
}

#[test]
fn p9_rebind_discards_in_flight_and_held() {
    let mut pool = pool_with(l3());
    let a = live(&mut pool);

    // d(1) releases, so next_event_seq is 2 and the Begin needs cursor 1
    // (cursor 0 would be BehindReplica by P-5 step 4).
    assert_eq!(
        pool.offer_delta(a, &d(1), 1_000),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    assert_eq!(
        pool.offer_transfer(a, &g_at(1, 1), 1_000),
        Ok(TransferOutcome::Begun)
    );
    // Each is above the in-flight cursor 1, so each is held.
    assert_eq!(
        pool.offer_delta(a, &d(2), 1_000),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(
        pool.offer_delta(a, &d(3), 1_000),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(pool.staging_bytes(), 4_194_304);
    // Held plus queued: d(2) and d(3) held, d(1) still queued.
    assert_eq!(pool.held_delta_bytes(), 15);

    // Rebind discards G(1) and both held deltas, keeping the queued d(1).
    assert_eq!(
        pool.rebind_receiver(a, 1_000),
        Ok(Rebound {
            discarded: Some(tid(1)),
            dropped_deltas: 2,
        })
    );
    assert_eq!(
        pool.status(a).expect("receiver exists"),
        ReceiverStatusV3 {
            epoch: Z16,
            mode: GateMode::AwaitSnapshot,
            in_flight: None,
            retired_through: 1,
            published: None,
            next_event_seq: 2,
            held_frames: 1,
            held_bytes: 5,
            queued_updates: 1,
            resync: None,
        }
    );
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(pool.held_delta_bytes(), 5);
    assert_eq!(pool.pop_update(a), Some(ReplicaUpdateV3::Delta(dframe(1))));
    assert_eq!(pool.held_delta_bytes(), 0);
    assert_eq!(pool.pop_update(a), None);

    // A removed id is unknown, and the refusal changes nothing.
    pool.remove_receiver(a);
    let before = pool.clone();
    assert_eq!(
        pool.rebind_receiver(a, 1_000),
        Err(TransferError::UnknownReceiver)
    );
    assert_eq!(pool, before, "a refusal changed the pool");
}

// ---------------------------------------------------------------------------
// P10: every Begin, Chunk and Cancel arm.
// ---------------------------------------------------------------------------

fn ob(t: u64, base: u64, cursor: u64) -> Vec<u8> {
    begin_frame(BeginTransfer {
        epoch: Z16,
        transfer: tid(t),
        base_tick: base,
        cursor,
        total_bytes: 8_192,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 2,
    })
}

fn oc(t: u64, index: u8, len: usize) -> Vec<u8> {
    encode_down(&TransferDown::Chunk(TransferChunk {
        epoch: Z16,
        transfer: tid(t),
        index,
        bytes: vec![t as u8; len],
    }))
    .expect("chunk encodes")
}

#[test]
fn p10_begin_chunk_and_cancel_arms() {
    let mut pool = pool_with(l3());
    let a = live(&mut pool);
    let published = PublicationV3 {
        transfer: tid(1),
        base_tick: 20,
        cursor: 0,
        body: e0(),
    };

    // 1. A snapshot is published and stays queued.
    let first = plan(1, 20, 0, 1, &e0());
    assert_eq!(
        pool.offer_transfer(a, &first[0], 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool.offer_transfer(a, &first[1], 1_000),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );

    // 2. A second transfer begins.
    assert_eq!(
        pool.offer_transfer(a, &ob(2, 20, 0), 1_000),
        Ok(TransferOutcome::Begun)
    );

    // 3. Another transfer while one is in flight.
    let before = pool.clone();
    assert_eq!(
        pool.offer_transfer(a, &ob(3, 20, 0), 1_000),
        Err(TransferError::InFlight)
    );
    assert_eq!(pool, before);

    // 4. A chunk of an unknown, unretired transfer.
    let before = pool.clone();
    assert_eq!(
        pool.offer_transfer(a, &oc(5, 0, 4_096), 1_000),
        Err(TransferError::UnknownTransfer)
    );
    assert_eq!(pool, before);

    // 5. Wrong epoch on a chunk and on a cancel.
    let wrong_epoch_chunk = encode_down(&TransferDown::Chunk(TransferChunk {
        epoch: [1; 16],
        transfer: tid(2),
        index: 0,
        bytes: vec![0; 4_096],
    }))
    .expect("chunk encodes");
    refused_transfer(
        &mut pool,
        a,
        &wrong_epoch_chunk,
        1_000,
        TransferError::WrongEpoch,
    );
    let wrong_epoch_cancel = encode_down(&TransferDown::Cancel {
        epoch: [1; 16],
        transfer: tid(2),
        reason: CancelReason::Superseded,
    })
    .expect("cancel encodes");
    refused_transfer(
        &mut pool,
        a,
        &wrong_epoch_cancel,
        1_000,
        TransferError::WrongEpoch,
    );

    // 6. Cancelling an already-retired id keeps the in-flight transfer.
    assert_eq!(
        pool.offer_transfer(a, &cancel(1), 1_000),
        Ok(TransferOutcome::Cancelled { discarded: false })
    );
    let status = pool.status(a).expect("receiver exists");
    assert_eq!(status.in_flight.as_ref().map(|i| i.transfer), Some(tid(2)));
    assert_eq!(status.retired_through, 1);

    // 7. A conflicting Begin with the in-flight id.
    assert_eq!(
        pool.offer_transfer(a, &ob(2, 20, 1), 1_000),
        Err(TransferError::Failed(FailReason::Conflict))
    );
    let status = pool.status(a).expect("receiver exists");
    assert_eq!(status.in_flight, None);
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(status.retired_through, 2);
    assert_eq!(
        status.resync,
        Some(crpg_net::protocol_transfer::ResyncReason::Invalid)
    );
    assert_eq!(status.mode, GateMode::AwaitSnapshot);
    assert_eq!(status.queued_updates, 1);

    // 8. Behind the published base, then an equal base is accepted.
    let before = pool.clone();
    assert_eq!(
        pool.offer_transfer(a, &ob(3, 19, 0), 1_000),
        Err(TransferError::BehindReplica)
    );
    assert_eq!(pool, before);
    assert_eq!(
        pool.offer_transfer(a, &ob(3, 20, 0), 1_000),
        Ok(TransferOutcome::Begun)
    );

    // 9. A chunk index equal to the chunk count.
    assert_eq!(
        pool.offer_transfer(a, &oc(3, 2, 4_096), 1_000),
        Err(TransferError::Failed(FailReason::Conflict))
    );
    let status = pool.status(a).expect("receiver exists");
    assert_eq!(status.in_flight, None);
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(status.retired_through, 3);
    assert_eq!(
        status.resync,
        Some(crpg_net::protocol_transfer::ResyncReason::Invalid)
    );
    assert_eq!(status.mode, GateMode::AwaitSnapshot);
    assert_eq!(status.queued_updates, 1);

    // 10. A chunk one byte short.
    assert_eq!(
        pool.offer_transfer(a, &ob(4, 20, 0), 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool.offer_transfer(a, &oc(4, 0, 4_095), 1_000),
        Err(TransferError::Failed(FailReason::Conflict))
    );
    let status = pool.status(a).expect("receiver exists");
    assert_eq!(status.in_flight, None);
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(status.retired_through, 4);
    assert_eq!(
        status.resync,
        Some(crpg_net::protocol_transfer::ResyncReason::Invalid)
    );
    assert_eq!(status.mode, GateMode::AwaitSnapshot);
    assert_eq!(status.queued_updates, 1);

    // The first publication survived all of it.
    assert_eq!(
        pool.pop_update(a),
        Some(ReplicaUpdateV3::Snapshot(published))
    );
    assert_eq!(pool.pop_update(a), None);
}

// ---------------------------------------------------------------------------
// P11: queue clear, byte budgets and poll order.
// ---------------------------------------------------------------------------

fn timeline_frame(n: u64, k: u64) -> Vec<u8> {
    encode_delta(&DeltaFrame {
        lane: 0,
        server_tick: n,
        event_seq: n,
        ops: vec![DeltaOp::Timeline {
            entries: Some(
                (1..=k)
                    .map(|i| TimelineEntry {
                        key: 0,
                        entity: id(i),
                    })
                    .collect(),
            ),
        }],
    })
    .expect("fixture timeline frame encodes")
}

#[test]
fn p11_queue_clear_overflow_bytes_and_poll_order() {
    // (a) A new publication clears a queued snapshot and its delta.
    let mut pool = pool_with(l3());
    let a = live(&mut pool);
    let first = plan(1, 0, 0, 1, &e0());
    assert_eq!(
        pool.offer_transfer(a, &first[0], 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool.offer_transfer(a, &first[1], 1_000),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    assert_eq!(
        pool.offer_delta(a, &d(1), 1_000),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    let second = plan(2, 0, 1, 1, &e0());
    assert_eq!(
        pool.offer_transfer(a, &second[0], 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool.offer_transfer(a, &second[1], 1_000),
        Ok(TransferOutcome::Published {
            transfer: tid(2),
            released: 0,
        })
    );
    let status = pool.status(a).expect("receiver exists");
    assert_eq!(status.queued_updates, 1);
    assert_eq!(pool.held_delta_bytes(), 0);
    assert_eq!(
        pool.pop_update(a),
        Some(ReplicaUpdateV3::Snapshot(PublicationV3 {
            transfer: tid(2),
            base_tick: 0,
            cursor: 1,
            body: e0(),
        }))
    );
    assert_eq!(pool.pop_update(a), None);

    // (b) The per-receiver byte budget.
    let mut pool = pool_with(TransferLimitsV3 {
        delta_bytes_per_receiver: 65_536,
        ..l3()
    });
    let a = awaiting(&mut pool);
    for n in 1..=22u64 {
        let frame = timeline_frame(n, 1_024);
        assert_eq!(frame.len(), 2_954);
        assert_eq!(
            pool.offer_delta(a, &frame, 1_000),
            Ok(DeltaOutcome::Buffered),
            "frame {n}"
        );
    }
    let last = timeline_frame(23, 222);
    assert_eq!(last.len(), 548);
    assert_eq!(
        pool.offer_delta(a, &last, 1_000),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(pool.status(a).expect("receiver exists").held_bytes, 65_536);
    assert_eq!(
        pool.offer_delta(a, &d(24), 1_000),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    let status = pool.status(a).expect("receiver exists");
    assert_eq!(status.held_frames, 0);
    assert_eq!(status.held_bytes, 0);
    assert_eq!(
        status.resync,
        Some(crpg_net::protocol_transfer::ResyncReason::Overflow)
    );
    assert_eq!(pool.held_delta_bytes(), 0);

    // (c) The pool byte budget.
    let mut pool = pool_with(TransferLimitsV3 {
        delta_bytes_per_receiver: 65_536,
        delta_bytes_total: 65_536,
        ..l3()
    });
    let a = awaiting(&mut pool);
    let b = awaiting(&mut pool);
    for n in 1..=22u64 {
        assert_eq!(
            pool.offer_delta(a, &timeline_frame(n, 1_024), 1_000),
            Ok(DeltaOutcome::Buffered)
        );
    }
    assert_eq!(pool.held_delta_bytes(), 64_988);
    assert_eq!(
        pool.offer_delta(b, &timeline_frame(1, 222), 1_000),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(pool.held_delta_bytes(), 65_536);
    assert_eq!(
        pool.offer_delta(b, &d(2), 1_000),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    assert_eq!(pool.held_delta_bytes(), 64_988);
    assert_eq!(pool.status(a).expect("receiver exists").held_bytes, 64_988);

    // (d) Queued updates survive an overflow.
    let mut pool = pool_with(TransferLimitsV3 {
        delta_frames_per_receiver: 2,
        ..l3()
    });
    let a = live(&mut pool);
    assert_eq!(
        pool.offer_delta(a, &d(1), 1_000),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    assert_eq!(
        pool.offer_delta(a, &d(3), 1_000),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(
        pool.offer_delta(a, &d(4), 1_000),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    let status = pool.status(a).expect("receiver exists");
    assert_eq!(status.queued_updates, 1);
    assert_eq!(status.held_frames, 1);
    assert_eq!(status.held_bytes, 5);
    assert_eq!(status.mode, GateMode::AwaitSnapshot);
    assert_eq!(
        pool.pop_update(a),
        Some(ReplicaUpdateV3::Delta(DeltaFrame {
            lane: 0,
            server_tick: 1,
            event_seq: 1,
            ops: vec![],
        }))
    );

    // (e) Poll is ascending by receiver id.
    let mut pool = pool_with(l3());
    let a = live(&mut pool);
    let b = live(&mut pool);
    assert_eq!(
        pool.offer_transfer(b, &g(1), 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool.offer_transfer(a, &g(1), 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(pool.poll(21_000), vec![a, b]);
    assert_eq!(pool.staging_bytes(), 0);
}

// ---------------------------------------------------------------------------
// P8: the mirror against T024's v1 pool.
// ---------------------------------------------------------------------------

/// The v3 form of a v1/v2 frame: byte 0 replaced, nothing else.
fn upgraded(frame: &[u8]) -> Vec<u8> {
    let mut out = frame.to_vec();
    match out[0] {
        0x02 => out[0] = 0x03,
        0x81 => out[0] = 0x82,
        other => panic!("not a v1/v2 frame: {other:#04x}"),
    }
    out
}

/// The contract's `D(n)` (v2 form).
fn d_v2(n: u64) -> Vec<u8> {
    assert!(n <= 127);
    vec![0x02, 0x00, n as u8, n as u8, 0x00]
}

/// The contract's `O(t, c)` (v1 form).
fn o_begin(t: u64, cursor: u64) -> Vec<u8> {
    codec_transfer::encode_down(&TransferDown::Begin(BeginTransfer {
        epoch: Z16,
        transfer: tid(t),
        base_tick: 10,
        cursor,
        total_bytes: 8_192,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 2,
    }))
    .expect("O encodes")
}

/// `O(t)#i` and `O(t)#0'` (v1 form).
fn o_chunk(t: u64, index: u8, fill: u8) -> Vec<u8> {
    codec_transfer::encode_down(&TransferDown::Chunk(TransferChunk {
        epoch: Z16,
        transfer: tid(t),
        index,
        bytes: vec![fill; 4_096],
    }))
    .expect("chunk encodes")
}

/// The contract's `C(t)`.
fn c_frame(t: u64) -> Vec<u8> {
    codec_transfer::encode_down(&TransferDown::Cancel {
        epoch: Z16,
        transfer: tid(t),
        reason: CancelReason::Superseded,
    })
    .expect("cancel encodes")
}

/// The contract's `S(t, c)` v1 frames.
fn s_v1(t: u64, cursor: u64, epoch: [u8; 16]) -> Vec<Vec<u8>> {
    codec_transfer::plan_transfer(
        epoch,
        tid(t),
        10,
        cursor,
        1,
        &SnapshotBody {
            entities: vec![],
            turn: Some(TurnFact {
                active: None,
                round: 16_384,
            }),
        },
    )
    .expect("S plans")
}

/// The contract's `S(t, c)` v3 frames.
fn s_v3(t: u64, cursor: u64, epoch: [u8; 16]) -> Vec<Vec<u8>> {
    plan_transfer(epoch, tid(t), 10, cursor, 1, &e0()).expect("S plans")
}

struct Mirror {
    v1: ReassemblyPool,
    v3: ReassemblyPoolV3,
    live: Vec<(ReceiverId, ReceiverIdV3)>,
}

impl Mirror {
    fn new() -> Self {
        Self {
            v1: ReassemblyPool::new(TransferLimits {
                delta_frames_per_receiver: 6,
                ..TransferLimits::v1()
            })
            .expect("v1 limits"),
            v3: ReassemblyPoolV3::new(TransferLimitsV3 {
                deadline_ms: 5_000,
                delta_frames_per_receiver: 6,
                ..TransferLimitsV3::v3()
            })
            .expect("v3 limits"),
            live: Vec::new(),
        }
    }

    fn add(&mut self, mode: GateMode) -> (ReceiverId, ReceiverIdV3) {
        let r1 = self
            .v1
            .add_receiver(Z16, DeltaWire::V2, mode)
            .expect("v1 receiver admitted");
        let r3 = self
            .v3
            .add_receiver(Z16, mode)
            .expect("v3 receiver admitted");
        assert_eq!(r1.get(), r3.get(), "receiver ids diverge");
        self.live.push((r1, r3));
        (r1, r3)
    }

    fn remove(&mut self, r: (ReceiverId, ReceiverIdV3)) {
        self.v1.remove_receiver(r.0);
        self.v3.remove_receiver(r.1);
        self.live.retain(|(id, _)| *id != r.0);
    }

    fn transfer_pair(
        &mut self,
        r: (ReceiverId, ReceiverIdV3),
        v1_frame: &[u8],
        v3_frame: &[u8],
        now_ms: u64,
    ) -> Result<TransferOutcome, TransferError> {
        let got1 = self.v1.offer_transfer(r.0, v1_frame, now_ms);
        let got3 = self.v3.offer_transfer(r.1, v3_frame, now_ms);
        assert_eq!(got3, got1, "offer_transfer diverges");
        got3
    }

    fn transfer(
        &mut self,
        r: (ReceiverId, ReceiverIdV3),
        frame: &[u8],
        now_ms: u64,
    ) -> Result<TransferOutcome, TransferError> {
        self.transfer_pair(r, frame, &upgraded(frame), now_ms)
    }

    fn delta(
        &mut self,
        r: (ReceiverId, ReceiverIdV3),
        frame: &[u8],
        now_ms: u64,
    ) -> Result<DeltaOutcome, TransferError> {
        let got1 = self.v1.offer_delta(r.0, frame, now_ms);
        let got3 = self.v3.offer_delta(r.1, &upgraded(frame), now_ms);
        assert_eq!(got3, got1, "offer_delta diverges");
        got3
    }

    fn poll(&mut self, now_ms: u64) -> Vec<ReceiverIdV3> {
        let p1 = self.v1.poll(now_ms);
        let p3 = self.v3.poll(now_ms);
        assert_eq!(
            p1.iter().map(|r| r.get()).collect::<Vec<_>>(),
            p3.iter().map(|r| r.get()).collect::<Vec<_>>(),
            "poll diverges"
        );
        p3
    }

    fn pop(&mut self, r: (ReceiverId, ReceiverIdV3)) -> Option<ReplicaUpdateV3> {
        let got1 = self.v1.pop_update(r.0);
        let got3 = self.v3.pop_update(r.1);
        match (got1, got3) {
            (None, None) => None,
            (Some(ReplicaUpdate::Snapshot(p1)), Some(ReplicaUpdateV3::Snapshot(p3))) => {
                assert_eq!(p1.transfer, p3.transfer);
                assert_eq!(p1.base_tick, p3.base_tick);
                assert_eq!(p1.cursor, p3.cursor);
                assert_eq!(p3.body, e0(), "the v3 body is E0");
                Some(ReplicaUpdateV3::Snapshot(p3))
            }
            (Some(ReplicaUpdate::Delta(GatedDelta::V2(f))), Some(ReplicaUpdateV3::Delta(g))) => {
                assert_eq!(
                    g,
                    DeltaFrame {
                        lane: f.lane,
                        server_tick: f.server_tick,
                        event_seq: f.event_seq,
                        ops: vec![],
                    }
                );
                Some(ReplicaUpdateV3::Delta(g))
            }
            (got1, got3) => panic!("popped updates diverge: {got1:?} vs {got3:?}"),
        }
    }

    fn status(&self, r: (ReceiverId, ReceiverIdV3)) -> (ReceiverStatus, ReceiverStatusV3) {
        (
            self.v1.status(r.0).expect("v1 receiver exists"),
            self.v3.status(r.1).expect("v3 receiver exists"),
        )
    }

    /// After every step: ids agree, both totals agree, and every live
    /// receiver's v1 status with `wire` dropped equals the v3 status.
    fn check(&self) {
        assert_eq!(
            self.v1.staging_bytes(),
            self.v3.staging_bytes(),
            "staging_bytes diverges"
        );
        assert_eq!(
            self.v1.held_delta_bytes(),
            self.v3.held_delta_bytes(),
            "held_delta_bytes diverges"
        );
        for (r1, r3) in &self.live {
            assert_eq!(r1.get(), r3.get());
            let s1 = self.v1.status(*r1).expect("v1 receiver exists");
            let s3 = self.v3.status(*r3).expect("v3 receiver exists");
            assert_eq!(s1.epoch, s3.epoch, "epoch");
            assert_eq!(s1.mode, s3.mode, "mode");
            assert_eq!(s1.in_flight, s3.in_flight, "in_flight");
            assert_eq!(s1.retired_through, s3.retired_through, "retired_through");
            assert_eq!(s1.published, s3.published, "published");
            assert_eq!(s1.next_event_seq, s3.next_event_seq, "next_event_seq");
            assert_eq!(s1.held_frames, s3.held_frames, "held_frames");
            assert_eq!(s1.held_bytes, s3.held_bytes, "held_bytes");
            assert_eq!(s1.queued_updates, s3.queued_updates, "queued_updates");
            assert_eq!(s1.resync, s3.resync, "resync");
        }
    }
}

#[test]
fn p8_mirror_matches_the_v1_pool() {
    let mut m = Mirror::new();

    // 1: add A.
    let a = m.add(GateMode::Live);
    assert_eq!(a.1.get(), 1);
    m.check();

    // 2: two released deltas.
    assert_eq!(
        m.delta(a, &d_v2(1), 1_000),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    assert_eq!(
        m.delta(a, &d_v2(2), 1_000),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    m.check();

    // 3: a transfer begins behind the queue's next sequence.
    assert_eq!(
        m.transfer(a, &o_begin(1, 4), 1_000),
        Ok(TransferOutcome::Begun)
    );
    m.check();

    // 4-6: above the cursor is held, the contiguous one releases the rest.
    assert_eq!(m.delta(a, &d_v2(4), 1_000), Ok(DeltaOutcome::Buffered));
    m.check();
    assert_eq!(
        m.delta(a, &d_v2(3), 1_000),
        Ok(DeltaOutcome::Released { count: 2 })
    );
    m.check();
    assert_eq!(m.delta(a, &d_v2(5), 1_000), Ok(DeltaOutcome::Buffered));
    m.check();

    // 7-11: Begin and Chunk arms.
    assert_eq!(
        m.transfer(a, &o_begin(1, 4), 1_000),
        Ok(TransferOutcome::Duplicate)
    );
    m.check();
    assert_eq!(
        m.transfer(a, &o_chunk(1, 0, 1), 1_000),
        Ok(TransferOutcome::Stored {
            received: 1,
            chunk_count: 2,
        })
    );
    m.check();
    assert_eq!(
        m.transfer(a, &o_chunk(1, 0, 1), 1_000),
        Ok(TransferOutcome::Duplicate)
    );
    m.check();
    assert_eq!(
        m.transfer(a, &o_chunk(1, 0, 2), 1_000),
        Err(TransferError::Failed(FailReason::Conflict))
    );
    m.check();
    let before = m.v3.clone();
    assert_eq!(
        m.transfer(a, &o_chunk(1, 1, 1), 1_000),
        Err(TransferError::StaleTransfer)
    );
    assert_eq!(m.v3, before, "a refusal changed the v3 pool");
    m.check();
    let before = m.v3.clone();
    assert_eq!(
        m.transfer(a, &o_begin(2, 3), 1_000),
        Err(TransferError::BehindReplica)
    );
    assert_eq!(m.v3, before, "a refusal changed the v3 pool");
    m.check();

    // 12-14: a planned snapshot publishes and releases the held D(6).
    let s3 = s_v1(3, 5, Z16);
    let s3v = s_v3(3, 5, Z16);
    assert_eq!(s3[0][0], 0x81);
    assert_eq!(s3v[0][0], 0x82);
    assert_eq!(
        &s3[0][1..],
        &s3v[0][1..],
        "the two Begins differ only in byte 0"
    );
    assert_eq!(
        m.transfer_pair(a, &s3[0], &s3v[0], 1_000),
        Ok(TransferOutcome::Begun)
    );
    m.check();

    // 13: D(6) is above the in-flight cursor.
    assert_eq!(m.delta(a, &d_v2(6), 1_000), Ok(DeltaOutcome::Buffered));
    m.check();

    // 14: the completing chunk publishes, drops D(5) and releases D(6).
    assert_eq!(
        m.transfer_pair(a, &s3[1], &s3v[1], 1_000),
        Ok(TransferOutcome::Published {
            transfer: tid(3),
            released: 1,
        })
    );
    m.check();

    // 15: D(5) is below the new cursor.
    assert_eq!(m.delta(a, &d_v2(5), 1_000), Ok(DeltaOutcome::Superseded));
    m.check();

    // 16: FIFO order.
    for n in 1..=4 {
        assert_eq!(m.pop(a), Some(ReplicaUpdateV3::Delta(dframe(n))));
    }
    assert_eq!(
        m.pop(a),
        Some(ReplicaUpdateV3::Snapshot(PublicationV3 {
            transfer: tid(3),
            base_tick: 10,
            cursor: 5,
            body: e0(),
        }))
    );
    assert_eq!(m.pop(a), Some(ReplicaUpdateV3::Delta(dframe(6))));
    assert_eq!(m.pop(a), None);
    m.check();

    // 17: cancel retires through 9; a Begin at 9 is stale.
    assert_eq!(
        m.transfer(a, &c_frame(9), 1_000),
        Ok(TransferOutcome::Cancelled { discarded: false })
    );
    m.check();
    let s9 = s_v1(9, 6, Z16);
    let s9v = s_v3(9, 6, Z16);
    let before = m.v3.clone();
    assert_eq!(
        m.transfer_pair(a, &s9[0], &s9v[0], 1_000),
        Err(TransferError::StaleTransfer)
    );
    assert_eq!(m.v3, before, "a refusal changed the v3 pool");
    m.check();

    // 18: transfer 10 begins at 2,000.
    assert_eq!(
        m.transfer(a, &o_begin(10, 6), 2_000),
        Ok(TransferOutcome::Begun)
    );
    m.check();

    // 19: B joins.
    let b = m.add(GateMode::Live);
    assert_eq!(b.1.get(), 2);
    m.check();

    // 20-21: B releases through 2; the sweep at 7,000 expires A's transfer.
    assert_eq!(
        m.delta(b, &d_v2(1), 6_999),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    m.check();
    assert_eq!(
        m.delta(b, &d_v2(2), 7_000),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    m.check();
    let a_status = m.status(a).1;
    assert_eq!(a_status.mode, GateMode::AwaitSnapshot);
    assert_eq!(a_status.resync, Some(ResyncReason::Expired));
    assert_eq!(a_status.retired_through, 10);
    assert_eq!(a_status.in_flight, None);
    assert_eq!(m.v3.staging_bytes(), 0);

    // 22: the expired transfer's chunk is stale.
    let before = m.v3.clone();
    assert_eq!(
        m.transfer(a, &o_chunk(10, 0, 10), 7_000),
        Err(TransferError::StaleTransfer)
    );
    assert_eq!(m.v3, before, "a refusal changed the v3 pool");
    m.check();

    // 23-24: transfer 11 expires exactly on its deadline.
    assert_eq!(
        m.transfer(a, &o_begin(11, 6), 8_000),
        Ok(TransferOutcome::Begun)
    );
    m.check();
    assert_eq!(
        m.transfer(a, &o_chunk(11, 0, 11), 13_000),
        Err(TransferError::Failed(FailReason::Expired))
    );
    m.check();

    // 25-26: transfer 12 is live at 18,999 and expired by poll(19,000).
    assert_eq!(
        m.transfer(a, &o_begin(12, 6), 14_000),
        Ok(TransferOutcome::Begun)
    );
    m.check();
    assert_eq!(m.poll(18_999), vec![]);
    m.check();
    assert_eq!(m.poll(19_000), vec![a.1]);
    m.check();
    assert_eq!(m.poll(100), vec![]);
    m.check();

    // 27: cancel discards transfer 13 once.
    assert_eq!(
        m.transfer(a, &o_begin(13, 6), 19_000),
        Ok(TransferOutcome::Begun)
    );
    m.check();
    assert_eq!(
        m.transfer(a, &c_frame(13), 19_000),
        Ok(TransferOutcome::Cancelled { discarded: true })
    );
    m.check();
    assert_eq!(
        m.transfer(a, &c_frame(13), 19_000),
        Ok(TransferOutcome::Cancelled { discarded: false })
    );
    m.check();

    // 28: identical held bytes duplicate; other bytes conflict and keep D(8).
    assert_eq!(m.delta(a, &d_v2(8), 19_000), Ok(DeltaOutcome::Buffered));
    m.check();
    assert_eq!(m.delta(a, &d_v2(8), 19_000), Ok(DeltaOutcome::Duplicate));
    m.check();
    assert_eq!(
        m.delta(a, &[0x02, 0x00, 0x6C, 0x08, 0x00], 19_000),
        Err(TransferError::Failed(FailReason::Conflict))
    );
    m.check();

    // 29: the seventh held frame overflows and drops every held delta.
    for n in 9..=13 {
        assert_eq!(m.delta(a, &d_v2(n), 19_000), Ok(DeltaOutcome::Buffered));
        m.check();
    }
    assert_eq!(
        m.delta(a, &d_v2(14), 19_000),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    m.check();

    // 30: rebind drops the one held delta.
    assert_eq!(m.delta(a, &d_v2(8), 19_000), Ok(DeltaOutcome::Buffered));
    m.check();
    let r1 = m.v1.rebind_receiver(a.0, 19_000);
    let r3 = m.v3.rebind_receiver(a.1, 19_000);
    assert_eq!(r3, r1, "rebind_receiver diverges");
    assert_eq!(
        r3,
        Ok(Rebound {
            discarded: None,
            dropped_deltas: 1,
        })
    );
    m.check();

    // 31: a second snapshot publishes with nothing to release.
    let s14 = s_v1(14, 7, Z16);
    let s14v = s_v3(14, 7, Z16);
    assert_eq!(
        m.transfer_pair(a, &s14[0], &s14v[0], 19_000),
        Ok(TransferOutcome::Begun)
    );
    m.check();
    assert_eq!(
        m.transfer_pair(a, &s14[1], &s14v[1], 19_000),
        Ok(TransferOutcome::Published {
            transfer: tid(14),
            released: 0,
        })
    );
    m.check();

    // 32: D(8) releases after the snapshot.
    assert_eq!(
        m.delta(a, &d_v2(8), 19_000),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    m.check();
    assert_eq!(
        m.pop(a),
        Some(ReplicaUpdateV3::Snapshot(PublicationV3 {
            transfer: tid(14),
            base_tick: 10,
            cursor: 7,
            body: e0(),
        }))
    );
    assert_eq!(m.pop(a), Some(ReplicaUpdateV3::Delta(dframe(8))));
    assert_eq!(m.pop(a), None);
    m.check();

    // 33: a foreign epoch is refused.
    let foreign = [1u8; 16];
    let s15 = s_v1(15, 8, foreign);
    let s15v = s_v3(15, 8, foreign);
    let before = m.v3.clone();
    assert_eq!(
        m.transfer_pair(a, &s15[0], &s15v[0], 19_000),
        Err(TransferError::WrongEpoch)
    );
    assert_eq!(m.v3, before, "a refusal changed the v3 pool");
    m.check();

    // 34: event_seq 0 is refused before the superseded check.
    let before = m.v3.clone();
    assert_eq!(
        m.delta(a, &[0x02, 0x00, 0x00, 0x00, 0x00], 19_000),
        Err(TransferError::DeltaSequence)
    );
    assert_eq!(m.v3, before, "a refusal changed the v3 pool");
    m.check();

    // 35: C buffers because it awaits a snapshot.
    let c = m.add(GateMode::AwaitSnapshot);
    assert_eq!(c.1.get(), 3);
    m.check();
    assert_eq!(m.delta(c, &d_v2(1), 19_000), Ok(DeltaOutcome::Buffered));
    m.check();

    // 36: removal frees B's queued bytes and makes its id unknown.
    assert_eq!(m.v3.held_delta_bytes(), 15);
    m.remove(b);
    assert_eq!(m.v3.held_delta_bytes(), 5);
    m.check();
    let before = m.v3.clone();
    assert_eq!(m.v1.status(b.0), Err(TransferError::UnknownReceiver));
    assert_eq!(m.v3.status(b.1), Err(TransferError::UnknownReceiver));
    assert_eq!(m.v3, before, "a refusal changed the v3 pool");
    let before = m.v3.clone();
    assert_eq!(
        m.delta(b, &d_v2(3), 19_000),
        Err(TransferError::UnknownReceiver)
    );
    assert_eq!(m.v3, before, "a refusal changed the v3 pool");
    m.check();
}
