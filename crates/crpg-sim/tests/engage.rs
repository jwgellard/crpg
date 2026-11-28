//! T157c: engagement rules, `engagement_due` and the legacy definition shape
//! (ADR-0029 Amendment 1 S3; contract S-C1 to S-C3; tests SC1 to SC5).
//!
//! `move_rt()` is the contract's MOVE_RT: the standard two-combatant fixture
//! with the movement edits of `movement_core_contract.rs`'s `move_fixture()`
//! (pace stat, pool 106 of 6, a `movement` block; A paces 1 m a tick, B 0.5 m
//! a tick, B stands at (10, 0, 0)), `round_ticks = 4`, A (creature 121) in
//! faction F1, B (creature 122) in F2, and one `proximity` rule of 2 m.
//! Every expected value is worked out by hand from the contract.
//!
//! SC6 (the survivors of a finished turn-based fight) is appended to
//! `movement_core_contract.rs`, as the contract says; SC7 is that file's and
//! `movement_core.rs`'s two changed assertions.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_data::EngageTriggerWire;
use crpg_sim::{
    engagement_due, fight_is_live, perform_action, request_mode, seg_len, start_encounter_in_mode,
    start_encounter_with_members, state_hash, tick, CombatDefinition, CombatError, EncounterMode,
    EngageRule, EntityMeta, Entrant, MoveRoute, NavRevision, RouteTag, World,
};
use serde_json::{json, Value};
use support::creature::{add_outsider, enlist, try_with_json, with_json, with_record};
use support::draws::set_initiative_rule;
use support::rt::{bytes_of, make_lethal, use_action};
use support::surface::{always_success, set_stat_raw};
use support::{uid, Fixture};

const METRE: i32 = 65_536;
const LEGACY_KEYS: [&str; 15] = [
    "encounter",
    "ruleset",
    "ability",
    "dice",
    "attribute",
    "health_stat",
    "attribute_names",
    "damage",
    "cost",
    "requires_target",
    "allow_self_target",
    "outcome_table",
    "pool_id",
    "pool_max",
    "pool_refresh",
];

fn f1() -> Ulid {
    Ulid::from_u128(131)
}

fn f2() -> Ulid {
    Ulid::from_u128(132)
}

fn pl(value: u128) -> Ulid {
    Ulid::from_u128(value)
}

fn p(x: i32, z: i32) -> [Fx16_16; 3] {
    [Fx16_16::from_int(x), Fx16_16::ZERO, Fx16_16::from_int(z)]
}

fn rev() -> NavRevision {
    NavRevision::of_bytes(b"mesh-a")
}

fn route(tag: u64, points: Vec<[Fx16_16; 3]>) -> MoveRoute {
    MoveRoute {
        revision: rev(),
        tag: RouteTag(tag),
        points,
    }
}

fn proximity(raw: i32) -> EngageTriggerWire {
    EngageTriggerWire::Proximity {
        distance: Fx16_16::from_raw(raw),
    }
}

fn rule(raw: i32) -> EngageRule {
    EngageRule::Proximity {
        distance: Fx16_16::from_raw(raw),
    }
}

fn faction(fx: &mut Fixture, creature: u128, which: Option<Ulid>) {
    fx.creatures
        .get_mut(&pl(creature))
        .expect("the creature exists")
        .faction = which;
}

fn set_position(fx: &mut Fixture, placement: u128, position: [Fx16_16; 3]) {
    fx.placements
        .get_mut(&pl(placement))
        .expect("the placement exists")
        .0
        .transform
        .position = position;
}

