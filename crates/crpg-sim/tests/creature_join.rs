//! T139b2: joining a live encounter (Behaviour B23-B28 and B33; tests E5-E11).
//! E12-E16 (leaving, release, determinism) are in `creature_exit.rs`, and E4's
//! joiner cases are in `creature_entry.rs`.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Ulid};
use crpg_data::{ActionPoolTemplate, LateJoinWire, RefreshWire};
use crpg_rules::MAX_TAGS;
use crpg_sim::{
    join_encounter, perform_action, start_encounter_with_members, state_hash, CombatError,
    EncounterMode, EntityMeta, Entrant, SimEvent, World,
};
use serde_json::{json, Value};
use support::creature::{add_outsider, edit_combatant, set_hooks, try_with_json, with_record};
use support::draws::{draw, grant, oracle, set_initiative_rule};
use support::rt::{capable, make_lethal, pool_of, ticks, timeline_of, use_action, value_of};
use support::surface::{
    add_derived, always_success, attach_to_ability, binary, edit_ability, effect_doc, end, lit,
    modifier, stat,
};
use support::{uid, Fixture};

const J: u128 = 133;
const K: u128 = 134;

fn id(value: u128) -> Ulid {
    Ulid::from_u128(value)
}

fn bound(fixture: &Fixture) -> World {
    World::new_in_area(5, fixture.area)
}

/// Everything a failed operation must leave alone: the bytes, the hash, the
/// RNG (the serialization also holds the interners and events).
fn snapshot(world: &World) -> (Vec<u8>, [u8; 32], crpg_core::DeterministicRng) {
    let mut copy = world.clone();
    (
        serde_json::to_vec(world).expect("the world serializes"),
        state_hash(world),
        copy.rng_mut().clone(),
    )
}

fn events_of(world: &mut World) -> Vec<SimEvent> {
    world
        .events_mut()
        .drain()
        .into_iter()
        .map(|envelope| envelope.payload)
        .collect()
}

fn combatant_ids(world: &World) -> Vec<EntityId> {
    world
        .combatants()
        .iter()
        .map(|(entity, _)| entity)
        .collect()
}

fn entrant(entity: EntityId, initiative: i32) -> Entrant {
    Entrant { entity, initiative }
}

fn register(fixture: &Fixture, world: &mut World, placement: u128) -> EntityId {
    fixture
        .register(world, id(placement))
        .expect("the outsider registers")
}

/// A turn-based start with no members.
fn begin(fixture: &Fixture, world: &mut World) {
    fixture
        .with_spec(|spec| start_encounter_with_members(world, spec, EncounterMode::TurnBased, &[]))
        .expect("the encounter starts");
}

/// A join that must fail with `expected` and leave `world` exactly as it was.
fn join_fails(world: &mut World, joiner: Entrant, expected: CombatError) {
    let before = snapshot(world);
    assert_eq!(join_encounter(world, &joiner), Err(expected.clone()));
    assert_eq!(
        snapshot(world),
        before,
        "{expected:?} mutated the world, its RNG or its hash"
    );
}

fn pending_of(world: &World) -> Vec<EntityId> {
    world.combat().expect("combat").pending_joins.clone()
}

/// Standard fixture, outsiders J, K and L, and a second pool refreshed on
/// every round start, so that one `RoundStart` refresh shows as exactly one
/// refilled unit (a `refill` of one on the record's pool).
fn join_fixture(late: Option<LateJoinWire>) -> Fixture {
    let mut fixture = Fixture::standard();
    fixture.ruleset.pools.push(ActionPoolTemplate {
        id: id(106),
        max: 3,
        refresh: RefreshWire::OnRoundStart,
    });
    add_outsider(&mut fixture, J, 143);
    add_outsider(&mut fixture, K, 144);
    set_hooks(&mut fixture, None, late, None);
    fixture
}

/// Empties both of a record's pools and gives the round pool a refill of one.
fn drain_pools(world: &World, entity: EntityId) -> World {
    with_record(world, entity, |record| {
        record["pools"][0]["current"] = json!(0);
        record["pools"][1]["current"] = json!(0);
        record["pools"][1]["refill"] = json!(1);
    })
}

fn effect_501(duration: u32) -> crpg_data::Effect {
    effect_doc(
        501,
        "target",
        "probe-type",
        "stack_all",
        vec![modifier(601, json!({"type": "roll"}), "add", 1)],
        duration,
    )
}

