//! T137b two-keys contract tests: `Opposed` draws, damage dice, rolled
//! initiative, tags and the load/replay rules, checked through the public
//! `crpg-sim` API against `tasks/T137.md`.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Ulid};
use crpg_sim::{
    perform_action, request_mode, state_hash, CombatAction, CombatError, EncounterMode, World,
    COMBAT_ROLL_STREAM,
};
use serde_json::json;
use support::draws;
use support::rt;
use support::surface;
use support::Fixture;

fn start(fixture: &Fixture, seed: u64) -> (World, Vec<EntityId>) {
    surface::start(fixture, seed)
}

fn save(world: &World) -> serde_json::Value {
    serde_json::to_value(world).expect("world serializes")
}

fn load(value: serde_json::Value) -> Result<World, serde_json::Error> {
    serde_json::from_value(value)
}

fn reload(world: World) -> World {
    let value = save(&world);
    serde_json::from_value(value).expect("reload")
}

fn rolled_keys(
    seed: u64,
    dice: &str,
    alpha_a: i32,
    alpha_b: i32,
    init_a: i32,
    init_b: i32,
) -> (i32, i32) {
    let mut rng = draws::oracle(seed);
    let a_roll = draws::draw(&mut rng, dice).total;
    let b_roll = draws::draw(&mut rng, dice).total;
    (
        a_roll.saturating_add(alpha_a).saturating_add(init_a),
        b_roll.saturating_add(alpha_b).saturating_add(init_b),
    )
}

// ---------------------------------------------------------------------------
// B13: Opposed draw order
// ---------------------------------------------------------------------------

#[test]
fn opposed_draws_actor_then_target_on_one_stream() {
    let mut fixture = Fixture::standard();
    draws::defend_opposed(&mut fixture, "1d6", &[]);
    draws::set_table(
        &mut fixture,
        json!([{"min_margin": i64::MIN, "outcome": {"type": "failure"}}]),
        json!([]),
    );
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 7);
    let (a, b) = (ids[0], ids[1]);
    let actor = world.combat().expect("encounter").active.expect("active");
    let target = if actor == a { b } else { a };

    let mut rng = world.rng_mut().clone();
    let actor_roll = draws::draw(&mut rng, "2d6");
    let opposing_roll = draws::draw(&mut rng, "1d6");

    let outcome = surface::act(&mut world, actor, ability, target);
    assert_eq!(
        outcome.margin,
        (actor_roll.total - opposing_roll.total) as i64,
        "the margin is the actor total minus the opposing total"
    );
    assert_eq!(*world.rng_mut(), rng, "the two draws happen in that order");
}

// ---------------------------------------------------------------------------
// B13/B14: target Roll effect folds; target Dc effect does not
// ---------------------------------------------------------------------------

fn opposed_fixture(mod_target: Option<&str>, value: i32) -> Fixture {
    let mut fixture = Fixture::standard();
    draws::defend_opposed(&mut fixture, "1d6", &[]);
    surface::always_success(&mut fixture);
    if let Some(target_name) = mod_target {
        let target_json = match target_name {
            "roll" => json!({"type": "roll"}),
            "dc" => json!({"type": "dc"}),
            _ => panic!("unknown target"),
        };
        let effect = surface::effect_doc(
            141,
            "target",
            "probe-type",
            "stack_all",
            vec![surface::modifier(142, target_json, "add", value)],
            3,
        );
        surface::attach_to_ability(&mut fixture, effect);
    }
    fixture.ruleset.pools[0].max = 10;
    fixture
}

#[test]
fn opposed_target_roll_effect_folds_into_opposing_roll() {
    let base = opposed_fixture(None, 0);
    let with = opposed_fixture(Some("roll"), 5);
    let seed = 11;
    let (mut w0, ids0) = start(&base, seed);
    let (mut w1, ids1) = start(&with, seed);
    let (b0, a0) = (ids0[1], ids0[0]);
    let (b1, a1) = (ids1[1], ids1[0]);
    surface::act(&mut w0, b0, base.ability, a0);
    surface::act(&mut w1, b1, with.ability, a1);
    surface::end(&mut w0, a0);
    surface::end(&mut w1, a1);
    let m0 = surface::act(&mut w0, b0, base.ability, a0).margin;
    let m1 = surface::act(&mut w1, b1, with.ability, a1).margin;
    assert_eq!(
        m1,
        m0 - 5,
        "the target's Roll effect shifts the opposing roll"
    );
}

#[test]
fn opposed_target_dc_effect_does_not_fold() {
    let base = opposed_fixture(None, 0);
    let with = opposed_fixture(Some("dc"), 5);
    let seed = 11;
    let (mut w0, ids0) = start(&base, seed);
    let (mut w1, ids1) = start(&with, seed);
    let (b0, a0) = (ids0[1], ids0[0]);
    let (b1, a1) = (ids1[1], ids1[0]);
    surface::act(&mut w0, b0, base.ability, a0);
    surface::act(&mut w1, b1, with.ability, a1);
    surface::end(&mut w0, a0);
    surface::end(&mut w1, a1);
    let m0 = surface::act(&mut w0, b0, base.ability, a0).margin;
    let m1 = surface::act(&mut w1, b1, with.ability, a1).margin;
    assert_eq!(m1, m0, "the target's Dc effect does not fold into Opposed");
}

