//! T022 acceptance suite (`tasks/T022.md` §9): public API only.
//!
//! Oracles are independent of the host's own projection:
//!
//! - **Authority oracle**: a test-owned mirror `HistoryWorld` loaded from the
//!   same fixture, driven through public `perform_action` with every action
//!   the test expects the host to accept and acknowledged on the same
//!   schedule; `history_hash` of the mirror must equal
//!   `Host::authority_hash`, and each capture's retained events must equal
//!   the mirror's own `read_after` page for the same range.
//! - **Permitted-state oracle**: per peer, built from the mirror's public
//!   `world()` reads plus the hand-assigned replica ids, compared with a
//!   replica assembled from decoded delivery frames.
//! - **Ordered op oracle**: hand-written expected op lists per schedule and
//!   version (drop/duplicate/swap-sensitive), and hand-encoded wire bytes.
//!
//! The fixture (`fixtures/three_combatants.json`) is a `HistoryWorld` after
//! `start_encounter` of a three-participant encounter with its four start
//! envelopes still pending: A (index 0, health 10) acts first, then B
//! (index 1, health 10), then C (index 2, health 3). Abilities: strike
//! (always `success`, 3 damage), smite (always `critical_success`,
//! 4 000 000 000 damage: overkill), whiff (always `failure`, 0 damage); each
//! costs the single one-point pool and ends the turn.

use std::collections::{BTreeMap, BTreeSet};

use crpg_core::{EntityId, Ulid};
use crpg_net::codec;
use crpg_net::codec_v2;
use crpg_net::protocol::{
    self, IntentBody, IntentFrame, NetId, ReceiptStatus, RejectionCode, LANE_COMBAT,
    MAX_INTENT_FRAME_BYTES, SEQ_LAST,
};
use crpg_net::protocol_v2::{self, DeltaOp};
use crpg_server::capture::{
    CapturedHealth, CapturedOutcome, CapturedRecord, CapturedTurn, CapturedView, MAX_CAPTURE_PAGE,
    MAX_CAPTURE_RECORDS, MAX_CAPTURE_RECORD_BYTES, MAX_DELIVERY_BYTES_PER_COMMAND_PEER,
    MAX_DELIVERY_OPS_PER_COMMAND, MAX_NEW_EVENTS_PER_COMMAND,
};
use crpg_server::checkpoint::{
    load_checkpoint, load_checkpoint_from_reader, CheckpointError, CHECKPOINT_VERSION,
    MAX_CHECKPOINT_BYTES,
};
use crpg_server::host::{
    AdmissionResult, ControlGrant, DisclosureGrants, EntityDisclosure, GrantUpdate, Host,
    HostConfig, HostError, IngestDisposition, PeerHandle, ProtocolSelection, PumpSummary,
    MAX_PEERS,
};
use crpg_sim::{
    history_hash, CombatAction, CombatError, HistoryEnvelope, HistoryEvent, HistoryWorld,
};
use serde_json::json;

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");

const V1: ProtocolSelection = ProtocolSelection::V1;
const V2: ProtocolSelection = ProtocolSelection::V2;

/// Runs one named case body under both selected versions, as
/// `<case>::v1` and `<case>::v2` (modules and functions share a name in
/// separate namespaces).
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
// Fixture identities.
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
fn smite() -> Ulid {
    Ulid::from_u128(604)
}
fn whiff() -> Ulid {
    Ulid::from_u128(605)
}
fn encounter() -> Ulid {
    Ulid::from_u128(601)
}
fn unknown_ability() -> Ulid {
    Ulid::from_u128(699)
}

fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}

/// The fixture authority with its start envelopes still pending.
fn fixture_unacked() -> HistoryWorld {
    serde_json::from_str(FIXTURE).expect("fixture loads through validated serde")
}

/// The fixture authority after the embedding captured and acknowledged the
/// start envelopes (pre-wrap choreography).
fn fixture() -> HistoryWorld {
    let mut history = fixture_unacked();
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
        abilities: vec![strike(), smite(), whiff()],
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

fn control(actors: &[EntityId]) -> ControlGrant {
    ControlGrant {
        actors: actors.to_vec(),
    }
}

fn all_actors() -> ControlGrant {
    control(&[a(), b(), c()])
}

// ---------------------------------------------------------------------------
// Hand-encoded wire (independent of the codec).
// ---------------------------------------------------------------------------

fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn text(out: &mut Vec<u8>, value: &str) {
    varint(out, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
}

fn version_byte(protocol: ProtocolSelection) -> u8 {
    match protocol {
        ProtocolSelection::V1 => 1,
        ProtocolSelection::V2 => 2,
    }
}

fn intent_header(version: u8, epoch: [u8; 16], seq: u64, observed: u64, actor: u64) -> Vec<u8> {
    let mut out = vec![version];
    out.extend_from_slice(&epoch);
    out.push(LANE_COMBAT);
    varint(&mut out, seq);
    varint(&mut out, observed);
    varint(&mut out, actor);
    out
}

/// Hand-encoded `DeclareAction` intent bytes.
fn declare(
    version: u8,
    epoch: [u8; 16],
    seq: u64,
    observed: u64,
    actor: u64,
    ability: Ulid,
    target: u64,
) -> Vec<u8> {
    let mut out = intent_header(version, epoch, seq, observed, actor);
    out.push(0);
    text(&mut out, &ability.to_string());
    varint(&mut out, target);
    out
}

/// Hand-encoded `EndTurn` intent bytes.
fn end_turn(version: u8, epoch: [u8; 16], seq: u64, observed: u64, actor: u64) -> Vec<u8> {
    let mut out = intent_header(version, epoch, seq, observed, actor);
    out.push(1);
    out
}

fn epoch_of(incarnation: u64, handle: u64) -> [u8; 16] {
    let mut epoch = [0u8; 16];
    epoch[..8].copy_from_slice(&incarnation.to_le_bytes());
    epoch[8..].copy_from_slice(&handle.to_le_bytes());
    epoch
}

// ---------------------------------------------------------------------------
// Expected-op constructors (the ordered op oracle's vocabulary).
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
fn receipt(epoch: [u8; 16], seq: u64, tick: u64, status: ReceiptStatus) -> DeltaOp {
    legacy(protocol::DeltaOp::Receipt {
        epoch,
        lane: LANE_COMBAT,
        seq,
        processed_tick: tick,
        status,
    })
}
fn resolved(actor: u64, target: u64, ability: Ulid, outcome: &str, damage: u32) -> DeltaOp {
    DeltaOp::ActionResolved {
        actor: n(actor),
        target: n(target),
        ability,
        outcome: outcome.to_owned(),
        damage,
    }
}
fn started(actor: u64, round: u64) -> DeltaOp {
    DeltaOp::TurnStarted {
        actor: n(actor),
        round,
    }
}
fn ended(round: u64) -> DeltaOp {
    DeltaOp::EncounterEnded {
        encounter: encounter(),
        round,
    }
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

const APPLIED: ReceiptStatus = ReceiptStatus::Applied;

fn rejected(code: RejectionCode) -> ReceiptStatus {
    ReceiptStatus::Rejected(code)
}

// ---------------------------------------------------------------------------
// Decoded delivery and the replica.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    event_seq: u64,
    server_tick: u64,
    ops: Vec<DeltaOp>,
    bytes: Vec<u8>,
}

fn decode_frame(protocol: ProtocolSelection, bytes: &[u8]) -> Frame {
    match protocol {
        ProtocolSelection::V1 => {
            let frame = codec::decode_delta(bytes).expect("v1 delivery decodes");
            Frame {
                event_seq: frame.event_seq,
                server_tick: frame.server_tick,
                ops: frame.ops.into_iter().map(DeltaOp::Legacy).collect(),
                bytes: bytes.to_vec(),
            }
        }
        ProtocolSelection::V2 => {
            let frame = codec_v2::decode_delta(bytes).expect("v2 delivery decodes");
            Frame {
                event_seq: frame.event_seq,
                server_tick: frame.server_tick,
                ops: frame.ops,
                bytes: bytes.to_vec(),
            }
        }
    }
}

/// Permitted replica state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct State {
    entered: BTreeSet<u64>,
    health: BTreeMap<u64, (u32, u32, bool)>,
    turn: Option<(Option<u64>, u64)>,
}

/// A client replica assembled only from delivery bytes.
#[derive(Debug, Default)]
struct Replica {
    next: u64,
    state: State,
    events: Vec<DeltaOp>,
    receipts: Vec<DeltaOp>,
}

impl Replica {
    fn new() -> Self {
        Self {
            next: 1,
            ..Self::default()
        }
    }

    /// Applies new frames in order; duplicates (already seen `event_seq`)
    /// are skipped, gaps fail.
    fn apply(&mut self, frames: &[Frame]) {
        for frame in frames {
            if frame.event_seq < self.next {
                continue;
            }
            assert_eq!(frame.event_seq, self.next, "delivery is gapless");
            self.next += 1;
            for op in &frame.ops {
                match op {
                    DeltaOp::Legacy(protocol::DeltaOp::EntityEnter { entity }) => {
                        self.state.entered.insert(entity.get());
                    }
                    DeltaOp::Legacy(protocol::DeltaOp::EntityLeave { entity }) => {
                        self.state.entered.remove(&entity.get());
                        self.state.health.remove(&entity.get());
                    }
                    DeltaOp::Legacy(protocol::DeltaOp::Health {
                        entity,
                        health,
                        max_health,
                        dead,
                    }) => {
                        self.state
                            .health
                            .insert(entity.get(), (*health, *max_health, *dead));
                    }
                    DeltaOp::Legacy(protocol::DeltaOp::Turn { active, round }) => {
                        self.state.turn = Some((active.map(NetId::get), *round));
                    }
                    DeltaOp::Legacy(protocol::DeltaOp::Receipt { .. }) => {
                        self.receipts.push(op.clone());
                    }
                    other => self.events.push(other.clone()),
                }
            }
        }
    }
}

/// The independent permitted-state oracle: mirror reads plus hand-assigned
/// replica ids.
fn expected_state(
    oracle: &HistoryWorld,
    grants: &DisclosureGrants,
    ids: &BTreeMap<EntityId, u64>,
) -> State {
    let world = oracle.world();
    let mut state = State::default();
    let present: BTreeSet<EntityId> = grants
        .entities
        .iter()
        .filter(|entry| entry.present)
        .map(|entry| entry.entity)
        .collect();
    for entry in &grants.entities {
        if !entry.present {
            continue;
        }
        let id = ids[&entry.entity];
        state.entered.insert(id);
        if entry.health {
            if let Some(combatant) = world.combatants().get(entry.entity) {
                state.health.insert(
                    id,
                    (combatant.health(), combatant.max_health(), combatant.dead()),
                );
            }
        }
    }
    if grants.turn_state {
        if let Some(combat) = world.combat() {
            let active = combat
                .active
                .filter(|entity| present.contains(entity))
                .map(|entity| ids[&entity]);
            state.turn = Some((active, u64::from(combat.round)));
        }
    }
    state
}

fn standard_ids() -> BTreeMap<EntityId, u64> {
    BTreeMap::from([(a(), 1), (b(), 2), (c(), 3)])
}

// ---------------------------------------------------------------------------
// Harness.
// ---------------------------------------------------------------------------

/// One host plus its independent mirror authority.
struct H {
    host: Host,
    protocol: ProtocolSelection,
    version: u8,
    incarnation: u64,
    now: u64,
    oracle: HistoryWorld,
}

impl H {
    fn new(protocol: ProtocolSelection) -> Self {
        Self::with_incarnation(protocol, 1)
    }

    fn with_incarnation(protocol: ProtocolSelection, incarnation: u64) -> Self {
        let host = Host::from_history(
            fixture(),
            HostConfig {
                protocol,
                incarnation,
            },
            0,
        )
        .expect("wraps an acknowledged authority");
        Self {
            host,
            protocol,
            version: version_byte(protocol),
            incarnation,
            now: 0,
            oracle: fixture(),
        }
    }

    fn bind(&mut self, control: ControlGrant, grants: DisclosureGrants) -> PeerHandle {
        self.host
            .bind_peer(control, grants, self.now)
            .expect("binds")
    }

    fn epoch(&self, peer: PeerHandle) -> [u8; 16] {
        self.host.epoch(peer).expect("bound")
    }

    fn tick_now(&self) -> u64 {
        self.host.server_tick()
    }

    /// Advances injected time by 100 ms: inside every sustained rate budget.
    fn advance(&mut self) {
        self.now += 100;
    }

    fn ingest(&mut self, peer: PeerHandle, bytes: &[u8]) {
        self.advance();
        assert_eq!(
            self.host.ingest(peer, bytes, self.now),
            Ok(IngestDisposition::Staged)
        );
    }

    fn pump(&mut self) -> PumpSummary {
        self.host.pump(self.now).expect("pump")
    }

    /// Ingests and pumps exactly one command.
    fn send(&mut self, peer: PeerHandle, bytes: &[u8]) -> AdmissionResult {
        self.ingest(peer, bytes);
        let summary = self.pump();
        assert_eq!(summary.results.len(), 1, "{summary:?}");
        summary.results[0].clone()
    }

    fn declare(
        &self,
        peer: PeerHandle,
        seq: u64,
        actor: u64,
        ability: Ulid,
        target: u64,
    ) -> Vec<u8> {
        declare(
            self.version,
            self.epoch(peer),
            seq,
            self.tick_now(),
            actor,
            ability,
            target,
        )
    }

    fn end_turn(&self, peer: PeerHandle, seq: u64, actor: u64) -> Vec<u8> {
        end_turn(self.version, self.epoch(peer), seq, self.tick_now(), actor)
    }

    /// Mirrors one action the host is expected to accept, acknowledging on
    /// the host's schedule (after capture).
    fn mirror(&mut self, action: CombatAction) -> Vec<HistoryEnvelope> {
        let before = self.oracle.last_sequence();
        self.oracle
            .perform_action(&action)
            .expect("the mirror accepts the same action");
        let page = if self.oracle.last_sequence() > before {
            self.oracle.read_after(before, 256).expect("readable")
        } else {
            Vec::new()
        };
        self.oracle
            .acknowledge(self.oracle.last_sequence())
            .expect("acks");
        page
    }

    fn assert_authority(&self) {
        assert_eq!(self.host.authority_hash(), history_hash(&self.oracle));
    }

    /// Takes, decodes, and acknowledges every pending frame.
    fn drain(&mut self, peer: PeerHandle) -> Vec<Frame> {
        let frames: Vec<Frame> = self
            .host
            .take_delivery(peer)
            .expect("bound")
            .iter()
            .map(|bytes| decode_frame(self.protocol, bytes))
            .collect();
        if let Some(last) = frames.last() {
            self.host
                .acknowledge_delivery(peer, last.event_seq)
                .expect("acks");
        }
        frames
    }

    fn last_record(&self) -> CapturedRecord {
        let last = self.host.last_capture();
        assert!(last > self.host.capture_acknowledged());
        self.host
            .read_captures(last - 1, 1)
            .expect("readable")
            .pop()
            .expect("one record")
    }
}

/// Complete-state-unchanged evidence: authority hash, executions, captures.
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    hash: [u8; 32],
    executions: u64,
    last_capture: u64,
    capture_acknowledged: u64,
}

