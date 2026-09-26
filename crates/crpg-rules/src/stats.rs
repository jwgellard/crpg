//! Typed stats, definitions, expressions, blocks, and symbolic persistence.
//!
//! The staged value model covers `Int`, `Fixed`, `Bool`, `Enum`, `Tags`, and
//! `Dice` expression values. [`StatBlock`] is the only mutable state:
//! insertion validates standalone value shape and limits before changing
//! anything. Definitions and queries validate kinds, domains, and expressions
//! at the pipeline boundary, which owns the [`Interners`] conversion pair
//! [`StatBlock::to_serializable`]/[`StatBlock::from_serializable`] that keeps
//! persisted bytes in string form per ADR-0006 Decision 4. Dice values
//! persist as canonical notation strings parsed during preflight.

use std::collections::BTreeSet;

use crpg_core::{Fx16_16, Interners, StatId, TagId};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::dice::DiceExpr;
use crate::error::{RulesError, RulesErrorCode};
use crate::{MAX_STATS, MAX_TAGS};

/// The tag set attached to one entity for condition evaluation.
pub type TagSet = BTreeSet<TagId>;

/// One enum-valued datum: the domain it belongs to and the selected member.
///
/// Both strings are nonempty, exact, case-sensitive UTF-8. Domain membership
/// is checked against pipeline definitions at query time; the standalone
/// conversions only require nonempty strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnumValue {
    /// The enum domain identifier.
    pub enum_id: String,
    /// The selected variant within the domain.
    pub variant: String,
}

/// One typed stat value.
///
/// `StatValue` is runtime-only and deliberately has no `Serialize`
/// implementation: persistence resolves handles through the issuing
/// [`Interners`] into [`SerializableStatValue`].
///
/// ```compile_fail
/// # use crpg_rules::StatValue;
/// fn assert_serialize<T: serde::Serialize>(_: T) {}
/// assert_serialize(StatValue::Int(1));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatValue {
    /// A whole number.
    Int(i32),
    /// A fixed-point number.
    Fixed(Fx16_16),
    /// A flag.
    Bool(bool),
    /// A member of a declared enum domain.
    Enum(EnumValue),
    /// A set of tag handles.
    Tags(TagSet),
    /// A dice expression value. Querying it returns the expression itself;
    /// only explicit evaluation or resolution draws dice.
    Dice(DiceExpr),
}

/// The declared kind of one stat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatKind {
    /// Whole numbers.
    Int,
    /// Fixed-point numbers.
    Fixed,
    /// Flags.
    Bool,
    /// A named domain with its exact variant set.
    Enum {
        /// The enum domain identifier, nonempty.
        enum_id: String,
        /// The exact variant set, all nonempty.
        variants: BTreeSet<String>,
    },
    /// Tag sets.
    Tags,
    /// Dice expression values.
    Dice,
}

/// A typed derived-stat expression.
///
/// `Literal` and `Stat` nodes can produce any supported kind. All six binary
/// operators require two `Int` or two `Fixed` operands and produce that same
/// kind; there is no implicit conversion. Operands evaluate left before
/// right using the authored tree grouping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// A constant value.
    Literal(StatValue),
    /// The fully modified value of another stat on the same entity.
    Stat(StatId),
    /// Saturated integer addition or fixed-point addition.
    Add(Box<Expr>, Box<Expr>),
    /// Saturated integer subtraction or fixed-point subtraction.
    Subtract(Box<Expr>, Box<Expr>),
    /// Saturated integer multiplication or fixed-point multiplication.
    Multiply(Box<Expr>, Box<Expr>),
    /// Flooring division; division by zero is a typed error.
    Divide(Box<Expr>, Box<Expr>),
    /// The smaller of two same-kind numeric values.
    Min(Box<Expr>, Box<Expr>),
    /// The larger of two same-kind numeric values.
    Max(Box<Expr>, Box<Expr>),
}

