#![forbid(unsafe_code)]
//! T142 two-keys contract tests, written blind from `tasks/T142.md` §"Behaviour"
//! B1–B34, §"Errors" and §"Tests" (the copy that carries the 2026-11-08 user
//! bound on B31). Each test names the B items it covers. Campaign fixtures are
//! built in memory from the read-only `combat_basic` / `combat_srd` bytes.
use crpg_core::Ulid;
use crpg_data::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

type Files = BTreeMap<SourcePath, Vec<u8>>;

const BASIC_RULESET: &str = "0000000000000000000000000J";
const BASIC_ABILITY: &str = "0000000000000000000000000K";
const BASIC_POOL: &str = "0000000000000000000000000P";
const BASIC_CREATURE: &str = "0000000000000000000000000G";
const BASIC_CAMPAIGN: &str = "0000000000000000000000000B";
const SRD_RULESET: &str = "00000000000000000000000016";
const SRD_HEAVY: &str = "00000000000000000000000017";
const SRD_FOCUS: &str = "00000000000000000000000018";
const SRD_HEAVY_TABLE: &str = "00000000000000000000000019";
const SRD_FOCUS_TABLE: &str = "0000000000000000000000001A";
const SRD_EFFECT: &str = "0000000000000000000000001B";
const SRD_POOL_1: &str = "0000000000000000000000001E";
const SRD_POOL_2: &str = "0000000000000000000000001F";
const SRD_CAMPAIGN: &str = "0000000000000000000000000Z";
const FOREIGN: &str = "0000000000000000000000000Y";
const KEY: &str = "fixture.strike"; // present in combat_basic's locale table
const SRD_KEY: &str = "fixture.heavy"; // present in combat_srd's locale table

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    crate_dir().join("..").join("..")
}

