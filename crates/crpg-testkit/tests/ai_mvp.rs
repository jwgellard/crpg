#![forbid(unsafe_code)]

//! G1-G22: the MVP decision over real turn-based and real-time encounters.
//!
//! Every value below is `tasks/T056.md`'s "T056b: crates/crpg-testkit/tests/
//! ai_mvp.rs" literal, and every decision is applied through the sim verbs the
//! host uses (`perform_action`, `perform_move` over `host_route`'s route, and
//! `set_route`). The fixture is the contract's own authored-document fixture.

mod ai_fixture;

use ai_fixture::{
    apply_move, bind, extra_cost, far, lethal, m, moving, moving_rt, ms, near, p, place_c, ranged,
    rev, reversed_abilities, rp, shared, sides, std, step_cost, trio, tune, twin, ulid,
    use_ability, R15, R3,
};
use crpg_ai::{decide, host_route, Decision, Sides};
use crpg_core::EntityId;
use crpg_nav::NavMesh;
use crpg_sim::{
    legal_actions, perform_action, state_hash, tick, validate_action, CombatAction, CombatError,
    EntityMeta, LegalActionsError, MoveOutcome, MoveRoute, RouteTag, StandingOrder, World,
};

/// G13's check: `decide` reads only — `state_hash` and `World` value equal
/// before and after, two calls equal, and the same answer on a clone (M-9).
fn assert_pure(world: &World, actor: EntityId, sides: &Sides, nav: Option<&NavMesh>) {
    let before = world.clone();
    let hash = state_hash(world);
    let first = decide(world, actor, sides, nav);
    assert_eq!(state_hash(world), hash);
    assert_eq!(world, &before);
    let second = decide(world, actor, sides, nav);
    assert_eq!(second, first);
    assert_eq!(decide(&before, actor, sides, nav), first);
}

/// G19's reference rule: the first `UseAbility` of `L(x)` whose target is not
/// `x`, else the last option.
fn reference_rule(options: &[CombatAction], actor: EntityId) -> CombatAction {
    options
        .iter()
        .find(
            |option| matches!(option, CombatAction::UseAbility { target, .. } if *target != actor),
        )
        .copied()
        .unwrap_or_else(|| *options.last().expect("the list closes with EndTurn"))
}

// ---------------------------------------------------------------------------
// Turn-based (G1-G13, G20).
// ---------------------------------------------------------------------------

#[test]
fn g1_tb_attacks_the_nearest_attackable() {
    let fixture = trio(moving());
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    let split = sides(&[(a, 1), (b, 2), (c, 1)]);

    // dist(A) is 655360, dist(C) is 926820.
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Act(use_ability(b, 103, a)))
    );

    // Control: C at 327680 from B; L(B) lists A first and C second.
    world.place(c, p(10, 5)).expect("C has a mover");
    assert_eq!(
        legal_actions(&world, b).expect("B may act"),
        vec![
            use_ability(b, 103, a),
            use_ability(b, 103, c),
            CombatAction::EndTurn { actor: b },
        ]
    );
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Act(use_ability(b, 103, c)))
    );
}

#[test]
fn g2_tb_tie_breaks_by_entity_id() {
    let fixture = trio(moving());
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    let split = sides(&[(a, 1), (b, 2), (c, 1)]);

    // Both hostiles are 655360 away; the lower EntityId wins.
    world.place(c, p(10, 10)).expect("C has a mover");
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Act(use_ability(b, 103, a)))
    );

    // Control: A one metre left is 720896 away, so C now wins.
    world.place(a, p(-1, 0)).expect("A has a mover");
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Act(use_ability(b, 103, c)))
    );
}

#[test]
fn g3_tb_same_side_is_not_hostile() {
    let fixture = trio(moving());
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    world.place(c, p(10, 5)).expect("C has a mover");

    // C is B's ally, so the farther A is the only hostile.
    assert_eq!(
        decide(&world, b, &sides(&[(a, 1), (b, 2), (c, 2)]), Some(&mesh)),
        Ok(Decision::Act(use_ability(b, 103, a)))
    );

    // Controls: C on B's enemy side is attacked; an unlisted C is hostile to
    // no one, so A is attacked again.
    assert_eq!(
        decide(&world, b, &sides(&[(a, 1), (b, 2), (c, 1)]), Some(&mesh)),
        Ok(Decision::Act(use_ability(b, 103, c)))
    );
    assert_eq!(
        decide(&world, b, &sides(&[(a, 1), (b, 2)]), Some(&mesh)),
        Ok(Decision::Act(use_ability(b, 103, a)))
    );
}

#[test]
fn g4_tb_first_ability_wins() {
    let fixture = twin(moving());
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    assert_eq!(
        legal_actions(&world, b).expect("B may act"),
        vec![
            CombatAction::UseAbility {
                actor: b,
                ability: ulid(103),
                target: a,
            },
            CombatAction::UseAbility {
                actor: b,
                ability: ulid(107),
                target: a,
            },
            CombatAction::EndTurn { actor: b },
        ]
    );
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Act(use_ability(b, 103, a)))
    );
}

