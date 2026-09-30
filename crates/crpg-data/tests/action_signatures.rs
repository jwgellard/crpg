#![forbid(unsafe_code)]
//! IR action-signature declarations and bounded call validation (T029a, D16).
//!
//! Every assertion goes through the production entry points: the store
//! constructor, bundle reader/writer, and call validate/read/write. Oracles
//! are independently authored literals (canonical bytes and a BLAKE3 digest
//! computed here from hand-written preimage bytes), never the store's own
//! private map compared with itself.

use crpg_core::{Fx16_16, Ulid};
use crpg_data::*;

fn bundle_id() -> Ulid {
    Ulid::from_u128(7)
}

fn param(name: &str, value_type: ValueType, required: bool) -> ActionParameter {
    ActionParameter {
        name: name.to_owned(),
        value_type,
        required,
    }
}

fn sig(action_id: &str, parameters: Vec<ActionParameter>) -> ActionSignature {
    ActionSignature {
        action_id: action_id.to_owned(),
        parameters,
    }
}

fn call(action_id: &str, args: Vec<(&str, DataValue)>) -> ActionCall {
    ActionCall {
        action_id: action_id.to_owned(),
        args: args
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    }
}

/// The small two-action bundle used by the literal oracle.
fn small() -> Vec<ActionSignature> {
    vec![
        sig(
            "b.move",
            vec![
                param("target", ValueType::ObjectRef, true),
                param("speed", ValueType::Unsigned, false),
            ],
        ),
        sig("a.say", vec![param("text", ValueType::Text, true)]),
    ]
}

fn store(signatures: Vec<ActionSignature>) -> ActionSignatureStore {
    ActionSignatureStore::new(bundle_id(), signatures).expect("valid declarations")
}

fn code_at(error: &SignatureError) -> (SignatureErrorCode, &str) {
    (error.code.clone(), error.pointer.as_str())
}

fn limit(kind: SignatureLimit, limit: usize) -> SignatureErrorCode {
    SignatureErrorCode::LimitExceeded { kind, limit }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

const SMALL_ACTIONS: &str = r#"  "actions": [
    {
      "action_id": "a.say",
      "parameters": [
        {
          "name": "text",
          "required": true,
          "value_type": "text"
        }
      ]
    },
    {
      "action_id": "b.move",
      "parameters": [
        {
          "name": "target",
          "required": true,
          "value_type": "object_ref"
        },
        {
          "name": "speed",
          "required": false,
          "value_type": "unsigned"
        }
      ]
    }
  ],
  "bundle": "00000000000000000000000007",
"#;

#[test]
fn bundle_revision_is_canonical() {
    // Independent oracle: hand-written canonical preimage and wire bytes.
    let preimage = format!("{{\n{SMALL_ACTIONS}  \"version\": 1\n}}\n");
    let revision = hex(blake3::hash(preimage.as_bytes()).as_bytes());
    let expected =
        format!("{{\n{SMALL_ACTIONS}  \"revision\": \"{revision}\",\n  \"version\": 1\n}}\n");

    let store = store(small());
    assert_eq!(store.identity().bundle, bundle_id());
    assert_eq!(store.identity().revision.to_string(), revision);
    let bytes = write_action_signatures(&store).expect("writes");
    assert_eq!(String::from_utf8(bytes.clone()).expect("utf-8"), expected);
    // The writer's form is exactly the crate's canonical JSON of the same
    // content through the unchanged ActionSignature serde.
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(canonical_json(&value).expect("canonical"), bytes);

    // Round trip is exact; input order does not matter.
    let read = read_action_signatures(&bytes).expect("reads");
    assert_eq!(read, store);
    assert_eq!(write_action_signatures(&read).expect("writes"), bytes);
    let mut reversed = small();
    reversed.reverse();
    assert_eq!(
        write_action_signatures(&self::store(reversed)).expect("writes"),
        bytes
    );

    // The empty bundle is valid and canonical too.
    let empty = self::store(Vec::new());
    assert!(empty.is_empty());
    let preimage = "{\n  \"actions\": [],\n  \"bundle\": \"00000000000000000000000007\",\n  \"version\": 1\n}\n";
    assert_eq!(
        empty.identity().revision.to_string(),
        hex(blake3::hash(preimage.as_bytes()).as_bytes())
    );
}

