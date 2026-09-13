//! Typed entity and explicit aggregate documents with versioned envelopes.

use crate::{
    ActionCall, Bounds, DataError, DataValue, EventGraph, PackageId, PackageRequirement, Transform,
    VarDecl,
};
use crpg_core::{Fx16_16, Ulid};
use schemars::JsonSchema;
use semver::{Version, VersionReq};
use serde::{de, Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Campaign manifest and explicit entry placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Campaign {
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
    /// Immutable publication coordinate.
    pub package: PackageId,
    /// Published semantic version.
    #[schemars(with = "crate::schema::VersionText")]
    pub version: Version,
    /// Compatible engine version range.
    #[schemars(with = "crate::schema::RequirementText")]
    pub engine: VersionReq,
    /// Flat dependency requirements, retaining authored order.
    pub requires: Vec<PackageRequirement>,
    /// Starting world, area and placement.
    pub entry: EntryPoint,
}

/// Authored-object references defining the campaign entry point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntryPoint {
    /// Starting world identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub world: Ulid,
    /// Starting area identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub area: Ulid,
    /// Placement identity whose transform supplies the spawn point.
    #[schemars(with = "crate::schema::UlidText")]
    pub spawn: Ulid,
}

/// Authored world and its area references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct World {
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
    /// Area identities in authored order.
    #[schemars(with = "Vec<crate::schema::UlidText>")]
    pub areas: Vec<Ulid>,
    /// World variable declarations.
    pub variables: Vec<VarDecl>,
}

/// Authored area metadata, separate from placements and triggers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Area {
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
    /// Authored fixed-point bounds.
    pub bounds: Bounds,
    /// Adjacent area identities in authored order.
    #[schemars(with = "Vec<crate::schema::UlidText>")]
    pub neighbours: Vec<Ulid>,
    /// Optional asset key; explicit null is required when absent.
    #[serde(deserialize_with = "crate::types::required_nullable")]
    #[schemars(required, schema_with = "crate::schema::nullable_text")]
    pub ambience: Option<String>,
}

/// Authored creature prefab with symbolic stats and tags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Creature {
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
    /// Symbolic stat keys mapped to raw fixed-point values.
    #[schemars(with = "BTreeMap<String, i32>")]
    pub stats: BTreeMap<String, Fx16_16>,
    /// Symbolic tags in authored order.
    pub tags: Vec<String>,
    /// Faction reference; explicit null is required when absent.
    #[serde(deserialize_with = "crate::types::required_nullable")]
    #[schemars(required, schema_with = "crate::schema::nullable_ulid")]
    pub faction: Option<Ulid>,
    /// Item identities in authored order.
    #[schemars(with = "Vec<crate::schema::UlidText>")]
    pub inventory: Vec<Ulid>,
}

/// Authored item prefab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Item {
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
    /// Symbolic stat keys mapped to raw fixed-point values.
    #[schemars(with = "BTreeMap<String, i32>")]
    pub stats: BTreeMap<String, Fx16_16>,
    /// Symbolic tags in authored order.
    pub tags: Vec<String>,
}

/// Authored dialogue and identified nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Dialogue {
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
    /// Initial dialogue node identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub entry: Ulid,
    /// Identified nodes in authored order.
    pub nodes: Vec<DialogueNode>,
}

/// Identified dialogue entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DialogueNode {
    /// Stable node identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub id: Ulid,
    /// Dialogue operation.
    pub body: DialogueBody,
}

/// Dialogue operation, without presentation or execution semantics.
///
/// Deserialization rejects unknown fields for every variant, including the unit
/// `End` shape, because serde's derived `deny_unknown_fields` ignores trailing
/// fields on internally tagged unit variants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DialogueBody {
    /// NPC speech.
    NpcLine {
        /// Speaker object identity.
        #[schemars(with = "crate::schema::UlidText")]
        speaker: Ulid,
        /// Locale key for speech text.
        text_key: String,
        /// Opaque conditions in authored order.
        conditions: Vec<String>,
        /// Entry actions in authored order.
        on_enter: Vec<ActionCall>,
        /// Optional continuation; explicit null is required when absent.
        #[serde(deserialize_with = "crate::types::required_nullable")]
        #[schemars(required, schema_with = "crate::schema::nullable_ulid")]
        next: Option<Ulid>,
    },
    /// Player-selectable response.
    PlayerChoice {
        /// Locale key for choice text.
        text_key: String,
        /// Opaque conditions in authored order.
        conditions: Vec<String>,
        /// Selection actions in authored order.
        on_select: Vec<ActionCall>,
        /// Continuation identity.
        #[schemars(with = "crate::schema::UlidText")]
        next: Ulid,
    },
    /// Jump to a dialogue node.
    Jump {
        /// Target identity.
        #[schemars(with = "crate::schema::UlidText")]
        target: Ulid,
    },
    /// Link to another dialogue object.
    Link {
        /// Target identity.
        #[schemars(with = "crate::schema::UlidText")]
        target: Ulid,
    },
    /// End the dialogue.
    End,
}

