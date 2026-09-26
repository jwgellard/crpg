//! `StatBlock` behaviour: ordering, equality, standalone validation, limits.

#[allow(dead_code)]
#[path = "common/mod.rs"]
mod common;

use crpg_core::{Fx16_16, Interners};
use crpg_rules::{EnumValue, RulesErrorCode, StatBlock, StatValue};

use common::{block, fixture, raw_fx};

#[test]
fn new_block_is_empty() {
    let block = StatBlock::new();
    assert!(block.is_empty());
    assert_eq!(block.len(), 0);
    assert_eq!(block.iter().count(), 0);
    assert_eq!(block, StatBlock::default());
}

#[test]
fn all_five_kinds_round_trip_through_get() {
    let fixed = fixture();
    let pairs = [
        (fixed.stats[0], StatValue::Int(-7)),
        (fixed.stats[1], StatValue::Fixed(raw_fx(98304))),
        (fixed.stats[2], StatValue::Bool(true)),
        (fixed.stats[3], common::enum_value("edomain", "emember")),
        (
            fixed.stats[4],
            StatValue::Tags(common::tag_set(&fixed.tags[..2])),
        ),
    ];
    let stored = block(&pairs);
    assert_eq!(stored.len(), pairs.len());
    for (stat, expected) in &pairs {
        assert_eq!(stored.get(*stat), Some(expected));
    }
}

#[test]
fn insert_preserves_insertion_order() {
    let fixed = fixture();
    let stored = block(&[
        (fixed.stats[2], StatValue::Int(2)),
        (fixed.stats[0], StatValue::Int(0)),
        (fixed.stats[1], StatValue::Int(1)),
    ]);
    let order: Vec<crpg_core::StatId> = stored.iter().map(|(stat, _)| stat).collect();
    assert_eq!(order, vec![fixed.stats[2], fixed.stats[0], fixed.stats[1]]);
}

#[test]
fn replacement_retains_its_slot() {
    let fixed = fixture();
    let mut stored = block(&[
        (fixed.stats[0], StatValue::Int(0)),
        (fixed.stats[1], StatValue::Int(1)),
        (fixed.stats[2], StatValue::Int(2)),
    ]);
    let previous = stored.insert(fixed.stats[1], StatValue::Int(11));
    assert_eq!(previous, Ok(Some(StatValue::Int(1))));
    let order: Vec<(crpg_core::StatId, StatValue)> = stored
        .iter()
        .map(|(stat, value)| (stat, value.clone()))
        .collect();
    assert_eq!(
        order,
        vec![
            (fixed.stats[0], StatValue::Int(0)),
            (fixed.stats[1], StatValue::Int(11)),
            (fixed.stats[2], StatValue::Int(2)),
        ]
    );
}

#[test]
fn removal_preserves_remaining_order_and_reinsertion_appends() {
    let fixed = fixture();
    let mut stored = block(&[
        (fixed.stats[0], StatValue::Int(0)),
        (fixed.stats[1], StatValue::Int(1)),
        (fixed.stats[2], StatValue::Int(2)),
    ]);
    assert_eq!(stored.remove(fixed.stats[1]), Some(StatValue::Int(1)));
    assert_eq!(stored.remove(fixed.stats[1]), None);
    let order: Vec<crpg_core::StatId> = stored.iter().map(|(stat, _)| stat).collect();
    assert_eq!(order, vec![fixed.stats[0], fixed.stats[2]]);
    assert_eq!(stored.insert(fixed.stats[1], StatValue::Int(9)), Ok(None));
    let order: Vec<crpg_core::StatId> = stored.iter().map(|(stat, _)| stat).collect();
    assert_eq!(order, vec![fixed.stats[0], fixed.stats[2], fixed.stats[1]]);
}

#[test]
fn equality_ignores_insertion_order() {
    let fixed = fixture();
    let first = block(&[
        (fixed.stats[0], StatValue::Int(0)),
        (fixed.stats[1], StatValue::Bool(true)),
        (fixed.stats[2], StatValue::Fixed(raw_fx(1))),
    ]);
    let second = block(&[
        (fixed.stats[2], StatValue::Fixed(raw_fx(1))),
        (fixed.stats[0], StatValue::Int(0)),
        (fixed.stats[1], StatValue::Bool(true)),
    ]);
    assert_eq!(first, second);
    let third = block(&[
        (fixed.stats[0], StatValue::Int(0)),
        (fixed.stats[1], StatValue::Bool(false)),
        (fixed.stats[2], StatValue::Fixed(raw_fx(1))),
    ]);
    assert_ne!(first, third);
    let shorter = block(&[
        (fixed.stats[0], StatValue::Int(0)),
        (fixed.stats[1], StatValue::Bool(true)),
    ]);
    assert_ne!(first, shorter);
}

