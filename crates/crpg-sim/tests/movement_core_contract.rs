//! T036a two-keys suite: written from `tasks/T036.md` alone (arithmetic, route
//! checks, creation, operations, lifecycle, load coherence, byte identity).
//! Expected values come from the contract text and hand arithmetic.

#[allow(dead_code)]
mod support;

use std::error::Error;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_rules::MAX_TAGS;
use crpg_sim::*;
use serde_json::{json, Value};
use support::creature::{add_outsider, enlist, top_level_keys, try_with_json, with_json};
use support::draws::{set_tags, start_into};
use support::rt::{bytes_of, json_keys, make_lethal, use_action, value_of};
use support::surface::{always_success, set_stat_raw};
use support::{uid, Fixture};

const LIMIT: i32 = 1 << 29;
/// `World::new(7)` as written before T036a (pinned by `creature_state.rs`).
const EMPTY_WORLD_SEED_7: &str = r#"{"entities":{"slots":[],"free":[]},"transforms":[],"timeline":[],"events":{"envelopes":[],"next_seq":0},"rng":{"seed":7,"streams":{}},"tick":0}"#;

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
fn add_fixed_stat(f: &mut Fixture, decl: Value) {
    edit_ruleset(f, |r| r["stats"].as_array_mut().unwrap().push(decl));
}
fn set_speed(f: &mut Fixture, wire: Value) {
    edit_ruleset(f, |r| r["movement"]["speed"] = wire);
}
fn set_step(f: &mut Fixture, wire: Value) {
    edit_ruleset(f, |r| r["movement"]["turn"]["step"] = wire);
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
    set_stat_raw(&mut fx, pl(121), "pace", 65536);
    set_stat_raw(&mut fx, pl(122), "pace", 32768);
    set_position(&mut fx, 112, rp(655360, 0, 0));
    add_outsider(&mut fx, 113, 123);
    set_position(&mut fx, 113, rp(0, 0, 655360));
    for name in ["alpha", "beta", "gamma"] {
        set_stat_raw(&mut fx, pl(123), name, 5 * 65536);
    }
    set_stat_raw(&mut fx, pl(123), "health", 4 * 65536);
    set_stat_raw(&mut fx, pl(123), "pace", 65536);
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
    let found = w
        .creatures()
        .iter()
        .find(|(_, r)| r.placement() == pl(placement))
        .map(|(id, _)| id)
        .or_else(|| {
            w.combatants()
                .iter()
                .find(|(_, c)| c.placement() == pl(placement))
                .map(|(id, _)| id)
        });
    found.expect("the placement is live")
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
const MOVING: [u32; 3] = [0x3f80_0000, 0, 0];
/// Runs `op`; on `Err` the world must serialize to the same bytes.
fn unchanged_on_err<T: std::fmt::Debug>(
    w: &mut World,
    op: impl FnOnce(&mut World) -> Result<T, MoveError>,
) -> MoveError {
    let before = bytes_of(w);
    let err = op(w).unwrap_err();
    assert_eq!(bytes_of(w), before, "a failing operation changes nothing");
    err
}
fn eid(index: u32, generation: u32) -> EntityId {
    serde_json::from_value(json!({"index": index, "generation": generation})).unwrap()
}

// ---------------------------------------------------------------- arithmetic

#[test]
fn lengths_a1() {
    assert_eq!(seg_len(p(0, 0), p(3, 4)), 327_680);
    assert_eq!(seg_len(rp(0, 0, 0), rp(1, 0, 1)), 2);
    assert_eq!(seg_len(rp(0, 0, 0), rp(3, 0, 0)), 3);
    assert_eq!(seg_len(rp(0, 0, 0), rp(65536, 0, 65536)), 92_682);
    assert_eq!(seg_len(p(0, 0), rp(0, 327_680, 0)), 0);
    assert_eq!(
        seg_len(rp(-LIMIT, 0, -LIMIT), rp(LIMIT, 0, LIMIT)),
        1_518_500_250
    );
    assert_eq!(
        seg_len(rp(i32::MIN, 0, i32::MIN), rp(i32::MAX, 0, i32::MAX)),
        6_074_000_999
    );
}

#[test]
fn lengths_extra_exact_and_rounded_up() {
    // 3-4-5 raw: an exact root is not rounded up.
    assert_eq!(seg_len(rp(0, 0, 0), rp(3, 0, 4)), 5);
    // sqrt(8) = 2.83 rounds up to 3; sqrt(10) to 4; sqrt(16) stays 4.
    assert_eq!(seg_len(rp(0, 0, 0), rp(2, 0, 2)), 3);
    assert_eq!(seg_len(rp(0, 0, 0), rp(1, 0, 3)), 4);
    assert_eq!(seg_len(rp(0, 0, 0), rp(0, 0, 4)), 4);
    // Symmetric, sign-blind, and each horizontal axis counts on its own.
    assert_eq!(seg_len(rp(5, 0, 5), rp(2, 0, 1)), 5);
    assert_eq!(seg_len(rp(2, 0, 1), rp(5, 0, 5)), 5);
    assert_eq!(seg_len(rp(0, 0, 0), rp(0, 0, 65536)), 65536);
    assert_eq!(seg_len(rp(0, 0, 0), rp(65536, 0, 0)), 65536);
    // Height never counts, whatever its sign.
    assert_eq!(seg_len(rp(0, -900_000, 0), rp(3, 7, 4)), 5);
}

#[test]
fn interpolation_a2() {
    assert_eq!(
        point_at(p(0, 0), p(3, 4), 327_680, 163_840),
        rp(98_304, 0, 131_072)
    );
    assert_eq!(point_at(rp(0, 0, 0), rp(-2, 0, -1), 3, 1), rp(0, 0, 0));
    assert_eq!(point_at(rp(0, 0, 0), rp(-2, 0, -1), 3, 2), rp(-1, 0, 0));
    assert_eq!(
        point_at(rp(0, 0, 0), rp(655_360, 131_072, 0), 655_360, 327_680),
        rp(327_680, 65_536, 0)
    );
    let (a, b) = (p(1, 2), p(5, 9));
    assert_eq!(point_at(a, b, 77, 77), b);
    assert_eq!(point_at(a, b, 77, 78), b);
    assert_eq!(point_at(a, b, 77, u64::MAX), b);
    assert_eq!(point_at(a, b, 0, 0), a);
    assert_eq!(point_at(a, b, 0, 5), a);
}

#[test]
fn interpolation_truncates_toward_zero_both_ways() {
    // Positive offsets: 2/3 -> 0 and 4/3 -> 1 (floor and trunc agree).
    assert_eq!(point_at(rp(0, 0, 0), rp(2, 0, 1), 3, 1), rp(0, 0, 0));
    assert_eq!(point_at(rp(0, 0, 0), rp(2, 0, 1), 3, 2), rp(1, 0, 0));
    // Negative height offset: -5/3 -> -1, -10/3 -> -3 (a flooring `/` gives -2, -4).
    assert_eq!(point_at(rp(0, 0, 0), rp(0, -5, 3), 3, 1), rp(0, -1, 1));
    assert_eq!(point_at(rp(0, 0, 0), rp(0, -5, 3), 3, 2), rp(0, -3, 2));
    // Offsets from a nonzero start.
    assert_eq!(point_at(rp(1, 0, 0), rp(10, 0, 0), 9, 4), rp(5, 0, 0));
}

#[test]
fn interpolation_is_total_at_the_extremes() {
    let a = rp(i32::MIN, i32::MIN, i32::MIN);
    let b = rp(i32::MAX, i32::MAX, i32::MAX);
    let len = seg_len(a, b);
    for d in [1, len / 2, len - 1] {
        let got = raw3(point_at(a, b, len, d));
        for (i, g) in got.iter().enumerate() {
            let exact = i128::from(a[i].to_raw())
                + (i128::from(b[i].to_raw()) - i128::from(a[i].to_raw())) * i128::from(d)
                    / i128::from(len);
            assert_eq!(i128::from(*g), exact);
        }
    }
}

#[test]
fn measures_and_projection_a3() {
    let pts = [p(0, 0), p(3, 4), p(3, 10)];
    assert_eq!(route_length(&pts), Some(720_896));
    assert_eq!(route_measure(Measure::Euclidean, &pts), Some(720_896));
    assert_eq!(route_length(&[]), None);
    assert_eq!(route_length(&[p(0, 0)]), None);
    assert_eq!(route_measure(Measure::Euclidean, &[p(0, 0)]), None);
    assert_eq!(route_length(&[p(0, 0), rp(LIMIT + 1, 0, 0)]), None);
    assert_eq!(route_length(&[rp(LIMIT + 1, 0, 0), p(0, 0)]), None);
    assert_eq!(route_length(&[p(0, 0), rp(0, 0, -LIMIT - 1)]), None);
    assert_eq!(route_length(&[p(0, 0), rp(0, LIMIT + 1, 0), p(1, 0)]), None);
    assert_eq!(route_length(&[p(0, 0), p(1, 0), rp(0, i32::MIN, 0)]), None);
    assert_eq!(
        route_length(&[rp(-LIMIT, 0, 0), rp(LIMIT, 0, 0)]),
        Some(1 << 30)
    );
    assert_eq!(
        route_length(&[rp(0, -LIMIT, 0), rp(0, LIMIT, LIMIT)]),
        Some(1 << 29)
    );
    assert_eq!(snap(Measure::Euclidean, rp(1, 2, 3)), rp(1, 2, 3));
    assert_eq!(
        snap(Measure::Euclidean, rp(i32::MIN, 0, 7)),
        rp(i32::MIN, 0, 7)
    );
    assert_eq!(Measure::default(), Measure::Euclidean);
}

#[test]
fn to_f32_bit_patterns() {
    for (raw, want) in [
        (65_536, 0x3f80_0000_u32),
        (-98_304, 0xbfc0_0000),
        (1, 0x3780_0000),
        (LIMIT, 0x4600_0000),
        (16_777_217, 0x4380_0000),
        (i32::MIN, 0xc700_0000),
        (0, 0),
        (196_608, 0x4040_0000),
        (-1, 0xb780_0000),
    ] {
        assert_eq!(to_f32(Fx16_16::from_raw(raw)).to_bits(), want, "raw {raw}");
    }
}

#[test]
fn nav_revision_is_the_blake3_of_the_bytes() {
    let empty = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    let hex: String = NavRevision::of_bytes(b"")
        .0
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(hex, empty);
    assert_eq!(
        NavRevision::of_bytes(b"mesh-a").0,
        *blake3::hash(b"mesh-a").as_bytes()
    );
    assert_ne!(rev(), other());
    assert_eq!(rev(), NavRevision::of_bytes(b"mesh-a"));
}

#[test]
fn serde_shapes_of_the_value_types() {
    let v = serde_json::to_value(rev()).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 32);
    assert_eq!(serde_json::from_value::<NavRevision>(v).unwrap(), rev());
    assert_eq!(serde_json::to_value(RouteTag(9)).unwrap(), json!(9));
    assert_eq!(
        serde_json::to_value(Measure::Euclidean).unwrap(),
        json!("euclidean")
    );
    let r = route(3, vec![p(0, 0), p(1, 0)]);
    let mut v = serde_json::to_value(&r).unwrap();
    assert_eq!(serde_json::from_value::<MoveRoute>(v.clone()).unwrap(), r);
    v["extra"] = json!(1);
    assert!(serde_json::from_value::<MoveRoute>(v).is_err());
}

