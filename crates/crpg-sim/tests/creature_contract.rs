//! T139a contract tests, written from `tasks/T139.md` alone: the store,
//! registration and life outside encounters (B1-B10, R1-R6, T1, K1-K12, P1),
//! and the lifecycle at start and release (B11-B20, S1-S4). Every rejection
//! compares the world's bytes before and after.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_data::{ActionPoolTemplate, CreaturePersistenceWire, EffectsOnExitWire, RefreshWire};
use crpg_rules::{RefreshTrigger, ResourcePool, ResourcePoolId, MAX_TAGS};
use crpg_sim::{
    end_encounter, history_hash, register_creature, request_mode, state_hash, transfer_entity,
    transfer_history_entity, AreaError, AttachedEffect, CombatError, CreatureSpec, EffectsOnExit,
    EncounterMode, EntityMeta, HistoryEvent, HistoryWorld, Lifecycle, SimEvent, SuspendedEffect,
    Transform, World, MAX_COMBAT_STATS,
};
use serde_json::{json, Value};
use support::creature::{
    add_outsider, edit_combatant, edit_record, record, set_hooks, top_level_keys, try_with_json,
    with_json, with_record,
};
use support::draws::{draw, grant, oracle, set_initiative_rule, set_tags, start_into};
use support::rt::{bytes_of as bytes, json_keys, tick_pools, ticks, timeline_of, value_of};
use support::surface::{
    act, add_derived, add_stat, always_success, attach_to_ability, effect_doc, end, modifier,
    remove_stat, set_stat, set_stat_raw, stat,
};
use support::{uid, Fixture};

fn id(v: u128) -> Ulid {
    Ulid::from_u128(v)
}
fn ids_of(w: &World) -> Vec<EntityId> {
    w.combatants().iter().map(|(id, _)| id).collect()
}
fn rec_bytes(w: &World, e: EntityId) -> Vec<u8> {
    serde_json::to_vec(w.creatures().get(e).expect("a record")).expect("a record serializes")
}
fn events(w: &mut World, spawn: bool) -> Vec<EntityId> {
    w.events_mut()
        .drain()
        .into_iter()
        .filter_map(|env| match env.payload {
            SimEvent::Spawned { entity } if spawn => Some(entity),
            SimEvent::Despawned { entity } if !spawn => Some(entity),
            _ => None,
        })
        .collect()
}
fn spawned(w: &mut World) -> Vec<EntityId> {
    events(w, true)
}
fn despawned(w: &mut World) -> Vec<EntityId> {
    events(w, false)
}
fn rejected<T: PartialEq + std::fmt::Debug>(
    result: Result<T, CombatError>,
    expected: CombatError,
    w: &World,
    before: &[u8],
) {
    assert_eq!(result, Err(expected));
    assert_eq!(bytes(w), before, "a rejection leaves the world unchanged");
}
fn reg_rej(f: &Fixture, w: &mut World, placement: u128, expected: CombatError) {
    let before = bytes(w);
    rejected(f.register(w, id(placement)), expected, w, &before);
}
fn start_rej(f: &Fixture, w: &mut World, expected: CombatError) {
    let before = bytes(w);
    rejected(start_into(f, w), expected, w, &before);
}
fn reg(f: &Fixture, w: &mut World, placement: u128) -> EntityId {
    f.register(w, id(placement)).expect("registers")
}
fn outsider() -> Fixture {
    let mut f = Fixture::standard();
    add_outsider(&mut f, 131, 141);
    f
}
fn spec<'a>(f: &'a Fixture, placement: Ulid, creature: Ulid) -> CreatureSpec<'a> {
    let (document, area) = &f.placements[&placement];
    CreatureSpec {
        ruleset: &f.ruleset,
        placement: document,
        area: *area,
        creature: &f.creatures[&creature],
    }
}
fn add_pool(f: &mut Fixture) {
    f.ruleset.pools.push(ActionPoolTemplate {
        id: id(106),
        max: 3,
        refresh: RefreshWire::OnTurnStart,
    });
}
fn add_speed(f: &mut Fixture) {
    add_stat(f, json!({"name": "speed", "kind": "fixed"}));
    let (a, b) = (f.creature_a, f.creature_b);
    set_stat_raw(f, a, "speed", 98_304);
    set_stat_raw(f, b, "speed", 98_304);
}
fn many_tags() -> Vec<String> {
    (0..=MAX_TAGS).map(|i| format!("t{i:05}")).collect()
}
fn set_tags_n(f: &mut Fixture, which: Ulid, count: usize) {
    let many = many_tags();
    let refs: Vec<&str> = many[..count].iter().map(String::as_str).collect();
    set_tags(f, which, &refs);
}
fn bad_derived(f: &mut Fixture, name: &str) {
    let four = json!({"type": "literal", "value": {"type": "int", "value": 4}});
    let divide = json!({"type": "divide", "value": {
        "left": stat("alpha"),
        "right": {"type": "subtract", "value": {"left": stat("health"), "right": four}}
    }});
    add_derived(f, name, divide);
}
fn attach_grant(f: &mut Fixture, effect_id: u128, tags: &[&str]) {
    let mods = vec![modifier(effect_id + 1, json!({"type": "roll"}), "add", 1)];
    let effect = effect_doc(effect_id, "target", "probe-type", "stack_all", mods, 2);
    attach_to_ability(f, grant(effect, tags));
}
fn tagged_speed() -> Fixture {
    let mut f = Fixture::standard();
    add_speed(&mut f);
    let (a, b) = (f.creature_a, f.creature_b);
    set_tags(&mut f, a, &["b", "a", "a"]);
    set_tags(&mut f, b, &["solo"]);
    f
}

/// A fixture whose stats are authored out of alphabetical order, with two
/// `Fixed` stats, so the record's `attributes` and `fixed` order is proven to
/// be authored order (B6), not sorted.
fn out_of_order() -> Fixture {
    let mut f = Fixture::standard();
    add_stat(&mut f, json!({"name": "zeta", "kind": "fixed"}));
    add_stat(&mut f, json!({"name": "delta", "kind": "fixed"}));
    let (a, b) = (f.creature_a, f.creature_b);
    set_stat_raw(&mut f, a, "zeta", 98_304);
    set_stat_raw(&mut f, b, "zeta", 98_304);
    set_stat_raw(&mut f, a, "delta", 131_072);
    set_stat_raw(&mut f, b, "delta", 131_072);
    // Author the Int stat declarations out of alphabetical order.
    f.ruleset.stats.swap(0, 2);
    add_outsider(&mut f, 131, 141);
    f
}

/// B1: the store is the only persistent state; absence and single copy.
#[test]
fn b1_store_absence_and_single_copy() {
    let empty = World::new(7);
    assert!(empty.creatures().is_empty());
    assert!(!json_keys(&value_of(&empty)).contains("creatures"));
    let f = outsider();
    let mut w = World::new(7);
    let e = reg(&f, &mut w, 131);
    assert_eq!(w.creatures().len(), 1);
    assert_eq!(
        top_level_keys(&bytes(&w)).last().map(String::as_str),
        Some("creatures")
    );
    assert!(!w.transforms().contains(e) && !w.timeline().contains(e));
    let mut w = World::new(7);
    let entered = f.register(&mut w, f.placement_a).expect("registers");
    start_into(&f, &mut w).expect("starts");
    assert!(
        w.creatures().get(entered).is_none(),
        "never both a record and a combatant"
    );
    assert!(w.combatants().get(entered).is_some());
}

