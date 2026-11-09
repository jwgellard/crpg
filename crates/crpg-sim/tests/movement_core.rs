//! T036a: movement state, routes and operations (Behaviour B-U1..B-M4,
//! B-L6; tests A-1..A-10). The MOVE fixture is built from the standard
//! fixture by editing its documents through JSON, as the contract says.

#[allow(dead_code)]
mod support;

use std::error::Error as _;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_sim::{
    end_encounter, leave_encounter, perform_action, point_at, request_mode, route_length,
    route_measure, seg_len, snap, to_f32, transfer_entity, transfer_history_entity, validate_route,
    CombatError, EncounterMode, EntityMeta, HistoryError, HistoryWorld, Measure, MoveError,
    MoveRoute, NavRevision, RouteTag, World, COORD_LIMIT_RAW, MAX_ROUTE_LENGTH_RAW,
    MAX_ROUTE_POINTS,
};
use serde_json::{json, Value};
use support::creature::{top_level_keys, try_with_json};
use support::draws::start_into;
use support::rt::{bytes_of, json_keys, make_lethal, use_action, value_of};
use support::{fx_raw, uid, Fixture};

const LIMIT: i32 = COORD_LIMIT_RAW;

type Point = [Fx16_16; 3];

/// `p(x, z)`: whole metres on the ground plane.
fn p(x: i32, z: i32) -> Point {
    [Fx16_16::from_int(x), Fx16_16::ZERO, Fx16_16::from_int(z)]
}

/// `rp(x, y, z)`: the point with those raw values.
fn rp(x: i32, y: i32, z: i32) -> Point {
    [
        Fx16_16::from_raw(x),
        Fx16_16::from_raw(y),
        Fx16_16::from_raw(z),
    ]
}

fn rev() -> NavRevision {
    NavRevision::of_bytes(b"mesh-a")
}

fn other() -> NavRevision {
    NavRevision::of_bytes(b"mesh-b")
}

fn route(tag: u64, points: Vec<Point>) -> MoveRoute {
    MoveRoute {
        revision: rev(),
        tag: RouteTag(tag),
        points,
    }
}

fn pl(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

fn bits(values: [f32; 3]) -> [u32; 3] {
    [
        values[0].to_bits(),
        values[1].to_bits(),
        values[2].to_bits(),
    ]
}

/// Edits the ruleset document through JSON.
fn edit_ruleset(fixture: &mut Fixture, edit: impl FnOnce(&mut Value)) {
    let mut value = serde_json::to_value(&fixture.ruleset).expect("the ruleset serializes");
    edit(&mut value);
    fixture.ruleset = serde_json::from_value(value).expect("the edited ruleset parses");
}

fn set_creature_stat(fixture: &mut Fixture, creature: u128, name: &str, raw: i32) {
    fixture
        .creatures
        .get_mut(&pl(creature))
        .expect("the creature exists")
        .stats
        .insert(name.to_owned(), Fx16_16::from_raw(raw));
}

/// The MOVE fixture of the contract.
fn move_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    edit_ruleset(&mut fixture, |ruleset| {
        ruleset["stats"]
            .as_array_mut()
            .expect("stats")
            .push(json!({"name": "pace", "kind": "fixed"}));
        ruleset["pools"]
            .as_array_mut()
            .expect("pools")
            .push(json!({"id": uid(106), "max": 6, "refresh": {"type": "on_turn_start"}}));
        ruleset["movement"] = json!({
            "speed": {"type": "stat", "value": "pace"},
            "turn": {
                "pool": uid(106),
                "step": {"type": "constant", "value": 98304},
                "step_cost": 1
            }
        });
    });
    set_creature_stat(&mut fixture, 121, "pace", 65536);
    set_creature_stat(&mut fixture, 122, "pace", 32768);
    fixture
        .placements
        .get_mut(&pl(112))
        .expect("placement B")
        .0
        .transform
        .position = [Fx16_16::from_raw(655360), Fx16_16::ZERO, Fx16_16::ZERO];
    // Creature C and placement 113 (not a participant).
    let mut creature_c = fixture.creatures[&pl(121)].clone();
    creature_c.id = pl(123);
    creature_c.slug = "probe-creature-c".to_owned();
    creature_c.name = "probe.creature-c".to_owned();
    creature_c
        .stats
        .insert("alpha".to_owned(), Fx16_16::from_int(5));
    creature_c
        .stats
        .insert("beta".to_owned(), Fx16_16::from_int(5));
    creature_c
        .stats
        .insert("gamma".to_owned(), Fx16_16::from_int(5));
    creature_c
        .stats
        .insert("health".to_owned(), Fx16_16::from_int(4));
    creature_c
        .stats
        .insert("pace".to_owned(), Fx16_16::from_raw(65536));
    creature_c.tags.clear();
    fixture.creatures.insert(pl(123), creature_c);
    let mut doc_c = fixture.placements[&pl(111)].0.clone();
    doc_c.id = pl(113);
    doc_c.slug = "probe-c".to_owned();
    doc_c.name = "probe.c".to_owned();
    doc_c.prefab = pl(123);
    doc_c.transform.position = [Fx16_16::ZERO, Fx16_16::ZERO, Fx16_16::from_raw(655360)];
    let area = fixture.area;
    fixture.placements.insert(pl(113), (doc_c, area));
    fixture
}

/// MOVE_RT: MOVE with `round_ticks: 4`.
fn move_rt_fixture() -> Fixture {
    let mut fixture = move_fixture();
    fixture.ruleset.round_ticks = Some(4);
    fixture
}

fn reg(fixture: &Fixture, world: &mut World, placement: u128) -> EntityId {
    fixture
        .register(world, pl(placement))
        .expect("the placement registers")
}

/// MOVE, `World::new(7)`, with A and B registered.
fn registered() -> (Fixture, World, EntityId, EntityId) {
    let fixture = move_fixture();
    let mut world = World::new(7);
    let a = reg(&fixture, &mut world, 111);
    let b = reg(&fixture, &mut world, 112);
    (fixture, world, a, b)
}

/// A spawned plain entity.
fn plain(world: &mut World) -> EntityId {
    world.spawn(EntityMeta {})
}

/// A despawned id.
fn despawned(world: &mut World) -> EntityId {
    let id = plain(world);
    assert!(world.despawn(id));
    id
}

/// MOVE turn-based fight where B (acting first) has killed A.
fn lethal_fight() -> (Fixture, World, EntityId, EntityId) {
    let mut fixture = move_fixture();
    make_lethal(&mut fixture, 99);
    let (mut world, ids) = fixture.start(7);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B kills A");
    assert!(world.combatants().get(a).expect("A").dead());
    (fixture, world, a, b)
}

fn mover_state(world: &World, entity: EntityId) -> (Point, i32, i32, Option<u64>) {
    let mover = world.movers().get(entity).expect("a mover");
    (
        mover.position(),
        mover.speed().to_raw(),
        mover.step().to_raw(),
        mover.route().map(|active| active.tag().0),
    )
}

fn mirror_of(world: &World, entity: EntityId) -> ([u32; 3], [u32; 3]) {
    let transform = world.transforms().get(entity).expect("a mirror");
    (bits(transform.position), bits(transform.velocity))
}

/// Gives the entity a nonzero velocity, so that a zero-velocity rewrite shows.
fn poison_velocity(world: &mut World, entity: EntityId) {
    world
        .transforms_mut()
        .get_mut(entity)
        .expect("a mirror")
        .velocity = [1.0, 1.0, 1.0];
}

