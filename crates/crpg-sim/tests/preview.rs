//! T143c: the read-only action preview (`preview_action`) and the one
//! admission path (`admit`) it shares with `validate_action`.
//!
//! Every expected value is the contract's literal, derived from the stage
//! order of Reading 4. The first test in this file, T14, pins today's
//! `validate_action` over the worlds of T8 and T9 and was committed before
//! `admit` existed.

#[allow(dead_code)]
mod support;

use std::collections::BTreeMap;

use crpg_core::{DeterministicRng, EntityId, Fx16_16, Ulid};
use crpg_data::DefenseWire;
use crpg_rules::{DiceExpr, Outcome};
use crpg_sim::{
    perform_action, preview_action, state_hash, tick, validate_action, ActionPreview, CombatAction,
    CombatError, EncounterMode, OutcomeOdds, PreviewCount, PreviewError, PreviewPart, PreviewRules,
    PreviewScope, World, COMBAT_ROLL_STREAM, MAX_PREVIEW_COST,
};
use serde_json::{json, Value};
use support::draws::{always_failure, defend_opposed, draw, oracle, set_damage, set_table};
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
// ---------------------------------------------------------------------------
// Notation (the contract's)
// ---------------------------------------------------------------------------

const S: Outcome = Outcome::Success;
const F: Outcome = Outcome::Failure;
const CS: Outcome = Outcome::CriticalSuccess;
const CF: Outcome = Outcome::CriticalFailure;

/// `C(k)`.
fn cu(k: u8) -> Outcome {
    Outcome::Custom(k)
}

/// `c(n)`.
fn c(n: u64) -> PreviewCount {
    PreviewCount::from(n)
}

/// `r(o, count, natural, space, damage)`.
fn r(outcome: Outcome, count: u64, natural: u64, space: u64, damage: &[(u32, u64)]) -> OutcomeOdds {
    OutcomeOdds {
        outcome,
        count: c(count),
        natural: c(natural),
        damage_space: c(space),
        damage: damage.iter().map(|(v, n)| (*v, c(*n))).collect(),
    }
}

type Odds = Result<ActionPreview, PreviewError>;

/// `ok(n, rows)`.
fn ok(n: u64, rows: Vec<OutcomeOdds>) -> Odds {
    Ok(ActionPreview {
        attack_space: c(n),
        outcomes: rows,
    })
}

/// `TL(part, cost, limit)`.
fn tl(part: PreviewPart, cost: u64, limit: u64) -> Odds {
    Err(PreviewError::TooLarge {
        part,
        cost: Some(cost),
        limit,
    })
}

/// `Rejected(error)`.
fn rejected(error: CombatError) -> Odds {
    Err(PreviewError::Rejected(error))
}

const ACT: PreviewRules = PreviewRules {
    scope: PreviewScope::Actionable,
    max_cost: MAX_PREVIEW_COST,
};
const PLAN: PreviewRules = PreviewRules {
    scope: PreviewScope::Planning,
    max_cost: MAX_PREVIEW_COST,
};

/// The contract's `act(m)`: `ACT` with `max_cost: m`.
fn lim(max_cost: u64) -> PreviewRules {
    PreviewRules {
        scope: PreviewScope::Actionable,
        max_cost,
    }
}

/// One preview call, checked read-only: Ok or Err, the world afterwards
/// equals a clone taken before, with the same bytes and the same hash (B12).
fn pva(world: &World, x: EntityId, ability: Ulid, y: EntityId, rules: &PreviewRules) -> Odds {
    let before = world.clone();
    let bytes = serde_json::to_vec(world).expect("the world serializes");
    let hash = state_hash(world);
    let got = preview_action(world, x, ability, y, rules);
    assert_eq!(*world, before, "the preview changed the world");
    assert_eq!(
        serde_json::to_vec(world).expect("the world serializes"),
        bytes,
        "the preview changed the world's bytes"
    );
    assert_eq!(state_hash(world), hash, "the preview changed the hash");
    got
}

/// The contract's `P(w, x, y)` under `rules`.
fn pv(world: &World, x: EntityId, y: EntityId, rules: &PreviewRules) -> Odds {
    pva(world, x, a103(), y, rules)
}

/// `P(w, x, y)` under `ACT`.
fn p(world: &World, x: EntityId, y: EntityId) -> Odds {
    pv(world, x, y, &ACT)
}

/// `T1rows`.
fn t1_rows() -> Vec<OutcomeOdds> {
    vec![r(S, 15, 0, 1, &[(2, 1)]), r(F, 21, 0, 1, &[(0, 1)])]
}

/// A's odds against B in `F0`: success on a total of 8 or less.
fn a_rows() -> Vec<OutcomeOdds> {
    vec![r(S, 26, 0, 1, &[(2, 1)]), r(F, 10, 0, 1, &[(0, 1)])]
}

/// The standard fixture with `edit` applied, started with seed 1.
fn f0_with(edit: impl FnOnce(&mut Fixture)) -> (World, [EntityId; 2]) {
    let mut fixture = Fixture::standard();
    edit(&mut fixture);
    let (world, ids) = start(&fixture, 1);
    (world, [ids[0], ids[1]])
}

fn outcome_json(name: &str) -> Value {
    json!({"type": name})
}

fn custom_json(k: u8) -> Value {
    json!({"type": "custom", "value": k})
}

fn damage_json(outcome: &str, amount: u32, dice: Option<&str>) -> Value {
    match dice {
        Some(dice) => json!({"outcome": outcome_json(outcome), "amount": amount, "dice": dice}),
        None => json!({"outcome": outcome_json(outcome), "amount": amount}),
    }
}

/// The decimal sum of decimal strings (the counts have no public addition).
fn decimal_sum(parts: &[String]) -> String {
    let mut total: Vec<u32> = vec![0];
    for part in parts {
        let digits: Vec<u32> = part.bytes().rev().map(|b| u32::from(b - b'0')).collect();
        let mut carry = 0;
        for index in 0..digits.len().max(total.len()) {
            if index == total.len() {
                total.push(0);
            }
            let sum = total[index] + digits.get(index).copied().unwrap_or(0) + carry;
            total[index] = sum % 10;
            carry = sum / 10;
        }
        if carry > 0 {
            total.push(carry);
        }
    }
    total
        .iter()
        .rev()
        .map(|digit| char::from(b'0' + u8::try_from(*digit).expect("a digit")))
        .collect()
}

/// The counts of a row's damage list as decimal strings.
fn damage_counts(row: &OutcomeOdds) -> Vec<String> {
    row.damage.iter().map(|(_, n)| n.to_string()).collect()
}

/// 6 to the power 100 (U1's literal).
const SIX_100: &str =
    "653318623500070906096690267158057820537143710472954871543071966369497141477376";

// ---------------------------------------------------------------------------
// Fixtures of the numbered cases
// ---------------------------------------------------------------------------

/// T2: B's ability against `defense`.
fn t2_world(defense: DefenseWire) -> (World, [EntityId; 2]) {
    f0_with(|f| edit_ability(f, |ability| ability.defense = defense))
}

/// T3's fixture.
fn t3_fixture() -> Fixture {
    let mut f = Fixture::standard();
    edit_ability(&mut f, |ability| {
        ability.dice = "1d20".to_owned();
        ability.natural_die = Some(0);
        ability.defense = DefenseWire::Constant(11);
    });
    set_table(
        &mut f,
        json!([
            {"min_margin": i64::MIN, "outcome": outcome_json("failure")},
            {"min_margin": 0, "outcome": outcome_json("success")},
            {"min_margin": 10, "outcome": outcome_json("critical_success")}
        ]),
        json!([
            {"face": 20, "effect": {"type": "shift", "value": 1}},
            {"face": 1, "effect": {"type": "override", "value": outcome_json("critical_failure")}},
            {"face": 2, "effect": {"type": "shift", "value": 0}}
        ]),
    );
    set_damage(
        &mut f,
        json!([
            damage_json("success", 3, Some("1d4")),
            damage_json("critical_success", 3, Some("2d4")),
            damage_json("failure", 0, None)
        ]),
    );
    f
}

