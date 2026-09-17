#![forbid(unsafe_code)]
mod support;
use crpg_core::{Fx16_16, Ulid};
use crpg_data::*;
use proptest::prelude::*;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

fn value_strategy() -> BoxedStrategy<DataValue> {
    prop_oneof![
        any::<bool>().prop_map(DataValue::Bool),
        any::<i64>().prop_map(DataValue::Integer),
        any::<u64>().prop_map(DataValue::Unsigned),
        any::<i32>().prop_map(|v| DataValue::Fixed(Fx16_16::from_raw(v))),
        prop::collection::vec(any::<char>(), 0..20)
            .prop_map(|v| DataValue::Text(v.into_iter().collect())),
        any::<u128>().prop_map(|v| DataValue::ObjectRef(Ulid::from_u128(v))),
    ]
    .prop_recursive(3, 48, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..5).prop_map(DataValue::List),
            prop::collection::btree_map("[a-z]{0,6}", inner, 0..5).prop_map(DataValue::Map),
        ]
    })
    .boxed()
}

fn assert_value<T: Serialize + DeserializeOwned + std::fmt::Debug + PartialEq>(value: &T) {
    let bytes = canonical_json(value).unwrap();
    let back: T = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(&back, value);
    assert_eq!(canonical_json(&back).unwrap(), bytes);
}

fn categories() -> Vec<ValueType> {
    vec![
        ValueType::Bool,
        ValueType::Integer,
        ValueType::Unsigned,
        ValueType::Fixed,
        ValueType::Text,
        ValueType::ObjectRef,
        ValueType::List,
        ValueType::Map,
    ]
}

fn triggers(ticks: u64, text: &str) -> Vec<Trigger> {
    vec![
        Trigger::AreaEnter,
        Trigger::AreaExit,
        Trigger::Interact,
        Trigger::DialogueNode,
        Trigger::QuestStateChange,
        Trigger::Timer { ticks },
        Trigger::CombatStart,
        Trigger::CombatEnd,
        Trigger::Death,
        Trigger::ItemAcquired,
        Trigger::Custom { id: text.into() },
    ]
}

