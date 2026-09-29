//! Atomic same-pool affordability regressions (T019).
//!
//! Same-pool cost entries are summed as one total at the existing
//! affordability stage (after actor/target/turn/ability/self-target checks,
//! before RNG or mutation) in authored template order, primary first, with
//! checked addition (`ValueOverflow`) and `InsufficientAction` carrying the
//! total and original balance. First failing pool in template order wins.
//! All fixtures are hand-built neutral documents parsed through the
//! production data shapes; no engine vocabulary enters as knowledge.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Ulid};
use crpg_data::{AbilityCost, RefreshWire};
use crpg_sim::{
    perform_action, start_encounter, state_hash, CombatAction, CombatError, EncounterSpec, World,
    COMBAT_ROLL_STREAM,
};
use support::Fixture;

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

fn primary_pool() -> Ulid {
    Ulid::from_u128(105)
}

/// Single-pool fixture with `max`, primary `cost` plus same-pool `extra`
/// amounts, non-ending turns and no refresh so spends stay visible.
fn single_pool(cost: u32, extra: &[u32], max: u32) -> Fixture {
    let mut fixture = Fixture::standard();
    fixture.ruleset.pools[0].max = max;
    fixture.ruleset.pools[0].refresh = RefreshWire::Never;
    {
        let ability_id = fixture.ability;
        let primary = primary_pool();
        let ability = fixture.abilities.get_mut(&ability_id).expect("ability");
        ability.cost = cost;
        ability.extra_costs = extra
            .iter()
            .map(|amount| AbilityCost {
                pool: primary,
                amount: *amount,
            })
            .collect();
        ability.ends_turn = false;
    }
    fixture
}

fn active_ids(world: &World) -> (EntityId, EntityId) {
    let ids: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
    assert_eq!(ids.len(), 2);
    let active = world.combat().expect("combat").active.expect("turn");
    assert_eq!(active, ids[1], "B holds the first turn");
    (ids[0], ids[1])
}

#[test]
fn same_pool_sum_rejects_without_mutation() {
    let fixture = single_pool(3, &[3], 5);
    let (mut world, _) = fixture.start(5);
    let (_a, b) = active_ids(&world);
    let before_bytes = snapshot(&world);
    let before_hash = state_hash(&world);
    let action = CombatAction::UseAbility {
        actor: b,
        ability: fixture.ability,
        target: _a,
    };
    let result = perform_action(&mut world, &action);
    assert_eq!(
        result,
        Err(CombatError::InsufficientAction {
            pool: primary_pool(),
            cost: 6,
            current: 5,
        })
    );
    assert_eq!(
        format!("{}", result.unwrap_err()),
        "InsufficientAction at combat/pool"
    );
    assert_eq!(before_bytes, snapshot(&world));
    assert_eq!(before_hash, state_hash(&world));
    assert_no_combat_stream(&mut world);
    // Adapter accepted the shape: each entry fits individually.
    assert_eq!(
        world
            .combatants()
            .get(b)
            .expect("B")
            .action_pool()
            .current(),
        5
    );
}

#[test]
fn positive_controls_spend_exact_sums() {
    // 2+3 succeeds and spends 5.
    let fixture = single_pool(2, &[3], 5);
    let (mut world, _) = fixture.start(9);
    let (a, b) = active_ids(&world);
    let outcome = perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        },
    )
    .expect("2+3 is affordable")
    .expect("attack");
    // Seed 9 opens [2, 3] against alpha 6: RNG order unchanged.
    let faces: Vec<u32> = outcome.roll.dice.iter().map(|die| die.value).collect();
    assert_eq!(faces, vec![2, 3]);
    assert_eq!(
        world
            .combatants()
            .get(b)
            .expect("B")
            .action_pool()
            .current(),
        0,
        "2+3 spends exactly 5"
    );
    assert_eq!(world.combat().expect("combat").active, Some(b));

    // 2+2 leaves 1.
    let fixture = single_pool(2, &[2], 5);
    let (mut world, _) = fixture.start(9);
    let (a, b) = active_ids(&world);
    perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        },
    )
    .expect("2+2 is affordable")
    .expect("attack");
    assert_eq!(
        world
            .combatants()
            .get(b)
            .expect("B")
            .action_pool()
            .current(),
        1,
        "2+2 leaves exactly 1"
    );

    // Ordinary single cost works.
    let fixture = single_pool(1, &[], 5);
    let (mut world, _) = fixture.start(9);
    let (a, b) = active_ids(&world);
    perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        },
    )
    .expect("single cost works")
    .expect("attack");
    assert_eq!(
        world
            .combatants()
            .get(b)
            .expect("B")
            .action_pool()
            .current(),
        4
    );
}

