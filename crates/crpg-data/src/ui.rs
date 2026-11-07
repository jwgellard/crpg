//! `crpg.ui/1`: ruleset-described player UI (ADR-0029 U1-U10).
//!
//! The document is presentation data. It is never loaded into a `World`, never
//! hashed, and the server ignores it. This module owns the wire model, the
//! read-time checks, the borrowed [`UiSource`] and the two pure default
//! functions [`generated_ui`] and [`effective_ui`]. Semantic validation of a
//! UI document against its ruleset lives in `validation`.
//!
//! Six closed sets are *tolerant* (U6): an unknown spelling decodes to an
//! `Unknown` arm and re-encodes byte-identically, so a misspelt kind fails
//! campaign validation but never `load_campaign`. Every other structural
//! defect stays `Malformed`, as in every other family. Never make a tolerant
//! set strict, and never let the JSON Schema list `Unknown`.

use crate::combat::{exact_keys, field};
use crate::error::malformed;
use crate::{
    Ability, DataError, Document, Effect, NaturalEffectWire, OutcomeTable, OutcomeWire, Ruleset,
    SourcePath,
};
use crpg_core::Ulid;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::ser::SerializeMap;
use serde::{de, Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::BTreeMap;

/// The document tag (`Document::Ui`'s serde rename).
pub const UI_SCHEMA: &str = "crpg.ui/1";
/// Most entries in a top-level list, a selector list, `category_tags` or enum labels.
pub const MAX_UI_ENTRIES: usize = 4096;
/// Most screens in a document, and most sections in a screen.
pub const MAX_UI_SECTIONS: usize = 64;
/// Longest string, in UTF-8 bytes (`str::len`, never `chars().count()`).
pub const MAX_UI_TEXT_BYTES: usize = 128;
/// Known screen kinds in U4 order; `level_up` and `character_creation` are reserved.
pub const UI_SCREEN_KINDS: [&str; 7] = [
    "character_sheet",
    "action_bar",
    "spellbook",
    "inventory",
    "level_up",
    "combat_hud",
    "character_creation",
];
/// Known section types in U4 order; `choice_steps` is reserved.
pub const UI_SECTION_TYPES: [&str; 8] = [
    "stat_group",
    "pool_group",
    "ability_list",
    "effect_list",
    "item_list",
    "timeline",
    "log",
    "choice_steps",
];
/// Known stat formats in U3 order.
pub const UI_FORMAT_TYPES: [&str; 8] = [
    "integer", "signed", "fraction", "percent", "boolean", "enum", "tags", "dice",
];
/// Known pool styles in U3 order.
pub const UI_POOL_STYLES: [&str; 3] = ["pips", "bar", "number"];
/// Known target hints.
pub const UI_TARGET_HINTS: [&str; 4] = ["none", "enemy", "ally", "any"];
/// Known ability-list sorts.
pub const UI_SORTS: [&str; 2] = ["declared", "label"];

// Positions in the two payload-bearing constant lists. A unit test pins each
// index to its spelling, so reordering a constant fails loudly.
const FMT_INTEGER: usize = 0;
const FMT_SIGNED: usize = 1;
const FMT_FRACTION: usize = 2;
const FMT_PERCENT: usize = 3;
const FMT_BOOLEAN: usize = 4;
const FMT_ENUM: usize = 5;
const FMT_TAGS: usize = 6;
const FMT_DICE: usize = 7;
const SEC_STAT_GROUP: usize = 0;
const SEC_POOL_GROUP: usize = 1;
const SEC_ABILITY_LIST: usize = 2;
const SEC_EFFECT_LIST: usize = 3;
const SEC_ITEM_LIST: usize = 4;
const SEC_TIMELINE: usize = 5;
const SEC_LOG: usize = 6;
const SEC_CHOICE_STEPS: usize = 7;

/// One `crpg.ui/1` document: presentation data for one ruleset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiDocument {
    /// The ruleset this document describes.
    #[schemars(with = "crate::schema::UlidText")]
    pub ruleset: Ulid,
    /// Stat display entries; absent means generated for every stat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<Vec<UiStat>>,
    /// Pool display entries; absent means generated for every pool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pools: Option<Vec<UiPool>>,
    /// Ability display entries; absent means generated for every listed ability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abilities: Option<Vec<UiAbility>>,
    /// Effect display entries; absent means generated for every applied effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<UiEffect>>,
    /// Outcome labels; absent means generated for every produced outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcomes: Option<Vec<UiOutcome>>,
    /// Screens; absent means every non-reserved default screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screens: Option<Vec<UiScreen>>,
    /// Preserved author annotation.
    #[serde(rename = "_note", default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Display entry for one declared stat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiStat {
    /// Declared stat name.
    pub stat: String,
    /// Locale key of the label.
    pub label: String,
    /// Locale key of the short label; absent means use `label`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_label: Option<String>,
    /// Locale key of the description; absent means none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Locale key naming the group; absent means ungrouped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// How to format the value; absent means render the replicated value by its own type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<UiFormat>,
}

/// One enum-stat variant label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiEnumLabel {
    /// A variant of the stat's enum.
    pub variant: String,
    /// Locale key of the label.
    pub label: String,
}

/// Display entry for one action-pool template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiPool {
    /// A pool-template id of the ruleset.
    #[schemars(with = "crate::schema::UlidText")]
    pub pool: Ulid,
    /// Locale key of the label.
    pub label: String,
    /// `assets.lock` key of the icon; absent means none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// How to draw the pool; absent means a bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<UiPoolStyle>,
    /// Whether the pool shows on the HUD; absent means true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_on_hud: Option<bool>,
}