/// Runs `call` and asserts the world serialized to the same bytes after it.
fn unchanged<T>(world: &mut World, call: impl FnOnce(&mut World) -> T) -> T {
    let before = bytes_of(world);
    let out = call(world);
    assert_eq!(bytes_of(world), before, "a failing call changes nothing");
    out
}

// ---------------------------------------------------------------------------
// A-1 .. A-3
// ---------------------------------------------------------------------------

#[test]
fn a1_lengths() {
    assert_eq!(seg_len(p(0, 0), p(3, 4)), 327680);
    assert_eq!(seg_len(p(3, 4), p(0, 0)), 327680);
    assert_eq!(seg_len(rp(0, 0, 0), rp(1, 0, 1)), 2);
    assert_eq!(seg_len(rp(0, 0, 0), rp(3, 0, 0)), 3);
    assert_eq!(seg_len(rp(0, 0, 0), rp(65536, 0, 65536)), 92682);
    assert_eq!(seg_len(p(0, 0), rp(0, 327680, 0)), 0);
    assert_eq!(
        seg_len(rp(-LIMIT, 0, -LIMIT), rp(LIMIT, 0, LIMIT)),
        1518500250
    );
    assert_eq!(
        seg_len(rp(i32::MIN, 0, i32::MIN), rp(i32::MAX, 0, i32::MAX)),
        6074000999
    );
    // `y` never counts.
    assert_eq!(
        seg_len(rp(0, i32::MIN, 0), rp(3, i32::MAX, 4)),
        seg_len(rp(0, 0, 0), rp(3, 0, 4))
    );
}

#[test]
fn a2_interpolation() {
    assert_eq!(
        point_at(p(0, 0), p(3, 4), 327680, 163840),
        rp(98304, 0, 131072)
    );
    // Truncation toward zero, not flooring.
    assert_eq!(point_at(rp(0, 0, 0), rp(-2, 0, -1), 3, 1), rp(0, 0, 0));
    assert_eq!(point_at(rp(0, 0, 0), rp(-2, 0, -1), 3, 2), rp(-1, 0, 0));
    // `y` interpolates.
    assert_eq!(
        point_at(rp(0, 0, 0), rp(655360, 131072, 0), 655360, 327680),
        rp(327680, 65536, 0)
    );
    // `d == len` and `d > len` give `b`; `len == 0` gives `a`.
    assert_eq!(point_at(p(0, 0), p(3, 4), 327680, 327680), p(3, 4));
    assert_eq!(point_at(p(0, 0), p(3, 4), 327680, 999999), p(3, 4));
    assert_eq!(point_at(p(0, 0), p(3, 4), 0, 5), p(0, 0));
    // Total: extreme inputs do not panic; the midpoint truncates toward zero.
    let low = rp(i32::MIN, i32::MIN, i32::MIN);
    let high = rp(i32::MAX, i32::MAX, i32::MAX);
    assert_eq!(point_at(low, high, u64::MAX, u64::MAX / 2), rp(-1, -1, -1));
}

fn unhex(text: &str) -> [u8; 32] {
    let mut out = [0_u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).expect("hex");
    }
    out
}

#[test]
fn a3_measures_and_projection() {
    let route_points = [p(0, 0), p(3, 4), p(3, 10)];
    assert_eq!(route_length(&route_points), Some(720896));
    assert_eq!(
        route_measure(Measure::Euclidean, &route_points),
        Some(720896)
    );
    assert_eq!(route_length(&[]), None);
    assert_eq!(route_length(&[p(0, 0)]), None);
    assert_eq!(route_measure(Measure::Euclidean, &[p(0, 0)]), None);
    assert_eq!(route_length(&[p(0, 0), rp(LIMIT + 1, 0, 0)]), None);
    assert_eq!(route_length(&[rp(0, 0, -LIMIT - 1), p(0, 0)]), None);
    assert_eq!(route_length(&[p(0, 0), rp(0, i32::MIN, 0)]), None);
    assert_eq!(
        route_length(&[p(0, 0), rp(LIMIT, 0, 0)]),
        Some(LIMIT as u64)
    );
    assert_eq!(
        route_length(&[p(0, 0), rp(-LIMIT, 0, 0)]),
        Some(LIMIT as u64)
    );
    assert_eq!(snap(Measure::Euclidean, rp(1, 2, 3)), rp(1, 2, 3));
    assert_eq!(Measure::default(), Measure::Euclidean);

    let cases: [(i32, u32); 6] = [
        (65536, 0x3f80_0000),
        (-98304, 0xbfc0_0000),
        (1, 0x3780_0000),
        (1 << 29, 0x4600_0000),
        (16_777_217, 0x4380_0000),
        (i32::MIN, 0xc700_0000),
    ];
    for (raw, expected) in cases {
        assert_eq!(
            to_f32(Fx16_16::from_raw(raw)).to_bits(),
            expected,
            "to_f32({raw})"
        );
    }
    assert_eq!(
        NavRevision::of_bytes(b"").0,
        unhex("af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262")
    );
    assert_eq!(NavRevision::of_bytes(b"x"), NavRevision::of_bytes(b"x"));
    assert_ne!(rev(), other());
    assert_eq!(MAX_ROUTE_POINTS, 64, "U7 bounds the point count");
    assert_eq!(MAX_ROUTE_LENGTH_RAW, 1 << 28);
    assert_eq!(COORD_LIMIT_RAW, 1 << 29);
}

// ---------------------------------------------------------------------------
// A-4
// ---------------------------------------------------------------------------

fn check(points: Vec<Point>, nav: Option<NavRevision>, start: Point) -> Result<u64, MoveError> {
    validate_route(&route(1, points), nav, start)
}

fn ok_check(points: Vec<Point>) -> Result<u64, MoveError> {
    check(points, Some(rev()), p(0, 0))
}

