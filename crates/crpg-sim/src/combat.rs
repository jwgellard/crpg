//! Authoritative headless combat (T016b, ADR-0013; generalized T017d, ADR-0016).
//!
//! The data-to-runtime adapter ([`start_encounter`]) instantiates validated
//! authored encounters into world-owned state, and the combat controller
//! ([`perform_action`]) applies one authoritative action at a time through
//! the rules kernel. All behavior-affecting encounter state lives in
//! [`World`](crate::World) — combatants, encounter state, and the issuing
//! interner — and hashes in full; nothing authoritative sits in caller
//! closures or un-hashed side structures.
//!
//! The controller owns round structure while [`Timeline`](crate::Timeline)
//! stays a container and [`end_turn`](crate::end_turn) stays pop-only:
//! ties break on ascending [`EntityId`](crpg_core::EntityId), an accepted
//! attack pops the actor and advances to the next head unless its definition
//! keeps the turn, the round rolls over when the timeline empties with two
//! or more combatants alive, and the encounter goes terminal (`active` of
//! `None`) otherwise. Rejected actions leave the entire world unchanged,
//! including resources, RNG streams, the event queue, and the event sequence.
//!
//! Names, thresholds, dice expressions, damage values, and outcome mappings
//! are authored data consumed read-only here. Production code in this module
//! holds no ruleset identifier, attribute name, die count, threshold, or
//! damage value; abilities are addressed by identity and pools by template.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crpg_core::{EntityId, GenerationalArena, Interners, Tick, Ulid};
use crpg_data::{
    Ability as AuthoredAbility, Creature as AuthoredCreature, DefenseWire,
    Effect as AuthoredEffect, Encounter as AuthoredEncounter, ExprWire,
    OutcomeTable as AuthoredOutcomeTable, Placement as AuthoredPlacement,
    RefreshWire as AuthoredRefresh, Ruleset as AuthoredRuleset, StatKindWire, StatValueWire,
};
use crpg_rules::{
    resolve, Against, DiceExpr, DiceRoll, Expr, ModOp, ModTypeId, Modifier, ModifierPipeline,
    ModifierTarget, NaturalEffect, NaturalRule, Outcome, OutcomeBand, OutcomeTable, OutcomeTableId,
    QueryContext, RefreshEvent, RefreshTrigger, ResolutionContext, ResolutionRequest, ResourcePool,
    ResourcePoolId, RestId as RulesRestId, RollRequest, RollSpec, RollTag, SourceRef,
    StackingPolicy, StatBlock, StatDefinition, StatKind, StatValue, TagSet,
};

use crate::event::SimEvent;
use crate::tick::end_turn;
use crate::timeline::InitiativeKey;
use crate::world::{EntityMeta, World};

/// Maximum participants in one encounter.
pub const MAX_COMBATANTS: usize = 1024;
/// Maximum stat declarations in one encounter ruleset.
pub const MAX_COMBAT_STATS: usize = 1024;
/// Maximum options one [`legal_actions`] query returns, `EndTurn` included.
pub const MAX_LEGAL_ACTIONS: usize = 4096;
/// The named RNG stream used for all combat resolution draws.
pub const COMBAT_ROLL_STREAM: &str = "combat.roll";
/// The tag symbol interned for the resolution roll tag.
pub const COMBAT_ROLL_TAG: &str = "roll";
/// The opaque source kind recorded on rebuilt effect modifiers.
const EFFECT_SOURCE_KIND: &str = "effect";
/// The reserved modifier type of bonus-stat roll contributions (T137). An
/// encounter whose abilities carry bonus stats rejects an effect using it.
pub const COMBAT_BONUS_MOD_TYPE: &str = "combat.bonus_stat";
/// The opaque source kind recorded on bonus-stat modifiers.
const BONUS_SOURCE_KIND: &str = "bonus_stat";
/// The panic text of a broken data-owned derived-stat precondition (T137 B4).
const DERIVED_PRECONDITION: &str = "derived int stats are validated by data read-time checks";

/// Encounter time policy (ADR-0029 T1): per encounter, hashed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncounterMode {
    /// One head acts at a time (ADR-0013 Decision 5, ADR-0016).
    #[default]
    TurnBased,
    /// Due standing orders execute inside the tick (ADR-0029 T5–T8).
    RealTime,
}

/// The largest encounter-clock value: `i32::MAX`, the largest
/// [`InitiativeKey`] (ADR-0029 T5). The clock freezes here.
pub const RT_CLOCK_MAX: u64 = 2_147_483_647;

/// The clock of one RT-capable encounter (ADR-0029 T2, T3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncounterClock {
    /// Ticks per round, nonzero, copied from the ruleset's `round_ticks`.
    pub round_ticks: u32,
    /// Encounter ticks elapsed, `0..=RT_CLOCK_MAX`.
    pub elapsed: u64,
}

/// One real-time standing order (ADR-0029 T8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandingOrder {
    /// The ordered ability; one listed ability.
    pub ability: Ulid,
    /// The ordered target combatant.
    pub target: EntityId,
    /// Whether the order survives an execution (the ability's flag).
    pub repeat: bool,
}

fn is_turn_based(mode: &EncounterMode) -> bool {
    *mode == EncounterMode::TurnBased
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// One derived `Int` stat formula, symbolic: stat names, never handles (T137).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum IntFormula {
    /// A constant whole number.
    Literal(i32),
    /// The fully modified value of another `Int` stat of the same combatant.
    Stat(String),
    /// Saturating addition.
    Add {
        /// Left operand, evaluated first.
        left: Box<IntFormula>,
        /// Right operand.
        right: Box<IntFormula>,
    },
    /// Saturating subtraction.
    Subtract {
        /// Left operand, evaluated first.
        left: Box<IntFormula>,
        /// Right operand.
        right: Box<IntFormula>,
    },
    /// Saturating multiplication.
    Multiply {
        /// Left operand, evaluated first.
        left: Box<IntFormula>,
        /// Right operand.
        right: Box<IntFormula>,
    },
    /// Flooring division; a zero divisor fails the query.
    Divide {
        /// Left operand, evaluated first.
        left: Box<IntFormula>,
        /// Right operand.
        right: Box<IntFormula>,
    },
    /// The smaller operand.
    Min {
        /// Left operand, evaluated first.
        left: Box<IntFormula>,
        /// Right operand.
        right: Box<IntFormula>,
    },
    /// The larger operand.
    Max {
        /// Left operand, evaluated first.
        left: Box<IntFormula>,
        /// Right operand.
        right: Box<IntFormula>,
    },
}

/// One derived `Int` stat, in authored stat order (T137).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DerivedStat {
    /// The declared stat name.
    pub name: String,
    /// The formula evaluated before the stat's modifiers.
    pub formula: IntFormula,
}

/// Rolled initiative: dice plus stat values plus each participant's key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitiativeRule {
    /// Parsed initiative dice.
    pub dice: DiceExpr,
    /// Non-health `Int` stat names added to the roll, in authored order.
    pub bonus_stats: Vec<String>,
}

/// One attached lifetime-bearing effect on a combatant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachedEffect {
    /// The effect identity attached.
    pub effect: Ulid,
    /// The round at which the attachment lapses (exclusive bound).
    pub expires_round: u32,
}

/// One combatant's authoritative mutable state, keyed by entity in a
/// [`ComponentStore`](crate::ComponentStore). Insertion order is authored
/// participant order, giving the deterministic placement-to-entity mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Combatant {
    placement: Ulid,
    initiative: i32,
    attributes: Vec<(String, i32)>,
    health: u32,
    max_health: u32,
    dead: bool,
    action_pool: ResourcePool,
    /// Non-primary pool balances in template order; skipped when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    extra_pools: Vec<ResourcePool>,
    /// Attached effects in ascending effect identity order; skipped when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    attached: Vec<AttachedEffect>,
    /// The RT standing order; always `None` in TB (ADR-0029 T8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    order: Option<StandingOrder>,
}

impl Combatant {
    /// The placed-creature identity this combatant was instantiated from.
    pub fn placement(&self) -> Ulid {
        self.placement
    }

    /// The authored initiative key scheduling this combatant.
    pub fn initiative(&self) -> i32 {
        self.initiative
    }

    /// Non-health stat values in authored stat order.
    pub fn attributes(&self) -> &[(String, i32)] {
        &self.attributes
    }

    /// Current health, floored at zero by saturating damage.
    pub fn health(&self) -> u32 {
        self.health
    }

    /// Health at encounter start; never exceeded after.
    pub fn max_health(&self) -> u32 {
        self.max_health
    }

    /// Whether this combatant died (health reached zero).
    pub fn dead(&self) -> bool {
        self.dead
    }

    /// The single action pool, refreshed once per logical turn start.
    pub fn action_pool(&self) -> &ResourcePool {
        &self.action_pool
    }

    /// Non-primary pool balances in template order.
    pub fn extra_pools(&self) -> &[ResourcePool] {
        &self.extra_pools
    }

    /// Attached effects in ascending effect identity order.
    pub fn attached(&self) -> &[AttachedEffect] {
        &self.attached
    }

    /// The RT standing order, if any.
    pub fn order(&self) -> Option<StandingOrder> {
        self.order
    }
}

/// How an ability selects its defense value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum AbilityDefense {
    /// Uses the actor's checked attribute value.
    ActorAttribute,
    /// Uses the target's named non-health stat value.
    TargetStat {
        /// Declared non-health stat name read from the target.
        stat: String,
    },
    /// Uses a constant defence value; the target's DC modifiers apply.
    Constant(i32),
    /// Uses the actor's named `Int` stat; the actor's DC modifiers apply.
    ActorStat {
        /// Non-health `Int` stat name read from the actor.
        stat: String,
    },
}

/// Who an effect's modifiers apply to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum EffectAim {
    /// Modifiers apply to the actor that received the effect.
    Slf,
    /// Modifiers apply to the effect target.
    Target,
}

/// Which numeric fold an effect modifier contributes to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum EffectTarget {
    /// Contributes to the Roll fold.
    Roll,
    /// Contributes to the Dc fold.
    Dc,
    /// Contributes to the named `Int` stat of the holder.
    Stat {
        /// Non-health `Int` stat name.
        stat: String,
    },
}

/// How an effect modifier combines with its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum EffectOp {
    /// Adds its value to the running total.
    Add,
    /// Replaces the running total with its value.
    Set,
}

/// One modifier entry inside a persisted effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectModifier {
    /// Stable modifier identity doubling as kernel ordering tiebreak input.
    pub id: Ulid,
    /// Which numeric fold this modifier contributes to.
    pub target: EffectTarget,
    /// How this modifier combines with its base.
    pub op: EffectOp,
    /// Signed contribution value.
    pub value: i32,
    /// Ordering priority; higher runs later.
    pub priority: i16,
    /// Optional name used by per-name stacking policies.
    pub name: Option<String>,
}

/// One persisted effect: lifetime-bearing modifiers with one shared policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectDefinition {
    /// Stable effect identity.
    pub effect: Ulid,
    /// Who the modifiers apply to.
    pub aim: EffectAim,
    /// Shared modifier-type name governing all modifiers in this effect.
    pub mod_type: String,
    /// Stacking policy applied to every modifier in this effect.
    pub policy: StackingPolicy,
    /// Modifiers in authored order; every entry shares `mod_type`.
    pub modifiers: Vec<EffectModifier>,
    /// Lifetime in rounds, always at least one.
    pub duration_rounds: u32,
}

/// One persisted pool template owned by the definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolTemplate {
    /// Shared template identity for per-combatant runtime pools.
    pub id: Ulid,
    /// Maximum balance; combatants initialize current to this value.
    pub max: u32,
    /// Authored refresh trigger.
    pub refresh: RefreshTrigger,
}

/// One persisted per-ability definition addressed by identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbilityDefinition {
    /// Stable ability identity.
    pub ability: Ulid,
    /// Parsed dice expression.
    pub dice: DiceExpr,
    /// Ability-checked attribute name.
    pub attribute: String,
    /// How the defense value is selected.
    pub defense: AbilityDefense,
    /// Built outcome table.
    pub outcome_table: OutcomeTable,
    /// Flat damage per outcome in authored order.
    pub damage: Vec<(Outcome, u32)>,
    /// Per-pool costs in template order with the primary pool first.
    pub costs: Vec<(ResourcePoolId, u32)>,
    /// Whether an accepted use ends the actor's turn.
    pub ends_turn: bool,
    /// Optional effect applied on success.
    pub effect: Option<Ulid>,
    /// Optional zero-based raw-die index for natural-face rules.
    pub natural_die: Option<u32>,
    /// Whether the ability requires a target.
    pub requires_target: bool,
    /// Whether the ability allows self-targeting.
    pub allow_self_target: bool,
    /// RT recovery ticks after a use; `None` means `round_ticks`. Always
    /// `None` in a TB-only encounter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_ticks: Option<u32>,
    /// Whether an RT order for this ability repeats. Always `false` in a
    /// TB-only encounter.
    #[serde(default, skip_serializing_if = "is_false")]
    pub repeats_in_real_time: bool,
    /// Non-health `Int` stat names added to this ability's roll. Skipped
    /// when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bonus_stats: Vec<String>,
    /// Damage dice per outcome, added to the outcome's flat `damage`.
    /// Skipped when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub damage_dice: Vec<(Outcome, DiceExpr)>,
}

/// The immutable, symbolically-serialized combat definition.
///
/// Every field is a symbolic string, ULID, integer, or rules value with its
/// own string-form serialization — never an interned handle — so saves
/// survive differently ordered interners (E014). Legacy single-ability/pool
/// fields are always persisted from the first-listed entry for hash
/// compatibility; the new vectors persist together only for non-legacy
/// shapes and drive execution when present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatDefinition {
    /// Encounter identity this definition was built from.
    pub encounter: Ulid,
    /// Ruleset identity governing the encounter.
    pub ruleset: Ulid,
    /// The encounter ability (the first-listed ability, legacy copy).
    pub ability: Ulid,
    /// Parsed dice expression of the encounter ability (legacy copy).
    pub dice: DiceExpr,
    /// Ability-checked attribute name (legacy copy).
    pub attribute: String,
    /// Name of the health stat.
    pub health_stat: String,
    /// Non-health, non-derived `Int` stat names in authored stat order.
    pub attribute_names: Vec<String>,
    /// Flat damage per outcome in authored order (legacy copy).
    pub damage: Vec<(Outcome, u32)>,
    /// Primary-pool cost of one use (legacy copy).
    pub cost: u32,
    /// Whether the ability requires a target (legacy copy).
    pub requires_target: bool,
    /// Whether the ability allows self-targeting (legacy copy).
    pub allow_self_target: bool,
    /// Built outcome table of the encounter ability (legacy copy).
    pub outcome_table: OutcomeTable,
    /// Shared primary-pool template identity (legacy copy).
    pub pool_id: Ulid,
    /// Primary-pool maximum (legacy copy).
    pub pool_max: u32,
    /// Primary-pool refresh trigger (legacy copy).
    pub pool_refresh: RefreshTrigger,
    /// Per-ability definitions in ascending ability identity order.
    pub abilities: Vec<AbilityDefinition>,
    /// Pool templates in authored template order.
    pub pools: Vec<PoolTemplate>,
    /// Effect definitions in ascending effect identity order.
    pub effects: Vec<EffectDefinition>,
    /// Derived non-health `Int` stats in authored stat order.
    pub derived: Vec<DerivedStat>,
    /// Rolled initiative; `None` means each participant's authored key.
    pub initiative: Option<InitiativeRule>,
}

/// Reports whether one ability entry carries only legacy-compatible behavior.
fn ability_is_legacy(entry: &AbilityDefinition, primary: Ulid) -> bool {
    matches!(entry.defense, AbilityDefense::ActorAttribute)
        && entry.ends_turn
        && entry.effect.is_none()
        && entry.natural_die.is_none()
        && entry.costs.len() == 1
        && entry.costs[0].0 .0 == primary
        && entry.recovery_ticks.is_none()
        && !entry.repeats_in_real_time
        && entry.bonus_stats.is_empty()
        && entry.damage_dice.is_empty()
}

