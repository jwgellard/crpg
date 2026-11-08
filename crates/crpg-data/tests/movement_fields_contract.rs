#![forbid(unsafe_code)]
//! Two-keys contract tests for T148 (the optional `movement` block on
//! `crpg.ruleset/2`), written from `tasks/T148.md` alone, including its
//! "Readings of the ADR" (D1-D9) and the D48 amendment. Test names carry the
//! Behaviour item (B1-B11) or reading (D1-D9) they pin. Integers only: every
//! JSON number that is not an integer is built from text.
use crpg_core::{Fx16_16, Ulid};
use crpg_data::{
    DataError, Diagnostic, DiagnosticCode, Document, LengthWire, LoadedCampaign, MeasureWire,
    MovementWire, Ruleset, Severity, SourcePath, TurnMovementWire, MAX_SPEED_PER_TICK_RAW,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const POOL_1E: &str = "0000000000000000000000001E";
const POOL_1F: &str = "0000000000000000000000001F";
const ABILITY_17: &str = "00000000000000000000000017";
const RULESET_16: &str = "00000000000000000000000016";
const HERO: &str = "00000000000000000000000014";
const GOBLIN: &str = "00000000000000000000000015";

const SPEED_MSG: &str = "ruleset movement speed must be 1..=262144 raw";
const SPEED_STAT_MSG: &str = "ruleset movement speed stat must name a declared fixed stat";
const POOL_MSG: &str = "ruleset movement pool must be a pool template of this ruleset";
const STEP_MSG: &str = "ruleset movement step must be positive";
const STEP_STAT_MSG: &str = "ruleset movement step stat must name a declared fixed stat";
const COST_MSG: &str = "ruleset movement step_cost must be at least 1";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn json_file(relative: &str) -> Value {
    serde_json::from_slice(&std::fs::read(repo_root().join(relative)).unwrap()).unwrap()
}

fn srd_ruleset() -> Value {
    json_file("rulesets/srd-lite/ruleset.json")
}

/// A JSON value from text, so that non-integer numbers never appear as Rust
/// literals.
fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn ulid(text: &str) -> Ulid {
    text.parse().unwrap()
}

/// The contract's `WIRE` example.
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

fn derived_pace2() -> Value {
    json!({
        "kind": "fixed",
        "name": "pace2",
        "derived": { "type": "literal", "value": { "type": "fixed", "value": 65536 } }
    })
}

/// srd-lite plus extra stats and the given `movement` value.
fn ruleset_with(movement: Value, extra_stats: &[Value]) -> Value {
    let mut out = srd_ruleset();
    for stat in extra_stats {
        out["stats"].as_array_mut().unwrap().push(stat.clone());
    }
    out["movement"] = movement;
    out
}

/// The wire example after a mutation, on srd-lite with a fixed stat "pace".
fn case(mutate: impl FnOnce(&mut Value)) -> Value {
    let mut movement = wire();
    mutate(&mut movement);
    ruleset_with(movement, &[fixed_stat("pace")])
}

/// Removes the member at a JSON pointer.
fn del(value: &mut Value, pointer: &str) {
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    value
        .pointer_mut(parent)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove(key);
}

fn read(value: &Value) -> Result<Document, DataError> {
    crpg_data::read_document(&serde_json::to_vec(value).unwrap())
}

fn ruleset_of(doc: Document) -> Ruleset {
    match doc {
        Document::Ruleset(ruleset) => ruleset,
        other => panic!("expected ruleset, got {other:?}"),
    }
}

fn read_ruleset(value: &Value) -> Ruleset {
    ruleset_of(read(value).unwrap_or_else(|err| panic!("should read: {err}\n{value}")))
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

fn written_of(value: &Value) -> String {
    written(&Document::Ruleset(read_ruleset(value)))
}

fn constant(raw: i32) -> LengthWire {
    LengthWire::Constant(Fx16_16::from_raw(raw))
}

// ---------------------------------------------------------------- B1

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

/// B1 (M-1): every ruleset document in the repository decodes with
/// `movement == None`; every one already at `crpg.ruleset/2` re-encodes to its
/// exact bytes, and no written ruleset contains the text `"movement"`.
#[test]
fn b1_absent_movement_is_unchanged() {
    let mut files = Vec::new();
    for dir in [
        "rulesets",
        "campaigns/fixtures",
        "crates/crpg-data/tests/fixtures",
    ] {
        collect_json(&repo_root().join(dir), &mut files);
    }
    let (mut at_two, mut seen) = (0, 0);
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let Some(schema) = value.get("schema").and_then(Value::as_str) else {
            continue;
        };
        if !schema.starts_with("crpg.ruleset/") {
            continue;
        }
        seen += 1;
        let doc = crpg_data::read_document(&bytes)
            .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        let Document::Ruleset(ruleset) = &doc else {
            panic!("{}: expected ruleset", path.display());
        };
        assert!(ruleset.movement.is_none(), "{}", path.display());
        let out = crpg_data::write_document(&doc).unwrap();
        assert!(
            !String::from_utf8(out.clone())
                .unwrap()
                .contains("\"movement\""),
            "{}",
            path.display()
        );
        if schema == "crpg.ruleset/2" {
            at_two += 1;
            assert_eq!(out, bytes, "{} re-encodes unchanged", path.display());
        }
    }
    assert!(seen >= 4, "found only {seen} ruleset files");
    assert!(at_two >= 2, "found only {at_two} ruleset/2 files");
}

/// B1: an old ruleset without `movement` loads, has no movement and
/// round-trips unchanged (read, write, read again).
#[test]
fn b1_old_ruleset_round_trips_unchanged() {
    let bytes = std::fs::read(repo_root().join("rulesets/srd-lite/ruleset.json")).unwrap();
    let doc = crpg_data::read_document(&bytes).unwrap();
    assert_eq!(ruleset_of(doc.clone()).movement, None);
    let out = crpg_data::write_document(&doc).unwrap();
    assert_eq!(out, bytes);
    assert_eq!(crpg_data::read_document(&out).unwrap(), doc);
    assert!(!written(&doc).contains("movement"));
}

// ---------------------------------------------------------------- B2

/// B2 (M-2): the wire example reads into exactly the contract's literal.
#[test]
fn b2_wire_example_reads_to_the_literal() {
    let ruleset = read_ruleset(&ruleset_with(wire(), &[]));
    let want = MovementWire {
        speed: constant(16384),
        measure: Some(MeasureWire::Euclidean),
        turn: TurnMovementWire {
            pool: ulid(POOL_1F),
            step: constant(98304),
            step_cost: 1,
        },
    };
    assert_eq!(ruleset.movement.as_deref(), Some(&want));
}

/// B2 and D2 (M-2): write then read is equal; the written movement block has
/// exactly the contract's wire shape, with canonically sorted keys.
#[test]
fn b2_wire_example_round_trips_with_exact_json() {
    let doc = read(&ruleset_with(wire(), &[])).unwrap();
    let text = written(&doc);
    assert!(text.contains("\"measure\": \"euclidean\""), "{text}");
    let back = crpg_data::read_document(text.as_bytes()).unwrap();
    assert_eq!(back, doc);
    assert_eq!(written(&back), text);
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        value["movement"],
        json!({
            "measure": "euclidean",
            "speed": { "type": "constant", "value": 16384 },
            "turn": {
                "pool": POOL_1F,
                "step": { "type": "constant", "value": 98304 },
                "step_cost": 1
            }
        })
    );
    // Canonical key sort puts `movement` between `id` and `name`.
    let at = |key: &str| text.find(&format!("\n  \"{key}\":")).unwrap();
    assert!(at("id") < at("movement") && at("movement") < at("name"));
}

