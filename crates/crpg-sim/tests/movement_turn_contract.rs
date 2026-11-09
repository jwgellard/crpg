//! T036c two-keys suite: the turn-based move, history version 3 and the
//! persisted `TurnMovement`.
//!
//! Written independently from the contract text alone (`tasks/T036.md`,
//! Behaviour B-C1–B-C5 and B-H1–B-H4, test items C-1–C-6). Every expected
//! value is a contract literal or hand arithmetic over one; nothing here was
//! read off the implementation. Test names carry the item they prove.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_sim::*;
use serde_json::{json, Value};
use support::creature::{add_outsider, top_level_keys, try_with_json, with_json};
use support::rt::{bytes_of, make_lethal, value_of};
use support::surface::{always_success, set_stat_raw};
use support::{uid, Fixture};

// ----------------------------------------------------------------- fixtures

fn p(x: i32, z: i32) -> [Fx16_16; 3] {
    [Fx16_16::from_int(x), Fx16_16::ZERO, Fx16_16::from_int(z)]
}
fn rp(x: i32, y: i32, z: i32) -> [Fx16_16; 3] {
    [
        Fx16_16::from_raw(x),
        Fx16_16::from_raw(y),
        Fx16_16::from_raw(z),
    ]
}
fn raw3(v: [Fx16_16; 3]) -> [i32; 3] {
    v.map(Fx16_16::to_raw)
}
/// The step as a fixed-point length.
fn step(raw: i32) -> Fx16_16 {
    Fx16_16::from_raw(raw)
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
fn pl(n: u128) -> Ulid {
    Ulid::from_u128(n)
}
fn edit_ruleset(f: &mut Fixture, edit: impl FnOnce(&mut Value)) {
    let mut v = serde_json::to_value(&f.ruleset).unwrap();
    edit(&mut v);
    f.ruleset = serde_json::from_value(v).unwrap();
}
fn set_position(f: &mut Fixture, placement: u128, position: [Fx16_16; 3]) {
    f.placements
        .get_mut(&pl(placement))
        .unwrap()
        .0
        .transform
        .position = position;
}
/// The MOVE fixture of the contract: A at `p(0,0)` (1 m/tick), B at
/// `p(10,0)` (0.5 m/tick), C at `p(0,10)`, every step 1.5 m, pool 106 holds 6.
fn move_fixture() -> Fixture {
    let mut fx = Fixture::standard();
    edit_ruleset(&mut fx, |r| {
        r["stats"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "pace", "kind": "fixed"}));
        r["pools"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": uid(106), "max": 6, "refresh": {"type": "on_turn_start"}}));
        r["movement"] = json!({"speed": {"type": "stat", "value": "pace"},
            "turn": {"pool": uid(106), "step": {"type": "constant", "value": 98304}, "step_cost": 1}});
    });
    set_stat_raw(&mut fx, pl(121), "pace", 65_536);
    set_stat_raw(&mut fx, pl(122), "pace", 32_768);
    set_position(&mut fx, 112, rp(655_360, 0, 0));
    add_outsider(&mut fx, 113, 123);
    set_position(&mut fx, 113, rp(0, 0, 655_360));
    for name in ["alpha", "beta", "gamma"] {
        set_stat_raw(&mut fx, pl(123), name, 5 * 65_536);
    }
    set_stat_raw(&mut fx, pl(123), "health", 4 * 65_536);
    set_stat_raw(&mut fx, pl(123), "pace", 65_536);
    fx
}
/// MOVE with `round_ticks`, so an encounter is real-time capable.
fn move_rt() -> Fixture {
    let mut fx = move_fixture();
    fx.ruleset.round_ticks = Some(4);
    fx
}
/// MOVE1: MOVE without pool 106 and with the ability's pool `uid(105)` as the
/// movement pool.
fn move1() -> Fixture {
    let mut fx = move_fixture();
    edit_ruleset(&mut fx, |r| {
        r["pools"]
            .as_array_mut()
            .unwrap()
            .retain(|pool| pool["id"] != json!(uid(106)));
        r["movement"]["turn"]["pool"] = json!(uid(105));
    });
    fx
}
fn lethal(fx: &mut Fixture) {
    always_success(fx);
    make_lethal(fx, 99);
}
/// A MOVE turn-based world with the nav bound, returning the two combatant
/// ids in authored order (A, B).
fn tb_world(fx: &Fixture) -> (World, EntityId, EntityId) {
    let (mut w, ids) = fx.start(7);
    w.bind_nav(rev());
    (w, ids[0], ids[1])
}
/// The MOVE creature of `placement` in a live world.
fn who(w: &World, placement: u128) -> EntityId {
    w.creatures()
        .iter()
        .find(|(_, r)| r.placement() == pl(placement))
        .map(|(id, _)| id)
        .or_else(|| {
            w.combatants()
                .iter()
                .find(|(_, c)| c.placement() == pl(placement))
                .map(|(id, _)| id)
        })
        .expect("the placement is live")
}
fn bits(w: &World, e: EntityId) -> ([u32; 3], [u32; 3]) {
    let t = w.transforms().get(e).expect("a transform");
    (t.position.map(f32::to_bits), t.velocity.map(f32::to_bits))
}
/// B's movement pool, 106, the first non-primary template.
fn pool106(w: &World, e: EntityId) -> u32 {
    w.combatants()
        .get(e)
        .expect("a combatant")
        .extra_pools()[0]
        .current()
}
fn remove_mover(save: &mut Value, entity: EntityId) {
    let key = serde_json::to_value(entity).expect("the id serializes");
    save["movers"]
        .as_array_mut()
        .expect("a mover list")
        .retain(|entry| entry[0] != key);
}
fn set_mirror_velocity(save: &mut Value, entity: EntityId, velocity: Value) {
    let key = serde_json::to_value(entity).expect("the id serializes");
    let entries = save["transforms"].as_array_mut().expect("a transform list");
    let entry = entries
        .iter_mut()
        .find(|entry| entry[0] == key)
        .expect("the entity holds a transform");
    entry[1]["velocity"] = velocity;
}

