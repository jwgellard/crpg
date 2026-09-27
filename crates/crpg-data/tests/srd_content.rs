#![forbid(unsafe_code)]
//! T017a second-ruleset vocabulary and srd-lite content acceptance.
use crpg_core::{Fx16_16, Ulid};
use crpg_data::{DiagnosticCode, Document, LoadedCampaign, ObjectKind, SourcePath};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn srd_root() -> PathBuf {
    crate_dir()
        .join("..")
        .join("..")
        .join("campaigns")
        .join("fixtures")
        .join("combat_srd")
}

fn ruleset_root() -> PathBuf {
    crate_dir()
        .join("..")
        .join("..")
        .join("rulesets")
        .join("srd-lite")
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

fn load_srd() -> (BTreeMap<SourcePath, Vec<u8>>, LoadedCampaign) {
    let files = read_tree(&srd_root());
    let loaded = crpg_data::load_campaign(&files, &engine()).expect("combat_srd loads");
    (files, loaded)
}

fn id(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

#[test]
fn combat_srd_loads_validates_clean_and_round_trips() {
    let (files, loaded) = load_srd();
    assert_eq!(files.len(), 17);
    assert_eq!(loaded.index.len(), 14);
    let expected: Vec<(Ulid, ObjectKind, &str, &str)> = vec![
        (id(31), ObjectKind::Campaign, "campaign.json", ""),
        (id(32), ObjectKind::World, "worlds/world.json", ""),
        (id(33), ObjectKind::Area, "areas/start/area.json", ""),
        (
            id(34),
            ObjectKind::Placement,
            "areas/start/placements.json",
            "/placements/0",
        ),
        (
            id(35),
            ObjectKind::Placement,
            "areas/start/placements.json",
            "/placements/1",
        ),
        (id(36), ObjectKind::Creature, "creatures/hero.json", ""),
        (id(37), ObjectKind::Creature, "creatures/goblin.json", ""),
        (id(38), ObjectKind::Ruleset, "rulesets/srd-lite.json", ""),
        (id(39), ObjectKind::Ability, "abilities/heavy.json", ""),
        (id(40), ObjectKind::Ability, "abilities/focus.json", ""),
        (
            id(41),
            ObjectKind::OutcomeTable,
            "outcome_tables/heavy_table.json",
            "",
        ),
        (
            id(42),
            ObjectKind::OutcomeTable,
            "outcome_tables/focus_table.json",
            "",
        ),
        (id(43), ObjectKind::Effect, "effects/focusing.json", ""),
        (
            id(44),
            ObjectKind::Encounter,
            "encounters/first-blood.json",
            "",
        ),
    ];
    for (want_id, want_kind, want_path, want_pointer) in expected {
        let entry = loaded.index.get(&want_id).expect("indexed id");
        assert_eq!(entry.kind, want_kind, "{want_id}");
        assert_eq!(entry.path.as_str(), want_path);
        assert_eq!(entry.pointer, want_pointer);
    }
    // Pool templates and effect modifiers are values, never indexed objects.
    assert!(!loaded.index.contains_key(&id(46)));
    assert!(!loaded.index.contains_key(&id(47)));
    assert!(!loaded.index.contains_key(&id(48)));
    assert!(!loaded.index.contains_key(&id(49)));
    assert_eq!(crpg_data::validate(&loaded), Vec::new());
    let once = crpg_data::serialize_campaign(&loaded).expect("serializes");
    assert_eq!(once, files);
    let reloaded = crpg_data::load_campaign(&once, &engine()).expect("reloads");
    let twice = crpg_data::serialize_campaign(&reloaded).expect("reserializes");
    assert_eq!(twice, files);
}

#[test]
fn canonical_source_parity_and_byte_identity() {
    let source = read_tree(&ruleset_root());
    assert_eq!(source.len(), 6);
    let (_, loaded) = load_srd();
    for (logical, bytes) in &source {
        let doc = crpg_data::read_document(bytes).expect("source reads");
        let found = loaded.documents.values().any(|candidate| candidate == &doc);
        assert!(found, "source {logical} has a campaign copy");
    }
    let pairs = [
        ("ruleset.json", "rulesets/srd-lite.json"),
        ("heavy.json", "abilities/heavy.json"),
        ("focus.json", "abilities/focus.json"),
        ("heavy_table.json", "outcome_tables/heavy_table.json"),
        ("focus_table.json", "outcome_tables/focus_table.json"),
        ("focusing.json", "effects/focusing.json"),
    ];
    let campaign_files = read_tree(&srd_root());
    for (source_name, campaign_name) in pairs {
        let source_key: SourcePath = source_name.parse().unwrap();
        let campaign_key: SourcePath = campaign_name.parse().unwrap();
        assert_eq!(
            source[&source_key], campaign_files[&campaign_key],
            "byte parity for {source_name}"
        );
    }
}

#[test]
fn srd_lite_content_pins() {
    let (_, loaded) = load_srd();
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
        vec!["might", "guile", "resolve", "ward", "health"]
    );
    assert_eq!(ruleset.health_stat, "health");
    assert_eq!(ruleset.attributes, vec!["might", "guile", "resolve"]);
    assert_eq!(ruleset.pools.len(), 2);
    assert_eq!(ruleset.pools[0].id, id(46));
    assert_eq!(ruleset.pools[0].max, 1);
    assert!(matches!(
        ruleset.pools[0].refresh,
        crpg_data::RefreshWire::OnTurnStart
    ));
    assert_eq!(ruleset.pools[1].id, id(47));
    assert_eq!(ruleset.pools[1].max, 2);
    assert!(matches!(
        ruleset.pools[1].refresh,
        crpg_data::RefreshWire::OnRoundStart
    ));
    assert_eq!(ruleset.abilities, vec![id(39), id(40)]);
    assert_eq!(ruleset.package.as_str(), "srd-lite");

    let heavy = loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            Document::Ability(v) if v.id == id(39) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(heavy.dice, "2d6");
    assert_eq!(heavy.attribute, "might");
    assert!(matches!(
        heavy.defense,
        crpg_data::DefenseWire::TargetStat { ref stat } if stat == "ward"
    ));
    assert_eq!(heavy.outcome_table, id(41));
    assert_eq!(heavy.damage.len(), 3);
    assert!(matches!(
        heavy.damage[0].outcome,
        crpg_data::OutcomeWire::Failure
    ));
    assert_eq!(heavy.damage[0].amount, 0);
    assert!(matches!(
        heavy.damage[1].outcome,
        crpg_data::OutcomeWire::Success
    ));
    assert_eq!(heavy.damage[1].amount, 3);
    assert!(matches!(
        heavy.damage[2].outcome,
        crpg_data::OutcomeWire::CriticalSuccess
    ));
    assert_eq!(heavy.damage[2].amount, 5);
    assert_eq!(heavy.cost, 1);
    assert!(heavy.extra_costs.is_empty());
    assert!(heavy.ends_turn);
    assert_eq!(heavy.effect, None);
    assert_eq!(heavy.natural_die, Some(0));
    assert!(heavy.requires_target);
    assert!(!heavy.allow_self_target);

    let focus = loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            Document::Ability(v) if v.id == id(40) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(focus.dice, "1d6");
    assert_eq!(focus.attribute, "might");
    assert!(matches!(
        focus.defense,
        crpg_data::DefenseWire::ActorAttribute
    ));
    assert_eq!(focus.outcome_table, id(42));
    assert_eq!(focus.damage.len(), 2);
    assert_eq!(focus.cost, 0);
    assert_eq!(focus.extra_costs.len(), 1);
    assert_eq!(focus.extra_costs[0].pool, id(47));
    assert_eq!(focus.extra_costs[0].amount, 1);
    assert!(!focus.ends_turn);
    assert_eq!(focus.effect, Some(id(43)));
    assert_eq!(focus.natural_die, None);
    assert!(focus.requires_target);
    assert!(focus.allow_self_target);

    let heavy_table = loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            Document::OutcomeTable(v) if v.id == id(41) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(heavy_table.bands.len(), 3);
    assert_eq!(heavy_table.bands[0].min_margin, i64::MIN);
    assert!(matches!(
        heavy_table.bands[0].outcome,
        crpg_data::OutcomeWire::Failure
    ));
    assert_eq!(heavy_table.bands[1].min_margin, -2);
    assert!(matches!(
        heavy_table.bands[1].outcome,
        crpg_data::OutcomeWire::Success
    ));
    assert_eq!(heavy_table.bands[2].min_margin, 2);
    assert!(matches!(
        heavy_table.bands[2].outcome,
        crpg_data::OutcomeWire::CriticalSuccess
    ));
    assert_eq!(heavy_table.natural_rules.len(), 1);
    assert_eq!(heavy_table.natural_rules[0].face, 6);
    assert!(matches!(
        heavy_table.natural_rules[0].effect,
        crpg_data::NaturalEffectWire::Shift(1)
    ));

    let focus_table = loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            Document::OutcomeTable(v) if v.id == id(42) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(focus_table.bands.len(), 2);
    assert_eq!(focus_table.bands[0].min_margin, i64::MIN);
    assert!(matches!(
        focus_table.bands[0].outcome,
        crpg_data::OutcomeWire::Success
    ));
    assert_eq!(focus_table.bands[1].min_margin, 1);
    assert!(matches!(
        focus_table.bands[1].outcome,
        crpg_data::OutcomeWire::Failure
    ));
    assert!(focus_table.natural_rules.is_empty());

    let effect = loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            Document::Effect(v) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(effect.id, id(43));
    assert_eq!(effect.slug, "focusing");
    assert!(matches!(effect.aim, crpg_data::EffectAimWire::Slf));
    assert_eq!(effect.mod_type, "focus");
    assert!(matches!(effect.policy, crpg_data::PolicyWire::StackAll));
    assert_eq!(effect.modifiers.len(), 2);
    assert_eq!(effect.modifiers[0].id, id(48));
    assert!(matches!(
        effect.modifiers[0].target,
        crpg_data::EffectTargetWire::Roll
    ));
    assert!(matches!(
        effect.modifiers[0].op,
        crpg_data::EffectOpWire::Add
    ));
    assert_eq!(effect.modifiers[0].value, 2);
    assert_eq!(effect.modifiers[0].priority, 0);
    assert_eq!(effect.modifiers[1].id, id(49));
    assert_eq!(effect.modifiers[1].value, 1);
    assert_eq!(effect.modifiers[1].priority, 1);
    assert_eq!(effect.duration_rounds, 2);

    let encounter = loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            Document::Encounter(v) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(encounter.ruleset, id(38));
    assert_eq!(encounter.participants[0].placement, id(34));
    assert_eq!(encounter.participants[0].initiative, 10);
    assert_eq!(encounter.participants[1].placement, id(35));
    assert_eq!(encounter.participants[1].initiative, 9);

    for creature_id in [id(36), id(37)] {
        let creature = loaded
            .documents
            .values()
            .find_map(|doc| match doc {
                Document::Creature(v) if v.id == creature_id => Some(v),
                _ => None,
            })
            .unwrap();
        for stat in ["might", "guile", "resolve", "ward", "health"] {
            let value = creature.stats.get(stat).unwrap();
            assert_eq!(value.to_raw() % 65536, 0, "{stat}");
        }
    }
    let hero = loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            Document::Creature(v) if v.id == id(36) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(hero.stats.get("ward").unwrap().to_raw(), 7 * 65536);
    assert_eq!(hero.stats.get("health").unwrap().to_raw(), 10 * 65536);
}

fn effect_value() -> serde_json::Value {
    let (_, loaded) = load_srd();
    let doc = loaded
        .documents
        .values()
        .find(|doc| matches!(doc, Document::Effect(_)))
        .unwrap();
    serde_json::from_slice(&crpg_data::write_document(doc).unwrap()).unwrap()
}

fn ability_value(which: u128) -> serde_json::Value {
    let (_, loaded) = load_srd();
    let doc = loaded
        .documents
        .values()
        .find(|doc| matches!(doc, Document::Ability(v) if v.id == id(which)))
        .unwrap();
    serde_json::from_slice(&crpg_data::write_document(doc).unwrap()).unwrap()
}

fn ruleset_value() -> serde_json::Value {
    let (_, loaded) = load_srd();
    let doc = loaded
        .documents
        .values()
        .find(|doc| matches!(doc, Document::Ruleset(_)))
        .unwrap();
    serde_json::from_slice(&crpg_data::write_document(doc).unwrap()).unwrap()
}

fn read_bytes(value: &serde_json::Value) -> Result<Document, crpg_data::DataError> {
    let bytes = crpg_data::canonical_json(value).unwrap();
    crpg_data::read_document(&bytes)
}

#[test]
fn structural_tags_reject_mismatches_and_futures() {
    // A /2 ability missing its new required defense fails typed decode.
    let mut bad = ability_value(39);
    bad.as_object_mut().unwrap().remove("defense");
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // A /2 ruleset missing pools fails typed decode.
    let mut bad = ruleset_value();
    bad.as_object_mut().unwrap().remove("pools");
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Future versions are unsupported with the original tag preserved.
    for tag in ["crpg.ability/3", "crpg.ruleset/3", "crpg.effect/2"] {
        let mut bad = ability_value(39);
        if tag.starts_with("crpg.ruleset") {
            bad = ruleset_value();
        } else if tag.starts_with("crpg.effect") {
            bad = effect_value();
        }
        bad["schema"] = serde_json::json!(tag);
        assert!(
            matches!(
                read_bytes(&bad),
                Err(crpg_data::DataError::UnsupportedSchema { .. })
            ),
            "{tag}"
        );
    }
    // Unknown family.
    let mut bad = effect_value();
    bad["schema"] = serde_json::json!("crpg.nope/1");
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::UnsupportedSchema { .. })
    ));
}

