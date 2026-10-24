#![forbid(unsafe_code)]
//! T136b two-keys contract tests, written from `tasks/T136b.md` Behaviour 1–15
//! alone. Each test names the Behaviour items it covers. Items marked [bump]
//! (schema-version bumps and migration steps) are out of the T136b scope that
//! landed and are not tested here.
use crpg_core::{Fx16_16, Ulid};
use crpg_data::{
    DataError, Diagnostic, DiagnosticCode, Document, EffectTargetWire, ExprWire, LoadedCampaign,
    Severity, SourcePath, StatKindWire, StatValueWire,
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

// Base documents. All ULIDs are synthetic; dangling references are only a
// semantic concern, never a read-time one, so single-document reads need no
// matching targets.

fn stat(name: &str, kind: Value) -> Value {
    json!({"name": name, "kind": kind})
}

fn stat_derived(name: &str, kind: Value, derived: Value) -> Value {
    json!({"name": name, "kind": kind, "derived": derived})
}

/// A ruleset whose `stats` are appended after `might`/`health`.
fn ruleset(extra_stats: Vec<Value>) -> Value {
    let mut stats = vec![stat("might", json!("int")), stat("health", json!("int"))];
    stats.extend(extra_stats);
    json!({
        "schema": "crpg.ruleset/2",
        "id": id(100).to_string(),
        "slug": "r",
        "name": "fixture.r",
        "package": "test.ruleset",
        "version": "0.1.0",
        "stats": stats,
        "health_stat": "health",
        "attributes": ["might"],
        "pools": [{"id": id(105).to_string(), "max": 1, "refresh": {"type": "on_turn_start"}}],
        "abilities": [id(101).to_string()]
    })
}

fn ability() -> Value {
    json!({
        "schema": "crpg.ability/2",
        "id": id(101).to_string(),
        "slug": "a",
        "name": "fixture.a",
        "dice": "2d6",
        "attribute": "might",
        "outcome_table": id(102).to_string(),
        "damage": [{"outcome": {"type": "success"}, "amount": 2}],
        "cost": 1,
        "extra_costs": [],
        "ends_turn": true,
        "defense": {"type": "actor_attribute"},
        "requires_target": true,
        "allow_self_target": false
    })
}

fn effect() -> Value {
    json!({
        "schema": "crpg.effect/1",
        "id": id(103).to_string(),
        "slug": "e",
        "name": "fixture.e",
        "aim": {"type": "slf"},
        "mod_type": "focus",
        "policy": {"type": "stack_all"},
        "modifiers": [{
            "id": id(104).to_string(),
            "target": {"type": "roll"},
            "op": {"type": "add"},
            "value": 1,
            "priority": 0,
            "name": null
        }],
        "duration_rounds": 1
    })
}

// ---------------------------------------------------------------------------
// Item 1 — Decode `StatKindWire`.
// ---------------------------------------------------------------------------

/// Behaviour 1: the six wire forms decode, and each rejection is `Malformed`.
#[test]
fn item1_stat_kind_wire_decodes_and_rejects() {
    // The six accepted forms.
    let accepted: Vec<(Value, StatKindWire)> = vec![
        (json!("int"), StatKindWire::Int),
        (json!("fixed"), StatKindWire::Fixed),
        (json!("bool"), StatKindWire::Bool),
        (json!("tags"), StatKindWire::Tags),
        (json!("dice"), StatKindWire::Dice),
        (
            json!({"enum": {"enum_id": "color", "variants": ["red", "blue"]}}),
            StatKindWire::Enum {
                enum_id: "color".into(),
                variants: vec!["red".into(), "blue".into()],
            },
        ),
    ];
    for (wire, want) in accepted {
        let doc = read_ok(&ruleset(vec![stat("aux", wire)]));
        let Document::Ruleset(r) = &doc else {
            panic!("ruleset")
        };
        assert_eq!(r.stats[2].kind, want);
        assert_eq!(read(&json_value(&written(&doc))).unwrap(), doc);
    }

    // Every item-1 rejection is `Malformed`.
    for bad in [
        json!("enum"),
        json!("Int"),
        json!({"int": null}),
        json!({"enum": {"enum_id": "e", "variants": ["a"]}, "x": 0}),
        json!({"enum": {"enum_id": "e"}}), // missing variants
        json!({"enum": {"enum_id": "e", "variants": ["a"], "extra": 1}}), // extra key
        json!({"type": "int"}),
    ] {
        assert!(
            matches!(
                read(&ruleset(vec![stat("aux", bad.clone())])),
                Err(DataError::Malformed { .. })
            ),
            "kind accepted: {bad}"
        );
    }

    // A `"kind": "int"` source keeps its bytes (no string shape change).
    let doc = read_ok(&ruleset(vec![]));
    assert!(written(&doc).contains("\"kind\": \"int\""));
}

// ---------------------------------------------------------------------------
// Item 2 — Decode `derived`.
// ---------------------------------------------------------------------------

/// Behaviour 2: every `ExprWire` and `StatValueWire` arm decodes, `null`/absent
/// read as `None`, and unknown/missing/extra keys plus out-of-range integers
/// are `Malformed`.
#[test]
fn item2_derived_expr_and_value_wire() {
    let int = json!({"type": "int", "value": 5});
    let lit = |v: Value| json!({"type": "literal", "value": v});

    // All eight ExprWire arms.
    let arms: Vec<(Value, ExprWire)> = vec![
        (lit(int.clone()), ExprWire::Literal(StatValueWire::Int(5))),
        (
            json!({"type": "stat", "value": "might"}),
            ExprWire::Stat("might".into()),
        ),
        (
            json!({"type": "add", "value": {"left": lit(int.clone()), "right": lit(int.clone())}}),
            ExprWire::Add {
                left: Box::new(ExprWire::Literal(StatValueWire::Int(5))),
                right: Box::new(ExprWire::Literal(StatValueWire::Int(5))),
            },
        ),
        (
            json!({"type": "subtract", "value": {"left": lit(int.clone()), "right": lit(int.clone())}}),
            ExprWire::Subtract {
                left: Box::new(ExprWire::Literal(StatValueWire::Int(5))),
                right: Box::new(ExprWire::Literal(StatValueWire::Int(5))),
            },
        ),
        (
            json!({"type": "multiply", "value": {"left": lit(int.clone()), "right": lit(int.clone())}}),
            ExprWire::Multiply {
                left: Box::new(ExprWire::Literal(StatValueWire::Int(5))),
                right: Box::new(ExprWire::Literal(StatValueWire::Int(5))),
            },
        ),
        (
            json!({"type": "divide", "value": {"left": lit(int.clone()), "right": lit(int.clone())}}),
            ExprWire::Divide {
                left: Box::new(ExprWire::Literal(StatValueWire::Int(5))),
                right: Box::new(ExprWire::Literal(StatValueWire::Int(5))),
            },
        ),
        (
            json!({"type": "min", "value": {"left": lit(int.clone()), "right": lit(int.clone())}}),
            ExprWire::Min {
                left: Box::new(ExprWire::Literal(StatValueWire::Int(5))),
                right: Box::new(ExprWire::Literal(StatValueWire::Int(5))),
            },
        ),
        (
            json!({"type": "max", "value": {"left": lit(int.clone()), "right": lit(int.clone())}}),
            ExprWire::Max {
                left: Box::new(ExprWire::Literal(StatValueWire::Int(5))),
                right: Box::new(ExprWire::Literal(StatValueWire::Int(5))),
            },
        ),
    ];
    for (wire, want) in arms {
        let doc = read_ok(&ruleset(vec![stat_derived("aux", json!("int"), wire)]));
        let Document::Ruleset(r) = &doc else {
            panic!("ruleset")
        };
        assert_eq!(r.stats[2].derived, Some(want));
    }

    // The six StatValueWire arms inside a literal, each on a stat whose kind
    // matches the literal so the read-time kind check (item 5.10) does not fire.
    let value_arms: Vec<(Value, Value, StatValueWire)> = vec![
        (
            json!("int"),
            json!({"type": "int", "value": 5}),
            StatValueWire::Int(5),
        ),
        (
            json!("fixed"),
            json!({"type": "fixed", "value": 65536}),
            StatValueWire::Fixed(Fx16_16::from_int(1)),
        ),
        (
            json!("bool"),
            json!({"type": "bool", "value": true}),
            StatValueWire::Bool(true),
        ),
        (
            json!({"enum": {"enum_id": "color", "variants": ["red"]}}),
            json!({"type": "enum", "value": {"enum_id": "color", "variant": "red"}}),
            StatValueWire::Enum {
                enum_id: "color".into(),
                variant: "red".into(),
            },
        ),
        (
            json!("tags"),
            json!({"type": "tags", "value": ["a", "b"]}),
            StatValueWire::Tags(vec!["a".into(), "b".into()]),
        ),
        (
            json!("dice"),
            json!({"type": "dice", "value": "2d6"}),
            StatValueWire::Dice("2d6".into()),
        ),
    ];
    for (kind, wire, want) in value_arms {
        let doc = read_ok(&ruleset(vec![stat_derived("aux", kind, lit(wire))]));
        let Document::Ruleset(r) = &doc else {
            panic!("ruleset")
        };
        assert_eq!(r.stats[2].derived, Some(ExprWire::Literal(want)));
    }

    // Absent and explicit null read as None.
    assert_eq!(
        ruleset_of(&read_ok(&ruleset(vec![stat("aux", json!("int"))]))).stats[2].derived,
        None
    );
    assert_eq!(
        ruleset_of(&read_ok(&ruleset(vec![stat_derived(
            "aux",
            json!("int"),
            Value::Null
        )])))
        .stats[2]
            .derived,
        None
    );

    // Rejections: unknown type, missing value, extra keys at every level,
    // out-of-range integers.
    for bad in [
        json!({"type": "bogus", "value": 1}),
        json!({"type": "stat"}),
        json!({"type": "stat", "value": "might", "extra": 0}),
        json!({"type": "literal", "value": {"type": "int", "value": 5, "extra": 0}}),
        json!({"type": "add", "value": {"left": lit(int.clone())}}),
        json!({"type": "add", "value": {"left": lit(int.clone()), "right": lit(int.clone()), "extra": 0}}),
        lit(json!({"type": "int", "value": 2147483648u64})), // out of i32 range
        lit(json!({"type": "fixed", "value": 2147483648u64})),
    ] {
        assert!(
            matches!(
                read(&ruleset(vec![stat_derived(
                    "aux",
                    json!("int"),
                    bad.clone()
                )])),
                Err(DataError::Malformed { .. })
            ),
            "derived accepted: {bad}"
        );
    }

    // The i32 boundary is accepted: i32::MAX fits in both `int` and raw
    // `fixed`; one past it is the out-of-range case exercised above.
    assert!(read(&ruleset(vec![stat_derived(
        "aux",
        json!("int"),
        lit(json!({"type": "int", "value": 2147483647}))
    )]))
    .is_ok());
    assert!(read(&ruleset(vec![stat_derived(
        "aux",
        json!("fixed"),
        lit(json!({"type": "fixed", "value": 2147483647}))
    )]))
    .is_ok());
}

/// A nested `add` expression string, `depth` wraps deep, built as raw text so
/// `read_document` parses it directly and serde_json's recursion limit applies.
fn deep_expr_json(depth: usize) -> String {
    let mut expr = r#"{"type":"literal","value":{"type":"int","value":1}}"#.to_string();
    for _ in 0..depth {
        expr = format!(
            r#"{{"type":"add","value":{{"left":{},"right":{{"type":"literal","value":{{"type":"int","value":1}}}}}}}}"#,
            expr
        );
    }
    expr
}

/// A full ruleset document with `derived_json` spliced into one `int` stat.
fn ruleset_doc_with_derived(derived_json: &str) -> String {
    format!(
        r#"{{"schema":"crpg.ruleset/2","id":"{}","slug":"r","name":"fixture.r","package":"test.ruleset","version":"0.1.0","stats":[{{"name":"might","kind":"int"}},{{"name":"health","kind":"int"}},{{"name":"aux","kind":"int","derived":{}}}],"health_stat":"health","attributes":["might"],"pools":[{{"id":"{}","max":1,"refresh":{{"type":"on_turn_start"}}}}],"abilities":["{}"]}}"#,
        id(100),
        derived_json,
        id(105),
        id(101)
    )
}

/// Behaviour 2 (last sentence): a derived tree deeper than serde_json's
/// recursion limit fails at parse as `Malformed`, before item 5 runs.
#[test]
fn item2_deep_tree_fails_at_parse() {
    let bytes = ruleset_doc_with_derived(&deep_expr_json(256)).into_bytes();
    let msg = malformed_message(crpg_data::read_document(&bytes));
    assert_ne!(
        msg,
        "ruleset derived expression must be at most 4096 nodes and 32 deep"
    );
    assert!(
        msg.contains("recursion"),
        "expected a serde recursion failure, got: {msg}"
    );
}

fn ruleset_of(doc: &Document) -> crpg_data::Ruleset {
    match doc {
        Document::Ruleset(r) => r.clone(),
        other => panic!("expected ruleset, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Items 3 and 4 — Decode/encode the remaining optional shapes.
// ---------------------------------------------------------------------------

/// Behaviour 3: optional-missing fields, explicit `null`, the required
/// initiative/opposed inner fields, and the exact defense/target forms.
#[test]
fn item3_optional_and_defense_target_shapes() {
    // Initiative round-trips with an empty bonus list.
    let mut r = ruleset(vec![]);
    r["initiative"] = json!({"dice": "1d20", "bonus_stats": []});
    let doc = read_ok(&r);
    let Document::Ruleset(rs) = &doc else {
        panic!("ruleset")
    };
    let Some(init) = &rs.initiative else {
        panic!("initiative present")
    };
    assert_eq!(init.dice, "1d20");
    assert!(init.bonus_stats.is_empty());

    // InitiativeWire requires both `dice` and `bonus_stats` and rejects extras.
    for bad in [
        json!({"dice": "1d20"}),                                // missing bonus_stats
        json!({"bonus_stats": []}),                             // missing dice
        json!({"dice": "1d20", "bonus_stats": [], "extra": 0}), // extra key
    ] {
        let mut r = ruleset(vec![]);
        r["initiative"] = bad;
        assert!(
            matches!(read(&r), Err(DataError::Malformed { .. })),
            "initiative accepted: {r:?}"
        );
    }

    // Explicit null reads as absent for initiative, bonus_stats, dice, grants.
    let mut r = ruleset(vec![]);
    r["initiative"] = Value::Null;
    assert_eq!(ruleset_of(&read_ok(&r)).initiative, None);

    let mut a = ability();
    a["bonus_stats"] = Value::Null;
    assert_eq!(ability_of(&read_ok(&a)).bonus_stats, None);
    let mut a = ability();
    a["damage"][0]["dice"] = Value::Null;
    assert_eq!(ability_of(&read_ok(&a)).damage[0].dice, None);

    let mut e = effect();
    e["grants"] = Value::Null;
    assert_eq!(effect_of(&read_ok(&e)).grants, None);

    // The exact defense/target wire forms.
    let defense_cases = [
        (json!({"type": "constant", "value": 12}), true),
        (
            json!({"type": "actor_stat", "value": {"stat": "guile"}}),
            true,
        ),
        (
            json!({"type": "opposed", "value": {"bonus_stats": ["might"], "dice": "2d6"}}),
            true,
        ),
        (
            // The opposed list may be empty (item 3: "the list may be empty").
            json!({"type": "opposed", "value": {"bonus_stats": [], "dice": "2d6"}}),
            true,
        ),
        // Rejections: extra/missing keys.
        (json!({"type": "constant", "value": 12, "extra": 0}), false),
        (json!({"type": "constant"}), false),
        (
            json!({"type": "actor_stat", "value": {"stat": "guile", "extra": 0}}),
            false,
        ),
        (json!({"type": "actor_stat", "value": {}}), false),
        (
            json!({"type": "opposed", "value": {"bonus_stats": ["might"]}}),
            false,
        ),
        (json!({"type": "opposed", "value": {"dice": "2d6"}}), false),
        (
            json!({"type": "opposed", "value": {"bonus_stats": ["might"], "dice": "2d6", "extra": 0}}),
            false,
        ),
    ];
    for (defense, ok) in defense_cases {
        let mut a = ability();
        a["defense"] = defense;
        let result = read(&a);
        assert_eq!(result.is_ok(), ok, "defense {ok}: {result:?}");
    }

    let target_cases = [
        (json!({"type": "stat", "value": {"stat": "might"}}), true),
        (
            json!({"type": "stat", "value": {"stat": "might", "extra": 0}}),
            false,
        ),
        (json!({"type": "stat", "value": {}}), false),
        (json!({"type": "stat"}), false),
    ];
    for (target, ok) in target_cases {
        let mut e = effect();
        e["modifiers"][0]["target"] = target;
        let result = read(&e);
        assert_eq!(result.is_ok(), ok, "target {ok}: {result:?}");
    }
}

fn ability_of(doc: &Document) -> crpg_data::Ability {
    match doc {
        Document::Ability(a) => a.clone(),
        other => panic!("expected ability, got {other:?}"),
    }
}

fn effect_of(doc: &Document) -> crpg_data::Effect {
    match doc {
        Document::Effect(e) => e.clone(),
        other => panic!("expected effect, got {other:?}"),
    }
}

/// Behaviour 4: an absent optional writes no key; authored array order is kept.
#[test]
fn item4_absent_optional_omits_key_and_preserves_order() {
    // Absent optional writes no key.
    let doc = read_ok(&ruleset(vec![]));
    let out = written(&doc);
    assert!(!out.contains("initiative"));
    let doc = read_ok(&ability());
    assert!(!written(&doc).contains("bonus_stats"));
    let doc = read_ok(&effect());
    assert!(!written(&doc).contains("grants"));

    // Absent `derived` (per stat) and damage `dice` also write no key.
    let doc = read_ok(&ruleset(vec![stat("aux", json!("int"))]));
    assert!(!written(&doc).contains("derived"));
    let doc = read_ok(&ability());
    let out = json_value(&written(&doc));
    assert!(out["damage"][0].get("dice").is_none());

    // Enum variants, bonus_stats and grants keep authored order through the
    // canonical writer (it sorts keys, not array elements).
    let doc = read_ok(&ruleset(vec![stat(
        "aux",
        json!({"enum": {"enum_id": "c", "variants": ["z", "a"]}}),
    )]));
    let reread = read_ok(&json_value(&written(&doc)));
    let Document::Ruleset(r) = &reread else {
        panic!("ruleset")
    };
    assert_eq!(
        r.stats[2].kind,
        StatKindWire::Enum {
            enum_id: "c".into(),
            variants: vec!["z".into(), "a".into()]
        }
    );

    let mut a = ability();
    a["bonus_stats"] = json!(["z", "a"]);
    let reread = read_ok(&json_value(&written(&read_ok(&a))));
    assert_eq!(
        ability_of(&reread).bonus_stats,
        Some(vec!["z".to_string(), "a".to_string()])
    );

    let mut e = effect();
    e["grants"] = json!(["z", "a"]);
    let reread = read_ok(&json_value(&written(&read_ok(&e))));
    assert_eq!(
        effect_of(&reread).grants,
        Some(vec!["z".to_string(), "a".to_string()])
    );
}

// ---------------------------------------------------------------------------
// Item 5 — Read-time ruleset checks.
// ---------------------------------------------------------------------------

fn enum_stat(enum_id: &str, variants: Value) -> Value {
    stat(
        "color",
        json!({"enum": {"enum_id": enum_id, "variants": variants}}),
    )
}

/// Behaviour 5.1–5.4: enum shape, shared-domain consistency, health and
/// attribute kinds.
#[test]
fn item5_enum_and_health_attribute_checks() {
    // enum_id nonempty.
    assert_eq!(
        malformed_message(read(&ruleset(vec![enum_stat("", json!(["a"]))]))),
        "ruleset enum stat enum_id must be nonempty"
    );
    // variants 1..=1024 (empty and over-large both fall here).
    let too_many: Vec<String> = (0..1025).map(|n| format!("v{n}")).collect();
    assert_eq!(
        malformed_message(read(&ruleset(vec![enum_stat("c", json!(too_many))]))),
        "ruleset enum stat variants must be 1..=1024"
    );
    assert_eq!(
        malformed_message(read(&ruleset(vec![enum_stat("c", json!([]))]))),
        "ruleset enum stat variants must be 1..=1024"
    );
    // distinct and nonempty (with a valid count).
    assert_eq!(
        malformed_message(read(&ruleset(vec![enum_stat("c", json!(["a", ""]))]))),
        "ruleset enum stat variants must be distinct and nonempty"
    );
    assert_eq!(
        malformed_message(read(&ruleset(vec![enum_stat("c", json!(["a", "a"]))]))),
        "ruleset enum stat variants must be distinct and nonempty"
    );
    // Sharing an enum_id must declare the same variants (compared as sets).
    let two = vec![
        stat(
            "c1",
            json!({"enum": {"enum_id": "c", "variants": ["a", "b"]}}),
        ),
        stat("c2", json!({"enum": {"enum_id": "c", "variants": ["a"]}})),
    ];
    assert_eq!(
        malformed_message(read(&ruleset(two))),
        "ruleset enum stats sharing an enum_id must declare the same variants"
    );

    // health_stat must name a non-derived int stat.
    let mut r = ruleset(vec![stat("flag", json!("bool"))]);
    r["health_stat"] = json!("flag");
    assert_eq!(
        malformed_message(read(&r)),
        "ruleset health_stat must name a non-derived int stat"
    );

    // attributes must name int stats (a non-Int attribute fails).
    let mut r = ruleset(vec![stat("flag", json!("bool"))]);
    r["attributes"] = json!(["flag"]);
    assert_eq!(
        malformed_message(read(&r)),
        "ruleset attributes must name int stats"
    );

    // A derived Int stat is not a valid health_stat (must be non-derived).
    let mut r = ruleset(vec![stat_derived("d", json!("int"), int_lit())]);
    r["health_stat"] = json!("d");
    assert_eq!(
        malformed_message(read(&r)),
        "ruleset health_stat must name a non-derived int stat"
    );

    // A derived Int stat is a valid attribute (derived allowed).
    let mut r = ruleset(vec![stat_derived("d", json!("int"), int_lit())]);
    r["attributes"] = json!(["d"]);
    assert!(read(&r).is_ok());
}

fn int_lit() -> Value {
    json!({"type": "literal", "value": {"type": "int", "value": 1}})
}

fn add_node(left: Value, right: Value) -> Value {
    json!({"type": "add", "value": {"left": left, "right": right}})
}

/// A left-deep spine of `levels` `Add` nodes: expression depth = levels + 1.
fn left_chain(levels: usize) -> Value {
    let mut expr = int_lit();
    for _ in 0..levels {
        expr = add_node(expr, int_lit());
    }
    expr
}

/// A perfect binary tree with `levels` of `Add` nodes: node count
/// 2^(levels+1) - 1, depth levels + 1.
fn full_tree(levels: usize) -> Value {
    if levels == 0 {
        return int_lit();
    }
    let sub = full_tree(levels - 1);
    add_node(sub.clone(), sub)
}

fn derived_stats(names: Vec<(String, Value)>) -> Value {
    ruleset(
        names
            .into_iter()
            .map(|(name, derived)| stat_derived(&name, json!("int"), derived))
            .collect(),
    )
}

/// Behaviour 5.5–5.10: derived expression bounds, references, cycles, chains
/// and kind matching.
#[test]
fn item5_derived_bounds_references_and_kind() {
    // 5.5 — depth 32 ok, 33 fails. Node count: 4095 (full_tree(11)) is the
    // largest odd count reachable at or below 4096 and is accepted; 4097 is
    // one past and is rejected. 4096 itself is unreachable because every
    // expression tree is a full binary tree, whose node count is always odd.
    assert!(read(&derived_stats(vec![("d".to_string(), left_chain(31))])).is_ok());
    assert_eq!(
        malformed_message(read(&derived_stats(vec![(
            "d".to_string(),
            left_chain(32)
        )]))),
        "ruleset derived expression must be at most 4096 nodes and 32 deep"
    );
    assert!(read(&derived_stats(vec![("d".to_string(), full_tree(11))])).is_ok());
    assert_eq!(
        malformed_message(read(&derived_stats(vec![(
            "d".to_string(),
            add_node(full_tree(11), int_lit()) // 4095 + 2 = 4097 nodes, depth 13
        )]))),
        "ruleset derived expression must be at most 4096 nodes and 32 deep"
    );

    // 5.6 — total node budget. 16 x 4095 = 65520 is the largest reachable
    // total at or below 65536 (65536 itself is unreachable: a sum of odd
    // per-stat counts is odd for an odd number of stats and at most 65520
    // for an even number). 65537 (16 x 4095 + 17) is one past and reachable.
    let sixteen: Vec<(String, Value)> = (0..16).map(|n| (format!("d{n}"), full_tree(11))).collect();
    assert!(read(&derived_stats(sixteen)).is_ok());
    let mut seventeen: Vec<(String, Value)> =
        (0..16).map(|n| (format!("d{n}"), full_tree(11))).collect();
    seventeen.push(("d16".to_string(), left_chain(8))); // 17 nodes
    assert_eq!(
        malformed_message(read(&derived_stats(seventeen))),
        "ruleset derived expressions must total at most 65536 nodes"
    );

    // 5.7 — preorder node checks.
    let tags_stat = |tags: Value| {
        stat_derived(
            "aux",
            json!("tags"),
            json!({"type": "literal", "value": {"type": "tags", "value": tags}}),
        )
    };
    let many_tags: Vec<String> = (0..1025).map(|n| format!("t{n}")).collect();
    assert_eq!(
        malformed_message(read(&ruleset(vec![tags_stat(json!(many_tags))]))),
        "ruleset derived tags literal must hold at most 1024 distinct nonempty tags"
    );
    assert!(read(&ruleset(vec![tags_stat(json!((0..1024)
        .map(|n| format!("t{n}"))
        .collect::<Vec<_>>()))]))
    .is_ok());

    // Duplicate and empty tag entries fail the same distinct/nonempty rule.
    for bad in [json!(["a", "a"]), json!(["a", ""])] {
        assert_eq!(
            malformed_message(read(&ruleset(vec![tags_stat(bad)]))),
            "ruleset derived tags literal must hold at most 1024 distinct nonempty tags"
        );
    }

    let dice_stat = |dice: &str| {
        stat_derived(
            "aux",
            json!("dice"),
            json!({"type": "literal", "value": {"type": "dice", "value": dice}}),
        )
    };
    assert_eq!(
        malformed_message(read(&ruleset(vec![dice_stat(&"x".repeat(129))]))),
        "ruleset derived dice literal must be 1..=128 UTF-8 bytes"
    );
    assert!(read(&ruleset(vec![dice_stat(&"x".repeat(128))])).is_ok());

    let enum_lit = stat_derived(
        "aux",
        json!({"enum": {"enum_id": "color", "variants": ["red"]}}),
        json!({"type": "literal", "value": {"type": "enum", "value": {"enum_id": "foreign", "variant": "red"}}}),
    );
    assert_eq!(
        malformed_message(read(&ruleset(vec![enum_lit]))),
        "ruleset derived enum literal must name a declared enum variant"
    );

    // An in-domain enum_id with an undeclared variant fails the same rule.
    let undeclared_variant = stat_derived(
        "aux",
        json!({"enum": {"enum_id": "color", "variants": ["red"]}}),
        json!({"type": "literal", "value": {"type": "enum", "value": {"enum_id": "color", "variant": "blue"}}}),
    );
    assert_eq!(
        malformed_message(read(&ruleset(vec![undeclared_variant]))),
        "ruleset derived enum literal must name a declared enum variant"
    );

    let undeclared = stat_derived(
        "aux",
        json!("int"),
        json!({"type": "stat", "value": "nope"}),
    );
    assert_eq!(
        malformed_message(read(&ruleset(vec![undeclared]))),
        "ruleset derived expression must reference declared stats"
    );

    // 5.8 — cycle (self and mutual).
    let self_cycle = stat_derived("aux", json!("int"), json!({"type": "stat", "value": "aux"}));
    assert_eq!(
        malformed_message(read(&ruleset(vec![self_cycle]))),
        "ruleset derived stats must not form a cycle"
    );
    let mutual = vec![
        stat_derived("a", json!("int"), json!({"type": "stat", "value": "b"})),
        stat_derived("b", json!("int"), json!({"type": "stat", "value": "a"})),
    ];
    assert_eq!(
        malformed_message(read(&ruleset(mutual))),
        "ruleset derived stats must not form a cycle"
    );

    // 5.9 — dependency chains at most 128 (endpoints included).
    let chain = |len: usize| {
        let stats: Vec<Value> = (0..len)
            .map(|i| {
                let name = format!("s{i}");
                let derived = if i + 1 < len {
                    json!({"type": "stat", "value": format!("s{}", i + 1)})
                } else {
                    int_lit()
                };
                stat_derived(&name, json!("int"), derived)
            })
            .collect();
        ruleset(stats)
    };
    assert!(read(&chain(128)).is_ok());
    assert_eq!(
        malformed_message(read(&chain(129))),
        "ruleset derived dependency chains must be at most 128 stats"
    );

    // 5.10 — kind matching.
    let bool_root = stat_derived(
        "aux",
        json!("int"),
        json!({"type": "literal", "value": {"type": "bool", "value": true}}),
    );
    assert_eq!(
        malformed_message(read(&ruleset(vec![bool_root]))),
        "ruleset derived expression kind must match its stat"
    );
    let mixed = stat_derived(
        "aux",
        json!("int"),
        add_node(
            int_lit(),
            json!({"type": "literal", "value": {"type": "fixed", "value": 65536}}),
        ),
    );
    assert_eq!(
        malformed_message(read(&ruleset(vec![mixed]))),
        "ruleset derived expression kind must match its stat"
    );

    // An Enum root whose enum_id differs from the stat's own domain.
    let enum_mismatch = ruleset(vec![
        stat(
            "shape",
            json!({"enum": {"enum_id": "shape", "variants": ["square"]}}),
        ),
        stat_derived(
            "aux",
            json!({"enum": {"enum_id": "color", "variants": ["red"]}}),
            json!({"type": "literal", "value": {"type": "enum", "value": {"enum_id": "shape", "variant": "square"}}}),
        ),
    ]);
    assert_eq!(
        malformed_message(read(&enum_mismatch)),
        "ruleset derived expression kind must match its stat"
    );

    // A `stat` node naming a declared Bool stat mismatches an Int root.
    let bool_stat_ref = ruleset(vec![
        stat("flag", json!("bool")),
        stat_derived(
            "aux",
            json!("int"),
            json!({"type": "stat", "value": "flag"}),
        ),
    ]);
    assert_eq!(
        malformed_message(read(&bool_stat_ref)),
        "ruleset derived expression kind must match its stat"
    );

    // A binary operator over non-numeric operands cannot type.
    let nonnumeric = stat_derived(
        "aux",
        json!("int"),
        add_node(
            json!({"type": "literal", "value": {"type": "bool", "value": true}}),
            json!({"type": "literal", "value": {"type": "bool", "value": false}}),
        ),
    );
    assert_eq!(
        malformed_message(read(&ruleset(vec![nonnumeric]))),
        "ruleset derived expression kind must match its stat"
    );
}

/// Behaviour 5.11: initiative dice and bonus stats.
#[test]
fn item5_initiative_checks() {
    let mut r = ruleset(vec![]);
    r["initiative"] = json!({"dice": "x".repeat(129), "bonus_stats": ["might"]});
    assert_eq!(
        malformed_message(read(&r)),
        "ruleset initiative dice must be 1..=128 UTF-8 bytes"
    );
    // An empty dice string is zero bytes and fails the same bound.
    let mut r = ruleset(vec![]);
    r["initiative"] = json!({"dice": "", "bonus_stats": ["might"]});
    assert_eq!(
        malformed_message(read(&r)),
        "ruleset initiative dice must be 1..=128 UTF-8 bytes"
    );

    for bad in [
        json!(["health"]),         // non-health required
        json!(["bogus"]),          // declared required
        json!(["might", "might"]), // distinct required
    ] {
        let mut r = ruleset(vec![]);
        r["initiative"] = json!({"dice": "1d20", "bonus_stats": bad});
        assert_eq!(
            malformed_message(read(&r)),
            "ruleset initiative bonus stats must name distinct non-health int stats"
        );
    }

    // A declared non-Int stat is rejected (only int stats qualify).
    let mut r = ruleset(vec![stat("flag", json!("bool"))]);
    r["initiative"] = json!({"dice": "1d20", "bonus_stats": ["flag"]});
    assert_eq!(
        malformed_message(read(&r)),
        "ruleset initiative bonus stats must name distinct non-health int stats"
    );

    // A derived Int stat is still an int stat, so it is accepted.
    let mut r = ruleset(vec![stat_derived("d", json!("int"), int_lit())]);
    r["initiative"] = json!({"dice": "1d20", "bonus_stats": ["d"]});
    assert!(read(&r).is_ok());
}

// ---------------------------------------------------------------------------
// Item 6 — Read-time ability checks.
// ---------------------------------------------------------------------------

/// Behaviour 6: widened defense check, bonus_stats, opposed and damage dice.
#[test]
fn item6_ability_read_time_checks() {
    // ActorStat with an empty name joins the existing defense check.
    let mut a = ability();
    a["defense"] = json!({"type": "actor_stat", "value": {"stat": ""}});
    assert_eq!(
        malformed_message(read(&a)),
        "ability defense stat must be nonempty"
    );

    // bonus_stats must be a nonempty list of distinct nonempty names.
    for bad in [json!([]), json!([""]), json!(["a", "a"])] {
        let mut a = ability();
        a["bonus_stats"] = bad;
        assert_eq!(
            malformed_message(read(&a)),
            "ability bonus_stats must be a nonempty list of distinct nonempty names"
        );
    }

    // Opposed dice length, then opposed bonus_stats distinctness.
    let mut a = ability();
    a["defense"] =
        json!({"type": "opposed", "value": {"dice": "x".repeat(129), "bonus_stats": ["might"]}});
    assert_eq!(
        malformed_message(read(&a)),
        "ability opposed dice must be 1..=128 UTF-8 bytes"
    );
    // An empty opposed dice string fails the same bound; 128 bytes pass.
    let mut a = ability();
    a["defense"] = json!({"type": "opposed", "value": {"dice": "", "bonus_stats": ["might"]}});
    assert_eq!(
        malformed_message(read(&a)),
        "ability opposed dice must be 1..=128 UTF-8 bytes"
    );
    let mut a = ability();
    a["defense"] =
        json!({"type": "opposed", "value": {"dice": "x".repeat(128), "bonus_stats": ["might"]}});
    assert!(read(&a).is_ok());
    let mut a = ability();
    a["defense"] = json!({"type": "opposed", "value": {"dice": "2d6", "bonus_stats": ["a", "a"]}});
    assert_eq!(
        malformed_message(read(&a)),
        "ability opposed bonus_stats must be distinct nonempty names"
    );

    // Damage dice length.
    let mut a = ability();
    a["damage"][0]["dice"] = json!("x".repeat(129));
    assert_eq!(
        malformed_message(read(&a)),
        "ability damage dice must be 1..=128 UTF-8 bytes"
    );
    let mut a = ability();
    a["damage"][0]["dice"] = json!("x".repeat(128));
    assert!(read(&a).is_ok());
}

// ---------------------------------------------------------------------------
// Item 7 — Read-time effect checks.
// ---------------------------------------------------------------------------

/// Behaviour 7: modifier `Stat` target name and grants bounds.
#[test]
fn item7_effect_read_time_checks() {
    let mut e = effect();
    e["modifiers"][0]["target"] = json!({"type": "stat", "value": {"stat": ""}});
    assert_eq!(
        malformed_message(read(&e)),
        "effect modifier stat must be nonempty"
    );

    let mut e = effect();
    let many: Vec<String> = (0..1025).map(|n| format!("t{n}")).collect();
    e["grants"] = json!(many);
    assert_eq!(
        malformed_message(read(&e)),
        "effect grants must hold 1..=1024 distinct nonempty tags"
    );
    // Duplicate and empty tags fail the same rule.
    let mut e = effect();
    e["grants"] = json!(["a", "a"]);
    assert_eq!(
        malformed_message(read(&e)),
        "effect grants must hold 1..=1024 distinct nonempty tags"
    );
    let mut e = effect();
    e["grants"] = json!([""]);
    assert_eq!(
        malformed_message(read(&e)),
        "effect grants must hold 1..=1024 distinct nonempty tags"
    );
}

// ---------------------------------------------------------------------------
// Item 8 — Order: the earlier fault wins.
// ---------------------------------------------------------------------------

/// Behaviour 8: a new check never masks an earlier existing check.
#[test]
fn item8_earlier_fault_wins() {
    // Empty attribute beats bonus_stats [""].
    let mut a = ability();
    a["attribute"] = json!("");
    a["bonus_stats"] = json!([""]);
    assert_eq!(
        malformed_message(read(&a)),
        "ability attribute must be nonempty"
    );

    // Duplicate stat name beats a bad derived.
    let mut r = ruleset(vec![stat("aux", json!("int"))]);
    let dup = r["stats"][0].clone();
    r["stats"].as_array_mut().unwrap().push(dup);
    r["stats"].as_array_mut().unwrap().push(stat_derived(
        "d",
        json!("int"),
        json!({"type": "stat", "value": "nope"}),
    ));
    assert_eq!(malformed_message(read(&r)), "duplicate ruleset stat name");

    // Within the derived checks, the earlier check wins: a too-deep tree that
    // also references an undeclared stat reports the depth bound (5.5) before
    // the reference check (5.7).
    let mut deep_bad = json!({"type": "stat", "value": "nope"});
    for _ in 0..32 {
        deep_bad = add_node(deep_bad, int_lit());
    }
    assert_eq!(
        malformed_message(read(&ruleset(vec![stat_derived(
            "d",
            json!("int"),
            deep_bad
        )]))),
        "ruleset derived expression must be at most 4096 nodes and 32 deep"
    );

    // Empty mod_type beats an empty stat target.
    let mut e = effect();
    e["mod_type"] = json!("");
    e["modifiers"][0]["target"] = json!({"type": "stat", "value": {"stat": ""}});
    assert_eq!(
        malformed_message(read(&e)),
        "effect mod_type must be nonempty"
    );
}

// ---------------------------------------------------------------------------
// Items 9–11 — Semantic checks over a loaded campaign.
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

fn mutate_srd(change: impl FnOnce(&mut BTreeMap<SourcePath, Document>)) -> Vec<Diagnostic> {
    let mut docs = load_combat_srd();
    change(&mut docs);
    crpg_data::validate(&LoadedCampaign {
        documents: docs,
        index: BTreeMap::new(),
    })
}

/// Behaviour 9: the four `unknown_stat` walk_combat cases, with pointers and
/// messages, plus an orphan staying clean.
#[test]
fn item9_semantic_unknown_stat() {
    // (a) unknown bonus stat: undeclared, declared-health and declared
    // non-Int stats are all rejected with the same message.
    for (name, declare_bool) in [("bogus", false), ("health", false), ("flag", true)] {
        let findings = mutate_srd(|docs| {
            if declare_bool {
                for doc in docs.values_mut() {
                    if let Document::Ruleset(r) = doc {
                        r.stats.push(crpg_data::StatDecl {
                            name: "flag".into(),
                            kind: StatKindWire::Bool,
                            derived: None,
                        });
                    }
                }
            }
            for doc in docs.values_mut() {
                if let Document::Ability(a) = doc {
                    if a.id == id(39) {
                        a.bonus_stats = Some(vec![name.to_string()]);
                    }
                }
            }
        });
        assert_eq!(findings.len(), 1, "{name}: {findings:?}");
        assert_eq!(findings[0].code, DiagnosticCode::UnknownStat);
        assert_eq!(findings[0].pointer, "/bonus_stats/0");
        assert_eq!(
            findings[0].message,
            format!(
                "unknown bonus stat {name:?} for ability: not a non-health int stat of ruleset {}",
                id(38)
            )
        );
    }

    // (b) TargetStat naming an undeclared or health stat keeps the existing
    // message; a declared non-Int stat gives the new message.
    for stat in ["bogus", "health"] {
        let findings = mutate_srd(|docs| {
            for doc in docs.values_mut() {
                if let Document::Ability(a) = doc {
                    if a.id == id(39) {
                        a.defense = crpg_data::DefenseWire::TargetStat { stat: stat.into() };
                    }
                }
            }
        });
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, DiagnosticCode::UnknownStat);
        assert_eq!(findings[0].pointer, "/defense");
        assert!(
            findings[0].message.contains(&format!("{stat:?}")),
            "{findings:?}"
        );
        assert!(findings[0].message.contains(&id(38).to_string()));
    }
    // Declared non-health but not Int.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ruleset(r) = doc {
                r.stats.push(crpg_data::StatDecl {
                    name: "flag".into(),
                    kind: StatKindWire::Bool,
                    derived: None,
                });
            }
            if let Document::Ability(a) = doc {
                if a.id == id(39) {
                    a.defense = crpg_data::DefenseWire::ActorStat {
                        stat: "flag".into(),
                    };
                }
            }
        }
    });
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, DiagnosticCode::UnknownStat);
    assert_eq!(findings[0].pointer, "/defense");
    assert_eq!(
        findings[0].message,
        format!(
            "defense stat \"flag\" for ability is not an int stat of ruleset {}",
            id(38)
        )
    );

    // (c) unknown opposed bonus stat, for undeclared, health and Bool stats.
    for (name, declare_bool) in [("bogus", false), ("health", false), ("flag", true)] {
        let findings = mutate_srd(|docs| {
            if declare_bool {
                for doc in docs.values_mut() {
                    if let Document::Ruleset(r) = doc {
                        r.stats.push(crpg_data::StatDecl {
                            name: "flag".into(),
                            kind: StatKindWire::Bool,
                            derived: None,
                        });
                    }
                }
            }
            for doc in docs.values_mut() {
                if let Document::Ability(a) = doc {
                    if a.id == id(39) {
                        a.defense = crpg_data::DefenseWire::Opposed {
                            dice: "2d6".into(),
                            bonus_stats: vec![name.to_string()],
                        };
                    }
                }
            }
        });
        assert_eq!(findings.len(), 1, "{name}: {findings:?}");
        assert_eq!(findings[0].code, DiagnosticCode::UnknownStat);
        assert_eq!(findings[0].pointer, "/defense/value/bonus_stats/0");
        assert_eq!(
            findings[0].message,
            format!(
                "unknown opposed bonus stat {name:?} for ability: not a non-health int stat of ruleset {}",
                id(38)
            )
        );
    }

    // (d) unknown effect stat, in the effect's file, for undeclared, health
    // and Bool stats.
    for (name, declare_bool) in [("bogus", false), ("health", false), ("flag", true)] {
        let findings = mutate_srd(|docs| {
            if declare_bool {
                for doc in docs.values_mut() {
                    if let Document::Ruleset(r) = doc {
                        r.stats.push(crpg_data::StatDecl {
                            name: "flag".into(),
                            kind: StatKindWire::Bool,
                            derived: None,
                        });
                    }
                }
            }
            for doc in docs.values_mut() {
                if let Document::Effect(e) = doc {
                    if e.id == id(43) {
                        e.modifiers[0].target = EffectTargetWire::Stat {
                            stat: name.to_string(),
                        };
                    }
                }
            }
        });
        assert_eq!(findings.len(), 1, "{name}: {findings:?}");
        assert_eq!(findings[0].code, DiagnosticCode::UnknownStat);
        assert_eq!(
            findings[0].file.as_ref().map(SourcePath::as_str),
            Some("effects/focusing.json")
        );
        assert_eq!(findings[0].pointer, "/modifiers/0/target");
        assert_eq!(
            findings[0].message,
            format!(
                "unknown effect stat {name:?} for effect {}: not a non-health int stat of ruleset {}",
                id(43),
                id(38)
            )
        );
    }

    // The once-per-(effect, modifier, ruleset) rule: two abilities in the same
    // ruleset referencing the same effect report its bad modifier once.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ability(a) = doc {
                if a.id == id(39) {
                    a.effect = Some(id(43));
                }
            }
        }
        for doc in docs.values_mut() {
            if let Document::Effect(e) = doc {
                if e.id == id(43) {
                    e.modifiers[0].target = EffectTargetWire::Stat {
                        stat: "bogus".into(),
                    };
                }
            }
        }
    });
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].code, DiagnosticCode::UnknownStat);
    assert_eq!(findings[0].pointer, "/modifiers/0/target");

    // An orphan ability/effect is clean of these checks.
    let findings = mutate_srd(|docs| {
        docs.insert(
            "abilities/orphan.json".parse().unwrap(),
            Document::Ability(crpg_data::Ability {
                id: id(200),
                slug: "orphan".into(),
                name: "fixture.orphan".into(),
                note: None,
                dice: "1d6".into(),
                attribute: "bogus".into(),
                outcome_table: id(201),
                damage: vec![crpg_data::DamageEntry {
                    outcome: crpg_data::OutcomeWire::Success,
                    amount: 0,
                    dice: None,
                }],
                cost: 1,
                extra_costs: Vec::new(),
                ends_turn: true,
                effect: None,
                defense: crpg_data::DefenseWire::ActorAttribute,
                natural_die: None,
                requires_target: false,
                allow_self_target: false,
                recovery_ticks: None,
                repeats_in_real_time: None,
                bonus_stats: Some(vec!["bogus".into()]),
            }),
        );
    });
    assert!(!findings
        .iter()
        .any(|d| d.code == DiagnosticCode::UnknownStat));

    // An orphan effect (no ability references it) is clean of these checks.
    let findings = mutate_srd(|docs| {
        docs.insert(
            "effects/orphan.json".parse().unwrap(),
            Document::Effect(crpg_data::Effect {
                id: id(202),
                slug: "orphan-effect".into(),
                name: "fixture.orphan-effect".into(),
                note: None,
                aim: crpg_data::EffectAimWire::Slf,
                mod_type: "focus".into(),
                policy: crpg_data::PolicyWire::StackAll,
                modifiers: vec![crpg_data::EffectModifierWire {
                    id: id(203),
                    target: EffectTargetWire::Stat {
                        stat: "bogus".into(),
                    },
                    op: crpg_data::EffectOpWire::Add,
                    value: 1,
                    priority: 0,
                    name: None,
                }],
                duration_rounds: 1,
                grants: None,
            }),
        );
    });
    assert!(
        !findings
            .iter()
            .any(|d| d.code == DiagnosticCode::UnknownStat),
        "{findings:?}"
    );
}

