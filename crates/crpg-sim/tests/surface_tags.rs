//! T137b tags (R12): the tag carriage gate, the union bound and its error.
//!
//! Creature tags are carried into combat state only in encounters whose
//! referenced effects grant tags, so a content set without grants hashes
//! exactly as before. The projection itself (placement tags plus unexpired
//! grants) is pinned by the crate-private test in `combat.rs`.

#[allow(dead_code)]
mod support;

use crpg_data::Effect;
use crpg_sim::{state_hash, CombatError, World};
use serde_json::{json, Value};
use support::draws::{grant, set_tags, start_into};
use support::surface::{attach_to_ability, effect_doc, modifier, start};
use support::Fixture;

/// An effect on the target with one `Roll` modifier, granting `tags`.
fn granting(id: u128, tags: &[&str]) -> Effect {
    grant(
        effect_doc(
            id,
            "target",
            "probe-type",
            "stack_all",
            vec![modifier(id + 1, json!({"type": "roll"}), "add", 1)],
            2,
        ),
        tags,
    )
}

fn names(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("tag-{index:04}")).collect()
}

fn refs(values: &[String]) -> Vec<&str> {
    values.iter().map(String::as_str).collect()
}

fn save(world: &World) -> Value {
    serde_json::to_value(world).expect("the world serializes")
}

/// R12 (B19): creature tags appear only with a granting effect, sorted and
/// distinct; the grants are carried in authored order.
#[test]
fn tags_are_carried_only_with_a_granting_effect() {
    let mut fixture = Fixture::standard();
    let (a, b) = (fixture.creature_a, fixture.creature_b);
    set_tags(&mut fixture, a, &["zed", "ant", "zed", "mid"]);
    set_tags(&mut fixture, b, &["solo"]);

    // No effect at all: nothing is carried.
    let (world, ids) = start(&fixture, 1);
    assert!(world.combatants().get(ids[0]).expect("A").tags().is_empty());
    assert!(save(&world)["combatants"]
        .to_string()
        .find("tags")
        .is_none());

    // An effect without grants: nothing is carried.
    attach_to_ability(
        &mut fixture,
        effect_doc(
            141,
            "target",
            "probe-type",
            "stack_all",
            vec![modifier(142, json!({"type": "roll"}), "add", 1)],
            2,
        ),
    );
    let (world, ids) = start(&fixture, 1);
    assert!(world.combatants().get(ids[0]).expect("A").tags().is_empty());
    assert!(!save(&world).to_string().contains("grants"));

    // A granting effect: the placement tags, sorted and distinct.
    attach_to_ability(&mut fixture, granting(141, &["hasted", "alert"]));
    let (world, ids) = start(&fixture, 1);
    assert_eq!(
        world.combatants().get(ids[0]).expect("A").tags(),
        ["ant", "mid", "zed"]
    );
    assert_eq!(world.combatants().get(ids[1]).expect("B").tags(), ["solo"]);
    let value = save(&world);
    assert_eq!(
        value["combatants"].to_string().matches("\"tags\"").count(),
        2
    );
    assert_eq!(
        value["combat"]["definition"]["effects"][0]["grants"],
        json!(["hasted", "alert"]),
        "authored order"
    );
    let text = serde_json::to_string(&world).expect("serializes");
    let loaded: World = serde_json::from_str(&text).expect("the save loads");
    assert_eq!(loaded, world);
}

/// R12 (B19): only a referenced effect counts; an unreferenced granting
/// effect in the spec map carries nothing.
#[test]
fn an_unreferenced_granting_effect_carries_nothing() {
    let mut fixture = Fixture::standard();
    let a = fixture.creature_a;
    set_tags(&mut fixture, a, &["ant"]);
    let stray = granting(151, &["stray"]);
    fixture.effects.insert(stray.id, stray);
    let (world, ids) = start(&fixture, 1);
    assert!(world.combatants().get(ids[0]).expect("A").tags().is_empty());
    assert!(world
        .combat()
        .expect("encounter")
        .definition
        .effects
        .is_empty());
}

