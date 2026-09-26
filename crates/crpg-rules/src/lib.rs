#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Game-system-neutral stat and modifier kernel (spec §24 T14).
//!
//! Typed in-memory stat definitions, data-selected modifier stacking,
//! always-present query breakdowns, derived-stat validation, explicit string
//! persistence, and kernel lifecycle-hook types. Queries are pure borrows
//! over caller-owned state; callers own modifier membership, entity liveness,
//! and authority. Dice-valued stats and resolution-dependent hooks are T015.

pub mod derived;
pub mod error;
pub mod hooks;
pub mod modifier;
pub mod stats;

pub use error::{RulesError, RulesErrorCode};
pub use hooks::{HookHandler, HookMutation, KernelHook};
pub use modifier::{
    ConditionExpr, Contribution, ContributionStatus, ModOp, ModTypeId, Modifier, ModifierBreakdown,
    ModifierPipeline, ModifierTarget, QueryContext, SourceRef, StackingPolicy, StatTrace,
};
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
