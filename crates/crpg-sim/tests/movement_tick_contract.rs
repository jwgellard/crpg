//! T036b two-keys suite: stage 4, movement facts and the W3 stops. Written
//! from `tasks/T036.md` alone (B-W1–B-W4, B-T1–B-T4, B-1–B-7, B-9); expected
//! values are hand arithmetic and the contract's literal bit patterns.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_sim::*;
use serde_json::{json, Value};
use support::creature::{add_outsider, enlist, with_json};
use support::rt::{bytes_of, elapsed, make_lethal, use_action, value_of};
use support::surface::{always_success, set_stat_raw};
use support::{uid, Fixture};

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
fn rev() -> NavRevision {
    NavRevision::of_bytes(b"mesh-a")
}
fn other() -> NavRevision {
    NavRevision::of_bytes(b"mesh-b")
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
/// The MOVE fixture of the contract.
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
fn move_rt() -> Fixture {
    let mut fx = move_fixture();
    fx.ruleset.round_ticks = Some(4);
    fx
}
fn lethal(fx: &mut Fixture) {
    always_success(fx);
    make_lethal(fx, 99);
}
fn start_in(w: &mut World, fx: &Fixture, mode: EncounterMode) -> Result<(), CombatError> {
    fx.with_spec(|spec| start_encounter_in_mode(w, spec, mode))
}
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
/// A MOVE world with the given placements registered (in order), nav `rev`.
fn registered(fx: &Fixture, placements: &[u128]) -> (World, Vec<EntityId>) {
    let mut w = World::new(7);
    let ids = placements
        .iter()
        .map(|n| fx.register(&mut w, pl(*n)).unwrap())
        .collect();
    w.bind_nav(rev());
    (w, ids)
}
fn bits(w: &World, e: EntityId) -> ([u32; 3], [u32; 3]) {
    let t = w.transforms().get(e).expect("a transform");
    (t.position.map(f32::to_bits), t.velocity.map(f32::to_bits))
}
fn mirror_bits(position: [i32; 3]) -> [u32; 3] {
    position.map(|r| to_f32(Fx16_16::from_raw(r)).to_bits())
}
fn tamper_velocity(w: &mut World, e: EntityId) {
    w.transforms_mut().get_mut(e).unwrap().velocity = [1.0, 0.0, 0.0];
}
/// Edits one mover's route inside a saved world's JSON (the persisted
/// `ActiveRoute` shape: `route`, `segment`, `along`).
fn set_mover_route(save: &mut Value, entity: EntityId, tag: u64, points: Value) {
    let key = serde_json::to_value(entity).expect("the id serializes");
    let entries = save["movers"].as_array_mut().expect("a mover list");
    let entry = entries
        .iter_mut()
        .find(|entry| entry[0] == key)
        .expect("the entity holds a mover");
    entry[1]["route"] = json!({
        "route": {
            "revision": serde_json::to_value(rev()).expect("the revision serializes"),
            "tag": tag,
            "points": points,
        },
        "segment": 0,
        "along": 0,
    });
}
/// The RT two-mover scene of B-5: A walks [p(0,0), p(3,4)], B walks
/// [p(10,0), p(10,2), p(8,2)].
fn rt_walking() -> (World, EntityId, EntityId) {
    let fx = move_rt();
    let mut w = World::new(7);
    w.bind_nav(rev());
    start_in(&mut w, &fx, EncounterMode::RealTime).unwrap();
    let (a, b) = (who(&w, 111), who(&w, 112));
    w.set_route(a, route(1, vec![p(0, 0), p(3, 4)])).unwrap();
    w.set_route(b, route(2, vec![p(10, 0), p(10, 2), p(8, 2)]))
        .unwrap();
    (w, a, b)
}

// ------------------------------------------------------------ B-1 single walk

#[test]
fn single_walk_b1() {
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111]);
    let a = ids[0];
    w.set_route(a, route(7, vec![p(0, 0), p(3, 4)])).unwrap();
    // W5: no tick, no movement.
    assert_eq!(raw3(w.movers().get(a).unwrap().position()), [0, 0, 0]);
    let want = [
        [39_321, 0, 52_428],
        [78_643, 0, 104_857],
        [117_964, 0, 157_286],
        [157_286, 0, 209_715],
        [196_608, 0, 262_144],
        [196_608, 0, 262_144],
    ];
    for (i, point) in want.iter().enumerate() {
        let facts = tick_with_movement(&mut w);
        let k = i + 1;
        assert_eq!(
            raw3(w.movers().get(a).unwrap().position()),
            *point,
            "tick {k}"
        );
        match k {
            1 => assert_eq!(
                bits(&w, a),
                ([0x3f19_9900, 0, 0x3f4c_cc00], [0x3f19_9900, 0, 0x3f4c_cc00]),
                "tick 1 position and velocity mirrors"
            ),
            4 => {
                let active = w.movers().get(a).unwrap().route().unwrap();
                assert_eq!((active.segment(), active.along()), (0, 262_144));
            }
            5 => {
                assert_eq!(bits(&w, a).1, mirror_bits([39_322, 0, 52_429]));
                assert_eq!(
                    facts,
                    vec![MovementFact::Arrived {
                        entity: a,
                        tag: RouteTag(7)
                    }]
                );
            }
            6 => {
                assert!(w.movers().get(a).unwrap().route().is_none());
                assert_eq!(bits(&w, a), ([0x4040_0000, 0, 0x4080_0000], [0; 3]));
            }
            _ => {}
        }
        if k != 5 {
            assert!(facts.is_empty(), "tick {k} reports nothing");
        }
    }
}

