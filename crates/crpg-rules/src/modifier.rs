//! Modifiers, stacking policies, the validated pipeline, and breakdowns.
//!
//! [`ModifierPipeline::new`] validates definitions and policies once, in the
//! contract's fixed phase order. [`ModifierPipeline::query`] borrows one
//! entity's base block, tag set, and caller-owned modifier slice, validates
//! the context phase by phase, then evaluates the requested dependency
//! closure with per-query memoization. Callers own modifier membership:
//! removing a source means filtering its modifiers out and querying again
//! against the same base block.

use std::collections::{btree_map::Entry, BTreeMap, BTreeSet};

use crpg_core::{EntityId, Fx16_16, StatId, TagId, Ulid};

use crate::derived::{
    eval_expr, find_cycle, int_add, int_scale, over_depth_stat, preorder, ExprKind,
};
use crate::error::{RulesError, RulesErrorCode};
use crate::stats::{
    check_standalone_value, max_tags_literal, measure_expr, sorted_expr_refs, Expr, StatBlock,
    StatDefinition, StatKind, StatValue, TagSet, ValueKind,
};
use crate::{
    MAX_CONDITION_DEPTH, MAX_CONDITION_NODES, MAX_ENUM_VARIANTS, MAX_EXPR_DEPTH, MAX_EXPR_NODES,
    MAX_MODIFIERS, MAX_POLICIES, MAX_STATS, MAX_TAGS, MAX_TOTAL_EXPR_NODES,
};

/// Escapes one symbolic path component per RFC 6901 (`~` then `/`).
pub(crate) fn escape_segment(name: &str) -> String {
    name.replace('~', "~0").replace('/', "~1")
}

/// The symbolic name selecting one stacking policy, matched exactly.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModTypeId(pub String);

/// The provenance of one modifier: an opaque kind plus an authored identity.
///
/// Distinct source kinds with the same [`Ulid`] are distinct sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    /// The opaque source kind, nonempty.
    pub kind: String,
    /// The authored source identity.
    pub id: Ulid,
}

/// The stat a modifier applies to, or the roll/DC tag it adjusts.
///
/// `Stat` modifiers require a declared target; `Roll`/`Dc` modifiers adjust
/// an integer the caller supplies (a raw roll total or a defence value) and
/// need no stat declaration. Roll/DC tags wrap caller-issued tag handles;
/// the issuing interner is the caller's responsibility, as with conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierTarget {
    /// A stat on the queried entity.
    Stat(StatId),
    /// A roll total for the tagged roll.
    Roll(RollTag),
    /// A difficulty or defence value for the tagged roll.
    Dc(RollTag),
}

/// The runtime tag identifying one roll for modifier targeting.
///
/// Wraps a caller-issued [`TagId`]; it carries no serde implementation and
/// no declaration. New runtime roll identifiers wrap existing tag handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RollTag(pub TagId);

/// One modifier operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModOp {
    /// Replaces the running value; accepts exactly the target kind.
    Set(StatValue),
    /// Adds a same-kind `Int` or `Fixed` amount with saturation.
    Add(StatValue),
    /// Scales a numeric target by an explicit fixed factor; full-range raw
    /// scaling on `Int`, core multiplication on `Fixed`.
    Multiply(Fx16_16),
    /// Clamps the running value into `[min, max]`; each interval needs
    /// `min <= max`, while separate modifiers may disagree.
    Clamp {
        /// The inclusive lower bound, of the target kind.
        min: StatValue,
        /// The inclusive upper bound, of the target kind.
        max: StatValue,
    },
}

/// A tag condition gating one modifier.
///
/// `None` on the modifier means true. `HasTag` reads only the query's tag
/// set, never a stat's `Tags` value. Empty `All` is true; empty `Any` is
/// false.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionExpr {
    /// True when the query tag set holds the tag.
    HasTag(TagId),
    /// Negation.
    Not(Box<ConditionExpr>),
    /// Conjunction, left to right with short circuit.
    All(Vec<ConditionExpr>),
    /// Disjunction, left to right with short circuit.
    Any(Vec<ConditionExpr>),
}

/// One caller-owned modifier: a stable identity plus its operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Modifier {
    /// The stable caller-supplied identity; arbitrary values are accepted.
    pub id: Ulid,
    /// The provenance used for source-scoped removal.
    pub source: SourceRef,
    /// The stat this modifier applies to.
    pub target: ModifierTarget,
    /// The operation to apply when selected.
    pub op: ModOp,
    /// The symbolic stacking-policy selector, matched exactly.
    pub mod_type: ModTypeId,
    /// The stacking name; required under `HighestPriorityPerName`.
    pub name: Option<String>,
    /// The tag gate; `None` means true.
    pub condition: Option<ConditionExpr>,
    /// Selection and application order: higher runs later.
    pub priority: i16,
}

/// The data-selected stacking policy for one modifier type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackingPolicy {
    /// Keeps every true-condition candidate.
    StackAll,
    /// `Add` only: keeps the largest strictly positive and the smallest
    /// strictly negative amount independently; zeros are `ZeroAdd`.
    HighestBonusWorstPenalty,
    /// Keeps the greatest `(priority, id)` per exact name in each phase.
    HighestPriorityPerName,
}

/// The borrowed per-entity view for exactly one query.
///
/// All modifiers in the slice belong to [`QueryContext::entity`]; rules
/// checks equality with the queried entity, never arena liveness or
/// authority.
#[derive(Debug, Clone, Copy)]
pub struct QueryContext<'a> {
    /// The entity this view describes.
    pub entity: EntityId,
    /// The stored base values.
    pub stats: &'a StatBlock,
    /// The tag set conditions read.
    pub tags: &'a TagSet,
    /// The caller-owned active modifiers for this entity.
    pub modifiers: &'a [Modifier],
}

