//! Spawn, despawn, ordering and persistence tests for the T007 skeleton.
//!
//! The operation sequences below are small hand-built programs over a `World`
//! plus one 10,000-case property test in the shape spec §24 T7 demands:
//! random spawn/despawn/mutate sequences must round-trip through serde
//! unchanged, leave no dangling references, and replay identically on a
//! second instance.

use crpg_core::{EntityId, Tick};
use crpg_sim::{EntityMeta, InitiativeKey, SimEvent, Transform, World};
use proptest::prelude::*;

const CASES_10K: u32 = 10_000;

/// One step of a generated world program.
#[derive(Debug, Clone)]
enum Op {
    Spawn,
    Despawn(usize),
    SetTransform(usize, [f32; 3]),
    Schedule(usize, i32),
    Draw(usize),
}

fn op_strategy(live_upper_bound: usize) -> impl Strategy<Value = Op> {
    let index = 0..live_upper_bound;
    prop_oneof![
        Just(Op::Spawn),
        index.clone().prop_map(Op::Despawn),
        (index.clone(), -1000.0f32..1000.0f32)
            .prop_map(|(i, v)| { Op::SetTransform(i, [v, -v, v / 2.0]) }),
        (index.clone(), any::<i32>()).prop_map(|(i, k)| Op::Schedule(i, k)),
        index.prop_map(Op::Draw),
    ]
}

fn finite(v: f32) -> f32 {
    // Halving keeps every generated value finite, so PartialEq round-trips
    // cannot meet NaN. Range strategies already exclude NaN and infinities.
    v / 2.0
}

/// Runs `ops` against a fresh world, resolving `usize` operands against the
/// ids spawned so far (modulo the live count, so despawn operands sometimes
/// name dead ids — that is the point).
fn run_ops(seed: u64, ops: &[Op]) -> (World, Vec<EntityId>) {
    let mut world = World::new(seed);
    let mut minted: Vec<EntityId> = Vec::new();
    for op in ops {
        match op {
            Op::Spawn => minted.push(world.spawn(EntityMeta {})),
            Op::Despawn(i) => {
                if !minted.is_empty() {
                    world.despawn(minted[i % minted.len()]);
                }
            }
            Op::SetTransform(i, pos) => {
                if !minted.is_empty() {
                    let id = minted[i % minted.len()];
                    if world.contains(id) {
                        world.transforms_mut().insert(
                            id,
                            Transform {
                                position: *pos,
                                velocity: [finite(pos[0]), 0.0, 0.0],
                            },
                        );
                    }
                }
            }
            Op::Schedule(i, key) => {
                if !minted.is_empty() {
                    let id = minted[i % minted.len()];
                    if world.contains(id) {
                        world.timeline_mut().insert(InitiativeKey(*key), id);
                    }
                }
            }
            Op::Draw(i) => {
                let streams = ["combat", "ambient"];
                let _ = world.rng_mut().stream(streams[i % 2]).next_u32();
            }
        }
    }
    (world, minted)
}

/// Every store key and timeline entry must address a live entity.
fn assert_no_dangling(world: &World) {
    for (id, _) in world.transforms().iter() {
        assert!(world.contains(id), "dangling transform for dead {id:?}");
    }
    for (_, id) in world.timeline().iter() {
        assert!(
            world.contains(id),
            "dangling timeline entry for dead {id:?}"
        );
    }
    assert!(world.transforms().len() <= world.len());
    assert!(world.timeline().len() <= world.len());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES_10K))]

    #[test]
    fn skeleton_round_trip_over_random_op_sequences(
        seed in any::<u64>(),
        ops in prop::collection::vec(op_strategy(8), 0..60),
    ) {
        let (world, _) = run_ops(seed, &ops);
        assert_no_dangling(&world);
        let json = serde_json::to_string(&world).unwrap();
        let loaded: World = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(loaded, world);
    }

    #[test]
    fn identical_sequences_replay_identically(
        seed in any::<u64>(),
        ops in prop::collection::vec(op_strategy(8), 0..60),
    ) {
        let (first, _) = run_ops(seed, &ops);
        let (second, _) = run_ops(seed, &ops);
        let a = serde_json::to_string(&first).unwrap();
        let b = serde_json::to_string(&second).unwrap();
        prop_assert_eq!(a, b);
        prop_assert_eq!(first, second);
    }
}

