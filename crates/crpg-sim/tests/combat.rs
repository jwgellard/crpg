//! Authoritative headless combat coverage (T016b).
//!
//! All fixtures are hand-built neutral documents parsed through the
//! production data shapes (see `support`); no ruleset vocabulary enters
//! the assertions as engine knowledge. Recorded dice faces below are fixed
//! vectors for the pinned build: seed 1 opens `[6, 3]`, seed 3 opens
//! `[1, 5]`, seed 7 opens `[1, 5]`, seed 9 opens `[2, 3]`.

mod support;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_data::{
    DamageEntry, EncounterParticipant, OutcomeWire, RefreshWire, StatDecl, StatKindWire,
};
use crpg_sim::{
    perform_action, start_encounter, state_hash, tick, CombatAction, CombatError, EncounterSpec,
    EntityMeta, SimEvent, World, COMBAT_ROLL_STREAM,
};
use support::Fixture;

/// The standard first action: B (initiative 9) strikes A.
fn first_action(ids: &[EntityId], ability: Ulid) -> CombatAction {
    CombatAction::UseAbility {
        actor: ids[1],
        ability,
        target: ids[0],
    }
}

/// Serialized bytes snapshotting the entire world for rollback checks.
fn snapshot(world: &World) -> Vec<u8> {
    serde_json::to_vec(world).expect("the world serializes")
}

/// Asserts no combat stream was created on the world's RNG.
fn assert_no_combat_stream(world: &mut World) {
    assert!(
        !world.rng_mut().has_stream(COMBAT_ROLL_STREAM),
        "a rejected operation must not create the combat stream"
    );
}

#[test]
fn init_publishes_both_combatants() {
    let fixture = Fixture::standard();
    let (world, ids) = fixture.start(5);
    assert_eq!(ids.len(), 2);

    // Deterministic placement-to-entity mapping: authored order.
    assert_eq!(
        world.combatants().get(ids[0]).expect("A").placement(),
        fixture.placement_a
    );
    assert_eq!(
        world.combatants().get(ids[1]).expect("B").placement(),
        fixture.placement_b
    );

    let a = world.combatants().get(ids[0]).expect("A");
    let b = world.combatants().get(ids[1]).expect("B");
    assert_eq!(a.initiative(), 10);
    assert_eq!(b.initiative(), 9);
    assert_eq!(a.health(), 10);
    assert_eq!(a.max_health(), 10);
    assert_eq!(b.health(), 6);
    assert_eq!(b.max_health(), 6);
    assert!(!a.dead());
    assert!(!b.dead());
    assert_eq!(a.action_pool().current(), 1);
    assert_eq!(b.action_pool().current(), 1);

    // Lower initiative acts first, even though A is listed first.
    let order: Vec<(crpg_sim::InitiativeKey, EntityId)> = world.timeline().iter().collect();
    assert_eq!(order.len(), 2);
    assert_eq!(order[0].1, ids[1]);
    assert_eq!(order[1].1, ids[0]);

    let combat = world.combat().expect("an encounter is active");
    assert_eq!(combat.round, 0);
    assert_eq!(combat.active, Some(ids[1]));
    assert_eq!(combat.definition.ability, fixture.ability);
    assert_eq!(combat.definition.cost, 1);
}

#[test]
fn init_missing_placement_leaves_world_unchanged() {
    let mut fixture = Fixture::standard();
    fixture.placements.remove(&fixture.placement_a);
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
    let mut world = World::new(5);
    let before = snapshot(&world);
    let result = start_encounter(&mut world, &spec);
    assert_eq!(
        result,
        Err(CombatError::MissingPlacement {
            placement: fixture.placement_a
        })
    );
    assert_eq!(before, snapshot(&world));
    assert_no_combat_stream(&mut world);
}

#[test]
fn init_mixed_area_leaves_world_unchanged() {
    let mut fixture = Fixture::standard();
    let other = Ulid::from_u128(999);
    fixture
        .placements
        .get_mut(&fixture.placement_b)
        .expect("B")
        .1 = other;
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
    let mut world = World::new(5);
    let before = snapshot(&world);
    let result = start_encounter(&mut world, &spec);
    assert_eq!(
        result,
        Err(CombatError::MixedArea {
            area_a: fixture.area,
            area_b: other
        })
    );
    assert_eq!(before, snapshot(&world));
    assert_no_combat_stream(&mut world);
}

#[test]
fn init_missing_creature_leaves_world_unchanged() {
    let mut fixture = Fixture::standard();
    fixture.creatures.remove(&fixture.creature_a);
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
    let mut world = World::new(5);
    let before = snapshot(&world);
    let result = start_encounter(&mut world, &spec);
    assert_eq!(
        result,
        Err(CombatError::MissingCreature {
            prefab: fixture.creature_a
        })
    );
    assert_eq!(before, snapshot(&world));
    assert_no_combat_stream(&mut world);
}

#[test]
fn init_missing_stat_leaves_world_unchanged() {
    let mut fixture = Fixture::standard();
    fixture
        .creatures
        .get_mut(&fixture.creature_a)
        .expect("A")
        .stats
        .remove("beta");
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
    let mut world = World::new(5);
    let before = snapshot(&world);
    let result = start_encounter(&mut world, &spec);
    assert_eq!(
        result,
        Err(CombatError::MissingStat {
            stat: "beta".to_owned()
        })
    );
    assert_eq!(before, snapshot(&world));
    assert_no_combat_stream(&mut world);
}

#[test]
fn init_fractional_stat_leaves_world_unchanged() {
    let mut fixture = Fixture::standard();
    fixture
        .creatures
        .get_mut(&fixture.creature_b)
        .expect("B")
        .stats
        .insert("alpha".to_owned(), Fx16_16::from_raw(1));
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
    let mut world = World::new(5);
    let before = snapshot(&world);
    let result = start_encounter(&mut world, &spec);
    assert_eq!(
        result,
        Err(CombatError::InvalidStatValue {
            stat: "alpha".to_owned()
        })
    );
    assert_eq!(before, snapshot(&world));
    assert_no_combat_stream(&mut world);
}

#[test]
fn init_zero_health_leaves_world_unchanged() {
    let mut fixture = Fixture::standard();
    fixture
        .creatures
        .get_mut(&fixture.creature_a)
        .expect("A")
        .stats
        .insert("health".to_owned(), Fx16_16::from_int(0));
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
    let mut world = World::new(5);
    let before = snapshot(&world);
    let result = start_encounter(&mut world, &spec);
    assert_eq!(
        result,
        Err(CombatError::InvalidStatValue {
            stat: "health".to_owned()
        })
    );
    assert_eq!(before, snapshot(&world));
    assert_no_combat_stream(&mut world);
}

