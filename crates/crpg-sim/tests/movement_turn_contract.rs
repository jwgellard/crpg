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
