//! The run loop (T154): one cycle order, two hostings. No clock, no thread,
//! never blocks.
//!
//! `HostLoop` owns one cycle order — intake, AI stage, advance, deliver,
//! retire captures — on a fixed grid, driven by an injected `now_ms`. A
//! `Hosting` supplies the five stages: `LocalHost` (one `Host` plus in-memory
//! local links) here, the QUIC hosting later. A late step runs one cycle and
//! skips the rest, so game time slows under overload and never bursts.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError, TrySendError};

use crpg_net::movement_v4::MOVEMENT_VERSION;
use crpg_net::protocol::{ReceiptStatus, RejectionCode};
use crpg_net::protocol_transfer::TRANSFER_VERSION;
use crpg_net::protocol_v3::RejectionCode as CodeV3;
use crpg_net::transfer_v3::TRANSFER_VERSION_V3;
use crpg_net::transport::Transport;

use crate::host::{
    AdvanceReport, Host, HostError, IngestDisposition, MoveAdmission, PeerHandle,
    TransferDisposition, TransferRefusal,
};
use crate::quic::MAX_DRAIN_PER_CONNECTION;

/// The cycle period at the 20 Hz cadence (engine spec §2.4).
pub const TICK_PERIOD_MS: u64 = 50;

/// The most retirement records a `LocalHost` keeps; the oldest is dropped
/// first (B-9; crate invariant 6: every ledger is bounded).
pub const MAX_RETIRED_RECORDS: usize = 64;

/// One hosting of the one `Host`: the five stages of a cycle.
pub trait Hosting {
    /// The hosting's stage failure (the `Host` error for every hosting here).
    type Error: std::error::Error + Send + 'static;

    /// Takes every waiting frame (at most the per-peer drain cap each) and
    /// then pumps once.
    fn intake(&mut self, now_ms: u64) -> Result<(), Self::Error>;

    /// Runs the AI hook, if any.
    fn drive_ai(&mut self, now_ms: u64) -> Result<(), Self::Error>;

    /// Advances the world one step (the pause-aware driver).
    fn advance(&mut self, now_ms: u64) -> Result<AdvanceReport, Self::Error>;

    /// Offers each peer's unacknowledged delivery and acknowledges what was
    /// accepted.
    fn deliver(&mut self, now_ms: u64) -> Result<(), Self::Error>;

    /// Retires the capture journal through its last record.
    fn retire_captures(&mut self) -> Result<(), Self::Error>;
}

/// What one cycle did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleReport {
    /// The step's accepted time.
    pub now_ms: u64,
    /// The advance stage's report.
    pub advance: AdvanceReport,
}

/// What one [`HostLoop::step`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepReport {
    /// The cycle this step ran, or `None` when no slot was due.
    pub cycle: Option<CycleReport>,
    /// Due slots this step skipped, so game time slows instead of bursting.
    pub missed: u64,
}

/// A rejected step: the loop's schedule error, or a hosting stage's failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopError<E> {
    /// `now_ms` is below the last accepted one; nothing changed.
    TimeRegression {
        /// The last accepted time.
        last_ms: u64,
        /// The refused time.
        now_ms: u64,
    },
    /// A stage failed; later stages did not run.
    Hosting(E),
}

impl<E: fmt::Display> fmt::Display for LoopError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimeRegression { .. } => write!(f, "TimeRegression at run/step"),
            Self::Hosting(error) => write!(f, "Hosting at run/step: {error}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for LoopError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::TimeRegression { .. } => None,
            Self::Hosting(error) => Some(error),
        }
    }
}

/// The scheduled run loop over one hosting: one cycle per due slot, never
/// more, on a grid of `start_ms + k × period_ms`.
pub struct HostLoop<H: Hosting> {
    /// The hosting whose five stages each cycle runs.
    hosting: H,
    /// The grid period in milliseconds.
    period_ms: u64,
    /// The next slot that is due.
    next_due_ms: u64,
    /// The last accepted `now_ms`.
    last_ms: u64,
    /// Cycles run, failed ones included.
    cycles: u64,
    /// Slots skipped in total.
    missed: u64,
}

impl<H: Hosting> HostLoop<H> {
    /// A loop at the 20 Hz cadence whose first slot is `start_ms`.
    pub fn new(hosting: H, start_ms: u64) -> Self {
        Self {
            hosting,
            period_ms: TICK_PERIOD_MS,
            next_due_ms: start_ms,
            last_ms: start_ms,
            cycles: 0,
            missed: 0,
        }
    }

