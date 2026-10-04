#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Persistence: a versioned, checksummed, `zstd`-compressed save envelope
//! around opaque caller bytes, with the S001 input-size and decompressed-size
//! caps, plus an atomic-replace file store (T038).
//!
//! The envelope never encodes or inspects what it carries. The unit of save
//! (for example T022's host checkpoint) belongs to the caller. See
//! `docs/architecture/crpg-persist.md` and `tasks/T038.md`.

pub mod envelope;
pub mod file;

pub use envelope::{
    decode_envelope, encode_envelope, EnvelopeError, PayloadKind, DIGEST_CONTEXT, FLAG_ZSTD,
    HEADER_BYTES, MAX_ENVELOPE_BYTES, MAX_PAYLOAD_BYTES, SAVE_FORMAT_VERSION, SAVE_MAGIC,
    ZSTD_LEVEL, ZSTD_WINDOW_LOG_MAX,
};
pub use file::{load_file, read_capped, save_file, FileError, FileOp, TEMP_SUFFIX};
