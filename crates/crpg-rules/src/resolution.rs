//! Data-driven outcome tables and generic resolution.
//!
//! An [`OutcomeTable`] partitions signed margins with sorted lower bounds
//! and applies at most one ordered natural-face rule to an explicitly
//! selected raw die. [`resolve`] evaluates one actor roll against a constant
//! DC, a fully modified stat DC, or an opposed roll: it validates table
//! identity, entity existence, roll shapes, and each unique participant
//! context once in ascending entity order, checks tag unions, evaluates
//! stat-based DCs in preflight and reuses those traces, draws actor then
//! opponent dice on a staged RNG clone, folds roll modifiers onto each raw
//! total, and commits the staged RNG only after complete success. Failed
//! calls leave the caller's RNG exactly unchanged. Resolution mutates
//! nothing else and emits no hooks; the host owns liveness, authority, and
//! dispatch.

use std::collections::{BTreeMap, BTreeSet};

use crpg_core::{DeterministicRng, EntityId, StatId};
use serde::{Deserialize, Serialize};

use crate::dice::{check_stream, DiceExpr, DiceRoll};
use crate::error::{RulesError, RulesErrorCode};
use crate::modifier::{
    ModifierBreakdown, ModifierPipeline, ModifierTarget, NumericModifierBreakdown, QueryContext,
    RollTag,
};
use crate::stats::{StatValue, TagSet};
use crate::{MAX_DIE_SIDES, MAX_NATURAL_RULES, MAX_OUTCOME_BANDS, MAX_TAGS};

/// One data-selected outcome label.
///
/// Labels carry no numeric or critical behaviour in the engine; band order
/// is the authored degree order. Serializes with adjacent `type`/`value`
/// tags in snake case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Outcome {
    /// The best authored band.
    CriticalSuccess,
    /// A successful band.
    Success,
    /// A failed band.
    Failure,
    /// The worst authored band.
    CriticalFailure,
    /// An author-defined label.
    Custom(u8),
}

/// The authored identity of one outcome table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OutcomeTableId(pub crpg_core::Ulid);

/// One margin band: the inclusive lower bound selecting an outcome.
///
/// Bands partition signed margins with sorted lower bounds; upper bounds
/// are exclusive and implicit. The first band must start at `i64::MIN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomeBand {
    /// The inclusive lower margin bound of this band.
    pub min_margin: i64,
    /// The outcome selected while this band applies.
    pub outcome: Outcome,
}

/// What a matching natural-face rule does to the selected band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum NaturalEffect {
    /// Shifts the selected band index toward higher margins for positive
    /// values (clamped to the first/last band) or toward lower margins for
    /// negative values.
    Shift(i16),
    /// Replaces the selected band's outcome.
    Override(Outcome),
}

/// One ordered natural-face rule applied to an explicitly selected raw die.
///
/// Faces need no table-wide ordering; repeated faces are rejected so
/// priority stays unambiguous. A rule for a face unavailable on a
/// particular die simply does not match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NaturalRule {
    /// The raw face value this rule matches.
    pub face: u32,
    /// What matching does to the selected band.
    pub effect: NaturalEffect,
}

/// One validated data-driven outcome table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeTable {
    id: OutcomeTableId,
    bands: Vec<OutcomeBand>,
    natural_rules: Vec<NaturalRule>,
}

