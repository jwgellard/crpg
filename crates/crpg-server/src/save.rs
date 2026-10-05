//! Host save adapter over `crpg-persist` (T039).
//!
//! A host save is a `crpg-persist` envelope of kind [`HOST_SAVE_KIND`] whose
//! payload is a small fixed-order binary identity header followed by T022's
//! checkpoint bytes **verbatim** (`Host::save_checkpoint` output):
//!
//! | Offset | Size | Field | Encoding |
//! |---|---|---|---|
//! | 0 | 4 | `save_version` | u32 LE, [`HOST_SAVE_VERSION`] |
//! | 4 | 16 | `campaign_id` | `Ulid::to_u128().to_be_bytes()` |
//! | 20 | 1 | `cv_len` | u8, `1..=64` |
//! | 21 | `cv_len` | `campaign_version` | version-alphabet ASCII |
//! | 21 + `cv_len` | 1 | `ev_len` | u8, `1..=64` |
//! | 22 + `cv_len` | `ev_len` | `engine_version` | version-alphabet ASCII |
//! | 22 + `cv_len` + `ev_len` | rest | checkpoint | T022 compact JSON |
//!
//! Offsets 0..4 are a frozen prefix: every future host save version keeps
//! them, so any build can refuse a foreign version before reading anything
//! else. The campaign id and version are trusted caller facts and must match
//! on load; the engine version is recorded and returned, never enforced.
//!
//! Loading checks the whole header structurally, then the identity, and only
//! then hands the checkpoint to [`load_checkpoint`], so every T022 rule
//! (pre-parse cap, full validation, fresh sessions under a strictly greater
//! incarnation) applies unchanged. This module is the crate's only
//! filesystem I/O, and only through `crpg_persist::save_file`/`load_file`.

use std::fmt;
use std::path::Path;

use crpg_core::Ulid;
use crpg_persist::{FileError, PayloadKind, MAX_PAYLOAD_BYTES};

use crate::checkpoint::{load_checkpoint, CheckpointError};
use crate::host::Host;

/// Envelope payload tag for a host save: ASCII `HOSTCKPT`.
pub const HOST_SAVE_KIND: PayloadKind = match PayloadKind::new(*b"HOSTCKPT") {
    Some(kind) => kind,
    None => panic!("HOSTCKPT is inside the PayloadKind alphabet"),
};
/// The only host save payload version this build writes or reads.
pub const HOST_SAVE_VERSION: u32 = 1;
/// Engine version recorded in every save: this crate's package version.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Upper bound, in bytes, on each version text (campaign and engine).
pub const MAX_VERSION_TEXT_BYTES: usize = 64;
/// Header bytes besides the two version texts: version, campaign id, two length bytes.
pub const HOST_SAVE_FIXED_BYTES: usize = 22;
/// Largest possible header (150 bytes).
pub const MAX_HOST_SAVE_HEADER_BYTES: usize = HOST_SAVE_FIXED_BYTES + 2 * MAX_VERSION_TEXT_BYTES;

/// Offset of the campaign id.
const CAMPAIGN_ID_AT: usize = 4;
/// Offset of the campaign version length byte.
const CV_LEN_AT: usize = 20;
/// Offset of the campaign version text.
const CV_AT: usize = 21;

/// True iff `bytes` is 1..=`MAX_VERSION_TEXT_BYTES` bytes of
/// `0-9 A-Z a-z . + -` (the semver character set).
const fn valid_version_text(bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.len() > MAX_VERSION_TEXT_BYTES {
        return false;
    }
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if !(b.is_ascii_digit()
            || b.is_ascii_uppercase()
            || b.is_ascii_lowercase()
            || b == b'.'
            || b == b'+'
            || b == b'-')
        {
            return false;
        }
        i += 1;
    }
    true
}

// An invalid package version fails the build instead of producing saves that
// this build refuses.
const _: () = assert!(valid_version_text(ENGINE_VERSION.as_bytes()));

/// Trusted campaign identity supplied by the embedding (spec §8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveIdentity {
    /// The campaign's stable authored id (`Campaign.id`).
    pub campaign_id: Ulid,
    /// The campaign's published version text, 1..=64 bytes of `0-9 A-Z a-z . + -`.
    pub campaign_version: String,
}

