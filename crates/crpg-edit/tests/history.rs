//! T058 §11 `tests/history.rs`: byte-exact undo and redo, redo invalidation,
//! revision counting and both history bounds.

mod support;

use crpg_data::{Document, Item, SourcePath};
use crpg_edit::{
    CampaignDocument, ChangeKind, CommandReceipt, EditCommand, EditError, EditTarget,
    MAX_HISTORY_BYTES, MAX_UNDO_ENTRIES,
};
use std::collections::{BTreeMap, VecDeque};
use support::*;

/// About 3.5 MiB of note text, so a handful of documents fill the history cap.
const BIG_NOTE_BYTES: usize = 3_670_016;

fn locale_string(key: &str, text: &str) -> EditCommand {
    EditCommand::SetValue {
        target: EditTarget::Document(sp("locale/en.json")),
        pointer: format!("/strings/{key}"),
        value: json_str(text),
    }
}

fn creature_slug(slug: &str) -> EditCommand {
    EditCommand::SetValue {
        target: EditTarget::Object(id(4)),
        pointer: "/slug".to_owned(),
        value: json_str(slug),
    }
}

fn item(n: u128, note: Option<String>) -> Document {
    Document::Item(Item {
        id: id(n),
        slug: format!("item-{n}"),
        name: "fixture.creature".to_owned(),
        note,
        stats: BTreeMap::new(),
        tags: Vec::new(),
    })
}

fn big_item(n: u128) -> Document {
    item(n, Some("b".repeat(BIG_NOTE_BYTES)))
}

fn item_path(n: u128) -> SourcePath {
    sp(&format!("items/i{n}.json"))
}

/// §6.3 entry size, computed from the maps before and after a receipt.
fn entry_bytes(
    before: &BTreeMap<SourcePath, Vec<u8>>,
    after: &BTreeMap<SourcePath, Vec<u8>>,
    receipt: &CommandReceipt,
) -> usize {
    receipt
        .changes
        .iter()
        .map(|c| before.get(&c.path).map_or(0, Vec::len) + after.get(&c.path).map_or(0, Vec::len))
        .sum()
}

#[test]
fn undo_restores_exact_bytes_and_redo_reapplies() {
    let mut doc = open();
    let commands = vec![
        creature_slug("goblin"),
        EditCommand::CreateDocument {
            path: item_path(20),
            document: item(20, None),
        },
        EditCommand::DeleteDocument {
            path: sp("areas/start/triggers.json"),
        },
        EditCommand::RenameDocument {
            from: sp("creatures/creature.json"),
            to: sp("creatures/goblin.json"),
        },
        locale_string("mvp.npc", "Mayor"),
    ];
    let mut states = vec![doc.canonical_files().clone()];
    for command in commands {
        doc.apply(command).unwrap();
        states.push(doc.canonical_files().clone());
    }
    for expected in states.iter().rev().skip(1) {
        let receipt = doc.undo().unwrap();
        assert_eq!(doc.canonical_files(), expected);
        assert_eq!(receipt.diagnostics, crpg_data::validate(doc.campaign()));
    }
    assert_eq!(doc.campaign(), &fixture());
    for expected in states.iter().skip(1) {
        doc.redo().unwrap();
        assert_eq!(doc.canonical_files(), expected);
    }
}

#[test]
fn undo_receipt_swaps_created_and_deleted() {
    let mut doc = open();
    let receipt = doc
        .apply(EditCommand::RenameDocument {
            from: sp("creatures/creature.json"),
            to: sp("creatures/goblin.json"),
        })
        .unwrap();
    assert_eq!(
        receipt.changes,
        vec![
            change("creatures/creature.json", ChangeKind::Deleted),
            change("creatures/goblin.json", ChangeKind::Created),
        ]
    );
    let undone = doc.undo().unwrap();
    assert_eq!(
        undone.changes,
        vec![
            change("creatures/creature.json", ChangeKind::Created),
            change("creatures/goblin.json", ChangeKind::Deleted),
        ]
    );
    let redone = doc.redo().unwrap();
    assert_eq!(redone.changes, receipt.changes);
    assert_eq!(redone.revision, 3);
}

#[test]
fn empty_stacks_report_nothing_to_do() {
    let mut doc = open();
    let before = snapshot(&doc);
    assert_eq!(doc.undo(), Err(EditError::NothingToUndo));
    assert_unchanged(&before, &doc);
    assert_eq!(doc.redo(), Err(EditError::NothingToRedo));
    assert_unchanged(&before, &doc);

    // Positive controls, then the stacks are empty again on the other side.
    doc.apply(creature_slug("goblin")).unwrap();
    doc.undo().unwrap();
    let before = snapshot(&doc);
    assert_eq!(doc.undo(), Err(EditError::NothingToUndo));
    assert_unchanged(&before, &doc);
    doc.redo().unwrap();
    let before = snapshot(&doc);
    assert_eq!(doc.redo(), Err(EditError::NothingToRedo));
    assert_unchanged(&before, &doc);
}

#[test]
fn committed_apply_clears_redo() {
    let mut doc = open();
    doc.apply(creature_slug("a")).unwrap();
    doc.apply(creature_slug("b")).unwrap();
    doc.undo().unwrap();
    assert_eq!((doc.undo_depth(), doc.redo_depth()), (1, 1));
    doc.apply(locale_string("k", "v")).unwrap();
    assert_eq!((doc.undo_depth(), doc.redo_depth()), (2, 0));
    assert_eq!(doc.redo(), Err(EditError::NothingToRedo));
}

