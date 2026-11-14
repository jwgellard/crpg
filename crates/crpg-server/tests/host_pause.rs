//! T141b acceptance suite (`tasks/T141.md`, tests B1 to B6, B8 and B9 plus the
//! B-1..B-9 branches they cover): operator settings, pause and unpause,
//! auto-pause, the pause-aware tick driver `advance` and the turn timer,
//! public API only. (The timer epoch and the "other error disarms" branch of
//! B-8 are unit tests in `src/host.rs`: neither is reachable through the
//! public API.)
//!
//! Oracles are independent of the host: a test-owned mirror `HistoryWorld`
//! loaded from the same JSON and driven with the same sim calls (acknowledged
//! after each), hand-written expected envelopes and hand-written checkpoint
//! text. The fixtures are `tests/fixtures/three_combatants.json` as saved-game
//! JSON, edited in memory:
//!
//! - `TB3`: the file as is (A, B, C act in keys 1, 2, 3; A first).
//! - `TB3c`: `TB3` at history version 2 with a round clock, so the encounter
//!   is real-time capable while turn-based.
//! - `RT3`: `TB3c` in real-time mode, no active head, every timeline key 0.
//!
//! Time is explicit: every call passes its own `now_ms`, at or above the
//! last accepted one. Every refusal is compared against a `State` taken just
//! before the call and sits beside an accepted control.

use crpg_core::{EntityId, Ulid};
use crpg_net::codec_v2;
use crpg_net::protocol::{IntentBody, IntentFrame, NetId, ReceiptStatus, LANE_COMBAT};
use crpg_net::protocol_v3::{AutoPauseToggles, AutoPauseTrigger, PauseReason};
use crpg_server::capture::{
    CaptureEntry, HostOp, HostRecord, MAX_CAPTURE_PAGE, MAX_CAPTURE_RECORDS,
};
use crpg_server::checkpoint::load_checkpoint;
use crpg_server::host::{
    AdmissionResult, AdvanceReport, ControlGrant, DeliveryEntry, DisclosureGrants,
    EntityDisclosure, Host, HostConfig, HostError, HostSettings, IngestDisposition, PeerHandle,
    ProtocolSelection, TransferState, SINGLE_PLAYER_AUTO_PAUSE_BITS,
};
use crpg_sim::{
    history_hash, CombatAction, EncounterMode, HistoryEnvelope, HistoryEvent, HistoryWorld,
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
        encounters: vec![Ulid::from_u128(601)],
        turn_round: true,
        encounter_end: true,
        turn_state: true,
    }
}

fn control(actors: &[EntityId]) -> ControlGrant {
    ControlGrant {
        actors: actors.to_vec(),
    }
}

fn config() -> HostConfig {
    HostConfig {
        protocol: ProtocolSelection::V2,
        incarnation: 1,
    }
}

fn toggles(bits: u8) -> AutoPauseToggles {
    AutoPauseToggles::from_bits(bits).expect("defined bits")
}

