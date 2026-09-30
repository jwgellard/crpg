//! Opt-in authoritative history acceptance (T020, ADR-0017).
//!
//! Public-API semantic oracles, independent of the producer logic: every
//! ordering assertion below hand-authors the exact expected payload sequence
//! with concrete identities, rounds, outcomes, and damage, and the oracle
//! itself is shown to fail on omission, duplication, and reordering. Atomic
//! rejection and capacity rollback compare complete wrapper bytes and hashes
//! before and after; read/ack boundaries, malformed loads, sequence
//! exhaustion, and checkpoint continuation go through the public surface and
//! valid serialized boundary states only — no production counter setter.
//! Legacy compatibility is pinned by byte-identical wrapper/legacy world
//! comparison plus the untouched legacy suites.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Ulid};
use crpg_data::{DamageEntry, OutcomeBandWire, OutcomeWire};
use crpg_sim::{
    history_hash, state_hash, CombatAction, CombatError, EncounterSpec, EntityMeta, HistoryError,
    HistoryEvent, HistoryWorld, World,
};
use support::Fixture;

/// Complete canonical wrapper bytes for rollback comparisons.
fn snapshot(history: &HistoryWorld) -> Vec<u8> {
    serde_json::to_vec(history).expect("the wrapper serializes")
}

/// Assembles the borrowed encounter spec one wrapper operation consumes.
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

/// Starts the fixture encounter in a fresh wrapper, returning combatant
/// entities in authored participant order.
fn start_history(fixture: &Fixture, seed: u64) -> (HistoryWorld, Vec<EntityId>) {
    let bundle = fixture.bundle();
    let spec = spec_for(fixture, &bundle);
    let mut history = HistoryWorld::new(seed);
    history.start_encounter(&spec).expect("the fixture starts");
    assert!(
        history.world().events().is_empty(),
        "the inner queue stays empty at wrapper boundaries"
    );
    let ids: Vec<EntityId> = history
        .world()
        .combatants()
        .iter()
        .map(|(id, _)| id)
        .collect();
    (history, ids)
}

/// The retained payload sequence, oldest first.
fn payloads(history: &HistoryWorld) -> Vec<HistoryEvent> {
    history
        .read_after(history.acknowledged(), 256)
        .expect("a full read fits one page")
        .into_iter()
        .map(|envelope| envelope.payload)
        .collect()
}

/// The standard encounter identity both fixtures share.
fn encounter_id() -> Ulid {
    Ulid::from_u128(101)
}

/// The standard outcome-table identity.
fn table_id() -> Ulid {
    Ulid::from_u128(104)
}

/// The standard fixture with lethal always-successful damage.
fn lethal_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    fixture
        .abilities
        .get_mut(&fixture.ability)
        .expect("ability")
        .damage = vec![
        DamageEntry {
            outcome: OutcomeWire::Success,
            amount: 99,
        },
        DamageEntry {
            outcome: OutcomeWire::Failure,
            amount: 0,
        },
    ];
    fixture.tables.get_mut(&table_id()).expect("table").bands = vec![OutcomeBandWire {
        min_margin: i64::MIN,
        outcome: OutcomeWire::Success,
    }];
    fixture
}

/// The standard fixture with an always-failing table.
fn failing_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    fixture.tables.get_mut(&table_id()).expect("table").bands = vec![OutcomeBandWire {
        min_margin: i64::MIN,
        outcome: OutcomeWire::Failure,
    }];
    fixture
}

/// The current balance of one combatant's primary pool.
fn pool_current(history: &HistoryWorld, combatant: EntityId) -> u32 {
    history
        .world()
        .combatants()
        .get(combatant)
        .expect("combatant")
        .action_pool()
        .current()
}

#[test]
fn action_death_turn_order() {
    let fixture = lethal_fixture();
    let (mut history, ids) = start_history(&fixture, 3);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        history.world().combat().expect("combat").active,
        Some(b),
        "B holds the first turn"
    );
    let outcome = history
        .perform_action(&CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        })
        .expect("the lethal attack is accepted")
        .expect("attack");
    assert_eq!(outcome.outcome, crpg_rules::Outcome::Success);
    assert_eq!(outcome.damage, 99);
    assert!(outcome.target_died);
    // Hand-authored oracle: spawns in authored order, the initial turn, then
    // resolution, death in drain order, and the natural end — never inferred.
    let expected = vec![
        HistoryEvent::Spawned { entity: a },
        HistoryEvent::Spawned { entity: b },
        HistoryEvent::TurnStarted { actor: b, round: 0 },
        HistoryEvent::ActionResolved {
            actor: b,
            target: a,
            ability: fixture.ability,
            outcome: "success".to_owned(),
            damage: 99,
        },
        HistoryEvent::Died { entity: a },
        HistoryEvent::EncounterEnded {
            encounter: encounter_id(),
            round: 0,
        },
    ];
    assert_eq!(payloads(&history), expected);
    // The oracle is order- and content-sensitive.
    let mut omitted = expected.clone();
    omitted.remove(3);
    assert_ne!(payloads(&history), omitted, "omission must fail the oracle");
    let mut duplicated = expected.clone();
    duplicated.insert(3, expected[3].clone());
    assert_ne!(
        payloads(&history),
        duplicated,
        "duplication must fail the oracle"
    );
    let mut swapped = expected.clone();
    swapped.swap(3, 4);
    assert_ne!(
        payloads(&history),
        swapped,
        "reordering must fail the oracle"
    );
    // Coordinates: contiguous sequences from one, all at tick zero.
    let journal = history.read_after(0, 256).expect("read");
    for (index, envelope) in journal.iter().enumerate() {
        assert_eq!(envelope.seq, index as u64 + 1);
        assert_eq!(envelope.tick.get(), 0);
    }
    assert!(history.world().events().is_empty());
    assert_eq!(
        history.world().combat().expect("combat").active,
        None,
        "the encounter is terminal"
    );
}

