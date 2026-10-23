//! Real-time persistence, load coherence and despawn (T138a, ADR-0029 T16):
//! R14.
//!
//! Exact bytes are pinned for the new shapes, and every load rule L1-L5 is
//! exercised by editing the raw JSON of a valid save.

#[allow(dead_code)]
mod support;

use crpg_sim::{
    end_encounter, legal_actions, perform_action, request_mode, state_hash, tick, CombatError,
    EncounterMode, LegalActionsError, World, RT_CLOCK_MAX,
};
use support::rt::{
    bytes_of, capable, capable_trio, edit_ability, elapsed, pool_of, ticks, timeline_of,
    use_action, value_of,
};
use support::Fixture;

const COMBATANT_WITH_ORDER: &str = concat!(
    r#"{"placement":"0000000000000000000000003F","initiative":10,"#,
    r#""attributes":[["alpha",8],["beta",6],["gamma",7]],"health":10,"max_health":10,"#,
    r#""dead":false,"action_pool":{"id":"00000000000000000000000039","max":1,"current":1,"#,
    r#""refresh":{"type":"on_turn_start"}},"#,
    r#""order":{"ability":"00000000000000000000000037","target":{"index":1,"generation":1},"#,
    r#""repeat":false}}"#
);

/// R14 (B33): exact bytes of an RT `CombatState` and of an ordered combatant.
#[test]
fn r14_exact_bytes() {
    let fixture = capable(4);
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1])).expect("order");
    let order_bytes =
        serde_json::to_string(world.combatants().get(ids[0]).expect("A")).expect("serialize");
    assert_eq!(order_bytes, COMBATANT_WITH_ORDER);
    let idle =
        serde_json::to_string(world.combatants().get(ids[1]).expect("B")).expect("serialize");
    assert!(!idle.contains("order"), "no order, no key");

    let combat = world.combat().expect("encounter");
    let definition = serde_json::to_string(&combat.definition).expect("serialize");
    assert_eq!(
        serde_json::to_string(combat).expect("serialize"),
        format!(
            r#"{{"definition":{definition},"round":0,"active":null,"mode":"real_time","clock":{{"round_ticks":4,"elapsed":0}}}}"#
        )
    );
    request_mode(&mut world, EncounterMode::TurnBased).expect("request");
    let combat = world.combat().expect("encounter");
    assert_eq!(
        serde_json::to_string(combat).expect("serialize"),
        format!(
            r#"{{"definition":{definition},"round":0,"active":null,"mode":"real_time","clock":{{"round_ticks":4,"elapsed":0}},"pending_mode":"turn_based"}}"#
        )
    );

    // A TB state of an RT-capable fight omits the mode; a TB-only one the lot.
    let (tb, _) = fixture.start(1);
    let combat = tb.combat().expect("encounter");
    assert_eq!(
        serde_json::to_string(combat).expect("serialize"),
        format!(
            r#"{{"definition":{definition},"round":0,"active":{{"index":1,"generation":1}},"clock":{{"round_ticks":4,"elapsed":0}}}}"#
        )
    );
    let (only, _) = Fixture::standard().start(1);
    let combat = only.combat().expect("encounter");
    let definition = serde_json::to_string(&combat.definition).expect("serialize");
    assert_eq!(
        serde_json::to_string(combat).expect("serialize"),
        format!(r#"{{"definition":{definition},"round":0,"active":{{"index":1,"generation":1}}}}"#)
    );
}

/// R14 (B33): save then load is the identity, mid-fight, in both modes.
#[test]
fn r14_round_trip() {
    let mut fixture = capable_trio(4);
    edit_ability(&mut fixture, |ability| {
        ability.recovery_ticks = Some(2);
        ability.repeats_in_real_time = Some(true);
    });
    let (mut world, ids) = fixture.start_rt(5, EncounterMode::RealTime);
    perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1])).expect("order");
    perform_action(&mut world, &use_action(ids[2], fixture.ability, ids[0])).expect("order");
    ticks(&mut world, 3);
    request_mode(&mut world, EncounterMode::TurnBased).expect("request");
    let saved = serde_json::to_string(&world).expect("serialize");
    let mut loaded: World = serde_json::from_str(&saved).expect("an RT save loads");
    assert_eq!(loaded, world);
    assert_eq!(state_hash(&loaded), state_hash(&world));
    assert_eq!(serde_json::to_string(&loaded).expect("serialize"), saved);
    // The loaded world keeps behaving identically.
    for _ in 0..10 {
        tick(&mut world);
        tick(&mut loaded);
        assert_eq!(state_hash(&world), state_hash(&loaded));
    }
    assert_eq!(
        loaded.combat().expect("encounter").mode,
        EncounterMode::TurnBased
    );
    let saved = serde_json::to_string(&loaded).expect("serialize");
    assert_eq!(
        serde_json::from_str::<World>(&saved).expect("TB save"),
        loaded
    );
}

