//! T149b: ability range in `crpg-sim` (Behaviour B-S1..B-S6; tests R-1..R-9).
//!
//! Every expected value is the contract's literal. A rejection is always
//! checked through `validate_action` and through `perform_action` on a clone
//! whose bytes stay unchanged.

#[allow(dead_code)]
mod support;

// The MOVE fixture is shared with the movement suites, one source.
#[path = "support/movement_schedule.rs"]
mod movement_schedule;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_sim::{
    legal_actions, perform_action, start_encounter, validate_action, within_range, CombatAction,
    CombatError, EncounterMode, HistoryError, HistoryEvent, HistoryWorld, Measure, StandingOrder,
    World,
};
use movement_schedule::{move_fixture, move_rt_fixture, p, rp};
use serde_json::{json, Value};
use support::creature::with_json;
use support::rt::{bytes_of, elapsed, json_keys, pool_of, timeline_of, use_action, value_of};
use support::surface::{
    act, add_derived, always_success, attach_to_ability, binary, effect_doc, end, modifier, stat,
    stat_target,
};
use support::Fixture;

type Point = [Fx16_16; 3];

const M: i32 = 65536;

fn raw(value: i32) -> Fx16_16 {
    Fx16_16::from_raw(value)
}

fn ability() -> Ulid {
    Ulid::from_u128(103)
}

fn use_ability(actor: EntityId, target: EntityId) -> CombatAction {
    use_action(actor, ability(), target)
}

/// MOVE (or MOVE_RT) with the standard ability's range set to raw `range`.
fn with_range(mut fixture: Fixture, range: i32) -> Fixture {
    support::rt::edit_ability(&mut fixture, |ability| ability.range = Some(raw(range)));
    fixture
}

fn ranged(range: i32) -> Fixture {
    with_range(move_fixture(), range)
}

fn ranged_rt(range: i32) -> Fixture {
    with_range(move_rt_fixture(), range)
}

/// A turn-based world of `fixture` and `[A, B]`; B holds the first turn.
fn tb(fixture: &Fixture) -> (World, EntityId, EntityId) {
    let mut world = World::new(7);
    fixture
        .with_spec(|spec| start_encounter(&mut world, spec))
        .expect("the turn-based encounter starts");
    let ids: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
    (world, ids[0], ids[1])
}

