//! Explicit v4 lane-0 wire vocabulary: movement on the lane-0 session
//! (T026p1, ADR-0036 L1–L6).
//!
//! v1, v2 and v3 ([`crate::protocol`], [`crate::protocol_v2`],
//! [`crate::protocol_v3`]) and the `0x81`/`0x82`/`0x91` subprotocols are
//! frozen: their bytes are untouched. v4 is a new, explicitly selected
//! lane-0 version (first byte 4); there is no negotiation or downgrade.
//!
//! A v4 delta carries every v3 op by exactly its v3 bytes, wrapped in
//! [`DeltaOp::V3`] (tags 0..=20, decoded only by the v3 codec), plus two
//! movement ops: [`DeltaOp::Moved`] (tag 21) and [`DeltaOp::Position`]
//! (tag 22). A v4 intent carries the six v3 bodies re-declared here plus
//! [`IntentBody::DeclareMove`] (tag 6). No rejection code is added: v4
//! refusals reuse [`RejectionCode`] (a new code needs a new version).
//!
//! A [`Point`] is a position in raw `Fx16_16` metres with every component
//! within the ADR-0036 U3 bound [`COORD_LIMIT_RAW`], so no encoded point can
//! exceed 15 bytes. Nothing here reads `World`, mints replica ids or
//! authorises anything.

use crate::codec::CodecError;

pub use crate::protocol::{NetId, SessionEpoch};
pub use crate::protocol_v3::{EncounterMode, ReceiptStatus, RejectionCode};

/// Wire protocol version for explicitly selected v4 lane-0 frames.
pub const PROTOCOL_VERSION: u8 = 4;

/// [`IntentBody::DeclareMove`] tag; 0..=5 are `crate::protocol_v3::INTENT_TAG_*`.
pub const INTENT_TAG_DECLARE_MOVE: u8 = 6;
/// [`DeltaOp::Moved`] tag; 0..=20 are v3's tags, carried by [`DeltaOp::V3`].
pub const DELTA_TAG_MOVED: u8 = 21;
/// [`DeltaOp::Position`] tag.
pub const DELTA_TAG_POSITION: u8 = 22;

/// Largest `|raw|` of any coordinate (2^29, ADR-0036 U3); equals
/// `crpg_sim::COORD_LIMIT_RAW` (asserted by the W11 test, not imported).
pub const COORD_LIMIT_RAW: i32 = 1 << 29;
/// Most points in a [`DeltaOp::Moved`]; equals `crpg_sim::MAX_ROUTE_POINTS`
/// (asserted by the W11 test, not imported).
pub const MAX_MOVED_POINTS: usize = 64;

/// A position in raw `Fx16_16` metres `[x, y, z]`, every component within U3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point([i32; 3]);

impl Point {
    /// Wraps raw components, or returns [`None`] if any component's
    /// `unsigned_abs()` exceeds [`COORD_LIMIT_RAW`].
    pub fn new(raw: [i32; 3]) -> Option<Self> {
        let limit = COORD_LIMIT_RAW as u32;
        if raw.iter().any(|component| component.unsigned_abs() > limit) {
            None
        } else {
            Some(Self(raw))
        }
    }

    /// Returns the raw `[x, y, z]` components.
    pub fn raw(self) -> [i32; 3] {
        self.0
    }
}

/// A disclosed position: where the entity is and whether it has a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionFact {
    /// Where the entity is.
    pub point: Point,
    /// Whether the entity currently has a route.
    pub moving: bool,
}

/// One versioned, lane-tagged v4 client command frame.
///
/// The header is the v3 header. `actor` is nonzero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntentFrame {
    /// Session generation this command belongs to.
    pub epoch: [u8; 16],
    /// Must equal [`LANE_COMBAT`](crate::protocol::LANE_COMBAT).
    pub lane: u8,
    /// Per-lane command identity, `SEQ_FIRST..=SEQ_LAST`.
    pub seq: u64,
    /// Last authoritative tick this client acknowledges; advisory only.
    pub observed_tick: u64,
    /// Acting replica entity.
    pub actor: NetId,
    /// The closed verb.
    pub body: IntentBody,
}

