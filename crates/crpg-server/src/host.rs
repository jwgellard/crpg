//! Authority, admission, delivery, and lifecycle (T022 §1–§6; ADR-0022).
//!
//! One serial [`Host`] privately owns one `HistoryWorld`. Lane-0 intent bytes
//! enter through [`Host::ingest`] (byte-level admission only: open, time,
//! binding, rate, frame cap, queue room) and are executed FIFO by
//! [`Host::pump`], which runs the §3 pipeline per staged command:
//! decode → session epoch → per-peer seq/cache → freshness → mapping /
//! ownership / disclosure → *reservation* → execution through the public
//! `HistoryWorld::perform_action` → commit. Every outcome is determined
//! before any ledger mutates, worst-case room for that outcome class is
//! checked without mutating, and only then is the outcome committed;
//! reservation failure leaves the command staged (retryable head-of-line
//! `QueueFull`).
//!
//! Each accepted command runs execute → capture → project → cache →
//! acknowledge to completion before the next one starts, so the returned
//! `ActionOutcome` and every peer's permitted post-state are captured before
//! any later mutation (see [`crate::capture`]). Receipts go to the
//! originator only; permitted events (V2) and state ops fan out to every
//! bound peer, each with its own replica ids and delivery sequence.
//!
//! Every method here is a trusted authority/adapter API. The client-facing
//! capability is a copyable [`PeerHandle`] plus the adapter's byte channel;
//! no client ever receives `&Host`, `&mut HistoryWorld`, or capture access.
//! Authentication of bindings happens above this layer (T023/T023b); this
//! host takes already-authenticated facts.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use crpg_core::{EntityId, Ulid};
use crpg_net::codec::{self, CodecError};
use crpg_net::codec_v2;
use crpg_net::projection_v2::{project_events, EventCandidate};
use crpg_net::protocol::{
    self, IntentBody, IntentFrame, NetId, ReceiptStatus, RejectionCode, LANE_COMBAT, MAX_DELTA_OPS,
    MAX_INTENT_FRAME_BYTES, MAX_VISIBLE_ENTITIES, POLICY_OBSERVED_TICK_WINDOW, POLICY_PROOF_PEERS,
    POLICY_RETRY_CACHE_BYTES_PER_PEER_LANE, POLICY_RETRY_CACHE_PER_PEER_LANE, SEQ_LAST,
};
use crpg_net::protocol_v2;
use crpg_net::sim::{QueueCaps, RateCaps, TokenBucket};
use crpg_sim::{
    history_hash, validate_action, CombatAction, CombatError, HistoryEnvelope, HistoryError,
    HistoryEvent, HistoryWorld, World, MAX_HISTORY_EVENTS, MAX_HISTORY_PAGE,
};

use crate::capture::{
    record_len, CapturedHealth, CapturedOutcome, CapturedRecord, CapturedTurn, CapturedView,
    MAX_CAPTURE_BYTES, MAX_CAPTURE_PAGE, MAX_CAPTURE_RECORDS, MAX_CAPTURE_RECORD_BYTES,
    MAX_DELIVERY_BYTES_PER_COMMAND_PEER, MAX_DELIVERY_OPS_PER_COMMAND, MAX_NEW_EVENTS_PER_COMMAND,
};
use crate::checkpoint::CheckpointError;

// ---------------------------------------------------------------------------
// Constants (§2).
// ---------------------------------------------------------------------------

/// Per-peer live entity map bound (references
/// [`MAX_VISIBLE_ENTITIES`](crpg_net::protocol::MAX_VISIBLE_ENTITIES)).
pub const MAX_MAPPED_PER_PEER: usize = MAX_VISIBLE_ENTITIES;
/// [`ControlGrant`] bound (references the same constant).
pub const MAX_CONTROLLED_PER_PEER: usize = MAX_VISIBLE_ENTITIES;
/// Bound for each [`DisclosureGrants`] vector (1024).
pub const MAX_GRANT_IDENTITIES: usize = MAX_VISIBLE_ENTITIES;
/// Bound peers per host (aliases
/// [`POLICY_PROOF_PEERS`](crpg_net::protocol::POLICY_PROOF_PEERS), 8).
pub const MAX_PEERS: usize = POLICY_PROOF_PEERS;

// ---------------------------------------------------------------------------
// Public value types (§1).
// ---------------------------------------------------------------------------

/// The explicitly selected wire version; there is no negotiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolSelection {
    /// Frozen v1: receipts plus permitted legacy state ops.
    V1,
    /// v2: receipts plus permitted legacy/event ops plus state ops.
    V2,
}

impl ProtocolSelection {
    /// The wire version byte: `1` or `2`.
    pub fn as_u8(self) -> u8 {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
        }
    }

    /// The selection for a wire version byte, or `None`.
    pub(crate) fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::V1),
            2 => Some(Self::V2),
            _ => None,
        }
    }
}

/// Trusted host construction facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostConfig {
    /// The wire version every binding of this host speaks.
    pub protocol: ProtocolSelection,
    /// Strictly increasing per host of one authority (including restores);
    /// the high half of every session epoch.
    pub incarnation: u64,
}

/// Opaque, copyable, never-reused peer binding handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PeerHandle(u64);

impl PeerHandle {
    /// The raw handle value (from 1 within one host lifetime).
    pub fn get(self) -> u64 {
        self.0
    }
}

/// Which entities a peer may act as. Stored sorted ascending and deduped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlGrant {
    /// At most [`MAX_CONTROLLED_PER_PEER`] entries (checked before dedup).
    pub actors: Vec<EntityId>,
}

/// Per-entity, per-field disclosure for one peer. Unknown fields are denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityDisclosure {
    /// The whole entity id (generation included).
    pub entity: EntityId,
    /// The entity is visible and receives a replica id.
    pub present: bool,
    /// Health state may be delivered.
    pub health: bool,
    /// Spawn notices may be delivered.
    pub spawn: bool,
    /// Despawn notices may be delivered.
    pub despawn: bool,
    /// Death notices may be delivered.
    pub died: bool,
    /// The entity may appear as an action's actor (and be named as one).
    pub action_actor: bool,
    /// The entity may appear as an action's target (and be named as one).
    pub action_target: bool,
    /// Action outcomes involving this entity may be disclosed.
    pub action_outcome: bool,
    /// Action damage involving this entity may be disclosed.
    pub action_damage: bool,
    /// Turn starts of this entity may be disclosed.
    pub turn_start: bool,
}

/// Everything one peer may learn. `Default` is empty: everything denied.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DisclosureGrants {
    /// At most [`MAX_GRANT_IDENTITIES`] unique whole entity ids.
    pub entities: Vec<EntityDisclosure>,
    /// At most [`MAX_GRANT_IDENTITIES`] unique disclosable ability ids.
    pub abilities: Vec<Ulid>,
    /// At most [`MAX_GRANT_IDENTITIES`] unique disclosable encounter ids.
    pub encounters: Vec<Ulid>,
    /// Turn-start rounds may be disclosed.
    pub turn_round: bool,
    /// Natural encounter ends may be disclosed.
    pub encounter_end: bool,
    /// The `Turn` state op may be delivered.
    pub turn_state: bool,
}

/// The delivery effect of [`Host::set_grants`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantUpdate {
    /// Purely widening (or identical): delivery continuity preserved.
    Unchanged,
    /// Narrowing: the unacknowledged log was discarded and delivery restarted
    /// at 1 with a fresh filtered state; the adapter must fence bytes it
    /// already took.
    RebindRequired,
}

/// The byte-level outcome of [`Host::ingest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestDisposition {
    /// Staged for the next [`Host::pump`].
    Staged,
    /// Ingress-level refusal: nothing staged, no sequence consumed, empty
    /// reply.
    Refused {
        /// The fail-closed wire status.
        status: ReceiptStatus,
    },
}

/// One [`Host::pump`] or [`Host::shutdown`] per-command result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionResult {
    /// The submitting binding.
    pub peer: PeerHandle,
    /// The decoded command sequence, when decoding got that far.
    pub seq: Option<u64>,
    /// The terminal or retryable status.
    pub status: ReceiptStatus,
    /// Answered from the retry cache without re-execution.
    pub cached: bool,
    /// The command stayed staged (head-of-line `QueueFull`).
    pub retained_in_ingress: bool,
}

/// What one [`Host::pump`] did.
///
/// `admitted == applied + rejected` (commands removed from the staged
/// queue); `backpressured` is 0 or 1 (head-of-line stop; the command stays
/// staged); `dropped_replies` counts failure-budget drops (terminal outcome
/// retained, empty reply).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PumpSummary {
    /// Commands removed from the staged queue.
    pub admitted: usize,
    /// Removed commands whose status is `Applied` (cached retries included).
    pub applied: usize,
    /// Removed commands whose status is `Rejected`.
    pub rejected: usize,
    /// 1 when the pump stopped at a retained head, else 0.
    pub backpressured: usize,
    /// Rejection receipts dropped by the failure-response budgets.
    pub dropped_replies: usize,
    /// One result per removed command, plus the retained head (if any).
    pub results: Vec<AdmissionResult>,
}