#[test]
fn a4_route_validation() {
    assert_eq!(
        ok_check(vec![p(0, 0)]),
        Err(MoveError::RouteSize { points: 1 })
    );
    assert_eq!(ok_check(vec![]), Err(MoveError::RouteSize { points: 0 }));
    let sixty_five: Vec<Point> = (0..65).map(|x| p(x, 0)).collect();
    assert_eq!(
        ok_check(sixty_five),
        Err(MoveError::RouteSize { points: 65 })
    );
    let sixty_four: Vec<Point> = (0..64).map(|x| p(x, 0)).collect();
    assert_eq!(ok_check(sixty_four), Ok(4128768));
    assert_eq!(
        ok_check(vec![p(0, 0), rp(LIMIT + 1, 0, 0)]),
        Err(MoveError::OutOfBounds { index: 1 })
    );
    assert_eq!(
        ok_check(vec![p(0, 0), rp(0, -LIMIT - 1, 0)]),
        Err(MoveError::OutOfBounds { index: 1 })
    );
    assert_eq!(
        ok_check(vec![p(0, 0), rp(0, 0, i32::MIN)]),
        Err(MoveError::OutOfBounds { index: 1 })
    );
    assert_eq!(
        ok_check(vec![p(0, 0), p(0, 0), rp(LIMIT + 1, 0, 0)]),
        Err(MoveError::OutOfBounds { index: 2 })
    );
    assert_eq!(
        ok_check(vec![p(0, 0), p(1, 0), rp(65536, 327680, 0)]),
        Err(MoveError::ZeroSegment { index: 1 })
    );
    assert_eq!(ok_check(vec![p(0, 0), p(4096, 0)]), Ok(268435456));
    assert_eq!(
        ok_check(vec![p(0, 0), rp(268435457, 0, 0)]),
        Err(MoveError::TooLong { length: 268435457 })
    );
    assert_eq!(
        ok_check(vec![p(0, 0), p(2048, 0), p(0, 1)]),
        Err(MoveError::TooLong { length: 268435472 })
    );
    let valid = vec![p(0, 0), p(3, 4)];
    assert_eq!(
        check(valid.clone(), None, p(0, 0)),
        Err(MoveError::NavUnbound)
    );
    assert_eq!(
        check(valid.clone(), Some(other()), p(0, 0)),
        Err(MoveError::StaleRevision)
    );
    assert_eq!(
        check(valid.clone(), Some(rev()), rp(0, 65536, 0)),
        Err(MoveError::StartMismatch)
    );
    assert_eq!(
        check(valid.clone(), Some(rev()), rp(65536, 0, 0)),
        Err(MoveError::StartMismatch)
    );
    assert_eq!(
        check(valid.clone(), Some(rev()), rp(0, 0, 1)),
        Err(MoveError::StartMismatch)
    );
    assert_eq!(check(valid, Some(rev()), p(0, 0)), Ok(327680));
    // The first offending index wins.
    assert_eq!(
        ok_check(vec![p(0, 0), rp(LIMIT + 1, 0, 0), rp(LIMIT + 1, 0, 0)]),
        Err(MoveError::OutOfBounds { index: 1 })
    );
    // Every adjacent pair of the order, each case failing both checks.
    assert_eq!(
        ok_check(vec![rp(LIMIT + 1, 0, 0)]),
        Err(MoveError::RouteSize { points: 1 }),
        "size before bounds"
    );
    assert_eq!(
        ok_check(vec![p(0, 0), p(0, 0), rp(LIMIT + 1, 0, 0)]),
        Err(MoveError::OutOfBounds { index: 2 }),
        "bounds before zero segments"
    );
    assert_eq!(
        ok_check(vec![p(0, 0), p(0, 0), p(4097, 0)]),
        Err(MoveError::ZeroSegment { index: 0 }),
        "zero segment before length"
    );
    assert_eq!(
        check(vec![p(0, 0), rp(268435457, 0, 0)], None, p(0, 0)),
        Err(MoveError::TooLong { length: 268435457 }),
        "length before binding"
    );
    assert_eq!(
        validate_route(
            &MoveRoute {
                revision: other(),
                tag: RouteTag(1),
                points: vec![p(0, 0), p(3, 4)]
            },
            None,
            p(0, 0)
        ),
        Err(MoveError::NavUnbound),
        "binding before revision"
    );
    assert_eq!(
        check(vec![p(0, 0), p(3, 4)], Some(other()), rp(0, 65536, 0)),
        Err(MoveError::StaleRevision),
        "revision before start"
    );
    assert_eq!(
        check(vec![p(0, 0), p(3, 4)], None, rp(0, 65536, 0)),
        Err(MoveError::NavUnbound)
    );
}

// ---------------------------------------------------------------------------
// A-5
// ---------------------------------------------------------------------------

#[test]
fn a5_registration_creates_movers() {
    let (_, world, a, b) = registered();
    let a_mover = world.movers().get(a).expect("A's mover");
    assert_eq!(a_mover.position(), p(0, 0));
    assert_eq!(a_mover.speed().to_raw(), 65536);
    assert_eq!(a_mover.step().to_raw(), 98304);
    assert!(a_mover.route().is_none());
    assert_eq!(mirror_of(&world, a), ([0; 3], [0; 3]));
    let b_mover = world.movers().get(b).expect("B's mover");
    assert_eq!(b_mover.position(), rp(655360, 0, 0));
    assert_eq!(b_mover.speed().to_raw(), 32768);
    assert_eq!(mirror_of(&world, b), ([0x4120_0000, 0, 0], [0; 3]));
    assert_eq!(world.nav(), None);
}

#[test]
fn a5_standard_fixture_has_no_mover() {
    let fixture = Fixture::standard();
    let mut world = World::new(7);
    reg(&fixture, &mut world, 111);
    assert!(world.movers().is_empty());
    assert!(world.transforms().is_empty());
    let (started, _) = Fixture::standard().start(7);
    assert!(started.movers().is_empty());
    assert!(started.transforms().is_empty());
}

#[test]
fn a5_start_creates_movers_in_authored_order() {
    let fixture = move_fixture();
    let (world, ids) = fixture.start(7);
    let stored: Vec<EntityId> = world.movers().iter().map(|(id, _)| id).collect();
    assert_eq!(stored, ids);
    assert_eq!(mover_state(&world, ids[0]).0, p(0, 0));
    assert_eq!(mover_state(&world, ids[1]).0, p(10, 0));
    assert_eq!(mover_state(&world, ids[0]).1, 65536);
    assert_eq!(mover_state(&world, ids[1]).1, 32768);
    assert_eq!(mover_state(&world, ids[0]).2, 98304);
    assert_eq!(mirror_of(&world, ids[0]), ([0; 3], [0; 3]));
    assert_eq!(mirror_of(&world, ids[1]).0, [0x4120_0000, 0, 0]);
    // The real-time start creates them too.
    let (rt_world, rt_ids) = move_rt_fixture().start_rt(7, EncounterMode::RealTime);
    assert_eq!(rt_world.movers().len(), 2);
    assert!(rt_world.movers().contains(rt_ids[0]));
    assert!(rt_world.movers().contains(rt_ids[1]));
}

#[test]
fn a5_creation_draws_nothing_and_enqueues_no_extra_event() {
    let fixture = move_fixture();
    let mut with = World::new(7);
    let mut without = World::new(7);
    reg(&fixture, &mut with, 111);
    reg(&Fixture::standard(), &mut without, 111);
    assert_eq!(value_of(&with)["rng"], value_of(&without)["rng"]);
    assert_eq!(value_of(&with)["events"], value_of(&without)["events"]);
    assert_eq!(
        value_of(&with)["interners"],
        value_of(&without)["interners"]
    );
    assert!(with.interners().stats().is_empty());
    assert!(with.interners().tags().is_empty());
    let (started, _) = fixture.start(7);
    let (standard, _) = Fixture::standard().start(7);
    assert_eq!(value_of(&started)["rng"], value_of(&standard)["rng"]);
    assert_eq!(value_of(&started)["events"], value_of(&standard)["events"]);
}

fn register_error(mutate: impl FnOnce(&mut Fixture)) -> CombatError {
    let mut fixture = move_fixture();
    mutate(&mut fixture);
    let mut world = World::new(7);
    let before = bytes_of(&world);
    let error = fixture
        .register(&mut world, pl(111))
        .expect_err("registration fails");
    assert_eq!(bytes_of(&world), before, "world unchanged");
    error
}

fn set_pos_x(fixture: &mut Fixture, raw: i32) {
    fixture
        .placements
        .get_mut(&pl(111))
        .expect("placement A")
        .0
        .transform
        .position[0] = Fx16_16::from_raw(raw);
}

fn set_speed_constant(fixture: &mut Fixture, raw: i32) {
    edit_ruleset(fixture, |ruleset| {
        ruleset["movement"]["speed"] = json!({"type": "constant", "value": raw});
    });
}