/// Reports whether the new vectors describe a legacy-equivalent shape.
fn vectors_are_legacy(definition: &CombatDefinition) -> bool {
    if definition.abilities.len() != 1
        || definition.pools.len() != 1
        || !definition.effects.is_empty()
    {
        return false;
    }
    ability_is_legacy(&definition.abilities[0], definition.pools[0].id)
}

/// `vectors_are_legacy` plus the two definition-level fields (T137).
fn definition_is_legacy(definition: &CombatDefinition) -> bool {
    vectors_are_legacy(definition)
        && definition.derived.is_empty()
        && definition.initiative.is_none()
}

fn policy_to_text(policy: StackingPolicy) -> &'static str {
    match policy {
        StackingPolicy::StackAll => "stack_all",
        StackingPolicy::HighestBonusWorstPenalty => "highest_bonus_worst_penalty",
        StackingPolicy::HighestPriorityPerName => "highest_priority_per_name",
    }
}

fn policy_from_text(text: &str) -> Option<StackingPolicy> {
    match text {
        "stack_all" => Some(StackingPolicy::StackAll),
        "highest_bonus_worst_penalty" => Some(StackingPolicy::HighestBonusWorstPenalty),
        "highest_priority_per_name" => Some(StackingPolicy::HighestPriorityPerName),
        _ => None,
    }
}

impl Serialize for EffectDefinition {
    /// Serializes the `{effect, aim, mod_type, policy, modifiers,
    /// duration_rounds}` wire object with the policy as stable text.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut shape = serializer.serialize_struct("EffectDefinition", 6)?;
        shape.serialize_field("effect", &self.effect)?;
        shape.serialize_field("aim", &self.aim)?;
        shape.serialize_field("mod_type", &self.mod_type)?;
        shape.serialize_field("policy", policy_to_text(self.policy))?;
        shape.serialize_field("modifiers", &self.modifiers)?;
        shape.serialize_field("duration_rounds", &self.duration_rounds)?;
        shape.end()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectDefinitionRepr {
    effect: Ulid,
    aim: EffectAim,
    mod_type: String,
    policy: String,
    modifiers: Vec<EffectModifier>,
    duration_rounds: u32,
}

impl<'de> Deserialize<'de> for EffectDefinition {
    /// Decodes through the stable policy text, rejecting unknown policies.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let repr = EffectDefinitionRepr::deserialize(deserializer)?;
        let policy = policy_from_text(repr.policy.as_str()).ok_or_else(|| {
            serde::de::Error::custom("effect definition carries an unknown policy")
        })?;
        Ok(Self {
            effect: repr.effect,
            aim: repr.aim,
            mod_type: repr.mod_type,
            policy,
            modifiers: repr.modifiers,
            duration_rounds: repr.duration_rounds,
        })
    }
}

impl Serialize for CombatDefinition {
    /// Serializes legacy fields always, in legacy order, with the new
    /// vectors appended together only for non-legacy shapes. Legacy-equivalent
    /// worlds therefore keep their exact T016-era bytes.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let legacy = definition_is_legacy(self);
        let len = if legacy {
            15
        } else {
            18 + usize::from(!self.derived.is_empty()) + usize::from(self.initiative.is_some())
        };
        let mut shape = serializer.serialize_struct("CombatDefinition", len)?;
        shape.serialize_field("encounter", &self.encounter)?;
        shape.serialize_field("ruleset", &self.ruleset)?;
        shape.serialize_field("ability", &self.ability)?;
        shape.serialize_field("dice", &self.dice)?;
        shape.serialize_field("attribute", &self.attribute)?;
        shape.serialize_field("health_stat", &self.health_stat)?;
        shape.serialize_field("attribute_names", &self.attribute_names)?;
        shape.serialize_field("damage", &self.damage)?;
        shape.serialize_field("cost", &self.cost)?;
        shape.serialize_field("requires_target", &self.requires_target)?;
        shape.serialize_field("allow_self_target", &self.allow_self_target)?;
        shape.serialize_field("outcome_table", &self.outcome_table)?;
        shape.serialize_field("pool_id", &self.pool_id)?;
        shape.serialize_field("pool_max", &self.pool_max)?;
        shape.serialize_field("pool_refresh", &self.pool_refresh)?;
        if !legacy {
            shape.serialize_field("abilities", &self.abilities)?;
            shape.serialize_field("pools", &self.pools)?;
            shape.serialize_field("effects", &self.effects)?;
            if !self.derived.is_empty() {
                shape.serialize_field("derived", &self.derived)?;
            }
            if let Some(initiative) = &self.initiative {
                shape.serialize_field("initiative", initiative)?;
            }
        }
        shape.end()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CombatDefinitionRepr {
    encounter: Ulid,
    ruleset: Ulid,
    ability: Ulid,
    dice: DiceExpr,
    attribute: String,
    health_stat: String,
    attribute_names: Vec<String>,
    damage: Vec<(Outcome, u32)>,
    cost: u32,
    requires_target: bool,
    allow_self_target: bool,
    outcome_table: OutcomeTable,
    pool_id: Ulid,
    pool_max: u32,
    pool_refresh: RefreshTrigger,
    #[serde(default)]
    abilities: Option<Vec<AbilityDefinition>>,
    #[serde(default)]
    pools: Option<Vec<PoolTemplate>>,
    #[serde(default)]
    effects: Option<Vec<EffectDefinition>>,
    #[serde(default)]
    derived: Vec<DerivedStat>,
    #[serde(default)]
    initiative: Option<InitiativeRule>,
}

impl<'de> Deserialize<'de> for CombatDefinition {
    /// Decodes legacy saves by rebuilding the single-entry vectors from the
    /// legacy copies; vector-carrying saves keep their vectors for the
    /// world-level coherence check to validate.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let repr = CombatDefinitionRepr::deserialize(deserializer)?;
        let legacy_present =
            repr.abilities.is_none() && repr.pools.is_none() && repr.effects.is_none();
        if legacy_present {
            let primary = ResourcePoolId(repr.pool_id);
            let rebuilt = AbilityDefinition {
                ability: repr.ability,
                dice: repr.dice.clone(),
                attribute: repr.attribute.clone(),
                defense: AbilityDefense::ActorAttribute,
                outcome_table: repr.outcome_table.clone(),
                damage: repr.damage.clone(),
                costs: vec![(primary, repr.cost)],
                ends_turn: true,
                effect: None,
                natural_die: None,
                requires_target: repr.requires_target,
                allow_self_target: repr.allow_self_target,
                recovery_ticks: None,
                repeats_in_real_time: false,
                bonus_stats: Vec::new(),
                damage_dice: Vec::new(),
            };
            let pools = vec![PoolTemplate {
                id: repr.pool_id,
                max: repr.pool_max,
                refresh: repr.pool_refresh,
            }];
            Ok(Self {
                encounter: repr.encounter,
                ruleset: repr.ruleset,
                ability: repr.ability,
                dice: repr.dice,
                attribute: repr.attribute,
                health_stat: repr.health_stat,
                attribute_names: repr.attribute_names,
                damage: repr.damage,
                cost: repr.cost,
                requires_target: repr.requires_target,
                allow_self_target: repr.allow_self_target,
                outcome_table: repr.outcome_table,
                pool_id: repr.pool_id,
                pool_max: repr.pool_max,
                pool_refresh: repr.pool_refresh,
                abilities: vec![rebuilt],
                pools,
                effects: Vec::new(),
                derived: repr.derived,
                initiative: repr.initiative,
            })
        } else {
            Ok(Self {
                encounter: repr.encounter,
                ruleset: repr.ruleset,
                ability: repr.ability,
                dice: repr.dice,
                attribute: repr.attribute,
                health_stat: repr.health_stat,
                attribute_names: repr.attribute_names,
                damage: repr.damage,
                cost: repr.cost,
                requires_target: repr.requires_target,
                allow_self_target: repr.allow_self_target,
                outcome_table: repr.outcome_table,
                pool_id: repr.pool_id,
                pool_max: repr.pool_max,
                pool_refresh: repr.pool_refresh,
                abilities: repr.abilities.unwrap_or_default(),
                pools: repr.pools.unwrap_or_default(),
                effects: repr.effects.unwrap_or_default(),
                derived: repr.derived,
                initiative: repr.initiative,
            })
        }
    }
}

/// Encounter-level state; `None` when no encounter is active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombatState {
    /// The immutable symbolic combat definition.
    pub definition: CombatDefinition,
    /// Completed rounds; zero until the first rollover.
    pub round: u32,
    /// The entity whose turn it is, or `None` when terminal.
    pub active: Option<EntityId>,
    /// The encounter's time policy; omitted when turn-based (ADR-0029 T1).
    #[serde(default, skip_serializing_if = "is_turn_based")]
    pub mode: EncounterMode,
    /// Round length and clock; `None` means TB-only (ADR-0029 T2, T3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<EncounterClock>,
    /// The requested mode, applied at the next round boundary (ADR-0029 T9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_mode: Option<EncounterMode>,
}

/// One authored participant's resolved references for the adapter.
#[derive(Debug)]
pub struct PlacementAndArea<'a> {
    /// The placed-creature document.
    pub placement: &'a AuthoredPlacement,
    /// The area owning the placement.
    pub area: Ulid,
}

/// Resolved references one encounter adapter consumes.
///
/// All maps are caller-assembled from data-validated documents: the adapter
/// re-checks the combat table (positions below) but does not re-run data
/// validation. The caller must supply every ruleset-listed ability and
/// outcome table the encounter transitively needs, plus every referenced
/// effect.
#[derive(Debug)]
pub struct EncounterSpec<'a> {
    /// The authored encounter (participants in authored order).
    pub encounter: &'a AuthoredEncounter,
    /// The encounter's governing ruleset.
    pub ruleset: &'a AuthoredRuleset,
    /// Ability documents by identity.
    pub abilities: &'a BTreeMap<Ulid, &'a AuthoredAbility>,
    /// Outcome-table documents by identity.
    pub outcome_tables: &'a BTreeMap<Ulid, &'a AuthoredOutcomeTable>,
    /// Placement documents with their owning areas, by placement identity.
    pub placements: &'a BTreeMap<Ulid, PlacementAndArea<'a>>,
    /// Creature prefabs by identity.
    pub creatures: &'a BTreeMap<Ulid, &'a AuthoredCreature>,
    /// Effect documents by identity.
    pub effects: &'a BTreeMap<Ulid, &'a AuthoredEffect>,
}

/// Terminal per-participant result retained by [`end_encounter`] (never
/// persisted).
///
/// Death results survive release through this summary and the queued
/// [`Died`](crate::SimEvent::Died) envelopes: the entities stay live, only
/// their combat scheduling and components are stripped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticipantResult {
    /// The authored placement this combatant was instantiated from.
    pub placement: Ulid,
    /// The runtime entity (stays live across release).
    pub entity: EntityId,
    /// Health at release (zero for the dead).
    pub health: u32,
    /// Whether this combatant died (`health == 0` agreement holds).
    pub dead: bool,
}

/// The retained outcome of one released encounter (never persisted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncounterSummary {
    /// Encounter identity the released fight was built from.
    pub encounter: Ulid,
    /// Ruleset identity governing the released fight.
    pub ruleset: Ulid,
    /// Rounds completed at release.
    pub round: u32,
    /// Per-participant results in authored participant order.
    pub results: Vec<ParticipantResult>,
}

/// One player/authoritative action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatAction {
    /// Use one listed ability from `actor` onto `target`.
    UseAbility {
        /// The acting combatant; must hold the active turn.
        actor: EntityId,
        /// The ability to use; must be one listed ability.
        ability: Ulid,
        /// The targeted combatant.
        target: EntityId,
    },
    /// Complete the active turn without spending, drawing, damage, or effect.
    EndTurn {
        /// The acting combatant; must hold the active turn.
        actor: EntityId,
    },
    /// Clear `actor`'s RT standing order (ADR-0029 T8). RT only.
    Stop {
        /// The combatant whose order is cleared.
        actor: EntityId,
    },
}

/// The observed result of one accepted action (never persisted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionOutcome {
    /// The acting combatant.
    pub actor: EntityId,
    /// The targeted combatant.
    pub target: EntityId,
    /// The ability that was used.
    pub ability: Ulid,
    /// The dice trace drawn on the combat stream.
    pub roll: DiceRoll,
    /// The unsaturated actor-minus-defence difference.
    pub margin: i64,
    /// The table-selected outcome.
    pub outcome: Outcome,
    /// The flat damage applied (zero on failure or unmapped outcomes).
    pub damage: u32,
    /// Whether this action killed the target.
    pub target_died: bool,
}

/// Every combat failure, with pinned multifault precedence.
///
/// Action-time variants check in declaration order. `Display` is
/// `<Code> at <location>` with a stable location per variant; new variants
/// hide fields, matching the existing style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CombatError {
    /// No encounter is active in the world.
    NoEncounter,
    /// An encounter is already active in the world.
    EncounterActive,
    /// The entity is live but not a combatant.
    NotParticipant {
        /// The live non-combatant entity.
        entity: EntityId,
    },
    /// The actor id addresses no live combatant.
    AbsentActor {
        /// The absent actor.
        actor: EntityId,
    },
    /// The target id addresses no live combatant.
    AbsentTarget {
        /// The absent target.
        target: EntityId,
    },
    /// The actor combatant is dead.
    DeadActor {
        /// The dead actor.
        actor: EntityId,
    },
    /// The target combatant is dead.
    DeadTarget {
        /// The dead target.
        target: EntityId,
    },
    /// The actor does not hold the active turn.
    OutOfTurn {
        /// The acting entity.
        actor: EntityId,
        /// The entity holding the turn, if any.
        active: Option<EntityId>,
    },
    /// The ability is not one listed ability (or the spec map lacks one
    /// listed ability at adapter time).
    UnknownAbility {
        /// The unknown ability identity.
        ability: Ulid,
    },
    /// The ability disallows self-targeting and target equals actor.
    SelfTarget,
    /// One pool cannot afford its ability cost.
    InsufficientAction {
        /// The pool that cannot afford its cost.
        pool: Ulid,
        /// The pool's cost.
        cost: u32,
        /// The pool's current balance.
        current: u32,
    },
    /// A participant's placement is absent from the spec.
    MissingPlacement {
        /// The unresolvable placement identity.
        placement: Ulid,
    },
    /// Participants span more than one area.
    MixedArea {
        /// The first participant's area.
        area_a: Ulid,
        /// The disagreeing participant's area.
        area_b: Ulid,
    },
    /// A placement's prefab is not a supplied creature.
    MissingCreature {
        /// The unresolvable prefab identity.
        prefab: Ulid,
    },
    /// A creature lacks a ruleset stat (or an ability names an
    /// undeclared attribute, or a defense names an undeclared stat).
    MissingStat {
        /// The absent stat name.
        stat: String,
    },
    /// A combat stat value is fractional, or the health value is not
    /// positive.
    InvalidStatValue {
        /// The offending stat name.
        stat: String,
    },
    /// An ability cost is empty or exceeds its pool maximum.
    InvalidCost {
        /// The offending cost.
        cost: u32,
        /// The pool maximum.
        max: u32,
    },
    /// An ability dice notation fails to parse.
    InvalidDice,
    /// An outcome table is missing or fails rules construction.
    InvalidOutcomeTable,
    /// A checked conversion overflowed (defensive; whole fixed values
    /// always fit, per the data contract).
    ValueOverflow,
    /// Participants exceed [`MAX_COMBATANTS`].
    TooManyParticipants,
    /// Ruleset stats exceed [`MAX_COMBAT_STATS`].
    TooManyStats,
    /// An ability names a pool outside its ruleset templates.
    MissingPool {
        /// The undeclared pool identity.
        pool: Ulid,
    },
    /// An ability effect resolves outside the supplied effect map.
    MissingEffect {
        /// The unresolvable effect identity.
        effect: Ulid,
    },
    /// An effect carries an empty mod-type, no modifiers, a nonpositive
    /// duration, or an operation its policy rejects.
    InvalidEffect {
        /// The offending effect identity.
        effect: Ulid,
    },
    /// One modifier identity appears in more than one effect entry.
    DuplicateModifier {
        /// The duplicated modifier identity.
        id: Ulid,
    },
    /// A natural selector names no drawn die.
    InvalidNaturalDie {
        /// The offending selector index.
        index: u32,
    },
    /// One modifier type names two different policies across effects.
    PolicyConflict {
        /// The conflicting modifier-type name.
        mod_type: String,
    },
    /// The encounter's participants resolve to an area other than the bound
    /// world's (T027a, ADR-0020).
    AreaMismatch {
        /// The bound world's area.
        expected: Ulid,
        /// The participants' resolved area.
        found: Ulid,
    },
    /// An authored surface feature the adapter does not execute yet (T137).
    UnsupportedAuthored,
    /// RT was requested for an encounter whose ruleset declares no
    /// `round_ticks` (ADR-0029 T2).
    ModeUnsupported,
    /// The action does not exist in the encounter's current mode:
    /// `EndTurn` in RT, `Stop` in TB (ADR-0029 T8).
    WrongMode {
        /// The encounter's current mode.
        mode: EncounterMode,
    },
    /// A derived stat the action reads fails to evaluate (a zero divisor).
    StatEvaluation {
        /// The queried stat whose evaluation failed.
        stat: String,
    },
}