#[test]
fn natural_rule_does_not_fire_on_opposing_roll() {
    for seed in 0u64..5000 {
        let mut fixture = Fixture::standard();
        draws::defend_opposed(&mut fixture, "1d20", &[]);
        surface::edit_ability(&mut fixture, |ability| {
            ability.dice = "1d1".to_owned();
        });
        draws::set_table(
            &mut fixture,
            json!([{"min_margin": i64::MIN, "outcome": {"type": "failure"}}]),
            json!([{"face": 20, "effect": {"type": "override", "value": {"type": "success"}}}]),
        );
        surface::edit_ability(&mut fixture, |ability| {
            ability.allow_self_target = true;
            ability.ends_turn = false;
            ability.cost = 1;
            ability.extra_costs = vec![];
        });
        fixture.ruleset.pools[0].max = 10;
        let (mut world, _ids) = start(&fixture, seed);
        let actor = world.combat().expect("encounter").active.expect("active");
        let outcome = surface::act(&mut world, actor, fixture.ability, actor);
        if outcome.margin == 1 - 20 {
            assert_eq!(
                outcome.outcome,
                crpg_rules::Outcome::Failure,
                "the opposing natural 20 does not override the outcome"
            );
            return;
        }
    }
    panic!("no seed produced an opposing roll of 20");
}

// ---------------------------------------------------------------------------
// B21: start-order precedence (one double fault per adjacent rung pair)
// ---------------------------------------------------------------------------

/// Starts the fixture into a fresh world, returning the error and asserting
/// that the rejected start left the world exactly as it was.
fn rejected(fixture: &Fixture) -> CombatError {
    let mut world = World::new(3);
    let before = world.clone();
    let err = draws::start_into(fixture, &mut world).expect_err("the start is rejected");
    assert_eq!(world, before, "a rejected start leaves the world unchanged");
    err
}

fn missing(stat: &str) -> CombatError {
    CombatError::MissingStat {
        stat: stat.to_owned(),
    }
}

#[test]
fn b21_initiative_dice_before_its_bonus_names() {
    let rule = |dice: &str, bonus: &[&str]| {
        let mut fixture = Fixture::standard();
        draws::set_initiative_rule(&mut fixture, dice, bonus);
        fixture
    };
    assert_eq!(rejected(&rule("1d20", &["nope"])), missing("nope"));
    assert_eq!(
        rejected(&rule("not-dice", &["nope"])),
        CombatError::InvalidDice
    );
}

#[test]
fn b21_opposed_dice_before_its_bonus_names() {
    let opposed = |dice: &str, bonus: &[&str]| {
        let mut fixture = Fixture::standard();
        draws::defend_opposed(&mut fixture, dice, bonus);
        fixture
    };
    assert_eq!(rejected(&opposed("1d6", &["nope"])), missing("nope"));
    assert_eq!(
        rejected(&opposed("not-dice", &["nope"])),
        CombatError::InvalidDice
    );
}

#[test]
fn b21_defense_bonus_names_before_ability_bonus_names() {
    let both = |opposed: &str, own: &str| {
        let mut fixture = Fixture::standard();
        draws::defend_opposed(&mut fixture, "1d6", &[opposed]);
        surface::edit_ability(&mut fixture, |ability| {
            ability.bonus_stats = Some(vec![own.to_owned()]);
        });
        fixture
    };
    assert_eq!(
        rejected(&both("nope-opposed", "nope-own")),
        missing("nope-opposed")
    );
    assert_eq!(rejected(&both("beta", "nope-own")), missing("nope-own"));
}

#[test]
fn b21_ability_bonus_names_before_damage_dice() {
    let fixture = |bonus: Option<&str>, dice: &str| {
        let mut fixture = Fixture::standard();
        draws::set_damage(
            &mut fixture,
            json!([{"outcome": {"type": "success"}, "amount": 2, "dice": dice}]),
        );
        surface::edit_ability(&mut fixture, |ability| {
            ability.bonus_stats = bonus.map(|name| vec![name.to_owned()]);
        });
        fixture
    };
    assert_eq!(
        rejected(&fixture(Some("nope"), "not-dice")),
        missing("nope")
    );
    assert_eq!(
        rejected(&fixture(Some("beta"), "not-dice")),
        CombatError::InvalidDice
    );
    assert_eq!(
        rejected(&fixture(None, "not-dice")),
        CombatError::InvalidDice
    );
}

const E1: u128 = 141;
const E2: u128 = 151;

/// A second listed ability (id 203) so that two effects are referenced:
/// effect `E1` (modifier 142) on the standard ability, `E2` (modifier 152) on
/// the second. Both are valid, `stack_all` and share the mod type.
fn two_effects() -> Fixture {
    let mut fixture = Fixture::standard();
    let first = surface::effect_doc(
        E1,
        "target",
        "probe-type",
        "stack_all",
        vec![surface::modifier(142, json!({"type": "roll"}), "add", 1)],
        3,
    );
    let second = surface::effect_doc(
        E2,
        "target",
        "probe-type",
        "stack_all",
        vec![surface::modifier(152, json!({"type": "roll"}), "add", 1)],
        3,
    );
    surface::attach_to_ability(&mut fixture, first);
    let mut other = fixture
        .abilities
        .get(&fixture.ability)
        .expect("the ability")
        .clone();
    other.id = Ulid::from_u128(203);
    other.slug = "probe-attack-2".to_owned();
    other.name = "probe.attack-2".to_owned();
    other.effect = Some(Ulid::from_u128(E2));
    fixture.effects.insert(Ulid::from_u128(E2), second);
    fixture.ruleset.abilities.push(other.id);
    fixture.abilities.insert(other.id, other);
    fixture
}

