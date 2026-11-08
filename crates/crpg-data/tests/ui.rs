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
        // T142b adds locale and `unknown_kind` findings to this file; this
        // test pins only the generic reference pass.
        .filter(|d| d.code == DiagnosticCode::DanglingReference)
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
// ---------------------------------------------------------------------------
// T142b: `walk_ui` and the eight codes (B11-B24), plus review-added gap tests.

const L: &str = "fixture.strike"; // a key `combat_basic`'s locale table holds
const SRD: &str = "00000000000000000000000016";
const SRD_HEAVY: &str = "00000000000000000000000017";
const SRD_FOCUS: &str = "00000000000000000000000018";
const SRD_EFFECT: &str = "0000000000000000000000001B";
const SRD_POOL1: &str = "0000000000000000000000001E";
const SRD_POOL2: &str = "0000000000000000000000001F";
const MISSING: &str = "0000000000000000000000000Z";
const EXTRA: &str = "0000000000000000000000000Y";
const GONE: &str = "0000000000000000000000000X";

const FIX_DANGLING: &str = "point at an existing object id or create the target";
const FIX_WRONG: &str = "point at an object of the expected kind";
const FIX_KIND: &str = "use a kind this schema version defines";
const FIX_RESERVED: &str = "remove it; the generated default omits reserved kinds";
const FIX_DUP: &str = "keep one entry";
const FIX_STAT: &str = "use a stat the ruleset declares";
const FIX_POOL: &str = "use a pool template id of the ruleset";
const FIX_ABILITY: &str = "list the ability in the ruleset or remove the entry";
const FIX_EFFECT: &str = "use an effect a listed ability applies or remove the entry";
const FIX_MISMATCH: &str = "choose a format that suits the stat's kind";

fn campaign(base: &str) -> LoadedCampaign {
    load_campaign(&fixture(base), &engine()).unwrap()
}

fn finding(
    file: &str,
    pointer: &str,
    severity: Severity,
    code: DiagnosticCode,
    message: &str,
    fix: &str,
) -> Diagnostic {
    Diagnostic {
        file: Some(sp(file)),
        pointer: pointer.into(),
        severity,
        code,
        message: message.into(),
        suggested_fix: Some(fix.into()),
    }
}

fn err(file: &str, pointer: &str, code: DiagnosticCode, message: &str, fix: &str) -> Diagnostic {
    finding(file, pointer, Severity::Error, code, message, fix)
}

fn ui_with(fields: Value) -> Value {
    let mut value = minimal();
    for (key, field) in fields.as_object().unwrap() {
        value[key] = field.clone();
    }
    value
}

/// Inserts the UI documents into a copy of `base` (typed, no byte round trip),
/// validates, and returns the findings in those files. Asserts the two
/// contract guards of every case: every other finding equals the base
/// campaign's, and no pointer carries two findings (locale coverage aside).
fn run(base: &LoadedCampaign, files: &[(&str, &Value)]) -> Vec<Diagnostic> {
    let mut campaign = base.clone();
    for (name, ui) in files {
        campaign.documents.insert(sp(name), read_value(ui).unwrap());
    }
    let names: Vec<&str> = files.iter().map(|(name, _)| *name).collect();
    let (ui, rest): (Vec<_>, Vec<_>) = validate(&campaign).into_iter().partition(|d| {
        d.file
            .as_ref()
            .is_some_and(|file| names.contains(&file.as_str()))
    });
    assert_eq!(rest, validate(base), "UI files changed other findings");
    let mut seen = std::collections::BTreeSet::new();
    for d in &ui {
        if d.code != DiagnosticCode::MissingLocaleKey {
            assert!(
                seen.insert((d.file.clone(), d.pointer.clone())),
                "two findings at one pointer: {d:?}"
            );
        }
    }
    ui
}

fn run_one(base: &LoadedCampaign, ui: &Value) -> Vec<Diagnostic> {
    run(base, &[("ui/x.json", ui)])
}

fn ruleset_mut(campaign: &mut LoadedCampaign) -> &mut Ruleset {
    campaign
        .documents
        .values_mut()
        .find_map(|d| match d {
            Document::Ruleset(r) => Some(r),
            _ => None,
        })
        .unwrap()
}

fn locales_mut(campaign: &mut LoadedCampaign) -> Vec<&mut LocaleDocument> {
    campaign
        .documents
        .values_mut()
        .filter_map(|d| match d {
            Document::Locale(l) => Some(l),
            _ => None,
        })
        .collect()
}

fn ability_clone(docs: &BTreeMap<SourcePath, Document>, path: &str) -> Ability {
    match &docs[&sp(path)] {
        Document::Ability(a) => a.clone(),
        other => panic!("{other:?}"),
    }
}

fn add_ability(docs: &mut BTreeMap<SourcePath, Document>, path: &str, mut ability: Ability) {
    ability.slug = path.replace(['/', '.'], "-");
    docs.insert(sp(path), Document::Ability(ability));
}

fn ability_of<'a>(docs: &'a mut BTreeMap<SourcePath, Document>, path: &str) -> &'a mut Ability {
    match docs.get_mut(&sp(path)).unwrap() {
        Document::Ability(a) => a,
        other => panic!("{other:?}"),
    }
}

fn effect_clone(docs: &BTreeMap<SourcePath, Document>, path: &str) -> Effect {
    match &docs[&sp(path)] {
        Document::Effect(e) => e.clone(),
        other => panic!("{other:?}"),
    }
}

fn damage(outcome: OutcomeWire) -> DamageEntry {
    DamageEntry {
        outcome,
        amount: 0,
        dice: None,
    }
}

// --- U-4 second half (B3, B14-B20) ----------------------------------------

#[test]
fn unknown_kinds_validate_once_each() {
    let base = campaign("combat_basic");
    let hud = |section: Value| json!({"screens": [{"kind": "combat_hud", "sections": [section]}]});
    let cases: Vec<(Value, &str, &str)> = vec![
        (
            json!({"screens": [{"kind": "journal", "sections": []}]}),
            "/screens/0/kind",
            "unknown screen kind \"journal\"",
        ),
        (
            json!({"pools": [{"pool": POOL, "label": L, "style": "dial"}]}),
            "/pools/0/style",
            "unknown pool style \"dial\"",
        ),
        (
            json!({"abilities": [{"ability": ABILITY, "label": L, "target_hint": "self"}]}),
            "/abilities/0/target_hint",
            "unknown target hint \"self\"",
        ),
        (
            json!({"screens": [{"kind": "action_bar", "sections": [
                {"type": "ability_list",
                 "value": {"select": {"type": "all"}, "sort": "newest"}}]}]}),
            "/screens/0/sections/0/value/sort",
            "unknown sort \"newest\"",
        ),
        (
            json!({"stats": [{"stat": "might", "label": L,
                "format": {"type": "money", "value": {"a": [1, 2]}}}]}),
            "/stats/0/format/type",
            "unknown format \"money\"",
        ),
        (
            hud(json!({"type": "carousel", "value": {"speed": 3}})),
            "/screens/0/sections/0/type",
            "unknown section type \"carousel\"",
        ),
        (
            hud(json!({"type": "marquee"})),
            "/screens/0/sections/0/type",
            "unknown section type \"marquee\"",
        ),
    ];
    for (fields, pointer, message) in cases {
        let found = run_one(&base, &ui_with(fields));
        assert_eq!(
            found,
            [err(
                "ui/x.json",
                pointer,
                DiagnosticCode::UnknownKind,
                message,
                FIX_KIND
            )],
            "{pointer}"
        );
    }
    // Control: the known spellings give no finding.
    let known = ui_with(json!({
        "pools": [{"pool": POOL, "label": L, "style": "pips"}],
        "abilities": [{"ability": ABILITY, "label": L, "target_hint": "enemy"}],
        "stats": [{"stat": "might", "label": L, "format": {"type": "percent"}}],
        "screens": [{"kind": "action_bar", "sections": [
            {"type": "ability_list", "value": {"select": {"type": "all"}, "sort": "label"}}]}]
    }));
    assert_eq!(run_one(&base, &known), Vec::new());
    // The kitchen-sink unknowns in one file give one `unknown_kind` each at the
    // contract pointers (plus the findings its other entries earn).
    let kinds: Vec<_> = run_one(&base, &unknown_doc())
        .into_iter()
        .filter(|d| d.code == DiagnosticCode::UnknownKind)
        .map(|d| d.pointer)
        .collect();
    assert_eq!(
        kinds,
        [
            "/abilities/0/target_hint",
            "/pools/0/style",
            "/screens/0/kind",
            "/screens/1/sections/0/value/sort",
            "/stats/0/format/type",
        ]
    );
}

