//! T018c conformance driver: sim-backed test double (test-only, never shipped).
//!
//! [`NetDriver`] sequences public sim calls only (`start_encounter` is test
//! choreography beforehand; `perform_action` inside [`admit`](NetDriver::admit);
//! `spawn`/`despawn` reads by tests directly on the exposed world). It never
//! drains the authoritative queue — projection observes a detached clone —
//! never changes hash exclusions, and never invents gameplay: `perform_action`
//! precedence stays authoritative, mapped through to `IllegalAction`
//! receipts with the typed error retained for diagnosis.
//!
//! Two independent paths read the same world: the delivery path
//! ([`deltas`](NetDriver::deltas), redeliverable per-client frames with
//! gapless `event_seq`) and the oracle path
//! ([`project`](NetDriver::project), direct permitted-view computation).
//! Replica convergence is proved by comparing them in the conformance suite.

use std::collections::{BTreeMap, BTreeSet};

use crpg_core::{EntityId, EventEnvelope, Tick};
use crpg_net::codec::{decode_intent, encode_delta, CodecError};
use crpg_net::protocol::{
    DeltaFrame, DeltaOp, IntentBody, NetId, ReceiptStatus, RejectionCode, LANE_COMBAT,
    POLICY_OBSERVED_TICK_WINDOW, POLICY_PROOF_PEERS, POLICY_RETRY_CACHE_BYTES_PER_PEER_LANE,
    POLICY_RETRY_CACHE_PER_PEER_LANE,
};
use crpg_net::sim::{PeerId, PeerState, RateCaps};
use crpg_sim::{perform_action, state_hash, CombatAction, SimEvent, World};

// ---------------------------------------------------------------------------
// Supplied tables: fixtures, not perception implementation.
// ---------------------------------------------------------------------------

/// Per-entity disclosure for one peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility {
    /// The peer may learn presence (enter/spawn/leave/despawn/died notices).
    pub present: bool,
    /// The peer may learn health/dead values.
    pub health: bool,
}

impl Visibility {
    /// Presence plus health.
    pub fn full() -> Self {
        Self {
            present: true,
            health: true,
        }
    }

    /// Presence without health values.
    pub fn presence_only() -> Self {
        Self {
            present: true,
            health: false,
        }
    }

    /// Nothing disclosable (the default for unlisted pairs).
    pub fn none() -> Self {
        Self {
            present: false,
            health: false,
        }
    }
}

/// Supplied peer-to-entity disclosure table.
///
/// Unlisted pairs are [`Visibility::none`]: projection is allow-listed, and
/// a retained mapping never grants disclosure on its own.
#[derive(Debug, Clone, Default)]
pub struct VisibilityTable {
    map: BTreeMap<(PeerId, EntityId), Visibility>,
}

impl VisibilityTable {
    /// Creates an empty (deny-all) table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets one pair's disclosure.
    pub fn set(&mut self, peer: PeerId, entity: EntityId, visibility: Visibility) {
        self.map.insert((peer, entity), visibility);
    }

    /// Reads one pair's disclosure, defaulting to hidden.
    pub fn get(&self, peer: PeerId, entity: EntityId) -> Visibility {
        self.map
            .get(&(peer, entity))
            .copied()
            .unwrap_or_else(Visibility::none)
    }
}

/// Supplied peer-to-actor control table: which entities a peer may act as.
#[derive(Debug, Clone, Default)]
pub struct OwnershipTable {
    map: BTreeMap<PeerId, Vec<EntityId>>,
}

impl OwnershipTable {
    /// Creates an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the actors one peer controls.
    pub fn set(&mut self, peer: PeerId, actors: Vec<EntityId>) {
        self.map.insert(peer, actors);
    }

    /// Whether the peer controls the entity.
    pub fn can(&self, peer: PeerId, entity: EntityId) -> bool {
        self.map
            .get(&peer)
            .is_some_and(|actors| actors.contains(&entity))
    }
}

// ---------------------------------------------------------------------------
// Expected view: the independent oracle path.
// ---------------------------------------------------------------------------

/// One filtered event in a permitted view, keyed by replica id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FilteredEvent {
    /// A visible spawn notice.
    Spawned {
        /// The spawned replica entity.
        entity: NetId,
    },
    /// A disclosable despawn notice.
    Despawned {
        /// The removed replica entity.
        entity: NetId,
    },
    /// A disclosable death notice; never implies despawn.
    Died {
        /// The replica entity that died.
        entity: NetId,
    },
}

