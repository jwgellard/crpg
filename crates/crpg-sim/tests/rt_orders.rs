//! Real-time admission, `Stop`, `EndTurn` and enumeration (T138a, ADR-0029
//! T8, T15): R10.
//!
//! `validate_action` is the one legality source, so every rung of every
//! precedence ladder is checked three ways: through `validate_action`, through
//! `perform_action` on a clone, and as "the world did not change".

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Ulid};
use crpg_data::Ability;
use crpg_sim::{
    legal_actions, perform_action, state_hash, tick, validate_action, CombatAction, CombatError,
    EncounterMode, EntityMeta, LegalActionsError, World, MAX_LEGAL_ACTIONS,
};
use support::rt::{
    bytes_of, capable, capable_trio, edit_ability, make_lethal, pool_of, ticks, timeline_of,
    use_action, value_of,
};
use support::Fixture;

/// Asserts that `action` is rejected with `expected` by both the validator
/// and the controller, and that the rejection changed nothing.
fn rejected(world: &World, action: CombatAction, expected: CombatError) {
    assert_eq!(validate_action(world, &action), Err(expected.clone()));
    let mut probe = world.clone();
    assert_eq!(perform_action(&mut probe, &action), Err(expected));
    assert_eq!(
        bytes_of(world),
        bytes_of(&probe),
        "a rejection changes nothing"
    );
}

/// The world as JSON with every combatant's `order` removed.
fn without_orders(world: &World) -> serde_json::Value {
    let mut value = value_of(world);
    if let Some(pairs) = value["combatants"].as_array_mut() {
        for pair in pairs {
            if let Some(object) = pair[1].as_object_mut() {
                object.remove("order");
            }
        }
    }
    value
}

fn derived(base: &Ability, id: u128, edit: impl FnOnce(&mut Ability)) -> Ability {
    let mut ability = base.clone();
    ability.id = Ulid::from_u128(id);
    ability.slug = format!("probe-ability-{id}");
    ability.name = format!("probe.ability-{id}");
    edit(&mut ability);
    ability
}

fn add_ability(fixture: &mut Fixture, ability: Ability) {
    fixture.ruleset.abilities.push(ability.id);
    fixture.abilities.insert(ability.id, ability);
}

/// A despawned (stale) id and a live non-combatant in `world`.
fn ghosts(world: &mut World) -> (EntityId, EntityId) {
    let gone = world.spawn(EntityMeta {});
    assert!(world.despawn(gone));
    let live = world.spawn(EntityMeta {});
    (gone, live)
}

/// R10 (B19): the `UseAbility` ladder in RT, rung by rung.
#[test]
fn r10_use_ability_ladder_in_rt() {
    let mut fixture = capable_trio(4);
    make_lethal(&mut fixture, 100);
    let ability = fixture.ability;
    let unknown = Ulid::from_u128(999);

    // NoEncounter.
    let mut empty = World::new(1);
    let (_, live) = ghosts(&mut empty);
    rejected(
        &empty,
        use_action(live, ability, live),
        CombatError::NoEncounter,
    );

    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    let (gone2, live2) = ghosts(&mut world);
    // NotParticipant: the actor first, then the target, both before absence.
    rejected(
        &world,
        use_action(live2, ability, a),
        CombatError::NotParticipant { entity: live2 },
    );
    rejected(
        &world,
        use_action(a, ability, live2),
        CombatError::NotParticipant { entity: live2 },
    );
    rejected(
        &world,
        use_action(gone2, ability, live2),
        CombatError::NotParticipant { entity: live2 },
    );
    // AbsentActor before AbsentTarget.
    rejected(
        &world,
        use_action(gone2, ability, a),
        CombatError::AbsentActor { actor: gone2 },
    );
    rejected(
        &world,
        use_action(gone2, ability, gone2),
        CombatError::AbsentActor { actor: gone2 },
    );
    rejected(
        &world,
        use_action(a, ability, gone2),
        CombatError::AbsentTarget { target: gone2 },
    );

    // One combatant dead (C killed by A in the first pass).
    perform_action(&mut world, &use_action(a, ability, c)).expect("order");
    tick(&mut world);
    assert!(world.combatants().get(c).expect("C").dead());
    rejected(
        &world,
        use_action(c, ability, a),
        CombatError::DeadActor { actor: c },
    );
    rejected(
        &world,
        use_action(a, ability, c),
        CombatError::DeadTarget { target: c },
    );
    rejected(
        &world,
        use_action(c, ability, c),
        CombatError::DeadActor { actor: c },
    );
    rejected(
        &world,
        use_action(c, ability, gone2),
        CombatError::AbsentTarget { target: gone2 },
    );
    // A spent action still shows as unaffordable (current balances).
    rejected(
        &world,
        use_action(a, ability, b),
        CombatError::InsufficientAction {
            pool: Ulid::from_u128(105),
            cost: 1,
            current: 0,
        },
    );
    // SelfTarget precedes affordability; UnknownAbility precedes SelfTarget.
    rejected(&world, use_action(a, ability, a), CombatError::SelfTarget);
    rejected(
        &world,
        use_action(a, unknown, a),
        CombatError::UnknownAbility { ability: unknown },
    );
    // The key is not due (A was re-keyed to 5), yet an affordable order is
    // admitted: B is unspent.
    assert!(validate_action(&world, &use_action(b, ability, a)).is_ok());

    // Terminal: only A is left alive. The gate precedes every later rung.
    ticks(&mut world, 4);
    perform_action(&mut world, &use_action(a, ability, b)).expect("A orders again");
    ticks(&mut world, 1);
    assert!(world.combatants().get(b).expect("B").dead());
    let gate = CombatError::OutOfTurn {
        actor: a,
        active: None,
    };
    rejected(&world, use_action(a, ability, a), gate.clone());
    rejected(&world, use_action(a, unknown, a), gate.clone());
    // Dead combatants still win over the gate.
    rejected(
        &world,
        use_action(a, ability, b),
        CombatError::DeadTarget { target: b },
    );
}

