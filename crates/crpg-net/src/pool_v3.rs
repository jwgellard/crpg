//! Protocol v3 receiver gate: T024's reassembly and cutover pool for one v3
//! session (T140r1).
//!
//! [`ReassemblyPoolV3`] is a transliteration of
//! [`crate::reassembly::ReassemblyPool`] onto the v3 wire
//! ([`crate::transfer_v3`]'s `0x82` frames and [`crate::codec_v3`]'s delta
//! frames) and the v3 envelope ([`TransferLimitsV3`],
//! [`crate::reassembly_v3::TransferAssemblyV3`]). Every method body is
//! T024's with exactly the substitutions T140r1 P-1 lists; check order,
//! saturating arithmetic, the clamp-then-sweep prologue, `BTreeMap`
//! iteration order and FIFO queue handling are unchanged. It fixes no byte
//! and reads no op: a decoded v3 delta is gated by `event_seq` alone.
//!
//! One pool holds one wire. A v1/v2 session keeps
//! [`crate::reassembly::ReassemblyPool`]; a `ReassemblyPoolV3` refuses v1,
//! v2 and `0x81` bytes with `Codec(UnsupportedVersion)`, and the v1 pool
//! refuses v3 bytes the same way.

use std::collections::BTreeMap;

use crate::codec::CodecError;
use crate::codec_v3::decode_delta;
use crate::protocol::SEQ_LAST;
use crate::protocol_transfer::{BeginTransfer, TransferChunk, TransferDown, TransferId};
use crate::protocol_v3::DeltaFrame;
use crate::reassembly_v3::{AssemblyError, ChunkOutcome, TransferAssemblyV3};

pub use crate::protocol_transfer::ResyncReason;
pub use crate::reassembly::{
    DeltaOutcome, FailReason, GateMode, InFlightStatus, PublishedBase, Rebound, TransferError,
    TransferOutcome,
};
pub use crate::reassembly_v3::{LimitsError, TransferLimitsV3};

// ---------------------------------------------------------------------------
// Public types.
// ---------------------------------------------------------------------------

/// One published `0x82` snapshot: the transfer's identity and its decoded body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationV3 {
    /// The transfer the snapshot was published from.
    pub transfer: TransferId,
    /// The authoritative tick the snapshot was cut at.
    pub base_tick: u64,
    /// The delivery cursor the snapshot was cut at.
    pub cursor: u64,
    /// The decoded v3 snapshot body.
    pub body: crate::transfer_v3::SnapshotBodyV3,
}

/// What `pop_update` releases, in FIFO order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicaUpdateV3 {
    /// A decoded `0x82` snapshot body.
    Snapshot(PublicationV3),
    /// A decoded v3 delta frame.
    Delta(DeltaFrame),
}

/// A receiver of one [`ReassemblyPoolV3`]; ids start at 1 and are never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiverIdV3(u32);

impl ReceiverIdV3 {
    /// Returns the raw id.
    pub fn get(self) -> u32 {
        self.0
    }
}

/// `reassembly::ReceiverStatus` without `wire`; same fields, same order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverStatusV3 {
    /// The session epoch this receiver was added with.
    pub epoch: [u8; 16],
    /// The gate mode.
    pub mode: GateMode,
    /// The in-flight transfer, if any.
    pub in_flight: Option<InFlightStatus>,
    /// Every transfer id at or below this value is retired.
    pub retired_through: u64,
    /// The last published snapshot's identity.
    pub published: Option<PublishedBase>,
    /// The next `event_seq` the gate will release.
    pub next_event_seq: u64,
    /// Held plus queued delta frames.
    pub held_frames: usize,
    /// The encoded lengths of those delta frames.
    pub held_bytes: usize,
    /// Queued updates of every kind.
    pub queued_updates: usize,
    /// Why the receiver last failed closed, if it did.
    pub resync: Option<ResyncReason>,
}