impl std::fmt::Display for CombatError {
    /// Renders `<Code> at <location>` with a stable location per variant.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoEncounter => write!(f, "NoEncounter at combat"),
            Self::EncounterActive => write!(f, "EncounterActive at combat"),
            Self::NotParticipant { entity } => {
                write!(f, "NotParticipant at combatants/{entity:?}")
            }
            Self::AbsentActor { actor } => write!(f, "AbsentActor at combatants/{actor:?}"),
            Self::AbsentTarget { target } => {
                write!(f, "AbsentTarget at combatants/{target:?}")
            }
            Self::DeadActor { actor } => write!(f, "DeadActor at combatants/{actor:?}"),
            Self::DeadTarget { target } => write!(f, "DeadTarget at combatants/{target:?}"),
            Self::OutOfTurn { .. } => write!(f, "OutOfTurn at combat/active"),
            Self::UnknownAbility { .. } => write!(f, "UnknownAbility at combat/ability"),
            Self::SelfTarget => write!(f, "SelfTarget at combat/target"),
            Self::InsufficientAction { .. } => write!(f, "InsufficientAction at combat/pool"),
            Self::MissingPlacement { .. } => write!(f, "MissingPlacement at spec/placements"),
            Self::MixedArea { .. } => write!(f, "MixedArea at spec/areas"),
            Self::MissingCreature { .. } => write!(f, "MissingCreature at spec/creatures"),
            Self::MissingStat { .. } => write!(f, "MissingStat at spec/stats"),
            Self::InvalidStatValue { .. } => write!(f, "InvalidStatValue at spec/stats"),
            Self::InvalidCost { .. } => write!(f, "InvalidCost at spec/cost"),
            Self::InvalidDice => write!(f, "InvalidDice at spec/dice"),
            Self::InvalidOutcomeTable => write!(f, "InvalidOutcomeTable at spec/outcome-table"),
            Self::ValueOverflow => write!(f, "ValueOverflow at spec/conversion"),
            Self::TooManyParticipants => write!(f, "TooManyParticipants at spec/participants"),
            Self::TooManyStats => write!(f, "TooManyStats at spec/stats"),
            Self::MissingPool { .. } => write!(f, "MissingPool at spec/pools"),
            Self::MissingEffect { .. } => write!(f, "MissingEffect at spec/effects"),
            Self::InvalidEffect { .. } => write!(f, "InvalidEffect at spec/effects"),
            Self::DuplicateModifier { .. } => write!(f, "DuplicateModifier at spec/effects"),
            Self::InvalidNaturalDie { .. } => write!(f, "InvalidNaturalDie at spec/ability"),
            Self::PolicyConflict { .. } => write!(f, "PolicyConflict at spec/effects"),
            Self::AreaMismatch { .. } => write!(f, "AreaMismatch at spec/areas"),
            Self::UnsupportedAuthored => write!(f, "UnsupportedAuthored at spec/authored"),
            Self::ModeUnsupported => write!(f, "ModeUnsupported at combat/mode"),
            Self::WrongMode { .. } => write!(f, "WrongMode at combat/mode"),
            Self::StatEvaluation { .. } => write!(f, "StatEvaluation at combat/stats"),
        }
    }
}

impl std::error::Error for CombatError {}

/// Branch-observed transition facts for the opt-in history wrapper.
///
/// Recorded by the controller branches that actually fire — never inferred
/// from changed actor ids or round arithmetic — so logical turn starts are
/// still reported when the same actor returns, including saturating `u32`
/// round rollover, and terminal transitions are reported exactly once on the
/// first active-`Some` to active-`None` edge (T020, ADR-0017).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct TransitionFacts {
    /// The new logical turn head with its round, when an advance branch
    /// refreshed one.
    pub turn_started: Option<(EntityId, u32)>,
    /// The naturally completed encounter with its round, on the first
    /// transition to terminal.
    pub encounter_ended: Option<(Ulid, u32)>,
    /// The mode now in force with the round the switch started, when a
    /// pending mode applied at a boundary or rollover (ADR-0029 T10).
    pub mode_changed: Option<(EncounterMode, u32)>,
    /// The real-time round that started (ADR-0029 T6), when one did.
    pub round_started: Option<u32>,
}

/// One stage-3 fact, in execution order (T138b).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TickEvent {
    /// One due order resolved through [`resolve_use`].
    Resolved(ActionOutcome),
    /// One due order failed validation and was cleared.
    OrderEnded {
        /// The combatant whose order ended.
        actor: EntityId,
    },
}

/// Everything one tick observed, for the history wrapper (T138b).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct TickFacts {
    /// The stage-2 transition and the terminal edge of stage 3.
    pub transitions: TransitionFacts,
    /// The stage-3 facts in execution order.
    pub events: Vec<TickEvent>,
}

/// Initial-turn facts one encounter start actually published.
///
/// The incoming turn head with its round (always round zero), when the
/// timeline scheduled one (T020, ADR-0017).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct StartFacts {
    /// The initial turn head with its round, when scheduled.
    pub initial_turn: Option<(EntityId, u32)>,
}

/// Maps one authored outcome label to its runtime counterpart.
fn map_outcome(outcome: crpg_data::OutcomeWire) -> Outcome {
    match outcome {
        crpg_data::OutcomeWire::CriticalSuccess => Outcome::CriticalSuccess,
        crpg_data::OutcomeWire::Success => Outcome::Success,
        crpg_data::OutcomeWire::Failure => Outcome::Failure,
        crpg_data::OutcomeWire::CriticalFailure => Outcome::CriticalFailure,
        crpg_data::OutcomeWire::Custom(byte) => Outcome::Custom(byte),
    }
}

/// Converts one authored refresh trigger to its runtime counterpart.
///
/// A zero tick period violates data's read-time validation, which owns
/// that invariant; encountering one here is a caller-contract breach.
fn convert_refresh(refresh: AuthoredRefresh) -> RefreshTrigger {
    match refresh {
        AuthoredRefresh::OnTurnStart => RefreshTrigger::OnTurnStart,
        AuthoredRefresh::OnRoundStart => RefreshTrigger::OnRoundStart,
        AuthoredRefresh::OnRest(rest) => RefreshTrigger::OnRest(RulesRestId(rest.0)),
        AuthoredRefresh::OnTick(period) => {
            assert!(
                period.0 != 0,
                "pool tick period is validated nonzero by data read-time checks"
            );
            RefreshTrigger::OnTick(period.0)
        }
        AuthoredRefresh::Never => RefreshTrigger::Never,
    }
}

/// Maps one authored stacking policy to its runtime counterpart.
fn map_policy(policy: crpg_data::PolicyWire) -> StackingPolicy {
    match policy {
        crpg_data::PolicyWire::StackAll => StackingPolicy::StackAll,
        crpg_data::PolicyWire::HighestBonusWorstPenalty => StackingPolicy::HighestBonusWorstPenalty,
        crpg_data::PolicyWire::HighestPriorityPerName => StackingPolicy::HighestPriorityPerName,
    }
}

/// Validates and publishes one encounter into `world`, all-or-nothing.
///
/// Every check below runs before any entity is spawned or any state is
/// published; any error leaves `world` unchanged (no entities, no combat,
/// empty timeline, empty events, unchanged RNG streams).
///
/// Check order (pinned): an already-active encounter; collection bounds;
/// unsupported authored features; each participant's placement, then
/// single-area agreement, then each placement's creature; per creature, per
/// ruleset stat in authored order, presence, wholeness, and health positivity
/// with checked conversion (only base `Int` stats are read; a base `Fixed`
/// stat must be present, T137); the derived-stat preflight (`InvalidStatValue`
/// naming the derived stat); pool templates carried in order; per
/// ruleset-listed ability in authored order, the spec-map entry,
/// declared-attribute, dice, outcome-table, per-pool costs in template order,
/// defense, bonus-stat, effect, and natural checks; global effect checks in
/// ascending effect order (reserved bonus type, then per modifier the policy
/// checks and the `Stat` target name); then publication.
///
/// Data-owned preconditions (enforced at data read time, asserted here):
/// participants and the ruleset ability list are nonempty.
pub fn start_encounter(world: &mut World, spec: &EncounterSpec<'_>) -> Result<(), CombatError> {
    start_encounter_tracked(world, spec).map(|_| ())
}

/// Validates and publishes one encounter in the host-chosen `mode`
/// (ADR-0029 T11); otherwise identical to [`start_encounter`].
///
/// `RealTime` on a ruleset without `round_ticks` fails `ModeUnsupported`
/// after `EncounterActive` and before every other check, leaving `world`
/// unchanged. A real-time publication keys every combatant at `0`, holds no
/// active turn and refreshes no pool.
pub fn start_encounter_in_mode(
    world: &mut World,
    spec: &EncounterSpec<'_>,
    mode: EncounterMode,
) -> Result<(), CombatError> {
    start_encounter_in_mode_tracked(world, spec, mode).map(|_| ())
}

/// Validates and publishes one encounter into `world`, reporting the initial
/// turn the publication actually scheduled.
///
/// Crate-visible only: [`start_encounter`] delegates here and discards the
/// facts, so legacy emissions, bytes and signatures are unchanged; the
/// history wrapper consumes the facts to journal its initial `TurnStarted`
/// (T020, ADR-0017).
pub(crate) fn start_encounter_tracked(
    world: &mut World,
    spec: &EncounterSpec<'_>,
) -> Result<StartFacts, CombatError> {
    start_encounter_in_mode_tracked(world, spec, EncounterMode::TurnBased)
}

