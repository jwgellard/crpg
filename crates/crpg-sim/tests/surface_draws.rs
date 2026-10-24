//! T137b draws (R9–R11): the `Opposed` defence, damage dice and rolled
//! initiative, plus the start errors their data introduces.
//!
//! Every expected number is precomputed by evaluating the authored
//! `DiceExpr`s on a clone of the world's own RNG, in the order the contract
//! pins, so a wrong draw order or a missing or extra draw fails an exact
//! comparison.
//!
//! The creatures are the standard fixture's: B (alpha 6, beta 7, gamma 5,
//! health 6, initiative 9) acts first, A (alpha 8, beta 6, gamma 7, health
//! 10, initiative 10) second. The standard table is roll-under: a margin of
//! zero or less succeeds.

#[allow(dead_code)]
mod support;

use crpg_core::Ulid;
use crpg_sim::{
    perform_action, request_mode, state_hash, validate_action, CombatAction, CombatError,
    EncounterMode, World, COMBAT_ROLL_STREAM,
};
use serde_json::json;
use support::draws::{
    always_failure, defend_opposed, died_events, draw, grant, oracle, set_authored_initiative,
    set_damage, set_initiative_rule, set_table, set_tags, start_into,
};
use support::rt::{bytes_of, capable, ticks, timeline_of};
use support::surface::{
    act, add_derived, always_success, attach_to_ability, binary, edit_ability, effect_doc, end,
    lit, modifier, start, stat, stat_target,
};
use support::Fixture;

/// R9 (B13): the actor's dice draw first and the opposing dice second, on
/// one stream, and the margin is the difference of the two totals.
#[test]
fn opposed_draws_actor_then_target_and_the_margin_is_the_difference() {
    let mut fixture = Fixture::standard();
    defend_opposed(&mut fixture, "3d4", &[]);
    let ability = fixture.ability;
    for seed in [1, 2, 3, 4, 5, 6] {
        let (mut world, ids) = start(&fixture, seed);
        let mut expected = oracle(seed);
        let actor = draw(&mut expected, "2d6");
        let opposing = draw(&mut expected, "3d4");
        let outcome = act(&mut world, ids[1], ability, ids[0]);
        assert_eq!(outcome.roll, actor, "seed {seed}: the actor's trace");
        assert_eq!(
            outcome.margin,
            i64::from(actor.total) - i64::from(opposing.total),
            "seed {seed}"
        );
        assert_eq!(
            *world.rng_mut(),
            expected,
            "seed {seed}: exactly two draws, in this order"
        );
    }
}

/// R9 (B13, B14): the target's `Roll` effect and the opposing bonus fold into
/// the opposing total, and the actor's bonus into the actor's; the target's
/// `Dc` effect folds nowhere.
#[test]
fn opposed_folds_the_targets_roll_effect_and_bonus_but_not_its_dc_effect() {
    let mut fixture = Fixture::standard();
    always_success(&mut fixture);
    defend_opposed(&mut fixture, "2d6", &["beta"]);
    edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = Some(vec!["alpha".to_owned()]);
    });
    attach_to_ability(
        &mut fixture,
        effect_doc(
            141,
            "target",
            "probe-type",
            "stack_all",
            vec![
                modifier(142, json!({"type": "roll"}), "add", 5),
                modifier(143, json!({"type": "dc"}), "add", 100),
            ],
            2,
        ),
    );
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 8);
    let (a, b) = (ids[0], ids[1]);

    // Round 0: no effect on A yet. B's bonus is alpha 6, A's opposing bonus
    // is beta 6.
    let mut expected = oracle(8);
    let actor = draw(&mut expected, "2d6");
    let opposing = draw(&mut expected, "2d6");
    let first = act(&mut world, b, ability, a);
    assert_eq!(
        first.margin,
        i64::from(actor.total + 6) - i64::from(opposing.total + 6)
    );

    // Round 1: the effect (Roll +5, Dc +100) is attached to A.
    end(&mut world, a);
    let mut expected = world.rng_mut().clone();
    let actor = draw(&mut expected, "2d6");
    let opposing = draw(&mut expected, "2d6");
    let second = act(&mut world, b, ability, a);
    assert_eq!(
        second.margin,
        i64::from(actor.total + 6) - i64::from(opposing.total + 6 + 5),
        "A's Roll effect folds into its opposing total; its Dc effect does not"
    );
    assert_eq!(*world.rng_mut(), expected);
}

