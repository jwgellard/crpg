//! Explicit v2 history-projection wire (T021, ADR-0019).
//!
//! v1 ([`crate::protocol`]) is frozen: its version byte, tags 0..7, field
//! order, codes, and fixtures are unchanged by this module. v2 carries the
//! old projection vocabulary unchanged in meaning through
//! [`DeltaOp::Legacy`] — a Rust-only wrapper that contributes **no extra
//! tag** on the wire — plus three event tags projected from T020
//! authoritative history: [`DELTA_TAG_ACTION_RESOLVED`],
//! [`DELTA_TAG_TURN_STARTED`], and [`DELTA_TAG_ENCOUNTER_ENDED`].
//!
//! Version selection is explicit: the v1 codec refuses version byte 2 and
//! the v2 codec refuses version byte 1. There is no autodetection,
//! negotiation, or downgrade. A new tag inside an otherwise valid v1
//! envelope is [`CodecError::UnknownMessage`](crate::codec::CodecError),
//! which is a different rejection from the version refusal.
//!
//! The wire layout, bounds, decode precedence, and disclosure contract are
//! pinned by `tasks/T021.md` (specification revision 2026-09-29). Encoding
//! and decoding live in [`crate::codec_v2`]; host-fed per-field projection
//! inputs live in [`crate::projection_v2`]. Nothing here executes gameplay,
//! mints replica ids, or retains history.

use crpg_core::Ulid;

// ---------------------------------------------------------------------------
// Version and new event tags.
// ---------------------------------------------------------------------------

/// Wire protocol version for explicitly selected v2 frames.
///
/// The v1 codec refuses this byte with
/// [`CodecError::UnsupportedVersion`](crate::codec::CodecError); the v2
/// codec refuses byte 1 the same way.
pub const PROTOCOL_VERSION: u8 = 2;

/// [`DeltaOp::ActionResolved`] tag: actor, target, ability, outcome, damage.
pub const DELTA_TAG_ACTION_RESOLVED: u8 = 8;

/// [`DeltaOp::TurnStarted`] tag: actor, round.
pub const DELTA_TAG_TURN_STARTED: u8 = 9;

/// [`DeltaOp::EncounterEnded`] tag: encounter, round.
pub const DELTA_TAG_ENCOUNTER_ENDED: u8 = 10;

// ---------------------------------------------------------------------------
// Re-exported unchanged v1 value types (not v1's version constant).
// ---------------------------------------------------------------------------

pub use crate::protocol::{
    IntentBody, IntentFrame, NetId, ReceiptStatus, RejectionCode, SessionEpoch,
};

// ---------------------------------------------------------------------------
// Server deltas (downstream).
// ---------------------------------------------------------------------------

/// One explicitly versioned v2 server delta frame.
///
/// `lane` must equal [`LANE_COMBAT`](crate::protocol::LANE_COMBAT).
/// `server_tick` is the authoritative tick the frame was produced at.
/// `event_seq` is the per-client delivery sequence the host assigns after
/// suppression and chunking, starting at 1; it never reuses an
/// authoritative history sequence. A suppressed-only page produces no frame
/// and consumes no sequence. `ops` holds `0..=MAX_DELTA_OPS` operations in
/// approved journal order; senders chunk at operation boundaries. Empty op
/// lists are allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaFrame {
    /// Must equal [`LANE_COMBAT`](crate::protocol::LANE_COMBAT) in v2.
    pub lane: u8,
    /// Authoritative tick this delta was produced at.
    pub server_tick: u64,
    /// Per-client delivery sequence, gapless per peer.
    pub event_seq: u64,
    /// Filtered replica operations for this chunk, in journal order.
    pub ops: Vec<DeltaOp>,
}

/// Explicit v2 delta vocabulary: the frozen v1 ops plus history events.
///
/// [`DeltaOp::Legacy`] wraps one unchanged v1 op and encodes to exactly the
/// v1 tag and payload bytes. The three event variants carry only
/// disclosable identities, symbolic outcomes, reported damage, and widened
/// rounds — never rolls, DCs, margins, grants, entity generations, sim
/// sequences, or internal operation ids. Undisclosed events are omitted by
/// the host, never zero-filled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaOp {
    /// One unchanged v1 op (tags 0..7, identical payload bytes).
    Legacy(crate::protocol::DeltaOp),
    /// One disclosable ability resolution (tag [`DELTA_TAG_ACTION_RESOLVED`]).
    ActionResolved {
        /// The disclosable acting replica entity.
        actor: NetId,
        /// The disclosable targeted replica entity.
        target: NetId,
        /// The authored ability identity.
        ability: Ulid,
        /// The symbolic outcome: `critical_success`, `success`, `failure`,
        /// `critical_failure`, or `custom:<n>` with decimal `n` in
        /// `0..=255` and no leading zeros.
        outcome: String,
        /// The reported flat damage (possibly overkill, zero on failure).
        damage: u32,
    },
    /// One disclosable logical turn start (tag [`DELTA_TAG_TURN_STARTED`]).
    TurnStarted {
        /// The disclosable incoming turn head.
        actor: NetId,
        /// The combat round widened from the persisted counter.
        round: u64,
    },
    /// The first transition to terminal (tag [`DELTA_TAG_ENCOUNTER_ENDED`]).
    EncounterEnded {
        /// The naturally completed encounter identity.
        encounter: Ulid,
        /// The closing combat round widened from the persisted counter.
        round: u64,
    },
}