fn snapshot(host: &Host) -> Snapshot {
    Snapshot {
        hash: host.authority_hash(),
        executions: host.executions(),
        last_capture: host.last_capture(),
        capture_acknowledged: host.capture_acknowledged(),
    }
}

fn result(peer: PeerHandle, seq: Option<u64>, status: ReceiptStatus) -> AdmissionResult {
    AdmissionResult {
        peer,
        seq,
        status,
        cached: false,
        retained_in_ingress: false,
    }
}

fn ops_of(frames: &[Frame]) -> Vec<Vec<DeltaOp>> {
    frames.iter().map(|frame| frame.ops.clone()).collect()
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

// ---------------------------------------------------------------------------
// Cases.
// ---------------------------------------------------------------------------

fn valid_action_applied_through_bytes(protocol: ProtocolSelection) {
    let mut h = H::new(protocol);
    let actor_peer = h.bind(all_actors(), full_grants());
    let watcher = h.bind(control(&[]), observer_grants());
    let epoch = h.epoch(actor_peer);
    assert_eq!(epoch, epoch_of(1, 1));
    assert_eq!(h.epoch(watcher), epoch_of(1, 2));

    // Initial permitted state from delivery seq 1.
    let first = h.drain(actor_peer);
    assert_eq!(ops_of(&first), vec![initial_full_state()]);
    assert_eq!(first[0].event_seq, 1);
    let watcher_first = h.drain(watcher);
    assert_eq!(ops_of(&watcher_first), vec![initial_full_state()]);

    // The intent bytes are hand-encoded; they agree with the codec.
    let bytes = h.declare(actor_peer, 1, 1, strike(), 2);
    let frame = IntentFrame {
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: n(1),
        body: IntentBody::DeclareAction {
            ability: strike(),
            target: n(2),
        },
    };
    let codec_bytes = match protocol {
        ProtocolSelection::V1 => codec::encode_intent(&frame),
        ProtocolSelection::V2 => codec_v2::encode_intent(&frame),
    };
    assert_eq!(codec_bytes.expect("encodes"), bytes);

    h.ingest(actor_peer, &bytes);
    let summary = h.pump();
    assert_eq!(
        summary,
        PumpSummary {
            admitted: 1,
            applied: 1,
            rejected: 0,
            backpressured: 0,
            dropped_replies: 0,
            results: vec![result(actor_peer, Some(1), APPLIED)],
        }
    );
    assert_eq!(h.host.executions(), 1);
    let page = h.mirror(CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    });
    h.assert_authority();

    // Ordered op oracle per version.
    let frames = h.drain(actor_peer);
    let expected = for_version(
        protocol,
        vec![
            receipt(epoch, 1, 0, APPLIED),
            resolved(1, 2, strike(), "success", 3),
            started(2, 0),
            health(1, 10, 10, false),
            health(2, 7, 10, false),
            turn(Some(2), 0),
        ],
    );
    assert_eq!(ops_of(&frames), vec![expected]);
    assert_eq!(frames[0].event_seq, 2);

    // Hand-encoded byte oracle for the whole frame.
    let mut wire = vec![version_byte(protocol), LANE_COMBAT, 0, 2];
    match protocol {
        ProtocolSelection::V1 => wire.push(4),
        ProtocolSelection::V2 => wire.push(6),
    }
    wire.push(7);
    wire.extend_from_slice(&epoch);
    wire.extend([LANE_COMBAT, 1, 0, 0]);
    if protocol == V2 {
        wire.extend([8, 1, 2]);
        text(&mut wire, &strike().to_string());
        text(&mut wire, "success");
        wire.push(3);
        wire.extend([9, 2, 0]);
    }
    wire.extend([5, 1, 10, 10, 0]);
    wire.extend([5, 2, 7, 10, 0]);
    wire.extend([6, 1, 2, 0]);
    assert_eq!(frames[0].bytes, wire);

    // The observer converges on state without any event op.
    let watcher_frames = h.drain(watcher);
    assert_eq!(
        ops_of(&watcher_frames),
        vec![vec![
            health(1, 10, 10, false),
            health(2, 7, 10, false),
            turn(Some(2), 0)
        ]]
    );

    // Replicas equal the independent permitted-state oracle.
    let mut replica = Replica::new();
    replica.apply(&first);
    replica.apply(&frames);
    assert_eq!(
        replica.state,
        expected_state(&h.oracle, &full_grants(), &standard_ids())
    );
    let mut watcher_replica = Replica::new();
    watcher_replica.apply(&watcher_first);
    watcher_replica.apply(&watcher_frames);
    assert_eq!(
        watcher_replica.state,
        expected_state(&h.oracle, &observer_grants(), &standard_ids())
    );
    assert!(watcher_replica.events.is_empty());

    // The capture holds the exact mirrored journal range.
    let record = h.last_record();
    assert_eq!(record.events, page);
    assert_eq!(record.epoch, epoch);
    assert_eq!(record.seq, 1);
}
both_versions!(valid_action_applied_through_bytes);

fn rejected_action_leaves_state_identical(protocol: ProtocolSelection) {
    let mut h = H::new(protocol);
    let peer = h.bind(all_actors(), full_grants());
    let epoch = h.epoch(peer);
    h.drain(peer);
    let mut seq = 1;

    let reject = |h: &mut H, bytes: Vec<u8>, seq: u64, check: fn(&CombatError) -> bool| {
        let before = snapshot(&h.host);
        let outcome = h.send(peer, &bytes);
        assert_eq!(
            outcome,
            result(peer, Some(seq), rejected(RejectionCode::IllegalAction))
        );
        let error = h.host.last_combat_error().expect("typed cause retained");
        assert!(check(&error), "{error:?}");
        assert_eq!(snapshot(&h.host), before, "complete state unchanged");
        h.assert_authority();
        assert_eq!(
            h.host.cached_status(peer, seq),
            Ok(Some(rejected(RejectionCode::IllegalAction)))
        );
        let frames = h.drain(peer);
        assert_eq!(
            ops_of(&frames),
            vec![vec![receipt(
                epoch,
                seq,
                h.tick_now(),
                rejected(RejectionCode::IllegalAction)
            )]]
        );
    };

    // Bad turn: B acts on A's turn.
    let bytes = h.declare(peer, seq, 2, strike(), 1);
    reject(&mut h, bytes, seq, |e| {
        matches!(e, CombatError::OutOfTurn { .. })
    });
    seq += 1;
    // Unknown ability.
    let bytes = h.declare(peer, seq, 1, unknown_ability(), 2);
    reject(&mut h, bytes, seq, |e| {
        matches!(e, CombatError::UnknownAbility { .. })
    });
    seq += 1;
    // Paired positive control: A smites C (overkill kill).
    let bytes = h.declare(peer, seq, 1, smite(), 3);
    assert_eq!(h.send(peer, &bytes), result(peer, Some(seq), APPLIED));
    h.mirror(CombatAction::UseAbility {
        actor: a(),
        ability: smite(),
        target: c(),
    });
    h.assert_authority();
    h.drain(peer);
    seq += 1;
    // Dead actor: C ends a turn.
    let bytes = h.end_turn(peer, seq, 3);
    reject(&mut h, bytes, seq, |e| {
        matches!(e, CombatError::DeadActor { .. })
    });
    seq += 1;
    // Paired positive control: B, the active head, ends its turn.
    let bytes = h.end_turn(peer, seq, 2);
    assert_eq!(h.send(peer, &bytes), result(peer, Some(seq), APPLIED));
    h.mirror(CombatAction::EndTurn { actor: b() });
    h.assert_authority();
    assert_eq!(h.host.executions(), 2);
}
both_versions!(rejected_action_leaves_state_identical);

