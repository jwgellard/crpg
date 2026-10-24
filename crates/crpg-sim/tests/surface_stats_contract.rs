//! Two-keys contract tests for the T137a surface-stats Behaviour items,
//! written from `tasks/T137.md` alone, through the public `crpg-sim` API.
//!
//! Coverage: B2, B3, B4, B5, B6, B7, B8, B9, B10, B11, B12, B14, B15, B18,
//! B21, B22, B23 L1–L5, and D6.

mod support;

use crpg_core::{EntityId, Ulid};
use crpg_data::{Ability, DefenseWire};
use crpg_sim::{
    legal_actions, perform_action, start_encounter, validate_action, CombatAction, CombatError,
    EncounterSpec, IntFormula, World, COMBAT_ROLL_TAG,
};
use serde_json::{json, Value};
use support::surface;
use support::Fixture;

/// Serialized bytes of a world, for "unchanged" assertions.
fn snapshot(world: &World) -> Vec<u8> {
    serde_json::to_vec(world).expect("the world serializes")
}

/// Runs `start_encounter` on a fresh world and reports the result plus the
/// before/after snapshots.
fn try_start(fixture: &Fixture, seed: u64) -> (Result<(), CombatError>, Vec<u8>, Vec<u8>) {
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
    let mut world = World::new(seed);
    let before = snapshot(&world);
    let result = start_encounter(&mut world, &spec);
    let after = snapshot(&world);
    (result, before, after)
}

/// Declares a derived `Int` stat that divides `alpha` by `beta - beta`, i.e.
/// a zero divisor, so its evaluation fails (B5 at start, B15 mid-fight).
fn add_zero_divisor(fixture: &mut Fixture, name: &str) {
    surface::add_derived(
        fixture,
        name,
        surface::binary(
            "divide",
            surface::stat("alpha"),
            surface::binary("subtract", surface::stat("beta"), surface::stat("beta")),
        ),
    );
}

/// Adds a self-buff ability (id 901) that always succeeds, deals no damage,
/// costs one primary action, and applies a single-modifier effect (aim `slf`
/// or `target`) to its aim. Returns the new ability's id.
#[allow(clippy::too_many_arguments)]
fn add_buff(
    fixture: &mut Fixture,
    effect_id: u128,
    modifier_id: u128,
    aim: &str,
    target: Value,
    op: &str,
    value: i32,
    ends_turn: bool,
) -> Ulid {
    let effect = surface::effect_doc(
        effect_id,
        aim,
        "probe-buff",
        "stack_all",
        vec![surface::modifier(modifier_id, target, op, value)],
        2,
    );
    let buff_id = Ulid::from_u128(901);
    let buff: Ability = serde_json::from_value(json!({
        "id": support::uid(901),
        "slug": "probe-buff",
        "name": "probe.buff",
        "dice": "1d6",
        "attribute": "alpha",
        "outcome_table": support::uid(104),
        "damage": [{"outcome": {"type": "success"}, "amount": 0}],
        "cost": 1,
        "extra_costs": [],
        "ends_turn": ends_turn,
        "effect": support::uid(effect_id),
        "defense": {"type": "actor_attribute"},
        "requires_target": true,
        "allow_self_target": true
    }))
    .expect("the buff ability is valid");
    fixture.effects.insert(effect.id, effect);
    fixture.abilities.insert(buff_id, buff);
    fixture.ruleset.abilities.push(buff_id);
    buff_id
}

/// One `UseAbility` action.
fn use_action(actor: EntityId, ability: Ulid, target: EntityId) -> CombatAction {
    CombatAction::UseAbility {
        actor,
        ability,
        target,
    }
}

// ---------------------------------------------------------------------------
// B5, B21 — derived preflight and start order/transactionality
// ---------------------------------------------------------------------------

#[test]
fn b5_zero_divisor_preflight_fails_naming_the_derived_stat() {
    let mut fixture = Fixture::standard();
    add_zero_divisor(&mut fixture, "boom");
    let (result, before, after) = try_start(&fixture, 5);
    assert_eq!(
        result,
        Err(CombatError::InvalidStatValue {
            stat: "boom".to_owned()
        })
    );
    assert_eq!(before, after, "a rejected start leaves the world unchanged");
    assert_eq!(
        format!(
            "{}",
            CombatError::InvalidStatValue {
                stat: "boom".to_owned()
            }
        ),
        "InvalidStatValue at spec/stats"
    );
}

