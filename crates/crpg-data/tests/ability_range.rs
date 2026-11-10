#![forbid(unsafe_code)]
//! T149a: the optional `range` field on `crpg.ability/2` (B-A1..B-A6). Absence
//! means unlimited and writes nothing, so every existing document is unchanged.
use crpg_core::{Fx16_16, Ulid};
use crpg_data::{
    DataError, Diagnostic, DiagnosticCode, Document, LengthWire, LoadedCampaign, MeasureWire,
    MovementWire, Severity, SourcePath, TurnMovementWire,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const FOCUS_ID: &str = "00000000000000000000000018";
const POOL_1F: &str = "0000000000000000000000001F";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn focus_bytes() -> Vec<u8> {
    std::fs::read(repo_root().join("rulesets/srd-lite/focus.json")).unwrap()
}

fn focus() -> Value {
    serde_json::from_slice(&focus_bytes()).unwrap()
}

/// `FOCUS` with `"range"` set to `range`.
fn with(range: Value) -> Value {
    let mut out = focus();
    out["range"] = range;
    out
}

fn read(value: &Value) -> Result<Document, DataError> {
    crpg_data::read_document(&serde_json::to_vec(value).unwrap())
}

fn message(result: Result<Document, DataError>) -> String {
    match result {
        Err(DataError::Malformed { message, .. }) => message,
        other => panic!("expected Malformed, got {other:?}"),
    }
}

/// The message ends with `expected` and carries it exactly once.
fn exact(result: Result<Document, DataError>, expected: &str) {
    let got = message(result);
    assert!(
        got.ends_with(expected),
        "expected {expected:?}, got {got:?}"
    );
    assert_eq!(got.matches(expected).count(), 1, "{got}");
}

fn ability_of(doc: Document) -> crpg_data::Ability {
    match doc {
        Document::Ability(ability) => ability,
        other => panic!("expected ability, got {other:?}"),
    }
}

fn written(doc: &Document) -> String {
    String::from_utf8(crpg_data::write_document(doc).unwrap()).unwrap()
}

fn ulid(text: &str) -> Ulid {
    text.parse().unwrap()
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

fn srd_campaign() -> LoadedCampaign {
    let engine: semver::Version = "0.1.0".parse().unwrap();
    crpg_data::load_campaign(&srd_files(), &engine).expect("combat_srd loads")
}

fn validate_docs(docs: &BTreeMap<SourcePath, Document>) -> Vec<Diagnostic> {
    crpg_data::validate(&LoadedCampaign {
        documents: docs.clone(),
        index: BTreeMap::new(),
    })
}

fn mutate_srd(mutate: impl FnOnce(&mut BTreeMap<SourcePath, Document>)) -> Vec<Diagnostic> {
    let mut docs = srd_campaign().documents;
    mutate(&mut docs);
    validate_docs(&docs)
}

fn focus_ability_mut(docs: &mut BTreeMap<SourcePath, Document>) -> &mut crpg_data::Ability {
    docs.values_mut()
        .find_map(|doc| match doc {
            Document::Ability(ability) if ability.id == ulid(FOCUS_ID) => Some(ability),
            _ => None,
        })
        .unwrap()
}

fn ruleset_mut(docs: &mut BTreeMap<SourcePath, Document>) -> &mut crpg_data::Ruleset {
    docs.values_mut()
        .find_map(|doc| match doc {
            Document::Ruleset(ruleset) => Some(ruleset),
            _ => None,
        })
        .unwrap()
}

fn movement_wire() -> MovementWire {
    let constant = |raw| LengthWire::Constant(Fx16_16::from_raw(raw));
    MovementWire {
        speed: constant(16384),
        measure: Some(MeasureWire::Euclidean),
        turn: TurnMovementWire {
            pool: ulid(POOL_1F),
            step: constant(98304),
            step_cost: 1,
        },
    }
}

// D-1 / B-A1: absence and null write no key; a present range round-trips in
// canonical key order.
#[test]
fn range_round_trips() {
    let doc = ability_of(read(&with(json!(98304))).unwrap());
    assert_eq!(doc.range, Some(Fx16_16::from_raw(98304)));
    let text = written(&Document::Ability(doc.clone()));
    assert!(text.contains("\"range\": 98304"), "{text}");
    let outcome = text.find("\"outcome_table\"").unwrap();
    let range = text.find("\"range\"").unwrap();
    let requires = text.find("\"requires_target\"").unwrap();
    assert!(outcome < range && range < requires, "{text}");
    let reread = ability_of(crpg_data::read_document(text.as_bytes()).unwrap());
    assert_eq!(reread, doc);

    let doc = ability_of(read(&with(Value::Null)).unwrap());
    assert_eq!(doc.range, None);
    assert!(!written(&Document::Ability(doc)).contains("\"range\""));

    let bytes = focus_bytes();
    let doc = ability_of(crpg_data::read_document(&bytes).unwrap());
    assert_eq!(doc.range, None);
    assert_eq!(
        crpg_data::write_document(&Document::Ability(doc)).unwrap(),
        bytes
    );
}

// D-2 / B-A2, B-A3: the raw i32 boundaries read; zero and negatives fail the
// read-time positivity rule; non-integer forms fail structure.
#[test]
fn range_bounds() {
    assert_eq!(
        ability_of(read(&with(json!(1))).unwrap()).range,
        Some(Fx16_16::from_raw(1))
    );
    assert_eq!(
        ability_of(read(&with(json!(2147483647))).unwrap()).range,
        Some(Fx16_16::from_raw(i32::MAX))
    );
    for raw in [0, -1, -2147483648] {
        exact(read(&with(json!(raw))), "ability range must be positive");
    }
    for bad in [
        json!("65536"),
        json!(1.5),
        json!(2147483648i64),
        json!(-2147483649i64),
    ] {
        assert!(
            matches!(read(&with(bad)), Err(DataError::Malformed { .. })),
            "expected Malformed"
        );
    }
}

// D-3 / B-A3: the range check is the last check of the ability arm.
#[test]
fn range_check_is_last() {
    let mut value = with(json!(0));
    value["damage"][0]["dice"] = json!("");
    exact(
        read(&value),
        "ability damage dice must be 1..=128 UTF-8 bytes",
    );
    exact(read(&with(json!(0))), "ability range must be positive");
}

// D-4 / B-A1, B-A6: no shipped ability declares a range or changes bytes.
#[test]
fn shipped_abilities_unchanged() {
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
    let mut files = Vec::new();
    walk(&repo_root().join("rulesets"), &mut files);
    walk(&repo_root().join("campaigns/fixtures"), &mut files);
    let mut checked = 0;
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let Some(tag) = value.get("schema").and_then(Value::as_str) else {
            continue;
        };
        if !tag.starts_with("crpg.ability/") {
            continue;
        }
        checked += 1;
        let doc = crpg_data::read_document(&bytes)
            .unwrap_or_else(|err| panic!("{} should read: {err}", path.display()));
        assert_eq!(ability_of(doc.clone()).range, None, "{}", path.display());
        let out = crpg_data::write_document(&doc).unwrap();
        assert!(
            !String::from_utf8(out.clone())
                .unwrap()
                .contains("\"range\""),
            "{}",
            path.display()
        );
        if tag == "crpg.ability/2" {
            assert_eq!(out, bytes, "{}", path.display());
        }
    }
    assert!(checked > 0, "no ability files found");
    let ability = crpg_data::schema_versions()
        .iter()
        .find(|entry| entry.schema_type == "crpg.ability")
        .unwrap();
    assert_eq!(ability.current, 2);
}

// D-5 / B-A4: a ranged ability under a movement-less ruleset warns once; the
// same ruleset with movement is clean.
#[test]
fn range_without_movement_warns() {
    assert!(validate_docs(&srd_campaign().documents).is_empty());

    let findings = mutate_srd(|docs| {
        focus_ability_mut(docs).range = Some(Fx16_16::from_raw(98304));
    });
    assert_eq!(findings.len(), 1, "{findings:?}");
    let finding = &findings[0];
    assert_eq!(
        finding.file.as_ref().unwrap().as_str(),
        "abilities/focus.json"
    );
    assert_eq!(finding.pointer, "/range");
    assert_eq!(finding.code, DiagnosticCode::RangeWithoutMovement);
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(
        finding.message,
        "ability 00000000000000000000000018 declares a range but ruleset 00000000000000000000000016 has no movement"
    );
    assert_eq!(
        finding.suggested_fix.as_deref(),
        Some("add movement to the ruleset or remove the range")
    );

    let findings = mutate_srd(|docs| {
        focus_ability_mut(docs).range = Some(Fx16_16::from_raw(98304));
        ruleset_mut(docs).movement = Some(Box::new(movement_wire()));
    });
    assert!(findings.is_empty(), "{findings:?}");
}

// D-6 / B-A4: stable spelling and ordering.
#[test]
fn code_spelling() {
    assert_eq!(
        DiagnosticCode::RangeWithoutMovement.as_str(),
        "range_without_movement"
    );
    assert_eq!(
        serde_json::to_value(DiagnosticCode::RangeWithoutMovement).unwrap(),
        json!("range_without_movement")
    );
    assert!(DiagnosticCode::FormatMismatch < DiagnosticCode::RangeWithoutMovement);
}

// D-7 / B-A5: the generated schema carries the optional range; the checked-in
// baseline equals the generated bytes.
#[test]
fn schema_holds_range() {
    let schemas = crpg_data::generated_schemas().unwrap();
    let bytes = &schemas["ability.schema.json"];
    let value: Value = serde_json::from_slice(bytes).unwrap();
    let property = &value["oneOf"][0]["properties"]["range"];
    assert_eq!(
        property["description"],
        json!("Largest horizontal distance to the target, raw fixed-point metres; absent means unlimited.")
    );
    assert!(property.is_object(), "{property}");
    let required = value["oneOf"][0]["required"].as_array().unwrap();
    assert!(!required.contains(&json!("range")));
    let checked_in = std::fs::read(repo_root().join("schemas/ability.schema.json")).unwrap();
    assert_eq!(bytes, &checked_in);
}