/// Display entry for one ability listed by the ruleset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiAbility {
    /// An ability listed by the ruleset.
    #[schemars(with = "crate::schema::UlidText")]
    pub ability: Ulid,
    /// Locale key of the label.
    pub label: String,
    /// Locale key of the description; absent means none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `assets.lock` key of the icon; absent means none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Category tags; absent means none, nonempty when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_tags: Option<Vec<String>>,
    /// Who the ability may target; absent means no hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_hint: Option<UiTargetHint>,
    /// Whether odds are shown (T143 and T051 read it); absent means false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_odds: Option<bool>,
}

/// Display entry for one effect applied by a listed ability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiEffect {
    /// An effect applied by a listed ability.
    #[schemars(with = "crate::schema::UlidText")]
    pub effect: Ulid,
    /// Locale key of the label.
    pub label: String,
    /// `assets.lock` key of the icon; absent means none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Whether the remaining duration shows; absent means true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_duration: Option<bool>,
}

/// Label for one outcome the ruleset can produce.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiOutcome {
    /// Outcome spelling, for example `success` or `custom:3`.
    pub outcome: String,
    /// Locale key of the label.
    pub label: String,
}

/// One screen: a kind and its sections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiScreen {
    /// Which screen this is.
    pub kind: UiScreenKind,
    /// Sections in display order; may be empty, at most 64.
    pub sections: Vec<UiSection>,
}

/// Payload of an `ability_list` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiAbilityList {
    /// Which abilities to list.
    pub select: UiAbilitySelect,
    /// Ordering; absent means declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<UiSort>,
}

/// The value a `fraction` format divides by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum UiMeter {
    /// A declared stat, by name.
    Stat(String),
    /// A pool template of the ruleset.
    Pool(#[schemars(with = "crate::schema::UlidText")] Ulid),
}

/// How a stat value is formatted (U3). Unknown spellings are tolerated (U6).
#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum UiFormat {
    /// A whole number.
    Integer,
    /// A whole number with an explicit sign.
    Signed,
    /// The value over a meter, for example `3 / 5`.
    Fraction(UiMeter),
    /// A percentage.
    Percent,
    /// A yes or no flag.
    Boolean,
    /// An enum with a label per variant.
    Enum(Vec<UiEnumLabel>),
    /// A tag list.
    Tags,
    /// A dice expression.
    Dice,
    /// A kind this schema version does not define; round-trips byte-identically.
    #[schemars(skip)]
    Unknown {
        /// The unrecognised `type` text.
        kind: String,
        /// The `value` member, when the object had one.
        value: Option<Value>,
    },
}

/// One section of a screen (U4). Unknown spellings are tolerated (U6).
#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum UiSection {
    /// Stats chosen by a selector.
    StatGroup(UiStatSelect),
    /// Pools chosen by a selector.
    PoolGroup(UiPoolSelect),
    /// Abilities chosen by a selector, with an ordering.
    AbilityList(UiAbilityList),
    /// The effects currently on the combatant (a runtime fact).
    EffectList,
    /// Items chosen by a selector.
    ItemList(UiItemSelect),
    /// The turn-order timeline.
    Timeline,
    /// The combat log.
    Log,
    /// Reserved until the level-up and creation data families exist.
    ChoiceSteps,
    /// A kind this schema version does not define; round-trips byte-identically.
    #[schemars(skip)]
    Unknown {
        /// The unrecognised `type` text.
        kind: String,
        /// The `value` member, when the object had one.
        value: Option<Value>,
    },
}

/// Which stats a `stat_group` section shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum UiStatSelect {
    /// Every stat.
    All,
    /// Stats whose `stats` entry has this group key.
    Group(String),
    /// These stats, by name.
    Names(Vec<String>),
}

/// Which pools a `pool_group` section shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum UiPoolSelect {
    /// Every pool.
    All,
    /// These pool templates.
    Ids(#[schemars(with = "Vec<crate::schema::UlidText>")] Vec<Ulid>),
}

/// Which abilities an `ability_list` section shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum UiAbilitySelect {
    /// Every ability.
    All,
    /// These abilities.
    Ids(#[schemars(with = "Vec<crate::schema::UlidText>")] Vec<Ulid>),
    /// Abilities whose `abilities` entry carries this category tag.
    Tag(String),
}

/// Which items an `item_list` section shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum UiItemSelect {
    /// Every item.
    All,
    /// Items carrying this tag.
    Tag(String),
}

/// Screen kind (U4). Unknown spellings are tolerated (U6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiScreenKind {
    /// The character sheet.
    CharacterSheet,
    /// The action bar.
    ActionBar,
    /// The spellbook.
    Spellbook,
    /// The inventory.
    Inventory,
    /// Reserved until the level-up data family exists.
    LevelUp,
    /// The combat HUD.
    CombatHud,
    /// Reserved until the character-creation data family exists.
    CharacterCreation,
    /// A kind this schema version does not define.
    Unknown(String),
}

/// Pool drawing style (U3). Unknown spellings are tolerated (U6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiPoolStyle {
    /// One pip per point.
    Pips,
    /// A filled bar.
    Bar,
    /// A bare number.
    Number,
    /// A style this schema version does not define.
    Unknown(String),
}

/// Who an ability may target (U3). Unknown spellings are tolerated (U6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiTargetHint {
    /// No target is needed (spelt `none`).
    NoTarget,
    /// An enemy.
    Enemy,
    /// An ally.
    Ally,
    /// Anyone.
    Any,
    /// A hint this schema version does not define.
    Unknown(String),
}

/// Ordering of an `ability_list` section. Unknown spellings are tolerated (U6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiSort {
    /// Declared order.
    Declared,
    /// Display-label order.
    Label,
    /// A sort this schema version does not define.
    Unknown(String),
}

