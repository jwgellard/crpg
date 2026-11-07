#![forbid(unsafe_code)]
//! T139d two-keys contract tests, written from `tasks/T139.md` Behaviour
//! B-D1–B-D5 alone. Each test names the Behaviour items it proves. B-D6 ("no
//! crate outside `crpg-data` builds a `Ruleset` or `Encounter` by struct
//! literal; every downstream crate compiles unchanged") is the existing-test
//! gate (L1), which the full `cargo test -p crpg-data` run shows.
use crpg_core::Ulid;
use crpg_data::{
    CreaturePersistenceWire, DataError, Diagnostic, Document, EffectsOnExitWire, LateJoinWire,
    LoadedCampaign, SourcePath,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn id(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

fn read(value: &Value) -> Result<Document, DataError> {
    crpg_data::read_document(&serde_json::to_vec(value).unwrap())
}

fn read_ok(value: &Value) -> Document {
    read(value).unwrap_or_else(|err| panic!("document should read: {err}"))
}

fn malformed_message(result: Result<Document, DataError>) -> String {
    match result {
        Err(DataError::Malformed { message, .. }) => message,
        other => panic!("expected Malformed, got {other:?}"),
    }
}

fn written(doc: &Document) -> String {
    String::from_utf8(crpg_data::write_document(doc).unwrap()).unwrap()
}

fn json_value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn ruleset_of(doc: &Document) -> crpg_data::Ruleset {
    match doc {
        Document::Ruleset(r) => r.clone(),
        other => panic!("expected ruleset, got {other:?}"),
    }
}

fn encounter_of(doc: &Document) -> crpg_data::Encounter {
    match doc {
        Document::Encounter(e) => e.clone(),
        other => panic!("expected encounter, got {other:?}"),
    }
}

/// A minimal `crpg.ruleset/2` document with none of the four new fields.
fn ruleset() -> Value {
    json!({
        "schema": "crpg.ruleset/2",
        "id": id(100).to_string(),
        "slug": "r",
        "name": "fixture.r",
        "package": "test.ruleset",
        "version": "0.1.0",
        "stats": [
            {"name": "might", "kind": "int"},
            {"name": "health", "kind": "int"}
        ],
        "health_stat": "health",
        "attributes": ["might"],
        "pools": [{"id": id(105).to_string(), "max": 1, "refresh": {"type": "on_turn_start"}}],
        "abilities": [id(101).to_string()]
    })
}

/// A minimal `crpg.encounter/1` document with no `respawns`.
fn encounter() -> Value {
    json!({
        "schema": "crpg.encounter/1",
        "id": id(110).to_string(),
        "slug": "e",
        "name": "fixture.e",
        "ruleset": id(100).to_string(),
        "participants": [{"placement": id(120).to_string(), "initiative": 10}]
    })
}

// ---------------------------------------------------------------------------
// B-D1 — Decode the four hooks.
// ---------------------------------------------------------------------------

/// Behaviour B-D1/B-D2 (accepted direction): every enum variant and both
/// `respawns` booleans decode, and read → write → read gives an equal
/// document for each new field.
#[test]
fn bd1_each_enum_value_and_respawns_decode() {
    for (wire, want) in [
        (json!("survivors"), CreaturePersistenceWire::Survivors),
        (json!("registered"), CreaturePersistenceWire::Registered),
    ] {
        let mut r = ruleset();
        r["creature_persistence"] = wire;
        let doc = read_ok(&r);
        assert_eq!(ruleset_of(&doc).creature_persistence, Some(want));
        assert_eq!(read(&json_value(&written(&doc))).unwrap(), doc);
    }
    for (wire, want) in [
        (json!("next_round"), LateJoinWire::NextRound),
        (json!("after_active"), LateJoinWire::AfterActive),
    ] {
        let mut r = ruleset();
        r["late_join"] = wire;
        let doc = read_ok(&r);
        assert_eq!(ruleset_of(&doc).late_join, Some(want));
        assert_eq!(read(&json_value(&written(&doc))).unwrap(), doc);
    }
    for (wire, want) in [
        (json!("suspend"), EffectsOnExitWire::Suspend),
        (json!("strip"), EffectsOnExitWire::Strip),
        (json!("strip_at_release"), EffectsOnExitWire::StripAtRelease),
    ] {
        let mut r = ruleset();
        r["effects_on_exit"] = wire;
        let doc = read_ok(&r);
        assert_eq!(ruleset_of(&doc).effects_on_exit, Some(want));
        assert_eq!(read(&json_value(&written(&doc))).unwrap(), doc);
    }
    for (wire, want) in [(json!(true), true), (json!(false), false)] {
        let mut e = encounter();
        e["respawns"] = wire;
        let doc = read_ok(&e);
        assert_eq!(encounter_of(&doc).respawns, Some(want));
        assert_eq!(read(&json_value(&written(&doc))).unwrap(), doc);
    }
}

/// Behaviour B-D1: a present enum key must be a JSON string naming a variant;
/// any other JSON form fails with the exact policy-name message.
#[test]
fn bd1_non_string_forms_fail_with_policy_name_message() {
    for (key, msg) in [
        (
            "creature_persistence",
            "creature_persistence must be a policy name",
        ),
        ("late_join", "late_join must be a policy name"),
        ("effects_on_exit", "effects_on_exit must be a policy name"),
    ] {
        for bad in [json!({"type": "survivors"}), json!(1), json!(true)] {
            let mut r = ruleset();
            r[key] = bad;
            assert_eq!(malformed_message(read(&r)), msg, "{key}");
        }
    }
}

/// Behaviour B-D1: an unknown string fails `unknown variant` and lists every
/// variant, in enum order.
#[test]
fn bd1_unknown_string_lists_every_variant() {
    for (key, bad, variants) in [
        (
            "creature_persistence",
            "survivor",
            &["survivors", "registered"][..],
        ),
        (
            "late_join",
            "after_head",
            &["next_round", "after_active"][..],
        ),
        (
            "effects_on_exit",
            "strip_on_release",
            &["suspend", "strip", "strip_at_release"][..],
        ),
    ] {
        let mut r = ruleset();
        r[key] = json!(bad);
        let msg = malformed_message(read(&r));
        assert!(
            msg.contains(&format!("unknown variant `{bad}`")),
            "{key}: {msg}"
        );
        for variant in variants {
            assert!(
                msg.contains(&format!("`{variant}`")),
                "{key}: `{variant}` missing from {msg}"
            );
        }
    }
}

/// Behaviour B-D1: a present `respawns` must be a JSON boolean.
#[test]
fn bd1_respawns_must_be_a_boolean() {
    for bad in [json!(1), json!("true"), json!({"type": "respawns"})] {
        let mut e = encounter();
        e["respawns"] = bad;
        assert!(matches!(read(&e), Err(DataError::Malformed { .. })));
    }
}

/// Behaviour B-D1: an explicit `null` decodes as absent for all four keys.
#[test]
fn bd1_explicit_null_decodes_as_absent() {
    let mut r = ruleset();
    r["creature_persistence"] = Value::Null;
    assert_eq!(ruleset_of(&read_ok(&r)).creature_persistence, None);

    let mut r = ruleset();
    r["late_join"] = Value::Null;
    assert_eq!(ruleset_of(&read_ok(&r)).late_join, None);

    let mut r = ruleset();
    r["effects_on_exit"] = Value::Null;
    assert_eq!(ruleset_of(&read_ok(&r)).effects_on_exit, None);

    let mut e = encounter();
    e["respawns"] = Value::Null;
    assert_eq!(encounter_of(&read_ok(&e)).respawns, None);
}

/// Behaviour B-D1: the near-miss keys are rejected as unknown fields.
#[test]
fn bd1_near_miss_keys_are_unknown_fields() {
    for (mut doc, key, value) in [
        (ruleset(), "creature_persistance", json!("survivors")),
        (ruleset(), "late_joins", json!("next_round")),
        (ruleset(), "effect_on_exit", json!("suspend")),
        (encounter(), "respawn", json!(true)),
    ] {
        doc[key] = value;
        assert!(
            matches!(read(&doc), Err(DataError::Malformed { .. })),
            "{key}"
        );
    }
}

/// Behaviour B-D1: the remaining "any other JSON form" rejections — an array
/// and a float fail for each enum key (a float at the integer-only number
/// stage), an array and a float fail for `respawns`, and a mis-cased string
/// fails as an unknown variant listing every variant.
#[test]
fn bd1_array_float_and_miscased_forms_rejected() {
    // A JSON float, built from text so this file itself holds no float
    // literal; it is rejected at the integer-only number stage before the enum
    // decoder ever sees it.
    let float: Value = serde_json::from_str("1.5").unwrap();

    for (key, msg) in [
        (
            "creature_persistence",
            "creature_persistence must be a policy name",
        ),
        ("late_join", "late_join must be a policy name"),
        ("effects_on_exit", "effects_on_exit must be a policy name"),
    ] {
        // An array is "any other JSON form": the exact policy-name message.
        let mut r = ruleset();
        r[key] = json!([]);
        assert_eq!(malformed_message(read(&r)), msg, "{key} array");

        // A float is rejected before the enum decoder runs.
        let mut r = ruleset();
        r[key] = float.clone();
        assert!(
            matches!(read(&r), Err(DataError::Malformed { .. })),
            "{key} float"
        );
    }

    // A present `respawns` must be a boolean: an array and a float fail.
    for bad in [json!([]), float.clone()] {
        let mut e = encounter();
        e["respawns"] = bad;
        assert!(
            matches!(read(&e), Err(DataError::Malformed { .. })),
            "respawns"
        );
    }

    // A mis-cased string names no variant: `unknown variant`, listing all.
    for (key, bad, variants) in [
        (
            "creature_persistence",
            "Survivors",
            &["survivors", "registered"][..],
        ),
        (
            "late_join",
            "AfterActive",
            &["next_round", "after_active"][..],
        ),
        (
            "effects_on_exit",
            "StripAtRelease",
            &["suspend", "strip", "strip_at_release"][..],
        ),
    ] {
        let mut r = ruleset();
        r[key] = json!(bad);
        let msg = malformed_message(read(&r));
        assert!(
            msg.contains(&format!("unknown variant `{bad}`")),
            "{key}: {msg}"
        );
        for variant in variants {
            assert!(
                msg.contains(&format!("`{variant}`")),
                "{key}: `{variant}` missing from {msg}"
            );
        }
    }
}

/// Behaviour B-D1: a string must name exactly one variant, so a string that
/// differs from a valid snake_case name only by letter case is an unknown
/// variant (not a case-folded match), listing every variant in enum order.
#[test]
fn bd1_case_variants_of_valid_names_are_unknown_variants() {
    for (key, bads, variants) in [
        (
            "creature_persistence",
            &["SURVIVORS", "Registered", "rEgIsTeReD"][..],
            &["survivors", "registered"][..],
        ),
        (
            "late_join",
            &["After_Active", "AFTER_ACTIVE", "Next_round", "NEXT_ROUND"][..],
            &["next_round", "after_active"][..],
        ),
        (
            "effects_on_exit",
            &["STRIP_AT_RELEASE", "Strip_At_Release", "STRIP", "Suspend"][..],
            &["suspend", "strip", "strip_at_release"][..],
        ),
    ] {
        for bad in bads {
            let mut r = ruleset();
            r[key] = json!(bad);
            let msg = malformed_message(read(&r));
            assert!(
                msg.contains(&format!("unknown variant `{bad}`")),
                "{key}: {bad}: {msg}"
            );
            let positions: Vec<usize> = variants
                .iter()
                .map(|variant| {
                    msg.find(&format!("`{variant}`"))
                        .unwrap_or_else(|| panic!("{key}: `{variant}` missing from {msg}"))
                })
                .collect();
            assert!(positions.windows(2).all(|w| w[0] < w[1]), "{key}: {msg}");
        }
    }
}

// ---------------------------------------------------------------------------
// B-D2 — Encode the four hooks.
// ---------------------------------------------------------------------------

/// Behaviour B-D2: an absent field writes no key; a present field writes its
/// key where the canonical key sort places it, explicit defaults included.
#[test]
fn bd2_absent_omits_key_and_present_writes_explicit_defaults() {
    let out = written(&read_ok(&ruleset()));
    assert!(!out.contains("creature_persistence"));
    assert!(!out.contains("late_join"));
    assert!(!out.contains("effects_on_exit"));
    assert!(!written(&read_ok(&encounter())).contains("respawns"));

    // Explicit defaults are still written.
    let mut r = ruleset();
    r["creature_persistence"] = json!("survivors");
    r["late_join"] = json!("next_round");
    r["effects_on_exit"] = json!("suspend");
    let out = written(&read_ok(&r));
    assert!(
        out.contains("\"creature_persistence\": \"survivors\""),
        "{out}"
    );
    assert!(out.contains("\"late_join\": \"next_round\""), "{out}");
    assert!(out.contains("\"effects_on_exit\": \"suspend\""), "{out}");
    // Canonical key sort places them alphabetically among the ruleset keys.
    let cp = out.find("\"creature_persistence\"").unwrap();
    let eoe = out.find("\"effects_on_exit\"").unwrap();
    let lj = out.find("\"late_join\"").unwrap();
    assert!(cp < eoe && eoe < lj, "{out}");

    for (value, want) in [(json!(false), "false"), (json!(true), "true")] {
        let mut e = encounter();
        e["respawns"] = value;
        let out = written(&read_ok(&e));
        assert!(out.contains(&format!("\"respawns\": {want}")), "{out}");
        // `respawns` sorts between `participants` and `ruleset`.
        let participants = out.find("\"participants\"").unwrap();
        let respawns = out.find("\"respawns\"").unwrap();
        let ruleset_key = out.find("\"ruleset\"").unwrap();
        assert!(participants < respawns && respawns < ruleset_key, "{out}");
    }
}

// ---------------------------------------------------------------------------
// B-D3 — No read-time or semantic rule.
// ---------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn combat_srd_files() -> BTreeMap<SourcePath, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<SourcePath, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
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

fn load_combat_srd() -> BTreeMap<SourcePath, Document> {
    let engine: semver::Version = "0.1.0".parse().unwrap();
    crpg_data::load_campaign(&combat_srd_files(), &engine)
        .expect("combat_srd loads")
        .documents
}

fn validate_docs(docs: &BTreeMap<SourcePath, Document>) -> Vec<Diagnostic> {
    crpg_data::validate(&LoadedCampaign {
        documents: docs.clone(),
        index: BTreeMap::new(),
    })
}

/// Behaviour B-D3: no read-time check, semantic rule or diagnostic is added
/// for any value or mix of the four fields. Setting all four to their
/// non-default values keeps a clean campaign clean.
#[test]
fn bd3_no_rule_for_any_value_or_mix() {
    let mut docs = load_combat_srd();
    for doc in docs.values_mut() {
        match doc {
            Document::Ruleset(r) => {
                r.creature_persistence = Some(CreaturePersistenceWire::Registered);
                r.late_join = Some(LateJoinWire::AfterActive);
                r.effects_on_exit = Some(EffectsOnExitWire::StripAtRelease);
            }
            Document::Encounter(e) => {
                e.respawns = Some(true);
            }
            _ => {}
        }
    }
    assert_eq!(validate_docs(&docs), Vec::new());
}

/// Behaviour B-D3 helper: re-read the `combat_srd` ruleset and encounter
/// through `read_document` (JSON text in, as the contract's reader does) with
/// the given edits, then validate the whole campaign.
fn bd3_validate_mix(ruleset_fields: &[(&str, Value)], respawns: Option<bool>) -> Vec<Diagnostic> {
    let mut docs = load_combat_srd();
    for key in docs.keys().cloned().collect::<Vec<_>>() {
        let doc = docs.get(&key).unwrap();
        let edited = match doc {
            Document::Ruleset(r) => {
                let mut value = json_value(&written(&Document::Ruleset(r.clone())));
                for (field, val) in ruleset_fields {
                    value[field] = val.clone();
                }
                Some(read_ok(&value))
            }
            Document::Encounter(e) => {
                let mut value = json_value(&written(&Document::Encounter(e.clone())));
                if let Some(respawn) = respawns {
                    value["respawns"] = json!(respawn);
                }
                Some(read_ok(&value))
            }
            _ => None,
        };
        if let Some(edited) = edited {
            docs.insert(key, edited);
        }
    }
    validate_docs(&docs)
}

/// Behaviour B-D3: each single field alone reads clean through `read_document`,
/// with the other three fields absent.
#[test]
fn bd3_each_single_field_alone_reads_clean() {
    let mut r = ruleset();
    r["creature_persistence"] = json!("registered");
    let doc = read_ok(&r);
    assert_eq!(
        ruleset_of(&doc).creature_persistence,
        Some(CreaturePersistenceWire::Registered)
    );
    assert_eq!(ruleset_of(&doc).late_join, None);
    assert_eq!(ruleset_of(&doc).effects_on_exit, None);

    let mut r = ruleset();
    r["late_join"] = json!("after_active");
    let doc = read_ok(&r);
    assert_eq!(ruleset_of(&doc).late_join, Some(LateJoinWire::AfterActive));
    assert_eq!(ruleset_of(&doc).creature_persistence, None);
    assert_eq!(ruleset_of(&doc).effects_on_exit, None);

    let mut r = ruleset();
    r["effects_on_exit"] = json!("strip");
    let doc = read_ok(&r);
    assert_eq!(
        ruleset_of(&doc).effects_on_exit,
        Some(EffectsOnExitWire::Strip)
    );
    assert_eq!(ruleset_of(&doc).creature_persistence, None);
    assert_eq!(ruleset_of(&doc).late_join, None);

    let mut e = encounter();
    e["respawns"] = json!(true);
    let doc = read_ok(&e);
    assert_eq!(encounter_of(&doc).respawns, Some(true));
}

/// Behaviour B-D3: contradictory-looking mixes of the four fields read without
/// `Malformed` and validate with zero diagnostics — no read-time check or
/// semantic rule is added for any mix.
#[test]
fn bd3_contradictory_mixes_read_and_validate_clean() {
    // `effects_on_exit: strip` with `registered` must not be rejected as
    // `Malformed` (the survived mutant).
    let mut r = ruleset();
    r["effects_on_exit"] = json!("strip");
    r["creature_persistence"] = json!("registered");
    let doc = read_ok(&r);
    assert_eq!(
        ruleset_of(&doc).effects_on_exit,
        Some(EffectsOnExitWire::Strip)
    );
    assert_eq!(
        ruleset_of(&doc).creature_persistence,
        Some(CreaturePersistenceWire::Registered)
    );

    // `creature_persistence: survivors` with `respawns: true`.
    let findings = bd3_validate_mix(&[("creature_persistence", json!("survivors"))], Some(true));
    assert_eq!(findings, Vec::new());

    // `registered` with `respawns: false`.
    let findings = bd3_validate_mix(
        &[("creature_persistence", json!("registered"))],
        Some(false),
    );
    assert_eq!(findings, Vec::new());

    // `effects_on_exit: strip` with `registered` in a full campaign.
    let findings = bd3_validate_mix(
        &[
            ("effects_on_exit", json!("strip")),
            ("creature_persistence", json!("registered")),
        ],
        None,
    );
    assert_eq!(findings, Vec::new());
}

// ---------------------------------------------------------------------------
// B-D4 — Versions and migration.
// ---------------------------------------------------------------------------

/// Behaviour B-D4: the tags stay `crpg.ruleset/2` and `crpg.encounter/1`, and
/// a `/1` ruleset migrates as today with all three ruleset fields absent.
#[test]
fn bd4_versions_and_migration_produce_absent_fields() {
    let versions = crpg_data::schema_versions();
    let current = |family: &str| {
        versions
            .iter()
            .find(|entry| entry.schema_type == family)
            .map(|entry| entry.current)
            .unwrap_or_else(|| panic!("missing family {family}"))
    };
    assert_eq!(current("crpg.ruleset"), 2);
    assert_eq!(current("crpg.encounter"), 1);

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/migration_v1/campaign/rulesets/migrated.json");
    let bytes = std::fs::read(root).expect("required migration fixture");
    let doc = crpg_data::read_document(&bytes).expect("/1 ruleset migrates");
    let rs = ruleset_of(&doc);
    assert_eq!(rs.creature_persistence, None);
    assert_eq!(rs.late_join, None);
    assert_eq!(rs.effects_on_exit, None);
    let out = written(&doc);
    assert!(!out.contains("creature_persistence"));
    assert!(!out.contains("late_join"));
    assert!(!out.contains("effects_on_exit"));
}

// ---------------------------------------------------------------------------
// B-D5 — Schemas.
// ---------------------------------------------------------------------------

/// Behaviour B-D5: after generation, `ruleset.schema.json` and
/// `encounter.schema.json` gain exactly the four properties, none of which is
/// in `required`.
#[test]
fn bd5_schemas_gain_exactly_the_four_properties() {
    let schemas = crpg_data::generated_schemas().unwrap();
    assert_eq!(schemas.len(), 22);

    let ruleset: Value = serde_json::from_slice(&schemas["ruleset.schema.json"]).unwrap();
    let shape = &ruleset["oneOf"][0];
    let props = shape["properties"].as_object().unwrap();

    // The three ruleset properties, rendered as anyOf($ref, null) with the
    // field doc comment as description.
    for (key, def, description) in [
        (
            "creature_persistence",
            "CreaturePersistenceWire",
            "Who keeps creature state after an encounter; absent means survivors.",
        ),
        (
            "late_join",
            "LateJoinWire",
            "Where a late first-round joiner acts; absent means the next round.",
        ),
        (
            "effects_on_exit",
            "EffectsOnExitWire",
            "What attached effects become at exit; absent means suspend.",
        ),
    ] {
        let prop = &props[key];
        assert_eq!(prop["description"], json!(description), "{key}");
        let any_of = prop["anyOf"].as_array().expect("anyOf present");
        assert_eq!(any_of.len(), 2, "{key}");
        assert_eq!(any_of[0]["$ref"], json!(format!("#/$defs/{def}")), "{key}");
        assert_eq!(any_of[1], json!({"type": "null"}), "{key}");
    }

    // The $defs enum entries accept exactly their snake_case names, each
    // carrying the enum's and each variant's doc comment as description.
    for (def, variants, description) in [
        (
            "CreaturePersistenceWire",
            &[
                (
                    "survivors",
                    "Every placed creature that leaves or outlives an encounter keeps its state, and the dead stay dead.",
                ),
                (
                    "registered",
                    "Only creatures that entered from a kept state keep it; authored participants spawn fresh at every start.",
                ),
            ][..],
            "Which placed creatures keep their state after an encounter.",
        ),
        (
            "LateJoinWire",
            &[
                ("next_round", "It waits for the next round."),
                ("after_active", "It acts right after the current turn."),
            ][..],
            "Where a turn-based first-round joiner acts when its initiative slot has passed.",
        ),
        (
            "EffectsOnExitWire",
            &[
                (
                    "suspend",
                    "They keep their remaining rounds and resume at the next entry.",
                ),
                ("strip", "They are removed at leave and at release."),
                ("strip_at_release", "They are removed at release and kept at leave."),
            ][..],
            "What a creature's attached effects become when it leaves an encounter or the encounter ends.",
        ),
    ] {
        let entry = &ruleset["$defs"][def];
        assert_eq!(entry["description"], json!(description), "{def}");
        let one_of = entry["oneOf"].as_array().expect("oneOf present");
        assert_eq!(one_of.len(), variants.len(), "{def}");
        for (variant, (name, variant_description)) in one_of.iter().zip(variants) {
            assert_eq!(variant["type"], json!("string"), "{def}");
            assert_eq!(variant["const"], json!(name), "{def}");
            assert_eq!(
                variant["description"],
                json!(variant_description),
                "{def}::{name}"
            );
        }
    }

    // None of the three is in `required`.
    let required = shape["required"].as_array().unwrap();
    for key in ["creature_persistence", "late_join", "effects_on_exit"] {
        assert!(!required.contains(&json!(key)), "{key}");
    }

    let encounter: Value = serde_json::from_slice(&schemas["encounter.schema.json"]).unwrap();
    let shape = &encounter["oneOf"][0];
    let respawns = &shape["properties"]["respawns"];
    assert_eq!(
        respawns["type"],
        json!(["boolean", "null"]),
        "respawns rendered inline"
    );
    assert_eq!(
        respawns["description"],
        json!(
            "Whether every start spawns the participants fresh and keeps none; absent means false."
        )
    );
    // No inline $ref, so no `$defs` entry is required for the boolean.
    assert!(respawns.get("$ref").is_none());
    let required = shape["required"].as_array().unwrap();
    assert!(!required.contains(&json!("respawns")));
    // `encounter.schema.json` gained no new `$defs` entry: `respawns` is a
    // plain inline boolean, and the three enum definitions are ruleset-only.
    let defs: Vec<&str> = encounter["$defs"]
        .as_object()
        .expect("encounter schema has $defs")
        .keys()
        .map(String::as_str)
        .collect();
    let mut defs_sorted = defs.clone();
    defs_sorted.sort_unstable();
    assert_eq!(defs_sorted, vec!["EncounterParticipant", "UlidText"]);
}