/// R9 (B13): a natural rule fires on the actor's die only, never on the
/// opposing roll.
#[test]
fn a_natural_rule_never_fires_on_the_opposing_roll() {
    let mut fixture = Fixture::standard();
    defend_opposed(&mut fixture, "1d6", &[]);
    edit_ability(&mut fixture, |ability| {
        ability.dice = "1d6".to_owned();
        ability.natural_die = Some(0);
    });
    set_table(
        &mut fixture,
        json!([
            {"min_margin": i64::MIN, "outcome": {"type": "success"}},
            {"min_margin": 1, "outcome": {"type": "failure"}},
            {"min_margin": 1000, "outcome": {"type": "critical_failure"}}
        ]),
        json!([{"face": 1, "effect": {"type": "override",
                "value": {"type": "critical_failure"}}}]),
    );
    let ability = fixture.ability;
    let (mut quiet, mut control) = (None, None);
    for seed in 0..400_u64 {
        let mut probe = oracle(seed);
        let actor = draw(&mut probe, "1d6").dice[0].value;
        let opposing = draw(&mut probe, "1d6").dice[0].value;
        if actor != 1 && opposing == 1 && quiet.is_none() {
            quiet = Some(seed);
        }
        if actor == 1 && control.is_none() {
            control = Some(seed);
        }
    }
    let (quiet, control) = (
        quiet.expect("a seed with only the opposing face 1"),
        control.expect("a seed with the actor's face 1"),
    );

    let (mut world, ids) = start(&fixture, quiet);
    let outcome = act(&mut world, ids[1], ability, ids[0]);
    assert_ne!(
        outcome.outcome,
        crpg_rules::Outcome::CriticalFailure,
        "the opposing roll's natural 1 does nothing"
    );
    let (mut world, ids) = start(&fixture, control);
    let outcome = act(&mut world, ids[1], ability, ids[0]);
    assert_eq!(
        outcome.outcome,
        crpg_rules::Outcome::CriticalFailure,
        "the actor's natural 1 still overrides"
    );
}

/// R9 (B14): a self-targeted `Opposed` use has one view, so both bonus lists
/// fold into both rolls and cancel in the margin.
#[test]
fn a_self_targeted_opposed_use_concatenates_both_bonus_lists() {
    let mut fixture = Fixture::standard();
    defend_opposed(&mut fixture, "2d6", &["beta"]);
    edit_ability(&mut fixture, |ability| {
        ability.allow_self_target = true;
        ability.bonus_stats = Some(vec!["alpha".to_owned()]);
    });
    let ability = fixture.ability;
    for seed in [3, 4, 5] {
        let (mut world, ids) = start(&fixture, seed);
        let mut expected = oracle(seed);
        let actor = draw(&mut expected, "2d6");
        let opposing = draw(&mut expected, "2d6");
        let outcome = act(&mut world, ids[1], ability, ids[1]);
        // One view folds the same modifiers into both rolls, so the margin
        // is the raw difference; the lists themselves are pinned by the
        // crate-private plan test in `combat.rs`.
        assert_eq!(
            outcome.margin,
            i64::from(actor.total) - i64::from(opposing.total),
            "seed {seed}"
        );
        assert_eq!(*world.rng_mut(), expected);
    }
}

