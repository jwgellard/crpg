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
    let a_roll = draws::draw(&mut rng, dice).total as i32;
    let b_roll = draws::draw(&mut rng, dice).total as i32;
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
    assert_eq!((before - after) as u32, outcome.damage);
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
    assert!(matches!(err, Err(CombatError::InvalidDice { .. })));
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
    assert!(matches!(err, Err(CombatError::InvalidDice { .. })));
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
    assert!(matches!(err, Err(CombatError::InvalidDice { .. })));
    assert_eq!(world, before);
}

// ---------------------------------------------------------------------------
// B19/B20: tags
// ---------------------------------------------------------------------------

#[test]
fn tags_carried_only_with_a_granting_effect() {
    let mut fixture = Fixture::standard();
    let creature_id = fixture.creature_a;
    draws::set_tags(&mut fixture, creature_id, &["pc", "hero", "hero"]);
    let (world, ids) = start(&fixture, 3);
    assert!(world.combatants().get(ids[0]).expect("A").tags().is_empty());

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

fn run_script(seed: u64, reload_at: Option<usize>) -> Vec<[u8; 32]> {
    let mut fixture = Fixture::standard();
    draws::defend_opposed(&mut fixture, "1d6", &["beta"]);
    draws::set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": 2, "dice": "1d4"}]),
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

    let (mut world, ids) = start(&fixture, seed);
    let ability = fixture.ability;
    let mut hashes = vec![state_hash(&world)];

    let actor = world.combat().expect("encounter").active.expect("active");
    let target = if actor == ids[0] { ids[1] } else { ids[0] };
    surface::act(&mut world, actor, ability, target);
    if reload_at == Some(1) {
        world = reload(world);
    }
    hashes.push(state_hash(&world));

    let next_actor = world.combat().expect("encounter").active;
    if let Some(next) = next_actor {
        let next_target = if next == ids[0] { ids[1] } else { ids[0] };
        perform_action_raw(&mut world, next, ability, next_target);
    }
    if reload_at == Some(2) {
        world = reload(world);
    }
    hashes.push(state_hash(&world));

    hashes
}

fn perform_action_raw(world: &mut World, actor: EntityId, ability: Ulid, target: EntityId) {
    let _ = perform_action(
        world,
        &CombatAction::UseAbility {
            actor,
            ability,
            target,
        },
    );
}

#[test]
fn same_seed_replay_and_save_load_continuity() {
    let a = run_script(9, None);
    let b = run_script(9, None);
    assert!(a.len() >= 3);
    assert_eq!(a, b);
    assert_ne!(a, run_script(10, None));
    let c = run_script(9, Some(1));
    assert_eq!(a, c);
}
