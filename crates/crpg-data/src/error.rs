//! Fail-fast structural errors; byte APIs never perform I/O.

use crate::{Digest, PackageId, SourcePath};
use crpg_core::Ulid;
use std::fmt;

/// The first structural error encountered in the documented validation order.
#[derive(Debug)]
pub enum DataError {
    /// Invalid standalone package coordinate.
    InvalidPackageId {
        /// Rejected text.
        value: String,
    },
    /// Invalid standalone logical path.
    InvalidPath {
        /// Rejected text.
        value: String,
    },
    /// Invalid standalone lowercase digest.
    InvalidDigest {
        /// Rejected text.
        value: String,
    },
    /// Invalid JSON, primitive text or typed document shape.
    Malformed {
        /// Logical source, if loading a campaign.
        path: Option<SourcePath>,
        /// Human-readable detail, not a stable diagnostic code.
        message: String,
    },
    /// Unrecognized schema id or version.
    UnsupportedSchema {
        /// Logical source, if loading a campaign.
        path: Option<SourcePath>,
        /// Supplied schema string.
        found: String,
    },
    /// Missing file, case collision or misplaced document.
    Layout {
        /// Offending logical path when available.
        path: Option<SourcePath>,
        /// Explanation of the layout violation.
        message: String,
    },
    /// The caller's engine version does not satisfy the campaign.
    EngineIncompatible {
        /// Normalized requirement.
        required: String,
        /// Supplied engine version.
        actual: String,
    },
    /// An object id was published more than once.
    DuplicateId {
        /// Repeated object identity.
        id: Ulid,
        /// First source in indexing order.
        first: SourcePath,
        /// Second source, possibly the same file.
        second: SourcePath,
    },
    /// Requirements disagree about the kind of a package.
    PackageKindConflict {
        /// Conflicting coordinate.
        package: PackageId,
    },
    /// An exact candidate version has inconsistent kind or checksum.
    CandidateConflict {
        /// Conflicting coordinate.
        package: PackageId,
        /// Complete exact version string.
        version: String,
    },
    /// No supplied candidate satisfies all requirements.
    UnresolvedPackage {
        /// Unresolved coordinate.
        package: PackageId,
        /// Sorted, deduplicated normalized ranges.
        requirements: Vec<String>,
    },
    /// A lock violates local or campaign coverage invariants.
    InvalidLock {
        /// Explanation of the invariant violation.
        message: String,
    },
    /// Canonical asset-lock bytes do not match the campaign lock.
    AssetsLockMismatch {
        /// Digest stored in campaign.lock.
        expected: Digest,
        /// Digest of canonical assets.lock bytes.
        actual: Digest,
    },
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPackageId { value } => write!(f, "invalid package id: {value:?}"),
            Self::InvalidPath { value } => write!(f, "invalid source path: {value:?}"),
            Self::InvalidDigest { value } => write!(f, "invalid digest: {value:?}"),
            Self::Malformed {
                path: Some(path),
                message,
            } => write!(f, "malformed document at {path}: {message}"),
            Self::Malformed {
                path: None,
                message,
            } => write!(f, "malformed document: {message}"),
            Self::UnsupportedSchema {
                path: Some(path),
                found,
            } => write!(f, "unsupported schema {found:?} at {path}"),
            Self::UnsupportedSchema { path: None, found } => {
                write!(f, "unsupported schema {found:?}")
            }
            Self::Layout {
                path: Some(path),
                message,
            } => write!(f, "invalid layout at {path}: {message}"),
            Self::Layout {
                path: None,
                message,
            } => write!(f, "invalid layout: {message}"),
            Self::EngineIncompatible { required, actual } => {
                write!(f, "engine {actual} does not satisfy {required}")
            }
            Self::DuplicateId { id, first, second } => {
                write!(f, "duplicate id {id} in {first} and {second}")
            }
            Self::PackageKindConflict { package } => {
                write!(f, "conflicting requirement kinds for {package}")
            }
            Self::CandidateConflict { package, version } => {
                write!(f, "conflicting candidate {package}@{version}")
            }
            Self::UnresolvedPackage {
                package,
                requirements,
            } => write!(f, "unresolved {package}: {}", requirements.join(", ")),
            Self::InvalidLock { message } => write!(f, "invalid lock: {message}"),
            Self::AssetsLockMismatch { expected, actual } => write!(
                f,
                "assets lock digest mismatch: expected {expected}, actual {actual}"
            ),
        }
    }
}

impl std::error::Error for DataError {}

pub(crate) fn malformed(error: impl fmt::Display) -> DataError {
    DataError::Malformed {
        path: None,
        message: error.to_string(),
    }
}

pub(crate) fn invalid_lock(message: impl Into<String>) -> DataError {
    DataError::InvalidLock {
        message: message.into(),
    }
}