#[test]
fn init_invalid_cost_leaves_world_unchanged() {
    let mut fixture = Fixture::standard();
    fixture
        .abilities
        .get_mut(&fixture.ability)
        .expect("ability")
        .cost = 2;
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
    let mut world = World::new(5);
    let before = snapshot(&world);
    let result = start_encounter(&mut world, &spec);
    assert_eq!(result, Err(CombatError::InvalidCost { cost: 2, max: 1 }));
    assert_eq!(before, snapshot(&world));
    assert_no_combat_stream(&mut world);
}

#[test]
fn init_invalid_dice_leaves_world_unchanged() {
    let mut fixture = Fixture::standard();
    fixture
        .abilities
        .get_mut(&fixture.ability)
        .expect("ability")
        .dice = "0d6".to_owned();
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
    let mut world = World::new(5);
    let before = snapshot(&world);
    let result = start_encounter(&mut world, &spec);
    assert_eq!(result, Err(CombatError::InvalidDice));
    assert_eq!(before, snapshot(&world));
    assert_no_combat_stream(&mut world);
}

#[test]
fn init_missing_table_leaves_world_unchanged() {
    let mut fixture = Fixture::standard();
    fixture.tables.clear();
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
    let mut world = World::new(5);
    let before = snapshot(&world);
    let result = start_encounter(&mut world, &spec);
    assert_eq!(result, Err(CombatError::InvalidOutcomeTable));
    assert_eq!(before, snapshot(&world));
    assert_no_combat_stream(&mut world);
}

#[test]
fn init_malformed_bands_leave_world_unchanged() {
    let mut fixture = Fixture::standard();
    let table = fixture
        .tables
        .get_mut(&Ulid::from_u128(104))
        .expect("table");
    table.bands[0].min_margin = i64::MIN + 1;
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
    let mut world = World::new(5);
    let before = snapshot(&world);
    let result = start_encounter(&mut world, &spec);
    assert_eq!(result, Err(CombatError::InvalidOutcomeTable));
    assert_eq!(before, snapshot(&world));
    assert_no_combat_stream(&mut world);
}

#[test]
fn init_undeclared_ability_attribute_is_missing_stat() {
    let mut fixture = Fixture::standard();
    fixture
        .abilities
        .get_mut(&fixture.ability)
        .expect("ability")
        .attribute = "nope".to_owned();
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
    let mut world = World::new(5);
    let before = snapshot(&world);
    let result = start_encounter(&mut world, &spec);
    assert_eq!(
        result,
        Err(CombatError::MissingStat {
            stat: "nope".to_owned()
        })
    );
    assert_eq!(before, snapshot(&world));
    assert_no_combat_stream(&mut world);
}

#[test]
fn init_too_many_participants_leaves_world_unchanged() {
    let mut fixture = Fixture::standard();
    for index in 0..1025_u32 {
        let placement = Ulid::from_u128(10_000 + u128::from(index));
        let prefab = Ulid::from_u128(20_000 + u128::from(index));
        let stats = fixture
            .creatures
            .get(&fixture.creature_a)
            .expect("A")
            .stats
            .clone();
        let creature = crpg_data::Creature {
            id: prefab,
            slug: "probe-extra".to_owned(),
            name: "probe.extra".to_owned(),
            note: None,
            stats,
            tags: Vec::new(),
            faction: None,
            inventory: Vec::new(),
        };
        let placement_doc = crpg_data::Placement {
            id: placement,
            slug: "probe-extra".to_owned(),
            name: "probe.extra".to_owned(),
            note: None,
            prefab,
            transform: crpg_data::Transform {
                position: [Fx16_16::from_int(0); 3],
                rotation: [Fx16_16::from_int(0); 3],
                scale: [Fx16_16::from_int(1); 3],
            },
            overrides: Default::default(),
        };
        fixture.creatures.insert(prefab, creature);
        fixture
            .placements
            .insert(placement, (placement_doc, fixture.area));
        fixture.encounter.participants.push(EncounterParticipant {
            placement,
            initiative: 0,
        });
    }
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
    let mut world = World::new(5);
    let before = snapshot(&world);
    let result = start_encounter(&mut world, &spec);
    assert_eq!(result, Err(CombatError::TooManyParticipants));
    assert_eq!(before, snapshot(&world));
    assert_no_combat_stream(&mut world);
}

#[test]
fn init_too_many_stats_leaves_world_unchanged() {
    let mut fixture = Fixture::standard();
    for index in 0..1025_usize {
        fixture.ruleset.stats.push(StatDecl {
            name: format!("extra-{index}"),
            kind: StatKindWire::Int,
        });
    }
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
    let mut world = World::new(5);
    let before = snapshot(&world);
    let result = start_encounter(&mut world, &spec);
    assert_eq!(result, Err(CombatError::TooManyStats));
    assert_eq!(before, snapshot(&world));
    assert_no_combat_stream(&mut world);
}

#[test]
fn init_while_active_reports_encounter_active() {
    let fixture = Fixture::standard();
    let (mut world, _) = fixture.start(5);
    // A structurally broken spec still reports the active encounter first.
    let mut broken = Fixture::standard();
    broken.placements.clear();
    let bundle = broken.bundle();
    let spec = EncounterSpec {
        encounter: &broken.encounter,
        ruleset: &broken.ruleset,
        abilities: &bundle.abilities,
        outcome_tables: &bundle.tables,
        placements: &bundle.placements,
        creatures: &bundle.creatures,
        effects: &bundle.effects,
    };
    let before = snapshot(&world);
    assert_eq!(
        start_encounter(&mut world, &spec),
        Err(CombatError::EncounterActive)
    );
    assert_eq!(before, snapshot(&world));
}