/// Why one contribution did or did not change the running value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContributionStatus {
    /// The modifier's condition evaluated to false.
    ConditionFalse,
    /// A zero `Add` under `HighestBonusWorstPenalty`; no winner applies.
    ZeroAdd,
    /// A stacking loser; names the selected same-sign or same-name winner.
    Suppressed {
        /// The winning modifier identity.
        winner: Ulid,
    },
    /// The modifier ran, with the running value before and after.
    Applied {
        /// The running value before this modifier.
        before: StatValue,
        /// The running value after this modifier.
        after: StatValue,
    },
}

/// One modifier plus its outcome for one stat's trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contribution {
    /// The supplied modifier.
    pub modifier: Modifier,
    /// Its outcome.
    pub status: ContributionStatus,
}

/// The complete computation record for one evaluated stat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatTrace {
    /// The stored base, or the evaluated derived expression, before modifiers.
    pub base: StatValue,
    /// The fully modified value; the query returns the root's value.
    pub value: StatValue,
    /// The sorted, deduplicated direct references in the formula.
    pub dependencies: Vec<StatId>,
    /// Every supplied modifier targeting this stat, ordered by phase then
    /// ascending `(priority, id)`, including excluded contributions.
    pub contributions: Vec<Contribution>,
}

/// The complete breakdown for one successful query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModifierBreakdown {
    /// The queried entity.
    pub entity: EntityId,
    /// The requested stat.
    pub stat: StatId,
    /// One trace per transitively evaluated stat; unqueried stats excluded.
    pub traces: BTreeMap<StatId, StatTrace>,
}

/// The complete computation record for one evaluated integer target.
///
/// Returned by [`ModifierPipeline::query_numeric`]: the supplied base, the
/// folded value, and every target-matching modifier in canonical
/// phase/`(priority, id)` order, including excluded contributions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumericModifierBreakdown {
    /// The queried entity.
    pub entity: EntityId,
    /// The roll/DC target that was folded.
    pub target: ModifierTarget,
    /// The caller-supplied base the fold started from.
    pub base: i32,
    /// The fully modified value.
    pub value: i32,
    /// Every supplied modifier targeting this target, ordered by phase then
    /// ascending `(priority, id)`, including excluded contributions.
    pub contributions: Vec<Contribution>,
}

/// The immutable validated definitions and stacking policies for queries.
#[derive(Debug, Clone)]
pub struct ModifierPipeline {
    definitions: BTreeMap<StatId, StatDefinition>,
    policies: BTreeMap<ModTypeId, StackingPolicy>,
}

/// The application phase of one operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Set,
    Add,
    Multiply,
    Clamp,
}

fn phase_of(op: &ModOp) -> Phase {
    match op {
        ModOp::Set(_) => Phase::Set,
        ModOp::Add(_) => Phase::Add,
        ModOp::Multiply(_) => Phase::Multiply,
        ModOp::Clamp { .. } => Phase::Clamp,
    }
}

/// Iterative condition shape: node count and tree depth, root as depth one.
fn measure_condition(condition: &ConditionExpr) -> (usize, usize) {
    let mut nodes = 0_usize;
    let mut deepest = 0_usize;
    let mut stack = vec![(condition, 1_usize)];
    while let Some((node, depth)) = stack.pop() {
        nodes += 1;
        deepest = deepest.max(depth);
        match node {
            ConditionExpr::HasTag(_) => {}
            ConditionExpr::Not(child) => stack.push((child, depth + 1)),
            ConditionExpr::All(children) | ConditionExpr::Any(children) => {
                for child in children {
                    stack.push((child, depth + 1));
                }
            }
        }
    }
    (nodes, deepest)
}

/// Evaluates a validated condition against the query tag set.
fn eval_condition(condition: &ConditionExpr, tags: &TagSet) -> bool {
    match condition {
        ConditionExpr::HasTag(tag) => tags.contains(tag),
        ConditionExpr::Not(child) => !eval_condition(child, tags),
        ConditionExpr::All(children) => children.iter().all(|child| eval_condition(child, tags)),
        ConditionExpr::Any(children) => children.iter().any(|child| eval_condition(child, tags)),
    }
}

/// The signed amount of an `Add` operand for bonus/penalty selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Amount {
    Int(i32),
    Fixed(Fx16_16),
}

fn add_amount(value: &StatValue) -> Option<Amount> {
    match value {
        StatValue::Int(v) => Some(Amount::Int(*v)),
        StatValue::Fixed(v) => Some(Amount::Fixed(*v)),
        _ => None,
    }
}

fn amount_sign(amount: Amount) -> i8 {
    match amount {
        Amount::Int(v) => v.cmp(&0) as i8,
        Amount::Fixed(v) => {
            if v == Fx16_16::ZERO {
                0
            } else if v.is_negative() {
                -1
            } else {
                1
            }
        }
    }
}

/// Compares two same-kind amounts numerically.
fn compare_amount(left: Amount, right: Amount) -> std::cmp::Ordering {
    match (left, right) {
        (Amount::Int(a), Amount::Int(b)) => a.cmp(&b),
        (Amount::Fixed(a), Amount::Fixed(b)) => a.cmp(&b),
        _ => std::cmp::Ordering::Equal,
    }
}

