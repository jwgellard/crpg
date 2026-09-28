//! Deterministic in-memory simulated transport (T018b).
//!
//! The byte fabric between net endpoints in one process: bounded duplex
//! staging per peer, injected time, and reproducible fault schedules. It
//! proves admission, backpressure, retry, and ordering behavior without a
//! real network; reliable-ordered lane-0 delivery with loss/dup/reorder
//! injection, exact ingress/egress accounting, and per-lane seq/cache state
//! that later host and QUIC work consume unchanged.
//!
//! Fallible paths map to the frozen [`RejectionCode`]: unknown peers read as
//! [`RejectionCode::Unauthenticated`], exhausted budgets as
//! [`RejectionCode::RateLimited`], oversized datagrams as
//! [`RejectionCode::FrameTooLarge`], full queues as
//! [`RejectionCode::QueueFull`], and a full peer table as
//! [`RejectionCode::ServerBusy`]. No sleep, no threads, no I/O, no wall
//! clock: time enters only through [`SimClock`], randomness only through the
//! seeded `"net-fault"` stream.
//!
//! The transport stages and pumps bytes; it never decodes them. Admission
//! past the byte level (epoch, per-lane seq/cache, freshness window,
//! mapping/visibility, gameplay legality) belongs to the net-local
//! test-double drivers (T018b/T018c suites) and later the real host, which
//! keep their per-peer [`PeerState`] — never to this fabric.

use std::collections::{BTreeMap, VecDeque};

use crpg_core::DeterministicRng;

use crate::protocol::{
    ReceiptStatus, RejectionCode, LANE_COMBAT, MAX_DELTA_FRAME_BYTES, POLICY_EGRESS_BYTES_HOST,
    POLICY_EGRESS_BYTES_PER_PEER, POLICY_EGRESS_FRAMES_PER_PEER, POLICY_FAIL_BYTES_PER_SEC,
    POLICY_FAIL_BYTE_BURST, POLICY_FAIL_FRAMES_PER_SEC, POLICY_FAIL_FRAME_BURST,
    POLICY_INGRESS_BYTES_HOST, POLICY_INGRESS_BYTES_PER_PEER, POLICY_INGRESS_FRAMES_HOST,
    POLICY_INGRESS_FRAMES_PER_PEER, POLICY_INTENT_BYTES_PER_SEC, POLICY_INTENT_BYTE_BURST,
    POLICY_INTENT_FRAMES_PER_SEC, POLICY_INTENT_FRAME_BURST, POLICY_PROOF_PEERS, SEQ_FIRST,
    SEQ_LAST,
};

// ---------------------------------------------------------------------------
// Injected time.
// ---------------------------------------------------------------------------

/// Injected millisecond clock; the transport's only time source.
///
/// Never `SystemTime`: tests and drivers advance this explicitly, so a paused
/// world cannot freeze or flood token budgets. `World` tick never drives
/// replenishment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimClock(pub u64);

impl SimClock {
    /// Returns the current injected millisecond timestamp.
    pub fn now(&self) -> u64 {
        self.0
    }

    /// Advances the clock by `dt_ms`, saturating rather than wrapping.
    pub fn advance(&mut self, dt_ms: u64) {
        self.0 = self.0.saturating_add(dt_ms);
    }
}

// ---------------------------------------------------------------------------
// Capacity and rate configuration.
// ---------------------------------------------------------------------------

/// Bounded-queue depths: per-peer ceilings plus whole-host ceilings.
///
/// Both the frame count AND the byte total apply; a send past either ceiling
/// fails with [`RejectionCode::QueueFull`] and grows nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueCaps {
    /// Maximum staged datagrams for one peer.
    pub per_peer_frames: usize,
    /// Maximum staged bytes for one peer.
    pub per_peer_bytes: usize,
    /// Maximum staged datagrams across all peers.
    pub host_frames: usize,
    /// Maximum staged bytes across all peers.
    pub host_bytes: usize,
}