/// Permitted health fact in a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthFact {
    /// The replica entity described.
    pub entity: NetId,
    /// Current health.
    pub health: u32,
    /// Maximum health.
    pub max_health: u32,
    /// Terminal flag.
    pub dead: bool,
}

/// Permitted turn fact in a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnFact {
    /// Active actor, if the encounter has one.
    pub active: Option<NetId>,
    /// Current round.
    pub round: u64,
}

/// One peer's independently projected permitted view.
///
/// Presence, health, and turn come straight from the world plus the tables;
/// events come from the full authoritative queue filtered the same way, with
/// per-client `event_seq` assigned in envelope order. Nothing here reads the
/// delivery machinery (`known` sets, cursors, sent logs), so equality with a
/// reconstructed replica is a real cross-check, not a copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedView {
    /// Visible replica entities, ascending.
    pub present: Vec<NetId>,
    /// Permitted health facts, ascending by entity.
    pub health: Vec<HealthFact>,
    /// Permitted turn fact, if any may be disclosed.
    pub turn: Option<TurnFact>,
    /// Filtered events with per-client sequence, in envelope order.
    pub events: Vec<(u64, FilteredEvent)>,
}

// ---------------------------------------------------------------------------
// The driver.
// ---------------------------------------------------------------------------

/// Per-lane admission decision before any world access.
enum SeqDecision {
    /// Retry of retained canonical bytes with its terminal outcome.
    Cached(ReceiptStatus),
    /// Same seq, different bytes: close the epoch.
    Conflict,
    /// Lane exhausted (`next_new_seq` past `SEQ_LAST`): rebind to a new epoch.
    /// Checked after cache lookup so retained retries still recover, and
    /// before gap/stale dispatch so exhaustion is never misreported as a
    /// retryable gap. Consumes no `seq`, closes nothing.
    Exhausted,
    /// Ahead of the next new seq: retry the missing command.
    Gap,
    /// Older than the retained cache: never reapplied.
    Stale,
    /// The next new seq: validate further.
    New,
}

/// Sim-backed net-local host stand-in.
///
/// Admits decoded intents through epoch, per-lane seq/cache, freshness,
/// mapping/visibility/ownership, and immediate `perform_action` legality,
/// returning receipt bytes; projects filtered deltas and independent views.
/// A test double passing here is not host integration passing — E012/E018/
/// E022 still own the real host.
pub struct NetDriver {
    /// The authoritative world. Tests choreograph encounters, spawns,
    /// despawns, and ticks directly; the driver only performs admitted
    /// actions and reads.
    pub world: World,
    /// The host's current tick view: freshness window anchor, receipt
    /// `processed_tick`, and delta `server_tick`. Tests control it.
    pub server_tick: u64,
    vis: VisibilityTable,
    own: OwnershipTable,
    states: BTreeMap<PeerId, PeerState>,
    closed: BTreeSet<[u8; 16]>,
    delivery: BTreeMap<PeerId, u64>,
    cursor: BTreeMap<PeerId, (Tick, u64)>,
    known: BTreeMap<PeerId, BTreeSet<NetId>>,
    map: BTreeMap<PeerId, BTreeMap<NetId, EntityId>>,
    graves: BTreeMap<PeerId, BTreeMap<EntityId, NetId>>,
    next_net: BTreeMap<PeerId, u64>,
    last_health: BTreeMap<PeerId, BTreeMap<NetId, (u32, u32, bool)>>,
    last_turn: BTreeMap<PeerId, Option<TurnFact>>,
    sent: BTreeMap<PeerId, BTreeMap<u64, (u64, Vec<DeltaOp>)>>,
    executions: u64,
    last_error: Option<String>,
    fail_now_ms: u64,
}

impl NetDriver {
    /// Binds the world with its supplied tables; the tick view starts at the
    /// world's current tick.
    pub fn new(world: World, vis: VisibilityTable, own: OwnershipTable) -> Self {
        let server_tick = world.tick().get();
        Self {
            world,
            server_tick,
            vis,
            own,
            states: BTreeMap::new(),
            closed: BTreeSet::new(),
            delivery: BTreeMap::new(),
            cursor: BTreeMap::new(),
            known: BTreeMap::new(),
            map: BTreeMap::new(),
            graves: BTreeMap::new(),
            next_net: BTreeMap::new(),
            last_health: BTreeMap::new(),
            last_turn: BTreeMap::new(),
            sent: BTreeMap::new(),
            executions: 0,
            last_error: None,
            fail_now_ms: 0,
        }
    }

