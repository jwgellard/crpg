//! Lifecycle hook vocabulary, mutation proposals, and handler signatures.
//!
//! [`KernelHook`] is the rules-owned vocabulary with core-closed fields only,
//! per ADR-0008 and ADR-0011: entities, round counters, and authored ULIDs,
//! never interned handles or component data. It is the one rules type besides
//! the persistence DTOs that serializes, so it can ride the generic
//! [`EventQueue`](crpg_core::EventQueue). [`HookMutation`] is runtime-only
//! command data proposing a base-stat update or a source removal; it never
//! serializes and never travels as an event payload. There is no handler
//! registry or dispatcher here: caller-provided handlers run directly, and
//! future dispatch work must preserve handler and vector order.

use crpg_core::{EntityId, StatId, Ulid};

use crate::error::RulesError;
use crate::modifier::SourceRef;
use crate::stats::StatValue;

/// One lifecycle hook: the kernel vocabulary hosts dispatch above.
///
/// Round zero and zero ULIDs are valid; event meaning and entity liveness
/// belong to the host. Serializes internally tagged in snake case with
/// unknown fields rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelHook {
    /// An entity died.
    OnDeath {
        /// The entity that died.
        entity: EntityId,
    },
    /// An entity's turn started.
    OnTurnStart {
        /// The entity whose turn started.
        entity: EntityId,
    },
    /// An entity's turn ended.
    OnTurnEnd {
        /// The entity whose turn ended.
        entity: EntityId,
    },
    /// A round started.
    OnRoundStart {
        /// The round that started, zero-based.
        round: u32,
    },
    /// An encounter started.
    OnEncounterStart {
        /// The encounter that started.
        encounter: Ulid,
    },
    /// A resolution started rolling for an actor.
    ///
    /// The host supplies the correlation ULID and owns the emission point;
    /// `resolve` itself emits nothing.
    BeforeRoll {
        /// The entity rolling.
        actor: EntityId,
        /// Contextual identity only.
        target: Option<EntityId>,
        /// The host-supplied correlation identity for this resolution.
        resolution: Ulid,
    },
    /// A resolution finished rolling for an actor.
    ///
    /// Carries the final rolled total. The host supplies the correlation
    /// ULID and owns the emission point; `resolve` itself emits nothing.
    AfterRoll {
        /// The entity that rolled.
        actor: EntityId,
        /// Contextual identity only.
        target: Option<EntityId>,
        /// The host-supplied correlation identity for this resolution.
        resolution: Ulid,
        /// The final rolled total.
        total: i32,
    },
    /// Caller-computed damage is about to apply from a source to a target.
    ///
    /// The amount is caller-computed, caller-applied, and nonnegative; rules
    /// performs no damage calculation, application, or authority check here.
    /// The host supplies the correlation ULID and owns the emission point.
    BeforeDamage {
        /// The entity the damage is attributed to.
        source: EntityId,
        /// The entity receiving the damage.
        target: EntityId,
        /// The host-supplied correlation identity for this resolution.
        resolution: Ulid,
        /// The caller-computed nonnegative damage amount.
        amount: i32,
    },
    /// Caller-computed damage was applied from a source to a target.
    ///
    /// The amount is caller-computed, caller-applied, and nonnegative; rules
    /// performs no damage calculation, application, or authority check here.
    /// The host supplies the correlation ULID and owns the emission point.
    AfterDamage {
        /// The entity the damage is attributed to.
        source: EntityId,
        /// The entity that received the damage.
        target: EntityId,
        /// The host-supplied correlation identity for this resolution.
        resolution: Ulid,
        /// The caller-computed nonnegative damage amount.
        amount: i32,
    },
}

/// One runtime-only mutation proposal from a hook handler.
///
/// Proposes a base-stat update or a source removal without mutating anything
/// itself. The eventual host validates declarations, liveness, permissions,
/// and ordering before applying proposals. Never serialized and never
/// submitted as an event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookMutation {
    /// Proposes setting one entity's stored base stat.
    SetBaseStat {
        /// The entity whose base changes.
        entity: EntityId,
        /// The stat whose base changes.
        stat: StatId,
        /// The proposed base value.
        value: StatValue,
    },
    /// Proposes removing every modifier from one source on one entity.
    RemoveSource {
        /// The entity whose modifiers shrink.
        entity: EntityId,
        /// The source whose modifiers leave.
        source: SourceRef,
    },
}

/// A pure hook handler: one hook in, an ordered proposal list out.
///
/// Handler purity is a documented convention, not a compiler guarantee: the
/// `fn` type carries no host or world reference, but nothing stops a handler
/// from reading ambient state. Returning `Err` produces no mutation list and
/// applies nothing in rules.
pub type HookHandler = fn(&KernelHook) -> Result<Vec<HookMutation>, RulesError>;
