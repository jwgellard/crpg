//! T137a kernel-range resolution (R3–R6): derived and typed stats, `Stat`
//! effect modifiers, the `Constant` and `ActorStat` defences, bonus stats,
//! and the mid-fight `StatEvaluation` legality stage.
//!
//! The creatures are the standard fixture's: B (alpha 6, beta 7, gamma 5,
//! health 6) acts first, A (alpha 8, beta 6, gamma 7, health 10) second. A
//! hit's defence is read back as `roll.total - margin`, which is exact
//! because the raw total is the dice trace and the margin is the folded total
//! minus the defence.

#[allow(dead_code)]
mod support;

use crpg_core::{Interners, Ulid};
use crpg_data::DefenseWire;
use crpg_sim::{legal_actions, perform_action, validate_action, CombatAction, CombatError};
use serde_json::json;
use support::surface::{
    act, add_attribute, add_derived, add_stat, always_success, attach_to_ability, binary,
    edit_ability, effect_doc, end, lit, modifier, set_stat, start, stat, stat_target, try_start,
};
use support::Fixture;

/// The defence one hit faced.
fn defence(outcome: &crpg_sim::ActionOutcome) -> i64 {
    i64::from(outcome.roll.total) - outcome.margin
}

/// Edits the standard ability's defence.
fn defend(fixture: &mut Fixture, defense: DefenseWire) {
    edit_ability(fixture, |ability| ability.defense = defense);
}

/// Runs the pinned three-hit script of B on A, ending every other turn:
/// round 0 hit, round 1 hit, round 2 none, round 3 hit. Returns the
/// defence each hit faced. The effect on A lasts two rounds, and a hit
/// refreshes it, so it covers the round-1 hit and lapses before round 3.
fn three_hits(fixture: &Fixture) -> Vec<i64> {
    let ability = fixture.ability;
    let (mut world, ids) = start(fixture, 21);
    let (a, b) = (ids[0], ids[1]);
    let mut seen = Vec::new();
    seen.push(defence(&act(&mut world, b, ability, a)));
    end(&mut world, a);
    seen.push(defence(&act(&mut world, b, ability, a)));
    end(&mut world, a);
    end(&mut world, b);
    end(&mut world, a);
    seen.push(defence(&act(&mut world, b, ability, a)));
    seen
}

/// A fixture where B's hit always succeeds, defended by `guard`, a derived
/// stat of A's `gamma` and `beta`, and applies one effect to A.
fn stat_effect_fixture(op: &str, value: i32, defense_stat: &str) -> Fixture {
    let mut fixture = Fixture::standard();
    add_derived(
        &mut fixture,
        "guard",
        binary("add", stat("gamma"), stat("beta")),
    );
    always_success(&mut fixture);
    defend(
        &mut fixture,
        DefenseWire::TargetStat {
            stat: defense_stat.to_owned(),
        },
    );
    attach_to_ability(
        &mut fixture,
        effect_doc(
            141,
            "target",
            "probe-type",
            "stack_all",
            vec![modifier(142, stat_target("gamma"), op, value)],
            2,
        ),
    );
    fixture
}

/// R3 (B6, B9): a derived checked attribute is evaluated through the
/// kernel.
#[test]
fn a_derived_attribute_supplies_the_defence() {
    let mut fixture = Fixture::standard();
    add_derived(
        &mut fixture,
        "power",
        binary(
            "add",
            binary("multiply", stat("alpha"), lit(2)),
            stat("beta"),
        ),
    );
    add_attribute(&mut fixture, "power");
    edit_ability(&mut fixture, |ability| {
        ability.attribute = "power".to_owned();
    });
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 3);
    let outcome = act(&mut world, ids[1], ability, ids[0]);
    assert_eq!(defence(&outcome), 6 * 2 + 7, "B's power");
}

/// R3 (B6, B10): a derived target-stat defence gives a precomputed margin.
#[test]
fn a_derived_target_stat_supplies_the_defence() {
    let fixture = stat_effect_fixture("add", 0, "guard");
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 3);
    let outcome = act(&mut world, ids[1], ability, ids[0]);
    assert_eq!(defence(&outcome), 7 + 6, "A's guard is gamma plus beta");
}

