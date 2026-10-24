//! T136b authored-feature bridge coverage (T136c, C1–C4), narrowed by T137a.
//!
//! One case per trigger the adapter still refuses until T137b: a rolled
//! initiative rule (C2b), an opposed defense, damage dice (C2c), and effect
//! grants (C2d). T137a executes the other triggers and deleted their cases
//! (R8). Every case must return the bridge `CombatError` variant, publish no
//! state, and render the pinned location; the grants case stays
//! un-referenced on purpose, because the contract refuses *any* value in
//! `spec.effects`, not only the ones the encounter uses.

#[allow(dead_code)]
mod support;

use crpg_data::{DefenseWire, Effect, InitiativeWire};
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