/// B2: `measure` absent stays absent.
#[test]
fn b2_absent_measure_stays_absent() {
    let mut movement = wire();
    del(&mut movement, "/measure");
    let value = ruleset_with(movement, &[]);
    let ruleset = read_ruleset(&value);
    assert_eq!(ruleset.movement.as_ref().unwrap().measure, None);
    let text = written_of(&value);
    assert!(!text.contains("measure"), "{text}");
    let back: Value = serde_json::from_str(&text).unwrap();
    assert!(back["movement"].get("measure").is_none());
    assert!(back["movement"].get("speed").is_some());
}

/// B2 and D3: `"movement": null` reads as absent and writes no key.
#[test]
fn b2_null_movement_reads_as_absent() {
    let mut value = srd_ruleset();
    value["movement"] = Value::Null;
    let ruleset = read_ruleset(&value);
    assert_eq!(ruleset.movement, None);
    let text = written_of(&value);
    assert!(!text.contains("movement"), "{text}");
}

/// B2 and D3: `"measure": null` reads as absent and writes no key.
#[test]
fn b2_null_measure_reads_as_absent() {
    let mut movement = wire();
    movement["measure"] = Value::Null;
    let value = ruleset_with(movement, &[]);
    let ruleset = read_ruleset(&value);
    assert_eq!(ruleset.movement.as_ref().unwrap().measure, None);
    let text = written_of(&value);
    assert!(!text.contains("measure"), "{text}");
    assert!(text.contains("\"movement\""));
}

/// B2 and D2: a stat-valued speed and step round-trip in the exact
/// `{"type":"stat","value":<name>}` shape.
#[test]
fn b2_stat_lengths_round_trip() {
    let movement = json!({
        "speed": { "type": "stat", "value": "pace" },
        "turn": {
            "pool": POOL_1E,
            "step": { "type": "stat", "value": "stride" },
            "step_cost": 3
        }
    });
    let value = ruleset_with(
        movement.clone(),
        &[fixed_stat("pace"), fixed_stat("stride")],
    );
    let ruleset = read_ruleset(&value);
    let want = MovementWire {
        speed: LengthWire::Stat("pace".into()),
        measure: None,
        turn: TurnMovementWire {
            pool: ulid(POOL_1E),
            step: LengthWire::Stat("stride".into()),
            step_cost: 3,
        },
    };
    assert_eq!(ruleset.movement.as_deref(), Some(&want));
    let doc = Document::Ruleset(ruleset);
    let text = written(&doc);
    let back: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(back["movement"], movement);
    assert_eq!(crpg_data::read_document(text.as_bytes()).unwrap(), doc);
}

