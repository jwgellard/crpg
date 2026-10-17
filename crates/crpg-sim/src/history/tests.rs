//! T097 unit tests: the `HistoryWorld` journal byte cache and world-only
//! staging (`src/history.rs`).
//!
//! `reference` is the pre-T097 master (`07b3e77`) body of every mutating
//! operation, of `acknowledge` and of `finish`, copied as free functions over
//! `&mut HistoryWorld` with exactly the contract's three substitutions: reads
//! go through `Journal::as_slice`, appends adopt the oracle's own full-rescan
//! byte total, and acknowledgement drains through `Journal::drain_prefix`. It
//! is an independent implementation of both the old cost model and the byte
//! total's definition. `ENCODINGS` counts canonical encodings per test
//! thread, so the regression guard is a count of work, never a wall clock.

#[path = "../../tests/support/mod.rs"]
#[allow(dead_code)]
mod support;

use proptest::prelude::*;

use crpg_core::EntityId;
use support::Fixture;

use crate::combat::{CombatAction, EncounterSpec};
use crate::history::{
    envelope_len, HistoryEnvelope, HistoryError, HistoryEvent, HistoryWorld, ENCODINGS,
    MAX_HISTORY_BYTES, MAX_HISTORY_EVENTS, MAX_HISTORY_STRING_BYTES,
};
use crate::world::EntityMeta;

/// The master (`07b3e77`) operation bodies, kept as the differential oracle.
mod reference {
    use crpg_core::{EntityId, Tick};

    use crate::combat::{ActionOutcome, CombatAction, EncounterSpec, EncounterSummary};
    use crate::history::{
        envelope_len, is_valid_outcome, outcome_text, push_transition, translate_legacy,
        HistoryEnvelope, HistoryError, HistoryEvent, HistoryWorld, MAX_HISTORY_BYTES,
        MAX_HISTORY_EVENTS, MAX_HISTORY_STRING_BYTES,
    };
    use crate::world::EntityMeta;

    /// The master `finish`: it re-encodes the whole retained journal.
    pub(super) fn finish(
        history: &mut HistoryWorld,
        events: Vec<HistoryEvent>,
        tick: Tick,
    ) -> Result<(), HistoryError> {
        for event in &events {
            if let HistoryEvent::ActionResolved { outcome, .. } = event {
                if outcome.len() > MAX_HISTORY_STRING_BYTES {
                    return Err(HistoryError::StringLimit);
                }
                debug_assert!(
                    is_valid_outcome(outcome),
                    "the controller maps only the symbolic outcome vocabulary"
                );
            }
        }
        let count = events.len() as u64;
        let end = history
            .next_seq
            .checked_add(count)
            .ok_or(HistoryError::SequenceExhausted)?;
        let retained = history
            .pending
            .len()
            .checked_add(events.len())
            .ok_or(HistoryError::EventLimit)?;
        if retained > MAX_HISTORY_EVENTS {
            return Err(HistoryError::EventLimit);
        }
        let mut bytes: usize = 0;
        for envelope in history.pending.as_slice() {
            bytes = bytes
                .checked_add(envelope_len(envelope))
                .ok_or(HistoryError::ByteLimit)?;
            if bytes > MAX_HISTORY_BYTES {
                return Err(HistoryError::ByteLimit);
            }
        }
        let mut envelopes = Vec::with_capacity(events.len());
        let mut seq = history.next_seq;
        for event in events {
            let envelope = HistoryEnvelope {
                tick,
                seq,
                payload: event,
            };
            let encoded = envelope_len(&envelope);
            bytes = bytes.checked_add(encoded).ok_or(HistoryError::ByteLimit)?;
            if bytes > MAX_HISTORY_BYTES {
                return Err(HistoryError::ByteLimit);
            }
            envelopes.push(envelope);
            seq = seq.checked_add(1).ok_or(HistoryError::SequenceExhausted)?;
        }
        history.pending.append(envelopes, bytes);
        history.next_seq = end;
        Ok(())
    }

    /// The master `spawn` body.
    pub(super) fn spawn(
        history: &mut HistoryWorld,
        meta: EntityMeta,
    ) -> Result<EntityId, HistoryError> {
        let mut staged = history.clone();
        let id = staged.world.spawn(meta);
        let drained = staged.world.events_mut().drain();
        let tick = staged.world.tick();
        debug_assert_eq!(drained.len(), 1, "spawn enqueues exactly one event");
        let mut events = Vec::with_capacity(drained.len());
        for envelope in &drained {
            events.push(translate_legacy(&envelope.payload));
        }
        finish(&mut staged, events, tick)?;
        *history = staged;
        Ok(id)
    }

