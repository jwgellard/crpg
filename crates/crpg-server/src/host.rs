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
use crpg_net::codec_transfer;
use crpg_net::codec_v2;
use crpg_net::projection_v2::{project_events, EventCandidate};
use crpg_net::protocol::{
    self, IntentBody, IntentFrame, NetId, ReceiptStatus, RejectionCode, LANE_COMBAT, MAX_DELTA_OPS,
    MAX_INTENT_FRAME_BYTES, MAX_VISIBLE_ENTITIES, POLICY_OBSERVED_TICK_WINDOW, POLICY_PROOF_PEERS,
    POLICY_RETRY_CACHE_BYTES_PER_PEER_LANE, POLICY_RETRY_CACHE_PER_PEER_LANE, SEQ_LAST,
};
use crpg_net::protocol_transfer::{
    CancelReason, HealthFact, SnapshotBody, SnapshotEntity, TransferDown, TransferId, TransferUp,
    TurnFact,
};
use crpg_net::protocol_v2;
use crpg_net::protocol_v3::{AutoPauseToggles, AutoPauseTrigger, PauseReason};
use crpg_net::sim::{QueueCaps, RateCaps, TokenBucket};
use crpg_sim::{
    history_hash, legal_actions, validate_action, CombatAction, CombatError, EncounterMode,
    HistoryEnvelope, HistoryError, HistoryEvent, HistoryWorld, LegalActionsError, World,
    MAX_HISTORY_EVENTS, MAX_HISTORY_PAGE,
};

use crate::capture::{
    host_record_len, record_len, CaptureEntry, CapturedHealth, CapturedOutcome, CapturedRecord,
    CapturedTurn, CapturedView, HostOp, HostRecord, MAX_CAPTURE_BYTES, MAX_CAPTURE_PAGE,
    MAX_CAPTURE_RECORDS, MAX_CAPTURE_RECORD_BYTES, MAX_DELIVERY_BYTES_PER_COMMAND_PEER,
    MAX_DELIVERY_OPS_PER_COMMAND, MAX_HOST_RECORD_BYTES, MAX_HOST_RECORD_EVENTS,
    MAX_NEW_EVENTS_PER_COMMAND_RT,
};
use crate::checkpoint::{check_host_record, CheckpointError};

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

/// Segments packed into each snapshot-transfer chunk (S§7). Every valid v1
/// body fits in one chunk, so a transfer is one `Begin` plus one `Chunk`.
const SNAPSHOT_SEGMENTS_PER_CHUNK: u8 = 8;

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

/// What a delivery entry carries, so an adapter picks the channel without
/// reading bytes (T024 S§4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryKind {
    /// A lane-0 delta frame (the host's `ProtocolSelection`).
    Delta,
    /// A snapshot-transfer frame (`crpg_net::codec_transfer`, first byte 0x81).
    Transfer,
}

/// One unacknowledged delivery entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryEntry {
    /// Position in this session's delivery order, from 1, never reused
    /// within the session (S§6).
    pub ordinal: u64,
    /// Delta or transfer frame.
    pub kind: DeliveryKind,
    /// The encoded frame.
    pub bytes: Vec<u8>,
}

/// One action `peer` may request now, in that peer's replica ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegalIntent {
    /// The acting entity, in `peer`'s replica ids.
    pub actor: NetId,
    /// The wire verb and its ids, ready for `IntentFrame.body`.
    pub body: IntentBody,
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
    /// Narrowing of a snapshot session: unacknowledged entries were
    /// discarded and a snapshot transfer staged; delivery continuity kept.
    SnapshotStaged,
}

/// The outcome of [`Host::ingest_transfer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferDisposition {
    /// `Published` for the in-flight transfer: it is retired.
    Published,
    /// `Resync`: a fresh snapshot transfer was staged.
    Resynced,
    /// `Published` for an issued transfer that is no longer in flight.
    Stale,
    /// Refused; nothing changed (rate tokens aside, S§9.2).
    Refused(TransferRefusal),
}

/// Why [`Host::ingest_transfer`] refused a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferRefusal {
    /// The handle is not bound.
    Unbound,
    /// The session was bound with `bind_peer` (no transfer channel).
    NotSnapshotPeer,
    /// The session's intent rate budget is spent.
    RateLimited,
    /// `crpg_net::codec_transfer::decode_up` failed.
    Malformed,
    /// The frame's epoch is not the session's.
    WrongEpoch,
    /// The frame names a transfer id this session never issued.
    UnknownTransfer,
}

/// A snapshot session's transfer ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferState {
    /// The id the next staged transfer will take (1 at bind).
    pub next_transfer: u64,
    /// The issued transfer not yet answered by `Published` or cancelled.
    pub in_flight: Option<u64>,
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

/// The single-player auto-pause toggles (ADR-0029 P6): `EncounterStarted`,
/// `OrderEnded` and `PartyMemberDied` (bits 0, 3 and 4).
pub const SINGLE_PLAYER_AUTO_PAUSE_BITS: u8 = 0x19;

/// Operator-owned session settings (ADR-0029 T11, P6, P8). Never in `World`,
/// the journal or the checkpoint: a restored host starts with
/// [`HostSettings::multiplayer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostSettings {
    /// The mode T040's `start_encounter` will publish (T11); stored only.
    pub initial_mode: EncounterMode,
    /// Which auto-pause triggers are on (P6).
    pub auto_pause: AutoPauseToggles,
    /// Turn-timer length in milliseconds; `None` is off (P8). `Some(0)` is
    /// refused by [`Host::set_settings`].
    pub turn_timer_ms: Option<u32>,
}

impl HostSettings {
    /// `{ TurnBased, SINGLE_PLAYER_AUTO_PAUSE_BITS, None }`.
    pub fn single_player() -> Self {
        Self {
            initial_mode: EncounterMode::TurnBased,
            auto_pause: AutoPauseToggles::from_bits(SINGLE_PLAYER_AUTO_PAUSE_BITS)
                .expect("the single-player bits are defined toggles"),
            turn_timer_ms: None,
        }
    }

    /// `{ TurnBased, bits 0, None }`.
    pub fn multiplayer() -> Self {
        Self {
            initial_mode: EncounterMode::TurnBased,
            auto_pause: AutoPauseToggles::default(),
            turn_timer_ms: None,
        }
    }
}

impl Default for HostSettings {
    fn default() -> Self {
        Self::multiplayer()
    }
}

/// What one [`Host::advance`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdvanceReport {
    /// A world tick ran.
    pub ticked: bool,
    /// The head whose turn the timer ended this call, if any.
    pub timer_ended_turn: Option<EntityId>,
    /// The pause state after the call.
    pub paused: Option<PauseReason>,
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
    /// The capture journal has no room for a host record; the trusted
    /// consumer must `acknowledge_captures` first.
    CaptureFull,
    /// The sim refused a host operation; nothing changed.
    Combat(CombatError),
    /// `HostSettings.turn_timer_ms` is `Some(0)`.
    InvalidSettings,
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
            Self::CaptureFull => "CaptureFull",
            Self::Combat(_) => "Combat",
            Self::InvalidSettings => "InvalidSettings",
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
    /// `(delivery generation, ordinal)` of the most recently issued
    /// receipt frame, if any.
    receipt: Option<(u64, u64)>,
    /// Ledger bytes: canonical intent plus the receipt-only template.
    bytes: usize,
}

/// One unacknowledged delivery frame. `event_seq` is `Some` for a lane-0
/// delta and `None` for a snapshot-transfer frame; `ordinal` is assigned by
/// [`push_frames`] when the entry enters the log.
#[derive(Debug)]
struct LoggedFrame {
    ordinal: u64,
    event_seq: Option<u64>,
    bytes: Vec<u8>,
}

/// Why a snapshot is being staged: the cancel reason it emits for an
/// in-flight transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotCause {
    /// Narrowing grants: `Cancel(Aborted)`.
    Narrowed,
    /// Upstream `Resync`: `Cancel(Superseded)`.
    Resync,
}

impl SnapshotCause {
    fn cancel_reason(self) -> CancelReason {
        match self {
            Self::Narrowed => CancelReason::Aborted,
            Self::Resync => CancelReason::Superseded,
        }
    }
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
    /// Whether the session carries the transfer subprotocol (`bind_snapshot_peer`).
    snapshot: bool,
    /// The next delivery ordinal (starts at 1; advances per pushed entry).
    next_ordinal: u64,
    /// The id the next staged transfer will take (1 at bind).
    next_transfer: u64,
    /// The issued transfer not yet `Published` or cancelled.
    in_flight: Option<u64>,
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

/// How an armed turn timer counts (B-9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimerClock {
    /// Running: expires when `now_ms >= deadline`.
    Running { deadline: u64 },
    /// Frozen by a pause: this many milliseconds remain.
    Frozen { remaining: u64 },
}

