//! T139a1: the persistent creature store, registration and load checks
//! (Behaviour B1-B7, B9 and the world half of B10; tests A1-A6 and A8).
//!
//! Every rejection compares the world's bytes and hash before and after and
//! has a positive control.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_data::RefreshWire;
use crpg_rules::{RefreshTrigger, ResourcePool, ResourcePoolId, MAX_TAGS};
use crpg_sim::{
    perform_action, register_creature, request_mode, state_hash, tick, CombatAction, CombatError,
    CreatureSpec, EncounterMode, EntityMeta, SimEvent, World, MAX_COMBAT_STATS,
};
use serde_json::{json, Value};
use support::creature::{
    add_outsider, edit_record, record, top_level_keys, try_with_json, with_json, with_record,
};
use support::draws::set_tags;
use support::rt::{json_keys, tick_pools, ticks, value_of};
use support::surface::{add_stat, remove_stat, set_stat, set_stat_raw};
use support::{uid, Fixture};

fn id(value: u128) -> Ulid {
    Ulid::from_u128(value)
}

fn bound(fixture: &Fixture) -> World {
    World::new_in_area(5, fixture.area)
}

fn bytes(world: &World) -> Vec<u8> {
    serde_json::to_vec(world).expect("the world serializes")
}

/// The whole world, for complete-state equality.
fn state(world: &World) -> (Vec<u8>, [u8; 32]) {
    (bytes(world), state_hash(world))
}

fn expect_fail(fixture: &Fixture, world: &mut World, placement: Ulid, expected: CombatError) {
    let before = state(world);
    assert_eq!(fixture.register(world, placement), Err(expected.clone()));
    assert_eq!(state(world), before, "{expected:?} mutated the world");
}

/// The `speed` fixture of A2: a `Fixed` stat on the ruleset and every creature.
fn speed_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    add_stat(&mut fixture, json!({"name": "speed", "kind": "fixed"}));
    add_outsider(&mut fixture, 131, 141);
    let creatures: Vec<Ulid> = fixture.creatures.keys().copied().collect();
    for creature in creatures {
        set_stat_raw(&mut fixture, creature, "speed", 98_304);
    }
    set_tags(&mut fixture, id(141), &["b", "a", "a"]);
    fixture
}

fn events_of(world: &mut World) -> Vec<SimEvent> {
    world
        .events_mut()
        .drain()
        .into_iter()
        .map(|envelope| envelope.payload)
        .collect()
}

/// A1 (B1, B11, B12): a world that uses none of it writes none of it.
/// Control: one registration writes `creatures`, last.
#[test]
fn a1_absent_when_unused() {
    let fixture = Fixture::standard();
    let (mut world, _) = fixture.start(3);
    let forbidden = ["creatures", "fixed", "entered", "lifecycle"];
    for step in 0..20 {
        let keys = json_keys(&value_of(&world));
        for key in forbidden {
            assert!(!keys.contains(key), "step {step}: `{key}` is written");
        }
        tick(&mut world);
        let actor = world
            .combat()
            .and_then(|state| state.active)
            .expect("a turn is active");
        perform_action(&mut world, &CombatAction::EndTurn { actor }).expect("the turn ends");
    }
    let empty = bytes(&World::new(7));
    assert_eq!(
        top_level_keys(&empty),
        [
            "entities",
            "transforms",
            "timeline",
            "events",
            "rng",
            "tick"
        ],
        "an empty world serializes as before"
    );
    assert_eq!(
        String::from_utf8(empty).expect("utf-8"),
        EMPTY_WORLD_SEED_7,
        "an empty world serializes byte for byte as before"
    );

    let mut control = bound(&fixture);
    fixture
        .register(&mut control, fixture.placement_a)
        .expect("registers");
    assert!(json_keys(&value_of(&control)).contains("creatures"));
    assert_eq!(
        top_level_keys(&bytes(&control)).last().map(String::as_str),
        Some("creatures")
    );
}