#[test]
fn b21_b2_errors_beat_the_derived_preflight() {
    let mut fixture = Fixture::standard();
    add_zero_divisor(&mut fixture, "boom");
    let creature_a = fixture.creature_a;
    surface::remove_stat(&mut fixture, creature_a, "beta");
    let (result, before, after) = try_start(&fixture, 5);
    assert_eq!(
        result,
        Err(CombatError::MissingStat {
            stat: "beta".to_owned()
        })
    );
    assert_eq!(before, after, "a rejected start leaves the world unchanged");
}

#[test]
fn b21_the_derived_preflight_beats_ability_checks() {
    let mut fixture = Fixture::standard();
    add_zero_divisor(&mut fixture, "boom");
    surface::edit_ability(&mut fixture, |ability| ability.dice = "bogus".to_owned());
    let (result, before, after) = try_start(&fixture, 5);
    assert_eq!(
        result,
        Err(CombatError::InvalidStatValue {
            stat: "boom".to_owned()
        })
    );
    assert_eq!(before, after, "a rejected start leaves the world unchanged");
}

#[test]
fn b21_the_derived_preflight_beats_effect_checks() {
    let mut fixture = Fixture::standard();
    add_zero_divisor(&mut fixture, "boom");
    let effect = surface::effect_doc(999, "slf", "probe-type", "stack_all", vec![], 0);
    surface::attach_to_ability(&mut fixture, effect);
    let (result, before, after) = try_start(&fixture, 5);
    assert_eq!(
        result,
        Err(CombatError::InvalidStatValue {
            stat: "boom".to_owned()
        })
    );
    assert_eq!(before, after, "a rejected start leaves the world unchanged");
}

// ---------------------------------------------------------------------------
// D6, B7 — the world interner is never written by stat evaluation
// ---------------------------------------------------------------------------

#[test]
fn d6_stat_evaluation_never_writes_the_world_interner() {
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    surface::add_derived(
        &mut fixture,
        "power",
        surface::binary("add", surface::stat("alpha"), surface::stat("beta")),
    );
    surface::edit_ability(&mut fixture, |ability| {
        ability.defense = DefenseWire::TargetStat {
            stat: "power".to_owned(),
        };
        for entry in &mut ability.damage {
            entry.amount = 0;
        }
    });
    let (mut world, ids) = surface::start(&fixture, 5);
    let roll_tag = COMBAT_ROLL_TAG.to_string();
    let before = serde_json::to_value(&world).expect("serializes");
    assert_eq!(before["interners"]["tags"], json!([roll_tag]));
    assert_eq!(before["interners"]["stats"], json!([]));

    // validate_action runs the B15 stage on a scratch interner and mutates
    // nothing: the derived base names never reach the world interner.
    let action = use_action(ids[1], fixture.ability, ids[0]);
    assert!(validate_action(&world, &action).is_ok());
    assert_eq!(
        serde_json::to_value(&world).expect("serializes")["interners"],
        before["interners"]
    );

    // The one world-interner write on the action path is B10's TargetStat.
    perform_action(&mut world, &action)
        .expect("accepted")
        .expect("attack");
    let after = serde_json::to_value(&world).expect("serializes");
    assert_eq!(after["interners"]["tags"], json!([roll_tag]));
    assert_eq!(after["interners"]["stats"], json!(["power"]));
}

// ---------------------------------------------------------------------------
// B2 — reading stats at start
// ---------------------------------------------------------------------------

#[test]
fn b2_base_int_stat_presence_and_wholeness() {
    let mut fixture = Fixture::standard();
    let creature_a = fixture.creature_a;
    surface::remove_stat(&mut fixture, creature_a, "beta");
    assert_eq!(
        surface::try_start(&fixture, 5),
        Err(CombatError::MissingStat {
            stat: "beta".to_owned()
        })
    );

    let mut fixture = Fixture::standard();
    let creature_b = fixture.creature_b;
    surface::set_stat_raw(&mut fixture, creature_b, "alpha", 1);
    assert_eq!(
        surface::try_start(&fixture, 5),
        Err(CombatError::InvalidStatValue {
            stat: "alpha".to_owned()
        })
    );
}

