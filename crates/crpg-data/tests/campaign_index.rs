#![forbid(unsafe_code)]
//! T058a: the writer's structural check returning the loader's index.

mod support;
use crpg_core::{Fx16_16, Ulid};
use crpg_data::*;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

type Documents = BTreeMap<SourcePath, Document>;

fn engine() -> semver::Version {
    "0.1.0".parse().unwrap()
}
fn id(n: u128) -> Ulid {
    Ulid::from_u128(n)
}
fn path(s: &str) -> SourcePath {
    s.parse().unwrap()
}

fn read_tree(root: &Path) -> BTreeMap<SourcePath, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<SourcePath, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                walk(&path, root, out);
            } else if kind.is_file() {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                if name == ".gitattributes" {
                    continue;
                }
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let key: SourcePath = relative.parse().unwrap();
                out.insert(key, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// Every direct subdirectory of repo-root `campaigns/fixtures/` plus the
/// crate's three campaign fixtures.
fn fixture_roots() -> Vec<PathBuf> {
    let shared = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../campaigns/fixtures");
    let mut roots: Vec<PathBuf> = std::fs::read_dir(&shared)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_type().unwrap().is_dir())
        .map(|entry| entry.path())
        .collect();
    roots.sort();
    for required in ["combat_basic", "combat_srd"] {
        assert!(roots.contains(&shared.join(required)), "{required}");
    }
    let local = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in [
        "one_area_one_creature",
        "broken_references",
        "migration_v1/campaign",
    ] {
        roots.push(local.join(name));
    }
    roots
}

fn fixture_documents() -> Documents {
    load_campaign(&support::fixture_files(), &engine())
        .unwrap()
        .documents
}

/// The `one_area_one_creature` index, restated from `loader.rs`'s
/// `exact_fixture_index_bytes_and_models`.
fn fixture_index() -> BTreeMap<Ulid, IndexEntry> {
    [
        (1, ObjectKind::Campaign, "campaign.json", ""),
        (2, ObjectKind::World, "worlds/world.json", ""),
        (3, ObjectKind::Area, "areas/start/area.json", ""),
        (4, ObjectKind::Creature, "creatures/creature.json", ""),
        (
            5,
            ObjectKind::Placement,
            "areas/start/placements.json",
            "/placements/0",
        ),
        (
            6,
            ObjectKind::Graph,
            "areas/start/triggers.json",
            "/graphs/0",
        ),
        (
            7,
            ObjectKind::Node,
            "areas/start/triggers.json",
            "/graphs/0/nodes/0",
        ),
    ]
    .into_iter()
    .map(|(n, kind, p, pointer)| {
        (
            id(n),
            IndexEntry {
                kind,
                path: path(p),
                pointer: pointer.into(),
            },
        )
    })
    .collect()
}

/// What the loader derives after the writer serializes `documents`.
fn loader_index(documents: &Documents) -> BTreeMap<Ulid, IndexEntry> {
    let files = serialize_campaign(&LoadedCampaign {
        documents: documents.clone(),
        index: BTreeMap::new(),
    })
    .unwrap();
    load_campaign(&files, &engine()).unwrap().index
}

fn placement(n: u128) -> Placement {
    Placement {
        id: id(n),
        slug: format!("placement-{n}"),
        name: "fixture.spawn".into(),
        note: None,
        prefab: id(4),
        transform: Transform {
            position: [Fx16_16::ONE; 3],
            rotation: [Fx16_16::ZERO; 3],
            scale: [Fx16_16::ONE; 3],
        },
        overrides: BTreeMap::new(),
    }
}

fn edit_in(documents: &mut Documents, source: &str, edit: PointerEdit) {
    let key = path(source);
    let edited = edit_document(&documents[&key], &edit).unwrap();
    documents.insert(key, edited);
}

fn append_placement(documents: &mut Documents, n: u128) {
    edit_in(
        documents,
        "areas/start/placements.json",
        PointerEdit::Insert {
            pointer: "/placements/-".into(),
            value: canonical_json(&placement(n)).unwrap(),
        },
    );
}

#[test]
fn campaign_index_equals_loader_index_for_every_fixture_campaign() {
    let roots = fixture_roots();
    assert_eq!(roots.len(), 5);
    for root in roots {
        let loaded = load_campaign(&read_tree(&root), &engine())
            .unwrap_or_else(|e| panic!("{}: {e}", root.display()));
        assert!(!loaded.index.is_empty(), "{}", root.display());
        assert_eq!(
            campaign_index(&loaded.documents).unwrap(),
            loaded.index,
            "{}",
            root.display()
        );
    }
}

#[test]
fn campaign_index_pins_the_fixture_index() {
    let documents = fixture_documents();
    let before = documents.clone();
    assert_eq!(campaign_index(&documents).unwrap(), fixture_index());
    assert_eq!(documents, before);
}

fn assert_same_error(a: &DataError, b: &DataError, context: &str) {
    assert_eq!(
        std::mem::discriminant(a),
        std::mem::discriminant(b),
        "{context}: {a:?} vs {b:?}"
    );
    assert_eq!(a.to_string(), b.to_string(), "{context}");
}

#[test]
fn campaign_index_matches_writer_rejections() {
    for defect in 0..7 {
        let mut documents = fixture_documents();
        match defect {
            0 => {
                documents.remove(&path("campaign.lock"));
            }
            1 => {
                documents.insert(
                    path("CREATURES/creature.json"),
                    documents[&path("creatures/creature.json")].clone(),
                );
            }
            2 => {
                documents.insert(
                    path("wrong.json"),
                    documents[&path("creatures/creature.json")].clone(),
                );
            }
            3 => {
                let Some(Document::Creature(v)) =
                    documents.get_mut(&path("creatures/creature.json"))
                else {
                    panic!()
                };
                v.id = id(3);
            }
            4 => {
                let Some(Document::CampaignLock(v)) = documents.get_mut(&path("campaign.lock"))
                else {
                    panic!()
                };
                v.assets_lock = Digest::from_bytes([0; 32]);
            }
            5 => {
                let Some(Document::Campaign(v)) = documents.get_mut(&path("campaign.json")) else {
                    panic!()
                };
                v.requires.push(PackageRequirement {
                    kind: PackageKind::Module,
                    package: "missing".parse().unwrap(),
                    version: "*".parse().unwrap(),
                });
            }
            6 => {
                let Some(Document::AssetsLock(v)) = documents.get_mut(&path("assets/assets.lock"))
                else {
                    panic!()
                };
                v.assets.insert(
                    path("bad"),
                    AssetRecord {
                        hash: Digest::from_bytes([0; 32]),
                        import: BTreeMap::new(),
                    },
                );
            }
            _ => unreachable!(),
        }
        let before = documents.clone();
        let error = campaign_index(&documents).unwrap_err();
        assert_eq!(documents, before, "defect {defect}");
        match (defect, &error) {
            (0..=2, DataError::Layout { .. })
            | (3, DataError::DuplicateId { .. })
            | (4, DataError::AssetsLockMismatch { .. })
            | (5..=6, DataError::InvalidLock { .. }) => {}
            (_, error) => panic!("defect {defect}: {error}"),
        }
        // The writer ignores any caller index, stale or empty.
        for index in [BTreeMap::new(), fixture_index()] {
            let campaign = LoadedCampaign {
                documents: documents.clone(),
                index,
            };
            let writer = serialize_campaign(&campaign).unwrap_err();
            assert_same_error(&error, &writer, &format!("defect {defect} writer"));
            let explain = explain_object(&campaign, id(1)).unwrap_err();
            assert_same_error(&error, &explain, &format!("defect {defect} explain"));
        }
    }
}

#[test]
fn campaign_index_uses_writer_precedence() {
    let mut documents = fixture_documents();
    let Some(Document::AssetsLock(lock)) = documents.get_mut(&path("assets/assets.lock")) else {
        panic!()
    };
    lock.assets.insert(
        path("bad"),
        AssetRecord {
            hash: Digest::from_bytes([0; 32]),
            import: BTreeMap::new(),
        },
    );
    let bad_lock = documents[&path("assets/assets.lock")].clone();
    // The lock fault is document-local: the writer refuses it on its own,
    // and the loader's read phase reports it before layout.
    assert!(matches!(
        write_document(&bad_lock),
        Err(DataError::InvalidLock { .. })
    ));
    assert!(matches!(
        campaign_index(&documents),
        Err(DataError::InvalidLock { .. })
    ));
    documents.insert(
        path("wrong.json"),
        documents[&path("creatures/creature.json")].clone(),
    );
    let error = campaign_index(&documents).unwrap_err();
    assert!(
        matches!(&error, DataError::Layout { path: Some(p), .. } if p.as_str() == "wrong.json"),
        "{error}"
    );
    let files: BTreeMap<SourcePath, Vec<u8>> = documents
        .iter()
        .map(|(p, d)| (p.clone(), canonical_json(d).unwrap()))
        .collect();
    assert!(matches!(
        load_campaign(&files, &engine()),
        Err(DataError::InvalidLock { .. })
    ));
}

#[test]
fn campaign_index_ignores_engine_and_semantics() {
    let mut documents = fixture_documents();
    let Some(Document::Campaign(manifest)) = documents.get_mut(&path("campaign.json")) else {
        panic!()
    };
    manifest.engine = ">=99".parse().unwrap();
    let Some(Document::Creature(creature)) = documents.get_mut(&path("creatures/creature.json"))
    else {
        panic!()
    };
    creature.faction = Some(id(77));
    let campaign = LoadedCampaign {
        documents: documents.clone(),
        index: BTreeMap::new(),
    };
    // Both faults are real: the engine fails the loader and the reference
    // fails semantic validation.
    assert!(validate(&campaign).iter().any(|d| d.pointer == "/faction"));
    let files = serialize_campaign(&campaign).unwrap();
    assert!(matches!(
        load_campaign(&files, &engine()),
        Err(DataError::EngineIncompatible { .. })
    ));
    assert_eq!(campaign_index(&documents).unwrap(), fixture_index());
}

#[test]
fn edited_documents_reindex_like_the_loader() {
    let mut inserted = fixture_documents();
    append_placement(&mut inserted, 9);
    let index = campaign_index(&inserted).unwrap();
    assert_eq!(index[&id(9)].pointer, "/placements/1");
    assert_eq!(index, loader_index(&inserted));

    let mut removed = fixture_documents();
    edit_in(
        &mut removed,
        "areas/start/placements.json",
        PointerEdit::Remove {
            pointer: "/placements/0".into(),
        },
    );
    let index = campaign_index(&removed).unwrap();
    assert!(!index.contains_key(&id(5)));
    assert_eq!(index, loader_index(&removed));

    let mut moved = fixture_documents();
    let creature = moved.remove(&path("creatures/creature.json")).unwrap();
    moved.insert(path("creatures/humanoids/goblin.json"), creature);
    let index = campaign_index(&moved).unwrap();
    assert_eq!(index[&id(4)].path, path("creatures/humanoids/goblin.json"));
    assert_eq!(index, loader_index(&moved));

    // All three in sequence: the placement added first now sits at index 0.
    let mut all = fixture_documents();
    append_placement(&mut all, 9);
    edit_in(
        &mut all,
        "areas/start/placements.json",
        PointerEdit::Remove {
            pointer: "/placements/0".into(),
        },
    );
    let creature = all.remove(&path("creatures/creature.json")).unwrap();
    all.insert(path("creatures/humanoids/goblin.json"), creature);
    let index = campaign_index(&all).unwrap();
    assert_eq!(index[&id(9)].pointer, "/placements/0");
    assert_eq!(index, loader_index(&all));
}

#[test]
fn edited_duplicate_id_is_rejected() {
    let mut documents = fixture_documents();
    append_placement(&mut documents, 4);
    let before = documents.clone();
    let error = campaign_index(&documents).unwrap_err();
    assert_eq!(documents, before);
    // `build_index` folds occurrences in lexical path order, and
    // `areas/start/placements.json` sorts before `creatures/creature.json`.
    // T058a §6 names the reverse order; see the completion record.
    match error {
        DataError::DuplicateId {
            id: dup,
            first,
            second,
        } => {
            assert_eq!(dup, id(4));
            assert_eq!(first, path("areas/start/placements.json"));
            assert_eq!(second, path("creatures/creature.json"));
        }
        other => panic!("{other}"),
    }
    // Positive control: a fresh id is accepted.
    let mut documents = fixture_documents();
    append_placement(&mut documents, 9);
    assert!(campaign_index(&documents).is_ok());
}