// --- U-9 (B10-B12) --------------------------------------------------------

#[test]
fn ruleset_binding() {
    let base = campaign("combat_basic");
    let hero = ulid_of_hero();
    // Undeclared stat, foreign pool and unknown outcome sit in the document, but
    // with no resolving ruleset only the generic `/ruleset` finding appears.
    let body = json!({
        "stats": [{"stat": "ghost", "label": L}],
        "pools": [{"pool": hero, "label": L}],
        "outcomes": [{"outcome": "critical_success", "label": L}],
        "screens": [{"kind": "character_sheet", "sections": [
            {"type": "stat_group", "value": {"type": "names", "value": ["ghost"]}}]}]
    });
    let mut dangling = ui_with(body.clone());
    dangling["ruleset"] = json!(MISSING);
    assert_eq!(
        run_one(&base, &dangling),
        [err(
            "ui/x.json",
            "/ruleset",
            DiagnosticCode::DanglingReference,
            &format!("dangling ui ruleset reference to {MISSING}"),
            FIX_DANGLING
        )]
    );
    let mut wrong = ui_with(body.clone());
    wrong["ruleset"] = json!(hero);
    assert_eq!(
        run_one(&base, &wrong),
        [err(
            "ui/x.json",
            "/ruleset",
            DiagnosticCode::WrongReferenceKind,
            &format!(
                "wrong kind for ui ruleset reference to {hero}: found creature, expected ruleset"
            ),
            FIX_WRONG
        )]
    );
    // Control: the same body under the real ruleset does get its (R) findings.
    let found = run_one(&base, &ui_with(body));
    let codes: Vec<_> = found.iter().map(|d| (d.pointer.as_str(), d.code)).collect();
    assert_eq!(
        codes,
        [
            ("/outcomes/0/outcome", DiagnosticCode::UnknownOutcome),
            ("/pools/0/pool", DiagnosticCode::UnknownPool),
            (
                "/screens/0/sections/0/value/value/0",
                DiagnosticCode::UnknownStat
            ),
            ("/stats/0/stat", DiagnosticCode::UnknownStat),
        ]
    );

    // Two UI files for one ruleset: the later path duplicates the earlier.
    let one = ui_with(json!({}));
    assert_eq!(run(&base, &[("ui/a.json", &one)]), Vec::new());
    assert_eq!(
        run(&base, &[("ui/b.json", &one), ("ui/a.json", &one)]),
        [err(
            "ui/b.json",
            "/ruleset",
            DiagnosticCode::DuplicateBinding,
            &format!("duplicate ui for ruleset {RULESET} (first at ui/a.json/ruleset)"),
            FIX_DUP
        )]
    );
    // A third file still points at the first.
    let three = run(
        &base,
        &[
            ("ui/a.json", &one),
            ("ui/b.json", &one),
            ("ui/c.json", &one),
        ],
    );
    assert_eq!(three.len(), 2);
    assert_eq!(three[1].file, Some(sp("ui/c.json")));
    assert_eq!(
        three[1].message,
        format!("duplicate ui for ruleset {RULESET} (first at ui/a.json/ruleset)")
    );
    // Two files for an unresolved ruleset are two generic findings, no duplicate.
    let found = run(&base, &[("ui/a.json", &dangling), ("ui/b.json", &dangling)]);
    assert_eq!(
        found.iter().map(|d| d.code).collect::<Vec<_>>(),
        [DiagnosticCode::DanglingReference; 2]
    );
}

// --- U-10 (B13, B15-B17, B24) ---------------------------------------------

#[test]
fn entry_resolution() {
    let hero = ulid_of_hero();
    let mut base = campaign("combat_basic");
    let mut extra = ability_clone(&base.documents, "abilities/strike.json");
    extra.id = ulid(EXTRA);
    add_ability(&mut base.documents, "abilities/extra.json", extra);

    let ui = ui_with(json!({
        "stats": [{"stat": "ghost", "label": L}, {"stat": "might", "label": L},
                  {"stat": "might", "label": L}],
        "pools": [{"pool": hero, "label": L}, {"pool": POOL, "label": L},
                  {"pool": POOL, "label": L}, {"pool": MISSING, "label": L}],
        "abilities": [{"ability": MISSING, "label": L}, {"ability": EXTRA, "label": L},
                      {"ability": ABILITY, "label": L}, {"ability": ABILITY, "label": L},
                      {"ability": EXTRA, "label": L}, {"ability": hero, "label": L},
                      {"ability": MISSING, "label": L}],
        "effects": [{"effect": MISSING, "label": L}, {"effect": hero, "label": L}]
    }));
    let f = "ui/x.json";
    let dup = |what: &str, first: &str| format!("duplicate {what} (first at ui/x.json{first})");
    let dangling = |what: &str| format!("dangling ui {what} reference to {MISSING}");
    let wrong = |what: &str| {
        format!("wrong kind for ui {what} reference to {hero}: found creature, expected {what}")
    };
    let no_pool =
        |id: &str| format!("unknown pool {id} for ui: not a template of ruleset {RULESET}");
    let expected = vec![
        err(
            f,
            "/abilities/0/ability",
            DiagnosticCode::DanglingReference,
            &dangling("ability"),
            FIX_DANGLING,
        ),
        err(
            f,
            "/abilities/1/ability",
            DiagnosticCode::ForeignReference,
            &format!("ui ability {EXTRA} is not listed by ruleset {RULESET}"),
            FIX_ABILITY,
        ),
        err(
            f,
            "/abilities/3/ability",
            DiagnosticCode::DuplicateBinding,
            &dup(&format!("ability {ABILITY}"), "/abilities/2/ability"),
            FIX_DUP,
        ),
        err(
            f,
            "/abilities/4/ability",
            DiagnosticCode::DuplicateBinding,
            &dup(&format!("ability {EXTRA}"), "/abilities/1/ability"),
            FIX_DUP,
        ),
        err(
            f,
            "/abilities/5/ability",
            DiagnosticCode::WrongReferenceKind,
            &wrong("ability"),
            FIX_WRONG,
        ),
        err(
            f,
            "/abilities/6/ability",
            DiagnosticCode::DanglingReference,
            &dangling("ability"),
            FIX_DANGLING,
        ),
        err(
            f,
            "/effects/0/effect",
            DiagnosticCode::DanglingReference,
            &dangling("effect"),
            FIX_DANGLING,
        ),
        err(
            f,
            "/effects/1/effect",
            DiagnosticCode::WrongReferenceKind,
            &wrong("effect"),
            FIX_WRONG,
        ),
        err(
            f,
            "/pools/0/pool",
            DiagnosticCode::UnknownPool,
            &no_pool(&hero),
            FIX_POOL,
        ),
        err(
            f,
            "/pools/2/pool",
            DiagnosticCode::DuplicateBinding,
            &dup(&format!("pool {POOL}"), "/pools/1/pool"),
            FIX_DUP,
        ),
        err(
            f,
            "/pools/3/pool",
            DiagnosticCode::UnknownPool,
            &no_pool(MISSING),
            FIX_POOL,
        ),
        err(
            f,
            "/stats/0/stat",
            DiagnosticCode::UnknownStat,
            &format!("unknown stat \"ghost\" for ui: not declared by ruleset {RULESET}"),
            FIX_STAT,
        ),
        err(
            f,
            "/stats/2/stat",
            DiagnosticCode::DuplicateBinding,
            &dup("stat \"might\"", "/stats/1/stat"),
            FIX_DUP,
        ),
    ];
    assert_eq!(run_one(&base, &ui), expected);

    // Control: the valid ids alone give nothing.
    let valid = ui_with(json!({
        "stats": [{"stat": "might", "label": L}],
        "pools": [{"pool": POOL, "label": L}],
        "abilities": [{"ability": ABILITY, "label": L}]
    }));
    assert_eq!(run_one(&base, &valid), Vec::new());

    // Effects, on combat_srd: a second effect no listed ability applies.
    let mut srd = campaign("combat_srd");
    let mut other = effect_clone(&srd.documents, "effects/focusing.json");
    other.id = ulid(EXTRA);
    other.slug = "extra".into();
    srd.documents
        .insert(sp("effects/extra.json"), Document::Effect(other));
    let entry = |id: &str| json!({"effect": id, "label": "fixture.focusing"});
    let ui = ui_with(json!({"ruleset": SRD, "effects": [
        entry(SRD_EFFECT), entry(EXTRA), entry(GONE), entry(SRD_EFFECT), entry(EXTRA)]}));
    let expected = vec![
        err(
            f,
            "/effects/1/effect",
            DiagnosticCode::ForeignReference,
            &format!("ui effect {EXTRA} is applied by no ability of ruleset {SRD}"),
            FIX_EFFECT,
        ),
        err(
            f,
            "/effects/2/effect",
            DiagnosticCode::DanglingReference,
            &format!("dangling ui effect reference to {GONE}"),
            FIX_DANGLING,
        ),
        err(
            f,
            "/effects/3/effect",
            DiagnosticCode::DuplicateBinding,
            &dup(&format!("effect {SRD_EFFECT}"), "/effects/0/effect"),
            FIX_DUP,
        ),
        err(
            f,
            "/effects/4/effect",
            DiagnosticCode::DuplicateBinding,
            &dup(&format!("effect {EXTRA}"), "/effects/1/effect"),
            FIX_DUP,
        ),
    ];
    assert_eq!(run_one(&srd, &ui), expected);
    // Pools of the second ruleset resolve against its own templates.
    let pool = |id: &str| json!({"pool": id, "label": "fixture.heavy"});
    let pools = ui_with(json!({"ruleset": SRD, "pools": [
        pool(SRD_POOL1), pool(SRD_POOL2), pool(POOL)]}));
    assert_eq!(
        run_one(&srd, &pools),
        [err(
            f,
            "/pools/2/pool",
            DiagnosticCode::UnknownPool,
            &format!("unknown pool {POOL} for ui: not a template of ruleset {SRD}"),
            FIX_POOL
        )]
    );
}

