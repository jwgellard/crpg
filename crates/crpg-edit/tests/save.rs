//! T058 §11 `tests/save.rs`: the pure save plan, `mark_saved`, the loader
//! round trip and the no-migration rule.

mod support;

use crpg_data::{read_document, schema_versions, write_document, Document, Item, SourcePath};
use crpg_edit::{CampaignDocument, EditCommand, EditError, EditTarget};
use std::collections::{BTreeMap, BTreeSet};
use support::*;

/// A `crpg.item/1` document that `load_campaign` migrates in memory.
const MIGRATED_ITEM: &[u8] =
    include_bytes!("../../crpg-data/tests/fixtures/migration_v1/campaign/items/item.json");

fn slug(target: EditTarget, slug: &str) -> EditCommand {
    EditCommand::SetValue {
        target,
        pointer: "/slug".to_owned(),
        value: json_str(slug),
    }
}

fn sword() -> EditCommand {
    EditCommand::CreateDocument {
        path: sp("items/sword.json"),
        document: Document::Item(Item {
            id: id(20),
            slug: "sword".to_owned(),
            name: "fixture.creature".to_owned(),
            note: None,
            stats: BTreeMap::new(),
            tags: Vec::new(),
        }),
    }
}

/// The four kinds of change: create, modify, delete and rename.
fn mixed_edits(doc: &mut CampaignDocument) {
    doc.apply(sword()).unwrap();
    doc.apply(slug(EditTarget::Object(id(4)), "goblin"))
        .unwrap();
    doc.apply(EditCommand::DeleteDocument {
        path: sp("areas/start/triggers.json"),
    })
    .unwrap();
    doc.apply(EditCommand::RenameDocument {
        from: sp("worlds/world.json"),
        to: sp("worlds/main.json"),
    })
    .unwrap();
}

fn paths(list: &[&str]) -> BTreeSet<SourcePath> {
    list.iter().map(|p| sp(p)).collect()
}

/// The top-level `schema` member of canonical bytes (two-space indent).
fn schema_tag(bytes: &[u8]) -> String {
    let text = std::str::from_utf8(bytes).expect("canonical bytes are UTF-8");
    let line = text
        .lines()
        .find_map(|line| line.strip_prefix("  \"schema\": \""))
        .expect("a top-level schema member");
    line.trim_end_matches(',').trim_end_matches('"').to_owned()
}

fn current_tag(tag: &str) -> String {
    let (family, _) = tag.split_once('/').expect("a versioned tag");
    let version = schema_versions()
        .iter()
        .find(|v| v.schema_type == family)
        .expect("a registered family");
    format!("{}/{}", version.schema_type, version.current)
}

#[test]
fn fresh_document_has_empty_save_plan() {
    let doc = open();
    let plan = doc.save_plan();
    assert_eq!(plan.revision, 0);
    assert!(plan.write.is_empty());
    assert!(plan.remove.is_empty());
}

#[test]
fn save_plan_lists_writes_and_removals() {
    let mut doc = open();
    mixed_edits(&mut doc);
    let plan = doc.save_plan();
    assert_eq!(plan.revision, 4);
    assert_eq!(
        plan.write.keys().cloned().collect::<BTreeSet<_>>(),
        paths(&[
            "creatures/creature.json",
            "items/sword.json",
            "worlds/main.json"
        ])
    );
    for (path, bytes) in &plan.write {
        assert_eq!(bytes, &doc.canonical_files()[path]);
    }
    assert_eq!(
        plan.remove,
        paths(&["areas/start/triggers.json", "worlds/world.json"])
    );
}

#[test]
fn save_plan_round_trips_through_loader() {
    let mut doc = open();
    let baseline = doc.canonical_files().clone();
    assert_eq!(baseline, fixture_files());
    mixed_edits(&mut doc);
    let saved = apply_plan(&baseline, &doc.save_plan());
    assert_eq!(&saved, doc.canonical_files());
    let reopened = CampaignDocument::open(load(&saved)).unwrap();
    assert_eq!(reopened.canonical_files(), doc.canonical_files());
    assert_eq!(reopened.campaign(), doc.campaign());
}

#[test]
fn undo_to_baseline_empties_the_plan() {
    let mut doc = open();
    mixed_edits(&mut doc);
    assert!(!doc.save_plan().write.is_empty());
    for _ in 0..4 {
        doc.undo().unwrap();
    }
    let plan = doc.save_plan();
    assert_eq!(plan.revision, 8);
    assert!(plan.write.is_empty() && plan.remove.is_empty());
}

#[test]
fn mark_saved_rebaselines_and_keeps_history() {
    let mut doc = open();
    mixed_edits(&mut doc);
    let plan = doc.save_plan();
    let (undo, redo, revision) = (doc.undo_depth(), doc.redo_depth(), doc.revision());
    doc.mark_saved(&plan).unwrap();
    assert_eq!(
        (doc.undo_depth(), doc.redo_depth(), doc.revision()),
        (undo, redo, revision)
    );
    let after_save = doc.save_plan();
    assert!(after_save.write.is_empty() && after_save.remove.is_empty());
    // The saved state stays undoable, and undoing makes the plan non-empty.
    doc.undo().unwrap();
    let plan = doc.save_plan();
    assert_eq!(plan.remove, paths(&["worlds/main.json"]));
    assert_eq!(
        plan.write.keys().cloned().collect::<BTreeSet<_>>(),
        paths(&["worlds/world.json"])
    );
}

#[test]
fn stale_save_plan_is_refused() {
    let mut doc = open();
    doc.apply(sword()).unwrap();
    let stale = doc.save_plan();
    doc.apply(slug(EditTarget::Object(id(4)), "goblin"))
        .unwrap();
    let before = snapshot(&doc);
    assert_eq!(doc.mark_saved(&stale), Err(EditError::StaleSavePlan));
    assert_unchanged(&before, &doc);
    let fresh = doc.save_plan();
    doc.mark_saved(&fresh).unwrap();
    assert!(doc.save_plan().write.is_empty());
}

#[test]
fn edits_never_migrate_or_retag() {
    let mut files = fixture_files();
    files.insert(sp("items/item.json"), MIGRATED_ITEM.to_vec());
    assert_eq!(schema_tag(MIGRATED_ITEM), "crpg.item/1");
    let mut doc = CampaignDocument::open(load(&files)).unwrap();
    // Opening never schedules a write of a migrated, untouched document.
    assert!(doc.save_plan().write.is_empty());

    mixed_edits(&mut doc);
    doc.apply(EditCommand::PlaceInstance {
        area: id(3),
        placement: crpg_data::Placement {
            id: id(30),
            slug: "guard".to_owned(),
            name: "fixture.spawn".to_owned(),
            note: None,
            prefab: id(4),
            transform: transform(2),
            overrides: BTreeMap::new(),
        },
    })
    .unwrap();
    assert!(!doc.save_plan().write.contains_key(&sp("items/item.json")));
    doc.apply(slug(EditTarget::Object(id(8)), "relic")).unwrap();

    for (path, bytes) in doc.canonical_files() {
        let tag = schema_tag(bytes);
        assert_eq!(tag, current_tag(&tag), "{}", path.as_str());
        assert_eq!(
            &write_document(&read_document(bytes).unwrap()).unwrap(),
            bytes,
            "{}",
            path.as_str()
        );
    }
    let plan = doc.save_plan();
    assert_eq!(
        schema_tag(&plan.write[&sp("items/item.json")]),
        "crpg.item/2"
    );
}
