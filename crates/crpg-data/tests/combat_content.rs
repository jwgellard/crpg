#![forbid(unsafe_code)]
//! T016a combat vocabulary and minimal-d6 content acceptance (crpg-data).
use crpg_core::{Fx16_16, Ulid};
use crpg_data::{DiagnosticCode, Document, LoadedCampaign, ObjectKind, SourcePath};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn combat_root() -> PathBuf {
    crate_dir()
        .join("..")
        .join("..")
        .join("campaigns")
        .join("fixtures")
        .join("combat_basic")
}

fn ruleset_root() -> PathBuf {
    crate_dir()
        .join("..")
        .join("..")
        .join("rulesets")
        .join("minimal-d6")
}

fn read_tree(root: &Path) -> BTreeMap<SourcePath, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<SourcePath, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                walk(&path, root, out);
            } else if kind.is_file() {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                if name == ".gitattributes" {
                    continue;
                }
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let key: SourcePath = relative.parse().unwrap();
                out.insert(key, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn engine() -> semver::Version {
    "0.1.0".parse().unwrap()
}

fn load_combat() -> (BTreeMap<SourcePath, Vec<u8>>, LoadedCampaign) {
    let files = read_tree(&combat_root());
    let loaded = crpg_data::load_campaign(&files, &engine()).expect("combat_basic loads");
    (files, loaded)
}

fn id(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

#[test]
fn combat_basic_loads_validates_clean_and_round_trips() {
    let (files, loaded) = load_combat();
    assert_eq!(files.len(), 14);
    let index = &loaded.index;
    assert_eq!(index.len(), 11);
    let expected: Vec<(Ulid, ObjectKind, &str, &str)> = vec![
        (id(11), ObjectKind::Campaign, "campaign.json", ""),
        (id(12), ObjectKind::World, "worlds/world.json", ""),
        (id(13), ObjectKind::Area, "areas/start/area.json", ""),
        (
            id(14),
            ObjectKind::Placement,
            "areas/start/placements.json",
            "/placements/0",
        ),
        (
            id(15),
            ObjectKind::Placement,
            "areas/start/placements.json",
            "/placements/1",
        ),
        (id(16), ObjectKind::Creature, "creatures/hero.json", ""),
        (id(17), ObjectKind::Creature, "creatures/goblin.json", ""),
        (id(18), ObjectKind::Ruleset, "rulesets/minimal-d6.json", ""),
        (id(19), ObjectKind::Ability, "abilities/strike.json", ""),
        (
            id(20),
            ObjectKind::OutcomeTable,
            "outcome_tables/roll-under.json",
            "",
        ),
        (
            id(21),
            ObjectKind::Encounter,
            "encounters/first-blood.json",
            "",
        ),
    ];
    for (want_id, want_kind, want_path, want_pointer) in expected {
        let entry = index.get(&want_id).expect("indexed id");
        assert_eq!(entry.kind, want_kind);
        assert_eq!(entry.path.as_str(), want_path);
        assert_eq!(entry.pointer, want_pointer);
    }
    assert_eq!(crpg_data::validate(&loaded), Vec::new());
    // Minimal-d6 ships at /1 and loads through the /1 → /2 edges with
    // preserved meaning; serialization writes current /2, so the first
    // pass differs from the source bytes exactly by the migrated shapes
    // while the second pass round-trips byte-identically.
    let once = crpg_data::serialize_campaign(&loaded).expect("serializes");
    assert_eq!(once.len(), files.len());
    for (path, bytes) in &files {
        let source: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        let migrated: serde_json::Value = serde_json::from_slice(&once[path]).unwrap();
        let source_tag = source["schema"].as_str().unwrap().to_string();
        if source_tag == "crpg.ruleset/1" {
            assert_eq!(migrated["schema"], serde_json::json!("crpg.ruleset/2"));
            assert_eq!(
                migrated["pools"],
                serde_json::json!([source["action_pool"]])
            );
            assert!(migrated.get("action_pool").is_none());
        } else if source_tag == "crpg.ability/1" {
            assert_eq!(migrated["schema"], serde_json::json!("crpg.ability/2"));
            assert_eq!(migrated["cost"], source["cost"]);
            assert_eq!(migrated["extra_costs"], serde_json::json!([]));
            assert_eq!(migrated["ends_turn"], serde_json::json!(true));
            assert_eq!(
                migrated["defense"],
                serde_json::json!({"type": "actor_attribute"})
            );
        } else {
            assert_eq!(bytes, &once[path], "{}", path.as_str());
        }
    }
    let reloaded = crpg_data::load_campaign(&once, &engine()).expect("reloads");
    let twice = crpg_data::serialize_campaign(&reloaded).expect("reserializes");
    assert_eq!(twice, once);
}

#[test]
fn canonical_ruleset_source_matches_campaign_copies() {
    let source = read_tree(&ruleset_root());
    assert_eq!(source.len(), 3);
    let (_, loaded) = load_combat();
    for (logical, bytes) in &source {
        let doc = crpg_data::read_document(bytes).expect("source reads");
        let found = loaded.documents.values().any(|candidate| candidate == &doc);
        assert!(found, "source {logical} has a campaign copy");
    }
    let by_id = |n: u128| {
        loaded
            .documents
            .values()
            .find(|doc| {
                matches!(
                    doc,
                    Document::Ruleset(v) if v.id == id(n)
                ) || matches!(
                    doc,
                    Document::Ability(v) if v.id == id(n)
                ) || matches!(
                    doc,
                    Document::OutcomeTable(v) if v.id == id(n)
                )
            })
            .expect("combat id present")
            .clone()
    };
    assert_eq!(
        by_id(18),
        crpg_data::read_document(&source[&"ruleset.json".parse().unwrap()]).unwrap()
    );
    assert_eq!(
        by_id(19),
        crpg_data::read_document(&source[&"strike.json".parse().unwrap()]).unwrap()
    );
    assert_eq!(
        by_id(20),
        crpg_data::read_document(&source[&"roll_under.json".parse().unwrap()]).unwrap()
    );
}

#[test]
fn minimal_d6_content_pins() {
    let (_, loaded) = load_combat();
    let ruleset = loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            Document::Ruleset(v) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        ruleset
            .stats
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        vec!["might", "guile", "resolve", "health"]
    );
    assert_eq!(ruleset.health_stat, "health");
    assert_eq!(ruleset.attributes, vec!["might", "guile", "resolve"]);
    assert_eq!(ruleset.pools.len(), 1);
    assert_eq!(ruleset.pools[0].max, 1);
    assert!(matches!(
        ruleset.pools[0].refresh,
        crpg_data::RefreshWire::OnTurnStart
    ));
    assert_eq!(ruleset.abilities, vec![id(19)]);
    assert_eq!(ruleset.package.as_str(), "minimal-d6");

    let ability = loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            Document::Ability(v) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(ability.dice, "2d6");
    assert_eq!(ability.attribute, "might");
    assert_eq!(ability.outcome_table, id(20));
    assert_eq!(ability.damage.len(), 2);
    assert!(matches!(
        ability.damage[0].outcome,
        crpg_data::OutcomeWire::Success
    ));
    assert_eq!(ability.damage[0].amount, 2);
    assert!(matches!(
        ability.damage[1].outcome,
        crpg_data::OutcomeWire::Failure
    ));
    assert_eq!(ability.damage[1].amount, 0);
    assert_eq!(ability.cost, 1);
    assert!(ability.extra_costs.is_empty());
    assert!(ability.ends_turn);
    assert_eq!(ability.effect, None);
    assert!(matches!(
        ability.defense,
        crpg_data::DefenseWire::ActorAttribute
    ));
    assert_eq!(ability.natural_die, None);
    assert!(ability.requires_target);
    assert!(!ability.allow_self_target);

    let table = loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            Document::OutcomeTable(v) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(table.bands.len(), 2);
    assert_eq!(table.bands[0].min_margin, i64::MIN);
    assert!(matches!(
        table.bands[0].outcome,
        crpg_data::OutcomeWire::Success
    ));
    assert_eq!(table.bands[1].min_margin, 1);
    assert!(matches!(
        table.bands[1].outcome,
        crpg_data::OutcomeWire::Failure
    ));
    assert!(table.natural_rules.is_empty());

    let encounter = loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            Document::Encounter(v) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(encounter.ruleset, id(18));
    assert_eq!(encounter.participants.len(), 2);
    assert_eq!(encounter.participants[0].placement, id(14));
    assert_eq!(encounter.participants[0].initiative, 10);
    assert_eq!(encounter.participants[1].placement, id(15));
    assert_eq!(encounter.participants[1].initiative, 9);

    let hero = loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            Document::Creature(v) if v.id == id(16) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(hero.stats.get("might").unwrap().to_raw(), 8 * 65536);
    assert_eq!(hero.stats.get("health").unwrap().to_raw(), 10 * 65536);
}

fn ability_value() -> serde_json::Value {
    let (_, loaded) = load_combat();
    let doc = loaded
        .documents
        .values()
        .find(|doc| matches!(doc, Document::Ability(_)))
        .unwrap();
    serde_json::from_slice(&crpg_data::write_document(doc).unwrap()).unwrap()
}

fn read_bytes(value: &serde_json::Value) -> Result<Document, crpg_data::DataError> {
    let bytes = crpg_data::canonical_json(value).unwrap();
    crpg_data::read_document(&bytes)
}

#[test]
fn structural_shapes_reject_malformed_combat() {
    // Unknown field on the ability envelope.
    let mut bad = ability_value();
    bad["extra"] = serde_json::json!(0);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Trailing field on a unit outcome variant.
    let mut bad = ability_value();
    bad["damage"][0]["outcome"] = serde_json::json!({"type": "success", "value": 1});
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Missing value on a data outcome variant.
    let mut bad = ability_value();
    bad["damage"][0]["outcome"] = serde_json::json!({"type": "custom"});
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Unknown outcome label.
    let mut bad = ability_value();
    bad["damage"][0]["outcome"] = serde_json::json!({"type": "bogus"});
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Empty dice, empty damage, zero cost.
    let mut bad = ability_value();
    bad["dice"] = serde_json::json!("");
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut bad = ability_value();
    bad["damage"] = serde_json::json!([]);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut bad = ability_value();
    bad["cost"] = serde_json::json!(0);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Duplicate damage outcomes.
    let mut bad = ability_value();
    bad["damage"] = serde_json::json!([
        {"outcome": {"type": "success"}, "amount": 2},
        {"outcome": {"type": "success"}, "amount": 3}
    ]);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Unknown schema tag and future version for a new family.
    let mut bad = ability_value();
    bad["schema"] = serde_json::json!("crpg.ability/3");
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::UnsupportedSchema { .. })
    ));
    let mut bad = ability_value();
    bad["schema"] = serde_json::json!("crpg.nope/1");
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::UnsupportedSchema { .. })
    ));
}