/// D6: a constant step has no upper bound; `step_cost` reaches `u32::MAX`.
#[test]
fn d6_step_constant_and_cost_have_no_upper_bound() {
    let mut movement = wire();
    movement["turn"]["step"]["value"] = json!(i32::MAX);
    movement["turn"]["step_cost"] = json!(u32::MAX);
    let ruleset = read_ruleset(&ruleset_with(movement, &[]));
    let turn = &ruleset.movement.as_ref().unwrap().turn;
    assert_eq!(turn.step, constant(i32::MAX));
    assert_eq!(turn.step_cost, u32::MAX);
}

// ---------------------------------------------------------------- B3

/// B3 (M-3): control, then structural rejections without a stated message.
#[test]
fn b3_control_and_unnamed_rejections() {
    read_ruleset(&case(|_| {}));
    let cases: Vec<(&str, Value)> = vec![
        (
            "constant above i32",
            case(|m| m["speed"]["value"] = parse("2147483648")),
        ),
        (
            "constant below i32",
            case(|m| m["speed"]["value"] = parse("-2147483649")),
        ),
        (
            "step constant above i32",
            case(|m| m["turn"]["step"]["value"] = parse("2147483648")),
        ),
        ("step_cost -1", case(|m| m["turn"]["step_cost"] = json!(-1))),
        (
            "step_cost above u32",
            case(|m| m["turn"]["step_cost"] = parse("4294967296")),
        ),
        (
            "step_cost non-integer",
            case(|m| m["turn"]["step_cost"] = parse("1.5")),
        ),
        (
            "step_cost string",
            case(|m| m["turn"]["step_cost"] = json!("1")),
        ),
        (
            "pool not a ulid",
            case(|m| m["turn"]["pool"] = json!("not-a-ulid")),
        ),
        ("pool a number", case(|m| m["turn"]["pool"] = json!(31))),
        (
            "constant a string",
            case(|m| m["speed"]["value"] = json!("16384")),
        ),
        (
            "stat value a number",
            case(|m| m["speed"] = json!({"type": "stat", "value": 7})),
        ),
        ("length a bare number", case(|m| m["speed"] = json!(16384))),
        ("movement a string", case(|m| *m = json!("fast"))),
        ("turn a string", case(|m| m["turn"] = json!("slow"))),
    ];
    for (name, value) in cases {
        let got = read(&value);
        assert!(
            matches!(got, Err(DataError::Malformed { .. })),
            "{name}: {got:?}"
        );
    }
}

/// B3 (M-3): unknown keys at `movement`, `turn` and a `Length` are rejected
/// with serde's `unknown field` (the `Length` form per the D48 amendment).
#[test]
fn b3_unknown_keys_are_rejected() {
    let movement = message(read(&case(|m| m["speeed"] = json!(1))));
    assert!(movement.contains("unknown field"), "{movement}");
    assert!(movement.contains("speeed"), "{movement}");
    let turn = message(read(&case(|m| m["turn"]["steps"] = json!(1))));
    assert!(turn.contains("unknown field"), "{turn}");
    assert!(turn.contains("steps"), "{turn}");
    // A Length: the adjacently-tagged decoder reports the expected keys (D48).
    let lengths = [
        case(|m| m["speed"]["extra"] = json!(1)),
        case(|m| m["turn"]["step"]["extra"] = json!(1)),
        case(|m| m["speed"] = json!({"type": "stat", "value": "pace", "extra": 1})),
    ];
    for value in lengths {
        let got = message(read(&value));
        assert!(
            got.contains("extra") && got.contains("expected \"type\" or \"value\""),
            "{got}"
        );
    }
}

/// B3 (M-3): missing required keys are `missing field` errors.
#[test]
fn b3_missing_keys_are_rejected() {
    for pointer in [
        "/speed",
        "/turn",
        "/turn/pool",
        "/turn/step",
        "/turn/step_cost",
    ] {
        let got = message(read(&case(|m| del(m, pointer))));
        assert!(got.contains("missing field"), "{pointer}: {got}");
        let key = pointer.rsplit('/').next().unwrap();
        assert!(got.contains(key), "{pointer}: {got}");
    }
    for pointer in ["/speed/value", "/turn/step/value"] {
        let got = message(read(&case(|m| del(m, pointer))));
        assert!(got.contains("missing field"), "{pointer}: {got}");
    }
    // A Length without its `type` tag is also malformed.
    for pointer in ["/speed/type", "/turn/step/type"] {
        let got = read(&case(|m| del(m, pointer)));
        assert!(
            matches!(got, Err(DataError::Malformed { .. })),
            "{pointer}: {got:?}"
        );
    }
}

/// B3 (M-3): a `Length` type other than constant or stat is serde's
/// `unknown variant`.
#[test]
fn b3_unknown_length_type_is_rejected() {
    for target in ["speed", "step"] {
        let got = message(read(&case(|m| {
            let length = json!({"type": "metres", "value": 1});
            if target == "speed" {
                m["speed"] = length;
            } else {
                m["turn"]["step"] = length;
            }
        })));
        assert!(got.contains("unknown variant"), "{target}: {got}");
        assert!(got.contains("metres"), "{target}: {got}");
    }
}