impl PumpSummary {
    fn empty() -> Self {
        Self {
            admitted: 0,
            applied: 0,
            rejected: 0,
            backpressured: 0,
            dropped_replies: 0,
            results: Vec::new(),
        }
    }
}

/// Every trusted host API failure. `Display` is `<VariantName> at
/// host/admission`; there is no source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    /// The host was shut down.
    Closed,
    /// `now_ms` was earlier than the last accepted `now_ms`.
    TimeRegression,
    /// The handle is not currently bound.
    UnknownPeer,
    /// [`MAX_PEERS`] bindings already exist.
    ServerBusy,
    /// Delivery room for a trusted op (bind/grant state) is exhausted.
    QueueFull,
    /// The control grant exceeds its bound.
    InvalidControl,
    /// A grant vector exceeds its bound or holds duplicates.
    InvalidGrants,
    /// `from_history` was given a non-empty sim journal.
    PendingHistory,
    /// A monotonic counter reached its reserved sentinel.
    CounterExhausted,
    /// A capture page limit outside `1..=MAX_CAPTURE_PAGE`.
    InvalidPageLimit {
        /// The rejected limit.
        limit: usize,
    },
    /// A capture read below the acknowledgement watermark.
    StaleCaptureCursor {
        /// The requested cursor.
        requested: u64,
        /// The watermark.
        acknowledged: u64,
    },
    /// A capture read/ack past the last issued capture.
    FutureCaptureCursor {
        /// The requested cursor.
        requested: u64,
        /// The last issued capture sequence.
        last: u64,
    },
    /// A delivery ack past the last issued delivery sequence.
    FutureDeliveryCursor {
        /// The requested cursor.
        requested: u64,
        /// The last issued delivery sequence.
        last: u64,
    },
    /// An internal invariant failed (unreachable by construction).
    Invariant(&'static str),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Closed => "Closed",
            Self::TimeRegression => "TimeRegression",
            Self::UnknownPeer => "UnknownPeer",
            Self::ServerBusy => "ServerBusy",
            Self::QueueFull => "QueueFull",
            Self::InvalidControl => "InvalidControl",
            Self::InvalidGrants => "InvalidGrants",
            Self::PendingHistory => "PendingHistory",
            Self::CounterExhausted => "CounterExhausted",
            Self::InvalidPageLimit { .. } => "InvalidPageLimit",
            Self::StaleCaptureCursor { .. } => "StaleCaptureCursor",
            Self::FutureCaptureCursor { .. } => "FutureCaptureCursor",
            Self::FutureDeliveryCursor { .. } => "FutureDeliveryCursor",
            Self::Invariant(_) => "Invariant",
        };
        write!(f, "{name} at host/admission")
    }
}

impl std::error::Error for HostError {}

// ---------------------------------------------------------------------------
// Private state.
// ---------------------------------------------------------------------------

/// One staged ingress command with its submitting binding.
#[derive(Debug)]
struct StagedCommand {
    peer: PeerHandle,
    bytes: Vec<u8>,
}

/// Fixed-size retry-cache metadata plus the canonical intent bytes.
#[derive(Debug)]
struct CacheEntry {
    seq: u64,
    canonical: Vec<u8>,
    status: ReceiptStatus,
    processed_tick: u64,
    /// `(delivery generation, event_seq)` of the most recently issued
    /// receipt frame, if any.
    receipt: Option<(u64, u64)>,
    /// Ledger bytes: canonical intent plus the receipt-only template.
    bytes: usize,
}

/// One unacknowledged delivery frame.
#[derive(Debug)]
struct LoggedFrame {
    event_seq: u64,
    bytes: Vec<u8>,
}

/// One binding's session state.
#[derive(Debug)]
struct Session {
    epoch: [u8; 16],
    control: Vec<EntityId>,
    grants: DisclosureGrants,
    map: BTreeMap<EntityId, NetId>,
    reverse: BTreeMap<NetId, EntityId>,
    next_net: u64,
    next_new_seq: u64,
    cache: VecDeque<CacheEntry>,
    cache_bytes: usize,
    intent_frames: TokenBucket,
    intent_bytes: TokenBucket,
    fail_frames: TokenBucket,
    fail_bytes: TokenBucket,
    ingress_frames: usize,
    ingress_bytes: usize,
    log: VecDeque<LoggedFrame>,
    log_bytes: usize,
    next_event_seq: u64,
    delivered: u64,
    generation: u64,
    resync_pending: bool,
}

impl Session {
    fn install_map(&mut self, map: BTreeMap<EntityId, NetId>, next_net: u64) {
        self.reverse = map.iter().map(|(entity, net)| (*net, *entity)).collect();
        self.map = map;
        self.next_net = next_net;
    }

    fn view(&self) -> PeerView<'_> {
        PeerView {
            grants: &self.grants,
            map: &self.map,
        }
    }

    fn last_event_seq(&self) -> u64 {
        self.next_event_seq - 1
    }
}

/// One step of the §3 pipeline for the head command.
enum Step {
    /// The command was removed from the queue with this result.
    Removed {
        result: AdmissionResult,
        dropped: bool,
    },
    /// Reservation failed: the command stays staged.
    Blocked,
}

// ---------------------------------------------------------------------------
// The host.
// ---------------------------------------------------------------------------

/// The serial authoritative host: one privately owned `HistoryWorld` plus
/// bounded admission, capture, and delivery ledgers.
///
/// Neither `Clone` nor `Copy`; every method is a trusted authority/adapter
/// API, never a client object.
#[derive(Debug)]
pub struct Host {
    config: HostConfig,
    history: HistoryWorld,
    closed: bool,
    last_now_ms: u64,
    next_handle: u64,
    sessions: BTreeMap<PeerHandle, Session>,
    ingress: VecDeque<StagedCommand>,
    ingress_bytes: usize,
    egress_bytes: usize,
    executions: u64,
    last_combat_error: Option<CombatError>,
    captures: VecDeque<CapturedRecord>,
    capture_lens: VecDeque<usize>,
    capture_bytes: usize,
    capture_acknowledged: u64,
    last_capture: u64,
    ingress_caps: QueueCaps,
    egress_caps: QueueCaps,
    rates: RateCaps,
}

impl Host {
    /// An empty authority with no encounter; gameplay needs the
    /// [`from_history`](Self::from_history) path.
    pub fn new(seed: u64, config: HostConfig, now_ms: u64) -> Result<Self, HostError> {
        Ok(Self::assemble(config, HistoryWorld::new(seed), now_ms))
    }

    /// Wraps an embedding-choreographed authority. The sim journal must be
    /// empty ([`HostError::PendingHistory`] otherwise): the embedding may not
    /// smuggle uncaptured evidence past the wrap.
    pub fn from_history(
        history: HistoryWorld,
        config: HostConfig,
        now_ms: u64,
    ) -> Result<Self, HostError> {
        if history.pending_len() != 0 {
            return Err(HostError::PendingHistory);
        }
        Ok(Self::assemble(config, history, now_ms))
    }

    fn assemble(config: HostConfig, history: HistoryWorld, now_ms: u64) -> Self {
        Self {
            config,
            history,
            closed: false,
            last_now_ms: now_ms,
            next_handle: 1,
            sessions: BTreeMap::new(),
            ingress: VecDeque::new(),
            ingress_bytes: 0,
            egress_bytes: 0,
            executions: 0,
            last_combat_error: None,
            captures: VecDeque::new(),
            capture_lens: VecDeque::new(),
            capture_bytes: 0,
            capture_acknowledged: 0,
            last_capture: 0,
            ingress_caps: QueueCaps::v1(),
            egress_caps: QueueCaps::v1_egress(),
            rates: RateCaps::v1(),
        }
    }

    /// Rebuilds a host from validated checkpoint parts (fresh sessions).
    pub(crate) fn restore(
        config: HostConfig,
        history: HistoryWorld,
        captures: Vec<CapturedRecord>,
        capture_acknowledged: u64,
        now_ms: u64,
    ) -> Self {
        let mut host = Self::assemble(config, history, now_ms);
        for record in captures {
            let len = record_len(&record);
            host.capture_bytes += len;
            host.capture_lens.push_back(len);
            host.captures.push_back(record);
        }
        host.capture_acknowledged = capture_acknowledged;
        host.last_capture = capture_acknowledged + host.captures.len() as u64;
        host
    }

    /// The selected wire version.
    pub fn protocol(&self) -> ProtocolSelection {
        self.config.protocol
    }

    /// This host's incarnation.
    pub fn incarnation(&self) -> u64 {
        self.config.incarnation
    }

    /// The bound session epoch: `incarnation` LE bytes then handle LE bytes.
    pub fn epoch(&self, peer: PeerHandle) -> Result<[u8; 16], HostError> {
        self.sessions
            .get(&peer)
            .map(|session| session.epoch)
            .ok_or(HostError::UnknownPeer)
    }