// ------------------------------------------------------- C-1 / B-C1 route_cost

#[test]
fn c1_route_cost_b_c1() {
    // The contract's literals.
    assert_eq!(route_cost(720_896, step(98_304), 1), Some(8));
    assert_eq!(route_cost(327_680, step(98_304), 2), Some(8));
    assert_eq!(route_cost(0, step(98_304), 1), Some(0));
    assert_eq!(route_cost(1, step(65_536), 3), Some(3));
    assert_eq!(route_cost(4_294_967_295, step(1), 1), Some(4_294_967_295));
    assert_eq!(route_cost(4_294_967_296, step(1), 1), None);
    assert_eq!(route_cost(268_435_456, step(1), u32::MAX), None);
    assert_eq!(route_cost(720_896, step(0), 1), None);
    assert_eq!(route_cost(720_896, step(-1), 1), None);

    // Controls: one input differs, so the output changes exactly as
    // hand arithmetic says (1 raw over a whole step is one more step).
    assert_eq!(route_cost(720_896, step(98_304), 3), Some(24));
    assert_eq!(route_cost(327_680, step(98_304), 1), Some(4));
    assert_eq!(route_cost(393_216, step(98_304), 1), Some(4));
    assert_eq!(route_cost(393_217, step(98_304), 1), Some(5));
    assert_eq!(route_cost(4_294_967_294, step(1), 1), Some(4_294_967_294));

    // A `u64` product that does not fit is `None`, never a wrap.
    assert_eq!(route_cost(u64::MAX, step(1), u32::MAX), None);
    assert_eq!(route_cost(u64::MAX, step(1), 1), None);
}

// --------------------------------------- C-2 / B-C4 spending from the pool