#[test]
fn failed_attacks_spend_once() {
    // Seed 1 opens [6, 3]: total 9 against alpha 6, a failure that still spends.
    let fixture = single_pool(2, &[3], 5);
    let (mut world, _) = fixture.start(1);
    let (a, b) = active_ids(&world);
    let outcome = perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        },
    )
    .expect("failed attack is accepted")
    .expect("attack");
    assert_eq!(outcome.outcome, crpg_rules::Outcome::Failure);
    assert_eq!(
        world
            .combatants()
            .get(b)
            .expect("B")
            .action_pool()
            .current(),
        0,
        "even a failed attack spends its summed total once"
    );
}

fn two_pool_fixture() -> (Fixture, Ulid, Ulid) {
    let mut fixture = Fixture::standard();
    fixture.ruleset.pools[0].max = 5;
    fixture.ruleset.pools[0].refresh = RefreshWire::Never;
    let mut second = fixture.ruleset.pools[0];
    second.id = Ulid::from_u128(206);
    second.max = 5;
    second.refresh = crpg_data::RefreshWire::Never;
    fixture.ruleset.pools.push(second);
    {
        let ability = fixture
            .abilities
            .get_mut(&fixture.ability)
            .expect("ability");
        ability.ends_turn = false;
    }
    let first = fixture.ruleset.pools[0].id;
    let second_id = fixture.ruleset.pools[1].id;
    (fixture, first, second_id)
}

#[test]
fn multiple_pools_first_insufficient_wins_and_mutates_neither() {
    // Affordable first pool, repeated insufficient second pool.
    let (mut fixture, first, second) = two_pool_fixture();
    {
        let ability = fixture
            .abilities
            .get_mut(&fixture.ability)
            .expect("ability");
        ability.cost = 2;
        ability.extra_costs = vec![
            AbilityCost {
                pool: second,
                amount: 3,
            },
            AbilityCost {
                pool: second,
                amount: 3,
            },
        ];
    }
    let (mut world, _) = fixture.start(5);
    let (a, b) = active_ids(&world);
    let before = snapshot(&world);
    let before_hash = state_hash(&world);
    let result = perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        },
    );
    assert_eq!(
        result,
        Err(CombatError::InsufficientAction {
            pool: second,
            cost: 6,
            current: 5,
        })
    );
    assert_eq!(before, snapshot(&world));
    assert_eq!(before_hash, state_hash(&world));
    assert_eq!(
        world
            .combatants()
            .get(b)
            .expect("B")
            .action_pool()
            .current(),
        5,
        "affordable first pool is untouched on second-pool failure"
    );
    assert_eq!(
        world.combatants().get(b).expect("B").extra_pools()[0].current(),
        5
    );
    let _ = first;
}

