#![forbid(unsafe_code)]
//! T143a: the optional `preview` policy on `crpg.ruleset/2` (A1-A5). The field
//! is a hook only: no version moves, absence means no preview is answered, and
//! every existing document is unchanged.
use crpg_data::{DataError, Document, PreviewWhenWire, PreviewWire, MAX_PREVIEW_COST};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn srd_ruleset() -> Value {
    let bytes = std::fs::read(repo_root().join("rulesets/srd-lite/ruleset.json")).unwrap();
    serde_json::from_slice(&bytes).unwrap()
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

fn written(doc: &Document) -> String {
    String::from_utf8(crpg_data::write_document(doc).unwrap()).unwrap()
}

/// Byte offset of a top-level key (two-space indent) in canonical output.
fn top_key(text: &str, key: &str) -> usize {
    text.find(&format!("\n  \"{key}\":"))
        .unwrap_or_else(|| panic!("canonical output lacks top-level {key}"))
}

fn policy(when: &str, max_cost: Option<u64>) -> Value {
    match max_cost {
        Some(cost) => json!({"when": when, "max_cost": cost}),
        None => json!({"when": when}),
    }
}

fn json_files() -> Vec<PathBuf> {
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
    paths
}

// A1: every existing combat document reads, no ruleset has a preview, and
// no document gains a `preview` key or loses a byte.
#[test]
fn absent_is_none_and_unchanged() {
    let mut rulesets = 0;
    let mut documents = 0;
    let mut current_documents = 0;
    for path in json_files() {
        let bytes = std::fs::read(&path).unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        let is_document = value
            .get("schema")
            .and_then(Value::as_str)
            .is_some_and(|tag| tag.starts_with("crpg."));
        if !is_document {
            continue;
        }
        let tag = value["schema"].as_str().unwrap().to_owned();
        let doc = read_ok(&value);
        documents += 1;
        let text = written(&doc);
        if let Document::Ruleset(ruleset) = &doc {
            rulesets += 1;
            assert_eq!(ruleset.preview, None, "{}", path.display());
        }
        assert!(
            !text.contains("\"preview\""),
            "{} writes preview",
            path.display()
        );
        // Re-encoding a document that is already current is byte-exact; one
        // that migrates is compared with its own second round trip.
        let reread = crpg_data::read_document(text.as_bytes()).unwrap();
        assert_eq!(reread, doc, "{}", path.display());
        assert_eq!(written(&reread), text, "{}", path.display());
        let (family, version) = tag.split_once('/').unwrap();
        let current = crpg_data::schema_versions()
            .iter()
            .find(|entry| entry.schema_type == family)
            .map(|entry| entry.current);
        if Some(version.parse::<u32>().unwrap()) == current {
            current_documents += 1;
            let disk = String::from_utf8(bytes).unwrap().replace("\r\n", "\n");
            assert_eq!(
                text.trim_end_matches('\n'),
                disk.trim_end_matches('\n'),
                "{}",
                path.display()
            );
        }
    }
    // minimal-d6 and srd-lite sources plus both campaign copies.
    assert!(rulesets >= 4, "found {rulesets} rulesets");
    assert!(documents >= 10, "found {documents} documents");
    assert!(
        current_documents >= 5,
        "found {current_documents} current documents"
    );
}

// A2: every `when` and every `max_cost` shape round trips through the canonical
// writer, with `preview` placed by the key sort and `max_cost` omitted when absent.
#[test]
fn each_policy_round_trips() {
    let base = srd_ruleset();
    for (name, when) in [
        ("never", PreviewWhenWire::Never),
        ("actionable", PreviewWhenWire::Actionable),
        ("planning", PreviewWhenWire::Planning),
    ] {
        for max_cost in [None, Some(1), Some(16_777_216)] {
            let ruleset = ruleset_of(read_ok(&with(&base, "preview", policy(name, max_cost))));
            assert_eq!(
                ruleset.preview,
                Some(PreviewWire { when, max_cost }),
                "{name} {max_cost:?}"
            );
            let doc = Document::Ruleset(ruleset);
            let text = written(&doc);
            assert!(
                top_key(&text, "pools") < top_key(&text, "preview"),
                "{text}"
            );
            assert!(top_key(&text, "preview") < top_key(&text, "slug"), "{text}");
            assert!(
                text.contains(&format!("\"when\": \"{name}\"")),
                "{name}: {text}"
            );
            assert_eq!(
                text.contains("\"max_cost\""),
                max_cost.is_some(),
                "{name} {max_cost:?}: {text}"
            );
            if let Some(cost) = max_cost {
                assert!(
                    text.contains(&format!("\"max_cost\": {cost}")),
                    "{name}: {text}"
                );
            }
            assert_eq!(crpg_data::read_document(text.as_bytes()).unwrap(), doc);
        }
    }
    // The value serializes to the same names it decodes from.
    for (when, name) in [
        (PreviewWhenWire::Never, "never"),
        (PreviewWhenWire::Actionable, "actionable"),
        (PreviewWhenWire::Planning, "planning"),
    ] {
        assert_eq!(serde_json::to_value(when).unwrap(), json!(name));
    }
    // `null` reads as absent and is not written back.
    let ruleset = ruleset_of(read_ok(&with(&base, "preview", Value::Null)));
    assert_eq!(ruleset.preview, None);
    assert!(!written(&Document::Ruleset(ruleset)).contains("preview"));
}

// A3: the declared limit is 1..=16777216, off by one either way.
#[test]
fn max_cost_bounds() {
    let base = srd_ruleset();
    for ok in [1_u64, 2, 16_777_215, 16_777_216] {
        let ruleset = ruleset_of(read_ok(&with(
            &base,
            "preview",
            policy("actionable", Some(ok)),
        )));
        assert_eq!(
            ruleset.preview,
            Some(PreviewWire {
                when: PreviewWhenWire::Actionable,
                max_cost: Some(ok)
            })
        );
    }
    for bad in [0_u64, 16_777_217, u64::MAX] {
        for when in ["never", "actionable", "planning"] {
            assert_eq!(
                message(read(&with(&base, "preview", policy(when, Some(bad))))),
                "ruleset preview max_cost must be 1..=16777216",
                "{when} {bad}"
            );
        }
    }
    // The check runs after the existing ones: with a kernel or movement fault
    // and an out-of-range max_cost together, the earlier check's message wins.
    // Each fault alone, and the bad limit alone, are controls.
    let bad_limit = policy("actionable", Some(16_777_217));
    let kernel_fault = {
        let mut out = base.clone();
        out["stats"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "kind": "fixed", "name": "pace" }));
        out["health_stat"] = json!("pace");
        out
    };
    let movement_fault = {
        let mut out = base.clone();
        out["movement"] = json!({
            "speed": { "type": "constant", "value": 16384 },
            "turn": {
                "pool": base["pools"][0]["id"].clone(),
                "step": { "type": "constant", "value": 98304 },
                "step_cost": 0
            }
        });
        out
    };
    for (fault, expected) in [
        (
            &kernel_fault,
            "ruleset health_stat must name a non-derived int stat",
        ),
        (
            &movement_fault,
            "ruleset movement step_cost must be at least 1",
        ),
    ] {
        assert_eq!(message(read(fault)), expected);
        assert_eq!(
            message(read(&with(
                fault,
                "preview",
                policy("actionable", Some(16_777_216))
            ))),
            expected,
            "in-range limit does not mask the earlier fault"
        );
        assert_eq!(
            message(read(&with(fault, "preview", bad_limit.clone()))),
            expected,
            "earlier check wins over the preview bound"
        );
    }
    assert_eq!(
        message(read(&with(&base, "preview", bad_limit))),
        "ruleset preview max_cost must be 1..=16777216"
    );
    // A negative or fractional limit is not a u64 at all.
    for bad in [json!(-1), json!(1.5), json!("100")] {
        assert!(
            read(&with(
                &base,
                "preview",
                json!({"when": "actionable", "max_cost": bad.clone()})
            ))
            .is_err(),
            "max_cost {bad} accepted"
        );
    }
}

