#![forbid(unsafe_code)]
//! T157b: the optional encounter `engage` list of tagged triggers, `proximity`
//! only (S-B1). It joins `crpg.encounter/1` at its current tag; no document
//! that lacks it changes.
use crpg_core::Fx16_16;
use crpg_data::{read_document, write_document, DataError, Document, EngageTriggerWire};
use serde_json::{json, Value};
use std::path::PathBuf;

/// The text of `campaigns/fixtures/combat_basic/encounters/first-blood.json`.
const ENC: &str =
    include_str!("../../../campaigns/fixtures/combat_basic/encounters/first-blood.json");

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// `ENC` with an `engage` value added through the value tree.
fn with_engage(engage: Value) -> Value {
    let mut value: Value = serde_json::from_str(ENC).unwrap();
    value["engage"] = engage;
    value
}

fn read(value: &Value) -> Result<Document, DataError> {
    read_document(&serde_json::to_vec(value).unwrap())
}

fn engage_of(document: Document) -> Option<Vec<EngageTriggerWire>> {
    match document {
        Document::Encounter(encounter) => encounter.engage,
        other => panic!("expected encounter, got {other:?}"),
    }
}

fn message(result: Result<Document, DataError>) -> String {
    match result {
        Err(DataError::Malformed { message, .. }) => message,
        other => panic!("expected Malformed, got {other:?}"),
    }
}

fn proximity(distance: i64) -> Value {
    json!({"type": "proximity", "value": {"distance": distance}})
}

/// The same document text with `"engage": ...,` added before `"id"`, the first
/// key in canonical (sorted) order.
fn enc_with_engage_text(engage_text: &str) -> String {
    let (head, tail) = ENC.split_once("  \"id\"").expect("ENC has an id key");
    format!("{head}  \"engage\": {engage_text},\n  \"id\"{tail}")
}

// SB1 (S-B1): absent `engage` is today's document, byte for byte.
#[test]
fn absent_engage_is_today() {
    let document = read_document(ENC.as_bytes()).unwrap();
    assert_eq!(engage_of(document.clone()), None);
    assert_eq!(write_document(&document).unwrap(), ENC.as_bytes());
}

// SB2 (S-B1): a proximity trigger and the empty list round-trip; order is kept.
#[test]
fn proximity_round_trips() {
    let text = enc_with_engage_text(
        "[\n    {\n      \"type\": \"proximity\",\n      \"value\": {\n        \"distance\": 262144\n      }\n    }\n  ]",
    );
    let document = read_document(text.as_bytes()).unwrap();
    assert_eq!(
        engage_of(document.clone()),
        Some(vec![EngageTriggerWire::Proximity {
            distance: Fx16_16::from_raw(262144)
        }])
    );
    assert_eq!(write_document(&document).unwrap(), text.as_bytes());

    let text = enc_with_engage_text("[]");
    let document = read_document(text.as_bytes()).unwrap();
    assert_eq!(engage_of(document.clone()), Some(vec![]));
    assert_eq!(write_document(&document).unwrap(), text.as_bytes());

    let document = read(&with_engage(json!([proximity(65536), proximity(131072)]))).unwrap();
    assert_eq!(
        engage_of(document),
        Some(vec![
            EngageTriggerWire::Proximity {
                distance: Fx16_16::from_raw(65536)
            },
            EngageTriggerWire::Proximity {
                distance: Fx16_16::from_raw(131072)
            },
        ])
    );
}

// SB3 (S-B1): every refusal, with its exact message where it is a read-time rule.
#[test]
fn refusals() {
    for bad in [0, -1] {
        assert_eq!(
            message(read(&with_engage(json!([proximity(bad)])))),
            "encounter engage proximity distance must be positive",
            "distance {bad}"
        );
    }
    let nine: Vec<Value> = (0..9).map(|_| proximity(1)).collect();
    assert_eq!(
        message(read(&with_engage(Value::Array(nine)))),
        "encounter engage has too many triggers"
    );
    // The count is checked before the distances: nine entries of zero distance.
    let nine_bad: Vec<Value> = (0..9).map(|_| proximity(0)).collect();
    assert_eq!(
        message(read(&with_engage(Value::Array(nine_bad)))),
        "encounter engage has too many triggers"
    );

    let unknown = message(read(&with_engage(json!([{"type": "sight", "value": {}}]))));
    assert!(unknown.contains("unknown variant `sight`"), "{unknown}");
    assert!(unknown.contains("`proximity`"), "{unknown}");

    let extra_inner = message(read(&with_engage(json!([
        {"type": "proximity", "value": {"distance": 1, "x": 1}}
    ]))));
    assert!(extra_inner.contains("unknown field `x`"), "{extra_inner}");

    let no_distance = message(read(&with_engage(json!([
        {"type": "proximity", "value": {}}
    ]))));
    assert!(
        no_distance.contains("missing field `distance`"),
        "{no_distance}"
    );

    let no_value = message(read(&with_engage(json!([{"type": "proximity"}]))));
    assert!(no_value.contains("missing field `value`"), "{no_value}");

    let extra_outer = message(read(&with_engage(json!([
        {"type": "proximity", "value": {"distance": 1}, "extra": 0}
    ]))));
    assert!(
        extra_outer.contains("unknown field `extra`"),
        "{extra_outer}"
    );

    // Control: eight valid entries decode, in order.
    let eight: Vec<Value> = (1..=8).map(|n| proximity(n * 65536)).collect();
    let decoded = engage_of(read(&with_engage(Value::Array(eight))).unwrap()).unwrap();
    assert_eq!(decoded.len(), 8);
    assert_eq!(
        decoded[7],
        EngageTriggerWire::Proximity {
            distance: Fx16_16::from_raw(8 * 65536)
        }
    );
}

// SB4 (S-B1): the regenerated schema lists `engage` as an optional property
// whose items are exactly one variant, `proximity`.
#[test]
fn schema_lists_one_kind() {
    let bytes = std::fs::read(repo_root().join("schemas/encounter.schema.json")).unwrap();
    let schema: Value = serde_json::from_slice(&bytes).unwrap();
    let shape = &schema["oneOf"][0];
    let engage = &shape["properties"]["engage"];
    assert!(engage.is_object(), "no engage property");
    let required = shape["required"].as_array().unwrap();
    assert!(!required.contains(&json!("engage")));
    assert_eq!(engage["type"], json!(["array", "null"]));
    assert_eq!(engage["items"]["$ref"], json!("#/$defs/EngageTriggerWire"));
    let variants = schema["$defs"]["EngageTriggerWire"]["oneOf"]
        .as_array()
        .expect("EngageTriggerWire is a oneOf");
    assert_eq!(variants.len(), 1);
    assert_eq!(
        variants[0]["properties"]["type"]["const"],
        json!("proximity")
    );
}