#[test]
fn both_insufficient_reports_first_template_pool() {
    // Template order [900, 100]: ULID sort would pick 100 first, but the
    // controller must report 900 (first template).
    let mut fixture = Fixture::standard();
    fixture.ruleset.pools[0].id = Ulid::from_u128(900);
    fixture.ruleset.pools[0].max = 5;
    fixture.ruleset.pools[0].refresh = RefreshWire::Never;
    let mut second = fixture.ruleset.pools[0];
    second.id = Ulid::from_u128(100);
    second.max = 5;
    fixture.ruleset.pools.push(second);
    let first = Ulid::from_u128(900);
    let second_id = Ulid::from_u128(100);
    assert!(second_id < first, "ULIDs sort opposite template order");
    {
        let ability = fixture
            .abilities
            .get_mut(&fixture.ability)
            .expect("ability");
        ability.cost = 3;
        ability.extra_costs = vec![
            AbilityCost {
                pool: first,
                amount: 3,
            },
            AbilityCost {
                pool: second_id,
                amount: 3,
            },
            AbilityCost {
                pool: second_id,
                amount: 3,
            },
        ];
        ability.ends_turn = false;
    }
    let (mut world, _) = fixture.start(5);
    let (a, b) = active_ids(&world);
    let before = snapshot(&world);
    let result = perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        },
    );
    assert_eq!(
        result,
        Err(CombatError::InsufficientAction {
            pool: first,
            cost: 6,
            current: 5,
        }),
        "first template pool wins even when ULIDs sort differently"
    );
    assert_eq!(before, snapshot(&world));
}

#[test]
fn noncontiguous_repeats_sum_in_template_order() {
    // Costs [(P1,3),(P2,2),(P1,3)] are noncontiguous but must still sum P1=6.
    let (mut fixture, first, second) = two_pool_fixture();
    {
        let ability = fixture
            .abilities
            .get_mut(&fixture.ability)
            .expect("ability");
        ability.cost = 3;
        ability.extra_costs = vec![
            AbilityCost {
                pool: first,
                amount: 3,
            },
            AbilityCost {
                pool: second,
                amount: 2,
            },
        ];
        ability.ends_turn = false;
    }
    let (world, _) = fixture.start(5);
    let mut value = serde_json::to_value(&world).expect("serializes");
    // Reorder to noncontiguous: [P1,3],[P2,2],[P1,3].
    let pool_a = first.to_string();
    let pool_b = second.to_string();
    value["combat"]["definition"]["abilities"][0]["costs"] =
        serde_json::json!([[pool_a, 3], [pool_b, 2], [pool_a, 3],]);
    // Legacy cost stays the first P1 amount for coherence.
    value["combat"]["definition"]["cost"] = serde_json::Value::from(3);
    let mut reordered: World = serde_json::from_value(value).expect("reorder loads");
    let ids: Vec<EntityId> = reordered.combatants().iter().map(|(id, _)| id).collect();
    let active = reordered.combat().expect("combat").active.expect("turn");
    assert_eq!(active, ids[1]);
    let before = snapshot(&reordered);
    let result = perform_action(
        &mut reordered,
        &CombatAction::UseAbility {
            actor: ids[1],
            ability: fixture.ability,
            target: ids[0],
        },
    );
    assert_eq!(
        result,
        Err(CombatError::InsufficientAction {
            pool: first,
            cost: 6,
            current: 5,
        })
    );
    assert_eq!(before, snapshot(&reordered));
}

