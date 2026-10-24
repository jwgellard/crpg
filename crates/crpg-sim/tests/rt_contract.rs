//! T138a two-keys contract tests (B1–B35), written from `tasks/T138.md`
//! §"Decision", §"Interface", §"Tick order" and §"Behaviour" only.
//!
//! These tests exercise the encounter-mode and real-time policy surface
//! through the public API named in the contract, reusing the shared
//! fixtures in `support`. No test here reads any `src/` implementation;
//! failures are reported as suspected defects, never papered over.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Ulid};
use crpg_sim::{
    end_encounter, legal_actions, perform_action, request_mode, start_encounter_in_mode,
    state_hash, tick, CombatAction, CombatError, EncounterClock, EncounterMode, EncounterSpec,
    EntityMeta, LegalActionsError, StandingOrder, Transform, World, COMBAT_ROLL_STREAM,
    RT_CLOCK_MAX,
};
use support::rt::{self, use_action};
use support::Fixture;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn active(world: &World) -> Option<EntityId> {
    world.combat().expect("combat").active
}

fn round(world: &World) -> u32 {
    world.combat().expect("combat").round
}

fn mode(world: &World) -> EncounterMode {
    world.combat().expect("combat").mode
}

fn pending(world: &World) -> Option<EncounterMode> {
    world.combat().expect("combat").pending_mode
}

fn clock(world: &World) -> Option<EncounterClock> {
    world.combat().expect("combat").clock
}

fn elapsed(world: &World) -> u64 {
    world
        .combat()
        .expect("combat")
        .clock
        .expect("clock")
        .elapsed
}

fn order_of(world: &World, id: EntityId) -> Option<StandingOrder> {
    world.combatants().get(id).expect("combatant").order()
}

fn pool_of(world: &World, id: EntityId) -> u32 {
    world
        .combatants()
        .get(id)
        .expect("combatant")
        .action_pool()
        .current()
}

fn extra_pool_of(world: &World, id: EntityId, index: usize) -> u32 {
    world.combatants().get(id).expect("combatant").extra_pools()[index].current()
}

fn health_of(world: &World, id: EntityId) -> u32 {
    world.combatants().get(id).expect("combatant").health()
}

fn dead_of(world: &World, id: EntityId) -> bool {
    world.combatants().get(id).expect("combatant").dead()
}

fn timeline_of(world: &World) -> Vec<(i32, EntityId)> {
    world.timeline().iter().map(|(k, id)| (k.0, id)).collect()
}

fn value_of(world: &World) -> serde_json::Value {
    serde_json::to_value(world).expect("the world serializes")
}

fn bytes_of(world: &World) -> Vec<u8> {
    serde_json::to_vec(world).expect("the world serializes")
}

fn combat_bytes(world: &World) -> serde_json::Value {
    value_of(world)["combat"].clone()
}

/// The encounter's own state (combat, combatants, timeline), excluding the
/// world tick counter and event queue, which always advance/track time.
fn encounter_state(world: &World) -> serde_json::Value {
    let v = value_of(world);
    serde_json::json!({
        "combat": v["combat"].clone(),
        "combatants": v["combatants"].clone(),
        "timeline": v["timeline"].clone(),
    })
}

/// Asserts one surgically corrupted save fails to load with `expected`
/// somewhere in the load error message.
fn rejects_with(
    base: &serde_json::Value,
    expected: &str,
    mutate: impl FnOnce(&mut serde_json::Value),
) {
    let mut save = base.clone();
    mutate(&mut save);
    let err = serde_json::from_value::<World>(save).expect_err("must fail to load");
    assert!(
        err.to_string().contains(expected),
        "expected load error to contain {expected:?}, got {err}"
    );
}

// ---------------------------------------------------------------------------
// B1, B2 — mode is encounter-scoped and retained until release
// ---------------------------------------------------------------------------

#[test]
fn b1_b2_mode_is_encounter_scoped_and_retained_until_release() {
    // No encounter: no mode exists anywhere on the world.
    let empty = World::new(1);
    assert!(empty.combat().is_none());

    // A terminal RT encounter keeps mode and pending_mode until released.
    let mut fixture = rt::capable(4);
    rt::make_lethal(&mut fixture, 10);
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(mode(&world), EncounterMode::RealTime);
    request_mode(&mut world, EncounterMode::TurnBased).expect("request");
    assert_eq!(pending(&world), Some(EncounterMode::TurnBased));
    // B kills A: the encounter becomes terminal.
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("order");
    tick(&mut world);
    assert!(dead_of(&world, a));
    // Becoming terminal never rewrites mode or pending_mode.
    assert_eq!(mode(&world), EncounterMode::RealTime);
    assert_eq!(pending(&world), Some(EncounterMode::TurnBased));
    assert!(clock(&world).is_some());

    // Release drops mode, clock and pending mode along with the state.
    end_encounter(&mut world).expect("release");
    assert!(world.combat().is_none());
    assert!(world.combatants().is_empty());
}

// ---------------------------------------------------------------------------
// B3 — clock publication and ability RT fields
// ---------------------------------------------------------------------------