    /// The authoritative tick (`history.world().tick().get()`).
    pub fn server_tick(&self) -> u64 {
        self.history.world().tick().get()
    }

    /// Accepted commands executed by this host lifetime (accepted `EndTurn`
    /// included); cached retries never move it.
    pub fn executions(&self) -> u64 {
        self.executions
    }

    /// `history_hash` of the owned authority.
    pub fn authority_hash(&self) -> [u8; 32] {
        history_hash(&self.history)
    }

    /// The typed cause behind the last `IllegalAction`.
    pub fn last_combat_error(&self) -> Option<CombatError> {
        self.last_combat_error.clone()
    }

    /// Whether [`shutdown`](Self::shutdown) has run.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Immutable view of the owned authority, for trusted oracles and
    /// checkpoint encoding.
    pub(crate) fn history(&self) -> &HistoryWorld {
        &self.history
    }

    pub(crate) fn has_staged_ingress(&self) -> bool {
        !self.ingress.is_empty()
    }

    /// Live bindings (T023b `start`'s `HostInUse` check).
    pub(crate) fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// The last accepted `now_ms` (T023b `start`'s `TimeRegression` check).
    pub(crate) fn last_now_ms(&self) -> u64 {
        self.last_now_ms
    }

    pub(crate) fn retained_captures(&self) -> &VecDeque<CapturedRecord> {
        &self.captures
    }

    fn check_time(&self, now_ms: u64) -> Result<(), HostError> {
        if now_ms < self.last_now_ms {
            Err(HostError::TimeRegression)
        } else {
            Ok(())
        }
    }

    // -- bindings -----------------------------------------------------------