#[test]
fn roll_under_threshold_neighbors() {
    // Seed 3 opens [1, 5]: total 6 whatever the checked attribute is.
    for (alpha, margin, success) in [(6, 0_i64, true), (5, 1, false), (7, -1, true)] {
        let mut fixture = Fixture::standard();
        fixture.set_alpha(alpha, alpha);
        let (mut world, ids) = fixture.start(3);
        let outcome = perform_action(&mut world, &first_action(&ids, fixture.ability))
            .expect("the neighbor action is valid")
            .expect("attack");
        let faces: Vec<u32> = outcome.roll.dice.iter().map(|die| die.value).collect();
        assert_eq!(faces, vec![1, 5], "seed 3 draws are fixed");
        assert_eq!(outcome.roll.total, 6);
        assert_eq!(outcome.margin, margin);
        assert_eq!(
            outcome.outcome,
            if success {
                crpg_rules::Outcome::Success
            } else {
                crpg_rules::Outcome::Failure
            }
        );
        assert_eq!(outcome.damage, if success { 2 } else { 0 });
    }
}

#[test]
fn successful_attack_trace() {
    // Seed 9 opens [2, 3]: total 5 against alpha 6.
    let fixture = Fixture::standard();
    let (mut world, ids) = fixture.start(9);
    let outcome = perform_action(&mut world, &first_action(&ids, fixture.ability))
        .expect("the attack is valid")
        .expect("attack");
    let faces: Vec<u32> = outcome.roll.dice.iter().map(|die| die.value).collect();
    assert_eq!(faces, vec![2, 3]);
    assert_eq!(outcome.roll.total, 5);
    assert_eq!(outcome.margin, -1);
    assert_eq!(outcome.outcome, crpg_rules::Outcome::Success);
    assert_eq!(outcome.damage, 2);
    assert!(!outcome.target_died);
    assert_eq!(world.combatants().get(ids[0]).expect("A").health(), 8);
    assert_eq!(world.combatants().get(ids[1]).expect("B").health(), 6);
    // A valid attack consumes its action: the turn advanced.
    assert_eq!(world.combat().expect("combat").active, Some(ids[0]));
    assert_eq!(
        world
            .combatants()
            .get(ids[1])
            .expect("B")
            .action_pool()
            .current(),
        0
    );
}

#[test]
fn failed_attack_trace_consumes_action_without_damage() {
    // Seed 1 opens [6, 3]: total 9 against alpha 6.
    let fixture = Fixture::standard();
    let (mut world, ids) = fixture.start(1);
    let outcome = perform_action(&mut world, &first_action(&ids, fixture.ability))
        .expect("the attack is valid")
        .expect("attack");
    let faces: Vec<u32> = outcome.roll.dice.iter().map(|die| die.value).collect();
    assert_eq!(faces, vec![6, 3]);
    assert_eq!(outcome.roll.total, 9);
    assert_eq!(outcome.margin, 3);
    assert_eq!(outcome.outcome, crpg_rules::Outcome::Failure);
    assert_eq!(outcome.damage, 0);
    assert!(!outcome.target_died);
    assert_eq!(world.combatants().get(ids[0]).expect("A").health(), 10);
    // The failed attack still consumed its action.
    assert_eq!(
        world
            .combatants()
            .get(ids[1])
            .expect("B")
            .action_pool()
            .current(),
        0
    );
    assert_eq!(world.combat().expect("combat").active, Some(ids[0]));
    // A second immediate action is out of turn even after a failure.
    let again = perform_action(&mut world, &first_action(&ids, fixture.ability));
    assert_eq!(
        again,
        Err(CombatError::OutOfTurn {
            actor: ids[1],
            active: Some(ids[0])
        })
    );
}

#[test]
fn initiative_ties_break_on_entity_id() {
    let mut fixture = Fixture::standard();
    fixture.encounter.participants[0].initiative = 7;
    fixture.encounter.participants[1].initiative = 7;
    let (world, ids) = fixture.start(5);
    // A spawned first, so its lower id wins the tie.
    assert_eq!(world.combat().expect("combat").active, Some(ids[0]));
    let order: Vec<EntityId> = world.timeline().iter().map(|(_, id)| id).collect();
    assert_eq!(order, ids);
}

#[test]
fn round_rollover_refreshes_and_repeats_order() {
    // Seed 1: B fails ([6, 3]), A succeeds ([2, 3] total 5 vs alpha 8).
    let fixture = Fixture::standard();
    let (mut world, ids) = fixture.start(1);
    perform_action(&mut world, &first_action(&ids, fixture.ability))
        .expect("B acts")
        .expect("attack");
    let retaliation = CombatAction::UseAbility {
        actor: ids[0],
        ability: fixture.ability,
        target: ids[1],
    };
    let second = perform_action(&mut world, &retaliation)
        .expect("A acts")
        .expect("attack");
    let faces: Vec<u32> = second.roll.dice.iter().map(|die| die.value).collect();
    assert_eq!(faces, vec![2, 3]);
    assert_eq!(second.outcome, crpg_rules::Outcome::Success);
    // Both acted: the round rolled over with B to move again.
    let combat = world.combat().expect("combat");
    assert_eq!(combat.round, 1);
    assert_eq!(combat.active, Some(ids[1]));
    assert_eq!(
        world
            .combatants()
            .get(ids[1])
            .expect("B")
            .action_pool()
            .current(),
        1,
        "the incoming actor refreshes once per turn start"
    );
    assert_eq!(
        world
            .combatants()
            .get(ids[0])
            .expect("A")
            .action_pool()
            .current(),
        0
    );
    // The refreshed pool pays for B's next turn.
    perform_action(&mut world, &first_action(&ids, fixture.ability))
        .expect("B acts again")
        .expect("attack");
    assert_eq!(
        world
            .combatants()
            .get(ids[1])
            .expect("B")
            .action_pool()
            .current(),
        0
    );
}

#[test]
fn killing_blow_goes_terminal_with_exactly_one_death() {
    let mut fixture = Fixture::standard();
    fixture
        .creatures
        .get_mut(&fixture.creature_a)
        .expect("A")
        .stats
        .insert("health".to_owned(), Fx16_16::from_int(2));
    // Seed 3: B opens [1, 5], total 6 against alpha 6, damage 2.
    let (mut world, ids) = fixture.start(3);
    let outcome = perform_action(&mut world, &first_action(&ids, fixture.ability))
        .expect("the killing blow is valid")
        .expect("attack");
    assert_eq!(outcome.damage, 2);
    assert!(outcome.target_died);
    let dead = world.combatants().get(ids[0]).expect("A");
    assert_eq!(dead.health(), 0);
    assert!(dead.dead());

    // Death is scheduling-only: the entity and its terminal state remain.
    assert!(world.contains(ids[0]));
    assert_eq!(world.combatants().len(), 2);
    assert!(world.timeline().is_empty());
    let combat = world.combat().expect("combat");
    assert_eq!(combat.active, None);
    assert_eq!(combat.round, 0, "no rollover once one side is dead");

    // Exactly one death notification among the spawn notices.
    let mut deaths = 0;
    let mut spawns = 0;
    for envelope in world.events_mut().drain() {
        match envelope.payload {
            SimEvent::Died { entity } => {
                deaths += 1;
                assert_eq!(entity, ids[0]);
            }
            SimEvent::Spawned { .. } => spawns += 1,
            SimEvent::Despawned { .. } => panic!("no despawn happened"),
        }
    }
    assert_eq!(spawns, 2);
    assert_eq!(deaths, 1);
}