// The four plain-string tolerant enums share one implementation. The wire
// spellings come only from the constants; the variant list is positional.
macro_rules! tolerant_string_impls {
    ($name:ident, $consts:ident, [$($known:ident),+ $(,)?]) => {
        impl $name {
            fn known() -> [Self; $consts.len()] {
                [$(Self::$known),+]
            }

            /// The wire spelling, or the `Unknown` text.
            pub fn as_str(&self) -> &str {
                match self {
                    Self::Unknown(text) => text,
                    known => Self::known()
                        .iter()
                        .zip($consts.iter())
                        .find(|(variant, _)| *variant == known)
                        .map(|(_, spelling)| *spelling)
                        .unwrap_or_default(),
                }
            }

            fn from_text(text: String) -> Self {
                Self::known()
                    .into_iter()
                    .zip($consts.iter())
                    .find(|(_, spelling)| **spelling == text)
                    .map(|(variant, _)| variant)
                    .unwrap_or(Self::Unknown(text))
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                Ok(Self::from_text(String::deserialize(deserializer)?))
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                Schema::try_from(serde_json::json!({"type": "string", "enum": $consts}))
                    .expect("object schema")
            }
        }
    };
}

tolerant_string_impls!(
    UiScreenKind,
    UI_SCREEN_KINDS,
    [
        CharacterSheet,
        ActionBar,
        Spellbook,
        Inventory,
        LevelUp,
        CombatHud,
        CharacterCreation
    ]
);
tolerant_string_impls!(UiPoolStyle, UI_POOL_STYLES, [Pips, Bar, Number]);
tolerant_string_impls!(UiTargetHint, UI_TARGET_HINTS, [NoTarget, Enemy, Ally, Any]);
tolerant_string_impls!(UiSort, UI_SORTS, [Declared, Label]);

fn serialize_adjacent<S: serde::Serializer, T: Serialize + ?Sized>(
    serializer: S,
    kind: &str,
    value: Option<&T>,
) -> Result<S::Ok, S::Error> {
    let mut map = serializer.serialize_map(Some(1 + usize::from(value.is_some())))?;
    map.serialize_entry("type", kind)?;
    if let Some(value) = value {
        map.serialize_entry("value", value)?;
    }
    map.end()
}

/// The keys of an unknown tolerant object: exactly `{type}` or `{type, value}`.
fn unknown_parts<'de, D: serde::Deserializer<'de>>(
    map: &BTreeMap<String, Value>,
    kind: &str,
) -> Result<(String, Option<Value>), D::Error> {
    exact_keys::<D>(map, &["type", "value"]).or_else(|_| exact_keys::<D>(map, &["type"]))?;
    Ok((kind.to_owned(), map.get("value").cloned()))
}

impl UiFormat {
    /// The wire spelling, or the `Unknown` text.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Integer => UI_FORMAT_TYPES[FMT_INTEGER],
            Self::Signed => UI_FORMAT_TYPES[FMT_SIGNED],
            Self::Fraction(_) => UI_FORMAT_TYPES[FMT_FRACTION],
            Self::Percent => UI_FORMAT_TYPES[FMT_PERCENT],
            Self::Boolean => UI_FORMAT_TYPES[FMT_BOOLEAN],
            Self::Enum(_) => UI_FORMAT_TYPES[FMT_ENUM],
            Self::Tags => UI_FORMAT_TYPES[FMT_TAGS],
            Self::Dice => UI_FORMAT_TYPES[FMT_DICE],
            Self::Unknown { kind, .. } => kind,
        }
    }
}

impl Serialize for UiFormat {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let kind = self.as_str();
        match self {
            Self::Fraction(meter) => serialize_adjacent(serializer, kind, Some(meter)),
            Self::Enum(labels) => serialize_adjacent(serializer, kind, Some(labels)),
            Self::Unknown { value, .. } => serialize_adjacent(serializer, kind, value.as_ref()),
            _ => serialize_adjacent::<S, ()>(serializer, kind, None),
        }
    }
}

impl<'de> Deserialize<'de> for UiFormat {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        let kind: String = field::<D, String>(&map, "type")?;
        let unit = |map: &BTreeMap<String, Value>, format: Self| {
            exact_keys::<D>(map, &["type"])?;
            Ok(format)
        };
        match UI_FORMAT_TYPES.iter().position(|known| *known == kind) {
            Some(FMT_INTEGER) => unit(&map, Self::Integer),
            Some(FMT_SIGNED) => unit(&map, Self::Signed),
            Some(FMT_PERCENT) => unit(&map, Self::Percent),
            Some(FMT_BOOLEAN) => unit(&map, Self::Boolean),
            Some(FMT_TAGS) => unit(&map, Self::Tags),
            Some(FMT_DICE) => unit(&map, Self::Dice),
            Some(FMT_FRACTION) => {
                exact_keys::<D>(&map, &["type", "value"])?;
                Ok(Self::Fraction(field::<D, UiMeter>(&map, "value")?))
            }
            Some(FMT_ENUM) => {
                exact_keys::<D>(&map, &["type", "value"])?;
                Ok(Self::Enum(field::<D, Vec<UiEnumLabel>>(&map, "value")?))
            }
            _ => {
                let (kind, value) = unknown_parts::<D>(&map, &kind)?;
                Ok(Self::Unknown { kind, value })
            }
        }
    }
}

impl UiSection {
    /// The wire spelling, or the `Unknown` text.
    pub fn as_str(&self) -> &str {
        match self {
            Self::StatGroup(_) => UI_SECTION_TYPES[SEC_STAT_GROUP],
            Self::PoolGroup(_) => UI_SECTION_TYPES[SEC_POOL_GROUP],
            Self::AbilityList(_) => UI_SECTION_TYPES[SEC_ABILITY_LIST],
            Self::EffectList => UI_SECTION_TYPES[SEC_EFFECT_LIST],
            Self::ItemList(_) => UI_SECTION_TYPES[SEC_ITEM_LIST],
            Self::Timeline => UI_SECTION_TYPES[SEC_TIMELINE],
            Self::Log => UI_SECTION_TYPES[SEC_LOG],
            Self::ChoiceSteps => UI_SECTION_TYPES[SEC_CHOICE_STEPS],
            Self::Unknown { kind, .. } => kind,
        }
    }
}