/// R3 (B10, B7): the world interner after a `TargetStat` action is the
/// pre-T137 sequence: the roll tag, and only the defence stat.
#[test]
fn a_target_stat_action_interns_only_its_defence_stat() {
    for stat_name in ["gamma", "guard"] {
        let fixture = stat_effect_fixture("add", 0, stat_name);
        let ability = fixture.ability;
        let (mut world, ids) = start(&fixture, 3);
        assert!(world.interners().stats().is_empty());
        act(&mut world, ids[1], ability, ids[0]);
        let mut expected = Interners::new();
        expected.intern_tag("roll");
        expected.intern_stat(stat_name);
        assert_eq!(*world.interners(), expected, "{stat_name}");
    }
}

/// R3 (B18): a `Stat` `Add` effect shifts the next margin by the exact
/// delta while attached, and by none after expiry, derived dependants
/// included.
#[test]
fn a_stat_add_effect_shifts_the_defence_while_attached() {
    assert_eq!(
        three_hits(&stat_effect_fixture("add", 3, "guard")),
        [13, 16, 13],
        "the derived defence follows its base"
    );
    assert_eq!(
        three_hits(&stat_effect_fixture("add", 3, "gamma")),
        [7, 10, 7],
        "the base defence is shifted too"
    );
}

/// R3 (B18): a `Stat` `Set` effect replaces the base value, and derived
/// dependants follow it.
#[test]
fn a_stat_set_effect_replaces_the_defence_while_attached() {
    assert_eq!(
        three_hits(&stat_effect_fixture("set", 20, "guard")),
        [13, 26, 13]
    );
    assert_eq!(
        three_hits(&stat_effect_fixture("set", 20, "gamma")),
        [7, 20, 7]
    );
}

/// R3 (B8): a `Stat` effect on the actor shifts a derived checked
/// attribute through the actor's own view.
#[test]
fn a_stat_effect_on_the_actor_shifts_a_derived_attribute() {
    let mut fixture = Fixture::standard();
    add_derived(
        &mut fixture,
        "power",
        binary("add", stat("alpha"), stat("beta")),
    );
    add_attribute(&mut fixture, "power");
    always_success(&mut fixture);
    edit_ability(&mut fixture, |ability| {
        ability.attribute = "power".to_owned();
    });
    attach_to_ability(
        &mut fixture,
        effect_doc(
            141,
            "slf",
            "probe-type",
            "stack_all",
            vec![modifier(142, stat_target("beta"), "add", 5)],
            2,
        ),
    );
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 8);
    let (a, b) = (ids[0], ids[1]);
    let first = act(&mut world, b, ability, a);
    end(&mut world, a);
    let second = act(&mut world, b, ability, a);
    assert_eq!(defence(&first), 6 + 7);
    assert_eq!(defence(&second), 6 + 7 + 5);
}

/// The two-hit script of B on A, returning the defence of each hit. The
/// first hit attaches the effect, which lasts for both.
fn two_hits(fixture: &Fixture) -> (i64, i64) {
    let ability = fixture.ability;
    let (mut world, ids) = start(fixture, 13);
    let (a, b) = (ids[0], ids[1]);
    let first = defence(&act(&mut world, b, ability, a));
    end(&mut world, a);
    let second = defence(&act(&mut world, b, ability, a));
    (first, second)
}

/// A fixture whose hit always succeeds against `defense` and applies one
/// `Dc` effect of `value`, aimed at `aim`.
fn dc_effect_fixture(defense: DefenseWire, aim: &str, value: i32) -> Fixture {
    let mut fixture = Fixture::standard();
    always_success(&mut fixture);
    defend(&mut fixture, defense);
    attach_to_ability(
        &mut fixture,
        effect_doc(
            141,
            aim,
            "probe-type",
            "stack_all",
            vec![modifier(142, json!({"type": "dc"}), "add", value)],
            3,
        ),
    );
    fixture
}

/// R4 (B11): `Constant` equals the same-valued DC fight.
#[test]
fn constant_equals_the_same_valued_dc_fight() {
    let mut constant = Fixture::standard();
    defend(&mut constant, DefenseWire::Constant(9));
    let mut attribute = Fixture::standard();
    attribute.set_alpha(8, 9);
    let ability = attribute.ability;
    let (mut left, left_ids) = start(&constant, 5);
    let (mut right, right_ids) = start(&attribute, 5);
    let a = act(&mut left, left_ids[1], ability, left_ids[0]);
    let b = act(&mut right, right_ids[1], ability, right_ids[0]);
    assert_eq!(a, b);
    assert_eq!(defence(&a), 9);
}

