//! Who is on which side (T056a Reading 1).
//!
//! Sim, rules and core have no sides, teams or allegiance: faction exists only
//! as authored data, and `crpg-ai` may not read it. The host driver therefore
//! fills a [`Sides`] from authored data and hands it to the decision. Two
//! listed entities are hostile exactly when their sides differ; an unlisted
//! entity is hostile to no one and no one is hostile to it.

use std::fmt;

use crpg_core::EntityId;

/// A side in a fight; entities on different sides are hostile (Reading 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SideId(pub u32);

/// Who is on which side; built by the caller (Reading 1).
///
/// Entries are kept sorted by ascending [`EntityId`] and hold each entity at
/// most once, so [`Sides::side`] can be a binary search.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sides {
    entries: Vec<(EntityId, SideId)>,
}

/// Why [`Sides::new`] refused its input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidesError {
    /// The entity is listed more than once.
    Duplicate {
        /// The least entity listed twice.
        entity: EntityId,
    },
}

impl fmt::Display for SidesError {
    /// Renders `Duplicate at sides/{entity:?}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate { entity } => write!(f, "Duplicate at sides/{entity:?}"),
        }
    }
}

impl std::error::Error for SidesError {
    /// Nothing is wrapped, so there is no source.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

impl Sides {
    /// Sorts by `EntityId`; the first entity listed twice (in sorted order) is
    /// [`SidesError::Duplicate`].
    pub fn new(mut entries: Vec<(EntityId, SideId)>) -> Result<Self, SidesError> {
        entries.sort_by_key(|(entity, _)| *entity);
        for pair in entries.windows(2) {
            if pair[0].0 == pair[1].0 {
                return Err(SidesError::Duplicate { entity: pair[0].0 });
            }
        }
        Ok(Self { entries })
    }

    /// The entries, ascending `EntityId`.
    pub fn entries(&self) -> &[(EntityId, SideId)] {
        &self.entries
    }

    /// The entity's side, if listed.
    pub fn side(&self, entity: EntityId) -> Option<SideId> {
        self.entries
            .binary_search_by_key(&entity, |(listed, _)| *listed)
            .ok()
            .map(|index| self.entries[index].1)
    }

    /// Both listed, different sides (so never `hostile(e, e)`).
    pub fn hostile(&self, a: EntityId, b: EntityId) -> bool {
        match (self.side(a), self.side(b)) {
            (Some(left), Some(right)) => left != right,
            _ => false,
        }
    }
}