// --- U-11 (B18, B26) ------------------------------------------------------

#[test]
fn outcome_resolution() {
    let f = "ui/x.json";
    let srd = campaign("combat_srd");
    let labels = [
        "success",
        "failure",
        "critical_success",
        "critical_failure",
        "custom:0",
        "custom:",
        "custom:256",
        "custom:01",
        "Success",
        "success",
    ];
    let outcomes: Vec<Value> = labels
        .iter()
        .map(|o| json!({"outcome": o, "label": "fixture.heavy"}))
        .collect();
    let ui = ui_with(json!({"ruleset": SRD, "outcomes": outcomes}));
    let mut expected: Vec<Diagnostic> = (3..9)
        .map(|i| {
            err(
                f,
                &format!("/outcomes/{i}/outcome"),
                DiagnosticCode::UnknownOutcome,
                &format!("outcome {:?} is not produced by ruleset {SRD}", labels[i]),
                "use an outcome the ruleset's tables or damage entries produce",
            )
        })
        .collect();
    expected.push(err(
        f,
        "/outcomes/9/outcome",
        DiagnosticCode::DuplicateBinding,
        "duplicate outcome \"success\" (first at ui/x.json/outcomes/0/outcome)",
        FIX_DUP,
    ));
    assert_eq!(run_one(&srd, &ui), expected);

    // The label set follows the data: `combat_basic` produces no critical success.
    let mut basic = campaign("combat_basic");
    let ui = ui_with(json!({"outcomes": [
        {"outcome": "critical_success", "label": L}, {"outcome": "custom:3", "label": L},
        {"outcome": "failure", "label": L}]}));
    let found = run_one(&basic, &ui);
    assert_eq!(
        found.iter().map(|d| d.pointer.as_str()).collect::<Vec<_>>(),
        ["/outcomes/0/outcome", "/outcomes/1/outcome"]
    );
    // After a table band is changed to custom 3, that label resolves; the
    // damage entries still produce `failure`.
    for document in basic.documents.values_mut() {
        if let Document::OutcomeTable(table) = document {
            table.bands[1].outcome = OutcomeWire::Custom(3);
        }
    }
    let found = run_one(&basic, &ui);
    assert_eq!(
        found.iter().map(|d| d.pointer.as_str()).collect::<Vec<_>>(),
        ["/outcomes/0/outcome"]
    );
}

// --- U-12 (B20) -----------------------------------------------------------

#[test]
fn tags_and_groups() {
    let f = "ui/x.json";
    let mut base = campaign("combat_basic");
    let ui = ui_with(json!({
        "stats": [{"stat": "might", "label": L, "group": "fixture.hero"}],
        "abilities": [{"ability": ABILITY, "label": L, "category_tags": ["melee"]}],
        "screens": [
            {"kind": "character_sheet", "sections": [
                {"type": "stat_group", "value": {"type": "group", "value": "fixture.hero"}},
                {"type": "stat_group", "value": {"type": "group", "value": "fixture.goblin"}}]},
            {"kind": "action_bar", "sections": [
                {"type": "ability_list", "value": {"select": {"type": "tag", "value": "melee"}}},
                {"type": "ability_list", "value": {"select": {"type": "tag", "value": "ranged"}}}]},
            {"kind": "inventory", "sections": [
                {"type": "item_list", "value": {"type": "tag", "value": "potion"}}]}]
    }));
    let expected = vec![
        err(
            f,
            "/screens/0/sections/1/value/value",
            DiagnosticCode::UnknownGroup,
            "stat group \"fixture.goblin\" is the group of no stats entry",
            "set the group on a stats entry or fix the selector",
        ),
        err(
            f,
            "/screens/1/sections/1/value/select/value",
            DiagnosticCode::UnknownTag,
            "ability tag \"ranged\" is declared by no abilities entry",
            "add the tag to an abilities entry or fix the filter",
        ),
        finding(
            f,
            "/screens/2/sections/0/value/value",
            Severity::Warning,
            DiagnosticCode::UnknownTag,
            "item tag \"potion\" is carried by no item in this campaign",
            "add the tag to an item or fix the filter",
        ),
    ];
    let found = run_one(&base, &ui);
    assert_eq!(found, expected);
    // The item finding is a warning: only the two others are errors.
    assert_eq!(
        found
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count(),
        2
    );

    // An item carrying the tag clears the warning only.
    base.documents.insert(
        sp("items/potion.json"),
        Document::Item(Item {
            id: ulid(EXTRA),
            slug: "potion".into(),
            name: L.into(),
            note: None,
            stats: BTreeMap::new(),
            tags: vec!["other".into(), "potion".into()],
        }),
    );
    let found = run_one(&base, &ui);
    assert_eq!(found, expected[..2]);
    // Without any `stats` entry nothing declares the group.
    let bare = ui_with(json!({"screens": [{"kind": "character_sheet", "sections": [
        {"type": "stat_group", "value": {"type": "group", "value": "fixture.hero"}}]}]}));
    assert_eq!(
        run_one(&base, &bare)
            .iter()
            .map(|d| d.code)
            .collect::<Vec<_>>(),
        [DiagnosticCode::UnknownGroup]
    );
}
// --- U-13 (B14) -----------------------------------------------------------

fn matrix_base() -> LoadedCampaign {
    let mut base = campaign("combat_basic");
    let ruleset = ruleset_mut(&mut base);
    for (name, kind) in [
        ("s_fixed", StatKindWire::Fixed),
        ("s_bool", StatKindWire::Bool),
        (
            "s_enum",
            StatKindWire::Enum {
                enum_id: "e".into(),
                variants: vec!["a".into(), "b".into()],
            },
        ),
        ("s_tags", StatKindWire::Tags),
        ("s_dice", StatKindWire::Dice),
    ] {
        ruleset.stats.push(StatDecl {
            name: name.into(),
            kind,
            derived: None,
        });
    }
    base
}

