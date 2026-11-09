//! T036c: the turn-based move and history version 3 (Behaviour B-C1..B-C5,
//! B-H1..B-H4; tests C-1..C-6; the golden C-7 is `history_v3_golden.rs`).
//!
//! Every expected value is the contract's literal or is worked out by hand from
//! it: a move costs `ceil(measured / step) * step_cost` pool units, paid
//! atomically from the movement pool, and journals one `Moved`.

#[allow(dead_code)]
mod support;

// The MOVE fixture is shared with the golden schedule, one source.
#[path = "support/movement_schedule.rs"]
mod movement_schedule;

use std::error::Error as _;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_sim::{
    end_encounter, perform_action, perform_move, route_cost, start_encounter,
    start_encounter_in_mode, tick, validate_action, validate_move, CombatAction, CombatError,
    EncounterMode, EntityMeta, HistoryError, HistoryEvent, HistoryWorld, Measure, MoveError,
    MoveOutcome, TurnMovement, World, HISTORY_VERSION,
};
use movement_schedule::{bits, edit_ruleset, move_fixture, move_rt_fixture, p, pl, rev, route, rp};
use serde_json::{json, Value};
use support::creature::{try_with_json, with_json};
use support::rt::{bytes_of, make_lethal, use_action, value_of};
use support::{uid, Fixture};

type Point = [Fx16_16; 3];

/// MOVE, turn-based, the navmesh bound: `(fixture, world, [A, B, C])`; B holds
/// the turn first.
fn started(fixture: Fixture) -> (Fixture, World, [EntityId; 3]) {
    let mut world = World::new(7);
    let c = fixture.register(&mut world, pl(113)).expect("C registers");
    fixture
        .with_spec(|spec| start_encounter(&mut world, spec))
        .expect("the turn-based encounter starts");
    let ids: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
    world.bind_nav(rev());
    (fixture, world, [ids[0], ids[1], c])
}

fn tb() -> (Fixture, World, [EntityId; 3]) {
    started(move_fixture())
}

/// The balance of B's (or any combatant's) extra pool 106.
fn pool_106(world: &World, entity: EntityId) -> u32 {
    world
        .combatants()
        .get(entity)
        .expect("a combatant")
        .extra_pools()[0]
        .current()
}

fn mover_position(world: &World, entity: EntityId) -> Point {
    world.movers().get(entity).expect("a mover").position()
}

/// `(position bits, velocity bits)` of the entity's mirror.
fn mirror_of(world: &World, entity: EntityId) -> ([u32; 3], [u32; 3]) {
    let transform = world.transforms().get(entity).expect("a mirror");
    (bits(transform.position), bits(transform.velocity))
}

fn b_route(tag: u64) -> crpg_sim::MoveRoute {
    route(tag, vec![p(10, 0), p(10, 3), p(7, 3)])
}

/// MOVE1: MOVE without pool 106 and paying moves from the ability's pool 105.
fn move1_fixture() -> Fixture {
    let mut fixture = move_fixture();
    edit_ruleset(&mut fixture, |ruleset| {
        ruleset["pools"]
            .as_array_mut()
            .expect("pools")
            .retain(|pool| pool["id"] != json!(uid(106)));
        ruleset["movement"]["turn"]["pool"] = json!(uid(105));
    });
    fixture
}

/// Runs `body` on the world and asserts it left the bytes unchanged.
fn unchanged<R>(world: &mut World, body: impl FnOnce(&mut World) -> R) -> R {
    let before = bytes_of(world);
    let result = body(world);
    assert_eq!(bytes_of(world), before, "a failure changes nothing");
    result
}

/// An arbitrary entity id (historical references are never liveness-checked).
fn sample_entity() -> EntityId {
    serde_json::from_value(json!({"index": 3, "generation": 1})).expect("an id")
}

fn entity_json(entity: EntityId) -> Value {
    serde_json::to_value(entity).expect("an id serializes")
}

// ---------------------------------------------------------------------------
// C-1
// ---------------------------------------------------------------------------

#[test]
fn c1_route_cost() {
    let step = Fx16_16::from_raw;
    assert_eq!(route_cost(720896, step(98304), 1), Some(8));
    assert_eq!(route_cost(327680, step(98304), 2), Some(8));
    assert_eq!(route_cost(0, step(98304), 1), Some(0));
    assert_eq!(route_cost(1, step(65536), 3), Some(3));
    assert_eq!(
        route_cost(4294967295, step(1), 1),
        Some(4294967295),
        "the largest cost that fits"
    );
    assert_eq!(route_cost(4294967296, step(1), 1), None);
    assert_eq!(route_cost(268435456, step(1), u32::MAX), None);
    assert_eq!(route_cost(65536, step(0), 1), None, "step raw 0");
    assert_eq!(route_cost(65536, step(-1), 1), None, "step raw -1");
    assert_eq!(route_cost(65536, step(i32::MIN), 1), None);
    // Rounds up, never down: 1.5 m is one step, one raw unit more is two.
    assert_eq!(route_cost(98304, step(98304), 1), Some(1));
    assert_eq!(route_cost(98305, step(98304), 1), Some(2));
    assert_eq!(
        route_cost(u64::MAX, step(1), 2),
        None,
        "the u64 product overflows"
    );
}

