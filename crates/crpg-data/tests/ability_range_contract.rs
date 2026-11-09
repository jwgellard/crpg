#![forbid(unsafe_code)]
//! Two-keys contract tests for T149a (`crpg-data` ability `range`, X1),
//! written blind from `tasks/T149.md`: "Readings of the ADR", "Interface"
//! (both T149a sections), "Behaviour" `T149a: data (X1)` (B-A1..B-A6),
//! "Errors" and "Tests" (D-1..D-7). Every test names its D-n item and cites
//! the Behaviour items it covers. Integers only: the one non-integer JSON
//! number is built from text.
use crpg_core::{Fx16_16, Ulid};
use crpg_data::{
    Ability, DataError, Diagnostic, DiagnosticCode, Document, LengthWire, LoadedCampaign,
    MeasureWire, MovementWire, Ruleset, Severity, SourcePath, TurnMovementWire,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const ABILITY_18: &str = "00000000000000000000000018";
const RULESET_16: &str = "00000000000000000000000016";
const POOL_1F: &str = "0000000000000000000000001F";
const RANGE_MSG: &str = "ability range must be positive";
const DICE_MSG: &str = "ability damage dice must be 1..=128 UTF-8 bytes";
const SCHEMA_DESCRIPTION: &str =
    "Largest horizontal distance to the target, raw fixed-point metres; absent means unlimited.";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn json_file(relative: &str) -> Value {
    serde_json::from_slice(&std::fs::read(repo_root().join(relative)).unwrap()).unwrap()
}

/// A JSON value from text, so no Rust float literal ever appears.
fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

/// `rulesets/srd-lite/focus.json` parsed as JSON (the contract's `FOCUS`).
fn focus() -> Value {
    json_file("rulesets/srd-lite/focus.json")
}

/// `FOCUS` with `"range"` set to that JSON value (the contract's `with`).
fn with(range: Value) -> Value {
    let mut out = focus();
    out["range"] = range;
    out
}

fn read(value: &Value) -> Result<Document, DataError> {
    crpg_data::read_document(&serde_json::to_vec(value).unwrap())
}

fn read_ability(value: &Value) -> Ability {
    match read(value) {
        Ok(Document::Ability(ability)) => ability,
        other => panic!("expected a readable ability document: {other:?}"),
    }
}

fn message(result: Result<Document, DataError>) -> String {
    match result {
        Err(DataError::Malformed { message, .. }) => message,
        other => panic!("expected Malformed, got {other:?}"),
    }
}

fn written(doc: &Document) -> String {
    String::from_utf8(crpg_data::write_document(doc).unwrap()).unwrap()
}

fn ulid(text: &str) -> Ulid {
    text.parse().unwrap()
}

// ----------------------------------------------------------------- D-1

/// D-1 (B-A1): a present range round-trips as the raw `i32` in canonical key
/// order; absent and `null` read as `None` and write nothing; FOCUS itself
/// reads with `range == None` and writes its exact file bytes. The tag stays
/// `crpg.ability/2`.
#[test]
fn range_round_trips() {
    let doc = read_ability(&with(json!(98304)));
    assert_eq!(doc.range, Some(Fx16_16::from_raw(98304)));
    let text = written(&Document::Ability(doc.clone()));
    assert!(text.contains("\"range\": 98304"), "{text}");
    assert!(text.contains("\"schema\": \"crpg.ability/2\""), "{text}");
    let outcome_table = text.find("\"outcome_table\"").unwrap();
    let range = text.find("\"range\"").unwrap();
    let requires_target = text.find("\"requires_target\"").unwrap();
    assert!(
        outcome_table < range && range < requires_target,
        "range sorts after outcome_table and before requires_target: {text}"
    );
    let back = crpg_data::read_document(text.as_bytes()).unwrap();
    assert_eq!(back, Document::Ability(doc), "write then read is equal");

    let null_doc = read_ability(&with(Value::Null));
    assert_eq!(null_doc.range, None);
    assert!(
        !written(&Document::Ability(null_doc.clone())).contains("\"range\""),
        "a null range writes no key"
    );

    let bytes = std::fs::read(repo_root().join("rulesets/srd-lite/focus.json")).unwrap();
    let focus_doc = crpg_data::read_document(&bytes).unwrap();
    assert_eq!(
        crpg_data::write_document(&focus_doc).unwrap(),
        bytes,
        "FOCUS writes its exact file bytes"
    );
    let Document::Ability(ability) = &focus_doc else {
        panic!("expected ability, got {focus_doc:?}")
    };
    assert_eq!(ability.range, None);
    // An explicit `null` decodes to exactly the absent document.
    assert_eq!(Document::Ability(null_doc), focus_doc);
}

// ----------------------------------------------------------------- D-2

/// D-2 (B-A2, B-A3): the positive bound at raw `1` and `i32::MAX`, with raw
/// `42` as the control that differs in one input; raw `0`, `-1` and
/// `i32::MIN` fail with exactly the contract's message; a `range` that is not
/// a JSON integer within `i32` is `Malformed` (serde's text, not pinned).
#[test]
fn range_bounds() {
    for good in [1, 42, i32::MAX] {
        let ability = read_ability(&with(json!(good)));
        assert_eq!(ability.range, Some(Fx16_16::from_raw(good)), "range {good}");
    }
    for bad in [0, -1, i32::MIN] {
        assert_eq!(message(read(&with(json!(bad)))), RANGE_MSG, "range {bad}");
    }
    let malformed: [(&str, Value); 4] = [
        ("string", json!("65536")),
        ("fractional", parse("1.5")),
        ("above i32::MAX", json!(2147483648_i64)),
        ("below i32::MIN", json!(-2147483649_i64)),
    ];
    for (name, value) in malformed {
        assert!(
            matches!(read(&with(value)), Err(DataError::Malformed { .. })),
            "{name} is Malformed"
        );
    }
}

// ----------------------------------------------------------------- D-3

/// D-3 (B-A3): the range check is the last check of the `Ability` arm, so an
/// earlier failure wins. A document with bad dice plus `range: 0` reports the
/// dice error; without the bad dice the range error is reported.
#[test]
fn range_check_is_last() {
    let mut both = with(json!(0));
    both.pointer_mut("/damage/0").unwrap()["dice"] = json!("");
    assert_eq!(message(read(&both)), DICE_MSG, "the earlier failure wins");
    // Control that differs in one input: bad dice with no range, same error.
    let mut dice_only = focus();
    dice_only.pointer_mut("/damage/0").unwrap()["dice"] = json!("");
    assert_eq!(message(read(&dice_only)), DICE_MSG);
    // Without the bad dice the document's one problem is the range.
    assert_eq!(message(read(&with(json!(0)))), RANGE_MSG);
}

// ----------------------------------------------------------------- D-4

fn collect_json(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_json(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            out.push(path);
        }
    }
}