/// Multiplayer settings with the given auto-pause bits and timer length.
fn settings(bits: u8, timer_ms: Option<u32>) -> HostSettings {
    HostSettings {
        initial_mode: EncounterMode::TurnBased,
        auto_pause: toggles(bits),
        turn_timer_ms: timer_ms,
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

fn turn_started(actor: EntityId, round: u64) -> HistoryEvent {
    HistoryEvent::TurnStarted { actor, round }
}

fn payloads(envelopes: &[HistoryEnvelope]) -> Vec<HistoryEvent> {
    envelopes
        .iter()
        .map(|envelope| envelope.payload.clone())
        .collect()
}

fn end(actor: EntityId) -> CombatAction {
    CombatAction::EndTurn { actor }
}

fn use_ability(actor: EntityId, ability: Ulid, target: EntityId) -> CombatAction {
    CombatAction::UseAbility {
        actor,
        ability,
        target,
    }
}

fn host_record(entry: &CaptureEntry) -> &HostRecord {
    match entry {
        CaptureEntry::Host(record) => record,
        CaptureEntry::Command(record) => panic!("expected a host entry, got {record:?}"),
    }
}

fn report(
    ticked: bool,
    timer_ended_turn: Option<EntityId>,
    paused: Option<PauseReason>,
) -> AdvanceReport {
    AdvanceReport {
        ticked,
        timer_ended_turn,
        paused,
    }
}

// ---------------------------------------------------------------------------
// Rig: one host, its mirror authority and optional bound peers.
// ---------------------------------------------------------------------------

struct Rig {
    host: Host,
    oracle: HistoryWorld,
}

/// Everything a refusal must leave alone.
#[derive(Debug, PartialEq, Eq)]
struct State {
    authority: [u8; 32],
    last_capture: u64,
    capture_acknowledged: u64,
    executions: u64,
    server_tick: u64,
    settings: HostSettings,
    paused: Option<PauseReason>,
    remaining: Option<u64>,
    entries: Vec<CaptureEntry>,
    peers: Vec<(Option<TransferState>, Vec<DeliveryEntry>)>,
}

impl Rig {
    fn new(value: &Value) -> Self {
        Self {
            host: Host::from_history(history_of(value), config(), 0).expect("wraps"),
            oracle: history_of(value),
        }
    }

    /// A peer controlling `actors` with every disclosure flag, bound at
    /// `now`, its first state drained.
    fn bind(&mut self, actors: &[EntityId], now: u64) -> PeerHandle {
        let peer = self
            .host
            .bind_peer(control(actors), full_grants(), now)
            .expect("binds");
        self.drain(peer);
        peer
    }

    /// `V`: controls A, B and C.
    fn bind_v(&mut self, now: u64) -> PeerHandle {
        self.bind(&[a(), b(), c()], now)
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

    /// One intent through `ingest` then `pump` at `now`.
    fn send(
        &mut self,
        peer: PeerHandle,
        seq: u64,
        actor: u64,
        body: IntentBody,
        now: u64,
    ) -> AdmissionResult {
        let frame = IntentFrame {
            epoch: self.host.epoch(peer).expect("bound"),
            lane: LANE_COMBAT,
            seq,
            observed_tick: self.host.server_tick(),
            actor: n(actor),
            body,
        };
        let bytes = codec_v2::encode_intent(&frame).expect("encodes");
        assert_eq!(
            self.host.ingest(peer, &bytes, now),
            Ok(IngestDisposition::Staged)
        );
        let summary = self.host.pump(now).expect("pump");
        assert_eq!(summary.results.len(), 1, "{summary:?}");
        summary.results[0].clone()
    }

    fn end_turn(&mut self, peer: PeerHandle, seq: u64, actor: u64, now: u64) -> AdmissionResult {
        self.send(peer, seq, actor, IntentBody::EndTurn, now)
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

    /// `host.advance(now)`, mirrored: the timer's `EndTurn` first when the
    /// report says it ran, then the tick when it ticked. Returns the report
    /// and the mirrored (timer, tick) envelopes.
    fn advance(&mut self, now: u64) -> (AdvanceReport, Vec<HistoryEnvelope>, Vec<HistoryEnvelope>) {
        let advanced = self.host.advance(now).expect("advances");
        let timer = match advanced.timer_ended_turn {
            Some(head) => self.mirror(end(head)),
            None => Vec::new(),
        };
        let tick = if advanced.ticked {
            self.mirror_tick()
        } else {
            Vec::new()
        };
        self.assert_authority();
        (advanced, timer, tick)
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
            settings: self.host.settings(),
            paused: self.host.paused(),
            remaining: self.host.turn_timer_remaining_ms(),
            entries,
            peers,
        }
    }

    fn save(&self) -> String {
        String::from_utf8(self.host.save_checkpoint().expect("saves")).expect("utf8")
    }
}

// ---------------------------------------------------------------------------
// B1.
// ---------------------------------------------------------------------------

/// The constants and constructors, the defaults of every way to build a host,
/// and `set_settings`' three checks in order, each refusal beside an accepted
/// control.
#[test]
fn settings_defaults_and_validation() {
    assert_eq!(SINGLE_PLAYER_AUTO_PAUSE_BITS, 0x19);
    let single = HostSettings::single_player();
    assert_eq!(single.initial_mode, EncounterMode::TurnBased);
    assert_eq!(single.auto_pause.bits(), 0x19);
    assert!(single.auto_pause.get(AutoPauseTrigger::EncounterStarted));
    assert!(!single.auto_pause.get(AutoPauseTrigger::RoundStarted));
    assert!(!single.auto_pause.get(AutoPauseTrigger::ModeChanged));
    assert!(single.auto_pause.get(AutoPauseTrigger::OrderEnded));
    assert!(single.auto_pause.get(AutoPauseTrigger::PartyMemberDied));
    assert_eq!(single.turn_timer_ms, None);
    let multi = HostSettings::multiplayer();
    assert_eq!(multi.initial_mode, EncounterMode::TurnBased);
    assert_eq!(multi.auto_pause.bits(), 0x00);
    assert_eq!(multi.turn_timer_ms, None);
    assert_eq!(HostSettings::default(), multi);
    assert_ne!(single, multi);

    // Every way to make a host starts with multiplayer settings, running.
    let fresh = Host::new(7, config(), 0).expect("new");
    assert_eq!(fresh.settings(), multi);
    assert_eq!(fresh.paused(), None);
    let wrapped = Rig::new(&tb3_value());
    assert_eq!(wrapped.host.settings(), multi);
    assert_eq!(wrapped.host.paused(), None);
    assert_eq!(wrapped.host.turn_timer_remaining_ms(), None);

    // `Some(0)` is refused and stores nothing, records no time, and leaves
    // everything else alone; the accepted control is `Some(1)`.
    let mut rig = Rig::new(&tb3_value());
    let v = rig.bind_v(0);
    let before = rig.state(&[v]);
    let zero = HostSettings {
        initial_mode: EncounterMode::RealTime,
        auto_pause: toggles(0x1F),
        turn_timer_ms: Some(0),
    };
    assert_eq!(
        rig.host.set_settings(zero, 10),
        Err(HostError::InvalidSettings)
    );
    assert_eq!(rig.state(&[v]), before);
    assert_eq!(rig.host.settings(), multi);
    // The refusal recorded no time: 7 is still above the last accepted 0.
    assert_eq!(rig.host.pause(7), Ok(()));
    assert_eq!(rig.host.unpause(7), Ok(()));
    assert_eq!(
        HostError::InvalidSettings.to_string(),
        "InvalidSettings at host/admission"
    );
    assert!(std::error::Error::source(&HostError::InvalidSettings).is_none());

    // The same settings with a valid timer are accepted and stored whole:
    // `initial_mode` is stored only (the authority does not move).
    let hash = rig.host.authority_hash();
    let valid = HostSettings {
        turn_timer_ms: Some(1),
        ..zero
    };
    assert_eq!(rig.host.set_settings(valid, 10), Ok(()));
    assert_eq!(rig.host.settings(), valid);
    assert_eq!(rig.host.settings().initial_mode, EncounterMode::RealTime);
    assert_eq!(rig.host.authority_hash(), hash);
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1));
    assert_eq!(rig.host.last_capture(), 0);
    rig.assert_authority();

    // TimeRegression: below the last accepted 10, refused; the same call at
    // 10 is the control. (Both faults are covered by B8.)
    let before = rig.state(&[v]);
    assert_eq!(
        rig.host.set_settings(single, 5),
        Err(HostError::TimeRegression)
    );
    assert_eq!(rig.state(&[v]), before);
    assert_eq!(rig.host.set_settings(single, 10), Ok(()));
    assert_eq!(rig.host.settings(), single);
    // `single` has no timer: the changed length disarmed the armed one.
    assert_eq!(rig.host.turn_timer_remaining_ms(), None);

    // Closed: refused where the live host just accepted the same call.
    assert_eq!(rig.host.shutdown(), Vec::new());
    let before = rig.state(&[]);
    assert_eq!(rig.host.set_settings(valid, 10), Err(HostError::Closed));
    assert_eq!(rig.state(&[]), before);
    assert_eq!(rig.host.settings(), single);
}

// ---------------------------------------------------------------------------
// B2.
// ---------------------------------------------------------------------------

/// Operator pause: a paused `advance` does not tick; `tick` still does;
/// commands run; the reason stays; unpause resumes; time never goes back.
#[test]
fn operator_pause() {
    let mut rig = Rig::new(&tb3_value());
    let v = rig.bind_v(0);
    assert_eq!(rig.host.paused(), None);

    assert_eq!(rig.host.pause(10), Ok(()));
    assert_eq!(rig.host.paused(), Some(PauseReason::Operator));
    let (advanced, timer, tick) = rig.advance(20);
    assert_eq!(advanced, report(false, None, Some(PauseReason::Operator)));
    assert!(timer.is_empty() && tick.is_empty());
    assert_eq!(rig.host.server_tick(), 0);

    // `tick` ignores pause (B-4).
    assert_eq!(rig.host.tick(), Ok(()));
    rig.mirror_tick();
    assert_eq!(rig.host.server_tick(), 1);
    assert_eq!(rig.host.paused(), Some(PauseReason::Operator));

    // Commands run while paused (P2): V's EndTurn for A is applied.
    let result = rig.end_turn(v, 1, 1, 30);
    assert_eq!(result.status, ReceiptStatus::Applied);
    assert_eq!(rig.mirror(end(a())).len(), 1);
    assert_eq!(rig.host.executions(), 1);
    assert_eq!(rig.host.paused(), Some(PauseReason::Operator));
    rig.assert_authority();

    // A second pause is a no-op and the reason stays.
    assert_eq!(rig.host.pause(31), Ok(()));
    assert_eq!(rig.host.paused(), Some(PauseReason::Operator));

    assert_eq!(rig.host.unpause(40), Ok(()));
    assert_eq!(rig.host.paused(), None);
    // A second unpause is a no-op.
    assert_eq!(rig.host.unpause(41), Ok(()));
    assert_eq!(rig.host.paused(), None);

    let (advanced, _, _) = rig.advance(50);
    assert_eq!(advanced, report(true, None, None));
    assert_eq!(rig.host.server_tick(), 2);

    // Time never goes back: refused for each, beside the accepted control at
    // the same time.
    let before = rig.state(&[v]);
    assert_eq!(rig.host.pause(45), Err(HostError::TimeRegression));
    assert_eq!(rig.host.unpause(45), Err(HostError::TimeRegression));
    assert_eq!(rig.host.advance(45), Err(HostError::TimeRegression));
    assert_eq!(rig.state(&[v]), before);
    assert_eq!(rig.host.pause(50), Ok(()));
    assert_eq!(rig.host.paused(), Some(PauseReason::Operator));
    assert_eq!(rig.host.unpause(50), Ok(()));
    assert_eq!(rig.host.paused(), None);

    // Pause is never journaled, captured or delivered: no entries beyond
    // V's command, and V sees only that command's frame.
    let entries = rig.entries();
    assert_eq!(entries.len(), 1);
    assert!(matches!(entries[0], CaptureEntry::Command(_)));
}

/// A host action and a mode request run while paused (B-6), beside the same
/// calls on a running host.
#[test]
fn host_operations_run_while_paused() {
    let mut paused = Rig::new(&tb3c_value());
    let mut running = Rig::new(&tb3c_value());
    assert_eq!(paused.host.pause(1), Ok(()));
    for rig in [&mut paused, &mut running] {
        assert_eq!(rig.host.host_action(&end(a())), Ok(None));
        rig.mirror(end(a()));
        assert_eq!(rig.host.request_mode(EncounterMode::RealTime), Ok(()));
        rig.mirror_mode(EncounterMode::RealTime);
        rig.assert_authority();
    }
    assert_eq!(paused.host.paused(), Some(PauseReason::Operator));
    assert_eq!(running.host.paused(), None);
    assert_eq!(paused.host.authority_hash(), running.host.authority_hash());
    assert_eq!(paused.host.last_capture(), 1);
    assert_eq!(running.host.last_capture(), 1);
    // But only `advance`'s tick stops.
    assert_eq!(paused.host.advance(2).map(|r| r.ticked), Ok(false));
    assert_eq!(running.host.advance(2).map(|r| r.ticked), Ok(true));
    assert_eq!(paused.host.server_tick(), 0);
    assert_eq!(running.host.server_tick(), 1);
}

// ---------------------------------------------------------------------------
// B3.
// ---------------------------------------------------------------------------

/// A restored host starts `Restore`-paused with multiplayer settings; pause
/// and settings are never saved; `tick` still ticks; any unpause resumes.
#[test]
fn restore_starts_paused() {
    let mut rig = Rig::new(&tb3_value());
    assert_eq!(rig.host.paused(), None);
    let saved = rig.save();
    assert!(saved.starts_with("{\"version\":1,"));

    // Pause and settings are not in the bytes.
    assert_eq!(rig.host.pause(1), Ok(()));
    assert_eq!(rig.host.set_settings(settings(0x1F, Some(500)), 2), Ok(()));
    assert_eq!(rig.save(), saved, "pause and settings are never saved");

    let mut restored = load_checkpoint(saved.as_bytes(), 2, 0).expect("loads");
    assert_eq!(restored.paused(), Some(PauseReason::Restore));
    assert_eq!(restored.settings(), HostSettings::multiplayer());
    assert_eq!(restored.turn_timer_remaining_ms(), None);
    assert_eq!(restored.authority_hash(), history_hash(&rig.oracle));
    // The restored host saves the same bytes again under its new
    // incarnation (version 1 stays; pause is not in the bytes).
    assert_eq!(
        String::from_utf8(restored.save_checkpoint().expect("saves")).expect("utf8"),
        saved.replacen("\"incarnation\":1", "\"incarnation\":2", 1)
    );

    // `advance` does not tick; `tick` does (gate 10 of host_save is
    // unchanged); an unpause then lets `advance` tick.
    assert_eq!(
        restored.advance(1),
        Ok(report(false, None, Some(PauseReason::Restore)))
    );
    assert_eq!(restored.server_tick(), 0);
    assert_eq!(restored.tick(), Ok(()));
    assert_eq!(restored.server_tick(), 1);
    assert_eq!(restored.paused(), Some(PauseReason::Restore));
    assert_eq!(restored.unpause(2), Ok(()));
    assert_eq!(restored.paused(), None);
    assert_eq!(restored.advance(3), Ok(report(true, None, None)));
    assert_eq!(restored.server_tick(), 2);

    // Commands run on a `Restore`-paused host (P2).
    let mut second = load_checkpoint(saved.as_bytes(), 3, 0).expect("loads");
    let peer = second
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 1)
        .expect("binds");
    let frame = IntentFrame {
        epoch: second.epoch(peer).expect("bound"),
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: n(1),
        body: IntentBody::EndTurn,
    };
    let bytes = codec_v2::encode_intent(&frame).expect("encodes");
    assert_eq!(
        second.ingest(peer, &bytes, 2),
        Ok(IngestDisposition::Staged)
    );
    let summary = second.pump(2).expect("pumps");
    assert_eq!(summary.results[0].status, ReceiptStatus::Applied);
    assert_eq!(second.paused(), Some(PauseReason::Restore));

    // A version-2 checkpoint (a host record is retained) restores paused too.
    let mut with_record = Rig::new(&tb3_value());
    assert_eq!(with_record.host.host_action(&end(a())), Ok(None));
    with_record.mirror(end(a()));
    let text = with_record.save();
    assert!(text.starts_with("{\"version\":2,"));
    let loaded = load_checkpoint(text.as_bytes(), 2, 0).expect("loads");
    assert_eq!(loaded.paused(), Some(PauseReason::Restore));
    assert_eq!(loaded.settings(), HostSettings::multiplayer());

    // A host from `from_history` starts running (the control).
    assert_eq!(Rig::new(&tb3_value()).host.paused(), None);
}