/// R9 (B15, B14): the stat stage queries an `Opposed` defence's bonus stats
/// on the target, and a failure names the stat.
#[test]
fn an_opposed_bonus_stat_that_fails_to_evaluate_is_a_stat_evaluation() {
    let mut fixture = Fixture::standard();
    always_success(&mut fixture);
    add_derived(
        &mut fixture,
        "quotient",
        binary("divide", stat("alpha"), stat("gamma")),
    );
    defend_opposed(&mut fixture, "2d6", &["quotient"]);
    attach_to_ability(
        &mut fixture,
        effect_doc(
            141,
            "target",
            "probe-type",
            "stack_all",
            vec![modifier(142, stat_target("gamma"), "set", 0)],
            3,
        ),
    );
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 2);
    let (a, b) = (ids[0], ids[1]);
    act(&mut world, b, ability, a);
    end(&mut world, a);
    let action = CombatAction::UseAbility {
        actor: b,
        ability,
        target: a,
    };
    assert_eq!(
        validate_action(&world, &action),
        Err(CombatError::StatEvaluation {
            stat: "quotient".to_owned()
        })
    );
    let before = state_hash(&world);
    assert!(perform_action(&mut world, &action).is_err());
    assert_eq!(
        state_hash(&world),
        before,
        "a rejected action changes nothing"
    );
}

/// R9 (B21): the `Opposed` and initiative start errors, each with its
/// neighbour in the pinned order, leaving the world untouched.
#[test]
fn the_new_start_errors_fire_in_order_and_change_nothing() {
    fn refused(fixture: &Fixture) -> CombatError {
        let mut world = World::new(5);
        let before = state_hash(&world);
        let error = start_into(fixture, &mut world).expect_err("the start is refused");
        assert_eq!(state_hash(&world), before, "{error}");
        assert!(world.combat().is_none() && world.is_empty());
        error
    }
    let missing = |name: &str| CombatError::MissingStat {
        stat: name.to_owned(),
    };

    // Initiative: dice, then each bonus name.
    let mut fixture = Fixture::standard();
    support::draws::set_initiative_rule(&mut fixture, "bad", &["nowhere"]);
    assert_eq!(refused(&fixture), CombatError::InvalidDice);
    support::draws::set_initiative_rule(&mut fixture, "1d20", &["alpha", "nowhere"]);
    assert_eq!(refused(&fixture), missing("nowhere"));
    support::draws::set_initiative_rule(&mut fixture, "1d20", &["health"]);
    assert_eq!(refused(&fixture), missing("health"), "health is not usable");

    // The initiative checks precede the derived-stat preflight.
    let mut fixture = Fixture::standard();
    add_derived(&mut fixture, "zero", binary("divide", lit(1), lit(0)));
    assert_eq!(
        refused(&fixture),
        CombatError::InvalidStatValue {
            stat: "zero".to_owned()
        }
    );
    support::draws::set_initiative_rule(&mut fixture, "bad", &[]);
    assert_eq!(refused(&fixture), CombatError::InvalidDice);

    // `Opposed`: its dice, then its bonus names.
    let mut fixture = Fixture::standard();
    defend_opposed(&mut fixture, "bad", &["nowhere"]);
    assert_eq!(refused(&fixture), CombatError::InvalidDice);
    defend_opposed(&mut fixture, "2d6", &["nowhere"]);
    assert_eq!(refused(&fixture), missing("nowhere"));

    // The ability's own bonus names precede its damage dice, which precede
    // its effect.
    let mut fixture = Fixture::standard();
    set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": 1, "dice": "bad"}]),
    );
    edit_ability(&mut fixture, |ability| {
        ability.effect = Some(Ulid::from_u128(999));
    });
    assert_eq!(refused(&fixture), CombatError::InvalidDice);
    edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = Some(vec!["nowhere".to_owned()]);
    });
    assert_eq!(refused(&fixture), missing("nowhere"));
    edit_ability(&mut fixture, |ability| ability.bonus_stats = None);
    set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": 1, "dice": "1d4"}]),
    );
    assert_eq!(
        refused(&fixture),
        CombatError::MissingEffect {
            effect: Ulid::from_u128(999)
        }
    );
}

/// R10 (B16): `amount + dice` is exact, drawn right after the resolution,
/// and the report and the health delta agree.
#[test]
fn damage_is_the_flat_amount_plus_the_dice() {
    let mut fixture = Fixture::standard();
    always_success(&mut fixture);
    set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": 2, "dice": "1d4"}]),
    );
    let ability = fixture.ability;
    for seed in [1, 2, 3, 4, 5, 6, 7, 8] {
        let (mut world, ids) = start(&fixture, seed);
        let mut expected = oracle(seed);
        draw(&mut expected, "2d6");
        let rolled = draw(&mut expected, "1d4").total;
        let outcome = act(&mut world, ids[1], ability, ids[0]);
        let damage = u32::try_from(2 + rolled).expect("positive");
        assert_eq!(outcome.damage, damage, "seed {seed}");
        assert_eq!(
            world.combatants().get(ids[0]).expect("A").health(),
            10 - damage,
            "the report and the health delta agree"
        );
        assert_eq!(*world.rng_mut(), expected, "seed {seed}: one damage draw");
    }
}

