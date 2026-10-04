//! Envelope v1: a fixed 68-byte header, then the raw payload or exactly one
//! zstd frame (T038 §3).
//!
//! The header is magic, format version, flags, payload kind, payload length,
//! body length and a domain-separated BLAKE3 digest over everything before
//! the digest plus the stored body. [`decode_envelope`] checks the header in
//! a fixed first-failure-wins order and enforces the decompressed-size cap
//! while zstd is still producing bytes, so a decompression bomb is stopped at
//! the declared length.

use std::fmt;
use std::io::{ErrorKind, Read};

/// Envelope magic, offsets 0..8: ASCII "CRPGSAVE".
pub const SAVE_MAGIC: [u8; 8] = *b"CRPGSAVE";
/// The only format version this build writes or reads.
pub const SAVE_FORMAT_VERSION: u16 = 1;
/// Fixed v1 header length in bytes (see §3).
pub const HEADER_BYTES: usize = 68;
/// Flag bit 0: the body is one zstd frame. Clear: the body is the raw payload.
pub const FLAG_ZSTD: u16 = 0x0001;
/// Decompressed-size ceiling: 16 MiB (16 * 1024 * 1024 bytes) of payload.
/// Equal to T022's `MAX_CHECKPOINT_BYTES`, so every checkpoint the host accepts fits.
pub const MAX_PAYLOAD_BYTES: usize = 16_777_216;
/// Input-size ceiling: header plus the largest body (bytes). 16_777_284.
pub const MAX_ENVELOPE_BYTES: usize = HEADER_BYTES + MAX_PAYLOAD_BYTES;
/// zstd compression level used by the encoder.
pub const ZSTD_LEVEL: i32 = 3;
/// Decoder window ceiling, as log2 bytes: 2^24 = 16 MiB.
pub const ZSTD_WINDOW_LOG_MAX: u32 = 24;
/// BLAKE3 derive-key context for the envelope digest (hard-coded, never derived).
pub const DIGEST_CONTEXT: &str = "crpg-persist 2026-10-04 save envelope v1";

/// Size of the one scratch buffer used while decompressing.
const DECODE_CHUNK_BYTES: usize = 65_536;

/// Header field offsets (§3).
const VERSION_AT: usize = 8;
const FLAGS_AT: usize = 10;
const KIND_AT: usize = 12;
const PAYLOAD_LEN_AT: usize = 20;
const BODY_LEN_AT: usize = 28;
const DIGEST_AT: usize = 36;
/// Length of the frozen prefix (magic and version) every version keeps.
const FROZEN_PREFIX_BYTES: usize = 10;

/// Caller-chosen 8-byte payload tag; bytes restricted to `A-Z`, `0-9`, `_`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayloadKind([u8; 8]);

impl PayloadKind {
    /// `Some` iff every byte is in `b'A'..=b'Z'`, `b'0'..=b'9'` or `b'_'`.
    pub const fn new(tag: [u8; 8]) -> Option<Self> {
        let mut i = 0;
        while i < tag.len() {
            let b = tag[i];
            if !(b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') {
                return None;
            }
            i += 1;
        }
        Some(Self(tag))
    }

    /// The tag bytes, as written at header offsets 12..20.
    pub const fn as_bytes(&self) -> &[u8; 8] {
        &self.0
    }
}

/// Every envelope failure, with first-failure-wins precedence (§4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvelopeError {
    /// Input longer than `MAX_ENVELOPE_BYTES` (checked before anything else).
    InputTooLarge,
    /// Input ends before the frozen prefix, the header, or the declared body.
    Truncated,
    /// Offsets 0..8 differ from `SAVE_MAGIC`.
    BadMagic,
    /// Offsets 8..10 hold a version other than `SAVE_FORMAT_VERSION`.
    UnsupportedVersion,
    /// A flag bit other than `FLAG_ZSTD` is set.
    UnknownFlags,
    /// Header kind differs from the expected `PayloadKind`.
    KindMismatch,
    /// Payload longer than `MAX_PAYLOAD_BYTES` (encode input or declared length).
    PayloadTooLarge,
    /// `body_len` is inconsistent with `payload_len` and the flags (§4.2 step 9).
    LengthMismatch,
    /// Input continues past `HEADER_BYTES + body_len`.
    TrailingBytes,
    /// The BLAKE3 digest does not match the header prefix and body.
    ChecksumMismatch,
    /// The zstd frame produced more bytes than the declared `payload_len`.
    DecompressedTooLarge,
    /// The zstd frame ended before producing `payload_len` bytes.
    DecompressedTooShort,
    /// The zstd frame is invalid, needs a window above `ZSTD_WINDOW_LOG_MAX`,
    /// or is followed by further bytes inside the body.
    CorruptBody,
    /// The zstd compressor failed while encoding (context or allocation failure).
    Compressor,
}