    /// Binds one already-authenticated peer with its control and disclosure
    /// facts, staging its first permitted state from delivery seq 1.
    pub fn bind_peer(
        &mut self,
        control: ControlGrant,
        grants: DisclosureGrants,
        now_ms: u64,
    ) -> Result<PeerHandle, HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        self.check_time(now_ms)?;
        let control = normalize_control(control)?;
        let grants = normalize_grants(grants)?;
        if self.sessions.len() >= MAX_PEERS {
            return Err(HostError::ServerBusy);
        }
        if self.next_handle == u64::MAX {
            return Err(HostError::CounterExhausted);
        }
        let handle = PeerHandle(self.next_handle);
        let mut epoch = [0u8; 16];
        epoch[..8].copy_from_slice(&self.config.incarnation.to_le_bytes());
        epoch[8..].copy_from_slice(&handle.0.to_le_bytes());
        let (map, next_net) = plan_map(&BTreeMap::new(), 1, &grants, self.history.world())?;
        let mut session = Session {
            epoch,
            control,
            grants,
            map: BTreeMap::new(),
            reverse: BTreeMap::new(),
            next_net: 1,
            next_new_seq: protocol::SEQ_FIRST,
            cache: VecDeque::new(),
            cache_bytes: 0,
            intent_frames: TokenBucket::new(
                u64::from(self.rates.burst_frames),
                u64::from(self.rates.frames_per_sec),
            ),
            intent_bytes: TokenBucket::new(
                u64::from(self.rates.burst_bytes),
                u64::from(self.rates.bytes_per_sec),
            ),
            fail_frames: TokenBucket::new(
                u64::from(self.rates.burst_fail),
                u64::from(self.rates.fail_per_sec),
            ),
            fail_bytes: TokenBucket::new(
                u64::from(self.rates.burst_fail_bytes),
                u64::from(self.rates.fail_bytes_per_sec),
            ),
            ingress_frames: 0,
            ingress_bytes: 0,
            log: VecDeque::new(),
            log_bytes: 0,
            next_event_seq: 1,
            delivered: 0,
            generation: 0,
            resync_pending: false,
        };
        session.install_map(map, next_net);
        let frames = self.state_frames(&session)?;
        if !self.room(&session, &frames) {
            return Err(HostError::QueueFull);
        }
        push_frames(&mut self.egress_bytes, &mut session, frames)?;
        self.sessions.insert(handle, session);
        self.next_handle += 1;
        self.last_now_ms = now_ms;
        Ok(handle)
    }

    /// Closes one binding (its handle and epoch retire forever) and discards
    /// its delivery log, cache, and map. Commands it already staged are
    /// refused as `SessionExpired` by the next pump, never executed. Unknown
    /// handles are a no-op.
    pub fn unbind_peer(&mut self, peer: PeerHandle) {
        self.fence(peer);
    }

    fn fence(&mut self, peer: PeerHandle) {
        if let Some(session) = self.sessions.remove(&peer) {
            self.egress_bytes -= session.log_bytes;
        }
    }

    /// Replaces a peer's disclosure grants.
    ///
    /// Narrowing (removing an entity, clearing any flag, removing an
    /// ability/encounter, clearing a global) discards the unacknowledged
    /// delivery log, restarts delivery at seq 1 with a fresh filtered state,
    /// and returns [`GrantUpdate::RebindRequired`]; if that state has no
    /// room it returns [`HostError::QueueFull`] with the new grants installed
    /// and the reset pending (staged by a later
    /// [`take_delivery`](Self::take_delivery)). Purely widening changes stage
    /// the newly permitted state and return [`GrantUpdate::Unchanged`]; with
    /// no room they fail with `QueueFull` and change nothing.
    pub fn set_grants(
        &mut self,
        peer: PeerHandle,
        grants: DisclosureGrants,
    ) -> Result<GrantUpdate, HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        let session = self.sessions.get(&peer).ok_or(HostError::UnknownPeer)?;
        let grants = normalize_grants(grants)?;
        let (map, next_net) = plan_map(
            &session.map,
            session.next_net,
            &grants,
            self.history.world(),
        )?;
        if is_narrowing(&session.grants, &grants) {
            let session = self.sessions.get_mut(&peer).expect("checked above");
            session.grants = grants;
            session.install_map(map, next_net);
            self.egress_bytes -= session.log_bytes;
            session.log.clear();
            session.log_bytes = 0;
            session.generation += 1;
            session.next_event_seq = 1;
            session.delivered = 0;
            session.resync_pending = true;
            self.try_resync(peer)?;
            return Ok(GrantUpdate::RebindRequired);
        }
        let ops = if session.resync_pending {
            Vec::new()
        } else {
            let old = session.view();
            let new = PeerView {
                grants: &grants,
                map: &map,
            };
            widening_ops(self.history.world(), &old, &new)
        };
        let frames = self.chunk_frames(session, &ops)?;
        if !self.room(session, &frames) {
            return Err(HostError::QueueFull);
        }
        let session = self.sessions.get_mut(&peer).expect("checked above");
        session.grants = grants;
        session.install_map(map, next_net);
        push_frames(&mut self.egress_bytes, session, frames)?;
        Ok(GrantUpdate::Unchanged)
    }

    /// Replaces a peer's control grant; affects future admission only and
    /// never resynchronizes delivery.
    pub fn set_control(
        &mut self,
        peer: PeerHandle,
        control: ControlGrant,
    ) -> Result<(), HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        if !self.sessions.contains_key(&peer) {
            return Err(HostError::UnknownPeer);
        }
        let control = normalize_control(control)?;
        self.sessions.get_mut(&peer).expect("checked above").control = control;
        Ok(())
    }

    // -- ingress and admission ---------------------------------------------

    /// Byte-level admission: host open, time monotonic, peer bound, intent
    /// rate budgets, frame cap, then per-peer and host ingress room. Never
    /// decodes and never consumes lane sequence.
    pub fn ingest(
        &mut self,
        peer: PeerHandle,
        bytes: &[u8],
        now_ms: u64,
    ) -> Result<IngestDisposition, HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        self.check_time(now_ms)?;
        self.last_now_ms = now_ms;
        let refuse = |code| {
            Ok(IngestDisposition::Refused {
                status: ReceiptStatus::Rejected(code),
            })
        };
        let Some(session) = self.sessions.get_mut(&peer) else {
            return refuse(RejectionCode::Unauthenticated);
        };
        // Both budgets deduct per attempt, retries and invalid attempts
        // included, so neither short-circuits the other.
        let frames_ok = session.intent_frames.consume(now_ms, 1);
        let bytes_ok = session.intent_bytes.consume(now_ms, bytes.len() as u64);
        if !frames_ok || !bytes_ok {
            return refuse(RejectionCode::RateLimited);
        }
        if bytes.len() > MAX_INTENT_FRAME_BYTES {
            return refuse(RejectionCode::FrameTooLarge);
        }
        let caps = self.ingress_caps;
        if session.ingress_frames + 1 > caps.per_peer_frames
            || session.ingress_bytes + bytes.len() > caps.per_peer_bytes
            || self.ingress.len() + 1 > caps.host_frames
            || self.ingress_bytes + bytes.len() > caps.host_bytes
        {
            return refuse(RejectionCode::QueueFull);
        }
        session.ingress_frames += 1;
        session.ingress_bytes += bytes.len();
        self.ingress_bytes += bytes.len();
        self.ingress.push_back(StagedCommand {
            peer,
            bytes: bytes.to_vec(),
        });
        Ok(IngestDisposition::Staged)
    }

    /// Drains staged commands FIFO through the §3 pipeline, stopping at the
    /// first head whose reservation fails (it stays staged). After shutdown
    /// returns an empty summary.
    pub fn pump(&mut self, now_ms: u64) -> Result<PumpSummary, HostError> {
        let mut summary = PumpSummary::empty();
        if self.closed {
            return Ok(summary);
        }
        self.check_time(now_ms)?;
        self.last_now_ms = now_ms;
        while let Some(staged) = self.ingress.pop_front() {
            self.release_ingress(&staged);
            match self.admit(&staged, now_ms)? {
                Step::Removed { result, dropped } => {
                    summary.admitted += 1;
                    match result.status {
                        ReceiptStatus::Applied => summary.applied += 1,
                        ReceiptStatus::Rejected(_) => summary.rejected += 1,
                    }
                    if dropped {
                        summary.dropped_replies += 1;
                    }
                    summary.results.push(result);
                }
                Step::Blocked => {
                    let seq = self.decode(&staged.bytes).ok().map(|frame| frame.seq);
                    summary.backpressured = 1;
                    summary.results.push(AdmissionResult {
                        peer: staged.peer,
                        seq,
                        status: ReceiptStatus::Rejected(RejectionCode::QueueFull),
                        cached: false,
                        retained_in_ingress: true,
                    });
                    self.restage_front(staged);
                    break;
                }
            }
        }
        Ok(summary)
    }

    fn release_ingress(&mut self, staged: &StagedCommand) {
        self.ingress_bytes -= staged.bytes.len();
        if let Some(session) = self.sessions.get_mut(&staged.peer) {
            session.ingress_frames -= 1;
            session.ingress_bytes -= staged.bytes.len();
        }
    }

    fn restage_front(&mut self, staged: StagedCommand) {
        self.ingress_bytes += staged.bytes.len();
        if let Some(session) = self.sessions.get_mut(&staged.peer) {
            session.ingress_frames += 1;
            session.ingress_bytes += staged.bytes.len();
        }
        self.ingress.push_front(staged);
    }

    fn decode(&self, bytes: &[u8]) -> Result<IntentFrame, CodecError> {
        match self.config.protocol {
            ProtocolSelection::V1 => codec::decode_intent(bytes),
            ProtocolSelection::V2 => codec_v2::decode_intent(bytes),
        }
    }

    fn canonical(&self, frame: &IntentFrame, raw: &[u8]) -> Vec<u8> {
        let encoded = match self.config.protocol {
            ProtocolSelection::V1 => codec::encode_intent(frame),
            ProtocolSelection::V2 => codec_v2::encode_intent(frame),
        };
        encoded.unwrap_or_else(|_| raw.to_vec())
    }

    /// The §3 pipeline for one removed head command.
    fn admit(&mut self, staged: &StagedCommand, now_ms: u64) -> Result<Step, HostError> {
        let peer = staged.peer;
        let removed = |seq, status, cached| Step::Removed {
            result: AdmissionResult {
                peer,
                seq,
                status,
                cached,
                retained_in_ingress: false,
            },
            dropped: false,
        };
        // 1. Decode with the selected codec; identity code map.
        let frame = match self.decode(&staged.bytes) {
            Ok(frame) => frame,
            Err(error) => {
                return Ok(removed(
                    None,
                    ReceiptStatus::Rejected(map_codec(error)),
                    false,
                ));
            }
        };
        let seq = Some(frame.seq);
        // 2. Session epoch of the staged peer.
        let Some(session) = self.sessions.get(&peer) else {
            return Ok(removed(
                seq,
                ReceiptStatus::Rejected(RejectionCode::SessionExpired),
                false,
            ));
        };
        if frame.epoch != session.epoch {
            return Ok(removed(
                seq,
                ReceiptStatus::Rejected(RejectionCode::SessionExpired),
                false,
            ));
        }
        // 3. Seq/cache, cache-first.
        let canonical = self.canonical(&frame, &staged.bytes);
        if let Some(entry) = session.cache.iter().find(|entry| entry.seq == frame.seq) {
            if entry.canonical == canonical {
                return self.cache_hit(peer, frame.seq, now_ms);
            }
            self.fence(peer);
            return Ok(removed(
                seq,
                ReceiptStatus::Rejected(RejectionCode::SeqConflict),
                false,
            ));
        }
        if session.next_new_seq > SEQ_LAST {
            return Ok(removed(
                seq,
                ReceiptStatus::Rejected(RejectionCode::SeqExhausted),
                false,
            ));
        }
        if frame.seq > session.next_new_seq {
            return Ok(removed(
                seq,
                ReceiptStatus::Rejected(RejectionCode::SeqGap),
                false,
            ));
        }
        if frame.seq < session.next_new_seq {
            return Ok(removed(
                seq,
                ReceiptStatus::Rejected(RejectionCode::StaleSeq),
                false,
            ));
        }
        // 4. Freshness against the server tick.
        let server_tick = self.server_tick();
        let mut terminal: Option<(RejectionCode, Option<CombatError>)> = None;
        if frame.observed_tick > server_tick {
            terminal = Some((RejectionCode::FutureTick, None));
        } else if frame.observed_tick < server_tick.saturating_sub(POLICY_OBSERVED_TICK_WINDOW) {
            terminal = Some((RejectionCode::StaleTick, None));
        }
        // 5. Mapping / ownership / disclosure-gated identity.
        let mut action = None;
        if terminal.is_none() {
            match resolve_action(session, &frame) {
                Some(resolved) => action = Some(resolved),
                None => terminal = Some((RejectionCode::NotAuthorized, None)),
            }
        }
        // Gameplay class through sim's single legality source (no copy).
        if let Some(resolved) = &action {
            if let Err(error) = validate_action(self.history.world(), resolved) {
                terminal = Some((RejectionCode::IllegalAction, Some(error)));
            }
        }
        // 6. Reservation, then 7–8 execution and commit.
        match (terminal, action) {
            (Some((code, error)), _) => {
                let receipt = self.receipt_frame(
                    session,
                    frame.seq,
                    server_tick,
                    ReceiptStatus::Rejected(code),
                )?;
                if !session.resync_pending && !self.room(session, std::slice::from_ref(&receipt)) {
                    return Ok(Step::Blocked);
                }
                if let Some(error) = error {
                    self.last_combat_error = Some(error);
                }
                let dropped = self.commit_rejection(
                    peer,
                    &frame,
                    canonical,
                    ReceiptStatus::Rejected(code),
                    server_tick,
                    receipt,
                    now_ms,
                )?;
                Ok(Step::Removed {
                    result: AdmissionResult {
                        peer,
                        seq,
                        status: ReceiptStatus::Rejected(code),
                        cached: false,
                        retained_in_ingress: false,
                    },
                    dropped,
                })
            }
            (None, Some(resolved)) => {
                if !self.accepted_room()? {
                    return Ok(Step::Blocked);
                }
                let before = self.history.last_sequence();
                match self.history.perform_action(&resolved) {
                    Ok(outcome) => {
                        self.commit_accepted(peer, &frame, canonical, &resolved, outcome, before)?;
                        Ok(removed(seq, ReceiptStatus::Applied, false))
                    }
                    Err(HistoryError::Combat(error)) => {
                        // Unreachable after validate_action; committed under
                        // the accepted reservation, which is a superset.
                        let session = self.sessions.get(&peer).expect("bound above");
                        let receipt = self.receipt_frame(
                            session,
                            frame.seq,
                            server_tick,
                            ReceiptStatus::Rejected(RejectionCode::IllegalAction),
                        )?;
                        self.last_combat_error = Some(error);
                        let status = ReceiptStatus::Rejected(RejectionCode::IllegalAction);
                        let dropped = self.commit_rejection(
                            peer,
                            &frame,
                            canonical,
                            status,
                            server_tick,
                            receipt,
                            now_ms,
                        )?;
                        Ok(Step::Removed {
                            result: AdmissionResult {
                                peer,
                                seq,
                                status,
                                cached: false,
                                retained_in_ingress: false,
                            },
                            dropped,
                        })
                    }
                    // Sim capacity failures commit nothing: retryable.
                    Err(_) => Ok(Step::Blocked),
                }
            }
            (None, None) => Err(HostError::Invariant("admission without action or terminal")),
        }
    }

    /// Cache hit: return the retained outcome without re-execution and make
    /// sure the receipt is deliverable.
    fn cache_hit(&mut self, peer: PeerHandle, seq: u64, now_ms: u64) -> Result<Step, HostError> {
        let server_tick = self.server_tick();
        let protocol = self.config.protocol;
        let session = self.sessions.get(&peer).expect("bound by caller");
        let entry = session
            .cache
            .iter()
            .find(|entry| entry.seq == seq)
            .expect("hit by caller");
        let status = entry.status;
        let deliverable = entry.receipt.is_some_and(|(generation, event_seq)| {
            generation == session.generation && event_seq > session.delivered
        });
        let mut dropped = false;
        if !deliverable && !session.resync_pending {
            let ops = vec![receipt_op(session.epoch, seq, entry.processed_tick, status)];
            let frame = encode_frame(protocol, server_tick, session.next_event_seq, &ops)?;
            let frame = LoggedFrame {
                event_seq: session.next_event_seq,
                bytes: frame,
            };
            // Best effort under the egress caps: a cache hit reserves nothing,
            // and the sender's next identical retry recovers the receipt.
            if self.room(session, std::slice::from_ref(&frame)) {
                let session = self.sessions.get_mut(&peer).expect("bound by caller");
                let spend = match status {
                    ReceiptStatus::Applied => true,
                    ReceiptStatus::Rejected(_) => {
                        try_consume_failure(session, now_ms, frame.bytes.len())
                    }
                };
                if spend {
                    let generation = session.generation;
                    let event_seq = frame.event_seq;
                    push_frames(&mut self.egress_bytes, session, vec![frame])?;
                    let entry = session
                        .cache
                        .iter_mut()
                        .find(|entry| entry.seq == seq)
                        .expect("hit by caller");
                    entry.receipt = Some((generation, event_seq));
                } else {
                    dropped = true;
                }
            }
        }
        Ok(Step::Removed {
            result: AdmissionResult {
                peer,
                seq: Some(seq),
                status,
                cached: true,
                retained_in_ingress: false,
            },
            dropped,
        })
    }

    /// Builds the originator's receipt-only frame at its next delivery seq.
    fn receipt_frame(
        &self,
        session: &Session,
        seq: u64,
        processed_tick: u64,
        status: ReceiptStatus,
    ) -> Result<LoggedFrame, HostError> {
        let ops = vec![receipt_op(session.epoch, seq, processed_tick, status)];
        let bytes = encode_frame(
            self.config.protocol,
            self.server_tick(),
            session.next_event_seq,
            &ops,
        )?;
        Ok(LoggedFrame {
            event_seq: session.next_event_seq,
            bytes,
        })
    }

    /// Commits a terminal rejection: sequence, cache, receipt subject to the
    /// failure budgets. Returns whether the reply was dropped.
    #[allow(clippy::too_many_arguments)]
    fn commit_rejection(
        &mut self,
        peer: PeerHandle,
        frame: &IntentFrame,
        canonical: Vec<u8>,
        status: ReceiptStatus,
        processed_tick: u64,
        receipt: LoggedFrame,
        now_ms: u64,
    ) -> Result<bool, HostError> {
        let template = self.receipt_template_len(frame.epoch, frame.seq, processed_tick, status)?;
        let session = self.sessions.get_mut(&peer).expect("bound by caller");
        session.next_new_seq = frame.seq + 1;
        let mut entry = CacheEntry {
            seq: frame.seq,
            bytes: canonical.len() + template,
            canonical,
            status,
            processed_tick,
            receipt: None,
        };
        // A pending resync stages the fresh state first; the sender's retry
        // recovers this receipt afterwards (like the accepted path).
        let dropped = if session.resync_pending {
            false
        } else if try_consume_failure(session, now_ms, receipt.bytes.len()) {
            entry.receipt = Some((session.generation, receipt.event_seq));
            push_frames(&mut self.egress_bytes, session, vec![receipt])?;
            false
        } else {
            true
        };
        cache_insert(session, entry);
        Ok(dropped)
    }

    /// Commits an accepted command: capture, cache, sequence, per-peer
    /// delivery, then the sim acknowledgement through the captured range.
    fn commit_accepted(
        &mut self,
        peer: PeerHandle,
        frame: &IntentFrame,
        canonical: Vec<u8>,
        action: &CombatAction,
        outcome: Option<crpg_sim::ActionOutcome>,
        before: u64,
    ) -> Result<(), HostError> {
        let after = self.history.last_sequence();
        let count = after - before;
        if count as usize > MAX_NEW_EVENTS_PER_COMMAND {
            return Err(HostError::Invariant("command produced too many events"));
        }
        let events: Vec<HistoryEnvelope> = if count == 0 {
            Vec::new()
        } else {
            self.history
                .read_after(before, MAX_HISTORY_PAGE)
                .map_err(|_| HostError::Invariant("captured range unreadable"))?
        };
        if events.len() as u64 != count {
            return Err(HostError::Invariant("captured range incomplete"));
        }
        let captured_outcome = capture_outcome(outcome.as_ref(), &events)?;
        let world = self.history.world();
        let processed_tick = self.server_tick();
        let protocol = self.config.protocol;
        let involved = involved_entities(world, action);

        let mut deliveries: Vec<(PeerHandle, LoggedFrame)> = Vec::new();
        let mut views: Vec<CapturedView> = Vec::new();
        let mut receipt_ref = None;
        for (handle, session) in &self.sessions {
            if session.resync_pending {
                continue;
            }
            let view = session.view();
            let mut ops = Vec::new();
            if *handle == peer {
                ops.push(receipt_op(
                    session.epoch,
                    frame.seq,
                    processed_tick,
                    ReceiptStatus::Applied,
                ));
            }
            if protocol == ProtocolSelection::V2 {
                let candidates: Vec<EventCandidate> = events
                    .iter()
                    .map(|envelope| view.candidate(&envelope.payload))
                    .collect();
                let projected = project_events(&candidates)
                    .map_err(|_| HostError::Invariant("projection of sim vocabulary failed"))?;
                ops.extend(projected);
            }
            let health: Vec<CapturedHealth> = view.health_entries(world, &involved);
            let turn = view.turn(world);
            for entry in &health {
                ops.push(health_op(entry));
            }
            if let Some(turn) = &turn {
                ops.push(turn_op(turn));
            }
            if ops.is_empty() {
                continue;
            }
            if ops.len() > MAX_DELIVERY_OPS_PER_COMMAND {
                return Err(HostError::Invariant(
                    "delivery ops exceed the per-command bound",
                ));
            }
            let bytes = encode_frame(protocol, processed_tick, session.next_event_seq, &ops)?;
            if bytes.len() > MAX_DELIVERY_BYTES_PER_COMMAND_PEER {
                return Err(HostError::Invariant(
                    "delivery frame exceeds the per-command bound",
                ));
            }
            if *handle == peer {
                receipt_ref = Some((session.generation, session.next_event_seq));
            }
            deliveries.push((
                *handle,
                LoggedFrame {
                    event_seq: session.next_event_seq,
                    bytes,
                },
            ));
            views.push(CapturedView {
                epoch: session.epoch,
                health,
                turn,
            });
        }
        views.sort_by_key(|view| view.epoch);
        let record = CapturedRecord {
            capture_seq: self.last_capture + 1,
            epoch: frame.epoch,
            lane: LANE_COMBAT,
            seq: frame.seq,
            history_start: before,
            history_end: after,
            outcome: captured_outcome,
            processed_tick,
            events,
            views,
        };
        let len = record_len(&record);
        if len > MAX_CAPTURE_RECORD_BYTES {
            return Err(HostError::Invariant("capture record exceeds its bound"));
        }
        let template = self.receipt_template_len(
            frame.epoch,
            frame.seq,
            processed_tick,
            ReceiptStatus::Applied,
        )?;

        // Infallible appends from here on (room reserved before execution).
        self.executions += 1;
        self.last_capture = record.capture_seq;
        self.capture_bytes += len;
        self.capture_lens.push_back(len);
        self.captures.push_back(record);
        for (handle, logged) in deliveries {
            let session = self.sessions.get_mut(&handle).expect("iterated above");
            push_frames(&mut self.egress_bytes, session, vec![logged])?;
        }
        let session = self.sessions.get_mut(&peer).expect("bound by caller");
        session.next_new_seq = frame.seq + 1;
        cache_insert(
            session,
            CacheEntry {
                seq: frame.seq,
                bytes: canonical.len() + template,
                canonical,
                status: ReceiptStatus::Applied,
                processed_tick,
                receipt: receipt_ref,
            },
        );
        self.history
            .acknowledge(after)
            .map_err(|_| HostError::Invariant("sim acknowledgement failed"))?;
        Ok(())
    }

    /// The retry-cache receipt-only template length (event_seq = 0).
    fn receipt_template_len(
        &self,
        epoch: [u8; 16],
        seq: u64,
        processed_tick: u64,
        status: ReceiptStatus,
    ) -> Result<usize, HostError> {
        let ops = vec![receipt_op(epoch, seq, processed_tick, status)];
        Ok(encode_frame(self.config.protocol, processed_tick, 0, &ops)?.len())
    }

    /// Check-only worst-case room for one accepted command (§6).
    fn accepted_room(&self) -> Result<bool, HostError> {
        if self.last_capture >= u64::MAX - 1 {
            return Err(HostError::CounterExhausted);
        }
        if self.captures.len() + 1 > MAX_CAPTURE_RECORDS
            || self.capture_bytes + MAX_CAPTURE_RECORD_BYTES > MAX_CAPTURE_BYTES
        {
            return Ok(false);
        }
        if self.history.pending_len() + MAX_NEW_EVENTS_PER_COMMAND > MAX_HISTORY_EVENTS {
            return Ok(false);
        }
        let caps = self.egress_caps;
        let mut host = self.egress_bytes;
        for session in self.sessions.values() {
            if session.resync_pending {
                continue;
            }
            if session.log.len() + 1 > caps.per_peer_frames
                || session.log_bytes + MAX_DELIVERY_BYTES_PER_COMMAND_PEER > caps.per_peer_bytes
            {
                return Ok(false);
            }
            host += MAX_DELIVERY_BYTES_PER_COMMAND_PEER;
        }
        Ok(host <= caps.host_bytes)
    }

    /// Check-only egress room for `frames` on one session.
    fn room(&self, session: &Session, frames: &[LoggedFrame]) -> bool {
        let caps = self.egress_caps;
        let bytes: usize = frames.iter().map(|frame| frame.bytes.len()).sum();
        session.log.len() + frames.len() <= caps.per_peer_frames
            && session.log_bytes + bytes <= caps.per_peer_bytes
            && self.egress_bytes + bytes <= caps.host_bytes
    }

    /// Chunks `ops` into frames at the session's next delivery sequences.
    fn chunk_frames(
        &self,
        session: &Session,
        ops: &[protocol_v2::DeltaOp],
    ) -> Result<Vec<LoggedFrame>, HostError> {
        let mut frames = Vec::new();
        let mut event_seq = session.next_event_seq;
        for chunk in ops.chunks(MAX_DELTA_OPS) {
            let bytes = encode_frame(self.config.protocol, self.server_tick(), event_seq, chunk)?;
            frames.push(LoggedFrame { event_seq, bytes });
            event_seq = event_seq
                .checked_add(1)
                .ok_or(HostError::CounterExhausted)?;
        }
        Ok(frames)
    }

    /// The fresh filtered state frames for one session.
    fn state_frames(&self, session: &Session) -> Result<Vec<LoggedFrame>, HostError> {
        let ops = session.view().full_state_ops(self.history.world());
        self.chunk_frames(session, &ops)
    }

    /// Stages a pending resync state if there is room.
    fn try_resync(&mut self, peer: PeerHandle) -> Result<(), HostError> {
        let session = self.sessions.get(&peer).ok_or(HostError::UnknownPeer)?;
        if !session.resync_pending {
            return Ok(());
        }
        let frames = self.state_frames(session)?;
        if !self.room(session, &frames) {
            return Err(HostError::QueueFull);
        }
        let session = self.sessions.get_mut(&peer).expect("checked above");
        push_frames(&mut self.egress_bytes, session, frames)?;
        session.resync_pending = false;
        Ok(())
    }

    // -- delivery ------------------------------------------------------------

    /// Every unacknowledged frame for `peer`, in `event_seq` order; repeated
    /// calls return the same bytes until acknowledged. Stages a pending
    /// resync first ([`HostError::QueueFull`] when it still has no room).
    pub fn take_delivery(&mut self, peer: PeerHandle) -> Result<Vec<Vec<u8>>, HostError> {
        self.try_resync(peer)?;
        let session = self.sessions.get(&peer).ok_or(HostError::UnknownPeer)?;
        Ok(session
            .log
            .iter()
            .map(|frame| frame.bytes.clone())
            .collect())
    }

    /// Retires delivery frames through `through`. At or below the watermark
    /// is an idempotent no-op; past the last issued sequence is
    /// [`HostError::FutureDeliveryCursor`].
    pub fn acknowledge_delivery(
        &mut self,
        peer: PeerHandle,
        through: u64,
    ) -> Result<(), HostError> {
        let session = self.sessions.get_mut(&peer).ok_or(HostError::UnknownPeer)?;
        let last = session.last_event_seq();
        if through > last {
            return Err(HostError::FutureDeliveryCursor {
                requested: through,
                last,
            });
        }
        if through <= session.delivered {
            return Ok(());
        }
        while session
            .log
            .front()
            .is_some_and(|frame| frame.event_seq <= through)
        {
            let frame = session.log.pop_front().expect("checked front");
            session.log_bytes -= frame.bytes.len();
            self.egress_bytes -= frame.bytes.len();
        }
        session.delivered = through;
        Ok(())
    }

    /// The retained retry-cache status for `seq`, if still cached.
    pub fn cached_status(
        &self,
        peer: PeerHandle,
        seq: u64,
    ) -> Result<Option<ReceiptStatus>, HostError> {
        let session = self.sessions.get(&peer).ok_or(HostError::UnknownPeer)?;
        Ok(session
            .cache
            .iter()
            .find(|entry| entry.seq == seq)
            .map(|entry| entry.status))
    }

    // -- captures ------------------------------------------------------------

    /// Up to `limit` cloned records with `capture_seq > after`, in order.
    /// Inert: advances nothing.
    pub fn read_captures(
        &self,
        after: u64,
        limit: usize,
    ) -> Result<Vec<CapturedRecord>, HostError> {
        if limit == 0 || limit > MAX_CAPTURE_PAGE {
            return Err(HostError::InvalidPageLimit { limit });
        }
        if after < self.capture_acknowledged {
            return Err(HostError::StaleCaptureCursor {
                requested: after,
                acknowledged: self.capture_acknowledged,
            });
        }
        if after > self.last_capture {
            return Err(HostError::FutureCaptureCursor {
                requested: after,
                last: self.last_capture,
            });
        }
        let skip = (after - self.capture_acknowledged) as usize;
        Ok(self
            .captures
            .iter()
            .skip(skip)
            .take(limit)
            .cloned()
            .collect())
    }

    /// Advances the capture watermark through `through`, retiring the
    /// prefix and freeing capture bytes. At or below the watermark is an
    /// idempotent no-op; past the last capture is
    /// [`HostError::FutureCaptureCursor`].
    pub fn acknowledge_captures(&mut self, through: u64) -> Result<(), HostError> {
        if through > self.last_capture {
            return Err(HostError::FutureCaptureCursor {
                requested: through,
                last: self.last_capture,
            });
        }
        if through <= self.capture_acknowledged {
            return Ok(());
        }
        let drop = (through - self.capture_acknowledged) as usize;
        for _ in 0..drop {
            self.captures.pop_front();
            let len = self
                .capture_lens
                .pop_front()
                .expect("lengths track records");
            self.capture_bytes -= len;
        }
        self.capture_acknowledged = through;
        Ok(())
    }

    /// The capture acknowledgement watermark.
    pub fn capture_acknowledged(&self) -> u64 {
        self.capture_acknowledged
    }

    /// The last issued capture sequence (0 before the first).
    pub fn last_capture(&self) -> u64 {
        self.last_capture
    }

    // -- trusted ops and lifecycle -------------------------------------------

    /// One trusted `HistoryWorld::tick`; journals nothing.
    pub fn tick(&mut self) -> Result<(), HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        self.history
            .tick()
            .map_err(|_| HostError::Invariant("zero-event tick failed"))
    }

    /// Encodes the checkpoint (§7); fails with `PendingIngress` unless
    /// staged ingress is empty.
    pub fn save_checkpoint(&self) -> Result<Vec<u8>, CheckpointError> {
        crate::checkpoint::encode(self)
    }

    /// Idempotent synchronous fence: refuses every staged command as
    /// `SessionExpired` (FIFO, no execution, no sequence), then closes every
    /// binding and discards its queues, caches, and maps. Authority,
    /// captures, capture reads/acks, and save stay available.
    pub fn shutdown(&mut self) -> Vec<AdmissionResult> {
        if self.closed {
            return Vec::new();
        }
        let results = self
            .ingress
            .drain(..)
            .map(|staged| AdmissionResult {
                peer: staged.peer,
                seq: None,
                status: ReceiptStatus::Rejected(RejectionCode::SessionExpired),
                cached: false,
                retained_in_ingress: false,
            })
            .collect();
        self.ingress_bytes = 0;
        self.sessions.clear();
        self.egress_bytes = 0;
        self.closed = true;
        results
    }
}

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

