//! The read-only action preview (T143, ADR-0029 U9): exact odds of one
//! ability use, counted rather than sampled.
//!
//! [`PreviewCount`] is an exact unsigned integer of any size, and the dice
//! distributions behind it carry pinned cost formulas (T143b). Then
//! [`preview_action`] (T143c) admits one candidate `UseAbility` through the
//! one legality source (`admit`), builds the plan execution uses, takes the
//! defence value from the kernel's own `resolve`, and folds every possible
//! roll through the same modifier pipeline and outcome table. It takes
//! `&World`, so it mutates nothing and draws nothing from the world's RNG.
//! Every count is an exact integer over a stated total; no float and no
//! rounding appear anywhere in it. A preview over its limit is refused with
//! the cost it would have needed, never approximated.

use std::collections::BTreeMap;

use crpg_core::{DeterministicRng, EntityId, Ulid};
use crpg_rules::{
    resolve, DiceExpr, ModifierTarget, NaturalEffect, Outcome, OutcomeTable, QueryContext,
    ResolutionContext, ResolutionRequest, RollRequest, RollSpec, TagSet,
};

use crate::combat::{
    ability_entry, action_plan, admit, AbilityDefense, AbilityDefinition, ActionPlan, Admission,
    CombatAction, CombatError,
};
use crate::world::World;

mod count;
mod dist;

use dist::{cost, distribution, limbs, rows, tuples};

pub use count::PreviewCount;
pub use crpg_data::MAX_PREVIEW_COST;

/// Which admission a preview runs (T143 B2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewScope {
    /// Exactly `validate_action`.
    Actionable,
    /// `validate_action` without the turn and affordability stages.
    Planning,
}

/// A ruleset's preview policy as the sim applies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewRules {
    /// Which admission runs.
    pub scope: PreviewScope,
    /// The ruleset's cost limit; the effective limit is
    /// `min(max_cost, MAX_PREVIEW_COST)`.
    pub max_cost: u64,
}

impl PreviewRules {
    /// Maps a ruleset's field: `None` for absent or `never` (no preview is
    /// answered); otherwise the scope and `max_cost`, absent meaning
    /// `MAX_PREVIEW_COST`.
    #[must_use]
    pub fn from_wire(preview: Option<&crpg_data::PreviewWire>) -> Option<Self> {
        let wire = preview?;
        let scope = match wire.when {
            crpg_data::PreviewWhenWire::Never => return None,
            crpg_data::PreviewWhenWire::Actionable => PreviewScope::Actionable,
            crpg_data::PreviewWhenWire::Planning => PreviewScope::Planning,
        };
        Some(Self {
            scope,
            max_cost: wire.max_cost.unwrap_or(MAX_PREVIEW_COST),
        })
    }
}

/// The exact odds of one candidate `UseAbility`, read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionPreview {
    /// Equally likely attack draws: the actor's face tuples times the
    /// opposing face tuples (one factor of 1 without `Opposed`).
    pub attack_space: PreviewCount,
    /// One row per distinct outcome of the table, in table order; the
    /// counts sum to `attack_space`.
    pub outcomes: Vec<OutcomeOdds>,
}

/// One outcome's share of the attack draws and the damage it deals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeOdds {
    /// The outcome label.
    pub outcome: Outcome,
    /// Attack draws that decide this outcome; zero when none does.
    pub count: PreviewCount,
    /// Of `count`, the draws in which a natural-face rule matched.
    pub natural: PreviewCount,
    /// Equally likely damage draws: the face tuples of the outcome's damage
    /// dice, or one when it has none.
    pub damage_space: PreviewCount,
    /// `(damage, draws)`, ascending by damage, draws nonzero, summing to
    /// `damage_space`.
    pub damage: Vec<(u32, PreviewCount)>,
}

/// The part of a preview whose cost crossed the limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewPart {
    /// The ability's dice.
    Actor,
    /// An `Opposed` defence's dice.
    Opposing,
    /// Pairing every actor row with every opposing row.
    Attack,
    /// The damage dice of this outcome's row.
    Damage {
        /// The row's outcome.
        outcome: Outcome,
    },
}

