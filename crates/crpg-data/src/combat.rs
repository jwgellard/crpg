//! Generic authored combat vocabulary for data-driven encounters.
//!
//! Five document families (`Ruleset`, `Ability`, `OutcomeTable`,
//! `Encounter`, `Effect`) carry the ruleset-neutral combat data a later sim
//! adapter consumes: stat declarations with a health designation, dice-backed
//! abilities with per-pool costs, turn policy, defense selection, natural
//! selection and optional effects, margin-band outcome tables,
//! placement/initiative participant lists, and lifetime-bearing modifier
//! effects with effect-scoped stacking policy. Schemas stay generic;
//! minimal-d6 and srd-lite policy (attribute counts, band counts, pool
//! counts, ability counts) is content, never a schema or validation count.
//!
//! [`OutcomeWire`], [`RefreshWire`], [`NaturalEffectWire`],
//! [`DefenseWire`], [`EffectAimWire`], [`EffectTargetWire`],
//! [`EffectOpWire`], and [`PolicyWire`] mirror the T015 adjacent
//! `type`/`value` snake-case shapes as a copied convention; this crate never
//! depends on `crpg-rules`. Deserialization is hand-written to enforce exact
//! key sets per variant because serde's derived `deny_unknown_fields`
//! ignores trailing fields on unit variants. `Serialize`/`JsonSchema` stay
//! derived.

use crate::DataError;
use crpg_core::Ulid;
use schemars::JsonSchema;
use semver::Version;
use serde::{de, Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Dice notation input length in UTF-8 bytes, mirroring the rules kernel.
pub const MAX_DICE_INPUT_BYTES: usize = 128;
/// Margin bands in one authored outcome table, mirroring the rules kernel.
pub const MAX_OUTCOME_BANDS: usize = 256;
/// Natural-face rules in one authored outcome table, mirroring the kernel.
pub const MAX_NATURAL_RULES: usize = 256;
/// Sides on one die, mirroring the rules kernel.
pub const MAX_DIE_SIDES: u32 = 1_000_000;
/// Modifiers in one authored effect, mirroring the T014 kernel bound.
pub const MAX_EFFECT_MODIFIERS: usize = 4096;

/// Declared value category of one ruleset stat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatKindWire {
    /// Whole-number integer stat.
    Int,
}

/// One ruleset stat declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatDecl {
    /// Symbolic stat name referenced by abilities and creatures.
    pub name: String,
    /// Declared value category.
    pub kind: StatKindWire,
}

/// When an action pool refills to maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RefreshWire {
    /// Refills on every turn start.
    OnTurnStart,
    /// Refills on every round start.
    OnRoundStart,
    /// Refills on the matching rest only.
    OnRest(RestId),
    /// Refills on positive ticks divisible by the period.
    OnTick(TickPeriod),
    /// Never refills.
    Never,
}

/// Opaque rest identity for authored refresh triggers.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct RestId(#[schemars(with = "crate::schema::UlidText")] pub Ulid);

/// Nonzero tick period for authored refresh triggers.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct TickPeriod(pub u64);

/// One action-pool template owned by a ruleset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionPoolTemplate {
    /// Shared template identity for per-combatant runtime pools.
    #[schemars(with = "crate::schema::UlidText")]
    pub id: Ulid,
    /// Maximum balance; sim initializes current to this value.
    pub max: u32,
    /// Authored refresh trigger.
    pub refresh: RefreshWire,
}

/// How an ability selects its defense value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum DefenseWire {
    /// Uses the actor's checked attribute value.
    ActorAttribute,
    /// Uses the target's named stat value, never the health stat.
    TargetStat {
        /// Declared non-health stat name read from the target.
        stat: String,
    },
}

/// One non-primary pool cost entry on an ability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AbilityCost {
    /// Pool-template identity drawn from the owning ruleset.
    #[schemars(with = "crate::schema::UlidText")]
    pub pool: Ulid,
    /// Amount spent from that pool, always at least one.
    pub amount: u32,
}

