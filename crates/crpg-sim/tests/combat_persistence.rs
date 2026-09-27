//! Combat save/load coverage (T016b).
//!
//! Mid-combat saves resume bit-identically, malformed combat saves fail to
//! load, and differently ordered interners preserve symbol meaning because
//! no handle is ever serialized.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_sim::{
    end_encounter, perform_action, start_encounter, state_hash, CombatAction, CombatError,
    EncounterSpec, EntityMeta, SimEvent, World,
};
use support::Fixture;

/// Attacks whoever holds the turn, onto the other combatant.
fn turn_action(world: &World, ids: &[EntityId], ability: Ulid) -> CombatAction {
    let active = world.combat().expect("combat").active.expect("a turn");
    let other = if active == ids[0] { ids[1] } else { ids[0] };
    CombatAction::UseAbility {
        actor: active,
        ability,
        target: other,
    }
}

#[test]
fn mid_combat_save_load_continues_identically() {
    let mut fixture = Fixture::standard();
    for creature in fixture.creatures.values_mut() {
        creature
            .stats
            .insert("health".to_owned(), Fx16_16::from_int(30_000));
    }
    let (mut live, ids) = fixture.start(7);
    let first = turn_action(&live, &ids, fixture.ability);
    let seen_first = perform_action(&mut live, &first)
        .expect("action one is valid")
        .expect("attack");
    let second = turn_action(&live, &ids, fixture.ability);
    let seen_second = perform_action(&mut live, &second)
        .expect("action two is valid")
        .expect("attack");

    let bytes = serde_json::to_vec(&live).expect("the world serializes");
    let mut loaded: World = serde_json::from_slice(&bytes).expect("the save loads");

    // The loaded world resumes byte-identically.
    assert_eq!(
        bytes,
        serde_json::to_vec(&loaded).expect("the load serializes")
    );
    assert_eq!(state_hash(&live), state_hash(&loaded));

    // Identical remaining inputs give identical behavior and hashes.
    for _ in 0..3 {
        let next = turn_action(&live, &ids, fixture.ability);
        let expected = perform_action(&mut live, &next)
            .expect("the live action is valid")
            .expect("attack");
        let resumed = turn_action(&loaded, &ids, fixture.ability);
        assert_eq!(next, resumed, "the same turn recurs in both worlds");
        let actual = perform_action(&mut loaded, &resumed)
            .expect("the loaded action is valid")
            .expect("attack");
        assert_eq!(expected, actual);
        assert_eq!(state_hash(&live), state_hash(&loaded));
    }
    assert_eq!(
        serde_json::to_vec(&live).expect("the world serializes"),
        serde_json::to_vec(&loaded).expect("the load serializes")
    );
    assert_eq!(seen_first.damage, 2, "seed 7 opens successfully");
    assert_eq!(seen_second.damage, 0, "seed 7 retaliates with a failure");
}