/// B2, B4, B6: record shape, publication and stored stats.
#[test]
fn b2_b4_b6_registration_state() {
    let mut f = Fixture::standard();
    add_pool(&mut f);
    add_speed(&mut f);
    add_outsider(&mut f, 131, 141);
    set_tags(&mut f, id(141), &["b", "a", "a"]);
    let mut w = World::new(5);
    let interners = w.interners().clone();
    let rng = w.rng_mut().clone();
    let e = reg(&f, &mut w, 131);
    assert_eq!(spawned(&mut w), vec![e], "exactly one Spawned");
    assert!(!w.transforms().contains(e) && !w.timeline().contains(e));
    assert_eq!(
        (w.rng_mut().clone(), w.interners().clone()),
        (rng, interners),
        "no draw, no interning"
    );
    let r = w.creatures().get(e).expect("a record");
    assert_eq!((r.placement(), r.ruleset()), (id(131), f.ruleset.id));
    let attrs = [
        ("alpha".to_owned(), 8),
        ("beta".to_owned(), 6),
        ("gamma".to_owned(), 7),
    ];
    assert_eq!(r.attributes(), attrs);
    assert_eq!(r.fixed(), [("speed".to_owned(), Fx16_16::from_raw(98_304))]);
    assert_eq!((r.health(), r.max_health(), r.dead()), (10, 10, false));
    let pools = vec![
        ResourcePool::new(ResourcePoolId(id(105)), 1, 1, RefreshTrigger::OnTurnStart)
            .expect("pool"),
        ResourcePool::new(ResourcePoolId(id(106)), 3, 3, RefreshTrigger::OnTurnStart)
            .expect("pool"),
    ];
    assert_eq!(r.pools(), pools, "one pool per template, at max");
    assert!(r.suspended().is_empty());
    assert_eq!(r.tags(), ["a", "b"], "sorted and deduplicated");
    let names: Vec<&str> = r.attributes().iter().map(|(n, _)| n.as_str()).collect();
    assert!(r.fixed().iter().all(|(n, _)| !names.contains(&n.as_str())));
    let pool_ids: Vec<ResourcePoolId> = r.pools().iter().map(|p| p.id()).collect();
    for (index, first) in pool_ids.iter().enumerate() {
        assert!(!pool_ids[index + 1..].contains(first), "distinct pool ids");
    }
    let json = serde_json::to_vec(r).expect("serializes");
    assert_eq!(
        top_level_keys(&json).last().map(String::as_str),
        Some("tags")
    );
    let outsider = outsider();
    let (mut active, _) = Fixture::standard().start(5);
    assert!(active.combat().is_some());
    let other = outsider
        .register(&mut active, id(131))
        .expect("registration is allowed during an encounter");
    assert!(active.creatures().get(other).is_some());
}

/// B6: only base non-derived Int and Fixed stats are stored.
#[test]
fn b6_only_int_and_fixed_stored() {
    let mut f = Fixture::standard();
    add_stat(&mut f, json!({"name": "flag", "kind": "bool"}));
    add_stat(
        &mut f,
        json!({"name": "mood", "kind": {"enum": {"enum_id": "m", "variants": ["a", "b"]}}}),
    );
    add_stat(&mut f, json!({"name": "marks", "kind": "tags"}));
    add_stat(&mut f, json!({"name": "roll", "kind": "dice"}));
    add_derived(&mut f, "total", json!({"type": "stat", "value": "alpha"}));
    add_speed(&mut f);
    add_outsider(&mut f, 131, 141);
    let mut w = World::new(5);
    let entity = reg(&f, &mut w, 131);
    let r = w.creatures().get(entity).expect("a record");
    let json = serde_json::to_string(r).expect("serializes");
    for name in ["flag", "mood", "marks", "roll", "total"] {
        assert!(!json.contains(name), "{name} must not be stored");
    }
    assert_eq!(r.attributes().len(), 3);
    assert_eq!(r.fixed(), [("speed".to_owned(), Fx16_16::from_raw(98_304))]);
}

/// B4: a non-`OnTurnStart` pool refresh is converted, not flattened to
/// `OnTurnStart`.
#[test]
fn b4_converted_refresh_trigger() {
    let mut f = outsider();
    tick_pools(&mut f, 1, &[3, 7]);
    let mut w = World::new(5);
    let e = reg(&f, &mut w, 131);
    let pools = w.creatures().get(e).expect("r").pools().to_vec();
    assert_eq!(pools.len(), 3, "primary then two OnTick pools");
    assert_eq!(pools[0].refresh_trigger(), RefreshTrigger::OnTurnStart);
    assert_eq!(pools[1].refresh_trigger(), RefreshTrigger::OnTick(3));
    assert_eq!(pools[2].refresh_trigger(), RefreshTrigger::OnTick(7));
}

/// B6: attributes and fixed keep authored (not sorted) order, with more than
/// one Fixed stat.
#[test]
fn b6_authored_order_with_two_fixed() {
    let f = out_of_order();
    let mut w = World::new(5);
    let e = reg(&f, &mut w, 131);
    let r = w.creatures().get(e).expect("r");
    assert_eq!(
        r.attributes(),
        [
            ("gamma".to_owned(), 7),
            ("beta".to_owned(), 6),
            ("alpha".to_owned(), 8),
        ],
        "attributes follow the authored stat order"
    );
    assert_eq!(
        r.fixed(),
        [
            ("zeta".to_owned(), Fx16_16::from_raw(98_304)),
            ("delta".to_owned(), Fx16_16::from_raw(131_072)),
        ],
        "fixed follows the authored stat order"
    );
}