#[test]
fn bundle_identity_is_exact() {
    let base = store(small());
    let identity = base.identity().clone();
    let variants: Vec<(&str, ActionSignatureStore)> = vec![
        (
            "parameter order",
            store(vec![
                sig(
                    "b.move",
                    vec![
                        param("speed", ValueType::Unsigned, false),
                        param("target", ValueType::ObjectRef, true),
                    ],
                ),
                sig("a.say", vec![param("text", ValueType::Text, true)]),
            ]),
        ),
        (
            "required flag",
            store(vec![
                sig(
                    "b.move",
                    vec![
                        param("target", ValueType::ObjectRef, true),
                        param("speed", ValueType::Unsigned, true),
                    ],
                ),
                sig("a.say", vec![param("text", ValueType::Text, true)]),
            ]),
        ),
        (
            "value type",
            store(vec![
                sig(
                    "b.move",
                    vec![
                        param("target", ValueType::ObjectRef, true),
                        param("speed", ValueType::Integer, false),
                    ],
                ),
                sig("a.say", vec![param("text", ValueType::Text, true)]),
            ]),
        ),
        (
            "parameter name",
            store(vec![
                sig(
                    "b.move",
                    vec![
                        param("target", ValueType::ObjectRef, true),
                        param("pace", ValueType::Unsigned, false),
                    ],
                ),
                sig("a.say", vec![param("text", ValueType::Text, true)]),
            ]),
        ),
        (
            "action id",
            store(vec![
                sig(
                    "b.walk",
                    vec![
                        param("target", ValueType::ObjectRef, true),
                        param("speed", ValueType::Unsigned, false),
                    ],
                ),
                sig("a.say", vec![param("text", ValueType::Text, true)]),
            ]),
        ),
        ("added action", {
            let mut more = small();
            more.push(sig("c.wait", Vec::new()));
            store(more)
        }),
        (
            "bundle id",
            ActionSignatureStore::new(Ulid::from_u128(8), small()).expect("valid"),
        ),
    ];
    let mut seen = vec![identity.clone()];
    for (what, variant) in &variants {
        assert_ne!(variant.identity(), &identity, "{what} must change identity");
        assert!(!seen.contains(variant.identity()), "{what} collides");
        seen.push(variant.identity().clone());
        // A call naming another bundle's identity is refused at the root.
        let error = base
            .validate_call(variant.identity(), &call("a.say", vec![]))
            .expect_err("mismatch");
        assert_eq!(code_at(&error), (SignatureErrorCode::BundleMismatch, ""));
        assert_eq!(error.to_string(), "bundle_mismatch at /");
    }
    // Same bundle id, forged revision: still a mismatch.
    let forged = ActionBundleIdentity {
        bundle: identity.bundle,
        revision: Digest::from_bytes([0; 32]),
    };
    assert_eq!(
        base.validate_call(
            &forged,
            &call("a.say", vec![("text", DataValue::Text("hi".into()))])
        )
        .map_err(|error| error.code),
        Err(SignatureErrorCode::BundleMismatch)
    );
    // Identity serde is strict.
    let json = serde_json::to_value(&identity).expect("serialize");
    let back: ActionBundleIdentity = serde_json::from_value(json.clone()).expect("round trip");
    assert_eq!(back, identity);
    let mut extra = json;
    extra["extra"] = serde_json::Value::from(1);
    assert!(serde_json::from_value::<ActionBundleIdentity>(extra).is_err());
}

/// Every permutation of `items` (small inputs only).
fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for index in 0..items.len() {
        let mut rest = items.to_vec();
        let head = rest.remove(index);
        for mut tail in permutations(&rest) {
            tail.insert(0, head.clone());
            out.push(tail);
        }
    }
    out
}

fn assert_stable(signatures: &[ActionSignature], expected: (SignatureErrorCode, &str), what: &str) {
    for order in permutations(signatures) {
        let error = ActionSignatureStore::new(bundle_id(), order).expect_err(what);
        assert_eq!(code_at(&error), expected.clone(), "{what}");
    }
}

#[test]
fn declaration_permutation_errors_are_stable() {
    let dup_params = sig(
        "zz",
        vec![
            param("p", ValueType::Bool, true),
            param("p", ValueType::Bool, true),
        ],
    );
    let too_long = "a".repeat(129);
    // Invalid ids first, lexically: "" sorts before the 129-byte id.
    assert_stable(
        &[
            dup_params.clone(),
            sig("", Vec::new()),
            sig(&too_long, Vec::new()),
            sig("beta", Vec::new()),
            sig("beta", Vec::new()),
        ],
        (SignatureErrorCode::InvalidIdentifier, "/actions/"),
        "invalid ids",
    );
    assert_stable(
        &[
            dup_params.clone(),
            sig(&too_long, Vec::new()),
            sig("beta", Vec::new()),
            sig("beta", Vec::new()),
        ],
        (
            SignatureErrorCode::InvalidIdentifier,
            &format!("/actions/{too_long}"),
        ),
        "too-long id",
    );
    // Then the lexically first duplicate action id.
    assert_stable(
        &[
            dup_params.clone(),
            sig("gamma", Vec::new()),
            sig("gamma", Vec::new()),
            sig("beta", Vec::new()),
            sig("beta", Vec::new()),
        ],
        (SignatureErrorCode::DuplicateAction, "/actions/beta"),
        "duplicate actions",
    );
    // Then per action, lexically: the parameter count of "mid" precedes the
    // parameter problems of "zz".
    let crowded: Vec<ActionParameter> = (0..33)
        .map(|index| param(&format!("p{index:02}"), ValueType::Bool, false))
        .collect();
    assert_stable(
        &[dup_params.clone(), sig("mid", crowded.clone())],
        (
            limit(SignatureLimit::Parameters, MAX_ACTION_PARAMETERS),
            "/actions/mid/parameters",
        ),
        "parameter count",
    );
    // Within one action: invalid names (lexical) precede duplicates, and
    // pointers escape `/` and `~`.
    let messy = |order: &[&str]| {
        sig(
            "a/b~c",
            order
                .iter()
                .map(|name| param(name, ValueType::Bool, false))
                .collect(),
        )
    };
    for order in permutations(&["z/1", "", "b~", "b~"]) {
        let error = ActionSignatureStore::new(bundle_id(), vec![messy(&order)]).expect_err("bad");
        assert_eq!(
            code_at(&error),
            (
                SignatureErrorCode::InvalidIdentifier,
                "/actions/a~1b~0c/parameters/"
            )
        );
    }
    for order in permutations(&["z/1", "b~", "b~"]) {
        let error = ActionSignatureStore::new(bundle_id(), vec![messy(&order)]).expect_err("bad");
        assert_eq!(
            code_at(&error),
            (
                SignatureErrorCode::DuplicateParameter,
                "/actions/a~1b~0c/parameters/b~0"
            )
        );
        assert_eq!(
            error.to_string(),
            "duplicate_parameter at /actions/a~1b~0c/parameters/b~0"
        );
    }
    // The action count is checked before anything else, even duplicates.
    let flood: Vec<ActionSignature> = (0..=MAX_ACTION_SIGNATURES)
        .map(|_| sig("", Vec::new()))
        .collect();
    let error = ActionSignatureStore::new(bundle_id(), flood).expect_err("too many");
    assert_eq!(
        code_at(&error),
        (
            limit(SignatureLimit::Actions, MAX_ACTION_SIGNATURES),
            "/actions"
        )
    );
    assert_eq!(error.to_string(), "limit_exceeded at /actions");
    // Limit controls: exactly 1024 actions and exactly 32 parameters.
    let full: Vec<ActionSignature> = (0..MAX_ACTION_SIGNATURES)
        .map(|index| sig(&format!("action.{index:04}"), Vec::new()))
        .collect();
    assert_eq!(store(full).len(), MAX_ACTION_SIGNATURES);
    let store = store(vec![sig("mid", crowded[..32].to_vec())]);
    assert_eq!(store.get("mid").expect("declared").parameters.len(), 32);
}