#[test]
fn structural_unknown_fields_reject_including_unit_variants() {
    // Unknown field on the effect envelope.
    let mut bad = effect_value();
    bad["extra"] = serde_json::json!(0);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Trailing value on each unit defense/aim/target/op/policy variant.
    let mut bad = ability_value(40);
    bad["defense"] = serde_json::json!({"type": "actor_attribute", "value": 1});
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut bad = effect_value();
    bad["aim"] = serde_json::json!({"type": "slf", "value": 1});
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut bad = effect_value();
    bad["modifiers"][0]["target"] = serde_json::json!({"type": "roll", "value": 1});
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut bad = effect_value();
    bad["modifiers"][0]["op"] = serde_json::json!({"type": "add", "extra": 0});
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut bad = effect_value();
    bad["policy"] = serde_json::json!({"type": "stack_all", "value": 1});
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Trailing field inside a TargetStat value object.
    let mut bad = ability_value(39);
    bad["defense"] = serde_json::json!({
        "type": "target_stat",
        "value": {"stat": "ward", "extra": 0}
    });
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Unknown defense/aim/target/op/policy labels.
    let mut bad = ability_value(39);
    bad["defense"] = serde_json::json!({"type": "bogus"});
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut bad = effect_value();
    bad["aim"] = serde_json::json!({"type": "bogus"});
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Missing required fields.
    let mut bad = effect_value();
    bad.as_object_mut().unwrap().remove("mod_type");
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut bad = ability_value(39);
    bad.as_object_mut().unwrap().remove("defense");
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Empty pools/modifiers, duplicate ids, zero duration.
    let mut bad = ruleset_value();
    bad["pools"] = serde_json::json!([]);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut bad = ruleset_value();
    let first = bad["pools"][0].clone();
    bad["pools"].as_array_mut().unwrap().push(first);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut bad = effect_value();
    bad["modifiers"] = serde_json::json!([]);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut bad = effect_value();
    let first = bad["modifiers"][0].clone();
    bad["modifiers"].as_array_mut().unwrap().push(first);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut bad = effect_value();
    bad["duration_rounds"] = serde_json::json!(0);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Non-u32 natural index shapes.
    for natural in [serde_json::json!("0"), serde_json::json!(-1)] {
        let mut bad = ability_value(39);
        bad["natural_die"] = natural;
        assert!(matches!(
            read_bytes(&bad),
            Err(crpg_data::DataError::Malformed { .. })
        ));
    }
    // A floating natural index fails at the integer-only parse stage.
    let bad = ability_value(39);
    let raw = crpg_data::canonical_json(&bad).unwrap();
    let text = String::from_utf8(raw)
        .unwrap()
        .replace("\"natural_die\": 0", "\"natural_die\": 1.5");
    let raw = text.into_bytes();
    assert!(matches!(
        crpg_data::read_document(&raw),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Zero extra amount is malformed at read time.
    let mut bad = ability_value(40);
    bad["extra_costs"][0]["amount"] = serde_json::json!(0);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Empty total spend is malformed at read time.
    let mut bad = ability_value(40);
    bad["cost"] = serde_json::json!(0);
    bad["extra_costs"] = serde_json::json!([]);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Over-limit modifiers fail as malformed.
    let mut bad = effect_value();
    let one = bad["modifiers"][0].clone();
    let mut items = Vec::new();
    for n in 0..4097 {
        let mut entry = one.clone();
        entry["id"] = serde_json::json!(format!("0000000000000000000000{:04}", n));
        items.push(entry);
    }
    bad["modifiers"] = serde_json::Value::Array(items);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
}

fn validate_docs(docs: &BTreeMap<SourcePath, Document>) -> Vec<crpg_data::Diagnostic> {
    crpg_data::validate(&LoadedCampaign {
        documents: docs.clone(),
        index: BTreeMap::new(),
    })
}

fn mutate_srd(
    change: impl FnOnce(&mut BTreeMap<SourcePath, Document>),
) -> Vec<crpg_data::Diagnostic> {
    let (_, loaded) = load_srd();
    let mut docs = loaded.documents.clone();
    change(&mut docs);
    validate_docs(&docs)
}

#[test]
fn srd_references_report_positioned_codes() {
    // Dangling effect.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ability(ability) = doc {
                if ability.id == id(40) {
                    ability.effect = Some(id(999));
                }
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::DanglingReference);
    assert_eq!(findings[0].pointer, "/effect".to_string());
    // Wrong kind for the effect.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ability(ability) = doc {
                if ability.id == id(40) {
                    ability.effect = Some(id(36));
                }
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::WrongReferenceKind);
    // Dangling outcome table still reports generically.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ability(ability) = doc {
                if ability.id == id(39) {
                    ability.outcome_table = id(999);
                }
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::DanglingReference);
    // Unknown extra pool names ability and pool.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ability(ability) = doc {
                if ability.id == id(40) {
                    ability.extra_costs[0].pool = id(999);
                }
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::UnknownPool);
    assert_eq!(findings[0].pointer, "/extra_costs/0/pool".to_string());
    assert!(findings[0].message.contains(&id(40).to_string()));
    // Cost above the primary maximum.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ability(ability) = doc {
                if ability.id == id(39) {
                    ability.cost = 2;
                }
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::InvalidCost);
    assert_eq!(findings[0].pointer, "/cost".to_string());
    // Extra amount above its pool maximum.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ability(ability) = doc {
                if ability.id == id(40) {
                    ability.extra_costs[0].amount = 3;
                }
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::InvalidCost);
    assert_eq!(findings[0].pointer, "/extra_costs/0/amount".to_string());
    // Empty total spend is invalid_cost at /cost.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ability(ability) = doc {
                if ability.id == id(40) {
                    ability.cost = 0;
                    ability.extra_costs.clear();
                }
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::InvalidCost);
    assert_eq!(findings[0].pointer, "/cost".to_string());
    // Undeclared defense stat.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ability(ability) = doc {
                if ability.id == id(39) {
                    ability.defense = crpg_data::DefenseWire::TargetStat {
                        stat: "bogus".into(),
                    };
                }
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::UnknownStat);
    assert_eq!(findings[0].pointer, "/defense".to_string());
    // Health-as-defense is unknown_stat.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ability(ability) = doc {
                if ability.id == id(39) {
                    ability.defense = crpg_data::DefenseWire::TargetStat {
                        stat: "health".into(),
                    };
                }
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::UnknownStat);
    assert_eq!(findings[0].pointer, "/defense".to_string());
}