/// E5 (B23, B24, B26): in TB round 0 under the default `late_join`, a joiner
/// whose key sorts after the head is inserted at once; one that sorts before
/// it has no entry and no pending entry, and enters at the rollover with one
/// `RoundStart` refresh. `active` is unchanged, nothing is enqueued, nothing
/// is interned and pools are not refreshed on entry.
///
/// **Fails if** the join refreshes a pool, enqueues an event, moves `active`,
/// inserts the early joiner, or compares keys without the entity tiebreak.
#[test]
fn e5_round_zero_join() {
    let fixture = join_fixture(None);
    let mut world = bound(&fixture);
    let j = register(&fixture, &mut world, J);
    let world = drain_pools(&world, j);

    // After the head: inserted at its key.
    let mut after = world.clone();
    begin(&fixture, &mut after);
    let ids = combatant_ids(&after);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(ids.len(), 2, "J is a record, not a participant");
    events_of(&mut after);
    let interners = after.interners().clone();
    let rng = after.rng_mut().clone();
    join_encounter(&mut after, &entrant(j, 12)).expect("J joins");
    assert_eq!(timeline_of(&after), [(9, b), (10, a), (12, j)]);
    assert_eq!(after.combat().expect("combat").active, Some(b));
    assert_eq!(after.combat().expect("combat").round, 0);
    assert!(pending_of(&after).is_empty());
    assert!(events_of(&mut after).is_empty(), "a join enqueues nothing");
    assert_eq!(*after.interners(), interners, "a join interns nothing");
    assert_eq!(*after.rng_mut(), rng, "no rule, so no draw");
    assert!(after.creatures().get(j).is_none(), "the record is consumed");
    let joined = after.combatants().get(j).expect("J is a combatant");
    assert!(joined.entered());
    assert_eq!(joined.initiative(), 12);
    assert_eq!(joined.action_pool().current(), 0, "no refresh on entry");
    assert_eq!(joined.extra_pools()[0].current(), 0, "no refresh on entry");
    // Control: only J changed in the combatant store's ids.
    assert_eq!(combatant_ids(&after), [a, b, j]);

    // Before the head: no entry, not pending, and the rollover enters it.
    let mut early = world.clone();
    begin(&fixture, &mut early);
    events_of(&mut early);
    let before_join = timeline_of(&early);
    join_encounter(&mut early, &entrant(j, 5)).expect("J joins");
    assert_eq!(
        timeline_of(&early),
        before_join,
        "a waiting joiner is unscheduled"
    );
    assert!(!early.timeline().contains(j));
    assert!(pending_of(&early).is_empty());
    assert_eq!(early.combat().expect("combat").active, Some(b));
    assert!(events_of(&mut early).is_empty());
    let waiting = early.combatants().get(j).expect("J is a combatant");
    assert_eq!(
        (
            waiting.action_pool().current(),
            waiting.extra_pools()[0].current()
        ),
        (0, 0)
    );
    assert!(
        value_of(&early)["combat"].get("pending_joins").is_none(),
        "the key is absent while nothing is pending"
    );
    // A waiting joiner is alive and counts: B ends, A ends, the round rolls.
    end(&mut early, b);
    assert_eq!(early.combat().expect("combat").active, Some(a));
    assert!(!early.timeline().contains(j), "still waiting in round 0");
    end(&mut early, a);
    let combat = early.combat().expect("combat");
    assert_eq!((combat.round, combat.active), (1, Some(j)));
    assert_eq!(timeline_of(&early), [(5, j), (9, b), (10, a)]);
    let entered = early.combatants().get(j).expect("J");
    assert_eq!(
        entered.extra_pools()[0].current(),
        1,
        "exactly one RoundStart refresh (refill one)"
    );
    assert_eq!(
        entered.action_pool().current(),
        1,
        "the head's TurnStart refresh"
    );

    // The tiebreak: key 9 equals the head's. J (registered before the
    // participants, so a lower id) sorts before the head and waits; K
    // (registered after, a higher id) sorts after it and is inserted.
    let mut tied = world.clone();
    begin(&fixture, &mut tied);
    let k = register(&fixture, &mut tied, K);
    assert!(j < combatant_ids(&tied)[1] && combatant_ids(&tied)[1] < k);
    join_encounter(&mut tied, &entrant(j, 9)).expect("J joins");
    assert!(!tied.timeline().contains(j), "(9, J) sorts before (9, B)");
    join_encounter(&mut tied, &entrant(k, 9)).expect("K joins");
    assert!(tied.timeline().contains(k), "(9, K) sorts after (9, B)");
}

/// E6 (B26): in TB round 1 or later a joiner waits even with a late key,
/// under both `late_join` values, and enters at the next rollover. Control: A
/// kills B while J waits, and the round rolls over instead of ending, because
/// J counts as alive; without J the same kill ends the encounter.
///
/// **Fails if** a later-round joiner is inserted, goes to `pending_joins`, or
/// is not counted as alive.
#[test]
fn e6_later_round_waits() {
    for late in [
        None,
        Some(LateJoinWire::NextRound),
        Some(LateJoinWire::AfterActive),
    ] {
        let fixture = join_fixture(late);
        let mut world = bound(&fixture);
        let j = register(&fixture, &mut world, J);
        begin(&fixture, &mut world);
        let ids = combatant_ids(&world);
        let (a, b) = (ids[0], ids[1]);
        end(&mut world, b);
        end(&mut world, a);
        assert_eq!(world.combat().expect("combat").round, 1, "{late:?}");
        assert_eq!(world.combat().expect("combat").active, Some(b));
        events_of(&mut world);

        join_encounter(&mut world, &entrant(j, 12)).expect("J joins");
        assert!(!world.timeline().contains(j), "{late:?}: J waits");
        assert!(pending_of(&world).is_empty(), "{late:?}: not pending");
        assert_eq!(timeline_of(&world), [(9, b), (10, a)], "{late:?}");
        assert!(events_of(&mut world).is_empty());
        // The next rollover enters it at its key.
        end(&mut world, b);
        end(&mut world, a);
        assert_eq!(world.combat().expect("combat").round, 2, "{late:?}");
        assert_eq!(
            timeline_of(&world),
            [(9, b), (10, a), (12, j)],
            "{late:?}: J enters at the rollover"
        );
    }

    // Control: A kills B while J waits; the round rolls over.
    let mut fixture = join_fixture(None);
    always_success(&mut fixture);
    make_lethal(&mut fixture, 10);
    let mut world = bound(&fixture);
    let j = register(&fixture, &mut world, J);
    begin(&fixture, &mut world);
    let ids = combatant_ids(&world);
    let (a, b) = (ids[0], ids[1]);
    end(&mut world, b);
    end(&mut world, a);
    join_encounter(&mut world, &entrant(j, 12)).expect("J joins");
    end(&mut world, b);
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("A attacks B");
    assert!(world.combatants().get(b).expect("B").dead());
    let combat = world.combat().expect("combat");
    assert_eq!(combat.round, 2, "the round rolled over");
    assert_eq!(combat.active, Some(a));
    assert_eq!(timeline_of(&world), [(10, a), (12, j)]);

    // Without J the same kill ends the encounter.
    let mut world = bound(&fixture);
    begin(&fixture, &mut world);
    let ids = combatant_ids(&world);
    let (a, b) = (ids[0], ids[1]);
    end(&mut world, b);
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("A attacks B");
    assert_eq!(world.combat().expect("combat").active, None);
}