impl Serialize for UiSection {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let kind = self.as_str();
        match self {
            Self::StatGroup(select) => serialize_adjacent(serializer, kind, Some(select)),
            Self::PoolGroup(select) => serialize_adjacent(serializer, kind, Some(select)),
            Self::AbilityList(list) => serialize_adjacent(serializer, kind, Some(list)),
            Self::ItemList(select) => serialize_adjacent(serializer, kind, Some(select)),
            Self::Unknown { value, .. } => serialize_adjacent(serializer, kind, value.as_ref()),
            _ => serialize_adjacent::<S, ()>(serializer, kind, None),
        }
    }
}

impl<'de> Deserialize<'de> for UiSection {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        let kind: String = field::<D, String>(&map, "type")?;
        let unit = |map: &BTreeMap<String, Value>, section: Self| {
            exact_keys::<D>(map, &["type"])?;
            Ok(section)
        };
        let known = UI_SECTION_TYPES.iter().position(|known| *known == kind);
        if matches!(
            known,
            Some(SEC_STAT_GROUP | SEC_POOL_GROUP | SEC_ABILITY_LIST | SEC_ITEM_LIST)
        ) {
            exact_keys::<D>(&map, &["type", "value"])?;
        }
        match known {
            Some(SEC_STAT_GROUP) => Ok(Self::StatGroup(field::<D, UiStatSelect>(&map, "value")?)),
            Some(SEC_POOL_GROUP) => Ok(Self::PoolGroup(field::<D, UiPoolSelect>(&map, "value")?)),
            Some(SEC_ABILITY_LIST) => {
                Ok(Self::AbilityList(field::<D, UiAbilityList>(&map, "value")?))
            }
            Some(SEC_ITEM_LIST) => Ok(Self::ItemList(field::<D, UiItemSelect>(&map, "value")?)),
            Some(SEC_EFFECT_LIST) => unit(&map, Self::EffectList),
            Some(SEC_TIMELINE) => unit(&map, Self::Timeline),
            Some(SEC_LOG) => unit(&map, Self::Log),
            Some(SEC_CHOICE_STEPS) => unit(&map, Self::ChoiceSteps),
            _ => {
                let (kind, value) = unknown_parts::<D>(&map, &kind)?;
                Ok(Self::Unknown { kind, value })
            }
        }
    }
}

impl<'de> Deserialize<'de> for UiStatSelect {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        let kind: String = field::<D, String>(&map, "type")?;
        match kind.as_str() {
            "all" => {
                exact_keys::<D>(&map, &["type"])?;
                Ok(Self::All)
            }
            "group" => {
                exact_keys::<D>(&map, &["type", "value"])?;
                Ok(Self::Group(field::<D, String>(&map, "value")?))
            }
            "names" => {
                exact_keys::<D>(&map, &["type", "value"])?;
                Ok(Self::Names(field::<D, Vec<String>>(&map, "value")?))
            }
            other => Err(de::Error::unknown_variant(
                other,
                &["all", "group", "names"],
            )),
        }
    }
}

impl<'de> Deserialize<'de> for UiPoolSelect {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        let kind: String = field::<D, String>(&map, "type")?;
        match kind.as_str() {
            "all" => {
                exact_keys::<D>(&map, &["type"])?;
                Ok(Self::All)
            }
            "ids" => {
                exact_keys::<D>(&map, &["type", "value"])?;
                Ok(Self::Ids(field::<D, Vec<Ulid>>(&map, "value")?))
            }
            other => Err(de::Error::unknown_variant(other, &["all", "ids"])),
        }
    }
}

impl<'de> Deserialize<'de> for UiAbilitySelect {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        let kind: String = field::<D, String>(&map, "type")?;
        match kind.as_str() {
            "all" => {
                exact_keys::<D>(&map, &["type"])?;
                Ok(Self::All)
            }
            "ids" => {
                exact_keys::<D>(&map, &["type", "value"])?;
                Ok(Self::Ids(field::<D, Vec<Ulid>>(&map, "value")?))
            }
            "tag" => {
                exact_keys::<D>(&map, &["type", "value"])?;
                Ok(Self::Tag(field::<D, String>(&map, "value")?))
            }
            other => Err(de::Error::unknown_variant(other, &["all", "ids", "tag"])),
        }
    }
}

impl<'de> Deserialize<'de> for UiItemSelect {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        let kind: String = field::<D, String>(&map, "type")?;
        match kind.as_str() {
            "all" => {
                exact_keys::<D>(&map, &["type"])?;
                Ok(Self::All)
            }
            "tag" => {
                exact_keys::<D>(&map, &["type", "value"])?;
                Ok(Self::Tag(field::<D, String>(&map, "value")?))
            }
            other => Err(de::Error::unknown_variant(other, &["all", "tag"])),
        }
    }
}

// ---------------------------------------------------------------------------
// Read-time checks (B5, B6).

fn check_text(text: &str, pointer: &str) -> Result<(), DataError> {
    if text.is_empty() || text.len() > MAX_UI_TEXT_BYTES {
        return Err(malformed(format!(
            "ui text at {pointer} must be 1..={MAX_UI_TEXT_BYTES} UTF-8 bytes"
        )));
    }
    Ok(())
}

fn check_opt_text(text: &Option<String>, pointer: &str) -> Result<(), DataError> {
    text.as_deref()
        .map_or(Ok(()), |text| check_text(text, pointer))
}

/// Count first, then emptiness, as B5 orders them.
fn check_list(len: usize, max: usize, nonempty: bool, pointer: &str) -> Result<(), DataError> {
    if len > max {
        return Err(malformed(format!(
            "ui list at {pointer} exceeds {max} entries"
        )));
    }
    if nonempty && len == 0 {
        return Err(malformed(format!("ui list at {pointer} must not be empty")));
    }
    Ok(())
}