    /// Binds one peer to an epoch with fresh lane-0 seq/cache state.
    ///
    /// Rebinding an already-bound peer (epoch rotation) always succeeds and
    /// replaces its lane state. Binding a new peer fails with
    /// [`RejectionCode::ServerBusy`] once [`POLICY_PROOF_PEERS`] distinct
    /// peers are bound; the fabric enforces the same bound on its side, and
    /// this keeps the driver from outgrowing the proof population it models.
    pub fn bind_peer(
        &mut self,
        peer: PeerId,
        epoch: [u8; 16],
        rates: &RateCaps,
    ) -> Result<(), RejectionCode> {
        if !self.states.contains_key(&peer) && self.states.len() >= POLICY_PROOF_PEERS {
            return Err(RejectionCode::ServerBusy);
        }
        self.states.insert(peer, PeerState::new(epoch, rates));
        Ok(())
    }

    /// Advances the injected failure-response clock, saturating rather than
    /// wrapping. This is the only time source for the per-peer failure
    /// budgets; never `World` tick, never wall clock.
    pub fn advance_fail_time(&mut self, dt_ms: u64) {
        self.fail_now_ms = self.fail_now_ms.saturating_add(dt_ms);
    }

    /// Sets one disclosure pair (test choreography for grant/revoke flows).
    pub fn set_visibility(&mut self, peer: PeerId, entity: EntityId, visibility: Visibility) {
        self.vis.set(peer, entity, visibility);
    }

    /// Maps every currently visible live entity for the peer, in ascending
    /// entity order. Hidden entities consume no ids: a client must never
    /// infer them from a gap.
    pub fn assign(&mut self, peer: PeerId) {
        let entities: Vec<EntityId> = self.world.ids().collect();
        for entity in entities {
            if self.vis.get(peer, entity).present && self.world.contains(entity) {
                self.ensure_mapped(peer, entity);
            }
        }
    }

    /// Drops mappings for entities the peer may no longer see, returning how
    /// many actionable mappings were revoked. Dropped ids retire to graves
    /// (for oracle history) and are never reused.
    pub fn revoke(&mut self, peer: PeerId) -> usize {
        let hidden: Vec<NetId> = self
            .map
            .get(&peer)
            .map(|map| {
                map.iter()
                    .filter(|(_, entity)| !self.vis.get(peer, **entity).present)
                    .map(|(net, _)| *net)
                    .collect()
            })
            .unwrap_or_default();
        let count = hidden.len();
        for net in hidden {
            self.unmap(peer, net);
        }
        count
    }

    /// Resolves a replica id to its live actionable entity, if any.
    pub fn entity_of(&self, peer: PeerId, net: NetId) -> Option<EntityId> {
        self.map.get(&peer)?.get(&net).copied()
    }

    /// Resolves a live entity to its replica id for the peer, if mapped.
    pub fn net_of(&self, peer: PeerId, entity: EntityId) -> Option<NetId> {
        self.map
            .get(&peer)?
            .iter()
            .find(|(_, e)| **e == entity)
            .map(|(n, _)| *n)
    }

    /// Next new per-lane seq for the peer, if bound.
    pub fn next_seq(&self, peer: PeerId) -> Option<u64> {
        self.states.get(&peer).map(|state| state.next_new_seq)
    }

    /// Test-only choreography: jumps a bound peer's next new seq (for the
    /// exhaustion boundary, which no real command stream can reach by
    /// counting). Production numbering still starts at 1 and never wraps.
    pub fn test_set_next_seq(&mut self, peer: PeerId, seq: u64) {
        if let Some(state) = self.states.get_mut(&peer) {
            state.next_new_seq = seq;
        }
    }

    /// Accepted-command count: cached retries must never move it.
    pub fn executions(&self) -> u64 {
        self.executions
    }

    /// Typed sim error behind the last `IllegalAction` receipt, if any.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Complete authoritative hash: `state_hash` over the world, queue bytes
    /// included, exclusions none. Rejected inputs must leave it identical.
    pub fn authoritative_hash(&self) -> [u8; 32] {
        state_hash(&self.world)
    }

