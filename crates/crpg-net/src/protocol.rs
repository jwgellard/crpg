//! Lane-0 combat protocol v1: versioned wire vocabulary (T018a).
//!
//! This module defines the exact shapes in `tasks/T018a.md`: the version and
//! lane tags, the wire hard maxima, the session and replica identity types,
//! the closed client-intent and server-delta vocabularies, the frozen receipt
//! codes, and the wire discriminant tags. It also records the operational
//! policy defaults (host-supplied, tightening-only, never wire) that the
//! T018b simulated transport enforces and the T018c suite proves.
//!
//! Nothing here executes gameplay. Encoding and decoding live in
//! [`crate::codec`]; the in-memory byte pipe shape lives in
//! [`crate::transport`]. Host admission, visibility filtering, and gameplay
//! legality belong to later tasks and the host crate selected by E012/E022.

// ---------------------------------------------------------------------------
// Version, lane, sequence identity.
// ---------------------------------------------------------------------------

/// Wire protocol version implemented by this crate.
///
/// Only version 1 exists. Implementations reject any other version byte with
/// [`crate::codec::CodecError::UnsupportedVersion`]; additive variants, field
/// additions, or meaning changes require a new protocol version plus new
/// conformance fixtures, never silent reinterpretation.
pub const PROTOCOL_VERSION: u8 = 1;

/// The only implemented lane in v1: reliable-ordered combat commands upstream
/// and reliable-ordered combat deltas downstream.
///
/// Lanes 1+ are reserved (movement and later channels). A v1 implementation
/// rejects any other lane byte with
/// [`crate::codec::CodecError::WrongDirection`], including the lane echoed
/// inside [`DeltaOp::Receipt`].
pub const LANE_COMBAT: u8 = 0;

/// First sequence number of a per-epoch, per-lane command stream.
///
/// Lane-0 v1 policy is stop-and-wait: the next new `seq` equals the last
/// finalized `seq` + 1, and a sender advances only after the previous
/// terminal receipt (or a retry recovering it). Sequence 0 is never valid.
pub const SEQ_FIRST: u64 = 1;

/// Last sequence number of a per-epoch, per-lane command stream.
///
/// Sequences never wrap: exhaustion requires a new [`SessionEpoch`]. The
/// value `u64::MAX` itself is never a valid `seq` and is rejected as
/// malformed at the codec boundary.
pub const SEQ_LAST: u64 = u64::MAX - 1;

// ---------------------------------------------------------------------------
// Wire hard maxima (versioned; raising any of these is a protocol review).
// ---------------------------------------------------------------------------

/// Largest accepted client-intent frame in bytes, header included.
///
/// Enforced before decoding (and after encoding): oversized input is
/// rejected without allocation.
pub const MAX_INTENT_FRAME_BYTES: usize = 4096;

/// Largest accepted server-delta frame in bytes.
///
/// Senders chunk at operation boundaries so no frame exceeds this; oversized
/// input is rejected without allocation.
pub const MAX_DELTA_FRAME_BYTES: usize = 65536;

/// Largest accepted operation count in one [`DeltaFrame`].
///
/// Equals [`MAX_WIRE_COLLECTION`]; the count is checked before any per-op
/// decode or allocation.
pub const MAX_DELTA_OPS: usize = 256;

/// Largest accepted entry count in any wire collection.
pub const MAX_WIRE_COLLECTION: usize = 256;

/// Largest accepted wire string field in bytes (UTF-8).
///
/// In v1 the only string-carrying field is the canonical ULID text of
/// [`IntentBody::DeclareAction`]`ability` (always 26 bytes); the bound still
/// applies so hostile length prefixes fail closed.
pub const MAX_WIRE_STRING_BYTES: usize = 256;

/// Largest accepted wire byte field in bytes.
pub const MAX_WIRE_BYTES_FIELD: usize = 4096;

