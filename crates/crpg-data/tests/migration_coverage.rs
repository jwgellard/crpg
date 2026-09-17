#![forbid(unsafe_code)]
//! Non-vacuous migration gate: registry, schemas, serde and fixtures agree.
use crpg_data as gate_api;
use crpg_data::*;
use serde_json::{json, Value};
#[path = "support/migration_gate.rs"]
mod gate;

fn edit(files: &mut gate::Snapshot, name: &str, change: impl FnOnce(&mut Value)) {
    let mut value: Value = serde_json::from_slice(&files[name]).unwrap();
    change(&mut value);
    files.insert(name.into(), canonical_json(&value).unwrap());
}

#[test]
fn registry_schema_and_serde_agree_on_fifteen_current_tags() {
    let versions = schema_versions();
    assert_eq!(versions.len(), 15);
    let schemas = generated_schemas().unwrap();
    assert_eq!(schemas.len(), 17);
    let serialized = gate::representatives(&gate::snapshot().unwrap()).unwrap();
    assert_eq!(serialized.len(), 15);
    gate::check_tags(versions, &schemas, &serialized).unwrap();
    // Retain the concrete initial-version assertions independently of the checker.
    for version in versions {
        assert_eq!(
            version.current,
            if version.schema_type == "crpg.item" {
                2
            } else {
                1
            }
        );
    }
}

#[test]
fn manifest_covers_every_edge_with_checked_in_fixtures() {
    let files = gate::snapshot().unwrap();
    let entries = gate::check_manifest(schema_versions(), &files).unwrap();
    assert_eq!(
        entries,
        vec![gate::Entry {
            schema_type: "crpg.item".into(),
            from: 1,
            to: 2,
            root: "migration_v1/campaign".into(),
            document: "items/item.json".into(),
            golden: "migration_v1/expected.json".into(),
        }]
    );
    let roots: Value = serde_json::from_slice(&files["expected.json"]).unwrap();
    assert_eq!(roots.as_array().unwrap().len(), 3);
    let oracles = gate::check_fixtures(schema_versions(), &files).unwrap();
    // Public dispatch can independently verify latest edges. Private registry
    // unit tests execute every single edge, including superseded destinations.
    let latest: Vec<_> = oracles
        .into_iter()
        .filter(|oracle| {
            schema_versions().iter().any(|version| {
                version.schema_type == oracle.edge.0 && version.current == oracle.edge.2
            })
        })
        .collect();
    gate::check_steps(&latest, |_, value| {
        migrate_document(value).map_err(|e| e.to_string())
    })
    .unwrap();
}

#[test]
fn checker_fails_for_tag_registry_and_row_mutations() {
    let files = gate::snapshot().unwrap();
    let schemas = generated_schemas().unwrap();
    let serialized = gate::representatives(&files).unwrap();
    gate::check_tags(schema_versions(), &schemas, &serialized).unwrap();
    let mut bumped_schemas = schemas.clone();
    edit(&mut bumped_schemas, "item.schema.json", |v| {
        v["oneOf"][0]["properties"]["schema"]["const"] = json!("crpg.item/3")
    });
    assert!(gate::check_tags(schema_versions(), &bumped_schemas, &serialized).is_err());
    let mut bumped_serde = serialized.clone();
    let item = bumped_serde
        .iter_mut()
        .find(|bytes| serde_json::from_slice::<Value>(bytes).unwrap()["schema"] == "crpg.item/2")
        .unwrap();
    let mut value: Value = serde_json::from_slice(item).unwrap();
    value["schema"] = json!("crpg.item/3");
    *item = canonical_json(&value).unwrap();
    assert!(gate::check_tags(schema_versions(), &schemas, &bumped_serde).is_err());
    let mut missing_serde = serialized.clone();
    missing_serde.pop();
    assert!(gate::check_tags(schema_versions(), &schemas, &missing_serde).is_err());

    let mut bumped = schema_versions().to_vec();
    bumped
        .iter_mut()
        .find(|v| v.schema_type == "crpg.item")
        .unwrap()
        .current = 3;
    assert!(gate::check_manifest(&bumped, &files).is_err());
    let mut missing = files.clone();
    missing.remove("migrations.json");
    assert!(gate::check_manifest(schema_versions(), &missing).is_err());
    let row: Value = serde_json::from_slice::<Value>(&files["migrations.json"]).unwrap()[0].clone();
    let mut extra = row.clone();
    extra["schema_type"] = json!("crpg.world");
    let mut unknown = row.clone();
    unknown["schema_type"] = json!("unknown.type");
    let mut gap = row.clone();
    gap["from"] = json!(2);
    gap["to"] = json!(3);
    let mut jump = row.clone();
    jump["to"] = json!(3);
    let mut cycle = row.clone();
    cycle["from"] = json!(2);
    cycle["to"] = json!(1);
    for (name, rows, versions) in [
        ("missing row", json!([]), schema_versions()),
        ("duplicate row", json!([row, row]), schema_versions()),
        (
            "extra version-1 row",
            json!([row, extra]),
            schema_versions(),
        ),
        ("unknown family", json!([row, unknown]), schema_versions()),
        ("gap", json!([gap]), bumped.as_slice()),
        ("jump", json!([jump]), bumped.as_slice()),
        ("cycle", json!([row, cycle]), bumped.as_slice()),
        ("unsorted", json!([gap, row]), bumped.as_slice()),
    ] {
        let mut bad = files.clone();
        bad.insert("migrations.json".into(), canonical_json(&rows).unwrap());
        assert!(gate::check_manifest(versions, &bad).is_err(), "{name}");
    }
}