#[test]
fn b2_non_derived_fixed_presence_but_not_wholeness() {
    let mut fixture = Fixture::standard();
    surface::add_stat(&mut fixture, json!({"name": "weight", "kind": "fixed"}));
    // Present on A but missing on B: presence is still required.
    let creature_a = fixture.creature_a;
    surface::set_stat_raw(&mut fixture, creature_a, "weight", 1);
    assert_eq!(
        surface::try_start(&fixture, 5),
        Err(CombatError::MissingStat {
            stat: "weight".to_owned()
        })
    );

    // Both present, even fractional: the value is never read.
    let creature_b = fixture.creature_b;
    surface::set_stat_raw(&mut fixture, creature_b, "weight", 1);
    let (world, _) = surface::start(&fixture, 5);
    assert!(world.combat().is_some());
}

#[test]
fn b2_derived_and_non_int_stats_are_not_read() {
    let mut fixture = Fixture::standard();
    // A non-derived Bool stat is declared but absent from both creatures.
    surface::add_stat(&mut fixture, json!({"name": "flag", "kind": "bool"}));
    // A derived Int stat is declared but absent from `Creature.stats`.
    surface::add_derived(
        &mut fixture,
        "power",
        surface::binary("add", surface::stat("alpha"), surface::stat("beta")),
    );
    let (world, _) = surface::start(&fixture, 5);
    assert!(world.combat().is_some());
}

// ---------------------------------------------------------------------------
// B3 — attribute names hold base usable stats
// ---------------------------------------------------------------------------

#[test]
fn b3_attribute_names_hold_base_usable_stats() {
    let mut fixture = Fixture::standard();
    surface::add_derived(
        &mut fixture,
        "power",
        surface::binary("add", surface::stat("alpha"), surface::stat("beta")),
    );
    surface::add_attribute(&mut fixture, "power");
    let (world, _) = surface::start(&fixture, 5);
    let definition = &world.combat().expect("combat").definition;
    assert_eq!(
        definition.attribute_names,
        vec!["alpha".to_owned(), "beta".to_owned(), "gamma".to_owned()]
    );
    assert_eq!(definition.derived.len(), 1);
    assert_eq!(definition.derived[0].name, "power");
}

// ---------------------------------------------------------------------------
// B4 — derived carriage
// ---------------------------------------------------------------------------

#[test]
fn b4_derived_int_stats_are_carried_node_for_node() {
    let mut fixture = Fixture::standard();
    surface::add_derived(
        &mut fixture,
        "power",
        surface::binary(
            "divide",
            surface::binary("add", surface::stat("alpha"), surface::stat("beta")),
            surface::lit(2),
        ),
    );
    // A derived non-Int stat is not carried.
    surface::add_stat(
        &mut fixture,
        json!({
            "name": "fweight",
            "kind": "fixed",
            "derived": {"type": "literal", "value": {"type": "fixed", "value": 1}}
        }),
    );
    let (world, _) = surface::start(&fixture, 5);
    let derived = &world.combat().expect("combat").definition.derived;
    assert_eq!(derived.len(), 1, "only the Int derived stat is carried");
    assert_eq!(derived[0].name, "power");
    match &derived[0].formula {
        IntFormula::Divide { left, right } => {
            match left.as_ref() {
                IntFormula::Add { left: l, right: r } => {
                    assert_eq!(l.as_ref(), &IntFormula::Stat("alpha".to_owned()));
                    assert_eq!(r.as_ref(), &IntFormula::Stat("beta".to_owned()));
                }
                other => panic!("expected Add, got {other:?}"),
            }
            assert_eq!(right.as_ref(), &IntFormula::Literal(2));
        }
        other => panic!("expected Divide, got {other:?}"),
    }
    // The symbolic mirror persists and round-trips byte-identically.
    let bytes = snapshot(&world);
    let loaded: World = serde_json::from_slice(&bytes).expect("loads");
    assert_eq!(snapshot(&loaded), bytes);
}

// ---------------------------------------------------------------------------
// B22 — legacy shape: the standard fixture stays the 15-field definition
// ---------------------------------------------------------------------------