    /// The master `despawn` body.
    pub(super) fn despawn(
        history: &mut HistoryWorld,
        entity: EntityId,
    ) -> Result<bool, HistoryError> {
        let mut staged = history.clone();
        let (removed, facts) = staged.world.despawn_tracked(entity);
        let drained = staged.world.events_mut().drain();
        let tick = staged.world.tick();
        debug_assert!(drained.len() <= 1, "despawn enqueues at most one event");
        let mut events = Vec::with_capacity(drained.len() + 1);
        for envelope in &drained {
            events.push(translate_legacy(&envelope.payload));
        }
        push_transition(&mut events, &facts);
        finish(&mut staged, events, tick)?;
        *history = staged;
        Ok(removed)
    }

    /// The master `tick` body.
    pub(super) fn tick(history: &mut HistoryWorld) -> Result<(), HistoryError> {
        let mut staged = history.clone();
        crate::tick::tick(&mut staged.world);
        let drained = staged.world.events_mut().drain();
        let tick = staged.world.tick();
        let mut events = Vec::with_capacity(drained.len());
        for envelope in &drained {
            events.push(translate_legacy(&envelope.payload));
        }
        debug_assert!(
            events.is_empty(),
            "the tick system list currently emits no events"
        );
        finish(&mut staged, events, tick)?;
        *history = staged;
        Ok(())
    }

    /// The master `start_encounter` body.
    pub(super) fn start_encounter(
        history: &mut HistoryWorld,
        spec: &EncounterSpec<'_>,
    ) -> Result<(), HistoryError> {
        let mut staged = history.clone();
        let facts = match crate::combat::start_encounter_tracked(&mut staged.world, spec) {
            Ok(facts) => facts,
            Err(error) => return Err(HistoryError::Combat(error)),
        };
        let drained = staged.world.events_mut().drain();
        let tick = staged.world.tick();
        let mut events = Vec::with_capacity(drained.len() + 1);
        for envelope in &drained {
            events.push(translate_legacy(&envelope.payload));
        }
        if let Some((actor, round)) = facts.initial_turn {
            events.push(HistoryEvent::TurnStarted {
                actor,
                round: u64::from(round),
            });
        }
        finish(&mut staged, events, tick)?;
        *history = staged;
        Ok(())
    }

    /// The master `perform_action` body.
    pub(super) fn perform_action(
        history: &mut HistoryWorld,
        action: &CombatAction,
    ) -> Result<Option<ActionOutcome>, HistoryError> {
        let mut staged = history.clone();
        let (outcome, facts) =
            match crate::combat::perform_action_tracked(&mut staged.world, action) {
                Ok(done) => done,
                Err(error) => return Err(HistoryError::Combat(error)),
            };
        let drained = staged.world.events_mut().drain();
        let tick = staged.world.tick();
        let mut events = Vec::with_capacity(drained.len() + 2);
        if let Some(ref produced) = outcome {
            events.push(HistoryEvent::ActionResolved {
                actor: produced.actor,
                target: produced.target,
                ability: produced.ability,
                outcome: outcome_text(produced.outcome),
                damage: produced.damage,
            });
        }
        for envelope in &drained {
            events.push(translate_legacy(&envelope.payload));
        }
        push_transition(&mut events, &facts);
        finish(&mut staged, events, tick)?;
        *history = staged;
        Ok(outcome)
    }

    /// The master `end_encounter` body.
    pub(super) fn end_encounter(
        history: &mut HistoryWorld,
    ) -> Result<EncounterSummary, HistoryError> {
        let mut staged = history.clone();
        let summary = match crate::combat::end_encounter(&mut staged.world) {
            Ok(summary) => summary,
            Err(error) => return Err(HistoryError::Combat(error)),
        };
        let drained = staged.world.events_mut().drain();
        let tick = staged.world.tick();
        let mut events = Vec::with_capacity(drained.len());
        for envelope in &drained {
            events.push(translate_legacy(&envelope.payload));
        }
        debug_assert!(events.is_empty(), "release emits no legacy events");
        finish(&mut staged, events, tick)?;
        *history = staged;
        Ok(summary)
    }

    /// The master `acknowledge` body.
    pub(super) fn acknowledge(
        history: &mut HistoryWorld,
        through: u64,
    ) -> Result<(), HistoryError> {
        let last = history.last_sequence();
        if through > last {
            return Err(HistoryError::FutureCursor {
                requested: through,
                last,
            });
        }
        if through <= history.acknowledged {
            return Ok(());
        }
        let drop = (through - history.acknowledged) as usize;
        history.pending.drain_prefix(drop);
        history.acknowledged = through;
        Ok(())
    }
}