// ------------------------------------------------------------- B-2 carry over

#[test]
fn carry_over_b2() {
    let mut fx = move_fixture();
    set_stat_raw(&mut fx, pl(121), "pace", 98_304);
    let (mut w, ids) = registered(&fx, &[111]);
    let a = ids[0];
    w.set_route(a, route(1, vec![p(0, 0), p(1, 0), p(1, 2)]))
        .unwrap();
    let facts = tick_with_movement(&mut w);
    assert!(facts.is_empty());
    assert_eq!(
        raw3(w.movers().get(a).unwrap().position()),
        [65_536, 0, 32_768]
    );
    let active = w.movers().get(a).unwrap().route().unwrap();
    assert_eq!((active.segment(), active.along()), (1, 32_768));
    let facts = tick_with_movement(&mut w);
    assert_eq!(
        facts,
        vec![MovementFact::Arrived {
            entity: a,
            tag: RouteTag(1)
        }]
    );
    assert_eq!(
        raw3(w.movers().get(a).unwrap().position()),
        [65_536, 0, 131_072]
    );
}

// --------------------------------------------------- B-3 order, independence

#[test]
fn order_and_independence_b3() {
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111, 112]);
    let (a, b) = (ids[0], ids[1]);
    // B's route is set first; facts still come out in ascending EntityId.
    w.set_route(b, route(2, vec![p(10, 0), rp(655_360, 0, 32_768)]))
        .unwrap();
    w.set_route(a, route(1, vec![p(0, 0), p(1, 0)])).unwrap();
    let facts = tick_with_movement(&mut w);
    assert_eq!(
        facts,
        vec![
            MovementFact::Arrived {
                entity: a,
                tag: RouteTag(1)
            },
            MovementFact::Arrived {
                entity: b,
                tag: RouteTag(2)
            },
        ]
    );

    // A world whose movers hold no route changes only the tick counter.
    let (mut w, _) = registered(&fx, &[111, 112]);
    let before = value_of(&w);
    tick(&mut w);
    let mut after = value_of(&w);
    let next = before["tick"].as_u64().unwrap() + 1;
    assert_eq!(after["tick"], json!(next));
    after["tick"] = before["tick"].clone();
    assert_eq!(after, before);

    // A reused index sits after B in the store but before it by EntityId.
    let mut w = World::new(7);
    let a = fx.register(&mut w, pl(111)).unwrap();
    let b = fx.register(&mut w, pl(112)).unwrap();
    assert_eq!((a.index(), a.generation()), (0, 1));
    assert_eq!((b.index(), b.generation()), (1, 1));
    assert!(w.despawn(a));
    let c = fx.register(&mut w, pl(113)).unwrap();
    assert_eq!((c.index(), c.generation()), (0, 2));
    w.bind_nav(rev());
    w.set_route(b, route(2, vec![p(10, 0), rp(655_360, 0, 32_768)]))
        .unwrap();
    w.set_route(c, route(3, vec![p(0, 10), p(1, 10)])).unwrap();
    let facts = tick_with_movement(&mut w);
    assert_eq!(
        facts,
        vec![
            MovementFact::Arrived {
                entity: c,
                tag: RouteTag(3)
            },
            MovementFact::Arrived {
                entity: b,
                tag: RouteTag(2)
            },
        ]
    );
}

// ----------------------------------------------------------- B-4 dead cancel

#[test]
fn dead_combatant_cancels_b4() {
    let mut fx = move_rt();
    lethal(&mut fx);
    let mut w = World::new(7);
    w.bind_nav(rev());
    start_in(&mut w, &fx, EncounterMode::RealTime).unwrap();
    let (a, b) = (who(&w, 111), who(&w, 112));
    w.set_route(a, route(1, vec![p(0, 0), p(3, 4)])).unwrap();
    w.set_route(b, route(2, vec![p(10, 0), p(10, 2)])).unwrap();
    let order = perform_action(&mut w, &use_action(a, fx.ability, b)).unwrap();
    assert!(order.is_none(), "an RT use stores an order");
    let facts = tick_with_movement(&mut w);
    assert!(w.combatants().get(b).unwrap().dead());
    assert_eq!(
        facts,
        vec![MovementFact::Cancelled {
            entity: b,
            tag: RouteTag(2),
            reason: CancelReason::Dead
        }]
    );
    assert!(w.movers().get(b).unwrap().route().is_none());
    assert_eq!(raw3(w.movers().get(b).unwrap().position()), [655_360, 0, 0]);
    assert_eq!(
        raw3(w.movers().get(a).unwrap().position()),
        [39_321, 0, 52_428]
    );
}

