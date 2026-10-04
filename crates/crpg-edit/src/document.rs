//! The open campaign document: atomic apply, undo and redo, validation and
//! the pure save plan (T058 §5–§7).
//!
//! Every structural decision is `crpg-data`'s: pointer edits go through
//! `edit_document`, the index and the writer's structural checks come from
//! `campaign_index`, canonical bytes from `write_document`, and findings from
//! `validate`. This module only resolves commands, enforces protection,
//! identity and the input caps, and keeps the canonical-byte history.

use crate::command::{EditCommand, EditTarget};
use crate::error::{EditError, EditLimit};
use crate::history::{Entry, History, PathBytes};
use crpg_core::Ulid;
use crpg_data::{
    campaign_index, canonical_json, diagnostic_for_data_error, edit_document, pointer_tokens,
    read_document, write_document, DataError, Diagnostic, Document, IndexEntry, LoadedCampaign,
    ObjectKind, Placement, PlacementsDocument, PointerEdit, PointerEditError, SourcePath,
    Transform,
};
use std::collections::{BTreeMap, BTreeSet};

/// Most commands in one `apply_batch` call.
pub const MAX_BATCH_COMMANDS: usize = 1_024;
/// Longest caller-supplied pointer, in bytes (before any object prefix).
pub const MAX_POINTER_BYTES: usize = 4_096;
/// Longest caller-supplied JSON value text, in bytes: 1 MiB.
pub const MAX_VALUE_BYTES: usize = 1_048_576;
/// Largest canonical document a command may create or leave behind: 4 MiB.
pub const MAX_DOCUMENT_BYTES: usize = 4_194_304;
/// Most undo entries kept; the oldest is evicted first.
pub const MAX_UNDO_ENTRIES: usize = 256;
/// Most bytes held by undo plus redo entries: 32 MiB (§6.3 defines the measure).
pub const MAX_HISTORY_BYTES: usize = 33_554_432;

/// Derived lock documents owned by `crpgc lock` and the assets pipeline.
const LOCK_PATHS: [&str; 2] = ["campaign.lock", "assets/assets.lock"];
/// The manifest whose package members are read-only to commands.
const MANIFEST_PATH: &str = "campaign.json";
/// First reference tokens of the manifest that commands may not edit.
const MANIFEST_PROTECTED: [&str; 3] = ["package", "engine", "requires"];

/// An open, editable, always structurally valid campaign.
#[derive(Debug, Clone)]
pub struct CampaignDocument {
    campaign: LoadedCampaign,
    canonical: BTreeMap<SourcePath, Vec<u8>>,
    baseline: BTreeMap<SourcePath, Vec<u8>>,
    revision: u64,
    history: History,
}

/// How one path changed across a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeKind {
    /// Absent before, present after.
    Created,
    /// Present before and after, with different bytes.
    Modified,
    /// Present before, absent after.
    Deleted,
}

/// One changed path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathChange {
    /// Logical path.
    pub path: SourcePath,
    /// What happened to it.
    pub kind: ChangeKind,
}

/// The result of a committed (or no-op) apply, undo or redo; also the view
/// change notification (spec §11.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandReceipt {
    /// `revision()` after the call.
    pub revision: u64,
    /// Every path whose bytes differ between before and after the call, in
    /// lexical path order. Empty for a no-op.
    pub changes: Vec<PathChange>,
    /// `crpg_data::validate` over the campaign after the call.
    pub diagnostics: Vec<Diagnostic>,
}

/// Pure save instructions; the caller owns all I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavePlan {
    /// Revision the plan was computed at.
    pub revision: u64,
    /// Paths to (re)write with these exact canonical bytes.
    pub write: BTreeMap<SourcePath, Vec<u8>>,
    /// Paths present in the baseline and absent now.
    pub remove: BTreeSet<SourcePath>,
}