fn unknown_version_refused_at_ingress(protocol: ProtocolSelection) {
    let mut h = H::new(protocol);
    let peer = h.bind(all_actors(), full_grants());
    h.drain(peer);
    let epoch = h.epoch(peer);
    let other = match protocol {
        ProtocolSelection::V1 => 2,
        ProtocolSelection::V2 => 1,
    };
    let before = snapshot(&h.host);
    for version in [other, 0, 3, 255] {
        let bytes = end_turn(version, epoch, 1, 0, 1);
        assert_eq!(
            h.send(peer, &bytes),
            result(peer, None, rejected(RejectionCode::UnsupportedVersion))
        );
    }
    assert_eq!(snapshot(&h.host), before);
    assert!(h.drain(peer).is_empty(), "undecodable bytes stage no reply");
    // An intent body tag outside the closed vocabulary (including the delta
    // event tags 8/9/10) is UnknownMessage at decode.
    for tag in [2u8, 8, 9, 10] {
        let mut bytes = intent_header(h.version, epoch, 1, 0, 1);
        bytes.push(tag);
        assert_eq!(
            h.send(peer, &bytes),
            result(peer, None, rejected(RejectionCode::UnknownMessage))
        );
    }
    // No sequence was consumed: seq 1 under the selected version applies.
    let bytes = h.end_turn(peer, 1, 1);
    assert_eq!(h.send(peer, &bytes), result(peer, Some(1), APPLIED));
    h.mirror(CombatAction::EndTurn { actor: a() });
    h.assert_authority();

    // Direct decoder proof: a valid v1 delta envelope carrying each v2 event
    // tag is UnknownMessage, not a version refusal; the v2 decoder refuses
    // an unassigned tag the same way.
    for tag in [8u8, 9, 10] {
        let v1_delta = vec![1, LANE_COMBAT, 0, 1, 1, tag, 1, 1, 0];
        assert_eq!(
            codec::decode_delta(&v1_delta),
            Err(codec::CodecError::UnknownMessage)
        );
    }
    let v2_delta = vec![2, LANE_COMBAT, 0, 1, 1, 11, 1];
    assert_eq!(
        codec_v2::decode_delta(&v2_delta),
        Err(codec::CodecError::UnknownMessage)
    );
    // Positive controls: well-formed envelopes of each version decode.
    assert!(codec::decode_delta(&[1, LANE_COMBAT, 0, 1, 1, 4, 1]).is_ok());
    assert!(codec_v2::decode_delta(&[2, LANE_COMBAT, 0, 1, 1, 9, 1, 0]).is_ok());
}
both_versions!(unknown_version_refused_at_ingress);

/// One peer whose grants differ from full by `edit`, observing A strike B.
fn hidden_variant(
    protocol: ProtocolSelection,
    edit: fn(&mut DisclosureGrants),
    expected_v2: Vec<DeltaOp>,
    hidden: &[&[u8]],
) {
    let mut h = H::new(protocol);
    let actor_peer = h.bind(all_actors(), full_grants());
    let mut grants = full_grants();
    edit(&mut grants);
    let restricted = h.bind(control(&[]), grants.clone());
    h.drain(actor_peer);
    let first = h.drain(restricted);
    let bytes = h.declare(actor_peer, 1, 1, strike(), 2);
    assert_eq!(
        h.send(actor_peer, &bytes),
        result(actor_peer, Some(1), APPLIED)
    );
    h.mirror(CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    });
    h.assert_authority();
    let full_frames = h.drain(actor_peer);
    let frames = h.drain(restricted);
    let expected = for_version(protocol, expected_v2);
    if expected.is_empty() {
        assert!(frames.is_empty(), "no spurious empty frame");
    } else {
        assert_eq!(ops_of(&frames), vec![expected]);
    }
    // Two peers diverge: the full peer always sees the complete stream.
    assert_eq!(
        full_frames[0].ops,
        for_version(
            protocol,
            vec![
                receipt(h.epoch(actor_peer), 1, 0, APPLIED),
                resolved(1, 2, strike(), "success", 3),
                started(2, 0),
                health(1, 10, 10, false),
                health(2, 7, 10, false),
                turn(Some(2), 0),
            ]
        )
    );
    // Hidden facts never enter this peer's bytes.
    for frame in first.iter().chain(frames.iter()) {
        for needle in hidden {
            assert!(
                !frame.bytes.windows(needle.len()).any(|w| w == *needle),
                "hidden bytes leaked"
            );
        }
    }
    // The permitted state still converges.
    let ids: BTreeMap<EntityId, u64> = {
        let mut next = 0;
        grants
            .entities
            .iter()
            .filter(|entry| entry.present)
            .map(|entry| {
                next += 1;
                (entry.entity, next)
            })
            .collect()
    };
    let mut replica = Replica::new();
    replica.apply(&first);
    replica.apply(&frames);
    assert_eq!(replica.state, expected_state(&h.oracle, &grants, &ids));
    // The capture view for this peer carries only permitted state.
    let record = h.last_record();
    let epoch = h.epoch(restricted);
    if let Some(view) = record.views.iter().find(|view| view.epoch == epoch) {
        for entry in &view.health {
            assert!(replica.state.health.contains_key(&entry.entity.get()));
        }
    }
}

fn hidden_fields_absent_from_snapshots_and_replies(protocol: ProtocolSelection) {
    let success: &[u8] = b"success";
    // Actor presence removed: A unmapped (B=1, C=2). Whole event gone.
    hidden_variant(
        protocol,
        |g| g.entities[0].present = false,
        vec![started(1, 0), health(1, 7, 10, false), turn(Some(1), 0)],
        &[success],
    );
    // Target presence removed: B unmapped (A=1, C=2); its turn start too.
    hidden_variant(
        protocol,
        |g| g.entities[1].present = false,
        vec![health(1, 10, 10, false), turn(None, 0)],
        &[success],
    );
    // Ability membership removed.
    hidden_variant(
        protocol,
        |g| g.abilities.retain(|id| *id != strike()),
        vec![
            started(2, 0),
            health(1, 10, 10, false),
            health(2, 7, 10, false),
            turn(Some(2), 0),
        ],
        &[success],
    );
    let state_only = || {
        vec![
            started(2, 0),
            health(1, 10, 10, false),
            health(2, 7, 10, false),
            turn(Some(2), 0),
        ]
    };
    // Outcome on either end, damage on either end.
    hidden_variant(
        protocol,
        |g| g.entities[0].action_outcome = false,
        state_only(),
        &[success],
    );
    hidden_variant(
        protocol,
        |g| g.entities[1].action_outcome = false,
        state_only(),
        &[success],
    );
    hidden_variant(
        protocol,
        |g| g.entities[0].action_damage = false,
        state_only(),
        &[success],
    );
    hidden_variant(
        protocol,
        |g| g.entities[1].action_damage = false,
        state_only(),
        &[success],
    );
    // Action actor/target roles.
    hidden_variant(
        protocol,
        |g| g.entities[0].action_actor = false,
        state_only(),
        &[success],
    );
    hidden_variant(
        protocol,
        |g| g.entities[1].action_target = false,
        state_only(),
        &[success],
    );
    // Turn facts individually.
    let no_turn_start = || {
        vec![
            resolved(1, 2, strike(), "success", 3),
            health(1, 10, 10, false),
            health(2, 7, 10, false),
            turn(Some(2), 0),
        ]
    };
    hidden_variant(protocol, |g| g.turn_round = false, no_turn_start(), &[]);
    hidden_variant(
        protocol,
        |g| g.entities[1].turn_start = false,
        no_turn_start(),
        &[],
    );
    hidden_variant(
        protocol,
        |g| g.turn_state = false,
        vec![
            resolved(1, 2, strike(), "success", 3),
            started(2, 0),
            health(1, 10, 10, false),
            health(2, 7, 10, false),
        ],
        &[],
    );
    // Health on the target only: its Health op is gone, nothing else.
    hidden_variant(
        protocol,
        |g| g.entities[1].health = false,
        vec![
            resolved(1, 2, strike(), "success", 3),
            started(2, 0),
            health(1, 10, 10, false),
            turn(Some(2), 0),
        ],
        &[],
    );

    // Encounter facts: drive to a natural end; one peer lacks the
    // encounter id, one lacks the end grant.
    for edit in [
        (|g: &mut DisclosureGrants| g.encounters.clear()) as fn(&mut DisclosureGrants),
        |g: &mut DisclosureGrants| g.encounter_end = false,
    ] {
        let mut h = H::new(protocol);
        let peer = h.bind(all_actors(), full_grants());
        let mut grants = full_grants();
        edit(&mut grants);
        let restricted = h.bind(control(&[]), grants);
        let bytes = h.declare(peer, 1, 1, smite(), 3);
        assert_eq!(h.send(peer, &bytes).status, APPLIED);
        let bytes = h.end_turn(peer, 2, 2);
        assert_eq!(h.send(peer, &bytes).status, APPLIED);
        h.drain(restricted);
        h.drain(peer);
        let bytes = h.declare(peer, 3, 1, smite(), 2);
        assert_eq!(h.send(peer, &bytes).status, APPLIED);
        let full_frames = h.drain(peer);
        assert_eq!(
            full_frames[0].ops,
            for_version(
                protocol,
                vec![
                    receipt(h.epoch(peer), 3, 0, APPLIED),
                    resolved(1, 2, smite(), "critical_success", 4_000_000_000),
                    died(2),
                    ended(1),
                    health(1, 10, 10, false),
                    health(2, 0, 10, true),
                    turn(None, 1),
                ]
            )
        );
        let frames = h.drain(restricted);
        assert_eq!(
            ops_of(&frames),
            vec![for_version(
                protocol,
                vec![
                    resolved(1, 2, smite(), "critical_success", 4_000_000_000),
                    died(2),
                    health(1, 10, 10, false),
                    health(2, 0, 10, true),
                    turn(None, 1),
                ]
            )]
        );
    }

    // Cached replies carry the receipt only: retry the first command after
    // its delivery was acknowledged.
    let mut h = H::new(protocol);
    let peer = h.bind(all_actors(), full_grants());
    h.drain(peer);
    let bytes = h.declare(peer, 1, 1, strike(), 2);
    assert_eq!(h.send(peer, &bytes).status, APPLIED);
    h.drain(peer);
    let retry = h.send(peer, &bytes);
    assert!(retry.cached);
    let frames = h.drain(peer);
    assert_eq!(
        ops_of(&frames),
        vec![vec![receipt(h.epoch(peer), 1, 0, APPLIED)]]
    );
    assert!(!frames[0].bytes.windows(success.len()).any(|w| w == success));
}
both_versions!(hidden_fields_absent_from_snapshots_and_replies);