#[test]
fn dead_record_cancels_b4() {
    let mut fx = move_fixture();
    lethal(&mut fx);
    let (mut w, ids) = fx.start(7);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut w, &use_action(b, fx.ability, a)).unwrap();
    assert!(w.combatants().get(a).unwrap().dead());
    end_encounter(&mut w).unwrap();
    assert!(w.creatures().get(a).unwrap().dead(), "a corpse record");
    assert!(w.movers().get(a).is_some(), "the corpse keeps its mover");
    w.bind_nav(rev());
    let mut w = with_json(&w, |save| {
        set_mover_route(save, a, 5, json!([[0, 0, 0], [196_608, 0, 262_144]]));
    });
    let facts = tick_with_movement(&mut w);
    assert_eq!(
        facts,
        vec![MovementFact::Cancelled {
            entity: a,
            tag: RouteTag(5),
            reason: CancelReason::Dead
        }]
    );
    assert!(w.movers().get(a).unwrap().route().is_none());
    assert_eq!(raw3(w.movers().get(a).unwrap().position()), [0, 0, 0]);
}

// ------------------------------------------------------------ B-5 transitions

#[test]
fn tb_start_stops_entering_participants_b5() {
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111, 112, 113]);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    w.set_route(a, route(1, vec![p(0, 0), p(3, 4)])).unwrap();
    w.set_route(b, route(2, vec![p(10, 0), p(10, 2)])).unwrap();
    w.set_route(c, route(3, vec![p(0, 10), p(12, 10)])).unwrap();
    assert!(tick_with_movement(&mut w).is_empty());
    assert_eq!(
        raw3(w.movers().get(a).unwrap().position()),
        [39_321, 0, 52_428]
    );
    assert_eq!(bits(&w, a).1, [0x3f19_9900, 0, 0x3f4c_cc00]);
    assert_eq!(
        raw3(w.movers().get(b).unwrap().position()),
        [655_360, 0, 32_768]
    );
    assert_eq!(bits(&w, b).1, [0, 0, 0x3f00_0000]);
    assert_eq!(
        raw3(w.movers().get(c).unwrap().position()),
        [65_536, 0, 655_360]
    );
    assert_eq!(bits(&w, c).1, [0x3f80_0000, 0, 0]);

    start_in(&mut w, &fx, EncounterMode::TurnBased).unwrap();
    for (e, tag) in [(a, None), (b, None), (c, Some(RouteTag(3)))] {
        assert_eq!(w.movers().get(e).unwrap().route().map(|r| r.tag()), tag);
    }
    for e in [a, b] {
        assert_eq!(bits(&w, e).1, [0; 3], "the stop rewrites zero velocity");
    }
    assert_eq!(
        bits(&w, c).1,
        [0x3f80_0000, 0, 0],
        "an outsider is not stopped"
    );
    assert_eq!(
        raw3(w.movers().get(a).unwrap().position()),
        [39_321, 0, 52_428]
    );
    assert_eq!(
        raw3(w.movers().get(b).unwrap().position()),
        [655_360, 0, 32_768]
    );
}

#[test]
fn tb_start_stops_members_and_leaves_unrouted_mirrors_b5() {
    let fx = move_fixture();
    let mut w = World::new(7);
    let c = fx.register(&mut w, pl(113)).unwrap();
    w.bind_nav(rev());
    w.set_route(c, route(3, vec![p(0, 10), p(12, 10)])).unwrap();
    tamper_velocity(&mut w, c);
    fx.with_spec(|spec| {
        start_encounter_with_members(
            &mut w,
            spec,
            EncounterMode::TurnBased,
            &[Entrant {
                entity: c,
                initiative: 5,
            }],
        )
    })
    .unwrap();
    assert!(w.combatants().get(c).is_some());
    assert!(w.movers().get(c).unwrap().route().is_none());
    assert_eq!(bits(&w, c).1, [0; 3]);
    assert_eq!(raw3(w.movers().get(c).unwrap().position()), [0, 0, 655_360]);

    // B-T1's no-route branch: an unrouted mover's mirror is untouched.
    let (mut w, ids) = registered(&fx, &[111]);
    let a = ids[0];
    tamper_velocity(&mut w, a);
    start_in(&mut w, &fx, EncounterMode::TurnBased).unwrap();
    assert!(w.combatants().get(a).is_some(), "A entered from its record");
    assert!(w.movers().get(a).unwrap().route().is_none());
    assert_eq!(bits(&w, a).1, [0x3f80_0000, 0, 0]);
}

