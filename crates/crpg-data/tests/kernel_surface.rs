#![forbid(unsafe_code)]
//! T136b: kernel-range surface on `crpg.ruleset/2`, `crpg.ability/2` and
//! `crpg.effect/1` (typed and derived stats, resolution shapes, damage dice,
//! initiative, stat-target effects and tag grants). D32c: no version bump.
use crpg_core::{Fx16_16, Ulid};
use crpg_data::{
    Ability, DamageEntry, DataError, DefenseWire, Diagnostic, DiagnosticCode, Document, Effect,
    EffectTargetWire, ExprWire, InitiativeWire, LoadedCampaign, OutcomeWire, Ruleset, Severity,
    SourcePath, StatDecl, StatKindWire, StatValueWire,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn json_file(relative: &str) -> Value {
    let bytes = std::fs::read(repo_root().join(relative)).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn srd_ruleset() -> Value {
    json_file("rulesets/srd-lite/ruleset.json")
}

fn heavy() -> Value {
    json_file("rulesets/srd-lite/heavy.json")
}

fn focusing() -> Value {
    json_file("rulesets/srd-lite/focusing.json")
}

fn read(value: &Value) -> Result<Document, DataError> {
    crpg_data::read_document(&serde_json::to_vec(value).unwrap())
}

fn read_ok(value: &Value) -> Document {
    read(value).unwrap_or_else(|err| panic!("document should read: {err}"))
}

fn message(result: Result<Document, DataError>) -> String {
    match result {
        Err(DataError::Malformed { message, .. }) => message,
        other => panic!("expected Malformed, got {other:?}"),
    }
}

fn is_malformed(value: &Value) -> bool {
    matches!(read(value), Err(DataError::Malformed { .. }))
}

fn written(doc: &Document) -> Vec<u8> {
    crpg_data::write_document(doc).unwrap()
}

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap()
}

fn with(base: &Value, key: &str, value: Value) -> Value {
    let mut out = base.clone();
    out[key] = value;
    out
}

fn ruleset_of(doc: Document) -> Ruleset {
    match doc {
        Document::Ruleset(ruleset) => ruleset,
        other => panic!("expected ruleset, got {other:?}"),
    }
}

fn ability_of(doc: Document) -> Ability {
    match doc {
        Document::Ability(ability) => ability,
        other => panic!("expected ability, got {other:?}"),
    }
}

fn effect_of(doc: Document) -> Effect {
    match doc {
        Document::Effect(effect) => effect,
        other => panic!("expected effect, got {other:?}"),
    }
}

// ---- wire builders --------------------------------------------------------

fn lit_int(n: i64) -> Value {
    json!({"type": "literal", "value": {"type": "int", "value": n}})
}

fn stat_ref(name: &str) -> Value {
    json!({"type": "stat", "value": name})
}

fn bin(op: &str, left: Value, right: Value) -> Value {
    json!({"type": op, "value": {"left": left, "right": right}})
}

fn int_stat(name: &str) -> Value {
    json!({"name": name, "kind": "int"})
}

fn derived_int(name: &str, expr: Value) -> Value {
    json!({"name": name, "kind": "int", "derived": expr})
}

fn enum_stat(name: &str, enum_id: &str, variants: Value) -> Value {
    json!({"name": name, "kind": {"enum": {"enum_id": enum_id, "variants": variants}}})
}

/// The srd-lite ruleset with `extra` stats appended.
fn rs_with(extra: Vec<Value>) -> Value {
    let mut ruleset = srd_ruleset();
    for stat in extra {
        ruleset["stats"].as_array_mut().unwrap().push(stat);
    }
    ruleset
}

/// A full binary tree of exactly `nodes` (odd) nodes, balanced.
fn tree(nodes: usize) -> Value {
    assert!(!nodes.is_multiple_of(2));
    if nodes == 1 {
        return lit_int(1);
    }
    let rest = nodes - 1;
    let mut left = rest / 2;
    if left.is_multiple_of(2) {
        left -= 1;
    }
    bin("add", tree(left), tree(rest - left))
}

/// A right-leaning chain of the given depth (root at depth one).
fn caterpillar(depth: usize) -> Value {
    if depth == 1 {
        return lit_int(1);
    }
    bin("add", lit_int(1), caterpillar(depth - 1))
}

fn expect_message(ruleset: &Value, expected: &str) {
    assert_eq!(message(read(ruleset)), expected);
}

// ---- T2 (behaviour 1) -----------------------------------------------------

#[test]
fn t2_every_kind_round_trips_and_int_sources_keep_their_bytes() {
    let ruleset = rs_with(vec![
        json!({"name": "f", "kind": "fixed"}),
        json!({"name": "b", "kind": "bool"}),
        enum_stat("e", "mood", json!(["calm", "angry"])),
        json!({"name": "t", "kind": "tags"}),
        json!({"name": "d", "kind": "dice"}),
    ]);
    let doc = read_ok(&ruleset);
    let kinds: Vec<StatKindWire> = ruleset_of(doc.clone())
        .stats
        .into_iter()
        .skip(5)
        .map(|stat| stat.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            StatKindWire::Fixed,
            StatKindWire::Bool,
            StatKindWire::Enum {
                enum_id: "mood".into(),
                variants: vec!["calm".into(), "angry".into()],
            },
            StatKindWire::Tags,
            StatKindWire::Dice,
        ]
    );
    let bytes = written(&doc);
    assert_eq!(crpg_data::read_document(&bytes).unwrap(), doc);
    // The enum is externally tagged; the scalar kinds are bare strings.
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["stats"][5]["kind"], json!("fixed"));
    assert_eq!(
        value["stats"][7]["kind"],
        json!({"enum": {"enum_id": "mood", "variants": ["calm", "angry"]}})
    );
    // A source that only uses "int" keeps its canonical bytes.
    let source = srd_ruleset();
    assert_eq!(
        written(&read_ok(&source)),
        crpg_data::canonical_json(&source).unwrap()
    );
}

#[test]
fn t2_malformed_kind_shapes_are_rejected() {
    let bad_kinds = [
        json!("enum"),
        json!("Int"),
        json!({"int": null}),
        json!({"enum": {"enum_id": "m", "variants": ["a"]}, "x": 0}),
        json!({"enum": {"variants": ["a"]}}),
        json!({"enum": {"enum_id": "m"}}),
        json!({"enum": {"enum_id": "m", "variants": ["a"], "x": 0}}),
        json!({"enum": {"enum_id": 1, "variants": ["a"]}}),
        json!({"enum": {"enum_id": "m", "variants": [1]}}),
        json!({"enum": "m"}),
        json!({"type": "int"}),
        json!(null),
        json!(3),
        json!(["int"]),
    ];
    for kind in bad_kinds {
        let mut ruleset = srd_ruleset();
        ruleset["stats"][0]["kind"] = kind.clone();
        assert!(is_malformed(&ruleset), "kind {kind} must be Malformed");
    }
}

