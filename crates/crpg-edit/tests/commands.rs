//! T058 §11 `tests/commands.rs`: open, every command, protection, identity,
//! atomicity, bounds, receipts and the pinned `Display` strings.

mod support;

use crpg_data::{
    serialize_campaign, write_document, Diagnostic, DiagnosticCode, Document, IndexEntry, Item,
    ObjectKind, Placement, PointerEditError, Severity,
};
use crpg_edit::{
    CampaignDocument, ChangeKind, CommandReceipt, EditCommand, EditError, EditLimit, EditTarget,
    MAX_BATCH_COMMANDS, MAX_DOCUMENT_BYTES, MAX_POINTER_BYTES, MAX_VALUE_BYTES,
};
use std::collections::BTreeMap;
use support::*;

const CREATURE: &str = "creatures/creature.json";
const PLACEMENTS: &str = "areas/start/placements.json";
const LOCALE: &str = "locale/en.json";

fn obj(n: u128) -> EditTarget {
    EditTarget::Object(id(n))
}

fn at(path: &str) -> EditTarget {
    EditTarget::Document(sp(path))
}

fn set(target: EditTarget, pointer: &str, value: &[u8]) -> EditCommand {
    EditCommand::SetValue {
        target,
        pointer: pointer.to_owned(),
        value: value.to_vec(),
    }
}

fn insert(target: EditTarget, pointer: &str, value: &[u8]) -> EditCommand {
    EditCommand::InsertValue {
        target,
        pointer: pointer.to_owned(),
        value: value.to_vec(),
    }
}

fn remove(target: EditTarget, pointer: &str) -> EditCommand {
    EditCommand::RemoveValue {
        target,
        pointer: pointer.to_owned(),
    }
}

fn item(n: u128, slug: &str) -> Item {
    Item {
        id: id(n),
        slug: slug.to_owned(),
        name: "fixture.creature".to_owned(),
        note: None,
        stats: BTreeMap::new(),
        tags: Vec::new(),
    }
}

fn placement(n: u128, slug: &str) -> Placement {
    Placement {
        id: id(n),
        slug: slug.to_owned(),
        name: "fixture.spawn".to_owned(),
        note: None,
        prefab: id(4),
        transform: transform(0),
        overrides: BTreeMap::new(),
    }
}

fn ulid_json(n: u128) -> Vec<u8> {
    format!("\"{}\"", id(n)).into_bytes()
}

/// Applies a command that must fail, asserts §5.4 atomicity, returns the error.
fn refused(doc: &mut CampaignDocument, commands: Vec<EditCommand>) -> EditError {
    let before = snapshot(doc);
    let error = doc
        .apply_batch(commands)
        .expect_err("command must be refused");
    assert_unchanged(&before, doc);
    error
}

fn pointer_error(doc: &mut CampaignDocument, command: EditCommand) -> PointerEditError {
    match refused(doc, vec![command]) {
        EditError::Pointer {
            index: 0, error, ..
        } => error,
        other => panic!("expected a pointer error, got {other:?}"),
    }
}

fn rejected_code(error: EditError) -> DiagnosticCode {
    match error {
        EditError::Rejected {
            index: 0,
            diagnostic,
        } => diagnostic.code,
        other => panic!("expected Rejected, got {other:?}"),
    }
}

fn creature(doc: &CampaignDocument) -> crpg_data::Creature {
    match &doc.campaign().documents[&sp(CREATURE)] {
        Document::Creature(creature) => creature.clone(),
        other => panic!("expected a creature, got {other:?}"),
    }
}

fn fixture_creature() -> crpg_data::Creature {
    match fixture_document(CREATURE) {
        Document::Creature(creature) => creature,
        other => panic!("expected a creature, got {other:?}"),
    }
}

#[test]
fn open_rebuilds_index_and_canonical_bytes() {
    let doc = open();
    assert_eq!(
        doc.canonical_files(),
        &serialize_campaign(&fixture()).unwrap()
    );
    assert_eq!(doc.revision(), 0);
    assert_eq!(
        (doc.undo_depth(), doc.redo_depth(), doc.history_bytes()),
        (0, 0, 0)
    );
    assert_eq!(doc.campaign(), &fixture());

    let mut tampered = fixture();
    let real_index = tampered.index.clone();
    assert!(tampered.index.remove(&id(4)).is_some());
    tampered.index.insert(
        id(99),
        IndexEntry {
            kind: ObjectKind::Item,
            path: sp("items/ghost.json"),
            pointer: "/nowhere".to_owned(),
        },
    );
    let other = CampaignDocument::open(tampered).unwrap();
    assert_eq!(other.canonical_files(), doc.canonical_files());
    assert_eq!(other.campaign(), doc.campaign());
    assert_eq!(other.campaign().index, real_index);
}

