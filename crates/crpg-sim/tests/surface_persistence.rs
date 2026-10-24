//! T137a persistence and load coherence (R7): the new fields round-trip in
//! their absence-form bytes, legacy worlds stay byte-identical, and each new
//! load message is raised by one raw-JSON mutation of a valid save.

#[allow(dead_code)]
mod support;

use crpg_data::DefenseWire;
use crpg_sim::World;
use serde_json::{json, Value};
use support::surface::{
    act, add_derived, always_success, attach_to_ability, binary, edit_ability, effect_doc, end,
    lit, modifier, start, stat, stat_target,
};
use support::{uid, Fixture};

/// The save of a world carrying a derived stat, bonus stats, a `Stat`
/// effect target and, after one hit, an attachment.
fn rich_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    add_derived(
        &mut fixture,
        "half",
        binary("divide", binary("add", stat("alpha"), stat("beta")), lit(2)),
    );
    always_success(&mut fixture);
    attach_to_ability(
        &mut fixture,
        effect_doc(
            141,
            "target",
            "probe-type",
            "stack_all",
            vec![modifier(142, stat_target("gamma"), "add", 2)],
            2,
        ),
    );
    edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = Some(vec!["half".to_owned()]);
        ability.defense = DefenseWire::Constant(12);
    });
    fixture
}

fn save(world: &World) -> Value {
    serde_json::to_value(world).expect("the world serializes")
}

fn load(value: Value) -> Result<World, String> {
    serde_json::from_value::<World>(value).map_err(|error| error.to_string())
}

fn rich_save() -> Value {
    let fixture = rich_fixture();
    let (world, _) = start(&fixture, 6);
    save(&world)
}

/// Loads a mutation of the rich save and expects exactly `message`.
fn assert_refused(edit: impl FnOnce(&mut Value), message: &str) {
    let mut value = rich_save();
    edit(&mut value);
    let error = load(value).expect_err("the mutated save is refused");
    assert!(
        error.contains(message),
        "expected {message:?}, found {error:?}"
    );
}

fn definition(value: &mut Value) -> &mut Value {
    &mut value["combat"]["definition"]
}

/// R7 (B22): every new T137a field round-trips through its exact bytes.
#[test]
fn the_new_fields_round_trip_in_their_absence_forms() {
    let fixture = rich_fixture();
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 6);
    act(&mut world, ids[1], ability, ids[0]);
    let value = save(&world);
    let combat = &value["combat"]["definition"];
    assert_eq!(
        combat["derived"],
        json!([{"name": "half", "formula": {"type": "divide", "value": {
            "left": {"type": "add", "value": {
                "left": {"type": "stat", "value": "alpha"},
                "right": {"type": "stat", "value": "beta"}}},
            "right": {"type": "literal", "value": 2}}}}])
    );
    assert_eq!(combat["abilities"][0]["bonus_stats"], json!(["half"]));
    assert_eq!(
        combat["abilities"][0]["defense"],
        json!({"type": "constant", "value": 12})
    );
    assert_eq!(
        combat["effects"][0]["modifiers"][0]["target"],
        json!({"type": "stat", "value": {"stat": "gamma"}})
    );
    assert!(combat.get("initiative").is_none());
    assert!(combat["abilities"][0].get("damage_dice").is_none());
    let text = serde_json::to_string(&world).expect("serializes");
    let loaded: World = serde_json::from_str(&text).expect("the save loads");
    assert_eq!(loaded, world);
    assert_eq!(serde_json::to_string(&loaded).expect("serializes"), text);
}

/// R7 (B2.9): an `ActorStat` defence round-trips in its bytes.
#[test]
fn an_actor_stat_defense_round_trips() {
    let mut fixture = Fixture::standard();
    edit_ability(&mut fixture, |ability| {
        ability.defense = DefenseWire::ActorStat {
            stat: "alpha".to_owned(),
        };
    });
    let (world, _) = start(&fixture, 6);
    let value = save(&world);
    assert_eq!(
        value["combat"]["definition"]["abilities"][0]["defense"],
        json!({"type": "actor_stat", "value": {"stat": "alpha"}})
    );
    assert_eq!(load(value).expect("the save loads"), world);
}

