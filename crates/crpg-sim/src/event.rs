//! The live event vocabulary: [`SimEvent`].
//!
//! Per ADR-0008 the concrete `SimEvent` enum lives here, in the crate every
//! live-event consumer can reach, while the queue mechanism
//! ([`EventQueue`](crpg_core::EventQueue)) lives in `crpg-core`. This module
//! grows as systems arrive — component-set notices, timeline notices, delta
//! notices — but each addition is a vocabulary decision with consumers, not
//! speculative completeness. Spawn and despawn cover structural change;
//! combat adds `Died`, produced exactly once per death with the terminal
//! assertions and future AI/effect systems as its consumers.

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
    /// Combat killed this entity (T016b, ADR-0013).
    ///
    /// Produced exactly once by [`perform_action`](crate::perform_action) on
    /// the positive-health-to-zero transition. Death is distinct from arena
    /// despawn: the entity stays live with its zero-health terminal state
    /// and is only removed from combat scheduling. Consumers are the
    /// terminal-state assertions (tests, then T016c) and future AI/effect
    /// systems.
    Died {
        /// The combatant whose health reached zero.
        entity: EntityId,
    },
}