impl QueueCaps {
    /// v1 defaults from the T018a ingress policy; tightening-only overrides
    /// are the caller's to construct field by field.
    pub fn v1() -> Self {
        Self {
            per_peer_frames: POLICY_INGRESS_FRAMES_PER_PEER,
            per_peer_bytes: POLICY_INGRESS_BYTES_PER_PEER,
            host_frames: POLICY_INGRESS_FRAMES_HOST,
            host_bytes: POLICY_INGRESS_BYTES_HOST,
        }
    }

    /// Egress-side depths from the T018a egress policy, for drivers that
    /// account the host-to-peer direction against its own ceilings.
    pub fn v1_egress() -> Self {
        Self {
            per_peer_frames: POLICY_EGRESS_FRAMES_PER_PEER,
            per_peer_bytes: POLICY_EGRESS_BYTES_PER_PEER,
            host_frames: usize::MAX,
            host_bytes: POLICY_EGRESS_BYTES_HOST,
        }
    }
}

/// Token-bucket rates: sustained throughput plus burst allowance.
///
/// Frame and byte buckets are independent; an attempt deducts from each
/// budget separately, so retries and invalid attempts count toward ingress
/// exactly like first attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateCaps {
    /// Sustained datagrams per second.
    pub frames_per_sec: u32,
    /// Burst allowance in datagrams.
    pub burst_frames: u32,
    /// Sustained bytes per second.
    pub bytes_per_sec: u32,
    /// Burst allowance in bytes.
    pub burst_bytes: u32,
    /// Sustained failure responses per second (cached receipts included).
    pub fail_per_sec: u32,
    /// Burst allowance in failure responses.
    pub burst_fail: u32,
    /// Sustained failure-response bytes per second.
    pub fail_bytes_per_sec: u32,
    /// Burst allowance in failure-response bytes.
    pub burst_fail_bytes: u32,
}

impl RateCaps {
    /// v1 defaults from the T018a rate policy; tightening-only overrides are
    /// the caller's to construct field by field.
    pub fn v1() -> Self {
        Self {
            frames_per_sec: POLICY_INTENT_FRAMES_PER_SEC as u32,
            burst_frames: POLICY_INTENT_FRAME_BURST as u32,
            bytes_per_sec: POLICY_INTENT_BYTES_PER_SEC as u32,
            burst_bytes: POLICY_INTENT_BYTE_BURST as u32,
            fail_per_sec: POLICY_FAIL_FRAMES_PER_SEC as u32,
            burst_fail: POLICY_FAIL_FRAME_BURST as u32,
            fail_bytes_per_sec: POLICY_FAIL_BYTES_PER_SEC as u32,
            burst_fail_bytes: POLICY_FAIL_BYTE_BURST as u32,
        }
    }
}

/// Integer token bucket over injected time.
///
/// Accounting is in milli-tokens with saturating arithmetic, so any
/// partition of the same elapsed time credits exactly the same total:
/// advancing 10 ms ten times replenishes exactly what one 100 ms advance
/// does. Buckets start full. A backwards timestamp accrues nothing (time is
/// assumed monotonic-injected; the clock only moves forward by `advance`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenBucket {
    /// Capacity in milli-tokens (tokens times 1,000).
    capacity_mt: u64,
    /// Refill rate in milli-tokens per millisecond (tokens per second).
    refill_mt_per_ms: u64,
    /// Current balance in milli-tokens.
    available_mt: u64,
    /// Last timestamp credit was computed for.
    last_ms: u64,
}

impl TokenBucket {
    /// Builds a full bucket holding `burst` tokens and refilling `per_sec`
    /// tokens per second.
    pub fn new(burst: u64, per_sec: u64) -> Self {
        let capacity_mt = burst.saturating_mul(1_000);
        Self {
            capacity_mt,
            refill_mt_per_ms: per_sec,
            available_mt: capacity_mt,
            last_ms: 0,
        }
    }

    /// Attempts to spend `amount` tokens at `now_ms`.
    ///
    /// Credit for elapsed time accrues first (capped at capacity) whether or
    /// not the spend succeeds; only a successful spend deducts.
    pub fn consume(&mut self, now_ms: u64, amount: u64) -> bool {
        let elapsed = now_ms.saturating_sub(self.last_ms);
        self.last_ms = now_ms;
        self.available_mt = self
            .available_mt
            .saturating_add(elapsed.saturating_mul(self.refill_mt_per_ms))
            .min(self.capacity_mt);
        let need = amount.saturating_mul(1_000);
        if self.available_mt >= need {
            self.available_mt -= need;
            true
        } else {
            false
        }
    }