#[test]
fn empty_enum_strings_are_invalid_names() {
    let fixed = fixture();
    let mut stored = StatBlock::new();
    let error = stored
        .insert(
            fixed.stats[0],
            StatValue::Enum(EnumValue {
                enum_id: String::new(),
                variant: String::from("v"),
            }),
        )
        .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidName);
    assert_eq!(error.location, format!("/stats/{}", fixed.stats[0].index()));
    let error = stored
        .insert(
            fixed.stats[0],
            StatValue::Enum(EnumValue {
                enum_id: String::from("e"),
                variant: String::new(),
            }),
        )
        .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidName);
    assert!(stored.is_empty());
}

#[test]
fn oversized_tag_sets_are_rejected() {
    let mut interners = Interners::new();
    let stat = interners.intern_stat("alpha");
    let mut tags = Vec::new();
    for index in 0..1025 {
        tags.push(interners.intern_tag(&format!("overflow-tag-{index:04}")));
    }
    let oversized: std::collections::BTreeSet<crpg_core::TagId> = tags.into_iter().collect();
    assert_eq!(oversized.len(), 1025);
    let mut stored = StatBlock::new();
    let before = stored.clone();
    let error = stored.insert(stat, StatValue::Tags(oversized)).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::LimitExceeded);
    assert_eq!(stored, before);
}

#[test]
fn invalid_insert_leaves_the_block_unchanged() {
    let fixed = fixture();
    let mut stored = block(&[(fixed.stats[0], StatValue::Int(3))]);
    let before = stored.clone();
    let error = stored
        .insert(
            fixed.stats[1],
            StatValue::Enum(EnumValue {
                enum_id: String::new(),
                variant: String::from("v"),
            }),
        )
        .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidName);
    assert_eq!(stored, before);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored.get(fixed.stats[0]), Some(&StatValue::Int(3)));
}

#[test]
fn block_holds_full_capacity_and_replacement_still_succeeds() {
    let mut interners = Interners::new();
    let mut ids = Vec::new();
    for index in 0..1024 {
        ids.push(interners.intern_stat(&format!("slot-{index:04}")));
    }
    let mut stored = StatBlock::new();
    for (position, stat) in ids.iter().enumerate() {
        assert_eq!(
            stored.insert(*stat, StatValue::Int(position as i32)),
            Ok(None)
        );
    }
    assert_eq!(stored.len(), 1024);
    assert!(!stored.is_empty());
    // Replacement at full capacity keeps its slot and succeeds.
    assert_eq!(
        stored.insert(ids[7], StatValue::Int(-7)),
        Ok(Some(StatValue::Int(7)))
    );
    assert_eq!(stored.len(), 1024);
    assert_eq!(stored.get(ids[7]), Some(&StatValue::Int(-7)));
    // A new key past capacity fails at the collection root.
    let extra = interners.intern_stat("slot-extra");
    let before = stored.clone();
    let error = stored.insert(extra, StatValue::Int(0)).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::LimitExceeded);
    assert_eq!(error.location, String::from("/stats"));
    assert_eq!(stored, before);
}

#[test]
fn fixed_zero_and_extremes_store_exactly() {
    let fixed = fixture();
    let stored = block(&[
        (fixed.stats[0], StatValue::Fixed(Fx16_16::ZERO)),
        (fixed.stats[1], StatValue::Fixed(Fx16_16::MIN)),
        (fixed.stats[2], StatValue::Fixed(Fx16_16::MAX)),
    ]);
    assert_eq!(
        stored.get(fixed.stats[0]),
        Some(&StatValue::Fixed(Fx16_16::ZERO))
    );
    assert_eq!(
        stored.get(fixed.stats[1]),
        Some(&StatValue::Fixed(Fx16_16::MIN))
    );
    assert_eq!(
        stored.get(fixed.stats[2]),
        Some(&StatValue::Fixed(Fx16_16::MAX))
    );
}

#[test]
fn error_display_is_code_at_location() {
    let fixed = fixture();
    let mut stored = StatBlock::new();
    let error = stored
        .insert(
            fixed.stats[3],
            StatValue::Enum(EnumValue {
                enum_id: String::from("e"),
                variant: String::new(),
            }),
        )
        .unwrap_err();
    assert!(error.cycle.is_empty());
    assert_eq!(
        format!("{error}"),
        format!("InvalidName at /stats/{}", fixed.stats[3].index())
    );
}