/// R4 (B11): `Constant` folds the target's `Dc` effect, and not the
/// actor's.
#[test]
fn constant_folds_the_target_dc_effect_only() {
    let folded = dc_effect_fixture(DefenseWire::Constant(9), "target", 2);
    assert_eq!(two_hits(&folded), (9, 11));
    let ignored = dc_effect_fixture(DefenseWire::Constant(9), "slf", 2);
    assert_eq!(two_hits(&ignored), (9, 9));
}

/// R4 (B12): `ActorStat` reads the actor's stat and folds the actor's `Dc`
/// effect, and ignores the target's.
#[test]
fn actor_stat_folds_the_actor_dc_effect_only() {
    let defense = || DefenseWire::ActorStat {
        stat: "alpha".to_owned(),
    };
    let folded = dc_effect_fixture(defense(), "slf", 4);
    assert_eq!(two_hits(&folded), (6, 10), "B's alpha, then plus its Dc");
    let ignored = dc_effect_fixture(defense(), "target", 5);
    assert_eq!(two_hits(&ignored), (6, 6));
}

/// R4 (B12): an `ActorStat` self-target reads the one view.
#[test]
fn actor_stat_on_a_self_target_reads_the_one_view() {
    let mut fixture = dc_effect_fixture(
        DefenseWire::ActorStat {
            stat: "alpha".to_owned(),
        },
        "slf",
        4,
    );
    edit_ability(&mut fixture, |ability| ability.allow_self_target = true);
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 13);
    let (a, b) = (ids[0], ids[1]);
    let first = defence(&act(&mut world, b, ability, b));
    end(&mut world, a);
    let second = defence(&act(&mut world, b, ability, b));
    assert_eq!((first, second), (6, 10));
}

/// R4 (B12, B21): health, non-`Int` and undeclared names are `MissingStat`
/// for both stat defences.
#[test]
fn health_non_int_and_undeclared_names_are_missing() {
    for name in ["health", "flag", "nope"] {
        for wrap in [
            |stat: String| DefenseWire::ActorStat { stat },
            |stat: String| DefenseWire::TargetStat { stat },
        ] {
            let mut fixture = Fixture::standard();
            add_stat(&mut fixture, json!({"name": "flag", "kind": "bool"}));
            defend(&mut fixture, wrap(name.to_owned()));
            assert_eq!(
                try_start(&fixture, 1).expect_err("the name is refused"),
                CombatError::MissingStat {
                    stat: name.to_owned()
                },
                "{name}"
            );
        }
    }
}

/// A fixture where B's roll gains `bonus` and faces a constant 50, with
/// every hit successful.
fn bonus_fixture(bonus: &[&str]) -> Fixture {
    let mut fixture = Fixture::standard();
    always_success(&mut fixture);
    defend(&mut fixture, DefenseWire::Constant(50));
    edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = Some(bonus.iter().map(|name| (*name).to_owned()).collect());
    });
    fixture
}

/// R5 (B14): the margin shifts by the exact bonus sum.
#[test]
fn bonus_stats_shift_the_margin_by_their_sum() {
    for (bonus, sum) in [(vec![], 0), (vec!["beta"], 7), (vec!["beta", "gamma"], 12)] {
        let fixture = bonus_fixture(&bonus);
        let ability = fixture.ability;
        let (mut world, ids) = start(&fixture, 4);
        let outcome = act(&mut world, ids[1], ability, ids[0]);
        assert_eq!(
            outcome.margin,
            i64::from(outcome.roll.total) + sum - 50,
            "{bonus:?}"
        );
    }
}

/// R5 (B14): a derived bonus stat is read fully modified.
#[test]
fn a_derived_bonus_stat_is_read_through_the_kernel() {
    let mut fixture = bonus_fixture(&["twice"]);
    add_derived(
        &mut fixture,
        "twice",
        binary("multiply", stat("beta"), lit(2)),
    );
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 4);
    let outcome = act(&mut world, ids[1], ability, ids[0]);
    assert_eq!(outcome.margin, i64::from(outcome.roll.total) + 14 - 50);
}

/// R5 (B14): an effect under the `HighestBonusWorstPenalty` policy does not
/// suppress the bonus.
#[test]
fn a_highest_bonus_policy_effect_does_not_suppress_the_bonus() {
    let mut fixture = bonus_fixture(&["beta", "gamma"]);
    attach_to_ability(
        &mut fixture,
        effect_doc(
            141,
            "slf",
            "probe-type",
            "highest_bonus_worst_penalty",
            vec![modifier(142, json!({"type": "roll"}), "add", 1)],
            3,
        ),
    );
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 4);
    let (a, b) = (ids[0], ids[1]);
    let first = act(&mut world, b, ability, a);
    end(&mut world, a);
    let second = act(&mut world, b, ability, a);
    assert_eq!(first.margin, i64::from(first.roll.total) + 12 - 50);
    assert_eq!(
        second.margin,
        i64::from(second.roll.total) + 1 + 12 - 50,
        "the effect and the bonus both apply"
    );
}