fn effect_mut(fixture: &mut Fixture, id: u128) -> &mut crpg_data::Effect {
    fixture
        .effects
        .get_mut(&Ulid::from_u128(id))
        .expect("the effect")
}

fn bad_damage_dice(fixture: &mut Fixture) {
    draws::set_damage(
        fixture,
        json!([{"outcome": {"type": "success"}, "amount": 2, "dice": "not-dice"}]),
    );
}

/// Makes `E1` fail `InvalidEffect`.
fn fault_invalid_effect(fixture: &mut Fixture) -> CombatError {
    effect_mut(fixture, E1).duration_rounds = 0;
    CombatError::InvalidEffect {
        effect: Ulid::from_u128(E1),
    }
}

/// Makes the two effects share modifier id 142.
fn fault_duplicate_modifier(fixture: &mut Fixture) -> CombatError {
    effect_mut(fixture, E2).modifiers[0].id = Ulid::from_u128(142);
    CombatError::DuplicateModifier {
        id: Ulid::from_u128(142),
    }
}

/// Gives `E2` a different policy for the shared mod type.
fn fault_policy_conflict(fixture: &mut Fixture) -> CombatError {
    effect_mut(fixture, E2).policy = surface::effect_doc(
        E2,
        "target",
        "probe-type",
        "highest_bonus_worst_penalty",
        vec![surface::modifier(152, json!({"type": "roll"}), "add", 1)],
        3,
    )
    .policy;
    CombatError::PolicyConflict {
        mod_type: "probe-type".to_owned(),
    }
}

type Fault = fn(&mut Fixture) -> CombatError;
const FAULTS: [Fault; 3] = [
    fault_invalid_effect,
    fault_duplicate_modifier,
    fault_policy_conflict,
];

#[test]
fn b21_damage_dice_before_every_effect_check() {
    let mut control = two_effects();
    bad_damage_dice(&mut control);
    assert_eq!(
        rejected(&control),
        CombatError::InvalidDice,
        "control: damage dice alone"
    );
    for fault in FAULTS {
        let mut effect_only = two_effects();
        let expected = fault(&mut effect_only);
        assert_eq!(rejected(&effect_only), expected, "control: effect fault");

        let mut both = two_effects();
        fault(&mut both);
        bad_damage_dice(&mut both);
        assert_eq!(
            rejected(&both),
            CombatError::InvalidDice,
            "{expected:?} loses to the damage dice"
        );
    }
}

#[test]
fn b21_effect_checks_in_contract_order() {
    // InvalidEffect (8.1) before DuplicateModifier (9) before PolicyConflict (9).
    let mut fixture = two_effects();
    let invalid = fault_invalid_effect(&mut fixture);
    fault_duplicate_modifier(&mut fixture);
    fault_policy_conflict(&mut fixture);
    assert_eq!(rejected(&fixture), invalid);

    let mut fixture = two_effects();
    let duplicate = fault_duplicate_modifier(&mut fixture);
    fault_policy_conflict(&mut fixture);
    assert_eq!(rejected(&fixture), duplicate);

    let mut fixture = two_effects();
    let conflict = fault_policy_conflict(&mut fixture);
    assert_eq!(rejected(&fixture), conflict);
}

#[test]
fn b21_effect_checks_before_the_tag_bound() {
    let over_bound = |fixture: &mut Fixture| {
        fixture
            .creatures
            .get_mut(&fixture.creature_a)
            .expect("c")
            .tags = (0..1024).map(|i| format!("tag-{i:04}")).collect();
        let first = effect_mut(fixture, E1).clone();
        *effect_mut(fixture, E1) = draws::grant(first, &["hasted"]);
    };
    let mut control = two_effects();
    over_bound(&mut control);
    assert_eq!(rejected(&control), CombatError::TooManyTags, "control");

    for fault in FAULTS {
        let mut fixture = two_effects();
        over_bound(&mut fixture);
        let expected = fault(&mut fixture);
        assert_eq!(rejected(&fixture), expected, "{expected:?} before B20");
    }
}
// ---------------------------------------------------------------------------
// B14: Opposed bonus stats
// ---------------------------------------------------------------------------

/// Stat values of the standard fixture: A is (alpha 8, beta 6), B is
/// (alpha 6, beta 7).
fn stat_of(ids: &[EntityId], who: EntityId, stat: &str) -> i64 {
    match (who == ids[0], stat) {
        (true, "alpha") => 8,
        (true, "beta") => 6,
        (false, "alpha") => 6,
        (false, "beta") => 7,
        _ => panic!("unknown stat"),
    }
}

fn bonus_fixture(actor_bonus: &[&str], opposed_bonus: &[&str], self_target: bool) -> Fixture {
    let mut fixture = Fixture::standard();
    draws::defend_opposed(&mut fixture, "1d6", opposed_bonus);
    surface::always_success(&mut fixture);
    let names: Vec<String> = actor_bonus.iter().map(|n| (*n).to_owned()).collect();
    surface::edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = if names.is_empty() { None } else { Some(names) };
        if self_target {
            ability.allow_self_target = true;
            ability.ends_turn = false;
            ability.cost = 1;
            ability.extra_costs = vec![];
        }
    });
    fixture.ruleset.pools[0].max = 10;
    fixture
}