#[test]
fn zero_entries_are_rejected_before_execution() {
    // Adapter keeps its per-entry validation: zero extra amounts fail there.
    let mut fixture = Fixture::standard();
    fixture.ruleset.pools[0].max = 5;
    {
        let ability = fixture
            .abilities
            .get_mut(&fixture.ability)
            .expect("ability");
        ability.cost = 2;
        ability.extra_costs = vec![AbilityCost {
            pool: Ulid::from_u128(105),
            amount: 0,
        }];
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
    assert_eq!(
        start_encounter(&mut world, &spec),
        Err(CombatError::InvalidCost { cost: 0, max: 5 })
    );
    // A zero amount smuggled into a save fails to load, never executing.
    let good = single_pool(2, &[2], 5);
    let (world, _) = good.start(5);
    let mut value = serde_json::to_value(&world).expect("serializes");
    let pool = primary_pool().to_string();
    value["combat"]["definition"]["abilities"][0]["costs"] =
        serde_json::json!([[pool, 2], [pool, 0],]);
    assert!(
        serde_json::from_value::<World>(value).is_err(),
        "zero cost entries must fail to load"
    );
}

#[test]
fn deplete_then_reject_with_reload_evidence() {
    // Two abilities: single 2 to deplete 5->3, then 2+2=4 rejecting from 3
    // although each 2 fits individually.
    let mut fixture = Fixture::standard();
    fixture.ruleset.pools[0].max = 5;
    fixture.ruleset.pools[0].refresh = RefreshWire::Never;
    let second_id = Ulid::from_u128(206);
    let mut second = fixture
        .abilities
        .get(&fixture.ability)
        .expect("ability")
        .clone();
    second.id = second_id;
    second.slug = "probe-second".to_owned();
    second.name = "probe.second".to_owned();
    second.cost = 2;
    second.extra_costs = vec![AbilityCost {
        pool: Ulid::from_u128(105),
        amount: 2,
    }];
    second.ends_turn = false;
    {
        let first = fixture
            .abilities
            .get_mut(&fixture.ability)
            .expect("ability");
        first.cost = 2;
        first.extra_costs.clear();
        first.ends_turn = false;
    }
    fixture.abilities.insert(second_id, second);
    fixture.ruleset.abilities.push(second_id);

    let (mut world, _) = fixture.start(5);
    let ids: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
    let actor = ids[1];
    let target = ids[0];
    // Legal spend: 5 -> 3.
    perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor,
            ability: fixture.ability,
            target,
        },
    )
    .expect("depleting spend succeeds")
    .expect("attack");
    assert_eq!(
        world
            .combatants()
            .get(actor)
            .expect("actor")
            .action_pool()
            .current(),
        3
    );
    // Save/reload before the rejected action.
    let bytes = snapshot(&world);
    let hash = state_hash(&world);
    let mut loaded: World = serde_json::from_slice(&bytes).expect("reloads");
    assert_eq!(snapshot(&loaded), bytes);
    assert_eq!(state_hash(&loaded), hash);
    // Repeated total rejects although each 2 fits in 3.
    let probe = CombatAction::UseAbility {
        actor,
        ability: second_id,
        target,
    };
    let expected = Err(CombatError::InsufficientAction {
        pool: Ulid::from_u128(105),
        cost: 4,
        current: 3,
    });
    assert_eq!(perform_action(&mut world, &probe), expected);
    assert_eq!(bytes, snapshot(&world));
    assert_eq!(perform_action(&mut loaded, &probe), expected);
    assert_eq!(bytes, snapshot(&loaded));
    assert_eq!(state_hash(&world), state_hash(&loaded));
}

fn overflow_world(total: serde_json::Value, first_amount: u32) -> (World, Vec<EntityId>, Ulid) {
    let fixture = single_pool(1, &[], u32::MAX);
    // Start with a coherent max-MAX world, then shape its costs via surgery.
    // The fixture above uses max MAX with a single cost; surgery replaces the
    // costs vector while keeping legacy agreement (cost == first amount).
    let (world, _) = fixture.start(5);
    let pool = primary_pool();
    let mut value = serde_json::to_value(&world).expect("serializes");
    value["combat"]["definition"]["pools"][0]["max"] = serde_json::Value::from(u32::MAX);
    value["combat"]["definition"]["pool_max"] = serde_json::Value::from(u32::MAX);
    value["combat"]["definition"]["abilities"][0]["costs"] = total;
    value["combat"]["definition"]["cost"] = serde_json::Value::from(first_amount);
    for entry in value["combatants"].as_array_mut().expect("combatants") {
        entry[1]["action_pool"]["max"] = serde_json::Value::from(u32::MAX);
        entry[1]["action_pool"]["current"] = serde_json::Value::from(u32::MAX);
    }
    let loaded: World = serde_json::from_value(value).expect("overflow shape loads");
    let ids: Vec<EntityId> = loaded.combatants().iter().map(|(id, _)| id).collect();
    (loaded, ids, pool)
}