#[test]
fn manifest_parser_rejects_overflow_wrong_shapes_and_noncanonical_bytes() {
    let files = gate::snapshot().unwrap();
    gate::check_manifest(schema_versions(), &files).unwrap();
    for field in ["from", "to"] {
        for value in [
            json!(4294967296_u64),
            json!(4294967297_u64),
            json!(u64::MAX),
            json!(-1),
            json!(0),
            json!("1"),
            json!(null),
        ] {
            let mut bad = files.clone();
            edit(&mut bad, "migrations.json", |v| v[0][field] = value);
            assert!(
                gate::check_manifest(schema_versions(), &bad).is_err(),
                "{field}"
            );
        }
    }
    // Exact review reproduction: both values previously wrapped into 1 -> 2.
    let mut overflow = files.clone();
    edit(&mut overflow, "migrations.json", |v| {
        v[0]["from"] = json!(4294967297_u64);
        v[0]["to"] = json!(4294967298_u64);
    });
    assert!(gate::check_manifest(schema_versions(), &overflow).is_err());
    for field in ["schema_type", "from", "to", "root", "document", "golden"] {
        let mut bad = files.clone();
        edit(&mut bad, "migrations.json", |v| {
            v[0].as_object_mut().unwrap().remove(field);
        });
        assert!(
            gate::check_manifest(schema_versions(), &bad).is_err(),
            "missing {field}"
        );
    }
    let mut extra = files.clone();
    edit(&mut extra, "migrations.json", |v| {
        v[0]["extra"] = json!(true)
    });
    assert!(gate::check_manifest(schema_versions(), &extra).is_err());
    for bytes in [
        b"".as_slice(),
        b"{invalid}",
        b"{}\n",
        b"[1]\n",
        b"[{}]\n",
        b"[\n  {\"from\":1,\"from\":1}\n]\n",
    ] {
        let mut bad = files.clone();
        bad.insert("migrations.json".into(), bytes.to_vec());
        assert!(gate::check_manifest(schema_versions(), &bad).is_err());
    }
}

#[test]
fn checker_fails_for_fixture_and_golden_mutations() {
    let files = gate::snapshot().unwrap();
    gate::check_fixtures(schema_versions(), &files).unwrap();
    let source = "migration_v1/campaign/items/item.json";
    let golden = "migration_v1/expected.json";
    // Every mutation is passed to the same checker as the checked-in baseline.
    let mut missing_root = files.clone();
    missing_root.retain(|name, _| !name.starts_with("migration_v1/campaign/"));
    assert!(gate::check_fixtures(schema_versions(), &missing_root).is_err());
    for name in [source, golden, "expected.json"] {
        let mut missing = files.clone();
        missing.remove(name);
        assert!(
            gate::check_fixtures(schema_versions(), &missing).is_err(),
            "missing {name}"
        );
        for bytes in [b"".as_slice(), b"{invalid}", b"{}\n", b"[]\n"] {
            let mut bad = files.clone();
            bad.insert(name.into(), bytes.to_vec());
            assert!(
                gate::check_fixtures(schema_versions(), &bad).is_err(),
                "malformed/empty {name}"
            );
        }
    }
    let mut wrong_tag = files.clone();
    edit(&mut wrong_tag, source, |v| {
        v["schema"] = json!("crpg.item/2")
    });
    assert!(migrate_document(&mut serde_json::from_slice(&wrong_tag[source]).unwrap()).is_ok());
    assert!(gate::check_fixtures(schema_versions(), &wrong_tag).is_err());
    let mut wrong_output = files.clone();
    edit(&mut wrong_output, golden, |v| {
        v["items/item.json"] = json!(v["items/item.json"]
            .as_str()
            .unwrap()
            .replace("fixture.creature", "fixture.changed"));
    });
    assert!(gate::check_fixtures(schema_versions(), &wrong_output).is_err());
    let mut missing_key = files.clone();
    edit(&mut missing_key, golden, |v| {
        v.as_object_mut().unwrap().remove("items/item.json");
    });
    assert!(gate::check_fixtures(schema_versions(), &missing_key).is_err());
    let mut extra_key = files.clone();
    edit(&mut extra_key, golden, |v| {
        v["extra.json"] = v["items/item.json"].clone()
    });
    assert!(gate::check_fixtures(schema_versions(), &extra_key).is_err());
    let mut not_clean = files.clone();
    edit(&mut not_clean, "expected.json", |v| {
        v[1]["expect"] = json!("diagnostics")
    });
    assert!(gate::check_fixtures(schema_versions(), &not_clean).is_err());
    for name in [
        "migration_steps/crpg.item/1-to-2.json",
        "migration_steps/campaign.json",
    ] {
        let mut extra_step = files.clone();
        extra_step.insert(name.into(), b"{invalid oracle}".to_vec());
        assert!(gate::check_fixtures(schema_versions(), &extra_step).is_err());
    }
}