#[test]
fn malformed_combat_saves_fail_to_load() {
    let fixture = Fixture::standard();
    let (world, ids) = fixture.start(5);
    let _ = ids;
    let save = serde_json::to_value(&world).expect("the world serializes");

    // A combatant for a non-live entity.
    let mut other = World::new(5);
    for _ in 0..3 {
        other.spawn(EntityMeta {});
    }
    let foreign: EntityId = other.spawn(EntityMeta {});
    assert!(!world.contains(foreign));
    let mut dangling = save.clone();
    let combatant_json = dangling["combatants"][0][1].clone();
    dangling["combatants"]
        .as_array_mut()
        .expect("combatants")
        .push(serde_json::Value::Array(vec![
            serde_json::to_value(foreign).expect("an id"),
            combatant_json,
        ]));
    assert!(
        serde_json::from_value::<World>(dangling).is_err(),
        "a dangling combatant must fail to load"
    );

    // Encounter state with an empty combatant store.
    let mut some_empty = save.clone();
    *some_empty.get_mut("combatants").expect("combatants") = serde_json::Value::Array(Vec::new());
    assert!(
        serde_json::from_value::<World>(some_empty).is_err(),
        "an encounter without combatants must fail to load"
    );

    // A combatant store without encounter state.
    let mut none_full = save.clone();
    *none_full.get_mut("combat").expect("combat") = serde_json::Value::Null;
    assert!(
        serde_json::from_value::<World>(none_full).is_err(),
        "combatants without an encounter must fail to load"
    );

    // An active turn naming a non-combatant.
    let mut bad_active = save.clone();
    bad_active["combat"]["active"] = serde_json::to_value(foreign).expect("an id");
    assert!(
        serde_json::from_value::<World>(bad_active).is_err(),
        "a dangling active turn must fail to load"
    );

    // A malformed combat definition (dice that cannot parse).
    let mut bad_dice = save.clone();
    bad_dice["combat"]["definition"]["dice"] = serde_json::Value::String("bogus".to_owned());
    assert!(
        serde_json::from_value::<World>(bad_dice).is_err(),
        "an unparsable dice definition must fail to load"
    );
}
#[test]
fn differently_ordered_interner_preserves_meaning() {
    let fixture = Fixture::standard();
    let (ordered, ids) = fixture.start(7);
    let save = serde_json::to_value(&ordered).expect("the world serializes");

    // Rebuild the same symbols with a different insertion order: the roll
    // tag resolves to another handle, but no handle is persisted.
    let mut reordered = save.clone();
    let tags = reordered["interners"]["tags"]
        .as_array_mut()
        .expect("a tag list");
    tags.insert(0, serde_json::Value::String("alpha".to_owned()));
    tags.insert(0, serde_json::Value::String("beta".to_owned()));
    let shuffled: World = serde_json::from_value(reordered).expect("the reorder loads");
    assert_ne!(
        ordered.interners().tag("roll"),
        shuffled.interners().tag("roll"),
        "the test is vacuous unless the handles differ"
    );

    // Identical inputs give identical behavior from the shuffled world.
    let mut expected = ordered;
    let mut actual = shuffled;
    for _ in 0..4 {
        let next_expected = turn_action(&expected, &ids, fixture.ability);
        let next_actual = turn_action(&actual, &ids, fixture.ability);
        assert_eq!(next_expected, next_actual);
        let outcome_expected = perform_action(&mut expected, &next_expected)
            .expect("the ordered action is valid")
            .expect("attack");
        let outcome_actual = perform_action(&mut actual, &next_actual)
            .expect("the shuffled action is valid")
            .expect("attack");
        assert_eq!(outcome_expected, outcome_actual);
    }

    // Everything but the interner bytes agrees afterwards.
    let mut expected_value = serde_json::to_value(&expected).expect("the world serializes");
    let mut actual_value = serde_json::to_value(&actual).expect("the world serializes");
    expected_value
        .as_object_mut()
        .expect("an object")
        .remove("interners");
    actual_value
        .as_object_mut()
        .expect("an object")
        .remove("interners");
    assert_eq!(expected_value, actual_value);
}

/// Asserts one surgically corrupted save fails to load.
fn rejects(name: &str, base: &serde_json::Value, mutate: impl FnOnce(&mut serde_json::Value)) {
    let mut save = base.clone();
    mutate(&mut save);
    assert!(
        serde_json::from_value::<World>(save).is_err(),
        "{name} must fail to load"
    );
}