#[test]
fn rt_start_stops_nobody_b5() {
    let fx = move_rt();
    let (mut w, ids) = registered(&fx, &[111, 112]);
    let (a, b) = (ids[0], ids[1]);
    w.set_route(a, route(1, vec![p(0, 0), p(3, 4)])).unwrap();
    w.set_route(b, route(2, vec![p(10, 0), p(10, 2)])).unwrap();
    assert!(tick_with_movement(&mut w).is_empty());
    start_in(&mut w, &fx, EncounterMode::RealTime).unwrap();
    assert_eq!(
        w.movers().get(a).unwrap().route().unwrap().tag(),
        RouteTag(1)
    );
    assert_eq!(
        w.movers().get(b).unwrap().route().unwrap().tag(),
        RouteTag(2)
    );
    assert_eq!(bits(&w, a).1, [0x3f19_9900, 0, 0x3f4c_cc00]);
    assert_eq!(bits(&w, b).1, [0, 0, 0x3f00_0000]);
    assert_eq!(
        raw3(w.movers().get(a).unwrap().position()),
        [39_321, 0, 52_428]
    );
    assert_eq!(
        raw3(w.movers().get(b).unwrap().position()),
        [655_360, 0, 32_768]
    );
    // The routes are kept, so the movers keep walking.
    let facts = tick_with_movement(&mut w);
    assert!(facts.is_empty());
    assert_eq!(
        raw3(w.movers().get(a).unwrap().position()),
        [78_643, 0, 104_857]
    );
    assert_eq!(
        raw3(w.movers().get(b).unwrap().position()),
        [655_360, 0, 65_536]
    );
}

#[test]
fn tb_join_stops_the_joiner_b5() {
    let fx = move_fixture();
    let mut w = World::new(7);
    w.bind_nav(rev());
    start_in(&mut w, &fx, EncounterMode::TurnBased).unwrap();
    let c = fx.register(&mut w, pl(113)).unwrap();
    w.set_route(c, route(3, vec![p(0, 10), p(12, 10)])).unwrap();
    assert!(tick_with_movement(&mut w).is_empty());
    assert_eq!(bits(&w, c).1, [0x3f80_0000, 0, 0]);
    join_encounter(
        &mut w,
        &Entrant {
            entity: c,
            initiative: 5,
        },
    )
    .unwrap();
    assert!(w.combatants().get(c).is_some());
    assert!(w.movers().get(c).unwrap().route().is_none());
    assert_eq!(bits(&w, c).1, [0; 3]);
    assert_eq!(
        raw3(w.movers().get(c).unwrap().position()),
        [65_536, 0, 655_360]
    );
}

#[test]
fn rt_join_stops_nobody_b5() {
    let fx = move_rt();
    let mut w = World::new(7);
    w.bind_nav(rev());
    start_in(&mut w, &fx, EncounterMode::RealTime).unwrap();
    let c = fx.register(&mut w, pl(113)).unwrap();
    w.set_route(c, route(3, vec![p(0, 10), p(12, 10)])).unwrap();
    assert!(tick_with_movement(&mut w).is_empty());
    join_encounter(
        &mut w,
        &Entrant {
            entity: c,
            initiative: 5,
        },
    )
    .unwrap();
    assert!(w.combatants().get(c).is_some());
    assert_eq!(
        w.movers().get(c).unwrap().route().unwrap().tag(),
        RouteTag(3)
    );
    assert_eq!(bits(&w, c).1, [0x3f80_0000, 0, 0]);
    // The RT joiner keeps walking.
    let facts = tick_with_movement(&mut w);
    assert!(facts.is_empty());
    assert_eq!(
        raw3(w.movers().get(c).unwrap().position()),
        [131_072, 0, 655_360]
    );
}

#[test]
fn switch_to_turn_based_stops_the_encounter_b5() {
    let (mut w, a, b) = rt_walking();
    assert!(tick_with_movement(&mut w).is_empty());
    request_mode(&mut w, EncounterMode::TurnBased).unwrap();
    assert!(tick_with_movement(&mut w).is_empty());
    assert!(tick_with_movement(&mut w).is_empty());
    assert!(w.movers().get(a).unwrap().route().is_some());
    assert!(w.movers().get(b).unwrap().route().is_some());
    assert_eq!(
        raw3(w.movers().get(a).unwrap().position()),
        [117_964, 0, 157_286]
    );
    assert_eq!(
        raw3(w.movers().get(b).unwrap().position()),
        [655_360, 0, 98_304]
    );
    // Tick 4 is the RT boundary; stage 2 stops before stage 4 moves anybody.
    let facts = tick_with_movement(&mut w);
    assert!(facts.is_empty(), "a stop emits no movement fact");
    let combat = w.combat().expect("an encounter");
    assert_eq!(combat.mode, EncounterMode::TurnBased);
    assert!(combat.pending_mode.is_none());
    for e in [a, b] {
        assert!(w.movers().get(e).unwrap().route().is_none());
        assert_eq!(bits(&w, e).1, [0; 3]);
    }
    assert_eq!(
        raw3(w.movers().get(a).unwrap().position()),
        [117_964, 0, 157_286]
    );
    assert_eq!(
        raw3(w.movers().get(b).unwrap().position()),
        [655_360, 0, 98_304]
    );

    // Control: no request, so the movers keep walking through tick 4.
    let (mut w, a, b) = rt_walking();
    assert!(tick_with_movement(&mut w).is_empty());
    for _ in 0..3 {
        assert!(tick_with_movement(&mut w).is_empty());
    }
    assert_eq!(
        w.combat().expect("an encounter").mode,
        EncounterMode::RealTime
    );
    assert_eq!(
        w.movers().get(a).unwrap().route().unwrap().tag(),
        RouteTag(1)
    );
    assert_eq!(
        w.movers().get(b).unwrap().route().unwrap().tag(),
        RouteTag(2)
    );
    assert_eq!(
        raw3(w.movers().get(a).unwrap().position()),
        [157_286, 0, 209_715]
    );
    assert_eq!(
        raw3(w.movers().get(b).unwrap().position()),
        [655_360, 0, 131_072]
    );
}