#[test]
fn g5_tb_approach_then_attack() {
    let fixture = ranged(moving(), R15);
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    assert_eq!(
        legal_actions(&world, b).expect("B may act"),
        vec![CombatAction::EndTurn { actor: b }]
    );
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(98_240, 0, 0),
        })
    );
    assert_eq!(
        host_route(&mesh, &world, b, rp(98_240, 0, 0)),
        Ok(MoveRoute {
            revision: rev(&mesh),
            tag: RouteTag(0),
            points: vec![rp(655_360, 0, 0), rp(98_240, 0, 0)],
        })
    );
    let outcome = apply_move(&mut world, &mesh, b, rp(98_240, 0, 0)).expect("the move is accepted");
    assert_eq!(
        outcome,
        MoveOutcome {
            actor: b,
            cost: 6,
            from: rp(655_360, 0, 0),
            to: rp(98_240, 0, 0),
        }
    );
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Act(use_ability(b, 103, a)))
    );

    // Control: with no range the attack is legal at once and no move happens.
    let fixture = moving();
    let (mut world, ids) = fixture.start(7);
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        decide(&world, b, &sides(&[(a, 1), (b, 2)]), Some(&mesh)),
        Ok(Decision::Act(use_ability(b, 103, a)))
    );
}

#[test]
fn g6_tb_far_target_spends_the_budget() {
    let fixture = ranged(far(moving()), R15);
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    // d = min(1310720 - 98240, 6 * 98304) = 589824.
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(720_896, 0, 0),
        })
    );
    let outcome =
        apply_move(&mut world, &mesh, b, rp(720_896, 0, 0)).expect("the move is accepted");
    assert_eq!(outcome.cost, 6);
    // Pool 106 is empty, so the second call cannot move.
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Act(CombatAction::EndTurn { actor: b }))
    );
}

#[test]
fn g7_tb_shared_pool_keeps_the_attack() {
    let fixture = shared(ranged(far(moving()), R15));
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    // Balance 7 less the attack's 1 leaves six steps.
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(720_896, 0, 0),
        })
    );

    // Control: a near target walks the standoff, then attacks from the same
    // shared pool.
    let fixture = shared(ranged(moving(), R15));
    let (mut world, ids) = fixture.start(7);
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(98_240, 0, 0),
        })
    );
    let outcome = apply_move(&mut world, &mesh, b, rp(98_240, 0, 0)).expect("accepted");
    assert_eq!(outcome.cost, 6);
    assert_eq!(
        world
            .combatants()
            .get(b)
            .expect("B exists")
            .action_pool()
            .current(),
        1
    );
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Act(use_ability(b, 103, a)))
    );
}

#[test]
fn g8_tb_unreachable_ends_the_turn() {
    let fixture = ranged(moving(), R15);
    let (mut world, ids) = fixture.start(7);
    let wall = ms();
    bind(&mut world, &wall);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    // The halves of MS do not touch, so `find_path` is `Unreachable`.
    assert_eq!(
        decide(&world, b, &split, Some(&wall)),
        Ok(Decision::Act(CombatAction::EndTurn { actor: b }))
    );

    // Control: on M the same world walks.
    let mesh = m();
    bind(&mut world, &mesh);
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(98_240, 0, 0),
        })
    );
}

#[test]
fn g9_tb_no_mesh_or_unbound_ends_the_turn() {
    let fixture = ranged(moving(), R15);
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    assert_eq!(
        decide(&world, b, &split, None),
        Ok(Decision::Act(CombatAction::EndTurn { actor: b }))
    );
    // `host_route` on an unbound world is `NavUnbound`, so no move.
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Act(CombatAction::EndTurn { actor: b }))
    );

    // Control: bound and given the same mesh, the move is returned.
    bind(&mut world, &mesh);
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(98_240, 0, 0),
        })
    );
}

#[test]
fn g10_tb_dead_hostile_is_ignored() {
    let fixture = trio(lethal(ranged(near(moving()), R15)));
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    let split = sides(&[(a, 1), (b, 2), (c, 3)]);

    // A is 65536 away; C at 658629 is out of range.
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Act(use_ability(b, 103, a)))
    );
    assert!(perform_action(&mut world, &use_ability(b, 103, a))
        .expect("the attack is accepted")
        .is_some());
    assert!(world.combatants().get(a).expect("A exists").dead());
    assert_eq!(world.combat().and_then(|state| state.active), Some(c));

    // The dead, nearer A is not the approach target.
    assert_eq!(
        decide(&world, c, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: b,
            destination: rp(55_760, 0, 97_753),
        })
    );
    assert_eq!(
        host_route(&mesh, &world, c, rp(55_760, 0, 97_753)),
        Ok(MoveRoute {
            revision: rev(&mesh),
            tag: RouteTag(0),
            points: vec![rp(0, 0, 655_360), rp(55_760, 0, 97_753)],
        })
    );
    let outcome =
        apply_move(&mut world, &mesh, c, rp(55_760, 0, 97_753)).expect("the move is accepted");
    assert_eq!(outcome.cost, 6);
    assert_eq!(
        decide(&world, c, &split, Some(&mesh)),
        Ok(Decision::Act(use_ability(c, 103, b)))
    );
}

