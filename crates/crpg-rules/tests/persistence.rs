//! Symbolic persistence: exact bytes, strict decoding, duplicates,
//! rollback, normalization, and the cross-namespace round trip.

#[allow(dead_code)]
#[path = "common/mod.rs"]
mod common;

use crpg_core::{GenerationalArena, Interners};
use crpg_rules::{
    DiceExpr, EnumValue, ModifierPipeline, QueryContext, RulesErrorCode, SerializableStatBlock,
    SerializableStatEntry, SerializableStatValue, StackingPolicy, StatBlock, StatDefinition,
    StatValue, TagSet,
};
use proptest::prelude::*;

use common::{block, enum_kind, fixture, policies, raw_fx, tag_set, TYPE_A};

#[test]
fn exact_bytes_for_all_value_variants() {
    let fx = fixture();
    let stored = block(&[
        (fx.stats[0], StatValue::Int(-7)),
        (fx.stats[1], StatValue::Fixed(raw_fx(32768))),
        (fx.stats[2], StatValue::Bool(true)),
        (
            fx.stats[3],
            StatValue::Enum(EnumValue {
                enum_id: String::from("ed"),
                variant: String::from("ex"),
            }),
        ),
        (
            fx.stats[4],
            StatValue::Tags(tag_set(&[fx.tags[1], fx.tags[0]])),
        ),
    ]);
    let dto = stored.to_serializable(&fx.interners).unwrap();
    // Entries sort lexically by stat string; tags sort lexically as well.
    // Fixture order is alpha beta gamma delta epsilon, so lexical order is
    // alpha beta delta epsilon gamma.
    let json = serde_json::to_string(&dto).unwrap();
    assert_eq!(
        json,
        "{\"entries\":[\
         {\"stat\":\"alpha\",\"value\":{\"type\":\"int\",\"value\":-7}},\
         {\"stat\":\"beta\",\"value\":{\"type\":\"fixed\",\"value\":32768}},\
         {\"stat\":\"delta\",\"value\":{\"type\":\"enum\",\"value\":{\"enum_id\":\"ed\",\"variant\":\"ex\"}}},\
         {\"stat\":\"epsilon\",\"value\":{\"type\":\"tags\",\"value\":[\"ta\",\"tb\"]}},\
         {\"stat\":\"gamma\",\"value\":{\"type\":\"bool\",\"value\":true}}\
         ]}"
    );
}

#[test]
fn specification_example_bytes() {
    // The representative bytes pinned by the task contract.
    let mut interners = Interners::new();
    let alpha = interners.intern_stat("alpha");
    let beta = interners.intern_stat("beta");
    let stored = block(&[
        (beta, StatValue::Fixed(raw_fx(32768))),
        (alpha, StatValue::Int(7)),
    ]);
    let dto = stored.to_serializable(&interners).unwrap();
    assert_eq!(
        serde_json::to_string(&dto).unwrap(),
        "{\"entries\":[\
         {\"stat\":\"alpha\",\"value\":{\"type\":\"int\",\"value\":7}},\
         {\"stat\":\"beta\",\"value\":{\"type\":\"fixed\",\"value\":32768}}\
         ]}"
    );
}

#[test]
fn unknown_struct_fields_are_rejected() {
    let dto: Result<SerializableStatBlock, _> = serde_json::from_str(
        "{\"entries\":[{\"stat\":\"alpha\",\"value\":{\"type\":\"int\",\"value\":1}}],\"extra\":0}",
    );
    assert!(dto.is_err());
    let dto: Result<SerializableStatEntry, _> = serde_json::from_str(
        "{\"stat\":\"alpha\",\"value\":{\"type\":\"int\",\"value\":1},\"extra\":0}",
    );
    assert!(dto.is_err());
    let dto: Result<SerializableStatValue, _> =
        serde_json::from_str("{\"type\":\"int\",\"value\":1,\"extra\":0}");
    assert!(dto.is_err());
    let dto: Result<EnumValue, _> =
        serde_json::from_str("{\"enum_id\":\"ed\",\"variant\":\"ex\",\"extra\":0}");
    assert!(dto.is_err());
}