/// B6: an `Unknown` arm may hold only a spelling its set does not define.
fn check_unknown(text: &str, known: &[&str], what: &str, pointer: &str) -> Result<(), DataError> {
    check_text(text, pointer)?;
    if known.contains(&text) {
        return Err(malformed(format!(
            "ui unknown {what} at {pointer} uses a known spelling"
        )));
    }
    Ok(())
}

fn check_ids(len: usize, pointer: &str) -> Result<(), DataError> {
    check_list(len, MAX_UI_ENTRIES, true, pointer)
}

fn check_names(names: &[String], pointer: &str) -> Result<(), DataError> {
    check_list(names.len(), MAX_UI_ENTRIES, true, pointer)?;
    for (k, name) in names.iter().enumerate() {
        check_text(name, &format!("{pointer}/{k}"))?;
    }
    Ok(())
}

fn check_format(format: &UiFormat, pointer: &str) -> Result<(), DataError> {
    match format {
        UiFormat::Fraction(UiMeter::Stat(name)) => {
            check_text(name, &format!("{pointer}/value/value"))
        }
        UiFormat::Enum(labels) => {
            let list = format!("{pointer}/value");
            check_list(labels.len(), MAX_UI_ENTRIES, true, &list)?;
            for (k, label) in labels.iter().enumerate() {
                check_text(&label.variant, &format!("{list}/{k}/variant"))?;
                check_text(&label.label, &format!("{list}/{k}/label"))?;
            }
            Ok(())
        }
        UiFormat::Unknown { kind, .. } => {
            check_unknown(kind, &UI_FORMAT_TYPES, "format", &format!("{pointer}/type"))
        }
        _ => Ok(()),
    }
}

fn check_section(section: &UiSection, pointer: &str) -> Result<(), DataError> {
    match section {
        UiSection::Unknown { kind, .. } => check_unknown(
            kind,
            &UI_SECTION_TYPES,
            "section type",
            &format!("{pointer}/type"),
        ),
        UiSection::StatGroup(UiStatSelect::All)
        | UiSection::PoolGroup(UiPoolSelect::All)
        | UiSection::ItemList(UiItemSelect::All)
        | UiSection::EffectList
        | UiSection::Timeline
        | UiSection::Log
        | UiSection::ChoiceSteps => Ok(()),
        UiSection::StatGroup(UiStatSelect::Group(group)) => {
            check_text(group, &format!("{pointer}/value/value"))
        }
        UiSection::StatGroup(UiStatSelect::Names(names)) => {
            check_names(names, &format!("{pointer}/value/value"))
        }
        UiSection::PoolGroup(UiPoolSelect::Ids(ids)) => {
            check_ids(ids.len(), &format!("{pointer}/value/value"))
        }
        UiSection::ItemList(UiItemSelect::Tag(tag)) => {
            check_text(tag, &format!("{pointer}/value/value"))
        }
        UiSection::AbilityList(list) => {
            let select = format!("{pointer}/value/select/value");
            match &list.select {
                UiAbilitySelect::All => {}
                UiAbilitySelect::Ids(ids) => check_ids(ids.len(), &select)?,
                UiAbilitySelect::Tag(tag) => check_text(tag, &select)?,
            }
            match &list.sort {
                Some(UiSort::Unknown(text)) => {
                    check_unknown(text, &UI_SORTS, "sort", &format!("{pointer}/value/sort"))
                }
                _ => Ok(()),
            }
        }
    }
}