/// The raw dice difference (actor dice minus opposing dice) and the margin
/// of one action from the first active combatant.
fn bonus_margin(fixture: &Fixture, seed: u64, self_target: bool) -> (i64, i64, EntityId, EntityId) {
    let (mut world, ids) = start(fixture, seed);
    let actor = world.combat().expect("encounter").active.expect("active");
    let target = if self_target {
        actor
    } else if actor == ids[0] {
        ids[1]
    } else {
        ids[0]
    };
    let mut rng = world.rng_mut().clone();
    let actor_roll = draws::draw(&mut rng, "2d6").total;
    let opposing_roll = draws::draw(&mut rng, "1d6").total;
    let margin = surface::act(&mut world, actor, fixture.ability, target).margin;
    (
        i64::from(actor_roll) - i64::from(opposing_roll),
        margin,
        actor,
        target,
    )
}

#[test]
fn opposed_bonus_stats_fold_into_their_own_side() {
    let fixture = bonus_fixture(&["alpha"], &["beta"], false);
    let ids = start(&fixture, 13).1;
    let (diff, margin, actor, target) = bonus_margin(&fixture, 13, false);
    assert_eq!(
        margin,
        diff + stat_of(&ids, actor, "alpha") - stat_of(&ids, target, "beta"),
        "the actor's list lifts its roll; the Opposed list lifts the target's"
    );
    // Controls: each list alone, and neither.
    let (diff, margin, actor, target) =
        bonus_margin(&bonus_fixture(&["alpha"], &[], false), 13, false);
    assert_eq!(margin, diff + stat_of(&ids, actor, "alpha"));
    let _ = target;
    let (diff, margin, _, target) = bonus_margin(&bonus_fixture(&[], &["beta"], false), 13, false);
    assert_eq!(margin, diff - stat_of(&ids, target, "beta"));
    let (diff, margin, _, _) = bonus_margin(&bonus_fixture(&[], &[], false), 13, false);
    assert_eq!(margin, diff);
}

#[test]
fn self_targeted_opposed_folds_both_lists_into_both_rolls() {
    // Both lists share one view, so both rolls fold both: the margin is the
    // raw dice difference whatever the lists hold.
    for (actor_list, opposed_list) in [
        (vec!["alpha"], vec!["beta"]),
        (vec!["alpha", "beta"], vec![]),
        (vec![], vec!["alpha", "beta"]),
        (vec!["alpha", "beta"], vec!["alpha", "beta"]),
    ] {
        let fixture = bonus_fixture(&actor_list, &opposed_list, true);
        let (diff, margin, actor, target) = bonus_margin(&fixture, 13, true);
        assert_eq!(actor, target);
        assert_eq!(
            margin, diff,
            "self-targeted Opposed with {actor_list:?} / {opposed_list:?}"
        );
    }
    // Control: against another target the same lists do not cancel.
    let fixture = bonus_fixture(&["alpha", "beta"], &[], false);
    let ids = start(&fixture, 13).1;
    let (diff, margin, actor, _) = bonus_margin(&fixture, 13, false);
    assert_eq!(
        margin,
        diff + stat_of(&ids, actor, "alpha") + stat_of(&ids, actor, "beta")
    );
}

// ---------------------------------------------------------------------------
// B16: damage dice
// ---------------------------------------------------------------------------

#[test]
fn damage_dice_exact_sum() {
    let mut fixture = Fixture::standard();
    draws::set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": 2, "dice": "1d4"}]),
    );
    surface::always_success(&mut fixture);
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 7);
    let actor = world.combat().expect("encounter").active.expect("active");
    let target = if actor == ids[0] { ids[1] } else { ids[0] };

    let mut rng = world.rng_mut().clone();
    draws::draw(&mut rng, "2d6");
    let dice_roll = draws::draw(&mut rng, "1d4");
    let expected = 2i64 + dice_roll.total as i64;

    let outcome = surface::act(&mut world, actor, ability, target);
    assert_eq!(outcome.damage, expected as u32);
    assert_eq!(*world.rng_mut(), rng);
}

#[test]
fn damage_dice_clamps_at_zero_and_max() {
    // clamp to 0
    let mut fixture = Fixture::standard();
    draws::set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": 0, "dice": "1d4-10"}]),
    );
    surface::always_success(&mut fixture);
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 7);
    let actor = world.combat().expect("encounter").active.expect("active");
    let target = if actor == ids[0] { ids[1] } else { ids[0] };
    let outcome = surface::act(&mut world, actor, ability, target);
    assert_eq!(outcome.damage, 0);

    // clamp to u32::MAX
    let mut fixture = Fixture::standard();
    draws::set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": 4294967295u64, "dice": "1d4"}]),
    );
    surface::always_success(&mut fixture);
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 7);
    let actor = world.combat().expect("encounter").active.expect("active");
    let target = if actor == ids[0] { ids[1] } else { ids[0] };
    let outcome = surface::act(&mut world, actor, ability, target);
    assert_eq!(outcome.damage, u32::MAX);
}

#[test]
fn damage_kills_exactly_once() {
    let mut fixture = Fixture::standard();
    draws::set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": 100, "dice": "1d4"}]),
    );
    surface::always_success(&mut fixture);
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 7);
    let actor = world.combat().expect("encounter").active.expect("active");
    let target = if actor == ids[0] { ids[1] } else { ids[0] };
    surface::act(&mut world, actor, ability, target);
    assert_eq!(draws::died_events(&mut world), 1);
}