fn documents(
    text: String,
    value: DataValue,
    ticks: u64,
    raw: i32,
    identity: u128,
) -> Vec<Document> {
    let id = Ulid::from_u128(identity);
    let args = BTreeMap::from([(text.clone(), value.clone())]);
    let call = ActionCall {
        action_id: text.clone(),
        args: args.clone(),
    };
    let mut result: Vec<_> = support::fixture_files()
        .values()
        .map(|b| read_document(b).unwrap())
        .collect();
    let scopes = [
        VariableScope::Campaign,
        VariableScope::World,
        VariableScope::Area,
        VariableScope::Graph,
    ];
    let locals: Vec<_> = categories()
        .into_iter()
        .enumerate()
        .map(|(i, value_type)| VarDecl {
            name: text.clone(),
            value_type,
            default: value.clone(),
            scope: scopes[i % scopes.len()].clone(),
        })
        .collect();
    let bodies = [
        NodeBody::Condition { expr: text.clone() },
        NodeBody::Action { call: call.clone() },
        NodeBody::Branch {
            expr: text.clone(),
            cases: vec![value.clone(), DataValue::Unsigned(ticks)],
        },
        NodeBody::Sequence {
            nodes: vec![id, Ulid::NIL, id],
        },
        NodeBody::Wait { ticks },
        NodeBody::CallScript {
            script_id: text.clone(),
            args: args.clone(),
        },
        NodeBody::CallGraph {
            graph_id: id,
            args: args.clone(),
        },
    ];
    for entry in triggers(ticks, &text) {
        result.push(Document::Graph(EventGraph {
            id,
            slug: text.clone(),
            name: text.clone(),
            note: Some(text.clone()),
            entry,
            start: id,
            nodes: bodies
                .iter()
                .cloned()
                .map(|body| Node { id, body })
                .collect(),
            edges: [
                Port::Next,
                Port::True,
                Port::False,
                Port::Case { index: u32::MAX },
            ]
            .into_iter()
            .map(|port| Edge {
                from: id,
                port,
                to: Ulid::NIL,
            })
            .collect(),
            locals: locals.clone(),
        }));
    }
    result.push(Document::Item(Item {
        id,
        slug: text.clone(),
        name: text.clone(),
        note: Some(text.clone()),
        stats: BTreeMap::from([(text.clone(), Fx16_16::from_raw(raw))]),
        tags: vec![text.clone(), String::new()],
    }));
    result.push(Document::Dialogue(Dialogue {
        id,
        slug: text.clone(),
        name: text.clone(),
        note: Some(text.clone()),
        entry: id,
        nodes: vec![
            DialogueBody::NpcLine {
                speaker: id,
                text_key: text.clone(),
                conditions: vec![text.clone()],
                on_enter: vec![call.clone()],
                next: None,
            },
            DialogueBody::NpcLine {
                speaker: id,
                text_key: text.clone(),
                conditions: vec![],
                on_enter: vec![],
                next: Some(id),
            },
            DialogueBody::PlayerChoice {
                text_key: text.clone(),
                conditions: vec![text.clone()],
                on_select: vec![call.clone()],
                next: id,
            },
            DialogueBody::Jump { target: id },
            DialogueBody::Link { target: id },
            DialogueBody::End,
        ]
        .into_iter()
        .map(|body| DialogueNode { id, body })
        .collect(),
    }));
    result.push(Document::Quest(Quest {
        id,
        slug: text.clone(),
        name: text.clone(),
        note: Some(text.clone()),
        entry: id,
        states: vec![
            QuestState {
                id,
                name: text.clone(),
                terminal: false,
                on_enter: vec![call],
                transitions: vec![QuestTransition {
                    condition: text.clone(),
                    target: id,
                }],
            },
            QuestState {
                id: Ulid::NIL,
                name: text.clone(),
                terminal: true,
                on_enter: vec![],
                transitions: vec![],
            },
        ],
    }));
    result.push(Document::Faction(Faction {
        id,
        slug: text.clone(),
        name: text.clone(),
        note: Some(text.clone()),
        relations: vec![FactionRelation {
            faction: id,
            disposition: raw,
        }],
    }));
    for document in &mut result {
        match document {
            Document::Campaign(v) => {
                v.note = Some(text.clone());
                v.version = "1.2.3-alpha.2+build.z".parse().unwrap();
                v.requires = [
                    PackageKind::Campaign,
                    PackageKind::Module,
                    PackageKind::Ruleset,
                ]
                .into_iter()
                .map(|kind| PackageRequirement {
                    kind,
                    package: "example.pkg".parse().unwrap(),
                    version: ">=1.2.3-alpha.1, <2.0.0".parse().unwrap(),
                })
                .collect();
            }
            Document::World(v) => {
                v.note = Some(text.clone());
                v.variables = locals.clone();
                v.areas = vec![id, Ulid::NIL];
            }
            Document::Area(v) => {
                v.note = Some(text.clone());
                v.ambience = Some(text.clone());
                v.bounds.min = [Fx16_16::from_raw(raw); 3];
                v.neighbours = vec![id];
            }
            Document::Creature(v) => {
                v.note = Some(text.clone());
                v.faction = Some(id);
                v.inventory = vec![id, Ulid::NIL];
            }
            Document::Placements(v) => {
                v.note = Some(text.clone());
                v.placements[0].overrides = args.clone();
                v.placements[0].note = Some(text.clone());
                v.placements[0].transform.rotation = [Fx16_16::from_raw(raw); 3];
            }
            Document::Triggers(v) => {
                v.note = Some(text.clone());
                v.graphs[0].locals = locals.clone();
            }
            Document::Variables(v) => {
                v.note = Some(text.clone());
                v.variables = locals.clone();
            }
            Document::Locale(v) => {
                v.note = Some(text.clone());
                v.strings.insert(text.clone(), text.clone());
            }
            Document::CampaignLock(v) => {
                v.note = Some(text.clone());
                v.packages = [
                    PackageKind::Campaign,
                    PackageKind::Module,
                    PackageKind::Ruleset,
                ]
                .into_iter()
                .enumerate()
                .map(|(i, kind)| ResolvedPackage {
                    kind,
                    package: format!("pkg-{i}").parse().unwrap(),
                    version: "1.2.3-rc.1+z".parse().unwrap(),
                    checksum: Digest::from_bytes([i as u8; 32]),
                })
                .collect();
            }
            Document::AssetsLock(v) => {
                v.note = Some(text.clone());
                v.assets.insert(
                    "assets/source.bin".parse().unwrap(),
                    AssetRecord {
                        hash: Digest::from_bytes([42; 32]),
                        import: args.clone(),
                    },
                );
            }
            _ => {}
        }
    }
    result
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn every_document_and_ir_variant_round_trips(
        chars in prop::collection::vec(any::<char>(), 0..24), value in value_strategy(),
        ticks in any::<u64>(), raw in any::<i32>(), id in any::<u128>()
    ) {
        let text: String = chars.into_iter().collect();
        for document in documents(text.clone(), value, ticks, raw, id) {
            let bytes = write_document(&document).unwrap();
            let back = read_document(&bytes).unwrap();
            prop_assert_eq!(&back, &document);
            prop_assert_eq!(write_document(&back).unwrap(), bytes);
        }
        let signature = ActionSignature { action_id: text.clone(), parameters: categories().into_iter().enumerate().map(|(i, value_type)| ActionParameter { name: text.clone(), value_type, required: i % 2 == 0 }).collect() };
        assert_value(&signature);
    }
    #[test]
    fn recursive_tagged_values_round_trip(value in value_strategy()) { assert_value(&value); }
}

