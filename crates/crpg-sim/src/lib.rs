#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Simulation engine: purpose-built entity/component store, systems,
//! fixed-order tick loop, spatial queries, movement, LOS, and encounter
//! management — the authoritative world the server owns.
//!
//! `crpg-sim` sits above `crpg-core`, `crpg-data` and `crpg-rules` in the
//! dependency graph and is the crate every live-simulation consumer
//! (`crpg-net`, `crpg-script`, `crpg-persist`, the server) reaches. Today it
//! holds the [`World`] skeleton only: entity arena, component stores,
//! timeline container, live event queue, deterministic RNG and tick counter.
//! Systems, the tick loop, `state_hash` and movement arrive in later tasks;
//! the module docs say which task owns each missing piece so nothing here
//! reads as finished.

pub mod event;
pub mod store;
pub mod timeline;
pub mod transform;
pub mod world;

pub use event::SimEvent;
pub use store::ComponentStore;
pub use timeline::{InitiativeKey, Timeline};
pub use transform::Transform;
pub use world::{EntityMeta, World};
