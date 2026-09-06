//! The simulation skeleton: [`World`] and [`EntityMeta`].
//!
//! `World` owns one loaded area's worth of state: the entity arena (identity,
//! inherited from `crpg-core` with all five arena invariants), one component
//! store per engine component type, the timeline container, the live event
//! queue, the deterministic RNG, and the tick counter. Systems are ordinary
//! functions over `&mut World` called in a fixed hand-written order — but
//! that order, and the `tick` function that runs it, belong to T008, not
//! here. This module builds the data structure and its spawn/despawn/query
//! surface; it advances nothing.
//!
//! Serde covers the skeleton only (E014): entities, stores, timeline, event
//! queue, RNG, tick. There is no stat data in `World` until T014, and when
//! `StatBlock` arrives its persisted form is the explicit string-keyed
//! conversion pair from ADR-0006 Decision 4 — never a derived `Serialize` on
//! a map with `StatId` keys. If a struct in this module ever holds an
//! interned handle, that struct does not derive serde; it converts.
//!
//! Loading is validated: component and timeline ids must address live
//! entities, and duplicate timeline entities are rejected alongside the
//! [`Timeline`](crate::Timeline) guard.

use serde::{Deserialize, Serialize};

use crpg_core::{DeterministicRng, EntityId, EventQueue, GenerationalArena, Tick};

use crate::event::SimEvent;
use crate::store::ComponentStore;
use crate::timeline::Timeline;
use crate::transform::Transform;

/// Per-entity metadata: reserved for now.
///
/// The arena needs a value type per slot, and the spec sketch names this one,
/// but no consumer exists yet — no areas, no persistence, no authority model
/// — so any fields would be fiction a later task has to contradict. It is
/// passed explicitly at [`spawn`](World::spawn) so the call signature is
/// already stable for the day areas give it fields.
///
/// Deliberately a *braced* empty struct rather than a unit struct: the arena
/// stores slots as `Option<T>`, and in JSON a unit struct serializes as
/// `null` — indistinguishable from `None`. Every occupied slot would load as
/// vacant and the arena guard would reject its own save as corrupt (which is
/// exactly how this was found). `{}` and `null` stay distinct.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityMeta {}

/// One loaded area's simulation state.
///
/// Areas simulate independently (spec §2.4): cross-area effects travel
/// through a message queue on the `Campaign` object, which does not exist
/// yet. Likewise single-player runs this same struct in-process (spec §2.1);
/// there is no second world type for it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct World {
    entities: GenerationalArena<EntityMeta>,
    transforms: ComponentStore<Transform>,
    timeline: Timeline,
    events: EventQueue<SimEvent>,
    rng: DeterministicRng,
    tick: Tick,
}

/// Serialized shape of [`World`], in field order. Deserialization goes
/// through this shape and then validates cross-field invariants before
/// constructing a `World`, so persisted state cannot introduce dangling
/// component or timeline references.
#[derive(Deserialize)]
struct WorldRepr {
    entities: GenerationalArena<EntityMeta>,
    transforms: ComponentStore<Transform>,
    timeline: Timeline,
    events: EventQueue<SimEvent>,
    rng: DeterministicRng,
    tick: Tick,
}

impl<'de> Deserialize<'de> for World {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let repr = WorldRepr::deserialize(deserializer)?;
        for (id, _) in repr.transforms.iter() {
            if !repr.entities.contains(id) {
                return Err(serde::de::Error::custom(format!(
                    "dangling transform for non-live {id:?}"
                )));
            }
        }
        for (_, id) in repr.timeline.iter() {
            if !repr.entities.contains(id) {
                return Err(serde::de::Error::custom(format!(
                    "dangling timeline entry for non-live {id:?}"
                )));
            }
        }
        Ok(Self {
            entities: repr.entities,
            transforms: repr.transforms,
            timeline: repr.timeline,
            events: repr.events,
            rng: repr.rng,
            tick: repr.tick,
        })
    }
}

