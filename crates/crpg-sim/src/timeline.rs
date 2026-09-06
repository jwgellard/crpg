//! Turn/initiative ordering: [`InitiativeKey`] and [`Timeline`].
//!
//! Real-time-with-pause and turn-based play share one mechanism (spec §2.5):
//! an ordered queue of `(InitiativeKey, EntityId)`. This module is the
//! *container* only. The advance policy — every tick versus `EndTurn`, and
//! the mapping between §6.2 turn-start AI and the §10 per-tick loop — belongs
//! to T008's tick loop, not here. Do not add `advance`, `next_turn` or any
//! other policy method to this module; that policy is the most review-dense
//! decision in the crate (spec §15.2) and it gets its own task.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crpg_core::EntityId;

/// Position in the turn order. Lower acts first; ties break on [`EntityId`].
///
/// A bare `i32` so rulesets can space keys out (D&D-style descending counts,
/// tick-indexed real-time keys, negative surprise penalties) without the
/// engine caring what the numbers mean.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct InitiativeKey(pub i32);

/// The ordered set of entities awaiting a turn: at most one entry per entity.
///
/// Backed by a `BTreeMap`, so iteration is ascending
/// `(InitiativeKey, EntityId)` — deterministic by construction, with the id
/// as a total tiebreak. Removal by entity is an O(n) scan; that is documented
/// rather than indexed because populations are small and an index is a second
/// structure that can disagree with the first. A perf task may add one; this
/// task may not need one.
///
/// Serde is a list of `(key, id)` pairs, not a map: JSON maps need string
/// keys and a tuple is not one. The in-memory order is canonical
/// (`BTreeMap`), so the pair list round-trips exactly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Timeline {
    entries: BTreeMap<(InitiativeKey, EntityId), ()>,
}

impl From<Vec<(InitiativeKey, EntityId)>> for Timeline {
    fn from(pairs: Vec<(InitiativeKey, EntityId)>) -> Self {
        let mut timeline = Timeline::new();
        for (key, id) in pairs {
            timeline.entries.insert((key, id), ());
        }
        timeline
    }
}

impl Serialize for Timeline {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let pairs: Vec<(InitiativeKey, EntityId)> = self.entries.keys().copied().collect();
        pairs.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Timeline {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let pairs = Vec::<(InitiativeKey, EntityId)>::deserialize(deserializer)?;
        Ok(pairs.into())
    }
}

impl Timeline {
    /// An empty timeline.
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Schedules `id` at `key`, replacing any entry `id` already holds.
    ///
    /// One entity, one entry: replacing (rather than duplicating) is what
    /// keeps [`remove`](Self::remove) total — after any sequence of inserts,
    /// removing the entity leaves no entry behind.
    pub fn insert(&mut self, key: InitiativeKey, id: EntityId) {
        self.remove(id);
        self.entries.insert((key, id), ());
    }

    /// Drops `id`'s entry, if it holds one. Reports whether one existed.
    pub fn remove(&mut self, id: EntityId) -> bool {
        let slot = self.entries.keys().find(|(_, entry)| *entry == id).copied();
        match slot {
            Some(key) => {
                self.entries.remove(&key);
                true
            }
            None => false,
        }
    }

    /// Whether `id` currently holds a timeline entry.
    pub fn contains(&self, id: EntityId) -> bool {
        self.entries.keys().any(|(_, entry)| *entry == id)
    }

    /// The number of scheduled entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is scheduled.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// All entries in ascending `(InitiativeKey, EntityId)` order.
    pub fn iter(&self) -> impl Iterator<Item = (InitiativeKey, EntityId)> + '_ {
        self.entries.keys().copied()
    }
}