/// Complete canonical wrapper bytes.
fn snapshot(history: &HistoryWorld) -> Vec<u8> {
    serde_json::to_vec(history).expect("the wrapper serializes")
}

/// J1: the cached byte total equals a full rescan and stays inside the cap.
fn assert_j1(history: &HistoryWorld) {
    let mut total: usize = 0;
    for envelope in history.pending.as_slice() {
        total = total
            .checked_add(envelope_len(envelope))
            .expect("the canonical total fits");
    }
    assert_eq!(history.pending.bytes, total, "J1: cache matches a rescan");
    assert!(
        history.pending.bytes <= MAX_HISTORY_BYTES,
        "J1: cache stays inside the cap"
    );
}

/// Bytes, hash, and J1 agreement between the two twins.
fn assert_same(oracle: &HistoryWorld, fresh: &HistoryWorld, what: &str) {
    assert_eq!(
        snapshot(fresh),
        snapshot(oracle),
        "bytes diverged after {what}"
    );
    assert_eq!(
        crate::history_hash(fresh),
        crate::history_hash(oracle),
        "hash diverged after {what}"
    );
    assert_j1(oracle);
    assert_j1(fresh);
}

/// T5: an inert or rejected operation left the journal in place.
fn assert_journal_in_place(
    history: &HistoryWorld,
    pointer: *const HistoryEnvelope,
    capacity: usize,
    what: &str,
) {
    assert!(
        std::ptr::eq(history.pending.as_slice().as_ptr(), pointer),
        "{what} copied the retained journal"
    );
    assert_eq!(
        history.pending.envelopes.capacity(),
        capacity,
        "{what} reallocated the retained journal"
    );
}

/// The encodings this test thread has performed.
fn encodings() -> u64 {
    ENCODINGS.with(|count| count.get())
}

/// Runs `op` and reports the encodings it performed.
fn encodings_of<T>(op: impl FnOnce() -> T) -> (T, u64) {
    let before = encodings();
    let result = op();
    (result, encodings() - before)
}

/// Runs `op` and asserts it performed exactly `expected` encodings.
fn expect_encodings<T>(expected: u64, op: impl FnOnce() -> T) -> T {
    let (result, encoded) = encodings_of(op);
    assert_eq!(encoded, expected, "unexpected encoding count");
    result
}

/// Assembles the borrowed encounter spec one operation consumes.
fn spec_for<'a>(fixture: &'a Fixture, bundle: &'a support::SpecBundle<'a>) -> EncounterSpec<'a> {
    EncounterSpec {
        encounter: &fixture.encounter,
        ruleset: &fixture.ruleset,
        abilities: &bundle.abilities,
        outcome_tables: &bundle.tables,
        placements: &bundle.placements,
        creatures: &bundle.creatures,
        effects: &bundle.effects,
    }
}

/// A fresh wrapper with the standard encounter already started.
fn started(seed: u64) -> (HistoryWorld, Vec<EntityId>) {
    let fixture = Fixture::standard();
    let bundle = fixture.bundle();
    let spec = spec_for(&fixture, &bundle);
    let mut history = HistoryWorld::new(seed);
    history.start_encounter(&spec).expect("the fixture starts");
    let ids: Vec<EntityId> = history
        .world()
        .combatants()
        .iter()
        .map(|(id, _)| id)
        .collect();
    assert_j1(&history);
    (history, ids)
}

/// The T3 base: the standard encounter, then spawns to the cap.
fn cap_base() -> HistoryWorld {
    let fixture = Fixture::standard();
    let bundle = fixture.bundle();
    let spec = spec_for(&fixture, &bundle);
    let mut history = HistoryWorld::new(1);
    history.start_encounter(&spec).expect("the fixture starts");
    while history.pending_len() < MAX_HISTORY_EVENTS {
        history.spawn(EntityMeta {}).expect("the journal fills");
    }
    assert_j1(&history);
    history
}

/// A well-formed id that no world in these tests ever minted.
fn absent_id() -> EntityId {
    serde_json::from_value(serde_json::json!({"index": 1_000_000, "generation": 1}))
        .expect("a well-formed entity id decodes")
}

/// The `index`-th live id, or an absent one when nothing is live.
fn live_id(history: &HistoryWorld, index: usize) -> EntityId {
    let ids: Vec<EntityId> = history.world().ids().collect();
    if ids.is_empty() {
        absent_id()
    } else {
        ids[index % ids.len()]
    }
}