#[test]
fn failed_attack_is_history() {
    // Seed 1 opens [6, 3]: total 9 against alpha 6, a legal failure.
    let fixture = Fixture::standard();
    let (mut history, ids) = start_history(&fixture, 1);
    let (a, b) = (ids[0], ids[1]);
    let outcome = history
        .perform_action(&CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        })
        .expect("the failed attack is accepted")
        .expect("attack");
    assert_eq!(outcome.outcome, crpg_rules::Outcome::Failure);
    assert_eq!(outcome.damage, 0);
    assert!(!outcome.target_died);
    assert_eq!(
        payloads(&history),
        vec![
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
            HistoryEvent::TurnStarted { actor: b, round: 0 },
            HistoryEvent::ActionResolved {
                actor: b,
                target: a,
                ability: fixture.ability,
                outcome: "failure".to_owned(),
                damage: 0,
            },
            HistoryEvent::TurnStarted { actor: a, round: 0 },
        ]
    );
    assert_eq!(pool_current(&history, b), 0, "failure still spends");
    assert_eq!(
        history.world().combat().expect("combat").active,
        Some(a),
        "failure still consumes the turn"
    );
}

#[test]
fn nonending_action_has_no_turn() {
    let mut fixture = failing_fixture();
    fixture
        .abilities
        .get_mut(&fixture.ability)
        .expect("ability")
        .ends_turn = false;
    let (mut history, ids) = start_history(&fixture, 9);
    let (a, b) = (ids[0], ids[1]);
    history
        .perform_action(&CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        })
        .expect("the non-ending attack is accepted")
        .expect("attack");
    // No turn was invented: resolution only, and the actor keeps the turn.
    assert_eq!(
        payloads(&history),
        vec![
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
            HistoryEvent::TurnStarted { actor: b, round: 0 },
            HistoryEvent::ActionResolved {
                actor: b,
                target: a,
                ability: fixture.ability,
                outcome: "failure".to_owned(),
                damage: 0,
            },
        ]
    );
    assert_eq!(history.world().combat().expect("combat").active, Some(b));
    let ended = history
        .perform_action(&CombatAction::EndTurn { actor: b })
        .expect("end turn advances");
    assert_eq!(ended, None, "EndTurn resolves nothing");
    assert_eq!(
        payloads(&history)[4],
        HistoryEvent::TurnStarted { actor: a, round: 0 }
    );
}

#[test]
fn logical_turn_at_round_saturation() {
    let fixture = Fixture::standard();
    let (started, ids) = start_history(&fixture, 7);
    let (a, b) = (ids[0], ids[1]);
    // A valid serialized boundary state: the persisted round at its maximum.
    let mut value: serde_json::Value =
        serde_json::from_slice(&snapshot(&started)).expect("wrapper serializes");
    value["world"]["combat"]["round"] = serde_json::Value::from(u32::MAX);
    let mut history: HistoryWorld =
        serde_json::from_value(value).expect("the saturated round loads");
    assert_eq!(
        history.world().combat().expect("combat").round,
        u32::MAX,
        "the persisted counter keeps its u32 type"
    );
    // B acts last-but-one: no rollover, but the turn still starts at the max.
    history
        .perform_action(&CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        })
        .expect("accepted at saturation")
        .expect("attack");
    // A acts last: the timeline empties, the round saturates instead of
    // wrapping, and the logical turn still starts at the repeated number.
    history
        .perform_action(&CombatAction::UseAbility {
            actor: a,
            ability: fixture.ability,
            target: b,
        })
        .expect("accepted at saturation")
        .expect("attack");
    let tail = &payloads(&history)[3..];
    assert_eq!(tail.len(), 4, "two resolutions plus two turn starts");
    assert!(
        matches!(
            tail[1],
            HistoryEvent::TurnStarted { round, .. } if round == u64::from(u32::MAX)
        ),
        "first turn starts at the saturated round: {tail:?}"
    );
    assert!(
        matches!(
            tail[3],
            HistoryEvent::TurnStarted { round, .. } if round == u64::from(u32::MAX)
        ),
        "rollover still starts a logical turn at the repeated round: {tail:?}"
    );
    assert_eq!(
        history.world().combat().expect("combat").round,
        u32::MAX,
        "saturation semantics are unchanged"
    );
    assert!(history.world().events().is_empty());
}

#[test]
fn despawn_active_and_nonactive() {
    // Trio order: A(10), B(9), C(8), so C holds the first turn.
    let fixture = Fixture::trio();
    let (mut history, ids) = start_history(&fixture, 3);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    assert_eq!(history.world().combat().expect("combat").active, Some(c));
    // Removing the active combatant advances through the shared rule.
    assert!(history.despawn(c).expect("despawn"));
    // Removing any other combatant leaves the turn untouched.
    assert!(history.despawn(a).expect("despawn"));
    assert_eq!(
        payloads(&history),
        vec![
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
            HistoryEvent::Spawned { entity: c },
            HistoryEvent::TurnStarted { actor: c, round: 0 },
            HistoryEvent::Despawned { entity: c },
            HistoryEvent::TurnStarted { actor: b, round: 0 },
            HistoryEvent::Despawned { entity: a },
        ]
    );
    assert_eq!(history.world().combat().expect("combat").active, Some(b));
    // A dead id returns false and journals nothing.
    let len = history.pending_len();
    assert!(!history.despawn(c).expect("absent"));
    assert_eq!(history.pending_len(), len);
}