fn read_tree(root: &Path) -> Files {
    fn walk(dir: &Path, root: &Path, out: &mut Files) {
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

fn fixture(name: &str) -> Files {
    read_tree(&repo_root().join("campaigns").join("fixtures").join(name))
}

fn engine() -> semver::Version {
    "0.1.0".parse().unwrap()
}

fn path(text: &str) -> SourcePath {
    text.parse().unwrap()
}

fn ulid(text: &str) -> Ulid {
    text.parse().unwrap()
}

fn load(files: &Files) -> LoadedCampaign {
    load_campaign(files, &engine()).unwrap()
}

fn read(value: &Value) -> Result<Document, DataError> {
    read_document(&serde_json::to_vec(value).unwrap())
}

fn read_ok(value: &Value) -> Document {
    read(value).unwrap_or_else(|error| panic!("document should read: {error}"))
}

fn written(doc: &Document) -> Value {
    serde_json::from_slice(&write_document(doc).unwrap()).unwrap()
}

fn malformed(result: Result<Document, DataError>) -> String {
    match result {
        Err(DataError::Malformed { message, .. }) => message,
        other => panic!("expected Malformed, got {other:?}"),
    }
}

fn assert_malformed(value: &Value, needle: &str) {
    let message = malformed(read(value));
    assert!(message.contains(needle), "{message:?} lacks {needle:?}");
}

fn assert_text_error(value: &Value, pointer: &str) {
    assert_eq!(
        malformed(read(value)),
        format!("ui text at {pointer} must be 1..=128 UTF-8 bytes")
    );
}

fn assert_list_error(value: &Value, pointer: &str, max: usize) {
    assert_eq!(
        malformed(read(value)),
        format!("ui list at {pointer} exceeds {max} entries")
    );
}

fn assert_empty_error(value: &Value, pointer: &str) {
    assert_eq!(
        malformed(read(value)),
        format!("ui list at {pointer} must not be empty")
    );
}

fn canonical(value: &Value) -> Vec<u8> {
    canonical_json(value).unwrap()
}

fn add_json(base: &Files, name: &str, value: &Value) -> Files {
    let mut files = base.clone();
    files.insert(path(name), canonical(value));
    files
}

fn add_ui(base: &Files, name: &str, value: &Value) -> Files {
    add_json(base, name, value)
}

/// Adds asset keys and rebuilds `campaign.lock` so the digest stays correct.
fn add_assets(files: &mut Files, keys: &[&str]) {
    let assets_path = path("assets/assets.lock");
    let mut assets = read_assets_lock(&files[&assets_path]).unwrap();
    for key in keys {
        assets.assets.insert(
            key.parse().unwrap(),
            AssetRecord {
                hash: Digest::from_bytes([7; 32]),
                import: BTreeMap::new(),
            },
        );
    }
    let bytes = write_assets_lock(&assets).unwrap();
    let lock = make_campaign_lock(&[], &[], &assets).unwrap();
    files.insert(assets_path, bytes);
    files.insert(path("campaign.lock"), write_campaign_lock(&lock).unwrap());
}

/// Validates a campaign and splits the findings into UI-file and non-UI ones,
/// asserting that the UI file did not change any other finding (B8).
fn check_against(base: &Files, extra: &[(&str, &Value)], ui_names: &[&str]) -> Vec<Diagnostic> {
    let before = validate(&load(base));
    let mut files = base.clone();
    for (name, value) in extra {
        files.insert(path(name), canonical(value));
    }
    let all = validate(&load(&files));
    let is_ui = |diagnostic: &Diagnostic| {
        diagnostic
            .file
            .as_ref()
            .is_some_and(|file| ui_names.contains(&file.as_str()))
    };
    let (ui, rest): (Vec<Diagnostic>, Vec<Diagnostic>) = all.into_iter().partition(is_ui);
    assert_eq!(rest, before, "the UI file changed non-UI findings");
    ui
}

fn check_ui(base: &Files, name: &str, value: &Value) -> Vec<Diagnostic> {
    check_against(base, &[(name, value)], &[name])
}

fn only(diagnostics: &[Diagnostic]) -> Diagnostic {
    assert_eq!(
        diagnostics.len(),
        1,
        "expected one finding: {diagnostics:#?}"
    );
    diagnostics[0].clone()
}

fn assert_only(diagnostics: &[Diagnostic], code: DiagnosticCode, pointer: &str) -> Diagnostic {
    let diagnostic = only(diagnostics);
    assert_eq!(diagnostic.code, code, "{diagnostic:?}");
    assert_eq!(diagnostic.pointer, pointer, "{diagnostic:?}");
    assert_eq!(diagnostic.severity, Severity::Error, "{diagnostic:?}");
    diagnostic
}

fn assert_no(diagnostics: &[Diagnostic], code: DiagnosticCode, pointer: &str) {
    assert!(
        !diagnostics
            .iter()
            .any(|d| d.code == code && d.pointer == pointer),
        "unexpected {code:?} at {pointer}: {diagnostics:#?}"
    );
}

fn sorted(diagnostics: &[Diagnostic]) -> Vec<Diagnostic> {
    let mut copy = diagnostics.to_vec();
    copy.sort_by(|a, b| {
        (
            &a.file,
            &a.pointer,
            a.code.as_str(),
            a.severity,
            &a.message,
            &a.suggested_fix,
        )
            .cmp(&(
                &b.file,
                &b.pointer,
                b.code.as_str(),
                b.severity,
                &b.message,
                &b.suggested_fix,
            ))
    });
    copy
}

fn ui_json() -> Value {
    json!({"schema": "crpg.ui/1", "ruleset": BASIC_RULESET})
}

fn empty_ui() -> UiDocument {
    UiDocument {
        ruleset: ulid(BASIC_RULESET),
        stats: None,
        pools: None,
        abilities: None,
        effects: None,
        outcomes: None,
        screens: None,
        note: None,
    }
}

fn stat_entry(stat: &str) -> UiStat {
    UiStat {
        stat: stat.into(),
        label: KEY.into(),
        short_label: None,
        description: None,
        group: None,
        format: None,
    }
}

fn screen(kind: UiScreenKind) -> UiScreen {
    UiScreen {
        kind,
        sections: Vec::new(),
    }
}

fn six_kind_campaign() -> Files {
    let mut files = fixture("combat_srd");
    let mut ruleset: Value =
        serde_json::from_slice(&files[&path("rulesets/srd-lite.json")]).unwrap();
    ruleset["stats"] = json!([
        {"name": "might", "kind": "int"},
        {"name": "guile", "kind": "fixed"},
        {"name": "resolve", "kind": "bool"},
        {"name": "ward", "kind": {"enum": {"enum_id": "c", "variants": ["red", "blue"]}}},
        {"name": "talent", "kind": "tags"},
        {"name": "lore", "kind": "dice"},
        {"name": "health", "kind": "int"}
    ]);
    ruleset["attributes"] = json!(["might"]);
    files.insert(path("rulesets/srd-lite.json"), canonical(&ruleset));
    files
}

// ---------------------------------------------------------------------------
// U-1 — B1: reading and writing.
// ---------------------------------------------------------------------------

fn wire_example() -> Value {
    json!({
        "schema": "crpg.ui/1", "ruleset": BASIC_RULESET,
        "stats": [{"stat": "health", "label": "ui.health", "group": "ui.vitals",
            "format": {"type": "fraction", "value": {"type": "pool", "value": BASIC_POOL}}}],
        "pools": [{"pool": BASIC_POOL, "label": "ui.actions", "style": "pips"}],
        "abilities": [{"ability": BASIC_ABILITY, "label": "ui.strike",
            "category_tags": ["melee"], "target_hint": "enemy", "show_odds": true}],
        "outcomes": [{"outcome": "success", "label": "ui.hit"}],
        "screens": [
            {"kind": "character_sheet", "sections": [{"type": "stat_group",
                "value": {"type": "group", "value": "ui.vitals"}}]},
            {"kind": "action_bar", "sections": [{"type": "ability_list",
                "value": {"select": {"type": "tag", "value": "melee"}, "sort": "label"}}]},
            {"kind": "combat_hud", "sections": [{"type": "pool_group", "value": {"type": "all"}},
                {"type": "effect_list"}, {"type": "timeline"}, {"type": "log"}]},
            {"kind": "inventory", "sections": [{"type": "item_list", "value": {"type": "all"}}]}
        ]
    })
}

#[test]
fn u1_round_trip_full_and_minimal() {
    for value in [wire_example(), ui_json()] {
        let bytes = canonical(&value);
        let first = read_document(&bytes).unwrap();
        let written = write_document(&first).unwrap();
        assert_eq!(written, bytes);
        assert_eq!(read_document(&written).unwrap(), first);
    }
    // An absent list stays absent; `[]` stays `[]`.
    let minimal = canonical(&ui_json());
    assert!(!String::from_utf8(minimal).unwrap().contains("stats"));
    let empty = json!({
        "schema": "crpg.ui/1", "ruleset": BASIC_RULESET,
        "stats": [], "pools": [], "abilities": [], "effects": [],
        "outcomes": [], "screens": []
    });
    let doc = read_ok(&empty);
    let out = written(&doc);
    for field in [
        "stats",
        "pools",
        "abilities",
        "effects",
        "outcomes",
        "screens",
    ] {
        assert_eq!(out[field], json!([]), "{field}");
    }
    assert_eq!(write_document(&doc).unwrap(), canonical(&empty));
}

// ---------------------------------------------------------------------------
// U-2 — B2: structural rejections.
// ---------------------------------------------------------------------------

#[test]
fn u2_structural_rejections() {
    let mut extra = ui_json();
    extra["extra"] = json!(1);
    assert_malformed(&extra, "unknown field");
    let mut id_field = ui_json();
    id_field["id"] = json!(BASIC_CAMPAIGN);
    assert_malformed(&id_field, "unknown field");
    let no_ruleset = json!({"schema": "crpg.ui/1"});
    assert_malformed(&no_ruleset, "missing field `ruleset`");
    let mut no_label = ui_json();
    no_label["stats"] = json!([{"stat": "health"}]);
    assert_malformed(&no_label, "missing field `label`");
    let mut string_odds = ui_json();
    string_odds["abilities"] =
        json!([{"ability": BASIC_ABILITY, "label": KEY, "show_odds": "yes"}]);
    assert_malformed(&string_odds, "invalid type");
    let mut wrong_type = ui_json();
    wrong_type["stats"] = json!("x");
    assert_malformed(&wrong_type, "invalid type");

    let mut version = ui_json();
    version["schema"] = json!("crpg.ui/2");
    assert!(matches!(
        read(&version),
        Err(DataError::UnsupportedSchema { found, .. }) if found == "crpg.ui/2"
    ));
    let mut float = ui_json();
    float["abilities"] = json!([{"ability": BASIC_ABILITY, "label": KEY, "show_odds": 1.5}]);
    assert!(matches!(read(&float), Err(DataError::Malformed { .. })));

    // Every named required entry field is required.
    let cases = [
        (json!({"stats": [{"label": KEY}]}), "missing field `stat`"),
        (json!({"pools": [{"label": KEY}]}), "missing field `pool`"),
        (
            json!({"abilities": [{"label": KEY}]}),
            "missing field `ability`",
        ),
        (
            json!({"effects": [{"label": KEY}]}),
            "missing field `effect`",
        ),
        (
            json!({"outcomes": [{"label": KEY}]}),
            "missing field `outcome`",
        ),
        (
            json!({"stats": [{"stat": "health", "label": KEY,
                "format": {"type": "enum", "value": [{"label": KEY}]}}]}),
            "missing field `variant`",
        ),
        (
            json!({"screens": [{"sections": []}]}),
            "missing field `kind`",
        ),
        (
            json!({"screens": [{"kind": "inventory"}]}),
            "missing field `sections`",
        ),
        (
            json!({"screens": [{"kind": "action_bar", "sections": [
                {"type": "ability_list", "value": {}}]}]}),
            "missing field `select`",
        ),
    ];
    for (patch, needle) in cases {
        let mut value = ui_json();
        if let Value::Object(map) = &patch {
            for (key, item) in map {
                value[key] = item.clone();
            }
        }
        assert_malformed(&value, needle);
    }
    // Control: the unchanged document reads.
    assert!(read(&ui_json()).is_ok());
}

// ---------------------------------------------------------------------------
// U-4 — B3, B8: unknown spellings load, round-trip and are reported.
// ---------------------------------------------------------------------------

fn unknown_cases() -> Vec<(&'static str, Value, &'static str, &'static str)> {
    vec![
        (
            "ui/kind.json",
            json!({"screens": [{"kind": "journal", "sections": []}]}),
            "/screens/0/kind",
            "unknown screen kind \"journal\"",
        ),
        (
            "ui/section.json",
            json!({"screens": [{"kind": "inventory", "sections": [
                {"type": "gizmo", "value": {"level": 2}}]}]}),
            "/screens/0/sections/0/type",
            "unknown section type \"gizmo\"",
        ),
        (
            "ui/format.json",
            json!({"stats": [{"stat": "health", "label": KEY,
                "format": {"type": "gauge", "value": {"level": 2}}}]}),
            "/stats/0/format/type",
            "unknown format \"gauge\"",
        ),
        (
            "ui/style.json",
            json!({"pools": [{"pool": BASIC_POOL, "label": KEY, "style": "glass"}]}),
            "/pools/0/style",
            "unknown pool style \"glass\"",
        ),
        (
            "ui/hint.json",
            json!({"abilities": [{"ability": BASIC_ABILITY, "label": KEY,
                "target_hint": "sideways"}]}),
            "/abilities/0/target_hint",
            "unknown target hint \"sideways\"",
        ),
        (
            "ui/sort.json",
            json!({"screens": [{"kind": "action_bar", "sections": [
                {"type": "ability_list",
                 "value": {"select": {"type": "all"}, "sort": "rarity"}}]}]}),
            "/screens/0/sections/0/value/sort",
            "unknown sort \"rarity\"",
        ),
    ]
}

#[test]
fn u4_unknown_kinds_load_and_round_trip() {
    let base = fixture("combat_basic");
    for (name, patch, pointer, message) in unknown_cases() {
        let mut value = ui_json();
        if let Value::Object(map) = &patch {
            for (key, item) in map {
                value[key] = item.clone();
            }
        }
        let bytes = canonical(&value);
        let doc = read_document(&bytes).unwrap();
        assert_eq!(write_document(&doc).unwrap(), bytes, "{name} round trip");
        let findings = check_ui(&base, name, &value);
        let finding = assert_only(&findings, DiagnosticCode::UnknownKind, pointer);
        assert_eq!(finding.message, message);
        assert_eq!(
            finding.suggested_fix.as_deref(),
            Some("use a kind this schema version defines")
        );
    }
}

#[test]
fn u4_known_spelling_control_reports_nothing() {
    let base = fixture("combat_basic");
    let cases = [
        (
            "ui/a.json",
            json!({"screens": [{"kind": "spellbook", "sections": []}]}),
        ),
        (
            "ui/b.json",
            json!({"screens": [{"kind": "inventory", "sections": [{"type": "timeline"}]}]}),
        ),
        (
            "ui/c.json",
            json!({"stats": [{"stat": "health", "label": KEY,
                "format": {"type": "integer"}}]}),
        ),
        (
            "ui/d.json",
            json!({"pools": [{"pool": BASIC_POOL, "label": KEY, "style": "bar"}]}),
        ),
        (
            "ui/e.json",
            json!({"abilities": [{"ability": BASIC_ABILITY, "label": KEY,
                "target_hint": "any"}]}),
        ),
        (
            "ui/f.json",
            json!({"screens": [{"kind": "action_bar", "sections": [
                {"type": "ability_list",
                 "value": {"select": {"type": "all"}, "sort": "label"}}]}]}),
        ),
    ];
    for (name, patch) in cases {
        let mut value = ui_json();
        if let Value::Object(map) = &patch {
            for (key, item) in map {
                value[key] = item.clone();
            }
        }
        assert_eq!(check_ui(&base, name, &value), Vec::new(), "{name}");
    }
}

// ---------------------------------------------------------------------------
// U-5 — B3, B4: a known kind with a wrong body; an unknown body's key set.
// ---------------------------------------------------------------------------

fn doc_with_section(section: Value) -> Value {
    let mut value = ui_json();
    value["screens"] = json!([{"kind": "inventory", "sections": [section]}]);
    value
}

fn doc_with_format(format: Value) -> Value {
    let mut value = ui_json();
    value["stats"] = json!([{"stat": "health", "label": KEY, "format": format}]);
    value
}

#[test]
fn u5_known_kind_wrong_body() {
    for section in [
        json!({"type": "timeline", "value": 0}),
        json!({"type": "gizmo", "value": 1, "extra": 2}),
    ] {
        assert_malformed(&doc_with_section(section), "unknown field");
    }
    // The unit-variant trap includes an extra `value` on `timeline`; the
    // missing-`value` case on `stat_group` is a missing-field error.
    assert_malformed(
        &doc_with_section(json!({"type": "stat_group"})),
        "missing field `value`",
    );
    assert_malformed(
        &doc_with_format(json!({"type": "fraction"})),
        "missing field `value`",
    );
    assert_malformed(
        &doc_with_section(json!({"type": "stat_group", "value": {"type": "all", "value": []}})),
        "unknown field",
    );
    assert_malformed(
        &doc_with_format(json!({"type": "fraction", "value": {"type": "gauge", "value": "x"}})),
        "unknown variant",
    );
    assert_malformed(
        &doc_with_section(json!({"type": "stat_group", "value": {"type": "everything"}})),
        "unknown variant",
    );
    // Controls: the correct bodies.
    for section in [
        json!({"type": "timeline"}),
        json!({"type": "stat_group", "value": {"type": "all"}}),
        json!({"type": "gizmo", "value": 1}),
    ] {
        assert!(read(&doc_with_section(section)).is_ok());
    }
    assert!(read(&doc_with_format(
        json!({"type": "fraction", "value": {"type": "stat", "value": "might"}})
    ))
    .is_ok());
}

// ---------------------------------------------------------------------------
// B5: read-time bounds, in the contract's walk order.
// ---------------------------------------------------------------------------

fn entries(count: usize, make: impl Fn(usize) -> Value) -> Value {
    Value::Array((0..count).map(make).collect())
}

#[test]
fn u3_text_bounds_count_bytes_not_chars() {
    let at_limit = "é".repeat(64); // 128 UTF-8 bytes
    let over = format!("{}abc", "é".repeat(63)); // 129 bytes
    let mut value = ui_json();
    value["stats"] = json!([{"stat": at_limit, "label": KEY}]);
    assert!(read(&value).is_ok());
    value["stats"] = json!([{"stat": over, "label": KEY}]);
    assert_text_error(&value, "/stats/0/stat");
    value["stats"] = json!([{"stat": "health", "label": over}]);
    assert_text_error(&value, "/stats/0/label");
    // Field order inside one entry: stat, label, short_label, description, group.
    value["stats"] = json!([{"stat": over, "label": over}]);
    assert_text_error(&value, "/stats/0/stat");
    value["stats"] = json!([{"stat": "health", "label": KEY, "short_label": over,
        "description": over}]);
    assert_text_error(&value, "/stats/0/short_label");
}

#[test]
fn u3_every_string_site_is_bounded() {
    let over = "x".repeat(129);
    let cases: Vec<(Value, &str)> = vec![
        (
            json!({"stats": [{"stat": "health", "label": KEY, "description": over}]}),
            "/stats/0/description",
        ),
        (
            json!({"stats": [{"stat": "health", "label": KEY, "group": over}]}),
            "/stats/0/group",
        ),
        (
            json!({"stats": [{"stat": "health", "label": KEY,
                "format": {"type": "enum", "value": [{"variant": over, "label": KEY}]}}]}),
            "/stats/0/format/value/0/variant",
        ),
        (
            json!({"stats": [{"stat": "health", "label": KEY,
                "format": {"type": "enum", "value": [{"variant": "a", "label": over}]}}]}),
            "/stats/0/format/value/0/label",
        ),
        (
            json!({"pools": [{"pool": BASIC_POOL, "label": over}]}),
            "/pools/0/label",
        ),
        (
            json!({"pools": [{"pool": BASIC_POOL, "label": KEY, "icon": over}]}),
            "/pools/0/icon",
        ),
        (
            json!({"abilities": [{"ability": BASIC_ABILITY, "label": over}]}),
            "/abilities/0/label",
        ),
        (
            json!({"abilities": [{"ability": BASIC_ABILITY, "label": KEY, "description": over}]}),
            "/abilities/0/description",
        ),
        (
            json!({"abilities": [{"ability": BASIC_ABILITY, "label": KEY, "icon": over}]}),
            "/abilities/0/icon",
        ),
        (
            json!({"abilities": [{"ability": BASIC_ABILITY, "label": KEY,
                "category_tags": [over]}]}),
            "/abilities/0/category_tags/0",
        ),
        (
            json!({"effects": [{"effect": SRD_EFFECT, "label": over}]}),
            "/effects/0/label",
        ),
        (
            json!({"effects": [{"effect": SRD_EFFECT, "label": KEY, "icon": over}]}),
            "/effects/0/icon",
        ),
        (
            json!({"outcomes": [{"outcome": over, "label": KEY}]}),
            "/outcomes/0/outcome",
        ),
        (
            json!({"outcomes": [{"outcome": "success", "label": over}]}),
            "/outcomes/0/label",
        ),
        (
            json!({"screens": [{"kind": over, "sections": []}]}),
            "/screens/0/kind",
        ),
        (
            json!({"screens": [{"kind": "inventory", "sections": [{"type": over}]}]}),
            "/screens/0/sections/0/type",
        ),
        (
            json!({"screens": [{"kind": "action_bar", "sections": [
                {"type": "ability_list",
                 "value": {"select": {"type": "all"}, "sort": over}}]}]}),
            "/screens/0/sections/0/value/sort",
        ),
    ];
    for (patch, pointer) in cases {
        let mut value = ui_json();
        if let Value::Object(map) = &patch {
            for (key, item) in map {
                value[key] = item.clone();
            }
        }
        assert_text_error(&value, pointer);
    }
    // `_note` and `Unknown.value` contents are exempt.
    let mut note = ui_json();
    note["_note"] = json!("n".repeat(300));
    assert!(read(&note).is_ok());
    let mut unknown_value = ui_json();
    unknown_value["stats"] = json!([{"stat": "health", "label": KEY,
        "format": {"type": "gauge", "value": {"long": "v".repeat(300)}}}]);
    assert!(read(&unknown_value).is_ok());
}

#[test]
fn u3_list_bounds_and_emptiness() {
    let mut value = ui_json();
    value["stats"] = entries(4096, |_| json!({"stat": "a", "label": "a"}));
    assert!(read(&value).is_ok());
    value["stats"] = entries(4097, |_| json!({"stat": "a", "label": "a"}));
    assert_list_error(&value, "/stats", 4096);

    value = ui_json();
    value["screens"] = entries(64, |_| json!({"kind": "spellbook", "sections": []}));
    assert!(read(&value).is_ok());
    value["screens"] = entries(65, |_| json!({"kind": "spellbook", "sections": []}));
    assert_list_error(&value, "/screens", 64);

    value = ui_json();
    value["screens"] = json!([{"kind": "inventory",
        "sections": entries(64, |_| json!({"type": "timeline"}))}]);
    assert!(read(&value).is_ok());
    value["screens"] = json!([{"kind": "inventory",
        "sections": entries(65, |_| json!({"type": "timeline"}))}]);
    assert_list_error(&value, "/screens/0/sections", 64);

    value = ui_json();
    value["screens"] = json!([{"kind": "character_sheet", "sections": [
        {"type": "stat_group", "value": {"type": "names",
            "value": entries(4097, |i| json!(format!("s{i}")))}}
    ]}]);
    assert_list_error(&value, "/screens/0/sections/0/value/value", 4096);

    value = ui_json();
    value["screens"] = json!([{"kind": "combat_hud", "sections": [
        {"type": "pool_group", "value": {"type": "ids",
            "value": entries(4097, |i| json!(Ulid::from_u128(i as u128).to_string()))}}
    ]}]);
    assert_list_error(&value, "/screens/0/sections/0/value/value", 4096);

    value = ui_json();
    value["abilities"] = json!([{"ability": BASIC_ABILITY, "label": KEY,
        "category_tags": entries(4097, |i| json!(format!("t{i}")))}]);
    assert_list_error(&value, "/abilities/0/category_tags", 4096);

    value = ui_json();
    value["stats"] = json!([{"stat": "health", "label": KEY,
        "format": {"type": "enum", "value":
            entries(4097, |i| json!({"variant": format!("v{i}"), "label": KEY}))}}]);
    assert_list_error(&value, "/stats/0/format/value", 4096);

    // Lists that may be empty: top-level lists and `sections`.
    let mut empty = ui_json();
    empty["stats"] = json!([]);
    empty["screens"] = json!([{"kind": "inventory", "sections": []}]);
    assert!(read(&empty).is_ok());

    // Lists that may not be empty: selector names/ids, enum labels, tags.
    value = ui_json();
    value["screens"] = json!([{"kind": "character_sheet", "sections": [
        {"type": "stat_group", "value": {"type": "names", "value": []}}]}]);
    assert_empty_error(&value, "/screens/0/sections/0/value/value");
    value = ui_json();
    value["screens"] = json!([{"kind": "combat_hud", "sections": [
        {"type": "pool_group", "value": {"type": "ids", "value": []}}]}]);
    assert_empty_error(&value, "/screens/0/sections/0/value/value");
    value = ui_json();
    value["stats"] = json!([{"stat": "health", "label": KEY,
        "format": {"type": "enum", "value": []}}]);
    assert_empty_error(&value, "/stats/0/format/value");
    value = ui_json();
    value["abilities"] = json!([{"ability": BASIC_ABILITY, "label": KEY, "category_tags": []}]);
    assert_empty_error(&value, "/abilities/0/category_tags");
}

#[test]
fn u3_first_failure_follows_the_walk_order() {
    // A list's count check precedes its entries.
    let mut value = ui_json();
    value["stats"] = entries(4097, |_| json!({"stat": "x".repeat(129), "label": "a"}));
    assert_list_error(&value, "/stats", 4096);
    // Top-level order: stats before pools.
    value = ui_json();
    value["stats"] = json!([{"stat": "x".repeat(129), "label": KEY}]);
    value["pools"] = entries(4097, |_| json!({"pool": BASIC_POOL, "label": KEY}));
    assert_text_error(&value, "/stats/0/stat");
    // Within a screen, the kind precedes its sections.
    value = ui_json();
    value["screens"] = json!([{"kind": "k".repeat(129),
        "sections": [{"type": "t".repeat(129)}]}]);
    assert_text_error(&value, "/screens/0/kind");
}

// ---------------------------------------------------------------------------
// U-6 — B6: an `Unknown` holding a known spelling is Malformed on write.
// ---------------------------------------------------------------------------

#[test]
fn u6_known_spelling_as_unknown_rejected() {
    let cases: Vec<(UiDocument, &str, &str)> = vec![
        (
            UiDocument {
                screens: Some(vec![screen(UiScreenKind::Unknown("spellbook".into()))]),
                ..empty_ui()
            },
            "screen kind",
            "/screens/0/kind",
        ),
        (
            UiDocument {
                screens: Some(vec![UiScreen {
                    kind: UiScreenKind::Inventory,
                    sections: vec![UiSection::Unknown {
                        kind: "timeline".into(),
                        value: None,
                    }],
                }]),
                ..empty_ui()
            },
            "section type",
            "/screens/0/sections/0/type",
        ),
        (
            UiDocument {
                stats: Some(vec![UiStat {
                    format: Some(UiFormat::Unknown {
                        kind: "integer".into(),
                        value: None,
                    }),
                    ..stat_entry("health")
                }]),
                ..empty_ui()
            },
            "format",
            "/stats/0/format/type",
        ),
        (
            UiDocument {
                pools: Some(vec![UiPool {
                    pool: ulid(BASIC_POOL),
                    label: KEY.into(),
                    icon: None,
                    style: Some(UiPoolStyle::Unknown("pips".into())),
                    show_on_hud: None,
                }]),
                ..empty_ui()
            },
            "pool style",
            "/pools/0/style",
        ),
        (
            UiDocument {
                abilities: Some(vec![UiAbility {
                    ability: ulid(BASIC_ABILITY),
                    label: KEY.into(),
                    description: None,
                    icon: None,
                    category_tags: None,
                    target_hint: Some(UiTargetHint::Unknown("enemy".into())),
                    show_odds: None,
                }]),
                ..empty_ui()
            },
            "target hint",
            "/abilities/0/target_hint",
        ),
        (
            UiDocument {
                screens: Some(vec![UiScreen {
                    kind: UiScreenKind::ActionBar,
                    sections: vec![UiSection::AbilityList(UiAbilityList {
                        select: UiAbilitySelect::All,
                        sort: Some(UiSort::Unknown("label".into())),
                    })],
                }]),
                ..empty_ui()
            },
            "sort",
            "/screens/0/sections/0/value/sort",
        ),
    ];
    for (doc, what, pointer) in cases {
        let result = write_document(&Document::Ui(doc.clone()));
        match result {
            Err(DataError::Malformed { message, .. }) => assert_eq!(
                message,
                format!("ui unknown {what} at {pointer} uses a known spelling")
            ),
            other => panic!("expected Malformed, got {other:?}"),
        }
    }
    // Control: an unknown text that is not a known spelling writes.
    let doc = UiDocument {
        screens: Some(vec![screen(UiScreenKind::Unknown("journal".into()))]),
        ..empty_ui()
    };
    assert!(write_document(&Document::Ui(doc)).is_ok());
}

// ---------------------------------------------------------------------------
// U-7 — B7: the `ui/` layout family and the classifier.
// ---------------------------------------------------------------------------

#[test]
fn u7_layout_and_classifier() {
    let base = fixture("combat_basic");
    assert!(load_campaign(&add_ui(&base, "ui/a.json", &ui_json()), &engine()).is_ok());
    assert!(load_campaign(&add_ui(&base, "ui/x/y.json", &ui_json()), &engine()).is_ok());

    let creature: Value = serde_json::from_slice(&base[&path("creatures/goblin.json")]).unwrap();
    for name in ["rulesets/a.json", "a.json"] {
        let files = add_ui(&base, name, &ui_json());
        assert!(
            matches!(
                load_campaign(&files, &engine()),
                Err(DataError::Layout { path: Some(ref p), .. }) if p.as_str() == name
            ),
            "{name}"
        );
    }
    let files = add_ui(&base, "ui/a.json", &creature);
    assert!(matches!(
        load_campaign(&files, &engine()),
        Err(DataError::Layout { path: Some(ref p), .. }) if p.as_str() == "ui/a.json"
    ));
    // Control: the campaign without the UI file loads.
    assert!(load_campaign(&base, &engine()).is_ok());

    for accepted in ["ui/a.json", "ui/x/y.json"] {
        assert!(
            campaign_document_path(accepted).unwrap().is_some(),
            "{accepted}"
        );
    }
    for rejected in ["ui/.json", "ui.json"] {
        assert_eq!(
            campaign_document_path(rejected).unwrap(),
            None,
            "{rejected}"
        );
    }
}

// ---------------------------------------------------------------------------
// U-8 — B8, B9: loading is unaffected; `validate_files` equals `validate`.
// ---------------------------------------------------------------------------

const SPARE_ABILITY: &str = "00000000000000000000000020";

/// `combat_basic` plus an unlisted `Ability` document and an enum stat, so that
/// every B12-B23 defect kind can be planted.
fn defect_base() -> Files {
    let mut base = fixture("combat_basic");
    let mut ruleset: Value =
        serde_json::from_slice(&base[&path("rulesets/minimal-d6.json")]).unwrap();
    ruleset["stats"].as_array_mut().unwrap().push(
        json!({"name": "ward", "kind": {"enum": {"enum_id": "c", "variants": ["red", "blue"]}}}),
    );
    base.insert(path("rulesets/minimal-d6.json"), canonical(&ruleset));
    add_json(
        &base,
        "abilities/spare.json",
        &ability_doc(SPARE_ABILITY, "spare"),
    )
}

fn defect_ui() -> Value {
    json!({
        "schema": "crpg.ui/1", "ruleset": BASIC_RULESET,
        "stats": [
            {"stat": "might", "label": KEY},
            {"stat": "might", "label": KEY},
            {"stat": "nope", "label": KEY},
            {"stat": "ward", "label": "nokey.label", "format": {"type": "enum", "value": [
                {"variant": "green", "label": KEY}]}},
            {"stat": "health", "label": KEY, "format": {"type": "boolean"}}
        ],
        "pools": [
            {"pool": BASIC_POOL, "label": KEY},
            {"pool": FOREIGN, "label": KEY, "style": "glass"}
        ],
        "abilities": [
            {"ability": BASIC_ABILITY, "label": KEY, "target_hint": "sideways"},
            {"ability": FOREIGN, "label": KEY, "category_tags": ["a", "a"]},
            {"ability": SPARE_ABILITY, "label": KEY},
            {"ability": BASIC_CREATURE, "label": KEY}
        ],
        "effects": [{"effect": FOREIGN, "label": KEY, "icon": "ui/none.png"}],
        "outcomes": [
            {"outcome": "success", "label": KEY},
            {"outcome": "success", "label": KEY},
            {"outcome": "custom:0", "label": KEY}
        ],
        "screens": [
            {"kind": "level_up", "sections": [{"type": "timeline"}]},
            {"kind": "spellbook", "sections": [{"type": "gizmo"}]},
            {"kind": "spellbook", "sections": []},
            {"kind": "character_sheet", "sections": [
                {"type": "stat_group", "value": {"type": "group", "value": "ghost"}}]},
            {"kind": "inventory", "sections": [
                {"type": "item_list", "value": {"type": "tag", "value": "rare"}}]},
            {"kind": "action_bar", "sections": [{"type": "ability_list",
                "value": {"select": {"type": "tag", "value": "nope"}}}]}
        ]
    })
}

#[test]
fn u8_loading_is_unaffected() {
    let base = defect_base();
    let base_loaded = load(&base);
    let mut files = add_ui(&base, "ui/defect.json", &defect_ui());
    files = add_ui(&files, "ui/second.json", &ui_json());
    let loaded = load_campaign(&files, &engine()).expect("UI-only defects still load");
    assert_eq!(loaded.index, base_loaded.index);
    assert_eq!(loaded.index.len(), base_loaded.index.len());
    let mut plain = serialize_campaign(&base_loaded).unwrap();
    let mut with = serialize_campaign(&loaded).unwrap();
    assert_eq!(plain.remove(&path("ui/defect.json")), None);
    with.remove(&path("ui/defect.json"));
    with.remove(&path("ui/second.json"));
    assert_eq!(plain, with);

    let findings = validate(&loaded);
    let codes = [
        "duplicate_binding",
        "unknown_stat",
        "unknown_pool",
        "unknown_kind",
        "unknown_outcome",
        "reserved_kind",
        "unknown_tag",
        "missing_asset",
        "missing_locale_key",
        "dangling_reference",
        "wrong_reference_kind",
        "foreign_reference",
        "unknown_group",
        "unknown_variant",
        "format_mismatch",
    ];
    let ui_findings: Vec<&Diagnostic> = findings
        .iter()
        .filter(|d| {
            matches!(
                d.file.as_ref().map(SourcePath::as_str),
                Some("ui/defect.json" | "ui/second.json")
            )
        })
        .collect();
    for finding in &ui_findings {
        assert!(
            codes.contains(&finding.code.as_str()),
            "unexpected UI code {finding:?}"
        );
    }
    // Every planted defect kind is reported, by file, code and pointer.
    let defect = "ui/defect.json";
    let expected: Vec<(&str, DiagnosticCode, &str)> = vec![
        (defect, DiagnosticCode::DuplicateBinding, "/stats/1/stat"),
        (defect, DiagnosticCode::UnknownStat, "/stats/2/stat"),
        (defect, DiagnosticCode::MissingLocaleKey, "/stats/3/label"),
        (
            defect,
            DiagnosticCode::UnknownVariant,
            "/stats/3/format/value/0/variant",
        ),
        (defect, DiagnosticCode::FormatMismatch, "/stats/4/format"),
        (defect, DiagnosticCode::UnknownPool, "/pools/1/pool"),
        (defect, DiagnosticCode::UnknownKind, "/pools/1/style"),
        (
            defect,
            DiagnosticCode::UnknownKind,
            "/abilities/0/target_hint",
        ),
        (
            defect,
            DiagnosticCode::DanglingReference,
            "/abilities/1/ability",
        ),
        (
            defect,
            DiagnosticCode::DuplicateBinding,
            "/abilities/1/category_tags/1",
        ),
        (
            defect,
            DiagnosticCode::ForeignReference,
            "/abilities/2/ability",
        ),
        (
            defect,
            DiagnosticCode::WrongReferenceKind,
            "/abilities/3/ability",
        ),
        (
            defect,
            DiagnosticCode::DanglingReference,
            "/effects/0/effect",
        ),
        (defect, DiagnosticCode::MissingAsset, "/effects/0/icon"),
        (
            defect,
            DiagnosticCode::DuplicateBinding,
            "/outcomes/1/outcome",
        ),
        (
            defect,
            DiagnosticCode::UnknownOutcome,
            "/outcomes/2/outcome",
        ),
        (defect, DiagnosticCode::ReservedKind, "/screens/0/kind"),
        (
            defect,
            DiagnosticCode::UnknownKind,
            "/screens/1/sections/0/type",
        ),
        (defect, DiagnosticCode::DuplicateBinding, "/screens/2/kind"),
        (
            defect,
            DiagnosticCode::UnknownGroup,
            "/screens/3/sections/0/value/value",
        ),
        (
            defect,
            DiagnosticCode::UnknownTag,
            "/screens/4/sections/0/value/value",
        ),
        (
            defect,
            DiagnosticCode::UnknownTag,
            "/screens/5/sections/0/value/select/value",
        ),
        (
            "ui/second.json",
            DiagnosticCode::DuplicateBinding,
            "/ruleset",
        ),
    ];
    for (file, code, pointer) in &expected {
        assert!(
            ui_findings.iter().any(|d| {
                d.file.as_ref().map(SourcePath::as_str) == Some(*file)
                    && d.code == *code
                    && d.pointer == *pointer
            }),
            "missing {code:?} at {file}{pointer}: {ui_findings:#?}"
        );
    }
    assert_eq!(ui_findings.len(), expected.len(), "{ui_findings:#?}");
    assert_eq!(validate_files(&files, &engine()), findings);
    // B9: a UI document is absent from the object index.
    assert!(loaded
        .index
        .values()
        .all(|entry| !entry.path.as_str().starts_with("ui/")));
}

// ---------------------------------------------------------------------------
// U-9 — B10, B11, B12: reference sites, the (R) gate and the ruleset binding.
// ---------------------------------------------------------------------------

#[test]
fn u9_reference_occurrences_use_the_contract_pointers() {
    let base = fixture("combat_basic");

    let mut dangling = ui_json();
    dangling["ruleset"] = json!(FOREIGN);
    let finding = only(&check_ui(&base, "ui/a.json", &dangling));
    assert_eq!(finding.code, DiagnosticCode::DanglingReference);
    assert_eq!(finding.pointer, "/ruleset");
    assert_eq!(
        finding.message,
        format!("dangling ui ruleset reference to {FOREIGN}")
    );

    let mut wrong = ui_json();
    wrong["ruleset"] = json!(BASIC_CREATURE);
    let finding = only(&check_ui(&base, "ui/a.json", &wrong));
    assert_eq!(finding.code, DiagnosticCode::WrongReferenceKind);
    assert_eq!(finding.pointer, "/ruleset");
    assert_eq!(
        finding.message,
        format!(
            "wrong kind for ui ruleset reference to {BASIC_CREATURE}: found creature, expected ruleset"
        )
    );

    let mut meter = ui_json();
    meter["stats"] = json!([{"stat": "health", "label": KEY,
        "format": {"type": "fraction", "value": {"type": "pool", "value": FOREIGN}}}]);
    let finding = only(&check_ui(&base, "ui/a.json", &meter));
    assert_eq!(finding.code, DiagnosticCode::UnknownPool);
    assert_eq!(finding.pointer, "/stats/0/format/value/value");

    let mut pools = ui_json();
    pools["pools"] = json!([{"pool": FOREIGN, "label": KEY}]);
    let finding = only(&check_ui(&base, "ui/a.json", &pools));
    assert_eq!(finding.code, DiagnosticCode::UnknownPool);
    assert_eq!(finding.pointer, "/pools/0/pool");

    let mut abilities = ui_json();
    abilities["abilities"] = json!([{"ability": FOREIGN, "label": KEY}]);
    let finding = only(&check_ui(&base, "ui/a.json", &abilities));
    assert_eq!(finding.code, DiagnosticCode::DanglingReference);
    assert_eq!(finding.pointer, "/abilities/0/ability");
    assert_eq!(
        finding.message,
        format!("dangling ui ability reference to {FOREIGN}")
    );

    let mut effects = ui_json();
    effects["effects"] = json!([{"effect": FOREIGN, "label": KEY}]);
    let finding = only(&check_ui(&base, "ui/a.json", &effects));
    assert_eq!(finding.code, DiagnosticCode::DanglingReference);
    assert_eq!(finding.pointer, "/effects/0/effect");
    assert_eq!(
        finding.message,
        format!("dangling ui effect reference to {FOREIGN}")
    );

    let mut pool_group = ui_json();
    pool_group["screens"] = json!([{"kind": "combat_hud", "sections": [
        {"type": "pool_group", "value": {"type": "ids", "value": [FOREIGN]}}]}]);
    let finding = only(&check_ui(&base, "ui/a.json", &pool_group));
    assert_eq!(finding.pointer, "/screens/0/sections/0/value/value/0");
    assert_eq!(finding.code, DiagnosticCode::UnknownPool);

    let mut ability_list = ui_json();
    ability_list["screens"] = json!([{"kind": "action_bar", "sections": [
        {"type": "ability_list", "value": {"select": {"type": "ids", "value": [FOREIGN]}}}]}]);
    let finding = only(&check_ui(&base, "ui/a.json", &ability_list));
    assert_eq!(
        finding.pointer,
        "/screens/0/sections/0/value/select/value/0"
    );
    assert_eq!(finding.code, DiagnosticCode::DanglingReference);
    assert_eq!(
        finding.message,
        format!("dangling ui ability reference to {FOREIGN}")
    );

    // Sections inside a reserved screen are still enumerated (B10), but B20
    // does not judge them.
    let mut reserved = ui_json();
    reserved["screens"] = json!([{"kind": "level_up", "sections": [
        {"type": "ability_list", "value": {"select": {"type": "ids", "value": [FOREIGN]}}}]}]);
    let findings = check_ui(&base, "ui/a.json", &reserved);
    assert_eq!(findings.len(), 2, "{findings:#?}");
    assert!(findings
        .iter()
        .any(|d| d.code == DiagnosticCode::ReservedKind && d.pointer == "/screens/0/kind"));
    assert!(findings
        .iter()
        .any(|d| d.code == DiagnosticCode::DanglingReference
            && d.pointer == "/screens/0/sections/0/value/select/value/0"));

    // `Unknown.value` is never scanned for ULIDs.
    let mut opaque = ui_json();
    opaque["stats"] = json!([{"stat": "health", "label": KEY,
        "format": {"type": "gauge", "value": {"id": FOREIGN}}}]);
    let findings = check_ui(&base, "ui/a.json", &opaque);
    let finding = only(&findings);
    assert_eq!(finding.code, DiagnosticCode::UnknownKind);
    assert_eq!(finding.pointer, "/stats/0/format/type");
}

#[test]
fn u9_r_gate_blocks_only_ruleset_dependent_checks() {
    let base = fixture("combat_basic");
    let mut value = ui_json();
    value["ruleset"] = json!(FOREIGN);
    value["stats"] = json!([
        {"stat": "might", "label": KEY},
        {"stat": "might", "label": KEY},
        {"stat": "nope", "label": KEY, "format": {"type": "boolean"}}
    ]);
    value["pools"] = json!([{"pool": BASIC_POOL, "label": KEY, "style": "glass"}]);
    let findings = check_ui(&base, "ui/a.json", &value);
    let pointers: Vec<(DiagnosticCode, &str)> = findings
        .iter()
        .map(|d| (d.code, d.pointer.as_str()))
        .collect();
    assert_eq!(
        pointers,
        vec![
            (DiagnosticCode::UnknownKind, "/pools/0/style"),
            (DiagnosticCode::DanglingReference, "/ruleset"),
            (DiagnosticCode::DuplicateBinding, "/stats/1/stat"),
        ],
        "{findings:#?}"
    );
    assert!(findings.iter().all(|d| d.severity == Severity::Error));
}

#[test]
fn u9_ruleset_duplicate_binding() {
    let base = fixture("combat_basic");
    let findings = check_against(
        &base,
        &[("ui/a.json", &ui_json()), ("ui/b.json", &ui_json())],
        &["ui/a.json", "ui/b.json"],
    );
    let finding = only(&findings);
    assert_eq!(
        finding.file.as_ref().map(SourcePath::as_str),
        Some("ui/b.json")
    );
    assert_eq!(finding.code, DiagnosticCode::DuplicateBinding);
    assert_eq!(finding.pointer, "/ruleset");
    assert_eq!(
        finding.message,
        format!("duplicate ui for ruleset {BASIC_RULESET} (first at ui/a.json/ruleset)")
    );
    assert_eq!(finding.suggested_fix.as_deref(), Some("keep one entry"));
    // Control: one file, no finding.
    assert_eq!(check_ui(&base, "ui/a.json", &ui_json()), Vec::new());

    // A non-ruleset id never participates in the duplicate rule.
    let mut first = ui_json();
    first["ruleset"] = json!(BASIC_CREATURE);
    let mut second = ui_json();
    second["ruleset"] = json!(BASIC_CREATURE);
    let findings = check_against(
        &base,
        &[("ui/a.json", &first), ("ui/b.json", &second)],
        &["ui/a.json", "ui/b.json"],
    );
    assert_eq!(findings.len(), 2, "{findings:#?}");
    assert!(findings
        .iter()
        .all(|d| d.code == DiagnosticCode::WrongReferenceKind && d.pointer == "/ruleset"));
}

// ---------------------------------------------------------------------------
// U-10 — B13, B15–B17, B24: entry resolution and one finding per pointer.
// ---------------------------------------------------------------------------

const BASIC_TABLE: &str = "0000000000000000000000000M";

fn ability_doc(id: &str, slug: &str) -> Value {
    json!({
        "schema": "crpg.ability/2", "id": id, "slug": slug, "name": KEY,
        "dice": "2d6", "attribute": "might", "outcome_table": BASIC_TABLE,
        "damage": [{"outcome": {"type": "success"}, "amount": 1}],
        "cost": 1, "extra_costs": [], "ends_turn": true,
        "defense": {"type": "actor_attribute"},
        "requires_target": true, "allow_self_target": false
    })
}

fn effect_doc(id: &str, slug: &str) -> Value {
    json!({
        "schema": "crpg.effect/1", "id": id, "slug": slug, "name": SRD_KEY,
        "aim": {"type": "slf"}, "mod_type": "focus",
        "policy": {"type": "stack_all"},
        "modifiers": [{"id": FOREIGN, "target": {"type": "roll"},
            "op": {"type": "add"}, "value": 1, "priority": 0, "name": null}],
        "duration_rounds": 1
    })
}

#[test]
fn u10_stat_binding() {
    let base = fixture("combat_basic");
    let mut duplicate = ui_json();
    duplicate["stats"] = json!([
        {"stat": "might", "label": KEY},
        {"stat": "might", "label": KEY}
    ]);
    let finding = only(&check_ui(&base, "ui/a.json", &duplicate));
    assert_eq!(finding.code, DiagnosticCode::DuplicateBinding);
    assert_eq!(finding.pointer, "/stats/1/stat");
    assert_eq!(
        finding.message,
        "duplicate stat \"might\" (first at ui/a.json/stats/0/stat)"
    );

    let mut unknown = ui_json();
    unknown["stats"] = json!([{"stat": "nope", "label": KEY}]);
    let finding = only(&check_ui(&base, "ui/a.json", &unknown));
    assert_eq!(finding.code, DiagnosticCode::UnknownStat);
    assert_eq!(finding.pointer, "/stats/0/stat");
    assert_eq!(
        finding.message,
        format!("unknown stat \"nope\" for ui: not declared by ruleset {BASIC_RULESET}")
    );

    let mut control = ui_json();
    control["stats"] = json!([{"stat": "might", "label": KEY}]);
    assert_eq!(check_ui(&base, "ui/a.json", &control), Vec::new());

    // A duplicate is reported as a duplicate even when the name is also
    // undeclared: duplicate_binding precedes the (R) resolution.
    let mut both = ui_json();
    both["stats"] = json!([
        {"stat": "nope", "label": KEY},
        {"stat": "nope", "label": KEY}
    ]);
    let findings = check_ui(&base, "ui/a.json", &both);
    assert_eq!(findings.len(), 2, "{findings:#?}");
    assert_eq!(findings[0].code, DiagnosticCode::UnknownStat);
    assert_eq!(findings[0].pointer, "/stats/0/stat");
    assert_eq!(findings[1].code, DiagnosticCode::DuplicateBinding);
    assert_eq!(findings[1].pointer, "/stats/1/stat");
}

#[test]
fn u10_pool_binding_and_style() {
    let base = fixture("combat_basic");
    let mut duplicate = ui_json();
    duplicate["pools"] = json!([
        {"pool": BASIC_POOL, "label": KEY},
        {"pool": BASIC_POOL, "label": KEY}
    ]);
    let finding = only(&check_ui(&base, "ui/a.json", &duplicate));
    assert_eq!(finding.code, DiagnosticCode::DuplicateBinding);
    assert_eq!(finding.pointer, "/pools/1/pool");
    assert_eq!(
        finding.message,
        format!("duplicate pool {BASIC_POOL} (first at ui/a.json/pools/0/pool)")
    );

    let mut foreign = ui_json();
    foreign["pools"] = json!([{"pool": FOREIGN, "label": KEY}]);
    let finding = only(&check_ui(&base, "ui/a.json", &foreign));
    assert_eq!(finding.code, DiagnosticCode::UnknownPool);
    assert_eq!(finding.pointer, "/pools/0/pool");
    assert_eq!(
        finding.message,
        format!("unknown pool {FOREIGN} for ui: not a template of ruleset {BASIC_RULESET}")
    );

    let mut style = ui_json();
    style["pools"] = json!([{"pool": BASIC_POOL, "label": KEY, "style": "glass"}]);
    let finding = only(&check_ui(&base, "ui/a.json", &style));
    assert_eq!(finding.code, DiagnosticCode::UnknownKind);
    assert_eq!(finding.pointer, "/pools/0/style");
    assert_eq!(finding.message, "unknown pool style \"glass\"");

    let mut control = ui_json();
    control["pools"] = json!([{"pool": BASIC_POOL, "label": KEY}]);
    assert_eq!(check_ui(&base, "ui/a.json", &control), Vec::new());
}

#[test]
fn u10_ability_binding() {
    let base = fixture("combat_basic");
    let mut control = ui_json();
    control["abilities"] = json!([{"ability": BASIC_ABILITY, "label": KEY}]);
    assert_eq!(check_ui(&base, "ui/a.json", &control), Vec::new());

    let mut duplicate = ui_json();
    duplicate["abilities"] = json!([
        {"ability": BASIC_ABILITY, "label": KEY},
        {"ability": BASIC_ABILITY, "label": KEY}
    ]);
    let finding = only(&check_ui(&base, "ui/a.json", &duplicate));
    assert_eq!(finding.code, DiagnosticCode::DuplicateBinding);
    assert_eq!(finding.pointer, "/abilities/1/ability");
    assert_eq!(
        finding.message,
        format!("duplicate ability {BASIC_ABILITY} (first at ui/a.json/abilities/0/ability)")
    );

    // An Ability document the owning ruleset does not list is a foreign
    // reference (the object index resolves it; the ruleset does not).
    let spare_id = "00000000000000000000000020";
    let spare = ability_doc(spare_id, "spare");
    let with_spare = add_json(&base, "abilities/spare.json", &spare);
    let mut foreign = ui_json();
    foreign["abilities"] = json!([{"ability": spare_id, "label": KEY}]);
    let finding = only(&check_ui(&with_spare, "ui/a.json", &foreign));
    assert_eq!(finding.code, DiagnosticCode::ForeignReference);
    assert_eq!(finding.pointer, "/abilities/0/ability");
    assert_eq!(
        finding.message,
        format!("ui ability {spare_id} is not listed by ruleset {BASIC_RULESET}")
    );

    // A dangling id is flagged by the generic pass only, even when repeated.
    let mut dangling = ui_json();
    dangling["abilities"] = json!([
        {"ability": FOREIGN, "label": KEY},
        {"ability": FOREIGN, "label": KEY}
    ]);
    let findings = check_ui(&base, "ui/a.json", &dangling);
    assert_eq!(findings.len(), 2, "{findings:#?}");
    for (index, finding) in findings.iter().enumerate() {
        assert_eq!(finding.code, DiagnosticCode::DanglingReference);
        assert_eq!(finding.pointer, format!("/abilities/{index}/ability"));
    }

    let mut tags = ui_json();
    tags["abilities"] = json!([{"ability": BASIC_ABILITY, "label": KEY,
        "category_tags": ["melee", "melee"]}]);
    let finding = only(&check_ui(&base, "ui/a.json", &tags));
    assert_eq!(finding.code, DiagnosticCode::DuplicateBinding);
    assert_eq!(finding.pointer, "/abilities/0/category_tags/1");
    assert_eq!(
        finding.message,
        "duplicate tag \"melee\" (first at ui/a.json/abilities/0/category_tags/0)"
    );

    let mut hint = ui_json();
    hint["abilities"] = json!([{"ability": BASIC_ABILITY, "label": KEY,
        "target_hint": "backwards"}]);
    let finding = only(&check_ui(&base, "ui/a.json", &hint));
    assert_eq!(finding.code, DiagnosticCode::UnknownKind);
    assert_eq!(finding.pointer, "/abilities/0/target_hint");
    assert_eq!(finding.message, "unknown target hint \"backwards\"");
}

#[test]
fn u10_effect_binding() {
    let base = fixture("combat_srd");
    let mut control = ui_json();
    control["ruleset"] = json!(SRD_RULESET);
    control["effects"] = json!([{"effect": SRD_EFFECT, "label": SRD_KEY}]);
    assert_eq!(check_ui(&base, "ui/a.json", &control), Vec::new());

    let spare_id = "00000000000000000000000020";
    let spare = effect_doc(spare_id, "spare-effect");
    let with_spare = add_json(&base, "effects/spare.json", &spare);
    let mut foreign = ui_json();
    foreign["ruleset"] = json!(SRD_RULESET);
    foreign["effects"] = json!([{"effect": spare_id, "label": SRD_KEY}]);
    let finding = only(&check_ui(&with_spare, "ui/a.json", &foreign));
    assert_eq!(finding.code, DiagnosticCode::ForeignReference);
    assert_eq!(finding.pointer, "/effects/0/effect");
    assert_eq!(
        finding.message,
        format!("ui effect {spare_id} is applied by no ability of ruleset {SRD_RULESET}")
    );

    let mut duplicate = ui_json();
    duplicate["ruleset"] = json!(SRD_RULESET);
    duplicate["effects"] = json!([
        {"effect": spare_id, "label": SRD_KEY},
        {"effect": spare_id, "label": SRD_KEY}
    ]);
    let findings = check_ui(&with_spare, "ui/a.json", &duplicate);
    assert_eq!(findings.len(), 2, "{findings:#?}");
    assert_eq!(findings[0].code, DiagnosticCode::ForeignReference);
    assert_eq!(findings[0].pointer, "/effects/0/effect");
    assert_eq!(findings[1].code, DiagnosticCode::DuplicateBinding);
    assert_eq!(findings[1].pointer, "/effects/1/effect");
    assert_eq!(
        findings[1].message,
        format!("duplicate effect {spare_id} (first at ui/a.json/effects/0/effect)")
    );

    let mut dangling = ui_json();
    dangling["ruleset"] = json!(SRD_RULESET);
    dangling["effects"] = json!([{"effect": FOREIGN, "label": SRD_KEY}]);
    let finding = only(&check_ui(&base, "ui/a.json", &dangling));
    assert_eq!(finding.code, DiagnosticCode::DanglingReference);
    assert_eq!(finding.pointer, "/effects/0/effect");
    assert_eq!(
        finding.message,
        format!("dangling ui effect reference to {FOREIGN}")
    );
}

#[test]
fn u11_outcome_resolution_and_grammar() {
    let base = fixture("combat_srd");
    let cases = [
        ("success", true),
        ("failure", true),
        ("critical_success", true),
        ("critical_failure", false),
        ("custom:0", false),
        ("custom:", false),
        ("custom:256", false),
        ("custom:01", false),
        ("Success", false),
    ];
    for (text, accepted) in cases {
        let mut value = ui_json();
        value["ruleset"] = json!(SRD_RULESET);
        value["outcomes"] = json!([{"outcome": text, "label": SRD_KEY}]);
        let findings = check_ui(&base, "ui/a.json", &value);
        if accepted {
            assert_eq!(findings, Vec::new(), "{text}");
        } else {
            let finding = assert_only(
                &findings,
                DiagnosticCode::UnknownOutcome,
                "/outcomes/0/outcome",
            );
            assert_eq!(
                finding.message,
                format!("outcome {text:?} is not produced by ruleset {SRD_RULESET}")
            );
            assert_eq!(
                finding.suggested_fix.as_deref(),
                Some("use an outcome the ruleset's tables or damage entries produce")
            );
        }
    }

    let mut duplicate = ui_json();
    duplicate["ruleset"] = json!(SRD_RULESET);
    duplicate["outcomes"] = json!([
        {"outcome": "success", "label": SRD_KEY},
        {"outcome": "success", "label": SRD_KEY}
    ]);
    let finding = only(&check_ui(&base, "ui/a.json", &duplicate));
    assert_eq!(finding.code, DiagnosticCode::DuplicateBinding);
    assert_eq!(finding.pointer, "/outcomes/1/outcome");
    assert_eq!(
        finding.message,
        "duplicate outcome \"success\" (first at ui/a.json/outcomes/0/outcome)"
    );

    // B26: the label spellings are exact inverses over the B18 grammar.
    for (wire, text) in [
        (OutcomeWire::CriticalSuccess, "critical_success"),
        (OutcomeWire::Success, "success"),
        (OutcomeWire::Failure, "failure"),
        (OutcomeWire::CriticalFailure, "critical_failure"),
        (OutcomeWire::Custom(0), "custom:0"),
        (OutcomeWire::Custom(255), "custom:255"),
    ] {
        assert_eq!(ui_outcome_text(wire), text);
        assert_eq!(ui_outcome(text), Some(wire));
    }
    for text in [
        "custom:",
        "custom:256",
        "custom:01",
        "custom:00",
        "Success",
        "",
    ] {
        assert_eq!(ui_outcome(text), None, "{text}");
    }
}

// ---------------------------------------------------------------------------
// U-13 — B14: the format × stat-kind matrix, meters and enum labels.
// ---------------------------------------------------------------------------

#[test]
fn u13_format_matrix() {
    let base = six_kind_campaign();
    let kinds = [
        ("might", "int"),
        ("guile", "fixed"),
        ("resolve", "bool"),
        ("ward", "enum"),
        ("talent", "tags"),
        ("lore", "dice"),
    ];
    let formats = [
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
            json!({"type": "enum", "value": [{"variant": "red", "label": SRD_KEY}]}),
        ),
        ("tags", json!({"type": "tags"})),
        ("dice", json!({"type": "dice"})),
    ];
    for (format, body) in &formats {
        for (stat, kind) in kinds {
            let allowed = match *format {
                "integer" | "signed" | "fraction" | "percent" => kind == "int" || kind == "fixed",
                "boolean" => kind == "bool",
                "enum" => kind == "enum",
                "tags" => kind == "tags",
                "dice" => kind == "dice",
                _ => false,
            };
            let mut value = ui_json();
            value["ruleset"] = json!(SRD_RULESET);
            value["stats"] = json!([{"stat": stat, "label": SRD_KEY, "format": body}]);
            let findings = check_ui(&base, "ui/a.json", &value);
            if allowed {
                assert_eq!(findings, Vec::new(), "{format} on {kind}");
            } else {
                let finding =
                    assert_only(&findings, DiagnosticCode::FormatMismatch, "/stats/0/format");
                assert_eq!(
                    finding.message,
                    format!("format {format} does not suit {kind} stat {stat:?}")
                );
                assert_eq!(
                    finding.suggested_fix.as_deref(),
                    Some("choose a format that suits the stat's kind")
                );
            }
        }
    }
}

#[test]
fn u13_fraction_meters_and_enum_variants() {
    let base = six_kind_campaign();
    let mut cases = Vec::new();

    let mut undeclared = ui_json();
    undeclared["ruleset"] = json!(SRD_RULESET);
    undeclared["stats"] = json!([{"stat": "might", "label": SRD_KEY,
        "format": {"type": "fraction", "value": {"type": "stat", "value": "nope"}}}]);
    cases.push((
        undeclared,
        DiagnosticCode::UnknownStat,
        "/stats/0/format/value/value",
        format!("unknown stat \"nope\" for ui: not declared by ruleset {SRD_RULESET}"),
    ));

    let mut bool_meter = ui_json();
    bool_meter["ruleset"] = json!(SRD_RULESET);
    bool_meter["stats"] = json!([{"stat": "might", "label": SRD_KEY,
        "format": {"type": "fraction", "value": {"type": "stat", "value": "resolve"}}}]);
    cases.push((
        bool_meter,
        DiagnosticCode::FormatMismatch,
        "/stats/0/format/value/value",
        "fraction meter stat \"resolve\" is bool, not int or fixed".into(),
    ));

    let mut pool_meter = ui_json();
    pool_meter["ruleset"] = json!(SRD_RULESET);
    pool_meter["stats"] = json!([{"stat": "might", "label": SRD_KEY,
        "format": {"type": "fraction", "value": {"type": "pool", "value": FOREIGN}}}]);
    cases.push((
        pool_meter,
        DiagnosticCode::UnknownPool,
        "/stats/0/format/value/value",
        format!("unknown pool {FOREIGN} for ui: not a template of ruleset {SRD_RULESET}"),
    ));

    let mut repeated = ui_json();
    repeated["ruleset"] = json!(SRD_RULESET);
    repeated["stats"] = json!([{"stat": "ward", "label": SRD_KEY,
        "format": {"type": "enum", "value": [
            {"variant": "red", "label": SRD_KEY},
            {"variant": "red", "label": SRD_KEY}]}}]);
    cases.push((
        repeated,
        DiagnosticCode::DuplicateBinding,
        "/stats/0/format/value/1/variant",
        "duplicate variant \"red\" (first at ui/a.json/stats/0/format/value/0/variant)".into(),
    ));

    let mut variant = ui_json();
    variant["ruleset"] = json!(SRD_RULESET);
    variant["stats"] = json!([{"stat": "ward", "label": SRD_KEY,
        "format": {"type": "enum", "value": [{"variant": "green", "label": SRD_KEY}]}}]);
    cases.push((
        variant,
        DiagnosticCode::UnknownVariant,
        "/stats/0/format/value/0/variant",
        "variant \"green\" is not declared by enum stat \"ward\"".into(),
    ));

    for (value, code, pointer, message) in cases {
        let finding = assert_only(&check_ui(&base, "ui/a.json", &value), code, pointer);
        assert_eq!(finding.message, message);
    }
}

// ---------------------------------------------------------------------------
// U-14 — B19–B21: screen and section rules.
// ---------------------------------------------------------------------------

#[test]
fn u14_screen_kinds_and_repeats() {
    let base = fixture("combat_basic");
    for kind in ["level_up", "character_creation"] {
        let mut value = ui_json();
        value["screens"] = json!([{"kind": kind, "sections": []}]);
        let finding = assert_only(
            &check_ui(&base, "ui/a.json", &value),
            DiagnosticCode::ReservedKind,
            "/screens/0/kind",
        );
        assert_eq!(
            finding.message,
            format!("screen kind {kind:?} is reserved until its data family exists")
        );
        assert_eq!(
            finding.suggested_fix.as_deref(),
            Some("remove it; the generated default omits reserved kinds")
        );
    }
    let mut choice = ui_json();
    choice["screens"] = json!([{"kind": "inventory", "sections": [{"type": "choice_steps"}]}]);
    let finding = assert_only(
        &check_ui(&base, "ui/a.json", &choice),
        DiagnosticCode::ReservedKind,
        "/screens/0/sections/0/type",
    );
    assert_eq!(
        finding.message,
        "section type \"choice_steps\" is reserved until its data family exists"
    );

    // A reserved screen's sections get no B20 check.
    let mut skipped = ui_json();
    skipped["screens"] = json!([{"kind": "level_up", "sections": [
        {"type": "stat_group", "value": {"type": "group", "value": "ghost"}}]}]);
    let findings = check_ui(&base, "ui/a.json", &skipped);
    assert_eq!(findings.len(), 1, "{findings:#?}");
    assert_eq!(findings[0].code, DiagnosticCode::ReservedKind);
    assert_no(
        &findings,
        DiagnosticCode::UnknownGroup,
        "/screens/0/sections/0/value/value",
    );

    // An unknown screen kind skips its sections too.
    let mut unknown = ui_json();
    unknown["screens"] = json!([{"kind": "journal", "sections": [
        {"type": "stat_group", "value": {"type": "group", "value": "ghost"}}]}]);
    let findings = check_ui(&base, "ui/a.json", &unknown);
    assert_eq!(findings.len(), 1, "{findings:#?}");
    assert_eq!(findings[0].code, DiagnosticCode::UnknownKind);
    assert_eq!(findings[0].pointer, "/screens/0/kind");
    assert_no(
        &findings,
        DiagnosticCode::UnknownGroup,
        "/screens/0/sections/0/value/value",
    );

    let mut repeated = ui_json();
    repeated["screens"] = json!([
        {"kind": "spellbook", "sections": []},
        {"kind": "spellbook", "sections": []}
    ]);
    let finding = only(&check_ui(&base, "ui/a.json", &repeated));
    assert_eq!(finding.code, DiagnosticCode::DuplicateBinding);
    assert_eq!(finding.pointer, "/screens/1/kind");
    assert_eq!(
        finding.message,
        "duplicate screen kind \"spellbook\" (first at ui/a.json/screens/0/kind)"
    );

    // The later duplicate's sections are still checked.
    let mut checked = ui_json();
    checked["screens"] = json!([
        {"kind": "spellbook", "sections": []},
        {"kind": "spellbook", "sections": [{"type": "gizmo"}]}
    ]);
    let findings = check_ui(&base, "ui/a.json", &checked);
    assert_eq!(findings.len(), 2, "{findings:#?}");
    assert!(findings
        .iter()
        .any(|d| d.code == DiagnosticCode::DuplicateBinding && d.pointer == "/screens/1/kind"));
    assert!(findings.iter().any(
        |d| d.code == DiagnosticCode::UnknownKind && d.pointer == "/screens/1/sections/0/type"
    ));

    // Reserved kinds never join the duplicate rule.
    let mut two_reserved = ui_json();
    two_reserved["screens"] = json!([
        {"kind": "level_up", "sections": []},
        {"kind": "level_up", "sections": []}
    ]);
    let findings = check_ui(&base, "ui/a.json", &two_reserved);
    assert_eq!(findings.len(), 2, "{findings:#?}");
    assert!(findings
        .iter()
        .all(|d| d.code == DiagnosticCode::ReservedKind));

    // B21: no screen–section compatibility rule.
    let mut mixed = ui_json();
    mixed["screens"] = json!([{"kind": "inventory", "sections": [{"type": "timeline"}]}]);
    assert_eq!(check_ui(&base, "ui/a.json", &mixed), Vec::new());
}

#[test]
fn u12_groups_names_pools_and_tags() {
    let base = fixture("combat_basic");
    let group = |sections: Value| {
        let mut value = ui_json();
        value["screens"] = json!([{"kind": "character_sheet", "sections": sections}]);
        value
    };

    let unresolved = group(json!([
        {"type": "stat_group", "value": {"type": "group", "value": "ghost"}}]));
    let finding = only(&check_ui(&base, "ui/a.json", &unresolved));
    assert_eq!(finding.code, DiagnosticCode::UnknownGroup);
    assert_eq!(finding.pointer, "/screens/0/sections/0/value/value");
    assert_eq!(
        finding.message,
        "stat group \"ghost\" is the group of no stats entry"
    );
    assert_eq!(
        finding.suggested_fix.as_deref(),
        Some("set the group on a stats entry or fix the selector")
    );
    let mut resolved = group(json!([
        {"type": "stat_group", "value": {"type": "group", "value": KEY}}]));
    resolved["stats"] = json!([{"stat": "might", "label": KEY, "group": KEY}]);
    assert_eq!(check_ui(&base, "ui/a.json", &resolved), Vec::new());

    let declared = group(json!([
        {"type": "stat_group", "value": {"type": "names", "value": ["might"]}}]));
    assert_eq!(check_ui(&base, "ui/a.json", &declared), Vec::new());

    let unknown_name = group(json!([
        {"type": "stat_group", "value": {"type": "names", "value": ["nope"]}}]));
    let finding = only(&check_ui(&base, "ui/a.json", &unknown_name));
    assert_eq!(finding.code, DiagnosticCode::UnknownStat);
    assert_eq!(finding.pointer, "/screens/0/sections/0/value/value/0");
    assert_eq!(
        finding.message,
        format!("unknown stat \"nope\" for ui: not declared by ruleset {BASIC_RULESET}")
    );

    let repeated_name = group(json!([
        {"type": "stat_group", "value": {"type": "names", "value": ["might"]}},
        {"type": "stat_group", "value": {"type": "names", "value": ["might"]}}]));
    let finding = only(&check_ui(&base, "ui/a.json", &repeated_name));
    assert_eq!(finding.code, DiagnosticCode::DuplicateBinding);
    assert_eq!(finding.pointer, "/screens/0/sections/1/value/value/0");
    assert_eq!(
        finding.message,
        "duplicate stat \"might\" (first at ui/a.json/screens/0/sections/0/value/value/0)"
    );

    // `all` and a `names` selector may overlap.
    let overlap = group(json!([
        {"type": "stat_group", "value": {"type": "all"}},
        {"type": "stat_group", "value": {"type": "names", "value": ["might"]}}]));
    assert_eq!(check_ui(&base, "ui/a.json", &overlap), Vec::new());

    let repeated_pool = group(json!([
        {"type": "pool_group", "value": {"type": "ids", "value": [BASIC_POOL]}},
        {"type": "pool_group", "value": {"type": "ids", "value": [BASIC_POOL]}}]));
    let finding = only(&check_ui(&base, "ui/a.json", &repeated_pool));
    assert_eq!(finding.code, DiagnosticCode::DuplicateBinding);
    assert_eq!(finding.pointer, "/screens/0/sections/1/value/value/0");
    assert_eq!(
        finding.message,
        format!(
            "duplicate pool {BASIC_POOL} (first at ui/a.json/screens/0/sections/0/value/value/0)"
        )
    );

    let foreign_pool = group(json!([
        {"type": "pool_group", "value": {"type": "ids", "value": [FOREIGN]}}]));
    let finding = only(&check_ui(&base, "ui/a.json", &foreign_pool));
    assert_eq!(finding.code, DiagnosticCode::UnknownPool);
    assert_eq!(finding.pointer, "/screens/0/sections/0/value/value/0");

    // The same pool on two screens is not a repeat within one screen.
    let mut two_screens = ui_json();
    two_screens["screens"] = json!([
        {"kind": "combat_hud", "sections": [{"type": "pool_group",
            "value": {"type": "ids", "value": [BASIC_POOL]}}]},
        {"kind": "character_sheet", "sections": [{"type": "pool_group",
            "value": {"type": "ids", "value": [BASIC_POOL]}}]}
    ]);
    assert_eq!(check_ui(&base, "ui/a.json", &two_screens), Vec::new());

    let repeated_ability = group(json!([
        {"type": "ability_list", "value": {"select": {"type": "ids", "value": [BASIC_ABILITY]}}},
        {"type": "ability_list", "value": {"select": {"type": "ids", "value": [BASIC_ABILITY]}}}]));
    let finding = only(&check_ui(&base, "ui/a.json", &repeated_ability));
    assert_eq!(finding.code, DiagnosticCode::DuplicateBinding);
    assert_eq!(
        finding.pointer,
        "/screens/0/sections/1/value/select/value/0"
    );
    assert_eq!(
        finding.message,
        format!("duplicate ability {BASIC_ABILITY} (first at ui/a.json/screens/0/sections/0/value/select/value/0)")
    );

    let tag_unresolved = group(json!([
        {"type": "ability_list", "value": {"select": {"type": "tag", "value": "melee"}}}]));
    let finding = only(&check_ui(&base, "ui/a.json", &tag_unresolved));
    assert_eq!(finding.code, DiagnosticCode::UnknownTag);
    assert_eq!(finding.pointer, "/screens/0/sections/0/value/select/value");
    assert_eq!(
        finding.message,
        "ability tag \"melee\" is declared by no abilities entry"
    );
    let mut tag_resolved = tag_unresolved.clone();
    tag_resolved["abilities"] = json!([{"ability": BASIC_ABILITY, "label": KEY,
        "category_tags": ["melee"]}]);
    let tag_resolved = tag_resolved;
    assert_eq!(check_ui(&base, "ui/a.json", &tag_resolved), Vec::new());

    // A screen-local repeat of a dangling ability id stays two generic
    // findings: the generic pass wins, and no duplicate is added.
    let dangling_repeat = group(json!([
        {"type": "ability_list", "value": {"select": {"type": "ids", "value": [FOREIGN]}}},
        {"type": "ability_list", "value": {"select": {"type": "ids", "value": [FOREIGN]}}}]));
    let findings = check_ui(&base, "ui/a.json", &dangling_repeat);
    assert_eq!(findings.len(), 2, "{findings:#?}");
    assert!(findings
        .iter()
        .all(|d| d.code == DiagnosticCode::DanglingReference));

    // An item tag resolves in the campaign and is only a warning.
    let mut item = ui_json();
    item["screens"] = json!([{"kind": "inventory", "sections": [
        {"type": "item_list", "value": {"type": "tag", "value": "rare"}}]}]);
    let finding = only(&check_ui(&base, "ui/a.json", &item));
    assert_eq!(finding.code, DiagnosticCode::UnknownTag);
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.pointer, "/screens/0/sections/0/value/value");
    assert_eq!(
        finding.message,
        "item tag \"rare\" is carried by no item in this campaign"
    );
    assert_eq!(
        finding.suggested_fix.as_deref(),
        Some("add the tag to an item or fix the filter")
    );
    let item_doc = json!({
        "schema": "crpg.item/2", "id": "00000000000000000000000030",
        "slug": "rare", "name": KEY, "stats": {}, "tags": ["rare"]
    });
    let with_item = add_json(&base, "items/rare.json", &item_doc);
    assert_eq!(check_ui(&with_item, "ui/a.json", &item), Vec::new());
}