/// E7 (B26, B27): under `late_join: after_active`, a round-0 joiner whose
/// slot has passed goes to `pending_joins` and acts right after the current
/// turn, once; pending joiners act in key order; one whose slot has not passed
/// is inserted at once; under `next_round` the same joiner waits.
///
/// **Fails if** `after_active` sends every round-0 joiner to `pending_joins`,
/// pending joiners are flushed in join order, the flush is skipped or leaves
/// the list populated, or the joiner gets no `TurnStart` refresh.
#[test]
fn e7_after_active() {
    let fixture = join_fixture(Some(LateJoinWire::AfterActive));
    let mut world = bound(&fixture);
    let j = register(&fixture, &mut world, J);
    let world = drain_pools(&world, j);
    let mut started = world.clone();
    begin(&fixture, &mut started);
    let ids = combatant_ids(&started);
    let (a, b) = (ids[0], ids[1]);

    // J at key 5 is pending.
    join_encounter(&mut started, &entrant(j, 5)).expect("J joins");
    assert_eq!(pending_of(&started), [j]);
    assert!(!started.timeline().contains(j));
    assert_eq!(started.combat().expect("combat").active, Some(b));
    let text = serde_json::to_string(&started).expect("serializes");
    assert!(
        text.contains("\"pending_joins\":[{\"index\":"),
        "a pending joiner is written"
    );
    let lifecycle = text.find("\"lifecycle\"").expect("lifecycle key");
    let pending = text.find("\"pending_joins\"").expect("pending key");
    assert!(lifecycle < pending, "pending_joins follows lifecycle");
    // B's EndTurn makes J active, with one TurnStart refresh.
    let mut run = started.clone();
    end(&mut run, b);
    assert_eq!(run.combat().expect("combat").active, Some(j));
    assert!(pending_of(&run).is_empty(), "the flush clears the list");
    assert_eq!(run.combat().expect("combat").round, 0);
    assert_eq!(timeline_of(&run), [(5, j), (10, a)]);
    assert_eq!(pool_of(&run, j), 1, "one TurnStart refresh");
    assert_eq!(
        run.combatants().get(j).expect("J").extra_pools()[0].current(),
        0,
        "no RoundStart refresh"
    );
    assert!(value_of(&run)["combat"].get("pending_joins").is_none());
    // J's EndTurn makes A active, still in round 0.
    end(&mut run, j);
    let combat = run.combat().expect("combat");
    assert_eq!((combat.active, combat.round), (Some(a), 0));

    // Control: under next_round, B's EndTurn makes A active, and J enters at
    // the rollover.
    let plain = join_fixture(Some(LateJoinWire::NextRound));
    let mut world = bound(&plain);
    let j2 = register(&plain, &mut world, J);
    begin(&plain, &mut world);
    let ids = combatant_ids(&world);
    join_encounter(&mut world, &entrant(j2, 5)).expect("J joins");
    assert!(pending_of(&world).is_empty());
    end(&mut world, ids[1]);
    assert_eq!(world.combat().expect("combat").active, Some(ids[0]));
    end(&mut world, ids[0]);
    assert_eq!(world.combat().expect("combat").round, 1);
    assert_eq!(timeline_of(&world)[0], (5, j2));

    // Two pending joiners: J2 (key 7) joined before J1 (key 3); after B's
    // turn, J1, then J2, then A.
    let mut world = bound(&fixture);
    let first = register(&fixture, &mut world, J);
    let second = register(&fixture, &mut world, K);
    begin(&fixture, &mut world);
    let ids = combatant_ids(&world);
    let (a, b) = (ids[0], ids[1]);
    join_encounter(&mut world, &entrant(second, 7)).expect("joins");
    join_encounter(&mut world, &entrant(first, 3)).expect("joins");
    assert_eq!(pending_of(&world), [second, first], "in join order");
    end(&mut world, b);
    assert_eq!(world.combat().expect("combat").active, Some(first));
    assert_eq!(timeline_of(&world), [(3, first), (7, second), (10, a)]);
    end(&mut world, first);
    assert_eq!(world.combat().expect("combat").active, Some(second));
    end(&mut world, second);
    let combat = world.combat().expect("combat");
    assert_eq!((combat.active, combat.round), (Some(a), 0));

    // After the head, under after_active: inserted at once.
    let mut world = bound(&fixture);
    let j3 = register(&fixture, &mut world, J);
    begin(&fixture, &mut world);
    let ids = combatant_ids(&world);
    let (a, b) = (ids[0], ids[1]);
    join_encounter(&mut world, &entrant(j3, 12)).expect("J3 joins");
    assert_eq!(timeline_of(&world), [(9, b), (10, a), (12, j3)]);
    assert!(pending_of(&world).is_empty());
    assert!(
        value_of(&world)["combat"].get("pending_joins").is_none(),
        "the key is absent"
    );
    let mut reloaded: World =
        serde_json::from_value(serde_json::to_value(&world).expect("serializes"))
            .expect("the save reloads");
    assert_eq!(state_hash(&reloaded), state_hash(&world));
    end(&mut world, b);
    end(&mut reloaded, b);
    assert_eq!(state_hash(&reloaded), state_hash(&world));
}

/// The after-active world of E8: J pending behind head B.
fn pending_world(fixture: &Fixture) -> (World, EntityId, EntityId, EntityId) {
    let mut world = bound(fixture);
    begin(fixture, &mut world);
    // Registered after the start, so J's id is above B's.
    let j = register(fixture, &mut world, J);
    let ids = combatant_ids(&world);
    join_encounter(&mut world, &entrant(j, 5)).expect("J joins");
    assert_eq!(pending_of(&world), [j]);
    (world, ids[0], ids[1], j)
}

