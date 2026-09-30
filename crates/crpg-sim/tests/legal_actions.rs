//! Shared read-only combat legality (T028, ADR-0018).
//!
//! `validate_action` is the single pre-mutation admission path that
//! `perform_action` runs first, and `legal_actions` enumerates exactly the
//! candidates it accepts. Every inclusion claim here is checked against an
//! independent oracle — `perform_action` on a public clone of the same world —
//! never against `validate_action` itself. Fixtures are neutral documents
//! parsed through the production data shapes.

#[allow(dead_code)]
mod support;

use std::error::Error as _;

use crpg_core::{EntityId, Ulid};
use crpg_data::{Ability, AbilityCost, ActionPoolTemplate, DefenseWire, RefreshWire};
use crpg_sim::{
    legal_actions, perform_action, start_encounter, state_hash, validate_action, CombatAction,
    CombatError, EncounterSpec, EntityMeta, LegalActionsError, World, COMBAT_ROLL_STREAM,
    MAX_LEGAL_ACTIONS,
};
use support::Fixture;

/// Serialized bytes snapshotting the entire world.
fn snapshot(world: &World) -> Vec<u8> {
    serde_json::to_vec(world).expect("the world serializes")
}

/// Asserts that bytes and hash both still match a snapshot.
fn assert_unchanged(world: &World, bytes: &[u8], hash: [u8; 32], context: &str) {
    assert_eq!(
        bytes,
        snapshot(world).as_slice(),
        "bytes changed: {context}"
    );
    assert_eq!(hash, state_hash(world), "hash changed: {context}");
}

/// Independent oracle: does `perform_action` accept `action` on a clone?
fn executes(world: &World, action: &CombatAction) -> Result<(), CombatError> {
    let mut probe = world.clone();
    perform_action(&mut probe, action).map(|_| ())
}

/// Clones `base` under a fresh identity, then applies `edit`.
fn derived(base: &Ability, id: u128, edit: impl FnOnce(&mut Ability)) -> Ability {
    let mut ability = base.clone();
    ability.id = Ulid::from_u128(id);
    ability.slug = format!("probe-ability-{id}");
    ability.name = format!("probe.ability-{id}");
    edit(&mut ability);
    ability
}

/// Adds one ability to the fixture's documents and ruleset listing.
fn add_ability(fixture: &mut Fixture, ability: Ability) {
    fixture.ruleset.abilities.push(ability.id);
    fixture.abilities.insert(ability.id, ability);
}

fn primary() -> Ulid {
    Ulid::from_u128(105)
}

fn second_pool() -> Ulid {
    Ulid::from_u128(106)
}

fn ability(value: u128) -> Ulid {
    Ulid::from_u128(value)
}

/// The legality matrix fixture over the trio (C acts first, then B, then A):
///
/// - 103 standard: targeted, self-target disallowed, primary cost 1.
/// - 201 self-target allowed.
/// - 202 `requires_target == false`, self-target disallowed.
/// - 203 multi-pool: primary 1 plus second pool 2 (max 2), affordable.
/// - 204 repeated primary entries summing 2 over max 1: never affordable.
/// - 205 target-stat defense on `beta`, non-ending, self-target allowed.
///
/// Every checked attribute is 100 so every roll succeeds; damage stays 2.
fn matrix() -> Fixture {
    let mut fixture = Fixture::trio();
    fixture.ruleset.pools.push(ActionPoolTemplate {
        id: second_pool(),
        max: 2,
        refresh: RefreshWire::Never,
    });
    let base = fixture.abilities[&fixture.ability].clone();
    add_ability(
        &mut fixture,
        derived(&base, 201, |a| a.allow_self_target = true),
    );
    add_ability(
        &mut fixture,
        derived(&base, 202, |a| a.requires_target = false),
    );
    add_ability(
        &mut fixture,
        derived(&base, 203, |a| {
            a.extra_costs = vec![AbilityCost {
                pool: second_pool(),
                amount: 2,
            }];
        }),
    );
    add_ability(
        &mut fixture,
        derived(&base, 204, |a| {
            a.extra_costs = vec![AbilityCost {
                pool: primary(),
                amount: 1,
            }];
        }),
    );
    add_ability(
        &mut fixture,
        derived(&base, 205, |a| {
            a.defense = DefenseWire::TargetStat {
                stat: "beta".to_owned(),
            };
            a.ends_turn = false;
            a.allow_self_target = true;
        }),
    );
    let ids: Vec<Ulid> = fixture.creatures.keys().copied().collect();
    for id in ids {
        fixture
            .creatures
            .get_mut(&id)
            .expect("creature")
            .stats
            .insert("alpha".to_owned(), crpg_core::Fx16_16::from_int(100));
    }
    fixture
}