/// D-4 (B-A1, B-A6): every ability file under `rulesets/` and
/// `campaigns/fixtures/` reads with `range == None`, every one already at
/// `crpg.ability/2` writes its exact bytes, no written text holds a `range`
/// key, and `schema_versions()` holds `crpg.ability` at 2 (no new edge).
#[test]
fn shipped_abilities_unchanged() {
    let mut files = Vec::new();
    for dir in ["rulesets", "campaigns/fixtures"] {
        collect_json(&repo_root().join(dir), &mut files);
    }
    let (mut seen, mut current, mut older) = (0, 0, 0);
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
        seen += 1;
        let doc = crpg_data::read_document(&bytes)
            .unwrap_or_else(|err| panic!("{} should read: {err}", path.display()));
        let Document::Ability(ability) = &doc else {
            panic!("{}: expected ability, got {doc:?}", path.display())
        };
        assert_eq!(ability.range, None, "{}", path.display());
        let out = crpg_data::write_document(&doc).unwrap();
        let text = String::from_utf8(out.clone()).unwrap();
        assert!(!text.contains("\"range\""), "{}", path.display());
        match tag {
            "crpg.ability/2" => {
                current += 1;
                assert_eq!(out, bytes, "{} re-encodes unchanged", path.display());
            }
            "crpg.ability/1" => {
                older += 1;
                // Control that differs in one input: a /1 file migrates to
                // /2 on write and still invents no range.
                assert!(
                    text.contains("\"schema\": \"crpg.ability/2\""),
                    "{} writes at the current tag",
                    path.display()
                );
            }
            other => panic!("{}: unexpected ability tag {other}", path.display()),
        }
    }
    assert!(seen >= 6, "found only {seen} ability files");
    assert!(current >= 4, "found only {current} crpg.ability/2 files");
    assert!(older >= 2, "found only {older} crpg.ability/1 files");
    let version = crpg_data::schema_versions()
        .iter()
        .find(|entry| entry.schema_type == "crpg.ability")
        .map(|entry| entry.current);
    assert_eq!(version, Some(2), "the tag stays crpg.ability/2");
}

// ----------------------------------------------------------------- D-5

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

fn validate_docs(mutate: impl FnOnce(&mut BTreeMap<SourcePath, Document>)) -> Vec<Diagnostic> {
    let engine: semver::Version = "0.1.0".parse().unwrap();
    let campaign = crpg_data::load_campaign(&srd_files(), &engine).expect("combat_srd loads");
    let mut docs = campaign.documents;
    mutate(&mut docs);
    crpg_data::validate(&LoadedCampaign {
        documents: docs,
        index: BTreeMap::new(),
    })
}