// ---- T3 (behaviour 2) -----------------------------------------------------

fn all_arms_ruleset() -> Value {
    let mut extra = vec![
        json!({"name": "v_fixed", "kind": "fixed",
            "derived": {"type": "literal", "value": {"type": "fixed", "value": 65536}}}),
        json!({"name": "v_bool", "kind": "bool",
            "derived": {"type": "literal", "value": {"type": "bool", "value": true}}}),
        json!({"name": "e", "kind": {"enum": {"enum_id": "mood", "variants": ["calm", "angry"]}},
            "derived": {"type": "literal", "value": {"type": "enum",
                "value": {"enum_id": "mood", "variant": "calm"}}}}),
        json!({"name": "v_tags", "kind": "tags",
            "derived": {"type": "literal", "value": {"type": "tags", "value": ["a", "b"]}}}),
        json!({"name": "v_dice", "kind": "dice",
            "derived": {"type": "literal", "value": {"type": "dice", "value": "1d6"}}}),
        derived_int("v_int", lit_int(5)),
        derived_int("by_stat", stat_ref("might")),
    ];
    for op in ["add", "subtract", "multiply", "divide", "min", "max"] {
        extra.push(derived_int(
            &format!("op_{op}"),
            bin(op, stat_ref("might"), lit_int(2)),
        ));
    }
    rs_with(extra)
}

#[test]
fn t3_all_expression_and_value_arms_round_trip() {
    let ruleset = all_arms_ruleset();
    let doc = read_ok(&ruleset);
    let typed = ruleset_of(doc.clone());
    let derived: Vec<&ExprWire> = typed
        .stats
        .iter()
        .filter_map(|stat| stat.derived.as_ref())
        .collect();
    assert_eq!(derived.len(), 13);
    assert!(matches!(
        derived[6],
        ExprWire::Stat(name) if name == "might"
    ));
    assert!(matches!(derived[7], ExprWire::Add { .. }));
    assert!(matches!(derived[8], ExprWire::Subtract { .. }));
    assert!(matches!(derived[9], ExprWire::Multiply { .. }));
    assert!(matches!(derived[10], ExprWire::Divide { .. }));
    assert!(matches!(derived[11], ExprWire::Min { .. }));
    assert!(matches!(derived[12], ExprWire::Max { .. }));
    assert_eq!(
        derived[0],
        &ExprWire::Literal(StatValueWire::Fixed(Fx16_16::from_raw(65536)))
    );
    assert_eq!(derived[1], &ExprWire::Literal(StatValueWire::Bool(true)));
    assert_eq!(
        derived[2],
        &ExprWire::Literal(StatValueWire::Enum {
            enum_id: "mood".into(),
            variant: "calm".into()
        })
    );
    assert_eq!(
        derived[3],
        &ExprWire::Literal(StatValueWire::Tags(vec!["a".into(), "b".into()]))
    );
    assert_eq!(
        derived[4],
        &ExprWire::Literal(StatValueWire::Dice("1d6".into()))
    );
    assert_eq!(derived[5], &ExprWire::Literal(StatValueWire::Int(5)));
    let bytes = written(&doc);
    assert_eq!(crpg_data::read_document(&bytes).unwrap(), doc);
    assert_eq!(
        written(&crpg_data::read_document(&bytes).unwrap()),
        bytes,
        "canonical output is stable"
    );
}

#[test]
fn t3_null_derived_reads_as_absent() {
    let mut ruleset = srd_ruleset();
    ruleset["stats"][0]["derived"] = Value::Null;
    let doc = read_ok(&ruleset);
    assert_eq!(ruleset_of(doc.clone()).stats[0].derived, None);
    assert!(!text(&written(&doc)).contains("derived"));
}

#[test]
fn t3_expression_rejections() {
    let derived_at = |expr: Value| {
        let mut ruleset = srd_ruleset();
        ruleset["stats"][0]["derived"] = expr;
        ruleset
    };
    let rejected = [
        // Extra key at the outer level.
        json!({"type": "stat", "value": "ward", "x": 0}),
        // Unknown names.
        json!({"type": "negate", "value": "ward"}),
        // Missing value.
        json!({"type": "stat"}),
        json!({"type": "add"}),
        // Extra and missing keys inside a binary.
        json!({"type": "add", "value": {"left": lit_int(1), "right": lit_int(1), "x": 0}}),
        json!({"type": "add", "value": {"left": lit_int(1)}}),
        // Literal inner level.
        json!({"type": "literal", "value": {"type": "int", "value": 1, "x": 0}}),
        json!({"type": "literal", "value": {"type": "int"}}),
        json!({"type": "literal", "value": {"type": "money", "value": 1}}),
        json!({"type": "literal", "value": {"type": "int", "value": 2147483648_i64}}),
        json!({"type": "literal", "value": {"type": "int", "value": "1"}}),
        json!({"type": "literal", "value": {"type": "bool", "value": 1}}),
        json!({"type": "literal", "value": {"type": "fixed", "value": 4294967296_i64}}),
        json!({"type": "literal", "value": {"type": "enum", "value": {"enum_id": "m"}}}),
        json!({"type": "literal", "value": {"type": "enum",
            "value": {"enum_id": "m", "variant": "a", "x": 0}}}),
        json!({"type": "literal", "value": {"type": "tags", "value": "a"}}),
        json!({"type": "literal", "value": {"type": "dice", "value": 6}}),
        json!("stat"),
        json!(1),
    ];
    for expr in rejected {
        assert!(is_malformed(&derived_at(expr.clone())), "{expr}");
    }
}

// ---- T4 (behaviours 3, 4) -------------------------------------------------