/// Starts `fixture` in `world` (which may already hold other entities).
fn start_in(fixture: &Fixture, world: &mut World) -> Vec<EntityId> {
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
    let before: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
    start_encounter(world, &spec).expect("the fixture starts");
    world
        .combatants()
        .iter()
        .map(|(id, _)| id)
        .filter(|id| !before.contains(id))
        .collect()
}

/// Active combatant of a started encounter.
fn active(world: &World) -> EntityId {
    world
        .combat()
        .expect("combat")
        .active
        .expect("an active turn")
}

fn use_ability(actor: EntityId, ability: Ulid, target: EntityId) -> CombatAction {
    CombatAction::UseAbility {
        actor,
        ability,
        target,
    }
}

#[test]
fn options_match_independent_execution_matrix() {
    let fixture = matrix();
    let mut world = World::new(11);
    let outsider = world.spawn(EntityMeta {});
    let ids = start_in(&fixture, &mut world);
    // Authored order A, B, C; C (initiative 8) acts first.
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    assert_eq!(active(&world), c);

    // Hand-authored expectation, in the pinned (ability ULID, target id)
    // order with EndTurn last. Target ids ascend as A < B < C here.
    assert!(a < b && b < c);
    let expected = vec![
        use_ability(c, ability(103), a),
        use_ability(c, ability(103), b),
        use_ability(c, ability(201), a),
        use_ability(c, ability(201), b),
        use_ability(c, ability(201), c),
        use_ability(c, ability(202), a),
        use_ability(c, ability(202), b),
        use_ability(c, ability(203), a),
        use_ability(c, ability(203), b),
        use_ability(c, ability(205), a),
        use_ability(c, ability(205), b),
        use_ability(c, ability(205), c),
        CombatAction::EndTurn { actor: c },
    ];
    let options = legal_actions(&world, c).expect("C may act");
    assert_eq!(options, expected);

    // Candidate universe: every ability (plus an unlisted one) against every
    // live entity, combatant or not. Inclusion must match execution.
    let abilities = [103, 201, 202, 203, 204, 205, 999].map(ability);
    let targets = [a, b, c, outsider];
    for ability_id in abilities {
        for target in targets {
            let candidate = use_ability(c, ability_id, target);
            let executed = executes(&world, &candidate);
            assert_eq!(
                options.contains(&candidate),
                executed.is_ok(),
                "inclusion disagrees with execution for {candidate:?}: {executed:?}"
            );
        }
    }
    assert_eq!(
        executes(&world, &use_ability(c, ability(204), a)),
        Err(CombatError::InsufficientAction {
            pool: primary(),
            cost: 2,
            current: 1,
        }),
        "the repeated-pool total is the reason 204 is absent"
    );
    assert!(executes(&world, &CombatAction::EndTurn { actor: c }).is_ok());
    // No duplicates.
    for (index, option) in options.iter().enumerate() {
        assert!(
            !options[index + 1..].contains(option),
            "duplicate {option:?}"
        );
    }

    // A dead target disappears from every ability, through execution.
    let mut world = world;
    perform_action(&mut world, &use_ability(c, ability(103), b)).expect("hit");
    perform_action(&mut world, &use_ability(a, ability(103), b))
        .expect_err("A does not hold the turn yet");
    // Kill B outright: B has health 6, every roll succeeds for 2 damage.
    let mut attacker = active(&world);
    while !world.combatants().get(b).expect("B").dead() {
        let target = if attacker == b { a } else { b };
        perform_action(&mut world, &use_ability(attacker, ability(103), target)).expect("hit");
        attacker = active(&world);
    }
    let actor = active(&world);
    let options = legal_actions(&world, actor).expect("a living actor may act");
    for option in &options {
        if let CombatAction::UseAbility { target, .. } = option {
            assert_ne!(*target, b, "dead target listed: {option:?}");
        }
        assert!(executes(&world, option).is_ok(), "{option:?} must execute");
    }
    assert_eq!(
        executes(&world, &use_ability(actor, ability(201), b)),
        Err(CombatError::DeadTarget { target: b })
    );
}