/// E8 (B27, B30, B33): a pending joiner that leaves, dies or is despawned is
/// removed from `pending_joins` without touching B's turn; a dead one is not
/// inserted at B's `EndTurn`.
///
/// **Fails if** a leave or despawn leaves the entry behind, or the flush
/// inserts a dead pending joiner.
#[test]
fn e8_pending_edges() {
    let fixture = join_fixture(Some(LateJoinWire::AfterActive));

    // A pending J leaves: gone from pending_joins; B's turn is untouched.
    let (mut world, a, b, j) = pending_world(&fixture);
    let timeline = timeline_of(&world);
    crpg_sim::leave_encounter(&mut world, j).expect("J leaves");
    assert!(pending_of(&world).is_empty());
    assert_eq!(world.combat().expect("combat").active, Some(b));
    assert_eq!(timeline_of(&world), timeline);
    assert!(world.creatures().get(j).is_some(), "J is a record again");
    end(&mut world, b);
    assert_eq!(world.combat().expect("combat").active, Some(a));

    // A pending J is despawned: removed from pending_joins.
    let (mut world, _, b, j) = pending_world(&fixture);
    assert!(world.despawn(j));
    assert!(pending_of(&world).is_empty());
    assert_eq!(world.combat().expect("combat").active, Some(b));
    assert!(!world.contains(j));

    // B kills pending J: at B's EndTurn, J is not inserted, and A is head.
    let mut lethal = join_fixture(Some(LateJoinWire::AfterActive));
    always_success(&mut lethal);
    make_lethal(&mut lethal, 10);
    edit_ability(&mut lethal, |ability| ability.ends_turn = false);
    let (mut world, a, b, j) = pending_world(&lethal);
    perform_action(&mut world, &use_action(b, lethal.ability, j)).expect("B attacks J");
    assert!(world.combatants().get(j).expect("J").dead());
    assert_eq!(pending_of(&world), [j], "death leaves the entry in place");
    assert_eq!(world.combat().expect("combat").active, Some(b));
    // The save with a dead pending joiner loads and continues equally.
    let mut reloaded: World =
        serde_json::from_value(serde_json::to_value(&world).expect("serializes"))
            .expect("a dead pending joiner round-trips");
    assert_eq!(state_hash(&reloaded), state_hash(&world));
    end(&mut world, b);
    end(&mut reloaded, b);
    assert_eq!(world.combat().expect("combat").active, Some(a));
    assert!(
        !world.timeline().contains(j),
        "a dead joiner is not inserted"
    );
    assert!(pending_of(&world).is_empty());
    assert_eq!(timeline_of(&world), [(10, a)]);
    assert_eq!(state_hash(&reloaded), state_hash(&world));
}

/// E8 (B33): Q1-Q5 by raw JSON. Control: a valid pending save, and one with a
/// waiting joiner, round-trip and continue with equal hashes.
///
/// **Fails if** any check is missing, mis-ordered, or reads the wrong
/// condition (Q1's three disjuncts are separate cases).
#[test]
fn e8_pending_load_checks() {
    let fixture = join_fixture(Some(LateJoinWire::AfterActive));
    let (world, a, b, j) = pending_world(&fixture);
    let j_json = serde_json::to_value(j).expect("id");
    let a_json = serde_json::to_value(a).expect("id");
    let b_json = serde_json::to_value(b).expect("id");

    // Control: the valid save round-trips and continues with equal hashes.
    let mut copy = try_with_json(&world, |_| {}).expect("the pending save loads");
    assert_eq!(copy, world);
    let mut original = world.clone();
    end(&mut original, b);
    end(&mut copy, b);
    assert_eq!(state_hash(&copy), state_hash(&original));
    assert_eq!(copy.combat().expect("combat").active, Some(j));

    let load = |edit: &dyn Fn(&mut Value)| try_with_json(&world, |save| edit(save)).map(|_| ());
    // Q1: not a first TB round (three disjuncts).
    assert_eq!(
        load(&|save| save["combat"]["round"] = json!(1)),
        Err("combat pending joins outside a first turn-based round".to_owned())
    );
    assert_eq!(
        load(&|save| save["combat"]["active"] = Value::Null),
        Err("combat pending joins outside a first turn-based round".to_owned())
    );
    let capable_fixture = {
        let mut f = capable(8);
        set_hooks(&mut f, None, Some(LateJoinWire::AfterActive), None);
        add_outsider(&mut f, J, 143);
        f
    };
    let mut rt_world = bound(&capable_fixture);
    let rt_j = register(&capable_fixture, &mut rt_world, J);
    begin(&capable_fixture, &mut rt_world);
    join_encounter(&mut rt_world, &entrant(rt_j, 5)).expect("J joins");
    assert_eq!(pending_of(&rt_world), [rt_j]);
    assert_eq!(
        try_with_json(&rt_world, |save| save["combat"]["mode"] =
            json!("real_time"))
        .map(|_| ()),
        Err("combat pending joins outside a first turn-based round".to_owned())
    );
    // Q2: a repeated entity (checked before the entries).
    assert_eq!(
        load(&|save| save["combat"]["pending_joins"] = json!([j_json.clone(), j_json.clone()])),
        Err("combat pending joins repeat an entity".to_owned())
    );
    // Q3: a name that is no combatant (A's twin: a record-holding outsider).
    let mut with_record_holder = world.clone();
    let k = register(&fixture, &mut with_record_holder, K);
    let k_json = serde_json::to_value(k).expect("id");
    assert_eq!(
        try_with_json(&with_record_holder, |save| save["combat"]
            ["pending_joins"] =
            json!([k_json.clone()]))
        .map(|_| ()),
        Err("combat pending join names a non-combatant".to_owned())
    );
    // Q3 is per entry: a good first entry does not hide a bad second one.
    assert_eq!(
        try_with_json(&with_record_holder, |save| save["combat"]
            ["pending_joins"] =
            json!([j_json.clone(), k_json.clone()]))
        .map(|_| ()),
        Err("combat pending join names a non-combatant".to_owned())
    );
    // Q1 beats Q2: a duplicate in a round-1 save is a Q1 fault.
    assert_eq!(
        load(&|save| {
            save["combat"]["round"] = json!(1);
            save["combat"]["pending_joins"] = json!([j_json.clone(), j_json.clone()]);
        }),
        Err("combat pending joins outside a first turn-based round".to_owned())
    );
    // Q2 beats Q3: a repeated non-combatant is a Q2 fault, not a Q3 one.
    assert_eq!(
        try_with_json(&with_record_holder, |save| save["combat"]
            ["pending_joins"] =
            json!([k_json.clone(), k_json.clone()]))
        .map(|_| ()),
        Err("combat pending joins repeat an entity".to_owned())
    );
    // Q4: a pending entry that holds a timeline entry. Its key sorts after
    // the head, so Q5 would also fail: Q4 must win.
    assert_eq!(
        load(&|save| {
            edit_combatant(save, j, |combatant| combatant["initiative"] = json!(12));
            save["timeline"]
                .as_array_mut()
                .expect("timeline")
                .push(json!([12, j_json.clone()]));
        }),
        Err("combat pending join is scheduled".to_owned())
    );
    // Q5: a key that sorts after the head, and one equal to it with a larger
    // id (J is registered after B, so (9, J) > (9, B)).
    assert_eq!(
        load(&|save| edit_combatant(save, j, |combatant| combatant["initiative"] = json!(12))),
        Err("combat pending join sorts after the active head".to_owned())
    );
    assert_eq!(
        load(&|save| edit_combatant(save, j, |combatant| combatant["initiative"] = json!(9))),
        Err("combat pending join sorts after the active head".to_owned())
    );
    // Boundary: one below the head loads.
    let boundary = try_with_json(&world, |save| {
        edit_combatant(save, j, |combatant| combatant["initiative"] = json!(8));
    });
    assert!(boundary.is_ok(), "(8, J) sorts before (9, B)");
    // A pending entry naming A (scheduled) is Q4, naming B (the head) too.
    assert_eq!(
        load(&|save| save["combat"]["pending_joins"] = json!([a_json.clone()])),
        Err("combat pending join is scheduled".to_owned())
    );
    assert_eq!(
        load(&|save| save["combat"]["pending_joins"] = json!([b_json.clone()])),
        Err("combat pending join is scheduled".to_owned())
    );

    // Control: a waiting joiner (no pending entry, no timeline entry)
    // round-trips and continues with equal hashes.
    let mut waiting = bound(&fixture);
    let w = register(&fixture, &mut waiting, J);
    begin(&fixture, &mut waiting);
    let ids = combatant_ids(&waiting);
    end(&mut waiting, ids[1]);
    end(&mut waiting, ids[0]);
    join_encounter(&mut waiting, &entrant(w, 12)).expect("J waits");
    assert!(!waiting.timeline().contains(w));
    let mut reloaded = try_with_json(&waiting, |_| {}).expect("the waiting save loads");
    assert_eq!(reloaded, waiting);
    end(&mut waiting, ids[1]);
    end(&mut reloaded, ids[1]);
    end(&mut waiting, ids[0]);
    end(&mut reloaded, ids[0]);
    assert_eq!(state_hash(&reloaded), state_hash(&waiting));
    assert!(waiting.timeline().contains(w), "it entered at the rollover");
}