#[test]
fn format_matrix() {
    let base = matrix_base();
    let f = "ui/x.json";
    let formats: [(&str, Value); 8] = [
        ("integer", json!({"type": "integer"})),
        ("signed", json!({"type": "signed"})),
        (
            "fraction",
            json!({"type": "fraction", "value": {"type": "stat", "value": "might"}}),
        ),
        ("percent", json!({"type": "percent"})),
        ("boolean", json!({"type": "boolean"})),
        (
            "enum",
            json!({"type": "enum", "value": [{"variant": "a", "label": L}]}),
        ),
        ("tags", json!({"type": "tags"})),
        ("dice", json!({"type": "dice"})),
    ];
    let stats = [
        ("might", "int"),
        ("s_fixed", "fixed"),
        ("s_bool", "bool"),
        ("s_enum", "enum"),
        ("s_tags", "tags"),
        ("s_dice", "dice"),
    ];
    let mut cells = 0;
    for (stat, kind) in stats {
        for (name, format) in &formats {
            let allowed = match *name {
                "integer" | "signed" | "fraction" | "percent" => kind == "int" || kind == "fixed",
                "boolean" => kind == "bool",
                "enum" => kind == "enum",
                "tags" => kind == "tags",
                _ => kind == "dice",
            };
            let ui = ui_with(json!({"stats": [{"stat": stat, "label": L, "format": format}]}));
            let found = run_one(&base, &ui);
            if allowed {
                assert_eq!(found, Vec::new(), "{name} on {kind}");
            } else {
                assert_eq!(
                    found,
                    [err(
                        f,
                        "/stats/0/format",
                        DiagnosticCode::FormatMismatch,
                        &format!("format {name} does not suit {kind} stat {stat:?}"),
                        FIX_MISMATCH
                    )],
                    "{name} on {kind}"
                );
            }
            cells += 1;
        }
    }
    assert_eq!(cells, 48);

    // Meters.
    let meter = |stat: &str, meter: Value| {
        ui_with(json!({"stats": [{"stat": stat, "label": L,
            "format": {"type": "fraction", "value": meter}}]}))
    };
    let at = "/stats/0/format/value/value";
    let not_meter = |name: &str, kind: &str| {
        format!("fraction meter stat {name:?} is {kind}, not int or fixed")
    };
    assert_eq!(
        run_one(
            &base,
            &meter("might", json!({"type": "stat", "value": "s_bool"}))
        ),
        [err(
            f,
            at,
            DiagnosticCode::FormatMismatch,
            &not_meter("s_bool", "bool"),
            FIX_MISMATCH
        )]
    );
    assert_eq!(
        run_one(
            &base,
            &meter("might", json!({"type": "stat", "value": "s_enum"}))
        ),
        [err(
            f,
            at,
            DiagnosticCode::FormatMismatch,
            &not_meter("s_enum", "enum"),
            FIX_MISMATCH
        )]
    );
    let unknown_ghost = format!("unknown stat \"ghost\" for ui: not declared by ruleset {RULESET}");
    assert_eq!(
        run_one(
            &base,
            &meter("might", json!({"type": "stat", "value": "ghost"}))
        ),
        [err(
            f,
            at,
            DiagnosticCode::UnknownStat,
            &unknown_ghost,
            FIX_STAT
        )]
    );
    let hero = ulid_of_hero();
    assert_eq!(
        run_one(
            &base,
            &meter("might", json!({"type": "pool", "value": hero}))
        ),
        [err(
            f,
            at,
            DiagnosticCode::UnknownPool,
            &format!("unknown pool {hero} for ui: not a template of ruleset {RULESET}"),
            FIX_POOL
        )]
    );
    // Controls: a fixed stat and a template are fine meters.
    for ok in [
        json!({"type": "stat", "value": "s_fixed"}),
        json!({"type": "pool", "value": POOL}),
    ] {
        assert_eq!(run_one(&base, &meter("might", ok)), Vec::new());
    }
    // A bad format and a bad meter are two findings at two pointers.
    let found = run_one(
        &base,
        &meter("s_bool", json!({"type": "stat", "value": "ghost"})),
    );
    assert_eq!(
        found
            .iter()
            .map(|d| (d.pointer.as_str(), d.code))
            .collect::<Vec<_>>(),
        [
            ("/stats/0/format", DiagnosticCode::FormatMismatch),
            (at, DiagnosticCode::UnknownStat)
        ]
    );

    // Enum labels: undeclared and repeated variants.
    let labels = |stat: &str, variants: &[&str]| {
        let labels: Vec<Value> = variants
            .iter()
            .map(|v| json!({"variant": v, "label": L}))
            .collect();
        ui_with(json!({"stats": [{"stat": stat, "label": L,
            "format": {"type": "enum", "value": labels}}]}))
    };
    let repeat = "duplicate variant \"a\" (first at ui/x.json/stats/0/format/value/0/variant)";
    assert_eq!(
        run_one(&base, &labels("s_enum", &["a", "zzz", "a", "b"])),
        [
            err(
                f,
                "/stats/0/format/value/1/variant",
                DiagnosticCode::UnknownVariant,
                "variant \"zzz\" is not declared by enum stat \"s_enum\"",
                "use a variant of the stat's enum"
            ),
            err(
                f,
                "/stats/0/format/value/2/variant",
                DiagnosticCode::DuplicateBinding,
                repeat,
                FIX_DUP
            ),
        ]
    );
    // On a non-enum stat the format mismatches and repeats still repeat, but no
    // variant is judged against a domain the stat does not have.
    assert_eq!(
        run_one(&base, &labels("might", &["a", "a"])),
        [
            err(
                f,
                "/stats/0/format",
                DiagnosticCode::FormatMismatch,
                "format enum does not suit int stat \"might\"",
                FIX_MISMATCH
            ),
            err(
                f,
                "/stats/0/format/value/1/variant",
                DiagnosticCode::DuplicateBinding,
                repeat,
                FIX_DUP
            ),
        ]
    );
    // An undeclared stat is `unknown_stat` and nothing about its format.
    let ghost = ui_with(json!({"stats": [{"stat": "ghost", "label": L,
        "format": {"type": "boolean"}}]}));
    assert_eq!(
        run_one(&base, &ghost),
        [err(
            f,
            "/stats/0/stat",
            DiagnosticCode::UnknownStat,
            &unknown_ghost,
            FIX_STAT
        )]
    );
}

// --- U-14 (B19-B21) -------------------------------------------------------

#[test]
fn screen_rules() {
    let f = "ui/x.json";
    let base = campaign("combat_basic");
    let ghost = || json!({"type": "stat_group", "value": {"type": "names", "value": ["ghost"]}});
    let pools = |ids: Value| json!({"type": "pool_group", "value": {"type": "ids", "value": ids}});
    let abilities = |ids: Value| json!({"type": "ability_list", "value": {"select": {"type": "ids", "value": ids}}});
    let ui = ui_with(json!({"screens": [
        {"kind": "level_up", "sections": [
            {"type": "choice_steps"}, pools(json!([MISSING])), abilities(json!([MISSING]))]},
        {"kind": "character_creation", "sections": []},
        {"kind": "combat_hud", "sections": [{"type": "choice_steps"}, ghost()]},
        {"kind": "spellbook", "sections": []},
        {"kind": "spellbook", "sections": [ghost()]},
        {"kind": "action_bar", "sections": [
            pools(json!([POOL])), pools(json!([POOL])),
            abilities(json!([ABILITY])), abilities(json!([ABILITY]))]},
        {"kind": "inventory", "sections": [{"type": "timeline"}, pools(json!([POOL]))]},
        {"kind": "character_sheet", "sections": [
            pools(json!([POOL])), {"type": "pool_group", "value": {"type": "all"}},
            {"type": "stat_group", "value": {"type": "all"}},
            {"type": "stat_group", "value": {"type": "names", "value": ["might"]}},
            {"type": "ability_list", "value": {"select": {"type": "all"}}},
            abilities(json!([ABILITY]))]}]}));
    let reserved = |what: &str, text: &str| {
        format!("{what} \"{text}\" is reserved until its data family exists")
    };
    let unknown_ghost = format!("unknown stat \"ghost\" for ui: not declared by ruleset {RULESET}");
    let expected = vec![
        err(f, "/screens/0/kind", DiagnosticCode::ReservedKind,
            &reserved("screen kind", "level_up"), FIX_RESERVED),
        err(f, "/screens/0/sections/2/value/select/value/0", DiagnosticCode::DanglingReference,
            &format!("dangling ui ability reference to {MISSING}"), FIX_DANGLING),
        err(f, "/screens/1/kind", DiagnosticCode::ReservedKind,
            &reserved("screen kind", "character_creation"), FIX_RESERVED),
        err(f, "/screens/2/sections/0/type", DiagnosticCode::ReservedKind,
            &reserved("section type", "choice_steps"), FIX_RESERVED),
        err(f, "/screens/2/sections/1/value/value/0", DiagnosticCode::UnknownStat,
            &unknown_ghost, FIX_STAT),
        err(f, "/screens/4/kind", DiagnosticCode::DuplicateBinding,
            "duplicate screen kind \"spellbook\" (first at ui/x.json/screens/3/kind)", FIX_DUP),
        err(f, "/screens/4/sections/0/value/value/0", DiagnosticCode::UnknownStat,
            &unknown_ghost, FIX_STAT),
        err(f, "/screens/5/sections/1/value/value/0", DiagnosticCode::DuplicateBinding,
            &format!("duplicate pool {POOL} (first at ui/x.json/screens/5/sections/0/value/value/0)"),
            FIX_DUP),
        err(f, "/screens/5/sections/3/value/select/value/0", DiagnosticCode::DuplicateBinding,
            &format!("duplicate ability {ABILITY} (first at ui/x.json/screens/5/sections/2/value/select/value/0)"),
            FIX_DUP),
    ];
    assert_eq!(run_one(&base, &ui), expected);

    // A name repeated inside one section, and a reserved screen repeated.
    let ui = ui_with(json!({"screens": [
        {"kind": "level_up", "sections": []},
        {"kind": "level_up", "sections": []},
        {"kind": "character_sheet", "sections": [
            {"type": "stat_group", "value": {"type": "names", "value": ["might", "might"]}}]}]}));
    assert_eq!(
        run_one(&base, &ui),
        [
            err(
                f,
                "/screens/0/kind",
                DiagnosticCode::ReservedKind,
                &reserved("screen kind", "level_up"),
                FIX_RESERVED
            ),
            err(
                f,
                "/screens/1/kind",
                DiagnosticCode::ReservedKind,
                &reserved("screen kind", "level_up"),
                FIX_RESERVED
            ),
            err(
                f,
                "/screens/2/sections/0/value/value/1",
                DiagnosticCode::DuplicateBinding,
                "duplicate stat \"might\" (first at ui/x.json/screens/2/sections/0/value/value/0)",
                FIX_DUP
            ),
        ]
    );
}