#[test]
fn query_and_validation_preserve_complete_state() {
    let fixture = matrix();
    let mut world = World::new(21);
    let outsider = world.spawn(EntityMeta {});
    let ids = start_in(&fixture, &mut world);
    let (a, _b, c) = (ids[0], ids[1], ids[2]);
    let pristine = world.clone();
    let bytes = snapshot(&world);
    let hash = state_hash(&world);

    let first = legal_actions(&world, c).expect("C may act");
    assert_unchanged(&world, &bytes, hash, "first query");
    let second = legal_actions(&world, c).expect("C may act");
    assert_unchanged(&world, &bytes, hash, "repeated query");
    assert_eq!(first, second, "queries are deterministic");

    let probes = [
        use_ability(c, ability(205), a),
        use_ability(c, ability(204), a),
        use_ability(c, ability(103), c),
        use_ability(c, ability(999), a),
        use_ability(a, ability(103), c),
        use_ability(c, ability(103), outsider),
        CombatAction::EndTurn { actor: a },
        CombatAction::EndTurn { actor: c },
    ];
    for probe in probes {
        let _ = validate_action(&world, &probe);
        assert_unchanged(&world, &bytes, hash, &format!("validate {probe:?}"));
    }
    for actor in [a, outsider] {
        let _ = legal_actions(&world, actor);
        assert_unchanged(&world, &bytes, hash, "rejected query");
    }
    assert!(
        !world.rng_mut().has_stream(COMBAT_ROLL_STREAM),
        "queries never create the combat stream"
    );

    // Resolving after queries matches resolving on a never-queried twin:
    // no RNG draw, stream, or interner entry was consumed by the queries.
    let action = use_ability(c, ability(205), a);
    let mut twin = pristine;
    let queried = perform_action(&mut world, &action).expect("legal");
    let fresh = perform_action(&mut twin, &action).expect("legal");
    assert_eq!(queried, fresh);
    assert_eq!(snapshot(&world), snapshot(&twin));
    assert_eq!(state_hash(&world), state_hash(&twin));
}

/// Asserts validation and execution agree on one literal error.
fn assert_rejects(world: &World, action: CombatAction, expected: CombatError) {
    assert_eq!(
        validate_action(world, &action),
        Err(expected.clone()),
        "validate_action for {action:?}"
    );
    let bytes = snapshot(world);
    let mut probe = world.clone();
    assert_eq!(
        perform_action(&mut probe, &action),
        Err(expected),
        "perform_action for {action:?}"
    );
    assert_eq!(bytes, snapshot(&probe), "rejection mutated {action:?}");
}

#[test]
fn action_error_precedence_is_unchanged() {
    // No encounter wins for both verbs, whatever the ids.
    let mut empty = World::new(1);
    let stray = empty.spawn(EntityMeta {});
    assert_rejects(
        &empty,
        use_ability(stray, ability(103), stray),
        CombatError::NoEncounter,
    );
    assert_rejects(
        &empty,
        CombatAction::EndTurn { actor: stray },
        CombatError::NoEncounter,
    );

    // Trio with B at health 2 so the first hit kills it.
    let mut fixture = matrix();
    let creature_b = fixture.creature_b;
    fixture
        .creatures
        .get_mut(&creature_b)
        .expect("B")
        .stats
        .insert("health".to_owned(), crpg_core::Fx16_16::from_int(2));
    let mut world = World::new(3);
    let outsider = world.spawn(EntityMeta {});
    let gone = world.spawn(EntityMeta {});
    assert!(world.despawn(gone));
    let ids = start_in(&fixture, &mut world);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    assert!(!world.contains(gone));

    // Live nonparticipant target precedes an absent actor.
    assert_rejects(
        &world,
        use_ability(gone, ability(103), outsider),
        CombatError::NotParticipant { entity: outsider },
    );
    // Nonparticipant actor precedes nonparticipant target.
    assert_rejects(
        &world,
        use_ability(outsider, ability(103), outsider),
        CombatError::NotParticipant { entity: outsider },
    );
    assert_rejects(
        &world,
        CombatAction::EndTurn { actor: outsider },
        CombatError::NotParticipant { entity: outsider },
    );
    // Absent actor precedes absent target.
    assert_rejects(
        &world,
        use_ability(gone, ability(103), gone),
        CombatError::AbsentActor { actor: gone },
    );
    assert_rejects(
        &world,
        use_ability(c, ability(103), gone),
        CombatError::AbsentTarget { target: gone },
    );
    assert_rejects(
        &world,
        CombatAction::EndTurn { actor: gone },
        CombatError::AbsentActor { actor: gone },
    );
    // Out of turn precedes an unknown ability.
    assert_rejects(
        &world,
        use_ability(a, ability(999), c),
        CombatError::OutOfTurn {
            actor: a,
            active: Some(c),
        },
    );
    // Unknown ability precedes a self-target.
    assert_rejects(
        &world,
        use_ability(c, ability(999), c),
        CombatError::UnknownAbility {
            ability: ability(999),
        },
    );
    // Self-target precedes an unaffordable total.
    assert_rejects(
        &world,
        use_ability(c, ability(204), c),
        CombatError::SelfTarget,
    );

    // Kill B: C hits for 2.
    perform_action(&mut world, &use_ability(c, ability(103), b)).expect("hit");
    assert!(world.combatants().get(b).expect("B").dead());
    let turn = active(&world);
    assert_eq!(turn, a, "B is skipped once dead");
    // Dead actor precedes dead target, and dead target precedes out-of-turn.
    assert_rejects(
        &world,
        use_ability(b, ability(201), b),
        CombatError::DeadActor { actor: b },
    );
    assert_rejects(
        &world,
        use_ability(c, ability(103), b),
        CombatError::DeadTarget { target: b },
    );
    assert_rejects(
        &world,
        CombatAction::EndTurn { actor: b },
        CombatError::DeadActor { actor: b },
    );
    assert_rejects(
        &world,
        CombatAction::EndTurn { actor: c },
        CombatError::OutOfTurn {
            actor: c,
            active: Some(a),
        },
    );
}