/// R12 (B20): the union of one participant's distinct tags and every grant
/// is accepted at 1024 and fails `TooManyTags` at 1025, leaving the world
/// untouched.
#[test]
fn the_tag_union_is_bounded_at_the_kernel_limit() {
    let creature_tags = names(1023);
    let mut fixture = Fixture::standard();
    let a = fixture.creature_a;
    set_tags(&mut fixture, a, &refs(&creature_tags));

    // 1023 + one new grant = 1024.
    attach_to_ability(&mut fixture, granting(141, &["fresh"]));
    let (world, ids) = start(&fixture, 1);
    assert_eq!(
        world.combatants().get(ids[0]).expect("A").tags().len(),
        1023
    );

    // 1023 + two new grants = 1025.
    attach_to_ability(&mut fixture, granting(141, &["fresh", "extra"]));
    let mut world = World::new(1);
    let before = state_hash(&world);
    assert_eq!(
        start_into(&fixture, &mut world),
        Err(CombatError::TooManyTags)
    );
    assert_eq!(state_hash(&world), before);
    assert!(world.combat().is_none() && world.is_empty());

    // Grants that repeat a creature tag count once: 1023 + 1 + 1 repeated.
    attach_to_ability(
        &mut fixture,
        granting(141, &["fresh", creature_tags[0].as_str()]),
    );
    let (world, _) = start(&fixture, 1);
    assert!(world.combat().is_some());

    // 1024 creature tags + one new grant = 1025.
    let full = names(1024);
    set_tags(&mut fixture, a, &refs(&full));
    attach_to_ability(&mut fixture, granting(141, &["fresh"]));
    let mut world = World::new(1);
    assert_eq!(
        start_into(&fixture, &mut world),
        Err(CombatError::TooManyTags)
    );

    // 1024 creature tags + a grant already among them = 1024.
    attach_to_ability(&mut fixture, granting(141, &[full[7].as_str()]));
    assert!(start_into(&fixture, &mut World::new(1)).is_ok());
}

/// R12 (B21): `TooManyTags` follows the effect checks, so a duplicate
/// modifier identity wins over it; the bound is a start check only for
/// encounters that grant.
#[test]
fn too_many_tags_comes_after_the_effect_checks() {
    let full = names(1024);
    let mut fixture = Fixture::standard();
    let a = fixture.creature_a;
    set_tags(&mut fixture, a, &refs(&full));
    let mut effect = granting(141, &["fresh"]);
    attach_to_ability(&mut fixture, effect.clone());
    assert_eq!(
        start_into(&fixture, &mut World::new(1)),
        Err(CombatError::TooManyTags)
    );
    // The same modifier identity twice in one effect.
    effect.modifiers.push(effect.modifiers[0].clone());
    attach_to_ability(&mut fixture, effect);
    assert!(matches!(
        start_into(&fixture, &mut World::new(1)),
        Err(CombatError::DuplicateModifier { .. })
    ));
}

/// R12: the `Display` form is pinned.
#[test]
fn too_many_tags_displays_its_location() {
    assert_eq!(
        CombatError::TooManyTags.to_string(),
        "TooManyTags at spec/tags"
    );
}

/// R12 (B20): the tag bound is per participant, so disjoint sets that
/// together exceed 1024 are accepted.
#[test]
fn the_tag_bound_is_per_participant() {
    let a_tags = names(600);
    let b_tags: Vec<String> = (0..600).map(|i| format!("other-{i:04}")).collect();
    let mut fixture = Fixture::standard();
    let a = fixture.creature_a;
    let b = fixture.creature_b;
    set_tags(&mut fixture, a, &refs(&a_tags));
    set_tags(&mut fixture, b, &refs(&b_tags));
    attach_to_ability(&mut fixture, granting(141, &["fresh"]));
    let (world, ids) = start(&fixture, 1);
    assert!(world.combat().is_some());
    assert_eq!(world.combatants().get(ids[0]).expect("A").tags().len(), 600);
    assert_eq!(world.combatants().get(ids[1]).expect("B").tags().len(), 600);
}