#[test]
fn parameters_preserve_declared_order() {
    let declared = vec![
        param("zeta", ValueType::Bool, true),
        param("alpha", ValueType::Bool, true),
        param("mu", ValueType::Bool, false),
    ];
    let store = store(vec![
        sig("x", declared.clone()),
        sig("b", Vec::new()),
        sig("a", Vec::new()),
    ]);
    assert_eq!(store.get("x").expect("declared").parameters, declared);
    let ids: Vec<&str> = store.iter().map(|(id, _)| id).collect();
    assert_eq!(ids, vec!["a", "b", "x"], "lexical id order");
    assert!(store.get("X").is_none(), "exact spelling only");
    assert!(store.get("").is_none(), "inert for invalid ids");
    assert!(store.get(&"a".repeat(200)).is_none());

    // Written bytes keep declaration order; a round trip preserves it.
    let bytes = write_action_signatures(&store).expect("writes");
    let text = String::from_utf8(bytes.clone()).expect("utf-8");
    let at = |needle: &str| text.find(needle).expect("present");
    assert!(at("\"zeta\"") < at("\"alpha\"") && at("\"alpha\"") < at("\"mu\""));
    let read = read_action_signatures(&bytes).expect("reads");
    assert_eq!(read.get("x").expect("declared").parameters, declared);

    // Missing arguments are reported in declaration order, not lexically.
    let error = store
        .validate_call(store.identity(), &call("x", vec![]))
        .expect_err("missing");
    assert_eq!(
        code_at(&error),
        (SignatureErrorCode::MissingArgument, "/args/zeta")
    );
    let ok = store
        .validate_call(
            store.identity(),
            &call(
                "x",
                vec![
                    ("zeta", DataValue::Bool(true)),
                    ("alpha", DataValue::Bool(false)),
                ],
            ),
        )
        .expect("optional mu may be absent");
    assert_eq!(ok.action_id, "x");
}

/// One value of every category, in `ValueType` declaration order.
fn samples() -> Vec<(ValueType, DataValue)> {
    vec![
        (ValueType::Bool, DataValue::Bool(true)),
        (ValueType::Integer, DataValue::Integer(-1)),
        (ValueType::Unsigned, DataValue::Unsigned(1)),
        (ValueType::Fixed, DataValue::Fixed(Fx16_16::from_int(1))),
        (ValueType::Text, DataValue::Text("t".to_owned())),
        (
            ValueType::ObjectRef,
            DataValue::ObjectRef(Ulid::from_u128(9)),
        ),
        (
            ValueType::List,
            DataValue::List(vec![DataValue::Bool(false), DataValue::Integer(2)]),
        ),
        (
            ValueType::Map,
            DataValue::Map(
                [("k".to_owned(), DataValue::Text("v".to_owned()))]
                    .into_iter()
                    .collect(),
            ),
        ),
    ]
}

fn type_name(value_type: &ValueType) -> String {
    serde_json::to_value(value_type)
        .expect("serialize")
        .as_str()
        .expect("text")
        .to_owned()
}