#[test]
fn last_removal_and_release_do_not_double_end() {
    let fixture = lethal_fixture();
    let (mut history, ids) = start_history(&fixture, 3);
    let (a, b) = (ids[0], ids[1]);
    history
        .perform_action(&CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        })
        .expect("the kill is accepted")
        .expect("attack");
    assert_eq!(
        history.world().combat().expect("combat").active,
        None,
        "the kill is naturally terminal"
    );
    // Removing the dead non-active participant: despawn only.
    assert!(history.despawn(a).expect("despawn"));
    assert!(history.world().combat().is_some());
    // Removing the last participant of an already terminal encounter
    // releases it with no second end.
    assert!(history.despawn(b).expect("despawn"));
    assert!(history.world().combat().is_none());
    let journal = payloads(&history);
    assert_eq!(journal.len(), 3 + 3 + 1 + 1);
    let ends = journal
        .iter()
        .filter(|event| matches!(event, HistoryEvent::EncounterEnded { .. }))
        .count();
    assert_eq!(ends, 1, "exactly one natural end: {journal:?}");
    // Explicit release of the released encounter fails without mutation.
    let before = snapshot(&history);
    assert_eq!(
        history.end_encounter(),
        Err(HistoryError::Combat(CombatError::NoEncounter))
    );
    assert_eq!(snapshot(&history), before);
}

#[test]
fn capacity_failure_rolls_back_rng() {
    // A full journal blocks an emitting spawn with complete rollback.
    let mut history = HistoryWorld::new(0);
    for _ in 0..4096 {
        history.spawn(EntityMeta {}).expect("the journal fills");
    }
    assert_eq!(history.pending_len(), 4096);
    let before = snapshot(&history);
    let before_hash = history_hash(&history);
    assert_eq!(history.spawn(EntityMeta {}), Err(HistoryError::EventLimit));
    assert_eq!(snapshot(&history), before);
    assert_eq!(history_hash(&history), before_hash);
    assert_eq!(history.last_sequence(), 4096);
    // A multi-event operation at 4095 rolls back entity allocation too.
    let mut history = HistoryWorld::new(0);
    for _ in 0..4095 {
        history.spawn(EntityMeta {}).expect("the journal fills");
    }
    let fixture = Fixture::standard();
    let bundle = fixture.bundle();
    let spec = spec_for(&fixture, &bundle);
    assert_eq!(
        history.start_encounter(&spec),
        Err(HistoryError::EventLimit)
    );
    assert_eq!(history.pending_len(), 4095);
    assert_eq!(history.world().len(), 4095, "no entity was allocated");
    // A valid attack at a full journal rolls back RNG draws as well: the
    // complete bytes (RNG streams included) are unchanged.
    let fixture = Fixture::standard();
    let (mut history, ids) = start_history(&fixture, 1);
    let (a, b) = (ids[0], ids[1]);
    for _ in 0..4093 {
        history.spawn(EntityMeta {}).expect("the journal fills");
    }
    assert_eq!(history.pending_len(), 4096);
    let before = snapshot(&history);
    let before_hash = history_hash(&history);
    assert_eq!(
        history.perform_action(&CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        }),
        Err(HistoryError::EventLimit)
    );
    assert_eq!(snapshot(&history), before);
    assert_eq!(history_hash(&history), before_hash);
    // Zero-event operations need no free journal slot.
    history.tick().expect("tick succeeds when full");
    assert_eq!(history.pending_len(), 4096);
    assert_eq!(history.world().tick().get(), 1);
}

#[test]
fn rejected_actions_commit_nothing() {
    let fixture = Fixture::standard();
    let (mut history, ids) = start_history(&fixture, 1);
    let (a, b) = (ids[0], ids[1]);
    let before = snapshot(&history);
    let before_hash = history_hash(&history);
    // A holds no turn: the gameplay error wins before history validation.
    assert_eq!(
        history.perform_action(&CombatAction::UseAbility {
            actor: a,
            ability: fixture.ability,
            target: b,
        }),
        Err(HistoryError::Combat(CombatError::OutOfTurn {
            actor: a,
            active: Some(b),
        }))
    );
    assert_eq!(snapshot(&history), before);
    assert_eq!(history_hash(&history), before_hash);
    // A fresh wrapper has no encounter to act in or release.
    let mut fresh = HistoryWorld::new(0);
    assert_eq!(
        fresh.perform_action(&CombatAction::EndTurn { actor: a }),
        Err(HistoryError::Combat(CombatError::NoEncounter))
    );
    assert_eq!(
        fresh.end_encounter(),
        Err(HistoryError::Combat(CombatError::NoEncounter))
    );
    assert_eq!(fresh.pending_len(), 0);
    assert!(fresh.world().events().is_empty());
}