#[test]
fn b22_legacy_definition_writes_fifteen_fields() {
    let fixture = Fixture::standard();
    let (world, _) = surface::start(&fixture, 5);
    let value = serde_json::to_value(&world).expect("serializes");
    let definition = value["combat"]["definition"].as_object().expect("object");
    // The legacy single-ability shape has no vector keys and no new field.
    assert_eq!(definition.len(), 15, "legacy definition writes 15 fields");
    for absent in ["derived", "initiative", "abilities", "pools", "effects"] {
        assert!(
            !definition.contains_key(absent),
            "legacy has no {absent} key"
        );
    }
}

#[test]
fn b22_derived_or_bonus_stats_round_trip() {
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    surface::add_derived(
        &mut fixture,
        "power",
        surface::binary("add", surface::stat("alpha"), surface::stat("beta")),
    );
    surface::edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = Some(vec!["beta".to_owned()]);
        ability.defense = DefenseWire::TargetStat {
            stat: "power".to_owned(),
        };
    });
    let (world, _) = surface::start(&fixture, 5);
    let value = serde_json::to_value(&world).expect("serializes");
    let definition = value["combat"]["definition"].as_object().expect("object");
    assert!(definition.contains_key("derived"));
    assert!(definition.contains_key("abilities"));
    // Save then load gives an equal world (symbolic mirror persists).
    let bytes = snapshot(&world);
    let loaded: World = serde_json::from_slice(&bytes).expect("loads");
    assert_eq!(snapshot(&loaded), bytes);
}

// ---------------------------------------------------------------------------
// B23 L1–L5 — load coherence
// ---------------------------------------------------------------------------

/// Asserts one surgically corrupted save fails to load with `expected`
/// somewhere in the load error message.
fn rejects_with(
    base: &serde_json::Value,
    expected: &str,
    mutate: impl FnOnce(&mut serde_json::Value),
) {
    let mut save = base.clone();
    mutate(&mut save);
    let err = serde_json::from_value::<World>(save).expect_err("must fail to load");
    assert!(
        err.to_string().contains(expected),
        "expected load error to contain {expected:?}, got {err}"
    );
}

/// A valid save with one derived `Int` stat (`power = alpha + beta`).
fn derived_save() -> serde_json::Value {
    let mut fixture = Fixture::standard();
    surface::add_derived(
        &mut fixture,
        "power",
        surface::binary("add", surface::stat("alpha"), surface::stat("beta")),
    );
    let (world, _) = surface::start(&fixture, 5);
    serde_json::to_value(&world).expect("serializes")
}

#[test]
fn b23_l1_a_misdeclared_derived_name_is_refused() {
    let base = derived_save();
    // Empty name.
    rejects_with(&base, "combat derived stat is misdeclared", |value| {
        value["combat"]["definition"]["derived"][0]["name"] = json!("");
    });
    // A derived name equal to the health stat.
    rejects_with(&base, "combat derived stat is misdeclared", |value| {
        value["combat"]["definition"]["derived"][0]["name"] = json!("health");
    });
    // A derived name repeated as an attribute.
    rejects_with(&base, "combat derived stat is misdeclared", |value| {
        value["combat"]["definition"]["derived"][0]["name"] = json!("alpha");
    });
}

#[test]
fn b23_l2_an_invalid_derived_formula_is_refused() {
    let base = derived_save();
    rejects_with(
        &base,
        "combat derived stats fail their kernel validation",
        |value| {
            value["combat"]["definition"]["derived"][0]["formula"] =
                json!({"type": "stat", "value": "nope"});
        },
    );
}

#[test]
fn b23_l3_an_undeclared_bonus_stat_is_refused() {
    let mut fixture = Fixture::standard();
    surface::edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = Some(vec!["beta".to_owned()]);
    });
    let (world, _) = surface::start(&fixture, 5);
    let base = serde_json::to_value(&world).expect("serializes");
    rejects_with(
        &base,
        "combat bonus stat names no declared int stat",
        |value| {
            value["combat"]["definition"]["abilities"][0]["bonus_stats"] = json!(["nope"]);
        },
    );
}

#[test]
fn b23_l4_an_undeclared_effect_stat_target_is_refused() {
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    let effect = surface::effect_doc(
        999,
        "slf",
        "probe-type",
        "stack_all",
        vec![surface::modifier(
            700,
            surface::stat_target("alpha"),
            "add",
            4,
        )],
        2,
    );
    surface::attach_to_ability(&mut fixture, effect);
    let (world, _) = surface::start(&fixture, 5);
    let base = serde_json::to_value(&world).expect("serializes");
    rejects_with(
        &base,
        "combat effect stat target names no declared int stat",
        |value| {
            value["combat"]["definition"]["effects"][0]["modifiers"][0]["target"]["value"]
                ["stat"] = json!("nope");
        },
    );
}