#[test]
fn t4_optional_keys_absent_null_and_present() {
    // initiative (ruleset)
    let absent = ruleset_of(read_ok(&srd_ruleset()));
    assert_eq!(absent.initiative, None);
    let null = ruleset_of(read_ok(&with(&srd_ruleset(), "initiative", Value::Null)));
    assert_eq!(null.initiative, None);
    let present = read_ok(&with(
        &srd_ruleset(),
        "initiative",
        json!({"dice": "1d20", "bonus_stats": ["guile", "might"]}),
    ));
    assert_eq!(
        ruleset_of(present.clone()).initiative,
        Some(InitiativeWire {
            dice: "1d20".into(),
            bonus_stats: vec!["guile".into(), "might".into()],
        })
    );
    let bytes = written(&present);
    assert!(text(&bytes).contains("\"initiative\": {"));
    assert!(text(&bytes).find("guile").unwrap() < text(&bytes).find("might\"\n").unwrap());
    assert_eq!(crpg_data::read_document(&bytes).unwrap(), present);
    assert!(!text(&written(&Document::Ruleset(null))).contains("initiative"));
    assert!(!text(&written(&Document::Ruleset(absent))).contains("initiative"));

    // bonus_stats (ability)
    let absent = ability_of(read_ok(&heavy()));
    assert_eq!(absent.bonus_stats, None);
    let null = ability_of(read_ok(&with(&heavy(), "bonus_stats", Value::Null)));
    assert_eq!(null.bonus_stats, None);
    let present = read_ok(&with(&heavy(), "bonus_stats", json!(["might", "guile"])));
    assert_eq!(
        ability_of(present.clone()).bonus_stats,
        Some(vec!["might".into(), "guile".into()])
    );
    let bytes = written(&present);
    assert_eq!(crpg_data::read_document(&bytes).unwrap(), present);
    assert!(!text(&written(&Document::Ability(null))).contains("bonus_stats"));
    assert!(!text(&written(&Document::Ability(absent))).contains("bonus_stats"));

    // damage dice
    let mut value = heavy();
    value["damage"][1]["dice"] = json!("1d8");
    value["damage"][2]["dice"] = Value::Null;
    let doc = read_ok(&value);
    let ability = ability_of(doc.clone());
    assert_eq!(ability.damage[0].dice, None);
    assert_eq!(ability.damage[1].dice.as_deref(), Some("1d8"));
    assert_eq!(ability.damage[2].dice, None);
    let bytes = written(&doc);
    assert_eq!(text(&bytes).matches("\"dice\": \"1d8\"").count(), 1);
    assert_eq!(crpg_data::read_document(&bytes).unwrap(), doc);

    // grants (effect)
    let absent = effect_of(read_ok(&focusing()));
    assert_eq!(absent.grants, None);
    let null = effect_of(read_ok(&with(&focusing(), "grants", Value::Null)));
    assert_eq!(null.grants, None);
    let present = read_ok(&with(&focusing(), "grants", json!(["z", "a"])));
    assert_eq!(
        effect_of(present.clone()).grants,
        Some(vec!["z".into(), "a".into()])
    );
    let bytes = written(&present);
    assert!(text(&bytes).find("\"z\"").unwrap() < text(&bytes).find("\"a\"").unwrap());
    assert_eq!(crpg_data::read_document(&bytes).unwrap(), present);
    assert!(!text(&written(&Document::Effect(null))).contains("grants"));
    assert!(!text(&written(&Document::Effect(absent))).contains("grants"));
}

#[test]
fn t4_initiative_requires_both_keys_and_allows_an_empty_list() {
    for bad in [
        json!({"dice": "1d20"}),
        json!({"bonus_stats": []}),
        json!({"dice": "1d20", "bonus_stats": [], "x": 0}),
    ] {
        assert!(is_malformed(&with(&srd_ruleset(), "initiative", bad)));
    }
    let doc = read_ok(&with(
        &srd_ruleset(),
        "initiative",
        json!({"dice": "1d20", "bonus_stats": []}),
    ));
    assert_eq!(
        ruleset_of(doc).initiative,
        Some(InitiativeWire {
            dice: "1d20".into(),
            bonus_stats: Vec::new()
        })
    );
}

#[test]
fn t4_defense_and_target_forms() {
    let defense = |value: Value| with(&heavy(), "defense", value);
    let doc = read_ok(&defense(json!({"type": "constant", "value": 12})));
    assert_eq!(ability_of(doc.clone()).defense, DefenseWire::Constant(12));
    assert!(text(&written(&doc)).contains("\"type\": \"constant\""));
    let doc = read_ok(&defense(json!({"type": "constant", "value": -3})));
    assert_eq!(ability_of(doc).defense, DefenseWire::Constant(-3));
    let doc = read_ok(&defense(
        json!({"type": "actor_stat", "value": {"stat": "guile"}}),
    ));
    assert_eq!(
        ability_of(doc.clone()).defense,
        DefenseWire::ActorStat {
            stat: "guile".into()
        }
    );
    assert_eq!(crpg_data::read_document(&written(&doc)).unwrap(), doc);
    let doc = read_ok(&defense(
        json!({"type": "opposed", "value": {"bonus_stats": ["might"], "dice": "2d6"}}),
    ));
    assert_eq!(
        ability_of(doc.clone()).defense,
        DefenseWire::Opposed {
            dice: "2d6".into(),
            bonus_stats: vec!["might".into()]
        }
    );
    assert_eq!(crpg_data::read_document(&written(&doc)).unwrap(), doc);
    let empty = read_ok(&defense(
        json!({"type": "opposed", "value": {"bonus_stats": [], "dice": "2d6"}}),
    ));
    assert_eq!(crpg_data::read_document(&written(&empty)).unwrap(), empty);

    for bad in [
        json!({"type": "constant"}),
        json!({"type": "constant", "value": "12"}),
        json!({"type": "constant", "value": 2147483648_i64}),
        json!({"type": "constant", "value": 12, "x": 0}),
        json!({"type": "actor_stat"}),
        json!({"type": "actor_stat", "value": {}}),
        json!({"type": "actor_stat", "value": {"stat": "guile", "x": 0}}),
        json!({"type": "opposed", "value": {"dice": "2d6"}}),
        json!({"type": "opposed", "value": {"bonus_stats": []}}),
        json!({"type": "opposed", "value": {"bonus_stats": [], "dice": "2d6", "x": 0}}),
        json!({"type": "opposed"}),
        json!({"type": "wild", "value": 1}),
    ] {
        assert!(is_malformed(&defense(bad.clone())), "{bad}");
    }

    let target = |value: Value| {
        let mut effect = focusing();
        effect["modifiers"][0]["target"] = value;
        effect
    };
    let doc = read_ok(&target(json!({"type": "stat", "value": {"stat": "might"}})));
    assert_eq!(
        effect_of(doc.clone()).modifiers[0].target,
        EffectTargetWire::Stat {
            stat: "might".into()
        }
    );
    assert_eq!(crpg_data::read_document(&written(&doc)).unwrap(), doc);
    for bad in [
        json!({"type": "stat"}),
        json!({"type": "stat", "value": {}}),
        json!({"type": "stat", "value": {"stat": "might", "x": 0}}),
        json!({"type": "stat", "value": "might"}),
        json!({"type": "roll", "value": {"stat": "might"}}),
    ] {
        assert!(is_malformed(&target(bad.clone())), "{bad}");
    }
}

// ---- T5 (behaviour 5) -----------------------------------------------------

fn names(prefix: &str, count: usize) -> Vec<String> {
    (0..count).map(|n| format!("{prefix}{n}")).collect()
}