#[test]
fn all_eight_categories_no_coercion() {
    let samples = samples();
    let parameters: Vec<ActionParameter> = samples
        .iter()
        .map(|(value_type, _)| param(&type_name(value_type), value_type.clone(), true))
        .collect();
    let store = store(vec![sig("all", parameters)]);
    let valid: Vec<(String, DataValue)> = samples
        .iter()
        .map(|(value_type, value)| (type_name(value_type), value.clone()))
        .collect();
    let valid_call = ActionCall {
        action_id: "all".to_owned(),
        args: valid.iter().cloned().collect(),
    };
    store
        .validate_call(store.identity(), &valid_call)
        .expect("every category matches");
    // Heterogeneous containers only constrain their root category.
    for (target_type, _) in &samples {
        for (actual_type, value) in &samples {
            if actual_type == target_type {
                continue;
            }
            let mut wrong = valid_call.clone();
            wrong.args.insert(type_name(target_type), value.clone());
            let error = store
                .validate_call(store.identity(), &wrong)
                .expect_err("no coercion");
            assert_eq!(
                code_at(&error),
                (
                    SignatureErrorCode::TypeMismatch {
                        expected: target_type.clone(),
                        actual: actual_type.clone(),
                    },
                    format!("/args/{}", type_name(target_type)).as_str()
                )
            );
            assert_eq!(
                error.to_string(),
                format!("type_mismatch at /args/{}", type_name(target_type))
            );
        }
    }
}

#[test]
fn optional_is_not_wrong_typed() {
    let store = store(vec![sig(
        "act",
        vec![
            param("need", ValueType::Integer, true),
            param("maybe", ValueType::Text, false),
        ],
    )]);
    let id = store.identity().clone();
    let need = ("need", DataValue::Integer(3));
    store
        .validate_call(&id, &call("act", vec![need.clone()]))
        .expect("optional absent");
    store
        .validate_call(
            &id,
            &call(
                "act",
                vec![need.clone(), ("maybe", DataValue::Text("x".into()))],
            ),
        )
        .expect("optional supplied correctly");
    let error = store
        .validate_call(
            &id,
            &call("act", vec![need.clone(), ("maybe", DataValue::Unsigned(1))]),
        )
        .expect_err("optional wrong type");
    assert_eq!(
        code_at(&error),
        (
            SignatureErrorCode::TypeMismatch {
                expected: ValueType::Text,
                actual: ValueType::Unsigned,
            },
            "/args/maybe"
        )
    );
    // Signed, unsigned and fixed stay distinct.
    let error = store
        .validate_call(&id, &call("act", vec![("need", DataValue::Unsigned(3))]))
        .expect_err("unsigned is not integer");
    assert_eq!(
        error.code,
        SignatureErrorCode::TypeMismatch {
            expected: ValueType::Integer,
            actual: ValueType::Unsigned,
        }
    );
    let error = store
        .validate_call(&id, &call("act", vec![]))
        .expect_err("required missing");
    assert_eq!(
        code_at(&error),
        (SignatureErrorCode::MissingArgument, "/args/need")
    );
    // Extra arguments (lexical) precede missing and type errors.
    let error = store
        .validate_call(
            &id,
            &call(
                "act",
                vec![
                    ("zz", DataValue::Bool(true)),
                    ("extra", DataValue::Bool(true)),
                    ("maybe", DataValue::Bool(true)),
                ],
            ),
        )
        .expect_err("extra");
    assert_eq!(
        code_at(&error),
        (SignatureErrorCode::ExtraArgument, "/args/extra")
    );
    let error = store
        .validate_call(&id, &call("nope", vec![("extra", DataValue::Bool(true))]))
        .expect_err("unknown");
    assert_eq!(
        code_at(&error),
        (SignatureErrorCode::UnknownAction, "/action_id")
    );
}

/// A value nested `depth` levels deep, alternating list and map containers,
/// with a boolean leaf at the bottom (the leaf counts as one level).
fn nested(depth: usize) -> DataValue {
    let mut value = DataValue::Bool(true);
    for level in 1..depth {
        value = if level % 2 == 0 {
            DataValue::List(vec![value])
        } else {
            DataValue::Map([("k".to_owned(), value)].into_iter().collect())
        };
    }
    value
}

/// The pointer of the deepest node of `nested(depth)` under `/args/<arg>`.
fn nested_pointer(arg: &str, depth: usize) -> String {
    let mut pointer = format!("/args/{arg}");
    for level in (1..depth).rev() {
        pointer.push_str(if level % 2 == 0 {
            "/value/0"
        } else {
            "/value/k"
        });
    }
    pointer
}

