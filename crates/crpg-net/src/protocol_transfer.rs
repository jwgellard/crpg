#![allow(missing_docs)]
//! Transfer subprotocol v1 (T024): separate versioned snapshot-transfer wire.
//!
//! First byte [`TRANSFER_VERSION`] (`0x81`), lane [`LANE_SNAPSHOT`] (`2`).
//! Constants, identity, message and body types only; codecs live in
//! [`crate::codec_transfer`], reassembly in [`crate::reassembly`].

use crate::codec::CodecError;
use crate::protocol::{MAX_DELTA_FRAME_BYTES, MAX_INTENT_FRAME_BYTES, MAX_SNAPSHOT_BYTES,
    MAX_SNAPSHOT_CHUNKS, MAX_VISIBLE_ENTITIES, MAX_WIRE_BYTES_FIELD, SEQ_LAST};

// ---------------------------------------------------------------------------
// Version, lane, tags.
// ---------------------------------------------------------------------------

/// First byte of every transfer frame.
pub const TRANSFER_VERSION: u8 = 0x81;

/// Lane byte for snapshot transfer.
pub const LANE_SNAPSHOT: u8 = 2;

/// Downstream tag: Begin.
pub const TRANSFER_TAG_BEGIN: u8 = 0x10;
/// Downstream tag: Chunk.
pub const TRANSFER_TAG_CHUNK: u8 = 0x11;
/// Downstream tag: Cancel.
pub const TRANSFER_TAG_CANCEL: u8 = 0x12;
/// Upstream tag: Published.
pub const TRANSFER_TAG_PUBLISHED: u8 = 0x20;
/// Upstream tag: Resync.
pub const TRANSFER_TAG_RESYNC: u8 = 0x21;

// ---------------------------------------------------------------------------
// S§7 caps.
// ---------------------------------------------------------------------------

/// Maximum segment bytes (= `MAX_WIRE_BYTES_FIELD`).
pub const MAX_SEGMENT_BYTES: usize = MAX_WIRE_BYTES_FIELD;
/// Maximum segments per chunk.
pub const MAX_SEGMENTS_PER_CHUNK: u8 = 8;
/// Maximum chunk payload bytes.
pub const MAX_CHUNK_BYTES: usize = 32_768;
/// Maximum downstream frame bytes.
pub const MAX_DOWN_FRAME_BYTES: usize = MAX_DELTA_FRAME_BYTES;
/// Maximum upstream frame bytes.
pub const MAX_UP_FRAME_BYTES: usize = MAX_INTENT_FRAME_BYTES;
/// Maximum encoded body bytes.
pub const MAX_ENCODED_BODY_BYTES: usize = 22_552;
/// Staging bytes per receiver.
pub const POLICY_SNAPSHOT_STAGING_BYTES_PER_PEER: usize = MAX_SNAPSHOT_BYTES;
/// Staging bytes for the whole pool.
pub const POLICY_SNAPSHOT_STAGING_BYTES_HOST: usize = 8 * 1_048_576;

const _: () = assert!(MAX_CHUNK_BYTES * MAX_SNAPSHOT_CHUNKS == MAX_SNAPSHOT_BYTES);
const _: () = assert!(MAX_SEGMENT_BYTES * MAX_SEGMENTS_PER_CHUNK as usize == MAX_CHUNK_BYTES);

pub use crate::protocol::NetId;

// ---------------------------------------------------------------------------
// Identity.
// ---------------------------------------------------------------------------

/// Host-assigned per-epoch transfer id, strictly increasing from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransferId(u64);

impl TransferId {
    /// Returns `None` for zero.
    pub fn new(raw: u64) -> Option<Self> {
        if raw == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }
    /// Returns the raw id.
    pub fn get(self) -> u64 {
        self.0
    }
}

// ---------------------------------------------------------------------------
// Messages.
// ---------------------------------------------------------------------------

/// Downstream Begin header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginTransfer {
    /// Epoch.
    pub epoch: [u8; 16],
    /// Transfer id.
    pub transfer: TransferId,
    /// Base tick.
    pub base_tick: u64,
    /// Delivery cursor.
    pub cursor: u64,
    /// Total bytes.
    pub total_bytes: u32,
    /// Entity count.
    pub entity_count: u16,
    /// Segments per chunk.
    pub segments_per_chunk: u8,
    /// Chunk count.
    pub chunk_count: u8,
}