fn set_step_constant(fixture: &mut Fixture, raw: i32) {
    edit_ruleset(fixture, |ruleset| {
        ruleset["movement"]["turn"]["step"] = json!({"type": "constant", "value": raw});
    });
}

#[test]
fn a5_registration_errors() {
    let bad_placement = CombatError::InvalidPosition { placement: pl(111) };
    let bad_speed = CombatError::InvalidSpeed { placement: pl(111) };
    let bad_step = CombatError::InvalidStep { placement: pl(111) };
    assert_eq!(register_error(|f| set_pos_x(f, LIMIT + 1)), bad_placement);
    assert_eq!(
        register_error(|f| set_creature_stat(f, 121, "pace", 0)),
        bad_speed
    );
    assert_eq!(
        register_error(|f| set_creature_stat(f, 121, "pace", 262145)),
        bad_speed
    );
    assert_eq!(
        register_error(|f| set_creature_stat(f, 121, "pace", -1)),
        bad_speed
    );
    assert_eq!(register_error(|f| set_speed_constant(f, 0)), bad_speed);
    assert_eq!(register_error(|f| set_step_constant(f, 0)), bad_step);
    assert_eq!(register_error(|f| set_step_constant(f, -5)), bad_step);
    assert_eq!(
        register_error(|f| {
            edit_ruleset(f, |ruleset| {
                ruleset["stats"]
                    .as_array_mut()
                    .expect("stats")
                    .push(json!({"name": "stride", "kind": "fixed"}));
                ruleset["movement"]["turn"]["step"] = json!({"type": "stat", "value": "stride"});
            });
            for creature in [121, 122, 123] {
                set_creature_stat(f, creature, "stride", 0);
            }
        }),
        bad_step
    );
    // Order.
    assert_eq!(
        register_error(|f| {
            set_pos_x(f, LIMIT + 1);
            set_creature_stat(f, 121, "pace", 0);
        }),
        bad_placement
    );
    assert_eq!(
        register_error(|f| {
            set_creature_stat(f, 121, "pace", 0);
            set_step_constant(f, 0);
        }),
        bad_speed
    );
    assert_eq!(
        register_error(|f| {
            f.creatures
                .get_mut(&pl(121))
                .expect("A")
                .stats
                .remove("pace");
        }),
        CombatError::MissingStat {
            stat: "pace".to_owned()
        }
    );
    // The limits themselves register.
    let mut fixture = move_fixture();
    set_creature_stat(&mut fixture, 121, "pace", 262144);
    set_pos_x(&mut fixture, LIMIT);
    let mut world = World::new(7);
    let a = reg(&fixture, &mut world, 111);
    assert_eq!(mover_state(&world, a).1, 262144);
    assert_eq!(mover_state(&world, a).0[0].to_raw(), LIMIT);
    set_creature_stat(&mut fixture, 122, "pace", 1);
    let b = reg(&fixture, &mut world, 112);
    assert_eq!(mover_state(&world, b).1, 1);
}

fn add_derived_fixed(fixture: &mut Fixture, name: &str, formula: Value) {
    edit_ruleset(fixture, |ruleset| {
        ruleset["stats"]
            .as_array_mut()
            .expect("stats")
            .push(json!({"name": name, "kind": "fixed", "derived": formula}));
        ruleset["movement"]["speed"] = json!({"type": "stat", "value": name});
    });
}

fn literal_fixed(raw: i32) -> Value {
    json!({"type": "literal", "value": {"type": "fixed", "value": raw}})
}

fn stat_ref(name: &str) -> Value {
    json!({"type": "stat", "value": name})
}

#[test]
fn a5_derived_fixed_lengths_are_evaluated() {
    let mut fixture = move_fixture();
    add_derived_fixed(
        &mut fixture,
        "quick",
        json!({"type": "add", "value": {"left": stat_ref("pace"), "right": literal_fixed(16384)}}),
    );
    let mut world = World::new(7);
    let a = reg(&fixture, &mut world, 111);
    assert_eq!(mover_state(&world, a).1, 81920);
    assert!(world.interners().stats().is_empty(), "nothing is interned");

    // A zero divisor fails the resolution.
    let error = register_error(|f| {
        add_derived_fixed(
            f,
            "quick",
            json!({"type": "divide", "value": {"left": stat_ref("pace"), "right": literal_fixed(0)}}),
        );
    });
    assert_eq!(error, CombatError::InvalidSpeed { placement: pl(111) });

    // A derived stat naming an Int stat or a missing stat fails; so does an
    // Int literal. Nothing panics.
    for formula in [
        json!({"type": "add", "value": {"left": stat_ref("alpha"), "right": literal_fixed(1)}}),
        json!({"type": "add", "value": {"left": stat_ref("nowhere"), "right": literal_fixed(1)}}),
        json!({"type": "literal", "value": {"type": "int", "value": 3}}),
    ] {
        let error = register_error(|f| add_derived_fixed(f, "quick", formula.clone()));
        assert_eq!(error, CombatError::InvalidSpeed { placement: pl(111) });
    }

    // A stat the ruleset never declares fails.
    let error = register_error(|f| {
        edit_ruleset(f, |ruleset| {
            ruleset["movement"]["speed"] = json!({"type": "stat", "value": "ghost"});
        });
    });
    assert_eq!(error, CombatError::InvalidSpeed { placement: pl(111) });

    // A derived step resolves as well; min and max, subtract and multiply too.
    let mut fixture = move_fixture();
    edit_ruleset(&mut fixture, |ruleset| {
        ruleset["stats"].as_array_mut().expect("stats").push(json!({
            "name": "reach", "kind": "fixed",
            "derived": {"type": "max", "value": {
                "left": {"type": "min", "value": {"left": stat_ref("pace"), "right": literal_fixed(98304)}},
                "right": {"type": "subtract", "value": {"left": literal_fixed(131072), "right": literal_fixed(65536)}}
            }}
        }));
        ruleset["stats"].as_array_mut().expect("stats").push(json!({
            "name": "double", "kind": "fixed",
            "derived": {"type": "multiply", "value": {"left": stat_ref("reach"), "right": literal_fixed(131072)}}
        }));
        ruleset["movement"]["turn"]["step"] = json!({"type": "stat", "value": "double"});
    });
    let mut world = World::new(7);
    let a = reg(&fixture, &mut world, 111);
    // reach = max(min(65536, 98304), 131072 - 65536) = 65536; double = 131072.
    assert_eq!(mover_state(&world, a).2, 131072);
}