#[test]
fn rejected_actions_leave_the_world_unchanged() {
    let fixture = Fixture::standard();
    // NoEncounter first, on a fresh world with two live outsiders.
    let mut fresh = World::new(5);
    let outsider_a = fresh.spawn(EntityMeta {});
    let outsider_b = fresh.spawn(EntityMeta {});
    let probe = CombatAction::UseAbility {
        actor: outsider_a,
        ability: fixture.ability,
        target: outsider_b,
    };
    let before = snapshot(&fresh);
    assert_eq!(
        perform_action(&mut fresh, &probe),
        Err(CombatError::NoEncounter)
    );
    assert_eq!(before, snapshot(&fresh));
    assert_no_combat_stream(&mut fresh);

    let (mut world, ids) = fixture.start(5);
    // OutOfTurn: A moves before B's turn ends.
    let early = CombatAction::UseAbility {
        actor: ids[0],
        ability: fixture.ability,
        target: ids[1],
    };
    let before = snapshot(&world);
    assert_eq!(
        perform_action(&mut world, &early),
        Err(CombatError::OutOfTurn {
            actor: ids[0],
            active: Some(ids[1])
        })
    );
    assert_eq!(before, snapshot(&world));
    assert_no_combat_stream(&mut world);

    // UnknownAbility on the active turn.
    let strange = CombatAction::UseAbility {
        actor: ids[1],
        ability: Ulid::from_u128(999),
        target: ids[0],
    };
    let before = snapshot(&world);
    assert_eq!(
        perform_action(&mut world, &strange),
        Err(CombatError::UnknownAbility {
            ability: Ulid::from_u128(999)
        })
    );
    assert_eq!(before, snapshot(&world));
    assert_no_combat_stream(&mut world);

    // SelfTarget on the active turn.
    let selfish = CombatAction::UseAbility {
        actor: ids[1],
        ability: fixture.ability,
        target: ids[1],
    };
    let before = snapshot(&world);
    assert_eq!(
        perform_action(&mut world, &selfish),
        Err(CombatError::SelfTarget)
    );
    assert_eq!(before, snapshot(&world));
    assert_no_combat_stream(&mut world);

    // NotParticipant: a live outsider.
    let outsider = world.spawn(EntityMeta {});
    let intruder = CombatAction::UseAbility {
        actor: outsider,
        ability: fixture.ability,
        target: ids[0],
    };
    let before = snapshot(&world);
    assert_eq!(
        perform_action(&mut world, &intruder),
        Err(CombatError::NotParticipant { entity: outsider })
    );
    assert_eq!(before, snapshot(&world));

    // AbsentActor / AbsentTarget: despawned ids.
    assert!(world.despawn(outsider));
    let ghost = CombatAction::UseAbility {
        actor: outsider,
        ability: fixture.ability,
        target: ids[0],
    };
    let before = snapshot(&world);
    assert_eq!(
        perform_action(&mut world, &ghost),
        Err(CombatError::AbsentActor { actor: outsider })
    );
    assert_eq!(before, snapshot(&world));
    let ghost_target = CombatAction::UseAbility {
        actor: ids[1],
        ability: fixture.ability,
        target: outsider,
    };
    let before = snapshot(&world);
    assert_eq!(
        perform_action(&mut world, &ghost_target),
        Err(CombatError::AbsentTarget { target: outsider })
    );
    assert_eq!(before, snapshot(&world));
}

#[test]
fn insufficient_action_needs_an_unrefreshed_pool() {
    let mut fixture = Fixture::standard();
    fixture.ruleset.pools[0].refresh = RefreshWire::Never;
    let (mut world, ids) = fixture.start(11);
    // Seed 11: B opens [6, 4] failing, A follows [1, 2] succeeding.
    perform_action(&mut world, &first_action(&ids, fixture.ability))
        .expect("B acts")
        .expect("attack");
    let retaliation = CombatAction::UseAbility {
        actor: ids[0],
        ability: fixture.ability,
        target: ids[1],
    };
    perform_action(&mut world, &retaliation)
        .expect("A acts")
        .expect("attack");
    // Rollover without refresh: B holds the turn with an empty pool.
    assert_eq!(world.combat().expect("combat").active, Some(ids[1]));
    assert_eq!(
        world
            .combatants()
            .get(ids[1])
            .expect("B")
            .action_pool()
            .current(),
        0
    );
    let before = snapshot(&world);
    assert_eq!(
        perform_action(&mut world, &first_action(&ids, fixture.ability)),
        Err(CombatError::InsufficientAction {
            pool: Ulid::from_u128(105),
            cost: 1,
            current: 0
        })
    );
    assert_eq!(before, snapshot(&world));
}

#[test]
fn multifault_precedence_pins_order() {
    let fixture = Fixture::standard();
    let (mut world, ids) = fixture.start(5);

    // OutOfTurn (8) beats UnknownAbility (9).
    let early_strange = CombatAction::UseAbility {
        actor: ids[0],
        ability: Ulid::from_u128(999),
        target: ids[1],
    };
    assert_eq!(
        perform_action(&mut world, &early_strange),
        Err(CombatError::OutOfTurn {
            actor: ids[0],
            active: Some(ids[1])
        })
    );

    // UnknownAbility (9) beats SelfTarget (10).
    let strange_self = CombatAction::UseAbility {
        actor: ids[1],
        ability: Ulid::from_u128(999),
        target: ids[1],
    };
    assert_eq!(
        perform_action(&mut world, &strange_self),
        Err(CombatError::UnknownAbility {
            ability: Ulid::from_u128(999)
        })
    );

    // NotParticipant (actor, 3) beats AbsentTarget (5).
    let outsider = world.spawn(EntityMeta {});
    let mut gone = World::new(0);
    let foreign = gone.spawn(EntityMeta {});
    assert!(gone.despawn(foreign));
    let mixed = CombatAction::UseAbility {
        actor: outsider,
        ability: fixture.ability,
        target: foreign,
    };
    assert_eq!(
        perform_action(&mut world, &mixed),
        Err(CombatError::NotParticipant { entity: outsider })
    );

    // AbsentActor (4) beats AbsentTarget (5): a world-local dead id names
    // nothing live, unlike the cross-world collision above.
    let local_ghost = world.spawn(EntityMeta {});
    assert!(world.despawn(local_ghost));
    let double_ghost = CombatAction::UseAbility {
        actor: local_ghost,
        ability: fixture.ability,
        target: local_ghost,
    };
    assert_eq!(
        perform_action(&mut world, &double_ghost),
        Err(CombatError::AbsentActor { actor: local_ghost })
    );
}

