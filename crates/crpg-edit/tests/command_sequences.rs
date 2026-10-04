//! T058 §11 `tests/command_sequences.rs` (spec §19.2 #15): random command
//! sequences round-trip through undo, redo, replay and save, and undo/redo
//! interleavings match a snapshot model.
//!
//! Generated ops are resolved against the current state when executed, so
//! sequences stay meaningful as the campaign changes. The runner uses a fixed
//! seed, so every run generates the same cases.

mod support;

use crpg_core::Ulid;
use crpg_data::{Document, Item, ObjectKind, Placement, SourcePath};
use crpg_edit::{CampaignDocument, CommandReceipt, EditCommand, EditError, EditTarget};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use std::collections::BTreeMap;
use support::*;

/// Fixed so the generated cases are reproducible from run to run.
const SEED: u64 = 0x0058_0058_0058_0058;

fn config() -> ProptestConfig {
    ProptestConfig {
        rng_seed: RngSeed::Fixed(SEED),
        ..ProptestConfig::with_cases(256)
    }
}

/// One generated operation; `usize` fields pick from the current lists.
#[derive(Debug, Clone)]
enum Op {
    SetSlug { pick: usize, seed: u8 },
    SetNote { pick: usize, seed: u8 },
    RemoveNote { pick: usize },
    AddTag { pick: usize, seed: u8 },
    RemoveFirstTag { pick: usize },
    Place { seed: u8, area: usize },
    Move { pick: usize, seed: u8 },
    DeleteObject { pick: usize },
    CreateItem { seed: u8 },
    DeleteDocument { pick: usize },
    Rename { pick: usize },
    Protected { which: u8 },
    Malformed { which: u8 },
    Batch(Vec<Op>),
    Undo,
    Redo,
}

fn leaf() -> impl Strategy<Value = Op> {
    let pick = 0_usize..64;
    let seed = 0_u8..8;
    prop_oneof![
        (pick.clone(), seed.clone()).prop_map(|(pick, seed)| Op::SetSlug { pick, seed }),
        (pick.clone(), seed.clone()).prop_map(|(pick, seed)| Op::SetNote { pick, seed }),
        pick.clone().prop_map(|pick| Op::RemoveNote { pick }),
        (pick.clone(), seed.clone()).prop_map(|(pick, seed)| Op::AddTag { pick, seed }),
        pick.clone().prop_map(|pick| Op::RemoveFirstTag { pick }),
        (seed.clone(), pick.clone()).prop_map(|(seed, area)| Op::Place { seed, area }),
        (pick.clone(), seed.clone()).prop_map(|(pick, seed)| Op::Move { pick, seed }),
        pick.clone().prop_map(|pick| Op::DeleteObject { pick }),
        seed.clone().prop_map(|seed| Op::CreateItem { seed }),
        pick.clone().prop_map(|pick| Op::DeleteDocument { pick }),
        pick.prop_map(|pick| Op::Rename { pick }),
        (0_u8..4).prop_map(|which| Op::Protected { which }),
        (0_u8..3).prop_map(|which| Op::Malformed { which }),
    ]
}

fn apply_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => leaf(),
        1 => prop::collection::vec(leaf(), 1..=4).prop_map(Op::Batch),
    ]
}

fn any_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => apply_op(),
        1 => Just(Op::Undo),
        1 => Just(Op::Redo),
    ]
}

fn pick<T: Clone>(list: &[T], index: usize, fallback: T) -> T {
    if list.is_empty() {
        fallback
    } else {
        list[index % list.len()].clone()
    }
}

fn ids(doc: &CampaignDocument, kind: Option<ObjectKind>) -> Vec<Ulid> {
    doc.campaign()
        .index
        .iter()
        .filter(|(_, entry)| kind.is_none_or(|k| entry.kind == k))
        .map(|(id, _)| *id)
        .collect()
}