fn t3_rows() -> Vec<OutcomeOdds> {
    vec![
        r(F, 9, 1, 1, &[(0, 1)]),
        r(S, 9, 0, 4, &[(4, 1), (5, 1), (6, 1), (7, 1)]),
        r(
            CS,
            1,
            1,
            16,
            &[(5, 1), (6, 2), (7, 3), (8, 4), (9, 3), (10, 2), (11, 1)],
        ),
        r(CF, 1, 1, 1, &[(0, 1)]),
    ]
}

/// T4's fixture: ten custom bands `C(0)..C(9)` against `Constant(0)`.
fn t4_fixture(dice: &str, natural: Option<u32>, rules: Value) -> Fixture {
    let mut f = Fixture::standard();
    edit_ability(&mut f, |ability| {
        ability.dice = dice.to_owned();
        ability.natural_die = natural;
        ability.defense = DefenseWire::Constant(0);
    });
    let bands: Vec<Value> = (0..10_u8)
        .map(|k| {
            let min: i64 = if k == 0 { i64::MIN } else { i64::from(k) };
            json!({"min_margin": min, "outcome": custom_json(k)})
        })
        .collect();
    set_table(&mut f, Value::Array(bands), rules);
    f
}

/// T4's expected result: ten rows with unit damage `[(0, 1)]`.
fn t4_expect(space: u64, counts: [u64; 10], naturals: [u64; 10]) -> Odds {
    ok(
        space,
        (0..10_u8)
            .map(|k| {
                r(
                    cu(k),
                    counts[usize::from(k)],
                    naturals[usize::from(k)],
                    1,
                    &[(0, 1)],
                )
            })
            .collect(),
    )
}

/// T5's fixture.
fn t5_fixture() -> Fixture {
    let mut f = Fixture::standard();
    edit_ability(&mut f, |ability| {
        ability.dice = "2d4kh1".to_owned();
        ability.natural_die = Some(1);
        ability.defense = DefenseWire::Constant(3);
    });
    set_table(
        &mut f,
        json!([
            {"min_margin": i64::MIN, "outcome": outcome_json("failure")},
            {"min_margin": 0, "outcome": outcome_json("success")}
        ]),
        json!([{"face": 4, "effect": {"type": "override", "value": outcome_json("critical_success")}}]),
    );
    f
}

fn t5_rows() -> Vec<OutcomeOdds> {
    vec![
        r(F, 4, 0, 1, &[(0, 1)]),
        r(S, 8, 0, 1, &[(2, 1)]),
        r(CS, 4, 4, 1, &[(0, 1)]),
    ]
}

/// T6's fixture: an opposed `1d6`, optionally with bonus stats on both
/// sides and a natural-1 override.
fn t6_fixture(bonus: bool, natural: bool) -> Fixture {
    let mut f = Fixture::standard();
    edit_ability(&mut f, |ability| {
        ability.dice = "1d6".to_owned();
        if natural {
            ability.natural_die = Some(0);
        }
        if bonus {
            ability.bonus_stats = Some(vec!["gamma".to_owned()]);
        }
    });
    defend_opposed(&mut f, "1d6", if bonus { &["beta"] } else { &[] });
    set_table(
        &mut f,
        json!([
            {"min_margin": i64::MIN, "outcome": outcome_json("failure")},
            {"min_margin": 1, "outcome": outcome_json("success")}
        ]),
        if natural {
            json!([{"face": 1, "effect": {"type": "override", "value": outcome_json("critical_failure")}}])
        } else {
            json!([])
        },
    );
    set_damage(
        &mut f,
        json!([
            damage_json("success", 1, Some("1d6")),
            damage_json("failure", 0, None)
        ]),
    );
    f
}

const D6: [(u32, u64); 6] = [(2, 1), (3, 1), (4, 1), (5, 1), (6, 1), (7, 1)];

fn t6_world(bonus: bool, natural: bool) -> (World, [EntityId; 2]) {
    let (world, ids) = start(&t6_fixture(bonus, natural), 1);
    (world, [ids[0], ids[1]])
}

fn t6_rows(failure: u64, success: u64, critical: Option<u64>) -> Vec<OutcomeOdds> {
    let mut rows = vec![r(F, failure, 0, 1, &[(0, 1)]), r(S, success, 0, 6, &D6)];
    if let Some(count) = critical {
        rows.push(r(CF, count, count, 1, &[(0, 1)]));
    }
    rows
}

/// T7: the smallest seed whose first `2d6` is at most `dc`.
fn smallest_seed(dc: i32) -> u64 {
    (0..1000_u64)
        .find(|seed| draw(&mut oracle(*seed), "2d6").total <= dc)
        .expect("a seed within 1000 draws that low")
}

/// T7's script: `start(F, s*)`, `B -> A` (a success), `end(A)`.
fn t7_world(
    defense: Option<DefenseWire>,
    effect: crpg_data::Effect,
    dc: i32,
) -> (World, [EntityId; 2]) {
    let mut f = Fixture::standard();
    if let Some(defense) = defense {
        edit_ability(&mut f, |ability| ability.defense = defense);
    }
    attach_to_ability(&mut f, effect);
    let (mut world, ids) = start(&f, smallest_seed(dc));
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(act(&mut world, b, a103(), a).outcome, S, "the script hits");
    end(&mut world, a);
    (world, [a, b])
}

fn dc_plus_two(aim: &str) -> crpg_data::Effect {
    effect_doc(
        141,
        aim,
        "probe-type",
        "stack_all",
        vec![modifier(142, json!({"type": "dc"}), "add", 2)],
        3,
    )
}

fn actor_gamma() -> Option<DefenseWire> {
    Some(DefenseWire::ActorStat {
        stat: "gamma".to_owned(),
    })
}

// ---------------------------------------------------------------------------
// T1, T2: the standard odds and every defence
// ---------------------------------------------------------------------------

/// T1 (B5, B8, B10): `F0`, B against A.
#[test]
fn t1_standard_odds() {
    let (world, [a, b]) = f0_world();
    assert_eq!(p(&world, b, a), ok(36, t1_rows()));
}

/// T2 (B5): every defence reads the right side and stat. Each is checked
/// against a different control value, so no two readings agree.
#[test]
fn t2_every_defence() {
    let damage = |success: u64, failure: u64| {
        vec![
            r(S, success, 0, 1, &[(2, 1)]),
            r(F, failure, 0, 1, &[(0, 1)]),
        ]
    };
    let (world, [a, b]) = t2_world(DefenseWire::TargetStat {
        stat: "gamma".to_owned(),
    });
    assert_eq!(p(&world, b, a), ok(36, damage(21, 15)), "A's gamma 7");
    let (world, [a, b]) = t2_world(DefenseWire::ActorStat {
        stat: "gamma".to_owned(),
    });
    assert_eq!(p(&world, b, a), ok(36, damage(10, 26)), "B's own gamma 5");
    let (world, [a, b]) = t2_world(DefenseWire::Constant(8));
    assert_eq!(p(&world, b, a), ok(36, damage(26, 10)));
    // Control: the default defence is the actor's alpha.
    let (world, [a, b]) = f0_world();
    assert_eq!(p(&world, b, a), ok(36, t1_rows()));
}

// ---------------------------------------------------------------------------
// T3 - T5: natural rules, damage dice, selections and tags
// ---------------------------------------------------------------------------

/// T3 (B6, B8, B9, B10).
#[test]
fn t3_natural_rules_and_damage_dice() {
    let (world, ids) = start(&t3_fixture(), 1);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(p(&world, b, a), ok(20, t3_rows()));
}