/// B3: registration checks R1-R6, each mutating nothing.
#[test]
fn b3_registration_checks() {
    let mut f = outsider();
    for i in 0..MAX_COMBAT_STATS - f.ruleset.stats.len() {
        add_stat(&mut f, json!({"name": format!("s{i}"), "kind": "int"}));
        set_stat(&mut f, id(141), &format!("s{i}"), 1);
    }
    assert_eq!(f.ruleset.stats.len(), MAX_COMBAT_STATS);
    let mut w = World::new(5);
    reg(&f, &mut w, 131);

    let mut f = outsider();
    for i in 0..MAX_COMBAT_STATS - f.ruleset.stats.len() + 1 {
        add_stat(&mut f, json!({"name": format!("s{i}"), "kind": "int"}));
    }
    reg_rej(&f, &mut World::new(5), 131, CombatError::TooManyStats);

    // R1 beats R2: over-bound stats plus a foreign area reports TooManyStats.
    let mut bound = World::new_in_area(5, id(999));
    reg_rej(&f, &mut bound, 131, CombatError::TooManyStats);

    let f = outsider();
    let mut bound = World::new_in_area(5, id(999));
    let before = bytes(&bound);
    let expected = CombatError::AreaMismatch {
        expected: id(999),
        found: f.area,
    };
    rejected(f.register(&mut bound, id(131)), expected, &bound, &before);
    let mut same = World::new_in_area(5, f.area);
    f.register(&mut same, id(131)).expect("same area registers");

    let f = outsider();
    let mut w = World::new(5);
    let before = bytes(&w);
    let result = register_creature(&mut w, &spec(&f, id(131), f.creature_b));
    rejected(
        result,
        CombatError::MissingCreature { prefab: id(141) },
        &w,
        &before,
    );

    let r4: [fn(&mut Fixture); 4] = [
        |f| remove_stat(f, id(141), "beta"),
        |f| set_stat_raw(f, id(141), "alpha", 1),
        |f| set_stat(f, id(141), "health", 0),
        |f| add_speed(f),
    ];
    let expected = [
        CombatError::MissingStat {
            stat: "beta".into(),
        },
        CombatError::InvalidStatValue {
            stat: "alpha".into(),
        },
        CombatError::InvalidStatValue {
            stat: "health".into(),
        },
        CombatError::MissingStat {
            stat: "speed".into(),
        },
    ];
    for (edit, expected) in r4.into_iter().zip(expected) {
        let mut f = outsider();
        edit(&mut f);
        reg_rej(&f, &mut World::new(5), 131, expected);
    }

    // R4's sub-order: the first fractional base Int stat in authored order wins.
    let mut f = Fixture::standard();
    f.ruleset.stats.swap(0, 2);
    add_outsider(&mut f, 131, 141);
    set_stat_raw(&mut f, id(141), "gamma", 1);
    set_stat_raw(&mut f, id(141), "alpha", 1);
    reg_rej(
        &f,
        &mut World::new(5),
        131,
        CombatError::InvalidStatValue {
            stat: "gamma".into(),
        },
    );

    let f = outsider();
    let mut w = World::new(5);
    reg(&f, &mut w, 131);
    reg_rej(
        &f,
        &mut w,
        131,
        CombatError::PlacementInUse { placement: id(131) },
    );
    let f = Fixture::standard();
    let mut w = World::new(5);
    start_into(&f, &mut w).expect("starts");
    let placement = f.placement_a;
    let before = bytes(&w);
    let expected = CombatError::PlacementInUse { placement };
    rejected(f.register(&mut w, placement), expected, &w, &before);
    let a = ids_of(&w)[0];
    w.despawn(a);
    f.register(&mut w, placement)
        .expect("free placement registers");

    let mut f = outsider();
    set_tags_n(&mut f, id(141), MAX_TAGS + 1);
    reg_rej(&f, &mut World::new(5), 131, CombatError::TooManyTags);
    let mut f = outsider();
    let mut repeated = many_tags();
    repeated.truncate(MAX_TAGS);
    repeated.push("t00000".to_owned());
    let refs: Vec<&str> = repeated.iter().map(String::as_str).collect();
    set_tags(&mut f, id(141), &refs);
    let mut w = World::new(5);
    let e = reg(&f, &mut w, 131);
    assert_eq!(w.creatures().get(e).expect("r").tags().len(), MAX_TAGS);
}

/// B3: an empty pool template list panics with start's text.
#[test]
#[should_panic(expected = "ruleset pools are validated nonempty by data read-time checks")]
fn b3_empty_pools_panics() {
    let mut f = outsider();
    f.ruleset.pools.clear();
    let mut w = World::new(5);
    let _ = f.register(&mut w, id(131));
}

/// B3: R2 beats R3, R3 beats R4, R4 beats R5, R5 beats R6.
#[test]
fn b3_registration_precedence() {
    let f = outsider();
    let mut w = World::new_in_area(5, id(999));
    let before = bytes(&w);
    let result = register_creature(&mut w, &spec(&f, id(131), f.creature_b));
    rejected(
        result,
        CombatError::AreaMismatch {
            expected: id(999),
            found: f.area,
        },
        &w,
        &before,
    );

    let mut f = outsider();
    let rival = f.creature_b;
    remove_stat(&mut f, rival, "beta");
    let mut w = World::new(5);
    let before = bytes(&w);
    let result = register_creature(&mut w, &spec(&f, id(131), rival));
    rejected(
        result,
        CombatError::MissingCreature { prefab: id(141) },
        &w,
        &before,
    );

    let mut f = outsider();
    let mut w = World::new(5);
    reg(&f, &mut w, 131);
    remove_stat(&mut f, id(141), "beta");
    reg_rej(
        &f,
        &mut w,
        131,
        CombatError::MissingStat {
            stat: "beta".into(),
        },
    );

    let mut f = outsider();
    let mut w = World::new(5);
    reg(&f, &mut w, 131);
    set_tags_n(&mut f, id(141), MAX_TAGS + 1);
    reg_rej(
        &f,
        &mut w,
        131,
        CombatError::PlacementInUse { placement: id(131) },
    );
}

/// B5: nothing changes a record outside an encounter, and no combat op touches it.
#[test]
fn b5_nothing_happens_outside() {
    let mut f = outsider();
    tick_pools(&mut f, 1, &[1, 7]);
    let mut w = World::new(3);
    let e = reg(&f, &mut w, 131);
    let mut w = with_record(&w, e, |r| {
        for pool in r["pools"].as_array_mut().expect("pools") {
            pool["current"] = json!(0);
        }
        r["suspended"] = json!([{"effect": uid(200), "remaining_rounds": 2}]);
    });
    let before = rec_bytes(&w, e);
    ticks(&mut w, 100);
    assert_eq!(w.tick().get(), 100);
    assert_eq!(
        request_mode(&mut w, EncounterMode::RealTime),
        Err(CombatError::NoEncounter)
    );
    assert_eq!(rec_bytes(&w, e), before, "no tick, refresh or countdown");
    let r = w.creatures().get(e).expect("r");
    assert_eq!(r.suspended()[0].remaining_rounds, 2);
    assert!(r.pools().iter().all(|pool| pool.current() == 0));

    let f = outsider();
    let mut w = World::new(6);
    let e = reg(&f, &mut w, 131);
    start_into(&f, &mut w).expect("starts");
    let before = rec_bytes(&w, e);
    let ids = ids_of(&w);
    act(&mut w, ids[1], f.ability, ids[0]);
    assert_eq!(rec_bytes(&w, e), before, "a combat op on others leaves it");
}

/// B7: despawn strips the record with the rest and enqueues one Despawned.
#[test]
fn b7_despawn_strips_record() {
    let mut f = Fixture::standard();
    add_outsider(&mut f, 131, 141);
    add_outsider(&mut f, 132, 142);
    let mut w = World::new(5);
    let first = reg(&f, &mut w, 131);
    reg(&f, &mut w, 132);
    assert!(w.despawn(first));
    assert!(w.creatures().get(first).is_none());
    assert_eq!(w.creatures().len(), 1, "the other record is intact");
    assert_eq!(despawned(&mut w), vec![first]);
    assert!(!w.despawn(first), "a dead id despawns nothing");
    assert!(w.events().is_empty(), "and enqueues nothing");
}

