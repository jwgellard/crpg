//! Shared builders for the rules-owned integration suites.
//!
//! Neutral stat, tag, and type names only: no game-system vocabulary enters
//! production or tests. Entity ids always come from a core arena, modifier
//! ids from explicit small integers, and fixed-point factors from raw
//! integers, never decimal literals.

use std::collections::BTreeMap;

use crpg_core::{EntityId, Fx16_16, GenerationalArena, Interners, StatId, TagId, Ulid};
use crpg_rules::{
    ConditionExpr, EnumValue, Expr, ModOp, ModTypeId, Modifier, ModifierTarget, QueryContext,
    SourceRef, StackingPolicy, StatBlock, StatDefinition, StatKind, StatValue, TagSet,
};

/// Neutral stat names, interned in this order by [`fixture`].
pub const STAT_NAMES: [&str; 10] = [
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
];

/// Neutral tag names, interned in this order by [`fixture`].
pub const TAG_NAMES: [&str; 4] = ["ta", "tb", "tc", "td"];

/// Neutral stacking-policy names.
pub const TYPE_A: &str = "ta-type";
/// Neutral stacking-policy names.
pub const TYPE_B: &str = "tb-type";
/// Neutral stacking-policy names.
pub const TYPE_C: &str = "tc-type";

/// One ready-made per-entity harness: interners, an arena, and its entity.
pub struct Fixture {
    /// Stat and tag namespaces with the fixed neutral names interned.
    pub interners: Interners,
    /// The arena that minted [`Fixture::entity`].
    pub arena: GenerationalArena<()>,
    /// The entity every context in the test describes.
    pub entity: EntityId,
    /// [`STAT_NAMES`] handles in intern order.
    pub stats: Vec<StatId>,
    /// [`TAG_NAMES`] handles in intern order.
    pub tags: Vec<TagId>,
}

/// Builds a fixture with neutral names interned in fixed order.
#[must_use]
pub fn fixture() -> Fixture {
    let mut interners = Interners::new();
    let mut stats = Vec::new();
    for name in STAT_NAMES {
        stats.push(interners.intern_stat(name));
    }
    let mut tags = Vec::new();
    for name in TAG_NAMES {
        tags.push(interners.intern_tag(name));
    }
    let mut arena = GenerationalArena::new();
    let entity = arena.insert(());
    Fixture {
        interners,
        arena,
        entity,
        stats,
        tags,
    }
}

/// Builds a modifier identity from a small integer.
#[must_use]
pub fn uid(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

/// Builds a fixed-point value from its raw integer, never a decimal.
#[must_use]
pub fn raw_fx(raw: i32) -> Fx16_16 {
    Fx16_16::from_raw(raw)
}

/// Builds one stat definition.
#[must_use]
pub fn def(id: StatId, kind: StatKind, derived: Option<Expr>) -> StatDefinition {
    StatDefinition { id, kind, derived }
}

/// Builds an `Int` definition.
#[must_use]
pub fn int_def(id: StatId) -> StatDefinition {
    def(id, StatKind::Int, None)
}

/// Builds an enum kind from a domain id and variant names.
#[must_use]
pub fn enum_kind(enum_id: &str, variants: &[&str]) -> StatKind {
    StatKind::Enum {
        enum_id: enum_id.to_owned(),
        variants: variants
            .iter()
            .map(|variant| (*variant).to_owned())
            .collect(),
    }
}

/// Builds an enum value.
#[must_use]
pub fn enum_value(enum_id: &str, variant: &str) -> StatValue {
    StatValue::Enum(EnumValue {
        enum_id: enum_id.to_owned(),
        variant: variant.to_owned(),
    })
}

/// Builds a tag set from handles.
#[must_use]
pub fn tag_set(tags: &[TagId]) -> TagSet {
    tags.iter().copied().collect()
}

/// Builds a block from entries.
#[must_use]
pub fn block(entries: &[(StatId, StatValue)]) -> StatBlock {
    let mut block = StatBlock::new();
    for (stat, value) in entries {
        block.insert(*stat, value.clone()).unwrap_or_else(|error| {
            panic!("test block insert failed for {stat:?}: {error}");
        });
    }
    block
}

/// Builds one policy map from pairs.
#[must_use]
pub fn policies(pairs: &[(&str, StackingPolicy)]) -> BTreeMap<ModTypeId, StackingPolicy> {
    pairs
        .iter()
        .map(|(name, policy)| (ModTypeId((*name).to_owned()), *policy))
        .collect()
}

/// Builds one modifier with explicit parts.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn modifier(
    id: u128,
    source_kind: &str,
    source_id: u128,
    target: StatId,
    op: ModOp,
    mod_type: &str,
    name: Option<&str>,
    condition: Option<ConditionExpr>,
    priority: i16,
) -> Modifier {
    Modifier {
        id: uid(id),
        source: SourceRef {
            kind: source_kind.to_owned(),
            id: uid(source_id),
        },
        target: ModifierTarget::Stat(target),
        op,
        mod_type: ModTypeId(mod_type.to_owned()),
        name: name.map(str::to_owned),
        condition,
        priority,
    }
}

/// Builds a query context borrowing one entity's view.
pub fn context<'a>(
    entity: EntityId,
    stats: &'a StatBlock,
    tags: &'a TagSet,
    modifiers: &'a [Modifier],
) -> QueryContext<'a> {
    QueryContext {
        entity,
        stats,
        tags,
        modifiers,
    }
}