/// The identity codec-to-receipt map (§3 step 1).
fn map_codec(error: CodecError) -> RejectionCode {
    match error {
        CodecError::FrameTooLarge => RejectionCode::FrameTooLarge,
        CodecError::UnsupportedVersion => RejectionCode::UnsupportedVersion,
        CodecError::WrongDirection => RejectionCode::WrongDirection,
        CodecError::UnknownMessage => RejectionCode::UnknownMessage,
        CodecError::Malformed => RejectionCode::Malformed,
        CodecError::LimitExceeded => RejectionCode::LimitExceeded,
    }
}

fn receipt_op(
    epoch: [u8; 16],
    seq: u64,
    processed_tick: u64,
    status: ReceiptStatus,
) -> protocol_v2::DeltaOp {
    protocol_v2::DeltaOp::Legacy(protocol::DeltaOp::Receipt {
        epoch,
        lane: LANE_COMBAT,
        seq,
        processed_tick,
        status,
    })
}

fn health_op(entry: &CapturedHealth) -> protocol_v2::DeltaOp {
    protocol_v2::DeltaOp::Legacy(protocol::DeltaOp::Health {
        entity: entry.entity,
        health: entry.health,
        max_health: entry.max_health,
        dead: entry.dead,
    })
}

fn turn_op(turn: &CapturedTurn) -> protocol_v2::DeltaOp {
    protocol_v2::DeltaOp::Legacy(protocol::DeltaOp::Turn {
        active: turn.active,
        round: turn.round,
    })
}