    /// A loop with an explicit period; its first slot is `start_ms`.
    pub fn with_period(hosting: H, start_ms: u64, period_ms: NonZeroU64) -> Self {
        Self {
            hosting,
            period_ms: period_ms.get(),
            next_due_ms: start_ms,
            last_ms: start_ms,
            cycles: 0,
            missed: 0,
        }
    }

    /// One step at `now_ms`: runs at most one cycle, at the first due slot.
    ///
    /// A time below the last accepted one changes nothing. A step past
    /// several slots runs one cycle and skips the rest.
    pub fn step(&mut self, now_ms: u64) -> Result<StepReport, LoopError<H::Error>> {
        if now_ms < self.last_ms {
            return Err(LoopError::TimeRegression {
                last_ms: self.last_ms,
                now_ms,
            });
        }
        self.last_ms = now_ms;
        if now_ms < self.next_due_ms {
            return Ok(StepReport {
                cycle: None,
                missed: 0,
            });
        }
        let skipped = (now_ms - self.next_due_ms) / self.period_ms;
        self.next_due_ms = self
            .next_due_ms
            .saturating_add(skipped.saturating_add(1).saturating_mul(self.period_ms));
        self.missed += skipped;
        self.cycles += 1;
        match self.run_cycle(now_ms) {
            Ok(cycle) => Ok(StepReport {
                cycle: Some(cycle),
                missed: skipped,
            }),
            Err(error) => Err(LoopError::Hosting(error)),
        }
    }

    /// The one cycle order: intake, AI stage, advance, deliver, retire
    /// captures — each once, stopping at the first failure.
    fn run_cycle(&mut self, now_ms: u64) -> Result<CycleReport, H::Error> {
        self.hosting.intake(now_ms)?;
        self.hosting.drive_ai(now_ms)?;
        let advance = self.hosting.advance(now_ms)?;
        self.hosting.deliver(now_ms)?;
        self.hosting.retire_captures()?;
        Ok(CycleReport { now_ms, advance })
    }

    /// The grid period in milliseconds.
    pub fn period_ms(&self) -> u64 {
        self.period_ms
    }

    /// The next slot that is due.
    pub fn next_due_ms(&self) -> u64 {
        self.next_due_ms
    }

    /// Cycles run, failed ones included.
    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Slots skipped in total.
    pub fn missed(&self) -> u64 {
        self.missed
    }

    /// The last accepted `now_ms`; `start_ms` before any step.
    pub fn last_ms(&self) -> u64 {
        self.last_ms
    }

    /// The hosting, for trusted reads.
    pub fn hosting(&self) -> &H {
        &self.hosting
    }

    /// The hosting, for trusted operator calls between cycles.
    pub fn hosting_mut(&mut self) -> &mut H {
        &mut self.hosting
    }

    /// Takes the hosting back out, with its links and schedule untouched.
    pub fn into_hosting(self) -> H {
        self.hosting
    }
}

/// The AI stage of a `LocalHost`: the embedding's hook, given the host and
/// the cycle's `now_ms`.
pub type AiHook = Box<dyn FnMut(&mut Host, u64) -> Result<(), HostError> + Send>;

/// One link's counters. `#[non_exhaustive]`: a later part adds a field, so
/// code outside `crpg-server` reads fields or compares with
/// [`LinkStats::default()`] and cannot build one by a struct literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct LinkStats {
    /// Frames taken from the up queue (a held frame once).
    pub received: u64,
    /// Frames refused and dropped.
    pub refused: u64,
    /// A lane-0 frame waits for retry (`QueueFull`).
    pub held: bool,
    /// Frames the down queue accepted.
    pub sent: u64,
    /// Lane-1 frames held for retry (T154c, B-11).
    pub held_moves: u32,
}

/// Why a link could not be attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalError {
    /// The handle is not bound on the host.
    UnknownPeer,
    /// The handle already has a link.
    AlreadyAttached,
}

impl fmt::Display for LocalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPeer => write!(f, "UnknownPeer at run/attach"),
            Self::AlreadyAttached => write!(f, "AlreadyAttached at run/attach"),
        }
    }
}

impl std::error::Error for LocalError {}

/// Why a [`LocalClient`] call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkError {
    /// The bounded queue is full; nothing was queued.
    Full,
    /// The server end of the link is gone.
    Closed,
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => write!(f, "Full at run/link"),
            Self::Closed => write!(f, "Closed at run/link"),
        }
    }
}

impl std::error::Error for LinkError {}