/// Who an effect's modifiers apply to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum EffectAimWire {
    /// Modifiers apply to the actor that received the effect.
    Slf,
    /// Modifiers apply to the effect target.
    Target,
}

/// Which numeric fold an effect modifier contributes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum EffectTargetWire {
    /// Contributes to the Roll fold.
    Roll,
    /// Contributes to the Dc fold.
    Dc,
}

/// How an effect modifier combines with its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum EffectOpWire {
    /// Adds its value to the running total.
    Add,
    /// Replaces the running total with its value.
    Set,
}

/// Effect-scoped stacking policy governing all modifiers in one effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PolicyWire {
    /// Keeps every candidate modifier.
    StackAll,
    /// Keeps the largest strictly positive and smallest strictly negative Adds.
    HighestBonusWorstPenalty,
    /// Keeps the greatest `(priority, id)` per exact name.
    HighestPriorityPerName,
}

/// One modifier entry inside an authored effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectModifierWire {
    /// Stable modifier identity doubling as kernel ordering tiebreak input.
    #[schemars(with = "crate::schema::UlidText")]
    pub id: Ulid,
    /// Which numeric fold this modifier contributes to.
    pub target: EffectTargetWire,
    /// How this modifier combines with its base.
    pub op: EffectOpWire,
    /// Signed contribution value.
    pub value: i32,
    /// Ordering priority; higher runs later.
    pub priority: i16,
    /// Optional name used by per-name stacking policies; explicit null when absent.
    #[serde(deserialize_with = "crate::types::required_nullable")]
    #[schemars(required, schema_with = "crate::schema::nullable_text")]
    pub name: Option<String>,
}

/// Authored effect: lifetime-bearing modifiers with one shared policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    /// Stable authored-object identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub id: Ulid,
    /// Renameable human-facing slug.
    pub slug: String,
    /// Locale key for the display name.
    pub name: String,
    /// Preserved author annotation.
    #[serde(rename = "_note", default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Who the modifiers apply to.
    pub aim: EffectAimWire,
    /// Shared modifier-type name governing all modifiers in this effect.
    pub mod_type: String,
    /// Stacking policy applied to every modifier in this effect.
    pub policy: PolicyWire,
    /// Modifiers in authored order; every entry shares `mod_type`.
    pub modifiers: Vec<EffectModifierWire>,
    /// Lifetime in rounds, always at least one.
    pub duration_rounds: u32,
}

/// Authored ruleset: stat declarations, health designation, pools, abilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ruleset {
    /// Stable authored-object identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub id: Ulid,
    /// Renameable human-facing slug.
    pub slug: String,
    /// Locale key for the display name.
    pub name: String,
    /// Preserved author annotation.
    #[serde(rename = "_note", default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Immutable publication coordinate self-identifying the ruleset.
    pub package: crate::PackageId,
    /// Published semantic version of the ruleset.
    #[schemars(with = "crate::schema::VersionText")]
    pub version: Version,
    /// Declared stats in authored order.
    pub stats: Vec<StatDecl>,
    /// Name of the declared health stat.
    pub health_stat: String,
    /// Names of the declared attributes (never containing health).
    pub attributes: Vec<String>,
    /// Action-pool templates in authored order; the first is the primary pool.
    pub pools: Vec<ActionPoolTemplate>,
    /// Ability identities in authored order.
    #[schemars(with = "Vec<crate::schema::UlidText>")]
    pub abilities: Vec<Ulid>,
}

/// One data-selected outcome label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum OutcomeWire {
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

/// Flat damage for one outcome label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DamageEntry {
    /// Outcome label this amount applies to.
    pub outcome: OutcomeWire,
    /// Nonnegative flat damage amount.
    pub amount: u32,
}