/// The active combatant when one holds the turn, else a live (or absent) id.
fn active_or_live(history: &HistoryWorld, index: usize) -> EntityId {
    match history.world().combat().and_then(|combat| combat.active) {
        Some(actor) => actor,
        None => live_id(history, index),
    }
}

/// The active combatant and one other live combatant, for an accepted action.
fn active_pair(history: &HistoryWorld) -> (EntityId, EntityId) {
    let active = history
        .world()
        .combat()
        .expect("an active encounter")
        .active
        .expect("a scheduled turn");
    let other = history
        .world()
        .combatants()
        .iter()
        .map(|(id, _)| id)
        .find(|id| *id != active)
        .expect("a second combatant");
    (active, other)
}

/// Reloads a wrapper from its own bytes, as the round-trip operation does.
fn reload(history: &HistoryWorld) -> HistoryWorld {
    serde_json::from_slice(&snapshot(history)).expect("the wrapper reloads")
}

/// The T2 starting state: fresh, or the near-exhaustion boundary state.
fn start_twin(seed: u64, loaded: bool) -> HistoryWorld {
    if !loaded {
        return HistoryWorld::new(seed);
    }
    let mut value: serde_json::Value =
        serde_json::to_value(HistoryWorld::new(seed)).expect("the wrapper serializes");
    value["acknowledged"] = serde_json::Value::from(u64::MAX - 4);
    value["next_seq"] = serde_json::Value::from(u64::MAX - 3);
    let history: HistoryWorld = serde_json::from_value(value).expect("the boundary state loads");
    assert_j1(&history);
    history
}

/// The cap base with its sequence coordinates rewritten to end at `next_seq`.
fn renumbered(base: &HistoryWorld, next_seq: u64) -> HistoryWorld {
    let mut value: serde_json::Value = serde_json::to_value(base).expect("the base serializes");
    let acknowledged = next_seq - 1 - base.pending_len() as u64;
    value["acknowledged"] = serde_json::Value::from(acknowledged);
    value["next_seq"] = serde_json::Value::from(next_seq);
    for index in 0..base.pending_len() {
        value["pending"][index]["seq"] = serde_json::Value::from(acknowledged + 1 + index as u64);
    }
    serde_json::from_value(value).expect("the renumbered base loads")
}

/// One generated differential step, resolved against live state when applied.
#[derive(Debug, Clone)]
enum Op {
    Spawn,
    DespawnLive(usize),
    DespawnStale(usize),
    Tick,
    StartEncounter,
    UseAbility { actor: usize, target: usize },
    EndTurn(usize),
    EndEncounter,
    AckAhead(usize),
    RoundTrip,
}

/// The T2/T3 operation alphabet.
fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => Just(Op::Spawn),
        2 => (0usize..8).prop_map(Op::DespawnLive),
        2 => (0usize..8).prop_map(Op::DespawnStale),
        2 => Just(Op::Tick),
        1 => Just(Op::StartEncounter),
        3 => (0usize..8, 0usize..8).prop_map(|(actor, target)| Op::UseAbility { actor, target }),
        1 => (0usize..8).prop_map(Op::EndTurn),
        1 => Just(Op::EndEncounter),
        2 => (0usize..8).prop_map(Op::AckAhead),
        2 => Just(Op::RoundTrip),
    ]
}

/// The authored fixture plus the state a differential step resolves against.
struct Driver {
    fixture: Fixture,
    retired: Vec<EntityId>,
}

impl Driver {
    fn new() -> Self {
        Self {
            fixture: Fixture::standard(),
            retired: Vec::new(),
        }
    }

    /// An id the twins have retired, or one no world ever minted.
    fn stale_id(&self, index: usize) -> EntityId {
        if self.retired.is_empty() {
            absent_id()
        } else {
            self.retired[index % self.retired.len()]
        }
    }

    /// `acknowledged + k` for `k` in `0..=pending_len + 1`, saturating at the
    /// sequence ceiling the loaded boundary state sits beside.
    fn ack_target(&self, history: &HistoryWorld, k: usize) -> u64 {
        let bound = history.pending_len() + 2;
        history.acknowledged().saturating_add((k % bound) as u64)
    }
}

