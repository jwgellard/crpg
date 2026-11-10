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
    w.combatants().get(e).expect("a combatant").extra_pools()[0].current()
}
fn remove_mover(save: &mut Value, entity: EntityId) {
    let key = serde_json::to_value(entity).expect("the id serializes");
    save["movers"]
        .as_array_mut()
        .expect("a mover list")
        .retain(|entry| entry[0] != key);
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
    assert_eq!(route_cost(1, step(1), u32::MAX), Some(u32::MAX));
    assert_eq!(route_cost(2, step(1), u32::MAX), None);

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
    let outcome =
        perform_move(&mut w, b, &route(2, vec![p(7, 3), p(7, 1)])).expect("two metres cost two");
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
            let key = serde_json::to_value(b).expect("the id serializes");
            let entry = save["transforms"]
                .as_array_mut()
                .expect("a transform list")
                .iter_mut()
                .find(|entry| entry[0] == key)
                .expect("B holds a transform");
            entry[1]["velocity"] = json!([1.0_f32, 0.0_f32, 0.0_f32]);
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
    assert_eq!(
        validate_move(&w, b, &route_b(1)),
        Err(MoveError::NavUnbound)
    );
}

#[test]
fn c3_route_failures_reach_the_mover_b_c3() {
    // `validate_move` inherits every `validate_route` failure, with its
    // exact payload (A-4's literals, the same checker).
    let fx = move_fixture();
    let (w, _, b) = tb_world(&fx);
    let cases = [
        (route(1, vec![p(10, 0)]), MoveError::RouteSize { points: 1 }),
        (
            route(1, vec![p(10, 0), rp((1 << 29) + 1, 0, 0)]),
            MoveError::OutOfBounds { index: 1 },
        ),
        (
            route(1, vec![p(10, 0), p(10, 0)]),
            MoveError::ZeroSegment { index: 0 },
        ),
        (
            // From B's start, one segment reaches at most 267 780 097 raw;
            // two segments sum to 536 215 554, over the 2^28 limit.
            route(1, vec![p(10, 0), rp(268_435_457, 0, 0), rp(0, 0, 0)]),
            MoveError::TooLong {
                length: 536_215_554,
            },
        ),
        (
            MoveRoute {
                revision: NavRevision::of_bytes(b"mesh-b"),
                tag: RouteTag(1),
                points: vec![p(10, 0), p(11, 0)],
            },
            MoveError::StaleRevision,
        ),
    ];
    for (move_route, expected) in cases {
        assert_eq!(validate_move(&w, b, &move_route), Err(expected));
    }
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

// -------------------------------- C-5 / B-C5, B-H1: the journal and version

/// A MOVE `HistoryWorld` with the nav bound, turn-based, ids A then B.
fn history_tb(fx: &Fixture) -> (HistoryWorld, EntityId, EntityId) {
    let mut h = HistoryWorld::new(7);
    h.bind_nav(rev()).expect("nav binds");
    fx.with_spec(|spec| h.start_encounter_in_mode(spec, EncounterMode::TurnBased))
        .expect("the fight starts");
    let ids: Vec<EntityId> = h.world().combatants().iter().map(|(id, _)| id).collect();
    (h, ids[0], ids[1])
}

#[test]
fn c5_history_v3_journal_and_version_b_h1_b_c5() {
    assert_eq!(HISTORY_VERSION, 3);
    assert_eq!(HistoryWorld::new(0).version(), 1);

    let fx = move_fixture();
    let (mut h, _a, b) = history_tb(&fx);
    assert_eq!(h.version(), 1, "a turn-based start keeps version 1");

    let seq = h.last_sequence();
    let outcome = h
        .perform_move(b, &route(1, vec![p(10, 0), p(10, 3), p(7, 3)]))
        .expect("the move is paid from pool 106");
    assert_eq!(
        outcome,
        MoveOutcome {
            actor: b,
            cost: 4,
            from: p(10, 0),
            to: p(7, 3),
        }
    );
    assert_eq!(h.version(), 3, "the first committed Moved sets version 3");
    assert_eq!(pool106(h.world(), b), 2);
    let added = h.read_after(seq, 256).expect("a short read");
    assert_eq!(added.len(), 1, "exactly one envelope");
    assert_eq!(
        added[0].payload,
        HistoryEvent::Moved {
            entity: b,
            points: vec![
                rp(655_360, 0, 0),
                rp(655_360, 0, 196_608),
                rp(458_752, 0, 196_608),
            ],
        }
    );

    // Release and acknowledgement are sticky.
    h.end_encounter().expect("release");
    assert_eq!(h.version(), 3, "release keeps 3");
    let last = h.last_sequence();
    h.acknowledge(last).expect("acknowledge");
    assert_eq!(h.version(), 3, "acknowledgement keeps 3");

    // A rejected move commits nothing and leaves version 1.
    let (mut h, _, b) = history_tb(&fx);
    let before = serde_json::to_vec(&h).expect("serializes");
    assert_eq!(
        h.perform_move(b, &route(1, vec![p(0, 0), p(1, 0)])),
        Err(HistoryError::Move(MoveError::StartMismatch))
    );
    assert_eq!(h.version(), 1);
    assert_eq!(serde_json::to_vec(&h).expect("serializes"), before);
    assert_eq!(pool106(h.world(), b), 6, "nothing was spent");

    // An RT-capable wrapper writes 2, and only a `Moved` raises it to 3.
    let (mut h, _, b) = history_tb(&move_rt());
    assert_eq!(h.version(), 2, "an RT-capable start still writes version 2");
    h.perform_move(b, &route(1, vec![p(10, 0), p(10, 1)]))
        .expect("one step");
    assert_eq!(h.version(), 3);

    // A move that is not A's commits nothing, so version 1 stays.
    let (mut h, a, _) = history_tb(&fx);
    assert!(h
        .perform_move(a, &route(2, vec![p(10, 0), p(11, 0)]))
        .is_err());
    assert_eq!(h.version(), 1);
}

#[test]
fn c5_history_loader_versions_b_h2() {
    let fx = move_fixture();
    let (mut h, _, b) = history_tb(&fx);
    h.perform_move(b, &route(1, vec![p(10, 0), p(10, 1)]))
        .expect("one step");
    let value = serde_json::to_value(&h).expect("serializes");

    // Versions 1 and 2 may not retain a v3 event.
    for forged in [1, 2] {
        let mut edited = value.clone();
        edited["version"] = json!(forged);
        let err = serde_json::from_value::<HistoryWorld>(edited)
            .expect_err("a v3 event below version 3 fails")
            .to_string();
        assert!(
            err.contains("history journal holds a v3 event below version 3"),
            "version {forged}: {err}"
        );
    }
    // 0 and 4 are unknown versions.
    for forged in [0, 4] {
        let mut edited = value.clone();
        edited["version"] = json!(forged);
        let err = serde_json::from_value::<HistoryWorld>(edited)
            .expect_err("an unknown version fails")
            .to_string();
        assert!(
            err.contains("unknown history version"),
            "version {forged}: {err}"
        );
    }

    // 1, 2 and 3 are read as written.
    let fresh = HistoryWorld::new(0);
    let text = serde_json::to_vec(&fresh).expect("serializes");
    let loaded: HistoryWorld = serde_json::from_slice(&text).expect("version 1 loads");
    assert_eq!(loaded.version(), 1);

    let (rt, _, _) = history_tb(&move_rt());
    assert_eq!(rt.version(), 2);
    let text = serde_json::to_vec(&rt).expect("serializes");
    let loaded: HistoryWorld = serde_json::from_slice(&text).expect("version 2 loads");
    assert_eq!(loaded.version(), 2);

    let bytes = serde_json::to_vec(&h).expect("serializes");
    let loaded: HistoryWorld = serde_json::from_slice(&bytes).expect("version 3 loads");
    assert_eq!(loaded.version(), 3);
    assert_eq!(
        serde_json::to_vec(&loaded).expect("serializes"),
        bytes,
        "a v3 wrapper round-trips byte-identically"
    );
}

// ----------------------------------------- B-H3: the `moved` payload decoder

fn entity_text() -> &'static str {
    r#"{"index":1,"generation":1}"#
}
fn point_items(n: usize) -> String {
    let items: Vec<&str> = (0..n).map(|_| "[0,0,0]").collect();
    items.join(",")
}
/// A JSON array of `n` whole-number points.
fn points_text(n: usize) -> String {
    format!("[{}]", point_items(n))
}
fn decode_moved(value: &str) -> String {
    serde_json::from_str::<HistoryEvent>(&format!("{{\"type\":\"moved\",\"value\":{value}}}"))
        .expect_err("the payload is invalid")
        .to_string()
}

