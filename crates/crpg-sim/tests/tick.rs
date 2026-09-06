//! Hash-sequence, counter and advance-policy tests for T008a.
//!
//! The backbone assertion first: two runs from the same seed produce
//! identical hash sequences over 10,000 ticks, and different seeds diverge.
//! Everything else here pins the pieces that assertion rests on — the
//! counter, the tick/queue-bytes-in-hash rule, and both advance policies.

use crpg_core::Tick;
use crpg_sim::{end_turn, state_hash, tick, EntityMeta, InitiativeKey, SimEvent, Transform, World};
use proptest::prelude::*;

const LONG_RUN: usize = 10_000;

/// A small but busy world: entities with transforms, a shared-key timeline,
/// touched RNG streams, and queued spawn events still undrained.
fn scripted_world(seed: u64) -> World {
    let mut world = World::new(seed);
    let mut ids = Vec::new();
    for _ in 0..8 {
        ids.push(world.spawn(EntityMeta {}));
    }
    for (i, id) in ids.iter().enumerate() {
        let v = i as f32;
        world.transforms_mut().insert(
            *id,
            Transform {
                position: [v, -v, v / 2.0],
                velocity: [1.0, 0.0, -1.0],
            },
        );
        world
            .timeline_mut()
            .insert(InitiativeKey((i as i32 % 3) - 1), *id);
    }
    for stream in ["combat", "ambient"] {
        let _ = world.rng_mut().stream(stream).next_u32();
    }
    world
}

fn hash_sequence(seed: u64, ticks: usize) -> Vec<[u8; 32]> {
    let mut world = scripted_world(seed);
    (0..ticks)
        .map(|_| {
            tick(&mut world);
            state_hash(&world)
        })
        .collect()
}

#[test]
fn hash_sequence_identity_over_10k_ticks() {
    assert_eq!(
        hash_sequence(0x5EED, LONG_RUN),
        hash_sequence(0x5EED, LONG_RUN)
    );
}

proptest! {
    #[test]
    fn different_seeds_diverge(
        first in any::<u64>(),
        second in any::<u64>(),
    ) {
        prop_assume!(first != second);
        // RNG state is world state, so divergence starts at the first hash.
        prop_assert_ne!(hash_sequence(first, 64), hash_sequence(second, 64));
    }
}

#[test]
fn queue_and_tick_bytes_are_hashed() {
    let mut drained = scripted_world(11);
    let queued = drained.clone();
    drained.events_mut().drain();
    assert_ne!(
        state_hash(&drained),
        state_hash(&queued),
        "draining the queue must move the hash: queue bytes are hashed"
    );
    let before = state_hash(&queued);
    let mut advanced = queued;
    tick(&mut advanced);
    assert_ne!(state_hash(&advanced), before, "ticking must move the hash");
}

#[test]
fn counter_advances_exactly_one_per_tick() {
    let mut world = World::new(13);
    for n in 1..=50u64 {
        tick(&mut world);
        assert_eq!(world.tick(), Tick::new(n));
    }
}

#[test]
fn no_input_ticks_preserve_order() {
    let mut world = scripted_world(17);
    let timeline_before: Vec<_> = world.timeline().iter().collect();
    // The script leaves 8 spawn events queued; ticks must add none.
    let queued = world.events().len();
    for _ in 0..100 {
        tick(&mut world);
    }
    assert_eq!(world.timeline().iter().collect::<Vec<_>>(), timeline_before);
    assert_eq!(world.events().len(), queued);
    // Payloads untouched: the original spawns, in spawn order.
    let ids: Vec<_> = world.ids().collect();
    let drained = world.events_mut().drain();
    assert_eq!(drained.len(), ids.len());
    for (envelope, id) in drained.iter().zip(ids.iter()) {
        assert_eq!(envelope.payload, SimEvent::Spawned { entity: *id });
    }
}

#[test]
fn real_time_ticks_keep_standing_order() {
    let mut world = World::new(19);
    let a = world.spawn(EntityMeta {});
    let b = world.spawn(EntityMeta {});
    let c = world.spawn(EntityMeta {});
    world.timeline_mut().insert(InitiativeKey(30), c);
    world.timeline_mut().insert(InitiativeKey(10), b);
    world.timeline_mut().insert(InitiativeKey(10), a);
    tick(&mut world);
    let order: Vec<_> = world.timeline().iter().collect();
    assert_eq!(
        order,
        vec![
            (InitiativeKey(10), a),
            (InitiativeKey(10), b),
            (InitiativeKey(30), c),
        ]
    );
}

#[test]
fn end_turn_pops_ascending_and_never_despawns() {
    let mut world = World::new(23);
    let a = world.spawn(EntityMeta {});
    let b = world.spawn(EntityMeta {});
    let c = world.spawn(EntityMeta {});
    world.timeline_mut().insert(InitiativeKey(30), c);
    world.timeline_mut().insert(InitiativeKey(10), b);
    world.timeline_mut().insert(InitiativeKey(10), a);
    assert_eq!(end_turn(&mut world), Some((InitiativeKey(10), a)));
    assert_eq!(end_turn(&mut world), Some((InitiativeKey(10), b)));
    assert_eq!(end_turn(&mut world), Some((InitiativeKey(30), c)));
    assert_eq!(end_turn(&mut world), None);
    // Popping schedules; every entity is still live.
    assert!(world.contains(a) && world.contains(b) && world.contains(c));
    assert_eq!(world.len(), 3);
}
