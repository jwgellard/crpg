//! T137a typed and derived stats at start (R1, R2).
//!
//! R1: `Int` stats are read, a non-derived `Fixed` stat must be present, and
//! every other stat kind is carried by nothing (B2, B3, B4). R2: the start
//! preflight and the pinned start-error order for the new checks (B5, B21).

#[allow(dead_code)]
mod support;

use crpg_core::Ulid;
use crpg_sim::{start_encounter, state_hash, CombatError, World};
use serde_json::json;
use support::surface::{
    add_attribute, add_derived, add_stat, attach_to_ability, binary, edit_ability, effect_doc, lit,
    modifier, remove_stat, set_stat, set_stat_raw, start, stat, stat_target, try_start,
};
use support::Fixture;

/// `half = (alpha + beta) / 2` over the standard stats, plus a base `Fixed`,
/// a base `Bool` and a derived `Tags` stat.
fn typed_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    add_derived(
        &mut fixture,
        "half",
        binary("divide", binary("add", stat("alpha"), stat("beta")), lit(2)),
    );
    add_stat(&mut fixture, json!({"name": "speed", "kind": "fixed"}));
    add_stat(&mut fixture, json!({"name": "flag", "kind": "bool"}));
    add_stat(
        &mut fixture,
        json!({"name": "marks", "kind": "tags",
               "derived": {"type": "literal", "value": {"type": "tags", "value": []}}}),
    );
    let (a, b) = (fixture.creature_a, fixture.creature_b);
    set_stat(&mut fixture, a, "speed", 3);
    set_stat(&mut fixture, b, "speed", 4);
    fixture
}

fn start_error(fixture: &Fixture) -> CombatError {
    try_start(fixture, 5).expect_err("the start is refused")
}

/// R1 (B2, B3, B4): only base `Int` stats are read; the derived formula is
/// carried symbolically in exact bytes.
#[test]
fn base_ints_derived_formula_and_typed_stats_at_start() {
    let fixture = typed_fixture();
    let (world, ids) = start(&fixture, 5);
    let definition = &world.combat().expect("encounter").definition;
    assert_eq!(definition.attribute_names, ["alpha", "beta", "gamma"]);
    let a = world.combatants().get(ids[0]).expect("combatant A");
    assert_eq!(
        a.attributes(),
        [
            ("alpha".to_owned(), 8),
            ("beta".to_owned(), 6),
            ("gamma".to_owned(), 7)
        ]
    );
    assert_eq!(a.health(), 10);
    assert_eq!(
        serde_json::to_string(&definition.derived).expect("derived serializes"),
        concat!(
            r#"[{"name":"half","formula":{"type":"divide","value":{"left":"#,
            r#"{"type":"add","value":{"left":{"type":"stat","value":"alpha"},"#,
            r#""right":{"type":"stat","value":"beta"}}},"#,
            r#""right":{"type":"literal","value":2}}}}]"#
        )
    );
    let saved = serde_json::to_value(&world).expect("the world serializes");
    assert_eq!(
        saved["combat"]["definition"]["derived"][0]["name"],
        json!("half")
    );
    assert!(
        world.interners().stats().is_empty(),
        "the start preflight interns nothing into the world"
    );
}

/// R1 (B2): a creature without the `Bool` stat starts, and the typed stats
/// never reach a combatant.
#[test]
fn a_creature_without_the_bool_stat_starts() {
    let fixture = typed_fixture();
    assert!(!fixture.creatures[&fixture.creature_a]
        .stats
        .contains_key("flag"));
    let (world, ids) = start(&fixture, 5);
    for id in ids {
        let combatant = world.combatants().get(id).expect("combatant");
        assert!(combatant
            .attributes()
            .iter()
            .all(|(name, _)| name != "flag" && name != "speed" && name != "half"));
    }
}

/// R1 (B2): a missing base `Fixed` stat is `MissingStat`.
#[test]
fn a_missing_fixed_stat_is_missing() {
    let mut fixture = typed_fixture();
    let b = fixture.creature_b;
    remove_stat(&mut fixture, b, "speed");
    assert_eq!(
        start_error(&fixture),
        CombatError::MissingStat {
            stat: "speed".to_owned()
        }
    );
}

/// R1 (B2): a fractional `Fixed` stat starts, a fractional `Int` does not.
#[test]
fn a_fractional_fixed_starts_and_a_fractional_int_does_not() {
    let mut fixture = typed_fixture();
    let a = fixture.creature_a;
    set_stat_raw(&mut fixture, a, "speed", 98_304);
    let (world, _) = start(&fixture, 5);
    assert!(world.combat().is_some());
    set_stat_raw(&mut fixture, a, "alpha", 98_304);
    assert_eq!(
        start_error(&fixture),
        CombatError::InvalidStatValue {
            stat: "alpha".to_owned()
        }
    );
}