/// Largest accepted reassembled snapshot in bytes.
///
/// No snapshot transfer wire exists in v1 lane 0; the ceiling is frozen now
/// so the future snapshot lane inherits a reviewed bound.
pub const MAX_SNAPSHOT_BYTES: usize = 1_048_576;

/// Largest accepted visible-entity count in one replica view.
pub const MAX_VISIBLE_ENTITIES: usize = 1024;

/// Largest accepted snapshot-chunk count in one reassembly.
pub const MAX_SNAPSHOT_CHUNKS: usize = 32;

// ---------------------------------------------------------------------------
// Operational policy defaults (host-supplied, tightening-only, NOT wire).
// ---------------------------------------------------------------------------
//
// These are the T018a-contract defaults the T018b drivers enforce and the
// T018c suite proves. They are plain constants for net-local test doubles:
// no host API is invented here, and changing a value later must not change
// the wire. Operators may tighten them; loosening beyond this envelope needs
// the same review as a hard-maximum change.

/// Intent arrival rate per authenticated peer: sustained frames per second.
pub const POLICY_INTENT_FRAMES_PER_SEC: u64 = 40;
/// Intent arrival burst allowance per authenticated peer, in frames.
pub const POLICY_INTENT_FRAME_BURST: u64 = 80;
/// Intent arrival rate per authenticated peer: sustained bytes per second.
pub const POLICY_INTENT_BYTES_PER_SEC: u64 = 64 * 1024;
/// Intent arrival burst allowance per authenticated peer, in bytes.
pub const POLICY_INTENT_BYTE_BURST: u64 = 128 * 1024;

/// Failure-response budget per peer: sustained responses per second.
///
/// Covers all failure responses including cached rejection receipts, not
/// normal outbound deltas. Excess responses are dropped; the sender recovers
/// through the retained sequence policy.
pub const POLICY_FAIL_FRAMES_PER_SEC: u64 = 10;
/// Failure-response burst allowance per peer, in responses.
pub const POLICY_FAIL_FRAME_BURST: u64 = 10;
/// Failure-response budget per peer: sustained bytes per second.
pub const POLICY_FAIL_BYTES_PER_SEC: u64 = 4 * 1024;
/// Failure-response burst allowance per peer, in bytes.
pub const POLICY_FAIL_BYTE_BURST: u64 = 4 * 1024;

/// Ingress queue depth per peer, in frames.
pub const POLICY_INGRESS_FRAMES_PER_PEER: usize = 128;
/// Ingress queue depth per peer, in bytes.
pub const POLICY_INGRESS_BYTES_PER_PEER: usize = 256 * 1024;
/// Ingress queue depth for the whole host, in frames.
pub const POLICY_INGRESS_FRAMES_HOST: usize = 1024;
/// Ingress queue depth for the whole host, in bytes.
pub const POLICY_INGRESS_BYTES_HOST: usize = 2 * 1024 * 1024;

/// Egress queue depth per peer, in frames.
pub const POLICY_EGRESS_FRAMES_PER_PEER: usize = 128;
/// Egress queue depth per peer, in bytes.
pub const POLICY_EGRESS_BYTES_PER_PEER: usize = 2 * 1024 * 1024;
/// Egress queue depth for the whole host, in bytes.
pub const POLICY_EGRESS_BYTES_HOST: usize = 16 * 1024 * 1024;

/// Retry cache depth per peer per lane: retained finalized receipts plus
/// their canonical intent bytes.
pub const POLICY_RETRY_CACHE_PER_PEER_LANE: usize = 256;
/// Retry cache size bound per peer per lane, in bytes.
pub const POLICY_RETRY_CACHE_BYTES_PER_PEER_LANE: usize = 2 * 1024 * 1024;

/// Proof population for the T018 conformance exercise, in peers.
pub const POLICY_PROOF_PEERS: usize = 8;

/// Snapshot reassembly: concurrent in-flight snapshots per peer.
pub const POLICY_SNAPSHOT_IN_FLIGHT_PER_PEER: usize = 1;
/// Snapshot reassembly deadline, in seconds.
pub const POLICY_SNAPSHOT_REASSEMBLY_SECS: u64 = 5;

