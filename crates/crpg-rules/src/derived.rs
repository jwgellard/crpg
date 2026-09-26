//! Derived-stat dependency graphs and expression evaluation.
//!
//! Graph validation runs at [`ModifierPipeline::new`](crate::ModifierPipeline)
//! time: reference resolution, canonical cycle detection, and the acyclic
//! depth bound. Evaluation reads fully modified values through a per-query
//! memo so diamonds compute once. Integer arithmetic saturates per operation
//! over `i64` intermediates and floors division; fixed-point arithmetic uses
//! core's saturating and flooring methods.

use std::collections::{BTreeMap, BTreeSet};

use crpg_core::{Fx16_16, StatId};

use crate::error::{RulesError, RulesErrorCode};
use crate::stats::{Expr, StatKind, StatValue, ValueKind};
use crate::MAX_DERIVED_DEPTH;

/// Saturating integer addition over a 64-bit intermediate.
pub(crate) fn int_add(left: i32, right: i32) -> i32 {
    (i64::from(left) + i64::from(right)).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// Saturating integer subtraction over a 64-bit intermediate.
pub(crate) fn int_subtract(left: i32, right: i32) -> i32 {
    (i64::from(left) - i64::from(right)).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// Saturating integer multiplication over a 64-bit intermediate.
pub(crate) fn int_multiply(left: i32, right: i32) -> i32 {
    (i64::from(left) * i64::from(right)).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// Flooring integer division: `None` on a zero divisor, saturating `MIN / -1`.
pub(crate) fn int_divide(left: i32, right: i32) -> Option<i32> {
    if right == 0 {
        return None;
    }
    if left == i32::MIN && right == -1 {
        return Some(i32::MAX);
    }
    let quotient = left / right;
    if left % right != 0 && (left < 0) != (right < 0) {
        Some(quotient - 1)
    } else {
        Some(quotient)
    }
}

/// Modifier integer scaling: `floor(value * factor / 65536)` clamped to `i32`.
///
/// Uses the factor's raw integer directly so full-range values never pass
/// through the fixed-point range. Negative and zero factors are legal.
pub(crate) fn int_scale(value: i32, factor: Fx16_16) -> i32 {
    let product = i64::from(value) * i64::from(factor.to_raw());
    let mut quotient = product / 65536;
    if product % 65536 != 0 && product < 0 {
        quotient -= 1;
    }
    quotient.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// The computed kind of an expression node used during validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExprKind {
    Int,
    Fixed,
    Bool,
    Enum(String, BTreeSet<String>),
    Tags,
    Dice,
}

impl ExprKind {
    pub(crate) fn discriminant(&self) -> ValueKind {
        match self {
            Self::Int => ValueKind::Int,
            Self::Fixed => ValueKind::Fixed,
            Self::Bool => ValueKind::Bool,
            Self::Enum(_, _) => ValueKind::Enum,
            Self::Tags => ValueKind::Tags,
            Self::Dice => ValueKind::Dice,
        }
    }

    pub(crate) fn matches_declaration(&self, kind: &StatKind) -> bool {
        match (self, kind) {
            (Self::Int, StatKind::Int)
            | (Self::Fixed, StatKind::Fixed)
            | (Self::Bool, StatKind::Bool)
            | (Self::Tags, StatKind::Tags)
            | (Self::Dice, StatKind::Dice) => true,
            (
                Self::Enum(id, variants),
                StatKind::Enum {
                    enum_id,
                    variants: declared,
                },
            ) => id == enum_id && variants == declared,
            _ => false,
        }
    }
}

/// Applies one validated binary operator to two evaluated operands.
///
/// Kind mismatches are unreachable after pipeline validation; they report
/// [`RulesErrorCode::TypeMismatch`] instead of panicking. Division by zero
/// reports [`RulesErrorCode::DivisionByZero`] at the caller's location.
pub(crate) fn apply_binary(
    name: BinaryOp,
    left: &StatValue,
    right: &StatValue,
    location: &str,
) -> Result<StatValue, RulesError> {
    let mismatch = || RulesError::at(RulesErrorCode::TypeMismatch, location.to_owned());
    let zero = || RulesError::at(RulesErrorCode::DivisionByZero, location.to_owned());
    match (left, right) {
        (StatValue::Int(a), StatValue::Int(b)) => {
            let value = match name {
                BinaryOp::Add => int_add(*a, *b),
                BinaryOp::Subtract => int_subtract(*a, *b),
                BinaryOp::Multiply => int_multiply(*a, *b),
                BinaryOp::Divide => int_divide(*a, *b).ok_or_else(zero)?,
                BinaryOp::Min => (*a).min(*b),
                BinaryOp::Max => (*a).max(*b),
            };
            Ok(StatValue::Int(value))
        }
        (StatValue::Fixed(a), StatValue::Fixed(b)) => {
            let value = match name {
                BinaryOp::Add => a.saturating_add(*b),
                BinaryOp::Subtract => a.saturating_sub(*b),
                BinaryOp::Multiply => a.saturating_mul(*b),
                BinaryOp::Divide => {
                    if *b == Fx16_16::ZERO {
                        return Err(zero());
                    }
                    a.saturating_div(*b)
                }
                BinaryOp::Min => (*a).min(*b),
                BinaryOp::Max => (*a).max(*b),
            };
            Ok(StatValue::Fixed(value))
        }
        _ => Err(mismatch()),
    }
}

/// The six binary expression operators, for shared evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Min,
    Max,
}

/// Evaluates an expression with stat references resolved by `lookup`.
///
/// Operands evaluate left before right with the authored grouping. Division
/// by zero reports [`RulesErrorCode::DivisionByZero`] at `location`.
pub(crate) fn eval_expr(
    expr: &Expr,
    lookup: &mut dyn FnMut(StatId) -> Result<StatValue, RulesError>,
    location: &str,
) -> Result<StatValue, RulesError> {
    match expr {
        Expr::Literal(value) => Ok(value.clone()),
        Expr::Stat(stat) => lookup(*stat),
        Expr::Add(left, right) => {
            let a = eval_expr(left, lookup, location)?;
            let b = eval_expr(right, lookup, location)?;
            apply_binary(BinaryOp::Add, &a, &b, location)
        }
        Expr::Subtract(left, right) => {
            let a = eval_expr(left, lookup, location)?;
            let b = eval_expr(right, lookup, location)?;
            apply_binary(BinaryOp::Subtract, &a, &b, location)
        }
        Expr::Multiply(left, right) => {
            let a = eval_expr(left, lookup, location)?;
            let b = eval_expr(right, lookup, location)?;
            apply_binary(BinaryOp::Multiply, &a, &b, location)
        }
        Expr::Divide(left, right) => {
            let a = eval_expr(left, lookup, location)?;
            let b = eval_expr(right, lookup, location)?;
            apply_binary(BinaryOp::Divide, &a, &b, location)
        }
        Expr::Min(left, right) => {
            let a = eval_expr(left, lookup, location)?;
            let b = eval_expr(right, lookup, location)?;
            apply_binary(BinaryOp::Min, &a, &b, location)
        }
        Expr::Max(left, right) => {
            let a = eval_expr(left, lookup, location)?;
            let b = eval_expr(right, lookup, location)?;
            apply_binary(BinaryOp::Max, &a, &b, location)
        }
    }
}

/// Preorder expression walk used by reference and type validation.
pub(crate) fn preorder<'a>(expr: &'a Expr, visit: &mut dyn FnMut(&'a Expr)) {
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        visit(node);
        match node {
            Expr::Literal(_) | Expr::Stat(_) => {}
            Expr::Add(left, right)
            | Expr::Subtract(left, right)
            | Expr::Multiply(left, right)
            | Expr::Divide(left, right)
            | Expr::Min(left, right)
            | Expr::Max(left, right) => {
                stack.push(right);
                stack.push(left);
            }
        }
    }
}

/// Finds the first directed cycle with an iterative ascending DFS.
///
/// Roots and neighbours traverse in ascending [`StatId`] order. The reported
/// path is the DFS cycle with its repeated closing vertex, rotated to the
/// smallest stat id.
pub(crate) fn find_cycle(adjacency: &BTreeMap<StatId, Vec<StatId>>) -> Option<Vec<StatId>> {
    const WHITE: u8 = 0;
    const GRAY: u8 = 1;
    const BLACK: u8 = 2;
    let mut color: BTreeMap<StatId, u8> = adjacency.keys().map(|stat| (*stat, WHITE)).collect();
    for root in adjacency.keys() {
        if color[root] == BLACK || color[root] == GRAY {
            continue;
        }
        let mut path = vec![*root];
        let mut stack = vec![(*root, 0_usize)];
        color.insert(*root, GRAY);
        while let Some((node, child)) = stack.pop() {
            let neighbours = adjacency.get(&node).cloned().unwrap_or_default();
            if child < neighbours.len() {
                stack.push((node, child + 1));
                let next = neighbours[child];
                match color.get(&next).copied().unwrap_or(BLACK) {
                    GRAY => {
                        let start = path.iter().position(|stat| stat == &next).unwrap_or(0);
                        let mut cycle: Vec<StatId> = path[start..].to_vec();
                        cycle.push(next);
                        return Some(rotate_cycle(&cycle));
                    }
                    WHITE => {
                        color.insert(next, GRAY);
                        path.push(next);
                        stack.push((next, 0));
                    }
                    _ => {}
                }
            } else {
                color.insert(node, BLACK);
                path.pop();
            }
        }
    }
    None
}

/// Rotates a cycle (with repeated closing vertex) to its smallest stat id.
fn rotate_cycle(cycle: &[StatId]) -> Vec<StatId> {
    let body = &cycle[..cycle.len() - 1];
    let mut start = 0_usize;
    for (index, stat) in body.iter().enumerate() {
        if stat < &body[start] {
            start = index;
        }
    }
    let mut rotated: Vec<StatId> = body[start..]
        .iter()
        .chain(body[..start].iter())
        .copied()
        .collect();
    rotated.push(rotated[0]);
    rotated
}

/// Checks the longest dependency path against [`MAX_DERIVED_DEPTH`].
///
/// Endpoints are included in the path length. Runs on a topological order so
/// diamonds share subproblem results instead of expanding every path.
/// Returns the smallest stat id whose longest path exceeds the bound.
pub(crate) fn over_depth_stat(adjacency: &BTreeMap<StatId, Vec<StatId>>) -> Option<StatId> {
    // Kahn's algorithm over dependency-before-dependent order, ascending.
    let mut indegree: BTreeMap<StatId, usize> = BTreeMap::new();
    let mut dependents: BTreeMap<StatId, Vec<StatId>> = BTreeMap::new();
    for stat in adjacency.keys() {
        indegree.insert(*stat, 0);
    }
    for (stat, refs) in adjacency {
        for reference in refs {
            dependents.entry(*reference).or_default().push(*stat);
            if let Some(degree) = indegree.get_mut(stat) {
                *degree += 1;
            }
        }
    }
    // Longest path ending at each stat, endpoints included.
    let mut longest: BTreeMap<StatId, usize> = BTreeMap::new();
    let mut ready: Vec<StatId> = indegree
        .iter()
        .filter_map(|(stat, degree)| if *degree == 0 { Some(*stat) } else { None })
        .collect();
    ready.sort();
    let mut queue = std::collections::VecDeque::from(ready);
    while let Some(node) = queue.pop_front() {
        let base = longest.get(&node).copied().unwrap_or(1).max(1);
        longest.insert(node, base);
        let mut next: Vec<StatId> = dependents.get(&node).cloned().unwrap_or_default();
        next.sort();
        for dependent in next {
            let candidate = base + 1;
            let slot = longest.entry(dependent).or_insert(1);
            *slot = (*slot).max(candidate);
            if let Some(remaining) = indegree.get_mut(&dependent) {
                *remaining -= 1;
                if *remaining == 0 {
                    queue.push_back(dependent);
                }
            }
        }
    }
    // Deterministic report: smallest stat id whose path exceeds the bound.
    let mut over: Vec<StatId> = longest
        .iter()
        .filter_map(|(stat, length)| {
            if *length > MAX_DERIVED_DEPTH {
                Some(*stat)
            } else {
                None
            }
        })
        .collect();
    over.sort();
    over.into_iter().next()
}
