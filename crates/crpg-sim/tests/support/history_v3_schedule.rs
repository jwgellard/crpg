//! Shared sim-local history v3 golden schedule (T036c, ADR-0036 W4).
//!
//! Compiled by `tests/history_v3_golden.rs` and by
//! `examples/generate_history_v3_golden.rs`, both via `#[path]`, so the
//! authoring example and the verification test can never drift apart (the T020
//! pattern). Every includer declares `mod support;` and
//! `mod movement_schedule;` at its crate root: the schedule runs on the MOVE
//! fixture of `support/movement_schedule.rs`.
//!
//! The schedule is pinned (seed 42, area 131, MOVE, turn-based): changing any
//! step, seed, identity, stat or route requires golden review, because every
//! target-scoped golden file records the resulting hashes.

#![allow(dead_code)]

use crpg_core::EntityId;
use crpg_sim::{
    history_hash, CombatAction, EncounterMode, HistoryEnvelope, HistoryWorld, MoveOutcome,
    MoveRoute, NavRevision, RouteTag,
};

use crate::movement_schedule::{move_fixture, p, pl, Point};

/// The pinned golden seed.
pub const GOLDEN_SEED: u64 = 42;

/// The number of hashes the schedule records: step zero through step ten.
pub const GOLDEN_STEPS: usize = 11;

/// The navmesh revision the schedule binds.
pub fn golden_revision() -> NavRevision {
    NavRevision::of_bytes(b"history-v3")
}

/// A route on the schedule's revision.
fn golden_route(tag: u64, points: Vec<Point>) -> MoveRoute {
    MoveRoute {
        revision: golden_revision(),
        tag: RouteTag(tag),
        points,
    }
}

/// Everything the golden schedule observed.
pub struct GoldenRun {
    /// `history_hash` after each step, step zero first.
    pub hashes: Vec<[u8; 32]>,
    /// The wrapper version after each step.
    pub versions: Vec<u32>,
    /// C's position after each step.
    pub c_positions: Vec<Point>,
    /// The movement pool (106) of B after step 4.
    pub b_pool_after_move: u32,
    /// The movement pool (106) of A after step 7.
    pub a_pool_after_move: u32,
    /// The outcomes of B's move (step 4) and A's move (step 7).
    pub outcomes: [MoveOutcome; 2],
    /// The full journal after the last step.
    pub journal: Vec<HistoryEnvelope>,
    /// The final wrapper.
    pub world: HistoryWorld,
    /// The entities `[A, B, C]`.
    pub ids: [EntityId; 3],
}

/// The balance of the movement pool 106 of one combatant.
fn pool_106(world: &HistoryWorld, entity: EntityId) -> u32 {
    let combatant = world.world().combatants().get(entity).expect("a combatant");
    combatant
        .extra_pools()
        .iter()
        .find(|pool| pool.id().0 == pl(106))
        .expect("the movement pool")
        .current()
}

/// Runs the pinned schedule: seed 42, MOVE, `HistoryWorld::new_in_area(42, 131)`.
///
/// Step 0 registers C; 1 binds the navmesh; 2 sets C's route (tag 9); 3 starts
/// the turn-based encounter; 4 B moves (cost 4); 5 ticks; 6 ends B's turn; 7 A
/// moves (cost 2); 8-10 tick.
pub fn run_golden_schedule() -> GoldenRun {
    let fixture = move_fixture();
    let mut world = HistoryWorld::new_in_area(GOLDEN_SEED, fixture.area);
    let mut hashes = Vec::with_capacity(GOLDEN_STEPS);
    let mut versions = Vec::with_capacity(GOLDEN_STEPS);
    let mut c_positions = Vec::with_capacity(GOLDEN_STEPS);

    // Step 0: C registers (Spawned C).
    let c = fixture
        .register_history(&mut world, pl(113))
        .expect("step 0 registers C");
    let mut record = |world: &HistoryWorld| {
        hashes.push(history_hash(world));
        versions.push(world.version());
        let at = world.world().movers().get(c).expect("C's mover").position();
        c_positions.push(at);
    };
    record(&world);

    // Step 1.
    world.bind_nav(golden_revision()).expect("step 1 binds");
    record(&world);

    // Step 2.
    world
        .set_route(c, golden_route(9, vec![p(0, 10), p(4, 10)]))
        .expect("step 2 routes C");
    record(&world);

    // Step 3: a turn-based start (Spawned A, Spawned B, TurnStarted B).
    fixture
        .with_spec(|spec| world.start_encounter_in_mode(spec, EncounterMode::TurnBased))
        .expect("step 3 starts");
    let combatants: Vec<EntityId> = world
        .world()
        .combatants()
        .iter()
        .map(|(id, _)| id)
        .collect();
    let (a, b) = (combatants[0], combatants[1]);
    record(&world);

    // Step 4: B moves 3 m + 3 m = 6 m = 4 steps of 1.5 m (cost 4).
    let first = world
        .perform_move(b, &golden_route(1, vec![p(10, 0), p(10, 3), p(7, 3)]))
        .expect("step 4 moves B");
    let b_pool_after_move = pool_106(&world, b);
    record(&world);

    // Step 5: a tick.
    world.tick().expect("step 5 ticks");
    record(&world);

    // Step 6: B ends its turn; A's turn starts.
    world
        .perform_action(&CombatAction::EndTurn { actor: b })
        .expect("step 6 ends B's turn");
    record(&world);

    // Step 7: A moves 3 m = 2 steps (cost 2).
    let second = world
        .perform_move(a, &golden_route(2, vec![p(0, 0), p(3, 0)]))
        .expect("step 7 moves A");
    let a_pool_after_move = pool_106(&world, a);
    record(&world);

    // Steps 8-10: ticks.
    for step in 8..=10 {
        world
            .tick()
            .unwrap_or_else(|error| panic!("step {step} ticks: {error}"));
        record(&world);
    }

    let journal = world.read_after(0, 256).expect("the journal fits");
    GoldenRun {
        hashes,
        versions,
        c_positions,
        b_pool_after_move,
        a_pool_after_move,
        outcomes: [first, second],
        journal,
        world,
        ids: [a, b, c],
    }
}