#[test]
fn dead_combatants_reject_without_duplicate_death() {
    let mut fixture = Fixture::standard();
    fixture
        .creatures
        .get_mut(&fixture.creature_a)
        .expect("A")
        .stats
        .insert("health".to_owned(), Fx16_16::from_int(2));
    let (mut world, ids) = fixture.start(3);
    perform_action(&mut world, &first_action(&ids, fixture.ability))
        .expect("kill")
        .expect("attack");
    let events_after_kill = world.events().len();

    // DeadTarget (7) beats OutOfTurn (8) in the terminal encounter.
    let at_corpse = CombatAction::UseAbility {
        actor: ids[1],
        ability: fixture.ability,
        target: ids[0],
    };
    assert_eq!(
        perform_action(&mut world, &at_corpse),
        Err(CombatError::DeadTarget { target: ids[0] })
    );
    // DeadActor (6) fires for the corpse's own action.
    let by_corpse = CombatAction::UseAbility {
        actor: ids[0],
        ability: fixture.ability,
        target: ids[1],
    };
    assert_eq!(
        perform_action(&mut world, &by_corpse),
        Err(CombatError::DeadActor { actor: ids[0] })
    );
    assert_eq!(
        world.events().len(),
        events_after_kill,
        "rejected actions emit no second death"
    );
}

#[test]
fn overkill_floors_at_zero_without_wrapping() {
    let mut fixture = Fixture::standard();
    fixture
        .abilities
        .get_mut(&fixture.ability)
        .expect("ability")
        .damage = vec![
        DamageEntry {
            outcome: OutcomeWire::Success,
            amount: u32::MAX,
        },
        DamageEntry {
            outcome: OutcomeWire::Failure,
            amount: 0,
        },
    ];
    // Seed 9: B succeeds against alpha 6.
    let (mut world, ids) = fixture.start(9);
    let outcome = perform_action(&mut world, &first_action(&ids, fixture.ability))
        .expect("the overkill is valid")
        .expect("attack");
    assert_eq!(outcome.damage, u32::MAX);
    assert!(outcome.target_died);
    assert_eq!(world.combatants().get(ids[0]).expect("A").health(), 0);
    assert!(world.combatants().get(ids[0]).expect("A").dead());
}

#[test]
fn despawn_clears_combat_scheduling() {
    let fixture = Fixture::standard();
    let (mut world, ids) = fixture.start(5);
    // Despawn the active combatant: component, schedule, and turn clear.
    assert!(world.despawn(ids[1]));
    assert!(!world.combatants().contains(ids[1]));
    assert!(!world.timeline().contains(ids[1]));
    assert_eq!(world.combat().expect("combat").active, None);
    assert!(!world.despawn(ids[1]));
    for (id, _) in world.combatants().iter() {
        assert!(world.contains(id), "no dangling combatant for dead {id:?}");
    }
    for (_, id) in world.timeline().iter() {
        assert!(
            world.contains(id),
            "no dangling timeline entry for dead {id:?}"
        );
    }
    // The survivor keeps its component and schedule but holds no turn.
    assert!(world.combatants().contains(ids[0]));
    assert!(world.timeline().contains(ids[0]));
    let stranded = CombatAction::UseAbility {
        actor: ids[0],
        ability: fixture.ability,
        target: ids[0],
    };
    assert_eq!(
        perform_action(&mut world, &stranded),
        Err(CombatError::OutOfTurn {
            actor: ids[0],
            active: None
        })
    );
}

#[test]
fn despawn_waiter_keeps_turn_in_trio() {
    let fixture = Fixture::trio();
    let (mut world, ids) = fixture.start(5);
    // C (ids[2], initiative 8) holds the first turn over B then A.
    assert_eq!(world.combat().expect("combat").active, Some(ids[2]));
    assert!(world.despawn(ids[0]));
    assert_eq!(world.combat().expect("combat").active, Some(ids[2]));
    let order: Vec<EntityId> = world.timeline().iter().map(|(_, id)| id).collect();
    assert_eq!(order, vec![ids[2], ids[1]]);
    for id in &ids[1..] {
        assert_eq!(
            world
                .combatants()
                .get(*id)
                .expect("survivor")
                .action_pool()
                .current(),
            1,
            "removing a waiter touches no pool"
        );
    }
    let mut spawned = 0;
    let mut despawned = 0;
    for envelope in world.events_mut().drain() {
        match envelope.payload {
            SimEvent::Spawned { .. } => spawned += 1,
            SimEvent::Despawned { entity } => {
                despawned += 1;
                assert_eq!(entity, ids[0]);
            }
            SimEvent::Died { .. } => panic!("cleanup emits no death"),
        }
    }
    assert_eq!(spawned, 3);
    assert_eq!(despawned, 1);
}

#[test]
fn despawn_active_passes_turn_in_trio() {
    let fixture = Fixture::trio();
    let (mut world, ids) = fixture.start(5);
    assert!(world.despawn(ids[2]));
    // B (initiative 9) is the new head with a real turn.
    assert_eq!(world.combat().expect("combat").active, Some(ids[1]));
    let order: Vec<EntityId> = world.timeline().iter().map(|(_, id)| id).collect();
    assert_eq!(order, vec![ids[1], ids[0]]);
    let passed = CombatAction::UseAbility {
        actor: ids[1],
        ability: fixture.ability,
        target: ids[0],
    };
    perform_action(&mut world, &passed)
        .expect("the passed turn plays")
        .expect("attack");
    for envelope in world.events_mut().drain() {
        assert!(
            !matches!(envelope.payload, SimEvent::Died { .. }),
            "cleanup emits no death"
        );
    }
}