/// R5 (B14, D9): a reserved-type effect is refused when an ability carries
/// bonus stats, and starts without them.
#[test]
fn the_reserved_bonus_type_is_refused_only_with_bonus_stats() {
    let reserved = |bonus: &[&str]| {
        let mut fixture = bonus_fixture(bonus);
        attach_to_ability(
            &mut fixture,
            effect_doc(
                141,
                "slf",
                crpg_sim::COMBAT_BONUS_MOD_TYPE,
                "stack_all",
                vec![modifier(142, json!({"type": "roll"}), "add", 1)],
                1,
            ),
        );
        fixture
    };
    assert_eq!(
        try_start(&reserved(&["beta"]), 1).expect_err("refused"),
        CombatError::InvalidEffect {
            effect: Ulid::from_u128(141)
        }
    );
    assert!(try_start(&reserved(&[]), 1).is_ok());
}

/// R5 (B14): an authored modifier using the first synthetic id does not
/// collide with it; the synthetic id moves on (the kernel rejects a
/// duplicate identity within a view).
#[test]
fn an_authored_modifier_with_the_first_id_does_not_collide() {
    let mut fixture = bonus_fixture(&["beta"]);
    attach_to_ability(
        &mut fixture,
        effect_doc(
            141,
            "slf",
            "probe-type",
            "stack_all",
            vec![modifier(1, json!({"type": "roll"}), "add", 1)],
            3,
        ),
    );
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 4);
    let (a, b) = (ids[0], ids[1]);
    act(&mut world, b, ability, a);
    end(&mut world, a);
    let second = act(&mut world, b, ability, a);
    assert_eq!(second.margin, i64::from(second.roll.total) + 1 + 7 - 50);
}

/// The fixture of R6: B's hit always succeeds against `ratio`, A's
/// `alpha / beta`, and zeroes A's `beta` for three rounds. The ability keeps
/// the turn, so one pool point funds one use per turn.
fn zeroing_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    add_derived(
        &mut fixture,
        "ratio",
        binary("divide", stat("alpha"), stat("beta")),
    );
    always_success(&mut fixture);
    defend(
        &mut fixture,
        DefenseWire::TargetStat {
            stat: "ratio".to_owned(),
        },
    );
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

fn use_ability(
    actor: crpg_core::EntityId,
    ability: Ulid,
    target: crpg_core::EntityId,
) -> CombatAction {
    CombatAction::UseAbility {
        actor,
        ability,
        target,
    }
}

/// R6 (B15): a `Stat` effect zeroes a divisor mid-fight; the action is
/// illegal with `StatEvaluation`, `InsufficientAction` wins when both hold,
/// `legal_actions` omits the candidate, and `perform_action` rejects with
/// the world unchanged.
#[test]
fn a_zeroed_divisor_makes_the_action_illegal() {
    let fixture = zeroing_fixture();
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 17);
    let (a, b) = (ids[0], ids[1]);
    act(&mut world, b, ability, a);
    let blocked = use_ability(b, ability, a);
    assert!(
        matches!(
            validate_action(&world, &blocked),
            Err(CombatError::InsufficientAction { .. })
        ),
        "the spent pool wins over the evaluation stage"
    );
    end(&mut world, b);
    end(&mut world, a);
    assert_eq!(world.combat().expect("encounter").active, Some(b));

    let error = CombatError::StatEvaluation {
        stat: "ratio".to_owned(),
    };
    assert_eq!(validate_action(&world, &blocked), Err(error.clone()));
    let options = legal_actions(&world, b).expect("B can act");
    assert!(!options.contains(&blocked), "{options:?}");
    assert!(
        options.contains(&use_ability(b, ability, b)),
        "a candidate whose stats evaluate stays legal"
    );
    assert!(options.contains(&CombatAction::EndTurn { actor: b }));

    let before = world.clone();
    assert_eq!(perform_action(&mut world, &blocked), Err(error.clone()));
    assert_eq!(world, before, "a rejected action changes nothing");
    assert_eq!(error.to_string(), "StatEvaluation at combat/stats");
}