/// A real-time world of `fixture` and `[A, B]`.
fn rt(fixture: &Fixture) -> (World, EntityId, EntityId) {
    let (world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    (world, ids[0], ids[1])
}

fn place(world: &mut World, entity: EntityId, to: Point) {
    world.place(entity, to).expect("the entity is placed");
}

/// Asserts both entry points reject `action` with `expected` and change nothing.
fn rejected(world: &World, action: CombatAction, expected: CombatError) {
    assert_eq!(validate_action(world, &action), Err(expected.clone()));
    let mut probe = world.clone();
    assert_eq!(perform_action(&mut probe, &action), Err(expected));
    assert_eq!(
        bytes_of(&probe),
        bytes_of(world),
        "a rejection changes nothing"
    );
}

fn accepted(world: &World, action: CombatAction) {
    assert_eq!(validate_action(world, &action), Ok(()));
}

fn out_of_range(world: &World, action: CombatAction) {
    rejected(world, action, CombatError::OutOfRange);
}

fn entity_json(entity: EntityId) -> Value {
    serde_json::to_value(entity).expect("an id serializes")
}

// ---------------------------------------------------------------------------
// R-1
// ---------------------------------------------------------------------------

#[test]
fn r1_within_range_formula() {
    let e = Measure::Euclidean;
    let w = |range: i32, from: Point, to: Point| within_range(e, raw(range), from, to);

    // 10 m along x.
    assert!(w(655360, p(0, 0), p(10, 0)));
    assert!(!w(655359, p(0, 0), p(10, 0)));
    // The 3-4-5 triangle.
    assert!(w(327680, p(0, 0), p(3, 4)));
    assert!(!w(327679, p(0, 0), p(3, 4)));
    // The root rounds up: sqrt(2) raw is 1.41, so 2.
    assert!(w(2, rp(0, 0, 0), rp(1, 0, 1)));
    assert!(!w(1, rp(0, 0, 0), rp(1, 0, 1)));
    // One metre by one metre: 65536 * sqrt(2) = 92681.9.
    assert!(w(92682, p(0, 0), rp(M, 0, M)));
    assert!(!w(92681, p(0, 0), rp(M, 0, M)));
    // Five metres straight up: y never counts; a negative range is never met.
    assert!(w(0, rp(0, 0, 0), rp(0, 327680, 0)));
    assert!(!w(-1, rp(0, 0, 0), rp(0, 327680, 0)));
    // The largest measurable distance, corner to corner of the box.
    let lo = rp(-(1 << 29), 0, -(1 << 29));
    let hi = rp(1 << 29, 0, 1 << 29);
    assert!(w(1518500250, lo, hi));
    assert!(!w(1518500249, lo, hi));
    assert!(w(i32::MAX, lo, hi));
    // A point outside the coordinate box has no measure.
    assert!(!w(i32::MAX, p(0, 0), rp((1 << 29) + 1, 0, 0)));
    assert!(!w(i32::MAX, rp((1 << 29) + 1, 0, 0), p(0, 0)));
    // A point to itself measures 0; i32::MIN is never met.
    assert!(w(0, p(2, 2), p(2, 2)));
    assert!(!w(i32::MIN, p(2, 2), p(2, 2)));
}

// ---------------------------------------------------------------------------
// R-2
// ---------------------------------------------------------------------------

#[test]
fn r2_admission_at_the_boundary() {
    let (world, a, b) = tb(&ranged(655360));
    assert_eq!(world.combat().expect("encounter").active, Some(b));
    accepted(&world, use_ability(b, a));

    let (world, a, b) = tb(&ranged(655359));
    out_of_range(&world, use_ability(b, a));

    // dx 7 m, dz 4 m.
    let (mut world, a, b) = tb(&ranged(528369));
    place(&mut world, a, p(3, 4));
    accepted(&world, use_ability(b, a));
    let (mut world, a, b) = tb(&ranged(528368));
    place(&mut world, a, p(3, 4));
    out_of_range(&world, use_ability(b, a));

    // Height is ignored.
    let (mut world, a, b) = tb(&ranged(655360));
    place(&mut world, a, rp(0, 327680, 0));
    accepted(&world, use_ability(b, a));

    // Coincident targets: range 0 admits, range -1 admits nothing.
    let (mut world, a, b) = tb(&ranged(0));
    place(&mut world, a, p(10, 0));
    accepted(&world, use_ability(b, a));
    let (mut world, a, b) = tb(&ranged(-1));
    place(&mut world, a, p(10, 0));
    out_of_range(&world, use_ability(b, a));

    // No range: unlimited, even for a target at the edge of the box.
    let (mut world, a, b) = tb(&move_fixture());
    place(&mut world, a, rp(1 << 29, 0, 0));
    accepted(&world, use_ability(b, a));
}

// ---------------------------------------------------------------------------
// R-3
// ---------------------------------------------------------------------------

#[test]
fn r3_missing_mover() {
    // No movement in the ruleset, so no movers.
    let standard = with_range(Fixture::standard(), i32::MAX);
    let (world, ids) = support::surface::start(&standard, 7);
    let (a, b) = (ids[0], ids[1]);
    assert!(world.movers().get(a).is_none() && world.movers().get(b).is_none());
    out_of_range(&world, use_ability(b, a));
    let (world, ids) = support::surface::start(&Fixture::standard(), 7);
    accepted(&world, use_ability(ids[1], ids[0]));

    // A mover removed from a save: either side.
    let (world, a, b) = tb(&ranged(655360));
    accepted(&world, use_ability(b, a));
    for gone in [a, b] {
        let key = entity_json(gone);
        let stripped = with_json(&world, |save| {
            save["movers"]
                .as_array_mut()
                .expect("movers")
                .retain(|entry| entry[0] != key);
        });
        assert!(!stripped.movers().contains(gone));
        out_of_range(&stripped, use_ability(b, a));
    }
}

// ---------------------------------------------------------------------------
// R-4
// ---------------------------------------------------------------------------

#[test]
fn r4_range_stage_is_last() {
    // OutOfTurn.
    let (world, a, b) = tb(&ranged(1));
    rejected(
        &world,
        use_ability(a, b),
        CombatError::OutOfTurn {
            actor: a,
            active: Some(b),
        },
    );

    // SelfTarget, then OutOfRange once self-targets are allowed.
    let (world, _, b) = tb(&ranged(-1));
    rejected(&world, use_ability(b, b), CombatError::SelfTarget);
    let mut fixture = ranged(-1);
    support::rt::edit_ability(&mut fixture, |ability| ability.allow_self_target = true);
    let (world, _, b) = tb(&fixture);
    out_of_range(&world, use_ability(b, b));

    // InsufficientAction wins over a failing range.
    let mut fixture = ranged(655360);
    support::rt::edit_ability(&mut fixture, |ability| ability.ends_turn = false);
    let (mut world, a, b) = tb(&fixture);
    perform_action(&mut world, &use_ability(b, a)).expect("the first use succeeds");
    place(&mut world, a, p(-1, 0));
    rejected(
        &world,
        use_ability(b, a),
        CombatError::InsufficientAction {
            pool: Ulid::from_u128(105),
            cost: 1,
            current: 0,
        },
    );

    // DeadTarget wins over a failing range.
    let mut fixture = ranged(655360);
    always_success(&mut fixture);
    support::rt::edit_ability(&mut fixture, |ability| {
        ability.damage[0].amount = 99;
        ability.ends_turn = false;
    });
    let (mut world, a, b) = tb(&fixture);
    perform_action(&mut world, &use_ability(b, a)).expect("B kills A");
    place(&mut world, a, p(-1, 0));
    rejected(
        &world,
        use_ability(b, a),
        CombatError::DeadTarget { target: a },
    );
}

/// The derived stage (T137 B15) runs before the range stage.
#[test]
fn r4_derived_stage_precedes_the_range_stage() {
    let mut fixture = Fixture::standard();
    add_derived(
        &mut fixture,
        "ratio",
        binary("divide", stat("alpha"), stat("beta")),
    );
    always_success(&mut fixture);
    support::rt::edit_ability(&mut fixture, |ability| {
        ability.defense = crpg_data::DefenseWire::TargetStat {
            stat: "ratio".to_owned(),
        }
    });
    attach_to_ability(
        &mut fixture,
        effect_doc(
            141,
            "target",
            "probe-type",
            "stack_all",
            vec![modifier(142, stat_target("beta"), "set", 0)],
            3,
        ),
    );
    support::rt::edit_ability(&mut fixture, |ability| {
        ability.allow_self_target = true;
        ability.ends_turn = false;
    });
    let ability = fixture.ability;
    let (mut world, ids) = support::surface::start(&fixture, 17);
    let (a, b) = (ids[0], ids[1]);
    act(&mut world, b, ability, a);
    end(&mut world, b);
    end(&mut world, a);
    let stat_error = CombatError::StatEvaluation {
        stat: "ratio".to_owned(),
    };
    assert_eq!(
        validate_action(&world, &use_action(b, ability, a)),
        Err(stat_error.clone())
    );

    // Give the ability a range through the save; the definition is already
    // non-legacy (it holds an effect). The standard fixture has no movers.
    let ranged_save = with_json(&world, |save| {
        let entry = &mut save["combat"]["definition"]["abilities"][0];
        entry["range"] = json!(65536);
    });
    assert_eq!(
        ranged_save
            .combat()
            .expect("encounter")
            .definition
            .abilities[0]
            .range,
        Some(raw(65536))
    );
    rejected(&ranged_save, use_action(b, ability, a), stat_error);
    out_of_range(&ranged_save, use_action(b, ability, b));
}

// ---------------------------------------------------------------------------
// R-5
// ---------------------------------------------------------------------------

#[test]
fn r5_legal_actions_omit() {
    let end_turn = |actor| CombatAction::EndTurn { actor };

    let (world, a, b) = tb(&ranged(655360));
    assert_eq!(
        legal_actions(&world, b),
        Ok(vec![use_ability(b, a), end_turn(b)])
    );
    let (world, _, b) = tb(&ranged(655359));
    assert_eq!(legal_actions(&world, b), Ok(vec![end_turn(b)]));

    let mut fixture = ranged(655359);
    support::rt::edit_ability(&mut fixture, |ability| ability.allow_self_target = true);
    let (world, _, b) = tb(&fixture);
    assert_eq!(
        legal_actions(&world, b),
        Ok(vec![use_ability(b, b), end_turn(b)])
    );

    let (world, a, b) = tb(&move_fixture());
    assert_eq!(
        legal_actions(&world, b),
        Ok(vec![use_ability(b, a), end_turn(b)])
    );

    // Real time: the closing action is Stop.
    let (mut world, a, b) = rt(&ranged_rt(655360));
    assert_eq!(
        legal_actions(&world, a),
        Ok(vec![use_ability(a, b), CombatAction::Stop { actor: a }])
    );
    place(&mut world, b, rp(655361, 0, 0));
    assert_eq!(
        legal_actions(&world, a),
        Ok(vec![CombatAction::Stop { actor: a }])
    );
}

// ---------------------------------------------------------------------------
// R-6
// ---------------------------------------------------------------------------

#[test]
fn r6_rejections_change_nothing() {
    // Turn-based.
    let fixture = ranged(655359);
    let (mut world, a, b) = tb(&fixture);
    let before = bytes_of(&world);
    assert_eq!(
        perform_action(&mut world, &use_ability(b, a)),
        Err(CombatError::OutOfRange)
    );
    assert_eq!(bytes_of(&world), before);

    // Real time.
    let fixture_rt = ranged_rt(655359);
    let (mut world, a, b) = rt(&fixture_rt);
    let before = bytes_of(&world);
    assert_eq!(
        perform_action(&mut world, &use_ability(a, b)),
        Err(CombatError::OutOfRange)
    );
    assert!(world.combatants().get(a).expect("A").order().is_none());
    assert_eq!(bytes_of(&world), before);

    // The history wrapper, in both modes.
    for (fixture, mode) in [
        (&fixture, EncounterMode::TurnBased),
        (&fixture_rt, EncounterMode::RealTime),
    ] {
        let (mut history, ids) = fixture.start_history(7, mode).expect("history starts");
        let (a, b) = (ids[0], ids[1]);
        let action = if mode == EncounterMode::TurnBased {
            use_ability(b, a)
        } else {
            use_ability(a, b)
        };
        let (sequence, pending) = (history.last_sequence(), history.pending_len());
        let before = bytes_of(history.world());
        let error = history.perform_action(&action).expect_err("out of range");
        assert!(matches!(
            error,
            HistoryError::Combat(CombatError::OutOfRange)
        ));
        assert_eq!(error.to_string(), "OutOfRange at combat/target");
        assert_eq!(history.last_sequence(), sequence);
        assert_eq!(history.pending_len(), pending);
        assert_eq!(bytes_of(history.world()), before);
    }
    assert_eq!(
        CombatError::OutOfRange.to_string(),
        "OutOfRange at combat/target"
    );
}

// ---------------------------------------------------------------------------
// R-7
// ---------------------------------------------------------------------------

/// The events of `history` after sequence `after`.
fn journal_after(history: &HistoryWorld, after: u64) -> Vec<HistoryEvent> {
    history
        .read_after(after, 256)
        .expect("the journal reads")
        .into_iter()
        .map(|envelope| envelope.payload)
        .collect()
}

fn resolved(events: &[HistoryEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, HistoryEvent::ActionResolved { .. }))
        .count()
}

