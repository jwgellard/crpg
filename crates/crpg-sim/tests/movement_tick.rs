//! T036b: stage 4 of the tick, the movement facts and the transitions that
//! stop movers (Behaviour B-W1..B-W4, B-T1..B-T4; tests B-1..B-7 and B-9; the
//! golden B-8 is `movement_golden.rs`).
//!
//! Every expected value is the contract's literal or is worked out by hand
//! from it: stage 4 walks each route by the mover's speed in raw U4 units,
//! carries leftover budget over a vertex, and discards it at arrival.

#[allow(dead_code)]
mod support;

// The MOVE fixture is shared with the golden schedule, one source.
#[path = "support/movement_schedule.rs"]
mod movement_schedule;

use crpg_core::EntityId;
use crpg_sim::{
    end_encounter, join_encounter, leave_encounter, perform_action, request_mode, tick,
    tick_with_movement, CancelReason, CombatAction, EncounterMode, Entrant, HistoryWorld,
    MovementFact, RouteTag, World, RT_CLOCK_MAX,
};
use movement_schedule::{
    bits, move_fixture, move_rt_fixture, other, p, pl, rev, route, rp, set_creature_stat, Point,
};
use serde_json::{json, Value};
use support::creature::with_json;
use support::draws::start_into;
use support::rt::{bytes_of, make_lethal, use_action, value_of};
use support::Fixture;

fn reg(fixture: &Fixture, world: &mut World, placement: u128) -> EntityId {
    fixture
        .register(world, pl(placement))
        .expect("the placement registers")
}

/// MOVE, `World::new(7)`, A, B and C registered, the navmesh bound.
fn registered() -> (Fixture, World, [EntityId; 3]) {
    let fixture = move_fixture();
    let mut world = World::new(7);
    let a = reg(&fixture, &mut world, 111);
    let b = reg(&fixture, &mut world, 112);
    let c = reg(&fixture, &mut world, 113);
    world.bind_nav(rev());
    (fixture, world, [a, b, c])
}

fn set(world: &mut World, entity: EntityId, tag: u64, points: Vec<Point>) {
    world
        .set_route(entity, route(tag, points))
        .expect("the route is accepted");
}

fn position(world: &World, entity: EntityId) -> Point {
    world.movers().get(entity).expect("a mover").position()
}

fn route_tag(world: &World, entity: EntityId) -> Option<u64> {
    world
        .movers()
        .get(entity)
        .expect("a mover")
        .route()
        .map(|active| active.tag().0)
}

/// `(position bits, velocity bits)` of the entity's mirror.
fn mirror_of(world: &World, entity: EntityId) -> ([u32; 3], [u32; 3]) {
    let transform = world.transforms().get(entity).expect("a mirror");
    (bits(transform.position), bits(transform.velocity))
}

fn velocity_bits(world: &World, entity: EntityId) -> [u32; 3] {
    mirror_of(world, entity).1
}

fn without_tick(world: &World) -> Value {
    let mut value = value_of(world);
    value.as_object_mut().expect("a world").remove("tick");
    value
}

fn arrived(entity: EntityId, tag: u64) -> MovementFact {
    MovementFact::Arrived {
        entity,
        tag: RouteTag(tag),
    }
}

fn cancelled(entity: EntityId, tag: u64) -> MovementFact {
    MovementFact::Cancelled {
        entity,
        tag: RouteTag(tag),
        reason: CancelReason::Dead,
    }
}

const ZERO: [u32; 3] = [0, 0, 0];

// ---------------------------------------------------------------------------
// B-1 .. B-3
// ---------------------------------------------------------------------------