/// Closed v4 intent vocabulary: the six v3 bodies plus `DeclareMove`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentBody {
    /// Declare one single-target ability use (tag 0).
    DeclareAction {
        /// Authored ability identity.
        ability: crpg_core::Ulid,
        /// Single target replica entity.
        target: NetId,
    },
    /// End the actor's turn (tag 1).
    EndTurn,
    /// Stop the actor's standing order (tag 2).
    Stop,
    /// Ask the host to pause (tag 3).
    Pause,
    /// Ask the host to unpause (tag 4).
    Unpause,
    /// Ask the host to change the encounter mode (tag 5, then the mode byte).
    RequestMode {
        /// The requested mode.
        mode: EncounterMode,
    },
    /// Move the header's actor to `destination` in turn-based mode (tag 6,
    /// then one point; ADR-0036 L6).
    DeclareMove {
        /// Where the actor is to move.
        destination: Point,
    },
}

impl From<crate::protocol_v3::IntentBody> for IntentBody {
    fn from(body: crate::protocol_v3::IntentBody) -> Self {
        use crate::protocol_v3::IntentBody as V3;
        match body {
            V3::DeclareAction { ability, target } => Self::DeclareAction { ability, target },
            V3::EndTurn => Self::EndTurn,
            V3::Stop => Self::Stop,
            V3::Pause => Self::Pause,
            V3::Unpause => Self::Unpause,
            V3::RequestMode { mode } => Self::RequestMode { mode },
        }
    }
}

/// One versioned v4 lane-0 server delta frame.
///
/// `lane` must equal [`LANE_COMBAT`](crate::protocol::LANE_COMBAT).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaFrame {
    /// Must equal [`LANE_COMBAT`](crate::protocol::LANE_COMBAT).
    pub lane: u8,
    /// Authoritative tick this delta was produced at.
    pub server_tick: u64,
    /// Per-client delivery sequence, gapless per peer.
    pub event_seq: u64,
    /// Filtered replica operations for this frame, in order.
    pub ops: Vec<DeltaOp>,
}

/// Closed v4 delta vocabulary: every v3 op by its v3 bytes, plus two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaOp {
    /// A v3 op (tags 0..=20), with exactly its v3 bytes.
    V3(crate::protocol_v3::DeltaOp),
    /// A turn-based move along a route (tag 21): 2..=[`MAX_MOVED_POINTS`]
    /// points, no zero horizontal segment.
    Moved {
        /// The entity that moved (the host's mapping).
        entity: NetId,
        /// The route's points, in order.
        points: Vec<Point>,
    },
    /// A disclosed position, or its withdrawal (tag 22): [`None`] means the
    /// position is withheld from now on while the entity stays visible.
    Position {
        /// The entity placed.
        entity: NetId,
        /// The position, or [`None`] to forget it.
        position: Option<PositionFact>,
    },
}

impl From<crate::protocol_v3::DeltaOp> for DeltaOp {
    fn from(op: crate::protocol_v3::DeltaOp) -> Self {
        Self::V3(op)
    }
}

/// Checks a `Moved` route's points, in order: a count over
/// [`MAX_MOVED_POINTS`] is [`CodecError::LimitExceeded`], a count under 2 is
/// [`CodecError::Malformed`], then two consecutive points with equal `x` and
/// `z` (a zero horizontal segment; `y` may differ) are
/// [`CodecError::Malformed`].
pub fn check_points(points: &[Point]) -> Result<(), CodecError> {
    if points.len() > MAX_MOVED_POINTS {
        return Err(CodecError::LimitExceeded);
    }
    if points.len() < 2 {
        return Err(CodecError::Malformed);
    }
    for pair in points.windows(2) {
        if is_zero_segment(pair[0], pair[1]) {
            return Err(CodecError::Malformed);
        }
    }
    Ok(())
}

/// Reports whether two consecutive points share `x` and `z`.
pub(crate) fn is_zero_segment(from: Point, to: Point) -> bool {
    let (a, b) = (from.raw(), to.raw());
    a[0] == b[0] && a[2] == b[2]
}