/// The movement edits of `movement_core_contract.rs`'s `move_fixture()`,
/// without its outsider.
fn movable() -> Fixture {
    let mut fx = Fixture::standard();
    let mut ruleset = serde_json::to_value(&fx.ruleset).expect("the ruleset serializes");
    ruleset["stats"]
        .as_array_mut()
        .expect("stats")
        .push(json!({"name": "pace", "kind": "fixed"}));
    ruleset["pools"]
        .as_array_mut()
        .expect("pools")
        .push(json!({"id": uid(106), "max": 6, "refresh": {"type": "on_turn_start"}}));
    ruleset["movement"] = json!({"speed": {"type": "stat", "value": "pace"},
        "turn": {"pool": uid(106), "step": {"type": "constant", "value": 98304}, "step_cost": 1}});
    fx.ruleset = serde_json::from_value(ruleset).expect("the ruleset loads");
    set_stat_raw(&mut fx, pl(121), "pace", METRE);
    set_stat_raw(&mut fx, pl(122), "pace", METRE / 2);
    set_position(
        &mut fx,
        112,
        [Fx16_16::from_int(10), Fx16_16::ZERO, Fx16_16::ZERO],
    );
    fx
}

/// The contract's MOVE_RT.
fn move_rt() -> Fixture {
    let mut fx = movable();
    fx.ruleset.round_ticks = Some(4);
    faction(&mut fx, 121, Some(f1()));
    faction(&mut fx, 122, Some(f2()));
    fx.encounter.engage = Some(vec![proximity(2 * METRE)]);
    fx
}

/// MOVE_RT plus C (placement 113, creature 123): in F2, 50 m away from both,
/// with a mover.
fn move_rt_with_far_c() -> Fixture {
    let mut fx = move_rt();
    add_outsider(&mut fx, 113, 123);
    enlist(&mut fx, 113, 8);
    for name in ["alpha", "beta", "gamma"] {
        set_stat_raw(&mut fx, pl(123), name, 5 * METRE);
    }
    set_stat_raw(&mut fx, pl(123), "health", 4 * METRE);
    set_stat_raw(&mut fx, pl(123), "pace", METRE);
    faction(&mut fx, 123, Some(f2()));
    set_position(
        &mut fx,
        113,
        [Fx16_16::ZERO, Fx16_16::ZERO, Fx16_16::from_int(50)],
    );
    fx
}

fn try_start(fx: &Fixture, mode: EncounterMode) -> (World, Result<(), CombatError>) {
    let mut w = World::new(7);
    w.bind_nav(rev());
    let started = fx.with_spec(|spec| start_encounter_in_mode(&mut w, spec, mode));
    (w, started)
}

fn start(fx: &Fixture, mode: EncounterMode) -> World {
    let (w, started) = try_start(fx, mode);
    started.expect("the fixture starts");
    w
}

fn who(w: &World, placement: u128) -> EntityId {
    w.combatants()
        .iter()
        .find(|(_, c)| c.placement() == pl(placement))
        .map(|(id, _)| id)
        .expect("the placement is a combatant")
}

fn position(w: &World, e: EntityId) -> [Fx16_16; 3] {
    w.movers().get(e).expect("a mover").position()
}

fn gap(w: &World) -> u64 {
    seg_len(position(w, who(w, 111)), position(w, who(w, 112)))
}

fn engage_of(w: &World) -> Vec<EngageRule> {
    w.combat().expect("encounter").definition.engage.clone()
}

/// Starts RT, sends A (1 m a tick) along (0, 0) to (8, 0) and ticks until A
/// stands at (8, 0, 0), 2 m from B. Returns the world and the ticks it took.
fn arrival(fx: &Fixture) -> (World, usize) {
    let mut w = start(fx, EncounterMode::RealTime);
    let a = who(&w, 111);
    assert_eq!(w.set_route(a, route(1, vec![p(0, 0), p(8, 0)])), Ok(None));
    let mut ticks = 0;
    while position(&w, a) != p(8, 0) {
        tick(&mut w);
        ticks += 1;
        assert!(ticks <= 20, "A does not arrive");
    }
    (w, ticks)
}

fn edit(w: &World, change: impl FnOnce(&mut Value)) -> World {
    with_json(w, change)
}