#[test]
fn t5_enum_rules_and_bounds() {
    expect_message(
        &rs_with(vec![enum_stat("e", "", json!(["a"]))]),
        "ruleset enum stat enum_id must be nonempty",
    );
    expect_message(
        &rs_with(vec![enum_stat("e", "m", json!([]))]),
        "ruleset enum stat variants must be 1..=1024",
    );
    expect_message(
        &rs_with(vec![enum_stat("e", "m", json!(names("v", 1025)))]),
        "ruleset enum stat variants must be 1..=1024",
    );
    read_ok(&rs_with(vec![enum_stat("e", "m", json!(names("v", 1024)))]));
    expect_message(
        &rs_with(vec![enum_stat("e", "m", json!(["a", "a"]))]),
        "ruleset enum stat variants must be distinct and nonempty",
    );
    expect_message(
        &rs_with(vec![enum_stat("e", "m", json!(["a", ""]))]),
        "ruleset enum stat variants must be distinct and nonempty",
    );
    // Shared domains: same set in another order is clean; a different set is not.
    read_ok(&rs_with(vec![
        enum_stat("e1", "m", json!(["a", "b"])),
        enum_stat("e2", "m", json!(["b", "a"])),
    ]));
    expect_message(
        &rs_with(vec![
            enum_stat("e1", "m", json!(["a", "b"])),
            enum_stat("e2", "m", json!(["a", "c"])),
        ]),
        "ruleset enum stats sharing an enum_id must declare the same variants",
    );
}

#[test]
fn t5_health_and_attribute_kinds() {
    let mut ruleset = srd_ruleset();
    ruleset["stats"][4]["derived"] = lit_int(5);
    expect_message(
        &ruleset,
        "ruleset health_stat must name a non-derived int stat",
    );
    let mut ruleset = srd_ruleset();
    ruleset["stats"][4]["kind"] = json!("fixed");
    expect_message(
        &ruleset,
        "ruleset health_stat must name a non-derived int stat",
    );
    let mut ruleset = srd_ruleset();
    ruleset["stats"][0]["kind"] = json!("bool");
    expect_message(&ruleset, "ruleset attributes must name int stats");
    // A derived Int attribute is allowed.
    let mut ruleset = srd_ruleset();
    ruleset["stats"][0] = derived_int("might", lit_int(3));
    read_ok(&ruleset);
}

#[test]
fn t5_expression_size_bounds() {
    let nodes_message = "ruleset derived expression must be at most 4096 nodes and 32 deep";
    // Full binary trees have an odd node count, so 4095 is the largest
    // reachable count at or below the 4096 bound; 4097 is the first past it.
    read_ok(&rs_with(vec![derived_int("big", tree(4095))]));
    expect_message(
        &rs_with(vec![derived_int("big", tree(4097))]),
        nodes_message,
    );
    read_ok(&rs_with(vec![derived_int("deep", caterpillar(32))]));
    expect_message(
        &rs_with(vec![derived_int("deep", caterpillar(33))]),
        nodes_message,
    );

    let total_message = "ruleset derived expressions must total at most 65536 nodes";
    let many = |sizes: &[usize]| {
        rs_with(
            sizes
                .iter()
                .enumerate()
                .map(|(n, size)| derived_int(&format!("s{n}"), tree(*size)))
                .collect(),
        )
    };
    let mut at_bound = vec![3641; 17];
    at_bound.push(3639);
    assert_eq!(at_bound.iter().sum::<usize>(), 65_536);
    read_ok(&many(&at_bound));
    let mut past_bound = vec![3449; 18];
    past_bound.push(3455);
    assert_eq!(past_bound.iter().sum::<usize>(), 65_537);
    expect_message(&many(&past_bound), total_message);
}

#[test]
fn t5_literals_and_references() {
    let tags = |count: usize| {
        json!({"name": "t", "kind": "tags", "derived": {"type": "literal",
            "value": {"type": "tags", "value": names("g", count)}}})
    };
    let tags_message = "ruleset derived tags literal must hold at most 1024 distinct nonempty tags";
    read_ok(&rs_with(vec![tags(1024)]));
    expect_message(&rs_with(vec![tags(1025)]), tags_message);
    for bad in [json!(["a", "a"]), json!(["a", ""])] {
        expect_message(
            &rs_with(vec![json!({"name": "t", "kind": "tags", "derived": {
                "type": "literal", "value": {"type": "tags", "value": bad}}})]),
            tags_message,
        );
    }
    let dice = |text: &str| {
        json!({"name": "d", "kind": "dice", "derived": {
            "type": "literal", "value": {"type": "dice", "value": text}}})
    };
    let dice_message = "ruleset derived dice literal must be 1..=128 UTF-8 bytes";
    read_ok(&rs_with(vec![dice(&"d".repeat(128))]));
    expect_message(&rs_with(vec![dice(&"d".repeat(129))]), dice_message);
    expect_message(&rs_with(vec![dice("")]), dice_message);

    let enum_message = "ruleset derived enum literal must name a declared enum variant";
    let enum_lit = |id: &str, variant: &str| {
        json!({"name": "e", "kind": {"enum": {"enum_id": "mood", "variants": ["calm"]}},
            "derived": {"type": "literal", "value": {"type": "enum",
                "value": {"enum_id": id, "variant": variant}}}})
    };
    read_ok(&rs_with(vec![enum_lit("mood", "calm")]));
    expect_message(&rs_with(vec![enum_lit("mood", "angry")]), enum_message);
    // A literal from a domain declared by no stat of this ruleset.
    expect_message(&rs_with(vec![enum_lit("weather", "calm")]), enum_message);

    expect_message(
        &rs_with(vec![derived_int("x", stat_ref("nope"))]),
        "ruleset derived expression must reference declared stats",
    );
    // Preorder: the first failing node decides (left literal before right stat).
    expect_message(
        &rs_with(vec![derived_int(
            "x",
            bin(
                "add",
                json!({"type": "literal", "value": {"type": "dice", "value": ""}}),
                stat_ref("nope"),
            ),
        )]),
        dice_message,
    );
}

#[test]
fn t5_cycles_and_chains() {
    let cycle = "ruleset derived stats must not form a cycle";
    expect_message(&rs_with(vec![derived_int("loop", stat_ref("loop"))]), cycle);
    expect_message(
        &rs_with(vec![
            derived_int("a", stat_ref("b")),
            derived_int("b", stat_ref("a")),
        ]),
        cycle,
    );
    let chain = |length: usize| {
        let mut extra = vec![int_stat("c0")];
        for n in 1..length {
            extra.push(derived_int(
                &format!("c{n}"),
                stat_ref(&format!("c{}", n - 1)),
            ));
        }
        rs_with(extra)
    };
    read_ok(&chain(128));
    expect_message(
        &chain(129),
        "ruleset derived dependency chains must be at most 128 stats",
    );
    // Forward references in authored order are fine.
    read_ok(&rs_with(vec![
        derived_int("late", stat_ref("early")),
        derived_int("early", lit_int(1)),
    ]));
}