/// One stat declaration: its identity, kind, and optional derived formula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatDefinition {
    /// The stat this declaration defines.
    pub id: StatId,
    /// The declared kind every value of this stat must match exactly.
    pub kind: StatKind,
    /// The formula evaluated before this stat's modifiers, if derived.
    pub derived: Option<Expr>,
}

/// The discriminant of a [`StatValue`] or [`StatKind`], for exact matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueKind {
    Int,
    Fixed,
    Bool,
    Enum,
    Tags,
    Dice,
}

impl StatValue {
    /// Returns the discriminant of this value.
    pub(crate) fn kind(&self) -> ValueKind {
        match self {
            Self::Int(_) => ValueKind::Int,
            Self::Fixed(_) => ValueKind::Fixed,
            Self::Bool(_) => ValueKind::Bool,
            Self::Enum(_) => ValueKind::Enum,
            Self::Tags(_) => ValueKind::Tags,
            Self::Dice(_) => ValueKind::Dice,
        }
    }
}

impl StatKind {
    /// Returns the discriminant of this declaration.
    pub(crate) fn kind(&self) -> ValueKind {
        match self {
            Self::Int => ValueKind::Int,
            Self::Fixed => ValueKind::Fixed,
            Self::Bool => ValueKind::Bool,
            Self::Enum { .. } => ValueKind::Enum,
            Self::Tags => ValueKind::Tags,
            Self::Dice => ValueKind::Dice,
        }
    }
}

/// Checks standalone value shape and limits, without definitions.
///
/// Accepts every well-formed value: nonempty enum strings and tag sets
/// within [`MAX_TAGS`]. Definition-kind matching happens at the pipeline
/// boundary, not here.
pub(crate) fn check_standalone_value(value: &StatValue) -> Result<(), RulesErrorCode> {
    match value {
        StatValue::Int(_) | StatValue::Fixed(_) | StatValue::Bool(_) => Ok(()),
        StatValue::Dice(_) => Ok(()),
        StatValue::Enum(entry) => {
            if entry.enum_id.is_empty() || entry.variant.is_empty() {
                return Err(RulesErrorCode::InvalidName);
            }
            Ok(())
        }
        StatValue::Tags(tags) => {
            if tags.len() > MAX_TAGS {
                return Err(RulesErrorCode::LimitExceeded);
            }
            Ok(())
        }
    }
}

/// An insertion-ordered map from stat identities to base values.
///
/// New blocks are empty. Iteration preserves insertion order: replacement
/// retains its slot, removal preserves the remaining relative order, and
/// re-insertion appends. Equality compares key/value contents, never order.
/// There is no serde implementation; see [`StatBlock::to_serializable`].
///
/// ```compile_fail
/// # use crpg_rules::StatBlock;
/// fn assert_serialize<T: serde::Serialize>(_: T) {}
/// assert_serialize(StatBlock::new());
/// ```
#[derive(Debug, Clone, Default)]
pub struct StatBlock {
    entries: IndexMap<StatId, StatValue>,
}

impl PartialEq for StatBlock {
    fn eq(&self, other: &Self) -> bool {
        self.entries.len() == other.entries.len()
            && self
                .entries
                .iter()
                .all(|(stat, value)| other.entries.get(stat) == Some(value))
    }
}

impl Eq for StatBlock {}

impl StatBlock {
    /// Creates an empty block.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a value after validating its standalone shape and limits.
    ///
    /// Fails with [`RulesErrorCode::LimitExceeded`] at `/stats` when a new
    /// key would exceed [`MAX_STATS`]; replacement at full capacity succeeds.
    /// Fails with the value fault at `/stats/<stat-index>` otherwise. An
    /// invalid insert leaves the block unchanged and returns the previous
    /// value on replacement.
    pub fn insert(
        &mut self,
        stat: StatId,
        value: StatValue,
    ) -> Result<Option<StatValue>, RulesError> {
        if let Err(code) = check_standalone_value(&value) {
            return Err(RulesError::at(code, format!("/stats/{}", stat.index())));
        }
        if !self.entries.contains_key(&stat) && self.entries.len() >= MAX_STATS {
            return Err(RulesError::at(
                RulesErrorCode::LimitExceeded,
                String::from("/stats"),
            ));
        }
        Ok(self.entries.insert(stat, value))
    }

