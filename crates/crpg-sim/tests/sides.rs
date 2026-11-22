//! T157a: sides (ADR-0029 Amendment 1 S1, S2; contract S-A1 to S-A6; tests
//! SA1 to SA11).
//!
//! The standard three-combatant fixture is used throughout: C (initiative 8)
//! acts before B (9) before A (10). "Factions X" names the creatures whose
//! authored faction is set before the start; "no factions" leaves every
//! creature without one, so every combatant is a side of one. Every expected
//! value is worked out by hand from the contract, never read from a run.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Ulid};
use crpg_sim::{
    end_encounter, fight_is_live, join_encounter, leave_encounter, opposed, perform_action,
    side_of, start_encounter_in_mode, start_encounter_with_members, state_hash, tick,
    validate_action, CombatAction, CombatError, EncounterMode, EntityMeta, Entrant, HistoryEvent,
    HistoryWorld, Side, World,
};
use support::creature::{add_outsider, enlist};
use support::rt::{make_lethal, use_action};
use support::surface::always_success;
use support::Fixture;

fn f1() -> Ulid {
    Ulid::from_u128(131)
}

fn f2() -> Ulid {
    Ulid::from_u128(132)
}

fn pl(value: u128) -> Ulid {
    Ulid::from_u128(value)
}

/// `Fixture::standard()` with every roll a success and lethal damage, plus C
/// (placement 113, creature 123, initiative 8).
fn three() -> Fixture {
    let mut fx = Fixture::standard();
    always_success(&mut fx);
    make_lethal(&mut fx, 99);
    add_outsider(&mut fx, 113, 123);
    enlist(&mut fx, 113, 8);
    fx
}

/// Sets the authored faction of creature `creature` (121 A, 122 B, 123 C,
/// 124 the fourth).
fn faction(fx: &mut Fixture, creature: u128, which: Ulid) {
    fx.creatures
        .get_mut(&pl(creature))
        .expect("the creature exists")
        .faction = Some(which);
}

/// Factions: A and C in F1, B in F2.
fn ac_vs_b(fx: &mut Fixture) {
    faction(fx, 121, f1());
    faction(fx, 123, f1());
    faction(fx, 122, f2());
}

/// Factions: A and B in F1, C in F2.
fn ab_vs_c(fx: &mut Fixture) {
    faction(fx, 121, f1());
    faction(fx, 122, f1());
    faction(fx, 123, f2());
}

fn rt(fx: &mut Fixture) {
    fx.ruleset.round_ticks = Some(4);
}

fn dead(world: &World, entity: EntityId) -> bool {
    world.combatants().get(entity).expect("combatant").dead()
}

fn active(world: &World) -> Option<EntityId> {
    world.combat().expect("encounter").active
}

fn round(world: &World) -> u32 {
    world.combat().expect("encounter").round
}

fn elapsed(world: &World) -> u64 {
    world
        .combat()
        .expect("encounter")
        .clock
        .expect("clock")
        .elapsed
}

/// One accepted turn-based strike.
fn strike(world: &mut World, fx: &Fixture, actor: EntityId, target: EntityId) {
    perform_action(world, &use_action(actor, fx.ability, target)).expect("the strike is accepted");
}

/// A real-time standing order, then ticks until `target` is dead.
fn rt_kill(world: &mut World, fx: &Fixture, actor: EntityId, target: EntityId) {
    let stored = perform_action(world, &use_action(actor, fx.ability, target))
        .expect("the order is accepted");
    assert!(stored.is_none(), "a real-time use stores an order");
    for _ in 0..20 {
        if dead(world, target) {
            return;
        }
        tick(world);
    }
    panic!("the target did not die within 20 ticks");
}

fn since(history: &HistoryWorld, after: u64) -> Vec<HistoryEvent> {
    history
        .read_after(after, 256)
        .expect("a page fits")
        .into_iter()
        .map(|envelope| envelope.payload)
        .collect()
}

fn ended(events: &[HistoryEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, HistoryEvent::EncounterEnded { .. }))
        .count()
}