/// Runs one step on both twins and asserts result, bytes, hash, and J1 agree.
fn compare_step(op: &Op, oracle: &mut HistoryWorld, fresh: &mut HistoryWorld, driver: &mut Driver) {
    match op {
        Op::Spawn => {
            let a = reference::spawn(oracle, EntityMeta {});
            let b = fresh.spawn(EntityMeta {});
            assert_eq!(a, b, "spawn diverged");
        }
        Op::DespawnLive(index) => {
            let id = live_id(fresh, *index);
            let removed = fresh.despawn(id);
            assert_eq!(reference::despawn(oracle, id), removed, "despawn diverged");
            if removed == Ok(true) {
                driver.retired.push(id);
            }
        }
        Op::DespawnStale(index) => {
            let id = driver.stale_id(*index);
            let a = reference::despawn(oracle, id);
            let b = fresh.despawn(id);
            assert_eq!(a, b, "stale despawn diverged");
        }
        Op::Tick => {
            let a = reference::tick(oracle);
            let b = fresh.tick();
            assert_eq!(a, b, "tick diverged");
        }
        Op::StartEncounter => {
            let bundle = driver.fixture.bundle();
            let spec = spec_for(&driver.fixture, &bundle);
            let a = reference::start_encounter(oracle, &spec);
            let b = fresh.start_encounter(&spec);
            assert_eq!(a, b, "start_encounter diverged");
        }
        Op::UseAbility { actor, target } => {
            let action = CombatAction::UseAbility {
                actor: live_id(fresh, *actor),
                ability: driver.fixture.ability,
                target: live_id(fresh, *target),
            };
            let a = reference::perform_action(oracle, &action);
            let b = fresh.perform_action(&action);
            assert_eq!(a, b, "use-ability diverged");
        }
        Op::EndTurn(index) => {
            // Keep both reaching later turns (active actor) and exercising the
            // rejected EndTurn path (a random live id), chosen by the index's
            // low bit, as the manager's attempt-3 note allows.
            let actor = if index % 2 == 0 {
                live_id(fresh, *index)
            } else {
                active_or_live(fresh, *index)
            };
            let action = CombatAction::EndTurn { actor };
            let a = reference::perform_action(oracle, &action);
            let b = fresh.perform_action(&action);
            assert_eq!(a, b, "end-turn diverged");
        }
        Op::EndEncounter => {
            let a = reference::end_encounter(oracle);
            let b = fresh.end_encounter();
            assert_eq!(a, b, "end_encounter diverged");
        }
        Op::AckAhead(k) => {
            let through = driver.ack_target(fresh, *k);
            let a = reference::acknowledge(oracle, through);
            let b = fresh.acknowledge(through);
            assert_eq!(a, b, "acknowledge diverged");
        }
        Op::RoundTrip => {
            *fresh = reload(fresh);
            *oracle = reload(oracle);
        }
    }
    assert_same(oracle, fresh, &format!("{op:?}"));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// T2: free-play differential, oracle twin against new-path twin.
    #[test]
    fn free_play_matches_the_oracle(
        seed in any::<u64>(),
        loaded in any::<bool>(),
        ops in proptest::collection::vec(op_strategy(), 0..=48),
    ) {
        let mut oracle = start_twin(seed, loaded);
        let mut fresh = oracle.clone();
        let mut driver = Driver::new();
        for op in &ops {
            compare_step(op, &mut oracle, &mut fresh, &mut driver);
        }
        assert_same(&oracle, &fresh, "the whole case");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    /// T3: differential at the cap, from one base built by the new path.
    #[test]
    fn at_the_cap_matches_the_oracle(
        ack in 0usize..=4,
        ops in proptest::collection::vec(op_strategy(), 0..=6),
    ) {
        let mut fresh = cap_base();
        let mut oracle = fresh.clone();
        let mut driver = Driver::new();
        let through = fresh.acknowledged() + ack as u64;
        let a = reference::acknowledge(&mut oracle, through);
        let b = fresh.acknowledge(through);
        assert_eq!(a, b, "the cap acknowledgement diverged");
        assert_same(&oracle, &fresh, "the cap acknowledgement");
        for op in &ops {
            compare_step(op, &mut oracle, &mut fresh, &mut driver);
        }
        assert_same(&oracle, &fresh, "the whole cap case");
    }
}

/// T3d: directed cap cases, oracle against new path, bytes and hash compared.
#[test]
fn directed_cap_cases() {
    let base = cap_base();

    // (a) A spawn at the cap fails as EventLimit.
    let mut oracle = base.clone();
    let mut fresh = base.clone();
    let a = reference::spawn(&mut oracle, EntityMeta {});
    let b = fresh.spawn(EntityMeta {});
    assert_eq!(a, b, "(a) diverged");
    assert_eq!(a, Err(HistoryError::EventLimit));
    assert_same(&oracle, &fresh, "(a)");

    // (b) The active actor's attack at the cap fails as EventLimit.
    let mut oracle = base.clone();
    let mut fresh = base.clone();
    let (actor, target) = active_pair(&fresh);
    let action = CombatAction::UseAbility {
        actor,
        ability: Fixture::standard().ability,
        target,
    };
    let a = reference::perform_action(&mut oracle, &action);
    let b = fresh.perform_action(&action);
    assert_eq!(a, b, "(b) diverged");
    assert_eq!(a, Err(HistoryError::EventLimit));
    assert_same(&oracle, &fresh, "(b)");

    // (c) A release, an acknowledgement down to 4095, then a start: EventLimit.
    let mut oracle = base.clone();
    let mut fresh = base.clone();
    let a = reference::end_encounter(&mut oracle);
    let b = fresh.end_encounter();
    assert_eq!(a, b, "(c) release diverged");
    let a = reference::acknowledge(&mut oracle, 1);
    let b = fresh.acknowledge(1);
    assert_eq!(a, b, "(c) acknowledgement diverged");
    assert_eq!(b, Ok(()));
    assert_eq!(fresh.pending_len(), 4095);
    let fixture = Fixture::standard();
    let bundle = fixture.bundle();
    let spec = spec_for(&fixture, &bundle);
    let a = reference::start_encounter(&mut oracle, &spec);
    let b = fresh.start_encounter(&spec);
    assert_eq!(a, b, "(c) start diverged");
    assert_eq!(a, Err(HistoryError::EventLimit));
    assert_same(&oracle, &fresh, "(c)");

    // (d) The sequence range is checked before the event count.
    let mut oracle = renumbered(&base, u64::MAX);
    let mut fresh = oracle.clone();
    let a = reference::spawn(&mut oracle, EntityMeta {});
    let b = fresh.spawn(EntityMeta {});
    assert_eq!(a, b, "(d) diverged");
    assert_eq!(a, Err(HistoryError::SequenceExhausted));
    assert_same(&oracle, &fresh, "(d)");

    // (e) One lower: the last sequence fits, the event count does not.
    let mut oracle = renumbered(&base, u64::MAX - 1);
    let mut fresh = oracle.clone();
    let a = reference::spawn(&mut oracle, EntityMeta {});
    let b = fresh.spawn(EntityMeta {});
    assert_eq!(a, b, "(e) diverged");
    assert_eq!(a, Err(HistoryError::EventLimit));
    assert_same(&oracle, &fresh, "(e)");
}

/// B3/B4: the check order inside `commit`, and that failure writes nothing.
#[test]
fn commit_check_order_and_rollback() {
    let (history, _) = started(1);
    let before = snapshot(&history);
    let tick = history.world().tick();
    let mut capped = history.clone();
    while capped.pending_len() < MAX_HISTORY_EVENTS {
        capped.spawn(EntityMeta {}).expect("the journal fills");
    }
    let before_cap = snapshot(&capped);

    // An over-long outcome wins over the event-count stage that follows it.
    let oversized = HistoryEvent::ActionResolved {
        actor: live_id(&capped, 0),
        target: live_id(&capped, 1),
        ability: Fixture::standard().ability,
        outcome: "x".repeat(MAX_HISTORY_STRING_BYTES + 1),
        damage: 0,
    };
    let world = capped.world.clone();
    assert_eq!(
        capped.commit(world, vec![oversized], tick),
        Err(HistoryError::StringLimit)
    );
    assert_eq!(snapshot(&capped), before_cap, "nothing was committed");

    // A full journal's event-count stage runs before the byte stage.
    capped.pending.bytes = MAX_HISTORY_BYTES;
    let spawned = HistoryEvent::Spawned {
        entity: live_id(&capped, 0),
    };
    let world = capped.world.clone();
    assert_eq!(
        capped.commit(world, vec![spawned], tick),
        Err(HistoryError::EventLimit)
    );

    // The rejected commits left the world, journal and cache untouched.
    assert_eq!(snapshot(&history), before);
    assert_j1(&history);
    assert_eq!(snapshot(&capped), before_cap);
    assert_eq!(capped.pending.len(), MAX_HISTORY_EVENTS);
}

/// T4: per-operation encoding counts on single-event and zero-event paths.
#[test]
fn encoding_counts_for_single_event_and_zero_event_operations() {
    let fixture = Fixture::standard();
    let bundle = fixture.bundle();
    let spec = spec_for(&fixture, &bundle);
    let mut history = HistoryWorld::new(1);
    let started = expect_encodings(3, || history.start_encounter(&spec));
    started.expect("the fixture starts");

    // An accepted spawn encodes exactly its one new envelope.
    let spawned = expect_encodings(1, || history.spawn(EntityMeta {}));
    assert!(spawned.is_ok());

    // Zero-event operations never encode anything.
    assert!(expect_encodings(0, || history.tick()).is_ok());
    assert!(expect_encodings(0, || history.end_encounter()).is_ok());
    let absent = expect_encodings(0, || history.despawn(absent_id()));
    assert_eq!(absent, Ok(false), "an absent despawn journals nothing");
    let bytes = expect_encodings(0, || history.pending_bytes());
    assert_eq!(
        bytes, history.pending.bytes,
        "pending_bytes reads the cache"
    );

    // Draining the whole journal resets the cache without encoding it.
    let through = history.last_sequence();
    assert!(expect_encodings(0, || history.acknowledge(through)).is_ok());
    assert_eq!(history.pending_bytes(), 0);
    assert_j1(&history);

    // A non-emptying acknowledgement encodes exactly the drained prefix.
    let mut journal = HistoryWorld::new(0);
    for _ in 0..MAX_HISTORY_EVENTS {
        journal.spawn(EntityMeta {}).expect("the journal fills");
    }
    assert!(expect_encodings(3, || journal.acknowledge(3)).is_ok());
    assert_eq!(journal.pending_len(), MAX_HISTORY_EVENTS - 3);
    assert_j1(&journal);

    // A 4095-envelope journal still accepts its last envelope for one
    // encoding, and draining a full journal never re-encodes it.
    let mut almost = HistoryWorld::new(0);
    for _ in 0..MAX_HISTORY_EVENTS - 1 {
        almost.spawn(EntityMeta {}).expect("the journal fills");
    }
    let last = expect_encodings(1, || almost.spawn(EntityMeta {}));
    assert!(last.is_ok());
    let through = almost.last_sequence();
    assert!(expect_encodings(0, || almost.acknowledge(through)).is_ok());
    assert_j1(&almost);

    // Building a 4096-envelope base is 4096 encodings; the cap is free.
    let mut full = HistoryWorld::new(0);
    let (_, encoded) = encodings_of(|| {
        for _ in 0..MAX_HISTORY_EVENTS {
            full.spawn(EntityMeta {}).expect("the journal fills");
        }
    });
    assert_eq!(encoded, MAX_HISTORY_EVENTS as u64, "one per spawn");
    let refused = expect_encodings(0, || full.spawn(EntityMeta {}));
    assert_eq!(refused, Err(HistoryError::EventLimit));
    assert_j1(&full);
}

/// T4: an accepted action's encoding count equals its journal growth.
#[test]
fn encoding_count_for_actions_matches_journal_growth() {
    let fixture = Fixture::standard();
    let mut history = HistoryWorld::new(1);
    let bundle = fixture.bundle();
    let spec = spec_for(&fixture, &bundle);
    history.start_encounter(&spec).expect("the fixture starts");
    let (actor, target) = active_pair(&history);
    let action = CombatAction::UseAbility {
        actor,
        ability: fixture.ability,
        target,
    };
    let len_before = history.pending_len();
    let (outcome, encoded) = encodings_of(|| history.perform_action(&action));
    outcome.expect("accepted").expect("attack");
    let growth = history.pending_len() - len_before;
    assert!(
        (2..=3).contains(&growth),
        "two or three envelopes: {growth}"
    );
    assert_eq!(encoded, growth as u64, "one encoding per new envelope");

    // At the cap, acknowledged down to 4090 pending.
    let mut full = cap_base();
    full.acknowledge(6).expect("drain six");
    assert_eq!(full.pending_len(), MAX_HISTORY_EVENTS - 6);
    let (actor, target) = active_pair(&full);
    let action = CombatAction::UseAbility {
        actor,
        ability: fixture.ability,
        target,
    };
    let len_before = full.pending_len();
    let (outcome, encoded) = encodings_of(|| full.perform_action(&action));
    outcome.expect("accepted").expect("attack");
    let growth = full.pending_len() - len_before;
    assert!(
        (2..=3).contains(&growth),
        "two or three envelopes: {growth}"
    );
    assert_eq!(encoded, growth as u64, "one encoding per new envelope");
    assert_j1(&full);
}

/// T5: inert and rejected operations leave the journal in place.
#[test]
fn inert_and_rejected_operations_leave_the_journal_in_place() {
    let (mut history, ids) = started(3);
    let mut pointer = history.pending.as_slice().as_ptr();
    let mut capacity = history.pending.envelopes.capacity();
    history.tick().expect("tick");
    assert_journal_in_place(&history, pointer, capacity, "tick");
    history.end_encounter().expect("release");
    assert_journal_in_place(&history, pointer, capacity, "end_encounter");
    assert!(history.despawn(ids[0]).expect("despawn the live id"));
    pointer = history.pending.as_slice().as_ptr();
    capacity = history.pending.envelopes.capacity();
    assert!(!history.despawn(ids[0]).expect("the id is already dead"));
    assert_journal_in_place(&history, pointer, capacity, "an absent despawn");

    // A rejection at the cap leaves the full journal in place too.
    let mut full = cap_base();
    let pointer = full.pending.as_slice().as_ptr();
    let capacity = full.pending.envelopes.capacity();
    assert_eq!(full.spawn(EntityMeta {}), Err(HistoryError::EventLimit));
    assert_journal_in_place(&full, pointer, capacity, "an EventLimit spawn");
}

/// The canonical length of the envelope the next spawn would append.
fn next_spawn_len(history: &HistoryWorld) -> usize {
    let mut probe = history.clone();
    probe.spawn(EntityMeta {}).expect("probe");
    let produced = probe
        .read_after(0, 256)
        .expect("read")
        .pop()
        .expect("the probe journaled one envelope");
    envelope_len(&produced)
}

/// T6: `ByteLimit` rolls back, and the `>` boundary is exact.
#[test]
fn byte_limit_rejects_and_rolls_back() {
    // A deliberately off-invariant cache: one byte above the budget once the
    // next envelope lands.
    let mut history = HistoryWorld::new(0);
    let before = snapshot(&history);
    let pointer = history.pending.as_slice().as_ptr();
    let encoded = next_spawn_len(&history);
    history.pending.bytes = MAX_HISTORY_BYTES - encoded + 1;
    assert_eq!(history.spawn(EntityMeta {}), Err(HistoryError::ByteLimit));
    assert_eq!(snapshot(&history), before, "the rejection commits nothing");
    assert_eq!(history.world().len(), 0, "no entity was allocated");
    assert!(history.pending.as_slice().is_empty());
    assert_eq!(
        history.pending.bytes,
        MAX_HISTORY_BYTES - encoded + 1,
        "the rejected append left the cache alone"
    );
    assert!(
        std::ptr::eq(history.pending.as_slice().as_ptr(), pointer),
        "the rejection copied the journal"
    );

    // Exactly at the budget the same append fits (this cache is deliberately
    // off-invariant, so J1 is not asserted on it).
    let mut fits = HistoryWorld::new(0);
    let encoded = next_spawn_len(&fits);
    fits.pending.bytes = MAX_HISTORY_BYTES - encoded;
    fits.spawn(EntityMeta {}).expect("the last byte fits");
    assert_eq!(fits.pending_len(), 1);
    assert_eq!(fits.pending_bytes(), MAX_HISTORY_BYTES);
}

/// T7: the journal's `Debug` output is the envelope sequence's.
#[test]
fn journal_debug_matches_the_envelope_sequence() {
    let (history, _) = started(1);
    let journal = &history.pending;
    assert_eq!(journal.len(), 3, "the fixture journaled three envelopes");
    assert_eq!(format!("{journal:?}"), format!("{:?}", journal.envelopes));
    assert!(
        format!("{history:?}").contains(&format!("{:?}", journal.envelopes)),
        "the wrapper debug still prints the envelopes"
    );
}

/// T8: a deterministic source guard over the envelope encoding sites.
#[test]
fn every_envelope_encoding_site_is_accounted_for() {
    let source = include_str!("../history.rs");
    let mut sites: Vec<&str> = source
        .match_indices("serde_json::to_")
        .map(|(offset, _)| enclosing_fn(source, offset))
        .collect();
    assert_eq!(sites.len(), 3, "exactly three encoding sites may exist");
    sites.sort_unstable();
    assert_eq!(
        sites,
        ["envelope_len", "history_hash", "visit_seq"],
        "every encoding site sits in its reviewed function"
    );
    assert!(
        source.contains("serde_json::to_vec(world).expect("),
        "history_hash's encoding site moved"
    );
    assert!(
        source.contains("thread_local! {") && !source.contains("std::thread_local"),
        "the counter must be a bare `thread_local!`"
    );
}

/// The name of the function body containing `source[offset]`.
fn enclosing_fn(source: &str, offset: usize) -> &str {
    source[..offset]
        .lines()
        .rev()
        .find_map(|line| line.split_once("fn "))
        .map(|(_, rest)| {
            rest.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .unwrap_or(rest)
        })
        .expect("every encoding site sits inside a function")
}