/// Authored attack ability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ability {
    /// Stable authored-object identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub id: Ulid,
    /// Renameable human-facing slug.
    pub slug: String,
    /// Locale key for the display name.
    pub name: String,
    /// Preserved author annotation.
    #[serde(rename = "_note", default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Dice notation evaluated by the sim through the rules kernel.
    pub dice: String,
    /// Attribute name checked by this ability (a ruleset attribute).
    pub attribute: String,
    /// Outcome-table identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub outcome_table: Ulid,
    /// Per-outcome flat damage in authored order.
    pub damage: Vec<DamageEntry>,
    /// Primary-pool cost of one use, spent from the ruleset's first pool.
    pub cost: u32,
    /// Non-primary pool costs in authored order.
    pub extra_costs: Vec<AbilityCost>,
    /// Whether a successful use ends the actor's turn.
    pub ends_turn: bool,
    /// Optional effect applied on success; explicit absence omits the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<crate::schema::UlidText>")]
    pub effect: Option<Ulid>,
    /// How the defense value is selected.
    pub defense: DefenseWire,
    /// Optional zero-based raw-die index for natural-face rules; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natural_die: Option<u32>,
    /// Whether a target is required.
    pub requires_target: bool,
    /// Whether self-targeting is allowed.
    pub allow_self_target: bool,
}

/// One margin band: the inclusive lower bound selecting an outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutcomeBandWire {
    /// The inclusive lower margin bound of this band.
    pub min_margin: i64,
    /// The outcome selected while this band applies.
    pub outcome: OutcomeWire,
}

/// What a matching natural-face rule does to the selected band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum NaturalEffectWire {
    /// Shifts the selected band index toward higher margins for positive
    /// values or toward lower margins for negative values.
    Shift(i16),
    /// Replaces the selected band's outcome.
    Override(OutcomeWire),
}

/// One natural-face rule applied to an explicitly selected raw die.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NaturalRuleWire {
    /// The raw face value this rule matches.
    pub face: u32,
    /// What matching does to the selected band.
    pub effect: NaturalEffectWire,
}

/// Authored outcome table: margin bands plus natural-face rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutcomeTable {
    /// Stable authored-object identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub id: Ulid,
    /// Renameable human-facing slug.
    pub slug: String,
    /// Locale key for the display name.
    pub name: String,
    /// Preserved author annotation.
    #[serde(rename = "_note", default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Bands in authored order; never silently sorted.
    pub bands: Vec<OutcomeBandWire>,
    /// Natural-face rules in authored order.
    pub natural_rules: Vec<NaturalRuleWire>,
}

/// One encounter participant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EncounterParticipant {
    /// Placed-creature identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub placement: Ulid,
    /// Authored initiative key; ties are allowed.
    pub initiative: i32,
}

/// Authored encounter: ruleset plus ordered participants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Encounter {
    /// Stable authored-object identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub id: Ulid,
    /// Renameable human-facing slug.
    pub slug: String,
    /// Locale key for the display name.
    pub name: String,
    /// Preserved author annotation.
    #[serde(rename = "_note", default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Ruleset identity governing this encounter.
    #[schemars(with = "crate::schema::UlidText")]
    pub ruleset: Ulid,
    /// Participants in authored order.
    pub participants: Vec<EncounterParticipant>,
}

fn exact_keys<'de, D: serde::Deserializer<'de>>(
    map: &BTreeMap<String, Value>,
    allowed: &'static [&'static str],
) -> Result<(), D::Error> {
    let expected: BTreeSet<&str> = allowed.iter().copied().collect();
    let actual: BTreeSet<&str> = map.keys().map(String::as_str).collect();
    if actual == expected {
        return Ok(());
    }
    for key in &actual {
        if !expected.contains(key) {
            return Err(de::Error::unknown_field(key, allowed));
        }
    }
    for key in &expected {
        if !actual.contains(key) {
            return Err(de::Error::missing_field(key));
        }
    }
    Ok(())
}

