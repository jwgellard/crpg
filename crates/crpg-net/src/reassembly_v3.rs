//! Protocol v3 receiver sizing (T140e, Reading 18).
//!
//! [`TransferAssemblyV3`] is T024's [`crate::reassembly::TransferAssembly`]
//! with a `u128` chunk bitmap and the v3 Begin rules
//! ([`crate::transfer_v3::check_begin`], [`crate::transfer_v3::chunk_len`]),
//! so it holds a 4 MiB, 128-chunk `0x82` transfer. [`TransferLimitsV3`] is
//! T024's [`crate::reassembly::TransferLimits`] re-sized for that envelope.
//!
//! This module fixes no byte: every number here is receiver policy, never
//! wire. The v1 types and their 1 MiB / 32-chunk caps are untouched, and
//! [`AssemblyError`], [`ChunkOutcome`] and [`LimitsError`] are reused by path.

use crate::codec::CodecError;
use crate::protocol_transfer::{BeginTransfer, TransferChunk, MAX_SEGMENT_BYTES};
use crate::transfer_v3::{
    check_begin, chunk_len, MAX_ENCODED_BODY_BYTES_V3, MAX_SNAPSHOT_BYTES_V3,
    MAX_SNAPSHOT_CHUNKS_V3,
};

pub use crate::reassembly::{AssemblyError, ChunkOutcome, LimitsError};

/// Reassembly deadline in milliseconds: T024's 5 s per 1 MiB, scaled by the
/// 4x envelope (the same floor of 1 MiB per 5 s).
pub const POLICY_SNAPSHOT_REASSEMBLY_MS_V3: u64 = 20_000;
/// Staging bytes per receiver: the whole v3 envelope, 4,194,304.
pub const POLICY_SNAPSHOT_STAGING_BYTES_PER_PEER_V3: usize = MAX_SNAPSHOT_BYTES_V3;
/// Staging bytes per pool: eight maximal transfers at once, 33,554,432.
pub const POLICY_SNAPSHOT_STAGING_BYTES_HOST_V3: usize =
    8 * POLICY_SNAPSHOT_STAGING_BYTES_PER_PEER_V3;

const _: () = assert!(
    POLICY_SNAPSHOT_REASSEMBLY_MS_V3
        == crate::protocol::POLICY_SNAPSHOT_REASSEMBLY_SECS
            * 1_000
            * (MAX_SNAPSHOT_BYTES_V3 / crate::protocol::MAX_SNAPSHOT_BYTES) as u64
);
const _: () = assert!(MAX_SNAPSHOT_CHUNKS_V3 <= 128);

/// A 128-chunk assembly of one `0x82` transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferAssemblyV3 {
    begin: BeginTransfer,
    buffer: Vec<u8>,
    bitmap: u128,
    received: u8,
}

impl TransferAssemblyV3 {
    /// Starts an assembly for a Begin that passes [`check_begin`].
    pub fn new(begin: &BeginTransfer) -> Result<Self, CodecError> {
        check_begin(begin)?;
        Ok(Self {
            begin: begin.clone(),
            buffer: vec![0u8; begin.total_bytes as usize],
            bitmap: 0,
            received: 0,
        })
    }

    /// The Begin this assembly was made for.
    pub fn begin(&self) -> &BeginTransfer {
        &self.begin
    }

    /// Stores one chunk. An `Err` or `Duplicate` changes nothing.
    pub fn accept(&mut self, chunk: &TransferChunk) -> Result<ChunkOutcome, AssemblyError> {
        if chunk.epoch != self.begin.epoch || chunk.transfer != self.begin.transfer {
            return Err(AssemblyError::WrongTransfer);
        }
        if chunk.index >= self.begin.chunk_count {
            return Err(AssemblyError::OutOfRange);
        }
        let expected_len = chunk_len(&self.begin, chunk.index).ok_or(AssemblyError::OutOfRange)?;
        if chunk.bytes.len() != expected_len {
            return Err(AssemblyError::WrongLength);
        }
        let bit = 1u128 << chunk.index;
        let chunk_size = self.begin.segments_per_chunk as usize * MAX_SEGMENT_BYTES;
        let start = chunk.index as usize * chunk_size;
        if self.bitmap & bit != 0 {
            let stored = &self.buffer[start..start + expected_len];
            return if stored == chunk.bytes.as_slice() {
                Ok(ChunkOutcome::Duplicate)
            } else {
                Err(AssemblyError::Conflict)
            };
        }
        self.buffer[start..start + expected_len].copy_from_slice(&chunk.bytes);
        self.bitmap |= bit;
        self.received += 1;
        if self.received == self.begin.chunk_count {
            Ok(ChunkOutcome::Complete)
        } else {
            Ok(ChunkOutcome::Stored)
        }
    }

    /// How many distinct chunks are stored.
    pub fn received_chunks(&self) -> u8 {
        self.received
    }

    /// True when every chunk of a nonempty transfer is stored.
    pub fn is_complete(&self) -> bool {
        self.received == self.begin.chunk_count && self.begin.chunk_count > 0
    }