/// Observed-tick freshness window, in ticks.
///
/// A new command's advisory `observed_tick` must lie within
/// `[server_tick.saturating_sub(200), server_tick)]`. Ticks, not wall time,
/// age observations; a pause ages nothing by itself. Host policy placement
/// is durable; the value 200 is the approved provisional default.
pub const POLICY_OBSERVED_TICK_WINDOW: u64 = 200;

// ---------------------------------------------------------------------------
// Frozen wire discriminant tags (v1).
// ---------------------------------------------------------------------------

/// [`IntentBody`] tag for `DeclareAction`.
pub const INTENT_TAG_DECLARE_ACTION: u8 = 0;
/// [`IntentBody`] tag for `EndTurn`.
pub const INTENT_TAG_END_TURN: u8 = 1;

/// [`DeltaOp`] tag for `EntityEnter`.
pub const DELTA_TAG_ENTITY_ENTER: u8 = 0;
/// [`DeltaOp`] tag for `EntityLeave`.
pub const DELTA_TAG_ENTITY_LEAVE: u8 = 1;
/// [`DeltaOp`] tag for `Spawned`.
pub const DELTA_TAG_SPAWNED: u8 = 2;
/// [`DeltaOp`] tag for `Despawned`.
pub const DELTA_TAG_DESPAWNED: u8 = 3;
/// [`DeltaOp`] tag for `Died`.
pub const DELTA_TAG_DIED: u8 = 4;
/// [`DeltaOp`] tag for `Health`.
pub const DELTA_TAG_HEALTH: u8 = 5;
/// [`DeltaOp`] tag for `Turn`.
pub const DELTA_TAG_TURN: u8 = 6;
/// [`DeltaOp`] tag for `Receipt`.
pub const DELTA_TAG_RECEIPT: u8 = 7;

/// [`ReceiptStatus`] tag for `Applied`.
pub const RECEIPT_TAG_APPLIED: u8 = 0;
/// [`ReceiptStatus`] tag for `Rejected`.
pub const RECEIPT_TAG_REJECTED: u8 = 1;

// ---------------------------------------------------------------------------
// Identity.
// ---------------------------------------------------------------------------

/// Session generation, host-issued.
///
/// Replay isolation, not authentication credentials. A new epoch resets
/// per-lane numbering; session reattachment is not supported in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionEpoch(pub [u8; 16]);

/// Client-scoped replica id.
///
/// Zero is never valid and is rejected as malformed at the codec boundary;
/// ids are never reused within one epoch. Internally, drivers keep the whole
/// [`crpg_core::EntityId`] including its generation and never serialize raw
/// arena ids; revoked or hidden ids lose their actionable mapping before new
/// requests execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetId(u64);

impl NetId {
    /// Wraps a raw id, returning [`None`] for the never-valid zero.
    pub fn new(raw: u64) -> Option<Self> {
        if raw == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }

    /// Returns the raw replica id.
    pub fn get(self) -> u64 {
        self.0
    }
}

// ---------------------------------------------------------------------------
// Client intents (upstream).
// ---------------------------------------------------------------------------

/// One versioned, lane-tagged client command frame.
///
/// `observed_tick` is advisory only; host policy validates it against the
/// [`POLICY_OBSERVED_TICK_WINDOW`] window. No client-supplied costs, damage,
/// ownership, or RNG state travels here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntentFrame {
    /// Session generation this command belongs to.
    pub epoch: [u8; 16],
    /// Must equal [`LANE_COMBAT`] in v1.
    pub lane: u8,
    /// Per-lane command identity, `SEQ_FIRST..=SEQ_LAST`.
    pub seq: u64,
    /// Last authoritative tick this client acknowledges; advisory only.
    pub observed_tick: u64,
    /// Acting replica entity, resolved through the authenticated peer binding.
    pub actor: NetId,
    /// The closed combat verb.
    pub body: IntentBody,
}

