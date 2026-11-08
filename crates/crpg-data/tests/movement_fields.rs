#![forbid(unsafe_code)]
//! T148: the optional `movement` block on `crpg.ruleset/2` (B1-B11). The block
//! is data only: no version moves, absence writes nothing, and every existing
//! document is unchanged.
use crpg_core::{Fx16_16, Ulid};
use crpg_data::{
    DataError, Diagnostic, DiagnosticCode, Document, LengthWire, LoadedCampaign, MeasureWire,
    MovementWire, Severity, SourcePath, StatDecl, StatKindWire, TurnMovementWire,
    MAX_SPEED_PER_TICK_RAW,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const POOL_1E: &str = "0000000000000000000000001E";
const POOL_1F: &str = "0000000000000000000000001F";
const ABILITY_17: &str = "00000000000000000000000017";
const HERO: &str = "00000000000000000000000014";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn srd_ruleset() -> Value {
    let bytes = std::fs::read(repo_root().join("rulesets/srd-lite/ruleset.json")).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn wire() -> Value {
    json!({
        "speed": { "type": "constant", "value": 16384 },
        "measure": "euclidean",
        "turn": {
            "pool": POOL_1F,
            "step": { "type": "constant", "value": 98304 },
            "step_cost": 1
        }
    })
}

fn fixed_stat(name: &str) -> Value {
    json!({ "kind": "fixed", "name": name })
}

/// srd-lite with `movement` set and `extra` stats appended.
fn ruleset_with(movement: Value, extra: &[Value]) -> Value {
    let mut out = srd_ruleset();
    out["movement"] = movement;
    for stat in extra {
        out["stats"].as_array_mut().unwrap().push(stat.clone());
    }
    out
}

fn movement_with(f: impl FnOnce(&mut Value)) -> Value {
    let mut movement = wire();
    f(&mut movement);
    movement
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

fn written(doc: &Document) -> String {
    String::from_utf8(crpg_data::write_document(doc).unwrap()).unwrap()
}

fn constant(raw: i32) -> LengthWire {
    LengthWire::Constant(Fx16_16::from_raw(raw))
}

fn stat(name: &str) -> LengthWire {
    LengthWire::Stat(name.to_string())
}

fn ulid(text: &str) -> Ulid {
    text.parse().unwrap()
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

// B1 / M-1: absence is unchanged for every existing ruleset file.
#[test]
fn absent_movement_is_unchanged() {
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
    let mut current = 0;
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let Some(tag) = value.get("schema").and_then(Value::as_str) else {
            continue;
        };
        if !tag.starts_with("crpg.ruleset/") {
            continue;
        }
        let doc = crpg_data::read_document(&bytes)
            .unwrap_or_else(|err| panic!("{} should read: {err}", path.display()));
        assert_eq!(ruleset_of(doc.clone()).movement, None, "{}", path.display());
        let out = crpg_data::write_document(&doc).unwrap();
        assert!(
            !String::from_utf8(out.clone())
                .unwrap()
                .contains("\"movement\""),
            "{}",
            path.display()
        );
        if tag == "crpg.ruleset/2" {
            assert_eq!(out, bytes, "{} must re-encode to its bytes", path.display());
            current += 1;
        }
        checked += 1;
    }
    assert!(checked >= 3, "found only {checked} ruleset files");
    assert!(current >= 2, "found only {current} current ruleset files");
}

// B2 / M-2: the wire example round-trips; null reads as absent.
#[test]
fn wire_example_round_trips() {
    let doc = read_ok(&ruleset_with(wire(), &[]));
    assert_eq!(
        ruleset_of(doc.clone()).movement,
        Some(MovementWire {
            speed: constant(16384),
            measure: Some(MeasureWire::Euclidean),
            turn: TurnMovementWire {
                pool: ulid(POOL_1F),
                step: constant(98304),
                step_cost: 1,
            },
        })
    );
    let text = written(&doc);
    assert!(text.contains("\"measure\": \"euclidean\""), "{text}");
    assert_eq!(
        crpg_data::read_document(text.as_bytes()).unwrap(),
        doc,
        "write then read"
    );

    let no_measure = movement_with(|m| {
        m.as_object_mut().unwrap().remove("measure");
    });
    let doc = read_ok(&ruleset_with(no_measure, &[]));
    assert_eq!(ruleset_of(doc.clone()).movement.unwrap().measure, None);
    let text = written(&doc);
    assert!(!text.contains("\"measure\""), "{text}");
    assert!(text.contains("\"movement\""), "{text}");

    let null_measure = movement_with(|m| m["measure"] = Value::Null);
    let doc = read_ok(&ruleset_with(null_measure, &[]));
    assert_eq!(ruleset_of(doc.clone()).movement.unwrap().measure, None);
    assert!(!written(&doc).contains("\"measure\""));

    let doc = read_ok(&ruleset_with(Value::Null, &[]));
    assert_eq!(ruleset_of(doc.clone()).movement, None);
    assert!(!written(&doc).contains("\"movement\""));

    let pace = movement_with(|m| m["speed"] = json!({ "type": "stat", "value": "pace" }));
    let doc = read_ok(&ruleset_with(pace, &[fixed_stat("pace")]));
    assert_eq!(
        ruleset_of(doc.clone()).movement.unwrap().speed,
        stat("pace")
    );
    assert_eq!(
        crpg_data::read_document(written(&doc).as_bytes()).unwrap(),
        doc
    );
}

// B3 / M-3: structural rejections.
#[test]
fn structural_rejections() {
    let edit = |f: &dyn Fn(&mut Value)| {
        let mut movement = wire();
        f(&mut movement);
        ruleset_with(movement, &[])
    };
    read_ok(&edit(&|_| {}));
    let with_substring: Vec<(&str, Value, &str)> = vec![
        (
            "unknown key in movement",
            edit(&|m| m["speeed"] = json!(1)),
            "unknown field",
        ),
        (
            "unknown key in turn",
            edit(&|m| m["turn"]["steps"] = json!(1)),
            "unknown field",
        ),
        (
            "unknown key in length",
            edit(&|m| m["speed"]["extra"] = json!(1)),
            // The contract names `unknown field`, but the derived adjacently
            // tagged decoder (which the contract also requires) words it so.
            "string \"extra\", expected \"type\" or \"value\"",
        ),
        (
            "missing turn",
            edit(&|m| {
                m.as_object_mut().unwrap().remove("turn");
            }),
            "missing field",
        ),
        (
            "missing speed",
            edit(&|m| {
                m.as_object_mut().unwrap().remove("speed");
            }),
            "missing field",
        ),
        (
            "missing pool",
            edit(&|m| {
                m["turn"].as_object_mut().unwrap().remove("pool");
            }),
            "missing field",
        ),
        (
            "missing step",
            edit(&|m| {
                m["turn"].as_object_mut().unwrap().remove("step");
            }),
            "missing field",
        ),
        (
            "missing step_cost",
            edit(&|m| {
                m["turn"].as_object_mut().unwrap().remove("step_cost");
            }),
            "missing field",
        ),
        (
            "length without value",
            edit(&|m| {
                m["speed"].as_object_mut().unwrap().remove("value");
            }),
            "missing field",
        ),
        (
            "length without type",
            edit(&|m| {
                m["speed"].as_object_mut().unwrap().remove("type");
            }),
            "missing field",
        ),
        (
            "unknown length type",
            edit(&|m| m["speed"] = json!({ "type": "metres", "value": 1 })),
            "unknown variant",
        ),
        (
            "fractional constant",
            edit(&|m| m["speed"]["value"] = json!(1.5)),
            // The contract names `non-integer number`; the strict parser
            // rejects the float first, with this wording.
            "only integer numbers",
        ),
    ];
    for (name, doc, needle) in with_substring {
        let got = message(read(&doc));
        assert!(got.contains(needle), "{name}: {got}");
    }
    let without_substring: Vec<(&str, Value)> = vec![
        (
            "constant above i32",
            edit(&|m| m["speed"]["value"] = json!(2147483648_i64)),
        ),
        (
            "negative step_cost",
            edit(&|m| m["turn"]["step_cost"] = json!(-1)),
        ),
        (
            "step_cost above u32",
            edit(&|m| m["turn"]["step_cost"] = json!(4294967296_i64)),
        ),
        (
            "non-integer step_cost",
            edit(&|m| m["turn"]["step_cost"] = json!("1")),
        ),
        (
            "pool not a ulid",
            edit(&|m| m["turn"]["pool"] = json!("not-a-ulid")),
        ),
    ];
    for (name, doc) in without_substring {
        assert!(
            matches!(read(&doc), Err(DataError::Malformed { .. })),
            "{name}"
        );
    }
    for bad in [json!(1), json!({ "type": "euclidean" })] {
        let got = message(read(&edit(&|m| m["measure"] = bad.clone())));
        assert!(got.ends_with("measure must be a measure name"), "{got}");
    }
    let got = message(read(&edit(&|m| m["measure"] = json!("manhattan"))));
    assert!(
        got.contains("unknown variant `manhattan`, expected `euclidean`"),
        "{got}"
    );
}

// B4 / M-4.
#[test]
fn speed_bounds() {
    for raw in [1, 262144] {
        let m = movement_with(|m| m["speed"]["value"] = json!(raw));
        read_ok(&ruleset_with(m, &[]));
    }
    assert_eq!(MAX_SPEED_PER_TICK_RAW, 262144);
    for raw in [0, -1, 262145] {
        let m = movement_with(|m| m["speed"]["value"] = json!(raw));
        exact(
            read(&ruleset_with(m, &[])),
            "ruleset movement speed must be 1..=262144 raw",
        );
    }
    let pace = |name: &str| movement_with(|m| m["speed"] = json!({"type":"stat","value":name}));
    read_ok(&ruleset_with(pace("pace"), &[fixed_stat("pace")]));
    for name in ["might", "nope", ""] {
        exact(
            read(&ruleset_with(pace(name), &[fixed_stat("pace")])),
            "ruleset movement speed stat must name a declared fixed stat",
        );
    }
    let derived = json!({
        "kind": "fixed", "name": "pace2",
        "derived": { "type": "literal", "value": { "type": "fixed", "value": 65536 } }
    });
    read_ok(&ruleset_with(pace("pace2"), &[derived]));
}

// B5 / M-5.
#[test]
fn pool_must_be_a_template() {
    for pool in [POOL_1E, POOL_1F] {
        let m = movement_with(|m| m["turn"]["pool"] = json!(pool));
        read_ok(&ruleset_with(m, &[]));
    }
    let m = movement_with(|m| m["turn"]["pool"] = json!(ABILITY_17));
    exact(
        read(&ruleset_with(m, &[])),
        "ruleset movement pool must be a pool template of this ruleset",
    );
}

// B6 / M-6.
#[test]
fn step_and_cost() {
    let m = movement_with(|m| m["turn"]["step"]["value"] = json!(1));
    read_ok(&ruleset_with(m, &[]));
    for raw in [0, -65536] {
        let m = movement_with(|m| m["turn"]["step"]["value"] = json!(raw));
        exact(
            read(&ruleset_with(m, &[])),
            "ruleset movement step must be positive",
        );
    }
    let m = movement_with(|m| m["turn"]["step"] = json!({"type":"stat","value":"might"}));
    exact(
        read(&ruleset_with(m, &[])),
        "ruleset movement step stat must name a declared fixed stat",
    );
    let m = movement_with(|m| m["turn"]["step"] = json!({"type":"stat","value":"stride"}));
    read_ok(&ruleset_with(m, &[fixed_stat("stride")]));
    let m = movement_with(|m| m["turn"]["step_cost"] = json!(0));
    exact(
        read(&ruleset_with(m, &[])),
        "ruleset movement step_cost must be at least 1",
    );
    let m = movement_with(|m| m["turn"]["step_cost"] = json!(4294967295_u32));
    read_ok(&ruleset_with(m, &[]));
}

// B4-B6 order and call order / M-7.
#[test]
fn first_failure_order() {
    let m = movement_with(|m| {
        m["speed"]["value"] = json!(0);
        m["turn"]["step_cost"] = json!(0);
    });
    exact(
        read(&ruleset_with(m, &[])),
        "ruleset movement speed must be 1..=262144 raw",
    );
    let m = movement_with(|m| {
        m["turn"]["pool"] = json!(ABILITY_17);
        m["turn"]["step"]["value"] = json!(0);
    });
    exact(
        read(&ruleset_with(m, &[])),
        "ruleset movement pool must be a pool template of this ruleset",
    );
    let m = movement_with(|m| {
        m["turn"]["step"]["value"] = json!(0);
        m["turn"]["step_cost"] = json!(0);
    });
    exact(
        read(&ruleset_with(m, &[])),
        "ruleset movement step must be positive",
    );
    let bad_enum = json!({
        "kind": { "enum": { "enum_id": "", "variants": ["a"] } },
        "name": "mood"
    });
    let m = movement_with(|m| m["speed"]["value"] = json!(0));
    exact(
        read(&ruleset_with(m, &[bad_enum])),
        "ruleset enum stat enum_id must be nonempty",
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

fn srd_campaign() -> LoadedCampaign {
    let engine: semver::Version = "0.1.0".parse().unwrap();
    crpg_data::load_campaign(&srd_files(), &engine).expect("combat_srd loads")
}

fn validate_docs(mutate: impl FnOnce(&mut BTreeMap<SourcePath, Document>)) -> Vec<Diagnostic> {
    let mut docs = srd_campaign().documents;
    mutate(&mut docs);
    crpg_data::validate(&LoadedCampaign {
        documents: docs,
        index: BTreeMap::new(),
    })
}

fn ruleset_mut(docs: &mut BTreeMap<SourcePath, Document>) -> &mut crpg_data::Ruleset {
    docs.values_mut()
        .find_map(|doc| match doc {
            Document::Ruleset(ruleset) => Some(ruleset),
            _ => None,
        })
        .unwrap()
}

fn creature_mut<'a>(
    docs: &'a mut BTreeMap<SourcePath, Document>,
    id: &str,
) -> &'a mut crpg_data::Creature {
    let id = ulid(id);
    docs.values_mut()
        .find_map(|doc| match doc {
            Document::Creature(creature) if creature.id == id => Some(creature),
            _ => None,
        })
        .unwrap()
}

fn movement_wire() -> MovementWire {
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

// B8 / M-8.
#[test]
fn pool_edge_in_introspection() {
    let mut campaign = srd_campaign();
    for doc in campaign.documents.values_mut() {
        if let Document::Ruleset(ruleset) = doc {
            ruleset.movement = Some(movement_wire());
        }
    }
    let bytes = crpg_data::explain_object(&campaign, ulid("00000000000000000000000016"))
        .unwrap()
        .expect("ruleset is found");
    let report: Value = serde_json::from_slice(&bytes).unwrap();
    let outbound = report["outbound"].as_array().unwrap();
    assert_eq!(outbound.len(), 3, "{outbound:?}");
    assert_eq!(
        outbound[2],
        json!({
            "file": "rulesets/srd-lite.json",
            "pointer": "/movement/turn/pool",
            "source": "00000000000000000000000016",
            "target": POOL_1F,
            "target_location": null
        })
    );
    assert_eq!(crpg_data::validate(&campaign), Vec::new());
}

/// combat_srd with `pace`/`stride` declared and held by both creatures.
fn movement_setup(docs: &mut BTreeMap<SourcePath, Document>, speed: LengthWire, step: LengthWire) {
    let ruleset = ruleset_mut(docs);
    for name in ["pace", "stride"] {
        ruleset.stats.push(StatDecl {
            name: name.to_string(),
            kind: StatKindWire::Fixed,
            derived: None,
        });
    }
    ruleset.movement = Some(MovementWire {
        speed,
        measure: None,
        turn: TurnMovementWire {
            pool: ulid(POOL_1F),
            step,
            step_cost: 1,
        },
    });
    for id in [HERO, "00000000000000000000000015"] {
        let creature = creature_mut(docs, id);
        creature
            .stats
            .insert("pace".into(), Fx16_16::from_raw(65536));
        creature
            .stats
            .insert("stride".into(), Fx16_16::from_raw(65536));
    }
}

fn hero_with(pace: i32, stride: i32, speed: LengthWire, step: LengthWire) -> Vec<Diagnostic> {
    validate_docs(|docs| {
        movement_setup(docs, speed, step);
        let hero = creature_mut(docs, HERO);
        hero.stats.insert("pace".into(), Fx16_16::from_raw(pace));
        hero.stats
            .insert("stride".into(), Fx16_16::from_raw(stride));
    })
}

fn speed_finding() -> Diagnostic {
    Diagnostic {
        file: Some("encounters/first-blood.json".parse().unwrap()),
        pointer: "/participants/0/placement".into(),
        severity: Severity::Error,
        code: DiagnosticCode::InvalidStatValue,
        message: format!("movement speed stat \"pace\" on creature {HERO} must be 1..=262144 raw"),
        suggested_fix: Some("author a speed above zero and at most 4 metres per tick".into()),
    }
}

fn step_finding() -> Diagnostic {
    Diagnostic {
        file: Some("encounters/first-blood.json".parse().unwrap()),
        pointer: "/participants/0/placement".into(),
        severity: Severity::Error,
        code: DiagnosticCode::InvalidStatValue,
        message: format!("movement step stat \"stride\" on creature {HERO} must be positive"),
        suggested_fix: Some("author a positive step length".into()),
    }
}

// B9, B10 / M-9.
#[test]
fn encounter_creature_values() {
    let (speed, step) = (stat("pace"), stat("stride"));
    let run = |pace, stride| hero_with(pace, stride, speed.clone(), step.clone());
    assert_eq!(run(65536, 65536), Vec::new());
    assert_eq!(run(0, 65536), vec![speed_finding()]);
    assert_eq!(run(262145, 65536), vec![speed_finding()]);
    assert_eq!(run(262144, 65536), Vec::new());
    assert_eq!(run(65536, 0), vec![step_finding()]);
    assert_eq!(run(65536, -1), vec![step_finding()]);
    // A step-only stat above the speed ceiling is a valid step.
    assert_eq!(run(65536, 1 << 20), Vec::new());
    // One stat used for both: the speed finding wins and is the only one.
    assert_eq!(
        hero_with(0, 65536, stat("pace"), stat("pace")),
        vec![speed_finding()]
    );
    // A missing stat is `missing_stat` only, never a movement finding.
    let lacking = validate_docs(|docs| {
        movement_setup(docs, speed.clone(), step.clone());
        creature_mut(docs, HERO).stats.remove("pace");
    });
    assert_eq!(lacking.len(), 1, "{lacking:?}");
    assert_eq!(lacking[0].code, DiagnosticCode::MissingStat);
    assert!(!lacking[0].message.contains("movement"));
    // Control: without movement the same creature values give no finding.
    let control = validate_docs(|docs| {
        movement_setup(docs, speed.clone(), step.clone());
        ruleset_mut(docs).movement = None;
        let hero = creature_mut(docs, HERO);
        hero.stats.insert("pace".into(), Fx16_16::from_raw(0));
        hero.stats.insert("stride".into(), Fx16_16::from_raw(-1));
    });
    assert_eq!(control, Vec::new());
}

fn collect_spellings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                match (key.as_str(), inner) {
                    ("const", Value::String(text)) => out.push(text.clone()),
                    ("enum", Value::Array(items)) => {
                        for item in items {
                            if let Value::String(text) = item {
                                out.push(text.clone());
                            }
                        }
                    }
                    _ => {}
                }
                collect_spellings(inner, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| collect_spellings(item, out)),
        _ => {}
    }
}

// B11 / M-10.
#[test]
fn schema_holds_the_block() {
    let schemas = crpg_data::generated_schemas().unwrap();
    let generated = &schemas["ruleset.schema.json"];
    let root: Value = serde_json::from_slice(generated).unwrap();
    let shape = &root["oneOf"][0];
    assert!(shape["properties"]
        .as_object()
        .unwrap()
        .contains_key("movement"));
    assert!(!shape["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry == "movement"));
    for def in [
        "MovementWire",
        "TurnMovementWire",
        "LengthWire",
        "MeasureWire",
    ] {
        assert!(root["$defs"].get(def).is_some(), "$defs lacks {def}");
    }
    let mut spellings = Vec::new();
    collect_spellings(&root["$defs"]["MeasureWire"], &mut spellings);
    assert_eq!(spellings, vec!["euclidean".to_string()]);
    let checked_in = std::fs::read(repo_root().join("schemas/ruleset.schema.json")).unwrap();
    assert_eq!(generated, &checked_in, "checked-in schema drifted");
    let current = crpg_data::schema_versions()
        .iter()
        .find(|entry| entry.schema_type == "crpg.ruleset")
        .map(|entry| entry.current);
    assert_eq!(current, Some(2));
}
