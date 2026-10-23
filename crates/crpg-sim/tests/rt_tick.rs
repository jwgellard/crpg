//! The real-time clock and execution stages (T138a, ADR-0029 T3-T8, T12,
//! T15-T16): R5, R6, R7, R8, R9, R13 and R15.
//!
//! Expected values are worked out by hand from the contract: stage 2 runs the
//! clock, stage 3 runs the due orders, and every scenario states the tick on
//! which each pool, key and order must change.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Fx16_16};
use crpg_data::{ActionPoolTemplate, RefreshWire};
use crpg_sim::{
    end_encounter, legal_actions, perform_action, request_mode, state_hash, tick, CombatAction,
    CombatError, EncounterMode, EntityMeta, LegalActionsError, Transform, World, RT_CLOCK_MAX,
};
use support::rt::{
    bytes_of, capable, capable_trio, edit_ability, elapsed, make_lethal, pool_of, tick_pools,
    ticks, timeline_of, use_action, value_of,
};
use support::Fixture;

const MAX_KEY: i32 = i32::MAX;

/// Extra-pool balances of one combatant, template order.
fn extras(world: &World, id: EntityId) -> Vec<u32> {
    world
        .combatants()
        .get(id)
        .expect("combatant")
        .extra_pools()
        .iter()
        .map(|pool| pool.current())
        .collect()
}

/// The world as JSON without the tick counter.
fn without_tick(world: &World) -> serde_json::Value {
    let mut value = value_of(world);
    value.as_object_mut().expect("world").remove("tick");
    value
}

/// Gives creature `id`'s health a large value so it survives long scripts.
fn tough(fixture: &mut Fixture, creature: crpg_core::Ulid) {
    fixture
        .creatures
        .get_mut(&creature)
        .expect("creature")
        .stats
        .insert("health".to_owned(), Fx16_16::from_int(1000));
}

/// Loads a world from JSON through the validated path.
fn load(value: serde_json::Value) -> World {
    serde_json::from_value(value).expect("the edited save loads")
}

/// R5 (B8): in RT the clock grows by exactly one per tick.
#[test]
fn r5_rt_clock_advances_one_per_tick() {
    let (mut world, _) = capable(4).start_rt(1, EncounterMode::RealTime);
    for expected in 1..=9_u64 {
        tick(&mut world);
        assert_eq!(elapsed(&world), expected);
    }
    // The world tick is separate and still advances.
    assert_eq!(world.tick().get(), 9);
}

/// R5 (B8): a TB rollover adds exactly `round_ticks`; nothing moves the clock
/// between rollovers, in a TB-only encounter, or when terminal.
#[test]
fn r5_tb_rollover_adds_round_ticks() {
    let fixture = capable(4);
    let (mut world, ids) = fixture.start(1);
    assert_eq!(elapsed(&world), 0);
    perform_action(&mut world, &CombatAction::EndTurn { actor: ids[1] }).expect("B ends");
    assert_eq!(elapsed(&world), 0, "mid-round");
    ticks(&mut world, 3);
    assert_eq!(elapsed(&world), 0, "TB ticks never advance the clock");
    perform_action(&mut world, &CombatAction::EndTurn { actor: ids[0] }).expect("A ends");
    assert_eq!(elapsed(&world), 4, "one rollover, one round");
    assert_eq!(world.combat().expect("encounter").round, 1);
    perform_action(&mut world, &CombatAction::EndTurn { actor: ids[1] }).expect("B ends");
    perform_action(&mut world, &CombatAction::EndTurn { actor: ids[0] }).expect("A ends");
    assert_eq!(elapsed(&world), 8);

    // A TB-only encounter has no clock to move.
    let (mut world, ids) = Fixture::standard().start(1);
    perform_action(&mut world, &CombatAction::EndTurn { actor: ids[1] }).expect("B ends");
    perform_action(&mut world, &CombatAction::EndTurn { actor: ids[0] }).expect("A ends");
    assert!(world.combat().expect("encounter").clock.is_none());
}