#[test]
fn sa1_no_faction_changes_nothing() {
    let fx = three();
    let (mut w, ids) = fx.start(1);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    let json = serde_json::to_string(&w).expect("the world serializes");
    assert!(
        !json.contains("\"faction\""),
        "an absent faction is not written"
    );
    strike(&mut w, &fx, c, b);
    assert!(fight_is_live(&w));
    assert_eq!(active(&w), Some(a));
    let json = serde_json::to_string(&w).expect("the world serializes");
    assert!(!json.contains("\"faction\""));
    let back: World = serde_json::from_str(&json).expect("the world loads");
    assert_eq!(back, w);
    assert_eq!(state_hash(&back), state_hash(&w));
}

#[test]
fn sa2_one_side_left_ends_a_turn_based_action() {
    let mut fx = three();
    ac_vs_b(&mut fx);
    let (mut w, ids) = fx.start(1);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    assert!(fight_is_live(&w));
    assert_eq!(active(&w), Some(c));
    strike(&mut w, &fx, c, b);
    assert!(dead(&w, b));
    assert_eq!(active(&w), None);
    assert!(!fight_is_live(&w));
    assert!(!dead(&w, a));
    assert!(!dead(&w, c));
}

#[test]
fn sa2b_the_action_end_counts_sides() {
    let mut fx = three();
    fx.abilities
        .get_mut(&fx.ability)
        .expect("the ability")
        .ends_turn = false;
    ac_vs_b(&mut fx);
    let (mut w, ids) = fx.start(1);
    let (b, c) = (ids[1], ids[2]);
    strike(&mut w, &fx, c, b);
    assert_eq!(active(&w), None);
    assert!(!fight_is_live(&w));

    // Control, no factions: A and C stay opposed, and C keeps its turn.
    let mut fx = three();
    fx.abilities
        .get_mut(&fx.ability)
        .expect("the ability")
        .ends_turn = false;
    let (mut w, ids) = fx.start(1);
    let (b, c) = (ids[1], ids[2]);
    strike(&mut w, &fx, c, b);
    assert_eq!(active(&w), Some(c));
    assert!(fight_is_live(&w));
}

#[test]
fn sa3_one_side_left_ends_a_real_time_fight() {
    let mut fx = three();
    rt(&mut fx);
    ac_vs_b(&mut fx);
    let (mut h, ids) = fx
        .start_history(1, EncounterMode::RealTime)
        .expect("the real-time start");
    let (b, c) = (ids[1], ids[2]);
    h.perform_action(&use_action(c, fx.ability, b))
        .expect("the order is accepted");
    let mut ends = 0;
    for _ in 0..20 {
        let before = h.last_sequence();
        h.tick().expect("the tick journals");
        ends += ended(&since(&h, before));
        if dead(h.world(), b) {
            break;
        }
    }
    assert!(dead(h.world(), b));
    assert_eq!(ends, 1, "the killing tick journals one EncounterEnded");
    assert!(!fight_is_live(h.world()));
    let clock = elapsed(h.world());
    for _ in 0..2 {
        h.tick().expect("a tick after the end");
    }
    assert_eq!(
        elapsed(h.world()),
        clock,
        "the clock stopped with one side left"
    );

    // Control, no factions: the fight goes on and the clock runs.
    let mut fx = three();
    rt(&mut fx);
    let (mut h, ids) = fx
        .start_history(1, EncounterMode::RealTime)
        .expect("the real-time start");
    let (b, c) = (ids[1], ids[2]);
    h.perform_action(&use_action(c, fx.ability, b))
        .expect("the order is accepted");
    let mut ends = 0;
    for _ in 0..20 {
        let before = h.last_sequence();
        h.tick().expect("the tick journals");
        ends += ended(&since(&h, before));
        if dead(h.world(), b) {
            break;
        }
    }
    assert!(dead(h.world(), b));
    assert_eq!(ends, 0);
    assert!(fight_is_live(h.world()));
    let clock = elapsed(h.world());
    for _ in 0..2 {
        h.tick().expect("a tick");
    }
    assert_eq!(elapsed(h.world()), clock + 2);
}