/// B3 (M-3): a fractional constant is `Malformed`; the D48 amendment pins the
/// strict parser's message.
#[test]
fn b3_fractional_constant_is_rejected() {
    // Built from text: the bytes carry a real JSON fraction.
    let text = serde_json::to_string(&case(|m| m["speed"]["value"] = json!(0))).unwrap();
    let text = text.replace(
        "\"speed\":{\"type\":\"constant\",\"value\":0}",
        "\"speed\":{\"type\":\"constant\",\"value\":1.5}",
    );
    assert!(text.contains("1.5"));
    let got = message(crpg_data::read_document(text.as_bytes()));
    assert!(
        got.contains("expected JSON containing only integer numbers"),
        "{got}"
    );
}

/// B3 (M-3): `measure` message texts are exact.
#[test]
fn b3_measure_messages_are_exact() {
    for bad in [
        json!(1),
        json!({"type": "euclidean"}),
        json!(true),
        json!([]),
    ] {
        let got = message(read(&case(|m| m["measure"] = bad.clone())));
        assert_eq!(got, "measure must be a measure name", "{bad}");
    }
    for text in ["manhattan", "Euclidean", ""] {
        let got = message(read(&case(|m| m["measure"] = json!(text))));
        assert_eq!(
            got,
            format!("unknown variant `{text}`, expected `euclidean`")
        );
    }
}

fn load_with_movement_edit(edit: impl FnOnce(&mut Value)) -> (SourcePath, DataError) {
    let mut files = srd_files();
    let path: SourcePath = "rulesets/srd-lite.json".parse().unwrap();
    let mut value: Value = serde_json::from_slice(&files[&path]).unwrap();
    let mut movement = wire();
    edit(&mut movement);
    value["movement"] = movement;
    files.insert(path.clone(), serde_json::to_vec_pretty(&value).unwrap());
    let engine: semver::Version = "0.1.0".parse().unwrap();
    match crpg_data::load_campaign(&files, &engine) {
        Err(err) => (path, err),
        Ok(_) => panic!("expected the campaign to be rejected"),
    }
}

/// B3, D5: the same structural rejection comes from `load_campaign`, with the
/// ruleset's logical path attached.
#[test]
fn b3_load_campaign_reports_malformed() {
    let (path, err) = load_with_movement_edit(|m| m["measure"] = json!("manhattan"));
    match err {
        DataError::Malformed {
            path: Some(found),
            message,
        } => {
            assert_eq!(found, path);
            assert_eq!(message, "unknown variant `manhattan`, expected `euclidean`");
        }
        other => panic!("expected Malformed, got {other:?}"),
    }
}

/// B4, D5: a read-time rule is `Malformed` through `load_campaign` too.
#[test]
fn b4_load_campaign_reports_read_time_rule() {
    let (path, err) = load_with_movement_edit(|m| m["speed"]["value"] = json!(0));
    match err {
        DataError::Malformed {
            path: Some(found),
            message,
        } => {
            assert_eq!(found, path);
            assert_eq!(message, SPEED_MSG);
        }
        other => panic!("expected Malformed, got {other:?}"),
    }
}

// ---------------------------------------------------------------- B4

/// D9: the data constant is 4 metres per tick in raw `Fx16_16`.
#[test]
fn d9_speed_constant_is_one_shift_eighteen() {
    assert_eq!(MAX_SPEED_PER_TICK_RAW, 262_144);
    assert_eq!(MAX_SPEED_PER_TICK_RAW, 1 << 18);
}

/// B4 (M-4): constant speed is within `1..=MAX_SPEED_PER_TICK_RAW`; both sides
/// of both limits.
#[test]
fn b4_constant_speed_bounds() {
    for ok in [1, 2, 16384, 262_143, 262_144] {
        let ruleset = read_ruleset(&case(|m| m["speed"]["value"] = json!(ok)));
        assert_eq!(ruleset.movement.as_ref().unwrap().speed, constant(ok));
    }
    for bad in [0, -1, -65536, 262_145, 262_146, 1 << 20, i32::MAX, i32::MIN] {
        let got = message(read(&case(|m| m["speed"]["value"] = json!(bad))));
        assert_eq!(got, SPEED_MSG, "speed {bad}");
    }
}

/// B4 (M-4): a stat speed must name a declared fixed stat.
#[test]
fn b4_stat_speed_must_be_a_declared_fixed_stat() {
    read_ruleset(&case(|m| {
        m["speed"] = json!({"type": "stat", "value": "pace"})
    }));
    for bad in ["might", "nope", "", "Pace", "health"] {
        let got = message(read(&case(|m| {
            m["speed"] = json!({"type": "stat", "value": bad});
        })));
        assert_eq!(got, SPEED_STAT_MSG, "stat {bad:?}");
    }
}

/// B4, D5 (M-4): a derived fixed stat is allowed as a speed (and a step).
#[test]
fn b4_derived_fixed_stat_is_allowed() {
    let mut movement = wire();
    movement["speed"] = json!({"type": "stat", "value": "pace2"});
    movement["turn"]["step"] = json!({"type": "stat", "value": "pace2"});
    let ruleset = read_ruleset(&ruleset_with(movement, &[derived_pace2()]));
    let movement = ruleset.movement.as_ref().unwrap();
    assert_eq!(movement.speed, LengthWire::Stat("pace2".into()));
    assert_eq!(movement.turn.step, LengthWire::Stat("pace2".into()));
}

// ---------------------------------------------------------------- B5