#[test]
fn b1_single_walk() {
    let fixture = move_fixture();
    let mut world = World::new(7);
    let a = reg(&fixture, &mut world, 111);
    world.bind_nav(rev());
    set(&mut world, a, 7, vec![p(0, 0), p(3, 4)]);
    // W5: no tick yet, no movement.
    assert_eq!(position(&world, a), p(0, 0));
    assert_eq!(velocity_bits(&world, a), ZERO);

    let expected = [
        rp(39321, 0, 52428),
        rp(78643, 0, 104857),
        rp(117964, 0, 157286),
        rp(157286, 0, 209715),
        p(3, 4),
        p(3, 4),
    ];
    for (index, want) in expected.iter().enumerate() {
        let tick_number = index + 1;
        let facts = tick_with_movement(&mut world);
        assert_eq!(position(&world, a), *want, "A after tick {tick_number}");
        if tick_number == 5 {
            assert_eq!(facts, vec![arrived(a, 7)]);
        } else {
            assert!(facts.is_empty(), "facts of tick {tick_number}");
        }
        if tick_number == 1 {
            let tick_one = ([0x3f19_9900, 0, 0x3f4c_cc00], [0x3f19_9900, 0, 0x3f4c_cc00]);
            assert_eq!(mirror_of(&world, a), tick_one);
        }
        if tick_number == 4 {
            let active = world.movers().get(a).expect("a mover").route();
            let active = active.expect("still walking");
            assert_eq!((active.segment(), active.along()), (0, 262144));
        }
    }
    assert_eq!(
        mirror_of(&world, a),
        ([0x4040_0000, 0, 0x4080_0000], ZERO),
        "tick 6: at rest"
    );
    assert_eq!(route_tag(&world, a), None);
}

#[test]
fn b1_the_budget_lands_exactly_on_a_vertex_and_carries_on() {
    // Speed 1 m per tick, route 1 m east then 1 m south: tick 1 ends exactly on
    // the vertex (budget == left), tick 2 arrives.
    let (_, mut world, [a, ..]) = registered();
    set(&mut world, a, 4, vec![p(0, 0), p(1, 0), p(1, 1)]);
    assert!(tick_with_movement(&mut world).is_empty());
    assert_eq!(position(&world, a), p(1, 0));
    let active = world.movers().get(a).expect("a mover").route();
    let active = active.expect("still walking");
    assert_eq!((active.segment(), active.along()), (1, 0));
    assert_eq!(
        mirror_of(&world, a),
        ([0x3f80_0000, 0, 0], [0x3f80_0000, 0, 0])
    );
    assert_eq!(tick_with_movement(&mut world), vec![arrived(a, 4)]);
    assert_eq!(position(&world, a), p(1, 1));
    assert_eq!(
        velocity_bits(&world, a),
        [0, 0, 0x3f80_0000],
        "the second leg moves along z"
    );
}

#[test]
fn b1_a_negative_displacement_gives_a_negative_velocity() {
    // B walks west at 0.5 m per tick.
    let (_, mut world, [_, b, _]) = registered();
    set(&mut world, b, 6, vec![p(10, 0), p(0, 0)]);
    assert!(tick_with_movement(&mut world).is_empty());
    assert_eq!(position(&world, b), rp(622592, 0, 0));
    assert_eq!(
        mirror_of(&world, b),
        ([0x4118_0000, 0, 0], [0xbf00_0000, 0, 0])
    );
}

#[test]
fn b1_arrival_moves_every_axis_and_the_velocity_is_the_displacement() {
    // A route that descends in y and z: y is not walked, so it jumps to the
    // last point on arrival; the velocity is the whole displacement.
    let (_, mut world, [a, ..]) = registered();
    world.place(a, rp(0, 65536, 65536)).expect("placed");
    set(&mut world, a, 8, vec![rp(0, 65536, 65536), rp(0, 0, 0)]);
    assert_eq!(tick_with_movement(&mut world), vec![arrived(a, 8)]);
    assert_eq!(position(&world, a), rp(0, 0, 0));
    assert_eq!(velocity_bits(&world, a), [0, 0xbf80_0000, 0xbf80_0000]);
}

#[test]
fn b2_carry_over() {
    let mut fixture = move_fixture();
    set_creature_stat(&mut fixture, 121, "pace", 98304);
    let mut world = World::new(7);
    let a = reg(&fixture, &mut world, 111);
    world.bind_nav(rev());
    set(&mut world, a, 2, vec![p(0, 0), p(1, 0), p(1, 2)]);
    assert!(tick_with_movement(&mut world).is_empty());
    assert_eq!(position(&world, a), rp(65536, 0, 32768));
    let active = world.movers().get(a).expect("a mover").route();
    let active = active.expect("still walking");
    assert_eq!((active.segment(), active.along()), (1, 32768));
    // Leftover budget never skips ahead past the last point.
    assert_eq!(tick_with_movement(&mut world), vec![arrived(a, 2)]);
    assert_eq!(position(&world, a), p(1, 2));
    assert_eq!(route_tag(&world, a), None);
}