    /// Whole tokens currently available (floored; for assertions).
    pub fn available(&self) -> u64 {
        self.available_mt / 1_000
    }
}

// ---------------------------------------------------------------------------
// Peer identity and per-peer admission-adjacent state.
// ---------------------------------------------------------------------------

/// Opaque authenticated-peer handle, issued by [`InMemoryTransport::add_peer`].
///
/// Monotonic and never reused within one transport lifetime (matching the
/// `NetId` no-reuse discipline); removal frees the slot but retires the id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeerId(u32);

impl PeerId {
    /// Returns the raw handle value.
    pub fn get(self) -> u32 {
        self.0
    }
}

/// Per-peer lane-0 progress kept by the net-local drivers.
///
/// The transport fabric never reads this: drivers (test doubles now, the
/// host later) keep one per [`PeerId`] and implement epoch match, per-lane
/// seq/cache retry semantics, freshness window, and failure-response budgets
/// against it. Failure budgets here cover cached rejection receipts as well
/// as fresh ones; both share the egress queue caps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerState {
    /// Session generation bound to this peer.
    pub epoch: [u8; 16],
    /// Lane this state tracks (`LANE_COMBAT` only in v1).
    pub lane: u8,
    /// Next new per-lane seq; starts at 1 and never wraps (`SeqGap` does not
    /// advance it; exhaustion requires a new epoch).
    pub next_new_seq: u64,
    /// Last finalized `(seq, canonical intent bytes, terminal outcome)` for
    /// cached-retry delivery without re-execution.
    pub cache: Vec<(u64, Vec<u8>, ReceiptStatus)>,
    /// Failure-response frame budget for this peer.
    pub fail_budget: TokenBucket,
    /// Failure-response byte budget for this peer.
    pub byte_budget: TokenBucket,
}

impl PeerState {
    /// Binds `epoch` on the combat lane with empty seq/cache state and full
    /// failure-response budgets from `rates`.
    pub fn new(epoch: [u8; 16], rates: &RateCaps) -> Self {
        Self {
            epoch,
            lane: LANE_COMBAT,
            next_new_seq: SEQ_FIRST,
            cache: Vec::new(),
            fail_budget: TokenBucket::new(
                u64::from(rates.burst_fail),
                u64::from(rates.fail_per_sec),
            ),
            byte_budget: TokenBucket::new(
                u64::from(rates.burst_fail_bytes),
                u64::from(rates.fail_bytes_per_sec),
            ),
        }
    }

    /// Whether the lane is exhausted: every issuable `seq` (`SEQ_FIRST..=SEQ_LAST`)
    /// is finalized, so no further new command is representable on this epoch.
    ///
    /// Drivers check this after cache lookup and before gap/stale/new dispatch:
    /// cached retries still return their retained outcome, but any other
    /// non-cached admit on an exhausted lane reports
    /// [`RejectionCode::SeqExhausted`] (consuming no `seq`, closing nothing)
    /// so the peer rebinds to a fresh epoch instead of wrapping. The codec
    /// rejects `u64::MAX` as malformed, so exhaustion surfaces here from
    /// driver state, never from decoding `MAX`.
    pub fn is_exhausted(&self) -> bool {
        self.next_new_seq > SEQ_LAST
    }

    /// Attempts to spend one failure response (`1` frame plus `byte_len` bytes)
    /// at `now_ms`.
    ///
    /// Both budgets deduct per attempt — retries and invalid attempts
    /// included — mirroring the fabric ingress accounting, so neither side
    /// short-circuits the other. Returns `false` (spending nothing further)
    /// when either budget is empty: the driver drops the response bytes while
    /// the terminal outcome stays retained under the sequence policy, and the
    /// sender retries to recover it. Normal outbound deltas never touch these
    /// budgets; they share only the egress queue caps.
    pub fn try_consume_failure(&mut self, now_ms: u64, byte_len: u64) -> bool {
        let frames_ok = self.fail_budget.consume(now_ms, 1);
        let bytes_ok = self.byte_budget.consume(now_ms, byte_len);
        frames_ok && bytes_ok
    }
}