#[test]
fn sa4_sides_of_combatants_and_records() {
    let mut fx = three();
    ac_vs_b(&mut fx);
    let (mut w, ids) = fx.start(1);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    assert_eq!(side_of(&w, a), Some(Side::Faction(f1())));
    assert_eq!(side_of(&w, b), Some(Side::Faction(f2())));
    assert_eq!(w.combatants().get(a).expect("A").faction(), Some(f1()));
    assert!(opposed(&w, a, b));
    assert!(!opposed(&w, a, c));
    assert!(!opposed(&w, a, a));
    let bare = w.spawn(EntityMeta {});
    assert_eq!(side_of(&w, bare), None);
    assert!(!opposed(&w, a, bare));
    assert!(!opposed(&w, bare, a));

    // No factions: every combatant is a side of one.
    let fx0 = three();
    let (w0, ids0) = fx0.start(1);
    assert_eq!(side_of(&w0, ids0[0]), Some(Side::Alone(ids0[0])));
    assert!(opposed(&w0, ids0[0], ids0[2]));

    // (a) A record takes its creature document's faction.
    let mut fx2 = three();
    add_outsider(&mut fx2, 114, 124);
    faction(&mut fx2, 124, f1());
    let mut w2 = World::new(1);
    let r = fx2
        .register(&mut w2, pl(114))
        .expect("the record registers");
    assert_eq!(w2.creatures().get(r).expect("record").faction(), Some(f1()));
    assert_eq!(side_of(&w2, r), Some(Side::Faction(f1())));
    let text = serde_json::to_string(w2.creatures().get(r).expect("record")).expect("record");
    assert!(text.contains("\"faction\""));
    let mut fx3 = three();
    add_outsider(&mut fx3, 114, 124);
    let mut w3 = World::new(1);
    let r3 = fx3
        .register(&mut w3, pl(114))
        .expect("the record registers");
    assert_eq!(side_of(&w3, r3), Some(Side::Alone(r3)));
    let text = serde_json::to_string(w3.creatures().get(r3).expect("record")).expect("record");
    assert!(!text.contains("\"faction\""));

    // (b) Enter: a member copies its record's faction.
    ac_vs_b(&mut fx2);
    let mut w2 = World::new(1);
    let r = fx2
        .register(&mut w2, pl(114))
        .expect("the record registers");
    fx2.with_spec(|spec| {
        start_encounter_with_members(
            &mut w2,
            spec,
            EncounterMode::TurnBased,
            &[Entrant {
                entity: r,
                initiative: 5,
            }],
        )
    })
    .expect("the start with a member");
    assert_eq!(w2.combatants().get(r).expect("R").faction(), Some(f1()));
    assert_eq!(side_of(&w2, r), Some(Side::Faction(f1())));

    // (c) Leave: the record copies the combatant's faction back.
    leave_encounter(&mut w2, r).expect("the leave");
    assert_eq!(w2.creatures().get(r).expect("record").faction(), Some(f1()));

    // (d) End: every survivor's record carries its faction.
    end_encounter(&mut w).expect("the release");
    assert_eq!(w.creatures().get(a).expect("A").faction(), Some(f1()));
    assert_eq!(w.creatures().get(b).expect("B").faction(), Some(f2()));
    assert_eq!(w.creatures().get(c).expect("C").faction(), Some(f1()));
}

#[test]
fn sa5_two_on_each_side_fight_on() {
    let mut fx = three();
    add_outsider(&mut fx, 114, 124);
    enlist(&mut fx, 114, 7);
    faction(&mut fx, 121, f1());
    faction(&mut fx, 123, f1());
    faction(&mut fx, 122, f2());
    faction(&mut fx, 124, f2());
    let (mut w, ids) = fx.start(1);
    let (a, b, c, d) = (ids[0], ids[1], ids[2], ids[3]);
    assert_eq!(active(&w), Some(d));
    strike(&mut w, &fx, d, a);
    assert!(dead(&w, a));
    assert!(fight_is_live(&w));
    strike(&mut w, &fx, c, d);
    assert!(dead(&w, d));
    assert!(fight_is_live(&w));
    assert_eq!(active(&w), Some(b));
    perform_action(&mut w, &CombatAction::EndTurn { actor: b }).expect("B passes");
    assert!(fight_is_live(&w));
    assert_eq!(active(&w), Some(c));
    strike(&mut w, &fx, c, b);
    assert!(!fight_is_live(&w));
    assert_eq!(active(&w), None);
}

#[test]
fn sa6_faction_round_trips_and_hashes() {
    let mut fx = three();
    ac_vs_b(&mut fx);
    let (w, _) = fx.start(1);
    let json = serde_json::to_string(&w).expect("the world serializes");
    assert_eq!(json.matches("\"faction\"").count(), 3);
    let back: World = serde_json::from_str(&json).expect("the world loads");
    assert_eq!(back, w);
    assert_eq!(state_hash(&back), state_hash(&w));
    let (plain, _) = three().start(1);
    assert_ne!(state_hash(&w), state_hash(&plain));
    let json = serde_json::to_string(&plain).expect("the world serializes");
    assert_eq!(json.matches("\"faction\"").count(), 0);
}