#[test]
fn participant_ward_coverage_reports_codes() {
    // Missing ward on the hero.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Creature(creature) = doc {
                if creature.id == id(36) {
                    creature.stats.remove("ward");
                }
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::MissingStat);
    assert_eq!(findings[0].pointer, "/participants/0/placement".to_string());
    // Fractional ward.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Creature(creature) = doc {
                if creature.id == id(36) {
                    creature.stats.insert("ward".into(), Fx16_16::from_raw(1));
                }
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::InvalidStatValue);
    // Zero health.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Creature(creature) = doc {
                if creature.id == id(37) {
                    creature.stats.insert("health".into(), Fx16_16::from_int(0));
                }
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::InvalidStatValue);
    // Structural beats semantic: broken bytes win over findings.
    let mut files = read_tree(&srd_root());
    files.insert(
        "abilities/heavy.json".parse().unwrap(),
        b"{invalid".to_vec(),
    );
    let findings = crpg_data::validate_files(&files, &engine());
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::Malformed);
}

#[test]
fn minimal_d6_v1_loads_with_preserved_meaning() {
    let root = crate_dir()
        .join("..")
        .join("..")
        .join("campaigns")
        .join("fixtures")
        .join("combat_basic");
    let mut files = BTreeMap::new();
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
    walk(&root, &root, &mut files);
    let loaded = crpg_data::load_campaign(&files, &engine()).unwrap();
    assert_eq!(crpg_data::validate(&loaded), Vec::new());
    let ruleset = loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            Document::Ruleset(v) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(ruleset.pools.len(), 1);
    let ability = loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            Document::Ability(v) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(ability.cost, 1);
    assert!(ability.extra_costs.is_empty());
    assert!(ability.ends_turn);
    assert_eq!(ability.effect, None);
    assert!(matches!(
        ability.defense,
        crpg_data::DefenseWire::ActorAttribute
    ));
    assert_eq!(ability.natural_die, None);
}