#[test]
fn b23_l5_the_reserved_bonus_type_is_refused_beside_bonus_stats() {
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    surface::edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = Some(vec!["beta".to_owned()]);
    });
    let effect = surface::effect_doc(
        999,
        "slf",
        "probe-type",
        "stack_all",
        vec![surface::modifier(700, json!({"type": "roll"}), "add", 1)],
        2,
    );
    surface::attach_to_ability(&mut fixture, effect);
    let (world, _) = surface::start(&fixture, 5);
    let base = serde_json::to_value(&world).expect("serializes");
    rejects_with(
        &base,
        "combat effect uses the reserved bonus modifier type",
        |value| {
            value["combat"]["definition"]["effects"][0]["mod_type"] =
                json!(crpg_sim::COMBAT_BONUS_MOD_TYPE);
        },
    );
}

// ---------------------------------------------------------------------------
// B9, B10 — ActorAttribute and TargetStat resolutions
// ---------------------------------------------------------------------------

#[test]
fn b9_actor_attribute_reads_the_actors_stat() {
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    surface::edit_ability(&mut fixture, |ability| {
        for entry in &mut ability.damage {
            entry.amount = 0;
        }
    });
    let (mut world, ids) = surface::start(&fixture, 5);
    // B (alpha 6) attacks A: DC = the actor's own attribute.
    assert_eq!(
        world.combatants().get(ids[1]).unwrap().placement(),
        fixture.placement_b
    );
    let outcome = surface::act(&mut world, ids[1], fixture.ability, ids[0]);
    assert_eq!(outcome.margin, i64::from(outcome.roll.total) - 6);
}

#[test]
fn b10_target_stat_reads_the_targets_stat_and_interns_it() {
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    surface::edit_ability(&mut fixture, |ability| {
        ability.defense = DefenseWire::TargetStat {
            stat: "beta".to_owned(),
        };
        for entry in &mut ability.damage {
            entry.amount = 0;
        }
    });
    let (mut world, ids) = surface::start(&fixture, 5);
    // B attacks A: DC = A's beta (6).
    let outcome = surface::act(&mut world, ids[1], fixture.ability, ids[0]);
    assert_eq!(outcome.margin, i64::from(outcome.roll.total) - 6);
    // The accepted TargetStat action interned the stat into the world (B10).
    let value = serde_json::to_value(&world).expect("serializes");
    assert_eq!(value["interners"]["stats"], json!(["beta"]));
}

// ---------------------------------------------------------------------------
// B11, B12 — Constant vs ActorStat fold side
// ---------------------------------------------------------------------------

#[test]
fn b11_constant_folds_the_targets_dc_only() {
    // A buffs itself +5 Dc, then B strikes A with a Constant(50) defence: the
    // target's Dc effect folds (55), so the margin is total - 55.
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    let buff = add_buff(
        &mut fixture,
        700,
        700,
        "slf",
        json!({"type": "dc"}),
        "add",
        5,
        true,
    );
    surface::edit_ability(&mut fixture, |ability| {
        ability.defense = DefenseWire::Constant(50);
        for entry in &mut ability.damage {
            entry.amount = 0;
        }
    });
    fixture.encounter.participants[0].initiative = 5;
    fixture.encounter.participants[1].initiative = 9;
    let (mut world, ids) = surface::start(&fixture, 5);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(a, buff, a)).expect("A buffs");
    let outcome = surface::act(&mut world, b, fixture.ability, a);
    assert_eq!(outcome.margin, i64::from(outcome.roll.total) - 55);

    // B's own +5 Dc does not fold into a Constant check: margin is total - 50.
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    let buff = add_buff(
        &mut fixture,
        700,
        700,
        "slf",
        json!({"type": "dc"}),
        "add",
        5,
        true,
    );
    surface::edit_ability(&mut fixture, |ability| {
        ability.defense = DefenseWire::Constant(50);
        for entry in &mut ability.damage {
            entry.amount = 0;
        }
    });
    let (mut world, ids) = surface::start(&fixture, 5); // B acts first
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(b, buff, b)).expect("B buffs");
    perform_action(&mut world, &CombatAction::EndTurn { actor: a }).expect("A ends");
    let outcome = surface::act(&mut world, b, fixture.ability, a);
    assert_eq!(outcome.margin, i64::from(outcome.roll.total) - 50);
}