// ---------------------------------------------------------------------------
// U-15 — B22, B23: locale coverage and asset keys.
// ---------------------------------------------------------------------------

fn locale_doc(campaign: &str, locale: &str, strings: &[(&str, &str)]) -> Value {
    let map: serde_json::Map<String, Value> = strings
        .iter()
        .map(|(key, text)| ((*key).to_string(), json!(text)))
        .collect();
    json!({"schema": "crpg.locale/1", "campaign": campaign, "locale": locale,
        "strings": Value::Object(map)})
}

fn locale_ui() -> Value {
    json!({
        "schema": "crpg.ui/1", "ruleset": SRD_RULESET,
        "stats": [{"stat": "ward", "label": "k1", "short_label": "k2",
            "description": "k3", "group": "k4",
            "format": {"type": "enum", "value": [
                {"variant": "red", "label": "k5"},
                {"variant": "blue", "label": "k6"}]}}],
        "pools": [{"pool": SRD_POOL_1, "label": "k7"}],
        "abilities": [{"ability": SRD_HEAVY, "label": "k8", "description": "k9"}],
        "effects": [{"effect": SRD_EFFECT, "label": "k10"}],
        "outcomes": [{"outcome": "success", "label": "k11"}]
    })
}

fn all_locale_keys() -> Vec<(&'static str, &'static str)> {
    vec![
        ("k1", "v1"),
        ("k2", "v2"),
        ("k3", "v3"),
        ("k4", "v4"),
        ("k5", "v5"),
        ("k6", "v6"),
        ("k7", "v7"),
        ("k8", "v8"),
        ("k9", "v9"),
        ("k10", "v10"),
        ("k11", "v11"),
    ]
}