#[test]
fn b3_clock_and_ability_fields_on_publication() {
    // RT-capable ruleset publishes a clock whatever the mode; TB-only does not.
    let capable = rt::capable(4);
    let (rt_world, _) = capable.start_rt(7, EncounterMode::RealTime);
    assert_eq!(
        clock(&rt_world),
        Some(EncounterClock {
            round_ticks: 4,
            elapsed: 0
        })
    );
    assert_eq!(pending(&rt_world), None);

    let (tb_world, _) = capable.start_mode(7, EncounterMode::TurnBased).expect("tb");
    assert_eq!(
        clock(&tb_world),
        Some(EncounterClock {
            round_ticks: 4,
            elapsed: 0
        }),
        "an RT-capable TB fight still has a clock"
    );

    let tb_only = Fixture::standard();
    let (tb_only_world, _) = tb_only.start(7);
    assert_eq!(clock(&tb_only_world), None);

    // Ability RT fields are copied in an RT-capable encounter, whatever the mode.
    let mut with_fields = rt::capable(4);
    rt::edit_ability(&mut with_fields, |a| {
        a.recovery_ticks = Some(2);
        a.repeats_in_real_time = Some(true);
    });
    let (with_world, _) = with_fields
        .start_mode(7, EncounterMode::TurnBased)
        .expect("tb");
    let def = &with_world.combat().expect("combat").definition;
    assert_eq!(def.abilities[0].recovery_ticks, Some(2));
    assert!(def.abilities[0].repeats_in_real_time);

    // A TB-only encounter drops the authored RT fields to None/false.
    let mut tb_only_fields = Fixture::standard();
    rt::edit_ability(&mut tb_only_fields, |a| {
        a.recovery_ticks = Some(2);
        a.repeats_in_real_time = Some(true);
    });
    let (dropped_world, _) = tb_only_fields.start(7);
    let def = &dropped_world.combat().expect("combat").definition;
    assert_eq!(def.abilities[0].recovery_ticks, None);
    assert!(!def.abilities[0].repeats_in_real_time);
}

// ---------------------------------------------------------------------------
// B4, B30 — ModeUnsupported and EncounterActive precedence
// ---------------------------------------------------------------------------

#[test]
fn b4_b30_mode_unsupported_and_encounter_active_precedence() {
    // RT on a TB-only ruleset is ModeUnsupported, and nothing changes.
    let fixture = Fixture::standard();
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
    let mut fresh = World::new(7);
    let before = bytes_of(&fresh);
    assert_eq!(
        start_encounter_in_mode(&mut fresh, &spec, EncounterMode::RealTime),
        Err(CombatError::ModeUnsupported)
    );
    assert_eq!(
        before,
        bytes_of(&fresh),
        "no entity, event, RNG or interner change"
    );
    assert_eq!(fresh.events().len(), 0);
    assert!(!fresh.rng_mut().has_stream(COMBAT_ROLL_STREAM));

    // A second start reports EncounterActive before ModeUnsupported (B30).
    let (mut world, _) = fixture.start(5);
    assert_eq!(
        start_encounter_in_mode(&mut world, &spec, EncounterMode::RealTime),
        Err(CombatError::EncounterActive)
    );
}

// ---------------------------------------------------------------------------
// B5 — data-owned zero preconditions panic
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "ruleset round_ticks is validated nonzero by data read-time checks")]
fn b5a_zero_round_ticks_panics() {
    let fixture = rt::capable(0);
    let _ = fixture.start_mode(7, EncounterMode::RealTime);
}

#[test]
#[should_panic(expected = "ability recovery_ticks is validated nonzero by data read-time checks")]
fn b5b_zero_recovery_ticks_panics() {
    let mut fixture = rt::capable(4);
    rt::edit_ability(&mut fixture, |a| a.recovery_ticks = Some(0));
    let _ = fixture.start_mode(7, EncounterMode::RealTime);
}

// ---------------------------------------------------------------------------
// B6 — TB-only worlds write none of the six RT keys
// ---------------------------------------------------------------------------

fn no_rt_keys(world: &World) -> bool {
    let keys = rt::json_keys(&value_of(world));
    rt::RT_KEYS.iter().all(|k| !keys.contains(*k))
}

#[test]
fn b6_tb_only_worlds_write_no_rt_keys() {
    let fixture = Fixture::standard();
    let (mut world, ids) = fixture.start(7);
    assert!(no_rt_keys(&world));
    // Still absent after actions and ticks.
    for _ in 0..2 {
        let actor = active(&world).expect("turn");
        let other = if actor == ids[0] { ids[1] } else { ids[0] };
        perform_action(&mut world, &use_action(actor, fixture.ability, other)).expect("acts");
    }
    tick(&mut world);
    tick(&mut world);
    assert!(no_rt_keys(&world));
}

// ---------------------------------------------------------------------------
// B7 — RT fields force the vectors; save/load round trips
// ---------------------------------------------------------------------------

#[test]
fn b7_legacy_shape_round_trips() {
    // No RT field: legacy shape (no `abilities` vector) survives unchanged.
    let legacy = rt::capable(4);
    let (legacy_world, _) = legacy.start_rt(7, EncounterMode::RealTime);
    let legacy_value = value_of(&legacy_world);
    assert!(
        legacy_value["combat"]["definition"]
            .get("abilities")
            .is_none(),
        "a field-free single-ability encounter keeps the legacy shape"
    );

    // With an RT field: the vectors appear and save/load gives an equal world.
    let mut vector = rt::capable(4);
    rt::edit_ability(&mut vector, |a| {
        a.recovery_ticks = Some(2);
        a.repeats_in_real_time = Some(true);
    });
    let (vector_world, _) = vector.start_rt(7, EncounterMode::RealTime);
    let vector_value = value_of(&vector_world);
    assert!(
        vector_value["combat"]["definition"]
            .get("abilities")
            .is_some(),
        "RT fields force the abilities vector"
    );
    assert!(vector_value["combat"]["definition"].get("pools").is_some());
    assert!(vector_value["combat"]["definition"]
        .get("effects")
        .is_some());
    let loaded: World = serde_json::from_value(vector_value).expect("the RT-capable save loads");
    assert_eq!(
        bytes_of(&vector_world),
        bytes_of(&loaded),
        "save then load gives an equal world"
    );
}

