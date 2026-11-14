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
use crpg_net::codec_v3;
use crpg_net::facts_v3::{
    clock_op, host_facts_op, project_page, project_snapshot_v3, EncounterGrants, HostGrants,
    HostInput,
};
use crpg_net::interest::{EntityKey, InterestView};
use crpg_net::projection_v2::{project_events, EventCandidate};
use crpg_net::projection_v3::{
    project_events as project_events_v3, EventCandidate as EventCandidateV3,
};
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
use crpg_net::protocol_v3::{
    AutoPauseToggles, AutoPauseTrigger, DeltaFrame as DeltaFrameV3, DeltaOp as DeltaOpV3,
    IntentBody as IntentBodyV3, IntentFrame as IntentFrameV3, PauseReason, PauseRights,
    ReceiptStatus as ReceiptV3, RejectionCode as CodeV3, POLICY_CLOCK_HEARTBEAT_TICKS,
};
use crpg_net::sim::{QueueCaps, RateCaps, TokenBucket};
use crpg_net::transfer_v3;
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
use crate::host_v3::{
    host_facts_image, key_of, net_mode, outbox_fits, read_world, sim_mode, Gate, OutboxEntry,
    PagePlan, Throttle, V3Session, V3Stage,
};

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

/// `World::area()` for an unbound world in every v3 `EntityKey` (T141
/// Reading 8).
pub const HOST_UNBOUND_AREA: Ulid = Ulid::from_u128(0);

/// The host-fact grants a v3 session gets unless the operator says otherwise:
/// all three (D52, T140 Question 3).
pub const DEFAULT_HOST_GRANTS: HostGrants = HostGrants {
    pause_reason: true,
    auto_pause: true,
    turn_timer: true,
};

/// While a v3 session's transfer is in flight it gets at most one delta page
/// per this many ticks (T141e, H-8, D54). Policy, never wire.
pub const POLICY_V3_THROTTLE_TICKS: u64 = 10;
/// Delta frames one v3 session may receive while its transfer is in flight:
/// the receiver's held-delta budget (`TransferLimitsV3::v3()`, T140e).
pub const POLICY_V3_IN_FLIGHT_FRAMES: usize = protocol::POLICY_EGRESS_FRAMES_PER_PEER;
/// Delta bytes one v3 session may receive while its transfer is in flight:
/// the receiver's held-delta budget (`TransferLimitsV3::v3()`, T140e).
pub const POLICY_V3_IN_FLIGHT_BYTES: usize = protocol::POLICY_EGRESS_BYTES_PER_PEER;

/// One entity's v3 fact grants. There is no `order` flag: the standing order
/// follows control (T141 Reading 9, N2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityFactGrant {
    /// The whole entity id (generation included).
    pub entity: EntityId,
    /// The entity's pools may be disclosed.
    pub pools: bool,
    /// The entity's attached effects may be disclosed.
    pub effects: bool,
    /// The entity's stat values may be disclosed.
    pub stats: bool,
    /// The entity's record tags may be disclosed.
    pub tags: bool,
}

/// A v3 session's fact grants. `Default` denies everything.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FactGrants {
    /// At most [`MAX_GRANT_IDENTITIES`] entries, unique `entity`.
    pub entities: Vec<EntityFactGrant>,
    /// What the viewer may see of the encounter.
    pub encounter: EncounterGrants,
    /// What the viewer may see of the host's pause and timer.
    pub host: HostGrants,
}

/// What a delivery entry carries, so an adapter picks the channel without
/// reading bytes (T024 S§4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryKind {
    /// A lane-0 delta frame (the host's `ProtocolSelection`, or v3 in a
    /// session bound with `bind_v3_peer`).
    Delta,
    /// A snapshot-transfer frame (`crpg_net::codec_transfer`, first byte 0x81,
    /// or `0x82` in a v3 session).
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
    /// The call needs a session bound with `bind_v3_peer`.
    NotV3Peer,
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
            Self::NotV3Peer => "NotV3Peer",
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
    /// The exact v3 status for every session (a v1/v2 status converts by
    /// `From`, losslessly); the trusted API reports its v1 projection.
    status: ReceiptV3,
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
    /// The v3 state of a session bound with `bind_v3_peer` (T141c).
    v3: Option<Box<V3Session>>,
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

/// One decoded intent in the pipeline's common form: a v1/v2 verb maps into
/// the v3 vocabulary (T141d D-1).
#[derive(Debug, Clone, Copy)]
struct Frame {
    epoch: [u8; 16],
    seq: u64,
    observed_tick: u64,
    actor: NetId,
    body: IntentBodyV3,
}

/// The two header fields the capture steps read from an intent, so that both
/// the v1/v2 and the v3 frame feed one commit.
trait IntentHeader {
    fn header_epoch(&self) -> [u8; 16];
    fn header_seq(&self) -> u64;
}

impl IntentHeader for Frame {
    fn header_epoch(&self) -> [u8; 16] {
        self.epoch
    }
    fn header_seq(&self) -> u64 {
        self.seq
    }
}

impl IntentHeader for IntentFrame {
    fn header_epoch(&self) -> [u8; 16] {
        self.epoch
    }
    fn header_seq(&self) -> u64 {
        self.seq
    }
}

/// What a resolved intent asks of the host (D-2).
#[derive(Debug, Clone, Copy)]
enum Intent {
    /// A sim action, validated by `validate_action`.
    Action(CombatAction),
    /// `Pause`: needs the `pause` right.
    Pause,
    /// `Unpause`: needs the `unpause` right.
    Unpause,
    /// `RequestMode`: needs the `pause` right.
    Mode(EncounterMode),
}

/// An intent resolved against one session: the acting entity and the request.
#[derive(Debug, Clone, Copy)]
struct Resolved {
    actor: EntityId,
    intent: Intent,
}

/// What one operation decided for one v3 session's publication (D-9).
#[derive(Debug)]
enum V3Decision {
    /// Log the page's frames (possibly none) and install its clones.
    Deliver(PagePlan),
    /// The page did not fit: stage a snapshot instead and drop the page.
    Restage(V3Stage),
    /// The page or the restage could not be built: fence the session.
    Fence,
    /// The in-flight throttle logs nothing (T141e, E-2) and changes no view;
    /// `close` ends the session's deltas until its transfer ends (a page the
    /// held-delta budget refused).
    Hold { close: bool },
}