fn field<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(
    map: &BTreeMap<String, Value>,
    name: &'static str,
) -> Result<T, D::Error> {
    map.get(name)
        .cloned()
        .ok_or_else(|| de::Error::missing_field(name))
        .and_then(|value| serde_json::from_value(value).map_err(de::Error::custom))
}

fn outcome_from_parts<'de, D: serde::Deserializer<'de>>(
    kind: &str,
    value: Option<Value>,
) -> Result<OutcomeWire, D::Error> {
    match kind {
        "critical_success" => {
            if value.is_some() {
                return Err(de::Error::invalid_length(1, &"unit outcome takes no value"));
            }
            Ok(OutcomeWire::CriticalSuccess)
        }
        "success" => {
            if value.is_some() {
                return Err(de::Error::invalid_length(1, &"unit outcome takes no value"));
            }
            Ok(OutcomeWire::Success)
        }
        "failure" => {
            if value.is_some() {
                return Err(de::Error::invalid_length(1, &"unit outcome takes no value"));
            }
            Ok(OutcomeWire::Failure)
        }
        "critical_failure" => {
            if value.is_some() {
                return Err(de::Error::invalid_length(1, &"unit outcome takes no value"));
            }
            Ok(OutcomeWire::CriticalFailure)
        }
        "custom" => {
            let value = value.ok_or_else(|| de::Error::missing_field("value"))?;
            let byte: u8 = serde_json::from_value(value).map_err(de::Error::custom)?;
            Ok(OutcomeWire::Custom(byte))
        }
        other => Err(de::Error::unknown_variant(
            other,
            &[
                "critical_success",
                "success",
                "failure",
                "critical_failure",
                "custom",
            ],
        )),
    }
}

impl<'de> Deserialize<'de> for OutcomeWire {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        exact_keys::<D>(&map, &["type", "value"]).or_else(|_| exact_keys::<D>(&map, &["type"]))?;
        let kind: String = field::<D, String>(&map, "type")?;
        let value = map.get("value").cloned();
        outcome_from_parts::<D>(&kind, value)
    }
}

impl<'de> Deserialize<'de> for RefreshWire {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        exact_keys::<D>(&map, &["type", "value"]).or_else(|_| exact_keys::<D>(&map, &["type"]))?;
        let kind: String = field::<D, String>(&map, "type")?;
        let value = map.get("value").cloned();
        match kind.as_str() {
            "on_turn_start" => {
                if value.is_some() {
                    return Err(de::Error::invalid_length(1, &"unit trigger takes no value"));
                }
                Ok(RefreshWire::OnTurnStart)
            }
            "on_round_start" => {
                if value.is_some() {
                    return Err(de::Error::invalid_length(1, &"unit trigger takes no value"));
                }
                Ok(RefreshWire::OnRoundStart)
            }
            "on_rest" => {
                let value = value.ok_or_else(|| de::Error::missing_field("value"))?;
                let rest: RestId = serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(RefreshWire::OnRest(rest))
            }
            "on_tick" => {
                let value = value.ok_or_else(|| de::Error::missing_field("value"))?;
                let period: TickPeriod =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(RefreshWire::OnTick(period))
            }
            "never" => {
                if value.is_some() {
                    return Err(de::Error::invalid_length(1, &"unit trigger takes no value"));
                }
                Ok(RefreshWire::Never)
            }
            other => Err(de::Error::unknown_variant(
                other,
                &[
                    "on_turn_start",
                    "on_round_start",
                    "on_rest",
                    "on_tick",
                    "never",
                ],
            )),
        }
    }
}

impl<'de> Deserialize<'de> for NaturalEffectWire {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        exact_keys::<D>(&map, &["type", "value"])?;
        let kind: String = field::<D, String>(&map, "type")?;
        let value: Value = field::<D, Value>(&map, "value")?;
        match kind.as_str() {
            "shift" => {
                let steps: i16 = serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(NaturalEffectWire::Shift(steps))
            }
            "override" => {
                let outcome: OutcomeWire =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(NaturalEffectWire::Override(outcome))
            }
            other => Err(de::Error::unknown_variant(other, &["shift", "override"])),
        }
    }
}