#[test]
fn c5_moved_payload_decoder_b_h3() {
    let e = entity_text();
    let two = points_text(2);

    // A well-formed payload decodes, and encodes with raw `i32` points.
    let valid = format!("{{\"entity\":{e},\"points\":{two}}}");
    let decoded: HistoryEvent =
        serde_json::from_str(&format!("{{\"type\":\"moved\",\"value\":{valid}}}"))
            .expect("decodes");
    assert!(matches!(decoded, HistoryEvent::Moved { .. }));

    let mut world = HistoryWorld::new(0);
    let _first = world.spawn(EntityMeta {}).expect("spawn");
    let second = world.spawn(EntityMeta {}).expect("spawn");
    assert_eq!((second.index(), second.generation()), (1, 1));
    assert_eq!(
        serde_json::to_string(&HistoryEvent::Moved {
            entity: second,
            points: vec![rp(1, 2, 3), rp(-4, 5, -6)],
        })
        .expect("serializes"),
        concat!(
            r#"{"type":"moved","value":{"entity":{"index":1,"generation":1},"#,
            r#""points":[[1,2,3],[-4,5,-6]]}}"#
        )
    );

    // The two key texts.
    assert!(decode_moved(&format!(
        "{{\"entity\":{e},\"entity\":{e},\"points\":{two}}}"
    ))
    .contains("duplicate history payload field `entity`"));
    assert!(decode_moved(&format!(
        "{{\"entity\":{e},\"points\":{two},\"points\":{two}}}"
    ))
    .contains("duplicate history payload field `points`"));
    assert!(
        decode_moved(&format!("{{\"entity\":{e},\"points\":{two},\"extra\":1}}"))
            .contains("unknown history payload field")
    );
    assert!(
        decode_moved(&format!("{{\"points\":{two}}}")).contains("history payload misses `entity`")
    );
    assert!(
        decode_moved(&format!("{{\"entity\":{e}}}")).contains("history payload misses `points`")
    );

    // The bounded sequence: at least two, at most 64, streamed.
    assert!(
        decode_moved(&format!("{{\"entity\":{e},\"points\":[[0,0,0]]}}"))
            .contains("history move points below 2")
    );
    assert!(decode_moved(&format!("{{\"entity\":{e},\"points\":[]}}"))
        .contains("history move points below 2"));
    assert!(decode_moved(&format!(
        "{{\"entity\":{e},\"points\":{}}}",
        points_text(65)
    ))
    .contains("history move points exceed 64"));
    // The 65th element is never decoded, so its content cannot matter.
    assert!(decode_moved(&format!(
        "{{\"entity\":{e},\"points\":[{},\"x\"]}}",
        point_items(64)
    ))
    .contains("history move points exceed 64"));
    // 64 points are fine.
    serde_json::from_str::<HistoryEvent>(&format!(
        "{{\"type\":\"moved\",\"value\":{{\"entity\":{e},\"points\":{}}}}}",
        points_text(64)
    ))
    .expect("64 points decode");

    // Stream order decides which fault wins; the missing-key checks come
    // only after the map ends.
    assert!(decode_moved(&format!(
        "{{\"entity\":{e},\"entity\":{e},\"extra\":1,\"points\":{two}}}"
    ))
    .contains("duplicate history payload field `entity`"));
    assert!(decode_moved(&format!(
        "{{\"extra\":1,\"entity\":{e},\"entity\":{e},\"points\":{two}}}"
    ))
    .contains("unknown history payload field"));
    assert!(decode_moved("{\"points\":[[0,0,0]]}").contains("history move points below 2"));
}