/// What committing one [`V3Decision`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum V3Outcome {
    /// The page was committed; `receipt` is the `(generation, ordinal)` of its
    /// first frame, if it had one.
    Logged { receipt: Option<(u64, u64)> },
    /// A snapshot was staged instead of the page.
    Staged,
    /// The session was fenced.
    Fenced,
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
        let world = self.history.world();
        let net_of = |entity: EntityId| match &session.v3 {
            Some(v3) => v3.view.net_id(key_of(world, entity)),
            None => session.map.get(&entity).copied(),
        };
        let Some(actor_net) = net_of(active) else {
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
                    legal_body(&session.grants, &net_of, action).map(|body| LegalIntent {
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
        if let Some(v3) = &session.v3 {
            return Ok(v3.view.net_id(key_of(self.history.world(), entity)));
        }
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
        let epoch = self.session_epoch(handle);
        let (map, next_net) = plan_map(&BTreeMap::new(), 1, &grants, self.history.world())?;
        let mut session = self.blank_session(epoch, control, grants, snapshot);
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

    /// The epoch of `handle`: `incarnation` LE bytes then handle LE bytes.
    fn session_epoch(&self, handle: PeerHandle) -> [u8; 16] {
        let mut epoch = [0u8; 16];
        epoch[..8].copy_from_slice(&self.config.incarnation.to_le_bytes());
        epoch[8..].copy_from_slice(&handle.0.to_le_bytes());
        epoch
    }

    /// A session with nothing mapped, nothing logged and full rate budgets.
    fn blank_session(
        &self,
        epoch: [u8; 16],
        control: Vec<EntityId>,
        grants: DisclosureGrants,
        snapshot: bool,
    ) -> Session {
        Session {
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
            v3: None,
        }
    }

    /// Binds a protocol v3 session (T141c, C-1): always a snapshot session,
    /// whatever `HostConfig.protocol` says, with its first delivery a `0x82`
    /// transfer 1 at cursor 0 followed by one `[Clock, HostFacts]` delta.
    ///
    /// Precedence: [`Closed`](HostError::Closed),
    /// [`TimeRegression`](HostError::TimeRegression),
    /// [`InvalidControl`](HostError::InvalidControl), then
    /// [`InvalidGrants`](HostError::InvalidGrants) (the disclosure grants,
    /// then the fact grants), [`ServerBusy`](HostError::ServerBusy) and
    /// [`CounterExhausted`](HostError::CounterExhausted). It never returns
    /// [`QueueFull`](HostError::QueueFull): the staged plan waits in a paced
    /// outbox instead of needing egress room.
    pub fn bind_v3_peer(
        &mut self,
        control: ControlGrant,
        grants: DisclosureGrants,
        facts: FactGrants,
        rights: PauseRights,
        now_ms: u64,
    ) -> Result<PeerHandle, HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        self.check_time(now_ms)?;
        let control = normalize_control(control)?;
        let grants = normalize_grants(grants)?;
        let facts = normalize_fact_grants(facts)?;
        if self.sessions.len() >= MAX_PEERS {
            return Err(HostError::ServerBusy);
        }
        if self.next_handle == u64::MAX {
            return Err(HostError::CounterExhausted);
        }
        let handle = PeerHandle(self.next_handle);
        let epoch = self.session_epoch(handle);
        let mut session = self.blank_session(epoch, control, grants, true);
        session.v3 = Some(Box::new(V3Session::new(epoch, facts, rights)));
        let stage = self.plan_v3_stage(&session, &session.grants, SnapshotCause::Resync)?;
        commit_v3_stage(
            &mut self.egress_bytes,
            &self.egress_caps,
            &mut session,
            stage,
            None,
        );
        self.sessions.insert(handle, session);
        self.next_handle += 1;
        self.last_now_ms = now_ms;
        Ok(handle)
    }

    /// The wire version byte of `peer`'s session: `1`, `2` or `3`. Not checked
    /// against shutdown: a closed host has no sessions, so it is
    /// [`UnknownPeer`](HostError::UnknownPeer).
    pub fn session_wire(&self, peer: PeerHandle) -> Result<u8, HostError> {
        let session = self.sessions.get(&peer).ok_or(HostError::UnknownPeer)?;
        Ok(if session.v3.is_some() {
            3
        } else {
            self.config.protocol.as_u8()
        })
    }

    /// Replaces a v3 session's fact grants (C-4) and publishes the change to
    /// that session alone (D-11). Precedence: [`Closed`](HostError::Closed),
    /// [`UnknownPeer`](HostError::UnknownPeer),
    /// [`NotV3Peer`](HostError::NotV3Peer), then
    /// [`InvalidGrants`](HostError::InvalidGrants).
    pub fn set_fact_grants(
        &mut self,
        peer: PeerHandle,
        facts: FactGrants,
    ) -> Result<(), HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        let session = self.sessions.get_mut(&peer).ok_or(HostError::UnknownPeer)?;
        let v3 = session.v3.as_mut().ok_or(HostError::NotV3Peer)?;
        v3.fact_grants = normalize_fact_grants(facts)?;
        self.publish_v3(Some(peer), &[]);
        Ok(())
    }

    /// Replaces a v3 session's own pause rights (C-4) and publishes them to
    /// that session alone (D-11); same precedence as
    /// [`set_fact_grants`](Self::set_fact_grants) minus the grant check.
    pub fn set_pause_rights(
        &mut self,
        peer: PeerHandle,
        rights: PauseRights,
    ) -> Result<(), HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        let session = self.sessions.get_mut(&peer).ok_or(HostError::UnknownPeer)?;
        session.v3.as_mut().ok_or(HostError::NotV3Peer)?.rights = rights;
        self.publish_v3(Some(peer), &[]);
        Ok(())
    }

    /// A v3 session's own pause rights. Not checked against shutdown (a
    /// closed host has no sessions): [`UnknownPeer`](HostError::UnknownPeer),
    /// then [`NotV3Peer`](HostError::NotV3Peer).
    pub fn pause_rights(&self, peer: PeerHandle) -> Result<PauseRights, HostError> {
        let session = self.sessions.get(&peer).ok_or(HostError::UnknownPeer)?;
        Ok(session.v3.as_ref().ok_or(HostError::NotV3Peer)?.rights)
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
        if self
            .sessions
            .get(&peer)
            .ok_or(HostError::UnknownPeer)?
            .v3
            .is_some()
        {
            return self.set_v3_grants(peer, grants);
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

    /// [`set_grants`](Self::set_grants) on a v3 session (C-8): the same
    /// precedence, minus the v2 id map, which a v3 session does not keep. A
    /// narrowing stages a fresh `0x82` snapshot (`Aborted`) under the new
    /// grants and changes nothing if the staging fails; anything else stores
    /// the grants, publishes the widening to that session (D-11) and returns
    /// `Unchanged`.
    fn set_v3_grants(
        &mut self,
        peer: PeerHandle,
        grants: DisclosureGrants,
    ) -> Result<GrantUpdate, HostError> {
        let grants = normalize_grants(grants)?;
        let session = self.sessions.get(&peer).ok_or(HostError::UnknownPeer)?;
        if !is_narrowing(&session.grants, &grants) {
            self.sessions.get_mut(&peer).expect("checked above").grants = grants;
            self.publish_v3(Some(peer), &[]);
            return Ok(GrantUpdate::Unchanged);
        }
        let stage = self.plan_v3_stage(session, &grants, SnapshotCause::Narrowed)?;
        let session = self.sessions.get_mut(&peer).expect("checked above");
        commit_v3_stage(
            &mut self.egress_bytes,
            &self.egress_caps,
            session,
            stage,
            Some(grants),
        );
        Ok(GrantUpdate::SnapshotStaged)
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
        // The `order` fact follows control (Reading 9): a v3 session is told.
        self.publish_v3(Some(peer), &[]);
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
                    let seq = self
                        .decode(staged.peer, &staged.bytes)
                        .ok()
                        .map(|frame| frame.seq);
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

    /// Decodes with the session's codec: v3 for a session bound with
    /// `bind_v3_peer`, else the host-wide v1/v2 (T141d D-1). A peer with no
    /// session decodes with the host-wide codec (and is then `SessionExpired`).
    fn decode(&self, peer: PeerHandle, bytes: &[u8]) -> Result<Frame, CodecError> {
        if self
            .sessions
            .get(&peer)
            .is_some_and(|session| session.v3.is_some())
        {
            let frame = codec_v3::decode_intent(bytes)?;
            return Ok(Frame {
                epoch: frame.epoch,
                seq: frame.seq,
                observed_tick: frame.observed_tick,
                actor: frame.actor,
                body: frame.body,
            });
        }
        let frame = match self.config.protocol {
            ProtocolSelection::V1 => codec::decode_intent(bytes),
            ProtocolSelection::V2 => codec_v2::decode_intent(bytes),
        }?;
        Ok(Frame {
            epoch: frame.epoch,
            seq: frame.seq,
            observed_tick: frame.observed_tick,
            actor: frame.actor,
            body: match frame.body {
                IntentBody::DeclareAction { ability, target } => {
                    IntentBodyV3::DeclareAction { ability, target }
                }
                IntentBody::EndTurn => IntentBodyV3::EndTurn,
            },
        })
    }

    /// The canonical bytes of `frame` in its session's codec (D-1); the raw
    /// bytes when the frame cannot be re-encoded.
    fn canonical(&self, v3: bool, frame: &Frame, raw: &[u8]) -> Vec<u8> {
        let encoded = if v3 {
            codec_v3::encode_intent(&IntentFrameV3 {
                epoch: frame.epoch,
                lane: LANE_COMBAT,
                seq: frame.seq,
                observed_tick: frame.observed_tick,
                actor: frame.actor,
                body: frame.body,
            })
        } else {
            let body = match frame.body {
                IntentBodyV3::DeclareAction { ability, target } => {
                    IntentBody::DeclareAction { ability, target }
                }
                IntentBodyV3::EndTurn => IntentBody::EndTurn,
                _ => return raw.to_vec(),
            };
            let legacy = IntentFrame {
                epoch: frame.epoch,
                lane: LANE_COMBAT,
                seq: frame.seq,
                observed_tick: frame.observed_tick,
                actor: frame.actor,
                body,
            };
            match self.config.protocol {
                ProtocolSelection::V1 => codec::encode_intent(&legacy),
                ProtocolSelection::V2 => codec_v2::encode_intent(&legacy),
            }
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
        // 1. Decode with the session's codec; identity code map.
        let frame = match self.decode(peer, &staged.bytes) {
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
        let v3 = session.v3.is_some();
        // 3. Seq/cache, cache-first.
        let canonical = self.canonical(v3, &frame, &staged.bytes);
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
        let mut terminal: Option<(CodeV3, Option<CombatError>)> = None;
        if frame.observed_tick > server_tick {
            terminal = Some((RejectionCode::FutureTick.into(), None));
        } else if frame.observed_tick < server_tick.saturating_sub(POLICY_OBSERVED_TICK_WINDOW) {
            terminal = Some((RejectionCode::StaleTick.into(), None));
        }
        // 5. Mapping / ownership / disclosure-gated identity (D-2).
        let mut resolved = None;
        if terminal.is_none() {
            let found = match &session.v3 {
                Some(v3_session) => resolve_v3(self.history.world(), session, v3_session, &frame),
                None => resolve_action(session, &frame).ok_or(CodeV3::NotAuthorized),
            };
            match found {
                Ok(found) => resolved = Some(found),
                Err(code) => terminal = Some((code, None)),
            }
        }
        // Gameplay class through sim's single legality source (no copy).
        if let Some(Resolved {
            intent: Intent::Action(action),
            ..
        }) = &resolved
        {
            if let Err(error) = validate_action(self.history.world(), action) {
                terminal = Some((refusal_code(v3, &error), Some(error)));
            }
        }
        // 6. Reservation, then 7–8 execution and commit.
        match (terminal, resolved) {
            (Some((code, error)), _) => {
                self.reject(peer, &frame, canonical, code, error, now_ms, !v3)
            }
            (None, Some(resolved)) => {
                if !self.accepted_room()? {
                    return Ok(Step::Blocked);
                }
                let before = self.history.last_sequence();
                // The involved entities of a request that is no sim action are
                // the requester and the head, as for `Stop`.
                let evidence = match resolved.intent {
                    Intent::Action(action) => action,
                    _ => CombatAction::Stop {
                        actor: resolved.actor,
                    },
                };
                let outcome = match resolved.intent {
                    Intent::Action(action) => match self.history.perform_action(&action) {
                        Ok(outcome) => outcome,
                        // Unreachable after validate_action; committed under
                        // the accepted reservation, which is a superset.
                        Err(HistoryError::Combat(error)) => {
                            let code = refusal_code(v3, &error);
                            return self.reject(
                                peer,
                                &frame,
                                canonical,
                                code,
                                Some(error),
                                now_ms,
                                false,
                            );
                        }
                        // Sim capacity failures commit nothing: retryable.
                        Err(_) => return Ok(Step::Blocked),
                    },
                    // A mode request reserves the accepted class; the sim's
                    // refusal commits a rejection under that reservation (D-3).
                    Intent::Mode(mode) => match self.history.request_mode(mode) {
                        Ok(()) => None,
                        Err(HistoryError::Combat(error)) => {
                            let code = refusal_code(v3, &error);
                            return self.reject(
                                peer,
                                &frame,
                                canonical,
                                code,
                                Some(error),
                                now_ms,
                                false,
                            );
                        }
                        Err(_) => return Ok(Step::Blocked),
                    },
                    // Pause and unpause cannot fail (D-5).
                    Intent::Pause => {
                        if self.paused.is_none() {
                            self.set_paused(PauseReason::Player);
                        }
                        None
                    }
                    Intent::Unpause => {
                        if self.paused.take().is_some() {
                            self.resume_timer();
                        }
                        None
                    }
                };
                self.commit_accepted(peer, &frame, canonical, &evidence, outcome, before)?;
                Ok(removed(seq, ReceiptStatus::Applied, false))
            }
            (None, None) => Err(HostError::Invariant("admission without action or terminal")),
        }
    }

    /// Commits a terminal rejection and builds its result. A v1/v2 session's
    /// receipt must fit its egress room (`block`, else the head stays staged);
    /// a v3 session never blocks: its receipt is logged when it fits and
    /// dropped otherwise, and the client's retry recovers it (Reading 12).
    #[allow(clippy::too_many_arguments)]
    fn reject(
        &mut self,
        peer: PeerHandle,
        frame: &Frame,
        canonical: Vec<u8>,
        code: CodeV3,
        error: Option<CombatError>,
        now_ms: u64,
        block: bool,
    ) -> Result<Step, HostError> {
        let server_tick = self.server_tick();
        let status = ReceiptV3::Rejected(code);
        let session = self.sessions.get(&peer).expect("bound by caller");
        let receipt = self.receipt_frame(session, frame.seq, server_tick, status)?;
        let fits = self.room(session, std::slice::from_ref(&receipt));
        let receipt = if session.v3.is_some() {
            // The throttle decides too (D-6, E-2): a receipt is one delta
            // frame, so it counts toward the in-flight allowance.
            let gate = receipt_gate(session, server_tick, receipt.bytes.len());
            if gate == Gate::Over {
                self.close_throttle(peer);
            }
            (fits && gate == Gate::Open).then_some(receipt)
        } else if block && !session.resync_pending && !fits {
            return Ok(Step::Blocked);
        } else {
            Some(receipt)
        };
        if let Some(error) = error {
            self.last_combat_error = Some(error);
        }
        let dropped =
            self.commit_rejection(peer, frame, canonical, status, server_tick, receipt, now_ms)?;
        Ok(Step::Removed {
            result: AdmissionResult {
                peer,
                seq: Some(frame.seq),
                status: trusted_status(status),
                cached: false,
                retained_in_ingress: false,
            },
            dropped,
        })
    }

    /// Cache hit: return the retained outcome without re-execution and make
    /// sure the receipt is deliverable.
    fn cache_hit(&mut self, peer: PeerHandle, seq: u64, now_ms: u64) -> Result<Step, HostError> {
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
            let frame = self.receipt_frame(session, seq, entry.processed_tick, status)?;
            // Best effort under the egress caps: a cache hit reserves nothing,
            // and the sender's next identical retry recovers the receipt.
            // A v3 session's receipt is also a throttled delta frame (E-2).
            let gate = receipt_gate(session, self.server_tick(), frame.bytes.len());
            if gate == Gate::Over {
                self.close_throttle(peer);
            }
            let session = self.sessions.get(&peer).expect("bound by caller");
            if gate == Gate::Open && self.room(session, std::slice::from_ref(&frame)) {
                let tick = self.server_tick();
                let session = self.sessions.get_mut(&peer).expect("bound by caller");
                let spend = match status {
                    ReceiptV3::Applied => true,
                    ReceiptV3::Rejected(_) => {
                        try_consume_failure(session, now_ms, frame.bytes.len())
                    }
                };
                if spend {
                    let generation = session.generation;
                    let ordinal = session.next_ordinal;
                    let len = frame.bytes.len();
                    push_frames(&mut self.egress_bytes, session, vec![frame])?;
                    record_throttle(session, tick, len);
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
                status: trusted_status(status),
                cached: true,
                retained_in_ingress: false,
            },
            dropped,
        })
    }

    /// Builds the originator's receipt-only frame at its next delivery seq,
    /// in the session's wire (v3 sessions carry the exact code).
    fn receipt_frame(
        &self,
        session: &Session,
        seq: u64,
        processed_tick: u64,
        status: ReceiptV3,
    ) -> Result<LoggedFrame, HostError> {
        let bytes = if session.v3.is_some() {
            codec_v3::encode_delta(&DeltaFrameV3 {
                lane: LANE_COMBAT,
                server_tick: self.server_tick(),
                event_seq: session.next_event_seq,
                ops: vec![receipt_op_v3(session.epoch, seq, processed_tick, status)],
            })
            .map_err(|_| HostError::Invariant(V3_ENCODE))?
        } else {
            let ops = vec![receipt_op(
                session.epoch,
                seq,
                processed_tick,
                trusted_status(status),
            )];
            encode_frame(
                self.config.protocol,
                self.server_tick(),
                session.next_event_seq,
                &ops,
            )?
        };
        Ok(LoggedFrame {
            ordinal: 0,
            event_seq: Some(session.next_event_seq),
            bytes,
        })
    }

    /// Commits a terminal rejection: sequence, cache, receipt subject to the
    /// failure budgets. `receipt` is `None` when a v3 session had no room for
    /// it. Returns whether the reply was dropped by a budget.
    #[allow(clippy::too_many_arguments)]
    fn commit_rejection(
        &mut self,
        peer: PeerHandle,
        frame: &Frame,
        canonical: Vec<u8>,
        status: ReceiptV3,
        processed_tick: u64,
        receipt: Option<LoggedFrame>,
        now_ms: u64,
    ) -> Result<bool, HostError> {
        let v3 = self
            .sessions
            .get(&peer)
            .is_some_and(|session| session.v3.is_some());
        let template =
            self.receipt_template_len(v3, frame.epoch, frame.seq, processed_tick, status)?;
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
        let dropped = match receipt {
            _ if session.resync_pending => false,
            None => false,
            Some(receipt) => {
                if try_consume_failure(session, now_ms, receipt.bytes.len()) {
                    entry.receipt = Some((session.generation, session.next_ordinal));
                    let len = receipt.bytes.len();
                    push_frames(&mut self.egress_bytes, session, vec![receipt])?;
                    record_throttle(session, processed_tick, len);
                    false
                } else {
                    true
                }
            }
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
        frame: &impl IntentHeader,
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
    ///
    /// Every v3 session's page is decided (read-only) before the record is
    /// built, so a fenced session contributes no view; nothing is delivered or
    /// appended unless the record is built (as for v1/v2 deliveries).
    #[allow(clippy::too_many_arguments)]
    fn capture_accepted(
        &mut self,
        peer: PeerHandle,
        frame: &impl IntentHeader,
        canonical: Vec<u8>,
        action: &CombatAction,
        outcome: Option<crpg_sim::ActionOutcome>,
        before: u64,
        after: u64,
    ) -> Result<(), HostError> {
        let (epoch, seq) = (frame.header_epoch(), frame.header_seq());
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
            // A v3 session never gets v2 frames; its page is decided below.
            if session.resync_pending || session.v3.is_some() {
                continue;
            }
            let view = session.view();
            let mut ops = Vec::new();
            if *handle == peer {
                ops.push(receipt_op(
                    session.epoch,
                    seq,
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
        // v3 sessions (D-6 to D-10): each page is decided without changing
        // anything; a session that gets at least one frame adds its view.
        let decisions =
            self.decide_v3_pages(None, &events, Some((peer, seq)), Some(&involved), false);
        for (_, decision) in &decisions {
            if let V3Decision::Deliver(plan) = decision {
                if let (false, Some(view)) = (plan.frames.is_empty(), &plan.capture) {
                    views.push(view.clone());
                }
            }
        }
        views.sort_by_key(|view| view.epoch);
        let record = CapturedRecord {
            capture_seq: self.last_capture + 1,
            epoch,
            lane: LANE_COMBAT,
            seq,
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
        let v3_originator = self
            .sessions
            .get(&peer)
            .is_some_and(|session| session.v3.is_some());
        let template = self.receipt_template_len(
            v3_originator,
            epoch,
            seq,
            processed_tick,
            ReceiptV3::Applied,
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
        for (handle, decision) in decisions {
            let outcome = self.commit_v3(handle, decision);
            if handle == peer {
                receipt_ref = match outcome {
                    V3Outcome::Logged { receipt } => receipt,
                    V3Outcome::Staged | V3Outcome::Fenced => None,
                };
            }
        }
        // A fenced originator has no session to update (A-1): its staged
        // commands later fail `SessionExpired`.
        if let Some(session) = self.sessions.get_mut(&peer) {
            session.next_new_seq = seq + 1;
            cache_insert(
                session,
                CacheEntry {
                    seq,
                    bytes: canonical.len() + template,
                    canonical,
                    status: ReceiptV3::Applied,
                    processed_tick,
                    receipt: receipt_ref,
                },
            );
        }
        Ok(())
    }

    /// The retry-cache receipt-only template length (event_seq = 0), in the
    /// session wire's codec (D-1).
    fn receipt_template_len(
        &self,
        v3: bool,
        epoch: [u8; 16],
        seq: u64,
        processed_tick: u64,
        status: ReceiptV3,
    ) -> Result<usize, HostError> {
        if v3 {
            return Ok(codec_v3::encode_delta(&DeltaFrameV3 {
                lane: LANE_COMBAT,
                server_tick: processed_tick,
                event_seq: 0,
                ops: vec![receipt_op_v3(epoch, seq, processed_tick, status)],
            })
            .map_err(|_| HostError::Invariant(V3_ENCODE))?
            .len());
        }
        let ops = vec![receipt_op(
            epoch,
            seq,
            processed_tick,
            trusted_status(status),
        )];
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
            if session.resync_pending || session.v3.is_some() {
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
        let bytes: usize = frames.iter().map(|frame| frame.bytes.len()).sum();
        self.room_for(session, frames.len(), bytes)
    }

    /// Check-only egress room for `count` frames of `bytes` encoded bytes.
    fn room_for(&self, session: &Session, count: usize, bytes: usize) -> bool {
        let caps = self.egress_caps;
        session.log.len() + count <= caps.per_peer_frames
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

    /// The host's values for a viewer with `rights` (C-6): the remaining
    /// timer clamped to a `u32`.
    fn host_input(&self, rights: PauseRights) -> HostInput {
        HostInput {
            paused: self.paused,
            rights,
            auto_pause: self.settings.auto_pause,
            turn_timer_ms: self
                .turn_timer_remaining_ms()
                .map(|ms| u32::try_from(ms).unwrap_or(u32::MAX)),
        }
    }

    /// C-5 steps 1 to 4 for a v3 session: reads only, so a failure changes
    /// nothing. `grants` are the disclosure grants the snapshot is cut under
    /// (the session's own, or the narrower new ones).
    fn plan_v3_stage(
        &self,
        session: &Session,
        grants: &DisclosureGrants,
        cause: SnapshotCause,
    ) -> Result<V3Stage, HostError> {
        let v3 = session.v3.as_ref().ok_or(HostError::NotV3Peer)?;
        // 1. Transfer-id counter sentinel.
        if session.next_transfer == u64::MAX {
            return Err(HostError::CounterExhausted);
        }
        let transfer = session.next_transfer;
        let epoch = session.epoch;
        let tick = self.server_tick();
        // 2. Cancel the in-flight transfer first, if any.
        let cancel = match session.in_flight {
            Some(old) => Some(
                transfer_v3::encode_down(&TransferDown::Cancel {
                    epoch,
                    transfer: TransferId::new(old).expect("in-flight id is nonzero"),
                    reason: cause.cancel_reason(),
                })
                .map_err(|_| HostError::Invariant(V3_ENCODE))?,
            ),
            None => None,
        };
        // 3. The snapshot, projected on clones of the two views.
        let read = read_world(
            self.history.world(),
            tick,
            grants,
            &v3.fact_grants,
            &session.control,
        );
        let mut view = v3.view.clone();
        let mut facts = v3.facts.clone();
        let projection = project_snapshot_v3(
            &mut view,
            &mut facts,
            &read.viewer(epoch),
            &read.fact_context(),
        )
        .map_err(|_| HostError::Invariant("v3 snapshot projection failed"))?;
        // 4. The canonical plan, then C-6's host-facts delta.
        let frames = transfer_v3::plan_transfer(
            epoch,
            TransferId::new(transfer).expect("nonzero by the step-1 check"),
            tick,
            session.last_event_seq(),
            SNAPSHOT_SEGMENTS_PER_CHUNK,
            &projection.body,
        )
        .map_err(|_| HostError::Invariant(V3_ENCODE))?;
        let input = self.host_input(v3.rights);
        let clock = clock_op(&input, v3.fact_grants.host);
        let delta_seq = session.next_event_seq;
        let delta = codec_v3::encode_delta(&DeltaFrameV3 {
            lane: LANE_COMBAT,
            server_tick: tick,
            event_seq: delta_seq,
            ops: vec![clock.clone(), host_facts_op(&input, v3.fact_grants.host)],
        })
        .map_err(|_| HostError::Invariant(V3_ENCODE))?;
        let count = frames.len() + usize::from(cancel.is_some()) + 1;
        let bytes = frames.iter().map(Vec::len).sum::<usize>()
            + cancel.as_ref().map_or(0, Vec::len)
            + delta.len();
        let ordinal_room = (u64::MAX - 1).saturating_sub(session.next_ordinal);
        if count as u64 > ordinal_room || delta_seq == u64::MAX {
            return Err(HostError::CounterExhausted);
        }
        if !outbox_fits(count, bytes) {
            return Err(HostError::Invariant("v3 plan exceeds the outbox bound"));
        }
        Ok(V3Stage {
            view,
            facts,
            cancel,
            frames,
            delta,
            delta_seq,
            transfer,
            tick,
            clock,
            image: host_facts_image(
                v3.rights,
                self.settings.auto_pause,
                v3.fact_grants.host,
                self.timer_epoch(),
            ),
        })
    }

    /// Ends a v3 session's deltas until its transfer ends (E-2): a frame the
    /// held-delta budget refused.
    fn close_throttle(&mut self, peer: PeerHandle) {
        if let Some(throttle) = self
            .sessions
            .get_mut(&peer)
            .and_then(|session| session.v3.as_mut())
            .and_then(|v3| v3.throttle.as_mut())
        {
            throttle.close();
        }
    }

    /// Moves `peer`'s outbox into its delivery log while the egress caps
    /// allow (C-7).
    fn pace(&mut self, peer: PeerHandle) {
        if let Some(session) = self.sessions.get_mut(&peer) {
            pace_session(&mut self.egress_bytes, &self.egress_caps, session);
        }
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

    // -- v3 publication (T141d) ----------------------------------------------

    /// One v3 session's page for one operation (D-7), built on clones of its
    /// two views and so reading only: `[receipt?] ++ enters ++ events ++ rest
    /// of the page ++ [Clock?] ++ [HostFacts?]`, cut by `frame_spans` (H-6),
    /// one `event_seq` per span. `involved` (an accepted peer command's
    /// entities) asks for the session's capture view (D-10). `heartbeat` marks
    /// a running host's tick publication, which adds a due `Clock` (E-1).
    fn plan_v3_page(
        &self,
        session: &Session,
        events: &[HistoryEnvelope],
        receipt: Option<DeltaOpV3>,
        involved: Option<&BTreeSet<EntityId>>,
        heartbeat: bool,
    ) -> Result<PagePlan, HostError> {
        const PAGE: &str = "v3 page projection failed";
        let v3 = session.v3.as_ref().ok_or(HostError::NotV3Peer)?;
        let world = self.history.world();
        let tick = self.server_tick();
        let epoch = session.epoch;
        // (1) The contexts, with the `Despawned` events granted present and
        // despawn as the removals this page may read as `Despawned`.
        let mut despawned: Vec<EntityKey> = events
            .iter()
            .filter_map(|envelope| match &envelope.payload {
                HistoryEvent::Despawned { entity }
                    if lookup(&session.grants, *entity)
                        .is_some_and(|grant| grant.present && grant.despawn) =>
                {
                    Some(key_of(world, *entity))
                }
                _ => None,
            })
            .collect();
        despawned.sort();
        despawned.dedup();
        let read = read_world(
            world,
            tick,
            &session.grants,
            &v3.fact_grants,
            &session.control,
        )
        .with_despawned(despawned);
        // (2) The page, on clones of the two views.
        let mut view = v3.view.clone();
        let mut facts = v3.facts.clone();
        let page = project_page(
            &mut view,
            &mut facts,
            &read.viewer(epoch),
            &read.fact_context(),
        )
        .map_err(|_| HostError::Invariant(PAGE))?;
        // (3) The event ops: ids resolve against the ids live after the page
        // or, failing that, before it (D-8).
        let ids = id_map(world, &session.grants, &view, Some(&v3.view));
        let peer_view = PeerView {
            grants: &session.grants,
            map: &ids,
        };
        let candidates: Vec<EventCandidateV3> = events
            .iter()
            .filter_map(|envelope| {
                peer_view.candidate_v3(
                    &session.control,
                    v3.fact_grants.encounter.mode,
                    &envelope.payload,
                )
            })
            .collect();
        let mut event_ops: Vec<DeltaOpV3> = Vec::new();
        for chunk in candidates.chunks(MAX_DELTA_OPS) {
            event_ops.extend(project_events_v3(chunk).map_err(|_| HostError::Invariant(PAGE))?);
        }
        // (4) The host facts, current after auto-pause and the timer (D-12).
        let input = self.host_input(v3.rights);
        let clock = clock_op(&input, v3.fact_grants.host);
        let image = host_facts_image(
            v3.rights,
            self.settings.auto_pause,
            v3.fact_grants.host,
            self.timer_epoch(),
        );
        let heartbeat_due =
            heartbeat && tick.saturating_sub(v3.last_clock_tick) >= POLICY_CLOCK_HEARTBEAT_TICKS;
        let send_clock = heartbeat_due || v3.sent_clock.as_ref() != Some(&clock);
        let send_facts = v3.sent_host_facts != Some(image);
        let mut ops: Vec<DeltaOpV3> = Vec::new();
        ops.extend(receipt);
        ops.extend_from_slice(&page.ops[..page.enters]);
        ops.extend(event_ops);
        ops.extend_from_slice(&page.ops[page.enters..]);
        if send_clock {
            ops.push(clock.clone());
        }
        if send_facts {
            ops.push(host_facts_op(&input, v3.fact_grants.host));
        }
        // (5, 6) No ops: no frame. Else one frame per span.
        let mut frames: Vec<(u64, Vec<u8>)> = Vec::new();
        if !ops.is_empty() {
            let spans = codec_v3::frame_spans(&ops).map_err(|_| HostError::Invariant(V3_ENCODE))?;
            let mut event_seq = session.next_event_seq;
            for span in spans {
                let bytes = codec_v3::encode_delta(&DeltaFrameV3 {
                    lane: LANE_COMBAT,
                    server_tick: tick,
                    event_seq,
                    ops: ops[span].to_vec(),
                })
                .map_err(|_| HostError::Invariant(V3_ENCODE))?;
                frames.push((event_seq, bytes));
                event_seq = event_seq
                    .checked_add(1)
                    .ok_or(HostError::CounterExhausted)?;
            }
        }
        let capture = involved.filter(|_| !frames.is_empty()).map(|involved| {
            let after_ids = id_map(world, &session.grants, &view, None);
            let after = PeerView {
                grants: &session.grants,
                map: &after_ids,
            };
            CapturedView {
                epoch,
                health: after.health_entries(world, involved),
                turn: after.turn(world),
            }
        });
        Ok(PagePlan {
            view,
            facts,
            frames,
            clock: send_clock.then_some(clock),
            image: send_facts.then_some(image),
            tick,
            capture,
        })
    }

    /// What publication does for one v3 session (D-9), reading only: the page
    /// if its frames fit the egress room, else a restage, and a fence if
    /// either plan cannot be built. While the session's transfer is in flight
    /// the throttle decides first (E-2): too soon is [`V3Decision::Hold`]
    /// with nothing planned; an allowed page carries no event ops and no
    /// receipt, and one the held-delta budget refuses closes the session's
    /// deltas.
    fn decide_v3(
        &self,
        session: &Session,
        events: &[HistoryEnvelope],
        receipt: Option<DeltaOpV3>,
        involved: Option<&BTreeSet<EntityId>>,
        heartbeat: bool,
    ) -> V3Decision {
        let tick = self.server_tick();
        let throttle = session
            .v3
            .as_ref()
            .and_then(|v3| v3.throttle.as_ref())
            .filter(|_| session.in_flight.is_some());
        if let Some(throttle) = throttle {
            if throttle.gate(tick, 0, 0) != Gate::Open {
                return V3Decision::Hold { close: false };
            }
        }
        let (events, receipt) = if throttle.is_some() {
            (&[][..], None)
        } else {
            (events, receipt)
        };
        let Ok(plan) = self.plan_v3_page(session, events, receipt, involved, heartbeat) else {
            return V3Decision::Fence;
        };
        let bytes: usize = plan.frames.iter().map(|(_, bytes)| bytes.len()).sum();
        if let Some(throttle) = throttle {
            if throttle.gate(tick, plan.frames.len(), bytes) != Gate::Open {
                return V3Decision::Hold { close: true };
            }
        }
        if self.room_for(session, plan.frames.len(), bytes) {
            return V3Decision::Deliver(plan);
        }
        match self.plan_v3_stage(session, &session.grants, SnapshotCause::Resync) {
            Ok(stage) => V3Decision::Restage(stage),
            Err(_) => V3Decision::Fence,
        }
    }

    /// The decisions for every v3 session (`only` limits it to one), ascending
    /// handle. `origin` names the accepted command's originator, whose page
    /// leads with the `Applied` receipt.
    fn decide_v3_pages(
        &self,
        only: Option<PeerHandle>,
        events: &[HistoryEnvelope],
        origin: Option<(PeerHandle, u64)>,
        involved: Option<&BTreeSet<EntityId>>,
        heartbeat: bool,
    ) -> Vec<(PeerHandle, V3Decision)> {
        let tick = self.server_tick();
        self.sessions
            .iter()
            .filter(|(handle, session)| {
                session.v3.is_some() && only.is_none_or(|one| one == **handle)
            })
            .map(|(handle, session)| {
                let receipt = origin
                    .filter(|(originator, _)| originator == handle)
                    .map(|(_, seq)| receipt_op_v3(session.epoch, seq, tick, ReceiptV3::Applied));
                (
                    *handle,
                    self.decide_v3(session, events, receipt, involved, heartbeat),
                )
            })
            .collect()
    }

    /// Commits one decision: logs the page and installs its clones, stages the
    /// replacement snapshot, or fences the session (D-9).
    fn commit_v3(&mut self, handle: PeerHandle, decision: V3Decision) -> V3Outcome {
        match decision {
            V3Decision::Fence => {
                self.fence(handle);
                V3Outcome::Fenced
            }
            V3Decision::Hold { close } => {
                if close {
                    if let Some(throttle) = self
                        .sessions
                        .get_mut(&handle)
                        .and_then(|session| session.v3.as_mut())
                        .and_then(|v3| v3.throttle.as_mut())
                    {
                        throttle.close();
                    }
                }
                V3Outcome::Logged { receipt: None }
            }
            V3Decision::Restage(stage) => {
                let Some(session) = self.sessions.get_mut(&handle) else {
                    return V3Outcome::Fenced;
                };
                commit_v3_stage(
                    &mut self.egress_bytes,
                    &self.egress_caps,
                    session,
                    stage,
                    None,
                );
                V3Outcome::Staged
            }
            V3Decision::Deliver(plan) => {
                let Some(session) = self.sessions.get_mut(&handle) else {
                    return V3Outcome::Fenced;
                };
                let generation = session.generation;
                let first = session.next_ordinal;
                let frames: Vec<LoggedFrame> = plan
                    .frames
                    .into_iter()
                    .map(|(event_seq, bytes)| LoggedFrame {
                        ordinal: 0,
                        event_seq: Some(event_seq),
                        bytes,
                    })
                    .collect();
                let logged = !frames.is_empty();
                let logged_bytes: usize = frames.iter().map(|frame| frame.bytes.len()).sum();
                let logged_frames = frames.len();
                if push_frames(&mut self.egress_bytes, session, frames).is_err() {
                    self.fence(handle);
                    return V3Outcome::Fenced;
                }
                let v3 = session
                    .v3
                    .as_mut()
                    .expect("a v3 decision needs a v3 session");
                v3.view = plan.view;
                v3.facts = plan.facts;
                if let Some(throttle) = v3.throttle.as_mut() {
                    throttle.record(plan.tick, logged_frames, logged_bytes);
                }
                if let Some(clock) = plan.clock {
                    v3.sent_clock = Some(clock);
                    v3.last_clock_tick = plan.tick;
                }
                if let Some(image) = plan.image {
                    v3.sent_host_facts = Some(image);
                }
                V3Outcome::Logged {
                    receipt: logged.then_some((generation, first)),
                }
            }
        }
    }

    /// Publishes one operation to v3 sessions (`only` one, else every one,
    /// ascending by handle). A failure fences that session and never fails
    /// the operation, which has already committed (D-9).
    fn publish_v3(&mut self, only: Option<PeerHandle>, events: &[HistoryEnvelope]) {
        self.publish_v3_with(only, events, false);
    }

    /// [`publish_v3`](Self::publish_v3) for one operation; `heartbeat` marks
    /// a tick of a running host, whose page carries a due `Clock` (E-1).
    fn publish_v3_with(
        &mut self,
        only: Option<PeerHandle>,
        events: &[HistoryEnvelope],
        heartbeat: bool,
    ) {
        let decisions = self.decide_v3_pages(only, events, None, None, heartbeat);
        for (handle, decision) in decisions {
            self.commit_v3(handle, decision);
        }
    }

    /// Whether `op`'s publication is a running host's tick (E-1): only a tick
    /// of an unpaused host sends the heartbeat.
    fn heartbeat_for(&self, op: HostOp) -> bool {
        op == HostOp::Tick && self.paused.is_none()
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
        self.pace(peer);
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
        self.pace(peer);
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
        // The freed room lets a v3 session's outbox move on (C-7).
        pace_session(&mut self.egress_bytes, &self.egress_caps, session);
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
            ResyncV3,
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
            // A v3 session speaks `0x82` only (C-9); a `0x81` frame is
            // malformed there, as a `0x82` frame is on a v2 snapshot session.
            let decoded = if session.v3.is_some() {
                transfer_v3::decode_up(bytes)
            } else {
                codec_transfer::decode_up(bytes)
            };
            let message = match decoded {
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
                        if let Some(v3) = session.v3.as_mut() {
                            v3.throttle = None;
                        }
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
                    } else if session.v3.is_some() {
                        Effect::ResyncV3
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
            Effect::ResyncV3 => {
                let session = self.sessions.get(&peer).ok_or(HostError::UnknownPeer)?;
                let stage = self.plan_v3_stage(session, &session.grants, SnapshotCause::Resync)?;
                let session = self.sessions.get_mut(&peer).expect("checked above");
                commit_v3_stage(
                    &mut self.egress_bytes,
                    &self.egress_caps,
                    session,
                    stage,
                    None,
                );
                Ok(TransferDisposition::Resynced)
            }
        }
    }

    /// The retained retry-cache status for `seq`, if still cached. It keeps
    /// the frozen v1 type: a v3 code `21..=29` reads `IllegalAction` and `30`
    /// reads `NotAuthorized` (T141 Reading 17); see
    /// [`cached_status_v3`](Self::cached_status_v3) for the exact code.
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
            .map(|entry| trusted_status(entry.status)))
    }

    /// The retained retry-cache status for `seq` with its exact v3 code, if
    /// still cached (T141d D-4). Precedence: [`UnknownPeer`](HostError::UnknownPeer),
    /// then [`NotV3Peer`](HostError::NotV3Peer) for a v1/v2 session.
    pub fn cached_status_v3(
        &self,
        peer: PeerHandle,
        seq: u64,
    ) -> Result<Option<ReceiptV3>, HostError> {
        let session = self.sessions.get(&peer).ok_or(HostError::UnknownPeer)?;
        if session.v3.is_none() {
            return Err(HostError::NotV3Peer);
        }
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
            // A zero-event operation makes no record but still publishes: an
            // RT order, a pending mode or a tick's clock changes v3 state.
            self.publish_v3_with(None, &[], self.heartbeat_for(op));
            return Ok(());
        }
        let record = self.read_host_record(op, before, after);
        self.finish_host_op(op, before, record)
    }

    /// The envelopes of `(before, after]`, read from the sim journal.
    fn read_events(&self, before: u64, after: u64) -> Result<Vec<HistoryEnvelope>, HostError> {
        let mut events: Vec<HistoryEnvelope> = Vec::with_capacity(
            usize::try_from(after - before)
                .unwrap_or(0)
                .min(MAX_HISTORY_EVENTS),
        );
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
        Ok(events)
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
        let events = self.read_events(before, after)?;
        build_host_record(
            self.last_capture + 1,
            op,
            before,
            after,
            self.server_tick(),
            events,
        )
    }

    /// Steps 3 to 5 of the tail (S-3): publication to v3 sessions (a failure
    /// fences that session only), the append, then the sim acknowledgement,
    /// which always runs. A `record` that could not be built is returned only
    /// after the acknowledgement, with nothing appended: evidence is then
    /// missing, so the next checkpoint fails to load, visibly. v3 sessions
    /// still get their page: the sim has committed.
    fn finish_host_op(
        &mut self,
        op: HostOp,
        before: u64,
        record: Result<HostRecord, HostError>,
    ) -> Result<(), HostError> {
        let after = self.history.last_sequence();
        let events = self.read_events(before, after).unwrap_or_default();
        self.publish_v3_with(None, &events, self.heartbeat_for(op));
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
        self.publish_v3(None, &[]);
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
        self.publish_v3(None, &[]);
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
        self.publish_v3(None, &[]);
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
    /// The host-facts image reads it (D-12), so that the countdown alone never
    /// re-sends `HostFacts`.
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

/// What the in-flight throttle says of one receipt frame of `bytes` bytes at
/// `tick`: [`Gate::Open`] when the session is not a v3 session or its transfer
/// is not in flight (E-2).
fn receipt_gate(session: &Session, tick: u64, bytes: usize) -> Gate {
    session
        .v3
        .as_ref()
        .and_then(|v3| v3.throttle.as_ref())
        .filter(|_| session.in_flight.is_some())
        .map_or(Gate::Open, |throttle| throttle.gate(tick, 1, bytes))
}

/// Counts one logged receipt frame of `bytes` bytes at `tick` toward a v3
/// session's in-flight allowance (E-2).
fn record_throttle(session: &mut Session, tick: u64, bytes: usize) {
    if let Some(throttle) = session.v3.as_mut().and_then(|v3| v3.throttle.as_mut()) {
        throttle.record(tick, 1, bytes);
    }
}

/// The `Invariant` text of a v3 frame that failed to encode.
const V3_ENCODE: &str = "v3 frame failed to encode";

/// C-5 step 5: installs a planned staging on a v3 session. It cannot fail.
/// `grants`, when given, are a narrowing's new disclosure grants and are
/// installed first. The log is discarded (and acknowledged, so the ordinals
/// stay gapless), the outbox becomes `[Cancel?, Begin, Chunk..., Delta]`, and
/// pacing (C-7) logs what the egress caps allow.
fn commit_v3_stage(
    egress_bytes: &mut usize,
    caps: &QueueCaps,
    session: &mut Session,
    stage: V3Stage,
    grants: Option<DisclosureGrants>,
) {
    if let Some(grants) = grants {
        session.grants = grants;
    }
    *egress_bytes -= session.log_bytes;
    session.log.clear();
    session.log_bytes = 0;
    session.delivered = session.next_ordinal - 1;
    session.in_flight = Some(stage.transfer);
    session.next_transfer += 1;
    session.next_event_seq = stage.delta_seq + 1;
    let v3 = session
        .v3
        .as_mut()
        .expect("a v3 staging needs a v3 session");
    v3.view = stage.view;
    v3.facts = stage.facts;
    v3.outbox.clear();
    let transfer_frames = stage.cancel.into_iter().chain(stage.frames);
    v3.outbox.extend(transfer_frames.map(|bytes| OutboxEntry {
        event_seq: None,
        bytes,
    }));
    let stage_delta_len = stage.delta.len();
    v3.outbox.push_back(OutboxEntry {
        event_seq: Some(stage.delta_seq),
        bytes: stage.delta,
    });
    v3.sent_clock = Some(stage.clock);
    v3.sent_host_facts = Some(stage.image);
    v3.last_clock_tick = stage.tick;
    // A staging restarts the throttle; C-6's frame counts toward the budget
    // (E-2).
    v3.throttle = Some(Throttle::new(stage.tick, 1, stage_delta_len));
    pace_session(egress_bytes, caps, session);
}

/// C-7: moves a v3 session's outbox entries, front first, into its delivery
/// log while each fits the egress caps, assigning ordinals in order. It stops
/// at the first entry that does not fit. A delta's `event_seq` was taken when
/// it was built, so nothing here advances `next_event_seq`.
fn pace_session(egress_bytes: &mut usize, caps: &QueueCaps, session: &mut Session) {
    let Some(v3) = session.v3.as_mut() else {
        return;
    };
    while let Some(front) = v3.outbox.front() {
        let len = front.bytes.len();
        let fits = session.log.len() < caps.per_peer_frames
            && session.log_bytes + len <= caps.per_peer_bytes
            && *egress_bytes + len <= caps.host_bytes;
        let Some(next) = session.next_ordinal.checked_add(1) else {
            break;
        };
        if !fits {
            break;
        }
        let entry = v3.outbox.pop_front().expect("checked front");
        session.log.push_back(LoggedFrame {
            ordinal: session.next_ordinal,
            event_seq: entry.event_seq,
            bytes: entry.bytes,
        });
        session.next_ordinal = next;
        session.log_bytes += len;
        *egress_bytes += len;
    }
}

/// A fact-grant vector within its bound, sorted by entity, with no entity
/// twice (C-1, C-4).
fn normalize_fact_grants(facts: FactGrants) -> Result<FactGrants, HostError> {
    if facts.entities.len() > MAX_GRANT_IDENTITIES {
        return Err(HostError::InvalidGrants);
    }
    let mut facts = facts;
    facts.entities.sort_by_key(|grant| grant.entity);
    if facts
        .entities
        .windows(2)
        .any(|pair| pair[0].entity == pair[1].entity)
    {
        return Err(HostError::InvalidGrants);
    }
    Ok(facts)
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
fn resolve_action(session: &Session, frame: &Frame) -> Option<Resolved> {
    let actor = *session.reverse.get(&frame.actor)?;
    let actor_grant = lookup(&session.grants, actor)?;
    if !(actor_grant.present && actor_grant.action_actor) {
        return None;
    }
    if session.control.binary_search(&actor).is_err() {
        return None;
    }
    let action = match frame.body {
        IntentBodyV3::DeclareAction { ability, target } => {
            let target = *session.reverse.get(&target)?;
            let target_grant = lookup(&session.grants, target)?;
            if !(target_grant.present && target_grant.action_target) {
                return None;
            }
            CombatAction::UseAbility {
                actor,
                ability,
                target,
            }
        }
        IntentBodyV3::EndTurn => CombatAction::EndTurn { actor },
        _ => return None,
    };
    Some(Resolved {
        actor,
        intent: Intent::Action(action),
    })
}

/// D-2: resolves a v3 intent through the session's `InterestView`: the actor
/// must be mapped, granted present and `action_actor`, and controlled, else
/// `NotAuthorized`; then the body (`Pause` and `Unpause` need their right,
/// else `NoPauseRight`; `RequestMode` needs `pause`, else `NotAuthorized`).
fn resolve_v3(
    world: &World,
    session: &Session,
    v3: &V3Session,
    frame: &Frame,
) -> Result<Resolved, CodeV3> {
    let deny = CodeV3::NotAuthorized;
    let area = world.area().unwrap_or(HOST_UNBOUND_AREA);
    let entity_of = |net: NetId| {
        v3.view
            .entity(net)
            .filter(|key| key.area == area)
            .map(|key| key.entity)
    };
    let actor = entity_of(frame.actor).ok_or(deny)?;
    let grant = lookup(&session.grants, actor).ok_or(deny)?;
    if !(grant.present && grant.action_actor) || session.control.binary_search(&actor).is_err() {
        return Err(deny);
    }
    let intent = match frame.body {
        IntentBodyV3::DeclareAction { ability, target } => {
            let target = entity_of(target).ok_or(deny)?;
            let grant = lookup(&session.grants, target).ok_or(deny)?;
            if !(grant.present && grant.action_target) {
                return Err(deny);
            }
            Intent::Action(CombatAction::UseAbility {
                actor,
                ability,
                target,
            })
        }
        IntentBodyV3::EndTurn => Intent::Action(CombatAction::EndTurn { actor }),
        IntentBodyV3::Stop => Intent::Action(CombatAction::Stop { actor }),
        IntentBodyV3::Pause if v3.rights.pause => Intent::Pause,
        IntentBodyV3::Unpause if v3.rights.unpause => Intent::Unpause,
        IntentBodyV3::Pause | IntentBodyV3::Unpause => return Err(CodeV3::NoPauseRight),
        IntentBodyV3::RequestMode { mode } if v3.rights.pause => Intent::Mode(sim_mode(mode)),
        IntentBodyV3::RequestMode { .. } => return Err(deny),
    };
    Ok(Resolved { actor, intent })
}

/// The ids a viewer's mapped entities hold: live after the page, else (when
/// `before` is given) live before it (D-8).
fn id_map(
    world: &World,
    grants: &DisclosureGrants,
    after: &InterestView,
    before: Option<&InterestView>,
) -> BTreeMap<EntityId, NetId> {
    grants
        .entities
        .iter()
        .filter_map(|grant| {
            let key = key_of(world, grant.entity);
            after
                .net_id(key)
                .or_else(|| before.and_then(|earlier| earlier.net_id(key)))
                .map(|net| (grant.entity, net))
        })
        .collect()
}

/// H-5: the v3 code of a sim refusal. The nine named variants take
/// `21..=29`; every other refusal is `IllegalAction`, as it is on v1/v2.
fn code(error: &CombatError) -> CodeV3 {
    match error {
        CombatError::ModeUnsupported => CodeV3::ModeUnsupported,
        CombatError::WrongMode { .. } => CodeV3::WrongMode,
        CombatError::StatEvaluation { .. } => CodeV3::StatEvaluation,
        CombatError::EncounterTerminal => CodeV3::EncounterTerminal,
        CombatError::RulesetMismatch { .. } => CodeV3::RulesetMismatch,
        CombatError::PlacementInUse { .. } => CodeV3::PlacementInUse,
        CombatError::NotCreature { .. } => CodeV3::NotCreature,
        CombatError::AlreadyParticipant { .. } => CodeV3::AlreadyParticipant,
        CombatError::NoEntrants => CodeV3::NoEntrants,
        _ => CodeV3::IllegalAction,
    }
}

/// The code a session's receipt carries for a sim refusal: exact for v3,
/// `IllegalAction` for v1/v2 (D-4).
fn refusal_code(v3: bool, error: &CombatError) -> CodeV3 {
    if v3 {
        code(error)
    } else {
        CodeV3::IllegalAction
    }
}

/// The trusted API's projection of a v3 code onto the frozen v1 type (T141
/// Reading 17): `21..=29` read `IllegalAction`, `30` reads `NotAuthorized`.
fn trusted_code(code: CodeV3) -> RejectionCode {
    match code {
        CodeV3::FrameTooLarge => RejectionCode::FrameTooLarge,
        CodeV3::QueueFull => RejectionCode::QueueFull,
        CodeV3::UnsupportedVersion => RejectionCode::UnsupportedVersion,
        CodeV3::WrongDirection => RejectionCode::WrongDirection,
        CodeV3::UnknownMessage => RejectionCode::UnknownMessage,
        CodeV3::Malformed => RejectionCode::Malformed,
        CodeV3::LimitExceeded => RejectionCode::LimitExceeded,
        CodeV3::RateLimited => RejectionCode::RateLimited,
        CodeV3::Unauthenticated => RejectionCode::Unauthenticated,
        CodeV3::SessionExpired => RejectionCode::SessionExpired,
        CodeV3::SeqConflict => RejectionCode::SeqConflict,
        CodeV3::SeqGap => RejectionCode::SeqGap,
        CodeV3::StaleSeq => RejectionCode::StaleSeq,
        CodeV3::SeqExhausted => RejectionCode::SeqExhausted,
        CodeV3::StaleTick => RejectionCode::StaleTick,
        CodeV3::FutureTick => RejectionCode::FutureTick,
        CodeV3::NotAuthorized | CodeV3::NoPauseRight => RejectionCode::NotAuthorized,
        CodeV3::IllegalAction
        | CodeV3::ModeUnsupported
        | CodeV3::WrongMode
        | CodeV3::StatEvaluation
        | CodeV3::EncounterTerminal
        | CodeV3::RulesetMismatch
        | CodeV3::PlacementInUse
        | CodeV3::NotCreature
        | CodeV3::AlreadyParticipant
        | CodeV3::NoEntrants => RejectionCode::IllegalAction,
        CodeV3::ServerBusy => RejectionCode::ServerBusy,
        CodeV3::SnapshotExpired => RejectionCode::SnapshotExpired,
    }
}

/// [`trusted_code`] for a whole receipt status.
fn trusted_status(status: ReceiptV3) -> ReceiptStatus {
    match status {
        ReceiptV3::Applied => ReceiptStatus::Applied,
        ReceiptV3::Rejected(code) => ReceiptStatus::Rejected(trusted_code(code)),
    }
}

/// One v3 `Receipt` op.
fn receipt_op_v3(epoch: [u8; 16], seq: u64, processed_tick: u64, status: ReceiptV3) -> DeltaOpV3 {
    DeltaOpV3::Receipt {
        epoch,
        lane: LANE_COMBAT,
        seq,
        processed_tick,
        status,
    }
}

/// `resolve_action`'s mapping/ownership/disclosure test for one sim action;
/// `None` drops it. The actor is the L3-eligible `active`.
fn legal_body(
    grants: &DisclosureGrants,
    net_of: &dyn Fn(EntityId) -> Option<NetId>,
    action: &CombatAction,
) -> Option<IntentBody> {
    match *action {
        CombatAction::UseAbility {
            ability, target, ..
        } => {
            let target_net = net_of(target)?;
            let target_grant = lookup(grants, target)?;
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

    /// Maps one history fact to a v3 candidate (D-8, H-3, H-4): the six v2
    /// events as `V2`; the five v3 events from the grants, `control` (the
    /// `OrderEnded` actor) and `mode_granted` (the `ModeChanged` mode);
    /// `Moved` stays withheld (H-4).
    fn candidate_v3(
        &self,
        control: &[EntityId],
        mode_granted: bool,
        event: &HistoryEvent,
    ) -> Option<EventCandidateV3> {
        match event {
            HistoryEvent::RoundStarted { round } => Some(EventCandidateV3::RoundStarted {
                round: self.grants.turn_round.then_some(*round),
            }),
            HistoryEvent::ModeChanged { mode, round } => Some(EventCandidateV3::ModeChanged {
                mode: mode_granted.then(|| net_mode(*mode)),
                round: self.grants.turn_round.then_some(*round),
            }),
            HistoryEvent::OrderEnded { actor } => Some(EventCandidateV3::OrderEnded {
                actor: control
                    .binary_search(actor)
                    .ok()
                    .and_then(|_| self.net_if(*actor, |_| true)),
            }),
            HistoryEvent::Joined { entity } => Some(EventCandidateV3::Joined {
                entity: self.net_if(*entity, |_| true),
                disclose: self.grant(*entity).is_some_and(|g| g.present && g.spawn),
            }),
            HistoryEvent::Left { entity } => Some(EventCandidateV3::Left {
                entity: self.net_if(*entity, |_| true),
                disclose: self.grant(*entity).is_some_and(|g| g.present && g.despawn),
            }),
            HistoryEvent::Moved { .. } => None,
            other => self.candidate(other).map(EventCandidateV3::V2),
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

    // -- T141c: v3 outbox pacing and staging bounds (C9, C-5, C-11) ----------

    fn v3_facts() -> FactGrants {
        FactGrants {
            entities: (0..3)
                .map(|index| EntityFactGrant {
                    entity: eid(index),
                    pools: true,
                    effects: true,
                    stats: true,
                    tags: true,
                })
                .collect(),
            encounter: EncounterGrants {
                mode: true,
                pending_mode: true,
                clock: true,
                timeline: true,
            },
            host: DEFAULT_HOST_GRANTS,
        }
    }

    /// A v3 session on a turn-based `V2` host, its first delivery drained and
    /// acknowledged, so its log and egress are empty.
    fn drained_v3_host() -> (Host, PeerHandle) {
        let mut host = v2_host(false);
        let peer = host
            .bind_v3_peer(
                ControlGrant {
                    actors: vec![eid(0)],
                },
                full_grants(),
                v3_facts(),
                PauseRights {
                    pause: true,
                    unpause: true,
                },
                0,
            )
            .expect("binds");
        ack_all(&mut host, peer);
        assert_eq!(host.egress_bytes, 0);
        (host, peer)
    }

    /// Replaces the outbox with `count` transfer-kind entries of `len` bytes,
    /// each filled with its own index so order is checkable.
    fn load_outbox(host: &mut Host, peer: PeerHandle, count: usize, len: usize) {
        let session = host.sessions.get_mut(&peer).expect("bound");
        let outbox = &mut session.v3.as_mut().expect("v3").outbox;
        outbox.clear();
        for index in 0..count {
            outbox.push_back(OutboxEntry {
                event_seq: None,
                bytes: vec![index as u8; len],
            });
        }
    }

    fn logged(host: &Host, peer: PeerHandle) -> Vec<(u64, u8)> {
        let session = host.sessions.get(&peer).expect("bound");
        session
            .log
            .iter()
            .map(|frame| (frame.ordinal, frame.bytes[0]))
            .collect()
    }

    fn outbox_len(host: &Host, peer: PeerHandle) -> usize {
        let session = host.sessions.get(&peer).expect("bound");
        session.v3.as_ref().expect("v3").outbox.len()
    }

    /// C9, part 1: pacing moves entries in order, consecutive ordinals, while
    /// the per-peer frame and byte caps and the host byte cap allow.
    #[test]
    fn pacing_respects_every_cap_in_order() {
        // 200 entries of 20,000 bytes: 104 fit 2,097,152 (104 x 20,000 =
        // 2,080,000; a 105th would make 2,100,000).
        let (mut host, peer) = drained_v3_host();
        let start = host.sessions.get(&peer).expect("bound").next_ordinal;
        load_outbox(&mut host, peer, 200, 20_000);
        host.pace(peer);
        let log = logged(&host, peer);
        assert_eq!(log.len(), 104);
        assert_eq!(host.egress_bytes, 2_080_000);
        for (position, (ordinal, first)) in log.iter().enumerate() {
            assert_eq!(*ordinal, start + position as u64);
            assert_eq!(*first, position as u8);
        }
        assert_eq!(outbox_len(&host, peer), 96);
        // Acknowledging 10 frees 200,000 bytes: exactly 10 more move.
        host.acknowledge_delivery(peer, start + 9).expect("acks");
        let log = logged(&host, peer);
        assert_eq!(log.len(), 104);
        assert_eq!(log[0], (start + 10, 10));
        assert_eq!(log[103], (start + 113, 113));
        assert_eq!(outbox_len(&host, peer), 86);

        // 130 entries of 100 bytes: the 128-frame cap stops at 128.
        let (mut host, peer) = drained_v3_host();
        let start = host.sessions.get(&peer).expect("bound").next_ordinal;
        load_outbox(&mut host, peer, 130, 100);
        host.pace(peer);
        assert_eq!(logged(&host, peer).len(), 128);
        assert_eq!(outbox_len(&host, peer), 2);
        host.acknowledge_delivery(peer, start).expect("acks one");
        let log = logged(&host, peer);
        assert_eq!(log.len(), 128);
        assert_eq!(log[127], (start + 128, 128));
        assert_eq!(outbox_len(&host, peer), 1);

        // Host egress at 16,727,216: 50,000 bytes of host room, so 2 entries.
        let (mut host, peer) = drained_v3_host();
        load_outbox(&mut host, peer, 10, 20_000);
        host.egress_bytes = 16_727_216;
        host.pace(peer);
        assert_eq!(logged(&host, peer).len(), 2);
        assert_eq!(host.egress_bytes, 16_727_216 + 40_000);
        assert_eq!(outbox_len(&host, peer), 8);
    }

    /// The T140 D5 body: 1,024 fully disclosed entities, every bound at its
    /// maximum (as in `crates/crpg-net/tests/v3_snapshot.rs`).
    fn largest_body() -> crpg_net::transfer_v3::SnapshotBodyV3 {
        use crpg_net::protocol_v3::{
            ClockFact, EffectFact, EncounterFacts, FactSet, NetId as Net, OrderFact, PoolFact,
            StatFact, StatValue, TimelineEntry,
        };
        use crpg_net::transfer_v3::{SnapshotBodyV3, SnapshotEntityV3};
        let net = |raw: u64| Net::new(raw).expect("nonzero");
        let base = 1u64 << 63;
        let entities = (0..1024u64)
            .map(|i| SnapshotEntityV3 {
                entity: net(base + i),
                health: Some(HealthFact {
                    health: u32::MAX,
                    max_health: u32::MAX,
                    dead: true,
                }),
                facts: FactSet {
                    pools: Some(
                        (1..=16u128)
                            .map(|n| PoolFact {
                                pool: Ulid::from_u128(n),
                                current: u32::MAX,
                                max: u32::MAX,
                            })
                            .collect(),
                    ),
                    effects: Some(
                        (1..=16u128)
                            .map(|n| EffectFact {
                                effect: Ulid::from_u128(n),
                                expires_round: u64::MAX,
                            })
                            .collect(),
                    ),
                    order: Some(Some(OrderFact {
                        ability: Ulid::from_u128(1),
                        target: net(base),
                        repeat: true,
                    })),
                    stats: Some(
                        (0..32)
                            .map(|n| StatFact {
                                name: format!("{n:032}"),
                                value: StatValue::Int(i32::MIN),
                            })
                            .collect(),
                    ),
                    tags: Some((0..8).map(|n| format!("{n:032}")).collect()),
                },
            })
            .collect();
        SnapshotBodyV3 {
            entities,
            turn: Some(TurnFact {
                active: Some(net(base)),
                round: u64::MAX,
            }),
            encounter: EncounterFacts {
                mode: Some(crpg_net::protocol_v3::EncounterMode::RealTime),
                pending_mode: Some(Some(crpg_net::protocol_v3::EncounterMode::TurnBased)),
                clock: Some(Some(ClockFact {
                    round_ticks: u32::MAX,
                    elapsed: 2_147_483_647,
                })),
            },
            timeline: Some(
                (0..1024u64)
                    .map(|i| TimelineEntry {
                        key: i32::MIN,
                        entity: net(base + i),
                    })
                    .collect(),
            ),
        }
    }

    /// C9, part 2: the D5 plan (88 frames, 2,851,124 bytes) exceeds the 2 MiB
    /// per-peer egress, so pacing logs the longest prefix that fits and the
    /// rest after acknowledgements, never skipping or reordering a frame.
    #[test]
    fn pacing_moves_the_d5_plan_in_order() {
        let frames = transfer_v3::plan_transfer(
            [1; 16],
            TransferId::new(1).expect("nonzero"),
            0,
            0,
            SNAPSHOT_SEGMENTS_PER_CHUNK,
            &largest_body(),
        )
        .expect("plans");
        assert_eq!(frames.len(), 88);
        assert_eq!(frames.iter().map(Vec::len).sum::<usize>(), 2_851_124);

        let (mut host, peer) = drained_v3_host();
        let start = host.sessions.get(&peer).expect("bound").next_ordinal;
        {
            let session = host.sessions.get_mut(&peer).expect("bound");
            let outbox = &mut session.v3.as_mut().expect("v3").outbox;
            outbox.clear();
            outbox.extend(frames.iter().map(|bytes| OutboxEntry {
                event_seq: None,
                bytes: bytes.clone(),
            }));
        }
        host.pace(peer);
        // The 30-byte Begin plus 63 chunks of 32,806 bytes: 2,066,808; a 64th
        // chunk would make 2,099,614, over 2,097,152.
        let session = host.sessions.get(&peer).expect("bound");
        assert_eq!(session.log.len(), 64);
        assert_eq!(session.log_bytes, 2_066_808);
        assert_eq!(outbox_len(&host, peer), 24);
        // Drain in rounds: every frame appears once, in plan order.
        let mut seen: Vec<Vec<u8>> = Vec::new();
        for _ in 0..4 {
            let entries = host.take_delivery_entries(peer).expect("bound");
            let Some(last) = entries.last().map(|entry| entry.ordinal) else {
                break;
            };
            seen.extend(entries.into_iter().map(|entry| entry.bytes));
            host.acknowledge_delivery(peer, last).expect("acks");
        }
        assert_eq!(seen, frames);
        assert_eq!(outbox_len(&host, peer), 0);
        assert_eq!(
            host.sessions.get(&peer).expect("bound").next_ordinal,
            start + 88
        );
    }

    /// C-11: the outbox bound is exactly 90 entries and 2,852,148 bytes.
    #[test]
    fn outbox_bound_is_exact() {
        assert!(outbox_fits(90, 2_852_148));
        assert!(outbox_fits(0, 0));
        assert!(outbox_fits(89, 2_852_148));
        assert!(outbox_fits(90, 0));
        assert!(!outbox_fits(91, 100));
        assert!(!outbox_fits(10, 2_852_149));
        assert!(!outbox_fits(91, 2_852_149));
    }

    /// C-1, C-7, C-11: a bind never needs egress room. With the host's egress
    /// full the plan waits in the outbox (not counted as egress), and a later
    /// take logs it once there is room; a narrowing replaces the outbox; an
    /// unbind discards it.
    #[test]
    fn bind_never_blocks_on_egress_and_outbox_is_replaced() {
        let mut host = v2_host(false);
        let full = host.egress_caps.host_bytes;
        host.egress_bytes = full;
        let peer = host
            .bind_v3_peer(
                ControlGrant {
                    actors: vec![eid(0)],
                },
                full_grants(),
                v3_facts(),
                PauseRights::default(),
                0,
            )
            .expect("binds without egress room");
        assert_eq!(host.egress_bytes, full, "the outbox is not egress");
        assert_eq!(outbox_len(&host, peer), 3);
        assert_eq!(host.take_delivery_entries(peer), Ok(Vec::new()));
        // A narrowing replaces the plan: Cancel (transfer 1 is in flight),
        // Begin, Chunk and a delta that took event_seq 2.
        let mut narrow = full_grants();
        narrow.entities.pop();
        assert_eq!(
            host.set_grants(peer, narrow),
            Ok(GrantUpdate::SnapshotStaged)
        );
        assert_eq!(outbox_len(&host, peer), 4);
        let session = host.sessions.get(&peer).expect("bound");
        let v3 = session.v3.as_ref().expect("v3");
        assert!(matches!(
            transfer_v3::decode_down(&v3.outbox[0].bytes),
            Ok(TransferDown::Cancel { .. })
        ));
        assert_eq!(v3.outbox[3].event_seq, Some(2));
        assert_eq!(session.next_event_seq, 3);
        assert_eq!(host.take_delivery_entries(peer), Ok(Vec::new()));
        // Room appears: the whole plan is logged, in order, from ordinal 1.
        host.egress_bytes = 0;
        let entries = host.take_delivery_entries(peer).expect("bound");
        let ordinals: Vec<u64> = entries.iter().map(|entry| entry.ordinal).collect();
        assert_eq!(ordinals, vec![1, 2, 3, 4]);
        assert_eq!(entries[3].kind, DeliveryKind::Delta);
        assert_eq!(outbox_len(&host, peer), 0);

        // Unbind discards the outbox without touching egress.
        let mut host = v2_host(false);
        host.egress_bytes = full;
        let peer = host
            .bind_v3_peer(
                ControlGrant { actors: vec![] },
                full_grants(),
                v3_facts(),
                PauseRights::default(),
                0,
            )
            .expect("binds");
        assert_eq!(outbox_len(&host, peer), 3);
        host.unbind_peer(peer);
        assert!(host.sessions.is_empty());
        assert_eq!(host.egress_bytes, full);
    }

    /// The comparable state of one v3 session after a refused staging.
    #[derive(Debug, PartialEq)]
    struct StagingEvidence {
        log: Vec<(u64, Option<u64>, Vec<u8>)>,
        outbox: Vec<(Option<u64>, Vec<u8>)>,
        next_ordinal: u64,
        delivered: u64,
        next_transfer: u64,
        in_flight: Option<u64>,
        next_event_seq: u64,
        egress_bytes: usize,
        views: (
            crpg_net::interest::InterestView,
            crpg_net::facts_v3::FactView,
        ),
    }

    fn staging_evidence(host: &Host, peer: PeerHandle) -> StagingEvidence {
        let session = host.sessions.get(&peer).expect("bound");
        let v3 = session.v3.as_ref().expect("v3");
        StagingEvidence {
            log: session
                .log
                .iter()
                .map(|frame| (frame.ordinal, frame.event_seq, frame.bytes.clone()))
                .collect(),
            outbox: v3
                .outbox
                .iter()
                .map(|entry| (entry.event_seq, entry.bytes.clone()))
                .collect(),
            next_ordinal: session.next_ordinal,
            delivered: session.delivered,
            next_transfer: session.next_transfer,
            in_flight: session.in_flight,
            next_event_seq: session.next_event_seq,
            egress_bytes: host.egress_bytes,
            views: (v3.view.clone(), v3.facts.clone()),
        }
    }

    /// C-5 steps 1 and 4: a staging at a counter sentinel, or without ordinal
    /// room for its whole plan, is `CounterExhausted` and changes nothing; one
    /// ordinal inside the bound it commits.
    #[test]
    fn v3_staging_counters_are_checked_before_anything_changes() {
        let resync = |host: &Host, peer: PeerHandle| {
            transfer_v3::encode_up(&TransferUp::Resync {
                epoch: host.epoch(peer).expect("bound"),
                after: None,
                reason: crpg_net::protocol_transfer::ResyncReason::Overflow,
            })
            .expect("encodes")
        };
        // (a) the transfer-id sentinel.
        let (mut host, peer) = drained_v3_host();
        host.sessions.get_mut(&peer).expect("bound").next_transfer = u64::MAX;
        let before = staging_evidence(&host, peer);
        let bytes = resync(&host, peer);
        assert_eq!(
            host.ingest_transfer(peer, &bytes, 0),
            Err(HostError::CounterExhausted)
        );
        let mut narrow = full_grants();
        narrow.entities.pop();
        assert_eq!(
            host.set_grants(peer, narrow.clone()),
            Err(HostError::CounterExhausted)
        );
        assert_eq!(staging_evidence(&host, peer), before);
        host.sessions.get_mut(&peer).expect("bound").next_transfer = u64::MAX - 1;
        assert_eq!(
            host.set_grants(peer, narrow),
            Ok(GrantUpdate::SnapshotStaged)
        );
        let session = host.sessions.get(&peer).expect("bound");
        assert_eq!(session.next_transfer, u64::MAX);
        assert_eq!(session.in_flight, Some(u64::MAX - 1));

        // (b) ordinal room: transfer 1 is still in flight, so the plan is a
        // Cancel, a Begin, a Chunk and the host-facts delta: 4 entries.
        let (mut host, peer) = drained_v3_host();
        {
            let session = host.sessions.get_mut(&peer).expect("bound");
            session.next_ordinal = u64::MAX - 4;
            session.delivered = u64::MAX - 5;
        }
        let before = staging_evidence(&host, peer);
        let bytes = resync(&host, peer);
        assert_eq!(
            host.ingest_transfer(peer, &bytes, 0),
            Err(HostError::CounterExhausted)
        );
        assert_eq!(staging_evidence(&host, peer), before);
        {
            let session = host.sessions.get_mut(&peer).expect("bound");
            session.next_ordinal = u64::MAX - 5;
            session.delivered = u64::MAX - 6;
        }
        let bytes = resync(&host, peer);
        assert_eq!(
            host.ingest_transfer(peer, &bytes, 0),
            Ok(TransferDisposition::Resynced)
        );
        let session = host.sessions.get(&peer).expect("bound");
        assert_eq!(session.next_ordinal, u64::MAX - 1);
        let ordinals: Vec<u64> = session.log.iter().map(|frame| frame.ordinal).collect();
        assert_eq!(
            ordinals,
            vec![u64::MAX - 5, u64::MAX - 4, u64::MAX - 3, u64::MAX - 2]
        );
    }

    /// C-5, C-6: a v3 staging's delta takes the next `event_seq` when it is
    /// built, so the delivery sequence stays gapless across stagings.
    #[test]
    fn v3_staging_takes_one_event_seq_per_plan() {
        let (mut host, peer) = drained_v3_host();
        assert_eq!(host.sessions.get(&peer).expect("bound").next_event_seq, 2);
        let mut narrow = full_grants();
        narrow.entities.pop();
        assert_eq!(
            host.set_grants(peer, narrow),
            Ok(GrantUpdate::SnapshotStaged)
        );
        assert_eq!(host.sessions.get(&peer).expect("bound").next_event_seq, 3);
        let entries = host.take_delivery_entries(peer).expect("bound");
        let delta = entries.last().expect("a delta");
        assert_eq!(delta.kind, DeliveryKind::Delta);
        let frame = crpg_net::codec_v3::decode_delta(&delta.bytes).expect("decodes");
        assert_eq!(frame.event_seq, 2);
        assert_eq!(frame.server_tick, 0);
    }

    /// D-9 (kept from T141c): v3 sessions take no part in `accepted_room`, so a
    /// full v3 log never blocks admission, while an equally full v2 log does.
    #[test]
    fn accepted_room_skips_v3_sessions() {
        let fill = |host: &mut Host, peer: PeerHandle| {
            let session = host.sessions.get_mut(&peer).expect("bound");
            for index in 0..host.egress_caps.per_peer_frames {
                session.log.push_back(LoggedFrame {
                    ordinal: 1_000 + index as u64,
                    event_seq: None,
                    bytes: vec![0],
                });
            }
        };
        let (mut host, peer) = drained_v3_host();
        fill(&mut host, peer);
        assert_eq!(host.accepted_room(), Ok(true));
        // Control: the same fill on a v2 session blocks.
        let v2 = host
            .bind_peer(all_actors(), full_grants(), 0)
            .expect("binds");
        assert_eq!(host.accepted_room(), Ok(true));
        fill(&mut host, v2);
        assert_eq!(host.accepted_room(), Ok(false));
    }

    /// A v2 peer's command reaches a v3 session only as a v3 page, never as v2
    /// bytes, and a v2-format intent from a v3 session is refused as an
    /// unsupported version with no frame, sequence or execution. T141c pinned
    /// the interim "no page at all" rule here; T141d (D-6) replaced that half
    /// with the page, whose ops `tests/host_v3_publish.rs` D1 pins.
    #[test]
    fn v3_sessions_are_not_served_v2_bytes() {
        let mut host = v2_host(false);
        let v = host
            .bind_peer(all_actors(), full_grants(), 0)
            .expect("binds");
        let p = host
            .bind_v3_peer(
                all_actors(),
                full_grants(),
                v3_facts(),
                PauseRights::default(),
                0,
            )
            .expect("binds");
        ack_all(&mut host, v);
        ack_all(&mut host, p);
        // E-2 and the T141c/T141d test rules: end the transfer first, so the throttle does not hold the page.
        host.sessions.get_mut(&p).expect("bound").in_flight = None;
        let bytes = intent(&host, v, 1, 1, IntentBody::EndTurn);
        assert_eq!(host.ingest(v, &bytes, 1), Ok(IngestDisposition::Staged));
        let summary = host.pump(1).expect("pump");
        assert_eq!(summary.applied, 1);
        assert_eq!(host.executions(), 1);
        assert!(!host.take_delivery(v).expect("bound").is_empty());
        // V's own frame is 43 bytes of the host's egress total; P's log holds
        // exactly one frame, a v3 delta that no v2 decoder reads.
        let before = staging_evidence(&host, p);
        assert_eq!(before.log.len(), 1);
        let (_, event_seq, page) = &before.log[0];
        assert_eq!(*event_seq, Some(2));
        assert!(crpg_net::codec_v3::decode_delta(page).is_ok());
        assert!(codec_v2::decode_delta(page).is_err());
        assert_eq!(before.egress_bytes, 43 + page.len());

        // The v3 session's own v2-format intent.
        let bytes = intent(&host, p, 1, 1, IntentBody::EndTurn);
        assert_eq!(host.ingest(p, &bytes, 2), Ok(IngestDisposition::Staged));
        let summary = host.pump(2).expect("pump");
        assert_eq!(summary.rejected, 1);
        assert_eq!(
            summary.results[0].status,
            ReceiptStatus::Rejected(RejectionCode::UnsupportedVersion)
        );
        assert_eq!(host.executions(), 1);
        assert_eq!(host.cached_status(p, 1), Ok(None));
        assert_eq!(staging_evidence(&host, p), before);
    }
    // -- T141c fix round: review survivors (private-field cases) ---------------

    /// Replaces the outbox with one entry per size in `sizes`, each filled with
    /// its own index.
    fn load_sizes(host: &mut Host, peer: PeerHandle, sizes: &[usize]) {
        let session = host.sessions.get_mut(&peer).expect("bound");
        let outbox = &mut session.v3.as_mut().expect("v3").outbox;
        outbox.clear();
        for (index, len) in sizes.iter().enumerate() {
            outbox.push_back(OutboxEntry {
                event_seq: None,
                bytes: vec![index as u8; *len],
            });
        }
    }

    /// C-7: the per-peer byte cap (2,097,152) and the host byte cap
    /// (16,777,216) each admit a frame that lands exactly on them and refuse
    /// one that is a byte over.
    #[test]
    fn pacing_byte_caps_are_inclusive() {
        let (mut host, peer) = drained_v3_host();
        assert_eq!(host.egress_caps.per_peer_bytes, 2_097_152);
        assert_eq!(host.egress_caps.host_bytes, 16_777_216);
        // Per peer: exactly on the cap moves both; a byte over stops the second.
        load_sizes(&mut host, peer, &[1_048_576, 1_048_576]);
        host.pace(peer);
        assert_eq!(logged(&host, peer).len(), 2);
        assert_eq!(
            host.sessions.get(&peer).expect("bound").log_bytes,
            2_097_152
        );
        let (mut host, peer) = drained_v3_host();
        load_sizes(&mut host, peer, &[1_048_576, 1_048_577]);
        host.pace(peer);
        assert_eq!(logged(&host, peer).len(), 1);
        assert_eq!(outbox_len(&host, peer), 1);
        // Host: exactly on the cap moves both; a byte over stops the second.
        let (mut host, peer) = drained_v3_host();
        load_sizes(&mut host, peer, &[20_000, 20_000]);
        host.egress_bytes = 16_777_216 - 40_000;
        host.pace(peer);
        assert_eq!(logged(&host, peer).len(), 2);
        assert_eq!(host.egress_bytes, 16_777_216);
        let (mut host, peer) = drained_v3_host();
        load_sizes(&mut host, peer, &[20_000, 20_000]);
        host.egress_bytes = 16_777_216 - 39_999;
        host.pace(peer);
        assert_eq!(logged(&host, peer).len(), 1);
        assert_eq!(host.egress_bytes, 16_777_216 - 39_999 + 20_000);
        assert_eq!(outbox_len(&host, peer), 1);
    }

    /// C-7: `take_delivery` (the bytes-only read) paces the outbox, as
    /// `take_delivery_entries` does.
    #[test]
    fn take_delivery_paces_the_outbox() {
        let mut host = v2_host(false);
        let full = host.egress_caps.host_bytes;
        host.egress_bytes = full;
        let peer = host
            .bind_v3_peer(
                ControlGrant {
                    actors: vec![eid(0)],
                },
                full_grants(),
                v3_facts(),
                PauseRights::default(),
                0,
            )
            .expect("binds without egress room");
        assert_eq!(host.take_delivery(peer), Ok(Vec::new()));
        assert_eq!(outbox_len(&host, peer), 3);
        host.egress_bytes = 0;
        let frames = host.take_delivery(peer).expect("bound");
        assert_eq!(frames.len(), 3);
        assert_eq!(outbox_len(&host, peer), 0);
        assert_eq!(logged(&host, peer).len(), 3);
    }

    /// C-5 step 5: a restage retires the old log's bytes from egress and
    /// acknowledges its ordinals, so the ordinals stay gapless.
    #[test]
    fn restage_retires_the_old_log() {
        let mut host = v2_host(false);
        let peer = host
            .bind_v3_peer(
                ControlGrant {
                    actors: vec![eid(0)],
                },
                full_grants(),
                v3_facts(),
                PauseRights::default(),
                0,
            )
            .expect("binds");
        let session = host.sessions.get(&peer).expect("bound");
        let old_bytes = session.log_bytes;
        assert!(old_bytes > 0);
        assert_eq!(host.egress_bytes, old_bytes);
        assert_eq!((session.delivered, session.next_ordinal), (0, 4));
        let mut narrow = full_grants();
        narrow.entities.pop();
        assert_eq!(
            host.set_grants(peer, narrow),
            Ok(GrantUpdate::SnapshotStaged)
        );
        let session = host.sessions.get(&peer).expect("bound");
        let new_bytes: usize = session.log.iter().map(|frame| frame.bytes.len()).sum();
        assert_eq!(session.delivered, 3, "the old ordinals are acknowledged");
        assert_eq!(session.log.front().map(|frame| frame.ordinal), Some(4));
        assert_eq!(session.log_bytes, new_bytes);
        assert_eq!(host.egress_bytes, new_bytes, "the old log left egress");
        // Control: a drained session restages to the same accounting.
        let (mut host, peer) = drained_v3_host();
        let mut narrow = full_grants();
        narrow.entities.pop();
        assert_eq!(
            host.set_grants(peer, narrow),
            Ok(GrantUpdate::SnapshotStaged)
        );
        let session = host.sessions.get(&peer).expect("bound");
        assert_eq!(session.delivered, 3);
        assert_eq!(host.egress_bytes, session.log_bytes);
    }

    /// C-6: the host-facts delta clamps a timer remainder above `u32::MAX`.
    #[test]
    fn host_input_clamps_the_timer_remainder() {
        let mut host = v2_host(false);
        let armed = |remaining: u64| {
            Some(TurnTimer {
                head: eid(0),
                round: 0,
                clock: TimerClock::Frozen { remaining },
            })
        };
        host.timer = armed(1000);
        assert_eq!(
            host.host_input(PauseRights::default()).turn_timer_ms,
            Some(1000)
        );
        host.timer = armed(u64::from(u32::MAX));
        assert_eq!(
            host.host_input(PauseRights::default()).turn_timer_ms,
            Some(u32::MAX)
        );
        host.timer = armed(u64::from(u32::MAX) + 1);
        assert_eq!(
            host.host_input(PauseRights::default()).turn_timer_ms,
            Some(u32::MAX)
        );
        host.timer = armed(5_000_000_000);
        let peer = host
            .bind_v3_peer(
                ControlGrant { actors: vec![] },
                full_grants(),
                v3_facts(),
                PauseRights::default(),
                0,
            )
            .expect("binds");
        let entries = host.take_delivery_entries(peer).expect("bound");
        let delta = crpg_net::codec_v3::decode_delta(&entries[2].bytes).expect("decodes");
        match &delta.ops[1] {
            crpg_net::protocol_v3::DeltaOp::HostFacts(facts) => {
                assert_eq!(facts.turn_timer_ms, Some(Some(u32::MAX)));
            }
            other => panic!("expected HostFacts, got {other:?}"),
        }
    }

    /// The `Display` of `NotV3Peer` is `<VariantName> at host/admission`.
    #[test]
    fn not_v3_peer_display_is_pinned() {
        assert_eq!(
            HostError::NotV3Peer.to_string(),
            "NotV3Peer at host/admission"
        );
    }

    /// C-5, C-6: a staging whose delta would take `event_seq` `u64::MAX` is
    /// `CounterExhausted` and changes nothing; one below it commits.
    #[test]
    fn v3_staging_refuses_the_last_event_seq() {
        let resync = |host: &Host, peer: PeerHandle| {
            transfer_v3::encode_up(&TransferUp::Resync {
                epoch: host.epoch(peer).expect("bound"),
                after: None,
                reason: crpg_net::protocol_transfer::ResyncReason::Overflow,
            })
            .expect("encodes")
        };
        let (mut host, peer) = drained_v3_host();
        host.sessions.get_mut(&peer).expect("bound").next_event_seq = u64::MAX;
        let before = staging_evidence(&host, peer);
        let bytes = resync(&host, peer);
        assert_eq!(
            host.ingest_transfer(peer, &bytes, 0),
            Err(HostError::CounterExhausted)
        );
        assert_eq!(staging_evidence(&host, peer), before);
        host.sessions.get_mut(&peer).expect("bound").next_event_seq = u64::MAX - 1;
        let bytes = resync(&host, peer);
        assert_eq!(
            host.ingest_transfer(peer, &bytes, 0),
            Ok(TransferDisposition::Resynced)
        );
        let session = host.sessions.get(&peer).expect("bound");
        assert_eq!(session.next_event_seq, u64::MAX);
    }

    // -- T141d: codes, candidates, fencing and page cutting (D5, D13, D14) ----

    /// D5: the nine named sim refusals take codes 21 to 29 in order and every
    /// other refusal stays 18; the trusted projection folds 21 to 29 onto
    /// `IllegalAction` and 30 onto `NotAuthorized`; v1/v2 sessions never get a
    /// v3 code.
    #[test]
    fn code_map_and_trusted_projection() {
        let entity = eid(0);
        let named = [
            (CombatError::ModeUnsupported, CodeV3::ModeUnsupported, 21),
            (
                CombatError::WrongMode {
                    mode: EncounterMode::RealTime,
                },
                CodeV3::WrongMode,
                22,
            ),
            (
                CombatError::StatEvaluation {
                    stat: "alpha".to_owned(),
                },
                CodeV3::StatEvaluation,
                23,
            ),
            (
                CombatError::EncounterTerminal,
                CodeV3::EncounterTerminal,
                24,
            ),
            (
                CombatError::RulesetMismatch { entity },
                CodeV3::RulesetMismatch,
                25,
            ),
            (
                CombatError::PlacementInUse {
                    placement: Ulid::from_u128(1),
                },
                CodeV3::PlacementInUse,
                26,
            ),
            (CombatError::NotCreature { entity }, CodeV3::NotCreature, 27),
            (
                CombatError::AlreadyParticipant { entity },
                CodeV3::AlreadyParticipant,
                28,
            ),
            (CombatError::NoEntrants, CodeV3::NoEntrants, 29),
        ];
        for (error, expected, wire) in named {
            assert_eq!(code(&error), expected);
            assert_eq!(code(&error).as_u8(), wire);
            assert_eq!(refusal_code(true, &error), expected);
            // v1/v2 sessions keep `IllegalAction` for all of them.
            assert_eq!(refusal_code(false, &error), CodeV3::IllegalAction);
            assert_eq!(trusted_code(code(&error)), RejectionCode::IllegalAction);
        }
        let others = [
            CombatError::NoEncounter,
            CombatError::OutOfTurn {
                actor: entity,
                active: None,
            },
            CombatError::InsufficientAction {
                pool: Ulid::from_u128(606),
                cost: 1,
                current: 0,
            },
        ];
        for error in others {
            assert_eq!(code(&error), CodeV3::IllegalAction);
            assert_eq!(code(&error).as_u8(), 18);
            assert_eq!(refusal_code(true, &error), CodeV3::IllegalAction);
        }
        // The trusted projection: 30 reads `NotAuthorized`, the v1 codes read
        // as themselves.
        assert_eq!(
            trusted_code(CodeV3::NoPauseRight),
            RejectionCode::NotAuthorized
        );
        assert_eq!(
            trusted_status(ReceiptV3::Rejected(CodeV3::NoPauseRight)),
            ReceiptStatus::Rejected(RejectionCode::NotAuthorized)
        );
        assert_eq!(trusted_status(ReceiptV3::Applied), ReceiptStatus::Applied);
        for raw in 1..=20u8 {
            let v3 = CodeV3::from_u8(raw).expect("a v1 code");
            assert_eq!(trusted_code(v3).as_u8(), raw);
        }
        for raw in 21..=29u8 {
            let v3 = CodeV3::from_u8(raw).expect("a v3 code");
            assert_eq!(trusted_code(v3), RejectionCode::IllegalAction);
        }
    }

    /// A hand-built grant table: `entity(1)` fully disclosed and mapped to
    /// `NetId` 1; `entity(2)` present with no notice grants, `NetId` 2;
    /// `entity(3)` not present and unmapped.
    fn candidate_table(turn_round: bool) -> (DisclosureGrants, BTreeMap<EntityId, NetId>) {
        let mut grants = DisclosureGrants {
            entities: vec![
                EntityDisclosure {
                    entity: eid(1),
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
                },
                EntityDisclosure {
                    entity: eid(2),
                    present: true,
                    health: false,
                    spawn: false,
                    despawn: false,
                    died: false,
                    action_actor: false,
                    action_target: false,
                    action_outcome: false,
                    action_damage: false,
                    turn_start: false,
                },
                EntityDisclosure {
                    entity: eid(3),
                    present: false,
                    health: true,
                    spawn: true,
                    despawn: true,
                    died: true,
                    action_actor: true,
                    action_target: true,
                    action_outcome: true,
                    action_damage: true,
                    turn_start: true,
                },
            ],
            abilities: Vec::new(),
            encounters: Vec::new(),
            turn_round,
            encounter_end: false,
            turn_state: false,
        };
        grants.entities.sort_by_key(|entry| entry.entity);
        let map = BTreeMap::from([(eid(1), net(1)), (eid(2), net(2))]);
        (grants, map)
    }

    /// D13: the private v3 candidate builder against D-8, both directions.
    #[test]
    fn candidates_follow_the_grant_table() {
        let (grants, map) = candidate_table(true);
        let view = PeerView {
            grants: &grants,
            map: &map,
        };
        let control = [eid(1)];
        let build = |event: HistoryEvent| view.candidate_v3(&control, true, &event);

        // Joined and Left need `present && spawn` / `present && despawn`.
        assert_eq!(
            build(HistoryEvent::Joined { entity: eid(1) }),
            Some(EventCandidateV3::Joined {
                entity: Some(net(1)),
                disclose: true,
            })
        );
        assert_eq!(
            build(HistoryEvent::Joined { entity: eid(2) }),
            Some(EventCandidateV3::Joined {
                entity: Some(net(2)),
                disclose: false,
            })
        );
        assert_eq!(
            build(HistoryEvent::Joined { entity: eid(3) }),
            Some(EventCandidateV3::Joined {
                entity: None,
                disclose: false,
            })
        );
        assert_eq!(
            build(HistoryEvent::Left { entity: eid(1) }),
            Some(EventCandidateV3::Left {
                entity: Some(net(1)),
                disclose: true,
            })
        );
        assert_eq!(
            build(HistoryEvent::Left { entity: eid(2) }),
            Some(EventCandidateV3::Left {
                entity: Some(net(2)),
                disclose: false,
            })
        );
        // `Moved` stays withheld (H-4).
        assert_eq!(
            build(HistoryEvent::Moved {
                entity: eid(1),
                points: Vec::new(),
            }),
            None
        );
        // `ModeChanged` needs the mode grant for the mode and `turn_round` for
        // the round.
        let changed = HistoryEvent::ModeChanged {
            mode: EncounterMode::RealTime,
            round: 7,
        };
        assert_eq!(
            view.candidate_v3(&control, true, &changed),
            Some(EventCandidateV3::ModeChanged {
                mode: Some(crpg_net::protocol_v3::EncounterMode::RealTime),
                round: Some(7),
            })
        );
        assert_eq!(
            view.candidate_v3(&control, false, &changed),
            Some(EventCandidateV3::ModeChanged {
                mode: None,
                round: Some(7),
            })
        );
        // `OrderEnded` needs control and presence of the actor.
        assert_eq!(
            build(HistoryEvent::OrderEnded { actor: eid(1) }),
            Some(EventCandidateV3::OrderEnded {
                actor: Some(net(1)),
            })
        );
        assert_eq!(
            build(HistoryEvent::OrderEnded { actor: eid(2) }),
            Some(EventCandidateV3::OrderEnded { actor: None })
        );
        assert_eq!(
            view.candidate_v3(&[eid(3)], true, &HistoryEvent::OrderEnded { actor: eid(3) }),
            Some(EventCandidateV3::OrderEnded { actor: None })
        );
        // `RoundStarted` needs `turn_round`.
        assert_eq!(
            build(HistoryEvent::RoundStarted { round: 4 }),
            Some(EventCandidateV3::RoundStarted { round: Some(4) })
        );
        let (no_round, map) = candidate_table(false);
        let hidden = PeerView {
            grants: &no_round,
            map: &map,
        };
        assert_eq!(
            hidden.candidate_v3(&control, true, &HistoryEvent::RoundStarted { round: 4 }),
            Some(EventCandidateV3::RoundStarted { round: None })
        );
        assert_eq!(
            hidden.candidate_v3(&control, true, &changed),
            Some(EventCandidateV3::ModeChanged {
                mode: Some(crpg_net::protocol_v3::EncounterMode::RealTime),
                round: None,
            })
        );
        // The six v2 events go through the v2 predicates, wrapped.
        assert!(matches!(
            build(HistoryEvent::Died { entity: eid(1) }),
            Some(EventCandidateV3::V2(_))
        ));
    }

    /// An `RT3` host with `P` (controls A) and `Q` (controls `q_control`) bound
    /// as v3 sessions and both first deliveries acknowledged, and a mirror
    /// authority for the oracle.
    fn fence_fixture(q_control: &[u32]) -> (Host, HistoryWorld, PeerHandle, PeerHandle) {
        let mut host = v2_host(true);
        let mut mirror: HistoryWorld =
            serde_json::from_value(host_fixture_value(true)).expect("fixture loads");
        mirror
            .acknowledge(mirror.last_sequence())
            .expect("acknowledges");
        let bind = |host: &mut Host, actors: &[u32]| {
            host.bind_v3_peer(
                ControlGrant {
                    actors: actors.iter().copied().map(eid).collect(),
                },
                full_grants(),
                v3_facts(),
                PauseRights {
                    pause: true,
                    unpause: true,
                },
                100,
            )
            .expect("binds")
        };
        let p = bind(&mut host, &[0]);
        let q = bind(&mut host, q_control);
        ack_all(&mut host, p);
        ack_all(&mut host, q);
        // Publish both transfers (the first-delivery rule for T141c/T141d).
        for peer in [p, q] {
            host.sessions.get_mut(&peer).expect("bound").in_flight = None;
        }
        // Three acknowledged ticks, then 128 that only Q acknowledges: P's log
        // is exactly full, and the next tick (132) starts a round, which
        // journals an event and so makes a host record.
        for tick in 0..131 {
            host.tick().expect("ticks");
            mirror.tick().expect("the mirror ticks");
            let last = mirror.last_sequence();
            mirror.acknowledge(last).expect("acknowledges");
            ack_all(&mut host, q);
            if tick < 3 {
                ack_all(&mut host, p);
            }
        }
        assert_eq!(host.sessions.get(&p).expect("bound").log.len(), 128);
        (host, mirror, p, q)
    }

    fn v3_intent(
        host: &Host,
        peer: PeerHandle,
        seq: u64,
        actor: u64,
        body: IntentBodyV3,
    ) -> Vec<u8> {
        codec_v3::encode_intent(&IntentFrameV3 {
            epoch: host.epoch(peer).expect("bound"),
            lane: LANE_COMBAT,
            seq,
            observed_tick: host.server_tick(),
            actor: net(actor),
            body,
        })
        .expect("encodes")
    }

    /// D14 (a): a session whose page does not fit and whose restage cannot be
    /// planned is fenced; the tick still succeeds, its host record is
    /// appended and the other session gets its page.
    #[test]
    fn a_failed_restage_fences_one_session_and_the_tick_succeeds() {
        let (mut host, mut mirror, p, q) = fence_fixture(&[1]);
        host.sessions.get_mut(&p).expect("bound").next_transfer = u64::MAX;
        let before_capture = host.last_capture();
        assert_eq!(host.session_wire(p), Ok(3));
        let bytes = v3_intent(&host, p, 1, 1, IntentBodyV3::EndTurn);

        assert_eq!(host.tick(), Ok(()));
        mirror.tick().expect("the mirror ticks");
        let last = mirror.last_sequence();
        mirror.acknowledge(last).expect("acknowledges");
        assert_eq!(host.authority_hash(), history_hash(&mirror));
        assert_eq!(host.session_wire(p), Err(HostError::UnknownPeer));
        assert_eq!(host.session_wire(q), Ok(3));
        assert_eq!(host.history.pending_len(), 0);

        // The host record was appended: exactly one new entry, a tick record.
        assert_eq!(host.last_capture(), before_capture + 1);
        let entries = host.read_capture_entries(before_capture, 4).expect("reads");
        assert_eq!(entries.len(), 1);
        let CaptureEntry::Host(record) = &entries[0] else {
            panic!("a host entry");
        };
        assert_eq!(record.op, HostOp::Tick);
        assert_eq!(record.events.len(), 1);
        assert_eq!(
            record.events[0].payload,
            HistoryEvent::RoundStarted { round: 33 }
        );

        // Q got its page: the round start is in it.
        let frames = host.take_delivery_entries(q).expect("bound");
        assert_eq!(frames.len(), 1);
        let page = codec_v3::decode_delta(&frames[0].bytes).expect("v3");
        assert!(page.ops.contains(&DeltaOpV3::RoundStarted { round: 33 }));
        // Control: the fenced session is gone from ingress too.
        assert_eq!(
            host.ingest(p, &bytes, 200),
            Ok(IngestDisposition::Refused {
                status: ReceiptStatus::Rejected(RejectionCode::Unauthenticated),
            })
        );
    }

    /// D14 (b): the fenced session is the originator of an accepted command:
    /// no panic, the command is `Applied`, the record is appended with only
    /// the other session's view, and the sim journal is acknowledged. (`Q`
    /// controls A here, so it sees the order and gets a frame; with `Q`
    /// controlling only B it would get none and the record no view, D-10.)
    #[test]
    fn a_fenced_originator_still_commits() {
        let (mut host, mut mirror, p, q) = fence_fixture(&[0, 1]);
        host.sessions.get_mut(&p).expect("bound").next_transfer = u64::MAX;
        let before_capture = host.last_capture();

        let bytes = v3_intent(
            &host,
            p,
            1,
            1,
            IntentBodyV3::DeclareAction {
                ability: Ulid::from_u128(603),
                target: net(2),
            },
        );
        assert_eq!(host.ingest(p, &bytes, 200), Ok(IngestDisposition::Staged));
        let summary = host.pump(200).expect("pump");
        assert_eq!(summary.applied, 1);
        assert_eq!(summary.results[0].status, ReceiptStatus::Applied);
        mirror
            .perform_action(&CombatAction::UseAbility {
                actor: eid(0),
                ability: Ulid::from_u128(603),
                target: eid(1),
            })
            .expect("the mirror accepts");
        let last = mirror.last_sequence();
        mirror.acknowledge(last).expect("acknowledges");
        assert_eq!(host.authority_hash(), history_hash(&mirror));
        assert_eq!(host.executions(), 1);
        assert_eq!(host.history.pending_len(), 0);
        assert_eq!(host.session_wire(p), Err(HostError::UnknownPeer));

        assert_eq!(host.last_capture(), before_capture + 1);
        let entries = host.read_capture_entries(before_capture, 4).expect("reads");
        let CaptureEntry::Command(record) = &entries[0] else {
            panic!("a command entry");
        };
        let epochs: Vec<[u8; 16]> = record.views.iter().map(|view| view.epoch).collect();
        assert_eq!(epochs, vec![host.epoch(q).expect("bound")]);
        // Q's page carries the order it can now see.
        let frames = host.take_delivery_entries(q).expect("bound");
        assert_eq!(frames.len(), 1);
    }

    /// D-7 step 6 and D-8: a page with more candidates than one frame holds
    /// is cut by `frame_spans`, one `event_seq` per span, in order, and no op
    /// is lost; the candidates are projected in chunks of at most 256.
    #[test]
    fn a_large_page_is_cut_into_gapless_spans() {
        let (mut host, peer) = drained_v3_host();
        // E-2 and the T141c/T141d test rules: end the transfer first, so the throttle does not hold the page.
        host.sessions.get_mut(&peer).expect("bound").in_flight = None;
        let mut events = Vec::new();
        for index in 0..300u64 {
            events.push(round_started(5 + index));
        }
        let first = host.sessions.get(&peer).expect("bound").next_event_seq;
        host.publish_v3(Some(peer), &events);
        let log = &host.sessions.get(&peer).expect("bound").log;
        let frames: Vec<crpg_net::protocol_v3::DeltaFrame> = log
            .iter()
            .map(|frame| codec_v3::decode_delta(&frame.bytes).expect("v3"))
            .collect();
        assert!(frames.len() >= 2);
        for (position, frame) in frames.iter().enumerate() {
            assert_eq!(frame.event_seq, first + position as u64);
            assert!(frame.ops.len() <= MAX_DELTA_OPS);
        }
        let rounds: usize = frames
            .iter()
            .flat_map(|frame| frame.ops.iter())
            .filter(|op| matches!(op, DeltaOpV3::RoundStarted { round: 1 }))
            .count();
        assert_eq!(rounds, 300);
        assert_eq!(
            host.sessions.get(&peer).expect("bound").next_event_seq,
            first + frames.len() as u64
        );
        // Control: no events and nothing changed publishes nothing.
        let logged = host.sessions.get(&peer).expect("bound").log.len();
        host.publish_v3(Some(peer), &[]);
        assert_eq!(host.sessions.get(&peer).expect("bound").log.len(), logged);
    }

    // -- T141d review survivors (D-7, D-8, D-9, Reading 5) -------------------

    /// Every op of every frame logged for `peer`, in order, acknowledging them.
    fn logged_ops(host: &mut Host, peer: PeerHandle) -> Vec<DeltaOpV3> {
        let entries = host.take_delivery_entries(peer).expect("bound");
        let ops = entries
            .iter()
            .filter(|entry| entry.kind == DeliveryKind::Delta)
            .flat_map(|entry| codec_v3::decode_delta(&entry.bytes).expect("v3").ops)
            .collect();
        ack_all(host, peer);
        ops
    }

    fn envelope(seq: u64, kind: &str, entity: u32) -> HistoryEnvelope {
        serde_json::from_value(serde_json::json!({
            "tick": 0,
            "seq": seq,
            "payload": {
                "type": kind,
                "value": {"entity": {"index": entity, "generation": 1}}
            }
        }))
        .expect("envelope parses")
    }

    /// A turn-based v3 host (C's id is 3), its transfer ended.
    fn open_v3_host() -> (Host, PeerHandle) {
        let (mut host, peer) = drained_v3_host();
        host.sessions.get_mut(&peer).expect("bound").in_flight = None;
        (host, peer)
    }

    /// D-7 (4), Reading 11: a page that holds both an enter and an event puts
    /// the enter first, the event after every enter and before the state ops.
    #[test]
    fn a_page_puts_enters_before_events_before_state() {
        let (mut host, peer) = open_v3_host();
        let full = host.sessions.get(&peer).expect("bound").grants.clone();
        let mut hidden = full.clone();
        hidden.entities[2].present = false;
        // C leaves the viewer's interest, then returns in the same page as an
        // event.
        host.sessions.get_mut(&peer).expect("bound").grants = hidden;
        host.publish_v3(Some(peer), &[]);
        let left = logged_ops(&mut host, peer);
        assert!(left.contains(&DeltaOpV3::EntityLeave { entity: net(3) }));
        host.sessions.get_mut(&peer).expect("bound").grants = full;
        host.publish_v3(Some(peer), &[round_started(5)]);
        let ops = logged_ops(&mut host, peer);
        assert_eq!(ops[0], DeltaOpV3::EntityEnter { entity: net(4) });
        assert_eq!(ops[1], DeltaOpV3::RoundStarted { round: 1 });
        assert_eq!(
            ops[2],
            DeltaOpV3::Health {
                entity: net(4),
                health: 3,
                max_health: 3,
                dead: false,
            }
        );
        // Control: the same event with nothing entering is the first op.
        host.publish_v3(Some(peer), &[round_started(6)]);
        assert_eq!(
            logged_ops(&mut host, peer),
            vec![DeltaOpV3::RoundStarted { round: 1 }]
        );
    }

    /// D-8: `Joined` and `Left` need `present && spawn` / `present &&
    /// despawn`, in a real page.
    #[test]
    fn joined_and_left_pages_follow_their_grants() {
        for (kind, flag) in [("joined", "spawn"), ("left", "despawn")] {
            let wanted = |ops: &[DeltaOpV3]| {
                ops.iter()
                    .filter(|op| matches!(op, DeltaOpV3::Joined { .. } | DeltaOpV3::Left { .. }))
                    .cloned()
                    .collect::<Vec<_>>()
            };
            // Control: the grant is there.
            let (mut host, peer) = open_v3_host();
            host.publish_v3(Some(peer), &[envelope(5, kind, 2)]);
            let ops = logged_ops(&mut host, peer);
            let expected = if kind == "joined" {
                DeltaOpV3::Joined { entity: net(3) }
            } else {
                DeltaOpV3::Left { entity: net(3) }
            };
            assert_eq!(wanted(&ops), vec![expected]);
            // The notice flag off: no notice.
            let (mut host, peer) = open_v3_host();
            let entry = &mut host.sessions.get_mut(&peer).expect("bound").grants.entities[2];
            match flag {
                "spawn" => entry.spawn = false,
                _ => entry.despawn = false,
            }
            host.publish_v3(Some(peer), &[envelope(5, kind, 2)]);
            assert_eq!(wanted(&logged_ops(&mut host, peer)), vec![]);
            // Not present: no notice either.
            let (mut host, peer) = open_v3_host();
            host.sessions.get_mut(&peer).expect("bound").grants.entities[2].present = false;
            host.publish_v3(Some(peer), &[envelope(5, kind, 2)]);
            assert_eq!(wanted(&logged_ops(&mut host, peer)), vec![]);
        }
    }

    /// D-7 (1), D-8: an entity that leaves the world reads as `Despawned` only
    /// for a viewer granted `present && despawn`; the event's id is the one
    /// live before the page, since the page retires it.
    #[test]
    fn a_despawn_page_needs_the_despawn_grant_and_names_the_retired_id() {
        let despawns = |ops: &[DeltaOpV3]| {
            ops.iter()
                .filter(|op| matches!(op, DeltaOpV3::Despawned { .. }))
                .count()
        };
        let run = |despawn_granted: bool| {
            let (mut host, peer) = open_v3_host();
            host.sessions.get_mut(&peer).expect("bound").grants.entities[2].despawn =
                despawn_granted;
            let before = host.history.last_sequence();
            assert_eq!(host.history.despawn(eid(2)), Ok(true));
            let events = host
                .history
                .read_after(before, MAX_HISTORY_PAGE)
                .expect("reads");
            assert!(events
                .iter()
                .any(|e| e.payload == HistoryEvent::Despawned { entity: eid(2) }));
            host.publish_v3(Some(peer), &events);
            logged_ops(&mut host, peer)
        };
        // Granted: the view's removal and the event are both `Despawned`, both
        // with the retired id 3.
        let ops = run(true);
        assert_eq!(despawns(&ops), 2);
        assert_eq!(
            ops.iter()
                .filter(|op| **op == DeltaOpV3::Despawned { entity: net(3) })
                .count(),
            2
        );
        assert!(!ops.contains(&DeltaOpV3::EntityLeave { entity: net(3) }));
        // Not granted: a plain leave and no notice.
        let ops = run(false);
        assert_eq!(despawns(&ops), 0);
        assert!(ops.contains(&DeltaOpV3::EntityLeave { entity: net(3) }));
    }

    /// An `RT3` v3 session controlling A, its first delivery acknowledged.
    fn rt3_v3_host() -> (Host, PeerHandle) {
        let mut host = v2_host(true);
        let peer = host
            .bind_v3_peer(
                ControlGrant {
                    actors: vec![eid(0)],
                },
                full_grants(),
                v3_facts(),
                PauseRights {
                    pause: true,
                    unpause: true,
                },
                0,
            )
            .expect("binds");
        ack_all(&mut host, peer);
        (host, peer)
    }

    /// S-3, Reading 5: a host operation whose record cannot be built still
    /// publishes to v3 sessions (the sim has committed), and the error is
    /// returned after the acknowledgement.
    #[test]
    fn a_failed_record_still_publishes_the_page() {
        let (mut host, peer) = rt3_v3_host();
        host.sessions.get_mut(&peer).expect("bound").in_flight = None;
        let before = host.history.last_sequence();
        host.history.tick().expect("the sim commits the tick");
        assert_eq!(
            host.finish_host_op(HostOp::Tick, before, Err(RECORD_BOUND)),
            Err(RECORD_BOUND)
        );
        let clock = |elapsed| {
            DeltaOpV3::Encounter(crpg_net::protocol_v3::EncounterFacts {
                mode: Some(crpg_net::protocol_v3::EncounterMode::RealTime),
                pending_mode: Some(None),
                clock: Some(Some(crpg_net::protocol_v3::ClockFact {
                    round_ticks: 4,
                    elapsed,
                })),
            })
        };
        assert!(logged_ops(&mut host, peer).contains(&clock(1)));
        assert_eq!(host.history.pending_len(), 0);
        assert_eq!(host.last_capture(), 0);
        // Control: a normal tick publishes its page too.
        host.tick().expect("ticks");
        assert!(logged_ops(&mut host, peer).contains(&clock(2)));
    }

    /// Adds one unacknowledged log entry of `bytes` bytes.
    fn stuff_log(host: &mut Host, peer: PeerHandle, bytes: usize) {
        let session = host.sessions.get_mut(&peer).expect("bound");
        let ordinal = session.next_ordinal;
        session.next_ordinal += 1;
        session.log.push_back(LoggedFrame {
            ordinal,
            event_seq: None,
            bytes: vec![0; bytes],
        });
        session.log_bytes += bytes;
        host.egress_bytes += bytes;
    }

    /// D-9: a page that overflows the per-peer byte cap (one frame, far under
    /// the frame cap) restages a snapshot; one that fits is logged.
    #[test]
    fn a_page_over_the_byte_cap_restages() {
        let cap = QueueCaps::v1_egress().per_peer_bytes;
        // Control: room for the page.
        let (mut host, peer) = rt3_v3_host();
        host.sessions.get_mut(&peer).expect("bound").in_flight = None;
        stuff_log(&mut host, peer, cap - 1_000);
        host.tick().expect("ticks");
        assert_eq!(
            host.transfer_state(peer),
            Ok(Some(TransferState {
                next_transfer: 2,
                in_flight: None,
            }))
        );
        assert_eq!(host.sessions.get(&peer).expect("bound").log.len(), 2);
        // Overflow by bytes alone.
        let (mut host, peer) = rt3_v3_host();
        host.sessions.get_mut(&peer).expect("bound").in_flight = None;
        stuff_log(&mut host, peer, cap - 10);
        host.tick().expect("ticks");
        assert_eq!(
            host.transfer_state(peer),
            Ok(Some(TransferState {
                next_transfer: 3,
                in_flight: Some(2),
            }))
        );
        let entries = host.take_delivery_entries(peer).expect("bound");
        assert_eq!(entries.len(), 3);
        assert!(matches!(
            transfer_v3::decode_down(&entries[0].bytes),
            Ok(TransferDown::Begin(begin)) if begin.transfer == TransferId::new(2).unwrap()
        ));
    }

    /// D-9: an overflow while a transfer is in flight cancels it, with the
    /// `Superseded` cause (not `Aborted`, which a narrowing uses), then stages
    /// the replacement.
    #[test]
    fn an_overflow_with_a_transfer_in_flight_cancels_it_superseded() {
        let cap = QueueCaps::v1_egress().per_peer_bytes;
        let (mut host, peer) = rt3_v3_host();
        assert_eq!(host.sessions.get(&peer).expect("bound").in_flight, Some(1));
        let epoch = host.epoch(peer).expect("bound");
        stuff_log(&mut host, peer, cap - 10);
        // The throttle holds nine ticks (nothing logged, nothing restaged).
        for _ in 0..9 {
            host.tick().expect("ticks");
        }
        assert_eq!(host.sessions.get(&peer).expect("bound").log.len(), 1);
        assert_eq!(
            host.transfer_state(peer),
            Ok(Some(TransferState {
                next_transfer: 2,
                in_flight: Some(1),
            }))
        );
        // The tenth lets a page through; it does not fit, so it restages.
        host.tick().expect("ticks");
        assert_eq!(
            host.transfer_state(peer),
            Ok(Some(TransferState {
                next_transfer: 3,
                in_flight: Some(2),
            }))
        );
        let entries = host.take_delivery_entries(peer).expect("bound");
        assert_eq!(entries.len(), 4);
        assert_eq!(
            transfer_v3::decode_down(&entries[0].bytes),
            Ok(TransferDown::Cancel {
                epoch,
                transfer: TransferId::new(1).unwrap(),
                reason: CancelReason::Superseded,
            })
        );
        assert!(matches!(
            transfer_v3::decode_down(&entries[1].bytes),
            Ok(TransferDown::Begin(begin)) if begin.transfer == TransferId::new(2).unwrap()
        ));
    }
    // -- T141e: the in-flight throttle (E-2, E5) -------------------------------

    /// The policy constants are the receiver's held-delta budget (T140e), the
    /// throttle spacing is ten ticks and the heartbeat is 100.
    #[test]
    fn t141e_in_flight_policy_equals_the_receiver_limits() {
        let limits = crpg_net::pool_v3::TransferLimitsV3::v3();
        assert_eq!(POLICY_V3_IN_FLIGHT_FRAMES, limits.delta_frames_per_receiver);
        assert_eq!(POLICY_V3_IN_FLIGHT_BYTES, limits.delta_bytes_per_receiver);
        assert_eq!(POLICY_V3_IN_FLIGHT_FRAMES, 128);
        assert_eq!(POLICY_V3_IN_FLIGHT_BYTES, 2_097_152);
        assert_eq!(POLICY_V3_THROTTLE_TICKS, 10);
        assert_eq!(POLICY_CLOCK_HEARTBEAT_TICKS, 100);
    }

    /// E-2: the spacing is exact (9 ticks held, 10 and 11 open) and counts from
    /// the last logged frames; a page with no frames moves nothing.
    #[test]
    fn t141e_throttle_spacing_counts_from_the_last_logged_frames() {
        let mut throttle = Throttle::new(30, 1, 100);
        assert_eq!(throttle.gate(30, 1, 100), Gate::Held);
        assert_eq!(throttle.gate(39, 1, 100), Gate::Held);
        assert_eq!(throttle.gate(40, 1, 100), Gate::Open);
        assert_eq!(throttle.gate(41, 1, 100), Gate::Open);
        // An empty page changes nothing.
        throttle.record(40, 0, 0);
        assert_eq!(throttle.gate(40, 1, 100), Gate::Open);
        // Logged frames restart the spacing from their tick.
        throttle.record(45, 1, 100);
        assert_eq!(throttle.gate(54, 1, 100), Gate::Held);
        assert_eq!(throttle.gate(55, 1, 100), Gate::Open);
        // A staging at tick `s` restarts it from `s`.
        assert_eq!(Throttle::new(500, 1, 100).gate(509, 0, 0), Gate::Held);
        assert_eq!(Throttle::new(500, 1, 100).gate(510, 0, 0), Gate::Open);
    }

    /// E-2: the frame budget holds a page that fits exactly and refuses one
    /// frame more; a closed throttle holds everything.
    #[test]
    fn t141e_frame_budget_is_exact_and_closing_is_final() {
        let throttle = Throttle::new(0, 127, 1_000);
        assert_eq!(throttle.gate(10, 1, 100), Gate::Open);
        assert_eq!(throttle.gate(10, 2, 100), Gate::Over);
        let full = Throttle::new(0, 128, 1_000);
        assert_eq!(full.gate(10, 0, 0), Gate::Open);
        assert_eq!(full.gate(10, 1, 1), Gate::Over);
        let mut closed = Throttle::new(0, 1, 100);
        closed.close();
        assert_eq!(closed.gate(10_000, 0, 0), Gate::Held);
        assert_eq!(closed.gate(10_000, 1, 1), Gate::Held);
    }

    /// E5: with pages of 20,000 bytes the byte budget stops the allowance at
    /// 104 frames (2,080,000 bytes), before the 128-frame budget; the
    /// remaining 17,152 bytes admit a page of exactly that size and not one
    /// byte more.
    #[test]
    fn t141e_byte_budget_stops_before_the_frame_budget() {
        let mut throttle = Throttle::new(0, 0, 0);
        let mut tick = 0;
        let mut pages = 0;
        loop {
            tick += POLICY_V3_THROTTLE_TICKS;
            if throttle.gate(tick, 1, 20_000) != Gate::Open {
                break;
            }
            throttle.record(tick, 1, 20_000);
            pages += 1;
        }
        assert_eq!(pages, 104);
        assert_eq!(throttle.gate(tick, 1, 20_000), Gate::Over);
        assert_eq!(throttle.gate(tick, 1, 17_152), Gate::Open);
        assert_eq!(throttle.gate(tick, 1, 17_153), Gate::Over);
        // Control: pages of 100 bytes reach the frame budget instead.
        let mut small = Throttle::new(0, 0, 0);
        let mut small_pages = 0;
        let mut tick = 0;
        loop {
            tick += POLICY_V3_THROTTLE_TICKS;
            if small.gate(tick, 1, 100) != Gate::Open {
                break;
            }
            small.record(tick, 1, 100);
            small_pages += 1;
        }
        assert_eq!(small_pages, 128);
    }
}
