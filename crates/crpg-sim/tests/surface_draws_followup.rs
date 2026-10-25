//! T137 follow-up contract tests (two-keys gaps): the self-targeted `Opposed`
//! bonus-list concatenation (B13/B14) and mid-script save/load continuity
//! (B24), checked through the public `crpg-sim` API against `tasks/T137.md`.

#[allow(dead_code)]
mod support;

use crpg_sim::{state_hash, World};
use serde_json::json;
use support::draws;
use support::surface;
use support::Fixture;

fn start(fixture: &Fixture, seed: u64) -> (World, Vec<crpg_core::EntityId>) {
    surface::start(fixture, seed)
}

fn reload(world: World) -> World {
    let value = serde_json::to_value(&world).expect("the world serializes");
    serde_json::from_value(value).expect("the world reloads")
}

/// B13/B14: the actor list is `alpha`, the opposing list is `gamma`, with
/// distinct values per creature so a wrong-side fold shifts the margin.
/// A (alpha 2, gamma 7), B (alpha 10, gamma 3); B acts first.
fn self_opposed_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    draws::defend_opposed(&mut fixture, "2d6", &["gamma"]);
    surface::edit_ability(&mut fixture, |ability| {
        ability.allow_self_target = true;
        ability.bonus_stats = Some(vec!["alpha".to_owned()]);
    });
    let (a, b) = (fixture.creature_a, fixture.creature_b);
    surface::set_stat(&mut fixture, a, "alpha", 2);
    surface::set_stat(&mut fixture, a, "gamma", 7);
    surface::set_stat(&mut fixture, b, "alpha", 10);
    surface::set_stat(&mut fixture, b, "gamma", 3);
    fixture
}

// ---------------------------------------------------------------------------
// B13/B14: self-targeted `Opposed` concatenates the bonus lists
// ---------------------------------------------------------------------------

/// B14: on a self-targeted use the one view holds the actor list followed by
/// the opposing list, so both rolls fold both. The folded totals therefore
/// both gain the same sum (B: alpha 10 + gamma 3) and the margin is the raw
/// difference; the draws are the actor's dice then the opposing dice.
#[test]
fn self_targeted_opposed_folds_both_bonus_lists_into_both_rolls() {
    for seed in [5u64, 6, 7] {
        let fixture = self_opposed_fixture();
        let ability = fixture.ability;
        let (mut world, ids) = start(&fixture, seed);
        let actor = ids[1]; // B acts first

        let mut rng = world.rng_mut().clone();
        let raw_actor = draws::draw(&mut rng, "2d6");
        let raw_opposing = draws::draw(&mut rng, "2d6");
        let shared = 10 + 3;
        let actor_folded = i64::from(raw_actor.total) + shared;
        let opposing_folded = i64::from(raw_opposing.total) + shared;

        let outcome = surface::act(&mut world, actor, ability, actor);
        assert_eq!(
            outcome.margin,
            actor_folded - opposing_folded,
            "seed {seed}: both folded totals carry both lists"
        );
        assert_eq!(
            outcome.margin,
            i64::from(raw_actor.total) - i64::from(raw_opposing.total),
            "seed {seed}: equal folds cancel in the margin"
        );
        assert_eq!(
            *world.rng_mut(),
            rng,
            "seed {seed}: actor dice first, opposing dice second, one stream"
        );
    }
}

/// B13/B14 control: on a different target each roll folds only its own
/// side's list. A (alpha 2) attacks B (gamma 3), so the actor total gains 2
/// and the opposing total gains 3: the margin is the raw difference minus 1.
#[test]
fn different_target_opposed_folds_only_its_own_side_list() {
    for seed in [5u64, 6, 7] {
        let fixture = self_opposed_fixture();
        let ability = fixture.ability;
        let (mut world, ids) = start(&fixture, seed);
        let (a, b) = (ids[0], ids[1]);
        surface::end(&mut world, b); // B acts first; hand the turn to A

        let mut rng = world.rng_mut().clone();
        let raw_actor = draws::draw(&mut rng, "2d6");
        let raw_opposing = draws::draw(&mut rng, "2d6");
        let actor_folded = i64::from(raw_actor.total) + 2; // A's alpha
        let opposing_folded = i64::from(raw_opposing.total) + 3; // B's gamma

        let outcome = surface::act(&mut world, a, ability, b);
        assert_eq!(
            outcome.margin,
            actor_folded - opposing_folded,
            "seed {seed}: the actor's list stays on the actor roll"
        );
        assert_eq!(
            outcome.margin,
            i64::from(raw_actor.total) - i64::from(raw_opposing.total) - 1,
            "seed {seed}: the opposing list stays on the opposing roll"
        );
        assert_eq!(
            *world.rng_mut(),
            rng,
            "seed {seed}: actor dice first, opposing dice second"
        );
    }
}

// ---------------------------------------------------------------------------
// B24: save/load continuity in the middle of a script
// ---------------------------------------------------------------------------

/// Every T137b shape at once: an `Opposed` defence with its own bonus, damage
/// dice, a rolled initiative rule and an effect that grants a tag.
fn replay_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    draws::defend_opposed(&mut fixture, "1d6", &["beta"]);
    draws::set_damage(
        &mut fixture,
        json!([
            {"outcome": {"type": "success"}, "amount": 1, "dice": "1d2"},
            {"outcome": {"type": "failure"}, "amount": 0}
        ]),
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
    let (a, b) = (fixture.creature_a, fixture.creature_b);
    draws::set_tags(&mut fixture, a, &["hero"]);
    // Four successes must not end the fight early.
    surface::set_stat(&mut fixture, a, "health", 100);
    surface::set_stat(&mut fixture, b, "health", 100);
    surface::always_success(&mut fixture);
    fixture
}

const SCRIPT_STEPS: usize = 4;

/// Runs four accepted actions, reloading after action `reload_at`, and
/// returns one `state_hash` after every step.
fn run_script(seed: u64, reload_at: Option<usize>) -> Vec<[u8; 32]> {
    let fixture = replay_fixture();
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, seed);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        world.combat().expect("encounter").definition.effects[0].grants,
        vec!["hasted".to_owned()],
        "the fixture carries the tag grant"
    );

    let mut hashes = Vec::new();
    for step in 1..=SCRIPT_STEPS {
        let actor = world.combat().expect("encounter").active.expect("active");
        let target = if actor == a { b } else { a };
        assert!(
            world.combatants().get(target).expect("target").health() > 0,
            "step {step}: the script keeps both combatants alive"
        );
        surface::act(&mut world, actor, ability, target);
        if reload_at == Some(step) {
            world = reload(world);
        }
        hashes.push(state_hash(&world));
    }
    hashes
}

/// B24: two same-seed runs agree, a reload after any of the first three
/// actions continues to the same per-step hashes, and a different seed gives
/// a different sequence.
#[test]
fn save_load_after_each_mid_script_action_preserves_the_hashes() {
    let base = run_script(9, None);
    assert_eq!(base.len(), SCRIPT_STEPS);
    assert_eq!(base, run_script(9, None), "two same-seed runs agree");
    for k in 1..=3 {
        assert_eq!(
            run_script(9, Some(k)),
            base,
            "a reload after action {k} continues to the same hashes"
        );
    }
    assert_ne!(
        run_script(10, None),
        base,
        "a different seed gives a different sequence"
    );
}