#[test]
fn sa7_rollover_gates_count_sides() {
    // (a) The head leaves with the timeline not empty.
    let mut fx = three();
    ab_vs_c(&mut fx);
    let (mut w, ids) = fx.start(1);
    let c = ids[2];
    assert_eq!(active(&w), Some(c));
    leave_encounter(&mut w, c).expect("the leave");
    assert_eq!(active(&w), None);
    assert!(!fight_is_live(&w));
    let fx0 = three();
    let (mut w0, ids0) = fx0.start(1);
    leave_encounter(&mut w0, ids0[2]).expect("the leave");
    assert_eq!(active(&w0), Some(ids0[1]));
    assert!(fight_is_live(&w0));

    // (b) The last head leaves with the timeline empty.
    let mut fx = three();
    faction(&mut fx, 122, f1());
    faction(&mut fx, 123, f1());
    faction(&mut fx, 121, f2());
    let (mut w, ids) = fx.start(1);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    perform_action(&mut w, &CombatAction::EndTurn { actor: c }).expect("C passes");
    perform_action(&mut w, &CombatAction::EndTurn { actor: b }).expect("B passes");
    assert_eq!(active(&w), Some(a));
    assert_eq!(round(&w), 0);
    leave_encounter(&mut w, a).expect("the leave");
    assert_eq!(active(&w), None);
    assert_eq!(round(&w), 0);
    assert!(!fight_is_live(&w));
    let fx0 = three();
    let (mut w0, ids0) = fx0.start(1);
    let (a0, b0, c0) = (ids0[0], ids0[1], ids0[2]);
    perform_action(&mut w0, &CombatAction::EndTurn { actor: c0 }).expect("C passes");
    perform_action(&mut w0, &CombatAction::EndTurn { actor: b0 }).expect("B passes");
    assert_eq!(active(&w0), Some(a0));
    leave_encounter(&mut w0, a0).expect("the leave");
    assert_eq!(active(&w0), Some(c0));
    assert_eq!(round(&w0), 1);
}

#[test]
fn sa8_real_time_leave_counts_sides() {
    let mut fx = three();
    rt(&mut fx);
    ab_vs_c(&mut fx);
    let (mut h, ids) = fx
        .start_history(1, EncounterMode::RealTime)
        .expect("the real-time start");
    let before = h.last_sequence();
    h.leave_encounter(ids[2]).expect("the leave");
    assert_eq!(ended(&since(&h, before)), 1);
    assert!(!fight_is_live(h.world()));

    let mut fx = three();
    rt(&mut fx);
    let (mut h, ids) = fx
        .start_history(1, EncounterMode::RealTime)
        .expect("the real-time start");
    let before = h.last_sequence();
    h.leave_encounter(ids[2]).expect("the leave");
    assert_eq!(ended(&since(&h, before)), 0);
    assert!(fight_is_live(h.world()));
}

#[test]
fn sa8b_a_finished_real_time_fight_ends_once() {
    // A kills C, so A and B (one side) are all that is left; B leaving then
    // does not end the encounter a second time.
    let mut fx = three();
    rt(&mut fx);
    ab_vs_c(&mut fx);
    let (mut h, ids) = fx
        .start_history(1, EncounterMode::RealTime)
        .expect("the real-time start");
    let (b, c) = (ids[1], ids[2]);
    h.perform_action(&use_action(ids[0], fx.ability, c))
        .expect("A's order");
    h.tick().expect("the tick");
    assert!(dead(h.world(), c));
    assert!(!fight_is_live(h.world()));
    let before = h.last_sequence();
    h.leave_encounter(b).expect("the leave");
    assert_eq!(ended(&since(&h, before)), 0);
}