#[test]
fn duplicate_struct_fields_are_rejected() {
    let dto: Result<SerializableStatBlock, _> =
        serde_json::from_str("{\"entries\":[],\"entries\":[]}");
    assert!(dto.is_err());
    let dto: Result<SerializableStatEntry, _> = serde_json::from_str(
        "{\"stat\":\"alpha\",\"stat\":\"beta\",\"value\":{\"type\":\"int\",\"value\":1}}",
    );
    assert!(dto.is_err());
}

#[test]
fn duplicate_stat_entries_are_rejected_at_the_second_occurrence() {
    let dto = SerializableStatBlock {
        entries: vec![
            SerializableStatEntry {
                stat: String::from("beta"),
                value: SerializableStatValue::Int(1),
            },
            SerializableStatEntry {
                stat: String::from("alpha"),
                value: SerializableStatValue::Int(2),
            },
            SerializableStatEntry {
                stat: String::from("beta"),
                value: SerializableStatValue::Int(3),
            },
        ],
    };
    let mut interners = Interners::new();
    let before = interners.clone();
    let error = StatBlock::from_serializable(dto, &mut interners).unwrap_err();
    assert_eq!(error.code, crpg_rules::RulesErrorCode::DuplicateStat);
    assert_eq!(error.location, "/persisted/entries/2");
    assert_eq!(interners, before);
}

#[test]
fn duplicate_tags_are_rejected_at_the_second_occurrence() {
    let dto = SerializableStatBlock {
        entries: vec![SerializableStatEntry {
            stat: String::from("alpha"),
            value: SerializableStatValue::Tags(vec![
                String::from("ta"),
                String::from("tb"),
                String::from("ta"),
            ]),
        }],
    };
    let mut interners = Interners::new();
    let before = interners.clone();
    let error = StatBlock::from_serializable(dto, &mut interners).unwrap_err();
    assert_eq!(error.code, crpg_rules::RulesErrorCode::DuplicateTag);
    assert_eq!(error.location, "/persisted/entries/0/value/2");
    assert_eq!(interners, before);
}

#[test]
fn empty_symbols_are_rejected() {
    let mut interners = Interners::new();
    for dto in [
        SerializableStatBlock {
            entries: vec![SerializableStatEntry {
                stat: String::new(),
                value: SerializableStatValue::Int(0),
            }],
        },
        SerializableStatBlock {
            entries: vec![SerializableStatEntry {
                stat: String::from("alpha"),
                value: SerializableStatValue::Tags(vec![String::new()]),
            }],
        },
        SerializableStatBlock {
            entries: vec![SerializableStatEntry {
                stat: String::from("alpha"),
                value: SerializableStatValue::Enum(EnumValue {
                    enum_id: String::new(),
                    variant: String::from("ex"),
                }),
            }],
        },
    ] {
        let before = interners.clone();
        let error = StatBlock::from_serializable(dto, &mut interners).unwrap_err();
        assert_eq!(error.code, crpg_rules::RulesErrorCode::InvalidName);
        assert_eq!(interners, before);
    }
}

#[test]
fn persisted_entry_count_has_a_boundary() {
    let mut entries = Vec::new();
    for index in 0..1024 {
        entries.push(SerializableStatEntry {
            stat: format!("slot-{index:04}"),
            value: SerializableStatValue::Int(index),
        });
    }
    let mut interners = Interners::new();
    let dto = SerializableStatBlock { entries };
    assert!(StatBlock::from_serializable(dto, &mut interners).is_ok());
    let mut entries = Vec::new();
    for index in 0..1025 {
        entries.push(SerializableStatEntry {
            stat: format!("slot-{index:04}"),
            value: SerializableStatValue::Int(0),
        });
    }
    let mut interners = Interners::new();
    let before = interners.clone();
    let error = StatBlock::from_serializable(SerializableStatBlock { entries }, &mut interners)
        .unwrap_err();
    assert_eq!(error.code, crpg_rules::RulesErrorCode::LimitExceeded);
    assert_eq!(error.location, "/persisted/entries");
    assert_eq!(interners, before);
}