/// Closed combat intent vocabulary.
///
/// No other variants exist in v1; unknown discriminants decode as
/// [`crate::codec::CodecError::UnknownMessage`]. Ability is the authored
/// combat ability identity in canonical 26-character uppercase ULID text,
/// distinct from IR action-id strings (E017 B4). Movement, positional and
/// multi-target abilities, dialogue, items, interaction, save/admin, and chat
/// are future verbs with no v1 tag; arbitrary tags are `UnknownMessage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentBody {
    /// Declare one single-target ability use (tag [`INTENT_TAG_DECLARE_ACTION`]).
    DeclareAction {
        /// Authored ability identity.
        ability: crpg_core::Ulid,
        /// Single target replica entity, matching the implemented sim shape.
        target: NetId,
    },
    /// End the actor's turn (tag [`INTENT_TAG_END_TURN`]).
    EndTurn,
}

// ---------------------------------------------------------------------------
// Server deltas (downstream).
// ---------------------------------------------------------------------------

/// One versioned server delta frame: a chunk of a client's filtered replica.
///
/// `event_seq` is the per-client delivery order, gapless per peer; drivers
/// preserve relative `(Tick, sim_seq)` order when assigning it. `ops` holds
/// `0..=MAX_DELTA_OPS` operations; senders chunk at operation boundaries.
/// Undisclosed fields are omitted, never zero-filled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaFrame {
    /// Must equal [`LANE_COMBAT`] in v1.
    pub lane: u8,
    /// Authoritative tick this delta was produced at.
    pub server_tick: u64,
    /// Per-client delivery sequence, gapless per peer.
    pub event_seq: u64,
    /// Filtered replica operations for this chunk.
    pub ops: Vec<DeltaOp>,
}

/// Closed filtered-replica vocabulary.
///
/// There is no transform op in v1 (spatial replication waits for the lane-1
/// movement channel) and no open component bag. `Despawned` is sent only when
/// disclosable per E017 A2/B2; otherwise the driver sends `EntityLeave`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaOp {
    /// A permitted entity becomes visible to this client (tag [`DELTA_TAG_ENTITY_ENTER`]).
    EntityEnter {
        /// The replica entity now visible.
        entity: NetId,
    },
    /// A still-known entity stops being visible; cause-neutral (tag [`DELTA_TAG_ENTITY_LEAVE`]).
    EntityLeave {
        /// The replica entity no longer visible.
        entity: NetId,
    },
    /// A visible spawn notice for a permitted initial state (tag [`DELTA_TAG_SPAWNED`]).
    Spawned {
        /// The spawned replica entity.
        entity: NetId,
    },
    /// A typed despawn notice, sent only when disclosable (tag [`DELTA_TAG_DESPAWNED`]).
    Despawned {
        /// The removed replica entity.
        entity: NetId,
    },
    /// A disclosable death notice; never implies despawn (tag [`DELTA_TAG_DIED`]).
    Died {
        /// The replica entity that died.
        entity: NetId,
    },
    /// Permitted health/dead replica state (tag [`DELTA_TAG_HEALTH`]).
    Health {
        /// The replica entity described.
        entity: NetId,
        /// Current health.
        health: u32,
        /// Maximum health.
        max_health: u32,
        /// Terminal flag.
        dead: bool,
    },
    /// Permitted active-actor/round replica state (tag [`DELTA_TAG_TURN`]).
    Turn {
        /// Active actor, if the encounter has one.
        active: Option<NetId>,
        /// Current round.
        round: u64,
    },
    /// Terminal per-command outcome for one intent (tag [`DELTA_TAG_RECEIPT`]).
    Receipt {
        /// Session generation of the command.
        epoch: [u8; 16],
        /// Lane of the command.
        lane: u8,
        /// Sequence of the command.
        seq: u64,
        /// Authoritative tick the command finalized at.
        processed_tick: u64,
        /// Outcome: applied or rejected with a frozen code.
        status: ReceiptStatus,
    },
}