#[test]
fn a5_start_errors() {
    let mut fixture = move_fixture();
    set_creature_stat(&mut fixture, 121, "pace", 0);
    let mut world = World::new(7);
    let before = bytes_of(&world);
    let error = start_into(&fixture, &mut world).expect_err("start fails");
    assert_eq!(error, CombatError::InvalidSpeed { placement: pl(111) });
    assert_eq!(bytes_of(&world), before, "world unchanged");

    // Existing checks win.
    support::rt::edit_ability(&mut fixture, |ability| ability.dice = "not dice".to_owned());
    let mut world = World::new(7);
    assert_eq!(
        start_into(&fixture, &mut world).expect_err("start fails"),
        CombatError::InvalidDice
    );

    // Authored order: the first bad participant names itself.
    let mut fixture = move_fixture();
    set_pos_x(&mut fixture, LIMIT + 1);
    fixture
        .placements
        .get_mut(&pl(112))
        .expect("B")
        .0
        .transform
        .position[0] = Fx16_16::from_raw(LIMIT + 1);
    let mut world = World::new(7);
    assert_eq!(
        start_into(&fixture, &mut world).expect_err("start fails"),
        CombatError::InvalidPosition { placement: pl(111) }
    );
    let mut fixture = move_fixture();
    set_creature_stat(&mut fixture, 122, "pace", 0);
    let mut world = World::new(7);
    assert_eq!(
        start_into(&fixture, &mut world).expect_err("start fails"),
        CombatError::InvalidSpeed { placement: pl(112) }
    );
    let mut fixture = move_fixture();
    set_step_constant(&mut fixture, 0);
    let mut world = World::new(7);
    assert_eq!(
        start_into(&fixture, &mut world).expect_err("start fails"),
        CombatError::InvalidStep { placement: pl(111) }
    );
}

#[test]
fn a5_entering_keeps_its_mover() {
    let fixture = move_fixture();
    let mut world = World::new(7);
    let a = reg(&fixture, &mut world, 111);
    world.bind_nav(rev());
    world.place(a, p(5, 5)).expect("places");
    start_into(&fixture, &mut world).expect("starts");
    assert!(world.combatants().contains(a), "A entered");
    assert_eq!(mover_state(&world, a).0, p(5, 5));
    let b = world
        .combatants()
        .iter()
        .map(|(id, _)| id)
        .find(|id| *id != a)
        .expect("B was spawned");
    assert_eq!(mover_state(&world, b).0, p(10, 0));
    assert_eq!(world.movers().len(), 2);
}

// ---------------------------------------------------------------------------
// A-6
// ---------------------------------------------------------------------------

#[test]
fn a6_bind_nav() {
    let mut world = World::new(7);
    assert_eq!(world.nav(), None);
    assert_eq!(world.bind_nav(rev()), vec![]);
    assert_eq!(world.nav(), Some(rev()));
    let bytes = bytes_of(&world);
    assert_eq!(world.bind_nav(rev()), vec![]);
    assert_eq!(bytes_of(&world), bytes, "rebinding changes nothing");

    let (_, mut world, a, b) = registered();
    world.bind_nav(rev());
    assert_eq!(
        world.set_route(b, route(2, vec![p(10, 0), p(10, 2)])),
        Ok(None)
    );
    assert_eq!(
        world.set_route(a, route(1, vec![p(0, 0), p(3, 4)])),
        Ok(None)
    );
    poison_velocity(&mut world, a);
    let positions = (mover_state(&world, a).0, mover_state(&world, b).0);
    let events = value_of(&world)["events"].clone();
    let rng = value_of(&world)["rng"].clone();
    let tick = world.tick();
    assert_eq!(
        world.bind_nav(other()),
        vec![(a, RouteTag(1)), (b, RouteTag(2))]
    );
    assert_eq!(world.nav(), Some(other()));
    assert!(world.movers().get(a).expect("A").route().is_none());
    assert!(world.movers().get(b).expect("B").route().is_none());
    assert_eq!(
        (mover_state(&world, a).0, mover_state(&world, b).0),
        positions
    );
    assert_eq!(
        mirror_of(&world, a).1,
        [0; 3],
        "the cleared mirror is rewritten"
    );
    // No RNG, event or time.
    assert_eq!(value_of(&world)["events"], events);
    assert_eq!(value_of(&world)["rng"], rng);
    assert_eq!(world.tick(), tick);
}

#[test]
fn a6_bind_nav_rewrites_only_cleared_mirrors() {
    let (_, mut world, a, b) = registered();
    world.bind_nav(rev());
    world
        .set_route(a, route(1, vec![p(0, 0), p(3, 4)]))
        .expect("route");
    poison_velocity(&mut world, a);
    poison_velocity(&mut world, b);
    world.bind_nav(other());
    assert_eq!(mirror_of(&world, a).1, [0; 3]);
    assert_eq!(
        mirror_of(&world, b).1,
        [1.0_f32.to_bits(); 3],
        "a mover without a route keeps its mirror"
    );
}

#[test]
fn a6_place() {
    let (_, mut world, a, _) = registered();
    world.bind_nav(rev());
    let gone = despawned(&mut world);
    let thing = plain(&mut world);
    assert_eq!(
        unchanged(&mut world, |w| w.place(gone, p(0, 0))),
        Err(MoveError::AbsentEntity { entity: gone })
    );
    assert_eq!(
        unchanged(&mut world, |w| w.place(thing, p(0, 0))),
        Err(MoveError::NotMover { entity: thing })
    );
    assert_eq!(
        unchanged(&mut world, |w| w.place(a, rp(LIMIT + 1, 0, 0))),
        Err(MoveError::OutOfBounds { index: 0 })
    );
    assert_eq!(
        unchanged(&mut world, |w| w.place(a, rp(0, i32::MIN, 0))),
        Err(MoveError::OutOfBounds { index: 0 })
    );
    world
        .set_route(a, route(1, vec![p(0, 0), p(3, 4)]))
        .expect("route");
    poison_velocity(&mut world, a);
    let events = value_of(&world)["events"].clone();
    assert_eq!(world.place(a, p(2, 3)), Ok(()));
    assert!(world.movers().get(a).expect("A").route().is_none());
    assert_eq!(mover_state(&world, a).0, p(2, 3));
    assert_eq!(
        mirror_of(&world, a),
        ([0x4000_0000, 0, 0x4040_0000], [0; 3])
    );
    assert_eq!(value_of(&world)["events"], events);
    // The limit itself is accepted.
    assert_eq!(world.place(a, rp(LIMIT, -LIMIT, LIMIT)), Ok(()));
}

fn sets(world: &mut World, entity: EntityId, tag: u64) -> Result<Option<RouteTag>, MoveError> {
    world.set_route(entity, route(tag, vec![p(0, 0), p(3, 4)]))
}

#[test]
fn a6_set_route_order_and_dead() {
    let (fixture, mut world, a, b) = lethal_fight();
    world.bind_nav(rev());
    let gone = despawned(&mut world);
    let thing = plain(&mut world);
    assert_eq!(
        unchanged(&mut world, |w| sets(w, gone, 1)),
        Err(MoveError::AbsentEntity { entity: gone })
    );
    assert_eq!(
        unchanged(&mut world, |w| sets(w, thing, 1)),
        Err(MoveError::NotMover { entity: thing })
    );
    // A is dead: DeadCombatant wins over TurnBased.
    assert_eq!(
        unchanged(&mut world, |w| sets(w, a, 1)),
        Err(MoveError::DeadCombatant { entity: a })
    );
    // B is alive in the terminal turn-based encounter.
    assert_eq!(
        unchanged(&mut world, |w| sets(w, b, 1)),
        Err(MoveError::TurnBased { entity: b })
    );
    // C is a non-participant and may set a route during the encounter.
    let c = reg(&fixture, &mut world, 113);
    assert_eq!(
        unchanged(&mut world, |w| {
            w.set_route(c, route(1, vec![p(1, 1), p(3, 4)]))
        }),
        Err(MoveError::StartMismatch)
    );
    assert_eq!(
        world.set_route(c, route(3, vec![p(0, 10), p(0, 12)])),
        Ok(None)
    );
    let active = world.movers().get(c).expect("C").route().expect("route");
    assert_eq!(
        (active.segment(), active.along(), active.tag()),
        (0, 0, RouteTag(3))
    );
}