#[test]
fn pool_precedence_matches_t019() {
    // Pools in template order: primary (max 1), then a wide second pool.
    let mut fixture = Fixture::standard();
    fixture.ruleset.pools.push(ActionPoolTemplate {
        id: second_pool(),
        max: u32::MAX,
        refresh: RefreshWire::Never,
    });
    let base = fixture.abilities[&fixture.ability].clone();
    // Primary insufficient (1 + 1 > 1) before a later overflow.
    add_ability(
        &mut fixture,
        derived(&base, 301, |a| {
            a.extra_costs = vec![
                AbilityCost {
                    pool: second_pool(),
                    amount: u32::MAX,
                },
                AbilityCost {
                    pool: primary(),
                    amount: 1,
                },
                AbilityCost {
                    pool: second_pool(),
                    amount: u32::MAX,
                },
            ];
        }),
    );
    // Primary affordable (0), second overflows.
    add_ability(
        &mut fixture,
        derived(&base, 302, |a| {
            a.cost = 0;
            a.extra_costs = vec![
                AbilityCost {
                    pool: second_pool(),
                    amount: u32::MAX,
                },
                AbilityCost {
                    pool: second_pool(),
                    amount: 1,
                },
            ];
        }),
    );
    let (world, ids) = fixture.start(7);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(active(&world), b);
    assert_rejects(
        &world,
        use_ability(b, ability(301), a),
        CombatError::InsufficientAction {
            pool: primary(),
            cost: 2,
            current: 1,
        },
    );
    assert_rejects(
        &world,
        use_ability(b, ability(302), a),
        CombatError::ValueOverflow,
    );
    // A failing candidate excludes only itself.
    assert_eq!(
        legal_actions(&world, b).expect("B may act"),
        vec![
            use_ability(b, ability(103), a),
            CombatAction::EndTurn { actor: b },
        ]
    );
}