/// R10 (B20): the `Stop` ladder in both modes.
#[test]
fn r10_stop_ladder() {
    let mut fixture = capable_trio(4);
    make_lethal(&mut fixture, 100);
    let ability = fixture.ability;

    let mut empty = World::new(1);
    let (_, live) = ghosts(&mut empty);
    rejected(
        &empty,
        CombatAction::Stop { actor: live },
        CombatError::NoEncounter,
    );

    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    let (gone, live) = ghosts(&mut world);
    rejected(
        &world,
        CombatAction::Stop { actor: live },
        CombatError::NotParticipant { entity: live },
    );
    rejected(
        &world,
        CombatAction::Stop { actor: gone },
        CombatError::AbsentActor { actor: gone },
    );

    // Accepted: Ok(None), a no-op without an order.
    let before = bytes_of(&world);
    assert_eq!(
        perform_action(&mut world, &CombatAction::Stop { actor: b }),
        Ok(None)
    );
    assert_eq!(before, bytes_of(&world), "Stop without an order is a no-op");
    // With an order it clears exactly that order.
    perform_action(&mut world, &use_action(b, ability, a)).expect("order");
    perform_action(&mut world, &use_action(a, ability, c)).expect("order");
    let base = without_orders(&world);
    assert_eq!(
        perform_action(&mut world, &CombatAction::Stop { actor: b }),
        Ok(None)
    );
    assert!(world.combatants().get(b).expect("B").order().is_none());
    assert!(world.combatants().get(a).expect("A").order().is_some());
    assert_eq!(base, without_orders(&world), "only `order` changed");

    // DeadActor.
    tick(&mut world);
    assert!(world.combatants().get(c).expect("C").dead());
    rejected(
        &world,
        CombatAction::Stop { actor: c },
        CombatError::DeadActor { actor: c },
    );

    // Terminal gate: A kills B too, leaving A alone.
    ticks(&mut world, 3);
    perform_action(&mut world, &use_action(a, ability, b)).expect("order");
    tick(&mut world);
    assert!(world.combatants().get(b).expect("B").dead());
    rejected(
        &world,
        CombatAction::Stop { actor: a },
        CombatError::OutOfTurn {
            actor: a,
            active: None,
        },
    );
    rejected(
        &world,
        CombatAction::Stop { actor: b },
        CombatError::DeadActor { actor: b },
    );

    // Turn-based: WrongMode { TurnBased } once the actor checks pass.
    let (tb, ids) = fixture.start(1);
    rejected(
        &tb,
        CombatAction::Stop { actor: ids[0] },
        CombatError::WrongMode {
            mode: EncounterMode::TurnBased,
        },
    );
    let (gone, _) = ghosts(&mut tb.clone());
    rejected(
        &tb,
        CombatAction::Stop { actor: gone },
        CombatError::AbsentActor { actor: gone },
    );
    assert_eq!(
        CombatError::WrongMode {
            mode: EncounterMode::TurnBased
        }
        .to_string(),
        "WrongMode at combat/mode"
    );
}

