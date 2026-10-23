//! Mode switches at round boundaries (T138a, ADR-0029 T10): R11.
//!
//! The pools are `OnTurnStart` with a maximum of one, so a refill is visible
//! only when a `TurnStart` reaches that combatant. That is what tells the TB
//! round start (head only) from the RT round start (every alive combatant).

#[allow(dead_code)]
mod support;

use crpg_sim::{perform_action, request_mode, tick, CombatAction, EncounterMode};
use support::rt::{
    capable, capable_trio, edit_ability, elapsed, make_lethal, pool_of, ticks, timeline_of,
    use_action,
};

/// R11 (B25): RT to TB at the boundary clears orders, restores initiative,
/// seats the head and gives exactly one `TurnStart`.
#[test]
fn r11_rt_to_tb_at_the_boundary() {
    let mut fixture = capable(4);
    edit_ability(&mut fixture, |ability| {
        ability.repeats_in_real_time = Some(true);
    });
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("A's order");
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B's order");
    tick(&mut world);
    assert_eq!(
        (pool_of(&world, a), pool_of(&world, b)),
        (0, 0),
        "both acted"
    );
    assert!(
        world.combatants().get(a).expect("A").order().is_some(),
        "repeating"
    );
    request_mode(&mut world, EncounterMode::TurnBased).expect("request");
    ticks(&mut world, 2);
    assert_eq!(
        world.combat().expect("encounter").mode,
        EncounterMode::RealTime
    );

    tick(&mut world); // tick 4: the boundary
    let combat = world.combat().expect("encounter");
    assert_eq!(combat.mode, EncounterMode::TurnBased);
    assert_eq!(combat.pending_mode, None);
    assert_eq!(combat.round, 1);
    assert_eq!(combat.active, Some(b), "B (initiative 9) heads the round");
    assert_eq!(elapsed(&world), 4);
    assert_eq!(
        timeline_of(&world),
        vec![(9, b), (10, a)],
        "stored initiative replaces the RT keys"
    );
    for id in [a, b] {
        assert!(world.combatants().get(id).expect("c").order().is_none());
    }
    // One TurnStart, for the head only; no RT round start ran.
    assert_eq!(pool_of(&world, b), 1);
    assert_eq!(pool_of(&world, a), 0);

    // TB rules now apply: Stop is out, play proceeds, A's turn refreshes it.
    assert!(perform_action(&mut world, &CombatAction::Stop { actor: a }).is_err());
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B attacks");
    assert_eq!(world.combat().expect("encounter").active, Some(a));
    assert_eq!(pool_of(&world, a), 1, "a normal TB TurnStart");
    // Ticks leave TB alone.
    ticks(&mut world, 20);
    assert_eq!(elapsed(&world), 4);
}

/// R11 (B25): the dead are not rescheduled by the switch.
#[test]
fn r11_rt_to_tb_skips_the_dead() {
    let mut fixture = capable_trio(4);
    make_lethal(&mut fixture, 100);
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[2])).expect("order");
    tick(&mut world);
    request_mode(&mut world, EncounterMode::TurnBased).expect("request");
    ticks(&mut world, 3);
    assert_eq!(
        world.combat().expect("encounter").mode,
        EncounterMode::TurnBased
    );
    assert_eq!(
        timeline_of(&world),
        vec![(9, ids[1]), (10, ids[0])],
        "C is dead and unscheduled"
    );
}

/// R11 (B26): TB to RT at the rollover keys everyone at the clock, clears the
/// active turn and runs the RT round start (every alive combatant refreshed).
#[test]
fn r11_tb_to_rt_at_the_rollover() {
    let fixture = capable(4);
    let (mut world, ids) = fixture.start(1);
    let (a, b) = (ids[0], ids[1]);
    request_mode(&mut world, EncounterMode::RealTime).expect("request");
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B attacks");
    assert_eq!(pool_of(&world, b), 0);
    assert_eq!(pool_of(&world, a), 1, "A's own TurnStart");
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("A attacks");

    let combat = world.combat().expect("encounter");
    assert_eq!(combat.mode, EncounterMode::RealTime);
    assert_eq!(combat.pending_mode, None);
    assert_eq!(combat.active, None);
    assert_eq!(combat.round, 1);
    assert_eq!(elapsed(&world), 4);
    assert_eq!(timeline_of(&world), vec![(4, a), (4, b)]);
    // The RT round start refreshed both; a TB rollover would refresh only B.
    assert_eq!((pool_of(&world, a), pool_of(&world, b)), (1, 1));

    // RT now runs: an order at key 4 executes on the first tick (clock 5).
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("order");
    tick(&mut world);
    assert_eq!(pool_of(&world, a), 0);
    assert_eq!(
        timeline_of(&world),
        vec![(4, b), (9, a)],
        "5 + round_ticks 4"
    );
    // The pending switch applied once: a second boundary stays RT.
    ticks(&mut world, 3);
    assert_eq!(
        world.combat().expect("encounter").mode,
        EncounterMode::RealTime
    );
    assert_eq!(world.combat().expect("encounter").round, 2);
}

/// R11 (B25, B26): each switch is one round start, however often the mode is
/// toggled.
#[test]
fn r11_each_switch_is_one_round() {
    let fixture = capable(4);
    let (mut world, ids) = fixture.start(1);
    let mut expected_round = 0;
    for step in 0..6 {
        let mode = world.combat().expect("encounter").mode;
        match mode {
            EncounterMode::TurnBased => {
                request_mode(&mut world, EncounterMode::RealTime).expect("request");
                for actor in [ids[1], ids[0]] {
                    perform_action(&mut world, &CombatAction::EndTurn { actor }).expect("end");
                }
            }
            EncounterMode::RealTime => {
                request_mode(&mut world, EncounterMode::TurnBased).expect("request");
                ticks(&mut world, 4);
            }
        }
        expected_round += 1;
        let combat = world.combat().expect("encounter");
        assert_eq!(combat.round, expected_round, "step {step}");
        assert_ne!(combat.mode, mode, "step {step}: the mode toggled");
    }
    assert_eq!(elapsed(&world), 24);
}