// ---------------------------------------------------------------------------
// Fault schedules.
// ---------------------------------------------------------------------------

/// Reproducible delivery faults applied by [`InMemoryTransport::tick`].
///
/// Every field is deterministic: periodic loss/duplication count datagrams
/// per peer from 1 (the `n`th, `2n`th, … datagram is affected; `None` or `0`
/// disables), and reorder rotates each tick's surviving batch in consecutive
/// blocks of `reorder_depth + 1` by amounts drawn from the seeded
/// `"net-fault"` stream (`0` disables), so no datagram moves more than
/// `reorder_depth` places and batches never cross a tick boundary. Same seed
/// plus same send/tick sequence delivers the same byte sequence on the same
/// byte sequence on the same build. Reliable-ordered lane-0 traffic recovers
/// by retry/dedup at the driver level; an intentionally unreliable lane is
/// never used for combat commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaultSchedule {
    /// Drop every `n`th datagram per peer; `None` or `0` disables.
    pub loss_every: Option<u64>,
    /// Deliver every `n`th datagram twice in a row; `None` or `0` disables.
    pub dup_every: Option<u64>,
    /// Maximum reorder displacement within one tick's batch; `0` disables.
    pub reorder_depth: usize,
    /// Master seed for the `"net-fault"` stream.
    pub seed: u64,
}

