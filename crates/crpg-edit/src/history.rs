//! Bounded undo and redo stacks of canonical-byte entries (T058 §6).

use crate::document::{MAX_HISTORY_BYTES, MAX_UNDO_ENTRIES};
use crpg_data::SourcePath;
use std::collections::VecDeque;

/// One changed path: canonical bytes before and after, `None` when absent.
#[derive(Debug, Clone)]
pub(crate) struct PathBytes {
    pub(crate) path: SourcePath,
    pub(crate) before: Option<Vec<u8>>,
    pub(crate) after: Option<Vec<u8>>,
}

/// One undoable change: every path whose bytes differ, in lexical path order.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub(crate) paths: Vec<PathBytes>,
    /// Sum over `paths` of `before.len() + after.len()` (§6.3).
    pub(crate) bytes: usize,
}

impl Entry {
    pub(crate) fn new(paths: Vec<PathBytes>) -> Self {
        let bytes = paths.iter().fold(0_usize, |sum, p| {
            sum.saturating_add(p.before.as_ref().map_or(0, Vec::len))
                .saturating_add(p.after.as_ref().map_or(0, Vec::len))
        });
        Self { paths, bytes }
    }
}

/// Undo entries oldest first, redo entries with the newest undone last.
#[derive(Debug, Clone, Default)]
pub(crate) struct History {
    undo: VecDeque<Entry>,
    redo: Vec<Entry>,
    bytes: usize,
}

impl History {
    pub(crate) fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub(crate) fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn newest_undo(&self) -> Option<&Entry> {
        self.undo.back()
    }

    pub(crate) fn newest_redo(&self) -> Option<&Entry> {
        self.redo.last()
    }

    /// Commits an applied change: clears redo, pushes, then evicts the oldest
    /// undo entries until both bounds hold. The caller has already refused an
    /// entry larger than `MAX_HISTORY_BYTES`, so the new entry always survives.
    pub(crate) fn commit(&mut self, entry: Entry) {
        for dropped in self.redo.drain(..) {
            self.bytes = self.bytes.saturating_sub(dropped.bytes);
        }
        self.bytes = self.bytes.saturating_add(entry.bytes);
        self.undo.push_back(entry);
        while self.undo.len() > MAX_UNDO_ENTRIES || self.bytes > MAX_HISTORY_BYTES {
            match self.undo.pop_front() {
                Some(oldest) => self.bytes = self.bytes.saturating_sub(oldest.bytes),
                None => break,
            }
        }
    }

    /// Moves the newest undo entry to the redo stack; bytes are unchanged.
    pub(crate) fn move_to_redo(&mut self) {
        if let Some(entry) = self.undo.pop_back() {
            self.redo.push(entry);
        }
    }

    /// Moves the newest redo entry back to the undo stack; bytes are unchanged.
    pub(crate) fn move_to_undo(&mut self) {
        if let Some(entry) = self.redo.pop() {
            self.undo.push_back(entry);
        }
    }
}