// --- U-15 (B22, B23) ------------------------------------------------------

fn locale_ui() -> Value {
    ui_with(json!({"ruleset": SRD,
        "stats": [
            {"stat": "might", "label": "k.sl", "short_label": "k.ss", "description": "k.sd",
             "group": "k.sg"},
            {"stat": "guile", "label": "k.g",
             "format": {"type": "enum", "value": [{"variant": "a", "label": "k.ev"}]}}],
        "pools": [{"pool": SRD_POOL1, "label": "k.p", "icon": "assets/p.png"}],
        "abilities": [{"ability": SRD_HEAVY, "label": "k.a", "description": "k.ad",
                       "icon": "assets/a.png"}],
        "effects": [{"effect": SRD_EFFECT, "label": "k.e", "icon": "assets/e.png"}],
        "outcomes": [{"outcome": "success", "label": "k.o"}]}))
}

const LOCALE_SITES: [(&str, &str); 11] = [
    ("/abilities/0/description", "k.ad"),
    ("/abilities/0/label", "k.a"),
    ("/effects/0/label", "k.e"),
    ("/outcomes/0/label", "k.o"),
    ("/pools/0/label", "k.p"),
    ("/stats/0/description", "k.sd"),
    ("/stats/0/group", "k.sg"),
    ("/stats/0/label", "k.sl"),
    ("/stats/0/short_label", "k.ss"),
    ("/stats/1/format/value/0/label", "k.ev"),
    ("/stats/1/label", "k.g"),
];

fn only(found: Vec<Diagnostic>, code: DiagnosticCode) -> Vec<Diagnostic> {
    found.into_iter().filter(|d| d.code == code).collect()
}

#[test]
fn locale_and_assets() {
    let f = "ui/x.json";
    let mut base = campaign("combat_srd");
    let ui = locale_ui();
    let missing = |locale: &str, sites: &[(&str, &str)]| -> Vec<Diagnostic> {
        sites
            .iter()
            .map(|(pointer, key)| {
                err(
                    f,
                    pointer,
                    DiagnosticCode::MissingLocaleKey,
                    &format!("locale key {key:?} is missing from locale {locale:?}"),
                    &format!("add {key:?} to locale {locale:?}"),
                )
            })
            .collect()
    };
    // One table that lacks every key: every site, once.
    assert_eq!(
        only(run_one(&base, &ui), DiagnosticCode::MissingLocaleKey),
        missing("en", &LOCALE_SITES)
    );

    // Two tables: the key is reported for the table that lacks it only.
    for locale in locales_mut(&mut base) {
        for (_, key) in LOCALE_SITES {
            locale.strings.insert(key.into(), "x".into());
        }
    }
    let mut fr = locales_mut(&mut base)[0].clone();
    fr.locale = "fr".into();
    fr.strings.remove("k.o");
    fr.strings.remove("k.ev");
    base.documents
        .insert(sp("locale/fr.json"), Document::Locale(fr));
    assert_eq!(
        only(run_one(&base, &ui), DiagnosticCode::MissingLocaleKey),
        missing(
            "fr",
            &[
                ("/outcomes/0/label", "k.o"),
                ("/stats/1/format/value/0/label", "k.ev")
            ]
        )
    );
    // Control: with every key in both tables there is none.
    for locale in locales_mut(&mut base) {
        for (_, key) in LOCALE_SITES {
            locale.strings.insert(key.into(), "x".into());
        }
    }
    assert_eq!(
        only(run_one(&base, &ui), DiagnosticCode::MissingLocaleKey),
        Vec::new()
    );

    // With no table at all every site is reported once, without a key.
    let mut none = base.clone();
    none.documents
        .retain(|_, d| !matches!(d, Document::Locale(_)));
    none.documents.insert(sp(f), read_value(&ui).unwrap());
    let found: Vec<_> = validate(&none)
        .into_iter()
        .filter(|d| d.file == Some(sp(f)) && d.code == DiagnosticCode::MissingLocaleKey)
        .collect();
    assert_eq!(
        found,
        LOCALE_SITES
            .iter()
            .map(|(pointer, _)| err(
                f,
                pointer,
                DiagnosticCode::MissingLocaleKey,
                "locale key has no locale table to resolve against",
                "add a locale table containing the key"
            ))
            .collect::<Vec<_>>()
    );

    // Assets: absent icons are `missing_asset` at their pointers, present ones pass.
    let asset = |pointer: &str, key: &str| {
        err(
            f,
            pointer,
            DiagnosticCode::MissingAsset,
            &format!("asset key {key:?} is not in assets.lock"),
            "add the asset to assets.lock or fix the key",
        )
    };
    assert_eq!(
        only(run_one(&base, &ui), DiagnosticCode::MissingAsset),
        [
            asset("/abilities/0/icon", "assets/a.png"),
            asset("/effects/0/icon", "assets/e.png"),
            asset("/pools/0/icon", "assets/p.png"),
        ]
    );
    for document in base.documents.values_mut() {
        if let Document::AssetsLock(lock) = document {
            for (path, seed) in [("assets/a.png", 3), ("assets/p.png", 4)] {
                lock.assets.insert(
                    sp(path),
                    AssetRecord {
                        hash: Digest::from_bytes([seed; 32]),
                        import: BTreeMap::new(),
                    },
                );
            }
        }
    }
    assert_eq!(
        only(run_one(&base, &ui), DiagnosticCode::MissingAsset),
        [asset("/effects/0/icon", "assets/e.png")]
    );
}

// --- U-16 (B24) -----------------------------------------------------------

#[test]
fn order_and_determinism() {
    let ui = defect_ui();
    let base = fixture("combat_basic");
    let build = |names: &[&str]| {
        let mut files = base.clone();
        for name in names {
            files.insert(sp(name), bytes(&ui));
        }
        files
    };
    let one = validate_files(&build(&["ui/a.json", "ui/b.json"]), &engine());
    let two = validate_files(&build(&["ui/b.json", "ui/a.json"]), &engine());
    let again = validate_files(&build(&["ui/a.json", "ui/b.json"]), &engine());
    assert_eq!(one, two);
    assert_eq!(one, again);
    // Sorted by file, pointer, code, severity, message, fix.
    let mut sorted = one.clone();
    sorted.sort_by_key(|d| {
        (
            d.file.clone(),
            d.pointer.clone(),
            d.code.as_str(),
            d.severity,
            d.message.clone(),
            d.suggested_fix.clone(),
        )
    });
    assert_eq!(sorted, one);
    // The result is not vacuous.
    let codes: std::collections::BTreeSet<_> = one.iter().map(|d| d.code.as_str()).collect();
    for code in [
        "duplicate_binding",
        "unknown_kind",
        "reserved_kind",
        "unknown_tag",
        "unknown_group",
        "format_mismatch",
        "unknown_outcome",
        "unknown_stat",
        "unknown_pool",
        "missing_asset",
        "missing_locale_key",
    ] {
        assert!(codes.contains(code), "{code} missing from {codes:?}");
    }
    let dup_at = |file: &str| {
        one.iter().any(|d| {
            d.file == Some(sp(file))
                && d.pointer == "/ruleset"
                && d.code == DiagnosticCode::DuplicateBinding
        })
    };
    assert!(dup_at("ui/b.json"));
    assert!(!dup_at("ui/a.json"));
}