    /// Admits one intent frame, returning receipt bytes.
    ///
    /// Pipeline: decode → epoch → seq/cache → freshness → mapping/visibility/
    /// ownership → immediate `perform_action` legality → receipt. Codec,
    /// unknown-peer, epoch, gap, stale, and exhausted failures consume no
    /// seq; terminal outcomes (applied or rejected) consume exactly one seq
    /// and cache their canonical bytes with the outcome. Frames that never
    /// decode have no command identity to echo, so they yield no reply bytes
    /// at all (the outcome stays observable through
    /// [`admit_status`](Self::admit_status)). Rejected outcomes additionally
    /// spend the peer's failure-response budgets (one frame plus the receipt
    /// byte length, retries and cached rejections included): when either
    /// budget is empty the response is dropped on the wire — this returns
    /// empty while [`admit_status`](Self::admit_status) still reports the
    /// retained terminal outcome — and the sender retries under the sequence
    /// policy after the injected failure clock advances. Applied outcomes
    /// never touch the failure budgets; normal deltas share only the egress
    /// queue caps enforced at fabric staging.
    pub fn admit(&mut self, peer: PeerId, bytes: &[u8]) -> Vec<u8> {
        let status = self.admit_status(peer, bytes);
        let frame = match decode_intent(bytes) {
            Err(_) => return Vec::new(),
            Ok(frame) => frame,
        };
        if matches!(status, ReceiptStatus::Rejected(_)) {
            let estimate = self.encode_receipt(peer, frame.epoch, frame.lane, frame.seq, &status);
            let state = match self.states.get_mut(&peer) {
                Some(state) => state,
                None => return Vec::new(),
            };
            if !state.try_consume_failure(self.fail_now_ms, estimate.len() as u64) {
                return Vec::new();
            }
        }
        self.receipt_bytes(peer, frame.epoch, frame.lane, frame.seq, &status)
    }

    /// Admits one intent frame, returning its terminal outcome for direct
    /// assertion. [`admit`](Self::admit) wraps this with receipt encoding.
    pub fn admit_status(&mut self, peer: PeerId, bytes: &[u8]) -> ReceiptStatus {
        let frame = match decode_intent(bytes) {
            Err(error) => return ReceiptStatus::Rejected(map_codec(error)),
            Ok(frame) => frame,
        };
        if !self.states.contains_key(&peer) {
            return ReceiptStatus::Rejected(RejectionCode::Unauthenticated);
        }
        let epoch_live = match self.states.get(&peer) {
            Some(state) => !self.closed.contains(&frame.epoch) && state.epoch == frame.epoch,
            None => false,
        };
        if !epoch_live {
            return ReceiptStatus::Rejected(RejectionCode::SessionExpired);
        }
        let decision = {
            let state = self.states.get(&peer).expect("peer is bound");
            match state.cache.iter().find(|(seq, _, _)| *seq == frame.seq) {
                Some((_, canonical, status)) if canonical.as_slice() == bytes => {
                    SeqDecision::Cached(*status)
                }
                Some(_) => SeqDecision::Conflict,
                None if state.is_exhausted() => SeqDecision::Exhausted,
                None if frame.seq > state.next_new_seq => SeqDecision::Gap,
                None if frame.seq < state.next_new_seq => SeqDecision::Stale,
                None => SeqDecision::New,
            }
        };
        match decision {
            SeqDecision::Cached(status) => status,
            SeqDecision::Conflict => {
                self.closed.insert(frame.epoch);
                ReceiptStatus::Rejected(RejectionCode::SeqConflict)
            }
            SeqDecision::Exhausted => ReceiptStatus::Rejected(RejectionCode::SeqExhausted),
            SeqDecision::Gap => ReceiptStatus::Rejected(RejectionCode::SeqGap),
            SeqDecision::Stale => ReceiptStatus::Rejected(RejectionCode::StaleSeq),
            SeqDecision::New => self.admit_new(peer, &frame, bytes),
        }
    }

