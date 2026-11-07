#![forbid(unsafe_code)]
//! T142a `crpg.ui/1` model, read-time checks, layout, defaults, schema and registry.
//!
//! Campaign fixtures are built in memory from the read-only bytes of the
//! checked-in `combat_basic` and `combat_srd` roots.
use crpg_core::Ulid;
use crpg_data::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const RULESET: &str = "0000000000000000000000000J";
const POOL: &str = "0000000000000000000000000P";
const ABILITY: &str = "0000000000000000000000000K";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
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
                out.insert(relative.parse().unwrap(), std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn fixture(name: &str) -> BTreeMap<SourcePath, Vec<u8>> {
    read_tree(&repo_root().join("campaigns").join("fixtures").join(name))
}

fn engine() -> semver::Version {
    "0.1.0".parse().unwrap()
}

fn sp(text: &str) -> SourcePath {
    text.parse().unwrap()
}

fn ulid(text: &str) -> Ulid {
    text.parse().unwrap()
}

fn bytes(value: &Value) -> Vec<u8> {
    canonical_json(value).unwrap()
}

fn read_value(value: &Value) -> Result<Document, DataError> {
    read_document(&bytes(value))
}

fn read_ui(value: &Value) -> UiDocument {
    match read_value(value).unwrap() {
        Document::Ui(ui) => ui,
        other => panic!("not a ui document: {other:?}"),
    }
}

fn malformed_message(error: DataError) -> String {
    match error {
        DataError::Malformed {
            path: None,
            message,
        } => message,
        other => panic!("expected Malformed without a path, got {other:?}"),
    }
}

fn read_err(value: &Value) -> String {
    malformed_message(read_value(value).unwrap_err())
}

fn minimal() -> Value {
    json!({"schema": "crpg.ui/1", "ruleset": RULESET})
}

/// The contract's wire example; touches every known kind.
fn wire_example() -> Value {
    json!({
        "schema": "crpg.ui/1", "ruleset": RULESET,
        "stats": [{"stat": "health", "label": "ui.health", "group": "ui.vitals",
            "format": {"type": "fraction", "value": {"type": "pool", "value": POOL}}}],
        "pools": [{"pool": POOL, "label": "ui.actions", "style": "pips"}],
        "abilities": [{"ability": ABILITY, "label": "ui.strike",
            "category_tags": ["melee"], "target_hint": "enemy", "show_odds": true}],
        "outcomes": [{"outcome": "success", "label": "ui.hit"}],
        "screens": [
            {"kind": "character_sheet", "sections": [
                {"type": "stat_group", "value": {"type": "group", "value": "ui.vitals"}}]},
            {"kind": "action_bar", "sections": [
                {"type": "ability_list",
                 "value": {"select": {"type": "tag", "value": "melee"}, "sort": "label"}}]},
            {"kind": "combat_hud", "sections": [
                {"type": "pool_group", "value": {"type": "all"}},
                {"type": "effect_list"}, {"type": "timeline"}, {"type": "log"}]},
            {"kind": "inventory", "sections": [
                {"type": "item_list", "value": {"type": "all"}}]}]
    })
}

/// A document with a value for every text-bearing field.
fn kitchen_sink() -> Value {
    json!({
        "schema": "crpg.ui/1", "ruleset": RULESET, "_note": "n",
        "stats": [
            {"stat": "s", "label": "l", "short_label": "sl", "description": "d", "group": "g",
             "format": {"type": "fraction", "value": {"type": "stat", "value": "m"}}},
            {"stat": "e", "label": "l",
             "format": {"type": "enum", "value": [{"variant": "v", "label": "vl"}]}}],
        "pools": [{"pool": POOL, "label": "l", "icon": "i", "style": "pips"}],
        "abilities": [{"ability": ABILITY, "label": "l", "description": "d", "icon": "i",
            "category_tags": ["t"], "target_hint": "enemy"}],
        "effects": [{"effect": ABILITY, "label": "l", "icon": "i"}],
        "outcomes": [{"outcome": "success", "label": "l"}],
        "screens": [
            {"kind": "character_sheet", "sections": [
                {"type": "stat_group", "value": {"type": "group", "value": "g"}},
                {"type": "stat_group", "value": {"type": "names", "value": ["n"]}},
                {"type": "pool_group", "value": {"type": "ids", "value": [POOL]}}]},
            {"kind": "action_bar", "sections": [
                {"type": "ability_list",
                 "value": {"select": {"type": "tag", "value": "t"}, "sort": "label"}},
                {"type": "ability_list",
                 "value": {"select": {"type": "ids", "value": [ABILITY]}}}]},
            {"kind": "inventory", "sections": [
                {"type": "item_list", "value": {"type": "tag", "value": "t"}}]}]
    })
}

fn set(root: &mut Value, pointer: &str, value: Value) {
    *root.pointer_mut(pointer).unwrap() = value;
}

fn combat_basic_with(name: &str, ui: &Value) -> BTreeMap<SourcePath, Vec<u8>> {
    let mut files = fixture("combat_basic");
    files.insert(sp(name), bytes(ui));
    files
}

// ---------------------------------------------------------------------------

#[test]
fn round_trip_full_and_minimal() {
    // U-1 (B1)
    let wire = wire_example();
    let input = bytes(&wire);
    let document = read_document(&input).unwrap();
    assert_eq!(write_document(&document).unwrap(), input);
    assert_eq!(
        read_document(&write_document(&document).unwrap()).unwrap(),
        document
    );
    let expected = UiDocument {
        ruleset: ulid(RULESET),
        stats: Some(vec![UiStat {
            stat: "health".into(),
            label: "ui.health".into(),
            short_label: None,
            description: None,
            group: Some("ui.vitals".into()),
            format: Some(UiFormat::Fraction(UiMeter::Pool(ulid(POOL)))),
        }]),
        pools: Some(vec![UiPool {
            pool: ulid(POOL),
            label: "ui.actions".into(),
            icon: None,
            style: Some(UiPoolStyle::Pips),
            show_on_hud: None,
        }]),
        abilities: Some(vec![UiAbility {
            ability: ulid(ABILITY),
            label: "ui.strike".into(),
            description: None,
            icon: None,
            category_tags: Some(vec!["melee".into()]),
            target_hint: Some(UiTargetHint::Enemy),
            show_odds: Some(true),
        }]),
        effects: None,
        outcomes: Some(vec![UiOutcome {
            outcome: "success".into(),
            label: "ui.hit".into(),
        }]),
        screens: Some(vec![
            UiScreen {
                kind: UiScreenKind::CharacterSheet,
                sections: vec![UiSection::StatGroup(UiStatSelect::Group(
                    "ui.vitals".into(),
                ))],
            },
            UiScreen {
                kind: UiScreenKind::ActionBar,
                sections: vec![UiSection::AbilityList(UiAbilityList {
                    select: UiAbilitySelect::Tag("melee".into()),
                    sort: Some(UiSort::Label),
                })],
            },
            UiScreen {
                kind: UiScreenKind::CombatHud,
                sections: vec![
                    UiSection::PoolGroup(UiPoolSelect::All),
                    UiSection::EffectList,
                    UiSection::Timeline,
                    UiSection::Log,
                ],
            },
            UiScreen {
                kind: UiScreenKind::Inventory,
                sections: vec![UiSection::ItemList(UiItemSelect::All)],
            },
        ]),
        note: None,
    };
    assert_eq!(document, Document::Ui(expected));

    // The smallest valid document keeps every absent part absent.
    let min = read_ui(&minimal());
    assert_eq!(
        min,
        UiDocument {
            ruleset: ulid(RULESET),
            stats: None,
            pools: None,
            abilities: None,
            effects: None,
            outcomes: None,
            screens: None,
            note: None,
        }
    );
    let min_bytes = bytes(&minimal());
    assert_eq!(write_document(&Document::Ui(min)).unwrap(), min_bytes);

    // Control: `[]` stays `[]` and is not the same as absent.
    let mut empty = minimal();
    for key in [
        "stats",
        "pools",
        "abilities",
        "effects",
        "outcomes",
        "screens",
    ] {
        empty[key] = json!([]);
    }
    empty["_note"] = json!("kept");
    let empty_bytes = bytes(&empty);
    let ui = read_ui(&empty);
    assert_eq!(ui.stats, Some(Vec::new()));
    assert_eq!(ui.screens, Some(Vec::new()));
    assert_eq!(ui.note.as_deref(), Some("kept"));
    assert_eq!(write_document(&Document::Ui(ui)).unwrap(), empty_bytes);
    assert_ne!(empty_bytes, min_bytes);
}

#[test]
fn structural_rejections() {
    // U-2 (B2). Control: the unchanged document reads.
    read_ui(&wire_example());

    let mut extra = wire_example();
    extra["bogus"] = json!(1);
    let message = read_err(&extra);
    assert!(
        message.starts_with("unknown field `bogus`, expected one of `ruleset`"),
        "{message}"
    );

    for forbidden in ["id", "slug", "name"] {
        let mut value = minimal();
        value[forbidden] = json!("x");
        let message = read_err(&value);
        assert!(
            message.starts_with(&format!("unknown field `{forbidden}`, expected one of")),
            "{message}"
        );
    }

    assert_eq!(
        read_err(&json!({"schema": "crpg.ui/1"})),
        "missing field `ruleset`"
    );

    let mut no_label = wire_example();
    no_label["stats"][0]
        .as_object_mut()
        .unwrap()
        .remove("label");
    assert_eq!(read_err(&no_label), "missing field `label`");

    let mut no_stat = wire_example();
    no_stat["stats"][0].as_object_mut().unwrap().remove("stat");
    assert_eq!(read_err(&no_stat), "missing field `stat`");
    let mut no_pool = wire_example();
    no_pool["pools"][0].as_object_mut().unwrap().remove("pool");
    assert_eq!(read_err(&no_pool), "missing field `pool`");
    let mut no_ability = wire_example();
    no_ability["abilities"][0]
        .as_object_mut()
        .unwrap()
        .remove("ability");
    assert_eq!(read_err(&no_ability), "missing field `ability`");
    let mut no_outcome = wire_example();
    no_outcome["outcomes"][0]
        .as_object_mut()
        .unwrap()
        .remove("outcome");
    assert_eq!(read_err(&no_outcome), "missing field `outcome`");
    let mut no_effect = kitchen_sink();
    no_effect["effects"][0]
        .as_object_mut()
        .unwrap()
        .remove("effect");
    assert_eq!(read_err(&no_effect), "missing field `effect`");
    let mut no_kind = wire_example();
    no_kind["screens"][0]
        .as_object_mut()
        .unwrap()
        .remove("kind");
    assert_eq!(read_err(&no_kind), "missing field `kind`");
    let mut no_sections = wire_example();
    no_sections["screens"][0]
        .as_object_mut()
        .unwrap()
        .remove("sections");
    assert_eq!(read_err(&no_sections), "missing field `sections`");
    let mut no_select = wire_example();
    no_select["screens"][1]["sections"][0]["value"]
        .as_object_mut()
        .unwrap()
        .remove("select");
    assert_eq!(read_err(&no_select), "missing field `select`");
    let mut no_variant = kitchen_sink();
    no_variant["stats"][1]["format"]["value"][0]
        .as_object_mut()
        .unwrap()
        .remove("variant");
    assert_eq!(read_err(&no_variant), "missing field `variant`");

    let mut wrong = wire_example();
    wrong["abilities"][0]["show_odds"] = json!("yes");
    assert_eq!(
        read_err(&wrong),
        "invalid type: string \"yes\", expected a boolean"
    );

    // A wrong type on a tolerant set is a serde type error, not `Unknown`.
    let mut kind_number = wire_example();
    kind_number["screens"][0]["kind"] = json!(5);
    assert_eq!(
        read_err(&kind_number),
        "invalid type: integer `5`, expected a string"
    );

    let mut future = wire_example();
    future["schema"] = json!("crpg.ui/2");
    assert!(
        matches!(
            read_value(&future).unwrap_err(),
            DataError::UnsupportedSchema { path: None, found } if found == "crpg.ui/2"
        ),
        "crpg.ui/2 is an unsupported schema"
    );

    let fractional = br#"{"schema":"crpg.ui/1","ruleset":"0000000000000000000000000J","stats":[{"stat":"a","label":"b","format":{"type":"x","value":1.5}}]}"#;
    let message = malformed_message(read_document(fractional).unwrap_err());
    assert!(message.contains("floating point `1.5`"), "{message}");
}

fn at_limit(text: &str, what: &str) -> Value {
    let mut value = kitchen_sink();
    set(&mut value, what, json!(text));
    value
}

const TEXT_POINTERS: &[&str] = &[
    "/stats/0/stat",
    "/stats/0/label",
    "/stats/0/short_label",
    "/stats/0/description",
    "/stats/0/group",
    "/stats/0/format/value/value",
    "/stats/1/format/value/0/variant",
    "/stats/1/format/value/0/label",
    "/pools/0/label",
    "/pools/0/icon",
    "/abilities/0/label",
    "/abilities/0/description",
    "/abilities/0/icon",
    "/abilities/0/category_tags/0",
    "/effects/0/label",
    "/effects/0/icon",
    "/outcomes/0/outcome",
    "/outcomes/0/label",
    "/screens/0/sections/0/value/value",
    "/screens/0/sections/1/value/value/0",
    "/screens/1/sections/0/value/select/value",
    "/screens/2/sections/0/value/value",
];

fn list_of(count: usize, entry: Value) -> Value {
    Value::Array(vec![entry; count])
}

#[test]
fn read_time_bounds() {
    // U-3 (B5). Control: the kitchen sink and an empty `sections` read.
    read_ui(&kitchen_sink());
    let mut control = kitchen_sink();
    set(&mut control, "/screens/0/sections", json!([]));
    read_ui(&control);

    let two_byte = "é".repeat(64); // 64 chars, 128 bytes
    let over = format!("{two_byte}a"); // 65 chars, 129 bytes
    let three_byte = "€".repeat(43); // 43 chars, 129 bytes: a char count would accept it
    assert_eq!(
        (two_byte.len(), over.len(), three_byte.len()),
        (128, 129, 129)
    );
    assert_eq!(three_byte.chars().count(), 43);
    for pointer in TEXT_POINTERS {
        read_value(&at_limit(&two_byte, pointer)).unwrap();
        read_value(&at_limit("x", pointer)).unwrap();
        for bad in [&over, &three_byte, &String::new()] {
            assert_eq!(
                read_err(&at_limit(bad, pointer)),
                format!("ui text at {pointer} must be 1..=128 UTF-8 bytes"),
                "{pointer}"
            );
        }
    }

    // Unknown kind texts are bounded at their own pointers.
    let unknowns: &[(&str, &str)] = &[
        ("/screens/0/kind", "/screens/0/kind"),
        ("/pools/0/style", "/pools/0/style"),
        ("/abilities/0/target_hint", "/abilities/0/target_hint"),
        (
            "/screens/1/sections/0/value/sort",
            "/screens/1/sections/0/value/sort",
        ),
        ("/screens/0/sections/0/type", "/screens/0/sections/0/type"),
        ("/stats/0/format/type", "/stats/0/format/type"),
    ];
    for (set_at, reported) in unknowns {
        read_value(&at_limit(&two_byte, set_at)).unwrap();
        for bad in [&over, &String::new()] {
            assert_eq!(
                read_err(&at_limit(bad, set_at)),
                format!("ui text at {reported} must be 1..=128 UTF-8 bytes")
            );
        }
    }

    // `_note` and `Unknown.value` are exempt.
    let mut exempt = kitchen_sink();
    exempt["_note"] = json!(over);
    set(
        &mut exempt,
        "/screens/0/sections/0",
        json!({"type": "carousel", "value": over}),
    );
    read_value(&exempt).unwrap();

    // List counts: at the limit passes, one over gives the exact text.
    let entry = |key: &str| match key {
        "stats" => json!({"stat": "s", "label": "l"}),
        "pools" => json!({"pool": POOL, "label": "l"}),
        "abilities" => json!({"ability": ABILITY, "label": "l"}),
        "effects" => json!({"effect": ABILITY, "label": "l"}),
        _ => json!({"outcome": "success", "label": "l"}),
    };
    for key in ["stats", "pools", "abilities", "effects", "outcomes"] {
        let mut value = minimal();
        value[key] = list_of(4096, entry(key));
        read_value(&value).unwrap();
        value[key] = list_of(4097, entry(key));
        assert_eq!(
            read_err(&value),
            format!("ui list at /{key} exceeds 4096 entries")
        );
    }
    let screen = json!({"kind": "log_screen", "sections": []});
    let mut screens = minimal();
    screens["screens"] = list_of(64, screen.clone());
    read_value(&screens).unwrap();
    screens["screens"] = list_of(65, screen);
    assert_eq!(read_err(&screens), "ui list at /screens exceeds 64 entries");

    let mut sections = minimal();
    let timeline = json!({"type": "timeline"});
    sections["screens"] =
        json!([{"kind": "combat_hud", "sections": list_of(64, timeline.clone())}]);
    read_value(&sections).unwrap();
    sections["screens"] = json!([{"kind": "combat_hud", "sections": list_of(65, timeline)}]);
    assert_eq!(
        read_err(&sections),
        "ui list at /screens/0/sections exceeds 64 entries"
    );

    // Selector lists, category tags and enum labels.
    let cases: &[(&str, Value, Value)] = &[
        ("/screens/0/sections/1/value/value", json!("n"), json!([])),
        ("/screens/0/sections/2/value/value", json!(POOL), json!([])),
        (
            "/screens/1/sections/1/value/select/value",
            json!(ABILITY),
            json!([]),
        ),
        ("/abilities/0/category_tags", json!("t"), json!([])),
        (
            "/stats/1/format/value",
            json!({"variant": "v", "label": "l"}),
            json!([]),
        ),
    ];
    for (pointer, filler, empty) in cases {
        let mut value = kitchen_sink();
        set(&mut value, pointer, list_of(4096, filler.clone()));
        read_value(&value).unwrap();
        set(&mut value, pointer, list_of(4097, filler.clone()));
        assert_eq!(
            read_err(&value),
            format!("ui list at {pointer} exceeds 4096 entries")
        );
        set(&mut value, pointer, empty.clone());
        assert_eq!(
            read_err(&value),
            format!("ui list at {pointer} must not be empty")
        );
    }

    // Top-level lists and `sections` may be empty.
    let mut empties = kitchen_sink();
    for key in [
        "stats",
        "pools",
        "abilities",
        "effects",
        "outcomes",
        "screens",
    ] {
        empties[key] = json!([]);
    }
    read_value(&empties).unwrap();

    // Order: pre-order walk; the count check precedes the entries; first failure wins.
    let mut ordered = minimal();
    ordered["stats"] = json!([{"stat": "", "label": "l"}]);
    ordered["pools"] = list_of(4097, entry("pools"));
    assert_eq!(
        read_err(&ordered),
        "ui text at /stats/0/stat must be 1..=128 UTF-8 bytes"
    );
    let mut counted = minimal();
    counted["stats"] = Value::Array(
        std::iter::once(json!({"stat": "", "label": "l"}))
            .chain(std::iter::repeat_n(entry("stats"), 4096))
            .collect(),
    );
    assert_eq!(read_err(&counted), "ui list at /stats exceeds 4096 entries");
    let mut within = kitchen_sink();
    set(&mut within, "/stats/0/label", json!(""));
    set(&mut within, "/stats/0/stat", json!(""));
    assert_eq!(
        read_err(&within),
        "ui text at /stats/0/stat must be 1..=128 UTF-8 bytes"
    );
    let mut earlier = kitchen_sink();
    set(&mut earlier, "/pools/0/label", json!(""));
    set(&mut earlier, "/screens/2/sections/0/value/value", json!(""));
    assert_eq!(
        read_err(&earlier),
        "ui text at /pools/0/label must be 1..=128 UTF-8 bytes"
    );
}

fn unknown_doc() -> Value {
    let mut value = kitchen_sink();
    set(&mut value, "/screens/0/kind", json!("journal"));
    set(&mut value, "/pools/0/style", json!("dial"));
    set(&mut value, "/abilities/0/target_hint", json!("self"));
    set(
        &mut value,
        "/screens/1/sections/0/value/sort",
        json!("newest"),
    );
    set(
        &mut value,
        "/stats/0/format",
        json!({"type": "money", "value": {"a": [1, 2]}}),
    );
    set(
        &mut value,
        "/screens/0/sections",
        json!([{"type": "carousel", "value": {"speed": 3}}, {"type": "marquee"}]),
    );
    value
}

#[test]
fn unknown_kinds_load_and_round_trip() {
    // U-4 (B3, B8): every tolerant set keeps an unknown spelling.
    let value = unknown_doc();
    let input = bytes(&value);
    let document = read_document(&input).unwrap();
    assert_eq!(write_document(&document).unwrap(), input);
    let Document::Ui(ui) = &document else {
        panic!()
    };
    let screens = ui.screens.as_ref().unwrap();
    assert_eq!(screens[0].kind, UiScreenKind::Unknown("journal".into()));
    assert_eq!(
        ui.pools.as_ref().unwrap()[0].style,
        Some(UiPoolStyle::Unknown("dial".into()))
    );
    assert_eq!(
        ui.abilities.as_ref().unwrap()[0].target_hint,
        Some(UiTargetHint::Unknown("self".into()))
    );
    let UiSection::AbilityList(list) = &screens[1].sections[0] else {
        panic!()
    };
    assert_eq!(list.sort, Some(UiSort::Unknown("newest".into())));
    assert_eq!(
        ui.stats.as_ref().unwrap()[0].format,
        Some(UiFormat::Unknown {
            kind: "money".into(),
            value: Some(json!({"a": [1, 2]}))
        })
    );
    assert_eq!(
        screens[0].sections[0],
        UiSection::Unknown {
            kind: "carousel".into(),
            value: Some(json!({"speed": 3}))
        }
    );
    // A section without a value stays without one; `as_str` gives the text.
    assert_eq!(
        screens[0].sections[1],
        UiSection::Unknown {
            kind: "marquee".into(),
            value: None
        }
    );
    assert_eq!(screens[0].sections[1].as_str(), "marquee");
    assert_eq!(screens[0].kind.as_str(), "journal");

    // `value: null` is a value and survives.
    let mut null_value = minimal();
    null_value["screens"] = json!([{"kind": "combat_hud",
        "sections": [{"type": "carousel", "value": null}]}]);
    let ui = read_ui(&null_value);
    assert_eq!(
        ui.screens.as_ref().unwrap()[0].sections[0],
        UiSection::Unknown {
            kind: "carousel".into(),
            value: Some(Value::Null)
        }
    );
    assert_eq!(
        write_document(&Document::Ui(ui)).unwrap(),
        bytes(&null_value)
    );

    // The campaign loads with the unknown kinds in it (the server path).
    let ruleset = ulid(RULESET);
    let files = combat_basic_with("ui/odd.json", &value);
    let loaded = load_campaign(&files, &engine()).unwrap();
    assert!(
        matches!(&loaded.documents[&sp("ui/odd.json")], Document::Ui(ui) if ui.ruleset == ruleset)
    );
    let written = serialize_campaign(&loaded).unwrap();
    assert_eq!(written[&sp("ui/odd.json")], input);

    // Control: known spellings decode to their known variants.
    let known = read_ui(&kitchen_sink());
    assert_eq!(
        known.screens.as_ref().unwrap()[0].kind,
        UiScreenKind::CharacterSheet
    );
    assert_eq!(
        known.pools.as_ref().unwrap()[0].style,
        Some(UiPoolStyle::Pips)
    );
    assert_eq!(
        known.abilities.as_ref().unwrap()[0].target_hint,
        Some(UiTargetHint::Enemy)
    );

    // Every constant spelling decodes to a known variant and `as_str` returns it.
    for kind in UI_SCREEN_KINDS {
        let mut value = minimal();
        value["screens"] = json!([{"kind": kind, "sections": []}]);
        let ui = read_ui(&value);
        let decoded = &ui.screens.as_ref().unwrap()[0].kind;
        assert!(!matches!(decoded, UiScreenKind::Unknown(_)), "{kind}");
        assert_eq!(decoded.as_str(), kind);
    }
    for (i, hint) in UI_TARGET_HINTS.iter().enumerate() {
        let mut value = kitchen_sink();
        set(&mut value, "/abilities/0/target_hint", json!(hint));
        let ui = read_ui(&value);
        let decoded = ui.abilities.unwrap()[0].target_hint.clone().unwrap();
        assert!(!matches!(decoded, UiTargetHint::Unknown(_)));
        assert_eq!(decoded.as_str(), *hint);
        assert_eq!(decoded == UiTargetHint::NoTarget, i == 0);
    }
}

#[test]
fn known_kind_wrong_body() {
    // U-5 (B3, B4). Control: the correct bodies read.
    let sections = |section: Value| {
        let mut value = minimal();
        value["screens"] = json!([{"kind": "combat_hud", "sections": [section]}]);
        value
    };
    let format = |format: Value| {
        let mut value = minimal();
        value["stats"] = json!([{"stat": "s", "label": "l", "format": format}]);
        value
    };
    read_value(&sections(json!({"type": "timeline"}))).unwrap();
    read_value(&sections(
        json!({"type": "stat_group", "value": {"type": "all"}}),
    ))
    .unwrap();
    read_value(&format(
        json!({"type": "fraction", "value": {"type": "stat", "value": "m"}}),
    ))
    .unwrap();

    // The unit-variant trap: an extra `value` on a unit kind must not be ignored.
    for unit in ["effect_list", "timeline", "log", "choice_steps"] {
        assert_eq!(
            read_err(&sections(json!({"type": unit, "value": 0}))),
            "unknown field `value`, expected `type`",
            "{unit}"
        );
    }
    for unit in ["integer", "signed", "percent", "boolean", "tags", "dice"] {
        assert_eq!(
            read_err(&format(json!({"type": unit, "value": 0}))),
            "unknown field `value`, expected `type`",
            "{unit}"
        );
    }
    for needs in ["stat_group", "pool_group", "ability_list", "item_list"] {
        assert_eq!(
            read_err(&sections(json!({"type": needs}))),
            "missing field `value`",
            "{needs}"
        );
    }
    for needs in ["fraction", "enum"] {
        assert_eq!(
            read_err(&format(json!({"type": needs}))),
            "missing field `value`",
            "{needs}"
        );
    }
    assert_eq!(
        read_err(&sections(json!({"value": 1}))),
        "missing field `type`"
    );

    // Selectors and meters are closed.
    let select = |select: Value| sections(json!({"type": "stat_group", "value": select}));
    assert_eq!(
        read_err(&select(json!({"type": "all", "value": []}))),
        "unknown field `value`, expected `type`"
    );
    assert_eq!(
        read_err(&select(json!({"type": "group"}))),
        "missing field `value`"
    );
    assert_eq!(
        read_err(&select(json!({"type": "names", "value": "x"}))),
        "invalid type: string \"x\", expected a sequence"
    );
    assert_eq!(
        read_err(&select(json!({"type": "bogus"}))),
        "unknown variant `bogus`, expected one of `all`, `group`, `names`"
    );
    assert_eq!(
        read_err(&sections(
            json!({"type": "pool_group", "value": {"type": "tag", "value": "x"}})
        )),
        "unknown variant `tag`, expected `all` or `ids`"
    );
    assert_eq!(
        read_err(&sections(
            json!({"type": "item_list", "value": {"type": "ids", "value": []}})
        )),
        "unknown variant `ids`, expected `all` or `tag`"
    );
    assert_eq!(
        read_err(&sections(json!({"type": "ability_list",
            "value": {"select": {"type": "group", "value": "x"}}}))),
        "unknown variant `group`, expected one of `all`, `ids`, `tag`"
    );
    assert_eq!(
        read_err(&sections(json!({"type": "ability_list",
            "value": {"select": {"type": "all"}, "extra": 1}}))),
        "unknown field `extra`, expected `select` or `sort`"
    );
    assert_eq!(
        read_err(&format(
            json!({"type": "fraction", "value": {"type": "bogus", "value": "m"}})
        )),
        "unknown variant `bogus`, expected `stat` or `pool`"
    );
    assert_eq!(
        read_err(&format(
            json!({"type": "fraction", "value": {"type": "stat", "value": "m", "x": 1}})
        )),
        "invalid value: string \"x\", expected \"type\" or \"value\""
    );

    // An unknown section or format must have exactly {type} or {type, value}.
    assert_eq!(
        read_err(&sections(
            json!({"type": "carousel", "value": 1, "extra": 2})
        )),
        "unknown field `extra`, expected `type`"
    );
    assert_eq!(
        read_err(&format(json!({"type": "money", "extra": 2}))),
        "unknown field `extra`, expected `type`"
    );
    read_value(&sections(json!({"type": "carousel"}))).unwrap();
    read_value(&sections(json!({"type": "carousel", "value": [1]}))).unwrap();
}

#[test]
fn known_spelling_as_unknown_rejected() {
    // U-6 (B6)
    let write = |ui: UiDocument| write_document(&Document::Ui(ui)).map(|_| ());
    let doc = |screens: Vec<UiScreen>| UiDocument {
        ruleset: ulid(RULESET),
        stats: None,
        pools: None,
        abilities: None,
        effects: None,
        outcomes: None,
        screens: Some(screens),
        note: None,
    };
    let screen = |kind: UiScreenKind, sections: Vec<UiSection>| UiScreen { kind, sections };

    assert_eq!(
        malformed_message(
            write(doc(vec![screen(
                UiScreenKind::Unknown("spellbook".into()),
                vec![]
            )]))
            .unwrap_err()
        ),
        "ui unknown screen kind at /screens/0/kind uses a known spelling"
    );
    write(doc(vec![screen(
        UiScreenKind::Unknown("journal".into()),
        vec![],
    )]))
    .unwrap();

    for (known, fresh) in [("timeline", "carousel"), ("choice_steps", "carousel")] {
        let unknown = |kind: &str| {
            doc(vec![screen(
                UiScreenKind::CombatHud,
                vec![UiSection::Unknown {
                    kind: kind.into(),
                    value: None,
                }],
            )])
        };
        assert_eq!(
            malformed_message(write(unknown(known)).unwrap_err()),
            "ui unknown section type at /screens/0/sections/0/type uses a known spelling"
        );
        write(unknown(fresh)).unwrap();
    }

    let with_stat = |format: UiFormat| UiDocument {
        stats: Some(vec![UiStat {
            stat: "s".into(),
            label: "l".into(),
            short_label: None,
            description: None,
            group: None,
            format: Some(format),
        }]),
        ..doc(vec![])
    };
    let unknown_format = |kind: &str| UiFormat::Unknown {
        kind: kind.into(),
        value: Some(json!(1)),
    };
    assert_eq!(
        malformed_message(write(with_stat(unknown_format("dice"))).unwrap_err()),
        "ui unknown format at /stats/0/format/type uses a known spelling"
    );
    write(with_stat(unknown_format("money"))).unwrap();

    let with_pool = |style: &str| UiDocument {
        pools: Some(vec![UiPool {
            pool: ulid(POOL),
            label: "l".into(),
            icon: None,
            style: Some(UiPoolStyle::Unknown(style.into())),
            show_on_hud: None,
        }]),
        ..doc(vec![])
    };
    assert_eq!(
        malformed_message(write(with_pool("bar")).unwrap_err()),
        "ui unknown pool style at /pools/0/style uses a known spelling"
    );
    write(with_pool("dial")).unwrap();

    let with_hint = |hint: &str| UiDocument {
        abilities: Some(vec![UiAbility {
            ability: ulid(ABILITY),
            label: "l".into(),
            description: None,
            icon: None,
            category_tags: None,
            target_hint: Some(UiTargetHint::Unknown(hint.into())),
            show_odds: None,
        }]),
        ..doc(vec![])
    };
    assert_eq!(
        malformed_message(write(with_hint("none")).unwrap_err()),
        "ui unknown target hint at /abilities/0/target_hint uses a known spelling"
    );
    write(with_hint("self")).unwrap();

    let with_sort = |sort: &str| {
        doc(vec![screen(
            UiScreenKind::ActionBar,
            vec![UiSection::AbilityList(UiAbilityList {
                select: UiAbilitySelect::All,
                sort: Some(UiSort::Unknown(sort.into())),
            })],
        )])
    };
    assert_eq!(
        malformed_message(write(with_sort("label")).unwrap_err()),
        "ui unknown sort at /screens/0/sections/0/value/sort uses a known spelling"
    );
    write(with_sort("newest")).unwrap();
}

#[test]
fn layout_and_classifier() {
    // U-7 (B7). Control: the campaign without the file loads.
    let base = fixture("combat_basic");
    load_campaign(&base, &engine()).unwrap();

    let ui = bytes(&wire_example());
    let mut ok = base.clone();
    ok.insert(sp("ui/a.json"), ui.clone());
    load_campaign(&ok, &engine()).unwrap();
    let mut nested = base.clone();
    nested.insert(sp("ui/x/y.json"), ui.clone());
    load_campaign(&nested, &engine()).unwrap();

    let creature = base[&sp("creatures/hero.json")].clone();
    let cases = [
        ("rulesets/a.json", ui.clone()),
        ("a.json", ui.clone()),
        ("ui/a.json", creature),
    ];
    for (path, content) in cases {
        let mut files = base.clone();
        files.insert(sp(path), content);
        let error = load_campaign(&files, &engine()).unwrap_err();
        assert!(
            matches!(
                &error,
                DataError::Layout { path: Some(at), message }
                    if *at == sp(path) && message == "document kind does not match its storage path"
            ),
            "{path}: {error:?}"
        );
        assert_eq!(
            error.to_string(),
            format!("invalid layout at {path}: document kind does not match its storage path")
        );
    }

    assert_eq!(
        campaign_document_path("ui/a.json").unwrap(),
        Some(sp("ui/a.json"))
    );
    assert_eq!(
        campaign_document_path("ui/x/y.json").unwrap(),
        Some(sp("ui/x/y.json"))
    );
    assert_eq!(campaign_document_path("ui/.json").unwrap(), None);
    assert_eq!(campaign_document_path("ui.json").unwrap(), None);
    assert_eq!(campaign_document_path("ui/a.txt").unwrap(), None);
}

fn defect_ui() -> Value {
    // One defect of every B12-B23 kind, all of them semantic.
    json!({
        "schema": "crpg.ui/1", "ruleset": RULESET,
        "stats": [
            {"stat": "might", "label": "no.such.key", "format": {"type": "boolean"}},
            {"stat": "might", "label": "no.such.key"},
            {"stat": "nope", "label": "l",
             "format": {"type": "fraction", "value": {"type": "stat", "value": "ghost"}}},
            {"stat": "guile", "label": "l", "group": "no.such.group",
             "format": {"type": "enum", "value": [{"variant": "a", "label": "l"},
                                                   {"variant": "a", "label": "l"}]}}],
        "pools": [{"pool": "0000000000000000000000000Z", "label": "l", "icon": "no/icon.png",
                   "style": "dial"}],
        "abilities": [{"ability": "0000000000000000000000000Z", "label": "l",
                       "category_tags": ["a", "a"], "target_hint": "self"}],
        "effects": [{"effect": "0000000000000000000000000Z", "label": "l"}],
        "outcomes": [{"outcome": "Success", "label": "l"}, {"outcome": "custom:01", "label": "l"}],
        "screens": [
            {"kind": "level_up", "sections": [{"type": "choice_steps"}]},
            {"kind": "journal", "sections": []},
            {"kind": "spellbook", "sections": [
                {"type": "stat_group", "value": {"type": "group", "value": "missing"}},
                {"type": "pool_group", "value": {"type": "ids", "value": [POOL, POOL]}},
                {"type": "ability_list", "value": {"select": {"type": "tag", "value": "none"},
                                                    "sort": "newest"}},
                {"type": "item_list", "value": {"type": "tag", "value": "none"}},
                {"type": "carousel"}]},
            {"kind": "spellbook", "sections": []}]
    })
}

#[test]
fn loading_is_unaffected() {
    // U-8 (B8, B9)
    let base = fixture("combat_basic");
    let without = load_campaign(&base, &engine()).unwrap();
    let files = combat_basic_with("ui/default.json", &wire_example());
    let with = load_campaign(&files, &engine()).unwrap();
    assert_eq!(with.index, without.index);
    assert!(!with.index.values().any(|e| e.path == sp("ui/default.json")));
    let other = |written: BTreeMap<SourcePath, Vec<u8>>| {
        written
            .into_iter()
            .filter(|(path, _)| path.as_str() != "ui/default.json")
            .collect::<BTreeMap<_, _>>()
    };
    let written_with = serialize_campaign(&with).unwrap();
    assert_eq!(written_with[&sp("ui/default.json")], bytes(&wire_example()));
    assert_eq!(
        other(written_with),
        other(serialize_campaign(&without).unwrap())
    );
    // Writing the UI file changes no other byte of the campaign lock either.
    assert_eq!(with.documents.len(), without.documents.len() + 1);

    // A UI file holding semantic defects of every kind still loads.
    let defects = combat_basic_with("ui/defects.json", &defect_ui());
    let loaded = load_campaign(&defects, &engine()).unwrap();
    assert_eq!(loaded.index, without.index);
    assert_eq!(validate_files(&defects, &engine()), validate(&loaded));
    // And the same defects with a dangling ruleset load too.
    let mut dangling = defect_ui();
    dangling["ruleset"] = json!("0000000000000000000000000Z");
    let loaded = load_campaign(&combat_basic_with("ui/dangling.json", &dangling), &engine());
    assert_eq!(loaded.unwrap().index, without.index);

    // Structural defects still fail the load, as in every other family.
    let mut broken = defect_ui();
    broken["bogus"] = json!(1);
    let error =
        load_campaign(&combat_basic_with("ui/broken.json", &broken), &engine()).unwrap_err();
    assert!(matches!(error, DataError::Malformed { .. }), "{error:?}");

    // `validate` still reports the campaign's own findings unchanged.
    let clean = validate(&without);
    let with_ui = validate(&with);
    let outside_ui: Vec<_> = with_ui
        .iter()
        .filter(|d| d.file.as_ref().map(|f| f.as_str()) != Some("ui/default.json"))
        .cloned()
        .collect();
    assert_eq!(outside_ui, clean);
}

#[test]
fn generated_ui_literals() {
    // U-17 (B25-B29)
    let loaded = load_campaign(&fixture("combat_basic"), &engine()).unwrap();
    let source = UiSource::from_documents(&loaded.documents, ulid(RULESET)).unwrap();
    let generated = generated_ui(&source);
    let written = write_document(&Document::Ui(generated.clone())).unwrap();
    let actual: Value = serde_json::from_slice(&written).unwrap();
    let literal = json!({
        "abilities": [{"ability": ABILITY, "label": "fixture.strike", "target_hint": "enemy"}],
        "effects": [],
        "outcomes": [{"label": "success", "outcome": "success"},
                     {"label": "failure", "outcome": "failure"}],
        "pools": [{"label": "pool-1", "pool": POOL}],
        "ruleset": RULESET, "schema": "crpg.ui/1",
        "screens": [
            {"kind": "character_sheet", "sections": [{"type": "stat_group", "value": {"type": "all"}}]},
            {"kind": "action_bar", "sections": [{"type": "ability_list", "value": {"select": {"type": "all"}}}]},
            {"kind": "spellbook", "sections": [{"type": "ability_list",
                "value": {"select": {"type": "all"}, "sort": "label"}}]},
            {"kind": "inventory", "sections": [{"type": "item_list", "value": {"type": "all"}}]},
            {"kind": "combat_hud", "sections": [
                {"type": "stat_group", "value": {"type": "names", "value": ["health"]}},
                {"type": "pool_group", "value": {"type": "all"}},
                {"type": "timeline"}, {"type": "log"}]}],
        "stats": [{"label": "might", "stat": "might"}, {"label": "guile", "stat": "guile"},
                  {"label": "resolve", "stat": "resolve"}, {"label": "health", "stat": "health"}]
    });
    assert_eq!(actual, literal);
    assert_eq!(
        read_document(&written).unwrap(),
        Document::Ui(generated.clone())
    );
    assert_eq!(generated.note, None);

    // combat_srd: two pools, two abilities, one effect, three outcomes.
    let srd = load_campaign(&fixture("combat_srd"), &engine()).unwrap();
    let source =
        UiSource::from_documents(&srd.documents, ulid("00000000000000000000000016")).unwrap();
    assert_eq!(
        source.outcome_labels(),
        ["failure", "success", "critical_success"]
    );
    let ui = generated_ui(&source);
    let pools = ui.pools.unwrap();
    assert_eq!(
        pools
            .iter()
            .map(|p| (p.pool, p.label.as_str()))
            .collect::<Vec<_>>(),
        [
            (ulid("0000000000000000000000001E"), "pool-1"),
            (ulid("0000000000000000000000001F"), "pool-2")
        ]
    );
    let abilities = ui.abilities.unwrap();
    assert_eq!(
        abilities
            .iter()
            .map(|a| (a.ability, a.label.as_str(), a.target_hint.clone()))
            .collect::<Vec<_>>(),
        [
            (
                ulid("00000000000000000000000017"),
                "fixture.heavy",
                Some(UiTargetHint::Enemy)
            ),
            (
                ulid("00000000000000000000000018"),
                "fixture.focus",
                Some(UiTargetHint::Any)
            )
        ]
    );
    assert_eq!(
        ui.effects.unwrap(),
        [UiEffect {
            effect: ulid("0000000000000000000000001B"),
            label: "fixture.focusing".into(),
            icon: None,
            show_duration: None
        }]
    );
    assert_eq!(
        ui.outcomes
            .unwrap()
            .iter()
            .map(|o| (o.outcome.as_str(), o.label.as_str()))
            .collect::<Vec<_>>(),
        [
            ("failure", "failure"),
            ("success", "success"),
            ("critical_success", "critical_success")
        ]
    );
    assert_eq!(
        ui.stats
            .unwrap()
            .iter()
            .map(|s| s.stat.as_str())
            .collect::<Vec<_>>(),
        ["might", "guile", "resolve", "ward", "health"]
    );

    // No ruleset with that id: nothing to resolve.
    assert!(
        UiSource::from_documents(&loaded.documents, ulid("0000000000000000000000000Z")).is_none()
    );
    // A creature id is not a ruleset.
    assert!(UiSource::from_documents(&loaded.documents, ulid(POOL)).is_none());
}

#[test]
fn outcome_text_and_grammar() {
    // B26 spellings and the B18 grammar (exact inverse).
    let names = [
        (OutcomeWire::CriticalSuccess, "critical_success"),
        (OutcomeWire::Success, "success"),
        (OutcomeWire::Failure, "failure"),
        (OutcomeWire::CriticalFailure, "critical_failure"),
        (OutcomeWire::Custom(0), "custom:0"),
        (OutcomeWire::Custom(255), "custom:255"),
    ];
    for (outcome, text) in names {
        assert_eq!(ui_outcome_text(outcome), text);
        assert_eq!(ui_outcome(text), Some(outcome));
    }
    for bad in [
        "custom:",
        "custom:256",
        "custom:01",
        "custom:00",
        "custom:-1",
        "custom:+1",
        "custom:1 ",
        "Success",
        "",
        "custom",
        "custom:x",
        "custom:１",
    ] {
        assert_eq!(ui_outcome(bad), None, "{bad:?}");
    }
}

#[test]
fn effective_ui_merge() {
    // U-18 (B30)
    let loaded = load_campaign(&fixture("combat_basic"), &engine()).unwrap();
    let source = UiSource::from_documents(&loaded.documents, ulid(RULESET)).unwrap();
    let generated = generated_ui(&source);
    assert_eq!(effective_ui(None, &source), generated);

    let authored = UiDocument {
        ruleset: ulid("0000000000000000000000000Z"),
        stats: Some(vec![
            UiStat {
                stat: "health".into(),
                label: "ui.health".into(),
                short_label: None,
                description: None,
                group: None,
                format: None,
            },
            UiStat {
                stat: "undeclared".into(),
                label: "ui.who".into(),
                short_label: None,
                description: None,
                group: None,
                format: None,
            },
        ]),
        screens: Some(vec![
            UiScreen {
                kind: UiScreenKind::ActionBar,
                sections: vec![UiSection::Timeline],
            },
            UiScreen {
                kind: UiScreenKind::LevelUp,
                sections: vec![UiSection::ChoiceSteps],
            },
            UiScreen {
                kind: UiScreenKind::Unknown("journal".into()),
                sections: vec![],
            },
        ]),
        note: Some("dropped".into()),
        ..generated.clone()
    };
    let authored = UiDocument {
        pools: None,
        abilities: None,
        effects: None,
        outcomes: None,
        ..authored
    };
    let merged = effective_ui(Some(&authored), &source);
    assert_eq!(merged.ruleset, ulid(RULESET));
    assert_eq!(merged.note, None);
    let names: Vec<_> = merged
        .stats
        .as_ref()
        .unwrap()
        .iter()
        .map(|s| (s.stat.as_str(), s.label.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            ("health", "ui.health"),
            ("undeclared", "ui.who"),
            ("might", "might"),
            ("guile", "guile"),
            ("resolve", "resolve")
        ]
    );
    let kinds: Vec<_> = merged
        .screens
        .as_ref()
        .unwrap()
        .iter()
        .map(|s| s.kind.as_str().to_owned())
        .collect();
    assert_eq!(
        kinds,
        [
            "action_bar",
            "level_up",
            "journal",
            "character_sheet",
            "spellbook",
            "inventory",
            "combat_hud"
        ]
    );
    // The authored action_bar is kept unchanged, not replaced by the default.
    assert_eq!(
        merged.screens.as_ref().unwrap()[0].sections,
        vec![UiSection::Timeline]
    );
    // Absent authored lists take the generated lists whole.
    assert_eq!(merged.pools, generated.pools);
    assert_eq!(merged.abilities, generated.abilities);
    assert_eq!(merged.effects, generated.effects);
    assert_eq!(merged.outcomes, generated.outcomes);

    // Authored entries with the same key are not duplicated or filled in.
    let partial = UiDocument {
        outcomes: Some(vec![UiOutcome {
            outcome: "failure".into(),
            label: "ui.miss".into(),
        }]),
        pools: Some(Vec::new()),
        ..authored.clone()
    };
    let merged = effective_ui(Some(&partial), &source);
    assert_eq!(
        merged.outcomes.unwrap(),
        [
            UiOutcome {
                outcome: "failure".into(),
                label: "ui.miss".into()
            },
            UiOutcome {
                outcome: "success".into(),
                label: "success".into()
            }
        ]
    );
    assert_eq!(merged.pools, generated.pools);

    // Control: a fully authored document comes back with `ruleset` set and `note` dropped.
    let full = UiDocument {
        ruleset: ulid("0000000000000000000000000Z"),
        note: Some("n".into()),
        ..generated.clone()
    };
    assert_eq!(effective_ui(Some(&full), &source), generated);

    // Generated and effective documents pass the read-time checks and round trip.
    for ui in [generated.clone(), effective_ui(Some(&authored), &source)] {
        let written = write_document(&Document::Ui(ui.clone())).unwrap();
        assert_eq!(read_document(&written).unwrap(), Document::Ui(ui));
    }
}

fn type_keys(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(kind) = map.get("type") {
                // A `type` key under `properties` is a property named `type`; its
                // value is a schema object, never a JSON string or array.
                match kind {
                    Value::String(text) => found.push(text.clone()),
                    Value::Array(items) => {
                        found.extend(items.iter().filter_map(|i| i.as_str().map(str::to_owned)))
                    }
                    _ => {}
                }
            }
            for child in map.values() {
                type_keys(child, found);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| type_keys(i, found)),
        _ => {}
    }
}