/// R5 (B8): a terminal encounter never moves its clock, in either mode.
#[test]
fn r5_terminal_encounters_do_not_advance() {
    let mut fixture = capable(4);
    make_lethal(&mut fixture, 100);

    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1])).expect("order");
    tick(&mut world);
    assert_eq!(elapsed(&world), 1);
    let frozen = without_tick(&world);
    ticks(&mut world, 12);
    assert_eq!(elapsed(&world), 1, "terminal RT");
    assert_eq!(frozen, without_tick(&world));

    let (mut world, ids) = fixture.start(1);
    perform_action(&mut world, &use_action(ids[1], fixture.ability, ids[0])).expect("kill");
    assert_eq!(world.combat().expect("encounter").active, None);
    ticks(&mut world, 12);
    assert_eq!(elapsed(&world), 0, "terminal TB");
}

/// R5 (B8): the TB jump saturates at the bound rather than passing it.
#[test]
fn r5_tb_rollover_saturates_at_the_bound() {
    let fixture = capable(4);
    let (world, ids) = fixture.start(1);
    let mut value = value_of(&world);
    value["combat"]["clock"]["elapsed"] = serde_json::json!(RT_CLOCK_MAX - 1);
    let mut world = load(value);
    perform_action(&mut world, &CombatAction::EndTurn { actor: ids[1] }).expect("B ends");
    perform_action(&mut world, &CombatAction::EndTurn { actor: ids[0] }).expect("A ends");
    assert_eq!(elapsed(&world), RT_CLOCK_MAX, "min(old + 4, bound)");
    perform_action(&mut world, &CombatAction::EndTurn { actor: ids[1] }).expect("B ends");
    perform_action(&mut world, &CombatAction::EndTurn { actor: ids[0] }).expect("A ends");
    assert_eq!(elapsed(&world), RT_CLOCK_MAX, "stays at the bound");
    assert_eq!(
        world.combat().expect("encounter").round,
        2,
        "TB play goes on"
    );
}