#[test]
fn read_ack_boundaries() {
    let mut history = HistoryWorld::new(0);
    assert_eq!(history.acknowledged(), 0);
    assert_eq!(history.last_sequence(), 0);
    assert_eq!(history.read_after(0, 10).expect("empty read"), Vec::new());
    assert_eq!(
        history.read_after(0, 0),
        Err(HistoryError::InvalidPageLimit { limit: 0 })
    );
    assert_eq!(
        history.read_after(0, 257),
        Err(HistoryError::InvalidPageLimit { limit: 257 })
    );
    assert_eq!(
        history.acknowledge(5),
        Err(HistoryError::FutureCursor {
            requested: 5,
            last: 0
        })
    );
    history.acknowledge(0).expect("zero is idempotent");
    let first = history.spawn(EntityMeta {}).expect("spawn");
    let second = history.spawn(EntityMeta {}).expect("spawn");
    let third = history.spawn(EntityMeta {}).expect("spawn");
    assert_eq!(history.last_sequence(), 3);
    // Pages slice the contiguous suffix in order with an exact byte account.
    let page = history.read_after(0, 2).expect("page");
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].seq, 1);
    assert_eq!(page[1].seq, 2);
    let tail = history.read_after(2, 256).expect("tail");
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].seq, 3);
    assert_eq!(
        history.read_after(3, 1).expect("at last"),
        Vec::new(),
        "after == last returns empty"
    );
    assert_eq!(
        history.read_after(4, 1),
        Err(HistoryError::FutureCursor {
            requested: 4,
            last: 3
        })
    );
    let manual: usize = history
        .read_after(0, 256)
        .expect("full")
        .iter()
        .map(|envelope| serde_json::to_vec(envelope).expect("serializes").len())
        .sum();
    assert_eq!(history.pending_bytes(), manual);
    // Acknowledgements retire the prefix; the last sequence survives them.
    history.acknowledge(2).expect("advance");
    assert_eq!(history.acknowledged(), 2);
    assert_eq!(history.last_sequence(), 3);
    assert_eq!(history.pending_len(), 1);
    assert_eq!(
        history.read_after(0, 10),
        Err(HistoryError::StaleCursor {
            requested: 0,
            acknowledged: 2
        })
    );
    assert_eq!(
        history.read_after(1, 10),
        Err(HistoryError::StaleCursor {
            requested: 1,
            acknowledged: 2
        })
    );
    let rest = history.read_after(2, 10).expect("remainder");
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].seq, 3);
    history.acknowledge(2).expect("idempotent, never stale");
    assert_eq!(
        history.acknowledge(9),
        Err(HistoryError::FutureCursor {
            requested: 9,
            last: 3
        })
    );
    history.acknowledge(3).expect("drain");
    assert_eq!(history.pending_len(), 0);
    assert_eq!(history.last_sequence(), 3);
    assert_eq!(
        history.read_after(3, 10).expect("empty at last"),
        Vec::new()
    );
    // The journal still names every retired entity in order.
    assert_ne!(first, second);
    assert_ne!(second, third);
    assert!(first.index() < second.index() && second.index() < third.index());
}

/// Error spellings: history variants hide payloads; combat delegates.
#[test]
fn history_error_display() {
    assert_eq!(
        format!("{}", HistoryError::InvalidPageLimit { limit: 0 }),
        "InvalidPageLimit at history"
    );
    assert_eq!(
        format!(
            "{}",
            HistoryError::StaleCursor {
                requested: 0,
                acknowledged: 2
            }
        ),
        "StaleCursor at history"
    );
    assert_eq!(
        format!(
            "{}",
            HistoryError::FutureCursor {
                requested: 9,
                last: 3
            }
        ),
        "FutureCursor at history"
    );
    assert_eq!(
        format!("{}", HistoryError::SequenceExhausted),
        "SequenceExhausted at history"
    );
    assert_eq!(
        format!("{}", HistoryError::EventLimit),
        "EventLimit at history"
    );
    assert_eq!(
        format!("{}", HistoryError::ByteLimit),
        "ByteLimit at history"
    );
    assert_eq!(
        format!("{}", HistoryError::StringLimit),
        "StringLimit at history"
    );
    assert_eq!(
        format!("{}", HistoryError::Combat(CombatError::NoEncounter)),
        "NoEncounter at combat"
    );
    let combat = HistoryError::Combat(CombatError::NoEncounter);
    assert!(
        std::error::Error::source(&combat).is_some(),
        "combat delegates its source"
    );
    assert!(
        std::error::Error::source(&HistoryError::EventLimit).is_none(),
        "history variants carry no source"
    );
}

#[test]
fn sequence_exhaustion_atomic() {
    // A valid serialized boundary state: the exhausted sentinel with an
    // empty journal.
    let fresh = snapshot(&HistoryWorld::new(0));
    let mut value: serde_json::Value = serde_json::from_slice(&fresh).expect("serializes");
    value["acknowledged"] = serde_json::Value::from(u64::MAX - 1);
    value["next_seq"] = serde_json::Value::from(u64::MAX);
    let mut history: HistoryWorld = serde_json::from_value(value).expect("exhaustion loads");
    assert_eq!(history.last_sequence(), u64::MAX - 1);
    let before = snapshot(&history);
    assert_eq!(
        history.spawn(EntityMeta {}),
        Err(HistoryError::SequenceExhausted)
    );
    assert_eq!(snapshot(&history), before, "nothing was reserved");
    let fixture = Fixture::standard();
    let bundle = fixture.bundle();
    let spec = spec_for(&fixture, &bundle);
    assert_eq!(
        history.start_encounter(&spec),
        Err(HistoryError::SequenceExhausted)
    );
    assert_eq!(history.world().len(), 0, "no entity was allocated");
    // Zero-event operations still succeed at exhaustion.
    history.tick().expect("tick succeeds at exhaustion");
    assert_eq!(history.world().tick().get(), 1);
    assert_eq!(history.last_sequence(), u64::MAX - 1);
    assert_eq!(history.pending_len(), 0);
    // One slot below exhaustion: a single event fits, the next does not,
    // and a multi-event range reserves atomically or not at all.
    let fresh = snapshot(&HistoryWorld::new(0));
    let mut value: serde_json::Value = serde_json::from_slice(&fresh).expect("serializes");
    value["acknowledged"] = serde_json::Value::from(u64::MAX - 2);
    value["next_seq"] = serde_json::Value::from(u64::MAX - 1);
    let mut history: HistoryWorld = serde_json::from_value(value).expect("near-end loads");
    history
        .spawn(EntityMeta {})
        .expect("the last sequence issues");
    assert_eq!(history.last_sequence(), u64::MAX - 1);
    assert_eq!(
        history.spawn(EntityMeta {}),
        Err(HistoryError::SequenceExhausted)
    );
    assert_eq!(
        history.start_encounter(&spec),
        Err(HistoryError::SequenceExhausted)
    );
    assert_eq!(history.world().len(), 1, "only the single spawn committed");
}