impl OutcomeTable {
    /// Builds a table after validating its authored data.
    ///
    /// Bands must be nonempty, within the band limit, begin at `i64::MIN`,
    /// and carry strictly increasing lower bounds in supplied order; they
    /// are never silently sorted. Natural rules are bounded, each face lies
    /// in `1..=MAX_DIE_SIDES`, and faces do not repeat. Collection limits
    /// precede contents, bands precede natural rules, and entries validate
    /// in authored order. Failures report
    /// [`RulesErrorCode::InvalidOutcomeTable`] at `/outcome_table/bands`,
    /// indexed `/outcome_table/bands/<index>/min_margin`,
    /// `/outcome_table/natural_rules`, or
    /// `/outcome_table/natural_rules/<index>/face`.
    pub fn new(
        id: OutcomeTableId,
        bands: Vec<OutcomeBand>,
        natural_rules: Vec<NaturalRule>,
    ) -> Result<Self, RulesError> {
        if bands.is_empty() || bands.len() > MAX_OUTCOME_BANDS {
            return Err(RulesError::at(
                RulesErrorCode::InvalidOutcomeTable,
                String::from("/outcome_table/bands"),
            ));
        }
        for (index, band) in bands.iter().enumerate() {
            let location = format!("/outcome_table/bands/{index}/min_margin");
            if index == 0 {
                if band.min_margin != i64::MIN {
                    return Err(RulesError::at(
                        RulesErrorCode::InvalidOutcomeTable,
                        location,
                    ));
                }
            } else if band.min_margin <= bands[index - 1].min_margin {
                return Err(RulesError::at(
                    RulesErrorCode::InvalidOutcomeTable,
                    location,
                ));
            }
        }
        if natural_rules.len() > MAX_NATURAL_RULES {
            return Err(RulesError::at(
                RulesErrorCode::InvalidOutcomeTable,
                String::from("/outcome_table/natural_rules"),
            ));
        }
        let mut faces = BTreeSet::new();
        for (index, rule) in natural_rules.iter().enumerate() {
            let location = format!("/outcome_table/natural_rules/{index}/face");
            if rule.face == 0 || rule.face > MAX_DIE_SIDES {
                return Err(RulesError::at(
                    RulesErrorCode::InvalidOutcomeTable,
                    location,
                ));
            }
            if !faces.insert(rule.face) {
                return Err(RulesError::at(
                    RulesErrorCode::InvalidOutcomeTable,
                    location,
                ));
            }
        }
        Ok(Self {
            id,
            bands,
            natural_rules,
        })
    }

    /// Returns the table identity.
    #[must_use]
    pub const fn id(&self) -> OutcomeTableId {
        self.id
    }

    /// Returns the bands in authored order.
    #[must_use]
    pub fn bands(&self) -> &[OutcomeBand] {
        &self.bands
    }

    /// Returns the natural rules in authored order.
    #[must_use]
    pub fn natural_rules(&self) -> &[NaturalRule] {
        &self.natural_rules
    }

    /// Selects the outcome for a margin with an optional raw natural face.
    ///
    /// Selects the greatest lower bound at or below `margin`, then applies
    /// at most one matching natural rule: a shift moves the selected band
    /// index (clamped to the first/last band) while the recorded
    /// `band_index` stays unshifted, and an override replaces the outcome.
    /// No natural value means no rule; there is no chaining.
    #[must_use]
    pub fn evaluate(&self, margin: i64, natural: Option<u32>) -> OutcomeDecision {
        let mut band_index = 0_usize;
        for (index, band) in self.bands.iter().enumerate() {
            if band.min_margin <= margin {
                band_index = index;
            } else {
                break;
            }
        }
        let natural_rule_index =
            natural.and_then(|face| self.natural_rules.iter().position(|rule| rule.face == face));
        let outcome = match natural_rule_index.map(|index| &self.natural_rules[index].effect) {
            None => self.bands[band_index].outcome,
            Some(NaturalEffect::Override(outcome)) => *outcome,
            Some(NaturalEffect::Shift(shift)) => {
                let shifted = (band_index as i64 + i64::from(*shift))
                    .clamp(0, self.bands.len() as i64 - 1) as usize;
                self.bands[shifted].outcome
            }
        };
        OutcomeDecision {
            band_index,
            natural_rule_index,
            outcome,
        }
    }
}