#[test]
fn call_budget_boundaries() {
    let open: Vec<ActionParameter> = (0..32)
        .map(|index| param(&format!("a{index:02}"), ValueType::Text, false))
        .collect();
    let deep_params = vec![
        param("d", ValueType::Map, false),
        param("e", ValueType::List, false),
        param("m", ValueType::Map, false),
        param("t", ValueType::Text, false),
        param("x", ValueType::List, false),
        param("y", ValueType::List, false),
    ];
    let store = store(vec![sig("open", open), sig("deep", deep_params)]);
    let id = store.identity().clone();
    let check = |call: &ActionCall| store.validate_call(&id, call).map(|_| ());

    // Arguments: 32 accepted, 33 refused at /args.
    let mut args: Vec<(String, DataValue)> = (0..32)
        .map(|index| (format!("a{index:02}"), DataValue::Text("v".into())))
        .collect();
    let at_limit = ActionCall {
        action_id: "open".into(),
        args: args.iter().cloned().collect(),
    };
    check(&at_limit).expect("32 arguments");
    args.push(("a32".into(), DataValue::Text("v".into())));
    let over = ActionCall {
        action_id: "open".into(),
        args: args.into_iter().collect(),
    };
    assert_eq!(
        check(&over).map_err(|e| code_at(&e).0),
        Err(limit(SignatureLimit::Arguments, 32))
    );
    assert_eq!(check(&over).unwrap_err().pointer, "/args");

    // Depth: alternating list/map nesting, 32 accepted, 33 refused at the
    // first node past the limit.
    check(&call("deep", vec![("d", nested(32))])).expect("depth 32");
    let error = check(&call("deep", vec![("d", nested(33))])).expect_err("depth 33");
    assert_eq!(
        code_at(&error),
        (
            limit(SignatureLimit::ValueDepth, 32),
            nested_pointer("d", 33).as_str()
        )
    );
    // In-memory nesting far past serde's recursion limit fails iteratively.
    let error = check(&call("deep", vec![("d", nested(1000))])).expect_err("deep");
    assert_eq!(error.code, limit(SignatureLimit::ValueDepth, 32));

    // Nodes: the total spans arguments (lexical order). 4096 nodes pass the
    // node budget (then exceed the canonical size); 4097 fail at the first
    // extra node.
    let list = |items: usize| DataValue::List(vec![DataValue::Bool(true); items]);
    let error = check(&call("deep", vec![("x", list(2047)), ("y", list(2047))]))
        .expect_err("4096 nodes are too large to encode");
    assert_eq!(
        code_at(&error),
        (limit(SignatureLimit::CallBytes, 65_536), "")
    );
    let error =
        check(&call("deep", vec![("x", list(2047)), ("y", list(2048))])).expect_err("4097 nodes");
    assert_eq!(
        code_at(&error),
        (
            limit(SignatureLimit::ValueNodes, 4096),
            "/args/y/value/2047"
        )
    );
    check(&call("deep", vec![("x", list(100)), ("y", list(100))])).expect("valid control");
    // Empty containers still count one node each.
    let empties = DataValue::List(vec![DataValue::List(Vec::new()); 4096]);
    let error = check(&call("deep", vec![("e", empties)])).expect_err("4097 nodes");
    assert_eq!(
        code_at(&error),
        (
            limit(SignatureLimit::ValueNodes, 4096),
            "/args/e/value/4095"
        )
    );

    // Strings: 4096 bytes accepted, 4097 refused at the text payload.
    check(&call(
        "deep",
        vec![("t", DataValue::Text("é".repeat(2048)))],
    ))
    .expect("4096 bytes");
    let error = check(&call(
        "deep",
        vec![("t", DataValue::Text(format!("{}a", "é".repeat(2048))))],
    ))
    .expect_err("4097 bytes");
    assert_eq!(
        code_at(&error),
        (
            limit(SignatureLimit::ValueStringBytes, 4096),
            "/args/t/value"
        )
    );
    // Map keys: checked before their child, pointing at the key's value.
    let key_ok = format!("a/{}", "~".repeat(4094));
    let key_bad = format!("{key_ok}x");
    check(&call(
        "deep",
        vec![(
            "m",
            DataValue::Map([(key_ok, DataValue::Bool(true))].into_iter().collect()),
        )],
    ))
    .expect("4096-byte key");
    let error = check(&call(
        "deep",
        vec![(
            "m",
            DataValue::Map([(key_bad, nested(40))].into_iter().collect()),
        )],
    ))
    .expect_err("4097-byte key");
    assert_eq!(error.code, limit(SignatureLimit::ValueStringBytes, 4096));
    assert_eq!(
        error.pointer,
        format!("/args/m/value/a~1{}x", "~0".repeat(4094))
    );
    // Empty map keys are allowed.
    check(&call(
        "deep",
        vec![(
            "m",
            DataValue::Map(
                [(String::new(), DataValue::Bool(true))]
                    .into_iter()
                    .collect(),
            ),
        )],
    ))
    .expect("empty key");

    // Canonical size, escaping and pretty-print overhead included: control
    // characters count 1 byte against the string budget but encode as 6.
    let sized = |filler: usize| {
        let mut args: Vec<(String, DataValue)> = (0..2)
            .map(|index| {
                (
                    format!("a{index:02}"),
                    DataValue::Text("\u{1}".repeat(4096)),
                )
            })
            .collect();
        for index in 2..5 {
            args.push((format!("a{index:02}"), DataValue::Text("x".repeat(4096))));
        }
        args.push(("a05".into(), DataValue::Text("x".repeat(filler))));
        ActionCall {
            action_id: "open".into(),
            args: args.into_iter().collect(),
        }
    };
    let base = canonical_json(&sized(0)).expect("encodes").len();
    assert!(base < MAX_ACTION_CALL_BYTES && base > 2 * 6 * 4096);
    let exact = sized(MAX_ACTION_CALL_BYTES - base);
    let bytes = store.write_call(&id, &exact).expect("exactly 65536 bytes");
    assert_eq!(bytes.len(), MAX_ACTION_CALL_BYTES);
    assert_eq!(bytes, canonical_json(&exact).expect("encodes"));
    assert_eq!(store.read_call(&id, &bytes).expect("reads back"), exact);
    let error = check(&sized(MAX_ACTION_CALL_BYTES - base + 1)).expect_err("65537 bytes");
    assert_eq!(
        code_at(&error),
        (limit(SignatureLimit::CallBytes, 65_536), "")
    );
    assert_eq!(error.to_string(), "limit_exceeded at /");
    // The raw reader cap fires before parsing or identity checks.
    let mut raw = bytes.clone();
    raw.push(b' ');
    let wrong = ActionBundleIdentity {
        bundle: Ulid::from_u128(1),
        revision: id.revision,
    };
    let error = store.read_call(&wrong, &raw).expect_err("raw cap");
    assert_eq!(
        code_at(&error),
        (limit(SignatureLimit::CallBytes, 65_536), "")
    );
}