impl ModifierPipeline {
    /// Validates definitions and policies into an immutable pipeline.
    ///
    /// Validates limits, duplicate ids, declaration/domain/policy shape,
    /// references, cycles and depth, then expression types, in that order.
    /// Roots traverse in ascending [`StatId`] order and policy names
    /// lexically; within one object, declaration field order and expression
    /// preorder decide. Accepts forward references and an empty set.
    pub fn new(
        definitions: Vec<StatDefinition>,
        policies: BTreeMap<ModTypeId, StackingPolicy>,
    ) -> Result<Self, RulesError> {
        if definitions.len() > MAX_STATS {
            return Err(RulesError::at(
                RulesErrorCode::LimitExceeded,
                String::from("/definitions"),
            ));
        }
        if policies.len() > MAX_POLICIES {
            return Err(RulesError::at(
                RulesErrorCode::LimitExceeded,
                String::from("/policies"),
            ));
        }
        let mut ordered: Vec<&StatDefinition> = definitions.iter().collect();
        ordered.sort_by_key(|definition| definition.id);
        // Per-definition expression limits before any other object check.
        let mut total_nodes = 0_usize;
        for definition in &ordered {
            let location = format!("/definitions/{}/derived", definition.id.index());
            if let Some(expr) = &definition.derived {
                let (nodes, depth) = measure_expr(expr);
                if nodes > MAX_EXPR_NODES || depth > MAX_EXPR_DEPTH {
                    return Err(RulesError::at(RulesErrorCode::LimitExceeded, location));
                }
                if max_tags_literal(expr) > MAX_TAGS {
                    return Err(RulesError::at(RulesErrorCode::LimitExceeded, location));
                }
                total_nodes += nodes;
            }
            if let StatKind::Enum { variants, .. } = &definition.kind {
                if variants.is_empty() || variants.len() > MAX_ENUM_VARIANTS {
                    return Err(RulesError::at(
                        RulesErrorCode::LimitExceeded,
                        format!("/definitions/{}/kind", definition.id.index()),
                    ));
                }
            }
        }
        if total_nodes > MAX_TOTAL_EXPR_NODES {
            return Err(RulesError::at(
                RulesErrorCode::LimitExceeded,
                String::from("/definitions"),
            ));
        }
        // Duplicate stat ids: the smallest duplicated id decides.
        let mut ids: Vec<StatId> = ordered.iter().map(|definition| definition.id).collect();
        ids.sort();
        let mut duplicate: Option<StatId> = None;
        for pair in ids.windows(2) {
            if pair[0] == pair[1] {
                duplicate = Some(pair[0]);
                break;
            }
        }
        if let Some(stat) = duplicate {
            return Err(RulesError::at(
                RulesErrorCode::DuplicateStat,
                format!("/definitions/{}", stat.index()),
            ));
        }
        let table: BTreeMap<StatId, &StatDefinition> = ordered
            .iter()
            .map(|definition| (definition.id, *definition))
            .collect();
        // Declaration, domain, and policy shape.
        let mut domains: BTreeMap<&str, &BTreeSet<String>> = BTreeMap::new();
        for definition in &ordered {
            let location = format!("/definitions/{}/kind", definition.id.index());
            if let StatKind::Enum { enum_id, variants } = &definition.kind {
                if enum_id.is_empty() {
                    return Err(RulesError::at(RulesErrorCode::InvalidName, location));
                }
                for variant in variants {
                    if variant.is_empty() {
                        return Err(RulesError::at(
                            RulesErrorCode::InvalidName,
                            location.clone(),
                        ));
                    }
                }
                match domains.get(enum_id.as_str()) {
                    Some(declared) if *declared != variants => {
                        return Err(RulesError::at(RulesErrorCode::InvalidEnum, location));
                    }
                    Some(_) => {}
                    None => {
                        domains.insert(enum_id.as_str(), variants);
                    }
                }
            }
        }
        for name in policies.keys() {
            if name.0.is_empty() {
                return Err(RulesError::at(
                    RulesErrorCode::InvalidName,
                    format!("/policies/{}", escape_segment(&name.0)),
                ));
            }
        }
        // References and enum literal domains/members, in preorder.
        for definition in &ordered {
            let location = format!("/definitions/{}/derived", definition.id.index());
            if let Some(expr) = &definition.derived {
                let mut fault: Option<RulesError> = None;
                crate::derived::preorder(expr, &mut |node| {
                    if fault.is_some() {
                        return;
                    }
                    match node {
                        Expr::Stat(stat) => {
                            if !table.contains_key(stat) {
                                fault = Some(RulesError::at(
                                    RulesErrorCode::UnknownStat,
                                    location.clone(),
                                ));
                            }
                        }
                        Expr::Literal(StatValue::Enum(entry)) => {
                            if entry.enum_id.is_empty() || entry.variant.is_empty() {
                                fault = Some(RulesError::at(
                                    RulesErrorCode::InvalidName,
                                    location.clone(),
                                ));
                            } else {
                                match domains.get(entry.enum_id.as_str()) {
                                    Some(variants) if variants.contains(&entry.variant) => {}
                                    _ => {
                                        fault = Some(RulesError::at(
                                            RulesErrorCode::InvalidEnum,
                                            location.clone(),
                                        ));
                                    }
                                }
                            }
                        }
                        Expr::Literal(_) => {}
                        Expr::Add(_, _)
                        | Expr::Subtract(_, _)
                        | Expr::Multiply(_, _)
                        | Expr::Divide(_, _)
                        | Expr::Min(_, _)
                        | Expr::Max(_, _) => {}
                    }
                });
                if let Some(error) = fault {
                    return Err(error);
                }
            }
        }
        // Cycles, then acyclic depth.
        let mut adjacency: BTreeMap<StatId, Vec<StatId>> = BTreeMap::new();
        for definition in &ordered {
            let refs = match &definition.derived {
                Some(expr) => sorted_expr_refs(expr),
                None => Vec::new(),
            };
            adjacency.insert(definition.id, refs);
        }
        if let Some(cycle) = find_cycle(&adjacency) {
            let smallest = cycle[0];
            return Err(RulesError::cycle(
                format!("/definitions/{}/derived", smallest.index()),
                cycle,
            ));
        }
        if let Some(stat) = over_depth_stat(&adjacency) {
            return Err(RulesError::at(
                RulesErrorCode::LimitExceeded,
                format!("/definitions/{}/derived", stat.index()),
            ));
        }
        // Expression types in preorder, then the final-kind match.
        let owned_domains: BTreeMap<String, BTreeSet<String>> = domains
            .iter()
            .map(|(id, variants)| ((*id).to_owned(), (*variants).clone()))
            .collect();
        for definition in &ordered {
            let location = format!("/definitions/{}/derived", definition.id.index());
            if let Some(expr) = &definition.derived {
                check_expr_types(expr, &table, &owned_domains, &location)?;
                let computed = expr_kind(expr, &table, &owned_domains)
                    .map_err(|()| RulesError::at(RulesErrorCode::TypeMismatch, location.clone()))?;
                if !computed.matches_declaration(&definition.kind) {
                    return Err(RulesError::at(RulesErrorCode::TypeMismatch, location));
                }
            }
        }
        let definitions = table
            .into_iter()
            .map(|(id, definition)| (id, (*definition).clone()))
            .collect();
        Ok(Self {
            definitions,
            policies,
        })
    }