// ---------------------------------------------------------------------------
// B8 — the clock advances only by the stated rules
// ---------------------------------------------------------------------------

#[test]
fn b8_clock_advances_only_by_rules() {
    // RT: +1 per tick while alive and below the bound.
    let fixture = rt::capable(4);
    let (mut world, _) = fixture.start_rt(7, EncounterMode::RealTime);
    for n in 1..=5u64 {
        tick(&mut world);
        assert_eq!(elapsed(&world), n);
    }

    // TB rollover of an RT-capable encounter: +round_ticks (not +1).
    let (mut tb, ids) = fixture.start_mode(7, EncounterMode::TurnBased).expect("tb");
    assert_eq!(elapsed(&tb), 0, "no clock change in TB before a rollover");
    perform_action(&mut tb, &use_action(ids[1], fixture.ability, ids[0])).expect("B acts");
    assert_eq!(elapsed(&tb), 0, "still no clock change between rollovers");
    perform_action(&mut tb, &use_action(ids[0], fixture.ability, ids[1])).expect("A acts");
    assert_eq!(elapsed(&tb), 4, "one TB rollover adds round_ticks");

    // A TB-only encounter has no clock at all.
    let (tb_only, _) = Fixture::standard().start(7);
    assert!(clock(&tb_only).is_none());
}

// ---------------------------------------------------------------------------
// B9 — OnTick pools refresh only on clock crossings
// ---------------------------------------------------------------------------

#[test]
fn b9_on_tick_pools_refresh_only_on_crossings() {
    // round_ticks = 4; an OnTick(3) extra pool (max 3); ability costs 3 extra.
    let mut fixture = rt::capable(4);
    rt::tick_pools(&mut fixture, 1, &[3]);
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        perform_action(&mut world, &use_action(b, fixture.ability, a)),
        Ok(None)
    );
    // Tick 1: clock 0->1; B executes and spends the OnTick(3) pool.
    tick(&mut world);
    assert_eq!(extra_pool_of(&world, b, 0), 0, "the pool was spent");
    // Tick 2: clock 1->2; no multiple of 3 crossed, so no refresh.
    tick(&mut world);
    assert_eq!(
        extra_pool_of(&world, b, 0),
        0,
        "no refresh without a crossing"
    );
    // Tick 3: clock 2->3; 3/3 > 0/3, exactly one refresh.
    tick(&mut world);
    assert_eq!(
        extra_pool_of(&world, b, 0),
        3,
        "exactly one refresh on the crossing"
    );
}

// ---------------------------------------------------------------------------
// B10 — an RT-capable TB fight behaves exactly as a TB-only fight
// ---------------------------------------------------------------------------

#[test]
fn b10_rt_capable_tb_fight_matches_tb_only() {
    let run = |capable: bool| -> (Vec<u32>, u32, Option<EntityId>) {
        let fixture = if capable {
            rt::capable(4)
        } else {
            Fixture::standard()
        };
        let (mut world, ids) = fixture.start_mode(7, EncounterMode::TurnBased).expect("tb");
        for _ in 0..2 {
            let actor = active(&world).expect("turn");
            let other = if actor == ids[0] { ids[1] } else { ids[0] };
            perform_action(&mut world, &use_action(actor, fixture.ability, other)).expect("acts");
        }
        let healths = vec![health_of(&world, ids[0]), health_of(&world, ids[1])];
        (healths, round(&world), active(&world))
    };
    assert_eq!(run(true), run(false));
}

// ---------------------------------------------------------------------------
// B11 — RT timeline keys are clock-based
// ---------------------------------------------------------------------------

#[test]
fn b11_rt_keys_are_clock_based() {
    let mut fixture = rt::capable(4);
    rt::edit_ability(&mut fixture, |a| a.recovery_ticks = Some(2));
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    // RT start: exactly one entry per alive combatant, every key 0.
    assert_eq!(timeline_of(&world), vec![(0, a), (0, b)]);
    assert_eq!(world.timeline().len(), 2);
    // After an execution the actor is re-keyed at elapsed + recovery_ticks.
    assert_eq!(
        perform_action(&mut world, &use_action(b, fixture.ability, a)),
        Ok(None)
    );
    tick(&mut world); // clock -> 1; B executes and is re-keyed to 1 + 2.
    assert_eq!(timeline_of(&world), vec![(0, a), (3, b)]);
}

// ---------------------------------------------------------------------------
// B12 — the clock freezes at RT_CLOCK_MAX
// ---------------------------------------------------------------------------

#[test]
fn b12_clock_freezes_at_max() {
    let fixture = rt::capable(4);
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        perform_action(&mut world, &use_action(b, fixture.ability, a)),
        Ok(None),
        "B holds a due order (key 0)"
    );
    let mut value = value_of(&world);
    value["combat"]["clock"]["elapsed"] = serde_json::json!(RT_CLOCK_MAX - 1);
    let mut world: World = serde_json::from_value(value).expect("the near-bound save loads");

    // The freeze tick reaches the bound and does nothing else: the due order
    // is not executed, nothing is spent.
    tick(&mut world);
    assert_eq!(elapsed(&world), RT_CLOCK_MAX);
    assert_eq!(
        order_of(&world, b),
        Some(StandingOrder {
            ability: fixture.ability,
            target: a,
            repeat: false,
        }),
        "the due order was not executed"
    );
    assert_eq!(pool_of(&world, b), 1, "no spend on the freeze tick");

    // From here the encounter is frozen: later ticks change nothing in it.
    let frozen = combat_bytes(&world);
    let tick_before = world.tick();
    for _ in 0..10 {
        tick(&mut world);
    }
    assert_eq!(combat_bytes(&world), frozen, "frozen ticks change nothing");
    assert_ne!(world.tick(), tick_before, "the world tick still advances");
}