// ------------------------------- C-6 / B-H4: the persisted TurnMovement

#[test]
fn c6_definition_persistence_b_h4_b_c2() {
    let fx = move_fixture();
    let (w, _ids) = fx.start(7);
    let combat = w.combat().expect("an encounter");
    assert_eq!(
        combat.definition.movement,
        Some(TurnMovement {
            measure: Measure::Euclidean,
            pool: Ulid::from_u128(106),
            step_cost: 1,
        })
    );
    assert_eq!(
        serde_json::to_string(&combat.definition.movement).expect("serializes"),
        format!(
            "{{\"measure\":\"euclidean\",\"pool\":\"{}\",\"step_cost\":1}}",
            uid(106)
        )
    );

    // Without `movement` in the ruleset the definition holds none.
    let mut bare = move_fixture();
    edit_ruleset(&mut bare, |r| {
        r.as_object_mut()
            .expect("a ruleset object")
            .remove("movement");
    });
    let (bare_world, _) = bare.start(7);
    assert_eq!(
        bare_world
            .combat()
            .expect("an encounter")
            .definition
            .movement,
        None
    );

    // The standard fixture keeps the 15-field legacy form with no movement.
    let (plain, _) = Fixture::standard().start(1);
    let definition =
        serde_json::to_value(&plain).expect("serializes")["combat"]["definition"].clone();
    let object = definition.as_object().expect("an object");
    assert_eq!(object.len(), 15, "the legacy definition writes 15 fields");
    assert!(!object.contains_key("movement"));
    // Both repr branches read `movement` with `#[serde(default)]`.
    let bytes = serde_json::to_vec(&plain).expect("serializes");
    let loaded: World = serde_json::from_slice(&bytes).expect("the legacy form loads");
    assert_eq!(
        loaded.combat().expect("an encounter").definition.movement,
        None
    );
    assert_eq!(loaded, plain);

    // The loader rejects an alien pool, then a zero step cost.
    let err = try_with_json(&w, |save| {
        save["combat"]["definition"]["movement"]["pool"] = json!(uid(999));
    })
    .expect_err("the movement pool must be one of the definition's pools");
    assert!(
        err.contains("combat movement pool outside its pools"),
        "{err}"
    );
    let err = try_with_json(&w, |save| {
        save["combat"]["definition"]["movement"]["step_cost"] = json!(0);
    })
    .expect_err("a zero movement step cost is refused");
    assert!(err.contains("combat movement step cost is zero"), "{err}");

    // Start: a movement pool outside the ruleset's templates.
    let mut alien = move_fixture();
    edit_ruleset(&mut alien, |r| {
        r["movement"]["turn"]["pool"] = json!(uid(999));
    });
    let mut world = World::new(7);
    assert_eq!(
        alien.with_spec(|spec| start_encounter(&mut world, spec)),
        Err(CombatError::MissingPool {
            pool: Ulid::from_u128(999)
        })
    );

    // Start: a zero step cost names that pool's maximum.
    let mut free = move_fixture();
    edit_ruleset(&mut free, |r| {
        r["movement"]["turn"]["step_cost"] = json!(0);
    });
    let mut world = World::new(7);
    assert_eq!(
        free.with_spec(|spec| start_encounter(&mut world, spec)),
        Err(CombatError::InvalidCost { cost: 0, max: 6 })
    );
}