#[test]
fn g11_tb_errors_pass_through() {
    let fixture = moving();
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    assert_eq!(
        decide(&world, a, &split, Some(&mesh)),
        Err(LegalActionsError::Actor(CombatError::OutOfTurn {
            actor: a,
            active: Some(b),
        }))
    );
    let bystander = world.spawn(EntityMeta {});
    assert_eq!(
        decide(&world, bystander, &split, Some(&mesh)),
        Err(LegalActionsError::Actor(CombatError::NotParticipant {
            entity: bystander,
        }))
    );

    // Control: the actor holding the turn still decides.
    assert!(decide(&world, b, &split, Some(&mesh)).is_ok());
}

#[test]
fn g12_tb_no_hostile_ends_the_turn() {
    let fixture = moving();
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);

    assert_eq!(
        decide(&world, b, &sides(&[(a, 1), (b, 1)]), Some(&mesh)),
        Ok(Decision::Act(CombatAction::EndTurn { actor: b }))
    );
    assert_eq!(
        decide(&world, b, &Sides::default(), Some(&mesh)),
        Ok(Decision::Act(CombatAction::EndTurn { actor: b }))
    );

    // Control: one differing side makes A a hostile and the attack appears.
    assert_eq!(
        decide(&world, b, &sides(&[(a, 1), (b, 2)]), Some(&mesh)),
        Ok(Decision::Act(use_ability(b, 103, a)))
    );
}

#[test]
fn g13_decide_is_pure() {
    // G5's world.
    let fixture = ranged(moving(), R15);
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    assert_pure(&world, b, &sides(&[(a, 1), (b, 2)]), Some(&mesh));

    // G10's world, before the attack.
    let fixture = trio(lethal(ranged(near(moving()), R15)));
    let (mut world, ids) = fixture.start(7);
    bind(&mut world, &mesh);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    assert_pure(&world, b, &sides(&[(a, 1), (b, 2), (c, 3)]), Some(&mesh));

    // G14's world.
    let fixture = ranged(moving_rt(), R15);
    let (mut world, ids) = fixture.start_rt(7);
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    assert_pure(&world, a, &sides(&[(a, 1), (b, 2)]), Some(&mesh));
}

#[test]
fn g20_tb_intended_ability_is_the_least_out_of_range() {
    // One shared pool 105 (max 7): 103 has range 1.5 m and costs 4, 107 has
    // range 3 m and costs 2.
    let tw = tune(tune(shared(twin(moving())), 103, R15, 4), 107, R3, 2);
    let (mut world, ids) = tw.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    // Case 1: both out of range, so I is 103 by least ULID.
    assert_eq!(
        legal_actions(&world, b).expect("B may act"),
        vec![CombatAction::EndTurn { actor: b }]
    );
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(360_448, 0, 0),
        })
    );
    let outcome =
        apply_move(&mut world, &mesh, b, rp(360_448, 0, 0)).expect("the move is accepted");
    assert_eq!(outcome.cost, 3);
    assert_eq!(
        world
            .combatants()
            .get(b)
            .expect("B exists")
            .action_pool()
            .current(),
        4
    );
    // 103 is still affordable and out of range, but the reserve leaves no step.
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Act(CombatAction::EndTurn { actor: b }))
    );

    // Case 2: 103 fails InsufficientAction first, so I is 107.
    let (mut world, ids) = tw.start(7);
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    assert!(apply_move(&mut world, &mesh, b, rp(262_144, 0, 0)).is_some());
    assert_eq!(
        world
            .combatants()
            .get(b)
            .expect("B exists")
            .action_pool()
            .current(),
        3
    );
    assert_eq!(
        legal_actions(&world, b).expect("B may act"),
        vec![CombatAction::EndTurn { actor: b }]
    );
    assert_eq!(
        validate_action(&world, &use_ability(b, 103, a)),
        Err(CombatError::InsufficientAction {
            pool: ulid(105),
            cost: 4,
            current: 3,
        })
    );
    assert_eq!(
        validate_action(
            &world,
            &CombatAction::UseAbility {
                actor: b,
                ability: ulid(107),
                target: a,
            }
        ),
        Err(CombatError::OutOfRange)
    );
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(196_544, 0, 0),
        })
    );
    let outcome =
        apply_move(&mut world, &mesh, b, rp(196_544, 0, 0)).expect("the move is accepted");
    assert_eq!(outcome.cost, 1);
    assert_eq!(
        world
            .combatants()
            .get(b)
            .expect("B exists")
            .action_pool()
            .current(),
        2
    );
    assert_eq!(
        legal_actions(&world, b).expect("B may act"),
        vec![use_ability(b, 107, a), CombatAction::EndTurn { actor: b },]
    );
    // 103 is still unaffordable (2 < 4), so 107 is the legal attack.
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Act(use_ability(b, 107, a)))
    );
}

// ---------------------------------------------------------------------------
// Real time (G14-G18, G21-G22).
// ---------------------------------------------------------------------------