#[test]
fn despawn_active_with_one_alive_goes_terminal() {
    let fixture = Fixture::standard();
    let (mut world, ids) = fixture.start(5);
    assert!(world.despawn(ids[1]));
    let combat = world.combat().expect("combat");
    assert_eq!(combat.active, None);
    assert_eq!(combat.round, 0, "no rollover for a single survivor");
    assert!(world.timeline().contains(ids[0]));
    assert_eq!(
        world
            .combatants()
            .get(ids[0])
            .expect("A")
            .action_pool()
            .current(),
        1,
        "no spurious refresh on the terminal path"
    );
}

#[test]
fn despawn_advance_rolls_over_when_emptied_with_two_alive() {
    let fixture = Fixture::trio();
    let (mut world, ids) = fixture.start(1);
    // Seed 1 opens [6, 3]: C fails onto B without damage; B's retaliation
    // onto A is valid whatever its faces (A's health absorbs any 2d6 hit).
    let first = CombatAction::UseAbility {
        actor: ids[2],
        ability: fixture.ability,
        target: ids[1],
    };
    perform_action(&mut world, &first)
        .expect("C acts")
        .expect("attack");
    let second = CombatAction::UseAbility {
        actor: ids[1],
        ability: fixture.ability,
        target: ids[0],
    };
    perform_action(&mut world, &second)
        .expect("B acts")
        .expect("attack");
    assert_eq!(world.combat().expect("combat").active, Some(ids[0]));
    // Removing A empties the timeline with B and C alive: rollover.
    assert!(world.despawn(ids[0]));
    let combat = world.combat().expect("combat");
    assert_eq!(combat.round, 1);
    assert_eq!(combat.active, Some(ids[2]));
    let order: Vec<EntityId> = world.timeline().iter().map(|(_, id)| id).collect();
    assert_eq!(order, vec![ids[2], ids[1]]);
    for envelope in world.events_mut().drain() {
        assert!(
            !matches!(envelope.payload, SimEvent::Died { .. }),
            "cleanup emits no death"
        );
    }
}

#[test]
fn no_input_ticks_invent_nothing() {
    let fixture = Fixture::standard();
    let (mut world, ids) = fixture.start(5);
    perform_action(&mut world, &first_action(&ids, fixture.ability))
        .expect("B acts")
        .expect("attack");
    let mut value = serde_json::to_value(&world).expect("the world serializes");
    value.as_object_mut().expect("an object").remove("tick");
    tick(&mut world);
    tick(&mut world);
    let mut after = serde_json::to_value(&world).expect("the world serializes");
    after.as_object_mut().expect("an object").remove("tick");
    assert_eq!(value, after, "no-input ticks change only the counter");
}

#[test]
fn hash_sensitivity_covers_combat_state() {
    let fixture = Fixture::standard();
    let (world, _) = fixture.start(5);
    let base = state_hash(&world);
    let value = serde_json::to_value(&world).expect("the world serializes");

    // Health.
    let mut health = value.clone();
    health["combatants"][0][1]["health"] = serde_json::to_value(9).expect("a number");
    assert_ne!(base, state_hash(&load(&health)), "health is hashed");

    // Resource balance.
    let mut pool = value.clone();
    pool["combatants"][0][1]["action_pool"]["current"] = serde_json::to_value(0).expect("a number");
    assert_ne!(base, state_hash(&load(&pool)), "pool balance is hashed");

    // Round.
    let mut round = value.clone();
    round["combat"]["round"] = serde_json::to_value(4).expect("a number");
    assert_ne!(base, state_hash(&load(&round)), "round is hashed");

    // Active turn: B holds the first turn, so clearing it changes bytes.
    // Clearing is the stranded/terminal-legal shape; handing the turn to an
    // unscheduled combatant is a corrupt save (see combat_persistence).
    let mut active = value.clone();
    active["combat"]["active"] = serde_json::Value::Null;
    assert_ne!(base, state_hash(&load(&active)), "active turn is hashed");

    // Timeline keys: rekeying the head entry changes bytes while the head,
    // order, and turn stay put, so the save stays valid.
    let mut timeline = value.clone();
    timeline["timeline"][0][0] = serde_json::to_value(8).expect("a number");
    assert_ne!(base, state_hash(&load(&timeline)), "timeline is hashed");

    // Queued events.
    let mut world_events = fixture.start(5).0;
    let stamp = world_events.tick();
    let first_combatant = world_events
        .combatants()
        .iter()
        .next()
        .expect("a combatant")
        .0;
    world_events.events_mut().push(
        stamp,
        crpg_sim::SimEvent::Spawned {
            entity: first_combatant,
        },
    );
    assert_ne!(base, state_hash(&world_events), "queued events are hashed");

    // Combat definition.
    let mut definition = value.clone();
    definition["combat"]["definition"]["damage"][0][1] = serde_json::to_value(7).expect("a number");
    assert_ne!(base, state_hash(&load(&definition)), "definition is hashed");

    // Interner contents.
    let mut interners = value.clone();
    interners["interners"]["tags"]
        .as_array_mut()
        .expect("a tag list")
        .push(serde_json::Value::String("zzz".to_owned()));
    assert_ne!(base, state_hash(&load(&interners)), "interner is hashed");

    // RNG streams, isolated: one action advances only the RNG here.
    let (mut advanced, _) = fixture.start(7);
    let advanced_actor = advanced.combat().expect("combat").active.expect("a turn");
    let advanced_target = advanced.combatants().iter().next().expect("a combatant").0;
    perform_action(
        &mut advanced,
        &CombatAction::UseAbility {
            actor: advanced_actor,
            ability: fixture.ability,
            target: advanced_target,
        },
    )
    .expect("the action is valid")
    .expect("attack");
    let mut advanced_value = serde_json::to_value(&advanced).expect("the world serializes");
    for key in [
        "combatants",
        "combat",
        "timeline",
        "events",
        "interners",
        "entities",
        "transforms",
    ] {
        advanced_value[key] = value[key].clone();
    }
    assert_ne!(base, state_hash(&load(&advanced_value)), "RNG is hashed");
}

/// Loads a surgically edited save, expecting validity.
fn load(value: &serde_json::Value) -> World {
    serde_json::from_value(value.clone()).expect("the surgical save stays valid")
}

