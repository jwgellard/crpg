//! Shared sim-local movement golden schedule and MOVE fixture (T036b,
//! ADR-0036 W1-W3).
//!
//! Compiled by `tests/movement_golden.rs`, by `tests/movement_tick.rs` (for the
//! MOVE fixture) and by `examples/generate_movement_golden.rs`, all via
//! `#[path]`, so the authoring example and the verification test can never
//! drift apart (the T020 pattern). Every includer declares `mod support;` at its
//! crate root, because the fixture starts from `support::Fixture::standard()`.
//!
//! The schedule is pinned (seed 42, area 131, MOVE_RT): changing any step,
//! seed, identity, stat or route requires golden review, because every
//! target-scoped golden file records the resulting hashes.

#![allow(dead_code)]

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_sim::{
    request_mode, start_encounter_in_mode, state_hash, tick_with_movement, EncounterMode,
    MoveRoute, MovementFact, NavRevision, RouteTag, World,
};
use serde_json::{json, Value};

use crate::support::{uid, Fixture};

/// A position `[x, y, z]`.
pub type Point = [Fx16_16; 3];

/// The pinned golden seed.
pub const GOLDEN_SEED: u64 = 42;

/// The number of hashes the schedule records: step zero through step fifteen.
pub const GOLDEN_STEPS: usize = 16;

/// The number of ticks the schedule runs.
pub const GOLDEN_TICKS: usize = 12;

/// The navmesh revision the schedule binds.
pub fn golden_revision() -> NavRevision {
    NavRevision::of_bytes(b"movement-v1")
}

/// `p(x, z)`: whole metres on the ground plane.
pub fn p(x: i32, z: i32) -> Point {
    [Fx16_16::from_int(x), Fx16_16::ZERO, Fx16_16::from_int(z)]
}

/// `rp(x, y, z)`: the point with those raw values.
pub fn rp(x: i32, y: i32, z: i32) -> Point {
    [
        Fx16_16::from_raw(x),
        Fx16_16::from_raw(y),
        Fx16_16::from_raw(z),
    ]
}

/// The bound navmesh of the unit tests.
pub fn rev() -> NavRevision {
    NavRevision::of_bytes(b"mesh-a")
}

/// A different navmesh revision.
pub fn other() -> NavRevision {
    NavRevision::of_bytes(b"mesh-b")
}

/// A route on `rev()`.
pub fn route(tag: u64, points: Vec<Point>) -> MoveRoute {
    MoveRoute {
        revision: rev(),
        tag: RouteTag(tag),
        points,
    }
}