/// Removes `entity`'s mover from a world's JSON.
fn without_mover(w: &World, entity: EntityId) -> World {
    let key = serde_json::to_value(entity).expect("an entity id serializes");
    edit(w, |s| {
        let movers = s["movers"].as_array_mut().expect("movers");
        let before = movers.len();
        movers.retain(|pair| pair[0] != key);
        assert_eq!(movers.len(), before - 1, "exactly one mover is removed");
    })
}

fn rt_kill(w: &mut World, fx: &Fixture, actor: EntityId, target: EntityId) {
    let stored =
        perform_action(w, &use_action(actor, fx.ability, target)).expect("the order is accepted");
    assert!(stored.is_none(), "a real-time use stores an order");
    for _ in 0..20 {
        if w.combatants().get(target).expect("combatant").dead() {
            return;
        }
        tick(w);
    }
    panic!("the target did not die within 20 ticks");
}

#[test]
fn sc1_engage_is_resolved() {
    let w = start(&move_rt(), EncounterMode::RealTime);
    assert_eq!(
        engage_of(&w),
        vec![EngageRule::Proximity {
            distance: Fx16_16::from_raw(131_072)
        }]
    );
    let wire = serde_json::to_value(rule(131_072)).expect("a rule serializes");
    assert_eq!(
        wire,
        json!({"type": "proximity", "value": {"distance": 131072}})
    );
    let back: EngageRule = serde_json::from_value(wire).expect("a rule loads");
    assert_eq!(back, rule(131_072));
    // Authored order, several entries.
    let mut fx = move_rt();
    fx.encounter.engage = Some(vec![proximity(3 * METRE), proximity(METRE)]);
    let w = start(&fx, EncounterMode::RealTime);
    assert_eq!(engage_of(&w), vec![rule(3 * METRE), rule(METRE)]);
    // The rule travels with the world: save, load, same bytes and hash.
    let json = serde_json::to_string(&w).expect("the world serializes");
    let loaded: World = serde_json::from_str(&json).expect("the world loads");
    assert_eq!(loaded, w);
    assert_eq!(state_hash(&loaded), state_hash(&w));
    // Unknown keys inside a rule are refused.
    assert!(serde_json::from_value::<EngageRule>(
        json!({"type": "proximity", "value": {"distance": 1, "x": 1}})
    )
    .is_err());
    assert!(serde_json::from_value::<EngageRule>(
        json!({"type": "sight", "value": {"distance": 1}})
    )
    .is_err());

    // Absent and empty give an empty list and the bytes of a start that never
    // set the field.
    let mut never = move_rt();
    never.encounter.engage = None;
    let w_never = start(&never, EncounterMode::RealTime);
    assert!(engage_of(&w_never).is_empty());
    let mut empty = move_rt();
    empty.encounter.engage = Some(Vec::new());
    let w_empty = start(&empty, EncounterMode::RealTime);
    assert!(engage_of(&w_empty).is_empty());
    assert_eq!(bytes_of(&w_empty), bytes_of(&w_never));
    assert_eq!(state_hash(&w_empty), state_hash(&w_never));
    assert!(!serde_json::to_string(&w_never)
        .expect("the world serializes")
        .contains("engage"));
}