/// R7 (B22): a legacy world carries none of the new keys and keeps its
/// 15-field definition.
#[test]
fn a_legacy_world_writes_no_new_key() {
    let (world, _) = start(&Fixture::standard(), 6);
    let value = save(&world);
    let keys = support::rt::json_keys(&value);
    for key in ["derived", "bonus_stats", "damage_dice", "abilities"] {
        assert!(!keys.contains(key), "{key}");
    }
    assert!(value["combat"]["definition"].get("initiative").is_none());
    assert_eq!(
        value["combat"]["definition"]
            .as_object()
            .expect("an object")
            .len(),
        15
    );
    assert_eq!(load(value).expect("the save loads"), world);
}

/// R7 (B22): an initiative rule alone forces the vector shape in its
/// absence-form bytes, and save then load is equal. (The legacy-shape
/// predicates for damage dice are pinned by the crate-private table in
/// `src/combat.rs`.)
#[test]
fn an_initiative_rule_forces_the_vector_shape() {
    let (world, _) = start(&Fixture::standard(), 6);
    let mut value = save(&world);
    definition(&mut value)["initiative"] = json!({"dice": "1d20", "bonus_stats": ["alpha"]});
    let world = load(value).expect("a legacy save with a rule loads");
    let value = save(&world);
    let object = value["combat"]["definition"]
        .as_object()
        .expect("an object");
    assert_eq!(
        object["initiative"],
        json!({"dice": "1d20", "bonus_stats": ["alpha"]})
    );
    assert_eq!(object.len(), 19);
    assert!(object.contains_key("abilities"));
    assert_eq!(load(value).expect("the save loads"), world);
}

/// R7 (B22): damage dice round-trip in their absence-form bytes.
#[test]
fn damage_dice_round_trip_in_their_absence_form() {
    let mut value = rich_save();
    definition(&mut value)["abilities"][0]["damage_dice"] = json!([[{"type": "success"}, "1d8"]]);
    let world = load(value).expect("the save loads");
    let value = save(&world);
    assert_eq!(
        value["combat"]["definition"]["abilities"][0]["damage_dice"],
        json!([[{"type": "success"}, "1d8"]])
    );
    assert_eq!(load(value).expect("the save loads"), world);
}

/// R7 (B22): bonus stats and a derived stat each force the vector shape.
#[test]
fn bonus_stats_or_a_derived_stat_force_the_vector_shape() {
    let mut fixture = Fixture::standard();
    edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = Some(vec!["beta".to_owned()]);
    });
    let (world, _) = start(&fixture, 6);
    let value = save(&world);
    assert_eq!(
        value["combat"]["definition"]
            .as_object()
            .expect("an object")
            .len(),
        18
    );
    assert_eq!(load(value).expect("loads"), world);

    let mut fixture = Fixture::standard();
    add_derived(
        &mut fixture,
        "half",
        binary("divide", stat("alpha"), lit(2)),
    );
    let (world, _) = start(&fixture, 6);
    let value = save(&world);
    assert_eq!(
        value["combat"]["definition"]
            .as_object()
            .expect("an object")
            .len(),
        19
    );
    assert_eq!(load(value).expect("loads"), world);
}

/// R7 (B23 L1): a misdeclared derived name is refused.
#[test]
fn a_misdeclared_derived_name_is_refused() {
    let message = "combat derived stat is misdeclared";
    for name in ["", "health", "alpha"] {
        assert_refused(
            |value| definition(value)["derived"][0]["name"] = json!(name),
            message,
        );
    }
    assert_refused(
        |value| {
            let first = definition(value)["derived"][0].clone();
            definition(value)["derived"]
                .as_array_mut()
                .expect("an array")
                .push(first);
        },
        message,
    );
}