fn set(target: EditTarget, pointer: &str, value: Vec<u8>) -> EditCommand {
    EditCommand::SetValue {
        target,
        pointer: pointer.to_owned(),
        value,
    }
}

fn remove(target: EditTarget, pointer: &str) -> EditCommand {
    EditCommand::RemoveValue {
        target,
        pointer: pointer.to_owned(),
    }
}

/// Resolves one non-batch op against the current state.
fn command(doc: &CampaignDocument, op: &Op) -> EditCommand {
    let unknown = id(0x3fff);
    let object = |index: usize| EditTarget::Object(pick(&ids(doc, None), index, unknown));
    let paths: Vec<SourcePath> = doc.canonical_files().keys().cloned().collect();
    let manifest = || EditTarget::Document(sp("campaign.json"));
    match op {
        Op::SetSlug { pick, seed } => set(object(*pick), "/slug", json_str(&format!("s{seed}"))),
        Op::SetNote { pick, seed } => set(object(*pick), "/_note", json_str(&format!("n{seed}"))),
        Op::RemoveNote { pick } => remove(object(*pick), "/_note"),
        Op::AddTag { pick, seed } => EditCommand::InsertValue {
            target: object(*pick),
            pointer: "/tags/-".to_owned(),
            value: json_str(&format!("t{seed}")),
        },
        Op::RemoveFirstTag { pick } => remove(object(*pick), "/tags/0"),
        Op::Place { seed, area } => EditCommand::PlaceInstance {
            area: pick(&ids(doc, Some(ObjectKind::Area)), *area, unknown),
            placement: Placement {
                id: id(0x1000 + u128::from(*seed)),
                slug: format!("p{seed}"),
                name: "fixture.spawn".to_owned(),
                note: None,
                prefab: id(4),
                transform: transform(0),
                overrides: BTreeMap::new(),
            },
        },
        Op::Move { pick: index, seed } => EditCommand::MovePlacement {
            placement: pick(&ids(doc, Some(ObjectKind::Placement)), *index, unknown),
            transform: transform(i32::from(*seed)),
        },
        Op::DeleteObject { pick: index } => EditCommand::DeleteObject {
            id: pick(&ids(doc, None), *index, unknown),
        },
        Op::CreateItem { seed } => EditCommand::CreateDocument {
            path: sp(&format!("items/i{seed}.json")),
            document: Document::Item(Item {
                id: id(0x2000 + u128::from(*seed)),
                slug: format!("i{seed}"),
                name: "fixture.creature".to_owned(),
                note: None,
                stats: BTreeMap::new(),
                tags: Vec::new(),
            }),
        },
        Op::DeleteDocument { pick: index } => EditCommand::DeleteDocument {
            path: pick(&paths, *index, sp("missing.json")),
        },
        Op::Rename { pick: index } => {
            let from = pick(&paths, *index, sp("missing.json"));
            let to = match from.as_str().strip_suffix(".json") {
                Some(stem) => format!("{stem}_r.json"),
                None => format!("{}_r", from.as_str()),
            };
            EditCommand::RenameDocument { from, to: sp(&to) }
        }
        Op::Protected { which } => match which {
            0 => set(manifest(), "/schema", json_str("crpg.item/2")),
            1 => set(manifest(), "/engine", json_str(">=0.1.0")),
            2 => set(
                EditTarget::Document(sp("campaign.lock")),
                "/packages",
                b"[]".to_vec(),
            ),
            _ => set(manifest(), "/id", json_str(&id(0x3ffe).to_string())),
        },
        Op::Malformed { which } => {
            let value: &[u8] = match which {
                0 => b"1.5",
                1 => br#"{"a":1,"a":2}"#,
                _ => &[0xff, 0xfe],
            };
            set(manifest(), "/_note", value.to_vec())
        }
        Op::Batch(_) | Op::Undo | Op::Redo => unreachable!("resolved by the caller"),
    }
}