/// T4 (B6, B8): selections, offsets and the natural tag.
#[test]
fn t4_selections_and_tags() {
    let none = || json!([]);
    let zero = [0_u64; 10];
    let cases: Vec<(&str, [u64; 10])> = vec![
        ("3d3", [0, 0, 0, 1, 3, 6, 7, 6, 3, 1]),
        ("3d3kh1", [0, 1, 7, 19, 0, 0, 0, 0, 0, 0]),
        ("3d3kl1", [0, 19, 7, 1, 0, 0, 0, 0, 0, 0]),
        ("3d3dh1", [0, 0, 7, 9, 7, 3, 1, 0, 0, 0]),
        ("3d3kl2", [0, 0, 7, 9, 7, 3, 1, 0, 0, 0]),
        ("3d3dl1", [0, 0, 1, 3, 7, 9, 7, 0, 0, 0]),
        ("3d3kh2", [0, 0, 1, 3, 7, 9, 7, 0, 0, 0]),
        ("3d3-2", [0, 1, 3, 6, 7, 6, 3, 1, 0, 0]),
    ];
    for (dice, counts) in cases {
        let (world, ids) = start(&t4_fixture(dice, None, none()), 1);
        assert_eq!(
            p(&world, ids[1], ids[0]),
            t4_expect(27, counts, zero),
            "{dice}"
        );
    }
    let (world, ids) = start(&t4_fixture("1d3-3", None, none()), 1);
    assert_eq!(
        p(&world, ids[1], ids[0]),
        t4_expect(3, [3, 0, 0, 0, 0, 0, 0, 0, 0, 0], zero),
        "1d3-3 merges into C(0)"
    );
    let (world, ids) = start(
        &t4_fixture(
            "3d3",
            Some(2),
            json!([{"face": 3, "effect": {"type": "shift", "value": 0}}]),
        ),
        1,
    );
    assert_eq!(
        p(&world, ids[1], ids[0]),
        t4_expect(
            27,
            [0, 0, 0, 1, 3, 6, 7, 6, 3, 1],
            [0, 0, 0, 0, 0, 1, 2, 3, 2, 1]
        ),
        "the tagged counts sit on the totals 5..=9"
    );
}

/// T5 (B6, B8): the natural face of a dropped die decides.
#[test]
fn t5_natural_face_of_a_dropped_die() {
    let (world, ids) = start(&t5_fixture(), 1);
    assert_eq!(p(&world, ids[1], ids[0]), ok(16, t5_rows()));
}

// ---------------------------------------------------------------------------
// T6: Opposed
// ---------------------------------------------------------------------------

/// T6 (B7, B8): the opposed roll, bonus lists and the opposing face.
#[test]
fn t6_opposed() {
    let cases = [
        (false, false, t6_rows(21, 15, None)),
        (true, false, t6_rows(26, 10, None)),
        (false, true, t6_rows(15, 15, Some(6))),
        (true, true, t6_rows(20, 10, Some(6))),
    ];
    for (bonus, natural, rows) in cases {
        let (world, [a, b]) = t6_world(bonus, natural);
        assert_eq!(
            p(&world, b, a),
            ok(36, rows),
            "bonus {bonus}, natural {natural}"
        );
    }
}

// ---------------------------------------------------------------------------
// T7: attached effects fold
// ---------------------------------------------------------------------------

/// T7 (B5, B7): an attached effect changes the odds on the side it sits on.
#[test]
fn t7_attached_effects_fold() {
    let rows = |success: u64, failure: u64| {
        vec![
            r(S, success, 0, 1, &[(2, 1)]),
            r(F, failure, 0, 1, &[(0, 1)]),
        ]
    };
    // (a) a target `Dc +2` lifts A's defence from 6 to 8.
    let (world, [a, b]) = t7_world(None, dc_plus_two("target"), 6);
    assert_eq!(p(&world, b, a), ok(36, rows(26, 10)));
    // (b) a self `Roll set 7` makes B's total 7, above the defence 6.
    let set_seven = effect_doc(
        141,
        "slf",
        "probe-type",
        "stack_all",
        vec![modifier(142, json!({"type": "roll"}), "set", 7)],
        3,
    );
    let (world, [a, b]) = t7_world(None, set_seven, 6);
    assert_eq!(p(&world, b, a), ok(36, rows(0, 36)));
    // (c) `ActorStat gamma` with a self `Dc +2`: 5 + 2.
    let (world, [a, b]) = t7_world(actor_gamma(), dc_plus_two("slf"), 5);
    assert_eq!(p(&world, b, a), ok(36, rows(21, 15)));
    // (d) `ActorStat gamma` with a target `Dc +2`: the actor's own 5.
    let (world, [a, b]) = t7_world(actor_gamma(), dc_plus_two("target"), 5);
    assert_eq!(p(&world, b, a), ok(36, rows(10, 26)));
}

// ---------------------------------------------------------------------------
// T8, T9: the two scopes
// ---------------------------------------------------------------------------

/// T8 (B1, B14): `Actionable` is `validate_action`.
#[test]
fn t8_actionable_is_validate_action() {
    let oot = |actor: EntityId, active: EntityId| {
        rejected(CombatError::OutOfTurn {
            actor,
            active: Some(active),
        })
    };
    let broke = || {
        rejected(CombatError::InsufficientAction {
            pool: pool105(),
            cost: 1,
            current: 0,
        })
    };

    // Turn-based: only the active combatant passes.
    let (world, [a, b]) = f0_world();
    assert_eq!(p(&world, a, b), oot(a, b));
    assert_eq!(p(&world, b, a), ok(36, t1_rows()), "control");

    // Real time: any live combatant passes while two or more are alive.
    let (world, [a, b]) = rt_start_world();
    assert_eq!(p(&world, a, b), ok(36, a_rows()));
    assert_eq!(p(&world, b, a), ok(36, t1_rows()));

    // Real-time affordability: B's spent pool refuses `Actionable` and not
    // `Planning`.
    let (world, [a, b]) = rt_spent_world();
    assert_eq!(p(&world, b, a), broke());
    assert_eq!(pv(&world, b, a, &PLAN), ok(36, t1_rows()));
    assert_eq!(p(&world, a, b), ok(36, a_rows()), "A's pool is funded");

    // Range: no movers exist, so a ranged ability is out of range in both
    // scopes. The control (no range) is the real-time start.
    let (world, [a, b]) = rt_range_world();
    assert_eq!(p(&world, a, b), rejected(CombatError::OutOfRange));
    assert_eq!(pv(&world, a, b, &PLAN), rejected(CombatError::OutOfRange));
    let (world, [a, b]) = rt_start_world();
    assert_eq!(p(&world, a, b), ok(36, a_rows()));
    assert_eq!(pv(&world, a, b, &PLAN), ok(36, a_rows()));

    // No encounter, unknown ability, self-target.
    let (donor, [a, b]) = f0_world();
    let empty = World::new(0);
    assert_eq!(p(&empty, b, a), rejected(CombatError::NoEncounter));
    assert_eq!(p(&donor, b, a), ok(36, t1_rows()), "control");
    assert_eq!(
        pva(&donor, b, a999(), a, &ACT),
        rejected(CombatError::UnknownAbility { ability: a999() })
    );
    assert_eq!(p(&donor, b, b), rejected(CombatError::SelfTarget));
    let (world, [_, b]) = f0_with(|f| edit_ability(f, |ability| ability.allow_self_target = true));
    assert_eq!(p(&world, b, b), ok(36, t1_rows()));

    // The stat evaluation stage (T137 R6).
    let (world, [a, b]) = zeroing_world();
    assert_eq!(
        p(&world, b, a),
        rejected(CombatError::StatEvaluation {
            stat: "ratio".to_owned()
        })
    );
    assert_eq!(p(&world, b, b), ok(36, vec![r(S, 36, 0, 1, &[(2, 1)])]));
}