// ---------------------------------------------------------------------------
// Receipts.
// ---------------------------------------------------------------------------

/// Terminal outcome of one finalized command.
///
/// `Applied` covers accepted attacks including legal failed attacks, and
/// accepted `EndTurn`. Duplicates return the original terminal receipt; they
/// never re-execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptStatus {
    /// The command finalized successfully (tag [`RECEIPT_TAG_APPLIED`]).
    Applied,
    /// The command finalized as rejected (tag [`RECEIPT_TAG_REJECTED`]).
    Rejected(RejectionCode),
}

/// Frozen v1 rejection codes.
///
/// Wire numeric values are frozen: new codes require a protocol version bump.
/// Codec-level shape failures use [`crate::codec::CodecError`] instead; these
/// codes are host/transport admission and gameplay outcomes carried inside
/// [`DeltaOp::Receipt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RejectionCode {
    /// Frame exceeded its byte cap before decode.
    FrameTooLarge = 1,
    /// Bounded ingress queue was full.
    QueueFull = 2,
    /// Unknown protocol version.
    UnsupportedVersion = 3,
    /// Wrong lane or direction.
    WrongDirection = 4,
    /// Unknown message tag.
    UnknownMessage = 5,
    /// Truncated or ill-formed frame.
    Malformed = 6,
    /// A wire collection, string, or op bound was exceeded.
    LimitExceeded = 7,
    /// Per-peer rate budget exhausted.
    RateLimited = 8,
    /// Connection not authenticated.
    Unauthenticated = 9,
    /// Session epoch expired or unknown.
    SessionExpired = 10,
    /// Same seq retried with different canonical bytes; closes the epoch.
    SeqConflict = 11,
    /// New seq skipped ahead of the next expected seq; does not advance it.
    SeqGap = 12,
    /// Seq older than the retained cache; never reapplied.
    StaleSeq = 13,
    /// Sequence space exhausted; a new epoch is required.
    SeqExhausted = 14,
    /// Observed tick older than the freshness window.
    StaleTick = 15,
    /// Observed tick ahead of the server tick.
    FutureTick = 16,
    /// Actor/target not resolvable for this client (public code for
    /// missing, hidden, and foreign ids alike).
    NotAuthorized = 17,
    /// Gameplay-illegal at execution time; sim precedence authoritative.
    IllegalAction = 18,
    /// Server refuses a new peer for the proof population bound.
    ServerBusy = 19,
    /// Snapshot reassembly expired (deadline or chunk bound).
    SnapshotExpired = 20,
}

impl RejectionCode {
    /// Returns the frozen v1 wire value (`1..=20`).
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Maps a wire byte to its code, or [`None`] for unassigned values.
    ///
    /// Unassigned values (including 0) decode as
    /// [`crate::codec::CodecError::UnknownMessage`], never as a neighboring
    /// code.
    pub fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::FrameTooLarge),
            2 => Some(Self::QueueFull),
            3 => Some(Self::UnsupportedVersion),
            4 => Some(Self::WrongDirection),
            5 => Some(Self::UnknownMessage),
            6 => Some(Self::Malformed),
            7 => Some(Self::LimitExceeded),
            8 => Some(Self::RateLimited),
            9 => Some(Self::Unauthenticated),
            10 => Some(Self::SessionExpired),
            11 => Some(Self::SeqConflict),
            12 => Some(Self::SeqGap),
            13 => Some(Self::StaleSeq),
            14 => Some(Self::SeqExhausted),
            15 => Some(Self::StaleTick),
            16 => Some(Self::FutureTick),
            17 => Some(Self::NotAuthorized),
            18 => Some(Self::IllegalAction),
            19 => Some(Self::ServerBusy),
            20 => Some(Self::SnapshotExpired),
            _ => None,
        }
    }
}