#[test]
fn u15_locale_coverage() {
    let mut base = six_kind_campaign();
    base.remove(&path("locale/en.json"));
    let findings = check_ui(&base, "ui/a.json", &locale_ui());
    let mut pointers: Vec<&str> = findings.iter().map(|d| d.pointer.as_str()).collect();
    pointers.sort_unstable();
    assert_eq!(
        pointers,
        vec![
            "/abilities/0/description",
            "/abilities/0/label",
            "/effects/0/label",
            "/outcomes/0/label",
            "/pools/0/label",
            "/stats/0/description",
            "/stats/0/format/value/0/label",
            "/stats/0/format/value/1/label",
            "/stats/0/group",
            "/stats/0/label",
            "/stats/0/short_label",
        ],
        "{findings:#?}"
    );
    assert!(findings
        .iter()
        .all(|d| d.code == DiagnosticCode::MissingLocaleKey && d.severity == Severity::Error));
    // The no-table case uses the existing message for a campaign with no table.
    assert!(
        findings
            .iter()
            .all(|d| d.message == "locale key has no locale table to resolve against"),
        "{findings:#?}"
    );

    // With two tables, a key missing from one is reported for that table only.
    let all = all_locale_keys();
    let en = locale_doc(SRD_CAMPAIGN, "en", &all);
    let without_group: Vec<(&str, &str)> = all
        .iter()
        .copied()
        .filter(|(key, _)| *key != "k4")
        .collect();
    let fr = locale_doc(SRD_CAMPAIGN, "fr", &without_group);
    let two = add_json(
        &add_json(&base, "locale/en.json", &en),
        "locale/fr.json",
        &fr,
    );
    let findings = check_ui(&two, "ui/a.json", &locale_ui());
    let finding = only(&findings);
    assert_eq!(finding.code, DiagnosticCode::MissingLocaleKey);
    assert_eq!(finding.pointer, "/stats/0/group");
    assert_eq!(
        finding.message,
        "locale key \"k4\" is missing from locale \"fr\""
    );
    // Control: the complete table alone reports nothing.
    let en_only = add_json(&base, "locale/en.json", &en);
    assert_eq!(check_ui(&en_only, "ui/a.json", &locale_ui()), Vec::new());
}

