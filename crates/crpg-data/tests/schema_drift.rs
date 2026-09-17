#![forbid(unsafe_code)]
use crpg_data::generated_schemas;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

const NAMES: [&str; 17] = [
    "action-signature",
    "area",
    "assets-lock",
    "campaign-lock",
    "campaign",
    "creature",
    "dialogue",
    "faction",
    "graph",
    "item",
    "locale",
    "placement",
    "placements",
    "quest",
    "triggers",
    "variables",
    "world",
];

fn compare(
    generated: &BTreeMap<String, Vec<u8>>,
    baseline: &BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    if generated.keys().collect::<BTreeSet<_>>() != baseline.keys().collect::<BTreeSet<_>>() {
        return Err("schema filename set differs".into());
    }
    for (name, bytes) in generated {
        if baseline[name] != *bytes {
            return Err(format!("schema bytes differ: {name}"));
        }
    }
    Ok(())
}

#[test]
fn schema_drift_exact_filename_set_and_bytes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
    let mut baseline = BTreeMap::new();
    for entry in fs::read_dir(root).expect("required schema directory") {
        let entry = entry.unwrap();
        let name = entry
            .file_name()
            .into_string()
            .expect("UTF-8 schema filename");
        if name.ends_with(".schema.json") {
            baseline.insert(name, fs::read(entry.path()).expect("required schema bytes"));
        }
    }
    let generated = generated_schemas().unwrap();
    let expected: BTreeSet<_> = NAMES
        .into_iter()
        .map(|name| format!("{name}.schema.json"))
        .collect();
    assert_eq!(generated.keys().cloned().collect::<BTreeSet<_>>(), expected);
    compare(&generated, &baseline).unwrap();
}

#[test]
fn comparison_fails_for_missing_extra_changed_and_crlf_bytes() {
    let generated = generated_schemas().unwrap();
    for defect in 0..4 {
        let mut baseline = generated.clone();
        match defect {
            0 => {
                baseline.remove("area.schema.json");
            }
            1 => {
                baseline.insert("extra.schema.json".into(), b"{}\n".to_vec());
            }
            2 => {
                baseline.get_mut("area.schema.json").unwrap().push(b' ');
            }
            3 => {
                let bytes = baseline.get_mut("area.schema.json").unwrap();
                *bytes = bytes
                    .iter()
                    .flat_map(|b| {
                        if *b == b'\n' {
                            vec![b'\r', b'\n']
                        } else {
                            vec![*b]
                        }
                    })
                    .collect();
            }
            _ => unreachable!(),
        }
        assert!(compare(&generated, &baseline).is_err());
    }
}

fn walk(value: &Value, root: &Value) {
    match value {
        Value::Object(map) => {
            assert!(!map.contains_key("$id"));
            if let Some(reference) = map.get("$ref") {
                let reference = reference.as_str().unwrap();
                assert!(reference.starts_with("#/$defs/"), "{reference}");
                assert!(
                    root.pointer(&reference[1..]).is_some(),
                    "unresolved local reference {reference}"
                );
            }
            if let Some(format) = map.get("format").and_then(Value::as_str) {
                let expected = match format {
                    "int32" => Some((json!(i32::MIN), json!(i32::MAX))),
                    "int64" => Some((json!(i64::MIN), json!(i64::MAX))),
                    "uint32" => Some((json!(0), json!(u32::MAX))),
                    "uint64" => Some((json!(0), json!(u64::MAX))),
                    _ => None,
                };
                if let Some((min, max)) = expected {
                    assert_eq!(map["type"], "integer");
                    assert_eq!(map["minimum"], min);
                    assert_eq!(map["maximum"], max);
                }
            }
            for value in map.values() {
                walk(value, root);
            }
        }
        Value::Array(values) => {
            for value in values {
                walk(value, root);
            }
        }
        _ => {}
    }
}