#[test]
fn t5_kind_matching() {
    let mismatch = "ruleset derived expression kind must match its stat";
    let fixed = json!({"type": "literal", "value": {"type": "fixed", "value": 65536}});
    expect_message(
        &rs_with(vec![derived_int(
            "x",
            bin("add", lit_int(1), fixed.clone()),
        )]),
        mismatch,
    );
    expect_message(
        &rs_with(vec![derived_int(
            "x",
            json!({"type": "literal", "value": {"type": "bool", "value": true}}),
        )]),
        mismatch,
    );
    // Fixed root under an Int stat, and Fixed + Fixed under a Fixed stat.
    expect_message(&rs_with(vec![derived_int("x", fixed.clone())]), mismatch);
    read_ok(&rs_with(vec![json!({
        "name": "x", "kind": "fixed",
        "derived": bin("multiply", fixed.clone(), fixed.clone())})]));
    // Enum roots need the same enum_id.
    let mood = json!({"enum": {"enum_id": "mood", "variants": ["calm"]}});
    let other = json!({"enum": {"enum_id": "weather", "variants": ["calm"]}});
    let literal = json!({"type": "literal", "value": {"type": "enum",
        "value": {"enum_id": "weather", "variant": "calm"}}});
    expect_message(
        &rs_with(vec![
            json!({"name": "w", "kind": other.clone()}),
            json!({"name": "m", "kind": mood, "derived": literal.clone()}),
        ]),
        mismatch,
    );
    read_ok(&rs_with(vec![json!({
        "name": "w", "kind": other, "derived": literal})]));
    // A stat node takes that stat's declared kind.
    expect_message(
        &rs_with(vec![
            json!({"name": "flag", "kind": "bool"}),
            derived_int("x", stat_ref("flag")),
        ]),
        mismatch,
    );
}

#[test]
fn t5_initiative_rules() {
    let rule = |dice: &str, bonus: Value| {
        with(
            &srd_ruleset(),
            "initiative",
            json!({"dice": dice, "bonus_stats": bonus}),
        )
    };
    let dice_message = "ruleset initiative dice must be 1..=128 UTF-8 bytes";
    expect_message(&rule("", json!([])), dice_message);
    read_ok(&rule(&"d".repeat(128), json!([])));
    expect_message(&rule(&"d".repeat(129), json!([])), dice_message);
    let stats_message = "ruleset initiative bonus stats must name distinct non-health int stats";
    for bad in [
        json!(["nope"]),
        json!([""]),
        json!(["health"]),
        json!(["might", "might"]),
    ] {
        expect_message(&rule("1d20", bad.clone()), stats_message);
    }
    let mut ruleset = rule("1d20", json!(["flag"]));
    ruleset["stats"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "flag", "kind": "bool"}));
    expect_message(&ruleset, stats_message);
    // A derived Int stat is a valid bonus.
    let mut ruleset = rule("1d20", json!(["bonus"]));
    ruleset["stats"]
        .as_array_mut()
        .unwrap()
        .push(derived_int("bonus", lit_int(2)));
    read_ok(&ruleset);
}

// ---- T6 (behaviours 6, 7) -------------------------------------------------

fn ability_message(value: Value, expected: &str) {
    assert_eq!(message(read(&value)), expected);
}

#[test]
fn t6_ability_checks() {
    let bonus_message = "ability bonus_stats must be a nonempty list of distinct nonempty names";
    for bad in [json!([]), json!(["might", "might"]), json!([""])] {
        ability_message(with(&heavy(), "bonus_stats", bad), bonus_message);
    }
    let opposed = |dice: &str, bonus: Value| {
        with(
            &heavy(),
            "defense",
            json!({"type": "opposed", "value": {"bonus_stats": bonus, "dice": dice}}),
        )
    };
    let dice_message = "ability opposed dice must be 1..=128 UTF-8 bytes";
    ability_message(opposed("", json!([])), dice_message);
    read_ok(&opposed(&"d".repeat(128), json!([])));
    ability_message(opposed(&"d".repeat(129), json!([])), dice_message);
    let stats_message = "ability opposed bonus_stats must be distinct nonempty names";
    ability_message(opposed("1d20", json!(["a", "a"])), stats_message);
    ability_message(opposed("1d20", json!([""])), stats_message);
    // Dice are checked before the stat names.
    ability_message(opposed("", json!([""])), dice_message);

    let damage_message = "ability damage dice must be 1..=128 UTF-8 bytes";
    let damage = |dice: &str| {
        let mut value = heavy();
        value["damage"][2]["dice"] = json!(dice);
        value
    };
    read_ok(&damage(&"d".repeat(128)));
    ability_message(damage(&"d".repeat(129)), damage_message);
    ability_message(damage(""), damage_message);

    ability_message(
        with(
            &heavy(),
            "defense",
            json!({"type": "actor_stat", "value": {"stat": ""}}),
        ),
        "ability defense stat must be nonempty",
    );
    ability_message(
        with(
            &heavy(),
            "defense",
            json!({"type": "target_stat", "value": {"stat": ""}}),
        ),
        "ability defense stat must be nonempty",
    );
}

#[test]
fn t6_effect_checks() {
    let target = |stat: &str| {
        let mut effect = focusing();
        effect["modifiers"][1]["target"] = json!({"type": "stat", "value": {"stat": stat}});
        effect
    };
    ability_message(target(""), "effect modifier stat must be nonempty");
    read_ok(&target("might"));
    let grants_message = "effect grants must hold 1..=1024 distinct nonempty tags";
    let grants = |tags: Value| with(&focusing(), "grants", tags);
    read_ok(&grants(json!(names("g", 1024))));
    ability_message(grants(json!(names("g", 1025))), grants_message);
    for bad in [json!([]), json!(["a", "a"]), json!(["a", ""])] {
        ability_message(grants(bad), grants_message);
    }
}

// ---- T7 (behaviour 8) -----------------------------------------------------

#[test]
fn t7_new_checks_come_after_existing_ones() {
    let mut ability = heavy();
    ability["attribute"] = json!("");
    ability["bonus_stats"] = json!([""]);
    ability_message(ability, "ability attribute must be nonempty");

    let mut ruleset = rs_with(vec![int_stat("might")]);
    ruleset["stats"]
        .as_array_mut()
        .unwrap()
        .push(derived_int("bad", stat_ref("nope")));
    expect_message(&ruleset, "duplicate ruleset stat name");

    let mut effect = focusing();
    effect["mod_type"] = json!("");
    effect["modifiers"][0]["target"] = json!({"type": "stat", "value": {"stat": ""}});
    effect["grants"] = json!([]);
    ability_message(effect, "effect mod_type must be nonempty");

    // The widened defense check keeps its place before recovery_ticks.
    let mut ability = heavy();
    ability["defense"] = json!({"type": "actor_stat", "value": {"stat": ""}});
    ability["recovery_ticks"] = json!(0);
    ability_message(ability, "ability defense stat must be nonempty");

    // Existing ruleset checks precede the new ones.
    let mut ruleset = srd_ruleset();
    ruleset["round_ticks"] = json!(0);
    ruleset["stats"][4]["derived"] = lit_int(1);
    expect_message(&ruleset, "ruleset round_ticks must be nonzero");
}