/// B5 (M-5): the pool is a template of this ruleset: 1E and 1F read.
#[test]
fn b5_pool_templates_read() {
    for pool in [POOL_1E, POOL_1F] {
        let ruleset = read_ruleset(&case(|m| m["turn"]["pool"] = json!(pool)));
        assert_eq!(ruleset.movement.as_ref().unwrap().turn.pool, ulid(pool));
    }
}

/// B5, D4 (M-5): an ability id, the ruleset's own id and an unknown id are not
/// pool templates.
#[test]
fn b5_non_template_pool_is_rejected() {
    for pool in [
        ABILITY_17,
        RULESET_16,
        "00000000000000000000000000",
        "0000000000000000000000002F",
    ] {
        let got = message(read(&case(|m| m["turn"]["pool"] = json!(pool))));
        assert_eq!(got, POOL_MSG, "pool {pool}");
    }
}

// ---------------------------------------------------------------- B6

/// B6 (M-6): step bounds, both sides of zero, for a constant step.
#[test]
fn b6_constant_step_must_be_positive() {
    for ok in [1, 2, 98304, i32::MAX] {
        let ruleset = read_ruleset(&case(|m| m["turn"]["step"]["value"] = json!(ok)));
        assert_eq!(ruleset.movement.as_ref().unwrap().turn.step, constant(ok));
    }
    for bad in [0, -1, -65536, i32::MIN] {
        let got = message(read(&case(|m| m["turn"]["step"]["value"] = json!(bad))));
        assert_eq!(got, STEP_MSG, "step {bad}");
    }
}

/// B6 (M-6): a stat step must name a declared fixed stat; derived is allowed.
#[test]
fn b6_stat_step_must_be_a_declared_fixed_stat() {
    read_ruleset(&case(|m| {
        m["turn"]["step"] = json!({"type": "stat", "value": "pace"});
    }));
    for bad in ["might", "nope", ""] {
        let got = message(read(&case(|m| {
            m["turn"]["step"] = json!({"type": "stat", "value": bad});
        })));
        assert_eq!(got, STEP_STAT_MSG, "stat {bad:?}");
    }
}

/// B6 (M-6): `step_cost` is at least one, up to `u32::MAX`.
#[test]
fn b6_step_cost_is_at_least_one() {
    for ok in [1_u32, 2, u32::MAX] {
        let ruleset = read_ruleset(&case(|m| m["turn"]["step_cost"] = json!(ok)));
        assert_eq!(ruleset.movement.as_ref().unwrap().turn.step_cost, ok);
    }
    let got = message(read(&case(|m| m["turn"]["step_cost"] = json!(0))));
    assert_eq!(got, COST_MSG);
}

/// B7: `measure` adds no read-time rule; its one value is accepted.
#[test]
fn b7_measure_adds_no_read_time_rule() {
    let a = read_ruleset(&case(|m| m["measure"] = json!("euclidean")));
    assert_eq!(
        a.movement.as_ref().unwrap().measure,
        Some(MeasureWire::Euclidean)
    );
}

// ---------------------------------------------------------------- order

/// B4-B6 order (M-7): the first failure is returned.
#[test]
fn first_failure_order() {
    // speed before step_cost
    let got = message(read(&case(|m| {
        m["speed"]["value"] = json!(0);
        m["turn"]["step_cost"] = json!(0);
    })));
    assert_eq!(got, SPEED_MSG);
    // speed stat before pool and step
    let got = message(read(&case(|m| {
        m["speed"] = json!({"type": "stat", "value": "nope"});
        m["turn"]["pool"] = json!(ABILITY_17);
        m["turn"]["step"]["value"] = json!(0);
    })));
    assert_eq!(got, SPEED_STAT_MSG);
    // pool before step
    let got = message(read(&case(|m| {
        m["turn"]["pool"] = json!(ABILITY_17);
        m["turn"]["step"]["value"] = json!(0);
    })));
    assert_eq!(got, POOL_MSG);
    // pool before step_cost
    let got = message(read(&case(|m| {
        m["turn"]["pool"] = json!(ABILITY_17);
        m["turn"]["step_cost"] = json!(0);
    })));
    assert_eq!(got, POOL_MSG);
    // step before step_cost
    let got = message(read(&case(|m| {
        m["turn"]["step"]["value"] = json!(0);
        m["turn"]["step_cost"] = json!(0);
    })));
    assert_eq!(got, STEP_MSG);
    // step stat before step_cost
    let got = message(read(&case(|m| {
        m["turn"]["step"] = json!({"type": "stat", "value": "nope"});
        m["turn"]["step_cost"] = json!(0);
    })));
    assert_eq!(got, STEP_STAT_MSG);
}

/// Call order (M-7): the existing kernel checks run before the movement ones.
#[test]
fn kernel_message_precedes_movement_message() {
    let mut value = case(|m| m["speed"]["value"] = json!(0));
    value["stats"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "stance", "kind": {"enum": {"enum_id": "", "variants": ["calm"]}}}));
    let got = message(read(&value));
    assert_eq!(got, "ruleset enum stat enum_id must be nonempty");
}

// ---------------------------------------------------------------- campaigns