/// B8 + T1: transfer carries the record; placement collisions refuse.
#[test]
fn b8_transfer_carries_record_and_collisions() {
    let f = Fixture::standard();
    let (x_area, y_area) = (f.area, id(777));
    let mut source = World::new_in_area(5, x_area);
    let e = f.register(&mut source, f.placement_a).expect("registers");
    let saved = record(&source, e);
    let mut dest = World::new_in_area(5, y_area);
    let moved = transfer_entity(&mut source, &mut dest, e).expect("moves");
    assert!(source.creatures().get(e).is_none());
    assert_eq!(record(&dest, moved), saved, "byte-equal record");
    let mut hs = HistoryWorld::new_in_area(5, x_area);
    let he = f
        .register_history(&mut hs, f.placement_a)
        .expect("registers");
    let saved = record(hs.world(), he);
    let mut hd = HistoryWorld::new_in_area(5, y_area);
    let hm = transfer_history_entity(&mut hs, &mut hd, he).expect("moves");
    assert!(hs.world().creatures().get(he).is_none());
    assert_eq!(record(hd.world(), hm), saved);
    let two = f
        .register_history(&mut hs, f.placement_a)
        .expect("free in X");
    let before = (history_hash(&hs), history_hash(&hd));
    assert_eq!(
        transfer_history_entity(&mut hs, &mut hd, two),
        Err(AreaError::PlacementInUse {
            placement: f.placement_a
        }),
        "the history pair refuses too"
    );
    assert_eq!(
        (history_hash(&hs), history_hash(&hd)),
        before,
        "both journals unchanged"
    );

    let mut source = World::new_in_area(5, x_area);
    let bare = source.spawn(EntityMeta {});
    let mut dest = World::new_in_area(5, y_area);
    let moved = transfer_entity(&mut source, &mut dest, bare).expect("moves");
    assert!(
        dest.creatures().get(moved).is_none(),
        "a bare entity carries nothing"
    );
    let mut source = World::new_in_area(5, x_area);
    start_into(&f, &mut source).expect("starts");
    let a = ids_of(&source)[0];
    let mut dest = World::new_in_area(5, y_area);
    let expected = AreaError::CombatParticipant { entity: a };
    assert_eq!(transfer_entity(&mut source, &mut dest, a), Err(expected));

    let seeded = || {
        let mut x = World::new_in_area(5, x_area);
        let mut y = World::new_in_area(5, y_area);
        let one = f.register(&mut x, f.placement_a).expect("registers");
        transfer_entity(&mut x, &mut y, one).expect("moves");
        (x, y)
    };
    let assert_collision = |x: &mut World, y: &mut World, entity: EntityId| {
        let before = (bytes(x), bytes(y));
        let expected = AreaError::PlacementInUse {
            placement: f.placement_a,
        };
        assert_eq!(transfer_entity(x, y, entity), Err(expected));
        assert_eq!((bytes(x), bytes(y)), before, "both worlds unchanged");
    };
    let (mut x, mut y) = seeded();
    let two = f.register(&mut x, f.placement_a).expect("free in X");
    assert_collision(&mut x, &mut y, two);
    let (mut x, mut y) = seeded();
    start_into(&f, &mut x).expect("A is a combatant in X");
    let occupant = y.creatures().iter().next().expect("record").0;
    assert_collision(&mut y, &mut x, occupant);
    // T1's combatant half also covers a dead combatant (B8: "dead or alive").
    let (mut x, mut y) = seeded();
    let mut dead_f = Fixture::standard();
    let creature = dead_f.creature_a;
    set_stat(&mut dead_f, creature, "health", 1);
    always_success(&mut dead_f);
    start_into(&dead_f, &mut x).expect("A is a combatant in X");
    let (a, b) = (ids_of(&x)[0], ids_of(&x)[1]);
    act(&mut x, b, dead_f.ability, a);
    assert!(x.combatants().get(a).expect("A").dead());
    let occupant = y.creatures().iter().next().expect("record").0;
    let before = (bytes(&x), bytes(&y));
    assert_eq!(
        transfer_entity(&mut y, &mut x, occupant),
        Err(AreaError::PlacementInUse {
            placement: f.placement_a
        }),
        "a dead combatant still blocks the placement"
    );
    assert_eq!((bytes(&x), bytes(&y)), before);
    let (mut x, mut y) = seeded();
    let two = f.register(&mut x, f.placement_a).expect("registers again");
    let nan = Transform {
        position: [f32::NAN, 0.0, 0.0],
        velocity: [0.0; 3],
    };
    x.transforms_mut().insert(two, nan);
    let before = (bytes(&x), bytes(&y));
    let expected = AreaError::InvalidTransform { entity: two };
    assert_eq!(
        transfer_entity(&mut x, &mut y, two),
        Err(expected),
        "existing check wins"
    );
    assert_eq!((bytes(&x), bytes(&y)), before);
}

/// B9: load checks K1-K12 and P1, with their exact messages.
#[test]
fn b9_load_checks() {
    let mut f = Fixture::standard();
    add_pool(&mut f);
    add_speed(&mut f);
    add_outsider(&mut f, 131, 141);
    add_outsider(&mut f, 132, 142);
    let mut w = World::new(5);
    let one = reg(&f, &mut w, 131);
    let two = reg(&f, &mut w, 132);
    start_into(&f, &mut w).expect("starts");
    let loaded: World = serde_json::from_value(value_of(&w)).expect("round-trips");

    let donor = record(&loaded, one);
    let message = try_with_json(&loaded, |s| {
        s["creatures"]
            .as_array_mut()
            .expect("creatures")
            .push(json!([{"index": 99, "generation": 1}, donor]));
    })
    .expect_err("dangling is refused");
    assert!(
        message.starts_with("dangling creature for non-live "),
        "{message}"
    );
    let combatant = loaded.combatants().iter().next().expect("a combatant").0;
    let message = try_with_json(&loaded, |s| {
        let entry = s["creatures"][0].clone();
        s["creatures"] = json!([[serde_json::to_value(combatant).unwrap(), entry[1].clone()]]);
    })
    .expect_err("creature for a combatant is refused");
    assert_eq!(message, "creature state for a combatant");

    // K1 runs before K2: a dangling record and a combatant-owned record in the
    // same save report the dangling one.
    let message = try_with_json(&loaded, |s| {
        let donor = record(&loaded, one);
        let entries = s["creatures"].as_array_mut().expect("creatures");
        entries[0][0] = serde_json::to_value(combatant).unwrap();
        entries.push(json!([{"index": 99, "generation": 1}, donor]));
    })
    .expect_err("dangling wins over a combatant-owned record");
    assert!(
        message.starts_with("dangling creature for non-live "),
        "{message}"
    );

    macro_rules! check {
        ($edit:expr, $expected:expr) => {{
            assert_eq!(
                try_with_json(&loaded, $edit).expect_err("refused"),
                $expected
            );
        }};
    }
    check!(
        |s| edit_record(s, one, |r| r["max_health"] = json!(0)),
        "creature with a nonpositive maximum"
    );
    check!(
        |s| edit_record(s, one, |r| r["health"] = json!(99)),
        "creature health exceeds its maximum"
    );
    check!(
        |s| edit_record(s, one, |r| r["pools"] = json!([])),
        "creature declares no pools"
    );
    check!(
        |s| edit_record(s, one, |r| {
            let pool = r["pools"][0].clone();
            r["pools"] = json!([pool.clone(), pool]);
        }),
        "creature declares a duplicate pool"
    );
    check!(
        |s| edit_record(s, one, |r| r["fixed"] = json!([["alpha", 98_304]])),
        "creature stats repeat a name"
    );
    check!(
        |s| edit_record(s, one, |r| {
            r["suspended"] = json!([{"effect": uid(201), "remaining_rounds": 2},
            {"effect": uid(200), "remaining_rounds": 2}]);
        }),
        "creature attachments break ascending order"
    );
    check!(
        |s| edit_record(s, one, |r| {
            r["suspended"] = json!([{"effect": uid(200), "remaining_rounds": 2},
            {"effect": uid(200), "remaining_rounds": 2}]);
        }),
        "creature attachments break ascending order"
    );
    check!(
        |s| edit_record(s, one, |r| {
            r["pools"] = json!([]);
            r["suspended"] = json!([{"effect": uid(201), "remaining_rounds": 2},
            {"effect": uid(200), "remaining_rounds": 2}]);
        }),
        "creature declares no pools"
    );
    check!(
        |s| edit_record(s, one, |r| {
            r["suspended"] = json!([{"effect": uid(200), "remaining_rounds": 0}]);
        }),
        "creature attachment carries no remaining rounds"
    );
    check!(
        |s| edit_record(s, two, |r| r["placement"] = json!(uid(131))),
        "creatures share a placement identity"
    );
    check!(
        |s| edit_record(s, one, |r| r["tags"] = json!(["b", "a"])),
        "creature tags break ascending order"
    );
    check!(
        |s| edit_record(s, one, |r| r["tags"] = json!(["a", "a"])),
        "creature tags break ascending order"
    );
    check!(
        |s| edit_record(s, one, |r| r["tags"] = json!(many_tags())),
        "creature tags exceed their bound"
    );

    let (combat, ids) = tagged_speed().start(5);
    let key = serde_json::to_value(ids[0]).unwrap();
    let message = try_with_json(&combat, |s| {
        let entries = s["combatants"].as_array_mut().expect("combatants");
        let entry = entries
            .iter_mut()
            .find(|e| e[0] == key)
            .expect("the entity");
        entry[1]["fixed"] = json!([["speed", 98_304], ["speed", 98_304]]);
    })
    .expect_err("repeated fixed is refused");
    assert_eq!(message, "combatant fixed stats repeat a name");
}