#[test]
fn sc2_engage_survives_the_legacy_shape() {
    // A legacy-equivalent definition: Fixture::standard(), turn-based.
    let plain = Fixture::standard();
    let w0 = start(&plain, EncounterMode::TurnBased);
    let def0 = w0.combat().expect("encounter").definition.clone();
    let v0 = serde_json::to_value(&def0).expect("the definition serializes");
    let keys0: Vec<&str> = v0
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    let mut want: Vec<&str> = LEGACY_KEYS.to_vec();
    want.sort_unstable();
    let mut got = keys0.clone();
    got.sort_unstable();
    assert_eq!(got, want, "exactly the 15 legacy keys");

    // With one rule: the full shape plus `engage`.
    let mut ruled = Fixture::standard();
    ruled.encounter.engage = Some(vec![proximity(METRE)]);
    let w1 = start(&ruled, EncounterMode::TurnBased);
    let def1 = w1.combat().expect("encounter").definition.clone();
    assert_eq!(def1.engage, vec![rule(METRE)]);
    let v1 = serde_json::to_value(&def1).expect("the definition serializes");
    let object = v1.as_object().expect("an object");
    for key in LEGACY_KEYS
        .iter()
        .chain(["abilities", "pools", "effects", "engage"].iter())
    {
        assert!(object.contains_key(*key), "{key}");
    }
    assert_eq!(object.len(), 19);
    assert_eq!(
        v1["engage"],
        json!([{"type": "proximity", "value": {"distance": 65536}}])
    );
    let back: CombatDefinition = serde_json::from_value(v1).expect("the full shape loads");
    assert_eq!(back, def1);
    assert_ne!(state_hash(&w1), state_hash(&w0));
    let loaded: World =
        serde_json::from_str(&serde_json::to_string(&w1).expect("saves")).expect("the world loads");
    assert_eq!(loaded, w1);
    assert_eq!(engage_of(&loaded), vec![rule(METRE)]);

    // A hand-made legacy-shaped save (the 15 keys) plus `engage` loads through
    // the legacy arm with the rule kept, and saves in the full shape.
    let mut legacy = v0.clone();
    legacy["engage"] = json!([{"type": "proximity", "value": {"distance": 65536}}]);
    let loaded: CombatDefinition =
        serde_json::from_value(legacy).expect("the legacy arm carries engage");
    assert_eq!(loaded.engage, vec![rule(METRE)]);
    assert_eq!(loaded, def1);
    let saved = serde_json::to_value(&loaded).expect("the definition serializes");
    let object = saved.as_object().expect("an object");
    assert_eq!(object.len(), 19);
    for key in ["abilities", "pools", "effects", "engage"] {
        assert!(object.contains_key(key), "{key}");
    }
    // Without the key the legacy arm still gives an empty list.
    let bare: CombatDefinition = serde_json::from_value(v0).expect("the legacy shape loads");
    assert!(bare.engage.is_empty());
    assert_eq!(bare, def0);
}

#[test]
fn sc3_non_positive_distance_is_refused() {
    for raw in [0, -1] {
        let mut fx = move_rt();
        fx.encounter.engage = Some(vec![proximity(raw)]);
        let (w, started) = try_start(&fx, EncounterMode::RealTime);
        assert_eq!(started, Err(CombatError::InvalidEngage), "raw {raw}");
        assert!(w.combat().is_none());
        // Nothing started: the world is exactly a fresh one with the nav bound.
        let mut clean = World::new(7);
        clean.bind_nav(rev());
        assert_eq!(bytes_of(&w), bytes_of(&clean), "raw {raw}");
        // A later valid rule after the bad one does not rescue the start.
        let mut mixed = move_rt();
        mixed.encounter.engage = Some(vec![proximity(METRE), proximity(raw)]);
        let (_, started) = try_start(&mixed, EncounterMode::RealTime);
        assert_eq!(started, Err(CombatError::InvalidEngage), "raw {raw}");
    }
    assert_eq!(
        format!("{}", CombatError::InvalidEngage),
        "InvalidEngage at combat/engage"
    );

    // Saved definitions: the legacy arm and the full shape both refuse.
    let w0 = start(&Fixture::standard(), EncounterMode::TurnBased);
    let v0 = serde_json::to_value(&w0.combat().expect("encounter").definition)
        .expect("the definition serializes");
    let mut legacy = v0.clone();
    legacy["engage"] = json!([{"type": "proximity", "value": {"distance": 0}}]);
    let error = serde_json::from_value::<CombatDefinition>(legacy).expect_err("refused");
    assert!(
        error
            .to_string()
            .contains("engage proximity distance must be positive"),
        "{error}"
    );
    let mut ruled = Fixture::standard();
    ruled.encounter.engage = Some(vec![proximity(METRE)]);
    let w1 = start(&ruled, EncounterMode::TurnBased);
    let mut full = serde_json::to_value(&w1.combat().expect("encounter").definition)
        .expect("the definition serializes");
    full["engage"] = json!([{"type": "proximity", "value": {"distance": -65536}}]);
    let error = serde_json::from_value::<CombatDefinition>(full.clone()).expect_err("refused");
    assert!(
        error
            .to_string()
            .contains("engage proximity distance must be positive"),
        "{error}"
    );
    // Control: the positive distance loads.
    full["engage"] = json!([{"type": "proximity", "value": {"distance": 65536}}]);
    assert!(serde_json::from_value::<CombatDefinition>(full).is_ok());
    // A whole saved world carrying the bad rule fails to load too.
    let bad = try_with_json(&w1, |s| {
        s["combat"]["definition"]["engage"] =
            json!([{"type": "proximity", "value": {"distance": 0}}]);
    })
    .expect_err("the world is refused");
    assert!(
        bad.contains("engage proximity distance must be positive"),
        "{bad}"
    );
}