#[test]
fn unresolved_symbols_fail_without_mutating_the_block_path() {
    let fx = fixture();
    let stored = block(&[(fx.stats[0], StatValue::Int(1))]);
    // An empty foreign interner holds no handle for alpha at all.
    let foreign = Interners::new();
    let error = stored.to_serializable(&foreign).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::UnresolvedHandle);
    assert_eq!(error.location, format!("/stats/{}", fx.stats[0].index()));
    // An empty foreign tag namespace cannot resolve the stored tag either.
    let foreign = Interners::new();
    let tagged = block(&[(fx.stats[0], StatValue::Tags(tag_set(&fx.tags[..1])))]);
    let error = tagged.to_serializable(&foreign).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::UnresolvedHandle);
}

#[test]
fn unresolved_stat_keys_report_their_index() {
    let mut interners = Interners::new();
    let alpha = interners.intern_stat("alpha");
    let beta = interners.intern_stat("beta");
    let stored = block(&[(alpha, StatValue::Int(1)), (beta, StatValue::Int(2))]);
    let mut foreign = Interners::new();
    foreign.intern_stat("alpha");
    let error = stored.to_serializable(&foreign).unwrap_err();
    assert_eq!(error.code, crpg_rules::RulesErrorCode::UnresolvedHandle);
    assert_eq!(error.location, format!("/stats/{}", beta.index()));
}

#[test]
fn enum_strings_pass_standalone_but_fail_domain_checks() {
    // Standalone conversion accepts enum strings no pipeline declares; the
    // pipeline rejects the unknown domain at query time.
    let dto = SerializableStatBlock {
        entries: vec![SerializableStatEntry {
            stat: String::from("alpha"),
            value: SerializableStatValue::Enum(EnumValue {
                enum_id: String::from("undeclared"),
                variant: String::from("ex"),
            }),
        }],
    };
    let mut interners = Interners::new();
    let stored = StatBlock::from_serializable(dto, &mut interners).unwrap();
    let stat = interners.stat("alpha").unwrap();
    let pipeline = ModifierPipeline::new(
        vec![StatDefinition {
            id: stat,
            kind: enum_kind("ed", &["ex"]),
            derived: None,
        }],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let mut arena = GenerationalArena::new();
    let entity = arena.insert(());
    let tags = TagSet::new();
    let mods: Vec<crpg_rules::Modifier> = vec![];
    let ctx = QueryContext {
        entity,
        stats: &stored,
        tags: &tags,
        modifiers: &mods,
    };
    let error = pipeline.query(entity, stat, &ctx).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidEnum);
    assert_eq!(error.location, format!("/stats/{}", stat.index()));
}

#[test]
fn failed_loads_leave_the_interner_unchanged() {
    let mut interners = Interners::new();
    interners.intern_stat("alpha");
    interners.intern_tag("ta");
    let before = interners.clone();
    // Duplicate entry, empty name, oversized tags, and unresolved shapes all
    // fail before any interner mutation.
    let duplicate = SerializableStatBlock {
        entries: vec![
            SerializableStatEntry {
                stat: String::from("alpha"),
                value: SerializableStatValue::Int(1),
            },
            SerializableStatEntry {
                stat: String::from("alpha"),
                value: SerializableStatValue::Int(2),
            },
        ],
    };
    assert!(StatBlock::from_serializable(duplicate, &mut interners).is_err());
    assert_eq!(interners, before);
    let fresh = SerializableStatBlock {
        entries: vec![SerializableStatEntry {
            stat: String::from("brand-new-stat"),
            value: SerializableStatValue::Tags(vec![String::from("ta"), String::from("ta")]),
        }],
    };
    assert!(StatBlock::from_serializable(fresh, &mut interners).is_err());
    assert_eq!(interners, before);
}