fn walk_files(dir: &Path, root: &Path, out: &mut BTreeMap<SourcePath, Vec<u8>>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk_files(&path, root, out);
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

fn fixture_files(name: &str) -> BTreeMap<SourcePath, Vec<u8>> {
    let root = repo_root().join("campaigns/fixtures").join(name);
    let mut out = BTreeMap::new();
    walk_files(&root, &root, &mut out);
    out
}

fn srd_files() -> BTreeMap<SourcePath, Vec<u8>> {
    fixture_files("combat_srd")
}

fn load_srd() -> LoadedCampaign {
    let engine: semver::Version = "0.1.0".parse().unwrap();
    crpg_data::load_campaign(&srd_files(), &engine).expect("combat_srd loads")
}

type StatEdits<'a> = &'a [(&'a str, Option<i32>)];

/// combat_srd in memory with the ruleset replaced by `ruleset` (read through
/// `read_document`) and creature stats overridden: `Some` sets, `None` removes.
fn campaign_with(ruleset: &Value, hero: StatEdits, goblin: StatEdits) -> LoadedCampaign {
    let mut campaign = load_srd();
    for doc in campaign.documents.values_mut() {
        match doc {
            Document::Ruleset(slot) => *slot = read_ruleset(ruleset),
            Document::Creature(creature) => {
                let edits = if creature.id == ulid(HERO) {
                    hero
                } else if creature.id == ulid(GOBLIN) {
                    goblin
                } else {
                    continue;
                };
                for (name, value) in edits {
                    match value {
                        Some(raw) => {
                            creature
                                .stats
                                .insert((*name).to_string(), Fx16_16::from_raw(*raw));
                        }
                        None => {
                            creature.stats.remove(*name);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    campaign
}

// ---------------------------------------------------------------- B8

fn outbound_of_ruleset(campaign: &LoadedCampaign) -> Vec<Value> {
    let bytes = crpg_data::explain_object(campaign, ulid(RULESET_16))
        .unwrap()
        .expect("ruleset is an object");
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    value["outbound"].as_array().unwrap().clone()
}

/// B8, D8 (M-8): the movement pool is an inventoried outbound edge of the
/// ruleset, and yields no finding.
#[test]
fn b8_pool_edge_in_introspection() {
    let campaign = campaign_with(&ruleset_with(wire(), &[]), &[], &[]);
    let outbound = outbound_of_ruleset(&campaign);
    assert_eq!(outbound.len(), 3, "{outbound:?}");
    assert_eq!(
        outbound[2],
        json!({
            "file": "rulesets/srd-lite.json",
            "pointer": "/movement/turn/pool",
            "source": RULESET_16,
            "target": POOL_1F,
            "target_location": null
        })
    );
    assert_eq!(crpg_data::validate(&campaign), Vec::new());
}

/// B8: the edge follows the declared pool; without `movement` there is none.
#[test]
fn b8_edge_follows_the_pool_and_absence_adds_none() {
    let mut movement = wire();
    movement["turn"]["pool"] = json!(POOL_1E);
    let campaign = campaign_with(&ruleset_with(movement, &[]), &[], &[]);
    let outbound = outbound_of_ruleset(&campaign);
    assert_eq!(outbound.len(), 3);
    assert_eq!(outbound[2]["pointer"], json!("/movement/turn/pool"));
    assert_eq!(outbound[2]["target"], json!(POOL_1E));

    let outbound = outbound_of_ruleset(&load_srd());
    assert_eq!(outbound.len(), 2);
    assert!(outbound
        .iter()
        .all(|edge| edge["pointer"] != json!("/movement/turn/pool")));
}

// ---------------------------------------------------------------- B9

fn length_json(length: &LengthWire) -> Value {
    match length {
        LengthWire::Constant(v) => json!({"type": "constant", "value": v.to_raw()}),
        LengthWire::Stat(name) => json!({"type": "stat", "value": name}),
    }
}

fn movement_setup(speed: &LengthWire, step: &LengthWire) -> Value {
    let movement = json!({
        "speed": length_json(speed),
        "turn": { "pool": POOL_1F, "step": length_json(step), "step_cost": 1 }
    });
    ruleset_with(movement, &[fixed_stat("pace"), fixed_stat("stride")])
}

fn pace_stride() -> Value {
    movement_setup(
        &LengthWire::Stat("pace".into()),
        &LengthWire::Stat("stride".into()),
    )
}

/// Both creatures hold both stats at 65536 unless overridden.
fn findings(ruleset: &Value, hero: StatEdits) -> Vec<Diagnostic> {
    findings_for(ruleset, hero, &[])
}

fn findings_for(ruleset: &Value, hero: StatEdits, goblin: StatEdits) -> Vec<Diagnostic> {
    let mut hero_edits = vec![("pace", Some(65536)), ("stride", Some(65536))];
    hero_edits.extend_from_slice(hero);
    let mut goblin_edits = vec![("pace", Some(65536)), ("stride", Some(65536))];
    goblin_edits.extend_from_slice(goblin);
    crpg_data::validate(&campaign_with(ruleset, &hero_edits, &goblin_edits))
}

fn speed_finding(creature: &str, index: usize) -> Diagnostic {
    Diagnostic {
        file: Some("encounters/first-blood.json".parse().unwrap()),
        pointer: format!("/participants/{index}/placement"),
        severity: Severity::Error,
        code: DiagnosticCode::InvalidStatValue,
        message: format!(
            "movement speed stat \"pace\" on creature {creature} must be 1..=262144 raw"
        ),
        suggested_fix: Some("author a speed above zero and at most 4 metres per tick".into()),
    }
}

fn step_finding(creature: &str, index: usize) -> Diagnostic {
    Diagnostic {
        file: Some("encounters/first-blood.json".parse().unwrap()),
        pointer: format!("/participants/{index}/placement"),
        severity: Severity::Error,
        code: DiagnosticCode::InvalidStatValue,
        message: format!("movement step stat \"stride\" on creature {creature} must be positive"),
        suggested_fix: Some("author a positive step length".into()),
    }
}

/// B9 / M-9 baseline: in-range values give no finding.
#[test]
fn b9_valid_creature_values_give_no_finding() {
    assert_eq!(findings(&pace_stride(), &[]), Vec::new());
    for pace in [1, 262_144] {
        assert_eq!(
            findings(&pace_stride(), &[("pace", Some(pace))]),
            Vec::new(),
            "pace {pace}"
        );
    }
    for stride in [1, i32::MAX] {
        assert_eq!(
            findings(&pace_stride(), &[("stride", Some(stride))]),
            Vec::new(),
            "stride {stride}"
        );
    }
}

/// B9 (M-9): a speed stat of 0, below zero or above `1<<18` is exactly one
/// `invalid_stat_value` at the participant's placement.
#[test]
fn b9_out_of_range_speed_stat() {
    for pace in [0, -1, 262_145, i32::MAX, i32::MIN] {
        assert_eq!(
            findings(&pace_stride(), &[("pace", Some(pace))]),
            vec![speed_finding(HERO, 0)],
            "pace {pace}"
        );
    }
}

/// B9 (M-9): the goblin is participant 1; the finding names its prefab.
#[test]
fn b9_second_participant_is_pointed_at() {
    assert_eq!(
        findings_for(&pace_stride(), &[], &[("pace", Some(0))]),
        vec![speed_finding(GOBLIN, 1)]
    );
    assert_eq!(
        findings_for(&pace_stride(), &[], &[("stride", Some(-5))]),
        vec![step_finding(GOBLIN, 1)]
    );
}

/// B9 (M-9): a step stat of 0 or below is exactly one finding.
#[test]
fn b9_non_positive_step_stat() {
    for stride in [0, -1, i32::MIN] {
        assert_eq!(
            findings(&pace_stride(), &[("stride", Some(stride))]),
            vec![step_finding(HERO, 0)],
            "stride {stride}"
        );
    }
}

/// B9, D6: a step stat has no upper bound, a speed stat has one.
#[test]
fn b9_step_stat_above_speed_limit_is_fine() {
    assert_eq!(
        findings(&pace_stride(), &[("stride", Some(262_145))]),
        Vec::new()
    );
}

/// B9 (M-9): one stat used for both speed and step reports only the speed
/// finding when it is out of the speed range.
#[test]
fn b9_shared_stat_reports_the_speed_finding_only() {
    let stat = LengthWire::Stat("pace".into());
    let shared = movement_setup(&stat, &stat);
    for pace in [0, -3, 262_145] {
        assert_eq!(
            findings(&shared, &[("pace", Some(pace))]),
            vec![speed_finding(HERO, 0)],
            "pace {pace}"
        );
    }
    assert_eq!(findings(&shared, &[("pace", Some(1))]), Vec::new());
}

/// B9: two bad stats on one creature give two findings.
#[test]
fn b9_two_bad_stats_give_two_findings() {
    let mut got = findings(&pace_stride(), &[("pace", Some(0)), ("stride", Some(0))]);
    got.sort_by(|a, b| a.message.cmp(&b.message));
    let mut want = vec![speed_finding(HERO, 0), step_finding(HERO, 0)];
    want.sort_by(|a, b| a.message.cmp(&b.message));
    assert_eq!(got, want);
}

/// B9: only the stat that speed or step names is checked; a constant movement
/// gets no creature finding whatever the creature holds.
#[test]
fn b9_constant_lengths_and_other_stats_get_no_finding() {
    let constants = movement_setup(&constant(16384), &constant(98304));
    assert_eq!(
        findings(&constants, &[("pace", Some(0)), ("stride", Some(-1))]),
        Vec::new()
    );
    // Only speed is a stat: a bad stride is not a movement stat.
    let speed_only = movement_setup(&LengthWire::Stat("pace".into()), &constant(98304));
    assert_eq!(findings(&speed_only, &[("stride", Some(0))]), Vec::new());
    assert_eq!(
        findings(&speed_only, &[("pace", Some(0))]),
        vec![speed_finding(HERO, 0)]
    );
    // Only step is a stat: a bad pace is not a movement stat.
    let step_only = movement_setup(&constant(16384), &LengthWire::Stat("stride".into()));
    assert_eq!(findings(&step_only, &[("pace", Some(0))]), Vec::new());
    assert_eq!(
        findings(&step_only, &[("stride", Some(0))]),
        vec![step_finding(HERO, 0)]
    );
}

/// B9 (M-9): a creature lacking the stat gets exactly one `missing_stat` and no
/// movement finding.
#[test]
fn b9_missing_stat_gets_no_movement_finding() {
    for stat in ["pace", "stride"] {
        let got = findings(&pace_stride(), &[(stat, None)]);
        assert_eq!(got.len(), 1, "{stat}: {got:?}");
        assert_eq!(got[0].code, DiagnosticCode::MissingStat, "{stat}");
        assert_eq!(got[0].severity, Severity::Error, "{stat}");
        assert!(!got[0].message.contains("movement"), "{}", got[0].message);
    }
}

/// B9: a derived stat is not checked against a creature.
#[test]
fn b9_derived_stat_gets_no_movement_finding() {
    let mut movement = wire();
    movement["speed"] = json!({"type": "stat", "value": "pace2"});
    let ruleset = ruleset_with(movement, &[derived_pace2()]);
    assert_eq!(
        crpg_data::validate(&campaign_with(&ruleset, &[], &[])),
        Vec::new()
    );
}

/// B9, B10 (M-9 control): without `movement`, the same creature values give no
/// movement finding, and combat_srd keeps its findings.
#[test]
fn b10_no_movement_no_finding() {
    assert_eq!(crpg_data::validate(&load_srd()), Vec::new());
    let ruleset = ruleset_with(Value::Null, &[fixed_stat("pace"), fixed_stat("stride")]);
    assert_eq!(
        findings(&ruleset, &[("pace", Some(0)), ("stride", Some(-1))]),
        Vec::new()
    );
}

/// B10: combat_basic and combat_srd still load and validate clean.
#[test]
fn b10_existing_campaigns_still_validate_clean() {
    let engine: semver::Version = "0.1.0".parse().unwrap();
    for name in ["combat_basic", "combat_srd"] {
        let campaign = crpg_data::load_campaign(&fixture_files(name), &engine).expect(name);
        assert_eq!(crpg_data::validate(&campaign), Vec::new(), "{name}");
    }
}

// ---------------------------------------------------------------- B11

fn strings_under_const_or_enum(value: &Value, in_enum: bool, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                match key.as_str() {
                    "const" => {
                        if let Some(text) = child.as_str() {
                            out.push(text.to_string());
                        }
                    }
                    "enum" => strings_under_const_or_enum(child, true, out),
                    _ => strings_under_const_or_enum(child, in_enum, out),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                strings_under_const_or_enum(item, in_enum, out);
            }
        }
        Value::String(text) if in_enum => out.push(text.clone()),
        _ => {}
    }
}

/// B11 (M-10): the generated ruleset schema holds an optional `movement`
/// property and the four definitions; `MeasureWire` allows exactly
/// `"euclidean"`.
#[test]
fn b11_schema_holds_the_block() {
    let schemas = crpg_data::generated_schemas().unwrap();
    let root: Value = serde_json::from_slice(&schemas["ruleset.schema.json"]).unwrap();
    let shape = &root["oneOf"][0];
    assert!(shape["properties"]
        .as_object()
        .unwrap()
        .contains_key("movement"));
    let required: Vec<&str> = shape["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry.as_str().unwrap())
        .collect();
    assert!(!required.contains(&"movement"), "{required:?}");
    let defs = root["$defs"].as_object().unwrap();
    for name in [
        "MovementWire",
        "TurnMovementWire",
        "LengthWire",
        "MeasureWire",
    ] {
        assert!(defs.contains_key(name), "$defs lacks {name}");
    }
    let mut strings = Vec::new();
    strings_under_const_or_enum(&defs["MeasureWire"], false, &mut strings);
    assert_eq!(strings, vec!["euclidean".to_string()]);
    // Both Length forms are in the definition.
    let length = serde_json::to_string(&defs["LengthWire"]).unwrap();
    assert!(
        length.contains("constant") && length.contains("stat"),
        "{length}"
    );
    // The members are present.
    let movement = &defs["MovementWire"]["properties"];
    for key in ["speed", "measure", "turn"] {
        assert!(movement.get(key).is_some(), "MovementWire lacks {key}");
    }
    let turn = &defs["TurnMovementWire"]["properties"];
    for key in ["pool", "step", "step_cost"] {
        assert!(turn.get(key).is_some(), "TurnMovementWire lacks {key}");
    }
}

/// B11 (M-10), D1: every generated schema equals its checked-in file (so only
/// `ruleset.schema.json` could have changed), and the registry still holds
/// `crpg.ruleset` at 2.
#[test]
fn b11_checked_in_schemas_match_and_ruleset_stays_at_two() {
    let schemas = crpg_data::generated_schemas().unwrap();
    for (file, bytes) in &schemas {
        let checked_in = std::fs::read(repo_root().join("schemas").join(file))
            .unwrap_or_else(|err| panic!("{file}: {err}"));
        assert_eq!(&checked_in, bytes, "{file} differs from the generator");
    }
    let versions = crpg_data::schema_versions();
    let current = versions
        .iter()
        .find(|entry| entry.schema_type == "crpg.ruleset")
        .map(|entry| entry.current);
    assert_eq!(current, Some(2));
    let mut value = srd_ruleset();
    value["movement"] = wire();
    assert_eq!(value["schema"], json!("crpg.ruleset/2"));
    assert_eq!(written_of(&value).matches("crpg.ruleset/2").count(), 1);
}