// ---------------------------------------------------------------------------
// B13 — rt_round_start increments the round and refreshes
// ---------------------------------------------------------------------------

#[test]
fn b13_round_start_increments_and_refreshes() {
    let fixture = rt::capable(4);
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(round(&world), 0);
    assert_eq!(active(&world), None);
    assert_eq!(
        perform_action(&mut world, &use_action(b, fixture.ability, a)),
        Ok(None)
    );
    tick(&mut world); // clock 1; B executes and spends its primary pool.
    assert_eq!(pool_of(&world, b), 0);
    for _ in 0..3 {
        tick(&mut world); // clocks 2, 3, 4 -> boundary at 4
    }
    assert_eq!(round(&world), 1, "the boundary starts a round");
    assert_eq!(active(&world), None, "active stays None in RT");
    assert_eq!(
        pool_of(&world, b),
        1,
        "RoundStart then TurnStart refresh the pool"
    );
}

// ---------------------------------------------------------------------------
// B14 — a due snapshot acts at most once per combatant per tick
// ---------------------------------------------------------------------------

#[test]
fn b14_due_snapshot_acts_at_most_once_per_tick() {
    let mut fixture = rt::capable(4);
    rt::edit_ability(&mut fixture, |a| a.repeats_in_real_time = Some(true));
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        perform_action(&mut world, &use_action(b, fixture.ability, a)),
        Ok(None)
    );
    tick(&mut world);
    // B acted once: spent once, re-keyed out of the snapshot to 1 + 4, and
    // its repeating order survives (a second execution would have cleared it
    // or spent a second time).
    assert_eq!(timeline_of(&world), vec![(0, a), (5, b)]);
    assert_eq!(pool_of(&world, b), 0, "spent exactly once");
    assert_eq!(
        order_of(&world, b),
        Some(StandingOrder {
            ability: fixture.ability,
            target: a,
            repeat: true,
        })
    );
}

// ---------------------------------------------------------------------------
// B15 — RT resolution draws like TB and clears one-shot orders
// ---------------------------------------------------------------------------

#[test]
fn b15_rt_resolution_matches_tb_and_one_shot_clears() {
    // Seed 9 opens [2, 3]: B succeeds against alpha 6 for 2 damage.
    let tb_fixture = Fixture::standard();
    let (mut tb_world, tb_ids) = tb_fixture.start(9);
    let before = health_of(&tb_world, tb_ids[0]);
    perform_action(
        &mut tb_world,
        &use_action(tb_ids[1], tb_fixture.ability, tb_ids[0]),
    )
    .expect("B acts")
    .expect("attack");
    let tb_dealt = before - health_of(&tb_world, tb_ids[0]);

    let fixture = rt::capable(4);
    let (mut world, ids) = fixture.start_rt(9, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    let before = health_of(&world, a);
    assert_eq!(
        perform_action(&mut world, &use_action(b, fixture.ability, a)),
        Ok(None),
        "RT order setting returns Ok(None)"
    );
    tick(&mut world); // stage 3 executes B's order
    let dealt = before - health_of(&world, a);
    assert_eq!(dealt, tb_dealt, "RT resolves through the same draw as TB");
    assert_eq!(dealt, 2);
    assert_eq!(order_of(&world, b), None, "a one-shot order clears");
}

// ---------------------------------------------------------------------------
// B16 — an invalid due order clears without spending or re-keying
// ---------------------------------------------------------------------------

#[test]
fn b16_invalid_due_order_clears_without_spend() {
    // recovery_ticks (2) < round_ticks (4): a repeating order comes due again
    // before the boundary refunds the pool, so it fails validation.
    let mut fixture = rt::capable(4);
    rt::edit_ability(&mut fixture, |a| {
        a.recovery_ticks = Some(2);
        a.repeats_in_real_time = Some(true);
    });
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        perform_action(&mut world, &use_action(b, fixture.ability, a)),
        Ok(None)
    );
    tick(&mut world); // B executes, pool 1->0, re-keyed to 1 + 2 = 3, order stays.
    assert_eq!(pool_of(&world, b), 0);
    assert!(order_of(&world, b).is_some());
    tick(&mut world); // clock 1->2, not due
    tick(&mut world); // clock 2->3, B due; unaffordable -> cleared, no re-key.
    assert_eq!(
        order_of(&world, b),
        None,
        "the unaffordable order is cleared"
    );
    assert_eq!(
        timeline_of(&world),
        vec![(0, a), (3, b)],
        "no re-key: the entry stays due"
    );
    assert_eq!(pool_of(&world, b), 0, "no spend on the failing tick");
}

// ---------------------------------------------------------------------------
// B17 — an orderless due combatant keeps its key and waits
// ---------------------------------------------------------------------------

#[test]
fn b17_orderless_due_combatant_waits() {
    let fixture = rt::capable(4);
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        perform_action(&mut world, &use_action(a, fixture.ability, b)),
        Ok(None)
    );
    tick(&mut world);
    // A executed (re-keyed to 5); B never acted, so B keeps key 0.
    assert_eq!(timeline_of(&world), vec![(0, b), (5, a)]);
    assert_eq!(pool_of(&world, b), 1, "B never spent");
    // B receives an order and acts on the first tick after, since it is due.
    assert_eq!(
        perform_action(&mut world, &use_action(b, fixture.ability, a)),
        Ok(None)
    );
    tick(&mut world);
    assert_eq!(
        pool_of(&world, b),
        0,
        "B acted on the first tick after its order"
    );
}

// ---------------------------------------------------------------------------
// B18 — RT terminal stops the pass; a tiny RT start is terminal from the start
// ---------------------------------------------------------------------------