// ---- campaign-level helpers (T8-T10) --------------------------------------

fn read_tree(root: &Path) -> BTreeMap<SourcePath, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<SourcePath, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
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
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn srd_docs() -> BTreeMap<SourcePath, Document> {
    let files = read_tree(&repo_root().join("campaigns/fixtures/combat_srd"));
    let engine: semver::Version = "0.1.0".parse().unwrap();
    crpg_data::load_campaign(&files, &engine)
        .expect("combat_srd loads")
        .documents
}

fn validate_docs(docs: &BTreeMap<SourcePath, Document>) -> Vec<Diagnostic> {
    crpg_data::validate(&LoadedCampaign {
        documents: docs.clone(),
        index: BTreeMap::new(),
    })
}

fn mutate_srd(change: impl FnOnce(&mut BTreeMap<SourcePath, Document>)) -> Vec<Diagnostic> {
    let mut docs = srd_docs();
    change(&mut docs);
    validate_docs(&docs)
}

fn id(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

const RULESET_ID: u128 = 38;
const HEAVY_ID: u128 = 39;
const FOCUS_ID: u128 = 40;
const EFFECT_ID: u128 = 43;

fn with_ruleset(docs: &mut BTreeMap<SourcePath, Document>, change: impl FnOnce(&mut Ruleset)) {
    for doc in docs.values_mut() {
        if let Document::Ruleset(ruleset) = doc {
            change(ruleset);
            return;
        }
    }
}

fn with_ability(
    docs: &mut BTreeMap<SourcePath, Document>,
    which: u128,
    change: impl FnOnce(&mut Ability),
) {
    for doc in docs.values_mut() {
        if let Document::Ability(ability) = doc {
            if ability.id == id(which) {
                change(ability);
                return;
            }
        }
    }
    panic!("ability {which} not found");
}

fn with_effect(docs: &mut BTreeMap<SourcePath, Document>, change: impl FnOnce(&mut Effect)) {
    for doc in docs.values_mut() {
        if let Document::Effect(effect) = doc {
            change(effect);
            return;
        }
    }
}

fn path_of(docs: &BTreeMap<SourcePath, Document>, want: impl Fn(&Document) -> bool) -> SourcePath {
    docs.iter()
        .find(|(_, doc)| want(doc))
        .map(|(path, _)| path.clone())
        .expect("document present")
}

fn add_stat(docs: &mut BTreeMap<SourcePath, Document>, name: &str, kind: StatKindWire) {
    with_ruleset(docs, |ruleset| {
        ruleset.stats.push(StatDecl {
            name: name.into(),
            kind,
            derived: None,
        })
    });
}

fn only(findings: Vec<Diagnostic>) -> Diagnostic {
    assert_eq!(findings.len(), 1, "{findings:?}");
    findings.into_iter().next().unwrap()
}

// ---- T8 (behaviour 9) -----------------------------------------------------

/// Names that are invalid as non-health int stats, with the stat added first.
const BAD_STATS: [&str; 3] = ["nope", "health", "flag"];

fn prepare_flag(docs: &mut BTreeMap<SourcePath, Document>) {
    add_stat(docs, "flag", StatKindWire::Bool);
}

#[test]
fn t8_bonus_stats_pointer_and_message() {
    for name in BAD_STATS {
        let diag = only(mutate_srd(|docs| {
            prepare_flag(docs);
            with_ability(docs, HEAVY_ID, |a| a.bonus_stats = Some(vec![name.into()]));
        }));
        assert_eq!(diag.code, DiagnosticCode::UnknownStat);
        assert_eq!(diag.severity, Severity::Error);
        assert_eq!(diag.pointer, "/bonus_stats/0");
        assert_eq!(
            diag.message,
            format!(
                "unknown bonus stat {name:?} for ability: not a non-health int stat of ruleset {}",
                id(RULESET_ID)
            )
        );
        assert_eq!(
            diag.suggested_fix.as_deref(),
            Some("use a declared non-health int stat of the ruleset")
        );
    }
    // A valid Int stat is clean, and only the invalid entry is reported.
    assert!(mutate_srd(|docs| {
        with_ability(docs, HEAVY_ID, |a| {
            a.bonus_stats = Some(vec!["might".into(), "ward".into()])
        })
    })
    .is_empty());
    let diag = only(mutate_srd(|docs| {
        with_ability(docs, HEAVY_ID, |a| {
            a.bonus_stats = Some(vec!["might".into(), "nope".into()])
        })
    }));
    assert_eq!(diag.pointer, "/bonus_stats/1");
}

#[test]
fn t8_defense_stat_messages() {
    for make in [
        |stat: String| DefenseWire::TargetStat { stat },
        |stat: String| DefenseWire::ActorStat { stat },
    ] {
        for name in ["nope", "health"] {
            let diag = only(mutate_srd(|docs| {
                with_ability(docs, HEAVY_ID, |a| a.defense = make(name.into()));
            }));
            assert_eq!(diag.code, DiagnosticCode::UnknownStat);
            assert_eq!(diag.pointer, "/defense");
            assert_eq!(
                diag.message,
                format!(
                    "unknown defense stat {name:?} for ability: not a non-health declared stat of ruleset {}",
                    id(RULESET_ID)
                )
            );
            assert_eq!(
                diag.suggested_fix.as_deref(),
                Some("use a declared non-health stat of the ruleset")
            );
        }
        let diag = only(mutate_srd(|docs| {
            prepare_flag(docs);
            with_ability(docs, HEAVY_ID, |a| a.defense = make("flag".into()));
        }));
        assert_eq!(diag.pointer, "/defense");
        assert_eq!(
            diag.message,
            format!(
                "defense stat \"flag\" for ability is not an int stat of ruleset {}",
                id(RULESET_ID)
            )
        );
        assert_eq!(
            diag.suggested_fix.as_deref(),
            Some("use a declared non-health int stat of the ruleset")
        );
    }
}

#[test]
fn t8_opposed_bonus_stats_pointer_and_message() {
    for name in BAD_STATS {
        let diag = only(mutate_srd(|docs| {
            prepare_flag(docs);
            with_ability(docs, HEAVY_ID, |a| {
                a.defense = DefenseWire::Opposed {
                    dice: "1d20".into(),
                    bonus_stats: vec!["might".into(), name.into()],
                }
            });
        }));
        assert_eq!(diag.code, DiagnosticCode::UnknownStat);
        assert_eq!(diag.pointer, "/defense/value/bonus_stats/1");
        assert_eq!(
            diag.message,
            format!(
                "unknown opposed bonus stat {name:?} for ability: not a non-health int stat of ruleset {}",
                id(RULESET_ID)
            )
        );
    }
}

fn stat_target(name: &str) -> EffectTargetWire {
    EffectTargetWire::Stat { stat: name.into() }
}

#[test]
fn t8_effect_stat_targets_are_reported_in_the_effect_file() {
    for name in BAD_STATS {
        let mut effect_file = None;
        let diag = only(mutate_srd(|docs| {
            prepare_flag(docs);
            effect_file = Some(path_of(docs, |d| matches!(d, Document::Effect(_))));
            with_effect(docs, |e| e.modifiers[1].target = stat_target(name));
        }));
        assert_eq!(diag.file, effect_file);
        assert_eq!(diag.code, DiagnosticCode::UnknownStat);
        assert_eq!(diag.pointer, "/modifiers/1/target");
        assert_eq!(
            diag.message,
            format!(
                "unknown effect stat {name:?} for effect {}: not a non-health int stat of ruleset {}",
                id(EFFECT_ID),
                id(RULESET_ID)
            )
        );
    }
    // Reported once per (effect, modifier, ruleset), however many abilities use it.
    let findings = mutate_srd(|docs| {
        with_effect(docs, |e| e.modifiers[0].target = stat_target("nope"));
        with_ability(docs, HEAVY_ID, |a| a.effect = Some(id(EFFECT_ID)));
    });
    assert_eq!(findings.len(), 1, "{findings:?}");
    // A valid target is clean.
    assert!(mutate_srd(|docs| {
        with_effect(docs, |e| e.modifiers[0].target = stat_target("ward"))
    })
    .is_empty());
}

#[test]
fn t8_orphans_are_clean() {
    // An ability listed by no ruleset is not judged.
    let findings = mutate_srd(|docs| {
        with_ruleset(docs, |r| r.abilities.retain(|a| *a != id(HEAVY_ID)));
        with_ability(docs, HEAVY_ID, |a| {
            a.bonus_stats = Some(vec!["nope".into()]);
            a.defense = DefenseWire::ActorStat {
                stat: "nope".into(),
            };
        });
    });
    assert_eq!(findings, Vec::new());
    // An effect used by no ability is not judged.
    let findings = mutate_srd(|docs| {
        with_ability(docs, FOCUS_ID, |a| a.effect = None);
        with_effect(docs, |e| e.modifiers[0].target = stat_target("nope"));
    });
    assert_eq!(findings, Vec::new());
}

// ---- T9 (behaviour 10) ----------------------------------------------------

#[test]
fn t9_prefab_completeness_is_narrowed() {
    // Derived Int, Bool and Tags stats need no creature value.
    let findings = mutate_srd(|docs| {
        add_stat(docs, "flag", StatKindWire::Bool);
        add_stat(docs, "labels", StatKindWire::Tags);
        add_stat(docs, "roll", StatKindWire::Dice);
        with_ruleset(docs, |ruleset| {
            ruleset.stats.push(StatDecl {
                name: "bonus".into(),
                kind: StatKindWire::Int,
                derived: Some(ExprWire::Stat("might".into())),
            })
        });
    });
    assert_eq!(findings, Vec::new());
    // A plain Int stat still must be present.
    let findings = mutate_srd(|docs| add_stat(docs, "extra", StatKindWire::Int));
    assert!(!findings.is_empty());
    assert!(findings
        .iter()
        .all(|d| d.code == DiagnosticCode::MissingStat));
    // A fractional Fixed value is clean, a missing one is not.
    let findings = mutate_srd(|docs| {
        add_stat(docs, "speed", StatKindWire::Fixed);
        for doc in docs.values_mut() {
            if let Document::Creature(creature) = doc {
                creature
                    .stats
                    .insert("speed".into(), Fx16_16::from_raw(65_536 + 1));
            }
        }
    });
    assert_eq!(findings, Vec::new());
    let findings = mutate_srd(|docs| add_stat(docs, "speed", StatKindWire::Fixed));
    assert!(!findings.is_empty());
    assert!(findings
        .iter()
        .all(|d| d.code == DiagnosticCode::MissingStat));
    // A fractional Int value is still rejected.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Creature(creature) = doc {
                creature.stats.insert("might".into(), Fx16_16::from_raw(1));
            }
        }
    });
    assert!(!findings.is_empty());
    assert!(findings
        .iter()
        .all(|d| d.code == DiagnosticCode::InvalidStatValue));
}