#[test]
fn corrupt_combat_saves_fail_to_load() {
    let fixture = Fixture::standard();
    // ids[0] is A (initiative 10); ids[1] is B (initiative 9, active head).
    // Combatants serialize in authored order; the timeline serializes
    // ascending, so timeline[0] is B (key 9) and timeline[1] is A (key 10).
    let (world, ids) = fixture.start(5);
    let base = serde_json::to_value(&world).expect("the world serializes");

    // The issuing roll tag is gone: the next action would panic.
    rejects("a missing roll tag", &base, |save| {
        save["interners"]["tags"] = serde_json::Value::Array(Vec::new());
    });
    // The checked attribute names something undeclared: the same panic.
    rejects("an unknown checked attribute", &base, |save| {
        save["combat"]["definition"]["attribute"] = serde_json::Value::from("nope");
    });
    rejects("an attribute absent from its declarations", &base, |save| {
        save["combat"]["definition"]["attribute_names"] =
            serde_json::Value::Array(vec!["beta".into(), "gamma".into()]);
    });
    rejects("a health stat inside its declarations", &base, |save| {
        save["combat"]["definition"]["health_stat"] = serde_json::Value::from("alpha");
    });
    rejects("a duplicate declared attribute", &base, |save| {
        save["combat"]["definition"]["attribute_names"]
            .as_array_mut()
            .expect("names")
            .push(serde_json::Value::from("alpha"));
    });
    rejects("empty declared attributes", &base, |save| {
        save["combat"]["definition"]["attribute_names"] = serde_json::Value::Array(Vec::new());
    });
    rejects("a zero ability cost", &base, |save| {
        save["combat"]["definition"]["cost"] = serde_json::Value::from(0);
    });
    rejects("a cost above its pool maximum", &base, |save| {
        save["combat"]["definition"]["cost"] = serde_json::Value::from(99);
    });
    rejects("health above its maximum", &base, |save| {
        save["combatants"][0][1]["health"] = serde_json::Value::from(11);
    });
    rejects("a nonpositive maximum", &base, |save| {
        save["combatants"][0][1]["max_health"] = serde_json::Value::from(0);
    });
    rejects("zero health on a live combatant", &base, |save| {
        save["combatants"][0][1]["health"] = serde_json::Value::from(0);
        save["combatants"][0][1]["dead"] = serde_json::Value::from(false);
    });
    rejects("positive health on a dead combatant", &base, |save| {
        save["combatants"][0][1]["dead"] = serde_json::Value::from(true);
    });
    rejects("a pool identity outside its definition", &base, |save| {
        save["combatants"][0][1]["action_pool"]["id"] =
            serde_json::Value::from(Ulid::from_u128(999).to_string());
    });
    rejects("a pool maximum outside its definition", &base, |save| {
        save["combatants"][0][1]["action_pool"]["max"] = serde_json::Value::from(2);
    });
    rejects("a pool refresh outside its definition", &base, |save| {
        save["combatants"][0][1]["action_pool"]["refresh"] =
            serde_json::json!({"type": "on_round_start"});
    });
    rejects("reordered combatant attributes", &base, |save| {
        let attributes = save["combatants"][0][1]["attributes"]
            .as_array_mut()
            .expect("attributes");
        attributes.swap(0, 1);
    });
    rejects("a combatant missing its checked attribute", &base, |save| {
        save["combatants"][0][1]["attributes"]
            .as_array_mut()
            .expect("attributes")
            .remove(0);
    });
    rejects("combatants sharing a placement", &base, |save| {
        let placement = save["combatants"][0][1]["placement"].clone();
        save["combatants"][1][1]["placement"] = placement;
    });
    // The active turn still names B but B is no longer scheduled.
    rejects("an active turn absent from its timeline", &base, |save| {
        save["timeline"].as_array_mut().expect("timeline").remove(0);
    });
    // The head becomes A while the turn still names B.
    rejects("an active turn behind its scheduled head", &base, |save| {
        save["timeline"][0][0] = serde_json::Value::from(11);
    });
    let _ = ids;
}