#[test]
fn full_identity_order_survives_slot_reuse() {
    let mut fixture = Fixture::trio();
    let base = fixture.abilities[&fixture.ability].clone();
    // List the self-allowed ability first: output order is by ULID, not
    // by authored listing.
    let wide = derived(&base, 201, |a| a.allow_self_target = true);
    fixture.ruleset.abilities.insert(0, wide.id);
    fixture.abilities.insert(wide.id, wide);

    let mut world = World::new(5);
    let scratch = world.spawn(EntityMeta {});
    assert!(world.despawn(scratch));
    let ids = start_in(&fixture, &mut world);
    let shape: Vec<(u32, u32)> = ids.iter().map(|id| (id.index(), id.generation())).collect();
    assert_eq!(shape, vec![(0, 2), (1, 1), (2, 1)], "A reuses slot 0");

    // Reverse the persisted combatant store so insertion order no longer
    // matches identity order, then reload through the validated path.
    let mut value = serde_json::to_value(&world).expect("serialize");
    value["combatants"]
        .as_array_mut()
        .expect("combatants are a pair-list")
        .reverse();
    let world: World = serde_json::from_value(value).expect("reordered save loads");
    let stored: Vec<(u32, u32)> = world
        .combatants()
        .iter()
        .map(|(id, _)| (id.index(), id.generation()))
        .collect();
    assert_eq!(stored, vec![(2, 1), (1, 1), (0, 2)], "store order reversed");

    let actor = active(&world);
    assert_eq!((actor.index(), actor.generation()), (2, 1), "C acts first");
    let options = legal_actions(&world, actor).expect("C may act");
    let literal: Vec<(u128, (u32, u32))> = options
        .iter()
        .filter_map(|option| match option {
            CombatAction::UseAbility {
                ability, target, ..
            } => Some((ability.to_u128(), (target.index(), target.generation()))),
            CombatAction::EndTurn { .. } => None,
        })
        .collect();
    assert_eq!(
        literal,
        vec![
            (103, (0, 2)),
            (103, (1, 1)),
            (201, (0, 2)),
            (201, (1, 1)),
            (201, (2, 1)),
        ]
    );
    assert_eq!(
        options.last(),
        Some(&CombatAction::EndTurn { actor }),
        "EndTurn is last"
    );
    for option in &options {
        assert!(executes(&world, option).is_ok(), "{option:?} must execute");
    }
}

/// A crowd fixture: `participants` clones of creature A, the first at
/// initiative 0 (it acts first), and `abilities` self-allowed abilities.
fn crowd(participants: usize, abilities: usize) -> Fixture {
    let mut fixture = Fixture::standard();
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
                initiative: if index == 0 { 0 } else { 10 },
            });
    }
    fixture
}

#[test]
fn end_turn_is_last_and_counts_toward_limit() {
    // 5 abilities x 819 targets = 4095, plus EndTurn = exactly 4096.
    let (world, ids) = crowd(819, 5).start(1);
    let actor = ids[0];
    assert_eq!(active(&world), actor);
    let options = legal_actions(&world, actor).expect("exactly the limit succeeds");
    assert_eq!(options.len(), MAX_LEGAL_ACTIONS);
    assert_eq!(options.last(), Some(&CombatAction::EndTurn { actor }));
    assert!(options[..MAX_LEGAL_ACTIONS - 1]
        .iter()
        .all(|option| matches!(option, CombatAction::UseAbility { .. })));

    // 4 abilities x 1024 targets = 4096 UseAbility options: EndTurn would be
    // the 4097th, so the whole query fails with no partial list.
    let (world, ids) = crowd(1024, 4).start(1);
    let actor = ids[0];
    let bytes = snapshot(&world);
    let hash = state_hash(&world);
    let error = legal_actions(&world, actor).expect_err("4097 options exceed the bound");
    assert_eq!(error, LegalActionsError::TooManyOptions { limit: 4096 });
    assert_eq!(error.to_string(), "TooManyOptions at combat/options");
    assert!(error.source().is_none());
    assert_unchanged(&world, &bytes, hash, "over-limit query");
    // Every one of those candidates is individually executable.
    let last = ids[ids.len() - 1];
    assert!(executes(&world, &use_ability(actor, ability(403), last)).is_ok());
}

#[test]
fn stale_options_revalidate() {
    // Spend: non-ending cost-1 ability over a max-1 pool that never refills.
    let mut fixture = Fixture::trio();
    {
        let id = fixture.ability;
        let ability = fixture.abilities.get_mut(&id).expect("ability");
        ability.ends_turn = false;
    }
    fixture.ruleset.pools[0].refresh = RefreshWire::Never;
    let (mut world, ids) = fixture.start(13);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    assert_eq!(active(&world), c);
    let options = legal_actions(&world, c).expect("C may act");
    let attack = use_ability(c, fixture.ability, a);
    assert!(options.contains(&attack));
    perform_action(&mut world, &attack).expect("affordable once");
    let bytes = snapshot(&world);
    let hash = state_hash(&world);
    let mut probe = world.clone();
    assert_eq!(
        perform_action(&mut probe, &attack),
        Err(CombatError::InsufficientAction {
            pool: primary(),
            cost: 1,
            current: 0,
        })
    );
    assert_eq!(bytes, snapshot(&probe));
    assert_eq!(hash, state_hash(&probe));

    // Advance: the stale EndTurn is out of turn once the turn has passed.
    let end = CombatAction::EndTurn { actor: c };
    assert!(options.contains(&end));
    perform_action(&mut world, &end).expect("C ends its turn");
    let bytes = snapshot(&world);
    let mut probe = world.clone();
    assert_eq!(
        perform_action(&mut probe, &end),
        Err(CombatError::OutOfTurn {
            actor: c,
            active: Some(b),
        })
    );
    assert_eq!(bytes, snapshot(&probe));

    // Despawn: B's option against A goes absent when A leaves.
    let options = legal_actions(&world, b).expect("B may act");
    let hit_a = use_ability(b, fixture.ability, a);
    assert!(options.contains(&hit_a));
    assert!(world.despawn(a));
    let bytes = snapshot(&world);
    let mut probe = world.clone();
    assert_eq!(
        perform_action(&mut probe, &hit_a),
        Err(CombatError::AbsentTarget { target: a })
    );
    assert_eq!(bytes, snapshot(&probe));
}