/// R5 (B12, D4): the clock freezes at the bound. The tick that reaches it
/// does nothing else, later ticks change nothing, and commands still apply.
#[test]
fn r5_clock_freezes_at_the_bound() {
    let mut fixture = capable(4);
    make_lethal(&mut fixture, 2);
    let (world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    let mut value = value_of(&world);
    value["combat"]["clock"]["elapsed"] = serde_json::json!(RT_CLOCK_MAX - 2);
    let mut world = load(value);
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("A's order");

    // One tick below the bound still executes: A acts and is re-keyed at the
    // bound (clamped), and its one-shot order clears.
    tick(&mut world);
    assert_eq!(elapsed(&world), RT_CLOCK_MAX - 1);
    assert_eq!(pool_of(&world, a), 0, "A spent its action");
    assert_eq!(world.combatants().get(b).expect("B").health(), 4);
    assert!(world.combatants().get(a).expect("A").order().is_none());
    assert_eq!(timeline_of(&world), vec![(0, b), (MAX_KEY, a)]);

    // B receives an order and is due, but the tick that reaches the bound
    // does nothing else.
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B's order");
    tick(&mut world);
    assert_eq!(elapsed(&world), RT_CLOCK_MAX);
    assert_eq!(pool_of(&world, b), 1, "B did not act on the freezing tick");
    assert_eq!(world.combatants().get(a).expect("A").health(), 10);
    assert!(world.combatants().get(b).expect("B").order().is_some());
    assert_eq!(timeline_of(&world), vec![(0, b), (MAX_KEY, a)], "no re-key");

    // Ten more ticks leave the encounter's bytes alone (the world tick moves).
    let frozen = without_tick(&world);
    ticks(&mut world, 10);
    assert_eq!(frozen, without_tick(&world));
    assert_eq!(world.tick().get(), 12, "the world tick still advances");

    // Commands, requests, despawn and release still apply.
    let mut probe = world.clone();
    perform_action(&mut probe, &CombatAction::Stop { actor: b }).expect("Stop applies");
    assert!(probe.combatants().get(b).expect("B").order().is_none());
    let mut probe = world.clone();
    request_mode(&mut probe, EncounterMode::TurnBased).expect("request applies");
    ticks(&mut probe, 8);
    assert_eq!(
        probe.combat().expect("encounter").pending_mode,
        Some(EncounterMode::TurnBased),
        "a frozen clock never reaches a boundary"
    );
    let mut probe = world.clone();
    assert!(probe.despawn(a));
    assert_eq!(probe.combatants().len(), 1);
    let mut probe = world.clone();
    end_encounter(&mut probe).expect("release applies");
}

/// R6 (B9): `OnTick` pools refresh exactly when the clock crosses a multiple
/// of their period, in RT.
#[test]
fn r6_on_tick_pools_in_rt() {
    let mut fixture = capable(6);
    // Periods 1, 3, 4, round_ticks (6) and 7 (never lands on a boundary).
    tick_pools(&mut fixture, 1, &[1, 3, 4, 6, 7]);
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("A's order");

    // Tick 1: delivery first (everything full), then A spends every pool.
    tick(&mut world);
    assert_eq!(extras(&world, a), vec![0, 0, 0, 0, 0]);
    let expected: [(&str, [u32; 5]); 6] = [
        ("tick 2", [3, 0, 0, 0, 0]),
        ("tick 3", [3, 3, 0, 0, 0]),
        ("tick 4", [3, 3, 3, 0, 0]),
        ("tick 5", [3, 3, 3, 0, 0]),
        ("tick 6", [3, 3, 3, 3, 0]),
        ("tick 7", [3, 3, 3, 3, 3]),
    ];
    for (label, balances) in expected {
        tick(&mut world);
        assert_eq!(extras(&world, a), balances.to_vec(), "{label}");
    }
    // The untouched combatant was full throughout.
    assert_eq!(extras(&world, b), vec![3, 3, 3, 3, 3]);
}

/// R6 (B9): across TB rollovers the delivered tick is the last multiple the
/// jump crossed, so a jump that lands on no multiple still refreshes; a pool
/// whose next multiple lies beyond the jump does not; a TB-only encounter
/// delivers nothing.
#[test]
fn r6_on_tick_pools_across_tb_rollovers() {
    let mut fixture = capable(6);
    tick_pools(&mut fixture, 1, &[1, 3, 4, 6, 7]);
    let (mut world, ids) = fixture.start(1);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B attacks");
    assert_eq!(extras(&world, b), vec![0, 0, 0, 0, 0]);
    assert_eq!(elapsed(&world), 0);
    // No delivery before the rollover.
    ticks(&mut world, 5);
    assert_eq!(extras(&world, b), vec![0, 0, 0, 0, 0]);
    // A's turn ends the round: the clock jumps 0 -> 6. Periods 1, 3, 4 and 6
    // cross a multiple (4 without landing on one); 7 does not.
    perform_action(&mut world, &CombatAction::EndTurn { actor: a }).expect("A ends");
    assert_eq!(elapsed(&world), 6);
    assert_eq!(extras(&world, b), vec![3, 3, 3, 3, 0]);
    // The next jump, 6 -> 12, crosses 7 without landing on it.
    perform_action(&mut world, &CombatAction::EndTurn { actor: b }).expect("B ends");
    perform_action(&mut world, &CombatAction::EndTurn { actor: a }).expect("A ends");
    assert_eq!(elapsed(&world), 12);
    assert_eq!(extras(&world, b), vec![3, 3, 3, 3, 3]);

    // TB-only: the same pools, no clock, no delivery.
    let mut plain = Fixture::standard();
    tick_pools(&mut plain, 1, &[1, 3, 4, 6, 7]);
    let (mut world, ids) = plain.start(1);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(b, plain.ability, a)).expect("B attacks");
    perform_action(&mut world, &CombatAction::EndTurn { actor: a }).expect("A ends");
    perform_action(&mut world, &CombatAction::EndTurn { actor: b }).expect("B ends");
    perform_action(&mut world, &CombatAction::EndTurn { actor: a }).expect("A ends");
    assert_eq!(world.combat().expect("encounter").round, 2);
    assert_eq!(extras(&world, b), vec![0, 0, 0, 0, 0], "nothing delivered");
}

/// One step record of a TB script.
#[derive(Debug, PartialEq)]
struct Step {
    outcome: String,
    active: Option<EntityId>,
    round: u32,
    timeline: Vec<(i32, EntityId)>,
}