/// R10 (B16): an entry with no flat amount and dice only, and a negative
/// offset that clamps to zero.
#[test]
fn damage_dice_clamp_at_zero_and_stand_alone() {
    let mut fixture = Fixture::standard();
    always_success(&mut fixture);
    let ability = fixture.ability;

    set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": 2, "dice": "1d4-10"}]),
    );
    let (mut world, ids) = start(&fixture, 3);
    let mut expected = oracle(3);
    draw(&mut expected, "2d6");
    draw(&mut expected, "1d4-10");
    let outcome = act(&mut world, ids[1], ability, ids[0]);
    assert_eq!(outcome.damage, 0, "2 + (1d4 - 10) clamps to zero");
    assert!(!outcome.target_died);
    assert_eq!(world.combatants().get(ids[0]).expect("A").health(), 10);
    assert_eq!(died_events(&mut world), 0);
    assert_eq!(*world.rng_mut(), expected, "the dice were still drawn");

    set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": 0, "dice": "1d4+1"}]),
    );
    let (mut world, ids) = start(&fixture, 3);
    let mut expected = oracle(3);
    draw(&mut expected, "2d6");
    let rolled = draw(&mut expected, "1d4+1").total;
    let outcome = act(&mut world, ids[1], ability, ids[0]);
    assert_eq!(outcome.damage, u32::try_from(rolled).expect("positive"));
}

/// R10 (B16): the sum is taken wide and clamped once into `u32`, and `Died`
/// fires exactly once.
#[test]
fn damage_dice_clamp_at_the_top_and_died_fires_once() {
    let mut fixture = Fixture::standard();
    always_success(&mut fixture);
    let ability = fixture.ability;

    set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": u32::MAX, "dice": "1d4"}]),
    );
    let (mut world, ids) = start(&fixture, 3);
    let outcome = act(&mut world, ids[1], ability, ids[0]);
    assert_eq!(outcome.damage, u32::MAX);
    assert!(outcome.target_died);
    let target = world.combatants().get(ids[0]).expect("A");
    assert_eq!((target.health(), target.dead()), (0, true));
    assert_eq!(died_events(&mut world), 1, "exactly one Died");

    // A negative offset on a huge amount is exact, not clamped.
    set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": u32::MAX, "dice": "1d4-10"}]),
    );
    let (mut world, ids) = start(&fixture, 3);
    let mut expected = oracle(3);
    draw(&mut expected, "2d6");
    let rolled = draw(&mut expected, "1d4-10").total;
    let outcome = act(&mut world, ids[1], ability, ids[0]);
    let exact = i64::from(u32::MAX) + i64::from(rolled);
    assert_eq!(i64::from(outcome.damage), exact);
}

/// R10 (B16): an outcome without damage dice draws nothing after the
/// resolution, even when another outcome authors dice.
#[test]
fn an_outcome_without_dice_leaves_the_rng_untouched() {
    let mut fixture = Fixture::standard();
    always_failure(&mut fixture);
    set_damage(
        &mut fixture,
        json!([
            {"outcome": {"type": "success"}, "amount": 2, "dice": "1d4"},
            {"outcome": {"type": "failure"}, "amount": 1}
        ]),
    );
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 4);
    let mut expected = oracle(4);
    draw(&mut expected, "2d6");
    let outcome = act(&mut world, ids[1], ability, ids[0]);
    assert_eq!(outcome.damage, 1, "the flat failure entry");
    assert_eq!(*world.rng_mut(), expected, "no damage draw");

    // An outcome with no entry at all deals nothing and draws nothing.
    set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": 2, "dice": "1d4"}]),
    );
    let (mut world, ids) = start(&fixture, 4);
    let outcome = act(&mut world, ids[1], ability, ids[0]);
    assert_eq!(outcome.damage, 0);
    assert_eq!(*world.rng_mut(), expected);
}