#[test]
fn c6_legacy_shape_with_movement_b_h4() {
    let fx = move1();
    let (mut world, ids) = fx.start(7);
    let b = ids[1];
    let keys = {
        let combat = world.combat().expect("an encounter");
        assert_eq!(
            serde_json::to_string(&combat.definition.movement).expect("serializes"),
            format!(
                "{{\"measure\":\"euclidean\",\"pool\":\"{}\",\"step_cost\":1}}",
                uid(105)
            )
        );
        top_level_keys(
            serde_json::to_string(&combat.definition)
                .expect("serializes")
                .as_bytes(),
        )
    };
    for name in ["abilities", "pools", "effects", "movement"] {
        assert!(
            keys.iter().any(|key| key == name),
            "{name} missing from {keys:?}"
        );
    }
    assert_eq!(keys.len(), 19, "the non-legacy form: {keys:?}");
    assert_eq!(
        keys.last().map(String::as_str),
        Some("movement"),
        "movement is the last definition field"
    );

    let bytes = serde_json::to_vec(&world).expect("serializes");
    let loaded: World = serde_json::from_slice(&bytes).expect("loads");
    assert_eq!(loaded, world);
    assert!(loaded
        .combat()
        .expect("an encounter")
        .definition
        .movement
        .is_some());
    assert_eq!(
        serde_json::to_vec(&loaded).expect("serializes"),
        bytes,
        "a legacy-shaped definition with movement re-serializes identically"
    );

    // The T036a creation checks run before the movement pool check.
    let mut broken = move_fixture();
    edit_ruleset(&mut broken, |r| {
        r["movement"]["turn"]["pool"] = json!(uid(999));
        r["movement"]["turn"]["step"] = json!({"type": "constant", "value": 0});
    });
    let mut empty = World::new(7);
    assert_eq!(
        broken.with_spec(|spec| start_encounter(&mut empty, spec)),
        Err(CombatError::InvalidStep {
            placement: Ulid::from_u128(111)
        })
    );

    // The movement pool is the primary pool, so a move spends it.
    world.bind_nav(rev());
    let outcome =
        perform_move(&mut world, b, &route(1, vec![p(10, 0), p(11, 0)])).expect("one step");
    assert_eq!(
        (
            outcome.cost,
            world
                .combatants()
                .get(b)
                .expect("B")
                .action_pool()
                .current()
        ),
        (1, 0)
    );

    // `movement` persists after `initiative`.
    let mut drawn = move1();
    support::draws::set_initiative_rule(&mut drawn, "1d6", &[]);
    let (world, _) = drawn.start(7);
    let keys = top_level_keys(
        serde_json::to_string(&world.combat().expect("an encounter").definition)
            .expect("serializes")
            .as_bytes(),
    );
    let initiative = keys
        .iter()
        .position(|key| key == "initiative")
        .expect("an initiative key");
    assert_eq!(
        keys.get(initiative + 1).map(String::as_str),
        Some("movement"),
        "{keys:?}"
    );
}