#[test]
fn failed_and_noop_applies_keep_both_stacks() {
    let mut doc = open();
    doc.apply(creature_slug("a")).unwrap();
    doc.apply(creature_slug("b")).unwrap();
    doc.undo().unwrap();
    let before = snapshot(&doc);
    doc.apply(EditCommand::DeleteObject { id: id(99) })
        .unwrap_err();
    assert_unchanged(&before, &doc);
    let receipt = doc.apply(creature_slug("a")).unwrap();
    assert!(receipt.changes.is_empty());
    assert_unchanged(&before, &doc);
    doc.redo().unwrap();
    assert_eq!((doc.undo_depth(), doc.redo_depth()), (2, 0));
}

#[test]
fn revision_counts_commits_only() {
    let mut doc = open();
    assert_eq!(doc.apply(creature_slug("a")).unwrap().revision, 1);
    assert_eq!(doc.apply(creature_slug("b")).unwrap().revision, 2);
    assert_eq!(doc.undo().unwrap().revision, 3);
    assert_eq!(doc.redo().unwrap().revision, 4);
    doc.apply(EditCommand::DeleteObject { id: id(99) })
        .unwrap_err();
    assert_eq!(doc.revision(), 4);
    assert_eq!(doc.apply(creature_slug("b")).unwrap().revision, 4);
    doc.redo().unwrap_err();
    let plan = doc.save_plan();
    doc.mark_saved(&plan).unwrap();
    assert_eq!(doc.revision(), 4);
    assert_eq!(doc.undo().unwrap().revision, 5);
}

#[test]
fn undo_depth_evicts_oldest() {
    let mut doc = open();
    doc.apply(locale_string("k0", "v")).unwrap();
    let after_first = doc.canonical_files().clone();
    for i in 1..=MAX_UNDO_ENTRIES {
        doc.apply(locale_string(&format!("k{i}"), "v")).unwrap();
    }
    assert_eq!(doc.undo_depth(), MAX_UNDO_ENTRIES);
    for _ in 0..MAX_UNDO_ENTRIES {
        doc.undo().unwrap();
    }
    assert_eq!(doc.canonical_files(), &after_first);
    let before = snapshot(&doc);
    assert_eq!(doc.undo(), Err(EditError::NothingToUndo));
    assert_unchanged(&before, &doc);
    assert_eq!(doc.redo_depth(), MAX_UNDO_ENTRIES);
}

#[test]
fn history_bytes_evicts_oldest() {
    let mut doc = open();
    let mut model: VecDeque<usize> = VecDeque::new();
    let mut evicted = 0;
    for n in 0..10_u128 {
        let before = doc.canonical_files().clone();
        let receipt = doc
            .apply(EditCommand::CreateDocument {
                path: item_path(100 + n),
                document: big_item(100 + n),
            })
            .unwrap();
        let size = entry_bytes(&before, doc.canonical_files(), &receipt);
        assert!(size > 3 * 1_048_576 && size <= 4 * 1_048_576);
        model.push_back(size);
        while model.len() > MAX_UNDO_ENTRIES || model.iter().sum::<usize>() > MAX_HISTORY_BYTES {
            model.pop_front();
            evicted += 1;
        }
        assert_eq!(doc.history_bytes(), model.iter().sum::<usize>());
        assert_eq!(doc.undo_depth(), model.len());
        assert!(doc.history_bytes() <= MAX_HISTORY_BYTES);
    }
    assert!(evicted > 0, "the cap evicted at least one entry");
    // Undo and redo move entries without changing the measure.
    let total = doc.history_bytes();
    doc.undo().unwrap();
    assert_eq!(doc.history_bytes(), total);
    assert!(!doc.canonical_files().contains_key(&item_path(109)));
    doc.redo().unwrap();
    assert_eq!(doc.history_bytes(), total);
    // The newest entry is always kept: every apply can be undone once.
    for _ in 0..doc.undo_depth() {
        doc.undo().unwrap();
    }
    assert_eq!(doc.undo(), Err(EditError::NothingToUndo));
    assert!(doc
        .canonical_files()
        .contains_key(&item_path(100 + evicted - 1)));
    assert!(!doc
        .canonical_files()
        .contains_key(&item_path(100 + evicted)));
}

#[test]
fn oversized_change_is_refused() {
    let mut campaign = fixture();
    for n in 0..5_u128 {
        campaign
            .documents
            .insert(item_path(200 + n), big_item(200 + n));
    }
    let mut doc = CampaignDocument::open(campaign).unwrap();
    let rename = |n: u128| EditCommand::SetValue {
        target: EditTarget::Object(id(200 + n)),
        pointer: "/slug".to_owned(),
        value: json_str(&format!("renamed-{n}")),
    };
    let before = snapshot(&doc);
    match doc.apply_batch((0..5).map(rename).collect()) {
        Err(EditError::HistoryTooLarge { bytes }) => assert!(bytes > MAX_HISTORY_BYTES),
        other => panic!("expected HistoryTooLarge, got {other:?}"),
    }
    assert_unchanged(&before, &doc);
    // Positive control: four of the five fit under the cap.
    let receipt = doc.apply_batch((0..4).map(rename).collect()).unwrap();
    assert_eq!(receipt.changes.len(), 4);
    assert!(doc.history_bytes() <= MAX_HISTORY_BYTES);
}