/// Authored quest state machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Quest {
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
    /// Initial state identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub entry: Ulid,
    /// Identified states in authored order.
    pub states: Vec<QuestState>,
}

/// Identified quest state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QuestState {
    /// Stable state identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub id: Ulid,
    /// Locale key for the display name.
    pub name: String,
    /// Whether this is a terminal state.
    pub terminal: bool,
    /// Entry actions in authored order.
    pub on_enter: Vec<ActionCall>,
    /// Transitions in authored order.
    pub transitions: Vec<QuestTransition>,
}

/// Authored quest transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QuestTransition {
    /// Opaque condition expression.
    pub condition: String,
    /// Destination state identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub target: Ulid,
}

/// Authored faction relations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Faction {
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
    /// Relations in authored order.
    pub relations: Vec<FactionRelation>,
}

/// Authored relation to another faction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FactionRelation {
    /// Target faction identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub faction: Ulid,
    /// Rules-interpreted integer disposition.
    pub disposition: i32,
}

/// Identified placed prefab instance, stored only inside an aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Placement {
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
    /// Prefab object identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub prefab: Ulid,
    /// Authored fixed-point transform.
    pub transform: Transform,
    /// Named tagged overrides.
    pub overrides: BTreeMap<String, DataValue>,
}

/// Explicit area-owned placement aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlacementsDocument {
    /// Owning area identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub area: Ulid,
    /// Instances in authored order.
    pub placements: Vec<Placement>,
    /// Preserved author annotation.
    #[serde(rename = "_note", default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Explicit area-owned trigger graph aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TriggersDocument {
    /// Owning area identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub area: Ulid,
    /// Graphs in authored order.
    pub graphs: Vec<EventGraph>,
    /// Preserved author annotation.
    #[serde(rename = "_note", default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Explicit campaign-owned localized string table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocaleDocument {
    /// Owning campaign identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub campaign: Ulid,
    /// Locale matching the file stem.
    pub locale: String,
    /// Locale keys mapped to display text.
    pub strings: BTreeMap<String, String>,
    /// Preserved author annotation.
    #[serde(rename = "_note", default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Explicit campaign-owned variable declaration aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariablesDocument {
    /// Owning campaign identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub campaign: Ulid,
    /// Declarations in authored order.
    pub variables: Vec<VarDecl>,
    /// Preserved author annotation.
    #[serde(rename = "_note", default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Complete version-1 persistence envelope; fields share the schema-tag object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "schema", deny_unknown_fields)]
pub enum Document {
    /// Campaign manifest.
    #[serde(rename = "crpg.campaign/1")]
    Campaign(Campaign),
    /// World entity.
    #[serde(rename = "crpg.world/1")]
    World(World),
    /// Area entity.
    #[serde(rename = "crpg.area/1")]
    Area(Area),
    /// Creature prefab.
    #[serde(rename = "crpg.creature/1")]
    Creature(Creature),
    /// Item prefab.
    #[serde(rename = "crpg.item/1")]
    Item(Item),
    /// Dialogue entity.
    #[serde(rename = "crpg.dialogue/1")]
    Dialogue(Dialogue),
    /// Quest entity.
    #[serde(rename = "crpg.quest/1")]
    Quest(Quest),
    /// Faction entity.
    #[serde(rename = "crpg.faction/1")]
    Faction(Faction),
    /// Independently stored event graph.
    #[serde(rename = "crpg.graph/1")]
    Graph(EventGraph),
    /// Area placement aggregate.
    #[serde(rename = "crpg.placements/1")]
    Placements(PlacementsDocument),
    /// Area trigger aggregate.
    #[serde(rename = "crpg.triggers/1")]
    Triggers(TriggersDocument),
    /// Campaign locale table.
    #[serde(rename = "crpg.locale/1")]
    Locale(LocaleDocument),
    /// Campaign variable declarations.
    #[serde(rename = "crpg.variables/1")]
    Variables(VariablesDocument),
    /// Package resolution and assets-lock digest authority.
    #[serde(rename = "crpg.campaign-lock/1")]
    CampaignLock(crate::CampaignLock),
    /// Source-asset hash and import-settings authority.
    #[serde(rename = "crpg.assets-lock/1")]
    AssetsLock(crate::AssetsLock),
}