#[test]
fn open_rejects_structurally_invalid_campaign() {
    let mut campaign = fixture();
    campaign.documents.remove(&sp("campaign.lock"));
    match CampaignDocument::open(campaign) {
        Err(EditError::Invalid { diagnostic }) => {
            assert_eq!(diagnostic.code, DiagnosticCode::Layout)
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    // Positive control: the untouched fixture opens.
    assert!(CampaignDocument::open(fixture()).is_ok());
}

#[test]
fn set_value_replaces_existing_member() {
    let mut doc = open();
    let receipt = doc
        .apply(set(obj(4), "/slug", &json_str("goblin")))
        .unwrap();
    assert_eq!(receipt.revision, 1);
    assert_eq!(doc.revision(), 1);
    assert_eq!(
        receipt.changes,
        vec![change(CREATURE, ChangeKind::Modified)]
    );
    let mut expected = fixture_creature();
    expected.slug = "goblin".to_owned();
    assert_eq!(
        doc.canonical_files()[&sp(CREATURE)],
        write_document(&Document::Creature(expected)).unwrap()
    );
}

#[test]
fn set_value_adds_absent_object_member() {
    let mut doc = open();
    let receipt = doc
        .apply(set(at(LOCALE), "/strings/mvp.npc", &json_str("Mayor")))
        .unwrap();
    assert_eq!(receipt.changes, vec![change(LOCALE, ChangeKind::Modified)]);
    let Document::Locale(mut expected) = fixture_document(LOCALE) else {
        panic!("fixture locale");
    };
    expected
        .strings
        .insert("mvp.npc".to_owned(), "Mayor".to_owned());
    assert_eq!(
        doc.canonical_files()[&sp(LOCALE)],
        write_document(&Document::Locale(expected)).unwrap()
    );
}

#[test]
fn set_value_identical_is_noop() {
    // On a fresh document: revision stays 0 and both stacks stay empty.
    let mut doc = open();
    let before = snapshot(&doc);
    let receipt = doc
        .apply(set(obj(4), "/slug", &json_str("creature")))
        .unwrap();
    assert_eq!(
        receipt,
        CommandReceipt {
            revision: 0,
            changes: Vec::new(),
            diagnostics: doc.validate(),
        }
    );
    assert_unchanged(&before, &doc);

    // With a pre-existing redo entry: the no-op keeps it and the revision.
    doc.apply(set(obj(4), "/slug", &json_str("goblin")))
        .unwrap();
    doc.undo().unwrap();
    assert_eq!(
        (doc.revision(), doc.undo_depth(), doc.redo_depth()),
        (2, 0, 1)
    );
    let before = snapshot(&doc);
    let receipt = doc
        .apply(set(obj(4), "/slug", &json_str("creature")))
        .unwrap();
    assert!(receipt.changes.is_empty());
    assert_eq!(receipt.revision, 2);
    assert_unchanged(&before, &doc);
    doc.redo().unwrap();
    assert_eq!(creature(&doc).slug, "goblin");
}

#[test]
fn insert_value_appends_and_inserts() {
    let mut doc = open();
    doc.apply(insert(obj(4), "/tags/-", &json_str("boss")))
        .unwrap();
    assert_eq!(creature(&doc).tags, ["fixture", "boss"]);
    let receipt = doc
        .apply(insert(obj(4), "/tags/0", &json_str("first")))
        .unwrap();
    assert_eq!(
        receipt.changes,
        vec![change(CREATURE, ChangeKind::Modified)]
    );
    assert_eq!(creature(&doc).tags, ["first", "fixture", "boss"]);
}

#[test]
fn remove_value_removes_member_and_element() {
    let mut doc = open();
    doc.apply(remove(obj(4), "/tags/0")).unwrap();
    assert!(creature(&doc).tags.is_empty());
    let receipt = doc.apply(remove(obj(1), "/_note")).unwrap();
    assert_eq!(
        receipt.changes,
        vec![change("campaign.json", ChangeKind::Modified)]
    );
    let Document::Campaign(campaign) = &doc.campaign().documents[&sp("campaign.json")] else {
        panic!("campaign manifest");
    };
    assert_eq!(campaign.note, None);
}

#[test]
fn pointer_errors_are_atomic() {
    let mut doc = open();
    let value = json_str("x");
    assert_eq!(
        pointer_error(&mut doc, set(obj(4), "slug", &value)),
        PointerEditError::InvalidPointer
    );
    assert_eq!(
        pointer_error(&mut doc, set(obj(4), "/tags/01", &value)),
        PointerEditError::InvalidPointer
    );
    assert_eq!(
        pointer_error(&mut doc, set(obj(4), "/tags/9", &value)),
        PointerEditError::NotFound
    );
    assert_eq!(
        pointer_error(&mut doc, set(obj(4), "/nosuch/x", &value)),
        PointerEditError::NotFound
    );
    assert_eq!(
        pointer_error(&mut doc, insert(obj(4), "/slug/0", &value)),
        PointerEditError::NotFound
    );
    // A relative pointer is decoded on its own before the object's index
    // pointer is prefixed, and the error names the object's document.
    match refused(&mut doc, vec![set(obj(5), "slug", &value)]) {
        EditError::Pointer { index, path, error } => {
            assert_eq!(
                (index, path, error),
                (0, sp(PLACEMENTS), PointerEditError::InvalidPointer)
            )
        }
        other => panic!("expected a pointer error, got {other:?}"),
    }
    // Positive controls.
    doc.apply(set(obj(4), "/tags/0", &value)).unwrap();
    doc.apply(set(obj(5), "/slug", &value)).unwrap();
}

#[test]
fn malformed_values_are_atomic() {
    let mut doc = open();
    let values: [&[u8]; 5] = [b"1.5", br#"{"a":1,"a":2}"#, b"\"unterminated", b"", &[0xff]];
    for value in values {
        assert!(matches!(
            pointer_error(&mut doc, set(obj(4), "/_note", value)),
            PointerEditError::Value { .. }
        ));
    }
    doc.apply(set(obj(4), "/_note", &json_str("fine"))).unwrap();
}

#[test]
fn typed_decode_failures_are_atomic() {
    let mut doc = open();
    assert!(matches!(
        pointer_error(&mut doc, set(obj(4), "/stats/health", &json_str("x"))),
        PointerEditError::Document { .. }
    ));
    assert!(matches!(
        pointer_error(&mut doc, set(obj(4), "/bogus", b"1")),
        PointerEditError::Document { .. }
    ));
    doc.apply(set(obj(4), "/stats/health", b"65536")).unwrap();
}

#[test]
fn schema_tag_and_root_are_refused() {
    let mut doc = open();
    assert_eq!(
        pointer_error(&mut doc, set(obj(4), "/schema", &json_str("crpg.item/2"))),
        PointerEditError::SchemaTag
    );
    assert_eq!(
        pointer_error(&mut doc, set(at(CREATURE), "", b"{}")),
        PointerEditError::RootPointer
    );
    assert_eq!(
        pointer_error(&mut doc, remove(obj(4), "")),
        PointerEditError::RootPointer
    );
    doc.apply(set(at(CREATURE), "/slug", &json_str("ok")))
        .unwrap();
}

fn protected(error: EditError) -> (String, String) {
    match error {
        EditError::Protected {
            index: 0,
            path,
            pointer,
        } => (path.as_str().to_owned(), pointer),
        other => panic!("expected Protected, got {other:?}"),
    }
}

#[test]
fn lock_documents_are_protected() {
    let mut doc = open();
    let lock = || (String::from("campaign.lock"), String::new());
    assert_eq!(
        protected(refused(
            &mut doc,
            vec![set(at("campaign.lock"), "/packages", b"[]")]
        )),
        lock()
    );
    assert_eq!(
        protected(refused(
            &mut doc,
            vec![set(at("assets/assets.lock"), "/assets", b"{}")]
        )),
        (String::from("assets/assets.lock"), String::new())
    );
    assert_eq!(
        protected(refused(
            &mut doc,
            vec![EditCommand::DeleteDocument {
                path: sp("campaign.lock")
            }]
        )),
        lock()
    );
    assert_eq!(
        protected(refused(
            &mut doc,
            vec![EditCommand::RenameDocument {
                from: sp("campaign.lock"),
                to: sp("items/x.json"),
            }]
        )),
        lock()
    );
    // CONTRACT CONTRADICTION (see the T058 completion record): §11 expects
    // `Protected` for a rename onto a lock, but §5.3 checks resolution (step
    // 2: `RenameDocument.to` present → `PathExists`) before protection
    // (step 3), and both lock paths are required files, so they are always
    // present in an open document. The contract's own precedence yields
    // `PathExists`, which is asserted here pending a decision.
    assert_eq!(
        refused(
            &mut doc,
            vec![EditCommand::RenameDocument {
                from: sp(CREATURE),
                to: sp("campaign.lock"),
            }]
        ),
        EditError::PathExists {
            index: 0,
            path: sp("campaign.lock")
        }
    );
    for lock_document in [
        fixture_document("campaign.lock"),
        fixture_document("assets/assets.lock"),
    ] {
        assert_eq!(
            protected(refused(
                &mut doc,
                vec![EditCommand::CreateDocument {
                    path: sp("items/x.json"),
                    document: lock_document,
                }]
            )),
            (String::from("items/x.json"), String::new())
        );
    }
    // Positive control: an ordinary document at the same path is accepted.
    doc.apply(EditCommand::CreateDocument {
        path: sp("items/x.json"),
        document: Document::Item(item(20, "x")),
    })
    .unwrap();
}

#[test]
fn manifest_package_fields_are_protected() {
    let mut doc = open();
    let cases = [
        set(obj(1), "/engine", &json_str(">=0.1.0")),
        set(obj(1), "/package", &json_str("fixture.other")),
        set(obj(1), "/requires", b"[]"),
        insert(obj(1), "/requires/-", b"{}"),
        remove(at("campaign.json"), "/engine"),
    ];
    let pointers = ["/engine", "/package", "/requires", "/requires/-", "/engine"];
    for (command, pointer) in cases.into_iter().zip(pointers) {
        assert_eq!(
            protected(refused(&mut doc, vec![command])),
            (String::from("campaign.json"), pointer.to_owned())
        );
    }
    // Positive controls: `/version` and `/entry/spawn` stay editable.
    doc.apply(set(obj(1), "/version", &json_str("0.2.0")))
        .unwrap();
    doc.apply(EditCommand::PlaceInstance {
        area: id(3),
        placement: placement(8, "second-spawn"),
    })
    .unwrap();
    let receipt = doc
        .apply(set(obj(1), "/entry/spawn", &ulid_json(8)))
        .unwrap();
    assert_eq!(
        receipt.changes,
        vec![change("campaign.json", ChangeKind::Modified)]
    );
}

#[test]
fn identity_changes_are_refused() {
    let mut doc = open();
    assert_eq!(
        refused(&mut doc, vec![set(obj(4), "/id", &ulid_json(99))]),
        EditError::IdentityChanged {
            index: 0,
            path: sp(CREATURE),
            id: id(4)
        }
    );
    let other = crpg_data::canonical_json(&placement(98, "spawn")).unwrap();
    assert_eq!(
        refused(&mut doc, vec![set(at(PLACEMENTS), "/placements/0", &other)]),
        EditError::IdentityChanged {
            index: 0,
            path: sp(PLACEMENTS),
            id: id(5)
        }
    );
    let same = crpg_data::canonical_json(&placement(5, "renamed")).unwrap();
    let receipt = doc
        .apply(set(at(PLACEMENTS), "/placements/0", &same))
        .unwrap();
    assert_eq!(
        receipt.changes,
        vec![change(PLACEMENTS, ChangeKind::Modified)]
    );
}

#[test]
fn create_document_adds_entity() {
    let mut doc = open();
    let receipt = doc
        .apply(EditCommand::CreateDocument {
            path: sp("items/sword.json"),
            document: Document::Item(item(20, "sword")),
        })
        .unwrap();
    assert_eq!(
        receipt.changes,
        vec![change("items/sword.json", ChangeKind::Created)]
    );
    assert_eq!(
        doc.campaign().index[&id(20)],
        IndexEntry {
            kind: ObjectKind::Item,
            path: sp("items/sword.json"),
            pointer: String::new(),
        }
    );
}

#[test]
fn create_document_rejections() {
    let mut doc = open();
    assert_eq!(
        refused(
            &mut doc,
            vec![EditCommand::CreateDocument {
                path: sp(CREATURE),
                document: Document::Item(item(20, "x")),
            }]
        ),
        EditError::PathExists {
            index: 0,
            path: sp(CREATURE)
        }
    );
    let mut misplaced = fixture_creature();
    misplaced.id = id(21);
    assert_eq!(
        rejected_code(refused(
            &mut doc,
            vec![EditCommand::CreateDocument {
                path: sp("items/x.json"),
                document: Document::Creature(misplaced),
            }]
        )),
        DiagnosticCode::Layout
    );
    assert_eq!(
        rejected_code(refused(
            &mut doc,
            vec![EditCommand::CreateDocument {
                path: sp("items/x.json"),
                document: Document::Item(item(4, "x")),
            }]
        )),
        DiagnosticCode::DuplicateId
    );
    doc.apply(EditCommand::CreateDocument {
        path: sp("items/x.json"),
        document: Document::Item(item(20, "x")),
    })
    .unwrap();
}

#[test]
fn delete_document_reports_dangling_references() {
    let mut doc = open();
    let receipt = doc
        .apply(EditCommand::DeleteDocument { path: sp(CREATURE) })
        .unwrap();
    assert_eq!(receipt.changes, vec![change(CREATURE, ChangeKind::Deleted)]);
    assert!(
        receipt.diagnostics.iter().any(|d| {
            d.code == DiagnosticCode::DanglingReference
                && d.file == Some(sp(PLACEMENTS))
                && d.pointer == "/placements/0/prefab"
        }),
        "{:?}",
        receipt.diagnostics
    );
    assert!(!doc.campaign().index.contains_key(&id(4)));
}

#[test]
fn delete_required_document_is_rejected() {
    let mut doc = open();
    assert_eq!(
        rejected_code(refused(
            &mut doc,
            vec![EditCommand::DeleteDocument {
                path: sp("campaign.json")
            }]
        )),
        DiagnosticCode::Layout
    );
    doc.apply(EditCommand::DeleteDocument {
        path: sp("areas/start/triggers.json"),
    })
    .unwrap();
}

#[test]
fn rename_document_preserves_bytes_and_identity() {
    let mut doc = open();
    let goblin = "creatures/humanoids/goblin.json";
    let bytes = doc.canonical_files()[&sp(CREATURE)].clone();
    let receipt = doc
        .apply(EditCommand::RenameDocument {
            from: sp(CREATURE),
            to: sp(goblin),
        })
        .unwrap();
    assert_eq!(
        receipt.changes,
        vec![
            change(CREATURE, ChangeKind::Deleted),
            change(goblin, ChangeKind::Created),
        ]
    );
    assert_eq!(doc.canonical_files()[&sp(goblin)], bytes);
    assert!(!doc.canonical_files().contains_key(&sp(CREATURE)));
    assert_eq!(doc.campaign().index[&id(4)].path, sp(goblin));

    let rename = |from: &str, to: &str| EditCommand::RenameDocument {
        from: sp(from),
        to: sp(to),
    };
    assert_eq!(
        refused(&mut doc, vec![rename(CREATURE, "creatures/x.json")]),
        EditError::MissingPath {
            index: 0,
            path: sp(CREATURE)
        }
    );
    assert_eq!(
        refused(&mut doc, vec![rename(goblin, LOCALE)]),
        EditError::PathExists {
            index: 0,
            path: sp(LOCALE)
        }
    );
    assert_eq!(
        rejected_code(refused(
            &mut doc,
            vec![rename(goblin, "items/creature.json")]
        )),
        DiagnosticCode::Layout
    );
}

#[test]
fn delete_object_root_and_nested() {
    let mut doc = open();
    let receipt = doc.apply(EditCommand::DeleteObject { id: id(5) }).unwrap();
    assert_eq!(
        receipt.changes,
        vec![change(PLACEMENTS, ChangeKind::Modified)]
    );
    let Document::Placements(aggregate) = &doc.campaign().documents[&sp(PLACEMENTS)] else {
        panic!("placements aggregate");
    };
    assert!(aggregate.placements.is_empty());
    assert!(!doc.campaign().index.contains_key(&id(5)));

    let receipt = doc.apply(EditCommand::DeleteObject { id: id(4) }).unwrap();
    assert_eq!(receipt.changes, vec![change(CREATURE, ChangeKind::Deleted)]);
    assert!(!doc.canonical_files().contains_key(&sp(CREATURE)));

    assert_eq!(
        refused(&mut doc, vec![EditCommand::DeleteObject { id: id(99) }]),
        EditError::UnknownObject {
            index: 0,
            id: id(99)
        }
    );
}

#[test]
fn place_instance_appends_or_creates_aggregate() {
    let mut doc = open();
    let receipt = doc
        .apply(EditCommand::PlaceInstance {
            area: id(3),
            placement: placement(8, "guard"),
        })
        .unwrap();
    assert_eq!(
        receipt.changes,
        vec![change(PLACEMENTS, ChangeKind::Modified)]
    );
    assert_eq!(
        doc.campaign().index[&id(8)],
        IndexEntry {
            kind: ObjectKind::Placement,
            path: sp(PLACEMENTS),
            pointer: "/placements/1".to_owned(),
        }
    );

    doc.apply(EditCommand::DeleteDocument {
        path: sp(PLACEMENTS),
    })
    .unwrap();
    let receipt = doc
        .apply(EditCommand::PlaceInstance {
            area: id(3),
            placement: placement(9, "guard"),
        })
        .unwrap();
    assert_eq!(
        receipt.changes,
        vec![change(PLACEMENTS, ChangeKind::Created)]
    );
    let expected = Document::Placements(crpg_data::PlacementsDocument {
        area: id(3),
        placements: vec![placement(9, "guard")],
        note: None,
    });
    assert_eq!(
        doc.canonical_files()[&sp(PLACEMENTS)],
        write_document(&expected).unwrap()
    );

    assert_eq!(
        refused(
            &mut doc,
            vec![EditCommand::PlaceInstance {
                area: id(4),
                placement: placement(10, "x"),
            }]
        ),
        EditError::WrongKind {
            index: 0,
            id: id(4),
            expected: ObjectKind::Area,
            found: ObjectKind::Creature,
        }
    );
    assert_eq!(
        refused(
            &mut doc,
            vec![EditCommand::PlaceInstance {
                area: id(99),
                placement: placement(10, "x"),
            }]
        ),
        EditError::UnknownObject {
            index: 0,
            id: id(99)
        }
    );
}

#[test]
fn move_placement_sets_transform() {
    let mut doc = open();
    let receipt = doc
        .apply(EditCommand::MovePlacement {
            placement: id(5),
            transform: transform(7),
        })
        .unwrap();
    assert_eq!(
        receipt.changes,
        vec![change(PLACEMENTS, ChangeKind::Modified)]
    );
    let Document::Placements(mut aggregate) = fixture_document(PLACEMENTS) else {
        panic!("placements aggregate");
    };
    aggregate.placements[0].transform = transform(7);
    assert_eq!(
        doc.canonical_files()[&sp(PLACEMENTS)],
        write_document(&Document::Placements(aggregate)).unwrap()
    );
    assert_eq!(
        refused(
            &mut doc,
            vec![EditCommand::MovePlacement {
                placement: id(4),
                transform: transform(1),
            }]
        ),
        EditError::WrongKind {
            index: 0,
            id: id(4),
            expected: ObjectKind::Placement,
            found: ObjectKind::Creature,
        }
    );
}

#[test]
fn move_placement_between_areas_in_one_batch() {
    let mut doc = open();
    let Document::Area(mut second) = fixture_document("areas/start/area.json") else {
        panic!("fixture area");
    };
    second.id = id(30);
    second.slug = "second".to_owned();
    doc.apply(EditCommand::CreateDocument {
        path: sp("areas/second/area.json"),
        document: Document::Area(second),
    })
    .unwrap();
    let depth = doc.undo_depth();
    let Document::Placements(spawn) = fixture_document(PLACEMENTS) else {
        panic!("placements aggregate");
    };
    let receipt = doc
        .apply_batch(vec![
            EditCommand::DeleteObject { id: id(5) },
            EditCommand::PlaceInstance {
                area: id(30),
                placement: spawn.placements[0].clone(),
            },
        ])
        .unwrap();
    assert_eq!(
        receipt.changes,
        vec![
            change("areas/second/placements.json", ChangeKind::Created),
            change(PLACEMENTS, ChangeKind::Modified),
        ]
    );
    assert_eq!(doc.undo_depth(), depth + 1);
    assert_eq!(
        doc.campaign().index[&id(5)].path,
        sp("areas/second/placements.json")
    );
}

#[test]
fn batch_is_atomic_and_reports_index() {
    let mut doc = open();
    let first = set(obj(4), "/slug", &json_str("goblin"));
    let second = set(at(LOCALE), "/strings/mvp.npc", &json_str("Mayor"));
    let bad = set(obj(4), "/nosuch/x", &json_str("x"));
    match refused(&mut doc, vec![first.clone(), second.clone(), bad]) {
        EditError::Pointer { index, .. } => assert_eq!(index, 2),
        other => panic!("expected a pointer error, got {other:?}"),
    }
    let before = doc.canonical_files().clone();
    let receipt = doc.apply_batch(vec![first, second]).unwrap();
    assert_eq!(doc.undo_depth(), 1);
    assert_eq!(receipt.revision, 1);
    assert_eq!(
        receipt.changes,
        vec![
            change(CREATURE, ChangeKind::Modified),
            change(LOCALE, ChangeKind::Modified),
        ]
    );
    assert_eq!(receipt.changes, diff(&before, doc.canonical_files()));
}

#[test]
fn batch_size_bounds() {
    let mut doc = open();
    assert_eq!(
        refused(&mut doc, Vec::new()),
        EditError::BatchSize { len: 0 }
    );
    let command = |i: usize| set(at(LOCALE), &format!("/strings/k{i:04}"), &json_str("v"));
    let too_many: Vec<_> = (0..MAX_BATCH_COMMANDS + 1).map(command).collect();
    assert_eq!(
        refused(&mut doc, too_many),
        EditError::BatchSize { len: 1025 }
    );
    let full: Vec<_> = (0..MAX_BATCH_COMMANDS).map(command).collect();
    let receipt = doc.apply_batch(full).unwrap();
    assert_eq!(receipt.changes, vec![change(LOCALE, ChangeKind::Modified)]);
    assert_eq!(doc.undo_depth(), 1);
    let Document::Locale(locale) = &doc.campaign().documents[&sp(LOCALE)] else {
        panic!("locale table");
    };
    assert_eq!(locale.strings.len(), 6 + MAX_BATCH_COMMANDS);
}

#[test]
fn input_size_bounds() {
    let mut doc = open();
    let pointer = |bytes: usize| format!("/{}", "a".repeat(bytes - 1));
    assert_eq!(
        refused(
            &mut doc,
            vec![remove(obj(4), &pointer(MAX_POINTER_BYTES + 1))]
        ),
        EditError::InputTooLarge {
            index: 0,
            limit: EditLimit::Pointer
        }
    );
    assert_eq!(
        pointer_error(&mut doc, remove(obj(4), &pointer(MAX_POINTER_BYTES))),
        PointerEditError::NotFound
    );
    let value = |bytes: usize| json_str(&"n".repeat(bytes - 2));
    assert_eq!(value(MAX_VALUE_BYTES).len(), 1_048_576);
    assert_eq!(
        refused(
            &mut doc,
            vec![set(obj(4), "/_note", &value(MAX_VALUE_BYTES + 1))]
        ),
        EditError::InputTooLarge {
            index: 0,
            limit: EditLimit::Value
        }
    );
    doc.apply(set(obj(4), "/_note", &value(MAX_VALUE_BYTES)))
        .unwrap();
    assert_eq!(
        creature(&doc).note.map(|n| n.len()),
        Some(MAX_VALUE_BYTES - 2)
    );
}

#[test]
fn document_size_bound() {
    let tag = "t".repeat(MAX_VALUE_BYTES - 2);
    // Independently find the first insert that pushes the creature past the cap.
    let mut expected = fixture_creature();
    let mut crossing = None;
    for i in 0..8 {
        expected.tags.push(tag.clone());
        let len = write_document(&Document::Creature(expected.clone()))
            .unwrap()
            .len();
        if len > MAX_DOCUMENT_BYTES {
            crossing = Some(i);
            break;
        }
    }
    let crossing = crossing.expect("the cap is crossed within eight inserts");
    let command = |_| insert(obj(4), "/tags/-", &json_str(&tag));

    let mut doc = open();
    assert_eq!(
        refused(&mut doc, (0..=crossing).map(command).collect()),
        EditError::InputTooLarge {
            index: crossing,
            limit: EditLimit::Document
        }
    );
    doc.apply_batch((0..crossing).map(command).collect())
        .unwrap();
    assert!(doc.canonical_files()[&sp(CREATURE)].len() <= MAX_DOCUMENT_BYTES);
    assert_eq!(creature(&doc).tags.len(), 1 + crossing);
}

#[test]
fn receipt_diagnostics_are_crpg_data_validate() {
    let mut doc = open();
    let commands = [
        set(at(LOCALE), "/strings/mvp.npc", &json_str("Mayor")),
        set(obj(5), "/slug", &json_str("creature")),
        set(obj(4), "/name", &json_str("missing.key")),
        EditCommand::DeleteDocument { path: sp(CREATURE) },
        EditCommand::DeleteObject { id: id(5) },
    ];
    let mut saw_findings = false;
    for command in commands {
        let receipt = doc.apply(command).unwrap();
        assert_eq!(receipt.diagnostics, crpg_data::validate(doc.campaign()));
        assert_eq!(
            receipt.diagnostics,
            validate_files(&doc.canonical_files().clone())
        );
        assert_eq!(receipt.diagnostics, doc.validate());
        saw_findings |= !receipt.diagnostics.is_empty();
    }
    assert!(saw_findings, "at least one receipt carries findings");
}

#[test]
fn document_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<CampaignDocument>();
    assert_send_sync::<EditCommand>();
    assert_send_sync::<EditError>();
}

#[test]
fn error_display_strings_are_pinned() {
    let diagnostic = Diagnostic {
        file: Some(sp("campaign.json")),
        pointer: "/id".to_owned(),
        severity: Severity::Error,
        code: DiagnosticCode::Layout,
        message: "m".to_owned(),
        suggested_fix: None,
    };
    let path = sp("items/x.json");
    let rows: Vec<(EditError, &str)> = vec![
        (
            EditError::Invalid {
                diagnostic: diagnostic.clone(),
            },
            "campaign cannot be opened for editing: campaign.json/id: error[layout]: m",
        ),
        (
            EditError::BatchSize { len: 1025 },
            "command batch has 1025 commands; expected 1 to 1024",
        ),
        (
            EditError::InputTooLarge {
                index: 2,
                limit: EditLimit::Pointer,
            },
            "command 2: pointer exceeds its byte cap",
        ),
        (
            EditError::InputTooLarge {
                index: 2,
                limit: EditLimit::Value,
            },
            "command 2: value exceeds its byte cap",
        ),
        (
            EditError::InputTooLarge {
                index: 2,
                limit: EditLimit::Document,
            },
            "command 2: document exceeds its byte cap",
        ),
        (
            EditError::MissingPath {
                index: 1,
                path: path.clone(),
            },
            "command 1: no document at items/x.json",
        ),
        (
            EditError::PathExists {
                index: 1,
                path: path.clone(),
            },
            "command 1: a document already exists at items/x.json",
        ),
        (
            EditError::UnknownObject {
                index: 3,
                id: id(4),
            },
            "command 3: unknown object 00000000000000000000000004",
        ),
        (
            EditError::WrongKind {
                index: 0,
                id: id(4),
                expected: ObjectKind::Area,
                found: ObjectKind::Creature,
            },
            "command 0: object 00000000000000000000000004 has the wrong kind for this command",
        ),
        (
            EditError::Protected {
                index: 0,
                path: sp("campaign.json"),
                pointer: "/engine".to_owned(),
            },
            "command 0: campaign.json/engine is protected",
        ),
        (
            EditError::Protected {
                index: 0,
                path: sp("campaign.lock"),
                pointer: String::new(),
            },
            "command 0: campaign.lock is protected",
        ),
        (
            EditError::Pointer {
                index: 0,
                path: sp(CREATURE),
                error: PointerEditError::NotFound,
            },
            "command 0: creatures/creature.json: json pointer target not found",
        ),
        (
            EditError::IdentityChanged {
                index: 0,
                path: sp(CREATURE),
                id: id(4),
            },
            "command 0: edit would change the identity of object 00000000000000000000000004 in creatures/creature.json",
        ),
        (
            EditError::Rejected {
                index: 0,
                diagnostic,
            },
            "command 0: rejected: campaign.json/id: error[layout]: m",
        ),
        (
            EditError::HistoryTooLarge { bytes: 5 },
            "change of 5 bytes exceeds the undo history cap",
        ),
        (EditError::NothingToUndo, "nothing to undo"),
        (EditError::NothingToRedo, "nothing to redo"),
        (EditError::StaleSavePlan, "save plan is stale"),
        (EditError::HistoryCorrupt, "undo history is corrupt"),
    ];
    for (error, text) in rows {
        assert_eq!(error.to_string(), text);
        assert!(std::error::Error::source(&error).is_none());
    }
    assert_eq!(EditLimit::Pointer.to_string(), "pointer");
    assert_eq!(EditLimit::Value.to_string(), "value");
    assert_eq!(EditLimit::Document.to_string(), "document");
}
