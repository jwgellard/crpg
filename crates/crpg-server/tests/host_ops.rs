//! T141a acceptance suite (`tasks/T141.md`, tests A1 and A2 plus the A-1..A-4
//! branches they cover): host actions, the operator's mode request and
//! `awaiting_orders`, public API only. (The A-1 tail order for peer commands
//! is the unit test `command_tail_acknowledges_even_when_the_record_fails` in
//! `src/host.rs`; later parts add the pause, v3 and timer cases.)
//!
//! Oracles are independent of the host: a test-owned mirror `HistoryWorld`
//! loaded from the same JSON and driven with the same sim calls (acknowledged
//! after each), hand-written expected envelopes, and hand-edited checkpoint
//! text. The fixtures are `tests/fixtures/three_combatants.json` as saved-game
//! JSON, edited in memory:
//!
//! - `TB3`: the file as is (A, B, C act in keys 1, 2, 3; A first).
//! - `TB3c`: `TB3` at history version 2 with a round clock, so the encounter
//!   is real-time capable while turn-based.
//! - `RT3`: `TB3c` in real-time mode, no active head, every timeline key 0.
//!
//! Every refusal is compared against a `State` taken just before the call
//! (authority hash, capture counters, executions, server tick, the typed last
//! combat error, every capture entry, and every peer's transfer state and
//! delivery log), and each refusal sits beside an accepted control.

use crpg_core::{EntityId, Ulid};
use crpg_net::codec;
use crpg_net::codec_v2;
use crpg_net::protocol::{
    IntentBody, IntentFrame, NetId, ReceiptStatus, RejectionCode, LANE_COMBAT,
};
use crpg_server::capture::{
    CaptureEntry, HostOp, HostRecord, MAX_CAPTURE_PAGE, MAX_CAPTURE_RECORDS,
};
use crpg_server::checkpoint::load_checkpoint;
use crpg_server::host::{
    AdmissionResult, ControlGrant, DeliveryEntry, DisclosureGrants, EntityDisclosure, Host,
    HostConfig, HostError, IngestDisposition, PeerHandle, ProtocolSelection, TransferState,
};
use crpg_sim::{
    history_hash, ActionOutcome, CombatAction, CombatError, EncounterMode, HistoryEnvelope,
    HistoryEvent, HistoryWorld, RT_CLOCK_MAX,
};
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");

// ---------------------------------------------------------------------------
// Identities and fixtures.
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
fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}

fn tb3_value() -> Value {
    serde_json::from_str(FIXTURE).expect("fixture parses")
}

fn tb3c_value() -> Value {
    let mut value = tb3_value();
    value["version"] = json!(2);
    value["world"]["combat"]["clock"] = json!({"round_ticks": 4, "elapsed": 0});
    value
}

fn rt3_value() -> Value {
    let mut value = tb3c_value();
    value["world"]["combat"]["mode"] = json!("real_time");
    value["world"]["combat"]["active"] = Value::Null;
    for entry in value["world"]["timeline"].as_array_mut().expect("timeline") {
        entry[0] = json!(0);
    }
    value
}

/// `RT3` with explicit timeline keys for A, B, C and a clock value.
fn rt3_keyed(keys: [i64; 3], elapsed: u64) -> Value {
    let mut value = rt3_value();
    for (slot, key) in value["world"]["timeline"]
        .as_array_mut()
        .expect("timeline")
        .iter_mut()
        .zip(keys)
    {
        slot[0] = json!(key);
    }
    value["world"]["combat"]["clock"]["elapsed"] = json!(elapsed);
    value
}

