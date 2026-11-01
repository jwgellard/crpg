#![forbid(unsafe_code)]
//! T139d: optional creature-lifecycle fields on `crpg.ruleset/2` and
//! `crpg.encounter/1` (L1-L8). The fields are hooks only: no rule reads them
//! here, no version moves, and every existing document is unchanged.
use crpg_data::{
    CreaturePersistenceWire, DataError, Document, EffectsOnExitWire, LateJoinWire, LoadedCampaign,
    SourcePath,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const RULESET_KEYS: [&str; 3] = ["creature_persistence", "late_join", "effects_on_exit"];
const ENCOUNTER_KEY: &str = "respawns";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn json_file(relative: &str) -> Value {
    let bytes = std::fs::read(repo_root().join(relative)).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn srd_ruleset() -> Value {
    json_file("rulesets/srd-lite/ruleset.json")
}

fn srd_encounter() -> Value {
    json_file("campaigns/fixtures/combat_srd/encounters/first-blood.json")
}

fn with(base: &Value, key: &str, value: Value) -> Value {
    let mut out = base.clone();
    out[key] = value;
    out
}

fn read(value: &Value) -> Result<Document, DataError> {
    crpg_data::read_document(&serde_json::to_vec(value).unwrap())
}

fn read_ok(value: &Value) -> Document {
    read(value).unwrap_or_else(|err| panic!("document should read: {err}"))
}

fn message(result: Result<Document, DataError>) -> String {
    match result {
        Err(DataError::Malformed { message, .. }) => message,
        other => panic!("expected Malformed, got {other:?}"),
    }
}

fn ruleset_of(doc: Document) -> crpg_data::Ruleset {
    match doc {
        Document::Ruleset(ruleset) => ruleset,
        other => panic!("expected ruleset, got {other:?}"),
    }
}

fn encounter_of(doc: Document) -> crpg_data::Encounter {
    match doc {
        Document::Encounter(encounter) => encounter,
        other => panic!("expected encounter, got {other:?}"),
    }
}

fn written(doc: &Document) -> String {
    String::from_utf8(crpg_data::write_document(doc).unwrap()).unwrap()
}

/// Byte offset of a top-level key (two-space indent) in canonical output.
fn top_key(text: &str, key: &str) -> usize {
    text.find(&format!("\n  \"{key}\":"))
        .unwrap_or_else(|| panic!("canonical output lacks top-level {key}"))
}

// L2 (B-D1, B-D2): every enum value and both booleans round trip, and the
// canonical writer places each key where the sort puts it.
#[test]
fn every_value_round_trips_through_the_canonical_writer() {
    let base = srd_ruleset();
    for (key, values) in [
        (
            "creature_persistence",
            ["survivors", "registered"].as_slice(),
        ),
        ("late_join", ["next_round", "after_active"].as_slice()),
        (
            "effects_on_exit",
            ["suspend", "strip", "strip_at_release"].as_slice(),
        ),
    ] {
        for value in values {
            let ruleset = ruleset_of(read_ok(&with(&base, key, json!(value))));
            let doc = Document::Ruleset(ruleset.clone());
            match key {
                "creature_persistence" => assert_eq!(
                    ruleset
                        .creature_persistence
                        .map(|v| serde_json::to_value(v).unwrap()),
                    Some(json!(value))
                ),
                "late_join" => assert_eq!(
                    ruleset.late_join.map(|v| serde_json::to_value(v).unwrap()),
                    Some(json!(value))
                ),
                _ => assert_eq!(
                    ruleset
                        .effects_on_exit
                        .map(|v| serde_json::to_value(v).unwrap()),
                    Some(json!(value))
                ),
            }
            let text = written(&doc);
            assert!(
                text.contains(&format!("\"{key}\": \"{value}\"")),
                "{key}={value}: {text}"
            );
            assert_eq!(crpg_data::read_document(text.as_bytes()).unwrap(), doc);
        }
    }

    // All three together: the sort places each key among the existing ones.
    let mut all = base.clone();
    all["creature_persistence"] = json!("registered");
    all["late_join"] = json!("after_active");
    all["effects_on_exit"] = json!("strip_at_release");
    let doc = read_ok(&all);
    let text = written(&doc);
    assert!(top_key(&text, "attributes") < top_key(&text, "creature_persistence"));
    assert!(top_key(&text, "creature_persistence") < top_key(&text, "effects_on_exit"));
    assert!(top_key(&text, "effects_on_exit") < top_key(&text, "health_stat"));
    assert!(top_key(&text, "late_join") < top_key(&text, "name"));
    assert!(top_key(&text, "health_stat") < top_key(&text, "late_join"));
    assert_eq!(crpg_data::read_document(text.as_bytes()).unwrap(), doc);

    let base = srd_encounter();
    for respawns in [true, false] {
        let encounter = encounter_of(read_ok(&with(&base, ENCOUNTER_KEY, json!(respawns))));
        assert_eq!(encounter.respawns, Some(respawns));
        let doc = Document::Encounter(encounter);
        let text = written(&doc);
        // `false` is kept, not dropped.
        assert!(
            text.contains(&format!("\"respawns\": {respawns}")),
            "{text}"
        );
        assert!(top_key(&text, "participants") < top_key(&text, "respawns"));
        assert!(top_key(&text, "respawns") < top_key(&text, "ruleset"));
        assert_eq!(crpg_data::read_document(text.as_bytes()).unwrap(), doc);
    }
}

fn existing_documents() -> Vec<(String, Value)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "json") {
                out.push(path);
            }
        }
    }
    let mut paths = Vec::new();
    for dir in [
        "rulesets/minimal-d6",
        "rulesets/srd-lite",
        "campaigns/fixtures/combat_basic",
        "campaigns/fixtures/combat_srd",
    ] {
        walk(&repo_root().join(dir), &mut paths);
    }
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let tag = value.get("schema").and_then(Value::as_str).unwrap_or("");
        if tag.starts_with("crpg.ruleset/") || tag.starts_with("crpg.encounter/") {
            out.push((path.display().to_string(), value));
        }
    }
    out
}