// -------------------------------------------------------------------- routes

fn checked(
    points: Vec<[Fx16_16; 3]>,
    nav: Option<NavRevision>,
    start: [Fx16_16; 3],
) -> Result<u64, MoveError> {
    validate_route(&route(1, points), nav, start)
}
fn line(n: i32) -> Vec<[Fx16_16; 3]> {
    (0..n).map(|x| p(x, 0)).collect()
}

#[test]
fn route_validation_a4_each_error() {
    let ok = Some(rev());
    let start = p(0, 0);
    assert_eq!(checked(vec![p(0, 0), p(1, 0)], ok, start), Ok(65_536));
    assert_eq!(
        checked(vec![], ok, start),
        Err(MoveError::RouteSize { points: 0 })
    );
    assert_eq!(
        checked(vec![p(0, 0)], ok, start),
        Err(MoveError::RouteSize { points: 1 })
    );
    assert_eq!(checked(line(64), ok, start), Ok(4_128_768));
    assert_eq!(
        checked(line(65), ok, start),
        Err(MoveError::RouteSize { points: 65 })
    );
    for bad in [
        rp(LIMIT + 1, 0, 0),
        rp(0, -LIMIT - 1, 0),
        rp(0, 0, LIMIT + 1),
        rp(i32::MIN, 0, 0),
    ] {
        assert_eq!(
            checked(vec![p(0, 0), bad], ok, start),
            Err(MoveError::OutOfBounds { index: 1 }),
            "{bad:?}"
        );
    }
    assert_eq!(
        checked(vec![rp(LIMIT + 1, 0, 0), p(1, 0)], ok, start),
        Err(MoveError::OutOfBounds { index: 0 })
    );
    assert_eq!(
        checked(
            vec![p(0, 0), rp(LIMIT + 1, 0, 0), rp(LIMIT + 1, 0, 0)],
            ok,
            start
        ),
        Err(MoveError::OutOfBounds { index: 1 })
    );
    assert_eq!(
        checked(vec![p(0, 0), p(1, 0), rp(65_536, 327_680, 0)], ok, start),
        Err(MoveError::ZeroSegment { index: 1 })
    );
    assert_eq!(
        checked(vec![p(0, 0), p(0, 0), p(1, 0)], ok, start),
        Err(MoveError::ZeroSegment { index: 0 })
    );
    assert_eq!(
        checked(vec![p(0, 0), p(1, 0), p(2, 0), p(2, 0)], ok, start),
        Err(MoveError::ZeroSegment { index: 2 })
    );
    assert_eq!(
        checked(vec![p(0, 0), p(4096, 0)], ok, start),
        Ok(268_435_456)
    );
    assert_eq!(
        checked(vec![p(0, 0), rp(268_435_457, 0, 0)], ok, start),
        Err(MoveError::TooLong {
            length: 268_435_457
        })
    );
    assert_eq!(
        checked(vec![p(0, 0), p(2048, 0), p(0, 1)], ok, start),
        Err(MoveError::TooLong {
            length: 268_435_472
        })
    );
    assert_eq!(
        checked(vec![p(0, 0), p(2048, 0), p(4096, 0)], ok, start),
        Ok(268_435_456)
    );
    assert_eq!(
        checked(vec![p(0, 0), p(2048, 0), rp(268_435_457, 0, 0)], ok, start),
        Err(MoveError::TooLong {
            length: 268_435_457
        })
    );
    assert_eq!(
        checked(vec![p(0, 0), p(1, 0)], None, start),
        Err(MoveError::NavUnbound)
    );
    assert_eq!(
        checked(vec![p(0, 0), p(1, 0)], Some(other()), start),
        Err(MoveError::StaleRevision)
    );
    for off in [rp(0, 65_536, 0), rp(1, 0, 0), rp(0, 0, -1)] {
        assert_eq!(
            checked(vec![p(0, 0), p(1, 0)], ok, off),
            Err(MoveError::StartMismatch)
        );
    }
}

#[test]
fn route_bounds_are_inclusive_at_exactly_the_limit() {
    // 2^29 is inside (so the failure is the length), 2^29 + 1 is outside.
    assert_eq!(
        checked(vec![p(0, 0), rp(LIMIT, 0, 0)], Some(rev()), p(0, 0)),
        Err(MoveError::TooLong { length: 1 << 29 })
    );
    assert_eq!(
        checked(vec![p(0, 0), rp(-LIMIT, 0, 0)], Some(rev()), p(0, 0)),
        Err(MoveError::TooLong { length: 1 << 29 })
    );
    // Height is inside at +-2^29 and ignored by the length.
    assert_eq!(
        checked(vec![p(0, 0), rp(0, LIMIT, 0)], Some(rev()), p(0, 0)),
        Err(MoveError::ZeroSegment { index: 0 })
    );
    assert_eq!(
        checked(vec![p(0, 0), rp(0, -LIMIT, 0)], Some(rev()), p(0, 0)),
        Err(MoveError::ZeroSegment { index: 0 })
    );
    // A start away from zero uses the same horizontal length.
    let start = rp(0, 65_536, 0);
    assert_eq!(
        checked(vec![start, p(1, 0)], Some(rev()), start),
        Ok(65_536)
    );
}

#[test]
fn route_check_order_every_adjacent_pair() {
    let ok = Some(rev());
    let far = rp(LIMIT + 1, 0, 0);
    let off = rp(0, 65_536, 0);
    // size before bounds
    assert_eq!(
        checked(vec![far], ok, p(0, 0)),
        Err(MoveError::RouteSize { points: 1 })
    );
    // bounds before zero segments
    assert_eq!(
        checked(vec![p(0, 0), p(0, 0), far], ok, p(0, 0)),
        Err(MoveError::OutOfBounds { index: 2 })
    );
    // zero segment before length
    assert_eq!(
        checked(vec![p(0, 0), p(0, 0), p(4097, 0)], ok, p(0, 0)),
        Err(MoveError::ZeroSegment { index: 0 })
    );
    // length before binding
    assert_eq!(
        checked(vec![p(0, 0), rp(268_435_457, 0, 0)], None, p(0, 0)),
        Err(MoveError::TooLong {
            length: 268_435_457
        })
    );
    // binding before revision
    let stale = MoveRoute {
        revision: other(),
        tag: RouteTag(1),
        points: vec![p(0, 0), p(1, 0)],
    };
    assert_eq!(
        validate_route(&stale, None, p(0, 0)),
        Err(MoveError::NavUnbound)
    );
    // revision before start
    assert_eq!(
        checked(vec![p(0, 0), p(1, 0)], Some(other()), off),
        Err(MoveError::StaleRevision)
    );
    // binding before start
    assert_eq!(
        checked(vec![p(0, 0), p(1, 0)], None, off),
        Err(MoveError::NavUnbound)
    );
    // and the full chain on one input: everything wrong, size wins
    assert_eq!(
        checked(vec![], None, off),
        Err(MoveError::RouteSize { points: 0 })
    );
}

// ------------------------------------------------------------------ creation

#[test]
fn registration_builds_the_mover_and_mirror_a5() {
    let fx = move_fixture();
    let (w, ids) = registered(&fx, &[111, 112]);
    let (a, b) = (ids[0], ids[1]);
    let ma = w.movers().get(a).unwrap();
    assert_eq!(raw3(ma.position()), [0, 0, 0]);
    assert_eq!(ma.speed().to_raw(), 65_536);
    assert_eq!(ma.step().to_raw(), 98_304);
    assert!(ma.route().is_none());
    assert_eq!(bits(&w, a), ([0; 3], [0; 3]));
    let mb = w.movers().get(b).unwrap();
    assert_eq!(raw3(mb.position()), [655_360, 0, 0]);
    assert_eq!(mb.speed().to_raw(), 32_768);
    assert_eq!(mb.step().to_raw(), 98_304);
    assert_eq!(bits(&w, b), ([0x4120_0000, 0, 0], [0; 3]));
    assert_eq!(w.movers().len(), 2);
}

#[test]
fn registration_without_movement_builds_nothing_and_checks_nothing() {
    let mut fx = Fixture::standard();
    let mut w = World::new(7);
    let a = fx.register(&mut w, pl(111)).unwrap();
    assert!(w.movers().get(a).is_none());
    assert!(w.movers().is_empty());
    assert!(w.transforms().is_empty());
    assert!(!top_level_keys(&bytes_of(&w)).contains(&"movers".to_owned()));
    // Out-of-range position is not checked without `movement`.
    set_position(&mut fx, 112, rp(LIMIT + 1, 0, 0));
    assert!(fx.register(&mut w, pl(112)).is_ok());
    // Nor at start.
    let (w2, _) = fx.start(7);
    assert!(w2.movers().is_empty());
    assert!(w2.transforms().is_empty());
}

fn reg_err(fx: &Fixture) -> CombatError {
    let mut w = World::new(7);
    let err = fx.register(&mut w, pl(111)).unwrap_err();
    assert_eq!(
        serde_json::to_string(&w).unwrap(),
        EMPTY_WORLD_SEED_7,
        "no mutation"
    );
    err
}