#[test]
fn u15_asset_keys() {
    let base = fixture("combat_srd");
    let keys = [
        "assets/ui/pool.png",
        "assets/ui/pool2.png",
        "assets/ui/heavy.png",
        "assets/ui/focus.png",
    ];
    let mut ui = ui_json();
    ui["ruleset"] = json!(SRD_RULESET);
    ui["pools"] = json!([
        {"pool": SRD_POOL_1, "label": SRD_KEY, "icon": keys[0]},
        {"pool": SRD_POOL_2, "label": SRD_KEY, "icon": keys[1]}
    ]);
    ui["abilities"] = json!([{"ability": SRD_HEAVY, "label": SRD_KEY, "icon": keys[2]}]);
    ui["effects"] = json!([{"effect": SRD_EFFECT, "label": SRD_KEY, "icon": keys[3]}]);
    let findings = check_ui(&base, "ui/a.json", &ui);
    let pointers: Vec<&str> = findings.iter().map(|d| d.pointer.as_str()).collect();
    assert_eq!(
        pointers,
        vec![
            "/abilities/0/icon",
            "/effects/0/icon",
            "/pools/0/icon",
            "/pools/1/icon",
        ],
        "{findings:#?}"
    );
    for finding in &findings {
        assert_eq!(finding.code, DiagnosticCode::MissingAsset);
        let index = ["abilities/0", "effects/0", "pools/0", "pools/1"]
            .iter()
            .position(|prefix| finding.pointer == format!("/{prefix}/icon"))
            .map(|position| [2, 3, 0, 1][position])
            .unwrap();
        assert_eq!(
            finding.message,
            format!("asset key \"{}\" is not in assets.lock", keys[index])
        );
        assert_eq!(
            finding.suggested_fix.as_deref(),
            Some("add the asset to assets.lock or fix the key")
        );
    }
    let mut with_assets = base.clone();
    add_assets(&mut with_assets, &keys);
    assert_eq!(check_ui(&with_assets, "ui/a.json", &ui), Vec::new());
}

// ---------------------------------------------------------------------------
// U-16 — B24: one finding per pointer, sorted and independent of insertion.
// ---------------------------------------------------------------------------

#[test]
fn u16_order_one_finding_per_pointer_and_determinism() {
    let base = fixture("combat_basic");
    let mut ui = ui_json();
    ui["stats"] = json!([
        {"stat": "might", "label": KEY},
        {"stat": "might", "label": KEY}
    ]);
    ui["pools"] = json!([
        {"pool": FOREIGN, "label": KEY},
        {"pool": FOREIGN, "label": KEY}
    ]);
    ui["abilities"] = json!([
        {"ability": FOREIGN, "label": KEY},
        {"ability": FOREIGN, "label": KEY}
    ]);
    ui["outcomes"] = json!([
        {"outcome": "bogus", "label": KEY},
        {"outcome": "bogus", "label": KEY}
    ]);
    let findings = check_ui(&base, "ui/a.json", &ui);
    let expected = [
        ("/abilities/0/ability", DiagnosticCode::DanglingReference),
        ("/abilities/1/ability", DiagnosticCode::DanglingReference),
        ("/outcomes/0/outcome", DiagnosticCode::UnknownOutcome),
        ("/outcomes/1/outcome", DiagnosticCode::DuplicateBinding),
        ("/pools/0/pool", DiagnosticCode::UnknownPool),
        ("/pools/1/pool", DiagnosticCode::DuplicateBinding),
        ("/stats/1/stat", DiagnosticCode::DuplicateBinding),
    ];
    assert_eq!(findings.len(), expected.len(), "{findings:#?}");
    for (finding, (pointer, code)) in findings.iter().zip(expected) {
        assert_eq!(finding.pointer, pointer);
        assert_eq!(finding.code, code);
    }
    assert_eq!(sorted(&findings), findings);

    // `Files` is a `BTreeMap`, so a reversed insertion order is not a
    // distinct input and is not tested. A second run, and `validate_files`,
    // must give byte-identical output instead.
    let files = add_ui(&base, "ui/a.json", &ui);
    let first = validate(&load(&files));
    let second = validate(&load(&files));
    assert_eq!(format!("{first:?}"), format!("{second:?}"));
    assert_eq!(
        format!("{first:?}"),
        format!("{:?}", validate_files(&files, &engine()))
    );
    let ours: Vec<Diagnostic> = first
        .into_iter()
        .filter(|d| d.file.as_ref().map(SourcePath::as_str) == Some("ui/a.json"))
        .collect();
    assert_eq!(format!("{ours:?}"), format!("{findings:?}"));
}

// ---------------------------------------------------------------------------
// U-17 — B25–B29: the generated defaults, exactly.
// ---------------------------------------------------------------------------

fn basic_literal() -> Value {
    json!({
        "abilities": [{"ability": BASIC_ABILITY, "label": "fixture.strike",
            "target_hint": "enemy"}],
        "effects": [],
        "outcomes": [
            {"label": "success", "outcome": "success"},
            {"label": "failure", "outcome": "failure"}
        ],
        "pools": [{"label": "pool-1", "pool": BASIC_POOL}],
        "ruleset": BASIC_RULESET,
        "schema": "crpg.ui/1",
        "screens": [
            {"kind": "character_sheet",
             "sections": [{"type": "stat_group", "value": {"type": "all"}}]},
            {"kind": "action_bar",
             "sections": [{"type": "ability_list", "value": {"select": {"type": "all"}}}]},
            {"kind": "spellbook", "sections": [{"type": "ability_list",
                "value": {"select": {"type": "all"}, "sort": "label"}}]},
            {"kind": "inventory",
             "sections": [{"type": "item_list", "value": {"type": "all"}}]},
            {"kind": "combat_hud", "sections": [
                {"type": "stat_group", "value": {"type": "names", "value": ["health"]}},
                {"type": "pool_group", "value": {"type": "all"}},
                {"type": "timeline"}, {"type": "log"}]}
        ],
        "stats": [
            {"label": "might", "stat": "might"},
            {"label": "guile", "stat": "guile"},
            {"label": "resolve", "stat": "resolve"},
            {"label": "health", "stat": "health"}
        ]
    })
}

fn srd_literal() -> Value {
    json!({
        "abilities": [
            {"ability": SRD_HEAVY, "label": "fixture.heavy", "target_hint": "enemy"},
            {"ability": SRD_FOCUS, "label": "fixture.focus", "target_hint": "any"}
        ],
        "effects": [{"effect": SRD_EFFECT, "label": "fixture.focusing"}],
        "outcomes": [
            {"label": "failure", "outcome": "failure"},
            {"label": "success", "outcome": "success"},
            {"label": "critical_success", "outcome": "critical_success"}
        ],
        "pools": [
            {"label": "pool-1", "pool": SRD_POOL_1},
            {"label": "pool-2", "pool": SRD_POOL_2}
        ],
        "ruleset": SRD_RULESET,
        "schema": "crpg.ui/1",
        "screens": [
            {"kind": "character_sheet",
             "sections": [{"type": "stat_group", "value": {"type": "all"}}]},
            {"kind": "action_bar",
             "sections": [{"type": "ability_list", "value": {"select": {"type": "all"}}}]},
            {"kind": "spellbook", "sections": [{"type": "ability_list",
                "value": {"select": {"type": "all"}, "sort": "label"}}]},
            {"kind": "inventory",
             "sections": [{"type": "item_list", "value": {"type": "all"}}]},
            {"kind": "combat_hud", "sections": [
                {"type": "stat_group", "value": {"type": "names", "value": ["health"]}},
                {"type": "pool_group", "value": {"type": "all"}},
                {"type": "timeline"}, {"type": "log"}]}
        ],
        "stats": [
            {"label": "might", "stat": "might"},
            {"label": "guile", "stat": "guile"},
            {"label": "resolve", "stat": "resolve"},
            {"label": "ward", "stat": "ward"},
            {"label": "health", "stat": "health"}
        ]
    })
}

#[test]
fn u17_generated_ui_literals() {
    let basic = load(&fixture("combat_basic"));
    let source = UiSource::from_documents(&basic.documents, ulid(BASIC_RULESET)).unwrap();
    assert_eq!(source.ruleset.id, ulid(BASIC_RULESET));
    let ui = generated_ui(&source);
    assert_eq!(written(&Document::Ui(ui.clone())), basic_literal());
    assert!(ui.note.is_none());

    let srd = load(&fixture("combat_srd"));
    let source = UiSource::from_documents(&srd.documents, ulid(SRD_RULESET)).unwrap();
    let ui = generated_ui(&source);
    assert_eq!(written(&Document::Ui(ui.clone())), srd_literal());
}

#[test]
fn u17_generated_ui_options_and_order() {
    let srd = load(&fixture("combat_srd"));
    let source = UiSource::from_documents(&srd.documents, ulid(SRD_RULESET)).unwrap();
    let ui = generated_ui(&source);

    let stats = ui.stats.as_ref().unwrap();
    assert_eq!(
        stats.iter().map(|s| s.stat.as_str()).collect::<Vec<_>>(),
        vec!["might", "guile", "resolve", "ward", "health"]
    );
    for stat in stats {
        assert_eq!(stat.stat, stat.label);
        assert!(stat.short_label.is_none());
        assert!(stat.description.is_none());
        assert!(stat.group.is_none());
        assert!(stat.format.is_none());
    }
    let pools = ui.pools.as_ref().unwrap();
    assert_eq!(
        pools.iter().map(|p| p.label.as_str()).collect::<Vec<_>>(),
        vec!["pool-1", "pool-2"]
    );
    for pool in pools {
        assert!(pool.icon.is_none());
        assert!(pool.style.is_none());
        assert!(pool.show_on_hud.is_none());
    }
    let abilities = ui.abilities.as_ref().unwrap();
    assert_eq!(
        abilities
            .iter()
            .map(|a| a.label.as_str())
            .collect::<Vec<_>>(),
        vec!["fixture.heavy", "fixture.focus"]
    );
    for ability in abilities {
        assert!(ability.description.is_none());
        assert!(ability.icon.is_none());
        assert!(ability.category_tags.is_none());
        assert!(ability.show_odds.is_none());
    }
    for effect in ui.effects.as_ref().unwrap() {
        assert!(effect.icon.is_none());
        assert!(effect.show_duration.is_none());
    }
    for outcome in ui.outcomes.as_ref().unwrap() {
        assert_eq!(outcome.label, outcome.outcome);
    }

    // B28: the default screens in order, and nothing reserved or unknown.
    let screens = ui.screens.as_ref().unwrap();
    let kinds: Vec<&str> = screens.iter().map(|s| s.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            "character_sheet",
            "action_bar",
            "spellbook",
            "inventory",
            "combat_hud"
        ]
    );
    assert_eq!(
        screens[2].sections,
        vec![UiSection::AbilityList(UiAbilityList {
            select: UiAbilitySelect::All,
            sort: Some(UiSort::Label),
        })]
    );
    assert_eq!(
        screens[4].sections,
        vec![
            UiSection::StatGroup(UiStatSelect::Names(vec!["health".into()])),
            UiSection::PoolGroup(UiPoolSelect::All),
            UiSection::Timeline,
            UiSection::Log,
        ]
    );
    for screen in screens {
        assert!(!matches!(
            screen.kind,
            UiScreenKind::LevelUp | UiScreenKind::CharacterCreation | UiScreenKind::Unknown(_)
        ));
        assert!(!screen
            .sections
            .iter()
            .any(|section| matches!(section, UiSection::EffectList | UiSection::Unknown { .. })));
    }
}

#[test]
fn u17_target_hint_rule() {
    let srd = load(&fixture("combat_srd"));
    let source = UiSource::from_documents(&srd.documents, ulid(SRD_RULESET)).unwrap();
    let ui = generated_ui(&source);
    let hints: Vec<UiTargetHint> = ui
        .abilities
        .as_ref()
        .unwrap()
        .iter()
        .map(|a| a.target_hint.clone().unwrap())
        .collect();
    assert_eq!(hints, vec![UiTargetHint::Enemy, UiTargetHint::Any]);

    let mut files = fixture("combat_srd");
    let mut heavy: Value = serde_json::from_slice(&files[&path("abilities/heavy.json")]).unwrap();
    heavy["requires_target"] = json!(false);
    files.insert(path("abilities/heavy.json"), canonical(&heavy));
    let loaded = load(&files);
    let source = UiSource::from_documents(&loaded.documents, ulid(SRD_RULESET)).unwrap();
    let ui = generated_ui(&source);
    assert_eq!(
        ui.abilities.unwrap()[0].target_hint,
        Some(UiTargetHint::NoTarget)
    );
}

// ---------------------------------------------------------------------------
// U-17 — B25, B26: source resolution order and outcome labels.
// ---------------------------------------------------------------------------

#[test]
fn u17_source_and_outcome_order() {
    let srd = load(&fixture("combat_srd"));
    let source = UiSource::from_documents(&srd.documents, ulid(SRD_RULESET)).unwrap();
    assert_eq!(source.ruleset.id, ulid(SRD_RULESET));
    assert_eq!(
        source
            .abilities
            .iter()
            .map(|a| a.id.to_string())
            .collect::<Vec<_>>(),
        vec![SRD_HEAVY, SRD_FOCUS]
    );
    assert_eq!(
        source
            .effects
            .iter()
            .map(|e| e.id.to_string())
            .collect::<Vec<_>>(),
        vec![SRD_EFFECT]
    );
    assert_eq!(
        source
            .tables
            .iter()
            .map(|t| t.id.to_string())
            .collect::<Vec<_>>(),
        vec![SRD_HEAVY_TABLE, SRD_FOCUS_TABLE]
    );
    assert_eq!(
        source.outcome_labels(),
        vec!["failure", "success", "critical_success"]
    );
    assert!(UiSource::from_documents(&srd.documents, ulid(FOREIGN)).is_none());

    // Unresolved ids are skipped, never invented.
    let mut files = fixture("combat_srd");
    files.remove(&path("abilities/heavy.json"));
    let loaded = load(&files);
    let source = UiSource::from_documents(&loaded.documents, ulid(SRD_RULESET)).unwrap();
    assert_eq!(
        source
            .abilities
            .iter()
            .map(|a| a.id.to_string())
            .collect::<Vec<_>>(),
        vec![SRD_FOCUS]
    );
    assert_eq!(
        source
            .tables
            .iter()
            .map(|t| t.id.to_string())
            .collect::<Vec<_>>(),
        vec![SRD_FOCUS_TABLE]
    );

    let basic = load(&fixture("combat_basic"));
    let source = UiSource::from_documents(&basic.documents, ulid(BASIC_RULESET)).unwrap();
    assert_eq!(source.outcome_labels(), vec!["success", "failure"]);
}

// ---------------------------------------------------------------------------
// U-18 — B30: `effective_ui` merges without dropping or reordering.
// ---------------------------------------------------------------------------

