//! T024 snapshot transfer suite (`tasks/T024.md` S§12), public API only.
//!
//! The oracle in this file is test-local and independent of production code:
//! it applies popped [`ReplicaUpdate`]s to a `BTreeMap` replica and compares
//! the result fact by fact with hand-authored expected facts. It never calls a
//! production projection, and there is no World hash. The host stand-in builds
//! bodies from a fixture table of facts and grants. Wire bytes for the codec
//! cases are built by hand here, not by the codec under test.

use std::collections::BTreeMap;

use crpg_net::codec;
use crpg_net::codec_transfer::*;
use crpg_net::codec_v2;
use crpg_net::protocol::{self, DeltaOp, SEQ_LAST};
use crpg_net::protocol_transfer::*;
use crpg_net::protocol_v2;
use crpg_net::reassembly::*;
use crpg_net::sim::{FaultSchedule, InMemoryTransport, PeerId, QueueCaps, RateCaps, SimClock};

const EPOCH: [u8; 16] = [0x11; 16];
const OTHER_EPOCH: [u8; 16] = [0x22; 16];
const ZERO_EPOCH: [u8; 16] = [0; 16];
/// The first `NetId` of the maximal body: 1,024 ids that all need ten varint
/// bytes.
const MAX_BASE: u64 = u64::MAX - 1023;
const MAX_BODY_BYTES: usize = 22_552;
const MIB: usize = 1_048_576;
const T0: u64 = 1_000;
/// `big_fixture()`'s body: 2 + 1,024 x 22 + a 13-byte turn.
const BIG_FIXTURE_BYTES: usize = 22_543;

// ---------------------------------------------------------------------------
// Small constructors and hand-written wire builders.
// ---------------------------------------------------------------------------

fn net(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}

fn tid(raw: u64) -> TransferId {
    TransferId::new(raw).expect("nonzero")
}

fn health(h: u32, m: u32, dead: bool) -> HealthFact {
    HealthFact {
        health: h,
        max_health: m,
        dead,
    }
}

/// LEB128, written here so the codec is never its own oracle.
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

/// A ten-byte varint whose last byte overflows `u64`.
fn overflow_varint() -> Vec<u8> {
    let mut v = vec![0xFF; 9];
    v.push(0x02);
    v
}

fn header(version: u8, lane: u8, tag: u8) -> Vec<u8> {
    vec![version, lane, tag]
}

fn transfer_header(tag: u8) -> Vec<u8> {
    header(TRANSFER_VERSION, LANE_SNAPSHOT, tag)
}

/// A `Begin` frame from raw varint bytes for the three count fields.
#[allow(clippy::too_many_arguments)]
fn begin_wire(
    epoch: [u8; 16],
    transfer: &[u8],
    base_tick: u64,
    cursor: &[u8],
    total: &[u8],
    entities: &[u8],
    spc: u8,
    chunks: u8,
) -> Vec<u8> {
    let mut b = transfer_header(0x10);
    b.extend(epoch);
    b.extend(transfer);
    b.extend(varint(base_tick));
    b.extend(cursor);
    b.extend(total);
    b.extend(entities);
    b.push(spc);
    b.push(chunks);
    b
}

fn begin_wire_of(b: &BeginTransfer) -> Vec<u8> {
    begin_wire(
        b.epoch,
        &varint(b.transfer.get()),
        b.base_tick,
        &varint(b.cursor),
        &varint(u64::from(b.total_bytes)),
        &varint(u64::from(b.entity_count)),
        b.segments_per_chunk,
        b.chunk_count,
    )
}

/// A `Chunk` frame header up to and including the segment count, then `tail`.
fn chunk_wire(transfer: &[u8], index: u8, segment_count: u8, tail: &[u8]) -> Vec<u8> {
    let mut b = transfer_header(0x11);
    b.extend(ZERO_EPOCH);
    b.extend(transfer);
    b.push(index);
    b.push(segment_count);
    b.extend(tail);
    b
}

/// One well-formed segment: `len` varint then `len` bytes of `fill`.
fn seg(len: usize, fill: u8) -> Vec<u8> {
    let mut s = varint(len as u64);
    s.extend(vec![fill; len]);
    s
}

fn cancel_wire(transfer: u64, reason: u8) -> Vec<u8> {
    let mut b = transfer_header(0x12);
    b.extend(ZERO_EPOCH);
    b.extend(varint(transfer));
    b.push(reason);
    b
}

fn published_wire(transfer: u64) -> Vec<u8> {
    let mut b = transfer_header(0x20);
    b.extend(ZERO_EPOCH);
    b.extend(varint(transfer));
    b
}

fn resync_wire(after: u64, reason: u8) -> Vec<u8> {
    let mut b = transfer_header(0x21);
    b.extend(ZERO_EPOCH);
    b.extend(varint(after));
    b.push(reason);
    b
}

// ---------------------------------------------------------------------------
// Frame builders on top of the codec (used by the pool cases).
// ---------------------------------------------------------------------------

fn begin_of(
    epoch: [u8; 16],
    id: u64,
    base_tick: u64,
    cursor: u64,
    total_bytes: u32,
    entity_count: u16,
    spc: u8,
) -> BeginTransfer {
    let chunk_size = u32::from(spc) * 4096;
    BeginTransfer {
        epoch,
        transfer: tid(id),
        base_tick,
        cursor,
        total_bytes,
        entity_count,
        segments_per_chunk: spc,
        chunk_count: total_bytes.div_ceil(chunk_size) as u8,
    }
}

fn begin_frame(b: &BeginTransfer) -> Vec<u8> {
    encode_down(&TransferDown::Begin(b.clone())).expect("begin encodes")
}

fn chunk_frame(epoch: [u8; 16], id: u64, index: u8, bytes: &[u8]) -> Vec<u8> {
    encode_down(&TransferDown::Chunk(TransferChunk {
        epoch,
        transfer: tid(id),
        index,
        bytes: bytes.to_vec(),
    }))
    .expect("chunk encodes")
}

fn cancel_frame(epoch: [u8; 16], id: u64, reason: CancelReason) -> Vec<u8> {
    encode_down(&TransferDown::Cancel {
        epoch,
        transfer: tid(id),
        reason,
    })
    .expect("cancel encodes")
}

/// Begin plus canonically cut chunks (S§5.3) for an arbitrary payload.
fn payload_frames(
    epoch: [u8; 16],
    id: u64,
    base_tick: u64,
    cursor: u64,
    spc: u8,
    entity_count: u16,
    payload: &[u8],
) -> Vec<Vec<u8>> {
    let begin = begin_of(
        epoch,
        id,
        base_tick,
        cursor,
        payload.len() as u32,
        entity_count,
        spc,
    );
    let chunk_size = usize::from(spc) * 4096;
    let mut frames = vec![begin_frame(&begin)];
    for (i, part) in payload.chunks(chunk_size).enumerate() {
        frames.push(chunk_frame(epoch, id, i as u8, part));
    }
    assert_eq!(frames.len() - 1, usize::from(begin.chunk_count));
    frames
}

fn plan(id: u64, base_tick: u64, cursor: u64, spc: u8, body: &SnapshotBody) -> Vec<Vec<u8>> {
    plan_transfer(EPOCH, tid(id), base_tick, cursor, spc, body).expect("plan")
}

/// The maximal body: 1,024 entities, every fact, maximal values (22,552 B).
fn max_body() -> SnapshotBody {
    SnapshotBody {
        entities: (0..1024u64)
            .map(|i| SnapshotEntity {
                entity: net(MAX_BASE + i),
                health: Some(health(u32::MAX, u32::MAX, true)),
            })
            .collect(),
        turn: Some(TurnFact {
            active: Some(net(MAX_BASE)),
            round: u64::MAX,
        }),
    }
}

fn max_body_facts() -> Facts {
    let mut f = Facts::default();
    for i in 0..1024u64 {
        f.entities
            .insert(MAX_BASE + i, Some((u32::MAX, u32::MAX, true)));
    }
    f.turn = Some((Some(MAX_BASE), u64::MAX));
    f
}

/// A small body whose health value tags it: entities 1 and 2.
fn small_body(tag: u32) -> SnapshotBody {
    SnapshotBody {
        entities: vec![
            SnapshotEntity {
                entity: net(1),
                health: Some(health(tag, 100, false)),
            },
            SnapshotEntity {
                entity: net(2),
                health: None,
            },
        ],
        turn: None,
    }
}

fn small_facts(tag: u32) -> Facts {
    facts(&[(1, Some((tag, 100, false))), (2, None)], None)
}

// ---------------------------------------------------------------------------
// The test-local oracle.
// ---------------------------------------------------------------------------

type HealthTuple = (u32, u32, bool);
type TurnTuple = (Option<u64>, u64);

/// A replica view as plain facts: entity -> optional health, plus the turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Facts {
    entities: BTreeMap<u64, Option<HealthTuple>>,
    turn: Option<TurnTuple>,
}

/// Hand-authored expected facts.
fn facts(entities: &[(u64, Option<HealthTuple>)], turn: Option<TurnTuple>) -> Facts {
    Facts {
        entities: entities.iter().copied().collect(),
        turn,
    }
}

/// Applies popped updates, in pop order, to a `BTreeMap` replica.
///
/// A snapshot replaces the whole view; `EntityEnter` starts a fresh record
/// with no known health (a newly visible entity carries no facts yet);
/// `Health` and `Turn` set facts; `EntityLeave` forgets the entity.
#[derive(Debug, Clone, Default)]
struct Replica {
    facts: Facts,
    /// `"S"` for a snapshot and `"D<n>"` for the delta with `event_seq` n.
    order: Vec<String>,
    snapshots: usize,
}

impl Replica {
    fn apply_op(&mut self, op: &DeltaOp) {
        match op {
            DeltaOp::EntityEnter { entity } => {
                self.facts.entities.insert(entity.get(), None);
            }
            DeltaOp::EntityLeave { entity } => {
                self.facts.entities.remove(&entity.get());
            }
            DeltaOp::Health {
                entity,
                health,
                max_health,
                dead,
            } => {
                self.facts
                    .entities
                    .insert(entity.get(), Some((*health, *max_health, *dead)));
            }
            DeltaOp::Turn { active, round } => {
                self.facts.turn = Some((active.map(|a| a.get()), *round));
            }
            DeltaOp::Spawned { .. }
            | DeltaOp::Despawned { .. }
            | DeltaOp::Died { .. }
            | DeltaOp::Receipt { .. } => {}
        }
    }

    fn apply(&mut self, update: &ReplicaUpdate) {
        match update {
            ReplicaUpdate::Snapshot(p) => {
                self.facts = Facts::default();
                for e in &p.body.entities {
                    self.facts.entities.insert(
                        e.entity.get(),
                        e.health.as_ref().map(|h| (h.health, h.max_health, h.dead)),
                    );
                }
                self.facts.turn = p
                    .body
                    .turn
                    .as_ref()
                    .map(|t| (t.active.map(|a| a.get()), t.round));
                self.order.push("S".to_string());
                self.snapshots += 1;
            }
            ReplicaUpdate::Delta(GatedDelta::V2(d)) => {
                for op in &d.ops {
                    if let protocol_v2::DeltaOp::Legacy(op) = op {
                        self.apply_op(op);
                    }
                }
                self.order.push(format!("D{}", d.event_seq));
            }
            ReplicaUpdate::Delta(GatedDelta::V1(d)) => {
                for op in &d.ops {
                    self.apply_op(op);
                }
                self.order.push(format!("D{}", d.event_seq));
            }
        }
    }

    /// Pops everything queued for `rid` and applies it in pop order.
    fn drain(&mut self, pool: &mut ReassemblyPool, rid: ReceiverId) -> usize {
        let mut n = 0;
        while let Some(u) = pool.pop_update(rid) {
            self.apply(&u);
            n += 1;
        }
        n
    }
}

/// Fact-by-fact comparison; empty means the replica equals `want`.
fn diff(got: &Facts, want: &Facts) -> Vec<String> {
    let mut out = Vec::new();
    for (id, w) in &want.entities {
        match got.entities.get(id) {
            None => out.push(format!("missing entity {id}")),
            Some(g) if g != w => out.push(match (g, w) {
                (Some(_), None) => format!("leak: health of entity {id} is not permitted"),
                (None, Some(_)) => format!("missing health of entity {id}"),
                _ => format!("mismatch health of entity {id}: got {g:?} want {w:?}"),
            }),
            Some(_) => {}
        }
    }
    for id in got.entities.keys() {
        if !want.entities.contains_key(id) {
            out.push(format!("leak: entity {id} is not permitted"));
        }
    }
    match (&got.turn, &want.turn) {
        (None, Some(_)) => out.push("missing turn".to_string()),
        (Some(_), None) => out.push("leak: turn is not permitted".to_string()),
        (Some(g), Some(w)) if g != w => out.push(format!("mismatch turn: got {g:?} want {w:?}")),
        _ => {}
    }
    out
}