#[test]
fn registration_errors_each_variant_with_a_control() {
    let bad_pos = |x: i32, y: i32, z: i32| {
        let mut fx = move_fixture();
        set_position(&mut fx, 111, rp(x, y, z));
        fx
    };
    let want = CombatError::InvalidPosition { placement: pl(111) };
    for (x, y, z) in [
        (LIMIT + 1, 0, 0),
        (0, LIMIT + 1, 0),
        (0, 0, -LIMIT - 1),
        (i32::MIN, 0, 0),
    ] {
        assert_eq!(reg_err(&bad_pos(x, y, z)), want, "{x} {y} {z}");
    }
    for (x, y, z) in [(LIMIT, LIMIT, LIMIT), (-LIMIT, -LIMIT, -LIMIT)] {
        let mut w = World::new(7);
        let a = bad_pos(x, y, z).register(&mut w, pl(111)).unwrap();
        assert_eq!(raw3(w.movers().get(a).unwrap().position()), [x, y, z]);
    }

    let speed_of = |raw: i32| {
        let mut fx = move_fixture();
        set_stat_raw(&mut fx, pl(121), "pace", raw);
        fx
    };
    let want = CombatError::InvalidSpeed { placement: pl(111) };
    for raw in [0, -1, 262_145, i32::MAX] {
        assert_eq!(reg_err(&speed_of(raw)), want, "pace {raw}");
    }
    for raw in [1, 262_144] {
        let mut w = World::new(7);
        let a = speed_of(raw).register(&mut w, pl(111)).unwrap();
        assert_eq!(w.movers().get(a).unwrap().speed().to_raw(), raw);
    }
    for wire in [
        json!({"type": "constant", "value": 0}),
        json!({"type": "constant", "value": 262_145}),
        json!({"type": "constant", "value": -5}),
    ] {
        let mut fx = move_fixture();
        set_speed(&mut fx, wire);
        assert_eq!(reg_err(&fx), want);
    }
    let mut fx = move_fixture();
    set_speed(&mut fx, json!({"type": "constant", "value": 262_144}));
    let mut w = World::new(7);
    let a = fx.register(&mut w, pl(111)).unwrap();
    assert_eq!(w.movers().get(a).unwrap().speed().to_raw(), 262_144);

    let want = CombatError::InvalidStep { placement: pl(111) };
    for raw in [0, -1] {
        let mut fx = move_fixture();
        set_step(&mut fx, json!({"type": "constant", "value": raw}));
        assert_eq!(reg_err(&fx), want, "step {raw}");
    }
    let mut fx = move_fixture();
    set_step(&mut fx, json!({"type": "constant", "value": 1}));
    let mut w = World::new(7);
    let a = fx.register(&mut w, pl(111)).unwrap();
    assert_eq!(w.movers().get(a).unwrap().step().to_raw(), 1);
}

#[test]
fn step_stat_is_read_per_creature() {
    let mut fx = move_fixture();
    add_fixed_stat(&mut fx, json!({"name": "stride", "kind": "fixed"}));
    set_step(&mut fx, json!({"type": "stat", "value": "stride"}));
    set_stat_raw(&mut fx, pl(121), "stride", 0);
    set_stat_raw(&mut fx, pl(122), "stride", 131_072);
    set_stat_raw(&mut fx, pl(123), "stride", 1);
    assert_eq!(
        reg_err(&fx),
        CombatError::InvalidStep { placement: pl(111) }
    );
    set_stat_raw(&mut fx, pl(121), "stride", 98_304);
    let (w, ids) = registered(&fx, &[111, 112]);
    assert_eq!(w.movers().get(ids[0]).unwrap().step().to_raw(), 98_304);
    assert_eq!(w.movers().get(ids[1]).unwrap().step().to_raw(), 131_072);
}

#[test]
fn registration_error_order() {
    // bad position and bad speed -> position
    let mut fx = move_fixture();
    set_position(&mut fx, 111, rp(LIMIT + 1, 0, 0));
    set_stat_raw(&mut fx, pl(121), "pace", 0);
    assert_eq!(
        reg_err(&fx),
        CombatError::InvalidPosition { placement: pl(111) }
    );
    // bad position and bad step -> position
    let mut fx = move_fixture();
    set_position(&mut fx, 111, rp(LIMIT + 1, 0, 0));
    set_step(&mut fx, json!({"type": "constant", "value": 0}));
    assert_eq!(
        reg_err(&fx),
        CombatError::InvalidPosition { placement: pl(111) }
    );
    // bad speed and bad step -> speed
    let mut fx = move_fixture();
    set_stat_raw(&mut fx, pl(121), "pace", 0);
    set_step(&mut fx, json!({"type": "constant", "value": 0}));
    assert_eq!(
        reg_err(&fx),
        CombatError::InvalidSpeed { placement: pl(111) }
    );
    // a creature lacking `pace` is an existing check and wins
    let mut fx = move_fixture();
    support::surface::remove_stat(&mut fx, pl(121), "pace");
    set_position(&mut fx, 111, rp(LIMIT + 1, 0, 0));
    assert!(matches!(reg_err(&fx), CombatError::MissingStat { stat } if stat == "pace"));
    // area mismatch (R2) and a second registration (R5) win over a bad position
    let mut fx = move_fixture();
    set_position(&mut fx, 111, rp(LIMIT + 1, 0, 0));
    let mut w = World::new_in_area(7, pl(999));
    assert!(matches!(
        fx.register(&mut w, pl(111)).unwrap_err(),
        CombatError::AreaMismatch { .. }
    ));
    let good = move_fixture();
    let mut w = World::new(7);
    good.register(&mut w, pl(111)).unwrap();
    let before = bytes_of(&w);
    assert_eq!(
        fx.register(&mut w, pl(111)).unwrap_err(),
        CombatError::PlacementInUse { placement: pl(111) }
    );
    assert_eq!(bytes_of(&w), before);
}

fn quick(formula: Value) -> Fixture {
    let mut fx = move_fixture();
    add_fixed_stat(
        &mut fx,
        json!({"name": "quick", "kind": "fixed", "derived": formula}),
    );
    set_speed(&mut fx, json!({"type": "stat", "value": "quick"}));
    fx
}
fn fixed_lit(raw: i32) -> Value {
    json!({"type": "literal", "value": {"type": "fixed", "value": raw}})
}
fn st(name: &str) -> Value {
    json!({"type": "stat", "value": name})
}
fn bin(kind: &str, l: Value, r: Value) -> Value {
    json!({"type": kind, "value": {"left": l, "right": r}})
}

#[test]
fn derived_fixed_lengths_are_evaluated() {
    for (formula, want) in [
        (bin("add", st("pace"), fixed_lit(16_384)), 81_920),
        (bin("subtract", st("pace"), fixed_lit(16_384)), 49_152),
        (bin("multiply", st("pace"), fixed_lit(98_304)), 98_304),
        (bin("min", st("pace"), fixed_lit(16_384)), 16_384),
        (bin("max", st("pace"), fixed_lit(200_000)), 200_000),
        (fixed_lit(40_000), 40_000),
    ] {
        let fx = quick(formula.clone());
        let mut w = World::new(7);
        let a = fx.register(&mut w, pl(111)).unwrap();
        assert_eq!(
            w.movers().get(a).unwrap().speed().to_raw(),
            want,
            "{formula}"
        );
    }
    // A derived stat may name another derived stat.
    let mut fx = quick(bin("add", st("pace"), fixed_lit(16_384)));
    add_fixed_stat(
        &mut fx,
        json!({"name": "quicker", "kind": "fixed", "derived": bin("max", st("quick"), fixed_lit(200_000))}),
    );
    set_speed(&mut fx, json!({"type": "stat", "value": "quicker"}));
    let mut w = World::new(7);
    let a = fx.register(&mut w, pl(111)).unwrap();
    assert_eq!(w.movers().get(a).unwrap().speed().to_raw(), 200_000);
    // Derived steps work the same way.
    let mut fx = quick(bin("add", st("pace"), fixed_lit(16_384)));
    set_step(&mut fx, json!({"type": "stat", "value": "quick"}));
    set_speed(&mut fx, json!({"type": "stat", "value": "pace"}));
    let mut w = World::new(7);
    let a = fx.register(&mut w, pl(111)).unwrap();
    assert_eq!(w.movers().get(a).unwrap().step().to_raw(), 81_920);
}

#[test]
fn derived_fixed_failures_are_creation_errors() {
    let speed = CombatError::InvalidSpeed { placement: pl(111) };
    for formula in [
        bin("divide", st("pace"), fixed_lit(0)),
        bin("subtract", st("pace"), fixed_lit(65_536)),
        bin(
            "add",
            st("pace"),
            json!({"type": "literal", "value": {"type": "int", "value": 1}}),
        ),
        bin("add", st("pace"), st("alpha")),
        bin("add", st("pace"), fixed_lit(200_000)),
        json!({"type": "literal", "value": {"type": "bool", "value": true}}),
    ] {
        assert_eq!(reg_err(&quick(formula.clone())), speed, "{formula}");
    }
    // A plain Stat naming an Int stat is not a Fixed length.
    let mut fx = move_fixture();
    set_speed(&mut fx, json!({"type": "stat", "value": "alpha"}));
    assert_eq!(reg_err(&fx), speed);
    // The same failure as a step is InvalidStep.
    let mut fx = quick(bin("divide", st("pace"), fixed_lit(0)));
    set_speed(&mut fx, json!({"type": "stat", "value": "pace"}));
    set_step(&mut fx, json!({"type": "stat", "value": "quick"}));
    assert_eq!(
        reg_err(&fx),
        CombatError::InvalidStep { placement: pl(111) }
    );
}

#[test]
fn creation_draws_no_rng_and_adds_no_event() {
    let mut plain = World::new(7);
    Fixture::standard().register(&mut plain, pl(111)).unwrap();
    let (moving, _) = registered(&move_fixture(), &[111]);
    let (a, b) = (value_of(&plain), value_of(&moving));
    for key in ["rng", "events", "tick", "interners", "entities"] {
        assert_eq!(a.get(key), b.get(key), "{key}");
    }
}

#[test]
fn start_builds_movers_for_spawned_participants_in_authored_order() {
    for mode in [EncounterMode::TurnBased, EncounterMode::RealTime] {
        let fx = move_rt();
        let mut w = World::new(7);
        start_in(&mut w, &fx, mode).unwrap();
        let ids: Vec<EntityId> = w.combatants().iter().map(|(id, _)| id).collect();
        let movers: Vec<EntityId> = w.movers().iter().map(|(id, _)| id).collect();
        assert_eq!(ids, movers);
        let (a, b) = (who(&w, 111), who(&w, 112));
        assert_eq!(ids, vec![a, b]);
        assert_eq!(raw3(w.movers().get(a).unwrap().position()), [0, 0, 0]);
        assert_eq!(w.movers().get(a).unwrap().speed().to_raw(), 65_536);
        assert_eq!(raw3(w.movers().get(b).unwrap().position()), [655_360, 0, 0]);
        assert_eq!(w.movers().get(b).unwrap().speed().to_raw(), 32_768);
        assert_eq!(w.movers().get(b).unwrap().step().to_raw(), 98_304);
        assert_eq!(bits(&w, b), ([0x4120_0000, 0, 0], [0; 3]));
        assert_eq!(bits(&w, a), ([0; 3], [0; 3]));
        // The non-participant outsider gets nothing at start.
        assert_eq!(w.movers().len(), 2);
    }
    // History wrapper start builds them too.
    let (h, ids) = move_rt().start_history(7, EncounterMode::RealTime).unwrap();
    assert_eq!(h.world().movers().len(), 2);
    assert_eq!(bits(h.world(), ids[1]).0, [0x4120_0000, 0, 0]);
}

