#![forbid(unsafe_code)]
//! Old-schema campaign loading through the registered item dummy edge.
mod support;
use crpg_core::Ulid;
use crpg_data::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

type Files = BTreeMap<SourcePath, Vec<u8>>;

fn engine() -> semver::Version {
    "0.1.0".parse().unwrap()
}

fn path(text: &str) -> SourcePath {
    text.parse().unwrap()
}

fn migration_files() -> Files {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/migration_v1/campaign");
    [
        "campaign.json",
        "campaign.lock",
        "worlds/world.json",
        "areas/start/area.json",
        "areas/start/placements.json",
        "areas/start/triggers.json",
        "creatures/creature.json",
        "items/item.json",
        "variables/campaign_state.json",
        "assets/assets.lock",
        "locale/en.json",
    ]
    .into_iter()
    .map(|name| {
        (
            path(name),
            fs::read(root.join(name)).expect("migration fixture file"),
        )
    })
    .collect()
}

fn golden_map() -> Files {
    let golden: BTreeMap<String, String> = serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/migration_v1/expected.json"),
        )
        .unwrap(),
    )
    .unwrap();
    golden
        .into_iter()
        .map(|(name, text)| (path(&name), text.into_bytes()))
        .collect()
}

#[test]
fn v1_campaign_matches_golden_with_eight_ids_and_clean() {
    let files = migration_files();
    assert_eq!(files.len(), 11);
    let before = files.clone();
    let campaign = load_campaign(&files, &engine()).unwrap();
    // Caller bytes are borrowed and unchanged.
    assert_eq!(files, before);
    let expected_entries = [
        (1, ObjectKind::Campaign, "campaign.json", ""),
        (2, ObjectKind::World, "worlds/world.json", ""),
        (3, ObjectKind::Area, "areas/start/area.json", ""),
        (4, ObjectKind::Creature, "creatures/creature.json", ""),
        (
            5,
            ObjectKind::Placement,
            "areas/start/placements.json",
            "/placements/0",
        ),
        (
            6,
            ObjectKind::Graph,
            "areas/start/triggers.json",
            "/graphs/0",
        ),
        (
            7,
            ObjectKind::Node,
            "areas/start/triggers.json",
            "/graphs/0/nodes/0",
        ),
        (8, ObjectKind::Item, "items/item.json", ""),
    ];
    assert_eq!(campaign.index.len(), 8);
    for (id, kind, file, pointer) in expected_entries {
        assert_eq!(
            campaign.index[&Ulid::from_u128(id)],
            IndexEntry {
                kind,
                path: path(file),
                pointer: pointer.into(),
            },
            "{id}"
        );
    }
    assert_eq!(validate(&campaign), Vec::new());
    assert_eq!(validate_files(&files, &engine()), Vec::new());
    assert_eq!(serialize_campaign(&campaign).unwrap(), golden_map());
    // Ten unaffected files equal their source bytes; the item differs only by tag.
    let golden = golden_map();
    for (name, bytes) in &files {
        let output = &golden[name];
        if name.as_str() == "items/item.json" {
            let source: Value = serde_json::from_slice(bytes).unwrap();
            let migrated: Value = serde_json::from_slice(output).unwrap();
            assert_eq!(source["schema"], json!("crpg.item/1"));
            assert_eq!(migrated["schema"], json!("crpg.item/2"));
            let mut source_tagged = source.clone();
            source_tagged["schema"] = json!("crpg.item/2");
            assert_eq!(source_tagged, migrated);
        } else {
            assert_eq!(bytes, output, "{}", name.as_str());
        }
    }
}

#[test]
fn current_reload_and_write_are_byte_stable() {
    let golden = golden_map();
    let campaign = load_campaign(&golden, &engine()).unwrap();
    assert_eq!(campaign.index.len(), 8);
    assert_eq!(serialize_campaign(&campaign).unwrap(), golden);
    let again = load_campaign(&serialize_campaign(&campaign).unwrap(), &engine()).unwrap();
    assert_eq!(serialize_campaign(&again).unwrap(), golden);
    assert_eq!(validate(&campaign), Vec::new());
}