impl Serialize for OutcomeTable {
    /// Serializes the `{id, bands, natural_rules}` wire object.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut shape = serializer.serialize_struct("OutcomeTable", 3)?;
        shape.serialize_field("id", &self.id)?;
        shape.serialize_field("bands", &self.bands)?;
        shape.serialize_field("natural_rules", &self.natural_rules)?;
        shape.end()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeTableRepr {
    id: OutcomeTableId,
    bands: Vec<OutcomeBand>,
    natural_rules: Vec<NaturalRule>,
}

impl<'de> Deserialize<'de> for OutcomeTable {
    /// Decodes through [`OutcomeTable::new`], rejecting invalid tables.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let repr = OutcomeTableRepr::deserialize(deserializer)?;
        Self::new(repr.id, repr.bands, repr.natural_rules).map_err(serde::de::Error::custom)
    }
}

/// The selected outcome for one margin and natural face.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeDecision {
    /// The unshifted band index selected by the margin.
    pub band_index: usize,
    /// The matched natural rule in authored order, if any.
    pub natural_rule_index: Option<usize>,
    /// The resulting outcome after any shift or override.
    pub outcome: Outcome,
}

/// What dice (if any) an actor or opponent rolls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollSpec {
    /// No roll: the raw total is zero and no stream is touched.
    None,
    /// Rolls this expression on the request's stream.
    Dice(DiceExpr),
}

/// One side's roll: whose modifiers apply, what rolls, and which tags gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollRequest {
    /// The entity whose view supplies tags, stats, and modifiers.
    pub actor: EntityId,
    /// What dice (if any) this side rolls.
    pub roll: RollSpec,
    /// The roll tag selecting bonus modifiers for the raw total.
    pub bonus_target: RollTag,
    /// The zero-based raw draw index selected as the natural face, even
    /// when that die is later dropped. Never the sum or modified total.
    /// Must be `None` for [`RollSpec::None`] and below the dice count
    /// otherwise.
    pub natural_die: Option<u32>,
    /// The named RNG stream drawn for [`RollSpec::Dice`]; empty for
    /// [`RollSpec::None`].
    pub stream: String,
    /// Request tags unioned with the actor's entity tags for roll
    /// conditions. The union must satisfy the tag bound.
    pub tags: TagSet,
}

/// What the actor's total is measured against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Against {
    /// A constant value plus its owner's DC modifiers.
    Dc {
        /// The constant difficulty before modifiers.
        value: i32,
        /// The entity whose view supplies tags and DC modifiers.
        owner: EntityId,
        /// The DC tag selecting modifiers for the constant.
        modifier_target: RollTag,
    },
    /// A fully modified `Int` stat plus DC modifiers on its final value.
    Stat {
        /// The entity whose view supplies the stat and DC modifiers.
        entity: EntityId,
        /// The stat queried through the existing stat pipeline.
        stat: StatId,
        /// The DC tag selecting modifiers for the final stat value.
        modifier_target: RollTag,
    },
    /// An opposed roll: its final total, with no additional DC fold.
    Opposed(RollRequest),
}

/// One generic resolution request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionRequest {
    /// The actor's roll.
    pub roll: RollRequest,
    /// Contextual identity only: neither chooses the DC owner nor changes
    /// modifier membership. A provided view is required when set.
    pub target: Option<EntityId>,
    /// What the actor's total is measured against.
    pub against: Against,
    /// The table whose identity must equal the context table's.
    pub outcome_table: OutcomeTableId,
}

/// The borrowed views one resolution evaluates against.
#[derive(Debug, Clone, Copy)]
pub struct ResolutionContext<'a> {
    /// The validated definitions and stacking policies.
    pub pipeline: &'a ModifierPipeline,
    /// One borrowed per-entity view per participant; unused views are
    /// ignored. Each used map key must match its view's entity.
    pub entities: &'a BTreeMap<EntityId, QueryContext<'a>>,
    /// The outcome table applied to the actor's margin and natural face.
    pub outcome_table: &'a OutcomeTable,
}