#[test]
fn damage_without_dice_draws_nothing() {
    let mut fixture = Fixture::standard();
    draws::set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": 2}]),
    );
    surface::always_success(&mut fixture);
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 7);
    let actor = world.combat().expect("encounter").active.expect("active");
    let target = if actor == ids[0] { ids[1] } else { ids[0] };

    let mut rng = world.rng_mut().clone();
    draws::draw(&mut rng, "2d6");
    let outcome = surface::act(&mut world, actor, ability, target);
    assert_eq!(outcome.damage, 2);
    assert_eq!(*world.rng_mut(), rng);
}

#[test]
fn damage_report_matches_health_delta() {
    let mut fixture = Fixture::standard();
    draws::set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": 3, "dice": "1d4"}]),
    );
    surface::always_success(&mut fixture);
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 7);
    let actor = world.combat().expect("encounter").active.expect("active");
    let target = if actor == ids[0] { ids[1] } else { ids[0] };
    let before = world.combatants().get(target).expect("target").health();
    let outcome = surface::act(&mut world, actor, ability, target);
    let after = world.combatants().get(target).expect("target").health();
    assert_eq!(before - after, outcome.damage);
}

#[test]
fn nonlethal_damaging_hit_emits_no_died() {
    for dice in [None, Some("1d4")] {
        let mut fixture = Fixture::standard();
        let entry = match dice {
            None => json!([{"outcome": {"type": "success"}, "amount": 1}]),
            Some(d) => json!([{"outcome": {"type": "success"}, "amount": 1, "dice": d}]),
        };
        draws::set_damage(&mut fixture, entry);
        surface::always_success(&mut fixture);
        let ability = fixture.ability;
        let (mut world, ids) = start(&fixture, 7);
        let actor = world.combat().expect("encounter").active.expect("active");
        let target = if actor == ids[0] { ids[1] } else { ids[0] };
        let outcome = surface::act(&mut world, actor, ability, target);
        assert!(outcome.damage >= 1, "the hit does damage");
        assert!(!outcome.target_died, "and does not kill");
        assert_eq!(draws::died_events(&mut world), 0);
    }
}

#[test]
fn damage_dice_for_an_undecided_outcome_draw_nothing() {
    let entries = json!([
        {"outcome": {"type": "success"}, "amount": 2},
        {"outcome": {"type": "failure"}, "amount": 1, "dice": "1d4"}
    ]);
    // Dice authored only for failure; success is decided: flat, no draw.
    let mut fixture = Fixture::standard();
    draws::set_damage(&mut fixture, entries.clone());
    surface::always_success(&mut fixture);
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 7);
    let actor = world.combat().expect("encounter").active.expect("active");
    let target = if actor == ids[0] { ids[1] } else { ids[0] };
    let mut rng = world.rng_mut().clone();
    draws::draw(&mut rng, "2d6");
    let outcome = surface::act(&mut world, actor, ability, target);
    assert_eq!(outcome.damage, 2);
    assert_eq!(*world.rng_mut(), rng, "no damage draw for the success");

    // Control: the same dice do draw when failure is decided.
    let mut fixture = Fixture::standard();
    draws::set_damage(&mut fixture, entries);
    draws::always_failure(&mut fixture);
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 7);
    let actor = world.combat().expect("encounter").active.expect("active");
    let target = if actor == ids[0] { ids[1] } else { ids[0] };
    let mut rng = world.rng_mut().clone();
    draws::draw(&mut rng, "2d6");
    let dice_roll = draws::draw(&mut rng, "1d4");
    let outcome = surface::act(&mut world, actor, ability, target);
    assert_eq!(outcome.damage, 1 + dice_roll.total as u32);
    assert_eq!(*world.rng_mut(), rng);
}
// ---------------------------------------------------------------------------
// B17: initiative
// ---------------------------------------------------------------------------

#[test]
fn rolled_initiative_keys_equal_precomputed_sums() {
    let mut fixture = Fixture::standard();
    draws::set_initiative_rule(&mut fixture, "1d20", &["alpha"]);
    let (world, ids) = start(&fixture, 5);
    let (key_a, key_b) = rolled_keys(5, "1d20", 8, 6, 10, 9);
    assert_eq!(
        world.combatants().get(ids[0]).expect("A").initiative(),
        key_a
    );
    assert_eq!(
        world.combatants().get(ids[1]).expect("B").initiative(),
        key_b
    );
}

#[test]
fn initiative_ties_break_on_entity_id() {
    let mut fixture = Fixture::standard();
    draws::set_initiative_rule(&mut fixture, "1d1", &[]);
    draws::set_authored_initiative(&mut fixture, 5, 5);
    let (world, ids) = start(&fixture, 2);
    let ka = world.combatants().get(ids[0]).expect("A").initiative();
    let kb = world.combatants().get(ids[1]).expect("B").initiative();
    assert_eq!(ka, kb);
    let timeline = rt::timeline_of(&world);
    assert_eq!(timeline[0].1, ids[0]);
    assert_eq!(world.combat().expect("encounter").active, Some(ids[0]));
}

#[test]
fn tb_rollover_reuses_rolled_keys() {
    let mut fixture = Fixture::standard();
    draws::set_initiative_rule(&mut fixture, "1d20", &["alpha"]);
    let (mut world, _ids) = start(&fixture, 6);
    let keys = rt::timeline_of(&world);
    let ability = fixture.ability;
    let first = keys[0].1;
    let second = keys[1].1;
    surface::act(&mut world, first, ability, second);
    let before = world.rng_mut().clone();
    surface::end(&mut world, second);
    assert_eq!(rt::timeline_of(&world), keys, "the same keys");
    assert_eq!(*world.rng_mut(), before, "the rollover draws nothing");
}