    /// Returns the stored base value for a stat, if present.
    #[must_use]
    pub fn get(&self, stat: StatId) -> Option<&StatValue> {
        self.entries.get(&stat)
    }

    /// Removes a stat, preserving the remaining relative order.
    pub fn remove(&mut self, stat: StatId) -> Option<StatValue> {
        self.entries.shift_remove(&stat)
    }

    /// Iterates entries in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (StatId, &StatValue)> {
        self.entries.iter().map(|(stat, value)| (*stat, value))
    }

    /// Returns the number of stored entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the block holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Converts the block to symbolic strings through the issuing interner.
    ///
    /// Entries sort lexically by stat string and tags lexically by tag
    /// string. Fails with [`RulesErrorCode::UnresolvedHandle`] when a key or
    /// tag has no string in the supplied interner, or
    /// [`RulesErrorCode::InvalidName`] for an empty resolved name.
    ///
    /// The caller must supply the interner that issued the handles. A valid
    /// index in the wrong interner resolves to whatever string sits at that
    /// position there and cannot be detected; this limitation is inherent to
    /// positional handles, not a provenance check this function performs.
    pub fn to_serializable(
        &self,
        interners: &Interners,
    ) -> Result<SerializableStatBlock, RulesError> {
        let mut entries = Vec::with_capacity(self.entries.len());
        for (stat, value) in self.iter() {
            let location = format!("/stats/{}", stat.index());
            let name = interners.resolve_stat(stat).ok_or_else(|| {
                RulesError::at(RulesErrorCode::UnresolvedHandle, location.clone())
            })?;
            if name.is_empty() {
                return Err(RulesError::at(RulesErrorCode::InvalidName, location));
            }
            let converted = match value {
                StatValue::Int(v) => SerializableStatValue::Int(*v),
                StatValue::Fixed(v) => SerializableStatValue::Fixed(*v),
                StatValue::Bool(v) => SerializableStatValue::Bool(*v),
                StatValue::Enum(entry) => SerializableStatValue::Enum(entry.clone()),
                StatValue::Dice(expr) => SerializableStatValue::Dice(expr.to_string()),
                StatValue::Tags(tags) => {
                    let mut names = Vec::with_capacity(tags.len());
                    for tag in tags {
                        let resolved = interners.resolve_tag(*tag).ok_or_else(|| {
                            RulesError::at(RulesErrorCode::UnresolvedHandle, location.clone())
                        })?;
                        if resolved.is_empty() {
                            return Err(RulesError::at(
                                RulesErrorCode::InvalidName,
                                location.clone(),
                            ));
                        }
                        names.push(resolved.to_owned());
                    }
                    names.sort();
                    SerializableStatValue::Tags(names)
                }
            };
            entries.push(SerializableStatEntry {
                stat: name.to_owned(),
                value: converted,
            });
        }
        entries.sort_by(|a, b| a.stat.cmp(&b.stat));
        Ok(SerializableStatBlock { entries })
    }

    /// Rebuilds a block from symbolic strings, interning through `interners`.
    ///
    /// Accepts unsorted entries and tags and normalizes both to lexical
    /// order, interning stat strings in canonical lexical entry order and
    /// tags lexically within each value. Rejects duplicate stat strings with
    /// [`RulesErrorCode::DuplicateStat`] and duplicate tags within one value
    /// with [`RulesErrorCode::DuplicateTag`], both at the second occurrence
    /// in authored array order. Validates every name, value, and limit before
    /// any interner mutation, so a returned error leaves `interners`
    /// unchanged. Dice notation strings are parsed during this preflight, so
    /// a malformed dice value fails before any symbol is interned; parser
    /// paths are prefixed with the entry's persisted path. Enum domain
    /// membership is checked later against pipeline definitions; this
    /// conversion requires only nonempty enum strings.
    pub fn from_serializable(
        value: SerializableStatBlock,
        interners: &mut Interners,
    ) -> Result<Self, RulesError> {
        if value.entries.len() > MAX_STATS {
            return Err(RulesError::at(
                RulesErrorCode::LimitExceeded,
                String::from("/persisted/entries"),
            ));
        }
        let mut seen_stats = BTreeSet::new();
        let mut parsed_dice: Vec<Option<DiceExpr>> = Vec::with_capacity(value.entries.len());
        for (index, entry) in value.entries.iter().enumerate() {
            let location = format!("/persisted/entries/{index}");
            if entry.stat.is_empty() {
                return Err(RulesError::at(RulesErrorCode::InvalidName, location));
            }
            if !seen_stats.insert(entry.stat.as_str()) {
                return Err(RulesError::at(RulesErrorCode::DuplicateStat, location));
            }
            parsed_dice.push(check_persisted_value(&entry.value, &location)?);
        }
        let mut order: Vec<usize> = (0..value.entries.len()).collect();
        order.sort_by(|left, right| value.entries[*left].stat.cmp(&value.entries[*right].stat));
        let mut block = Self::new();
        for position in order {
            let entry = &value.entries[position];
            let mut tags_sorted: Vec<&str> = Vec::new();
            if let SerializableStatValue::Tags(tags) = &entry.value {
                tags_sorted = tags.iter().map(String::as_str).collect();
                tags_sorted.sort();
            }
            let stat = interners.intern_stat(entry.stat.as_str());
            let converted = match &entry.value {
                SerializableStatValue::Int(v) => StatValue::Int(*v),
                SerializableStatValue::Fixed(v) => StatValue::Fixed(*v),
                SerializableStatValue::Bool(v) => StatValue::Bool(*v),
                SerializableStatValue::Enum(entry) => StatValue::Enum(entry.clone()),
                SerializableStatValue::Dice(_) => StatValue::Dice(
                    parsed_dice[position]
                        .clone()
                        .expect("dice preflight parses every dice entry"),
                ),
                SerializableStatValue::Tags(_) => {
                    let mut set = TagSet::new();
                    for name in tags_sorted {
                        set.insert(interners.intern_tag(name));
                    }
                    StatValue::Tags(set)
                }
            };
            // Every name, value, and limit passed above; insertion cannot fail.
            let _ = block.entries.insert(stat, converted);
        }
        Ok(block)
    }
}