/// The batch's private working copy; discarded on any failure.
struct Working {
    documents: BTreeMap<SourcePath, Document>,
    index: BTreeMap<Ulid, IndexEntry>,
    canonical: BTreeMap<SourcePath, Vec<u8>>,
}

/// What a resolved command does to the working documents.
enum Mutation {
    /// Insert a whole document at an unused path.
    Insert {
        path: SourcePath,
        document: Document,
    },
    /// Remove the document at an existing path.
    Remove { path: SourcePath },
    /// Move a document unchanged.
    Rename { from: SourcePath, to: SourcePath },
    /// One `edit_document` call. `relative` holds the decoder's refusal of an
    /// object-relative pointer, checked before the index pointer was prefixed.
    Edit {
        path: SourcePath,
        edit: PointerEdit,
        relative: Option<PointerEditError>,
    },
    /// Append a placement to an existing aggregate.
    AppendPlacement {
        path: SourcePath,
        placement: Placement,
    },
    /// Replace a placement's transform.
    SetTransform {
        path: SourcePath,
        pointer: String,
        transform: Transform,
    },
}

impl Mutation {
    /// Every path the mutation writes, creates or removes.
    fn touched(&self) -> Vec<SourcePath> {
        match self {
            Self::Insert { path, .. }
            | Self::Remove { path }
            | Self::Edit { path, .. }
            | Self::AppendPlacement { path, .. }
            | Self::SetTransform { path, .. } => vec![path.clone()],
            Self::Rename { from, to } => vec![from.clone(), to.clone()],
        }
    }
}

/// How a command's touched paths must keep their `(id, kind)` sets (§5.3 step 7).
enum IdentityRule {
    /// `CreateDocument`: step 6's duplicate-id check covers it.
    Unchecked,
    /// Old set equals new set.
    Equal,
    /// Old set is a subset of the new set.
    Grow,
    /// New set is a subset of the old set.
    Shrink,
    /// The old set at `from` equals the new set at `to`.
    Rename { from: SourcePath, to: SourcePath },
}

/// A command after resolution against the working state.
struct Resolved {
    mutation: Mutation,
    rule: IdentityRule,
    /// The path and absolute pointer of a `SetValue`/`InsertValue`/
    /// `RemoveValue` on `campaign.json`, for the manifest protection check.
    manifest_pointer: Option<(SourcePath, String)>,
}

fn rejected(index: usize, error: &DataError) -> EditError {
    EditError::Rejected {
        index,
        diagnostic: diagnostic_for_data_error(error),
    }
}

fn invalid(error: &DataError) -> EditError {
    EditError::Invalid {
        diagnostic: diagnostic_for_data_error(error),
    }
}

/// Paths whose bytes differ between two canonical maps, in lexical order.
fn diff(
    before: &BTreeMap<SourcePath, Vec<u8>>,
    after: &BTreeMap<SourcePath, Vec<u8>>,
) -> Vec<PathChange> {
    let paths: BTreeSet<&SourcePath> = before.keys().chain(after.keys()).collect();
    paths
        .into_iter()
        .filter_map(|path| {
            let kind = match (before.get(path), after.get(path)) {
                (None, Some(_)) => ChangeKind::Created,
                (Some(_), None) => ChangeKind::Deleted,
                (Some(old), Some(new)) if old != new => ChangeKind::Modified,
                _ => return None,
            };
            Some(PathChange {
                path: path.clone(),
                kind,
            })
        })
        .collect()
}

/// The `(id, kind)` entries an index records at `path`.
fn identities(
    index: &BTreeMap<Ulid, IndexEntry>,
    path: &SourcePath,
) -> BTreeSet<(Ulid, ObjectKind)> {
    index
        .iter()
        .filter(|(_, entry)| &entry.path == path)
        .map(|(id, entry)| (*id, entry.kind))
        .collect()
}

/// The lexically smallest ULID in the symmetric difference of two sets.
fn smallest_changed(
    old: &BTreeSet<(Ulid, ObjectKind)>,
    new: &BTreeSet<(Ulid, ObjectKind)>,
) -> Option<Ulid> {
    old.symmetric_difference(new).map(|(id, _)| *id).min()
}