/// THREE in real time with a record R (placement 114, creature 124, F1 when
/// `factions`) registered before the start.
fn join_world(factions: bool) -> (World, Fixture, Vec<EntityId>, EntityId) {
    let mut fx = three();
    rt(&mut fx);
    add_outsider(&mut fx, 114, 124);
    if factions {
        ab_vs_c(&mut fx);
        faction(&mut fx, 124, f1());
    }
    let mut w = World::new(1);
    let r = fx.register(&mut w, pl(114)).expect("the record registers");
    fx.with_spec(|spec| start_encounter_in_mode(&mut w, spec, EncounterMode::RealTime))
        .expect("the real-time start");
    let ids: Vec<EntityId> = w.combatants().iter().map(|(id, _)| id).collect();
    (w, fx, ids, r)
}

#[test]
fn sa9_join_refused_once_one_side_is_left() {
    let entrant = |entity| Entrant {
        entity,
        initiative: 5,
    };
    let (mut w, _, _, r) = join_world(true);
    assert_eq!(join_encounter(&mut w, &entrant(r)), Ok(()));
    assert_eq!(w.combatants().get(r).expect("R").faction(), Some(f1()));

    let (mut w, fx, ids, r) = join_world(true);
    rt_kill(&mut w, &fx, ids[0], ids[2]);
    assert_eq!(
        join_encounter(&mut w, &entrant(r)),
        Err(CombatError::EncounterTerminal)
    );

    // Control, no factions: A and B are two sides, so the join passes.
    let (mut w, fx, ids, r) = join_world(false);
    rt_kill(&mut w, &fx, ids[0], ids[2]);
    assert_eq!(join_encounter(&mut w, &entrant(r)), Ok(()));
}

#[test]
fn sa10_real_time_validation_counts_sides() {
    let mut fx = three();
    rt(&mut fx);
    ab_vs_c(&mut fx);
    let (mut w, ids) = fx.start_rt(1, EncounterMode::RealTime);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    rt_kill(&mut w, &fx, a, c);
    let refused = Err(CombatError::OutOfTurn {
        actor: a,
        active: None,
    });
    assert_eq!(
        validate_action(&w, &CombatAction::Stop { actor: a }),
        refused
    );
    assert_eq!(validate_action(&w, &use_action(a, fx.ability, b)), refused);

    // Control, no factions: A and B are opposed and the fight is live.
    let mut fx = three();
    rt(&mut fx);
    let (mut w, ids) = fx.start_rt(1, EncounterMode::RealTime);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    rt_kill(&mut w, &fx, a, c);
    assert_eq!(
        validate_action(&w, &CombatAction::Stop { actor: a }),
        Ok(())
    );
    assert!(matches!(
        validate_action(&w, &use_action(a, fx.ability, b)),
        Err(CombatError::InsufficientAction { .. })
    ));
    while !elapsed(&w).is_multiple_of(4) {
        tick(&mut w);
    }
    assert_eq!(validate_action(&w, &use_action(a, fx.ability, b)), Ok(()));
}

#[test]
fn sa11_stage_three_stops_when_one_side_is_left() {
    let mut fx = three();
    rt(&mut fx);
    ab_vs_c(&mut fx);
    let (mut h, ids) = fx
        .start_history(1, EncounterMode::RealTime)
        .expect("the real-time start");
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    // Every key is 0, at or below elapsed + 1.
    h.perform_action(&use_action(a, fx.ability, c))
        .expect("A's order");
    h.perform_action(&use_action(b, fx.ability, c))
        .expect("B's order");
    let before = h.last_sequence();
    h.tick().expect("the tick");
    assert!(dead(h.world(), c));
    assert!(h.world().combatants().get(b).expect("B").order().is_some());
    let events = since(&h, before);
    assert!(!events
        .iter()
        .any(|event| matches!(event, HistoryEvent::OrderEnded { actor } if *actor == b)));

    // Control, no factions: the pass goes on and B's order fails DeadTarget.
    let mut fx = three();
    rt(&mut fx);
    let (mut h, ids) = fx
        .start_history(1, EncounterMode::RealTime)
        .expect("the real-time start");
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    h.perform_action(&use_action(a, fx.ability, c))
        .expect("A's order");
    h.perform_action(&use_action(b, fx.ability, c))
        .expect("B's order");
    let before = h.last_sequence();
    h.tick().expect("the tick");
    assert!(dead(h.world(), c));
    assert!(h.world().combatants().get(b).expect("B").order().is_none());
    let events = since(&h, before);
    assert!(events
        .iter()
        .any(|event| matches!(event, HistoryEvent::OrderEnded { actor } if *actor == b)));
}