#[test]
fn corrupt_terminal_saves_fail_to_load() {
    // Seed 3 opens [1, 5]: total 6 against alpha 6, damage 2 onto health 2.
    let mut fixture = Fixture::standard();
    fixture
        .creatures
        .get_mut(&fixture.creature_a)
        .expect("A")
        .stats
        .insert("health".to_owned(), Fx16_16::from_int(2));
    let (mut world, ids) = fixture.start(3);
    let kill = CombatAction::UseAbility {
        actor: ids[1],
        ability: fixture.ability,
        target: ids[0],
    };
    assert!(
        perform_action(&mut world, &kill)
            .expect("kill")
            .expect("attack")
            .target_died
    );
    // A live outsider for the foreign-scheduling row.
    let outsider = world.spawn(EntityMeta {});
    let base = serde_json::to_value(&world).expect("the world serializes");

    // The dead keep zero health; scheduling them again is corrupt.
    rejects("a timeline scheduling its dead", &base, |save| {
        let entry = serde_json::json!([5, save["combatants"][0][0].clone()]);
        save["timeline"]
            .as_array_mut()
            .expect("timeline")
            .push(entry);
    });
    // The turn names the corpse the encounter just kept for its terminal state.
    rejects("an active turn naming its dead", &base, |save| {
        save["combat"]["active"] = save["combatants"][0][0].clone();
    });
    // Preexisting noncombat scheduling is not silently assumed away.
    rejects("a timeline scheduling an outsider", &base, |save| {
        let entry = serde_json::json!([5, serde_json::to_value(outsider).expect("an id")]);
        save["timeline"]
            .as_array_mut()
            .expect("timeline")
            .push(entry);
    });
}

#[test]
fn despawn_transitions_stay_reloadable() {
    let fixture = Fixture::standard();
    let (mut world, ids) = fixture.start(5);
    // B (ids[1]) holds the first turn; A (ids[0]) waits.
    assert!(world.despawn(ids[0]));
    let bytes = serde_json::to_vec(&world).expect("the world serializes");
    let waiting_gone: World =
        serde_json::from_slice(&bytes).expect("despawning the waiter reloads");
    assert_eq!(waiting_gone.combatants().len(), 1);
    assert_eq!(waiting_gone.combat().expect("combat").active, Some(ids[1]));

    assert!(world.despawn(ids[1]));
    let bytes = serde_json::to_vec(&world).expect("the world serializes");
    let released: World =
        serde_json::from_slice(&bytes).expect("despawning every combatant reloads");
    assert!(released.combat().is_none());
    assert!(released.combatants().is_empty());

    // Reentry through the existing operations: a second encounter starts in
    // the same world and plays.
    let bundle = fixture.bundle();
    let spec = EncounterSpec {
        encounter: &fixture.encounter,
        ruleset: &fixture.ruleset,
        abilities: &bundle.abilities,
        outcome_tables: &bundle.tables,
        placements: &bundle.placements,
        creatures: &bundle.creatures,
        effects: &bundle.effects,
    };
    start_encounter(&mut world, &spec).expect("a second encounter starts");
    assert_eq!(world.combat().expect("combat").round, 0);
    let again: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
    assert_eq!(again.len(), 2);
    let next = turn_action(&world, &again, fixture.ability);
    perform_action(&mut world, &next)
        .expect("the second fight plays")
        .expect("attack");
    let bytes = serde_json::to_vec(&world).expect("the world serializes");
    assert!(
        serde_json::from_slice::<World>(&bytes).is_ok(),
        "the second fight saves and reloads"
    );
}

