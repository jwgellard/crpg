//! Every editing failure. A failed call changes nothing.

use std::fmt;

/// Which input cap was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditLimit {
    /// `MAX_POINTER_BYTES`.
    Pointer,
    /// `MAX_VALUE_BYTES`.
    Value,
    /// `MAX_DOCUMENT_BYTES`.
    Document,
}

impl fmt::Display for EditLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pointer => "pointer",
            Self::Value => "value",
            Self::Document => "document",
        })
    }
}

/// Every editing failure. A failed call changes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// The opened campaign fails the writer's structural checks.
    Invalid {
        /// The structural failure, converted by `diagnostic_for_data_error`.
        diagnostic: crpg_data::Diagnostic,
    },
    /// A batch is empty or longer than `MAX_BATCH_COMMANDS`.
    BatchSize {
        /// Number of commands supplied.
        len: usize,
    },
    /// Command `index` exceeds an input cap.
    InputTooLarge {
        /// Zero-based position of the command in its batch.
        index: usize,
        /// The cap that was exceeded.
        limit: EditLimit,
    },
    /// Command `index` names a path with no document.
    MissingPath {
        /// Zero-based position of the command in its batch.
        index: usize,
        /// The path with no document.
        path: crpg_data::SourcePath,
    },
    /// Command `index` would create or rename onto an occupied path.
    PathExists {
        /// Zero-based position of the command in its batch.
        index: usize,
        /// The occupied path.
        path: crpg_data::SourcePath,
    },
    /// Command `index` names an id absent from the index.
    UnknownObject {
        /// Zero-based position of the command in its batch.
        index: usize,
        /// The unknown identity.
        id: crpg_core::Ulid,
    },
    /// Command `index` names an object of the wrong kind.
    WrongKind {
        /// Zero-based position of the command in its batch.
        index: usize,
        /// The named identity.
        id: crpg_core::Ulid,
        /// The kind the command requires.
        expected: crpg_data::ObjectKind,
        /// The kind the index records.
        found: crpg_data::ObjectKind,
    },
    /// Command `index` touches a protected document or member.
    Protected {
        /// Zero-based position of the command in its batch.
        index: usize,
        /// The protected document's path.
        path: crpg_data::SourcePath,
        /// The absolute pointer edited, or empty for a whole document.
        pointer: String,
    },
    /// `crpg_data::edit_document` refused command `index`'s pointer edit.
    Pointer {
        /// Zero-based position of the command in its batch.
        index: usize,
        /// The edited document's path.
        path: crpg_data::SourcePath,
        /// Why the edit was refused.
        error: crpg_data::PointerEditError,
    },
    /// Command `index` would change the id of an existing object.
    IdentityChanged {
        /// Zero-based position of the command in its batch.
        index: usize,
        /// The document whose identities would change.
        path: crpg_data::SourcePath,
        /// Lexically smallest identity that would appear or disappear.
        id: crpg_core::Ulid,
    },
    /// Command `index` leaves the campaign structurally invalid.
    Rejected {
        /// Zero-based position of the command in its batch.
        index: usize,
        /// The structural failure, converted by `diagnostic_for_data_error`.
        diagnostic: crpg_data::Diagnostic,
    },
    /// The committed change would exceed `MAX_HISTORY_BYTES` on its own.
    HistoryTooLarge {
        /// Size of the change's undo entry.
        bytes: usize,
    },
    /// The undo stack is empty.
    NothingToUndo,
    /// The redo stack is empty.
    NothingToRedo,
    /// `mark_saved` received a plan from another revision.
    StaleSavePlan,
    /// A history entry failed to re-read or re-index (unreachable; documented).
    HistoryCorrupt,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid { diagnostic } => {
                write!(f, "campaign cannot be opened for editing: {diagnostic}")
            }
            Self::BatchSize { len } => write!(
                f,
                "command batch has {len} commands; expected 1 to {}",
                crate::MAX_BATCH_COMMANDS
            ),
            Self::InputTooLarge { index, limit } => {
                write!(f, "command {index}: {limit} exceeds its byte cap")
            }
            Self::MissingPath { index, path } => {
                write!(f, "command {index}: no document at {}", path.as_str())
            }
            Self::PathExists { index, path } => write!(
                f,
                "command {index}: a document already exists at {}",
                path.as_str()
            ),
            Self::UnknownObject { index, id } => {
                write!(f, "command {index}: unknown object {id}")
            }
            Self::WrongKind { index, id, .. } => write!(
                f,
                "command {index}: object {id} has the wrong kind for this command"
            ),
            Self::Protected {
                index,
                path,
                pointer,
            } => write!(
                f,
                "command {index}: {}{pointer} is protected",
                path.as_str()
            ),
            Self::Pointer { index, path, error } => {
                write!(f, "command {index}: {}: {error}", path.as_str())
            }
            Self::IdentityChanged { index, path, id } => write!(
                f,
                "command {index}: edit would change the identity of object {id} in {}",
                path.as_str()
            ),
            Self::Rejected { index, diagnostic } => {
                write!(f, "command {index}: rejected: {diagnostic}")
            }
            Self::HistoryTooLarge { bytes } => {
                write!(f, "change of {bytes} bytes exceeds the undo history cap")
            }
            Self::NothingToUndo => f.write_str("nothing to undo"),
            Self::NothingToRedo => f.write_str("nothing to redo"),
            Self::StaleSavePlan => f.write_str("save plan is stale"),
            Self::HistoryCorrupt => f.write_str("undo history is corrupt"),
        }
    }
}

impl std::error::Error for EditError {}