/// B10 (D12): journaled registration.
#[test]
fn b10_history_registration() {
    let f = outsider();
    let mut h = HistoryWorld::new(5);
    let e = f.register_history(&mut h, id(131)).expect("registers");
    assert_eq!(h.version(), 1, "registration never upgrades");
    let page = h.read_after(0, 256).expect("read");
    let payloads: Vec<HistoryEvent> = page.iter().map(|env| env.payload.clone()).collect();
    assert_eq!(payloads, vec![HistoryEvent::Spawned { entity: e }]);
    let mut fresh = HistoryWorld::new(5);
    let before = history_hash(&fresh);
    let bad = spec(&f, id(131), f.creature_b);
    assert!(fresh.register_creature(&bad).is_err());
    assert!(fresh.read_after(0, 256).expect("read").is_empty());
    assert_eq!(history_hash(&fresh), before);
    let loaded: HistoryWorld =
        serde_json::from_value(serde_json::to_value(&h).unwrap()).expect("loads");
    assert_eq!(
        (loaded.version(), history_hash(&loaded)),
        (1, history_hash(&h))
    );
    assert_eq!(
        serde_json::to_vec(&loaded).expect("serializes"),
        serde_json::to_vec(&h).expect("serializes"),
        "a version-1 wrapper with a record round-trips byte-identically"
    );
    // `spawn_with_creature` (the transfer destination spawn) journals one Spawned.
    let mut hsrc = HistoryWorld::new_in_area(5, f.area);
    let hs = f.register_history(&mut hsrc, id(131)).expect("registers");
    let mut hdst = HistoryWorld::new_in_area(5, id(777));
    let moved = transfer_history_entity(&mut hsrc, &mut hdst, hs).expect("moves");
    let page = hdst.read_after(0, 256).expect("read");
    let payloads: Vec<HistoryEvent> = page.iter().map(|env| env.payload.clone()).collect();
    assert_eq!(payloads, vec![HistoryEvent::Spawned { entity: moved }]);
}

/// B11 (D13): lifecycle at start.
#[test]
fn b11_lifecycle_mapping() {
    let cases = [
        (None, None, None, Lifecycle::default()),
        (None, Some(false), None, Lifecycle::default()),
        (
            None,
            Some(true),
            None,
            Lifecycle {
                respawns: true,
                effects_on_exit: EffectsOnExit::Suspend,
                join_after_active: false,
            },
        ),
        (
            Some(CreaturePersistenceWire::Registered),
            None,
            None,
            Lifecycle {
                respawns: true,
                effects_on_exit: EffectsOnExit::Suspend,
                join_after_active: false,
            },
        ),
        (
            Some(CreaturePersistenceWire::Registered),
            Some(false),
            None,
            Lifecycle {
                respawns: true,
                effects_on_exit: EffectsOnExit::Suspend,
                join_after_active: false,
            },
        ),
        (
            None,
            None,
            Some(EffectsOnExitWire::Strip),
            Lifecycle {
                respawns: false,
                effects_on_exit: EffectsOnExit::Strip,
                join_after_active: false,
            },
        ),
        (
            None,
            None,
            Some(EffectsOnExitWire::StripAtRelease),
            Lifecycle {
                respawns: false,
                effects_on_exit: EffectsOnExit::StripAtRelease,
                join_after_active: false,
            },
        ),
        (
            Some(CreaturePersistenceWire::Survivors),
            None,
            Some(EffectsOnExitWire::Suspend),
            Lifecycle::default(),
        ),
    ];
    for (persistence, respawns, effects, expected) in cases {
        let mut f = Fixture::standard();
        set_hooks(&mut f, persistence, None, effects);
        f.encounter.respawns = respawns;
        let (w, _) = f.start(5);
        assert_eq!(w.combat().expect("combat").lifecycle, expected);
        if expected == Lifecycle::default() {
            assert!(
                !json_keys(&value_of(&w)).contains("lifecycle"),
                "no key at defaults"
            );
        }
    }
}

/// B12 (D5, D6): every combatant carries `fixed` and `tags`; the bound runs.
#[test]
fn b12_carriage() {
    let f = tagged_speed();
    let (w, ids) = f.start(5);
    let (a, b) = (
        w.combatants().get(ids[0]).expect("A"),
        w.combatants().get(ids[1]).expect("B"),
    );
    let speed = [("speed".to_owned(), Fx16_16::from_raw(98_304))];
    assert_eq!((a.fixed(), b.fixed()), (&speed[..], &speed[..]));
    assert_eq!(a.tags(), ["a", "b"], "spawned tags sorted and deduplicated");
    assert_eq!(b.tags(), ["solo"]);
    let (plain, ids) = Fixture::standard().start(5);
    let save = value_of(&plain);
    assert!(!json_keys(&save).contains("fixed"));
    let key = serde_json::to_value(ids[0]).unwrap();
    let saved = save["combatants"]
        .as_array()
        .expect("combatants")
        .iter()
        .find(|e| e[0] == key)
        .expect("entity")[1]
        .clone();
    assert!(saved.get("tags").is_none(), "no tags key without tags");
    let mut f = Fixture::standard();
    let creature = f.creature_a;
    set_tags_n(&mut f, creature, MAX_TAGS + 1);
    let mut w = World::new(5);
    start_rej(&f, &mut w, CombatError::TooManyTags);
    let mut f = Fixture::standard();
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("registers first");
    let creature = f.creature_a;
    set_tags_n(&mut f, creature, MAX_TAGS + 1);
    let mut w = with_record(&w, a, |r| r["health"] = json!(0));
    start_rej(&f, &mut w, CombatError::TooManyTags);
}