#[test]
fn g14_rt_approach() {
    let fixture = ranged(moving_rt(), R15);
    let (mut world, ids) = fixture.start_rt(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    assert_eq!(
        legal_actions(&world, a).expect("A may act"),
        vec![CombatAction::Stop { actor: a }]
    );
    assert_eq!(
        decide(&world, a, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: b,
            destination: rp(557_120, 0, 0),
        })
    );

    // Control: in range and with no route, the attack is the order.
    let fixture = ranged(near(moving_rt()), R15);
    let (mut world, ids) = fixture.start_rt(7);
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        decide(&world, a, &sides(&[(a, 1), (b, 2)]), Some(&mesh)),
        Ok(Decision::Act(use_ability(a, 103, b)))
    );
}

#[test]
fn g15_rt_hold_then_replan() {
    let fixture = ranged(moving_rt(), R15);
    let (mut world, ids) = fixture.start_rt(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    assert!(apply_move(&mut world, &mesh, a, rp(557_120, 0, 0)).is_none());
    tick(&mut world);
    assert_eq!(
        world.movers().get(a).expect("A has a mover").position(),
        rp(65_536, 0, 0)
    );
    // The standing route ends 98240 from B, inside R15.
    assert_eq!(decide(&world, a, &split, Some(&mesh)), Ok(Decision::Hold));

    // The target moved to x = 20 m, so the walk no longer ends in range.
    world.place(b, p(20, 0)).expect("B has a mover");
    assert_eq!(
        decide(&world, a, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: b,
            destination: rp(1_212_480, 0, 0),
        })
    );
}

#[test]
fn g16_rt_attack_halts_first_and_never_reissues() {
    let fixture = near(moving_rt());
    let (mut world, ids) = fixture.start_rt(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    assert_eq!(
        legal_actions(&world, a).expect("A may act"),
        vec![use_ability(a, 103, b), CombatAction::Stop { actor: a }]
    );
    // Walking while the attack is available: halt before striking.
    assert!(apply_move(&mut world, &mesh, a, p(0, 5)).is_none());
    assert_eq!(decide(&world, a, &split, Some(&mesh)), Ok(Decision::Halt));

    world.clear_route(a).expect("A has a mover");
    assert_eq!(
        decide(&world, a, &split, Some(&mesh)),
        Ok(Decision::Act(use_ability(a, 103, b)))
    );
    assert_eq!(
        perform_action(&mut world, &use_ability(a, 103, b)).expect("the order is accepted"),
        None
    );
    assert_eq!(
        world.combatants().get(a).expect("A exists").order(),
        Some(StandingOrder {
            ability: ulid(103),
            target: b,
            repeat: false,
        })
    );
    // The same order is never re-issued.
    assert_eq!(decide(&world, a, &split, Some(&mesh)), Ok(Decision::Hold));

    // Control: another ability's standing order does not hold `k`'s.
    let fixture = twin(ranged(near(moving_rt()), R15));
    let (mut world, ids) = fixture.start_rt(7);
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        legal_actions(&world, a).expect("A may act"),
        vec![
            use_ability(a, 103, b),
            use_ability(a, 107, b),
            CombatAction::Stop { actor: a },
        ]
    );
    assert_eq!(
        perform_action(&mut world, &use_ability(a, 107, b)).expect("the order is accepted"),
        None
    );
    assert_eq!(
        world.combatants().get(a).expect("A exists").order(),
        Some(StandingOrder {
            ability: ulid(107),
            target: b,
            repeat: false,
        })
    );
    assert_eq!(
        decide(&world, a, &sides(&[(a, 1), (b, 2)]), Some(&mesh)),
        Ok(Decision::Act(use_ability(a, 103, b)))
    );
}

#[test]
fn g17_rt_no_hostile() {
    let fixture = near(moving_rt());
    let (mut world, ids) = fixture.start_rt(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let same = sides(&[(a, 1), (b, 1)]);

    assert_eq!(decide(&world, a, &same, Some(&mesh)), Ok(Decision::Hold));

    // Sides do not bind sim: the order is accepted even between allies.
    assert_eq!(
        perform_action(&mut world, &use_ability(a, 103, b)).expect("the order is accepted"),
        None
    );
    assert_eq!(
        decide(&world, a, &same, Some(&mesh)),
        Ok(Decision::Act(CombatAction::Stop { actor: a }))
    );
    assert_eq!(
        perform_action(&mut world, &CombatAction::Stop { actor: a }).expect("the stop is accepted"),
        None
    );
    assert!(apply_move(&mut world, &mesh, a, p(0, 5)).is_none());
    assert_eq!(decide(&world, a, &same, Some(&mesh)), Ok(Decision::Halt));
}

#[test]
fn g18_rt_unreachable_holds() {
    let fixture = ranged(moving_rt(), R15);
    let (mut world, ids) = fixture.start_rt(7);
    let wall = ms();
    bind(&mut world, &wall);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    // No route and no path: the no-intent fallback holds.
    assert_eq!(decide(&world, a, &split, Some(&wall)), Ok(Decision::Hold));
    // A route the AI measured as short of range, with no replacement: halt.
    assert!(apply_move(&mut world, &wall, a, p(0, 5)).is_none());
    assert_eq!(decide(&world, a, &split, Some(&wall)), Ok(Decision::Halt));

    // Control: on M the same world walks.
    let mesh = m();
    bind(&mut world, &mesh);
    assert_eq!(
        decide(&world, a, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: b,
            destination: rp(557_120, 0, 0),
        })
    );
}

