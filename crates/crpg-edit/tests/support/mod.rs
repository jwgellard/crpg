//! Test-only helpers shared by the crpg-edit suites. Fixture bytes are
//! embedded read-only; no test writes any file.
#![allow(dead_code)]

use crpg_core::Ulid;
use crpg_data::{load_campaign, Diagnostic, Document, LoadedCampaign, SourcePath, Transform};
use crpg_edit::{CampaignDocument, ChangeKind, PathChange, SavePlan};
use std::collections::BTreeMap;

/// The ten files of `crpg-data`'s `one_area_one_creature` fixture, by explicit path.
const FIXTURE: [(&str, &[u8]); 10] = [
    (
        "areas/start/area.json",
        include_bytes!(
            "../../../crpg-data/tests/fixtures/one_area_one_creature/areas/start/area.json"
        ),
    ),
    (
        "areas/start/placements.json",
        include_bytes!(
            "../../../crpg-data/tests/fixtures/one_area_one_creature/areas/start/placements.json"
        ),
    ),
    (
        "areas/start/triggers.json",
        include_bytes!(
            "../../../crpg-data/tests/fixtures/one_area_one_creature/areas/start/triggers.json"
        ),
    ),
    (
        "assets/assets.lock",
        include_bytes!(
            "../../../crpg-data/tests/fixtures/one_area_one_creature/assets/assets.lock"
        ),
    ),
    (
        "campaign.json",
        include_bytes!("../../../crpg-data/tests/fixtures/one_area_one_creature/campaign.json"),
    ),
    (
        "campaign.lock",
        include_bytes!("../../../crpg-data/tests/fixtures/one_area_one_creature/campaign.lock"),
    ),
    (
        "creatures/creature.json",
        include_bytes!(
            "../../../crpg-data/tests/fixtures/one_area_one_creature/creatures/creature.json"
        ),
    ),
    (
        "locale/en.json",
        include_bytes!("../../../crpg-data/tests/fixtures/one_area_one_creature/locale/en.json"),
    ),
    (
        "variables/campaign_state.json",
        include_bytes!(
            "../../../crpg-data/tests/fixtures/one_area_one_creature/variables/campaign_state.json"
        ),
    ),
    (
        "worlds/world.json",
        include_bytes!("../../../crpg-data/tests/fixtures/one_area_one_creature/worlds/world.json"),
    ),
];

/// The fixture's file bytes by logical path.
pub fn fixture_files() -> BTreeMap<SourcePath, Vec<u8>> {
    FIXTURE
        .iter()
        .map(|(path, bytes)| (sp(path), bytes.to_vec()))
        .collect()
}

/// Loads a file map at engine `0.1.0`.
pub fn load(files: &BTreeMap<SourcePath, Vec<u8>>) -> LoadedCampaign {
    load_campaign(files, &"0.1.0".parse().unwrap()).expect("campaign loads")
}

/// `crpg_data::validate_files` at engine `0.1.0`.
pub fn validate_files(files: &BTreeMap<SourcePath, Vec<u8>>) -> Vec<Diagnostic> {
    crpg_data::validate_files(files, &"0.1.0".parse().unwrap())
}

/// The fixture campaign, loaded.
pub fn fixture() -> LoadedCampaign {
    load(&fixture_files())
}

/// A freshly opened fixture document.
pub fn open() -> CampaignDocument {
    CampaignDocument::open(fixture()).expect("fixture opens")
}

/// Fixture ids: 1 campaign, 2 world, 3 area, 4 creature, 5 spawn placement.
pub fn id(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

/// A logical path.
pub fn sp(path: &str) -> SourcePath {
    path.parse().expect("valid logical path")
}

/// JSON text of a string value (test inputs use no characters needing escapes).
pub fn json_str(text: &str) -> Vec<u8> {
    format!("\"{text}\"").into_bytes()
}

/// The fixture's typed document at `path`.
pub fn fixture_document(path: &str) -> Document {
    fixture().documents[&sp(path)].clone()
}

/// The identity transform with `x` whole units of position.
pub fn transform(x: i32) -> Transform {
    let one = crpg_core::Fx16_16::from_raw(65_536);
    let zero = crpg_core::Fx16_16::from_raw(0);
    Transform {
        position: [crpg_core::Fx16_16::from_raw(x * 65_536), zero, zero],
        rotation: [zero, zero, zero],
        scale: [one, one, one],
    }
}

/// Everything §5.4 says a failed call leaves exactly as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub files: BTreeMap<SourcePath, Vec<u8>>,
    pub campaign: LoadedCampaign,
    pub revision: u64,
    pub undo_depth: usize,
    pub redo_depth: usize,
    pub history_bytes: usize,
    pub save_plan: SavePlan,
}

/// Captures a document's observable state.
pub fn snapshot(doc: &CampaignDocument) -> Snapshot {
    Snapshot {
        files: doc.canonical_files().clone(),
        campaign: doc.campaign().clone(),
        revision: doc.revision(),
        undo_depth: doc.undo_depth(),
        redo_depth: doc.redo_depth(),
        history_bytes: doc.history_bytes(),
        save_plan: doc.save_plan(),
    }
}

/// Asserts the atomicity guarantee against an earlier snapshot.
pub fn assert_unchanged(before: &Snapshot, doc: &CampaignDocument) {
    assert_eq!(before, &snapshot(doc), "a failed call changed the document");
}

/// The changed paths between two canonical maps, computed independently of
/// the crate under test.
pub fn diff(
    before: &BTreeMap<SourcePath, Vec<u8>>,
    after: &BTreeMap<SourcePath, Vec<u8>>,
) -> Vec<PathChange> {
    let mut changes = Vec::new();
    for (path, old) in before {
        match after.get(path) {
            None => changes.push(PathChange {
                path: path.clone(),
                kind: ChangeKind::Deleted,
            }),
            Some(new) if new != old => changes.push(PathChange {
                path: path.clone(),
                kind: ChangeKind::Modified,
            }),
            Some(_) => {}
        }
    }
    for path in after.keys() {
        if !before.contains_key(path) {
            changes.push(PathChange {
                path: path.clone(),
                kind: ChangeKind::Created,
            });
        }
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    changes
}

/// Applies a save plan to a file map, as a caller's writer would.
pub fn apply_plan(
    baseline: &BTreeMap<SourcePath, Vec<u8>>,
    plan: &SavePlan,
) -> BTreeMap<SourcePath, Vec<u8>> {
    let mut files = baseline.clone();
    for path in &plan.remove {
        files.remove(path);
    }
    for (path, bytes) in &plan.write {
        files.insert(path.clone(), bytes.clone());
    }
    files
}

/// Shorthand for one change.
pub fn change(path: &str, kind: ChangeKind) -> PathChange {
    PathChange {
        path: sp(path),
        kind,
    }
}
