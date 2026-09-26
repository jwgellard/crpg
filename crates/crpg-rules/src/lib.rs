#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Game-system-neutral stat and modifier kernel (spec §24 T14) with dice,
//! outcome tables, resolution, and resources (spec §24 T15).
//!
//! Typed in-memory stat definitions, data-selected modifier stacking,
//! always-present query breakdowns, derived-stat validation, explicit string
//! persistence, bounded dice evaluation, data-driven outcome tables, generic
//! resolution with transactional randomness, resource pools, and kernel
//! lifecycle-hook types. Queries are pure borrows over caller-owned state;
//! callers own modifier membership, entity liveness, and authority.

pub mod derived;
pub mod dice;
pub mod error;
pub mod hooks;
pub mod modifier;
pub mod resolution;
pub mod resource;
pub mod stats;

pub use dice::{DiceExpr, DiceRoll, DiceSelection, DieResult};
pub use error::{RulesError, RulesErrorCode};
pub use hooks::{HookHandler, HookMutation, KernelHook};
pub use modifier::{
    ConditionExpr, Contribution, ContributionStatus, ModOp, ModTypeId, Modifier, ModifierBreakdown,
    ModifierPipeline, ModifierTarget, NumericModifierBreakdown, QueryContext, RollTag, SourceRef,
    StackingPolicy, StatTrace,
};
pub use resolution::{
    resolve, Against, AgainstBreakdown, NaturalEffect, NaturalRule, Outcome, OutcomeBand,
    OutcomeDecision, OutcomeTable, OutcomeTableId, ResolutionContext, ResolutionRequest,
    ResolutionResult, ResolvedRoll, RollRequest, RollSpec,
};
pub use resource::{RefreshEvent, RefreshTrigger, ResourcePool, ResourcePoolId, RestId};
pub use stats::{
    EnumValue, Expr, SerializableStatBlock, SerializableStatEntry, SerializableStatValue,
    StatBlock, StatDefinition, StatKind, StatValue, TagSet,
};

/// Definitions, block entries, and persisted entries in one set.
pub const MAX_STATS: usize = 1024;
/// Tags in one set, including query tags, literals, and persisted tags.
pub const MAX_TAGS: usize = 1024;
/// Variants in one enum domain.
pub const MAX_ENUM_VARIANTS: usize = 1024;
/// Stacking-policy entries in one pipeline.
pub const MAX_POLICIES: usize = 1024;
/// Modifiers in one context, before target filtering.
pub const MAX_MODIFIERS: usize = 4096;
/// Nodes in one derived expression.
pub const MAX_EXPR_NODES: usize = 4096;
/// Expression nodes in one definition set.
pub const MAX_TOTAL_EXPR_NODES: usize = 65536;
/// Expression tree depth, with the root counting as depth one.
pub const MAX_EXPR_DEPTH: usize = 32;
/// Longest stat dependency path, endpoints included.
pub const MAX_DERIVED_DEPTH: usize = 128;
/// Nodes in one modifier condition.
pub const MAX_CONDITION_NODES: usize = 256;
/// Condition tree depth, with the root counting as depth one.
pub const MAX_CONDITION_DEPTH: usize = 32;
/// Dice notation input length in UTF-8 bytes.
pub const MAX_DICE_INPUT_BYTES: usize = 128;
/// Dice in one expression.
pub const MAX_DICE_COUNT: usize = 1024;
/// Sides on one die.
pub const MAX_DIE_SIDES: u32 = 1_000_000;
/// Margin bands in one outcome table.
pub const MAX_OUTCOME_BANDS: usize = 256;
/// Natural-face rules in one outcome table.
pub const MAX_NATURAL_RULES: usize = 256;
/// Named RNG stream length in UTF-8 bytes.
pub const MAX_RNG_STREAM_BYTES: usize = 256;