impl CampaignDocument {
    /// Opens a loaded campaign for editing (§5.1). The supplied index is ignored.
    pub fn open(campaign: LoadedCampaign) -> Result<Self, EditError> {
        let documents = campaign.documents;
        let index = campaign_index(&documents).map_err(|e| invalid(&e))?;
        let mut canonical = BTreeMap::new();
        for (path, document) in &documents {
            let bytes = write_document(document).map_err(|e| invalid(&e))?;
            canonical.insert(path.clone(), bytes);
        }
        Ok(Self {
            campaign: LoadedCampaign { documents, index },
            baseline: canonical.clone(),
            canonical,
            revision: 0,
            history: History::default(),
        })
    }

    /// The current documents and their freshly derived index.
    pub fn campaign(&self) -> &LoadedCampaign {
        &self.campaign
    }

    /// Current canonical bytes of every document, by logical path.
    pub fn canonical_files(&self) -> &BTreeMap<SourcePath, Vec<u8>> {
        &self.canonical
    }

    /// 0 at open; +1 for every committed apply, undo or redo.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Applies one command atomically. Same as `apply_batch(vec![command])`.
    pub fn apply(&mut self, command: EditCommand) -> Result<CommandReceipt, EditError> {
        self.apply_batch(vec![command])
    }

    /// Applies 1..=MAX_BATCH_COMMANDS commands atomically as one undo entry (§5.3).
    pub fn apply_batch(&mut self, commands: Vec<EditCommand>) -> Result<CommandReceipt, EditError> {
        if commands.is_empty() || commands.len() > MAX_BATCH_COMMANDS {
            return Err(EditError::BatchSize {
                len: commands.len(),
            });
        }
        let mut work = Working {
            documents: self.campaign.documents.clone(),
            index: self.campaign.index.clone(),
            canonical: self.canonical.clone(),
        };
        for (index, command) in commands.into_iter().enumerate() {
            apply_one(&mut work, index, command)?;
        }

        let changes = diff(&self.canonical, &work.canonical);
        if changes.is_empty() {
            return Ok(CommandReceipt {
                revision: self.revision,
                changes,
                diagnostics: self.validate(),
            });
        }
        let entry = Entry::new(
            changes
                .iter()
                .map(|change| PathBytes {
                    path: change.path.clone(),
                    before: self.canonical.get(&change.path).cloned(),
                    after: work.canonical.get(&change.path).cloned(),
                })
                .collect(),
        );
        if entry.bytes > MAX_HISTORY_BYTES {
            return Err(EditError::HistoryTooLarge { bytes: entry.bytes });
        }
        self.history.commit(entry);
        let campaign = LoadedCampaign {
            documents: work.documents,
            index: work.index,
        };
        Ok(self.install(campaign, work.canonical, changes))
    }

    /// Reverts the newest undo entry (§6).
    pub fn undo(&mut self) -> Result<CommandReceipt, EditError> {
        let entry = self.history.newest_undo().ok_or(EditError::NothingToUndo)?;
        let (campaign, canonical) = self.restore(entry, |p| p.before.as_ref())?;
        self.history.move_to_redo();
        let changes = diff(&self.canonical, &canonical);
        Ok(self.install(campaign, canonical, changes))
    }

    /// Re-applies the newest redo entry (§6).
    pub fn redo(&mut self) -> Result<CommandReceipt, EditError> {
        let entry = self.history.newest_redo().ok_or(EditError::NothingToRedo)?;
        let (campaign, canonical) = self.restore(entry, |p| p.after.as_ref())?;
        self.history.move_to_undo();
        let changes = diff(&self.canonical, &canonical);
        Ok(self.install(campaign, canonical, changes))
    }

    /// Number of undo entries.
    pub fn undo_depth(&self) -> usize {
        self.history.undo_depth()
    }