impl fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InputTooLarge => "save input exceeds its byte cap",
            Self::Truncated => "save input is truncated",
            Self::BadMagic => "not a save envelope",
            Self::UnsupportedVersion => "unsupported save format version",
            Self::UnknownFlags => "unknown save envelope flags",
            Self::KindMismatch => "save payload kind mismatch",
            Self::PayloadTooLarge => "save payload exceeds its byte cap",
            Self::LengthMismatch => "save envelope lengths are inconsistent",
            Self::TrailingBytes => "save input has trailing bytes",
            Self::ChecksumMismatch => "save checksum mismatch",
            Self::DecompressedTooLarge => "decompressed save exceeds its declared length",
            Self::DecompressedTooShort => "decompressed save is shorter than its declared length",
            Self::CorruptBody => "compressed save body is corrupt",
            Self::Compressor => "save compressor failed",
        })
    }
}

impl std::error::Error for EnvelopeError {}

/// Encode `payload` as a v1 envelope. Pure; deterministic per build (§5).
pub fn encode_envelope(kind: PayloadKind, payload: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
    // Step 1: the payload cap, before any zstd work.
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(EnvelopeError::PayloadTooLarge);
    }

    // Step 2: a fresh context per call, pinned parameters, everything else default.
    let compressed = compress(payload)?;

    // Step 3: keep the zstd frame only when it is strictly shorter.
    let (flags, body): (u16, &[u8]) = if compressed.len() < payload.len() {
        (FLAG_ZSTD, &compressed)
    } else {
        (0, payload)
    };

    // Step 4: header in §3 order, digest, body; one allocation.
    let mut out = Vec::with_capacity(HEADER_BYTES + body.len());
    out.extend_from_slice(&SAVE_MAGIC);
    out.extend_from_slice(&SAVE_FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(kind.as_bytes());
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(&(body.len() as u64).to_le_bytes());
    let digest = digest(&out[..DIGEST_AT], body);
    out.extend_from_slice(&digest);
    out.extend_from_slice(body);
    Ok(out)
}