/// Behaviour 10: prefab completeness is narrowed to non-derived Int/Fixed and
/// the whole-number check to Int.
#[test]
fn item10_prefab_completeness_narrowed() {
    // A derived Int, a Bool and a Tags stat under a prefab that lacks them are
    // clean (missing_stat skips derived and non-numeric stats).
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ruleset(r) = doc {
                r.stats.push(crpg_data::StatDecl {
                    name: "derived_hp".into(),
                    kind: StatKindWire::Int,
                    derived: Some(ExprWire::Literal(StatValueWire::Int(5))),
                });
                r.stats.push(crpg_data::StatDecl {
                    name: "flag".into(),
                    kind: StatKindWire::Bool,
                    derived: None,
                });
                r.stats.push(crpg_data::StatDecl {
                    name: "tags".into(),
                    kind: StatKindWire::Tags,
                    derived: None,
                });
            }
        }
    });
    assert!(
        !findings
            .iter()
            .any(|d| d.code == DiagnosticCode::MissingStat),
        "{findings:?}"
    );

    // A missing plain Int stat still reports missing_stat.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Creature(c) = doc {
                if c.id == id(36) {
                    c.stats.remove("ward");
                }
            }
        }
    });
    assert!(findings
        .iter()
        .any(|d| d.code == DiagnosticCode::MissingStat));

    // A fractional Fixed value is clean (the whole-number check is Int-only).
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ruleset(r) = doc {
                r.stats.push(crpg_data::StatDecl {
                    name: "weight".into(),
                    kind: StatKindWire::Fixed,
                    derived: None,
                });
            }
            if let Document::Creature(c) = doc {
                if c.id == id(36) {
                    c.stats.insert("weight".into(), Fx16_16::from_raw(1));
                }
            }
        }
    });
    assert!(
        !findings
            .iter()
            .any(|d| d.code == DiagnosticCode::InvalidStatValue),
        "{findings:?}"
    );

    // A missing non-derived Fixed stat still reports missing_stat.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ruleset(r) = doc {
                r.stats.push(crpg_data::StatDecl {
                    name: "weight".into(),
                    kind: StatKindWire::Fixed,
                    derived: None,
                });
            }
        }
    });
    assert!(
        findings
            .iter()
            .any(|d| d.code == DiagnosticCode::MissingStat && d.message.contains("weight")),
        "{findings:?}"
    );
}