/// R7 (B23 L2): a formula the kernel refuses is refused at load.
#[test]
fn a_formula_the_kernel_refuses_is_refused() {
    let message = "combat derived stats fail their kernel validation";
    assert_refused(
        |value| {
            definition(value)["derived"][0]["formula"] = json!({"type": "stat", "value": "nope"});
        },
        message,
    );
    assert_refused(
        |value| {
            definition(value)["derived"][0]["formula"] = json!({"type": "stat", "value": "half"});
        },
        message,
    );
}

/// R7 (B23 L3): a bonus name outside the usable names is refused.
#[test]
fn a_bonus_name_outside_the_usable_names_is_refused() {
    let message = "combat bonus stat names no declared int stat";
    for name in ["nope", "health"] {
        assert_refused(
            |value| definition(value)["abilities"][0]["bonus_stats"] = json!([name]),
            message,
        );
    }
    assert_refused(
        |value| {
            definition(value)["initiative"] = json!({"dice": "1d6", "bonus_stats": ["nope"]});
        },
        message,
    );
}

/// R7 (B23 L4): a `Stat` target outside the usable names is refused.
#[test]
fn a_stat_target_outside_the_usable_names_is_refused() {
    for name in ["nope", "health"] {
        assert_refused(
            |value| {
                definition(value)["effects"][0]["modifiers"][0]["target"] =
                    json!({"type": "stat", "value": {"stat": name}});
            },
            "combat effect stat target names no declared int stat",
        );
    }
}

/// R7 (B23 L5): the reserved bonus modifier type is refused beside bonus
/// stats.
#[test]
fn the_reserved_type_is_refused_beside_bonus_stats() {
    assert_refused(
        |value| definition(value)["effects"][0]["mod_type"] = json!("combat.bonus_stat"),
        "combat effect uses the reserved bonus modifier type",
    );
}

/// R7 (B23): the relaxed checks test the usable names, and a legacy save
/// meets each exactly as before.
#[test]
fn the_relaxed_checks_use_the_usable_names() {
    assert_refused(
        |value| definition(value)["attribute"] = json!("nope"),
        "combat definition names an undeclared checked attribute",
    );
    assert_refused(
        |value| {
            let mut second = definition(value)["abilities"][0].clone();
            second["ability"] = json!(uid(200));
            second["attribute"] = json!("nope");
            definition(value)["abilities"]
                .as_array_mut()
                .expect("an array")
                .push(second);
        },
        "combat ability names an undeclared checked attribute",
    );
    assert_refused(
        |value| {
            definition(value)["abilities"][0]["defense"] =
                json!({"type": "actor_stat", "value": {"stat": "health"}});
        },
        "combat ability names an undeclared defense stat",
    );
    assert_refused(
        |value| {
            definition(value)["abilities"][0]["defense"] =
                json!({"type": "target_stat", "value": {"stat": "nope"}});
        },
        "combat ability names an undeclared defense stat",
    );
    // Both lists empty is refused; a derived stat alone passes that check.
    assert_refused(
        |value| {
            definition(value)["attribute_names"] = json!([]);
            definition(value)["derived"] = json!([]);
        },
        "combat definition declares no attributes",
    );
    assert_refused(
        |value| {
            definition(value)["attribute_names"] = json!([]);
        },
        "names an undeclared checked attribute",
    );
    // A derived name is a legal checked attribute and a legal defence.
    let mut value = rich_save();
    definition(&mut value)["attribute"] = json!("half");
    definition(&mut value)["abilities"][0]["attribute"] = json!("half");
    definition(&mut value)["abilities"][0]["defense"] =
        json!({"type": "actor_stat", "value": {"stat": "half"}});
    load(value).expect("a derived name is usable");

    let (world, _) = start(&Fixture::standard(), 6);
    let mut legacy = save(&world);
    legacy["combat"]["definition"]["attribute_names"] = json!([]);
    let error = load(legacy).expect_err("refused");
    assert!(error.contains("combat definition declares no attributes"));
}