/// Plays a deterministic TB script to its terminal and records every step.
fn play(world: &mut World, ability: crpg_core::Ulid) -> Vec<Step> {
    let mut steps = Vec::new();
    for index in 0..200 {
        let Some(actor) = world.combat().and_then(|combat| combat.active) else {
            break;
        };
        let target = world
            .combatants()
            .iter()
            .map(|(id, _)| id)
            .find(|id| *id != actor && !world.combatants().get(*id).expect("c").dead())
            .expect("a foe");
        let action = if index % 5 == 4 {
            CombatAction::EndTurn { actor }
        } else {
            use_action(actor, ability, target)
        };
        let outcome = perform_action(world, &action).expect("legal step");
        tick(world);
        steps.push(Step {
            outcome: format!("{outcome:?}"),
            active: world.combat().expect("encounter").active,
            round: world.combat().expect("encounter").round,
            timeline: timeline_of(world),
        });
    }
    steps
}

/// R7 (B10): an RT-capable TB fight matches the TB-only fixture in heads,
/// pops, rollovers, outcomes and terminal; only `clock` differs.
#[test]
fn r7_rt_capable_tb_fight_matches_tb_only() {
    let mut only = Fixture::standard();
    only.set_alpha(100, 100);
    let mut cap = capable(4);
    cap.set_alpha(100, 100);
    let (mut world_only, _) = only.start(7);
    let (mut world_cap, _) = cap.start(7);
    let script_only = play(&mut world_only, only.ability);
    let script_cap = play(&mut world_cap, cap.ability);
    assert!(script_only.len() > 5, "the script spans several rounds");
    assert_eq!(script_only.last().expect("steps").active, None, "terminal");
    assert_eq!(script_only, script_cap);

    let mut cap_value = value_of(&world_cap);
    let removed = cap_value["combat"]
        .as_object_mut()
        .expect("combat")
        .remove("clock");
    assert!(removed.is_some(), "the RT-capable fight carries a clock");
    assert_eq!(cap_value, value_of(&world_only), "only `clock` differs");
    assert!(
        elapsed(&world_cap) > 0 && elapsed(&world_cap).is_multiple_of(4),
        "rollovers advanced the clock by whole rounds"
    );
}

/// R8 (B11): the RT timeline holds one entry per alive combatant, keyed 0
/// after an RT start and at the clock after a TB to RT switch.
#[test]
fn r8_keys_after_start_and_switch() {
    let fixture = capable_trio(4);
    let (world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    assert_eq!(
        timeline_of(&world),
        vec![(0, ids[0]), (0, ids[1]), (0, ids[2])]
    );

    // TB to RT: one full round (C, B, A) then the rollover applies the switch.
    let (mut world, ids) = fixture.start(1);
    request_mode(&mut world, EncounterMode::RealTime).expect("request");
    for actor in [ids[2], ids[1], ids[0]] {
        perform_action(&mut world, &CombatAction::EndTurn { actor }).expect("end");
    }
    assert_eq!(
        world.combat().expect("encounter").mode,
        EncounterMode::RealTime
    );
    assert_eq!(world.combat().expect("encounter").active, None);
    assert_eq!(
        timeline_of(&world),
        vec![(4, ids[0]), (4, ids[1]), (4, ids[2])],
        "keyed at the clock, ascending (key, id)"
    );
}

/// R8 (B11, B14): after an execution the actor is re-keyed at
/// `elapsed + recovery`, clamped at the bound; entries stay ordered; a kill
/// removes the dead combatant's entry.
#[test]
fn r8_keys_after_execution() {
    // Default recovery is round_ticks.
    let fixture = capable_trio(10);
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1])).expect("order");
    tick(&mut world);
    assert_eq!(
        timeline_of(&world),
        vec![(0, ids[1]), (0, ids[2]), (11, ids[0])],
        "elapsed 1 + round_ticks 10, after the due entries"
    );

    // An authored recovery wins.
    let mut fixture = capable_trio(10);
    edit_ability(&mut fixture, |ability| ability.recovery_ticks = Some(3));
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1])).expect("order");
    tick(&mut world);
    assert_eq!(timeline_of(&world)[2], (4, ids[0]), "1 + 3");

    // Clamped at the bound: elapsed bound - 4 + default recovery 10.
    let fixture = capable_trio(10);
    let (world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    let mut value = value_of(&world);
    value["combat"]["clock"]["elapsed"] = serde_json::json!(RT_CLOCK_MAX - 5);
    let mut world = load(value);
    perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1])).expect("order");
    tick(&mut world);
    assert_eq!(timeline_of(&world)[2], (MAX_KEY, ids[0]), "clamped");

    // A kill removes the victim's entry; the actor is re-keyed.
    let mut fixture = capable_trio(10);
    make_lethal(&mut fixture, 100);
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1])).expect("order");
    tick(&mut world);
    assert_eq!(timeline_of(&world), vec![(0, ids[2]), (11, ids[0])]);
    assert!(world.combatants().get(ids[1]).expect("B").order().is_none());
}