/// A host restored from a save, plus the engine version that wrote it.
#[derive(Debug)]
pub struct LoadedSave {
    /// The restored host: fresh sessions under the new incarnation (T022 §7).
    pub host: Host,
    /// The saving build's `ENGINE_VERSION`. Recorded, never enforced.
    pub engine_version: String,
}

/// Every save-adapter failure. First failure wins (§4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// A `SaveIdentity` campaign version is empty, longer than
    /// `MAX_VERSION_TEXT_BYTES`, or outside the version alphabet.
    InvalidIdentity,
    /// The payload would exceed (encode) or exceeds (decode) `crpg_persist::MAX_PAYLOAD_BYTES`.
    TooLarge {
        /// Header plus checkpoint length (encode), or the payload length (decode).
        len: usize,
    },
    /// The header is truncated, or a length or text field is out of range.
    Malformed,
    /// Payload offsets 0..4 hold a version other than `HOST_SAVE_VERSION`.
    UnsupportedVersion {
        /// The rejected version.
        version: u32,
    },
    /// The saved campaign id differs from the expected one.
    CampaignMismatch,
    /// The saved campaign version text differs from the expected one.
    CampaignVersionMismatch,
    /// `Host::save_checkpoint` or `checkpoint::load_checkpoint` failed.
    Checkpoint(CheckpointError),
    /// `crpg_persist::save_file` or `load_file` failed.
    Persist(FileError),
}

impl From<CheckpointError> for SaveError {
    fn from(e: CheckpointError) -> Self {
        SaveError::Checkpoint(e)
    }
}

impl From<FileError> for SaveError {
    fn from(e: FileError) -> Self {
        SaveError::Persist(e)
    }
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::InvalidIdentity => "InvalidIdentity",
            Self::TooLarge { .. } => "TooLarge",
            Self::Malformed => "Malformed",
            Self::UnsupportedVersion { .. } => "UnsupportedVersion",
            Self::CampaignMismatch => "CampaignMismatch",
            Self::CampaignVersionMismatch => "CampaignVersionMismatch",
            Self::Checkpoint(e) => return fmt::Display::fmt(e, f),
            Self::Persist(e) => return fmt::Display::fmt(e, f),
        };
        write!(f, "{name} at host/save")
    }
}

impl std::error::Error for SaveError {}

/// Builds a host save payload: the §3 header, then `checkpoint` verbatim.
/// Pure; never inspects `checkpoint`.
pub fn encode_host_save(identity: &SaveIdentity, checkpoint: &[u8]) -> Result<Vec<u8>, SaveError> {
    // 1. Identity text.
    let campaign_version = identity.campaign_version.as_bytes();
    if !valid_version_text(campaign_version) {
        return Err(SaveError::InvalidIdentity);
    }
    // 2. Payload cap. `header_len <= MAX_HOST_SAVE_HEADER_BYTES`, so the
    // subtraction cannot underflow.
    let engine_version = ENGINE_VERSION.as_bytes();
    let header_len = HOST_SAVE_FIXED_BYTES + campaign_version.len() + engine_version.len();
    if checkpoint.len() > MAX_PAYLOAD_BYTES - header_len {
        return Err(SaveError::TooLarge {
            len: header_len.saturating_add(checkpoint.len()),
        });
    }
    // 3. One allocation; fields in §3 order, then the checkpoint unread. Both
    // text lengths are at most 64 after the checks above, so `as u8` is
    // lossless.
    let mut out = Vec::with_capacity(header_len + checkpoint.len());
    out.extend_from_slice(&HOST_SAVE_VERSION.to_le_bytes());
    out.extend_from_slice(&identity.campaign_id.to_u128().to_be_bytes());
    out.push(campaign_version.len() as u8);
    out.extend_from_slice(campaign_version);
    out.push(engine_version.len() as u8);
    out.extend_from_slice(engine_version);
    out.extend_from_slice(checkpoint);
    Ok(out)
}