/// One side's evaluated roll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRoll {
    /// The dice trace, when this side rolled.
    pub roll: Option<DiceRoll>,
    /// The selected raw natural face, when this side rolled with a selector.
    pub natural: Option<u32>,
    /// The final total after roll modifiers (or the raw total of zero).
    pub total: i32,
    /// The roll-modifier fold trace.
    pub modifiers: NumericModifierBreakdown,
}

/// How the defence side was evaluated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgainstBreakdown {
    /// A constant DC plus its owner's DC modifiers.
    Dc(NumericModifierBreakdown),
    /// A fully modified stat plus DC modifiers on its final value. The
    /// `stat` trace is reused from preflight, never queried twice.
    Stat {
        /// The stat pipeline trace reused from preflight.
        stat: ModifierBreakdown,
        /// The DC-modifier fold on the final stat value.
        dc: NumericModifierBreakdown,
    },
    /// An opposed roll's resolved total.
    Opposed(ResolvedRoll),
}

/// The complete result of one resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionResult {
    /// The actor's resolved roll.
    pub actor: ResolvedRoll,
    /// How the defence side was evaluated.
    pub against: AgainstBreakdown,
    /// The final defence value the margin subtracts.
    pub against_value: i32,
    /// The unsaturated `i64` actor-minus-defence difference.
    pub margin: i64,
    /// The table decision for the margin and the actor's natural face.
    pub decision: OutcomeDecision,
}

/// Prefixes a forwarded stat/modifier failure with its participant path so
/// two sides stay distinguishable. The nested code and cycle are preserved.
fn prefixed(prefix: &str, error: RulesError) -> RulesError {
    RulesError {
        code: error.code,
        location: format!("{prefix}{}", error.location),
        cycle: error.cycle,
    }
}