/// [`start_encounter_tracked`] in the host-chosen `mode` (ADR-0029 T11).
///
/// Crate-visible only. The turn-based wrapper above keeps its two-argument
/// signature because `src/history/tests.rs` holds a second caller.
pub(crate) fn start_encounter_in_mode_tracked(
    world: &mut World,
    spec: &EncounterSpec<'_>,
    mode: EncounterMode,
) -> Result<StartFacts, CombatError> {
    use CombatError::{
        DuplicateModifier, EncounterActive, InvalidCost, InvalidDice, InvalidEffect,
        InvalidNaturalDie, InvalidOutcomeTable, InvalidStatValue, MissingCreature, MissingEffect,
        MissingPlacement, MissingPool, MissingStat, MixedArea, PolicyConflict, TooManyParticipants,
        TooManyStats, UnknownAbility, UnsupportedAuthored, ValueOverflow,
    };

    let encounter = spec.encounter;
    let ruleset = spec.ruleset;

    if world.combat().is_some() {
        return Err(EncounterActive);
    }
    if mode == EncounterMode::RealTime && ruleset.round_ticks.is_none() {
        return Err(CombatError::ModeUnsupported);
    }
    if encounter.participants.len() > MAX_COMBATANTS {
        return Err(TooManyParticipants);
    }
    if ruleset.stats.len() > MAX_COMBAT_STATS {
        return Err(TooManyStats);
    }
    if encounter.participants.is_empty() {
        panic!("encounter participants are validated nonempty by data read-time checks");
    }
    if ruleset.abilities.is_empty() {
        panic!("ruleset abilities are validated nonempty by data read-time checks");
    }
    if ruleset.round_ticks == Some(0) {
        panic!("ruleset round_ticks is validated nonzero by data read-time checks");
    }
    if ruleset.round_ticks.is_some() {
        for ability_id in &ruleset.abilities {
            if let Some(ability) = spec.abilities.get(ability_id) {
                if ability.recovery_ticks == Some(0) {
                    panic!("ability recovery_ticks is validated nonzero by data read-time checks");
                }
            }
        }
    }
    let unsupported = ruleset.initiative.is_some()
        || spec.abilities.values().any(|ability| {
            matches!(&ability.defense, DefenseWire::Opposed { .. })
                || ability.damage.iter().any(|entry| entry.dice.is_some())
        })
        || spec.effects.values().any(|effect| effect.grants.is_some());
    if unsupported {
        return Err(UnsupportedAuthored);
    }
    // T137 B3, B4: the base usable stats and the derived `Int` formulas.
    let int_names: BTreeSet<&str> = ruleset
        .stats
        .iter()
        .filter(|declaration| matches!(declaration.kind, StatKindWire::Int))
        .map(|declaration| declaration.name.as_str())
        .collect();
    let mut derived: Vec<DerivedStat> = Vec::new();
    for declaration in &ruleset.stats {
        if let (StatKindWire::Int, Some(formula)) = (&declaration.kind, &declaration.derived) {
            derived.push(DerivedStat {
                name: declaration.name.clone(),
                formula: convert_formula(formula, &int_names),
            });
        }
    }
    let attribute_names: Vec<String> = ruleset
        .stats
        .iter()
        .filter(|declaration| {
            matches!(declaration.kind, StatKindWire::Int)
                && declaration.derived.is_none()
                && declaration.name != ruleset.health_stat
        })
        .map(|declaration| declaration.name.clone())
        .collect();
    let usable = |name: &str| {
        attribute_names.iter().any(|entry| entry == name)
            || derived.iter().any(|entry| entry.name == name)
    };
    let clock = ruleset.round_ticks.map(|round_ticks| EncounterClock {
        round_ticks,
        elapsed: 0,
    });

    let mut areas = Vec::with_capacity(encounter.participants.len());
    let mut placements = Vec::with_capacity(encounter.participants.len());
    for participant in &encounter.participants {
        let resolved = spec
            .placements
            .get(&participant.placement)
            .ok_or(MissingPlacement {
                placement: participant.placement,
            })?;
        areas.push(resolved.area);
        placements.push((participant, resolved));
    }
    let first_area = areas[0];
    for area in areas.iter().skip(1) {
        if *area != first_area {
            return Err(MixedArea {
                area_a: first_area,
                area_b: *area,
            });
        }
    }
    if let Some(expected) = world.area() {
        if expected != first_area {
            return Err(CombatError::AreaMismatch {
                expected,
                found: first_area,
            });
        }
    }

    struct Pending {
        placement: Ulid,
        initiative: i32,
        attributes: Vec<(String, i32)>,
        health: u32,
    }

    let mut pending = Vec::with_capacity(encounter.participants.len());
    for (participant, resolved) in &placements {
        let creature = spec
            .creatures
            .get(&resolved.placement.prefab)
            .ok_or(MissingCreature {
                prefab: resolved.placement.prefab,
            })?;
        let mut stats = Vec::with_capacity(ruleset.stats.len());
        let mut health = None;
        for declaration in &ruleset.stats {
            if declaration.derived.is_some() {
                continue;
            }
            match &declaration.kind {
                StatKindWire::Int => {}
                StatKindWire::Fixed => {
                    if !creature.stats.contains_key(&declaration.name) {
                        return Err(MissingStat {
                            stat: declaration.name.clone(),
                        });
                    }
                    continue;
                }
                _ => continue,
            }
            let fixed = creature.stats.get(&declaration.name).ok_or(MissingStat {
                stat: declaration.name.clone(),
            })?;
            let raw = fixed.to_raw();
            if raw % 65536 != 0 {
                return Err(InvalidStatValue {
                    stat: declaration.name.clone(),
                });
            }
            let wide = i64::from(raw) / 65536;
            let value = i32::try_from(wide).map_err(|_| ValueOverflow)?;
            if declaration.name == ruleset.health_stat {
                if wide <= 0 {
                    return Err(InvalidStatValue {
                        stat: declaration.name.clone(),
                    });
                }
                let positive = u32::try_from(wide).map_err(|_| ValueOverflow)?;
                health = Some(positive);
            }
            stats.push((declaration.name.clone(), value));
        }
        let health = health.expect("the health stat names a declared stat");
        let mut attributes = Vec::with_capacity(stats.len());
        for (name, value) in &stats {
            if *name != ruleset.health_stat {
                attributes.push((name.clone(), *value));
            }
        }
        pending.push(Pending {
            placement: resolved.placement.id,
            initiative: participant.initiative,
            attributes,
            health,
        });
    }

    if !derived.is_empty() {
        let mut scratch = world.interners().clone();
        let definitions = int_definitions(
            &ruleset.health_stat,
            &attribute_names,
            &derived,
            &mut scratch,
        );
        let pipeline = ModifierPipeline::new(definitions, BTreeMap::new())
            .unwrap_or_else(|_| panic!("{DERIVED_PRECONDITION}"));
        let mut probe_arena: GenerationalArena<()> = GenerationalArena::new();
        let probe = probe_arena.insert(());
        let no_tags = TagSet::new();
        for member in &pending {
            let block = base_block(
                &ruleset.health_stat,
                member.health,
                &member.attributes,
                &mut scratch,
            );
            let context = QueryContext {
                entity: probe,
                stats: &block,
                tags: &no_tags,
                modifiers: &[],
            };
            for stat in &derived {
                if query_int(&pipeline, probe, &stat.name, &scratch, &context).is_none() {
                    return Err(InvalidStatValue {
                        stat: stat.name.clone(),
                    });
                }
            }
        }
    }

    if ruleset.pools.is_empty() {
        panic!("ruleset pools are validated nonempty by data read-time checks");
    }
    let mut templates = Vec::with_capacity(ruleset.pools.len());
    for template in &ruleset.pools {
        templates.push(PoolTemplate {
            id: template.id,
            max: template.max,
            refresh: convert_refresh(template.refresh),
        });
    }
    let template_index = |pool: Ulid| templates.iter().position(|entry| entry.id == pool);
    let template_max = |pool: Ulid| {
        templates
            .iter()
            .find(|entry| entry.id == pool)
            .map(|entry| entry.max)
    };

    struct ValidatedAbility<'a> {
        id: Ulid,
        authored: &'a AuthoredAbility,
        dice: DiceExpr,
        table: OutcomeTable,
        costs: Vec<(ResourcePoolId, u32)>,
    }

    let mut validated: Vec<ValidatedAbility<'_>> = Vec::with_capacity(ruleset.abilities.len());
    for ability_id in &ruleset.abilities {
        let ability = spec.abilities.get(ability_id).ok_or(UnknownAbility {
            ability: *ability_id,
        })?;
        if !ruleset
            .attributes
            .iter()
            .any(|name| name == &ability.attribute)
            || !usable(&ability.attribute)
        {
            return Err(MissingStat {
                stat: ability.attribute.clone(),
            });
        }
        let dice: DiceExpr = ability.dice.parse().map_err(|_| InvalidDice)?;
        let authored = spec
            .outcome_tables
            .get(&ability.outcome_table)
            .ok_or(InvalidOutcomeTable)?;
        let bands = authored
            .bands
            .iter()
            .map(|band| OutcomeBand {
                min_margin: band.min_margin,
                outcome: map_outcome(band.outcome),
            })
            .collect();
        let natural_rules = authored
            .natural_rules
            .iter()
            .map(|rule| NaturalRule {
                face: rule.face,
                effect: match rule.effect {
                    crpg_data::NaturalEffectWire::Shift(steps) => NaturalEffect::Shift(steps),
                    crpg_data::NaturalEffectWire::Override(outcome) => {
                        NaturalEffect::Override(map_outcome(outcome))
                    }
                },
            })
            .collect();
        let table = OutcomeTable::new(OutcomeTableId(authored.id), bands, natural_rules)
            .map_err(|_| InvalidOutcomeTable)?;
        let mut costs = Vec::new();
        if ability.cost > 0 {
            costs.push((ResourcePoolId(templates[0].id), ability.cost));
        }
        let mut ordered_extra: Vec<(usize, Ulid, u32)> =
            Vec::with_capacity(ability.extra_costs.len());
        for entry in &ability.extra_costs {
            match template_index(entry.pool) {
                Some(index) => ordered_extra.push((index, entry.pool, entry.amount)),
                None => {
                    return Err(MissingPool { pool: entry.pool });
                }
            }
        }
        ordered_extra.sort_by_key(|(index, _, _)| *index);
        for (_, pool, amount) in ordered_extra {
            costs.push((ResourcePoolId(pool), amount));
        }
        if costs.is_empty() {
            return Err(InvalidCost {
                cost: 0,
                max: templates[0].max,
            });
        }
        for (pool, amount) in &costs {
            let max = template_max(pool.0).expect("cost pools resolve above");
            if *amount == 0 || *amount > max {
                return Err(InvalidCost { cost: *amount, max });
            }
        }
        if let crpg_data::DefenseWire::TargetStat { stat }
        | crpg_data::DefenseWire::ActorStat { stat } = &ability.defense
        {
            if !usable(stat) {
                return Err(MissingStat { stat: stat.clone() });
            }
        }
        for name in ability.bonus_stats.iter().flatten() {
            if !usable(name) {
                return Err(MissingStat { stat: name.clone() });
            }
        }
        if let Some(effect) = ability.effect {
            if !spec.effects.contains_key(&effect) {
                return Err(MissingEffect { effect });
            }
        }
        if let Some(index) = ability.natural_die {
            if index >= dice.count() {
                return Err(InvalidNaturalDie { index });
            }
        }
        validated.push(ValidatedAbility {
            id: *ability_id,
            authored: ability,
            dice,
            table,
            costs,
        });
    }

    let mut referenced: Vec<Ulid> = Vec::new();
    for entry in &validated {
        if let Some(effect) = entry.authored.effect {
            if !referenced.contains(&effect) {
                referenced.push(effect);
            }
        }
    }
    referenced.sort();
    let carries_bonus_stats = validated.iter().any(|entry| {
        entry
            .authored
            .bonus_stats
            .as_ref()
            .is_some_and(|names| !names.is_empty())
            || matches!(
                &entry.authored.defense,
                crpg_data::DefenseWire::Opposed { bonus_stats, .. } if !bonus_stats.is_empty()
            )
    });
    for effect in &referenced {
        let authored = spec
            .effects
            .get(effect)
            .expect("effect references resolve above");
        if authored.duration_rounds == 0
            || authored.modifiers.is_empty()
            || authored.mod_type.is_empty()
        {
            return Err(InvalidEffect { effect: *effect });
        }
        if carries_bonus_stats && authored.mod_type == COMBAT_BONUS_MOD_TYPE {
            return Err(InvalidEffect { effect: *effect });
        }
        let policy = map_policy(authored.policy);
        for modifier in &authored.modifiers {
            let nameless = modifier.name.is_none();
            let is_set = matches!(modifier.op, crpg_data::EffectOpWire::Set);
            if is_set && matches!(policy, StackingPolicy::HighestBonusWorstPenalty) {
                return Err(InvalidEffect { effect: *effect });
            }
            if nameless && matches!(policy, StackingPolicy::HighestPriorityPerName) {
                return Err(InvalidEffect { effect: *effect });
            }
            if let crpg_data::EffectTargetWire::Stat { stat } = &modifier.target {
                if !usable(stat) {
                    return Err(MissingStat { stat: stat.clone() });
                }
            }
        }
    }
    {
        let mut ordered_ids: Vec<Ulid> = Vec::new();
        for effect in &referenced {
            let authored = spec
                .effects
                .get(effect)
                .expect("effect references resolve above");
            for modifier in &authored.modifiers {
                ordered_ids.push(modifier.id);
            }
        }
        ordered_ids.sort();
        let mut duplicate: Option<Ulid> = None;
        for pair in ordered_ids.windows(2) {
            if pair[0] == pair[1] {
                duplicate = Some(pair[0]);
                break;
            }
        }
        if let Some(id) = duplicate {
            return Err(DuplicateModifier { id });
        }
    }
    {
        let mut by_type: BTreeMap<&str, StackingPolicy> = BTreeMap::new();
        let mut conflicts: Vec<String> = Vec::new();
        for effect in &referenced {
            let authored = spec
                .effects
                .get(effect)
                .expect("effect references resolve above");
            let policy = map_policy(authored.policy);
            match by_type.get(authored.mod_type.as_str()) {
                None => {
                    by_type.insert(authored.mod_type.as_str(), policy);
                }
                Some(kept) if *kept != policy => {
                    if !conflicts.contains(&authored.mod_type) {
                        conflicts.push(authored.mod_type.clone());
                    }
                }
                Some(_) => {}
            }
        }
        conflicts.sort();
        if let Some(mod_type) = conflicts.into_iter().next() {
            return Err(PolicyConflict { mod_type });
        }
    }

    let mut abilities: Vec<AbilityDefinition> = Vec::with_capacity(validated.len());
    for entry in &validated {
        let defense = match &entry.authored.defense {
            crpg_data::DefenseWire::ActorAttribute => AbilityDefense::ActorAttribute,
            crpg_data::DefenseWire::TargetStat { stat } => {
                AbilityDefense::TargetStat { stat: stat.clone() }
            }
            crpg_data::DefenseWire::Constant(value) => AbilityDefense::Constant(*value),
            crpg_data::DefenseWire::ActorStat { stat } => {
                AbilityDefense::ActorStat { stat: stat.clone() }
            }
            crpg_data::DefenseWire::Opposed { .. } => {
                unreachable!("rejected as UnsupportedAuthored at encounter start")
            }
        };
        let damage = entry
            .authored
            .damage
            .iter()
            .map(|item| (map_outcome(item.outcome), item.amount))
            .collect();
        abilities.push(AbilityDefinition {
            ability: entry.id,
            dice: entry.dice.clone(),
            attribute: entry.authored.attribute.clone(),
            defense,
            outcome_table: entry.table.clone(),
            damage,
            costs: entry.costs.clone(),
            ends_turn: entry.authored.ends_turn,
            effect: entry.authored.effect,
            natural_die: entry.authored.natural_die,
            requires_target: entry.authored.requires_target,
            allow_self_target: entry.authored.allow_self_target,
            recovery_ticks: if clock.is_some() {
                entry.authored.recovery_ticks
            } else {
                None
            },
            repeats_in_real_time: clock.is_some()
                && entry.authored.repeats_in_real_time.unwrap_or(false),
            bonus_stats: entry.authored.bonus_stats.clone().unwrap_or_default(),
            damage_dice: Vec::new(),
        });
    }
    abilities.sort_by_key(|entry| entry.ability);
    let mut effects: Vec<EffectDefinition> = Vec::with_capacity(referenced.len());
    for effect in &referenced {
        let authored = spec
            .effects
            .get(effect)
            .expect("effect references resolve above");
        let modifiers = authored
            .modifiers
            .iter()
            .map(|item| EffectModifier {
                id: item.id,
                target: match &item.target {
                    crpg_data::EffectTargetWire::Roll => EffectTarget::Roll,
                    crpg_data::EffectTargetWire::Dc => EffectTarget::Dc,
                    crpg_data::EffectTargetWire::Stat { stat } => {
                        EffectTarget::Stat { stat: stat.clone() }
                    }
                },
                op: match item.op {
                    crpg_data::EffectOpWire::Add => EffectOp::Add,
                    crpg_data::EffectOpWire::Set => EffectOp::Set,
                },
                value: item.value,
                priority: item.priority,
                name: item.name.clone(),
            })
            .collect();
        effects.push(EffectDefinition {
            effect: authored.id,
            aim: match authored.aim {
                crpg_data::EffectAimWire::Slf => EffectAim::Slf,
                crpg_data::EffectAimWire::Target => EffectAim::Target,
            },
            mod_type: authored.mod_type.clone(),
            policy: map_policy(authored.policy),
            modifiers,
            duration_rounds: authored.duration_rounds,
        });
    }
    effects.sort_by_key(|entry| entry.effect);

    let first_id = ruleset.abilities[0];
    let first = validated
        .iter()
        .find(|entry| entry.id == first_id)
        .expect("the first-listed ability validates above");
    let primary = &templates[0];
    let first_damage = first
        .authored
        .damage
        .iter()
        .map(|item| (map_outcome(item.outcome), item.amount))
        .collect();
    let definition = CombatDefinition {
        encounter: encounter.id,
        ruleset: ruleset.id,
        ability: first.id,
        dice: first.dice.clone(),
        attribute: first.authored.attribute.clone(),
        health_stat: ruleset.health_stat.clone(),
        attribute_names,
        damage: first_damage,
        cost: first.authored.cost,
        requires_target: first.authored.requires_target,
        allow_self_target: first.authored.allow_self_target,
        outcome_table: first.table.clone(),
        pool_id: primary.id,
        pool_max: primary.max,
        pool_refresh: primary.refresh,
        abilities,
        pools: templates,
        effects,
        derived,
        initiative: None,
    };

    for member in pending {
        let id = world.spawn(EntityMeta {});
        let mut extra = Vec::new();
        for template in definition.pools.iter().skip(1) {
            let pool = ResourcePool::new(
                ResourcePoolId(template.id),
                template.max,
                template.max,
                template.refresh,
            )
            .expect("pool templates carry validated data values");
            extra.push(pool);
        }
        let pool = ResourcePool::new(
            ResourcePoolId(definition.pool_id),
            definition.pool_max,
            definition.pool_max,
            definition.pool_refresh,
        )
        .expect("the pool template carries validated data values");
        world.combatants_mut().insert(
            id,
            Combatant {
                placement: member.placement,
                initiative: member.initiative,
                attributes: member.attributes,
                health: member.health,
                max_health: member.health,
                dead: false,
                action_pool: pool,
                extra_pools: extra,
                attached: Vec::new(),
                order: None,
            },
        );
        let key = match mode {
            EncounterMode::TurnBased => member.initiative,
            EncounterMode::RealTime => 0,
        };
        world.timeline_mut().insert(InitiativeKey(key), id);
    }
    world.interners_mut().intern_tag(COMBAT_ROLL_TAG);
    let active = match mode {
        EncounterMode::TurnBased => world.timeline().iter().next().map(|(_, id)| id),
        EncounterMode::RealTime => None,
    };
    if let Some(head) = active {
        let combatant = world
            .combatants_mut()
            .get_mut(head)
            .expect("scheduled entities are combatants");
        combatant.action_pool.refresh(RefreshEvent::TurnStart);
        for pool in combatant.extra_pools.iter_mut() {
            pool.refresh(RefreshEvent::TurnStart);
        }
    }
    *world.combat_mut() = Some(CombatState {
        definition,
        round: 0,
        active,
        mode,
        clock,
        pending_mode: None,
    });
    Ok(StartFacts {
        initial_turn: active.map(|actor| (actor, 0)),
    })
}

/// Looks up one ability entry by identity, scanning in ascending order.
fn ability_entry(definition: &CombatDefinition, ability: Ulid) -> Option<&AbilityDefinition> {
    definition
        .abilities
        .iter()
        .find(|entry| entry.ability == ability)
}

/// Looks up one effect entry by identity, scanning in ascending order.
fn effect_entry(definition: &CombatDefinition, effect: Ulid) -> Option<&EffectDefinition> {
    definition
        .effects
        .iter()
        .find(|entry| entry.effect == effect)
}

/// Returns the balance of one template pool on one combatant.
fn pool_balance(combatant: &Combatant, pool: Ulid) -> Option<u32> {
    if combatant.action_pool.id().0 == pool {
        return Some(combatant.action_pool.current());
    }
    combatant
        .extra_pools
        .iter()
        .find(|entry| entry.id().0 == pool)
        .map(|entry| entry.current())
}

