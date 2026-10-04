#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Headless editor document model: an open, always structurally valid
//! campaign, the closed `EditCommand` set, atomic apply with receipts,
//! bounded byte-exact undo and redo, `crpg-data` validation, and a pure save
//! plan. Callers own all I/O and supply every new object id.

pub mod command;
pub mod document;
pub mod error;
mod history;

pub use command::{EditCommand, EditTarget};
pub use document::{
    CampaignDocument, ChangeKind, CommandReceipt, PathChange, SavePlan, MAX_BATCH_COMMANDS,
    MAX_DOCUMENT_BYTES, MAX_HISTORY_BYTES, MAX_POINTER_BYTES, MAX_UNDO_ENTRIES, MAX_VALUE_BYTES,
};
pub use error::{EditError, EditLimit};