/// R1 (B2): a nonpositive health still fails, and a derived stat of any kind
/// is not required in `Creature.stats`.
#[test]
fn health_discipline_is_unchanged_and_derived_stats_need_no_value() {
    let mut fixture = typed_fixture();
    let a = fixture.creature_a;
    assert!(!fixture.creatures[&a].stats.contains_key("half"));
    assert!(!fixture.creatures[&a].stats.contains_key("marks"));
    set_stat(&mut fixture, a, "health", 0);
    assert_eq!(
        start_error(&fixture),
        CombatError::InvalidStatValue {
            stat: "health".to_owned()
        }
    );
}

/// R1 (B4): an `Int` formula holding a non-`Int` literal breaks the data
/// precondition.
#[test]
#[should_panic(expected = "derived int stats are validated by data read-time checks")]
fn a_fixed_literal_in_an_int_formula_panics() {
    let mut fixture = Fixture::standard();
    add_derived(
        &mut fixture,
        "bad",
        json!({"type": "literal", "value": {"type": "fixed", "value": 65_536}}),
    );
    let _ = try_start(&fixture, 5);
}

/// R1 (B4): an `Int` formula naming a stat that is not an `Int` stat breaks
/// the data precondition.
#[test]
#[should_panic(expected = "derived int stats are validated by data read-time checks")]
fn a_formula_naming_a_fixed_stat_panics() {
    let mut fixture = Fixture::standard();
    add_stat(&mut fixture, json!({"name": "speed", "kind": "fixed"}));
    add_derived(&mut fixture, "bad", stat("speed"));
    let _ = try_start(&fixture, 5);
}

/// R1 (B4): a formula set the kernel refuses to build breaks the data
/// precondition.
#[test]
#[should_panic(expected = "derived int stats are validated by data read-time checks")]
fn a_cyclic_formula_set_panics() {
    let mut fixture = Fixture::standard();
    add_derived(&mut fixture, "left", stat("right"));
    add_derived(&mut fixture, "right", stat("left"));
    let _ = try_start(&fixture, 5);
}

/// A fixture whose creature B has a zero `beta`, read as a divisor.
fn zero_divisor_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    add_derived(
        &mut fixture,
        "ratio",
        binary("divide", stat("alpha"), stat("beta")),
    );
    let b = fixture.creature_b;
    set_stat(&mut fixture, b, "beta", 0);
    fixture
}

/// R2 (B5): a zero base divisor fails the preflight naming the derived stat,
/// and the world, its interner and its RNG are untouched.
#[test]
fn a_zero_divisor_fails_the_preflight_and_changes_nothing() {
    let fixture = zero_divisor_fixture();
    let mut world = World::new(11);
    let before = world.clone();
    let hash = state_hash(&world);
    let bytes = serde_json::to_vec(&world).expect("the world serializes");
    let error = fixture
        .with_spec(|spec| start_encounter(&mut world, spec))
        .expect_err("the preflight fails");
    assert_eq!(
        error,
        CombatError::InvalidStatValue {
            stat: "ratio".to_owned()
        }
    );
    assert_eq!(error.to_string(), "InvalidStatValue at spec/stats");
    assert_eq!(world, before, "entities, RNG streams and interners match");
    assert_eq!(state_hash(&world), hash);
    assert_eq!(serde_json::to_vec(&world).expect("serializes"), bytes);
    assert!(world.interners().stats().is_empty());
    assert!(world.interners().tags().is_empty());
}

/// R2 (B5, B21): the preflight is participants outer and derived stats
/// inner, so participant A's failure on its second derived stat wins over
/// participant B's failure on the first. Swapping the nesting names `first`
/// instead. The rejected start changes no world byte, interner or RNG.
#[test]
fn the_preflight_scans_participants_before_derived_stats() {
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
    let (a, b) = (fixture.creature_a, fixture.creature_b);
    set_stat(&mut fixture, a, "gamma", 0);
    set_stat(&mut fixture, b, "beta", 0);

    let mut world = World::new(23);
    let before = world.clone();
    let bytes = serde_json::to_vec(&world).expect("the world serializes");
    let hash = state_hash(&world);
    let interners = world.interners().clone();
    let error = fixture
        .with_spec(|spec| start_encounter(&mut world, spec))
        .expect_err("the preflight fails on A's second derived stat");
    assert_eq!(
        error,
        CombatError::InvalidStatValue {
            stat: "second".to_owned()
        },
        "participants are the outer loop, derived stats the inner"
    );
    assert_eq!(error.to_string(), "InvalidStatValue at spec/stats");
    assert_eq!(world, before);
    assert_eq!(serde_json::to_vec(&world).expect("serializes"), bytes);
    assert_eq!(state_hash(&world), hash);
    assert_eq!(*world.interners(), interners);
    assert!(world.interners().stats().is_empty());
    assert!(
        !world.rng_mut().has_stream(crpg_sim::COMBAT_ROLL_STREAM),
        "the rejected start creates no RNG stream"
    );
}