#[test]
fn utf8_and_escaped_name_boundaries() {
    // Byte length, not character count: 64 two-byte characters fit, one
    // more ASCII byte does not; 32 four-byte characters fit.
    let two = "é".repeat(64);
    let four = "\u{1F600}".repeat(32);
    let over = format!("{two}a");
    assert_eq!(over.chars().count(), 65);
    let store = store(vec![
        sig(&two, vec![param(&four, ValueType::Bool, true)]),
        sig(&four, Vec::new()),
    ]);
    assert!(store.get(&two).is_some() && store.get(&four).is_some());
    let error = ActionSignatureStore::new(bundle_id(), vec![sig(&over, Vec::new())])
        .expect_err("129 bytes");
    assert_eq!(
        code_at(&error),
        (
            SignatureErrorCode::InvalidIdentifier,
            format!("/actions/{over}").as_str()
        )
    );
    let error = ActionSignatureStore::new(
        bundle_id(),
        vec![sig("x", vec![param(&over, ValueType::Bool, true)])],
    )
    .expect_err("129-byte parameter");
    assert_eq!(error.pointer, format!("/actions/x/parameters/{over}"));
    // No normalization: composed and decomposed spellings are distinct.
    let store2 = self::store(vec![sig("\u{e9}", Vec::new()), sig("e\u{301}", Vec::new())]);
    assert_eq!(store2.len(), 2);

    // Call identifiers follow the same byte rule, action first.
    let id = store.identity().clone();
    store
        .validate_call(&id, &call(&two, vec![(&four, DataValue::Bool(true))]))
        .expect("128-byte names");
    let error = store
        .validate_call(&id, &call(&over, vec![("", DataValue::Bool(true))]))
        .expect_err("action first");
    assert_eq!(
        code_at(&error),
        (SignatureErrorCode::InvalidIdentifier, "/action_id")
    );
    let error = store
        .validate_call(
            &id,
            &call(
                &two,
                vec![("b/~", DataValue::Bool(true)), ("", DataValue::Bool(true))],
            ),
        )
        .expect_err("empty argument name");
    assert_eq!(
        code_at(&error),
        (SignatureErrorCode::InvalidIdentifier, "/args/")
    );
    let error = store
        .validate_call(&id, &call(&two, vec![("b/~", DataValue::Bool(true))]))
        .expect_err("extra");
    assert_eq!(
        code_at(&error),
        (SignatureErrorCode::ExtraArgument, "/args/b~1~0")
    );

    // Worst escaped-name bundle: 1024 actions x 32 parameters, every name
    // 128 bytes of control characters (6 escaped bytes each), still fits.
    let control = |seed: usize, width: usize| -> String {
        let mut name: Vec<char> = vec!['\u{1}'; 128];
        let mut value = seed;
        for slot in name.iter_mut().rev().take(width) {
            *slot = char::from_u32(1 + (value % 31) as u32).expect("control");
            value /= 31;
        }
        name.into_iter().collect()
    };
    let worst: Vec<ActionSignature> = (0..MAX_ACTION_SIGNATURES)
        .map(|action| ActionSignature {
            action_id: control(action, 3),
            parameters: (0..MAX_ACTION_PARAMETERS)
                .map(|index| param(&control(index, 2), ValueType::ObjectRef, false))
                .collect(),
        })
        .collect();
    assert!(worst.iter().all(|s| s.action_id.len() == 128));
    let worst = self::store(worst);
    let bytes = write_action_signatures(&worst).expect("worst case fits");
    assert!(bytes.len() <= MAX_ACTION_BUNDLE_BYTES);
    assert!(
        bytes.len() > 24 * 1024 * 1024,
        "escaping is really exercised"
    );
    assert_eq!(read_action_signatures(&bytes).expect("reads"), worst);
}