/// E9 (B28): an RT join is keyed at `elapsed` with no order. It executes only
/// after `UseAbility` sets an order, and no pool refreshes on entry. Controls:
/// a joiner with an emptied pool still has it empty (its order is refused for
/// cost); a join into an RT encounter with one alive combatant fails
/// `EncounterTerminal`, unchanged.
///
/// **Fails if** the joiner is keyed at 0, inherits an order, is refreshed on
/// entry, or an RT join is refused for a live encounter.
#[test]
fn e9_rt_join() {
    let mut fixture = capable(8);
    add_outsider(&mut fixture, J, 143);
    add_outsider(&mut fixture, K, 144);
    always_success(&mut fixture);
    make_lethal(&mut fixture, 10);
    let mut world = bound(&fixture);
    let j = register(&fixture, &mut world, J);
    fixture
        .with_spec(|spec| {
            start_encounter_with_members(&mut world, spec, EncounterMode::RealTime, &[])
        })
        .expect("starts in real time");
    let a = combatant_ids(&world)[0];
    ticks(&mut world, 3);
    assert_eq!(support::rt::elapsed(&world), 3);
    events_of(&mut world);

    join_encounter(&mut world, &entrant(j, 7)).expect("J joins");
    assert_eq!(
        timeline_of(&world)
            .into_iter()
            .find(|(_, entity)| *entity == j),
        Some((3, j)),
        "keyed at elapsed, not at its entrant key"
    );
    let joiner = world.combatants().get(j).expect("J");
    assert!(joiner.order().is_none());
    assert_eq!(joiner.initiative(), 7, "the entrant key is stored");
    assert_eq!(pool_of(&world, j), 1, "the record's balance, untouched");
    assert!(pending_of(&world).is_empty());
    assert!(events_of(&mut world).is_empty());
    // With no order it never acts, however many ticks pass.
    ticks(&mut world, 2);
    assert_eq!(pool_of(&world, j), 1);
    assert!(world.combatants().get(j).expect("J").order().is_none());
    // After an order it acts on the first tick: the cost is spent.
    perform_action(&mut world, &use_action(j, fixture.ability, a)).expect("J orders an attack");
    assert!(world.combatants().get(j).expect("J").order().is_some());
    assert_eq!(pool_of(&world, j), 1, "admission spends nothing");
    ticks(&mut world, 1);
    assert_eq!(pool_of(&world, j), 0, "it executed on the first tick");
    assert!(world.combatants().get(a).expect("A").dead());

    // No refresh on entry: K's emptied pool is still empty, so its order is
    // refused for cost.
    let mut world = bound(&fixture);
    let k = register(&fixture, &mut world, K);
    let mut world = with_record(&world, k, |record| record["pools"][0]["current"] = json!(0));
    fixture
        .with_spec(|spec| {
            start_encounter_with_members(&mut world, spec, EncounterMode::RealTime, &[])
        })
        .expect("starts in real time");
    let a = combatant_ids(&world)[0];
    join_encounter(&mut world, &entrant(k, 7)).expect("K joins");
    assert_eq!(pool_of(&world, k), 0);
    let refused = perform_action(&mut world, &use_action(k, fixture.ability, a));
    assert!(
        matches!(
            refused,
            Err(CombatError::InsufficientAction { current: 0, .. })
        ),
        "{refused:?}"
    );

    // Control: one alive combatant is terminal.
    let mut world = bound(&fixture);
    let j = register(&fixture, &mut world, J);
    fixture
        .with_spec(|spec| {
            start_encounter_with_members(&mut world, spec, EncounterMode::RealTime, &[])
        })
        .expect("starts in real time");
    let ids = combatant_ids(&world);
    perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1])).expect("orders");
    ticks(&mut world, 1);
    assert!(world.combatants().get(ids[1]).expect("B").dead());
    join_fails(&mut world, entrant(j, 1), CombatError::EncounterTerminal);
}

