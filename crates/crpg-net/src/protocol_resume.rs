//! Resume subprotocol v1 (T025a1): separate versioned resume wire.
//!
//! First byte [`RESUME_VERSION`] (`0x91`), lane [`LANE_SESSION`] (`3`).
//! Constants, identity, the opaque [`ResumeProof`] and the message types only;
//! the codec lives in [`crate::codec_resume`]. Nothing here executes a resume:
//! the host owns authentication and policy.

use core::fmt;

use crate::codec::CodecError;
use crate::protocol::{LANE_COMBAT, MAX_INTENT_FRAME_BYTES, SEQ_LAST};

// ---------------------------------------------------------------------------
// Version, lane, tags.
// ---------------------------------------------------------------------------

/// First byte of every resume frame (the `0x9_` family).
pub const RESUME_VERSION: u8 = 0x91;

/// Lane byte for the session resume subprotocol.
pub const LANE_SESSION: u8 = 3;

/// Downstream tag: Ticket (the proof a later resume of the epoch must present).
pub const RESUME_TAG_TICKET: u8 = 0x30;
/// Downstream tag: Accepted.
pub const RESUME_TAG_ACCEPTED: u8 = 0x31;
/// Downstream tag: Rejected.
pub const RESUME_TAG_REJECTED: u8 = 0x32;
/// Upstream tag: Request.
pub const RESUME_TAG_REQUEST: u8 = 0x40;

// ---------------------------------------------------------------------------
// Caps.
// ---------------------------------------------------------------------------

/// Largest accepted resume frame in bytes, in either direction.
///
/// Equals [`MAX_INTENT_FRAME_BYTES`](crate::protocol::MAX_INTENT_FRAME_BYTES);
/// it fits both of the amended T024q direction caps.
pub const MAX_RESUME_FRAME_BYTES: usize = MAX_INTENT_FRAME_BYTES;

/// Smallest accepted proof length in bytes.
pub const MIN_RESUME_PROOF_BYTES: usize = 16;

/// Largest accepted proof length in bytes.
pub const MAX_RESUME_PROOF_BYTES: usize = 128;

/// Largest accepted lane-entry count in one resume list.
pub const MAX_RESUME_LANES: usize = 4;

// The resume wire shares no first byte, family, lane or tag with the other
// wire vocabularies (S§5). These are compile-time, so a collision is caught
// at build time rather than on the wire.
const _: () = assert!(RESUME_VERSION != crate::protocol::PROTOCOL_VERSION);
const _: () = assert!(RESUME_VERSION != crate::protocol_v2::PROTOCOL_VERSION);
const _: () = assert!(RESUME_VERSION != crate::protocol_transfer::TRANSFER_VERSION);
const _: () = assert!(RESUME_VERSION & 0xF0 != crate::protocol_transfer::TRANSFER_VERSION & 0xF0);
const _: () = assert!(LANE_SESSION != crate::protocol::LANE_COMBAT);
const _: () = assert!(LANE_SESSION != crate::protocol_transfer::LANE_SNAPSHOT);
const _: () = assert!(MAX_RESUME_PROOF_BYTES <= u8::MAX as usize);
const _: () = assert!(MAX_RESUME_FRAME_BYTES <= crate::protocol::MAX_DELTA_FRAME_BYTES);

// ---------------------------------------------------------------------------
// Identity.
// ---------------------------------------------------------------------------

/// A session's connection generation. A session's first connection holds
/// [`FIRST`](ConnectionFence::FIRST); each accepted resume grants the next
/// value. Never 0. Not a secret and not a credential: it orders connections,
/// it proves nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnectionFence(u64);

impl ConnectionFence {
    /// The fence of a session's first connection.
    pub const FIRST: ConnectionFence = ConnectionFence(1);

    /// Wraps a raw fence, returning [`None`] for the never-valid 0.
    pub fn new(raw: u64) -> Option<ConnectionFence> {
        if raw == 0 {
            None
        } else {
            Some(ConnectionFence(raw))
        }
    }

    /// Returns the raw fence value.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// Host-issued, session-bound resume proof: opaque bytes, 16..=128 long.
///
/// No format and no meaning here. No `PartialEq`/`Eq`/`Ord`/`Hash`/`Display`.
#[derive(Clone)]
pub struct ResumeProof(Vec<u8>);

impl ResumeProof {
    /// Below 16 bytes (0 included): `Err(Malformed)`; above 128:
    /// `Err(LimitExceeded)`.
    pub fn new(bytes: Vec<u8>) -> Result<ResumeProof, CodecError> {
        if bytes.len() < MIN_RESUME_PROOF_BYTES {
            return Err(CodecError::Malformed);
        }
        if bytes.len() > MAX_RESUME_PROOF_BYTES {
            return Err(CodecError::LimitExceeded);
        }
        Ok(ResumeProof(bytes))
    }

    /// The opaque proof bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Constant-time equality over equal lengths (unequal lengths: `false`
    /// without reading content); XOR fold, no early exit, `black_box`.
    pub fn ct_eq(&self, expected: &[u8]) -> bool {
        if self.0.len() != expected.len() {
            return false;
        }
        let mut acc = 0u8;
        for (a, b) in self.0.iter().zip(expected) {
            acc |= a ^ b;
        }
        core::hint::black_box(acc) == 0
    }
}

impl fmt::Debug for ResumeProof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ResumeProof(<redacted>)")
    }
}