/// The ULID of fixture identity `n`.
pub fn pl(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

/// The three f32 components as raw bit patterns.
pub fn bits(values: [f32; 3]) -> [u32; 3] {
    [
        values[0].to_bits(),
        values[1].to_bits(),
        values[2].to_bits(),
    ]
}

/// Edits the ruleset document through JSON.
pub fn edit_ruleset(fixture: &mut Fixture, edit: impl FnOnce(&mut Value)) {
    let mut value = serde_json::to_value(&fixture.ruleset).expect("the ruleset serializes");
    edit(&mut value);
    fixture.ruleset = serde_json::from_value(value).expect("the edited ruleset parses");
}

/// Sets one base `Fixed` stat of a creature to a raw value.
pub fn set_creature_stat(fixture: &mut Fixture, creature: u128, name: &str, raw: i32) {
    fixture
        .creatures
        .get_mut(&pl(creature))
        .expect("the creature exists")
        .stats
        .insert(name.to_owned(), Fx16_16::from_raw(raw));
}

/// The MOVE fixture of the contract: the standard fixture with movement rules,
/// a movement pool, paces for A (1 m per tick) and B (0.5 m per tick), B at
/// `p(10, 0)`, and creature C at `p(0, 10)` (placement 113, not a participant).
pub fn move_fixture() -> Fixture {
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
    for stat in ["alpha", "beta", "gamma"] {
        creature_c
            .stats
            .insert(stat.to_owned(), Fx16_16::from_int(5));
    }
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
pub fn move_rt_fixture() -> Fixture {
    let mut fixture = move_fixture();
    fixture.ruleset.round_ticks = Some(4);
    fixture
}

/// Everything the golden schedule observed.
pub struct GoldenRun {
    /// The world after the last step.
    pub world: World,
    /// One `state_hash` per step, step zero first.
    pub hashes: Vec<[u8; 32]>,
    /// The entities `[A, B, C]`.
    pub ids: [EntityId; 3],
    /// The positions of `[A, B, C]` after tick `k` at index `k - 1`.
    pub positions: Vec<[Point; 3]>,
    /// The route tags of `[A, B, C]` after tick `k` at index `k - 1`.
    pub routes: Vec<[Option<u64>; 3]>,
    /// The facts of tick `k` at index `k - 1`.
    pub facts: Vec<Vec<MovementFact>>,
    /// The encounter mode after tick `k` at index `k - 1`.
    pub modes: Vec<EncounterMode>,
}

/// Runs the pinned schedule: seed 42, MOVE_RT, `World::new_in_area(42, 131)`.
///
/// Step 0 registers C and binds the navmesh; step 1 starts the real-time
/// encounter; step 2 sets the three routes; steps 3-7 are ticks 1-5; step 8
/// requests a switch to turn-based; steps 9-15 are ticks 6-12.
pub fn run_golden_schedule() -> GoldenRun {
    let fixture = move_rt_fixture();
    let mut world = World::new_in_area(GOLDEN_SEED, fixture.area);
    let mut hashes = Vec::with_capacity(GOLDEN_STEPS);

    // Step 0.
    let c = fixture.register(&mut world, pl(113)).expect("C registers");
    world.bind_nav(golden_revision());
    hashes.push(state_hash(&world));

    // Step 1.
    fixture
        .with_spec(|spec| start_encounter_in_mode(&mut world, spec, EncounterMode::RealTime))
        .expect("the real-time encounter starts");
    let ids: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
    let (a, b) = (ids[0], ids[1]);
    hashes.push(state_hash(&world));

    // Step 2.
    let revision = golden_revision();
    let golden_route = |tag: u64, points: Vec<Point>| MoveRoute {
        revision,
        tag: RouteTag(tag),
        points,
    };
    world
        .set_route(a, golden_route(1, vec![p(0, 0), p(3, 4)]))
        .expect("A's route");
    world
        .set_route(b, golden_route(2, vec![p(10, 0), p(10, 2), p(8, 2)]))
        .expect("B's route");
    world
        .set_route(c, golden_route(3, vec![p(0, 10), p(12, 10)]))
        .expect("C's route");
    hashes.push(state_hash(&world));

    let mut run = GoldenRun {
        world: World::new(0),
        hashes: Vec::new(),
        ids: [a, b, c],
        positions: Vec::new(),
        routes: Vec::new(),
        facts: Vec::new(),
        modes: Vec::new(),
    };
    let mut tick = |world: &mut World, hashes: &mut Vec<[u8; 32]>| {
        let facts = tick_with_movement(world);
        let at = |entity: EntityId| world.movers().get(entity).expect("a mover").position();
        let tag = |entity: EntityId| {
            world
                .movers()
                .get(entity)
                .expect("a mover")
                .route()
                .map(|active| active.tag().0)
        };
        run.positions.push([at(a), at(b), at(c)]);
        run.routes.push([tag(a), tag(b), tag(c)]);
        run.facts.push(facts);
        run.modes.push(world.combat().expect("an encounter").mode);
        hashes.push(state_hash(world));
    };

    // Steps 3-7: ticks 1-5.
    for _ in 0..5 {
        tick(&mut world, &mut hashes);
    }
    // Step 8.
    request_mode(&mut world, EncounterMode::TurnBased).expect("a pending switch");
    hashes.push(state_hash(&world));
    // Steps 9-15: ticks 6-12.
    for _ in 0..7 {
        tick(&mut world, &mut hashes);
    }

    run.world = world;
    run.hashes = hashes;
    run
}