#[test]
fn unsorted_input_normalizes_to_lexical_order() {
    let dto = SerializableStatBlock {
        entries: vec![
            SerializableStatEntry {
                stat: String::from("gamma"),
                value: SerializableStatValue::Tags(vec![String::from("tb"), String::from("ta")]),
            },
            SerializableStatEntry {
                stat: String::from("alpha"),
                value: SerializableStatValue::Int(7),
            },
        ],
    };
    let mut interners = Interners::new();
    let stored = StatBlock::from_serializable(dto, &mut interners).unwrap();
    let round = stored.to_serializable(&interners).unwrap();
    let json = serde_json::to_string(&round).unwrap();
    assert_eq!(
        json,
        "{\"entries\":[\
         {\"stat\":\"alpha\",\"value\":{\"type\":\"int\",\"value\":7}},\
         {\"stat\":\"gamma\",\"value\":{\"type\":\"tags\",\"value\":[\"ta\",\"tb\"]}}\
         ]}"
    );
}

#[test]
fn handle_assignments_actually_differ_across_namespaces() {
    // Two interners with different prepopulation assign different handles to
    // the same strings; the round trip compares meanings, never handles.
    let mut first = Interners::new();
    first.intern_stat("zeta");
    first.intern_stat("alpha");
    first.intern_tag("tb");
    first.intern_tag("ta");
    let mut second = Interners::new();
    second.intern_stat("alpha");
    second.intern_tag("ta");
    let alpha_first = first.stat("alpha").unwrap();
    let alpha_second = second.stat("alpha").unwrap();
    assert_ne!(alpha_first, alpha_second);
    assert_eq!(first.resolve_stat(alpha_first), Some("alpha"));
    assert_eq!(second.resolve_stat(alpha_second), Some("alpha"));
    let stored = block(&[
        (alpha_first, StatValue::Int(7)),
        (
            first.stat("zeta").unwrap(),
            StatValue::Tags(tag_set(&[first.tag("tb").unwrap()])),
        ),
    ]);
    let dto = stored.to_serializable(&first).unwrap();
    let restored = StatBlock::from_serializable(dto, &mut second).unwrap();
    let round = restored.to_serializable(&second).unwrap();
    let json = serde_json::to_string(&round).unwrap();
    assert_eq!(
        json,
        "{\"entries\":[\
         {\"stat\":\"alpha\",\"value\":{\"type\":\"int\",\"value\":7}},\
         {\"stat\":\"zeta\",\"value\":{\"type\":\"tags\",\"value\":[\"tb\"]}}\
         ]}"
    );
}

// ---------------------------------------------------------------------------
// Symbolic round-trip property into differently prepopulated interners.
// ---------------------------------------------------------------------------

fn arbitrary_value() -> impl Strategy<Value = SerializableStatValue> {
    prop_oneof![
        any::<i32>().prop_map(SerializableStatValue::Int),
        any::<i32>().prop_map(|raw| SerializableStatValue::Fixed(raw_fx(raw))),
        any::<bool>().prop_map(SerializableStatValue::Bool),
        (1..4usize).prop_map(|variant| SerializableStatValue::Enum(EnumValue {
            enum_id: String::from("ed"),
            variant: format!("e{variant}"),
        })),
        prop::collection::btree_set(0..6usize, 0..4).prop_map(|picked| {
            SerializableStatValue::Tags(
                picked
                    .into_iter()
                    .map(|tag| format!("ptag-{tag}"))
                    .collect(),
            )
        }),
    ]
}

