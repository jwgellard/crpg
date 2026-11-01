#![forbid(unsafe_code)]
//! Two-keys contract tests for T136a (optional real-time fields on the
//! ruleset and ability families), written from `tasks/T136.md`
//! §"T136a exact contract" alone. Test names carry the contract's test ids
//! (T2–T10); each test documents the Behaviour items it proves.
use crpg_data::{DataError, Document, LoadedCampaign, SourcePath};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read_repo(relative: &str) -> Vec<u8> {
    std::fs::read(repo_root().join(relative)).expect("required repository file")
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
                if path.file_name().unwrap() == ".gitattributes" {
                    continue;
                }
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(relative.parse().unwrap(), std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn srd_ruleset() -> Value {
    serde_json::from_slice(&read_repo("rulesets/srd-lite/ruleset.json")).unwrap()
}

fn heavy_ability() -> Value {
    serde_json::from_slice(&read_repo("rulesets/srd-lite/heavy.json")).unwrap()
}

fn read_value(value: &Value) -> Result<Document, DataError> {
    crpg_data::read_document(&serde_json::to_vec(value).unwrap())
}

fn malformed_message(result: Result<Document, DataError>) -> String {
    match result {
        Err(DataError::Malformed { message, .. }) => message,
        other => panic!("expected Malformed, got {other:?}"),
    }
}

fn contains(bytes: &[u8], needle: &str) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn offset(bytes: &[u8], needle: &str) -> usize {
    bytes
        .windows(needle.len())
        .position(|w| w == needle.as_bytes())
        .unwrap_or_else(|| panic!("missing {needle}"))
}

/// T2 (Behaviour 1, 2): a present `round_ticks` decodes, is written under the
/// canonical key sort, and a re-read gives an equal document.
#[test]
fn t2_ruleset_round_ticks_round_trips() {
    let mut value = srd_ruleset();
    value["round_ticks"] = json!(120);
    let doc = read_value(&value).expect("decodes");
    let Document::Ruleset(ruleset) = &doc else {
        panic!("expected ruleset, got {doc:?}")
    };
    assert_eq!(ruleset.round_ticks, Some(120));
    let bytes = crpg_data::write_document(&doc).unwrap();
    assert!(contains(&bytes, "\"round_ticks\": 120"));
    // Canonical key sort places round_ticks between pools and schema.
    assert!(offset(&bytes, "\"pools\"") < offset(&bytes, "\"round_ticks\""));
    assert!(offset(&bytes, "\"round_ticks\"") < offset(&bytes, "\"schema\""));
    let back = crpg_data::read_document(&bytes).unwrap();
    assert_eq!(back, doc);
}

/// T2 (Behaviour 1, 2): ability RT fields round trip; `false` is kept.
#[test]
fn t2_ability_rt_fields_round_trip() {
    for repeats in [true, false] {
        let mut value = heavy_ability();
        value["recovery_ticks"] = json!(60);
        value["repeats_in_real_time"] = json!(repeats);
        let doc = read_value(&value).expect("decodes");
        let Document::Ability(ability) = &doc else {
            panic!("expected ability, got {doc:?}")
        };
        assert_eq!(ability.recovery_ticks, Some(60));
        assert_eq!(ability.repeats_in_real_time, Some(repeats));
        let bytes = crpg_data::write_document(&doc).unwrap();
        assert!(contains(&bytes, "\"recovery_ticks\": 60"));
        let want = format!("\"repeats_in_real_time\": {repeats}");
        assert!(contains(&bytes, &want), "repeats={repeats} is kept");
        // Canonical key sort places both between outcome_table and
        // requires_target.
        assert!(offset(&bytes, "\"outcome_table\"") < offset(&bytes, "\"recovery_ticks\""));
        assert!(offset(&bytes, "\"recovery_ticks\"") < offset(&bytes, "\"repeats_in_real_time\""));
        assert!(offset(&bytes, "\"repeats_in_real_time\"") < offset(&bytes, "\"requires_target\""));
        let back = crpg_data::read_document(&bytes).unwrap();
        assert_eq!(back, doc);
    }
}

/// T3 (Behaviour 2, 5): every existing ruleset and ability document reads
/// with all three fields absent, and the canonical writer emits no key.
#[test]
fn t3_existing_documents_have_all_three_fields_absent() {
    const DOCS: [&str; 10] = [
        "rulesets/minimal-d6/ruleset.json",
        "rulesets/minimal-d6/strike.json",
        "rulesets/srd-lite/ruleset.json",
        "rulesets/srd-lite/heavy.json",
        "rulesets/srd-lite/focus.json",
        "campaigns/fixtures/combat_basic/rulesets/minimal-d6.json",
        "campaigns/fixtures/combat_basic/abilities/strike.json",
        "campaigns/fixtures/combat_srd/rulesets/srd-lite.json",
        "campaigns/fixtures/combat_srd/abilities/heavy.json",
        "campaigns/fixtures/combat_srd/abilities/focus.json",
    ];
    for path in DOCS {
        let doc =
            crpg_data::read_document(&read_repo(path)).unwrap_or_else(|e| panic!("{path}: {e:?}"));
        match &doc {
            Document::Ruleset(ruleset) => assert_eq!(ruleset.round_ticks, None, "{path}"),
            Document::Ability(ability) => {
                assert_eq!(ability.recovery_ticks, None, "{path}");
                assert_eq!(ability.repeats_in_real_time, None, "{path}");
            }
            other => panic!("{path}: expected ruleset or ability, got {other:?}"),
        }
        let bytes = crpg_data::write_document(&doc).unwrap();
        for key in ["round_ticks", "recovery_ticks", "repeats_in_real_time"] {
            assert!(!contains(&bytes, key), "{path} writes {key}");
        }
    }
}

/// T4 (Behaviour 3): zero counts fail `read_document` with the exact
/// contract messages.
#[test]
fn t4_zero_counts_are_malformed_with_exact_messages() {
    let mut ruleset = srd_ruleset();
    ruleset["round_ticks"] = json!(0);
    assert_eq!(
        malformed_message(read_value(&ruleset)),
        "ruleset round_ticks must be nonzero"
    );
    let mut ability = heavy_ability();
    ability["recovery_ticks"] = json!(0);
    assert_eq!(
        malformed_message(read_value(&ability)),
        "ability recovery_ticks must be nonzero"
    );
}

/// T5 (Behaviour 1): bounds and type forms for the three keys.
#[test]
fn t5_bounds_and_type_forms() {
    for (want, ok) in [(1u32, json!(1u32)), (u32::MAX, json!(u32::MAX))] {
        let mut ruleset = srd_ruleset();
        ruleset["round_ticks"] = ok.clone();
        let Document::Ruleset(r) = read_value(&ruleset).expect("accepted") else {
            panic!("expected ruleset")
        };
        assert_eq!(r.round_ticks, Some(want));
        let mut ability = heavy_ability();
        ability["recovery_ticks"] = ok;
        let Document::Ability(a) = read_value(&ability).expect("accepted") else {
            panic!("expected ability")
        };
        assert_eq!(a.recovery_ticks, Some(want));
    }
    for bad in [json!(4294967296u64), json!(-1), json!("120")] {
        let mut ruleset = srd_ruleset();
        ruleset["round_ticks"] = bad.clone();
        assert!(
            matches!(read_value(&ruleset), Err(DataError::Malformed { .. })),
            "round_ticks {bad} rejected"
        );
        let mut ability = heavy_ability();
        ability["recovery_ticks"] = bad.clone();
        assert!(
            matches!(read_value(&ability), Err(DataError::Malformed { .. })),
            "recovery_ticks {bad} rejected"
        );
    }
    // Non-integer numeric forms fail at the integer-only parse stage. The
    // `120.0` text lives in a string, never a float literal (crate rule).
    for (mut value, key) in [
        (srd_ruleset(), "round_ticks"),
        (heavy_ability(), "recovery_ticks"),
    ] {
        value[key] = json!(120);
        let raw = String::from_utf8(serde_json::to_vec(&value).unwrap())
            .unwrap()
            .replace(&format!("\"{key}\":120"), &format!("\"{key}\":120.0"));
        assert!(matches!(
            crpg_data::read_document(raw.as_bytes()),
            Err(DataError::Malformed { .. }),
        ));
    }
    // A non-boolean repeats_in_real_time is rejected.
    for bad in [json!(1), json!("true")] {
        let mut ability = heavy_ability();
        ability["repeats_in_real_time"] = bad.clone();
        assert!(
            matches!(read_value(&ability), Err(DataError::Malformed { .. })),
            "repeats_in_real_time {bad} rejected"
        );
    }
}

/// T6 (Behaviour 1, 2): explicit `null` decodes as absent and the key is not
/// written back.
#[test]
fn t6_explicit_null_decodes_as_absent() {
    let mut ruleset = srd_ruleset();
    ruleset["round_ticks"] = Value::Null;
    let doc = read_value(&ruleset).expect("null decodes");
    let Document::Ruleset(r) = &doc else {
        panic!("expected ruleset")
    };
    assert_eq!(r.round_ticks, None);
    assert!(!contains(
        &crpg_data::write_document(&doc).unwrap(),
        "round_ticks"
    ));
    let mut ability = heavy_ability();
    ability["recovery_ticks"] = Value::Null;
    ability["repeats_in_real_time"] = Value::Null;
    let doc = read_value(&ability).expect("null decodes");
    let Document::Ability(a) = &doc else {
        panic!("expected ability")
    };
    assert_eq!(a.recovery_ticks, None);
    assert_eq!(a.repeats_in_real_time, None);
    let bytes = crpg_data::write_document(&doc).unwrap();
    assert!(!contains(&bytes, "recovery_ticks"));
    assert!(!contains(&bytes, "repeats_in_real_time"));
}

/// T7 (Behaviour 1): near-miss keys stay unknown fields.
#[test]
fn t7_near_miss_keys_are_rejected() {
    let mut ruleset = srd_ruleset();
    ruleset["round_tick"] = json!(120);
    assert!(matches!(
        read_value(&ruleset),
        Err(DataError::Malformed { .. })
    ));
    let mut ability = heavy_ability();
    ability["recovery_tick"] = json!(60);
    assert!(matches!(
        read_value(&ability),
        Err(DataError::Malformed { .. })
    ));
}

/// T8 (Behaviour 3): the zero checks are the last check of each arm, so an
/// earlier fault is reported instead.
#[test]
fn t8_zero_checks_come_last() {
    let mut ruleset = srd_ruleset();
    let first = ruleset["abilities"][0].clone();
    ruleset["abilities"].as_array_mut().unwrap().push(first);
    ruleset["round_ticks"] = json!(0);
    assert_eq!(
        malformed_message(read_value(&ruleset)),
        "duplicate ruleset ability"
    );
    let mut ability = heavy_ability();
    ability["attribute"] = json!("");
    ability["recovery_ticks"] = json!(0);
    assert_eq!(
        malformed_message(read_value(&ability)),
        "ability attribute must be nonempty"
    );
}

/// T9 (Behaviour 4): `validate` emits no diagnostic for any mix of RT
/// fields, including RT ability fields under a turn-based-only ruleset.
#[test]
fn t9_validate_emits_no_rt_diagnostics() {
    let files = read_tree(&repo_root().join("campaigns/fixtures/combat_srd"));
    let engine: semver::Version = "0.1.0".parse().unwrap();
    let loaded = crpg_data::load_campaign(&files, &engine).expect("combat_srd loads");
    let mut docs = loaded.documents.clone();
    for doc in docs.values_mut() {
        if let Document::Ability(ability) = doc {
            ability.recovery_ticks = Some(60);
            ability.repeats_in_real_time = Some(true);
        }
    }
    let findings = crpg_data::validate(&LoadedCampaign {
        documents: docs.clone(),
        index: BTreeMap::new(),
    });
    // T136b item 11 supersedes the no-diagnostic expectation: exactly two
    // warnings, one per ability, at /recovery_ticks.
    assert_eq!(findings.len(), 2, "{findings:?}");
    for finding in &findings {
        assert_eq!(finding.severity, crpg_data::Severity::Warning);
        assert_eq!(
            finding.code,
            crpg_data::DiagnosticCode::RealTimeFieldsWithoutRoundTicks
        );
        assert_eq!(finding.pointer, "/recovery_ticks");
    }
    // With round_ticks added to the ruleset, the result is clean.
    for doc in docs.values_mut() {
        if let Document::Ruleset(ruleset) = doc {
            ruleset.round_ticks = Some(120);
        }
    }
    let findings = crpg_data::validate(&LoadedCampaign {
        documents: docs,
        index: BTreeMap::new(),
    });
    assert!(findings.is_empty(), "{findings:?}");
}

/// Behaviour 5: the tags stay at /2 and a /1 source migrates as today, with
/// all three fields absent in the result.
#[test]
fn b5_v1_sources_migrate_to_v2_with_rt_fields_absent() {
    let doc = crpg_data::read_document(&read_repo("rulesets/minimal-d6/ruleset.json")).unwrap();
    let Document::Ruleset(ruleset) = &doc else {
        panic!("expected ruleset")
    };
    assert_eq!(ruleset.round_ticks, None);
    let bytes = crpg_data::write_document(&doc).unwrap();
    assert!(contains(&bytes, "\"schema\": \"crpg.ruleset/2\""));
    assert!(!contains(&bytes, "round_ticks"));
    let doc = crpg_data::read_document(&read_repo("rulesets/minimal-d6/strike.json")).unwrap();
    let Document::Ability(ability) = &doc else {
        panic!("expected ability")
    };
    assert_eq!(ability.recovery_ticks, None);
    assert_eq!(ability.repeats_in_real_time, None);
    let bytes = crpg_data::write_document(&doc).unwrap();
    assert!(contains(&bytes, "\"schema\": \"crpg.ability/2\""));
    assert!(!contains(&bytes, "recovery_ticks"));
    assert!(!contains(&bytes, "repeats_in_real_time"));
}

/// T10 (Behaviour 5, 6): the generated ruleset and ability schemas hold
/// exactly the new properties, none of them required, at the unchanged tags.
#[test]
fn t10_generated_schemas_carry_optional_rt_properties() {
    let schemas = crpg_data::generated_schemas().unwrap();
    let shape = |name: &str| -> Value {
        serde_json::from_slice::<Value>(&schemas[&format!("{name}.schema.json")]).unwrap()["oneOf"]
            [0]
        .clone()
    };
    let keys = |shape: &Value| -> Vec<String> {
        shape["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    };
    let ruleset = shape("ruleset");
    assert_eq!(
        ruleset["properties"]["schema"]["const"],
        json!("crpg.ruleset/2")
    );
    assert_eq!(
        keys(&ruleset),
        [
            "_note",
            "abilities",
            "attributes",
            "creature_persistence",
            "effects_on_exit",
            "health_stat",
            "id",
            "initiative",
            "late_join",
            "name",
            "package",
            "pools",
            "round_ticks",
            "schema",
            "slug",
            "stats",
            "version",
        ]
    );
    assert!(!ruleset["required"]
        .as_array()
        .unwrap()
        .contains(&json!("round_ticks")));
    let ability = shape("ability");
    assert_eq!(
        ability["properties"]["schema"]["const"],
        json!("crpg.ability/2")
    );
    assert_eq!(
        keys(&ability),
        [
            "_note",
            "allow_self_target",
            "attribute",
            "bonus_stats",
            "cost",
            "damage",
            "defense",
            "dice",
            "effect",
            "ends_turn",
            "extra_costs",
            "id",
            "name",
            "natural_die",
            "outcome_table",
            "recovery_ticks",
            "repeats_in_real_time",
            "requires_target",
            "schema",
            "slug",
        ]
    );
    let required = ability["required"].as_array().unwrap();
    assert!(!required.contains(&json!("recovery_ticks")));
    assert!(!required.contains(&json!("repeats_in_real_time")));
}