/// Converts one authored derived formula node for node (T137 B4).
///
/// Data owns two preconditions, asserted here: an `Int` formula holds only
/// `Int` literals, and it names only `Int` stats of the ruleset. A violation
/// panics with the data read-time text.
fn convert_formula(expr: &ExprWire, int_names: &BTreeSet<&str>) -> IntFormula {
    let pair = |left: &ExprWire, right: &ExprWire| {
        (
            Box::new(convert_formula(left, int_names)),
            Box::new(convert_formula(right, int_names)),
        )
    };
    match expr {
        ExprWire::Literal(StatValueWire::Int(value)) => IntFormula::Literal(*value),
        ExprWire::Literal(_) => panic!("{DERIVED_PRECONDITION}"),
        ExprWire::Stat(name) => {
            if !int_names.contains(name.as_str()) {
                panic!("{DERIVED_PRECONDITION}");
            }
            IntFormula::Stat(name.clone())
        }
        ExprWire::Add { left, right } => {
            let (left, right) = pair(left, right);
            IntFormula::Add { left, right }
        }
        ExprWire::Subtract { left, right } => {
            let (left, right) = pair(left, right);
            IntFormula::Subtract { left, right }
        }
        ExprWire::Multiply { left, right } => {
            let (left, right) = pair(left, right);
            IntFormula::Multiply { left, right }
        }
        ExprWire::Divide { left, right } => {
            let (left, right) = pair(left, right);
            IntFormula::Divide { left, right }
        }
        ExprWire::Min { left, right } => {
            let (left, right) = pair(left, right);
            IntFormula::Min { left, right }
        }
        ExprWire::Max { left, right } => {
            let (left, right) = pair(left, right);
            IntFormula::Max { left, right }
        }
    }
}

/// Builds the kernel expression of one symbolic formula; every stat handle
/// comes from `scratch` (T137 D6).
fn formula_expr(formula: &IntFormula, scratch: &mut Interners) -> Expr {
    fn binary(
        left: &IntFormula,
        right: &IntFormula,
        scratch: &mut Interners,
    ) -> (Box<Expr>, Box<Expr>) {
        let left = formula_expr(left, scratch);
        let right = formula_expr(right, scratch);
        (Box::new(left), Box::new(right))
    }
    match formula {
        IntFormula::Literal(value) => Expr::Literal(StatValue::Int(*value)),
        IntFormula::Stat(name) => Expr::Stat(scratch.intern_stat(name)),
        IntFormula::Add { left, right } => {
            let (left, right) = binary(left, right, scratch);
            Expr::Add(left, right)
        }
        IntFormula::Subtract { left, right } => {
            let (left, right) = binary(left, right, scratch);
            Expr::Subtract(left, right)
        }
        IntFormula::Multiply { left, right } => {
            let (left, right) = binary(left, right, scratch);
            Expr::Multiply(left, right)
        }
        IntFormula::Divide { left, right } => {
            let (left, right) = binary(left, right, scratch);
            Expr::Divide(left, right)
        }
        IntFormula::Min { left, right } => {
            let (left, right) = binary(left, right, scratch);
            Expr::Min(left, right)
        }
        IntFormula::Max { left, right } => {
            let (left, right) = binary(left, right, scratch);
            Expr::Max(left, right)
        }
    }
}

/// The `Int` stat definitions of one operation (T137 B6): health and each
/// attribute as plain `Int`, then each derived stat with its formula. The
/// scratch interner issues handles in exactly that order.
pub(crate) fn int_definitions(
    health_stat: &str,
    attribute_names: &[String],
    derived: &[DerivedStat],
    scratch: &mut Interners,
) -> Vec<StatDefinition> {
    let mut definitions = Vec::with_capacity(1 + attribute_names.len() + derived.len());
    let health = scratch.intern_stat(health_stat);
    definitions.push(StatDefinition {
        id: health,
        kind: StatKind::Int,
        derived: None,
    });
    for name in attribute_names {
        let id = scratch.intern_stat(name);
        definitions.push(StatDefinition {
            id,
            kind: StatKind::Int,
            derived: None,
        });
    }
    let ids: Vec<_> = derived
        .iter()
        .map(|stat| scratch.intern_stat(&stat.name))
        .collect();
    for (stat, id) in derived.iter().zip(ids) {
        definitions.push(StatDefinition {
            id,
            kind: StatKind::Int,
            derived: Some(formula_expr(&stat.formula, scratch)),
        });
    }
    definitions
}

/// One combatant's base values (T137 B6): `health` holds the current health
/// and every attribute its stored value.
fn base_block(
    health_stat: &str,
    health: u32,
    attributes: &[(String, i32)],
    scratch: &mut Interners,
) -> StatBlock {
    let mut block = StatBlock::new();
    let id = scratch.intern_stat(health_stat);
    block
        .insert(
            id,
            StatValue::Int(i32::try_from(health).unwrap_or(i32::MAX)),
        )
        .expect("combat stat counts are bounded");
    for (name, value) in attributes {
        let id = scratch.intern_stat(name);
        block
            .insert(id, StatValue::Int(*value))
            .expect("combat stat counts are bounded");
    }
    block
}

/// The fully modified `Int` value of one named stat, or `None` when the
/// kernel query fails (T137 D5).
fn query_int(
    pipeline: &ModifierPipeline,
    entity: EntityId,
    name: &str,
    scratch: &Interners,
    context: &QueryContext<'_>,
) -> Option<i32> {
    let stat = scratch.stat(name)?;
    match pipeline.query(entity, stat, context) {
        Ok((StatValue::Int(value), _)) => Some(value),
        _ => None,
    }
}

/// Rebuilds one kernel modifier from one persisted entry; a `Stat` target's
/// handle comes from `scratch` (T137 B18).
fn rebuild_modifier(
    effect: Ulid,
    mod_type: &str,
    entry: &EffectModifier,
    tag: RollTag,
    scratch: &mut Interners,
) -> Modifier {
    let target = match &entry.target {
        EffectTarget::Roll => ModifierTarget::Roll(tag),
        EffectTarget::Dc => ModifierTarget::Dc(tag),
        EffectTarget::Stat { stat } => ModifierTarget::Stat(scratch.intern_stat(stat)),
    };
    let op = match entry.op {
        EffectOp::Add => ModOp::Add(StatValue::Int(entry.value)),
        EffectOp::Set => ModOp::Set(StatValue::Int(entry.value)),
    };
    Modifier {
        id: entry.id,
        source: SourceRef {
            kind: EFFECT_SOURCE_KIND.to_owned(),
            id: effect,
        },
        target,
        op,
        mod_type: ModTypeId(mod_type.to_owned()),
        name: entry.name.clone(),
        condition: None,
        priority: entry.priority,
    }
}

/// The numeric folds a view carries besides `Stat` modifiers (T137 B8).
#[derive(Debug, Clone, Copy)]
struct Folds {
    roll: bool,
    dc: bool,
}

/// Collects one combatant's unexpired modifiers of the wanted kinds, in
/// attachment order and then authored modifier order (T137 B8). `Stat`
/// modifiers always belong to the view.
fn view_modifiers(
    definition: &CombatDefinition,
    combatant: &Combatant,
    round: u32,
    tag: RollTag,
    scratch: &mut Interners,
    folds: Folds,
) -> Vec<Modifier> {
    let mut out = Vec::new();
    for attached in &combatant.attached {
        if attached.expires_round <= round {
            continue;
        }
        let Some(entry) = effect_entry(definition, attached.effect) else {
            continue;
        };
        for modifier in &entry.modifiers {
            let wanted = match modifier.target {
                EffectTarget::Roll => folds.roll,
                EffectTarget::Dc => folds.dc,
                EffectTarget::Stat { .. } => true,
            };
            if wanted {
                out.push(rebuild_modifier(
                    entry.effect,
                    &entry.mod_type,
                    modifier,
                    tag,
                    scratch,
                ));
            }
        }
    }
    out
}

/// The synthetic `Roll` modifiers of one view's bonus stats (T137 B14), one
/// per value in authored order. The k-th takes the k-th smallest `Ulid`
/// from `1` that no modifier already in `view` uses.
fn bonus_modifiers(
    ability: Ulid,
    values: &[i32],
    tag: RollTag,
    view: &[Modifier],
) -> Vec<Modifier> {
    let used: BTreeSet<Ulid> = view.iter().map(|modifier| modifier.id).collect();
    let mut next: u128 = 1;
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        while used.contains(&Ulid::from_u128(next)) {
            next += 1;
        }
        out.push(Modifier {
            id: Ulid::from_u128(next),
            source: SourceRef {
                kind: BONUS_SOURCE_KIND.to_owned(),
                id: ability,
            },
            target: ModifierTarget::Roll(tag),
            op: ModOp::Add(StatValue::Int(*value)),
            mod_type: ModTypeId(COMBAT_BONUS_MOD_TYPE.to_owned()),
            name: None,
            condition: None,
            priority: 0,
        });
        next += 1;
    }
    out
}

/// The pipeline, views and defence one action resolves with (T137 B15).
///
/// Built from a scratch interner, so no handle in it reaches the world.
/// Shared by the validation stage and by execution, which cannot disagree.
struct ActionPlan {
    pipeline: ModifierPipeline,
    roll_tag: RollTag,
    actor_stats: StatBlock,
    target_stats: StatBlock,
    /// The actor's modifiers; the one combined view when self-targeted.
    actor_mods: Vec<Modifier>,
    /// The target's modifiers; empty when self-targeted.
    target_mods: Vec<Modifier>,
    against: Against,
}

/// Builds one action's plan, querying in the B15 order: the defence-side
/// stat, then the actor's bonus stats. The first failed query is
/// `StatEvaluation` naming that stat. Reads the world, writes nothing.
fn action_plan(
    world: &World,
    mut scratch: Interners,
    entry: &AbilityDefinition,
    actor: EntityId,
    target: EntityId,
) -> Result<ActionPlan, CombatError> {
    let combat = world.combat().expect("validation checks the encounter");
    let definition = &combat.definition;
    let round = combat.round;
    let combatants = world.combatants();
    let actor_state = combatants
        .get(actor)
        .expect("validation checks the actor is a live combatant");
    let target_state = combatants
        .get(target)
        .expect("validation checks the target is a live combatant");
    let roll_tag = RollTag(
        world
            .interners()
            .tag(COMBAT_ROLL_TAG)
            .expect("the roll tag is interned at encounter start"),
    );
    let definitions = int_definitions(
        &definition.health_stat,
        &definition.attribute_names,
        &definition.derived,
        &mut scratch,
    );
    let mut policies: BTreeMap<ModTypeId, StackingPolicy> = BTreeMap::new();
    for effect in &definition.effects {
        policies.insert(ModTypeId(effect.mod_type.clone()), effect.policy);
    }
    if !entry.bonus_stats.is_empty() {
        policies.insert(
            ModTypeId(COMBAT_BONUS_MOD_TYPE.to_owned()),
            StackingPolicy::StackAll,
        );
    }
    let pipeline = ModifierPipeline::new(definitions, policies)
        .expect("combat preflight guarantees pipeline success");
    let actor_stats = base_block(
        &definition.health_stat,
        actor_state.health,
        &actor_state.attributes,
        &mut scratch,
    );
    let target_stats = base_block(
        &definition.health_stat,
        target_state.health,
        &target_state.attributes,
        &mut scratch,
    );
    let self_targeted = actor == target;
    let actor_folds = Folds {
        roll: true,
        dc: self_targeted || matches!(entry.defense, AbilityDefense::ActorStat { .. }),
    };
    let mut actor_mods = view_modifiers(
        definition,
        actor_state,
        round,
        roll_tag,
        &mut scratch,
        actor_folds,
    );
    let target_mods = if self_targeted {
        Vec::new()
    } else {
        view_modifiers(
            definition,
            target_state,
            round,
            roll_tag,
            &mut scratch,
            Folds {
                roll: false,
                dc: true,
            },
        )
    };
    let no_tags = TagSet::new();
    let fail = |stat: &str| CombatError::StatEvaluation {
        stat: stat.to_owned(),
    };
    let actor_context = QueryContext {
        entity: actor,
        stats: &actor_stats,
        tags: &no_tags,
        modifiers: &actor_mods,
    };
    let target_context = if self_targeted {
        QueryContext {
            entity: target,
            stats: &actor_stats,
            tags: &no_tags,
            modifiers: &actor_mods,
        }
    } else {
        QueryContext {
            entity: target,
            stats: &target_stats,
            tags: &no_tags,
            modifiers: &target_mods,
        }
    };
    let against = match &entry.defense {
        AbilityDefense::ActorAttribute => {
            let value = query_int(&pipeline, actor, &entry.attribute, &scratch, &actor_context)
                .ok_or_else(|| fail(&entry.attribute))?;
            Against::Dc {
                value,
                owner: target,
                modifier_target: roll_tag,
            }
        }
        AbilityDefense::TargetStat { stat } => {
            query_int(&pipeline, target, stat, &scratch, &target_context)
                .ok_or_else(|| fail(stat))?;
            Against::Stat {
                entity: target,
                stat: scratch
                    .stat(stat)
                    .expect("the defense stat is interned with the definitions"),
                modifier_target: roll_tag,
            }
        }
        AbilityDefense::Constant(value) => Against::Dc {
            value: *value,
            owner: target,
            modifier_target: roll_tag,
        },
        AbilityDefense::ActorStat { stat } => {
            query_int(&pipeline, actor, stat, &scratch, &actor_context)
                .ok_or_else(|| fail(stat))?;
            Against::Stat {
                entity: actor,
                stat: scratch
                    .stat(stat)
                    .expect("the defense stat is interned with the definitions"),
                modifier_target: roll_tag,
            }
        }
    };
    let mut bonus_values = Vec::with_capacity(entry.bonus_stats.len());
    for name in &entry.bonus_stats {
        let value = query_int(&pipeline, actor, name, &scratch, &actor_context)
            .ok_or_else(|| fail(name))?;
        bonus_values.push(value);
    }
    let synthetic = bonus_modifiers(entry.ability, &bonus_values, roll_tag, &actor_mods);
    actor_mods.extend(synthetic);
    Ok(ActionPlan {
        pipeline,
        roll_tag,
        actor_stats,
        target_stats,
        actor_mods,
        target_mods,
        against,
    })
}