/// R8 (B14): the due set is taken before acting, so a combatant acts at most
/// once per tick even with the shortest recovery.
#[test]
fn r8_at_most_one_action_per_tick() {
    let mut fixture = capable(50);
    let creature_b = fixture.creature_b;
    tough(&mut fixture, creature_b);
    tick_pools(&mut fixture, 100, &[]);
    edit_ability(&mut fixture, |ability| {
        ability.recovery_ticks = Some(1);
        ability.repeats_in_real_time = Some(true);
    });
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1])).expect("order");
    for count in 1..=10_u32 {
        tick(&mut world);
        assert_eq!(pool_of(&world, ids[0]), 100 - count, "tick {count}");
    }
    assert!(
        world.combatants().get(ids[0]).expect("A").order().is_some(),
        "a repeating order stays"
    );
    assert_eq!(pool_of(&world, ids[1]), 100, "B holds no order and waits");
}

/// R8 (B13): a boundary increments the round once and refreshes both
/// `RoundStart` and `TurnStart` pools of every alive combatant.
#[test]
fn r8_boundary_round_and_refresh() {
    let mut fixture = capable(4);
    // Primary pool OnTurnStart (max 1), one extra pool OnRoundStart (max 3).
    fixture.ruleset.pools = vec![
        ActionPoolTemplate {
            id: crpg_core::Ulid::from_u128(105),
            max: 1,
            refresh: RefreshWire::OnTurnStart,
        },
        ActionPoolTemplate {
            id: crpg_core::Ulid::from_u128(106),
            max: 3,
            refresh: RefreshWire::OnRoundStart,
        },
    ];
    edit_ability(&mut fixture, |ability| {
        ability.extra_costs = vec![crpg_data::AbilityCost {
            pool: crpg_core::Ulid::from_u128(106),
            amount: 3,
        }];
    });
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("order");
    let mut rounds = Vec::new();
    let mut after = Vec::new();
    for _ in 0..8 {
        tick(&mut world);
        rounds.push(world.combat().expect("encounter").round);
        after.push((pool_of(&world, a), extras(&world, a)[0]));
    }
    assert_eq!(rounds, vec![0, 0, 0, 1, 1, 1, 1, 2]);
    // Tick 1 spends both pools; the tick-4 boundary refills both at once.
    assert_eq!(after[0], (0, 0));
    assert_eq!(after[2], (0, 0));
    assert_eq!(after[3], (1, 3), "boundary: TurnStart and RoundStart pools");
    assert_eq!(world.combat().expect("encounter").active, None);
}

/// R8 (B17): a due combatant without an order keeps its key and acts on the
/// first tick after it receives one.
#[test]
fn r8_orderless_due_combatant_waits() {
    let fixture = capable(4);
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    ticks(&mut world, 3);
    assert_eq!(timeline_of(&world), vec![(0, a), (0, b)], "keys unchanged");
    assert_eq!(pool_of(&world, b), 1);
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("order at tick 3");
    assert_eq!(pool_of(&world, b), 1, "admission does not act");
    tick(&mut world);
    // Stage 2 refreshes at the boundary first, then stage 3 spends.
    assert_eq!(
        pool_of(&world, b),
        0,
        "B acted on the first tick after its order"
    );
    assert_eq!(
        timeline_of(&world),
        vec![(0, a), (8, b)],
        "4 + round_ticks 4"
    );
    assert!(world.combatants().get(b).expect("B").order().is_none());
}