#[test]
fn switch_stops_dead_combatants_b5() {
    let mut fx = move_rt();
    enlist(&mut fx, 113, 8);
    lethal(&mut fx);
    let mut w = World::new(7);
    w.bind_nav(rev());
    start_in(&mut w, &fx, EncounterMode::RealTime).unwrap();
    let (a, b) = (who(&w, 111), who(&w, 112));
    let order = perform_action(&mut w, &use_action(a, fx.ability, b)).unwrap();
    assert!(order.is_none());
    assert!(tick_with_movement(&mut w).is_empty());
    assert!(w.combatants().get(b).unwrap().dead());
    assert!(tick_with_movement(&mut w).is_empty());
    assert!(tick_with_movement(&mut w).is_empty());
    assert_eq!(elapsed(&w), 3);
    let mut w = with_json(&w, |save| {
        set_mover_route(save, b, 9, json!([[655_360, 0, 0], [655_360, 0, 131_072]]));
    });
    request_mode(&mut w, EncounterMode::TurnBased).unwrap();
    let facts = tick_with_movement(&mut w);
    assert!(
        facts.is_empty(),
        "the switch stops a dead combatant before stage 4"
    );
    assert!(w.movers().get(b).unwrap().route().is_none());
    assert_eq!(raw3(w.movers().get(b).unwrap().position()), [655_360, 0, 0]);
    assert_eq!(
        w.combat().expect("an encounter").mode,
        EncounterMode::TurnBased
    );
}

#[test]
fn tb_to_rt_leave_and_release_stop_nobody_b5() {
    // TB to RT: a route the loader let a TB combatant hold survives.
    let fx = move_rt();
    let mut w = World::new(7);
    w.bind_nav(rev());
    start_in(&mut w, &fx, EncounterMode::TurnBased).unwrap();
    let (a, b) = (who(&w, 111), who(&w, 112));
    let mut w = with_json(&w, |save| {
        set_mover_route(save, a, 9, json!([[0, 0, 0], [65_536, 0, 0]]));
    });
    request_mode(&mut w, EncounterMode::RealTime).unwrap();
    perform_action(&mut w, &CombatAction::EndTurn { actor: b }).unwrap();
    perform_action(&mut w, &CombatAction::EndTurn { actor: a }).unwrap();
    assert_eq!(
        w.combat().expect("an encounter").mode,
        EncounterMode::RealTime
    );
    assert_eq!(
        w.movers().get(a).unwrap().route().unwrap().tag(),
        RouteTag(9)
    );
    let facts = tick_with_movement(&mut w);
    assert_eq!(
        facts,
        vec![MovementFact::Arrived {
            entity: a,
            tag: RouteTag(9)
        }]
    );

    // Leave and release keep every route.
    let fx = move_rt();
    let mut w = World::new(7);
    w.bind_nav(rev());
    start_in(&mut w, &fx, EncounterMode::RealTime).unwrap();
    let (a, b) = (who(&w, 111), who(&w, 112));
    w.set_route(a, route(1, vec![p(0, 0), p(10, 0)])).unwrap();
    w.set_route(b, route(2, vec![p(10, 0), p(10, 10)])).unwrap();
    leave_encounter(&mut w, a).unwrap();
    assert!(w.creatures().get(a).is_some());
    assert_eq!(
        w.movers().get(a).unwrap().route().unwrap().tag(),
        RouteTag(1)
    );
    end_encounter(&mut w).unwrap();
    assert!(w.creatures().get(b).is_some());
    assert_eq!(
        w.movers().get(b).unwrap().route().unwrap().tag(),
        RouteTag(2)
    );
}

// -------------------------------------------------------- B-6 mirror rewrite