// ---------------------------------------------------------------------------
// C-2
// ---------------------------------------------------------------------------

#[test]
fn c2_a_move_spends_and_keeps_the_turn() {
    let (_, mut world, [a, b, _]) = tb();
    assert_eq!(world.combat().expect("encounter").active, Some(b));
    assert_eq!(pool_106(&world, b), 6, "the movement pool holds 6");
    let before = value_of(&world);

    let outcome = perform_move(&mut world, b, &b_route(1)).expect("the move is paid");
    assert_eq!(
        outcome,
        MoveOutcome {
            actor: b,
            cost: 4,
            from: p(10, 0),
            to: p(7, 3),
        }
    );
    assert_eq!(pool_106(&world, b), 2);
    let mover = world.movers().get(b).expect("a mover");
    assert_eq!(mover.position(), p(7, 3));
    assert!(mover.route().is_none());
    assert_eq!(
        mirror_of(&world, b),
        ([0x40e0_0000, 0, 0x4040_0000], [0, 0, 0])
    );
    assert_eq!(world.combat().expect("encounter").active, Some(b));
    let after = value_of(&world);
    assert_eq!(before["rng"], after["rng"], "no RNG is drawn");
    assert_eq!(before["events"], after["events"], "no event is enqueued");
    assert_eq!(before["tick"], after["tick"], "time does not advance");
    // A's pool and mover are untouched.
    assert_eq!(pool_106(&world, a), 6);
    assert_eq!(mover_position(&world, a), p(0, 0));

    // The move repeats while the pool pays.
    let second = route(2, vec![p(7, 3), p(7, 1)]);
    let outcome = perform_move(&mut world, b, &second).expect("2 m is 2 steps");
    assert_eq!(
        (outcome.cost, outcome.from, outcome.to),
        (2, p(7, 3), p(7, 1))
    );
    assert_eq!(pool_106(&world, b), 0);
    let third = route(3, vec![p(7, 1), p(7, 0)]);
    assert_eq!(
        unchanged(&mut world, |w| perform_move(w, b, &third)),
        Err(MoveError::CannotAfford {
            cost: 1,
            current: 0
        })
    );
    assert_eq!(mover_position(&world, b), p(7, 1));
    assert_eq!(world.combat().expect("encounter").active, Some(b));
    // The move never ended the turn: B can still end it.
    perform_action(&mut world, &CombatAction::EndTurn { actor: b }).expect("the turn ends");
    assert_eq!(world.combat().expect("encounter").active, Some(a));
}

#[test]
fn c2_the_pool_boundary_is_exact() {
    // 9 m = 6 steps costs exactly the whole pool; 10 m = 7 steps does not fit.
    let (_, mut world, [_, b, _]) = tb();
    let nine = route(1, vec![p(10, 0), p(10, 9)]);
    assert_eq!(validate_move(&world, b, &nine), Ok(6));
    let ten = route(1, vec![p(10, 0), p(10, 10)]);
    assert_eq!(
        unchanged(&mut world, |w| perform_move(w, b, &ten)),
        Err(MoveError::CannotAfford {
            cost: 7,
            current: 6
        })
    );
    let outcome = perform_move(&mut world, b, &nine).expect("9 m fits");
    assert_eq!(outcome.cost, 6);
    assert_eq!(pool_106(&world, b), 0);
}

#[test]
fn c2_the_cost_is_reported_in_u64() {
    // Step raw 1 and step_cost u32::MAX: 65536 steps of 4294967295 each.
    let mut fixture = move_fixture();
    edit_ruleset(&mut fixture, |ruleset| {
        ruleset["movement"]["turn"]["step"] = json!({"type": "constant", "value": 1});
        ruleset["movement"]["turn"]["step_cost"] = json!(u32::MAX);
    });
    let (_, mut world, [_, b, _]) = started(fixture);
    let one_metre = route(1, vec![p(10, 0), p(11, 0)]);
    let expected = MoveError::CannotAfford {
        cost: 281_474_976_645_120,
        current: 6,
    };
    assert_eq!(validate_move(&world, b, &one_metre), Err(expected.clone()));
    assert_eq!(
        unchanged(&mut world, |w| perform_move(w, b, &one_metre)),
        Err(expected)
    );
}

#[test]
fn c2_a_move_rewrites_a_poisoned_mirror_velocity() {
    let (_, world, [_, b, _]) = tb();
    let mut poisoned = with_json(&world, |save| {
        let key = entity_json(b);
        let entry = save["transforms"]
            .as_array_mut()
            .expect("transforms")
            .iter_mut()
            .find(|entry| entry[0] == key)
            .expect("B's transform");
        entry[1]["velocity"] = json!([1.0, 0.0, 0.0]);
    });
    assert_eq!(mirror_of(&poisoned, b).1, [0x3f80_0000, 0, 0]);
    perform_move(&mut poisoned, b, &b_route(1)).expect("the move is paid");
    assert_eq!(mirror_of(&poisoned, b).1, [0, 0, 0]);
}