#[test]
fn same_seed_repeats_mutation_changes_behavior() {
    let mut fixture = Fixture::standard();
    for creature in fixture.creatures.values_mut() {
        creature
            .stats
            .insert("health".to_owned(), Fx16_16::from_int(30_000));
    }
    let run = |seed: u64| {
        let (mut world, ids) = fixture.start(seed);
        let mut hashes = vec![state_hash(&world)];
        for _ in 0..4 {
            let active = world.combat().expect("combat").active.expect("a turn");
            let other = if active == ids[0] { ids[1] } else { ids[0] };
            perform_action(
                &mut world,
                &CombatAction::UseAbility {
                    actor: active,
                    ability: fixture.ability,
                    target: other,
                },
            )
            .expect("four actions stay valid at high health")
            .expect("attack");
            hashes.push(state_hash(&world));
        }
        hashes
    };
    assert_eq!(run(21), run(21), "same seed, content, and inputs repeat");

    // Authored damage changes independently expected behavior.
    let (mut damaged, ids) = fixture.start(9);
    let before = damaged.combatants().get(ids[0]).expect("A").health();
    perform_action(&mut damaged, &first_action(&ids, fixture.ability))
        .expect("B acts")
        .expect("attack");
    let dealt = before - damaged.combatants().get(ids[0]).expect("A").health();

    let mut heavy = Fixture::standard();
    for creature in heavy.creatures.values_mut() {
        creature
            .stats
            .insert("health".to_owned(), Fx16_16::from_int(30_000));
    }
    heavy
        .abilities
        .get_mut(&heavy.ability)
        .expect("ability")
        .damage = vec![
        DamageEntry {
            outcome: OutcomeWire::Success,
            amount: 9,
        },
        DamageEntry {
            outcome: OutcomeWire::Failure,
            amount: 0,
        },
    ];
    let (mut heavy_world, heavy_ids) = heavy.start(9);
    let heavy_before = heavy_world
        .combatants()
        .get(heavy_ids[0])
        .expect("A")
        .health();
    perform_action(&mut heavy_world, &first_action(&heavy_ids, heavy.ability))
        .expect("B acts")
        .expect("attack");
    let heavy_dealt = heavy_before
        - heavy_world
            .combatants()
            .get(heavy_ids[0])
            .expect("A")
            .health();
    assert_eq!(dealt, 2);
    assert_eq!(heavy_dealt, 9);

    // Authored thresholds change independently expected behavior.
    let mut easy = Fixture::standard();
    let table = easy.tables.get_mut(&Ulid::from_u128(104)).expect("table");
    table.bands[1].min_margin = 0;
    let (mut easy_world, easy_ids) = easy.start(7);
    let outcome = perform_action(&mut easy_world, &first_action(&easy_ids, easy.ability))
        .expect("B acts")
        .expect("attack");
    assert_eq!(outcome.margin, 0);
    assert_eq!(outcome.outcome, crpg_rules::Outcome::Failure);
    assert_eq!(outcome.damage, 0);
}