    /// Queries one stat for one entity against a borrowed context.
    ///
    /// Validates entity equality, stat existence, context limits, stored
    /// entries by ascending id, modifier id uniqueness, then every modifier
    /// by ascending id, before evaluating the dependency closure. Returns the
    /// fully modified value with its complete breakdown; errors mutate
    /// nothing.
    pub fn query(
        &self,
        entity: EntityId,
        stat: StatId,
        context: &QueryContext<'_>,
    ) -> Result<(StatValue, ModifierBreakdown), RulesError> {
        if context.entity != entity {
            return Err(RulesError::at(
                RulesErrorCode::EntityMismatch,
                String::from("/context/entity"),
            ));
        }
        if !self.definitions.contains_key(&stat) {
            return Err(RulesError::at(
                RulesErrorCode::UnknownStat,
                format!("/stats/{}", stat.index()),
            ));
        }
        self.check_context(context)?;
        let mut evaluation = Evaluation {
            pipeline: self,
            context,
            traces: BTreeMap::new(),
        };
        let value = evaluation.eval_stat(stat)?;
        Ok((
            value,
            ModifierBreakdown {
                entity,
                stat,
                traces: evaluation.traces,
            },
        ))
    }

    /// Folds roll/DC modifiers onto a caller-supplied integer base.
    ///
    /// Accepts only [`ModifierTarget::Roll`]/[`ModifierTarget::Dc`],
    /// rejecting [`ModifierTarget::Stat`] with
    /// [`RulesErrorCode::InvalidTarget`] at `/target`. Checks entity, then
    /// target kind, then the entire supplied context with the same phases
    /// [`ModifierPipeline::query`] uses. Applies modifiers to the supplied
    /// base, never to a fabricated stat, reusing the shared
    /// selection/folding helpers so stacking has one implementation.
    /// `Set`/`Add`/`Clamp` require `Int` operands; `Multiply` uses the
    /// full-range fixed scaling. Contributions retain all target-matching
    /// modifiers, including excluded ones, in phase/`(priority, id)` order.
    pub fn query_numeric(
        &self,
        entity: EntityId,
        target: ModifierTarget,
        base: i32,
        context: &QueryContext<'_>,
    ) -> Result<(i32, NumericModifierBreakdown), RulesError> {
        if context.entity != entity {
            return Err(RulesError::at(
                RulesErrorCode::EntityMismatch,
                String::from("/context/entity"),
            ));
        }
        if matches!(target, ModifierTarget::Stat(_)) {
            return Err(RulesError::at(
                RulesErrorCode::InvalidTarget,
                String::from("/target"),
            ));
        }
        self.check_context(context)?;
        let mut targeting: Vec<&Modifier> = context
            .modifiers
            .iter()
            .filter(|modifier| modifier.target == target)
            .collect();
        targeting.sort_by_key(|modifier| (phase_of(&modifier.op), modifier.priority, modifier.id));
        let (value, contributions) =
            fold_numeric(targeting.as_slice(), base, context.tags, &self.policies);
        Ok((
            value,
            NumericModifierBreakdown {
                entity,
                target,
                base,
                value,
                contributions,
            },
        ))
    }

    /// Validates one borrowed context in the contract's fixed phase order:
    /// top-level context limits, all stored base entries by ascending id,
    /// modifier id uniqueness, then every modifier by ascending id.
    /// Entity equality and target/stat existence stay with the callers so
    /// each entry point keeps its specified check order. Resolution reuses
    /// this to validate each unique participant once, prefixing failures
    /// with the participant path.
    pub(crate) fn check_context(&self, context: &QueryContext<'_>) -> Result<(), RulesError> {
        if context.stats.len() > MAX_STATS {
            return Err(RulesError::at(
                RulesErrorCode::LimitExceeded,
                String::from("/stats"),
            ));
        }
        if context.tags.len() > MAX_TAGS {
            return Err(RulesError::at(
                RulesErrorCode::LimitExceeded,
                String::from("/context"),
            ));
        }
        if context.modifiers.len() > MAX_MODIFIERS {
            return Err(RulesError::at(
                RulesErrorCode::LimitExceeded,
                String::from("/modifiers"),
            ));
        }
        let mut stored: Vec<(StatId, &StatValue)> = context.stats.iter().collect();
        stored.sort_by_key(|(stat, _)| *stat);
        for (id, value) in &stored {
            let location = format!("/stats/{}", id.index());
            let definition = self
                .definitions
                .get(id)
                .ok_or_else(|| RulesError::at(RulesErrorCode::UnknownStat, location.clone()))?;
            if definition.derived.is_some() {
                return Err(RulesError::at(RulesErrorCode::DerivedBase, location));
            }
            check_stored_kind(value, &definition.kind, &location)?;
        }
        let mut modifier_ids: Vec<Ulid> = context
            .modifiers
            .iter()
            .map(|modifier| modifier.id)
            .collect();
        modifier_ids.sort();
        for pair in modifier_ids.windows(2) {
            if pair[0] == pair[1] {
                return Err(RulesError::at(
                    RulesErrorCode::DuplicateModifier,
                    format!("/modifiers/{pair}", pair = pair[0]),
                ));
            }
        }
        let mut ordered: Vec<&Modifier> = context.modifiers.iter().collect();
        ordered.sort_by_key(|modifier| modifier.id);
        for modifier in &ordered {
            self.check_modifier(modifier)?;
        }
        Ok(())
    }