// A4: malformed shapes fail; the control reads.
#[test]
fn bad_shapes() {
    let base = srd_ruleset();
    assert_eq!(
        ruleset_of(read_ok(&with(
            &base,
            "preview",
            json!({"when": "actionable"})
        )))
        .preview,
        Some(PreviewWire {
            when: PreviewWhenWire::Actionable,
            max_cost: None
        })
    );

    for bad in [json!(3), json!(true), json!(["never"]), json!({"v": 1})] {
        let got = message(read(&with(&base, "preview", json!({ "when": bad }))));
        assert_eq!(got, "preview when must be a policy name", "when={bad}");
    }

    let got = message(read(&with(&base, "preview", json!({"when": "sometimes"}))));
    assert!(got.contains("unknown variant `sometimes`"), "{got}");
    for variant in ["never", "actionable", "planning"] {
        assert!(got.contains(&format!("`{variant}`")), "{got}");
    }

    let got = message(read(&with(
        &base,
        "preview",
        json!({"when": "actionable", "extra": 1}),
    )));
    assert!(got.contains("unknown field"), "{got}");
    assert!(got.contains("`extra`"), "{got}");

    let got = message(read(&with(&base, "preview", json!({}))));
    assert!(got.contains("missing field `when`"), "{got}");

    let got = message(read(&with(&base, "preview", json!({"max_cost": 5}))));
    assert!(got.contains("missing field `when`"), "{got}");

    for bad in [json!("actionable"), json!(7), json!(true)] {
        assert!(
            matches!(
                read(&with(&base, "preview", bad.clone())),
                Err(DataError::Malformed { .. })
            ),
            "preview={bad} accepted"
        );
    }

    // A near-miss key on the ruleset is an unknown field, not a preview.
    let got = message(read(&with(
        &base,
        "previews",
        json!({"when": "actionable"}),
    )));
    assert!(got.contains("unknown field"), "{got}");
}