#[test]
fn terminal_despawns_stay_reloadable() {
    // Seed 3 opens [1, 5]: total 6 against alpha 6, damage 2 onto health 2.
    let mut fixture = Fixture::standard();
    fixture
        .creatures
        .get_mut(&fixture.creature_a)
        .expect("A")
        .stats
        .insert("health".to_owned(), Fx16_16::from_int(2));
    let kill = |ability: Ulid, ids: &[EntityId]| CombatAction::UseAbility {
        actor: ids[1],
        ability,
        target: ids[0],
    };

    let (mut world, ids) = fixture.start(3);
    assert!(
        perform_action(&mut world, &kill(fixture.ability, &ids))
            .expect("kill")
            .expect("attack")
            .target_died
    );
    let bytes = serde_json::to_vec(&world).expect("the world serializes");
    let terminal: World = serde_json::from_slice(&bytes).expect("the terminal encounter reloads");
    assert_eq!(terminal.combat().expect("combat").active, None);

    // Despawning the corpse keeps the survivor's terminal state loadable.
    assert!(world.despawn(ids[0]));
    let bytes = serde_json::to_vec(&world).expect("the world serializes");
    let survivor: World = serde_json::from_slice(&bytes).expect("despawning the corpse reloads");
    assert_eq!(survivor.combatants().len(), 1);
    assert!(survivor.combat().is_some());
    assert_eq!(survivor.combat().expect("combat").active, None);

    // Despawning the survivor instead keeps the retained corpse loadable.
    let (mut world, ids) = fixture.start(3);
    assert!(
        perform_action(&mut world, &kill(fixture.ability, &ids))
            .expect("kill")
            .expect("attack")
            .target_died
    );
    assert!(world.despawn(ids[1]));
    let bytes = serde_json::to_vec(&world).expect("the world serializes");
    let corpse: World = serde_json::from_slice(&bytes).expect("despawning the survivor reloads");
    assert_eq!(corpse.combatants().len(), 1);
    let kept = corpse.combatants().get(ids[0]).expect("the corpse");
    assert_eq!(kept.health(), 0);
    assert!(kept.dead());
    // Despawning the last participant releases the encounter.
    assert!(world.despawn(ids[0]));
    let bytes = serde_json::to_vec(&world).expect("the world serializes");
    let released: World = serde_json::from_slice(&bytes).expect("despawning the corpse reloads");
    assert!(released.combat().is_none());
    assert!(released.combatants().is_empty());
}

/// Builds the terminal two-creature world: seed 3 opens [1, 5], total 6
/// against alpha 6, damage 2 onto health 2.
fn terminal_world() -> (Fixture, World, Vec<EntityId>) {
    let mut fixture = Fixture::standard();
    fixture
        .creatures
        .get_mut(&fixture.creature_a)
        .expect("A")
        .stats
        .insert("health".to_owned(), Fx16_16::from_int(2));
    let (mut world, ids) = fixture.start(3);
    let kill = CombatAction::UseAbility {
        actor: ids[1],
        ability: fixture.ability,
        target: ids[0],
    };
    assert!(
        perform_action(&mut world, &kill)
            .expect("kill")
            .expect("attack")
            .target_died
    );
    (fixture, world, ids)
}

/// Assembles the encounter spec one `start_encounter` borrows.
fn spec_of<'a>(fixture: &'a Fixture, bundle: &'a support::SpecBundle<'a>) -> EncounterSpec<'a> {
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

#[test]
fn encounter_release_full_lifecycle() {
    let (fixture, world, ids) = terminal_world();
    let bytes = serde_json::to_vec(&world).expect("the world serializes");
    let mut loaded: World = serde_json::from_slice(&bytes).expect("terminal reloads");

    let summary = end_encounter(&mut loaded).expect("release");
    assert_eq!(summary.encounter, fixture.encounter.id);
    assert_eq!(summary.ruleset, fixture.ruleset.id);
    assert_eq!(summary.round, 0);
    assert_eq!(summary.results.len(), 2);
    assert_eq!(summary.results[0].placement, fixture.placement_a);
    assert_eq!(summary.results[0].entity, ids[0]);
    assert_eq!(summary.results[0].health, 0);
    assert!(summary.results[0].dead);
    assert_eq!(summary.results[1].entity, ids[1]);
    assert_eq!(summary.results[1].health, 6);
    assert!(!summary.results[1].dead);

    // The released world is non-combat with live entities and kept events.
    assert!(loaded.combat().is_none());
    assert!(loaded.combatants().is_empty());
    assert!(loaded.contains(ids[0]));
    assert!(loaded.contains(ids[1]));
    assert_eq!(loaded.events().len(), 3);

    let bytes = serde_json::to_vec(&loaded).expect("the world serializes");
    let mut released: World = serde_json::from_slice(&bytes).expect("released reloads");
    assert!(released.combat().is_none());

    // A second encounter reuses the content with fresh runtime identity.
    let bundle = fixture.bundle();
    let spec = spec_of(&fixture, &bundle);
    start_encounter(&mut released, &spec).expect("a second encounter starts");
    assert_eq!(released.combat().expect("combat").round, 0);
    let again: Vec<EntityId> = released.combatants().iter().map(|(id, _)| id).collect();
    assert_eq!(again.len(), 2);
    assert!(
        !ids.contains(&again[0]) && !ids.contains(&again[1]),
        "the arena mints fresh entities for the second fight"
    );
    let next = turn_action(&released, &again, fixture.ability);
    perform_action(&mut released, &next)
        .expect("the second fight plays")
        .expect("attack");
    let bytes = serde_json::to_vec(&released).expect("the world serializes");
    assert!(
        serde_json::from_slice::<World>(&bytes).is_ok(),
        "the second fight saves and reloads"
    );
}