/// The two rolled keys of `seed` for the standard fixture: A rolls first.
fn rolled_keys(seed: u64, dice: &str, bonus: (i32, i32), authored: (i32, i32)) -> (i32, i32) {
    let mut expected = oracle(seed);
    let a = draw(&mut expected, dice).total;
    let b = draw(&mut expected, dice).total;
    (
        a.saturating_add(bonus.0).saturating_add(authored.0),
        b.saturating_add(bonus.1).saturating_add(authored.1),
    )
}

/// R11 (B17): each key is the dice total plus the bonus stat plus the
/// authored key, rolled in authored participant order after every check.
#[test]
fn rolled_keys_equal_the_precomputed_sums() {
    let mut fixture = Fixture::standard();
    set_initiative_rule(&mut fixture, "1d20", &["alpha"]);
    for seed in [1, 2, 3, 4, 5, 6] {
        let (mut world, ids) = start(&fixture, seed);
        let (key_a, key_b) = rolled_keys(seed, "1d20", (8, 6), (10, 9));
        assert_eq!(
            world.combatants().get(ids[0]).expect("A").initiative(),
            key_a
        );
        assert_eq!(
            world.combatants().get(ids[1]).expect("B").initiative(),
            key_b
        );
        let mut expected = vec![(key_a, ids[0]), (key_b, ids[1])];
        expected.sort();
        assert_eq!(timeline_of(&world), expected, "seed {seed}");
        let mut after = oracle(seed);
        draw(&mut after, "1d20");
        draw(&mut after, "1d20");
        assert_eq!(*world.rng_mut(), after, "one draw per participant");
    }
}

/// R11 (B17): equal keys break on the entity id, ascending.
#[test]
fn equal_rolled_keys_break_ties_on_the_entity_id() {
    let mut fixture = Fixture::standard();
    set_initiative_rule(&mut fixture, "1d1", &["alpha"]);
    fixture.set_alpha(5, 5);
    set_authored_initiative(&mut fixture, 3, 3);
    let (world, ids) = start(&fixture, 2);
    assert_eq!(timeline_of(&world), vec![(9, ids[0]), (9, ids[1])]);
    assert_eq!(world.combat().expect("encounter").active, Some(ids[0]));
}

/// R11 (B17): a TB rollover reuses the stored keys and draws nothing.
#[test]
fn a_tb_rollover_reuses_the_rolled_keys() {
    let mut fixture = Fixture::standard();
    set_initiative_rule(&mut fixture, "1d20", &["alpha"]);
    let (mut world, ids) = start(&fixture, 6);
    let keys = timeline_of(&world);
    let ability = fixture.ability;
    let (first, second) = (keys[0].1, keys[1].1);
    act(&mut world, first, ability, second);
    let before = world.rng_mut().clone();
    end(&mut world, second);
    assert_eq!(world.combat().expect("encounter").round, 1, "rolled over");
    assert_eq!(timeline_of(&world), keys, "the same keys");
    assert_eq!(*world.rng_mut(), before, "the rollover draws nothing");
    assert!(ids.contains(&first));
}

/// R11 (B17): an RT start keys every combatant `0` and stores the rolls,
/// and the RT-to-TB switch rebuilds the timeline at them.
#[test]
fn an_rt_start_keys_zero_and_keeps_the_rolls_for_the_switch() {
    let mut fixture = capable(4);
    set_initiative_rule(&mut fixture, "1d20", &["alpha"]);
    let seed = 5;
    let (mut world, ids) = fixture.start_rt(seed, EncounterMode::RealTime);
    let (key_a, key_b) = rolled_keys(seed, "1d20", (8, 6), (10, 9));
    assert_eq!(timeline_of(&world), vec![(0, ids[0]), (0, ids[1])]);
    assert_eq!(
        world.combatants().get(ids[0]).expect("A").initiative(),
        key_a
    );
    assert_eq!(
        world.combatants().get(ids[1]).expect("B").initiative(),
        key_b
    );
    let mut after = oracle(seed);
    draw(&mut after, "1d20");
    draw(&mut after, "1d20");
    assert_eq!(*world.rng_mut(), after, "the draws happen in RT too");

    request_mode(&mut world, EncounterMode::TurnBased).expect("the request");
    ticks(&mut world, 4);
    assert_eq!(
        world.combat().expect("encounter").mode,
        EncounterMode::TurnBased
    );
    let mut expected = vec![(key_a, ids[0]), (key_b, ids[1])];
    expected.sort();
    assert_eq!(timeline_of(&world), expected, "rebuilt at the rolled keys");
}