#[test]
fn c2_move_spends_and_keeps_the_turn_b_c4() {
    let fx = move_fixture();
    let (mut w, ids) = fx.start(7);
    let (_, b) = (ids[0], ids[1]);
    w.bind_nav(rev());
    assert_eq!(w.combat().expect("an encounter").active, Some(b));
    assert_eq!(pool106(&w, b), 6);
    let before = value_of(&w);

    let outcome = perform_move(&mut w, b, &route(1, vec![p(10, 0), p(10, 3), p(7, 3)]))
        .expect("six metres at 1.5 m a step costs four");
    assert_eq!(
        outcome,
        MoveOutcome {
            actor: b,
            cost: 4,
            from: p(10, 0),
            to: p(7, 3),
        }
    );
    assert_eq!(pool106(&w, b), 2, "the pool pays exactly the cost");
    assert_eq!(
        raw3(w.movers().get(b).expect("a mover").position()),
        [458_752, 0, 196_608]
    );
    assert!(w.movers().get(b).expect("a mover").route().is_none());
    assert_eq!(
        bits(&w, b),
        ([0x40e0_0000, 0, 0x4040_0000], [0; 3]),
        "the mirror follows the mover with zero velocity"
    );
    assert_eq!(
        w.combat().expect("an encounter").active,
        Some(b),
        "the move keeps the turn"
    );
    let after = value_of(&w);
    assert_eq!(after["rng"], before["rng"], "no draw");
    assert_eq!(after["events"], before["events"], "no event");

    // Its turn is still B's: it moves again, then cannot afford one metre.
    let outcome = perform_move(&mut w, b, &route(2, vec![p(7, 3), p(7, 1)]))
        .expect("two metres cost two");
    assert_eq!((outcome.cost, pool106(&w, b)), (2, 0));
    let before = bytes_of(&w);
    assert_eq!(
        perform_move(&mut w, b, &route(3, vec![p(7, 1), p(7, 0)])),
        Err(MoveError::CannotAfford {
            cost: 1,
            current: 0
        })
    );
    assert_eq!(bytes_of(&w), before, "a rejection changes nothing");

    // A cost exactly equal to the pool is paid; one more is refused.
    let (mut w, ids) = fx.start(7);
    let b9 = ids[1];
    w.bind_nav(rev());
    let outcome =
        perform_move(&mut w, b9, &route(4, vec![p(10, 0), p(10, 9)])).expect("six steps fit six");
    assert_eq!((outcome.cost, pool106(&w, b9)), (6, 0));
    let (mut w, ids) = fx.start(7);
    let b10 = ids[1];
    w.bind_nav(rev());
    assert_eq!(
        perform_move(&mut w, b10, &route(5, vec![p(10, 0), p(10, 10)])),
        Err(MoveError::CannotAfford {
            cost: 7,
            current: 6
        })
    );

    // A `u32::MAX`-sized cost is reported at its true `u64` value.
    let mut wide = move_fixture();
    edit_ruleset(&mut wide, |r| {
        r["movement"]["turn"]["step"] = json!({"type": "constant", "value": 1});
        r["movement"]["turn"]["step_cost"] = json!(u32::MAX);
    });
    let (mut w, ids) = wide.start(7);
    let b = ids[1];
    w.bind_nav(rev());
    assert_eq!(
        perform_move(&mut w, b, &route(6, vec![p(10, 0), p(11, 0)])),
        Err(MoveError::CannotAfford {
            cost: 281_474_976_645_120,
            current: 6
        })
    );
}

#[test]
fn c2_step_cost_scales_the_charge_b_c1() {
    // One input differs from the C-2 world: `step_cost` 3 instead of 1.
    let mut fx = move_fixture();
    edit_ruleset(&mut fx, |r| {
        r["movement"]["turn"]["step_cost"] = json!(3);
    });
    let (mut w, ids) = fx.start(7);
    let b = ids[1];
    w.bind_nav(rev());
    assert_eq!(
        perform_move(&mut w, b, &route(1, vec![p(10, 0), p(10, 3), p(7, 3)])),
        Err(MoveError::CannotAfford {
            cost: 12,
            current: 6
        })
    );
    let outcome =
        perform_move(&mut w, b, &route(2, vec![p(10, 0), p(11, 0)])).expect("one step costs three");
    assert_eq!((outcome.cost, pool106(&w, b)), (3, 3));
}

#[test]
fn c2_mirror_velocity_is_rewritten_b_c4() {
    let fx = move_fixture();
    let (w, ids) = fx.start(7);
    let b = ids[1];
    let mut w = {
        // The save loads with a velocity (L6 reads positions only).
        let mut w = w;
        w.bind_nav(rev());
        with_json(&w, |save| {
            set_mirror_velocity(save, b, json!([1.0_f32, 0.0_f32, 0.0_f32]))
        })
    };
    assert_eq!(bits(&w, b).1, [0x3f80_0000, 0, 0]);
    perform_move(&mut w, b, &route(1, vec![p(10, 0), p(10, 3), p(7, 3)])).expect("the move");
    assert_eq!(bits(&w, b).1, [0; 3], "the move writes zero velocity");
}

#[test]
fn c2_primary_pool_balance_and_spend_b_c3_b_c4() {
    // MOVE1: the movement pool is the ability's own primary pool.
    let fx = move1();
    let (mut w, ids) = fx.start(7);
    let (a, b) = (ids[0], ids[1]);
    w.bind_nav(rev());
    assert_eq!(w.combatants().get(b).expect("B").action_pool().current(), 1);
    assert!(w.combatants().get(b).expect("B").extra_pools().is_empty());
    let outcome = perform_move(&mut w, b, &route(1, vec![p(10, 0), p(11, 0)])).expect("one step");
    assert_eq!(outcome.cost, 1);
    assert_eq!(w.combatants().get(b).expect("B").action_pool().current(), 0);
    assert_eq!(
        perform_move(&mut w, b, &route(2, vec![p(11, 0), p(12, 0)])),
        Err(MoveError::CannotAfford {
            cost: 1,
            current: 0
        })
    );

    // The ability that shares the pool now cannot be afforded either.
    assert_eq!(
        validate_action(
            &w,
            &CombatAction::UseAbility {
                actor: b,
                ability: fx.ability,
                target: a,
            }
        ),
        Err(CombatError::InsufficientAction {
            pool: Ulid::from_u128(105),
            cost: 1,
            current: 0
        })
    );
}