// ---- T10 (behaviour 11) ---------------------------------------------------

#[test]
fn t10_real_time_warning() {
    let set_rt = |docs: &mut BTreeMap<SourcePath, Document>,
                  recovery: Option<u32>,
                  repeats: Option<bool>| {
        for doc in docs.values_mut() {
            if let Document::Ability(ability) = doc {
                ability.recovery_ticks = recovery;
                ability.repeats_in_real_time = repeats;
            }
        }
    };
    let findings = mutate_srd(|docs| set_rt(docs, Some(60), None));
    assert_eq!(findings.len(), 2, "{findings:?}");
    for finding in &findings {
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(
            finding.code,
            DiagnosticCode::RealTimeFieldsWithoutRoundTicks
        );
        assert_eq!(finding.pointer, "/recovery_ticks");
        assert_eq!(
            finding.suggested_fix.as_deref(),
            Some("add round_ticks to the ruleset or remove the real-time fields")
        );
    }
    let ids: Vec<String> = [HEAVY_ID, FOCUS_ID]
        .iter()
        .map(|n| {
            format!(
                "ability {} declares real-time fields but ruleset {} has no round_ticks",
                id(*n),
                id(RULESET_ID)
            )
        })
        .collect();
    let mut messages: Vec<String> = findings.iter().map(|d| d.message.clone()).collect();
    messages.sort();
    let mut expected = ids;
    expected.sort();
    assert_eq!(messages, expected);

    let findings = mutate_srd(|docs| set_rt(docs, None, Some(true)));
    assert_eq!(findings.len(), 2);
    assert!(findings
        .iter()
        .all(|d| d.pointer == "/repeats_in_real_time"));
    // Warnings never fail validation, and false counts as absent.
    assert!(findings.iter().all(|d| d.severity == Severity::Warning));
    assert_eq!(
        mutate_srd(|docs| set_rt(docs, None, Some(false))),
        Vec::new()
    );
    // With round_ticks the same fields are clean, and an unused round_ticks is clean.
    assert_eq!(
        mutate_srd(|docs| {
            set_rt(docs, Some(60), Some(true));
            with_ruleset(docs, |r| r.round_ticks = Some(120));
        }),
        Vec::new()
    );
    assert_eq!(
        mutate_srd(|docs| with_ruleset(docs, |r| r.round_ticks = Some(120))),
        Vec::new()
    );
    // Orphan abilities are skipped.
    assert_eq!(
        mutate_srd(|docs| {
            with_ruleset(docs, |r| r.abilities.retain(|a| *a != id(HEAVY_ID)));
            with_ability(docs, HEAVY_ID, |a| a.recovery_ticks = Some(60));
        }),
        Vec::new()
    );
    // The code spelling and Display are pinned.
    assert_eq!(
        DiagnosticCode::RealTimeFieldsWithoutRoundTicks.as_str(),
        "real_time_fields_without_round_ticks"
    );
    assert_eq!(
        DiagnosticCode::RealTimeFieldsWithoutRoundTicks.to_string(),
        "real_time_fields_without_round_ticks"
    );
}