// A5: the constant and the regenerated schema.
#[test]
fn constant_and_schema() {
    assert_eq!(MAX_PREVIEW_COST, 16_777_216);
    let schemas = crpg_data::generated_schemas().unwrap();
    let on_disk: Value = serde_json::from_slice(
        &std::fs::read(repo_root().join("schemas/ruleset.schema.json")).unwrap(),
    )
    .unwrap();
    let generated: Value = serde_json::from_slice(&schemas["ruleset.schema.json"]).unwrap();
    assert_eq!(on_disk, generated, "schemas/ruleset.schema.json is stale");

    let shape = &generated["oneOf"][0];
    assert!(shape["properties"]
        .as_object()
        .unwrap()
        .contains_key("preview"));
    assert!(!shape["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry == "preview"));
    assert_eq!(
        shape["properties"]["preview"]["anyOf"][0]["$ref"],
        json!("#/$defs/PreviewWire")
    );

    let when: Vec<&str> = generated["$defs"]["PreviewWhenWire"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["const"].as_str().unwrap())
        .collect();
    assert_eq!(when, ["never", "actionable", "planning"]);

    let wire = &generated["$defs"]["PreviewWire"];
    assert_eq!(wire["required"], json!(["when"]));
    assert_eq!(wire["additionalProperties"], json!(false));
    assert_eq!(wire["properties"]["max_cost"]["minimum"], json!(1));
    assert_eq!(wire["properties"]["max_cost"]["maximum"], json!(16_777_216));
    assert_eq!(
        wire["properties"]["when"]["$ref"],
        json!("#/$defs/PreviewWhenWire")
    );

    // The version tag and registry are untouched.
    let versions = crpg_data::schema_versions();
    let current = versions
        .iter()
        .find(|entry| entry.schema_type == "crpg.ruleset")
        .map(|entry| entry.current);
    assert_eq!(current, Some(2));
    assert_eq!(versions.len(), 21);
}