#[test]
fn mirror_rewrite_b6() {
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111]);
    let a = ids[0];
    w.transforms_mut().remove(a);
    assert!(tick_with_movement(&mut w).is_empty());
    assert_eq!(bits(&w, a), ([0; 3], [0; 3]));

    // A mover-free world gains no transform from a tick.
    let (mut plain, _) = Fixture::standard().start(7);
    assert!(plain.transforms().is_empty());
    tick_with_movement(&mut plain);
    assert!(plain.transforms().is_empty());

    // Every mover is mirrored each tick; an unrouted one gets zero velocity.
    let (mut w, ids) = registered(&fx, &[111, 112]);
    let (a, b) = (ids[0], ids[1]);
    w.set_route(a, route(1, vec![p(0, 0), p(1, 0)])).unwrap();
    tamper_velocity(&mut w, b);
    let facts = tick_with_movement(&mut w);
    assert_eq!(
        facts,
        vec![MovementFact::Arrived {
            entity: a,
            tag: RouteTag(1)
        }]
    );
    assert_eq!(bits(&w, a).1, [0x3f80_0000, 0, 0]);
    assert_eq!(bits(&w, b), ([0x4120_0000, 0, 0], [0; 3]));
}

// ---------------------------------------------------------- B-7 history tick

#[test]
fn history_tick_b7() {
    let fx = move_fixture();
    let (mut plain, plain_ids) = registered(&fx, &[111]);
    let pa = plain_ids[0];
    plain
        .set_route(pa, route(7, vec![p(0, 0), p(3, 4)]))
        .unwrap();
    let mut h = HistoryWorld::new(7);
    let a = fx.register_history(&mut h, pl(111)).unwrap();
    h.bind_nav(rev()).unwrap();
    h.set_route(a, route(7, vec![p(0, 0), p(3, 4)])).unwrap();
    let before = (h.pending_len(), h.last_sequence(), h.version());
    for k in 1..=5 {
        let facts = h.tick_with_movement().unwrap();
        let plain_facts = tick_with_movement(&mut plain);
        assert_eq!(facts, plain_facts, "tick {k}");
        assert_eq!(
            raw3(h.world().movers().get(a).unwrap().position()),
            raw3(plain.movers().get(pa).unwrap().position()),
            "tick {k}"
        );
    }
    assert_eq!(
        (h.pending_len(), h.last_sequence(), h.version()),
        before,
        "movement facts are not journal events"
    );

    // The wrapper journals exactly what `tick` journals.
    let build = || {
        let mut h = HistoryWorld::new(7);
        let a = fx.register_history(&mut h, pl(111)).unwrap();
        h.bind_nav(rev()).unwrap();
        h.set_route(a, route(7, vec![p(0, 0), p(3, 4)])).unwrap();
        h
    };
    let mut via_tick = build();
    let mut via_facts = build();
    via_tick.tick().unwrap();
    via_facts.tick_with_movement().unwrap();
    assert_eq!(
        serde_json::to_string(&via_tick).unwrap(),
        serde_json::to_string(&via_facts).unwrap()
    );
}

// ------------------------------------------------- B-9 zero velocity on ops

#[test]
fn operations_zero_velocity_b9() {
    let fx = move_fixture();
    let moving = || {
        let (mut w, ids) = registered(&fx, &[111]);
        let a = ids[0];
        w.set_route(a, route(7, vec![p(0, 0), p(3, 4)])).unwrap();
        tick_with_movement(&mut w);
        (w, a)
    };
    let (mut w, a) = moving();
    assert_eq!(
        bits(&w, a),
        ([0x3f19_9900, 0, 0x3f4c_cc00], [0x3f19_9900, 0, 0x3f4c_cc00])
    );
    w.place(a, p(2, 3)).unwrap();
    assert_eq!(bits(&w, a), ([0x4000_0000, 0, 0x4040_0000], [0; 3]));

    let (mut w, a) = moving();
    assert_eq!(w.clear_route(a), Ok(Some(RouteTag(7))));
    assert_eq!(bits(&w, a), ([0x3f19_9900, 0, 0x3f4c_cc00], [0; 3]));

    let (mut w, a) = moving();
    assert_eq!(
        w.set_route(a, route(8, vec![rp(39_321, 0, 52_428), p(3, 4)])),
        Ok(Some(RouteTag(7)))
    );
    assert_eq!(bits(&w, a).1, [0; 3]);

    let (mut w, a) = moving();
    assert_eq!(w.bind_nav(other()), vec![(a, RouteTag(7))]);
    assert_eq!(bits(&w, a).1, [0; 3]);

    // Control: the same revision binds nothing and leaves the mirror alone.
    let (mut w, a) = moving();
    assert_eq!(w.bind_nav(rev()), vec![]);
    assert_eq!(bits(&w, a).1, [0x3f19_9900, 0, 0x3f4c_cc00]);
}

// ------------------------------------------------------- W2 boundary handling

