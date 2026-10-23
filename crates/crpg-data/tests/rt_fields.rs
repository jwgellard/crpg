#![forbid(unsafe_code)]
//! T136a: optional real-time fields on `crpg.ruleset/2` and `crpg.ability/2`.
use crpg_data::{DataError, Document, LoadedCampaign, SourcePath};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const RULESET_MSG: &str = "ruleset round_ticks must be nonzero";
const ABILITY_MSG: &str = "ability recovery_ticks must be nonzero";
const KEYS: [&str; 3] = ["round_ticks", "recovery_ticks", "repeats_in_real_time"];

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

fn heavy() -> Value {
    json_file("rulesets/srd-lite/heavy.json")
}

fn read(value: &Value) -> Result<Document, DataError> {
    crpg_data::read_document(&serde_json::to_vec(value).unwrap())
}

fn read_ok(value: &Value) -> Document {
    read(value).unwrap_or_else(|err| panic!("document should read: {err}"))
}

fn malformed_message(result: Result<Document, DataError>) -> String {
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

fn ability_of(doc: Document) -> crpg_data::Ability {
    match doc {
        Document::Ability(ability) => ability,
        other => panic!("expected ability, got {other:?}"),
    }
}

fn with(base: &Value, key: &str, value: Value) -> Value {
    let mut out = base.clone();
    out[key] = value;
    out
}

fn written(doc: &Document) -> Vec<u8> {
    crpg_data::write_document(doc).unwrap()
}

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap()
}

// T2 (behaviour 1, 2): present values round trip and the writer emits the key.
#[test]
fn present_fields_round_trip_through_the_canonical_writer() {
    let ruleset = ruleset_of(read_ok(&with(&srd_ruleset(), "round_ticks", json!(120))));
    assert_eq!(ruleset.round_ticks, Some(120));
    let doc = Document::Ruleset(ruleset);
    let bytes = written(&doc);
    assert!(text(&bytes).contains("\"round_ticks\": 120"));
    assert_eq!(crpg_data::read_document(&bytes).unwrap(), doc);

    for repeats in [true, false] {
        let mut value = with(&heavy(), "recovery_ticks", json!(60));
        value["repeats_in_real_time"] = json!(repeats);
        let ability = ability_of(read_ok(&value));
        assert_eq!(ability.recovery_ticks, Some(60));
        assert_eq!(ability.repeats_in_real_time, Some(repeats));
        let doc = Document::Ability(ability);
        let bytes = written(&doc);
        assert!(text(&bytes).contains("\"recovery_ticks\": 60"));
        // `false` is kept, not dropped.
        assert!(text(&bytes).contains(&format!("\"repeats_in_real_time\": {repeats}")));
        assert_eq!(crpg_data::read_document(&bytes).unwrap(), doc);
    }
}

fn existing_rt_documents() -> Vec<(String, Value)> {
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
        if tag.starts_with("crpg.ruleset/") || tag.starts_with("crpg.ability/") {
            out.push((path.display().to_string(), value));
        }
    }
    out
}

// T3 (behaviour 2, 5): every existing document has all three fields absent.
#[test]
fn existing_documents_have_all_fields_absent() {
    let documents = existing_rt_documents();
    // minimal-d6 (3 files incl. one table) + srd-lite + both campaign copies.
    assert!(documents.len() >= 10, "found {}", documents.len());
    let mut migrated_from_v1 = 0;
    for (path, value) in documents {
        let tag = value["schema"].as_str().unwrap().to_string();
        if tag.ends_with("/1") {
            migrated_from_v1 += 1;
        }
        let doc = read_ok(&value);
        match &doc {
            Document::Ruleset(ruleset) => assert_eq!(ruleset.round_ticks, None, "{path}"),
            Document::Ability(ability) => {
                assert_eq!(ability.recovery_ticks, None, "{path}");
                assert_eq!(ability.repeats_in_real_time, None, "{path}");
            }
            other => panic!("{path}: unexpected {other:?}"),
        }
        let bytes = written(&doc);
        for key in KEYS {
            assert!(!text(&bytes).contains(key), "{path} writes {key}");
        }
    }
    assert!(migrated_from_v1 > 0, "a /1 source must be exercised");
}

// T4 (behaviour 3): zero fails with the exact message.
#[test]
fn zero_ticks_fail_with_the_exact_messages() {
    assert_eq!(
        malformed_message(read(&with(&srd_ruleset(), "round_ticks", json!(0)))),
        RULESET_MSG
    );
    assert_eq!(
        malformed_message(read(&with(&heavy(), "recovery_ticks", json!(0)))),
        ABILITY_MSG
    );
}