fn two_peers_share_lane_sequences_safely(protocol: ProtocolSelection) {
    let mut h = H::new(protocol);
    let first = h.bind(control(&[a()]), full_grants());
    let second = h.bind(control(&[b(), c()]), full_grants());
    assert_ne!(h.epoch(first), h.epoch(second));
    let bytes_first = h.end_turn(first, 1, 1);
    assert_eq!(h.send(first, &bytes_first), result(first, Some(1), APPLIED));
    h.mirror(CombatAction::EndTurn { actor: a() });
    let bytes_second = h.end_turn(second, 1, 2);
    assert_eq!(
        h.send(second, &bytes_second),
        result(second, Some(1), APPLIED)
    );
    h.mirror(CombatAction::EndTurn { actor: b() });
    h.assert_authority();
    let records = h.host.read_captures(0, 256).expect("readable");
    assert_eq!(records.len(), 2);
    assert_eq!((records[0].epoch, records[0].seq), (h.epoch(first), 1));
    assert_eq!((records[1].epoch, records[1].seq), (h.epoch(second), 1));
    // Receipts went to each originator only.
    let first_frames = h.drain(first);
    let second_frames = h.drain(second);
    let receipts = |frames: &[Frame]| {
        let mut replica = Replica::new();
        replica.apply(frames);
        replica.receipts
    };
    assert_eq!(
        receipts(&first_frames),
        vec![receipt(h.epoch(first), 1, 0, APPLIED)]
    );
    assert_eq!(
        receipts(&second_frames),
        vec![receipt(h.epoch(second), 1, 0, APPLIED)]
    );
    // A byte-swap conflict on the first session fences only it.
    let first_epoch = h.epoch(first);
    let conflict = end_turn(h.version, first_epoch, 1, 0, 3);
    let before = snapshot(&h.host);
    assert_eq!(
        h.send(first, &conflict),
        result(first, Some(1), rejected(RejectionCode::SeqConflict))
    );
    assert_eq!(snapshot(&h.host), before);
    assert_eq!(h.host.epoch(first), Err(HostError::UnknownPeer));
    assert_eq!(h.host.take_delivery(first), Err(HostError::UnknownPeer));
    // Further bytes on the fenced handle are unauthenticated at ingress.
    h.advance();
    assert_eq!(
        h.host.ingest(first, &bytes_first, h.now),
        Ok(IngestDisposition::Refused {
            status: rejected(RejectionCode::Unauthenticated)
        })
    );
    // The other session continues.
    let bytes = h.end_turn(second, 2, 3);
    assert_eq!(h.send(second, &bytes), result(second, Some(2), APPLIED));
    h.mirror(CombatAction::EndTurn { actor: c() });
    h.assert_authority();
    // A rebind gets a new handle/epoch and fresh replica ids.
    let rebound = h.bind(control(&[a()]), full_grants());
    assert_eq!(rebound.get(), 3);
    assert_eq!(h.epoch(rebound), epoch_of(1, 3));
}
both_versions!(two_peers_share_lane_sequences_safely);

fn capture_before_second_mutation(protocol: ProtocolSelection) {
    let mut h = H::new(protocol);
    let peer = h.bind(all_actors(), full_grants());
    let epoch = h.epoch(peer);
    let bytes = h.declare(peer, 1, 1, strike(), 2);
    assert_eq!(h.send(peer, &bytes).status, APPLIED);
    let page_one = h.mirror(CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    });
    let first_view = CapturedView {
        epoch,
        health: vec![
            CapturedHealth {
                entity: n(1),
                health: 10,
                max_health: 10,
                dead: false,
            },
            CapturedHealth {
                entity: n(2),
                health: 7,
                max_health: 10,
                dead: false,
            },
        ],
        turn: Some(CapturedTurn {
            active: Some(n(2)),
            round: 0,
        }),
    };
    let expected_first = CapturedRecord {
        capture_seq: 1,
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        history_start: 4,
        history_end: 6,
        outcome: Some(CapturedOutcome {
            actor: a(),
            target: b(),
            ability: strike(),
            outcome: "success".to_owned(),
            damage: 3,
        }),
        processed_tick: 0,
        events: page_one,
        views: vec![first_view],
    };
    assert_eq!(h.last_record(), expected_first);
    // The second mutation changes A's health; the first record is intact.
    let bytes = h.declare(peer, 2, 2, strike(), 1);
    assert_eq!(h.send(peer, &bytes).status, APPLIED);
    h.mirror(CombatAction::UseAbility {
        actor: b(),
        ability: strike(),
        target: a(),
    });
    let records = h.host.read_captures(0, 256).expect("readable");
    assert_eq!(records[0], expected_first);
    assert_eq!(records[1].views[0].health[0].health, 7);
    // Accepted EndTurn: Applied, outcome None, no ActionResolved.
    let bytes = h.end_turn(peer, 3, 3);
    assert_eq!(h.send(peer, &bytes).status, APPLIED);
    let page = h.mirror(CombatAction::EndTurn { actor: c() });
    h.assert_authority();
    let end_record = h.last_record();
    assert_eq!(end_record.outcome, None);
    assert_eq!(end_record.events, page);
    assert!(end_record
        .events
        .iter()
        .all(|e| !matches!(e.payload, HistoryEvent::ActionResolved { .. })));
    assert!(matches!(
        end_record.events[0].payload,
        HistoryEvent::TurnStarted { round: 1, .. }
    ));
    assert_eq!(h.host.executions(), 3);

    // Paging, cursors, limits, with positive controls.
    assert_eq!(h.host.last_capture(), 3);
    assert_eq!(h.host.read_captures(0, 2).expect("page").len(), 2);
    assert_eq!(
        h.host.read_captures(2, 256).expect("page")[0].capture_seq,
        3
    );
    assert!(h.host.read_captures(3, 1).expect("page").is_empty());
    assert_eq!(
        h.host.read_captures(0, 0),
        Err(HostError::InvalidPageLimit { limit: 0 })
    );
    assert_eq!(
        h.host.read_captures(0, MAX_CAPTURE_PAGE + 1),
        Err(HostError::InvalidPageLimit {
            limit: MAX_CAPTURE_PAGE + 1
        })
    );
    assert!(h.host.read_captures(0, MAX_CAPTURE_PAGE).is_ok());
    assert_eq!(
        h.host.read_captures(4, 1),
        Err(HostError::FutureCaptureCursor {
            requested: 4,
            last: 3
        })
    );
    assert_eq!(
        h.host.acknowledge_captures(4),
        Err(HostError::FutureCaptureCursor {
            requested: 4,
            last: 3
        })
    );
    h.host.acknowledge_captures(1).expect("acks");
    assert_eq!(h.host.capture_acknowledged(), 1);
    assert_eq!(
        h.host.read_captures(0, 1),
        Err(HostError::StaleCaptureCursor {
            requested: 0,
            acknowledged: 1
        })
    );
    h.host.acknowledge_captures(1).expect("idempotent");
    h.host.acknowledge_captures(0).expect("idempotent below");
    assert_eq!(h.host.read_captures(1, 256).expect("page").len(), 2);
}
both_versions!(capture_before_second_mutation);

fn lost_reply_duplicate_delivery_executes_once(protocol: ProtocolSelection) {
    let mut h = H::new(protocol);
    let peer = h.bind(all_actors(), full_grants());
    let epoch = h.epoch(peer);
    let bytes = h.declare(peer, 1, 1, strike(), 2);
    assert_eq!(h.send(peer, &bytes).status, APPLIED);
    h.mirror(CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    });
    let before = snapshot(&h.host);
    // Delivery is lost: re-take returns identical bytes and sequences.
    let lost = h.host.take_delivery(peer).expect("bound");
    assert_eq!(lost.len(), 2);
    assert_eq!(h.host.take_delivery(peer).expect("bound"), lost);
    // Duplicate delivery through the byte harness applies once.
    let frames: Vec<Frame> = lost.iter().map(|b| decode_frame(protocol, b)).collect();
    let mut replica = Replica::new();
    replica.apply(&frames);
    replica.apply(&frames);
    replica.apply(&frames[1..]);
    assert_eq!(replica.receipts.len(), 1);
    // The client retries the identical bytes: cached, nothing re-executes,
    // and the still-unacknowledged receipt is not duplicated.
    h.host.tick().expect("trusted tick");
    h.oracle.tick().expect("mirror tick");
    let retry = h.send(peer, &bytes);
    assert_eq!(
        retry,
        AdmissionResult {
            cached: true,
            ..result(peer, Some(1), APPLIED)
        }
    );
    assert_eq!(h.host.take_delivery(peer).expect("bound"), lost);
    let after_tick = Snapshot {
        hash: h.host.authority_hash(),
        ..before
    };
    assert_eq!(snapshot(&h.host), after_tick);
    h.assert_authority();
    // After acknowledgement, a retry stages a fresh identical receipt with
    // the original processed tick at the next delivery sequence.
    h.host.acknowledge_delivery(peer, 2).expect("acks");
    h.host
        .acknowledge_delivery(peer, 1)
        .expect("idempotent below");
    assert_eq!(
        h.host.acknowledge_delivery(peer, 3),
        Err(HostError::FutureDeliveryCursor {
            requested: 3,
            last: 2
        })
    );
    let retry = h.send(peer, &bytes);
    assert!(retry.cached);
    let frames = h.drain(peer);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].event_seq, 3);
    assert_eq!(frames[0].server_tick, 1);
    assert_eq!(frames[0].ops, vec![receipt(epoch, 1, 0, APPLIED)]);
    replica.apply(&frames);
    assert_eq!(replica.receipts.len(), 2);
    assert_eq!(replica.receipts[0], replica.receipts[1]);
    assert_eq!(h.host.executions(), 1);
    assert_eq!(h.host.last_capture(), 1);
    h.assert_authority();
}
both_versions!(lost_reply_duplicate_delivery_executes_once);

fn narrowing_grants_resynchronize_without_leak(protocol: ProtocolSelection) {
    let mut h = H::new(protocol);
    let watcher = h.bind(control(&[]), full_grants());
    let actor_peer = h.bind(all_actors(), full_grants());
    h.drain(actor_peer);
    let bytes = h.declare(actor_peer, 1, 1, strike(), 2);
    assert_eq!(h.send(actor_peer, &bytes).status, APPLIED);
    h.mirror(CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    });
    // Detailed delivery is staged but unacknowledged.
    assert_eq!(h.host.take_delivery(watcher).expect("bound").len(), 2);
    let mut narrowed = full_grants();
    narrowed.entities[1].present = false;
    assert_eq!(
        h.host.set_grants(watcher, narrowed.clone()),
        Ok(GrantUpdate::RebindRequired)
    );
    // Stale frames are gone; delivery restarts at 1 with fresh state.
    let frames = h.drain(watcher);
    assert_eq!(
        ops_of(&frames),
        vec![vec![
            enter(1),
            enter(3),
            health(1, 10, 10, false),
            health(3, 3, 3, false),
            turn(None, 0)
        ]]
    );
    assert_eq!(frames[0].event_seq, 1);
    // A later command by hidden B (killing C, rolling the round over to A)
    // leaks nothing about B (old id 2).
    let bytes = h.declare(actor_peer, 2, 2, strike(), 3);
    assert_eq!(h.send(actor_peer, &bytes).status, APPLIED);
    h.mirror(CombatAction::UseAbility {
        actor: b(),
        ability: strike(),
        target: c(),
    });
    let later = h.drain(watcher);
    assert_eq!(
        ops_of(&later),
        vec![for_version(
            protocol,
            vec![
                died(3),
                started(1, 1),
                health(1, 10, 10, false),
                health(3, 0, 3, true),
                turn(Some(1), 1)
            ]
        )]
    );
    assert_eq!(later[0].event_seq, 2);
    let mut replica = Replica::new();
    replica.apply(&frames);
    replica.apply(&later);
    let ids = BTreeMap::from([(a(), 1), (c(), 3)]);
    assert_eq!(replica.state, expected_state(&h.oracle, &narrowed, &ids));
    for frame in &later {
        for op in &frame.ops {
            assert!(!format!("{op:?}").contains("NetId(2)"), "{op:?}");
        }
    }
    // Widening preserves continuity and mints a fresh id for B (never 2).
    assert_eq!(
        h.host.set_grants(watcher, full_grants()),
        Ok(GrantUpdate::Unchanged)
    );
    let widened = h.drain(watcher);
    assert_eq!(
        ops_of(&widened),
        vec![vec![enter(4), health(4, 7, 10, false)]]
    );
    assert_eq!(widened[0].event_seq, 3);
    replica.apply(&widened);
    let ids = BTreeMap::from([(a(), 1), (b(), 4), (c(), 3)]);
    assert_eq!(
        replica.state,
        expected_state(&h.oracle, &full_grants(), &ids)
    );
    // set_control never resynchronizes delivery.
    h.host.set_control(watcher, control(&[a()])).expect("bound");
    assert!(h.host.take_delivery(watcher).expect("bound").is_empty());
    h.assert_authority();
}
both_versions!(narrowing_grants_resynchronize_without_leak);