    /// Validates a new-seq frame past freshness, mapping, ownership, and
    /// immediate gameplay legality, finalizing exactly once.
    fn admit_new(
        &mut self,
        peer: PeerId,
        frame: &crpg_net::protocol::IntentFrame,
        bytes: &[u8],
    ) -> ReceiptStatus {
        if frame.observed_tick > self.server_tick {
            return self.finalize(
                peer,
                frame,
                bytes,
                ReceiptStatus::Rejected(RejectionCode::FutureTick),
            );
        }
        if frame.observed_tick < self.server_tick.saturating_sub(POLICY_OBSERVED_TICK_WINDOW) {
            return self.finalize(
                peer,
                frame,
                bytes,
                ReceiptStatus::Rejected(RejectionCode::StaleTick),
            );
        }
        let actor = match self.entity_of(peer, frame.actor) {
            Some(entity) => entity,
            None => {
                return self.finalize(
                    peer,
                    frame,
                    bytes,
                    ReceiptStatus::Rejected(RejectionCode::NotAuthorized),
                );
            }
        };
        if !self.own.can(peer, actor) {
            return self.finalize(
                peer,
                frame,
                bytes,
                ReceiptStatus::Rejected(RejectionCode::NotAuthorized),
            );
        }
        let action = match frame.body {
            IntentBody::DeclareAction { ability, target } => {
                let target = match self.entity_of(peer, target) {
                    Some(entity) => entity,
                    None => {
                        return self.finalize(
                            peer,
                            frame,
                            bytes,
                            ReceiptStatus::Rejected(RejectionCode::NotAuthorized),
                        );
                    }
                };
                if !self.vis.get(peer, target).present {
                    return self.finalize(
                        peer,
                        frame,
                        bytes,
                        ReceiptStatus::Rejected(RejectionCode::NotAuthorized),
                    );
                }
                CombatAction::UseAbility {
                    actor,
                    ability,
                    target,
                }
            }
            IntentBody::EndTurn => CombatAction::EndTurn { actor },
        };
        // `perform_action` precedence stays authoritative: every gameplay
        // rejection maps to the public code with the typed error retained.
        match perform_action(&mut self.world, &action) {
            Ok(_) => {
                self.executions += 1;
                self.finalize(peer, frame, bytes, ReceiptStatus::Applied)
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                self.finalize(
                    peer,
                    frame,
                    bytes,
                    ReceiptStatus::Rejected(RejectionCode::IllegalAction),
                )
            }
        }
    }

    /// Caches canonical bytes with their terminal outcome and advances the
    /// lane, enforcing the 256-entry / 2-MiB retention bound by evicting the
    /// oldest first.
    fn finalize(
        &mut self,
        peer: PeerId,
        frame: &crpg_net::protocol::IntentFrame,
        bytes: &[u8],
        status: ReceiptStatus,
    ) -> ReceiptStatus {
        let state = self.states.get_mut(&peer).expect("peer is bound");
        state.cache.push((frame.seq, bytes.to_vec(), status));
        while state.cache.len() > POLICY_RETRY_CACHE_PER_PEER_LANE {
            state.cache.remove(0);
        }
        while cache_bytes(&state.cache) > POLICY_RETRY_CACHE_BYTES_PER_PEER_LANE {
            state.cache.remove(0);
        }
        state.next_new_seq = frame.seq.saturating_add(1);
        status
    }