fn ended(events: &[HistoryEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, HistoryEvent::OrderEnded { .. }))
        .count()
}

#[test]
fn r7_out_of_range_order_stays_due() {
    let fixture = ranged_rt(655360);
    let (mut history, ids) = fixture
        .start_history(7, EncounterMode::RealTime)
        .expect("history starts");
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(timeline_of(history.world()), vec![(0, a), (0, b)]);
    history
        .perform_action(&use_ability(a, b))
        .expect("A's order is accepted");
    let order = StandingOrder {
        ability: ability(),
        target: b,
        repeat: false,
    };
    assert_eq!(
        history.world().combatants().get(a).expect("A").order(),
        Some(order)
    );
    let after_order = history.last_sequence();

    // One raw unit past the range.
    history.place(b, rp(655361, 0, 0)).expect("B is placed");
    for _ in 0..2 {
        history.tick().expect("the tick commits");
    }
    let world = history.world();
    assert_eq!(elapsed(world), 2);
    assert_eq!(world.combatants().get(a).expect("A").order(), Some(order));
    assert_eq!(timeline_of(world), vec![(0, a), (0, b)]);
    assert_eq!(pool_of(world, a), 1);
    let events = journal_after(&history, after_order);
    assert_eq!(ended(&events), 0, "{events:?}");
    assert_eq!(resolved(&events), 0, "{events:?}");

    // Back in range: the order executes at once, once.
    history.place(b, p(10, 0)).expect("B is placed");
    let before_third = history.last_sequence();
    history.tick().expect("the tick commits");
    let world = history.world();
    assert_eq!(elapsed(world), 3);
    let events = journal_after(&history, before_third);
    assert_eq!(ended(&events), 0, "{events:?}");
    assert_eq!(resolved(&events), 1, "{events:?}");
    assert!(events.iter().any(|event| matches!(
        event,
        HistoryEvent::ActionResolved { actor, target, ability: used, .. }
            if *actor == a && *target == b && *used == ability()
    )));
    assert!(world.combatants().get(a).expect("A").order().is_none());
    assert_eq!(pool_of(world, a), 0);
    assert_eq!(timeline_of(world), vec![(0, b), (7, a)]);
}