/// R7 (B22): a hit after reload continues exactly as the unsaved world.
#[test]
fn a_reloaded_world_continues_equal() {
    let fixture = rich_fixture();
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, 6);
    act(&mut world, ids[1], ability, ids[0]);
    let mut reloaded = load(save(&world)).expect("the save loads");
    end(&mut world, ids[0]);
    end(&mut reloaded, ids[0]);
    let left = act(&mut world, ids[1], ability, ids[0]);
    let right = act(&mut reloaded, ids[1], ability, ids[0]);
    assert_eq!(left, right);
    assert_eq!(world, reloaded);
}

// ---------------------------------------------------------------------------
// T137b (R13): every shape at once, the T137b load messages, and replay.
// ---------------------------------------------------------------------------

/// A fixture using every authored shape: a derived stat, an `Opposed`
/// defence with a bonus, ability bonus stats, damage dice, a `Stat`-target
/// effect that grants a tag, rolled initiative and creature tags.
fn every_shape_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    add_derived(
        &mut fixture,
        "half",
        binary("divide", binary("add", stat("alpha"), stat("beta")), lit(2)),
    );
    always_success(&mut fixture);
    let (a, b) = (fixture.creature_a, fixture.creature_b);
    support::draws::set_tags(&mut fixture, a, &["pc", "hero"]);
    support::draws::set_tags(&mut fixture, b, &["foe"]);
    support::draws::set_initiative_rule(&mut fixture, "1d20", &["half"]);
    support::draws::defend_opposed(&mut fixture, "1d6", &["half"]);
    support::draws::set_damage(
        &mut fixture,
        json!([{"outcome": {"type": "success"}, "amount": 1, "dice": "1d3"}]),
    );
    edit_ability(&mut fixture, |ability| {
        ability.bonus_stats = Some(vec!["beta".to_owned()]);
    });
    attach_to_ability(
        &mut fixture,
        support::draws::grant(
            effect_doc(
                141,
                "target",
                "probe-type",
                "stack_all",
                vec![
                    modifier(142, json!({"type": "roll"}), "add", 1),
                    modifier(143, stat_target("gamma"), "add", 1),
                ],
                2,
            ),
            &["hasted"],
        ),
    );
    fixture
}

/// The state hash after the start and after each step of a fixed script in
/// which the active combatant hits the other, optionally saving and
/// reloading the world after `reload_after` steps.
fn run_script(seed: u64, steps: usize, reload_after: Option<usize>) -> Vec<[u8; 32]> {
    let fixture = every_shape_fixture();
    let ability = fixture.ability;
    let (mut world, ids) = start(&fixture, seed);
    let mut hashes = vec![crpg_sim::state_hash(&world)];
    for step in 0..steps {
        let Some(actor) = world.combat().expect("encounter").active else {
            break;
        };
        let target = if actor == ids[0] { ids[1] } else { ids[0] };
        act(&mut world, actor, ability, target);
        hashes.push(crpg_sim::state_hash(&world));
        if reload_after == Some(step + 1) {
            let text = serde_json::to_string(&world).expect("serializes");
            world = serde_json::from_str(&text).expect("the save loads");
            assert_eq!(crpg_sim::state_hash(&world), *hashes.last().expect("hash"));
        }
    }
    hashes
}