#[test]
fn g19_full_fight_matches_reference() {
    let (mut world, ids) = std().start(7);
    let (mut reference, reference_ids) = std().start(7);
    assert_eq!(ids, reference_ids);
    let split = sides(&[(ids[0], 1), (ids[1], 2)]);

    let mut iterations = 0_u32;
    while let Some(actor) = world.combat().and_then(|state| state.active) {
        iterations += 1;
        assert!(iterations <= 1_000, "the fight ends");
        let options = legal_actions(&world, actor).expect("the active actor may act");
        let Decision::Act(action) = decide(&world, actor, &split, None).expect("the actor decides")
        else {
            panic!("turn-based decisions are actions");
        };
        assert!(options.contains(&action), "the decision is a listed option");
        perform_action(&mut world, &action).expect("the option is executable");

        let reference_actor = reference
            .combat()
            .and_then(|state| state.active)
            .expect("both worlds hold the same turn");
        assert_eq!(reference_actor, actor);
        let reference_options =
            legal_actions(&reference, reference_actor).expect("the reference actor may act");
        perform_action(
            &mut reference,
            &reference_rule(&reference_options, reference_actor),
        )
        .expect("the reference option is executable");
    }

    assert!(iterations > 0);
    assert_eq!(reference.combat().and_then(|state| state.active), None);
    assert_eq!(state_hash(&world), state_hash(&reference));
}

#[test]
fn g21_rt_no_intent_keeps_walking() {
    // Case 1: the order spends the pool, then every ability fails
    // affordability before the range stage, so no ability is the intent.
    let fixture = ranged(near(moving_rt()), R15);
    let (mut world, ids) = fixture.start_rt(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    assert_eq!(
        legal_actions(&world, a).expect("A may act"),
        vec![use_ability(a, 103, b), CombatAction::Stop { actor: a }]
    );
    assert_eq!(
        perform_action(&mut world, &use_ability(a, 103, b)).expect("the order is accepted"),
        None
    );
    tick(&mut world);
    assert_eq!(
        world
            .combat()
            .and_then(|state| state.clock)
            .map(|c| c.elapsed),
        Some(1)
    );
    let actor_state = world.combatants().get(a).expect("A exists").clone();
    assert_eq!(actor_state.action_pool().current(), 0);
    assert_eq!(actor_state.order(), None);
    // One hit for at most 2 damage leaves the 6-health B alive.
    let target_state = world.combatants().get(b).expect("B exists").clone();
    assert!(!target_state.dead());
    assert!(target_state.health() >= 4);

    world.place(b, p(10, 0)).expect("B has a mover");
    assert_eq!(
        legal_actions(&world, a).expect("A may act"),
        vec![CombatAction::Stop { actor: a }]
    );
    assert_eq!(
        validate_action(&world, &use_ability(a, 103, b)),
        Err(CombatError::InsufficientAction {
            pool: ulid(105),
            cost: 1,
            current: 0,
        })
    );
    assert_eq!(decide(&world, a, &split, Some(&mesh)), Ok(Decision::Hold));

    assert!(apply_move(&mut world, &mesh, a, p(0, 5)).is_none());
    assert_eq!(decide(&world, a, &split, Some(&mesh)), Ok(Decision::Hold));
    assert_eq!(
        world
            .movers()
            .get(a)
            .expect("A has a mover")
            .route()
            .expect("the order stands")
            .route()
            .points,
        vec![rp(0, 0, 0), rp(0, 0, 327_680)]
    );

    // Control: no hostile leaves the standing route to `Halt`.
    assert_eq!(
        decide(&world, a, &sides(&[(a, 1), (b, 1)]), Some(&mesh)),
        Ok(Decision::Halt)
    );

    // Case 2: a negative range is out of range even at 65536, and the
    // negative `r_i32` is no intent — never `u64` near 2^64.
    let fixture = ranged(near(moving_rt()), -1);
    let (mut world, ids) = fixture.start_rt(7);
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        legal_actions(&world, a).expect("A may act"),
        vec![CombatAction::Stop { actor: a }]
    );
    assert!(apply_move(&mut world, &mesh, a, p(0, 5)).is_none());
    assert_eq!(
        decide(&world, a, &sides(&[(a, 1), (b, 2)]), Some(&mesh)),
        Ok(Decision::Hold)
    );
}

