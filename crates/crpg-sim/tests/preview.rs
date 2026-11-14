//! T143c: the read-only action preview (`preview_action`) and the one
//! admission path (`admit`) it shares with `validate_action`.
//!
//! Every expected value is the contract's literal, derived from the stage
//! order of Reading 4. The first test in this file, T14, pins today's
//! `validate_action` over the worlds of T8 and T9 and was committed before
//! `admit` existed.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_sim::{
    perform_action, tick, validate_action, CombatAction, CombatError, EncounterMode, World,
};
use support::rt;
use support::surface::{
    act, add_derived, always_success, attach_to_ability, binary, edit_ability, effect_doc, end,
    modifier, start, stat, stat_target,
};
use support::Fixture;

/// Ability `103`, the fixture's one ability.
fn a103() -> Ulid {
    Ulid::from_u128(103)
}

/// An ability the fixture does not list.
fn a999() -> Ulid {
    Ulid::from_u128(999)
}

/// The pool every fixture ability spends.
fn pool105() -> Ulid {
    Ulid::from_u128(105)
}

/// `F0` without edits, started turn-based: B acts first.
fn f0_world() -> (World, [EntityId; 2]) {
    let (world, ids) = start(&Fixture::standard(), 1);
    (world, [ids[0], ids[1]])
}

/// T137 R6's zeroing fixture (a copy of `tests/surface_resolution.rs`'s).
fn zeroing_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    add_derived(
        &mut fixture,
        "ratio",
        binary("divide", stat("alpha"), stat("beta")),
    );
    always_success(&mut fixture);
    edit_ability(&mut fixture, |ability| {
        ability.defense = crpg_data::DefenseWire::TargetStat {
            stat: "ratio".to_owned(),
        };
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
    edit_ability(&mut fixture, |ability| {
        ability.allow_self_target = true;
        ability.ends_turn = false;
    });
    fixture
}

/// The zeroing world after its script: seed 17, `B -> A`, `end(B)`, `end(A)`.
fn zeroing_world() -> (World, [EntityId; 2]) {
    let (mut world, ids) = start(&zeroing_fixture(), 17);
    let (a, b) = (ids[0], ids[1]);
    act(&mut world, b, a103(), a);
    end(&mut world, b);
    end(&mut world, a);
    (world, [a, b])
}

/// `F0` with `ends_turn = false`, after `B -> A`: B keeps the turn with pool 0.
fn spent_turn_world() -> (World, [EntityId; 2]) {
    let mut fixture = Fixture::standard();
    edit_ability(&mut fixture, |ability| ability.ends_turn = false);
    let (mut world, ids) = start(&fixture, 1);
    act(&mut world, ids[1], a103(), ids[0]);
    (world, [ids[0], ids[1]])
}

/// A lethal fixture where `B -> A` kills A and ends the encounter.
fn lethal_world() -> (World, [EntityId; 2]) {
    let mut fixture = Fixture::standard();
    rt::make_lethal(&mut fixture, 100);
    edit_ability(&mut fixture, |ability| ability.allow_self_target = true);
    let (mut world, ids) = start(&fixture, 1);
    act(&mut world, ids[1], a103(), ids[0]);
    (world, [ids[0], ids[1]])
}

/// The real-time start: `capable(10)`.
fn rt_start_world() -> (World, [EntityId; 2]) {
    let (world, ids) = rt::capable(10).start_rt(1, EncounterMode::RealTime);
    (world, [ids[0], ids[1]])
}

/// The real-time spent-pool world: B's order resolved and spent its pool.
fn rt_spent_world() -> (World, [EntityId; 2]) {
    let (mut world, ids) = rt::capable(4).start_rt(7, EncounterMode::RealTime);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        perform_action(&mut world, &rt::use_action(b, a103(), a)),
        Ok(None)
    );
    tick(&mut world);
    assert_eq!(rt::pool_of(&world, b), 0, "B's order spent its pool");
    (world, [a, b])
}

