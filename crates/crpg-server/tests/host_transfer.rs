//! T024h1 acceptance suite (`tasks/T024h.md` S§11 cases 1–11): public API only.
//!
//! The client stand-in is a `crpg_net::reassembly::ReassemblyPool`
//! (`TransferLimits::v1()`, the host's `DeltaWire`, `GateMode::Live`) fed every
//! delivery entry in ordinal order; its `ReplicaUpdate`s are applied to a
//! test-local `BTreeMap` replica. The oracle reads only public host output (no
//! `PeerView`, `full_state_ops`, `project_events` or a World hash) plus the
//! hand-written bodies of T024 S§5.4.
//!
//! Every refusal is checked against a clone of the evidence taken before:
//! `take_delivery_entries`, `transfer_state`, and the complete-state snapshot
//! (authority hash, executions, last capture, capture watermark).

use std::collections::BTreeMap;

use crpg_core::{EntityId, Ulid};
use crpg_net::codec;
use crpg_net::codec_transfer;
use crpg_net::codec_v2;
use crpg_net::protocol::{
    self, IntentBody, IntentFrame, NetId, ReceiptStatus, RejectionCode, LANE_COMBAT,
    MAX_INTENT_FRAME_BYTES,
};
use crpg_net::protocol_transfer::{
    BeginTransfer, CancelReason, HealthFact, ResyncReason, SnapshotBody, SnapshotEntity,
    TransferDown, TransferId, TransferUp, TurnFact,
};
use crpg_net::protocol_v2::{self, DeltaOp};
use crpg_net::reassembly::{
    DeltaOutcome, DeltaWire, GateMode, GatedDelta, ReassemblyPool, ReceiverId, ReplicaUpdate,
    TransferError, TransferLimits, TransferOutcome,
};
use crpg_server::host::{
    AdmissionResult, ControlGrant, DeliveryEntry, DeliveryKind, DisclosureGrants, EntityDisclosure,
    GrantUpdate, Host, HostConfig, HostError, IngestDisposition, PeerHandle, ProtocolSelection,
    PumpSummary, TransferDisposition, TransferRefusal, TransferState, MAX_CONTROLLED_PER_PEER,
    MAX_PEERS,
};
use crpg_sim::HistoryWorld;
use serde_json::json;

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");

const V1: ProtocolSelection = ProtocolSelection::V1;
const V2: ProtocolSelection = ProtocolSelection::V2;