#[test]
fn checkpoint_continuation() {
    let fixture = Fixture::standard();
    let (mut live, ids) = start_history(&fixture, 1);
    let (a, b) = (ids[0], ids[1]);
    live.perform_action(&CombatAction::UseAbility {
        actor: b,
        ability: fixture.ability,
        target: a,
    })
    .expect("attack")
    .expect("attack");
    live.perform_action(&CombatAction::UseAbility {
        actor: a,
        ability: fixture.ability,
        target: b,
    })
    .expect("attack")
    .expect("attack");
    live.acknowledge(3).expect("ack");
    let checkpoint = snapshot(&live);
    let mut restored: HistoryWorld =
        serde_json::from_slice(&checkpoint).expect("the checkpoint loads");
    assert_eq!(live, restored, "load reconstructs the continuation");
    // Identical subsequent commands: identical events, journal, and hash.
    for history in [&mut live, &mut restored] {
        history
            .perform_action(&CombatAction::EndTurn {
                actor: history
                    .world()
                    .combat()
                    .expect("combat")
                    .active
                    .expect("turn"),
            })
            .expect("end turn");
        history.tick().expect("tick");
        history.acknowledge(7).expect("ack");
    }
    assert_eq!(snapshot(&live), snapshot(&restored));
    assert_eq!(history_hash(&live), history_hash(&restored));
    assert_eq!(live.acknowledged(), restored.acknowledged());
    assert_eq!(payloads(&live), payloads(&restored));
}

#[test]
fn full_hash_field_sensitivity() {
    let base = HistoryWorld::new(0);
    let base_hash = history_hash(&base);
    let mut ticked = base.clone();
    ticked.tick().expect("tick");
    assert_ne!(history_hash(&ticked), base_hash, "the world tick is hashed");
    let mut spawned = base.clone();
    spawned.spawn(EntityMeta {}).expect("spawn");
    assert_ne!(
        history_hash(&spawned),
        base_hash,
        "pending payloads are hashed"
    );
    let mut acked = spawned.clone();
    acked.acknowledge(1).expect("ack");
    assert_ne!(
        history_hash(&acked),
        history_hash(&spawned),
        "acknowledgement state is hashed"
    );
    // Same gameplay with the same ack schedule hashes identically, while a
    // different ack schedule is a different deterministic input.
    let (mut first, _) = start_history(&Fixture::standard(), 1);
    let (mut second, _) = start_history(&Fixture::standard(), 1);
    assert_eq!(history_hash(&first), history_hash(&second));
    first.acknowledge(2).expect("ack");
    assert_ne!(
        history_hash(&first),
        history_hash(&second),
        "the ack schedule is part of the hashed input"
    );
    second.acknowledge(2).expect("ack");
    assert_eq!(history_hash(&first), history_hash(&second));
}