#[test]
fn b12_actor_stat_folds_the_actors_dc_only() {
    // B buffs itself +5 Dc, then strikes A with ActorStat(alpha): the actor's
    // Dc effect folds (6 + 5), so the margin is total - 11.
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    let buff = add_buff(
        &mut fixture,
        700,
        700,
        "slf",
        json!({"type": "dc"}),
        "add",
        5,
        true,
    );
    surface::edit_ability(&mut fixture, |ability| {
        ability.defense = DefenseWire::ActorStat {
            stat: "alpha".to_owned(),
        };
        for entry in &mut ability.damage {
            entry.amount = 0;
        }
    });
    let (mut world, ids) = surface::start(&fixture, 5); // B acts first
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(b, buff, b)).expect("B buffs");
    perform_action(&mut world, &CombatAction::EndTurn { actor: a }).expect("A ends");
    let outcome = surface::act(&mut world, b, fixture.ability, a);
    assert_eq!(outcome.margin, i64::from(outcome.roll.total) - 11);

    // A's +5 Dc does not fold into an ActorStat check: margin is total - 6.
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    let buff = add_buff(
        &mut fixture,
        700,
        700,
        "slf",
        json!({"type": "dc"}),
        "add",
        5,
        true,
    );
    surface::edit_ability(&mut fixture, |ability| {
        ability.defense = DefenseWire::ActorStat {
            stat: "alpha".to_owned(),
        };
        for entry in &mut ability.damage {
            entry.amount = 0;
        }
    });
    fixture.encounter.participants[0].initiative = 5;
    fixture.encounter.participants[1].initiative = 9;
    let (mut world, ids) = surface::start(&fixture, 5);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(a, buff, a)).expect("A buffs");
    let outcome = surface::act(&mut world, b, fixture.ability, a);
    assert_eq!(outcome.margin, i64::from(outcome.roll.total) - 6);
}

// ---------------------------------------------------------------------------
// B14 — bonus stats
// ---------------------------------------------------------------------------

#[test]
fn b14_bonus_stats_shift_the_margin_by_their_sum() {
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    surface::edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = Some(vec!["beta".to_owned()]);
        ability.defense = DefenseWire::Constant(50);
        for entry in &mut ability.damage {
            entry.amount = 0;
        }
    });
    let (mut world, ids) = surface::start(&fixture, 5);
    // B (beta 7) attacks A: the bonus folds into the roll.
    let outcome = surface::act(&mut world, ids[1], fixture.ability, ids[0]);
    assert_eq!(outcome.margin, i64::from(outcome.roll.total) + 7 - 50);
}

#[test]
fn b14_multiple_bonus_stats_fold_as_distinct_modifiers() {
    // Two bonus stats contribute two distinct synthetic modifiers; a shared
    // synthetic id would drop one and under-count the margin.
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    surface::edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = Some(vec!["beta".to_owned(), "gamma".to_owned()]);
        ability.defense = DefenseWire::Constant(50);
        for entry in &mut ability.damage {
            entry.amount = 0;
        }
    });
    let (mut world, ids) = surface::start(&fixture, 5);
    // B (beta 7, gamma 5) attacks A: both fold.
    let outcome = surface::act(&mut world, ids[1], fixture.ability, ids[0]);
    assert_eq!(outcome.margin, i64::from(outcome.roll.total) + 12 - 50);
}

#[test]
fn b14_synthetic_id_skips_an_authored_modifier_id() {
    // B buffs itself a Roll +5 whose modifier id is ...01, then strikes with a
    // bonus stat: the synthetic bonus takes ...02, so both fold without
    // colliding (margin = total + 5 + 7 - 50).
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    let buff = add_buff(
        &mut fixture,
        200,
        1,
        "slf",
        json!({"type": "roll"}),
        "add",
        5,
        true,
    );
    surface::edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = Some(vec!["beta".to_owned()]);
        ability.defense = DefenseWire::Constant(50);
        for entry in &mut ability.damage {
            entry.amount = 0;
        }
    });
    let (mut world, ids) = surface::start(&fixture, 5); // B acts first
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(b, buff, b)).expect("B buffs");
    perform_action(&mut world, &CombatAction::EndTurn { actor: a }).expect("A ends");
    let outcome = surface::act(&mut world, b, fixture.ability, a);
    assert_eq!(outcome.margin, i64::from(outcome.roll.total) + 5 + 7 - 50);
}