    /// Number of redo entries.
    pub fn redo_depth(&self) -> usize {
        self.history.redo_depth()
    }

    /// Bytes held by undo plus redo entries (§6.3).
    pub fn history_bytes(&self) -> usize {
        self.history.bytes()
    }

    /// `crpg_data::validate` over the current campaign; nothing else.
    pub fn validate(&self) -> Vec<Diagnostic> {
        crpg_data::validate(&self.campaign)
    }

    /// The writes and removals that bring the saved baseline to the current state (§7).
    pub fn save_plan(&self) -> SavePlan {
        let write = self
            .canonical
            .iter()
            .filter(|(path, bytes)| self.baseline.get(*path) != Some(*bytes))
            .map(|(path, bytes)| (path.clone(), bytes.clone()))
            .collect();
        let remove = self
            .baseline
            .keys()
            .filter(|path| !self.canonical.contains_key(*path))
            .cloned()
            .collect();
        SavePlan {
            revision: self.revision,
            write,
            remove,
        }
    }

    /// Records that `plan` was written; it becomes the new saved baseline (§7).
    pub fn mark_saved(&mut self, plan: &SavePlan) -> Result<(), EditError> {
        if plan.revision != self.revision {
            return Err(EditError::StaleSavePlan);
        }
        self.baseline = self.canonical.clone();
        Ok(())
    }

    /// Builds the state one history side describes, without committing it.
    fn restore(
        &self,
        entry: &Entry,
        side: impl Fn(&PathBytes) -> Option<&Vec<u8>>,
    ) -> Result<(LoadedCampaign, BTreeMap<SourcePath, Vec<u8>>), EditError> {
        let mut documents = self.campaign.documents.clone();
        let mut canonical = self.canonical.clone();
        for path_bytes in &entry.paths {
            match side(path_bytes) {
                Some(bytes) => {
                    let document = read_document(bytes).map_err(|_| EditError::HistoryCorrupt)?;
                    documents.insert(path_bytes.path.clone(), document);
                    canonical.insert(path_bytes.path.clone(), bytes.clone());
                }
                None => {
                    documents.remove(&path_bytes.path);
                    canonical.remove(&path_bytes.path);
                }
            }
        }
        let index = campaign_index(&documents).map_err(|_| EditError::HistoryCorrupt)?;
        Ok((LoadedCampaign { documents, index }, canonical))
    }

    /// Installs a new state, bumps the revision and builds the receipt.
    fn install(
        &mut self,
        campaign: LoadedCampaign,
        canonical: BTreeMap<SourcePath, Vec<u8>>,
        changes: Vec<PathChange>,
    ) -> CommandReceipt {
        self.campaign = campaign;
        self.canonical = canonical;
        self.revision = self.revision.saturating_add(1);
        CommandReceipt {
            revision: self.revision,
            changes,
            diagnostics: self.validate(),
        }
    }
}

/// §5.3 steps 1–8 for one command against the working state.
fn apply_one(work: &mut Working, index: usize, command: EditCommand) -> Result<(), EditError> {
    check_caps(index, &command)?;
    let resolved = resolve(work, index, command)?;
    check_protection(index, &resolved)?;
    let touched = resolved.mutation.touched();
    mutate(work, index, resolved.mutation)?;

    // Step 5: document cap over every touched document still present.
    for path in &touched {
        match work.documents.get(path) {
            Some(document) => {
                let bytes = write_document(document).map_err(|e| rejected(index, &e))?;
                if bytes.len() > MAX_DOCUMENT_BYTES {
                    return Err(EditError::InputTooLarge {
                        index,
                        limit: EditLimit::Document,
                    });
                }
                work.canonical.insert(path.clone(), bytes);
            }
            None => {
                work.canonical.remove(path);
            }
        }
    }

    // Step 6: the writer's structural acceptance and the fresh index.
    let new_index = campaign_index(&work.documents).map_err(|e| rejected(index, &e))?;

    // Step 7: identity.
    check_identity(&work.index, &new_index, index, &resolved.rule, &touched)?;

    // Step 8.
    work.index = new_index;
    Ok(())
}

