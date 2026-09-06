#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Simulation engine: purpose-built entity/component store, systems,
//! fixed-order tick loop, spatial queries, movement, LOS, and encounter
//! management — the authoritative world the server owns.
//!
//! `crpg-sim` sits above `crpg-core`, `crpg-data` and `crpg-rules` in the
//! dependency graph and is the crate every live-simulation consumer
//! (`crpg-net`, `crpg-script`, `crpg-persist`, the server) reaches. Today it
//! holds the [`World`] skeleton, the fixed-step [`tick`] loop with its first
//! system, the [`end_turn`] advance primitive, and the [`state_hash`]
//! measurement instrument. Further systems, movement and components arrive in
//! later tasks; the module docs say which task owns each missing piece so
//! nothing here reads as finished.

pub mod event;
pub mod hash;
pub mod store;
pub mod tick;
pub mod timeline;
pub mod transform;
pub mod world;

pub use event::SimEvent;
pub use hash::state_hash;
pub use store::ComponentStore;
pub use tick::{end_turn, tick};
pub use timeline::{InitiativeKey, Timeline};
pub use transform::Transform;
pub use world::{EntityMeta, World};