#[test]
fn b2_a_budget_larger_than_the_whole_route_is_discarded() {
    let mut fixture = move_fixture();
    set_creature_stat(&mut fixture, 121, "pace", 262144);
    let mut world = World::new(7);
    let a = reg(&fixture, &mut world, 111);
    world.bind_nav(rev());
    // Three one-metre legs, a four-metre budget: it ends on the last point.
    set(&mut world, a, 9, vec![p(0, 0), p(1, 0), p(1, 1), p(2, 1)]);
    assert_eq!(tick_with_movement(&mut world), vec![arrived(a, 9)]);
    assert_eq!(position(&world, a), p(2, 1));
    // The tick after has no route to walk.
    assert!(tick_with_movement(&mut world).is_empty());
    assert_eq!(position(&world, a), p(2, 1));
}

#[test]
fn b3_order_and_independence() {
    let fixture = move_fixture();
    let mut world = World::new(7);
    let a = reg(&fixture, &mut world, 111);
    let b = reg(&fixture, &mut world, 112);
    world.bind_nav(rev());
    // B's route is set before A's.
    set(&mut world, b, 2, vec![p(10, 0), rp(655360, 0, 32768)]);
    set(&mut world, a, 1, vec![p(0, 0), p(1, 0)]);
    assert_eq!(
        tick_with_movement(&mut world),
        vec![arrived(a, 1), arrived(b, 2)]
    );
}

#[test]
fn b3_reused_index_stage_four_follows_entity_order_not_store_order() {
    let fixture = move_fixture();
    let mut world = World::new(7);
    let a = reg(&fixture, &mut world, 111);
    let b = reg(&fixture, &mut world, 112);
    assert!(world.despawn(a));
    let c = reg(&fixture, &mut world, 113);
    assert_eq!(
        world
            .movers()
            .iter()
            .map(|(entity, _)| entity)
            .collect::<Vec<_>>(),
        vec![b, c],
        "C is stored after B"
    );
    assert!(c < b);
    world.bind_nav(rev());
    set(&mut world, b, 2, vec![p(10, 0), rp(655360, 0, 32768)]);
    set(&mut world, c, 3, vec![p(0, 10), p(1, 10)]);
    assert_eq!(
        tick_with_movement(&mut world),
        vec![arrived(c, 3), arrived(b, 2)]
    );
}

#[test]
fn b3_a_world_of_routeless_movers_changes_only_the_tick_counter() {
    let (_, mut world, _) = registered();
    let before = without_tick(&world);
    let tick_before = world.tick();
    assert!(tick_with_movement(&mut world).is_empty());
    tick(&mut world);
    assert_eq!(without_tick(&world), before);
    assert_eq!(world.tick().get(), tick_before.get() + 2);
}

#[test]
fn b3_tick_is_tick_with_movement_without_the_facts() {
    let (_, mut one, [a, ..]) = registered();
    let mut two = one.clone();
    set(&mut one, a, 1, vec![p(0, 0), p(1, 0)]);
    set(&mut two, a, 1, vec![p(0, 0), p(1, 0)]);
    tick(&mut one);
    assert_eq!(tick_with_movement(&mut two), vec![arrived(a, 1)]);
    assert_eq!(bytes_of(&one), bytes_of(&two));
}

// ---------------------------------------------------------------------------
// B-W1, B-W3: when stage 4 runs and what it leaves alone
// ---------------------------------------------------------------------------