/// R11 (B17): without a rule there is no draw and the authored key stands.
#[test]
fn no_initiative_rule_means_no_draw() {
    let fixture = Fixture::standard();
    let (mut world, ids) = start(&fixture, 7);
    assert!(!world.rng_mut().has_stream(COMBAT_ROLL_STREAM));
    assert_eq!(*world.rng_mut(), oracle(7));
    assert_eq!(timeline_of(&world), vec![(9, ids[1]), (10, ids[0])]);
}

/// R11 (B17): the sum saturates at `i32::MIN` and `i32::MAX`.
#[test]
fn the_rolled_key_saturates_at_both_ends() {
    let mut fixture = Fixture::standard();
    set_initiative_rule(&mut fixture, "1d6", &[]);
    set_authored_initiative(&mut fixture, i32::MAX, i32::MIN);
    let (world, ids) = start(&fixture, 3);
    assert_eq!(
        world.combatants().get(ids[0]).expect("A").initiative(),
        i32::MAX
    );
    let mut low = fixture;
    set_initiative_rule(&mut low, "1d4-5", &[]);
    let (world, ids) = start(&low, 3);
    assert_eq!(
        world.combatants().get(ids[1]).expect("B").initiative(),
        i32::MIN,
        "a negative roll cannot wrap"
    );
    assert_eq!(
        timeline_of(&world).first().map(|entry| entry.0),
        Some(i32::MIN)
    );
}

/// R11 (B17, B21): a rejected start with a valid initiative rule draws
/// nothing, leaving bytes, RNG and hash untouched.
#[test]
fn a_rejected_start_with_a_valid_initiative_rule_draws_nothing() {
    // Invalid damage dice.
    {
        let mut fixture = Fixture::standard();
        set_initiative_rule(&mut fixture, "1d20", &["alpha"]);
        set_damage(
            &mut fixture,
            json!([{"outcome": {"type": "success"}, "amount": 1, "dice": "bad"}]),
        );
        let mut world = World::new(7);
        let before_bytes = bytes_of(&world);
        let before_rng = world.rng_mut().clone();
        let before_hash = state_hash(&world);
        assert_eq!(
            start_into(&fixture, &mut world),
            Err(CombatError::InvalidDice)
        );
        assert_eq!(bytes_of(&world), before_bytes);
        assert_eq!(*world.rng_mut(), before_rng);
        assert_eq!(state_hash(&world), before_hash);
    }
    // MissingEffect.
    {
        let mut fixture = Fixture::standard();
        set_initiative_rule(&mut fixture, "1d20", &["alpha"]);
        edit_ability(&mut fixture, |ability| {
            ability.effect = Some(Ulid::from_u128(999));
        });
        let mut world = World::new(7);
        let before_bytes = bytes_of(&world);
        let before_rng = world.rng_mut().clone();
        let before_hash = state_hash(&world);
        assert_eq!(
            start_into(&fixture, &mut world),
            Err(CombatError::MissingEffect {
                effect: Ulid::from_u128(999)
            })
        );
        assert_eq!(bytes_of(&world), before_bytes);
        assert_eq!(*world.rng_mut(), before_rng);
        assert_eq!(state_hash(&world), before_hash);
    }
    // DuplicateModifier.
    {
        let mut fixture = Fixture::standard();
        set_initiative_rule(&mut fixture, "1d20", &["alpha"]);
        let mut effect = effect_doc(
            141,
            "target",
            "probe-type",
            "stack_all",
            vec![modifier(142, json!({"type": "roll"}), "add", 1)],
            2,
        );
        effect.modifiers.push(effect.modifiers[0].clone());
        attach_to_ability(&mut fixture, effect);
        let mut world = World::new(7);
        let before_bytes = bytes_of(&world);
        let before_rng = world.rng_mut().clone();
        let before_hash = state_hash(&world);
        assert!(matches!(
            start_into(&fixture, &mut world),
            Err(CombatError::DuplicateModifier { .. })
        ));
        assert_eq!(bytes_of(&world), before_bytes);
        assert_eq!(*world.rng_mut(), before_rng);
        assert_eq!(state_hash(&world), before_hash);
    }
    // TooManyTags.
    {
        let mut fixture = Fixture::standard();
        set_initiative_rule(&mut fixture, "1d20", &["alpha"]);
        let a = fixture.creature_a;
        let full: Vec<String> = (0..1024).map(|i| format!("tag-{i:04}")).collect();
        // reuse surface_tags helper? define local
        let tags: Vec<&str> = full.iter().map(String::as_str).collect();
        set_tags(&mut fixture, a, &tags);
        attach_to_ability(
            &mut fixture,
            grant(
                effect_doc(
                    141,
                    "target",
                    "probe-type",
                    "stack_all",
                    vec![modifier(142, json!({"type": "roll"}), "add", 1)],
                    2,
                ),
                &["fresh"],
            ),
        );
        let mut world = World::new(7);
        let before_bytes = bytes_of(&world);
        let before_rng = world.rng_mut().clone();
        let before_hash = state_hash(&world);
        assert_eq!(
            start_into(&fixture, &mut world),
            Err(CombatError::TooManyTags)
        );
        assert_eq!(bytes_of(&world), before_bytes);
        assert_eq!(*world.rng_mut(), before_rng);
        assert_eq!(state_hash(&world), before_hash);
    }
}