#[test]
fn g22_rt_no_mesh_or_unbound() {
    let fixture = ranged(moving_rt(), R15);
    let mesh = m();
    let split_target = |ids: &[EntityId]| sides(&[(ids[0], 1), (ids[1], 2)]);

    // (a) Bound, no mesh, no route: nothing to measure or replace.
    let (mut world, ids) = fixture.start_rt(7);
    assert_eq!(world.nav(), None);
    bind(&mut world, &mesh);
    assert_eq!(world.nav(), Some(rev(&mesh)));
    assert_eq!(
        decide(&world, ids[0], &split_target(&ids), None),
        Ok(Decision::Hold)
    );

    // (b) Unbound with a mesh: `host_route` is `NavUnbound`.
    let (world, ids) = fixture.start_rt(7);
    assert_eq!(world.nav(), None);
    assert_eq!(
        decide(&world, ids[0], &split_target(&ids), Some(&mesh)),
        Ok(Decision::Hold)
    );

    // Control: bound, given the mesh, no route: G14's move.
    let (mut world, ids) = fixture.start_rt(7);
    bind(&mut world, &mesh);
    assert_eq!(
        decide(&world, ids[0], &split_target(&ids), Some(&mesh)),
        Ok(Decision::Move {
            target: ids[1],
            destination: rp(557_120, 0, 0),
        })
    );

    // (c) A route ending 732715 from B is short of range: halt.
    let (mut world, ids) = fixture.start_rt(7);
    bind(&mut world, &mesh);
    assert!(apply_move(&mut world, &mesh, ids[0], p(0, 5)).is_none());
    assert_eq!(
        decide(&world, ids[0], &split_target(&ids), None),
        Ok(Decision::Halt)
    );

    // (d) A route ending 98240 from B is inside R15: hold before the mesh.
    assert!(apply_move(&mut world, &mesh, ids[0], rp(557_120, 0, 0)).is_none());
    assert_eq!(
        decide(&world, ids[0], &split_target(&ids), None),
        Ok(Decision::Hold)
    );
}

// ---------------------------------------------------------------------------
// Fix round 1: the review's mutant gaps (G23-G34).
// ---------------------------------------------------------------------------

#[test]
fn g23_tb_approach_target_is_the_nearest_hostile() {
    // M-5: T is the least (dist, EntityId) over H — nearer first, then id.
    let fixture = ranged(trio(moving()), R15);
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    let split = sides(&[(a, 1), (b, 2), (c, 1)]);

    // Both hostiles are out of range: C at 327680 is nearer than A at 655360,
    // so the higher id is the approach target.
    world.place(c, p(10, 5)).expect("C has a mover");
    assert_eq!(
        legal_actions(&world, b).expect("B may act"),
        vec![CombatAction::EndTurn { actor: b }]
    );
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: c,
            destination: rp(655_360, 0, 229_440),
        })
    );

    // Both are 655360 away, so the lower EntityId A wins the tie.
    world.place(c, p(20, 0)).expect("C has a mover");
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(98_240, 0, 0),
        })
    );

    // Control: A one metre left is 720896 away, so C is now the nearer.
    world.place(a, p(-1, 0)).expect("A has a mover");
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: c,
            destination: rp(1_212_480, 0, 0),
        })
    );
}

