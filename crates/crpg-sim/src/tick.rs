//! The fixed-step loop: [`tick`], [`end_turn`], and the ordered system list.
//!
//! Tick order is a rules-visible decision (spec §10), so it lives here,
//! handwritten — never a scheduler, never parallelism inside one area's tick
//! (spec §2.4 rule 1). Changing this order changes behaviour globally, which
//! is why the crate contract marks it serialise-one-agent-at-a-time
//! territory (spec §15.2): later systems append with their tasks, each below
//! a comment naming its spec §10 stage.

use crpg_core::EntityId;

use crate::combat::TickFacts;
use crate::timeline::InitiativeKey;
use crate::world::World;

/// One fixed simulation step: the tick counter advances by one (saturating),
/// then the ordered system list runs.
///
/// The counter moves first, so a system observing `world.tick()` sees the
/// tick it runs under. `run_systems` is the whole point of the function —
/// the list, its order, and its stage comments are the contract.
pub fn tick(world: &mut World) {
    let _ = tick_tracked(world);
}

/// [`tick`], reporting the transitions and executions the encounter stages
/// observed (T138b, ADR-0029 N5).
///
/// Crate-visible only: [`tick`] delegates here and discards the facts, so its
/// behaviour and bytes are unchanged; the history wrapper journals the facts.
pub(crate) fn tick_tracked(world: &mut World) -> TickFacts {
    world.advance_tick();
    let mut facts = TickFacts::default();
    run_systems(world, &mut facts);
    facts
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
/// Each system appends with its task and its stage comment; reordering
/// existing entries is a behaviour change and gets reviewed as one.
fn run_systems(world: &mut World, facts: &mut TickFacts) {
    // §10 stage 2 (timers, durations, refreshes) — T138: the encounter clock.
    encounter_clock_system(world, facts);
    // §10 stage 3 (timeline order) — T138: RT execution of due orders.
    timeline_system(world, facts);
}

/// Encounter-clock step (T138, ADR-0029 T3, T6, T9, T10): advances the clock
/// of a running real-time encounter, delivers `OnTick` refreshes, and applies
/// the round boundary. Every other world, including every turn-based
/// encounter, is untouched.
fn encounter_clock_system(world: &mut World, facts: &mut TickFacts) {
    crate::combat::clock_stage(world, facts);
}

/// Timeline step: the standing queue is walked in order every tick, and
/// standing order is preserved — no reorder, no rekey — except for the
/// real-time execution of due orders (T138, ADR-0029 T7).
///
/// The per-entry dispatch loop is already here, applying no effect itself.
/// The execution stage only runs the orders that commands stored earlier;
/// it never invents an action. Net observable effect of a no-input tick:
/// counter +1, plus the clock of a running real-time encounter.
fn timeline_system(world: &mut World, facts: &mut TickFacts) {
    for _entity in world.timeline().iter().map(|(_, id)| id) {
        // No per-entity effect of its own: RT execution follows below.
    }
    crate::combat::execution_stage(world, facts);
}