// ---------------------------------------------------------------------------
// B4.
// ---------------------------------------------------------------------------

/// The journal an RT fight produces when A (smite) and C (strike) both order
/// B: B dies and C's order then fails.
fn fight_events() -> Vec<HistoryEvent> {
    vec![
        action_resolved(a(), b(), smite(), "critical_success", 4_000_000_000),
        HistoryEvent::Died { entity: b() },
        HistoryEvent::OrderEnded { actor: c() },
    ]
}

/// `RT3` with the two orders set by host actions (zero events), mirrored.
fn fight(bits: u8, peer_controls: Option<&[EntityId]>) -> (Rig, Option<PeerHandle>) {
    let mut rig = Rig::new(&rt3_value());
    let peer = peer_controls.map(|actors| rig.bind(actors, 0));
    assert_eq!(rig.host.set_settings(settings(bits, None), 0), Ok(()));
    for action in [
        use_ability(a(), smite(), b()),
        use_ability(c(), strike(), b()),
    ] {
        assert_eq!(rig.host.host_action(&action), Ok(None));
        assert!(
            rig.mirror(action).is_empty(),
            "an RT order journals nothing"
        );
    }
    // Setting an order never pauses: zero-event operations never auto-pause.
    assert_eq!(rig.host.paused(), None);
    (rig, peer)
}

/// Round 4 starts at tick 4: `RoundStarted` pauses, a paused host stops
/// ticking, and a trigger whose toggle is off does not.
#[test]
fn auto_pause_round_started() {
    let mut rig = Rig::new(&rt3_value());
    assert_eq!(rig.host.set_settings(settings(0x02, None), 0), Ok(()));
    for now in 1..=3u64 {
        let (advanced, _, tick) = rig.advance(now);
        assert_eq!(advanced, report(true, None, None), "advance({now})");
        assert!(tick.is_empty());
    }
    let (advanced, _, tick) = rig.advance(4);
    assert_eq!(
        advanced,
        report(
            true,
            None,
            Some(PauseReason::Auto(AutoPauseTrigger::RoundStarted))
        )
    );
    assert_eq!(
        payloads(&tick),
        vec![HistoryEvent::RoundStarted { round: 1 }]
    );
    // The tick was recorded as a host record.
    let entries = rig.entries();
    assert_eq!(entries.len(), 1);
    let record = host_record(&entries[0]);
    assert_eq!((record.op, record.events.clone()), (HostOp::Tick, tick));

    let (advanced, _, _) = rig.advance(5);
    assert_eq!(
        advanced,
        report(
            false,
            None,
            Some(PauseReason::Auto(AutoPauseTrigger::RoundStarted))
        )
    );
    assert_eq!(rig.host.server_tick(), 4);

    // Control: the same ticks with `RoundStarted` off never pause.
    let mut off = Rig::new(&rt3_value());
    assert_eq!(off.host.set_settings(settings(0x1D, None), 0), Ok(()));
    for now in 1..=5u64 {
        let (advanced, _, _) = off.advance(now);
        assert_eq!(advanced, report(true, None, None), "advance({now})");
    }
    assert_eq!(off.host.server_tick(), 5);
}