#[test]
fn mixed_version_inputs_migrate_together() {
    // Old item plus an extra current item coexist; both end current.
    let mut files = migration_files();
    let extra = json!({
        "schema": "crpg.item/2",
        "id": Ulid::from_u128(10),
        "slug": "extra",
        "name": "fixture.creature",
        "stats": {},
        "tags": []
    });
    files.insert(path("items/extra.json"), canonical_json(&extra).unwrap());
    let campaign = load_campaign(&files, &engine()).unwrap();
    assert_eq!(campaign.index.len(), 9);
    assert_eq!(
        campaign.index[&Ulid::from_u128(10)],
        IndexEntry {
            kind: ObjectKind::Item,
            path: path("items/extra.json"),
            pointer: String::new(),
        }
    );
    let output = serialize_campaign(&campaign).unwrap();
    assert_eq!(output.len(), 12);
    let migrated: Value = serde_json::from_slice(&output[&path("items/item.json")]).unwrap();
    assert_eq!(migrated["schema"], json!("crpg.item/2"));
    let stayed: Value = serde_json::from_slice(&output[&path("items/extra.json")]).unwrap();
    assert_eq!(stayed["schema"], json!("crpg.item/2"));
}

#[test]
fn notes_ids_and_array_order_survive_tag_only_migration() {
    let mut value = json!({
        "_note": "keep me",
        "id": Ulid::from_u128(8),
        "name": "fixture.creature",
        "schema": "crpg.item/1",
        "slug": "item",
        "stats": {"b": 2, "a": 1},
        "tags": ["z", "a", "z"]
    });
    let before = value.clone();
    migrate_document(&mut value).unwrap();
    assert_eq!(value["schema"], json!("crpg.item/2"));
    for key in ["id", "slug", "name", "_note", "stats"] {
        assert_eq!(value[key], before[key], "{key}");
    }
    assert_eq!(value["tags"], json!(["z", "a", "z"]));
    // Campaign author note from the untouched files is also preserved.
    let files = migration_files();
    let campaign = load_campaign(&files, &engine()).unwrap();
    let Document::Campaign(manifest) = &campaign.documents[&path("campaign.json")] else {
        panic!("campaign doc")
    };
    assert_eq!(manifest.note.as_deref(), Some("T010 canonical fixture"));
    let Document::Item(item) = &campaign.documents[&path("items/item.json")] else {
        panic!("item doc")
    };
    assert_eq!(item.note.as_deref(), Some("T012 migration fixture"));
    assert_eq!(item.id, Ulid::from_u128(8));
}

#[test]
fn public_value_rollback_and_idempotence() {
    // Successful migration publishes once and then stays put.
    let mut value: Value =
        serde_json::from_slice(&migration_files()[&path("items/item.json")]).unwrap();
    migrate_document(&mut value).unwrap();
    assert_eq!(value["schema"], json!("crpg.item/2"));
    let once = value.clone();
    migrate_document(&mut value).unwrap();
    assert_eq!(value, once);
    // Float-bearing values are built from byte literals, never Rust floats.
    let float_value: Value =
        serde_json::from_slice(b"{\"schema\":\"crpg.item/1\",\"id\":1.0}").unwrap();
    // Every failure below leaves the caller exactly unchanged.
    let mut failures: Vec<Value> = vec![
        json!(null),
        json!([]),
        json!({}),
        json!({"schema": 1}),
        json!({"schema": "future/1"}),
        json!({"schema": "crpg.item/0"}),
        json!({"schema": "crpg.item/01"}),
        json!({"schema": "crpg.item/+1"}),
        json!({"schema": "crpg.item/4294967296"}),
        json!({"schema": "crpg.creature/2"}),
        json!({"schema": "crpg.item/3"}),
        json!({"schema": "crpg.item/1", "id": "not-a-ulid"}),
    ];
    failures.push(float_value);
    for mut bad in failures {
        let before = bad.clone();
        assert!(migrate_document(&mut bad).is_err());
        assert_eq!(bad, before);
    }
}