/// R11 (B17): the bonus sum saturates at both ends.
#[test]
fn the_rolled_bonus_sum_saturates_at_both_ends() {
    // MAX case: two bonus stats of i32::MAX.
    {
        let mut fixture = Fixture::standard();
        add_derived(&mut fixture, "huge1", lit(i32::MAX));
        add_derived(&mut fixture, "huge2", lit(i32::MAX));
        set_initiative_rule(&mut fixture, "1d1", &["huge1", "huge2"]);
        set_authored_initiative(&mut fixture, 0, 0);
        let (world, ids) = start(&fixture, 3);
        for id in ids {
            assert_eq!(
                world.combatants().get(id).expect("c").initiative(),
                i32::MAX,
                "MAX saturation"
            );
        }
    }
    // MIN case.
    {
        let mut fixture = Fixture::standard();
        add_derived(&mut fixture, "tiny1", lit(i32::MIN));
        add_derived(&mut fixture, "tiny2", lit(i32::MIN));
        set_initiative_rule(&mut fixture, "1d4-5", &["tiny1", "tiny2"]);
        set_authored_initiative(&mut fixture, 0, 0);
        let (world, ids) = start(&fixture, 3);
        for id in ids {
            assert_eq!(
                world.combatants().get(id).expect("c").initiative(),
                i32::MIN,
                "MIN saturation"
            );
        }
    }
}

/// R9 (B14): opposed actor and opposing bonuses land on their own sides.
#[test]
fn opposed_actor_and_opposing_bonuses_land_on_their_own_sides() {
    let mut fixture = Fixture::standard();
    always_success(&mut fixture);
    add_derived(&mut fixture, "actorbonus", lit(10));
    add_derived(&mut fixture, "oppbonus", lit(3));
    defend_opposed(&mut fixture, "1d8", &["oppbonus"]);
    edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = Some(vec!["actorbonus".to_owned()]);
    });
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 5);
    let (a, b) = (ids[0], ids[1]);
    let mut expected = oracle(5);
    let actor = draw(&mut expected, "2d6");
    let opposing = draw(&mut expected, "1d8");
    let outcome = act(&mut world, b, ability, a);
    assert_eq!(
        outcome.margin,
        i64::from(actor.total + 10) - i64::from(opposing.total + 3),
        "actor bonus 10 on actor side, opposing bonus 3 on opposing side"
    );
    assert_eq!(*world.rng_mut(), expected);
}