// ---------------------------------------------------------------------------
// Lane claims and progress.
// ---------------------------------------------------------------------------

/// Advisory: the highest seq on `lane` for which the client holds a terminal
/// receipt (0 = none). Never changes host state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneClaim {
    /// Lane.
    pub lane: u8,
    /// Highest receipted seq.
    pub receipted_through: u64,
}

/// Authoritative: the highest seq on `lane` the host finalized (0 = none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneProgress {
    /// Lane.
    pub lane: u8,
    /// Highest finalized seq.
    pub finalized_through: u64,
}

// ---------------------------------------------------------------------------
// Upstream message.
// ---------------------------------------------------------------------------

/// Upstream `0x40`. `Debug` is redacted through [`ResumeProof`].
#[derive(Debug, Clone)]
pub struct ResumeRequest {
    /// The retained session epoch the client claims (not identity by itself).
    pub epoch: [u8; 16],
    /// The last fence the client held on this session.
    pub fence: ConnectionFence,
    /// The proof the client presents.
    pub proof: ResumeProof,
    /// Per-lane advisory claims.
    pub claims: Vec<LaneClaim>,
}

impl ResumeRequest {
    /// The L-rules on `claims`.
    pub fn validate(&self) -> Result<(), CodecError> {
        check_lanes(self.claims.iter().map(|c| (c.lane, c.receipted_through)))
    }
}

// ---------------------------------------------------------------------------
// Downstream messages.
// ---------------------------------------------------------------------------

/// Downstream `0x30`: the proof a later resume of `epoch` must present.
#[derive(Debug, Clone)]
pub struct ResumeTicket {
    /// Epoch.
    pub epoch: [u8; 16],
    /// The proof.
    pub proof: ResumeProof,
}

/// Closed resume rejection.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeRejection {
    /// Resumption is impossible: start a new session.
    Denied = 1,
    /// Not now: the session is retained; the same request may be retried.
    Busy = 2,
}

impl ResumeRejection {
    /// Returns the wire value.
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Maps a wire byte, returning [`None`] for unassigned values.
    pub fn from_u8(raw: u8) -> Option<ResumeRejection> {
        match raw {
            1 => Some(ResumeRejection::Denied),
            2 => Some(ResumeRejection::Busy),
            _ => None,
        }
    }
}

/// Downstream `0x31`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeAccepted {
    /// Epoch.
    pub epoch: [u8; 16],
    /// The fence this connection now holds.
    pub granted: ConnectionFence,
    /// Authoritative per-lane progress.
    pub progress: Vec<LaneProgress>,
}

/// Downstream `0x32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumeRejected {
    /// Epoch.
    pub epoch: [u8; 16],
    /// Cause-neutral reason.
    pub reason: ResumeRejection,
}

/// Downstream answer to one resume request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeResult {
    /// Accepted (tag `0x31`).
    Accepted(ResumeAccepted),
    /// Rejected (tag `0x32`).
    Rejected(ResumeRejected),
}

impl ResumeResult {
    /// The epoch either answer names.
    pub fn epoch(&self) -> [u8; 16] {
        match self {
            ResumeResult::Accepted(a) => a.epoch,
            ResumeResult::Rejected(r) => r.epoch,
        }
    }

    /// Accepted: `granted` ≥ 2 (`Malformed`), then the L-rules on `progress`.
    /// Rejected: always `Ok`.
    pub fn validate(&self) -> Result<(), CodecError> {
        match self {
            ResumeResult::Accepted(a) => {
                if a.granted.get() < 2 {
                    return Err(CodecError::Malformed);
                }
                check_lanes(a.progress.iter().map(|p| (p.lane, p.finalized_through)))
            }
            ResumeResult::Rejected(_) => Ok(()),
        }
    }
}

/// Every downstream resume frame.
#[derive(Debug, Clone)]
pub enum ResumeDown {
    /// Ticket (tag `0x30`).
    Ticket(ResumeTicket),
    /// An answer (`Accepted` tag `0x31`, `Rejected` tag `0x32`).
    Answer(ResumeResult),
}

// ---------------------------------------------------------------------------
// Shared lane-list rules.
// ---------------------------------------------------------------------------

/// The L-rules (S§6.3) over a whole list: the count from `len()` before any
/// entry is looked at, then each `(lane, through)` entry in order.
pub(crate) fn check_lanes(
    entries: impl ExactSizeIterator<Item = (u8, u64)>,
) -> Result<(), CodecError> {
    let count = entries.len();
    if count == 0 {
        return Err(CodecError::Malformed);
    }
    if count > MAX_RESUME_LANES {
        return Err(CodecError::LimitExceeded);
    }
    let mut previous: Option<u8> = None;
    for (lane, through) in entries {
        if lane != LANE_COMBAT {
            return Err(CodecError::UnknownMessage);
        }
        if let Some(prev) = previous {
            if lane <= prev {
                return Err(CodecError::Malformed);
            }
        }
        previous = Some(lane);
        if through > SEQ_LAST {
            return Err(CodecError::Malformed);
        }
    }
    Ok(())
}