fn capacity_boundary_no_partial_commit(protocol: ProtocolSelection) {
    // (a) Capture journal: 4096 accepted commands fit, the last being the
    // worst-case action (an overkill kill producing three events) at
    // bound-minus-one; the next is retryable QueueFull until the trusted
    // consumer acknowledges. The retry cache sits at its 256-entry bound
    // throughout (eviction is its retirement path).
    let mut h = H::new(protocol);
    let peer = h.bind(all_actors(), full_grants());
    let order = [1u64, 2, 3];
    for seq in 1..MAX_CAPTURE_RECORDS as u64 {
        let actor = order[((seq - 1) % 3) as usize];
        let bytes = h.end_turn(peer, seq, actor);
        assert_eq!(h.send(peer, &bytes).status, APPLIED, "seq {seq}");
        h.drain(peer);
    }
    let last = MAX_CAPTURE_RECORDS as u64;
    assert_eq!(order[((last - 1) % 3) as usize], 1, "A holds the turn");
    let bytes = h.declare(peer, last, 1, smite(), 3);
    assert_eq!(h.send(peer, &bytes).status, APPLIED);
    h.drain(peer);
    let worst = h.last_record();
    assert_eq!(worst.events.len(), MAX_NEW_EVENTS_PER_COMMAND);
    assert!(serde_json::to_vec(&worst).expect("encodes").len() <= MAX_CAPTURE_RECORD_BYTES);
    assert_eq!(h.host.last_capture(), MAX_CAPTURE_RECORDS as u64);
    assert_eq!(h.host.cached_status(peer, 1), Ok(None), "evicted");
    let seq = MAX_CAPTURE_RECORDS as u64 + 1;
    // C is dead; B holds the next turn.
    let blocked = h.end_turn(peer, seq, 2);
    let before = snapshot(&h.host);
    h.ingest(peer, &blocked);
    let summary = h.pump();
    assert_eq!(summary.backpressured, 1);
    assert_eq!(summary.admitted, 0);
    assert_eq!(
        summary.results,
        vec![AdmissionResult {
            retained_in_ingress: true,
            ..result(peer, Some(seq), rejected(RejectionCode::QueueFull))
        }]
    );
    assert_eq!(snapshot(&h.host), before);
    assert_eq!(h.host.cached_status(peer, seq), Ok(None));
    assert_eq!(
        h.host.save_checkpoint(),
        Err(CheckpointError::PendingIngress)
    );
    // The client's identical retry joins the queue behind the head.
    h.ingest(peer, &blocked);
    h.host.acknowledge_captures(1).expect("consumer acks");
    let summary = h.pump();
    assert_eq!(
        summary.results,
        vec![
            result(peer, Some(seq), APPLIED),
            AdmissionResult {
                cached: true,
                ..result(peer, Some(seq), APPLIED)
            }
        ]
    );
    assert_eq!(h.host.executions(), MAX_CAPTURE_RECORDS as u64 + 1);

    // (b) Egress: an originator that never acknowledges blocks its own
    // terminal freshness/authorization candidates and new accepts.
    for candidate in ["future", "unauthorized", "accepted"] {
        let mut h = H::new(protocol);
        let peer = h.bind(all_actors(), full_grants());
        let mut seq = 1;
        // Initial state frame plus 127 accepted commands fill 128 frames.
        for _ in 0..127 {
            let actor = order[((seq - 1) % 3) as usize];
            let bytes = h.end_turn(peer, seq, actor);
            assert_eq!(h.send(peer, &bytes).status, APPLIED);
            h.mirror(CombatAction::EndTurn {
                actor: [a(), b(), c()][((seq - 1) % 3) as usize],
            });
            seq += 1;
        }
        let actor = order[((seq - 1) % 3) as usize];
        let (bytes, status) = match candidate {
            "future" => (
                end_turn(h.version, h.epoch(peer), seq, 99, actor),
                rejected(RejectionCode::FutureTick),
            ),
            "unauthorized" => (
                h.end_turn(peer, seq, 77),
                rejected(RejectionCode::NotAuthorized),
            ),
            _ => (h.end_turn(peer, seq, actor), APPLIED),
        };
        let before = snapshot(&h.host);
        h.ingest(peer, &bytes);
        let summary = h.pump();
        assert_eq!(summary.backpressured, 1, "{candidate}");
        assert_eq!(snapshot(&h.host), before);
        assert_eq!(h.host.cached_status(peer, seq), Ok(None));
        h.drain(peer);
        let summary = h.pump();
        assert_eq!(summary.results, vec![result(peer, Some(seq), status)]);
        assert_eq!(h.host.cached_status(peer, seq), Ok(Some(status)));
        if status == APPLIED {
            h.mirror(CombatAction::EndTurn {
                actor: [a(), b(), c()][((seq - 1) % 3) as usize],
            });
        }
        h.assert_authority();
    }

    // (c) A non-acknowledging observer blocks accepts (per-peer worst case
    // across all bound peers), never by dropping an accepted result.
    let mut h = H::new(protocol);
    let peer = h.bind(all_actors(), full_grants());
    let slow = h.bind(control(&[]), observer_grants());
    for seq in 1..=127u64 {
        let actor = order[((seq - 1) % 3) as usize];
        let bytes = h.end_turn(peer, seq, actor);
        assert_eq!(h.send(peer, &bytes).status, APPLIED);
        h.drain(peer);
    }
    let bytes = h.end_turn(peer, 128, order[(127 % 3) as usize]);
    h.ingest(peer, &bytes);
    assert_eq!(h.pump().backpressured, 1);
    h.drain(slow);
    assert_eq!(h.pump().results, vec![result(peer, Some(128), APPLIED)]);

    // (d) Worst-case shapes fit the per-command bounds: a real overkill
    // kill ending the encounter, fully disclosed to eight peers.
    let mut h = H::new(protocol);
    let peers: Vec<PeerHandle> = (0..MAX_PEERS)
        .map(|_| h.bind(all_actors(), full_grants()))
        .collect();
    assert_eq!(
        h.host.bind_peer(all_actors(), full_grants(), h.now),
        Err(HostError::ServerBusy)
    );
    let lead = peers[0];
    let bytes = h.declare(lead, 1, 1, smite(), 3);
    assert_eq!(h.send(lead, &bytes).status, APPLIED);
    let bytes = h.end_turn(lead, 2, 2);
    assert_eq!(h.send(lead, &bytes).status, APPLIED);
    for peer in &peers {
        h.drain(*peer);
    }
    let bytes = h.declare(lead, 3, 1, smite(), 2);
    assert_eq!(h.send(lead, &bytes).status, APPLIED);
    let record = h.last_record();
    assert_eq!(record.events.len(), MAX_NEW_EVENTS_PER_COMMAND);
    assert_eq!(record.views.len(), MAX_PEERS);
    let record_bytes = serde_json::to_vec(&record).expect("encodes").len();
    assert!(record_bytes <= MAX_CAPTURE_RECORD_BYTES, "{record_bytes}");
    for peer in &peers {
        for frame in h.drain(*peer) {
            assert!(frame.ops.len() <= MAX_DELIVERY_OPS_PER_COMMAND);
            assert!(frame.bytes.len() <= MAX_DELIVERY_BYTES_PER_COMMAND_PEER);
        }
    }
}
both_versions!(capacity_boundary_no_partial_commit);

/// Max-shape measurement: the largest representable record and delivery
/// frame fit their bounds (ADR-0022 consequences), independent of any run.
#[test]
fn worst_case_shapes_fit_their_bounds() {
    let id: EntityId =
        serde_json::from_value(json!({"index": u32::MAX, "generation": u32::MAX - 1}))
            .expect("largest issuable id");
    let ulid: Ulid = "7ZZZZZZZZZZZZZZZZZZZZZZZZZ".parse().expect("max ULID");
    let outcome = "critical_success";
    let envelope = |seq, payload| HistoryEnvelope {
        tick: crpg_core::Tick::new(u64::MAX),
        seq,
        payload,
    };
    let events = vec![
        envelope(
            u64::MAX - 3,
            HistoryEvent::ActionResolved {
                actor: id,
                target: id,
                ability: ulid,
                outcome: outcome.to_owned(),
                damage: u32::MAX,
            },
        ),
        envelope(u64::MAX - 2, HistoryEvent::Died { entity: id }),
        envelope(
            u64::MAX - 1,
            HistoryEvent::EncounterEnded {
                encounter: ulid,
                round: u64::MAX,
            },
        ),
    ];
    let max_net = n(u64::MAX);
    let view = CapturedView {
        epoch: [255; 16],
        health: vec![
            CapturedHealth {
                entity: max_net,
                health: u32::MAX,
                max_health: u32::MAX,
                dead: false,
            };
            3
        ],
        turn: Some(CapturedTurn {
            active: Some(max_net),
            round: u64::MAX,
        }),
    };
    let record = CapturedRecord {
        capture_seq: u64::MAX - 1,
        epoch: [255; 16],
        lane: LANE_COMBAT,
        seq: SEQ_LAST,
        history_start: u64::MAX - 4,
        history_end: u64::MAX - 1,
        outcome: Some(CapturedOutcome {
            actor: id,
            target: id,
            ability: ulid,
            outcome: outcome.to_owned(),
            damage: u32::MAX,
        }),
        processed_tick: u64::MAX,
        events,
        views: vec![view; MAX_PEERS],
    };
    let record_bytes = serde_json::to_vec(&record).expect("encodes").len();
    assert!(record_bytes <= MAX_CAPTURE_RECORD_BYTES, "{record_bytes}");

    let max_ops = vec![
        receipt(
            [255; 16],
            SEQ_LAST,
            u64::MAX,
            rejected(RejectionCode::SnapshotExpired),
        ),
        DeltaOp::ActionResolved {
            actor: max_net,
            target: max_net,
            ability: ulid,
            outcome: outcome.to_owned(),
            damage: u32::MAX,
        },
        died(u64::MAX),
        DeltaOp::EncounterEnded {
            encounter: ulid,
            round: u64::MAX,
        },
        health(u64::MAX, u32::MAX, u32::MAX, true),
        health(u64::MAX, u32::MAX, u32::MAX, true),
        health(u64::MAX, u32::MAX, u32::MAX, true),
        turn(Some(u64::MAX), u64::MAX),
    ];
    assert_eq!(max_ops.len(), MAX_DELIVERY_OPS_PER_COMMAND);
    let v2 = codec_v2::encode_delta(&protocol_v2::DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: u64::MAX,
        event_seq: u64::MAX,
        ops: max_ops,
    })
    .expect("encodes");
    assert!(
        v2.len() <= MAX_DELIVERY_BYTES_PER_COMMAND_PEER,
        "{}",
        v2.len()
    );
}