impl FaultSchedule {
    /// Clean delivery: no loss, duplication, or reorder.
    pub fn clean() -> Self {
        Self {
            loss_every: None,
            dup_every: None,
            reorder_depth: 0,
            seed: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// The fabric.
// ---------------------------------------------------------------------------

/// One peer's staged datagrams plus its ingress budgets.
///
/// The fabric keys peers by handle and never decodes bytes, so the session
/// binding for `add_peer`'s epoch lives in the drivers' [`PeerState`], not
/// here; the parameter stays for the approved contract and future host use.
#[derive(Debug)]
struct PeerEntry {
    /// Staged but not yet pumped.
    pending: VecDeque<Vec<u8>>,
    /// Pumped and awaiting `recv_from`.
    ready: VecDeque<Vec<u8>>,
    /// Staged datagram count (`pending` plus `ready`) against frame caps.
    staged_frames: usize,
    /// Staged byte total (`pending` plus `ready`) against byte caps.
    staged_bytes: usize,
    /// Ingress datagram budget.
    frame_budget: TokenBucket,
    /// Ingress byte budget.
    byte_budget: TokenBucket,
    /// Lifetime datagram counter for periodic faults (starts at 0; the first
    /// staged datagram is number 1).
    datagrams: u64,
}

/// Deterministic in-memory delivery fabric for lane-0 bytes.
///
/// `send_to` stages one datagram under ingress rate budgets and per-peer /
/// host queue caps; `tick` pumps staged datagrams through the fault schedule
/// into per-peer ready queues; `recv_from` takes the next ready datagram and
/// releases its accounting. Staged datagrams count against the caps until
/// received or the peer is removed, so slow peers terminate with
/// [`RejectionCode::QueueFull`] and reliable traffic is never silently lost.
/// Peers iterate in ascending [`PeerId`] order, keeping multi-peer pumps
/// deterministic.
#[derive(Debug)]
pub struct InMemoryTransport {
    caps: QueueCaps,
    rates: RateCaps,
    clock: SimClock,
    peers: BTreeMap<PeerId, PeerEntry>,
    next_peer: u32,
    staged_host_frames: usize,
    staged_host_bytes: usize,
    fault_seed: Option<u64>,
    fault_rng: DeterministicRng,
}

impl InMemoryTransport {
    /// Builds an empty fabric; the fault stream seeds lazily from the first
    /// [`FaultSchedule`] passed to [`tick`](Self::tick).
    pub fn new(caps: QueueCaps, rates: RateCaps, clock: SimClock) -> Self {
        Self {
            caps,
            rates,
            clock,
            peers: BTreeMap::new(),
            next_peer: 0,
            staged_host_frames: 0,
            staged_host_bytes: 0,
            fault_seed: None,
            fault_rng: DeterministicRng::from_seed(0),
        }
    }

    /// Borrows the injected clock; the only way time enters the fabric.
    pub fn clock_mut(&mut self) -> &mut SimClock {
        &mut self.clock
    }

    /// Admits one authenticated peer on `epoch`, or
    /// [`RejectionCode::ServerBusy`] when the proof-population table is full.
    ///
    /// The fabric keys the peer by handle; the session binding itself is
    /// recorded by drivers in [`PeerState`], which is why this parameter is
    /// intentionally unused past the approved signature.
    pub fn add_peer(&mut self, _epoch: [u8; 16]) -> Result<PeerId, RejectionCode> {
        if self.peers.len() >= POLICY_PROOF_PEERS {
            return Err(RejectionCode::ServerBusy);
        }
        let raw = self.next_peer;
        self.next_peer = self
            .next_peer
            .checked_add(1)
            .ok_or(RejectionCode::ServerBusy)?;
        let id = PeerId(raw);
        self.peers.insert(
            id,
            PeerEntry {
                pending: VecDeque::new(),
                ready: VecDeque::new(),
                staged_frames: 0,
                staged_bytes: 0,
                frame_budget: TokenBucket::new(
                    u64::from(self.rates.burst_frames),
                    u64::from(self.rates.frames_per_sec),
                ),
                byte_budget: TokenBucket::new(
                    u64::from(self.rates.burst_bytes),
                    u64::from(self.rates.bytes_per_sec),
                ),
                datagrams: 0,
            },
        );
        Ok(id)
    }

    /// Drops a peer, releasing its staged accounting; unknown ids are a no-op.
    pub fn remove_peer(&mut self, peer: PeerId) {
        if let Some(entry) = self.peers.remove(&peer) {
            self.staged_host_frames = self.staged_host_frames.saturating_sub(entry.staged_frames);
            self.staged_host_bytes = self.staged_host_bytes.saturating_sub(entry.staged_bytes);
        }
    }

    /// Stages one datagram for `peer`.
    ///
    /// In order: unknown peers read as [`RejectionCode::Unauthenticated`]
    /// (no authenticated binding exists); ingress budgets deduct per attempt
    /// — retries and invalid attempts included — else
    /// [`RejectionCode::RateLimited`]; datagrams over the fabric MTU
    /// ([`MAX_DELTA_FRAME_BYTES`], the largest v1 frame; intent frames are
    /// further bound at decode) read as [`RejectionCode::FrameTooLarge`];
    /// then per-peer and host frame/byte ceilings else
    /// [`RejectionCode::QueueFull`], growing nothing. Checked addition fails
    /// closed to `QueueFull`.
    pub fn send_to(&mut self, peer: PeerId, bytes: Vec<u8>) -> Result<(), RejectionCode> {
        let entry = self
            .peers
            .get_mut(&peer)
            .ok_or(RejectionCode::Unauthenticated)?;
        let now = self.clock.now();
        // Both budgets deduct per attempt — retries and invalid attempts
        // included — so neither side short-circuits the other.
        let frames_ok = entry.frame_budget.consume(now, 1);
        let bytes_ok = entry.byte_budget.consume(now, bytes.len() as u64);
        if !frames_ok || !bytes_ok {
            return Err(RejectionCode::RateLimited);
        }
        if bytes.len() > MAX_DELTA_FRAME_BYTES {
            return Err(RejectionCode::FrameTooLarge);
        }
        let frames = entry
            .staged_frames
            .checked_add(1)
            .ok_or(RejectionCode::QueueFull)?;
        let peer_bytes = entry
            .staged_bytes
            .checked_add(bytes.len())
            .ok_or(RejectionCode::QueueFull)?;
        let host_frames = self
            .staged_host_frames
            .checked_add(1)
            .ok_or(RejectionCode::QueueFull)?;
        let host_bytes = self
            .staged_host_bytes
            .checked_add(bytes.len())
            .ok_or(RejectionCode::QueueFull)?;
        if frames > self.caps.per_peer_frames
            || peer_bytes > self.caps.per_peer_bytes
            || host_frames > self.caps.host_frames
            || host_bytes > self.caps.host_bytes
        {
            return Err(RejectionCode::QueueFull);
        }
        entry.staged_frames = frames;
        entry.staged_bytes = peer_bytes;
        self.staged_host_frames = host_frames;
        self.staged_host_bytes = host_bytes;
        entry.pending.push_back(bytes);
        Ok(())
    }

    /// Takes the next pumped datagram for `peer`, releasing its accounting,
    /// or [`None`] when nothing is ready (unknown peers included).
    pub fn recv_from(&mut self, peer: PeerId) -> Option<Vec<u8>> {
        let entry = self.peers.get_mut(&peer)?;
        let bytes = entry.ready.pop_front()?;
        entry.staged_frames = entry.staged_frames.saturating_sub(1);
        entry.staged_bytes = entry.staged_bytes.saturating_sub(bytes.len());
        self.staged_host_frames = self.staged_host_frames.saturating_sub(1);
        self.staged_host_bytes = self.staged_host_bytes.saturating_sub(bytes.len());
        Some(bytes)
    }

    /// Deterministic pump only: moves every peer's staged datagrams through
    /// `faults` into its ready queue, in ascending [`PeerId`] order.
    ///
    /// Per peer: drop each periodic loss datagram (recovered by driver
    /// retry); duplicate each periodic dup datagram in place; then rotate
    /// the surviving batch in consecutive blocks of `reorder_depth + 1` (no
    /// datagram moves more than `reorder_depth` places; batches never cross
    /// a tick boundary). Duplicates are copies of already-admitted bytes and
    /// carry no extra accounting.
    pub fn tick(&mut self, faults: &FaultSchedule) {
        if self.fault_seed != Some(faults.seed) {
            self.fault_rng = DeterministicRng::from_seed(faults.seed);
            self.fault_seed = Some(faults.seed);
        }
        let Self {
            peers, fault_rng, ..
        } = self;
        let stream = fault_rng.stream("net-fault");
        for entry in peers.values_mut() {
            // Grows by push from already-bounded staging: no reservation
            // from hostile lengths anywhere on this path.
            let mut batch = Vec::new();
            for bytes in entry.pending.drain(..) {
                entry.datagrams = entry.datagrams.wrapping_add(1);
                if periodic(faults.loss_every, entry.datagrams) {
                    continue;
                }
                if periodic(faults.dup_every, entry.datagrams) {
                    batch.push(bytes.clone());
                }
                batch.push(bytes);
            }
            if faults.reorder_depth > 0 && batch.len() > 1 {
                window_shuffle(stream, &mut batch, faults.reorder_depth);
            }
            entry.ready.extend(batch);
        }
    }
}

/// Periodic fault predicate: the `n`th, `2n`th, … datagram is affected.
/// `None` or `0` disables. Counters start at 1 for the first datagram.
fn periodic(every: Option<u64>, counter: u64) -> bool {
    match every {
        None | Some(0) => false,
        Some(n) => counter.is_multiple_of(n),
    }
}

/// Deterministic bounded shuffle: the batch is split into consecutive
/// blocks of `depth + 1` datagrams and each block rotates left by a
/// uniformly drawn amount, so no datagram moves more than `depth` places and
/// batches never cross a tick boundary. (A forward windowed swap would let a
/// datagram surf right without bound; rotation keeps the proven bound.)
/// Draws come from the seeded stream, keeping identical send/tick sequences
/// byte-identical on the same build.
fn window_shuffle(stream: &mut crpg_core::Pcg32, batch: &mut [Vec<u8>], depth: usize) {
    use core::num::NonZeroU32;
    let block = depth.saturating_add(1).max(1);
    let mut start = 0;
    while start < batch.len() {
        let end = (start + block).min(batch.len());
        let len = end - start;
        if len > 1 {
            let bound = NonZeroU32::new(len as u32).expect("block holds at least two");
            batch[start..end].rotate_left(stream.gen_range_u32(bound) as usize);
        }
        start = end;
    }
}