/// Validate a v1 envelope and return its payload. Pure; first failure wins (§4.2).
pub fn decode_envelope(expected: PayloadKind, bytes: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
    // 1. Input cap; nothing else is read.
    if bytes.len() > MAX_ENVELOPE_BYTES {
        return Err(EnvelopeError::InputTooLarge);
    }
    // 2. Frozen prefix present.
    if bytes.len() < FROZEN_PREFIX_BYTES {
        return Err(EnvelopeError::Truncated);
    }
    // 3. Magic.
    if bytes[..VERSION_AT] != SAVE_MAGIC {
        return Err(EnvelopeError::BadMagic);
    }
    // 4. Version, before any version-1 layout is assumed.
    if read_u16(bytes, VERSION_AT) != SAVE_FORMAT_VERSION {
        return Err(EnvelopeError::UnsupportedVersion);
    }
    // 5. Full v1 header present.
    if bytes.len() < HEADER_BYTES {
        return Err(EnvelopeError::Truncated);
    }
    // 6. Flags.
    let flags = read_u16(bytes, FLAGS_AT);
    if flags & !FLAG_ZSTD != 0 {
        return Err(EnvelopeError::UnknownFlags);
    }
    // 7. Kind.
    if bytes[KIND_AT..PAYLOAD_LEN_AT] != expected.as_bytes()[..] {
        return Err(EnvelopeError::KindMismatch);
    }
    // 8. Declared payload cap, compared as u64.
    let payload_len = read_u64(bytes, PAYLOAD_LEN_AT);
    if payload_len > MAX_PAYLOAD_BYTES as u64 {
        return Err(EnvelopeError::PayloadTooLarge);
    }
    // 9. Lengths against flags: exactly the encoder's invariants.
    let body_len = read_u64(bytes, BODY_LEN_AT);
    let consistent = if flags == FLAG_ZSTD {
        body_len >= 1 && body_len < payload_len
    } else {
        body_len == payload_len
    };
    if !consistent {
        return Err(EnvelopeError::LengthMismatch);
    }
    // 10. Declared body against the input. Both sides are bounded by the
    // caps above, so the u64 arithmetic cannot overflow.
    let declared_total = HEADER_BYTES as u64 + body_len;
    let actual_total = bytes.len() as u64;
    if actual_total < declared_total {
        return Err(EnvelopeError::Truncated);
    }
    if actual_total > declared_total {
        return Err(EnvelopeError::TrailingBytes);
    }
    // 11. Digest over the header prefix and the stored body.
    let body = &bytes[HEADER_BYTES..];
    if digest(&bytes[..DIGEST_AT], body)[..] != bytes[DIGEST_AT..HEADER_BYTES] {
        return Err(EnvelopeError::ChecksumMismatch);
    }
    // 12. Payload. `payload_len <= MAX_PAYLOAD_BYTES` after step 8, so the
    // conversion is lossless.
    if flags == FLAG_ZSTD {
        decompress_bounded(body, payload_len as usize)
    } else {
        Ok(body.to_vec())
    }
}

/// The envelope digest: BLAKE3 in derive-key mode over `prefix ‖ body`.
fn digest(prefix: &[u8], body: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key(DIGEST_CONTEXT);
    hasher.update(prefix);
    hasher.update(body);
    *hasher.finalize().as_bytes()
}

/// §4.1 step 2: one zstd frame from a fresh context.
fn compress(payload: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
    let fail = |_| EnvelopeError::Compressor;
    let mut compressor = zstd::bulk::Compressor::new(ZSTD_LEVEL).map_err(fail)?;
    compressor.include_checksum(false).map_err(fail)?;
    compressor.include_dictid(false).map_err(fail)?;
    compressor.include_contentsize(true).map_err(fail)?;
    compressor.compress(payload).map_err(fail)
}

/// §4.3: streaming decompression that stops as soon as the output would pass
/// `payload_len`. No allocation is sized from the header or the frame.
fn decompress_bounded(body: &[u8], payload_len: usize) -> Result<Vec<u8>, EnvelopeError> {
    let mut decoder = zstd::stream::read::Decoder::with_buffer(body)
        .map_err(|_| EnvelopeError::CorruptBody)?
        .single_frame();
    decoder
        .window_log_max(ZSTD_WINDOW_LOG_MAX)
        .map_err(|_| EnvelopeError::CorruptBody)?;

    let mut out = Vec::new();
    let mut chunk = vec![0u8; DECODE_CHUNK_BYTES];
    loop {
        match decoder.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if out.len() + n > payload_len {
                    return Err(EnvelopeError::DecompressedTooLarge);
                }
                out.extend_from_slice(&chunk[..n]);
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => return Err(EnvelopeError::CorruptBody),
        }
    }
    if out.len() < payload_len {
        return Err(EnvelopeError::DecompressedTooShort);
    }
    // Anything left in the body after the one frame is corruption.
    if !decoder.finish().is_empty() {
        return Err(EnvelopeError::CorruptBody);
    }
    Ok(out)
}

fn read_u16(bytes: &[u8], at: usize) -> u16 {
    let mut le = [0u8; 2];
    le.copy_from_slice(&bytes[at..at + 2]);
    u16::from_le_bytes(le)
}

fn read_u64(bytes: &[u8], at: usize) -> u64 {
    let mut le = [0u8; 8];
    le.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(le)
}