/// Why a preview produced no odds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewError {
    /// Admission refused the candidate; this is its error, unchanged.
    Rejected(CombatError),
    /// The preview's cost exceeds its limit.
    TooLarge {
        /// The first part whose running cost crosses the limit.
        part: PreviewPart,
        /// The preview's full cost, or `None` when it overflows `u64`.
        cost: Option<u64>,
        /// The effective limit.
        limit: u64,
    },
}

impl std::fmt::Display for PreviewError {
    /// Renders the wrapped combat error unchanged, or `TooLarge at
    /// preview/<part>`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(error) => std::fmt::Display::fmt(error, f),
            Self::TooLarge { part, .. } => {
                let name = match part {
                    PreviewPart::Actor => "actor",
                    PreviewPart::Opposing => "opposing",
                    PreviewPart::Attack => "attack",
                    PreviewPart::Damage { .. } => "damage",
                };
                write!(f, "TooLarge at preview/{name}")
            }
        }
    }
}

impl std::error::Error for PreviewError {
    /// Exposes the wrapped combat error for `Rejected`; nothing otherwise.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Rejected(error) => Some(error),
            Self::TooLarge { .. } => None,
        }
    }
}

/// The distinct outcomes of a table, each at its first appearance: band
/// outcomes in authored order, then the `Override` outcomes of the natural
/// rules in rule order (B10).
fn row_outcomes(table: &OutcomeTable) -> Vec<Outcome> {
    let mut out: Vec<Outcome> = Vec::new();
    let mut push = |outcome: Outcome| {
        if !out.contains(&outcome) {
            out.push(outcome);
        }
    };
    for band in table.bands() {
        push(band.outcome);
    }
    for rule in table.natural_rules() {
        if let NaturalEffect::Override(outcome) = rule.effect {
            push(outcome);
        }
    }
    out
}

/// The first damage-dice entry for `outcome`, if any (`damage_for`'s rule).
fn damage_dice_of(entry: &AbilityDefinition, outcome: Outcome) -> Option<&DiceExpr> {
    entry
        .damage_dice
        .iter()
        .find(|(kind, _)| *kind == outcome)
        .map(|(_, dice)| dice)
}

/// The flat damage of `outcome`: the first flat entry, or zero.
fn flat_damage_of(entry: &AbilityDefinition, outcome: Outcome) -> u32 {
    entry
        .damage
        .iter()
        .find(|(kind, _)| *kind == outcome)
        .map_or(0, |(_, amount)| *amount)
}

/// Charges every part in B3's order and refuses the preview when a running
/// total exceeds `limit` or overflows (B4).
fn check_limit(
    entry: &AbilityDefinition,
    rule_faces: &[u32],
    damage_rows: &[(Outcome, Option<&DiceExpr>)],
    limit: u64,
) -> Result<(), PreviewError> {
    let actor = &entry.dice;
    let opposing = match &entry.defense {
        AbilityDefense::Opposed { dice, .. } => Some(dice),
        _ => None,
    };
    let mut parts: Vec<(PreviewPart, Option<u64>)> = vec![
        (
            PreviewPart::Actor,
            cost(actor, entry.natural_die, rule_faces),
        ),
        (
            PreviewPart::Opposing,
            opposing.map_or(Some(0), |dice| cost(dice, None, &[])),
        ),
    ];
    let (opposing_rows, opposing_limbs) =
        opposing.map_or((1, 1), |dice| (rows(dice, None, &[]), limbs(dice)));
    parts.push((
        PreviewPart::Attack,
        rows(actor, entry.natural_die, rule_faces)
            .checked_mul(opposing_rows)
            .and_then(|pairs| pairs.checked_mul(limbs(actor)))
            .and_then(|pairs| pairs.checked_mul(opposing_limbs)),
    ));
    for (outcome, dice) in damage_rows {
        if let Some(dice) = dice {
            parts.push((
                PreviewPart::Damage { outcome: *outcome },
                cost(dice, None, &[]),
            ));
        }
    }
    let mut running = Some(0_u64);
    let mut crossed = None;
    for (part, charge) in &parts {
        running = running
            .zip(*charge)
            .and_then(|(total, charge)| total.checked_add(charge));
        if crossed.is_none() && running.is_none_or(|total| total > limit) {
            crossed = Some(*part);
        }
    }
    match crossed {
        Some(part) => Err(PreviewError::TooLarge {
            part,
            cost: running,
            limit,
        }),
        None => Ok(()),
    }
}