/// Resolves an apply op (a leaf or a batch of leaves) to its command batch.
fn commands(doc: &CampaignDocument, op: &Op) -> Vec<EditCommand> {
    match op {
        Op::Batch(ops) => ops.iter().map(|op| command(doc, op)).collect(),
        op => vec![command(doc, op)],
    }
}

/// Applies a batch and checks the per-call guarantees.
fn checked_apply(
    doc: &mut CampaignDocument,
    batch: Vec<EditCommand>,
) -> Result<CommandReceipt, EditError> {
    let before = snapshot(doc);
    let result = doc.apply_batch(batch);
    match &result {
        Ok(receipt) => {
            assert_eq!(receipt.changes, diff(&before.files, doc.canonical_files()));
            assert_eq!(receipt.diagnostics, crpg_data::validate(doc.campaign()));
            assert_eq!(receipt.revision, doc.revision());
        }
        Err(_) => assert_unchanged(&before, doc),
    }
    result
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn random_command_sequences_round_trip(ops in prop::collection::vec(apply_op(), 1..=48)) {
        let mut doc = open();
        let original = doc.canonical_files().clone();
        let mut accepted: Vec<(Vec<EditCommand>, CommandReceipt)> = Vec::new();
        for op in &ops {
            let batch = commands(&doc, op);
            if let Ok(receipt) = checked_apply(&mut doc, batch.clone()) {
                accepted.push((batch, receipt));
            }
        }
        let last = doc.canonical_files().clone();

        // 1. Undo to the original bytes.
        loop {
            match doc.undo() {
                Ok(_) => {}
                Err(EditError::NothingToUndo) => break,
                Err(other) => panic!("undo failed: {other:?}"),
            }
        }
        prop_assert_eq!(doc.canonical_files(), &original);

        // 2. Redo to the final bytes.
        loop {
            match doc.redo() {
                Ok(_) => {}
                Err(EditError::NothingToRedo) => break,
                Err(other) => panic!("redo failed: {other:?}"),
            }
        }
        prop_assert_eq!(doc.canonical_files(), &last);

        // 3. Replaying only the accepted calls gives identical receipts.
        let mut replay = open();
        for (batch, receipt) in &accepted {
            prop_assert_eq!(&replay.apply_batch(batch.clone()).unwrap(), receipt);
        }
        prop_assert_eq!(replay.canonical_files(), &last);

        // 4. The original files plus the save plan load back to the same state.
        let saved = apply_plan(&original, &doc.save_plan());
        let reopened = CampaignDocument::open(load(&saved)).unwrap();
        prop_assert_eq!(reopened.canonical_files(), &last);
    }

    #[test]
    fn undo_redo_interleavings_match_snapshot_model(ops in prop::collection::vec(any_op(), 1..=64)) {
        let mut doc = open();
        let mut snapshots = vec![doc.canonical_files().clone()];
        let mut cursor = 0_usize;
        for op in &ops {
            match op {
                Op::Undo => match doc.undo() {
                    Ok(_) => cursor -= 1,
                    Err(error) => {
                        prop_assert_eq!(error, EditError::NothingToUndo);
                        prop_assert_eq!(cursor, 0);
                    }
                },
                Op::Redo => match doc.redo() {
                    Ok(_) => cursor += 1,
                    Err(error) => {
                        prop_assert_eq!(error, EditError::NothingToRedo);
                        prop_assert_eq!(cursor, snapshots.len() - 1);
                    }
                },
                op => {
                    let batch = commands(&doc, op);
                    if let Ok(receipt) = checked_apply(&mut doc, batch) {
                        if !receipt.changes.is_empty() {
                            snapshots.truncate(cursor + 1);
                            snapshots.push(doc.canonical_files().clone());
                            cursor += 1;
                        }
                    }
                }
            }
            prop_assert_eq!(doc.canonical_files(), &snapshots[cursor]);
            prop_assert_eq!(doc.undo_depth(), cursor);
            prop_assert_eq!(doc.redo_depth(), snapshots.len() - 1 - cursor);
        }
    }
}