/// The refusal comes after every existing start check (the creation-check
/// trap): a start that fails an earlier check reports that check, with a bad
/// distance or a good one.
#[test]
fn sc3_an_earlier_start_check_still_wins() {
    // Mover check (InvalidSpeed) before the engagement distance.
    let mut fx = move_rt();
    fx.encounter.engage = Some(vec![proximity(0)]);
    set_stat_raw(&mut fx, pl(121), "pace", 0);
    let (_, started) = try_start(&fx, EncounterMode::RealTime);
    assert_eq!(
        started,
        Err(CombatError::InvalidSpeed { placement: pl(111) })
    );
    // Control: the pace repaired, the distance is what fails.
    set_stat_raw(&mut fx, pl(121), "pace", METRE);
    let (_, started) = try_start(&fx, EncounterMode::RealTime);
    assert_eq!(started, Err(CombatError::InvalidEngage));

    // A member check (M3, NotCreature) before the engagement distance.
    let mut fx = move_rt();
    fx.encounter.engage = Some(vec![proximity(-1)]);
    let mut w = World::new(7);
    w.bind_nav(rev());
    let thing = w.spawn(EntityMeta {});
    let members = [Entrant {
        entity: thing,
        initiative: 5,
    }];
    let started = fx.with_spec(|spec| {
        start_encounter_with_members(&mut w, spec, EncounterMode::RealTime, &members)
    });
    assert_eq!(started, Err(CombatError::NotCreature { entity: thing }));
    // Control: no member, the distance is what fails.
    let (_, started) = try_start(&fx, EncounterMode::RealTime);
    assert_eq!(started, Err(CombatError::InvalidEngage));
}

/// M07 (S-C1 place): a start that fails both an earlier check and the
/// engagement check reports the earlier one. Both participants hold dead
/// records, so every participant is skipped and no members are listed: the
/// start has no entrants, and the 0-distance rule must not win over it.
#[test]
fn sc3b_an_earlier_check_wins_over_invalid_engage() {
    let mut fx = move_rt();
    fx.encounter.engage = Some(vec![proximity(0)]);
    let mut world = World::new(7);
    world.bind_nav(rev());
    let a = fx.register(&mut world, pl(111)).expect("A registers");
    let b = fx.register(&mut world, pl(112)).expect("B registers");
    let one_dead = with_record(&world, a, |record| record["health"] = json!(0));
    let mut both_dead = with_record(&one_dead, b, |record| record["health"] = json!(0));
    let before = bytes_of(&both_dead);
    let started =
        fx.with_spec(|spec| start_encounter_in_mode(&mut both_dead, spec, EncounterMode::RealTime));
    assert_eq!(started, Err(CombatError::NoEntrants));
    assert_eq!(
        bytes_of(&both_dead),
        before,
        "a refused start changes nothing"
    );
}