#[test]
fn checked_overflow_returns_value_overflow_without_mutation() {
    let pool = Ulid::from_u128(105).to_string();
    let max = u32::MAX;
    let (mut world, ids, expected_pool) =
        overflow_world(serde_json::json!([[pool, max], [pool, 1]]), max);
    assert_eq!(expected_pool, Ulid::from_u128(105));
    let active = world.combat().expect("combat").active.expect("turn");
    assert_eq!(active, ids[1]);
    let before = snapshot(&world);
    let before_hash = state_hash(&world);
    let result = perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: ids[1],
            ability: Ulid::from_u128(103),
            target: ids[0],
        },
    );
    assert_eq!(result, Err(CombatError::ValueOverflow));
    assert_eq!(
        format!("{}", CombatError::ValueOverflow),
        "ValueOverflow at spec/conversion"
    );
    assert_eq!(before, snapshot(&world));
    assert_eq!(before_hash, state_hash(&world));
    assert_no_combat_stream(&mut world);
}

#[test]
fn largest_representable_total_does_not_overflow() {
    // Total exactly u32::MAX is representable: affordable from full spends to 0.
    let pool = Ulid::from_u128(105).to_string();
    let max = u32::MAX;
    let (mut world, ids, _) =
        overflow_world(serde_json::json!([[pool, max - 1], [pool, 1]]), max - 1);
    let before = snapshot(&world);
    let result = perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: ids[1],
            ability: Ulid::from_u128(103),
            target: ids[0],
        },
    );
    assert!(
        result.is_ok(),
        "total u32::MAX from full balance must not overflow, got {result:?}"
    );
    assert_ne!(before, snapshot(&world));
    assert_eq!(
        world
            .combatants()
            .get(ids[1])
            .expect("actor")
            .action_pool()
            .current(),
        0
    );

    // Same total from MAX-1 is insufficient with the full total, not overflow.
    let (world, ids, expected_pool) =
        overflow_world(serde_json::json!([[pool, max - 1], [pool, 1]]), max - 1);
    let mut value = serde_json::to_value(&world).expect("serializes");
    for entry in value["combatants"].as_array_mut().expect("combatants") {
        if entry[0] == serde_json::to_value(ids[1]).expect("id") {
            entry[1]["action_pool"]["current"] = serde_json::Value::from(max - 1);
        }
    }
    let mut world: World = serde_json::from_value(value).expect("loads");
    let before = snapshot(&world);
    let result = perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: ids[1],
            ability: Ulid::from_u128(103),
            target: ids[0],
        },
    );
    assert_eq!(
        result,
        Err(CombatError::InsufficientAction {
            pool: expected_pool,
            cost: max,
            current: max - 1,
        })
    );
    assert_eq!(before, snapshot(&world));
}

#[test]
fn earlier_errors_win_over_cost_errors() {
    let pool = Ulid::from_u128(105).to_string();
    let max = u32::MAX;
    let (mut world, ids, _) = overflow_world(serde_json::json!([[pool, max], [pool, 1]]), max);
    // OutOfTurn beats ValueOverflow.
    assert_eq!(
        perform_action(
            &mut world,
            &CombatAction::UseAbility {
                actor: ids[0],
                ability: Ulid::from_u128(103),
                target: ids[1],
            },
        ),
        Err(CombatError::OutOfTurn {
            actor: ids[0],
            active: Some(ids[1])
        })
    );
    // UnknownAbility beats ValueOverflow.
    assert_eq!(
        perform_action(
            &mut world,
            &CombatAction::UseAbility {
                actor: ids[1],
                ability: Ulid::from_u128(999),
                target: ids[0],
            },
        ),
        Err(CombatError::UnknownAbility {
            ability: Ulid::from_u128(999)
        })
    );
    // SelfTarget beats ValueOverflow.
    assert_eq!(
        perform_action(
            &mut world,
            &CombatAction::UseAbility {
                actor: ids[1],
                ability: Ulid::from_u128(103),
                target: ids[1],
            },
        ),
        Err(CombatError::SelfTarget)
    );
    // NotParticipant beats ValueOverflow.
    let outsider = world.spawn(crpg_sim::EntityMeta {});
    let before = snapshot(&world);
    assert_eq!(
        perform_action(
            &mut world,
            &CombatAction::UseAbility {
                actor: outsider,
                ability: Ulid::from_u128(103),
                target: ids[0],
            },
        ),
        Err(CombatError::NotParticipant { entity: outsider })
    );
    assert_eq!(before, snapshot(&world));
}