fn arbitrary_block() -> impl Strategy<Value = SerializableStatBlock> {
    // Unique stat indices by construction, so the converter accepts every
    // generated block without filtering.
    prop::collection::btree_set(0..8usize, 0..8).prop_flat_map(|picked| {
        let entries: Vec<_> = picked
            .into_iter()
            .map(|index| (Just(format!("pool-{index}")), arbitrary_value()))
            .collect();
        entries.prop_map(|pairs| SerializableStatBlock {
            entries: pairs
                .into_iter()
                .map(|(stat, value)| SerializableStatEntry { stat, value })
                .collect(),
        })
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, ..ProptestConfig::default() })]

    #[test]
    fn symbolic_round_trip_preserves_meaning_across_namespaces(dto in arbitrary_block()) {
        // Inputs are valid by construction: unique stat names and tags.
        // Two differently prepopulated interners: meanings must survive.
        let mut first = Interners::new();
        first.intern_stat("unrelated-first");
        first.intern_tag("unrelated-first-tag");
        let mut second = Interners::new();
        second.intern_stat("unrelated-second");
        second.intern_stat("unrelated-second-again");
        second.intern_tag("unrelated-second-tag");
        let stored = StatBlock::from_serializable(dto, &mut first).unwrap();
        let wire = serde_json::to_string(&stored.to_serializable(&first).unwrap()).unwrap();
        let restored = StatBlock::from_serializable(
            serde_json::from_str(&wire).unwrap(),
            &mut second,
        )
        .unwrap();
        let rewired = serde_json::to_string(&restored.to_serializable(&second).unwrap()).unwrap();
        prop_assert_eq!(rewired, wire);
    }
}

#[test]
fn dice_values_round_trip_as_canonical_strings() {
    let fx = fixture();
    let stored = block(&[
        (fx.stats[0], StatValue::Dice("2d6+3".parse().unwrap())),
        (fx.stats[1], StatValue::Int(1)),
    ]);
    let dto = stored.to_serializable(&fx.interners).unwrap();
    assert_eq!(
        serde_json::to_string(&dto).unwrap(),
        "{\"entries\":[\
         {\"stat\":\"alpha\",\"value\":{\"type\":\"dice\",\"value\":\"2d6+3\"}},\
         {\"stat\":\"beta\",\"value\":{\"type\":\"int\",\"value\":1}}\
         ]}"
    );
    // Restoration into a differently ordered interner keeps the meaning
    // while the numeric handles actually differ.
    let mut second = Interners::new();
    second.intern_stat("beta");
    second.intern_stat("zeta");
    second.intern_stat("alpha");
    assert_ne!(second.stat("alpha"), fx.interners.stat("alpha"));
    let restored = StatBlock::from_serializable(dto, &mut second).unwrap();
    assert_eq!(
        restored.get(second.stat("alpha").unwrap()),
        Some(&StatValue::Dice("2d6+3".parse::<DiceExpr>().unwrap()))
    );
    assert_eq!(
        restored.get(second.stat("beta").unwrap()),
        Some(&StatValue::Int(1))
    );
    let rewired = restored.to_serializable(&second).unwrap();
    let fresh = fixture();
    let expected = stored.to_serializable(&fresh.interners).unwrap();
    assert_eq!(rewired, expected);
}

#[test]
fn failed_dice_decoding_interns_nothing() {
    // An otherwise-valid preceding entry is not interned when a later dice
    // value fails; parser paths carry the persisted entry prefix.
    let dto = SerializableStatBlock {
        entries: vec![
            SerializableStatEntry {
                stat: String::from("alpha"),
                value: SerializableStatValue::Dice(String::from("2d6+3")),
            },
            SerializableStatEntry {
                stat: String::from("beta"),
                value: SerializableStatValue::Dice(String::from("2d")),
            },
        ],
    };
    let mut interners = Interners::new();
    let before = interners.clone();
    let error = StatBlock::from_serializable(dto, &mut interners).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidDice);
    assert_eq!(error.location, "/persisted/entries/1/dice/input/2");
    assert_eq!(interners, before);
    assert!(interners.stat("alpha").is_none());
    // An over-limit dice string fails preflight the same way.
    let dto = SerializableStatBlock {
        entries: vec![SerializableStatEntry {
            stat: String::from("alpha"),
            value: SerializableStatValue::Dice(format!("1d{}", "7".repeat(127))),
        }],
    };
    let mut interners = Interners::new();
    let error = StatBlock::from_serializable(dto, &mut interners).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::LimitExceeded);
    assert_eq!(error.location, "/persisted/entries/0/dice/input");
    assert!(interners.stat("alpha").is_none());
}