#[test]
fn start_errors_keep_the_world_and_follow_authored_order() {
    let try_start = |fx: &Fixture| {
        let mut w = World::new(7);
        let err = start_into(fx, &mut w).unwrap_err();
        assert_eq!(serde_json::to_string(&w).unwrap(), EMPTY_WORLD_SEED_7);
        err
    };
    let mut fx = move_fixture();
    set_stat_raw(&mut fx, pl(121), "pace", 0);
    assert_eq!(
        try_start(&fx),
        CombatError::InvalidSpeed { placement: pl(111) }
    );
    // an existing check (dice) wins
    support::rt::edit_ability(&mut fx, |a| a.dice = "not dice".to_owned());
    assert_eq!(try_start(&fx), CombatError::InvalidDice);
    // first participant's error wins over the second's
    let mut fx = move_fixture();
    set_stat_raw(&mut fx, pl(121), "pace", 0);
    set_position(&mut fx, 112, rp(LIMIT + 1, 0, 0));
    assert_eq!(
        try_start(&fx),
        CombatError::InvalidSpeed { placement: pl(111) }
    );
    let mut fx = move_fixture();
    set_position(&mut fx, 112, rp(LIMIT + 1, 0, 0));
    assert_eq!(
        try_start(&fx),
        CombatError::InvalidPosition { placement: pl(112) }
    );
    // within a participant: position, speed, step
    set_stat_raw(&mut fx, pl(122), "pace", 0);
    assert_eq!(
        try_start(&fx),
        CombatError::InvalidPosition { placement: pl(112) }
    );
    let mut fx = move_fixture();
    set_step(&mut fx, json!({"type": "constant", "value": 0}));
    assert_eq!(
        try_start(&fx),
        CombatError::InvalidStep { placement: pl(111) }
    );
    set_stat_raw(&mut fx, pl(121), "pace", 0);
    assert_eq!(
        try_start(&fx),
        CombatError::InvalidSpeed { placement: pl(111) }
    );
    // a non-participant is not checked at start, but is at registration
    let mut fx = move_fixture();
    set_stat_raw(&mut fx, pl(123), "pace", 0);
    let mut w = World::new(7);
    start_into(&fx, &mut w).unwrap();
    assert_eq!(
        fx.register(&mut World::new(7), pl(113)).unwrap_err(),
        CombatError::InvalidSpeed { placement: pl(113) }
    );
}

#[test]
fn entering_participants_keep_their_registered_mover() {
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111]);
    let a = ids[0];
    w.place(a, p(5, 5)).unwrap();
    start_into(&fx, &mut w).unwrap();
    assert!(w.combatants().get(a).is_some(), "A entered from its record");
    assert_eq!(raw3(w.movers().get(a).unwrap().position()), raw3(p(5, 5)));
    assert_eq!(bits(&w, a).0, mirror_bits(raw3(p(5, 5))));
    let b = who(&w, 112);
    assert_eq!(raw3(w.movers().get(b).unwrap().position()), [655_360, 0, 0]);
    assert_eq!(w.movers().len(), 2);
}

#[test]
fn members_and_joiners_keep_the_mover_their_registration_made() {
    let fx = move_fixture();
    // a start member
    let mut w = World::new(7);
    let c = fx.register(&mut w, pl(113)).unwrap();
    w.place(c, p(3, 3)).unwrap();
    let member = Entrant {
        entity: c,
        initiative: 5,
    };
    fx.with_spec(|spec| {
        start_encounter_with_members(&mut w, spec, EncounterMode::TurnBased, &[member])
    })
    .unwrap();
    assert!(w.combatants().get(c).is_some());
    assert_eq!(w.movers().len(), 3);
    assert_eq!(raw3(w.movers().get(c).unwrap().position()), raw3(p(3, 3)));
    assert_eq!(bits(&w, c).0, mirror_bits(raw3(p(3, 3))));
    // a joiner
    let (mut w, _) = fx.start(7);
    let c = fx.register(&mut w, pl(113)).unwrap();
    w.place(c, p(4, 4)).unwrap();
    assert_eq!(w.movers().len(), 3);
    join_encounter(
        &mut w,
        &Entrant {
            entity: c,
            initiative: 5,
        },
    )
    .unwrap();
    assert!(w.combatants().get(c).is_some());
    assert_eq!(w.movers().len(), 3);
    assert_eq!(raw3(w.movers().get(c).unwrap().position()), raw3(p(4, 4)));
    assert_eq!(bits(&w, c).0, mirror_bits(raw3(p(4, 4))));
}

// ---------------------------------------------------------------- operations

#[test]
fn bind_nav_a6() {
    let mut w = World::new(7);
    assert_eq!(w.nav(), None);
    assert_eq!(w.bind_nav(rev()), vec![]);
    assert_eq!(w.nav(), Some(rev()));
    let before = bytes_of(&w);
    assert_eq!(w.bind_nav(rev()), vec![]);
    assert_eq!(bytes_of(&w), before);

    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111, 112, 113]);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    w.set_route(b, route(2, vec![p(10, 0), p(11, 0)])).unwrap();
    w.set_route(a, route(1, vec![p(0, 0), p(1, 0)])).unwrap();
    for e in [a, b, c] {
        tamper_velocity(&mut w, e);
    }
    // same revision: nothing changes, not even the tampered mirror
    let same = bytes_of(&w);
    assert_eq!(w.bind_nav(rev()), vec![]);
    assert_eq!(bytes_of(&w), same);
    assert_eq!(bits(&w, a).1, MOVING);
    let positions: Vec<_> = ids
        .iter()
        .map(|e| w.movers().get(*e).unwrap().position())
        .collect();
    assert_eq!(
        w.bind_nav(other()),
        vec![(a, RouteTag(1)), (b, RouteTag(2))]
    );
    assert_eq!(w.nav(), Some(other()));
    for (i, e) in ids.iter().enumerate() {
        assert!(w.movers().get(*e).unwrap().route().is_none());
        assert_eq!(w.movers().get(*e).unwrap().position(), positions[i]);
    }
    assert_eq!(bits(&w, a).1, [0; 3]);
    assert_eq!(bits(&w, b).1, [0; 3]);
    assert_eq!(
        bits(&w, c).1,
        MOVING,
        "a mover with no route keeps its mirror"
    );
    // a mover world with nothing routed returns [] on a real change
    assert_eq!(w.bind_nav(rev()), vec![]);
    assert_eq!(w.nav(), Some(rev()));
}

#[test]
fn bind_nav_orders_by_entity_id_not_store_order() {
    let fx = move_fixture();
    let mut w = World::new(7);
    let a = fx.register(&mut w, pl(111)).unwrap();
    let b = fx.register(&mut w, pl(112)).unwrap();
    assert_eq!((a.index(), a.generation()), (0, 1));
    assert_eq!((b.index(), b.generation()), (1, 1));
    assert!(w.despawn(a));
    let c = fx.register(&mut w, pl(113)).unwrap();
    assert_eq!((c.index(), c.generation()), (0, 2));
    let store: Vec<EntityId> = w.movers().iter().map(|(id, _)| id).collect();
    assert_eq!(store, vec![b, c], "the store is in insertion order");
    w.bind_nav(rev());
    w.set_route(c, route(3, vec![p(0, 10), p(1, 10)])).unwrap();
    w.set_route(b, route(2, vec![p(10, 0), p(11, 0)])).unwrap();
    assert_eq!(
        w.bind_nav(other()),
        vec![(c, RouteTag(3)), (b, RouteTag(2))]
    );
}

#[test]
fn place_a6() {
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111]);
    let a = ids[0];
    let plain = w.spawn(EntityMeta {});
    let gone = w.spawn(EntityMeta {});
    assert!(w.despawn(gone));
    let far = rp(LIMIT + 1, 0, 0);
    assert_eq!(
        unchanged_on_err(&mut w, |w| w.place(gone, p(1, 1))),
        MoveError::AbsentEntity { entity: gone }
    );
    assert_eq!(
        unchanged_on_err(&mut w, |w| w.place(plain, p(1, 1))),
        MoveError::NotMover { entity: plain }
    );
    assert_eq!(
        unchanged_on_err(&mut w, |w| w.place(a, far)),
        MoveError::OutOfBounds { index: 0 }
    );
    // absent before bounds, mover check before bounds
    assert_eq!(
        unchanged_on_err(&mut w, |w| w.place(gone, far)),
        MoveError::AbsentEntity { entity: gone }
    );
    assert_eq!(
        unchanged_on_err(&mut w, |w| w.place(plain, far)),
        MoveError::NotMover { entity: plain }
    );
    for bad in [
        rp(0, LIMIT + 1, 0),
        rp(0, 0, -LIMIT - 1),
        rp(i32::MIN, 0, 0),
    ] {
        assert_eq!(
            unchanged_on_err(&mut w, |w| w.place(a, bad)),
            MoveError::OutOfBounds { index: 0 }
        );
    }
    // exactly at the limit is accepted, in every component
    w.place(a, rp(LIMIT, -LIMIT, LIMIT)).unwrap();
    assert_eq!(
        raw3(w.movers().get(a).unwrap().position()),
        [LIMIT, -LIMIT, LIMIT]
    );

    // success: position set, route cleared, mirror written with zero velocity
    w.set_route(
        a,
        route(
            1,
            vec![rp(LIMIT, -LIMIT, LIMIT), rp(LIMIT, -LIMIT, LIMIT - 65_536)],
        ),
    )
    .unwrap();
    tamper_velocity(&mut w, a);
    assert_eq!(w.place(a, p(2, 3)), Ok(()));
    let m = w.movers().get(a).unwrap();
    assert!(m.route().is_none());
    assert_eq!(raw3(m.position()), raw3(p(2, 3)));
    assert_eq!(bits(&w, a), ([0x4000_0000, 0, 0x4040_0000], [0; 3]));
    assert_eq!(w.nav(), Some(rev()), "place leaves the binding alone");
    // The save with a placed mover loads (mirror and position agree).
    assert!(try_with_json(&w, |_| {}).is_ok());
}

#[test]
fn place_needs_no_binding_and_no_alive_check() {
    let fx = move_fixture();
    let mut w = World::new(7);
    let a = fx.register(&mut w, pl(111)).unwrap();
    assert_eq!(w.nav(), None);
    assert_eq!(w.place(a, p(1, 1)), Ok(()));
}

