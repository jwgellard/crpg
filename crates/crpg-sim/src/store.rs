//! Dense per-type component storage: [`ComponentStore`].
//!
//! One store holds one component type for every entity that has it, keyed by
//! [`EntityId`](crpg_core::EntityId). The store is deliberately dumb: it knows
//! nothing about liveness, so inserting under a dead id succeeds and reads
//! back. The no-dangling-entries invariant is maintained one layer up, by
//! [`World`](crate::World) — `despawn` strips every store — and that is where
//! it is tested. Do not add liveness checks here; the store has no arena to
//! check against, and borrowing one would tangle the two types for nothing.
//!
//! Iteration follows `IndexMap` insertion order, which is deterministic for
//! identical operation sequences. It is *not* sorted order, and consumers must
//! not assume it: if sorted iteration is ever needed, that is a new method
//! with its own test, not a silent change to [`iter`](ComponentStore::iter).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crpg_core::EntityId;

/// Storage for one component type, keyed by entity.
///
/// Backed by an `IndexMap` with its default hasher. The hasher never affects
/// observable behaviour — iteration is insertion-ordered, serialization
/// follows iteration order — so it costs nothing in determinism and there is
/// nothing to replace with a deterministic one.
///
/// Serde is a list of `(id, component)` pairs, not a map: JSON maps need
/// string keys and an [`EntityId`] is not one. Iteration order is insertion
/// order, so the pair list round-trips exactly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComponentStore<T> {
    entries: IndexMap<EntityId, T>,
}

impl<T> ComponentStore<T> {
    /// An empty store.
    pub fn new() -> Self {
        Self {
            entries: IndexMap::default(),
        }
    }

    /// Associates `value` with `id`, returning any value it replaced.
    ///
    /// No liveness check: the store does not know which ids are alive.
    pub fn insert(&mut self, id: EntityId, value: T) -> Option<T> {
        self.entries.insert(id, value)
    }

    /// Removes and returns the component for `id`, if present.
    pub fn remove(&mut self, id: EntityId) -> Option<T> {
        self.entries.shift_remove(&id)
    }

    /// The component for `id`, if present.
    pub fn get(&self, id: EntityId) -> Option<&T> {
        self.entries.get(&id)
    }

    /// Mutably, the component for `id`, if present.
    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut T> {
        self.entries.get_mut(&id)
    }

    /// Whether `id` has a component in this store.
    pub fn contains(&self, id: EntityId) -> bool {
        self.entries.contains_key(&id)
    }

    /// The number of stored components.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store holds no components.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drops every component. Entity liveness is unaffected.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// All entries in insertion order (see module docs: not sorted).
    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &T)> {
        self.entries.iter().map(|(id, value)| (*id, value))
    }

    /// Mutably, all entries in insertion order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (EntityId, &mut T)> {
        self.entries.iter_mut().map(|(id, value)| (*id, value))
    }
}

impl<T: Serialize> Serialize for ComponentStore<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let pairs: Vec<(&EntityId, &T)> = self.entries.iter().collect();
        pairs.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for ComponentStore<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let pairs = Vec::<(EntityId, T)>::deserialize(deserializer)?;
        let mut store = ComponentStore::new();
        for (id, value) in pairs {
            store.entries.insert(id, value);
        }
        Ok(store)
    }
}