#[test]
fn boundaries_cover_largest_valid_and_first_invalid() {
    // Pools: empty invalid, one/two/three valid (unbounded, nonempty only).
    let mut bad = ruleset_value();
    bad["pools"] = serde_json::json!([]);
    assert!(matches!(
        read_bytes(&bad),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    let mut three = ruleset_value();
    let mut third = three["pools"][0].clone();
    third["id"] = serde_json::json!(id(60).to_string());
    three["pools"].as_array_mut().unwrap().push(third);
    assert!(read_bytes(&three).is_ok());
    // Modifiers: 4096 valid, 4097 invalid, empty invalid.
    let mut valid = effect_value();
    let one = valid["modifiers"][0].clone();
    // Build 4096 distinct modifier ids deterministically.
    let mut distinct = Vec::new();
    for n in 0..4096u128 {
        let mut entry = one.clone();
        entry["id"] = serde_json::json!(Ulid::from_u128(1000 + n).to_string());
        distinct.push(entry);
    }
    valid["modifiers"] = serde_json::Value::Array(distinct);
    assert!(read_bytes(&valid).is_ok());
    let mut over = effect_value();
    let mut many = Vec::new();
    for n in 0..4097u128 {
        let mut entry = one.clone();
        entry["id"] = serde_json::json!(Ulid::from_u128(1000 + n).to_string());
        many.push(entry);
    }
    over["modifiers"] = serde_json::Value::Array(many);
    assert!(matches!(
        read_bytes(&over),
        Err(crpg_data::DataError::Malformed { .. })
    ));
    // Duration 1 valid, 0 invalid.
    let mut one_round = effect_value();
    one_round["duration_rounds"] = serde_json::json!(1);
    assert!(read_bytes(&one_round).is_ok());
    // Priorities i16 MIN/MAX load.
    let mut bounds = effect_value();
    bounds["modifiers"][0]["priority"] = serde_json::json!(i16::MIN);
    bounds["modifiers"][1]["priority"] = serde_json::json!(i16::MAX);
    assert!(read_bytes(&bounds).is_ok());
    // Values i32 MIN/MAX load.
    let mut values = effect_value();
    values["modifiers"][0]["value"] = serde_json::json!(i32::MIN);
    values["modifiers"][1]["value"] = serde_json::json!(i32::MAX);
    assert!(read_bytes(&values).is_ok());
    // Initiatives and damage extremes load without semantic findings.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Encounter(encounter) = doc {
                encounter.participants[0].initiative = i32::MIN;
                encounter.participants[1].initiative = i32::MAX;
            }
            if let Document::Ability(ability) = doc {
                if ability.id == id(39) {
                    ability.damage[2].amount = u32::MAX;
                }
            }
        }
    });
    assert_eq!(findings, Vec::new());
    // Health 1 valid.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Creature(creature) = doc {
                if creature.id == id(37) {
                    creature.stats.insert("health".into(), Fx16_16::from_int(1));
                }
            }
        }
    });
    assert_eq!(findings, Vec::new());
}