/// What routing one frame decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disposition {
    /// Routed; the drain goes on.
    Continue,
    /// Held for retry; this peer's intake stops.
    Hold,
    /// The link ends.
    Retire,
}

/// One attached peer's server end: a bounded up queue and a bounded down
/// queue of whole frames.
struct Link {
    /// Frames the client sends.
    up: Receiver<Vec<u8>>,
    /// Frames the client receives.
    down: SyncSender<Vec<u8>>,
    /// This link's counters.
    stats: LinkStats,
    /// The one frame a `QueueFull` refused, retried first.
    held: Option<Vec<u8>>,
    /// The client's own end is gone; the link retires at deliver.
    departed: bool,
    /// The `attach` capacity: the lane-1 hold queue stops the drain here.
    capacity: usize,
    /// Lane-1 frames held for retry, oldest first (B-11).
    moves_held: VecDeque<Vec<u8>>,
    /// Lane-1 receipts taken from the host, never dropped while live (B-12).
    receipts: VecDeque<Vec<u8>>,
    /// `take_move_receipts` said `NotV4Peer`: a v1–v3 session skips lane 1.
    no_lane1: bool,
}

/// The in-memory hosting: one `Host`, its local links and its AI hook.
pub struct LocalHost {
    /// The authoritative host every stage drives.
    host: Host,
    /// Attached peers, in ascending handle order for every pass.
    links: BTreeMap<PeerHandle, Link>,
    /// Retired links not yet taken, oldest first.
    retired: VecDeque<(PeerHandle, LinkStats)>,
    /// Retirement records dropped because the list was full, in total.
    retired_dropped: u64,
    /// The AI stage's hook, if the embedding set one.
    ai: Option<AiHook>,
}

impl LocalHost {
    /// A hosting over `host` with no links and no hook.
    pub fn new(host: Host) -> Self {
        Self {
            host,
            links: BTreeMap::new(),
            retired: VecDeque::new(),
            retired_dropped: 0,
            ai: None,
        }
    }

    /// Attaches `peer` with two bounded queues of `capacity` whole frames.
    ///
    /// `UnknownPeer` when the host has not bound the handle; `AlreadyAttached`
    /// when it already has a link.
    pub fn attach(
        &mut self,
        peer: PeerHandle,
        capacity: NonZeroUsize,
    ) -> Result<LocalClient, LocalError> {
        if self.host.epoch(peer).is_err() {
            return Err(LocalError::UnknownPeer);
        }
        if self.links.contains_key(&peer) {
            return Err(LocalError::AlreadyAttached);
        }
        let (up_send, up_recv) = sync_channel(capacity.get());
        let (down_send, down_recv) = sync_channel(capacity.get());
        self.links.insert(
            peer,
            Link {
                up: up_recv,
                down: down_send,
                stats: LinkStats::default(),
                held: None,
                departed: false,
                capacity: capacity.get(),
                moves_held: VecDeque::new(),
                receipts: VecDeque::new(),
                no_lane1: false,
            },
        );
        Ok(LocalClient {
            up: up_send,
            down: down_recv,
            closed: false,
        })
    }

    /// Sets or clears the AI stage's hook.
    pub fn set_ai(&mut self, hook: Option<AiHook>) {
        self.ai = hook;
    }

    /// The link's counters, or `None` when it has none.
    pub fn link_stats(&self, peer: PeerHandle) -> Option<LinkStats> {
        self.links.get(&peer).map(|link| link.stats)
    }

    /// The links retired since the last call, each with its final stats, in
    /// retirement order, at most [`MAX_RETIRED_RECORDS`] (the newest); the
    /// list is then empty (B-9).
    pub fn take_retired(&mut self) -> Vec<(PeerHandle, LinkStats)> {
        self.retired.drain(..).collect()
    }

    /// Records dropped because the list was full, in total; never reset.
    pub fn retired_dropped(&self) -> u64 {
        self.retired_dropped
    }

    /// The host.
    pub fn host(&self) -> &Host {
        &self.host
    }

    /// The host, for trusted operator calls between cycles.
    pub fn host_mut(&mut self) -> &mut Host {
        &mut self.host
    }

    /// Takes the host back out; the links are dropped.
    pub fn into_host(self) -> Host {
        self.host
    }

    /// Ends one link: unbind, drop its server ends, remove it, and record it.
    fn retire(&mut self, peer: PeerHandle) {
        let Some(link) = self.links.remove(&peer) else {
            return;
        };
        self.host.unbind_peer(peer);
        if self.retired.len() >= MAX_RETIRED_RECORDS {
            self.retired.pop_front();
            self.retired_dropped += 1;
        }
        self.retired.push_back((peer, link.stats));
    }