/// Runs one named case body under both selected versions, as
/// `<case>::v1` and `<case>::v2`.
macro_rules! both_versions {
    ($name:ident) => {
        mod $name {
            #[test]
            fn v1() {
                super::$name(super::V1);
            }
            #[test]
            fn v2() {
                super::$name(super::V2);
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Fixture identities and grants.
// ---------------------------------------------------------------------------

fn eid(index: u32) -> EntityId {
    serde_json::from_value(json!({"index": index, "generation": 1})).expect("valid entity id")
}

fn a() -> EntityId {
    eid(0)
}
fn b() -> EntityId {
    eid(1)
}
fn c() -> EntityId {
    eid(2)
}

fn strike() -> Ulid {
    Ulid::from_u128(603)
}
fn whiff() -> Ulid {
    Ulid::from_u128(605)
}
fn encounter() -> Ulid {
    Ulid::from_u128(601)
}

fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}

fn fixture() -> HistoryWorld {
    let mut history: HistoryWorld =
        serde_json::from_str(FIXTURE).expect("fixture loads through validated serde");
    history
        .acknowledge(history.last_sequence())
        .expect("acknowledges its own range");
    history
}

fn full(entity: EntityId) -> EntityDisclosure {
    EntityDisclosure {
        entity,
        present: true,
        health: true,
        spawn: true,
        despawn: true,
        died: true,
        action_actor: true,
        action_target: true,
        action_outcome: true,
        action_damage: true,
        turn_start: true,
    }
}

fn full_grants() -> DisclosureGrants {
    DisclosureGrants {
        entities: vec![full(a()), full(b()), full(c())],
        abilities: vec![strike(), Ulid::from_u128(604), whiff()],
        encounters: vec![encounter()],
        turn_round: true,
        encounter_end: true,
        turn_state: true,
    }
}

/// Presence plus health plus turn state only: no action/turn/death facts.
fn observer_grants() -> DisclosureGrants {
    let observer = |entity| EntityDisclosure {
        entity,
        present: true,
        health: true,
        spawn: false,
        despawn: false,
        died: false,
        action_actor: false,
        action_target: false,
        action_outcome: false,
        action_damage: false,
        turn_start: false,
    };
    DisclosureGrants {
        entities: vec![observer(a()), observer(b()), observer(c())],
        turn_state: true,
        ..DisclosureGrants::default()
    }
}

/// Full grants with B not present (`B_hidden`).
fn hide_b(mut grants: DisclosureGrants) -> DisclosureGrants {
    grants.entities[1].present = false;
    grants
}

/// Full grants with A's health off (`A_no_health_b`).
fn a_no_health_b(mut grants: DisclosureGrants) -> DisclosureGrants {
    grants.entities[0].health = false;
    grants
}

/// B not present and A's health off (`A_no_health`).
fn a_no_health(mut grants: DisclosureGrants) -> DisclosureGrants {
    grants.entities[0].health = false;
    grants.entities[1].present = false;
    grants
}

/// `turn_state` off (`no_turn`).
fn no_turn(mut grants: DisclosureGrants) -> DisclosureGrants {
    grants.turn_state = false;
    grants
}

/// `turn_state` off and B not present.
fn no_turn_hidden_b(grants: DisclosureGrants) -> DisclosureGrants {
    let mut grants = no_turn(grants);
    grants.entities[1].present = false;
    grants
}

fn control(actors: &[EntityId]) -> ControlGrant {
    ControlGrant {
        actors: actors.to_vec(),
    }
}

fn all_actors() -> ControlGrant {
    control(&[a(), b(), c()])
}

fn epoch_of(incarnation: u64, handle: u64) -> [u8; 16] {
    let mut epoch = [0u8; 16];
    epoch[..8].copy_from_slice(&incarnation.to_le_bytes());
    epoch[8..].copy_from_slice(&handle.to_le_bytes());
    epoch
}

// ---------------------------------------------------------------------------
// Literal T024 S§5.4 bodies.
// ---------------------------------------------------------------------------

fn b_hidden() -> Vec<u8> {
    vec![
        0x02, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x03, 0x01, 0x03, 0x03, 0x00, 0x01, 0x00, 0x00,
    ]
}

fn a_no_health_body() -> Vec<u8> {
    vec![
        0x02, 0x01, 0x00, 0x03, 0x01, 0x03, 0x03, 0x00, 0x01, 0x00, 0x00,
    ]
}

fn a_no_health_b_body() -> Vec<u8> {
    vec![
        0x03, 0x01, 0x00, 0x02, 0x01, 0x07, 0x0A, 0x00, 0x03, 0x01, 0x03, 0x03, 0x00, 0x01, 0x01,
        0x02, 0x00,
    ]
}

fn no_turn_body() -> Vec<u8> {
    vec![
        0x03, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x02, 0x01, 0x07, 0x0A, 0x00, 0x03, 0x01, 0x03, 0x03,
        0x00, 0x00,
    ]
}

fn full_after_1() -> Vec<u8> {
    vec![
        0x03, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x02, 0x01, 0x07, 0x0A, 0x00, 0x03, 0x01, 0x03, 0x03,
        0x00, 0x01, 0x01, 0x02, 0x00,
    ]
}

type EntityFacts = (u64, Option<(u32, u32, bool)>);

fn body(entities: &[EntityFacts], turn: Option<(Option<u64>, u64)>) -> SnapshotBody {
    SnapshotBody {
        entities: entities
            .iter()
            .map(|(entity, health)| SnapshotEntity {
                entity: n(*entity),
                health: health.map(|(health, max_health, dead)| HealthFact {
                    health,
                    max_health,
                    dead,
                }),
            })
            .collect(),
        turn: turn.map(|(active, round)| TurnFact {
            active: active.map(n),
            round,
        }),
    }
}

fn b_hidden_body() -> SnapshotBody {
    body(
        &[(1, Some((10, 10, false))), (3, Some((3, 3, false)))],
        Some((None, 0)),
    )
}

fn a_no_health_snapshot() -> SnapshotBody {
    body(&[(1, None), (3, Some((3, 3, false)))], Some((None, 0)))
}

fn a_no_health_b_snapshot() -> SnapshotBody {
    body(
        &[
            (1, None),
            (2, Some((7, 10, false))),
            (3, Some((3, 3, false))),
        ],
        Some((Some(2), 0)),
    )
}

fn no_turn_snapshot() -> SnapshotBody {
    body(
        &[
            (1, Some((10, 10, false))),
            (2, Some((7, 10, false))),
            (3, Some((3, 3, false))),
        ],
        None,
    )
}

fn full_after_1_body() -> SnapshotBody {
    body(
        &[
            (1, Some((10, 10, false))),
            (2, Some((7, 10, false))),
            (3, Some((3, 3, false))),
        ],
        Some((Some(2), 0)),
    )
}

// ---------------------------------------------------------------------------
// Expected-op constructors.
// ---------------------------------------------------------------------------

fn legacy(op: protocol::DeltaOp) -> DeltaOp {
    DeltaOp::Legacy(op)
}
fn enter(id: u64) -> DeltaOp {
    legacy(protocol::DeltaOp::EntityEnter { entity: n(id) })
}
fn health(id: u64, health: u32, max_health: u32, dead: bool) -> DeltaOp {
    legacy(protocol::DeltaOp::Health {
        entity: n(id),
        health,
        max_health,
        dead,
    })
}
fn turn(active: Option<u64>, round: u64) -> DeltaOp {
    legacy(protocol::DeltaOp::Turn {
        active: active.map(n),
        round,
    })
}
fn died(id: u64) -> DeltaOp {
    legacy(protocol::DeltaOp::Died { entity: n(id) })
}
fn started(actor: u64, round: u64) -> DeltaOp {
    DeltaOp::TurnStarted {
        actor: n(actor),
        round,
    }
}
fn receipt(epoch: [u8; 16], seq: u64, tick: u64, status: ReceiptStatus) -> DeltaOp {
    legacy(protocol::DeltaOp::Receipt {
        epoch,
        lane: LANE_COMBAT,
        seq,
        processed_tick: tick,
        status,
    })
}

/// The hand-authored V1 projection rule: V1 carries receipts and state ops
/// only, never event ops.
fn for_version(protocol: ProtocolSelection, ops: Vec<DeltaOp>) -> Vec<DeltaOp> {
    match protocol {
        ProtocolSelection::V2 => ops,
        ProtocolSelection::V1 => ops
            .into_iter()
            .filter(|op| {
                matches!(
                    op,
                    DeltaOp::Legacy(
                        protocol::DeltaOp::Receipt { .. }
                            | protocol::DeltaOp::Health { .. }
                            | protocol::DeltaOp::Turn { .. }
                            | protocol::DeltaOp::EntityEnter { .. }
                            | protocol::DeltaOp::EntityLeave { .. }
                    )
                )
            })
            .collect(),
    }
}

fn later_ops() -> Vec<DeltaOp> {
    vec![
        died(3),
        started(1, 1),
        health(1, 10, 10, false),
        health(3, 0, 3, true),
        turn(Some(1), 1),
    ]
}

fn initial_full_state() -> Vec<DeltaOp> {
    vec![
        enter(1),
        enter(2),
        enter(3),
        health(1, 10, 10, false),
        health(2, 10, 10, false),
        health(3, 3, 3, false),
        turn(Some(1), 0),
    ]
}

const APPLIED: ReceiptStatus = ReceiptStatus::Applied;

// ---------------------------------------------------------------------------
// Decoded delivery.
// ---------------------------------------------------------------------------

fn decode_frame(protocol: ProtocolSelection, bytes: &[u8]) -> protocol_v2::DeltaFrame {
    match protocol {
        ProtocolSelection::V1 => {
            let frame = codec::decode_delta(bytes).expect("v1 delivery decodes");
            protocol_v2::DeltaFrame {
                lane: frame.lane,
                server_tick: frame.server_tick,
                event_seq: frame.event_seq,
                ops: frame.ops.into_iter().map(DeltaOp::Legacy).collect(),
            }
        }
        ProtocolSelection::V2 => codec_v2::decode_delta(bytes).expect("v2 delivery decodes"),
    }
}

fn decode_transfer(bytes: &[u8]) -> TransferDown {
    codec_transfer::decode_down(bytes).expect("transfer frame decodes")
}

fn decode_begin(bytes: &[u8]) -> BeginTransfer {
    match decode_transfer(bytes) {
        TransferDown::Begin(begin) => begin,
        other => panic!("expected Begin, got {other:?}"),
    }
}

fn decode_chunk(bytes: &[u8]) -> Vec<u8> {
    match decode_transfer(bytes) {
        TransferDown::Chunk(chunk) => chunk.bytes,
        other => panic!("expected Chunk, got {other:?}"),
    }
}

fn decode_cancel(bytes: &[u8]) -> (TransferId, CancelReason) {
    match decode_transfer(bytes) {
        TransferDown::Cancel {
            transfer, reason, ..
        } => (transfer, reason),
        other => panic!("expected Cancel, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// The replica (client stand-in).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Replica {
    health: BTreeMap<u64, Option<(u32, u32, bool)>>,
    turn: Option<(Option<u64>, u64)>,
}

fn expected(items: &[(u64, (u32, u32, bool))], turn: Option<(Option<u64>, u64)>) -> Replica {
    Replica {
        health: items
            .iter()
            .map(|(id, facts)| (*id, Some(*facts)))
            .collect(),
        turn,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Fed {
    Delta(DeltaOutcome),
    Transfer(TransferOutcome),
}

struct Sink {
    pool: ReassemblyPool,
    rx: ReceiverId,
    replica: Replica,
    /// The highest ordinal fed so far.
    seen: u64,
    /// The last delta `event_seq` applied.
    last_delta_seq: u64,
    /// The highest published snapshot cursor.
    published_cursor: u64,
    /// Every update popped, in order (for the cutover-order assertions).
    popped: Vec<ReplicaUpdate>,
}

impl Sink {
    fn new(epoch: [u8; 16], protocol: ProtocolSelection) -> Self {
        let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("v1 limits");
        let wire = match protocol {
            ProtocolSelection::V1 => DeltaWire::V1,
            ProtocolSelection::V2 => DeltaWire::V2,
        };
        let rx = pool
            .add_receiver(epoch, wire, GateMode::Live)
            .expect("receiver");
        Self {
            pool,
            rx,
            replica: Replica::default(),
            seen: 0,
            last_delta_seq: 0,
            published_cursor: 0,
            popped: Vec::new(),
        }
    }

    fn offer(&mut self, entry: &DeliveryEntry, now: u64) -> Result<Fed, TransferError> {
        match entry.kind {
            DeliveryKind::Delta => self
                .pool
                .offer_delta(self.rx, &entry.bytes, now)
                .map(Fed::Delta),
            DeliveryKind::Transfer => self
                .pool
                .offer_transfer(self.rx, &entry.bytes, now)
                .map(Fed::Transfer),
        }
    }

    /// Pops every queued update, applies it, and returns how many were popped.
    fn drain(&mut self) -> usize {
        let mut count = 0;
        while let Some(update) = self.pool.pop_update(self.rx) {
            self.apply(&update);
            self.popped.push(update);
            count += 1;
        }
        count
    }

    fn apply(&mut self, update: &ReplicaUpdate) {
        match update {
            ReplicaUpdate::Snapshot(publication) => {
                self.replica.health.clear();
                self.replica.turn = publication
                    .body
                    .turn
                    .as_ref()
                    .map(|turn| (turn.active.map(NetId::get), turn.round));
                for entity in &publication.body.entities {
                    self.replica.health.insert(
                        entity.entity.get(),
                        entity
                            .health
                            .as_ref()
                            .map(|fact| (fact.health, fact.max_health, fact.dead)),
                    );
                }
                self.published_cursor = self.published_cursor.max(publication.cursor);
            }
            ReplicaUpdate::Delta(gated) => {
                let (seq, ops) = match gated {
                    GatedDelta::V1(frame) => (
                        frame.event_seq,
                        frame
                            .ops
                            .iter()
                            .cloned()
                            .map(DeltaOp::Legacy)
                            .collect::<Vec<_>>(),
                    ),
                    GatedDelta::V2(frame) => (frame.event_seq, frame.ops.clone()),
                };
                assert!(
                    seq > self.last_delta_seq,
                    "popped deltas have strictly increasing event_seq: {seq} after {}",
                    self.last_delta_seq
                );
                assert!(
                    seq > self.published_cursor,
                    "popped deltas are above the last published cursor: {seq} vs {}",
                    self.published_cursor
                );
                self.last_delta_seq = seq;
                for op in &ops {
                    match op {
                        DeltaOp::Legacy(protocol::DeltaOp::EntityEnter { entity }) => {
                            self.replica.health.insert(entity.get(), None);
                        }
                        DeltaOp::Legacy(protocol::DeltaOp::EntityLeave { entity }) => {
                            self.replica.health.remove(&entity.get());
                        }
                        DeltaOp::Legacy(protocol::DeltaOp::Health {
                            entity,
                            health,
                            max_health,
                            dead,
                        }) => {
                            self.replica
                                .health
                                .insert(entity.get(), Some((*health, *max_health, *dead)));
                        }
                        DeltaOp::Legacy(protocol::DeltaOp::Turn { active, round }) => {
                            self.replica.turn = Some((active.map(NetId::get), *round));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn feed(&mut self, entries: &[DeliveryEntry], now: u64) -> Vec<Fed> {
        let mut out = Vec::new();
        for entry in entries {
            if entry.ordinal <= self.seen {
                continue;
            }
            self.seen = entry.ordinal;
            out.push(
                self.offer(entry, now)
                    .expect("the pool accepts host frames"),
            );
        }
        self.drain();
        out
    }

    /// Asserts that no delta frame applied or queued from `from` onwards
    /// names `id`.
    fn assert_never_mentions(&self, from: usize, id: u64) {
        for update in &self.popped[from..] {
            if let ReplicaUpdate::Delta(gated) = update {
                let ops = match gated {
                    GatedDelta::V1(frame) => frame
                        .ops
                        .iter()
                        .map(|op| DeltaOp::Legacy(op.clone()))
                        .collect(),
                    GatedDelta::V2(frame) => frame.ops.clone(),
                };
                for op in ops {
                    assert!(
                        !format!("{op:?}").contains(&format!("NetId({id})")),
                        "{op:?}"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Harness.
// ---------------------------------------------------------------------------

struct Rig {
    host: Host,
    protocol: ProtocolSelection,
    now: u64,
}

impl Rig {
    fn new(protocol: ProtocolSelection) -> Self {
        let host = Host::from_history(
            fixture(),
            HostConfig {
                protocol,
                incarnation: 1,
            },
            0,
        )
        .expect("wraps an acknowledged authority");
        Self {
            host,
            protocol,
            now: 0,
        }
    }

    fn epoch(&self, peer: PeerHandle) -> [u8; 16] {
        self.host.epoch(peer).expect("bound")
    }

    fn tick(&self) -> u64 {
        self.host.server_tick()
    }

    fn bind(&mut self, control: ControlGrant, grants: DisclosureGrants) -> PeerHandle {
        self.host
            .bind_peer(control, grants, self.now)
            .expect("binds")
    }

    fn bind_snapshot(&mut self, control: ControlGrant, grants: DisclosureGrants) -> PeerHandle {
        self.host
            .bind_snapshot_peer(control, grants, self.now)
            .expect("binds")
    }

    fn advance(&mut self, ms: u64) {
        self.now += ms;
    }

    fn ingest(&mut self, peer: PeerHandle, bytes: &[u8]) {
        self.advance(100);
        assert_eq!(
            self.host.ingest(peer, bytes, self.now),
            Ok(IngestDisposition::Staged)
        );
    }

    fn pump(&mut self) -> PumpSummary {
        self.host.pump(self.now).expect("pump")
    }

    fn send(&mut self, peer: PeerHandle, bytes: &[u8]) -> AdmissionResult {
        self.ingest(peer, bytes);
        let summary = self.pump();
        assert_eq!(summary.results.len(), 1, "{summary:?}");
        summary.results[0].clone()
    }

    fn intent(&self, peer: PeerHandle, seq: u64, actor: u64, body: IntentBody) -> Vec<u8> {
        let frame = IntentFrame {
            epoch: self.epoch(peer),
            lane: LANE_COMBAT,
            seq,
            observed_tick: self.tick(),
            actor: n(actor),
            body,
        };
        match self.protocol {
            ProtocolSelection::V1 => codec::encode_intent(&frame),
            ProtocolSelection::V2 => codec_v2::encode_intent(&frame),
        }
        .expect("encodes")
    }

    fn declare(
        &self,
        peer: PeerHandle,
        seq: u64,
        actor: u64,
        ability: Ulid,
        target: u64,
    ) -> Vec<u8> {
        self.intent(
            peer,
            seq,
            actor,
            IntentBody::DeclareAction {
                ability,
                target: n(target),
            },
        )
    }

    fn entries(&mut self, peer: PeerHandle) -> Vec<DeliveryEntry> {
        self.host.take_delivery_entries(peer).expect("bound")
    }

    fn ack(&mut self, peer: PeerHandle, through: u64) {
        self.host.acknowledge_delivery(peer, through).expect("acks");
    }

    fn ack_all(&mut self, peer: PeerHandle) {
        let entries = self.entries(peer);
        if let Some(last) = entries.last() {
            let ordinal = last.ordinal;
            self.ack(peer, ordinal);
        }
    }

    fn transfer_state(&self, peer: PeerHandle) -> Option<TransferState> {
        self.host.transfer_state(peer).expect("bound")
    }

    fn complete(&self) -> CompleteState {
        CompleteState {
            hash: self.host.authority_hash(),
            executions: self.host.executions(),
            last_capture: self.host.last_capture(),
            capture_acknowledged: self.host.capture_acknowledged(),
        }
    }

    fn evidence(&mut self, peer: PeerHandle) -> Evidence {
        let entries = self.host.take_delivery_entries(peer);
        let state = self.host.transfer_state(peer);
        Evidence {
            entries,
            state,
            complete: self.complete(),
        }
    }
}

/// Complete-state-unchanged evidence: authority hash, executions, captures.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CompleteState {
    hash: [u8; 32],
    executions: u64,
    last_capture: u64,
    capture_acknowledged: u64,
}

/// One refusal's full evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Evidence {
    entries: Result<Vec<DeliveryEntry>, HostError>,
    state: Result<Option<TransferState>, HostError>,
    complete: CompleteState,
}

/// An ordinal-ordered log (each entry taken exactly once).
struct Log {
    seen: u64,
}

impl Log {
    fn new() -> Self {
        Self { seen: 0 }
    }

    fn take(&mut self, rig: &mut Rig, peer: PeerHandle) -> Vec<DeliveryEntry> {
        let all = rig.entries(peer);
        let fresh = all
            .into_iter()
            .filter(|entry| entry.ordinal > self.seen)
            .collect::<Vec<_>>();
        if let Some(last) = fresh.last() {
            self.seen = last.ordinal;
        }
        fresh
    }
}

fn resync(epoch: [u8; 16], after: Option<u64>, reason: ResyncReason) -> Vec<u8> {
    codec_transfer::encode_up(&TransferUp::Resync {
        epoch,
        after: after.and_then(TransferId::new),
        reason,
    })
    .expect("encodes")
}

fn published(epoch: [u8; 16], transfer: u64) -> Vec<u8> {
    codec_transfer::encode_up(&TransferUp::Published {
        epoch,
        transfer: TransferId::new(transfer).expect("nonzero"),
    })
    .expect("encodes")
}

const ZEROES_4K: [u8; MAX_INTENT_FRAME_BYTES] = [0u8; MAX_INTENT_FRAME_BYTES];
const ZEROES_4076: [u8; 4_076] = [0u8; 4_076];

// ---------------------------------------------------------------------------
// S§11 case 1.
// ---------------------------------------------------------------------------

fn bind_error(
    rig: &mut Rig,
    snapshot: bool,
    control: ControlGrant,
    grants: DisclosureGrants,
) -> Result<PeerHandle, HostError> {
    if snapshot {
        rig.host.bind_snapshot_peer(control, grants, rig.now)
    } else {
        rig.host.bind_peer(control, grants, rig.now)
    }
}

#[test]
fn snapshot_session_binds_like_a_lane0_session() {
    let mut lane = Rig::new(V2);
    let mut snap = Rig::new(V2);
    let w_lane = lane.bind(control(&[]), full_grants());
    let w_snap = snap.bind_snapshot(control(&[]), full_grants());
    assert_eq!(w_lane, w_snap);
    assert_eq!(lane.epoch(w_lane), snap.epoch(w_snap));
    assert_eq!(lane.epoch(w_lane), epoch_of(1, 1));
    let lane_bytes = lane.host.take_delivery(w_lane).expect("bound");
    let snap_bytes = snap.host.take_delivery(w_snap).expect("bound");
    assert_eq!(lane_bytes, snap_bytes);
    let expected_entries = vec![DeliveryEntry {
        ordinal: 1,
        kind: DeliveryKind::Delta,
        bytes: lane_bytes[0].clone(),
    }];
    assert_eq!(lane.entries(w_lane), expected_entries);
    assert_eq!(snap.entries(w_snap), expected_entries);
    assert_eq!(lane.host.transfer_state(w_lane), Ok(None));
    assert_eq!(
        snap.host.transfer_state(w_snap),
        Ok(Some(TransferState {
            next_transfer: 1,
            in_flight: None,
        }))
    );

    // An unbound handle from each host is one this host issued.
    let mut lane_dead = Rig::new(V2);
    let mut snap_dead = Rig::new(V2);
    let lane_ghost = lane_dead.bind(control(&[]), full_grants());
    let snap_ghost = snap_dead.bind_snapshot(control(&[]), full_grants());
    lane_dead.host.unbind_peer(lane_ghost);
    snap_dead.host.unbind_peer(snap_ghost);
    assert_eq!(
        lane_dead.host.transfer_state(lane_ghost),
        Err(HostError::UnknownPeer)
    );
    assert_eq!(
        snap_dead.host.transfer_state(snap_ghost),
        Err(HostError::UnknownPeer)
    );

    let over_bound = ControlGrant {
        actors: (0..(MAX_CONTROLLED_PER_PEER as u32 + 1)).map(eid).collect(),
    };
    let mut duplicate = full_grants();
    duplicate.entities.push(full(a()));

    // Closed wins over every later check.
    let mut closed_lane = Rig::new(V2);
    let mut closed_snap = Rig::new(V2);
    closed_lane.host.shutdown();
    closed_snap.host.shutdown();
    assert_eq!(
        bind_error(
            &mut closed_lane,
            false,
            over_bound.clone(),
            duplicate.clone()
        ),
        Err(HostError::Closed)
    );
    assert_eq!(
        bind_error(
            &mut closed_snap,
            true,
            over_bound.clone(),
            duplicate.clone()
        ),
        Err(HostError::Closed)
    );

    // Time regression wins over control and grant validation.
    let mut time_lane = Rig::new(V2);
    let mut time_snap = Rig::new(V2);
    time_lane.advance(100);
    time_snap.advance(100);
    time_lane.bind(control(&[]), full_grants());
    time_snap.bind_snapshot(control(&[]), full_grants());
    time_lane.now -= 1;
    time_snap.now -= 1;
    assert_eq!(
        bind_error(&mut time_lane, false, over_bound.clone(), duplicate.clone()),
        Err(HostError::TimeRegression)
    );
    assert_eq!(
        bind_error(&mut time_snap, true, over_bound.clone(), duplicate.clone()),
        Err(HostError::TimeRegression)
    );

    // Control validation wins over grant validation.
    let mut control_lane = Rig::new(V2);
    let mut control_snap = Rig::new(V2);
    assert_eq!(
        bind_error(
            &mut control_lane,
            false,
            over_bound.clone(),
            duplicate.clone()
        ),
        Err(HostError::InvalidControl)
    );
    assert_eq!(
        bind_error(
            &mut control_snap,
            true,
            over_bound.clone(),
            duplicate.clone()
        ),
        Err(HostError::InvalidControl)
    );

    // Grant validation wins over the peer bound.
    let mut grants_lane = Rig::new(V2);
    let mut grants_snap = Rig::new(V2);
    for _ in 0..MAX_PEERS {
        grants_lane.bind(control(&[]), full_grants());
        grants_snap.bind_snapshot(control(&[]), full_grants());
    }
    assert_eq!(
        bind_error(&mut grants_lane, false, control(&[]), duplicate.clone()),
        Err(HostError::InvalidGrants)
    );
    assert_eq!(
        bind_error(&mut grants_snap, true, control(&[]), duplicate.clone()),
        Err(HostError::InvalidGrants)
    );

    // The ninth binding is refused.
    let mut busy_lane = Rig::new(V2);
    let mut busy_snap = Rig::new(V2);
    for _ in 0..MAX_PEERS {
        busy_lane.bind(control(&[]), full_grants());
        busy_snap.bind_snapshot(control(&[]), full_grants());
    }
    assert_eq!(
        bind_error(&mut busy_lane, false, control(&[]), full_grants()),
        Err(HostError::ServerBusy)
    );
    assert_eq!(
        bind_error(&mut busy_snap, true, control(&[]), full_grants()),
        Err(HostError::ServerBusy)
    );
}

// ---------------------------------------------------------------------------
// S§11 case 2.
// ---------------------------------------------------------------------------

fn narrowing_stages_a_transfer_without_reset(protocol: ProtocolSelection) {
    let mut rig = Rig::new(protocol);
    let w = rig.bind_snapshot(control(&[]), full_grants());
    let p = rig.bind(all_actors(), full_grants());
    assert_eq!(w.get(), 1);
    assert_eq!(p.get(), 2);
    rig.ack_all(p);
    let epoch = rig.epoch(w);
    let script = rig.declare(p, 1, 1, strike(), 2);
    assert_eq!(rig.send(p, &script).status, APPLIED);
    assert_eq!(
        rig.entries(w)
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![(1, DeliveryKind::Delta), (2, DeliveryKind::Delta)]
    );

    assert_eq!(
        rig.host.set_grants(w, hide_b(full_grants())),
        Ok(GrantUpdate::SnapshotStaged)
    );
    let entries = rig.entries(w);
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![(3, DeliveryKind::Transfer), (4, DeliveryKind::Transfer)]
    );
    let begin = decode_begin(&entries[0].bytes);
    assert_eq!(begin.epoch, epoch);
    assert_eq!(begin.transfer, TransferId::new(1).expect("nonzero"));
    assert_eq!(begin.base_tick, rig.tick());
    assert_eq!(begin.cursor, 2);
    assert_eq!(begin.total_bytes, 14);
    assert_eq!(begin.entity_count, 2);
    assert_eq!(begin.segments_per_chunk, 8);
    assert_eq!(begin.chunk_count, 1);
    assert_eq!(decode_chunk(&entries[1].bytes), b_hidden());
    let planned = codec_transfer::plan_transfer(
        epoch,
        TransferId::new(1).expect("nonzero"),
        rig.tick(),
        2,
        8,
        &codec_transfer::decode_body(&b_hidden()).expect("decodes"),
    )
    .expect("plans");
    assert_eq!(
        vec![entries[0].bytes.clone(), entries[1].bytes.clone()],
        planned
    );
    assert_eq!(
        rig.transfer_state(w),
        Some(TransferState {
            next_transfer: 2,
            in_flight: Some(1),
        })
    );

    // P's seq 2 (B strikes C) reaches W as a delta at event_seq 3.
    let second = rig.declare(p, 2, 2, strike(), 3);
    assert_eq!(rig.send(p, &second).status, APPLIED);
    let entries = rig.entries(w);
    assert_eq!(entries.len(), 3);
    assert_eq!(
        (entries[2].ordinal, entries[2].kind),
        (5, DeliveryKind::Delta)
    );
    let frame = decode_frame(protocol, &entries[2].bytes);
    assert_eq!(frame.event_seq, 3);
    assert_eq!(frame.ops, for_version(protocol, later_ops()));

    // Widening is incremental, above the snapshot cursor.
    assert_eq!(
        rig.host.set_grants(w, full_grants()),
        Ok(GrantUpdate::Unchanged)
    );
    let entries = rig.entries(w);
    assert_eq!(entries.len(), 4);
    assert_eq!(
        (entries[3].ordinal, entries[3].kind),
        (6, DeliveryKind::Delta)
    );
    let frame = decode_frame(protocol, &entries[3].bytes);
    assert_eq!(frame.event_seq, 4);
    assert_eq!(
        frame.ops,
        for_version(protocol, vec![enter(4), health(4, 7, 10, false)])
    );

    // Control: a `bind_peer` watcher keeps the T022 reset.
    let mut lane = Rig::new(protocol);
    let w_lane = lane.bind(control(&[]), full_grants());
    let p_lane = lane.bind(all_actors(), full_grants());
    lane.ack_all(p_lane);
    let script = lane.declare(p_lane, 1, 1, strike(), 2);
    assert_eq!(lane.send(p_lane, &script).status, APPLIED);
    assert_eq!(
        lane.host.set_grants(w_lane, hide_b(full_grants())),
        Ok(GrantUpdate::RebindRequired)
    );
    let reset = lane.entries(w_lane);
    assert_eq!(reset.len(), 1);
    let frame = decode_frame(protocol, &reset[0].bytes);
    assert_eq!(frame.event_seq, 1);
    assert_eq!(
        frame.ops,
        for_version(
            protocol,
            vec![
                enter(1),
                enter(3),
                health(1, 10, 10, false),
                health(3, 3, 3, false),
                turn(None, 0),
            ]
        )
    );
}
both_versions!(narrowing_stages_a_transfer_without_reset);

// ---------------------------------------------------------------------------
// S§11 case 3.
// ---------------------------------------------------------------------------

#[test]
fn acknowledgement_is_by_ordinal() {
    let protocol = V2;
    let mut rig = Rig::new(protocol);
    let w = rig.bind_snapshot(control(&[]), full_grants());
    let p = rig.bind(all_actors(), full_grants());
    rig.ack_all(p);
    let script = rig.declare(p, 1, 1, strike(), 2);
    assert_eq!(rig.send(p, &script).status, APPLIED);
    assert_eq!(
        rig.host.set_grants(w, hide_b(full_grants())),
        Ok(GrantUpdate::SnapshotStaged)
    );

    // Through 2 is at or below the clear's watermark: nothing leaves.
    assert_eq!(rig.host.acknowledge_delivery(w, 2), Ok(()));
    assert_eq!(
        rig.entries(w)
            .iter()
            .map(|entry| entry.ordinal)
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
    assert_eq!(
        rig.host.acknowledge_delivery(w, 5),
        Err(HostError::FutureDeliveryCursor {
            requested: 5,
            last: 4,
        })
    );
    assert_eq!(rig.host.acknowledge_delivery(w, 3), Ok(()));
    assert_eq!(
        rig.entries(w)
            .iter()
            .map(|entry| entry.ordinal)
            .collect::<Vec<_>>(),
        vec![4]
    );
    assert_eq!(rig.host.acknowledge_delivery(w, 4), Ok(()));
    assert!(rig.entries(w).is_empty());
    assert_eq!(rig.host.acknowledge_delivery(w, 4), Ok(()));

    // Control: in a lane-0 session every ordinal equals its event_seq.
    let mut lane = Rig::new(protocol);
    let w_lane = lane.bind(control(&[]), full_grants());
    let p_lane = lane.bind(all_actors(), full_grants());
    lane.ack_all(p_lane);
    assert_lane0_ordinals(&mut lane, p_lane);
    let script = lane.declare(p_lane, 1, 1, strike(), 2);
    assert_eq!(lane.send(p_lane, &script).status, APPLIED);
    assert_lane0_ordinals(&mut lane, p_lane);
    assert_eq!(
        lane.host.set_grants(w_lane, hide_b(full_grants())),
        Ok(GrantUpdate::RebindRequired)
    );
    assert_lane0_ordinals(&mut lane, p_lane);
    let second = lane.declare(p_lane, 2, 2, strike(), 3);
    assert_eq!(lane.send(p_lane, &second).status, APPLIED);
    assert_lane0_ordinals(&mut lane, p_lane);
    assert_eq!(
        lane.host.set_grants(w_lane, full_grants()),
        Ok(GrantUpdate::Unchanged)
    );
    assert_lane0_ordinals(&mut lane, p_lane);
}

fn assert_lane0_ordinals(rig: &mut Rig, peer: PeerHandle) {
    for entry in rig.entries(peer) {
        assert_eq!(entry.kind, DeliveryKind::Delta);
        let frame = decode_frame(rig.protocol, &entry.bytes);
        assert_eq!(entry.ordinal, frame.event_seq);
    }
}

// ---------------------------------------------------------------------------
// S§11 case 4.
// ---------------------------------------------------------------------------

/// Runs script S (or S' with `whiff`) and narrows a fresh watcher, returning
/// the staged `Chunk` payload.
fn staged_chunk(protocol: ProtocolSelection, grants: DisclosureGrants, whiffed: bool) -> Vec<u8> {
    let mut rig = Rig::new(protocol);
    let w = rig.bind_snapshot(control(&[]), full_grants());
    let p = rig.bind(all_actors(), full_grants());
    rig.ack_all(p);
    let ability = if whiffed { whiff() } else { strike() };
    let script = rig.declare(p, 1, 1, ability, 2);
    assert_eq!(rig.send(p, &script).status, APPLIED);
    assert_eq!(
        rig.host.set_grants(w, grants),
        Ok(GrantUpdate::SnapshotStaged)
    );
    let entries = rig.entries(w);
    assert_eq!(entries.len(), 2);
    decode_chunk(&entries[1].bytes)
}

fn body_holds_only_granted_facts(protocol: ProtocolSelection) {
    let cases: [(DisclosureGrants, Vec<u8>, SnapshotBody); 3] = [
        (hide_b(full_grants()), b_hidden(), b_hidden_body()),
        (
            a_no_health_b(full_grants()),
            a_no_health_b_body(),
            a_no_health_b_snapshot(),
        ),
        (no_turn(full_grants()), no_turn_body(), no_turn_snapshot()),
    ];
    for (grants, bytes, expected_body) in cases {
        let chunk = staged_chunk(protocol, grants, false);
        assert_eq!(chunk, bytes);
        assert_eq!(
            codec_transfer::decode_body(&chunk).expect("decodes"),
            expected_body
        );
    }

    // Hidden-value control: S' (A whiffs B, so B stays 10/10) gives a
    // byte-identical chunk under B_hidden.
    let s = staged_chunk(protocol, hide_b(full_grants()), false);
    let s_prime = staged_chunk(protocol, hide_b(full_grants()), true);
    assert_eq!(s, s_prime);
    assert_eq!(s, b_hidden());

    // Positive control: with B visible and A's health off, S and S' differ in
    // exactly B's health byte.
    let visible = staged_chunk(protocol, a_no_health_b(full_grants()), false);
    let visible_prime = staged_chunk(protocol, a_no_health_b(full_grants()), true);
    assert_eq!(visible, a_no_health_b_body());
    let differing: Vec<usize> = (0..visible.len())
        .filter(|index| visible[*index] != visible_prime[*index])
        .collect();
    assert_eq!(differing, vec![5]);
    assert_eq!(visible[5], 0x07);
    assert_eq!(visible_prime[5], 0x0A);
}
both_versions!(body_holds_only_granted_facts);

// ---------------------------------------------------------------------------
// S§11 case 5.
// ---------------------------------------------------------------------------

fn cancel_precedes_every_superseding_transfer(protocol: ProtocolSelection) {
    let mut rig = Rig::new(protocol);
    let w = rig.bind_snapshot(control(&[]), full_grants());
    let p = rig.bind(all_actors(), full_grants());
    rig.ack_all(p);
    let epoch = rig.epoch(w);
    let mut log = Log::new();
    let mut pool = Sink::new(epoch, protocol);
    let feed = |rig: &mut Rig, pool: &mut Sink, log: &mut Log| {
        let fresh = log.take(rig, w);
        for entry in &fresh {
            pool.offer(entry, rig.now)
                .expect("every taken entry is acceptable in order");
        }
        pool.drain();
        fresh
    };

    let script = rig.declare(p, 1, 1, strike(), 2);
    assert_eq!(rig.send(p, &script).status, APPLIED);
    feed(&mut rig, &mut pool, &mut log);

    assert_eq!(
        rig.host.set_grants(w, hide_b(full_grants())),
        Ok(GrantUpdate::SnapshotStaged)
    );
    feed(&mut rig, &mut pool, &mut log);
    rig.ack(w, 4);

    // Narrow again: transfer 1 is cancelled, transfer 2 staged.
    assert_eq!(
        rig.host.set_grants(w, a_no_health(full_grants())),
        Ok(GrantUpdate::SnapshotStaged)
    );
    let fresh = feed(&mut rig, &mut pool, &mut log);
    assert_eq!(
        fresh
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![
            (5, DeliveryKind::Transfer),
            (6, DeliveryKind::Transfer),
            (7, DeliveryKind::Transfer),
        ]
    );
    let mut cancel_bytes = vec![0x81, 0x02, 0x12];
    cancel_bytes.extend_from_slice(&epoch);
    cancel_bytes.extend_from_slice(&[0x01, 0x02]);
    assert_eq!(fresh[0].bytes, cancel_bytes);
    assert_eq!(
        decode_cancel(&fresh[0].bytes),
        (TransferId::new(1).expect("nonzero"), CancelReason::Aborted)
    );
    let begin = decode_begin(&fresh[1].bytes);
    assert_eq!(begin.transfer, TransferId::new(2).expect("nonzero"));
    assert_eq!(begin.cursor, 2);
    assert_eq!(decode_chunk(&fresh[2].bytes), a_no_health_body());
    assert_eq!(
        codec_transfer::decode_body(&decode_chunk(&fresh[2].bytes)).expect("decodes"),
        a_no_health_snapshot()
    );
    assert_eq!(
        rig.transfer_state(w),
        Some(TransferState {
            next_transfer: 3,
            in_flight: Some(2),
        })
    );

    // Resync supersedes transfer 2 and stages 3 under the same grants.
    assert_eq!(
        rig.host
            .ingest_transfer(w, &resync(epoch, Some(2), ResyncReason::Expired), rig.now),
        Ok(TransferDisposition::Resynced)
    );
    let fresh = feed(&mut rig, &mut pool, &mut log);
    assert_eq!(
        fresh
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![
            (8, DeliveryKind::Transfer),
            (9, DeliveryKind::Transfer),
            (10, DeliveryKind::Transfer),
        ]
    );
    assert_eq!(&fresh[0].bytes[fresh[0].bytes.len() - 2..], &[0x02, 0x01]);
    assert_eq!(
        decode_cancel(&fresh[0].bytes),
        (
            TransferId::new(2).expect("nonzero"),
            CancelReason::Superseded
        )
    );
    let begin = decode_begin(&fresh[1].bytes);
    assert_eq!(begin.transfer, TransferId::new(3).expect("nonzero"));
    assert_eq!(begin.cursor, 2);
    assert_eq!(decode_chunk(&fresh[2].bytes), a_no_health_body());
    assert_eq!(
        rig.transfer_state(w),
        Some(TransferState {
            next_transfer: 4,
            in_flight: Some(3),
        })
    );

    assert_eq!(
        rig.host.ingest_transfer(w, &published(epoch, 3), rig.now),
        Ok(TransferDisposition::Published)
    );
    assert_eq!(
        rig.transfer_state(w),
        Some(TransferState {
            next_transfer: 4,
            in_flight: None,
        })
    );

    // No in-flight transfer: a narrowing stages Begin + Chunk with no Cancel.
    assert_eq!(
        rig.host.set_grants(w, no_turn_hidden_b(full_grants())),
        Ok(GrantUpdate::SnapshotStaged)
    );
    let fresh = feed(&mut rig, &mut pool, &mut log);
    assert_eq!(fresh.len(), 2);
    let begin = decode_begin(&fresh[0].bytes);
    assert_eq!(begin.transfer, TransferId::new(4).expect("nonzero"));
    assert_eq!(begin.cursor, 2);
    assert!(!fresh
        .iter()
        .any(|entry| { matches!(decode_transfer(&entry.bytes), TransferDown::Cancel { .. }) }));
}
both_versions!(cancel_precedes_every_superseding_transfer);

// ---------------------------------------------------------------------------
// S§11 case 6.
// ---------------------------------------------------------------------------

#[test]
fn ingest_transfer_precedence_and_refusals() {
    let mut rig = Rig::new(V2);
    let w = rig.bind_snapshot(control(&[]), full_grants());
    let p = rig.bind(all_actors(), full_grants());
    rig.ack_all(p);
    let script = rig.declare(p, 1, 1, strike(), 2);
    assert_eq!(rig.send(p, &script).status, APPLIED);
    assert_eq!(
        rig.host.set_grants(w, hide_b(full_grants())),
        Ok(GrantUpdate::SnapshotStaged)
    );
    let epoch = rig.epoch(w);
    let p_epoch = rig.epoch(p);
    let now = rig.now;
    let begin_frame = rig.entries(w)[0].bytes.clone();
    let before = rig.evidence(w);

    // Time regression (before the rate spend).
    assert_eq!(
        rig.host.ingest_transfer(w, &published(epoch, 1), now - 1),
        Err(HostError::TimeRegression)
    );
    assert_eq!(rig.evidence(w), before);

    // Unbound handles, even with garbage.
    let ghost = rig.bind_snapshot(control(&[]), full_grants());
    rig.host.unbind_peer(ghost);
    assert_eq!(
        rig.host.ingest_transfer(ghost, &[0xFF; 3], now),
        Ok(TransferDisposition::Refused(TransferRefusal::Unbound))
    );
    assert_eq!(rig.evidence(w), before);

    // A lane-0 session has no transfer channel, even for garbage.
    let before_p = rig.evidence(p);
    assert_eq!(
        rig.host.ingest_transfer(p, &[0xFF; 3], now),
        Ok(TransferDisposition::Refused(
            TransferRefusal::NotSnapshotPeer
        ))
    );
    assert_eq!(rig.evidence(p), before_p);
    assert_eq!(
        rig.host
            .ingest_transfer(p, &resync(p_epoch, None, ResyncReason::Overflow), now),
        Ok(TransferDisposition::Refused(
            TransferRefusal::NotSnapshotPeer
        ))
    );
    assert_eq!(rig.evidence(p), before_p);

    // Malformed inputs, each leaving the evidence untouched.
    let own_intent = rig.declare(p, 2, 1, strike(), 2);
    let mut bad_reason = resync(epoch, None, ResyncReason::Overflow);
    let last = bad_reason.len() - 1;
    bad_reason[last] = 0x00;
    let mut trailing = published(epoch, 1);
    trailing.push(0x00);
    let malformed: Vec<Vec<u8>> = vec![
        Vec::new(),
        vec![0u8; MAX_INTENT_FRAME_BYTES + 1],
        own_intent,
        begin_frame,
        bad_reason,
        trailing,
    ];
    for bytes in malformed {
        assert_eq!(
            rig.host.ingest_transfer(w, &bytes, now),
            Ok(TransferDisposition::Refused(TransferRefusal::Malformed)),
            "{bytes:02X?}"
        );
        assert_eq!(rig.evidence(w), before);
    }

    // Wrong epoch, then unissued transfer ids.
    assert_eq!(
        rig.host.ingest_transfer(w, &published(p_epoch, 1), now),
        Ok(TransferDisposition::Refused(TransferRefusal::WrongEpoch))
    );
    assert_eq!(rig.evidence(w), before);
    assert_eq!(
        rig.host.ingest_transfer(w, &published(epoch, 2), now),
        Ok(TransferDisposition::Refused(
            TransferRefusal::UnknownTransfer
        ))
    );
    assert_eq!(rig.evidence(w), before);
    assert_eq!(
        rig.host
            .ingest_transfer(w, &resync(epoch, Some(2), ResyncReason::Overflow), now),
        Ok(TransferDisposition::Refused(
            TransferRefusal::UnknownTransfer
        ))
    );
    assert_eq!(rig.evidence(w), before);

    // Positive control: the in-flight transfer publishes once, then is stale.
    assert_eq!(
        rig.host.ingest_transfer(w, &published(epoch, 1), now),
        Ok(TransferDisposition::Published)
    );
    let after_publish = rig.evidence(w);
    assert_eq!(
        rig.host.ingest_transfer(w, &published(epoch, 1), now),
        Ok(TransferDisposition::Stale)
    );
    assert_eq!(rig.evidence(w), after_publish);

    // All three rate peers bind at the same instant, before any call moves
    // the host-wide accepted time forward.
    let r = rig.bind_snapshot(control(&[]), full_grants());
    let r_epoch = rig.epoch(r);
    let r2 = rig.bind_snapshot(control(&[]), full_grants());
    let r2_epoch = rig.epoch(r2);
    let r3 = rig.bind_snapshot(control(&[]), full_grants());
    let r3_epoch = rig.epoch(r3);

    // Frame budget: 80 attempts spend the burst at one instant.
    for _ in 0..80 {
        assert_eq!(
            rig.host
                .ingest_transfer(r, &resync(r_epoch, Some(999), ResyncReason::Overflow), now),
            Ok(TransferDisposition::Refused(
                TransferRefusal::UnknownTransfer
            ))
        );
    }
    assert_eq!(
        rig.host
            .ingest_transfer(r, &resync(r_epoch, Some(999), ResyncReason::Overflow), now),
        Ok(TransferDisposition::Refused(TransferRefusal::RateLimited))
    );
    assert_eq!(
        rig.host.ingest(r, &[0x00], now),
        Ok(IngestDisposition::Refused {
            status: ReceiptStatus::Rejected(RejectionCode::RateLimited),
        })
    );

    // Byte budget: 32 × 4,096 bytes is exactly the burst.
    for _ in 0..32 {
        assert_eq!(
            rig.host.ingest_transfer(r2, &ZEROES_4K, now),
            Ok(TransferDisposition::Refused(TransferRefusal::Malformed))
        );
    }
    assert_eq!(
        rig.host.ingest_transfer(r2, &published(r2_epoch, 1), now),
        Ok(TransferDisposition::Refused(TransferRefusal::RateLimited))
    );
    assert_eq!(
        rig.host.ingest(r2, &[0x00], now),
        Ok(IngestDisposition::Refused {
            status: ReceiptStatus::Rejected(RejectionCode::RateLimited),
        })
    );

    // Boundary: 31 × 4,096 + 4,076 leaves exactly the 20-byte frame.
    for _ in 0..31 {
        assert_eq!(
            rig.host.ingest_transfer(r3, &ZEROES_4K, now),
            Ok(TransferDisposition::Refused(TransferRefusal::Malformed))
        );
    }
    assert_eq!(
        rig.host.ingest_transfer(r3, &ZEROES_4076, now),
        Ok(TransferDisposition::Refused(TransferRefusal::Malformed))
    );
    assert_eq!(
        rig.host.ingest_transfer(r3, &published(r3_epoch, 1), now),
        Ok(TransferDisposition::Refused(
            TransferRefusal::UnknownTransfer
        ))
    );
    assert_eq!(
        rig.host.ingest_transfer(r3, &published(r3_epoch, 1), now),
        Ok(TransferDisposition::Refused(TransferRefusal::RateLimited))
    );

    // After a second the frame bucket has refilled for R.
    assert_eq!(
        rig.host.ingest_transfer(
            r,
            &resync(r_epoch, Some(999), ResyncReason::Overflow),
            now + 1_000
        ),
        Ok(TransferDisposition::Refused(
            TransferRefusal::UnknownTransfer
        ))
    );

    // Closed wins over everything and changes no complete-state evidence.
    let closed_before = rig.complete();
    rig.host.shutdown();
    assert_eq!(
        rig.host.ingest_transfer(w, &published(epoch, 1), now),
        Err(HostError::Closed)
    );
    assert_eq!(rig.complete(), closed_before);
}

// ---------------------------------------------------------------------------
// S§11 case 7.
// ---------------------------------------------------------------------------

fn resync_clears_and_restarts_from_the_cursor(protocol: ProtocolSelection) {
    let mut rig = Rig::new(protocol);
    let w = rig.bind_snapshot(control(&[]), full_grants());
    let p = rig.bind(all_actors(), full_grants());
    rig.ack_all(p);
    let epoch = rig.epoch(w);
    let script = rig.declare(p, 1, 1, strike(), 2);
    assert_eq!(rig.send(p, &script).status, APPLIED);
    assert_eq!(
        rig.entries(w)
            .iter()
            .map(|entry| entry.ordinal)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );

    // Control: an unissued `after` is refused and changes nothing.
    let before = rig.evidence(w);
    assert_eq!(
        rig.host
            .ingest_transfer(w, &resync(epoch, Some(5), ResyncReason::Overflow), rig.now),
        Ok(TransferDisposition::Refused(
            TransferRefusal::UnknownTransfer
        ))
    );
    assert_eq!(rig.evidence(w), before);

    assert_eq!(
        rig.host
            .ingest_transfer(w, &resync(epoch, None, ResyncReason::Overflow), rig.now),
        Ok(TransferDisposition::Resynced)
    );
    let entries = rig.entries(w);
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![(3, DeliveryKind::Transfer), (4, DeliveryKind::Transfer)]
    );
    let begin = decode_begin(&entries[0].bytes);
    assert_eq!(begin.transfer, TransferId::new(1).expect("nonzero"));
    assert_eq!(begin.cursor, 2);
    assert_eq!(decode_chunk(&entries[1].bytes), full_after_1());
    assert_eq!(
        codec_transfer::decode_body(&decode_chunk(&entries[1].bytes)).expect("decodes"),
        full_after_1_body()
    );
    assert_eq!(
        rig.transfer_state(w),
        Some(TransferState {
            next_transfer: 2,
            in_flight: Some(1),
        })
    );

    let second = rig.declare(p, 2, 2, strike(), 3);
    assert_eq!(rig.send(p, &second).status, APPLIED);
    let entries = rig.entries(w);
    assert_eq!(entries.len(), 3);
    let frame = decode_frame(protocol, &entries[2].bytes);
    assert_eq!(frame.event_seq, 3);

    // A Resync naming the in-flight id cancels it before the new Begin.
    assert_eq!(
        rig.host
            .ingest_transfer(w, &resync(epoch, Some(1), ResyncReason::Expired), rig.now),
        Ok(TransferDisposition::Resynced)
    );
    let entries = rig.entries(w);
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![
            (6, DeliveryKind::Transfer),
            (7, DeliveryKind::Transfer),
            (8, DeliveryKind::Transfer),
        ]
    );
    assert_eq!(
        decode_cancel(&entries[0].bytes),
        (
            TransferId::new(1).expect("nonzero"),
            CancelReason::Superseded
        )
    );
    let begin = decode_begin(&entries[1].bytes);
    assert_eq!(begin.transfer, TransferId::new(2).expect("nonzero"));
    assert_eq!(begin.cursor, 3);
    assert_eq!(
        rig.transfer_state(w),
        Some(TransferState {
            next_transfer: 3,
            in_flight: Some(2),
        })
    );
}
both_versions!(resync_clears_and_restarts_from_the_cursor);

// ---------------------------------------------------------------------------
// S§11 case 8.
// ---------------------------------------------------------------------------

fn cleared_receipt_recovers_by_retry(protocol: ProtocolSelection) {
    let mut rig = Rig::new(protocol);
    let _w = rig.bind_snapshot(control(&[]), full_grants());
    let q = rig.bind_snapshot(all_actors(), full_grants());
    rig.ack_all(q);
    let epoch = rig.epoch(q);
    let bytes = rig.declare(q, 1, 1, strike(), 2);
    assert_eq!(rig.send(q, &bytes).status, APPLIED);
    assert_eq!(rig.entries(q).len(), 1);

    assert_eq!(
        rig.host
            .ingest_transfer(q, &resync(epoch, None, ResyncReason::Overflow), rig.now),
        Ok(TransferDisposition::Resynced)
    );
    let entries = rig.entries(q);
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![(3, DeliveryKind::Transfer), (4, DeliveryKind::Transfer)]
    );

    // The retry is a cache hit and stages a receipt-only delta above the
    // snapshot cursor, carrying the original processed tick.
    rig.ingest(q, &bytes);
    let summary = rig.pump();
    assert_eq!(summary.results.len(), 1);
    assert!(summary.results[0].cached);
    assert_eq!(summary.results[0].status, APPLIED);
    let entries = rig.entries(q);
    assert_eq!(entries.len(), 3);
    assert_eq!(
        (entries[2].ordinal, entries[2].kind),
        (5, DeliveryKind::Delta)
    );
    let frame = decode_frame(protocol, &entries[2].bytes);
    assert_eq!(frame.event_seq, 3);
    assert_eq!(
        frame.ops,
        for_version(protocol, vec![receipt(epoch, 1, 0, APPLIED)])
    );
    let receipt_ops = frame
        .ops
        .iter()
        .filter(|op| matches!(op, DeltaOp::Legacy(protocol::DeltaOp::Receipt { .. })))
        .count();
    assert_eq!(receipt_ops, 1);

    // Control: without the Resync the receipt is still deliverable, so the
    // retry adds no entry.
    let mut lane = Rig::new(protocol);
    let q2 = lane.bind_snapshot(all_actors(), full_grants());
    lane.ack_all(q2);
    let bytes = lane.declare(q2, 1, 1, strike(), 2);
    assert_eq!(lane.send(q2, &bytes).status, APPLIED);
    let before = lane.entries(q2);
    assert_eq!(before.len(), 1);
    lane.ingest(q2, &bytes);
    let summary = lane.pump();
    assert!(summary.results[0].cached);
    assert_eq!(lane.entries(q2), before);
}
both_versions!(cleared_receipt_recovers_by_retry);

// ---------------------------------------------------------------------------
// S§11 case 9.
// ---------------------------------------------------------------------------

fn later_deltas_follow_the_begin_and_cut_over(protocol: ProtocolSelection) {
    let mut rig = Rig::new(protocol);
    let w = rig.bind_snapshot(control(&[]), full_grants());
    let p = rig.bind(all_actors(), full_grants());
    rig.ack_all(p);
    let epoch = rig.epoch(w);
    let mut sink = Sink::new(epoch, protocol);

    let script = rig.declare(p, 1, 1, strike(), 2);
    assert_eq!(rig.send(p, &script).status, APPLIED);
    let first = rig.entries(w);
    let fed = sink.feed(&first, rig.now);
    assert_eq!(
        fed,
        vec![
            Fed::Delta(DeltaOutcome::Released { count: 1 }),
            Fed::Delta(DeltaOutcome::Released { count: 1 }),
        ]
    );

    assert_eq!(
        rig.host.set_grants(w, hide_b(full_grants())),
        Ok(GrantUpdate::SnapshotStaged)
    );
    let second = rig.declare(p, 2, 2, strike(), 3);
    assert_eq!(rig.send(p, &second).status, APPLIED);
    let third = rig.declare(p, 3, 1, strike(), 2);
    assert_eq!(rig.send(p, &third).status, APPLIED);

    let entries = rig.entries(w);
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![
            (3, DeliveryKind::Transfer),
            (4, DeliveryKind::Transfer),
            (5, DeliveryKind::Delta),
            (6, DeliveryKind::Delta),
        ]
    );
    let fed = sink.feed(&entries, rig.now);
    let transfer_id = TransferId::new(1).expect("nonzero");
    assert_eq!(
        fed,
        vec![
            Fed::Transfer(TransferOutcome::Begun),
            Fed::Transfer(TransferOutcome::Published {
                transfer: transfer_id,
                released: 0,
            }),
            Fed::Delta(DeltaOutcome::Released { count: 1 }),
            Fed::Delta(DeltaOutcome::Released { count: 1 }),
        ]
    );
    // D1, D2, Snapshot, D3, D4 in that order.
    assert_eq!(sink.popped.len(), 5);
    assert!(matches!(sink.popped[0], ReplicaUpdate::Delta(_)));
    assert!(matches!(sink.popped[1], ReplicaUpdate::Delta(_)));
    assert!(matches!(sink.popped[2], ReplicaUpdate::Snapshot(_)));
    assert!(matches!(sink.popped[3], ReplicaUpdate::Delta(_)));
    assert!(matches!(sink.popped[4], ReplicaUpdate::Delta(_)));

    // Negative control: D3 before the Begin releases at once, then the Begin
    // is behind the replica.
    let mut early = Sink::new(epoch, protocol);
    early.feed(&first, rig.now);
    let d3 = &entries[2];
    assert_eq!(
        early.offer(d3, rig.now),
        Ok(Fed::Delta(DeltaOutcome::Released { count: 1 }))
    );
    early.drain();
    assert_eq!(
        early.offer(&entries[0], rig.now),
        Err(TransferError::BehindReplica)
    );
}
both_versions!(later_deltas_follow_the_begin_and_cut_over);

// ---------------------------------------------------------------------------
// S§11 case 10.
// ---------------------------------------------------------------------------

fn widening_during_a_transfer_stays_incremental(protocol: ProtocolSelection) {
    let mut rig = Rig::new(protocol);
    let w = rig.bind_snapshot(control(&[]), full_grants());
    let p = rig.bind(all_actors(), full_grants());
    rig.ack_all(p);
    let epoch = rig.epoch(w);
    let mut sink = Sink::new(epoch, protocol);

    let script = rig.declare(p, 1, 1, strike(), 2);
    assert_eq!(rig.send(p, &script).status, APPLIED);
    let first = rig.entries(w);
    sink.feed(&first, rig.now);
    let mark = sink.popped.len();

    assert_eq!(
        rig.host.set_grants(w, hide_b(full_grants())),
        Ok(GrantUpdate::SnapshotStaged)
    );
    // Widen before the pool sees the Chunk.
    assert_eq!(
        rig.host.set_grants(w, full_grants()),
        Ok(GrantUpdate::Unchanged)
    );
    assert_eq!(
        rig.transfer_state(w).map(|state| state.next_transfer),
        Some(2)
    );
    let entries = rig.entries(w);
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![
            (3, DeliveryKind::Transfer),
            (4, DeliveryKind::Transfer),
            (5, DeliveryKind::Delta),
        ]
    );
    let widening = decode_frame(protocol, &entries[2].bytes);
    assert_eq!(widening.event_seq, 3);
    assert_eq!(
        widening.ops,
        for_version(protocol, vec![enter(4), health(4, 7, 10, false)])
    );

    let fed = sink.feed(&entries, rig.now);
    assert_eq!(
        fed,
        vec![
            Fed::Transfer(TransferOutcome::Begun),
            Fed::Transfer(TransferOutcome::Published {
                transfer: TransferId::new(1).expect("nonzero"),
                released: 0,
            }),
            Fed::Delta(DeltaOutcome::Released { count: 1 }),
        ]
    );
    assert_eq!(
        sink.replica,
        expected(
            &[
                (1, (10, 10, false)),
                (3, (3, 3, false)),
                (4, (7, 10, false)),
            ],
            Some((None, 0)),
        )
    );
    assert!(!sink.replica.health.contains_key(&2));
    sink.assert_never_mentions(mark, 2);

    // A second pool fed D3 after the Begin but before the Chunk buffers it,
    // then publishes with it released.
    let mut second = Sink::new(epoch, protocol);
    second.feed(&first, rig.now);
    assert_eq!(
        second.offer(&entries[0], rig.now),
        Ok(Fed::Transfer(TransferOutcome::Begun))
    );
    assert_eq!(
        second.offer(&entries[2], rig.now),
        Ok(Fed::Delta(DeltaOutcome::Buffered))
    );
    assert_eq!(
        second.offer(&entries[1], rig.now),
        Ok(Fed::Transfer(TransferOutcome::Published {
            transfer: TransferId::new(1).expect("nonzero"),
            released: 1,
        }))
    );
}
both_versions!(widening_during_a_transfer_stays_incremental);

// ---------------------------------------------------------------------------
// S§11 case 11.
// ---------------------------------------------------------------------------

fn independent_replica_oracle_over_a_scripted_fight(protocol: ProtocolSelection) {
    let mut rig = Rig::new(protocol);
    let w1 = rig.bind_snapshot(control(&[]), full_grants());
    let w2 = rig.bind_snapshot(control(&[]), observer_grants());
    let p = rig.bind(all_actors(), full_grants());
    rig.ack_all(p);
    let w1_epoch = rig.epoch(w1);
    let w2_epoch = rig.epoch(w2);
    let mut one = Sink::new(w1_epoch, protocol);
    let mut two = Sink::new(w2_epoch, protocol);

    let initial = expected(
        &[
            (1, (10, 10, false)),
            (2, (10, 10, false)),
            (3, (3, 3, false)),
        ],
        Some((Some(1), 0)),
    );
    let first_one = rig.entries(w1);
    assert_eq!(
        decode_frame(protocol, &first_one[0].bytes).ops,
        for_version(protocol, initial_full_state())
    );
    one.feed(&first_one, rig.now);
    two.feed(&rig.entries(w2), rig.now);
    assert_eq!(one.replica, initial);
    assert_eq!(two.replica, initial);

    // seq 1: A strikes B.
    let script = rig.declare(p, 1, 1, strike(), 2);
    assert_eq!(rig.send(p, &script).status, APPLIED);
    let after_one = expected(
        &[
            (1, (10, 10, false)),
            (2, (7, 10, false)),
            (3, (3, 3, false)),
        ],
        Some((Some(2), 0)),
    );
    one.feed(&rig.entries(w1), rig.now);
    two.feed(&rig.entries(w2), rig.now);
    assert_eq!(one.replica, after_one);
    assert_eq!(two.replica, after_one);

    // W2's narrowing publishes the B-free snapshot.
    let w2_mark = two.popped.len();
    assert_eq!(
        rig.host.set_grants(w2, hide_b(observer_grants())),
        Ok(GrantUpdate::SnapshotStaged)
    );
    let fed = two.feed(&rig.entries(w2), rig.now);
    assert_eq!(fed.len(), 2);
    assert_eq!(
        fed[1],
        Fed::Transfer(TransferOutcome::Published {
            transfer: TransferId::new(1).expect("nonzero"),
            released: 0,
        })
    );
    let w2_snapshot = expected(&[(1, (10, 10, false)), (3, (3, 3, false))], Some((None, 0)));
    assert_eq!(two.replica, w2_snapshot);
    assert!(!two.replica.health.contains_key(&2));
    assert_eq!(
        rig.host
            .ingest_transfer(w2, &published(w2_epoch, 1), rig.now),
        Ok(TransferDisposition::Published)
    );

    // seq 2: B strikes C.
    let second = rig.declare(p, 2, 2, strike(), 3);
    assert_eq!(rig.send(p, &second).status, APPLIED);
    one.feed(&rig.entries(w1), rig.now);
    two.feed(&rig.entries(w2), rig.now);
    assert_eq!(
        one.replica,
        expected(
            &[(1, (10, 10, false)), (2, (7, 10, false)), (3, (0, 3, true)),],
            Some((Some(1), 1)),
        )
    );
    assert_eq!(
        two.replica,
        expected(
            &[(1, (10, 10, false)), (3, (0, 3, true))],
            Some((Some(1), 1))
        )
    );

    // W1's Resync publishes the full snapshot.
    assert_eq!(
        rig.host
            .ingest_transfer(w1, &resync(w1_epoch, None, ResyncReason::Overflow), rig.now),
        Ok(TransferDisposition::Resynced)
    );
    let fed = one.feed(&rig.entries(w1), rig.now);
    assert_eq!(fed.len(), 2);
    let w1_snapshot = expected(
        &[(1, (10, 10, false)), (2, (7, 10, false)), (3, (0, 3, true))],
        Some((Some(1), 1)),
    );
    assert_eq!(one.replica, w1_snapshot);
    assert_eq!(
        rig.host
            .ingest_transfer(w1, &published(w1_epoch, 1), rig.now),
        Ok(TransferDisposition::Published)
    );

    // W2 widens back to the observer's full set: incremental, fresh id 4.
    assert_eq!(
        rig.host.set_grants(w2, observer_grants()),
        Ok(GrantUpdate::Unchanged)
    );
    two.feed(&rig.entries(w2), rig.now);
    assert_eq!(
        two.replica,
        expected(
            &[(1, (10, 10, false)), (3, (0, 3, true)), (4, (7, 10, false)),],
            Some((Some(1), 1)),
        )
    );

    // seq 3: A strikes B.
    let third = rig.declare(p, 3, 1, strike(), 2);
    assert_eq!(rig.send(p, &third).status, APPLIED);
    one.feed(&rig.entries(w1), rig.now);
    two.feed(&rig.entries(w2), rig.now);
    assert_eq!(
        one.replica,
        expected(
            &[(1, (10, 10, false)), (2, (4, 10, false)), (3, (0, 3, true)),],
            Some((Some(2), 1)),
        )
    );
    assert_eq!(
        two.replica,
        expected(
            &[(1, (10, 10, false)), (3, (0, 3, true)), (4, (4, 10, false)),],
            Some((Some(4), 1)),
        )
    );
    assert!(!two.replica.health.contains_key(&2));
    two.assert_never_mentions(w2_mark, 2);
}
both_versions!(independent_replica_oracle_over_a_scripted_fight);