#[test]
fn set_route_precedence_and_results_a6() {
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111]);
    let a = ids[0];
    let plain = w.spawn(EntityMeta {});
    let gone = w.spawn(EntityMeta {});
    assert!(w.despawn(gone));
    let bad = route(1, vec![p(9, 9)]);
    assert_eq!(
        unchanged_on_err(&mut w, |w| w.set_route(gone, bad.clone())),
        MoveError::AbsentEntity { entity: gone }
    );
    assert_eq!(
        unchanged_on_err(&mut w, |w| w.set_route(plain, bad.clone())),
        MoveError::NotMover { entity: plain }
    );
    assert_eq!(
        unchanged_on_err(&mut w, |w| w.set_route(a, bad.clone())),
        MoveError::RouteSize { points: 1 }
    );
    assert_eq!(
        unchanged_on_err(&mut w, |w| w.set_route(a, route(1, vec![p(1, 1), p(2, 1)]))),
        MoveError::StartMismatch
    );
    // success, first then replacement; the mirror is rewritten with zero velocity
    tamper_velocity(&mut w, a);
    let before_rng = value_of(&w)["rng"].clone();
    let before_events = value_of(&w)["events"].clone();
    let tick_before = w.tick();
    assert_eq!(w.set_route(a, route(1, vec![p(0, 0), p(3, 4)])), Ok(None));
    let active = w.movers().get(a).unwrap().route().unwrap();
    assert_eq!(
        (active.segment(), active.along(), active.tag()),
        (0, 0, RouteTag(1))
    );
    assert_eq!(active.route(), &route(1, vec![p(0, 0), p(3, 4)]));
    assert_eq!(bits(&w, a), ([0; 3], [0; 3]));
    assert_eq!(
        w.set_route(a, route(2, vec![p(0, 0), p(0, 5)])),
        Ok(Some(RouteTag(1)))
    );
    let active = w.movers().get(a).unwrap().route().unwrap();
    assert_eq!(
        (active.segment(), active.along(), active.tag()),
        (0, 0, RouteTag(2))
    );
    assert_eq!(active.route().points, vec![p(0, 0), p(0, 5)]);
    assert_eq!(value_of(&w)["rng"], before_rng);
    assert_eq!(value_of(&w)["events"], before_events);
    assert_eq!(w.tick(), tick_before);
    // binding checks: unbound, then stale
    let mut unbound = World::new(7);
    let a2 = fx.register(&mut unbound, pl(111)).unwrap();
    assert_eq!(
        unchanged_on_err(&mut unbound, |w| w
            .set_route(a2, route(1, vec![p(0, 0), p(1, 0)]))),
        MoveError::NavUnbound
    );
    let stale = MoveRoute {
        revision: other(),
        tag: RouteTag(1),
        points: vec![p(0, 0), p(1, 0)],
    };
    assert_eq!(
        unchanged_on_err(&mut w, |w| w.set_route(a, stale)),
        MoveError::StaleRevision
    );
    // the route is validated against the mover's position, not its mirror
    w.place(a, p(4, 4)).unwrap();
    w.transforms_mut().get_mut(a).unwrap().position = [9.0, 9.0, 9.0];
    assert_eq!(w.set_route(a, route(5, vec![p(4, 4), p(5, 4)])), Ok(None));
    assert_eq!(bits(&w, a).0, mirror_bits(raw3(p(4, 4))));
}

#[test]
fn set_route_mode_and_death_rules() {
    // A live turn-based fight: every participant is TurnBased, the outsider is not.
    let mut fx = move_fixture();
    lethal(&mut fx);
    let mut w = World::new(7);
    let c = fx.register(&mut w, pl(113)).unwrap();
    w.bind_nav(rev());
    start_in(&mut w, &fx, EncounterMode::TurnBased).unwrap();
    let (a, b) = (who(&w, 111), who(&w, 112));
    let good_a = route(1, vec![p(0, 0), p(1, 0)]);
    for (e, r) in [(a, good_a.clone()), (b, route(2, vec![p(1, 1)]))] {
        assert_eq!(
            unchanged_on_err(&mut w, |w| w.set_route(e, r)),
            MoveError::TurnBased { entity: e }
        );
    }
    assert_eq!(
        unchanged_on_err(&mut w, |w| w.set_route(c, route(3, vec![p(0, 0), p(1, 0)]))),
        MoveError::StartMismatch
    );
    assert_eq!(w.set_route(c, route(3, vec![p(0, 10), p(1, 10)])), Ok(None));

    // B acts first and kills A: A is dead, B is in a terminal turn-based encounter.
    perform_action(&mut w, &use_action(b, fx.ability, a)).unwrap();
    assert!(w.combatants().get(a).unwrap().dead());
    assert_eq!(
        unchanged_on_err(&mut w, |w| w.set_route(a, good_a.clone())),
        MoveError::DeadCombatant { entity: a }
    );
    assert_eq!(
        unchanged_on_err(&mut w, |w| w.set_route(a, route(1, vec![p(7, 7)]))),
        MoveError::DeadCombatant { entity: a }
    );
    assert_eq!(
        unchanged_on_err(&mut w, |w| w
            .set_route(b, route(2, vec![p(10, 0), p(10, 1)]))),
        MoveError::TurnBased { entity: b }
    );
    // The outsider may still set (replace) a route.
    assert_eq!(
        w.set_route(c, route(4, vec![p(0, 10), p(2, 10)])),
        Ok(Some(RouteTag(3)))
    );
}

#[test]
fn set_route_in_real_time_fights() {
    let mut fx = move_rt();
    lethal(&mut fx);
    let mut w = World::new(7);
    w.bind_nav(rev());
    start_in(&mut w, &fx, EncounterMode::RealTime).unwrap();
    let (a, b) = (who(&w, 111), who(&w, 112));
    assert_eq!(w.set_route(a, route(1, vec![p(0, 0), p(1, 0)])), Ok(None));
    assert_eq!(w.set_route(b, route(2, vec![p(10, 0), p(11, 0)])), Ok(None));
    // a pending switch to turn-based still allows a route
    request_mode(&mut w, EncounterMode::TurnBased).unwrap();
    assert_eq!(
        w.set_route(a, route(3, vec![p(0, 0), p(2, 0)])),
        Ok(Some(RouteTag(1)))
    );

    // a combatant killed in real time is dead
    let mut w = World::new(7);
    w.bind_nav(rev());
    start_in(&mut w, &fx, EncounterMode::RealTime).unwrap();
    let (a, b) = (who(&w, 111), who(&w, 112));
    perform_action(&mut w, &use_action(a, fx.ability, b)).unwrap();
    tick(&mut w);
    assert!(w.combatants().get(b).unwrap().dead());
    assert_eq!(
        unchanged_on_err(&mut w, |w| w
            .set_route(b, route(2, vec![p(10, 0), p(11, 0)]))),
        MoveError::DeadCombatant { entity: b }
    );
    assert_eq!(w.set_route(a, route(1, vec![p(0, 0), p(1, 0)])), Ok(None));
}

#[test]
fn a_dead_record_cannot_get_a_route_but_an_alive_one_can() {
    let mut fx = move_fixture();
    lethal(&mut fx);
    let (mut w, ids) = fx.start(7);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut w, &use_action(b, fx.ability, a)).unwrap();
    end_encounter(&mut w).unwrap();
    assert!(w.combat().is_none());
    assert!(w.combatants().get(a).is_none());
    assert!(w.creatures().get(a).unwrap().dead());
    assert!(w.movers().get(a).is_some(), "the corpse keeps its mover");
    w.bind_nav(rev());
    let r = route(1, vec![p(0, 0), p(3, 4)]);
    assert_eq!(
        unchanged_on_err(&mut w, |w| w.set_route(a, r)),
        MoveError::DeadCombatant { entity: a }
    );
    assert!(!w.creatures().get(b).unwrap().dead());
    assert_eq!(w.set_route(b, route(2, vec![p(10, 0), p(10, 1)])), Ok(None));
    assert_eq!(
        w.movers().get(b).unwrap().route().unwrap().tag(),
        RouteTag(2)
    );
}

#[test]
fn clear_route_a6() {
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111]);
    let a = ids[0];
    let plain = w.spawn(EntityMeta {});
    let gone = w.spawn(EntityMeta {});
    assert!(w.despawn(gone));
    assert_eq!(
        unchanged_on_err(&mut w, |w| w.clear_route(gone)),
        MoveError::AbsentEntity { entity: gone }
    );
    assert_eq!(
        unchanged_on_err(&mut w, |w| w.clear_route(plain)),
        MoveError::NotMover { entity: plain }
    );
    assert_eq!(w.clear_route(a), Ok(None));
    w.set_route(a, route(8, vec![p(0, 0), p(1, 0)])).unwrap();
    tamper_velocity(&mut w, a);
    assert_eq!(w.clear_route(a), Ok(Some(RouteTag(8))));
    assert!(w.movers().get(a).unwrap().route().is_none());
    assert_eq!(bits(&w, a), ([0; 3], [0; 3]));
    assert_eq!(w.clear_route(a), Ok(None));
    assert_eq!(raw3(w.movers().get(a).unwrap().position()), [0, 0, 0]);
}

#[test]
fn movement_never_reads_the_f32_mirror() {
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111]);
    let a = ids[0];
    // A host write to the mirror makes the save fail L6 ...
    w.transforms_mut().get_mut(a).unwrap().position = [123.0, 0.0, 0.0];
    let err = try_with_json(&w, |_| {}).expect_err("the tampered mirror fails to load");
    assert!(
        err.contains("mover transform disagrees with its position"),
        "{err}"
    );
    // ... but no operation reads it, and each rewrites it from the position.
    assert_eq!(w.set_route(a, route(1, vec![p(0, 0), p(1, 0)])), Ok(None));
    assert_eq!(bits(&w, a).0, mirror_bits([0, 0, 0]));
    w.transforms_mut().get_mut(a).unwrap().position = [123.0, 0.0, 0.0];
    assert_eq!(w.clear_route(a), Ok(Some(RouteTag(1))));
    assert_eq!(bits(&w, a).0, mirror_bits([0, 0, 0]));
    w.transforms_mut().get_mut(a).unwrap().position = [123.0, 0.0, 0.0];
    w.place(a, p(2, 2)).unwrap();
    assert_eq!(bits(&w, a).0, mirror_bits(raw3(p(2, 2))));
    // Velocity is not part of L6.
    tamper_velocity(&mut w, a);
    assert!(try_with_json(&w, |_| {}).is_ok());
    // A removed mirror is re-created by an operation.
    w.transforms_mut().remove(a);
    w.clear_route(a).unwrap();
    assert_eq!(bits(&w, a), (mirror_bits(raw3(p(2, 2))), [0; 3]));
}