#[test]
fn despawn_of_dead_id_is_silent() {
    let mut world = World::new(7);
    let live = world.spawn(EntityMeta {});
    // Despawn the id twice: the second call names a dead id.
    assert!(world.despawn(live));
    let events_before = world.events().len();
    assert!(!world.despawn(live));
    assert_eq!(world.events().len(), events_before);
    // And an id this world never minted is equally silent.
    let mut other = World::new(7);
    let foreign = other.spawn(EntityMeta {});
    assert!(!world.despawn(foreign));
    assert_eq!(world.events().len(), events_before);
}

#[test]
fn spawn_and_despawn_enqueue_exactly_one_event_each() {
    let mut world = World::new(1);
    let id = world.spawn(EntityMeta {});
    assert_eq!(world.events().len(), 1);
    assert!(world.despawn(id));
    let drained = world.events_mut().drain();
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].payload, SimEvent::Spawned { entity: id });
    assert_eq!(drained[1].payload, SimEvent::Despawned { entity: id });
    assert_eq!(drained[0].tick, Tick::ZERO);
    assert!(drained[0].seq < drained[1].seq);
}

#[test]
fn despawn_strips_components_and_timeline_entries() {
    let mut world = World::new(3);
    let id = world.spawn(EntityMeta {});
    world.transforms_mut().insert(id, Transform::default());
    world.timeline_mut().insert(InitiativeKey(10), id);
    assert!(world.despawn(id));
    assert!(world.transforms().get(id).is_none());
    assert!(!world.timeline().contains(id));
    assert_no_dangling(&world);
}

#[test]
fn round_trip_preserves_occupancy_after_despawn() {
    // Regression pin for the null-ambiguity trap: EntityMeta is a braced
    // empty struct because a unit struct serializes as `null`, which is also
    // how a vacant arena slot serializes — every occupied slot would load as
    // vacant and the arena guard would reject its own save. A world mixing
    // live and freed slots must round-trip exactly.
    let mut world = World::new(11);
    let keep = world.spawn(EntityMeta {});
    let drop = world.spawn(EntityMeta {});
    assert!(world.despawn(drop));
    let json = serde_json::to_string(&world).unwrap();
    let loaded: World = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded, world);
    assert!(loaded.contains(keep));
    assert!(!loaded.contains(drop));
    // And the loaded arena allocates what the saved one would have.
    let mut probe = world;
    let mut loaded_probe = loaded;
    assert_eq!(
        loaded_probe.spawn(EntityMeta {}),
        probe.spawn(EntityMeta {})
    );
}

#[test]
fn timeline_iterates_ascending_with_id_tiebreak() {
    let mut world = World::new(9);
    let a = world.spawn(EntityMeta {});
    let b = world.spawn(EntityMeta {});
    let c = world.spawn(EntityMeta {});
    // Scrambled insertion: keys out of order, shared key across entities.
    world.timeline_mut().insert(InitiativeKey(30), c);
    world.timeline_mut().insert(InitiativeKey(10), b);
    world.timeline_mut().insert(InitiativeKey(10), a);
    let order: Vec<_> = world.timeline().iter().collect();
    assert_eq!(order.len(), 3);
    assert!(order.windows(2).all(|w| w[0] <= w[1]));
    // Same key sorts by entity: no despawn happened, so a < b by slot order.
    assert_eq!(order[0], (InitiativeKey(10), a));
    assert_eq!(order[1], (InitiativeKey(10), b));
    assert_eq!(order[2], (InitiativeKey(30), c));
}
