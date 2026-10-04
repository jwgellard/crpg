//! Atomic-replace file store over the envelope (T038 §6).
//!
//! [`save_file`] writes a sibling temporary file, syncs it and renames it over
//! the destination, so a reader sees either the previous complete save or the
//! new one. [`load_file`] reads at most `MAX_ENVELOPE_BYTES + 1` bytes and
//! never trusts file metadata for the length.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::envelope::{
    decode_envelope, encode_envelope, EnvelopeError, PayloadKind, MAX_ENVELOPE_BYTES,
};

/// Suffix appended to the destination file name to form the temporary file.
pub const TEMP_SUFFIX: &str = ".crpg-tmp";

/// Filesystem step that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOp {
    /// Creating or truncating the temporary file.
    CreateTemp,
    /// Writing the envelope into the temporary file.
    WriteTemp,
    /// Flushing the temporary file to stable storage.
    SyncTemp,
    /// Renaming the temporary file over the destination.
    Rename,
    /// Flushing the parent directory entry (Unix only).
    SyncDir,
    /// Opening the save file for reading.
    Open,
    /// Reading the save file.
    Read,
}

impl fmt::Display for FileOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::CreateTemp => "create-temp",
            Self::WriteTemp => "write-temp",
            Self::SyncTemp => "sync-temp",
            Self::Rename => "rename",
            Self::SyncDir => "sync-dir",
            Self::Open => "open",
            Self::Read => "read",
        })
    }
}

/// Every file-store failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileError {
    /// Envelope encode or decode failure, including the input-size cap.
    Envelope(EnvelopeError),
    /// The path has no final file-name component (for example `""` or `".."`).
    InvalidPath,
    /// An I/O step failed; `kind` is the OS error's `std::io::ErrorKind`.
    Io {
        /// The step that failed.
        op: FileOp,
        /// The OS error's kind.
        kind: io::ErrorKind,
    },
}

impl From<EnvelopeError> for FileError {
    fn from(e: EnvelopeError) -> Self {
        FileError::Envelope(e)
    }
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Envelope(e) => fmt::Display::fmt(e, f),
            Self::InvalidPath => f.write_str("save path has no file name"),
            Self::Io { op, kind } => write!(f, "save {op} failed: {kind}"),
        }
    }
}

impl std::error::Error for FileError {}

fn io_error(op: FileOp) -> impl FnOnce(io::Error) -> FileError {
    move |e| FileError::Io { op, kind: e.kind() }
}

/// Encode `payload` and atomically replace `path` with the envelope (§6.1).
pub fn save_file(path: &Path, kind: PayloadKind, payload: &[u8]) -> Result<(), FileError> {
    // 1. A destination file name is required.
    let Some(name) = path.file_name() else {
        return Err(FileError::InvalidPath);
    };
    // 2. Encode before touching the filesystem.
    let envelope = encode_envelope(kind, payload)?;

    // 3. Sibling temp file, built with OsString so the name need not be UTF-8.
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let mut temp_name = OsString::from(name);
    temp_name.push(TEMP_SUFFIX);
    let temp: PathBuf = parent.join(temp_name);
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temp)
        .map_err(io_error(FileOp::CreateTemp))?;

    // 4–6. Write, sync, close and rename; any failure removes the temp file.
    let written = file
        .write_all(&envelope)
        .map_err(io_error(FileOp::WriteTemp))
        .and_then(|()| file.sync_all().map_err(io_error(FileOp::SyncTemp)));
    drop(file);
    let replaced = written.and_then(|()| fs::rename(&temp, path).map_err(io_error(FileOp::Rename)));
    if let Err(e) = replaced {
        // Best effort: the removal error is ignored and the step's error wins.
        let _ = fs::remove_file(&temp);
        return Err(e);
    }

    // 7. Unix only: make the directory entry durable. The new save is already
    // in place if this fails.
    #[cfg(unix)]
    File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(io_error(FileOp::SyncDir))?;

    Ok(())
}

/// Read at most `MAX_ENVELOPE_BYTES + 1` bytes from `path` and decode them (§6.2).
pub fn load_file(path: &Path, expected: PayloadKind) -> Result<Vec<u8>, FileError> {
    let file = File::open(path).map_err(io_error(FileOp::Open))?;
    let bytes = read_capped(file)?;
    Ok(decode_envelope(expected, &bytes)?)
}

/// Read at most `MAX_ENVELOPE_BYTES + 1` bytes from `reader`. Longer input is
/// `Envelope(InputTooLarge)`. The reader is never read past that bound.
pub fn read_capped<R: Read>(reader: R) -> Result<Vec<u8>, FileError> {
    let mut buf = Vec::new();
    reader
        .take(MAX_ENVELOPE_BYTES as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(io_error(FileOp::Read))?;
    if buf.len() > MAX_ENVELOPE_BYTES {
        return Err(FileError::Envelope(EnvelopeError::InputTooLarge));
    }
    Ok(buf)
}