/// R10 (B20): in TB the actor rungs precede the mode rung, so a live
/// non-participant is `NotParticipant` and a dead combatant is `DeadActor`
/// rather than `WrongMode`.
#[test]
fn r10_tb_stop_ladder_precedes_wrong_mode() {
    let mut fixture = capable(4);
    make_lethal(&mut fixture, 100);
    let (mut world, ids) = fixture.start(1);
    let (a, b) = (ids[0], ids[1]);

    // A live non-participant: the not-participant rung, before the mode.
    let (_, live) = ghosts(&mut world);
    rejected(
        &world,
        CombatAction::Stop { actor: live },
        CombatError::NotParticipant { entity: live },
    );

    // B heads the fixture and kills A; a dead actor is `DeadActor` in TB too.
    assert_eq!(world.combat().expect("encounter").active, Some(b));
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B's kill");
    assert!(world.combatants().get(a).expect("A").dead());
    rejected(
        &world,
        CombatAction::Stop { actor: a },
        CombatError::DeadActor { actor: a },
    );

    // The remaining rungs hold: absent, then a live participant's mode.
    let (gone, _) = ghosts(&mut world);
    rejected(
        &world,
        CombatAction::Stop { actor: gone },
        CombatError::AbsentActor { actor: gone },
    );
    rejected(
        &world,
        CombatAction::Stop { actor: b },
        CombatError::WrongMode {
            mode: EncounterMode::TurnBased,
        },
    );
}

/// R10 (B21): `EndTurn` is `WrongMode` in RT at the turn stage, and exactly
/// today's in TB.
#[test]
fn r10_end_turn_in_both_modes() {
    let mut fixture = capable_trio(4);
    make_lethal(&mut fixture, 100);

    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    let (gone, live) = ghosts(&mut world);
    rejected(
        &world,
        CombatAction::EndTurn { actor: live },
        CombatError::NotParticipant { entity: live },
    );
    rejected(
        &world,
        CombatAction::EndTurn { actor: gone },
        CombatError::AbsentActor { actor: gone },
    );
    let wrong = CombatError::WrongMode {
        mode: EncounterMode::RealTime,
    };
    rejected(&world, CombatAction::EndTurn { actor: a }, wrong.clone());
    // DeadActor still precedes the mode stage.
    perform_action(&mut world, &use_action(a, fixture.ability, ids[2])).expect("order");
    tick(&mut world);
    rejected(
        &world,
        CombatAction::EndTurn { actor: ids[2] },
        CombatError::DeadActor { actor: ids[2] },
    );
    // Terminal RT is still WrongMode, not the terminal gate.
    ticks(&mut world, 3);
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("order");
    tick(&mut world);
    rejected(&world, CombatAction::EndTurn { actor: a }, wrong);

    // Turn-based: OutOfTurn off-turn, Ok on-turn, and no order is ever held.
    let (mut tb, ids) = fixture.start(1);
    let head = ids[2];
    rejected(
        &tb,
        CombatAction::EndTurn { actor: ids[0] },
        CombatError::OutOfTurn {
            actor: ids[0],
            active: Some(head),
        },
    );
    rejected(
        &tb,
        use_action(ids[0], fixture.ability, ids[1]),
        CombatError::OutOfTurn {
            actor: ids[0],
            active: Some(head),
        },
    );
    assert!(matches!(
        perform_action(&mut tb, &use_action(head, fixture.ability, ids[0])),
        Ok(Some(_))
    ));
    for (_, combatant) in tb.combatants().iter() {
        assert!(combatant.order().is_none(), "a TB combatant holds no order");
    }
}

