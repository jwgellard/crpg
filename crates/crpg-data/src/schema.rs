//! Self-contained draft-2020-12 schemas derived from the typed wire shapes.

use crate::*;
use schemars::{generate::SchemaSettings, JsonSchema, Schema, SchemaGenerator};
use serde_json::json;
use std::{borrow::Cow, collections::BTreeMap};

macro_rules! string_adapter {
    ($name:ident, $description:literal, $pattern:literal) => {
        pub(crate) enum $name {}
        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> { stringify!($name).into() }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                Schema::try_from(json!({"type": "string", "description": $description, "pattern": $pattern})).expect("object schema")
            }
        }
    };
}

string_adapter!(UlidText, "Core ULID: case-insensitive Crockford base32, I/L aliases for 1 and O for 0; first decoded digit at most 7.", "^[0-7IiLlOo][0-9A-Ta-tV-Zv-z]{25}$");
string_adapter!(
    PackageText,
    "Immutable package publication coordinate.",
    "^[a-z0-9]+(?:[.-][a-z0-9]+)*$"
);
string_adapter!(
    PathText,
    "Portable campaign-relative logical path; dot and dot-dot components are forbidden.",
    "^(?!\\.{1,2}(?:/|$))[A-Za-z0-9._-]+(?:/(?!\\.{1,2}(?:/|$))[A-Za-z0-9._-]+)*$"
);
string_adapter!(
    DigestText,
    "BLAKE3 content digest as lowercase hexadecimal.",
    "^[0-9a-f]{64}$"
);

macro_rules! semver_adapter {
    ($name:ident, $description:literal) => {
        pub(crate) enum $name {}
        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> { stringify!($name).into() }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                Schema::try_from(json!({"type": "string", "description": $description})).expect("object schema")
            }
        }
    };
}
semver_adapter!(
    VersionText,
    "Semantic version string; semver 1.x parsing is authoritative."
);
semver_adapter!(RequirementText, "Semantic version requirement string; semver 1.x parsing and prerelease matching are authoritative.");

// `required` alone asks schemars to strip Option nullability. Explicit schema
// functions preserve null while the annotation keeps the field in required.
pub(crate) fn nullable_text(generator: &mut SchemaGenerator) -> Schema {
    <Option<String>>::json_schema(generator)
}

pub(crate) fn nullable_ulid(generator: &mut SchemaGenerator) -> Schema {
    <Option<UlidText>>::json_schema(generator)
}

fn integer_bounds(schema: &mut Schema) {
    // JSON Schema formats are annotations, not numeric acceptance constraints.
    // Translate schemars' derived integer formats to exact representable bounds.
    let bounds = match schema.get("format").and_then(|v| v.as_str()) {
        Some("int32") => Some((json!(i32::MIN), json!(i32::MAX))),
        Some("int64") => Some((json!(i64::MIN), json!(i64::MAX))),
        Some("uint32") => Some((json!(0), json!(u32::MAX))),
        Some("uint64") => Some((json!(0), json!(u64::MAX))),
        _ => None,
    };
    if let Some((min, max)) = bounds {
        schema.insert("minimum".into(), min);
        schema.insert("maximum".into(), max);
    }
    schemars::transform::transform_subschemas(&mut integer_bounds, schema);
}

fn root<T: JsonSchema>(title: &str) -> Result<Vec<u8>, DataError> {
    let mut schema = SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<T>();
    schema.insert("title".into(), title.into());
    integer_bounds(&mut schema);
    canonical_json(&schema)
}

/// Generates exactly 17 filename-to-canonical-byte schemas without filesystem I/O.
///
/// Document roots use single-variant derive wrappers to enforce one schema tag.
/// Each file has local definitions and no external schema identifier.
pub fn generated_schemas() -> Result<BTreeMap<String, Vec<u8>>, DataError> {
    let mut output = BTreeMap::new();
    macro_rules! document_schema {
        ($payload:ty, $tag:literal, $filename:literal) => {{
            #[derive(serde::Serialize, serde::Deserialize, JsonSchema)]
            #[serde(tag = "schema", deny_unknown_fields)]
            enum Envelope {
                #[serde(rename = $tag)]
                Payload($payload),
            }
            output.insert($filename.into(), root::<Envelope>(stringify!($payload))?);
        }};
    }
    document_schema!(Campaign, "crpg.campaign/1", "campaign.schema.json");
    document_schema!(World, "crpg.world/1", "world.schema.json");
    document_schema!(Area, "crpg.area/1", "area.schema.json");
    document_schema!(Creature, "crpg.creature/1", "creature.schema.json");
    document_schema!(Item, "crpg.item/2", "item.schema.json");
    document_schema!(Dialogue, "crpg.dialogue/1", "dialogue.schema.json");
    document_schema!(Quest, "crpg.quest/1", "quest.schema.json");
    document_schema!(Faction, "crpg.faction/1", "faction.schema.json");
    document_schema!(EventGraph, "crpg.graph/1", "graph.schema.json");
    document_schema!(
        PlacementsDocument,
        "crpg.placements/1",
        "placements.schema.json"
    );
    document_schema!(TriggersDocument, "crpg.triggers/1", "triggers.schema.json");
    document_schema!(LocaleDocument, "crpg.locale/1", "locale.schema.json");
    document_schema!(
        VariablesDocument,
        "crpg.variables/1",
        "variables.schema.json"
    );
    document_schema!(
        CampaignLock,
        "crpg.campaign-lock/1",
        "campaign-lock.schema.json"
    );
    document_schema!(AssetsLock, "crpg.assets-lock/1", "assets-lock.schema.json");
    output.insert(
        "placement.schema.json".into(),
        root::<Placement>("Placement")?,
    );
    output.insert(
        "action-signature.schema.json".into(),
        root::<ActionSignature>("ActionSignature")?,
    );
    Ok(output)
}
