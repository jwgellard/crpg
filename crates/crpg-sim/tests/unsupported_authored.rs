//! T136b authored-feature bridge coverage (T136c, C1–C4).
//!
//! One case per trigger the adapter refuses until T137: a non-`Int` or
//! derived stat (C2a), a rolled initiative rule (C2b), ability bonus stats,
//! a non-legacy defense, damage dice (C2c), effect grants, and a `Stat`
//! effect target (C2d). Every case must return the last `CombatError`
//! variant, publish no state, and render the pinned location; the effect
//! cases stay un-referenced on purpose, because the contract refuses *any*
//! value in `spec.effects`, not only the ones the encounter uses.

#[allow(dead_code)]
mod support;

use crpg_data::{DefenseWire, Effect, ExprWire, InitiativeWire, StatKindWire};
use crpg_sim::{start_encounter, state_hash, CombatError, EncounterSpec, World};
use serde_json::json;
use support::{uid, Fixture};

/// Runs one fixture through `start_encounter` and asserts the bridge
/// rejection and its unchanged world.
fn assert_unsupported(fixture: &Fixture) {
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
    let mut world = World::new(5);
    let before = state_hash(&world);
    let error = start_encounter(&mut world, &spec).expect_err("the authored feature is refused");
    assert_eq!(error, CombatError::UnsupportedAuthored);
    assert_eq!(
        error.to_string(),
        "UnsupportedAuthored at spec/authored",
        "C1 pins the Display location"
    );
    assert_eq!(
        state_hash(&world),
        before,
        "a rejected encounter leaves the world unchanged"
    );
    assert!(world.combat().is_none(), "no encounter is published");
    assert!(world.is_empty(), "no participant is spawned");
    assert!(world.timeline().is_empty(), "no turn is scheduled");
}

/// The standard ability, mutably, for one trigger.
fn ability(fixture: &mut Fixture) -> &mut crpg_data::Ability {
    fixture
        .abilities
        .get_mut(&fixture.ability)
        .expect("the standard ability exists")
}

/// A parse-valid effect document with the supplied modifier target.
fn effect(modifier_target: serde_json::Value) -> Effect {
    serde_json::from_value(json!({
        "id": uid(141),
        "slug": "probe-effect",
        "name": "probe.effect",
        "aim": {"type": "slf"},
        "mod_type": "probe-type",
        "policy": {"type": "stack_all"},
        "modifiers": [
            {"id": uid(142), "target": modifier_target, "op": {"type": "add"},
             "value": 1, "priority": 0, "name": null}
        ],
        "duration_rounds": 1
    }))
    .expect("the probe effect is valid")
}

/// C2a: the stat-kind bridge.
#[test]
fn non_int_stat_kind_is_unsupported() {
    let mut fixture = Fixture::standard();
    fixture.ruleset.stats[0].kind = StatKindWire::Bool;
    assert_unsupported(&fixture);
}

/// C2a: the derived-formula bridge.
#[test]
fn derived_stat_is_unsupported() {
    let mut fixture = Fixture::standard();
    fixture.ruleset.stats[0].derived = Some(ExprWire::Stat("beta".to_owned()));
    assert_unsupported(&fixture);
}

/// C2b: the rolled-initiative bridge.
#[test]
fn rolled_initiative_is_unsupported() {
    let mut fixture = Fixture::standard();
    fixture.ruleset.initiative = Some(InitiativeWire {
        dice: "1d6".to_owned(),
        bonus_stats: vec!["alpha".to_owned()],
    });
    assert_unsupported(&fixture);
}

/// C2c: the ability bonus-stat bridge.
#[test]
fn ability_bonus_stats_are_unsupported() {
    let mut fixture = Fixture::standard();
    ability(&mut fixture).bonus_stats = Some(vec!["alpha".to_owned()]);
    assert_unsupported(&fixture);
}

/// C2c: the constant-defense bridge.
#[test]
fn constant_defense_is_unsupported() {
    let mut fixture = Fixture::standard();
    ability(&mut fixture).defense = DefenseWire::Constant(12);
    assert_unsupported(&fixture);
}

/// C2c: the actor-stat defense bridge.
#[test]
fn actor_stat_defense_is_unsupported() {
    let mut fixture = Fixture::standard();
    ability(&mut fixture).defense = DefenseWire::ActorStat {
        stat: "alpha".to_owned(),
    };
    assert_unsupported(&fixture);
}

/// C2c: the opposed-roll defense bridge.
#[test]
fn opposed_defense_is_unsupported() {
    let mut fixture = Fixture::standard();
    ability(&mut fixture).defense = DefenseWire::Opposed {
        dice: "1d6".to_owned(),
        bonus_stats: vec!["alpha".to_owned()],
    };
    assert_unsupported(&fixture);
}

/// C2c: the damage-dice bridge.
#[test]
fn damage_dice_are_unsupported() {
    let mut fixture = Fixture::standard();
    ability(&mut fixture).damage[0].dice = Some("1d4".to_owned());
    assert_unsupported(&fixture);
}

/// C2d: the effect-grants bridge, on an un-referenced effect.
#[test]
fn effect_grants_are_unsupported() {
    let mut fixture = Fixture::standard();
    let mut document = effect(json!({"type": "roll"}));
    document.grants = Some(vec!["probe-tag".to_owned()]);
    fixture.effects.insert(document.id, document);
    assert_unsupported(&fixture);
}

/// C2d: the `Stat` effect-target bridge, on an un-referenced effect.
#[test]
fn stat_effect_target_is_unsupported() {
    let mut fixture = Fixture::standard();
    let document = effect(json!({"type": "stat", "value": {"stat": "alpha"}}));
    fixture.effects.insert(document.id, document);
    assert_unsupported(&fixture);
}