#[test]
fn actor_errors_and_unaffordable_control() {
    // No encounter.
    let mut empty = World::new(2);
    let stray = empty.spawn(EntityMeta {});
    assert_eq!(
        legal_actions(&empty, stray),
        Err(LegalActionsError::Actor(CombatError::NoEncounter))
    );

    // Two combatants, B at health 2, every roll succeeds.
    let mut fixture = matrix();
    fixture.encounter.participants.pop();
    let creature_b = fixture.creature_b;
    fixture
        .creatures
        .get_mut(&creature_b)
        .expect("B")
        .stats
        .insert("health".to_owned(), crpg_core::Fx16_16::from_int(2));
    let mut world = World::new(4);
    let outsider = world.spawn(EntityMeta {});
    let gone = world.spawn(EntityMeta {});
    assert!(world.despawn(gone));
    let ids = start_in(&fixture, &mut world);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(active(&world), b);

    let error = legal_actions(&world, outsider).expect_err("nonparticipant");
    assert_eq!(
        error,
        LegalActionsError::Actor(CombatError::NotParticipant { entity: outsider })
    );
    assert_eq!(
        error.to_string(),
        CombatError::NotParticipant { entity: outsider }.to_string(),
        "Actor displays the wrapped error unchanged"
    );
    let source = error.source().expect("Actor exposes its source");
    assert_eq!(
        source.to_string(),
        CombatError::NotParticipant { entity: outsider }.to_string()
    );
    assert_eq!(
        legal_actions(&world, gone),
        Err(LegalActionsError::Actor(CombatError::AbsentActor {
            actor: gone
        }))
    );
    assert_eq!(
        legal_actions(&world, a),
        Err(LegalActionsError::Actor(CombatError::OutOfTurn {
            actor: a,
            active: Some(b),
        }))
    );

    // B kills A (health 10 takes five hits; A answers at B, health 2, so
    // drive B's first hit onto A, then A's reply kills B).
    perform_action(&mut world, &use_ability(b, ability(103), a)).expect("hit");
    assert_eq!(active(&world), a);
    perform_action(&mut world, &use_ability(a, ability(103), b)).expect("kill");
    assert!(world.combatants().get(b).expect("B").dead());
    // Terminal: nobody holds the turn.
    assert_eq!(world.combat().expect("retained").active, None);
    assert_eq!(
        legal_actions(&world, b),
        Err(LegalActionsError::Actor(CombatError::DeadActor {
            actor: b
        }))
    );
    assert_eq!(
        legal_actions(&world, a),
        Err(LegalActionsError::Actor(CombatError::OutOfTurn {
            actor: a,
            active: None,
        }))
    );

    // A valid actor whose every ability is unaffordable still has EndTurn.
    let mut fixture = Fixture::standard();
    {
        let id = fixture.ability;
        fixture.abilities.get_mut(&id).expect("ability").ends_turn = false;
    }
    fixture.ruleset.pools[0].refresh = RefreshWire::Never;
    let (mut world, ids) = fixture.start(6);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_ability(b, fixture.ability, a)).expect("spend");
    assert_eq!(
        legal_actions(&world, b),
        Ok(vec![CombatAction::EndTurn { actor: b }])
    );
    assert_eq!(
        executes(&world, &use_ability(b, fixture.ability, a)),
        Err(CombatError::InsufficientAction {
            pool: primary(),
            cost: 1,
            current: 0,
        })
    );
}