/// M08 (S-C1 place): the engagement refusal runs before the T137 initiative
/// draws, so a refused start leaves the RNG (and every other byte) untouched.
#[test]
fn sc3c_invalid_engage_with_initiative_draws_nothing() {
    let mut fx = move_rt();
    set_initiative_rule(&mut fx, "1d6", &[]);
    fx.encounter.engage = Some(vec![proximity(0)]);
    let mut world = World::new(7);
    world.bind_nav(rev());
    let before = serde_json::to_string(&world).expect("the world serializes");
    let started =
        fx.with_spec(|spec| start_encounter_in_mode(&mut world, spec, EncounterMode::RealTime));
    assert_eq!(started, Err(CombatError::InvalidEngage));
    assert_eq!(
        serde_json::to_string(&world).expect("the world serializes"),
        before,
        "the refusal draws no initiative"
    );
}

#[test]
fn sc4_proximity_by_distance() {
    let fx = move_rt();
    let mut w = start(&fx, EncounterMode::RealTime);
    let (a, b) = (who(&w, 111), who(&w, 112));
    assert_eq!(gap(&w), 10 * METRE as u64);
    assert!(!engagement_due(&w), "10 m apart");
    assert_eq!(w.set_route(a, route(1, vec![p(0, 0), p(8, 0)])), Ok(None));
    let mut first_due = None;
    for n in 1..=20 {
        tick(&mut w);
        let near = gap(&w) <= 131_072;
        assert_eq!(engagement_due(&w), near, "tick {n}");
        if near && first_due.is_none() {
            first_due = Some(n);
        }
        if position(&w, a) == p(8, 0) {
            break;
        }
    }
    // 1 m a tick from 10 m apart: 2 m apart after exactly eight ticks.
    assert_eq!(first_due, Some(8));
    assert_eq!(position(&w, b), p(10, 0));
    assert_eq!(gap(&w), 131_072);
    assert!(engagement_due(&w));

    let (arrived, ticks) = arrival(&fx);
    assert_eq!(ticks, 8);
    assert_eq!(arrived, w, "the arrival state is reproducible");

    // Factions removed: A and B are each a side of one, still opposed.
    let mut alone = move_rt();
    faction(&mut alone, 121, None);
    faction(&mut alone, 122, None);
    assert!(engagement_due(&arrival(&alone).0));

    // A and B both in F1: not opposed (and the fight is over).
    let same = edit(&arrived, |s| {
        for pair in s["combatants"].as_array_mut().expect("combatants") {
            pair[1]["faction"] = json!(f1().to_string());
        }
    });
    assert!(!fight_is_live(&same));
    assert!(!engagement_due(&same));
    // Control: the same edit with the real factions is true.
    let control = edit(&arrived, |s| {
        let combatants = s["combatants"].as_array_mut().expect("combatants");
        combatants[0][1]["faction"] = json!(f1().to_string());
        combatants[1][1]["faction"] = json!(f2().to_string());
    });
    assert!(engagement_due(&control));

    // A live fight whose close pair is not opposed: A and B both in F1 beside
    // each other, the far C alone in F2. Not due (C is 50 m away).
    let mut allies = move_rt_with_far_c();
    faction(&mut allies, 122, Some(f1()));
    let (allies_world, _) = arrival(&allies);
    assert!(fight_is_live(&allies_world), "C keeps a second side");
    assert_eq!(gap(&allies_world), 131_072);
    assert!(!engagement_due(&allies_world));
    // Control: B back in F2, the same positions are due.
    let (opposed_world, _) = arrival(&move_rt_with_far_c());
    assert!(engagement_due(&opposed_world));

    // The rule's distance 1 m: 2 m apart is outside it.
    let mut tight = move_rt();
    tight.encounter.engage = Some(vec![proximity(METRE)]);
    assert!(!engagement_due(&arrival(&tight).0));
    // Equal to the distance counts (<=): 2 m apart, a 2 m rule held above; one
    // raw unit under does not.
    let mut under = move_rt();
    under.encounter.engage = Some(vec![proximity(131_071)]);
    assert!(!engagement_due(&arrival(&under).0));
    // Two rules, 1 m and 3 m: any one suffices.
    let mut two = move_rt();
    two.encounter.engage = Some(vec![proximity(METRE), proximity(3 * METRE)]);
    assert!(engagement_due(&arrival(&two).0));
    // No rule at all: never.
    let mut none = move_rt();
    none.encounter.engage = None;
    assert!(!engagement_due(&arrival(&none).0));
    let mut empty = move_rt();
    empty.encounter.engage = Some(Vec::new());
    assert!(!engagement_due(&arrival(&empty).0));

    // A pending mode request: not due; the boundary applies it: still not due.
    let mut pending = arrived.clone();
    request_mode(&mut pending, EncounterMode::TurnBased).expect("the request is accepted");
    assert!(!engagement_due(&pending));
    for _ in 0..8 {
        if pending.combat().expect("encounter").mode == EncounterMode::TurnBased {
            break;
        }
        tick(&mut pending);
    }
    assert_eq!(
        pending.combat().expect("encounter").mode,
        EncounterMode::TurnBased
    );
    assert!(fight_is_live(&pending));
    assert!(!engagement_due(&pending));

    // Reads only.
    let before = bytes_of(&arrived);
    assert!(engagement_due(&arrived));
    assert_eq!(bytes_of(&arrived), before);

    // A turn-based start is never due, whatever the distance.
    let tb = start(
        &{
            let mut fx = move_rt();
            fx.encounter.engage = Some(vec![proximity(1000 * METRE)]);
            fx
        },
        EncounterMode::TurnBased,
    );
    assert!(!engagement_due(&tb));
    // No encounter: not due.
    assert!(!engagement_due(&World::new(7)));
}