/// R6 (B15): the stage names the defence-side stat before the actor's bonus
/// stats.
#[test]
fn the_defense_stat_is_queried_before_the_bonus_stats() {
    let scenario = |defense_stat: &str, bonus: &[&str]| {
        let mut fixture = Fixture::standard();
        add_derived(
            &mut fixture,
            "first",
            binary("divide", stat("alpha"), stat("beta")),
        );
        add_derived(
            &mut fixture,
            "second",
            binary("divide", stat("alpha"), stat("gamma")),
        );
        always_success(&mut fixture);
        attach_to_ability(
            &mut fixture,
            effect_doc(
                141,
                "target",
                "probe-type",
                "stack_all",
                vec![
                    modifier(142, stat_target("beta"), "set", 0),
                    modifier(143, stat_target("gamma"), "set", 0),
                ],
                3,
            ),
        );
        edit_ability(&mut fixture, |ability| {
            ability.allow_self_target = true;
            ability.defense = DefenseWire::TargetStat {
                stat: defense_stat.to_owned(),
            };
            ability.bonus_stats = Some(bonus.iter().map(|name| (*name).to_owned()).collect());
        });
        let ability = fixture.ability;
        let (mut world, ids) = start(&fixture, 2);
        let (a, b) = (ids[0], ids[1]);
        act(&mut world, b, ability, b);
        end(&mut world, a);
        validate_action(&world, &use_ability(b, ability, b)).expect_err("the stage fails")
    };
    assert_eq!(
        scenario("first", &["second"]),
        CombatError::StatEvaluation {
            stat: "first".to_owned()
        }
    );
    assert_eq!(
        scenario("alpha", &["second"]),
        CombatError::StatEvaluation {
            stat: "second".to_owned()
        }
    );
}

/// A fixture whose target-stat defence is a derived stat reading health:
/// `vigour = health - 0`.
fn current_health_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    add_derived(
        &mut fixture,
        "vigour",
        binary("subtract", stat("health"), lit(0)),
    );
    always_success(&mut fixture);
    defend(
        &mut fixture,
        DefenseWire::TargetStat {
            stat: "vigour".to_owned(),
        },
    );
    fixture
}

/// R3 (B6, D5, B15): a derived stat reading health is evaluated from the
/// *current* health, so each hit lowers the next defence by the exact
/// damage dealt; max or start-time health would repeat the opening value.
#[test]
fn a_derived_health_stat_reads_current_health() {
    let fixture = current_health_fixture();
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 31);
    let (a, b) = (ids[0], ids[1]);
    let mut seen = Vec::new();
    for expected in [10, 8, 6] {
        let health = world.combatants().get(a).expect("A").health();
        assert_eq!(health, expected, "the hit before this one dealt 2");
        let outcome = act(&mut world, b, ability, a);
        assert_eq!(defence(&outcome), i64::from(health), "A's vigour");
        assert_eq!(
            world.combatants().get(a).expect("A").health(),
            health - 2,
            "one success deals 2"
        );
        seen.push(defence(&outcome));
        end(&mut world, a);
    }
    assert_eq!(
        seen,
        [10, 8, 6],
        "each hit's defence is the health it faced"
    );
}

/// A fixture whose target-stat defence is `alpha / (health - 6)`, which
/// reaches zero when A's health falls to 6. B's health is 7 so its divisor
/// stays nonzero at start. The turn does not end, so the pool funds the
/// three uses.
fn health_divisor_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    add_derived(
        &mut fixture,
        "ratio",
        binary(
            "divide",
            stat("alpha"),
            binary("subtract", stat("health"), lit(6)),
        ),
    );
    always_success(&mut fixture);
    defend(
        &mut fixture,
        DefenseWire::TargetStat {
            stat: "ratio".to_owned(),
        },
    );
    edit_ability(&mut fixture, |ability| ability.ends_turn = false);
    fixture.ruleset.pools[0].max = 3;
    let b = fixture.creature_b;
    set_stat(&mut fixture, b, "health", 7);
    fixture
}