#[test]
fn primitive_and_tick_boundaries_and_aliases() {
    for value in [
        DataValue::Integer(i64::MIN),
        DataValue::Integer(i64::MAX),
        DataValue::Unsigned(u64::MAX),
        DataValue::Fixed(Fx16_16::MIN),
        DataValue::Fixed(Fx16_16::MAX),
        DataValue::ObjectRef(Ulid::from_u128(u128::MAX)),
        DataValue::Map(BTreeMap::new()),
        DataValue::List(vec![]),
        DataValue::Bool(false),
        DataValue::Text("雪\n\t\"\\".into()),
    ] {
        assert_value(&value);
    }
    for ticks in [0, u64::MAX] {
        assert_value(&NodeBody::Wait { ticks });
        assert_value(&Trigger::Timer { ticks });
        for document in documents(
            "雪\n".into(),
            DataValue::Unsigned(ticks),
            ticks,
            i32::MIN,
            u128::MAX,
        ) {
            assert_eq!(
                read_document(&write_document(&document).unwrap()).unwrap(),
                document
            );
        }
    }
    let alias: DataValue =
        serde_json::from_value(json!({"type":"object_ref","value":"oooooooooooooooooooooooooL"}))
            .unwrap();
    assert_eq!(alias, DataValue::ObjectRef(Ulid::from_u128(1)));
    assert!(String::from_utf8(canonical_json(&alias).unwrap())
        .unwrap()
        .contains("00000000000000000000000001"));
    assert_eq!(
        canonical_json(&DataValue::Fixed(Fx16_16::from_raw(-123))).unwrap(),
        b"{\n  \"type\": \"fixed\",\n  \"value\": -123\n}\n"
    );
    let mut creature: serde_json::Value = serde_json::from_slice(
        &support::fixture_files()[&"creatures/creature.json".parse().unwrap()],
    )
    .unwrap();
    creature["id"] = json!("8ZZZZZZZZZZZZZZZZZZZZZZZZZ");
    assert!(matches!(
        read_document(&canonical_json(&creature).unwrap()),
        Err(DataError::Malformed { .. })
    ));
}

#[test]
fn required_nullable_and_unknown_fields_at_every_shape() {
    for bytes in support::fixture_files().values() {
        let value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        let mut unknown = value.clone();
        unknown["unknown"] = json!(true);
        assert!(matches!(
            read_document(&canonical_json(&unknown).unwrap()),
            Err(DataError::Malformed { .. })
        ));
        for key in value
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| k.as_str() != "_note")
        {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(key);
            assert!(
                matches!(
                    read_document(&canonical_json(&missing).unwrap()),
                    Err(DataError::Malformed { .. })
                ),
                "missing {key}"
            );
        }
    }
    assert!(serde_json::from_value::<DialogueBody>(
        json!({"kind":"npc_line","speaker":Ulid::NIL,"text_key":"x","conditions":[],"on_enter":[]})
    )
    .is_err());
    for value in [
        json!({"kind":"end","extra":0}),
        json!({"kind":"jump","target":Ulid::NIL,"extra":0}),
    ] {
        assert!(serde_json::from_value::<DialogueBody>(value).is_err());
    }
    assert!(serde_json::from_value::<Trigger>(json!({"kind":"area_enter","extra":0})).is_err());
    assert!(
        serde_json::from_value::<Trigger>(json!({"kind":"timer","ticks":1,"extra":0})).is_err()
    );
    assert!(serde_json::from_value::<Port>(json!({"kind":"next","extra":0})).is_err());
    assert!(serde_json::from_value::<Port>(json!({"kind":"case","index":0,"extra":0})).is_err());
    for mut value in [
        json!({"kind":"condition","expr":"x"}),
        json!({"kind":"action","call":{"action_id":"a","args":{}}}),
        json!({"kind":"branch","expr":"x","cases":[]}),
        json!({"kind":"sequence","nodes":[]}),
        json!({"kind":"wait","ticks":0}),
        json!({"kind":"call_script","script_id":"s","args":{}}),
        json!({"kind":"call_graph","graph_id":Ulid::NIL,"args":{}}),
    ] {
        value["extra"] = json!(0);
        assert!(serde_json::from_value::<NodeBody>(value).is_err());
    }
    assert!(
        serde_json::from_value::<DataValue>(json!({"type":"bool","value":true,"extra":0})).is_err()
    );
}

#[test]
fn old_and_new_item_bytes_converge_to_current() {
    let item = Item {
        id: Ulid::from_u128(8),
        slug: "item".into(),
        name: "fixture.creature".into(),
        note: Some("T012 migration fixture".into()),
        stats: BTreeMap::new(),
        tags: Vec::new(),
    };
    let current_bytes = write_document(&Document::Item(item.clone())).unwrap();
    let current_value: serde_json::Value = serde_json::from_slice(&current_bytes).unwrap();
    assert_eq!(current_value["schema"], json!("crpg.item/2"));
    let mut old_value = current_value.clone();
    old_value["schema"] = json!("crpg.item/1");
    let old_bytes = canonical_json(&old_value).unwrap();
    assert_eq!(
        read_document(&old_bytes).unwrap(),
        Document::Item(item.clone())
    );
    assert_eq!(read_document(&current_bytes).unwrap(), Document::Item(item));
    assert_eq!(
        write_document(&read_document(&old_bytes).unwrap()).unwrap(),
        current_bytes
    );
}