/// B13: Skip, S4 NoEntrants, S1-S3 and their precedence.
#[test]
fn b13_skip_and_no_entrants() {
    let f = Fixture::standard();
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("A");
    let b = f.register(&mut w, f.placement_b).expect("B");
    let mut w = with_json(&w, |s| {
        edit_record(s, a, |r| r["health"] = json!(0));
        edit_record(s, b, |r| r["health"] = json!(0));
    });
    start_rej(&f, &mut w, CombatError::NoEntrants);
    let f = Fixture::standard();
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("A");
    let b = f.register(&mut w, f.placement_b).expect("B");
    let mut w = with_json(&w, |s| edit_record(s, b, |r| r["health"] = json!(0)));
    let _ = spawned(&mut w);
    start_into(&f, &mut w).expect("starts with one entrant");
    assert_eq!(ids_of(&w), vec![a], "the dead participant is skipped");
    assert!(w.creatures().get(b).expect("skip").dead());
    assert_eq!(w.creatures().len(), 1);
    assert!(w.events().is_empty(), "no spawn for an entrant or a skip");
    let summary = end_encounter(&mut w).expect("release");
    assert_eq!(
        summary.results.len(),
        1,
        "the entrant is listed, the skip is not"
    );
}

/// B13: Skip wins over S1 — a dead record with a mismatched ruleset is
/// skipped, not a `RulesetMismatch`.
#[test]
fn b13_skip_beats_s1() {
    let f = Fixture::standard();
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("A");
    let b = f.register(&mut w, f.placement_b).expect("B");
    let mut w = with_json(&w, |s| {
        edit_record(s, a, |r| {
            r["health"] = json!(0);
            r["ruleset"] = json!(uid(999));
        });
    });
    start_into(&f, &mut w).expect("starts with B only");
    assert_eq!(
        ids_of(&w),
        vec![b],
        "the dead record is skipped, not a mismatch"
    );
    assert!(w.creatures().get(a).expect("dead").dead());
}

#[test]
fn b13_entry_checks() {
    let edits: [fn(&mut Value); 5] = [
        |r| r["ruleset"] = json!(uid(999)),
        |r| {
            r["attributes"].as_array_mut().unwrap().swap(0, 1);
        },
        |r| r["pools"][0]["max"] = json!(5),
        |r| r["pools"][0]["id"] = json!(uid(777)),
        |r| r["pools"][0]["refresh"] = json!({"type": "on_round_start"}),
    ];
    for edit in edits {
        let f = Fixture::standard();
        let mut w = World::new(5);
        let a = f.register(&mut w, f.placement_a).expect("registers");
        let mut w = with_record(&w, a, edit);
        start_rej(&f, &mut w, CombatError::RulesetMismatch { entity: a });
    }
    let mut f = Fixture::standard();
    add_pool(&mut f);
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("registers");
    let mut w = with_record(&w, a, |r| {
        r["pools"].as_array_mut().expect("pools").pop();
    });
    start_rej(&f, &mut w, CombatError::RulesetMismatch { entity: a });
    let mut f = Fixture::standard();
    add_pool(&mut f);
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("registers");
    f.ruleset.pools.pop();
    start_rej(&f, &mut w, CombatError::RulesetMismatch { entity: a });
    let mut f = Fixture::standard();
    add_pool(&mut f);
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("registers");
    let mut w = with_record(&w, a, |r| r["pools"][1]["max"] = json!(4));
    start_rej(&f, &mut w, CombatError::RulesetMismatch { entity: a });
    let mut f = Fixture::standard();
    bad_derived(&mut f, "bad");
    attach_grant(&mut f, 500, &["new-tag"]);
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("registers");
    let many = many_tags();
    let mut w = with_record(&w, a, |r| {
        r["health"] = json!(4);
        r["tags"] = json!(&many[..MAX_TAGS]);
    });
    start_rej(
        &f,
        &mut w,
        CombatError::InvalidStatValue { stat: "bad".into() },
    );
    let mut w = with_record(&w, a, |r| r["ruleset"] = json!(uid(999)));
    start_rej(&f, &mut w, CombatError::RulesetMismatch { entity: a });
    let mut f = Fixture::standard();
    attach_grant(&mut f, 400, &["new-tag"]);
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("registers");
    let many = many_tags();
    let mut w = with_record(&w, a, |r| r["tags"] = json!(&many[..MAX_TAGS]));
    start_rej(&f, &mut w, CombatError::TooManyTags);
    let mut f = Fixture::standard();
    attach_grant(&mut f, 400, &["t00000"]);
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("registers");
    let many = many_tags();
    let mut w = with_record(&w, a, |r| r["tags"] = json!(&many[..MAX_TAGS]));
    start_into(&f, &mut w).expect("the grant is already among the tags");
    let mut f = Fixture::standard();
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("registers");
    let mut w = with_record(&w, a, |r| r["ruleset"] = json!(uid(999)));
    f.placements.remove(&f.placement_b);
    start_rej(
        &f,
        &mut w,
        CombatError::MissingPlacement {
            placement: f.placement_b,
        },
    );
}

/// B14, B15: an entrant is seeded from its record, then the spawn body runs.
#[test]
fn b14_mixed_start_seeding() {
    let mut f = Fixture::standard();
    add_pool(&mut f);
    add_speed(&mut f);
    let creature = f.creature_a;
    set_tags(&mut f, creature, &["b", "a", "a"]);
    let creature_b = f.creature_b;
    set_tags(&mut f, creature_b, &["prefab"]);
    attach_grant(&mut f, 200, &["granted"]);
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("registers");
    let mut w = with_record(&w, a, |r| {
        r["health"] = json!(4);
        r["pools"][0]["current"] = json!(0);
        r["pools"][1]["current"] = json!(1);
        r["tags"] = json!(["c"]);
        r["suspended"] = json!([{"effect": uid(200), "remaining_rounds": 2},
            {"effect": uid(300), "remaining_rounds": 1}]);
    });
    let _ = spawned(&mut w);
    start_into(&f, &mut w).expect("starts");
    let ids = ids_of(&w);
    assert_eq!(ids[0], a, "store order: entered A, then new B");
    assert!(w.creatures().is_empty(), "no record survives entry");
    assert_eq!(timeline_of(&w), vec![(9, ids[1]), (10, ids[0])]);
    assert_eq!(w.combat().expect("combat").active, Some(ids[1]));
    assert_eq!(spawned(&mut w), vec![ids[1]], "only B spawns");
    let a = w.combatants().get(a).expect("A");
    assert_eq!((a.health(), a.max_health()), (4, 10));
    assert_eq!(
        (a.action_pool().current(), a.action_pool().max()),
        (0, 1),
        "unrefreshed"
    );
    let extra = &a.extra_pools()[0];
    assert_eq!((extra.current(), extra.max()), (1, 3), "unrefreshed");
    assert_eq!(
        a.attached(),
        [AttachedEffect {
            effect: id(200),
            expires_round: 2
        }]
    );
    assert!(a.entered());
    assert_eq!(a.fixed(), [("speed".to_owned(), Fx16_16::from_raw(98_304))]);
    assert_eq!(a.tags(), ["c"], "the record's tags, not a grant");
    let b = w.combatants().get(ids[1]).expect("B");
    assert!(!b.entered());
    assert_eq!(b.health(), 6);
    assert_eq!(b.tags(), ["prefab"], "a grant is never carried");
}