/// The armed turn timer: the head and round it was armed for (B-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TurnTimer {
    head: EntityId,
    round: u32,
    clock: TimerClock,
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
    captures: VecDeque<CaptureEntry>,
    capture_lens: VecDeque<usize>,
    capture_bytes: usize,
    capture_acknowledged: u64,
    last_capture: u64,
    ingress_caps: QueueCaps,
    egress_caps: QueueCaps,
    rates: RateCaps,
    settings: HostSettings,
    paused: Option<PauseReason>,
    timer: Option<TurnTimer>,
    timer_epoch: u64,
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
            settings: HostSettings::multiplayer(),
            paused: None,
            timer: None,
            timer_epoch: 0,
        }
    }

    /// Rebuilds a host from validated checkpoint parts (fresh sessions).
    pub(crate) fn restore(
        config: HostConfig,
        history: HistoryWorld,
        captures: Vec<CaptureEntry>,
        capture_acknowledged: u64,
        now_ms: u64,
    ) -> Self {
        let mut host = Self::assemble(config, history, now_ms);
        // A restored host starts paused (P5): the operator unpauses it.
        host.paused = Some(PauseReason::Restore);
        for entry in captures {
            let len = entry.json_len();
            host.capture_bytes += len;
            host.capture_lens.push_back(len);
            host.captures.push_back(entry);
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

    /// Every action `peer` may currently request, in sim enumeration order
    /// and in `peer`'s replica ids (T047a, ADR-0033 §7).
    ///
    /// Empty without an active encounter, and when the active head is not
    /// this session's to control or disclose. Options are advisory and confer
    /// no authorization: execution revalidates epoch, sequence and state.
    pub fn legal_intents(&self, peer: PeerHandle) -> Result<Vec<LegalIntent>, HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        let session = self.sessions.get(&peer).ok_or(HostError::UnknownPeer)?;
        let Some(active) = self
            .history
            .world()
            .combat()
            .and_then(|combat| combat.active)
        else {
            return Ok(Vec::new());
        };
        if session.control.binary_search(&active).is_err() {
            return Ok(Vec::new());
        }
        let Some(actor_net) = session.map.get(&active).copied() else {
            return Ok(Vec::new());
        };
        let eligible = lookup(&session.grants, active)
            .is_some_and(|grant| grant.present && grant.action_actor);
        if !eligible {
            return Ok(Vec::new());
        }
        match legal_actions(self.history.world(), active) {
            Ok(list) => Ok(list
                .iter()
                .filter_map(|action| {
                    legal_body(session, action).map(|body| LegalIntent {
                        actor: actor_net,
                        body,
                    })
                })
                .collect()),
            Err(LegalActionsError::Actor(_)) => Ok(Vec::new()),
            Err(LegalActionsError::TooManyOptions { .. }) => {
                Err(HostError::Invariant("legal options exceed their bound"))
            }
        }
    }

    /// The replica id this session's last map installation minted for
    /// `entity`, or `None` when it was not granted, present and live then
    /// (T047a, ADR-0033 §7). The map is a snapshot, not a liveness query.
    pub fn replica_id(
        &self,
        peer: PeerHandle,
        entity: EntityId,
    ) -> Result<Option<NetId>, HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        let session = self.sessions.get(&peer).ok_or(HostError::UnknownPeer)?;
        Ok(session.map.get(&entity).copied())
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

    pub(crate) fn retained_captures(&self) -> &VecDeque<CaptureEntry> {
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
        self.bind(control, grants, now_ms, false)
    }

    /// As [`bind_peer`](Self::bind_peer) (same precedence, errors and first
    /// delivery), but the session is a snapshot session (S§6.1): its delivery
    /// channel also carries the `0x81` transfer subprotocol in one total order
    /// with lane-0 frames.
    pub fn bind_snapshot_peer(
        &mut self,
        control: ControlGrant,
        grants: DisclosureGrants,
        now_ms: u64,
    ) -> Result<PeerHandle, HostError> {
        self.bind(control, grants, now_ms, true)
    }

    fn bind(
        &mut self,
        control: ControlGrant,
        grants: DisclosureGrants,
        now_ms: u64,
        snapshot: bool,
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
            snapshot,
            next_ordinal: 1,
            next_transfer: 1,
            in_flight: None,
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
    ///
    /// Narrowing a *snapshot* session instead follows S§8: the unacknowledged
    /// entries are discarded and a snapshot transfer is staged under the new
    /// grants and map, returning [`GrantUpdate::SnapshotStaged`] with no reset
    /// of `event_seq`, ordinals or the transfer ledger (an S§7 failure changes
    /// nothing, grants included).
    pub fn set_grants(
        &mut self,
        peer: PeerHandle,
        grants: DisclosureGrants,
    ) -> Result<GrantUpdate, HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        let (grants, map, next_net, narrowing, snapshot) = {
            let session = self.sessions.get(&peer).ok_or(HostError::UnknownPeer)?;
            let grants = normalize_grants(grants)?;
            let (map, next_net) = plan_map(
                &session.map,
                session.next_net,
                &grants,
                self.history.world(),
            )?;
            let narrowing = is_narrowing(&session.grants, &grants);
            let snapshot = session.snapshot;
            (grants, map, next_net, narrowing, snapshot)
        };
        if narrowing {
            if snapshot {
                self.stage_snapshot(peer, grants, map, next_net, SnapshotCause::Narrowed)?;
                return Ok(GrantUpdate::SnapshotStaged);
            }
            let session = self.sessions.get_mut(&peer).expect("checked above");
            session.grants = grants;
            session.install_map(map, next_net);
            self.egress_bytes -= session.log_bytes;
            session.log.clear();
            session.log_bytes = 0;
            session.generation += 1;
            session.next_event_seq = 1;
            session.next_ordinal = 1;
            session.delivered = 0;
            session.resync_pending = true;
            self.try_resync(peer)?;
            return Ok(GrantUpdate::RebindRequired);
        }
        let session = self.sessions.get(&peer).expect("checked above");
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
        let deliverable = entry.receipt.is_some_and(|(generation, ordinal)| {
            generation == session.generation && ordinal > session.delivered
        });
        let mut dropped = false;
        if !deliverable && !session.resync_pending {
            let ops = vec![receipt_op(session.epoch, seq, entry.processed_tick, status)];
            let frame = encode_frame(protocol, server_tick, session.next_event_seq, &ops)?;
            let frame = LoggedFrame {
                ordinal: 0,
                event_seq: Some(session.next_event_seq),
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
                    let ordinal = session.next_ordinal;
                    push_frames(&mut self.egress_bytes, session, vec![frame])?;
                    let entry = session
                        .cache
                        .iter_mut()
                        .find(|entry| entry.seq == seq)
                        .expect("hit by caller");
                    entry.receipt = Some((generation, ordinal));
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
            ordinal: 0,
            event_seq: Some(session.next_event_seq),
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
            entry.receipt = Some((session.generation, session.next_ordinal));
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
    ///
    /// After the sim has committed nothing undoes it (T141 Reading 5): the
    /// acknowledgement runs whether or not the capture steps succeeded, so a
    /// failed record leaves no unacknowledged evidence behind; the capture
    /// failure is then returned and the next checkpoint fails to load.
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
        // Step 1 of the tail (S-3, A-1): auto-pause, then the turn timer.
        self.after_commit(before, after);
        let result = self.capture_accepted(peer, frame, canonical, action, outcome, before, after);
        let acknowledged = self
            .history
            .acknowledge(after)
            .map_err(|_| HostError::Invariant("sim acknowledgement failed"));
        result.and(acknowledged)
    }

    /// The fallible capture steps of [`commit_accepted`](Self::commit_accepted).
    #[allow(clippy::too_many_arguments)]
    fn capture_accepted(
        &mut self,
        peer: PeerHandle,
        frame: &IntentFrame,
        canonical: Vec<u8>,
        action: &CombatAction,
        outcome: Option<crpg_sim::ActionOutcome>,
        before: u64,
        after: u64,
    ) -> Result<(), HostError> {
        let count = after - before;
        if count as usize > MAX_NEW_EVENTS_PER_COMMAND_RT {
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
                    .filter_map(|envelope| view.candidate(&envelope.payload))
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
                receipt_ref = Some((session.generation, session.next_ordinal));
            }
            deliveries.push((
                *handle,
                LoggedFrame {
                    ordinal: 0,
                    event_seq: Some(session.next_event_seq),
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
        self.captures.push_back(CaptureEntry::Command(record));
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
        if self.history.pending_len() + MAX_NEW_EVENTS_PER_COMMAND_RT > MAX_HISTORY_EVENTS {
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
            frames.push(LoggedFrame {
                ordinal: 0,
                event_seq: Some(event_seq),
                bytes,
            });
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

    /// S§7: stages one snapshot transfer for `peer`, atomically. Steps 1–5
    /// only read; step 6 commits. `cause` picks the `Cancel` reason for an
    /// in-flight transfer (`Aborted` on narrowing, `Superseded` on `Resync`).
    fn stage_snapshot(
        &mut self,
        peer: PeerHandle,
        grants: DisclosureGrants,
        map: BTreeMap<EntityId, NetId>,
        next_net: u64,
        cause: SnapshotCause,
    ) -> Result<(), HostError> {
        // Steps 1–5: determine, reading only.
        let (transfer_id, cancel, frames) = {
            let session = self.sessions.get(&peer).ok_or(HostError::UnknownPeer)?;
            // 1. Transfer-id counter sentinel.
            if session.next_transfer == u64::MAX {
                return Err(HostError::CounterExhausted);
            }
            let transfer_id = session.next_transfer;
            let epoch = session.epoch;
            let cursor = session.last_event_seq();
            // 2. Cancel the in-flight transfer first, if any.
            let cancel: Option<Vec<u8>> =
                match session.in_flight {
                    Some(old) => {
                        let message = TransferDown::Cancel {
                            epoch,
                            transfer: TransferId::new(old).expect("in-flight id is nonzero"),
                            reason: cause.cancel_reason(),
                        };
                        Some(codec_transfer::encode_down(&message).map_err(|_| {
                            HostError::Invariant("snapshot transfer failed to encode")
                        })?)
                    }
                    None => None,
                };
            // 3. Body from the (current or new) grants and map.
            let view = PeerView {
                grants: &grants,
                map: &map,
            };
            let body = view.snapshot_body(self.history.world());
            // 4. Canonical `plan_transfer` bytes.
            let frames = codec_transfer::plan_transfer(
                epoch,
                TransferId::new(transfer_id).expect("nonzero by the step-1 check"),
                self.server_tick(),
                cursor,
                SNAPSHOT_SEGMENTS_PER_CHUNK,
                &body,
            )
            .map_err(|_| HostError::Invariant("snapshot transfer failed to encode"))?;
            // 5. Room against the cleared log; the ordinal counter must stay
            //    below `u64::MAX`.
            let count = frames.len() + usize::from(cancel.is_some());
            let bytes: usize =
                frames.iter().map(Vec::len).sum::<usize>() + cancel.as_ref().map_or(0, Vec::len);
            let caps = self.egress_caps;
            let ordinal_room = (u64::MAX - 1).saturating_sub(session.next_ordinal);
            if count as u64 > ordinal_room {
                return Err(HostError::CounterExhausted);
            }
            if count > caps.per_peer_frames
                || bytes > caps.per_peer_bytes
                || self.egress_bytes - session.log_bytes + bytes > caps.host_bytes
            {
                return Err(HostError::QueueFull);
            }
            (transfer_id, cancel, frames)
        };

        // Step 6: commit (cannot fail).
        let session = self.sessions.get_mut(&peer).expect("checked above");
        if matches!(cause, SnapshotCause::Narrowed) {
            session.grants = grants;
            session.install_map(map, next_net);
        }
        self.egress_bytes -= session.log_bytes;
        session.log.clear();
        session.log_bytes = 0;
        session.delivered = session.next_ordinal - 1;
        let mut entries = Vec::new();
        if let Some(cancel) = cancel {
            entries.push(LoggedFrame {
                ordinal: 0,
                event_seq: None,
                bytes: cancel,
            });
        }
        for bytes in frames {
            entries.push(LoggedFrame {
                ordinal: 0,
                event_seq: None,
                bytes,
            });
        }
        push_frames(&mut self.egress_bytes, session, entries)?;
        session.in_flight = Some(transfer_id);
        session.next_transfer += 1;
        Ok(())
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

    /// Every unacknowledged frame for `peer`, in delivery (ordinal) order;
    /// repeated calls return the same bytes until acknowledged. A snapshot
    /// session's log carries transfer frames in the same total order as its
    /// deltas. Stages a pending resync first ([`HostError::QueueFull`] when
    /// it still has no room). Callers that must route the two kinds apart use
    /// [`take_delivery_entries`](Self::take_delivery_entries).
    pub fn take_delivery(&mut self, peer: PeerHandle) -> Result<Vec<Vec<u8>>, HostError> {
        self.try_resync(peer)?;
        let session = self.sessions.get(&peer).ok_or(HostError::UnknownPeer)?;
        Ok(session
            .log
            .iter()
            .map(|frame| frame.bytes.clone())
            .collect())
    }

    /// As [`take_delivery`](Self::take_delivery), with each entry's ordinal
    /// and kind so an adapter picks a channel without reading frame bytes
    /// (T024 S§4).
    pub fn take_delivery_entries(
        &mut self,
        peer: PeerHandle,
    ) -> Result<Vec<DeliveryEntry>, HostError> {
        self.try_resync(peer)?;
        let session = self.sessions.get(&peer).ok_or(HostError::UnknownPeer)?;
        Ok(session
            .log
            .iter()
            .map(|frame| DeliveryEntry {
                ordinal: frame.ordinal,
                kind: if frame.event_seq.is_some() {
                    DeliveryKind::Delta
                } else {
                    DeliveryKind::Transfer
                },
                bytes: frame.bytes.clone(),
            })
            .collect())
    }

    /// Retires delivery entries through the delivery **ordinal** `through`.
    /// At or below the watermark is an idempotent no-op; past the last issued
    /// ordinal is [`HostError::FutureDeliveryCursor`]. In a `bind_peer`
    /// session every ordinal equals its entry's `event_seq`, so nothing
    /// changes there.
    pub fn acknowledge_delivery(
        &mut self,
        peer: PeerHandle,
        through: u64,
    ) -> Result<(), HostError> {
        let session = self.sessions.get_mut(&peer).ok_or(HostError::UnknownPeer)?;
        let last = session.next_ordinal - 1;
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
            .is_some_and(|frame| frame.ordinal <= through)
        {
            let frame = session.log.pop_front().expect("checked front");
            session.log_bytes -= frame.bytes.len();
            self.egress_bytes -= frame.bytes.len();
        }
        session.delivered = through;
        Ok(())
    }

    /// The snapshot session's transfer ledger, or `Ok(None)` for a
    /// `bind_peer` session; `Err(UnknownPeer)` if unbound.
    pub fn transfer_state(&self, peer: PeerHandle) -> Result<Option<TransferState>, HostError> {
        let session = self.sessions.get(&peer).ok_or(HostError::UnknownPeer)?;
        if !session.snapshot {
            return Ok(None);
        }
        Ok(Some(TransferState {
            next_transfer: session.next_transfer,
            in_flight: session.in_flight,
        }))
    }

    /// One upstream transfer frame (S§9): `Published` or `Resync`, decoded by
    /// `crpg_net::codec_transfer::decode_up`. Refusals are a disposition and
    /// change nothing but the shared intent rate tokens; an honoured `Resync`
    /// stages a fresh snapshot transfer atomically.
    pub fn ingest_transfer(
        &mut self,
        peer: PeerHandle,
        bytes: &[u8],
        now_ms: u64,
    ) -> Result<TransferDisposition, HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        self.check_time(now_ms)?;
        self.last_now_ms = now_ms;

        enum Effect {
            Reply(TransferDisposition),
            Resync {
                grants: DisclosureGrants,
                map: BTreeMap<EntityId, NetId>,
                next_net: u64,
            },
        }

        let effect = {
            let Some(session) = self.sessions.get_mut(&peer) else {
                return Ok(TransferDisposition::Refused(TransferRefusal::Unbound));
            };
            if !session.snapshot {
                return Ok(TransferDisposition::Refused(
                    TransferRefusal::NotSnapshotPeer,
                ));
            }
            // Both budgets deduct per attempt, so neither short-circuits the
            // other (exactly `ingest`'s rule).
            let frames_ok = session.intent_frames.consume(now_ms, 1);
            let bytes_ok = session.intent_bytes.consume(now_ms, bytes.len() as u64);
            if !frames_ok || !bytes_ok {
                return Ok(TransferDisposition::Refused(TransferRefusal::RateLimited));
            }
            let message = match codec_transfer::decode_up(bytes) {
                Ok(message) => message,
                Err(_) => {
                    return Ok(TransferDisposition::Refused(TransferRefusal::Malformed));
                }
            };
            let epoch = match &message {
                TransferUp::Published { epoch, .. } | TransferUp::Resync { epoch, .. } => *epoch,
            };
            if epoch != session.epoch {
                return Ok(TransferDisposition::Refused(TransferRefusal::WrongEpoch));
            }
            match message {
                TransferUp::Published { transfer, .. } => {
                    let t = transfer.get();
                    let issued = session.next_transfer - 1;
                    if session.in_flight == Some(t) {
                        session.in_flight = None;
                        Effect::Reply(TransferDisposition::Published)
                    } else if t <= issued {
                        Effect::Reply(TransferDisposition::Stale)
                    } else {
                        Effect::Reply(TransferDisposition::Refused(
                            TransferRefusal::UnknownTransfer,
                        ))
                    }
                }
                TransferUp::Resync { after, .. } => {
                    let issued = session.next_transfer - 1;
                    let a = after.map_or(0, TransferId::get);
                    if a > issued {
                        Effect::Reply(TransferDisposition::Refused(
                            TransferRefusal::UnknownTransfer,
                        ))
                    } else {
                        Effect::Resync {
                            grants: session.grants.clone(),
                            map: session.map.clone(),
                            next_net: session.next_net,
                        }
                    }
                }
            }
        };

        match effect {
            Effect::Reply(reply) => Ok(reply),
            Effect::Resync {
                grants,
                map,
                next_net,
            } => {
                self.stage_snapshot(peer, grants, map, next_net, SnapshotCause::Resync)?;
                Ok(TransferDisposition::Resynced)
            }
        }
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

    /// The `Command` entries of the page
    /// [`read_capture_entries`](Self::read_capture_entries) returns for the
    /// same arguments, in order (a page may hold fewer than `limit` records
    /// when host records sit in it). Inert: advances nothing.
    pub fn read_captures(
        &self,
        after: u64,
        limit: usize,
    ) -> Result<Vec<CapturedRecord>, HostError> {
        Ok(self
            .read_capture_entries(after, limit)?
            .into_iter()
            .filter_map(|entry| match entry {
                CaptureEntry::Command(record) => Some(record),
                CaptureEntry::Host(_) => None,
            })
            .collect())
    }

    /// Up to `limit` cloned entries with `capture_seq > after`, in order: the
    /// same cursor rules and errors as [`read_captures`](Self::read_captures).
    /// Inert: advances nothing.
    pub fn read_capture_entries(
        &self,
        after: u64,
        limit: usize,
    ) -> Result<Vec<CaptureEntry>, HostError> {
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

    /// One trusted `HistoryWorld::tick`, the first host operation (T141s).
    ///
    /// A real-time tick journals its events, so it first reserves journal
    /// room ([`HostError::CaptureFull`]) and then records them as a
    /// [`HostRecord`] (`HostOp::Tick`) before the sim acknowledgement. A
    /// turn-based, turn-based-only or no-encounter tick reserves nothing and
    /// journals nothing. It is never a peer command: no sequence, no receipt,
    /// `executions()` unmoved, and no frame for v1/v2 sessions.
    pub fn tick(&mut self) -> Result<(), HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        if self.is_real_time() {
            self.reserve_host_record()?;
        }
        let before = self.history.last_sequence();
        self.history
            .tick()
            .map_err(|_| HostError::Invariant("tick refused by the history caps"))?;
        self.complete_host_op(HostOp::Tick, before)
    }

    /// One trusted host action (T141a, A-2): the AI's decision for a
    /// combatant, validated by [`validate_action`], journaled, recorded as a
    /// [`HostOp::Action`] [`HostRecord`] when it journaled any event, and
    /// returned with the sim's outcome.
    ///
    /// It reserves journal room first ([`HostError::CaptureFull`]), then
    /// maps any sim refusal to [`HostError::Combat`] with nothing changed.
    /// It is never a peer command: no sequence, no receipt, no
    /// [`executions`](Self::executions) move, no
    /// [`last_combat_error`](Self::last_combat_error), and no frame for a
    /// v1/v2 session. Pause does not stop it (ADR-0029 P2).
    pub fn host_action(
        &mut self,
        action: &CombatAction,
    ) -> Result<Option<crpg_sim::ActionOutcome>, HostError> {
        self.run_host_action(HostOp::Action, action)
    }

    /// The body of [`host_action`](Self::host_action), recorded as `op`.
    fn run_host_action(
        &mut self,
        op: HostOp,
        action: &CombatAction,
    ) -> Result<Option<crpg_sim::ActionOutcome>, HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        self.reserve_host_record()?;
        validate_action(self.history.world(), action).map_err(HostError::Combat)?;
        let before = self.history.last_sequence();
        let outcome = match self.history.perform_action(action) {
            Ok(outcome) => outcome,
            Err(HistoryError::Combat(error)) => return Err(HostError::Combat(error)),
            Err(_) => return Err(HostError::Invariant("history refused a host operation")),
        };
        self.complete_host_op(op, before)?;
        Ok(outcome)
    }

    /// The operator's mode request (T141a, A-3; ADR-0029 T9):
    /// [`HistoryWorld::request_mode`], applied at the next round boundary.
    ///
    /// A sim refusal (`NoEncounter`, `ModeUnsupported`) is
    /// [`HostError::Combat`] with nothing changed. A request journals no
    /// event, so it makes no record and reserves no room.
    pub fn request_mode(&mut self, mode: EncounterMode) -> Result<(), HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        let before = self.history.last_sequence();
        match self.history.request_mode(mode) {
            Ok(()) => {}
            Err(HistoryError::Combat(error)) => return Err(HostError::Combat(error)),
            Err(_) => return Err(HostError::Invariant("history refused a host operation")),
        }
        self.complete_host_op(HostOp::Action, before)
    }

    /// The combatants the AI must decide for now (T141a, A-4), in order.
    ///
    /// Reads the world only. Turn-based: the active head. Real-time (T15): when
    /// at least two combatants are alive and the clock is below its maximum,
    /// every timeline entry due at the clock (`key <= elapsed`) whose
    /// combatant is alive and has no standing order, in timeline order. A
    /// closed host or no encounter: empty. Which combatants are AI-controlled
    /// is the caller's fact; this lists everyone that has no order.
    pub fn awaiting_orders(&self) -> Vec<EntityId> {
        if self.closed {
            return Vec::new();
        }
        let world = self.history.world();
        let Some(combat) = world.combat() else {
            return Vec::new();
        };
        match combat.mode {
            EncounterMode::TurnBased => combat.active.into_iter().collect(),
            EncounterMode::RealTime => {
                let Some(clock) = combat.clock else {
                    return Vec::new();
                };
                let alive = world
                    .combatants()
                    .iter()
                    .filter(|(_, state)| !state.dead())
                    .count();
                if alive < 2 || clock.elapsed >= crpg_sim::RT_CLOCK_MAX {
                    return Vec::new();
                }
                world
                    .timeline()
                    .iter()
                    .filter(|(key, _)| i64::from(key.0) <= clock.elapsed as i64)
                    .filter(|(_, entity)| {
                        world
                            .combatants()
                            .get(*entity)
                            .is_some_and(|state| !state.dead() && state.order().is_none())
                    })
                    .map(|(_, entity)| entity)
                    .collect()
            }
        }
    }

    /// Whether the encounter is in real-time mode, the only mode whose ticks
    /// journal events.
    fn is_real_time(&self) -> bool {
        self.history
            .world()
            .combat()
            .is_some_and(|combat| combat.mode == EncounterMode::RealTime)
    }

    /// Check-only worst-case room for one host record (S-4 step 2).
    fn reserve_host_record(&self) -> Result<(), HostError> {
        if self.last_capture >= u64::MAX - 1 {
            return Err(HostError::CounterExhausted);
        }
        if self.captures.len() + 1 > MAX_CAPTURE_RECORDS
            || self.capture_bytes + MAX_HOST_RECORD_BYTES > MAX_CAPTURE_BYTES
        {
            return Err(HostError::CaptureFull);
        }
        Ok(())
    }

    /// The tail of a host operation whose sim call committed at
    /// `history.last_sequence()`, with `before` the sequence before the call
    /// (S-3). A zero-event operation makes no record.
    fn complete_host_op(&mut self, op: HostOp, before: u64) -> Result<(), HostError> {
        let after = self.history.last_sequence();
        // Step 1 of the tail (S-3): auto-pause, then the turn timer, for
        // every committed operation, recorded or not.
        self.after_commit(before, after);
        if after == before {
            return Ok(());
        }
        let record = self.read_host_record(op, before, after);
        self.finish_host_op(op, before, record)
    }

    /// Step 2 of the tail: reads the operation's envelopes and builds its
    /// record.
    fn read_host_record(
        &self,
        op: HostOp,
        before: u64,
        after: u64,
    ) -> Result<HostRecord, HostError> {
        if after - before > MAX_HOST_RECORD_EVENTS as u64 {
            return Err(HostError::Invariant("host record exceeds its bound"));
        }
        let mut events: Vec<HistoryEnvelope> = Vec::with_capacity((after - before) as usize);
        let mut cursor = before;
        while cursor < after {
            let page = self
                .history
                .read_after(cursor, MAX_HISTORY_PAGE)
                .map_err(|_| HostError::Invariant("captured range unreadable"))?;
            if page.is_empty() {
                return Err(HostError::Invariant("captured range incomplete"));
            }
            cursor += page.len() as u64;
            events.extend(page);
        }
        build_host_record(
            self.last_capture + 1,
            op,
            before,
            after,
            self.server_tick(),
            events,
        )
    }

    /// Steps 3 to 5 of the tail (S-3): publication (v3 sessions, later
    /// parts), the append, then the sim acknowledgement, which always runs.
    /// A `record` that could not be built is returned only after the
    /// acknowledgement, with nothing appended: evidence is then missing, so
    /// the next checkpoint fails to load, visibly.
    fn finish_host_op(
        &mut self,
        op: HostOp,
        before: u64,
        record: Result<HostRecord, HostError>,
    ) -> Result<(), HostError> {
        let after = self.history.last_sequence();
        let appended = record.map(|record| {
            debug_assert_eq!(record.op, op);
            debug_assert_eq!(record.history_start, before);
            debug_assert_eq!(record.history_end, after);
            let len = host_record_len(&record);
            self.last_capture = record.capture_seq;
            self.capture_bytes += len;
            self.capture_lens.push_back(len);
            self.captures.push_back(CaptureEntry::Host(record));
        });
        let acknowledged = self
            .history
            .acknowledge(after)
            .map_err(|_| HostError::Invariant("sim acknowledgement failed"));
        appended.and(acknowledged)
    }

    // -- pause, settings and the turn timer (T141b) -------------------------

    /// The operator's session settings.
    pub fn settings(&self) -> HostSettings {
        self.settings
    }

    /// Replaces the operator's settings and records `now_ms` (B-1).
    ///
    /// Precedence: [`Closed`](HostError::Closed),
    /// [`TimeRegression`](HostError::TimeRegression), then
    /// [`InvalidSettings`](HostError::InvalidSettings) for a zero timer
    /// length. A changed timer length disarms the timer and observes the
    /// encounter again, so the new length takes effect at once.
    pub fn set_settings(&mut self, settings: HostSettings, now_ms: u64) -> Result<(), HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        self.check_time(now_ms)?;
        if settings.turn_timer_ms == Some(0) {
            return Err(HostError::InvalidSettings);
        }
        self.last_now_ms = now_ms;
        let timer_changed = settings.turn_timer_ms != self.settings.turn_timer_ms;
        self.settings = settings;
        if timer_changed {
            self.disarm_timer();
            self.observe_timer();
        }
        Ok(())
    }

    /// The pause state: `None` when running (P1).
    pub fn paused(&self) -> Option<PauseReason> {
        self.paused
    }

    /// The operator's pause (reason `Operator`); a no-op when already paused,
    /// and the reason stays (B-2).
    pub fn pause(&mut self, now_ms: u64) -> Result<(), HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        self.check_time(now_ms)?;
        self.last_now_ms = now_ms;
        if self.paused.is_none() {
            self.set_paused(PauseReason::Operator);
        }
        Ok(())
    }

    /// The operator's unpause, whatever the reason; a no-op when running
    /// (B-2).
    pub fn unpause(&mut self, now_ms: u64) -> Result<(), HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        self.check_time(now_ms)?;
        self.last_now_ms = now_ms;
        if self.paused.take().is_some() {
            self.resume_timer();
        }
        Ok(())
    }

    /// The pause-aware tick driver (B-3): the turn timer, then one
    /// [`tick`](Self::tick) unless the host is paused.
    ///
    /// Precedence: [`Closed`](HostError::Closed),
    /// [`TimeRegression`](HostError::TimeRegression); `now_ms` is then
    /// recorded. A timer `EndTurn` stays committed even if the tick then
    /// fails. One call ticks at most once and ends at most one turn.
    pub fn advance(&mut self, now_ms: u64) -> Result<AdvanceReport, HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        self.check_time(now_ms)?;
        self.last_now_ms = now_ms;
        let timer_ended_turn = if self.paused.is_none() {
            self.expire_timer(now_ms)
        } else {
            None
        };
        if self.paused.is_some() {
            return Ok(AdvanceReport {
                ticked: false,
                timer_ended_turn,
                paused: self.paused,
            });
        }
        self.tick()?;
        Ok(AdvanceReport {
            ticked: true,
            timer_ended_turn,
            paused: self.paused,
        })
    }

    /// Remaining turn-timer milliseconds at the last accepted `now_ms`:
    /// frozen while paused, `None` when no timer is armed (B-9).
    pub fn turn_timer_remaining_ms(&self) -> Option<u64> {
        self.timer.map(|timer| match timer.clock {
            TimerClock::Running { deadline } => deadline.saturating_sub(self.last_now_ms),
            TimerClock::Frozen { remaining } => remaining,
        })
    }

    /// Counts the armed timer's arm, disarm, freeze and resume events (B-9).
    /// T141d's host-facts image reads it so that the countdown alone never
    /// re-sends `HostFacts`.
    #[allow(dead_code)]
    pub(crate) fn timer_epoch(&self) -> u64 {
        self.timer_epoch
    }

    /// Pauses with `reason` and freezes the armed timer (B-9). The caller
    /// checked that the host is running.
    fn set_paused(&mut self, reason: PauseReason) {
        self.paused = Some(reason);
        let now = self.last_now_ms;
        if let Some(timer) = &mut self.timer {
            if let TimerClock::Running { deadline } = timer.clock {
                timer.clock = TimerClock::Frozen {
                    remaining: deadline.saturating_sub(now),
                };
                self.timer_epoch += 1;
            }
        }
    }

    /// Resumes a frozen timer from the last accepted `now_ms` (B-9).
    fn resume_timer(&mut self) {
        let now = self.last_now_ms;
        if let Some(timer) = &mut self.timer {
            if let TimerClock::Frozen { remaining } = timer.clock {
                timer.clock = TimerClock::Running {
                    deadline: now.saturating_add(remaining),
                };
                self.timer_epoch += 1;
            }
        }
    }

    fn disarm_timer(&mut self) {
        if self.timer.take().is_some() {
            self.timer_epoch += 1;
        }
    }

    /// Step 1 of the post-commit tail for an operation whose sim call
    /// committed `(before, after]` (S-3, A-1): auto-pause, then the timer.
    /// Infallible, so nothing can fail between the sim commit and the record.
    fn after_commit(&mut self, before: u64, after: u64) {
        self.auto_pause(before, after);
        self.observe_timer();
    }

    /// Auto-pause (B-5): the first journaled event, in order, that maps to a
    /// trigger whose toggle is on pauses a running host. The party is the
    /// union of every bound session's control list.
    fn auto_pause(&mut self, before: u64, after: u64) {
        if self.paused.is_some() || after <= before || self.settings.auto_pause.bits() == 0 {
            return;
        }
        let party: BTreeSet<EntityId> = self
            .sessions
            .values()
            .flat_map(|session| session.control.iter().copied())
            .collect();
        let mut cursor = before;
        while cursor < after {
            let Ok(page) = self.history.read_after(cursor, MAX_HISTORY_PAGE) else {
                return;
            };
            if page.is_empty() {
                return;
            }
            for envelope in &page {
                let trigger = match &envelope.payload {
                    HistoryEvent::RoundStarted { .. } => Some(AutoPauseTrigger::RoundStarted),
                    HistoryEvent::ModeChanged { .. } => Some(AutoPauseTrigger::ModeChanged),
                    HistoryEvent::OrderEnded { .. } => Some(AutoPauseTrigger::OrderEnded),
                    HistoryEvent::Died { entity } if party.contains(entity) => {
                        Some(AutoPauseTrigger::PartyMemberDied)
                    }
                    _ => None,
                };
                if let Some(trigger) = trigger {
                    if self.settings.auto_pause.get(trigger) {
                        self.set_paused(PauseReason::Auto(trigger));
                        return;
                    }
                }
            }
            cursor += page.len() as u64;
        }
    }

    /// Timer observation (B-7): disarms when the timer is off or no
    /// turn-based head exists; arms with the full length when no timer is
    /// armed or the armed head or round differs.
    fn observe_timer(&mut self) {
        let key = self.history.world().combat().and_then(|combat| {
            if combat.mode == EncounterMode::TurnBased {
                combat.active.map(|head| (head, combat.round))
            } else {
                None
            }
        });
        let (Some(length), Some((head, round))) = (self.settings.turn_timer_ms, key) else {
            self.disarm_timer();
            return;
        };
        if self
            .timer
            .is_some_and(|timer| timer.head == head && timer.round == round)
        {
            return;
        }
        let length = u64::from(length);
        let clock = if self.paused.is_some() {
            TimerClock::Frozen { remaining: length }
        } else {
            TimerClock::Running {
                deadline: self.last_now_ms.saturating_add(length),
            }
        };
        self.timer = Some(TurnTimer { head, round, clock });
        self.timer_epoch += 1;
    }

    /// Timer expiry (B-8), `advance` step 3: ends the armed head's turn as a
    /// [`HostOp::TurnTimer`] host action. Returns the head whose turn ended.
    fn expire_timer(&mut self, now_ms: u64) -> Option<EntityId> {
        let timer = self.timer?;
        let TimerClock::Running { deadline } = timer.clock else {
            return None;
        };
        if now_ms < deadline {
            return None;
        }
        let action = CombatAction::EndTurn { actor: timer.head };
        match self.run_host_action(HostOp::TurnTimer, &action) {
            Ok(_) => Some(timer.head),
            // No journal room: stay armed and retry on the next call.
            Err(HostError::CaptureFull) => None,
            Err(_) => {
                self.disarm_timer();
                None
            }
        }
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
    for mut frame in frames {
        frame.ordinal = session.next_ordinal;
        session.next_ordinal = session
            .next_ordinal
            .checked_add(1)
            .ok_or(HostError::CounterExhausted)?;
        if let Some(event_seq) = frame.event_seq {
            debug_assert_eq!(event_seq, session.next_event_seq);
            session.next_event_seq = session
                .next_event_seq
                .checked_add(1)
                .ok_or(HostError::CounterExhausted)?;
        }
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

/// `resolve_action`'s mapping/ownership/disclosure test for one sim action;
/// `None` drops it. The actor is the L3-eligible `active`.
fn legal_body(session: &Session, action: &CombatAction) -> Option<IntentBody> {
    match *action {
        CombatAction::UseAbility {
            ability, target, ..
        } => {
            let target_net = *session.map.get(&target)?;
            let target_grant = lookup(&session.grants, target)?;
            if !(target_grant.present && target_grant.action_target) {
                return None;
            }
            Some(IntentBody::DeclareAction {
                ability,
                target: target_net,
            })
        }
        CombatAction::EndTurn { .. } => Some(IntentBody::EndTurn),
        CombatAction::Stop { .. } => None,
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
        CombatAction::EndTurn { actor } | CombatAction::Stop { actor } => {
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
/// Step 2 of the host-operation tail (T141s S-3): the record for one host
/// operation's envelopes, checked against the production bounds. A record
/// over a bound is `Invariant("host record exceeds its bound")`.
fn build_host_record(
    capture_seq: u64,
    op: HostOp,
    history_start: u64,
    history_end: u64,
    processed_tick: u64,
    events: Vec<HistoryEnvelope>,
) -> Result<HostRecord, HostError> {
    let record = HostRecord {
        capture_seq,
        op,
        history_start,
        history_end,
        processed_tick,
        events,
    };
    check_host_record(
        &record,
        processed_tick,
        history_end,
        MAX_HOST_RECORD_EVENTS,
        MAX_HOST_RECORD_BYTES,
    )
    .map_err(|_| HostError::Invariant("host record exceeds its bound"))?;
    Ok(record)
}

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
    /// missing field or grant is `None`/`false`. The five version 2 events
    /// are withheld (`None`) until T140/T141 give them a wire form.
    fn candidate(&self, event: &HistoryEvent) -> Option<EventCandidate> {
        match event {
            HistoryEvent::Spawned { entity } => Some(EventCandidate::Spawned {
                entity: self.net_if(*entity, |_| true),
                disclose: self.grant(*entity).is_some_and(|g| g.present && g.spawn),
            }),
            HistoryEvent::Despawned { entity } => Some(EventCandidate::Despawned {
                entity: self.net_if(*entity, |_| true),
                disclose: self.grant(*entity).is_some_and(|g| g.present && g.despawn),
            }),
            HistoryEvent::Died { entity } => Some(EventCandidate::Died {
                entity: self.net_if(*entity, |_| true),
                disclose: self.grant(*entity).is_some_and(|g| g.present && g.died),
            }),
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
                Some(EventCandidate::ActionResolved {
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
                })
            }
            HistoryEvent::TurnStarted { actor, round } => Some(EventCandidate::TurnStarted {
                actor: self.net_if(*actor, |g| g.turn_start),
                round: self.grants.turn_round.then_some(*round),
            }),
            HistoryEvent::EncounterEnded { encounter, round } => {
                Some(EventCandidate::EncounterEnded {
                    encounter: self
                        .grants
                        .encounters
                        .binary_search(encounter)
                        .is_ok()
                        .then_some(*encounter),
                    round: self.grants.encounter_end.then_some(*round),
                })
            }
            HistoryEvent::RoundStarted { .. }
            | HistoryEvent::ModeChanged { .. }
            | HistoryEvent::OrderEnded { .. }
            | HistoryEvent::Joined { .. }
            | HistoryEvent::Left { .. }
            | HistoryEvent::Moved { .. } => None,
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

    /// The S§7.1 snapshot body: one [`SnapshotEntity`] per mapped entity,
    /// ascending `NetId`, plus the permitted turn, from the same predicates
    /// as [`full_state_ops`](Self::full_state_ops). Nothing else can enter.
    fn snapshot_body(&self, world: &World) -> SnapshotBody {
        let mut mapped: Vec<(NetId, EntityId)> = self
            .map
            .iter()
            .map(|(entity, net)| (*net, *entity))
            .collect();
        mapped.sort();
        let entities: Vec<SnapshotEntity> = mapped
            .iter()
            .map(|(net, entity)| SnapshotEntity {
                entity: *net,
                health: self.health(world, *entity).map(|entry| HealthFact {
                    health: entry.health,
                    max_health: entry.max_health,
                    dead: entry.dead,
                }),
            })
            .collect();
        let turn = self.turn(world).map(|turn| TurnFact {
            active: turn.active,
            round: turn.round,
        });
        SnapshotBody { entities, turn }
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

    // -- snapshot-session defence-in-depth unit cases (S§11 cases 12–13) ----

    fn eid(index: u32) -> EntityId {
        serde_json::from_value(serde_json::json!({ "index": index, "generation": 1 }))
            .expect("valid entity id")
    }

    fn full_grants() -> DisclosureGrants {
        let full = |index: u32| EntityDisclosure {
            entity: eid(index),
            present: true,
            health: true,
            spawn: true,
            despawn: true,
            died: true,
            action_actor: true,
            action_target: true,
            action_outcome: true,
            action_damage: true,
            turn_start: true,
        };
        DisclosureGrants {
            entities: vec![full(0), full(1), full(2)],
            abilities: vec![
                Ulid::from_u128(603),
                Ulid::from_u128(604),
                Ulid::from_u128(605),
            ],
            encounters: vec![Ulid::from_u128(601)],
            turn_round: true,
            encounter_end: true,
            turn_state: true,
        }
    }

    fn hide_b(mut grants: DisclosureGrants) -> DisclosureGrants {
        grants.entities[1].present = false;
        grants
    }

    fn fixture_host() -> Host {
        let mut history: HistoryWorld =
            serde_json::from_str(include_str!("../tests/fixtures/three_combatants.json"))
                .expect("fixture loads");
        history
            .acknowledge(history.last_sequence())
            .expect("acknowledges its own range");
        Host::from_history(
            history,
            HostConfig {
                protocol: ProtocolSelection::V1,
                incarnation: 1,
            },
            0,
        )
        .expect("wraps an acknowledged authority")
    }

    fn ack_all(host: &mut Host, peer: PeerHandle) {
        let entries = host.take_delivery_entries(peer).expect("bound");
        if let Some(last) = entries.last() {
            host.acknowledge_delivery(peer, last.ordinal).expect("acks");
        }
    }

    /// The S§7 no-change field list (rate buckets excluded; the caller checks
    /// those separately when a `Resync` spends them).
    #[derive(Debug, PartialEq, Eq)]
    struct SessionEvidence {
        grants: DisclosureGrants,
        map: BTreeMap<EntityId, NetId>,
        log: Vec<(u64, Option<u64>, Vec<u8>)>,
        log_bytes: usize,
        delivered: u64,
        next_ordinal: u64,
        next_transfer: u64,
        in_flight: Option<u64>,
        egress_bytes: usize,
    }

    fn session_evidence(host: &Host, peer: PeerHandle) -> SessionEvidence {
        let session = host.sessions.get(&peer).expect("bound");
        SessionEvidence {
            grants: session.grants.clone(),
            map: session.map.clone(),
            log: session
                .log
                .iter()
                .map(|frame| (frame.ordinal, frame.event_seq, frame.bytes.clone()))
                .collect(),
            log_bytes: session.log_bytes,
            delivered: session.delivered,
            next_ordinal: session.next_ordinal,
            next_transfer: session.next_transfer,
            in_flight: session.in_flight,
            egress_bytes: host.egress_bytes,
        }
    }

    #[test]
    fn snapshot_staging_without_room_changes_nothing() {
        let mut host = fixture_host();
        let peer = host
            .bind_snapshot_peer(ControlGrant { actors: vec![] }, full_grants(), 0)
            .expect("binds");
        let caps = host.egress_caps;
        let original_egress = host.egress_bytes;
        host.egress_bytes = caps.host_bytes + 1;
        let before = session_evidence(&host, peer);
        assert_eq!(
            host.set_grants(peer, hide_b(full_grants())),
            Err(HostError::QueueFull)
        );
        assert_eq!(session_evidence(&host, peer), before);

        let resync = codec_transfer::encode_up(&TransferUp::Resync {
            epoch: host.epoch(peer).expect("bound"),
            after: None,
            reason: crpg_net::protocol_transfer::ResyncReason::Overflow,
        })
        .expect("encodes");
        assert_eq!(
            host.ingest_transfer(peer, &resync, 0),
            Err(HostError::QueueFull)
        );
        assert_eq!(session_evidence(&host, peer), before);

        // Control: with egress restored, both succeed.
        host.egress_bytes = original_egress;
        assert_eq!(
            host.set_grants(peer, hide_b(full_grants())),
            Ok(GrantUpdate::SnapshotStaged)
        );
        assert_eq!(
            host.ingest_transfer(peer, &resync, 1),
            Ok(TransferDisposition::Resynced)
        );
    }

    #[test]
    fn snapshot_counter_exhaustion_changes_nothing() {
        // (a) next_transfer sentinel.
        {
            let mut host = fixture_host();
            let peer = host
                .bind_snapshot_peer(ControlGrant { actors: vec![] }, full_grants(), 0)
                .expect("binds");
            ack_all(&mut host, peer);
            host.sessions.get_mut(&peer).expect("bound").next_transfer = u64::MAX;
            let before = session_evidence(&host, peer);
            assert_eq!(
                host.set_grants(peer, hide_b(full_grants())),
                Err(HostError::CounterExhausted)
            );
            assert_eq!(session_evidence(&host, peer), before);
            let resync = codec_transfer::encode_up(&TransferUp::Resync {
                epoch: host.epoch(peer).expect("bound"),
                after: None,
                reason: crpg_net::protocol_transfer::ResyncReason::Overflow,
            })
            .expect("encodes");
            assert_eq!(
                host.ingest_transfer(peer, &resync, 0),
                Err(HostError::CounterExhausted)
            );
            assert_eq!(session_evidence(&host, peer), before);
            // Control: u64::MAX − 1 stages Begin for that id and advances.
            host.sessions.get_mut(&peer).expect("bound").next_transfer = u64::MAX - 1;
            assert_eq!(
                host.set_grants(peer, hide_b(full_grants())),
                Ok(GrantUpdate::SnapshotStaged)
            );
            let session = host.sessions.get(&peer).expect("bound");
            assert_eq!(session.next_transfer, u64::MAX);
            assert_eq!(session.in_flight, Some(u64::MAX - 1));
        }
        // (b) next_ordinal bound.
        {
            let mut host = fixture_host();
            let peer = host
                .bind_snapshot_peer(ControlGrant { actors: vec![] }, full_grants(), 0)
                .expect("binds");
            ack_all(&mut host, peer);
            {
                let session = host.sessions.get_mut(&peer).expect("bound");
                session.next_ordinal = u64::MAX - 2;
                session.delivered = u64::MAX - 3;
            }
            let before = session_evidence(&host, peer);
            assert_eq!(
                host.set_grants(peer, hide_b(full_grants())),
                Err(HostError::CounterExhausted)
            );
            assert_eq!(session_evidence(&host, peer), before);
            let resync = codec_transfer::encode_up(&TransferUp::Resync {
                epoch: host.epoch(peer).expect("bound"),
                after: None,
                reason: crpg_net::protocol_transfer::ResyncReason::Overflow,
            })
            .expect("encodes");
            assert_eq!(
                host.ingest_transfer(peer, &resync, 0),
                Err(HostError::CounterExhausted)
            );
            assert_eq!(session_evidence(&host, peer), before);
            // Control: u64::MAX − 3 stages at u64::MAX − 3 and u64::MAX − 2.
            {
                let session = host.sessions.get_mut(&peer).expect("bound");
                session.next_ordinal = u64::MAX - 3;
                session.delivered = u64::MAX - 4;
            }
            assert_eq!(
                host.set_grants(peer, hide_b(full_grants())),
                Ok(GrantUpdate::SnapshotStaged)
            );
            let session = host.sessions.get(&peer).expect("bound");
            assert_eq!(session.next_ordinal, u64::MAX - 1);
            let entries = host.take_delivery_entries(peer).expect("bound");
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].ordinal, u64::MAX - 3);
            assert_eq!(entries[1].ordinal, u64::MAX - 2);
        }
    }

    // -- T047a read API smoke cases (the full suite is tests/host_legal.rs) --

    fn net(value: u64) -> NetId {
        NetId::new(value).expect("nonzero")
    }

    #[test]
    fn host_read_api_smoke() {
        let mut host = fixture_host();
        let peer = host
            .bind_peer(
                ControlGrant {
                    actors: vec![eid(0)],
                },
                full_grants(),
                0,
            )
            .expect("binds");
        assert_eq!(host.replica_id(peer, eid(0)), Ok(Some(net(1))));
        assert_eq!(host.replica_id(peer, eid(1)), Ok(Some(net(2))));
        assert_eq!(host.replica_id(peer, eid(2)), Ok(Some(net(3))));
        assert_eq!(host.replica_id(peer, eid(99)), Ok(None));

        let intents = host.legal_intents(peer).expect("reads");
        assert_eq!(intents.len(), 7);
        assert!(intents.iter().all(|intent| intent.actor == net(1)));
        assert_eq!(
            intents[0].body,
            IntentBody::DeclareAction {
                ability: Ulid::from_u128(603),
                target: net(2),
            }
        );
        assert_eq!(intents[6].body, IntentBody::EndTurn);

        // No encounter: no active head, nothing to map.
        let mut empty = Host::new(
            7,
            HostConfig {
                protocol: ProtocolSelection::V2,
                incarnation: 1,
            },
            0,
        )
        .expect("empty host");
        let empty_peer = empty
            .bind_peer(ControlGrant { actors: vec![] }, full_grants(), 0)
            .expect("binds");
        assert_eq!(empty.legal_intents(empty_peer), Ok(Vec::new()));
        assert_eq!(empty.replica_id(empty_peer, eid(0)), Ok(None));

        // Shutdown clears sessions first, so Closed wins over UnknownPeer.
        host.shutdown();
        assert_eq!(host.legal_intents(peer), Err(HostError::Closed));
        assert_eq!(host.replica_id(peer, eid(0)), Err(HostError::Closed));
    }

    // -- T141s host records: the tail's private seams (S7) and bounds -------

    /// `TB3` (`real_time: false`) or `RT3` saved-game JSON, as in
    /// `tests/host_records.rs`.
    fn host_fixture_value(real_time: bool) -> serde_json::Value {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/three_combatants.json"))
                .expect("fixture parses");
        if real_time {
            value["version"] = serde_json::json!(2);
            value["world"]["combat"]["clock"] = serde_json::json!({"round_ticks": 4, "elapsed": 0});
            value["world"]["combat"]["mode"] = serde_json::json!("real_time");
            value["world"]["combat"]["active"] = serde_json::Value::Null;
            for entry in value["world"]["timeline"].as_array_mut().expect("timeline") {
                entry[0] = serde_json::json!(0);
            }
        }
        value
    }

    fn v2_host(real_time: bool) -> Host {
        let mut history: HistoryWorld =
            serde_json::from_value(host_fixture_value(real_time)).expect("fixture loads");
        history
            .acknowledge(history.last_sequence())
            .expect("acknowledges its own range");
        Host::from_history(
            history,
            HostConfig {
                protocol: ProtocolSelection::V2,
                incarnation: 1,
            },
            0,
        )
        .expect("wraps an acknowledged authority")
    }

    fn all_actors() -> ControlGrant {
        ControlGrant {
            actors: vec![eid(0), eid(1), eid(2)],
        }
    }

    fn intent(host: &Host, peer: PeerHandle, seq: u64, actor: u64, body: IntentBody) -> Vec<u8> {
        codec_v2::encode_intent(&IntentFrame {
            epoch: host.epoch(peer).expect("bound"),
            lane: LANE_COMBAT,
            seq,
            observed_tick: host.server_tick(),
            actor: net(actor),
            body,
        })
        .expect("encodes")
    }

    /// `RT3` with `V` bound and A's standing order (A strikes B) set through
    /// a real command: entry 1, history `(4, 4]`.
    fn rt3_with_order() -> (Host, PeerHandle) {
        let mut host = v2_host(true);
        let peer = host
            .bind_peer(all_actors(), full_grants(), 0)
            .expect("binds");
        let bytes = intent(
            &host,
            peer,
            1,
            1,
            IntentBody::DeclareAction {
                ability: Ulid::from_u128(603),
                target: net(2),
            },
        );
        assert_eq!(
            host.ingest(peer, &bytes, 100),
            Ok(IngestDisposition::Staged)
        );
        let summary = host.pump(100).expect("pumps");
        assert_eq!(summary.results[0].status, ReceiptStatus::Applied);
        assert_eq!(host.last_capture(), 1);
        (host, peer)
    }

    fn round_started(seq: u64) -> HistoryEnvelope {
        serde_json::from_value(serde_json::json!({
            "tick": 0,
            "seq": seq,
            "payload": {"type": "round_started", "value": {"round": 1}}
        }))
        .expect("envelope parses")
    }

    const RECORD_BOUND: HostError = HostError::Invariant("host record exceeds its bound");

    #[test]
    fn tail_acknowledges_even_when_the_record_fails() {
        let (mut host, _peer) = rt3_with_order();
        host.history.tick().expect("the sim commits the tick");
        assert_eq!(host.history.last_sequence(), 5);
        assert_eq!(host.history.pending_len(), 1);
        let bytes = host.capture_bytes;
        assert_eq!(
            host.finish_host_op(HostOp::Tick, 4, Err(RECORD_BOUND)),
            Err(RECORD_BOUND)
        );
        assert_eq!(host.history.acknowledged(), 5);
        assert_eq!(host.history.pending_len(), 0);
        assert_eq!(host.last_capture(), 1);
        assert_eq!(host.captures.len(), 1);
        assert_eq!(host.capture_bytes, bytes);
        assert_eq!(host.capture_lens.len(), 1);

        // Control: the same state with a built record appends and acknowledges.
        let (mut host, _peer) = rt3_with_order();
        host.history.tick().expect("the sim commits the tick");
        let record = host.read_host_record(HostOp::Tick, 4, 5);
        let built = record.clone().expect("builds");
        assert_eq!(host.finish_host_op(HostOp::Tick, 4, record), Ok(()));
        assert_eq!(host.history.acknowledged(), 5);
        assert_eq!(host.last_capture(), 2);
        assert_eq!(
            host.captures.back(),
            Some(&CaptureEntry::Host(built.clone()))
        );
        assert_eq!(built.capture_seq, 2);
        assert_eq!(built.processed_tick, 1);
        assert_eq!((built.history_start, built.history_end), (4, 5));
        assert_eq!(built.events.len(), 1);
        assert_eq!(host.capture_bytes, host.capture_lens.iter().sum::<usize>());
    }

    #[test]
    fn build_host_record_applies_the_production_bounds() {
        let one = vec![round_started(5)];
        let record = build_host_record(2, HostOp::Tick, 4, 5, 1, one.clone()).expect("builds");
        assert_eq!(
            record,
            HostRecord {
                capture_seq: 2,
                op: HostOp::Tick,
                history_start: 4,
                history_end: 5,
                processed_tick: 1,
                events: one,
            }
        );
        let many =
            |count: u64| -> Vec<HistoryEnvelope> { (1..=count).map(round_started).collect() };
        let at_bound = build_host_record(2, HostOp::Action, 0, 4096, 1, many(4096));
        assert_eq!(at_bound.expect("4,096 events build").events.len(), 4096);
        assert_eq!(
            build_host_record(2, HostOp::Action, 0, 4097, 1, many(4097)),
            Err(RECORD_BOUND)
        );
        // An empty range, a width that disagrees with the events, and a gap
        // in the envelope sequences are each over the bound.
        assert_eq!(
            build_host_record(2, HostOp::Tick, 5, 5, 1, Vec::new()),
            Err(RECORD_BOUND)
        );
        assert_eq!(
            build_host_record(2, HostOp::Tick, 4, 6, 1, vec![round_started(5)]),
            Err(RECORD_BOUND)
        );
        assert_eq!(
            build_host_record(2, HostOp::Tick, 4, 5, 1, vec![round_started(6)]),
            Err(RECORD_BOUND)
        );
    }

    /// A peer command that journals more events than the real-time bound
    /// (four) is `Invariant`, and the sim acknowledgement still runs; four is
    /// the bound and is captured (A-1).
    #[test]
    fn command_tail_acknowledges_even_when_the_record_fails() {
        for (turns, accepted) in [(5usize, false), (4, true)] {
            let mut host = v2_host(false);
            let peer = host
                .bind_peer(all_actors(), full_grants(), 0)
                .expect("binds");
            for index in 0..turns {
                host.history
                    .perform_action(&CombatAction::EndTurn {
                        actor: eid((index % 3) as u32),
                    })
                    .expect("the sim accepts the turn");
            }
            let after = host.history.last_sequence();
            assert_eq!(after, 4 + turns as u64);
            let frame = IntentFrame {
                epoch: host.epoch(peer).expect("bound"),
                lane: LANE_COMBAT,
                seq: 1,
                observed_tick: 0,
                actor: net(1),
                body: IntentBody::EndTurn,
            };
            let result = host.commit_accepted(
                peer,
                &frame,
                Vec::new(),
                &CombatAction::EndTurn { actor: eid(0) },
                None,
                4,
            );
            assert_eq!(host.history.acknowledged(), after, "{turns} events");
            assert_eq!(host.history.pending_len(), 0);
            if accepted {
                assert_eq!(result, Ok(()));
                assert_eq!(host.executions(), 1);
                assert_eq!(host.last_capture(), 1);
                let CaptureEntry::Command(record) = &host.captures[0] else {
                    panic!("a command entry");
                };
                assert_eq!(record.events.len(), 4);
            } else {
                assert_eq!(
                    result,
                    Err(HostError::Invariant("command produced too many events"))
                );
                assert_eq!(host.executions(), 0);
                assert_eq!(host.last_capture(), 0);
                assert!(host.captures.is_empty());
                assert_eq!(host.capture_bytes, 0);
            }
        }
    }

    #[test]
    fn host_record_reservation_bounds() {
        let mut host = v2_host(true);
        // Bytes: room for exactly one worst-case host record is room.
        host.capture_bytes = MAX_CAPTURE_BYTES - MAX_HOST_RECORD_BYTES;
        assert_eq!(host.reserve_host_record(), Ok(()));
        host.capture_bytes += 1;
        assert_eq!(host.reserve_host_record(), Err(HostError::CaptureFull));
        // A real-time tick is refused and changes nothing; a turn-based tick
        // reserves nothing.
        let hash = host.authority_hash();
        assert_eq!(host.tick(), Err(HostError::CaptureFull));
        assert_eq!(host.server_tick(), 0);
        assert_eq!(host.authority_hash(), hash);
        let mut tb = v2_host(false);
        tb.capture_bytes = MAX_CAPTURE_BYTES;
        assert_eq!(tb.tick(), Ok(()));
        assert_eq!(tb.server_tick(), 1);
        // Entries: the count at the bound is full, one below is room.
        let mut host = v2_host(true);
        let filler = CaptureEntry::Host(HostRecord {
            capture_seq: 1,
            op: HostOp::Tick,
            history_start: 4,
            history_end: 5,
            processed_tick: 0,
            events: vec![round_started(5)],
        });
        for _ in 0..MAX_CAPTURE_RECORDS - 1 {
            host.captures.push_back(filler.clone());
        }
        assert_eq!(host.reserve_host_record(), Ok(()));
        host.captures.push_back(filler);
        assert_eq!(host.reserve_host_record(), Err(HostError::CaptureFull));
        // The counter sentinel.
        let mut host = v2_host(true);
        host.last_capture = u64::MAX - 2;
        assert_eq!(host.reserve_host_record(), Ok(()));
        host.last_capture = u64::MAX - 1;
        assert_eq!(host.reserve_host_record(), Err(HostError::CounterExhausted));
    }

    /// A tick the history caps refuse commits nothing and is `Invariant`.
    #[test]
    fn tick_refused_by_the_history_caps_changes_nothing() {
        for (spawns, refused) in [(MAX_HISTORY_EVENTS, true), (MAX_HISTORY_EVENTS - 1, false)] {
            let (mut host, _peer) = rt3_with_order();
            for _ in 0..spawns {
                host.history
                    .spawn(crpg_sim::EntityMeta::default())
                    .expect("fits the journal");
            }
            let hash = host.authority_hash();
            let last = host.last_capture();
            let result = host.tick();
            if refused {
                assert_eq!(
                    result,
                    Err(HostError::Invariant("tick refused by the history caps"))
                );
                assert_eq!(host.server_tick(), 0);
                assert_eq!(host.authority_hash(), hash);
                assert_eq!(host.last_capture(), last);
                assert_eq!(host.history.pending_len(), spawns);
            } else {
                assert_eq!(result, Ok(()));
                assert_eq!(host.server_tick(), 1);
                assert_eq!(host.last_capture(), last + 1);
                assert_eq!(host.history.pending_len(), 0);
            }
        }
    }

    /// A host action the history caps refuse after validation commits nothing
    /// and is `Invariant`; one event below the cap is accepted (A-2 step 4).
    #[test]
    fn host_action_refused_by_the_history_caps_changes_nothing() {
        for (spawns, refused) in [(MAX_HISTORY_EVENTS, true), (MAX_HISTORY_EVENTS - 1, false)] {
            let mut host = v2_host(false);
            for _ in 0..spawns {
                host.history
                    .spawn(crpg_sim::EntityMeta::default())
                    .expect("fits the journal");
            }
            let hash = host.authority_hash();
            let result = host.host_action(&CombatAction::EndTurn { actor: eid(0) });
            if refused {
                assert_eq!(
                    result,
                    Err(HostError::Invariant("history refused a host operation"))
                );
                assert_eq!(host.authority_hash(), hash);
                assert_eq!(host.last_capture(), 0);
                assert!(host.captures.is_empty());
                assert_eq!(host.history.pending_len(), spawns);
            } else {
                assert_eq!(result, Ok(None));
                assert_ne!(host.authority_hash(), hash);
                assert_eq!(host.last_capture(), 1);
                assert_eq!(host.history.pending_len(), 0);
                assert_eq!(host.history.acknowledged(), host.history.last_sequence());
            }
            assert_eq!(host.executions(), 0);
            assert_eq!(host.last_combat_error(), None);
        }
    }

    /// One ledger for both entry kinds: the byte total is the sum of the
    /// entries' compact lengths, and acknowledging retires both kinds.
    #[test]
    fn capture_ledger_counts_both_entry_kinds() {
        let (mut host, _peer) = rt3_with_order();
        host.tick().expect("ticks");
        assert_eq!(host.captures.len(), 2);
        let lengths: Vec<usize> = host.captures.iter().map(CaptureEntry::json_len).collect();
        assert_eq!(
            host.capture_lens.iter().copied().collect::<Vec<_>>(),
            lengths
        );
        assert_eq!(host.capture_bytes, lengths.iter().sum::<usize>());
        let CaptureEntry::Host(record) = &host.captures[1] else {
            panic!("the tick's record");
        };
        assert_eq!(
            lengths[1],
            serde_json::to_vec(record).expect("encodes").len()
        );
        host.acknowledge_captures(2).expect("acks");
        assert_eq!(host.capture_bytes, 0);
        assert!(host.captures.is_empty());
        assert!(host.capture_lens.is_empty());
        assert_eq!(host.capture_acknowledged(), 2);
        assert_eq!(host.last_capture(), 2);
    }

    #[test]
    fn capture_full_displays_and_has_no_source() {
        assert_eq!(
            HostError::CaptureFull.to_string(),
            "CaptureFull at host/admission"
        );
        assert!(std::error::Error::source(&HostError::CaptureFull).is_none());
    }

    /// A peer command reserves history room for four events (S-7).
    #[test]
    fn accepted_room_reserves_four_history_events() {
        let mut host = v2_host(false);
        for _ in 0..MAX_HISTORY_EVENTS - MAX_NEW_EVENTS_PER_COMMAND_RT {
            host.history
                .spawn(crpg_sim::EntityMeta::default())
                .expect("fits the journal");
        }
        assert_eq!(host.history.pending_len(), 4092);
        assert_eq!(host.accepted_room(), Ok(true));
        host.history
            .spawn(crpg_sim::EntityMeta::default())
            .expect("fits the journal");
        assert_eq!(host.history.pending_len(), 4093);
        assert_eq!(host.accepted_room(), Ok(false));
    }

    fn timer_settings(ms: Option<u32>) -> HostSettings {
        HostSettings {
            turn_timer_ms: ms,
            ..HostSettings::multiplayer()
        }
    }

    /// The timer epoch counts arm, disarm, freeze and resume of an armed
    /// timer only (B-9): pausing or unpausing with none armed, an unchanged
    /// setting and a repeated pause or unpause change nothing.
    #[test]
    fn timer_epoch_counts_arm_disarm_freeze_and_resume() {
        let mut host = v2_host(false);
        assert_eq!(host.timer_epoch(), 0);
        // No timer armed: pause and unpause leave the epoch alone.
        host.pause(1).expect("pauses");
        host.unpause(1).expect("unpauses");
        assert_eq!(host.timer_epoch(), 0);
        // Arm.
        host.set_settings(timer_settings(Some(1000)), 2)
            .expect("sets");
        assert_eq!(host.timer_epoch(), 1);
        // The same setting again: unchanged.
        host.set_settings(timer_settings(Some(1000)), 3)
            .expect("sets");
        assert_eq!(host.timer_epoch(), 1);
        // Freeze, a repeated pause (no-op), resume, a repeated unpause.
        host.pause(4).expect("pauses");
        assert_eq!(host.timer_epoch(), 2);
        host.pause(5).expect("pauses");
        assert_eq!(host.timer_epoch(), 2);
        host.unpause(6).expect("unpauses");
        assert_eq!(host.timer_epoch(), 3);
        host.unpause(7).expect("unpauses");
        assert_eq!(host.timer_epoch(), 3);
        // A changed length disarms and re-arms: two events.
        host.set_settings(timer_settings(Some(2000)), 8)
            .expect("sets");
        assert_eq!(host.timer_epoch(), 5);
        // A new head re-arms: a host action ends A's turn (the old timer is
        // replaced, one event).
        host.host_action(&CombatAction::EndTurn { actor: eid(0) })
            .expect("ends the turn");
        assert_eq!(host.timer_epoch(), 6);
        // Off disarms (one event); off again changes nothing.
        host.set_settings(timer_settings(None), 9).expect("sets");
        assert_eq!(host.timer_epoch(), 7);
        host.set_settings(timer_settings(None), 10).expect("sets");
        assert_eq!(host.timer_epoch(), 7);
    }

    /// B-8, "any other `Err` disarms": an expiry whose `EndTurn` the sim
    /// refuses (here, a stale armed head, unreachable through the public API
    /// because observation keeps the key current) disarms the timer, is not
    /// returned, changes nothing else, and the tick still runs. The tick's
    /// own tail then observes the real head and arms a fresh timer (B-7: no
    /// timer armed), from this call's time. The control is the same expiry
    /// with the right head.
    #[test]
    fn timer_expiry_with_a_refused_end_turn_disarms() {
        let stale = |head: EntityId| {
            let mut host = v2_host(false);
            host.set_settings(timer_settings(Some(1000)), 0)
                .expect("sets");
            host.timer.as_mut().expect("armed").head = head;
            host
        };

        let mut host = stale(eid(2));
        let hash = host.authority_hash();
        let epoch = host.timer_epoch();
        assert_eq!(
            host.advance(1000),
            Ok(AdvanceReport {
                ticked: true,
                timer_ended_turn: None,
                paused: None,
            })
        );
        // Disarm, then the tick's observation arms for the real head (A).
        assert_eq!(host.timer_epoch(), epoch + 2);
        assert_eq!(host.timer.map(|timer| timer.head), Some(eid(0)));
        assert_eq!(host.turn_timer_remaining_ms(), Some(1000));
        assert_eq!(host.last_capture(), 0);
        assert_eq!(host.history.last_sequence(), 4);
        assert_eq!(host.server_tick(), 1);
        assert_ne!(host.authority_hash(), hash, "the tick ran");
        // The fresh timer is not due until 2000.
        assert_eq!(host.advance(1999).map(|r| r.timer_ended_turn), Ok(None));
        assert_eq!(
            host.advance(2000).map(|r| r.timer_ended_turn),
            Ok(Some(eid(0)))
        );

        let mut host = stale(eid(0));
        assert_eq!(
            host.advance(1000),
            Ok(AdvanceReport {
                ticked: true,
                timer_ended_turn: Some(eid(0)),
                paused: None,
            })
        );
        assert_eq!(host.last_capture(), 1);
        assert_eq!(host.turn_timer_remaining_ms(), Some(1000));
    }
}