/// Validates one persisted value against standalone shape and limits.
///
/// Returns the parsed dice expression for dice values so the caller can
/// reuse the preflight parse instead of decoding twice. Dice parser paths
/// are prefixed with the entry's persisted path.
fn check_persisted_value(
    value: &SerializableStatValue,
    location: &str,
) -> Result<Option<DiceExpr>, RulesError> {
    let value_location = format!("{location}/value");
    match value {
        SerializableStatValue::Int(_)
        | SerializableStatValue::Fixed(_)
        | SerializableStatValue::Bool(_) => Ok(None),
        SerializableStatValue::Dice(notation) => {
            notation
                .parse::<DiceExpr>()
                .map(Some)
                .map_err(|error| RulesError {
                    code: error.code,
                    location: format!("{location}{}", error.location),
                    cycle: error.cycle,
                })
        }
        SerializableStatValue::Enum(entry) => {
            if entry.enum_id.is_empty() || entry.variant.is_empty() {
                return Err(RulesError::at(RulesErrorCode::InvalidName, value_location));
            }
            Ok(None)
        }
        SerializableStatValue::Tags(tags) => {
            if tags.len() > MAX_TAGS {
                return Err(RulesError::at(
                    RulesErrorCode::LimitExceeded,
                    value_location,
                ));
            }
            let mut seen = BTreeSet::new();
            for (child, name) in tags.iter().enumerate() {
                if name.is_empty() {
                    return Err(RulesError::at(
                        RulesErrorCode::InvalidName,
                        format!("{value_location}/{child}"),
                    ));
                }
                if !seen.insert(name.as_str()) {
                    return Err(RulesError::at(
                        RulesErrorCode::DuplicateTag,
                        format!("{value_location}/{child}"),
                    ));
                }
            }
            Ok(None)
        }
    }
}