#[test]
fn c2_a_move_spends_the_primary_pool_when_it_is_the_movement_pool() {
    let (fixture, mut world, [a, b, _]) = started(move1_fixture());
    let one = route(1, vec![p(10, 0), p(11, 0)]);
    let outcome = perform_move(&mut world, b, &one).expect("1 m is 1 step");
    assert_eq!(outcome.cost, 1);
    let combatant = world.combatants().get(b).expect("B");
    assert_eq!(combatant.action_pool().current(), 0);
    assert!(combatant.extra_pools().is_empty());
    let again = route(2, vec![p(11, 0), p(12, 0)]);
    assert_eq!(
        unchanged(&mut world, |w| perform_move(w, b, &again)),
        Err(MoveError::CannotAfford {
            cost: 1,
            current: 0
        })
    );
    // The same pool pays the ability: it is empty.
    assert_eq!(
        validate_action(&world, &use_action(b, fixture.ability, a)),
        Err(CombatError::InsufficientAction {
            pool: Ulid::from_u128(105),
            cost: 1,
            current: 0
        })
    );
    // A's primary pool is untouched.
    assert_eq!(
        world
            .combatants()
            .get(a)
            .expect("A")
            .action_pool()
            .current(),
        1
    );
}

#[test]
fn c2_an_extra_pool_is_the_one_spent_when_the_primary_is_not() {
    // B's primary pool (105) is untouched by a move paid from 106.
    let (_, mut world, [_, b, _]) = tb();
    let primary_before = world
        .combatants()
        .get(b)
        .expect("B")
        .action_pool()
        .current();
    perform_move(&mut world, b, &b_route(1)).expect("paid");
    let combatant = world.combatants().get(b).expect("B");
    assert_eq!(combatant.action_pool().current(), primary_before);
    assert_eq!(combatant.extra_pools().len(), 1);
}

#[test]
fn c2_a_step_boundary_is_ceil_not_floor() {
    // 1.5 m is one step, 1.5 m plus one raw unit is two.
    let (_, world, [_, b, _]) = tb();
    let exact = route(1, vec![rp(655360, 0, 0), rp(655360, 0, 98304)]);
    assert_eq!(validate_move(&world, b, &exact), Ok(1));
    let over = route(1, vec![rp(655360, 0, 0), rp(655360, 0, 98305)]);
    assert_eq!(validate_move(&world, b, &over), Ok(2));
}

// ---------------------------------------------------------------------------
// C-3
// ---------------------------------------------------------------------------

fn combat(error: CombatError) -> Result<u32, MoveError> {
    Err(MoveError::Combat(error))
}

/// Asserts that `perform_move` and `validate_move` agree on a failure and leave
/// the world's bytes unchanged.
fn assert_fails(world: &mut World, actor: EntityId, route: &crpg_sim::MoveRoute, want: MoveError) {
    assert_eq!(validate_move(world, actor, route), Err(want.clone()));
    let result = unchanged(world, |w| perform_move(w, actor, route));
    assert_eq!(result, Err(want));
}

#[test]
fn c3_no_encounter() {
    let fixture = move_fixture();
    let mut world = World::new(7);
    let c = fixture.register(&mut world, pl(113)).expect("C");
    world.bind_nav(rev());
    let walk = route(1, vec![p(0, 10), p(1, 10)]);
    assert_fails(
        &mut world,
        c,
        &walk,
        MoveError::Combat(CombatError::NoEncounter),
    );
    assert_eq!(
        validate_move(&world, c, &walk),
        combat(CombatError::NoEncounter)
    );
}

#[test]
fn c3_not_participant_and_absent() {
    let (_, mut world, [_, _, c]) = tb();
    let walk = route(1, vec![p(0, 10), p(1, 10)]);
    assert_fails(
        &mut world,
        c,
        &walk,
        MoveError::Combat(CombatError::NotParticipant { entity: c }),
    );
    let gone = world.spawn(EntityMeta {});
    assert!(world.despawn(gone));
    assert_fails(
        &mut world,
        gone,
        &walk,
        MoveError::Combat(CombatError::AbsentActor { actor: gone }),
    );
}

#[test]
fn c3_dead_actor_then_a_terminal_encounter() {
    let mut fixture = move_fixture();
    make_lethal(&mut fixture, 99);
    let (fixture, mut world, [a, b, _]) = started(fixture);
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B kills A");
    assert!(world.combatants().get(a).expect("A").dead());
    let from_a = route(1, vec![p(0, 0), p(1, 0)]);
    assert_fails(
        &mut world,
        a,
        &from_a,
        MoveError::Combat(CombatError::DeadActor { actor: a }),
    );
    // B survives but the encounter is over: nobody holds the turn.
    assert_fails(
        &mut world,
        b,
        &route(1, vec![p(10, 0), p(11, 0)]),
        MoveError::Combat(CombatError::OutOfTurn {
            actor: b,
            active: None,
        }),
    );
}