/// A real-time tick whose events are `[ActionResolved, Died { B },
/// OrderEnded { C }]`: the first trigger in journal order whose toggle is on
/// wins, a death counts only for the party, and a pause is never replaced.
#[test]
fn auto_pause_tick_triggers_and_party() {
    // (b) Bits 0x19, no session: B is nobody's, so `Died` is skipped and
    // `OrderEnded` wins.
    let (mut rig, _) = fight(0x19, None);
    let (advanced, _, tick) = rig.advance(1);
    assert_eq!(payloads(&tick), fight_events());
    assert_eq!(
        advanced,
        report(
            true,
            None,
            Some(PauseReason::Auto(AutoPauseTrigger::OrderEnded))
        )
    );
    assert_eq!(
        rig.host.paused(),
        Some(PauseReason::Auto(AutoPauseTrigger::OrderEnded))
    );

    // (c) A v2 peer controlling B makes the death a party death, and it
    // comes before `OrderEnded` in the journal.
    let (mut rig, peer) = fight(0x19, Some(&[b()]));
    assert!(peer.is_some());
    let (advanced, _, tick) = rig.advance(1);
    assert_eq!(payloads(&tick), fight_events());
    assert_eq!(
        advanced.paused,
        Some(PauseReason::Auto(AutoPauseTrigger::PartyMemberDied))
    );

    // (c2) The same party with `PartyMemberDied` off: the earlier event does
    // not fire, the later `OrderEnded` does (the wrong trigger cannot win).
    let (mut rig, _) = fight(0x09, Some(&[b()]));
    let (advanced, _, _) = rig.advance(1);
    assert_eq!(
        advanced.paused,
        Some(PauseReason::Auto(AutoPauseTrigger::OrderEnded))
    );

    // (c3) A party member who does not die: the party is the union of the
    // control lists, so C (whose order ends, not whose body falls) is not a
    // death; with only `PartyMemberDied` on nothing fires.
    let (mut rig, _) = fight(0x10, Some(&[a(), c()]));
    let (advanced, _, tick) = rig.advance(1);
    assert_eq!(payloads(&tick), fight_events());
    assert_eq!(advanced, report(true, None, None));
    // ... and the control: with B in the party the same toggle fires.
    let (mut rig, _) = fight(0x10, Some(&[a(), c(), b()]));
    let (advanced, _, _) = rig.advance(1);
    assert_eq!(
        advanced.paused,
        Some(PauseReason::Auto(AutoPauseTrigger::PartyMemberDied))
    );

    // The party is the union over sessions: two peers, B in the second.
    let (mut rig, _) = fight(0x10, Some(&[a()]));
    let _second = rig.bind(&[b()], 0);
    let (advanced, _, _) = rig.advance(1);
    assert_eq!(
        advanced.paused,
        Some(PauseReason::Auto(AutoPauseTrigger::PartyMemberDied))
    );

    // (d) Bits 0x00: nothing fires.
    let (mut rig, _) = fight(0x00, Some(&[b()]));
    let (advanced, _, tick) = rig.advance(1);
    assert_eq!(payloads(&tick), fight_events());
    assert_eq!(advanced, report(true, None, None));
    assert_eq!(rig.host.paused(), None);

    // (f) Already paused by the operator: `tick` ignores pause, journals the
    // same events, and the pause reason is not replaced.
    let (mut rig, _) = fight(0x19, None);
    assert_eq!(rig.host.pause(0), Ok(()));
    assert_eq!(rig.host.tick(), Ok(()));
    let tick = rig.mirror_tick();
    assert_eq!(payloads(&tick), fight_events());
    assert_eq!(rig.host.paused(), Some(PauseReason::Operator));
    let entries = rig.entries();
    assert_eq!(entries.len(), 1);
    let record = host_record(&entries[0]);
    assert_eq!(record.op, HostOp::Tick);
    assert_eq!(record.events, tick);
    rig.assert_authority();
}

/// A mode change pauses by toggle `ModeChanged` (peer command and timer
/// paths), the first event in journal order wins over `RoundStarted`, and
/// nothing fires early.
#[test]
fn auto_pause_mode_change() {
    // (e) The switch applies when C ends the last turn of the round.
    let scenario = |bits: u8| {
        let mut rig = Rig::new(&tb3c_value());
        let v = rig.bind_v(0);
        assert_eq!(rig.host.set_settings(settings(bits, None), 1), Ok(()));
        assert_eq!(rig.host.request_mode(EncounterMode::RealTime), Ok(()));
        rig.mirror_mode(EncounterMode::RealTime);
        for actor in [a(), b()] {
            assert_eq!(rig.host.host_action(&end(actor)), Ok(None));
            rig.mirror(end(actor));
            assert_eq!(rig.host.paused(), None, "no boundary yet");
        }
        let result = rig.end_turn(v, 1, 3, 2);
        assert_eq!(result.status, ReceiptStatus::Applied);
        let events = rig.mirror(end(c()));
        assert_eq!(
            payloads(&events),
            vec![
                HistoryEvent::ModeChanged {
                    mode: EncounterMode::RealTime,
                    round: 1
                },
                HistoryEvent::RoundStarted { round: 1 },
            ]
        );
        rig.assert_authority();
        rig.host.paused()
    };
    assert_eq!(
        scenario(0x04),
        Some(PauseReason::Auto(AutoPauseTrigger::ModeChanged))
    );
    // `ModeChanged` precedes `RoundStarted` in the journal: it wins when both
    // are on, and `RoundStarted` fires when it is the only one.
    assert_eq!(
        scenario(0x06),
        Some(PauseReason::Auto(AutoPauseTrigger::ModeChanged))
    );
    assert_eq!(
        scenario(0x02),
        Some(PauseReason::Auto(AutoPauseTrigger::RoundStarted))
    );
    // Control: every toggle that does not match stays running.
    assert_eq!(scenario(0x19), None);
    assert_eq!(scenario(0x00), None);
}

/// Zero-event operations never auto-pause, even with every toggle on: a mode
/// request, a host RT order and a peer RT order (all journal nothing).
#[test]
fn zero_event_operations_never_auto_pause() {
    let mut rig = Rig::new(&tb3c_value());
    assert_eq!(rig.host.set_settings(settings(0x1F, None), 0), Ok(()));
    assert_eq!(rig.host.request_mode(EncounterMode::RealTime), Ok(()));
    rig.mirror_mode(EncounterMode::RealTime);
    assert_eq!(rig.host.paused(), None);
    assert!(rig.entries().is_empty());

    let mut rt = Rig::new(&rt3_value());
    let v = rt.bind_v(0);
    assert_eq!(rt.host.set_settings(settings(0x1F, None), 1), Ok(()));
    let result = rt.send(
        v,
        1,
        1,
        IntentBody::DeclareAction {
            ability: strike(),
            target: n(2),
        },
        2,
    );
    assert_eq!(result.status, ReceiptStatus::Applied);
    assert!(rt.mirror(use_ability(a(), strike(), b())).is_empty());
    assert_eq!(
        rt.host.host_action(&use_ability(c(), whiff(), a())),
        Ok(None)
    );
    assert!(rt.mirror(use_ability(c(), whiff(), a())).is_empty());
    assert_eq!(rt.host.paused(), None);
    assert_eq!(
        rt.host.last_capture(),
        1,
        "only the peer command is an entry"
    );
    rt.assert_authority();
}