/// Drained `Died` payload entities, tick stamps ignored.
fn deaths(world: &World) -> Vec<EntityId> {
    world
        .clone()
        .events_mut()
        .drain()
        .into_iter()
        .filter_map(|envelope| match envelope.payload {
            crpg_sim::SimEvent::Died { entity } => Some(entity),
            _ => None,
        })
        .collect()
}

/// R9 (B15): an RT execution is the TB resolution: the same draw, damage,
/// spend and exactly-once `Died`.
#[test]
fn r9_execution_shares_the_tb_resolution_path() {
    for damage in [2, 100] {
        let mut fixture = capable(4);
        make_lethal(&mut fixture, damage);
        // TB: B (initiative 9) acts first on A.
        let (mut tb, ids) = fixture.start(21);
        let outcome = perform_action(&mut tb, &use_action(ids[1], fixture.ability, ids[0]))
            .expect("TB attack")
            .expect("an attack");
        // RT: B's standing order resolves on the first tick.
        let (mut rt, ids) = fixture.start_rt(21, EncounterMode::RealTime);
        perform_action(&mut rt, &use_action(ids[1], fixture.ability, ids[0])).expect("order");
        assert_eq!(
            value_of(&rt)["rng"],
            value_of(&fixture.start_rt(21, EncounterMode::RealTime).0)["rng"],
            "admission draws nothing"
        );
        tick(&mut rt);
        assert_eq!(value_of(&tb)["rng"], value_of(&rt)["rng"], "same draw");
        let tb_a = tb.combatants().get(ids[0]).expect("A");
        let rt_a = rt.combatants().get(ids[0]).expect("A");
        assert_eq!(tb_a.health(), rt_a.health());
        assert_eq!(tb_a.dead(), rt_a.dead());
        assert_eq!(pool_of(&tb, ids[1]), pool_of(&rt, ids[1]));
        assert_eq!(outcome.target_died, damage == 100);
        assert_eq!(deaths(&tb), deaths(&rt), "exactly-once Died, same entity");
        assert_eq!(deaths(&rt).len(), usize::from(damage == 100));
    }
}

/// R9 (B15): a one-shot order clears after executing; a repeating one stays.
#[test]
fn r9_one_shot_clears_and_repeat_stays() {
    for repeats in [false, true] {
        let mut fixture = capable(4);
        edit_ability(&mut fixture, |ability| {
            ability.repeats_in_real_time = Some(repeats);
        });
        let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
        perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1])).expect("order");
        let stored = world.combatants().get(ids[0]).expect("A").order();
        assert_eq!(stored.map(|order| order.repeat), Some(repeats));
        tick(&mut world);
        assert_eq!(pool_of(&world, ids[0]), 0, "it executed");
        assert_eq!(
            world.combatants().get(ids[0]).expect("A").order().is_some(),
            repeats
        );
    }
}

/// R9 (B16): a due order that fails validation is cleared with no spend, no
/// draw and no re-key.
#[test]
fn r9_unaffordable_due_order_clears_without_effect() {
    let mut fixture = capable(50);
    edit_ability(&mut fixture, |ability| {
        ability.recovery_ticks = Some(1);
        ability.repeats_in_real_time = Some(true);
    });
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("order");
    tick(&mut world);
    assert_eq!(pool_of(&world, a), 0);
    assert!(world.combatants().get(a).expect("A").order().is_some());
    assert_eq!(timeline_of(&world), vec![(0, b), (2, a)]);
    let after_first = value_of(&world);
    // Tick 2: A is due again, cannot pay, and its order ends.
    tick(&mut world);
    let mut expected = after_first;
    expected["tick"] = serde_json::json!(2);
    expected["combat"]["clock"]["elapsed"] = serde_json::json!(2);
    let combatants = expected["combatants"].as_array_mut().expect("pair list");
    for pair in combatants {
        if let Some(object) = pair[1].as_object_mut() {
            object.remove("order");
        }
    }
    assert_eq!(
        expected,
        value_of(&world),
        "only the order and the clock moved"
    );
    assert_eq!(timeline_of(&world), vec![(0, b), (2, a)], "still due");
    // Tick 3: no order, nothing happens.
    tick(&mut world);
    assert_eq!(pool_of(&world, a), 0);
}