#[test]
fn g24_rt_no_hostile_stops_before_halting() {
    // M-3: with no hostile a standing order is cleared with `Stop` before the
    // route falls to `Halt`.
    let fixture = near(moving_rt());
    let (mut world, ids) = fixture.start_rt(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let same = sides(&[(a, 1), (b, 1)]);

    assert!(apply_move(&mut world, &mesh, a, p(0, 5)).is_none());
    assert_eq!(
        perform_action(&mut world, &use_ability(a, 103, b)).expect("the order is accepted"),
        None
    );
    assert!(world
        .combatants()
        .get(a)
        .expect("A exists")
        .order()
        .is_some());
    assert!(world
        .movers()
        .get(a)
        .expect("A has a mover")
        .route()
        .is_some());
    assert_eq!(
        decide(&world, a, &same, Some(&mesh)),
        Ok(Decision::Act(CombatAction::Stop { actor: a }))
    );

    // Control: with the order cleared the same route gives `Halt`, not `Hold`.
    assert_eq!(
        perform_action(&mut world, &CombatAction::Stop { actor: a }).expect("the stop is accepted"),
        None
    );
    assert_eq!(decide(&world, a, &same, Some(&mesh)), Ok(Decision::Halt));
}

#[test]
fn g25_rt_attack_halts_a_standing_route() {
    // M-4: with an attack available a route gives `Halt`; the same order
    // without a route gives `Hold`.
    let fixture = near(moving_rt());
    let (mut world, ids) = fixture.start_rt(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    assert_eq!(
        perform_action(&mut world, &use_ability(a, 103, b)).expect("the order is accepted"),
        None
    );
    assert!(apply_move(&mut world, &mesh, a, p(0, 5)).is_none());
    assert_eq!(
        world.combatants().get(a).expect("A exists").order(),
        Some(StandingOrder {
            ability: ulid(103),
            target: b,
            repeat: false,
        })
    );
    assert_eq!(decide(&world, a, &split, Some(&mesh)), Ok(Decision::Halt));

    // Control: the same order with no route is the never-re-issued hold.
    world.clear_route(a).expect("A has a mover");
    assert_eq!(decide(&world, a, &split, Some(&mesh)), Ok(Decision::Hold));
}

#[test]
fn g26_rt_hold_is_only_for_the_order_of_k() {
    // M-4: the standing order must be k's, same ability and same hostile.
    let fixture = trio(near(moving_rt()));
    let (mut world, ids) = fixture.start_rt(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    let split = sides(&[(a, 1), (b, 2), (c, 2)]);

    assert_eq!(
        legal_actions(&world, a).expect("A may act"),
        vec![
            use_ability(a, 103, b),
            use_ability(a, 103, c),
            CombatAction::Stop { actor: a },
        ]
    );
    // The order is 103 on C: the same ability, but not k (103 on B).
    assert_eq!(
        perform_action(&mut world, &use_ability(a, 103, c)).expect("the order is accepted"),
        None
    );
    assert_eq!(
        decide(&world, a, &split, Some(&mesh)),
        Ok(Decision::Act(use_ability(a, 103, b)))
    );

    // Control: the order on B is k's, so the attack is not re-issued.
    let (mut world, ids) = fixture.start_rt(7);
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        perform_action(&mut world, &use_ability(a, 103, b)).expect("the order is accepted"),
        None
    );
    assert_eq!(decide(&world, a, &split, Some(&mesh)), Ok(Decision::Hold));
}

#[test]
fn g27_tb_zero_and_sub_margin_ranges() {
    // M-5(1)/M-7: range 0 is an intent (only a negative one is no intent),
    // and `remaining = r.saturating_sub(64)` saturates rather than underflow.
    let fixture = ranged(moving(), 0);
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    assert_eq!(
        legal_actions(&world, b).expect("B may act"),
        vec![CombatAction::EndTurn { actor: b }]
    );
    // r = 0: remaining 0, d = 655360 capped at 6 * 98304.
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(65_536, 0, 0),
        })
    );

    // Control: a positive range stops 64 raw inside it.
    let fixture = ranged(moving(), R15);
    let (mut world, ids) = fixture.start(7);
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(98_240, 0, 0),
        })
    );

    // A range below the 64-raw margin (40) saturates to the target's point.
    let fixture = ranged(near(moving()), 40);
    let (mut world, ids) = fixture.start(7);
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(0, 0, 0),
        })
    );

    // Control: at that distance a 65536 range is already in range.
    let fixture = ranged(near(moving()), 65_536);
    let (mut world, ids) = fixture.start(7);
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Act(use_ability(b, 103, a)))
    );
}

#[test]
fn g28_tb_step_cost_floors_the_units() {
    // M-7: units = (balance - reserve) / step_cost, integer floor.
    // One shared pool 105 (max 7), 103 range 1.5 m and cost 2.
    let fixture = step_cost(tune(shared(moving()), 103, R15, 2), 3);
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    // balance 7, reserve 2 → 5 / 3 = 1 unit = 98304 raw.
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(557_056, 0, 0),
        })
    );
    let outcome = apply_move(&mut world, &mesh, b, rp(557_056, 0, 0)).expect("accepted");
    assert_eq!(outcome.cost, 3);
    assert_eq!(
        world
            .combatants()
            .get(b)
            .expect("B exists")
            .action_pool()
            .current(),
        4
    );

    // Control: step_cost 2 → 5 / 2 = 2 units = 196608 raw.
    let fixture = step_cost(tune(shared(moving()), 103, R15, 2), 2);
    let (mut world, ids) = fixture.start(7);
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(458_752, 0, 0),
        })
    );
}

#[test]
fn g29_rt_hold_inside_the_range_band() {
    // M-6: the hold compares the route's end with the full range, not
    // range - 64, against the target's current position.
    let fixture = ranged(moving_rt(), R15);
    let (mut world, ids) = fixture.start_rt(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    assert!(apply_move(&mut world, &mesh, a, rp(557_120, 0, 0)).is_none());
    // B moves 40 raw away, so the end is 98280 from it: still inside R15.
    world.place(b, rp(655_400, 0, 0)).expect("B has a mover");
    assert_eq!(decide(&world, a, &split, Some(&mesh)), Ok(Decision::Hold));

    // Control: 65 raw away is 98305, one raw beyond R15, so it re-plans.
    world.place(b, rp(655_425, 0, 0)).expect("B has a mover");
    assert_eq!(
        decide(&world, a, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: b,
            destination: rp(557_185, 0, 0),
        })
    );
}

#[test]
fn g30_off_mesh_target_routes_to_the_nearest_mesh_point() {
    // M-7's goal and R-1's goal: `nearest_on_mesh(snap(.., pos(T)))`, never
    // the target's own off-mesh position.
    let fixture = ranged(near(moving()), 0);
    let (mut world, ids) = fixture.start(7);
    let wall = ms();
    bind(&mut world, &wall);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    // A stands inside MS's wall at x = 5 m; the nearest mesh point is x = 4 m.
    world.place(a, p(5, 0)).expect("A has a mover");
    assert_eq!(
        host_route(&wall, &world, b, p(5, 0)),
        Ok(MoveRoute {
            revision: rev(&wall),
            tag: RouteTag(0),
            points: vec![rp(65_536, 0, 0), rp(262_144, 0, 0)],
        })
    );
    // r = 0 leaves no standoff, so the walk ends exactly at that point.
    assert_eq!(
        decide(&world, b, &split, Some(&wall)),
        Ok(Decision::Move {
            target: a,
            destination: rp(262_144, 0, 0),
        })
    );

    // Control: on the mesh at x = 3 m the same walk ends at the target.
    world.place(a, p(3, 0)).expect("A has a mover");
    assert_eq!(
        decide(&world, b, &split, Some(&wall)),
        Ok(Decision::Move {
            target: a,
            destination: rp(196_608, 0, 0),
        })
    );
}