// --- U-19 (B31) -----------------------------------------------------------

fn add_locale_keys(campaign: &mut LoadedCampaign, keys: &[&str]) {
    for locale in locales_mut(campaign) {
        for key in keys {
            locale.strings.insert((*key).into(), "x".into());
        }
    }
}

fn missing_en(file: &str, sites: &[(&str, &str)]) -> Vec<Diagnostic> {
    sites
        .iter()
        .map(|(pointer, key)| {
            err(
                file,
                pointer,
                DiagnosticCode::MissingLocaleKey,
                &format!("locale key {key:?} is missing from locale \"en\""),
                &format!("add {key:?} to locale \"en\""),
            )
        })
        .collect()
}

#[test]
fn defaults_validate() {
    let f = "ui/default.json";
    type Case = (
        &'static str,
        &'static str,
        Vec<(&'static str, &'static str)>,
    );
    let cases: [Case; 2] = [
        (
            "combat_basic",
            RULESET,
            vec![
                ("/outcomes/0/label", "success"),
                ("/outcomes/1/label", "failure"),
                ("/pools/0/label", "pool-1"),
                ("/stats/0/label", "might"),
                ("/stats/1/label", "guile"),
                ("/stats/2/label", "resolve"),
                ("/stats/3/label", "health"),
            ],
        ),
        (
            "combat_srd",
            SRD,
            vec![
                ("/outcomes/0/label", "failure"),
                ("/outcomes/1/label", "success"),
                ("/outcomes/2/label", "critical_success"),
                ("/pools/0/label", "pool-1"),
                ("/pools/1/label", "pool-2"),
                ("/stats/0/label", "might"),
                ("/stats/1/label", "guile"),
                ("/stats/2/label", "resolve"),
                ("/stats/3/label", "ward"),
                ("/stats/4/label", "health"),
            ],
        ),
    ];
    for (name, ruleset, sites) in cases {
        let mut base = campaign(name);
        let generated = {
            let source = UiSource::from_documents(&base.documents, ulid(ruleset)).unwrap();
            generated_ui(&source)
        };
        // It loads as a file of the campaign.
        let mut files = fixture(name);
        files.insert(
            sp(f),
            write_document(&Document::Ui(generated.clone())).unwrap(),
        );
        load_campaign(&files, &engine()).unwrap();
        let ui = serde_json::from_slice::<Value>(&files[&sp(f)]).unwrap();
        // Only the stat, pool and outcome labels are missing, never ability or
        // effect labels (they reuse the objects' own `name` keys).
        assert_eq!(run(&base, &[(f, &ui)]), missing_en(f, &sites), "{name}");
        // With those keys in the table the generated file is clean.
        let keys: Vec<&str> = sites.iter().map(|(_, key)| *key).collect();
        add_locale_keys(&mut base, &keys);
        assert_eq!(run(&base, &[(f, &ui)]), Vec::new(), "{name}");
    }

    // An effective document is clean in the same sense.
    let mut srd = campaign("combat_srd");
    let authored = read_ui(&ui_with(json!({"ruleset": SRD,
        "stats": [{"stat": "health", "label": "fixture.hero"}],
        "pools": [{"pool": SRD_POOL2, "label": "fixture.hero"}],
        "screens": [{"kind": "action_bar", "sections": [{"type": "timeline"}]}]})));
    let effective = {
        let source = UiSource::from_documents(&srd.documents, ulid(SRD)).unwrap();
        effective_ui(Some(&authored), &source)
    };
    let ui = serde_json::from_slice::<Value>(&write_document(&Document::Ui(effective)).unwrap())
        .unwrap();
    let sites = [
        ("/outcomes/0/label", "failure"),
        ("/outcomes/1/label", "success"),
        ("/outcomes/2/label", "critical_success"),
        ("/pools/1/label", "pool-1"),
        ("/stats/1/label", "might"),
        ("/stats/2/label", "guile"),
        ("/stats/3/label", "resolve"),
        ("/stats/4/label", "ward"),
    ];
    assert_eq!(run(&srd, &[(f, &ui)]), missing_en(f, &sites));
    let keys: Vec<&str> = sites.iter().map(|(_, key)| *key).collect();
    add_locale_keys(&mut srd, &keys);
    assert_eq!(run(&srd, &[(f, &ui)]), Vec::new());
}
// ---------------------------------------------------------------------------
// Review-added gap tests for T142a behaviour (B1, B3, B5, B25, B26, B29, B30).

fn srd_docs() -> BTreeMap<SourcePath, Document> {
    campaign("combat_srd").documents
}

fn ruleset_in(docs: &mut BTreeMap<SourcePath, Document>) -> &mut Ruleset {
    docs.values_mut()
        .find_map(|d| match d {
            Document::Ruleset(r) => Some(r),
            _ => None,
        })
        .unwrap()
}

fn table_in<'a>(docs: &'a mut BTreeMap<SourcePath, Document>, path: &str) -> &'a mut OutcomeTable {
    match docs.get_mut(&sp(path)).unwrap() {
        Document::OutcomeTable(t) => t,
        other => panic!("{other:?}"),
    }
}

#[test]
fn outcome_label_order_bands_overrides_damage() {
    // B26: per ability, table bands, then `Override` naturals, then damage, with
    // first occurrence kept. Damage is not a subset of the bands here, and the
    // two loops share labels, so deleting or reordering either loop moves a label.
    let mut docs = srd_docs();
    table_in(&mut docs, "outcome_tables/heavy_table.json").natural_rules = vec![
        NaturalRuleWire {
            face: 6,
            effect: NaturalEffectWire::Override(OutcomeWire::Custom(7)),
        },
        NaturalRuleWire {
            face: 5,
            effect: NaturalEffectWire::Shift(1),
        },
        NaturalRuleWire {
            face: 4,
            effect: NaturalEffectWire::Override(OutcomeWire::CriticalFailure),
        },
    ];
    ability_of(&mut docs, "abilities/heavy.json").damage = vec![
        damage(OutcomeWire::Custom(9)),
        damage(OutcomeWire::CriticalFailure),
        damage(OutcomeWire::Failure),
        damage(OutcomeWire::Custom(7)),
    ];
    ability_of(&mut docs, "abilities/focus.json").damage = vec![
        damage(OutcomeWire::Success),
        damage(OutcomeWire::Failure),
        damage(OutcomeWire::Custom(1)),
    ];
    // A third listed ability whose table does not resolve still contributes damage.
    let mut third = ability_clone(&docs, "abilities/heavy.json");
    third.id = ulid("0000000000000000000000002A");
    third.outcome_table = ulid(GONE);
    third.damage = vec![damage(OutcomeWire::Custom(2)), damage(OutcomeWire::Success)];
    add_ability(&mut docs, "abilities/third.json", third);
    ruleset_in(&mut docs)
        .abilities
        .push(ulid("0000000000000000000000002A"));

    let source = UiSource::from_documents(&docs, ulid(SRD)).unwrap();
    let expected = [
        "failure",
        "success",
        "critical_success",
        "custom:7",
        "critical_failure",
        "custom:9",
        "custom:1",
        "custom:2",
    ];
    assert_eq!(source.outcome_labels(), expected);
    let generated = generated_ui(&source);
    assert_eq!(
        generated
            .outcomes
            .unwrap()
            .iter()
            .map(|o| (o.outcome.as_str(), o.label.as_str()))
            .collect::<Vec<_>>(),
        expected.map(|text| (text, text))
    );
}

#[test]
fn default_target_hint_follows_the_ability_flags() {
    // B29: no target needed is `none` whatever self-targeting says.
    let mut docs = srd_docs();
    let combos = [
        ("2A", false, false, UiTargetHint::NoTarget),
        ("2B", false, true, UiTargetHint::NoTarget),
        ("2C", true, true, UiTargetHint::Any),
        ("2D", true, false, UiTargetHint::Enemy),
    ];
    let mut listed = Vec::new();
    for (suffix, requires, allows, _) in &combos {
        let mut ability = ability_clone(&docs, "abilities/heavy.json");
        ability.id = ulid(&format!("000000000000000000000000{suffix}"));
        ability.requires_target = *requires;
        ability.allow_self_target = *allows;
        listed.push(ability.id);
        add_ability(&mut docs, &format!("abilities/t{suffix}.json"), ability);
    }
    ruleset_in(&mut docs).abilities = listed;
    let source = UiSource::from_documents(&docs, ulid(SRD)).unwrap();
    let hints: Vec<_> = generated_ui(&source)
        .abilities
        .unwrap()
        .into_iter()
        .map(|a| a.target_hint)
        .collect();
    assert_eq!(hints, combos.map(|(_, _, _, hint)| Some(hint)).to_vec());
    assert_eq!(UiTargetHint::NoTarget.as_str(), "none");
}

