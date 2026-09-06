//! The live event vocabulary: [`SimEvent`].
//!
//! Per ADR-0008 the concrete `SimEvent` enum lives here, in the crate every
//! live-event consumer can reach, while the queue mechanism
//! ([`EventQueue`](crpg_core::EventQueue)) lives in `crpg-core`. This module
//! grows as systems arrive — component-set notices, timeline notices, delta
//! notices — but each addition is a vocabulary decision with consumers, not
//! speculative completeness. Today the world only structurally changes by
//! spawn and despawn, so those are the only two variants.

use serde::{Deserialize, Serialize};

use crpg_core::EntityId;

/// Something that happened to the world, in the world's own terms.
///
/// Carries entity ids, never component data: a consumer that needs the data
/// queries the world at the drained tick. The envelope
/// ([`EventEnvelope`](crpg_core::EventEnvelope)) already stamps the tick, so
/// variants do not repeat it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimEvent {
    /// [`World`](crate::World) spawned this entity.
    Spawned {
        /// The minted entity.
        entity: EntityId,
    },
    /// [`World`](crate::World) despawned this entity.
    Despawned {
        /// The removed entity.
        entity: EntityId,
    },
}