impl BeginTransfer {
    /// S§5.1/S§6 checks in field order.
    pub fn validate(&self) -> Result<(), CodecError> {
        if self.transfer.get() == 0 {
            return Err(CodecError::Malformed);
        }
        if self.cursor > SEQ_LAST {
            return Err(CodecError::Malformed);
        }
        if self.total_bytes == 0 {
            return Err(CodecError::Malformed);
        }
        if self.total_bytes as usize > MAX_SNAPSHOT_BYTES {
            return Err(CodecError::LimitExceeded);
        }
        if self.entity_count as usize > MAX_VISIBLE_ENTITIES {
            return Err(CodecError::LimitExceeded);
        }
        if self.segments_per_chunk == 0 {
            return Err(CodecError::Malformed);
        }
        if self.segments_per_chunk > MAX_SEGMENTS_PER_CHUNK {
            return Err(CodecError::LimitExceeded);
        }
        if self.chunk_count == 0 {
            return Err(CodecError::Malformed);
        }
        if self.chunk_count as usize > MAX_SNAPSHOT_CHUNKS {
            return Err(CodecError::LimitExceeded);
        }
        let chunk_size = self.segments_per_chunk as usize * MAX_SEGMENT_BYTES;
        let expected = (self.total_bytes as usize).div_ceil(chunk_size);
        if expected != self.chunk_count as usize {
            return Err(CodecError::Malformed);
        }
        Ok(())
    }

    /// Exact payload length of chunk `index`, or None.
    pub fn chunk_len(&self, index: u8) -> Option<usize> {
        if index >= self.chunk_count {
            return None;
        }
        // validate first
        if self.validate().is_err() {
            return None;
        }
        let chunk_size = self.segments_per_chunk as usize * MAX_SEGMENT_BYTES;
        let total = self.total_bytes as usize;
        let start = index as usize * chunk_size;
        if start >= total {
            return None;
        }
        let end = ((index as usize + 1) * chunk_size).min(total);
        Some(end - start)
    }
}

/// Transfer chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferChunk {
    /// Epoch.
    pub epoch: [u8; 16],
    /// Transfer id.
    pub transfer: TransferId,
    /// Chunk index.
    pub index: u8,
    /// Payload bytes.
    pub bytes: Vec<u8>,
}

/// Cancel reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CancelReason {
    /// Superseded.
    Superseded = 1,
    /// Aborted.
    Aborted = 2,
}

impl CancelReason {
    /// Returns wire value.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
    /// Maps wire value.
    pub fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Superseded),
            2 => Some(Self::Aborted),
            _ => None,
        }
    }
}

/// Resync reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ResyncReason {
    /// Expired.
    Expired = 1,
    /// Invalid.
    Invalid = 2,
    /// Overflow.
    Overflow = 3,
}

impl ResyncReason {
    /// Returns wire value.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
    /// Maps wire value.
    pub fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Expired),
            2 => Some(Self::Invalid),
            3 => Some(Self::Overflow),
            _ => None,
        }
    }
}

/// Downstream transfer message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferDown {
    /// Begin.
    Begin(BeginTransfer),
    /// Chunk.
    Chunk(TransferChunk),
    /// Cancel.
    Cancel {
        /// Epoch.
        epoch: [u8; 16],
        /// Transfer.
        transfer: TransferId,
        /// Reason.
        reason: CancelReason,
    },
}

/// Upstream transfer message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferUp {
    /// Published.
    Published {
        /// Epoch.
        epoch: [u8; 16],
        /// Transfer.
        transfer: TransferId,
    },
    /// Resync.
    Resync {
        /// Epoch.
        epoch: [u8; 16],
        /// After.
        after: Option<TransferId>,
        /// Reason.
        reason: ResyncReason,
    },
}

// ---------------------------------------------------------------------------
// Body.
// ---------------------------------------------------------------------------

/// Snapshot body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotBody {
    /// Entities.
    pub entities: Vec<SnapshotEntity>,
    /// Turn.
    pub turn: Option<TurnFact>,
}

/// Snapshot entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotEntity {
    /// Entity id.
    pub entity: NetId,
    /// Health.
    pub health: Option<HealthFact>,
}

/// Health fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthFact {
    /// Health.
    pub health: u32,
    /// Max health.
    pub max_health: u32,
    /// Dead.
    pub dead: bool,
}

/// Turn fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnFact {
    /// Active.
    pub active: Option<NetId>,
    /// Round.
    pub round: u64,
}