/// A retained order is tried again on every later tick, with nothing spent.
#[test]
fn r7_a_retained_order_survives_many_ticks() {
    let (mut world, a, b) = rt(&ranged_rt(655360));
    perform_action(&mut world, &use_ability(a, b)).expect("accepted");
    place(&mut world, b, rp(655361, 0, 0));
    let order = world.combatants().get(a).expect("A").order();
    assert!(order.is_some());
    for tick in 1..=9 {
        crpg_sim::tick(&mut world);
        assert_eq!(elapsed(&world), tick);
        assert_eq!(world.combatants().get(a).expect("A").order(), order);
        assert_eq!(pool_of(&world, a), 1);
        assert_eq!(timeline_of(&world), vec![(0, a), (0, b)]);
    }
}

// ---------------------------------------------------------------------------
// R-8
// ---------------------------------------------------------------------------

#[test]
fn r8_other_failures_still_end() {
    let mut fixture = ranged_rt(655360);
    support::rt::edit_ability(&mut fixture, |ability| {
        ability.repeats_in_real_time = Some(true);
        ability.recovery_ticks = Some(1);
    });
    let (mut history, ids) = fixture
        .start_history(7, EncounterMode::RealTime)
        .expect("history starts");
    let (a, b) = (ids[0], ids[1]);
    history
        .perform_action(&use_ability(a, b))
        .expect("A's order is accepted");
    history.tick().expect("tick 1 commits");
    let world = history.world();
    assert_eq!(pool_of(world, a), 0);
    assert_eq!(timeline_of(world), vec![(0, b), (2, a)]);
    assert!(world.combatants().get(a).expect("A").order().is_some());

    history.place(b, rp(655361, 0, 0)).expect("B is placed");
    let before = history.last_sequence();
    history.tick().expect("tick 2 commits");
    assert!(history
        .world()
        .combatants()
        .get(a)
        .expect("A")
        .order()
        .is_none());
    let events = journal_after(&history, before);
    assert_eq!(ended(&events), 1, "{events:?}");
    assert!(events
        .iter()
        .any(|event| matches!(event, HistoryEvent::OrderEnded { actor } if *actor == a)));
    assert_eq!(resolved(&events), 0, "{events:?}");
}