#[test]
fn boundary_vertex_and_neighbours() {
    let fx = move_fixture();
    // Exactly on a vertex: the next segment starts at along 0, no fact.
    let (mut w, ids) = registered(&fx, &[111]);
    let a = ids[0];
    w.set_route(a, route(4, vec![p(0, 0), p(1, 0), p(1, 1)]))
        .unwrap();
    assert!(tick_with_movement(&mut w).is_empty());
    assert_eq!(raw3(w.movers().get(a).unwrap().position()), [65_536, 0, 0]);
    let active = w.movers().get(a).unwrap().route().unwrap();
    assert_eq!((active.segment(), active.along()), (1, 0));
    let facts = tick_with_movement(&mut w);
    assert_eq!(
        facts,
        vec![MovementFact::Arrived {
            entity: a,
            tag: RouteTag(4)
        }]
    );
    assert_eq!(
        raw3(w.movers().get(a).unwrap().position()),
        [65_536, 0, 65_536]
    );

    // Exactly on the last vertex: no leftover, no second move.
    let (mut w, ids) = registered(&fx, &[111]);
    let a = ids[0];
    w.set_route(a, route(5, vec![p(0, 0), p(1, 0)])).unwrap();
    let facts = tick_with_movement(&mut w);
    assert_eq!(
        facts,
        vec![MovementFact::Arrived {
            entity: a,
            tag: RouteTag(5)
        }]
    );
    assert_eq!(raw3(w.movers().get(a).unwrap().position()), [65_536, 0, 0]);

    // One raw unit short of the vertex: no carry, then the next step arrives.
    let mut short = move_fixture();
    set_stat_raw(&mut short, pl(121), "pace", 65_535);
    let (mut w, ids) = registered(&short, &[111]);
    let a = ids[0];
    w.set_route(a, route(6, vec![p(0, 0), p(1, 0)])).unwrap();
    assert!(tick_with_movement(&mut w).is_empty());
    assert_eq!(raw3(w.movers().get(a).unwrap().position()), [65_535, 0, 0]);
    assert_eq!(w.movers().get(a).unwrap().route().unwrap().along(), 65_535);
    assert_eq!(bits(&w, a).1, mirror_bits([65_535, 0, 0]));
    let facts = tick_with_movement(&mut w);
    assert_eq!(
        facts,
        vec![MovementFact::Arrived {
            entity: a,
            tag: RouteTag(6)
        }]
    );
    assert_eq!(raw3(w.movers().get(a).unwrap().position()), [65_536, 0, 0]);

    // One raw unit over: the leftover is discarded at arrival.
    let mut over = move_fixture();
    set_stat_raw(&mut over, pl(121), "pace", 65_537);
    let (mut w, ids) = registered(&over, &[111]);
    let a = ids[0];
    w.set_route(a, route(7, vec![p(0, 0), p(1, 0)])).unwrap();
    let facts = tick_with_movement(&mut w);
    assert_eq!(
        facts,
        vec![MovementFact::Arrived {
            entity: a,
            tag: RouteTag(7)
        }]
    );
    assert_eq!(raw3(w.movers().get(a).unwrap().position()), [65_536, 0, 0]);
}

#[test]
fn max_route_points_walks_to_the_end_b1() {
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111]);
    let a = ids[0];
    assert_eq!(MAX_ROUTE_POINTS, 64);
    let points: Vec<_> = (0..64).map(|x| p(x, 0)).collect();
    w.set_route(a, route(6, points)).unwrap();
    for k in 1..=62_i32 {
        assert!(tick_with_movement(&mut w).is_empty(), "tick {k}");
        assert_eq!(
            raw3(w.movers().get(a).unwrap().position()),
            [k * 65_536, 0, 0]
        );
        let active = w.movers().get(a).unwrap().route().unwrap();
        assert_eq!(
            (usize::from(active.segment()), active.along()),
            (k as usize, 0)
        );
    }
    let facts = tick_with_movement(&mut w);
    assert_eq!(
        facts,
        vec![MovementFact::Arrived {
            entity: a,
            tag: RouteTag(6)
        }]
    );
    assert_eq!(
        raw3(w.movers().get(a).unwrap().position()),
        [63 * 65_536, 0, 0]
    );
    assert_eq!(bits(&w, a).0, [0x427c_0000, 0, 0]);
}

// ------------------------------------------------ stage 4 reads no transform

#[test]
fn movement_never_reads_the_mirror() {
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111]);
    let a = ids[0];
    w.set_route(a, route(7, vec![p(0, 0), p(3, 4)])).unwrap();
    w.transforms_mut().get_mut(a).unwrap().position = [123.0, 45.0, -7.0];
    assert!(tick_with_movement(&mut w).is_empty());
    assert_eq!(
        raw3(w.movers().get(a).unwrap().position()),
        [39_321, 0, 52_428]
    );
    assert_eq!(bits(&w, a).0, [0x3f19_9900, 0, 0x3f4c_cc00]);
    w.transforms_mut().get_mut(a).unwrap().position = [-999.0, 0.0, 0.0];
    assert!(tick_with_movement(&mut w).is_empty());
    assert_eq!(
        raw3(w.movers().get(a).unwrap().position()),
        [78_643, 0, 104_857]
    );
}

// -------------------------------------------------------------- W4 tick form