#[test]
fn from_documents_resolution() {
    let hid = |text: &str| ulid(&format!("000000000000000000000000{text}"));

    // Unresolved listed ids are skipped; the first document with an id wins.
    let mut docs = srd_docs();
    ruleset_in(&mut docs).abilities =
        vec![ulid(MISSING), ulid(SRD_HEAVY), ulid(SRD_FOCUS), ulid(GONE)];
    let mut early_focus = ability_clone(&docs, "abilities/focus.json");
    early_focus.name = "first.focus".into();
    add_ability(&mut docs, "abilities/aaa.json", early_focus);
    let mut late_heavy = ability_clone(&docs, "abilities/heavy.json");
    late_heavy.name = "last.heavy".into();
    add_ability(&mut docs, "abilities/zzz.json", late_heavy);
    let mut early_ruleset = ruleset_in(&mut docs).clone();
    early_ruleset.name = "first.ruleset".into();
    early_ruleset.slug = "aaa".into();
    docs.insert(sp("rulesets/aaa.json"), Document::Ruleset(early_ruleset));
    let mut early_effect = effect_clone(&docs, "effects/focusing.json");
    early_effect.name = "first.effect".into();
    early_effect.slug = "aaa".into();
    docs.insert(sp("effects/aaa.json"), Document::Effect(early_effect));
    let mut early_table = table_in(&mut docs, "outcome_tables/focus_table.json").clone();
    early_table.name = "first.table".into();
    early_table.slug = "aaa".into();
    docs.insert(
        sp("outcome_tables/aaa.json"),
        Document::OutcomeTable(early_table),
    );

    let source = UiSource::from_documents(&docs, ulid(SRD)).unwrap();
    assert_eq!(source.ruleset.name, "first.ruleset");
    assert_eq!(
        source
            .abilities
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        ["fixture.heavy", "first.focus"]
    );
    assert_eq!(
        source
            .effects
            .iter()
            .map(|e| (e.id, e.name.as_str()))
            .collect::<Vec<_>>(),
        [(ulid(SRD_EFFECT), "first.effect")]
    );
    assert_eq!(
        source
            .tables
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        ["fixture.heavy-table", "first.table"]
    );

    // Effects and tables are kept on first use, in use order, not id order.
    let mut docs = srd_docs();
    let mut second = effect_clone(&docs, "effects/focusing.json");
    second.id = hid("2B");
    second.slug = "second".into();
    second.name = "second.effect".into();
    docs.insert(sp("effects/second.json"), Document::Effect(second));
    ability_of(&mut docs, "abilities/focus.json").effect = Some(hid("2B"));
    ability_of(&mut docs, "abilities/heavy.json").effect = Some(ulid(SRD_EFFECT));
    for (suffix, effect, table) in [
        ("2A", hid("2B"), ulid("0000000000000000000000001A")),
        ("2D", ulid(GONE), ulid(GONE)),
    ] {
        let mut ability = ability_clone(&docs, "abilities/heavy.json");
        ability.id = hid(suffix);
        ability.effect = Some(effect);
        ability.outcome_table = table;
        add_ability(&mut docs, &format!("abilities/t{suffix}.json"), ability);
    }
    ruleset_in(&mut docs).abilities = vec![ulid(SRD_FOCUS), ulid(SRD_HEAVY), hid("2A"), hid("2D")];
    let source = UiSource::from_documents(&docs, ulid(SRD)).unwrap();
    assert_eq!(
        source.abilities.iter().map(|a| a.id).collect::<Vec<_>>(),
        [ulid(SRD_FOCUS), ulid(SRD_HEAVY), hid("2A"), hid("2D")]
    );
    assert_eq!(
        source.effects.iter().map(|e| e.id).collect::<Vec<_>>(),
        [hid("2B"), ulid(SRD_EFFECT)]
    );
    assert_eq!(
        source.tables.iter().map(|t| t.id).collect::<Vec<_>>(),
        [
            ulid("0000000000000000000000001A"),
            ulid("00000000000000000000000019")
        ]
    );

    // A creature id, a missing id and a campaign without rulesets give nothing.
    assert!(UiSource::from_documents(&docs, ulid(&ulid_of_hero())).is_none());
    assert!(UiSource::from_documents(&docs, ulid(MISSING)).is_none());
    assert!(UiSource::from_documents(&BTreeMap::new(), ulid(SRD)).is_none());
}

#[test]
fn unit_formats_choice_steps_and_declared_sort_round_trip() {
    // B1/B3: each unit spelling decodes to its own variant and writes back.
    let unit = ["integer", "signed", "percent", "boolean", "tags", "dice"];
    let stats: Vec<Value> = unit
        .iter()
        .map(|t| json!({"stat": format!("s_{t}"), "label": "l", "format": {"type": t}}))
        .collect();
    let value = ui_with(json!({
        "stats": stats,
        "screens": [{"kind": "action_bar", "sections": [
            {"type": "ability_list", "value": {"select": {"type": "all"}, "sort": "declared"}},
            {"type": "choice_steps"}]}]
    }));
    let input = bytes(&value);
    let document = read_document(&input).unwrap();
    assert_eq!(write_document(&document).unwrap(), input);
    let ui = read_ui(&value);
    let formats: Vec<_> = ui
        .stats
        .as_ref()
        .unwrap()
        .iter()
        .map(|s| s.format.clone().unwrap())
        .collect();
    assert_eq!(
        formats,
        [
            UiFormat::Integer,
            UiFormat::Signed,
            UiFormat::Percent,
            UiFormat::Boolean,
            UiFormat::Tags,
            UiFormat::Dice
        ]
    );
    for (format, text) in formats.iter().zip(unit) {
        assert_eq!(format.as_str(), text);
    }
    let sections = &ui.screens.as_ref().unwrap()[0].sections;
    assert_eq!(
        sections[0],
        UiSection::AbilityList(UiAbilityList {
            select: UiAbilitySelect::All,
            sort: Some(UiSort::Declared)
        })
    );
    assert_eq!(sections[1], UiSection::ChoiceSteps);
    assert_eq!(sections[1].as_str(), "choice_steps");
    assert_eq!(UiSort::Declared.as_str(), "declared");

    // Written from the in-memory values, each arm gives its own spelling.
    for (format, text) in formats.into_iter().zip(unit) {
        let mut one = minimal();
        one["stats"] = json!([{"stat": "s", "label": "l", "format": {"type": text}}]);
        let ui = UiDocument {
            stats: Some(vec![UiStat {
                stat: "s".into(),
                label: "l".into(),
                short_label: None,
                description: None,
                group: None,
                format: Some(format),
            }]),
            ..read_ui(&minimal())
        };
        assert_eq!(
            write_document(&Document::Ui(ui)).unwrap(),
            bytes(&one),
            "{text}"
        );
    }
    // A section with a body for a unit kind stays `Malformed`.
    let mut bad = minimal();
    bad["screens"] = json!([{"kind": "action_bar", "sections": [
        {"type": "choice_steps", "value": 0}]}]);
    assert!(read_err(&bad).contains("unknown field"));
}