impl<'de> Deserialize<'de> for DefenseWire {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        exact_keys::<D>(&map, &["type", "value"]).or_else(|_| exact_keys::<D>(&map, &["type"]))?;
        let kind: String = field::<D, String>(&map, "type")?;
        let value = map.get("value").cloned();
        match kind.as_str() {
            "actor_attribute" => {
                if value.is_some() {
                    return Err(de::Error::invalid_length(1, &"unit defense takes no value"));
                }
                Ok(DefenseWire::ActorAttribute)
            }
            "target_stat" => {
                let value = value.ok_or_else(|| de::Error::missing_field("value"))?;
                let inner: BTreeMap<String, Value> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                exact_keys::<D>(&inner, &["stat"])?;
                let stat: String =
                    serde_json::from_value(inner["stat"].clone()).map_err(de::Error::custom)?;
                Ok(DefenseWire::TargetStat { stat })
            }
            other => Err(de::Error::unknown_variant(
                other,
                &["actor_attribute", "target_stat"],
            )),
        }
    }
}

impl<'de> Deserialize<'de> for EffectAimWire {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        exact_keys::<D>(&map, &["type", "value"]).or_else(|_| exact_keys::<D>(&map, &["type"]))?;
        let kind: String = field::<D, String>(&map, "type")?;
        let value = map.get("value").cloned();
        match kind.as_str() {
            "slf" => {
                if value.is_some() {
                    return Err(de::Error::invalid_length(1, &"unit aim takes no value"));
                }
                Ok(EffectAimWire::Slf)
            }
            "target" => {
                if value.is_some() {
                    return Err(de::Error::invalid_length(1, &"unit aim takes no value"));
                }
                Ok(EffectAimWire::Target)
            }
            other => Err(de::Error::unknown_variant(other, &["slf", "target"])),
        }
    }
}

impl<'de> Deserialize<'de> for EffectTargetWire {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        exact_keys::<D>(&map, &["type", "value"]).or_else(|_| exact_keys::<D>(&map, &["type"]))?;
        let kind: String = field::<D, String>(&map, "type")?;
        let value = map.get("value").cloned();
        match kind.as_str() {
            "roll" => {
                if value.is_some() {
                    return Err(de::Error::invalid_length(1, &"unit target takes no value"));
                }
                Ok(EffectTargetWire::Roll)
            }
            "dc" => {
                if value.is_some() {
                    return Err(de::Error::invalid_length(1, &"unit target takes no value"));
                }
                Ok(EffectTargetWire::Dc)
            }
            other => Err(de::Error::unknown_variant(other, &["roll", "dc"])),
        }
    }
}

impl<'de> Deserialize<'de> for EffectOpWire {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        exact_keys::<D>(&map, &["type", "value"]).or_else(|_| exact_keys::<D>(&map, &["type"]))?;
        let kind: String = field::<D, String>(&map, "type")?;
        let value = map.get("value").cloned();
        match kind.as_str() {
            "add" => {
                if value.is_some() {
                    return Err(de::Error::invalid_length(1, &"unit op takes no value"));
                }
                Ok(EffectOpWire::Add)
            }
            "set" => {
                if value.is_some() {
                    return Err(de::Error::invalid_length(1, &"unit op takes no value"));
                }
                Ok(EffectOpWire::Set)
            }
            other => Err(de::Error::unknown_variant(other, &["add", "set"])),
        }
    }
}

