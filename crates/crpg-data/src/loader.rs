//! Pure campaign loading, layout checks and complete authored-object indexing.

use crate::{DataError, Document, SourcePath};
use crpg_core::Ulid;
use semver::Version;
use std::collections::{btree_map::Entry, BTreeMap, BTreeSet};

/// Kind of an independently identified authored object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ObjectKind {
    /// Campaign root.
    Campaign,
    /// World root.
    World,
    /// Area root.
    Area,
    /// Creature prefab.
    Creature,
    /// Item prefab.
    Item,
    /// Dialogue root.
    Dialogue,
    /// Quest root.
    Quest,
    /// Faction root.
    Faction,
    /// Placement aggregate entry.
    Placement,
    /// Standalone or aggregate-owned graph.
    Graph,
    /// Graph node.
    Node,
    /// Dialogue node.
    DialogueNode,
    /// Quest state.
    QuestState,
}

/// Derived location of an identified containing object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    /// Authored object category.
    pub kind: ObjectKind,
    /// Logical source path.
    pub path: SourcePath,
    /// RFC 6901 pointer to the object, not its id field.
    pub pointer: String,
}

/// All loaded source documents and their derived ULID index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedCampaign {
    /// Source information, including author notes.
    pub documents: BTreeMap<SourcePath, Document>,
    /// Derived lookup; never serialized or trusted by the writer.
    pub index: BTreeMap<Ulid, IndexEntry>,
}

fn layout(path: Option<&SourcePath>, message: impl Into<String>) -> DataError {
    DataError::Layout {
        path: path.cloned(),
        message: message.into(),
    }
}

fn check_paths<'a>(paths: impl Iterator<Item = &'a SourcePath>) -> Result<(), DataError> {
    let mut seen = BTreeSet::new();
    let mut present = BTreeSet::new();
    for path in paths {
        if !seen.insert(path.as_str().to_ascii_lowercase()) {
            return Err(layout(Some(path), "ASCII-case path collision"));
        }
        present.insert(path.as_str());
    }
    for required in ["campaign.json", "campaign.lock", "assets/assets.lock"] {
        if !present.contains(required) {
            return Err(layout(None, format!("missing required file: {required}")));
        }
    }
    Ok(())
}

pub(crate) fn family(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|rest| rest.ends_with(".json") && rest.len() > 5)
}

pub(crate) fn area_file(path: &str, name: &str) -> bool {
    path.strip_prefix("areas/")
        .and_then(|rest| rest.strip_suffix(name))
        .is_some_and(|middle| middle.len() > 1 && middle.ends_with('/'))
}

/// Shape of a locale document path, without checking document contents.
/// Shared with the validation path classifier so the two lists cannot drift.
pub(crate) fn locale_shape(path: &str) -> bool {
    path.strip_prefix("locale/")
        .and_then(|s| s.strip_suffix(".json"))
        .is_some_and(|stem| !stem.is_empty() && !stem.contains('/'))
}

fn check_layout(documents: &BTreeMap<SourcePath, Document>) -> Result<(), DataError> {
    for (path, document) in documents {
        let p = path.as_str();
        let matches = match document {
            Document::Campaign(_) => p == "campaign.json",
            Document::CampaignLock(_) => p == "campaign.lock",
            Document::AssetsLock(_) => p == "assets/assets.lock",
            Document::World(_) => family(p, "worlds/"),
            Document::Creature(_) => family(p, "creatures/"),
            Document::Item(_) => family(p, "items/"),
            Document::Dialogue(_) => family(p, "dialogue/"),
            Document::Quest(_) => family(p, "quests/"),
            Document::Faction(_) => family(p, "factions/"),
            Document::Graph(_) => family(p, "scripts/graphs/"),
            Document::Area(_) => area_file(p, "area.json"),
            Document::Placements(_) => area_file(p, "placements.json"),
            Document::Triggers(_) => area_file(p, "triggers.json"),
            Document::Variables(_) => p == "variables/campaign_state.json",
            Document::Locale(locale) => {
                locale_shape(p)
                    && p.strip_prefix("locale/")
                        .and_then(|s| s.strip_suffix(".json"))
                        .is_some_and(|stem| stem == locale.locale)
            }
        };
        // A required path must also contain its own required kind.
        let reserved = match p {
            "campaign.json" => matches!(document, Document::Campaign(_)),
            "campaign.lock" => matches!(document, Document::CampaignLock(_)),
            "assets/assets.lock" => matches!(document, Document::AssetsLock(_)),
            _ => true,
        };
        if !matches || !reserved {
            return Err(layout(
                Some(path),
                "document kind does not match its storage path",
            ));
        }
    }
    Ok(())
}