/// T9 (B2, B14): `Planning` skips exactly the turn and affordability stages.
#[test]
fn t9_planning_scope() {
    let broke = || {
        rejected(CombatError::InsufficientAction {
            pool: pool105(),
            cost: 1,
            current: 0,
        })
    };
    // Out of turn: A's odds although it is B's turn. Control: `Actionable`.
    let (world, [a, b]) = f0_world();
    assert_eq!(pv(&world, a, b, &PLAN), ok(36, a_rows()));
    assert_eq!(
        p(&world, a, b),
        rejected(CombatError::OutOfTurn {
            actor: a,
            active: Some(b)
        })
    );
    // An empty pool on B's own turn.
    let (world, [a, b]) = spent_turn_world();
    assert_eq!(p(&world, b, a), broke());
    assert_eq!(pv(&world, b, a, &PLAN), ok(36, t1_rows()));
    // Stat evaluation still runs.
    let (world, [a, b]) = zeroing_world();
    assert_eq!(
        pv(&world, b, a, &PLAN),
        rejected(CombatError::StatEvaluation {
            stat: "ratio".to_owned()
        })
    );
    assert_eq!(
        pv(&world, b, b, &PLAN),
        ok(36, vec![r(S, 36, 0, 1, &[(2, 1)])]),
        "control: the same world, a target whose stat evaluates"
    );
    // Self-target and unknown ability still run.
    let (world, [a, b]) = f0_world();
    assert_eq!(pv(&world, b, b, &PLAN), rejected(CombatError::SelfTarget));
    assert_eq!(
        pva(&world, b, a999(), a, &PLAN),
        rejected(CombatError::UnknownAbility { ability: a999() })
    );
    assert_eq!(pv(&world, b, a, &PLAN), ok(36, t1_rows()), "control");
    // The finished encounter: the terminal gate refuses both scopes.
    let (world, [a, b]) = lethal_world();
    let gate = || {
        rejected(CombatError::OutOfTurn {
            actor: b,
            active: None,
        })
    };
    assert_eq!(pv(&world, b, b, &PLAN), gate());
    assert_eq!(pv(&world, b, b, &ACT), gate());
    assert_eq!(
        pv(&world, a, b, &PLAN),
        rejected(CombatError::DeadActor { actor: a }),
        "death precedes the gate"
    );
}

/// The error a stage after the turn and affordability stages gives a
/// candidate that `validate_action` refused at one of those two: the unknown
/// ability, then the self-target stage, then the stat evaluation stage.
fn later_stage(name: &str, ids: [EntityId; 2], action: &CombatAction) -> Option<CombatError> {
    let CombatAction::UseAbility {
        actor,
        ability,
        target,
    } = *action
    else {
        panic!("a UseAbility candidate");
    };
    if ability == a999() {
        return Some(CombatError::UnknownAbility { ability });
    }
    let self_allowed = matches!(name, "zeroing" | "lethal");
    if actor == target && !self_allowed {
        return Some(CombatError::SelfTarget);
    }
    if name == "zeroing" && target == ids[0] {
        return Some(stat_ratio());
    }
    None
}

/// T14 (B2), the second half: after the refactor, `Planning` agrees with
/// `validate_action` everywhere except the two skipped stages.
#[test]
fn t14_plan_follows_validate_action() {
    let mut checked = 0;
    for (name, world, ids) in all_worlds() {
        for action in candidates(ids) {
            let CombatAction::UseAbility {
                actor,
                ability,
                target,
            } = action
            else {
                continue;
            };
            let plan = pva(&world, actor, ability, target, &PLAN);
            let label = format!("{name}: {action:?}");
            match validate_action(&world, &action) {
                // Only the turn or affordability stage refused: planning
                // skips it, so the result is `Ok` unless a later stage
                // (unknown ability, self-target, stat evaluation) refuses.
                Err(CombatError::OutOfTurn {
                    active: Some(_), ..
                })
                | Err(CombatError::InsufficientAction { .. }) => {
                    match later_stage(name, ids, &action) {
                        Some(error) => assert_eq!(plan, rejected(error), "{label}"),
                        None => assert!(plan.is_ok(), "{label}: planning accepts {plan:?}"),
                    }
                }
                Ok(()) => assert!(plan.is_ok(), "{label}: planning accepts {plan:?}"),
                Err(error) => assert_eq!(plan, rejected(error), "{label}"),
            }
            checked += 1;
        }
    }
    assert_eq!(
        checked,
        7 * 8,
        "eight UseAbility candidates in seven worlds"
    );
    let (world, [_, b]) = lethal_world();
    for ability in [a103(), a999()] {
        assert_eq!(
            pva(&world, b, ability, b, &PLAN),
            rejected(CombatError::OutOfTurn {
                actor: b,
                active: None
            }),
            "a finished encounter has no active combatant"
        );
    }
}

// ---------------------------------------------------------------------------
// T10: bounds
// ---------------------------------------------------------------------------

/// T10 (a), (j) and Display (B3, B4, B1).
#[test]
fn t10_bounds_f0_and_rejection_first() {
    let (world, [a, b]) = f0_world();
    let cases = [
        (28, ok(36, t1_rows())),
        (27, tl(PreviewPart::Attack, 28, 27)),
        (17, tl(PreviewPart::Attack, 28, 17)),
        (16, tl(PreviewPart::Actor, 28, 16)),
    ];
    for (limit, want) in cases {
        assert_eq!(pv(&world, b, a, &lim(limit)), want, "limit {limit}");
    }
    // (j): rejection comes before any cost check.
    assert_eq!(
        pv(&world, a, b, &lim(1)),
        rejected(CombatError::OutOfTurn {
            actor: a,
            active: Some(b)
        })
    );
}

/// T10 (b): the damage parts in table order.
#[test]
fn t10_bounds_charge_damage_rows_in_order() {
    let (world, ids) = start(&t3_fixture(), 1);
    let (a, b) = (ids[0], ids[1]);
    let dmg = |outcome| PreviewPart::Damage { outcome };
    let cases = [
        (64, ok(20, t3_rows())),
        (63, tl(dmg(CS), 64, 63)),
        (53, tl(dmg(CS), 64, 53)),
        (52, tl(dmg(S), 64, 52)),
        (49, tl(dmg(S), 64, 49)),
        (48, tl(PreviewPart::Attack, 64, 48)),
        (25, tl(PreviewPart::Actor, 64, 25)),
    ];
    for (limit, want) in cases {
        assert_eq!(pv(&world, b, a, &lim(limit)), want, "limit {limit}");
    }
}

/// T10 (c), (d): the natural-face rows and the opposing part.
#[test]
fn t10_bounds_natural_and_opposed() {
    let (world, ids) = start(&t5_fixture(), 1);
    let (a, b) = (ids[0], ids[1]);
    let cases = [
        (548, ok(16, t5_rows())),
        (547, tl(PreviewPart::Attack, 548, 547)),
        (539, tl(PreviewPart::Actor, 548, 539)),
    ];
    for (limit, want) in cases {
        assert_eq!(pv(&world, b, a, &lim(limit)), want, "limit {limit}");
    }
    let (world, [a, b]) = t6_world(false, false);
    let dmg = PreviewPart::Damage { outcome: S };
    let cases = [
        (54, ok(36, t6_rows(21, 15, None))),
        (53, tl(dmg, 54, 53)),
        (48, tl(dmg, 54, 48)),
        (47, tl(PreviewPart::Attack, 54, 47)),
        (11, tl(PreviewPart::Opposing, 54, 11)),
        (5, tl(PreviewPart::Actor, 54, 5)),
    ];
    for (limit, want) in cases {
        assert_eq!(pv(&world, b, a, &lim(limit)), want, "limit {limit}");
    }
}

/// T10 (e), (f): limb widths and the kept-selection factor.
#[test]
fn t10_bounds_limbs_and_selections() {
    let mut f = Fixture::standard();
    edit_ability(&mut f, |ability| ability.dice = "33d2".to_owned());
    always_failure(&mut f);
    let (world, ids) = start(&f, 1);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        pv(&world, b, a, &lim(1256)),
        ok(8_589_934_592, vec![r(F, 8_589_934_592, 0, 1, &[(0, 1)])])
    );
    assert_eq!(
        pv(&world, b, a, &lim(1255)),
        tl(PreviewPart::Attack, 1256, 1255)
    );
    assert_eq!(
        pv(&world, b, a, &lim(1187)),
        tl(PreviewPart::Actor, 1256, 1187)
    );

    let (world, ids) = start(&t4_fixture("3d3dh1", None, json!([])), 1);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        pv(&world, b, a, &lim(341)),
        t4_expect(27, [0, 0, 7, 9, 7, 3, 1, 0, 0, 0], [0; 10])
    );
    assert_eq!(
        pv(&world, b, a, &lim(340)),
        tl(PreviewPart::Attack, 341, 340)
    );
}