#[test]
fn error_display_spellings() {
    let fixture = Fixture::standard();
    let (mut world, ids) = fixture.start(5);
    let outsider = world.spawn(EntityMeta {});
    let cases: Vec<(CombatError, String)> = vec![
        (CombatError::NoEncounter, "NoEncounter at combat".to_owned()),
        (
            CombatError::EncounterActive,
            "EncounterActive at combat".to_owned(),
        ),
        (
            CombatError::NotParticipant { entity: outsider },
            format!("NotParticipant at combatants/{outsider:?}"),
        ),
        (
            CombatError::AbsentActor { actor: ids[0] },
            format!("AbsentActor at combatants/{:?}", ids[0]),
        ),
        (
            CombatError::AbsentTarget { target: ids[1] },
            format!("AbsentTarget at combatants/{:?}", ids[1]),
        ),
        (
            CombatError::DeadActor { actor: ids[0] },
            format!("DeadActor at combatants/{:?}", ids[0]),
        ),
        (
            CombatError::DeadTarget { target: ids[1] },
            format!("DeadTarget at combatants/{:?}", ids[1]),
        ),
        (
            CombatError::OutOfTurn {
                actor: ids[0],
                active: Some(ids[1]),
            },
            "OutOfTurn at combat/active".to_owned(),
        ),
        (
            CombatError::UnknownAbility {
                ability: fixture.ability,
            },
            "UnknownAbility at combat/ability".to_owned(),
        ),
        (
            CombatError::SelfTarget,
            "SelfTarget at combat/target".to_owned(),
        ),
        (
            CombatError::InsufficientAction {
                pool: Ulid::from_u128(105),
                cost: 1,
                current: 0,
            },
            "InsufficientAction at combat/pool".to_owned(),
        ),
        (
            CombatError::MissingPlacement {
                placement: fixture.placement_a,
            },
            "MissingPlacement at spec/placements".to_owned(),
        ),
        (
            CombatError::MixedArea {
                area_a: fixture.area,
                area_b: fixture.area,
            },
            "MixedArea at spec/areas".to_owned(),
        ),
        (
            CombatError::MissingCreature {
                prefab: fixture.creature_a,
            },
            "MissingCreature at spec/creatures".to_owned(),
        ),
        (
            CombatError::MissingStat {
                stat: "alpha".to_owned(),
            },
            "MissingStat at spec/stats".to_owned(),
        ),
        (
            CombatError::InvalidStatValue {
                stat: "health".to_owned(),
            },
            "InvalidStatValue at spec/stats".to_owned(),
        ),
        (
            CombatError::InvalidCost { cost: 2, max: 1 },
            "InvalidCost at spec/cost".to_owned(),
        ),
        (
            CombatError::InvalidDice,
            "InvalidDice at spec/dice".to_owned(),
        ),
        (
            CombatError::InvalidOutcomeTable,
            "InvalidOutcomeTable at spec/outcome-table".to_owned(),
        ),
        (
            CombatError::ValueOverflow,
            "ValueOverflow at spec/conversion".to_owned(),
        ),
        (
            CombatError::TooManyParticipants,
            "TooManyParticipants at spec/participants".to_owned(),
        ),
        (
            CombatError::TooManyStats,
            "TooManyStats at spec/stats".to_owned(),
        ),
        (
            CombatError::MissingPool {
                pool: Ulid::from_u128(105),
            },
            "MissingPool at spec/pools".to_owned(),
        ),
        (
            CombatError::MissingEffect {
                effect: Ulid::from_u128(208),
            },
            "MissingEffect at spec/effects".to_owned(),
        ),
        (
            CombatError::InvalidEffect {
                effect: Ulid::from_u128(208),
            },
            "InvalidEffect at spec/effects".to_owned(),
        ),
        (
            CombatError::DuplicateModifier {
                id: Ulid::from_u128(209),
            },
            "DuplicateModifier at spec/effects".to_owned(),
        ),
        (
            CombatError::InvalidNaturalDie { index: 2 },
            "InvalidNaturalDie at spec/ability".to_owned(),
        ),
        (
            CombatError::PolicyConflict {
                mod_type: "probe-type".to_owned(),
            },
            "PolicyConflict at spec/effects".to_owned(),
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(format!("{error}"), expected);
    }
}

#[test]
fn production_holds_no_ruleset_knowledge() {
    // Names, dice, damage, and thresholds belong in content: a production
    // branch on any of them fails this test.
    let banned = [
        "might",
        "guile",
        "strike",
        "minimal-d6",
        "goblin",
        "first-blood",
        "2d6",
        "combat_basic",
        "srd-lite",
        "srd_lite",
        "combat-srd",
        "combat_srd",
        "first-blood-srd",
        "ward",
        "focus",
        "heavy",
    ];
    for file in [
        "src/combat.rs",
        "src/world.rs",
        "src/event.rs",
        "src/lib.rs",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
        let text = std::fs::read_to_string(&path).expect("production source reads");
        for word in banned {
            assert!(
                !text.contains(word),
                "{file} must not name ruleset content {word:?}"
            );
        }
    }
}

#[test]
fn compat_migrated_v1_starts_with_primary_pool() {
    // Old /1 bytes load through the production migration with preserved
    // meaning and start with the primary pool definition.
    let ruleset_v1 = serde_json::json!({
        "schema": "crpg.ruleset/1",
        "id": support::uid(102),
        "slug": "probe-ruleset-v1",
        "name": "probe.ruleset",
        "package": "probe-pack",
        "version": "0.1.0",
        "stats": [
            {"name": "alpha", "kind": "int"},
            {"name": "beta", "kind": "int"},
            {"name": "gamma", "kind": "int"},
            {"name": "health", "kind": "int"}
        ],
        "health_stat": "health",
        "attributes": ["alpha", "beta", "gamma"],
        "action_pool": {
            "id": support::uid(105),
            "max": 1,
            "refresh": {"type": "on_turn_start"}
        },
        "abilities": [support::uid(103)]
    });
    let ability_v1 = serde_json::json!({
        "schema": "crpg.ability/1",
        "id": support::uid(103),
        "slug": "probe-attack-v1",
        "name": "probe.attack",
        "dice": "2d6",
        "attribute": "alpha",
        "outcome_table": support::uid(104),
        "damage": [
            {"outcome": {"type": "success"}, "amount": 2},
            {"outcome": {"type": "failure"}, "amount": 0}
        ],
        "cost": 1,
        "requires_target": true,
        "allow_self_target": false
    });
    let ruleset_bytes = crpg_data::canonical_json(&ruleset_v1).unwrap();
    let ability_bytes = crpg_data::canonical_json(&ability_v1).unwrap();
    let crpg_data::Document::Ruleset(ruleset) =
        crpg_data::read_document(&ruleset_bytes).expect("v1 ruleset migrates")
    else {
        panic!("migrated ruleset");
    };
    let crpg_data::Document::Ability(ability) =
        crpg_data::read_document(&ability_bytes).expect("v1 ability migrates")
    else {
        panic!("migrated ability");
    };
    assert_eq!(ruleset.pools.len(), 1);
    assert_eq!(ruleset.pools[0].max, 1);
    assert!(ability.extra_costs.is_empty());
    assert!(ability.ends_turn);
    assert_eq!(ability.effect, None);
    assert!(matches!(
        ability.defense,
        crpg_data::DefenseWire::ActorAttribute
    ));
    assert_eq!(ability.natural_die, None);
    // The migrated pair starts through the existing adapter with primary
    // pool semantics and unchanged behavior.
    let mut fixture = Fixture::standard();
    fixture.ruleset = ruleset;
    fixture.abilities.clear();
    fixture
        .abilities
        .insert(Ulid::from_u128(103), ability.clone());
    fixture.ability = Ulid::from_u128(103);
    let (world, ids) = fixture.start(5);
    assert_eq!(ids.len(), 2);
    let combat = world.combat().expect("an encounter is active");
    assert_eq!(combat.definition.cost, 1);
    assert_eq!(combat.definition.pool_max, 1);
}

#[test]
fn compat_first_listed_ability_preserved() {
    // Two legacy abilities: the first governs the legacy copies while both
    // listed abilities execute through the generalized path.
    let mut fixture = Fixture::standard();
    let second_id = Ulid::from_u128(206);
    let mut second = fixture
        .abilities
        .get(&fixture.ability)
        .expect("standard ability")
        .clone();
    second.id = second_id;
    second.slug = "probe-second".into();
    second.name = "probe.second".into();
    fixture.abilities.insert(second_id, second);
    fixture.ruleset.abilities.push(second_id);
    let (mut world, ids) = fixture.start(5);
    let combat = world.combat().expect("an encounter is active");
    assert_eq!(combat.definition.ability, fixture.ability);
    // The non-first listed ability now executes (multi-ability path).
    let before = snapshot(&world);
    let action = CombatAction::UseAbility {
        actor: ids[1],
        ability: second_id,
        target: ids[0],
    };
    assert!(
        perform_action(&mut world, &action)
            .expect("the second ability is valid")
            .expect("attack")
            .damage
            <= 2
    );
    assert_ne!(snapshot(&world), before);
}

#[test]
fn retired_gate_now_executes_plural_shapes() {
    // ADR-0016 retirement smoke: previously rejected plural shapes now
    // start through the generalized adapter (full execution lives in
    // combat_multi; this pins that the gate is gone).
    let mut fixture = Fixture::standard();
    let mut second_pool = fixture.ruleset.pools[0];
    second_pool.id = Ulid::from_u128(207);
    fixture.ruleset.pools.push(second_pool);
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
    let mut world = World::new(5);
    start_encounter(&mut world, &spec).expect("two pools now start");
    assert_eq!(world.combatants().len(), 2);
}