#[test]
fn g31_tb_intended_ability_is_the_least_ulid() {
    // M-5: I is the least ability ULID of the definition, not the first one
    // the ruleset happens to list.
    let fixture = reversed_abilities(tune(tune(twin(moving()), 103, R15, 1), 107, R3, 1));
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    assert_eq!(fixture.ruleset.abilities, vec![ulid(107), ulid(103)]);
    // Both are out of range; I is 103 with R15, not the first-listed 107's R3.
    assert_eq!(
        legal_actions(&world, b).expect("B may act"),
        vec![CombatAction::EndTurn { actor: b }]
    );
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(98_240, 0, 0),
        })
    );

    // Control: swapping the ranges flips the destination while I stays 103.
    let fixture = reversed_abilities(tune(tune(twin(moving()), 103, R3, 1), 107, R15, 1));
    let (mut world, ids) = fixture.start(7);
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(196_544, 0, 0),
        })
    );
}

#[test]
fn g32_tb_reserve_sums_the_ability_costs() {
    // M-7: the reserve is the sum of I's costs in the movement pool.
    let fixture = extra_cost(shared(ranged(moving(), R15)), 103, 105, 2);
    let (mut world, ids) = fixture.start(7);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);

    // Primary 1 plus the extra 2 is a reserve of 3: units 4, 393216 raw.
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(262_144, 0, 0),
        })
    );
    let outcome = apply_move(&mut world, &mesh, b, rp(262_144, 0, 0)).expect("accepted");
    assert_eq!(outcome.cost, 4);
    assert_eq!(
        world
            .combatants()
            .get(b)
            .expect("B exists")
            .action_pool()
            .current(),
        3
    );

    // Control: without the extra entry the reserve is 1 and it walks further.
    let fixture = shared(ranged(moving(), R15));
    let (mut world, ids) = fixture.start(7);
    bind(&mut world, &mesh);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(98_240, 0, 0),
        })
    );
}

#[test]
fn g33_moverless_hostile_is_farther_than_any_mover() {
    // Reading 5: a pair without a mover on both sides has distance u64::MAX.
    let fixture = place_c(ranged(moving(), R15), 2, 0);
    let (mut world, ids) = fixture.start_with_c_record(7, false);
    let mesh = m();
    bind(&mut world, &mesh);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    assert!(
        world.movers().get(c).is_none(),
        "the member registered without movement has no mover"
    );
    assert!(!world.combatants().get(c).expect("C exists").dead());

    // C's placement is only 524288 from B, but without a mover its distance
    // is u64::MAX, so the farther A (655360) is the approach target.
    let split = sides(&[(a, 1), (b, 2), (c, 1)]);
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: a,
            destination: rp(98_240, 0, 0),
        })
    );

    // Control: the same record registered with the movement rules has a
    // mover, so the nearer C is the approach target.
    let (mut world, ids) = fixture.start_with_c_record(7, true);
    bind(&mut world, &mesh);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    assert!(world.movers().get(c).is_some());
    assert_eq!(
        decide(&world, b, &split, Some(&mesh)),
        Ok(Decision::Move {
            target: c,
            destination: rp(229_312, 0, 0),
        })
    );
    assert_ne!(a, c);
}

#[test]
fn g34_tb_one_raw_of_movement_moves() {
    // M-7: `d == 0` is the fallback and `d == 1` moves one raw.
    // B at 1 m, A inside MS's wall at 5 m: its goal is 4 m, 196608 away.
    let fixture = ranged(near(moving()), 196_671);
    let (mut world, ids) = fixture.start(7);
    let wall = ms();
    bind(&mut world, &wall);
    let (a, b) = (ids[0], ids[1]);
    let split = sides(&[(a, 1), (b, 2)]);
    world.place(a, p(5, 0)).expect("A has a mover");

    // remaining = 196671 - 64 = 196607 leaves exactly one raw of walk.
    assert_eq!(
        decide(&world, b, &split, Some(&wall)),
        Ok(Decision::Move {
            target: a,
            destination: rp(65_537, 0, 0),
        })
    );

    // Control: one raw more range leaves d = 0, so the turn ends instead.
    let fixture = ranged(near(moving()), 196_672);
    let (mut world, ids) = fixture.start(7);
    bind(&mut world, &wall);
    let (a, b) = (ids[0], ids[1]);
    world.place(a, p(5, 0)).expect("A has a mover");
    assert_eq!(
        decide(&world, b, &split, Some(&wall)),
        Ok(Decision::Act(CombatAction::EndTurn { actor: b }))
    );
}