// -------------------------------------- C-3 / B-C3 the ordered move checks

/// B's one-metre route from its start position.
fn route_b(tag: u64) -> MoveRoute {
    route(tag, vec![p(10, 0), p(11, 0)])
}

#[test]
fn c3_checks_in_order_b_c3() {
    let fx = move_fixture();

    // NoEncounter: a registered outsider in a world with no fight.
    let mut plain = World::new(7);
    let c = fx.register(&mut plain, pl(113)).expect("C registers");
    plain.bind_nav(rev());
    assert_eq!(
        validate_move(&plain, c, &route(1, vec![p(0, 10), p(1, 10)])),
        Err(MoveError::Combat(CombatError::NoEncounter))
    );

    // NotParticipant: the same outsider, this time in a live fight.
    let (mut w, a, b) = tb_world(&fx);
    let c = fx.register(&mut w, pl(113)).expect("C registers");
    assert_eq!(
        validate_move(&w, c, &route(1, vec![p(0, 10), p(1, 10)])),
        Err(MoveError::Combat(CombatError::NotParticipant { entity: c }))
    );

    // AbsentActor: a despawned world-local id.
    let ghost = w.spawn(EntityMeta {});
    assert!(w.despawn(ghost));
    assert_eq!(
        validate_move(&w, ghost, &route(1, vec![p(0, 0), p(1, 0)])),
        Err(MoveError::Combat(CombatError::AbsentActor { actor: ghost }))
    );

    // OutOfTurn: A asks while B holds the turn.
    assert_eq!(
        validate_move(&w, a, &route(1, vec![p(0, 0), p(1, 0)])),
        Err(MoveError::Combat(CombatError::OutOfTurn {
            actor: a,
            active: Some(b)
        }))
    );

    // DeadActor, then terminal OutOfTurn with no active actor.
    let mut fight = move_fixture();
    lethal(&mut fight);
    let (mut w, a, b) = tb_world(&fight);
    perform_action(
        &mut w,
        &CombatAction::UseAbility {
            actor: b,
            ability: fight.ability,
            target: a,
        },
    )
    .expect("B acts")
    .expect("a hit");
    assert!(w.combatants().get(a).expect("A").dead());
    assert_eq!(
        validate_move(&w, a, &route(1, vec![p(0, 0), p(1, 0)])),
        Err(MoveError::Combat(CombatError::DeadActor { actor: a }))
    );
    assert_eq!(
        validate_move(&w, b, &route(1, vec![p(10, 0), p(11, 0)])),
        Err(MoveError::Combat(CombatError::OutOfTurn {
            actor: b,
            active: None
        }))
    );

    // WrongMode: a real-time encounter's turn checks reject an EndTurn.
    let mut rt = World::new(7);
    rt.bind_nav(rev());
    move_rt()
        .with_spec(|spec| start_encounter_in_mode(&mut rt, spec, EncounterMode::RealTime))
        .expect("the RT fight starts");
    let b = who(&rt, 112);
    assert_eq!(
        validate_move(&rt, b, &route_b(1)),
        Err(MoveError::Combat(CombatError::WrongMode {
            mode: EncounterMode::RealTime
        }))
    );

    // NotMover: a combatant whose mover is missing from the save.
    let (w, _, b) = tb_world(&fx);
    let w = with_json(&w, |save| remove_mover(save, b));
    assert!(w.movers().get(b).is_none());
    assert_eq!(
        validate_move(&w, b, &route_b(1)),
        Err(MoveError::NotMover { entity: b })
    );

    // NotMover: a definition that holds no movement rules.
    let (w, _, b) = tb_world(&fx);
    let w = with_json(&w, |save| {
        save["combat"]["definition"]
            .as_object_mut()
            .expect("a definition object")
            .remove("movement")
            .expect("the definition holds movement");
    });
    assert!(w.movers().get(b).is_some(), "the mover survives the edit");
    assert_eq!(
        validate_move(&w, b, &route_b(1)),
        Err(MoveError::NotMover { entity: b })
    );

    // StartMismatch: the route does not start where the mover stands.
    let (w, _, b) = tb_world(&fx);
    assert_eq!(
        validate_move(&w, b, &route(1, vec![p(0, 0), p(1, 0)])),
        Err(MoveError::StartMismatch)
    );

    // NavUnbound: nothing bound, a valid route.
    let (w, ids) = fx.start(7);
    let b = ids[1];
    assert_eq!(w.nav(), None);
    assert_eq!(validate_move(&w, b, &route_b(1)), Err(MoveError::NavUnbound));
}