fn assert_matches(replica: &Replica, want: &Facts) {
    let d = diff(&replica.facts, want);
    assert!(
        d.is_empty(),
        "oracle mismatch ({} items), first: {:?}",
        d.len(),
        d.iter().take(4).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// The host stand-in: a fixture table of facts and grants.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Row {
    entity: u64,
    /// Whether the entity exists in the world at all.
    exists: bool,
    /// Presence grant: without it the entity has no field in the body.
    present_granted: bool,
    health: HealthTuple,
    /// Health grant: without it the entity is present with `None` health.
    health_granted: bool,
}

#[derive(Debug, Clone)]
struct TurnRow {
    active: Option<u64>,
    round: u64,
    /// The `turn_state` grant.
    granted: bool,
}

#[derive(Debug, Clone)]
struct Fixture {
    /// Ascending by `entity`.
    rows: Vec<Row>,
    turn: TurnRow,
}

impl Fixture {
    /// The stand-in builds the body from granted facts only (S§11 item 6).
    fn body(&self) -> SnapshotBody {
        let entities: Vec<SnapshotEntity> = self
            .rows
            .iter()
            .filter(|r| r.exists && r.present_granted)
            .map(|r| SnapshotEntity {
                entity: net(r.entity),
                health: r
                    .health_granted
                    .then(|| health(r.health.0, r.health.1, r.health.2)),
            })
            .collect();
        let turn = self.turn.granted.then(|| TurnFact {
            active: self
                .turn
                .active
                .filter(|a| entities.iter().any(|e| e.entity.get() == *a))
                .map(net),
            round: self.turn.round,
        });
        SnapshotBody { entities, turn }
    }

    fn frames(&self, id: u64, base_tick: u64, cursor: u64, spc: u8) -> Vec<Vec<u8>> {
        plan(id, base_tick, cursor, spc, &self.body())
    }
}

/// 1,024 granted rows whose maximal values make a 22,552-byte, six-chunk body.
fn big_fixture() -> Fixture {
    Fixture {
        rows: (0..1024u64)
            .map(|i| Row {
                entity: MAX_BASE + i,
                exists: true,
                present_granted: true,
                health: (u32::MAX - i as u32, u32::MAX, i % 2 == 0),
                health_granted: true,
            })
            .collect(),
        turn: TurnRow {
            active: Some(MAX_BASE),
            round: 9,
            granted: true,
        },
    }
}

/// Hand-authored facts for [`big_fixture`], with `hidden` entity indexes
/// carrying no health.
fn big_facts(hidden_health: &[u64]) -> Facts {
    let mut f = Facts::default();
    for i in 0..1024u64 {
        let h = if hidden_health.contains(&i) {
            None
        } else {
            Some((u32::MAX - i as u32, u32::MAX, i % 2 == 0))
        };
        f.entities.insert(MAX_BASE + i, h);
    }
    f.turn = Some((Some(MAX_BASE), 9));
    f
}

// ---------------------------------------------------------------------------
// Pool helpers: refusals take a whole-pool clone first and `assert_eq!` after.
// ---------------------------------------------------------------------------

fn pool_with(limits: TransferLimits) -> ReassemblyPool {
    ReassemblyPool::new(limits).expect("valid limits")
}

fn add(pool: &mut ReassemblyPool, wire: DeltaWire, mode: GateMode) -> ReceiverId {
    pool.add_receiver(EPOCH, wire, mode).expect("receiver")
}

/// A v1-limits pool with one V2, `Live` receiver.
fn mkpool() -> (ReassemblyPool, ReceiverId) {
    let mut pool = pool_with(TransferLimits::v1());
    let rid = add(&mut pool, DeltaWire::V2, GateMode::Live);
    (pool, rid)
}

/// Runs the pool-wide sweep at `now` and checks that nothing expired, so that
/// a following refusal is compared at an unchanged clock (S§10.5).
fn settle(pool: &mut ReassemblyPool, now: u64) {
    assert!(pool.poll(now).is_empty(), "unexpected expiry at {now}");
}

fn ok_t(pool: &mut ReassemblyPool, rid: ReceiverId, bytes: &[u8], now: u64) -> TransferOutcome {
    match pool.offer_transfer(rid, bytes, now) {
        Ok(o) => o,
        Err(e) => panic!("offer_transfer refused: {e:?}"),
    }
}

fn ok_d(pool: &mut ReassemblyPool, rid: ReceiverId, bytes: &[u8], now: u64) -> DeltaOutcome {
    match pool.offer_delta(rid, bytes, now) {
        Ok(o) => o,
        Err(e) => panic!("offer_delta refused: {e:?}"),
    }
}

/// A refusal: the call fails and the whole pool is unchanged.
fn refuse_t(pool: &mut ReassemblyPool, rid: ReceiverId, bytes: &[u8], now: u64) -> TransferError {
    settle(pool, now);
    let before = pool.clone();
    let err = pool
        .offer_transfer(rid, bytes, now)
        .expect_err("expected a refusal");
    assert_eq!(*pool, before, "a refusal ({err:?}) changed the pool");
    err
}

fn refuse_d(pool: &mut ReassemblyPool, rid: ReceiverId, bytes: &[u8], now: u64) -> TransferError {
    settle(pool, now);
    let before = pool.clone();
    let err = pool
        .offer_delta(rid, bytes, now)
        .expect_err("expected a refusal");
    assert_eq!(*pool, before, "a refusal ({err:?}) changed the pool");
    err
}

/// A failure (S§10.4): returns the reason; the caller checks the effects.
fn fail_t(pool: &mut ReassemblyPool, rid: ReceiverId, bytes: &[u8], now: u64) -> FailReason {
    match pool.offer_transfer(rid, bytes, now) {
        Err(TransferError::Failed(r)) => r,
        other => panic!("expected Failed(_), got {other:?}"),
    }
}

fn fail_d(pool: &mut ReassemblyPool, rid: ReceiverId, bytes: &[u8], now: u64) -> FailReason {
    match pool.offer_delta(rid, bytes, now) {
        Err(TransferError::Failed(r)) => r,
        other => panic!("expected Failed(_), got {other:?}"),
    }
}

/// Offers every frame at `now`; returns the last outcome.
fn offer_all(
    pool: &mut ReassemblyPool,
    rid: ReceiverId,
    frames: &[Vec<u8>],
    now: u64,
) -> TransferOutcome {
    let mut last = None;
    for f in frames {
        last = Some(ok_t(pool, rid, f, now));
    }
    last.expect("frames")
}

fn assert_failed_clean(pool: &ReassemblyPool, rid: ReceiverId, retired: u64, resync: ResyncReason) {
    let st = pool.status(rid).unwrap();
    assert!(st.in_flight.is_none());
    assert_eq!(st.retired_through, retired);
    assert_eq!(st.resync, Some(resync));
    assert_eq!(st.mode, GateMode::AwaitSnapshot);
    assert_eq!(pool.staging_bytes(), 0);
}

// ---------------------------------------------------------------------------
// Delta helpers (V2 and V1 frames of Legacy state ops).
// ---------------------------------------------------------------------------

fn enter(id: u64) -> DeltaOp {
    DeltaOp::EntityEnter { entity: net(id) }
}

fn leave(id: u64) -> DeltaOp {
    DeltaOp::EntityLeave { entity: net(id) }
}

fn health_op(id: u64, h: u32, m: u32, dead: bool) -> DeltaOp {
    DeltaOp::Health {
        entity: net(id),
        health: h,
        max_health: m,
        dead,
    }
}

fn turn_op(active: Option<u64>, round: u64) -> DeltaOp {
    DeltaOp::Turn {
        active: active.map(net),
        round,
    }
}

fn v2_frame(seq: u64, ops: Vec<DeltaOp>) -> Vec<u8> {
    codec_v2::encode_delta(&protocol_v2::DeltaFrame {
        lane: protocol::LANE_COMBAT,
        server_tick: 1,
        event_seq: seq,
        ops: ops.into_iter().map(protocol_v2::DeltaOp::Legacy).collect(),
    })
    .expect("v2 delta encodes")
}

fn v1_frame(seq: u64, ops: Vec<DeltaOp>) -> Vec<u8> {
    codec::encode_delta(&protocol::DeltaFrame {
        lane: protocol::LANE_COMBAT,
        server_tick: 1,
        event_seq: seq,
        ops,
    })
    .expect("v1 delta encodes")
}

/// A V2 delta of exactly `len` encoded bytes (header plus filler ops).
fn sized_frame(seq: u64, len: usize) -> Vec<u8> {
    for k in 0..=256usize {
        for j in 0..=1usize.min(k) {
            // `j` three-byte ops (id 128) pad by one byte each; the rest are
            // two-byte ops.
            let ops: Vec<DeltaOp> = (0..k)
                .map(|i| if i < j { enter(128) } else { enter(1) })
                .collect();
            let f = v2_frame(seq, ops);
            if f.len() == len {
                return f;
            }
        }
    }
    panic!("no frame of {len} bytes");
}

/// Frames for `first_seq..` whose encoded lengths sum to exactly `total`.
fn frames_summing(first_seq: u64, total: usize) -> Vec<Vec<u8>> {
    const BIG: usize = 518; // 256 two-byte ops plus a six-byte header
    const MIN: usize = 5; // an empty frame
    let mut out = Vec::new();
    let mut remaining = total;
    let mut seq = first_seq;
    while remaining > 0 {
        let take = if remaining <= BIG {
            remaining
        } else if remaining - BIG < MIN {
            remaining - MIN
        } else {
            BIG
        };
        out.push(sized_frame(seq, take));
        remaining -= take;
        seq += 1;
    }
    assert_eq!(out.iter().map(Vec::len).sum::<usize>(), total);
    out
}

// ===========================================================================
// 1. wire_identifiers_and_caps_are_pinned
// ===========================================================================

#[test]
fn wire_identifiers_and_caps_are_pinned() {
    assert_eq!(TRANSFER_VERSION, 0x81);
    assert_eq!(LANE_SNAPSHOT, 2);
    assert_eq!(TRANSFER_TAG_BEGIN, 0x10);
    assert_eq!(TRANSFER_TAG_CHUNK, 0x11);
    assert_eq!(TRANSFER_TAG_CANCEL, 0x12);
    assert_eq!(TRANSFER_TAG_PUBLISHED, 0x20);
    assert_eq!(TRANSFER_TAG_RESYNC, 0x21);

    // 0x81 is neither protocol version; the lane is not the combat lane.
    assert_eq!(protocol::PROTOCOL_VERSION, 1);
    assert_eq!(protocol_v2::PROTOCOL_VERSION, 2);
    assert!(
        ![protocol::PROTOCOL_VERSION, protocol_v2::PROTOCOL_VERSION].contains(&TRANSFER_VERSION)
    );
    assert_ne!(LANE_SNAPSHOT, protocol::LANE_COMBAT);

    // Transfer tags are disjoint from intent tags {0, 1} and delta tags 0..=10.
    let intent_tags = [
        protocol::INTENT_TAG_DECLARE_ACTION,
        protocol::INTENT_TAG_END_TURN,
    ];
    assert_eq!(intent_tags, [0, 1]);
    let delta_tags = [
        protocol::DELTA_TAG_ENTITY_ENTER,
        protocol::DELTA_TAG_ENTITY_LEAVE,
        protocol::DELTA_TAG_SPAWNED,
        protocol::DELTA_TAG_DESPAWNED,
        protocol::DELTA_TAG_DIED,
        protocol::DELTA_TAG_HEALTH,
        protocol::DELTA_TAG_TURN,
        protocol::DELTA_TAG_RECEIPT,
        protocol_v2::DELTA_TAG_ACTION_RESOLVED,
        protocol_v2::DELTA_TAG_TURN_STARTED,
        protocol_v2::DELTA_TAG_ENCOUNTER_ENDED,
    ];
    assert_eq!(delta_tags.to_vec(), (0..=10u8).collect::<Vec<_>>());
    let transfer_tags = [
        TRANSFER_TAG_BEGIN,
        TRANSFER_TAG_CHUNK,
        TRANSFER_TAG_CANCEL,
        TRANSFER_TAG_PUBLISHED,
        TRANSFER_TAG_RESYNC,
    ];
    for t in transfer_tags {
        assert!(!intent_tags.contains(&t), "tag {t:#x} collides with intent");
        assert!(!delta_tags.contains(&t), "tag {t:#x} collides with delta");
    }
    let mut sorted = transfer_tags;
    sorted.sort_unstable();
    for pair in sorted.windows(2) {
        assert_ne!(pair[0], pair[1], "transfer tags are distinct");
    }

    // S§7 constants.
    assert_eq!(MAX_SEGMENT_BYTES, 4096);
    assert_eq!(MAX_SEGMENT_BYTES, protocol::MAX_WIRE_BYTES_FIELD);
    assert_eq!(MAX_SEGMENTS_PER_CHUNK, 8u8);
    assert_eq!(MAX_CHUNK_BYTES, 32_768);
    assert_eq!(
        MAX_SEGMENT_BYTES * usize::from(MAX_SEGMENTS_PER_CHUNK),
        MAX_CHUNK_BYTES
    );
    assert_eq!(MAX_CHUNK_BYTES * 32, 1_048_576);
    assert_eq!(
        MAX_CHUNK_BYTES * protocol::MAX_SNAPSHOT_CHUNKS,
        protocol::MAX_SNAPSHOT_BYTES
    );
    assert_eq!(MAX_DOWN_FRAME_BYTES, 65_536);
    assert_eq!(MAX_DOWN_FRAME_BYTES, protocol::MAX_DELTA_FRAME_BYTES);
    assert_eq!(MAX_UP_FRAME_BYTES, 4096);
    assert_eq!(MAX_UP_FRAME_BYTES, protocol::MAX_INTENT_FRAME_BYTES);
    assert_eq!(MAX_ENCODED_BODY_BYTES, 22_552);
    assert_eq!(MAX_ENCODED_BODY_BYTES, 2 + 1024 * 22 + 22);
    assert_eq!(POLICY_SNAPSHOT_STAGING_BYTES_PER_PEER, 1_048_576);
    assert_eq!(
        POLICY_SNAPSHOT_STAGING_BYTES_PER_PEER,
        protocol::MAX_SNAPSHOT_BYTES
    );
    assert_eq!(POLICY_SNAPSHOT_STAGING_BYTES_HOST, 8 * 1_048_576);
    assert_eq!(POLICY_SNAPSHOT_STAGING_BYTES_HOST, 8_388_608);

    // The v1 constants are unchanged.
    assert_eq!(protocol::MAX_SNAPSHOT_BYTES, 1_048_576);
    assert_eq!(protocol::MAX_SNAPSHOT_CHUNKS, 32);
    assert_eq!(protocol::MAX_VISIBLE_ENTITIES, 1024);
    assert_eq!(protocol::MAX_WIRE_BYTES_FIELD, 4096);
    assert_eq!(protocol::MAX_DELTA_FRAME_BYTES, 65_536);
    assert_eq!(protocol::MAX_INTENT_FRAME_BYTES, 4096);
    assert_eq!(protocol::SEQ_FIRST, 1);
    assert_eq!(protocol::SEQ_LAST, u64::MAX - 1);
    assert_eq!(protocol::POLICY_SNAPSHOT_IN_FLIGHT_PER_PEER, 1);
    assert_eq!(protocol::POLICY_SNAPSHOT_REASSEMBLY_SECS, 5);
    assert_eq!(protocol::POLICY_PROOF_PEERS, 8);
    assert_eq!(protocol::POLICY_EGRESS_FRAMES_PER_PEER, 128);
    assert_eq!(protocol::POLICY_EGRESS_BYTES_PER_PEER, 2_097_152);
    assert_eq!(protocol::POLICY_EGRESS_BYTES_HOST, 16_777_216);

    // `TransferLimits::v1()` field by field, tied to the frozen policy.
    let v1 = TransferLimits::v1();
    assert_eq!(v1.max_receivers, 8);
    assert_eq!(v1.max_receivers, protocol::POLICY_PROOF_PEERS);
    assert_eq!(v1.deadline_ms, 5_000);
    assert_eq!(
        v1.deadline_ms,
        protocol::POLICY_SNAPSHOT_REASSEMBLY_SECS * 1000
    );
    assert_eq!(v1.staging_bytes_per_receiver, 1_048_576);
    assert_eq!(v1.staging_bytes_total, 8_388_608);
    assert_eq!(v1.delta_frames_per_receiver, 128);
    assert_eq!(
        v1.delta_frames_per_receiver,
        protocol::POLICY_EGRESS_FRAMES_PER_PEER
    );
    assert_eq!(v1.delta_bytes_per_receiver, 2_097_152);
    assert_eq!(
        v1.delta_bytes_per_receiver,
        protocol::POLICY_EGRESS_BYTES_PER_PEER
    );
    assert_eq!(v1.delta_bytes_total, 16_777_216);
    assert_eq!(v1.delta_bytes_total, protocol::POLICY_EGRESS_BYTES_HOST);
    assert_eq!(v1.validate(), Ok(()));

    // Identifiers and the closed reason enums.
    assert_eq!(TransferId::new(0), None);
    assert_eq!(TransferId::new(1).map(TransferId::get), Some(1));
    assert_eq!(
        TransferId::new(u64::MAX).map(TransferId::get),
        Some(u64::MAX)
    );
    assert!(tid(1) < tid(2));
    assert_eq!(CancelReason::Superseded.as_u8(), 1);
    assert_eq!(CancelReason::Aborted.as_u8(), 2);
    assert_eq!(CancelReason::from_u8(0), None);
    assert_eq!(CancelReason::from_u8(1), Some(CancelReason::Superseded));
    assert_eq!(CancelReason::from_u8(2), Some(CancelReason::Aborted));
    assert_eq!(CancelReason::from_u8(3), None);
    assert_eq!(ResyncReason::Expired.as_u8(), 1);
    assert_eq!(ResyncReason::Invalid.as_u8(), 2);
    assert_eq!(ResyncReason::Overflow.as_u8(), 3);
    assert_eq!(ResyncReason::from_u8(0), None);
    assert_eq!(ResyncReason::from_u8(1), Some(ResyncReason::Expired));
    assert_eq!(ResyncReason::from_u8(2), Some(ResyncReason::Invalid));
    assert_eq!(ResyncReason::from_u8(3), Some(ResyncReason::Overflow));
    assert_eq!(ResyncReason::from_u8(4), None);
    assert_eq!(FailReason::Expired.resync_reason(), ResyncReason::Expired);
    assert_eq!(FailReason::Conflict.resync_reason(), ResyncReason::Invalid);
    assert_eq!(
        FailReason::InvalidBody(CodecError::Malformed).resync_reason(),
        ResyncReason::Invalid
    );
    assert_eq!(FailReason::Overflow.resync_reason(), ResyncReason::Overflow);
}

// ===========================================================================
// 2. literal_byte_oracles
// ===========================================================================

#[test]
fn literal_byte_oracles() {
    // Minimal Begin (26 bytes).
    let begin = TransferDown::Begin(BeginTransfer {
        epoch: ZERO_EPOCH,
        transfer: tid(1),
        base_tick: 0,
        cursor: 0,
        total_bytes: 2,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    });
    let mut want = vec![0x81, 0x02, 0x10];
    want.extend([0u8; 16]);
    want.extend([0x01, 0x00, 0x00, 0x02, 0x00, 0x01, 0x01]);
    assert_eq!(want.len(), 26);
    let got = encode_down(&begin).unwrap();
    assert_eq!(got, want);
    assert_eq!(decode_down(&want), Ok(begin));

    // Its single Chunk, the empty body `00 00` (25 bytes).
    let chunk = TransferDown::Chunk(TransferChunk {
        epoch: ZERO_EPOCH,
        transfer: tid(1),
        index: 0,
        bytes: vec![0x00, 0x00],
    });
    let mut want = vec![0x81, 0x02, 0x11];
    want.extend([0u8; 16]);
    want.extend([0x01, 0x00, 0x01, 0x02, 0x00, 0x00]);
    assert_eq!(want.len(), 25);
    assert_eq!(encode_down(&chunk).unwrap(), want);
    assert_eq!(decode_down(&want), Ok(chunk));

    // Cancel (id 1, Superseded), 21 bytes.
    let cancel = TransferDown::Cancel {
        epoch: ZERO_EPOCH,
        transfer: tid(1),
        reason: CancelReason::Superseded,
    };
    let mut want = vec![0x81, 0x02, 0x12];
    want.extend([0u8; 16]);
    want.extend([0x01, 0x01]);
    assert_eq!(want.len(), 21);
    assert_eq!(encode_down(&cancel).unwrap(), want);
    assert_eq!(decode_down(&want), Ok(cancel));
    // The other reason is byte 2.
    let aborted = TransferDown::Cancel {
        epoch: ZERO_EPOCH,
        transfer: tid(1),
        reason: CancelReason::Aborted,
    };
    want[20] = 0x02;
    assert_eq!(encode_down(&aborted).unwrap(), want);
    assert_eq!(decode_down(&want), Ok(aborted));

    // Published (id 1), 20 bytes.
    let published = TransferUp::Published {
        epoch: ZERO_EPOCH,
        transfer: tid(1),
    };
    let mut want = vec![0x81, 0x02, 0x20];
    want.extend([0u8; 16]);
    want.push(0x01);
    assert_eq!(want.len(), 20);
    assert_eq!(encode_up(&published).unwrap(), want);
    assert_eq!(decode_up(&want), Ok(published));

    // Resync (none seen, Expired), 21 bytes.
    let resync = TransferUp::Resync {
        epoch: ZERO_EPOCH,
        after: None,
        reason: ResyncReason::Expired,
    };
    let mut want = vec![0x81, 0x02, 0x21];
    want.extend([0u8; 16]);
    want.extend([0x00, 0x01]);
    assert_eq!(want.len(), 21);
    assert_eq!(encode_up(&resync).unwrap(), want);
    assert_eq!(decode_up(&want), Ok(resync));
    // `after` Some(3) with Overflow.
    let resync = TransferUp::Resync {
        epoch: ZERO_EPOCH,
        after: Some(tid(3)),
        reason: ResyncReason::Overflow,
    };
    let mut want = vec![0x81, 0x02, 0x21];
    want.extend([0u8; 16]);
    want.extend([0x03, 0x03]);
    assert_eq!(encode_up(&resync).unwrap(), want);
    assert_eq!(decode_up(&want), Ok(resync));

    // Body: entity 1 at health 10/10 alive and no turn (7 bytes).
    let one = SnapshotEntity {
        entity: net(1),
        health: Some(health(10, 10, false)),
    };
    let body = SnapshotBody {
        entities: vec![one.clone()],
        turn: None,
    };
    let want = vec![0x01, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x00];
    assert_eq!(encode_body(&body).unwrap(), want);
    assert_eq!(decode_body(&want), Ok(body));
    // With turn (active 1, round 3) the last byte becomes `01 01 01 03`.
    let body = SnapshotBody {
        entities: vec![one],
        turn: Some(TurnFact {
            active: Some(net(1)),
            round: 3,
        }),
    };
    let want = vec![0x01, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x01, 0x01, 0x01, 0x03];
    assert_eq!(encode_body(&body).unwrap(), want);
    assert_eq!(decode_body(&want), Ok(body));
    // Empty body is `00 00`.
    let empty = SnapshotBody {
        entities: vec![],
        turn: None,
    };
    assert_eq!(encode_body(&empty).unwrap(), vec![0x00, 0x00]);

    // A 4,097-byte chunk is two segments: 4,096 then 1.
    let payload: Vec<u8> = (0..4097usize).map(|i| (i % 251) as u8).collect();
    let chunk = TransferDown::Chunk(TransferChunk {
        epoch: ZERO_EPOCH,
        transfer: tid(1),
        index: 0,
        bytes: payload.clone(),
    });
    let mut want = vec![0x81, 0x02, 0x11];
    want.extend([0u8; 16]);
    want.extend([0x01, 0x00, 0x02]); // transfer 1, index 0, two segments
    want.extend([0x80, 0x20]); // varint 4,096
    want.extend(&payload[..4096]);
    want.push(0x01); // varint 1
    want.push(payload[4096]);
    assert_eq!(want.len(), 3 + 16 + 3 + 2 + 4096 + 1 + 1);
    assert_eq!(encode_down(&chunk).unwrap(), want);
    assert_eq!(decode_down(&want), Ok(chunk));
}

// ===========================================================================
// 3. max_frames_fit_the_frame_ceilings
// ===========================================================================

#[test]
fn max_frames_fit_the_frame_ceilings() {
    let begin = BeginTransfer {
        epoch: [0xFF; 16],
        transfer: tid(u64::MAX),
        base_tick: u64::MAX,
        cursor: SEQ_LAST,
        total_bytes: 1_048_576,
        entity_count: 1024,
        segments_per_chunk: 8,
        chunk_count: 32,
    };
    let frame = encode_down(&TransferDown::Begin(begin.clone())).unwrap();
    assert_eq!(frame.len(), 56);
    assert!(frame.len() <= MAX_DOWN_FRAME_BYTES);
    assert_eq!(decode_down(&frame), Ok(TransferDown::Begin(begin)));

    let full = TransferChunk {
        epoch: [0xFF; 16],
        transfer: tid(u64::MAX),
        index: 31,
        bytes: vec![0xA5; 32_768],
    };
    let frame = encode_down(&TransferDown::Chunk(full.clone())).unwrap();
    assert_eq!(frame.len(), 32_815);
    assert!(frame.len() <= 65_536);
    assert_eq!(decode_down(&frame), Ok(TransferDown::Chunk(full)));

    let cancel = TransferDown::Cancel {
        epoch: [0xFF; 16],
        transfer: tid(u64::MAX),
        reason: CancelReason::Aborted,
    };
    let frame = encode_down(&cancel).unwrap();
    assert_eq!(frame.len(), 30);
    assert_eq!(decode_down(&frame), Ok(cancel));

    let published = TransferUp::Published {
        epoch: [0xFF; 16],
        transfer: tid(u64::MAX),
    };
    let frame = encode_up(&published).unwrap();
    assert_eq!(frame.len(), 29);
    assert!(frame.len() <= MAX_UP_FRAME_BYTES);
    assert_eq!(decode_up(&frame), Ok(published));

    let resync = TransferUp::Resync {
        epoch: [0xFF; 16],
        after: Some(tid(u64::MAX)),
        reason: ResyncReason::Overflow,
    };
    let frame = encode_up(&resync).unwrap();
    assert_eq!(frame.len(), 30);
    assert!(frame.len() <= MAX_UP_FRAME_BYTES);
    assert_eq!(decode_up(&frame), Ok(resync));

    // Chunk payload bounds, max-valid then first-invalid.
    let chunk = |index: u8, len: usize| {
        encode_down(&TransferDown::Chunk(TransferChunk {
            epoch: ZERO_EPOCH,
            transfer: tid(1),
            index,
            bytes: vec![7; len],
        }))
    };
    assert!(chunk(0, 32_768).is_ok());
    assert_eq!(chunk(0, 32_769), Err(CodecError::LimitExceeded));
    assert_eq!(chunk(0, 0), Err(CodecError::Malformed));
    // Order on encode: index, then empty, then too long.
    assert!(chunk(31, 1).is_ok());
    assert_eq!(chunk(32, 1), Err(CodecError::LimitExceeded));
    assert_eq!(chunk(32, 0), Err(CodecError::LimitExceeded));
    assert_eq!(chunk(32, 32_769), Err(CodecError::LimitExceeded));
}

// ===========================================================================
// 4. down_decode_precedence
// ===========================================================================

#[test]
fn down_decode_precedence() {
    let begin_tail: Vec<u8> = {
        let mut t = vec![0u8; 16];
        t.extend([0x01, 0x00, 0x00, 0x02, 0x00, 0x01, 0x01]);
        t
    };
    let frame = |v: u8, l: u8, t: u8, rest: &[u8]| {
        let mut b = header(v, l, t);
        b.extend(rest);
        b
    };
    // Positive control.
    assert!(matches!(
        decode_down(&frame(0x81, 2, 0x10, &begin_tail)),
        Ok(TransferDown::Begin(_))
    ));

    // The whole-frame cap comes first, before any read.
    let mut big = vec![0x01u8; 65_537];
    assert_eq!(decode_down(&big), Err(CodecError::FrameTooLarge));
    big[0] = 0x81; // even a valid header
    assert_eq!(decode_down(&big), Err(CodecError::FrameTooLarge));
    // 65,536 passes the cap, so the bad version is then reported.
    let at_cap = vec![0x01u8; 65_536];
    assert_eq!(decode_down(&at_cap), Err(CodecError::UnsupportedVersion));

    // Empty input and truncation before the tag are Malformed.
    assert_eq!(decode_down(&[]), Err(CodecError::Malformed));
    assert_eq!(decode_down(&[0x81]), Err(CodecError::Malformed));
    assert_eq!(decode_down(&[0x81, 2]), Err(CodecError::Malformed));

    // Versions 1, 2, 0x80, 0x82 (and others) are UnsupportedVersion.
    for v in [0u8, 1, 2, 0x80, 0x82, 0xFF] {
        assert_eq!(
            decode_down(&frame(v, 2, 0x10, &begin_tail)),
            Err(CodecError::UnsupportedVersion),
            "version {v:#x}"
        );
    }
    // Lanes 0, 1, 3 are WrongDirection.
    for lane in [0u8, 1, 3, 0xFF] {
        assert_eq!(
            decode_down(&frame(0x81, lane, 0x10, &begin_tail)),
            Err(CodecError::WrongDirection),
            "lane {lane}"
        );
    }
    // The other direction's tags are WrongDirection, body or not.
    for tag in [0x20u8, 0x21] {
        assert_eq!(
            decode_down(&frame(0x81, 2, tag, &[0u8; 16])),
            Err(CodecError::WrongDirection)
        );
        assert_eq!(
            decode_down(&frame(0x81, 2, tag, &[])),
            Err(CodecError::WrongDirection)
        );
    }
    // Any other tag is UnknownMessage.
    for tag in [0x00u8, 0x08, 0x13, 0x22, 0xFF] {
        assert_eq!(
            decode_down(&frame(0x81, 2, tag, &begin_tail)),
            Err(CodecError::UnknownMessage),
            "tag {tag:#x}"
        );
    }
    // Cancel reasons 0 and 3 are UnknownMessage; 1 and 2 decode.
    let cancel = |reason: u8, extra: &[u8]| {
        let mut t = vec![0u8; 16];
        t.push(0x01);
        t.push(reason);
        t.extend(extra);
        frame(0x81, 2, 0x12, &t)
    };
    for reason in [0u8, 3, 0xFF] {
        assert_eq!(
            decode_down(&cancel(reason, &[])),
            Err(CodecError::UnknownMessage),
            "reason {reason}"
        );
    }
    for reason in [1u8, 2] {
        assert!(matches!(
            decode_down(&cancel(reason, &[])),
            Ok(TransferDown::Cancel { .. })
        ));
    }

    // The hand-built twin of the S§5.5 Cancel decodes to the same message.
    assert_eq!(
        decode_down(&cancel_wire(1, 2)),
        Ok(TransferDown::Cancel {
            epoch: ZERO_EPOCH,
            transfer: tid(1),
            reason: CancelReason::Aborted
        })
    );

    // Combinations report the earliest stage.
    let all_bad = frame(0x01, 9, 0xFF, &[]);
    assert_eq!(decode_down(&all_bad), Err(CodecError::UnsupportedVersion));
    let lane_and_tag = frame(0x81, 9, 0xFF, &[]);
    assert_eq!(decode_down(&lane_and_tag), Err(CodecError::WrongDirection));
    // Unknown tag with a truncated body is UnknownMessage, not Malformed.
    assert_eq!(
        decode_down(&frame(0x81, 2, 0x13, &[0, 1])),
        Err(CodecError::UnknownMessage)
    );
    // Cancel: reason 3 plus a trailing byte is UnknownMessage (field before
    // trailing bytes); a valid reason plus a trailing byte is Malformed.
    assert_eq!(
        decode_down(&cancel(3, &[0x00])),
        Err(CodecError::UnknownMessage)
    );
    assert_eq!(decode_down(&cancel(1, &[0x00])), Err(CodecError::Malformed));
    // Cancel: transfer 0 with a bad reason is Malformed (transfer is read first).
    let mut t = vec![0u8; 16];
    t.extend([0x00, 0x03]);
    assert_eq!(
        decode_down(&frame(0x81, 2, 0x12, &t)),
        Err(CodecError::Malformed)
    );
}

// ===========================================================================
// 5. up_decode_precedence
// ===========================================================================

#[test]
fn up_decode_precedence() {
    // Cap: 4,097 is FrameTooLarge even with a bad version; 4,096 is not.
    let mut big = vec![0x01u8; 4_097];
    assert_eq!(decode_up(&big), Err(CodecError::FrameTooLarge));
    big[0] = 0x81;
    assert_eq!(decode_up(&big), Err(CodecError::FrameTooLarge));
    assert_eq!(
        decode_up(&vec![0x01u8; 4_096]),
        Err(CodecError::UnsupportedVersion)
    );
    // A frame legal downstream but over 4,096 B is refused upstream.
    assert_eq!(decode_up(&vec![0u8; 5_000]), Err(CodecError::FrameTooLarge));

    assert_eq!(decode_up(&[]), Err(CodecError::Malformed));
    assert_eq!(decode_up(&[0x81]), Err(CodecError::Malformed));
    assert_eq!(decode_up(&[0x81, 2]), Err(CodecError::Malformed));

    // Positive controls.
    assert!(matches!(
        decode_up(&published_wire(1)),
        Ok(TransferUp::Published { .. })
    ));
    assert!(matches!(
        decode_up(&resync_wire(0, 1)),
        Ok(TransferUp::Resync { after: None, .. })
    ));
    assert_eq!(
        decode_up(&resync_wire(7, 3)),
        Ok(TransferUp::Resync {
            epoch: ZERO_EPOCH,
            after: Some(tid(7)),
            reason: ResyncReason::Overflow
        })
    );

    // Version, then lane.
    for v in [0u8, 1, 2, 0x80, 0x82] {
        let mut b = published_wire(1);
        b[0] = v;
        assert_eq!(decode_up(&b), Err(CodecError::UnsupportedVersion));
    }
    for lane in [0u8, 1, 3] {
        let mut b = published_wire(1);
        b[1] = lane;
        assert_eq!(decode_up(&b), Err(CodecError::WrongDirection));
    }
    // Downstream tags 0x10..=0x12 are WrongDirection.
    for tag in 0x10u8..=0x12 {
        let mut b = transfer_header(tag);
        b.extend([0u8; 16]);
        b.push(0x01);
        assert_eq!(
            decode_up(&b),
            Err(CodecError::WrongDirection),
            "tag {tag:#x}"
        );
        assert_eq!(
            decode_up(&transfer_header(tag)),
            Err(CodecError::WrongDirection)
        );
    }
    // Other tags are UnknownMessage.
    for tag in [0x00u8, 0x0F, 0x13, 0x22, 0xFF] {
        let mut b = transfer_header(tag);
        b.extend([0u8; 16]);
        b.push(0x01);
        assert_eq!(
            decode_up(&b),
            Err(CodecError::UnknownMessage),
            "tag {tag:#x}"
        );
    }
    // Resync reasons 0 and 4 are UnknownMessage; 1, 2, 3 decode.
    for reason in [0u8, 4, 0xFF] {
        assert_eq!(
            decode_up(&resync_wire(0, reason)),
            Err(CodecError::UnknownMessage),
            "reason {reason}"
        );
    }
    for reason in [1u8, 2, 3] {
        assert!(decode_up(&resync_wire(0, reason)).is_ok());
    }
    // Trailing bytes are Malformed (after an otherwise valid frame).
    let mut b = published_wire(1);
    b.push(0x00);
    assert_eq!(decode_up(&b), Err(CodecError::Malformed));
    let mut b = resync_wire(2, 1);
    b.push(0x00);
    assert_eq!(decode_up(&b), Err(CodecError::Malformed));
    // A bad reason beats trailing bytes; a zero transfer beats a bad reason
    // in field order (Published has no reason, so check Resync order only).
    let mut b = resync_wire(0, 4);
    b.push(0x00);
    assert_eq!(decode_up(&b), Err(CodecError::UnknownMessage));
    // Published transfer 0 is Malformed; an overflowing varint is Malformed.
    assert_eq!(decode_up(&published_wire(0)), Err(CodecError::Malformed));
    let mut b = transfer_header(0x20);
    b.extend(ZERO_EPOCH);
    b.extend(overflow_varint());
    assert_eq!(decode_up(&b), Err(CodecError::Malformed));
    // Truncated epoch is Malformed.
    let mut b = transfer_header(0x20);
    b.extend([0u8; 15]);
    assert_eq!(decode_up(&b), Err(CodecError::Malformed));
}

// ===========================================================================
// 6. begin_bounds
// ===========================================================================

/// The S§5.1/S§6 field order, written independently of the codec.
fn ref_begin_err(
    transfer: u64,
    cursor: u64,
    total: u32,
    entities: u16,
    spc: u8,
    chunks: u8,
) -> Option<CodecError> {
    if transfer == 0 || cursor > SEQ_LAST || total == 0 {
        return Some(CodecError::Malformed);
    }
    if total > 1_048_576 || entities > 1024 {
        return Some(CodecError::LimitExceeded);
    }
    if spc == 0 {
        return Some(CodecError::Malformed);
    }
    if spc > 8 {
        return Some(CodecError::LimitExceeded);
    }
    if chunks == 0 {
        return Some(CodecError::Malformed);
    }
    if chunks > 32 {
        return Some(CodecError::LimitExceeded);
    }
    if total.div_ceil(u32::from(spc) * 4096) != u32::from(chunks) {
        return Some(CodecError::Malformed);
    }
    None
}

fn good_begin() -> BeginTransfer {
    BeginTransfer {
        epoch: ZERO_EPOCH,
        transfer: tid(1),
        base_tick: 0,
        cursor: 0,
        total_bytes: 2,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    }
}

#[test]
fn begin_bounds() {
    let good = good_begin();
    let one = varint(1);
    let two = varint(2);
    let decode_begin = |transfer: &[u8], cursor: &[u8], total: &[u8], ents: &[u8], spc, chunks| {
        decode_down(&begin_wire(
            ZERO_EPOCH, transfer, 0, cursor, total, ents, spc, chunks,
        ))
    };
    // Positive control, byte-identical to the S§5.5 example.
    assert_eq!(good.validate(), Ok(()));
    assert_eq!(
        decode_begin(&one, &varint(0), &two, &varint(0), 1, 1),
        Ok(TransferDown::Begin(good.clone()))
    );

    // transfer 1 / 0.
    assert_eq!(
        decode_begin(&varint(0), &varint(0), &two, &varint(0), 1, 1),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_begin(&overflow_varint(), &varint(0), &two, &varint(0), 1, 1),
        Err(CodecError::Malformed)
    );
    // cursor SEQ_LAST / u64::MAX.
    let mut c = good.clone();
    c.cursor = SEQ_LAST;
    assert_eq!(c.validate(), Ok(()));
    assert_eq!(
        decode_begin(&one, &varint(SEQ_LAST), &two, &varint(0), 1, 1),
        Ok(TransferDown::Begin(c.clone()))
    );
    assert_eq!(
        encode_down(&TransferDown::Begin(c)).map(|b| b.len()),
        Ok(3 + 16 + 1 + 1 + 9 + 1 + 1 + 1 + 1 + 1)
    );
    c = good.clone();
    c.cursor = u64::MAX;
    assert_eq!(c.validate(), Err(CodecError::Malformed));
    assert_eq!(
        decode_begin(&one, &varint(u64::MAX), &two, &varint(0), 1, 1),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_begin(&one, &overflow_varint(), &two, &varint(0), 1, 1),
        Err(CodecError::Malformed)
    );

    // total_bytes 1,048,576 (spc 8, 32 chunks) / 1,048,577 / 0.
    let mut t = good.clone();
    t.total_bytes = 1_048_576;
    t.segments_per_chunk = 8;
    t.chunk_count = 32;
    assert_eq!(t.validate(), Ok(()));
    assert!(TransferAssembly::new(&t).is_ok());
    assert_eq!(
        decode_begin(&one, &varint(0), &varint(1_048_576), &varint(0), 8, 32),
        Ok(TransferDown::Begin(t.clone()))
    );
    t.total_bytes = 1_048_577;
    assert_eq!(t.validate(), Err(CodecError::LimitExceeded));
    assert_eq!(
        decode_begin(&one, &varint(0), &varint(1_048_577), &varint(0), 8, 32),
        Err(CodecError::LimitExceeded)
    );
    t.total_bytes = 0;
    assert_eq!(t.validate(), Err(CodecError::Malformed));
    assert_eq!(
        decode_begin(&one, &varint(0), &varint(0), &varint(0), 8, 32),
        Err(CodecError::Malformed)
    );
    // Hostile varints: u64::MAX and a 2^32 wrap are LimitExceeded; overflow
    // is Malformed.
    assert_eq!(
        decode_begin(&one, &varint(0), &varint(u64::MAX), &varint(0), 1, 1),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        decode_begin(&one, &varint(0), &varint((1 << 32) | 2), &varint(0), 1, 1),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        decode_begin(&one, &varint(0), &overflow_varint(), &varint(0), 1, 1),
        Err(CodecError::Malformed)
    );

    // entity_count 1,024 / 1,025.
    let mut e = good.clone();
    e.entity_count = 1024;
    assert_eq!(e.validate(), Ok(()));
    assert_eq!(
        decode_begin(&one, &varint(0), &two, &varint(1024), 1, 1),
        Ok(TransferDown::Begin(e.clone()))
    );
    e.entity_count = 1025;
    assert_eq!(e.validate(), Err(CodecError::LimitExceeded));
    assert_eq!(
        decode_begin(&one, &varint(0), &two, &varint(1025), 1, 1),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        decode_begin(&one, &varint(0), &two, &varint(u64::MAX), 1, 1),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        decode_begin(&one, &varint(0), &two, &varint((1 << 16) | 3), 1, 1),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        decode_begin(&one, &varint(0), &two, &overflow_varint(), 1, 1),
        Err(CodecError::Malformed)
    );

    // segments_per_chunk 8 / 9 and 0.
    let mut s = good.clone();
    s.total_bytes = 8 * 4096;
    s.segments_per_chunk = 8;
    assert_eq!(s.validate(), Ok(()));
    s.segments_per_chunk = 9;
    assert_eq!(s.validate(), Err(CodecError::LimitExceeded));
    s.segments_per_chunk = 0;
    assert_eq!(s.validate(), Err(CodecError::Malformed));
    assert_eq!(
        decode_begin(&one, &varint(0), &varint(8 * 4096), &varint(0), 9, 1),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        decode_begin(&one, &varint(0), &varint(8 * 4096), &varint(0), 0, 1),
        Err(CodecError::Malformed)
    );

    // chunk_count 32 / 33, 0, and ceil +- 1.
    let mut k = good.clone();
    k.total_bytes = 32 * 4096;
    k.chunk_count = 32;
    assert_eq!(k.validate(), Ok(()));
    k.chunk_count = 33;
    assert_eq!(k.validate(), Err(CodecError::LimitExceeded));
    k.chunk_count = 0;
    assert_eq!(k.validate(), Err(CodecError::Malformed));
    k.chunk_count = 31; // ceil - 1
    assert_eq!(k.validate(), Err(CodecError::Malformed));
    let mut k2 = good.clone();
    k2.total_bytes = 4096;
    k2.chunk_count = 1; // exactly one full chunk
    assert_eq!(k2.validate(), Ok(()));
    k2.chunk_count = 2; // ceil + 1
    assert_eq!(k2.validate(), Err(CodecError::Malformed));
    k2.total_bytes = 4097;
    k2.chunk_count = 2; // one byte over a chunk needs two
    assert_eq!(k2.validate(), Ok(()));
    k2.chunk_count = 1;
    assert_eq!(k2.validate(), Err(CodecError::Malformed));
    k2.chunk_count = 3;
    assert_eq!(k2.validate(), Err(CodecError::Malformed));
    assert_eq!(
        decode_begin(&one, &varint(0), &varint(4097), &varint(0), 1, 3),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_begin(&one, &varint(0), &varint(4097), &varint(0), 1, 33),
        Err(CodecError::LimitExceeded)
    );

    // Field order: the first failing field wins.
    // total 0 (Malformed) before entity_count 1,025 (LimitExceeded).
    assert_eq!(
        decode_begin(&one, &varint(0), &varint(0), &varint(1025), 1, 1),
        Err(CodecError::Malformed)
    );
    // entity_count 1,025 (Limit) before spc 0 (Malformed).
    assert_eq!(
        decode_begin(&one, &varint(0), &two, &varint(1025), 0, 1),
        Err(CodecError::LimitExceeded)
    );
    // spc 9 (Limit) before chunk_count 0 (Malformed); spc 0 before chunk_count 33.
    assert_eq!(
        decode_begin(&one, &varint(0), &two, &varint(0), 9, 0),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        decode_begin(&one, &varint(0), &two, &varint(0), 0, 33),
        Err(CodecError::Malformed)
    );
    // cursor (Malformed) before total 1,048,577 (Limit).
    assert_eq!(
        decode_begin(
            &one,
            &varint(u64::MAX),
            &varint(1_048_577),
            &varint(0),
            1,
            1
        ),
        Err(CodecError::Malformed)
    );
    // chunk_count 33 (Limit) before the ceil mismatch (Malformed).
    assert_eq!(
        decode_begin(&one, &varint(0), &two, &varint(0), 1, 33),
        Err(CodecError::LimitExceeded)
    );

    // Truncation is Malformed; a trailing byte is Malformed.
    let full = begin_wire_of(&good);
    for cut in 0..full.len() {
        assert!(decode_down(&full[..cut]).is_err(), "prefix {cut}");
    }
    assert_eq!(
        decode_down(&full[..full.len() - 1]),
        Err(CodecError::Malformed)
    );
    let mut trailing = full.clone();
    trailing.push(0);
    assert_eq!(decode_down(&trailing), Err(CodecError::Malformed));

    // `encode_down`, `TransferAssembly::new`, `validate` and `decode_down`
    // agree with the reference and with each other for a grid covering every
    // field's boundaries, including values only their types can hold.
    let transfers = [1u64, u64::MAX];
    let cursors = [0u64, SEQ_LAST, u64::MAX];
    let totals = [
        0u32,
        1,
        4096,
        4097,
        32_768,
        32_769,
        1_048_576,
        1_048_577,
        u32::MAX,
    ];
    let entities = [0u16, 1, 1024, 1025, u16::MAX];
    let spcs = [0u8, 1, 8, 9, u8::MAX];
    let chunk_counts = [0u8, 1, 2, 32, 33, u8::MAX];
    let mut checked = 0usize;
    let mut valid = 0usize;
    for &transfer in &transfers {
        for &cursor in &cursors {
            for &total in &totals {
                for &ents in &entities {
                    for &spc in &spcs {
                        for &chunks in &chunk_counts {
                            let b = BeginTransfer {
                                epoch: [0x33; 16],
                                transfer: tid(transfer),
                                base_tick: 5,
                                cursor,
                                total_bytes: total,
                                entity_count: ents,
                                segments_per_chunk: spc,
                                chunk_count: chunks,
                            };
                            let want = ref_begin_err(transfer, cursor, total, ents, spc, chunks);
                            let enc = encode_down(&TransferDown::Begin(b.clone()));
                            assert_eq!(enc.as_ref().err().copied(), want, "encode {b:?}");
                            assert_eq!(b.validate().err(), want, "validate {b:?}");
                            assert_eq!(TransferAssembly::new(&b).err(), want, "assembly {b:?}");
                            let wire = begin_wire_of(&b);
                            match want {
                                None => {
                                    valid += 1;
                                    assert_eq!(enc.as_ref().unwrap(), &wire);
                                    assert_eq!(decode_down(&wire), Ok(TransferDown::Begin(b)));
                                }
                                Some(e) => assert_eq!(decode_down(&wire), Err(e), "decode {b:?}"),
                            }
                            checked += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(checked, 2 * 3 * 9 * 5 * 5 * 6);
    assert!(valid > 20, "the grid must contain valid headers ({valid})");
}

// ===========================================================================
// 7. chunk_bounds_and_hostile_lengths
// ===========================================================================

#[test]
fn chunk_bounds_and_hostile_lengths() {
    let one = varint(1);
    let ok_one = seg(1, 0xAA);
    let dec = |index: u8, segs: u8, tail: &[u8]| decode_down(&chunk_wire(&one, index, segs, tail));

    // Positive control.
    assert_eq!(
        dec(0, 1, &ok_one),
        Ok(TransferDown::Chunk(TransferChunk {
            epoch: ZERO_EPOCH,
            transfer: tid(1),
            index: 0,
            bytes: vec![0xAA]
        }))
    );
    // index 31 / 32 (the cap fires before anything else is read).
    assert!(matches!(dec(31, 1, &ok_one), Ok(TransferDown::Chunk(c)) if c.index == 31));
    assert_eq!(dec(32, 1, &ok_one), Err(CodecError::LimitExceeded));
    assert_eq!(dec(32, 1, &[]), Err(CodecError::LimitExceeded));
    assert_eq!(dec(255, 0, &[]), Err(CodecError::LimitExceeded));
    // transfer 0 is read before the index.
    assert_eq!(
        decode_down(&chunk_wire(&varint(0), 32, 1, &ok_one)),
        Err(CodecError::Malformed)
    );

    // segment_count 8 / 9, and 0.
    let eight: Vec<u8> = (0..8).flat_map(|_| seg(4096, 0x5A)).collect();
    match dec(0, 8, &eight) {
        Ok(TransferDown::Chunk(c)) => {
            assert_eq!(c.bytes.len(), 32_768);
            assert!(c.bytes.iter().all(|b| *b == 0x5A));
        }
        other => panic!("eight full segments must decode: {other:?}"),
    }
    assert_eq!(dec(0, 9, &eight), Err(CodecError::LimitExceeded));
    assert_eq!(dec(0, 9, &[]), Err(CodecError::LimitExceeded));
    assert_eq!(dec(0, 0, &ok_one), Err(CodecError::Malformed));
    assert_eq!(dec(0, 0, &[]), Err(CodecError::Malformed));

    // Segment len 4,096 / 4,097 (LimitExceeded with no body bytes present).
    assert!(
        matches!(dec(0, 1, &seg(4096, 1)), Ok(TransferDown::Chunk(c)) if c.bytes.len() == 4096)
    );
    assert_eq!(dec(0, 1, &varint(4097)), Err(CodecError::LimitExceeded));
    // With the full 4,097-byte body present the cap still wins.
    assert_eq!(dec(0, 1, &seg(4097, 1)), Err(CodecError::LimitExceeded));
    // len 0.
    assert_eq!(dec(0, 1, &varint(0)), Err(CodecError::Malformed));
    assert_eq!(
        dec(0, 2, &[seg(4096, 1), varint(0)].concat()),
        Err(CodecError::Malformed)
    );
    // A non-final segment of 4,095 is Malformed; a short final one is fine.
    assert_eq!(
        dec(0, 2, &[seg(4095, 1), seg(1, 2)].concat()),
        Err(CodecError::Malformed)
    );
    assert!(matches!(
        dec(0, 2, &[seg(4096, 1), seg(7, 2)].concat()),
        Ok(TransferDown::Chunk(c)) if c.bytes.len() == 4103
    ));
    // Cap beats the non-final-length rule: a 4,097 first segment is a limit.
    assert_eq!(
        dec(0, 2, &[varint(4097), vec![1; 4097], seg(1, 2)].concat()),
        Err(CodecError::LimitExceeded)
    );
    // A truncated body.
    let mut short = varint(10);
    short.extend([0u8; 5]);
    assert_eq!(dec(0, 1, &short), Err(CodecError::Malformed));
    assert_eq!(dec(0, 1, &varint(10)), Err(CodecError::Malformed));
    // Tenth-byte overflow is Malformed; u64::MAX and a 2^32 wrap are limits.
    assert_eq!(dec(0, 1, &overflow_varint()), Err(CodecError::Malformed));
    assert_eq!(dec(0, 1, &varint(u64::MAX)), Err(CodecError::LimitExceeded));
    let mut wrapped = varint((1 << 32) + 5);
    wrapped.extend([9u8; 5]);
    assert_eq!(dec(0, 1, &wrapped), Err(CodecError::LimitExceeded));
    // One trailing byte.
    let mut trailing = ok_one.clone();
    trailing.push(0x00);
    assert_eq!(dec(0, 1, &trailing), Err(CodecError::Malformed));
    // Fewer segments than declared.
    assert_eq!(dec(0, 2, &ok_one), Err(CodecError::Malformed));
    // Chunk decode never reads past the frame: an empty tail is Malformed.
    assert_eq!(dec(0, 1, &[]), Err(CodecError::Malformed));
}

// ===========================================================================
// 8. body_codec_bounds
// ===========================================================================

#[test]
fn body_codec_bounds() {
    // The maximal body is 22,552 bytes, round-trips and re-encodes identically.
    let body = max_body();
    let enc = encode_body(&body).unwrap();
    assert_eq!(enc.len(), 22_552);
    assert_eq!(enc.len(), MAX_ENCODED_BODY_BYTES);
    let dec = decode_body(&enc).unwrap();
    assert_eq!(dec, body);
    assert_eq!(encode_body(&dec).unwrap(), enc);

    // 1,025 entities: LimitExceeded on encode, even when also unordered.
    let many = |n: u64, descending: bool| SnapshotBody {
        entities: (1..=n)
            .map(|i| SnapshotEntity {
                entity: net(if descending { n + 1 - i } else { i }),
                health: None,
            })
            .collect(),
        turn: None,
    };
    assert!(encode_body(&many(1024, false)).is_ok());
    assert_eq!(
        encode_body(&many(1025, false)),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        encode_body(&many(1025, true)),
        Err(CodecError::LimitExceeded)
    );
    // On decode: the count alone, with no entity bytes, is LimitExceeded.
    assert_eq!(decode_body(&varint(1025)), Err(CodecError::LimitExceeded));
    assert_eq!(
        decode_body(&varint(u64::MAX)),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(decode_body(&overflow_varint()), Err(CodecError::Malformed));
    // 1,024 alone is within the cap, so the missing bytes are Malformed.
    assert_eq!(decode_body(&varint(1024)), Err(CodecError::Malformed));

    // Entity ids: repeated, descending and zero are Malformed.
    assert_eq!(
        decode_body(&[0x02, 0x01, 0x00, 0x01, 0x00, 0x00]),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_body(&[0x02, 0x02, 0x00, 0x01, 0x00, 0x00]),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_body(&[0x01, 0x00, 0x00, 0x00]),
        Err(CodecError::Malformed)
    );
    // Positive control for the same shape, ascending.
    assert!(decode_body(&[0x02, 0x01, 0x00, 0x02, 0x00, 0x00]).is_ok());
    let nid = |raw: u64| SnapshotEntity {
        entity: net(raw),
        health: None,
    };
    let enc_ids = |ids: &[u64]| {
        encode_body(&SnapshotBody {
            entities: ids.iter().map(|i| nid(*i)).collect(),
            turn: None,
        })
    };
    assert!(enc_ids(&[1, 2]).is_ok());
    assert_eq!(enc_ids(&[1, 1]), Err(CodecError::Malformed));
    assert_eq!(enc_ids(&[2, 1]), Err(CodecError::Malformed));

    // Flag bytes other than 0/1.
    assert_eq!(
        decode_body(&[0x01, 0x01, 0x02, 0x00]),
        Err(CodecError::Malformed)
    ); // health_present
    assert_eq!(
        decode_body(&[0x01, 0x01, 0x01, 0x0A, 0x0A, 0x02, 0x00]),
        Err(CodecError::Malformed)
    ); // dead
    assert_eq!(decode_body(&[0x00, 0x02]), Err(CodecError::Malformed)); // turn_present
    assert_eq!(
        decode_body(&[0x01, 0x01, 0x00, 0x01, 0x02, 0x03]),
        Err(CodecError::Malformed)
    ); // active_present
       // Flag byte 1 is the valid control.
    assert!(decode_body(&[0x01, 0x01, 0x01, 0x0A, 0x0A, 0x01, 0x00]).is_ok());

    // `active` absent from the entities (or zero).
    assert_eq!(
        decode_body(&[0x01, 0x01, 0x00, 0x01, 0x01, 0x05, 0x03]),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_body(&[0x01, 0x01, 0x00, 0x01, 0x01, 0x00, 0x03]),
        Err(CodecError::Malformed)
    );
    assert!(decode_body(&[0x01, 0x01, 0x00, 0x01, 0x01, 0x01, 0x03]).is_ok());
    let with_active = |active: u64| {
        encode_body(&SnapshotBody {
            entities: vec![nid(1)],
            turn: Some(TurnFact {
                active: Some(net(active)),
                round: 3,
            }),
        })
    };
    assert!(with_active(1).is_ok());
    assert_eq!(with_active(5), Err(CodecError::Malformed));
    // A turn with no active actor is fine.
    assert_eq!(
        decode_body(&[0x00, 0x01, 0x00, 0x07]),
        Ok(SnapshotBody {
            entities: vec![],
            turn: Some(TurnFact {
                active: None,
                round: 7
            })
        })
    );

    // u32 varints: 0xFFFF_FFFF is the maximum; one more bit overflows.
    assert!(
        decode_body(&[0x01, 0x01, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x01, 0x00, 0x00]).is_ok()
    );
    assert_eq!(
        decode_body(&[0x01, 0x01, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0x10, 0x01, 0x00, 0x00]),
        Err(CodecError::Malformed)
    );

    // Truncation: every strict prefix of a valid body is Malformed.
    let valid = [0x01u8, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x01, 0x01, 0x01, 0x03];
    assert!(decode_body(&valid).is_ok());
    for cut in 0..valid.len() {
        assert_eq!(
            decode_body(&valid[..cut]),
            Err(CodecError::Malformed),
            "prefix {cut}"
        );
    }
    // One trailing byte.
    let mut trailing = valid.to_vec();
    trailing.push(0);
    assert_eq!(decode_body(&trailing), Err(CodecError::Malformed));
    assert_eq!(decode_body(&[0x00, 0x00, 0x00]), Err(CodecError::Malformed));

    // Input size: 1,048,577 bytes is LimitExceeded; 1,048,576 is not.
    assert_eq!(
        decode_body(&vec![0u8; MIB + 1]),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(decode_body(&vec![0u8; MIB]), Err(CodecError::Malformed));
}

// ===========================================================================
// 9. transfer_and_lane_zero_refuse_each_other
// ===========================================================================

#[test]
fn transfer_and_lane_zero_refuse_each_other() {
    let begin = encode_down(&TransferDown::Begin(good_begin())).unwrap();
    let chunk = chunk_frame(ZERO_EPOCH, 1, 0, &[0, 0]);
    let cancel = cancel_frame(ZERO_EPOCH, 1, CancelReason::Superseded);
    let published = encode_up(&TransferUp::Published {
        epoch: ZERO_EPOCH,
        transfer: tid(1),
    })
    .unwrap();
    let resync = encode_up(&TransferUp::Resync {
        epoch: ZERO_EPOCH,
        after: None,
        reason: ResyncReason::Invalid,
    })
    .unwrap();
    // Every transfer frame is refused by v1 and v2 intent and delta decoders.
    for (name, f) in [
        ("begin", &begin),
        ("chunk", &chunk),
        ("cancel", &cancel),
        ("published", &published),
        ("resync", &resync),
    ] {
        assert_eq!(f[0], 0x81, "{name}");
        assert_eq!(
            codec::decode_intent(f),
            Err(CodecError::UnsupportedVersion),
            "v1 intent {name}"
        );
        assert_eq!(
            codec::decode_delta(f),
            Err(CodecError::UnsupportedVersion),
            "v1 delta {name}"
        );
        assert_eq!(
            codec_v2::decode_intent(f),
            Err(CodecError::UnsupportedVersion),
            "v2 intent {name}"
        );
        assert_eq!(
            codec_v2::decode_delta(f),
            Err(CodecError::UnsupportedVersion),
            "v2 delta {name}"
        );
    }

    // The transfer decoders refuse v1 and v2 frames the same way.
    let intent = protocol::IntentFrame {
        epoch: [0; 16],
        lane: protocol::LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: net(1),
        body: protocol::IntentBody::EndTurn,
    };
    let v1_intent = codec::encode_intent(&intent).unwrap();
    let v2_intent = codec_v2::encode_intent(&intent).unwrap();
    let delta_ops = vec![enter(1)];
    let v1_delta = v1_frame(1, delta_ops.clone());
    let v2_delta = v2_frame(1, delta_ops);
    assert_eq!(v1_intent[0], 1);
    assert_eq!(v2_intent[0], 2);
    assert_eq!(v1_delta[0], 1);
    assert_eq!(v2_delta[0], 2);
    for (name, f) in [
        ("v1 intent", &v1_intent),
        ("v2 intent", &v2_intent),
        ("v1 delta", &v1_delta),
        ("v2 delta", &v2_delta),
    ] {
        assert_eq!(
            decode_down(f),
            Err(CodecError::UnsupportedVersion),
            "down {name}"
        );
        assert_eq!(
            decode_up(f),
            Err(CodecError::UnsupportedVersion),
            "up {name}"
        );
    }

    // A valid v1 delta whose op tag is a transfer tag is UnknownMessage in v1
    // and, separately, in v2 (whose Legacy tags are the same 0..=7).
    assert!(codec::decode_delta(&v1_delta).is_ok());
    assert!(codec_v2::decode_delta(&v2_delta).is_ok());
    for tag in [0x10u8, 0x11, 0x12, 0x20, 0x21] {
        // `[version, lane, tick, seq, count, tag, entity]`: the op tag is the
        // second-to-last byte of the one-op frame.
        let mut f = v1_delta.clone();
        let at = f.len() - 2;
        assert_eq!(f[at], 0, "the EntityEnter tag");
        f[at] = tag;
        assert_eq!(
            codec::decode_delta(&f),
            Err(CodecError::UnknownMessage),
            "v1 delta tag {tag:#x}"
        );
        let mut f = v2_delta.clone();
        let at = f.len() - 2;
        assert_eq!(f[at], 0, "the EntityEnter tag");
        f[at] = tag;
        assert_eq!(
            codec_v2::decode_delta(&f),
            Err(CodecError::UnknownMessage),
            "v2 delta tag {tag:#x}"
        );
    }

    // A valid v1 intent with body tag 0x10 is UnknownMessage (and so is v2's).
    assert!(codec::decode_intent(&v1_intent).is_ok());
    let mut f = v1_intent.clone();
    let at = f.len() - 1;
    assert_eq!(f[at], protocol::INTENT_TAG_END_TURN);
    f[at] = 0x10;
    assert_eq!(codec::decode_intent(&f), Err(CodecError::UnknownMessage));
    let mut f = v2_intent.clone();
    let at = f.len() - 1;
    assert_eq!(f[at], protocol::INTENT_TAG_END_TURN);
    f[at] = 0x10;
    assert_eq!(codec_v2::decode_intent(&f), Err(CodecError::UnknownMessage));
}

// ===========================================================================
// 10. full_size_byte_assembly
// ===========================================================================

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 31 + 7) % 251) as u8).collect()
}

#[test]
fn full_size_byte_assembly() {
    for (spc, total, chunk_size) in [(8u8, MIB, 32_768usize), (1u8, 131_072usize, 4096usize)] {
        let payload = pattern(total);
        assert_eq!(payload[0], 7);
        assert_eq!(payload[1], 38);
        let frames = payload_frames(EPOCH, 1, 0, 0, spc, 0, &payload);
        assert_eq!(frames.len(), 33, "Begin plus 32 chunks");
        let begin = match decode_down(&frames[0]).unwrap() {
            TransferDown::Begin(b) => b,
            other => panic!("expected Begin, got {other:?}"),
        };
        assert_eq!(begin.total_bytes as usize, total);
        assert_eq!(begin.chunk_count, 32);
        assert_eq!(begin.segments_per_chunk, spc);
        for (i, f) in frames.iter().enumerate() {
            assert!(f.len() <= 65_536, "frame {i} is {} bytes", f.len());
        }
        // Chunk frame size: 22 header bytes plus segments of (2 + 4096).
        for f in &frames[1..] {
            assert_eq!(f.len(), 22 + usize::from(spc) * 4098);
        }
        let mut asm = TransferAssembly::new(&begin).unwrap();
        // Delivered in reverse: it completes on the last chunk, not before.
        for (n, idx) in (0..32u8).rev().enumerate() {
            let chunk = match decode_down(&frames[1 + usize::from(idx)]).unwrap() {
                TransferDown::Chunk(c) => c,
                other => panic!("expected Chunk, got {other:?}"),
            };
            assert_eq!(chunk.bytes.len(), chunk_size);
            assert_eq!(begin.chunk_len(idx), Some(chunk_size));
            let out = asm.accept(&chunk).unwrap();
            if n < 31 {
                assert_eq!(out, ChunkOutcome::Stored, "chunk {idx}");
                assert!(!asm.is_complete());
                assert_eq!(usize::from(asm.received_chunks()), n + 1);
            } else {
                assert_eq!(out, ChunkOutcome::Complete);
            }
        }
        assert!(asm.is_complete());
        assert_eq!(asm.received_chunks(), 32);
        assert_eq!(asm.into_bytes().unwrap(), payload);
    }
    // Boundary: one more byte than 32 x 4,096 at one segment needs 33 chunks.
    let mut over = good_begin();
    over.total_bytes = 131_073;
    over.chunk_count = 33;
    assert_eq!(over.validate(), Err(CodecError::LimitExceeded));
    assert!(TransferAssembly::new(&over).is_err());
}

// ===========================================================================
// 11. assembly_duplicate_conflict_shape
// ===========================================================================

#[test]
fn assembly_duplicate_conflict_shape() {
    // 8,193 bytes at one segment: chunks of 4,096, 4,096 and 1.
    let begin = BeginTransfer {
        epoch: EPOCH,
        transfer: tid(7),
        base_tick: 0,
        cursor: 0,
        total_bytes: 8193,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 3,
    };
    assert_eq!(begin.validate(), Ok(()));
    assert_eq!(begin.chunk_len(0), Some(4096));
    assert_eq!(begin.chunk_len(1), Some(4096));
    assert_eq!(begin.chunk_len(2), Some(1));
    assert_eq!(begin.chunk_len(3), None);
    let chunk = |epoch: [u8; 16], id: u64, index: u8, len: usize, fill: u8| TransferChunk {
        epoch,
        transfer: tid(id),
        index,
        bytes: vec![fill; len],
    };
    let mut asm = TransferAssembly::new(&begin).unwrap();
    assert_eq!(asm.begin(), &begin);
    assert_eq!(asm.received_chunks(), 0);
    assert!(!asm.is_complete());

    let c0 = chunk(EPOCH, 7, 0, 4096, 1);
    assert_eq!(asm.accept(&c0), Ok(ChunkOutcome::Stored));
    assert_eq!(asm.received_chunks(), 1);

    // Every refusal leaves the assembly equal to its clone from before.
    let errors: Vec<(&str, TransferChunk, AssemblyError)> = vec![
        (
            "different bytes",
            chunk(EPOCH, 7, 0, 4096, 9),
            AssemblyError::Conflict,
        ),
        (
            "index = chunk_count",
            chunk(EPOCH, 7, 3, 4096, 1),
            AssemblyError::OutOfRange,
        ),
        (
            "index 31",
            chunk(EPOCH, 7, 31, 4096, 1),
            AssemblyError::OutOfRange,
        ),
        (
            "non-final 1 byte short",
            chunk(EPOCH, 7, 1, 4095, 1),
            AssemblyError::WrongLength,
        ),
        (
            "non-final 1 byte long",
            chunk(EPOCH, 7, 1, 4097, 1),
            AssemblyError::WrongLength,
        ),
        (
            "final 1 byte long",
            chunk(EPOCH, 7, 2, 2, 1),
            AssemblyError::WrongLength,
        ),
        (
            "final empty",
            chunk(EPOCH, 7, 2, 0, 1),
            AssemblyError::WrongLength,
        ),
        (
            "wrong epoch",
            chunk(OTHER_EPOCH, 7, 1, 4096, 1),
            AssemblyError::WrongTransfer,
        ),
        (
            "wrong id",
            chunk(EPOCH, 8, 1, 4096, 1),
            AssemblyError::WrongTransfer,
        ),
        // Precedence: the transfer check comes first, then the index, then
        // the length, then the conflict test.
        (
            "wrong id and out of range",
            chunk(EPOCH, 8, 9, 1, 1),
            AssemblyError::WrongTransfer,
        ),
        (
            "out of range and wrong length",
            chunk(EPOCH, 7, 3, 1, 1),
            AssemblyError::OutOfRange,
        ),
        (
            "stored index with wrong length",
            chunk(EPOCH, 7, 0, 4095, 9),
            AssemblyError::WrongLength,
        ),
    ];
    for (name, c, want) in errors {
        let before = asm.clone();
        assert_eq!(asm.accept(&c), Err(want), "{name}");
        assert_eq!(asm, before, "{name} changed the assembly");
    }
    // An identical duplicate is Duplicate and also changes nothing.
    let before = asm.clone();
    assert_eq!(asm.accept(&c0), Ok(ChunkOutcome::Duplicate));
    assert_eq!(asm, before);
    assert_eq!(asm.received_chunks(), 1);

    // `into_bytes` while incomplete returns Err with the same assembly.
    let returned = asm.clone().into_bytes().expect_err("incomplete");
    assert_eq!(returned, asm);
    let mut asm2 = asm.clone();
    assert_eq!(
        asm2.accept(&chunk(EPOCH, 7, 2, 1, 3)),
        Ok(ChunkOutcome::Stored)
    );
    assert_eq!(
        asm2.clone().into_bytes().expect_err("still incomplete"),
        asm2
    );

    // Completing it, then the whole payload comes back by ownership.
    assert_eq!(
        asm2.accept(&chunk(EPOCH, 7, 1, 4096, 2)),
        Ok(ChunkOutcome::Complete)
    );
    assert!(asm2.is_complete());
    // A duplicate after completion is still a Duplicate, not a second Complete.
    let before = asm2.clone();
    assert_eq!(asm2.accept(&c0), Ok(ChunkOutcome::Duplicate));
    assert_eq!(asm2, before);
    let bytes = asm2.into_bytes().unwrap();
    assert_eq!(bytes.len(), 8193);
    assert!(bytes[..4096].iter().all(|b| *b == 1));
    assert!(bytes[4096..8192].iter().all(|b| *b == 2));
    assert_eq!(bytes[8192], 3);

    // `new` refuses an invalid header without allocating.
    let mut bad = begin.clone();
    bad.chunk_count = 2;
    assert_eq!(TransferAssembly::new(&bad), Err(CodecError::Malformed));
}

// ===========================================================================
// 12. limits_validate_tightening_only
// ===========================================================================

#[test]
fn limits_validate_tightening_only() {
    let v1 = TransferLimits::v1();
    assert_eq!(v1.validate(), Ok(()));
    assert!(ReassemblyPool::new(v1.clone()).is_ok());
    assert_eq!(pool_with(v1.clone()).limits(), v1);

    // Each field at its floor is valid (max-valid / first-invalid pairs).
    let floor = TransferLimits {
        max_receivers: 1,
        deadline_ms: 1,
        staging_bytes_per_receiver: 22_552,
        staging_bytes_total: 22_552,
        delta_frames_per_receiver: 1,
        delta_bytes_per_receiver: 65_536,
        delta_bytes_total: 65_536,
    };
    assert_eq!(floor.validate(), Ok(()));
    assert_eq!(pool_with(floor.clone()).limits(), floor);

    let loosens = |mut l: TransferLimits, f: &dyn Fn(&mut TransferLimits), field: &'static str| {
        f(&mut l);
        assert_eq!(
            l.validate(),
            Err(LimitsError::LoosensPolicy { field }),
            "{field}"
        );
        assert_eq!(
            ReassemblyPool::new(l).err(),
            Some(LimitsError::LoosensPolicy { field })
        );
    };
    loosens(v1.clone(), &|l| l.max_receivers = 9, "max_receivers");
    loosens(v1.clone(), &|l| l.deadline_ms = 5_001, "deadline_ms");
    loosens(
        v1.clone(),
        &|l| l.staging_bytes_per_receiver = 1_048_577,
        "staging_bytes_per_receiver",
    );
    loosens(
        v1.clone(),
        &|l| l.staging_bytes_total = 8_388_609,
        "staging_bytes_total",
    );
    loosens(
        v1.clone(),
        &|l| l.delta_frames_per_receiver = 129,
        "delta_frames_per_receiver",
    );
    loosens(
        v1.clone(),
        &|l| l.delta_bytes_per_receiver = 2_097_153,
        "delta_bytes_per_receiver",
    );
    loosens(
        v1.clone(),
        &|l| l.delta_bytes_total = 16_777_217,
        "delta_bytes_total",
    );
    loosens(
        v1.clone(),
        &|l| l.max_receivers = usize::MAX,
        "max_receivers",
    );
    loosens(v1.clone(), &|l| l.deadline_ms = u64::MAX, "deadline_ms");

    let invalid = |mut l: TransferLimits, f: &dyn Fn(&mut TransferLimits), field: &'static str| {
        f(&mut l);
        assert_eq!(
            l.validate(),
            Err(LimitsError::InvalidLimits { field }),
            "{field}"
        );
        assert_eq!(
            ReassemblyPool::new(l).err(),
            Some(LimitsError::InvalidLimits { field })
        );
    };
    invalid(v1.clone(), &|l| l.max_receivers = 0, "max_receivers");
    invalid(v1.clone(), &|l| l.deadline_ms = 0, "deadline_ms");
    invalid(
        v1.clone(),
        &|l| l.staging_bytes_per_receiver = 22_551,
        "staging_bytes_per_receiver",
    );
    invalid(
        v1.clone(),
        &|l| l.staging_bytes_per_receiver = 0,
        "staging_bytes_per_receiver",
    );
    // A total one below its per-receiver value.
    invalid(
        v1.clone(),
        &|l| l.staging_bytes_total = l.staging_bytes_per_receiver - 1,
        "staging_bytes_total",
    );
    invalid(
        v1.clone(),
        &|l| l.delta_frames_per_receiver = 0,
        "delta_frames_per_receiver",
    );
    invalid(
        v1.clone(),
        &|l| l.delta_bytes_per_receiver = 65_535,
        "delta_bytes_per_receiver",
    );
    invalid(
        v1.clone(),
        &|l| l.delta_bytes_total = l.delta_bytes_per_receiver - 1,
        "delta_bytes_total",
    );
    // The same one-below rule at the floor.
    invalid(
        floor.clone(),
        &|l| l.staging_bytes_total = 22_551,
        "staging_bytes_total",
    );
    invalid(
        floor.clone(),
        &|l| l.delta_bytes_total = 65_535,
        "delta_bytes_total",
    );

    // With two bad fields the first in declaration order is reported.
    let mut two = v1.clone();
    two.max_receivers = 0;
    two.deadline_ms = 0;
    assert_eq!(
        two.validate(),
        Err(LimitsError::InvalidLimits {
            field: "max_receivers"
        })
    );
    assert_eq!(
        ReassemblyPool::new(two).err(),
        Some(LimitsError::InvalidLimits {
            field: "max_receivers"
        })
    );
    let mut two = v1.clone();
    two.max_receivers = 9; // loosens, declared first
    two.deadline_ms = 0; // invalid, declared second
    assert_eq!(
        two.validate(),
        Err(LimitsError::LoosensPolicy {
            field: "max_receivers"
        })
    );
    let mut two = v1.clone();
    two.deadline_ms = 5_001;
    two.staging_bytes_per_receiver = 22_551;
    assert_eq!(
        two.validate(),
        Err(LimitsError::LoosensPolicy {
            field: "deadline_ms"
        })
    );
    let mut two = v1.clone();
    two.delta_bytes_total = 16_777_217;
    two.staging_bytes_total = 1; // below its per-receiver value
    assert_eq!(
        two.validate(),
        Err(LimitsError::InvalidLimits {
            field: "staging_bytes_total"
        })
    );
    let mut all = v1;
    all.delta_bytes_total = 0;
    all.delta_bytes_per_receiver = 0;
    all.delta_frames_per_receiver = 0;
    all.staging_bytes_total = 0;
    all.staging_bytes_per_receiver = 0;
    all.deadline_ms = 0;
    all.max_receivers = 0;
    assert_eq!(
        all.validate(),
        Err(LimitsError::InvalidLimits {
            field: "max_receivers"
        })
    );

    // Display is `<Variant> at transfer/limits: <field>`.
    assert_eq!(
        LimitsError::InvalidLimits {
            field: "deadline_ms"
        }
        .to_string(),
        "InvalidLimits at transfer/limits: deadline_ms"
    );
    assert_eq!(
        LimitsError::LoosensPolicy {
            field: "deadline_ms"
        }
        .to_string(),
        "LoosensPolicy at transfer/limits: deadline_ms"
    );
}
// ===========================================================================
// 13. max_valid_body_publishes_atomically
// ===========================================================================

#[test]
fn max_valid_body_publishes_atomically() {
    let body = max_body();
    let payload = encode_body(&body).unwrap();
    // 6 chunks shuffled at 1 segment per chunk; 1 chunk at 8.
    for (spc, order) in [(1u8, vec![4usize, 1, 5, 0, 3, 2]), (8u8, vec![0usize])] {
        let (mut pool, rid) = mkpool();
        let frames = plan(1, 100, 5, spc, &body);
        // The planner agrees with an independent hand cut of the payload.
        assert_eq!(
            frames,
            payload_frames(EPOCH, 1, 100, 5, spc, 1024, &payload)
        );
        let n = order.len();
        assert_eq!(frames.len(), n + 1);

        assert_eq!(ok_t(&mut pool, rid, &frames[0], T0), TransferOutcome::Begun);
        let st = pool.status(rid).unwrap();
        let inf = st.in_flight.clone().expect("in flight");
        assert_eq!(inf.transfer, tid(1));
        assert_eq!((inf.cursor, inf.base_tick), (5, 100));
        assert_eq!(inf.received, 0);
        assert_eq!(usize::from(inf.chunk_count), n);
        assert_eq!(inf.reserved_bytes, MAX_BODY_BYTES);
        assert_eq!(inf.deadline_ms, T0 + 5_000);
        assert_eq!(pool.staging_bytes(), MAX_BODY_BYTES);

        for (k, idx) in order[..n - 1].iter().enumerate() {
            assert_eq!(
                ok_t(&mut pool, rid, &frames[1 + idx], T0),
                TransferOutcome::Stored {
                    received: (k + 1) as u8,
                    chunk_count: n as u8
                }
            );
        }
        // Before the last chunk: n-1 of n received, nothing queued, 22,552
        // staged, nothing published.
        let st = pool.status(rid).unwrap();
        let inf = st.in_flight.expect("still in flight");
        assert_eq!(usize::from(inf.received), n - 1);
        assert_eq!(usize::from(inf.chunk_count), n);
        assert_eq!(inf.reserved_bytes, MAX_BODY_BYTES);
        assert_eq!(st.queued_updates, 0);
        assert_eq!(st.published, None);
        assert_eq!(st.retired_through, 0);
        assert_eq!(pool.staging_bytes(), 22_552);
        if spc == 1 {
            assert_eq!((inf.received, inf.chunk_count), (5, 6));
        }
        assert_eq!(pool.pop_update(rid), None);

        // The last chunk publishes, all in that one call.
        let last = order[n - 1];
        assert_eq!(
            ok_t(&mut pool, rid, &frames[1 + last], T0),
            TransferOutcome::Published {
                transfer: tid(1),
                released: 0
            }
        );
        let st = pool.status(rid).unwrap();
        assert_eq!(st.queued_updates, 1);
        assert_eq!(st.in_flight, None);
        assert_eq!(st.retired_through, 1);
        assert_eq!(
            st.published,
            Some(PublishedBase {
                transfer: tid(1),
                base_tick: 100,
                cursor: 5
            })
        );
        assert_eq!(st.next_event_seq, 6);
        assert_eq!(st.mode, GateMode::Live);
        assert_eq!(st.resync, None);
        assert_eq!(pool.staging_bytes(), 0);

        // `pop_update` yields the exact body, once.
        let update = pool.pop_update(rid).expect("snapshot queued");
        assert_eq!(
            update,
            ReplicaUpdate::Snapshot(Publication {
                transfer: tid(1),
                base_tick: 100,
                cursor: 5,
                body: body.clone()
            })
        );
        assert_eq!(pool.pop_update(rid), None);
        let mut replica = Replica::default();
        replica.apply(&update);
        assert_matches(&replica, &max_body_facts());
        assert_eq!(replica.facts.entities.len(), 1024);

        // The `Published` frame the client answers with round-trips.
        let reply = TransferUp::Published {
            epoch: EPOCH,
            transfer: tid(1),
        };
        assert_eq!(decode_up(&encode_up(&reply).unwrap()), Ok(reply));
    }
}

// ===========================================================================
// 14. second_in_flight_refused
// ===========================================================================

#[test]
fn second_in_flight_refused() {
    let (mut pool, rid) = mkpool();
    let f1 = plan(1, 100, 0, 1, &small_body(1));
    let f2 = plan(2, 100, 0, 1, &small_body(2));
    assert_eq!(f1.len(), 2);
    assert_eq!(ok_t(&mut pool, rid, &f1[0], T0), TransferOutcome::Begun);
    // Begin(2) while 1 is in flight is InFlight, with the pool unchanged.
    assert_eq!(
        refuse_t(&mut pool, rid, &f2[0], T0),
        TransferError::InFlight
    );
    assert_eq!(
        refuse_t(&mut pool, rid, &f2[0], T0 + 10),
        TransferError::InFlight
    );
    // Positive control: the in-flight id itself is an idempotent duplicate.
    assert_eq!(ok_t(&mut pool, rid, &f1[0], T0), TransferOutcome::Duplicate);
    // 1 publishes; Begin(2) is then accepted.
    assert!(matches!(
        ok_t(&mut pool, rid, &f1[1], T0),
        TransferOutcome::Published { .. }
    ));
    assert_eq!(ok_t(&mut pool, rid, &f2[0], T0), TransferOutcome::Begun);
    // Begin(1) is StaleTransfer, with the pool unchanged.
    assert_eq!(
        refuse_t(&mut pool, rid, &f1[0], T0),
        TransferError::StaleTransfer
    );
    // 2 is the one in flight and publishes over the queued 1.
    assert!(matches!(
        ok_t(&mut pool, rid, &f2[1], T0),
        TransferOutcome::Published { .. }
    ));
    let mut replica = Replica::default();
    replica.drain(&mut pool, rid);
    assert_eq!(replica.order, vec!["S"]);
    assert_matches(&replica, &small_facts(2));
}

// ===========================================================================
// 15. staging_caps_per_receiver_and_aggregate
// ===========================================================================

#[test]
fn staging_caps_per_receiver_and_aggregate() {
    let begin_of_size =
        |id: u64, total: u32, spc: u8| begin_frame(&begin_of(EPOCH, id, 0, 0, total, 0, spc));

    // At v1, eight receivers hold a 1,048,576-byte transfer each at once.
    let mut pool = pool_with(TransferLimits::v1());
    let ids: Vec<ReceiverId> = (0..8)
        .map(|_| add(&mut pool, DeltaWire::V2, GateMode::Live))
        .collect();
    for (n, rid) in ids.iter().enumerate() {
        assert_eq!(
            ok_t(&mut pool, *rid, &begin_of_size(1, MIB as u32, 8), T0),
            TransferOutcome::Begun
        );
        assert_eq!(pool.staging_bytes(), (n + 1) * MIB);
    }
    assert_eq!(pool.staging_bytes(), 8_388_608);
    // A ninth add_receiver is ReceiversFull, with the pool unchanged.
    let before = pool.clone();
    assert_eq!(
        pool.add_receiver(EPOCH, DeltaWire::V2, GateMode::Live),
        Err(TransferError::ReceiversFull)
    );
    assert_eq!(pool, before);
    // The ids are monotonic.
    assert!(ids.windows(2).all(|w| w[0] < w[1]));

    // Per receiver 22,552: Begin of 22,552 is accepted and 22,553 is
    // StagingFull.
    let mut limits = TransferLimits::v1();
    limits.staging_bytes_per_receiver = 22_552;
    let mut pool = pool_with(limits);
    let a = add(&mut pool, DeltaWire::V2, GateMode::Live);
    let b = add(&mut pool, DeltaWire::V2, GateMode::Live);
    assert_eq!(
        refuse_t(&mut pool, b, &begin_of_size(1, 22_553, 1), T0),
        TransferError::StagingFull
    );
    assert_eq!(
        refuse_t(&mut pool, b, &begin_of_size(1, MIB as u32, 8), T0),
        TransferError::StagingFull
    );
    assert_eq!(
        ok_t(&mut pool, a, &begin_of_size(1, 22_552, 1), T0),
        TransferOutcome::Begun
    );
    assert_eq!(pool.staging_bytes(), 22_552);
    // The failed receiver is untouched and can still begin a legal transfer.
    assert_eq!(
        ok_t(&mut pool, b, &begin_of_size(1, 22_552, 1), T0),
        TransferOutcome::Begun
    );

    // Pool total tightened to 1,048,576 + 22,552: a second receiver's 22,552
    // fits exactly and one byte more is AggregateFull.
    let mut limits = TransferLimits::v1();
    limits.staging_bytes_total = MIB + 22_552;
    for (second, want) in [
        (22_552u32, Ok(TransferOutcome::Begun)),
        (22_553u32, Err(TransferError::AggregateFull)),
    ] {
        let mut pool = pool_with(limits.clone());
        let r1 = add(&mut pool, DeltaWire::V2, GateMode::Live);
        let r2 = add(&mut pool, DeltaWire::V2, GateMode::Live);
        assert_eq!(
            ok_t(&mut pool, r1, &begin_of_size(1, MIB as u32, 8), T0),
            TransferOutcome::Begun
        );
        let frame = begin_of_size(1, second, 1);
        match want {
            Ok(o) => {
                assert_eq!(ok_t(&mut pool, r2, &frame, T0), o);
                assert_eq!(pool.staging_bytes(), MIB + 22_552);
            }
            Err(e) => {
                assert_eq!(refuse_t(&mut pool, r2, &frame, T0), e);
                assert_eq!(pool.staging_bytes(), MIB);
            }
        }
    }

    // Publish, expiry, cancel and remove_receiver each give the room back.
    let mut limits = TransferLimits::v1();
    limits.staging_bytes_per_receiver = 22_552;
    limits.staging_bytes_total = 22_552;
    let body = max_body();
    let frames_a = plan(1, 100, 0, 1, &body);
    let full_pool = || {
        let mut pool = pool_with(limits.clone());
        let a = add(&mut pool, DeltaWire::V2, GateMode::Live);
        let b = add(&mut pool, DeltaWire::V2, GateMode::Live);
        assert_eq!(ok_t(&mut pool, a, &frames_a[0], T0), TransferOutcome::Begun);
        assert_eq!(pool.staging_bytes(), 22_552);
        // The pool is full: B is refused, positive control below.
        assert_eq!(
            refuse_t(&mut pool, b, &frames_a[0], T0),
            TransferError::AggregateFull
        );
        (pool, a, b)
    };
    // publish
    let (mut pool, a, b) = full_pool();
    for f in &frames_a[1..] {
        ok_t(&mut pool, a, f, T0);
    }
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(ok_t(&mut pool, b, &frames_a[0], T0), TransferOutcome::Begun);
    // expiry
    let (mut pool, a, b) = full_pool();
    assert_eq!(pool.poll(T0 + 5_000), vec![a]);
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(
        ok_t(&mut pool, b, &frames_a[0], T0 + 5_000),
        TransferOutcome::Begun
    );
    // cancel
    let (mut pool, a, b) = full_pool();
    assert_eq!(
        ok_t(
            &mut pool,
            a,
            &cancel_frame(EPOCH, 1, CancelReason::Aborted),
            T0
        ),
        TransferOutcome::Cancelled { discarded: true }
    );
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(ok_t(&mut pool, b, &frames_a[0], T0), TransferOutcome::Begun);
    // remove_receiver (and a repeat is a no-op)
    let (mut pool, a, b) = full_pool();
    pool.remove_receiver(a);
    assert_eq!(pool.staging_bytes(), 0);
    let before = pool.clone();
    pool.remove_receiver(a);
    assert_eq!(pool, before);
    assert_eq!(pool.status(a), Err(TransferError::UnknownReceiver));
    assert_eq!(ok_t(&mut pool, b, &frames_a[0], T0), TransferOutcome::Begun);
    // A removed receiver's id is never reused.
    let c = add(&mut pool, DeltaWire::V2, GateMode::Live);
    assert!(c > b);
}

// ===========================================================================
// 16. duplicates_idempotent_no_deadline_extension
// ===========================================================================

#[test]
fn duplicates_idempotent_no_deadline_extension() {
    let t = T0;
    let (mut pool, rid) = mkpool();
    let frames = plan(1, 100, 0, 1, &max_body());
    assert_eq!(frames.len(), 7);
    assert_eq!(ok_t(&mut pool, rid, &frames[0], t), TransferOutcome::Begun);
    let deadline = |pool: &ReassemblyPool| pool.status(rid).unwrap().in_flight.unwrap().deadline_ms;
    assert_eq!(deadline(&pool), t + 5_000);
    assert_eq!(
        ok_t(&mut pool, rid, &frames[1], t),
        TransferOutcome::Stored {
            received: 1,
            chunk_count: 6
        }
    );

    // A duplicate Begin and a duplicate chunk at t + 4,000 are Duplicate and
    // do not move the deadline.
    assert_eq!(
        ok_t(&mut pool, rid, &frames[0], t + 4_000),
        TransferOutcome::Duplicate
    );
    assert_eq!(deadline(&pool), t + 5_000);
    assert_eq!(
        ok_t(&mut pool, rid, &frames[1], t + 4_000),
        TransferOutcome::Duplicate
    );
    assert_eq!(deadline(&pool), t + 5_000);
    // A fresh chunk is Stored and also does not extend it.
    assert_eq!(
        ok_t(&mut pool, rid, &frames[2], t + 4_000),
        TransferOutcome::Stored {
            received: 2,
            chunk_count: 6
        }
    );
    assert_eq!(deadline(&pool), t + 5_000);
    // A refused frame (another Begin) does not extend it either.
    let other = plan(2, 100, 0, 1, &small_body(1));
    assert_eq!(
        refuse_t(&mut pool, rid, &other[0], t + 4_500),
        TransferError::InFlight
    );
    assert_eq!(deadline(&pool), t + 5_000);
    // A duplicate does not change the receive count.
    let inf = pool.status(rid).unwrap().in_flight.unwrap();
    assert_eq!(inf.received, 2);

    for f in &frames[3..6] {
        ok_t(&mut pool, rid, f, t + 4_000);
    }
    assert_eq!(pool.status(rid).unwrap().in_flight.unwrap().received, 5);
    // The last chunk at t + 5,000 is Failed(Expired).
    assert_eq!(
        fail_t(&mut pool, rid, &frames[6], t + 5_000),
        FailReason::Expired
    );
    assert_failed_clean(&pool, rid, 1, ResyncReason::Expired);
    assert_eq!(pool.status(rid).unwrap().queued_updates, 0);
    // Positive control: the same last chunk one millisecond before would
    // have published, shown on a fresh pool.
    let (mut pool2, rid2) = mkpool();
    ok_t(&mut pool2, rid2, &frames[0], t);
    for f in &frames[1..6] {
        ok_t(&mut pool2, rid2, f, t);
    }
    ok_t(&mut pool2, rid2, &frames[0], t + 4_999); // a late duplicate
    assert!(matches!(
        ok_t(&mut pool2, rid2, &frames[6], t + 4_999),
        TransferOutcome::Published { .. }
    ));
}

// ===========================================================================
// 17. conflicts_discard_retire_and_flag
// ===========================================================================

/// A receiver with transfer 1 published and popped into the oracle, and
/// transfer 2 begun with chunk 0 stored.
fn conflict_setup() -> (ReassemblyPool, ReceiverId, Replica, Vec<Vec<u8>>) {
    let (mut pool, rid) = mkpool();
    let f1 = plan(1, 100, 0, 1, &small_body(10));
    assert!(matches!(
        offer_all(&mut pool, rid, &f1, T0),
        TransferOutcome::Published { .. }
    ));
    let mut replica = Replica::default();
    assert_eq!(replica.drain(&mut pool, rid), 1);
    assert_matches(&replica, &small_facts(10));
    assert_eq!(
        encode_body(&big_fixture().body()).unwrap().len(),
        BIG_FIXTURE_BYTES
    );
    let f2 = big_fixture().frames(2, 100, 0, 1);
    assert_eq!(f2.len(), 7);
    assert_eq!(ok_t(&mut pool, rid, &f2[0], T0), TransferOutcome::Begun);
    assert!(matches!(
        ok_t(&mut pool, rid, &f2[1], T0),
        TransferOutcome::Stored { received: 1, .. }
    ));
    (pool, rid, replica, f2)
}

#[test]
fn conflicts_discard_retire_and_flag() {
    type Trigger = fn(&[Vec<u8>]) -> Vec<u8>;
    let triggers: [(&str, Trigger); 4] = [
        ("conflicting chunk", |f| {
            // Chunk 0 again with one byte changed.
            let TransferDown::Chunk(mut c) = decode_down(&f[1]).unwrap() else {
                panic!("chunk")
            };
            c.bytes[100] ^= 0x01;
            encode_down(&TransferDown::Chunk(c)).unwrap()
        }),
        ("conflicting Begin", |f| {
            // Same id, other cursor.
            let TransferDown::Begin(mut b) = decode_down(&f[0]).unwrap() else {
                panic!("begin")
            };
            b.cursor += 1;
            encode_down(&TransferDown::Begin(b)).unwrap()
        }),
        ("index past chunk_count", |_| {
            // chunk_count is 6, and 6 is below the wire cap of 32.
            chunk_frame(EPOCH, 2, 6, &[0u8; 4096])
        }),
        ("wrong-length chunk", |_| {
            // Chunk 1 is not final and must carry exactly 4,096 bytes.
            chunk_frame(EPOCH, 2, 1, &[0u8; 100])
        }),
    ];
    for (name, make) in triggers {
        let (mut pool, rid, mut replica, f2) = conflict_setup();
        assert_eq!(pool.staging_bytes(), BIG_FIXTURE_BYTES, "{name}");
        let frame = make(&f2);
        assert_eq!(
            fail_t(&mut pool, rid, &frame, T0),
            FailReason::Conflict,
            "{name}"
        );
        // Frees staging, retires the id, flags Invalid, AwaitSnapshot,
        // queues nothing.
        assert_failed_clean(&pool, rid, 2, ResyncReason::Invalid);
        let st = pool.status(rid).unwrap();
        assert_eq!(st.queued_updates, 0, "{name}");
        // The earlier published snapshot is still the oracle's state.
        assert_eq!(replica.drain(&mut pool, rid), 0);
        assert_matches(&replica, &small_facts(10));
        assert_eq!(st.published.unwrap().transfer, tid(1));
        // A later chunk of that id is StaleTransfer (nothing changes).
        assert_eq!(
            refuse_t(&mut pool, rid, &f2[2], T0),
            TransferError::StaleTransfer,
            "{name}"
        );
        // A higher Begin clears `resync` and publishes.
        let f3 = big_fixture().frames(3, 100, 0, 1);
        assert_eq!(ok_t(&mut pool, rid, &f3[0], T0), TransferOutcome::Begun);
        let st = pool.status(rid).unwrap();
        assert_eq!(st.resync, None, "{name}");
        assert_eq!(st.mode, GateMode::AwaitSnapshot, "{name}");
        for f in &f3[1..f3.len() - 1] {
            ok_t(&mut pool, rid, f, T0);
        }
        assert!(matches!(
            ok_t(&mut pool, rid, f3.last().unwrap(), T0),
            TransferOutcome::Published { .. }
        ));
        let st = pool.status(rid).unwrap();
        assert_eq!(st.mode, GateMode::Live, "{name}");
        assert_eq!(st.retired_through, 3);
        assert_eq!(replica.drain(&mut pool, rid), 1);
        assert_matches(&replica, &big_facts(&[]));
        assert_eq!(replica.snapshots, 2);
    }
}

// ===========================================================================
// 18. reorder_and_missing_chunks
// ===========================================================================

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn permutation(seed: u64, n: usize) -> Vec<usize> {
    let mut rng = XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut v: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = (rng.next() % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
    v
}

#[test]
fn reorder_and_missing_chunks() {
    let frames = plan(1, 100, 0, 1, &max_body());
    assert_eq!(frames.len(), 7);
    let mut orders: Vec<Vec<usize>> = vec![
        vec![0, 1, 2, 3, 4, 5],
        vec![5, 4, 3, 2, 1, 0],
        vec![0, 5, 1, 4, 2, 3],
        vec![2, 0, 4, 1, 5, 3],
    ];
    for seed in 1..=64u64 {
        orders.push(permutation(seed, 6));
    }
    let distinct: std::collections::BTreeSet<_> = orders.iter().cloned().collect();
    assert!(
        distinct.len() >= 40,
        "permutations must vary ({})",
        distinct.len()
    );
    let mut first_body: Option<SnapshotBody> = None;
    for order in &orders {
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..6).collect::<Vec<_>>());
        let (mut pool, rid) = mkpool();
        assert_eq!(ok_t(&mut pool, rid, &frames[0], T0), TransferOutcome::Begun);
        for (k, idx) in order[..5].iter().enumerate() {
            assert_eq!(
                ok_t(&mut pool, rid, &frames[1 + idx], T0),
                TransferOutcome::Stored {
                    received: (k + 1) as u8,
                    chunk_count: 6
                },
                "order {order:?}"
            );
        }
        assert_eq!(pool.status(rid).unwrap().queued_updates, 0);
        assert_eq!(
            ok_t(&mut pool, rid, &frames[1 + order[5]], T0),
            TransferOutcome::Published {
                transfer: tid(1),
                released: 0
            },
            "order {order:?}"
        );
        let Some(ReplicaUpdate::Snapshot(p)) = pool.pop_update(rid) else {
            panic!("snapshot queued for {order:?}");
        };
        assert_eq!(pool.pop_update(rid), None);
        match &first_body {
            None => first_body = Some(p.body.clone()),
            Some(b) => assert_eq!(&p.body, b, "order {order:?} published another body"),
        }
        let mut replica = Replica::default();
        replica.apply(&ReplicaUpdate::Snapshot(p));
        assert_matches(&replica, &max_body_facts());
    }
    assert_eq!(first_body, Some(max_body()));

    // With chunk 3 missing: poll(t + 4,999) returns nothing and
    // poll(t + 5,000) returns the receiver; nothing is ever queued.
    let (mut pool, rid) = mkpool();
    ok_t(&mut pool, rid, &frames[0], T0);
    for (k, idx) in [0usize, 1, 2, 4, 5].iter().enumerate() {
        assert_eq!(
            ok_t(&mut pool, rid, &frames[1 + idx], T0),
            TransferOutcome::Stored {
                received: (k + 1) as u8,
                chunk_count: 6
            }
        );
    }
    assert_eq!(pool.status(rid).unwrap().queued_updates, 0);
    assert_eq!(pool.poll(T0 + 4_999), Vec::<ReceiverId>::new());
    let st = pool.status(rid).unwrap();
    assert_eq!(st.queued_updates, 0);
    assert_eq!(st.in_flight.unwrap().received, 5);
    assert_eq!(pool.staging_bytes(), 22_552);
    assert_eq!(pool.poll(T0 + 5_000), vec![rid]);
    assert_eq!(pool.status(rid).unwrap().queued_updates, 0);
    assert_failed_clean(&pool, rid, 1, ResyncReason::Expired);
    assert_eq!(pool.pop_update(rid), None);
    // The expiry has happened once: a repeat poll reports nothing.
    assert_eq!(pool.poll(T0 + 6_000), Vec::<ReceiverId>::new());
    // The missing chunk now arrives too late.
    assert_eq!(
        refuse_t(&mut pool, rid, &frames[1 + 3], T0 + 6_000),
        TransferError::StaleTransfer
    );
}

// ===========================================================================
// 19. deadline_boundary_4999_5000
// ===========================================================================

#[test]
fn deadline_boundary_4999_5000() {
    let t = 100;
    let frames = plan(1, 100, 0, 1, &max_body());
    // The last chunk at t + 4,999 publishes.
    let (mut pool, rid) = mkpool();
    ok_t(&mut pool, rid, &frames[0], t);
    for f in &frames[1..6] {
        ok_t(&mut pool, rid, f, t);
    }
    assert_eq!(
        ok_t(&mut pool, rid, &frames[6], t + 4_999),
        TransferOutcome::Published {
            transfer: tid(1),
            released: 0
        }
    );
    assert_eq!(pool.status(rid).unwrap().queued_updates, 1);

    // In a separate pool the last chunk at t + 5,000 is Failed(Expired) with
    // nothing queued and staging 0; the next chunk is StaleTransfer.
    let (mut pool, rid) = mkpool();
    ok_t(&mut pool, rid, &frames[0], t);
    for f in &frames[1..6] {
        ok_t(&mut pool, rid, f, t);
    }
    assert_eq!(pool.staging_bytes(), 22_552);
    assert_eq!(
        fail_t(&mut pool, rid, &frames[6], t + 5_000),
        FailReason::Expired
    );
    assert_failed_clean(&pool, rid, 1, ResyncReason::Expired);
    assert_eq!(pool.status(rid).unwrap().queued_updates, 0);
    assert_eq!(pool.pop_update(rid), None);
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(
        refuse_t(&mut pool, rid, &frames[6], t + 5_000),
        TransferError::StaleTransfer
    );
    assert_eq!(
        refuse_t(&mut pool, rid, &frames[1], t + 5_001),
        TransferError::StaleTransfer
    );

    // A subsequent call with the smaller now_ms t + 10 is clamped and changes
    // nothing: the whole pool equals its clone, and a new transfer's
    // deadline is measured from the clamped clock (the latest value the pool
    // has seen, t + 5,001), not from t + 10.
    let before = pool.clone();
    assert_eq!(
        pool.offer_transfer(rid, &frames[6], t + 10),
        Err(TransferError::StaleTransfer)
    );
    assert_eq!(pool, before);
    assert_eq!(pool.poll(t + 10), Vec::<ReceiverId>::new());
    assert_eq!(pool, before);
    let next = plan(2, 100, 0, 1, &small_body(1));
    assert_eq!(
        ok_t(&mut pool, rid, &next[0], t + 10),
        TransferOutcome::Begun
    );
    assert_eq!(
        pool.status(rid).unwrap().in_flight.unwrap().deadline_ms,
        (t + 5_001) + 5_000
    );

    // A Begin at u64::MAX - 10 has deadline u64::MAX (saturating).
    let (mut pool, rid) = mkpool();
    let begin = plan(1, 100, 0, 1, &small_body(1));
    let late = u64::MAX - 10;
    assert_eq!(
        ok_t(&mut pool, rid, &begin[0], late),
        TransferOutcome::Begun
    );
    assert_eq!(
        pool.status(rid).unwrap().in_flight.unwrap().deadline_ms,
        u64::MAX
    );
    assert_eq!(pool.poll(u64::MAX - 1), Vec::<ReceiverId>::new());
    assert_eq!(pool.poll(u64::MAX), vec![rid]);
    assert_failed_clean(&pool, rid, 1, ResyncReason::Expired);
}

// ===========================================================================
// 20. expiry_frees_and_retry_publishes
// ===========================================================================

#[test]
fn expiry_frees_and_retry_publishes() {
    let mut limits = TransferLimits::v1();
    limits.staging_bytes_per_receiver = 22_552;
    limits.staging_bytes_total = 22_552;
    let f1 = plan(1, 100, 0, 1, &max_body());
    let f2 = plan(2, 100, 0, 1, &max_body());
    let t = T0;
    let build = || {
        let mut pool = pool_with(limits.clone());
        let a = add(&mut pool, DeltaWire::V2, GateMode::Live);
        let b = add(&mut pool, DeltaWire::V2, GateMode::Live);
        assert_eq!(ok_t(&mut pool, a, &f1[0], t), TransferOutcome::Begun);
        ok_t(&mut pool, a, &f1[1], t);
        assert_eq!(pool.staging_bytes(), 22_552);
        (pool, a, b)
    };

    // The aggregate is full until A's deadline passes.
    let (mut pool, a, b) = build();
    assert_eq!(
        refuse_t(&mut pool, b, &f1[0], t + 4_999),
        TransferError::AggregateFull
    );
    // A call on B at t + 5,000 sweeps A first, so B's Begin fits at once.
    assert_eq!(
        ok_t(&mut pool, b, &f1[0], t + 5_000),
        TransferOutcome::Begun
    );
    assert_eq!(pool.staging_bytes(), 22_552); // B's, not A's and B's
    let sta = pool.status(a).unwrap();
    assert_eq!(sta.in_flight, None);
    assert_eq!(sta.resync, Some(ResyncReason::Expired));
    assert_eq!(sta.mode, GateMode::AwaitSnapshot);
    assert_eq!(sta.retired_through, 1);
    assert_eq!(sta.queued_updates, 0);
    assert!(pool.status(b).unwrap().in_flight.is_some());

    // A refused call on B is also a sweep point, and is the only change a
    // refused call may make: the pool equals a clone that ran `poll`.
    let (mut pool, a, b) = build();
    let before = pool.clone();
    let junk = chunk_frame(EPOCH, 9, 0, &[0, 0]);
    assert_eq!(
        pool.offer_transfer(b, &junk, t + 5_000),
        Err(TransferError::UnknownTransfer)
    );
    let mut model = before.clone();
    assert_eq!(model.poll(t + 5_000), vec![a]);
    assert_eq!(pool, model);
    // The aggregate room came back at once.
    assert_eq!(pool.staging_bytes(), 0);

    // A's resync is Expired and its Resync frame round-trips.
    let st = pool.status(a).unwrap();
    assert_eq!(st.resync, Some(ResyncReason::Expired));
    assert_eq!(st.in_flight, None);
    let resync = TransferUp::Resync {
        epoch: EPOCH,
        after: TransferId::new(st.retired_through),
        reason: st.resync.unwrap(),
    };
    assert_eq!(
        resync,
        TransferUp::Resync {
            epoch: EPOCH,
            after: Some(tid(1)),
            reason: ResyncReason::Expired
        }
    );
    assert_eq!(decode_up(&encode_up(&resync).unwrap()), Ok(resync));

    // A late chunk of the expired id is StaleTransfer.
    assert_eq!(
        refuse_t(&mut pool, a, &f1[2], t + 5_000),
        TransferError::StaleTransfer
    );
    // A's next id then publishes (cleanup and retry positive control).
    assert_eq!(
        ok_t(&mut pool, a, &f2[0], t + 5_000),
        TransferOutcome::Begun
    );
    for f in &f2[1..6] {
        ok_t(&mut pool, a, f, t + 5_000);
    }
    assert_eq!(
        ok_t(&mut pool, a, &f2[6], t + 5_000),
        TransferOutcome::Published {
            transfer: tid(2),
            released: 0
        }
    );
    let st = pool.status(a).unwrap();
    assert_eq!(st.resync, None);
    assert_eq!(st.mode, GateMode::Live);
    assert_eq!(pool.staging_bytes(), 0);
    let mut replica = Replica::default();
    assert_eq!(replica.drain(&mut pool, a), 1);
    assert_matches(&replica, &max_body_facts());
    // B was never affected.
    assert_eq!(pool.status(b).unwrap().resync, None);
}

// ===========================================================================
// 21. old_mixed_epoch_and_id_refusals
// ===========================================================================

#[test]
fn old_mixed_epoch_and_id_refusals() {
    let (mut pool, rid) = mkpool();
    // Transfer 1 publishes at cursor 5, base_tick 100: retired through 1 and
    // next_event_seq 6.
    let f1 = plan(1, 100, 5, 1, &small_body(1));
    assert!(matches!(
        offer_all(&mut pool, rid, &f1, T0),
        TransferOutcome::Published { .. }
    ));
    let st = pool.status(rid).unwrap();
    assert_eq!((st.retired_through, st.next_event_seq), (1, 6));

    // A Begin at or below retired_through is StaleTransfer.
    assert_eq!(
        refuse_t(&mut pool, rid, &f1[0], T0),
        TransferError::StaleTransfer
    );
    // A chunk for a retired id is Stale; for an id that never began, Unknown.
    assert_eq!(
        refuse_t(&mut pool, rid, &f1[1], T0),
        TransferError::StaleTransfer
    );
    let never = chunk_frame(EPOCH, 2, 0, &[0, 0]);
    assert_eq!(
        refuse_t(&mut pool, rid, &never, T0),
        TransferError::UnknownTransfer
    );
    let never = chunk_frame(EPOCH, 999, 0, &[0, 0]);
    assert_eq!(
        refuse_t(&mut pool, rid, &never, T0),
        TransferError::UnknownTransfer
    );

    // BehindReplica: cursor + 1 < next_event_seq; base_tick below published.
    let behind_cursor = plan(2, 100, 4, 1, &small_body(2));
    assert_eq!(
        refuse_t(&mut pool, rid, &behind_cursor[0], T0),
        TransferError::BehindReplica
    );
    let behind_tick = plan(2, 99, 5, 1, &small_body(2));
    assert_eq!(
        refuse_t(&mut pool, rid, &behind_tick[0], T0),
        TransferError::BehindReplica
    );
    // Both at once is also BehindReplica; still nothing changed.
    let behind_both = plan(2, 99, 0, 1, &small_body(2));
    assert_eq!(
        refuse_t(&mut pool, rid, &behind_both[0], T0),
        TransferError::BehindReplica
    );
    // An equal base_tick (and cursor + 1 == next_event_seq) is accepted.
    let f2 = plan(2, 100, 5, 1, &small_body(2));
    assert_eq!(ok_t(&mut pool, rid, &f2[0], T0), TransferOutcome::Begun);

    // A wrong-epoch Begin, chunk and Cancel are each WrongEpoch with the pool
    // unchanged, and the in-flight transfer still publishes.
    let foreign_begin = {
        let b = begin_of(OTHER_EPOCH, 3, 100, 5, 2, 0, 1);
        begin_frame(&b)
    };
    assert_eq!(
        refuse_t(&mut pool, rid, &foreign_begin, T0),
        TransferError::WrongEpoch
    );
    let foreign_chunk = chunk_frame(OTHER_EPOCH, 2, 0, &[0, 0]);
    assert_eq!(
        refuse_t(&mut pool, rid, &foreign_chunk, T0),
        TransferError::WrongEpoch
    );
    let foreign_cancel = cancel_frame(OTHER_EPOCH, 2, CancelReason::Aborted);
    assert_eq!(
        refuse_t(&mut pool, rid, &foreign_cancel, T0),
        TransferError::WrongEpoch
    );
    // WrongEpoch outranks a stale id on the same frame.
    let foreign_stale = cancel_frame(OTHER_EPOCH, 1, CancelReason::Superseded);
    assert_eq!(
        refuse_t(&mut pool, rid, &foreign_stale, T0),
        TransferError::WrongEpoch
    );
    let st = pool.status(rid).unwrap();
    assert_eq!(st.in_flight.unwrap().transfer, tid(2));
    assert_eq!(st.retired_through, 1);

    assert!(matches!(
        ok_t(&mut pool, rid, &f2[1], T0),
        TransferOutcome::Published { .. }
    ));
    // Publication replaced the queued snapshot of transfer 1.
    let mut replica = Replica::default();
    assert_eq!(replica.drain(&mut pool, rid), 1);
    assert_matches(&replica, &small_facts(2));
}

// ===========================================================================
// 22. invalid_body_never_publishes
// ===========================================================================

#[test]
fn invalid_body_never_publishes() {
    // A receiver with the earlier snapshot (transfer 1) queued, not yet popped.
    let with_earlier = || {
        let (mut pool, rid) = mkpool();
        let f1 = plan(1, 100, 0, 1, &small_body(10));
        assert!(matches!(
            offer_all(&mut pool, rid, &f1, T0),
            TransferOutcome::Published { .. }
        ));
        assert_eq!(pool.status(rid).unwrap().queued_updates, 1);
        (pool, rid)
    };
    let valid_payload = encode_body(&max_body()).unwrap();
    let mut trailing = valid_payload.clone();
    trailing.push(0x00);
    // (name, frames, expected error from the final chunk)
    type Case = (&'static str, Vec<Vec<u8>>, Option<CodecError>);
    let cases: Vec<Case> = vec![
        (
            "1 MiB opaque payload over 32 chunks",
            payload_frames(EPOCH, 2, 100, 0, 8, 0, &pattern(MIB)),
            None,
        ),
        (
            "maximal body under entity_count 1,023",
            payload_frames(EPOCH, 2, 100, 0, 1, 1023, &valid_payload),
            Some(CodecError::Malformed),
        ),
        (
            "body with a trailing byte",
            payload_frames(EPOCH, 2, 100, 0, 1, 1024, &trailing),
            Some(CodecError::Malformed),
        ),
    ];
    for (name, frames, want) in cases {
        let (mut pool, rid) = with_earlier();
        let chunks = frames.len() - 1;
        if name.starts_with("1 MiB") {
            assert_eq!(chunks, 32);
        }
        assert_eq!(
            ok_t(&mut pool, rid, &frames[0], T0),
            TransferOutcome::Begun,
            "{name}"
        );
        for f in &frames[1..frames.len() - 1] {
            assert!(
                matches!(ok_t(&mut pool, rid, f, T0), TransferOutcome::Stored { .. }),
                "{name}"
            );
        }
        // Assembly completes on the last chunk, which fails the body.
        match fail_t(&mut pool, rid, frames.last().unwrap(), T0) {
            FailReason::InvalidBody(e) => {
                if let Some(w) = want {
                    assert_eq!(e, w, "{name}");
                }
            }
            other => panic!("{name}: expected InvalidBody, got {other:?}"),
        }
        // Staging returns to 0, the id is retired, nothing new was queued.
        assert_failed_clean(&pool, rid, 2, ResyncReason::Invalid);
        let st = pool.status(rid).unwrap();
        assert_eq!(st.queued_updates, 1, "{name}: only the earlier snapshot");
        assert_eq!(st.published.unwrap().transfer, tid(1), "{name}");
        assert_eq!(st.next_event_seq, 1);
        // Later frames of the failed id are stale.
        assert_eq!(
            refuse_t(&mut pool, rid, frames.last().unwrap(), T0),
            TransferError::StaleTransfer,
            "{name}"
        );
        // The earlier snapshot is the only queued update and the oracle state.
        let mut replica = Replica::default();
        assert_eq!(replica.drain(&mut pool, rid), 1, "{name}");
        assert_eq!(replica.order, vec!["S"]);
        assert_matches(&replica, &small_facts(10));

        // Recovery: a valid snapshot under a higher id publishes, and it too
        // supersedes the queue (here the queue is already empty).
        let f3 = plan(3, 100, 0, 1, &max_body());
        assert!(matches!(
            offer_all(&mut pool, rid, &f3, T0),
            TransferOutcome::Published { .. }
        ));
        assert_eq!(replica.drain(&mut pool, rid), 1);
        assert_matches(&replica, &max_body_facts());
    }

    // While the earlier snapshot is still queued, the next valid publication
    // replaces it: at most one queued snapshot (S§10.6 step 3).
    let (mut pool, rid) = with_earlier();
    let bad = payload_frames(EPOCH, 2, 100, 0, 1, 1023, &valid_payload);
    offer_all(&mut pool, rid, &bad[..bad.len() - 1], T0);
    fail_t(&mut pool, rid, bad.last().unwrap(), T0);
    let f3 = plan(3, 100, 0, 1, &max_body());
    assert!(matches!(
        offer_all(&mut pool, rid, &f3, T0),
        TransferOutcome::Published { .. }
    ));
    assert_eq!(pool.status(rid).unwrap().queued_updates, 1);
    let mut replica = Replica::default();
    assert_eq!(replica.drain(&mut pool, rid), 1);
    assert_matches(&replica, &max_body_facts());
}
// ===========================================================================
// 23. cutover_releases_in_fifo_order
// ===========================================================================

/// Replays updates, in the given order, into a fresh oracle.
fn replay(updates: &[ReplicaUpdate]) -> Replica {
    let mut r = Replica::default();
    for u in updates {
        r.apply(u);
    }
    r
}

#[test]
fn cutover_releases_in_fifo_order() {
    let (mut pool, rid) = mkpool(); // V2, Live
                                    // The delta table: event_seq 1..=7. D1..D5 apply to the old base, D6 and
                                    // D7 to the new one.
    let deltas: Vec<Vec<u8>> = vec![
        v2_frame(1, vec![enter(1)]),
        v2_frame(2, vec![enter(2), enter(3)]),
        v2_frame(3, vec![health_op(1, 10, 10, false)]),
        v2_frame(4, vec![health_op(2, 8, 10, false)]),
        v2_frame(5, vec![enter(3)]), // a re-enter forgets what was known of 3
        v2_frame(6, vec![turn_op(Some(2), 2)]),
        v2_frame(7, vec![health_op(1, 3, 10, false)]),
    ];
    let delta = |seq: usize| deltas[seq - 1].as_slice();
    // The snapshot is captured at cursor 5 and includes D1..D5.
    let snapshot = SnapshotBody {
        entities: vec![
            SnapshotEntity {
                entity: net(1),
                health: Some(health(10, 10, false)),
            },
            SnapshotEntity {
                entity: net(2),
                health: Some(health(8, 10, false)),
            },
            SnapshotEntity {
                entity: net(3),
                health: Some(health(7, 7, false)),
            },
        ],
        turn: Some(TurnFact {
            active: Some(net(1)),
            round: 1,
        }),
    };
    let frames = plan(1, 100, 5, 1, &snapshot);
    assert_eq!(frames.len(), 2);

    // Deltas 1..=3 are released.
    for seq in 1..=3 {
        assert_eq!(
            ok_d(&mut pool, rid, delta(seq), T0),
            DeltaOutcome::Released { count: 1 }
        );
    }
    let st = pool.status(rid).unwrap();
    assert_eq!(
        (st.next_event_seq, st.queued_updates, st.held_frames),
        (4, 3, 3)
    );

    // Begin(cursor 5); delta 5 is Buffered, then delta 4 releases both.
    assert_eq!(ok_t(&mut pool, rid, &frames[0], T0), TransferOutcome::Begun);
    assert_eq!(ok_d(&mut pool, rid, delta(5), T0), DeltaOutcome::Buffered);
    assert_eq!(pool.status(rid).unwrap().queued_updates, 3);
    assert_eq!(
        ok_d(&mut pool, rid, delta(4), T0),
        DeltaOutcome::Released { count: 2 }
    );
    let st = pool.status(rid).unwrap();
    assert_eq!((st.next_event_seq, st.queued_updates), (6, 5));
    // Deltas 6 and 7 are above the cursor: Buffered, never queued before the
    // snapshot.
    assert_eq!(ok_d(&mut pool, rid, delta(6), T0), DeltaOutcome::Buffered);
    assert_eq!(ok_d(&mut pool, rid, delta(7), T0), DeltaOutcome::Buffered);
    let st = pool.status(rid).unwrap();
    assert_eq!((st.queued_updates, st.held_frames), (5, 7));
    assert_eq!(st.in_flight.unwrap().cursor, 5);

    // The last chunk is Published { released: 2 }.
    assert_eq!(
        ok_t(&mut pool, rid, &frames[1], T0),
        TransferOutcome::Published {
            transfer: tid(1),
            released: 2
        }
    );
    let st = pool.status(rid).unwrap();
    assert_eq!(st.next_event_seq, 8);
    assert_eq!(st.mode, GateMode::Live);
    assert_eq!(st.queued_updates, 8, "D1..D5, Snapshot, D6, D7");
    // Repeats of 5 and 6 are Superseded (and change nothing).
    let before = pool.clone();
    assert_eq!(ok_d(&mut pool, rid, delta(5), T0), DeltaOutcome::Superseded);
    assert_eq!(ok_d(&mut pool, rid, delta(6), T0), DeltaOutcome::Superseded);
    assert_eq!(pool, before);

    let mut full = Vec::new();
    while let Some(u) = pool.pop_update(rid) {
        full.push(u);
    }
    assert_eq!(full.len(), 8);

    // The oracle, applying in pop order, equals the expected view: first the
    // old base after D1..D5, then the cut over to the new base.
    let pre = replay(&full[..5]);
    assert_eq!(pre.order, vec!["D1", "D2", "D3", "D4", "D5"]);
    assert_matches(
        &pre,
        &facts(
            &[
                (1, Some((10, 10, false))),
                (2, Some((8, 10, false))),
                (3, None),
            ],
            None,
        ),
    );
    let at_snapshot = replay(&full[..6]);
    assert_matches(
        &at_snapshot,
        &facts(
            &[
                (1, Some((10, 10, false))),
                (2, Some((8, 10, false))),
                (3, Some((7, 7, false))),
            ],
            Some((Some(1), 1)),
        ),
    );
    let want = facts(
        &[
            (1, Some((3, 10, false))),
            (2, Some((8, 10, false))),
            (3, Some((7, 7, false))),
        ],
        Some((Some(2), 2)),
    );
    let good = replay(&full);
    assert_eq!(
        good.order,
        vec!["D1", "D2", "D3", "D4", "D5", "S", "D6", "D7"]
    );
    assert_eq!(good.snapshots, 1);
    assert_matches(&good, &want);

    // Negative control 1: applying D5 after the snapshot fails the oracle
    // (the re-enter forgets entity 3's health).
    let mut late_d5 = full.clone();
    late_d5.swap(4, 5);
    let bad = replay(&late_d5);
    assert_eq!(
        bad.order,
        vec!["D1", "D2", "D3", "D4", "S", "D5", "D6", "D7"]
    );
    assert_eq!(
        diff(&bad.facts, &want),
        vec!["missing health of entity 3".to_string()]
    );
    // Negative control 2: dropping D6 fails the oracle (the turn is stale).
    let mut no_d6 = full.clone();
    no_d6.remove(6);
    let bad = replay(&no_d6);
    assert_eq!(bad.order, vec!["D1", "D2", "D3", "D4", "D5", "S", "D7"]);
    let d = diff(&bad.facts, &want);
    assert_eq!(d.len(), 1);
    assert!(d[0].starts_with("mismatch turn"), "{d:?}");
    // Negative control 3: dropping the snapshot fails it too.
    let mut no_s = full.clone();
    no_s.remove(5);
    assert!(!diff(&replay(&no_s).facts, &want).is_empty());
}

// ===========================================================================
// 24. await_snapshot_holds_until_publication
// ===========================================================================

#[test]
fn await_snapshot_holds_until_publication() {
    let mut pool = pool_with(TransferLimits::v1());
    let rid = add(&mut pool, DeltaWire::V2, GateMode::AwaitSnapshot);
    let deltas: Vec<Vec<u8>> = vec![
        v2_frame(1, vec![enter(1)]),
        v2_frame(2, vec![enter(2)]),
        v2_frame(3, vec![health_op(2, 4, 10, false)]),
        v2_frame(4, vec![enter(3)]),
        v2_frame(5, vec![health_op(3, 6, 6, false), leave(1)]),
    ];
    // An AwaitSnapshot receiver holds deltas 1..=3 and queues nothing.
    for seq in 1..=3usize {
        assert_eq!(
            ok_d(&mut pool, rid, &deltas[seq - 1], T0),
            DeltaOutcome::Buffered
        );
    }
    let st = pool.status(rid).unwrap();
    assert_eq!(st.queued_updates, 0);
    assert_eq!(st.held_frames, 3);
    assert_eq!(st.mode, GateMode::AwaitSnapshot);
    assert_eq!(st.next_event_seq, 1);
    assert_eq!(pool.pop_update(rid), None);

    // Begin(cursor 2): still nothing is released while the transfer is open.
    let snapshot = SnapshotBody {
        entities: vec![
            SnapshotEntity {
                entity: net(1),
                health: Some(health(10, 10, false)),
            },
            SnapshotEntity {
                entity: net(2),
                health: Some(health(9, 10, false)),
            },
        ],
        turn: None,
    };
    let frames = plan(1, 100, 2, 1, &snapshot);
    assert_eq!(ok_t(&mut pool, rid, &frames[0], T0), TransferOutcome::Begun);
    assert_eq!(pool.status(rid).unwrap().queued_updates, 0);
    assert_eq!(pool.pop_update(rid), None);
    // The last chunk publishes: the queue is exactly Snapshot, D3.
    assert_eq!(
        ok_t(&mut pool, rid, &frames[1], T0),
        TransferOutcome::Published {
            transfer: tid(1),
            released: 1
        }
    );
    let st = pool.status(rid).unwrap();
    assert_eq!(st.queued_updates, 2);
    assert_eq!(st.mode, GateMode::Live);
    assert_eq!(st.next_event_seq, 4);
    assert_eq!(st.held_frames, 1); // D1 and D2 were dropped; D3 is queued
    let mut replica = Replica::default();
    assert_eq!(replica.drain(&mut pool, rid), 2);
    assert_eq!(replica.order, vec!["S", "D3"]);
    assert_matches(
        &replica,
        &facts(
            &[(1, Some((10, 10, false))), (2, Some((4, 10, false)))],
            None,
        ),
    );
    assert_eq!(pool.pop_update(rid), None);
    // D1 and D2 are behind the snapshot: Superseded, never applied.
    assert_eq!(
        ok_d(&mut pool, rid, &deltas[0], T0),
        DeltaOutcome::Superseded
    );
    assert_eq!(
        ok_d(&mut pool, rid, &deltas[1], T0),
        DeltaOutcome::Superseded
    );
    assert_eq!(
        ok_d(&mut pool, rid, &deltas[2], T0),
        DeltaOutcome::Superseded
    );

    // Delta 5 without 4 stays held, and delta 4 is then Released { count: 2 }.
    assert_eq!(ok_d(&mut pool, rid, &deltas[4], T0), DeltaOutcome::Buffered);
    assert_eq!(pool.pop_update(rid), None);
    let st = pool.status(rid).unwrap();
    assert_eq!(
        (st.held_frames, st.queued_updates, st.next_event_seq),
        (1, 0, 4)
    );
    assert_eq!(
        ok_d(&mut pool, rid, &deltas[3], T0),
        DeltaOutcome::Released { count: 2 }
    );
    assert_eq!(replica.drain(&mut pool, rid), 2);
    assert_eq!(replica.order, vec!["S", "D3", "D4", "D5"]);
    assert_matches(
        &replica,
        &facts(&[(2, Some((4, 10, false))), (3, Some((6, 6, false)))], None),
    );
}

// ===========================================================================
// 25. delta_hold_caps_and_overflow
// ===========================================================================

/// A V2 receiver with transfer 1 in flight at cursor 0, so every delta waits.
fn in_flight_pool(limits: TransferLimits) -> (ReassemblyPool, ReceiverId, Vec<Vec<u8>>) {
    let mut pool = pool_with(limits);
    let rid = add(&mut pool, DeltaWire::V2, GateMode::Live);
    let frames = plan(1, 100, 0, 1, &small_body(5));
    assert_eq!(ok_t(&mut pool, rid, &frames[0], T0), TransferOutcome::Begun);
    (pool, rid, frames)
}

fn assert_overflowed(pool: &ReassemblyPool, rid: ReceiverId) {
    let st = pool.status(rid).unwrap();
    assert_eq!(st.resync, Some(ResyncReason::Overflow));
    assert_eq!(st.mode, GateMode::AwaitSnapshot);
    assert_eq!(st.in_flight, None);
    assert_eq!(st.held_frames, 0);
    assert_eq!(st.held_bytes, 0);
    assert_eq!(st.queued_updates, 0);
    assert_eq!(pool.staging_bytes(), 0);
}

#[test]
fn delta_hold_caps_and_overflow() {
    // 128 held frames fit and the 129th is Failed(Overflow).
    let (mut pool, rid, f1) = in_flight_pool(TransferLimits::v1());
    let mut bytes = 0;
    for seq in 1..=128u64 {
        let f = v2_frame(seq, vec![]);
        bytes += f.len();
        assert_eq!(
            ok_d(&mut pool, rid, &f, T0),
            DeltaOutcome::Buffered,
            "seq {seq}"
        );
    }
    let st = pool.status(rid).unwrap();
    assert_eq!(st.held_frames, 128);
    assert_eq!(st.held_bytes, bytes);
    assert_eq!(pool.held_delta_bytes(), bytes);
    assert_eq!(st.queued_updates, 0);
    let d129 = v2_frame(129, vec![]);
    assert_eq!(fail_d(&mut pool, rid, &d129, T0), FailReason::Overflow);
    // It discards and retires the in-flight transfer, clears held deltas,
    // sets resync to Overflow and the mode to AwaitSnapshot.
    assert_overflowed(&pool, rid);
    let st = pool.status(rid).unwrap();
    assert_eq!(st.retired_through, 1);
    assert_eq!(st.next_event_seq, 1);
    assert_eq!(pool.held_delta_bytes(), 0);
    // Chunks of the discarded transfer are stale.
    assert_eq!(
        refuse_t(&mut pool, rid, &f1[1], T0),
        TransferError::StaleTransfer
    );

    // A new snapshot then publishes and later deltas flow ...
    let snap = |id: u64, cursor: u64| plan(id, 100, cursor, 1, &small_body(7));
    // (Dropped delta 129 is covered when the snapshot is captured at 129.)
    let covering = snap(2, 129);
    assert_eq!(
        ok_t(&mut pool, rid, &covering[0], T0),
        TransferOutcome::Begun
    );
    assert_eq!(pool.status(rid).unwrap().resync, None);
    assert!(matches!(
        ok_t(&mut pool, rid, &covering[1], T0),
        TransferOutcome::Published { released: 0, .. }
    ));
    let st = pool.status(rid).unwrap();
    assert_eq!((st.mode, st.next_event_seq), (GateMode::Live, 130));
    assert_eq!(
        ok_d(&mut pool, rid, &v2_frame(130, vec![enter(1)]), T0),
        DeltaOutcome::Released { count: 1 }
    );
    // ... and the old held deltas and the dropped delta are Superseded.
    assert_eq!(
        ok_d(&mut pool, rid, &v2_frame(5, vec![]), T0),
        DeltaOutcome::Superseded
    );
    assert_eq!(ok_d(&mut pool, rid, &d129, T0), DeltaOutcome::Superseded);
    let mut replica = Replica::default();
    assert_eq!(replica.drain(&mut pool, rid), 2);
    assert_eq!(replica.order, vec!["S", "D130"]);

    // ... but the gap left by the dropped delta is never released: a snapshot
    // captured at 128 leaves 129 missing, so 130 and 131 stay held forever.
    let (mut pool, rid, _) = in_flight_pool(TransferLimits::v1());
    for seq in 1..=128u64 {
        ok_d(&mut pool, rid, &v2_frame(seq, vec![]), T0);
    }
    fail_d(&mut pool, rid, &d129, T0);
    assert_overflowed(&pool, rid);
    let gapped = snap(2, 128);
    offer_all(&mut pool, rid, &gapped, T0);
    assert_eq!(pool.status(rid).unwrap().next_event_seq, 129);
    for seq in [130u64, 131] {
        assert_eq!(
            ok_d(&mut pool, rid, &v2_frame(seq, vec![enter(1)]), T0),
            DeltaOutcome::Buffered
        );
    }
    let mut replica = Replica::default();
    assert_eq!(replica.drain(&mut pool, rid), 1, "only the snapshot");
    let st = pool.status(rid).unwrap();
    assert_eq!(
        (st.held_frames, st.queued_updates, st.next_event_seq),
        (2, 0, 129)
    );
    assert_eq!(pool.poll(T0 + 60_000), Vec::<ReceiverId>::new());
    assert_eq!(pool.pop_update(rid), None);

    // delta_bytes_per_receiver 65,536: an exact fit is accepted and one byte
    // more overflows. Frames stay under the 128-frame cap, so bytes decide.
    let mut limits = TransferLimits::v1();
    limits.delta_bytes_per_receiver = 65_536;
    let exact = frames_summing(1, 65_536);
    let over = frames_summing(1, 65_537);
    assert!(over.len() <= 128);
    assert_eq!(exact[..exact.len() - 1], over[..over.len() - 1]);
    {
        let mut pool = pool_with(limits.clone());
        let rid = add(&mut pool, DeltaWire::V2, GateMode::AwaitSnapshot);
        for f in &exact {
            assert_eq!(ok_d(&mut pool, rid, f, T0), DeltaOutcome::Buffered);
        }
        let st = pool.status(rid).unwrap();
        assert_eq!(st.held_bytes, 65_536);
        assert_eq!(st.held_frames, exact.len());
        assert_eq!(pool.held_delta_bytes(), 65_536);
        assert_eq!(st.resync, None);
    }
    {
        let mut pool = pool_with(limits.clone());
        let rid = add(&mut pool, DeltaWire::V2, GateMode::AwaitSnapshot);
        for f in &over[..over.len() - 1] {
            assert_eq!(ok_d(&mut pool, rid, f, T0), DeltaOutcome::Buffered);
        }
        // Exactly one byte of room is missing for the last frame.
        let before_last: usize = over[..over.len() - 1].iter().map(Vec::len).sum();
        assert_eq!(pool.status(rid).unwrap().held_bytes, before_last);
        assert_eq!(before_last + over.last().unwrap().len(), 65_537);
        assert_eq!(over.last().unwrap().len(), exact.last().unwrap().len() + 1);
        assert_eq!(
            fail_d(&mut pool, rid, over.last().unwrap(), T0),
            FailReason::Overflow
        );
        assert_overflowed(&pool, rid);
        assert_eq!(pool.held_delta_bytes(), 0);
    }
    {
        // Positive control: the limit, not the frames, is what overflowed.
        let mut pool = pool_with(TransferLimits::v1());
        let rid = add(&mut pool, DeltaWire::V2, GateMode::AwaitSnapshot);
        for f in &over {
            assert_eq!(ok_d(&mut pool, rid, f, T0), DeltaOutcome::Buffered);
        }
        assert_eq!(pool.status(rid).unwrap().held_bytes, 65_537);
    }

    // The same across two receivers against the pool total.
    let mut limits = TransferLimits::v1();
    limits.delta_bytes_per_receiver = 65_536;
    limits.delta_bytes_total = 65_536;
    let half = frames_summing(1, 32_768);
    let half_plus = frames_summing(1, 32_769);
    {
        let mut pool = pool_with(limits.clone());
        let r1 = add(&mut pool, DeltaWire::V2, GateMode::AwaitSnapshot);
        let r2 = add(&mut pool, DeltaWire::V2, GateMode::AwaitSnapshot);
        for f in &half {
            ok_d(&mut pool, r1, f, T0);
        }
        for f in &half {
            assert_eq!(ok_d(&mut pool, r2, f, T0), DeltaOutcome::Buffered);
        }
        assert_eq!(pool.held_delta_bytes(), 65_536);
        assert_eq!(pool.status(r1).unwrap().held_bytes, 32_768);
        assert_eq!(pool.status(r2).unwrap().held_bytes, 32_768);
    }
    {
        let mut pool = pool_with(limits.clone());
        let r1 = add(&mut pool, DeltaWire::V2, GateMode::AwaitSnapshot);
        let r2 = add(&mut pool, DeltaWire::V2, GateMode::AwaitSnapshot);
        for f in &half {
            ok_d(&mut pool, r1, f, T0);
        }
        let r1_before = pool.status(r1).unwrap();
        for f in &half_plus[..half_plus.len() - 1] {
            assert_eq!(ok_d(&mut pool, r2, f, T0), DeltaOutcome::Buffered);
        }
        assert_eq!(
            fail_d(&mut pool, r2, half_plus.last().unwrap(), T0),
            FailReason::Overflow
        );
        assert_overflowed(&pool, r2);
        // Receiver 2 alone was far under its own cap; receiver 1 is untouched.
        assert_eq!(pool.status(r1).unwrap(), r1_before);
        assert_eq!(pool.held_delta_bytes(), 32_768);
    }
    {
        let mut pool = pool_with(TransferLimits::v1());
        let r1 = add(&mut pool, DeltaWire::V2, GateMode::AwaitSnapshot);
        let r2 = add(&mut pool, DeltaWire::V2, GateMode::AwaitSnapshot);
        for f in &half {
            ok_d(&mut pool, r1, f, T0);
        }
        for f in &half_plus {
            assert_eq!(ok_d(&mut pool, r2, f, T0), DeltaOutcome::Buffered);
        }
    }

    // A conflicting duplicate of a held delta is Failed(Conflict); an
    // identical duplicate is Duplicate.
    let (mut pool, rid, f1) = in_flight_pool(TransferLimits::v1());
    let held = v2_frame(3, vec![enter(1)]);
    assert_eq!(ok_d(&mut pool, rid, &held, T0), DeltaOutcome::Buffered);
    assert_eq!(ok_d(&mut pool, rid, &held, T0), DeltaOutcome::Duplicate);
    assert_eq!(pool.status(rid).unwrap().held_frames, 1);
    let conflicting = v2_frame(3, vec![enter(2)]);
    assert_eq!(
        fail_d(&mut pool, rid, &conflicting, T0),
        FailReason::Conflict
    );
    assert_failed_clean(&pool, rid, 1, ResyncReason::Invalid);
    assert_eq!(
        refuse_t(&mut pool, rid, &f1[1], T0),
        TransferError::StaleTransfer
    );
}

// ===========================================================================
// 26. delta_sequence_and_wire_refusals
// ===========================================================================

#[test]
fn delta_sequence_and_wire_refusals() {
    // A V2 receiver.
    let (mut pool, rid) = mkpool();
    let v2 = |seq: u64| v2_frame(seq, vec![enter(1)]);
    let v1 = |seq: u64| v1_frame(seq, vec![enter(1)]);
    // event_seq 0 and u64::MAX are DeltaSequence, each leaving the pool alone.
    assert_eq!(
        refuse_d(&mut pool, rid, &v2(0), T0),
        TransferError::DeltaSequence
    );
    assert_eq!(
        refuse_d(&mut pool, rid, &v2(u64::MAX), T0),
        TransferError::DeltaSequence
    );
    // SEQ_LAST is held (Buffered), not refused.
    assert_eq!(SEQ_LAST, u64::MAX - 1);
    assert_eq!(
        ok_d(&mut pool, rid, &v2(SEQ_LAST), T0),
        DeltaOutcome::Buffered
    );
    let st = pool.status(rid).unwrap();
    assert_eq!(
        (st.held_frames, st.queued_updates, st.next_event_seq),
        (1, 0, 1)
    );
    // Positive control: sequence 1 flows.
    assert_eq!(
        ok_d(&mut pool, rid, &v2(1), T0),
        DeltaOutcome::Released { count: 1 }
    );

    // A v1 frame to a V2 receiver is Codec(UnsupportedVersion).
    assert_eq!(
        refuse_d(&mut pool, rid, &v1(2), T0),
        TransferError::Codec(CodecError::UnsupportedVersion)
    );
    // Garbage and truncation are Codec errors with nothing changed.
    assert_eq!(
        refuse_d(&mut pool, rid, &[], T0),
        TransferError::Codec(CodecError::Malformed)
    );
    assert_eq!(
        refuse_d(&mut pool, rid, &v2(2)[..3], T0),
        TransferError::Codec(CodecError::Malformed)
    );
    // The v2 twin of the refused v1 frame is accepted.
    assert_eq!(
        ok_d(&mut pool, rid, &v2(2), T0),
        DeltaOutcome::Released { count: 1 }
    );

    // A V1 receiver mirrors every rule, and refuses v2 frames.
    let mut pool = pool_with(TransferLimits::v1());
    let rid = add(&mut pool, DeltaWire::V1, GateMode::Live);
    assert_eq!(pool.status(rid).unwrap().wire, DeltaWire::V1);
    assert_eq!(
        refuse_d(&mut pool, rid, &v1(0), T0),
        TransferError::DeltaSequence
    );
    assert_eq!(
        refuse_d(&mut pool, rid, &v1(u64::MAX), T0),
        TransferError::DeltaSequence
    );
    assert_eq!(
        ok_d(&mut pool, rid, &v1(SEQ_LAST), T0),
        DeltaOutcome::Buffered
    );
    assert_eq!(
        refuse_d(&mut pool, rid, &v2(1), T0),
        TransferError::Codec(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        refuse_d(&mut pool, rid, &v2(2), T0),
        TransferError::Codec(CodecError::UnsupportedVersion)
    );
    // The v1 twin is accepted, and queues as a v1 delta.
    assert_eq!(
        ok_d(&mut pool, rid, &v1(1), T0),
        DeltaOutcome::Released { count: 1 }
    );
    let mut replica = Replica::default();
    assert_eq!(replica.drain(&mut pool, rid), 1);
    assert_eq!(replica.order, vec!["D1"]);
    assert_matches(&replica, &facts(&[(1, None)], None));

    // Transfer frames are not deltas: a Begin offered as a delta is a codec
    // refusal, on either wire.
    let begin = plan(1, 100, 0, 1, &small_body(1));
    assert_eq!(
        refuse_d(&mut pool, rid, &begin[0], T0),
        TransferError::Codec(CodecError::UnsupportedVersion)
    );
}

// ===========================================================================
// 27. cancel_retires_and_supersedes
// ===========================================================================

#[test]
fn cancel_retires_and_supersedes() {
    let frames = big_fixture().frames(1, 100, 0, 1);
    // Cancel of the in-flight transfer.
    let (mut pool, rid) = mkpool();
    assert_eq!(ok_t(&mut pool, rid, &frames[0], T0), TransferOutcome::Begun);
    ok_t(&mut pool, rid, &frames[1], T0);
    ok_t(&mut pool, rid, &frames[2], T0);
    assert_eq!(pool.staging_bytes(), BIG_FIXTURE_BYTES);
    assert_eq!(pool.status(rid).unwrap().mode, GateMode::Live);
    let cancel = cancel_frame(EPOCH, 1, CancelReason::Superseded);
    assert_eq!(
        ok_t(&mut pool, rid, &cancel, T0),
        TransferOutcome::Cancelled { discarded: true }
    );
    let st = pool.status(rid).unwrap();
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(st.mode, GateMode::AwaitSnapshot);
    assert_eq!(st.resync, None, "the host started the cancel");
    assert_eq!(st.retired_through, 1);
    assert_eq!(st.in_flight, None);
    assert_eq!(st.queued_updates, 0);
    // Its remaining chunks are StaleTransfer.
    for f in &frames[3..] {
        assert_eq!(
            refuse_t(&mut pool, rid, f, T0),
            TransferError::StaleTransfer
        );
    }
    // A repeat is Cancelled { discarded: false } and an idempotent no-op.
    let before = pool.clone();
    assert_eq!(
        ok_t(&mut pool, rid, &cancel, T0),
        TransferOutcome::Cancelled { discarded: false }
    );
    assert_eq!(pool, before);
    // The reasons are cause-neutral: Aborted behaves the same.
    let (mut pool2, rid2) = mkpool();
    ok_t(&mut pool2, rid2, &frames[0], T0);
    assert_eq!(
        ok_t(
            &mut pool2,
            rid2,
            &cancel_frame(EPOCH, 1, CancelReason::Aborted),
            T0
        ),
        TransferOutcome::Cancelled { discarded: true }
    );

    // Cancel(9) with nothing in flight retires through 9; mode untouched.
    let (mut pool, rid) = mkpool();
    assert_eq!(
        ok_t(
            &mut pool,
            rid,
            &cancel_frame(EPOCH, 9, CancelReason::Superseded),
            T0
        ),
        TransferOutcome::Cancelled { discarded: false }
    );
    let st = pool.status(rid).unwrap();
    assert_eq!(
        (st.retired_through, st.mode, st.resync),
        (9, GateMode::Live, None)
    );
    let begin = |id: u64| plan(id, 100, 0, 1, &small_body(id as u32));
    assert_eq!(
        refuse_t(&mut pool, rid, &begin(9)[0], T0),
        TransferError::StaleTransfer
    );
    assert_eq!(
        refuse_t(&mut pool, rid, &begin(1)[0], T0),
        TransferError::StaleTransfer
    );
    // Begin(10) publishes.
    assert!(matches!(
        offer_all(&mut pool, rid, &begin(10), T0),
        TransferOutcome::Published { transfer, .. } if transfer == tid(10)
    ));
    // A lower Cancel never lowers the high-water mark.
    assert_eq!(
        ok_t(
            &mut pool,
            rid,
            &cancel_frame(EPOCH, 3, CancelReason::Superseded),
            T0
        ),
        TransferOutcome::Cancelled { discarded: false }
    );
    assert_eq!(pool.status(rid).unwrap().retired_through, 10);

    // A Cancel naming a lower id than the in-flight one discards nothing.
    let (mut pool, rid) = mkpool();
    let f5 = plan(5, 100, 0, 1, &small_body(5));
    ok_t(&mut pool, rid, &f5[0], T0);
    assert_eq!(
        ok_t(
            &mut pool,
            rid,
            &cancel_frame(EPOCH, 3, CancelReason::Superseded),
            T0
        ),
        TransferOutcome::Cancelled { discarded: false }
    );
    let st = pool.status(rid).unwrap();
    assert_eq!((st.retired_through, st.mode), (3, GateMode::Live));
    assert!(st.in_flight.is_some());
    assert!(matches!(
        ok_t(&mut pool, rid, &f5[1], T0),
        TransferOutcome::Published { .. }
    ));
    // A Cancel naming a higher id than the in-flight one discards it and
    // retires through the higher id.
    let (mut pool, rid) = mkpool();
    ok_t(&mut pool, rid, &f5[0], T0);
    assert_eq!(
        ok_t(
            &mut pool,
            rid,
            &cancel_frame(EPOCH, 7, CancelReason::Superseded),
            T0
        ),
        TransferOutcome::Cancelled { discarded: true }
    );
    let st = pool.status(rid).unwrap();
    assert_eq!((st.retired_through, st.mode), (7, GateMode::AwaitSnapshot));
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(
        refuse_t(&mut pool, rid, &f5[1], T0),
        TransferError::StaleTransfer
    );

    // Cancelling an already-published id leaves its queued snapshot in place.
    let (mut pool, rid) = mkpool();
    let f1 = plan(1, 100, 0, 1, &small_body(1));
    assert!(matches!(
        offer_all(&mut pool, rid, &f1, T0),
        TransferOutcome::Published { .. }
    ));
    assert_eq!(pool.status(rid).unwrap().queued_updates, 1);
    assert_eq!(
        ok_t(
            &mut pool,
            rid,
            &cancel_frame(EPOCH, 1, CancelReason::Aborted),
            T0
        ),
        TransferOutcome::Cancelled { discarded: false }
    );
    let st = pool.status(rid).unwrap();
    assert_eq!(st.queued_updates, 1);
    assert_eq!(st.mode, GateMode::Live);
    assert_eq!(st.published.unwrap().transfer, tid(1));
    let mut replica = Replica::default();
    assert_eq!(replica.drain(&mut pool, rid), 1);
    assert_matches(&replica, &small_facts(1));
}

// ===========================================================================
// 28. revocation_never_republishes_hidden_facts
// ===========================================================================

fn contains_subslice(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn revocation_never_republishes_hidden_facts() {
    // Entity E is row 500. Its health is granted in transfer 1 and revoked
    // before transfer 2. The sentinel values appear nowhere else.
    const E_ROW: usize = 500;
    let e_id = MAX_BASE + E_ROW as u64;
    let secret = (1_234_567u32, 7_654_321u32, true);
    let mut granted = big_fixture();
    granted.rows[E_ROW].health = secret;
    let mut narrowed = granted.clone();
    narrowed.rows[E_ROW].health_granted = false; // the revocation

    let secret_bytes = varint(u64::from(secret.0));
    let body1 = encode_body(&granted.body()).unwrap();
    let body2 = encode_body(&narrowed.body()).unwrap();
    assert!(
        contains_subslice(&body1, &secret_bytes),
        "control: disclosed"
    );
    assert!(
        !contains_subslice(&body2, &secret_bytes),
        "narrowed body leaks"
    );
    // E's disclosed health takes 3 + 4 + 1 bytes; the narrowed body drops
    // all of E's health bytes, and the flag byte stays as a single 0.
    assert_eq!(body1.len(), BIG_FIXTURE_BYTES - 3);
    assert_eq!(body2.len(), BIG_FIXTURE_BYTES - 11);

    let f1 = granted.frames(1, 100, 0, 1);
    let f2 = narrowed.frames(2, 100, 0, 1);
    assert_eq!((f1.len(), f2.len()), (7, 7), "six chunks each");

    let (mut pool, rid) = mkpool();
    assert_eq!(ok_t(&mut pool, rid, &f1[0], T0), TransferOutcome::Begun);
    for f in &f1[1..=5] {
        assert!(matches!(
            ok_t(&mut pool, rid, f, T0),
            TransferOutcome::Stored { .. }
        ));
    }
    // After 5 chunks the stand-in revokes and sends Cancel(Aborted).
    assert_eq!(
        ok_t(
            &mut pool,
            rid,
            &cancel_frame(EPOCH, 1, CancelReason::Aborted),
            T0
        ),
        TransferOutcome::Cancelled { discarded: true }
    );
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(pool.status(rid).unwrap().queued_updates, 0);
    // The sixth chunk and duplicates of all six are StaleTransfer.
    for (n, f) in f1[1..=6].iter().enumerate() {
        assert_eq!(
            refuse_t(&mut pool, rid, f, T0),
            TransferError::StaleTransfer,
            "chunk {n}"
        );
    }
    assert_eq!(
        refuse_t(&mut pool, rid, &f1[0], T0),
        TransferError::StaleTransfer
    );
    assert_eq!(
        pool.pop_update(rid),
        None,
        "nothing of transfer 1 reached the queue"
    );
    // Transfer 2, built under the narrowed grants, publishes.
    assert_eq!(ok_t(&mut pool, rid, &f2[0], T0), TransferOutcome::Begun);
    for f in &f2[1..f2.len() - 1] {
        ok_t(&mut pool, rid, f, T0);
    }
    assert!(matches!(
        ok_t(&mut pool, rid, f2.last().unwrap(), T0),
        TransferOutcome::Published { transfer, .. } if transfer == tid(2)
    ));
    let mut replica = Replica::default();
    assert_eq!(replica.drain(&mut pool, rid), 1);
    // The oracle finds E's health absent and every fact permitted.
    assert_eq!(replica.facts.entities.get(&e_id), Some(&None));
    assert_matches(&replica, &big_facts(&[E_ROW as u64]));

    // Negative control: without the cancel, chunk 6 publishes E's health and
    // the oracle flags the leak.
    let (mut pool, rid) = mkpool();
    ok_t(&mut pool, rid, &f1[0], T0);
    for f in &f1[1..=5] {
        ok_t(&mut pool, rid, f, T0);
    }
    assert!(matches!(
        ok_t(&mut pool, rid, &f1[6], T0),
        TransferOutcome::Published { transfer, .. } if transfer == tid(1)
    ));
    let mut leaked = Replica::default();
    assert_eq!(leaked.drain(&mut pool, rid), 1);
    assert_eq!(leaked.facts.entities.get(&e_id), Some(&Some(secret)));
    assert_eq!(
        diff(&leaked.facts, &big_facts(&[E_ROW as u64])),
        vec![format!("leak: health of entity {e_id} is not permitted")]
    );
}

// ===========================================================================
// 29. hidden_values_never_reach_bytes
// ===========================================================================

#[test]
fn hidden_values_never_reach_bytes() {
    // The fixture table: A is fully granted; B is present with no health
    // grant; C is not granted at all; D may not exist; turn_state is denied.
    let table = |b_health: HealthTuple,
                 c_health: HealthTuple,
                 d_exists: bool,
                 turn_round: u64,
                 turn_active: Option<u64>| Fixture {
        rows: vec![
            Row {
                entity: 1,
                exists: true,
                present_granted: true,
                health: (10, 10, false),
                health_granted: true,
            },
            Row {
                entity: 2,
                exists: true,
                present_granted: true,
                health: b_health,
                health_granted: false,
            },
            Row {
                entity: 3,
                exists: true,
                present_granted: false,
                health: c_health,
                health_granted: false,
            },
            Row {
                entity: 4,
                exists: d_exists,
                present_granted: false,
                health: (4, 4, false),
                health_granted: false,
            },
        ],
        turn: TurnRow {
            active: turn_active,
            round: turn_round,
            granted: false,
        },
    };
    let f1 = table(
        (777_777, 888_888, true),
        (555_555, 666_666, false),
        true,
        123_456_789,
        Some(3),
    );
    let f2 = table((1, 2, false), (3, 4, true), false, 9, None);
    let hidden_values = [777_777u64, 888_888, 555_555, 666_666, 123_456_789];

    // The bodies, and therefore every planned frame, are byte-identical.
    let want_body = SnapshotBody {
        entities: vec![
            SnapshotEntity {
                entity: net(1),
                health: Some(health(10, 10, false)),
            },
            SnapshotEntity {
                entity: net(2),
                health: None,
            },
        ],
        turn: None,
    };
    assert_eq!(f1.body(), want_body);
    assert_eq!(f2.body(), want_body);
    for spc in [1u8, 8] {
        let a = f1.frames(1, 100, 0, spc);
        let b = f2.frames(1, 100, 0, spc);
        assert_eq!(a, b, "spc {spc}");
        assert_eq!(a, plan(1, 100, 0, spc, &want_body));
        let all: Vec<u8> = a.concat();
        for v in hidden_values {
            assert!(
                !contains_subslice(&all, &varint(v)),
                "hidden value {v} reached the frames"
            );
        }
    }
    // A None health is one 0 byte: the exact body is
    // `count, A(id 1, present, 10, 10, alive), B(id 2, absent), no turn`.
    assert_eq!(
        encode_body(&want_body).unwrap(),
        vec![0x02, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x02, 0x00, 0x00]
    );
    let only_b = SnapshotBody {
        entities: vec![SnapshotEntity {
            entity: net(2),
            health: None,
        }],
        turn: None,
    };
    assert_eq!(encode_body(&only_b).unwrap(), vec![0x01, 0x02, 0x00, 0x00]);

    // Positive controls: a granted change produces different bytes, and the
    // detector does see a granted value.
    let base = f1.frames(1, 100, 0, 1);
    let mut granted_health = f1.clone();
    granted_health.rows[0].health = (11, 10, false);
    assert_ne!(granted_health.frames(1, 100, 0, 1), base, "granted value");
    let mut grant_b_health = f1.clone();
    grant_b_health.rows[1].health_granted = true;
    let b_frames = grant_b_health.frames(1, 100, 0, 1);
    assert_ne!(b_frames, base, "granted health of B");
    assert!(contains_subslice(&b_frames.concat(), &varint(777_777)));
    let mut grant_c = f1.clone();
    grant_c.rows[2].present_granted = true;
    assert_ne!(grant_c.frames(1, 100, 0, 1), base, "granted presence of C");
    let mut grant_turn = f1.clone();
    grant_turn.turn.granted = true;
    let turn_frames = grant_turn.frames(1, 100, 0, 1);
    assert_ne!(turn_frames, base, "granted turn_state");
    assert!(contains_subslice(
        &turn_frames.concat(),
        &varint(123_456_789)
    ));
    // `active` is None unless that entity is present: entity 3 is not.
    assert_eq!(
        grant_turn.body().turn,
        Some(TurnFact {
            active: None,
            round: 123_456_789
        })
    );
    let mut turn_on_a = grant_turn.clone();
    turn_on_a.turn.active = Some(1);
    assert_eq!(
        turn_on_a.body().turn,
        Some(TurnFact {
            active: Some(net(1)),
            round: 123_456_789
        })
    );
    // A granted entity that does not exist adds nothing.
    let mut ghost = f2.clone();
    ghost.rows[3].present_granted = true;
    assert_eq!(ghost.frames(1, 100, 0, 1), base);
    let mut real = f1.clone();
    real.rows[3].present_granted = true;
    assert_ne!(real.frames(1, 100, 0, 1), base, "granted presence of D");
}

// ===========================================================================
// 30. oracle_catches_dropped_and_corrupt_chunks
// ===========================================================================

#[test]
fn oracle_catches_dropped_and_corrupt_chunks() {
    // 1,000 entities (ids 1..=1000), each with a small health fact: two
    // chunks at one segment per chunk.
    let body = SnapshotBody {
        entities: (1..=1000u64)
            .map(|i| SnapshotEntity {
                entity: net(i),
                health: Some(health(10 + (i % 50) as u32, 100, false)),
            })
            .collect(),
        turn: None,
    };
    let mut want = Facts::default();
    for i in 1..=1000u64 {
        want.entities
            .insert(i, Some((10 + (i % 50) as u32, 100, false)));
    }
    let payload = encode_body(&body).unwrap();
    let frames = payload_frames(EPOCH, 1, 100, 0, 1, 1000, &payload);
    assert_eq!(frames, plan(1, 100, 0, 1, &body));
    assert_eq!(frames.len(), 1 + 2, "Begin and two chunks");

    // The unmodified transfer matches.
    let (mut pool, rid) = mkpool();
    assert!(matches!(
        offer_all(&mut pool, rid, &frames, T0),
        TransferOutcome::Published { .. }
    ));
    let mut replica = Replica::default();
    replica.drain(&mut pool, rid);
    assert_matches(&replica, &want);

    // Dropping a required chunk means no snapshot, and the oracle reports it
    // missing.
    let (mut pool, rid) = mkpool();
    assert_eq!(ok_t(&mut pool, rid, &frames[0], T0), TransferOutcome::Begun);
    assert!(matches!(
        ok_t(&mut pool, rid, &frames[1], T0),
        TransferOutcome::Stored {
            received: 1,
            chunk_count: 2
        }
    ));
    // (chunk 1 never arrives; even a duplicate of chunk 0 completes nothing)
    assert_eq!(
        ok_t(&mut pool, rid, &frames[1], T0),
        TransferOutcome::Duplicate
    );
    let st = pool.status(rid).unwrap();
    assert_eq!(st.published, None);
    assert_eq!(st.queued_updates, 0);
    assert_eq!(pool.poll(T0 + 5_000), vec![rid]);
    let mut replica = Replica::default();
    assert_eq!(replica.drain(&mut pool, rid), 0);
    let d = diff(&replica.facts, &want);
    assert_eq!(d.len(), 1000);
    assert_eq!(d[0], "missing entity 1");
    assert!(d.iter().all(|m| m.starts_with("missing entity")));

    // One health byte corrupted consistently (duplicates carry the same
    // corruption) gives a decodable wrong body that publishes, and the oracle
    // reports the mismatch.
    let needle = [11u8, 1, 21, 100, 0]; // entity 11, health 21/100, alive
    let at: Vec<usize> = payload
        .windows(needle.len())
        .enumerate()
        .filter(|(_, w)| *w == needle)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(at.len(), 1, "the needle must be unique");
    let mut bad_payload = payload.clone();
    bad_payload[at[0] + 2] = 22;
    assert!(
        decode_body(&bad_payload).is_ok(),
        "the corruption is decodable"
    );
    let bad = payload_frames(EPOCH, 1, 100, 0, 1, 1000, &bad_payload);
    assert_eq!(bad.len(), frames.len());
    assert_ne!(bad, frames);
    let (mut pool, rid) = mkpool();
    assert_eq!(ok_t(&mut pool, rid, &bad[0], T0), TransferOutcome::Begun);
    assert_eq!(
        ok_t(&mut pool, rid, &bad[0], T0),
        TransferOutcome::Duplicate
    );
    ok_t(&mut pool, rid, &bad[1], T0);
    assert_eq!(
        ok_t(&mut pool, rid, &bad[1], T0),
        TransferOutcome::Duplicate
    );
    assert_eq!(
        ok_t(&mut pool, rid, &bad[2], T0),
        TransferOutcome::Published {
            transfer: tid(1),
            released: 0
        }
    );
    // The duplicate of the last chunk carries the same corruption.
    assert_eq!(
        refuse_t(&mut pool, rid, &bad[2], T0),
        TransferError::StaleTransfer
    );
    let mut replica = Replica::default();
    assert_eq!(replica.drain(&mut pool, rid), 1);
    assert_eq!(
        diff(&replica.facts, &want),
        vec![
            "mismatch health of entity 11: got Some((22, 100, false)) want Some((21, 100, false))"
                .to_string()
        ]
    );
}

// ===========================================================================
// 31. fabric_transfer_with_faults_and_resync
// ===========================================================================

struct Link {
    transport: InMemoryTransport,
    peer: PeerId,
}

impl Link {
    fn new() -> Self {
        // Test-generous rates (S§1.5): the v1 intent rates would take 14 s to
        // admit a full transfer.
        let rates = RateCaps {
            frames_per_sec: 1_000_000,
            burst_frames: 1_000_000,
            bytes_per_sec: 1_000_000_000,
            burst_bytes: 1_000_000_000,
            fail_per_sec: 1_000_000,
            burst_fail: 1_000_000,
            fail_bytes_per_sec: 1_000_000_000,
            burst_fail_bytes: 1_000_000_000,
        };
        let mut transport = InMemoryTransport::new(QueueCaps::v1_egress(), rates, SimClock(0));
        let peer = transport.add_peer(EPOCH).expect("peer");
        Self { transport, peer }
    }

    fn send_all(&mut self, frames: &[Vec<u8>]) {
        for f in frames {
            self.transport
                .send_to(self.peer, f.clone())
                .expect("staged under the egress caps");
        }
    }

    /// One fabric tick under `faults`, then everything the client can read.
    fn deliver(&mut self, faults: &FaultSchedule) -> Vec<Vec<u8>> {
        self.transport.tick(faults);
        let mut out = Vec::new();
        while let Some(b) = self.transport.recv_from(self.peer) {
            out.push(b);
        }
        out
    }
}

/// The client: a pool receiver plus the oracle. Under reordering a chunk can
/// precede its `Begin` (S§11 item 3 is the host's duty on a reliable
/// channel), so the driver retries `UnknownTransfer` frames once a `Begin`
/// lands, like a reliable-ordered lane would.
struct Client {
    pool: ReassemblyPool,
    rid: ReceiverId,
    replica: Replica,
    published: usize,
    duplicates: usize,
    stale: usize,
    stash: Vec<Vec<u8>>,
}

impl Client {
    fn new() -> Self {
        let (pool, rid) = mkpool();
        Self {
            pool,
            rid,
            replica: Replica::default(),
            published: 0,
            duplicates: 0,
            stale: 0,
            stash: Vec::new(),
        }
    }

    fn receive(&mut self, frames: &[Vec<u8>], now: u64) {
        let mut queue: std::collections::VecDeque<Vec<u8>> = frames.iter().cloned().collect();
        while let Some(f) = queue.pop_front() {
            match self.pool.offer_transfer(self.rid, &f, now) {
                Ok(TransferOutcome::Begun) => queue.extend(self.stash.drain(..)),
                Ok(TransferOutcome::Published { .. }) => self.published += 1,
                Ok(TransferOutcome::Duplicate) => self.duplicates += 1,
                Ok(TransferOutcome::Stored { .. }) => {}
                Ok(other) => panic!("unexpected outcome {other:?}"),
                Err(TransferError::UnknownTransfer) => self.stash.push(f),
                Err(TransferError::StaleTransfer) => self.stale += 1,
                Err(e) => panic!("unexpected refusal {e:?}"),
            }
        }
    }

    fn drain(&mut self) -> usize {
        self.replica.drain(&mut self.pool, self.rid)
    }
}

/// The stand-in host: a fixture table, plus the transfer ids it has sent.
struct StandIn {
    fixture: Fixture,
    sent: Vec<u64>,
}

impl StandIn {
    fn send(&mut self, link: &mut Link, id: u64) -> Vec<Vec<u8>> {
        let frames = self.fixture.frames(id, 100, 0, 1);
        link.send_all(&frames);
        self.sent.push(id);
        frames
    }
}

#[test]
fn fabric_transfer_with_faults_and_resync() {
    let t = T0;
    let want = big_facts(&[]);

    // Dup and reorder faults: the transfer publishes exactly once.
    let mut host = StandIn {
        fixture: big_fixture(),
        sent: vec![],
    };
    let mut link = Link::new();
    let frames = host.send(&mut link, 1);
    assert_eq!(frames.len(), 7);
    let faults = FaultSchedule {
        loss_every: None,
        dup_every: Some(2),
        reorder_depth: 2,
        seed: 7,
    };
    let delivered = link.deliver(&faults);
    // Datagrams 2, 4 and 6 were duplicated: 7 + 3 frames arrive.
    assert_eq!(delivered.len(), 10);
    let mut in_order = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        in_order.push(f.clone());
        if (i + 1) % 2 == 0 {
            in_order.push(f.clone());
        }
    }
    assert_ne!(delivered, in_order, "the fabric must have reordered");
    let mut sorted_got = delivered.clone();
    let mut sorted_sent = in_order.clone();
    sorted_got.sort();
    sorted_sent.sort();
    assert_eq!(
        sorted_got, sorted_sent,
        "reorder neither loses nor invents frames"
    );

    let mut client = Client::new();
    client.receive(&delivered, t);
    assert_eq!(client.published, 1, "publishes exactly once");
    assert!(client.stash.is_empty(), "every early chunk found its Begin");
    assert!(
        client.duplicates + client.stale >= 3,
        "duplicates were absorbed"
    );
    assert_eq!(client.pool.status(client.rid).unwrap().retired_through, 1);
    assert_eq!(client.drain(), 1, "one snapshot update in the queue");
    assert_eq!(client.replica.snapshots, 1);
    assert_eq!(client.replica.order, vec!["S"]);
    assert_matches(&client.replica, &want);
    assert_eq!(client.pool.staging_bytes(), 0);
    // Re-delivering the whole batch publishes nothing again.
    client.receive(&delivered, t + 10);
    assert_eq!(client.published, 1);
    assert_eq!(client.drain(), 0);

    // A loss that hits a chunk: the transfer expires at t + 5,000 and the
    // client resyncs.
    let mut host = StandIn {
        fixture: big_fixture(),
        sent: vec![],
    };
    let mut link = Link::new();
    host.send(&mut link, 1);
    let lossy = FaultSchedule {
        loss_every: Some(4),
        dup_every: None,
        reorder_depth: 0,
        seed: 0,
    };
    let delivered = link.deliver(&lossy);
    assert_eq!(delivered.len(), 6, "datagram 4, chunk 2, was lost");
    let mut client = Client::new();
    client.receive(&delivered, t);
    assert_eq!(client.published, 0);
    let st = client.pool.status(client.rid).unwrap();
    assert_eq!(st.in_flight.as_ref().unwrap().received, 5);
    assert_eq!(client.pool.poll(t + 4_999), Vec::<ReceiverId>::new());
    assert_eq!(client.pool.poll(t + 5_000), vec![client.rid]);
    assert_eq!(
        client.drain(),
        0,
        "nothing of the failed transfer is queued"
    );
    assert_eq!(client.replica.snapshots, 0);
    assert_eq!(
        diff(&client.replica.facts, &want).len(),
        1024 + 1,
        "the oracle sees nothing yet: every entity and the turn are missing"
    );
    let st = client.pool.status(client.rid).unwrap();
    assert_eq!(st.resync, Some(ResyncReason::Expired));

    // The client encodes Resync { after: Some(1), Expired } ...
    let resync = TransferUp::Resync {
        epoch: EPOCH,
        after: TransferId::new(st.retired_through),
        reason: st.resync.unwrap(),
    };
    let wire = encode_up(&resync).unwrap();
    // ... the stand-in decodes it and sends transfer 2, which publishes.
    match decode_up(&wire).unwrap() {
        TransferUp::Resync {
            epoch,
            after,
            reason,
        } => {
            assert_eq!(epoch, EPOCH);
            assert_eq!(after, Some(tid(1)));
            assert_eq!(reason, ResyncReason::Expired);
            let next = after.map_or(1, |a| a.get() + 1);
            host.send(&mut link, next);
        }
        other => panic!("expected Resync, got {other:?}"),
    }
    assert_eq!(host.sent, vec![1, 2]);
    let delivered = link.deliver(&FaultSchedule::clean());
    assert_eq!(delivered.len(), 7);
    client.receive(&delivered, t + 5_000);
    assert_eq!(client.published, 1);
    assert_eq!(client.drain(), 1);
    assert_matches(&client.replica, &want);
    let st = client.pool.status(client.rid).unwrap();
    assert_eq!(
        (st.retired_through, st.resync, st.mode),
        (2, None, GateMode::Live)
    );
    // The Published frame the client answers with round-trips.
    let reply = TransferUp::Published {
        epoch: EPOCH,
        transfer: tid(2),
    };
    assert_eq!(decode_up(&encode_up(&reply).unwrap()), Ok(reply));
}