fn sequence_precedence_and_eviction(protocol: ProtocolSelection) {
    let mut h = H::new(protocol);
    let peer = h.bind(all_actors(), full_grants());
    let other = h.bind(all_actors(), full_grants());
    let order = [1u64, 2, 3];
    let actor_for = |seq: u64| order[((seq - 1) % 3) as usize];
    let first = h.end_turn(peer, 1, 1);
    assert_eq!(h.send(peer, &first), result(peer, Some(1), APPLIED));
    // Gap, stale-by-cache, and gap again consume nothing; counted in FIFO.
    let gap = h.end_turn(peer, 3, actor_for(3));
    let before = snapshot(&h.host);
    h.ingest(peer, &gap);
    h.ingest(peer, &first);
    h.ingest(peer, &gap);
    let summary = h.pump();
    assert_eq!(
        summary.results,
        vec![
            result(peer, Some(3), rejected(RejectionCode::SeqGap)),
            AdmissionResult {
                cached: true,
                ..result(peer, Some(1), APPLIED)
            },
            result(peer, Some(3), rejected(RejectionCode::SeqGap)),
        ]
    );
    assert_eq!((summary.applied, summary.rejected), (1, 2));
    assert_eq!(snapshot(&h.host), before);
    // seq 2 is still the next new sequence.
    for seq in 2..=257u64 {
        let bytes = h.end_turn(peer, seq, actor_for(seq));
        assert_eq!(h.send(peer, &bytes), result(peer, Some(seq), APPLIED));
        h.drain(peer);
        h.drain(other);
        if seq == 256 {
            assert_eq!(h.host.cached_status(peer, 1), Ok(Some(APPLIED)));
        }
    }
    // 257 entries: the oldest was evicted; its retry is StaleSeq, never
    // reapplied, while seq 2 still hits the cache.
    assert_eq!(h.host.cached_status(peer, 1), Ok(None));
    assert_eq!(h.host.cached_status(peer, 2), Ok(Some(APPLIED)));
    let before = snapshot(&h.host);
    assert_eq!(
        h.send(peer, &first),
        result(peer, Some(1), rejected(RejectionCode::StaleSeq))
    );
    let second = h.end_turn(peer, 2, actor_for(2));
    assert!(h.send(peer, &second).cached);
    assert_eq!(snapshot(&h.host), before);
    // Cache precedes conflict detection only for identical bytes; a
    // different-bytes reuse of a cached seq fences this session only.
    let swapped = end_turn(h.version, h.epoch(peer), 257, 0, 1);
    assert_eq!(
        h.send(peer, &swapped),
        result(peer, Some(257), rejected(RejectionCode::SeqConflict))
    );
    assert_eq!(snapshot(&h.host), before);
    assert_eq!(h.host.epoch(peer), Err(HostError::UnknownPeer));
    // The other peer is still admitted at its own seq 1.
    let bytes = h.end_turn(other, 1, actor_for(258));
    assert_eq!(h.send(other, &bytes), result(other, Some(1), APPLIED));
    assert_eq!(h.host.executions(), 258);
}
both_versions!(sequence_precedence_and_eviction);

/// Strips session identity from a record for cross-run comparison.
fn without_epochs(mut record: CapturedRecord) -> CapturedRecord {
    record.epoch = [0; 16];
    for view in &mut record.views {
        view.epoch = [0; 16];
    }
    record
}

/// Replaces receipt epochs in decoded ops for cross-run comparison.
fn without_receipt_epochs(ops: &[DeltaOp]) -> Vec<DeltaOp> {
    ops.iter()
        .map(|op| match op {
            DeltaOp::Legacy(protocol::DeltaOp::Receipt {
                lane,
                seq,
                processed_tick,
                status,
                ..
            }) => receipt_with(*lane, *seq, *processed_tick, *status),
            other => other.clone(),
        })
        .collect()
}

fn receipt_with(lane: u8, seq: u64, tick: u64, status: ReceiptStatus) -> DeltaOp {
    legacy(protocol::DeltaOp::Receipt {
        epoch: [0; 16],
        lane,
        seq,
        processed_tick: tick,
        status,
    })
}

/// The prefix schedule: three accepted commands, one rejected, one trusted
/// tick, and one capture acknowledgement.
fn run_prefix(h: &mut H, peer: PeerHandle) {
    let bytes = h.declare(peer, 1, 1, strike(), 2);
    assert_eq!(h.send(peer, &bytes).status, APPLIED);
    let bytes = h.declare(peer, 2, 2, whiff(), 1);
    assert_eq!(h.send(peer, &bytes).status, APPLIED);
    let bytes = h.declare(peer, 3, 1, strike(), 2);
    assert_eq!(
        h.send(peer, &bytes).status,
        rejected(RejectionCode::IllegalAction)
    );
    h.host.tick().expect("trusted tick");
    let bytes = h.end_turn(peer, 4, 3);
    assert_eq!(h.send(peer, &bytes).status, APPLIED);
    h.host.acknowledge_captures(1).expect("consumer acks");
    h.drain(peer);
}

/// The suffix schedule from a fresh binding at seq 1.
fn run_suffix(h: &mut H, peer: PeerHandle) -> Vec<Frame> {
    let mut frames = h.drain(peer);
    let bytes = h.declare(peer, 1, 1, smite(), 3);
    assert_eq!(h.send(peer, &bytes).status, APPLIED);
    let bytes = h.end_turn(peer, 2, 2);
    assert_eq!(h.send(peer, &bytes).status, APPLIED);
    let bytes = h.declare(peer, 3, 1, smite(), 2);
    assert_eq!(h.send(peer, &bytes).status, APPLIED);
    frames.extend(h.drain(peer));
    frames
}

fn checkpoint_continuation_equivalence(protocol: ProtocolSelection) {
    // Uninterrupted: prefix, rebind (fresh session), suffix.
    let mut uninterrupted = H::new(protocol);
    let peer = uninterrupted.bind(all_actors(), full_grants());
    run_prefix(&mut uninterrupted, peer);
    let retained_before = uninterrupted.host.read_captures(1, 256).expect("readable");
    uninterrupted.host.unbind_peer(peer);
    let peer = uninterrupted.bind(all_actors(), full_grants());
    let frames_u = run_suffix(&mut uninterrupted, peer);

    // Interrupted: prefix, save, destroy, load under a greater incarnation,
    // rebind identical operator facts, suffix.
    let mut interrupted = H::new(protocol);
    let peer = interrupted.bind(all_actors(), full_grants());
    run_prefix(&mut interrupted, peer);
    let saved = interrupted.host.save_checkpoint().expect("quiescent save");
    let saved_hash = interrupted.host.authority_hash();
    let now = interrupted.now;
    drop(std::mem::replace(
        &mut interrupted.host,
        Host::new(
            0,
            HostConfig {
                protocol,
                incarnation: 0,
            },
            0,
        )
        .expect("placeholder"),
    ));
    let loaded = match protocol {
        ProtocolSelection::V1 => load_checkpoint(&saved, 2, now),
        ProtocolSelection::V2 => load_checkpoint_from_reader(saved.as_slice(), 2, now),
    }
    .expect("loads");
    interrupted.host = loaded;
    interrupted.incarnation = 2;
    assert_eq!(interrupted.host.protocol(), protocol);
    assert_eq!(interrupted.host.incarnation(), 2);
    assert_eq!(interrupted.host.authority_hash(), saved_hash);
    assert_eq!(interrupted.host.executions(), 0, "per host lifetime");
    assert_eq!(interrupted.host.capture_acknowledged(), 1);
    assert_eq!(interrupted.host.last_capture(), 3);
    assert_eq!(
        interrupted.host.read_captures(1, 256).expect("readable"),
        retained_before
    );
    // Re-saving the loaded host differs only in the incarnation field.
    let resaved = interrupted.host.save_checkpoint().expect("save");
    assert_eq!(
        String::from_utf8(resaved).expect("utf8"),
        String::from_utf8(saved).expect("utf8").replacen(
            "\"incarnation\":1,",
            "\"incarnation\":2,",
            1
        )
    );
    let peer = interrupted.bind(all_actors(), full_grants());
    assert_eq!(interrupted.epoch(peer), epoch_of(2, 1));
    let frames_i = run_suffix(&mut interrupted, peer);

    // Authority, journal, captures, watermarks, RNG/event bytes identical.
    assert_eq!(
        interrupted.host.authority_hash(),
        uninterrupted.host.authority_hash()
    );
    assert_eq!(
        interrupted.host.last_capture(),
        uninterrupted.host.last_capture()
    );
    assert_eq!(
        interrupted.host.capture_acknowledged(),
        uninterrupted.host.capture_acknowledged()
    );
    let records_i = interrupted.host.read_captures(1, 256).expect("readable");
    let records_u = uninterrupted.host.read_captures(1, 256).expect("readable");
    assert_eq!(records_i.len(), 5);
    assert_eq!(
        records_i
            .into_iter()
            .map(without_epochs)
            .collect::<Vec<_>>(),
        records_u
            .into_iter()
            .map(without_epochs)
            .collect::<Vec<_>>()
    );
    // New receipts and delivery differ in epoch bytes only.
    assert_eq!(frames_i.len(), frames_u.len());
    for (left, right) in frames_i.iter().zip(&frames_u) {
        assert_eq!(left.event_seq, right.event_seq);
        assert_eq!(left.server_tick, right.server_tick);
        assert_eq!(
            without_receipt_epochs(&left.ops),
            without_receipt_epochs(&right.ops)
        );
        assert_eq!(left.bytes.len(), right.bytes.len());
    }
    // The mirror authority agrees with both.
    let mut mirror = fixture();
    for action in [
        CombatAction::UseAbility {
            actor: a(),
            ability: strike(),
            target: b(),
        },
        CombatAction::UseAbility {
            actor: b(),
            ability: whiff(),
            target: a(),
        },
    ] {
        mirror.perform_action(&action).expect("accepts");
        mirror.acknowledge(mirror.last_sequence()).expect("acks");
    }
    mirror.tick().expect("ticks");
    for action in [
        CombatAction::EndTurn { actor: c() },
        CombatAction::UseAbility {
            actor: a(),
            ability: smite(),
            target: c(),
        },
        CombatAction::EndTurn { actor: b() },
        CombatAction::UseAbility {
            actor: a(),
            ability: smite(),
            target: b(),
        },
    ] {
        mirror.perform_action(&action).expect("accepts");
        mirror.acknowledge(mirror.last_sequence()).expect("acks");
    }
    assert_eq!(interrupted.host.authority_hash(), history_hash(&mirror));
}
both_versions!(checkpoint_continuation_equivalence);