#[test]
fn b18_rt_terminal_stops_the_pass() {
    // A lethal encounter: A kills B on the first execution, then the pass stops.
    let mut fixture = rt::capable(4);
    rt::make_lethal(&mut fixture, 10);
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("A's order");
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B's order");
    tick(&mut world);
    assert!(dead_of(&world, b), "A killed B first (A id < B id)");
    assert_eq!(active(&world), None);
    let bytes = encounter_state(&world);
    for _ in 0..5 {
        tick(&mut world);
    }
    assert_eq!(
        encounter_state(&world),
        bytes,
        "terminal ticks change nothing"
    );

    // Fewer than two participants is terminal from the start.
    let mut single = Fixture::standard();
    single.encounter.participants.pop();
    single.ruleset.round_ticks = Some(4);
    let (mut single_world, sids) = single
        .start_mode(7, EncounterMode::RealTime)
        .expect("starts");
    assert_eq!(single_world.combatants().len(), 1);
    assert_eq!(
        perform_action(
            &mut single_world,
            &use_action(sids[0], single.ability, sids[0])
        ),
        Err(CombatError::OutOfTurn {
            actor: sids[0],
            active: None,
        }),
        "the terminal gate fires before SelfTarget in RT"
    );
}

// ---------------------------------------------------------------------------
// B19 — RT UseAbility admission ladder and order store
// ---------------------------------------------------------------------------

#[test]
fn b19_rt_use_ability_admission_and_order_store() {
    // NoEncounter first.
    let mut empty = World::new(1);
    let stray = empty.spawn(EntityMeta {});
    assert_eq!(
        perform_action(&mut empty, &use_action(stray, Ulid::from_u128(103), stray)),
        Err(CombatError::NoEncounter)
    );

    let fixture = rt::capable(4);
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    let outsider = world.spawn(EntityMeta {});
    let gone = world.spawn(EntityMeta {});
    assert!(world.despawn(gone));

    // Nonparticipant actor, then nonparticipant target.
    assert_eq!(
        perform_action(&mut world, &use_action(outsider, fixture.ability, a)),
        Err(CombatError::NotParticipant { entity: outsider })
    );
    assert_eq!(
        perform_action(&mut world, &use_action(b, fixture.ability, outsider)),
        Err(CombatError::NotParticipant { entity: outsider })
    );
    // Absent actor, then absent target.
    assert_eq!(
        perform_action(&mut world, &use_action(gone, fixture.ability, a)),
        Err(CombatError::AbsentActor { actor: gone })
    );
    assert_eq!(
        perform_action(&mut world, &use_action(b, fixture.ability, gone)),
        Err(CombatError::AbsentTarget { target: gone })
    );
    // Unknown ability, then self-target.
    assert_eq!(
        perform_action(&mut world, &use_action(b, Ulid::from_u128(999), a)),
        Err(CombatError::UnknownAbility {
            ability: Ulid::from_u128(999),
        })
    );
    assert_eq!(
        perform_action(&mut world, &use_action(b, fixture.ability, b)),
        Err(CombatError::SelfTarget)
    );

    // On success: stores the order, returns Ok(None), and changes nothing else.
    let key_before = timeline_of(&world);
    let pool_before = pool_of(&world, b);
    let events_before = world.events().len();
    let interners_before = value_of(&world)["interners"].clone();
    assert_eq!(
        perform_action(&mut world, &use_action(b, fixture.ability, a)),
        Ok(None)
    );
    assert_eq!(
        order_of(&world, b),
        Some(StandingOrder {
            ability: fixture.ability,
            target: a,
            repeat: false,
        })
    );
    assert_eq!(
        timeline_of(&world),
        key_before,
        "no key change on order store"
    );
    assert_eq!(
        pool_of(&world, b),
        pool_before,
        "no pool change on order store"
    );
    assert_eq!(
        world.events().len(),
        events_before,
        "no event on order store"
    );
    assert!(
        !world.rng_mut().has_stream(COMBAT_ROLL_STREAM),
        "no RNG stream on order store"
    );
    assert_eq!(
        value_of(&world)["interners"],
        interners_before,
        "no interner entry"
    );
}

// ---------------------------------------------------------------------------
// B20 — Stop validation ladder and no-op success
// ---------------------------------------------------------------------------

#[test]
fn b20_stop_validation_and_noop() {
    // NoEncounter first.
    let mut empty = World::new(1);
    let stray = empty.spawn(EntityMeta {});
    assert_eq!(
        perform_action(&mut empty, &CombatAction::Stop { actor: stray }),
        Err(CombatError::NoEncounter)
    );

    // TB: WrongMode { mode: TurnBased }.
    let fixture = rt::capable(4);
    let (mut tb, ids) = fixture.start_mode(7, EncounterMode::TurnBased).expect("tb");
    let b = ids[1];
    assert_eq!(
        perform_action(&mut tb, &CombatAction::Stop { actor: b }),
        Err(CombatError::WrongMode {
            mode: EncounterMode::TurnBased,
        })
    );

    // RT: nonparticipant, then success clears the order (no-op if none).
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    let outsider = world.spawn(EntityMeta {});
    assert_eq!(
        perform_action(&mut world, &CombatAction::Stop { actor: outsider }),
        Err(CombatError::NotParticipant { entity: outsider })
    );
    assert_eq!(
        perform_action(&mut world, &use_action(b, fixture.ability, a)),
        Ok(None)
    );
    assert_eq!(
        perform_action(&mut world, &CombatAction::Stop { actor: b }),
        Ok(None)
    );
    assert_eq!(order_of(&world, b), None, "Stop clears the order");
    assert_eq!(
        perform_action(&mut world, &CombatAction::Stop { actor: b }),
        Ok(None),
        "Stop without an order is an accepted no-op"
    );
}