/// §5.3 step 1.
fn check_caps(index: usize, command: &EditCommand) -> Result<(), EditError> {
    let too_large = |limit| EditError::InputTooLarge { index, limit };
    match command {
        EditCommand::SetValue { pointer, value, .. }
        | EditCommand::InsertValue { pointer, value, .. } => {
            if pointer.len() > MAX_POINTER_BYTES {
                return Err(too_large(EditLimit::Pointer));
            }
            if value.len() > MAX_VALUE_BYTES {
                return Err(too_large(EditLimit::Value));
            }
        }
        EditCommand::RemoveValue { pointer, .. } => {
            if pointer.len() > MAX_POINTER_BYTES {
                return Err(too_large(EditLimit::Pointer));
            }
        }
        EditCommand::CreateDocument { document, .. } => {
            let bytes = write_document(document).map_err(|e| rejected(index, &e))?;
            if bytes.len() > MAX_DOCUMENT_BYTES {
                return Err(too_large(EditLimit::Document));
            }
        }
        EditCommand::DeleteDocument { .. }
        | EditCommand::RenameDocument { .. }
        | EditCommand::DeleteObject { .. }
        | EditCommand::PlaceInstance { .. }
        | EditCommand::MovePlacement { .. } => {}
    }
    Ok(())
}

/// The index entry for `id`, or `UnknownObject`.
fn entry(work: &Working, index: usize, id: Ulid) -> Result<&IndexEntry, EditError> {
    work.index
        .get(&id)
        .ok_or(EditError::UnknownObject { index, id })
}

/// The index entry for `id`, required to be of `expected` kind.
fn entry_of_kind(
    work: &Working,
    index: usize,
    id: Ulid,
    expected: ObjectKind,
) -> Result<&IndexEntry, EditError> {
    let found = entry(work, index, id)?;
    if found.kind != expected {
        return Err(EditError::WrongKind {
            index,
            id,
            expected,
            found: found.kind,
        });
    }
    Ok(found)
}

/// Requires a document at `path`.
fn existing(work: &Working, index: usize, path: &SourcePath) -> Result<(), EditError> {
    if work.documents.contains_key(path) {
        Ok(())
    } else {
        Err(EditError::MissingPath {
            index,
            path: path.clone(),
        })
    }
}

/// Requires no document at `path`.
fn unused(work: &Working, index: usize, path: &SourcePath) -> Result<(), EditError> {
    if work.documents.contains_key(path) {
        Err(EditError::PathExists {
            index,
            path: path.clone(),
        })
    } else {
        Ok(())
    }
}

/// A pointer edit's document path, absolute pointer and relative-pointer refusal.
fn resolve_target(
    work: &Working,
    index: usize,
    target: &EditTarget,
    pointer: String,
) -> Result<(SourcePath, String, Option<PointerEditError>), EditError> {
    match target {
        EditTarget::Document(path) => {
            existing(work, index, path)?;
            Ok((path.clone(), pointer, None))
        }
        EditTarget::Object(id) => {
            let found = entry(work, index, *id)?;
            // Decode the relative pointer on its own first, so text such as
            // `slug` is refused rather than read as part of the prefix.
            let relative = pointer_tokens(&pointer).err();
            Ok((
                found.path.clone(),
                format!("{}{pointer}", found.pointer),
                relative,
            ))
        }
    }
}

/// A `SetValue`/`InsertValue`/`RemoveValue` after resolution.
fn pointer_command(
    work: &Working,
    index: usize,
    target: &EditTarget,
    pointer: String,
    make: impl FnOnce(String) -> PointerEdit,
    rule: IdentityRule,
) -> Result<Resolved, EditError> {
    let (path, absolute, relative) = resolve_target(work, index, target, pointer)?;
    let manifest_pointer =
        (path.as_str() == MANIFEST_PATH).then(|| (path.clone(), absolute.clone()));
    Ok(Resolved {
        mutation: Mutation::Edit {
            path,
            edit: make(absolute),
            relative,
        },
        rule,
        manifest_pointer,
    })
}