/// Peer commands and host actions auto-pause when their events map to an
/// enabled trigger (a party death), and not when the toggle is off or the
/// victim is not the party's.
#[test]
fn auto_pause_after_commands_and_actions() {
    // (g) A peer command: A smites C; V controls all of them.
    let peer_case = |bits: u8| {
        let mut rig = Rig::new(&tb3_value());
        let v = rig.bind_v(0);
        assert_eq!(rig.host.set_settings(settings(bits, None), 1), Ok(()));
        let result = rig.send(
            v,
            1,
            1,
            IntentBody::DeclareAction {
                ability: smite(),
                target: n(3),
            },
            2,
        );
        assert_eq!(result.status, ReceiptStatus::Applied);
        let events = rig.mirror(use_ability(a(), smite(), c()));
        assert_eq!(events.len(), 3);
        assert_eq!(
            payloads(&events)[..2],
            [
                action_resolved(a(), c(), smite(), "critical_success", 4_000_000_000),
                HistoryEvent::Died { entity: c() },
            ]
        );
        rig.assert_authority();
        rig.host.paused()
    };
    assert_eq!(
        peer_case(0x10),
        Some(PauseReason::Auto(AutoPauseTrigger::PartyMemberDied))
    );
    assert_eq!(peer_case(0x00), None);
    assert_eq!(peer_case(0x0F), None, "a toggle for another event");

    // A host action: the same death, only when the victim is the party's.
    let action_case = |party: &[EntityId]| {
        let mut rig = Rig::new(&tb3_value());
        let _v = rig.bind(party, 0);
        assert_eq!(rig.host.set_settings(settings(0x10, None), 1), Ok(()));
        assert!(rig
            .host
            .host_action(&use_ability(a(), smite(), c()))
            .is_ok());
        rig.mirror(use_ability(a(), smite(), c()));
        rig.assert_authority();
        rig.host.paused()
    };
    assert_eq!(
        action_case(&[c()]),
        Some(PauseReason::Auto(AutoPauseTrigger::PartyMemberDied))
    );
    assert_eq!(action_case(&[a(), b()]), None);

    // A rejected peer command journals nothing and never pauses (the control
    // is the accepted command above).
    let mut rig = Rig::new(&tb3_value());
    let v = rig.bind_v(0);
    assert_eq!(rig.host.set_settings(settings(0x1F, None), 1), Ok(()));
    let result = rig.end_turn(v, 1, 2, 2); // B is not the head
    assert!(matches!(result.status, ReceiptStatus::Rejected(_)));
    assert_eq!(rig.host.paused(), None);
}

/// Auto-pause freezes the armed timer; unpause resumes it.
#[test]
fn auto_pause_freezes_the_timer() {
    let mut rig = Rig::new(&tb3_value());
    let v = rig.bind_v(0);
    assert_eq!(
        rig.host.set_settings(settings(0x10, Some(1000)), 100),
        Ok(())
    );
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1000));
    // A kills C at 400: the head becomes B, whose timer is armed frozen with
    // the full length because the host is paused.
    let result = rig.send(
        v,
        1,
        1,
        IntentBody::DeclareAction {
            ability: smite(),
            target: n(3),
        },
        400,
    );
    assert_eq!(result.status, ReceiptStatus::Applied);
    rig.mirror(use_ability(a(), smite(), c()));
    assert_eq!(
        rig.host.paused(),
        Some(PauseReason::Auto(AutoPauseTrigger::PartyMemberDied))
    );
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1000));
    // Time passes while paused: nothing counts down and nothing expires.
    let (advanced, _, _) = rig.advance(5000);
    assert_eq!(
        advanced,
        report(
            false,
            None,
            Some(PauseReason::Auto(AutoPauseTrigger::PartyMemberDied))
        )
    );
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1000));
    // Unpause at 5000: the full length counts from there.
    assert_eq!(rig.host.unpause(5000), Ok(()));
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1000));
    let (advanced, timer, _) = rig.advance(5999);
    assert_eq!(advanced, report(true, None, None));
    assert!(timer.is_empty());
    let (advanced, timer, _) = rig.advance(6000);
    assert_eq!(advanced.timer_ended_turn, Some(b()));
    assert_eq!(payloads(&timer), vec![turn_started(a(), 1)]);
}

/// A timer expiry whose `EndTurn` triggers an auto-pause: the call reports
/// the ended turn, does not tick, and reports the pause.
#[test]
fn timer_expiry_can_auto_pause() {
    let mut rig = Rig::new(&tb3c_value());
    assert_eq!(rig.host.set_settings(settings(0x04, Some(1000)), 0), Ok(()));
    assert_eq!(rig.host.request_mode(EncounterMode::RealTime), Ok(()));
    rig.mirror_mode(EncounterMode::RealTime);
    let (advanced, _, _) = rig.advance(1000);
    assert_eq!(advanced, report(true, Some(a()), None));
    let (advanced, _, _) = rig.advance(2000);
    assert_eq!(advanced, report(true, Some(b()), None));
    let ticks = rig.host.server_tick();
    let (advanced, timer, tick) = rig.advance(3000);
    assert_eq!(
        advanced,
        report(
            false,
            Some(c()),
            Some(PauseReason::Auto(AutoPauseTrigger::ModeChanged))
        )
    );
    assert_eq!(
        payloads(&timer),
        vec![
            HistoryEvent::ModeChanged {
                mode: EncounterMode::RealTime,
                round: 1
            },
            HistoryEvent::RoundStarted { round: 1 },
        ]
    );
    assert!(tick.is_empty());
    assert_eq!(
        rig.host.server_tick(),
        ticks,
        "the pausing call did not tick"
    );
    // The encounter is real-time now: no turn-based head, so no timer.
    assert_eq!(rig.host.turn_timer_remaining_ms(), None);
    // Three timer records, in order.
    let entries = rig.entries();
    assert_eq!(entries.len(), 3);
    for entry in &entries {
        assert_eq!(host_record(entry).op, HostOp::TurnTimer);
    }
    assert_eq!(host_record(&entries[2]).events, timer);
}

// ---------------------------------------------------------------------------
// B5.
// ---------------------------------------------------------------------------