/// `World::new(7)` as serialized before T139 (the store is skipped when
/// empty).
const EMPTY_WORLD_SEED_7: &str = r#"{"entities":{"slots":[],"free":[]},"transforms":[],"timeline":[],"events":{"envelopes":[],"next_seq":0},"rng":{"seed":7,"streams":{}},"tick":0}"#;

/// A2 (B2, B4, B6): the exact record. Control: an outsider cloned from B.
#[test]
fn a2_registration_state() {
    let mut fixture = speed_fixture();
    tick_pools(&mut fixture, 4, &[3, 7]);
    let mut world = bound(&fixture);
    let rng_before = value_of(&world)["rng"].clone();
    let interners_before = value_of(&world).get("interners").cloned();
    let entity = fixture.register(&mut world, id(131)).expect("registers");

    let saved = world.creatures().get(entity).expect("a record");
    assert_eq!(saved.placement(), id(131));
    assert_eq!(saved.ruleset(), id(102));
    assert_eq!(
        saved.attributes(),
        [
            ("alpha".to_owned(), 8),
            ("beta".to_owned(), 6),
            ("gamma".to_owned(), 7)
        ]
    );
    assert_eq!(
        saved.fixed(),
        [("speed".to_owned(), Fx16_16::from_raw(98_304))]
    );
    assert_eq!((saved.health(), saved.max_health()), (10, 10));
    assert!(!saved.dead());
    let expected_pools = [
        ResourcePool::new(ResourcePoolId(id(105)), 4, 4, RefreshTrigger::OnTurnStart),
        ResourcePool::new(ResourcePoolId(id(106)), 3, 3, RefreshTrigger::OnTick(3)),
        ResourcePool::new(ResourcePoolId(id(107)), 3, 3, RefreshTrigger::OnTick(7)),
    ]
    .map(|pool| pool.expect("valid pool"));
    assert_eq!(saved.pools(), expected_pools);
    assert!(saved.suspended().is_empty());
    assert_eq!(saved.tags(), ["a", "b"]);
    let text = serde_json::to_string(saved).expect("serializes");
    assert!(text.ends_with(r#""tags":["a","b"]}"#), "{text}");
    assert_eq!(
        top_level_keys(text.as_bytes()).last().map(String::as_str),
        Some("tags")
    );
    assert!(!text.contains("suspended"));

    assert_eq!(events_of(&mut world), [SimEvent::Spawned { entity }]);
    assert!(world.transforms().get(entity).is_none());
    assert!(world.timeline().iter().next().is_none());
    assert!(world.combatants().is_empty() && world.combat().is_none());
    assert_eq!(value_of(&world)["rng"], rng_before);
    assert_eq!(value_of(&world).get("interners").cloned(), interners_before);

    // Control: an outsider cloned from B yields B's values.
    let mut control = speed_fixture();
    add_outsider(&mut control, 132, 142);
    let b = control.creatures[&control.creature_b].clone();
    let outsider = control.creatures.get_mut(&id(142)).expect("outsider");
    outsider.stats = b.stats.clone();
    outsider.tags = b.tags.clone();
    let mut world = bound(&control);
    let entity = control.register(&mut world, id(132)).expect("registers");
    let saved = world.creatures().get(entity).expect("a record");
    assert_eq!(
        saved.attributes(),
        [
            ("alpha".to_owned(), 6),
            ("beta".to_owned(), 7),
            ("gamma".to_owned(), 5)
        ]
    );
    assert_eq!((saved.health(), saved.max_health()), (6, 6));
    assert!(saved.tags().is_empty());
    assert!(!serde_json::to_string(saved)
        .expect("serializes")
        .contains("tags"));
}

/// A3 (B3): R1-R6 one by one and the precedence pairs.
#[test]
fn a3_registration_checks() {
    let outsider = id(131);
    let fresh = || {
        let mut fixture = Fixture::standard();
        add_outsider(&mut fixture, 131, 141);
        fixture
    };

    // R1.
    let mut fixture = fresh();
    for index in 0..MAX_COMBAT_STATS - fixture.ruleset.stats.len() + 1 {
        add_stat(
            &mut fixture,
            json!({"name": format!("flag{index}"), "kind": "bool"}),
        );
    }
    assert_eq!(fixture.ruleset.stats.len(), MAX_COMBAT_STATS + 1);
    expect_fail(
        &fixture,
        &mut bound(&fixture),
        outsider,
        CombatError::TooManyStats,
    );
    // R1 beats R2.
    expect_fail(
        &fixture,
        &mut World::new_in_area(5, id(900)),
        outsider,
        CombatError::TooManyStats,
    );

    // R2, with its controls.
    let fixture = fresh();
    expect_fail(
        &fixture,
        &mut World::new_in_area(5, id(900)),
        outsider,
        CombatError::AreaMismatch {
            expected: id(900),
            found: fixture.area,
        },
    );
    assert!(fixture.register(&mut bound(&fixture), outsider).is_ok());
    assert!(
        fixture.register(&mut World::new(5), outsider).is_ok(),
        "an unbound world accepts any area"
    );

    // R3, with R2 beating it.
    let mismatched = |fixture: &Fixture, world: &mut World| {
        let (document, area) = &fixture.placements[&outsider];
        let spec = CreatureSpec {
            ruleset: &fixture.ruleset,
            placement: document,
            area: *area,
            creature: &fixture.creatures[&fixture.creature_b],
        };
        let before = state(world);
        let result = register_creature(world, &spec);
        assert_eq!(state(world), before, "a failed registration mutated");
        result
    };
    assert_eq!(
        mismatched(&fixture, &mut bound(&fixture)),
        Err(CombatError::MissingCreature { prefab: id(141) })
    );
    assert_eq!(
        mismatched(&fixture, &mut World::new_in_area(5, id(900))),
        Err(CombatError::AreaMismatch {
            expected: id(900),
            found: fixture.area
        })
    );

    // R4: a missing Int, a fractional Int, health 0 and a missing Fixed.
    let mut broken = fresh();
    remove_stat(&mut broken, id(141), "beta");
    expect_fail(
        &broken,
        &mut bound(&broken),
        outsider,
        CombatError::MissingStat {
            stat: "beta".to_owned(),
        },
    );
    // R3 beats R4.
    assert_eq!(
        mismatched(&broken, &mut bound(&broken)),
        Err(CombatError::MissingCreature { prefab: id(141) })
    );
    let mut broken = fresh();
    set_stat_raw(&mut broken, id(141), "alpha", 8 * 65_536 + 1);
    expect_fail(
        &broken,
        &mut bound(&broken),
        outsider,
        CombatError::InvalidStatValue {
            stat: "alpha".to_owned(),
        },
    );
    let mut broken = fresh();
    set_stat(&mut broken, id(141), "health", 0);
    expect_fail(
        &broken,
        &mut bound(&broken),
        outsider,
        CombatError::InvalidStatValue {
            stat: "health".to_owned(),
        },
    );
    let mut broken = fresh();
    add_stat(&mut broken, json!({"name": "speed", "kind": "fixed"}));
    expect_fail(
        &broken,
        &mut bound(&broken),
        outsider,
        CombatError::MissingStat {
            stat: "speed".to_owned(),
        },
    );

    // R5: twice, with R4 beating it.
    let mut fixture = fresh();
    let mut world = bound(&fixture);
    fixture.register(&mut world, outsider).expect("first");
    expect_fail(
        &fixture,
        &mut world,
        outsider,
        CombatError::PlacementInUse {
            placement: outsider,
        },
    );
    remove_stat(&mut fixture, id(141), "beta");
    expect_fail(
        &fixture,
        &mut world,
        outsider,
        CombatError::MissingStat {
            stat: "beta".to_owned(),
        },
    );

    // R6, with R5 beating it, and its control.
    let mut fixture = fresh();
    let mut world = bound(&fixture);
    fixture.register(&mut world, outsider).expect("first");
    let many: Vec<String> = (0..=MAX_TAGS).map(|index| format!("t{index:05}")).collect();
    let many_refs: Vec<&str> = many.iter().map(String::as_str).collect();
    set_tags(&mut fixture, id(141), &many_refs);
    expect_fail(
        &fixture,
        &mut world,
        outsider,
        CombatError::PlacementInUse {
            placement: outsider,
        },
    );
    expect_fail(
        &fixture,
        &mut bound(&fixture),
        outsider,
        CombatError::TooManyTags,
    );
    let mut within = many_refs[..MAX_TAGS].to_vec();
    within.push(within[0]);
    set_tags(&mut fixture, id(141), &within);
    let mut world = bound(&fixture);
    let entity = fixture.register(&mut world, outsider).expect("fits");
    assert_eq!(
        world.creatures().get(entity).expect("record").tags().len(),
        MAX_TAGS
    );

    // R5 against a combatant; the control despawns the combatant.
    let fixture = fresh();
    let (mut world, ids) = fixture.start(3);
    expect_fail(
        &fixture,
        &mut world,
        fixture.placement_a,
        CombatError::PlacementInUse {
            placement: fixture.placement_a,
        },
    );
    assert!(world.despawn(ids[0]));
    fixture
        .register(&mut world, fixture.placement_a)
        .expect("a despawned combatant frees its placement");
}

/// A4 (B5): nothing changes a record outside an encounter.
#[test]
fn a4_nothing_happens_outside() {
    let mut fixture = Fixture::standard();
    add_outsider(&mut fixture, 131, 141);
    tick_pools(&mut fixture, 2, &[1, 1]);
    fixture.ruleset.pools[2].refresh = RefreshWire::OnRoundStart;
    let mut world = bound(&fixture);
    let entity = fixture.register(&mut world, id(131)).expect("registers");
    let mut world = with_record(&world, entity, |saved| {
        for pool in saved["pools"].as_array_mut().expect("pools") {
            pool["current"] = json!(0);
        }
        saved["suspended"] = json!([{"effect": uid(500), "remaining_rounds": 2}]);
    });
    let before = record(&world, entity);
    assert_eq!(before["pools"][1]["current"], json!(0));
    ticks(&mut world, 100);
    assert_eq!(
        request_mode(&mut world, EncounterMode::RealTime),
        Err(CombatError::NoEncounter)
    );
    assert_eq!(record(&world, entity), before, "the record is unchanged");
    // Control: the clock did run.
    assert_eq!(world.tick().get(), 100);
}

/// A5 (B6): only base `Int` and `Fixed` stats are stored.
#[test]
fn a5_only_int_and_fixed_stored() {
    let mut fixture = Fixture::standard();
    add_outsider(&mut fixture, 131, 141);
    add_stat(&mut fixture, json!({"name": "flag", "kind": "bool"}));
    add_stat(
        &mut fixture,
        json!({"name": "mood", "kind": {"enum": {"enum_id": "moods", "variants": ["calm", "fierce"]}}}),
    );
    add_stat(&mut fixture, json!({"name": "marks", "kind": "tags"}));
    add_stat(&mut fixture, json!({"name": "roll", "kind": "dice"}));
    add_stat(
        &mut fixture,
        json!({"name": "half", "kind": "int", "derived": {
            "type": "divide", "value": {
                "left": {"type": "stat", "value": "alpha"},
                "right": {"type": "literal", "value": {"type": "int", "value": 2}}
            }
        }}),
    );
    add_stat(&mut fixture, json!({"name": "speed", "kind": "fixed"}));
    set_stat_raw(&mut fixture, id(141), "speed", 98_304);
    let mut world = bound(&fixture);
    let entity = fixture.register(&mut world, id(131)).expect("registers");
    let saved = record(&world, entity);
    let text = saved.to_string();
    for name in ["flag", "mood", "marks", "roll", "half"] {
        assert!(!text.contains(name), "`{name}` is stored: {text}");
    }
    assert_eq!(
        saved["attributes"],
        json!([["alpha", 8], ["beta", 6], ["gamma", 7]])
    );
    // Control: the base `Fixed` stat is present.
    assert_eq!(saved["fixed"], json!([["speed", 98_304]]));
}

/// A6 (B7): despawn strips the record and enqueues one `Despawned`.
#[test]
fn a6_despawn_strips_record() {
    let mut fixture = Fixture::standard();
    add_outsider(&mut fixture, 131, 141);
    add_outsider(&mut fixture, 132, 142);
    let mut world = bound(&fixture);
    let first = fixture.register(&mut world, id(131)).expect("registers");
    let second = fixture.register(&mut world, id(132)).expect("registers");
    let _ = events_of(&mut world);
    assert!(world.despawn(first));
    assert!(world.creatures().get(first).is_none());
    assert_eq!(
        events_of(&mut world),
        [SimEvent::Despawned { entity: first }]
    );
    // Control: another record is intact, and a bare despawn leaves it so.
    assert_eq!(world.creatures().len(), 1);
    let bare = world.spawn(EntityMeta {});
    let kept = record(&world, second);
    assert!(world.despawn(bare));
    assert_eq!(record(&world, second), kept);
    assert!(world.despawn(second));
    assert!(world.creatures().is_empty());
    assert!(!json_keys(&value_of(&world)).contains("creatures"));
}

/// A world holding the records of placements A and B.
fn two_record_world() -> (World, EntityId, EntityId) {
    let fixture = Fixture::standard();
    let mut world = bound(&fixture);
    let first = fixture
        .register(&mut world, fixture.placement_a)
        .expect("registers");
    let second = fixture
        .register(&mut world, fixture.placement_b)
        .expect("registers");
    (world, first, second)
}

fn load_error(world: &World, edit: impl FnOnce(&mut Value)) -> String {
    try_with_json(world, edit).expect_err("the edited save is refused")
}

/// A8 (B9): K1, K3-K12, each with its exact message. P1 is T139a2's.
#[test]
fn a8_load_checks() {
    let (world, first, _) = two_record_world();
    assert_eq!(world.creatures().len(), 2);
    let saved = serde_json::to_value(&world).expect("serializes");
    let reloaded: World = serde_json::from_value(saved).expect("the save loads");
    assert_eq!(reloaded, world, "two records round-trip equal");

    let edit_first =
        |edit: fn(&mut Value)| load_error(&world, |save| edit_record(save, first, edit));
    let pool = record(&world, first)["pools"][0].clone();

    // K1: a record for a non-live entity.
    let message = load_error(&world, |save| {
        let copy = save["creatures"][0][1].clone();
        save["creatures"]
            .as_array_mut()
            .expect("list")
            .push(json!([{"index": 99, "generation": 1}, copy]));
    });
    assert!(
        message.starts_with("dangling creature for non-live "),
        "{message}"
    );
    // K1 beats K3.
    let message = load_error(&world, |save| {
        let mut copy = save["creatures"][0][1].clone();
        copy["max_health"] = json!(0);
        save["creatures"]
            .as_array_mut()
            .expect("list")
            .push(json!([{"index": 99, "generation": 1}, copy]));
    });
    assert!(message.starts_with("dangling creature"), "{message}");

    // K3-K9, K11 and K12 on the first record; K10 across the two.
    assert_eq!(
        edit_first(|r| r["max_health"] = json!(0)),
        "creature with a nonpositive maximum"
    );
    assert_eq!(
        edit_first(|r| r["health"] = json!(11)),
        "creature health exceeds its maximum"
    );
    assert_eq!(
        edit_first(|r| r["pools"] = json!([])),
        "creature declares no pools"
    );
    assert_eq!(
        load_error(&world, |save| edit_record(save, first, |r| {
            r["pools"] = json!([pool.clone(), pool.clone()]);
        })),
        "creature declares a duplicate pool"
    );
    assert_eq!(
        edit_first(|r| r["attributes"] = json!([["alpha", 1], ["alpha", 2]])),
        "creature stats repeat a name"
    );
    assert_eq!(
        edit_first(|r| r["fixed"] = json!([["alpha", 65_536]])),
        "creature stats repeat a name",
        "a name repeated across attributes and fixed"
    );
    assert_eq!(
        edit_first(|r| {
            r["suspended"] = json!([
                {"effect": uid(502), "remaining_rounds": 1},
                {"effect": uid(501), "remaining_rounds": 1}
            ]);
        }),
        "creature attachments break ascending order"
    );
    assert_eq!(
        edit_first(|r| {
            r["suspended"] = json!([
                {"effect": uid(501), "remaining_rounds": 1},
                {"effect": uid(501), "remaining_rounds": 1}
            ]);
        }),
        "creature attachments break ascending order",
        "equal identities are not strictly ascending"
    );
    assert_eq!(
        edit_first(|r| r["suspended"] = json!([{"effect": uid(501), "remaining_rounds": 0}])),
        "creature attachment carries no remaining rounds"
    );
    assert_eq!(
        load_error(&world, |save| {
            let placement = save["creatures"][0][1]["placement"].clone();
            save["creatures"][1][1]["placement"] = placement;
        }),
        "creatures share a placement identity"
    );
    assert_eq!(
        edit_first(|r| r["tags"] = json!(["b", "a"])),
        "creature tags break ascending order"
    );
    assert_eq!(
        edit_first(|r| r["tags"] = json!(["a", "a"])),
        "creature tags break ascending order",
        "a repeat fails here too"
    );
    let many: Vec<String> = (0..=MAX_TAGS).map(|index| format!("t{index:05}")).collect();
    assert_eq!(
        load_error(&world, |save| edit_record(save, first, |r| {
            r["tags"] = json!(many);
        })),
        "creature tags exceed their bound"
    );
    // Controls: MAX_TAGS ascending tags load; a pool above its maximum fails
    // in the pool's own decoder.
    let within = &many[..MAX_TAGS];
    let loaded = with_record(&world, first, |r| r["tags"] = json!(within));
    assert_eq!(
        loaded.creatures().get(first).expect("record").tags().len(),
        MAX_TAGS
    );
    assert!(load_error(&world, |save| edit_record(save, first, |r| {
        r["pools"][0]["current"] = json!(9);
    }))
    .contains("current"));
}

/// A8 (B9, K2 and the combatant-placement control): a record on a combatant
/// is refused; a record whose placement equals a combatant's loads.
#[test]
fn a8_load_checks_against_combatants() {
    let fixture = Fixture::standard();
    let mut donor_world = bound(&fixture);
    let donor_entity = fixture
        .register(&mut donor_world, fixture.placement_a)
        .expect("registers");
    let donor = record(&donor_world, donor_entity);
    let (mut world, ids) = fixture.start(3);
    let bare = world.spawn(EntityMeta {});
    let id_json = |entity: EntityId| serde_json::to_value(entity).expect("id");

    let message = load_error(&world, |save| {
        save["creatures"] = json!([[id_json(ids[0]), donor.clone()]]);
    });
    assert_eq!(message, "creature state for a combatant");
    // K1 beats K2: a record for a non-live entity and one for a combatant.
    let message = load_error(&world, |save| {
        save["creatures"] = json!([
            [id_json(ids[0]), donor.clone()],
            [{"index": 99, "generation": 1}, donor.clone()]
        ]);
    });
    assert!(message.starts_with("dangling creature"), "{message}");
    // Control: a record on a bare entity whose placement equals A's loads
    // (a respawning encounter, B13).
    let loaded = with_json(&world, |save| {
        save["creatures"] = json!([[id_json(bare), donor.clone()]]);
    });
    assert_eq!(loaded.creatures().len(), 1);
    assert_eq!(
        loaded.creatures().get(bare).expect("record").placement(),
        fixture.placement_a
    );
    assert_eq!(loaded.combatants().len(), 2);
}