#[test]
fn b14_reserved_type_is_invalid_with_bonus_stats_and_ok_without() {
    // With bonus stats, an effect using the reserved type is InvalidEffect.
    let mut fixture = Fixture::standard();
    surface::edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = Some(vec!["beta".to_owned()]);
    });
    let effect = surface::effect_doc(
        999,
        "slf",
        crpg_sim::COMBAT_BONUS_MOD_TYPE,
        "stack_all",
        vec![surface::modifier(700, json!({"type": "roll"}), "add", 1)],
        1,
    );
    surface::attach_to_ability(&mut fixture, effect);
    assert_eq!(
        surface::try_start(&fixture, 5),
        Err(CombatError::InvalidEffect {
            effect: Ulid::from_u128(999)
        })
    );

    // Without bonus stats the reserved type starts.
    let mut fixture = Fixture::standard();
    let effect = surface::effect_doc(
        999,
        "slf",
        crpg_sim::COMBAT_BONUS_MOD_TYPE,
        "stack_all",
        vec![surface::modifier(700, json!({"type": "roll"}), "add", 1)],
        1,
    );
    surface::attach_to_ability(&mut fixture, effect);
    let (world, _) = surface::start(&fixture, 5);
    assert!(world.combat().is_some());
}

// ---------------------------------------------------------------------------
// B15 — the stat evaluation stage
// ---------------------------------------------------------------------------

#[test]
fn b15_stat_evaluation_fails_the_action_without_mutation() {
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    surface::add_derived(
        &mut fixture,
        "power",
        surface::binary("divide", surface::stat("alpha"), surface::stat("beta")),
    );
    let buff = add_buff(
        &mut fixture,
        800,
        800,
        "target",
        surface::stat_target("beta"),
        "set",
        0,
        true,
    );
    surface::edit_ability(&mut fixture, |ability| {
        ability.defense = DefenseWire::ActorStat {
            stat: "power".to_owned(),
        };
        for entry in &mut ability.damage {
            entry.amount = 0;
        }
    });
    // A acts first and debuffs B (beta -> 0); B's next action reads
    // power = alpha / 0. B's pool is untouched, so only the B15 stage fails.
    fixture.encounter.participants[0].initiative = 5;
    fixture.encounter.participants[1].initiative = 9;
    let (mut world, ids) = surface::start(&fixture, 5);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(a, buff, b)).expect("A debuffs B");
    let action = use_action(b, fixture.ability, a);
    let before = snapshot(&world);
    assert_eq!(
        validate_action(&world, &action),
        Err(CombatError::StatEvaluation {
            stat: "power".to_owned()
        })
    );
    assert_eq!(snapshot(&world), before, "validate_action mutates nothing");
    assert_eq!(
        perform_action(&mut world, &action),
        Err(CombatError::StatEvaluation {
            stat: "power".to_owned()
        })
    );
    assert_eq!(
        snapshot(&world),
        before,
        "a rejected action leaves the world unchanged"
    );
    // legal_actions omits the failing UseAbility candidate (the strike), while
    // the buff ability remains legal.
    let options = legal_actions(&world, b).expect("B may act");
    assert!(
        !options.iter().any(
            |option| matches!(option, CombatAction::UseAbility { ability, .. } if *ability == fixture.ability)
        ),
        "the failing UseAbility candidate is omitted"
    );
    assert_eq!(
        format!(
            "{}",
            CombatError::StatEvaluation {
                stat: "power".to_owned()
            }
        ),
        "StatEvaluation at combat/stats"
    );
}