/// Encodes one frame with the selected version's codec. A V1 host only ever
/// builds `Legacy` ops; anything else is an invariant failure.
fn encode_frame(
    protocol: ProtocolSelection,
    server_tick: u64,
    event_seq: u64,
    ops: &[protocol_v2::DeltaOp],
) -> Result<Vec<u8>, HostError> {
    let encoded = match protocol {
        ProtocolSelection::V1 => {
            let mut legacy = Vec::with_capacity(ops.len());
            for op in ops {
                match op {
                    protocol_v2::DeltaOp::Legacy(op) => legacy.push(op.clone()),
                    _ => return Err(HostError::Invariant("v2 event op on a v1 host")),
                }
            }
            codec::encode_delta(&protocol::DeltaFrame {
                lane: LANE_COMBAT,
                server_tick,
                event_seq,
                ops: legacy,
            })
        }
        ProtocolSelection::V2 => codec_v2::encode_delta(&protocol_v2::DeltaFrame {
            lane: LANE_COMBAT,
            server_tick,
            event_seq,
            ops: ops.to_vec(),
        }),
    };
    encoded.map_err(|_| HostError::Invariant("host frame failed to encode"))
}

fn push_frames(
    host_total: &mut usize,
    session: &mut Session,
    frames: Vec<LoggedFrame>,
) -> Result<(), HostError> {
    for frame in frames {
        debug_assert_eq!(frame.event_seq, session.next_event_seq);
        session.next_event_seq = session
            .next_event_seq
            .checked_add(1)
            .ok_or(HostError::CounterExhausted)?;
        session.log_bytes += frame.bytes.len();
        *host_total += frame.bytes.len();
        session.log.push_back(frame);
    }
    Ok(())
}