/// T10 (g): the engine ceiling, and its clamp of a larger ruleset limit.
#[test]
fn t10_bounds_ceiling() {
    let build = |dice: &str| {
        let mut f = Fixture::standard();
        edit_ability(&mut f, |ability| {
            ability.dice = dice.to_owned();
            ability.defense = DefenseWire::Constant(682);
        });
        let (world, ids) = start(&f, 1);
        (world, ids[0], ids[1])
    };
    let (world, a, b) = build("2d1364kh1");
    assert_eq!(
        p(&world, b, a),
        ok(
            1_860_496,
            vec![
                r(S, 465_124, 0, 1, &[(2, 1)]),
                r(F, 1_395_372, 0, 1, &[(0, 1)])
            ]
        )
    );
    assert_eq!(
        pv(&world, b, a, &lim(16_758_104)),
        p(&world, b, a),
        "its cost 16758104 is accepted exactly"
    );
    assert_eq!(
        pv(&world, b, a, &lim(16_758_103)),
        tl(PreviewPart::Attack, 16_758_104, 16_758_103)
    );
    let (world, a, b) = build("2d1365kh1");
    assert_eq!(
        p(&world, b, a),
        tl(PreviewPart::Actor, 16_782_675, MAX_PREVIEW_COST)
    );
    assert_eq!(
        pv(&world, b, a, &lim(u64::MAX)),
        tl(PreviewPart::Actor, 16_782_675, MAX_PREVIEW_COST),
        "a ruleset limit above the ceiling is clamped"
    );
    let (world, a, b) = build("1024d1000000kh1000");
    assert_eq!(
        p(&world, b, a),
        Err(PreviewError::TooLarge {
            part: PreviewPart::Actor,
            cost: None,
            limit: MAX_PREVIEW_COST
        }),
        "an overflowing cost is `None`"
    );
}

/// T10 (h): exact counts far beyond any machine integer.
#[test]
fn t10_large_exact_counts() {
    let mut f = Fixture::standard();
    edit_ability(&mut f, |ability| ability.dice = "100d6".to_owned());
    always_failure(&mut f);
    let (world, ids) = start(&f, 1);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(pv(&world, b, a, &lim(129_255)).map(|_| ()), Ok(()));
    assert_eq!(
        pv(&world, b, a, &lim(129_254)),
        tl(PreviewPart::Attack, 129_255, 129_254)
    );
    let got = p(&world, b, a).expect("100d6 is within the ceiling");
    assert_eq!(got.attack_space.to_string(), SIX_100);
    assert_eq!(got.outcomes.len(), 1);
    assert_eq!(got.outcomes[0].outcome, F);
    assert_eq!(got.outcomes[0].count.to_string(), SIX_100);
    assert_eq!(got.outcomes[0].damage, vec![(0, c(1))]);

    // Damage 100d6: 501 values, the binomial-like counts of U3.
    let (world, [a, b]) = f0_with(|f| {
        always_success(f);
        set_damage(f, json!([damage_json("success", 0, Some("100d6"))]));
    });
    let got = p(&world, b, a).expect("within the ceiling");
    let row = &got.outcomes[0];
    assert_eq!(row.damage.len(), 501);
    let values: Vec<u32> = row.damage.iter().map(|(v, _)| *v).collect();
    assert_eq!(values, (100..=600).collect::<Vec<u32>>());
    let counts = damage_counts(row);
    assert_eq!(counts[..3], ["1", "100", "5050"]);
    assert_eq!(counts[498..], ["5050", "100", "1"]);
    assert_eq!(row.damage_space.to_string(), SIX_100);
    assert_eq!(decimal_sum(&counts), SIX_100, "the counts sum to the space");

    // Damage 10d10kh3: 28 values from 3 to 30.
    let (world, [a, b]) = f0_with(|f| {
        always_success(f);
        set_damage(f, json!([damage_json("success", 0, Some("10d10kh3"))]));
    });
    let got = p(&world, b, a).expect("within the ceiling");
    let row = &got.outcomes[0];
    assert_eq!(row.damage.len(), 28);
    let values: Vec<u32> = row.damage.iter().map(|(v, _)| *v).collect();
    assert_eq!(values, (3..=30).collect::<Vec<u32>>());
    let counts = damage_counts(row);
    assert_eq!(counts[0], "1");
    assert_eq!(counts[1], "10");
    assert_eq!(counts[26], "1182127725");
    assert_eq!(counts[27], "701908264");
    assert_eq!(row.damage_space.to_string(), "10000000000");
    assert_eq!(decimal_sum(&counts), "10000000000");
}

/// T10 (h2): large sides.
#[test]
fn t10_large_sides() {
    let (world, [a, b]) = f0_with(|f| {
        always_success(f);
        set_damage(f, json!([damage_json("success", 0, Some("10d1000"))]));
    });
    assert_eq!(
        pv(&world, b, a, &lim(109_937)),
        tl(PreviewPart::Damage { outcome: S }, 109_938, 109_937)
    );
    let got = pv(&world, b, a, &lim(109_938)).expect("exactly at the limit");
    let row = &got.outcomes[0];
    assert_eq!(row.damage.len(), 9991);
    let values: Vec<u32> = row.damage.iter().map(|(v, _)| *v).collect();
    assert_eq!(values, (10..=10_000).collect::<Vec<u32>>());
    let counts = damage_counts(row);
    assert_eq!(counts[..3], ["1", "10", "55"]);
    assert_eq!(counts[9988..], ["55", "10", "1"]);
    assert_eq!(row.damage_space.to_string(), format!("1{}", "0".repeat(30)));
    assert_eq!(decimal_sum(&counts), format!("1{}", "0".repeat(30)));
}

/// T10 (i): a row with count 0 is still charged, and an outcome without a row
/// is not.
#[test]
fn t10_charged_rows() {
    let damage = || {
        json!([
            damage_json("success", 2, None),
            damage_json("failure", 0, Some("33d2"))
        ])
    };
    let (world, [a, b]) = f0_with(|f| {
        set_table(
            f,
            json!([
                {"min_margin": i64::MIN, "outcome": outcome_json("success")},
                {"min_margin": 1000, "outcome": outcome_json("failure")}
            ]),
            json!([]),
        );
        set_damage(f, damage());
    });
    assert_eq!(
        pv(&world, b, a, &lim(28 + 1188 - 1)),
        tl(PreviewPart::Damage { outcome: F }, 1216, 1215)
    );
    let full = pv(&world, b, a, &lim(1216)).expect("accepted at the cost");
    assert_eq!(full.outcomes[1].count, c(0), "the failure row has no draws");
    assert_eq!(full.outcomes[1].damage_space.to_string(), "8589934592");
    // Control: no failure row, so its dice are never charged.
    let (world, [a, b]) = f0_with(|f| {
        always_success(f);
        set_damage(f, damage());
    });
    assert_eq!(
        pv(&world, b, a, &lim(28)),
        ok(36, vec![r(S, 36, 0, 1, &[(2, 1)])])
    );
}

/// T10 Display and `source`.
#[test]
fn t10_display_and_source() {
    let (world, [a, b]) = f0_world();
    let err = |result: Odds| result.expect_err("refused");
    let actor = err(pv(&world, b, a, &lim(16)));
    let attack = err(pv(&world, b, a, &lim(27)));
    let (world6, [a6, b6]) = t6_world(false, false);
    let opposing = err(pv(&world6, b6, a6, &lim(11)));
    let damage = err(pv(&world6, b6, a6, &lim(53)));
    let rejected_error = err(p(&world, a, b));
    assert_eq!(actor.to_string(), "TooLarge at preview/actor");
    assert_eq!(attack.to_string(), "TooLarge at preview/attack");
    assert_eq!(opposing.to_string(), "TooLarge at preview/opposing");
    assert_eq!(damage.to_string(), "TooLarge at preview/damage");
    for error in [&actor, &attack, &opposing, &damage] {
        assert!(std::error::Error::source(error).is_none(), "{error}");
    }
    assert_eq!(rejected_error.to_string(), "OutOfTurn at combat/active");
    let source = std::error::Error::source(&rejected_error).expect("the wrapped error");
    assert_eq!(source.to_string(), "OutOfTurn at combat/active");
}