/// R2 (B21): B2's per-stat errors and a missing creature precede the
/// preflight.
#[test]
fn the_preflight_comes_after_the_per_participant_checks() {
    let mut fixture = zero_divisor_fixture();
    let b = fixture.creature_b;
    remove_stat(&mut fixture, b, "gamma");
    assert_eq!(
        start_error(&fixture),
        CombatError::MissingStat {
            stat: "gamma".to_owned()
        },
        "B2 on participant B beats the preflight on B"
    );

    let mut fixture = zero_divisor_fixture();
    let a = fixture.creature_a;
    set_stat(&mut fixture, a, "beta", 0);
    fixture.creatures.remove(&fixture.creature_b);
    assert_eq!(
        start_error(&fixture),
        CombatError::MissingCreature {
            prefab: fixture.creature_b
        },
        "a missing creature beats the preflight on an earlier participant"
    );
}

/// R2 (B21): the preflight precedes every ability check.
#[test]
fn the_preflight_precedes_the_ability_checks() {
    let mut fixture = zero_divisor_fixture();
    edit_ability(&mut fixture, |ability| ability.dice = "bogus".to_owned());
    assert_eq!(
        start_error(&fixture),
        CombatError::InvalidStatValue {
            stat: "ratio".to_owned()
        }
    );
}

/// R2 (B21, step 7.2): the checked attribute must be usable, and that
/// precedes the dice check.
#[test]
fn a_non_usable_checked_attribute_is_missing_before_the_dice() {
    let mut fixture = typed_fixture();
    add_attribute(&mut fixture, "speed");
    edit_ability(&mut fixture, |ability| {
        ability.attribute = "speed".to_owned();
        ability.dice = "bogus".to_owned();
    });
    assert_eq!(
        start_error(&fixture),
        CombatError::MissingStat {
            stat: "speed".to_owned()
        }
    );
}

/// R2 (B21, step 7.6): a defence name must be usable, after the cost check.
#[test]
fn a_non_usable_defense_name_is_missing_after_the_costs() {
    let mut fixture = typed_fixture();
    edit_ability(&mut fixture, |ability| {
        ability.defense = crpg_data::DefenseWire::ActorStat {
            stat: "flag".to_owned(),
        };
    });
    assert_eq!(
        start_error(&fixture),
        CombatError::MissingStat {
            stat: "flag".to_owned()
        }
    );
    edit_ability(&mut fixture, |ability| ability.cost = 0);
    assert!(matches!(
        start_error(&fixture),
        CombatError::InvalidCost { cost: 0, .. }
    ));
}

/// R2 (B21, step 7.7): a bonus-stat name must be usable, after the defence
/// check and before the effect check.
#[test]
fn a_non_usable_bonus_name_is_missing_between_defense_and_effect() {
    let mut fixture = typed_fixture();
    edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = Some(vec!["flag".to_owned()]);
        ability.effect = Some(Ulid::from_u128(999));
    });
    assert_eq!(
        start_error(&fixture),
        CombatError::MissingStat {
            stat: "flag".to_owned()
        },
        "the bonus name beats the missing effect"
    );
    edit_ability(&mut fixture, |ability| {
        ability.defense = crpg_data::DefenseWire::TargetStat {
            stat: "nope".to_owned(),
        };
    });
    assert_eq!(
        start_error(&fixture),
        CombatError::MissingStat {
            stat: "nope".to_owned()
        },
        "the defence name beats the bonus name"
    );
}

/// R2 (B21, step 8): the reserved bonus type precedes the per-modifier
/// `Stat` name check, which precedes the duplicate-identity check.
#[test]
fn effect_checks_follow_their_pinned_order() {
    let reserved = |modifiers| {
        let mut fixture = Fixture::standard();
        attach_to_ability(
            &mut fixture,
            effect_doc(141, "slf", "combat.bonus_stat", "stack_all", modifiers, 1),
        );
        edit_ability(&mut fixture, |ability| {
            ability.bonus_stats = Some(vec!["beta".to_owned()]);
        });
        fixture
    };
    let fixture = reserved(vec![modifier(142, stat_target("nope"), "add", 1)]);
    assert_eq!(
        start_error(&fixture),
        CombatError::InvalidEffect {
            effect: Ulid::from_u128(141)
        },
        "the reserved type beats the Stat name"
    );

    let mut fixture = Fixture::standard();
    attach_to_ability(
        &mut fixture,
        effect_doc(
            141,
            "slf",
            "probe-type",
            "stack_all",
            vec![
                modifier(142, json!({"type": "roll"}), "add", 1),
                modifier(142, stat_target("nope"), "add", 1),
            ],
            1,
        ),
    );
    assert_eq!(
        start_error(&fixture),
        CombatError::MissingStat {
            stat: "nope".to_owned()
        },
        "the Stat name beats the duplicate identity"
    );
}