#[test]
fn c3_wrong_mode_in_real_time() {
    let fixture = move_rt_fixture();
    let mut world = World::new(7);
    fixture.register(&mut world, pl(113)).expect("C");
    fixture
        .with_spec(|spec| start_encounter_in_mode(&mut world, spec, EncounterMode::RealTime))
        .expect("the real-time encounter starts");
    world.bind_nav(rev());
    let ids: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
    for &entity in &ids {
        let position = mover_position(&world, entity);
        let walk = route(1, vec![position, rp(position[0].to_raw() + 65536, 0, 0)]);
        assert_fails(
            &mut world,
            entity,
            &walk,
            MoveError::Combat(CombatError::WrongMode {
                mode: EncounterMode::RealTime,
            }),
        );
    }
}

#[test]
fn c3_out_of_turn() {
    let (_, mut world, [a, b, _]) = tb();
    let from_a = route(1, vec![p(0, 0), p(1, 0)]);
    assert_fails(
        &mut world,
        a,
        &from_a,
        MoveError::Combat(CombatError::OutOfTurn {
            actor: a,
            active: Some(b),
        }),
    );
}

#[test]
fn c3_not_mover() {
    let (_, world, [_, b, _]) = tb();
    // B's mover is gone from the save (B holds the turn).
    let key = entity_json(b);
    let mut stripped = with_json(&world, |save| {
        save["movers"]
            .as_array_mut()
            .expect("movers")
            .retain(|entry| entry[0] != key);
    });
    assert!(!stripped.movers().contains(b));
    assert_fails(
        &mut stripped,
        b,
        &b_route(1),
        MoveError::NotMover { entity: b },
    );

    // The encounter's definition holds no movement rules.
    let mut bare = with_json(&world, |save| {
        save["combat"]["definition"]
            .as_object_mut()
            .expect("the definition")
            .remove("movement")
            .expect("the definition holds movement");
    });
    assert_eq!(bare.combat().expect("encounter").definition.movement, None);
    assert!(bare.movers().contains(b), "B still holds its mover");
    assert_fails(&mut bare, b, &b_route(1), MoveError::NotMover { entity: b });
}

#[test]
fn c3_route_checks_follow_the_turn_checks() {
    let (_, mut world, [_, b, _]) = tb();
    let from_origin = route(1, vec![p(0, 0), p(1, 0)]);
    assert_fails(&mut world, b, &from_origin, MoveError::StartMismatch);
    // Each route error of validate_route surfaces unwrapped.
    let one_point = route(1, vec![p(10, 0)]);
    assert_fails(
        &mut world,
        b,
        &one_point,
        MoveError::RouteSize { points: 1 },
    );
    let zero = route(1, vec![p(10, 0), p(10, 0)]);
    assert_fails(&mut world, b, &zero, MoveError::ZeroSegment { index: 0 });
    let mut stale = b_route(1);
    stale.revision = movement_schedule::other();
    assert_fails(&mut world, b, &stale, MoveError::StaleRevision);
}

#[test]
fn c3_nav_unbound() {
    let fixture = move_fixture();
    let mut world = World::new(7);
    fixture.register(&mut world, pl(113)).expect("C");
    fixture
        .with_spec(|spec| start_encounter(&mut world, spec))
        .expect("starts");
    let ids: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
    assert_eq!(world.nav(), None);
    assert_fails(&mut world, ids[1], &b_route(1), MoveError::NavUnbound);
}

#[test]
fn c3_pairs() {
    // NotMover beats NavUnbound.
    let fixture = move_fixture();
    let mut world = World::new(7);
    fixture.register(&mut world, pl(113)).expect("C");
    fixture
        .with_spec(|spec| start_encounter(&mut world, spec))
        .expect("starts");
    let b = world
        .combatants()
        .iter()
        .map(|(id, _)| id)
        .nth(1)
        .expect("B");
    let key = entity_json(b);
    let mut unbound_and_moverless = with_json(&world, |save| {
        save["movers"]
            .as_array_mut()
            .expect("movers")
            .retain(|entry| entry[0] != key);
    });
    assert_eq!(unbound_and_moverless.nav(), None);
    assert_fails(
        &mut unbound_and_moverless,
        b,
        &b_route(1),
        MoveError::NotMover { entity: b },
    );

    // StartMismatch beats CannotAfford: a 100 m route from the wrong start.
    let (_, mut bound, [_, b, _]) = tb();
    let far = route(1, vec![p(0, 0), p(100, 0)]);
    assert_fails(&mut bound, b, &far, MoveError::StartMismatch);
    // The same length from the right start is a CannotAfford.
    let far_from_b = route(1, vec![p(10, 0), p(110, 0)]);
    assert_fails(
        &mut bound,
        b,
        &far_from_b,
        MoveError::CannotAfford {
            cost: 67,
            current: 6,
        },
    );
}

// ---------------------------------------------------------------------------
// C-4
// ---------------------------------------------------------------------------

#[test]
fn c4_validate_move_is_read_only() {
    let (_, mut world, [_, b, _]) = tb();
    let before = bytes_of(&world);
    assert_eq!(validate_move(&world, b, &b_route(1)), Ok(4));
    assert_eq!(bytes_of(&world), before, "validate_move reads only");
    // And it agrees with what perform_move then spends.
    let outcome = perform_move(&mut world, b, &b_route(1)).expect("paid");
    assert_eq!(outcome.cost, 4);
    assert_eq!(pool_106(&world, b), 2);
}