#[test]
fn effective_ui_merges_every_list() {
    // B30 for pools, abilities, effects and outcomes with authored lists that
    // differ from the generated ones.
    let loaded = campaign("combat_srd");
    let source = UiSource::from_documents(&loaded.documents, ulid(SRD)).unwrap();
    let generated = generated_ui(&source);
    let authored = UiDocument {
        ruleset: ulid(MISSING),
        stats: None,
        pools: Some(vec![UiPool {
            pool: ulid(SRD_POOL2),
            label: "ui.p2".into(),
            icon: None,
            style: Some(UiPoolStyle::Pips),
            show_on_hud: Some(false),
        }]),
        abilities: Some(vec![
            UiAbility {
                ability: ulid(SRD_FOCUS),
                label: "ui.focus".into(),
                description: None,
                icon: None,
                category_tags: None,
                target_hint: None,
                show_odds: Some(true),
            },
            UiAbility {
                ability: ulid(GONE),
                label: "ui.ghost".into(),
                description: None,
                icon: None,
                category_tags: None,
                target_hint: None,
                show_odds: None,
            },
        ]),
        effects: Some(vec![UiEffect {
            effect: ulid(SRD_EFFECT),
            label: "ui.fx".into(),
            icon: None,
            show_duration: Some(false),
        }]),
        outcomes: Some(vec![
            UiOutcome {
                outcome: "critical_success".into(),
                label: "ui.crit".into(),
            },
            UiOutcome {
                outcome: "custom:5".into(),
                label: "ui.c5".into(),
            },
        ]),
        screens: None,
        note: Some("dropped".into()),
    };
    let merged = effective_ui(Some(&authored), &source);
    assert_eq!(merged.ruleset, ulid(SRD));
    assert_eq!(merged.note, None);
    // Pools: the authored entry, unchanged, then the generated pool it lacks.
    let pools = merged.pools.unwrap();
    assert_eq!(pools[0], authored.pools.as_ref().unwrap()[0]);
    assert_eq!(pools[1..], generated.pools.as_ref().unwrap()[..1]);
    assert_eq!(pools.len(), 2);
    // Abilities: authored (not filled with a hint), the undeclared one kept, then heavy.
    let abilities = merged.abilities.unwrap();
    assert_eq!(abilities[..2], authored.abilities.as_ref().unwrap()[..]);
    assert_eq!(abilities[2], generated.abilities.as_ref().unwrap()[0]);
    assert_eq!(abilities.len(), 3);
    assert_eq!(abilities[0].target_hint, None);
    // Effects: the one generated effect is already authored, so nothing is added.
    assert_eq!(merged.effects, authored.effects);
    assert_ne!(merged.effects, generated.effects);
    // Outcomes: authored first (one is generated too), then the generated others.
    let outcomes: Vec<_> = merged
        .outcomes
        .unwrap()
        .iter()
        .map(|o| (o.outcome.clone(), o.label.clone()))
        .collect();
    assert_eq!(
        outcomes,
        [
            ("critical_success".to_owned(), "ui.crit".to_owned()),
            ("custom:5".to_owned(), "ui.c5".to_owned()),
            ("failure".to_owned(), "failure".to_owned()),
            ("success".to_owned(), "success".to_owned()),
        ]
    );
    assert_eq!(merged.stats, generated.stats);
    assert_eq!(merged.screens, generated.screens);

    // Control: a fully authored document that differs from the generated one
    // comes back unchanged except for `ruleset` and `note`.
    let mut full = generated.clone();
    full.ruleset = ulid(MISSING);
    full.note = Some("n".into());
    for stat in full.stats.as_mut().unwrap() {
        stat.label = format!("L:{}", stat.stat);
    }
    full.pools.as_mut().unwrap().reverse();
    for ability in full.abilities.as_mut().unwrap() {
        ability.show_odds = Some(true);
    }
    for effect in full.effects.as_mut().unwrap() {
        effect.show_duration = Some(false);
    }
    full.outcomes.as_mut().unwrap().reverse();
    full.screens.as_mut().unwrap().reverse();
    assert_ne!(
        UiDocument {
            ruleset: ulid(SRD),
            note: None,
            ..full.clone()
        },
        generated
    );
    assert_eq!(
        effective_ui(Some(&full), &source),
        UiDocument {
            ruleset: ulid(SRD),
            note: None,
            ..full
        }
    );
}

#[test]
fn read_time_checks_run_in_pre_order() {
    // B5: the first failure in pre-order is the one returned.
    let pairs = [
        ("/stats/0/stat", "/stats/0/label"),
        ("/stats/0/label", "/stats/0/short_label"),
        ("/stats/0/short_label", "/stats/0/description"),
        ("/stats/0/description", "/stats/0/group"),
        ("/stats/0/group", "/stats/0/format/value/value"),
        ("/stats/0/format/value/value", "/stats/1/stat"),
        ("/stats/1/stat", "/stats/1/format/value/0/variant"),
        (
            "/stats/1/format/value/0/variant",
            "/stats/1/format/value/0/label",
        ),
        ("/stats/1/format/value/0/label", "/pools/0/label"),
        ("/pools/0/label", "/pools/0/icon"),
        ("/pools/0/icon", "/pools/0/style"),
        ("/pools/0/style", "/abilities/0/label"),
        ("/abilities/0/label", "/abilities/0/description"),
        ("/abilities/0/description", "/abilities/0/icon"),
        ("/abilities/0/icon", "/abilities/0/category_tags/0"),
        ("/abilities/0/category_tags/0", "/abilities/0/target_hint"),
        ("/abilities/0/target_hint", "/effects/0/label"),
        ("/effects/0/label", "/effects/0/icon"),
        ("/effects/0/icon", "/outcomes/0/outcome"),
        ("/outcomes/0/outcome", "/outcomes/0/label"),
        ("/outcomes/0/label", "/screens/0/kind"),
        ("/screens/0/kind", "/screens/0/sections/0/value/value"),
        (
            "/screens/0/sections/0/value/value",
            "/screens/0/sections/1/value/value/0",
        ),
        (
            "/screens/0/sections/1/value/value/0",
            "/screens/1/sections/0/value/select/value",
        ),
        (
            "/screens/1/sections/0/value/select/value",
            "/screens/1/sections/0/value/sort",
        ),
        (
            "/screens/1/sections/0/value/sort",
            "/screens/2/sections/0/value/value",
        ),
    ];
    let text_error = |pointer: &str| format!("ui text at {pointer} must be 1..=128 UTF-8 bytes");
    for (earlier, later) in pairs {
        for (first, second) in [(earlier, later), (later, earlier)] {
            let mut both = kitchen_sink();
            set(&mut both, first, json!(""));
            set(&mut both, second, json!(""));
            assert_eq!(
                read_err(&both),
                text_error(earlier),
                "{earlier} before {later}"
            );
        }
        for alone in [earlier, later] {
            assert_eq!(read_err(&at_limit("", alone)), text_error(alone));
        }
    }

    // A list's count check precedes its entries; an earlier list precedes a later one.
    let pool = json!({"pool": POOL, "label": "l"});
    let mut value = kitchen_sink();
    value["pools"] = list_of(4097, pool.clone());
    set(&mut value, "/abilities/0/label", json!(""));
    assert_eq!(read_err(&value), "ui list at /pools exceeds 4096 entries");
    let mut value = kitchen_sink();
    value["effects"] = list_of(4097, json!({"effect": ABILITY, "label": "l"}));
    set(&mut value, "/abilities/0/label", json!(""));
    assert_eq!(read_err(&value), text_error("/abilities/0/label"));
    let mut value = kitchen_sink();
    value["outcomes"] = list_of(4097, json!({"outcome": "success", "label": "l"}));
    set(&mut value, "/screens/0/kind", json!(""));
    assert_eq!(
        read_err(&value),
        "ui list at /outcomes exceeds 4096 entries"
    );

    // Screens: count, then each screen's kind, then its sections count, then sections.
    let mut screens = minimal();
    let mut list = vec![json!({"kind": "", "sections": []}); 65];
    screens["screens"] = Value::Array(list.clone());
    assert_eq!(read_err(&screens), "ui list at /screens exceeds 64 entries");
    list.truncate(64);
    screens["screens"] = Value::Array(list);
    assert_eq!(read_err(&screens), text_error("/screens/0/kind"));
    let timelines = list_of(65, json!({"type": "timeline"}));
    let mut sections = minimal();
    sections["screens"] = json!([{"kind": "", "sections": timelines.clone()}]);
    assert_eq!(read_err(&sections), text_error("/screens/0/kind"));
    let mut wide = Value::Array(
        std::iter::once(json!({"type": "carousel", "value": null}))
            .chain(std::iter::repeat_n(json!({"type": ""}), 64))
            .collect(),
    );
    sections["screens"] = json!([{"kind": "combat_hud", "sections": wide.take()}]);
    assert_eq!(
        read_err(&sections),
        "ui list at /screens/0/sections exceeds 64 entries"
    );

    // An empty selector list is reported at its place in the walk.
    let mut empty_first = kitchen_sink();
    set(
        &mut empty_first,
        "/screens/0/sections/1/value/value",
        json!([]),
    );
    set(
        &mut empty_first,
        "/screens/2/sections/0/value/value",
        json!(""),
    );
    assert_eq!(
        read_err(&empty_first),
        "ui list at /screens/0/sections/1/value/value must not be empty"
    );
    let mut text_first = kitchen_sink();
    set(
        &mut text_first,
        "/screens/0/sections/1/value/value",
        json!([]),
    );
    set(
        &mut text_first,
        "/screens/0/sections/0/value/value",
        json!(""),
    );
    assert_eq!(
        read_err(&text_first),
        text_error("/screens/0/sections/0/value/value")
    );
}