#[test]
fn history_wrappers_match_the_world_operations_and_commit_nothing() {
    let fx = move_fixture();
    let mut h = HistoryWorld::new(7);
    let a = fx.register_history(&mut h, pl(111)).unwrap();
    let b = fx.register_history(&mut h, pl(112)).unwrap();
    assert_eq!(bits(h.world(), b).0, [0x4120_0000, 0, 0]);
    let snapshot = |h: &HistoryWorld| (h.pending_len(), h.last_sequence(), h.version());
    let (len, seq, _) = snapshot(&h);
    assert_eq!(h.bind_nav(rev()), Ok(vec![]));
    assert_eq!(snapshot(&h), (len, seq, 1));
    let before = serde_json::to_string(&h).unwrap();
    let hash = history_hash(&h);
    // failures: HistoryError::Move, nothing changes
    let r = route(1, vec![p(0, 0), p(1, 0)]);
    let plain = h.spawn(EntityMeta {}).unwrap();
    let (len, seq, _) = snapshot(&h);
    let before_plain = serde_json::to_string(&h).unwrap();
    assert_eq!(
        h.place(plain, p(1, 1)),
        Err(HistoryError::Move(MoveError::NotMover { entity: plain }))
    );
    assert_eq!(
        h.set_route(plain, r.clone()),
        Err(HistoryError::Move(MoveError::NotMover { entity: plain }))
    );
    assert_eq!(
        h.clear_route(plain),
        Err(HistoryError::Move(MoveError::NotMover { entity: plain }))
    );
    assert_eq!(
        h.place(a, rp(LIMIT + 1, 0, 0)),
        Err(HistoryError::Move(MoveError::OutOfBounds { index: 0 }))
    );
    assert_eq!(
        h.set_route(a, route(1, vec![p(1, 1), p(2, 2)])),
        Err(HistoryError::Move(MoveError::StartMismatch))
    );
    assert_eq!(serde_json::to_string(&h).unwrap(), before_plain);
    assert_ne!(before, before_plain);
    assert_ne!(history_hash(&h), hash);
    // successes: the same results, no event, no sequence, no version change
    assert_eq!(h.set_route(a, r.clone()), Ok(None));
    assert_eq!(h.set_route(b, route(2, vec![p(10, 0), p(11, 0)])), Ok(None));
    assert_eq!(
        h.set_route(a, route(3, vec![p(0, 0), p(2, 0)])),
        Ok(Some(RouteTag(1)))
    );
    assert_eq!(h.clear_route(a), Ok(Some(RouteTag(3))));
    assert_eq!(h.clear_route(a), Ok(None));
    assert_eq!(h.place(a, p(2, 3)), Ok(()));
    assert_eq!(bits(h.world(), a).0, [0x4000_0000, 0, 0x4040_0000]);
    assert_eq!(h.bind_nav(other()), Ok(vec![(b, RouteTag(2))]));
    assert_eq!(h.world().nav(), Some(other()));
    assert_eq!(snapshot(&h), (len, seq, 1));
    // the wrapper still round-trips
    let text = serde_json::to_string(&h).unwrap();
    let back: HistoryWorld = serde_json::from_str(&text).unwrap();
    assert_eq!(serde_json::to_string(&back).unwrap(), text);
}

#[test]
fn history_wrappers_need_no_free_journal_slot() {
    let fx = move_fixture();
    let mut h = HistoryWorld::new(7);
    let a = fx.register_history(&mut h, pl(111)).unwrap();
    h.bind_nav(rev()).unwrap();
    while h.pending_len() < MAX_HISTORY_EVENTS {
        h.spawn(EntityMeta {}).unwrap();
    }
    assert!(h.spawn(EntityMeta {}).is_err(), "the journal is full");
    let seq = h.last_sequence();
    assert_eq!(h.set_route(a, route(1, vec![p(0, 0), p(1, 0)])), Ok(None));
    assert_eq!(h.place(a, p(1, 1)), Ok(()));
    assert_eq!(h.clear_route(a), Ok(None));
    assert_eq!(h.bind_nav(other()), Ok(vec![]));
    assert_eq!(
        (h.pending_len(), h.last_sequence(), h.version()),
        (MAX_HISTORY_EVENTS, seq, 1)
    );
}

// ------------------------------------------------------------------ lifecycle

#[test]
fn despawn_removes_the_mover() {
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111, 112]);
    let (a, b) = (ids[0], ids[1]);
    w.set_route(a, route(1, vec![p(0, 0), p(1, 0)])).unwrap();
    assert!(w.despawn(a));
    assert!(w.movers().get(a).is_none());
    assert!(w.movers().get(b).is_some());
    assert!(w.transforms().get(a).is_none());
    assert!(try_with_json(&w, |_| {}).is_ok());
    assert!(w.despawn(b));
    assert!(w.movers().is_empty());
    assert!(!top_level_keys(&bytes_of(&w)).contains(&"movers".to_owned()));
    // the despawned id is dead for every operation
    assert_eq!(w.clear_route(a), Err(MoveError::AbsentEntity { entity: a }));
}

fn transfer_pair() -> (Fixture, World, World, EntityId) {
    let fx = move_fixture();
    let mut src = World::new_in_area(1, pl(131));
    let dst = World::new_in_area(2, pl(132));
    let a = fx.register(&mut src, pl(111)).unwrap();
    src.bind_nav(rev());
    src.place(a, p(2, 3)).unwrap();
    src.set_route(a, route(1, vec![p(2, 3), p(5, 3)])).unwrap();
    tamper_velocity(&mut src, a);
    (fx, src, dst, a)
}

#[test]
fn transfer_copies_the_mover_without_its_route() {
    let (_fx, mut src, mut dst, a) = transfer_pair();
    dst.bind_nav(other());
    let new = transfer_entity(&mut src, &mut dst, a).unwrap();
    assert!(src.movers().get(a).is_none());
    assert!(src.movers().is_empty());
    let m = dst.movers().get(new).unwrap();
    assert_eq!(raw3(m.position()), raw3(p(2, 3)));
    assert_eq!(m.speed().to_raw(), 65_536);
    assert_eq!(m.step().to_raw(), 98_304);
    assert!(m.route().is_none());
    assert_eq!(bits(&dst, new), ([0x4000_0000, 0, 0x4040_0000], [0; 3]));
    assert_eq!(
        dst.nav(),
        Some(other()),
        "the destination binding is not read or changed"
    );
    assert!(try_with_json(&dst, |_| {}).is_ok());
    assert!(try_with_json(&src, |_| {}).is_ok());
    // the host then places the creature
    assert_eq!(dst.place(new, p(7, 7)), Ok(()));
}

#[test]
fn history_transfer_copies_the_mover_too() {
    let fx = move_fixture();
    let mut src = HistoryWorld::new_in_area(1, pl(131));
    let mut dst = HistoryWorld::new_in_area(2, pl(132));
    let a = fx.register_history(&mut src, pl(111)).unwrap();
    src.bind_nav(rev()).unwrap();
    src.place(a, p(2, 3)).unwrap();
    src.set_route(a, route(1, vec![p(2, 3), p(5, 3)])).unwrap();
    let new = transfer_history_entity(&mut src, &mut dst, a).unwrap();
    assert!(src.world().movers().is_empty());
    let m = dst.world().movers().get(new).unwrap();
    assert_eq!(raw3(m.position()), raw3(p(2, 3)));
    assert_eq!((m.speed().to_raw(), m.step().to_raw()), (65_536, 98_304));
    assert!(m.route().is_none());
    assert_eq!(
        bits(dst.world(), new),
        ([0x4000_0000, 0, 0x4040_0000], [0; 3])
    );
    assert_eq!(dst.world().nav(), None);
    assert_eq!(dst.version(), 1);
}

#[test]
fn leaving_and_releasing_an_encounter_leave_movers_alone() {
    let mut fx = move_rt();
    enlist(&mut fx, 113, 8);
    let mut w = World::new(7);
    w.bind_nav(rev());
    start_in(&mut w, &fx, EncounterMode::RealTime).unwrap();
    let (a, b) = (who(&w, 111), who(&w, 112));
    w.set_route(a, route(1, vec![p(0, 0), p(2, 0)])).unwrap();
    let movers = value_of(&w)["movers"].clone();
    leave_encounter(&mut w, a).unwrap();
    assert!(w.creatures().get(a).is_some());
    assert_eq!(value_of(&w)["movers"], movers);
    assert_eq!(
        w.movers().get(a).unwrap().route().unwrap().tag(),
        RouteTag(1)
    );
    assert!(w.combatants().get(b).is_some());
    end_encounter(&mut w).unwrap();
    assert_eq!(value_of(&w)["movers"], movers);
}

// -------------------------------------------------------------- load coherence

fn base() -> World {
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111]);
    w.set_route(ids[0], route(7, vec![p(0, 0), p(3, 4)]))
        .unwrap();
    w
}
fn load_err(w: &World, edit: impl FnOnce(&mut Value)) -> String {
    try_with_json(w, edit).expect_err("the edited save must not load")
}
fn expect_rule(edit: impl FnOnce(&mut Value), text: &str) {
    let err = load_err(&base(), edit);
    assert!(err.contains(text), "wanted `{text}`, got `{err}`");
}
const L2: &str = "mover speed outside its limits";
const L3: &str = "mover step is not positive";
const L4: &str = "mover position outside the coordinate limit";
const L5: &str = "mover without a transform";
const L6: &str = "mover transform disagrees with its position";
const L7: &str = "mover route without a bound navmesh";
const L8: &str = "mover route revision differs from the bound navmesh";
const L9: &str = "mover route fails validation";
const L10: &str = "mover route segment out of range";
const L11: &str = "mover route progress out of range";
const L12: &str = "mover position disagrees with its route";

fn mv(s: &mut Value) -> &mut Value {
    &mut s["movers"][0][1]
}
fn set_mirror_position(s: &mut Value, raw: [i32; 3]) {
    for (i, r) in raw.iter().enumerate() {
        s["transforms"][0][1]["position"][i] = json!(to_f32(Fx16_16::from_raw(*r)));
    }
}

#[test]
fn load_round_trips_keep_movers_and_nav_exactly() {
    let w = base();
    let text = serde_json::to_string(&w).unwrap();
    let back: World = serde_json::from_str(&text).unwrap();
    assert_eq!(serde_json::to_string(&back).unwrap(), text);
    assert_eq!(back.nav(), Some(rev()));
    let tag = back
        .movers()
        .iter()
        .next()
        .unwrap()
        .1
        .route()
        .unwrap()
        .tag();
    assert_eq!(tag, RouteTag(7));
    // a started world with a movement ruleset, and a no-route mover with no nav
    let (w, _) = move_fixture().start(7);
    let text = serde_json::to_string(&w).unwrap();
    let back: World = serde_json::from_str(&text).unwrap();
    assert_eq!(serde_json::to_string(&back).unwrap(), text);
    assert_eq!(back.movers().len(), 2);
    assert_eq!(back.nav(), None);
    // and the standard (mover-free) world
    let (w, _) = Fixture::standard().start(7);
    let text = serde_json::to_string(&w).unwrap();
    let back: World = serde_json::from_str(&text).unwrap();
    assert_eq!(serde_json::to_string(&back).unwrap(), text);
}