// ------------------------------------------- T036c-fix: the seven survivors

/// The current balance of the extra pool `id` of `e`.
fn extra_current(w: &World, e: EntityId, id: u128) -> u32 {
    w.combatants()
        .get(e)
        .expect("a combatant")
        .extra_pools()
        .iter()
        .find(|entry| entry.id().0 == pl(id))
        .expect("the extra pool exists")
        .current()
}

/// MOVE with one more extra pool, `uid(107)` holding 9, authored before 106.
fn move_two_extras() -> Fixture {
    let mut fx = move_fixture();
    edit_ruleset(&mut fx, |r| {
        let pools = r["pools"].as_array_mut().unwrap();
        let at = pools
            .iter()
            .position(|pool| pool["id"] == json!(uid(106)))
            .expect("pool 106 is authored");
        pools.insert(
            at,
            json!({"id": uid(107), "max": 9, "refresh": {"type": "on_turn_start"}}),
        );
    });
    fx
}

#[test]
fn c1_route_cost_overflow_in_u64() {
    // ceil(9223372036854775809 / 1) * 2 = 2^64 + 2 does not fit `u64`: `None`,
    // never the wrapped 2.
    assert_eq!(route_cost(9_223_372_036_854_775_809, step(1), 2), None);
    // 2^63 * 2 = 2^64 also overflows `u64` (the wrap would give `Some(0)`).
    assert_eq!(route_cost(9_223_372_036_854_775_808, step(1), 2), None);
    // Control: 2^64 - 2 fits `u64` but not `u32`: `None` all the same.
    assert_eq!(route_cost(9_223_372_036_854_775_807, step(1), 2), None);
    // Control: a small honest product is exact.
    assert_eq!(route_cost(3, step(1), 2), Some(6));
}

#[test]
fn c3_combat_checks_precede_the_route_checks_b_c3() {
    // A is out of turn (B holds it) and the route does not start at A's
    // position: the Combat check wins over StartMismatch (Reading 8).
    let fx = move_fixture();
    let (mut w, a, b) = tb_world(&fx);
    let wrong_start = route(1, vec![p(5, 5), p(6, 5)]);
    let expected = Err(MoveError::Combat(CombatError::OutOfTurn {
        actor: a,
        active: Some(b),
    }));
    assert_eq!(validate_move(&w, a, &wrong_start), expected);
    let before = bytes_of(&w);
    assert_eq!(
        perform_move(&mut w, a, &wrong_start),
        Err(MoveError::Combat(CombatError::OutOfTurn {
            actor: a,
            active: Some(b)
        }))
    );
    assert_eq!(bytes_of(&w), before);
    // Control: the same route from the turn holder is the route failure.
    assert_eq!(
        validate_move(&w, b, &wrong_start),
        Err(MoveError::StartMismatch)
    );
}