/// The damage distribution of one row (B9).
fn row_damage(
    entry: &AbilityDefinition,
    outcome: Outcome,
    dice: Option<&DiceExpr>,
) -> (PreviewCount, Vec<(u32, PreviewCount)>) {
    let flat = flat_damage_of(entry, outcome);
    let Some(dice) = dice else {
        return (PreviewCount::from(1), vec![(flat, PreviewCount::from(1))]);
    };
    let mut merged: BTreeMap<u32, PreviewCount> = BTreeMap::new();
    for ((total, _), draws) in distribution(dice, None, &[]) {
        let damage = (i64::from(flat) + i64::from(total)).clamp(0, i64::from(u32::MAX));
        let damage = u32::try_from(damage).expect("clamped into u32");
        merged.entry(damage).or_default().add_assign(&draws);
    }
    (
        tuples(dice),
        merged
            .into_iter()
            .filter(|(_, draws)| !draws.is_zero())
            .collect(),
    )
}

/// Returns the exact outcome and damage distribution of `UseAbility {
/// actor, ability, target }` against the current state under `rules`,
/// without mutating `world` or drawing from its RNG (T143, ADR-0029 U9).
///
/// Admission runs first (`Actionable` is `validate_action`; `Planning` skips
/// only the turn and affordability stages), then the cost limit, then the
/// plan execution uses, the kernel's own defence value, and every roll the
/// dice can make folded through the modifier pipeline and the outcome table.
///
/// # Errors
/// [`PreviewError::Rejected`] with admission's own error, or
/// [`PreviewError::TooLarge`] when the preview's cost exceeds
/// `min(rules.max_cost, MAX_PREVIEW_COST)`.
///
/// # Panics
/// Never after admission succeeds; each `expect` names the admission stage
/// that guarantees it.
pub fn preview_action(
    world: &World,
    actor: EntityId,
    ability: Ulid,
    target: EntityId,
    rules: &PreviewRules,
) -> Result<ActionPreview, PreviewError> {
    let admission = match rules.scope {
        PreviewScope::Actionable => Admission::Execute,
        PreviewScope::Planning => Admission::Plan,
    };
    admit(
        world,
        &CombatAction::UseAbility {
            actor,
            ability,
            target,
        },
        admission,
    )
    .map_err(PreviewError::Rejected)?;

    let combat = world
        .combat()
        .expect("admission checks the encounter exists");
    let entry =
        ability_entry(&combat.definition, ability).expect("admission checks the ability is listed");
    let table = &entry.outcome_table;
    let rule_faces: Vec<u32> = table.natural_rules().iter().map(|rule| rule.face).collect();
    let outcomes = row_outcomes(table);
    let damage_rows: Vec<(Outcome, Option<&DiceExpr>)> = outcomes
        .iter()
        .map(|outcome| (*outcome, damage_dice_of(entry, *outcome)))
        .collect();
    let limit = rules.max_cost.min(MAX_PREVIEW_COST);
    check_limit(entry, &rule_faces, &damage_rows, limit)?;

    let plan = action_plan(world, world.interners().clone(), entry, actor, target)
        .expect("admission runs the stat evaluation stage");
    let ActionPlan {
        pipeline,
        roll_tag,
        actor_stats,
        target_stats,
        actor_tags,
        target_tags,
        actor_mods,
        target_mods,
        against,
    } = plan;
    let actor_view = QueryContext {
        entity: actor,
        stats: &actor_stats,
        tags: &actor_tags,
        modifiers: &actor_mods,
    };
    let target_view = if actor == target {
        QueryContext {
            entity: target,
            stats: &actor_stats,
            tags: &actor_tags,
            modifiers: &actor_mods,
        }
    } else {
        QueryContext {
            entity: target,
            stats: &target_stats,
            tags: &target_tags,
            modifiers: &target_mods,
        }
    };

    // The defence's folded total per opposing entry: one entry of count 1
    // without `Opposed`, otherwise one per distinct opposing raw total.
    let opposing: Vec<(i32, PreviewCount)> = match &entry.defense {
        AbilityDefense::Opposed { dice, .. } => distribution(dice, None, &[])
            .into_iter()
            .map(|((raw, _), draws)| {
                let (folded, _) = pipeline
                    .query_numeric(target, ModifierTarget::Roll(roll_tag), raw, &target_view)
                    .expect("admission runs the stat evaluation stage");
                (folded, draws)
            })
            .collect(),
        _ => {
            let mut entities = BTreeMap::new();
            entities.insert(actor, actor_view);
            if actor != target {
                entities.insert(target, target_view);
            }
            let context = ResolutionContext {
                pipeline: &pipeline,
                entities: &entities,
                outcome_table: table,
            };
            let request = ResolutionRequest {
                roll: RollRequest {
                    actor,
                    roll: RollSpec::None,
                    bonus_target: roll_tag,
                    natural_die: None,
                    stream: String::new(),
                    tags: TagSet::new(),
                },
                target: Some(target),
                against,
                outcome_table: table.id(),
            };
            let resolved = resolve(&request, &context, &mut DeterministicRng::from_seed(0))
                .expect("a roll-free resolution cannot fail after admission");
            vec![(resolved.against_value, PreviewCount::from(1))]
        }
    };

    let mut counts = vec![PreviewCount::zero(); outcomes.len()];
    let mut naturals = vec![PreviewCount::zero(); outcomes.len()];
    let mut decisions: BTreeMap<(i64, Option<u32>), (usize, bool)> = BTreeMap::new();
    let mut folds: BTreeMap<i32, i32> = BTreeMap::new();
    for ((raw, tag), actor_draws) in distribution(&entry.dice, entry.natural_die, &rule_faces) {
        let folded = *folds.entry(raw).or_insert_with(|| {
            pipeline
                .query_numeric(actor, ModifierTarget::Roll(roll_tag), raw, &actor_view)
                .expect("admission runs the stat evaluation stage")
                .0
        });
        for (defence, opposing_draws) in &opposing {
            let margin = i64::from(folded) - i64::from(*defence);
            let (row, natural) = *decisions.entry((margin, tag)).or_insert_with(|| {
                let decision = table.evaluate(margin, tag);
                let row = outcomes
                    .iter()
                    .position(|outcome| *outcome == decision.outcome)
                    .expect("every decided outcome has a row");
                (row, decision.natural_rule_index.is_some())
            });
            let paired = actor_draws.mul(opposing_draws);
            counts[row].add_assign(&paired);
            if natural {
                naturals[row].add_assign(&paired);
            }
        }
    }

    let mut attack_space = tuples(&entry.dice);
    if let AbilityDefense::Opposed { dice, .. } = &entry.defense {
        attack_space = attack_space.mul(&tuples(dice));
    }
    let odds = damage_rows
        .iter()
        .zip(counts)
        .zip(naturals)
        .map(|(((outcome, dice), count), natural)| {
            let (damage_space, damage) = row_damage(entry, *outcome, *dice);
            OutcomeOdds {
                outcome: *outcome,
                count,
                natural,
                damage_space,
                damage,
            }
        })
        .collect();
    Ok(ActionPreview {
        attack_space,
        outcomes: odds,
    })
}