/// E10 (B23): J1-J11 one by one, each leaving the world unchanged, each with
/// its fixed control joining. J11's fault is a record holding `MAX_TAGS` tags
/// in an encounter that grants one new tag. Pairs: J1 beats J4, J2 beats J3,
/// and each check beats the next.
///
/// **Fails if** any check is missing, reordered, or returns another entity.
#[test]
fn e10_join_checks() {
    let mut fixture = Fixture::standard();
    add_derived(
        &mut fixture,
        "ratio",
        binary(
            "divide",
            stat("alpha"),
            binary("subtract", stat("gamma"), lit(4)),
        ),
    );
    always_success(&mut fixture);
    make_lethal(&mut fixture, 10);
    attach_to_ability(&mut fixture, grant(effect_501(3), &["zzz-new"]));
    add_outsider(&mut fixture, J, 143);
    add_outsider(&mut fixture, K, 144);
    let mut base = bound(&fixture);
    let j = register(&fixture, &mut base, J);
    let k = register(&fixture, &mut base, K);
    let tags: Vec<String> = (0..MAX_TAGS).map(|index| format!("t{index:05}")).collect();

    // J1: no encounter, and J1 beats J4 (a dead id).
    let mut gone_world = base.clone();
    let gone = gone_world.spawn(EntityMeta {});
    assert!(gone_world.despawn(gone));
    join_fails(&mut gone_world, entrant(gone, 12), CombatError::NoEncounter);
    join_fails(&mut base.clone(), entrant(j, 12), CombatError::NoEncounter);

    let mut world = base.clone();
    begin(&fixture, &mut world);
    let ids = combatant_ids(&world);
    let (a, b) = (ids[0], ids[1]);
    // The control: the unedited joiner joins.
    join_encounter(&mut world.clone(), &entrant(j, 12)).expect("the control joins");

    // J4: a dead id.
    let mut with_gone = world.clone();
    let gone = with_gone.spawn(EntityMeta {});
    assert!(with_gone.despawn(gone));
    join_fails(
        &mut with_gone,
        entrant(gone, 12),
        CombatError::AbsentActor { actor: gone },
    );
    // J5: an entity that holds a combatant; J5 beats J6 (A has no record).
    join_fails(
        &mut world,
        entrant(a, 12),
        CombatError::AlreadyParticipant { entity: a },
    );
    // J6: a bare live entity.
    let mut bare_world = world.clone();
    let bare = bare_world.spawn(EntityMeta {});
    join_fails(
        &mut bare_world,
        entrant(bare, 12),
        CombatError::NotCreature { entity: bare },
    );
    // J7: a dead record; J7 beats J8 (the dead record has another ruleset).
    let dead = with_record(&world, j, |record| {
        record["health"] = json!(0);
        record["ruleset"] = json!(uid(999));
    });
    let mut dead_world = dead.clone();
    join_fails(
        &mut dead_world,
        entrant(j, 12),
        CombatError::DeadActor { actor: j },
    );
    // J8: another ruleset, and separately an attribute order swapped; J8
    // beats J9 (placement of a current combatant) and J10.
    let mut other = with_record(&world, j, |record| {
        record["ruleset"] = json!(uid(999));
        record["placement"] = json!(uid(111));
        record["attributes"][2][1] = json!(4);
    });
    join_fails(
        &mut other,
        entrant(j, 12),
        CombatError::RulesetMismatch { entity: j },
    );
    let mut swapped = with_record(&world, j, |record| {
        let first = record["attributes"][0].clone();
        record["attributes"][0] = record["attributes"][1].clone();
        record["attributes"][1] = first;
    });
    join_fails(
        &mut swapped,
        entrant(j, 12),
        CombatError::RulesetMismatch { entity: j },
    );
    // J9: the placement of a current combatant, alive; J9 beats J10.
    let mut taken = with_record(&world, j, |record| {
        record["placement"] = json!(uid(111));
        record["attributes"][2][1] = json!(4);
    });
    join_fails(
        &mut taken,
        entrant(j, 12),
        CombatError::PlacementInUse { placement: id(111) },
    );
    // J9 counts a dead combatant too: B's placement after A kills it, in an
    // encounter that a third combatant keeps alive.
    {
        let mut trio = Fixture::standard();
        always_success(&mut trio);
        make_lethal(&mut trio, 10);
        support::rt::add_clone(&mut trio, 113, 123, 8);
        add_outsider(&mut trio, J, 143);
        let mut world = bound(&trio);
        let j = register(&trio, &mut world, J);
        begin(&trio, &mut world);
        let ids = combatant_ids(&world);
        let (a, b, c) = (ids[0], ids[1], ids[2]);
        end(&mut world, c);
        end(&mut world, b);
        perform_action(&mut world, &use_action(a, trio.ability, b)).expect("A attacks B");
        assert!(world.combatants().get(b).expect("B").dead());
        assert_eq!(world.combat().expect("combat").active, Some(c));
        let mut dead_holder =
            with_record(&world, j, |record| record["placement"] = json!(uid(112)));
        join_fails(
            &mut dead_holder,
            entrant(j, 12),
            CombatError::PlacementInUse { placement: id(112) },
        );
        // Control: the unedited record joins the same encounter.
        join_encounter(&mut world, &entrant(j, 12)).expect("J joins");
    }
    // Control: after the placement_a combatant leaves, its own record can
    // join back (J9 clears).
    {
        let mut leaver = world.clone();
        crpg_sim::leave_encounter(&mut leaver, a).expect("A leaves");
        assert_eq!(
            leaver.creatures().get(a).expect("record").placement(),
            id(111)
        );
        join_encounter(&mut leaver, &entrant(a, 12)).expect("A joins back");
        assert!(leaver.combatants().get(a).expect("A").entered());
        assert!(leaver.creatures().get(a).is_none());
    }
    // J10: a derived stat fails at gamma 4; J10 beats J11.
    let mut derived = with_record(&world, j, |record| {
        record["attributes"][2][1] = json!(4);
        record["tags"] = json!(tags);
    });
    join_fails(
        &mut derived,
        entrant(j, 12),
        CombatError::InvalidStatValue {
            stat: "ratio".to_owned(),
        },
    );
    // J11: MAX_TAGS tags plus the granted one.
    let mut crowded = with_record(&world, j, |record| record["tags"] = json!(tags));
    join_fails(&mut crowded, entrant(j, 12), CombatError::TooManyTags);
    // Control: one tag fewer fits next to the single grant.
    let fits: Vec<String> = tags[..MAX_TAGS - 1].to_vec();
    let mut fitting = with_record(&world, j, |record| record["tags"] = json!(fits));
    join_encounter(&mut fitting, &entrant(j, 12)).expect("MAX_TAGS - 1 tags plus a grant fit");

    // J2: a terminal encounter, and J2 beats J3 (a full one).
    let mut terminal = world.clone();
    end(&mut terminal, b);
    perform_action(&mut terminal, &use_action(a, fixture.ability, b)).expect("A attacks B");
    assert_eq!(terminal.combat().expect("combat").active, None);
    join_fails(
        &mut terminal,
        entrant(k, 12),
        CombatError::EncounterTerminal,
    );

    // J3: a full encounter; J2 beats J3 on the same world made terminal.
    let mut crowded_fixture = Fixture::standard();
    for index in 0..1022u128 {
        add_outsider(&mut crowded_fixture, 10_000 + index, 20_000 + index);
    }
    add_outsider(&mut crowded_fixture, J, 143);
    let mut crowd = bound(&crowded_fixture);
    let mut members = Vec::new();
    for index in 0..1022u128 {
        members.push(entrant(
            register(&crowded_fixture, &mut crowd, 10_000 + index),
            1,
        ));
    }
    let jc = register(&crowded_fixture, &mut crowd, J);
    crowded_fixture
        .with_spec(|spec| {
            start_encounter_with_members(&mut crowd, spec, EncounterMode::TurnBased, &members)
        })
        .expect("a full encounter starts");
    assert_eq!(crowd.combatants().len(), 1024);
    join_fails(
        &mut crowd,
        entrant(jc, 12),
        CombatError::TooManyParticipants,
    );
    let mut crowd_terminal = try_with_json(&crowd, |save| save["combat"]["active"] = Value::Null)
        .expect("a terminal save loads");
    join_fails(
        &mut crowd_terminal,
        entrant(jc, 12),
        CombatError::EncounterTerminal,
    );
    // Control: one seat fewer admits the joiner.
    let mut roomy = bound(&crowded_fixture);
    let mut members = Vec::new();
    for index in 0..1021u128 {
        members.push(entrant(
            register(&crowded_fixture, &mut roomy, 10_000 + index),
            1,
        ));
    }
    let jr = register(&crowded_fixture, &mut roomy, J);
    crowded_fixture
        .with_spec(|spec| {
            start_encounter_with_members(&mut roomy, spec, EncounterMode::TurnBased, &members)
        })
        .expect("a roomy encounter starts");
    assert_eq!(roomy.combatants().len(), 1023);
    join_encounter(&mut roomy, &entrant(jr, 12)).expect("the last seat is free");
    assert_eq!(roomy.combatants().len(), 1024);
}