/// B15: initiative draws use the record; a skip draws nothing.
#[test]
fn b15_initiative_uses_record_and_skip_draws_nothing() {
    let mut f = Fixture::standard();
    set_initiative_rule(&mut f, "1d6", &["alpha"]);
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("registers");
    let mut w = with_record(&w, a, |r| r["attributes"][0][1] = json!(50));
    start_into(&f, &mut w).expect("starts");
    let mut expected = oracle(5);
    let roll_a = draw(&mut expected, "1d6").total;
    let roll_b = draw(&mut expected, "1d6").total;
    assert_eq!(
        *w.rng_mut(),
        expected,
        "one draw per Spawn and Enter participant"
    );
    assert_eq!(
        w.combatants().get(a).expect("A").initiative(),
        roll_a + 50 + 10
    );
    assert_eq!(
        w.combatants().get(ids_of(&w)[1]).expect("B").initiative(),
        roll_b + 6 + 9
    );
    let mut f = Fixture::standard();
    set_initiative_rule(&mut f, "1d6", &["alpha"]);
    let mut w = World::new(5);
    f.register(&mut w, f.placement_a).expect("A");
    let b = f.register(&mut w, f.placement_b).expect("B");
    let mut w = with_json(&w, |s| edit_record(s, b, |r| r["health"] = json!(0)));
    start_into(&f, &mut w).expect("starts");
    let mut expected = oracle(5);
    draw(&mut expected, "1d6");
    assert_eq!(*w.rng_mut(), expected, "the skip draws nothing");
}

/// B16, B17: write-back at release; the summary and event behaviour.
#[test]
fn b16_b17_writeback_release() {
    let mut f = tagged_speed();
    add_pool(&mut f);
    let (mut w, ids) = f.start(9);
    let (a, b) = (ids[0], ids[1]);
    act(&mut w, b, f.ability, a);
    act(&mut w, a, f.ability, b);
    let (a_health, b_health) = (
        w.combatants().get(a).expect("A").health(),
        w.combatants().get(b).expect("B").health(),
    );
    let a_pool = w.combatants().get(a).expect("A").action_pool().current();
    let b_pool = w.combatants().get(b).expect("B").action_pool().current();
    let events = w.events().len();
    let summary = end_encounter(&mut w).expect("release");
    assert_eq!(
        (summary.encounter, summary.ruleset),
        (f.encounter.id, f.ruleset.id)
    );
    assert_eq!(summary.results.len(), 2, "every combatant, no skip");
    assert!(w.combat().is_none() && w.combatants().is_empty());
    assert!(w.contains(a) && w.contains(b), "entities stay live");
    assert!(
        w.timeline().is_empty(),
        "release removes every timeline entry"
    );
    assert_eq!(w.events().len(), events, "release emits no event");
    let (ra, rb) = (
        w.creatures().get(a).expect("A"),
        w.creatures().get(b).expect("B"),
    );
    assert_eq!(
        (ra.placement(), rb.placement()),
        (f.placement_a, f.placement_b)
    );
    assert_eq!(ra.ruleset(), f.ruleset.id);
    assert_eq!((ra.health(), ra.max_health()), (a_health, 10));
    assert_eq!((rb.health(), rb.max_health()), (b_health, 6));
    assert_eq!(
        ra.fixed(),
        [("speed".to_owned(), Fx16_16::from_raw(98_304))]
    );
    assert_eq!(ra.tags(), ["a", "b"]);
    assert_eq!(rb.tags(), ["solo"]);
    assert_eq!(
        (ra.pools()[0].current(), rb.pools()[0].current()),
        (a_pool, b_pool)
    );
    assert_eq!(ra.pools().len(), 2, "primary then extras, in order");
    assert_eq!(ra.pools()[1].id(), ResourcePoolId(id(106)));
    assert_eq!(ra.pools()[1].current(), 3);
    assert!(ra.suspended().is_empty());

    let f = tagged_speed();
    let (w, ids) = f.start(5);
    let a = ids[0];
    let mut w = with_json(&w, |s| {
        edit_combatant(s, a, |c| c["tags"] = json!(["z", "a", "z"]))
    });
    end_encounter(&mut w).expect("release");
    assert_eq!(
        w.creatures().get(a).expect("A").tags(),
        ["a", "z"],
        "write-back sorts"
    );
    serde_json::from_value::<World>(value_of(&w)).expect("the save reloads (K11)");
}

/// B16: `entered || !respawns` governs write-back.
#[test]
fn b16_respawns_and_entered_writeback() {
    let mut f = Fixture::standard();
    f.encounter.respawns = Some(true);
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("registers");
    start_into(&f, &mut w).expect("starts");
    let b = *ids_of(&w).iter().find(|id| **id != a).expect("B");
    end_encounter(&mut w).expect("release");
    assert!(
        w.creatures().get(a).is_some(),
        "the entrant is written back"
    );
    assert!(w.creatures().get(b).is_none(), "the spawned one is not");
    assert!(
        w.contains(b),
        "a combatant not written back stays a live entity"
    );
    let mut f = Fixture::standard();
    set_hooks(
        &mut f,
        Some(CreaturePersistenceWire::Registered),
        None,
        None,
    );
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("registers");
    start_into(&f, &mut w).expect("starts");
    end_encounter(&mut w).expect("release");
    assert!(w.creatures().get(a).is_some());
    assert_eq!(w.creatures().len(), 1, "only the entrant is written back");
    let f = Fixture::standard();
    let mut w = World::new(5);
    f.register(&mut w, f.placement_a).expect("registers");
    start_into(&f, &mut w).expect("starts");
    end_encounter(&mut w).expect("release");
    assert_eq!(w.creatures().len(), 2, "default: both are written back");
}

/// B16: an entered combatant is written back even when the lifecycle respawns
/// (`entered || !respawns`, the `entered && respawns` branch).
#[test]
fn b16_entered_written_back_under_respawns() {
    let f = Fixture::standard();
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("registers");
    start_into(&f, &mut w).expect("starts");
    assert!(w.combatants().get(a).expect("A").entered(), "A entered");
    let b = *ids_of(&w).iter().find(|id| **id != a).expect("B");
    let mut w = with_json(&w, |s| s["combat"]["lifecycle"] = json!({"respawns": true}));
    end_encounter(&mut w).expect("release");
    let ra = w.creatures().get(a).expect("the entrant is written back");
    assert_eq!(ra.placement(), f.placement_a);
    assert_eq!(ra.ruleset(), f.ruleset.id);
    assert_eq!((ra.health(), ra.max_health()), (10, 10));
    assert!(w.creatures().get(b).is_none(), "the spawned one is not");
    assert!(w.contains(b), "and stays a live entity");
}