#[test]
fn duplicate_json_keys_fail_closed() {
    let store = store(small());
    let bytes = String::from_utf8(write_action_signatures(&store).expect("writes")).expect("utf8");
    let malformed = |text: &str| {
        let error = read_action_signatures(text.as_bytes()).expect_err("malformed");
        assert_eq!(
            code_at(&error),
            (SignatureErrorCode::Malformed, ""),
            "{text}"
        );
    };
    // Duplicate envelope key (a last-key-wins parser would accept this).
    malformed(&bytes.replacen("{\n", "{\n  \"version\": 1,\n", 1));
    // Duplicate key inside a declaration.
    malformed(&bytes.replacen(
        "\"name\": \"text\",",
        "\"name\": \"text\",\n          \"name\": \"text\",",
        1,
    ));
    // Unknown and missing envelope fields, trailing input, fractions,
    // invalid UTF-8, and a non-object root.
    malformed(&bytes.replacen("{\n", "{\n  \"extra\": 1,\n", 1));
    malformed(&bytes.replacen("  \"version\": 1\n", "  \"versio\": 1\n", 1));
    malformed(&format!("{bytes}{{}}"));
    malformed(&bytes.replacen("\"version\": 1", "\"version\": 1.0", 1));
    malformed("[]");
    let mut invalid = bytes.clone().into_bytes();
    invalid.insert(10, 0xff);
    assert_eq!(
        read_action_signatures(&invalid).map_err(|e| e.code),
        Err(SignatureErrorCode::Malformed)
    );
    // Null in a nonnullable declaration field and unknown declaration
    // fields are typed-shape failures under /actions.
    for broken in [
        bytes.replacen("\"required\": true", "\"required\": null", 1),
        bytes.replacen(
            "\"required\": true",
            "\"required\": true,\n          \"x\": 1",
            1,
        ),
    ] {
        let error = read_action_signatures(broken.as_bytes()).expect_err("typed");
        assert_eq!(code_at(&error), (SignatureErrorCode::Malformed, "/actions"));
    }

    // Calls: duplicate argument keys and malformed tags fail closed, even
    // for an unknown action.
    let id = store.identity().clone();
    for text in [
        r#"{"action_id":"a.say","args":{"text":{"type":"text","value":"a"},"text":{"type":"text","value":"b"}}}"#,
        r#"{"action_id":"nope","args":{"x":{"type":"text","type":"text","value":"a"}}}"#,
        r#"{"action_id":"nope","args":{"x":{"type":"float","value":1}}}"#,
        r#"{"action_id":"a.say","args":{"text":{"type":"text","value":"a","more":1}}}"#,
        r#"{"action_id":"a.say","args":{},"extra":1}"#,
        r#"{"action_id":"a.say","args":{}} x"#,
        r#"{"action_id":"a.say","args":{"n":{"type":"integer","value":1.5}}}"#,
    ] {
        let error = store.read_call(&id, text.as_bytes()).expect_err(text);
        assert_eq!(
            code_at(&error),
            (SignatureErrorCode::Malformed, ""),
            "{text}"
        );
    }
    // Valid control through the same reader.
    let good = r#"{"action_id":"a.say","args":{"text":{"type":"text","value":"hi"}}}"#;
    assert_eq!(
        store.read_call(&id, good.as_bytes()).expect("valid"),
        call("a.say", vec![("text", DataValue::Text("hi".into()))])
    );
}