#[test]
fn c4_a_turn_based_participant_cannot_walk() {
    let (_, mut world, [a, b, c]) = tb();
    assert_eq!(
        unchanged(&mut world, |w| w.set_route(b, b_route(1))),
        Err(MoveError::TurnBased { entity: b })
    );
    assert_eq!(
        unchanged(&mut world, |w| w
            .set_route(a, route(1, vec![p(0, 0), p(1, 0)]))),
        Err(MoveError::TurnBased { entity: a })
    );
    // C is a non-participant: it may walk, and ticking moves nobody else.
    world
        .set_route(c, route(2, vec![p(0, 10), p(5, 10)]))
        .expect("C walks");
    let (a_at, b_at) = (mover_position(&world, a), mover_position(&world, b));
    for _ in 0..3 {
        tick(&mut world);
    }
    assert_eq!(mover_position(&world, a), a_at);
    assert_eq!(mover_position(&world, b), b_at);
    assert_eq!(mover_position(&world, c), p(3, 10));
    // A paid move still works after ticks.
    let outcome = perform_move(&mut world, b, &b_route(1)).expect("B moves");
    assert_eq!(outcome.from, p(10, 0));
}

#[test]
fn c4_a_move_leaves_other_movers_and_routes_alone() {
    let (_, mut world, [a, b, c]) = tb();
    world
        .set_route(c, route(5, vec![p(0, 10), p(5, 10)]))
        .expect("C walks");
    let c_mover = world.movers().get(c).expect("C").clone();
    let a_mover = world.movers().get(a).expect("A").clone();
    perform_move(&mut world, b, &b_route(1)).expect("B moves");
    assert_eq!(world.movers().get(c), Some(&c_mover));
    assert_eq!(world.movers().get(a), Some(&a_mover));
}

// ---------------------------------------------------------------------------
// C-5
// ---------------------------------------------------------------------------

/// A fresh MOVE history wrapper with C registered and the navmesh bound.
fn history_world(fixture: &Fixture) -> (HistoryWorld, EntityId) {
    let mut history = HistoryWorld::new_in_area(42, fixture.area);
    let c = fixture
        .register_history(&mut history, pl(113))
        .expect("C registers");
    history.bind_nav(rev()).expect("binds");
    (history, c)
}

fn started_history(fixture: &Fixture) -> (HistoryWorld, [EntityId; 2]) {
    let (mut history, _) = history_world(fixture);
    fixture
        .with_spec(|spec| history.start_encounter(spec))
        .expect("the turn-based start");
    let ids: Vec<EntityId> = history
        .world()
        .combatants()
        .iter()
        .map(|(id, _)| id)
        .collect();
    (history, [ids[0], ids[1]])
}

#[test]
fn c5_a_move_journals_and_raises_the_version() {
    let fixture = move_fixture();
    let (mut history, [a, b]) = started_history(&fixture);
    assert_eq!(
        history.version(),
        1,
        "a fresh wrapper and a TB start keep 1"
    );
    let mark = history.last_sequence();

    let outcome = history.perform_move(b, &b_route(1)).expect("B moves");
    assert_eq!((outcome.cost, outcome.to), (4, p(7, 3)));
    assert_eq!(history.version(), 3);
    assert_eq!(history.last_sequence(), mark + 1);
    let journal = history.read_after(mark, 10).expect("reads");
    assert_eq!(journal.len(), 1);
    assert_eq!(
        journal[0].payload,
        HistoryEvent::Moved {
            entity: b,
            points: vec![
                rp(655360, 0, 0),
                rp(655360, 0, 196608),
                rp(458752, 0, 196608)
            ],
        }
    );
    assert_eq!(pool_106(history.world(), b), 2);
    assert_eq!(mover_position(history.world(), b), p(7, 3));

    // A second move by the next actor journals the next event; 3 stays 3.
    history
        .perform_action(&CombatAction::EndTurn { actor: b })
        .expect("B ends the turn");
    history
        .perform_move(a, &route(2, vec![p(0, 0), p(3, 0)]))
        .expect("A moves");
    assert_eq!(history.version(), 3);

    // Release and acknowledgement keep 3.
    let last = history.last_sequence();
    history.acknowledge(last).expect("acknowledges");
    assert_eq!(history.version(), 3);
    history.end_encounter().expect("releases");
    assert_eq!(history.version(), 3);
}

#[test]
fn c5_a_rejected_move_commits_nothing() {
    let fixture = move_fixture();
    let (mut history, [a, b]) = started_history(&fixture);
    let before = serde_json::to_vec(&history).expect("serializes");
    let counters = |h: &HistoryWorld| (h.pending_len(), h.last_sequence(), h.version());
    let marks = counters(&history);

    let wrong_turn = history.perform_move(a, &route(1, vec![p(0, 0), p(1, 0)]));
    assert_eq!(
        wrong_turn,
        Err(HistoryError::Move(MoveError::Combat(
            CombatError::OutOfTurn {
                actor: a,
                active: Some(b)
            }
        )))
    );
    let too_far = route(1, vec![p(10, 0), p(10, 30)]);
    assert_eq!(
        history.perform_move(b, &too_far),
        Err(HistoryError::Move(MoveError::CannotAfford {
            cost: 20,
            current: 6
        }))
    );
    assert_eq!(counters(&history), marks);
    assert_eq!(history.version(), 1);
    assert_eq!(serde_json::to_vec(&history).expect("serializes"), before);
}