#[test]
fn release_commutes_with_save_load() {
    let (_, world, _) = terminal_world();
    let saved = serde_json::to_vec(&world).expect("the world serializes");
    let mut loaded: World = serde_json::from_slice(&saved).expect("terminal reloads");
    end_encounter(&mut loaded).expect("release after load");
    let after_load = serde_json::to_vec(&loaded).expect("the world serializes");

    let mut direct = world;
    end_encounter(&mut direct).expect("release before save");
    let direct_bytes = serde_json::to_vec(&direct).expect("the world serializes");

    assert_eq!(after_load, direct_bytes);
}

#[test]
fn release_rejections_preserve_state() {
    let fixture = Fixture::standard();
    let mut fresh = World::new(5);
    let before = serde_json::to_vec(&fresh).expect("the world serializes");
    assert_eq!(end_encounter(&mut fresh), Err(CombatError::NoEncounter));
    assert_eq!(
        before,
        serde_json::to_vec(&fresh).expect("the world serializes")
    );

    let (mut world, ids) = fixture.start(5);
    end_encounter(&mut world).expect("release");
    let bytes = serde_json::to_vec(&world).expect("the world serializes");
    assert_eq!(
        end_encounter(&mut world),
        Err(CombatError::NoEncounter),
        "repeated cleanup releases nothing twice"
    );
    assert_eq!(
        bytes,
        serde_json::to_vec(&world).expect("the world serializes")
    );

    let probe = CombatAction::UseAbility {
        actor: ids[0],
        ability: fixture.ability,
        target: ids[1],
    };
    assert_eq!(
        perform_action(&mut world, &probe),
        Err(CombatError::NoEncounter)
    );
    assert_eq!(
        bytes,
        serde_json::to_vec(&world).expect("the world serializes")
    );
}

#[test]
fn corpse_cleanup_never_duplicates_death() {
    let (_, mut world, ids) = terminal_world();
    assert!(world.despawn(ids[0]));
    assert!(
        !world.despawn(ids[0]),
        "re-despawning the corpse releases nothing twice"
    );
    let mut spawned = 0;
    let mut deaths = 0;
    let mut despawned = 0;
    for envelope in world.events_mut().drain() {
        match envelope.payload {
            SimEvent::Spawned { .. } => spawned += 1,
            SimEvent::Died { entity } => {
                deaths += 1;
                assert_eq!(entity, ids[0]);
            }
            SimEvent::Despawned { entity } => {
                despawned += 1;
                assert_eq!(entity, ids[0]);
            }
        }
    }
    assert_eq!(spawned, 2);
    assert_eq!(deaths, 1);
    assert_eq!(despawned, 1);
}

#[test]
fn mid_fight_release_returns_partial_summary() {
    let fixture = Fixture::standard();
    let (mut world, ids) = fixture.start(7);
    // Seed 7 opens successfully for damage 2 with no death.
    let first = turn_action(&world, &ids, fixture.ability);
    let seen = perform_action(&mut world, &first)
        .expect("the first action plays")
        .expect("attack");
    assert!(!seen.target_died);
    let summary = end_encounter(&mut world).expect("mid-fight release");
    assert_eq!(summary.results.len(), 2);
    assert!(summary.results.iter().all(|result| !result.dead));
    let remaining: u32 = summary.results.iter().map(|result| result.health).sum();
    assert_eq!(remaining, 14);
    assert!(world.combat().is_none());
}
