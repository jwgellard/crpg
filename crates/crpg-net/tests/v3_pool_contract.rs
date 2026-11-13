//! T140r1 two-keys suite: the v3 pool (`pool_v3`), written from
//! `tasks/T140r.md` (Behaviour P-1..P-16, Tests P1..P11) and the public API
//! only. The implementer's suite and source were not read.
//!
//! Expected values are contract literals or hand arithmetic, never output of
//! the code under test. Every refusal has an exact error, a pool compared
//! whole against its pre-call clone, and a passing control that differs in
//! one input.

use crpg_core::Ulid;
use crpg_net::codec::CodecError::Malformed;
use crpg_net::codec_transfer as ct;
use crpg_net::codec_v3;
use crpg_net::pool_v3::{
    DeltaOutcome, FailReason, GateMode, InFlightStatus, PublicationV3, PublishedBase,
    ReassemblyPoolV3, Rebound, ReceiverIdV3, ReceiverStatusV3, ReplicaUpdateV3, ResyncReason,
    TransferError, TransferLimitsV3, TransferOutcome,
};
use crpg_net::protocol::SEQ_LAST;
use crpg_net::protocol_transfer::{
    BeginTransfer, CancelReason, HealthFact, SnapshotBody, TransferChunk, TransferDown, TransferId,
    TurnFact,
};
use crpg_net::protocol_v3::{
    AutoPauseToggles, AutoPauseTrigger, ClockFact, DeltaFrame, DeltaOp, EncounterFacts,
    EncounterMode, FactSet, HostFacts, NetId, PauseReason, PauseRights, PoolFact, StatFact,
    StatValue, TimelineEntry,
};
use crpg_net::reassembly::{
    DeltaWire, GatedDelta, ReassemblyPool, ReceiverId, ReplicaUpdate, TransferLimits,
};
use crpg_net::transfer_v3::{self as t3, SnapshotBodyV3, SnapshotEntityV3};

const Z: [u8; 16] = [0; 16];
/// The default clock reading of the contract fixtures.
const T0: u64 = 1_000;

fn tid(n: u64) -> TransferId {
    TransferId::new(n).expect("nonzero transfer id")
}

fn id(n: u64) -> NetId {
    NetId::new(n).expect("nonzero net id")
}