/// §5.3 step 2.
fn resolve(work: &Working, index: usize, command: EditCommand) -> Result<Resolved, EditError> {
    let plain = |mutation, rule| Resolved {
        mutation,
        rule,
        manifest_pointer: None,
    };
    match command {
        EditCommand::CreateDocument { path, document } => {
            unused(work, index, &path)?;
            Ok(plain(
                Mutation::Insert { path, document },
                IdentityRule::Unchecked,
            ))
        }
        EditCommand::DeleteDocument { path } => {
            existing(work, index, &path)?;
            Ok(plain(Mutation::Remove { path }, IdentityRule::Shrink))
        }
        EditCommand::RenameDocument { from, to } => {
            existing(work, index, &from)?;
            unused(work, index, &to)?;
            Ok(plain(
                Mutation::Rename {
                    from: from.clone(),
                    to: to.clone(),
                },
                IdentityRule::Rename { from, to },
            ))
        }
        EditCommand::SetValue {
            target,
            pointer,
            value,
        } => pointer_command(
            work,
            index,
            &target,
            pointer,
            |pointer| PointerEdit::Set { pointer, value },
            IdentityRule::Equal,
        ),
        EditCommand::InsertValue {
            target,
            pointer,
            value,
        } => pointer_command(
            work,
            index,
            &target,
            pointer,
            |pointer| PointerEdit::Insert { pointer, value },
            IdentityRule::Grow,
        ),
        EditCommand::RemoveValue { target, pointer } => pointer_command(
            work,
            index,
            &target,
            pointer,
            |pointer| PointerEdit::Remove { pointer },
            IdentityRule::Shrink,
        ),
        EditCommand::DeleteObject { id } => {
            let found = entry(work, index, id)?;
            let path = found.path.clone();
            let mutation = if found.pointer.is_empty() {
                Mutation::Remove { path }
            } else {
                Mutation::Edit {
                    path,
                    edit: PointerEdit::Remove {
                        pointer: found.pointer.clone(),
                    },
                    relative: None,
                }
            };
            Ok(plain(mutation, IdentityRule::Shrink))
        }
        EditCommand::PlaceInstance { area, placement } => {
            let found = entry_of_kind(work, index, area, ObjectKind::Area)?;
            // Layout guarantees an area lives at `areas/<dir>/area.json`.
            let directory = found
                .path
                .as_str()
                .strip_suffix("area.json")
                .unwrap_or_default();
            let path: SourcePath = format!("{directory}placements.json")
                .parse()
                .map_err(|e| rejected(index, &e))?;
            let mutation = if work.documents.contains_key(&path) {
                Mutation::AppendPlacement { path, placement }
            } else {
                Mutation::Insert {
                    path,
                    document: Document::Placements(PlacementsDocument {
                        area,
                        placements: vec![placement],
                        note: None,
                    }),
                }
            };
            Ok(plain(mutation, IdentityRule::Grow))
        }
        EditCommand::MovePlacement {
            placement,
            transform,
        } => {
            let found = entry_of_kind(work, index, placement, ObjectKind::Placement)?;
            Ok(plain(
                Mutation::SetTransform {
                    path: found.path.clone(),
                    pointer: format!("{}/transform", found.pointer),
                    transform,
                },
                IdentityRule::Equal,
            ))
        }
    }
}