#[test]
fn load_rules_one_by_one() {
    let w = base();
    let ghost = serde_json::to_value(eid(99, 1)).unwrap();
    let err = load_err(&w, |s| s["movers"][0][0] = ghost.clone());
    assert!(
        err.contains(&format!("dangling mover for non-live {:?}", eid(99, 1))),
        "{err}"
    );
    for speed in [0, -1, 262_145] {
        expect_rule(|s| mv(s)["speed"] = json!(speed), L2);
    }
    for step in [0, -1] {
        expect_rule(|s| mv(s)["step"] = json!(step), L3);
    }
    for (i, v) in [(0, 536_870_913), (1, 536_870_913), (2, -536_870_913)] {
        expect_rule(|s| mv(s)["position"][i] = json!(v), L4);
    }
    expect_rule(|s| s["transforms"] = json!([]), L5);
    for i in 0..3 {
        expect_rule(
            |s| s["transforms"][0][1]["position"][i] = json!(1.0_f32),
            L6,
        );
    }
    expect_rule(
        |s| s["transforms"][0][1]["position"][0] = json!(1e-7_f32),
        L6,
    );
    expect_rule(
        |s| {
            s.as_object_mut().unwrap().remove("nav");
        },
        L7,
    );
    expect_rule(|s| s["nav"] = serde_json::to_value(other()).unwrap(), L8);
    expect_rule(
        |s| mv(s)["route"]["route"]["points"] = json!([[0, 0, 0]]),
        L9,
    );
    expect_rule(|s| mv(s)["route"]["segment"] = json!(1), L10);
    expect_rule(|s| mv(s)["route"]["segment"] = json!(65_535), L10);
    expect_rule(|s| mv(s)["route"]["along"] = json!(327_680), L11);
    expect_rule(|s| mv(s)["route"]["along"] = json!(65_536), L12);
}

#[test]
fn load_boundaries_accept_one_unit_inside() {
    // speed and step limits
    for speed in [1, 262_144] {
        assert!(try_with_json(&base(), |s| mv(s)["speed"] = json!(speed)).is_ok());
    }
    assert!(try_with_json(&base(), |s| mv(s)["step"] = json!(1)).is_ok());
    // exactly 2^29 passes L4 (and is then caught by L6, since the mirror disagrees)
    expect_rule(|s| mv(s)["position"][0] = json!(536_870_912), L6);
    expect_rule(|s| mv(s)["position"][1] = json!(-536_870_912), L6);
    // a velocity that disagrees is fine, and -0.0 == 0.0 under L6
    assert!(
        try_with_json(&base(), |s| s["transforms"][0][1]["velocity"] =
            json!([5.0, 1.0, -2.0]))
        .is_ok()
    );
    assert!(
        try_with_json(&base(), |s| s["transforms"][0][1]["position"][0] =
            json!(-0.0_f32))
        .is_ok()
    );
    // L11/L12 split: one below the segment length passes L11 and meets L12
    expect_rule(|s| mv(s)["route"]["along"] = json!(327_679), L12);
    // a mover with no route needs no nav, and with nav it loads too
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111]);
    w.bind_nav(rev());
    w.clear_route(ids[0]).unwrap();
    assert!(try_with_json(&w, |_| {}).is_ok());
    assert!(try_with_json(&w, |s| {
        s.as_object_mut().unwrap().remove("nav");
    })
    .is_ok());
    // a world with a nav but no mover loads
    let mut w = World::new(7);
    w.bind_nav(rev());
    assert_eq!(with_json(&w, |_| {}).nav(), Some(rev()));
}

fn crafted(
    points: Vec<[Fx16_16; 3]>,
    segment: u64,
    along: u64,
    pos: [i32; 3],
) -> Result<World, String> {
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111]);
    w.set_route(ids[0], route(7, points)).unwrap();
    try_with_json(&w, |s| {
        mv(s)["route"]["segment"] = json!(segment);
        mv(s)["route"]["along"] = json!(along);
        mv(s)["position"] = json!(pos);
        set_mirror_position(s, pos);
    })
}

#[test]
fn load_route_progress_control_cases() {
    // Contract control: along 65536 into p(0,0)->p(3,4) is [39321, 0, 52428].
    let w = crafted(vec![p(0, 0), p(3, 4)], 0, 65_536, [39_321, 0, 52_428]).unwrap();
    let m = w.movers().iter().next().unwrap().1;
    assert_eq!(
        (m.route().unwrap().segment(), m.route().unwrap().along()),
        (0, 65_536)
    );
    assert_eq!(raw3(m.position()), [39_321, 0, 52_428]);
    // one raw unit off either way on any component is L12
    for off in [
        [39_322, 0, 52_428],
        [39_321, 1, 52_428],
        [39_321, 0, 52_429],
        [39_320, 0, 52_428],
    ] {
        let err = crafted(vec![p(0, 0), p(3, 4)], 0, 65_536, off)
            .err()
            .unwrap();
        assert!(err.contains(L12), "{off:?}: {err}");
    }
    // second segment: the segment index and its own length are used
    let pts = vec![p(0, 0), p(3, 4), p(3, 10)];
    assert!(crafted(pts.clone(), 1, 0, [196_608, 0, 262_144]).is_ok());
    assert!(crafted(pts.clone(), 1, 327_680, [196_608, 0, 589_824]).is_ok());
    assert!(crafted(pts.clone(), 1, 393_215, [196_608, 0, 655_359]).is_ok());
    let err = crafted(pts.clone(), 1, 393_216, [196_608, 0, 655_360])
        .err()
        .unwrap();
    assert!(err.contains(L11), "{err}");
    let err = crafted(pts.clone(), 2, 0, [196_608, 0, 655_360])
        .err()
        .unwrap();
    assert!(err.contains(L10), "{err}");
    // the interpolation includes height
    let climb = vec![rp(0, 0, 0), rp(655_360, 131_072, 0)];
    assert!(crafted(climb.clone(), 0, 327_680, [327_680, 65_536, 0]).is_ok());
    let err = crafted(climb, 0, 327_680, [327_680, 0, 0]).err().unwrap();
    assert!(err.contains(L12), "{err}");
}

#[test]
fn load_l9_covers_the_first_four_route_checks_only() {
    let bad = |points: Value| {
        expect_rule(
            move |s| mv(s)["route"]["route"]["points"] = points.clone(),
            L9,
        )
    };
    bad(json!([]));
    bad(json!([[0, 0, 0], [0, 0, 0]]));
    bad(json!([[0, 0, 0], [536_870_913, 0, 0]]));
    bad(json!([[0, 0, 0], [268_435_457, 0, 0]]));
    bad(Value::Array(
        (0..65).map(|x| json!([x * 65_536, 0, 0])).collect(),
    ));
    // the start need not equal anything but the position (L12, not L9)
    expect_rule(
        |s| mv(s)["route"]["route"]["points"] = json!([[65_536, 0, 0], [196_608, 0, 262_144]]),
        L12,
    );
    // unknown fields are refused
    assert!(load_err(&base(), |s| mv(s)["extra"] = json!(1)).contains("unknown field"));
    assert!(load_err(&base(), |s| mv(s)["route"]["extra"] = json!(1)).contains("unknown field"));
}

#[test]
fn load_rule_order_every_adjacent_pair() {
    let ghost = serde_json::to_value(eid(99, 1)).unwrap();
    let nav_other = serde_json::to_value(other()).unwrap();
    type Edit = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(&str, Edit, &str)> = vec![
        (
            "L1 over L2",
            Box::new(move |s| {
                s["movers"][0][0] = ghost.clone();
                mv(s)["speed"] = json!(0);
            }),
            "dangling mover for non-live",
        ),
        (
            "L2 over L3",
            Box::new(|s| {
                mv(s)["speed"] = json!(0);
                mv(s)["step"] = json!(0);
            }),
            L2,
        ),
        (
            "L3 over L4",
            Box::new(|s| {
                mv(s)["step"] = json!(0);
                mv(s)["position"][0] = json!(LIMIT + 1);
            }),
            L3,
        ),
        (
            "L4 over L5",
            Box::new(|s| {
                mv(s)["position"][0] = json!(LIMIT + 1);
                s["transforms"] = json!([]);
            }),
            L4,
        ),
        (
            "L4 over L6",
            Box::new(|s| {
                mv(s)["position"][0] = json!(LIMIT + 1);
            }),
            L4,
        ),
        (
            "L5 over L7",
            Box::new(|s| {
                s["transforms"] = json!([]);
                s.as_object_mut().unwrap().remove("nav");
            }),
            L5,
        ),
        (
            "L6 over L7",
            Box::new(|s| {
                s["transforms"][0][1]["position"][0] = json!(1.0_f32);
                s.as_object_mut().unwrap().remove("nav");
            }),
            L6,
        ),
        (
            "L6 over L8",
            Box::new(move |s| {
                s["transforms"][0][1]["position"][0] = json!(1.0_f32);
                s["nav"] = nav_other.clone();
            }),
            L6,
        ),
        (
            "L7 over L9",
            Box::new(|s| {
                s.as_object_mut().unwrap().remove("nav");
                mv(s)["route"]["route"]["points"] = json!([[0, 0, 0]]);
            }),
            L7,
        ),
        (
            "L8 over L9",
            Box::new({
                let n = serde_json::to_value(other()).unwrap();
                move |s| {
                    s["nav"] = n.clone();
                    mv(s)["route"]["route"]["points"] = json!([[0, 0, 0]]);
                }
            }),
            L8,
        ),
        (
            "L9 over L10",
            Box::new(|s| {
                mv(s)["route"]["route"]["points"] = json!([[0, 0, 0]]);
                mv(s)["route"]["segment"] = json!(1);
            }),
            L9,
        ),
        (
            "L10 over L11",
            Box::new(|s| {
                mv(s)["route"]["segment"] = json!(1);
                mv(s)["route"]["along"] = json!(327_680);
            }),
            L10,
        ),
        (
            "L11 over L12",
            Box::new(|s| {
                mv(s)["route"]["along"] = json!(327_680);
            }),
            L11,
        ),
    ];
    for (name, edit, want) in cases {
        let err = load_err(&base(), |s| edit(s));
        assert!(err.contains(want), "{name}: wanted `{want}`, got `{err}`");
    }
}