/// Validates one roll shape: `None` takes no selector and an empty stream,
/// while `Dice` takes a valid nonempty stream and a selector below the dice
/// count. Failures use roll-field suffixes under `prefix`.
fn check_roll_shape(roll: &RollRequest, prefix: &str) -> Result<(), RulesError> {
    match &roll.roll {
        RollSpec::None => {
            if roll.natural_die.is_some() {
                return Err(RulesError::at(
                    RulesErrorCode::InvalidResolution,
                    format!("{prefix}/roll/natural_die"),
                ));
            }
            if !roll.stream.is_empty() {
                return Err(RulesError::at(
                    RulesErrorCode::InvalidResolution,
                    format!("{prefix}/roll/stream"),
                ));
            }
        }
        RollSpec::Dice(expr) => {
            if let Err(error) = check_stream(&roll.stream) {
                return Err(RulesError {
                    code: error.code,
                    location: format!("{prefix}/roll/stream"),
                    cycle: error.cycle,
                });
            }
            if let Some(selector) = roll.natural_die {
                if selector >= expr.count() {
                    return Err(RulesError::at(
                        RulesErrorCode::InvalidResolution,
                        format!("{prefix}/roll/natural_die"),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Unions entity tags with request tags for roll conditions.
///
/// Actor roll conditions use the union of the actor's entity tags and its
/// request tags; opposing roll conditions use the opponent's corresponding
/// union. DC/stat conditions use their owner's original context tags, so no
/// union is built for them. The union must satisfy the tag bound.
fn union_tags(
    entity_tags: &TagSet,
    request_tags: &TagSet,
    prefix: &str,
) -> Result<TagSet, RulesError> {
    let mut union = entity_tags.clone();
    union.extend(request_tags.iter().copied());
    if union.len() > MAX_TAGS {
        return Err(RulesError::at(
            RulesErrorCode::LimitExceeded,
            format!("{prefix}/roll/tags"),
        ));
    }
    Ok(union)
}

/// Resolves one request against borrowed views with transactional randomness.
///
/// Evaluation order: table identity; actor existence; optional target
/// existence; defender/DC-owner existence; actor roll shape; opposing roll
/// shape; each unique participant context once in ascending entity order;
/// tag unions; preflight DC/stat evaluation with reused traces; actor dice,
/// then opposing dice, on a staged RNG clone; roll modifiers on each raw
/// total; margin and table selection with the actor's natural face. The
/// staged RNG commits only after complete success, so any error preserves
/// the caller's RNG exactly; successful no-roll resolution mutates no RNG
/// state. Apart from that commit, resolution mutates nothing.
pub fn resolve(
    request: &ResolutionRequest,
    context: &ResolutionContext<'_>,
    rng: &mut DeterministicRng,
) -> Result<ResolutionResult, RulesError> {
    if context.outcome_table.id() != request.outcome_table {
        return Err(RulesError::at(
            RulesErrorCode::InvalidResolution,
            String::from("/resolution/outcome_table"),
        ));
    }
    if !context.entities.contains_key(&request.roll.actor) {
        return Err(RulesError::at(
            RulesErrorCode::MissingEntity,
            String::from("/resolution/actor"),
        ));
    }
    if let Some(target) = request.target {
        if !context.entities.contains_key(&target) {
            return Err(RulesError::at(
                RulesErrorCode::MissingEntity,
                String::from("/resolution/target"),
            ));
        }
    }
    let defender = match &request.against {
        Against::Dc { owner, .. } => *owner,
        Against::Stat { entity, .. } => *entity,
        Against::Opposed(opposed) => opposed.actor,
    };
    if !context.entities.contains_key(&defender) {
        return Err(RulesError::at(
            RulesErrorCode::MissingEntity,
            String::from("/resolution/against"),
        ));
    }
    check_roll_shape(&request.roll, "/resolution/actor")?;
    if let Against::Opposed(opposed) = &request.against {
        check_roll_shape(opposed, "/resolution/against")?;
    }
    // Each unique participant context once, ascending by entity.
    let mut participants: Vec<(EntityId, &str)> = vec![(request.roll.actor, "/resolution/actor")];
    if defender != request.roll.actor {
        participants.push((defender, "/resolution/against"));
    }
    participants.sort_by_key(|(entity, _)| *entity);
    for (entity, prefix) in &participants {
        let view = context
            .entities
            .get(entity)
            .expect("participant existence is checked above");
        if view.entity != *entity {
            return Err(RulesError::at(
                RulesErrorCode::EntityMismatch,
                (*prefix).to_owned(),
            ));
        }
        context
            .pipeline
            .check_context(view)
            .map_err(|error| prefixed(prefix, error))?;
    }
    let actor_view = context
        .entities
        .get(&request.roll.actor)
        .expect("actor existence is checked above");
    let actor_union = union_tags(actor_view.tags, &request.roll.tags, "/resolution/actor")?;
    let opposed_union = match &request.against {
        Against::Opposed(opposed) => {
            let opposed_view = context
                .entities
                .get(&opposed.actor)
                .expect("defender existence is checked above");
            Some(union_tags(
                opposed_view.tags,
                &opposed.tags,
                "/resolution/against",
            )?)
        }
        _ => None,
    };
    // Preflight defence for non-opposed sides precedes any draw; the
    // resulting traces are reused rather than queried again.
    let preflight: Option<(i32, AgainstBreakdown)> = match &request.against {
        Against::Dc {
            value,
            owner,
            modifier_target,
        } => {
            let view = context
                .entities
                .get(owner)
                .expect("defender existence is checked above");
            let (folded, breakdown) = context
                .pipeline
                .query_numeric(*owner, ModifierTarget::Dc(*modifier_target), *value, view)
                .map_err(|error| prefixed("/resolution/against", error))?;
            Some((folded, AgainstBreakdown::Dc(breakdown)))
        }
        Against::Stat {
            entity,
            stat,
            modifier_target,
        } => {
            let view = context
                .entities
                .get(entity)
                .expect("defender existence is checked above");
            let (stat_value, stat_breakdown) = context
                .pipeline
                .query(*entity, *stat, view)
                .map_err(|error| prefixed("/resolution/against", error))?;
            let StatValue::Int(base) = stat_value else {
                return Err(RulesError::at(
                    RulesErrorCode::TypeMismatch,
                    String::from("/resolution/against"),
                ));
            };
            let (folded, dc) = context
                .pipeline
                .query_numeric(*entity, ModifierTarget::Dc(*modifier_target), base, view)
                .map_err(|error| prefixed("/resolution/against", error))?;
            Some((
                folded,
                AgainstBreakdown::Stat {
                    stat: stat_breakdown,
                    dc,
                },
            ))
        }
        Against::Opposed(_) => None,
    };
    // Draw actor dice, then opposing dice, on a staged clone in that order
    // so shared stream names consume consecutive draws deterministically.
    let mut staged = rng.clone();
    let mut rolled = false;
    let (actor_trace, actor_natural, actor_raw) = match &request.roll.roll {
        RollSpec::None => (None, None, 0),
        RollSpec::Dice(expr) => {
            let trace = expr
                .evaluate(&mut staged, &request.roll.stream)
                .map_err(|error| prefixed("/resolution/actor/roll", error))?;
            let natural = request
                .roll
                .natural_die
                .map(|selector| trace.dice[selector as usize].value);
            let raw = trace.total;
            rolled = true;
            (Some(trace), natural, raw)
        }
    };
    let actor_query = QueryContext {
        entity: request.roll.actor,
        stats: actor_view.stats,
        tags: &actor_union,
        modifiers: actor_view.modifiers,
    };
    let (actor_total, actor_modifiers) = context
        .pipeline
        .query_numeric(
            request.roll.actor,
            ModifierTarget::Roll(request.roll.bonus_target),
            actor_raw,
            &actor_query,
        )
        .map_err(|error| prefixed("/resolution/actor", error))?;
    let (against_value, against_breakdown) = match &request.against {
        Against::Opposed(opposed) => {
            let opposed_view = context
                .entities
                .get(&opposed.actor)
                .expect("defender existence is checked above");
            let (opposed_trace, opposed_natural, opposed_raw) = match &opposed.roll {
                RollSpec::None => (None, None, 0),
                RollSpec::Dice(expr) => {
                    let trace = expr
                        .evaluate(&mut staged, &opposed.stream)
                        .map_err(|error| prefixed("/resolution/against/roll", error))?;
                    let natural = opposed
                        .natural_die
                        .map(|selector| trace.dice[selector as usize].value);
                    let raw = trace.total;
                    rolled = true;
                    (Some(trace), natural, raw)
                }
            };
            let opposed_query = QueryContext {
                entity: opposed.actor,
                stats: opposed_view.stats,
                tags: opposed_union
                    .as_ref()
                    .expect("opposed union is built above"),
                modifiers: opposed_view.modifiers,
            };
            let (opposed_total, opposed_modifiers) = context
                .pipeline
                .query_numeric(
                    opposed.actor,
                    ModifierTarget::Roll(opposed.bonus_target),
                    opposed_raw,
                    &opposed_query,
                )
                .map_err(|error| prefixed("/resolution/against", error))?;
            (
                opposed_total,
                AgainstBreakdown::Opposed(ResolvedRoll {
                    roll: opposed_trace,
                    natural: opposed_natural,
                    total: opposed_total,
                    modifiers: opposed_modifiers,
                }),
            )
        }
        Against::Dc { .. } | Against::Stat { .. } => match preflight {
            Some(done) => done,
            None => {
                return Err(RulesError::at(
                    RulesErrorCode::InvalidResolution,
                    String::from("/resolution/against"),
                ));
            }
        },
    };
    let margin = i64::from(actor_total) - i64::from(against_value);
    let decision = context.outcome_table.evaluate(margin, actor_natural);
    if rolled {
        *rng = staged;
    }
    Ok(ResolutionResult {
        actor: ResolvedRoll {
            roll: actor_trace,
            natural: actor_natural,
            total: actor_total,
            modifiers: actor_modifiers,
        },
        against: against_breakdown,
        against_value,
        margin,
        decision,
    })
}