impl<'de> Deserialize<'de> for PolicyWire {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        exact_keys::<D>(&map, &["type", "value"]).or_else(|_| exact_keys::<D>(&map, &["type"]))?;
        let kind: String = field::<D, String>(&map, "type")?;
        let value = map.get("value").cloned();
        match kind.as_str() {
            "stack_all" => {
                if value.is_some() {
                    return Err(de::Error::invalid_length(1, &"unit policy takes no value"));
                }
                Ok(PolicyWire::StackAll)
            }
            "highest_bonus_worst_penalty" => {
                if value.is_some() {
                    return Err(de::Error::invalid_length(1, &"unit policy takes no value"));
                }
                Ok(PolicyWire::HighestBonusWorstPenalty)
            }
            "highest_priority_per_name" => {
                if value.is_some() {
                    return Err(de::Error::invalid_length(1, &"unit policy takes no value"));
                }
                Ok(PolicyWire::HighestPriorityPerName)
            }
            other => Err(de::Error::unknown_variant(
                other,
                &[
                    "stack_all",
                    "highest_bonus_worst_penalty",
                    "highest_priority_per_name",
                ],
            )),
        }
    }
}

/// Checks single-document combat invariants shared by the byte reader.
///
/// All checks are local to one document; cross-document references belong to
/// semantic validation. Failures are [`DataError::Malformed`].
pub(crate) fn validate_combat_local(document: &crate::Document) -> Result<(), DataError> {
    fn malformed(message: impl std::fmt::Display) -> DataError {
        crate::error::malformed(message)
    }
    match document {
        crate::Document::Ruleset(ruleset) => {
            if ruleset.slug.is_empty() || ruleset.name.is_empty() {
                return Err(malformed("ruleset slug and name must be nonempty"));
            }
            if ruleset.stats.is_empty() {
                return Err(malformed("ruleset stats must be nonempty"));
            }
            let mut names = BTreeSet::new();
            for stat in &ruleset.stats {
                if stat.name.is_empty() {
                    return Err(malformed("ruleset stat names must be nonempty"));
                }
                if !names.insert(stat.name.as_str()) {
                    return Err(malformed("duplicate ruleset stat name"));
                }
            }
            if ruleset.health_stat.is_empty() || !names.contains(ruleset.health_stat.as_str()) {
                return Err(malformed("ruleset health_stat must name a declared stat"));
            }
            if ruleset.attributes.is_empty() {
                return Err(malformed("ruleset attributes must be nonempty"));
            }
            let mut seen_attributes = BTreeSet::new();
            for attribute in &ruleset.attributes {
                if attribute.is_empty()
                    || !names.contains(attribute.as_str())
                    || *attribute == ruleset.health_stat
                {
                    return Err(malformed(
                        "ruleset attributes must name declared non-health stats",
                    ));
                }
                if !seen_attributes.insert(attribute.as_str()) {
                    return Err(malformed("duplicate ruleset attribute"));
                }
            }
            if ruleset.pools.is_empty() {
                return Err(malformed("ruleset pools must be nonempty"));
            }
            let mut seen_pools = BTreeSet::new();
            for pool in &ruleset.pools {
                if !seen_pools.insert(pool.id) {
                    return Err(malformed("duplicate ruleset pool id"));
                }
                if matches!(pool.refresh, RefreshWire::OnTick(period) if period.0 == 0) {
                    return Err(malformed("action pool tick period must be nonzero"));
                }
            }
            if ruleset.abilities.is_empty() {
                return Err(malformed("ruleset abilities must be nonempty"));
            }
            let mut seen_abilities = BTreeSet::new();
            for ability in &ruleset.abilities {
                if !seen_abilities.insert(ability) {
                    return Err(malformed("duplicate ruleset ability"));
                }
            }
            Ok(())
        }
        crate::Document::Ability(ability) => {
            if ability.slug.is_empty() || ability.name.is_empty() {
                return Err(malformed("ability slug and name must be nonempty"));
            }
            let bytes = ability.dice.len();
            if bytes == 0 || bytes > MAX_DICE_INPUT_BYTES {
                return Err(malformed("ability dice must be 1..=128 UTF-8 bytes"));
            }
            if ability.attribute.is_empty() {
                return Err(malformed("ability attribute must be nonempty"));
            }
            if ability.damage.is_empty() {
                return Err(malformed("ability damage must be nonempty"));
            }
            let mut seen_outcomes = BTreeSet::new();
            for entry in &ability.damage {
                let discriminant = match &entry.outcome {
                    OutcomeWire::CriticalSuccess => (0u8, 0u8),
                    OutcomeWire::Success => (1, 0),
                    OutcomeWire::Failure => (2, 0),
                    OutcomeWire::CriticalFailure => (3, 0),
                    OutcomeWire::Custom(byte) => (4, *byte),
                };
                if !seen_outcomes.insert(discriminant) {
                    return Err(malformed("duplicate ability damage outcome"));
                }
            }
            for cost in &ability.extra_costs {
                if cost.amount == 0 {
                    return Err(malformed("ability extra cost amounts must be at least one"));
                }
            }
            if ability.cost == 0 && ability.extra_costs.is_empty() {
                return Err(malformed(
                    "ability must spend something: cost or extra costs",
                ));
            }
            if let DefenseWire::TargetStat { stat } = &ability.defense {
                if stat.is_empty() {
                    return Err(malformed("ability defense stat must be nonempty"));
                }
            }
            Ok(())
        }
        crate::Document::OutcomeTable(table) => {
            if table.slug.is_empty() || table.name.is_empty() {
                return Err(malformed("outcome table slug and name must be nonempty"));
            }
            if table.bands.is_empty() || table.bands.len() > MAX_OUTCOME_BANDS {
                return Err(malformed("outcome table bands must be 1..=256"));
            }
            for (index, band) in table.bands.iter().enumerate() {
                if index == 0 {
                    if band.min_margin != i64::MIN {
                        return Err(malformed("first outcome band must start at i64::MIN"));
                    }
                } else if band.min_margin <= table.bands[index - 1].min_margin {
                    return Err(malformed("outcome bands must strictly increase"));
                }
            }
            if table.natural_rules.len() > MAX_NATURAL_RULES {
                return Err(malformed("outcome table natural rules must be <=256"));
            }
            let mut seen_faces = BTreeSet::new();
            for rule in &table.natural_rules {
                if rule.face == 0 || rule.face > MAX_DIE_SIDES {
                    return Err(malformed("natural rule faces must be 1..=1000000"));
                }
                if !seen_faces.insert(rule.face) {
                    return Err(malformed("duplicate natural rule face"));
                }
            }
            Ok(())
        }
        crate::Document::Encounter(encounter) => {
            if encounter.slug.is_empty() || encounter.name.is_empty() {
                return Err(malformed("encounter slug and name must be nonempty"));
            }
            if encounter.participants.is_empty() {
                return Err(malformed("encounter participants must be nonempty"));
            }
            let mut seen_placements = BTreeSet::new();
            for participant in &encounter.participants {
                if !seen_placements.insert(participant.placement) {
                    return Err(malformed("duplicate encounter participant"));
                }
            }
            Ok(())
        }
        crate::Document::Effect(effect) => {
            if effect.slug.is_empty() || effect.name.is_empty() {
                return Err(malformed("effect slug and name must be nonempty"));
            }
            if effect.mod_type.is_empty() {
                return Err(malformed("effect mod_type must be nonempty"));
            }
            if effect.modifiers.is_empty() || effect.modifiers.len() > MAX_EFFECT_MODIFIERS {
                return Err(malformed("effect modifiers must be 1..=4096"));
            }
            let mut seen_modifiers = BTreeSet::new();
            for modifier in &effect.modifiers {
                if !seen_modifiers.insert(modifier.id) {
                    return Err(malformed("duplicate effect modifier id"));
                }
            }
            if effect.duration_rounds == 0 {
                return Err(malformed("effect duration must be at least one round"));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