// L3 (B-D2, B-D4): every existing ruleset and encounter reads, migrating where
// needed, with all four fields absent, and writes none of the four keys.
#[test]
fn existing_documents_have_all_fields_absent() {
    let documents = existing_documents();
    let mut rulesets = 0;
    let mut encounters = 0;
    let mut migrated_from_v1 = 0;
    for (path, value) in documents {
        if value["schema"].as_str().unwrap().ends_with("/1") && path.contains("ruleset") {
            migrated_from_v1 += 1;
        }
        let doc = read_ok(&value);
        match &doc {
            Document::Ruleset(ruleset) => {
                rulesets += 1;
                assert_eq!(ruleset.creature_persistence, None, "{path}");
                assert_eq!(ruleset.late_join, None, "{path}");
                assert_eq!(ruleset.effects_on_exit, None, "{path}");
            }
            Document::Encounter(encounter) => {
                encounters += 1;
                assert_eq!(encounter.respawns, None, "{path}");
            }
            other => panic!("{path}: unexpected {other:?}"),
        }
        let text = written(&doc);
        for key in RULESET_KEYS.iter().chain([&ENCOUNTER_KEY]) {
            assert!(!text.contains(key), "{path} writes {key}");
        }
    }
    // minimal-d6 + srd-lite + both campaign copies; two encounters.
    assert!(rulesets >= 4, "found {rulesets} rulesets");
    assert!(encounters >= 2, "found {encounters} encounters");
    assert!(
        migrated_from_v1 > 0,
        "a /1 ruleset source must be exercised"
    );
}

const MESSAGES: [(&str, &str, &str); 3] = [
    (
        "creature_persistence",
        "creature_persistence must be a policy name",
        "survivor",
    ),
    ("late_join", "late_join must be a policy name", "after_head"),
    (
        "effects_on_exit",
        "effects_on_exit must be a policy name",
        "strip_on_release",
    ),
];

// L4 (B-D1): wrong forms fail with the exact message; unknown names list every
// variant; respawns takes only a boolean.
#[test]
fn wrong_forms_and_unknown_names_are_rejected() {
    let base = srd_ruleset();
    for (key, expected, unknown) in MESSAGES {
        for bad in [json!({"type": "survivors"}), json!(1), json!(true)] {
            let got = message(read(&with(&base, key, bad.clone())));
            assert_eq!(got, expected, "{key}={bad}");
        }
        let got = message(read(&with(&base, key, json!(unknown))));
        assert!(got.contains("unknown variant"), "{key}: {got}");
        assert!(got.contains(&format!("`{unknown}`")), "{key}: {got}");
        let listed: &[&str] = match key {
            "creature_persistence" => &["survivors", "registered"],
            "late_join" => &["next_round", "after_active"],
            _ => &["suspend", "strip", "strip_at_release"],
        };
        for variant in listed {
            assert!(got.contains(&format!("`{variant}`")), "{key}: {got}");
        }
    }
    let encounter = srd_encounter();
    for bad in [json!(1), json!("true")] {
        assert!(
            matches!(
                read(&with(&encounter, ENCOUNTER_KEY, bad.clone())),
                Err(DataError::Malformed { .. })
            ),
            "respawns accepted {bad}"
        );
    }
}

// L5 (B-D1, B-D2): null reads as absent and is not written back.
#[test]
fn null_reads_as_absent_and_is_omitted_on_write() {
    let mut value = srd_ruleset();
    for key in RULESET_KEYS {
        value[key] = Value::Null;
    }
    let ruleset = ruleset_of(read_ok(&value));
    assert_eq!(ruleset.creature_persistence, None);
    assert_eq!(ruleset.late_join, None);
    assert_eq!(ruleset.effects_on_exit, None);
    let text = written(&Document::Ruleset(ruleset));
    for key in RULESET_KEYS {
        assert!(!text.contains(key), "{key} written");
    }

    let encounter = encounter_of(read_ok(&with(&srd_encounter(), ENCOUNTER_KEY, Value::Null)));
    assert_eq!(encounter.respawns, None);
    assert!(!written(&Document::Encounter(encounter)).contains(ENCOUNTER_KEY));
}