#[test]
fn tick_and_tick_with_movement_agree_b4() {
    let fx = move_rt();
    let build = || {
        let mut w = World::new(7);
        w.bind_nav(rev());
        start_in(&mut w, &fx, EncounterMode::RealTime).unwrap();
        let a = who(&w, 111);
        w.set_route(a, route(1, vec![p(0, 0), p(10, 0)])).unwrap();
        (w, a)
    };
    let (mut via_tick, _) = build();
    let (mut via_facts, a) = build();
    tick(&mut via_tick);
    let facts = tick_with_movement(&mut via_facts);
    assert!(facts.is_empty());
    assert_eq!(bytes_of(&via_tick), bytes_of(&via_facts));
    assert_eq!(state_hash(&via_tick), state_hash(&via_facts));
    assert_eq!(
        raw3(via_facts.movers().get(a).unwrap().position()),
        [65_536, 0, 0]
    );
    tick(&mut via_tick);
    tick_with_movement(&mut via_facts);
    assert_eq!(bytes_of(&via_tick), bytes_of(&via_facts));
    assert_eq!(state_hash(&via_tick), state_hash(&via_facts));
}

// --------------------------------------------------- W3 no side effects at all

#[test]
fn stage_four_touches_no_pool_order_timeline_rng_or_event() {
    // A plain routed world: every difference a tick makes is the counter, the
    // route progress and the mirror.
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111]);
    let a = ids[0];
    w.set_route(a, route(1, vec![p(0, 0), p(10, 0)])).unwrap();
    let before = value_of(&w);
    assert!(tick_with_movement(&mut w).is_empty());
    let mut after = value_of(&w);
    after["tick"] = before["tick"].clone();
    after["movers"] = before["movers"].clone();
    after["transforms"] = before["transforms"].clone();
    assert_eq!(after, before, "stage 4 touches nothing else");

    // In a real-time encounter, stage 4 leaves pools, RNG and events alone.
    let fx = move_rt();
    let mut w = World::new(7);
    w.bind_nav(rev());
    start_in(&mut w, &fx, EncounterMode::RealTime).unwrap();
    let (a, b) = (who(&w, 111), who(&w, 112));
    w.set_route(a, route(1, vec![p(0, 0), p(10, 0)])).unwrap();
    let pools = |w: &World| {
        let first = w.combatants().get(a).expect("A");
        let second = w.combatants().get(b).expect("B");
        (
            first.action_pool().current(),
            first.action_pool().max(),
            first
                .extra_pools()
                .iter()
                .map(|p| p.current())
                .collect::<Vec<_>>(),
            second.action_pool().current(),
            second.action_pool().max(),
            second
                .extra_pools()
                .iter()
                .map(|p| p.current())
                .collect::<Vec<_>>(),
        )
    };
    let before_pools = pools(&w);
    let before_value = value_of(&w);
    assert!(tick_with_movement(&mut w).is_empty());
    assert_eq!(pools(&w), before_pools);
    assert_eq!(value_of(&w)["rng"], before_value["rng"]);
    assert_eq!(value_of(&w)["events"], before_value["events"]);
    assert_eq!(
        w.movers().get(a).unwrap().route().unwrap().tag(),
        RouteTag(1)
    );

    // An RT order does not clear a route, and a route does not clear an order.
    let order = perform_action(&mut w, &use_action(a, fx.ability, b)).unwrap();
    assert!(order.is_none());
    assert!(w.combatants().get(a).unwrap().order().is_some());
    assert!(w.movers().get(a).unwrap().route().is_some());
    assert_eq!(
        w.set_route(a, route(2, vec![rp(65_536, 0, 0), p(10, 0)])),
        Ok(Some(RouteTag(1)))
    );
    assert!(
        w.combatants().get(a).unwrap().order().is_some(),
        "a route does not clear an order"
    );
    tick_with_movement(&mut w);
    assert_eq!(
        w.movers().get(a).unwrap().route().unwrap().tag(),
        RouteTag(2)
    );
}

// ------------------------------------------------------ W1 after clock freeze

#[test]
fn movement_runs_after_an_rt_clock_freeze() {
    let fx = move_rt();
    let mut w = World::new(7);
    w.bind_nav(rev());
    start_in(&mut w, &fx, EncounterMode::RealTime).unwrap();
    let a = who(&w, 111);
    let mut w = with_json(&w, |save| {
        save["combat"]["clock"]["elapsed"] = json!(RT_CLOCK_MAX);
    });
    assert_eq!(elapsed(&w), RT_CLOCK_MAX);
    w.set_route(a, route(1, vec![p(0, 0), p(1, 0)])).unwrap();
    let facts = tick_with_movement(&mut w);
    assert_eq!(
        facts,
        vec![MovementFact::Arrived {
            entity: a,
            tag: RouteTag(1)
        }]
    );
    assert_eq!(elapsed(&w), RT_CLOCK_MAX, "the clock stays frozen");
    assert_eq!(raw3(w.movers().get(a).unwrap().position()), [65_536, 0, 0]);
}