#[test]
fn load_rejects_corrupt_journal() {
    let fixture = Fixture::standard();
    let (mut history, ids) = start_history(&fixture, 1);
    let (a, b) = (ids[0], ids[1]);
    history
        .perform_action(&CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        })
        .expect("failed attack journals a resolution")
        .expect("attack");
    let base = serde_json::to_string(&history).expect("serializes");
    let journal = history.read_after(0, 256).expect("read");
    let payload0 = serde_json::to_string(&journal[0].payload).expect("payload");
    let env0 = serde_json::to_string(&journal[0]).expect("envelope");

    // Positive controls: the base round-trips, type-before-value loads, and
    // all other field orders are free on input.
    let loaded: HistoryWorld = serde_json::from_str(&base).expect("base loads");
    assert_eq!(loaded, history);
    let reordered = format!("{{\"payload\":{payload0},\"seq\":1,\"tick\":0}}");
    let variant = base.replacen(&env0, &reordered, 1);
    assert_ne!(variant, base);
    let loaded: HistoryWorld = serde_json::from_str(&variant).expect("envelope order is free");
    assert_eq!(loaded, history);
    let mut moved = base.clone();
    assert!(moved.starts_with("{\"version\":1,"));
    moved = moved.replacen("{\"version\":1,", "{", 1);
    moved.pop();
    moved.push_str(",\"version\":1}");
    let loaded: HistoryWorld = serde_json::from_str(&moved).expect("wrapper order is free");
    assert_eq!(loaded, history);

    // Wrapper shape faults.
    let mut value: serde_json::Value = serde_json::from_str(&base).expect("parses");
    value["version"] = serde_json::Value::from(2);
    assert!(serde_json::from_value::<HistoryWorld>(value)
        .expect_err("unknown version fails")
        .to_string()
        .contains("unknown history version"));
    let mut value: serde_json::Value = serde_json::from_str(&base).expect("parses");
    value["history_future"] = serde_json::Value::from(1);
    assert!(serde_json::from_value::<HistoryWorld>(value)
        .expect_err("unknown wrapper key fails")
        .to_string()
        .contains("unknown history wrapper field"));
    let mut value: serde_json::Value = serde_json::from_str(&base).expect("parses");
    value.as_object_mut().expect("object").remove("pending");
    assert!(serde_json::from_value::<HistoryWorld>(value)
        .expect_err("missing pending fails")
        .to_string()
        .contains("misses `pending`"));
    let doubled = base.replacen("{\"version\":1,", "{\"version\":1,\"version\":1,", 1);
    assert!(serde_json::from_str::<HistoryWorld>(&doubled)
        .expect_err("duplicate wrapper key fails")
        .to_string()
        .contains("duplicate history wrapper field"));

    // Envelope shape faults: unknown, duplicate, and missing keys fail.
    // A derived `Deserialize` would keep the last duplicate silently; the
    // manual envelope visitor rejects it instead.
    let unknown_env = env0.replacen("\"seq\":", "\"seq_nope\":", 1);
    assert_ne!(unknown_env, env0);
    let swapped_env = base.replacen(&env0, &unknown_env, 1);
    assert!(serde_json::from_str::<HistoryWorld>(&swapped_env)
        .expect_err("unknown envelope key fails")
        .to_string()
        .contains("unknown history envelope field"));
    let doubled_env = env0.replacen("\"payload\"", "\"seq\":1,\"payload\"", 1);
    assert_ne!(doubled_env, env0);
    let swapped_env = base.replacen(&env0, &doubled_env, 1);
    assert!(serde_json::from_str::<HistoryWorld>(&swapped_env)
        .expect_err("duplicate envelope key fails")
        .to_string()
        .contains("duplicate history envelope field"));
    let mut value: serde_json::Value = serde_json::from_str(&base).expect("parses");
    value["pending"][0]
        .as_object_mut()
        .expect("object")
        .remove("seq");
    assert!(serde_json::from_value::<HistoryWorld>(value)
        .expect_err("missing envelope key fails")
        .to_string()
        .contains("misses `seq`"));

    // Payload faults: unknown tags and the type-before-value rule.
    let unknown_tag = base.replacen("\"type\":\"spawned\"", "\"type\":\"ascended\"", 1);
    assert!(serde_json::from_str::<HistoryWorld>(&unknown_tag)
        .expect_err("unknown tag fails")
        .to_string()
        .contains("unknown history event type"));
    let id_json = serde_json::to_string(&ids[0]).expect("id serializes");
    let reversed = format!("{{\"value\":{{\"entity\":{id_json}}},\"type\":\"spawned\"}}");
    let swapped = base.replacen(&payload0, &reversed, 1);
    assert_ne!(swapped, base);
    assert!(serde_json::from_str::<HistoryWorld>(&swapped)
        .expect_err("value before type fails")
        .to_string()
        .contains("requires `type` before `value`"));
    // A malformed value after the premature key still reports the order
    // fault, proving rejection happens before decoding that value.
    let bad_value = "{\"value\":{\"entity\":5},\"type\":\"spawned\"}".to_owned();
    let swapped_bad = base.replacen(&payload0, &bad_value, 1);
    assert!(serde_json::from_str::<HistoryWorld>(&swapped_bad)
        .expect_err("premature value fails first")
        .to_string()
        .contains("requires `type` before `value`"));
    // Payload field faults: unknown, duplicate, and missing keys fail.
    // Derived field decoding would keep the last duplicate silently.
    let unknown_field = payload0.replacen("\"entity\"", "\"entityx\"", 1);
    assert_ne!(unknown_field, payload0);
    let swapped = base.replacen(&payload0, &unknown_field, 1);
    assert!(serde_json::from_str::<HistoryWorld>(&swapped)
        .expect_err("unknown payload field fails")
        .to_string()
        .contains("unknown history payload field"));
    let inner = format!("{{\"entity\":{id_json}}}");
    assert!(
        payload0.contains(&inner),
        "the spawned payload carries the bare entity reference"
    );
    let doubled_inner = format!("{{\"entity\":{id_json},\"entity\":{id_json}}}");
    let doubled_payload = payload0.replacen(&inner, &doubled_inner, 1);
    let swapped = base.replacen(&payload0, &doubled_payload, 1);
    assert_ne!(swapped, base);
    assert!(serde_json::from_str::<HistoryWorld>(&swapped)
        .expect_err("duplicate payload field fails")
        .to_string()
        .contains("duplicate history payload field"));
    let mut value: serde_json::Value = serde_json::from_str(&base).expect("parses");
    value["pending"][0]["payload"]["value"]
        .as_object_mut()
        .expect("object")
        .remove("entity");
    assert!(serde_json::from_value::<HistoryWorld>(value)
        .expect_err("missing payload field fails")
        .to_string()
        .contains("misses `entity`"));
    // The same duplicate rule covers the richer action payload: doubling
    // its `actor` fails rather than keeping one copy.
    let action_payload = serde_json::to_string(&journal[3].payload).expect("payload");
    let actor_json = serde_json::to_string(&b).expect("actor serializes");
    let actor_pair = format!("\"actor\":{actor_json}");
    assert!(
        action_payload.contains(&actor_pair),
        "the fourth payload is the failed action resolution"
    );
    assert!(matches!(
        journal[3].payload,
        HistoryEvent::ActionResolved { .. }
    ));
    let doubled_action =
        action_payload.replacen(&actor_pair, &format!("{actor_pair},{actor_pair}"), 1);
    let swapped = base.replacen(&action_payload, &doubled_action, 1);
    assert_ne!(swapped, base);
    assert!(serde_json::from_str::<HistoryWorld>(&swapped)
        .expect_err("duplicate action field fails")
        .to_string()
        .contains("duplicate history payload field"));

    // Outcome faults: oversized (plain and escaped), invalid, and custom
    // boundary spellings — all rejected without retention.
    let oversized = format!("\"outcome\":\"{}\"", "x".repeat(300));
    let corrupt = base.replacen("\"outcome\":\"failure\"", &oversized, 1);
    assert!(serde_json::from_str::<HistoryWorld>(&corrupt)
        .expect_err("oversized outcome fails")
        .to_string()
        .contains("exceeds 256 bytes"));
    let escaped = format!("\"outcome\":\"failure{}\"", "\\u0041".repeat(300));
    let corrupt = base.replacen("\"outcome\":\"failure\"", &escaped, 1);
    assert!(serde_json::from_str::<HistoryWorld>(&corrupt)
        .expect_err("oversized escaped outcome fails")
        .to_string()
        .contains("exceeds 256 bytes"));
    for bad in [
        "triumph",
        "custom:",
        "custom:007",
        "custom:256",
        "custom:1000",
        "success ",
    ] {
        let attempt = format!("\"outcome\":\"{bad}\"");
        let corrupt = base.replacen("\"outcome\":\"failure\"", &attempt, 1);
        assert!(
            serde_json::from_str::<HistoryWorld>(&corrupt)
                .expect_err("invalid outcome fails")
                .to_string()
                .contains("names no known outcome"),
            "outcome {bad:?} must fail vocabulary validation"
        );
    }
    // The validator still admits the exact symbolic forms it promises.
    let custom = base.replacen("\"outcome\":\"failure\"", "\"outcome\":\"custom:7\"", 1);
    let loaded: HistoryWorld = serde_json::from_str(&custom).expect("custom outcomes load");
    assert!(matches!(
        loaded.read_after(3, 1).expect("read")[0].payload,
        HistoryEvent::ActionResolved { ref outcome, .. } if outcome == "custom:7"
    ));

    // Sequence, acknowledgement, and tick faults.
    let mut value: serde_json::Value = serde_json::from_str(&base).expect("parses");
    value["pending"][1]["seq"] = serde_json::Value::from(5);
    assert!(serde_json::from_value::<HistoryWorld>(value)
        .expect_err("gap fails")
        .to_string()
        .contains("contiguous suffix"));
    let mut value: serde_json::Value = serde_json::from_str(&base).expect("parses");
    let next = value["next_seq"].clone();
    value["acknowledged"] = next;
    assert!(serde_json::from_value::<HistoryWorld>(value)
        .expect_err("ack at next fails")
        .to_string()
        .contains("must precede its next sequence"));
    let mut value: serde_json::Value = serde_json::from_str(&base).expect("parses");
    value["next_seq"] = serde_json::Value::from(0);
    assert!(serde_json::from_value::<HistoryWorld>(value)
        .expect_err("zero next fails")
        .to_string()
        .contains("must not be zero"));
    // Decreasing ticks and ticks past the world tick need two envelopes at
    // distinct ticks.
    let mut ticked = HistoryWorld::new(0);
    ticked.spawn(EntityMeta {}).expect("spawn");
    ticked.tick().expect("tick");
    ticked.spawn(EntityMeta {}).expect("spawn");
    let ticked_json = serde_json::to_string(&ticked).expect("serializes");
    let mut value: serde_json::Value = serde_json::from_str(&ticked_json).expect("parses");
    value["pending"][0]["tick"] = serde_json::Value::from(1);
    value["pending"][1]["tick"] = serde_json::Value::from(0);
    assert!(serde_json::from_value::<HistoryWorld>(value)
        .expect_err("decreasing ticks fail")
        .to_string()
        .contains("ticks decrease"));
    let mut value: serde_json::Value = serde_json::from_str(&ticked_json).expect("parses");
    value["pending"][1]["tick"] = serde_json::Value::from(2);
    assert!(serde_json::from_value::<HistoryWorld>(value)
        .expect_err("future ticks fail")
        .to_string()
        .contains("after its world"));
    // A nonempty inner legacy queue never loads behind the journal.
    let mut value: serde_json::Value = serde_json::from_str(&base).expect("parses");
    value["world"]["events"] = serde_json::json!({
        "envelopes": [{
            "tick": 0,
            "seq": 0,
            "payload": {"Spawned": {"entity": {"index": 0, "generation": 1}}},
        }],
        "next_seq": 1,
    });
    assert!(serde_json::from_value::<HistoryWorld>(value)
        .expect_err("inner queue fails")
        .to_string()
        .contains("nonempty inner event queue"));
}