    /// Validates one modifier against its target declaration and policy.
    ///
    /// Stat targets require a declaration; roll/DC targets need none and
    /// validate as integer targets. Operation support is checked before
    /// operand kinds: `Add` on `Bool` is `InvalidOperation`, while
    /// `Add(Fixed)` on `Int` is `TypeMismatch`. Dice targets accept `Set`
    /// only through this same ordering.
    fn check_modifier(&self, modifier: &Modifier) -> Result<(), RulesError> {
        let root = format!("/modifiers/{}", modifier.id);
        let target_kind: StatKind = match modifier.target {
            ModifierTarget::Stat(target) => self
                .definitions
                .get(&target)
                .ok_or_else(|| {
                    RulesError::at(RulesErrorCode::UnknownStat, format!("{root}/target"))
                })?
                .kind
                .clone(),
            ModifierTarget::Roll(_) | ModifierTarget::Dc(_) => StatKind::Int,
        };
        if modifier.mod_type.0.is_empty() {
            return Err(RulesError::at(
                RulesErrorCode::InvalidName,
                format!("{root}/mod_type"),
            ));
        }
        let policy = self.policies.get(&modifier.mod_type).ok_or_else(|| {
            RulesError::at(
                RulesErrorCode::UnknownModifierType,
                format!("{root}/mod_type"),
            )
        })?;
        if modifier.source.kind.is_empty() {
            return Err(RulesError::at(
                RulesErrorCode::InvalidName,
                format!("{root}/source"),
            ));
        }
        if let Some(name) = &modifier.name {
            if name.is_empty() {
                return Err(RulesError::at(
                    RulesErrorCode::InvalidName,
                    format!("{root}/name"),
                ));
            }
        }
        // Nested value and tree limits precede traversal of those objects.
        match &modifier.op {
            ModOp::Set(value) | ModOp::Add(value) => {
                if let Err(code) = check_standalone_value(value) {
                    return Err(RulesError::at(code, format!("{root}/op")));
                }
            }
            ModOp::Multiply(_) => {}
            ModOp::Clamp { min, max } => {
                if let Err(code) = check_standalone_value(min) {
                    return Err(RulesError::at(code, format!("{root}/op")));
                }
                if let Err(code) = check_standalone_value(max) {
                    return Err(RulesError::at(code, format!("{root}/op")));
                }
            }
        }
        if let Some(condition) = &modifier.condition {
            let (nodes, depth) = measure_condition(condition);
            if nodes > MAX_CONDITION_NODES || depth > MAX_CONDITION_DEPTH {
                return Err(RulesError::at(
                    RulesErrorCode::LimitExceeded,
                    format!("{root}/condition"),
                ));
            }
        }
        // Operation support precedes operand-kind checks.
        let numeric = matches!(target_kind, StatKind::Int | StatKind::Fixed);
        let supported = match &modifier.op {
            ModOp::Set(_) => true,
            ModOp::Add(_) | ModOp::Multiply(_) | ModOp::Clamp { .. } => numeric,
        };
        if !supported {
            return Err(RulesError::at(
                RulesErrorCode::InvalidOperation,
                format!("{root}/op"),
            ));
        }
        match &modifier.op {
            ModOp::Set(value) => check_modifier_value(value, &target_kind, &format!("{root}/op"))?,
            ModOp::Add(value) => {
                let same = matches!(
                    (&target_kind, value),
                    (StatKind::Int, StatValue::Int(_)) | (StatKind::Fixed, StatValue::Fixed(_))
                );
                if !same {
                    return Err(RulesError::at(
                        RulesErrorCode::TypeMismatch,
                        format!("{root}/op"),
                    ));
                }
            }
            ModOp::Multiply(_) => {}
            ModOp::Clamp { min, max } => {
                check_modifier_value(min, &target_kind, &format!("{root}/op"))?;
                check_modifier_value(max, &target_kind, &format!("{root}/op"))?;
                let ordered = match (min, max) {
                    (StatValue::Int(a), StatValue::Int(b)) => a <= b,
                    (StatValue::Fixed(a), StatValue::Fixed(b)) => a <= b,
                    _ => false,
                };
                if !ordered {
                    return Err(RulesError::at(
                        RulesErrorCode::InvalidClamp,
                        format!("{root}/op"),
                    ));
                }
            }
        }
        match policy {
            StackingPolicy::StackAll => {}
            StackingPolicy::HighestBonusWorstPenalty => {
                if !matches!(modifier.op, ModOp::Add(_)) {
                    return Err(RulesError::at(
                        RulesErrorCode::InvalidOperation,
                        format!("{root}/op"),
                    ));
                }
            }
            StackingPolicy::HighestPriorityPerName => {
                if modifier.name.is_none() {
                    return Err(RulesError::at(
                        RulesErrorCode::InvalidOperation,
                        format!("{root}/name"),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Computes the kind of one expression, or `Err(())` when a reference is
/// unknown, an enum literal names no declared domain, or a binary operator
/// mixes kinds. Reference and literal validity precede this phase, so the
/// caller maps the failure to [`RulesErrorCode::TypeMismatch`].
fn expr_kind(
    expr: &Expr,
    table: &BTreeMap<StatId, &StatDefinition>,
    domains: &BTreeMap<String, BTreeSet<String>>,
) -> Result<ExprKind, ()> {
    match expr {
        Expr::Literal(value) => match value {
            StatValue::Int(_) => Ok(ExprKind::Int),
            StatValue::Fixed(_) => Ok(ExprKind::Fixed),
            StatValue::Bool(_) => Ok(ExprKind::Bool),
            StatValue::Tags(_) => Ok(ExprKind::Tags),
            StatValue::Dice(_) => Ok(ExprKind::Dice),
            StatValue::Enum(entry) => domains
                .get(&entry.enum_id)
                .map(|variants| ExprKind::Enum(entry.enum_id.clone(), variants.clone()))
                .ok_or(()),
        },
        Expr::Stat(stat) => {
            let definition = table.get(stat).ok_or(())?;
            match &definition.kind {
                StatKind::Int => Ok(ExprKind::Int),
                StatKind::Fixed => Ok(ExprKind::Fixed),
                StatKind::Bool => Ok(ExprKind::Bool),
                StatKind::Tags => Ok(ExprKind::Tags),
                StatKind::Dice => Ok(ExprKind::Dice),
                StatKind::Enum { enum_id, variants } => {
                    Ok(ExprKind::Enum(enum_id.clone(), variants.clone()))
                }
            }
        }
        Expr::Add(left, right)
        | Expr::Subtract(left, right)
        | Expr::Multiply(left, right)
        | Expr::Divide(left, right)
        | Expr::Min(left, right)
        | Expr::Max(left, right) => {
            let first = expr_kind(left, table, domains)?;
            let second = expr_kind(right, table, domains)?;
            match (first.discriminant(), second.discriminant()) {
                (ValueKind::Int, ValueKind::Int) => Ok(ExprKind::Int),
                (ValueKind::Fixed, ValueKind::Fixed) => Ok(ExprKind::Fixed),
                _ => Err(()),
            }
        }
    }
}

/// Checks every binary operator in preorder: both operands must be `Int` or
/// both `Fixed`, with that same result kind.
fn check_expr_types(
    expr: &Expr,
    table: &BTreeMap<StatId, &StatDefinition>,
    domains: &BTreeMap<String, BTreeSet<String>>,
    location: &str,
) -> Result<(), RulesError> {
    let mut fault: Option<RulesError> = None;
    preorder(expr, &mut |node| {
        if fault.is_some() {
            return;
        }
        let (left, right) = match node {
            Expr::Add(left, right)
            | Expr::Subtract(left, right)
            | Expr::Multiply(left, right)
            | Expr::Divide(left, right)
            | Expr::Min(left, right)
            | Expr::Max(left, right) => (left, right),
            Expr::Literal(_) | Expr::Stat(_) => return,
        };
        let numeric = |side: &Expr| -> Option<ValueKind> {
            match expr_kind(side, table, domains) {
                Ok(kind) => match kind.discriminant() {
                    ValueKind::Int | ValueKind::Fixed => Some(kind.discriminant()),
                    _ => None,
                },
                Err(()) => None,
            }
        };
        match (numeric(left), numeric(right)) {
            (Some(ValueKind::Int), Some(ValueKind::Int))
            | (Some(ValueKind::Fixed), Some(ValueKind::Fixed)) => {}
            _ => {
                fault = Some(RulesError::at(
                    RulesErrorCode::TypeMismatch,
                    location.to_owned(),
                ));
            }
        }
    });
    match fault {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Checks a stored base entry against its declaration.
fn check_stored_kind(value: &StatValue, kind: &StatKind, location: &str) -> Result<(), RulesError> {
    if value.kind() != kind.kind() {
        return Err(RulesError::at(
            RulesErrorCode::TypeMismatch,
            location.to_owned(),
        ));
    }
    if let (StatValue::Enum(entry), StatKind::Enum { enum_id, variants }) = (value, kind) {
        if &entry.enum_id != enum_id || !variants.contains(&entry.variant) {
            return Err(RulesError::at(
                RulesErrorCode::InvalidEnum,
                location.to_owned(),
            ));
        }
    }
    Ok(())
}

/// Checks a modifier operand against its target declaration.
fn check_modifier_value(
    value: &StatValue,
    kind: &StatKind,
    location: &str,
) -> Result<(), RulesError> {
    if value.kind() != kind.kind() {
        return Err(RulesError::at(
            RulesErrorCode::TypeMismatch,
            location.to_owned(),
        ));
    }
    if let (StatValue::Enum(entry), StatKind::Enum { enum_id, variants }) = (value, kind) {
        if &entry.enum_id != enum_id || !variants.contains(&entry.variant) {
            return Err(RulesError::at(
                RulesErrorCode::InvalidEnum,
                location.to_owned(),
            ));
        }
    }
    Ok(())
}

/// One query's memoized evaluation state.
struct Evaluation<'a, 'b> {
    pipeline: &'a ModifierPipeline,
    context: &'b QueryContext<'b>,
    traces: BTreeMap<StatId, StatTrace>,
}

impl Evaluation<'_, '_> {
    /// Evaluates one stat's fully modified value, memoizing its trace.
    fn eval_stat(&mut self, stat: StatId) -> Result<StatValue, RulesError> {
        if let Some(trace) = self.traces.get(&stat) {
            return Ok(trace.value.clone());
        }
        let location = format!("/stats/{}", stat.index());
        let definition = self
            .pipeline
            .definitions
            .get(&stat)
            .ok_or_else(|| RulesError::at(RulesErrorCode::UnknownStat, location.clone()))?;
        let (base, dependencies) =
            match &definition.derived {
                Some(expr) => {
                    let refs = sorted_expr_refs(expr);
                    let mut pending: Vec<StatId> = refs.clone();
                    let mut resolved: BTreeMap<StatId, StatValue> = BTreeMap::new();
                    while let Some(reference) = pending.pop() {
                        if let Entry::Vacant(slot) = resolved.entry(reference) {
                            slot.insert(self.eval_stat(reference)?);
                        }
                    }
                    let mut lookup = |id: StatId| -> Result<StatValue, RulesError> {
                        resolved.get(&id).cloned().ok_or_else(|| {
                            RulesError::at(RulesErrorCode::UnknownStat, location.clone())
                        })
                    };
                    let value = eval_expr(expr, &mut lookup, &location)?;
                    (value, refs)
                }
                None => (
                    self.context.stats.get(stat).cloned().ok_or_else(|| {
                        RulesError::at(RulesErrorCode::MissingBase, location.clone())
                    })?,
                    Vec::new(),
                ),
            };
        let mut targeting: Vec<&Modifier> = self
            .context
            .modifiers
            .iter()
            .filter(|modifier| matches!(modifier.target, ModifierTarget::Stat(target) if target == stat))
            .collect();
        targeting.sort_by_key(|modifier| (phase_of(&modifier.op), modifier.priority, modifier.id));
        // False conditions filter first and take precedence over stacking.
        let mut live: Vec<&Modifier> = Vec::new();
        let mut status: BTreeMap<Ulid, ContributionStatus> = BTreeMap::new();
        for modifier in &targeting {
            let active = match &modifier.condition {
                None => true,
                Some(condition) => eval_condition(condition, self.context.tags),
            };
            if active {
                live.push(*modifier);
            } else {
                status.insert(modifier.id, ContributionStatus::ConditionFalse);
            }
        }
        // Group remaining candidates by type and phase; phases never
        // suppress each other across operation kinds.
        let mut groups: BTreeMap<(String, Phase), Vec<&Modifier>> = BTreeMap::new();
        for modifier in live {
            groups
                .entry((modifier.mod_type.0.clone(), phase_of(&modifier.op)))
                .or_default()
                .push(modifier);
        }
        let mut retained: BTreeSet<(Phase, i16, Ulid)> = BTreeSet::new();
        for ((type_name, _), members) in &groups {
            let policy = self
                .pipeline
                .policies
                .get(&ModTypeId(type_name.clone()))
                .copied()
                .unwrap_or(StackingPolicy::StackAll);
            select_group(policy, members, &mut retained, &mut status);
        }
        // Apply retained operations in canonical global order.
        let mut running = base.clone();
        let mut applied: BTreeMap<Ulid, (StatValue, StatValue)> = BTreeMap::new();
        let mut order: Vec<&Modifier> = targeting
            .iter()
            .filter(|modifier| {
                retained.contains(&(phase_of(&modifier.op), modifier.priority, modifier.id))
            })
            .copied()
            .collect();
        order.sort_by_key(|modifier| (phase_of(&modifier.op), modifier.priority, modifier.id));
        for modifier in order {
            let before = running.clone();
            running = apply_modifier(&running, &modifier.op);
            applied.insert(modifier.id, (before, running.clone()));
        }
        for modifier in &targeting {
            if status.contains_key(&modifier.id) {
                continue;
            }
            match applied.get(&modifier.id) {
                Some((before, after)) => {
                    status.insert(
                        modifier.id,
                        ContributionStatus::Applied {
                            before: before.clone(),
                            after: after.clone(),
                        },
                    );
                }
                None => {
                    // Stacking losers always record their winner in selection.
                    debug_assert!(false, "retained modifier without applied record");
                }
            }
        }
        let contributions = targeting
            .iter()
            .map(|modifier| Contribution {
                modifier: (*modifier).clone(),
                status: status
                    .get(&modifier.id)
                    .cloned()
                    .unwrap_or(ContributionStatus::ConditionFalse),
            })
            .collect();
        let value = running;
        self.traces.insert(
            stat,
            StatTrace {
                base,
                value: value.clone(),
                dependencies,
                contributions,
            },
        );
        Ok(value)
    }
}

/// Selects one stacking group, recording winners and losers.
fn select_group(
    policy: StackingPolicy,
    members: &[&Modifier],
    retained: &mut BTreeSet<(Phase, i16, Ulid)>,
    status: &mut BTreeMap<Ulid, ContributionStatus>,
) {
    let key = |modifier: &&Modifier| (phase_of(&modifier.op), modifier.priority, modifier.id);
    match policy {
        StackingPolicy::StackAll => {
            for modifier in members {
                retained.insert(key(modifier));
            }
        }
        StackingPolicy::HighestBonusWorstPenalty => {
            let mut positives: Vec<(&Modifier, Amount)> = Vec::new();
            let mut negatives: Vec<(&Modifier, Amount)> = Vec::new();
            for modifier in members {
                let ModOp::Add(value) = &modifier.op else {
                    continue;
                };
                let Some(amount) = add_amount(value) else {
                    continue;
                };
                match amount_sign(amount) {
                    0 => {
                        status.insert(modifier.id, ContributionStatus::ZeroAdd);
                    }
                    1 => positives.push((*modifier, amount)),
                    _ => negatives.push((*modifier, amount)),
                }
            }
            select_side(&positives, true, retained, status);
            select_side(&negatives, false, retained, status);
        }
        StackingPolicy::HighestPriorityPerName => {
            let mut names: BTreeMap<&str, Vec<&Modifier>> = BTreeMap::new();
            for modifier in members {
                if let Some(name) = &modifier.name {
                    names.entry(name.as_str()).or_default().push(*modifier);
                }
            }
            for members in names.values() {
                let winner = members
                    .iter()
                    .max_by_key(|modifier| (modifier.priority, modifier.id));
                if let Some(winner) = winner {
                    retained.insert(key(winner));
                    for modifier in members {
                        if modifier.id != winner.id {
                            status.insert(
                                modifier.id,
                                ContributionStatus::Suppressed { winner: winner.id },
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Selects one bonus/penalty side: extreme amount wins, ties prefer the
/// greatest `(priority, id)`.
fn select_side(
    side: &[(&Modifier, Amount)],
    positive: bool,
    retained: &mut BTreeSet<(Phase, i16, Ulid)>,
    status: &mut BTreeMap<Ulid, ContributionStatus>,
) {
    let mut best: Option<(&Modifier, Amount)> = None;
    for (modifier, amount) in side {
        let replace = match &best {
            None => true,
            Some((current, current_amount)) => {
                let ordering = compare_amount(*amount, *current_amount);
                if positive {
                    ordering == std::cmp::Ordering::Greater
                        || (ordering == std::cmp::Ordering::Equal
                            && (modifier.priority, modifier.id) > (current.priority, current.id))
                } else {
                    ordering == std::cmp::Ordering::Less
                        || (ordering == std::cmp::Ordering::Equal
                            && (modifier.priority, modifier.id) > (current.priority, current.id))
                }
            }
        };
        if replace {
            best = Some((*modifier, *amount));
        }
    }
    if let Some((winner, _)) = best {
        retained.insert((phase_of(&winner.op), winner.priority, winner.id));
        for (modifier, _) in side {
            if modifier.id != winner.id {
                status.insert(
                    modifier.id,
                    ContributionStatus::Suppressed { winner: winner.id },
                );
            }
        }
    }
}

/// Applies one retained operation to an integer running value.
///
/// Validation guarantees `Int` operands on integer targets, so any other
/// shape keeps the running value instead of panicking.
fn apply_numeric_modifier(running: i32, op: &ModOp) -> i32 {
    match op {
        ModOp::Set(StatValue::Int(value)) => *value,
        ModOp::Add(StatValue::Int(amount)) => int_add(running, *amount),
        ModOp::Multiply(factor) => int_scale(running, *factor),
        ModOp::Clamp {
            min: StatValue::Int(lo),
            max: StatValue::Int(hi),
        } => running.clamp(*lo, *hi),
        _ => running,
    }
}

/// Folds validated integer-target modifiers onto a base.
///
/// `targeting` is already ordered by phase then ascending `(priority, id)`.
/// False conditions filter first and take precedence over stacking; the
/// remaining candidates group by `(mod_type, phase)` and run the shared
/// [`select_group`] stacking selection, then retained operations apply in
/// canonical global order. Returns the folded value plus every targeting
/// modifier's contribution in that same order, including excluded ones.
fn fold_numeric(
    targeting: &[&Modifier],
    base: i32,
    tags: &TagSet,
    policies: &BTreeMap<ModTypeId, StackingPolicy>,
) -> (i32, Vec<Contribution>) {
    let mut live: Vec<&Modifier> = Vec::new();
    let mut status: BTreeMap<Ulid, ContributionStatus> = BTreeMap::new();
    for modifier in targeting {
        let active = match &modifier.condition {
            None => true,
            Some(condition) => eval_condition(condition, tags),
        };
        if active {
            live.push(*modifier);
        } else {
            status.insert(modifier.id, ContributionStatus::ConditionFalse);
        }
    }
    let mut groups: BTreeMap<(String, Phase), Vec<&Modifier>> = BTreeMap::new();
    for modifier in live {
        groups
            .entry((modifier.mod_type.0.clone(), phase_of(&modifier.op)))
            .or_default()
            .push(modifier);
    }
    let mut retained: BTreeSet<(Phase, i16, Ulid)> = BTreeSet::new();
    for ((type_name, _), members) in &groups {
        let policy = policies
            .get(&ModTypeId(type_name.clone()))
            .copied()
            .unwrap_or(StackingPolicy::StackAll);
        select_group(policy, members, &mut retained, &mut status);
    }
    let mut running = base;
    let mut applied: BTreeMap<Ulid, (i32, i32)> = BTreeMap::new();
    let mut order: Vec<&Modifier> = targeting
        .iter()
        .filter(|modifier| {
            retained.contains(&(phase_of(&modifier.op), modifier.priority, modifier.id))
        })
        .copied()
        .collect();
    order.sort_by_key(|modifier| (phase_of(&modifier.op), modifier.priority, modifier.id));
    for modifier in order {
        let before = running;
        running = apply_numeric_modifier(running, &modifier.op);
        applied.insert(modifier.id, (before, running));
    }
    let mut contributions = Vec::with_capacity(targeting.len());
    for modifier in targeting {
        if let Some(excluded) = status.get(&modifier.id) {
            contributions.push(Contribution {
                modifier: (*modifier).clone(),
                status: excluded.clone(),
            });
            continue;
        }
        match applied.get(&modifier.id) {
            Some((before, after)) => contributions.push(Contribution {
                modifier: (*modifier).clone(),
                status: ContributionStatus::Applied {
                    before: StatValue::Int(*before),
                    after: StatValue::Int(*after),
                },
            }),
            None => {
                debug_assert!(false, "retained modifier without applied record");
                contributions.push(Contribution {
                    modifier: (*modifier).clone(),
                    status: ContributionStatus::ConditionFalse,
                });
            }
        }
    }
    (running, contributions)
}

/// Applies one retained operation to the running value.
fn apply_modifier(running: &StatValue, op: &ModOp) -> StatValue {
    match op {
        ModOp::Set(value) => value.clone(),
        ModOp::Add(value) => match (running, value) {
            (StatValue::Int(current), StatValue::Int(amount)) => {
                StatValue::Int(crate::derived::int_add(*current, *amount))
            }
            (StatValue::Fixed(current), StatValue::Fixed(amount)) => {
                StatValue::Fixed(current.saturating_add(*amount))
            }
            _ => running.clone(),
        },
        ModOp::Multiply(factor) => match running {
            StatValue::Int(current) => StatValue::Int(crate::derived::int_scale(*current, *factor)),
            StatValue::Fixed(current) => StatValue::Fixed(current.saturating_mul(*factor)),
            _ => running.clone(),
        },
        ModOp::Clamp { min, max } => match (running, min, max) {
            (StatValue::Int(current), StatValue::Int(lo), StatValue::Int(hi)) => {
                StatValue::Int((*current).clamp(*lo, *hi))
            }
            (StatValue::Fixed(current), StatValue::Fixed(lo), StatValue::Fixed(hi)) => {
                let clamped = if current < lo {
                    *lo
                } else if current > hi {
                    *hi
                } else {
                    *current
                };
                StatValue::Fixed(clamped)
            }
            _ => running.clone(),
        },
    }
}