#[test]
fn rt_start_keys_zero_and_rebuilds_at_switch() {
    let mut fixture = rt::capable(4);
    draws::set_initiative_rule(&mut fixture, "1d20", &["alpha"]);
    let seed = 5u64;
    let (mut world, ids) = fixture.start_rt(seed, EncounterMode::RealTime);
    let (key_a, key_b) = rolled_keys(seed, "1d20", 8, 6, 10, 9);
    assert_eq!(rt::timeline_of(&world), vec![(0, ids[0]), (0, ids[1])]);
    assert_eq!(
        world.combatants().get(ids[0]).expect("A").initiative(),
        key_a
    );
    assert_eq!(
        world.combatants().get(ids[1]).expect("B").initiative(),
        key_b
    );
    let mut after = draws::oracle(seed);
    draws::draw(&mut after, "1d20");
    draws::draw(&mut after, "1d20");
    assert_eq!(*world.rng_mut(), after);

    request_mode(&mut world, EncounterMode::TurnBased).expect("request");
    rt::ticks(&mut world, 4);
    assert_eq!(
        world.combat().expect("encounter").mode,
        EncounterMode::TurnBased
    );
    let mut expected = vec![(key_a, ids[0]), (key_b, ids[1])];
    expected.sort();
    assert_eq!(rt::timeline_of(&world), expected);
}

#[test]
fn no_initiative_rule_means_no_draw() {
    let fixture = Fixture::standard();
    let (mut world, ids) = start(&fixture, 7);
    assert!(!world.rng_mut().has_stream(COMBAT_ROLL_STREAM));
    assert_eq!(*world.rng_mut(), draws::oracle(7));
    assert_eq!(rt::timeline_of(&world), vec![(9, ids[1]), (10, ids[0])]);
}

#[test]
fn rolled_key_saturates_at_both_ends() {
    let mut fixture = Fixture::standard();
    draws::set_initiative_rule(&mut fixture, "1d6", &[]);
    draws::set_authored_initiative(&mut fixture, i32::MAX, i32::MIN);
    let (world, ids) = start(&fixture, 3);
    assert_eq!(
        world.combatants().get(ids[0]).expect("A").initiative(),
        i32::MAX
    );

    let mut low = Fixture::standard();
    draws::set_initiative_rule(&mut low, "1d4-5", &[]);
    draws::set_authored_initiative(&mut low, i32::MAX, i32::MIN);
    let (world, ids) = start(&low, 3);
    assert_eq!(
        world.combatants().get(ids[1]).expect("B").initiative(),
        i32::MIN
    );
}

#[test]
fn initiative_bonus_and_total_saturate_instead_of_wrapping() {
    // Stored stats are 16.16 fixed point, so near-`i32` bonuses come from
    // derived `Int` stats with literal formulas.
    let key_a = |bonus: &[&str]| {
        let mut fixture = Fixture::standard();
        surface::add_derived(&mut fixture, "big", surface::lit(i32::MAX));
        surface::add_derived(&mut fixture, "small", surface::lit(5));
        surface::add_derived(&mut fixture, "low", surface::lit(i32::MIN));
        surface::add_derived(&mut fixture, "minus", surface::lit(-1));
        draws::set_initiative_rule(&mut fixture, "1d6", bonus);
        let (world, ids) = start(&fixture, 3);
        let roll = draws::draw(&mut draws::oracle(3), "1d6").total;
        let key = world.combatants().get(ids[0]).expect("A").initiative();
        (key, roll)
    };
    // The bonus sum saturates (wrapping would give a very negative sum).
    assert_eq!(key_a(&["big", "small"]).0, i32::MAX);
    // The dice total plus a maximal bonus, then the authored 10, saturate.
    assert_eq!(key_a(&["big"]).0, i32::MAX);
    // Negative end: the bonus sum saturates at i32::MIN, then roll + 10 add.
    let (key, roll) = key_a(&["low", "minus"]);
    assert_eq!(key, i32::MIN + roll + 10);
}

#[test]
fn rejected_start_with_initiative_rule_draws_nothing() {
    let mut fixture = Fixture::standard();
    draws::set_initiative_rule(&mut fixture, "1d20", &["alpha"]);
    let creature_id = fixture.creature_a;
    let tags: Vec<String> = (0..1024).map(|i| format!("tag-{i}")).collect();
    fixture
        .creatures
        .get_mut(&creature_id)
        .expect("creature")
        .tags = tags;
    let effect = surface::effect_doc(
        141,
        "target",
        "probe-type",
        "stack_all",
        vec![surface::modifier(142, json!({"type": "roll"}), "add", 0)],
        3,
    );
    let effect = draws::grant(effect, &["hasted"]);
    surface::attach_to_ability(&mut fixture, effect);

    let mut world = World::new(9);
    let before = world.rng_mut().clone();
    let err = draws::start_into(&fixture, &mut world);
    assert_eq!(err, Err(CombatError::TooManyTags));
    assert_eq!(*world.rng_mut(), before);
    assert!(world.combat().is_none());
}

// [b] rung: invalid initiative dice
#[test]
fn invalid_initiative_dice_rejects_start() {
    let mut fixture = Fixture::standard();
    fixture.ruleset.initiative =
        Some(serde_json::from_value(json!({"dice": "not-dice", "bonus_stats": []})).unwrap());
    let mut world = World::new(3);
    let before = world.clone();
    let err = draws::start_into(&fixture, &mut world);
    assert_eq!(err, Err(CombatError::InvalidDice));
    assert_eq!(world, before);
}