#[test]
fn w1_stage_four_runs_with_no_encounter_in_turn_based_mode_and_after_the_clock_freezes() {
    // No encounter at all (the registered worlds above); a turn-based
    // encounter: a non-participant keeps walking.
    let fixture = move_fixture();
    let (mut world, _) = fixture.start(7);
    let c = reg(&fixture, &mut world, 113);
    world.bind_nav(rev());
    set(&mut world, c, 3, vec![p(0, 10), p(5, 10)]);
    assert_eq!(
        world.combat().expect("an encounter").mode,
        EncounterMode::TurnBased
    );
    assert!(tick_with_movement(&mut world).is_empty());
    assert_eq!(position(&world, c), rp(65536, 0, 655360));

    // A real-time encounter whose clock has frozen at its bound.
    let fixture = move_rt_fixture();
    let (world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let mut frozen = with_json(&world, |save| {
        save["combat"]["clock"]["elapsed"] = json!(RT_CLOCK_MAX);
    });
    frozen.bind_nav(rev());
    set(&mut frozen, ids[0], 1, vec![p(0, 0), p(3, 4)]);
    assert!(tick_with_movement(&mut frozen).is_empty());
    assert_eq!(
        frozen
            .combat()
            .expect("an encounter")
            .clock
            .expect("a clock")
            .elapsed,
        RT_CLOCK_MAX,
        "the clock stays frozen"
    );
    assert_eq!(position(&frozen, ids[0]), rp(39321, 0, 52428));
}

#[test]
fn w3_stage_four_touches_no_pool_order_timeline_rng_or_event() {
    let fixture = move_rt_fixture();
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    world.bind_nav(rev());
    let (a, b) = (ids[0], ids[1]);
    set(&mut world, a, 1, vec![p(0, 0), p(3, 4)]);
    set(&mut world, b, 2, vec![p(10, 0), p(10, 2)]);
    let before = value_of(&world);
    let pools_before: Vec<u32> = ids
        .iter()
        .map(|id| support::rt::pool_of(&world, *id))
        .collect();
    assert!(tick_with_movement(&mut world).is_empty());
    let after = value_of(&world);
    for key in ["rng", "events", "timeline", "combatants"] {
        assert!(before.get(key).is_some(), "the world has `{key}`");
        assert_eq!(before[key], after[key], "stage 4 left `{key}` alone");
    }
    let pools_after: Vec<u32> = ids
        .iter()
        .map(|id| support::rt::pool_of(&world, *id))
        .collect();
    assert_eq!(pools_before, pools_after);
    // A route and the walk never end a turn or clear an order.
    assert_eq!(
        world.combat().expect("an encounter").mode,
        EncounterMode::RealTime
    );
}

// ---------------------------------------------------------------------------
// B-4
// ---------------------------------------------------------------------------

#[test]
fn b4_dead_cancel() {
    let mut fixture = move_rt_fixture();
    make_lethal(&mut fixture, 99);
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    world.bind_nav(rev());
    let (a, b) = (ids[0], ids[1]);
    set(&mut world, a, 1, vec![p(0, 0), p(3, 4)]);
    set(&mut world, b, 2, vec![p(10, 0), p(10, 2)]);
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("A's order");
    // A route set after the order does not clear it.
    assert!(world.combatants().get(a).expect("A").order().is_some());

    let facts = tick_with_movement(&mut world);
    assert_eq!(facts, vec![cancelled(b, 2)]);
    assert!(world.combatants().get(b).expect("B").dead());
    assert_eq!(position(&world, b), p(10, 0));
    assert_eq!(route_tag(&world, b), None);
    assert_eq!(position(&world, a), rp(39321, 0, 52428));
    // The order running does not clear A's route.
    assert_eq!(route_tag(&world, a), Some(1));
    assert_eq!(velocity_bits(&world, b), ZERO);
    // Afterwards the corpse produces no further fact.
    assert!(tick_with_movement(&mut world).is_empty());
}

#[test]
fn b4_a_dead_record_cancels() {
    let mut fixture = move_fixture();
    make_lethal(&mut fixture, 99);
    let (mut world, ids) = fixture.start(7);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B kills A");
    end_encounter(&mut world).expect("the encounter ends");
    assert!(world.creatures().get(a).expect("A's record").dead());
    world.bind_nav(rev());
    let key = serde_json::to_value(a).expect("an id");
    let revision = serde_json::to_value(rev()).expect("a revision");
    let mut world = with_json(&world, |save| {
        let entry = save["movers"]
            .as_array_mut()
            .expect("a mover list")
            .iter_mut()
            .find(|entry| entry[0] == key)
            .expect("A holds a mover");
        entry[1]["route"] = json!({
            "route": {
                "revision": revision,
                "tag": 5,
                "points": [[0, 0, 0], [196608, 0, 262144]]
            },
            "segment": 0,
            "along": 0
        });
    });
    assert_eq!(route_tag(&world, a), Some(5));
    assert_eq!(tick_with_movement(&mut world), vec![cancelled(a, 5)]);
    assert_eq!(position(&world, a), p(0, 0));
    assert_eq!(route_tag(&world, a), None);
    // The living record is untouched by the same stage.
    assert!(tick_with_movement(&mut world).is_empty());
}

// ---------------------------------------------------------------------------
// B-5: the transitions that stop movers
// ---------------------------------------------------------------------------

/// A, B and C registered in MOVE with routes A tag 1, B tag 2, C tag 3, and
/// one tick walked.
fn walking(fixture: &Fixture) -> (World, [EntityId; 3]) {
    let mut world = World::new(7);
    let a = reg(fixture, &mut world, 111);
    let b = reg(fixture, &mut world, 112);
    let c = reg(fixture, &mut world, 113);
    world.bind_nav(rev());
    set(&mut world, a, 1, vec![p(0, 0), p(3, 4)]);
    set(&mut world, b, 2, vec![p(10, 0), p(10, 2)]);
    set(&mut world, c, 3, vec![p(0, 10), p(12, 10)]);
    assert!(tick_with_movement(&mut world).is_empty());
    assert_eq!(position(&world, a), rp(39321, 0, 52428));
    assert_eq!(velocity_bits(&world, a), [0x3f19_9900, 0, 0x3f4c_cc00]);
    assert_eq!(position(&world, b), rp(655360, 0, 32768));
    assert_eq!(velocity_bits(&world, b), [0, 0, 0x3f00_0000]);
    assert_eq!(position(&world, c), rp(65536, 0, 655360));
    assert_eq!(velocity_bits(&world, c), [0x3f80_0000, 0, 0]);
    (world, [a, b, c])
}

#[test]
fn b5_a_turn_based_start_stops_the_entering_participants() {
    let fixture = move_fixture();
    let (mut world, [a, b, c]) = walking(&fixture);
    start_into(&fixture, &mut world).expect("the turn-based encounter starts");
    assert_eq!(
        world.combat().expect("an encounter").mode,
        EncounterMode::TurnBased
    );
    for (entity, at) in [(a, rp(39321, 0, 52428)), (b, rp(655360, 0, 32768))] {
        assert_eq!(route_tag(&world, entity), None);
        assert_eq!(position(&world, entity), at, "positions are unchanged");
        assert_eq!(velocity_bits(&world, entity), ZERO);
    }
    // C is no participant: its route and velocity are kept.
    assert_eq!(route_tag(&world, c), Some(3));
    assert_eq!(velocity_bits(&world, c), [0x3f80_0000, 0, 0]);
}

#[test]
fn b5_a_real_time_start_stops_nobody() {
    let fixture = move_rt_fixture();
    let (mut world, [a, b, c]) = walking(&fixture);
    fixture
        .with_spec(|spec| {
            crpg_sim::start_encounter_in_mode(&mut world, spec, EncounterMode::RealTime)
        })
        .expect("the real-time encounter starts");
    assert_eq!(route_tag(&world, a), Some(1));
    assert_eq!(route_tag(&world, b), Some(2));
    assert_eq!(route_tag(&world, c), Some(3));
    assert_eq!(velocity_bits(&world, a), [0x3f19_9900, 0, 0x3f4c_cc00]);
    assert_eq!(velocity_bits(&world, b), [0, 0, 0x3f00_0000]);
}

/// C registered with route tag 3 in a started encounter, one tick walked.
fn joiner_walking(fixture: &Fixture, mode: EncounterMode) -> (World, EntityId) {
    let (mut world, _) = fixture.start_rt(7, mode);
    let c = reg(fixture, &mut world, 113);
    world.bind_nav(rev());
    set(&mut world, c, 3, vec![p(0, 10), p(12, 10)]);
    assert!(tick_with_movement(&mut world).is_empty());
    assert_eq!(velocity_bits(&world, c), [0x3f80_0000, 0, 0]);
    (world, c)
}

#[test]
fn b5_a_joiner_of_a_turn_based_encounter_stops() {
    let fixture = move_rt_fixture();
    let (mut world, c) = joiner_walking(&fixture, EncounterMode::TurnBased);
    join_encounter(
        &mut world,
        &Entrant {
            entity: c,
            initiative: 5,
        },
    )
    .expect("C joins");
    assert!(world.combatants().contains(c));
    assert_eq!(route_tag(&world, c), None);
    assert_eq!(velocity_bits(&world, c), ZERO);
    assert_eq!(position(&world, c), rp(65536, 0, 655360));
}

#[test]
fn b5_a_joiner_of_a_real_time_encounter_keeps_walking() {
    let fixture = move_rt_fixture();
    let (mut world, c) = joiner_walking(&fixture, EncounterMode::RealTime);
    join_encounter(
        &mut world,
        &Entrant {
            entity: c,
            initiative: 5,
        },
    )
    .expect("C joins");
    assert!(world.combatants().contains(c));
    assert_eq!(route_tag(&world, c), Some(3));
    assert_eq!(velocity_bits(&world, c), [0x3f80_0000, 0, 0]);
}

#[test]
fn b5_the_switch_to_turn_based_stops_every_combatant_on_the_boundary() {
    let fixture = move_rt_fixture();
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    world.bind_nav(rev());
    let (a, b) = (ids[0], ids[1]);
    set(&mut world, a, 1, vec![p(0, 0), p(3, 4)]);
    set(&mut world, b, 2, vec![p(10, 0), p(10, 2), p(8, 2)]);
    assert!(tick_with_movement(&mut world).is_empty());
    request_mode(&mut world, EncounterMode::TurnBased).expect("a pending switch");
    for _ in 2..=3 {
        assert!(tick_with_movement(&mut world).is_empty());
        assert!(route_tag(&world, a).is_some() && route_tag(&world, b).is_some());
    }
    // Tick 4 is the round boundary: stage 2 switches and stops, stage 4 walks
    // nobody of the encounter.
    assert!(tick_with_movement(&mut world).is_empty());
    assert_eq!(
        world.combat().expect("an encounter").mode,
        EncounterMode::TurnBased
    );
    assert_eq!(route_tag(&world, a), None);
    assert_eq!(route_tag(&world, b), None);
    assert_eq!(position(&world, a), rp(117964, 0, 157286));
    assert_eq!(position(&world, b), rp(655360, 0, 98304));
    assert_eq!(velocity_bits(&world, a), ZERO);
    assert_eq!(velocity_bits(&world, b), ZERO);
}

#[test]
fn b5_the_switch_stops_a_dead_combatant_too() {
    // Three participants, so that killing B leaves a running RT encounter.
    let mut fixture = move_rt_fixture();
    make_lethal(&mut fixture, 99);
    support::rt::add_clone(&mut fixture, 114, 124, 8);
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    world.bind_nav(rev());
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("A's order");
    assert!(tick_with_movement(&mut world).is_empty());
    assert!(world.combatants().get(b).expect("B").dead());
    for _ in 2..=3 {
        assert!(tick_with_movement(&mut world).is_empty());
    }
    // Give the dead B a route in the save, then let the tick-4 boundary switch.
    let key = serde_json::to_value(b).expect("an id");
    let revision = serde_json::to_value(rev()).expect("a revision");
    let mut world = with_json(&world, |save| {
        let entry = save["movers"]
            .as_array_mut()
            .expect("a mover list")
            .iter_mut()
            .find(|entry| entry[0] == key)
            .expect("B holds a mover");
        entry[1]["route"] = json!({
            "route": {
                "revision": revision,
                "tag": 2,
                "points": [[655360, 0, 0], [655360, 0, 131072]]
            },
            "segment": 0,
            "along": 0
        });
    });
    request_mode(&mut world, EncounterMode::TurnBased).expect("a pending switch");
    assert_eq!(route_tag(&world, b), Some(2));
    // The switch stops the dead combatant's route in stage 2, so stage 4 has
    // nothing to cancel: no fact at all.
    assert!(tick_with_movement(&mut world).is_empty());
    assert_eq!(
        world.combat().expect("an encounter").mode,
        EncounterMode::TurnBased
    );
    assert_eq!(route_tag(&world, b), None);
    assert_eq!(velocity_bits(&world, b), ZERO);
}

#[test]
fn b5_leaving_ending_and_a_turn_based_to_real_time_switch_stop_nobody() {
    // leave_encounter and end_encounter in RT.
    let fixture = move_rt_fixture();
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    world.bind_nav(rev());
    let (a, b) = (ids[0], ids[1]);
    set(&mut world, a, 1, vec![p(0, 0), p(3, 4)]);
    set(&mut world, b, 2, vec![p(10, 0), p(10, 2)]);
    assert!(tick_with_movement(&mut world).is_empty());
    let velocity = velocity_bits(&world, a);
    leave_encounter(&mut world, a).expect("A leaves");
    assert_eq!(route_tag(&world, a), Some(1), "leave keeps A's route");
    assert_eq!(velocity_bits(&world, a), velocity);
    end_encounter(&mut world).expect("the encounter ends");
    assert_eq!(route_tag(&world, a), Some(1));
    assert_eq!(route_tag(&world, b), Some(2), "end keeps every route");

    // A turn-based start leaves a non-participant walking; the TB-to-RT switch
    // at the rollover stops nobody either.
    let (mut world, ids) = {
        let mut world = World::new(7);
        let c = reg(&fixture, &mut world, 113);
        world.bind_nav(rev());
        set(&mut world, c, 3, vec![p(0, 10), p(12, 10)]);
        assert!(tick_with_movement(&mut world).is_empty());
        fixture
            .with_spec(|spec| {
                crpg_sim::start_encounter_in_mode(&mut world, spec, EncounterMode::TurnBased)
            })
            .expect("the turn-based encounter starts");
        let ids: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
        assert_eq!(route_tag(&world, c), Some(3), "a TB start spares C");
        (world, [ids[0], ids[1], c])
    };
    let c = ids[2];
    request_mode(&mut world, EncounterMode::RealTime).expect("a pending switch");
    for _ in 0..2 {
        let active = world
            .combat()
            .expect("an encounter")
            .active
            .expect("a turn head");
        perform_action(&mut world, &CombatAction::EndTurn { actor: active })
            .expect("the turn ends");
    }
    assert_eq!(
        world.combat().expect("an encounter").mode,
        EncounterMode::RealTime
    );
    assert_eq!(route_tag(&world, c), Some(3));
    assert_eq!(velocity_bits(&world, c), [0x3f80_0000, 0, 0]);
}

// ---------------------------------------------------------------------------
// B-6, B-7, B-9
// ---------------------------------------------------------------------------

#[test]
fn b6_mirror_rewrite() {
    let (_, mut world, [a, ..]) = registered();
    let before = mirror_of(&world, a);
    assert!(world.transforms_mut().remove(a).is_some());
    assert!(world.transforms().get(a).is_none());
    tick(&mut world);
    assert_eq!(
        mirror_of(&world, a),
        before,
        "stage 4 re-inserts the mirror"
    );

    // The standard fixture has no movement: ticking creates no transform.
    let standard = Fixture::standard();
    let mut world = World::new(7);
    let _ = reg(&standard, &mut world, 111);
    let _ = reg(&standard, &mut world, 112);
    assert!(world.movers().is_empty());
    tick(&mut world);
    assert!(world.transforms().is_empty());
    let (mut started, _) = standard.start(7);
    tick(&mut started);
    assert!(started.transforms().is_empty());
}

#[test]
fn b7_history_tick() {
    let fixture = move_fixture();
    let mut history = HistoryWorld::new(7);
    let mut plain = World::new(7);
    let a = fixture
        .register_history(&mut history, pl(111))
        .expect("A registers");
    let b = fixture
        .register_history(&mut history, pl(112))
        .expect("B registers");
    assert_eq!(a, reg(&fixture, &mut plain, 111));
    assert_eq!(b, reg(&fixture, &mut plain, 112));
    history.bind_nav(rev()).expect("the navmesh binds");
    plain.bind_nav(rev());
    history
        .set_route(b, route(2, vec![p(10, 0), rp(655360, 0, 32768)]))
        .expect("B's route");
    history
        .set_route(a, route(1, vec![p(0, 0), p(3, 4)]))
        .expect("A's route");
    set(&mut plain, b, 2, vec![p(10, 0), rp(655360, 0, 32768)]);
    set(&mut plain, a, 1, vec![p(0, 0), p(3, 4)]);

    let (pending, last, version) = (
        history.pending_len(),
        history.last_sequence(),
        history.version(),
    );
    for tick_number in 1..=6 {
        let from_history = history.tick_with_movement().expect("the tick commits");
        let from_plain = tick_with_movement(&mut plain);
        assert_eq!(from_history, from_plain, "facts of tick {tick_number}");
        if tick_number == 1 {
            assert_eq!(from_history, vec![arrived(b, 2)]);
        }
        if tick_number == 5 {
            assert_eq!(from_history, vec![arrived(a, 1)]);
        }
    }
    // The wrapper drains the legacy queue into its journal; everything else
    // is the plain world's.
    let mut staged = value_of(history.world());
    let mut expected = value_of(&plain);
    staged.as_object_mut().expect("a world").remove("events");
    expected.as_object_mut().expect("a world").remove("events");
    assert_eq!(staged, expected);
    assert_eq!(history.pending_len(), pending, "no movement history event");
    assert_eq!(history.last_sequence(), last);
    assert_eq!(history.version(), version);
    assert_eq!(version, 1);
}

#[test]
fn b7_history_tick_journals_exactly_what_tick_journals() {
    let mut fixture = move_rt_fixture();
    make_lethal(&mut fixture, 99);
    let run = |use_facts: bool| {
        let (mut history, ids) = fixture
            .start_history(7, EncounterMode::RealTime)
            .expect("the history starts");
        history.bind_nav(rev()).expect("the navmesh binds");
        let (a, b) = (ids[0], ids[1]);
        history
            .set_route(a, route(1, vec![p(0, 0), p(3, 4)]))
            .expect("A's route");
        history
            .set_route(b, route(2, vec![p(10, 0), p(10, 2)]))
            .expect("B's route");
        history
            .perform_action(&use_action(a, fixture.ability, b))
            .expect("A's order");
        let facts = if use_facts {
            history.tick_with_movement().expect("the tick commits")
        } else {
            history.tick().expect("the tick commits");
            Vec::new()
        };
        let journal = history
            .read_after(history.acknowledged(), 256)
            .expect("the journal reads");
        (history, ids, facts, journal)
    };
    let (with_facts, ids, facts, journal_a) = run(true);
    let (without, _, _, journal_b) = run(false);
    assert_eq!(facts, vec![cancelled(ids[1], 2)]);
    assert_eq!(journal_a, journal_b, "the journal is what `tick` journals");
    assert_eq!(with_facts.world(), without.world());
    assert_eq!(with_facts.version(), without.version());
}

#[test]
fn b9_operations_write_zero_velocity() {
    let walked = || {
        let fixture = move_fixture();
        let mut world = World::new(7);
        let a = reg(&fixture, &mut world, 111);
        world.bind_nav(rev());
        set(&mut world, a, 7, vec![p(0, 0), p(3, 4)]);
        tick(&mut world);
        assert_eq!(position(&world, a), rp(39321, 0, 52428));
        assert_eq!(velocity_bits(&world, a), [0x3f19_9900, 0, 0x3f4c_cc00]);
        (world, a)
    };

    let (mut world, a) = walked();
    world.place(a, p(2, 3)).expect("placed");
    assert_eq!(mirror_of(&world, a), ([0x4000_0000, 0, 0x4040_0000], ZERO));

    let (mut world, a) = walked();
    assert_eq!(world.clear_route(a), Ok(Some(RouteTag(7))));
    assert_eq!(
        mirror_of(&world, a),
        ([0x3f19_9900, 0, 0x3f4c_cc00], ZERO),
        "the position is kept"
    );

    let (mut world, a) = walked();
    assert_eq!(
        world.set_route(a, route(8, vec![rp(39321, 0, 52428), p(3, 4)])),
        Ok(Some(RouteTag(7)))
    );
    assert_eq!(velocity_bits(&world, a), ZERO);

    let (mut world, a) = walked();
    assert_eq!(world.bind_nav(other()), vec![(a, RouteTag(7))]);
    assert_eq!(velocity_bits(&world, a), ZERO);

    // Control: binding the revision already bound changes nothing.
    let (mut world, a) = walked();
    assert!(world.bind_nav(rev()).is_empty());
    assert_eq!(velocity_bits(&world, a), [0x3f19_9900, 0, 0x3f4c_cc00]);
}