fn build_index(
    documents: &BTreeMap<SourcePath, Document>,
) -> Result<BTreeMap<Ulid, IndexEntry>, DataError> {
    let mut index: BTreeMap<Ulid, IndexEntry> = BTreeMap::new();
    for occurrence in crate::inventory::object_occurrences(documents) {
        match index.entry(occurrence.id) {
            Entry::Occupied(first) => {
                return Err(DataError::DuplicateId {
                    id: occurrence.id,
                    first: first.get().path.clone(),
                    second: occurrence.path,
                });
            }
            Entry::Vacant(entry) => {
                entry.insert(IndexEntry {
                    kind: occurrence.kind,
                    path: occurrence.path,
                    pointer: occurrence.pointer,
                });
            }
        }
    }
    Ok(index)
}

/// Structural acceptance shared by the writer and introspection.
///
/// Checks required files and case collisions, layout, document-local
/// invariants, duplicate identities, and lock consistency in the writer's
/// relative order. Engine compatibility is not checked because no engine
/// version is supplied.
pub(crate) fn structural_check(
    documents: &BTreeMap<SourcePath, Document>,
) -> Result<(), DataError> {
    check_paths(documents.keys())?;
    check_layout(documents)?;
    for document in documents.values() {
        crate::document::validate_local(document)?;
    }
    build_index(documents)?;
    check_locks(documents)?;
    Ok(())
}

fn manifest(documents: &BTreeMap<SourcePath, Document>) -> &crate::Campaign {
    documents
        .values()
        .find_map(|d| {
            if let Document::Campaign(v) = d {
                Some(v)
            } else {
                None
            }
        })
        .expect("layout verified campaign")
}

fn check_locks(documents: &BTreeMap<SourcePath, Document>) -> Result<(), DataError> {
    let campaign = manifest(documents);
    let lock = documents
        .values()
        .find_map(|d| {
            if let Document::CampaignLock(v) = d {
                Some(v)
            } else {
                None
            }
        })
        .expect("layout verified campaign lock");
    let assets = documents
        .values()
        .find_map(|d| {
            if let Document::AssetsLock(v) = d {
                Some(v)
            } else {
                None
            }
        })
        .expect("layout verified assets lock");
    crate::package::validate_coverage(&campaign.requires, lock)?;
    let actual = crate::assets_lock_digest(assets)?;
    if actual != lock.assets_lock {
        return Err(DataError::AssetsLockMismatch {
            expected: lock.assets_lock,
            actual,
        });
    }
    Ok(())
}

/// Loads supplied documents in fixed whole-input phase order without I/O.
pub fn load_campaign(
    files: &BTreeMap<SourcePath, Vec<u8>>,
    engine_version: &Version,
) -> Result<LoadedCampaign, DataError> {
    check_paths(files.keys())?;
    let mut documents = BTreeMap::new();
    for (path, bytes) in files {
        let document = crate::read_document(bytes).map_err(|mut error| {
            match &mut error {
                DataError::Malformed { path: source, .. }
                | DataError::UnsupportedSchema { path: source, .. } => *source = Some(path.clone()),
                _ => {}
            }
            error
        })?;
        documents.insert(path.clone(), document);
    }
    check_layout(&documents)?;
    let campaign = manifest(&documents);
    if !campaign.engine.matches(engine_version) {
        return Err(DataError::EngineIncompatible {
            required: campaign.engine.to_string(),
            actual: engine_version.to_string(),
        });
    }
    let index = build_index(&documents)?;
    check_locks(&documents)?;
    Ok(LoadedCampaign { documents, index })
}

/// Serializes well-formed documents, rebuilding identity independently of the index.
///
/// Checks layout, document-local invariants, duplicate ids and lock consistency.
/// Engine compatibility is not repeated because no engine version is supplied.
pub fn serialize_campaign(
    campaign: &LoadedCampaign,
) -> Result<BTreeMap<SourcePath, Vec<u8>>, DataError> {
    structural_check(&campaign.documents)?;
    campaign
        .documents
        .iter()
        .map(|(path, document)| Ok((path.clone(), crate::write_document(document)?)))
        .collect()
}