// L6 (B-D1): near-miss keys are unknown fields.
#[test]
fn near_miss_keys_are_rejected_as_unknown_fields() {
    for (base, key) in [
        (srd_encounter(), "respawn"),
        (srd_ruleset(), "creature_persistance"),
        (srd_ruleset(), "late_joins"),
        (srd_ruleset(), "effect_on_exit"),
    ] {
        let got = message(read(&with(&base, key, json!("suspend"))));
        assert!(got.contains("unknown field"), "{key}: {got}");
    }
}

fn srd_files() -> BTreeMap<SourcePath, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<SourcePath, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path
                .file_name()
                .is_some_and(|name| name != ".gitattributes")
            {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(relative.parse().unwrap(), std::fs::read(&path).unwrap());
            }
        }
    }
    let root = repo_root().join("campaigns/fixtures/combat_srd");
    let mut out = BTreeMap::new();
    walk(&root, &root, &mut out);
    out
}

fn validate(
    mutate: impl FnOnce(&mut crpg_data::Ruleset, &mut crpg_data::Encounter),
) -> Vec<crpg_data::Diagnostic> {
    let engine: semver::Version = "0.1.0".parse().unwrap();
    let loaded = crpg_data::load_campaign(&srd_files(), &engine).expect("combat_srd loads");
    let mut docs = loaded.documents.clone();
    let mut ruleset = None;
    let mut encounter = None;
    for doc in docs.values_mut() {
        match doc {
            Document::Ruleset(value) => ruleset = Some(value),
            Document::Encounter(value) => encounter = Some(value),
            _ => {}
        }
    }
    mutate(ruleset.unwrap(), encounter.unwrap());
    crpg_data::validate(&LoadedCampaign {
        documents: docs,
        index: BTreeMap::new(),
    })
}

// L7 (B-D3): no semantic rule or diagnostic exists for any value of the four.
#[test]
fn validation_adds_no_diagnostic_for_any_value() {
    assert_eq!(validate(|_, _| {}), Vec::new());
    let findings = validate(|ruleset, encounter| {
        ruleset.creature_persistence = Some(CreaturePersistenceWire::Registered);
        ruleset.late_join = Some(LateJoinWire::AfterActive);
        ruleset.effects_on_exit = Some(EffectsOnExitWire::StripAtRelease);
        encounter.respawns = Some(true);
    });
    assert_eq!(findings, Vec::new());
}

// L8 (B-D5): the generated roots hold the new properties, none required.
#[test]
fn schemas_hold_the_new_optional_properties() {
    let schemas = crpg_data::generated_schemas().unwrap();
    for (file, added) in [
        ("ruleset.schema.json", RULESET_KEYS.as_slice()),
        ("encounter.schema.json", [ENCOUNTER_KEY].as_slice()),
    ] {
        let root: Value = serde_json::from_slice(&schemas[file]).unwrap();
        let shape = &root["oneOf"][0];
        let properties = shape["properties"].as_object().unwrap();
        let required: Vec<&str> = shape["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry.as_str().unwrap())
            .collect();
        for name in added {
            assert!(properties.contains_key(*name), "{file} lacks {name}");
            assert!(!required.contains(name), "{file} requires {name}");
        }
    }
    // The boolean renders as the ability flag does; the enums as definitions.
    let encounter: Value = serde_json::from_slice(&schemas["encounter.schema.json"]).unwrap();
    assert_eq!(
        encounter["oneOf"][0]["properties"]["respawns"]["type"],
        json!(["boolean", "null"])
    );
    let ruleset: Value = serde_json::from_slice(&schemas["ruleset.schema.json"]).unwrap();
    for (def, names) in [
        ("CreaturePersistenceWire", vec!["survivors", "registered"]),
        ("LateJoinWire", vec!["next_round", "after_active"]),
        (
            "EffectsOnExitWire",
            vec!["suspend", "strip", "strip_at_release"],
        ),
    ] {
        let variants: Vec<&str> = ruleset["$defs"][def]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["const"].as_str().unwrap())
            .collect();
        assert_eq!(variants, names, "{def}");
    }
}

// B-D4: the version tags and registry are untouched.
#[test]
fn version_tags_and_registry_are_unchanged() {
    let versions = crpg_data::schema_versions();
    let current = |family: &str| {
        versions
            .iter()
            .find(|entry| entry.schema_type == family)
            .map(|entry| entry.current)
    };
    assert_eq!(current("crpg.ruleset"), Some(2));
    assert_eq!(current("crpg.encounter"), Some(1));
    assert_eq!(versions.len(), 20);
}