#[test]
fn every_tag_shape_maps_to_its_error_with_original_found() {
    // Missing, non-string and non-object envelopes are malformed.
    for bytes in [
        serde_json::to_vec(&json!({})).unwrap(),
        serde_json::to_vec(&json!({"schema": 1})).unwrap(),
        serde_json::to_vec(&json!(null)).unwrap(),
        serde_json::to_vec(&json!([])).unwrap(),
    ] {
        assert!(matches!(
            read_document(&bytes),
            Err(DataError::Malformed { path: None, .. })
        ));
    }
    let value: Value =
        serde_json::from_slice(b"{\"schema\":\"crpg.item/1\",\"count\":1.0}").unwrap();
    assert!(matches!(
        crate::migrate_document(&mut value.clone()),
        Err(DataError::Malformed { .. })
    ));
    // Unknown types, bad texts, zero, overflow and future versions are
    // unsupported with the original tag preserved.
    for tag in [
        "future/1",
        "crpg.unknown/1",
        "",
        "crpg.item",
        "crpg.item/1/extra",
        "crpg.item/01",
        "crpg.item/+1",
        "crpg.item/-1",
        "crpg.item/1 ",
        " crpg.item/1",
        "crpg.item/1a",
        "crpg.item/",
        "crpg.item/0",
        "crpg.item/00",
        "crpg.item/4294967296",
        "crpg.item/99999999999",
        "crpg.creature/2",
        "crpg.campaign/2",
        "crpg.item/3",
        "crpg.assets-lock/2",
        "crpg.campaign-lock/9",
    ] {
        let bytes = canonical_json(&json!({"schema": tag})).unwrap();
        assert!(
            matches!(
                read_document(&bytes),
                Err(DataError::UnsupportedSchema { path: None, found }) if found == tag
            ),
            "{tag}"
        );
        let mut value = json!({"schema": tag});
        assert!(
            matches!(
                migrate_document(&mut value),
                Err(DataError::UnsupportedSchema { path: None, found }) if found == tag
            ),
            "{tag}"
        );
    }
    // Syntax still wins over an unsupported schema, and unsupported still wins
    // over typed-field errors.
    assert!(matches!(
        read_document(b"{\"schema\":\"future/1\",\"x\":1.0}"),
        Err(DataError::Malformed { .. })
    ));
    assert!(matches!(
        read_document(b"{\"schema\":\"crpg.item/1\",\"x\":1.0}"),
        Err(DataError::Malformed { .. })
    ));
    assert!(matches!(
        read_document(b"{\"schema\":\"crpg.creature/9\",\"id\":false}"),
        Err(DataError::UnsupportedSchema { found, .. }) if found == "crpg.creature/9"
    ));
}

#[test]
fn malformed_old_payload_is_rejected_not_repaired() {
    let base: Value = serde_json::from_slice(&migration_files()[&path("items/item.json")]).unwrap();
    // Missing required field.
    let mut missing = base.clone();
    missing.as_object_mut().unwrap().remove("stats");
    let mut missing_bytes = canonical_json(&missing).unwrap();
    let _ = &mut missing_bytes;
    assert!(matches!(
        migrate_document(&mut missing.clone()),
        Err(DataError::Malformed { .. })
    ));
    assert!(matches!(
        read_document(&canonical_json(&missing).unwrap()),
        Err(DataError::Malformed { .. })
    ));
    // Unknown field.
    let mut unknown = base.clone();
    unknown["unknown"] = json!(true);
    assert!(matches!(
        migrate_document(&mut unknown.clone()),
        Err(DataError::Malformed { .. })
    ));
    assert!(matches!(
        read_document(&canonical_json(&unknown).unwrap()),
        Err(DataError::Malformed { .. })
    ));
    // Wrong primitive type.
    let mut wrong = base.clone();
    wrong["id"] = json!(false);
    assert!(matches!(
        migrate_document(&mut wrong.clone()),
        Err(DataError::Malformed { .. })
    ));
}

#[test]
fn old_and_new_item_bytes_read_through_one_dispatcher() {
    let v1 = migration_files()[&path("items/item.json")].clone();
    let Document::Item(migrated) = read_document(&v1).unwrap() else {
        panic!("migrated item")
    };
    assert_eq!(migrated.id, Ulid::from_u128(8));
    assert_eq!(migrated.slug, "item");
    let golden = golden_map();
    let v2 = golden[&path("items/item.json")].clone();
    let Document::Item(current) = read_document(&v2).unwrap() else {
        panic!("current item")
    };
    assert_eq!(migrated, current);
    assert_eq!(write_document(&Document::Item(current)).unwrap(), v2);
    // Both lock readers still accept only their current version paths.
    let files = support::fixture_files();
    let campaign_lock = files[&path("campaign.lock")].clone();
    assert!(read_campaign_lock(&campaign_lock).is_ok());
    let assets_lock = files[&path("assets/assets.lock")].clone();
    assert!(read_assets_lock(&assets_lock).is_ok());
    assert!(matches!(
        read_document(b"{\"schema\":\"crpg.campaign-lock/2\",\"packages\":[],\"assets_lock\":\"957bc137f1abb3cde6cee277d10c099b1dcd2f8814a5fd0e29b1df3506ee44fb\"}"),
        Err(DataError::UnsupportedSchema { .. })
    ));
}