    /// Encodes one downstream frame with the peer's next gapless `event_seq`.
    fn frame_bytes(&mut self, peer: PeerId, ops: Vec<DeltaOp>) -> Vec<u8> {
        let seq = self.delivery.get(&peer).copied().unwrap_or(0);
        self.delivery.insert(peer, seq.saturating_add(1));
        encode_delta(&DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: self.server_tick,
            event_seq: seq,
            ops,
        })
        .expect("driver frames encode")
    }

    /// Encodes a terminal receipt for one intent.
    fn receipt_bytes(
        &mut self,
        peer: PeerId,
        epoch: [u8; 16],
        lane: u8,
        seq: u64,
        status: &ReceiptStatus,
    ) -> Vec<u8> {
        self.frame_bytes(
            peer,
            vec![DeltaOp::Receipt {
                epoch,
                lane,
                seq,
                processed_tick: self.server_tick,
                status: *status,
            }],
        )
    }

    /// Pure receipt-size estimate for the failure-budget check: encodes the
    /// receipt the wire path would send at the peer's current delivery `seq`
    /// without advancing delivery or logging to the sent log, so a dropped
    /// response consumes budgets but never delivery numbering.
    fn encode_receipt(
        &self,
        peer: PeerId,
        epoch: [u8; 16],
        lane: u8,
        seq: u64,
        status: &ReceiptStatus,
    ) -> Vec<u8> {
        let event_seq = self.delivery.get(&peer).copied().unwrap_or(0);
        encode_delta(&DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: self.server_tick,
            event_seq,
            ops: vec![DeltaOp::Receipt {
                epoch,
                lane,
                seq,
                processed_tick: self.server_tick,
                status: *status,
            }],
        })
        .expect("driver receipt estimate encodes")
    }

    /// Whether the peer may currently learn presence of the entity.
    fn disclosed(&self, peer: PeerId, entity: EntityId) -> bool {
        self.vis.get(peer, entity).present
    }

    /// Maps a live entity for the peer, issuing the next never-reused id.
    /// Callers check disclosure first: hidden entities consume no ids.
    fn ensure_mapped(&mut self, peer: PeerId, entity: EntityId) -> NetId {
        if let Some(net) = self.net_of(peer, entity) {
            return net;
        }
        let raw = self.next_net.get(&peer).copied().unwrap_or(1).max(1);
        let net = NetId::new(raw).expect("replica ids start at 1 and never wrap here");
        self.map.entry(peer).or_default().insert(net, entity);
        self.next_net.insert(peer, raw.saturating_add(1).max(1));
        net
    }

    /// Drops one actionable mapping, retiring the id to graves for oracle
    /// history. Retired ids are never reused within the epoch.
    fn unmap(&mut self, peer: PeerId, net: NetId) {
        if let Some(entity) = self.map.get_mut(&peer).and_then(|map| map.remove(&net)) {
            self.graves.entry(peer).or_default().insert(entity, net);
        }
        if let Some(known) = self.known.get_mut(&peer) {
            known.remove(&net);
        }
    }

    /// Emits a health op when the permitted fact changed since last send.
    fn sync_health(&mut self, peer: PeerId, net: NetId, ops: &mut Vec<DeltaOp>) {
        let fact = self
            .world
            .combatants()
            .get(self.map[&peer][&net])
            .map(|state| (state.health(), state.max_health(), state.dead()));
        let last = self.last_health.entry(peer).or_default();
        match (fact, last.get(&net).copied()) {
            (Some(current), Some(previous)) if current == previous => {}
            (Some(current), _) => {
                last.insert(net, current);
                if self.vis.get(peer, self.map[&peer][&net]).health {
                    ops.push(DeltaOp::Health {
                        entity: net,
                        health: current.0,
                        max_health: current.1,
                        dead: current.2,
                    });
                }
            }
            (None, _) => {}
        }
    }

    /// Computes the disclosable turn fact: terminal `None` shows for any
    /// peer that knows an entity; a live active actor shows only to peers
    /// that see it (no disclosure is better than false disclosure).
    fn turn_fact(&self, peer: PeerId) -> Option<TurnFact> {
        let combat = self.world.combat()?;
        let knows_any = self.known.get(&peer).is_some_and(|known| !known.is_empty());
        if !knows_any {
            return None;
        }
        match combat.active {
            None => Some(TurnFact {
                active: None,
                round: u64::from(combat.round),
            }),
            Some(active) => {
                let net = self.net_of(peer, active)?;
                if !self.disclosed(peer, active) {
                    return None;
                }
                Some(TurnFact {
                    active: Some(net),
                    round: u64::from(combat.round),
                })
            }
        }
    }

    /// Pumps every due delta op for the peer into one frame, or [`None`]
    /// when nothing is due (no `event_seq` consumed then).
    ///
    /// Order per pump: visibility-leave sweep, new envelopes since the
    /// cursor (cursor always advances, disclosed or not), then state sync
    /// (entity-enter for newly visible, health/turn diffs). Undisclosed
    /// fields are omitted, never zero-filled; hidden spawns consume no ids
    /// and no sequences.
    pub fn deltas(&mut self, peer: PeerId) -> Option<Vec<u8>> {
        if !self.states.contains_key(&peer) {
            return None;
        }
        // Detached clone: the authoritative queue is never drained here.
        let ordered: Vec<EventEnvelope<SimEvent>> = {
            let mut view = self.world.events().clone();
            view.drain()
        };
        let mut ops: Vec<DeltaOp> = Vec::new();
        // Newly hidden known entities leave before events are read, so a
        // disclosable despawn later in this same pump still reads as a
        // despawn rather than a leave.
        let swept: Vec<NetId> = self
            .known
            .get(&peer)
            .map(|known| known.iter().copied().collect())
            .unwrap_or_default();
        for net in swept {
            let visible = self
                .map
                .get(&peer)
                .and_then(|map| map.get(&net))
                .is_some_and(|entity| self.disclosed(peer, *entity));
            if !visible {
                ops.push(DeltaOp::EntityLeave { entity: net });
                self.unmap(peer, net);
                self.last_health.entry(peer).or_default().remove(&net);
            }
        }
        for envelope in &ordered {
            if let Some((tick, seq)) = self.cursor.get(&peer) {
                if (envelope.tick, envelope.seq) <= (*tick, *seq) {
                    continue;
                }
            }
            match envelope.payload {
                SimEvent::Spawned { entity } => {
                    if self.disclosed(peer, entity) {
                        let net = self.ensure_mapped(peer, entity);
                        self.known.entry(peer).or_default().insert(net);
                        ops.push(DeltaOp::EntityEnter { entity: net });
                        self.sync_health(peer, net, &mut ops);
                        ops.push(DeltaOp::Spawned { entity: net });
                    }
                }
                SimEvent::Despawned { entity } => {
                    let net = self.grave_or_mapped(peer, entity);
                    if let Some(net) = net {
                        if self
                            .known
                            .get(&peer)
                            .is_some_and(|known| known.contains(&net))
                        {
                            if self.disclosed(peer, entity) {
                                ops.push(DeltaOp::Despawned { entity: net });
                            } else {
                                ops.push(DeltaOp::EntityLeave { entity: net });
                            }
                            self.unmap(peer, net);
                            self.last_health.entry(peer).or_default().remove(&net);
                        }
                    }
                }
                SimEvent::Died { entity } => {
                    if let Some(net) = self.net_of(peer, entity) {
                        if self
                            .known
                            .get(&peer)
                            .is_some_and(|known| known.contains(&net))
                            && self.disclosed(peer, entity)
                        {
                            ops.push(DeltaOp::Died { entity: net });
                        }
                    }
                }
            }
            self.cursor.insert(peer, (envelope.tick, envelope.seq));
        }
        // Newly visible mapped entities enter with their permitted state.
        let mapped: Vec<(NetId, EntityId)> = self
            .map
            .get(&peer)
            .map(|map| map.iter().map(|(net, entity)| (*net, *entity)).collect())
            .unwrap_or_default();
        for (net, entity) in mapped {
            if !self.world.contains(entity) || !self.disclosed(peer, entity) {
                continue;
            }
            let known = self.known.entry(peer).or_default();
            if !known.contains(&net) {
                known.insert(net);
                ops.push(DeltaOp::EntityEnter { entity: net });
            }
            self.sync_health(peer, net, &mut ops);
        }
        let fact = self.turn_fact(peer);
        if self.last_turn.get(&peer).copied().flatten() != fact {
            self.last_turn.insert(peer, fact);
            if let Some(fact) = fact {
                ops.push(DeltaOp::Turn {
                    active: fact.active,
                    round: fact.round,
                });
            }
        }
        if ops.is_empty() {
            return None;
        }
        let seq = self.delivery.get(&peer).copied().unwrap_or(0);
        self.delivery.insert(peer, seq.saturating_add(1));
        self.sent
            .entry(peer)
            .or_default()
            .insert(seq, (self.server_tick, ops.clone()));
        Some(
            encode_delta(&DeltaFrame {
                lane: LANE_COMBAT,
                server_tick: self.server_tick,
                event_seq: seq,
                ops,
            })
            .expect("driver frames encode"),
        )
    }

    /// Re-encodes a previously sent frame byte-identically for retry after
    /// loss, or [`None`] when that sequence was never sent to the peer.
    pub fn redeliver(&self, peer: PeerId, event_seq: u64) -> Option<Vec<u8>> {
        let (server_tick, ops) = self.sent.get(&peer)?.get(&event_seq)?;
        Some(
            encode_delta(&DeltaFrame {
                lane: LANE_COMBAT,
                server_tick: *server_tick,
                event_seq,
                ops: ops.clone(),
            })
            .expect("stored frames re-encode"),
        )
    }

    /// Resolves a possibly-retired entity to its replica id for oracle
    /// history (live mapping first, then graves).
    fn grave_or_mapped(&self, peer: PeerId, entity: EntityId) -> Option<NetId> {
        self.net_of(peer, entity)
            .or_else(|| self.graves.get(&peer)?.get(&entity).copied())
    }

    /// Independently projects the peer's permitted view straight from the
    /// world plus the tables: presence, health, and turn facts plus every
    /// disclosed event from genesis with per-client sequence in envelope
    /// order. Reads no delivery state (`known`, cursors, sent logs).
    pub fn project(&self, peer: PeerId) -> ExpectedView {
        let mut present = Vec::new();
        let mut health = Vec::new();
        if let Some(map) = self.map.get(&peer) {
            let mut nets: Vec<NetId> = map
                .iter()
                .filter(|(_, entity)| {
                    self.world.contains(**entity) && self.disclosed(peer, **entity)
                })
                .map(|(net, _)| *net)
                .collect();
            nets.sort();
            for net in nets {
                present.push(net);
                let entity = map[&net];
                if self.vis.get(peer, entity).health {
                    if let Some(state) = self.world.combatants().get(entity) {
                        health.push(HealthFact {
                            entity: net,
                            health: state.health(),
                            max_health: state.max_health(),
                            dead: state.dead(),
                        });
                    }
                }
            }
        }
        let turn = self.turn_fact_independent(peer);
        let mut view = self.world.events().clone();
        let mut events = Vec::new();
        for envelope in view.drain() {
            let event = match envelope.payload {
                SimEvent::Spawned { entity } => self
                    .resolve_historical(peer, entity)
                    .filter(|_| self.disclosed(peer, entity))
                    .map(|net| FilteredEvent::Spawned { entity: net }),
                SimEvent::Despawned { entity } => {
                    // A despawn discloses only when the removal itself was
                    // disclosable: the entity must have been visible, and the
                    // peer must not have been revoked before removal. The
                    // current tables are the test's disclosure context.
                    self.resolve_historical(peer, entity)
                        .filter(|_| self.disclosed(peer, entity))
                        .map(|net| FilteredEvent::Despawned { entity: net })
                }
                SimEvent::Died { entity } => self
                    .resolve_historical(peer, entity)
                    .filter(|_| self.disclosed(peer, entity))
                    .map(|net| FilteredEvent::Died { entity: net }),
            };
            if let Some(event) = event {
                let seq = events.len() as u64;
                events.push((seq, event));
            }
        }
        ExpectedView {
            present,
            health,
            turn,
            events,
        }
    }

    /// Turn disclosure for the oracle path (same rule as delivery, computed
    /// without delivery state: any mapped-visible entity counts).
    fn turn_fact_independent(&self, peer: PeerId) -> Option<TurnFact> {
        let combat = self.world.combat()?;
        let knows_any = self.map.get(&peer).is_some_and(|map| {
            map.values()
                .any(|entity| self.world.contains(*entity) && self.disclosed(peer, *entity))
        });
        if !knows_any {
            return None;
        }
        match combat.active {
            None => Some(TurnFact {
                active: None,
                round: u64::from(combat.round),
            }),
            Some(active) => {
                let net = self.resolve_historical(peer, active)?;
                if !self.disclosed(peer, active) {
                    return None;
                }
                Some(TurnFact {
                    active: Some(net),
                    round: u64::from(combat.round),
                })
            }
        }
    }

    /// Resolves any entity the peer ever mapped (live mapping, then graves)
    /// for oracle history.
    fn resolve_historical(&self, peer: PeerId, entity: EntityId) -> Option<NetId> {
        self.grave_or_mapped(peer, entity)
    }
}

fn map_codec(error: CodecError) -> RejectionCode {
    match error {
        CodecError::FrameTooLarge => RejectionCode::FrameTooLarge,
        CodecError::UnsupportedVersion => RejectionCode::UnsupportedVersion,
        CodecError::WrongDirection => RejectionCode::WrongDirection,
        CodecError::UnknownMessage => RejectionCode::UnknownMessage,
        CodecError::Malformed => RejectionCode::Malformed,
        CodecError::LimitExceeded => RejectionCode::LimitExceeded,
    }
}

fn cache_bytes(cache: &[(u64, Vec<u8>, ReceiptStatus)]) -> usize {
    cache.iter().map(|(_, bytes, _)| bytes.len()).sum()
}