fn old_epoch_refused_after_restart(protocol: ProtocolSelection) {
    let mut h = H::new(protocol);
    let peer = h.bind(all_actors(), full_grants());
    let old = h.end_turn(peer, 1, 1);
    assert_eq!(h.send(peer, &old).status, APPLIED);
    h.mirror(CombatAction::EndTurn { actor: a() });
    let pending_old = h.end_turn(peer, 2, 2);
    let saved = h.host.save_checkpoint().expect("save");
    // Equal or lower incarnations are refused.
    assert!(matches!(
        load_checkpoint(&saved, 1, h.now),
        Err(CheckpointError::EpochReused)
    ));
    assert!(matches!(
        load_checkpoint(&saved, 0, h.now),
        Err(CheckpointError::EpochReused)
    ));
    h.host = load_checkpoint(&saved, 5, h.now).expect("loads");
    let fresh = h.bind(all_actors(), full_grants());
    assert_eq!(fresh.get(), 1, "handles restart at 1");
    assert_eq!(h.epoch(fresh), epoch_of(5, 1));
    let before = snapshot(&h.host);
    // Pre-restart bytes on the new binding: SessionExpired, never executed.
    for (bytes, seq) in [(&old, 1), (&pending_old, 2)] {
        assert_eq!(
            h.send(fresh, bytes),
            result(fresh, Some(seq), rejected(RejectionCode::SessionExpired))
        );
    }
    assert_eq!(snapshot(&h.host), before);
    // The same logical action under the new epoch succeeds.
    let bytes = h.end_turn(fresh, 1, 2);
    assert_eq!(h.send(fresh, &bytes), result(fresh, Some(1), APPLIED));
    h.mirror(CombatAction::EndTurn { actor: b() });
    h.assert_authority();
}
both_versions!(old_epoch_refused_after_restart);

fn error_precedence_matrix(protocol: ProtocolSelection) {
    let mut h = H::new(protocol);
    let gone = h.bind(all_actors(), full_grants());
    h.host.unbind_peer(gone);
    let peer = h.bind(control(&[a(), c()]), full_grants());
    let epoch = h.epoch(peer);
    h.drain(peer);

    // Ingest: open, then time (a regression changes nothing), then binding.
    h.advance();
    let bytes = h.end_turn(peer, 1, 1);
    assert_eq!(
        h.host.ingest(gone, &bytes, h.now),
        Ok(IngestDisposition::Refused {
            status: rejected(RejectionCode::Unauthenticated)
        })
    );
    assert_eq!(
        h.host.ingest(peer, &bytes, h.now - 1),
        Err(HostError::TimeRegression)
    );
    assert_eq!(
        h.host.ingest(peer, &bytes, h.now),
        Ok(IngestDisposition::Staged)
    );
    assert_eq!(h.host.pump(h.now - 1), Err(HostError::TimeRegression));
    assert_eq!(h.pump().results, vec![result(peer, Some(1), APPLIED)]);
    h.mirror(CombatAction::EndTurn { actor: a() });
    h.drain(peer);
    let mut seq = 2;
    // Frame cap: oversized bytes are refused before staging.
    let huge = vec![h.version; MAX_INTENT_FRAME_BYTES + 1];
    assert_eq!(
        h.host.ingest(peer, &huge, h.now),
        Ok(IngestDisposition::Refused {
            status: rejected(RejectionCode::FrameTooLarge)
        })
    );
    // Rate budgets precede the frame cap: exhaust the burst at one instant.
    let junk = [h.version];
    let mut staged = 0;
    loop {
        match h.host.ingest(peer, &junk, h.now) {
            Ok(IngestDisposition::Staged) => staged += 1,
            Ok(IngestDisposition::Refused { status }) => {
                assert_eq!(status, rejected(RejectionCode::RateLimited));
                break;
            }
            Err(error) => panic!("{error:?}"),
        }
        assert!(staged < 200, "the burst is bounded");
    }
    assert_eq!(
        h.host.ingest(peer, &huge, h.now),
        Ok(IngestDisposition::Refused {
            status: rejected(RejectionCode::RateLimited)
        })
    );
    // Undecodable staged bytes: Malformed, no sequence consumed, no reply.
    let summary = h.host.pump(h.now).expect("pump");
    assert_eq!(summary.results.len(), staged);
    assert!(summary
        .results
        .iter()
        .all(|r| *r == result(peer, None, rejected(RejectionCode::Malformed))));
    assert!(h.drain(peer).is_empty());
    // Replenish with injected time.
    h.now += 10_000;

    // Pump order: decode < session < cache < exhaustion < gap/stale <
    // freshness < mapping < gameplay. Each negative pairs with a control.
    let before = snapshot(&h.host);
    let wrong_epoch = end_turn(h.version, [9; 16], seq, 0, 2);
    let mut truncated = wrong_epoch.clone();
    truncated.truncate(10);
    let cases: Vec<(Vec<u8>, Option<u64>, RejectionCode)> = vec![
        (truncated, None, RejectionCode::Malformed),
        (wrong_epoch, Some(seq), RejectionCode::SessionExpired),
        // A gap with a future tick is still a gap (no sequence consumed).
        (
            end_turn(h.version, epoch, seq + 1, 999, 2),
            Some(seq + 1),
            RejectionCode::SeqGap,
        ),
    ];
    for (bytes, expected_seq, code) in cases {
        assert_eq!(
            h.send(peer, &bytes),
            result(peer, expected_seq, rejected(code))
        );
    }
    assert_eq!(snapshot(&h.host), before);
    assert_eq!(h.host.cached_status(peer, seq), Ok(None));
    // Freshness precedes mapping: future tick with an unmapped actor.
    let bytes = end_turn(h.version, epoch, seq, 5, 99);
    assert_eq!(
        h.send(peer, &bytes),
        result(peer, Some(seq), rejected(RejectionCode::FutureTick))
    );
    seq += 1;
    // Stale tick at the window edge, after 300 trusted ticks.
    for _ in 0..300 {
        h.host.tick().expect("tick");
        h.oracle.tick().expect("tick");
    }
    let bytes = end_turn(h.version, epoch, seq, 99, 99);
    assert_eq!(
        h.send(peer, &bytes),
        result(peer, Some(seq), rejected(RejectionCode::StaleTick))
    );
    seq += 1;
    // Mapping: unmapped actor; uncontrolled active head; then a controlled
    // but out-of-turn actor passes mapping and fails gameplay.
    let bytes = end_turn(h.version, epoch, seq, 100, 99);
    assert_eq!(
        h.send(peer, &bytes),
        result(peer, Some(seq), rejected(RejectionCode::NotAuthorized))
    );
    seq += 1;
    let bytes = end_turn(h.version, epoch, seq, 100, 2);
    assert_eq!(
        h.send(peer, &bytes),
        result(peer, Some(seq), rejected(RejectionCode::NotAuthorized))
    );
    seq += 1;
    let bytes = end_turn(h.version, epoch, seq, 100, 3);
    assert_eq!(
        h.send(peer, &bytes),
        result(peer, Some(seq), rejected(RejectionCode::IllegalAction))
    );
    assert!(matches!(
        h.host.last_combat_error(),
        Some(CombatError::OutOfTurn { .. })
    ));
    seq += 1;
    let mut hidden = full_grants();
    hidden.entities[0].action_target = false;
    assert_eq!(
        h.host.set_grants(peer, hidden),
        Ok(GrantUpdate::RebindRequired)
    );
    h.drain(peer);
    h.host
        .set_control(peer, control(&[a(), b(), c()]))
        .expect("bound");
    let bytes = declare(h.version, epoch, seq, 100, 2, strike(), 1);
    assert_eq!(
        h.send(peer, &bytes),
        result(peer, Some(seq), rejected(RejectionCode::NotAuthorized))
    );
    seq += 1;
    let bytes = end_turn(h.version, epoch, seq, 100, 1);
    assert_eq!(
        h.send(peer, &bytes),
        result(peer, Some(seq), rejected(RejectionCode::IllegalAction))
    );
    seq += 1;
    h.restore_full_grants(peer);
    // Positive control at the freshness edge (observed == tick - 200).
    let bytes = declare(h.version, epoch, seq, 100, 2, strike(), 3);
    assert_eq!(h.send(peer, &bytes), result(peer, Some(seq), APPLIED));
    h.mirror(CombatAction::UseAbility {
        actor: b(),
        ability: strike(),
        target: c(),
    });
    h.assert_authority();
    seq += 1;
    h.drain(peer);

    // Failure budget: eleven terminal rejections in one instant; the
    // eleventh receipt is dropped while its outcome is retained.
    h.now += 10_000;
    let first_dropped = seq + 10;
    let tick = h.tick_now();
    for offset in 0..11 {
        let bytes = end_turn(h.version, epoch, seq + offset, tick + 1, 1);
        assert_eq!(
            h.host.ingest(peer, &bytes, h.now),
            Ok(IngestDisposition::Staged)
        );
    }
    let summary = h.host.pump(h.now).expect("pump");
    assert_eq!(summary.rejected, 11);
    assert_eq!(summary.dropped_replies, 1);
    let frames = h.drain(peer);
    assert_eq!(frames.len(), 10);
    assert_eq!(
        h.host.cached_status(peer, first_dropped),
        Ok(Some(rejected(RejectionCode::FutureTick)))
    );
    // Retrying at the same instant drops again; replenished time recovers
    // the retained receipt with its original processed tick.
    let retry = end_turn(h.version, epoch, first_dropped, tick + 1, 1);
    assert_eq!(
        h.host.ingest(peer, &retry, h.now),
        Ok(IngestDisposition::Staged)
    );
    let summary = h.host.pump(h.now).expect("pump");
    assert_eq!(summary.dropped_replies, 1);
    assert!(h.drain(peer).is_empty());
    h.host.tick().expect("tick");
    h.oracle.tick().expect("tick");
    h.now += 1_000;
    let outcome = h.send(peer, &retry);
    assert!(outcome.cached);
    let frames = h.drain(peer);
    assert_eq!(
        frames[0].ops,
        vec![receipt(
            epoch,
            first_dropped,
            tick,
            rejected(RejectionCode::FutureTick)
        )]
    );
    assert_eq!(frames[0].server_tick, tick + 1);
    h.assert_authority();

    // Reserve-before-commit for a terminal candidate under full egress.
    let seq = first_dropped + 1;
    let mut fill = 0;
    while h.host.take_delivery(peer).expect("bound").len() < 128 {
        let bytes = end_turn(h.version, epoch, seq + fill, h.tick_now() + 1, 1);
        h.now += 1_000;
        assert_eq!(
            h.send(peer, &bytes).status,
            rejected(RejectionCode::FutureTick)
        );
        fill += 1;
    }
    let bytes = end_turn(h.version, epoch, seq + fill, h.tick_now() + 1, 1);
    h.now += 1_000;
    let before = snapshot(&h.host);
    let outcome = h.send(peer, &bytes);
    assert!(outcome.retained_in_ingress);
    assert_eq!(outcome.status, rejected(RejectionCode::QueueFull));
    assert_eq!(snapshot(&h.host), before);
    assert_eq!(h.host.cached_status(peer, seq + fill), Ok(None));
    h.drain(peer);
    assert_eq!(
        h.pump().results,
        vec![result(
            peer,
            Some(seq + fill),
            rejected(RejectionCode::FutureTick)
        )]
    );
}
both_versions!(error_precedence_matrix);

impl H {
    fn restore_full_grants(&mut self, peer: PeerHandle) {
        self.host
            .set_grants(peer, full_grants())
            .expect("widening back");
        self.drain(peer);
    }
}