/// Read-time checks (B5, B6), called from `validate_local`.
///
/// The document is walked in pre-order and the first failure is returned.
pub(crate) fn validate_ui_local(document: &UiDocument) -> Result<(), DataError> {
    if let Some(stats) = &document.stats {
        check_list(stats.len(), MAX_UI_ENTRIES, false, "/stats")?;
        for (i, entry) in stats.iter().enumerate() {
            let base = format!("/stats/{i}");
            check_text(&entry.stat, &format!("{base}/stat"))?;
            check_text(&entry.label, &format!("{base}/label"))?;
            check_opt_text(&entry.short_label, &format!("{base}/short_label"))?;
            check_opt_text(&entry.description, &format!("{base}/description"))?;
            check_opt_text(&entry.group, &format!("{base}/group"))?;
            if let Some(format) = &entry.format {
                check_format(format, &format!("{base}/format"))?;
            }
        }
    }
    if let Some(pools) = &document.pools {
        check_list(pools.len(), MAX_UI_ENTRIES, false, "/pools")?;
        for (i, entry) in pools.iter().enumerate() {
            let base = format!("/pools/{i}");
            check_text(&entry.label, &format!("{base}/label"))?;
            check_opt_text(&entry.icon, &format!("{base}/icon"))?;
            if let Some(UiPoolStyle::Unknown(text)) = &entry.style {
                check_unknown(
                    text,
                    &UI_POOL_STYLES,
                    "pool style",
                    &format!("{base}/style"),
                )?;
            }
        }
    }
    if let Some(abilities) = &document.abilities {
        check_list(abilities.len(), MAX_UI_ENTRIES, false, "/abilities")?;
        for (i, entry) in abilities.iter().enumerate() {
            let base = format!("/abilities/{i}");
            check_text(&entry.label, &format!("{base}/label"))?;
            check_opt_text(&entry.description, &format!("{base}/description"))?;
            check_opt_text(&entry.icon, &format!("{base}/icon"))?;
            if let Some(tags) = &entry.category_tags {
                check_names(tags, &format!("{base}/category_tags"))?;
            }
            if let Some(UiTargetHint::Unknown(text)) = &entry.target_hint {
                check_unknown(
                    text,
                    &UI_TARGET_HINTS,
                    "target hint",
                    &format!("{base}/target_hint"),
                )?;
            }
        }
    }
    if let Some(effects) = &document.effects {
        check_list(effects.len(), MAX_UI_ENTRIES, false, "/effects")?;
        for (i, entry) in effects.iter().enumerate() {
            let base = format!("/effects/{i}");
            check_text(&entry.label, &format!("{base}/label"))?;
            check_opt_text(&entry.icon, &format!("{base}/icon"))?;
        }
    }
    if let Some(outcomes) = &document.outcomes {
        check_list(outcomes.len(), MAX_UI_ENTRIES, false, "/outcomes")?;
        for (i, entry) in outcomes.iter().enumerate() {
            let base = format!("/outcomes/{i}");
            check_text(&entry.outcome, &format!("{base}/outcome"))?;
            check_text(&entry.label, &format!("{base}/label"))?;
        }
    }
    if let Some(screens) = &document.screens {
        check_list(screens.len(), MAX_UI_SECTIONS, false, "/screens")?;
        for (i, screen) in screens.iter().enumerate() {
            let base = format!("/screens/{i}");
            if let UiScreenKind::Unknown(text) = &screen.kind {
                check_unknown(
                    text,
                    &UI_SCREEN_KINDS,
                    "screen kind",
                    &format!("{base}/kind"),
                )?;
            }
            let sections = format!("{base}/sections");
            check_list(screen.sections.len(), MAX_UI_SECTIONS, false, &sections)?;
            for (j, section) in screen.sections.iter().enumerate() {
                check_section(section, &format!("{sections}/{j}"))?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Defaults (U5).

/// Borrowed inputs for the defaults; the caller resolves them.
#[derive(Debug, Clone)]
pub struct UiSource<'a> {
    /// The ruleset the UI describes.
    pub ruleset: &'a Ruleset,
    /// `ruleset.abilities` that resolve to an `Ability`, in that order.
    pub abilities: Vec<&'a Ability>,
    /// Effects named by `abilities[*].effect` that resolve, first use, no repeats.
    pub effects: Vec<&'a Effect>,
    /// Tables named by `abilities[*].outcome_table` that resolve, first use, no repeats.
    pub tables: Vec<&'a OutcomeTable>,
}

impl<'a> UiSource<'a> {
    /// Resolves from loaded documents (B25); `None` when no `Ruleset` has that id.
    ///
    /// Documents are scanned in path order and the first match wins.
    pub fn from_documents(
        documents: &'a BTreeMap<SourcePath, Document>,
        ruleset: Ulid,
    ) -> Option<UiSource<'a>> {
        let ruleset = documents.values().find_map(|document| match document {
            Document::Ruleset(v) if v.id == ruleset => Some(v),
            _ => None,
        })?;
        let mut abilities = Vec::new();
        for id in &ruleset.abilities {
            let found = documents.values().find_map(|document| match document {
                Document::Ability(v) if v.id == *id => Some(v),
                _ => None,
            });
            abilities.extend(found);
        }
        let mut effects: Vec<&Effect> = Vec::new();
        let mut tables: Vec<&OutcomeTable> = Vec::new();
        for ability in &abilities {
            if let Some(id) = ability.effect {
                let found = documents.values().find_map(|document| match document {
                    Document::Effect(v) if v.id == id => Some(v),
                    _ => None,
                });
                if let Some(effect) = found {
                    if !effects.iter().any(|seen| seen.id == effect.id) {
                        effects.push(effect);
                    }
                }
            }
            let found = documents.values().find_map(|document| match document {
                Document::OutcomeTable(v) if v.id == ability.outcome_table => Some(v),
                _ => None,
            });
            if let Some(table) = found {
                if !tables.iter().any(|seen| seen.id == table.id) {
                    tables.push(table);
                }
            }
        }
        Some(UiSource {
            ruleset,
            abilities,
            effects,
            tables,
        })
    }

    /// Every outcome label the ruleset can produce, in B26 order.
    ///
    /// For each ability in order: its table's band outcomes, then the table's
    /// `Override` natural outcomes, then the ability's damage outcomes. Each
    /// label is kept on first occurrence only.
    pub fn outcome_labels(&self) -> Vec<String> {
        let mut labels: Vec<String> = Vec::new();
        let mut push = |outcome: OutcomeWire| {
            let text = ui_outcome_text(outcome);
            if !labels.contains(&text) {
                labels.push(text);
            }
        };
        for ability in &self.abilities {
            if let Some(table) = self
                .tables
                .iter()
                .find(|table| table.id == ability.outcome_table)
            {
                for band in &table.bands {
                    push(band.outcome);
                }
                for rule in &table.natural_rules {
                    if let NaturalEffectWire::Override(outcome) = rule.effect {
                        push(outcome);
                    }
                }
            }
            for entry in &ability.damage {
                push(entry.outcome);
            }
        }
        labels
    }
}

/// The UI label spelling of one outcome (B26).
pub fn ui_outcome_text(outcome: OutcomeWire) -> String {
    match outcome {
        OutcomeWire::CriticalSuccess => "critical_success".to_owned(),
        OutcomeWire::Success => "success".to_owned(),
        OutcomeWire::Failure => "failure".to_owned(),
        OutcomeWire::CriticalFailure => "critical_failure".to_owned(),
        OutcomeWire::Custom(n) => format!("custom:{n}"),
    }
}

/// Parses one UI outcome label (B18 grammar); `None` when it is not one.
///
/// The grammar is `critical_success`, `success`, `failure`, `critical_failure`
/// or `custom:<n>` with `<n>` a decimal `u8`, no sign and no leading zero.
pub fn ui_outcome(text: &str) -> Option<OutcomeWire> {
    match text {
        "critical_success" => Some(OutcomeWire::CriticalSuccess),
        "success" => Some(OutcomeWire::Success),
        "failure" => Some(OutcomeWire::Failure),
        "critical_failure" => Some(OutcomeWire::CriticalFailure),
        _ => {
            let digits = text.strip_prefix("custom:")?;
            let plain = !digits.is_empty()
                && digits.bytes().all(|b| b.is_ascii_digit())
                && (digits.len() == 1 || !digits.starts_with('0'));
            if !plain {
                return None;
            }
            digits.parse::<u8>().ok().map(OutcomeWire::Custom)
        }
    }
}

fn default_target_hint(ability: &Ability) -> UiTargetHint {
    if !ability.requires_target {
        UiTargetHint::NoTarget
    } else if ability.allow_self_target {
        UiTargetHint::Any
    } else {
        UiTargetHint::Enemy
    }
}

fn default_screens(source: &UiSource<'_>) -> Vec<UiScreen> {
    let screen = |kind: UiScreenKind, sections: Vec<UiSection>| UiScreen { kind, sections };
    let ability_list = |sort: Option<UiSort>| {
        UiSection::AbilityList(UiAbilityList {
            select: UiAbilitySelect::All,
            sort,
        })
    };
    vec![
        screen(
            UiScreenKind::CharacterSheet,
            vec![UiSection::StatGroup(UiStatSelect::All)],
        ),
        screen(UiScreenKind::ActionBar, vec![ability_list(None)]),
        screen(
            UiScreenKind::Spellbook,
            vec![ability_list(Some(UiSort::Label))],
        ),
        screen(
            UiScreenKind::Inventory,
            vec![UiSection::ItemList(UiItemSelect::All)],
        ),
        screen(
            UiScreenKind::CombatHud,
            vec![
                UiSection::StatGroup(UiStatSelect::Names(vec![source
                    .ruleset
                    .health_stat
                    .clone()])),
                UiSection::PoolGroup(UiPoolSelect::All),
                UiSection::Timeline,
                UiSection::Log,
            ],
        ),
    ]
}

/// U5: the complete default for a ruleset (B27-B29).
///
/// Pure and total: every list is `Some`, possibly empty, and no reserved or
/// `Unknown` kind is ever emitted.
pub fn generated_ui(source: &UiSource<'_>) -> UiDocument {
    let stats = source
        .ruleset
        .stats
        .iter()
        .map(|stat| UiStat {
            stat: stat.name.clone(),
            label: stat.name.clone(),
            short_label: None,
            description: None,
            group: None,
            format: None,
        })
        .collect();
    let pools = source
        .ruleset
        .pools
        .iter()
        .enumerate()
        .map(|(n, pool)| UiPool {
            pool: pool.id,
            label: format!("pool-{}", n + 1),
            icon: None,
            style: None,
            show_on_hud: None,
        })
        .collect();
    let abilities = source
        .abilities
        .iter()
        .map(|ability| UiAbility {
            ability: ability.id,
            label: ability.name.clone(),
            description: None,
            icon: None,
            category_tags: None,
            target_hint: Some(default_target_hint(ability)),
            show_odds: None,
        })
        .collect();
    let effects = source
        .effects
        .iter()
        .map(|effect| UiEffect {
            effect: effect.id,
            label: effect.name.clone(),
            icon: None,
            show_duration: None,
        })
        .collect();
    let outcomes = source
        .outcome_labels()
        .into_iter()
        .map(|text| UiOutcome {
            outcome: text.clone(),
            label: text,
        })
        .collect();
    UiDocument {
        ruleset: source.ruleset.id,
        stats: Some(stats),
        pools: Some(pools),
        abilities: Some(abilities),
        effects: Some(effects),
        outcomes: Some(outcomes),
        screens: Some(default_screens(source)),
        note: None,
    }
}

/// Authored entries first, unchanged and in order; then the generated entries
/// whose key no authored entry has.
fn merge_list<T: Clone, K: PartialEq>(
    authored: &Option<Vec<T>>,
    generated: Option<Vec<T>>,
    key: impl Fn(&T) -> K,
) -> Option<Vec<T>> {
    let Some(authored) = authored else {
        return generated;
    };
    let mut merged = authored.clone();
    for entry in generated.unwrap_or_default() {
        if !authored.iter().any(|have| key(have) == key(&entry)) {
            merged.push(entry);
        }
    }
    Some(merged)
}

/// The authored document with every omitted part filled from the default (B30).
///
/// With `None` it returns the generated default. With `Some`, `ruleset` comes
/// from the source, `note` is dropped, and no authored entry is judged,
/// reordered, filled in or dropped.
pub fn effective_ui(authored: Option<&UiDocument>, source: &UiSource<'_>) -> UiDocument {
    let generated = generated_ui(source);
    let Some(authored) = authored else {
        return generated;
    };
    UiDocument {
        ruleset: source.ruleset.id,
        stats: merge_list(&authored.stats, generated.stats, |e| e.stat.clone()),
        pools: merge_list(&authored.pools, generated.pools, |e| e.pool),
        abilities: merge_list(&authored.abilities, generated.abilities, |e| e.ability),
        effects: merge_list(&authored.effects, generated.effects, |e| e.effect),
        outcomes: merge_list(&authored.outcomes, generated.outcomes, |e| {
            e.outcome.clone()
        }),
        screens: merge_list(&authored.screens, generated.screens, |e| e.kind.clone()),
        note: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_indexes_match_their_spellings() {
        assert_eq!(UI_FORMAT_TYPES[FMT_INTEGER], "integer");
        assert_eq!(UI_FORMAT_TYPES[FMT_SIGNED], "signed");
        assert_eq!(UI_FORMAT_TYPES[FMT_FRACTION], "fraction");
        assert_eq!(UI_FORMAT_TYPES[FMT_PERCENT], "percent");
        assert_eq!(UI_FORMAT_TYPES[FMT_BOOLEAN], "boolean");
        assert_eq!(UI_FORMAT_TYPES[FMT_ENUM], "enum");
        assert_eq!(UI_FORMAT_TYPES[FMT_TAGS], "tags");
        assert_eq!(UI_FORMAT_TYPES[FMT_DICE], "dice");
        assert_eq!(UI_SECTION_TYPES[SEC_STAT_GROUP], "stat_group");
        assert_eq!(UI_SECTION_TYPES[SEC_POOL_GROUP], "pool_group");
        assert_eq!(UI_SECTION_TYPES[SEC_ABILITY_LIST], "ability_list");
        assert_eq!(UI_SECTION_TYPES[SEC_EFFECT_LIST], "effect_list");
        assert_eq!(UI_SECTION_TYPES[SEC_ITEM_LIST], "item_list");
        assert_eq!(UI_SECTION_TYPES[SEC_TIMELINE], "timeline");
        assert_eq!(UI_SECTION_TYPES[SEC_LOG], "log");
        assert_eq!(UI_SECTION_TYPES[SEC_CHOICE_STEPS], "choice_steps");
    }

    #[test]
    fn plain_string_variants_line_up_with_their_constants() {
        assert_eq!(UiScreenKind::CharacterSheet.as_str(), "character_sheet");
        assert_eq!(
            UiScreenKind::CharacterCreation.as_str(),
            "character_creation"
        );
        assert_eq!(UiPoolStyle::Number.as_str(), "number");
        assert_eq!(UiTargetHint::NoTarget.as_str(), "none");
        assert_eq!(UiTargetHint::Any.as_str(), "any");
        assert_eq!(UiSort::Label.as_str(), "label");
        assert_eq!(UiSort::Unknown("zig".into()).as_str(), "zig");
    }

    #[test]
    fn inventory_enumerates_every_ui_reference_site_in_order() {
        use crate::inventory::{reference_occurrences, RefPolicy};
        let id = Ulid::from_u128;
        let stat = |n: &str, format: Option<UiFormat>| UiStat {
            stat: n.into(),
            label: n.into(),
            short_label: None,
            description: None,
            group: None,
            format,
        };
        let pool = |n: u128| UiPool {
            pool: id(n),
            label: "l".into(),
            icon: None,
            style: None,
            show_on_hud: None,
        };
        let screen = |kind: UiScreenKind, sections: Vec<UiSection>| UiScreen { kind, sections };
        let ui = UiDocument {
            ruleset: id(1),
            stats: Some(vec![
                stat("a", Some(UiFormat::Fraction(UiMeter::Stat("m".into())))),
                stat("b", Some(UiFormat::Fraction(UiMeter::Pool(id(2))))),
                stat("c", Some(UiFormat::Dice)),
                stat("d", Some(UiFormat::Fraction(UiMeter::Pool(id(3))))),
            ]),
            pools: Some(vec![pool(4), pool(5)]),
            abilities: Some(vec![UiAbility {
                ability: id(6),
                label: "l".into(),
                description: None,
                icon: None,
                category_tags: None,
                target_hint: None,
                show_odds: None,
            }]),
            effects: Some(vec![UiEffect {
                effect: id(7),
                label: "l".into(),
                icon: None,
                show_duration: None,
            }]),
            outcomes: Some(vec![UiOutcome {
                outcome: "success".into(),
                label: "l".into(),
            }]),
            screens: Some(vec![
                screen(
                    UiScreenKind::LevelUp,
                    vec![
                        UiSection::PoolGroup(UiPoolSelect::Ids(vec![id(8), id(9)])),
                        UiSection::PoolGroup(UiPoolSelect::All),
                        UiSection::Unknown {
                            kind: "x".into(),
                            value: Some(serde_json::json!("0000000000000000000000000Z")),
                        },
                        UiSection::AbilityList(UiAbilityList {
                            select: UiAbilitySelect::Ids(vec![id(10)]),
                            sort: None,
                        }),
                    ],
                ),
                screen(
                    UiScreenKind::Unknown("j".into()),
                    vec![
                        UiSection::AbilityList(UiAbilityList {
                            select: UiAbilitySelect::Tag("t".into()),
                            sort: None,
                        }),
                        UiSection::StatGroup(UiStatSelect::Names(vec!["n".into()])),
                        UiSection::PoolGroup(UiPoolSelect::Ids(vec![id(11)])),
                    ],
                ),
            ]),
            note: None,
        };
        let path: SourcePath = "ui/x.json".parse().unwrap();
        let mut documents = BTreeMap::new();
        documents.insert(path.clone(), Document::Ui(ui));
        let found: Vec<String> = reference_occurrences(&documents)
            .into_iter()
            .map(|site| {
                assert_eq!(site.file, path);
                assert_eq!(site.source, None);
                let policy = match site.policy {
                    RefPolicy::Generic {
                        allowed,
                        owner: None,
                        what,
                    } => format!("generic {what} {allowed:?}"),
                    RefPolicy::PoolTemplate { what } => format!("pool {what}"),
                    _ => panic!("unexpected policy"),
                };
                format!("{} -> {} {policy}", site.pointer, site.target.to_u128())
            })
            .collect();
        assert_eq!(
            found,
            [
                "/ruleset -> 1 generic ui ruleset [Ruleset]",
                "/stats/1/format/value/value -> 2 pool ui pool",
                "/stats/3/format/value/value -> 3 pool ui pool",
                "/pools/0/pool -> 4 pool ui pool",
                "/pools/1/pool -> 5 pool ui pool",
                "/abilities/0/ability -> 6 generic ui ability [Ability]",
                "/effects/0/effect -> 7 generic ui effect [Effect]",
                "/screens/0/sections/0/value/value/0 -> 8 pool ui pool",
                "/screens/0/sections/0/value/value/1 -> 9 pool ui pool",
                "/screens/0/sections/3/value/select/value/0 -> 10 generic ui ability [Ability]",
                "/screens/1/sections/2/value/value/0 -> 11 pool ui pool",
            ]
        );
    }
}