/// The timer fires exactly at its deadline, freezes while paused, resumes
/// with its remainder, is recorded as `TurnTimer` host records and is never a
/// command; a real-time encounter has no timer.
#[test]
fn turn_timer() {
    let mut rig = Rig::new(&tb3_value());
    let v = rig.bind_v(0);
    assert_eq!(rig.host.set_settings(settings(0, Some(1000)), 5000), Ok(()));
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1000));

    // One millisecond early: no end.
    let (advanced, timer, _) = rig.advance(5999);
    assert_eq!(advanced, report(true, None, None));
    assert!(timer.is_empty());
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1));
    assert_eq!(rig.host.last_capture(), 0);

    // At the deadline: A's turn ends, B is next, the timer re-arms for B.
    let (advanced, timer_a, _) = rig.advance(6000);
    assert_eq!(advanced, report(true, Some(a()), None));
    assert_eq!(payloads(&timer_a), vec![turn_started(b(), 0)]);
    assert_eq!(rig.host.awaiting_orders(), vec![b()]);
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1000));

    // Pause at 6500: 500 remain, frozen.
    assert_eq!(rig.host.pause(6500), Ok(()));
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(500));
    let (advanced, timer, _) = rig.advance(9000);
    assert_eq!(advanced, report(false, None, Some(PauseReason::Operator)));
    assert!(timer.is_empty());
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(500));

    // Unpause at 9000: 500 remain from there; 9499 is early, 9500 fires.
    assert_eq!(rig.host.unpause(9000), Ok(()));
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(500));
    let (advanced, timer, _) = rig.advance(9499);
    assert_eq!(advanced, report(true, None, None));
    assert!(timer.is_empty());
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1));
    let (advanced, timer_b, _) = rig.advance(9500);
    assert_eq!(advanced, report(true, Some(b()), None));
    assert_eq!(payloads(&timer_b), vec![turn_started(c(), 0)]);
    assert_eq!(rig.host.awaiting_orders(), vec![c()]);
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1000));

    // Two `TurnTimer` host records, (4, 5] and (5, 6], and no command.
    let entries = rig.entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        host_record(&entries[0]),
        &HostRecord {
            capture_seq: 1,
            op: HostOp::TurnTimer,
            history_start: 4,
            history_end: 5,
            processed_tick: 1,
            events: timer_a,
        }
    );
    assert_eq!(
        host_record(&entries[1]),
        &HostRecord {
            capture_seq: 2,
            op: HostOp::TurnTimer,
            history_start: 5,
            history_end: 6,
            processed_tick: 3,
            events: timer_b,
        }
    );
    assert_eq!(rig.host.executions(), 0, "a timer end is not a command");
    assert_eq!(rig.host.last_combat_error(), None);
    // The v2 session saw nothing of either timer end (a host operation).
    assert!(rig.drain(v).is_empty());
    // The records are evidence: a version-2 checkpoint holds them as
    // `turn_timer` records and loads.
    let text = rig.save();
    assert!(text.starts_with("{\"version\":2,"), "{text}");
    assert_eq!(text.matches("\"op\":\"turn_timer\"").count(), 2);
    let restored = load_checkpoint(text.as_bytes(), 2, 9500).expect("loads");
    assert_eq!(restored.authority_hash(), rig.host.authority_hash());
    rig.assert_authority();

    // Real-time: no turn-based head, so no timer at all.
    let mut rt = Rig::new(&rt3_value());
    assert_eq!(rt.host.set_settings(settings(0, Some(1000)), 5000), Ok(()));
    assert_eq!(rt.host.turn_timer_remaining_ms(), None);
    let (advanced, timer, _) = rt.advance(99_000);
    assert_eq!(advanced, report(true, None, None));
    assert!(timer.is_empty());
    assert_eq!(rt.host.turn_timer_remaining_ms(), None);
}

/// At most one expiry per call, however late; the timer re-arms from the
/// call's time and never fires twice for one head.
#[test]
fn at_most_one_expiry_per_call() {
    let mut rig = Rig::new(&tb3_value());
    assert_eq!(rig.host.set_settings(settings(0, Some(1000)), 0), Ok(()));
    let (advanced, timer, tick) = rig.advance(100_000);
    assert_eq!(advanced, report(true, Some(a()), None));
    assert_eq!(payloads(&timer), vec![turn_started(b(), 0)]);
    assert!(tick.is_empty());
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1000));
    assert_eq!(rig.host.last_capture(), 1);
    // The same instant again: B's deadline is 101,000.
    let (advanced, _, _) = rig.advance(100_000);
    assert_eq!(advanced, report(true, None, None));
    assert_eq!(rig.host.last_capture(), 1);
    // 101,000 is B's deadline: B's turn ends, one more entry.
    let (advanced, _, _) = rig.advance(101_000);
    assert_eq!(advanced, report(true, Some(b()), None));
    assert_eq!(rig.host.last_capture(), 2);
}

/// A timer set while paused is armed frozen with the full length and runs
/// only from the unpause.
#[test]
fn timer_set_while_paused_is_frozen() {
    let mut rig = Rig::new(&tb3_value());
    assert_eq!(rig.host.pause(100), Ok(()));
    assert_eq!(rig.host.set_settings(settings(0, Some(1000)), 200), Ok(()));
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1000));
    let (advanced, _, _) = rig.advance(50_000);
    assert_eq!(advanced, report(false, None, Some(PauseReason::Operator)));
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1000));
    assert_eq!(rig.host.last_capture(), 0);
    assert_eq!(rig.host.unpause(60_000), Ok(()));
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1000));
    let (advanced, _, _) = rig.advance(60_999);
    assert_eq!(advanced, report(true, None, None));
    let (advanced, _, _) = rig.advance(61_000);
    assert_eq!(advanced, report(true, Some(a()), None));
    // Pausing and unpausing with no timer armed changes nothing observable.
    let mut bare = Rig::new(&tb3_value());
    assert_eq!(bare.host.pause(1), Ok(()));
    assert_eq!(bare.host.turn_timer_remaining_ms(), None);
    assert_eq!(bare.host.unpause(2), Ok(()));
    assert_eq!(bare.host.turn_timer_remaining_ms(), None);
}

// ---------------------------------------------------------------------------
// B6.
// ---------------------------------------------------------------------------

/// An unchanged timer setting never re-arms; a changed one does; a new head
/// arms with the full length from the last accepted time; `None` disarms.
#[test]
fn timer_rearms() {
    let mut rig = Rig::new(&tb3_value());
    assert_eq!(rig.host.set_settings(settings(0, Some(1000)), 5000), Ok(()));
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1000));

    // The same timer at 5400: 600 remain (not re-armed). Other settings
    // change without touching the timer.
    assert_eq!(
        rig.host.set_settings(settings(0x19, Some(1000)), 5400),
        Ok(())
    );
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(600));
    assert_eq!(rig.host.settings(), settings(0x19, Some(1000)));

    // A changed length at 5500: re-armed at the new length.
    assert_eq!(
        rig.host.set_settings(settings(0x19, Some(2000)), 5500),
        Ok(())
    );
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(2000));

    // A host action ends A's turn at the last accepted time (5500): the
    // timer re-arms for B with the full length.
    assert_eq!(rig.host.host_action(&end(a())), Ok(None));
    rig.mirror(end(a()));
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(2000));
    // Time passes: the countdown is against the last accepted time.
    assert_eq!(rig.host.pause(6000), Ok(()));
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1500));
    assert_eq!(rig.host.unpause(6000), Ok(()));
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1500));

    // `None` disarms; the same `None` again is unchanged; a timer length is
    // armed again from the new time.
    assert_eq!(
        rig.host.set_settings(settings(0x19, None), 5600 + 1000),
        Ok(())
    );
    assert_eq!(rig.host.turn_timer_remaining_ms(), None);
    assert_eq!(rig.host.set_settings(settings(0, None), 7000), Ok(()));
    assert_eq!(rig.host.turn_timer_remaining_ms(), None);
    assert_eq!(rig.host.set_settings(settings(0, Some(300)), 7100), Ok(()));
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(300));

    // A shorter new length takes effect at once: it fires at 7400.
    let (advanced, _, _) = rig.advance(7399);
    assert_eq!(advanced, report(true, None, None));
    let (advanced, _, _) = rig.advance(7400);
    assert_eq!(advanced, report(true, Some(b()), None));
    rig.assert_authority();

    // The documented example literally: set at 5000, same at 5400 is 600,
    // 2000 at 5500 is 2000, an end turn gives 2000 for the next head and
    // `None` at 5600 is `None`.
    let mut lit = Rig::new(&tb3_value());
    assert_eq!(lit.host.set_settings(settings(0, Some(1000)), 5000), Ok(()));
    assert_eq!(lit.host.set_settings(settings(0, Some(1000)), 5400), Ok(()));
    assert_eq!(lit.host.turn_timer_remaining_ms(), Some(600));
    assert_eq!(lit.host.set_settings(settings(0, Some(2000)), 5500), Ok(()));
    assert_eq!(lit.host.turn_timer_remaining_ms(), Some(2000));
    assert_eq!(lit.host.host_action(&end(a())), Ok(None));
    assert_eq!(lit.host.turn_timer_remaining_ms(), Some(2000));
    assert_eq!(lit.host.set_settings(settings(0, None), 5600), Ok(()));
    assert_eq!(lit.host.turn_timer_remaining_ms(), None);
}