// [b] rung: Opposed invalid dice
#[test]
fn opposed_invalid_dice_rejects_start() {
    let mut fixture = Fixture::standard();
    draws::defend_opposed(&mut fixture, "not-dice", &[]);
    let mut world = World::new(3);
    let before = world.clone();
    let err = draws::start_into(&fixture, &mut world);
    assert_eq!(err, Err(CombatError::InvalidDice));
    assert_eq!(world, before);
}

// [b] rung: damage dice invalid
#[test]
fn damage_dice_invalid_rejects_start() {
    let mut fixture = Fixture::standard();
    draws::set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": 2, "dice": "not-dice"}]),
    );
    let mut world = World::new(3);
    let before = world.clone();
    let err = draws::start_into(&fixture, &mut world);
    assert_eq!(err, Err(CombatError::InvalidDice));
    assert_eq!(world, before);
}

// ---------------------------------------------------------------------------
// B19/B20: tags
// ---------------------------------------------------------------------------

// T139 D5 removed B19's carriage gate: every combatant carries its creature's
// tags, sorted and deduplicated, with or without a granting effect. The
// grants on `EffectDefinition` are carried only as authored.
#[test]
fn tags_projection_sorted_distinct_and_grants_carried() {
    let mut fixture = Fixture::standard();
    let creature_id = fixture.creature_a;
    draws::set_tags(&mut fixture, creature_id, &["pc", "hero", "hero"]);
    let (world, ids) = start(&fixture, 3);
    assert_eq!(
        world.combatants().get(ids[0]).expect("A").tags(),
        ["hero", "pc"]
    );
    assert!(world
        .combat()
        .expect("encounter")
        .definition
        .effects
        .iter()
        .all(|effect| effect.grants.is_empty()));

    let mut fixture = Fixture::standard();
    let creature_id = fixture.creature_a;
    draws::set_tags(&mut fixture, creature_id, &["pc", "hero", "hero"]);
    let effect = surface::effect_doc(
        141,
        "target",
        "probe-type",
        "stack_all",
        vec![surface::modifier(142, json!({"type": "roll"}), "add", 0)],
        3,
    );
    let effect = draws::grant(effect, &["hasted"]);
    surface::attach_to_ability(&mut fixture, effect);
    let (world, ids) = start(&fixture, 3);
    assert_eq!(
        world.combatants().get(ids[0]).expect("A").tags(),
        ["hero", "pc"]
    );
    let def = &world.combat().expect("encounter").definition;
    let effect_def = &def.effects[0];
    assert_eq!(effect_def.grants, vec!["hasted".to_owned()]);
}

#[test]
fn tag_union_accepted_at_bound_and_rejected_over() {
    // accepted at MAX_TAGS
    let mut fixture = Fixture::standard();
    let tags: Vec<String> = (0..1023).map(|i| format!("tag-{i}")).collect();
    fixture
        .creatures
        .get_mut(&fixture.creature_a)
        .expect("c")
        .tags = tags;
    let effect = surface::effect_doc(
        141,
        "target",
        "probe-type",
        "stack_all",
        vec![surface::modifier(142, json!({"type": "roll"}), "add", 0)],
        3,
    );
    let effect = draws::grant(effect, &["hasted"]);
    surface::attach_to_ability(&mut fixture, effect);
    let (world, _) = start(&fixture, 3);
    assert!(world.combat().is_some());

    // rejected at MAX_TAGS + 1
    let mut fixture = Fixture::standard();
    let tags: Vec<String> = (0..1024).map(|i| format!("tag-{i}")).collect();
    fixture
        .creatures
        .get_mut(&fixture.creature_a)
        .expect("c")
        .tags = tags;
    let effect = surface::effect_doc(
        141,
        "target",
        "probe-type",
        "stack_all",
        vec![surface::modifier(142, json!({"type": "roll"}), "add", 0)],
        3,
    );
    let effect = draws::grant(effect, &["hasted"]);
    surface::attach_to_ability(&mut fixture, effect);
    let mut world = World::new(3);
    let before = world.rng_mut().clone();
    let err = draws::start_into(&fixture, &mut world);
    assert_eq!(err, Err(CombatError::TooManyTags));
    assert_eq!(err.unwrap_err().to_string(), "TooManyTags at spec/tags");
    assert_eq!(*world.rng_mut(), before);
}

fn tags_fixture(count: usize, grant: Option<&[&str]>) -> Fixture {
    let mut fixture = Fixture::standard();
    let tags: Vec<String> = (0..count).map(|i| format!("tag-{i:04}")).collect();
    fixture
        .creatures
        .get_mut(&fixture.creature_a)
        .expect("c")
        .tags = tags;
    if let Some(grant) = grant {
        let effect = surface::effect_doc(
            141,
            "target",
            "probe-type",
            "stack_all",
            vec![surface::modifier(142, json!({"type": "roll"}), "add", 0)],
            3,
        );
        surface::attach_to_ability(&mut fixture, draws::grant(effect, grant));
    }
    fixture
}

// T139 D5: the bound runs at every start, with an empty grant set when
// nothing grants (B20 as amended).
#[test]
fn tag_bound_runs_without_a_granting_effect() {
    let (world, _) = start(&tags_fixture(1024, None), 3);
    assert!(world.combat().is_some(), "1024 distinct tags are accepted");

    let fixture = tags_fixture(1025, None);
    let mut world = World::new(3);
    let before = world.clone();
    let err = draws::start_into(&fixture, &mut world);
    assert_eq!(err, Err(CombatError::TooManyTags));
    assert_eq!(world, before, "a rejected start leaves the world unchanged");
}