/// The real-time world whose ability has a range and no movers exist.
fn rt_range_world() -> (World, [EntityId; 2]) {
    let mut fixture = rt::capable(10);
    rt::edit_ability(&mut fixture, |ability| {
        ability.range = Some(Fx16_16::from_raw(i32::MAX));
    });
    let (world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    (world, [ids[0], ids[1]])
}

/// Every named world of T8 and T9.
fn all_worlds() -> Vec<(&'static str, World, [EntityId; 2])> {
    let mut out = Vec::new();
    for (name, (world, ids)) in [
        ("f0", f0_world()),
        ("spent_turn", spent_turn_world()),
        ("zeroing", zeroing_world()),
        ("lethal", lethal_world()),
        ("rt_start", rt_start_world()),
        ("rt_spent", rt_spent_world()),
        ("rt_range", rt_range_world()),
    ] {
        out.push((name, world, ids));
    }
    out
}

/// The twelve candidates in table order: abilities `103` then `999`, each
/// with actor `[A, B]` and target `[A, B]` (actor-major), then `EndTurn` and
/// `Stop` for each id.
fn candidates(ids: [EntityId; 2]) -> Vec<CombatAction> {
    let mut out = Vec::new();
    for ability in [a103(), a999()] {
        for actor in ids {
            for target in ids {
                out.push(CombatAction::UseAbility {
                    actor,
                    ability,
                    target,
                });
            }
        }
    }
    for actor in ids {
        out.push(CombatAction::EndTurn { actor });
    }
    for actor in ids {
        out.push(CombatAction::Stop { actor });
    }
    out
}

/// The literal expected `validate_action` results, derived from Reading 4's
/// stage order. Order: the 12 candidates of [`candidates`].
fn expected(name: &str, [a, b]: [EntityId; 2]) -> Vec<Result<(), CombatError>> {
    use CombatError::{
        DeadActor, DeadTarget, InsufficientAction, OutOfRange, OutOfTurn, SelfTarget,
        UnknownAbility, WrongMode,
    };
    let oot_a = Err(OutOfTurn {
        actor: a,
        active: Some(b),
    });
    let oot_b_none = Err(OutOfTurn {
        actor: b,
        active: None,
    });
    let unknown = Err(UnknownAbility { ability: a999() });
    let wrong_tb = Err(WrongMode {
        mode: EncounterMode::TurnBased,
    });
    let wrong_rt = Err(WrongMode {
        mode: EncounterMode::RealTime,
    });
    let broke = Err(InsufficientAction {
        pool: pool105(),
        cost: 1,
        current: 0,
    });
    match name {
        // B active with a funded pool: only B -> A passes.
        "f0" => vec![
            oot_a.clone(),
            oot_a.clone(),
            Ok(()),
            Err(SelfTarget),
            oot_a.clone(),
            oot_a.clone(),
            unknown.clone(),
            unknown,
            oot_a.clone(),
            Ok(()),
            wrong_tb.clone(),
            wrong_tb,
        ],
        // B active with an empty pool: affordability follows self-target.
        "spent_turn" => vec![
            oot_a.clone(),
            oot_a.clone(),
            broke,
            Err(SelfTarget),
            oot_a.clone(),
            oot_a.clone(),
            unknown.clone(),
            unknown,
            oot_a.clone(),
            Ok(()),
            wrong_tb.clone(),
            wrong_tb,
        ],
        // A's beta is zeroed: B -> A fails the stat stage, B -> B passes.
        "zeroing" => vec![
            oot_a.clone(),
            oot_a.clone(),
            Err(stat_ratio()),
            Ok(()),
            oot_a.clone(),
            oot_a.clone(),
            unknown.clone(),
            unknown,
            oot_a.clone(),
            Ok(()),
            wrong_tb.clone(),
            wrong_tb,
        ],
        // A is dead and the encounter is finished (active None).
        "lethal" => vec![
            Err(DeadActor { actor: a }),
            Err(DeadActor { actor: a }),
            Err(DeadTarget { target: a }),
            oot_b_none.clone(),
            Err(DeadActor { actor: a }),
            Err(DeadActor { actor: a }),
            Err(DeadTarget { target: a }),
            oot_b_none.clone(),
            Err(DeadActor { actor: a }),
            oot_b_none,
            Err(DeadActor { actor: a }),
            wrong_tb,
        ],
        // Real time: no turn, both pools funded.
        "rt_start" => vec![
            Err(SelfTarget),
            Ok(()),
            Ok(()),
            Err(SelfTarget),
            unknown.clone(),
            unknown.clone(),
            unknown.clone(),
            unknown,
            wrong_rt.clone(),
            wrong_rt,
            Ok(()),
            Ok(()),
        ],
        // Real time, B's pool spent by its resolved order.
        "rt_spent" => vec![
            Err(SelfTarget),
            Ok(()),
            broke,
            Err(SelfTarget),
            unknown.clone(),
            unknown.clone(),
            unknown.clone(),
            unknown,
            wrong_rt.clone(),
            wrong_rt,
            Ok(()),
            Ok(()),
        ],
        // Real time with a range and no movers: range is the last stage.
        "rt_range" => vec![
            Err(SelfTarget),
            Err(OutOfRange),
            Err(OutOfRange),
            Err(SelfTarget),
            unknown.clone(),
            unknown.clone(),
            unknown.clone(),
            unknown,
            wrong_rt.clone(),
            wrong_rt,
            Ok(()),
            Ok(()),
        ],
        other => panic!("no literal table for world {other}"),
    }
}

/// `StatEvaluation` naming the derived stat of the zeroing fixture.
fn stat_ratio() -> CombatError {
    CombatError::StatEvaluation {
        stat: "ratio".to_owned(),
    }
}

/// T14 `execute_is_todays_validate_action` (B2). `validate_action`'s result
/// for every candidate in every world equals the literal table. This test
/// was committed first, on the base before `admit` existed.
#[test]
fn t14_execute_is_todays_validate_action() {
    for (name, world, ids) in all_worlds() {
        let table = expected(name, ids);
        let list = candidates(ids);
        assert_eq!(table.len(), list.len(), "{name}: a literal per candidate");
        for (index, (action, want)) in list.iter().zip(table).enumerate() {
            assert_eq!(
                validate_action(&world, action),
                want,
                "{name}: candidate {index}: {action:?}"
            );
        }
    }
}