    /// Routes one frame by its first byte and records the outcome.
    fn route(
        &mut self,
        peer: PeerHandle,
        frame: &[u8],
        now_ms: u64,
    ) -> Result<Disposition, HostError> {
        match frame.first() {
            Some(&TRANSFER_VERSION) | Some(&TRANSFER_VERSION_V3) => {
                match self.host.ingest_transfer(peer, frame, now_ms)? {
                    TransferDisposition::Published
                    | TransferDisposition::Resynced
                    | TransferDisposition::Stale => Ok(Disposition::Continue),
                    TransferDisposition::Refused(TransferRefusal::Unbound) => {
                        self.count_refusal(peer);
                        Ok(Disposition::Retire)
                    }
                    TransferDisposition::Refused(_) => {
                        self.count_refusal(peer);
                        Ok(Disposition::Continue)
                    }
                }
            }
            // B-11: a lane-1 `MoveTo` frame goes to `ingest_move`. A held
            // frame is retried by the caller; `Hold` tells it where to wait.
            Some(&MOVEMENT_VERSION) => match self.host.ingest_move(peer, frame, now_ms) {
                Ok(
                    MoveAdmission::Admitted { .. }
                    | MoveAdmission::Rejected { .. }
                    | MoveAdmission::Duplicate { .. },
                ) => Ok(Disposition::Continue),
                Ok(MoveAdmission::Refused {
                    code: CodeV3::QueueFull | CodeV3::RateLimited,
                    ..
                }) => Ok(Disposition::Hold),
                Ok(MoveAdmission::Refused {
                    code: CodeV3::Unauthenticated,
                    ..
                }) => {
                    self.count_refusal(peer);
                    Ok(Disposition::Retire)
                }
                Ok(MoveAdmission::Refused { .. }) => {
                    self.count_refusal(peer);
                    Ok(Disposition::Continue)
                }
                // A v1–v3 session has no lane 1; count the frame and drop it.
                Err(HostError::NotV4Peer) => {
                    self.count_refusal(peer);
                    Ok(Disposition::Continue)
                }
                Err(error) => Err(error),
            },
            _ => match self.host.ingest(peer, frame, now_ms)? {
                IngestDisposition::Staged => Ok(Disposition::Continue),
                IngestDisposition::Refused {
                    status: ReceiptStatus::Rejected(RejectionCode::QueueFull),
                } => Ok(Disposition::Hold),
                IngestDisposition::Refused {
                    status: ReceiptStatus::Rejected(RejectionCode::Unauthenticated),
                } => {
                    self.count_refusal(peer);
                    Ok(Disposition::Retire)
                }
                IngestDisposition::Refused { .. } => {
                    self.count_refusal(peer);
                    Ok(Disposition::Continue)
                }
            },
        }
    }

    /// Counts one refused-and-dropped frame on the link, if it still has one.
    fn count_refusal(&mut self, peer: PeerHandle) {
        if let Some(link) = self.links.get_mut(&peer) {
            link.stats.refused += 1;
        }
    }

    /// Records the lane-1 hold-queue length in the link's stats, if live.
    fn record_move_hold(&mut self, peer: PeerHandle) {
        if let Some(link) = self.links.get_mut(&peer) {
            link.stats.held_moves = link.moves_held.len() as u32;
        }
    }

    /// Retries the held lane-1 frames front first until one holds (B-11).
    /// Returns true when the link retired and the caller moves on.
    fn retry_held_moves(&mut self, peer: PeerHandle, now_ms: u64) -> Result<bool, HostError> {
        loop {
            let Some(frame) = self
                .links
                .get_mut(&peer)
                .and_then(|link| link.moves_held.pop_front())
            else {
                return Ok(false);
            };
            match self.route(peer, &frame, now_ms)? {
                Disposition::Continue => {
                    self.record_move_hold(peer);
                }
                Disposition::Hold => {
                    if let Some(link) = self.links.get_mut(&peer) {
                        link.moves_held.push_front(frame);
                        link.stats.held_moves = link.moves_held.len() as u32;
                    }
                    return Ok(false);
                }
                Disposition::Retire => {
                    self.retire(peer);
                    return Ok(true);
                }
            }
        }
    }
}

impl Hosting for LocalHost {
    type Error = HostError;