// ---------------------------------------------------------------------------
// Internal.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct InFlightTransfer {
    assembly: TransferAssemblyV3,
    reserved_bytes: usize,
    deadline_ms: u64,
    begin: BeginTransfer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HeldDelta {
    frame: DeltaFrame,
    encoded_len: usize,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Receiver {
    id: ReceiverIdV3,
    epoch: [u8; 16],
    mode: GateMode,
    in_flight: Option<InFlightTransfer>,
    retired_through: u64,
    published: Option<PublishedBase>,
    next_event_seq: u64,
    held: BTreeMap<u64, HeldDelta>,
    held_bytes: usize,
    queued: Vec<ReplicaUpdateV3>,
    queued_lens: Vec<usize>,
    queued_bytes: usize,
    resync: Option<ResyncReason>,
}

// ---------------------------------------------------------------------------
// ReassemblyPoolV3
// ---------------------------------------------------------------------------

/// T024's reassembly and cutover gate for v3 sessions (`0x82`, v3 deltas).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReassemblyPoolV3 {
    limits: TransferLimitsV3,
    now_ms: u64,
    receivers: BTreeMap<ReceiverIdV3, Receiver>,
    receiver_counter: u32,
    staging_bytes: usize,
    held_delta_bytes: usize,
}

impl ReassemblyPoolV3 {
    /// Builds an empty pool, or returns `limits.validate()`'s error unchanged.
    pub fn new(limits: TransferLimitsV3) -> Result<Self, LimitsError> {
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

    /// Returns a value equal to the limits the pool was built with.
    pub fn limits(&self) -> TransferLimitsV3 {
        self.limits.clone()
    }

    /// Adds one receiver at `start`; ids start at 1 and are never reused.
    pub fn add_receiver(
        &mut self,
        epoch: [u8; 16],
        start: GateMode,
    ) -> Result<ReceiverIdV3, TransferError> {
        if self.receivers.len() >= self.limits.max_receivers {
            return Err(TransferError::ReceiversFull);
        }
        if self.receiver_counter == u32::MAX {
            return Err(TransferError::ReceiversFull);
        }
        self.receiver_counter += 1;
        let rid = ReceiverIdV3(self.receiver_counter);
        let recv = Receiver {
            id: rid,
            epoch,
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

    /// Removes a receiver and frees its staging reservation and delta bytes;
    /// an unknown id is a no-op.
    pub fn remove_receiver(&mut self, id: ReceiverIdV3) {
        if let Some(recv) = self.receivers.remove(&id) {
            if let Some(inf) = recv.in_flight {
                self.staging_bytes = self.staging_bytes.saturating_sub(inf.reserved_bytes);
            }
            self.held_delta_bytes = self
                .held_delta_bytes
                .saturating_sub(recv.held_bytes + recv.queued_bytes);
        }
    }

    fn sweep(&mut self, now_ms: u64) -> Vec<(ReceiverIdV3, TransferId)> {
        self.now_ms = self.now_ms.max(now_ms);
        let mut expired = Vec::new();
        let ids: Vec<ReceiverIdV3> = self.receivers.keys().copied().collect();
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
                let tid = self
                    .receivers
                    .get(&rid)
                    .unwrap()
                    .in_flight
                    .as_ref()
                    .unwrap()
                    .begin
                    .transfer;
                self.fail_in_flight(rid, FailReason::Expired);
                expired.push((rid, tid));
            }
        }
        expired
    }

    fn fail_in_flight(&mut self, rid: ReceiverIdV3, reason: FailReason) {
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

    /// Offers one downstream `0x82` frame; T024's check order, first failure
    /// wins.
    pub fn offer_transfer(
        &mut self,
        id: ReceiverIdV3,
        bytes: &[u8],
        now_ms: u64,
    ) -> Result<TransferOutcome, TransferError> {
        let now_ms = now_ms.max(self.now_ms);
        self.now_ms = now_ms;
        let expired_pairs = self.sweep(now_ms);
        if !self.receivers.contains_key(&id) {
            return Err(TransferError::UnknownReceiver);
        }
        let msg = crate::transfer_v3::decode_down(bytes).map_err(TransferError::Codec)?;
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
        if expired_pairs
            .iter()
            .any(|(rid, tid)| *rid == id && *tid == msg_tid)
        {
            return Err(TransferError::Failed(FailReason::Expired));
        }
        match msg {
            TransferDown::Begin(begin) => self.handle_begin(id, begin, now_ms),
            TransferDown::Chunk(chunk) => self.handle_chunk(id, chunk),
            TransferDown::Cancel { transfer, .. } => self.handle_cancel(id, transfer),
        }
    }

    fn handle_begin(
        &mut self,
        id: ReceiverIdV3,
        begin: BeginTransfer,
        now_ms: u64,
    ) -> Result<TransferOutcome, TransferError> {
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
            let behind_tick = recv
                .published
                .as_ref()
                .map(|p| begin.base_tick < p.base_tick)
                .unwrap_or(false);
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
        let assembly = TransferAssemblyV3::new(&begin).map_err(TransferError::Codec)?;
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

    fn handle_chunk(
        &mut self,
        id: ReceiverIdV3,
        chunk: TransferChunk,
    ) -> Result<TransferOutcome, TransferError> {
        // check in-flight matching
        let in_flight_transfer_id = self
            .receivers
            .get(&id)
            .and_then(|r| r.in_flight.as_ref().map(|i| i.begin.transfer));
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

    fn publish(&mut self, id: ReceiverIdV3) -> Result<TransferOutcome, TransferError> {
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
                recv.in_flight = Some(InFlightTransfer {
                    assembly: a,
                    reserved_bytes: reserved,
                    deadline_ms: inf.deadline_ms,
                    begin: inf.begin.clone(),
                });
                return Err(TransferError::Failed(FailReason::InvalidBody(
                    CodecError::Malformed,
                )));
            }
        };
        let body = match crate::transfer_v3::decode_body(&bytes) {
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
        let had_snapshot = recv
            .queued
            .iter()
            .any(|u| matches!(u, ReplicaUpdateV3::Snapshot(_)));
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
        let publication = PublicationV3 {
            transfer: transfer_id,
            base_tick,
            cursor,
            body,
        };
        recv.queued.push(ReplicaUpdateV3::Snapshot(publication));
        recv.queued_lens.push(0);
        // step7 move held deltas contiguous
        let mut released = 0usize;
        let mut seq = recv.next_event_seq;
        while let Some(hd) = recv.held.remove(&seq) {
            self.held_delta_bytes = self.held_delta_bytes.saturating_sub(hd.encoded_len);
            recv.held_bytes = recv.held_bytes.saturating_sub(hd.encoded_len);
            recv.queued.push(ReplicaUpdateV3::Delta(hd.frame));
            recv.queued_lens.push(hd.encoded_len);
            recv.queued_bytes += hd.encoded_len;
            self.held_delta_bytes += hd.encoded_len; // queued still counts in pool held_delta_bytes
            released += 1;
            seq += 1;
            recv.next_event_seq = seq;
        }
        // step8
        Ok(TransferOutcome::Published {
            transfer: transfer_id,
            released,
        })
    }

    fn handle_cancel(
        &mut self,
        id: ReceiverIdV3,
        t: TransferId,
    ) -> Result<TransferOutcome, TransferError> {
        let recv = self
            .receivers
            .get_mut(&id)
            .ok_or(TransferError::UnknownReceiver)?;
        let in_flight_id_le = recv
            .in_flight
            .as_ref()
            .map(|i| i.begin.transfer <= t)
            .unwrap_or(false);
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

    /// Offers one downstream v3 delta frame; T024's check order, first failure
    /// wins.
    pub fn offer_delta(
        &mut self,
        id: ReceiverIdV3,
        bytes: &[u8],
        now_ms: u64,
    ) -> Result<DeltaOutcome, TransferError> {
        let now_ms = now_ms.max(self.now_ms);
        self.now_ms = now_ms;
        let _expired = self.sweep(now_ms);
        if !self.receivers.contains_key(&id) {
            return Err(TransferError::UnknownReceiver);
        }
        let frame = match decode_delta(bytes) {
            Ok(f) => f,
            Err(e) => return Err(TransferError::Codec(e)),
        };
        let eseq = frame.event_seq;
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
            let queued_delta_frames = recv
                .queued
                .iter()
                .filter(|u| matches!(u, ReplicaUpdateV3::Delta(_)))
                .count();
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
            recv.queued.push(ReplicaUpdateV3::Delta(frame));
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
                if !r {
                    break;
                }
                let hd = match recv.held.remove(&seq) {
                    Some(h) => h,
                    None => break,
                };
                self.held_delta_bytes = self.held_delta_bytes.saturating_sub(hd.encoded_len);
                recv.held_bytes = recv.held_bytes.saturating_sub(hd.encoded_len);
                recv.queued.push(ReplicaUpdateV3::Delta(hd.frame));
                recv.queued_lens.push(hd.encoded_len);
                recv.queued_bytes += hd.encoded_len;
                self.held_delta_bytes += hd.encoded_len;
                count += 1;
                recv.next_event_seq = seq + 1;
            }
            Ok(DeltaOutcome::Released { count })
        } else {
            let hd = HeldDelta {
                frame,
                encoded_len: bytes.len(),
                bytes: bytes.to_vec(),
            };
            recv.held.insert(eseq, hd);
            recv.held_bytes += bytes.len();
            self.held_delta_bytes += bytes.len();
            Ok(DeltaOutcome::Buffered)
        }
    }

    /// Sweeps every receiver and returns the receivers this call expired,
    /// ascending.
    pub fn poll(&mut self, now_ms: u64) -> Vec<ReceiverIdV3> {
        let expired_pairs = self.sweep(now_ms.max(self.now_ms));
        let mut ids: Vec<ReceiverIdV3> = expired_pairs.into_iter().map(|(r, _)| r).collect();
        ids.sort();
        ids
    }

    /// Removes and returns the oldest queued update; `None` when empty or
    /// unknown.
    pub fn pop_update(&mut self, id: ReceiverIdV3) -> Option<ReplicaUpdateV3> {
        let recv = self.receivers.get_mut(&id)?;
        if recv.queued.is_empty() {
            return None;
        }
        let len = if recv.queued_lens.is_empty() {
            0
        } else {
            recv.queued_lens.remove(0)
        };
        let upd = recv.queued.remove(0);
        recv.queued_bytes = recv.queued_bytes.saturating_sub(len);
        self.held_delta_bytes = self.held_delta_bytes.saturating_sub(len);
        Some(upd)
    }

    /// Returns one receiver's status, or `UnknownReceiver`.
    pub fn status(&self, id: ReceiverIdV3) -> Result<ReceiverStatusV3, TransferError> {
        let recv = self
            .receivers
            .get(&id)
            .ok_or(TransferError::UnknownReceiver)?;
        let in_flight = recv.in_flight.as_ref().map(|inf| InFlightStatus {
            transfer: inf.begin.transfer,
            cursor: inf.begin.cursor,
            base_tick: inf.begin.base_tick,
            received: inf.assembly.received_chunks(),
            chunk_count: inf.assembly.begin().chunk_count,
            deadline_ms: inf.deadline_ms,
            reserved_bytes: inf.reserved_bytes,
        });
        let queued_deltas = recv
            .queued
            .iter()
            .filter(|u| matches!(u, ReplicaUpdateV3::Delta(_)))
            .count();
        Ok(ReceiverStatusV3 {
            epoch: recv.epoch,
            mode: recv.mode,
            in_flight,
            retired_through: recv.retired_through,
            published: recv.published.clone(),
            next_event_seq: recv.next_event_seq,
            held_frames: recv.held.len() + queued_deltas,
            held_bytes: recv.held_bytes + recv.queued_bytes,
            queued_updates: recv.queued.len(),
            resync: recv.resync,
        })
    }

    /// The pool's staging reservation total.
    pub fn staging_bytes(&self) -> usize {
        self.staging_bytes
    }

    /// The pool's held plus queued delta bytes.
    pub fn held_delta_bytes(&self) -> usize {
        self.held_delta_bytes
    }

    /// Re-anchor a retained receiver for a resumed connection (S§16).
    ///
    /// In one call, in this order: clamp `now_ms` and run the pool-wide
    /// expiry sweep; unknown `id` is `UnknownReceiver`; discard and retire the
    /// in-flight transfer; drop every held delta; set `AwaitSnapshot` and
    /// clear `resync`. `epoch`, `published`, `next_event_seq`,
    /// `retired_through` and every already-queued update are kept.
    pub fn rebind_receiver(
        &mut self,
        id: ReceiverIdV3,
        now_ms: u64,
    ) -> Result<Rebound, TransferError> {
        let _ = self.sweep(now_ms.max(self.now_ms));
        let recv = self
            .receivers
            .get_mut(&id)
            .ok_or(TransferError::UnknownReceiver)?;
        let mut discarded = None;
        if let Some(inf) = recv.in_flight.take() {
            self.staging_bytes = self.staging_bytes.saturating_sub(inf.reserved_bytes);
            if inf.begin.transfer.get() > recv.retired_through {
                recv.retired_through = inf.begin.transfer.get();
            }
            discarded = Some(inf.begin.transfer);
        }
        let dropped_deltas = recv.held.len();
        self.held_delta_bytes = self.held_delta_bytes.saturating_sub(recv.held_bytes);
        recv.held_bytes = 0;
        recv.held.clear();
        recv.mode = GateMode::AwaitSnapshot;
        recv.resync = None;
        Ok(Rebound {
            discarded,
            dropped_deltas,
        })
    }
}