#[test]
fn u18_effective_ui_merge() {
    let basic = load(&fixture("combat_basic"));
    let source = UiSource::from_documents(&basic.documents, ulid(BASIC_RULESET)).unwrap();
    assert_eq!(effective_ui(None, &source), generated_ui(&source));

    let authored = UiDocument {
        stats: Some(vec![stat_entry("health")]),
        screens: Some(vec![screen(UiScreenKind::ActionBar)]),
        ..empty_ui()
    };
    let merged = effective_ui(Some(&authored), &source);
    assert_eq!(merged.ruleset, ulid(BASIC_RULESET));
    assert_eq!(
        merged
            .stats
            .as_ref()
            .unwrap()
            .iter()
            .map(|s| s.stat.as_str())
            .collect::<Vec<_>>(),
        vec!["health", "might", "guile", "resolve"]
    );
    assert_eq!(
        merged
            .screens
            .as_ref()
            .unwrap()
            .iter()
            .map(|s| s.kind.as_str())
            .collect::<Vec<_>>(),
        vec![
            "action_bar",
            "character_sheet",
            "spellbook",
            "inventory",
            "combat_hud"
        ]
    );

    // Authored reserved screens and undeclared entries are kept verbatim.
    let extra = UiDocument {
        stats: Some(vec![stat_entry("mystery")]),
        screens: Some(vec![screen(UiScreenKind::LevelUp)]),
        note: Some("kept authored data".into()),
        ..empty_ui()
    };
    let merged = effective_ui(Some(&extra), &source);
    assert_eq!(merged.stats.as_ref().unwrap()[0], stat_entry("mystery"));
    assert_eq!(
        merged.screens.as_ref().unwrap()[0].kind,
        UiScreenKind::LevelUp
    );
    assert_eq!(merged.screens.as_ref().unwrap().len(), 6);
    assert!(merged.note.is_none());

    // Control: a fully authored document keeps every authored entry; only
    // `ruleset` is set and `note` is dropped.
    let full = UiDocument {
        ruleset: ulid(BASIC_CAMPAIGN),
        stats: Some(vec![
            stat_entry("might"),
            stat_entry("guile"),
            stat_entry("resolve"),
            stat_entry("health"),
        ]),
        pools: Some(vec![UiPool {
            pool: ulid(BASIC_POOL),
            label: KEY.into(),
            icon: None,
            style: None,
            show_on_hud: None,
        }]),
        abilities: Some(vec![UiAbility {
            ability: ulid(BASIC_ABILITY),
            label: KEY.into(),
            description: None,
            icon: None,
            category_tags: None,
            target_hint: Some(UiTargetHint::Enemy),
            show_odds: None,
        }]),
        effects: Some(Vec::new()),
        outcomes: Some(vec![
            UiOutcome {
                outcome: "success".into(),
                label: KEY.into(),
            },
            UiOutcome {
                outcome: "failure".into(),
                label: KEY.into(),
            },
        ]),
        screens: Some(vec![
            screen(UiScreenKind::CharacterSheet),
            screen(UiScreenKind::ActionBar),
            screen(UiScreenKind::Spellbook),
            screen(UiScreenKind::Inventory),
            screen(UiScreenKind::CombatHud),
        ]),
        note: Some("dropped".into()),
    };
    let merged = effective_ui(Some(&full), &source);
    assert_eq!(merged.ruleset, ulid(BASIC_RULESET));
    assert!(merged.note.is_none());
    assert_eq!(merged.stats, full.stats);
    assert_eq!(merged.pools, full.pools);
    assert_eq!(merged.abilities, full.abilities);
    assert_eq!(merged.effects, full.effects);
    assert_eq!(merged.outcomes, full.outcomes);
    assert_eq!(merged.screens, full.screens);

    // A present empty list is kept and completed, never treated as absent.
    let emptied = UiDocument {
        stats: Some(Vec::new()),
        ..empty_ui()
    };
    let merged = effective_ui(Some(&emptied), &source);
    assert_eq!(merged.stats.as_ref().unwrap().len(), 4);
}

// ---------------------------------------------------------------------------
// U-19 — B31: the generated default validates inside the user's bound.
// ---------------------------------------------------------------------------

#[test]
fn u19_defaults_validate() {
    let cases: Vec<(&str, &str, Vec<&str>)> = vec![
        (
            "combat_basic",
            BASIC_RULESET,
            vec![
                "/outcomes/0/label",
                "/outcomes/1/label",
                "/pools/0/label",
                "/stats/0/label",
                "/stats/1/label",
                "/stats/2/label",
                "/stats/3/label",
            ],
        ),
        (
            "combat_srd",
            SRD_RULESET,
            vec![
                "/outcomes/0/label",
                "/outcomes/1/label",
                "/outcomes/2/label",
                "/pools/0/label",
                "/pools/1/label",
                "/stats/0/label",
                "/stats/1/label",
                "/stats/2/label",
                "/stats/3/label",
                "/stats/4/label",
            ],
        ),
    ];
    for (fixture_name, ruleset, expected) in cases {
        let base = fixture(fixture_name);
        let loaded = load(&base);
        let source = UiSource::from_documents(&loaded.documents, ulid(ruleset)).unwrap();
        let ui = generated_ui(&source);
        let value = written(&Document::Ui(ui.clone()));
        // Round trips through the writer and the reader.
        assert_eq!(written(&read_ok(&value)), value);
        let findings = check_ui(&base, "ui/default.json", &value);
        let mut pointers: Vec<&str> = findings.iter().map(|d| d.pointer.as_str()).collect();
        pointers.sort_unstable();
        assert_eq!(pointers, expected, "{fixture_name}: {findings:#?}");
        for finding in &findings {
            assert_eq!(finding.code, DiagnosticCode::MissingLocaleKey);
            assert_eq!(finding.severity, Severity::Error);
            assert!(
                !finding.pointer.contains("/abilities/") && !finding.pointer.contains("/effects/"),
                "{finding:?}"
            );
        }
        let files = add_ui(&base, "ui/default.json", &value);
        assert_eq!(validate_files(&files, &engine()), validate(&load(&files)));
    }
}

#[test]
fn u19_effective_ui_respects_the_bound() {
    let base = fixture("combat_basic");
    let loaded = load(&base);
    let source = UiSource::from_documents(&loaded.documents, ulid(BASIC_RULESET)).unwrap();

    // 59 authored screens plus the five defaults is exactly the 64 cap.
    let at_cap = UiDocument {
        screens: Some(
            (0..59)
                .map(|_| screen(UiScreenKind::Unknown("journal".into())))
                .collect(),
        ),
        ..empty_ui()
    };
    let merged = effective_ui(Some(&at_cap), &source);
    assert_eq!(merged.screens.as_ref().unwrap().len(), 64);
    assert!(write_document(&Document::Ui(merged)).is_ok());

    let over_cap = UiDocument {
        screens: Some(
            (0..60)
                .map(|_| screen(UiScreenKind::Unknown("journal".into())))
                .collect(),
        ),
        ..empty_ui()
    };
    let merged = effective_ui(Some(&over_cap), &source);
    match write_document(&Document::Ui(merged)) {
        Err(DataError::Malformed { message, .. }) => {
            assert_eq!(message, "ui list at /screens exceeds 64 entries")
        }
        other => panic!("expected Malformed, got {other:?}"),
    }

    // Any locally accepted authored document round-trips through effective_ui.
    let accepted = UiDocument {
        stats: Some(vec![stat_entry("mystery")]),
        screens: Some(vec![screen(UiScreenKind::Unknown("journal".into()))]),
        ..empty_ui()
    };
    let merged = effective_ui(Some(&accepted), &source);
    let value = written(&Document::Ui(merged));
    assert_eq!(written(&read_ok(&value)), value);
}

// ---------------------------------------------------------------------------
// U-20 — B32, B33: no numeric types; the tolerant sets are pinned.
// ---------------------------------------------------------------------------

fn walk_json(value: &Value, visit: &mut dyn FnMut(&Value)) {
    visit(value);
    match value {
        Value::Object(map) => {
            for item in map.values() {
                walk_json(item, visit);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_json(item, visit);
            }
        }
        _ => {}
    }
}

fn ui_schema() -> Value {
    let schemas = generated_schemas().unwrap();
    serde_json::from_slice(&schemas["ui.schema.json"]).unwrap()
}

fn string_list(value: &Value) -> Option<Vec<String>> {
    let items = value.as_array()?;
    if items.is_empty() || !items.iter().all(Value::is_string) {
        return None;
    }
    Some(
        items
            .iter()
            .map(|item| item.as_str().unwrap().to_string())
            .collect(),
    )
}