// ---------------------------------------------------------------------------
// T11: clamps and repeated outcomes
// ---------------------------------------------------------------------------

/// T11 (B6, B9, B10).
#[test]
fn t11_clamps_and_repeated_outcomes() {
    // (a) The raw total clamps once at `i32::MAX`.
    let (world, [a, b]) = f0_with(|f| {
        edit_ability(f, |ability| ability.dice = "1d4+2147483647".to_owned());
        set_table(
            f,
            json!([
                {"min_margin": i64::MIN, "outcome": outcome_json("failure")},
                {"min_margin": 2_147_483_641_i64, "outcome": outcome_json("success")},
                {"min_margin": 2_147_483_642_i64, "outcome": outcome_json("critical_success")}
            ]),
            json!([]),
        );
    });
    assert_eq!(
        p(&world, b, a),
        ok(
            4,
            vec![
                r(F, 0, 0, 1, &[(0, 1)]),
                r(S, 4, 0, 1, &[(2, 1)]),
                r(CS, 0, 0, 1, &[(0, 1)]),
            ]
        )
    );
    // (b) Damage clamps at both ends and clamped values merge.
    let damage_of = |amount: u32, dice: &str| {
        let (world, [a, b]) = f0_with(|f| {
            always_success(f);
            set_damage(f, json!([damage_json("success", amount, Some(dice))]));
        });
        let got = p(&world, b, a).expect("within the ceiling");
        assert_eq!(got.outcomes.len(), 1);
        assert_eq!(got.outcomes[0].damage_space, c(4));
        got.outcomes[0].damage.clone()
    };
    let pairs = |list: &[(u32, u64)]| -> Vec<(u32, PreviewCount)> {
        list.iter().map(|(v, n)| (*v, c(*n))).collect()
    };
    assert_eq!(damage_of(0, "1d4-10"), pairs(&[(0, 4)]));
    assert_eq!(damage_of(5, "1d4-7"), pairs(&[(0, 2), (1, 1), (2, 1)]));
    assert_eq!(
        damage_of(4_294_967_294, "1d4-2"),
        pairs(&[(4_294_967_293, 1), (4_294_967_294, 1), (4_294_967_295, 2)])
    );
    assert_eq!(
        damage_of(4_294_967_295, "1d4"),
        pairs(&[(4_294_967_295, 4)])
    );
    // (c) One outcome in two bands is one row.
    let (world, [a, b]) = f0_with(|f| {
        edit_ability(f, |ability| {
            ability.dice = "1d20".to_owned();
            ability.defense = DefenseWire::Constant(11);
        });
        set_table(
            f,
            json!([
                {"min_margin": i64::MIN, "outcome": outcome_json("failure")},
                {"min_margin": 0, "outcome": outcome_json("success")},
                {"min_margin": 5, "outcome": outcome_json("failure")}
            ]),
            json!([]),
        );
    });
    assert_eq!(
        p(&world, b, a),
        ok(
            20,
            vec![r(F, 15, 0, 1, &[(0, 1)]), r(S, 5, 0, 1, &[(2, 1)])]
        )
    );
}

// ---------------------------------------------------------------------------
// T12, T13: read-only, deterministic, policy mapping
// ---------------------------------------------------------------------------

/// T12 (B11, B12): the preview does not write, draw or read the RNG.
#[test]
fn t12_read_only_and_deterministic() {
    let (mut world, [a, b]) = t2_world(DefenseWire::TargetStat {
        stat: "gamma".to_owned(),
    });
    assert!(
        world.interners().stat("gamma").is_none(),
        "not interned yet"
    );
    let first = p(&world, b, a);
    assert!(first.is_ok());
    assert!(
        world.interners().stat("gamma").is_none(),
        "the preview interned nothing"
    );
    // Control: execution does intern it.
    perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: b,
            ability: a103(),
            target: a,
        },
    )
    .expect("accepted");
    assert!(world.interners().stat("gamma").is_some());

    // Seeds 1 and 2, and a clone that has drawn, give equal previews.
    let build = |seed: u64| {
        let mut f = Fixture::standard();
        edit_ability(&mut f, |ability| {
            ability.defense = DefenseWire::TargetStat {
                stat: "gamma".to_owned(),
            };
        });
        start(&f, seed)
    };
    let (w1, ids1) = build(1);
    let (w2, ids2) = build(2);
    assert_eq!(ids1, ids2, "entity ids do not depend on the seed");
    let (a, b) = (ids1[0], ids1[1]);
    let mut drawn = w1.clone();
    draw(drawn.rng_mut(), "1d6");
    assert_ne!(drawn, w1, "the clone's RNG moved");
    let want = p(&w1, b, a);
    assert_eq!(
        want,
        ok(
            36,
            vec![r(S, 21, 0, 1, &[(2, 1)]), r(F, 15, 0, 1, &[(0, 1)])]
        )
    );
    assert_eq!(p(&w1, b, a), want, "repeated calls");
    assert_eq!(p(&w2, b, a), want, "another seed");
    assert_eq!(p(&drawn, b, a), want, "a drawn RNG");
}

/// T13 (B15): the policy mapping.
#[test]
fn t13_policy_mapping() {
    use crpg_data::{PreviewWhenWire, PreviewWire};
    let wire = |when, max_cost| PreviewWire { when, max_cost };
    assert_eq!(PreviewRules::from_wire(None), None);
    assert_eq!(
        PreviewRules::from_wire(Some(&wire(PreviewWhenWire::Never, Some(5)))),
        None
    );
    assert_eq!(
        PreviewRules::from_wire(Some(&wire(PreviewWhenWire::Never, None))),
        None
    );
    assert_eq!(
        PreviewRules::from_wire(Some(&wire(PreviewWhenWire::Actionable, None))),
        Some(PreviewRules {
            scope: PreviewScope::Actionable,
            max_cost: 16_777_216
        })
    );
    assert_eq!(
        PreviewRules::from_wire(Some(&wire(PreviewWhenWire::Actionable, Some(7)))),
        Some(PreviewRules {
            scope: PreviewScope::Actionable,
            max_cost: 7
        })
    );
    assert_eq!(
        PreviewRules::from_wire(Some(&wire(PreviewWhenWire::Planning, Some(500)))),
        Some(PreviewRules {
            scope: PreviewScope::Planning,
            max_cost: 500
        })
    );
    assert_eq!(
        PreviewRules::from_wire(Some(&wire(PreviewWhenWire::Planning, None))),
        Some(PreviewRules {
            scope: PreviewScope::Planning,
            max_cost: 16_777_216
        })
    );
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../rulesets/srd-lite/ruleset.json"
    );
    let bytes = std::fs::read(path).expect("the srd-lite ruleset is on disk");
    let crpg_data::Document::Ruleset(ruleset) =
        crpg_data::read_document(&bytes).expect("the ruleset reads")
    else {
        panic!("a ruleset document");
    };
    assert_eq!(ruleset.preview, None);
    assert_eq!(PreviewRules::from_wire(ruleset.preview.as_ref()), None);
    assert_eq!(MAX_PREVIEW_COST, crpg_data::MAX_PREVIEW_COST);
    assert_eq!(MAX_PREVIEW_COST, 16_777_216);
}

// ---------------------------------------------------------------------------
// G1 - G7: agreement with `perform_action`, draw for draw
// ---------------------------------------------------------------------------