/// B16: `effects_on_exit` decides what attached effects become at release.
#[test]
fn b16_effects_at_release() {
    let setup = |f: &mut Fixture, w: &mut World, remaining: u32| -> EntityId {
        attach_grant(f, 200, &["granted"]);
        let placement = f.placement_a;
        let a = f.register(w, placement).expect("registers");
        *w = with_record(w, a, |r| {
            r["suspended"] = json!([{"effect": uid(200), "remaining_rounds": remaining}]);
        });
        start_into(f, w).expect("starts");
        a
    };
    let two_turns = |w: &mut World| {
        let first = w.combat().expect("combat").active.expect("turn");
        end(w, first);
        let second = w.combat().expect("combat").active.expect("turn");
        end(w, second);
        assert_eq!(w.combat().expect("combat").round, 1);
    };
    let mut f = Fixture::standard();
    let mut w = World::new(5);
    let a = setup(&mut f, &mut w, 3);
    two_turns(&mut w);
    end_encounter(&mut w).expect("release");
    let expected = [SuspendedEffect {
        effect: id(200),
        remaining_rounds: 2,
    }];
    assert_eq!(w.creatures().get(a).expect("A").suspended(), expected);
    let mut f = Fixture::standard();
    let mut w = World::new(5);
    let a = setup(&mut f, &mut w, 1);
    two_turns(&mut w);
    end_encounter(&mut w).expect("release");
    assert!(
        w.creatures().get(a).expect("A").suspended().is_empty(),
        "expiry not after r"
    );
    for hook in [EffectsOnExitWire::Strip, EffectsOnExitWire::StripAtRelease] {
        let mut f = Fixture::standard();
        set_hooks(&mut f, None, None, Some(hook));
        let mut w = World::new(5);
        let a = setup(&mut f, &mut w, 3);
        end_encounter(&mut w).expect("release");
        assert!(w.creatures().get(a).expect("A").suspended().is_empty());
    }
}

/// B17: release removes every combatant's timeline entry, including one that
/// is not written back (a `respawns` spawn): the timeline is exactly empty.
#[test]
fn b17_release_clears_timeline_without_writeback() {
    // Every combatant spawned under `respawns`: none is written back.
    let mut f = Fixture::standard();
    f.encounter.respawns = Some(true);
    let mut w = World::new(5);
    start_into(&f, &mut w).expect("starts");
    let ids = ids_of(&w);
    assert_eq!(timeline_of(&w), vec![(9, ids[1]), (10, ids[0])]);
    end_encounter(&mut w).expect("release");
    assert!(w.creatures().is_empty(), "nothing is written back");
    assert!(
        !w.timeline().contains(ids[0]) && !w.timeline().contains(ids[1]),
        "each spawned combatant's entry is gone"
    );
    assert!(w.timeline().is_empty(), "the timeline is exactly empty");

    // Mixed: an entered combatant beside a spawned one (the
    // b16_entered_written_back_under_respawns shape).
    let f = Fixture::standard();
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("registers");
    start_into(&f, &mut w).expect("starts");
    let b = *ids_of(&w).iter().find(|id| **id != a).expect("B");
    let mut w = with_json(&w, |s| s["combat"]["lifecycle"] = json!({"respawns": true}));
    assert_eq!(timeline_of(&w), vec![(9, b), (10, a)]);
    end_encounter(&mut w).expect("release");
    assert!(
        w.creatures().get(a).is_some(),
        "the entrant is written back"
    );
    assert!(w.creatures().get(b).is_none(), "the spawned one is not");
    assert!(
        !w.timeline().contains(b),
        "the non-written-back combatant's entry is gone too"
    );
    assert!(w.timeline().is_empty(), "the timeline is exactly empty");
}

/// B18: the dead stay dead; only despawn removes a dead record.
#[test]
fn b18_dead_stay_dead() {
    let f = Fixture::standard();
    let mut w = World::new(5);
    let a = f.register(&mut w, f.placement_a).expect("A");
    let b = f.register(&mut w, f.placement_b).expect("B");
    let mut w = with_json(&w, |s| edit_record(s, a, |r| r["health"] = json!(0)));
    start_into(&f, &mut w).expect("starts with B only");
    assert_eq!(ids_of(&w), vec![b]);
    assert!(w.creatures().get(a).expect("dead").dead());
    end_encounter(&mut w).expect("release");
    assert!(w.creatures().get(b).expect("B").health() > 0);
    assert!(w.creatures().get(a).expect("dead").dead());
    w.despawn(a);
    assert!(w.creatures().get(a).is_none());
    start_into(&f, &mut w).expect("A spawns fresh");
    let ids = ids_of(&w);
    assert_eq!(ids.len(), 2, "A spawns fresh, B re-enters");
    assert!(!ids.contains(&a), "the old dead id is not reused");
    let mut f = Fixture::standard();
    let creature = f.creature_a;
    set_stat(&mut f, creature, "health", 1);
    always_success(&mut f);
    let (mut w, ids) = f.start(3);
    let (a, b) = (ids[0], ids[1]);
    act(&mut w, b, f.ability, a);
    assert!(w.combatants().get(a).expect("A").dead());
    end_encounter(&mut w).expect("release");
    assert_eq!(w.creatures().get(a).expect("A").health(), 0);
    assert!(w.creatures().get(a).expect("A").dead());
}

/// B19: every operation is all-or-nothing; equal scripts give equal hashes.
#[test]
fn b19_determinism() {
    fn run(seed: u64) -> Vec<[u8; 32]> {
        let f = tagged_speed();
        let mut w = World::new(seed);
        let mut hashes = vec![state_hash(&w)];
        f.register(&mut w, f.placement_a).expect("registers");
        hashes.push(state_hash(&w));
        ticks(&mut w, 2);
        hashes.push(state_hash(&w));
        start_into(&f, &mut w).expect("starts");
        hashes.push(state_hash(&w));
        let ids = ids_of(&w);
        act(&mut w, ids[1], f.ability, ids[0]);
        hashes.push(state_hash(&w));
        act(&mut w, ids[0], f.ability, ids[1]);
        hashes.push(state_hash(&w));
        end_encounter(&mut w).expect("release");
        hashes.push(state_hash(&w));
        start_into(&f, &mut w).expect("restarts");
        hashes.push(state_hash(&w));
        end_encounter(&mut w).expect("release");
        hashes.push(state_hash(&w));
        hashes
    }
    assert_eq!(run(11), run(11), "equal per-step hashes");
    let f = tagged_speed();
    let mut w = World::new(12);
    f.register(&mut w, f.placement_a).expect("registers");
    start_into(&f, &mut w).expect("starts");
    end_encounter(&mut w).expect("release");
    let reloaded: World = serde_json::from_value(value_of(&w)).expect("reloads");
    assert_eq!(state_hash(&reloaded), state_hash(&w));
}

/// B20: history at start and release; records never upgrade the version.
#[test]
fn b20_history_at_start_and_release() {
    let f = Fixture::standard();
    let mut h = HistoryWorld::new(5);
    let a = f
        .register_history(&mut h, f.placement_a)
        .expect("registers");
    f.with_spec(|s| h.start_encounter_in_mode(s, EncounterMode::TurnBased))
        .expect("starts");
    let page = h.read_after(0, 256).expect("read");
    let payloads: Vec<HistoryEvent> = page.iter().map(|env| env.payload.clone()).collect();
    assert_eq!(
        payloads.len(),
        3,
        "registration, start Spawned, TurnStarted"
    );
    assert!(matches!(payloads[0], HistoryEvent::Spawned { entity } if entity == a));
    assert!(matches!(payloads[1], HistoryEvent::Spawned { entity } if entity != a));
    assert!(matches!(payloads[2], HistoryEvent::TurnStarted { .. }));
    assert_eq!(h.version(), 1, "records never upgrade");
    h.end_encounter().expect("release");
    assert_eq!(
        h.read_after(0, 256).expect("read").len(),
        3,
        "release journals nothing"
    );
    assert_eq!(h.version(), 1);
}