#[test]
fn oversized_outcome_fails_before_later_fields() {
    // Bounded-decoding proof: the outcome length is enforced while its
    // field decodes, before the trailing `damage` field decodes.
    // Pairing an oversized outcome with an invalid trailing damage must
    // still report the length fault. Post-payload validation would decode
    // the oversized string first and then fail on the damage type instead.
    // All inputs go through `from_str` so no `serde_json::Value` holds the
    // oversized text beforehand.
    let fixture = Fixture::standard();
    let (mut history, ids) = start_history(&fixture, 1);
    let (a, b) = (ids[0], ids[1]);
    history
        .perform_action(&CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        })
        .expect("failed attack journals a resolution")
        .expect("attack");
    let base = serde_json::to_string(&history).expect("serializes");
    assert!(
        base.contains("\"outcome\":\"failure\""),
        "the fixture journals one failure"
    );
    assert!(
        base.contains("\"damage\":0"),
        "the fixture journals zero damage"
    );
    // Positive control: a valid outcome with a mistyped trailing damage
    // reports the damage fault, proving field order and sensitivity.
    let bad_damage = base.replacen("\"damage\":0", "\"damage\":\"bad\"", 1);
    assert_ne!(bad_damage, base);
    let damage_err =
        serde_json::from_str::<HistoryWorld>(&bad_damage).expect_err("mistyped damage fails");
    assert!(
        damage_err.to_string().contains("invalid type"),
        "valid outcome plus bad damage must report the damage fault: {damage_err}"
    );
    // Plain oversized outcome plus the same mistyped damage still reports
    // the length fault, proving the outcome never waited for later fields.
    let oversized = format!("\"outcome\":\"{}\"", "x".repeat(300));
    let corrupt = base
        .replacen("\"outcome\":\"failure\"", &oversized, 1)
        .replacen("\"damage\":0", "\"damage\":\"bad\"", 1);
    let err =
        serde_json::from_str::<HistoryWorld>(&corrupt).expect_err("oversized outcome fails first");
    assert!(
        err.to_string().contains("exceeds 256 bytes"),
        "bounded outcome must win over later damage: {err}"
    );
    // Escaped oversized outcome behaves the same: length is checked before
    // the trailing damage decodes, even though the parser resolves escapes.
    let escaped = format!("\"outcome\":\"failure{}\"", "\\u0041".repeat(300));
    let corrupt = base
        .replacen("\"outcome\":\"failure\"", &escaped, 1)
        .replacen("\"damage\":0", "\"damage\":\"bad\"", 1);
    let err = serde_json::from_str::<HistoryWorld>(&corrupt)
        .expect_err("oversized escaped outcome fails first");
    assert!(
        err.to_string().contains("exceeds 256 bytes"),
        "bounded escaped outcome must win over later damage: {err}"
    );
}