#[test]
fn c4_movement_pool_is_matched_by_id_among_extras_b_c3_b_c4() {
    let fx = move_two_extras();
    let (mut w, _a, b) = tb_world(&fx);
    // The other extra pool is authored first.
    let order: Vec<Ulid> = w
        .combatants()
        .get(b)
        .expect("B")
        .extra_pools()
        .iter()
        .map(|entry| entry.id().0)
        .collect();
    assert_eq!(order, vec![pl(107), pl(106)]);
    assert_eq!(extra_current(&w, b, 107), 9);
    assert_eq!(extra_current(&w, b, 106), 6);

    // The balance read is pool 106's: ten metres is seven steps, over six.
    assert_eq!(
        validate_move(&w, b, &route(1, vec![p(10, 0), p(10, 10)])),
        Err(MoveError::CannotAfford {
            cost: 7,
            current: 6
        })
    );
    // The spend lands on pool 106 and only there.
    let outcome = perform_move(&mut w, b, &route(2, vec![p(10, 0), p(10, 3), p(7, 3)]))
        .expect("four steps fit six");
    assert_eq!(outcome.cost, 4);
    assert_eq!(extra_current(&w, b, 106), 2, "the movement pool pays");
    assert_eq!(extra_current(&w, b, 107), 9, "the other pool is untouched");
}

#[test]
fn c5_version_never_decreases_after_a_move_b_h1() {
    let fx = move_rt();
    let (mut h, _a, b) = history_tb(&fx);
    assert_eq!(h.version(), 2);
    h.perform_move(b, &route(1, vec![p(10, 0), p(10, 1)]))
        .expect("one step");
    assert_eq!(h.version(), 3);

    // A later RT-capable commit (an accepted EndTurn) keeps version 3.
    h.perform_action(&CombatAction::EndTurn { actor: b })
        .expect("B ends the turn");
    assert_eq!(h.version(), 3, "a commit never lowers the version");
    let bytes = serde_json::to_vec(&h).expect("serializes");
    let loaded: HistoryWorld = serde_json::from_slice(&bytes).expect("version 3 loads");
    assert_eq!(loaded.version(), 3, "and the save reloads as 3");
    assert_eq!(serde_json::to_vec(&loaded).expect("serializes"), bytes);
}

#[test]
fn c6_legacy_form_reads_movement_b_h4() {
    let (plain, _) = Fixture::standard().start(1);
    let loaded = with_json(&plain, |save| {
        let definition = save["combat"]["definition"]
            .as_object_mut()
            .expect("a definition object");
        assert_eq!(definition.len(), 15, "the legacy form");
        let pool = definition["pool_id"].clone();
        definition.insert(
            "movement".to_string(),
            json!({"measure": "euclidean", "pool": pool, "step_cost": 1}),
        );
    });
    let definition = &loaded.combat().expect("an encounter").definition;
    assert_eq!(
        definition.movement,
        Some(TurnMovement {
            measure: Measure::Euclidean,
            pool: definition.pool_id,
            step_cost: 1,
        })
    );
}

#[test]
fn c6_pool_checks_precede_the_movement_checks_b_h4() {
    let (w, _) = move_fixture().start(7);
    let err = try_with_json(&w, |save| {
        save["combat"]["definition"]["pools"] = json!([]);
    })
    .expect_err("a definition with no pools is refused");
    assert!(err.contains("combat definition declares no pools"), "{err}");
    assert!(!err.contains("movement"), "{err}");
}

#[test]
fn c4_move_clears_the_actors_route_b_c4() {
    let fx = move_fixture();
    let (w, _a, b) = tb_world(&fx);
    let injected = serde_json::to_value(route(9, vec![p(10, 0), p(12, 0)])).expect("serializes");
    let mut w = with_json(&w, |save| {
        let key = serde_json::to_value(b).expect("the id serializes");
        let entry = save["movers"]
            .as_array_mut()
            .expect("a mover list")
            .iter_mut()
            .find(|entry| entry[0] == key)
            .expect("B holds a mover");
        entry[1]["route"] = json!({"route": injected, "segment": 0, "along": 0});
    });
    assert!(
        w.movers().get(b).expect("a mover").route().is_some(),
        "the injected route loaded"
    );
    perform_move(&mut w, b, &route(1, vec![p(10, 0), p(11, 0)])).expect("one step");
    assert!(
        w.movers().get(b).expect("a mover").route().is_none(),
        "the move clears the route"
    );
}