#[test]
fn a6_set_route_in_a_live_turn_based_encounter() {
    let fixture = move_fixture();
    let (mut world, ids) = fixture.start(7);
    world.bind_nav(rev());
    for id in &ids {
        let at = world.movers().get(*id).expect("mover").position();
        let result = unchanged(&mut world, |w| {
            w.set_route(*id, route(1, vec![at, p(20, 20)]))
        });
        assert_eq!(result, Err(MoveError::TurnBased { entity: *id }));
    }
}

#[test]
fn a6_set_route_and_clear_route() {
    let (_, mut world, a, _) = registered();
    assert_eq!(
        unchanged(&mut world, |w| sets(w, a, 1)),
        Err(MoveError::NavUnbound)
    );
    world.bind_nav(rev());
    assert_eq!(
        unchanged(&mut world, |w| w.set_route(a, route(1, vec![p(0, 0)]))),
        Err(MoveError::RouteSize { points: 1 })
    );
    poison_velocity(&mut world, a);
    let events = value_of(&world)["events"].clone();
    let rng = value_of(&world)["rng"].clone();
    assert_eq!(sets(&mut world, a, 1), Ok(None));
    let active = world.movers().get(a).expect("A").route().expect("route");
    assert_eq!((active.segment(), active.along()), (0, 0));
    assert_eq!(active.route().points, vec![p(0, 0), p(3, 4)]);
    assert_eq!(
        mover_state(&world, a).0,
        p(0, 0),
        "no movement without a tick"
    );
    assert_eq!(mirror_of(&world, a).1, [0; 3]);
    assert_eq!(
        sets(&mut world, a, 2),
        Ok(Some(RouteTag(1))),
        "a replacement returns the replaced tag"
    );
    assert_eq!(value_of(&world)["events"], events);
    assert_eq!(value_of(&world)["rng"], rng);

    let gone = despawned(&mut world);
    let thing = plain(&mut world);
    assert_eq!(
        unchanged(&mut world, |w| w.clear_route(gone)),
        Err(MoveError::AbsentEntity { entity: gone })
    );
    assert_eq!(
        unchanged(&mut world, |w| w.clear_route(thing)),
        Err(MoveError::NotMover { entity: thing })
    );
    poison_velocity(&mut world, a);
    assert_eq!(world.clear_route(a), Ok(Some(RouteTag(2))));
    assert_eq!(mirror_of(&world, a).1, [0; 3]);
    assert_eq!(world.clear_route(a), Ok(None));
    assert!(world.movers().get(a).expect("A").route().is_none());
}

#[test]
fn a6_real_time_combatants_may_set_routes() {
    let fixture = move_rt_fixture();
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    world.bind_nav(rev());
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(sets(&mut world, a, 1), Ok(None));
    assert_eq!(
        world.set_route(b, route(2, vec![p(10, 0), p(10, 2)])),
        Ok(None)
    );
    // A pending switch to turn-based does not forbid it.
    request_mode(&mut world, EncounterMode::TurnBased).expect("a pending switch");
    assert!(world.combat().expect("encounter").pending_mode.is_some());
    assert_eq!(sets(&mut world, a, 3), Ok(Some(RouteTag(1))));
    assert_eq!(world.clear_route(b), Ok(Some(RouteTag(2))));
}

#[test]
fn a6_dead_record_cannot_walk() {
    let mut fixture = move_fixture();
    make_lethal(&mut fixture, 99);
    let (mut world, ids) = fixture.start(7);
    let (a, b) = (ids[0], ids[1]);
    perform_action(&mut world, &use_action(b, fixture.ability, a)).expect("B kills A");
    end_encounter(&mut world).expect("the encounter ends");
    assert!(world.combat().is_none());
    assert!(world.creatures().get(a).expect("A's record").dead());
    assert!(world.movers().contains(a), "the corpse keeps its mover");
    world.bind_nav(rev());
    assert_eq!(
        unchanged(&mut world, |w| sets(w, a, 1)),
        Err(MoveError::DeadCombatant { entity: a })
    );
    assert!(!world.creatures().get(b).expect("B's record").dead());
    assert_eq!(
        world.set_route(b, route(2, vec![p(10, 0), p(10, 2)])),
        Ok(None)
    );
}

#[test]
fn a6_store_order_is_not_id_order() {
    let fixture = move_fixture();
    let mut world = World::new(7);
    let a = reg(&fixture, &mut world, 111);
    let b = reg(&fixture, &mut world, 112);
    assert_eq!((a, b), {
        let ids: Vec<EntityId> = world.ids().collect();
        (ids[0], ids[1])
    });
    assert!(world.despawn(a));
    let c = reg(&fixture, &mut world, 113);
    let stored: Vec<EntityId> = world.movers().iter().map(|(id, _)| id).collect();
    assert_eq!(stored, vec![b, c], "C is stored after B");
    assert!(c < b, "C reuses index 0 and sorts before B");
    world.bind_nav(rev());
    assert_eq!(
        world.set_route(c, route(3, vec![p(0, 10), p(0, 12)])),
        Ok(None)
    );
    assert_eq!(
        world.set_route(b, route(2, vec![p(10, 0), p(10, 2)])),
        Ok(None)
    );
    assert_eq!(
        world.bind_nav(other()),
        vec![(c, RouteTag(3)), (b, RouteTag(2))]
    );
}

#[test]
fn a6_despawn_removes_the_mover() {
    let (_, mut world, a, b) = registered();
    assert!(world.despawn(a));
    assert!(!world.movers().contains(a));
    assert!(!world.transforms().contains(a));
    assert!(world.movers().contains(b));
}

#[test]
fn a6_history_wrappers() {
    let fixture = move_fixture();
    let mut history = HistoryWorld::new(7);
    let a = fixture
        .register_history(&mut history, pl(111))
        .expect("A registers");
    let b = fixture
        .register_history(&mut history, pl(112))
        .expect("B registers");
    let gone = history.spawn(EntityMeta {}).expect("spawn");
    history.despawn(gone).expect("despawn");
    let thing = history.spawn(EntityMeta {}).expect("spawn");
    let counters = |h: &HistoryWorld| (h.pending_len(), h.last_sequence(), h.version());
    let before = counters(&history);
    assert_eq!(before.2, 1);

    assert_eq!(history.bind_nav(rev()), Ok(vec![]));
    assert_eq!(history.world().nav(), Some(rev()));
    assert_eq!(history.bind_nav(rev()), Ok(vec![]));
    assert_eq!(
        history.set_route(a, route(1, vec![p(0, 0), p(3, 4)])),
        Ok(None)
    );
    assert_eq!(
        history.set_route(b, route(2, vec![p(10, 0), p(10, 2)])),
        Ok(None)
    );
    assert_eq!(
        history.set_route(a, route(5, vec![p(0, 0), p(3, 4)])),
        Ok(Some(RouteTag(1)))
    );
    assert_eq!(history.clear_route(b), Ok(Some(RouteTag(2))));
    assert_eq!(history.clear_route(b), Ok(None));
    assert_eq!(history.place(b, p(1, 1)), Ok(()));
    assert_eq!(history.place(b, p(10, 0)), Ok(()));
    assert_eq!(
        history.set_route(b, route(6, vec![p(10, 0), p(10, 2)])),
        Ok(None)
    );
    assert_eq!(
        history.bind_nav(other()),
        Ok(vec![(a, RouteTag(5)), (b, RouteTag(6))])
    );
    assert_eq!(counters(&history), before, "nothing journaled");

    let bytes = serde_json::to_vec(&history).expect("serializes");
    let failures: Vec<(Result<(), HistoryError>, MoveError)> = vec![
        (
            history.place(gone, p(0, 0)),
            MoveError::AbsentEntity { entity: gone },
        ),
        (
            history.place(thing, p(0, 0)),
            MoveError::NotMover { entity: thing },
        ),
        (
            history.place(a, rp(LIMIT + 1, 0, 0)),
            MoveError::OutOfBounds { index: 0 },
        ),
        (
            history
                .set_route(a, route(1, vec![p(0, 0), p(3, 4)]))
                .map(|_| ()),
            MoveError::StaleRevision,
        ),
        (
            history.clear_route(gone).map(|_| ()),
            MoveError::AbsentEntity { entity: gone },
        ),
        (
            history.clear_route(thing).map(|_| ()),
            MoveError::NotMover { entity: thing },
        ),
    ];
    for (result, expected) in failures {
        assert_eq!(result, Err(HistoryError::Move(expected)));
    }
    assert_eq!(
        serde_json::to_vec(&history).expect("serializes"),
        bytes,
        "failures change nothing"
    );
    assert_eq!(counters(&history), before);
}