/// E11 (B25): under an initiative rule, a join makes exactly one
/// `combat.roll` draw, equal to the oracle, and its key is roll plus the
/// record's bonus plus the entrant's base, in TB (inserted at the key) and in
/// RT (the key is stored, the entry is at `elapsed`). Controls: with no rule
/// the key is the base and the RNG is unchanged.
///
/// **Fails if** the joiner draws nothing or twice, reads the prefab or the
/// entrant for the bonus, drops the base, or interns the roll tag.
#[test]
fn e11_join_initiative() {
    let mut fixture = join_fixture(None);
    set_initiative_rule(&mut fixture, "1d6", &["alpha"]);
    let mut world = bound(&fixture);
    let j = register(&fixture, &mut world, J);
    // J has alpha 3 (the prefab says 8), so the bonus reads the record.
    let world = with_record(&world, j, |record| record["attributes"][0][1] = json!(3));
    let mut world = world;
    begin(&fixture, &mut world);
    let mut expected = oracle(5);
    draw(&mut expected, "1d6");
    draw(&mut expected, "1d6");
    assert_eq!(*world.rng_mut(), expected, "the start made two draws");
    let ids = combatant_ids(&world);
    let interners = world.interners().clone();
    join_encounter(&mut world, &entrant(j, 100)).expect("J joins");
    let roll = draw(&mut expected, "1d6").total;
    assert_eq!(*world.rng_mut(), expected, "exactly one more draw");
    let key = roll + 3 + 100;
    assert_eq!(world.combatants().get(j).expect("J").initiative(), key);
    assert_eq!(
        timeline_of(&world).last().copied(),
        Some((key, j)),
        "inserted at the rolled key"
    );
    assert_eq!(world.combat().expect("combat").active, Some(ids[1]));
    assert_eq!(*world.interners(), interners);

    // Control: no rule, so the key is the base and nothing is drawn.
    let plain = join_fixture(None);
    let mut world = bound(&plain);
    let j = register(&plain, &mut world, J);
    begin(&plain, &mut world);
    let before = world.rng_mut().clone();
    join_encounter(&mut world, &entrant(j, 12)).expect("J joins");
    assert_eq!(world.combatants().get(j).expect("J").initiative(), 12);
    assert_eq!(*world.rng_mut(), before);

    // RT: the rolled key is stored, and the entry is at `elapsed`.
    let mut rt = capable(8);
    set_initiative_rule(&mut rt, "1d6", &["alpha"]);
    add_outsider(&mut rt, J, 143);
    let mut world = bound(&rt);
    let j = register(&rt, &mut world, J);
    let mut world = with_record(&world, j, |record| record["attributes"][0][1] = json!(3));
    rt.with_spec(|spec| {
        start_encounter_with_members(&mut world, spec, EncounterMode::RealTime, &[])
    })
    .expect("starts in real time");
    ticks(&mut world, 2);
    let mut expected = oracle(5);
    draw(&mut expected, "1d6");
    draw(&mut expected, "1d6");
    let roll = draw(&mut expected, "1d6").total;
    join_encounter(&mut world, &entrant(j, 4)).expect("J joins");
    assert_eq!(*world.rng_mut(), expected);
    assert_eq!(
        world.combatants().get(j).expect("J").initiative(),
        roll + 3 + 4
    );
    assert!(timeline_of(&world).contains(&(2, j)));
}