#[test]
fn u20_no_numbers_and_pinned_kinds() {
    let schema = ui_schema();
    let mut numeric = Vec::new();
    walk_json(&schema, &mut |value| {
        let Some(object) = value.as_object() else {
            return;
        };
        let Some(kind) = object.get("type") else {
            return;
        };
        match kind {
            Value::String(text) if text == "integer" || text == "number" => {
                numeric.push(text.clone());
            }
            Value::Array(items) => {
                for item in items {
                    if let Some(text) = item.as_str() {
                        if text == "integer" || text == "number" {
                            numeric.push(text.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    });
    assert!(numeric.is_empty(), "numeric schema types: {numeric:?}");

    // The constants equal literal arrays written here, not each other.
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

    // Schema string `enum` lists equal the four plain-string constants.
    let mut enums = Vec::new();
    walk_json(&schema, &mut |value| {
        if let Some(list) = value
            .as_object()
            .and_then(|object| object.get("enum"))
            .and_then(string_list)
        {
            enums.push(list);
        }
    });
    let mut expected_enums = vec![
        UI_SCREEN_KINDS.map(str::to_string).to_vec(),
        UI_POOL_STYLES.map(str::to_string).to_vec(),
        UI_TARGET_HINTS.map(str::to_string).to_vec(),
        UI_SORTS.map(str::to_string).to_vec(),
    ];
    expected_enums.sort();
    enums.sort();
    assert_eq!(enums, expected_enums);

    // Section and format `const` values equal their constants.
    let mut consts = Vec::new();
    walk_json(&schema, &mut |value| {
        if let Some(text) = value
            .as_object()
            .and_then(|object| object.get("const"))
            .and_then(Value::as_str)
        {
            consts.push(text.to_string());
        }
    });
    for (constant, name) in [(&UI_SECTION_TYPES, "section"), (&UI_FORMAT_TYPES, "format")] {
        let mut found: Vec<String> = consts
            .iter()
            .filter(|text| constant.contains(&text.as_str()))
            .cloned()
            .collect();
        let mut expected: Vec<String> = constant.iter().map(|s| (*s).to_string()).collect();
        found.sort();
        expected.sort();
        assert_eq!(found, expected, "{name} const set");
    }
}

// ---------------------------------------------------------------------------
// U-21 — B34: the registry and the checked-in schema.
// ---------------------------------------------------------------------------

#[test]
fn u21_schema_and_registry() {
    let versions = schema_versions();
    assert_eq!(versions.len(), 21);
    let ui = versions
        .iter()
        .find(|version| version.schema_type == "crpg.ui")
        .expect("crpg.ui registry row");
    assert_eq!(ui.current, 1);

    let schemas = generated_schemas().unwrap();
    assert_eq!(schemas.len(), 23);
    let checked = std::fs::read(repo_root().join("schemas").join("ui.schema.json")).unwrap();
    assert_eq!(schemas["ui.schema.json"], checked);

    let schema: Value = serde_json::from_slice(&schemas["ui.schema.json"]).unwrap();
    let branches = schema["oneOf"].as_array().unwrap();
    assert_eq!(branches.len(), 1);
    assert_eq!(
        branches[0]["properties"]["schema"]["const"],
        json!("crpg.ui/1")
    );
    assert_eq!(branches[0]["properties"]["schema"]["type"], json!("string"));

    // A current `crpg.ui/1` value is not migrated or retagged.
    let mut value = ui_json();
    let before = value.clone();
    migrate_document(&mut value).unwrap();
    assert_eq!(value, before);
}

// ===========================================================================
// Fix round: gaps found by the mutant review.
// ===========================================================================

fn ui_with(patch: Value) -> Value {
    let mut value = ui_json();
    if let Value::Object(map) = &patch {
        for (key, item) in map {
            value[key] = item.clone();
        }
    }
    value
}

/// (pointer, code) pairs of the findings, sorted by pointer then code.
fn pairs(diagnostics: &[Diagnostic]) -> Vec<(String, &'static str)> {
    let mut out: Vec<(String, &'static str)> = diagnostics
        .iter()
        .map(|d| (d.pointer.clone(), d.code.as_str()))
        .collect();
    out.sort();
    out
}

fn expect_pairs(diagnostics: &[Diagnostic], expected: &[(&str, DiagnosticCode)]) {
    let mut want: Vec<(String, &'static str)> = expected
        .iter()
        .map(|(pointer, code)| ((*pointer).to_string(), code.as_str()))
        .collect();
    want.sort();
    assert_eq!(pairs(diagnostics), want, "{diagnostics:#?}");
}

fn at<'a>(diagnostics: &'a [Diagnostic], pointer: &str) -> &'a Diagnostic {
    let found: Vec<&Diagnostic> = diagnostics
        .iter()
        .filter(|d| d.pointer == pointer)
        .collect();
    assert_eq!(found.len(), 1, "one finding at {pointer}: {diagnostics:#?}");
    found[0]
}

fn with_spare_ability(base: &Files) -> Files {
    add_json(
        base,
        "abilities/spare.json",
        &ability_doc(SPARE_ABILITY, "spare"),
    )
}

// --- Gap 1 (B14, B24): a repeated undeclared enum variant. ------------------

#[test]
fn u13_repeated_undeclared_variant_gets_duplicate_binding_only() {
    let base = six_kind_campaign();
    let labels = |variants: &[&str]| {
        let entries: Vec<Value> = variants
            .iter()
            .map(|v| json!({"variant": v, "label": SRD_KEY}))
            .collect();
        ui_with(json!({
            "ruleset": SRD_RULESET,
            "stats": [{"stat": "ward", "label": SRD_KEY,
                "format": {"type": "enum", "value": entries}}]
        }))
    };
    let findings = check_ui(
        &base,
        "ui/a.json",
        &labels(&["green", "green", "red", "green"]),
    );
    expect_pairs(
        &findings,
        &[
            (
                "/stats/0/format/value/0/variant",
                DiagnosticCode::UnknownVariant,
            ),
            (
                "/stats/0/format/value/1/variant",
                DiagnosticCode::DuplicateBinding,
            ),
            (
                "/stats/0/format/value/3/variant",
                DiagnosticCode::DuplicateBinding,
            ),
        ],
    );
    assert_eq!(
        at(&findings, "/stats/0/format/value/0/variant").message,
        "variant \"green\" is not declared by enum stat \"ward\""
    );
    assert_eq!(
        at(&findings, "/stats/0/format/value/1/variant").message,
        "duplicate variant \"green\" (first at ui/a.json/stats/0/format/value/0/variant)"
    );
    assert_eq!(
        at(&findings, "/stats/0/format/value/3/variant").message,
        "duplicate variant \"green\" (first at ui/a.json/stats/0/format/value/0/variant)"
    );
    // Control: no repeat, so only the first-occurrence finding remains.
    let findings = check_ui(&base, "ui/a.json", &labels(&["green", "red"]));
    expect_pairs(
        &findings,
        &[(
            "/stats/0/format/value/0/variant",
            DiagnosticCode::UnknownVariant,
        )],
    );
    // Control: declared variants, no repeat.
    assert_eq!(
        check_ui(&base, "ui/a.json", &labels(&["red", "blue"])),
        Vec::new()
    );
}

// --- Gap 2 (B20, B24): screen repeats of unresolved ids. --------------------

#[test]
fn u20_screen_repeats_of_unresolved_entries_get_duplicate_binding_only() {
    let base = with_spare_ability(&fixture("combat_basic"));
    let sections = json!([
        {"type": "stat_group", "value": {"type": "names", "value": ["nope"]}},
        {"type": "stat_group", "value": {"type": "names", "value": ["nope", "might", "might"]}},
        {"type": "pool_group", "value": {"type": "ids", "value": [FOREIGN]}},
        {"type": "pool_group", "value": {"type": "ids", "value": [FOREIGN]}},
        {"type": "ability_list", "value": {"select": {"type": "ids", "value": [SPARE_ABILITY]}}},
        {"type": "ability_list", "value": {"select": {"type": "ids", "value": [SPARE_ABILITY]}}}
    ]);
    let doc = ui_with(json!({"screens": [{"kind": "character_sheet", "sections": sections}]}));
    let findings = check_ui(&base, "ui/a.json", &doc);
    expect_pairs(
        &findings,
        &[
            (
                "/screens/0/sections/0/value/value/0",
                DiagnosticCode::UnknownStat,
            ),
            (
                "/screens/0/sections/1/value/value/0",
                DiagnosticCode::DuplicateBinding,
            ),
            (
                "/screens/0/sections/1/value/value/2",
                DiagnosticCode::DuplicateBinding,
            ),
            (
                "/screens/0/sections/2/value/value/0",
                DiagnosticCode::UnknownPool,
            ),
            (
                "/screens/0/sections/3/value/value/0",
                DiagnosticCode::DuplicateBinding,
            ),
            (
                "/screens/0/sections/4/value/select/value/0",
                DiagnosticCode::ForeignReference,
            ),
            (
                "/screens/0/sections/5/value/select/value/0",
                DiagnosticCode::DuplicateBinding,
            ),
        ],
    );
    assert_eq!(
        at(&findings, "/screens/0/sections/1/value/value/0").message,
        "duplicate stat \"nope\" (first at ui/a.json/screens/0/sections/0/value/value/0)"
    );
    assert_eq!(
        at(&findings, "/screens/0/sections/1/value/value/2").message,
        "duplicate stat \"might\" (first at ui/a.json/screens/0/sections/1/value/value/1)"
    );
    assert_eq!(
        at(&findings, "/screens/0/sections/3/value/value/0").message,
        format!("duplicate pool {FOREIGN} (first at ui/a.json/screens/0/sections/2/value/value/0)")
    );
    assert_eq!(
        at(&findings, "/screens/0/sections/5/value/select/value/0").message,
        format!(
            "duplicate ability {SPARE_ABILITY} (first at ui/a.json/screens/0/sections/4/value/select/value/0)"
        )
    );
    assert_eq!(
        at(&findings, "/screens/0/sections/4/value/select/value/0").message,
        format!("ui ability {SPARE_ABILITY} is not listed by ruleset {BASIC_RULESET}")
    );

    // Control: the same unresolved ids on two screens are not repeats.
    let one = json!([
        {"type": "stat_group", "value": {"type": "names", "value": ["nope"]}},
        {"type": "pool_group", "value": {"type": "ids", "value": [FOREIGN]}},
        {"type": "ability_list", "value": {"select": {"type": "ids", "value": [SPARE_ABILITY]}}}
    ]);
    let doc = ui_with(json!({"screens": [
        {"kind": "character_sheet", "sections": one},
        {"kind": "combat_hud", "sections": one}
    ]}));
    let findings = check_ui(&base, "ui/a.json", &doc);
    assert_eq!(findings.len(), 6, "{findings:#?}");
    assert!(findings
        .iter()
        .all(|d| d.code != DiagnosticCode::DuplicateBinding));
}

// --- Gap 3 (U-10): pool checks never consult the object index. -------------

#[test]
fn u10_pool_check_rejects_existing_non_pool_objects() {
    let base = fixture("combat_basic");
    for object in [BASIC_CREATURE, BASIC_ABILITY, BASIC_RULESET] {
        let message =
            format!("unknown pool {object} for ui: not a template of ruleset {BASIC_RULESET}");
        let docs = [
            (
                ui_with(json!({"pools": [{"pool": object, "label": KEY}]})),
                "/pools/0/pool",
            ),
            (
                ui_with(json!({"stats": [{"stat": "might", "label": KEY,
                    "format": {"type": "fraction", "value": {"type": "pool", "value": object}}}]})),
                "/stats/0/format/value/value",
            ),
            (
                ui_with(json!({"screens": [{"kind": "combat_hud", "sections": [
                    {"type": "pool_group", "value": {"type": "ids", "value": [object]}}]}]})),
                "/screens/0/sections/0/value/value/0",
            ),
        ];
        for (doc, pointer) in docs {
            let finding = assert_only(
                &check_ui(&base, "ui/a.json", &doc),
                DiagnosticCode::UnknownPool,
                pointer,
            );
            assert_eq!(finding.message, message);
        }
    }
    // Control: the real template passes at the same three sites.
    let ok = ui_with(json!({
        "pools": [{"pool": BASIC_POOL, "label": KEY}],
        "stats": [{"stat": "might", "label": KEY,
            "format": {"type": "fraction", "value": {"type": "pool", "value": BASIC_POOL}}}],
        "screens": [{"kind": "combat_hud", "sections": [
            {"type": "pool_group", "value": {"type": "ids", "value": [BASIC_POOL]}}]}]
    }));
    assert_eq!(check_ui(&base, "ui/a.json", &ok), Vec::new());
}

// --- Gap 4 (B16, B17, B24): wrong-kind ids; repeated unlisted ability. ------

#[test]
fn u10_wrong_kind_ability_ids_get_the_generic_finding_only() {
    let base = fixture("combat_basic");
    for wrong in [BASIC_CREATURE, BASIC_RULESET] {
        let doc = ui_with(json!({"abilities": [
            {"ability": wrong, "label": KEY},
            {"ability": wrong, "label": KEY}
        ]}));
        let findings = check_ui(&base, "ui/a.json", &doc);
        expect_pairs(
            &findings,
            &[
                ("/abilities/0/ability", DiagnosticCode::WrongReferenceKind),
                ("/abilities/1/ability", DiagnosticCode::WrongReferenceKind),
            ],
        );
        for (index, finding) in findings.iter().enumerate() {
            assert!(
                finding.message.starts_with(&format!(
                    "wrong kind for ui ability reference to {wrong}: found "
                )),
                "{index}: {finding:?}"
            );
        }
        // The same id in an `ability_list` selector: generic finding only.
        let doc = ui_with(json!({"screens": [{"kind": "action_bar", "sections": [
            {"type": "ability_list", "value": {"select": {"type": "ids", "value": [wrong, wrong]}}}
        ]}]}));
        let findings = check_ui(&base, "ui/a.json", &doc);
        expect_pairs(
            &findings,
            &[
                (
                    "/screens/0/sections/0/value/select/value/0",
                    DiagnosticCode::WrongReferenceKind,
                ),
                (
                    "/screens/0/sections/0/value/select/value/1",
                    DiagnosticCode::WrongReferenceKind,
                ),
            ],
        );
    }
}

#[test]
fn u10_wrong_kind_effect_ids_get_the_generic_finding_only() {
    let base = fixture("combat_srd");
    for wrong in [SRD_HEAVY, SRD_RULESET] {
        let doc = ui_with(json!({"ruleset": SRD_RULESET, "effects": [
            {"effect": wrong, "label": SRD_KEY},
            {"effect": wrong, "label": SRD_KEY}
        ]}));
        let findings = check_ui(&base, "ui/a.json", &doc);
        expect_pairs(
            &findings,
            &[
                ("/effects/0/effect", DiagnosticCode::WrongReferenceKind),
                ("/effects/1/effect", DiagnosticCode::WrongReferenceKind),
            ],
        );
        assert!(findings[0].message.starts_with(&format!(
            "wrong kind for ui effect reference to {wrong}: found "
        )));
    }
}

#[test]
fn u10_repeated_unlisted_ability_gets_one_finding_per_pointer() {
    let base = with_spare_ability(&fixture("combat_basic"));
    let doc = ui_with(json!({"abilities": [
        {"ability": SPARE_ABILITY, "label": KEY},
        {"ability": SPARE_ABILITY, "label": KEY},
        {"ability": SPARE_ABILITY, "label": KEY}
    ]}));
    let findings = check_ui(&base, "ui/a.json", &doc);
    expect_pairs(
        &findings,
        &[
            ("/abilities/0/ability", DiagnosticCode::ForeignReference),
            ("/abilities/1/ability", DiagnosticCode::DuplicateBinding),
            ("/abilities/2/ability", DiagnosticCode::DuplicateBinding),
        ],
    );
    assert_eq!(
        at(&findings, "/abilities/2/ability").message,
        format!("duplicate ability {SPARE_ABILITY} (first at ui/a.json/abilities/0/ability)")
    );
    // Control: listed once and listed-and-repeated differ only by the repeat.
    let doc = ui_with(json!({"abilities": [
        {"ability": BASIC_ABILITY, "label": KEY},
        {"ability": BASIC_ABILITY, "label": KEY}
    ]}));
    let findings = check_ui(&base, "ui/a.json", &doc);
    expect_pairs(
        &findings,
        &[("/abilities/1/ability", DiagnosticCode::DuplicateBinding)],
    );
}

// --- Gap 5 (B14): fraction meters by stat kind. -----------------------------

#[test]
fn u13_fraction_meter_stat_kinds() {
    let base = six_kind_campaign();
    let meter = |stat: &str, meter_stat: &str| {
        ui_with(
            json!({"ruleset": SRD_RULESET, "stats": [{"stat": stat, "label": SRD_KEY,
            "format": {"type": "fraction", "value": {"type": "stat", "value": meter_stat}}}]}),
        )
    };
    // int and fixed meters are accepted, a fixed stat included.
    for (stat, meter_stat) in [
        ("might", "might"),
        ("might", "guile"),
        ("guile", "might"),
        ("guile", "guile"),
    ] {
        assert_eq!(
            check_ui(&base, "ui/a.json", &meter(stat, meter_stat)),
            Vec::new(),
            "{stat} metered by {meter_stat}"
        );
    }
    // Every other declared kind gives format_mismatch at the meter pointer.
    for (meter_stat, kind) in [
        ("resolve", "bool"),
        ("ward", "enum"),
        ("talent", "tags"),
        ("lore", "dice"),
    ] {
        let finding = assert_only(
            &check_ui(&base, "ui/a.json", &meter("guile", meter_stat)),
            DiagnosticCode::FormatMismatch,
            "/stats/0/format/value/value",
        );
        assert_eq!(
            finding.message,
            format!("fraction meter stat \"{meter_stat}\" is {kind}, not int or fixed")
        );
        assert_eq!(
            finding.suggested_fix.as_deref(),
            Some("choose a format that suits the stat's kind")
        );
    }
    // A real pool template passes; an existing non-pool object does not.
    let pool = |id: &str| {
        ui_with(
            json!({"ruleset": SRD_RULESET, "stats": [{"stat": "guile", "label": SRD_KEY,
            "format": {"type": "fraction", "value": {"type": "pool", "value": id}}}]}),
        )
    };
    assert_eq!(check_ui(&base, "ui/a.json", &pool(SRD_POOL_1)), Vec::new());
    assert_only(
        &check_ui(&base, "ui/a.json", &pool(SRD_HEAVY)),
        DiagnosticCode::UnknownPool,
        "/stats/0/format/value/value",
    );
}

// --- Gap 6 (B20): several groups and tags. -----------------------------------

#[test]
fn u12_group_selector_distinguishes_this_group_from_any_group() {
    let base = fixture("combat_basic");
    let doc = |selected: &[&str]| {
        let sections: Vec<Value> = selected
            .iter()
            .map(|g| json!({"type": "stat_group", "value": {"type": "group", "value": g}}))
            .collect();
        ui_with(json!({
            "stats": [
                {"stat": "guile", "label": KEY},
                {"stat": "might", "label": KEY, "group": "fixture.hero"},
                {"stat": "resolve", "label": KEY, "group": "fixture.goblin"}
            ],
            "screens": [{"kind": "character_sheet", "sections": sections}]
        }))
    };
    assert_eq!(
        check_ui(
            &base,
            "ui/a.json",
            &doc(&["fixture.hero", "fixture.goblin"])
        ),
        Vec::new()
    );
    let findings = check_ui(
        &base,
        "ui/a.json",
        &doc(&["fixture.goblin", "fixture.start", "fixture.hero"]),
    );
    let finding = assert_only(
        &findings,
        DiagnosticCode::UnknownGroup,
        "/screens/0/sections/1/value/value",
    );
    assert_eq!(
        finding.message,
        "stat group \"fixture.start\" is the group of no stats entry"
    );
    // A group that exists on no entry, with no other group present, fails too.
    let mut bare = doc(&["fixture.hero"]);
    bare["stats"] = json!([{"stat": "guile", "label": KEY}]);
    assert_only(
        &check_ui(&base, "ui/a.json", &bare),
        DiagnosticCode::UnknownGroup,
        "/screens/0/sections/0/value/value",
    );
}

#[test]
fn u12_ability_tag_selector_distinguishes_this_tag_from_any_tag() {
    let base = fixture("combat_srd");
    let doc = |selected: &[&str]| {
        let sections: Vec<Value> = selected
            .iter()
            .map(|t| {
                json!({"type": "ability_list",
                    "value": {"select": {"type": "tag", "value": t}}})
            })
            .collect();
        ui_with(json!({
            "ruleset": SRD_RULESET,
            "abilities": [
                {"ability": SRD_HEAVY, "label": SRD_KEY, "category_tags": ["melee", "heavy"]},
                {"ability": SRD_FOCUS, "label": SRD_KEY, "category_tags": ["magic"]}
            ],
            "screens": [{"kind": "action_bar", "sections": sections}]
        }))
    };
    assert_eq!(
        check_ui(&base, "ui/a.json", &doc(&["melee", "heavy", "magic"])),
        Vec::new()
    );
    let findings = check_ui(&base, "ui/a.json", &doc(&["magic", "ghost", "heavy"]));
    let finding = assert_only(
        &findings,
        DiagnosticCode::UnknownTag,
        "/screens/0/sections/1/value/select/value",
    );
    assert_eq!(
        finding.message,
        "ability tag \"ghost\" is declared by no abilities entry"
    );
    assert_eq!(finding.severity, Severity::Error);
}

#[test]
fn u12_item_tag_selector_distinguishes_this_tag_from_any_tag() {
    let base = fixture("combat_basic");
    let item = |id: &str, slug: &str, tags: Value| {
        json!({"schema": "crpg.item/2", "id": id, "slug": slug, "name": KEY,
            "stats": {}, "tags": tags})
    };
    let base = add_json(
        &add_json(
            &base,
            "items/one.json",
            &item("00000000000000000000000030", "one", json!(["rare"])),
        ),
        "items/two.json",
        &item(
            "00000000000000000000000031",
            "two",
            json!(["common", "epic"]),
        ),
    );
    let doc = |selected: &[&str]| {
        let sections: Vec<Value> = selected
            .iter()
            .map(|t| json!({"type": "item_list", "value": {"type": "tag", "value": t}}))
            .collect();
        ui_with(json!({"screens": [{"kind": "inventory", "sections": sections}]}))
    };
    assert_eq!(
        check_ui(&base, "ui/a.json", &doc(&["rare", "common", "epic"])),
        Vec::new()
    );
    let findings = check_ui(&base, "ui/a.json", &doc(&["epic", "ghost", "rare"]));
    let finding = assert_only_warning(
        &findings,
        DiagnosticCode::UnknownTag,
        "/screens/0/sections/1/value/value",
    );
    assert_eq!(
        finding.message,
        "item tag \"ghost\" is carried by no item in this campaign"
    );
}

fn assert_only_warning(
    diagnostics: &[Diagnostic],
    code: DiagnosticCode,
    pointer: &str,
) -> Diagnostic {
    let diagnostic = only(diagnostics);
    assert_eq!(diagnostic.code, code, "{diagnostic:?}");
    assert_eq!(diagnostic.pointer, pointer, "{diagnostic:?}");
    assert_eq!(diagnostic.severity, Severity::Warning, "{diagnostic:?}");
    diagnostic
}

// --- Gap 7 (B5): the 1-byte lower bound and the walk order. -----------------

/// One document per string site, each holding `text` at that site.
fn string_sites(text: &str) -> Vec<(Value, &'static str)> {
    let t = json!(text);
    let sel = |value: Value, kind: &str| json!({"screens": [{"kind": kind, "sections": [value]}]});
    vec![
        (
            json!({"stats": [{"stat": t, "label": KEY}]}),
            "/stats/0/stat",
        ),
        (
            json!({"stats": [{"stat": "health", "label": t}]}),
            "/stats/0/label",
        ),
        (
            json!({"stats": [{"stat": "health", "label": KEY, "short_label": t}]}),
            "/stats/0/short_label",
        ),
        (
            json!({"stats": [{"stat": "health", "label": KEY, "description": t}]}),
            "/stats/0/description",
        ),
        (
            json!({"stats": [{"stat": "health", "label": KEY, "group": t}]}),
            "/stats/0/group",
        ),
        (
            json!({"stats": [{"stat": "health", "label": KEY,
                "format": {"type": "enum", "value": [{"variant": t, "label": KEY}]}}]}),
            "/stats/0/format/value/0/variant",
        ),
        (
            json!({"stats": [{"stat": "health", "label": KEY,
                "format": {"type": "enum", "value": [{"variant": "a", "label": t}]}}]}),
            "/stats/0/format/value/0/label",
        ),
        (
            json!({"stats": [{"stat": "health", "label": KEY, "format": {"type": t}}]}),
            "/stats/0/format/type",
        ),
        (
            json!({"pools": [{"pool": BASIC_POOL, "label": t}]}),
            "/pools/0/label",
        ),
        (
            json!({"pools": [{"pool": BASIC_POOL, "label": KEY, "icon": t}]}),
            "/pools/0/icon",
        ),
        (
            json!({"pools": [{"pool": BASIC_POOL, "label": KEY, "style": t}]}),
            "/pools/0/style",
        ),
        (
            json!({"abilities": [{"ability": BASIC_ABILITY, "label": t}]}),
            "/abilities/0/label",
        ),
        (
            json!({"abilities": [{"ability": BASIC_ABILITY, "label": KEY, "description": t}]}),
            "/abilities/0/description",
        ),
        (
            json!({"abilities": [{"ability": BASIC_ABILITY, "label": KEY, "icon": t}]}),
            "/abilities/0/icon",
        ),
        (
            json!({"abilities": [{"ability": BASIC_ABILITY, "label": KEY,
                "category_tags": [t]}]}),
            "/abilities/0/category_tags/0",
        ),
        (
            json!({"abilities": [{"ability": BASIC_ABILITY, "label": KEY, "target_hint": t}]}),
            "/abilities/0/target_hint",
        ),
        (
            json!({"effects": [{"effect": SRD_EFFECT, "label": t}]}),
            "/effects/0/label",
        ),
        (
            json!({"effects": [{"effect": SRD_EFFECT, "label": KEY, "icon": t}]}),
            "/effects/0/icon",
        ),
        (
            json!({"outcomes": [{"outcome": t, "label": KEY}]}),
            "/outcomes/0/outcome",
        ),
        (
            json!({"outcomes": [{"outcome": "success", "label": t}]}),
            "/outcomes/0/label",
        ),
        (
            json!({"screens": [{"kind": t, "sections": []}]}),
            "/screens/0/kind",
        ),
        (
            sel(json!({"type": t}), "inventory"),
            "/screens/0/sections/0/type",
        ),
        (
            sel(
                json!({"type": "stat_group", "value": {"type": "names", "value": [t]}}),
                "character_sheet",
            ),
            "/screens/0/sections/0/value/value/0",
        ),
        (
            sel(
                json!({"type": "stat_group", "value": {"type": "group", "value": t}}),
                "character_sheet",
            ),
            "/screens/0/sections/0/value/value",
        ),
        (
            sel(
                json!({"type": "ability_list",
                    "value": {"select": {"type": "tag", "value": t}}}),
                "action_bar",
            ),
            "/screens/0/sections/0/value/select/value",
        ),
        (
            sel(
                json!({"type": "ability_list",
                    "value": {"select": {"type": "all"}, "sort": t}}),
                "action_bar",
            ),
            "/screens/0/sections/0/value/sort",
        ),
        (
            sel(
                json!({"type": "item_list", "value": {"type": "tag", "value": t}}),
                "inventory",
            ),
            "/screens/0/sections/0/value/value",
        ),
    ]
}

#[test]
fn u3_empty_strings_are_rejected_at_every_site() {
    for (patch, pointer) in string_sites("") {
        assert_text_error(&ui_with(patch), pointer);
    }
    // Controls: a 1-byte and a 128-byte string pass at every site.
    for text in ["a".to_string(), "b".repeat(128)] {
        for (patch, pointer) in string_sites(&text) {
            assert!(
                read(&ui_with(patch)).is_ok(),
                "{} bytes at {pointer} should read",
                text.len()
            );
        }
    }
    // One byte over, at every site.
    for (patch, pointer) in string_sites(&"c".repeat(129)) {
        assert_text_error(&ui_with(patch), pointer);
    }
}

#[test]
fn u3_walk_order_between_and_within_entries() {
    let over = "x".repeat(129);
    let over = over.as_str();

    // Between top-level lists: ruleset, stats, pools, abilities, effects,
    // outcomes, screens.
    let lists: Vec<(&str, Value, &str)> = vec![
        (
            "stats",
            json!([{"stat": over, "label": KEY}]),
            "/stats/0/stat",
        ),
        (
            "pools",
            json!([{"pool": BASIC_POOL, "label": over}]),
            "/pools/0/label",
        ),
        (
            "abilities",
            json!([{"ability": BASIC_ABILITY, "label": over}]),
            "/abilities/0/label",
        ),
        (
            "effects",
            json!([{"effect": SRD_EFFECT, "label": over}]),
            "/effects/0/label",
        ),
        (
            "outcomes",
            json!([{"outcome": over, "label": KEY}]),
            "/outcomes/0/outcome",
        ),
        (
            "screens",
            json!([{"kind": over, "sections": []}]),
            "/screens/0/kind",
        ),
    ];
    for first in 0..lists.len() {
        let mut value = ui_json();
        for (name, list, _) in &lists[first..] {
            value[name] = list.clone();
        }
        assert_text_error(&value, lists[first].2);
    }
    // A later list's count failure does not pre-empt an earlier text failure,
    // and an earlier count failure wins over a later text failure.
    let mut value = ui_json();
    value["effects"] = json!([{"effect": SRD_EFFECT, "label": over}]);
    value["outcomes"] = entries(4097, |_| json!({"outcome": "success", "label": KEY}));
    assert_text_error(&value, "/effects/0/label");
    let mut value = ui_json();
    value["outcomes"] = entries(4097, |_| json!({"outcome": "success", "label": KEY}));
    value["screens"] = json!([{"kind": over, "sections": []}]);
    assert_list_error(&value, "/outcomes", 4096);

    // Within an entry: fields in declaration order. Each case lists the bad
    // fields still set; the first declared of them is the one reported.
    let stat_doc = |bad: &[&str]| {
        let pick = |field: &str, ok: &str| json!(if bad.contains(&field) { over } else { ok });
        ui_with(json!({"stats": [{
            "stat": pick("stat", "health"), "label": pick("label", KEY),
            "short_label": pick("short_label", KEY),
            "description": pick("description", KEY), "group": pick("group", KEY),
            "format": {"type": "enum", "value": [{"variant": over, "label": KEY}]}
        }]}))
    };
    let stat_fields = ["stat", "label", "short_label", "description", "group"];
    for first in 0..stat_fields.len() {
        assert_text_error(
            &stat_doc(&stat_fields[first..]),
            &format!("/stats/0/{}", stat_fields[first]),
        );
    }
    // `group` is declared before `format`, so group is reported first; with
    // `group` fine, the format's enum label is reached.
    assert_text_error(&stat_doc(&[]), "/stats/0/format/value/0/variant");

    let pool_doc = |bad: &[&str]| {
        let pick = |field: &str, ok: &str| json!(if bad.contains(&field) { over } else { ok });
        ui_with(
            json!({"pools": [{"pool": BASIC_POOL, "label": pick("label", KEY),
            "icon": pick("icon", "i"), "style": over}]}),
        )
    };
    assert_text_error(&pool_doc(&["label", "icon"]), "/pools/0/label");
    assert_text_error(&pool_doc(&["icon"]), "/pools/0/icon");
    assert_text_error(&pool_doc(&[]), "/pools/0/style");

    let ability_doc_with = |bad: &[&str]| {
        let pick = |field: &str, ok: &str| json!(if bad.contains(&field) { over } else { ok });
        ui_with(
            json!({"abilities": [{"ability": BASIC_ABILITY, "label": pick("label", KEY),
            "description": pick("description", KEY), "icon": pick("icon", "i"),
            "category_tags": [pick("category_tags", "t")], "target_hint": over}]}),
        )
    };
    assert_text_error(
        &ability_doc_with(&["label", "description", "icon", "category_tags"]),
        "/abilities/0/label",
    );
    assert_text_error(
        &ability_doc_with(&["description", "icon", "category_tags"]),
        "/abilities/0/description",
    );
    assert_text_error(
        &ability_doc_with(&["icon", "category_tags"]),
        "/abilities/0/icon",
    );
    assert_text_error(
        &ability_doc_with(&["category_tags"]),
        "/abilities/0/category_tags/0",
    );
    assert_text_error(&ability_doc_with(&[]), "/abilities/0/target_hint");

    // An effect's `label` is declared before its `icon`.
    let effect_doc = |label: &str, icon: &str| {
        ui_with(json!({"effects": [{"effect": SRD_EFFECT, "label": label, "icon": icon}]}))
    };
    assert_text_error(&effect_doc(over, over), "/effects/0/label");
    assert_text_error(&effect_doc(KEY, over), "/effects/0/icon");
    assert!(read(&effect_doc(KEY, "i")).is_ok());

    // An outcome's `outcome` precedes its `label`; an enum label's `variant`
    // precedes its `label`.
    assert_text_error(
        &ui_with(json!({"outcomes": [{"outcome": over, "label": over}]})),
        "/outcomes/0/outcome",
    );
    assert_text_error(
        &ui_with(json!({"stats": [{"stat": "health", "label": KEY,
            "format": {"type": "enum", "value": [{"variant": over, "label": over}]}}]})),
        "/stats/0/format/value/0/variant",
    );
    // A screen's `kind` precedes its `sections`.
    assert_text_error(
        &ui_with(json!({"screens": [{"kind": over, "sections": [{"type": over}]}]})),
        "/screens/0/kind",
    );
    assert_text_error(
        &ui_with(json!({"screens": [{"kind": "inventory", "sections": [{"type": over}]}]})),
        "/screens/0/sections/0/type",
    );
}

// --- Gap 8 (B25): first-wins resolution and first-use dedupe. ---------------

fn uid(n: u128) -> String {
    Ulid::from_u128(n).to_string()
}

fn srd_json(relative: &str) -> Value {
    serde_json::from_slice(&fixture("combat_srd")[&path(relative)]).unwrap()
}

fn doc_map(entries: Vec<(&str, Value)>) -> BTreeMap<SourcePath, Document> {
    entries
        .into_iter()
        .map(|(name, value)| (path(name), read_ok(&value)))
        .collect()
}

fn ruleset_json(id: u128, slug: &str, abilities: &[u128]) -> Value {
    let mut value = srd_json("rulesets/srd-lite.json");
    value["id"] = json!(uid(id));
    value["slug"] = json!(slug);
    value["abilities"] = json!(abilities.iter().map(|a| uid(*a)).collect::<Vec<_>>());
    value
}

fn ability_json(
    id: u128,
    name: &str,
    effect: Option<u128>,
    table: u128,
    damage: &[OutcomeWire],
) -> Value {
    let mut value = srd_json("abilities/heavy.json");
    value["id"] = json!(uid(id));
    value["slug"] = json!(format!("ability-{id}-{name}"));
    value["name"] = json!(name);
    value["outcome_table"] = json!(uid(table));
    value["damage"] = json!(damage
        .iter()
        .map(|outcome| json!({"outcome": outcome, "amount": 1}))
        .collect::<Vec<_>>());
    match effect {
        Some(effect) => value["effect"] = json!(uid(effect)),
        None => {
            value.as_object_mut().unwrap().remove("effect");
        }
    }
    value
}

fn effect_json(id: u128, name: &str) -> Value {
    let mut value = srd_json("effects/focusing.json");
    value["id"] = json!(uid(id));
    value["slug"] = json!(format!("effect-{id}-{name}"));
    value["name"] = json!(name);
    value
}

fn table_json(
    id: u128,
    bands: &[(i64, OutcomeWire)],
    naturals: &[(u32, NaturalEffectWire)],
) -> Value {
    json!({
        "schema": "crpg.outcome-table/1", "id": uid(id), "slug": format!("table-{id}"),
        "name": "fixture.table",
        "bands": bands.iter()
            .map(|(margin, outcome)| json!({"min_margin": margin, "outcome": outcome}))
            .collect::<Vec<_>>(),
        "natural_rules": naturals.iter()
            .map(|(face, effect)| json!({"face": face, "effect": effect}))
            .collect::<Vec<_>>()
    })
}

#[test]
fn u25_from_documents_first_wins_and_first_use_dedupe() {
    use OutcomeWire::{Failure, Success};
    let ruleset = uid(100);
    let documents = doc_map(vec![
        // Two rulesets with one id: the first in path order is the ruleset.
        (
            "rulesets/a.json",
            ruleset_json(100, "first-ruleset", &[2, 109, 1, 4, 5]),
        ),
        ("rulesets/b.json", ruleset_json(100, "second-ruleset", &[1])),
        // Two abilities with id 1: the first in path order wins.
        (
            "abilities/a1.json",
            ability_json(1, "first.a1", Some(11), 21, &[Success]),
        ),
        (
            "abilities/z1.json",
            ability_json(1, "second.a1", Some(13), 23, &[Failure]),
        ),
        (
            "abilities/a2.json",
            ability_json(2, "first.a2", Some(12), 22, &[Success]),
        ),
        // Ability 4 shares ability 2's effect and ability 1's table.
        (
            "abilities/a4.json",
            ability_json(4, "first.a4", Some(12), 21, &[Success]),
        ),
        // Ability 5 names an effect and a table that no document holds.
        (
            "abilities/a5.json",
            ability_json(5, "first.a5", Some(99), 98, &[Success]),
        ),
        // Two effects with id 11, two tables with id 21.
        ("effects/e11.json", effect_json(11, "first.e11")),
        ("effects/e11b.json", effect_json(11, "second.e11")),
        ("effects/e12.json", effect_json(12, "first.e12")),
        ("effects/e13.json", effect_json(13, "first.e13")),
        (
            "outcome_tables/t21.json",
            table_json(21, &[(i64::MIN, Failure)], &[]),
        ),
        (
            "outcome_tables/t21b.json",
            table_json(21, &[(i64::MIN, OutcomeWire::CriticalFailure)], &[]),
        ),
        (
            "outcome_tables/t22.json",
            table_json(22, &[(i64::MIN, Success)], &[]),
        ),
        (
            "outcome_tables/t23.json",
            table_json(23, &[(i64::MIN, Success)], &[]),
        ),
    ]);
    let source = UiSource::from_documents(&documents, ulid(&ruleset)).unwrap();
    assert_eq!(source.ruleset.slug, "first-ruleset");
    // Ruleset order, unresolved id 109 skipped, first document of id 1.
    let ids: Vec<String> = source.abilities.iter().map(|a| a.id.to_string()).collect();
    assert_eq!(ids, vec![uid(2), uid(1), uid(4), uid(5)]);
    let names: Vec<&str> = source.abilities.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, vec!["first.a2", "first.a1", "first.a4", "first.a5"]);
    // Effects: first use in ability order (12 then 11), no repeat of 12, the
    // unresolved 99 skipped, the first document of id 11, and 13 never used.
    let effects: Vec<(String, &str)> = source
        .effects
        .iter()
        .map(|e| (e.id.to_string(), e.name.as_str()))
        .collect();
    assert_eq!(
        effects,
        vec![(uid(12), "first.e12"), (uid(11), "first.e11")]
    );
    // Tables: first use (22, then 21), no repeat of 21, 98 skipped, first 21.
    let tables: Vec<String> = source.tables.iter().map(|t| t.id.to_string()).collect();
    assert_eq!(tables, vec![uid(22), uid(21)]);
    assert_eq!(
        source.tables[1].bands[0].outcome,
        OutcomeWire::Failure,
        "the first table document of id 21 wins"
    );
    // The generated entries follow the resolved lists.
    let ui = generated_ui(&source);
    let effect_labels: Vec<&str> = ui
        .effects
        .as_ref()
        .unwrap()
        .iter()
        .map(|e| e.label.as_str())
        .collect();
    assert_eq!(effect_labels, vec!["first.e12", "first.e11"]);
    assert_eq!(ui.abilities.as_ref().unwrap().len(), 4);
    // The second ruleset's id has no document of its own.
    assert!(UiSource::from_documents(&documents, ulid(&uid(101))).is_none());
    // A document of another kind with the ruleset's id is not a ruleset.
    assert!(UiSource::from_documents(&documents, ulid(&uid(1))).is_none());
}

// --- Gap 9 (B26): outcome label order. ---------------------------------------

#[test]
fn u26_outcome_labels_band_then_natural_then_damage_per_ability() {
    use NaturalEffectWire::{Override, Shift};
    use OutcomeWire::{CriticalFailure, CriticalSuccess, Custom, Failure, Success};
    let documents = doc_map(vec![
        ("rulesets/r.json", ruleset_json(100, "r", &[1, 2, 3])),
        (
            "abilities/a1.json",
            ability_json(1, "a1", None, 21, &[CriticalFailure, Custom(3), Success]),
        ),
        (
            "abilities/a2.json",
            ability_json(2, "a2", None, 22, &[Custom(5), Custom(4), Custom(6)]),
        ),
        // Ability 3's table has no document: only its damage is taken.
        (
            "abilities/a3.json",
            ability_json(3, "a3", None, 98, &[Custom(11), Success]),
        ),
        (
            "outcome_tables/t21.json",
            table_json(
                21,
                &[(i64::MIN, Failure), (0, Success)],
                &[
                    (6, Override(CriticalSuccess)),
                    (1, Shift(1)),
                    (2, Override(Custom(7))),
                ],
            ),
        ),
        (
            "outcome_tables/t22.json",
            table_json(
                22,
                &[(i64::MIN, Custom(9)), (0, Failure)],
                &[(4, Override(Custom(3))), (5, Override(Custom(4)))],
            ),
        ),
    ]);
    let source = UiSource::from_documents(&documents, ulid(&uid(100))).unwrap();
    assert_eq!(source.abilities.len(), 3);
    assert_eq!(source.tables.len(), 2);
    // Ability 1: bands, then natural overrides (the shift adds nothing), then
    // damage in order. Ability 2 then interleaves its own table before its
    // damage; labels already seen are not repeated.
    let expected = vec![
        "failure",
        "success",
        "critical_success",
        "custom:7",
        "critical_failure",
        "custom:3",
        "custom:9",
        "custom:4",
        "custom:5",
        "custom:6",
        "custom:11",
    ];
    assert_eq!(source.outcome_labels(), expected);
    let ui = generated_ui(&source);
    let outcomes = ui.outcomes.as_ref().unwrap();
    assert_eq!(
        outcomes
            .iter()
            .map(|o| o.outcome.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    for outcome in outcomes {
        assert_eq!(outcome.outcome, outcome.label);
    }
    // Every generated label is a B18 label.
    for text in &expected {
        assert!(ui_outcome(text).is_some(), "{text}");
    }
}

// --- Gap 10 (B18): `custom:` takes a decimal u8 with no sign. ----------------

#[test]
fn u18_custom_labels_take_no_sign() {
    for text in [
        "custom:+1",
        "custom:-1",
        "custom:+0",
        "custom:-0",
        "custom: 1",
        "custom:1 ",
    ] {
        assert_eq!(ui_outcome(text), None, "{text:?}");
    }
    assert_eq!(ui_outcome("custom:1"), Some(OutcomeWire::Custom(1)));
    assert_eq!(ui_outcome("custom:0"), Some(OutcomeWire::Custom(0)));
    assert_eq!(ui_outcome("custom:255"), Some(OutcomeWire::Custom(255)));

    // A ruleset that produces custom:0 and custom:1 accepts those two labels
    // and still rejects the signed spellings.
    let mut base = fixture("combat_srd");
    let mut heavy: Value = serde_json::from_slice(&base[&path("abilities/heavy.json")]).unwrap();
    heavy["damage"].as_array_mut().unwrap().extend([
        json!({"outcome": OutcomeWire::Custom(0), "amount": 1}),
        json!({"outcome": OutcomeWire::Custom(1), "amount": 1}),
    ]);
    base.insert(path("abilities/heavy.json"), canonical(&heavy));
    let check = |text: &str| {
        let doc = ui_with(json!({"ruleset": SRD_RULESET,
            "outcomes": [{"outcome": text, "label": SRD_KEY}]}));
        check_ui(&base, "ui/a.json", &doc)
    };
    assert_eq!(check("custom:0"), Vec::new());
    assert_eq!(check("custom:1"), Vec::new());
    for text in ["custom:+1", "custom:-1", "custom:+0", "custom:-0"] {
        let finding = assert_only(
            &check(text),
            DiagnosticCode::UnknownOutcome,
            "/outcomes/0/outcome",
        );
        assert_eq!(
            finding.message,
            format!("outcome {text:?} is not produced by ruleset {SRD_RULESET}")
        );
    }
}

// --- Gap 11 (B30): merge keys with partially authored lists. -----------------

fn pool_entry(id: &str, label: &str) -> UiPool {
    UiPool {
        pool: ulid(id),
        label: label.into(),
        icon: None,
        style: None,
        show_on_hud: None,
    }
}

fn ability_entry(id: &str, label: &str) -> UiAbility {
    UiAbility {
        ability: ulid(id),
        label: label.into(),
        description: None,
        icon: None,
        category_tags: None,
        target_hint: None,
        show_odds: None,
    }
}

fn effect_entry(id: &str, label: &str) -> UiEffect {
    UiEffect {
        effect: ulid(id),
        label: label.into(),
        icon: None,
        show_duration: None,
    }
}

fn outcome_entry(outcome: &str, label: &str) -> UiOutcome {
    UiOutcome {
        outcome: outcome.into(),
        label: label.into(),
    }
}

#[test]
fn u30_partially_authored_lists_get_missing_generated_entries_appended() {
    let srd = load(&fixture("combat_srd"));
    let source = UiSource::from_documents(&srd.documents, ulid(SRD_RULESET)).unwrap();
    let d = generated_ui(&source);
    let d_pools = d.pools.clone().unwrap();
    let d_abilities = d.abilities.clone().unwrap();
    let d_effects = d.effects.clone().unwrap();
    let d_outcomes = d.outcomes.clone().unwrap();
    let d_screens = d.screens.clone().unwrap();

    // Pools: the authored pool 2 first, then the generated pool 1.
    let authored = UiDocument {
        ruleset: ulid(SRD_RULESET),
        pools: Some(vec![pool_entry(SRD_POOL_2, "mine")]),
        ..empty_ui()
    };
    let merged = effective_ui(Some(&authored), &source);
    assert_eq!(
        merged.pools,
        Some(vec![pool_entry(SRD_POOL_2, "mine"), d_pools[0].clone()])
    );
    // Absent lists come from the default whole.
    assert_eq!(merged.stats, d.stats);
    assert_eq!(merged.abilities, d.abilities);
    assert_eq!(merged.effects, d.effects);
    assert_eq!(merged.outcomes, d.outcomes);
    assert_eq!(merged.screens, d.screens);
    // An authored pool the ruleset lacks is kept, and both templates follow.
    let authored = UiDocument {
        pools: Some(vec![pool_entry(FOREIGN, "alien")]),
        ..empty_ui()
    };
    assert_eq!(
        effective_ui(Some(&authored), &source).pools,
        Some(vec![
            pool_entry(FOREIGN, "alien"),
            d_pools[0].clone(),
            d_pools[1].clone()
        ])
    );

    // Abilities: the authored focus ability first, then the generated heavy.
    let authored = UiDocument {
        abilities: Some(vec![ability_entry(SRD_FOCUS, "mine")]),
        ..empty_ui()
    };
    assert_eq!(
        effective_ui(Some(&authored), &source).abilities,
        Some(vec![
            ability_entry(SRD_FOCUS, "mine"),
            d_abilities[0].clone()
        ])
    );

    // Effects: an authored list that lacks the generated effect gains it;
    // one that has it keeps its own entry once; an empty list gains it.
    let authored = UiDocument {
        effects: Some(vec![effect_entry(FOREIGN, "alien")]),
        ..empty_ui()
    };
    assert_eq!(
        effective_ui(Some(&authored), &source).effects,
        Some(vec![effect_entry(FOREIGN, "alien"), d_effects[0].clone()])
    );
    let authored = UiDocument {
        effects: Some(vec![effect_entry(SRD_EFFECT, "mine")]),
        ..empty_ui()
    };
    assert_eq!(
        effective_ui(Some(&authored), &source).effects,
        Some(vec![effect_entry(SRD_EFFECT, "mine")])
    );
    let authored = UiDocument {
        effects: Some(Vec::new()),
        ..empty_ui()
    };
    assert_eq!(
        effective_ui(Some(&authored), &source).effects,
        Some(d_effects.clone())
    );

    // Outcomes: keyed by text; the generated ones the author lacks follow in
    // the default's order.
    let authored = UiDocument {
        outcomes: Some(vec![outcome_entry("success", "mine")]),
        ..empty_ui()
    };
    assert_eq!(
        effective_ui(Some(&authored), &source).outcomes,
        Some(vec![
            outcome_entry("success", "mine"),
            d_outcomes[0].clone(),
            d_outcomes[2].clone()
        ])
    );
    let authored = UiDocument {
        outcomes: Some(vec![outcome_entry("custom:99", "alien")]),
        ..empty_ui()
    };
    let mut expected = vec![outcome_entry("custom:99", "alien")];
    expected.extend(d_outcomes.clone());
    assert_eq!(
        effective_ui(Some(&authored), &source).outcomes,
        Some(expected)
    );

    // Screens: authored first (sections kept), then the missing defaults.
    let authored = UiDocument {
        screens: Some(vec![
            screen(UiScreenKind::CombatHud),
            screen(UiScreenKind::Spellbook),
        ]),
        ..empty_ui()
    };
    let merged = effective_ui(Some(&authored), &source);
    let screens = merged.screens.unwrap();
    assert_eq!(
        screens.iter().map(|s| s.kind.as_str()).collect::<Vec<_>>(),
        vec![
            "combat_hud",
            "spellbook",
            "character_sheet",
            "action_bar",
            "inventory"
        ]
    );
    assert!(screens[0].sections.is_empty() && screens[1].sections.is_empty());
    assert_eq!(screens[2], d_screens[0]);
    assert_eq!(screens[3], d_screens[1]);
    assert_eq!(screens[4], d_screens[3]);
}