#[test]
fn sc5_proximity_ignores_the_dead_and_moverless() {
    let fx = move_rt();
    let (arrived, _) = arrival(&fx);
    let (a, b) = (who(&arrived, 111), who(&arrived, 112));
    assert!(engagement_due(&arrived), "control");

    // B killed: the fight is over, so not due.
    let mut lethal = move_rt();
    always_success(&mut lethal);
    make_lethal(&mut lethal, 99);
    let (mut killed_lethal, _) = arrival(&lethal);
    rt_kill(&mut killed_lethal, &lethal, a, b);
    assert!(!fight_is_live(&killed_lethal));
    assert!(!engagement_due(&killed_lethal));

    // The dead are skipped on their own: a far third combatant keeps the fight
    // live, and the dead B beside A must not count.
    let mut three = move_rt_with_far_c();
    always_success(&mut three);
    make_lethal(&mut three, 99);
    let (mut w, _) = arrival(&three);
    assert!(engagement_due(&w), "control: A and B within 2 m");
    rt_kill(&mut w, &three, a, b);
    assert!(fight_is_live(&w), "C of F2 keeps the fight live");
    assert!(w.combatants().get(b).expect("B").dead());
    assert_eq!(gap(&w), 131_072, "the dead B still stands 2 m from A");
    assert!(!engagement_due(&w));

    // A moverless B: false at any distance. A 1000 m rule from the start.
    let mut wide = move_rt();
    wide.encounter.engage = Some(vec![proximity(1000 * METRE)]);
    let start_world = start(&wide, EncounterMode::RealTime);
    let (a, b) = (who(&start_world, 111), who(&start_world, 112));
    assert!(engagement_due(&start_world), "control: 10 m inside 1000 m");
    assert!(!engagement_due(&without_mover(&start_world, b)));
    assert!(!engagement_due(&without_mover(&start_world, a)));
    // The same at the arrival state.
    let moverless = without_mover(&arrived, b);
    assert!(moverless.movers().get(b).is_none());
    assert!(moverless.movers().get(a).is_some());
    assert!(fight_is_live(&moverless));
    assert!(!engagement_due(&moverless));
}