// ---------------------------------------------------------------------------
// B21 — EndTurn is WrongMode in RT; TB never holds an order
// ---------------------------------------------------------------------------

#[test]
fn b21_endturn_wrong_mode_in_rt_and_tb_never_holds_an_order() {
    let fixture = rt::capable(4);
    // RT: EndTurn fails WrongMode { mode: RealTime }.
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let b = ids[1];
    assert_eq!(
        perform_action(&mut world, &CombatAction::EndTurn { actor: b }),
        Err(CombatError::WrongMode {
            mode: EncounterMode::RealTime,
        })
    );
    // TB: UseAbility and EndTurn behave as today, and no combatant holds an order.
    let (mut tb, ids) = fixture.start_mode(7, EncounterMode::TurnBased).expect("tb");
    for id in &ids {
        assert_eq!(
            order_of(&tb, *id),
            None,
            "a TB combatant never holds an order"
        );
    }
    assert_eq!(
        perform_action(&mut tb, &CombatAction::EndTurn { actor: ids[1] }),
        Ok(None)
    );
    for id in &ids {
        assert_eq!(
            order_of(&tb, *id),
            None,
            "orders never appear after a TB action"
        );
    }
}

// ---------------------------------------------------------------------------
// B22 — RT legal_actions lists UseAbility then Stop, never EndTurn
// ---------------------------------------------------------------------------

#[test]
fn b22_rt_legal_actions_lists_use_ability_then_stop() {
    let fixture = rt::capable(4);
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    let before = state_hash(&world);
    let options = legal_actions(&world, b).expect("B may act in RT");
    assert_eq!(
        options,
        vec![
            use_action(b, fixture.ability, a),
            CombatAction::Stop { actor: b },
        ]
    );
    assert!(
        !options
            .iter()
            .any(|o| matches!(o, CombatAction::EndTurn { .. })),
        "RT enumeration never lists EndTurn"
    );
    assert_eq!(state_hash(&world), before, "the query mutates nothing");

    // A non-participant fails Stop first: Actor(error).
    let outsider = world.spawn(EntityMeta {});
    assert_eq!(
        legal_actions(&world, outsider),
        Err(LegalActionsError::Actor(CombatError::NotParticipant {
            entity: outsider
        }))
    );

    // TB enumeration is unchanged and never lists Stop.
    let (tb_world, tb_ids) = fixture.start_mode(7, EncounterMode::TurnBased).expect("tb");
    let tb_b = tb_ids[1];
    let tb_options = legal_actions(&tb_world, tb_b).expect("B may act in TB");
    assert_eq!(
        tb_options,
        vec![
            use_action(tb_b, fixture.ability, tb_ids[0]),
            CombatAction::EndTurn { actor: tb_b },
        ]
    );
    assert!(
        !tb_options
            .iter()
            .any(|o| matches!(o, CombatAction::Stop { .. })),
        "TB enumeration never lists Stop"
    );
}

// ---------------------------------------------------------------------------
// B23, B24 — request_mode matrix; a switch applies only at a boundary
// ---------------------------------------------------------------------------

#[test]
fn b23_b24_request_mode_matrix_and_boundary_only() {
    // NoEncounter.
    let mut empty = World::new(1);
    assert_eq!(
        request_mode(&mut empty, EncounterMode::RealTime),
        Err(CombatError::NoEncounter)
    );

    // TB-only: RealTime is ModeUnsupported; TurnBased is Ok.
    let (mut tb_only, _) = Fixture::standard().start(7);
    assert_eq!(
        request_mode(&mut tb_only, EncounterMode::RealTime),
        Err(CombatError::ModeUnsupported)
    );
    assert_eq!(request_mode(&mut tb_only, EncounterMode::TurnBased), Ok(()));
    assert_eq!(pending(&tb_only), None);

    // RT encounter: request other mode sets pending; same mode clears it.
    let fixture = rt::capable(4);
    let (mut world, _) = fixture.start_rt(7, EncounterMode::RealTime);
    assert_eq!(request_mode(&mut world, EncounterMode::TurnBased), Ok(()));
    assert_eq!(pending(&world), Some(EncounterMode::TurnBased));
    assert_eq!(request_mode(&mut world, EncounterMode::RealTime), Ok(()));
    assert_eq!(
        pending(&world),
        None,
        "requesting the current mode clears pending"
    );
    assert_eq!(request_mode(&mut world, EncounterMode::TurnBased), Ok(()));
    assert_eq!(request_mode(&mut world, EncounterMode::TurnBased), Ok(()));
    assert_eq!(pending(&world), Some(EncounterMode::TurnBased));

    // B24: the pending switch never applies mid-round.
    for _ in 0..3 {
        tick(&mut world);
    }
    assert_eq!(mode(&world), EncounterMode::RealTime, "no mid-round switch");
    assert_eq!(pending(&world), Some(EncounterMode::TurnBased));
    // The fourth tick reaches the boundary: the switch applies.
    tick(&mut world);
    assert_eq!(mode(&world), EncounterMode::TurnBased);
    assert_eq!(pending(&world), None);
}

// ---------------------------------------------------------------------------
// B25 — RT→TB at a boundary clears orders and restores initiative
// ---------------------------------------------------------------------------