/// The symbolic persisted form of a [`StatBlock`]: an array of entries.
///
/// The array preserves duplicate-key evidence for validation; converters
/// reject duplicates instead of merging them. Unknown fields are rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SerializableStatBlock {
    /// The persisted entries, sorted lexically on write.
    pub entries: Vec<SerializableStatEntry>,
}

/// One persisted stat entry: its symbolic name and value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SerializableStatEntry {
    /// The symbolic stat name.
    pub stat: String,
    /// The symbolic value.
    pub value: SerializableStatValue,
}

/// The symbolic persisted form of a [`StatValue`].
///
/// Adjacently tagged with lowercase `type`/`value` names; unknown fields are
/// rejected. Fixed values store core's raw integer; enum names and members
/// remain strings; tags are an array of symbolic names; dice values are
/// canonical notation strings parsed during preflight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "lowercase",
    deny_unknown_fields
)]
pub enum SerializableStatValue {
    /// A whole number.
    Int(i32),
    /// Core's raw fixed-point integer.
    Fixed(Fx16_16),
    /// A flag.
    Bool(bool),
    /// An enum domain member by string names.
    Enum(EnumValue),
    /// Symbolic tag names.
    Tags(Vec<String>),
    /// A dice expression in canonical notation.
    Dice(String),
}

/// Sorts and deduplicates direct expression references in ascending order.
pub(crate) fn sorted_expr_refs(expr: &Expr) -> Vec<StatId> {
    let mut refs = Vec::new();
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Literal(_) => {}
            Expr::Stat(stat) => refs.push(*stat),
            Expr::Add(left, right)
            | Expr::Subtract(left, right)
            | Expr::Multiply(left, right)
            | Expr::Divide(left, right)
            | Expr::Min(left, right)
            | Expr::Max(left, right) => {
                stack.push(left);
                stack.push(right);
            }
        }
    }
    refs.sort();
    refs.dedup();
    refs
}

/// Iterative expression shape: node count and tree depth.
///
/// The root counts as depth one. Runs on an explicit stack so measuring never
/// recurses before the depth bound is proven.
pub(crate) fn measure_expr(expr: &Expr) -> (usize, usize) {
    let mut nodes = 0_usize;
    let mut deepest = 0_usize;
    let mut stack = vec![(expr, 1_usize)];
    while let Some((node, depth)) = stack.pop() {
        nodes += 1;
        deepest = deepest.max(depth);
        match node {
            Expr::Literal(_) | Expr::Stat(_) => {}
            Expr::Add(left, right)
            | Expr::Subtract(left, right)
            | Expr::Multiply(left, right)
            | Expr::Divide(left, right)
            | Expr::Min(left, right)
            | Expr::Max(left, right) => {
                stack.push((left, depth + 1));
                stack.push((right, depth + 1));
            }
        }
    }
    (nodes, deepest)
}

/// The largest tag-literal size inside an expression, iteratively.
pub(crate) fn max_tags_literal(expr: &Expr) -> usize {
    let mut largest = 0_usize;
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Literal(StatValue::Tags(tags)) => {
                largest = largest.max(tags.len());
            }
            Expr::Literal(_) | Expr::Stat(_) => {}
            Expr::Add(left, right)
            | Expr::Subtract(left, right)
            | Expr::Multiply(left, right)
            | Expr::Divide(left, right)
            | Expr::Min(left, right)
            | Expr::Max(left, right) => {
                stack.push(left);
                stack.push(right);
            }
        }
    }
    largest
}