/// The authority from `value`, its four start envelopes acknowledged.
fn history_of(value: &Value) -> HistoryWorld {
    let mut history: HistoryWorld =
        serde_json::from_value(value.clone()).expect("fixture loads through validated serde");
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

fn all_actors() -> ControlGrant {
    ControlGrant {
        actors: vec![a(), b(), c()],
    }
}

fn config(protocol: ProtocolSelection) -> HostConfig {
    HostConfig {
        protocol,
        incarnation: 1,
    }
}

fn action_resolved(
    actor: EntityId,
    target: EntityId,
    ability: Ulid,
    outcome: &str,
    damage: u32,
) -> HistoryEvent {
    HistoryEvent::ActionResolved {
        actor,
        target,
        ability,
        outcome: outcome.to_owned(),
        damage,
    }
}

fn payloads(envelopes: &[HistoryEnvelope]) -> Vec<HistoryEvent> {
    envelopes
        .iter()
        .map(|envelope| envelope.payload.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Rig: one host, its mirror authority and an optional bound peer `V`.
// ---------------------------------------------------------------------------

struct Rig {
    host: Host,
    oracle: HistoryWorld,
    now: u64,
    protocol: ProtocolSelection,
}

/// Everything a refusal must leave alone.
#[derive(Debug, PartialEq, Eq)]
struct State {
    authority: [u8; 32],
    last_capture: u64,
    capture_acknowledged: u64,
    executions: u64,
    server_tick: u64,
    last_combat_error: Option<CombatError>,
    entries: Vec<CaptureEntry>,
    peers: Vec<(Option<TransferState>, Vec<DeliveryEntry>)>,
}

impl Rig {
    fn new(value: &Value) -> Self {
        Self::with_protocol(value, ProtocolSelection::V2)
    }

    fn with_protocol(value: &Value, protocol: ProtocolSelection) -> Self {
        Self {
            host: Host::from_history(history_of(value), config(protocol), 0).expect("wraps"),
            oracle: history_of(value),
            now: 0,
            protocol,
        }
    }

    /// `V`: controls A, B and C, every disclosure flag, first state drained.
    fn bind_v(&mut self) -> PeerHandle {
        let peer = self
            .host
            .bind_peer(all_actors(), full_grants(), self.now)
            .expect("binds");
        self.drain(peer);
        peer
    }

    /// Takes and acknowledges every pending delivery frame of `peer`.
    fn drain(&mut self, peer: PeerHandle) -> Vec<Vec<u8>> {
        let frames = self.host.take_delivery(peer).expect("bound");
        let entries = self.host.take_delivery_entries(peer).expect("bound");
        assert_eq!(frames.len(), entries.len());
        if let Some(last) = entries.last() {
            self.host
                .acknowledge_delivery(peer, last.ordinal)
                .expect("acks");
        }
        frames
    }

    /// One intent through `ingest` then `pump`, +100 ms.
    fn send(
        &mut self,
        peer: PeerHandle,
        seq: u64,
        actor: u64,
        body: IntentBody,
    ) -> AdmissionResult {
        let frame = IntentFrame {
            epoch: self.host.epoch(peer).expect("bound"),
            lane: LANE_COMBAT,
            seq,
            observed_tick: self.host.server_tick(),
            actor: n(actor),
            body,
        };
        let bytes = match self.protocol {
            ProtocolSelection::V1 => codec::encode_intent(&frame),
            ProtocolSelection::V2 => codec_v2::encode_intent(&frame),
        }
        .expect("encodes");
        self.now += 100;
        assert_eq!(
            self.host.ingest(peer, &bytes, self.now),
            Ok(IngestDisposition::Staged)
        );
        let summary = self.host.pump(self.now).expect("pump");
        assert_eq!(summary.results.len(), 1, "{summary:?}");
        summary.results[0].clone()
    }

    fn end_turn(&mut self, peer: PeerHandle, seq: u64, actor: u64) -> AdmissionResult {
        self.send(peer, seq, actor, IntentBody::EndTurn)
    }

    /// Mirrors one accepted action; returns the envelopes it journaled.
    fn mirror(&mut self, action: CombatAction) -> Vec<HistoryEnvelope> {
        let before = self.oracle.last_sequence();
        self.oracle
            .perform_action(&action)
            .expect("the mirror accepts the same action");
        self.mirror_page(before)
    }

    /// Mirrors one tick; returns the envelopes it journaled.
    fn mirror_tick(&mut self) -> Vec<HistoryEnvelope> {
        let before = self.oracle.last_sequence();
        self.oracle.tick().expect("the mirror ticks");
        self.mirror_page(before)
    }

    /// As [`mirror`](Self::mirror), also returning the sim's outcome.
    fn mirror_outcome(
        &mut self,
        action: CombatAction,
    ) -> (Option<ActionOutcome>, Vec<HistoryEnvelope>) {
        let before = self.oracle.last_sequence();
        let outcome = self
            .oracle
            .perform_action(&action)
            .expect("the mirror accepts the same action");
        (outcome, self.mirror_page(before))
    }

    fn mirror_mode(&mut self, mode: EncounterMode) {
        self.oracle
            .request_mode(mode)
            .expect("the mirror accepts the same request");
    }

    fn mirror_page(&mut self, before: u64) -> Vec<HistoryEnvelope> {
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

    fn entries(&self) -> Vec<CaptureEntry> {
        let mut all = Vec::new();
        let mut after = self.host.capture_acknowledged();
        loop {
            let page = self
                .host
                .read_capture_entries(after, MAX_CAPTURE_PAGE)
                .expect("readable");
            if page.is_empty() {
                return all;
            }
            after = page.last().expect("non-empty").capture_seq();
            all.extend(page);
        }
    }

    /// The state a refusal must leave untouched; `peers` are the bound
    /// handles to include.
    fn state(&mut self, peers: &[PeerHandle]) -> State {
        let entries = self.entries();
        let peers = peers
            .iter()
            .map(|peer| {
                (
                    self.host.transfer_state(*peer).expect("bound"),
                    self.host.take_delivery_entries(*peer).expect("bound"),
                )
            })
            .collect();
        State {
            authority: self.host.authority_hash(),
            last_capture: self.host.last_capture(),
            capture_acknowledged: self.host.capture_acknowledged(),
            executions: self.host.executions(),
            server_tick: self.host.server_tick(),
            last_combat_error: self.host.last_combat_error(),
            entries,
            peers,
        }
    }

    /// `host_action` is refused with `error`, and nothing changed.
    fn assert_action_refused(
        &mut self,
        peers: &[PeerHandle],
        action: CombatAction,
        error: HostError,
    ) {
        let before = self.state(peers);
        assert_eq!(self.host.host_action(&action), Err(error));
        assert_eq!(self.state(peers), before);
        self.assert_authority();
    }

    /// `request_mode` is refused with `error`, and nothing changed.
    fn assert_mode_refused(&mut self, peers: &[PeerHandle], mode: EncounterMode, error: HostError) {
        let before = self.state(peers);
        assert_eq!(self.host.request_mode(mode), Err(error));
        assert_eq!(self.state(peers), before);
        self.assert_authority();
    }

    fn save(&self) -> String {
        String::from_utf8(self.host.save_checkpoint().expect("saves")).expect("utf8")
    }
}

fn host_record(entry: &CaptureEntry) -> &HostRecord {
    match entry {
        CaptureEntry::Host(record) => record,
        CaptureEntry::Command(record) => panic!("expected a host entry, got {record:?}"),
    }
}

fn use_ability(actor: EntityId, ability: Ulid, target: EntityId) -> CombatAction {
    CombatAction::UseAbility {
        actor,
        ability,
        target,
    }
}

fn end(actor: EntityId) -> CombatAction {
    CombatAction::EndTurn { actor }
}

// ---------------------------------------------------------------------------
// A1.
// ---------------------------------------------------------------------------

/// A host action is validated, journaled and recorded as a host entry, never
/// counted as a command; a refused one changes nothing; the operator's mode
/// request journals nothing; a closed host refuses all three.
#[test]
fn host_action_and_operator_mode() {
    // `TB3c` with `V`.
    let mut rig = Rig::new(&tb3c_value());
    let v = rig.bind_v();

    // EndTurn for B is out of turn: refused, nothing changes.
    rig.assert_action_refused(
        &[v],
        end(b()),
        HostError::Combat(CombatError::OutOfTurn {
            actor: b(),
            active: Some(a()),
        }),
    );
    assert_eq!(rig.host.last_capture(), 0);

    // Control: EndTurn for A is accepted, journals one envelope, returns no
    // outcome, and is entry 1 as a host record.
    let outcome = rig.host.host_action(&end(a()));
    assert_eq!(outcome, Ok(None));
    let mirrored = rig.mirror(end(a()));
    assert_eq!(
        payloads(&mirrored),
        vec![HistoryEvent::TurnStarted {
            actor: b(),
            round: 0
        }]
    );
    let entries = rig.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        host_record(&entries[0]),
        &HostRecord {
            capture_seq: 1,
            op: HostOp::Action,
            history_start: 4,
            history_end: 5,
            processed_tick: 0,
            events: mirrored,
        }
    );
    assert_eq!(rig.host.last_capture(), 1);
    assert_eq!(rig.host.executions(), 0, "a host action is not a command");
    assert_eq!(rig.host.last_combat_error(), None);
    assert!(
        rig.drain(v).is_empty(),
        "a host action sends a v2 session nothing"
    );
    rig.assert_authority();

    // The operator's mode request: no new entry, authority moved (pending
    // mode), mirrored.
    let before = rig.host.authority_hash();
    assert_eq!(rig.host.request_mode(EncounterMode::RealTime), Ok(()));
    rig.mirror_mode(EncounterMode::RealTime);
    assert_ne!(rig.host.authority_hash(), before, "the request is staged");
    assert_eq!(rig.host.last_capture(), 1);
    assert_eq!(rig.entries().len(), 1);
    assert_eq!(rig.host.executions(), 0);
    assert!(rig.drain(v).is_empty());
    rig.assert_authority();

    // `RT3`: an order is a zero-event action: Ok(None), no entry, but the
    // authority holds the order.
    let mut rt = Rig::new(&rt3_value());
    let before = rt.host.authority_hash();
    assert_eq!(
        rt.host.host_action(&use_ability(c(), whiff(), a())),
        Ok(None)
    );
    assert_eq!(
        rt.mirror(use_ability(c(), whiff(), a())),
        Vec::new(),
        "an RT order journals nothing"
    );
    assert_ne!(rt.host.authority_hash(), before, "the order is set");
    assert_eq!(rt.host.last_capture(), 0);
    assert!(rt.entries().is_empty());
    assert_eq!(rt.host.executions(), 0);
    assert_eq!(rt.host.awaiting_orders(), vec![a(), b()]);
    rt.assert_authority();
    // Its save is still a version-1 checkpoint: nothing was recorded.
    assert!(rt.save().starts_with("{\"version\":1,"));

    // `TB3`: a turn-based-only encounter refuses a mode request; the same
    // request on `TB3c` (above) is accepted.
    let mut tb = Rig::new(&tb3_value());
    tb.assert_mode_refused(
        &[],
        EncounterMode::RealTime,
        HostError::Combat(CombatError::ModeUnsupported),
    );
    // Requesting the mode already in force is accepted on the same encounter.
    assert_eq!(tb.host.request_mode(EncounterMode::TurnBased), Ok(()));
    tb.mirror_mode(EncounterMode::TurnBased);
    tb.assert_authority();

    // No encounter at all.
    let mut empty = Host::new(7, config(ProtocolSelection::V2), 0).expect("new");
    let empty_hash = empty.authority_hash();
    assert_eq!(
        empty.request_mode(EncounterMode::TurnBased),
        Err(HostError::Combat(CombatError::NoEncounter))
    );
    assert_eq!(
        empty.host_action(&end(a())),
        Err(HostError::Combat(CombatError::NoEncounter))
    );
    assert_eq!(empty.authority_hash(), empty_hash);
    assert_eq!(empty.last_capture(), 0);
    assert_eq!(
        HostError::Combat(CombatError::NoEncounter).to_string(),
        "Combat at host/admission"
    );
    assert!(std::error::Error::source(&HostError::Combat(CombatError::NoEncounter)).is_none());

    // A closed host refuses all three, where a live one accepted each.
    let mut closing = Rig::new(&tb3_value());
    assert_eq!(closing.host.tick(), Ok(()));
    assert_eq!(closing.host.host_action(&end(a())), Ok(None));
    assert_eq!(closing.host.request_mode(EncounterMode::TurnBased), Ok(()));
    let last = closing.host.last_capture();
    let hash = closing.host.authority_hash();
    assert_eq!(closing.host.shutdown(), Vec::new());
    assert_eq!(closing.host.tick(), Err(HostError::Closed));
    assert_eq!(closing.host.host_action(&end(b())), Err(HostError::Closed));
    assert_eq!(
        closing.host.request_mode(EncounterMode::TurnBased),
        Err(HostError::Closed)
    );
    assert_eq!(closing.host.last_capture(), last);
    assert_eq!(closing.host.authority_hash(), hash);
}

/// A host ability use returns the sim's outcome and records every envelope it
/// journaled, in journal order, as one host entry.
#[test]
fn host_action_returns_the_outcome_and_records_its_events() {
    let mut rig = Rig::new(&tb3_value());
    let action = use_ability(a(), strike(), b());
    let (expected, mirrored) = rig.mirror_outcome(action);
    let outcome = rig.host.host_action(&action).expect("accepted");
    assert_eq!(outcome, expected);
    let outcome = outcome.expect("an ability use has an outcome");
    assert_eq!(
        (
            outcome.actor,
            outcome.target,
            outcome.ability,
            outcome.damage,
            outcome.target_died
        ),
        (a(), b(), strike(), 3, false)
    );
    assert_eq!(
        payloads(&mirrored),
        vec![
            action_resolved(a(), b(), strike(), "success", 3),
            HistoryEvent::TurnStarted {
                actor: b(),
                round: 0
            },
        ]
    );
    let entries = rig.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        host_record(&entries[0]),
        &HostRecord {
            capture_seq: 1,
            op: HostOp::Action,
            history_start: 4,
            history_end: 6,
            processed_tick: 0,
            events: mirrored,
        }
    );
    assert_eq!(rig.host.executions(), 0);
    assert_eq!(rig.host.last_combat_error(), None);
    rig.assert_authority();
}

/// First failure wins for `host_action`, at every adjacent pair: Closed, then
/// CaptureFull, then the sim's refusal. (The last step, the sim refusing a
/// validated action as `Invariant`, cannot be built through the public API:
/// `validate_action` runs the same checks; `src/host.rs` has a unit test.)
#[test]
fn host_action_precedence() {
    let mut rig = Rig::new(&tb3_value());
    let v = rig.bind_v();
    // Fill the journal with 4,096 peer commands cycling A, B, C: the head is
    // then B (4,096 = 1 mod 3).
    for seq in 1..=MAX_CAPTURE_RECORDS as u64 {
        let actor = (seq - 1) % 3;
        assert_eq!(
            rig.end_turn(v, seq, actor + 1).status,
            ReceiptStatus::Applied,
            "seq {seq}"
        );
        rig.mirror(end(eid(actor as u32)));
        rig.drain(v);
    }
    assert_eq!(rig.host.last_capture(), MAX_CAPTURE_RECORDS as u64);
    assert_eq!(rig.host.executions(), MAX_CAPTURE_RECORDS as u64);
    rig.assert_authority();

    // (CaptureFull, Combat): both present, CaptureFull wins; and a valid
    // action is CaptureFull too.
    let invalid = end(a());
    let valid = end(b());
    rig.assert_action_refused(&[v], invalid, HostError::CaptureFull);
    rig.assert_action_refused(&[v], valid, HostError::CaptureFull);

    // Room for one entry: the sim's refusal is now what the invalid action
    // gets, and the valid one is accepted (the control for both).
    rig.host.acknowledge_captures(1).expect("acks");
    rig.assert_action_refused(
        &[v],
        invalid,
        HostError::Combat(CombatError::OutOfTurn {
            actor: a(),
            active: Some(b()),
        }),
    );
    assert_eq!(rig.host.host_action(&valid), Ok(None));
    let mirrored = rig.mirror(valid);
    assert_eq!(
        payloads(&mirrored),
        vec![HistoryEvent::TurnStarted {
            actor: c(),
            round: 1365
        }]
    );
    assert_eq!(rig.host.last_capture(), MAX_CAPTURE_RECORDS as u64 + 1);
    let last = rig
        .host
        .read_capture_entries(MAX_CAPTURE_RECORDS as u64, 1)
        .expect("reads")
        .pop()
        .expect("one entry");
    let record = host_record(&last);
    assert_eq!(record.op, HostOp::Action);
    assert_eq!(record.events, mirrored);
    assert_eq!(rig.host.executions(), MAX_CAPTURE_RECORDS as u64);
    rig.assert_authority();

    // The journal is full again (4,095 commands and this record). The head
    // is C: the next valid and invalid actions, as before.
    rig.assert_action_refused(&[v], end(c()), HostError::CaptureFull);

    // (Closed, CaptureFull): shut down with the journal full, an invalid and
    // a valid action are both Closed, as are the other two operations.
    assert_eq!(rig.host.shutdown().len(), 0);
    let hash = rig.host.authority_hash();
    assert_eq!(rig.host.host_action(&invalid), Err(HostError::Closed));
    assert_eq!(rig.host.host_action(&end(c())), Err(HostError::Closed));
    assert_eq!(
        rig.host.request_mode(EncounterMode::TurnBased),
        Err(HostError::Closed)
    );
    assert_eq!(rig.host.tick(), Err(HostError::Closed));
    assert_eq!(rig.host.authority_hash(), hash);
    assert_eq!(rig.host.last_capture(), MAX_CAPTURE_RECORDS as u64 + 1);
}

/// `request_mode` precedence: Closed beats the sim's refusal.
#[test]
fn request_mode_precedence() {
    let mut host = Host::new(7, config(ProtocolSelection::V2), 0).expect("new");
    // Control: live and refused by the sim.
    assert_eq!(
        host.request_mode(EncounterMode::RealTime),
        Err(HostError::Combat(CombatError::NoEncounter))
    );
    // Both faults present: closed wins.
    assert_eq!(host.shutdown(), Vec::new());
    assert_eq!(
        host.request_mode(EncounterMode::RealTime),
        Err(HostError::Closed)
    );
}

/// A mode request is real: the switch applies at the next round boundary,
/// and the events it causes are host-recorded (`ModeChanged`, `RoundStarted`).
#[test]
fn request_mode_applies_at_the_next_boundary() {
    let mut rig = Rig::new(&tb3c_value());
    assert_eq!(rig.host.request_mode(EncounterMode::RealTime), Ok(()));
    rig.mirror_mode(EncounterMode::RealTime);
    assert!(rig.entries().is_empty(), "the request journals nothing");
    // A and B end their turns: no boundary yet, one turn start each.
    for actor in [a(), b()] {
        assert_eq!(rig.host.host_action(&end(actor)), Ok(None));
        rig.mirror(end(actor));
    }
    assert_eq!(rig.host.last_capture(), 2);
    // C ends the last turn of the round: the pending switch applies.
    assert_eq!(rig.host.host_action(&end(c())), Ok(None));
    let mirrored = rig.mirror(end(c()));
    assert_eq!(
        payloads(&mirrored),
        vec![
            HistoryEvent::ModeChanged {
                mode: EncounterMode::RealTime,
                round: 1,
            },
            HistoryEvent::RoundStarted { round: 1 },
        ]
    );
    let entries = rig.entries();
    assert_eq!(entries.len(), 3);
    let record = host_record(&entries[2]);
    assert_eq!(record.capture_seq, 3);
    assert_eq!(record.op, HostOp::Action);
    assert_eq!((record.history_start, record.history_end), (6, 8));
    assert_eq!(record.events, mirrored);
    rig.assert_authority();
    // The encounter is now real-time: an EndTurn is refused by the sim (the
    // refusal is a plain `Combat` error and changes nothing), and the RT
    // ticks record as `Tick`, not `Action`.
    let before = rig.state(&[]);
    assert!(matches!(
        rig.host.host_action(&end(a())),
        Err(HostError::Combat(CombatError::WrongMode { .. }))
    ));
    assert_eq!(rig.state(&[]), before);
}

/// Host actions are not commands: no sequence consumed, no executions, no
/// `last_combat_error`, and a v1 session gets no frame; the peer's own
/// commands still run and still deliver.
#[test]
fn host_action_is_not_a_command() {
    for protocol in [ProtocolSelection::V1, ProtocolSelection::V2] {
        let mut rig = Rig::with_protocol(&tb3c_value(), protocol);
        let v = rig.bind_v();
        // A peer command the sim refuses sets the typed last error and burns
        // sequence 1.
        let result = rig.end_turn(v, 1, 2);
        assert_eq!(
            result.status,
            ReceiptStatus::Rejected(RejectionCode::IllegalAction)
        );
        let error = rig.host.last_combat_error();
        assert_eq!(
            error,
            Some(CombatError::OutOfTurn {
                actor: b(),
                active: Some(a()),
            })
        );
        rig.drain(v);

        // A refused host action leaves that error as it was; an accepted
        // one does too.
        rig.assert_action_refused(
            &[v],
            end(c()),
            HostError::Combat(CombatError::OutOfTurn {
                actor: c(),
                active: Some(a()),
            }),
        );
        assert_eq!(rig.host.host_action(&end(a())), Ok(None));
        rig.mirror(end(a()));
        assert_eq!(rig.host.last_combat_error(), error);
        assert_eq!(rig.host.executions(), 0);
        assert!(
            rig.drain(v).is_empty(),
            "{protocol:?}: a host action delivers nothing to a v1/v2 session"
        );
        rig.assert_authority();

        // The peer's next sequence is unaffected, and its command delivers
        // (the control for "delivers nothing").
        let result = rig.end_turn(v, 2, 2);
        assert_eq!(result.status, ReceiptStatus::Applied, "{protocol:?}");
        rig.mirror(end(b()));
        assert_eq!(rig.host.executions(), 1);
        assert!(
            !rig.drain(v).is_empty(),
            "{protocol:?}: the command delivers"
        );
        // Entry 1 is the host record; entry 2 the peer's command.
        let entries = rig.entries();
        assert!(matches!(
            entries.as_slice(),
            [CaptureEntry::Host(_), CaptureEntry::Command(_)]
        ));
        assert_eq!(entries[0].capture_seq(), 1);
        assert_eq!(entries[1].capture_seq(), 2);
        rig.assert_authority();
    }
}

/// A recorded host action is evidence: a checkpoint holds it as a version-2
/// host record and loads; a zero-event action or none leaves version 1.
#[test]
fn host_action_records_replay_in_the_checkpoint() {
    let mut rig = Rig::new(&tb3_value());
    assert!(rig.save().starts_with("{\"version\":1,"));
    assert_eq!(rig.host.host_action(&end(a())), Ok(None));
    let mirrored = rig.mirror(end(a()));
    let text = rig.save();
    assert!(
        text.starts_with("{\"version\":2,\"incarnation\":1,"),
        "{text}"
    );
    let prefix = "\"host_records\":[{\"capture_seq\":1,\"op\":\"action\",\"history_start\":4,\"history_end\":5,\"processed_tick\":0,\"events\":[";
    assert!(text.contains(prefix), "{text}");
    let restored = load_checkpoint(text.as_bytes(), 2, 0).expect("loads");
    assert_eq!(restored.authority_hash(), rig.host.authority_hash());
    assert_eq!(restored.last_capture(), 1);
    let entries = restored
        .read_capture_entries(0, MAX_CAPTURE_PAGE)
        .expect("reads");
    assert_eq!(entries.len(), 1);
    assert_eq!(host_record(&entries[0]).events, mirrored);

    // Acknowledged, nothing is retained and the save is version 1 again.
    rig.host.acknowledge_captures(1).expect("acks");
    assert!(rig.save().starts_with("{\"version\":1,"));
    rig.assert_authority();
}

// ---------------------------------------------------------------------------
// A2.
// ---------------------------------------------------------------------------

/// The turn-based head, the real-time due-and-unordered set, and a terminal
/// real-time tick whose host record holds five envelopes.
#[test]
fn awaiting_orders_and_terminal_tick() {
    // Turn-based: the head.
    let mut tb = Rig::new(&tb3_value());
    assert_eq!(tb.host.awaiting_orders(), vec![a()]);
    assert_eq!(tb.host.host_action(&end(a())), Ok(None));
    tb.mirror(end(a()));
    assert_eq!(tb.host.awaiting_orders(), vec![b()]);
    assert_eq!(tb.host.host_action(&end(b())), Ok(None));
    tb.mirror(end(b()));
    assert_eq!(tb.host.awaiting_orders(), vec![c()]);

    // Real-time.
    let mut rig = Rig::new(&rt3_value());
    assert_eq!(rig.host.awaiting_orders(), vec![a(), b(), c()]);
    assert_eq!(
        rig.host.host_action(&use_ability(a(), smite(), b())),
        Ok(None)
    );
    assert_eq!(rig.mirror(use_ability(a(), smite(), b())), Vec::new());
    assert_eq!(rig.host.awaiting_orders(), vec![b(), c()]);
    assert_eq!(
        rig.host.host_action(&use_ability(c(), smite(), a())),
        Ok(None)
    );
    assert_eq!(rig.mirror(use_ability(c(), smite(), a())), Vec::new());
    assert_eq!(rig.host.awaiting_orders(), vec![b()]);
    assert!(rig.entries().is_empty(), "orders journal nothing");

    // The tick executes both orders, kills both targets and ends the fight:
    // five envelopes in one host record (no per-command bound).
    assert_eq!(rig.host.tick(), Ok(()));
    let mirrored = rig.mirror_tick();
    let expected = vec![
        action_resolved(a(), b(), smite(), "critical_success", 4_000_000_000),
        HistoryEvent::Died { entity: b() },
        action_resolved(c(), a(), smite(), "critical_success", 4_000_000_000),
        HistoryEvent::Died { entity: a() },
        HistoryEvent::EncounterEnded {
            encounter: encounter(),
            round: 0,
        },
    ];
    assert_eq!(payloads(&mirrored), expected);
    let entries = rig.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        host_record(&entries[0]),
        &HostRecord {
            capture_seq: 1,
            op: HostOp::Tick,
            history_start: 4,
            history_end: 9,
            processed_tick: 1,
            events: mirrored,
        }
    );
    assert_eq!(rig.host.executions(), 0);
    // One combatant is alive: nobody is awaited, although C has a due entry
    // and no order any more.
    assert_eq!(rig.host.awaiting_orders(), Vec::<EntityId>::new());
    rig.assert_authority();

    // A host with no encounter.
    let host = Host::new(7, config(ProtocolSelection::V2), 0).expect("new");
    assert_eq!(host.awaiting_orders(), Vec::<EntityId>::new());
}

/// Every branch of `awaiting_orders`: ordering, due test (inclusive, negative
/// keys), the clock maximum, the two-alive floor, dead and ordered
/// combatants, terminal turn-based play, and a closed host.
#[test]
fn awaiting_orders_branches() {
    // Timeline order is (key, entity), not entity order.
    let rig = Rig::new(&rt3_keyed([2, 1, 1], 2));
    assert_eq!(rig.host.awaiting_orders(), vec![b(), c(), a()]);

    // Due is `key <= elapsed`: a key equal to the clock is due, one above is
    // not (the sim refuses negative real-time keys, so none is tested).
    let rig = Rig::new(&rt3_keyed([0, 0, 3], 3));
    assert_eq!(rig.host.awaiting_orders(), vec![a(), b(), c()]);
    let rig = Rig::new(&rt3_keyed([0, 0, 4], 3));
    assert_eq!(rig.host.awaiting_orders(), vec![a(), b()]);

    // The clock freezes at its maximum: nobody is awaited there, but one
    // tick below it all are.
    let rig = Rig::new(&rt3_keyed([0, 0, 0], RT_CLOCK_MAX - 1));
    assert_eq!(rig.host.awaiting_orders(), vec![a(), b(), c()]);
    let rig = Rig::new(&rt3_keyed([0, 0, 0], RT_CLOCK_MAX));
    assert_eq!(rig.host.awaiting_orders(), Vec::<EntityId>::new());

    // Two alive is enough: B dies of A's `smite`, the fight goes on with A
    // and C.
    let mut rig = Rig::new(&rt3_value());
    assert_eq!(
        rig.host.host_action(&use_ability(a(), smite(), b())),
        Ok(None)
    );
    rig.mirror(use_ability(a(), smite(), b()));
    assert_eq!(rig.host.tick(), Ok(()));
    let mirrored = rig.mirror_tick();
    assert_eq!(
        payloads(&mirrored),
        vec![
            action_resolved(a(), b(), smite(), "critical_success", 4_000_000_000),
            HistoryEvent::Died { entity: b() },
        ]
    );
    // B is dead and gone from the timeline; A's order ran and A is
    // rescheduled at key 5, so only C is due (A and C are the two alive).
    assert_eq!(rig.host.awaiting_orders(), vec![c()]);
    for tick in 2..=4u64 {
        assert_eq!(rig.host.tick(), Ok(()));
        let mirrored = rig.mirror_tick();
        if tick == 4 {
            assert_eq!(
                payloads(&mirrored),
                vec![HistoryEvent::RoundStarted { round: 1 }]
            );
        } else {
            assert!(mirrored.is_empty());
        }
        assert_eq!(rig.host.awaiting_orders(), vec![c()], "tick {tick}");
    }
    // At elapsed 5 A's key (5) is due (`key <= elapsed`, inclusive), listed
    // after C's key 0.
    assert_eq!(rig.host.tick(), Ok(()));
    assert!(rig.mirror_tick().is_empty());
    assert_eq!(rig.host.server_tick(), 5);
    assert_eq!(rig.host.awaiting_orders(), vec![c(), a()]);
    // An order takes a combatant out of the list again.
    assert_eq!(
        rig.host.host_action(&use_ability(c(), whiff(), a())),
        Ok(None)
    );
    rig.mirror(use_ability(c(), whiff(), a()));
    assert_eq!(rig.host.awaiting_orders(), vec![a()]);
    rig.assert_authority();

    // Turn-based terminal: no head, nobody awaited. A kills B, C kills A.
    let mut tb = Rig::new(&tb3_value());
    let (expected, _) = tb.mirror_outcome(use_ability(a(), smite(), b()));
    let outcome = tb
        .host
        .host_action(&use_ability(a(), smite(), b()))
        .expect("accepted");
    assert_eq!(outcome, expected);
    assert_eq!(tb.host.awaiting_orders(), vec![c()]);
    let (expected, mirrored) = tb.mirror_outcome(use_ability(c(), smite(), a()));
    let outcome = tb
        .host
        .host_action(&use_ability(c(), smite(), a()))
        .expect("accepted");
    assert_eq!(outcome, expected);
    assert!(mirrored
        .iter()
        .any(|envelope| matches!(envelope.payload, HistoryEvent::EncounterEnded { .. })));
    assert_eq!(tb.host.awaiting_orders(), Vec::<EntityId>::new());
    tb.assert_authority();

    // A closed host awaits nobody where a live one did.
    let mut rig = Rig::new(&rt3_value());
    assert_eq!(rig.host.awaiting_orders(), vec![a(), b(), c()]);
    assert_eq!(rig.host.shutdown(), Vec::new());
    assert_eq!(rig.host.awaiting_orders(), Vec::<EntityId>::new());
    let mut tb = Rig::new(&tb3_value());
    assert_eq!(tb.host.awaiting_orders(), vec![a()]);
    assert_eq!(tb.host.shutdown(), Vec::new());
    assert_eq!(tb.host.awaiting_orders(), Vec::<EntityId>::new());
}