#[test]
fn campaign_paths_accept_effects_and_reject_others() {
    for accepted in [
        "rulesets/srd-lite.json",
        "abilities/heavy.json",
        "abilities/focus.json",
        "outcome_tables/heavy_table.json",
        "outcome_tables/focus_table.json",
        "effects/focusing.json",
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
    let (_, loaded) = load_srd();
    let mut docs = loaded.documents.clone();
    let effect = docs
        .remove(&"effects/focusing.json".parse().unwrap())
        .unwrap();
    docs.insert("creatures/focusing.json".parse().unwrap(), effect);
    let error = crpg_data::serialize_campaign(&LoadedCampaign {
        documents: docs,
        index: BTreeMap::new(),
    })
    .unwrap_err();
    assert!(matches!(error, crpg_data::DataError::Layout { .. }));
}

#[test]
fn srd_objects_explain_with_typed_edges() {
    let (_, loaded) = load_srd();
    let report = crpg_data::explain_object(&loaded, id(40))
        .expect("structural")
        .expect("focus present");
    let value: serde_json::Value = serde_json::from_slice(&report).unwrap();
    assert_eq!(value["kind"], serde_json::json!("ability"));
    let outbound = value["outbound"].as_array().unwrap();
    assert!(outbound.iter().any(|edge| {
        edge["pointer"] == serde_json::json!("/outcome_table")
            && edge["target_location"]["kind"] == serde_json::json!("outcome_table")
    }));
    assert!(outbound.iter().any(|edge| {
        edge["pointer"] == serde_json::json!("/effect")
            && edge["target_location"]["kind"] == serde_json::json!("effect")
    }));
    assert!(outbound
        .iter()
        .any(|edge| { edge["pointer"] == serde_json::json!("/extra_costs/0/pool") }));
    let report = crpg_data::explain_object(&loaded, id(43))
        .expect("structural")
        .expect("effect present");
    let value: serde_json::Value = serde_json::from_slice(&report).unwrap();
    assert_eq!(value["kind"], serde_json::json!("effect"));
    let inbound = value["inbound"].as_array().unwrap();
    assert!(inbound
        .iter()
        .any(|edge| { edge["target"] == serde_json::json!(id(43).to_string()) }));
}

#[test]
fn registry_versions_and_spellings_cover_new_shapes() {
    let versions = crpg_data::schema_versions();
    assert_eq!(versions.len(), 20);
    for (family, current) in [
        ("crpg.ruleset", 2),
        ("crpg.ability", 2),
        ("crpg.effect", 1),
        ("crpg.outcome-table", 1),
        ("crpg.encounter", 1),
    ] {
        let entry = versions
            .iter()
            .find(|entry| entry.schema_type == family)
            .expect("family registered");
        assert_eq!(entry.current, current, "{family}");
    }
    let schemas = crpg_data::generated_schemas().unwrap();
    assert_eq!(schemas.len(), 22);
}

#[test]
fn duplicate_slugs_and_missing_locale_cover_effects() {
    let findings = mutate_srd(|docs| {
        docs.insert(
            "effects/second.json".parse().unwrap(),
            Document::Effect(crpg_data::Effect {
                id: id(60),
                slug: "focusing".into(),
                name: "fixture.focusing".into(),
                note: None,
                aim: crpg_data::EffectAimWire::Slf,
                mod_type: "focus".into(),
                policy: crpg_data::PolicyWire::StackAll,
                modifiers: vec![crpg_data::EffectModifierWire {
                    id: id(61),
                    target: crpg_data::EffectTargetWire::Roll,
                    op: crpg_data::EffectOpWire::Add,
                    value: 1,
                    priority: 0,
                    name: None,
                }],
                duration_rounds: 1,
            }),
        );
    });
    assert!(findings.iter().any(|finding| {
        finding.code == DiagnosticCode::DuplicateSlug && finding.pointer == "/slug"
    }));
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Locale(locale) = doc {
                locale.strings.remove("fixture.focusing");
            }
        }
    });
    assert!(findings.iter().any(|finding| {
        finding.code == DiagnosticCode::MissingLocaleKey && finding.pointer == "/name"
    }));
    // Unknown pool spelling is pinned.
    assert_eq!(DiagnosticCode::UnknownPool.as_str(), "unknown_pool");
}