#[test]
fn tag_bound_counts_distinct_union_of_tags_and_grants() {
    // The grant repeats an existing tag: the union stays at 1024.
    let (world, _) = start(&tags_fixture(1024, Some(&["tag-0000"])), 3);
    assert!(world.combat().is_some());
    // Two distinct new grants over 1023 tags reach 1025.
    let fixture = tags_fixture(1023, Some(&["new-a", "new-b"]));
    let mut world = World::new(3);
    assert_eq!(
        draws::start_into(&fixture, &mut world),
        Err(CombatError::TooManyTags)
    );
    // One new grant over 1023 tags reaches exactly 1024.
    let (world, _) = start(&tags_fixture(1023, Some(&["new-a"])), 3);
    assert!(world.combat().is_some());
}

// ---------------------------------------------------------------------------
// B23 L6: load coherence
// ---------------------------------------------------------------------------

#[test]
fn load_rejects_combatant_over_tag_bound() {
    let mut fixture = Fixture::standard();
    let effect = surface::effect_doc(
        141,
        "target",
        "probe-type",
        "stack_all",
        vec![surface::modifier(142, json!({"type": "roll"}), "add", 0)],
        3,
    );
    let effect = draws::grant(effect, &["hasted"]);
    surface::attach_to_ability(&mut fixture, effect);
    let creature_id = fixture.creature_a;
    draws::set_tags(&mut fixture, creature_id, &["hero"]);
    let (world, _) = start(&fixture, 6);

    let mut at_bound = save(&world);
    let names: Vec<String> = (0..1023).map(|i| format!("tag-{i:04}")).collect();
    at_bound["combatants"][0][1]["tags"] = json!(names);
    load(at_bound).expect("1023 tags plus one grant loads");

    let mut over = save(&world);
    let names: Vec<String> = (0..1024).map(|i| format!("tag-{i:04}")).collect();
    over["combatants"][0][1]["tags"] = json!(names);
    let err = load(over).expect_err("1025 distinct tags are refused");
    assert!(err
        .to_string()
        .contains("combatant tags exceed their bound"));

    let mut repeat = save(&world);
    let mut tags = (0..1023).map(|i| format!("tag-{i:04}")).collect::<Vec<_>>();
    tags.push("hasted".to_owned());
    repeat["combatants"][0][1]["tags"] = json!(tags);
    load(repeat).expect("a repeated grant counts once");
}

// ---------------------------------------------------------------------------
// B24: replay and save/load continuity
// ---------------------------------------------------------------------------

const SCRIPT_STEPS: usize = 6;

/// Runs a fixed script of up to `SCRIPT_STEPS` actions (the active
/// combatant uses the ability on the other one), recording the state hash
/// and the action report after each step. When `reload_at` is `Some(k)` the
/// world is saved and loaded again after step `k`. Returns the trace and the
/// final world.
fn run_script(seed: u64, reload_at: Option<usize>) -> (Vec<(String, [u8; 32])>, World) {
    let mut fixture = Fixture::standard();
    draws::defend_opposed(&mut fixture, "1d6", &["beta"]);
    draws::set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": 1, "dice": "1d4"}]),
    );
    draws::set_initiative_rule(&mut fixture, "1d20", &["alpha"]);
    let effect = surface::effect_doc(
        141,
        "target",
        "probe-type",
        "stack_all",
        vec![surface::modifier(142, json!({"type": "roll"}), "add", 1)],
        3,
    );
    let effect = draws::grant(effect, &["hasted"]);
    surface::attach_to_ability(&mut fixture, effect);
    let creature_id = fixture.creature_a;
    draws::set_tags(&mut fixture, creature_id, &["hero"]);
    fixture.ruleset.pools[0].max = 20;
    surface::edit_ability(&mut fixture, |ability| ability.cost = 1);

    let (mut world, ids) = start(&fixture, seed);
    let ability = fixture.ability;
    let mut trace = vec![("start".to_owned(), state_hash(&world))];
    if reload_at == Some(0) {
        world = reload(world);
    }
    for step in 1..=SCRIPT_STEPS {
        let Some(actor) = world.combat().and_then(|combat| combat.active) else {
            break;
        };
        let target = if actor == ids[0] { ids[1] } else { ids[0] };
        let report = perform_action(
            &mut world,
            &CombatAction::UseAbility {
                actor,
                ability,
                target,
            },
        );
        let report = format!("{report:?}");
        trace.push((report, state_hash(&world)));
        if reload_at == Some(step) {
            world = reload(world);
        }
    }
    (trace, world)
}

#[test]
fn same_seed_replay_is_identical_and_other_seed_differs() {
    let (a, wa) = run_script(9, None);
    let (b, wb) = run_script(9, None);
    assert!(a.len() >= 4, "the script performs several actions");
    assert!(
        a[1..]
            .iter()
            .all(|(report, _)| report.starts_with("Ok(Some(")),
        "every scripted action is accepted: {a:?}"
    );
    assert_eq!(a, b);
    assert_eq!(wa, wb);
    assert_ne!(a, run_script(10, None).0);
}

#[test]
fn save_and_load_mid_run_continues_identically() {
    let (straight, world) = run_script(9, None);
    for k in 0..straight.len() {
        let (resumed, resumed_world) = run_script(9, Some(k));
        assert_eq!(straight, resumed, "reloaded after step {k}");
        assert_eq!(world, resumed_world, "final world after reload at {k}");
    }
}