/// Behaviour 11: the real-time-fields warning, its pointer, message and fix.
#[test]
fn item11_rt_warning() {
    // recovery_ticks present under a roundless ruleset: one warning per
    // ability at /recovery_ticks.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ability(a) = doc {
                a.recovery_ticks = Some(60);
            }
        }
    });
    assert_eq!(findings.len(), 2, "{findings:?}");
    for f in &findings {
        assert_eq!(f.severity, Severity::Warning);
        assert_eq!(f.code, DiagnosticCode::RealTimeFieldsWithoutRoundTicks);
        assert_eq!(f.code.as_str(), "real_time_fields_without_round_ticks");
        assert_eq!(f.pointer, "/recovery_ticks");
        assert_eq!(
            f.suggested_fix.as_deref(),
            Some("add round_ticks to the ruleset or remove the real-time fields")
        );
    }
    let heavy = findings
        .iter()
        .find(|f| f.message.contains(&id(39).to_string()))
        .expect("heavy warning");
    assert_eq!(
        heavy.message,
        format!(
            "ability {} declares real-time fields but ruleset {} has no round_ticks",
            id(39),
            id(38)
        )
    );

    // When both real-time fields are present, the pointer is /recovery_ticks.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ability(a) = doc {
                a.recovery_ticks = Some(60);
                a.repeats_in_real_time = Some(true);
            }
        }
    });
    assert_eq!(findings.len(), 2, "{findings:?}");
    assert!(findings.iter().all(|f| f.pointer == "/recovery_ticks"));

    // repeats_in_real_time == Some(true) only: pointer is /repeats_in_real_time.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ability(a) = doc {
                a.repeats_in_real_time = Some(true);
            }
        }
    });
    assert_eq!(findings.len(), 2);
    assert!(findings
        .iter()
        .all(|f| f.pointer == "/repeats_in_real_time"));

    // repeats_in_real_time == Some(false) only: clean.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            if let Document::Ability(a) = doc {
                a.repeats_in_real_time = Some(false);
            }
        }
    });
    assert!(findings.is_empty(), "{findings:?}");

    // With round_ticks on the ruleset: clean.
    let findings = mutate_srd(|docs| {
        for doc in docs.values_mut() {
            match doc {
                Document::Ruleset(r) => r.round_ticks = Some(120),
                Document::Ability(a) => a.recovery_ticks = Some(60),
                _ => {}
            }
        }
    });
    assert!(findings.is_empty(), "{findings:?}");

    // An orphan ability (not listed by any ruleset) produces no warning.
    let findings = mutate_srd(|docs| {
        docs.insert(
            "abilities/orphan-rt.json".parse().unwrap(),
            Document::Ability(crpg_data::Ability {
                id: id(204),
                slug: "orphan-rt".into(),
                name: "fixture.orphan-rt".into(),
                note: None,
                dice: "1d6".into(),
                attribute: "might".into(),
                outcome_table: id(41),
                damage: vec![crpg_data::DamageEntry {
                    outcome: crpg_data::OutcomeWire::Success,
                    amount: 0,
                    dice: None,
                }],
                cost: 1,
                extra_costs: Vec::new(),
                ends_turn: true,
                effect: None,
                defense: crpg_data::DefenseWire::ActorAttribute,
                natural_die: None,
                requires_target: false,
                allow_self_target: false,
                recovery_ticks: Some(60),
                repeats_in_real_time: None,
                bonus_stats: None,
            }),
        );
    });
    assert!(
        !findings
            .iter()
            .any(|d| d.code == DiagnosticCode::RealTimeFieldsWithoutRoundTicks),
        "{findings:?}"
    );
}