/// Checks one action against the authoritative admission rules without
/// mutating anything (T028, ADR-0018).
///
/// This is the single pre-mutation validation path: [`perform_action`] runs
/// it first, and [`legal_actions`] filters candidates through it, so the two
/// can never disagree. The pinned precedence is unchanged. `UseAbility`:
/// `NoEncounter`; live nonparticipant actor; live nonparticipant target;
/// `AbsentActor`; `AbsentTarget`; `DeadActor`; `DeadTarget`; `OutOfTurn`;
/// `UnknownAbility`; `SelfTarget`; then per-pool affordability in template
/// order (checked sums, `ValueOverflow`/`InsufficientAction`, T019).
/// `EndTurn`: `NoEncounter`; live nonparticipant actor; `AbsentActor`;
/// `DeadActor`; `OutOfTurn`. Validation stops before interning, creating RNG
/// streams, resolving, spending, or advancing. `Ok` is not a lease: state can
/// change before execution, which always revalidates.
///
/// In a real-time encounter (T138, ADR-0029 T8) only the turn stage changes:
/// `UseAbility` and `Stop` meet the terminal gate (`OutOfTurn` with no
/// active, fewer than two alive) where turn-based play checks the head,
/// `EndTurn` fails `WrongMode`, and `Stop` in turn-based play fails
/// `WrongMode`. `Stop` checks `NoEncounter`; live nonparticipant actor;
/// `AbsentActor`; `DeadActor`; then the mode stage.
pub fn validate_action(world: &World, action: &CombatAction) -> Result<(), CombatError> {
    use CombatError::{
        AbsentActor, AbsentTarget, DeadActor, DeadTarget, InsufficientAction, NoEncounter,
        NotParticipant, OutOfTurn, SelfTarget, UnknownAbility, ValueOverflow, WrongMode,
    };

    let combat = world.combat().ok_or(NoEncounter)?;
    let combatants = world.combatants();
    match *action {
        CombatAction::Stop { actor } => {
            if world.contains(actor) && !combatants.contains(actor) {
                return Err(NotParticipant { entity: actor });
            }
            if !world.contains(actor) {
                return Err(AbsentActor { actor });
            }
            let actor_state = combatants
                .get(actor)
                .expect("the actor is a live combatant");
            if actor_state.dead {
                return Err(DeadActor { actor });
            }
            match combat.mode {
                EncounterMode::TurnBased => Err(WrongMode {
                    mode: EncounterMode::TurnBased,
                }),
                EncounterMode::RealTime => {
                    if rt_alive(world) < 2 {
                        return Err(OutOfTurn {
                            actor,
                            active: None,
                        });
                    }
                    Ok(())
                }
            }
        }
        CombatAction::EndTurn { actor } => {
            if world.contains(actor) && !combatants.contains(actor) {
                return Err(NotParticipant { entity: actor });
            }
            if !world.contains(actor) {
                return Err(AbsentActor { actor });
            }
            let actor_state = combatants
                .get(actor)
                .expect("the actor is a live combatant");
            if actor_state.dead {
                return Err(DeadActor { actor });
            }
            if combat.mode == EncounterMode::RealTime {
                return Err(WrongMode {
                    mode: EncounterMode::RealTime,
                });
            }
            if combat.active != Some(actor) {
                return Err(OutOfTurn {
                    actor,
                    active: combat.active,
                });
            }
            Ok(())
        }
        CombatAction::UseAbility {
            actor,
            ability,
            target,
        } => {
            let definition = &combat.definition;
            if world.contains(actor) && !combatants.contains(actor) {
                return Err(NotParticipant { entity: actor });
            }
            if world.contains(target) && !combatants.contains(target) {
                return Err(NotParticipant { entity: target });
            }
            if !world.contains(actor) {
                return Err(AbsentActor { actor });
            }
            if !world.contains(target) {
                return Err(AbsentTarget { target });
            }
            let actor_state = combatants
                .get(actor)
                .expect("the actor is a live combatant");
            let target_state = combatants
                .get(target)
                .expect("the target is a live combatant");
            if actor_state.dead {
                return Err(DeadActor { actor });
            }
            if target_state.dead {
                return Err(DeadTarget { target });
            }
            match combat.mode {
                EncounterMode::TurnBased => {
                    if combat.active != Some(actor) {
                        return Err(OutOfTurn {
                            actor,
                            active: combat.active,
                        });
                    }
                }
                EncounterMode::RealTime => {
                    if rt_alive(world) < 2 {
                        return Err(OutOfTurn {
                            actor,
                            active: None,
                        });
                    }
                }
            }
            let Some(entry) = ability_entry(definition, ability) else {
                return Err(UnknownAbility { ability });
            };
            if target == actor && !entry.allow_self_target {
                return Err(SelfTarget);
            }
            for template in &definition.pools {
                let mut total: u32 = 0;
                let mut present = false;
                for (pool, amount) in &entry.costs {
                    if pool.0 == template.id {
                        present = true;
                        total = total.checked_add(*amount).ok_or(ValueOverflow)?;
                    }
                }
                if !present {
                    continue;
                }
                let current = pool_balance(actor_state, template.id).unwrap_or(0);
                if current < total {
                    return Err(InsufficientAction {
                        pool: template.id,
                        cost: total,
                        current,
                    });
                }
            }
            let mut unknown: Vec<Ulid> = Vec::new();
            for (pool, _) in &entry.costs {
                if definition.pools.iter().any(|item| item.id == pool.0) {
                    continue;
                }
                if !unknown.contains(&pool.0) {
                    unknown.push(pool.0);
                }
            }
            for pool_id in unknown {
                let mut summed: u32 = 0;
                for (other, amount) in &entry.costs {
                    if other.0 == pool_id {
                        summed = summed.checked_add(*amount).ok_or(ValueOverflow)?;
                    }
                }
                let current = pool_balance(actor_state, pool_id).unwrap_or(0);
                if current < summed {
                    return Err(InsufficientAction {
                        pool: pool_id,
                        cost: summed,
                        current,
                    });
                }
            }
            // T137 B15: a derived formula can fail mid-fight (a zero
            // divisor), so the last stage runs the action's stat queries.
            if !definition.derived.is_empty() {
                action_plan(world, world.interners().clone(), entry, actor, target)?;
            }
            Ok(())
        }
    }
}

/// Why a [`legal_actions`] query produced no option list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegalActionsError {
    /// The actor cannot act at all: the `EndTurn { actor }` validation error
    /// (`Stop { actor }` in a real-time encounter).
    Actor(CombatError),
    /// The options would exceed [`MAX_LEGAL_ACTIONS`]; no partial list is
    /// returned.
    TooManyOptions {
        /// The exceeded output bound.
        limit: usize,
    },
}

impl std::fmt::Display for LegalActionsError {
    /// Renders the wrapped combat error unchanged, or `TooManyOptions at
    /// combat/options`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Actor(error) => std::fmt::Display::fmt(error, f),
            Self::TooManyOptions { .. } => write!(f, "TooManyOptions at combat/options"),
        }
    }
}

impl std::error::Error for LegalActionsError {
    /// Exposes the wrapped combat error for `Actor`; nothing otherwise.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Actor(error) => Some(error),
            Self::TooManyOptions { .. } => None,
        }
    }
}

/// Enumerates every action `actor` could currently execute, read-only
/// (T028, ADR-0018).
///
/// `EndTurn { actor }` is validated first; its failure is returned as
/// [`LegalActionsError::Actor`]. Otherwise every listed ability in ascending
/// ULID order is paired with every current combatant in ascending
/// [`EntityId`] order, and exactly the `UseAbility` candidates accepted by
/// [`validate_action`] are kept (a candidate's own failure, `ValueOverflow`
/// included, only excludes that candidate); `EndTurn` comes last. Every
/// `UseAbility` names a live combatant target, whatever the ability's
/// `requires_target` flag; `allow_self_target` alone governs self-targets.
///
/// In a real-time encounter (T138, ADR-0029 T8) `Stop { actor }` takes the
/// place of `EndTurn { actor }` at both ends: it is validated first and it
/// closes the list, and `EndTurn` is never listed.
///
/// The result is bounded by [`MAX_LEGAL_ACTIONS`], `EndTurn` included: the
/// query fails as a whole with `TooManyOptions` rather than truncating. The
/// bound limits output size, not running time. Options confer no
/// authorization and go stale when state changes; execution revalidates.
pub fn legal_actions(
    world: &World,
    actor: EntityId,
) -> Result<Vec<CombatAction>, LegalActionsError> {
    let real_time = world
        .combat()
        .is_some_and(|state| state.mode == EncounterMode::RealTime);
    let closing = if real_time {
        CombatAction::Stop { actor }
    } else {
        CombatAction::EndTurn { actor }
    };
    validate_action(world, &closing).map_err(LegalActionsError::Actor)?;
    let combat = world
        .combat()
        .expect("the closing action's validation checks the encounter");
    let mut abilities: Vec<Ulid> = combat
        .definition
        .abilities
        .iter()
        .map(|entry| entry.ability)
        .collect();
    abilities.sort_unstable();
    abilities.dedup();
    let mut targets: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
    targets.sort_unstable();

    let mut options: Vec<CombatAction> = Vec::new();
    let mut push = |option: CombatAction| {
        if options.len() == MAX_LEGAL_ACTIONS {
            return Err(LegalActionsError::TooManyOptions {
                limit: MAX_LEGAL_ACTIONS,
            });
        }
        options.push(option);
        Ok(())
    };
    for &ability in &abilities {
        for &target in &targets {
            let candidate = CombatAction::UseAbility {
                actor,
                ability,
                target,
            };
            if validate_action(world, &candidate).is_ok() {
                push(candidate)?;
            }
        }
    }
    push(closing)?;
    Ok(options)
}

/// Applies one validated action transactionally; rejected actions leave
/// `world` unchanged (resources, RNG, events, and event sequence included).
///
/// Validation runs in pinned precedence order before any mutation,
/// including before any RNG stream is created. An accepted attack then
/// runs one fixed order: kernel resolution over the world-owned
/// [`COMBAT_ROLL_STREAM`] with the interned [`COMBAT_ROLL_TAG`], cost
/// spending, saturating nonnegative damage with exactly-once death, effect
/// attachment on success, and turn advance (next head plus `TurnStart`
/// refresh, round rollover, or terminal; a non-ending turn keeps the active
/// head with no refresh). An accepted `EndTurn` advances with no spend, no
/// draw, no damage, and no effect. A valid failed attack consumes its
/// action; an invalid action consumes nothing.
///
/// In a real-time encounter (T138, ADR-0029 T8) an accepted `UseAbility`
/// only stores the actor's standing order and returns `Ok(None)`; `Stop`
/// clears it. Due orders resolve inside [`tick`](crate::tick).
pub fn perform_action(
    world: &mut World,
    action: &CombatAction,
) -> Result<Option<ActionOutcome>, CombatError> {
    perform_action_tracked(world, action).map(|(outcome, _)| outcome)
}

/// Applies one validated action transactionally, reporting the turn or
/// terminal transition the advance branches actually took.
///
/// Crate-visible only: [`perform_action`] delegates here and discards the
/// facts, so legacy emissions, bytes and signatures are unchanged; the
/// history wrapper consumes the facts to journal its trailing `TurnStarted`
/// or `EncounterEnded` (T020, ADR-0017).
pub(crate) fn perform_action_tracked(
    world: &mut World,
    action: &CombatAction,
) -> Result<(Option<ActionOutcome>, TransitionFacts), CombatError> {
    validate_action(world, action)?;
    match *action {
        CombatAction::Stop { actor } => {
            world
                .combatants_mut()
                .get_mut(actor)
                .expect("validation checks the actor is a live combatant")
                .order = None;
            Ok((None, TransitionFacts::default()))
        }
        CombatAction::EndTurn { actor } => {
            let dead: Vec<EntityId> = world
                .combatants()
                .iter()
                .filter(|(_, state)| state.dead)
                .map(|(id, _)| id)
                .collect();
            for id in dead {
                world.timeline_mut().remove(id);
            }
            let popped = end_turn(world);
            debug_assert_eq!(popped.map(|(_, id)| id), Some(actor));
            let facts = advance_tracked(world);
            Ok((None, facts))
        }
        CombatAction::UseAbility {
            actor,
            ability,
            target,
        } => {
            let combat = world.combat().expect("validation checks the encounter");
            if combat.mode == EncounterMode::RealTime {
                // ADR-0029 T8: an RT admission stores the order; it
                // resolves nothing, spends nothing and draws nothing.
                let repeat = ability_entry(&combat.definition, ability)
                    .expect("validation checks the ability is listed")
                    .repeats_in_real_time;
                world
                    .combatants_mut()
                    .get_mut(actor)
                    .expect("validation checks the actor is a live combatant")
                    .order = Some(StandingOrder {
                    ability,
                    target,
                    repeat,
                });
                return Ok((None, TransitionFacts::default()));
            }
            let outcome = resolve_use(world, actor, ability, target);
            let (encounter, round, ends_turn) = {
                let combat = world.combat().expect("validation checks the encounter");
                let entry = ability_entry(&combat.definition, ability)
                    .expect("validation checks the ability is listed");
                (combat.definition.encounter, combat.round, entry.ends_turn)
            };
            let dead: Vec<EntityId> = world
                .combatants()
                .iter()
                .filter(|(_, state)| state.dead)
                .map(|(id, _)| id)
                .collect();
            for id in dead {
                world.timeline_mut().remove(id);
            }
            let alive = world
                .combatants()
                .iter()
                .filter(|(_, state)| !state.dead)
                .count();
            let mut facts = TransitionFacts::default();
            let actor_dead = !world.timeline().contains(actor);
            if alive < 2 {
                if world.timeline().contains(actor) {
                    let popped = end_turn(world);
                    debug_assert_eq!(popped.map(|(_, id)| id), Some(actor));
                }
                world
                    .combat_mut()
                    .as_mut()
                    .expect("the encounter is checked above")
                    .active = None;
                facts.encounter_ended = Some((encounter, round));
            } else if actor_dead {
                facts = advance_tracked(world);
            } else if ends_turn {
                let popped = end_turn(world);
                debug_assert_eq!(popped.map(|(_, id)| id), Some(actor));
                facts = advance_tracked(world);
            } else {
                let head = world.timeline().iter().next().map(|(_, id)| id);
                debug_assert_eq!(head, Some(actor));
                world
                    .combat_mut()
                    .as_mut()
                    .expect("the encounter is checked above")
                    .active = Some(actor);
            }

            Ok((Some(outcome), facts))
        }
    }
}

/// The resolution half of an accepted `UseAbility`: kernel resolution on
/// `combat.roll`, cost spend, damage with exactly-once `Died`, and effect
/// attachment on success (T138, ADR-0029 T6, T7).
///
/// Extracted unchanged from [`perform_action_tracked`] so that turn-based
/// and real-time execution share one resolution, cost, damage and effect
/// pipeline and one draw order. The caller must have validated the action
/// and owns everything after it: scheduling, order bookkeeping and advance.
pub(crate) fn resolve_use(
    world: &mut World,
    actor: EntityId,
    ability: Ulid,
    target: EntityId,
) -> ActionOutcome {
    let combat = world.combat().expect("validation checks the encounter");
    let definition = combat.definition.clone();
    let round = combat.round;
    let entry = ability_entry(&definition, ability)
        .expect("validation checks the ability is listed")
        .clone();
    if let AbilityDefense::TargetStat { stat } = &entry.defense {
        world.interners_mut().intern_stat(stat);
    }
    let scratch = world.interners().clone();
    let plan = action_plan(world, scratch, &entry, actor, target)
        .expect("validation checks the stat evaluation stage");
    let ActionPlan {
        pipeline,
        roll_tag,
        actor_stats,
        target_stats,
        actor_mods,
        target_mods,
        against,
    } = plan;
    let request = ResolutionRequest {
        roll: RollRequest {
            actor,
            roll: RollSpec::Dice(entry.dice.clone()),
            bonus_target: roll_tag,
            natural_die: entry.natural_die,
            stream: COMBAT_ROLL_STREAM.to_owned(),
            tags: TagSet::new(),
        },
        target: Some(target),
        against,
        outcome_table: entry.outcome_table.id(),
    };
    let empty_tags = TagSet::new();
    let outcome = if actor == target {
        let view = QueryContext {
            entity: actor,
            stats: &actor_stats,
            tags: &empty_tags,
            modifiers: &actor_mods,
        };
        let mut entities = BTreeMap::new();
        entities.insert(actor, view);
        let context = ResolutionContext {
            pipeline: &pipeline,
            entities: &entities,
            outcome_table: &entry.outcome_table,
        };
        resolve(&request, &context, world.rng_mut())
            .expect("combat preflight guarantees resolution success")
    } else {
        let actor_view = QueryContext {
            entity: actor,
            stats: &actor_stats,
            tags: &empty_tags,
            modifiers: &actor_mods,
        };
        let target_view = QueryContext {
            entity: target,
            stats: &target_stats,
            tags: &empty_tags,
            modifiers: &target_mods,
        };
        let mut entities = BTreeMap::new();
        entities.insert(actor, actor_view);
        entities.insert(target, target_view);
        let context = ResolutionContext {
            pipeline: &pipeline,
            entities: &entities,
            outcome_table: &entry.outcome_table,
        };
        resolve(&request, &context, world.rng_mut())
            .expect("combat preflight guarantees resolution success")
    };
    let damage = entry
        .damage
        .iter()
        .find(|(outcome_kind, _)| *outcome_kind == outcome.decision.outcome)
        .map(|(_, amount)| *amount)
        .unwrap_or(0);

    spend_costs(world, actor, &entry.costs).expect("affordability is checked above");
    let target_died = if damage > 0 {
        let target_state = world
            .combatants_mut()
            .get_mut(target)
            .expect("the target is a live combatant");
        target_state.health = target_state.health.saturating_sub(damage);
        if target_state.health == 0 {
            target_state.dead = true;
            true
        } else {
            false
        }
    } else {
        false
    };
    if target_died {
        let tick = world.tick();
        world
            .events_mut()
            .push(tick, SimEvent::Died { entity: target });
    }
    let success = matches!(
        outcome.decision.outcome,
        Outcome::Success | Outcome::CriticalSuccess
    );
    if success {
        if let Some(effect) = entry.effect {
            attach_effect(world, &definition, effect, actor, target, round);
        }
    }

    ActionOutcome {
        actor,
        target,
        ability,
        roll: outcome
            .actor
            .roll
            .expect("combat always rolls its dice expression"),
        margin: outcome.margin,
        outcome: outcome.decision.outcome,
        damage,
        target_died,
    }
}