/// R9 (B16): an order whose target died earlier in the pass clears with no
/// spend and no draw.
#[test]
fn r9_dead_target_order_clears_without_effect() {
    let mut fixture = capable_trio(50);
    make_lethal(&mut fixture, 100);
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("A's order");
    let mut control = world.clone();
    perform_action(&mut world, &use_action(c, fixture.ability, b)).expect("C's order");
    tick(&mut world);
    tick(&mut control);
    assert!(
        world.combatants().get(b).expect("B").dead(),
        "A killed B first"
    );
    assert!(
        world.combatants().get(c).expect("C").order().is_none(),
        "cleared"
    );
    assert_eq!(pool_of(&world, c), 1, "no spend");
    assert_eq!(
        value_of(&world)["rng"],
        value_of(&control)["rng"],
        "no draw"
    );
    assert!(timeline_of(&world).contains(&(0, c)), "C is still due");
    assert_eq!(timeline_of(&world), timeline_of(&control));
}

/// R9 (B15, B18): a kill that leaves fewer than two alive ends the pass and
/// the encounter; later ticks change nothing.
#[test]
fn r9_kill_stops_the_pass() {
    let mut fixture = capable(4);
    make_lethal(&mut fixture, 100);
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("A's order");
    let mut control = world.clone();
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B's order");
    tick(&mut world);
    tick(&mut control);
    assert!(world.combatants().get(b).expect("B").dead());
    assert_eq!(
        world.combatants().get(a).expect("A").health(),
        10,
        "B never acted"
    );
    assert!(world.combatants().get(b).expect("B").order().is_none());
    assert_eq!(
        value_of(&world)["rng"],
        value_of(&control)["rng"],
        "one draw"
    );
    assert_eq!(timeline_of(&world), vec![(5, a)]);
    let frozen = without_tick(&world);
    ticks(&mut world, 30);
    assert_eq!(frozen, without_tick(&world), "terminal: nothing moves");
}

/// R9 (B15, B18): a self-targeted kill leaves the survivor terminal.
#[test]
fn r9_self_kill_is_terminal() {
    let mut fixture = capable(4);
    make_lethal(&mut fixture, 100);
    edit_ability(&mut fixture, |ability| ability.allow_self_target = true);
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(a, fixture.ability, a)).expect("order");
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B's order");
    tick(&mut world);
    assert!(world.combatants().get(a).expect("A").dead());
    assert!(!world.combatants().get(b).expect("B").dead());
    assert_eq!(pool_of(&world, b), 1, "B never acted");
    assert_eq!(timeline_of(&world), vec![(0, b)], "A's entry is gone");
    assert_eq!(deaths(&world), vec![a]);
}

/// R9 (B18): an RT encounter published with a single participant is terminal
/// from the start.
#[test]
fn r9_single_participant_rt_is_terminal() {
    let mut fixture = capable(4);
    fixture.encounter.participants.truncate(1);
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    let a = ids[0];
    ticks(&mut world, 9);
    assert_eq!(elapsed(&world), 0, "no clock advance");
    assert_eq!(
        legal_actions(&world, a),
        Err(LegalActionsError::Actor(CombatError::OutOfTurn {
            actor: a,
            active: None
        }))
    );
    for action in [
        CombatAction::Stop { actor: a },
        use_action(a, fixture.ability, a),
    ] {
        assert_eq!(
            perform_action(&mut world, &action),
            Err(CombatError::OutOfTurn {
                actor: a,
                active: None
            })
        );
    }
}