#[test]
fn mover_checks_run_after_creature_checks_and_before_combat_checks() {
    // creature record first
    let fx = move_fixture();
    let (w, ids) = registered(&fx, &[111]);
    let a = ids[0];
    let err = load_err(&w, |s| {
        mv(s)["speed"] = json!(0);
        support::creature::edit_record(s, a, |r| r["pools"] = json!([]));
    });
    assert!(err.contains("creature declares no pools"), "{err}");
    // then movers, then combat
    let (w, _) = fx.start(7);
    let err = load_err(&w, |s| {
        mv(s)["speed"] = json!(0);
        s["combat"]["definition"]["attribute"] = json!("undeclared-attribute");
    });
    assert!(err.contains(L2), "{err}");
    let err = load_err(&w, |s| {
        s["combat"]["definition"]["attribute"] = json!("undeclared-attribute");
    });
    assert!(
        err.contains("combat definition names an undeclared checked attribute"),
        "{err}"
    );
}

// ---------------------------------------------------------------- byte identity

#[test]
fn byte_identity_a9() {
    assert_eq!(
        serde_json::to_string(&World::new(7)).unwrap(),
        EMPTY_WORLD_SEED_7
    );
    let (w, _) = Fixture::standard().start(7);
    let keys = json_keys(&value_of(&w));
    for key in ["movers", "nav", "speed", "step", "segment"] {
        assert!(!keys.contains(key), "{key}");
    }
    // registering and starting a creature of a movement-free ruleset writes no new key
    let mut w = World::new(7);
    Fixture::standard().register(&mut w, pl(111)).unwrap();
    assert!(!json_keys(&value_of(&w)).contains("movers"));
    let fx = move_fixture();
    let (mut w, ids) = registered(&fx, &[111]);
    let _ = ids;
    // `registered` binds the nav: the last two keys are movers, nav
    let keys = top_level_keys(&bytes_of(&w));
    assert_eq!(&keys[keys.len() - 2..], ["movers", "nav"]);
    let mut w2 = World::new(7);
    fx.register(&mut w2, pl(111)).unwrap();
    let keys = top_level_keys(&bytes_of(&w2));
    assert_eq!(keys.last().map(String::as_str), Some("movers"));
    assert!(!keys.contains(&"nav".to_owned()));
    let mut only_nav = World::new(7);
    only_nav.bind_nav(rev());
    let keys = top_level_keys(&bytes_of(&only_nav));
    assert_eq!(keys.last().map(String::as_str), Some("nav"));
    assert!(!keys.contains(&"movers".to_owned()));
    // the mover-bearing world still writes the pre-existing keys unchanged in front
    w.despawn(who(&w, 111));
    let keys = top_level_keys(&bytes_of(&w));
    assert_eq!(keys.last().map(String::as_str), Some("nav"));
}

// ------------------------------------------------------------------ Display

#[test]
fn display_texts_a10() {
    let e = {
        let mut w = World::new(1);
        w.spawn(EntityMeta {})
    };
    let cases = [
        (
            MoveError::AbsentEntity { entity: e },
            format!("AbsentEntity at movers/{e:?}"),
        ),
        (
            MoveError::NotMover { entity: e },
            format!("NotMover at movers/{e:?}"),
        ),
        (
            MoveError::DeadCombatant { entity: e },
            format!("DeadCombatant at movers/{e:?}"),
        ),
        (
            MoveError::TurnBased { entity: e },
            format!("TurnBased at movers/{e:?}"),
        ),
        (
            MoveError::RouteSize { points: 3 },
            "RouteSize at route/points".to_owned(),
        ),
        (
            MoveError::OutOfBounds { index: 1 },
            "OutOfBounds at route/points".to_owned(),
        ),
        (
            MoveError::ZeroSegment { index: 1 },
            "ZeroSegment at route/points".to_owned(),
        ),
        (
            MoveError::TooLong { length: 9 },
            "TooLong at route/length".to_owned(),
        ),
        (MoveError::NavUnbound, "NavUnbound at nav".to_owned()),
        (
            MoveError::StaleRevision,
            "StaleRevision at route/revision".to_owned(),
        ),
        (
            MoveError::StartMismatch,
            "StartMismatch at route/points".to_owned(),
        ),
        (
            MoveError::CannotAfford {
                cost: 7,
                current: 6,
            },
            "CannotAfford at combat/pool".to_owned(),
        ),
        (
            MoveError::Combat(CombatError::NoEncounter),
            "NoEncounter at combat".to_owned(),
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
        let wants_source = matches!(error, MoveError::Combat(_));
        assert_eq!(error.source().is_some(), wants_source, "{error:?}");
    }
    let inner = MoveError::Combat(CombatError::NoEncounter);
    assert_eq!(inner.source().unwrap().to_string(), "NoEncounter at combat");
    let placement = pl(111);
    assert_eq!(
        CombatError::InvalidPosition { placement }.to_string(),
        "InvalidPosition at spec/placements"
    );
    assert_eq!(
        CombatError::InvalidSpeed { placement }.to_string(),
        "InvalidSpeed at spec/movement"
    );
    assert_eq!(
        CombatError::InvalidStep { placement }.to_string(),
        "InvalidStep at spec/movement"
    );
    for e in [
        MoveError::NavUnbound,
        MoveError::Combat(CombatError::NoEncounter),
    ] {
        let h = HistoryError::Move(e.clone());
        assert_eq!(h.to_string(), e.to_string());
        assert_eq!(h.source().expect("a source").to_string(), e.to_string());
    }
}

// ------------------------------------------- mutant-review regressions (T036a)

/// B-L3: registration runs the movement checks (R7-R9) only after R6
/// `TooManyTags`, so an over-tagged creature with an invalid speed is
/// `TooManyTags`. One tag fewer lets R7 fail as `InvalidSpeed`.
#[test]
fn registration_tag_bound_wins_over_the_movement_checks() {
    let names: Vec<String> = (0..=MAX_TAGS).map(|i| format!("t{i:05}")).collect();
    let over: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut fx = move_fixture();
    set_tags(&mut fx, pl(121), &over);
    set_stat_raw(&mut fx, pl(121), "pace", 0);
    let mut w = World::new(7);
    let before = serde_json::to_string(&w).unwrap();
    assert_eq!(fx.register(&mut w, pl(111)), Err(CombatError::TooManyTags));
    assert_eq!(serde_json::to_string(&w).unwrap(), before, "no mutation");
    let at_bound: Vec<&str> = over[..MAX_TAGS].to_vec();
    set_tags(&mut fx, pl(121), &at_bound);
    assert_eq!(
        reg_err(&fx),
        CombatError::InvalidSpeed { placement: pl(111) }
    );
}

/// B-L4: a turn-based start runs its movement checks after the tag-bound
/// check and before S1, so an over-tagged A with `pace` raw 0 is
/// `TooManyTags`. One tag fewer makes the movement check win.
#[test]
fn tb_start_tag_bound_wins_over_the_movement_checks() {
    let names: Vec<String> = (0..=MAX_TAGS).map(|i| format!("t{i:05}")).collect();
    let over: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut fx = move_fixture();
    set_tags(&mut fx, pl(121), &over);
    set_stat_raw(&mut fx, pl(121), "pace", 0);
    let mut w = World::new(7);
    let before = serde_json::to_string(&w).unwrap();
    assert_eq!(start_into(&fx, &mut w), Err(CombatError::TooManyTags));
    assert_eq!(serde_json::to_string(&w).unwrap(), before, "no mutation");
    let at_bound: Vec<&str> = over[..MAX_TAGS].to_vec();
    set_tags(&mut fx, pl(121), &at_bound);
    assert_eq!(
        start_into(&fx, &mut World::new(7)),
        Err(CombatError::InvalidSpeed { placement: pl(111) })
    );
}

/// B-R1: `ZeroSegment` reports the first zero-length segment's index.
#[test]
fn zero_segment_reports_the_first_zero_segment() {
    let start = p(0, 0);
    assert_eq!(
        validate_route(
            &route(1, vec![p(0, 0), p(0, 0), p(1, 0), p(1, 0)]),
            Some(rev()),
            start
        ),
        Err(MoveError::ZeroSegment { index: 0 })
    );
    // One point removed: the later zero segment is the first one left.
    assert_eq!(
        validate_route(
            &route(1, vec![p(0, 0), p(1, 0), p(1, 0)]),
            Some(rev()),
            start
        ),
        Err(MoveError::ZeroSegment { index: 1 })
    );
    assert_eq!(
        validate_route(&route(1, vec![p(0, 0), p(1, 0)]), Some(rev()), start),
        Ok(65_536)
    );
}

/// B-L2: the expression operators map one to one, so `divide` by a fixed
/// 131072 halves raw 65536 to 32768; `multiply` would give 131072.
#[test]
fn speed_division_maps_one_to_one() {
    let fx = quick(bin("divide", st("pace"), fixed_lit(131_072)));
    let mut w = World::new(7);
    let a = fx.register(&mut w, pl(111)).unwrap();
    assert_eq!(w.movers().get(a).unwrap().speed().to_raw(), 32_768);
    let fx = quick(bin("multiply", st("pace"), fixed_lit(131_072)));
    let mut w = World::new(7);
    let a = fx.register(&mut w, pl(111)).unwrap();
    assert_eq!(w.movers().get(a).unwrap().speed().to_raw(), 131_072);
}

/// B-M2: the history transfer writes `mirror(mover, zero velocity)` at the
/// destination, overwriting the transform copied from a source whose velocity
/// was hand-edited (L6 checks position only, so that save loads).
#[test]
fn history_transfer_rewrites_the_mirror_with_zero_velocity() {
    let fx = move_fixture();
    let mut src = HistoryWorld::new_in_area(1, pl(131));
    let a = fx.register_history(&mut src, pl(111)).unwrap();
    let mut save = serde_json::to_value(&src).unwrap();
    save["world"]["transforms"][0][1]["velocity"] = json!([1.0, 0.0, 0.0]);
    let mut src: HistoryWorld = serde_json::from_value(save).unwrap();
    assert_eq!(
        bits(src.world(), a).1,
        [0x3f80_0000, 0, 0],
        "the hand edit loads, so only the transfer can zero it"
    );
    let mut dst = HistoryWorld::new_in_area(2, pl(132));
    let new = transfer_history_entity(&mut src, &mut dst, a).unwrap();
    assert_eq!(
        raw3(dst.world().movers().get(new).unwrap().position()),
        [0, 0, 0]
    );
    assert_eq!(
        bits(dst.world(), new),
        ([0; 3], [0; 3]),
        "the mover's mirror wins over the copied transform"
    );
}