/// Spends one failure response (both budgets deduct per attempt).
fn try_consume_failure(session: &mut Session, now_ms: u64, len: usize) -> bool {
    let frames_ok = session.fail_frames.consume(now_ms, 1);
    let bytes_ok = session.fail_bytes.consume(now_ms, len as u64);
    frames_ok && bytes_ok
}

/// Inserts one entry, evicting the oldest first to stay within the
/// 256-entry / 2-MiB ledger (the cache's retirement path).
fn cache_insert(session: &mut Session, entry: CacheEntry) {
    while session.cache.len() >= POLICY_RETRY_CACHE_PER_PEER_LANE
        || session.cache_bytes + entry.bytes > POLICY_RETRY_CACHE_BYTES_PER_PEER_LANE
    {
        match session.cache.pop_front() {
            Some(old) => session.cache_bytes -= old.bytes,
            None => break,
        }
    }
    session.cache_bytes += entry.bytes;
    session.cache.push_back(entry);
}

pub(crate) fn normalize_control(control: ControlGrant) -> Result<Vec<EntityId>, HostError> {
    if control.actors.len() > MAX_CONTROLLED_PER_PEER {
        return Err(HostError::InvalidControl);
    }
    let mut actors = control.actors;
    actors.sort();
    actors.dedup();
    Ok(actors)
}

pub(crate) fn normalize_grants(grants: DisclosureGrants) -> Result<DisclosureGrants, HostError> {
    if grants.entities.len() > MAX_GRANT_IDENTITIES
        || grants.abilities.len() > MAX_GRANT_IDENTITIES
        || grants.encounters.len() > MAX_GRANT_IDENTITIES
    {
        return Err(HostError::InvalidGrants);
    }
    let mut grants = grants;
    grants.entities.sort_by_key(|entry| entry.entity);
    grants.abilities.sort();
    grants.encounters.sort();
    let duplicate_entity = grants
        .entities
        .windows(2)
        .any(|pair| pair[0].entity == pair[1].entity);
    let duplicate_ability = grants.abilities.windows(2).any(|pair| pair[0] == pair[1]);
    let duplicate_encounter = grants.encounters.windows(2).any(|pair| pair[0] == pair[1]);
    if duplicate_entity || duplicate_ability || duplicate_encounter {
        return Err(HostError::InvalidGrants);
    }
    Ok(grants)
}

fn disclosure_flags(entry: &EntityDisclosure) -> [bool; 10] {
    [
        entry.present,
        entry.health,
        entry.spawn,
        entry.despawn,
        entry.died,
        entry.action_actor,
        entry.action_target,
        entry.action_outcome,
        entry.action_damage,
        entry.turn_start,
    ]
}

fn lookup(grants: &DisclosureGrants, entity: EntityId) -> Option<&EntityDisclosure> {
    grants
        .entities
        .binary_search_by_key(&entity, |entry| entry.entity)
        .ok()
        .map(|index| &grants.entities[index])
}

/// Whether `new` removes anything `old` permitted.
pub(crate) fn is_narrowing(old: &DisclosureGrants, new: &DisclosureGrants) -> bool {
    for entry in &old.entities {
        let before = disclosure_flags(entry);
        let after = lookup(new, entry.entity).map_or([false; 10], disclosure_flags);
        if before.iter().zip(after).any(|(was, now)| *was && !now) {
            return true;
        }
    }
    old.abilities
        .iter()
        .any(|id| new.abilities.binary_search(id).is_err())
        || old
            .encounters
            .iter()
            .any(|id| new.encounters.binary_search(id).is_err())
        || (old.turn_round && !new.turn_round)
        || (old.encounter_end && !new.encounter_end)
        || (old.turn_state && !new.turn_state)
}

/// The live entity map for `grants`: kept ids stay, newly present live
/// entities mint the next never-reused ids in ascending `EntityId` order,
/// revoked ids retire.
fn plan_map(
    current: &BTreeMap<EntityId, NetId>,
    mut next_net: u64,
    grants: &DisclosureGrants,
    world: &World,
) -> Result<(BTreeMap<EntityId, NetId>, u64), HostError> {
    let mut map = BTreeMap::new();
    for entry in &grants.entities {
        if !entry.present || !world.contains(entry.entity) {
            continue;
        }
        let net = match current.get(&entry.entity) {
            Some(net) => *net,
            None => {
                if next_net == u64::MAX {
                    return Err(HostError::CounterExhausted);
                }
                let net = NetId::new(next_net).ok_or(HostError::Invariant("zero NetId"))?;
                next_net += 1;
                net
            }
        };
        map.insert(entry.entity, net);
    }
    debug_assert!(map.len() <= MAX_MAPPED_PER_PEER);
    Ok((map, next_net))
}

/// §3 step 5: resolve actor/target through the peer's live map, disclosure
/// and control; `None` is the single public `NotAuthorized`.
fn resolve_action(session: &Session, frame: &IntentFrame) -> Option<CombatAction> {
    let actor = *session.reverse.get(&frame.actor)?;
    let actor_grant = lookup(&session.grants, actor)?;
    if !(actor_grant.present && actor_grant.action_actor) {
        return None;
    }
    if session.control.binary_search(&actor).is_err() {
        return None;
    }
    match frame.body {
        IntentBody::DeclareAction { ability, target } => {
            let target = *session.reverse.get(&target)?;
            let target_grant = lookup(&session.grants, target)?;
            if !(target_grant.present && target_grant.action_target) {
                return None;
            }
            Some(CombatAction::UseAbility {
                actor,
                ability,
                target,
            })
        }
        IntentBody::EndTurn => Some(CombatAction::EndTurn { actor }),
    }
}

/// The entities whose state an accepted command may change: actor, target,
/// and the incoming active head.
fn involved_entities(world: &World, action: &CombatAction) -> BTreeSet<EntityId> {
    let mut involved = BTreeSet::new();
    match *action {
        CombatAction::UseAbility { actor, target, .. } => {
            involved.insert(actor);
            involved.insert(target);
        }
        CombatAction::EndTurn { actor } => {
            involved.insert(actor);
        }
    }
    if let Some(active) = world.combat().and_then(|combat| combat.active) {
        involved.insert(active);
    }
    involved
}