#[test]
fn a6_history_wrapper_in_an_rt_encounter_keeps_version_two() {
    let fixture = move_rt_fixture();
    let (mut history, ids) = fixture
        .start_history(7, EncounterMode::RealTime)
        .expect("starts");
    assert_eq!(history.version(), 2);
    history.bind_nav(rev()).expect("binds");
    let before = (history.pending_len(), history.last_sequence());
    history
        .set_route(ids[0], route(1, vec![p(0, 0), p(3, 4)]))
        .expect("an RT combatant may set a route");
    assert_eq!((history.pending_len(), history.last_sequence()), before);
    assert_eq!(history.version(), 2);
}

// ---------------------------------------------------------------------------
// A-7
// ---------------------------------------------------------------------------

#[test]
fn a7_transfer_copies_the_mover_without_its_route() {
    let fixture = move_fixture();
    let area_y = Ulid::from_u128(132);
    let mut source = World::new_in_area(1, fixture.area);
    let mut destination = World::new_in_area(2, area_y);
    let a = reg(&fixture, &mut source, 111);
    source.bind_nav(rev());
    source.place(a, p(2, 3)).expect("places");
    sets_from(&mut source, a, 4, p(2, 3));
    poison_velocity(&mut source, a);
    let moved = transfer_entity(&mut source, &mut destination, a).expect("transfers");
    assert!(source.movers().get(a).is_none());
    assert_eq!(
        mover_state(&destination, moved),
        (p(2, 3), 65536, 98304, None)
    );
    assert_eq!(
        mirror_of(&destination, moved),
        ([0x4000_0000, 0, 0x4040_0000], [0; 3])
    );
    assert_eq!(destination.nav(), None, "the destination's nav is not read");
}

fn sets_from(world: &mut World, entity: EntityId, tag: u64, start: Point) {
    world
        .set_route(entity, route(tag, vec![start, p(30, 30)]))
        .expect("route");
}

#[test]
fn a7_history_transfer_copies_the_mover_without_its_route() {
    let fixture = move_fixture();
    let area_y = Ulid::from_u128(132);
    let mut source = HistoryWorld::new_in_area(1, fixture.area);
    let mut destination = HistoryWorld::new_in_area(2, area_y);
    let a = fixture
        .register_history(&mut source, pl(111))
        .expect("registers");
    source.bind_nav(rev()).expect("binds");
    source.place(a, p(2, 3)).expect("places");
    source
        .set_route(a, route(4, vec![p(2, 3), p(30, 30)]))
        .expect("route");
    let moved = transfer_history_entity(&mut source, &mut destination, a).expect("transfers");
    assert!(source.world().movers().get(a).is_none());
    assert_eq!(
        mover_state(destination.world(), moved),
        (p(2, 3), 65536, 98304, None)
    );
    assert_eq!(
        mirror_of(destination.world(), moved),
        ([0x4000_0000, 0, 0x4040_0000], [0; 3])
    );
    assert_eq!(destination.version(), 1);
}

#[test]
fn a7_leave_and_end_leave_movers_alone() {
    let fixture = move_rt_fixture();
    let (mut world, ids) = fixture.start_rt(7, EncounterMode::RealTime);
    world.bind_nav(rev());
    let (a, b) = (ids[0], ids[1]);
    sets(&mut world, a, 1).expect("route");
    let a_before = world.movers().get(a).expect("A").clone();
    let b_before = world.movers().get(b).expect("B").clone();
    let mirror_before = mirror_of(&world, a);
    leave_encounter(&mut world, a).expect("A leaves");
    assert_eq!(world.movers().get(a), Some(&a_before));
    assert_eq!(mirror_of(&world, a), mirror_before);
    end_encounter(&mut world).expect("the encounter ends");
    assert_eq!(world.movers().get(a), Some(&a_before));
    assert_eq!(world.movers().get(b), Some(&b_before));
}

// ---------------------------------------------------------------------------
// A-8
// ---------------------------------------------------------------------------

fn base_world() -> World {
    let fixture = move_fixture();
    let mut world = World::new(7);
    let a = reg(&fixture, &mut world, 111);
    world.bind_nav(rev());
    world
        .set_route(a, route(7, vec![p(0, 0), p(3, 4)]))
        .expect("route");
    world
}

fn load_error(edit: impl FnOnce(&mut Value)) -> String {
    match try_with_json(&base_world(), edit) {
        Ok(_) => panic!("the edited save must not load"),
        Err(message) => message,
    }
}

