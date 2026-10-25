#![allow(missing_docs)]
//! Reassembly and cutover gate for snapshot transfer (T024).

use core::fmt;
use std::collections::BTreeMap;

use crate::codec::CodecError;
use crate::codec_transfer;
use crate::protocol::SEQ_LAST;
use crate::protocol_transfer::{
    BeginTransfer, ResyncReason, SnapshotBody, TransferChunk, TransferDown, TransferId,
    MAX_SEGMENT_BYTES,
};

// ---------------------------------------------------------------------------
// AssemblyError / ChunkOutcome
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssemblyError {
    WrongTransfer,
    OutOfRange,
    WrongLength,
    Conflict,
}

impl fmt::Display for AssemblyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::WrongTransfer => "WrongTransfer",
            Self::OutOfRange => "OutOfRange",
            Self::WrongLength => "WrongLength",
            Self::Conflict => "Conflict",
        };
        write!(f, "{} at transfer/assembly", name)
    }
}
impl std::error::Error for AssemblyError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkOutcome {
    Stored,
    Duplicate,
    Complete,
}

// ---------------------------------------------------------------------------
// TransferAssembly
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferAssembly {
    begin: BeginTransfer,
    buffer: Vec<u8>,
    bitmap: u32,
    received: u8,
}

impl TransferAssembly {
    pub fn new(begin: &BeginTransfer) -> Result<Self, CodecError> {
        begin.validate()?;
        let total = begin.total_bytes as usize;
        Ok(Self {
            begin: begin.clone(),
            buffer: vec![0u8; total],
            bitmap: 0,
            received: 0,
        })
    }
    pub fn begin(&self) -> &BeginTransfer {
        &self.begin
    }
    pub fn accept(&mut self, chunk: &TransferChunk) -> Result<ChunkOutcome, AssemblyError> {
        if chunk.epoch != self.begin.epoch || chunk.transfer != self.begin.transfer {
            return Err(AssemblyError::WrongTransfer);
        }
        if chunk.index >= self.begin.chunk_count {
            return Err(AssemblyError::OutOfRange);
        }
        let expected_len = self.begin.chunk_len(chunk.index).ok_or(AssemblyError::OutOfRange)?;
        if chunk.bytes.len() != expected_len {
            return Err(AssemblyError::WrongLength);
        }
        let bit = 1u32 << chunk.index;
        if self.bitmap & bit != 0 {
            let chunk_size = self.begin.segments_per_chunk as usize * MAX_SEGMENT_BYTES;
            let start = chunk.index as usize * chunk_size;
            let stored = &self.buffer[start..start + expected_len];
            if stored == chunk.bytes.as_slice() {
                return Ok(ChunkOutcome::Duplicate);
            } else {
                return Err(AssemblyError::Conflict);
            }
        }
        let chunk_size = self.begin.segments_per_chunk as usize * MAX_SEGMENT_BYTES;
        let start = chunk.index as usize * chunk_size;
        self.buffer[start..start + expected_len].copy_from_slice(&chunk.bytes);
        self.bitmap |= bit;
        self.received += 1;
        if self.received == self.begin.chunk_count {
            Ok(ChunkOutcome::Complete)
        } else {
            Ok(ChunkOutcome::Stored)
        }
    }
    pub fn received_chunks(&self) -> u8 {
        self.received
    }
    pub fn is_complete(&self) -> bool {
        self.received == self.begin.chunk_count && self.begin.chunk_count > 0
    }
    pub fn into_bytes(self) -> Result<Vec<u8>, Self> {
        if self.received != self.begin.chunk_count {
            return Err(self);
        }
        Ok(self.buffer)
    }
}

// ---------------------------------------------------------------------------
// TransferLimits
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferLimits {
    pub max_receivers: usize,
    pub deadline_ms: u64,
    pub staging_bytes_per_receiver: usize,
    pub staging_bytes_total: usize,
    pub delta_frames_per_receiver: usize,
    pub delta_bytes_per_receiver: usize,
    pub delta_bytes_total: usize,
}