#[test]
fn c5_an_rt_capable_start_is_2_and_a_move_makes_it_3() {
    let fixture = move_rt_fixture();
    let (mut history, _) = history_world(&fixture);
    assert_eq!(history.version(), 1);
    fixture
        .with_spec(|spec| history.start_encounter_in_mode(spec, EncounterMode::TurnBased))
        .expect("an RT-capable encounter starts turn-based");
    assert_eq!(history.version(), 2, "an RT-capable wrapper writes 2");
    let b = history
        .world()
        .combatants()
        .iter()
        .map(|(id, _)| id)
        .nth(1)
        .expect("B");
    history.perform_move(b, &b_route(1)).expect("B moves");
    assert_eq!(history.version(), 3);
}

/// A version 3 wrapper holding a `Moved`, as a JSON value.
fn v3_wrapper() -> (HistoryWorld, Value) {
    let fixture = move_fixture();
    let (mut history, [_, b]) = started_history(&fixture);
    history.perform_move(b, &b_route(1)).expect("B moves");
    let value = serde_json::to_value(&history).expect("serializes");
    (history, value)
}

#[test]
fn c5_a_v3_wrapper_round_trips_byte_identically() {
    let (history, value) = v3_wrapper();
    assert_eq!(value["version"], json!(3));
    let bytes = serde_json::to_vec(&history).expect("serializes");
    let loaded: HistoryWorld = serde_json::from_slice(&bytes).expect("a v3 wrapper loads");
    assert_eq!(loaded, history);
    assert_eq!(serde_json::to_vec(&loaded).expect("serializes"), bytes);
    assert_eq!(
        crpg_sim::history_hash(&loaded),
        crpg_sim::history_hash(&history)
    );
    assert_eq!(loaded.version(), 3);
}

#[test]
fn c5_the_moved_payload_is_adjacent_tagged_raw_integers() {
    let (history, value) = v3_wrapper();
    let b = history
        .world()
        .combatants()
        .iter()
        .map(|(id, _)| id)
        .nth(1)
        .expect("B");
    let pending = value["pending"].as_array().expect("pending");
    let moved = pending.last().expect("the Moved envelope");
    assert_eq!(
        moved["payload"],
        json!({
            "type": "moved",
            "value": {
                "entity": entity_json(b),
                "points": [[655360, 0, 0], [655360, 0, 196608], [458752, 0, 196608]]
            }
        })
    );
}

#[test]
fn c5_version_rules_on_load() {
    let (_, value) = v3_wrapper();
    let load = |version: u64| {
        let mut forged = value.clone();
        forged["version"] = json!(version);
        serde_json::from_value::<HistoryWorld>(forged).map_err(|error| error.to_string())
    };
    for version in [1, 2] {
        assert_eq!(
            load(version).expect_err("a Moved below version 3 fails"),
            "history journal holds a v3 event below version 3",
            "version {version}"
        );
    }
    for version in [0, 4] {
        assert_eq!(
            load(version).expect_err("unknown version"),
            "unknown history version",
            "version {version}"
        );
    }
    assert!(load(3).is_ok());

    // Versions 1 and 2 still load without a Moved.
    let fresh = serde_json::to_value(HistoryWorld::new(0)).expect("serializes");
    for version in [1, 2, 3] {
        let mut forged = fresh.clone();
        forged["version"] = json!(version);
        assert!(
            serde_json::from_value::<HistoryWorld>(forged).is_ok(),
            "version {version} is readable"
        );
    }
}

#[test]
fn c5_a_v1_wrapper_is_judged_by_the_v1_rules_first() {
    // Version 1 holding a Moved and an RT-capable encounter: the existing v1
    // check runs before the v3 one.
    let fixture = move_rt_fixture();
    let (mut history, _) = history_world(&fixture);
    fixture
        .with_spec(|spec| history.start_encounter_in_mode(spec, EncounterMode::TurnBased))
        .expect("starts");
    let b = history
        .world()
        .combatants()
        .iter()
        .map(|(id, _)| id)
        .nth(1)
        .expect("B");
    history.perform_move(b, &b_route(1)).expect("B moves");
    let mut forged = serde_json::to_value(&history).expect("serializes");
    forged["version"] = json!(1);
    assert_eq!(
        serde_json::from_value::<HistoryWorld>(forged)
            .expect_err("fails")
            .to_string(),
        "history v1 wrapper holds a real-time-capable encounter"
    );
}

/// The message of a failed `HistoryEvent` decode, without its position suffix.
fn payload_error(text: &str) -> String {
    let message = serde_json::from_str::<HistoryEvent>(text)
        .expect_err("the payload must not decode")
        .to_string();
    match message.split_once(" at line ") {
        Some((head, _)) => head.to_owned(),
        None => message,
    }
}