/// Validates a host save payload (§4.2) and loads its checkpoint with
/// `checkpoint::load_checkpoint(_, new_incarnation, now_ms)`.
pub fn decode_host_save(
    payload: &[u8],
    expected: &SaveIdentity,
    new_incarnation: u64,
    now_ms: u64,
) -> Result<LoadedSave, SaveError> {
    // 1. The caller's identity; nothing in `payload` is read.
    let expected_version = expected.campaign_version.as_bytes();
    if !valid_version_text(expected_version) {
        return Err(SaveError::InvalidIdentity);
    }
    // 2. Payload cap.
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(SaveError::TooLarge { len: payload.len() });
    }
    // 3–4. The frozen version prefix, before any v1 layout is assumed.
    if payload.len() < CAMPAIGN_ID_AT {
        return Err(SaveError::Malformed);
    }
    let mut version = [0u8; 4];
    version.copy_from_slice(&payload[..CAMPAIGN_ID_AT]);
    let version = u32::from_le_bytes(version);
    if version != HOST_SAVE_VERSION {
        return Err(SaveError::UnsupportedVersion { version });
    }
    // 5–8. Campaign version length and text.
    if payload.len() < CV_AT {
        return Err(SaveError::Malformed);
    }
    let cv_len = usize::from(payload[CV_LEN_AT]);
    if cv_len == 0 || cv_len > MAX_VERSION_TEXT_BYTES {
        return Err(SaveError::Malformed);
    }
    let ev_len_at = CV_AT + cv_len;
    if payload.len() < ev_len_at + 1 {
        return Err(SaveError::Malformed);
    }
    let campaign_version = &payload[CV_AT..ev_len_at];
    if !valid_version_text(campaign_version) {
        return Err(SaveError::Malformed);
    }
    // 9–11. Engine version length and text.
    let ev_len = usize::from(payload[ev_len_at]);
    if ev_len == 0 || ev_len > MAX_VERSION_TEXT_BYTES {
        return Err(SaveError::Malformed);
    }
    let ev_at = ev_len_at + 1;
    let header_len = ev_at + ev_len;
    if payload.len() < header_len {
        return Err(SaveError::Malformed);
    }
    let engine_version = &payload[ev_at..header_len];
    if !valid_version_text(engine_version) {
        return Err(SaveError::Malformed);
    }
    // 12–13. Identity, before any JSON is parsed.
    let mut campaign_id = [0u8; 16];
    campaign_id.copy_from_slice(&payload[CAMPAIGN_ID_AT..CV_LEN_AT]);
    if Ulid::from_u128(u128::from_be_bytes(campaign_id)) != expected.campaign_id {
        return Err(SaveError::CampaignMismatch);
    }
    if campaign_version != expected_version {
        return Err(SaveError::CampaignVersionMismatch);
    }
    // 14. T022's full checkpoint validation; an empty remainder is its
    // `Malformed`.
    let host = load_checkpoint(&payload[header_len..], new_incarnation, now_ms)?;
    // 15. The validated ASCII engine text, never compared with ENGINE_VERSION.
    let engine_version = engine_version.iter().copied().map(char::from).collect();
    Ok(LoadedSave {
        host,
        engine_version,
    })
}

/// Saves a quiescent host to `path` through `crpg_persist::save_file` (§4.3).
pub fn save_host_file(path: &Path, host: &Host, identity: &SaveIdentity) -> Result<(), SaveError> {
    // 1. Identity first: the host is not read and no file is touched.
    if !valid_version_text(identity.campaign_version.as_bytes()) {
        return Err(SaveError::InvalidIdentity);
    }
    // 2. Quiescent checkpoint (T022 §7).
    let checkpoint = host.save_checkpoint()?;
    // 3. Payload, still without touching the filesystem.
    let payload = encode_host_save(identity, &checkpoint)?;
    // 4. Atomic replace through the persist file store.
    crpg_persist::save_file(path, HOST_SAVE_KIND, &payload)?;
    Ok(())
}

/// Loads a host from `path` through `crpg_persist::load_file` (§4.4).
pub fn load_host_file(
    path: &Path,
    expected: &SaveIdentity,
    new_incarnation: u64,
    now_ms: u64,
) -> Result<LoadedSave, SaveError> {
    // 1. Identity first: the path is not opened.
    if !valid_version_text(expected.campaign_version.as_bytes()) {
        return Err(SaveError::InvalidIdentity);
    }
    // 2. Capped read and full envelope validation.
    let payload = crpg_persist::load_file(path, HOST_SAVE_KIND)?;
    // 3. Header, identity, then the checkpoint.
    decode_host_save(&payload, expected, new_incarnation, now_ms)
}