    fn intake(&mut self, now_ms: u64) -> Result<(), Self::Error> {
        let peers: Vec<PeerHandle> = self.links.keys().copied().collect();
        for peer in peers {
            // 1. The one held frame, retried first.
            let held = self.links.get_mut(&peer).and_then(|link| link.held.take());
            if let Some(frame) = held {
                match self.route(peer, &frame, now_ms)? {
                    Disposition::Continue => {
                        if let Some(link) = self.links.get_mut(&peer) {
                            link.stats.held = false;
                        }
                    }
                    Disposition::Hold => {
                        if let Some(link) = self.links.get_mut(&peer) {
                            link.held = Some(frame);
                        }
                        continue;
                    }
                    Disposition::Retire => {
                        self.retire(peer);
                        continue;
                    }
                }
            }
            // 1b. The held lane-1 frames, front first until one holds (B-11).
            if self.retry_held_moves(peer, now_ms)? {
                continue;
            }
            // 2. Fresh frames, up to the per-peer drain cap.
            for _ in 0..MAX_DRAIN_PER_CONNECTION {
                // B-11: a full lane-1 hold queue stops the drain before
                // `try_recv`, so no frame behind it is taken.
                let moves_full = self
                    .links
                    .get(&peer)
                    .is_some_and(|link| link.moves_held.len() >= link.capacity);
                if moves_full {
                    break;
                }
                let Some(link) = self.links.get_mut(&peer) else {
                    break;
                };
                match link.up.try_recv() {
                    Ok(frame) => {
                        link.stats.received += 1;
                        // B-11: behind held lane-1 frames a fresh `0xA1`
                        // frame joins the back with no admission attempt.
                        let behind_hold =
                            !link.moves_held.is_empty() && frame.first() == Some(&MOVEMENT_VERSION);
                        if behind_hold {
                            link.moves_held.push_back(frame);
                            link.stats.held_moves = link.moves_held.len() as u32;
                            continue;
                        }
                        match self.route(peer, &frame, now_ms)? {
                            Disposition::Continue => {}
                            Disposition::Hold => {
                                if frame.first() == Some(&MOVEMENT_VERSION) {
                                    // B-11: lane-1 ingest stops for this
                                    // cycle, but the drain goes on.
                                    if let Some(link) = self.links.get_mut(&peer) {
                                        link.moves_held.push_back(frame);
                                        link.stats.held_moves = link.moves_held.len() as u32;
                                    }
                                } else {
                                    if let Some(link) = self.links.get_mut(&peer) {
                                        link.held = Some(frame);
                                        link.stats.held = true;
                                    }
                                    break;
                                }
                            }
                            Disposition::Retire => {
                                self.retire(peer);
                                break;
                            }
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        if let Some(link) = self.links.get_mut(&peer) {
                            link.departed = true;
                        }
                        break;
                    }
                }
            }
        }
        // 3. One pump, links or none.
        self.host.pump(now_ms)?;
        Ok(())
    }

    fn drive_ai(&mut self, now_ms: u64) -> Result<(), Self::Error> {
        match self.ai.as_mut() {
            Some(hook) => hook(&mut self.host, now_ms),
            None => Ok(()),
        }
    }

    fn advance(&mut self, now_ms: u64) -> Result<AdvanceReport, Self::Error> {
        self.host.advance(now_ms)
    }

    fn deliver(&mut self, _now_ms: u64) -> Result<(), Self::Error> {
        let peers: Vec<PeerHandle> = self.links.keys().copied().collect();
        for peer in peers {
            // 1. A departed client retires here.
            let Some(link) = self.links.get(&peer) else {
                continue;
            };
            if link.departed {
                self.retire(peer);
                continue;
            }
            // 2. This peer's unacknowledged entries.
            let entries = match self.host.take_delivery_entries(peer) {
                Ok(entries) => entries,
                Err(HostError::UnknownPeer) => {
                    self.retire(peer);
                    continue;
                }
                Err(error) => return Err(error),
            };
            // 3. Offer them in order until the queue is full or gone.
            let mut accepted: Option<u64> = None;
            for entry in entries {
                let Some(link) = self.links.get_mut(&peer) else {
                    break;
                };
                match link.down.try_send(entry.bytes) {
                    Ok(()) => {
                        link.stats.sent += 1;
                        accepted = Some(entry.ordinal);
                    }
                    Err(TrySendError::Full(_)) => break,
                    Err(TrySendError::Disconnected(_)) => {
                        link.departed = true;
                        break;
                    }
                }
            }
            // 4. Acknowledge only what the client's queue accepted.
            if let Some(through) = accepted {
                self.host.acknowledge_delivery(peer, through)?;
            }
            // 5. A send that found the queue gone ends the link.
            if self.links.get(&peer).is_some_and(|link| link.departed) {
                self.retire(peer);
                continue;
            }
            // B-12. Receipts, after (3)–(4) for a live link. When the
            // backlog is empty `take_move_receipts` fills it; then backlog
            // frames go to the down queue in order until it is full. A
            // receipt is never dropped while the link lives.
            if !self.links.get(&peer).is_some_and(|link| link.no_lane1) {
                let backlog_empty = self
                    .links
                    .get(&peer)
                    .is_some_and(|link| link.receipts.is_empty());
                if backlog_empty {
                    match self.host.take_move_receipts(peer) {
                        Ok(frames) => {
                            if let Some(link) = self.links.get_mut(&peer) {
                                link.receipts.extend(frames);
                            }
                        }
                        // A v1–v3 session has no lane 1: B-12/B-13 never run
                        // for it again.
                        Err(HostError::NotV4Peer) => {
                            if let Some(link) = self.links.get_mut(&peer) {
                                link.no_lane1 = true;
                            }
                        }
                        Err(error) => return Err(error),
                    }
                }
                loop {
                    let frame = self
                        .links
                        .get_mut(&peer)
                        .and_then(|link| link.receipts.pop_front());
                    let Some(frame) = frame else { break };
                    let send = self
                        .links
                        .get_mut(&peer)
                        .map(|link| link.down.try_send(frame));
                    match send {
                        Some(Ok(())) => {
                            if let Some(link) = self.links.get_mut(&peer) {
                                link.stats.sent += 1;
                            }
                        }
                        Some(Err(TrySendError::Full(frame))) => {
                            if let Some(link) = self.links.get_mut(&peer) {
                                link.receipts.push_front(frame);
                            }
                            break;
                        }
                        Some(Err(TrySendError::Disconnected(frame))) => {
                            if let Some(link) = self.links.get_mut(&peer) {
                                link.receipts.push_front(frame);
                                link.departed = true;
                            }
                            break;
                        }
                        None => break,
                    }
                }
                if self.links.get(&peer).is_some_and(|link| link.departed) {
                    self.retire(peer);
                    continue;
                }
            }
            // B-13. Samples, after receipts, every cycle. Each goes to the
            // down queue; when the queue is full the rest are dropped, so a
            // sample is never held. A v1–v3 link never reaches this stage.
            if !self.links.get(&peer).is_some_and(|link| link.no_lane1) {
                for frame in self.host.take_samples(peer)? {
                    let send = self
                        .links
                        .get_mut(&peer)
                        .map(|link| link.down.try_send(frame));
                    match send {
                        Some(Ok(())) => {
                            if let Some(link) = self.links.get_mut(&peer) {
                                link.stats.sent += 1;
                            }
                        }
                        Some(Err(TrySendError::Full(_))) => break,
                        Some(Err(TrySendError::Disconnected(_))) => {
                            if let Some(link) = self.links.get_mut(&peer) {
                                link.departed = true;
                            }
                            break;
                        }
                        None => break,
                    }
                }
                if self.links.get(&peer).is_some_and(|link| link.departed) {
                    self.retire(peer);
                }
            }
        }
        Ok(())
    }

    fn retire_captures(&mut self) -> Result<(), Self::Error> {
        self.host.acknowledge_captures(self.host.last_capture())
    }
}

/// The client end of a link: `Send`, not `Clone`, not `Sync`.
pub struct LocalClient {
    /// Frames it sends to the host.
    up: SyncSender<Vec<u8>>,
    /// Frames the host sent to it.
    down: Receiver<Vec<u8>>,
    /// The server end is gone; set by a failed send or a drained queue.
    closed: bool,
}

impl LocalClient {
    /// Whether the server end of this link is gone.
    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

impl Transport for LocalClient {
    type Error = LinkError;

    fn send(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        if self.closed {
            return Err(LinkError::Closed);
        }
        match self.up.try_send(bytes.to_vec()) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(LinkError::Full),
            Err(TrySendError::Disconnected(_)) => {
                self.closed = true;
                Err(LinkError::Closed)
            }
        }
    }

    fn recv(&mut self) -> Option<Vec<u8>> {
        match self.down.try_recv() {
            Ok(frame) => Some(frame),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.closed = true;
                None
            }
        }
    }
}