pub(crate) const SCHEMA_IDS: [&str; 15] = [
    "crpg.campaign/1",
    "crpg.world/1",
    "crpg.area/1",
    "crpg.creature/1",
    "crpg.item/1",
    "crpg.dialogue/1",
    "crpg.quest/1",
    "crpg.faction/1",
    "crpg.graph/1",
    "crpg.placements/1",
    "crpg.triggers/1",
    "crpg.locale/1",
    "crpg.variables/1",
    "crpg.campaign-lock/1",
    "crpg.assets-lock/1",
];

/// Reads syntax, envelope, supported schema, typed fields, then local invariants.
pub fn read_document(bytes: &[u8]) -> Result<Document, DataError> {
    let value = crate::canonical::parse(bytes)?;
    let schema = value
        .as_object()
        .and_then(|o| o.get("schema"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| crate::error::malformed("expected object with string schema"))?;
    if !SCHEMA_IDS.contains(&schema) {
        return Err(DataError::UnsupportedSchema {
            path: None,
            found: schema.into(),
        });
    }
    let document = serde_json::from_value(value).map_err(crate::error::malformed)?;
    validate_local(&document)?;
    Ok(document)
}

/// Checks local invariants and writes a complete canonical schema envelope.
pub fn write_document(document: &Document) -> Result<Vec<u8>, DataError> {
    validate_local(document)?;
    crate::canonical_json(document)
}

pub(crate) fn validate_local(document: &Document) -> Result<(), DataError> {
    match document {
        Document::CampaignLock(lock) => crate::package::validate_campaign_lock(lock),
        Document::AssetsLock(lock) => crate::package::validate_assets_lock(lock),
        _ => Ok(()),
    }
}

fn exact_body_keys<'de, D: serde::Deserializer<'de>>(
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

fn body_field<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(
    map: &BTreeMap<String, Value>,
    name: &'static str,
) -> Result<T, D::Error> {
    map.get(name)
        .cloned()
        .ok_or_else(|| de::Error::missing_field(name))
        .and_then(|value| serde_json::from_value(value).map_err(de::Error::custom))
}

impl<'de> Deserialize<'de> for DialogueBody {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        let kind = map
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| de::Error::missing_field("kind"))?;
        match kind {
            "npc_line" => {
                exact_body_keys::<D>(
                    &map,
                    &[
                        "kind",
                        "speaker",
                        "text_key",
                        "conditions",
                        "on_enter",
                        "next",
                    ],
                )?;
                let conditions = body_field::<D, Vec<String>>(&map, "conditions")?;
                let next = body_field::<D, Option<Ulid>>(&map, "next")?;
                let on_enter = body_field::<D, Vec<ActionCall>>(&map, "on_enter")?;
                let speaker = body_field::<D, Ulid>(&map, "speaker")?;
                let text_key = body_field::<D, String>(&map, "text_key")?;
                Ok(Self::NpcLine {
                    speaker,
                    text_key,
                    conditions,
                    on_enter,
                    next,
                })
            }
            "player_choice" => {
                exact_body_keys::<D>(
                    &map,
                    &["kind", "text_key", "conditions", "on_select", "next"],
                )?;
                let conditions = body_field::<D, Vec<String>>(&map, "conditions")?;
                let next = body_field::<D, Ulid>(&map, "next")?;
                let on_select = body_field::<D, Vec<ActionCall>>(&map, "on_select")?;
                let text_key = body_field::<D, String>(&map, "text_key")?;
                Ok(Self::PlayerChoice {
                    text_key,
                    conditions,
                    on_select,
                    next,
                })
            }
            "jump" => {
                exact_body_keys::<D>(&map, &["kind", "target"])?;
                Ok(Self::Jump {
                    target: body_field::<D, Ulid>(&map, "target")?,
                })
            }
            "link" => {
                exact_body_keys::<D>(&map, &["kind", "target"])?;
                Ok(Self::Link {
                    target: body_field::<D, Ulid>(&map, "target")?,
                })
            }
            "end" => {
                exact_body_keys::<D>(&map, &["kind"])?;
                Ok(Self::End)
            }
            other => Err(de::Error::unknown_variant(
                other,
                &["npc_line", "player_choice", "jump", "link", "end"],
            )),
        }
    }
}