/// R10 (B19): an accepted RT `UseAbility` returns `Ok(None)`, changes only
/// the actor's order, replaces an earlier order and takes the repeat flag from
/// the ability.
#[test]
fn r10_use_ability_stores_exactly_one_order() {
    let mut fixture = capable_trio(4);
    // Pool 3 so an actor can still afford an order after acting.
    fixture.ruleset.pools[0].max = 3;
    let base = fixture.abilities[&fixture.ability].clone();
    add_ability(
        &mut fixture,
        derived(&base, 201, |ability| {
            ability.repeats_in_real_time = Some(true);
            ability.allow_self_target = true;
        }),
    );
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    let first = without_orders(&world);

    assert_eq!(
        perform_action(&mut world, &use_action(a, fixture.ability, b)),
        Ok(None)
    );
    let order = world
        .combatants()
        .get(a)
        .expect("A")
        .order()
        .expect("order");
    assert_eq!(
        (order.ability, order.target, order.repeat),
        (fixture.ability, b, false)
    );
    assert_eq!(first, without_orders(&world), "only `order` changed");
    // Replacement, with the other ability's repeat flag and a self target.
    let second = Ulid::from_u128(201);
    assert_eq!(
        perform_action(&mut world, &use_action(a, second, a)),
        Ok(None)
    );
    let order = world
        .combatants()
        .get(a)
        .expect("A")
        .order()
        .expect("order");
    assert_eq!(
        (order.ability, order.target, order.repeat),
        (second, a, true)
    );
    assert_eq!(first, without_orders(&world));
    assert!(world.combatants().get(b).expect("B").order().is_none());
    assert!(world.combatants().get(c).expect("C").order().is_none());

    // After acting (key not due) an affordable order is admitted against the
    // current balance: 3 - 1 = 2.
    tick(&mut world);
    assert_eq!(pool_of(&world, a), 2);
    assert_eq!(timeline_of(&world)[2].0, 5, "A is re-keyed, so not due");
    let before = without_orders(&world);
    assert_eq!(
        perform_action(&mut world, &use_action(a, fixture.ability, c)),
        Ok(None)
    );
    assert_eq!(
        before,
        without_orders(&world),
        "no key, pool or draw changed"
    );
}

/// The abilities and targets `legal_actions` lists, as literal ids.
fn literal(options: &[CombatAction]) -> Vec<(u128, Option<EntityId>)> {
    options
        .iter()
        .map(|option| match option {
            CombatAction::UseAbility {
                ability, target, ..
            } => (ability.to_u128(), Some(*target)),
            CombatAction::Stop { .. } => (0, None),
            CombatAction::EndTurn { .. } => (u128::MAX, None),
        })
        .collect()
}

/// R10 (B22): RT enumeration lists accepted uses by ascending ability and
/// target, then `Stop`, and never `EndTurn`.
#[test]
fn r10_rt_legal_actions_list() {
    let mut fixture = capable_trio(4);
    make_lethal(&mut fixture, 100);
    let base = fixture.abilities[&fixture.ability].clone();
    add_ability(
        &mut fixture,
        derived(&base, 201, |ability| ability.allow_self_target = true),
    );
    let (mut world, ids) = fixture.start_rt(3, EncounterMode::RealTime);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    let bytes = bytes_of(&world);
    let hash = state_hash(&world);

    let options = legal_actions(&world, a).expect("A may order");
    assert_eq!(
        literal(&options),
        vec![
            (103, Some(b)),
            (103, Some(c)),
            (201, Some(a)),
            (201, Some(b)),
            (201, Some(c)),
            (0, None),
        ]
    );
    assert_eq!(options.last(), Some(&CombatAction::Stop { actor: a }));
    for option in &options {
        let mut probe = world.clone();
        assert!(perform_action(&mut probe, option).is_ok(), "{option:?}");
    }
    assert_eq!(bytes, bytes_of(&world), "enumeration changed nothing");
    assert_eq!(hash, state_hash(&world));

    // An actor who cannot pay lists only `Stop`.
    perform_action(&mut world, &use_action(a, fixture.ability, b)).expect("order");
    tick(&mut world);
    assert_eq!(
        legal_actions(&world, a),
        Ok(vec![CombatAction::Stop { actor: a }])
    );
    // Dead and non-participating actors fail whole.
    assert!(world.combatants().get(b).expect("B").dead());
    assert_eq!(
        legal_actions(&world, b),
        Err(LegalActionsError::Actor(CombatError::DeadActor {
            actor: b
        }))
    );
    let (_, live) = ghosts(&mut world);
    assert_eq!(
        legal_actions(&world, live),
        Err(LegalActionsError::Actor(CombatError::NotParticipant {
            entity: live
        }))
    );
    // Dead targets are excluded for a live actor.
    let options = legal_actions(&world, c).expect("C may order");
    assert!(options.iter().all(|option| !matches!(
        option,
        CombatAction::UseAbility { target, .. } if *target == b
    )));
}