fn moved_text(value: &str) -> String {
    format!(r#"{{"type":"moved","value":{value}}}"#)
}

/// `count` valid points as JSON array text.
fn points_text(count: usize) -> String {
    let points: Vec<String> = (0..count)
        .map(|index| format!("[{},0,0]", index * 65536))
        .collect();
    format!("[{}]", points.join(","))
}

#[test]
fn c5_the_moved_decoder() {
    let entity = entity_json(sample_entity()).to_string();
    let two = points_text(2);
    let value = |body: &str| moved_text(&format!("{{{body}}}"));

    // A valid payload decodes.
    let ok: HistoryEvent =
        serde_json::from_str(&value(&format!(r#""entity":{entity},"points":{two}"#)))
            .expect("a valid payload decodes");
    assert_eq!(
        ok,
        HistoryEvent::Moved {
            entity: sample_entity(),
            points: vec![p(0, 0), rp(65536, 0, 0)],
        }
    );
    // Either key order decodes.
    assert!(serde_json::from_str::<HistoryEvent>(&value(&format!(
        r#""points":{two},"entity":{entity}"#
    )))
    .is_ok());
    // 2 and 64 points decode.
    for count in [2, 64] {
        let text = value(&format!(
            r#""entity":{entity},"points":{}"#,
            points_text(count)
        ));
        match serde_json::from_str::<HistoryEvent>(&text).expect("decodes") {
            HistoryEvent::Moved { points, .. } => assert_eq!(points.len(), count),
            other => panic!("unexpected event {other:?}"),
        }
    }

    // `points` bounds.
    let below = value(&format!(r#""entity":{entity},"points":{}"#, points_text(1)));
    assert_eq!(payload_error(&below), "history move points below 2");
    let empty = value(&format!(r#""entity":{entity},"points":[]"#));
    assert_eq!(payload_error(&empty), "history move points below 2");
    let sixty_five = value(&format!(
        r#""entity":{entity},"points":{}"#,
        points_text(65)
    ));
    assert_eq!(payload_error(&sixty_five), "history move points exceed 64");
    // The 65th element is never decoded: its content does not matter.
    let sixty_four = points_text(64);
    let with_junk = format!("{},\"x\"]", &sixty_four[..sixty_four.len() - 1]);
    let junk = value(&format!(r#""entity":{entity},"points":{with_junk}"#));
    assert_eq!(payload_error(&junk), "history move points exceed 64");

    // Keys.
    assert_eq!(
        payload_error(&value(&format!(
            r#""entity":{entity},"entity":{entity},"points":{two}"#
        ))),
        "duplicate history payload field `entity`"
    );
    assert_eq!(
        payload_error(&value(&format!(
            r#""entity":{entity},"points":{two},"points":{two}"#
        ))),
        "duplicate history payload field `points`"
    );
    assert_eq!(
        payload_error(&value(&format!(
            r#""entity":{entity},"points":{two},"extra":1"#
        ))),
        "unknown history payload field"
    );
    assert_eq!(
        payload_error(&value(&format!(r#""points":{two}"#))),
        "history payload misses `entity`"
    );
    assert_eq!(
        payload_error(&value(&format!(r#""entity":{entity}"#))),
        "history payload misses `points`"
    );

    // Stream order: the first bad key or value wins.
    assert_eq!(
        payload_error(&value(&format!(
            r#""entity":{entity},"entity":{entity},"extra":1,"points":{two}"#
        ))),
        "duplicate history payload field `entity`"
    );
    assert_eq!(
        payload_error(&value(&format!(
            r#""extra":1,"entity":{entity},"entity":{entity},"points":{two}"#
        ))),
        "unknown history payload field"
    );
    assert_eq!(
        payload_error(&value(r#""points":[[0,0,0]]"#)),
        "history move points below 2",
        "no `entity`: the points fail first, in stream order"
    );
}

#[test]
fn c5_is_v3_is_only_moved() {
    // Every other variant keeps a v1 wrapper at 1 or a v2 wrapper at 2.
    let fixture = move_fixture();
    let (mut history, [_, b]) = started_history(&fixture);
    assert_eq!(history.version(), 1);
    history
        .perform_action(&CombatAction::EndTurn { actor: b })
        .expect("ends");
    assert_eq!(history.version(), 1, "turn events do not raise the version");
    history.tick().expect("ticks");
    history.place(b, p(10, 0)).expect("places");
    assert_eq!(
        history.version(),
        1,
        "zero-event operations keep the version"
    );
}

// ---------------------------------------------------------------------------
// C-6
// ---------------------------------------------------------------------------

#[test]
fn c6_the_definition_persists_its_movement() {
    let (_, world, _) = tb();
    let save = value_of(&world);
    assert_eq!(
        save["combat"]["definition"]["movement"],
        json!({"measure": "euclidean", "pool": uid(106), "step_cost": 1})
    );
    assert_eq!(
        world.combat().expect("encounter").definition.movement,
        Some(TurnMovement {
            measure: Measure::Euclidean,
            pool: Ulid::from_u128(106),
            step_cost: 1,
        })
    );
    // The world with movement round-trips byte for byte.
    let reloaded: World = serde_json::from_slice(&bytes_of(&world)).expect("reloads");
    assert_eq!(reloaded, world);
    assert_eq!(bytes_of(&reloaded), bytes_of(&world));
}

#[test]
fn c6_the_standard_definition_keeps_its_fifteen_fields() {
    let fixture = Fixture::standard();
    let (world, _) = fixture.start(1);
    let save = value_of(&world);
    let definition = save["combat"]["definition"]
        .as_object()
        .expect("the definition");
    assert!(!definition.contains_key("movement"));
    assert_eq!(definition.len(), 15);
    assert_eq!(world.combat().expect("encounter").definition.movement, None);
}

#[test]
fn c6_load_checks() {
    let (_, world, _) = tb();
    let error = |edit: fn(&mut Value)| {
        try_with_json(&world, edit).expect_err("the edited save must not load")
    };
    assert_eq!(
        error(|save| save["combat"]["definition"]["movement"]["pool"] = json!(uid(999))),
        "combat movement pool outside its pools"
    );
    assert_eq!(
        error(|save| save["combat"]["definition"]["movement"]["step_cost"] = json!(0)),
        "combat movement step cost is zero"
    );
    // The control: another declared pool loads (the primary pool 105).
    assert!(try_with_json(&world, |save| {
        save["combat"]["definition"]["movement"]["pool"] = json!(uid(105))
    })
    .is_ok());
    // An unknown movement key is rejected.
    assert!(try_with_json(&world, |save| {
        save["combat"]["definition"]["movement"]["extra"] = json!(1)
    })
    .is_err());
}

#[test]
fn c6_start_checks_the_movement_pool() {
    let mut fixture = move_fixture();
    edit_ruleset(&mut fixture, |ruleset| {
        ruleset["movement"]["turn"]["pool"] = json!(uid(999));
    });
    let mut world = World::new(7);
    assert_eq!(
        support::draws::start_into(&fixture, &mut world),
        Err(CombatError::MissingPool {
            pool: Ulid::from_u128(999)
        })
    );
    assert!(world.combat().is_none());

    let mut free = move_fixture();
    edit_ruleset(&mut free, |ruleset| {
        ruleset["movement"]["turn"]["step_cost"] = json!(0);
    });
    let mut world = World::new(7);
    assert_eq!(
        support::draws::start_into(&free, &mut world),
        Err(CombatError::InvalidCost { cost: 0, max: 6 })
    );
    assert!(world.combat().is_none());
}

#[test]
fn c6_the_mover_checks_win_over_the_movement_pool() {
    // A bad speed (a T036a check) wins over a bad movement pool (B-C2).
    let mut fixture = move_fixture();
    edit_ruleset(&mut fixture, |ruleset| {
        ruleset["movement"]["turn"]["pool"] = json!(uid(999));
    });
    movement_schedule::set_creature_stat(&mut fixture, 121, "pace", 0);
    let mut world = World::new(7);
    assert!(matches!(
        support::draws::start_into(&fixture, &mut world),
        Err(CombatError::InvalidSpeed { .. })
    ));
}

#[test]
fn c6_a_legacy_shape_with_movement_is_written_non_legacy() {
    let (_, world, _) = started(move1_fixture());
    let save = value_of(&world);
    let definition = save["combat"]["definition"]
        .as_object()
        .expect("the definition");
    for key in ["abilities", "pools", "effects", "movement"] {
        assert!(definition.contains_key(key), "key {key}");
    }
    assert_eq!(
        definition["movement"],
        json!({"measure": "euclidean", "pool": uid(105), "step_cost": 1})
    );
    let bytes = bytes_of(&world);
    let reloaded: World = serde_json::from_slice(&bytes).expect("reloads");
    assert_eq!(reloaded, world);
    assert!(reloaded
        .combat()
        .expect("encounter")
        .definition
        .movement
        .is_some());
    assert_eq!(bytes_of(&reloaded), bytes, "re-serializes identically");
}

// ---------------------------------------------------------------------------
// Errors and exports
// ---------------------------------------------------------------------------

#[test]
fn display_and_source_of_the_move_errors() {
    let error = MoveError::CannotAfford {
        cost: 7,
        current: 6,
    };
    assert_eq!(error.to_string(), "CannotAfford at combat/pool");
    assert!(error.source().is_none());
    let wrapped = MoveError::Combat(CombatError::NoEncounter);
    assert_eq!(wrapped.to_string(), "NoEncounter at combat");
    assert_eq!(
        wrapped.source().map(ToString::to_string),
        Some("NoEncounter at combat".to_owned())
    );
    let history = HistoryError::Move(wrapped);
    assert_eq!(history.to_string(), "NoEncounter at combat");
    assert!(history.source().is_some());
    assert_eq!(HISTORY_VERSION, 3);
}

#[test]
fn a_released_corpse_cannot_move() {
    // Dead records: end the encounter, then the validation has no encounter.
    let mut fixture = move_fixture();
    make_lethal(&mut fixture, 99);
    let (fixture, mut world, [a, b, _]) = started(fixture);
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B kills A");
    end_encounter(&mut world).expect("released");
    assert_fails(
        &mut world,
        a,
        &route(1, vec![p(0, 0), p(1, 0)]),
        MoveError::Combat(CombatError::NoEncounter),
    );
}