// ---------------------------------------------------------------------------
// R-9
// ---------------------------------------------------------------------------

#[test]
fn r9_persistence() {
    let fixture = with_range(Fixture::standard(), M);
    let (world, _) = support::surface::start(&fixture, 7);
    let value = value_of(&world);
    let definition = &value["combat"]["definition"];
    let entry = &definition["abilities"][0];
    assert_eq!(entry["range"], json!(65536), "{definition}");
    assert_eq!(
        world.combat().expect("encounter").definition.abilities[0].range,
        Some(raw(65536))
    );
    let bytes = bytes_of(&world);
    let reloaded: World = serde_json::from_slice(&bytes).expect("the save reloads");
    assert_eq!(reloaded, world);
    assert_eq!(bytes_of(&reloaded), bytes);
    assert_eq!(
        reloaded.combat().expect("encounter").definition.abilities[0].range,
        Some(raw(65536))
    );

    // No range: the 15-field legacy form and no `range` key anywhere.
    let (plain, _) = support::surface::start(&Fixture::standard(), 7);
    let value = value_of(&plain);
    let definition = value["combat"]["definition"].as_object().expect("object");
    assert_eq!(definition.len(), 15, "{definition:?}");
    assert!(!definition.contains_key("abilities"));
    assert!(!json_keys(&value).contains("range"));
    let reloaded: World = serde_json::from_slice(&bytes_of(&plain)).expect("reloads");
    assert_eq!(
        reloaded.combat().expect("encounter").definition.abilities[0].range,
        None
    );
}