/// A crowd: `participants` clones of creature A and `abilities` self-allowed
/// abilities, all affordable.
fn crowd(participants: usize, abilities: usize) -> Fixture {
    let mut fixture = capable(4);
    let base = fixture.abilities[&fixture.ability].clone();
    fixture
        .abilities
        .get_mut(&fixture.ability)
        .expect("ability")
        .allow_self_target = true;
    for extra in 1..abilities {
        add_ability(
            &mut fixture,
            derived(&base, 400 + extra as u128, |a| a.allow_self_target = true),
        );
    }
    let template = fixture.placements[&fixture.placement_a].0.clone();
    fixture.placements.clear();
    fixture.encounter.participants.clear();
    for index in 0..participants {
        let id = Ulid::from_u128(10_000 + index as u128);
        let mut placement = template.clone();
        placement.id = id;
        placement.slug = format!("probe-crowd-{index}");
        placement.name = format!("probe.crowd-{index}");
        fixture.placements.insert(id, (placement, fixture.area));
        fixture
            .encounter
            .participants
            .push(crpg_data::EncounterParticipant {
                placement: id,
                initiative: 0,
            });
    }
    fixture
}

/// R10 (B22): `Stop` counts toward `MAX_LEGAL_ACTIONS`, and an overflow still
/// fails whole.
#[test]
fn r10_rt_legal_actions_bound_counts_stop() {
    // 63 x 65 = 4095 uses plus Stop = exactly the limit.
    let (world, ids) = crowd(65, 63).start_rt(1, EncounterMode::RealTime);
    let actor = ids[0];
    let options = legal_actions(&world, actor).expect("exactly the limit succeeds");
    assert_eq!(options.len(), MAX_LEGAL_ACTIONS);
    assert_eq!(options.last(), Some(&CombatAction::Stop { actor }));
    assert!(options[..MAX_LEGAL_ACTIONS - 1]
        .iter()
        .all(|option| matches!(option, CombatAction::UseAbility { .. })));

    // 64 x 64 = 4096 uses: Stop would be the 4097th.
    let (world, ids) = crowd(64, 64).start_rt(1, EncounterMode::RealTime);
    let bytes = bytes_of(&world);
    assert_eq!(
        legal_actions(&world, ids[0]),
        Err(LegalActionsError::TooManyOptions { limit: 4096 })
    );
    assert_eq!(bytes, bytes_of(&world));
}

/// R10 (B21, B22): TB enumeration is unchanged and never lists `Stop`, in an
/// RT-capable fight and a TB-only one alike.
#[test]
fn r10_tb_enumeration_is_unchanged() {
    let only = Fixture::trio();
    let cap = capable_trio(4);
    let (world_only, ids_only) = only.start(5);
    let (world_cap, ids_cap) = cap.start(5);
    let head = ids_only[2];
    assert_eq!(head, ids_cap[2]);
    let options_only = legal_actions(&world_only, head).expect("C may act");
    let options_cap = legal_actions(&world_cap, head).expect("C may act");
    assert_eq!(options_only, options_cap);
    assert_eq!(
        options_cap.last(),
        Some(&CombatAction::EndTurn { actor: head })
    );
    assert!(options_cap
        .iter()
        .all(|option| !matches!(option, CombatAction::Stop { .. })));
    assert_eq!(
        legal_actions(&world_cap, ids_cap[0]),
        Err(LegalActionsError::Actor(CombatError::OutOfTurn {
            actor: ids_cap[0],
            active: Some(head),
        }))
    );
}

/// R10 (B19): a mode switch to TB drops every order, and a TB combatant never
/// holds one afterwards; `edit_ability` keeps a repeating order alive until
/// then.
#[test]
fn r10_orders_do_not_survive_into_tb() {
    let mut fixture = capable(4);
    edit_ability(&mut fixture, |ability| {
        ability.repeats_in_real_time = Some(true);
    });
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1])).expect("order");
    crpg_sim::request_mode(&mut world, EncounterMode::TurnBased).expect("request");
    ticks(&mut world, 4);
    assert_eq!(
        world.combat().expect("encounter").mode,
        EncounterMode::TurnBased
    );
    for (_, combatant) in world.combatants().iter() {
        assert!(combatant.order().is_none());
    }
}