/// One world and candidate for the seed sweep.
struct Sweep {
    world: World,
    actor: EntityId,
    target: EntityId,
    actor_dice: &'static str,
    natural_die: Option<usize>,
    opposing_dice: Option<&'static str>,
    rule_faces: Vec<u32>,
    damage_dice: Vec<(Outcome, &'static str)>,
    flat: Vec<(Outcome, u32)>,
}

/// Every face tuple of dice with the given side counts.
fn product(sides: &[u32]) -> Vec<Vec<u32>> {
    let mut out: Vec<Vec<u32>> = vec![Vec::new()];
    for &side in sides {
        out = out
            .iter()
            .flat_map(|prefix| {
                (1..=side).map(move |face| {
                    let mut next = prefix.clone();
                    next.push(face);
                    next
                })
            })
            .collect();
    }
    out
}

fn sides_of(dice: &str) -> Vec<u32> {
    let expr: DiceExpr = dice.parse().expect("the dice parse");
    vec![expr.sides(); expr.count() as usize]
}

/// Evaluates `dice` on `rng`'s combat stream and returns the faces in draw
/// order.
fn faces_of(dice: &str, rng: &mut DeterministicRng) -> Vec<u32> {
    let expr: DiceExpr = dice.parse().expect("the dice parse");
    expr.evaluate(rng, COMBAT_ROLL_STREAM)
        .expect("a valid stream")
        .dice
        .iter()
        .map(|die| die.value)
        .collect()
}

impl Sweep {
    fn dice_of(&self, outcome: Outcome) -> Option<&'static str> {
        self.damage_dice
            .iter()
            .find(|(kind, _)| *kind == outcome)
            .map(|(_, dice)| *dice)
    }

    fn flat_of(&self, outcome: Outcome) -> u32 {
        self.flat
            .iter()
            .find(|(kind, _)| *kind == outcome)
            .map_or(0, |(_, amount)| *amount)
    }
}

/// Runs the sweep and compares `preview_action` with it and with `want`.
fn assert_agrees(sweep: &Sweep, want: Odds) {
    let got = preview_action(&sweep.world, sweep.actor, a103(), sweep.target, &ACT);
    assert_eq!(got, want, "the preview equals the named literal");
    let preview = got.expect("an accepted candidate");

    let mut attack_sides = sides_of(sweep.actor_dice);
    if let Some(opposing) = sweep.opposing_dice {
        attack_sides.extend(sides_of(opposing));
    }
    let attack_total = product(&attack_sides).len();
    let mut damage_total = 0;
    for row in &preview.outcomes {
        if let (false, Some(dice)) = (row.count.is_zero(), sweep.dice_of(row.outcome)) {
            damage_total += product(&sides_of(dice)).len();
        }
    }

    let mut attack: BTreeMap<Vec<u32>, usize> = BTreeMap::new();
    let mut damage: BTreeMap<(usize, Vec<u32>), u32> = BTreeMap::new();
    let mut done = false;
    for seed in 0..20_000_u64 {
        let mut world = sweep.world.clone();
        *world.rng_mut() = DeterministicRng::from_seed(seed);
        let mut peek = DeterministicRng::from_seed(seed);
        let mut tuple = faces_of(sweep.actor_dice, &mut peek);
        if let Some(opposing) = sweep.opposing_dice {
            tuple.extend(faces_of(opposing, &mut peek));
        }
        let outcome = perform_action(
            &mut world,
            &CombatAction::UseAbility {
                actor: sweep.actor,
                ability: a103(),
                target: sweep.target,
            },
        )
        .expect("the sweep's action is accepted")
        .expect("an ability use reports its outcome");
        let row = preview
            .outcomes
            .iter()
            .position(|odds| odds.outcome == outcome.outcome)
            .expect("the executed outcome has a row");
        if let Some(before) = attack.insert(tuple.clone(), row) {
            assert_eq!(before, row, "a repeated tuple {tuple:?} decides alike");
        }
        if let Some(dice) = sweep.dice_of(outcome.outcome) {
            let faces = faces_of(dice, &mut peek);
            if let Some(before) = damage.insert((row, faces.clone()), outcome.damage) {
                assert_eq!(before, outcome.damage, "damage tuple {faces:?} repeats");
            }
        } else {
            assert_eq!(outcome.damage, sweep.flat_of(outcome.outcome));
        }
        if attack.len() == attack_total && damage.len() == damage_total {
            done = true;
            break;
        }
    }
    assert!(done, "every tuple was seen within 20,000 seeds");
    assert_eq!(attack.len(), attack_total);

    for (index, row) in preview.outcomes.iter().enumerate() {
        let mine: Vec<&Vec<u32>> = attack
            .iter()
            .filter(|(_, owner)| **owner == index)
            .map(|(tuple, _)| tuple)
            .collect();
        assert_eq!(row.count, c(mine.len() as u64), "{:?} count", row.outcome);
        let natural = match sweep.natural_die {
            Some(die) => mine
                .iter()
                .filter(|tuple| sweep.rule_faces.contains(&tuple[die]))
                .count(),
            None => 0,
        };
        assert_eq!(row.natural, c(natural as u64), "{:?} natural", row.outcome);
        if row.count.is_zero() {
            continue;
        }
        match sweep.dice_of(row.outcome) {
            Some(_) => {
                let mut histogram: BTreeMap<u32, u64> = BTreeMap::new();
                let mut space = 0_u64;
                for ((owner, _), value) in &damage {
                    if *owner == index {
                        *histogram.entry(*value).or_default() += 1;
                        space += 1;
                    }
                }
                assert_eq!(row.damage_space, c(space), "{:?} space", row.outcome);
                let listed: Vec<(u32, PreviewCount)> =
                    histogram.into_iter().map(|(v, n)| (v, c(n))).collect();
                assert_eq!(row.damage, listed, "{:?} damage", row.outcome);
            }
            None => {
                assert_eq!(row.damage_space, c(1));
                assert_eq!(row.damage, vec![(sweep.flat_of(row.outcome), c(1))]);
            }
        }
    }
}

fn flat_f0() -> Vec<(Outcome, u32)> {
    vec![(S, 2), (F, 0)]
}

/// G1: T1.
#[test]
fn g1_agrees_standard() {
    let (world, [a, b]) = f0_world();
    assert_agrees(
        &Sweep {
            world,
            actor: b,
            target: a,
            actor_dice: "2d6",
            natural_die: None,
            opposing_dice: None,
            rule_faces: vec![],
            damage_dice: vec![],
            flat: flat_f0(),
        },
        ok(36, t1_rows()),
    );
}

/// G2: T3.
#[test]
fn g2_agrees_natural_rules_and_damage_dice() {
    let (world, ids) = start(&t3_fixture(), 1);
    assert_agrees(
        &Sweep {
            world,
            actor: ids[1],
            target: ids[0],
            actor_dice: "1d20",
            natural_die: Some(0),
            opposing_dice: None,
            rule_faces: vec![20, 1, 2],
            damage_dice: vec![(S, "1d4"), (CS, "2d4")],
            flat: vec![(S, 3), (CS, 3), (F, 0)],
        },
        ok(20, t3_rows()),
    );
}

/// G3: T6(d).
#[test]
fn g3_agrees_opposed() {
    let (world, [a, b]) = t6_world(true, true);
    assert_agrees(
        &Sweep {
            world,
            actor: b,
            target: a,
            actor_dice: "1d6",
            natural_die: Some(0),
            opposing_dice: Some("1d6"),
            rule_faces: vec![1],
            damage_dice: vec![(S, "1d6")],
            flat: vec![(S, 1), (F, 0)],
        },
        ok(36, t6_rows(20, 10, Some(6))),
    );
}

/// G4: T5.
#[test]
fn g4_agrees_dropped_natural_die() {
    let (world, ids) = start(&t5_fixture(), 1);
    assert_agrees(
        &Sweep {
            world,
            actor: ids[1],
            target: ids[0],
            actor_dice: "2d4kh1",
            natural_die: Some(1),
            opposing_dice: None,
            rule_faces: vec![4],
            damage_dice: vec![],
            flat: flat_f0(),
        },
        ok(16, t5_rows()),
    );
}

