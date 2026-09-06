//! The fixed-step loop: [`tick`], [`end_turn`], and the ordered system list.
//!
//! Tick order is a rules-visible decision (spec §10), so it lives here,
//! handwritten — never a scheduler, never parallelism inside one area's tick
//! (spec §2.4 rule 1). Changing this order changes behaviour globally, which
//! is why the crate contract marks it serialise-one-agent-at-a-time
//! territory (spec §15.2): later systems append with their tasks, each below
//! a comment naming its spec §10 stage.

use crpg_core::EntityId;

use crate::timeline::InitiativeKey;
use crate::world::World;

/// One fixed simulation step: the tick counter advances by one (saturating),
/// then the ordered system list runs.
///
/// The counter moves first, so a system observing `world.tick()` sees the
/// tick it runs under. `run_systems` is the whole point of the function —
/// the list, its order, and its stage comments are the contract.
pub fn tick(world: &mut World) {
    world.advance_tick();
    run_systems(world);
}

/// Turn-based advance primitive: pops and returns the head of the timeline
/// in ascending `(key, id)` order, or `None` on an empty timeline.
///
/// Popping is the whole semantic: no re-queueing, no counter advance, no
/// systems run. Who calls it — the combat loop, AI turn start — and what
/// re-queues the acted entity is T016-era design, not this function.
/// Popping schedules; it never despawns, and the popped entity stays live.
pub fn end_turn(world: &mut World) -> Option<(InitiativeKey, EntityId)> {
    let (key, id) = world.timeline().iter().next()?;
    world.timeline_mut().remove(id).then_some((key, id))
}

/// The hand-written ordered system list, spec §10 stage order.
///
/// Today exactly one entry. Each future system appends with its task and its
/// stage comment; reordering existing entries is a behaviour change and gets
/// reviewed as one.
fn run_systems(world: &mut World) {
    // §10 stage 3 (timeline order) — the only stage with a system yet.
    timeline_system(world);
}

/// Real-time policy step: the standing queue is processed in order every
/// tick, and standing order is preserved — no reorder, no rekey.
///
/// The per-entry dispatch loop is already here, applying no effect yet: its
/// body names the task that plugs the first effect in, so the shape future
/// systems inherit is the shape tested today. Net observable effect of a
/// no-input tick: counter +1, everything else bit-identical.
fn timeline_system(world: &mut World) {
    for _entity in world.timeline().iter().map(|(_, id)| id) {
        // No effect yet: the first per-entity system (T014-era stats or
        // T016-era combat) plugs in here.
    }
}