#[test]
fn a8_load_coherence() {
    let base = base_world();
    let loaded: World =
        serde_json::from_slice(&bytes_of(&base)).expect("the base world round-trips");
    assert_eq!(loaded, base);
    assert_eq!(bytes_of(&loaded), bytes_of(&base));

    // L1: a mover for a non-live entity.
    let ghost: EntityId =
        serde_json::from_value(json!({"index": 99, "generation": 1})).expect("an id");
    assert_eq!(
        load_error(|save| save["movers"][0][0]["index"] = json!(99)),
        format!("dangling mover for non-live {ghost:?}")
    );
    // L2.
    for speed in [0, 262145, -4] {
        assert_eq!(
            load_error(|save| save["movers"][0][1]["speed"] = json!(speed)),
            "mover speed outside its limits",
            "speed {speed}"
        );
    }
    // L3.
    for step in [0, -1] {
        assert_eq!(
            load_error(|save| save["movers"][0][1]["step"] = json!(step)),
            "mover step is not positive"
        );
    }
    // L4.
    assert_eq!(
        load_error(|save| save["movers"][0][1]["position"][0] = json!(536870913)),
        "mover position outside the coordinate limit"
    );
    // L5.
    assert_eq!(
        load_error(|save| save["transforms"] = json!([])),
        "mover without a transform"
    );
    // L6.
    assert_eq!(
        load_error(|save| save["transforms"][0][1]["position"][0] = json!(1.0_f32)),
        "mover transform disagrees with its position"
    );
    // L7.
    assert_eq!(
        load_error(|save| {
            save.as_object_mut().expect("object").remove("nav");
        }),
        "mover route without a bound navmesh"
    );
    // L8.
    assert_eq!(
        load_error(|save| save["nav"] = serde_json::to_value(other()).expect("nav")),
        "mover route revision differs from the bound navmesh"
    );
    // L9.
    assert_eq!(
        load_error(|save| save["movers"][0][1]["route"]["route"]["points"] = json!([[0, 0, 0]])),
        "mover route fails validation"
    );
    // L10.
    assert_eq!(
        load_error(|save| save["movers"][0][1]["route"]["segment"] = json!(1)),
        "mover route segment out of range"
    );
    // L11.
    assert_eq!(
        load_error(|save| save["movers"][0][1]["route"]["along"] = json!(327680)),
        "mover route progress out of range"
    );
    // L12.
    assert_eq!(
        load_error(|save| save["movers"][0][1]["route"]["along"] = json!(65536)),
        "mover position disagrees with its route"
    );
    // Control: a consistent mid-route state loads.
    let loaded = try_with_json(&base_world(), |save| {
        save["movers"][0][1]["route"]["along"] = json!(65536);
        save["movers"][0][1]["position"] = json!([39321, 0, 52428]);
        save["transforms"][0][1]["position"] = json!([
            to_f32(Fx16_16::from_raw(39321)),
            0.0_f32,
            to_f32(Fx16_16::from_raw(52428))
        ]);
    })
    .expect("a consistent mid-route state loads");
    let a = loaded.movers().iter().next().expect("a mover").0;
    let active = loaded.movers().get(a).expect("A").route().expect("route");
    assert_eq!(active.along(), 65536);
}

#[test]
fn a8_rules_run_in_order() {
    // A speed and a step failure together: L2 wins.
    assert_eq!(
        load_error(|save| {
            save["movers"][0][1]["speed"] = json!(0);
            save["movers"][0][1]["step"] = json!(0);
        }),
        "mover speed outside its limits"
    );
    // A bad position and a missing transform: L4 wins over L5.
    assert_eq!(
        load_error(|save| {
            save["movers"][0][1]["position"][0] = json!(536870913);
            save["transforms"] = json!([]);
        }),
        "mover position outside the coordinate limit"
    );
    // A missing nav and a bad segment: L7 wins over L10.
    assert_eq!(
        load_error(|save| {
            save.as_object_mut().expect("object").remove("nav");
            save["movers"][0][1]["route"]["segment"] = json!(1);
        }),
        "mover route without a bound navmesh"
    );
    // A mover with no route needs no nav.
    let (_, world, _, _) = registered();
    assert!(try_with_json(&world, |_| {}).is_ok());
    // The mover checks run before the combat checks.
    let fixture = move_fixture();
    let (started, _) = fixture.start(7);
    let error = try_with_json(&started, |save| {
        save["movers"][0][1]["speed"] = json!(0);
        save["combat"]["round"] = json!(0);
        save["combatants"] = json!([]);
    })
    .expect_err("fails");
    assert_eq!(error, "mover speed outside its limits");
}

// ---------------------------------------------------------------------------
// A-9, A-10
// ---------------------------------------------------------------------------

#[test]
fn a9_byte_identity() {
    let world = World::new(7);
    assert_eq!(
        top_level_keys(&bytes_of(&world)),
        [
            "entities",
            "transforms",
            "timeline",
            "events",
            "rng",
            "tick"
        ]
    );
    let (started, _) = Fixture::standard().start(7);
    let keys = json_keys(&value_of(&started));
    for key in ["movers", "nav", "speed", "step", "segment"] {
        assert!(!keys.contains(key), "no `{key}` key in a mover-free world");
    }

    let fixture = move_fixture();
    let mut world = World::new(7);
    reg(&fixture, &mut world, 111);
    assert_eq!(
        top_level_keys(&bytes_of(&world)).last().map(String::as_str),
        Some("movers")
    );
    world.bind_nav(rev());
    let keys = top_level_keys(&bytes_of(&world));
    assert_eq!(&keys[keys.len() - 2..], ["movers", "nav"]);

    let mut world = World::new(7);
    world.bind_nav(rev());
    assert_eq!(
        top_level_keys(&bytes_of(&world)).last().map(String::as_str),
        Some("nav")
    );
    // A routeless mover writes neither `route` nor `segment`.
    let (_, world, _, _) = registered();
    let keys = json_keys(&value_of(&world));
    assert!(keys.contains("speed") && keys.contains("step"));
    assert!(!keys.contains("route") && !keys.contains("segment"));
}

#[test]
fn a10_display_texts() {
    let mut world = World::new(0);
    let entity = world.spawn(EntityMeta {});
    let shown = format!("{entity:?}");
    let table: Vec<(MoveError, String)> = vec![
        (
            MoveError::AbsentEntity { entity },
            format!("AbsentEntity at movers/{shown}"),
        ),
        (
            MoveError::NotMover { entity },
            format!("NotMover at movers/{shown}"),
        ),
        (
            MoveError::DeadCombatant { entity },
            format!("DeadCombatant at movers/{shown}"),
        ),
        (
            MoveError::TurnBased { entity },
            format!("TurnBased at movers/{shown}"),
        ),
        (
            MoveError::RouteSize { points: 1 },
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
                current: 2,
            },
            "CannotAfford at combat/pool".to_owned(),
        ),
        (
            MoveError::Combat(CombatError::NoEncounter),
            "NoEncounter at combat".to_owned(),
        ),
    ];
    for (error, text) in &table {
        assert_eq!(&error.to_string(), text);
        match error {
            MoveError::Combat(_) => {
                let source = error.source().expect("Combat has a source");
                assert_eq!(source.to_string(), "NoEncounter at combat");
            }
            _ => assert!(error.source().is_none(), "{error:?} has no source"),
        }
    }
    let placement = Ulid::from_u128(1);
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
    for inner in [
        MoveError::NavUnbound,
        MoveError::Combat(CombatError::NoEncounter),
    ] {
        let wrapped = HistoryError::Move(inner.clone());
        assert_eq!(wrapped.to_string(), inner.to_string());
        assert_eq!(
            wrapped.source().expect("a source").to_string(),
            inner.to_string()
        );
    }
    assert_eq!(
        HistoryError::Move(MoveError::NavUnbound).to_string(),
        "NavUnbound at nav"
    );
    assert_eq!(
        HistoryError::Move(MoveError::Combat(CombatError::NoEncounter)).to_string(),
        "NoEncounter at combat"
    );
}

#[test]
fn route_and_nav_serialize_as_derived_serde() {
    let value = serde_json::to_value(rev()).expect("serializes");
    assert_eq!(value.as_array().expect("an array").len(), 32);
    let back: NavRevision = serde_json::from_value(value).expect("parses");
    assert_eq!(back, rev());
    let text = serde_json::to_string(&route(3, vec![p(0, 0), p(3, 4)])).expect("serializes");
    assert!(text.contains("\"tag\":3"));
    let back: MoveRoute = serde_json::from_str(&text).expect("parses");
    assert_eq!(back, route(3, vec![p(0, 0), p(3, 4)]));
    assert!(serde_json::from_str::<MoveRoute>(
        "{\"revision\":[0],\"tag\":1,\"points\":[],\"extra\":1}"
    )
    .is_err());
    let _ = fx_raw(1);
}