/// §5.3 step 3 (§5.2).
fn check_protection(index: usize, resolved: &Resolved) -> Result<(), EditError> {
    let protected = |path: &SourcePath, pointer: String| EditError::Protected {
        index,
        path: path.clone(),
        pointer,
    };
    for path in resolved.mutation.touched() {
        if LOCK_PATHS.contains(&path.as_str()) {
            return Err(protected(&path, String::new()));
        }
    }
    if let Mutation::Insert {
        path,
        document: Document::CampaignLock(_) | Document::AssetsLock(_),
    } = &resolved.mutation
    {
        return Err(protected(path, String::new()));
    }
    if let Some((path, pointer)) = &resolved.manifest_pointer {
        // A pointer the decoder refuses is not protected; `edit_document`
        // reports the same refusal in step 4.
        if let Ok(tokens) = pointer_tokens(pointer) {
            if tokens
                .first()
                .is_some_and(|first| MANIFEST_PROTECTED.contains(&first.as_str()))
            {
                return Err(protected(path, pointer.clone()));
            }
        }
    }
    Ok(())
}

/// Runs one `edit_document` call and installs its result.
fn edit_in_place(
    work: &mut Working,
    index: usize,
    path: SourcePath,
    edit: &PointerEdit,
) -> Result<(), EditError> {
    let Some(document) = work.documents.get(&path) else {
        return Err(EditError::MissingPath { index, path });
    };
    match edit_document(document, edit) {
        Ok(edited) => {
            work.documents.insert(path, edited);
            Ok(())
        }
        Err(error) => Err(EditError::Pointer { index, path, error }),
    }
}

/// §5.3 step 4.
fn mutate(work: &mut Working, index: usize, mutation: Mutation) -> Result<(), EditError> {
    match mutation {
        Mutation::Insert { path, document } => {
            work.documents.insert(path, document);
        }
        Mutation::Remove { path } => {
            work.documents.remove(&path);
        }
        Mutation::Rename { from, to } => {
            if let Some(document) = work.documents.remove(&from) {
                work.documents.insert(to, document);
            }
        }
        Mutation::Edit {
            path,
            edit,
            relative,
        } => {
            if let Some(error) = relative {
                return Err(EditError::Pointer { index, path, error });
            }
            edit_in_place(work, index, path, &edit)?;
        }
        Mutation::AppendPlacement { path, placement } => {
            let value = canonical_json(&placement).map_err(|e| rejected(index, &e))?;
            let edit = PointerEdit::Insert {
                pointer: "/placements/-".to_owned(),
                value,
            };
            edit_in_place(work, index, path, &edit)?;
        }
        Mutation::SetTransform {
            path,
            pointer,
            transform,
        } => {
            let value = canonical_json(&transform).map_err(|e| rejected(index, &e))?;
            edit_in_place(work, index, path, &PointerEdit::Set { pointer, value })?;
        }
    }
    Ok(())
}

/// §5.3 step 7.
fn check_identity(
    old_index: &BTreeMap<Ulid, IndexEntry>,
    new_index: &BTreeMap<Ulid, IndexEntry>,
    index: usize,
    rule: &IdentityRule,
    touched: &[SourcePath],
) -> Result<(), EditError> {
    let pairs: Vec<(&SourcePath, &SourcePath)> = match rule {
        IdentityRule::Unchecked => Vec::new(),
        IdentityRule::Rename { from, to } => vec![(from, to)],
        IdentityRule::Equal | IdentityRule::Grow | IdentityRule::Shrink => {
            touched.iter().map(|path| (path, path)).collect()
        }
    };
    for (before, after) in pairs {
        let old = identities(old_index, before);
        let new = identities(new_index, after);
        let holds = match rule {
            IdentityRule::Grow => old.is_subset(&new),
            IdentityRule::Shrink => new.is_subset(&old),
            IdentityRule::Unchecked | IdentityRule::Equal | IdentityRule::Rename { .. } => {
                old == new
            }
        };
        if !holds {
            if let Some(id) = smallest_changed(&old, &new) {
                return Err(EditError::IdentityChanged {
                    index,
                    path: after.clone(),
                    id,
                });
            }
        }
    }
    Ok(())
}
