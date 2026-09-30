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

pub mod area;
pub mod combat;
pub mod event;
pub mod hash;
pub mod history;
pub mod store;
pub mod tick;
pub mod timeline;
pub mod transform;
pub mod world;

pub use area::{transfer_entity, transfer_history_entity, AreaError};
pub use combat::{
    end_encounter, legal_actions, perform_action, start_encounter, validate_action, AbilityDefense,
    AbilityDefinition, ActionOutcome, AttachedEffect, CombatAction, CombatDefinition, CombatError,
    CombatState, Combatant, EffectAim, EffectDefinition, EffectModifier, EffectOp, EffectTarget,
    EncounterSpec, EncounterSummary, LegalActionsError, ParticipantResult, PlacementAndArea,
    PoolTemplate, COMBAT_ROLL_STREAM, COMBAT_ROLL_TAG, MAX_COMBATANTS, MAX_COMBAT_STATS,
    MAX_LEGAL_ACTIONS,
};
pub use event::SimEvent;
pub use hash::state_hash;
pub use history::{
    history_hash, HistoryEnvelope, HistoryError, HistoryEvent, HistoryWorld, HISTORY_VERSION,
    MAX_HISTORY_BYTES, MAX_HISTORY_EVENTS, MAX_HISTORY_PAGE, MAX_HISTORY_STRING_BYTES,
};
pub use store::ComponentStore;
pub use tick::{end_turn, tick};
pub use timeline::{InitiativeKey, Timeline};
pub use transform::Transform;
pub use world::{EntityMeta, World};