// ---------------------------------------------------------------------------
// B9.
// ---------------------------------------------------------------------------

/// A full journal does not disarm the timer or lose the turn end: the expiry
/// is retried on the next call, and recorded once there is room.
#[test]
fn timer_survives_a_full_journal() {
    let mut rig = Rig::new(&tb3_value());
    let v = rig.bind_v(0);
    // 4,096 `EndTurn` commands cycling A, B, C (+100 ms each).
    for seq in 1..=MAX_CAPTURE_RECORDS as u64 {
        let actor = (seq - 1) % 3;
        let result = rig.end_turn(v, seq, actor + 1, seq * 100);
        assert_eq!(result.status, ReceiptStatus::Applied, "seq {seq}");
        rig.mirror(end(eid(actor as u32)));
        rig.drain(v);
    }
    assert_eq!(rig.host.last_capture(), MAX_CAPTURE_RECORDS as u64);
    let last_now = MAX_CAPTURE_RECORDS as u64 * 100;
    assert_eq!(last_now, 409_600);
    rig.assert_authority();
    // 4,096 is one more than a multiple of three: after A's last end the
    // head is B (the contract text says A; the arithmetic says B).
    assert_eq!(rig.host.awaiting_orders(), vec![b()]);

    assert_eq!(
        rig.host.set_settings(settings(0, Some(1000)), 409_700),
        Ok(())
    );
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1000));
    let before = rig.state(&[v]);

    // The deadline: the journal has no room, the end of turn is not lost.
    let (advanced, timer, _) = rig.advance(410_700);
    assert_eq!(advanced, report(true, None, None));
    assert!(timer.is_empty());
    assert_eq!(
        rig.host.last_capture(),
        MAX_CAPTURE_RECORDS as u64,
        "no new entry"
    );
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(0));
    assert_eq!(rig.host.awaiting_orders(), vec![b()]);
    let after = rig.state(&[v]);
    assert_eq!(after.entries, before.entries);
    assert_eq!(
        after.server_tick,
        before.server_tick + 1,
        "the tick still ran"
    );
    assert_eq!(after.executions, before.executions);

    // Room made: the very next call ends the turn and records it.
    rig.host.acknowledge_captures(1).expect("acks");
    let (advanced, timer, _) = rig.advance(410_701);
    assert_eq!(advanced, report(true, Some(b()), None));
    assert_eq!(payloads(&timer), vec![turn_started(c(), 1365)]);
    assert_eq!(rig.host.last_capture(), MAX_CAPTURE_RECORDS as u64 + 1);
    let entries = rig.entries();
    let last = host_record(entries.last().expect("entries"));
    assert_eq!(last.op, HostOp::TurnTimer);
    assert_eq!(last.capture_seq, MAX_CAPTURE_RECORDS as u64 + 1);
    assert_eq!(last.events, timer);
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1000));
    assert_eq!(rig.host.awaiting_orders(), vec![c()]);
    rig.assert_authority();
}

// ---------------------------------------------------------------------------
// B8.
// ---------------------------------------------------------------------------

/// First failure wins at every adjacent pair with both faults present, each
/// beside the call that succeeds: Closed before TimeRegression, and
/// TimeRegression before InvalidSettings. (`advance`'s remaining pair, its
/// own tick failing after the timer, is `advance_returns_the_tick_error`.)
#[test]
fn precedence_pairs() {
    let build = || {
        let mut rig = Rig::new(&tb3_value());
        let v = rig.bind_v(0);
        // The last accepted time is 100.
        assert_eq!(rig.host.pause(100), Ok(()));
        assert_eq!(rig.host.unpause(100), Ok(()));
        (rig, v)
    };
    let zero = settings(0, Some(0));
    let ok_settings = settings(0, Some(500));

    // Live host, `now_ms` below the last: TimeRegression for every call.
    let (mut rig, v) = build();
    let before = rig.state(&[v]);
    assert_eq!(
        rig.host.set_settings(ok_settings, 99),
        Err(HostError::TimeRegression)
    );
    assert_eq!(rig.host.pause(99), Err(HostError::TimeRegression));
    assert_eq!(rig.host.unpause(99), Err(HostError::TimeRegression));
    assert_eq!(rig.host.advance(99), Err(HostError::TimeRegression));
    // Both faults: below the last and a zero timer: TimeRegression wins.
    assert_eq!(
        rig.host.set_settings(zero, 99),
        Err(HostError::TimeRegression)
    );
    assert_eq!(rig.state(&[v]), before);
    // The controls at the last accepted time.
    assert_eq!(
        rig.host.set_settings(zero, 100),
        Err(HostError::InvalidSettings)
    );
    assert_eq!(rig.host.set_settings(ok_settings, 100), Ok(()));
    assert_eq!(rig.host.pause(100), Ok(()));
    assert_eq!(rig.host.unpause(100), Ok(()));
    assert_eq!(rig.host.advance(100).map(|r| r.ticked), Ok(true));

    // Closed host, `now_ms` below the last: Closed wins, for every call and
    // for the zero timer as well.
    let (mut rig, _) = build();
    assert_eq!(rig.host.shutdown(), Vec::new());
    let before = rig.state(&[]);
    assert_eq!(
        rig.host.set_settings(ok_settings, 99),
        Err(HostError::Closed)
    );
    assert_eq!(rig.host.set_settings(zero, 99), Err(HostError::Closed));
    assert_eq!(rig.host.pause(99), Err(HostError::Closed));
    assert_eq!(rig.host.unpause(99), Err(HostError::Closed));
    assert_eq!(rig.host.advance(99), Err(HostError::Closed));
    // And at a valid time too (the pair with only the closed fault).
    assert_eq!(
        rig.host.set_settings(ok_settings, 100),
        Err(HostError::Closed)
    );
    assert_eq!(rig.host.pause(100), Err(HostError::Closed));
    assert_eq!(rig.host.unpause(100), Err(HostError::Closed));
    assert_eq!(rig.host.advance(100), Err(HostError::Closed));
    assert_eq!(rig.state(&[]), before);
}

// ---------------------------------------------------------------------------
// Review survivors.
// ---------------------------------------------------------------------------

/// B-2: `pause` over any existing pause changes nothing, so the reason stays
/// exactly (`Restore`, `Auto(trigger)`), beside the running host it pauses.
#[test]
fn pause_never_replaces_a_reason() {
    // Control: a running host takes `Operator`.
    let mut running = Rig::new(&tb3_value());
    assert_eq!(running.host.pause(1), Ok(()));
    assert_eq!(running.host.paused(), Some(PauseReason::Operator));

    // Over `Restore`.
    let saved = Rig::new(&tb3_value()).save();
    let mut restored = load_checkpoint(saved.as_bytes(), 2, 0).expect("loads");
    assert_eq!(restored.paused(), Some(PauseReason::Restore));
    assert_eq!(restored.pause(1), Ok(()));
    assert_eq!(restored.paused(), Some(PauseReason::Restore));

    // Over `Auto(OrderEnded)`.
    let (mut rig, _) = fight(0x19, None);
    let (advanced, _, _) = rig.advance(1);
    let auto = Some(PauseReason::Auto(AutoPauseTrigger::OrderEnded));
    assert_eq!(advanced.paused, auto);
    assert_eq!(rig.host.pause(2), Ok(()));
    assert_eq!(rig.host.paused(), auto);
    // Unpause clears whatever the reason was.
    assert_eq!(rig.host.unpause(3), Ok(()));
    assert_eq!(rig.host.paused(), None);
}