/// G5: T7(a) after its script, and T2's `TargetStat gamma` on a fresh world.
#[test]
fn g5_agrees_effects_and_target_stat() {
    let (world, [a, b]) = t7_world(None, dc_plus_two("target"), 6);
    assert_agrees(
        &Sweep {
            world,
            actor: b,
            target: a,
            actor_dice: "2d6",
            natural_die: None,
            opposing_dice: None,
            rule_faces: vec![],
            damage_dice: vec![],
            flat: flat_f0(),
        },
        ok(
            36,
            vec![r(S, 26, 0, 1, &[(2, 1)]), r(F, 10, 0, 1, &[(0, 1)])],
        ),
    );
    let (world, [a, b]) = t2_world(DefenseWire::TargetStat {
        stat: "gamma".to_owned(),
    });
    assert_agrees(
        &Sweep {
            world,
            actor: b,
            target: a,
            actor_dice: "2d6",
            natural_die: None,
            opposing_dice: None,
            rule_faces: vec![],
            damage_dice: vec![],
            flat: flat_f0(),
        },
        ok(
            36,
            vec![r(S, 21, 0, 1, &[(2, 1)]), r(F, 15, 0, 1, &[(0, 1)])],
        ),
    );
}

/// G6: `F0` with `allow_self_target`, `P(w, B, B)`.
#[test]
fn g6_agrees_self_target() {
    let (world, [_, b]) = f0_with(|f| edit_ability(f, |ability| ability.allow_self_target = true));
    assert_agrees(
        &Sweep {
            world,
            actor: b,
            target: b,
            actor_dice: "2d6",
            natural_die: None,
            opposing_dice: None,
            rule_faces: vec![],
            damage_dice: vec![],
            flat: flat_f0(),
        },
        ok(36, t1_rows()),
    );
}

/// G7: T4's `3d3dl1` and its `3d3` with the natural rule.
#[test]
fn g7_agrees_selections_and_tags() {
    let (world, ids) = start(&t4_fixture("3d3dl1", None, json!([])), 1);
    assert_agrees(
        &Sweep {
            world,
            actor: ids[1],
            target: ids[0],
            actor_dice: "3d3dl1",
            natural_die: None,
            opposing_dice: None,
            rule_faces: vec![],
            damage_dice: vec![],
            flat: vec![],
        },
        t4_expect(27, [0, 0, 1, 3, 7, 9, 7, 0, 0, 0], [0; 10]),
    );
    let (world, ids) = start(
        &t4_fixture(
            "3d3",
            Some(2),
            json!([{"face": 3, "effect": {"type": "shift", "value": 0}}]),
        ),
        1,
    );
    assert_agrees(
        &Sweep {
            world,
            actor: ids[1],
            target: ids[0],
            actor_dice: "3d3",
            natural_die: Some(2),
            opposing_dice: None,
            rule_faces: vec![3],
            damage_dice: vec![],
            flat: vec![],
        },
        t4_expect(
            27,
            [0, 0, 0, 1, 3, 6, 7, 6, 3, 1],
            [0, 0, 0, 0, 0, 1, 2, 3, 2, 1],
        ),
    );
}

// ---------------------------------------------------------------------------
// Review fixes: self-targeted modifiers, Opposed bounds with a natural die,
// and the opposing limb width
// ---------------------------------------------------------------------------

/// T6(a)'s opposed `1d6`, self-targeted, with B carrying a `Roll set 7`
/// effect from an earlier self-hit: one view serves both rolls, so both
/// totals are 7. Damage is flat (a self-hit must not kill B).
fn opposed_self_world() -> (World, [EntityId; 2]) {
    let mut f = t6_fixture(false, false);
    edit_ability(&mut f, |ability| ability.allow_self_target = true);
    set_damage(
        &mut f,
        json!([
            damage_json("success", 1, None),
            damage_json("failure", 0, None)
        ]),
    );
    attach_to_ability(
        &mut f,
        effect_doc(
            141,
            "slf",
            "probe-type",
            "stack_all",
            vec![modifier(142, json!({"type": "roll"}), "set", 7)],
            3,
        ),
    );
    let seed = (0..1000_u64)
        .find(|seed| {
            let mut rng = oracle(*seed);
            let actor = draw(&mut rng, "1d6").total;
            let opposing = draw(&mut rng, "1d6").total;
            actor > opposing
        })
        .expect("a seed whose first hit succeeds");
    let (mut world, ids) = start(&f, seed);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        act(&mut world, b, a103(), b).outcome,
        S,
        "the self-hit succeeds"
    );
    end(&mut world, a);
    (world, [a, b])
}

fn opposed_self_rows() -> Vec<OutcomeOdds> {
    vec![r(F, 36, 0, 1, &[(0, 1)]), r(S, 0, 0, 1, &[(1, 1)])]
}

/// A self-targeted action reads the actor's modifiers on both rolls
/// (B7: the one view serves both sides).
#[test]
fn t15_self_target_folds_the_actors_modifiers() {
    let (world, [_, b]) = opposed_self_world();
    assert_eq!(p(&world, b, b), ok(36, opposed_self_rows()));

    // A self `Dc` through the actor's own view: the defence is B's alpha 6
    // plus the effect's 2, so a total of 8 or less succeeds.
    let mut f = Fixture::standard();
    edit_ability(&mut f, |ability| ability.allow_self_target = true);
    attach_to_ability(&mut f, dc_plus_two("target"));
    let (mut world, ids) = start(&f, smallest_seed(6));
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(act(&mut world, b, a103(), b).outcome, S);
    end(&mut world, a);
    assert_eq!(p(&world, b, b), ok(36, a_rows()));
}

/// G8: the self-targeted opposed world against execution.
#[test]
fn g8_agrees_self_target_opposed_modifier() {
    let (world, [_, b]) = opposed_self_world();
    assert_agrees(
        &Sweep {
            world,
            actor: b,
            target: b,
            actor_dice: "1d6",
            natural_die: None,
            opposing_dice: Some("1d6"),
            rule_faces: vec![],
            damage_dice: vec![],
            flat: vec![(S, 1), (F, 0)],
        },
        ok(36, opposed_self_rows()),
    );
}

/// T10 (d2): Opposed with a natural rule (T6(c), T6(d)). The actor's `1d6`
/// with one rule face costs 8 and has 7 rows; the opposing `1d6` is never read
/// with a natural face, so it costs 6 and has 6 rows. Attack is 7 * 6 = 42,
/// damage 6: 8 + 6 + 42 + 6 = 62.
#[test]
fn t10_bounds_opposed_with_a_natural_die() {
    let dmg = PreviewPart::Damage { outcome: S };
    for (bonus, rows) in [
        (false, t6_rows(15, 15, Some(6))),
        (true, t6_rows(20, 10, Some(6))),
    ] {
        let (world, [a, b]) = t6_world(bonus, true);
        let cases = [
            (62, ok(36, rows)),
            (61, tl(dmg, 62, 61)),
            (56, tl(dmg, 62, 56)),
            (55, tl(PreviewPart::Attack, 62, 55)),
            (14, tl(PreviewPart::Attack, 62, 14)),
            (13, tl(PreviewPart::Opposing, 62, 13)),
            (8, tl(PreviewPart::Opposing, 62, 8)),
            (7, tl(PreviewPart::Actor, 62, 7)),
        ];
        for (limit, want) in cases {
            assert_eq!(
                pv(&world, b, a, &lim(limit)),
                want,
                "bonus {bonus}, limit {limit}"
            );
        }
    }
}

/// T10 (k): an opposing roll wider than one limb. `2d6` (17, 11 rows, 1
/// limb) against `33d2` (1188, 34 rows, 2 limbs): attack is 11 * 34 * 1 * 2
/// = 748, so the cost is 17 + 1188 + 748 = 1953.
#[test]
fn t10_bounds_opposing_limbs() {
    let (world, [a, b]) = f0_with(|f| defend_opposed(f, "33d2", &[]));
    let full = pv(&world, b, a, &lim(1953)).expect("accepted at the cost");
    assert_eq!(full.attack_space.to_string(), "309237645312", "36 * 2^33");
    assert_eq!(
        pv(&world, b, a, &lim(1952)),
        tl(PreviewPart::Attack, 1953, 1952)
    );
    assert_eq!(
        pv(&world, b, a, &lim(1205)),
        tl(PreviewPart::Attack, 1953, 1205)
    );
    assert_eq!(
        pv(&world, b, a, &lim(1204)),
        tl(PreviewPart::Opposing, 1953, 1204)
    );
    assert_eq!(pv(&world, b, a, &lim(16)), tl(PreviewPart::Actor, 1953, 16));
}