fn varint(mut v: u64) -> Vec<u8> {
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

/// `d(n)` of the contract: an empty v3 delta frame with tick and seq `n`.
fn d(n: u64) -> Vec<u8> {
    codec_v3::encode_delta(&DeltaFrame {
        lane: 0,
        server_tick: n,
        event_seq: n,
        ops: vec![],
    })
    .expect("d(n) encodes")
}

/// A delta frame built by hand on the wire, version 01/02/03; the v3 form and
/// the v2 form of the same frame share every byte but byte 0 (T140 A-5).
fn hand_delta(version: u8, tick: u64, seq: u64) -> Vec<u8> {
    let mut v = vec![version, 0x00];
    v.extend(varint(tick));
    v.extend(varint(seq));
    v.push(0x00);
    v
}

/// `E0` of the contract: the empty v3 body, 6 bytes.
fn e0() -> SnapshotBodyV3 {
    SnapshotBodyV3 {
        entities: vec![],
        turn: None,
        encounter: EncounterFacts::default(),
        timeline: None,
    }
}

/// `plan(t, base, cursor, segs, body)` of the contract.
fn plan(t: u64, base: u64, cursor: u64, segs: u8, body: &SnapshotBodyV3) -> Vec<Vec<u8>> {
    t3::plan_transfer(Z, tid(t), base, cursor, segs, body).expect("plan_transfer plans")
}

/// `G(t)` of the contract: the 4 MiB Begin with transfer `t` (cursor 0),
/// encoded for the v3 pool. `cursor` is overridable for P9.
fn g_begin(t: u64, cursor: u64) -> Vec<u8> {
    t3::encode_down(&TransferDown::Begin(BeginTransfer {
        epoch: Z,
        transfer: tid(t),
        base_tick: 0,
        cursor,
        total_bytes: 4_194_304,
        entity_count: 0,
        segments_per_chunk: 8,
        chunk_count: 128,
    }))
    .expect("G encodes")
}

/// `g(t, i)` of the contract: Chunk `i` with 32,768 bytes of `i`.
fn g_chunk(t: u64, i: u8) -> Vec<u8> {
    t3::encode_down(&TransferDown::Chunk(TransferChunk {
        epoch: Z,
        transfer: tid(t),
        index: i,
        bytes: vec![i; 32_768],
    }))
    .expect("g encodes")
}

/// `B(t, total)` of the contract: a Begin of `total` bytes, 8 segments.
fn b_begin(t: u64, total: u32) -> Vec<u8> {
    let chunks = (total as usize).div_ceil(32_768) as u8;
    t3::encode_down(&TransferDown::Begin(BeginTransfer {
        epoch: Z,
        transfer: tid(t),
        base_tick: 0,
        cursor: 0,
        total_bytes: total,
        entity_count: 0,
        segments_per_chunk: 8,
        chunk_count: chunks,
    }))
    .expect("B encodes")
}

/// `Cancel { Z16, tid(t), Superseded }` encoded for the v3 pool.
fn cancel(t: u64) -> Vec<u8> {
    t3::encode_down(&TransferDown::Cancel {
        epoch: Z,
        transfer: tid(t),
        reason: CancelReason::Superseded,
    })
    .expect("Cancel encodes")
}

fn lim(f: impl FnOnce(&mut TransferLimitsV3)) -> TransferLimitsV3 {
    let mut l = TransferLimitsV3::v3();
    f(&mut l);
    l
}

fn pool(limits: TransferLimitsV3) -> ReassemblyPoolV3 {
    ReassemblyPoolV3::new(limits).expect("valid v3 limits")
}

fn add(p: &mut ReassemblyPoolV3, mode: GateMode) -> ReceiverIdV3 {
    p.add_receiver(Z, mode).expect("receiver")
}

/// Runs the pool-wide sweep at `now` and checks that nothing expired, so that
/// a following refusal is compared at an unchanged clock (P-9/P-14).
fn settle(p: &mut ReassemblyPoolV3, now: u64) {
    assert!(p.poll(now).is_empty(), "unexpected expiry at {now}");
}

/// A refusal: the exact error comes back and the whole pool is unchanged.
fn refused_t(p: &mut ReassemblyPoolV3, rid: ReceiverIdV3, bytes: &[u8], now: u64) -> TransferError {
    settle(p, now);
    let before = p.clone();
    let err = p
        .offer_transfer(rid, bytes, now)
        .expect_err("expected a transfer refusal");
    assert_eq!(*p, before, "a refusal ({err:?}) changed the pool");
    err
}

fn refused_d(p: &mut ReassemblyPoolV3, rid: ReceiverIdV3, bytes: &[u8], now: u64) -> TransferError {
    settle(p, now);
    let before = p.clone();
    let err = p
        .offer_delta(rid, bytes, now)
        .expect_err("expected a delta refusal");
    assert_eq!(*p, before, "a refusal ({err:?}) changed the pool");
    err
}

/// P-3's literal status for a fresh receiver.
fn fresh_status(mode: GateMode) -> ReceiverStatusV3 {
    ReceiverStatusV3 {
        epoch: Z,
        mode,
        in_flight: None,
        retired_through: 0,
        published: None,
        next_event_seq: 1,
        held_frames: 0,
        held_bytes: 0,
        queued_updates: 0,
        resync: None,
    }
}

fn pool_of(id: u128, current: u32, max: u32) -> PoolFact {
    PoolFact {
        pool: Ulid::from_u128(id),
        current,
        max,
    }
}

/// The D5 body (`tasks/T140.md`): 1,024 full entities, a turn, a full
/// encounter, a timeline — 2,847,788 bytes encoded (copied from the T140e
/// two-keys suite's builder, per the brief).
fn largest_body() -> SnapshotBodyV3 {
    let base: u64 = 1 << 63;
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
                    (0..16)
                        .map(|j| pool_of(100 + j, u32::MAX, u32::MAX))
                        .collect(),
                ),
                effects: Some(
                    (0..16)
                        .map(|j| crpg_net::protocol_v3::EffectFact {
                            effect: Ulid::from_u128(200 + j),
                            expires_round: u64::MAX,
                        })
                        .collect(),
                ),
                order: Some(Some(crpg_net::protocol_v3::OrderFact {
                    ability: Ulid::from_u128(300),
                    target: id(base),
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

// P1 / P-2, P-3, P-12, P-14: surface, receivers, queries, refusals.

#[test]
fn p1_pool_v3_surface_and_receivers() {
    // P-2: `new` validates the limits unchanged and keeps them.
    assert_eq!(
        pool(TransferLimitsV3::v3()).limits(),
        TransferLimitsV3::v3()
    );
    let bad_max = lim(|l| l.max_receivers = 0);
    assert_eq!(
        ReassemblyPoolV3::new(bad_max.clone()),
        Err(bad_max.validate().expect_err("0 receivers are invalid")),
        "new passes the limits' error through"
    );
    assert_eq!(
        ReassemblyPoolV3::new(bad_max).err(),
        Some(crpg_net::reassembly_v3::LimitsError::InvalidLimits {
            field: "max_receivers"
        })
    );
    let bad_deadline = lim(|l| l.deadline_ms = 20_001);
    assert_eq!(
        ReassemblyPoolV3::new(bad_deadline.clone()),
        Err(bad_deadline.validate().expect_err("20,001 ms is invalid")),
        "new passes the limits' error through"
    );
    assert_eq!(
        ReassemblyPoolV3::new(bad_deadline).err(),
        Some(crpg_net::reassembly_v3::LimitsError::LoosensPolicy {
            field: "deadline_ms"
        })
    );
    // A fresh pool has no receivers and both totals 0.
    let mut p = pool(TransferLimitsV3::v3());
    assert_eq!(p.staging_bytes(), 0);
    assert_eq!(p.held_delta_bytes(), 0);

    // P-3: ids 1..=8, the ninth is ReceiversFull, ids are never reused.
    let mut ids = Vec::new();
    for i in 1..=8u32 {
        let r = add(&mut p, GateMode::Live);
        assert_eq!(r.get(), i, "receiver {i}");
        ids.push(r);
    }
    let before = p.clone();
    assert_eq!(
        p.add_receiver(Z, GateMode::Live),
        Err(TransferError::ReceiversFull),
        "the ninth receiver is refused"
    );
    assert_eq!(p, before, "ReceiversFull changed the pool");
    p.remove_receiver(ids[2]);
    let ninth = add(&mut p, GateMode::Live);
    assert_eq!(ninth.get(), 9, "the counter never reuses an id");
    assert_eq!(p.status(ninth), Ok(fresh_status(GateMode::Live)));

    // The same literal with the other start mode.
    p.remove_receiver(ids[0]);
    let tenth = add(&mut p, GateMode::AwaitSnapshot);
    assert_eq!(tenth.get(), 10);
    assert_eq!(p.status(tenth), Ok(fresh_status(GateMode::AwaitSnapshot)));

    // P-12 / P-14: a removed receiver is unknown, twice over.
    assert_eq!(
        p.status(ids[2]),
        Err(TransferError::UnknownReceiver),
        "status of a removed id"
    );
    assert_eq!(p.pop_update(ids[2]), None, "pop of a removed id");
    let before = p.clone();
    p.remove_receiver(ids[2]);
    assert_eq!(p, before, "an unknown removal changed the pool");
}

#[test]
fn p2_largest_body_publishes_through_the_pool() {
    let mut p = pool(TransferLimitsV3::v3());
    let a = add(&mut p, GateMode::AwaitSnapshot);
    let body = largest_body();
    let frames = plan(1, 0, 0, 8, &body);
    assert_eq!(frames.len(), 88, "the D5 plan is Begin + 87 chunks");
    assert_eq!(
        p.offer_transfer(a, &frames[0], T0),
        Ok(TransferOutcome::Begun)
    );
    // Chunks 86 down to 1, each above the 1 MiB / 32-chunk v1 envelope.
    for c in (1..=86u8).rev() {
        assert_eq!(
            p.offer_transfer(a, &frames[usize::from(c) + 1], T0),
            Ok(TransferOutcome::Stored {
                received: 87 - c,
                chunk_count: 87,
            }),
            "chunk {c}"
        );
    }
    let st = p.status(a).expect("status");
    let Some(InFlightStatus {
        transfer,
        cursor,
        base_tick,
        received,
        chunk_count,
        deadline_ms,
        reserved_bytes,
    }) = st.in_flight
    else {
        panic!("a transfer is in flight");
    };
    assert_eq!(transfer, tid(1));
    assert_eq!(cursor, 0);
    assert_eq!(base_tick, 0);
    assert_eq!(received, 86);
    assert_eq!(chunk_count, 87);
    assert_eq!(deadline_ms, 21_000, "1,000 + 20,000");
    assert_eq!(reserved_bytes, 2_847_788);
    assert_eq!(p.staging_bytes(), 2_847_788);
    assert_eq!(st.queued_updates, 0);
    // Chunk 0 completes the assembly and publishes the D5 body whole.
    assert_eq!(
        p.offer_transfer(a, &frames[1], T0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    assert_eq!(
        p.pop_update(a),
        Some(ReplicaUpdateV3::Snapshot(PublicationV3 {
            transfer: tid(1),
            base_tick: 0,
            cursor: 0,
            body,
        }))
    );
    assert_eq!(p.pop_update(a), None);
    assert_eq!(p.staging_bytes(), 0);
    let st = p.status(a).expect("status");
    assert_eq!(st.retired_through, 1);
    assert_eq!(
        st.published,
        Some(PublishedBase {
            transfer: tid(1),
            base_tick: 0,
            cursor: 0,
        })
    );
    assert_eq!(st.next_event_seq, 1);
    assert_eq!(st.mode, GateMode::Live);
    assert_eq!(st.resync, None);
}

#[test]
fn p3_full_envelope_staging_and_aggregate() {
    // (1) Eight 4 MiB Begins fill the aggregate exactly.
    let mut p = pool(TransferLimitsV3::v3());
    let mut rs = Vec::new();
    for _ in 0..8 {
        rs.push(add(&mut p, GateMode::Live));
    }
    for r in &rs {
        assert_eq!(
            p.offer_transfer(*r, &g_begin(1, 0), T0),
            Ok(TransferOutcome::Begun),
            "receiver {:?}",
            r.get()
        );
    }
    assert_eq!(p.staging_bytes(), 33_554_432);

    // (2) The per-receiver bound: the floor is accepted, one past it is not.
    let mut p = pool(lim(|l| l.staging_bytes_per_receiver = 2_847_788));
    let a = add(&mut p, GateMode::Live);
    assert_eq!(
        p.offer_transfer(a, &b_begin(1, 2_847_788), T0),
        Ok(TransferOutcome::Begun)
    );
    let b = add(&mut p, GateMode::Live);
    assert_eq!(
        refused_t(&mut p, b, &b_begin(1, 2_847_789), T0),
        TransferError::StagingFull
    );

    // (3) The aggregate bound: exact fits accepted, one byte over refused,
    // and the room comes back on Cancel and on removal.
    let mut p = pool(lim(|l| l.staging_bytes_total = 7_042_092));
    let a = add(&mut p, GateMode::Live);
    let b = add(&mut p, GateMode::Live);
    assert_eq!(
        p.offer_transfer(a, &g_begin(1, 0), T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        refused_t(&mut p, b, &b_begin(1, 2_847_789), T0),
        TransferError::AggregateFull
    );
    assert_eq!(
        p.offer_transfer(b, &b_begin(1, 2_847_788), T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(p.staging_bytes(), 7_042_092);
    assert_eq!(
        p.offer_transfer(a, &cancel(1), T0),
        Ok(TransferOutcome::Cancelled { discarded: true }),
        "A's own transfer is discarded"
    );
    assert_eq!(p.staging_bytes(), 2_847_788);
    p.remove_receiver(b);
    assert_eq!(p.staging_bytes(), 0);
}

#[test]
fn p4_deadline_boundary_19999_20000() {
    let frames = plan(1, 0, 0, 8, &largest_body());
    // Pool 1: live through t + 19,999.
    let mut p = pool(TransferLimitsV3::v3());
    let a = add(&mut p, GateMode::AwaitSnapshot);
    assert_eq!(
        p.offer_transfer(a, &frames[0], T0),
        Ok(TransferOutcome::Begun)
    );
    for c in (1..=86u8).rev() {
        assert_eq!(
            p.offer_transfer(a, &frames[usize::from(c) + 1], T0),
            Ok(TransferOutcome::Stored {
                received: 87 - c,
                chunk_count: 87,
            }),
            "chunk {c}"
        );
    }
    assert_eq!(
        p.offer_transfer(a, &frames[1], 20_999),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        }),
        "20,999 is still live"
    );
    // Pool 2: expired from t + 20,000.
    let mut p = pool(TransferLimitsV3::v3());
    let a = add(&mut p, GateMode::AwaitSnapshot);
    assert_eq!(
        p.offer_transfer(a, &frames[0], T0),
        Ok(TransferOutcome::Begun)
    );
    for c in (1..=86u8).rev() {
        assert_eq!(
            p.offer_transfer(a, &frames[usize::from(c) + 1], T0),
            Ok(TransferOutcome::Stored {
                received: 87 - c,
                chunk_count: 87,
            }),
            "chunk {c}"
        );
    }
    assert_eq!(
        p.offer_transfer(a, &frames[1], 21_000),
        Err(TransferError::Failed(FailReason::Expired)),
        "21,000 is expired"
    );
    let st = p.status(a).expect("status");
    assert!(st.in_flight.is_none(), "the expired transfer is gone");
    assert_eq!(p.staging_bytes(), 0);
    assert_eq!(st.retired_through, 1);
    assert_eq!(st.resync, Some(ResyncReason::Expired));
    assert_eq!(st.mode, GateMode::AwaitSnapshot);
    assert_eq!(st.queued_updates, 0);
    assert_eq!(
        refused_t(&mut p, a, &frames[1], 21_000),
        TransferError::StaleTransfer,
        "chunk 0 after expiry"
    );
    assert_eq!(
        refused_t(&mut p, a, &frames[2], 10),
        TransferError::StaleTransfer,
        "chunk 1 at a clamped clock leaves the pool as it was"
    );
    // Pool 3: `poll` reports the receiver it expired, at the exact boundary.
    let mut p = pool(TransferLimitsV3::v3());
    let a = add(&mut p, GateMode::AwaitSnapshot);
    assert_eq!(
        p.offer_transfer(a, &frames[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert!(p.poll(20_999).is_empty(), "nothing overdue at 20,999");
    assert_eq!(p.poll(21_000), vec![a], "exactly [a] at 21,000");
    // Pool 4: the deadline saturates.
    let mut p = pool(TransferLimitsV3::v3());
    let a = add(&mut p, GateMode::AwaitSnapshot);
    let frames = plan(1, 0, 0, 1, &e0());
    assert_eq!(
        p.offer_transfer(a, &frames[0], u64::MAX - 10),
        Ok(TransferOutcome::Begun)
    );
    let st = p.status(a).expect("status");
    let Some(InFlightStatus { deadline_ms, .. }) = st.in_flight else {
        panic!("a transfer is in flight");
    };
    assert_eq!(deadline_ms, u64::MAX, "u64::MAX - 10 + 20,000 saturates");
}

#[test]
fn p5_invalid_bodies_never_publish() {
    let mut p = pool(TransferLimitsV3::v3());
    let a = add(&mut p, GateMode::AwaitSnapshot);
    let frames = plan(1, 0, 0, 1, &e0());
    assert_eq!(
        p.offer_transfer(a, &frames[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        p.offer_transfer(a, &frames[1], T0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    // A 4 MiB assembly whose bytes are E0 followed by 4,194,298 trailing
    // bytes fails the body decode on the completing chunk (127).
    assert_eq!(
        p.offer_transfer(a, &g_begin(2, 0), T0),
        Ok(TransferOutcome::Begun)
    );
    for i in 0..=126u8 {
        assert_eq!(
            p.offer_transfer(a, &g_chunk(2, i), T0),
            Ok(TransferOutcome::Stored {
                received: i + 1,
                chunk_count: 128,
            }),
            "chunk {i}"
        );
    }
    assert_eq!(
        p.offer_transfer(a, &g_chunk(2, 127), T0),
        Err(TransferError::Failed(FailReason::InvalidBody(Malformed))),
        "E0 plus 4,194,298 trailing bytes is Malformed"
    );
    assert_eq!(p.staging_bytes(), 0);
    let st = p.status(a).expect("status");
    assert_eq!(st.retired_through, 2);
    assert_eq!(st.resync, Some(ResyncReason::Invalid));
    assert_eq!(st.mode, GateMode::AwaitSnapshot);
    assert_eq!(st.queued_updates, 1, "transfer 1's snapshot stays queued");
    // Transfer 3: a one-entity body against `entity_count` 0.
    let begin3 = t3::encode_down(&TransferDown::Begin(BeginTransfer {
        epoch: Z,
        transfer: tid(3),
        base_tick: 0,
        cursor: 0,
        total_bytes: 16,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    }))
    .expect("Begin 3 encodes");
    // T140 D3's 16-byte one-entity body.
    let body3: [u8; 16] = [1, 1, 1, 10, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let chunk3 = t3::encode_down(&TransferDown::Chunk(TransferChunk {
        epoch: Z,
        transfer: tid(3),
        index: 0,
        bytes: body3.to_vec(),
    }))
    .expect("Chunk 3 encodes");
    assert_eq!(p.offer_transfer(a, &begin3, T0), Ok(TransferOutcome::Begun));
    assert_eq!(
        p.offer_transfer(a, &chunk3, T0),
        Err(TransferError::Failed(FailReason::InvalidBody(Malformed))),
        "entity count 1 != 0"
    );
    assert_eq!(p.staging_bytes(), 0);
    assert_eq!(p.status(a).expect("status").retired_through, 3);
    // Transfer 4: E0 plus one trailing byte.
    let begin4 = t3::encode_down(&TransferDown::Begin(BeginTransfer {
        epoch: Z,
        transfer: tid(4),
        base_tick: 0,
        cursor: 0,
        total_bytes: 7,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    }))
    .expect("Begin 4 encodes");
    let chunk4 = t3::encode_down(&TransferDown::Chunk(TransferChunk {
        epoch: Z,
        transfer: tid(4),
        index: 0,
        bytes: vec![0; 7],
    }))
    .expect("Chunk 4 encodes");
    assert_eq!(p.offer_transfer(a, &begin4, T0), Ok(TransferOutcome::Begun));
    assert_eq!(
        p.offer_transfer(a, &chunk4, T0),
        Err(TransferError::Failed(FailReason::InvalidBody(Malformed))),
        "one trailing byte is Malformed"
    );
    assert_eq!(p.staging_bytes(), 0);
    assert_eq!(p.status(a).expect("status").retired_through, 4);
    // Every invalid body left the queue untouched: transfer 1's snapshot.
    assert_eq!(
        p.pop_update(a),
        Some(ReplicaUpdateV3::Snapshot(PublicationV3 {
            transfer: tid(1),
            base_tick: 0,
            cursor: 0,
            body: e0(),
        }))
    );
    assert_eq!(p.pop_update(a), None);
}

#[test]
fn p6_version_and_wire_refusals() {
    let mut p = pool(TransferLimitsV3::v3());
    let a = add(&mut p, GateMode::Live);
    let gone = add(&mut p, GateMode::Live);
    p.remove_receiver(gone);
    // T140 D1's `0x82` Begin, byte 0 set to `81` (a `0x81` frame).
    let mut begin81 = vec![0x81, 0x02, 0x10];
    begin81.extend(Z);
    begin81.extend([0x01, 0x00, 0x00, 0x06, 0x00, 0x01, 0x01]);
    // The same frame with the v3 version byte.
    let mut begin82 = begin81.clone();
    begin82[0] = 0x82;
    assert_eq!(
        refused_t(&mut p, a, &begin81, T0),
        TransferError::Codec(crpg_net::codec::CodecError::UnsupportedVersion),
        "a `0x81` transfer frame to offer_transfer"
    );
    assert_eq!(
        refused_d(&mut p, a, &[0x02, 0x00, 0x01, 0x01, 0x00], T0),
        TransferError::Codec(crpg_net::codec::CodecError::UnsupportedVersion),
        "a v2 delta frame to offer_delta"
    );
    assert_eq!(
        refused_d(&mut p, a, &[0x01, 0x00, 0x01, 0x01, 0x00], T0),
        TransferError::Codec(crpg_net::codec::CodecError::UnsupportedVersion),
        "a v1 delta frame to offer_delta"
    );
    assert_eq!(
        refused_t(&mut p, a, &d(1), T0),
        TransferError::Codec(crpg_net::codec::CodecError::UnsupportedVersion),
        "a v3 delta frame to offer_transfer"
    );
    assert_eq!(
        refused_d(&mut p, a, &begin82, T0),
        TransferError::Codec(crpg_net::codec::CodecError::UnsupportedVersion),
        "a `0x82` Begin to offer_delta"
    );
    assert_eq!(
        refused_d(&mut p, a, &[0x03, 0x00, 0x00, 0x00, 0x00], T0),
        TransferError::DeltaSequence,
        "event_seq 0"
    );
    let max_seq = codec_v3::encode_delta(&DeltaFrame {
        lane: 0,
        server_tick: 1,
        event_seq: u64::MAX,
        ops: vec![],
    })
    .expect("event_seq u64::MAX encodes");
    assert_eq!(
        refused_d(&mut p, a, &max_seq, T0),
        TransferError::DeltaSequence,
        "event_seq above SEQ_LAST"
    );
    assert_eq!(
        refused_t(&mut p, gone, &[0xFF], T0),
        TransferError::UnknownReceiver,
        "unknown receiver, before any decode"
    );
    assert_eq!(
        refused_d(&mut p, gone, &[0xFF], T0),
        TransferError::UnknownReceiver,
        "unknown receiver, before any decode"
    );
    // Positive control: event_seq SEQ_LAST is not refused, it is held.
    assert_eq!(
        p.offer_delta(a, &d(SEQ_LAST), T0),
        Ok(DeltaOutcome::Buffered)
    );
    // P-15: the v1 pool is unchanged and refuses both v3 forms.
    let mut v1 = ReassemblyPool::new(TransferLimits::v1()).expect("v1 limits");
    let r = v1
        .add_receiver(Z, DeltaWire::V2, GateMode::Live)
        .expect("v1 receiver");
    assert_eq!(
        v1.offer_transfer(r, &begin82, T0),
        Err(TransferError::Codec(
            crpg_net::codec::CodecError::UnsupportedVersion
        ))
    );
    assert_eq!(
        v1.offer_delta(r, &d(1), T0),
        Err(TransferError::Codec(
            crpg_net::codec::CodecError::UnsupportedVersion
        ))
    );
}

#[test]
fn p7_v3_ops_pass_the_gate_unchanged() {
    // Every fact op and notice, with ids that are live nowhere.
    let f = DeltaFrame {
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
                        name: "might".into(),
                        value: StatValue::Int(3),
                    }]),
                    tags: Some(vec!["elite".into()]),
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
                auto_pause: Some(AutoPauseToggles::from_bits(0x1F).expect("0x1F is the full mask")),
                turn_timer_ms: Some(Some(30_000)),
            }),
        ],
    };
    let bytes = codec_v3::encode_delta(&f).expect("F encodes");
    // To a Live receiver: released whole, in one update.
    let mut p = pool(TransferLimitsV3::v3());
    let a = add(&mut p, GateMode::Live);
    assert_eq!(
        p.offer_delta(a, &bytes, T0),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    assert_eq!(
        p.held_delta_bytes(),
        bytes.len(),
        "the frame length while queued"
    );
    assert_eq!(p.pop_update(a), Some(ReplicaUpdateV3::Delta(f.clone())));
    assert_eq!(p.held_delta_bytes(), 0, "the pop releases its bytes");
    // To an AwaitSnapshot receiver: held with its encoded length.
    let mut p = pool(TransferLimitsV3::v3());
    let a = add(&mut p, GateMode::AwaitSnapshot);
    assert_eq!(p.offer_delta(a, &bytes, T0), Ok(DeltaOutcome::Buffered));
    let st = p.status(a).expect("status");
    assert_eq!(st.held_frames, 1);
    assert_eq!(st.held_bytes, bytes.len());
    assert_eq!(p.held_delta_bytes(), bytes.len());
}

#[test]
fn p9_rebind_discards_in_flight_and_held() {
    let mut p = pool(TransferLimitsV3::v3());
    let a = add(&mut p, GateMode::Live);
    assert_eq!(
        p.offer_delta(a, &d(1), T0),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    // G(1) with cursor 1 (cursor 0 would be BehindReplica now).
    assert_eq!(
        p.offer_transfer(a, &g_begin(1, 1), T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(p.offer_delta(a, &d(2), T0), Ok(DeltaOutcome::Buffered));
    assert_eq!(p.offer_delta(a, &d(3), T0), Ok(DeltaOutcome::Buffered));
    let st = p.status(a).expect("status");
    let Some(InFlightStatus { transfer, .. }) = st.in_flight else {
        panic!("transfer 1 is in flight");
    };
    assert_eq!(transfer, tid(1));
    assert_eq!(p.staging_bytes(), 4_194_304);
    assert_eq!(p.held_delta_bytes(), 15, "queued d(1) plus two held frames");
    assert_eq!(
        p.rebind_receiver(a, T0),
        Ok(Rebound {
            discarded: Some(tid(1)),
            dropped_deltas: 2,
        })
    );
    let st = p.status(a).expect("status");
    assert!(st.in_flight.is_none());
    assert_eq!(st.retired_through, 1);
    assert_eq!(st.held_frames, 1, "the queued d(1)");
    assert_eq!(st.next_event_seq, 2);
    assert_eq!(st.mode, GateMode::AwaitSnapshot);
    assert_eq!(st.resync, None);
    assert_eq!(p.staging_bytes(), 0);
    assert_eq!(p.held_delta_bytes(), 5);
    match p.pop_update(a) {
        Some(ReplicaUpdateV3::Delta(f)) => {
            assert_eq!((f.server_tick, f.event_seq, f.ops.len()), (1, 1, 0));
        }
        other => panic!("expected d(1)'s frame, got {other:?}"),
    }
    // A removed id is unknown and changes nothing.
    let gone = add(&mut p, GateMode::Live);
    p.remove_receiver(gone);
    let before = p.clone();
    assert_eq!(
        p.rebind_receiver(gone, T0),
        Err(TransferError::UnknownReceiver)
    );
    assert_eq!(p, before, "an unknown rebind changed the pool");
}

// P8 / P-16, P-1: the same script on `ReassemblyPool` (V2 receivers) and
// `ReassemblyPoolV3`; every result, status, total and popped update is
// compared after every call.

/// The v1 and v3 forms of one transfer frame: identical past byte 0.
fn assert_same_but_version(v1: &[u8], v3: &[u8]) {
    assert_eq!(v1.len(), v3.len(), "frame lengths");
    assert_eq!(v1[0], 0x81, "the v1 version byte");
    assert_eq!(v3[0], 0x82, "the v3 version byte");
    assert_eq!(&v1[1..], &v3[1..], "the frames differ past byte 0");
}

/// `O(t, c)` of the contract, in both encodings.
fn o_pair(t: u64, cursor: u64) -> (Vec<u8>, Vec<u8>) {
    let begin = BeginTransfer {
        epoch: Z,
        transfer: tid(t),
        base_tick: 10,
        cursor,
        total_bytes: 8_192,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 2,
    };
    let v1 = ct::encode_down(&TransferDown::Begin(begin.clone())).expect("O encodes for v1");
    let v3 = t3::encode_down(&TransferDown::Begin(begin)).expect("O encodes for v3");
    assert_same_but_version(&v1, &v3);
    (v1, v3)
}

/// `O(t)#i` (and `O(t)#0'` with `fill = t + 1`), in both encodings.
fn chunk_pair(t: u64, index: u8, fill: u8) -> (Vec<u8>, Vec<u8>) {
    let chunk = TransferChunk {
        epoch: Z,
        transfer: tid(t),
        index,
        bytes: vec![fill; 4_096],
    };
    let v1 = ct::encode_down(&TransferDown::Chunk(chunk.clone())).expect("Chunk encodes for v1");
    let v3 = t3::encode_down(&TransferDown::Chunk(chunk)).expect("Chunk encodes for v3");
    assert_same_but_version(&v1, &v3);
    (v1, v3)
}

/// `S(t, c)` of the contract: a planned six-byte body, both encodings.
fn s_pair(t: u64, cursor: u64) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let v1 = ct::plan_transfer(
        Z,
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
    .expect("S plans for v1");
    let v3 = t3::plan_transfer(Z, tid(t), 10, cursor, 1, &e0()).expect("S plans for v3");
    assert_eq!(v1.len(), 2);
    assert_eq!(v3.len(), 2);
    // The two Begins carry the same fields, so they differ only in byte 0.
    // The chunks deliberately carry different bodies (the v1 turn body and
    // E0), so only their length is comparable.
    assert_same_but_version(&v1[0], &v3[0]);
    assert_eq!(v1[1].len(), v3[1].len());
    (v1, v3)
}

/// `C(t)` of the contract, in both encodings.
fn cancel_pair(t: u64) -> (Vec<u8>, Vec<u8>) {
    let msg = TransferDown::Cancel {
        epoch: Z,
        transfer: tid(t),
        reason: CancelReason::Superseded,
    };
    let v1 = ct::encode_down(&msg).expect("C encodes for v1");
    let v3 = t3::encode_down(&msg).expect("C encodes for v3");
    assert_same_but_version(&v1, &v3);
    (v1, v3)
}

/// One mirror: every numbered script step runs on both pools.
struct Mirror {
    v1: ReassemblyPool,
    v3: ReassemblyPoolV3,
    ids: Vec<(ReceiverId, ReceiverIdV3)>,
}

impl Mirror {
    fn new() -> Self {
        let v1 = ReassemblyPool::new(TransferLimits {
            delta_frames_per_receiver: 6,
            ..TransferLimits::v1()
        })
        .expect("the v1 mirror limits are valid");
        let v3 = ReassemblyPoolV3::new(TransferLimitsV3 {
            deadline_ms: 5_000,
            delta_frames_per_receiver: 6,
            ..TransferLimitsV3::v3()
        })
        .expect("the v3 mirror limits are valid");
        Self {
            v1,
            v3,
            ids: Vec::new(),
        }
    }

    fn add(&mut self, mode: GateMode) -> u32 {
        let a = self
            .v1
            .add_receiver(Z, DeltaWire::V2, mode)
            .expect("v1 receiver");
        let b = self.v3.add_receiver(Z, mode).expect("v3 receiver");
        assert_eq!(a.get(), b.get(), "receiver ids diverge");
        self.ids.push((a, b));
        self.check();
        a.get()
    }

    fn pair(&self, raw: u32) -> (ReceiverId, ReceiverIdV3) {
        *self
            .ids
            .iter()
            .find(|(a, _)| a.get() == raw)
            .unwrap_or_else(|| panic!("receiver {raw} is not live"))
    }

    fn delta(
        &mut self,
        raw: u32,
        tick: u64,
        seq: u64,
        now: u64,
    ) -> Result<DeltaOutcome, TransferError> {
        let (a, b) = self.pair(raw);
        let v1b = hand_delta(0x02, tick, seq);
        let v3b = hand_delta(0x03, tick, seq);
        let r1 = self.v1.offer_delta(a, &v1b, now);
        let r2 = self.v3.offer_delta(b, &v3b, now);
        assert_eq!(r1, r2, "delta ({tick}, {seq}) outcomes diverge");
        self.check();
        r2
    }

    fn transfer(
        &mut self,
        raw: u32,
        v1b: &[u8],
        v3b: &[u8],
        now: u64,
    ) -> Result<TransferOutcome, TransferError> {
        let (a, b) = self.pair(raw);
        let r1 = self.v1.offer_transfer(a, v1b, now);
        let r2 = self.v3.offer_transfer(b, v3b, now);
        assert_eq!(r1, r2, "transfer outcomes diverge");
        self.check();
        r2
    }

    fn poll(&mut self, now: u64) {
        let a: Vec<u32> = self.v1.poll(now).iter().map(|r| r.get()).collect();
        let b: Vec<u32> = self.v3.poll(now).iter().map(|r| r.get()).collect();
        assert_eq!(a, b, "poll diverged at {now}");
        self.check();
    }

    /// Pops from both pools, compares the pair, and returns the v3 update.
    fn pop(&mut self, raw: u32) -> Option<ReplicaUpdateV3> {
        let (a, b) = self.pair(raw);
        let u1 = self.v1.pop_update(a);
        let u2 = self.v3.pop_update(b);
        match (&u1, &u2) {
            (None, None) => {}
            (Some(ReplicaUpdate::Delta(GatedDelta::V2(f))), Some(ReplicaUpdateV3::Delta(g))) => {
                assert_eq!(
                    g,
                    &DeltaFrame {
                        lane: f.lane,
                        server_tick: f.server_tick,
                        event_seq: f.event_seq,
                        ops: vec![],
                    },
                    "popped deltas diverge"
                );
            }
            (Some(ReplicaUpdate::Snapshot(p)), Some(ReplicaUpdateV3::Snapshot(q))) => {
                assert_eq!(p.transfer, q.transfer, "snapshot transfer");
                assert_eq!(p.base_tick, q.base_tick, "snapshot base_tick");
                assert_eq!(p.cursor, q.cursor, "snapshot cursor");
                assert_eq!(q.body, e0(), "the v3 body of a mirror snapshot");
            }
            (u1, u2) => panic!("popped updates diverge: v1 {u1:?}, v3 {u2:?}"),
        }
        self.check();
        u2
    }

    fn rebind(&mut self, raw: u32, now: u64) -> Result<Rebound, TransferError> {
        let (a, b) = self.pair(raw);
        let r1 = self.v1.rebind_receiver(a, now);
        let r2 = self.v3.rebind_receiver(b, now);
        assert_eq!(r1, r2, "rebind diverged");
        self.check();
        r2
    }

    fn remove(&mut self, raw: u32) -> (ReceiverId, ReceiverIdV3) {
        let pair = self.pair(raw);
        self.v1.remove_receiver(pair.0);
        self.v3.remove_receiver(pair.1);
        self.ids.retain(|(x, _)| x.get() != raw);
        self.check();
        pair
    }

    /// Every live receiver's status (v1's `wire` dropped) and both totals.
    fn check(&self) {
        for (a, b) in &self.ids {
            let s1 = self.v1.status(*a).expect("v1 status");
            let s2 = self.v3.status(*b).expect("v3 status");
            assert_eq!(s1.epoch, s2.epoch, "epoch");
            assert_eq!(s1.mode, s2.mode, "mode");
            assert_eq!(s1.in_flight, s2.in_flight, "in_flight");
            assert_eq!(s1.retired_through, s2.retired_through, "retired_through");
            assert_eq!(s1.published, s2.published, "published");
            assert_eq!(s1.next_event_seq, s2.next_event_seq, "next_event_seq");
            assert_eq!(s1.held_frames, s2.held_frames, "held_frames");
            assert_eq!(s1.held_bytes, s2.held_bytes, "held_bytes");
            assert_eq!(s1.queued_updates, s2.queued_updates, "queued_updates");
            assert_eq!(s1.resync, s2.resync, "resync");
        }
        assert_eq!(
            self.v1.staging_bytes(),
            self.v3.staging_bytes(),
            "staging_bytes"
        );
        assert_eq!(
            self.v1.held_delta_bytes(),
            self.v3.held_delta_bytes(),
            "held_delta_bytes"
        );
    }
}

/// The expected pop of a mirror delta `D(seq)`.
fn popped_delta(m: &mut Mirror, raw: u32, seq: u64) {
    match m.pop(raw) {
        Some(ReplicaUpdateV3::Delta(f)) => {
            assert_eq!(f.event_seq, seq);
            assert_eq!(f.ops, vec![]);
        }
        other => panic!("expected Delta {seq}, got {other:?}"),
    }
}

/// The expected pop of a mirror snapshot `S(t, base, cursor)`.
fn popped_snapshot(m: &mut Mirror, raw: u32, t: u64, base: u64, cursor: u64) {
    match m.pop(raw) {
        Some(ReplicaUpdateV3::Snapshot(p)) => {
            let PublicationV3 {
                transfer,
                base_tick,
                cursor: got,
                body,
            } = p;
            assert_eq!((transfer, base_tick, got), (tid(t), base, cursor));
            assert_eq!(body, e0());
        }
        other => panic!("expected Snapshot {t}, got {other:?}"),
    }
}

#[test]
fn p8_mirror_matches_the_v1_pool() {
    let mut m = Mirror::new();
    // 1. add A (Live) -> id 1.
    assert_eq!(m.add(GateMode::Live), 1);
    // 2. D(1), then D(2): each Released { 1 }.
    assert_eq!(
        m.delta(1, 1, 1, T0),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    assert_eq!(
        m.delta(1, 2, 2, T0),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    // 3. O(1, 4): Begun.
    let (o1, o3) = o_pair(1, 4);
    assert_eq!(m.transfer(1, &o1, &o3, T0), Ok(TransferOutcome::Begun));
    // 4. D(4): Buffered (at the in-flight cursor).
    assert_eq!(m.delta(1, 4, 4, T0), Ok(DeltaOutcome::Buffered));
    // 5. D(3): Released { 2 } (D3 queued, D4 drains).
    assert_eq!(
        m.delta(1, 3, 3, T0),
        Ok(DeltaOutcome::Released { count: 2 })
    );
    // 6. D(5): Buffered (above the in-flight cursor).
    assert_eq!(m.delta(1, 5, 5, T0), Ok(DeltaOutcome::Buffered));
    // 7. O(1, 4) again: Duplicate.
    assert_eq!(m.transfer(1, &o1, &o3, T0), Ok(TransferOutcome::Duplicate));
    // 8. O(1)#0, again: Stored { 1, 2 }, Duplicate.
    let (c0, c3) = chunk_pair(1, 0, 1);
    assert_eq!(
        m.transfer(1, &c0, &c3, T0),
        Ok(TransferOutcome::Stored {
            received: 1,
            chunk_count: 2,
        })
    );
    assert_eq!(m.transfer(1, &c0, &c3, T0), Ok(TransferOutcome::Duplicate));
    // 9. O(1)#0': Failed(Conflict), which discards the in-flight transfer.
    let (c0x, c3x) = chunk_pair(1, 0, 2);
    assert_eq!(
        m.transfer(1, &c0x, &c3x, T0),
        Err(TransferError::Failed(FailReason::Conflict))
    );
    // 10. O(1)#1: StaleTransfer (transfer 1 retired by the conflict).
    let (c1, c31) = chunk_pair(1, 1, 1);
    assert_eq!(
        m.transfer(1, &c1, &c31, T0),
        Err(TransferError::StaleTransfer)
    );
    // 11. O(2, 3): BehindReplica (cursor 3 + 1 < next_event_seq 5).
    let (o2, o32) = o_pair(2, 3);
    assert_eq!(
        m.transfer(1, &o2, &o32, T0),
        Err(TransferError::BehindReplica)
    );
    // 12. S(3, 5) Begin: Begun.
    let (s3, s33) = s_pair(3, 5);
    assert_eq!(
        m.transfer(1, &s3[0], &s33[0], T0),
        Ok(TransferOutcome::Begun)
    );
    // 13. D(6): Buffered (above the new in-flight cursor 5).
    assert_eq!(m.delta(1, 6, 6, T0), Ok(DeltaOutcome::Buffered));
    // 14. S(3, 5) Chunk: Published { 3, 1 } (D5 dropped, D6 moves).
    assert_eq!(
        m.transfer(1, &s3[1], &s33[1], T0),
        Ok(TransferOutcome::Published {
            transfer: tid(3),
            released: 1,
        })
    );
    // 15. D(5): Superseded (below the published cursor).
    assert_eq!(m.delta(1, 5, 5, T0), Ok(DeltaOutcome::Superseded));
    // 16. pop A x 7: D1, D2, D3, D4, Snapshot(3, 10, 5), D6, None.
    popped_delta(&mut m, 1, 1);
    popped_delta(&mut m, 1, 2);
    popped_delta(&mut m, 1, 3);
    popped_delta(&mut m, 1, 4);
    popped_snapshot(&mut m, 1, 3, 10, 5);
    popped_delta(&mut m, 1, 6);
    assert_eq!(m.pop(1), None);
    // 17. C(9): Cancelled { false }; S(9, 6) Begin: StaleTransfer.
    let (c9, c93) = cancel_pair(9);
    assert_eq!(
        m.transfer(1, &c9, &c93, T0),
        Ok(TransferOutcome::Cancelled { discarded: false })
    );
    let (s9, s93) = s_pair(9, 6);
    assert_eq!(
        m.transfer(1, &s9[0], &s93[0], T0),
        Err(TransferError::StaleTransfer)
    );
    // 18. O(10, 6): Begun.
    let (o10, o103) = o_pair(10, 6);
    assert_eq!(
        m.transfer(1, &o10, &o103, 2_000),
        Ok(TransferOutcome::Begun)
    );
    // 19. add B (Live) -> id 2.
    assert_eq!(m.add(GateMode::Live), 2);
    // 20. B D(1) at 6,999: Released { 1 } (A's transfer is still live).
    assert_eq!(
        m.delta(2, 1, 1, 6_999),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    // 21. B D(2) at 7,000: Released { 1 }; A's transfer 10 expires in the sweep.
    assert_eq!(
        m.delta(2, 2, 2, 7_000),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    for (a, b) in &m.ids {
        if a.get() == 1 {
            let s1 = m.v1.status(*a).expect("v1 A");
            let s2 = m.v3.status(*b).expect("v3 A");
            assert_eq!(s1.resync, Some(ResyncReason::Expired));
            assert_eq!(s2.resync, Some(ResyncReason::Expired));
            assert_eq!(s1.mode, GateMode::AwaitSnapshot);
            assert_eq!(s2.mode, GateMode::AwaitSnapshot);
            assert_eq!(s1.retired_through, 10);
            assert_eq!(s2.retired_through, 10);
        }
    }
    // 22. A O(10)#0: StaleTransfer (the expiry already happened in step 21).
    let (c100, c1003) = chunk_pair(10, 0, 10);
    assert_eq!(
        m.transfer(1, &c100, &c1003, 7_000),
        Err(TransferError::StaleTransfer)
    );
    // 23. A O(11, 6): Begun at 8,000.
    let (o11, o113) = o_pair(11, 6);
    assert_eq!(
        m.transfer(1, &o11, &o113, 8_000),
        Ok(TransferOutcome::Begun)
    );
    // 24. A O(11)#0 at 13,000: the sweep expires transfer 11 and the frame
    // names it, so Failed(Expired).
    let (c110, c1103) = chunk_pair(11, 0, 11);
    assert_eq!(
        m.transfer(1, &c110, &c1103, 13_000),
        Err(TransferError::Failed(FailReason::Expired))
    );
    // 25. A O(12, 6): Begun at 14,000.
    let (o12, o123) = o_pair(12, 6);
    assert_eq!(
        m.transfer(1, &o12, &o123, 14_000),
        Ok(TransferOutcome::Begun)
    );
    // 26. poll: [], [A], [].
    m.poll(18_999);
    m.poll(19_000);
    m.poll(100);
    // 27. O(13, 6): Begun; C(13): Cancelled { true }; C(13): Cancelled { false }.
    let (o13, o133) = o_pair(13, 6);
    assert_eq!(
        m.transfer(1, &o13, &o133, 19_000),
        Ok(TransferOutcome::Begun)
    );
    let (c13, c133) = cancel_pair(13);
    assert_eq!(
        m.transfer(1, &c13, &c133, 19_000),
        Ok(TransferOutcome::Cancelled { discarded: true })
    );
    assert_eq!(
        m.transfer(1, &c13, &c133, 19_000),
        Ok(TransferOutcome::Cancelled { discarded: false })
    );
    // 28. D(8), D(8), D'(8): Buffered, Duplicate, Failed(Conflict).
    assert_eq!(m.delta(1, 8, 8, 19_000), Ok(DeltaOutcome::Buffered));
    assert_eq!(m.delta(1, 8, 8, 19_000), Ok(DeltaOutcome::Duplicate));
    assert_eq!(
        m.delta(1, 108, 8, 19_000),
        Err(TransferError::Failed(FailReason::Conflict))
    );
    // 29. D(9)..=D(13): Buffered x 5; D(14): Failed(Overflow), held dropped.
    for seq in 9..=13u64 {
        assert_eq!(m.delta(1, seq, seq, 19_000), Ok(DeltaOutcome::Buffered));
    }
    assert_eq!(
        m.delta(1, 14, 14, 19_000),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    // 30. D(8): Buffered; rebind: Rebound { None, 1 }.
    assert_eq!(m.delta(1, 8, 8, 19_000), Ok(DeltaOutcome::Buffered));
    assert_eq!(
        m.rebind(1, 19_000),
        Ok(Rebound {
            discarded: None,
            dropped_deltas: 1,
        })
    );
    // 31. S(14, 7) Begin, Chunk: Begun, Published { 14, 0 }.
    let (s14, s143) = s_pair(14, 7);
    assert_eq!(
        m.transfer(1, &s14[0], &s143[0], 19_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        m.transfer(1, &s14[1], &s143[1], 19_000),
        Ok(TransferOutcome::Published {
            transfer: tid(14),
            released: 0,
        })
    );
    // 32. D(8): Released { 1 }; pop A x 3: Snapshot(14, 10, 7), D8, None.
    assert_eq!(
        m.delta(1, 8, 8, 19_000),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    popped_snapshot(&mut m, 1, 14, 10, 7);
    popped_delta(&mut m, 1, 8);
    assert_eq!(m.pop(1), None);
    // 33. S(15, 8) planned with epoch [1; 16]: WrongEpoch.
    let begin15 = BeginTransfer {
        epoch: [1; 16],
        transfer: tid(15),
        base_tick: 10,
        cursor: 8,
        total_bytes: 6,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    };
    let v1 = ct::encode_down(&TransferDown::Begin(begin15.clone())).expect("S(15) encodes for v1");
    let v3 = t3::encode_down(&TransferDown::Begin(begin15)).expect("S(15) encodes for v3");
    assert_eq!(
        m.transfer(1, &v1, &v3, 19_000),
        Err(TransferError::WrongEpoch)
    );
    // 34. D(0): DeltaSequence.
    assert_eq!(m.delta(1, 0, 0, 19_000), Err(TransferError::DeltaSequence));
    // 35. add C (AwaitSnapshot) -> id 3; C D(1): Buffered.
    assert_eq!(m.add(GateMode::AwaitSnapshot), 3);
    assert_eq!(m.delta(3, 1, 1, 19_000), Ok(DeltaOutcome::Buffered));
    // 36. remove B; status(B): UnknownReceiver; B D(3): UnknownReceiver.
    let (gone1, gone3) = m.remove(2);
    assert_eq!(
        m.v1.status(gone1).err(),
        Some(TransferError::UnknownReceiver)
    );
    assert_eq!(
        m.v3.status(gone3).err(),
        Some(TransferError::UnknownReceiver)
    );
    let r1 = m.v1.offer_delta(gone1, &hand_delta(0x02, 3, 3), 19_000);
    let r2 = m.v3.offer_delta(gone3, &hand_delta(0x03, 3, 3), 19_000);
    assert_eq!(r1, r2, "a delta to a removed receiver");
    assert_eq!(r2, Err(TransferError::UnknownReceiver));
}

#[test]
fn p10_begin_chunk_and_cancel_arms() {
    /// `Ob(t, base, c)`: the 8,192-byte, two-chunk Begin of the contract.
    fn ob(t: u64, base: u64, c: u64) -> Vec<u8> {
        t3::encode_down(&TransferDown::Begin(BeginTransfer {
            epoch: Z,
            transfer: tid(t),
            base_tick: base,
            cursor: c,
            total_bytes: 8_192,
            entity_count: 0,
            segments_per_chunk: 1,
            chunk_count: 2,
        }))
        .expect("Ob encodes")
    }

    /// `oc(t, i, n)`: Chunk `i` with `n` bytes of `t as u8`.
    fn oc(t: u64, i: u8, n: usize) -> Vec<u8> {
        t3::encode_down(&TransferDown::Chunk(TransferChunk {
            epoch: Z,
            transfer: tid(t),
            index: i,
            bytes: vec![t as u8; n],
        }))
        .expect("oc encodes")
    }

    let mut p = pool(TransferLimitsV3::v3());
    let a = add(&mut p, GateMode::Live);
    let wrong_epoch: [u8; 16] = [1; 16];
    // 1. plan(1, 20, 0, 1, E0) publishes; its snapshot stays queued.
    let frames = plan(1, 20, 0, 1, &e0());
    assert_eq!(
        p.offer_transfer(a, &frames[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        p.offer_transfer(a, &frames[1], T0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    // 2. A Begin with an equal base is accepted.
    assert_eq!(
        p.offer_transfer(a, &ob(2, 20, 0), T0),
        Ok(TransferOutcome::Begun)
    );
    // 3. A second in-flight transfer is refused whole.
    assert_eq!(
        refused_t(&mut p, a, &ob(3, 20, 0), T0),
        TransferError::InFlight
    );
    // 4. A chunk for a never-begun id above `retired_through`.
    assert_eq!(
        refused_t(&mut p, a, &oc(5, 0, 4_096), T0),
        TransferError::UnknownTransfer
    );
    // 5. The wrong epoch is checked before the message, for every kind.
    let mut chunk_wrong = TransferChunk {
        epoch: wrong_epoch,
        transfer: tid(2),
        index: 0,
        bytes: vec![2; 4_096],
    };
    assert_eq!(
        refused_t(
            &mut p,
            a,
            &t3::encode_down(&TransferDown::Chunk(chunk_wrong.clone())).expect("encodes"),
            T0
        ),
        TransferError::WrongEpoch
    );
    assert_eq!(
        refused_t(
            &mut p,
            a,
            &t3::encode_down(&TransferDown::Cancel {
                epoch: wrong_epoch,
                transfer: tid(2),
                reason: CancelReason::Superseded,
            })
            .expect("encodes"),
            T0
        ),
        TransferError::WrongEpoch
    );
    chunk_wrong.epoch = Z;
    // 6. Cancelling an already-published id touches neither the in-flight
    // transfer nor the queue.
    assert_eq!(
        p.offer_transfer(a, &cancel(1), T0),
        Ok(TransferOutcome::Cancelled { discarded: false })
    );
    let st = p.status(a).expect("status");
    assert_eq!(st.retired_through, 1, "id 1 was already retired");
    let Some(InFlightStatus { transfer, .. }) = st.in_flight else {
        panic!("transfer 2 is still in flight");
    };
    assert_eq!(transfer, tid(2));
    // 7. The same id with a different field is a Conflict failure.
    assert_eq!(
        p.offer_transfer(a, &ob(2, 20, 1), T0),
        Err(TransferError::Failed(FailReason::Conflict)),
        "same id, cursor changed"
    );
    assert_failed_clean(&p, a, 2, 1);
    // 8. A base below the published one is BehindReplica; an equal one is not.
    assert_eq!(
        refused_t(&mut p, a, &ob(3, 19, 0), T0),
        TransferError::BehindReplica
    );
    assert_eq!(
        p.offer_transfer(a, &ob(3, 20, 0), T0),
        Ok(TransferOutcome::Begun)
    );
    // 9. An index at the chunk count is out of range, so Failed(Conflict).
    assert_eq!(
        p.offer_transfer(a, &oc(3, 2, 4_096), T0),
        Err(TransferError::Failed(FailReason::Conflict)),
        "index == chunk_count"
    );
    assert_failed_clean(&p, a, 3, 1);
    // 10. A chunk one byte short is a WrongLength, so Failed(Conflict).
    assert_eq!(
        p.offer_transfer(a, &ob(4, 20, 0), T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        p.offer_transfer(a, &oc(4, 0, 4_095), T0),
        Err(TransferError::Failed(FailReason::Conflict)),
        "4,095 != 4,096"
    );
    assert_failed_clean(&p, a, 4, 1);
    // The queued snapshot from step 1 survived every failure.
    assert_eq!(
        p.pop_update(a),
        Some(ReplicaUpdateV3::Snapshot(PublicationV3 {
            transfer: tid(1),
            base_tick: 20,
            cursor: 0,
            body: e0(),
        }))
    );
    assert_eq!(p.pop_update(a), None);
}

/// The post-failure state the contract names after P10's steps 7, 9 and 10.
fn assert_failed_clean(p: &ReassemblyPoolV3, rid: ReceiverIdV3, retired: u64, queued: usize) {
    let st = p.status(rid).expect("status");
    assert!(st.in_flight.is_none(), "no transfer in flight");
    assert_eq!(p.staging_bytes(), 0);
    assert_eq!(st.retired_through, retired);
    assert_eq!(st.resync, Some(ResyncReason::Invalid));
    assert_eq!(st.mode, GateMode::AwaitSnapshot);
    assert_eq!(st.queued_updates, queued);
}

#[test]
fn p11_queue_clear_overflow_bytes_and_poll_order() {
    /// `TL(n, k)` of the contract: one Timeline op with `k` entries.
    fn tl(n: u64, k: u64) -> Vec<u8> {
        codec_v3::encode_delta(&DeltaFrame {
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
        .expect("TL encodes")
    }

    // (a) Queue clear: a publication over a queued snapshot clears the whole
    // queue, its delta bytes included.
    let mut p = pool(TransferLimitsV3::v3());
    let a = add(&mut p, GateMode::Live);
    let frames = plan(1, 0, 0, 1, &e0());
    assert_eq!(
        p.offer_transfer(a, &frames[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        p.offer_transfer(a, &frames[1], T0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    assert_eq!(
        p.offer_delta(a, &d(1), T0),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    let frames = plan(2, 0, 1, 1, &e0());
    assert_eq!(
        p.offer_transfer(a, &frames[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        p.offer_transfer(a, &frames[1], T0),
        Ok(TransferOutcome::Published {
            transfer: tid(2),
            released: 0,
        })
    );
    let st = p.status(a).expect("status");
    assert_eq!(st.queued_updates, 1, "only the new snapshot is queued");
    assert_eq!(p.held_delta_bytes(), 0, "d(1) went with the queue");
    match p.pop_update(a) {
        Some(ReplicaUpdateV3::Snapshot(pub_)) => {
            let PublicationV3 {
                transfer,
                base_tick,
                cursor,
                body,
            } = pub_;
            assert_eq!((transfer, base_tick, cursor), (tid(2), 0, 1));
            assert_eq!(body, e0());
        }
        other => panic!("expected transfer 2's snapshot, got {other:?}"),
    }
    assert_eq!(p.pop_update(a), None);

    // (b) Per-receiver bytes: held bytes exactly at the bound, one frame past
    // it is Overflow, and Overflow drops every held delta.
    let mut p = pool(lim(|l| l.delta_bytes_per_receiver = 65_536));
    let a = add(&mut p, GateMode::AwaitSnapshot);
    for n in 1..=22u64 {
        assert_eq!(
            p.offer_delta(a, &tl(n, 1_024), T0),
            Ok(DeltaOutcome::Buffered),
            "TL({n})"
        );
    }
    assert_eq!(
        codec_v3::encode_delta(&DeltaFrame {
            lane: 0,
            server_tick: 23,
            event_seq: 23,
            ops: vec![DeltaOp::Timeline {
                entries: Some(
                    (1..=222)
                        .map(|i| TimelineEntry {
                            key: 0,
                            entity: id(i)
                        })
                        .collect()
                ),
            }],
        })
        .expect("TL(23, 222) encodes")
        .len(),
        548,
        "TL(n, 222) is 548 bytes"
    );
    assert_eq!(
        p.offer_delta(a, &tl(23, 222), T0),
        Ok(DeltaOutcome::Buffered)
    );
    let st = p.status(a).expect("status");
    assert_eq!(st.held_bytes, 65_536, "exactly at the bound");
    assert_eq!(st.held_frames, 23);
    assert_eq!(p.held_delta_bytes(), 65_536);
    assert_eq!(
        p.offer_delta(a, &d(24), T0),
        Err(TransferError::Failed(FailReason::Overflow)),
        "one 5-byte frame past the bound"
    );
    let st = p.status(a).expect("status");
    assert_eq!(st.held_frames, 0);
    assert_eq!(st.held_bytes, 0);
    assert_eq!(st.resync, Some(ResyncReason::Overflow));
    assert_eq!(st.mode, GateMode::AwaitSnapshot);
    assert_eq!(p.held_delta_bytes(), 0);

    // (c) Pool bytes: the pool total is the third check, and B's Overflow
    // leaves A untouched.
    let mut p = pool(lim(|l| {
        l.delta_bytes_per_receiver = 65_536;
        l.delta_bytes_total = 65_536;
    }));
    let a = add(&mut p, GateMode::AwaitSnapshot);
    let b = add(&mut p, GateMode::AwaitSnapshot);
    for n in 1..=22u64 {
        assert_eq!(
            p.offer_delta(a, &tl(n, 1_024), T0),
            Ok(DeltaOutcome::Buffered)
        );
    }
    assert_eq!(
        p.offer_delta(b, &tl(1, 222), T0),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(p.held_delta_bytes(), 65_536, "the pool is exactly full");
    assert_eq!(
        p.offer_delta(b, &d(2), T0),
        Err(TransferError::Failed(FailReason::Overflow)),
        "5 bytes past the pool total"
    );
    assert_eq!(p.held_delta_bytes(), 64_988, "B's held bytes came back");
    let st = p.status(a).expect("A is untouched");
    assert_eq!(st.held_bytes, 64_988);
    assert_eq!(st.held_frames, 22);

    // (d) Queued updates survive Overflow; only held deltas are dropped.
    let mut p = pool(lim(|l| l.delta_frames_per_receiver = 2));
    let a = add(&mut p, GateMode::Live);
    assert_eq!(
        p.offer_delta(a, &d(1), T0),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    assert_eq!(p.offer_delta(a, &d(3), T0), Ok(DeltaOutcome::Buffered));
    assert_eq!(
        p.offer_delta(a, &d(4), T0),
        Err(TransferError::Failed(FailReason::Overflow)),
        "a third frame past the two-frame bound"
    );
    let st = p.status(a).expect("status");
    assert_eq!(st.queued_updates, 1, "the released d(1) survives");
    assert_eq!(st.held_frames, 1, "held 0 plus queued 1");
    assert_eq!(st.held_bytes, 5);
    assert_eq!(st.mode, GateMode::AwaitSnapshot);
    match p.pop_update(a) {
        Some(ReplicaUpdateV3::Delta(f)) => {
            assert_eq!((f.server_tick, f.event_seq, f.ops.len()), (1, 1, 0));
        }
        other => panic!("expected d(1)'s frame, got {other:?}"),
    }

    // (e) Poll order is ascending by id, whatever the begin order was.
    let mut p = pool(TransferLimitsV3::v3());
    let a = add(&mut p, GateMode::Live);
    let b = add(&mut p, GateMode::Live);
    assert_eq!(
        p.offer_transfer(b, &g_begin(1, 0), T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        p.offer_transfer(a, &g_begin(1, 0), T0),
        Ok(TransferOutcome::Begun)
    );
    assert!(p.poll(20_999).is_empty());
    assert_eq!(p.poll(21_000), vec![a, b], "ascending by id");
    assert_eq!(p.staging_bytes(), 0);
}