#[test]
fn b25_rt_to_tb_switch_clears_orders_and_restores_initiative() {
    let mut fixture = rt::capable(4);
    rt::edit_ability(&mut fixture, |a| a.repeats_in_real_time = Some(true));
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("A's order");
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B's order");
    request_mode(&mut world, EncounterMode::TurnBased).expect("request");
    for _ in 0..4 {
        tick(&mut world); // boundary at clock 4
    }
    assert_eq!(mode(&world), EncounterMode::TurnBased);
    assert_eq!(round(&world), 1);
    assert_eq!(
        active(&world),
        Some(b),
        "B (initiative 9) heads the TB fight"
    );
    assert_eq!(
        timeline_of(&world),
        vec![(9, b), (10, a)],
        "initiative restored"
    );
    assert_eq!(order_of(&world, a), None, "all orders cleared");
    assert_eq!(order_of(&world, b), None, "all orders cleared");
}

// ---------------------------------------------------------------------------
// B26 — TB→RT at a TB rollover keys everyone at the clock
// ---------------------------------------------------------------------------

#[test]
fn b26_tb_to_rt_switch_keys_everyone_at_the_clock() {
    let fixture = rt::capable(4);
    let (mut world, ids) = fixture.start_mode(7, EncounterMode::TurnBased).expect("tb");
    request_mode(&mut world, EncounterMode::RealTime).expect("request");
    // Play a full round: B acts then A acts, emptying the timeline.
    perform_action(&mut world, &use_action(ids[1], fixture.ability, ids[0])).expect("B acts");
    perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1])).expect("A acts");
    assert_eq!(
        mode(&world),
        EncounterMode::RealTime,
        "the rollover applies TB→RT"
    );
    assert_eq!(active(&world), None);
    assert_eq!(
        elapsed(&world),
        4,
        "the rollover advanced the clock by round_ticks"
    );
    assert_eq!(round(&world), 1, "rt_round_start ran");
    assert_eq!(
        timeline_of(&world),
        vec![(4, ids[0]), (4, ids[1])],
        "everyone keyed at the new clock"
    );
}

// ---------------------------------------------------------------------------
// B27 — OnRoundStart pools refresh once per round
// ---------------------------------------------------------------------------

#[test]
fn b27_on_round_start_pools_refresh_once_per_round() {
    let mut fixture = rt::capable(4);
    fixture.ruleset.pools[0].refresh = crpg_data::RefreshWire::OnRoundStart;
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(pool_of(&world, b), 1);
    // Spend in round 0, then confirm no mid-round refill, then reach the boundary.
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("order");
    tick(&mut world); // B executes, pool 1->0 (clock 1)
    assert_eq!(pool_of(&world, b), 0);
    tick(&mut world); // clock 2: no refill mid-round
    assert_eq!(
        pool_of(&world, b),
        0,
        "OnRoundStart does not refill mid-round"
    );
    tick(&mut world); // clock 3
    tick(&mut world); // clock 4: the boundary
    assert_eq!(pool_of(&world, b), 1, "OnRoundStart refills once per round");
}

// ---------------------------------------------------------------------------
// B28 — the sim never chooses a mode
// ---------------------------------------------------------------------------

#[test]
fn b28_start_encounter_publishes_turn_based() {
    let fixture = rt::capable(4);
    let (world, ids) = fixture.start(7);
    assert_eq!(mode(&world), EncounterMode::TurnBased);
    assert_eq!(active(&world), Some(ids[1]), "B heads the TB fight");

    let (rt_world, rt_ids) = fixture.start_rt(7, EncounterMode::RealTime);
    assert_eq!(mode(&rt_world), EncounterMode::RealTime);
    assert_eq!(active(&rt_world), None);
    assert_eq!(
        timeline_of(&rt_world),
        vec![(0, rt_ids[0]), (0, rt_ids[1])],
        "every key is 0 at an RT start"
    );
}

// ---------------------------------------------------------------------------
// B29, B32 — non-participants are untouched; no AI invents actions
// ---------------------------------------------------------------------------

#[test]
fn b29_b32_nonparticipants_untouched_and_no_ai_invents_actions() {
    for mode in [EncounterMode::TurnBased, EncounterMode::RealTime] {
        let fixture = rt::capable(4);
        let (mut world, ids) = fixture.start_mode(7, mode).expect("starts");
        let outsider = world.spawn(EntityMeta {});
        world
            .transforms_mut()
            .insert(outsider, Transform::default());
        let transforms_before = value_of(&world)["transforms"].clone();
        let health_before: Vec<u32> = ids.iter().map(|id| health_of(&world, *id)).collect();
        for _ in 0..1000 {
            tick(&mut world);
        }
        assert!(!world.timeline().contains(outsider), "never keyed");
        assert!(!world.combatants().contains(outsider), "never a combatant");
        assert_eq!(
            value_of(&world)["transforms"],
            transforms_before,
            "no stage touches the outsider"
        );
        let health_after: Vec<u32> = ids.iter().map(|id| health_of(&world, *id)).collect();
        assert_eq!(
            health_before, health_after,
            "ticks with no orders never invent an action"
        );
    }
}

// ---------------------------------------------------------------------------
// B31 — spawning never joins the encounter
// ---------------------------------------------------------------------------

#[test]
fn b31_spawning_never_joins_the_encounter() {
    let fixture = rt::capable(4);
    let (mut world, _) = fixture.start_rt(7, EncounterMode::RealTime);
    let count = world.combatants().len();
    world.spawn(EntityMeta {});
    tick(&mut world);
    assert_eq!(world.combatants().len(), count, "no join operation exists");
}

// ---------------------------------------------------------------------------
// B33 — load coherence rejects incoherent RT saves (L1–L5)
// ---------------------------------------------------------------------------