// T5 (behaviour 1): bounds and types.
#[test]
fn counts_accept_one_and_u32_max_and_reject_other_forms() {
    for ok in [json!(1), json!(4_294_967_295u64)] {
        let ruleset = ruleset_of(read_ok(&with(&srd_ruleset(), "round_ticks", ok.clone())));
        assert_eq!(ruleset.round_ticks.map(u64::from), ok.as_u64());
        let ability = ability_of(read_ok(&with(&heavy(), "recovery_ticks", ok.clone())));
        assert_eq!(ability.recovery_ticks.map(u64::from), ok.as_u64());
    }
    for bad in [
        json!(4_294_967_296u64),
        json!(-1),
        json!("120"),
        json!(120.0),
    ] {
        assert!(
            matches!(
                read(&with(&srd_ruleset(), "round_ticks", bad.clone())),
                Err(DataError::Malformed { .. })
            ),
            "round_ticks accepted {bad}"
        );
        assert!(
            matches!(
                read(&with(&heavy(), "recovery_ticks", bad.clone())),
                Err(DataError::Malformed { .. })
            ),
            "recovery_ticks accepted {bad}"
        );
    }
    for bad in [json!(1), json!("true")] {
        assert!(
            matches!(
                read(&with(&heavy(), "repeats_in_real_time", bad.clone())),
                Err(DataError::Malformed { .. })
            ),
            "repeats_in_real_time accepted {bad}"
        );
    }
}

// T6 (behaviour 1, 2): explicit null reads as absent and is not re-encoded.
#[test]
fn null_reads_as_absent_and_is_omitted_on_write() {
    let ruleset = ruleset_of(read_ok(&with(&srd_ruleset(), "round_ticks", Value::Null)));
    assert_eq!(ruleset.round_ticks, None);
    let bytes = written(&Document::Ruleset(ruleset));
    assert!(!text(&bytes).contains("round_ticks"));

    let mut value = with(&heavy(), "recovery_ticks", Value::Null);
    value["repeats_in_real_time"] = Value::Null;
    let ability = ability_of(read_ok(&value));
    assert_eq!(ability.recovery_ticks, None);
    assert_eq!(ability.repeats_in_real_time, None);
    let bytes = written(&Document::Ability(ability));
    assert!(!text(&bytes).contains("recovery_ticks"));
    assert!(!text(&bytes).contains("repeats_in_real_time"));
}

// T7 (behaviour 1): near-miss keys are unknown fields.
#[test]
fn near_miss_keys_are_rejected_as_unknown_fields() {
    let message = malformed_message(read(&with(&srd_ruleset(), "round_tick", json!(120))));
    assert!(message.contains("unknown field"), "{message}");
    let message = malformed_message(read(&with(&heavy(), "recovery_tick", json!(60))));
    assert!(message.contains("unknown field"), "{message}");
}

// T8 (behaviour 3): the new checks come last in their arm.
#[test]
fn earlier_faults_are_reported_before_the_zero_checks() {
    let mut ruleset = srd_ruleset();
    let first = ruleset["abilities"][0].clone();
    ruleset["abilities"] = json!([first.clone(), first]);
    ruleset["round_ticks"] = json!(0);
    assert_eq!(
        malformed_message(read(&ruleset)),
        "duplicate ruleset ability"
    );

    let mut ability = heavy();
    ability["attribute"] = json!("");
    ability["recovery_ticks"] = json!(0);
    assert_eq!(
        malformed_message(read(&ability)),
        "ability attribute must be nonempty"
    );
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

fn validate_mutated(round_ticks: Option<u32>) -> Vec<crpg_data::Diagnostic> {
    let engine: semver::Version = "0.1.0".parse().unwrap();
    let loaded = crpg_data::load_campaign(&srd_files(), &engine).expect("combat_srd loads");
    let mut docs = loaded.documents.clone();
    let mut abilities = 0;
    for doc in docs.values_mut() {
        match doc {
            Document::Ability(ability) => {
                ability.recovery_ticks = Some(60);
                ability.repeats_in_real_time = Some(true);
                abilities += 1;
            }
            Document::Ruleset(ruleset) => ruleset.round_ticks = round_ticks,
            _ => {}
        }
    }
    assert_eq!(abilities, 2);
    crpg_data::validate(&LoadedCampaign {
        documents: docs,
        index: BTreeMap::new(),
    })
}

// T9 (behaviour 4), superseded by T136b item 11: RT fields under a ruleset
// with no round_ticks give one warning per ability; with round_ticks, none.
#[test]
fn validate_warns_for_rt_fields_without_round_ticks_only() {
    let findings = validate_mutated(None);
    assert_eq!(findings.len(), 2, "{findings:?}");
    for finding in &findings {
        assert_eq!(finding.severity, crpg_data::Severity::Warning);
        assert_eq!(
            finding.code,
            crpg_data::DiagnosticCode::RealTimeFieldsWithoutRoundTicks
        );
        assert_eq!(finding.pointer, "/recovery_ticks");
    }
    assert_eq!(validate_mutated(Some(120)), Vec::new());
}

// T10 (behaviour 6): regenerated schemas hold the properties, none required.
#[test]
fn schemas_gain_exactly_the_new_optional_properties() {
    let schemas = crpg_data::generated_schemas().unwrap();
    for (file, added, others) in [
        (
            "ruleset.schema.json",
            vec!["round_ticks"],
            ["recovery_ticks"],
        ),
        (
            "ability.schema.json",
            vec!["recovery_ticks", "repeats_in_real_time"],
            ["round_ticks"],
        ),
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
            assert!(properties.contains_key(name), "{file} lacks {name}");
            assert!(!required.contains(&name), "{file} requires {name}");
        }
        for name in others {
            assert!(!properties.contains_key(name), "{file} has stray {name}");
        }
    }
}

// Behaviour 5: the version tags are untouched.
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
    assert_eq!(current("crpg.ability"), Some(2));
    assert_eq!(versions.len(), 20);
}