#[test]
fn unknown_version_and_multifault_precedence() {
    let store = store(small());
    let bytes = String::from_utf8(write_action_signatures(&store).expect("writes")).expect("utf8");
    let revision = store.identity().revision.to_string();
    let read = |text: &str| read_action_signatures(text.as_bytes()).map(|_| ());
    let at = |text: &str| {
        let error = read(text).expect_err("rejects");
        (error.code, error.pointer)
    };
    let v2 = bytes.replacen("\"version\": 1", "\"version\": 2", 1);
    assert_eq!(
        at(&v2),
        (SignatureErrorCode::UnsupportedVersion, "/version".into())
    );
    assert_eq!(
        read(&v2).unwrap_err().to_string(),
        "unsupported_version at /version"
    );
    // Syntax first: a broken unknown-version document is Malformed at root.
    assert_eq!(
        at(&format!("{v2},")),
        (SignatureErrorCode::Malformed, String::new())
    );
    // Typed version.
    assert_eq!(
        at(&bytes.replacen("\"version\": 1", "\"version\": \"1\"", 1)),
        (SignatureErrorCode::Malformed, "/version".into())
    );
    assert_eq!(
        at(&bytes.replacen("\"version\": 1", "\"version\": -1", 1)),
        (SignatureErrorCode::Malformed, "/version".into())
    );
    // Version precedes every typed field and the revision.
    let v2_bad = v2.replacen(&revision, "zz", 1);
    assert_eq!(
        at(&v2_bad),
        (SignatureErrorCode::UnsupportedVersion, "/version".into())
    );
    assert_eq!(
        at(&bytes.replacen("00000000000000000000000007", "not-a-ulid", 1)),
        (SignatureErrorCode::Malformed, "/bundle".into())
    );
    assert_eq!(
        at(&bytes.replacen(&revision, "ABC", 1)),
        (SignatureErrorCode::Malformed, "/revision".into())
    );
    assert_eq!(
        at(&bytes.replacen("\"action_id\": \"a.say\"", "\"action_id\": 5", 1)),
        (SignatureErrorCode::Malformed, "/actions".into())
    );
    // Constructor checks precede the revision comparison.
    let duplicated = bytes.replacen("\"b.move\"", "\"a.say\"", 1);
    assert_eq!(
        at(&duplicated),
        (SignatureErrorCode::DuplicateAction, "/actions/a.say".into())
    );
    // A content change without a matching revision is refused.
    let tampered = bytes.replacen("\"required\": false", "\"required\": true", 1);
    assert_eq!(
        at(&tampered),
        (SignatureErrorCode::RevisionMismatch, "/revision".into())
    );
    // The raw cap precedes parsing.
    let huge = vec![b' '; MAX_ACTION_BUNDLE_BYTES + 1];
    let error = read_action_signatures(&huge).expect_err("too large");
    assert_eq!(
        code_at(&error),
        (
            limit(SignatureLimit::BundleBytes, MAX_ACTION_BUNDLE_BYTES),
            ""
        )
    );
    let mut padded = bytes.clone().into_bytes();
    padded.resize(MAX_ACTION_BUNDLE_BYTES, b' ');
    read_action_signatures(&padded).expect("exactly the cap parses");

    // Call precedence, pairwise.
    let id = store.identity().clone();
    let other = ActionSignatureStore::new(Ulid::from_u128(99), small()).expect("valid");
    let first = |identity: &ActionBundleIdentity, call: &ActionCall| {
        let error = store.validate_call(identity, call).expect_err("rejects");
        (error.code, error.pointer)
    };
    assert_eq!(
        first(other.identity(), &call("", vec![])),
        (SignatureErrorCode::BundleMismatch, String::new())
    );
    assert_eq!(
        first(&id, &call("", vec![("", DataValue::Bool(true))])),
        (SignatureErrorCode::InvalidIdentifier, "/action_id".into())
    );
    let many: Vec<(String, DataValue)> = (0..33)
        .map(|index| (format!("n{index:02}"), DataValue::Bool(true)))
        .collect();
    let mut with_bad_name = ActionCall {
        action_id: "a.say".into(),
        args: many.iter().cloned().collect(),
    };
    with_bad_name
        .args
        .insert(String::new(), DataValue::Bool(true));
    assert_eq!(
        first(&id, &with_bad_name),
        (SignatureErrorCode::InvalidIdentifier, "/args/".into())
    );
    let mut deep_and_many = ActionCall {
        action_id: "a.say".into(),
        args: many.into_iter().collect(),
    };
    deep_and_many.args.insert("n00".into(), nested(40));
    assert_eq!(
        first(&id, &deep_and_many).0,
        limit(SignatureLimit::Arguments, 32)
    );
    let huge_and_deep = call(
        "nope",
        vec![
            ("a", nested(40)),
            ("b", DataValue::Text("\u{1}".repeat(4096))),
            ("c", DataValue::Text("\u{1}".repeat(4096))),
            ("d", DataValue::Text("\u{1}".repeat(4096))),
        ],
    );
    assert_eq!(
        first(&id, &huge_and_deep).0,
        limit(SignatureLimit::ValueDepth, 32)
    );
    let huge_unknown = call(
        "nope",
        vec![
            ("b", DataValue::Text("\u{1}".repeat(4096))),
            ("c", DataValue::Text("\u{1}".repeat(4096))),
            ("d", DataValue::Text("\u{1}".repeat(4096))),
        ],
    );
    assert_eq!(
        first(&id, &huge_unknown).0,
        limit(SignatureLimit::CallBytes, 65_536)
    );
    // read_call: identity precedes parsing; parsing precedes validation.
    let broken = b"{\"action_id\": ";
    assert_eq!(
        store
            .read_call(other.identity(), broken)
            .map_err(|e| e.code),
        Err(SignatureErrorCode::BundleMismatch)
    );
    assert_eq!(
        store.read_call(&id, broken).map_err(|e| e.code),
        Err(SignatureErrorCode::Malformed)
    );
}

#[test]
fn failed_calls_are_read_only() {
    let store = store(small());
    let id = store.identity().clone();
    let before_store = store.clone();
    let before_bytes = write_action_signatures(&store).expect("writes");
    let failing = vec![
        call("nope", vec![]),
        call("a.say", vec![]),
        call("a.say", vec![("text", DataValue::Bool(true))]),
        call(
            "a.say",
            vec![
                ("text", DataValue::Text("x".into())),
                ("zz", DataValue::Bool(true)),
            ],
        ),
        call("a.say", vec![("text", nested(33))]),
        call("", vec![]),
    ];
    for candidate in &failing {
        let before_call = candidate.clone();
        assert!(store.validate_call(&id, candidate).is_err());
        assert!(store.write_call(&id, candidate).is_err());
        let encoded = canonical_json(candidate).expect("encodes");
        assert!(store.read_call(&id, &encoded).is_err());
        assert_eq!(candidate, &before_call, "caller data unchanged");
        assert_eq!(store, before_store, "store unchanged");
        assert_eq!(store.identity(), &id);
    }
    assert_eq!(
        write_action_signatures(&store).expect("writes"),
        before_bytes
    );
    // Valid control: the same store accepts and encodes a good call.
    let good = call(
        "b.move",
        vec![
            ("target", DataValue::ObjectRef(Ulid::from_u128(3))),
            ("speed", DataValue::Unsigned(2)),
        ],
    );
    let signature = store.validate_call(&id, &good).expect("valid");
    assert_eq!(signature.action_id, "b.move");
    let bytes = store.write_call(&id, &good).expect("writes");
    assert_eq!(bytes, canonical_json(&good).expect("canonical"));
    assert_eq!(store.read_call(&id, &bytes).expect("reads"), good);
    assert!(
        std::error::Error::source(&store.validate_call(&id, &failing[0]).unwrap_err()).is_none()
    );
}
