//! Persisted area identity and explicit entity transfer (T027a, ADR-0020).
//!
//! A [`World`] simulates one loaded area. A bound world
//! ([`World::new_in_area`]) persists that area's authored ULID, immutable after
//! construction; presence in the area is exactly liveness in the world, so
//! [`World::area_of`] needs no membership index that could dangle. Death is
//! not despawn: a dead, retained combatant is still present.
//!
//! [`transfer_entity`] and [`transfer_history_entity`] are the one explicit
//! simulation transition between two bound worlds, requested by a higher
//! authorized owner — not automatic navigation, and not a client operation.
//! They move only the currently supported noncombat, unscheduled shape
//! (`EntityMeta` plus an optional `Transform`), allocating a fresh identity in
//! the destination. Runtime identity across worlds is `(area, EntityId)`
//! because two worlds can mint equal arena ids. Anything the move cannot
//! carry fails the whole transfer instead of disappearing.

use std::fmt;

use crpg_core::{EntityId, Ulid};

use crate::history::{HistoryError, HistoryWorld};
use crate::world::World;

/// Every transfer failure, checked in declaration order.
///
/// `Display` is `<VariantName> at area/transfer` without fields; the two
/// history wrappers expose their journal error through `source`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AreaError {
    /// The source world carries no area identity.
    UnboundSource,
    /// The destination world carries no area identity.
    UnboundDestination,
    /// Source and destination name the same area.
    SameArea {
        /// The shared area identity.
        area: Ulid,
    },
    /// The entity is not live in the source world.
    AbsentEntity {
        /// The absent entity.
        entity: EntityId,
    },
    /// The entity holds a combatant component (dead or alive); release the
    /// encounter first.
    CombatParticipant {
        /// The combat participant.
        entity: EntityId,
    },
    /// The entity holds a timeline entry.
    ScheduledEntity {
        /// The scheduled entity.
        entity: EntityId,
    },
    /// The entity's transform holds a non-finite position or velocity.
    InvalidTransform {
        /// The entity with the corrupt transform.
        entity: EntityId,
    },
    /// The source journal cannot accept the removal.
    SourceHistory(HistoryError),
    /// The destination journal cannot accept the spawn.
    DestinationHistory(HistoryError),
}

impl fmt::Display for AreaError {
    /// Renders `<VariantName> at area/transfer`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::UnboundSource => "UnboundSource",
            Self::UnboundDestination => "UnboundDestination",
            Self::SameArea { .. } => "SameArea",
            Self::AbsentEntity { .. } => "AbsentEntity",
            Self::CombatParticipant { .. } => "CombatParticipant",
            Self::ScheduledEntity { .. } => "ScheduledEntity",
            Self::InvalidTransform { .. } => "InvalidTransform",
            Self::SourceHistory(_) => "SourceHistory",
            Self::DestinationHistory(_) => "DestinationHistory",
        };
        write!(f, "{name} at area/transfer")
    }
}

impl std::error::Error for AreaError {
    /// The wrapped journal error for the two history variants.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SourceHistory(error) | Self::DestinationHistory(error) => Some(error),
            _ => None,
        }
    }
}

/// Checks the pinned transfer preconditions in order, read-only.
fn preflight(source: &World, destination: &World, entity: EntityId) -> Result<(), AreaError> {
    let from = source.area().ok_or(AreaError::UnboundSource)?;
    let to = destination.area().ok_or(AreaError::UnboundDestination)?;
    if from == to {
        return Err(AreaError::SameArea { area: from });
    }
    if !source.contains(entity) {
        return Err(AreaError::AbsentEntity { entity });
    }
    if source.combatants().contains(entity) {
        return Err(AreaError::CombatParticipant { entity });
    }
    if source.timeline().contains(entity) {
        return Err(AreaError::ScheduledEntity { entity });
    }
    if let Some(transform) = source.transforms().get(entity) {
        let finite = transform
            .position
            .iter()
            .chain(transform.velocity.iter())
            .all(|value| value.is_finite());
        if !finite {
            return Err(AreaError::InvalidTransform { entity });
        }
    }
    Ok(())
}

/// Moves one noncombat, unscheduled entity between two bound worlds,
/// returning its new destination identity.
///
/// Preconditions check in [`AreaError`] declaration order. Both worlds are
/// staged and published together: the source despawns the entity (one
/// `Despawned` at its tick), the destination spawns the copied `EntityMeta`
/// (one `Spawned` at its tick) and receives the copied `Transform` if one
/// existed — an absent transform stays absent. RNG, interners, encounter
/// state and ticks do not migrate or change. The returned id may equal the
/// old one numerically; it belongs to the destination area. A rejection
/// leaves both worlds unchanged.
pub fn transfer_entity(
    source: &mut World,
    destination: &mut World,
    entity: EntityId,
) -> Result<EntityId, AreaError> {
    preflight(source, destination, entity)?;
    let meta = source
        .entity_meta(entity)
        .expect("preflight checks the entity is live");
    let transform = source.transforms().get(entity).copied();
    let mut staged_source = source.clone();
    let mut staged_destination = destination.clone();
    let removed = staged_source.despawn(entity);
    debug_assert!(removed, "preflight checks the entity is live");
    let moved = staged_destination.spawn(meta);
    if let Some(transform) = transform {
        staged_destination.transforms_mut().insert(moved, transform);
    }
    *source = staged_source;
    *destination = staged_destination;
    Ok(moved)
}

/// [`transfer_entity`] between two history wrappers, journaling the source
/// `Despawned` and the destination `Spawned` atomically.
///
/// After the same preflight, both full wrappers are staged and mutated only
/// through their typed despawn/spawn paths. A source journal failure
/// (`SourceHistory`) is reported before a destination one
/// (`DestinationHistory`); any failure publishes neither wrapper. There is no
/// global cross-area sequence: each journal keeps its own order and clock.
pub fn transfer_history_entity(
    source: &mut HistoryWorld,
    destination: &mut HistoryWorld,
    entity: EntityId,
) -> Result<EntityId, AreaError> {
    preflight(source.world(), destination.world(), entity)?;
    let meta = source
        .world()
        .entity_meta(entity)
        .expect("preflight checks the entity is live");
    let transform = source.world().transforms().get(entity).copied();
    let mut staged_source = source.clone();
    let mut staged_destination = destination.clone();
    let removed = staged_source
        .despawn(entity)
        .map_err(AreaError::SourceHistory)?;
    debug_assert!(removed, "preflight checks the entity is live");
    let moved = staged_destination
        .spawn(meta)
        .map_err(AreaError::DestinationHistory)?;
    if let Some(transform) = transform {
        staged_destination.attach_transform(moved, transform);
    }
    *source = staged_source;
    *destination = staged_destination;
    Ok(moved)
}