fn focus_mut(docs: &mut BTreeMap<SourcePath, Document>) -> &mut Ability {
    let id = ulid(ABILITY_18);
    docs.values_mut()
        .find_map(|doc| match doc {
            Document::Ability(ability) if ability.id == id => Some(ability),
            _ => None,
        })
        .expect("the focus ability is present")
}

fn ruleset_mut(docs: &mut BTreeMap<SourcePath, Document>) -> &mut Ruleset {
    docs.values_mut()
        .find_map(|doc| match doc {
            Document::Ruleset(ruleset) => Some(ruleset),
            _ => None,
        })
        .expect("the ruleset is present")
}

/// T148's wire example (`tasks/T148.md:189-200`), for the movement control.
fn movement_wire() -> MovementWire {
    MovementWire {
        speed: LengthWire::Constant(Fx16_16::from_raw(16384)),
        measure: Some(MeasureWire::Euclidean),
        turn: TurnMovementWire {
            pool: ulid(POOL_1F),
            step: LengthWire::Constant(Fx16_16::from_raw(98304)),
            step_cost: 1,
        },
    }
}

/// D-5 (B-A4): a ranged ability under a ruleset with no movement warns
/// exactly once, with the pinned file, pointer, code, severity, message and
/// fix. Controls: the baseline is clean, the same range with the T148
/// movement wire is clean, and movement without a range is clean.
#[test]
fn range_without_movement_warns() {
    assert_eq!(validate_docs(|_| {}), Vec::new(), "baseline is clean");
    let findings = validate_docs(|docs| {
        focus_mut(docs).range = Some(Fx16_16::from_raw(98304));
    });
    let expected = Diagnostic {
        file: Some("abilities/focus.json".parse().unwrap()),
        pointer: "/range".into(),
        severity: Severity::Warning,
        code: DiagnosticCode::RangeWithoutMovement,
        message: format!(
            "ability {ABILITY_18} declares a range but ruleset {RULESET_16} has no movement"
        ),
        suggested_fix: Some("add movement to the ruleset or remove the range".into()),
    };
    assert_eq!(findings, vec![expected]);
    assert_eq!(
        validate_docs(|docs| {
            focus_mut(docs).range = Some(Fx16_16::from_raw(98304));
            ruleset_mut(docs).movement = Some(Box::new(movement_wire()));
        }),
        Vec::new(),
        "the same range fires nothing under the T148 movement wire"
    );
    assert_eq!(
        validate_docs(|docs| {
            ruleset_mut(docs).movement = Some(Box::new(movement_wire()));
        }),
        Vec::new(),
        "movement without a range fires nothing"
    );
}

// ----------------------------------------------------------------- D-6

/// D-6 (B-A4): `RangeWithoutMovement.as_str()` and its serde string are
/// `range_without_movement`, and `FormatMismatch < RangeWithoutMovement`.
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
    // Controls that differ in one input: the neighbouring code differs.
    assert_ne!(
        DiagnosticCode::FormatMismatch.as_str(),
        "range_without_movement"
    );
    assert_ne!(
        serde_json::to_value(DiagnosticCode::FormatMismatch).unwrap(),
        json!("range_without_movement")
    );
}

// ----------------------------------------------------------------- D-7

/// D-7 (B-A5): `generated_schemas()["ability.schema.json"]` holds the
/// optional `range` property with the doc comment as its `description`,
/// `required` does not contain it, and the generated bytes equal the
/// checked-in file.
#[test]
fn schema_holds_range() {
    let schemas = crpg_data::generated_schemas().unwrap();
    let generated = schemas
        .get("ability.schema.json")
        .expect("the ability schema is generated");
    let root: Value = serde_json::from_slice(generated).unwrap();
    let shape = &root["oneOf"][0];
    let properties = shape["properties"].as_object().unwrap();
    assert_eq!(
        properties["range"]["description"],
        json!(SCHEMA_DESCRIPTION)
    );
    let required = shape["required"].as_array().unwrap();
    assert!(
        !required.contains(&json!("range")),
        "range must be optional"
    );
    assert!(
        required.contains(&json!("outcome_table")),
        "control: required is read from the same shape"
    );
    assert!(
        properties.contains_key("outcome_table"),
        "control: the neighbouring property is present"
    );
    let checked_in = std::fs::read(repo_root().join("schemas/ability.schema.json")).unwrap();
    assert_eq!(
        generated, &checked_in,
        "the checked-in schema has not drifted"
    );
}