#[test]
fn roots_are_single_tagged_closed_shapes_and_self_contained() {
    let schemas = generated_schemas().unwrap();
    for name in NAMES {
        let bytes = &schemas[&format!("{name}.schema.json")];
        assert!(bytes.ends_with(b"\n"));
        assert!(!bytes.ends_with(b"\n\n"));
        assert!(!bytes.contains(&b'\r'));
        assert!(!bytes.starts_with(&[239, 187, 191]));
        let root: Value = serde_json::from_slice(bytes).unwrap();
        assert_eq!(
            root["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        walk(&root, &root);
        let embedded = name == "placement" || name == "action-signature";
        let shape = if embedded {
            &root
        } else {
            assert_eq!(root["oneOf"].as_array().unwrap().len(), 1);
            &root["oneOf"][0]
        };
        assert_eq!(shape["type"], "object");
        assert_eq!(shape["additionalProperties"], false);
        let required = shape["required"].as_array().unwrap();
        if embedded {
            assert!(shape["properties"].get("schema").is_none());
        } else {
            let expected = if name == "item" {
                "crpg.item/2".to_string()
            } else {
                format!("crpg.{name}/1")
            };
            assert_eq!(shape["properties"]["schema"]["const"], expected);
            assert!(required.contains(&json!("schema")));
        }
        assert!(!required.contains(&json!("_note")));
        for key in shape["properties"]
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| k.as_str() != "_note")
        {
            assert!(required.contains(&json!(key)), "{name}.{key}");
        }
        let title = match name {
            "campaign" => "Campaign",
            "world" => "World",
            "area" => "Area",
            "creature" => "Creature",
            "item" => "Item",
            "dialogue" => "Dialogue",
            "quest" => "Quest",
            "faction" => "Faction",
            "graph" => "EventGraph",
            "placements" => "PlacementsDocument",
            "triggers" => "TriggersDocument",
            "locale" => "LocaleDocument",
            "variables" => "VariablesDocument",
            "campaign-lock" => "CampaignLock",
            "assets-lock" => "AssetsLock",
            "placement" => "Placement",
            "action-signature" => "ActionSignature",
            _ => unreachable!(),
        };
        assert_eq!(root["title"], title);
    }
}

#[test]
fn core_adapters_nullable_fields_and_embedded_shapes() {
    let schemas = generated_schemas().unwrap();
    let read = |name: &str| {
        serde_json::from_slice::<Value>(&schemas[&format!("{name}.schema.json")]).unwrap()
    };
    let creature = read("creature");
    let shape = &creature["oneOf"][0];
    assert_eq!(
        shape["properties"]["stats"]["additionalProperties"]["minimum"],
        json!(i32::MIN)
    );
    assert_eq!(
        shape["properties"]["stats"]["additionalProperties"]["maximum"],
        json!(i32::MAX)
    );
    assert_eq!(
        creature["$defs"]["UlidText"]["pattern"],
        "^[0-7IiLlOo][0-9A-Ta-tV-Zv-z]{25}$"
    );
    assert_eq!(creature["$defs"]["UlidText"]["type"], "string");
    assert!(shape["properties"]["faction"]["anyOf"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["type"] == "null"));
    let area = read("area");
    assert!(area["oneOf"][0]["properties"]["ambience"]["type"]
        .as_array()
        .unwrap()
        .contains(&json!("null")));
    let placement = read("placement");
    assert!(placement["properties"].get("prefab").is_some());
    assert!(placement["properties"].get("transform").is_some());
    let signature = read("action-signature");
    assert!(signature["properties"].get("parameters").is_some());
    assert_eq!(
        signature["$defs"]["ActionParameter"]["additionalProperties"],
        false
    );
    let campaign = read("campaign");
    assert_eq!(campaign["$defs"]["VersionText"]["type"], "string");
    assert_eq!(campaign["$defs"]["RequirementText"]["type"], "string");
    assert!(campaign["$defs"]["RequirementText"]["description"]
        .as_str()
        .unwrap()
        .contains("semver"));

    let graph = read("graph");
    let tagged = |definition: &str, tag: &str| {
        graph["$defs"][definition]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|variant| {
                variant["properties"]
                    .as_object()
                    .unwrap()
                    .values()
                    .any(|property| property["const"] == tag)
            })
            .unwrap()
    };
    for (schema, format, min, max) in [
        (
            &tagged("DataValue", "integer")["properties"]["value"],
            "int64",
            json!(i64::MIN),
            json!(i64::MAX),
        ),
        (
            &tagged("DataValue", "unsigned")["properties"]["value"],
            "uint64",
            json!(0),
            json!(u64::MAX),
        ),
        (
            &tagged("DataValue", "fixed")["properties"]["value"],
            "int32",
            json!(i32::MIN),
            json!(i32::MAX),
        ),
        (
            &tagged("Port", "case")["properties"]["index"],
            "uint32",
            json!(0),
            json!(u32::MAX),
        ),
    ] {
        assert_eq!(schema["type"], "integer");
        assert_eq!(schema["format"], format);
        assert_eq!(schema["minimum"], min);
        assert_eq!(schema["maximum"], max);
    }
}