    /// The joined bytes when complete; the assembly itself otherwise.
    pub fn into_bytes(self) -> Result<Vec<u8>, Self> {
        if self.received != self.begin.chunk_count {
            return Err(self);
        }
        Ok(self.buffer)
    }
}

/// The v3 counterpart of [`crate::reassembly::TransferLimits`]: same fields,
/// same order (field order is contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferLimitsV3 {
    /// Concurrent receivers, `1..=8`.
    pub max_receivers: usize,
    /// Reassembly deadline in milliseconds, `1..=20_000`.
    pub deadline_ms: u64,
    /// Staging bytes per receiver, `2_847_788..=4_194_304`.
    pub staging_bytes_per_receiver: usize,
    /// Staging bytes per pool, `staging_bytes_per_receiver..=33_554_432`.
    pub staging_bytes_total: usize,
    /// Held delta frames per receiver, `1..=128`.
    pub delta_frames_per_receiver: usize,
    /// Held delta bytes per receiver, `65_536..=2_097_152`.
    pub delta_bytes_per_receiver: usize,
    /// Held delta bytes per pool, `delta_bytes_per_receiver..=16_777_216`.
    pub delta_bytes_total: usize,
}

impl TransferLimitsV3 {
    /// The upper bound of every range: the v3 policy.
    pub fn v3() -> Self {
        Self {
            max_receivers: crate::protocol::POLICY_PROOF_PEERS,
            deadline_ms: POLICY_SNAPSHOT_REASSEMBLY_MS_V3,
            staging_bytes_per_receiver: POLICY_SNAPSHOT_STAGING_BYTES_PER_PEER_V3,
            staging_bytes_total: POLICY_SNAPSHOT_STAGING_BYTES_HOST_V3,
            delta_frames_per_receiver: crate::protocol::POLICY_EGRESS_FRAMES_PER_PEER,
            delta_bytes_per_receiver: crate::protocol::POLICY_EGRESS_BYTES_PER_PEER,
            delta_bytes_total: crate::protocol::POLICY_EGRESS_BYTES_HOST,
        }
    }

    /// Checks the fields in declaration order; first failure wins. Per field,
    /// zero or under its floor is `InvalidLimits` before an over-bound
    /// `LoosensPolicy`.
    pub fn validate(&self) -> Result<(), LimitsError> {
        if self.max_receivers == 0 {
            return Err(LimitsError::InvalidLimits {
                field: "max_receivers",
            });
        }
        if self.max_receivers > crate::protocol::POLICY_PROOF_PEERS {
            return Err(LimitsError::LoosensPolicy {
                field: "max_receivers",
            });
        }
        if self.deadline_ms == 0 {
            return Err(LimitsError::InvalidLimits {
                field: "deadline_ms",
            });
        }
        if self.deadline_ms > POLICY_SNAPSHOT_REASSEMBLY_MS_V3 {
            return Err(LimitsError::LoosensPolicy {
                field: "deadline_ms",
            });
        }
        if self.staging_bytes_per_receiver < MAX_ENCODED_BODY_BYTES_V3 {
            return Err(LimitsError::InvalidLimits {
                field: "staging_bytes_per_receiver",
            });
        }
        if self.staging_bytes_per_receiver > POLICY_SNAPSHOT_STAGING_BYTES_PER_PEER_V3 {
            return Err(LimitsError::LoosensPolicy {
                field: "staging_bytes_per_receiver",
            });
        }
        if self.staging_bytes_total < self.staging_bytes_per_receiver {
            return Err(LimitsError::InvalidLimits {
                field: "staging_bytes_total",
            });
        }
        if self.staging_bytes_total > POLICY_SNAPSHOT_STAGING_BYTES_HOST_V3 {
            return Err(LimitsError::LoosensPolicy {
                field: "staging_bytes_total",
            });
        }
        if self.delta_frames_per_receiver == 0 {
            return Err(LimitsError::InvalidLimits {
                field: "delta_frames_per_receiver",
            });
        }
        if self.delta_frames_per_receiver > crate::protocol::POLICY_EGRESS_FRAMES_PER_PEER {
            return Err(LimitsError::LoosensPolicy {
                field: "delta_frames_per_receiver",
            });
        }
        if self.delta_bytes_per_receiver < crate::protocol::MAX_DELTA_FRAME_BYTES {
            return Err(LimitsError::InvalidLimits {
                field: "delta_bytes_per_receiver",
            });
        }
        if self.delta_bytes_per_receiver > crate::protocol::POLICY_EGRESS_BYTES_PER_PEER {
            return Err(LimitsError::LoosensPolicy {
                field: "delta_bytes_per_receiver",
            });
        }
        if self.delta_bytes_total < self.delta_bytes_per_receiver {
            return Err(LimitsError::InvalidLimits {
                field: "delta_bytes_total",
            });
        }
        if self.delta_bytes_total > crate::protocol::POLICY_EGRESS_BYTES_HOST {
            return Err(LimitsError::LoosensPolicy {
                field: "delta_bytes_total",
            });
        }
        Ok(())
    }
}