#[test]
fn dice_byte_limit_has_largest_valid_and_first_invalid() {
    let mut valid = ability_value();
    valid["dice"] = serde_json::json!("x".repeat(128));
    assert!(read_bytes(&valid).is_ok());
    let mut invalid = ability_value();
    invalid["dice"] = serde_json::json!("x".repeat(129));
    assert!(matches!(
        read_bytes(&invalid),
        Err(crpg_data::DataError::Malformed { .. })
    ));
}

fn table_value() -> serde_json::Value {
    let (_, loaded) = load_combat();
    let doc = loaded
        .documents
        .values()
        .find(|doc| matches!(doc, Document::OutcomeTable(_)))
        .unwrap();
    serde_json::from_slice(&crpg_data::write_document(doc).unwrap()).unwrap()
}

fn bands(count: usize) -> serde_json::Value {
    let mut items = vec![serde_json::json!({
        "min_margin": i64::MIN,
        "outcome": {"type": "success"}
    })];
    for n in 1..count {
        items.push(serde_json::json!({
            "min_margin": n as i64,
            "outcome": {"type": "failure"}
        }));
    }
    serde_json::Value::Array(items)
}

#[test]
fn band_limits_and_order_reject_first_invalid() {
    let mut valid = table_value();
    valid["bands"] = bands(256);
    assert!(read_bytes(&valid).is_ok());
    let mut invalid = table_value();
    invalid["bands"] = bands(257);
    assert!(matches!(
        read_bytes(&invalid),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut invalid = table_value();
    invalid["bands"] = serde_json::json!([]);
    assert!(matches!(
        read_bytes(&invalid),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut invalid = table_value();
    invalid["bands"][0]["min_margin"] = serde_json::json!(i64::MIN + 1);
    assert!(matches!(
        read_bytes(&invalid),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut invalid = table_value();
    invalid["bands"][1]["min_margin"] = serde_json::json!(1);
    invalid["bands"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "min_margin": 1,
            "outcome": {"type": "failure"}
        }));
    assert!(matches!(
        read_bytes(&invalid),
        Err(crpg_data::DataError::Malformed { .. })
    ));
}

#[test]
fn natural_faces_reject_bounds_and_duplicates() {
    let faces = |list: Vec<u32>| {
        list.into_iter()
            .map(|face| {
                serde_json::json!({
                    "face": face,
                    "effect": {"type": "shift", "value": 1}
                })
            })
            .collect::<Vec<_>>()
    };
    let mut valid = table_value();
    valid["natural_rules"] = serde_json::Value::Array(faces((1..=256).collect()));
    assert!(read_bytes(&valid).is_ok());
    for bad_faces in [
        (0..=256).collect::<Vec<_>>(),
        (1..=257).collect::<Vec<_>>(),
        vec![1, 1],
        vec![1_000_001],
        vec![0],
    ] {
        let mut invalid = table_value();
        invalid["natural_rules"] = serde_json::Value::Array(faces(bad_faces));
        assert!(matches!(
            read_bytes(&invalid),
            Err(crpg_data::DataError::Malformed { .. })
        ));
    }
    let mut invalid = table_value();
    invalid["natural_rules"] = serde_json::json!([
        {"face": 6, "effect": {"type": "shift", "value": 1}, "extra": 0}
    ]);
    assert!(matches!(
        read_bytes(&invalid),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut face_max = table_value();
    face_max["natural_rules"] = serde_json::Value::Array(faces(vec![1_000_000]));
    assert!(read_bytes(&face_max).is_ok());
}

fn validate_docs(docs: &BTreeMap<SourcePath, Document>) -> Vec<crpg_data::Diagnostic> {
    crpg_data::validate(&LoadedCampaign {
        documents: docs.clone(),
        index: BTreeMap::new(),
    })
}

fn mutate_combat(
    change: impl FnOnce(&mut BTreeMap<SourcePath, Document>),
) -> Vec<crpg_data::Diagnostic> {
    let (_, loaded) = load_combat();
    let mut docs = loaded.documents.clone();
    change(&mut docs);
    validate_docs(&docs)
}

fn ability_mut(mut change: impl FnMut(&mut crpg_data::Ability)) -> Vec<crpg_data::Diagnostic> {
    mutate_combat(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ability(ability) = doc {
                change(ability);
            }
        }
    })
}

#[test]
fn combat_references_report_positioned_codes() {
    // Dangling outcome table.
    let findings = ability_mut(|ability| {
        ability.outcome_table = id(999);
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::DanglingReference);
    assert_eq!(findings[0].pointer, "/outcome_table".to_string());
    // Wrong kind for the outcome table.
    let findings = ability_mut(|ability| {
        ability.outcome_table = id(16);
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::WrongReferenceKind);
    // Unknown attribute name.
    let findings = ability_mut(|ability| {
        ability.attribute = "bogus".into();
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::UnknownStat);
    assert_eq!(findings[0].pointer, "/attribute".to_string());
    // Health is declared but is not an attribute.
    let findings = ability_mut(|ability| {
        ability.attribute = "health".into();
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::UnknownStat);
    // Cost above the pool maximum.
    let findings = ability_mut(|ability| {
        ability.cost = 2;
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::InvalidCost);
    assert_eq!(findings[0].pointer, "/cost".to_string());
    // Dangling encounter ruleset suppresses participant checks.
    let findings = mutate_combat(|docs| {
        for doc in docs.values_mut() {
            if let Document::Encounter(encounter) = doc {
                encounter.ruleset = id(999);
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::DanglingReference);
}

#[test]
fn participant_creature_coverage_reports_codes() {
    // Missing ruleset stat on the hero.
    let findings = mutate_combat(|docs| {
        for doc in docs.values_mut() {
            if let Document::Creature(creature) = doc {
                if creature.id == id(16) {
                    creature.stats.remove("guile");
                }
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::MissingStat);
    assert_eq!(findings[0].pointer, "/participants/0/placement".to_string());
    // Fractional combat stat value.
    let findings = mutate_combat(|docs| {
        for doc in docs.values_mut() {
            if let Document::Creature(creature) = doc {
                if creature.id == id(16) {
                    creature.stats.insert("might".into(), Fx16_16::from_raw(1));
                }
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::InvalidStatValue);
    // Zero initial health.
    let findings = mutate_combat(|docs| {
        for doc in docs.values_mut() {
            if let Document::Creature(creature) = doc {
                if creature.id == id(17) {
                    creature.stats.insert("health".into(), Fx16_16::from_int(0));
                }
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::InvalidStatValue);
    // Item prefab is not a combat creature.
    let findings = mutate_combat(|docs| {
        docs.insert(
            "items/item.json".parse().unwrap(),
            Document::Item(crpg_data::Item {
                id: id(30),
                slug: "item".into(),
                name: "fixture.item".into(),
                note: None,
                stats: BTreeMap::new(),
                tags: Vec::new(),
            }),
        );
        for doc in docs.values_mut() {
            if let Document::Placements(placements) = doc {
                for placement in &mut placements.placements {
                    if placement.id == id(14) {
                        placement.prefab = id(30);
                    }
                }
            }
        }
    });
    assert!(findings.iter().any(|finding| {
        finding.code == DiagnosticCode::WrongReferenceKind
            && finding.pointer == "/participants/0/placement"
    }));
}

#[test]
fn structural_failures_win_over_semantic_findings() {
    let mut files = read_tree(&combat_root());
    files.insert(
        "abilities/strike.json".parse().unwrap(),
        b"{invalid".to_vec(),
    );
    let findings = crpg_data::validate_files(&files, &engine());
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::Malformed);
}

#[test]
fn extreme_initiatives_and_max_damage_load() {
    let findings = mutate_combat(|docs| {
        for doc in docs.values_mut() {
            if let Document::Encounter(encounter) = doc {
                encounter.participants[0].initiative = i32::MIN;
                encounter.participants[1].initiative = i32::MAX;
            }
            if let Document::Ability(ability) = doc {
                ability.damage[0].amount = u32::MAX;
            }
        }
    });
    assert_eq!(findings, Vec::new());
}

#[test]
fn campaign_paths_accept_new_families_and_reject_others() {
    for accepted in [
        "rulesets/minimal-d6.json",
        "rulesets/nested/ruleset.json",
        "abilities/strike.json",
        "outcome_tables/roll-under.json",
        "encounters/first-blood.json",
    ] {
        assert!(
            crpg_data::campaign_document_path(accepted)
                .unwrap()
                .is_some(),
            "{accepted}"
        );
    }
    assert_eq!(
        crpg_data::campaign_document_path("assets/model.glb").unwrap(),
        None
    );
    // Misplaced combat documents fail layout, not classification.
    let (_, loaded) = load_combat();
    let mut docs = loaded.documents.clone();
    let ability = docs
        .remove(&"abilities/strike.json".parse().unwrap())
        .unwrap();
    docs.insert("creatures/strike.json".parse().unwrap(), ability);
    let error = crpg_data::serialize_campaign(&LoadedCampaign {
        documents: docs,
        index: BTreeMap::new(),
    })
    .unwrap_err();
    assert!(matches!(error, crpg_data::DataError::Layout { .. }));
}

#[test]
fn combat_objects_explain_with_typed_edges() {
    let (_, loaded) = load_combat();
    let report = crpg_data::explain_object(&loaded, id(19))
        .expect("structural")
        .expect("ability present");
    let value: serde_json::Value = serde_json::from_slice(&report).unwrap();
    assert_eq!(value["kind"], serde_json::json!("ability"));
    let outbound = value["outbound"].as_array().unwrap();
    assert!(outbound.iter().any(|edge| {
        edge["pointer"] == serde_json::json!("/outcome_table")
            && edge["target_location"]["kind"] == serde_json::json!("outcome_table")
    }));
    let report = crpg_data::explain_object(&loaded, id(20))
        .expect("structural")
        .expect("table present");
    let value: serde_json::Value = serde_json::from_slice(&report).unwrap();
    let inbound = value["inbound"].as_array().unwrap();
    assert!(inbound
        .iter()
        .any(|edge| { edge["target"] == serde_json::json!(id(20).to_string()) }));
}

#[test]
fn new_families_register_version_one_without_edges() {
    let versions = crpg_data::schema_versions();
    assert_eq!(versions.len(), 20);
    for (family, current) in [
        ("crpg.ruleset", 2),
        ("crpg.ability", 2),
        ("crpg.outcome-table", 1),
        ("crpg.encounter", 1),
        ("crpg.effect", 1),
    ] {
        let entry = versions
            .iter()
            .find(|entry| entry.schema_type == family)
            .expect("combat family registered");
        assert_eq!(entry.current, current, "{family}");
    }
}

#[test]
fn duplicate_slugs_and_missing_locale_cover_new_kinds() {
    let findings = mutate_combat(|docs| {
        docs.insert(
            "abilities/second.json".parse().unwrap(),
            Document::Ability(crpg_data::Ability {
                id: id(31),
                slug: "strike".into(),
                name: "fixture.strike".into(),
                note: None,
                dice: "2d6".into(),
                attribute: "might".into(),
                outcome_table: id(20),
                damage: vec![crpg_data::DamageEntry {
                    outcome: crpg_data::OutcomeWire::Success,
                    amount: 1,
                }],
                cost: 1,
                extra_costs: Vec::new(),
                ends_turn: true,
                effect: None,
                defense: crpg_data::DefenseWire::ActorAttribute,
                natural_die: None,
                requires_target: true,
                allow_self_target: false,
            }),
        );
    });
    assert!(findings.iter().any(|finding| {
        finding.code == DiagnosticCode::DuplicateSlug && finding.pointer == "/slug"
    }));
    let findings = mutate_combat(|docs| {
        for doc in docs.values_mut() {
            if let Document::Locale(locale) = doc {
                locale.strings.remove("fixture.strike");
            }
        }
    });
    assert!(findings.iter().any(|finding| {
        finding.code == DiagnosticCode::MissingLocaleKey && finding.pointer == "/name"
    }));
}

fn ruleset_value() -> serde_json::Value {
    let (_, loaded) = load_combat();
    let doc = loaded
        .documents
        .values()
        .find(|doc| matches!(doc, Document::Ruleset(_)))
        .unwrap();
    serde_json::from_slice(&crpg_data::write_document(doc).unwrap()).unwrap()
}

fn encounter_value() -> serde_json::Value {
    let (_, loaded) = load_combat();
    let doc = loaded
        .documents
        .values()
        .find(|doc| matches!(doc, Document::Encounter(_)))
        .unwrap();
    serde_json::from_slice(&crpg_data::write_document(doc).unwrap()).unwrap()
}

#[test]
fn combat_remaining_local_edges_reject_malformed() {
    // Zero tick period on the action-pool refresh.
    let mut bad = ruleset_value();
    bad["pools"][0]["refresh"] = serde_json::json!({"type": "on_tick", "value": 0});
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Trailing value on a unit refresh variant.
    let mut bad = ruleset_value();
    bad["pools"][0]["refresh"] = serde_json::json!({"type": "on_turn_start", "value": 1});
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Missing value on a payload refresh variant.
    let mut bad = ruleset_value();
    bad["pools"][0]["refresh"] = serde_json::json!({"type": "on_rest"});
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Trailing field inside a natural effect.
    let mut bad = table_value();
    bad["natural_rules"] = serde_json::json!([
        {"face": 6, "effect": {"type": "shift", "value": 1, "extra": 0}}
    ]);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Duplicate encounter participant placement.
    let mut bad = encounter_value();
    let first = bad["participants"][0].clone();
    bad["participants"].as_array_mut().unwrap().push(first);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Empty encounter participants.
    let mut bad = encounter_value();
    bad["participants"] = serde_json::json!([]);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Duplicate ruleset stat name.
    let mut bad = ruleset_value();
    let first = bad["stats"][0].clone();
    bad["stats"].as_array_mut().unwrap().push(first);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Empty ruleset attributes.
    let mut bad = ruleset_value();
    bad["attributes"] = serde_json::json!([]);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
}

#[test]
fn combat_health_boundary_and_byte_parity() {
    // Health 1 is the smallest valid initial health.
    let findings = mutate_combat(|docs| {
        for doc in docs.values_mut() {
            if let Document::Creature(creature) = doc {
                if creature.id == id(17) {
                    creature.stats.insert("health".into(), Fx16_16::from_int(1));
                }
            }
        }
    });
    assert_eq!(findings, Vec::new());
    // Campaign combat documents are byte-identical to the canonical source.
    let source = read_tree(&ruleset_root());
    let campaign_files = read_tree(&combat_root());
    let pairs = [
        ("ruleset.json", "rulesets/minimal-d6.json"),
        ("strike.json", "abilities/strike.json"),
        ("roll_under.json", "outcome_tables/roll-under.json"),
    ];
    for (source_name, campaign_name) in pairs {
        let source_key: SourcePath = source_name.parse().unwrap();
        let campaign_key: SourcePath = campaign_name.parse().unwrap();
        assert_eq!(
            source[&source_key], campaign_files[&campaign_key],
            "byte parity for {source_name}"
        );
    }
}