/// R13 (B1, B21): one start with every shape at once succeeds, and its save
/// carries each shape in its absence-form bytes.
#[test]
fn one_start_with_every_shape_succeeds() {
    let fixture = every_shape_fixture();
    let (world, ids) = start(&fixture, 6);
    let value = save(&world);
    let combat = &value["combat"]["definition"];
    assert_eq!(
        combat["abilities"][0]["defense"],
        json!({"type": "opposed", "value": {"dice": "1d6", "bonus_stats": ["half"]}})
    );
    assert_eq!(
        combat["abilities"][0]["damage_dice"],
        json!([[{"type": "success"}, "1d3"]])
    );
    assert_eq!(combat["abilities"][0]["bonus_stats"], json!(["beta"]));
    assert_eq!(
        combat["initiative"],
        json!({"dice": "1d20", "bonus_stats": ["half"]})
    );
    assert_eq!(combat["derived"][0]["name"], json!("half"));
    assert_eq!(combat["effects"][0]["grants"], json!(["hasted"]));
    let a = world.combatants().get(ids[0]).expect("A");
    assert_eq!(a.tags(), ["hero", "pc"]);
    let mut expected = support::draws::oracle(6);
    let key_a = support::draws::draw(&mut expected, "1d20").total + 7 + 10;
    assert_eq!(a.initiative(), key_a, "half of (8 + 6) is 7, authored 10");
    assert_eq!(load(value).expect("the save loads"), world);
}

/// R13 (B23 L3): an `Opposed` bonus name outside the usable names is refused.
#[test]
fn an_opposed_bonus_name_outside_the_usable_names_is_refused() {
    let fixture = every_shape_fixture();
    let (world, _) = start(&fixture, 6);
    for name in ["nope", "health"] {
        let mut value = save(&world);
        definition(&mut value)["abilities"][0]["defense"] =
            json!({"type": "opposed", "value": {"dice": "1d6", "bonus_stats": [name]}});
        let error = load(value).expect_err("refused");
        assert!(
            error.contains("combat bonus stat names no declared int stat"),
            "{error}"
        );
    }
    // A derived name is usable there.
    let mut value = save(&world);
    definition(&mut value)["abilities"][0]["defense"] =
        json!({"type": "opposed", "value": {"dice": "1d6", "bonus_stats": ["half", "beta"]}});
    load(value).expect("usable names load");
}

/// R13 (B23 L6): a combatant whose tags and the definition's grants exceed
/// the kernel bound is refused, and the bound itself loads.
#[test]
fn a_combatant_over_the_tag_bound_is_refused() {
    let fixture = every_shape_fixture();
    let (world, _) = start(&fixture, 6);
    let names = |count: usize| -> Vec<String> {
        (0..count).map(|index| format!("tag-{index:04}")).collect()
    };
    let mut at_bound = save(&world);
    at_bound["combatants"][0][1]["tags"] = json!(names(1023));
    load(at_bound).expect("1023 tags and one grant load");
    let mut over = save(&world);
    over["combatants"][0][1]["tags"] = json!(names(1024));
    let error = load(over).expect_err("1025 distinct tags are refused");
    assert!(
        error.contains("combatant tags exceed their bound"),
        "{error}"
    );
    // A grant already among the tags counts once.
    let mut repeat = save(&world);
    let mut tags = names(1023);
    tags.push("hasted".to_owned());
    repeat["combatants"][0][1]["tags"] = json!(tags);
    load(repeat).expect("a repeated grant counts once");
}

/// R13 (B24): two runs of one same-seed script give equal per-step hashes.
#[test]
fn two_same_seed_runs_give_equal_hashes() {
    let first = run_script(9, 4, None);
    let second = run_script(9, 4, None);
    assert!(first.len() >= 3, "the script ran");
    assert_eq!(first, second);
    assert_ne!(first, run_script(10, 4, None), "the seed matters");
}

/// R13 (B24): a run that saves and reloads mid-script continues to the same
/// hashes.
#[test]
fn a_save_and_reload_mid_script_continues_equal() {
    let straight = run_script(9, 4, None);
    for at in 0..=3 {
        assert_eq!(run_script(9, 4, Some(at)), straight, "reload after {at}");
    }
}