/// B-3 step 5: the tick's error is returned as is, and a timer `EndTurn` from
/// the same call stays committed. A full journal, a pending switch to
/// real-time and a timer on the last head of the round build it: the timer's
/// `EndTurn` crosses the boundary (filling the journal), and the real-time
/// tick then has no room for its host record.
#[test]
fn advance_returns_the_tick_error() {
    let mut rig = Rig::new(&tb3c_value());
    let v = rig.bind_v(0);
    for seq in 1..=MAX_CAPTURE_RECORDS as u64 {
        let actor = (seq - 1) % 3;
        let result = rig.end_turn(v, seq, actor + 1, seq * 100);
        assert_eq!(result.status, ReceiptStatus::Applied, "seq {seq}");
        rig.mirror(end(eid(actor as u32)));
        rig.drain(v);
    }
    // The head is B. Request real-time, make room for two entries, then B
    // ends its turn (a command): the head is C, the journal has room for one.
    assert_eq!(rig.host.request_mode(EncounterMode::RealTime), Ok(()));
    rig.mirror_mode(EncounterMode::RealTime);
    rig.host.acknowledge_captures(2).expect("acks");
    let result = rig.end_turn(v, 4097, 2, 409_700);
    assert_eq!(result.status, ReceiptStatus::Applied);
    rig.mirror(end(b()));
    rig.drain(v);
    assert_eq!(rig.host.awaiting_orders(), vec![c()]);
    assert_eq!(rig.host.last_capture(), 4097);
    assert_eq!(
        rig.host.set_settings(settings(0, Some(1000)), 409_800),
        Ok(())
    );
    let ticks = rig.host.server_tick();

    // The deadline: C's turn ends (boundary: the mode switches, the journal
    // fills), then the real-time tick has no room: `CaptureFull`, returned.
    assert_eq!(rig.host.advance(410_800), Err(HostError::CaptureFull));
    let events = rig.mirror(end(c()));
    assert_eq!(
        payloads(&events),
        vec![
            HistoryEvent::ModeChanged {
                mode: EncounterMode::RealTime,
                round: 1366
            },
            HistoryEvent::RoundStarted { round: 1366 },
        ]
    );
    rig.assert_authority();
    assert_eq!(rig.host.server_tick(), ticks, "the tick did not run");
    assert_eq!(
        rig.host.last_capture(),
        4098,
        "the timer's EndTurn stayed committed"
    );
    let entries = rig.entries();
    assert_eq!(entries.len(), MAX_CAPTURE_RECORDS);
    let record = host_record(entries.last().expect("entries"));
    assert_eq!(record.op, HostOp::TurnTimer);
    assert_eq!(record.capture_seq, 4098);
    assert_eq!(record.events, events);
    assert_eq!(
        rig.host.turn_timer_remaining_ms(),
        None,
        "real-time: no timer"
    );

    // Control: with room made, the same call succeeds and ticks.
    rig.host.acknowledge_captures(4098).expect("acks");
    let (advanced, _, tick) = rig.advance(410_801);
    assert_eq!(advanced, report(true, None, None));
    assert!(tick.is_empty());
    assert_eq!(rig.host.server_tick(), ticks + 1);
}

/// B-9: the remainder saturates at zero. A deadline already passed (no
/// `advance` ran) freezes at 0 on pause and reads 0 when running.
#[test]
fn timer_remainder_saturates() {
    // Reading past the deadline while running: 0, not a wrapped value. The
    // later calls are binds, which accept time without observing the timer.
    let mut rig = Rig::new(&tb3_value());
    assert_eq!(rig.host.set_settings(settings(0, Some(1000)), 0), Ok(()));
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1000));
    rig.bind(&[a()], 999);
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1));
    rig.bind(&[a()], 1000);
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(0));
    rig.bind(&[a()], 5000);
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(0));

    // Freezing past the deadline stores 0; the unpause counts from there and
    // the expiry is then due at once.
    assert_eq!(rig.host.pause(6000), Ok(()));
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(0));
    assert_eq!(rig.host.unpause(7000), Ok(()));
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(0));
    let (advanced, timer, _) = rig.advance(7000);
    assert_eq!(advanced, report(true, Some(a()), None));
    assert_eq!(payloads(&timer), vec![turn_started(b(), 0)]);
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(1000));
}

/// B-5: the party is the union of every session's control list: a member of
/// the first session's list dies, with a later session that does not hold it.
#[test]
fn party_is_the_union_of_every_session() {
    let (mut rig, _) = fight(0x10, Some(&[b()]));
    let _second = rig.bind(&[a()], 0);
    let (advanced, _, tick) = rig.advance(1);
    assert_eq!(payloads(&tick), fight_events());
    assert_eq!(
        advanced.paused,
        Some(PauseReason::Auto(AutoPauseTrigger::PartyMemberDied))
    );
    // Control: neither session holds B, so nothing fires.
    let (mut rig, _) = fight(0x10, Some(&[a()]));
    let _second = rig.bind(&[c()], 0);
    let (advanced, _, _) = rig.advance(1);
    assert_eq!(advanced, report(true, None, None));
}

/// B-5/B-9: an auto-pause freezes the timer even when the head does not
/// change (an ability that does not end the turn kills a party member, so the
/// same head's timer stays armed): the remainder holds while paused and
/// counts down again from the unpause.
#[test]
fn auto_pause_freezes_the_same_head_timer() {
    let mut value = tb3_value();
    // `smite` (the second authored ability) no longer ends the turn.
    value["world"]["combat"]["definition"]["abilities"][1]["ends_turn"] = json!(false);
    let mut rig = Rig::new(&value);
    assert_eq!(
        rig.host.set_settings(settings(0x10, Some(1000)), 100),
        Ok(())
    );
    // C is the party; binding at 400 accepts time: 700 remain.
    let _party = rig.bind(&[c()], 400);
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(700));
    assert_eq!(rig.host.paused(), None);

    // A smites C; the turn stays with A and the timer's key is unchanged.
    let outcome = rig.host.host_action(&use_ability(a(), smite(), c()));
    assert!(matches!(outcome, Ok(Some(_))), "{outcome:?}");
    let events = rig.mirror(use_ability(a(), smite(), c()));
    assert_eq!(
        payloads(&events),
        vec![
            action_resolved(a(), c(), smite(), "critical_success", 4_000_000_000),
            HistoryEvent::Died { entity: c() },
        ]
    );
    assert_eq!(
        rig.host.awaiting_orders(),
        vec![a()],
        "the head is unchanged"
    );
    assert_eq!(
        rig.host.paused(),
        Some(PauseReason::Auto(AutoPauseTrigger::PartyMemberDied))
    );
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(700));
    // Time passes while paused: the remainder holds.
    let (advanced, _, _) = rig.advance(5000);
    assert_eq!(
        advanced,
        report(
            false,
            None,
            Some(PauseReason::Auto(AutoPauseTrigger::PartyMemberDied))
        )
    );
    assert_eq!(rig.host.turn_timer_remaining_ms(), Some(700));
    // Unpause: 700 count from 5000.
    assert_eq!(rig.host.unpause(5000), Ok(()));
    let (advanced, _, _) = rig.advance(5699);
    assert_eq!(advanced, report(true, None, None));
    let (advanced, timer, _) = rig.advance(5700);
    assert_eq!(advanced, report(true, Some(a()), None));
    assert_eq!(payloads(&timer), vec![turn_started(b(), 0)]);
    rig.assert_authority();
}