/// Captures the returned outcome's symbolic fields. The outcome text is
/// T020's own translation of the same operation (the `ActionResolved`
/// envelope it journaled), cross-checked field by field against the
/// returned `ActionOutcome`; nothing is re-inferred from later state.
fn capture_outcome(
    outcome: Option<&crpg_sim::ActionOutcome>,
    events: &[HistoryEnvelope],
) -> Result<Option<CapturedOutcome>, HostError> {
    let resolved = events.iter().find_map(|envelope| match &envelope.payload {
        HistoryEvent::ActionResolved {
            actor,
            target,
            ability,
            outcome,
            damage,
        } => Some((*actor, *target, *ability, outcome.clone(), *damage)),
        _ => None,
    });
    match (outcome, resolved) {
        (None, None) => Ok(None),
        (Some(returned), Some((actor, target, ability, text, damage)))
            if returned.actor == actor
                && returned.target == target
                && returned.ability == ability
                && returned.damage == damage =>
        {
            Ok(Some(CapturedOutcome {
                actor,
                target,
                ability,
                outcome: text,
                damage,
            }))
        }
        _ => Err(HostError::Invariant(
            "returned outcome disagrees with its journaled resolution",
        )),
    }
}

/// One peer's disclosure facts, borrowed for projection.
struct PeerView<'a> {
    grants: &'a DisclosureGrants,
    map: &'a BTreeMap<EntityId, NetId>,
}

impl PeerView<'_> {
    /// The replica id when the entity is mapped and its `flag` is granted.
    fn net_if(&self, entity: EntityId, flag: impl Fn(&EntityDisclosure) -> bool) -> Option<NetId> {
        let grant = lookup(self.grants, entity)?;
        if !(grant.present && flag(grant)) {
            return None;
        }
        self.map.get(&entity).copied()
    }

    fn grant(&self, entity: EntityId) -> Option<&EntityDisclosure> {
        lookup(self.grants, entity)
    }

    /// Maps one history fact to a per-field projection candidate. Any
    /// missing field or grant is `None`/`false`.
    fn candidate(&self, event: &HistoryEvent) -> EventCandidate {
        match event {
            HistoryEvent::Spawned { entity } => EventCandidate::Spawned {
                entity: self.net_if(*entity, |_| true),
                disclose: self.grant(*entity).is_some_and(|g| g.present && g.spawn),
            },
            HistoryEvent::Despawned { entity } => EventCandidate::Despawned {
                entity: self.net_if(*entity, |_| true),
                disclose: self.grant(*entity).is_some_and(|g| g.present && g.despawn),
            },
            HistoryEvent::Died { entity } => EventCandidate::Died {
                entity: self.net_if(*entity, |_| true),
                disclose: self.grant(*entity).is_some_and(|g| g.present && g.died),
            },
            HistoryEvent::ActionResolved {
                actor,
                target,
                ability,
                outcome,
                damage,
            } => {
                let both = |flag: fn(&EntityDisclosure) -> bool| {
                    self.grant(*actor).is_some_and(flag) && self.grant(*target).is_some_and(flag)
                };
                EventCandidate::ActionResolved {
                    actor: self.net_if(*actor, |g| g.action_actor),
                    target: self.net_if(*target, |g| g.action_target),
                    ability: self
                        .grants
                        .abilities
                        .binary_search(ability)
                        .is_ok()
                        .then_some(*ability),
                    outcome: both(|g| g.action_outcome).then(|| outcome.clone()),
                    damage: both(|g| g.action_damage).then_some(*damage),
                }
            }
            HistoryEvent::TurnStarted { actor, round } => EventCandidate::TurnStarted {
                actor: self.net_if(*actor, |g| g.turn_start),
                round: self.grants.turn_round.then_some(*round),
            },
            HistoryEvent::EncounterEnded { encounter, round } => EventCandidate::EncounterEnded {
                encounter: self
                    .grants
                    .encounters
                    .binary_search(encounter)
                    .is_ok()
                    .then_some(*encounter),
                round: self.grants.encounter_end.then_some(*round),
            },
        }
    }

    /// Permitted health of one entity, if mapped, health-granted, and a
    /// combatant.
    fn health(&self, world: &World, entity: EntityId) -> Option<CapturedHealth> {
        let net = self.net_if(entity, |g| g.health)?;
        let combatant = world.combatants().get(entity)?;
        Some(CapturedHealth {
            entity: net,
            health: combatant.health(),
            max_health: combatant.max_health(),
            dead: combatant.dead(),
        })
    }

    /// Permitted health of the involved entities, ascending `NetId`.
    fn health_entries(&self, world: &World, involved: &BTreeSet<EntityId>) -> Vec<CapturedHealth> {
        let mut entries: Vec<CapturedHealth> = involved
            .iter()
            .filter_map(|entity| self.health(world, *entity))
            .collect();
        entries.sort_by_key(|entry| entry.entity);
        entries
    }

    /// Permitted turn state: `turn_state` granted and an encounter exists;
    /// an undisclosed active head reads as `None`.
    fn turn(&self, world: &World) -> Option<CapturedTurn> {
        if !self.grants.turn_state {
            return None;
        }
        let combat = world.combat()?;
        Some(CapturedTurn {
            active: combat
                .active
                .and_then(|active| self.map.get(&active).copied()),
            round: u64::from(combat.round),
        })
    }

    /// The complete fresh filtered state: `EntityEnter` per mapped entity,
    /// then permitted `Health`, then permitted `Turn`, ascending `NetId`.
    fn full_state_ops(&self, world: &World) -> Vec<protocol_v2::DeltaOp> {
        let mut mapped: Vec<(NetId, EntityId)> = self
            .map
            .iter()
            .map(|(entity, net)| (*net, *entity))
            .collect();
        mapped.sort();
        let mut ops = Vec::new();
        for (net, _) in &mapped {
            ops.push(protocol_v2::DeltaOp::Legacy(
                protocol::DeltaOp::EntityEnter { entity: *net },
            ));
        }
        for (_, entity) in &mapped {
            if let Some(entry) = self.health(world, *entity) {
                ops.push(health_op(&entry));
            }
        }
        if let Some(turn) = self.turn(world) {
            ops.push(turn_op(&turn));
        }
        ops
    }
}

/// The ops a purely widening grant change adds: `EntityEnter` for newly
/// mapped entities, `Health` where health became visible, `Turn` when
/// `turn_state` became granted.
fn widening_ops(
    world: &World,
    old: &PeerView<'_>,
    new: &PeerView<'_>,
) -> Vec<protocol_v2::DeltaOp> {
    let mut entered: Vec<NetId> = new
        .map
        .iter()
        .filter(|(entity, _)| !old.map.contains_key(entity))
        .map(|(_, net)| *net)
        .collect();
    entered.sort();
    let mut ops: Vec<protocol_v2::DeltaOp> = entered
        .iter()
        .map(|net| protocol_v2::DeltaOp::Legacy(protocol::DeltaOp::EntityEnter { entity: *net }))
        .collect();
    let mut health: Vec<CapturedHealth> = new
        .map
        .keys()
        .filter(|entity| old.health(world, **entity).is_none())
        .filter_map(|entity| new.health(world, *entity))
        .collect();
    health.sort_by_key(|entry| entry.entity);
    ops.extend(health.iter().map(health_op));
    if old.turn(world).is_none() {
        if let Some(turn) = new.turn(world) {
            ops.push(turn_op(&turn));
        }
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_reference_the_net_policy() {
        assert_eq!(MAX_MAPPED_PER_PEER, 1024);
        assert_eq!(MAX_CONTROLLED_PER_PEER, 1024);
        assert_eq!(MAX_GRANT_IDENTITIES, 1024);
        assert_eq!(MAX_PEERS, 8);
    }

    #[test]
    fn protocol_selection_bytes() {
        assert_eq!(ProtocolSelection::V1.as_u8(), 1);
        assert_eq!(ProtocolSelection::V2.as_u8(), 2);
        assert_eq!(ProtocolSelection::from_u8(1), Some(ProtocolSelection::V1));
        assert_eq!(ProtocolSelection::from_u8(2), Some(ProtocolSelection::V2));
        assert_eq!(ProtocolSelection::from_u8(0), None);
        assert_eq!(ProtocolSelection::from_u8(3), None);
    }

    #[test]
    fn errors_display_variant_and_location_only() {
        assert_eq!(HostError::Closed.to_string(), "Closed at host/admission");
        assert_eq!(
            HostError::InvalidPageLimit { limit: 9 }.to_string(),
            "InvalidPageLimit at host/admission"
        );
        assert_eq!(
            HostError::Invariant("x").to_string(),
            "Invariant at host/admission"
        );
        assert!(std::error::Error::source(&HostError::QueueFull).is_none());
    }

    #[test]
    fn codec_map_is_identity() {
        let pairs = [
            (CodecError::FrameTooLarge, RejectionCode::FrameTooLarge),
            (
                CodecError::UnsupportedVersion,
                RejectionCode::UnsupportedVersion,
            ),
            (CodecError::WrongDirection, RejectionCode::WrongDirection),
            (CodecError::UnknownMessage, RejectionCode::UnknownMessage),
            (CodecError::Malformed, RejectionCode::Malformed),
            (CodecError::LimitExceeded, RejectionCode::LimitExceeded),
        ];
        for (error, code) in pairs {
            assert_eq!(map_codec(error), code);
        }
    }
}