// ---- T12 (behaviour 13) ---------------------------------------------------

#[test]
fn t12_schemas_gain_optional_properties_only() {
    let schemas = crpg_data::generated_schemas().unwrap();
    let shape = |file: &str| -> Value {
        serde_json::from_slice::<Value>(&schemas[file]).unwrap()["oneOf"][0].clone()
    };
    for (file, added) in [
        ("ruleset.schema.json", vec!["initiative"]),
        ("ability.schema.json", vec!["bonus_stats"]),
        ("effect.schema.json", vec!["grants"]),
    ] {
        let shape = shape(file);
        let properties = shape["properties"].as_object().unwrap();
        let required = shape["required"].as_array().unwrap();
        for name in added {
            assert!(properties.contains_key(name), "{file} lacks {name}");
            assert!(!required.contains(&json!(name)), "{file} requires {name}");
        }
    }
    let ruleset = String::from_utf8(schemas["ruleset.schema.json"].clone()).unwrap();
    for needle in [
        "\"derived\"",
        "\"enum_id\"",
        "\"fixed\"",
        "\"tags\"",
        "\"dice\"",
        "\"subtract\"",
        "\"multiply\"",
        "\"divide\"",
    ] {
        assert!(ruleset.contains(needle), "ruleset schema lacks {needle}");
    }
    let ability = String::from_utf8(schemas["ability.schema.json"].clone()).unwrap();
    for needle in ["\"constant\"", "\"actor_stat\"", "\"opposed\""] {
        assert!(ability.contains(needle), "ability schema lacks {needle}");
    }
    let effect = String::from_utf8(schemas["effect.schema.json"].clone()).unwrap();
    assert!(effect.contains("\"stat\""));
    assert_eq!(schemas.len(), 22);
}

// ---- T13 (behaviour 15) ---------------------------------------------------

#[test]
fn t13_mapping_rows_decode_to_the_documented_values() {
    // StatKindWire / StatDecl{name, kind, derived} / ExprWire / StatValueWire.
    let stats: Vec<StatDecl> = serde_json::from_value(json!([
        {"name": "level", "kind": "int"},
        {"name": "mood", "kind": {"enum": {"enum_id": "m", "variants": ["calm", "angry"]}},
         "derived": {"type": "literal", "value": {"type": "enum",
            "value": {"enum_id": "m", "variant": "calm"}}}},
        {"name": "grit", "kind": "int", "derived": {"type": "add", "value": {
            "left": {"type": "stat", "value": "level"},
            "right": {"type": "literal", "value": {"type": "int", "value": 2}}}}},
        {"name": "scale", "kind": "fixed", "derived": {"type": "literal",
            "value": {"type": "fixed", "value": 98304}}}
    ]))
    .unwrap();
    assert_eq!(stats[0].kind, StatKindWire::Int);
    assert_eq!(stats[0].derived, None);
    assert_eq!(
        stats[1].kind,
        StatKindWire::Enum {
            enum_id: "m".into(),
            variants: vec!["calm".into(), "angry".into()]
        }
    );
    assert_eq!(
        stats[1].derived,
        Some(ExprWire::Literal(StatValueWire::Enum {
            enum_id: "m".into(),
            variant: "calm".into()
        }))
    );
    assert_eq!(
        stats[2].derived,
        Some(ExprWire::Add {
            left: Box::new(ExprWire::Stat("level".into())),
            right: Box::new(ExprWire::Literal(StatValueWire::Int(2))),
        })
    );
    // Fixed is the raw i32: 98304 / 65536 = 1.5.
    assert_eq!(
        stats[3].derived,
        Some(ExprWire::Literal(StatValueWire::Fixed(Fx16_16::from_raw(
            98_304
        ))))
    );

    // Ability.bonus_stats, DefenseWire, DamageEntry{amount, dice}.
    let mut value = heavy();
    value["bonus_stats"] = json!(["might"]);
    value["defense"] =
        json!({"type": "opposed", "value": {"bonus_stats": ["guile"], "dice": "1d20"}});
    value["damage"][1]["dice"] = json!("1d8");
    let ability = ability_of(read_ok(&value));
    assert_eq!(ability.bonus_stats, Some(vec!["might".into()]));
    assert_eq!(
        ability.defense,
        DefenseWire::Opposed {
            dice: "1d20".into(),
            bonus_stats: vec!["guile".into()]
        }
    );
    assert_eq!(
        ability.damage[1],
        DamageEntry {
            outcome: OutcomeWire::Success,
            amount: 3,
            dice: Some("1d8".into())
        }
    );
    value["defense"] = json!({"type": "constant", "value": 12});
    assert_eq!(
        ability_of(read_ok(&value)).defense,
        DefenseWire::Constant(12)
    );
    value["defense"] = json!({"type": "actor_stat", "value": {"stat": "resolve"}});
    assert_eq!(
        ability_of(read_ok(&value)).defense,
        DefenseWire::ActorStat {
            stat: "resolve".into()
        }
    );

    // Ruleset.initiative, EffectTargetWire::Stat, Effect.grants.
    let ruleset = ruleset_of(read_ok(&with(
        &srd_ruleset(),
        "initiative",
        json!({"dice": "1d20", "bonus_stats": ["guile"]}),
    )));
    assert_eq!(
        ruleset.initiative,
        Some(InitiativeWire {
            dice: "1d20".into(),
            bonus_stats: vec!["guile".into()]
        })
    );
    let mut effect = focusing();
    effect["modifiers"][0]["target"] = json!({"type": "stat", "value": {"stat": "ward"}});
    effect["grants"] = json!(["marked"]);
    let effect = effect_of(read_ok(&effect));
    assert_eq!(effect.modifiers[0].target, stat_target("ward"));
    assert_eq!(effect.grants, Some(vec!["marked".into()]));

    // Creature.tags is the existing authored source of combatant tags.
    let mut hero = json_file("campaigns/fixtures/combat_srd/creatures/hero.json");
    hero["tags"] = json!(["hero", "lead"]);
    match read_ok(&hero) {
        Document::Creature(creature) => {
            assert_eq!(creature.tags, vec!["hero".to_string(), "lead".to_string()])
        }
        other => panic!("expected creature, got {other:?}"),
    }
}