#[test]
fn load_bounds_retained_storage() {
    // One past the event cap is rejected before its envelope decodes, even
    // though no unbounded Vec is ever materialized.
    let fresh = serde_json::to_string(&HistoryWorld::new(0)).expect("serializes");
    let template: serde_json::Value = serde_json::json!({
        "tick": 0,
        "payload": {"type": "spawned", "value": {"entity": {"index": 0, "generation": 1}}},
    });
    let mut pending = Vec::with_capacity(4097);
    for seq in 1..=4097u64 {
        let mut envelope = template.clone();
        envelope["seq"] = serde_json::Value::from(seq);
        pending.push(envelope);
    }
    let mut value: serde_json::Value = serde_json::from_str(&fresh).expect("parses");
    value["pending"] = serde_json::Value::from(pending);
    value["next_seq"] = serde_json::Value::from(4098u64);
    assert!(serde_json::from_value::<HistoryWorld>(value)
        .expect_err("the 4097th entry fails")
        .to_string()
        .contains("exceeds 4096 events"));
    // Exactly 4096 minimal envelopes pass the count guard with room in the
    // byte budget to spare.
    let mut value: serde_json::Value = serde_json::from_str(&fresh).expect("parses");
    let mut pending = Vec::with_capacity(4096);
    for seq in 1..=4096u64 {
        let mut envelope = template.clone();
        envelope["seq"] = serde_json::Value::from(seq);
        pending.push(envelope);
    }
    value["pending"] = serde_json::Value::from(pending);
    value["next_seq"] = serde_json::Value::from(4097u64);
    let loaded: HistoryWorld = serde_json::from_value(value).expect("a full minimal journal loads");
    assert_eq!(loaded.pending_len(), 4096);
    // But 4096 maximum-size valid envelopes exceed the byte budget: the
    // journal checks canonical totals incrementally before retaining each
    // envelope. Every field here is vocabulary-valid — oversized entity and
    // tick coordinates need no liveness — so only the byte guard can fail.
    let ability = Ulid::from_u128(u128::MAX).to_string();
    let mut big = String::from("\"pending\":[");
    for seq in 1..=4096u64 {
        if seq > 1 {
            big.push(',');
        }
        big.push_str(&format!(
            "{{\"tick\":18446744073709551615,\"seq\":{seq},\"payload\":{{\"type\":\"action_resolved\",\"value\":{{\"actor\":{{\"index\":4294967295,\"generation\":4294967294}},\"target\":{{\"index\":4294967295,\"generation\":4294967294}},\"ability\":\"{ability}\",\"outcome\":\"critical_failure\",\"damage\":4294967295}}}}}}"
        ));
    }
    big.push(']');
    let corrupt = fresh
        .replacen("\"tick\":0", "\"tick\":18446744073709551615", 1)
        .replacen("\"next_seq\":1", "\"next_seq\":4097", 1)
        .replacen("\"pending\":[]", &big, 1);
    assert_ne!(corrupt, fresh);
    // Sanity: the stitched input really holds 4096 envelopes.
    let check: serde_json::Value = serde_json::from_str(&corrupt).expect("stitches");
    assert_eq!(check["pending"].as_array().expect("array").len(), 4096);
    assert!(serde_json::from_str::<HistoryWorld>(&corrupt)
        .expect_err("the byte budget fails")
        .to_string()
        .contains("byte budget"));
}

#[test]
fn wrapper_matches_legacy_world() {
    // One schedule through both entry points: identical world bytes and hash.
    let fixture = Fixture::standard();
    let bundle = fixture.bundle();
    let spec = spec_for(&fixture, &bundle);
    let mut world = World::new(3);
    crpg_sim::start_encounter(&mut world, &spec).expect("legacy starts");
    let mut history = HistoryWorld::new(3);
    history.start_encounter(&spec).expect("wrapper starts");
    let ids: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
    let (a, b) = (ids[0], ids[1]);
    let attack = CombatAction::UseAbility {
        actor: b,
        ability: fixture.ability,
        target: a,
    };
    let legacy = crpg_sim::perform_action(&mut world, &attack).expect("legacy acts");
    let wrapped = history.perform_action(&attack).expect("wrapper acts");
    assert_eq!(legacy, wrapped);
    let end = CombatAction::EndTurn { actor: a };
    assert!(world.combat().expect("combat").active == Some(a));
    crpg_sim::perform_action(&mut world, &end).expect("legacy ends");
    history.perform_action(&end).expect("wrapper ends");
    crpg_sim::tick(&mut world);
    history.tick().expect("wrapper ticks");
    // The legacy queue accumulated the same pushes the wrapper drained, so
    // draining it leaves byte-identical worlds with identical hashes.
    world.events_mut().drain();
    assert!(history.world().events().is_empty());
    assert_eq!(
        serde_json::to_vec(&world).expect("legacy serializes"),
        serde_json::to_vec(history.world()).expect("wrapped serializes")
    );
    assert_eq!(state_hash(&world), state_hash(history.world()));
}
