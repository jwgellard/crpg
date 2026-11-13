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
    DeltaOutcome, FailReason, GateMode, InFlightStatus, PublicationV3, PublishedBase, Rebound,
    ReassemblyPoolV3, ReceiverIdV3, ReceiverStatusV3, ReplicaUpdateV3, ResyncReason,
    TransferError, TransferLimitsV3, TransferOutcome,
};
use crpg_net::protocol::SEQ_LAST;
use crpg_net::protocol_transfer::{
    BeginTransfer, CancelReason, HealthFact, SnapshotBody, TransferChunk, TransferDown, TransferId,
    TurnFact,
};
use crpg_net::protocol_v3::{
    AutoPauseTrigger, AutoPauseToggles, ClockFact, DeltaFrame, DeltaOp, EncounterFacts,
    EncounterMode, FactSet, HostFacts, NetId, PauseReason, PauseRights, PoolFact, StatFact,
    StatValue, TimelineEntry,
};
use crpg_net::reassembly::{DeltaWire, GatedDelta, ReassemblyPool, ReceiverId, ReplicaUpdate, TransferLimits};
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

/// A refusal: the exact error comes back and the whole pool is unchanged.
fn refused_t(
    p: &mut ReassemblyPoolV3,
    rid: ReceiverIdV3,
    bytes: &[u8],
    now: u64,
) -> TransferError {
    let before = p.clone();
    let err = p
        .offer_transfer(rid, bytes, now)
        .expect_err("expected a transfer refusal");
    assert_eq!(*p, before, "a refusal ({err:?}) changed the pool");
    err
}

fn refused_d(
    p: &mut ReassemblyPoolV3,
    rid: ReceiverIdV3,
    bytes: &[u8],
    now: u64,
) -> TransferError {
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

// P1 / P-2, P-3, P-12, P-14: surface, receivers, queries, refusals.

#[test]
fn p1_pool_v3_surface_and_receivers() {
    todo!("P1")
}

#[test]
fn p2_largest_body_publishes_through_the_pool() {
    todo!("P2")
}

#[test]
fn p3_full_envelope_staging_and_aggregate() {
    todo!("P3")
}

#[test]
fn p4_deadline_boundary_19999_20000() {
    todo!("P4")
}

#[test]
fn p5_invalid_bodies_never_publish() {
    todo!("P5")
}

#[test]
fn p6_version_and_wire_refusals() {
    todo!("P6")
}

#[test]
fn p7_v3_ops_pass_the_gate_unchanged() {
    todo!("P7")
}

#[test]
fn p8_mirror_matches_the_v1_pool() {
    todo!("P8")
}

#[test]
fn p9_rebind_discards_in_flight_and_held() {
    todo!("P9")
}

#[test]
fn p10_begin_chunk_and_cancel_arms() {
    todo!("P10")
}

#[test]
fn p11_queue_clear_overflow_bytes_and_poll_order() {
    todo!("P11")
}