fn shutdown_and_pending_ingress(protocol: ProtocolSelection) {
    let mut h = H::new(protocol);
    let peer = h.bind(all_actors(), full_grants());
    let other = h.bind(control(&[]), observer_grants());
    let bytes = h.end_turn(peer, 1, 1);
    assert_eq!(h.send(peer, &bytes).status, APPLIED);
    h.mirror(CombatAction::EndTurn { actor: a() });
    let staged_one = h.end_turn(peer, 2, 2);
    let staged_two = h.end_turn(peer, 3, 3);
    h.ingest(peer, &staged_one);
    h.ingest(peer, &staged_two);
    assert_eq!(
        h.host.save_checkpoint(),
        Err(CheckpointError::PendingIngress)
    );
    let before = snapshot(&h.host);
    let refused = h.host.shutdown();
    let expired = result(peer, None, rejected(RejectionCode::SessionExpired));
    assert_eq!(refused, vec![expired.clone(), expired]);
    assert!(h.host.is_closed());
    assert_eq!(snapshot(&h.host), before);
    h.assert_authority();
    // Closed surfaces.
    assert_eq!(h.host.ingest(peer, &bytes, h.now), Err(HostError::Closed));
    assert_eq!(
        h.host.bind_peer(all_actors(), full_grants(), h.now),
        Err(HostError::Closed)
    );
    assert_eq!(
        h.host.set_grants(peer, full_grants()),
        Err(HostError::Closed)
    );
    assert_eq!(
        h.host.set_control(peer, all_actors()),
        Err(HostError::Closed)
    );
    assert_eq!(h.host.tick(), Err(HostError::Closed));
    assert_eq!(
        h.host.pump(h.now),
        Ok(PumpSummary {
            admitted: 0,
            applied: 0,
            rejected: 0,
            backpressured: 0,
            dropped_replies: 0,
            results: Vec::new(),
        })
    );
    assert_eq!(h.host.take_delivery(other), Err(HostError::UnknownPeer));
    assert_eq!(h.host.epoch(peer), Err(HostError::UnknownPeer));
    // Trusted capture reads/acks and save stay callable; shutdown is
    // idempotent.
    assert_eq!(h.host.read_captures(0, 1).expect("readable").len(), 1);
    h.host.acknowledge_captures(1).expect("acks");
    let saved = h.host.save_checkpoint().expect("post-fence save");
    assert!(h.host.shutdown().is_empty());
    let reloaded = load_checkpoint(&saved, 2, h.now).expect("loads");
    assert!(!reloaded.is_closed());
    assert_eq!(reloaded.authority_hash(), h.host.authority_hash());

    // unbind_peer fences one binding: its staged command is refused at the
    // next pump, never executed; the other peer is untouched.
    let mut h = H::new(protocol);
    let peer = h.bind(all_actors(), full_grants());
    let other = h.bind(all_actors(), full_grants());
    let bytes = h.end_turn(peer, 1, 1);
    h.ingest(peer, &bytes);
    h.host.unbind_peer(peer);
    h.host.unbind_peer(peer);
    let before = snapshot(&h.host);
    assert_eq!(
        h.pump().results,
        vec![result(
            peer,
            Some(1),
            rejected(RejectionCode::SessionExpired)
        )]
    );
    assert_eq!(snapshot(&h.host), before);
    let bytes = h.end_turn(other, 1, 1);
    assert_eq!(h.send(other, &bytes), result(other, Some(1), APPLIED));
}
both_versions!(shutdown_and_pending_ingress);

// ---------------------------------------------------------------------------
// Construction, binding validation, and checkpoint input validation.
// ---------------------------------------------------------------------------

#[test]
fn construction_and_binding_validation() {
    // from_history refuses uncaptured sim evidence.
    let config = HostConfig {
        protocol: V2,
        incarnation: 1,
    };
    assert!(matches!(
        Host::from_history(fixture_unacked(), config.clone(), 0),
        Err(HostError::PendingHistory)
    ));
    // An empty authority binds and stages no spurious state.
    let mut empty = Host::new(3, config.clone(), 10).expect("new");
    assert_eq!(empty.server_tick(), 0);
    let peer = empty
        .bind_peer(all_actors(), full_grants(), 10)
        .expect("binds");
    assert!(empty.take_delivery(peer).expect("bound").is_empty());
    assert_eq!(
        empty.bind_peer(all_actors(), full_grants(), 9),
        Err(HostError::TimeRegression)
    );

    let mut host = Host::from_history(fixture(), config, 0).expect("wraps");
    // Control bound is checked before dedup.
    let too_many = ControlGrant {
        actors: vec![a(); 1025],
    };
    assert_eq!(
        host.bind_peer(too_many, full_grants(), 0),
        Err(HostError::InvalidControl)
    );
    let dedup = ControlGrant {
        actors: vec![a(); 1024],
    };
    assert!(host.bind_peer(dedup, full_grants(), 0).is_ok());
    // Grant duplicates and bounds.
    let mut duplicate = full_grants();
    duplicate.entities.push(full(a()));
    assert_eq!(
        host.bind_peer(all_actors(), duplicate, 0),
        Err(HostError::InvalidGrants)
    );
    let mut duplicate = full_grants();
    duplicate.abilities.push(strike());
    assert_eq!(
        host.bind_peer(all_actors(), duplicate, 0),
        Err(HostError::InvalidGrants)
    );
    let oversized = DisclosureGrants {
        encounters: (0..1025).map(Ulid::from_u128).collect(),
        ..DisclosureGrants::default()
    };
    assert_eq!(
        host.bind_peer(all_actors(), oversized, 0),
        Err(HostError::InvalidGrants)
    );
    // Default grants deny everything: no state is staged.
    let blind = host
        .bind_peer(all_actors(), DisclosureGrants::default(), 0)
        .expect("binds");
    assert!(host.take_delivery(blind).expect("bound").is_empty());
    assert_eq!(
        host.set_grants(blind, DisclosureGrants::default()),
        Ok(GrantUpdate::Unchanged)
    );
    host.unbind_peer(blind);
    assert_eq!(
        host.set_grants(blind, full_grants()),
        Err(HostError::UnknownPeer)
    );
    assert_eq!(
        host.set_control(blind, all_actors()),
        Err(HostError::UnknownPeer)
    );
    assert_eq!(host.cached_status(blind, 1), Err(HostError::UnknownPeer));
    assert_eq!(
        host.acknowledge_delivery(blind, 0),
        Err(HostError::UnknownPeer)
    );
}

fn saved_checkpoint() -> (Vec<u8>, String) {
    let mut h = H::new(V2);
    let peer = h.bind(all_actors(), full_grants());
    let bytes = h.declare(peer, 1, 1, strike(), 2);
    assert_eq!(h.send(peer, &bytes).status, APPLIED);
    let bytes = h.end_turn(peer, 2, 2);
    assert_eq!(h.send(peer, &bytes).status, APPLIED);
    let saved = h.host.save_checkpoint().expect("save");
    let text = String::from_utf8(saved.clone()).expect("utf8");
    (saved, text)
}

#[test]
fn checkpoint_rejects_invalid_input() {
    let (saved, text) = saved_checkpoint();
    assert!(text.starts_with(&format!(
        "{{\"version\":{CHECKPOINT_VERSION},\"incarnation\":1,\"protocol\":2,\"server_tick\":0,\"history\":{{"
    )));
    assert!(text.ends_with(",\"capture_acknowledged\":0}"));
    assert!(load_checkpoint(&saved, 2, 0).is_ok());

    let load = |text: &str| load_checkpoint(text.as_bytes(), 2, 0).err();
    // Cap before parsing (bytes and reader entry points).
    let huge = vec![b' '; MAX_CHECKPOINT_BYTES + 1];
    assert_eq!(
        load_checkpoint(&huge, 2, 0).err(),
        Some(CheckpointError::TooLarge {
            len: MAX_CHECKPOINT_BYTES + 1
        })
    );
    assert_eq!(
        load_checkpoint_from_reader(std::io::repeat(b' '), 2, 0).err(),
        Some(CheckpointError::TooLarge {
            len: MAX_CHECKPOINT_BYTES + 1
        })
    );
    struct Failing;
    impl std::io::Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("boom"))
        }
    }
    assert_eq!(
        load_checkpoint_from_reader(Failing, 2, 0).err(),
        Some(CheckpointError::Io)
    );
    // Structure.
    assert_eq!(load("not json"), Some(CheckpointError::Malformed));
    assert_eq!(
        load(&format!("{text} ")),
        None,
        "trailing whitespace is fine"
    );
    assert_eq!(load(&format!("{text}x")), Some(CheckpointError::Malformed));
    assert_eq!(
        load(&text.replacen("{\"version\":1,", "{\"version\":1,\"extra\":0,", 1)),
        Some(CheckpointError::Malformed)
    );
    assert_eq!(
        load(&text.replacen("{\"version\":1,", "{\"version\":1,\"version\":1,", 1)),
        Some(CheckpointError::Malformed)
    );
    assert_eq!(
        load(&text.replacen("\"incarnation\":1,", "", 1)),
        Some(CheckpointError::Malformed)
    );
    assert_eq!(
        load(&text.replacen(",\"capture_acknowledged\":0}", "}", 1)),
        Some(CheckpointError::Malformed)
    );
    assert_eq!(
        load(&text.replacen("\"protocol\":2,", "\"protocol\":3,", 1)),
        Some(CheckpointError::Malformed)
    );
    // Version.
    assert_eq!(
        load(&text.replacen("{\"version\":1,", "{\"version\":2,", 1)),
        Some(CheckpointError::UnsupportedVersion { version: 2 })
    );
    // The wrapped authority validates itself.
    assert_eq!(
        load(&text.replacen(
            "\"history\":{\"version\":1,",
            "\"history\":{\"version\":9,",
            1
        )),
        Some(CheckpointError::AuthorityInvalid)
    );
    assert_eq!(
        load(&text.replacen(
            "\"history\":{\"version\":1,",
            "\"history\":{\"version\":1,\"bogus\":1,",
            1
        )),
        Some(CheckpointError::AuthorityInvalid)
    );
    // Consistency with the authority and the capture journal.
    assert_eq!(
        load(&text.replacen("\"server_tick\":0,", "\"server_tick\":1,", 1)),
        Some(CheckpointError::Malformed)
    );
    assert_eq!(
        load(&text.replacen(
            ",\"capture_acknowledged\":0}",
            ",\"capture_acknowledged\":1}",
            1
        )),
        Some(CheckpointError::Malformed)
    );
    assert_eq!(
        load(&text.replacen("\"capture_seq\":2,", "\"capture_seq\":3,", 1)),
        Some(CheckpointError::Malformed)
    );
    assert_eq!(
        load(&text.replacen("\"outcome\":\"success\"", "\"outcome\":\"custom:01\"", 1)),
        Some(CheckpointError::Malformed)
    );
    assert_eq!(
        load(&text.replacen("\"damage\":3}", "\"damage\":4}", 1)),
        Some(CheckpointError::Malformed)
    );
    assert_eq!(
        load(&text.replacen("\"history_end\":6,", "\"history_end\":7,", 1)),
        Some(CheckpointError::Malformed)
    );
    // Empty capture list with a matching watermark is valid.
    let start = text.find("\"captured\":[").expect("captured");
    let end = text.rfind("],\"capture_acknowledged\":0}").expect("tail");
    let emptied = format!(
        "{}\"captured\":[],\"capture_acknowledged\":2}}",
        &text[..start]
    );
    assert!(end > start);
    assert!(load_checkpoint(emptied.as_bytes(), 2, 0).is_ok());
    // Positive control for the incarnation rule.
    assert_eq!(
        load_checkpoint(&saved, 1, 0).err(),
        Some(CheckpointError::EpochReused)
    );
}