#[test]
fn c3_precedence_pairs_b_c3() {
    let fx = move_fixture();

    // NotMover beats NavUnbound: no mover, and nothing bound.
    let (w, ids) = fx.start(7);
    let b = ids[1];
    let w = with_json(&w, |save| remove_mover(save, b));
    assert_eq!(w.nav(), None);
    assert_eq!(
        validate_move(&w, b, &route_b(1)),
        Err(MoveError::NotMover { entity: b })
    );

    // StartMismatch beats CannotAfford: ten metres from the wrong start.
    let (w, _, b) = tb_world(&fx);
    assert_eq!(
        validate_move(&w, b, &route(1, vec![p(0, 0), p(0, 10)])),
        Err(MoveError::StartMismatch)
    );

    // Both checks in order would have failed: NotMover before the route.
    let (w, _, b) = tb_world(&fx);
    let w = with_json(&w, |save| {
        save["combat"]["definition"]
            .as_object_mut()
            .expect("a definition object")
            .remove("movement")
            .expect("the definition holds movement");
    });
    assert_eq!(
        validate_move(&w, b, &route(1, vec![p(0, 0), p(0, 10)])),
        Err(MoveError::NotMover { entity: b })
    );
}

// ------------------------------ C-4 read-only validation and TB immobility

#[test]
fn c4_validate_move_is_read_only_b_c3() {
    let fx = move_fixture();
    let (mut w, a, b) = tb_world(&fx);
    let before = bytes_of(&w);
    assert_eq!(
        validate_move(&w, b, &route(1, vec![p(10, 0), p(10, 3), p(7, 3)])),
        Ok(4)
    );
    assert_eq!(bytes_of(&w), before, "validation changes nothing");

    // Every C-3 rejection of `perform_move` leaves the same bytes behind.
    let cases: [(EntityId, MoveRoute, MoveError); 3] = [
        (
            a,
            route(1, vec![p(0, 0), p(1, 0)]),
            MoveError::Combat(CombatError::OutOfTurn {
                actor: a,
                active: Some(b),
            }),
        ),
        (
            b,
            route(1, vec![p(0, 0), p(1, 0)]),
            MoveError::StartMismatch,
        ),
        (
            b,
            route(1, vec![p(10, 0), p(10, 10)]),
            MoveError::CannotAfford {
                cost: 7,
                current: 6,
            },
        ),
    ];
    for (actor, move_route, expected) in cases {
        let before = bytes_of(&w);
        assert_eq!(perform_move(&mut w, actor, &move_route), Err(expected));
        assert_eq!(bytes_of(&w), before, "a rejected move changes nothing");
    }

    // The dead-actor rejection is read-only too.
    let mut fight = move_fixture();
    lethal(&mut fight);
    let (mut w, a, b) = tb_world(&fight);
    perform_action(
        &mut w,
        &CombatAction::UseAbility {
            actor: b,
            ability: fight.ability,
            target: a,
        },
    )
    .expect("B acts")
    .expect("a hit");
    let before = bytes_of(&w);
    assert_eq!(
        perform_move(&mut w, a, &route(1, vec![p(0, 0), p(1, 0)])),
        Err(MoveError::Combat(CombatError::DeadActor { actor: a }))
    );
    assert_eq!(bytes_of(&w), before);

    // A turn-based combatant takes no stage-4 route, and a tick moves nobody.
    let (mut w, a, b) = tb_world(&fx);
    assert_eq!(
        w.set_route(b, route_b(2)),
        Err(MoveError::TurnBased { entity: b })
    );
    let positions = [
        raw3(w.movers().get(a).expect("A").position()),
        raw3(w.movers().get(b).expect("B").position()),
    ];
    assert!(tick_with_movement(&mut w).is_empty());
    assert_eq!(
        [
            raw3(w.movers().get(a).expect("A").position()),
            raw3(w.movers().get(b).expect("B").position()),
        ],
        positions,
        "ticking moves no turn-based participant"
    );
}