#[test]
fn b15_insufficient_action_wins_over_stat_evaluation() {
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    surface::add_derived(
        &mut fixture,
        "power",
        surface::binary("divide", surface::stat("alpha"), surface::stat("beta")),
    );
    let buff = add_buff(
        &mut fixture,
        800,
        800,
        "slf",
        surface::stat_target("beta"),
        "set",
        0,
        false,
    );
    surface::edit_ability(&mut fixture, |ability| {
        ability.defense = DefenseWire::ActorStat {
            stat: "power".to_owned(),
        };
        for entry in &mut ability.damage {
            entry.amount = 0;
        }
    });
    // B buffs itself beta -> 0 without ending its turn, spending its primary
    // action: its next action is both unaffordable and would fail stat
    // evaluation, but affordability wins.
    let (mut world, ids) = surface::start(&fixture, 5); // B acts first
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(b, buff, b)).expect("B buffs");
    let action = use_action(b, fixture.ability, a);
    // Both hold (the stat would also fail), but affordability wins.
    assert!(matches!(
        validate_action(&world, &action),
        Err(CombatError::InsufficientAction { .. })
    ));
}

#[test]
fn b15_without_derived_stats_the_stage_never_runs() {
    let fixture = Fixture::standard();
    let (world, ids) = fixture.start(5);
    let action = use_action(ids[1], fixture.ability, ids[0]);
    assert!(validate_action(&world, &action).is_ok());
}

// ---------------------------------------------------------------------------
// B6, B18 — derived values track Stat Add/Set effects, and not after expiry
// ---------------------------------------------------------------------------

#[test]
fn b18_stat_add_changes_a_derived_defence_while_attached_and_not_after_expiry() {
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    surface::add_derived(
        &mut fixture,
        "power",
        surface::binary("add", surface::stat("alpha"), surface::stat("beta")),
    );
    let buff = add_buff(
        &mut fixture,
        900,
        900,
        "slf",
        surface::stat_target("alpha"),
        "add",
        4,
        true,
    );
    surface::edit_ability(&mut fixture, |ability| {
        ability.defense = DefenseWire::TargetStat {
            stat: "power".to_owned(),
        };
        for entry in &mut ability.damage {
            entry.amount = 0;
        }
    });
    // A acts first so it observes B's power (6 + 7 = 13).
    fixture.encounter.participants[0].initiative = 5;
    fixture.encounter.participants[1].initiative = 9;
    let (mut world, ids) = surface::start(&fixture, 5);
    let (a, b) = (ids[0], ids[1]);

    // Round 0: baseline power 13.
    let first = surface::act(&mut world, a, fixture.ability, b);
    assert_eq!(first.margin, i64::from(first.roll.total) - 13);

    // B buffs alpha +4 (power 17); rollover reaches round 1.
    perform_action(&mut world, &use_action(b, buff, b)).expect("B buffs");
    let second = surface::act(&mut world, a, fixture.ability, b);
    assert_eq!(second.margin, i64::from(second.roll.total) - 17);

    // B ends its round-1 turn; rollover reaches round 2, the effect expires.
    perform_action(&mut world, &CombatAction::EndTurn { actor: b }).expect("B ends");
    let third = surface::act(&mut world, a, fixture.ability, b);
    assert_eq!(third.margin, i64::from(third.roll.total) - 13);
}

#[test]
fn b18_stat_set_replaces_a_base_stat_while_attached() {
    let mut fixture = Fixture::standard();
    surface::always_success(&mut fixture);
    surface::add_derived(
        &mut fixture,
        "power",
        surface::binary("add", surface::stat("alpha"), surface::stat("beta")),
    );
    let buff = add_buff(
        &mut fixture,
        900,
        900,
        "slf",
        surface::stat_target("alpha"),
        "set",
        20,
        true,
    );
    surface::edit_ability(&mut fixture, |ability| {
        ability.defense = DefenseWire::TargetStat {
            stat: "power".to_owned(),
        };
        for entry in &mut ability.damage {
            entry.amount = 0;
        }
    });
    fixture.encounter.participants[0].initiative = 5;
    fixture.encounter.participants[1].initiative = 9;
    let (mut world, ids) = surface::start(&fixture, 5);
    let (a, b) = (ids[0], ids[1]);

    let first = surface::act(&mut world, a, fixture.ability, b);
    assert_eq!(first.margin, i64::from(first.roll.total) - 13);

    // Set alpha to 20: power = 20 + 7 = 27.
    perform_action(&mut world, &use_action(b, buff, b)).expect("B buffs");
    let second = surface::act(&mut world, a, fixture.ability, b);
    assert_eq!(second.margin, i64::from(second.roll.total) - 27);
}