/// R13 (B29): a non-participant with a `Transform` lives on beside a fight in
/// either mode, untouched and unkeyed.
#[test]
fn r13_non_participants_are_untouched() {
    let transform = Transform {
        position: [1.0, 2.0, 3.0],
        velocity: [0.25, 0.0, -0.5],
    };
    let fixture = capable(4);
    for mode in [EncounterMode::TurnBased, EncounterMode::RealTime] {
        let (mut world, ids) = fixture.start_rt(1, mode);
        let before = world.spawn(EntityMeta {});
        world.transforms_mut().insert(before, transform);
        // One standing order keeps the RT stages busy.
        if mode == EncounterMode::RealTime {
            perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1]))
                .expect("order");
        }
        ticks(&mut world, 1_000);
        assert!(world.contains(before));
        assert_eq!(world.transforms().get(before), Some(&transform));
        assert!(
            timeline_of(&world).iter().all(|(_, id)| *id != before),
            "{mode:?}: never keyed"
        );
        assert!(world.combatants().get(before).is_none());
        assert!(world.timeline().len() <= ids.len());
    }
}

/// R13 (B32): repeated RT `legal_actions` queries change nothing.
#[test]
fn r13_legal_actions_is_read_only() {
    let fixture = capable_trio(4);
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1])).expect("order");
    tick(&mut world);
    let hash = state_hash(&world);
    let bytes = bytes_of(&world);
    for _ in 0..5 {
        for id in &ids {
            legal_actions(&world, *id).expect("each combatant may order");
        }
    }
    assert_eq!(hash, state_hash(&world));
    assert_eq!(bytes, bytes_of(&world));
}

/// One scripted RT/TB run: per-tick hashes plus the modes visited.
fn scripted_run(seed: u64) -> (Vec<[u8; 32]>, Vec<EncounterMode>) {
    let mut fixture = capable_trio(10);
    for creature in [fixture.creature_a, fixture.creature_b] {
        tough(&mut fixture, creature);
    }
    let c_creature = *fixture
        .creatures
        .keys()
        .find(|id| id.to_u128() == 123)
        .expect("creature C");
    tough(&mut fixture, c_creature);
    edit_ability(&mut fixture, |ability| {
        ability.recovery_ticks = Some(3);
        ability.repeats_in_real_time = Some(true);
    });
    let (mut world, ids) = fixture.start_rt(seed, EncounterMode::RealTime);
    let mut hashes = Vec::new();
    let mut modes = Vec::new();
    for step in 0..1_000_usize {
        let mode = world.combat().expect("encounter").mode;
        match mode {
            EncounterMode::RealTime => {
                if step % 7 == 0 {
                    let actor = ids[(step / 7) % 3];
                    let target = ids[(step / 7 + 1 + step % 2) % 3];
                    let _ = perform_action(&mut world, &use_action(actor, fixture.ability, target));
                }
                if step % 53 == 0 {
                    let _ = perform_action(&mut world, &CombatAction::Stop { actor: ids[1] });
                }
            }
            EncounterMode::TurnBased => {
                if let Some(actor) = world.combat().and_then(|combat| combat.active) {
                    let target = ids[(step + 1) % 3];
                    let action = if step % 2 == 0 && target != actor {
                        use_action(actor, fixture.ability, target)
                    } else {
                        CombatAction::EndTurn { actor }
                    };
                    let _ = perform_action(&mut world, &action);
                }
            }
        }
        if step % 97 == 0 {
            let other = match mode {
                EncounterMode::RealTime => EncounterMode::TurnBased,
                EncounterMode::TurnBased => EncounterMode::RealTime,
            };
            request_mode(&mut world, other).expect("capable");
        }
        tick(&mut world);
        modes.push(mode);
        hashes.push(state_hash(&world));
    }
    (hashes, modes)
}

/// R15 (B35): two runs of one RT script give equal per-tick hashes, and the
/// script really exercises both modes.
#[test]
fn r15_two_runs_of_one_script_agree() {
    let (first, modes) = scripted_run(42);
    let (second, _) = scripted_run(42);
    assert_eq!(first.len(), 1_000);
    assert_eq!(first, second);
    assert!(modes.contains(&EncounterMode::RealTime));
    assert!(modes.contains(&EncounterMode::TurnBased));
    let distinct: std::collections::BTreeSet<_> = first.iter().collect();
    assert!(distinct.len() > 100, "the run is lively");
    let (other, _) = scripted_run(43);
    assert_ne!(first, other, "a different seed diverges");
}