/// Spends every pool cost after an accepted resolution.
fn spend_costs(
    world: &mut World,
    actor: EntityId,
    costs: &[(ResourcePoolId, u32)],
) -> Result<(), ()> {
    let combatant = world
        .combatants_mut()
        .get_mut(actor)
        .expect("the actor is a live combatant");
    for (pool, amount) in costs {
        if combatant.action_pool.id() == *pool {
            combatant.action_pool.spend(*amount).map_err(|_| ())?;
        } else {
            let entry = combatant
                .extra_pools
                .iter_mut()
                .find(|candidate| candidate.id() == *pool)
                .expect("cost pools resolve above");
            entry.spend(*amount).map_err(|_| ())?;
        }
    }
    Ok(())
}

/// Attaches one effect on success, refreshing any existing entry in place.
fn attach_effect(
    world: &mut World,
    definition: &CombatDefinition,
    effect: Ulid,
    actor: EntityId,
    target: EntityId,
    round: u32,
) {
    let Some(entry) = effect_entry(definition, effect) else {
        return;
    };
    let holder = match entry.aim {
        EffectAim::Slf => actor,
        EffectAim::Target => target,
    };
    let expires_round = round.saturating_add(entry.duration_rounds);
    let Some(combatant) = world.combatants_mut().get_mut(holder) else {
        return;
    };
    if let Some(existing) = combatant
        .attached
        .iter_mut()
        .find(|item| item.effect == effect)
    {
        existing.expires_round = expires_round;
    } else {
        combatant.attached.push(AttachedEffect {
            effect,
            expires_round,
        });
        combatant.attached.sort_by_key(|item| item.effect);
    }
}

/// The turn-based round start: `round + 1` (saturating), `RoundStart`
/// refresh of every alive combatant, attachment expiry, every alive
/// combatant (re)inserted at its stored initiative, one `TurnStart` refresh
/// for the head, and `active = Some(head)`. Returns the head and the new
/// round. The caller guarantees two or more alive combatants.
///
/// This is the rollover body of [`advance_tracked`] moved without change
/// (T138); the RT to TB switch runs it too (ADR-0029 T10).
fn tb_round_start(world: &mut World) -> (EntityId, u32) {
    let round = world
        .combat()
        .expect("the encounter is checked above")
        .round
        .saturating_add(1);
    for (_, state) in world.combatants_mut().iter_mut() {
        if !state.dead {
            state.action_pool.refresh(RefreshEvent::RoundStart);
            for pool in state.extra_pools.iter_mut() {
                pool.refresh(RefreshEvent::RoundStart);
            }
        }
    }
    for (_, state) in world.combatants_mut().iter_mut() {
        state.attached.retain(|item| item.expires_round > round);
    }
    let entries: Vec<(InitiativeKey, EntityId)> = world
        .combatants()
        .iter()
        .filter(|(_, state)| !state.dead)
        .map(|(id, state)| (InitiativeKey(state.initiative), id))
        .collect();
    for (key, id) in entries {
        world.timeline_mut().insert(key, id);
    }
    let head = world
        .timeline()
        .iter()
        .next()
        .expect("live participants are re-added above")
        .1;
    {
        let combatant = world
            .combatants_mut()
            .get_mut(head)
            .expect("scheduled entities are combatants");
        combatant.action_pool.refresh(RefreshEvent::TurnStart);
        for pool in combatant.extra_pools.iter_mut() {
            pool.refresh(RefreshEvent::TurnStart);
        }
    }
    world
        .combat_mut()
        .as_mut()
        .expect("the encounter is checked above")
        .active = Some(head);
    world
        .combat_mut()
        .as_mut()
        .expect("the encounter is checked above")
        .round = round;
    (head, round)
}

/// Settles the turn after the scheduled head left the timeline, reporting
/// the transition the branches actually took.
///
/// Crate-visible only: the tracked combat entry points and the history
/// wrapper consume the facts (T020, ADR-0017). The rule is one, owned here:
/// timeline nonempty with two or more alive sets the new head with one
/// `TurnStart` refresh; timeline empty with two or more alive rolls the round
/// over (`round + 1`, `RoundStart` refresh of all alive, re-add alive at
/// their initiative keys, head with `TurnStart`, expired attachments
/// dropped); otherwise terminal (`active = `None`). The caller has already
/// removed the departed entity from the timeline. Legacy behavior —
/// emissions, bytes and signatures — is unchanged; only the branch facts are
/// additionally reported.
pub(crate) fn advance_tracked(world: &mut World) -> TransitionFacts {
    let mut facts = TransitionFacts::default();
    if world.timeline().is_empty() {
        let alive = world
            .combatants()
            .iter()
            .filter(|(_, state)| !state.dead)
            .count();
        if alive >= 2 {
            let advanced = world
                .combat()
                .expect("the encounter is checked above")
                .clock
                .map(|clock| clock.round_ticks);
            if let Some(round_ticks) = advanced {
                // ADR-0029 T4: a TB rollover of an RT-capable encounter
                // advances the clock by one round, bounded.
                let combat = world
                    .combat_mut()
                    .as_mut()
                    .expect("the encounter is checked above");
                let clock = combat
                    .clock
                    .as_mut()
                    .expect("the clock presence is checked above");
                let old = clock.elapsed;
                let new = old.saturating_add(u64::from(round_ticks)).min(RT_CLOCK_MAX);
                clock.elapsed = new;
                let pending = combat.pending_mode.take();
                deliver_clock_advance(world, old, new);
                if pending == Some(EncounterMode::RealTime) {
                    let round = switch_to_real_time(world);
                    facts.mode_changed = Some((EncounterMode::RealTime, round));
                    facts.round_started = Some(round);
                    return facts;
                }
            }
            let (head, round) = tb_round_start(world);
            facts.turn_started = Some((head, round));
        } else {
            let ended = {
                let state = world.combat().expect("the encounter is checked above");
                (state.definition.encounter, state.round)
            };
            world
                .combat_mut()
                .as_mut()
                .expect("the encounter is checked above")
                .active = None;
            facts.encounter_ended = Some(ended);
        }
    } else {
        let alive = world
            .combatants()
            .iter()
            .filter(|(_, state)| !state.dead)
            .count();
        if alive >= 2 {
            let head = world
                .timeline()
                .iter()
                .next()
                .expect("the timeline is checked nonempty above")
                .1;
            {
                let combatant = world
                    .combatants_mut()
                    .get_mut(head)
                    .expect("scheduled entities are combatants");
                combatant.action_pool.refresh(RefreshEvent::TurnStart);
                for pool in combatant.extra_pools.iter_mut() {
                    pool.refresh(RefreshEvent::TurnStart);
                }
            }
            world
                .combat_mut()
                .as_mut()
                .expect("the encounter is checked above")
                .active = Some(head);
            let round = world
                .combat()
                .expect("the encounter is checked above")
                .round;
            facts.turn_started = Some((head, round));
        } else {
            let ended = {
                let state = world.combat().expect("the encounter is checked above");
                (state.definition.encounter, state.round)
            };
            world
                .combat_mut()
                .as_mut()
                .expect("the encounter is checked above")
                .active = None;
            facts.encounter_ended = Some(ended);
        }
    }
    facts
}

/// The number of combatants with `dead == false`; the RT terminal rule is
/// "fewer than two" (T138 D15).
fn rt_alive(world: &World) -> usize {
    world
        .combatants()
        .iter()
        .filter(|(_, state)| !state.dead)
        .count()
}

/// The ids of every alive combatant in ascending [`EntityId`] order.
fn alive_ids(world: &World) -> Vec<EntityId> {
    let mut ids: Vec<EntityId> = world
        .combatants()
        .iter()
        .filter(|(_, state)| !state.dead)
        .map(|(id, _)| id)
        .collect();
    ids.sort_unstable();
    ids
}

/// Host-authorized mode request (ADR-0029 T9): sets, replaces or clears
/// `pending_mode`; never changes `mode` immediately.
///
/// Checks `NoEncounter`, then `ModeUnsupported` when real time is requested
/// of an encounter without a clock. Requesting the mode already in force
/// clears the pending request; any other mode replaces it. The switch itself
/// happens only at the next boundary (RT to TB) or rollover (TB to RT), and
/// never in a terminal encounter. Nothing else changes and nothing is
/// emitted.
pub fn request_mode(world: &mut World, mode: EncounterMode) -> Result<(), CombatError> {
    let state = world
        .combat_mut()
        .as_mut()
        .ok_or(CombatError::NoEncounter)?;
    if mode == EncounterMode::RealTime && state.clock.is_none() {
        return Err(CombatError::ModeUnsupported);
    }
    state.pending_mode = if mode == state.mode { None } else { Some(mode) };
    Ok(())
}

/// Delivers one clock advance `old` to `new` to every `OnTick(p)` pool of
/// every alive combatant (ascending [`EntityId`], primary pool first, then
/// extras in template order): exactly one `Tick` refresh when `new / p >
/// old / p`, carrying the last multiple of `p` the advance crossed because
/// the kernel matches only ticks divisible by the period (T138 D14).
fn deliver_clock_advance(world: &mut World, old: u64, new: u64) {
    for id in alive_ids(world) {
        let combatant = world
            .combatants_mut()
            .get_mut(id)
            .expect("alive ids are combatants");
        let pools =
            std::iter::once(&mut combatant.action_pool).chain(combatant.extra_pools.iter_mut());
        for pool in pools {
            let RefreshTrigger::OnTick(period) = pool.refresh_trigger() else {
                continue;
            };
            if period != 0 && new / period > old / period {
                pool.refresh(RefreshEvent::Tick(Tick::new(new - new % period)));
            }
        }
    }
}

/// The real-time round start (ADR-0029 T6): `round + 1` (saturating), then
/// per alive combatant in ascending [`EntityId`] order a `RoundStart` and a
/// `TurnStart` refresh of its pools (primary first), then attachments keep
/// only `expires_round > round`. `active` stays `None`. Returns the new
/// round.
fn rt_round_start(world: &mut World) -> u32 {
    let round = world
        .combat()
        .expect("the encounter is checked above")
        .round
        .saturating_add(1);
    for id in alive_ids(world) {
        let combatant = world
            .combatants_mut()
            .get_mut(id)
            .expect("alive ids are combatants");
        for event in [RefreshEvent::RoundStart, RefreshEvent::TurnStart] {
            combatant.action_pool.refresh(event);
            for pool in combatant.extra_pools.iter_mut() {
                pool.refresh(event);
            }
        }
    }
    for (_, state) in world.combatants_mut().iter_mut() {
        state.attached.retain(|item| item.expires_round > round);
    }
    world
        .combat_mut()
        .as_mut()
        .expect("the encounter is checked above")
        .round = round;
    round
}

/// Applies a pending TB to RT switch at a TB rollover (ADR-0029 T10): the
/// mode flips, every alive combatant is keyed at the clock, `active` clears,
/// and the RT round start runs (no TB refresh, no `TurnStart` fact). Returns
/// the round the switch started.
fn switch_to_real_time(world: &mut World) -> u32 {
    let elapsed = {
        let combat = world
            .combat_mut()
            .as_mut()
            .expect("the encounter is checked above");
        combat.mode = EncounterMode::RealTime;
        combat.active = None;
        combat.clock.expect("a switch needs a clock").elapsed
    };
    let key = i32::try_from(elapsed).expect("the clock is bounded by the largest key");
    for id in alive_ids(world) {
        world.timeline_mut().insert(InitiativeKey(key), id);
    }
    rt_round_start(world)
}

/// Applies a pending RT to TB switch at an RT boundary (ADR-0029 T10): every
/// order clears, the mode flips, and the TB round start runs. Returns the new
/// turn head and the round the switch started.
fn switch_to_turn_based(world: &mut World) -> (EntityId, u32) {
    let ids: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
    for id in ids {
        world
            .combatants_mut()
            .get_mut(id)
            .expect("listed ids are combatants")
            .order = None;
    }
    world
        .combat_mut()
        .as_mut()
        .expect("the encounter is checked above")
        .mode = EncounterMode::TurnBased;
    tb_round_start(world)
}

/// Whether stages 2 and 3 act on this world: an RT encounter with two or
/// more alive combatants and a clock below its bound (ADR-0029 T5, D4).
fn rt_running(world: &World) -> bool {
    let Some(combat) = world.combat() else {
        return false;
    };
    combat.mode == EncounterMode::RealTime
        && combat
            .clock
            .is_some_and(|clock| clock.elapsed < RT_CLOCK_MAX)
        && rt_alive(world) >= 2
}

/// Tick stage 2 for an RT encounter (ADR-0029 T3, T6, T9, T10).
///
/// Advances the clock by one, delivers it to `OnTick` pools, and at a
/// boundary applies a pending RT to TB switch or runs the RT round start.
/// The tick that reaches [`RT_CLOCK_MAX`] does nothing else (D4).
pub(crate) fn clock_stage(world: &mut World, facts: &mut TickFacts) {
    if !rt_running(world) {
        return;
    }
    let (old, new, round_ticks) = {
        let clock = world
            .combat_mut()
            .as_mut()
            .and_then(|combat| combat.clock.as_mut())
            .expect("a running RT encounter has a clock");
        let old = clock.elapsed;
        clock.elapsed = old + 1;
        (old, clock.elapsed, clock.round_ticks)
    };
    if new == RT_CLOCK_MAX {
        return;
    }
    deliver_clock_advance(world, old, new);
    if new % u64::from(round_ticks) != 0 {
        return;
    }
    let pending = world
        .combat_mut()
        .as_mut()
        .expect("the encounter is checked above")
        .pending_mode
        .take();
    if pending == Some(EncounterMode::TurnBased) {
        let (head, round) = switch_to_turn_based(world);
        facts.transitions.mode_changed = Some((EncounterMode::TurnBased, round));
        facts.transitions.turn_started = Some((head, round));
    } else {
        facts.transitions.round_started = Some(rt_round_start(world));
    }
}