// ---------------------------------------------------------------------------
// Item 13 — Schemas gain only optional properties.
// ---------------------------------------------------------------------------

/// Behaviour 13: the three schema files hold the new optional properties, none
/// of which is `required`.
#[test]
fn item13_schemas_optional_properties() {
    let schemas = crpg_data::generated_schemas().unwrap();
    assert_eq!(schemas.len(), 22);
    for (file, added) in [
        ("ruleset.schema.json", vec!["initiative"]),
        ("ability.schema.json", vec!["bonus_stats"]),
        ("effect.schema.json", vec!["grants"]),
    ] {
        let root: Value = serde_json::from_slice(&schemas[file]).unwrap();
        let shape = &root["oneOf"][0];
        let properties = shape["properties"].as_object().unwrap();
        let required: Vec<&str> = shape["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry.as_str().unwrap())
            .collect();
        for name in added {
            assert!(properties.contains_key(name), "{file} lacks {name}");
            assert!(!required.contains(&name), "{file} requires {name}");
        }
    }
}

// ---------------------------------------------------------------------------
// Item 14 — Existing content reads today's value with each new field absent.
// ---------------------------------------------------------------------------

/// Behaviour 14: combat_basic and combat_srd validate clean, every new field
/// absent.
#[test]
fn item14_existing_content_clean() {
    for dir in [
        "campaigns/fixtures/combat_basic",
        "campaigns/fixtures/combat_srd",
    ] {
        let mut files = BTreeMap::new();
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
        let root = repo_root().join(dir);
        walk(&root, &root, &mut files);
        let engine: semver::Version = "0.1.0".parse().unwrap();
        let loaded = crpg_data::load_campaign(&files, &engine).expect("campaign loads");
        assert_eq!(
            crpg_data::validate(&loaded),
            Vec::new(),
            "{dir} has findings"
        );
        for doc in loaded.documents.values() {
            match doc {
                Document::Ruleset(r) => {
                    assert_eq!(r.initiative, None);
                    assert!(r.stats.iter().all(|s| s.derived.is_none()));
                }
                Document::Ability(a) => assert_eq!(a.bonus_stats, None),
                Document::Effect(e) => assert_eq!(e.grants, None),
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Item 15 — Kernel mapping: one wire literal per row decodes data-side.
// ---------------------------------------------------------------------------

/// Behaviour 15: each mapping row's wire literal decodes to the documented
/// data value (data-side only).
#[test]
fn item15_kernel_mapping_wire_literals() {
    // StatKindWire -> StatKind.
    let doc = read_ok(&ruleset(vec![stat("aux", json!("fixed"))]));
    let Document::Ruleset(r) = &doc else {
        panic!("ruleset")
    };
    assert_eq!(r.stats[2].kind, StatKindWire::Fixed);

    // StatDecl{name, kind, derived}.
    let doc = read_ok(&ruleset(vec![stat_derived(
        "aux",
        json!("int"),
        json!({"type": "literal", "value": {"type": "int", "value": 7}}),
    )]));
    let Document::Ruleset(r) = &doc else {
        panic!("ruleset")
    };
    assert_eq!(r.stats[2].name, "aux");
    assert_eq!(
        r.stats[2].derived,
        Some(ExprWire::Literal(StatValueWire::Int(7)))
    );

    // ExprWire / StatValueWire: Stat(name), Literal, Add, tags and dice.
    let doc = read_ok(&ruleset(vec![stat_derived(
        "aux",
        json!("int"),
        json!({"type": "stat", "value": "might"}),
    )]));
    let Document::Ruleset(r) = &doc else {
        panic!("ruleset")
    };
    assert_eq!(r.stats[2].derived, Some(ExprWire::Stat("might".into())));
    let doc = read_ok(&ruleset(vec![stat_derived(
        "aux",
        json!("tags"),
        json!({"type": "literal", "value": {"type": "tags", "value": ["a"]}}),
    )]));
    let Document::Ruleset(r) = &doc else {
        panic!("ruleset")
    };
    assert_eq!(
        r.stats[2].derived,
        Some(ExprWire::Literal(StatValueWire::Tags(vec!["a".into()])))
    );

    // Ability.bonus_stats.
    let mut a = ability();
    a["bonus_stats"] = json!(["might"]);
    assert_eq!(
        ability_of(&read_ok(&a)).bonus_stats,
        Some(vec!["might".to_string()])
    );

    // DefenseWire Constant / ActorStat / Opposed.
    let mut a = ability();
    a["defense"] = json!({"type": "constant", "value": 12});
    assert_eq!(
        ability_of(&read_ok(&a)).defense,
        crpg_data::DefenseWire::Constant(12)
    );
    let mut a = ability();
    a["defense"] = json!({"type": "actor_stat", "value": {"stat": "guile"}});
    assert_eq!(
        ability_of(&read_ok(&a)).defense,
        crpg_data::DefenseWire::ActorStat {
            stat: "guile".into()
        }
    );
    let mut a = ability();
    a["defense"] = json!({"type": "opposed", "value": {"dice": "2d6", "bonus_stats": ["might"]}});
    assert_eq!(
        ability_of(&read_ok(&a)).defense,
        crpg_data::DefenseWire::Opposed {
            dice: "2d6".into(),
            bonus_stats: vec!["might".into()]
        }
    );

    // DamageEntry{amount, dice}.
    let mut a = ability();
    a["damage"][0]["dice"] = json!("2d8");
    let entry = &ability_of(&read_ok(&a)).damage[0];
    assert_eq!(entry.amount, 2);
    assert_eq!(entry.dice, Some("2d8".into()));

    // Ruleset.initiative.
    let mut r = ruleset(vec![]);
    r["initiative"] = json!({"dice": "1d20", "bonus_stats": ["might"]});
    let Some(init) = &ruleset_of(&read_ok(&r)).initiative else {
        panic!("initiative")
    };
    assert_eq!(init.dice, "1d20");
    assert_eq!(init.bonus_stats, vec!["might".to_string()]);

    // EffectTargetWire::Stat.
    let mut e = effect();
    e["modifiers"][0]["target"] = json!({"type": "stat", "value": {"stat": "might"}});
    assert_eq!(
        effect_of(&read_ok(&e)).modifiers[0].target,
        EffectTargetWire::Stat {
            stat: "might".into()
        }
    );

    // Effect.grants and Creature tags.
    let mut e = effect();
    e["grants"] = json!(["tag1", "tag2"]);
    assert_eq!(
        effect_of(&read_ok(&e)).grants,
        Some(vec!["tag1".into(), "tag2".into()])
    );
}