/// B22's last sentence: `join_after_active` is published at every start,
/// including one with `members == &[]`, and only when the ruleset says so.
///
/// **Fails if** the flag is set only for starts with members, ignores the
/// ruleset, or is written at its default.
#[test]
fn e7_join_after_active_published_at_every_start() {
    for (late, expected) in [
        (None, false),
        (Some(LateJoinWire::NextRound), false),
        (Some(LateJoinWire::AfterActive), true),
    ] {
        let fixture = join_fixture(late);
        let mut world = bound(&fixture);
        begin(&fixture, &mut world);
        let combat = world.combat().expect("combat");
        assert_eq!(combat.lifecycle.join_after_active, expected, "{late:?}");
        assert!(combat.pending_joins.is_empty());
        let combat_json = value_of(&world)["combat"].clone();
        assert_eq!(
            combat_json.get("lifecycle").is_some(),
            expected,
            "{late:?}: the key is written only off its default"
        );
        if expected {
            assert_eq!(combat_json["lifecycle"], json!({"join_after_active": true}));
        }
        // The plain start function sets it too.
        let mut plain = bound(&fixture);
        fixture
            .with_spec(|spec| crpg_sim::start_encounter(&mut plain, spec))
            .expect("starts");
        assert_eq!(
            plain.combat().expect("combat").lifecycle.join_after_active,
            expected
        );
    }

    // Every key of a non-default lifecycle, in contract order.
    let mut fixture = join_fixture(Some(LateJoinWire::AfterActive));
    fixture.encounter.respawns = Some(true);
    fixture.ruleset.effects_on_exit = Some(crpg_data::EffectsOnExitWire::Strip);
    let mut world = bound(&fixture);
    begin(&fixture, &mut world);
    let text = serde_json::to_string(&world).expect("serializes");
    assert!(text.contains(
        "\"lifecycle\":{\"respawns\":true,\"effects_on_exit\":\"strip\",\"join_after_active\":true}"
    ));
}

/// E8 (B33, Q5): a dead pending joiner is still checked against the head.
///
/// **Fails if** Q5 skips a dead pending joiner.
#[test]
fn e8_dead_pending_join_q5() {
    let mut lethal = join_fixture(Some(LateJoinWire::AfterActive));
    always_success(&mut lethal);
    make_lethal(&mut lethal, 10);
    edit_ability(&mut lethal, |ability| ability.ends_turn = false);
    let (mut world, _, b, j) = pending_world(&lethal);
    perform_action(&mut world, &use_action(b, lethal.ability, j)).expect("B attacks J");
    assert!(world.combatants().get(j).expect("J").dead());
    assert_eq!(pending_of(&world), [j]);
    // Control: the dead pending joiner at its own key loads.
    try_with_json(&world, |_| {}).expect("the dead pending joiner loads");
    for key in [12, 9] {
        let result = try_with_json(&world, |save| {
            edit_combatant(save, j, |combatant| combatant["initiative"] = json!(key));
        })
        .map(|_| ());
        assert_eq!(
            result,
            Err("combat pending join sorts after the active head".to_owned()),
            "key {key}"
        );
    }
}

/// D45a (B33): a terminal transition empties `pending_joins`, so the terminal
/// save keeps B33's round trip. B kills pending J, then kills A, the last
/// other combatant: the kill ends the encounter without `advance_tracked`.
///
/// **Fails if** the `UseAbility` terminal branch leaves `pending_joins` alone.
#[test]
fn e8_terminal_transition_empties_pending_joins() {
    let mut fixture = join_fixture(Some(LateJoinWire::AfterActive));
    fixture.ruleset.pools[0].max = 5;
    always_success(&mut fixture);
    make_lethal(&mut fixture, 10);
    edit_ability(&mut fixture, |ability| ability.ends_turn = false);
    let (mut world, a, b, j) = pending_world(&fixture);
    perform_action(&mut world, &use_action(b, fixture.ability, j)).expect("B attacks J");
    assert_eq!(pending_of(&world), [j], "a dead joiner is still pending");
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B attacks A");
    assert_eq!(world.combat().expect("combat").active, None, "terminal");
    assert!(pending_of(&world).is_empty(), "the list is emptied");
    assert!(value_of(&world)["combat"].get("pending_joins").is_none());
    let reloaded = try_with_json(&world, |_| {}).expect("the terminal save reloads");
    assert_eq!(reloaded, world);
    assert_eq!(state_hash(&reloaded), state_hash(&world));
    // The waiting creature is still a combatant and is released like the rest.
    assert!(world.combatants().get(j).is_some());
    crpg_sim::end_encounter(&mut world).expect("release");
    assert!(world.creatures().get(j).is_some());
}