impl World {
    /// A new, empty world: tick zero, RNG seeded from `seed`.
    ///
    /// Same seed in, same world out — the seed is the first input the
    /// determinism story depends on.
    pub fn new(seed: u64) -> Self {
        Self {
            entities: GenerationalArena::new(),
            transforms: ComponentStore::new(),
            timeline: Timeline::new(),
            events: EventQueue::new(),
            rng: DeterministicRng::from_seed(seed),
            tick: Tick::ZERO,
        }
    }

    /// Mints an entity holding `meta` and enqueues `Spawned` at the current
    /// tick.
    ///
    /// Identity comes from the arena untouched: generations from 1, the
    /// `u32::MAX` tombstone never issued, ascending-index iteration. See the
    /// `crpg-core` contract, which this method inherits rather than restates.
    pub fn spawn(&mut self, meta: EntityMeta) -> EntityId {
        let id = self.entities.insert(meta);
        self.events
            .push(self.tick, SimEvent::Spawned { entity: id });
        id
    }

    /// Removes `id`, strips its component and timeline entries, and enqueues
    /// `Despawned`.
    ///
    /// Returns `false` — and enqueues nothing — for a dead id. After a
    /// successful despawn no store key and no timeline entry addresses `id`;
    /// maintaining that is this method's whole job, because the stores
    /// themselves do no liveness checking.
    pub fn despawn(&mut self, id: EntityId) -> bool {
        if self.entities.remove(id).is_none() {
            return false;
        }
        self.transforms.remove(id);
        self.timeline.remove(id);
        self.events
            .push(self.tick, SimEvent::Despawned { entity: id });
        true
    }

    /// Whether `id` names a live entity.
    pub fn contains(&self, id: EntityId) -> bool {
        self.entities.contains(id)
    }

    /// The number of live entities.
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Whether no entities are live.
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Every live id, in ascending index order (inherited arena invariant).
    pub fn ids(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.entities.ids()
    }

    /// The world's current tick. Getter only: advancing time — the tick
    /// loop, systems, event-graph runs — is T008.
    pub fn tick(&self) -> Tick {
        self.tick
    }

    /// Advances the tick counter by one, saturating at `u64::MAX`.
    ///
    /// Crate-visible only: the [`tick`](crate::tick) loop calls it; no outer
    /// code sets time, so there is deliberately no public setter (see the
    /// crate contract). Saturation is unreachable-by-construction — 2^64
    /// ticks is not a workload — exactly as the event `seq` counter in
    /// `crpg-core`, and for the same reason it saturates rather than wraps:
    /// wrapping would silently reorder time.
    pub(crate) fn advance_tick(&mut self) {
        self.tick = self.tick.saturating_add(1);
    }

    /// The transform store. No liveness check: check
    /// [`contains`](Self::contains) first, or keep the id you spawned.
    pub fn transforms(&self) -> &ComponentStore<Transform> {
        &self.transforms
    }

    /// Mutably, the transform store (same caveat as
    /// [`transforms`](Self::transforms)).
    pub fn transforms_mut(&mut self) -> &mut ComponentStore<Transform> {
        &mut self.transforms
    }

    /// The timeline container. Advance policy is T008's, not this type's.
    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// Mutably, the timeline container.
    pub fn timeline_mut(&mut self) -> &mut Timeline {
        &mut self.timeline
    }

    /// The live event queue. Draining it is T008's tick loop's job; reading
    /// it is anyone's.
    pub fn events(&self) -> &EventQueue<SimEvent> {
        &self.events
    }

    /// Mutably, the live event queue.
    pub fn events_mut(&mut self) -> &mut EventQueue<SimEvent> {
        &mut self.events
    }

    /// The world's deterministic RNG. Callers name their stream explicitly;
    /// streams they do not name they do not disturb.
    pub fn rng_mut(&mut self) -> &mut DeterministicRng {
        &mut self.rng
    }
}

impl Default for World {
    /// `new(0)`: for tests and tooling that need a world but own no seed.
    fn default() -> Self {
        Self::new(0)
    }
}