/// Every `const` and `enum` string in the schema.
fn literal_strings(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                match (key.as_str(), child) {
                    ("const", Value::String(text)) => found.push(text.clone()),
                    ("enum", Value::Array(items)) => {
                        found.extend(items.iter().filter_map(|i| i.as_str().map(str::to_owned)))
                    }
                    _ => {}
                }
                literal_strings(child, found);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| literal_strings(i, found)),
        _ => {}
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

fn variant_consts(def: &Value) -> Vec<String> {
    def["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|branch| {
            branch["properties"]["type"]["const"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

fn checked_in_schema() -> Value {
    let path = repo_root().join("schemas").join("ui.schema.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn no_numbers_and_pinned_kinds() {
    // U-20 (B32, B33). The constants equal literal arrays written here.
    assert_eq!(
        UI_SCREEN_KINDS,
        [
            "character_sheet",
            "action_bar",
            "spellbook",
            "inventory",
            "level_up",
            "combat_hud",
            "character_creation"
        ]
    );
    assert_eq!(
        UI_SECTION_TYPES,
        [
            "stat_group",
            "pool_group",
            "ability_list",
            "effect_list",
            "item_list",
            "timeline",
            "log",
            "choice_steps"
        ]
    );
    assert_eq!(
        UI_FORMAT_TYPES,
        ["integer", "signed", "fraction", "percent", "boolean", "enum", "tags", "dice"]
    );
    assert_eq!(UI_POOL_STYLES, ["pips", "bar", "number"]);
    assert_eq!(UI_TARGET_HINTS, ["none", "enemy", "ally", "any"]);
    assert_eq!(UI_SORTS, ["declared", "label"]);
    assert_eq!(
        (
            UI_SCHEMA,
            MAX_UI_ENTRIES,
            MAX_UI_SECTIONS,
            MAX_UI_TEXT_BYTES
        ),
        ("crpg.ui/1", 4096, 64, 128)
    );

    let schema = checked_in_schema();
    let mut types = Vec::new();
    type_keys(&schema, &mut types);
    assert!(!types.is_empty());
    assert!(
        !types.iter().any(|t| t == "integer" || t == "number"),
        "{types:?}"
    );

    let defs = &schema["$defs"];
    assert_eq!(strings(&defs["UiScreenKind"]["enum"]), UI_SCREEN_KINDS);
    assert_eq!(strings(&defs["UiPoolStyle"]["enum"]), UI_POOL_STYLES);
    assert_eq!(strings(&defs["UiTargetHint"]["enum"]), UI_TARGET_HINTS);
    assert_eq!(strings(&defs["UiSort"]["enum"]), UI_SORTS);
    assert_eq!(variant_consts(&defs["UiSection"]), UI_SECTION_TYPES);
    assert_eq!(variant_consts(&defs["UiFormat"]), UI_FORMAT_TYPES);
    // `Unknown` is never listed.
    let mut literals = Vec::new();
    literal_strings(&schema, &mut literals);
    assert!(literals.len() > 40);
    assert!(
        literals.iter().all(|text| !text.contains("nknown")),
        "{literals:?}"
    );
    assert!(schema["$defs"].get("UiFormatUnknown").is_none());
}

#[test]
fn schema_and_registry() {
    // U-21 (B34)
    let generated = generated_schemas().unwrap();
    assert_eq!(generated.len(), 23);
    let on_disk = std::fs::read(repo_root().join("schemas").join("ui.schema.json")).unwrap();
    assert_eq!(generated["ui.schema.json"], on_disk);

    let schema = checked_in_schema();
    let branches = schema["oneOf"].as_array().unwrap();
    assert_eq!(branches.len(), 1);
    assert_eq!(branches[0]["properties"]["schema"]["const"], "crpg.ui/1");
    assert_eq!(branches[0]["additionalProperties"], false);
    assert_eq!(schema["title"], "UiDocument");

    let versions = schema_versions();
    assert_eq!(versions.len(), 21);
    assert!(versions.contains(&SchemaVersion {
        schema_type: "crpg.ui",
        current: 1
    }));
    let names: Vec<_> = versions.iter().map(|v| v.schema_type).collect();
    let position = names.iter().position(|n| *n == "crpg.ui").unwrap();
    assert_eq!(names[position - 1], "crpg.triggers");
    assert_eq!(names[position + 1], "crpg.variables");

    let mut value = wire_example();
    let before = value.clone();
    migrate_document(&mut value).unwrap();
    assert_eq!(value, before);
    // No edge: the next version does not exist.
    let mut future = wire_example();
    future["schema"] = json!("crpg.ui/2");
    assert!(matches!(
        migrate_document(&mut future).unwrap_err(),
        DataError::UnsupportedSchema { .. }
    ));
}

#[test]
fn diagnostic_codes_for_ui_spell_as_the_contract_says() {
    // The eight codes exist and spell as contracted; their emission is T142b.
    let table = [
        (DiagnosticCode::UnknownKind, "unknown_kind"),
        (DiagnosticCode::ReservedKind, "reserved_kind"),
        (DiagnosticCode::DuplicateBinding, "duplicate_binding"),
        (DiagnosticCode::UnknownOutcome, "unknown_outcome"),
        (DiagnosticCode::UnknownTag, "unknown_tag"),
        (DiagnosticCode::UnknownGroup, "unknown_group"),
        (DiagnosticCode::UnknownVariant, "unknown_variant"),
        (DiagnosticCode::FormatMismatch, "format_mismatch"),
    ];
    for (code, text) in table {
        assert_eq!(code.as_str(), text);
        assert_eq!(serde_json::to_string(&code).unwrap(), format!("\"{text}\""));
    }
    // They sort after every existing code, in the contract's order.
    let mut sorted: Vec<_> = table.iter().map(|(c, _)| *c).collect();
    sorted.sort();
    assert_eq!(sorted, table.iter().map(|(c, _)| *c).collect::<Vec<_>>());
    assert!(DiagnosticCode::RealTimeFieldsWithoutRoundTicks < DiagnosticCode::UnknownKind);
}

#[test]
fn ui_files_add_generic_reference_findings_only_where_generic() {
    // B10: the generic pass reports a dangling ruleset, ability id and effect
    // id at their pointers; pool sites are PoolTemplate (walk_ui, T142b).
    let missing = "0000000000000000000000000Z";
    let ui = json!({
        "schema": "crpg.ui/1", "ruleset": missing,
        "pools": [{"pool": missing, "label": "l"}],
        "abilities": [{"ability": missing, "label": "l"}],
        "effects": [{"effect": missing, "label": "l"}],
        "screens": [
            {"kind": "journal", "sections": [
                {"type": "pool_group", "value": {"type": "ids", "value": [missing]}},
                {"type": "ability_list", "value": {"select": {"type": "ids", "value": [missing]}}}]}]
    });
    let loaded = load_campaign(&combat_basic_with("ui/d.json", &ui), &engine()).unwrap();
    let found: Vec<_> = validate(&loaded)
        .into_iter()
        .filter(|d| d.file.as_ref().map(|f| f.as_str()) == Some("ui/d.json"))
        .map(|d| (d.pointer, d.code, d.message))
        .collect();
    let dangling = |what: &str| format!("dangling {what} reference to {missing}");
    assert_eq!(
        found,
        [
            (
                "/abilities/0/ability".to_owned(),
                DiagnosticCode::DanglingReference,
                dangling("ui ability")
            ),
            (
                "/effects/0/effect".to_owned(),
                DiagnosticCode::DanglingReference,
                dangling("ui effect")
            ),
            (
                "/ruleset".to_owned(),
                DiagnosticCode::DanglingReference,
                dangling("ui ruleset")
            ),
            (
                "/screens/0/sections/1/value/select/value/0".to_owned(),
                DiagnosticCode::DanglingReference,
                dangling("ui ability")
            ),
        ]
    );
    // A creature id as the ruleset is the wrong kind.
    let mut wrong = ui.clone();
    wrong["ruleset"] = json!(ulid_of_hero());
    let loaded = load_campaign(&combat_basic_with("ui/d.json", &wrong), &engine()).unwrap();
    let kinds: Vec<_> = validate(&loaded)
        .into_iter()
        .filter(|d| d.pointer == "/ruleset")
        .map(|d| (d.code, d.message))
        .collect();
    assert_eq!(kinds.len(), 1);
    assert_eq!(kinds[0].0, DiagnosticCode::WrongReferenceKind);
    assert!(
        kinds[0]
            .1
            .starts_with("wrong kind for ui ruleset reference to "),
        "{}",
        kinds[0].1
    );
    assert!(
        kinds[0].1.ends_with("found creature, expected ruleset"),
        "{}",
        kinds[0].1
    );
}

fn ulid_of_hero() -> String {
    let files = fixture("combat_basic");
    match read_document(&files[&sp("creatures/hero.json")]).unwrap() {
        Document::Creature(creature) => creature.id.to_string(),
        other => panic!("{other:?}"),
    }
}