impl TransferLimits {
    pub fn v1() -> Self {
        Self {
            max_receivers: 8,
            deadline_ms: 5_000,
            staging_bytes_per_receiver: crate::protocol::MAX_SNAPSHOT_BYTES,
            staging_bytes_total: 8 * 1_048_576,
            delta_frames_per_receiver: 128,
            delta_bytes_per_receiver: 2 * 1_048_576,
            delta_bytes_total: 16 * 1_048_576,
        }
    }
    pub fn validate(&self) -> Result<(), LimitsError> {
        if self.max_receivers == 0 {
            return Err(LimitsError::InvalidLimits { field: "max_receivers" });
        }
        if self.max_receivers > 8 {
            return Err(LimitsError::LoosensPolicy { field: "max_receivers" });
        }
        if self.deadline_ms == 0 {
            return Err(LimitsError::InvalidLimits { field: "deadline_ms" });
        }
        if self.deadline_ms > 5_000 {
            return Err(LimitsError::LoosensPolicy { field: "deadline_ms" });
        }
        if self.staging_bytes_per_receiver < 22_552 {
            return Err(LimitsError::InvalidLimits { field: "staging_bytes_per_receiver" });
        }
        if self.staging_bytes_per_receiver > crate::protocol::MAX_SNAPSHOT_BYTES {
            return Err(LimitsError::LoosensPolicy { field: "staging_bytes_per_receiver" });
        }
        if self.staging_bytes_total < self.staging_bytes_per_receiver {
            return Err(LimitsError::InvalidLimits { field: "staging_bytes_total" });
        }
        if self.staging_bytes_total > 8 * 1_048_576 {
            return Err(LimitsError::LoosensPolicy { field: "staging_bytes_total" });
        }
        if self.delta_frames_per_receiver == 0 {
            return Err(LimitsError::InvalidLimits { field: "delta_frames_per_receiver" });
        }
        if self.delta_frames_per_receiver > 128 {
            return Err(LimitsError::LoosensPolicy { field: "delta_frames_per_receiver" });
        }
        if self.delta_bytes_per_receiver < 65_536 {
            return Err(LimitsError::InvalidLimits { field: "delta_bytes_per_receiver" });
        }
        if self.delta_bytes_per_receiver > 2 * 1_048_576 {
            return Err(LimitsError::LoosensPolicy { field: "delta_bytes_per_receiver" });
        }
        if self.delta_bytes_total < self.delta_bytes_per_receiver {
            return Err(LimitsError::InvalidLimits { field: "delta_bytes_total" });
        }
        if self.delta_bytes_total > 16 * 1_048_576 {
            return Err(LimitsError::LoosensPolicy { field: "delta_bytes_total" });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitsError {
    InvalidLimits { field: &'static str },
    LoosensPolicy { field: &'static str },
}

impl fmt::Display for LimitsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits { field } => write!(f, "InvalidLimits at transfer/limits: {}", field),
            Self::LoosensPolicy { field } => write!(f, "LoosensPolicy at transfer/limits: {}", field),
        }
    }
}
impl std::error::Error for LimitsError {}

// ---------------------------------------------------------------------------
// DeltaWire etc.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaWire {
    V1,
    V2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateMode {
    Live,
    AwaitSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatedDelta {
    V1(crate::protocol::DeltaFrame),
    V2(crate::protocol_v2::DeltaFrame),
}

impl GatedDelta {
    pub fn event_seq(&self) -> u64 {
        match self {
            Self::V1(f) => f.event_seq,
            Self::V2(f) => f.event_seq,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
    pub transfer: TransferId,
    pub base_tick: u64,
    pub cursor: u64,
    pub body: SnapshotBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicaUpdate {
    Snapshot(Publication),
    Delta(GatedDelta),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiverId(u32);

impl ReceiverId {
    pub fn get(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferOutcome {
    Begun,
    Stored { received: u8, chunk_count: u8 },
    Duplicate,
    Published { transfer: TransferId, released: usize },
    Cancelled { discarded: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaOutcome {
    Released { count: usize },
    Buffered,
    Duplicate,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailReason {
    Conflict,
    InvalidBody(CodecError),
    Expired,
    Overflow,
}

impl FailReason {
    pub fn resync_reason(&self) -> ResyncReason {
        match self {
            Self::Expired => ResyncReason::Expired,
            Self::Conflict => ResyncReason::Invalid,
            Self::InvalidBody(_) => ResyncReason::Invalid,
            Self::Overflow => ResyncReason::Overflow,
        }
    }
}

impl fmt::Display for FailReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Conflict => "Conflict",
            Self::InvalidBody(_) => "InvalidBody",
            Self::Expired => "Expired",
            Self::Overflow => "Overflow",
        };
        write!(f, "{} at transfer/fail", name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferError {
    Codec(CodecError),
    UnknownReceiver,
    ReceiversFull,
    WrongEpoch,
    StaleTransfer,
    UnknownTransfer,
    InFlight,
    BehindReplica,
    StagingFull,
    AggregateFull,
    DeltaSequence,
    Failed(FailReason),
}

impl fmt::Display for TransferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Codec(_) => "Codec",
            Self::UnknownReceiver => "UnknownReceiver",
            Self::ReceiversFull => "ReceiversFull",
            Self::WrongEpoch => "WrongEpoch",
            Self::StaleTransfer => "StaleTransfer",
            Self::UnknownTransfer => "UnknownTransfer",
            Self::InFlight => "InFlight",
            Self::BehindReplica => "BehindReplica",
            Self::StagingFull => "StagingFull",
            Self::AggregateFull => "AggregateFull",
            Self::DeltaSequence => "DeltaSequence",
            Self::Failed(_) => "Failed",
        };
        write!(f, "{} at transfer/pool", name)
    }
}
impl std::error::Error for TransferError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InFlightStatus {
    pub transfer: TransferId,
    pub cursor: u64,
    pub base_tick: u64,
    pub received: u8,
    pub chunk_count: u8,
    pub deadline_ms: u64,
    pub reserved_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedBase {
    pub transfer: TransferId,
    pub base_tick: u64,
    pub cursor: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverStatus {
    pub epoch: [u8; 16],
    pub wire: DeltaWire,
    pub mode: GateMode,
    pub in_flight: Option<InFlightStatus>,
    pub retired_through: u64,
    pub published: Option<PublishedBase>,
    pub next_event_seq: u64,
    pub held_frames: usize,
    pub held_bytes: usize,
    pub queued_updates: usize,
    pub resync: Option<ResyncReason>,
}

// ---------------------------------------------------------------------------
// Internal
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct InFlightTransfer {
    assembly: TransferAssembly,
    reserved_bytes: usize,
    deadline_ms: u64,
    begin: BeginTransfer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HeldDelta {
    gated: GatedDelta,
    encoded_len: usize,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Receiver {
    id: ReceiverId,
    epoch: [u8; 16],
    wire: DeltaWire,
    mode: GateMode,
    in_flight: Option<InFlightTransfer>,
    retired_through: u64,
    published: Option<PublishedBase>,
    next_event_seq: u64,
    held: BTreeMap<u64, HeldDelta>,
    held_bytes: usize,
    queued: Vec<ReplicaUpdate>,
    queued_lens: Vec<usize>,
    queued_bytes: usize,
    resync: Option<ResyncReason>,
}

// ---------------------------------------------------------------------------
// ReassemblyPool
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReassemblyPool {
    limits: TransferLimits,
    now_ms: u64,
    receivers: BTreeMap<ReceiverId, Receiver>,
    receiver_counter: u32,
    staging_bytes: usize,
    held_delta_bytes: usize,
}

impl ReassemblyPool {
    pub fn new(limits: TransferLimits) -> Result<Self, LimitsError> {
        limits.validate()?;
        Ok(Self {
            limits,
            now_ms: 0,
            receivers: BTreeMap::new(),
            receiver_counter: 0,
            staging_bytes: 0,
            held_delta_bytes: 0,
        })
    }
    pub fn limits(&self) -> TransferLimits {
        self.limits.clone()
    }
    pub fn add_receiver(&mut self, epoch: [u8; 16], wire: DeltaWire, start: GateMode) -> Result<ReceiverId, TransferError> {
        if self.receivers.len() >= self.limits.max_receivers {
            return Err(TransferError::ReceiversFull);
        }
        if self.receiver_counter == u32::MAX {
            return Err(TransferError::ReceiversFull);
        }
        self.receiver_counter += 1;
        let rid = ReceiverId(self.receiver_counter);
        let recv = Receiver {
            id: rid,
            epoch,
            wire,
            mode: start,
            in_flight: None,
            retired_through: 0,
            published: None,
            next_event_seq: 1,
            held: BTreeMap::new(),
            held_bytes: 0,
            queued: Vec::new(),
            queued_lens: Vec::new(),
            queued_bytes: 0,
            resync: None,
        };
        self.receivers.insert(rid, recv);
        Ok(rid)
    }
    pub fn remove_receiver(&mut self, id: ReceiverId) {
        if let Some(recv) = self.receivers.remove(&id) {
            if let Some(inf) = recv.in_flight {
                self.staging_bytes = self.staging_bytes.saturating_sub(inf.reserved_bytes);
            }
            self.held_delta_bytes = self.held_delta_bytes.saturating_sub(recv.held_bytes + recv.queued_bytes);
        }
    }
    fn sweep(&mut self, now_ms: u64) -> Vec<(ReceiverId, TransferId)> {
        self.now_ms = self.now_ms.max(now_ms);
        let mut expired = Vec::new();
        let ids: Vec<ReceiverId> = self.receivers.keys().copied().collect();
        for rid in ids {
            let should = {
                let r = self.receivers.get(&rid).unwrap();
                if let Some(inf) = &r.in_flight {
                    now_ms >= inf.deadline_ms
                } else {
                    false
                }
            };
            if should {
                let tid = self.receivers.get(&rid).unwrap().in_flight.as_ref().unwrap().begin.transfer;
                self.fail_in_flight(rid, FailReason::Expired);
                expired.push((rid, tid));
            }
        }
        expired
    }
    fn fail_in_flight(&mut self, rid: ReceiverId, reason: FailReason) {
        if let Some(recv) = self.receivers.get_mut(&rid) {
            if let Some(inf) = recv.in_flight.take() {
                self.staging_bytes = self.staging_bytes.saturating_sub(inf.reserved_bytes);
                if inf.begin.transfer.get() > recv.retired_through {
                    recv.retired_through = inf.begin.transfer.get();
                }
            }
            recv.mode = GateMode::AwaitSnapshot;
            recv.resync = Some(reason.resync_reason());
            if matches!(reason, FailReason::Overflow) {
                // drop held
                self.held_delta_bytes = self.held_delta_bytes.saturating_sub(recv.held_bytes);
                recv.held_bytes = 0;
                recv.held.clear();
            }
        }
    }
    pub fn offer_transfer(&mut self, id: ReceiverId, bytes: &[u8], now_ms: u64) -> Result<TransferOutcome, TransferError> {
        let now_ms = now_ms.max(self.now_ms);
        self.now_ms = now_ms;
        let expired_pairs = self.sweep(now_ms);
        if !self.receivers.contains_key(&id) {
            return Err(TransferError::UnknownReceiver);
        }
        let msg = codec_transfer::decode_down(bytes).map_err(TransferError::Codec)?;
        let recv_epoch = self.receivers.get(&id).unwrap().epoch;
        let msg_epoch = match &msg {
            TransferDown::Begin(b) => b.epoch,
            TransferDown::Chunk(c) => c.epoch,
            TransferDown::Cancel { epoch, .. } => *epoch,
        };
        if msg_epoch != recv_epoch {
            return Err(TransferError::WrongEpoch);
        }
        let msg_tid = match &msg {
            TransferDown::Begin(b) => b.transfer,
            TransferDown::Chunk(c) => c.transfer,
            TransferDown::Cancel { transfer, .. } => *transfer,
        };
        if expired_pairs.iter().any(|(rid, tid)| *rid == id && *tid == msg_tid) {
            return Err(TransferError::Failed(FailReason::Expired));
        }
        match msg {
            TransferDown::Begin(begin) => self.handle_begin(id, begin, now_ms),
            TransferDown::Chunk(chunk) => self.handle_chunk(id, chunk),
            TransferDown::Cancel { transfer, .. } => self.handle_cancel(id, transfer),
        }
    }
    fn handle_begin(&mut self, id: ReceiverId, begin: BeginTransfer, now_ms: u64) -> Result<TransferOutcome, TransferError> {
        // step1 same id in-flight
        {
            let recv = self.receivers.get(&id).unwrap();
            if let Some(inf) = &recv.in_flight {
                if inf.begin.transfer == begin.transfer {
                    if inf.begin == begin {
                        return Ok(TransferOutcome::Duplicate);
                    } else {
                        self.fail_in_flight(id, FailReason::Conflict);
                        return Err(TransferError::Failed(FailReason::Conflict));
                    }
                }
            }
        }
        let recv = self.receivers.get(&id).unwrap();
        // step2 retired
        if begin.transfer.get() <= recv.retired_through {
            return Err(TransferError::StaleTransfer);
        }
        // step3 another in flight
        if recv.in_flight.is_some() {
            return Err(TransferError::InFlight);
        }
        // step4 behind replica
        {
            let behind_cursor = begin.cursor + 1 < recv.next_event_seq;
            let behind_tick = recv.published.as_ref().map(|p| begin.base_tick < p.base_tick).unwrap_or(false);
            if behind_cursor || behind_tick {
                return Err(TransferError::BehindReplica);
            }
        }
        // step5 staging per receiver
        if begin.total_bytes as usize > self.limits.staging_bytes_per_receiver {
            return Err(TransferError::StagingFull);
        }
        // step6 aggregate
        if self.staging_bytes + begin.total_bytes as usize > self.limits.staging_bytes_total {
            return Err(TransferError::AggregateFull);
        }
        // step7 accept
        let assembly = TransferAssembly::new(&begin).map_err(TransferError::Codec)?;
        let recv = self.receivers.get_mut(&id).unwrap();
        recv.in_flight = Some(InFlightTransfer {
            assembly,
            reserved_bytes: begin.total_bytes as usize,
            deadline_ms: now_ms.saturating_add(self.limits.deadline_ms),
            begin: begin.clone(),
        });
        recv.resync = None;
        self.staging_bytes += begin.total_bytes as usize;
        Ok(TransferOutcome::Begun)
    }
    fn handle_chunk(&mut self, id: ReceiverId, chunk: TransferChunk) -> Result<TransferOutcome, TransferError> {
        // check in-flight matching
        let in_flight_transfer_id = self.receivers.get(&id).and_then(|r| r.in_flight.as_ref().map(|i| i.begin.transfer));
        if let Some(inf_tid) = in_flight_transfer_id {
            if inf_tid == chunk.transfer {
                // accept
                let recv = self.receivers.get_mut(&id).unwrap();
                let inf = recv.in_flight.as_mut().unwrap();
                let outcome = inf.assembly.accept(&chunk);
                match outcome {
                    Err(AssemblyError::WrongTransfer) => {
                        self.fail_in_flight(id, FailReason::Conflict);
                        return Err(TransferError::Failed(FailReason::Conflict));
                    }
                    Err(AssemblyError::OutOfRange) => {
                        self.fail_in_flight(id, FailReason::Conflict);
                        return Err(TransferError::Failed(FailReason::Conflict));
                    }
                    Err(AssemblyError::WrongLength) => {
                        self.fail_in_flight(id, FailReason::Conflict);
                        return Err(TransferError::Failed(FailReason::Conflict));
                    }
                    Err(AssemblyError::Conflict) => {
                        self.fail_in_flight(id, FailReason::Conflict);
                        return Err(TransferError::Failed(FailReason::Conflict));
                    }
                    Ok(ChunkOutcome::Duplicate) => {
                        return Ok(TransferOutcome::Duplicate);
                    }
                    Ok(ChunkOutcome::Stored) => {
                        let recv = self.receivers.get(&id).unwrap();
                        let inf = recv.in_flight.as_ref().unwrap();
                        return Ok(TransferOutcome::Stored {
                            received: inf.assembly.received_chunks(),
                            chunk_count: inf.assembly.begin().chunk_count,
                        });
                    }
                    Ok(ChunkOutcome::Complete) => {
                        return self.publish(id);
                    }
                }
            }
        }
        // not in-flight matching
        let recv = self.receivers.get(&id).unwrap();
        if chunk.transfer.get() <= recv.retired_through {
            return Err(TransferError::StaleTransfer);
        }
        Err(TransferError::UnknownTransfer)
    }
    fn publish(&mut self, id: ReceiverId) -> Result<TransferOutcome, TransferError> {
        // take in-flight
        let recv = self.receivers.get_mut(&id).unwrap();
        let inf = recv.in_flight.take().unwrap();
        let transfer_id = inf.begin.transfer;
        let cursor = inf.begin.cursor;
        let base_tick = inf.begin.base_tick;
        let entity_count = inf.begin.entity_count;
        let reserved = inf.reserved_bytes;
        // step1 into_bytes + decode_body
        let bytes = match inf.assembly.into_bytes() {
            Ok(b) => b,
            Err(a) => {
                // incomplete shouldn't happen
                recv.in_flight = Some(InFlightTransfer { assembly: a, reserved_bytes: reserved, deadline_ms: inf.deadline_ms, begin: inf.begin.clone() });
                return Err(TransferError::Failed(FailReason::InvalidBody(CodecError::Malformed)));
            }
        };
        let body = match codec_transfer::decode_body(&bytes) {
            Ok(b) => b,
            Err(e) => {
                // fail
                self.staging_bytes = self.staging_bytes.saturating_sub(reserved);
                recv.retired_through = recv.retired_through.max(transfer_id.get());
                recv.mode = GateMode::AwaitSnapshot;
                recv.resync = Some(FailReason::InvalidBody(e).resync_reason());
                // held unchanged (not overflow)
                return Err(TransferError::Failed(FailReason::InvalidBody(e)));
            }
        };
        if body.entities.len() != entity_count as usize {
            let e = CodecError::Malformed;
            self.staging_bytes = self.staging_bytes.saturating_sub(reserved);
            recv.retired_through = recv.retired_through.max(transfer_id.get());
            recv.mode = GateMode::AwaitSnapshot;
            recv.resync = Some(FailReason::InvalidBody(e).resync_reason());
            return Err(TransferError::Failed(FailReason::InvalidBody(e)));
        }
        // step2 free reservation, retire, clear in-flight (already taken)
        self.staging_bytes = self.staging_bytes.saturating_sub(reserved);
        recv.retired_through = recv.retired_through.max(transfer_id.get());
        // step3 if queue holds Snapshot, clear whole queue
        let had_snapshot = recv.queued.iter().any(|u| matches!(u, ReplicaUpdate::Snapshot(_)));
        if had_snapshot {
            // free queued delta bytes
            self.held_delta_bytes = self.held_delta_bytes.saturating_sub(recv.queued_bytes);
            recv.queued_bytes = 0;
            recv.queued_lens.clear();
            recv.queued.clear();
        }
        // step4 drop held deltas with event_seq <= cursor
        let mut to_remove = Vec::new();
        for (seq, hd) in &recv.held {
            if *seq <= cursor {
                to_remove.push(*seq);
                self.held_delta_bytes = self.held_delta_bytes.saturating_sub(hd.encoded_len);
                recv.held_bytes = recv.held_bytes.saturating_sub(hd.encoded_len);
            }
        }
        for s in to_remove {
            recv.held.remove(&s);
        }
        // step5 set published etc
        recv.published = Some(PublishedBase {
            transfer: transfer_id,
            base_tick,
            cursor,
        });
        recv.next_event_seq = cursor.saturating_add(1);
        recv.mode = GateMode::Live;
        recv.resync = None;
        // step6 push snapshot
        let publication = Publication {
            transfer: transfer_id,
            base_tick,
            cursor,
            body,
        };
        recv.queued.push(ReplicaUpdate::Snapshot(publication));
        recv.queued_lens.push(0);
        // step7 move held deltas contiguous
        let mut released = 0usize;
        let mut seq = recv.next_event_seq;
        while let Some(hd) = recv.held.remove(&seq) {
            self.held_delta_bytes = self.held_delta_bytes.saturating_sub(hd.encoded_len);
            recv.held_bytes = recv.held_bytes.saturating_sub(hd.encoded_len);
            recv.queued.push(ReplicaUpdate::Delta(hd.gated));
            recv.queued_lens.push(hd.encoded_len);
            recv.queued_bytes += hd.encoded_len;
            self.held_delta_bytes += hd.encoded_len; // queued still counts in pool held_delta_bytes
            released += 1;
            seq += 1;
            recv.next_event_seq = seq;
        }
        // note: released counts moved; but the offered delta already? This publish path has no offered delta. released here is deltas moved. But TransferOutcome::Published.released should be number released by this publication. Case 13 doesn't specify released for publish? Case 23 says Published { released: 2 }. That's deltas moved (6,7). Good. But what about deltas already queued? They remain. released= moved held. Good.
        // Actually if no held moved, released=0. Case 13 last chunk returns Published; released maybe 0. Fine.
        // step8
        // Need return, but recv borrow ended? We still hold &mut recv. Return value uses transfer_id, released.
        Ok(TransferOutcome::Published { transfer: transfer_id, released })
    }
    fn handle_cancel(&mut self, id: ReceiverId, t: TransferId) -> Result<TransferOutcome, TransferError> {
        let recv = self.receivers.get_mut(&id).ok_or(TransferError::UnknownReceiver)?;
        let in_flight_id_le = recv.in_flight.as_ref().map(|i| i.begin.transfer <= t).unwrap_or(false);
        if in_flight_id_le {
            // discard
            if let Some(inf) = recv.in_flight.take() {
                self.staging_bytes = self.staging_bytes.saturating_sub(inf.reserved_bytes);
            }
            if t.get() > recv.retired_through {
                recv.retired_through = t.get();
            }
            recv.mode = GateMode::AwaitSnapshot;
            // resync not set
            Ok(TransferOutcome::Cancelled { discarded: true })
        } else {
            if t.get() > recv.retired_through {
                recv.retired_through = t.get();
            }
            Ok(TransferOutcome::Cancelled { discarded: false })
        }
    }
    pub fn offer_delta(&mut self, id: ReceiverId, bytes: &[u8], now_ms: u64) -> Result<DeltaOutcome, TransferError> {
        let now_ms = now_ms.max(self.now_ms);
        self.now_ms = now_ms;
        let _expired = self.sweep(now_ms);
        if !self.receivers.contains_key(&id) {
            return Err(TransferError::UnknownReceiver);
        }
        let wire = self.receivers.get(&id).unwrap().wire;
        let gated = match wire {
            DeltaWire::V1 => crate::codec::decode_delta(bytes).map(GatedDelta::V1),
            DeltaWire::V2 => crate::codec_v2::decode_delta(bytes).map(GatedDelta::V2),
        };
        let gated = match gated {
            Ok(g) => g,
            Err(e) => return Err(TransferError::Codec(e)),
        };
        let eseq = gated.event_seq();
        if eseq == 0 || eseq > SEQ_LAST {
            return Err(TransferError::DeltaSequence);
        }
        // superseded
        {
            let recv = self.receivers.get(&id).unwrap();
            if eseq < recv.next_event_seq {
                return Ok(DeltaOutcome::Superseded);
            }
        }
        // already held
        {
            let recv = self.receivers.get(&id).unwrap();
            if let Some(hd) = recv.held.get(&eseq) {
                if hd.bytes == bytes {
                    return Ok(DeltaOutcome::Duplicate);
                } else {
                    self.fail_in_flight(id, FailReason::Conflict);
                    return Err(TransferError::Failed(FailReason::Conflict));
                }
            }
        }
        // overflow check
        {
            let recv = self.receivers.get(&id).unwrap();
            let queued_delta_frames = recv.queued.iter().filter(|u| matches!(u, ReplicaUpdate::Delta(_))).count();
            let new_frames = recv.held.len() + queued_delta_frames + 1;
            let new_bytes_recv = recv.held_bytes + recv.queued_bytes + bytes.len();
            if new_frames > self.limits.delta_frames_per_receiver
                || new_bytes_recv > self.limits.delta_bytes_per_receiver
                || self.held_delta_bytes + bytes.len() > self.limits.delta_bytes_total
            {
                self.fail_in_flight(id, FailReason::Overflow);
                return Err(TransferError::Failed(FailReason::Overflow));
            }
        }
        // releasable
        let recv = self.receivers.get(&id).unwrap();
        let releasable = recv.mode == GateMode::Live
            && match &recv.in_flight {
                None => true,
                Some(inf) => eseq <= inf.begin.cursor,
            };
        let recv = self.receivers.get_mut(&id).unwrap();
        if releasable && eseq == recv.next_event_seq {
            // queue offered
            recv.queued.push(ReplicaUpdate::Delta(gated));
            recv.queued_lens.push(bytes.len());
            recv.queued_bytes += bytes.len();
            self.held_delta_bytes += bytes.len();
            recv.next_event_seq = eseq + 1;
            // drain contiguous held
            let mut count = 1usize;
            loop {
                let seq = recv.next_event_seq;
                let r = recv.mode == GateMode::Live
                    && match &recv.in_flight {
                        None => true,
                        Some(inf) => seq <= inf.begin.cursor,
                    };
                if !r { break; }
                let hd = match recv.held.remove(&seq) {
                    Some(h) => h,
                    None => break,
                };
                self.held_delta_bytes = self.held_delta_bytes.saturating_sub(hd.encoded_len);
                recv.held_bytes = recv.held_bytes.saturating_sub(hd.encoded_len);
                recv.queued.push(ReplicaUpdate::Delta(hd.gated));
                recv.queued_lens.push(hd.encoded_len);
                recv.queued_bytes += hd.encoded_len;
                self.held_delta_bytes += hd.encoded_len;
                count += 1;
                recv.next_event_seq = seq + 1;
            }
            Ok(DeltaOutcome::Released { count })
        } else {
            let hd = HeldDelta {
                gated,
                encoded_len: bytes.len(),
                bytes: bytes.to_vec(),
            };
            recv.held.insert(eseq, hd);
            recv.held_bytes += bytes.len();
            self.held_delta_bytes += bytes.len();
            Ok(DeltaOutcome::Buffered)
        }
    }

    pub fn poll(&mut self, now_ms: u64) -> Vec<ReceiverId> {
        let expired_pairs = self.sweep(now_ms.max(self.now_ms));
        // sweep already clamped now_ms? sweep sets now_ms max. But if now_ms smaller, sweep uses max. poll should clamp. We passed max. Good.
        let mut ids: Vec<ReceiverId> = expired_pairs.into_iter().map(|(r, _)| r).collect();
        ids.sort();
        ids
    }
    pub fn pop_update(&mut self, id: ReceiverId) -> Option<ReplicaUpdate> {
        let recv = self.receivers.get_mut(&id)?;
        if recv.queued.is_empty() {
            return None;
        }
        let upd = recv.queued.remove(0);
        // adjust bytes
        if let ReplicaUpdate::Delta(_) = &upd {
            // need encoded len; we stored queued_bytes aggregate. Subtract? We don't know per update.
            // We need per-update encoded len. ReplicaUpdate::Delta doesn't carry. Hmm.
        }
        // We'll handle bytes by recomputing? Let's instead store encoded len in a parallel queue.
        None // placeholder, fix below
    }
    // ...
}