/// Asserts that loading `value` fails with `message`.
fn assert_rejects(value: serde_json::Value, message: &str) {
    let error = serde_json::from_value::<World>(value).expect_err("the edited save must not load");
    assert!(
        error.to_string().contains(message),
        "expected `{message}`, got `{error}`"
    );
}

fn two_ability_capable() -> Fixture {
    let mut fixture = capable(4);
    let base = fixture.abilities[&fixture.ability].clone();
    let mut second = base;
    second.id = crpg_core::Ulid::from_u128(201);
    second.slug = "probe-ability-201".to_owned();
    second.name = "probe.ability-201".to_owned();
    fixture.ruleset.abilities.push(second.id);
    fixture.abilities.insert(second.id, second);
    fixture
}

/// R14 (B33, L1): RT fields without a clock.
#[test]
fn r14_l1_fields_without_a_clock() {
    let (only, _) = Fixture::standard().start(1);
    let mut value = value_of(&only);
    value["combat"]["mode"] = serde_json::json!("real_time");
    assert_rejects(value, "combat real-time mode without a clock");

    let mut value = value_of(&only);
    value["combat"]["pending_mode"] = serde_json::json!("real_time");
    assert_rejects(value, "combat pending mode without a clock");

    // Vector-shaped definition from a TB-only two-ability ruleset.
    let mut fixture = two_ability_capable();
    fixture.ruleset.round_ticks = None;
    let (world, _) = fixture.start(1);
    let valid = value_of(&world);
    assert!(valid["combat"]["definition"]["abilities"].is_array());
    assert!(
        serde_json::from_value::<World>(valid.clone()).is_ok(),
        "valid control"
    );
    let mut value = valid.clone();
    value["combat"]["definition"]["abilities"][0]["recovery_ticks"] = serde_json::json!(3);
    assert_rejects(
        value,
        "combat ability carries real-time fields without a clock",
    );
    let mut value = valid;
    value["combat"]["definition"]["abilities"][1]["repeats_in_real_time"] = serde_json::json!(true);
    assert_rejects(
        value,
        "combat ability carries real-time fields without a clock",
    );
}

/// R14 (B33, L2, L3): a bad clock, zero recovery, and a pending mode equal to
/// the current one.
#[test]
fn r14_l2_l3_clock_and_pending() {
    let fixture = two_ability_capable();
    let (world, _) = fixture.start(1);
    let valid = value_of(&world);
    assert!(
        serde_json::from_value::<World>(valid.clone()).is_ok(),
        "valid control"
    );

    let mut value = valid.clone();
    value["combat"]["clock"]["round_ticks"] = serde_json::json!(0);
    assert_rejects(value, "combat clock declares zero round ticks");

    let mut value = valid.clone();
    value["combat"]["clock"]["elapsed"] = serde_json::json!(RT_CLOCK_MAX + 1);
    assert_rejects(value, "combat clock exceeds its bound");
    let mut value = valid.clone();
    value["combat"]["clock"]["elapsed"] = serde_json::json!(RT_CLOCK_MAX);
    assert!(
        serde_json::from_value::<World>(value).is_ok(),
        "the bound itself loads"
    );

    let mut value = valid.clone();
    value["combat"]["definition"]["abilities"][0]["recovery_ticks"] = serde_json::json!(0);
    assert_rejects(value, "combat ability declares zero recovery ticks");

    let mut value = valid.clone();
    value["combat"]["pending_mode"] = serde_json::json!("turn_based");
    assert_rejects(value, "combat pending mode equals its mode");
    let mut value = valid;
    value["combat"]["pending_mode"] = serde_json::json!("real_time");
    assert!(
        serde_json::from_value::<World>(value).is_ok(),
        "other mode is fine"
    );

    let (rt, _) = capable(4).start_rt(1, EncounterMode::RealTime);
    let mut value = value_of(&rt);
    value["combat"]["pending_mode"] = serde_json::json!("real_time");
    assert_rejects(value, "combat pending mode equals its mode");
}

/// R14 (B33, L4): the shape of an RT timeline.
#[test]
fn r14_l4_rt_shape() {
    let (world, _) = capable(4).start_rt(1, EncounterMode::RealTime);
    let valid = value_of(&world);
    assert!(
        serde_json::from_value::<World>(valid.clone()).is_ok(),
        "valid control"
    );

    let mut value = valid.clone();
    let head = value["timeline"][0][1].clone();
    value["combat"]["active"] = head;
    assert_rejects(value, "real-time combat holds an active turn");

    let mut value = valid.clone();
    value["timeline"].as_array_mut().expect("timeline").pop();
    assert_rejects(
        value,
        "real-time timeline disagrees with its live combatants",
    );

    let mut value = valid;
    value["timeline"][0][0] = serde_json::json!(-1);
    assert_rejects(value, "real-time timeline key is negative");
}