#[test]
fn b33_load_coherence_rejects_incoherent_rt_saves() {
    // Base RT save: legacy ability, B holding a one-shot order.
    let fixture = rt::capable(4);
    let (mut rt_world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, _b) = (ids[0], ids[1]);
    perform_action(&mut rt_world, &use_action(ids[1], fixture.ability, a)).expect("order");
    let rt_save = value_of(&rt_world);

    // Base RT-capable TB save carrying RT fields (non-legacy shape).
    let mut nlf = rt::capable(4);
    rt::edit_ability(&mut nlf, |x| {
        x.recovery_ticks = Some(2);
        x.repeats_in_real_time = Some(true);
    });
    let (nl_world, _) = nlf.start_mode(7, EncounterMode::TurnBased).expect("tb");
    let nl_save = value_of(&nl_world);

    // Base TB-only save (no clock, no mode, no RT fields).
    let (tb_only_world, _) = Fixture::standard().start(7);
    let tb_save = value_of(&tb_only_world);

    // L1: clock is None.
    rejects_with(&rt_save, "combat real-time mode without a clock", |s| {
        s["combat"]["clock"] = serde_json::Value::Null;
    });
    rejects_with(&tb_save, "combat pending mode without a clock", |s| {
        s["combat"]["pending_mode"] = serde_json::json!("turn_based");
    });
    rejects_with(
        &nl_save,
        "combat ability carries real-time fields without a clock",
        |s| {
            s["combat"]["clock"] = serde_json::Value::Null;
        },
    );

    // L2: clock is Some.
    rejects_with(&rt_save, "combat clock declares zero round ticks", |s| {
        s["combat"]["clock"]["round_ticks"] = serde_json::json!(0);
    });
    rejects_with(&rt_save, "combat clock exceeds its bound", |s| {
        s["combat"]["clock"]["elapsed"] = serde_json::json!(RT_CLOCK_MAX + 1);
    });
    rejects_with(
        &nl_save,
        "combat ability declares zero recovery ticks",
        |s| {
            s["combat"]["definition"]["abilities"][0]["recovery_ticks"] = serde_json::json!(0);
        },
    );

    // L3: pending_mode equals mode.
    rejects_with(&rt_save, "combat pending mode equals its mode", |s| {
        s["combat"]["pending_mode"] = s["combat"]["mode"].clone();
    });

    // L4: RT coherence. `a` is the timeline head, so the pre-existing
    // active-head coherence check passes and only L4's RT rule fires.
    rejects_with(&rt_save, "real-time combat holds an active turn", |s| {
        s["combat"]["active"] = serde_json::to_value(a).expect("an id");
    });
    rejects_with(
        &rt_save,
        "real-time timeline disagrees with its live combatants",
        |s| {
            s["timeline"].as_array_mut().expect("timeline").remove(0);
        },
    );
    rejects_with(&rt_save, "real-time timeline key is negative", |s| {
        s["timeline"][0][0] = serde_json::json!(-1);
    });

    // L5: orders.
    rejects_with(
        &tb_save,
        "turn-based combatant holds a standing order",
        |s| {
            let order = serde_json::json!({
                "ability": support::uid(103),
                "target": serde_json::to_value(ids[0]).expect("an id"),
                "repeat": false,
            });
            s["combatants"][0][1]["order"] = order;
        },
    );
    rejects_with(&rt_save, "dead combatant holds a standing order", |s| {
        s["combatants"][1][1]["dead"] = serde_json::json!(true);
        s["combatants"][1][1]["health"] = serde_json::json!(0);
        // The dead combatant leaves the timeline so L4 does not fire first.
        s["timeline"].as_array_mut().expect("timeline").remove(1);
    });
    rejects_with(&rt_save, "standing order names an unknown ability", |s| {
        s["combatants"][1][1]["order"]["ability"] = serde_json::json!(support::uid(999));
    });
    rejects_with(
        &rt_save,
        "standing order repeat disagrees with its ability",
        |s| {
            s["combatants"][1][1]["order"]["repeat"] = serde_json::json!(true);
        },
    );
}

// ---------------------------------------------------------------------------
// B34 — RT despawn removes the entry and the order without advancing turns
// ---------------------------------------------------------------------------

#[test]
fn b34_rt_despawn_removes_entry_and_order_without_advancing_turns() {
    let fixture = rt::capable_trio(4);
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("A's order");
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B's order");
    assert!(world.despawn(a));
    assert!(!world.timeline().contains(a), "A's entry is gone");
    assert!(!world.combatants().contains(a), "A's combatant is gone");
    assert_eq!(active(&world), None, "RT has no active turn to advance");
    // B's order targeting A survives until it is due, then fails.
    assert_eq!(
        order_of(&world, b),
        Some(StandingOrder {
            ability: fixture.ability,
            target: a,
            repeat: false,
        })
    );
    tick(&mut world); // B and C due; B's target is absent -> cleared, no spend.
    assert_eq!(
        order_of(&world, b),
        None,
        "the absent-target order clears at due time"
    );
    assert_eq!(pool_of(&world, b), 1, "no spend when the target is gone");
    assert!(world.timeline().contains(c), "C survives untouched");
}

// ---------------------------------------------------------------------------
// B35 — two runs of one RT script give equal per-tick hashes
// ---------------------------------------------------------------------------

#[test]
fn b35_two_runs_of_one_rt_script_match_per_tick() {
    fn script(seed: u64) -> Vec<[u8; 32]> {
        let fixture = rt::capable(4);
        let (mut world, ids) = fixture.start_rt(seed, EncounterMode::RealTime);
        let (a, b) = (ids[0], ids[1]);
        let mut hashes = Vec::new();
        for step in 0..1000u64 {
            if step % 7 == 0 {
                // Re-arm both combatants with fresh orders between ticks.
                let _ = perform_action(&mut world, &use_action(a, fixture.ability, b));
                let _ = perform_action(&mut world, &use_action(b, fixture.ability, a));
            }
            tick(&mut world);
            hashes.push(state_hash(&world));
        }
        hashes
    }
    assert_eq!(script(42), script(42));
}