/// Tick stage 3 for an RT encounter (ADR-0029 T7).
///
/// Snapshots every entry whose key is at or below the clock in ascending
/// `(key, EntityId)` order before acting, then executes each stored order
/// through [`validate_action`] and [`resolve_use`]. A due order that fails
/// validation is cleared with no other effect. An actor that survives is
/// re-keyed at `min(elapsed + recovery, RT_CLOCK_MAX)`; a one-shot order
/// clears. A combatant without an order keeps its key and waits.
pub(crate) fn execution_stage(world: &mut World, facts: &mut TickFacts) {
    if !rt_running(world) {
        return;
    }
    let (elapsed, round_ticks) = {
        let clock = world
            .combat()
            .and_then(|combat| combat.clock)
            .expect("a running RT encounter has a clock");
        (clock.elapsed, clock.round_ticks)
    };
    let due: Vec<EntityId> = world
        .timeline()
        .iter()
        .filter(|(key, _)| u64::try_from(key.0).map_or(true, |key| key <= elapsed))
        .map(|(_, id)| id)
        .collect();
    for actor in due {
        if rt_alive(world) < 2 {
            break;
        }
        let Some(state) = world.combatants().get(actor) else {
            continue;
        };
        if state.dead || !world.timeline().contains(actor) {
            continue;
        }
        let Some(order) = state.order else {
            continue;
        };
        let action = CombatAction::UseAbility {
            actor,
            ability: order.ability,
            target: order.target,
        };
        if validate_action(world, &action).is_err() {
            world
                .combatants_mut()
                .get_mut(actor)
                .expect("the actor is a live combatant")
                .order = None;
            facts.events.push(TickEvent::OrderEnded { actor });
            continue;
        }
        let recovery = {
            let combat = world.combat().expect("the encounter is checked above");
            ability_entry(&combat.definition, order.ability)
                .expect("validation checks the ability is listed")
                .recovery_ticks
                .unwrap_or(round_ticks)
        };
        let resolved = resolve_use(world, actor, order.ability, order.target);
        facts.events.push(TickEvent::Resolved(resolved));
        let dead: Vec<EntityId> = world
            .combatants()
            .iter()
            .filter(|(_, state)| state.dead)
            .map(|(id, _)| id)
            .collect();
        for id in dead {
            world.timeline_mut().remove(id);
            world
                .combatants_mut()
                .get_mut(id)
                .expect("listed ids are combatants")
                .order = None;
        }
        let actor_alive = world
            .combatants()
            .get(actor)
            .is_some_and(|state| !state.dead);
        if actor_alive {
            let key = elapsed
                .saturating_add(u64::from(recovery))
                .min(RT_CLOCK_MAX);
            let key = i32::try_from(key).expect("the key is bounded by the largest key");
            world.timeline_mut().insert(InitiativeKey(key), actor);
            if !order.repeat {
                world
                    .combatants_mut()
                    .get_mut(actor)
                    .expect("the actor is a live combatant")
                    .order = None;
            }
        }
    }
    if rt_alive(world) < 2 {
        // The pass made the encounter terminal: the first, and only, end.
        let state = world.combat().expect("the encounter is checked above");
        facts.transitions.encounter_ended = Some((state.definition.encounter, state.round));
    }
}

/// Releases the active encounter, retaining its summary for authoritative
/// consumers.
///
/// `combat = None` fails as `NoEncounter` without mutation. Otherwise the
/// summary is built first and the operation cannot partially fail: combat
/// scheduling and components are stripped, every entity stays live, and no
/// event is emitted (deaths were already emitted exactly once at their
/// kills, so repeated cleanup emits nothing). Release is allowed any time
/// the encounter is active; aborting an in-progress fight is caller intent
/// and the summary reflects the moment of release.
pub fn end_encounter(world: &mut World) -> Result<EncounterSummary, CombatError> {
    let state = world.combat().ok_or(CombatError::NoEncounter)?;
    let summary = EncounterSummary {
        encounter: state.definition.encounter,
        ruleset: state.definition.ruleset,
        round: state.round,
        results: world
            .combatants()
            .iter()
            .map(|(id, combatant)| ParticipantResult {
                placement: combatant.placement(),
                entity: id,
                health: combatant.health(),
                dead: combatant.dead(),
            })
            .collect(),
    };
    let ids: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
    for id in ids {
        world.timeline_mut().remove(id);
        world.combatants_mut().remove(id);
    }
    *world.combat_mut() = None;
    Ok(summary)
}

// The fixtures are shared with `src/history/tests.rs` and the integration
// tests by path, so the duplicate-module lint is expected here.
#[cfg(test)]
#[path = "../tests/support/mod.rs"]
#[allow(dead_code, clippy::duplicate_mod)]
mod support;

/// R12 (B9, B27, T138a): refresh bookkeeping across modes, with partial-refill
/// pools so a double or missed delivery shows as a wrong count.
#[cfg(test)]
mod rt_refresh_tests {
    use std::num::NonZeroU32;

    use super::support::rt::capable;
    use super::*;

    fn refill_one(id: Ulid, max: u32, trigger: RefreshTrigger) -> ResourcePool {
        ResourcePool::with_refill(ResourcePoolId(id), max, 0, trigger, NonZeroU32::new(1))
            .expect("a valid partial-refill pool")
    }

    /// Replaces every combatant's pools by empty partial-refill pools with
    /// the definition's identities and triggers.
    fn make_partial(world: &mut World, max: u32) {
        let templates = world.combat().expect("encounter").definition.pools.clone();
        let ids: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
        for id in ids {
            let state = world.combatants_mut().get_mut(id).expect("combatant");
            state.action_pool = refill_one(templates[0].id, max, templates[0].refresh);
            for (slot, template) in state.extra_pools.iter_mut().zip(templates.iter().skip(1)) {
                *slot = refill_one(template.id, max, template.refresh);
            }
        }
    }

    fn zero_all(world: &mut World) {
        let ids: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
        for id in ids {
            let state = world.combatants_mut().get_mut(id).expect("combatant");
            let current = state.action_pool.current();
            if current > 0 {
                state.action_pool.spend(current).expect("spend all");
            }
            for pool in state.extra_pools.iter_mut() {
                let current = pool.current();
                if current > 0 {
                    pool.spend(current).expect("spend all");
                }
            }
        }
    }

    fn balances(world: &World) -> Vec<(u32, u32)> {
        world
            .combatants()
            .iter()
            .map(|(_, state)| (state.action_pool.current(), state.extra_pools[0].current()))
            .collect()
    }

    #[test]
    fn clock_bound_is_the_largest_initiative_key() {
        assert_eq!(RT_CLOCK_MAX, i32::MAX as u64);
    }

    /// Eight rounds with the mode toggled in an irregular pattern: each alive
    /// combatant's `OnRoundStart` pool rises by exactly one per round, and its
    /// `OnTurnStart` pool follows the mode's turn-start rule.
    #[test]
    fn round_and_turn_start_pools_refresh_exactly_once() {
        use crpg_data::{ActionPoolTemplate, RefreshWire};
        let mut fixture = capable(4);
        fixture.ruleset.pools = vec![
            ActionPoolTemplate {
                id: Ulid::from_u128(105),
                max: 5,
                refresh: RefreshWire::OnRoundStart,
            },
            ActionPoolTemplate {
                id: Ulid::from_u128(106),
                max: 5,
                refresh: RefreshWire::OnTurnStart,
            },
        ];
        let (mut world, _) = fixture.start(1);
        make_partial(&mut world, 5);
        let pattern = [
            EncounterMode::RealTime,
            EncounterMode::TurnBased,
            EncounterMode::TurnBased,
            EncounterMode::RealTime,
            EncounterMode::RealTime,
            EncounterMode::RealTime,
            EncounterMode::TurnBased,
            EncounterMode::RealTime,
        ];
        for (index, wanted) in pattern.into_iter().enumerate() {
            let current = world.combat().expect("encounter").mode;
            request_mode(&mut world, wanted).expect("capable");
            let round = world.combat().expect("encounter").round;
            zero_all(&mut world);
            match current {
                EncounterMode::TurnBased => loop {
                    let state = world.combat().expect("encounter");
                    let actor = state.active.expect("a TB head");
                    if world.timeline().len() == 1 {
                        // The last turn: its end is the rollover.
                        zero_all(&mut world);
                    }
                    perform_action(&mut world, &CombatAction::EndTurn { actor }).expect("end");
                    if world.combat().expect("encounter").round != round {
                        break;
                    }
                },
                EncounterMode::RealTime => {
                    for _ in 0..4 {
                        crate::tick::tick(&mut world);
                    }
                }
            }
            let combat = world.combat().expect("encounter");
            assert_eq!(combat.round, round + 1, "boundary {index}");
            assert_eq!(combat.mode, wanted, "boundary {index}");
            let found = balances(&world);
            // The OnRoundStart pool: exactly +1 for everyone.
            assert!(
                found.iter().all(|(round_pool, _)| *round_pool == 1),
                "{index}: {found:?}"
            );
            // The OnTurnStart pool: everyone in RT, only the head in TB.
            let turn: Vec<u32> = found.iter().map(|(_, turn)| *turn).collect();
            match wanted {
                EncounterMode::RealTime => assert_eq!(turn, vec![1, 1], "boundary {index}"),
                EncounterMode::TurnBased => {
                    let head = combat.active.expect("head");
                    let expected: Vec<u32> = world
                        .combatants()
                        .iter()
                        .map(|(id, _)| u32::from(id == head))
                        .collect();
                    assert_eq!(turn, expected, "boundary {index}");
                }
            }
        }
    }

    /// `OnTick` counts: one delivery per advance that crosses a multiple, in
    /// RT ticks and in TB rollover jumps.
    #[test]
    fn on_tick_deliveries_are_counted_exactly() {
        use crpg_data::{ActionPoolTemplate, RefreshWire, TickPeriod};
        let tick_pool = |id: u128, period: u64| ActionPoolTemplate {
            id: Ulid::from_u128(id),
            max: 9,
            refresh: RefreshWire::OnTick(TickPeriod(period)),
        };
        let build = |periods: [u64; 3]| {
            let mut fixture = capable(4);
            fixture.ruleset.pools = vec![
                tick_pool(105, periods[0]),
                tick_pool(106, periods[1]),
                tick_pool(107, periods[2]),
            ];
            fixture
        };
        let current = |world: &World| -> Vec<Vec<u32>> {
            world
                .combatants()
                .iter()
                .map(|(_, state)| {
                    let mut all = vec![state.action_pool.current()];
                    all.extend(state.extra_pools.iter().map(|pool| pool.current()));
                    all
                })
                .collect()
        };

        // RT: periods 1, 3 and 4 over eight ticks.
        let fixture = build([1, 3, 4]);
        let (mut world, _) = fixture.start_rt(1, EncounterMode::RealTime);
        make_partial(&mut world, 9);
        for t in 1..=8_u32 {
            crate::tick::tick(&mut world);
            let expected = vec![t, t / 3, t / 4];
            for (index, balance) in current(&world).into_iter().enumerate() {
                assert_eq!(balance, expected, "combatant {index}, tick {t}");
            }
        }

        // TB: round_ticks 4 jumps; periods 1, 3 and 5 gain one per crossing
        // advance, however many multiples a jump passes (0..4, 4..8, 8..12
        // and 12..16 cross 5 at none, one, one and one of them).
        let fixture = build([1, 3, 5]);
        let (mut world, ids) = fixture.start(1);
        make_partial(&mut world, 9);
        for rollover in 1..=4_u32 {
            for actor in [ids[1], ids[0]] {
                perform_action(&mut world, &CombatAction::EndTurn { actor }).expect("end");
            }
            let expected = vec![rollover, rollover, [0, 1, 2, 3][rollover as usize - 1]];
            for (index, balance) in current(&world).into_iter().enumerate() {
                assert_eq!(balance, expected, "combatant {index}, rollover {rollover}");
            }
        }
    }
}

/// R7 (B22) and R5 (B14): the legacy-shape predicates and the synthetic
/// bonus identities, crate-private.
#[cfg(test)]
mod surface_tests {
    use super::support::Fixture;
    use super::*;

    /// A legacy-shaped definition from the standard fixture.
    fn legacy() -> CombatDefinition {
        let (world, _) = Fixture::standard().start(1);
        world.combat().expect("encounter").definition.clone()
    }

    fn dice(text: &str) -> DiceExpr {
        text.parse().expect("test dice parse")
    }

    /// Every ability-level departure from the legacy shape is non-legacy.
    #[test]
    fn ability_is_legacy_follows_its_table() {
        let base = legacy();
        let primary = base.pools[0].id;
        let entry = base.abilities[0].clone();
        assert!(ability_is_legacy(&entry, primary), "the baseline");
        type Edit = fn(&mut AbilityDefinition);
        let table: [(&str, Edit); 12] = [
            ("target stat", |e| {
                e.defense = AbilityDefense::TargetStat { stat: "x".into() }
            }),
            ("constant", |e| e.defense = AbilityDefense::Constant(1)),
            ("actor stat", |e| {
                e.defense = AbilityDefense::ActorStat { stat: "x".into() }
            }),
            ("ends turn", |e| e.ends_turn = false),
            ("effect", |e| e.effect = Some(Ulid::from_u128(1))),
            ("natural die", |e| e.natural_die = Some(0)),
            ("two costs", |e| {
                let first = e.costs[0];
                e.costs.push(first)
            }),
            ("other pool", |e| {
                e.costs[0].0 = ResourcePoolId(Ulid::from_u128(7))
            }),
            ("recovery", |e| e.recovery_ticks = Some(3)),
            ("repeats", |e| e.repeats_in_real_time = true),
            ("bonus stats", |e| e.bonus_stats.push("x".into())),
            ("damage dice", |e| {
                e.damage_dice.push((Outcome::Success, dice("1d4")))
            }),
        ];
        for (name, edit) in table {
            let mut changed = entry.clone();
            edit(&mut changed);
            assert!(!ability_is_legacy(&changed, primary), "{name}");
        }
    }

    /// Every definition-level departure from the legacy shape is non-legacy.
    #[test]
    fn definition_is_legacy_follows_its_table() {
        let base = legacy();
        assert!(definition_is_legacy(&base), "the baseline");
        type Edit = fn(&mut CombatDefinition);
        let table: [(&str, Edit); 7] = [
            ("derived", |d| {
                d.derived.push(DerivedStat {
                    name: "x".into(),
                    formula: IntFormula::Literal(1),
                })
            }),
            ("initiative", |d| {
                d.initiative = Some(InitiativeRule {
                    dice: dice("1d20"),
                    bonus_stats: Vec::new(),
                })
            }),
            ("second ability", |d| {
                let extra = d.abilities[0].clone();
                d.abilities.push(extra)
            }),
            ("second pool", |d| {
                let extra = d.pools[0];
                d.pools.push(extra)
            }),
            ("effects", |d| {
                d.effects.push(EffectDefinition {
                    effect: Ulid::from_u128(1),
                    aim: EffectAim::Slf,
                    mod_type: "x".into(),
                    policy: StackingPolicy::StackAll,
                    modifiers: Vec::new(),
                    duration_rounds: 1,
                })
            }),
            ("bonus stats", |d| {
                d.abilities[0].bonus_stats.push("x".into())
            }),
            ("damage dice", |d| {
                d.abilities[0]
                    .damage_dice
                    .push((Outcome::Success, dice("1d4")))
            }),
        ];
        for (name, edit) in table {
            let mut changed = base.clone();
            edit(&mut changed);
            assert!(!definition_is_legacy(&changed), "{name}");
        }
    }

    fn view_of(ids: &[u128]) -> Vec<Modifier> {
        let tag = RollTag(Interners::new().intern_tag("probe"));
        ids.iter()
            .map(|id| Modifier {
                id: Ulid::from_u128(*id),
                source: SourceRef {
                    kind: "probe".to_owned(),
                    id: Ulid::from_u128(0),
                },
                target: ModifierTarget::Roll(tag),
                op: ModOp::Add(StatValue::Int(1)),
                mod_type: ModTypeId("probe".to_owned()),
                name: None,
                condition: None,
                priority: 0,
            })
            .collect()
    }

    /// B14: the k-th synthetic modifier takes the k-th smallest unused id
    /// from one.
    #[test]
    fn synthetic_bonus_ids_skip_the_ids_a_view_uses() {
        let tag = RollTag(Interners::new().intern_tag("probe"));
        let ability = Ulid::from_u128(103);
        let ids = |view: &[u128], count: usize| -> Vec<u128> {
            bonus_modifiers(ability, &vec![5; count], tag, &view_of(view))
                .iter()
                .map(|modifier| modifier.id.to_u128())
                .collect()
        };
        assert_eq!(ids(&[], 2), [1, 2]);
        assert_eq!(
            ids(&[1], 2),
            [2, 3],
            "an authored ...01 pushes the first to ...02"
        );
        assert_eq!(ids(&[1, 3], 3), [2, 4, 5]);
        assert_eq!(ids(&[2, 9], 1), [1]);
        let first = &bonus_modifiers(ability, &[4], tag, &[])[0];
        assert_eq!(first.mod_type.0, COMBAT_BONUS_MOD_TYPE);
        assert_eq!(first.source.kind, BONUS_SOURCE_KIND);
        assert_eq!(first.source.id, ability);
        assert_eq!(first.op, ModOp::Add(StatValue::Int(4)));
        assert_eq!(
            (first.name.clone(), first.condition.clone(), first.priority),
            (None, None, 0)
        );
        assert_eq!(first.target, ModifierTarget::Roll(tag));
    }
}
