//! Self-killing actors must not break turn order (T117).
//!
//! When the active actor's own ability kills it, the encounter advances to
//! the next living combatant regardless of `ends_turn`, exactly as for any
//! other turn end: no turn is skipped and a dead actor is never `active`.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Ulid};
use crpg_data::{DamageEntry, OutcomeWire};
use crpg_sim::{perform_action, CombatAction, SimEvent, World};
use support::Fixture;

fn lethal_fixture(ends_turn: bool, trio: bool) -> (Fixture, Ulid) {
    let mut fixture = if trio {
        Fixture::trio()
    } else {
        Fixture::standard()
    };
    let ability = fixture.ability;
    let ab = fixture.abilities.get_mut(&ability).expect("ability");
    ab.allow_self_target = true;
    ab.ends_turn = ends_turn;
    ab.damage = vec![
        DamageEntry {
            outcome: OutcomeWire::Success,
            amount: 100,
            dice: None,
        },
        DamageEntry {
            outcome: OutcomeWire::Failure,
            amount: 100,
            dice: None,
        },
    ];
    (fixture, ability)
}

fn count_deaths(world: &mut World) -> (usize, Option<EntityId>) {
    let mut deaths = 0;
    let mut who = None;
    for env in world.events_mut().drain() {
        if let SimEvent::Died { entity } = env.payload {
            deaths += 1;
            who = Some(entity);
        }
    }
    (deaths, who)
}

#[test]
fn self_kill_ends_turn_true_advances_to_next() {
    let (fixture, ability) = lethal_fixture(true, true);
    let (mut world, ids) = fixture.start(3);
    assert_eq!(world.combat().expect("combat").active, Some(ids[2]));

    let outcome = perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: ids[2],
            ability,
            target: ids[2],
        },
    )
    .expect("valid")
    .expect("attack");
    assert!(outcome.target_died);

    let (deaths, who) = count_deaths(&mut world);
    assert_eq!(deaths, 1);
    assert_eq!(who, Some(ids[2]));

    assert_eq!(world.combat().expect("combat").active, Some(ids[1]));
    assert_eq!(
        world
            .combatants()
            .get(ids[1])
            .expect("B")
            .action_pool()
            .current(),
        1
    );
    let tl: Vec<EntityId> = world.timeline().iter().map(|(_, id)| id).collect();
    assert_eq!(tl, vec![ids[1], ids[0]]);
    assert!(!world.timeline().contains(ids[2]));
    assert!(world.combatants().get(ids[2]).expect("C").dead());
}

#[test]
fn self_kill_ends_turn_false_advances_to_next() {
    let (fixture, ability) = lethal_fixture(false, true);
    let (mut world, ids) = fixture.start(3);
    assert_eq!(world.combat().expect("combat").active, Some(ids[2]));

    let outcome = perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: ids[2],
            ability,
            target: ids[2],
        },
    )
    .expect("valid")
    .expect("attack");
    assert!(outcome.target_died);

    let (deaths, who) = count_deaths(&mut world);
    assert_eq!(deaths, 1);
    assert_eq!(who, Some(ids[2]));

    assert_eq!(world.combat().expect("combat").active, Some(ids[1]));
    assert_eq!(
        world
            .combatants()
            .get(ids[1])
            .expect("B")
            .action_pool()
            .current(),
        1
    );
    let tl: Vec<EntityId> = world.timeline().iter().map(|(_, id)| id).collect();
    assert_eq!(tl, vec![ids[1], ids[0]]);
    assert!(!world.timeline().contains(ids[2]));
}

#[test]
fn self_kill_last_of_round_rolls_over() {
    let (fixture, ability) = lethal_fixture(true, true);
    let (mut world, ids) = fixture.start(3);
    // End C and B so A (last in round) becomes active.
    perform_action(&mut world, &CombatAction::EndTurn { actor: ids[2] }).expect("end C");
    perform_action(&mut world, &CombatAction::EndTurn { actor: ids[1] }).expect("end B");
    assert_eq!(world.combat().expect("combat").active, Some(ids[0]));

    let outcome = perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: ids[0],
            ability,
            target: ids[0],
        },
    )
    .expect("valid")
    .expect("attack");
    assert!(outcome.target_died);

    let (deaths, who) = count_deaths(&mut world);
    assert_eq!(deaths, 1);
    assert_eq!(who, Some(ids[0]));

    let combat = world.combat().expect("combat");
    assert_eq!(combat.round, 1);
    assert_eq!(combat.active, Some(ids[2]));
    let tl: Vec<EntityId> = world.timeline().iter().map(|(_, id)| id).collect();
    assert_eq!(tl, vec![ids[2], ids[1]]);
    assert!(!world.timeline().contains(ids[0]));
}

#[test]
fn self_kill_two_alive_ends_encounter() {
    let (fixture, ability) = lethal_fixture(true, false);
    let (mut world, ids) = fixture.start(5);
    // active is ids[1] (B)
    assert_eq!(world.combat().expect("combat").active, Some(ids[1]));

    let survivor_health_before = world.combatants().get(ids[0]).expect("A").health();
    let survivor_pool_before = world
        .combatants()
        .get(ids[0])
        .expect("A")
        .action_pool()
        .current();

    let outcome = perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: ids[1],
            ability,
            target: ids[1],
        },
    )
    .expect("valid")
    .expect("attack");
    assert!(outcome.target_died);

    let (deaths, who) = count_deaths(&mut world);
    assert_eq!(deaths, 1);
    assert_eq!(who, Some(ids[1]));

    let combat = world.combat().expect("combat");
    assert_eq!(combat.active, None);
    assert_eq!(combat.round, 0);
    // The alive<2 branch leaves the (already-removed) actor untouched and
    // does not pop the surviving combatant, so it stays scheduled.
    assert!(world.timeline().contains(ids[0]));

    let survivor = world.combatants().get(ids[0]).expect("A");
    assert_eq!(survivor.health(), survivor_health_before);
    assert_eq!(survivor.action_pool().current(), survivor_pool_before);
    assert!(!survivor.dead());
}

#[test]
fn nonlethal_self_target_behaves_as_before() {
    let mut fixture = Fixture::standard();
    let ability = fixture.ability;
    {
        let ab = fixture.abilities.get_mut(&ability).expect("ability");
        ab.allow_self_target = true;
        ab.ends_turn = true;
        ab.damage = vec![DamageEntry {
            outcome: OutcomeWire::Success,
            amount: 0,
            dice: None,
        }];
    }
    let (mut world, ids) = fixture.start(5);
    assert_eq!(world.combat().expect("combat").active, Some(ids[1]));

    let outcome = perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: ids[1],
            ability,
            target: ids[1],
        },
    )
    .expect("valid")
    .expect("attack");
    assert!(!outcome.target_died);
    assert_eq!(outcome.damage, 0);

    let (deaths, _) = count_deaths(&mut world);
    assert_eq!(deaths, 0);

    // ends_turn true -> turn passes to ids[0]
    assert_eq!(world.combat().expect("combat").active, Some(ids[0]));
    let tl: Vec<EntityId> = world.timeline().iter().map(|(_, id)| id).collect();
    assert_eq!(tl, vec![ids[0]]);
}