#[test]
fn orphan_effects_keep_only_generic_checks_and_no_orphans_shipped() {
    // Minimal content has no orphans: every effect is listed by an ability
    // and every ability is listed by the ruleset.
    let (_, loaded) = load_srd();
    let ruleset = loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            Document::Ruleset(v) => Some(v),
            _ => None,
        })
        .unwrap();
    assert!(ruleset.abilities.contains(&id(39)));
    assert!(ruleset.abilities.contains(&id(40)));
    let focus = loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            Document::Ability(v) if v.id == id(40) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(focus.effect, Some(id(43)));
    // An orphan effect (listed by no ability) keeps no per-ruleset findings.
    let findings = mutate_srd(|docs| {
        docs.insert(
            "effects/orphan.json".parse().unwrap(),
            Document::Effect(crpg_data::Effect {
                id: id(70),
                slug: "orphan".into(),
                name: "fixture.focusing".into(),
                note: None,
                aim: crpg_data::EffectAimWire::Target,
                mod_type: "focus".into(),
                policy: crpg_data::PolicyWire::HighestBonusWorstPenalty,
                modifiers: vec![crpg_data::EffectModifierWire {
                    id: id(71),
                    target: crpg_data::EffectTargetWire::Dc,
                    op: crpg_data::EffectOpWire::Add,
                    value: 1,
                    priority: 0,
                    name: None,
                }],
                duration_rounds: 3,
            }),
        );
    });
    assert_eq!(findings, Vec::new());
    // An orphan ability skips attribute/cost/defense checks but keeps its
    // generic table reference.
    let findings = mutate_srd(|docs| {
        docs.insert(
            "abilities/orphan.json".parse().unwrap(),
            Document::Ability(crpg_data::Ability {
                id: id(72),
                slug: "orphan-ability".into(),
                name: "fixture.heavy".into(),
                note: None,
                dice: "1d6".into(),
                attribute: "bogus".into(),
                outcome_table: id(999),
                damage: vec![crpg_data::DamageEntry {
                    outcome: crpg_data::OutcomeWire::Success,
                    amount: 0,
                }],
                cost: 99,
                extra_costs: Vec::new(),
                ends_turn: true,
                effect: None,
                defense: crpg_data::DefenseWire::ActorAttribute,
                natural_die: None,
                requires_target: false,
                allow_self_target: false,
            }),
        );
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::DanglingReference);
}