/// R6 (B15, B6): a health-driven divisor reaches zero after damage;
/// `validate_action` returns `StatEvaluation` naming the derived stat,
/// `legal_actions` omits the candidate, and `perform_action` rejects with
/// the world unchanged. Max or start-time health would keep the divisor
/// nonzero and the action legal.
#[test]
fn a_health_driven_divisor_fails_the_stage_after_damage() {
    let fixture = health_divisor_fixture();
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 17);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(defence(&act(&mut world, b, ability, a)), 2, "8 / (10 - 6)");
    assert_eq!(defence(&act(&mut world, b, ability, a)), 4, "8 / (8 - 6)");
    assert_eq!(world.combatants().get(a).expect("A").health(), 6);

    let blocked = use_ability(b, ability, a);
    let error = CombatError::StatEvaluation {
        stat: "ratio".to_owned(),
    };
    assert_eq!(validate_action(&world, &blocked), Err(error.clone()));
    let options = legal_actions(&world, b).expect("B can act");
    assert!(!options.contains(&blocked), "{options:?}");
    assert!(
        options.contains(&CombatAction::EndTurn { actor: b }),
        "the turn can still end"
    );

    let before = world.clone();
    assert_eq!(perform_action(&mut world, &blocked), Err(error));
    assert_eq!(world, before, "a rejected action changes nothing");
}

/// R3 (B18 over B12): a `Stat` `Add` effect on the actor shifts an
/// `ActorStat` defence by the exact delta while attached, and by none after
/// expiry.
#[test]
fn a_stat_add_effect_shifts_an_actor_stat_defence_while_attached() {
    let mut fixture = Fixture::standard();
    always_success(&mut fixture);
    defend(
        &mut fixture,
        DefenseWire::ActorStat {
            stat: "alpha".to_owned(),
        },
    );
    attach_to_ability(
        &mut fixture,
        effect_doc(
            141,
            "slf",
            "probe-type",
            "stack_all",
            vec![modifier(142, stat_target("alpha"), "add", 4)],
            2,
        ),
    );
    assert_eq!(
        three_hits(&fixture),
        [6, 10, 6],
        "B's alpha 6, plus the attached 4, back to 6 after expiry"
    );
}

/// The pinned three-hit script's margins, each minus its roll total: the
/// bonus the actor's roll fold carried. The effect lasts two rounds and a
/// hit refreshes it, so it covers the round-1 hit and lapses before round 3.
fn three_bonus_folds(fixture: &Fixture) -> Vec<i64> {
    let ability = fixture.ability;
    let (mut world, ids) = start(fixture, 27);
    let (a, b) = (ids[0], ids[1]);
    let fold = |outcome: &crpg_sim::ActionOutcome| outcome.margin - i64::from(outcome.roll.total);
    let mut seen = Vec::new();
    seen.push(fold(&act(&mut world, b, ability, a)));
    end(&mut world, a);
    seen.push(fold(&act(&mut world, b, ability, a)));
    end(&mut world, a);
    end(&mut world, b);
    end(&mut world, a);
    seen.push(fold(&act(&mut world, b, ability, a)));
    seen
}

/// R3/R5 (B18 over B14): a `Stat` `Add` effect on a bonus stat shifts the
/// margin by the exact delta while attached, and by none after expiry. The
/// constant defence is 0, so a margin minus its roll total is the folded
/// bonus itself.
#[test]
fn a_stat_add_effect_shifts_a_bonus_stat_while_attached() {
    let mut fixture = Fixture::standard();
    always_success(&mut fixture);
    defend(&mut fixture, DefenseWire::Constant(0));
    edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = Some(vec!["gamma".to_owned()]);
    });
    attach_to_ability(
        &mut fixture,
        effect_doc(
            141,
            "slf",
            "probe-type",
            "stack_all",
            vec![modifier(142, stat_target("gamma"), "add", 4)],
            2,
        ),
    );
    assert_eq!(
        three_bonus_folds(&fixture),
        [5, 9, 5],
        "B's gamma 5, plus the attached 4, back to 5 after expiry"
    );
}

/// R6 (B15): a fight without derived stats never reaches the stage.
#[test]
fn a_fight_without_derived_stats_never_reaches_the_stage() {
    let mut fixture = Fixture::standard();
    always_success(&mut fixture);
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
        ability.ends_turn = false;
        ability.defense = DefenseWire::TargetStat {
            stat: "beta".to_owned(),
        };
    });
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 17);
    let (a, b) = (ids[0], ids[1]);
    act(&mut world, b, ability, a);
    end(&mut world, b);
    end(&mut world, a);
    let action = use_ability(b, ability, a);
    assert_eq!(validate_action(&world, &action), Ok(()));
    let outcome = act(&mut world, b, ability, a);
    assert_eq!(defence(&outcome), 0, "A's zeroed beta is the defence");
}