/// R14 (B33, L5): the rules for standing orders.
#[test]
fn r14_l5_orders() {
    let fixture = capable_trio(4);
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1])).expect("order");
    let valid = value_of(&world);
    assert!(
        serde_json::from_value::<World>(valid.clone()).is_ok(),
        "valid control"
    );
    let ordered = |value: &mut serde_json::Value| -> usize {
        value["combatants"]
            .as_array()
            .expect("pairs")
            .iter()
            .position(|pair| pair[1].get("order").is_some())
            .expect("one combatant holds the order")
    };

    // A TB fight with an order.
    let (tb, _) = capable_trio(4).start(1);
    let mut value = value_of(&tb);
    value["combatants"][0][1]["order"] =
        valid["combatants"][ordered(&mut valid.clone())][1]["order"].clone();
    assert_rejects(value, "turn-based combatant holds a standing order");

    // A dead holder (kept off the timeline so only L5 can object).
    let mut value = valid.clone();
    let index = ordered(&mut value);
    let id = value["combatants"][index][0].clone();
    value["combatants"][index][1]["health"] = serde_json::json!(0);
    value["combatants"][index][1]["dead"] = serde_json::json!(true);
    let timeline = value["timeline"].as_array_mut().expect("timeline");
    timeline.retain(|entry| entry[1] != id);
    assert_rejects(value, "dead combatant holds a standing order");

    // An unlisted ability.
    let mut value = valid.clone();
    let index = ordered(&mut value);
    value["combatants"][index][1]["order"]["ability"] =
        serde_json::json!(support::rt::ulid_text(999));
    assert_rejects(value, "standing order names an unknown ability");

    // A repeat flag that disagrees with the ability.
    let mut value = valid;
    let index = ordered(&mut value);
    value["combatants"][index][1]["order"]["repeat"] = serde_json::json!(true);
    assert_rejects(value, "standing order repeat disagrees with its ability");
}

/// R14 (B33): an order's target is not checked at load; a despawned target
/// fails at due time instead.
#[test]
fn r14_order_target_is_not_checked_at_load() {
    let fixture = capable(4);
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1])).expect("order");
    let mut value = value_of(&world);
    let index = value["combatants"]
        .as_array()
        .expect("pairs")
        .iter()
        .position(|pair| pair[1].get("order").is_some())
        .expect("ordered");
    value["combatants"][index][1]["order"]["target"] =
        serde_json::json!({"index": 77, "generation": 1});
    let mut loaded: World = serde_json::from_value(value).expect("loads");
    tick(&mut loaded);
    assert!(loaded
        .combatants()
        .get(ids[0])
        .expect("A")
        .order()
        .is_none());
    assert_eq!(pool_of(&loaded, ids[0]), 1, "no spend");
}

/// R14 (B34): RT `despawn` removes the entry and the order, never advances
/// turns, and leaves other combatants' orders to fail at due time.
#[test]
fn r14_rt_despawn() {
    let fixture = capable_trio(4);
    let (mut world, ids) = fixture.start_rt(1, EncounterMode::RealTime);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    perform_action(&mut world, &use_action(a, fixture.ability, c)).expect("A's order");
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B's order");
    perform_action(&mut world, &use_action(c, fixture.ability, a)).expect("C's order");

    // Despawn the target C: its entry and order go; A's order stays.
    assert!(world.despawn(c));
    let combat = world.combat().expect("encounter");
    assert_eq!((combat.active, combat.round), (None, 0), "no advance in RT");
    assert_eq!(timeline_of(&world), vec![(0, a), (0, b)]);
    assert_eq!(world.combatants().len(), 2);
    assert!(world.combatants().get(a).expect("A").order().is_some());
    tick(&mut world);
    assert!(
        world.combatants().get(a).expect("A").order().is_none(),
        "A's order fails at due time"
    );
    assert_eq!(pool_of(&world, a), 1, "and costs nothing");
    assert_eq!(pool_of(&world, b), 0, "B's order, targeting A, ran");

    // Despawn the holder: a dead or absent holder never executes. One
    // combatant is left, which is terminal by derivation.
    ticks(&mut world, 3); // the tick-4 boundary refills B's pool
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("order");
    assert!(world.despawn(b));
    assert_eq!(timeline_of(&world), vec![(0, a)]);
    let frozen = value_of(&world);
    ticks(&mut world, 9);
    let mut after = value_of(&world);
    after["tick"] = frozen["tick"].clone();
    assert_eq!(frozen, after, "terminal: nothing moves");
    assert_eq!(elapsed(&world), 4);
    assert_eq!(
        legal_actions(&world, a),
        Err(LegalActionsError::Actor(CombatError::OutOfTurn {
            actor: a,
            active: None
        }))
    );
    // The last combatant releases the encounter.
    assert!(world.despawn(a));
    assert!(world.combat().is_none());
    let bytes = bytes_of(&world);
    assert!(end_encounter(&mut world).is_err());
    assert_eq!(bytes, bytes_of(&world));
}
