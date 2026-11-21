//! The real-transport adapter (T023b, ADR-0026): one [`Host`] plus one
//! `crpg_net_quic::QuicServer`, driven by a synchronous, non-blocking
//! [`QuicHost::pump`].
//!
//! A QUIC connection confers no gameplay identity until a hello presents a
//! registered [`Invitation`] credential (checked in constant time, before
//! the wire version) and the host binds it. The invitation is the
//! principal: it carries the control and disclosure grants, so a reconnect
//! never brings back revoked disclosure. From then on the connection's
//! inbound frames enter the same `ingest` → `pump` admission as in-memory
//! peers, and the binding's delivery log is handed to the transport on each
//! entry's channel and acknowledged by ordinal when `try_send_channel`
//! accepts it.
//!
//! Each pump runs fixed phases (events, drain, admit, reconcile, deliver,
//! retire, report) and visits connections in ascending `ConnectionId`
//! order. Every host call it makes is recorded in
//! [`QuicPumpReport::calls`], so the host's transitions can be replayed
//! against an in-memory host (the equivalence proof in `tests/host_quic.rs`).
//!
//! This module reads no clock, spawns no thread, opens no socket and never
//! pauses: `now_ms` is injected by the caller, and the socket, runtime and
//! I/O thread belong to the `QuicServer`. Every transport call made by the
//! pump is non-blocking; `wait`, `shutdown` and dropping the adapter are the
//! only blocking operations, and only the caller makes them. The exact
//! contract is `tasks/T023b.md` (B§1–B§18 as amended by A§0–A§7).

use std::collections::BTreeMap;
use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;

use crpg_core::EntityId;
use crpg_net::codec_resume;
use crpg_net::protocol::{ReceiptStatus, RejectionCode, POLICY_INGRESS_FRAMES_PER_PEER};
use crpg_net::protocol_resume::{
    ConnectionFence, ResumeDown, ResumeProof, ResumeRejection, ResumeResult,
};
use crpg_net::protocol_v3::{PauseRights, RejectionCode as CodeV3};
use crpg_net_quic::{
    Channel, ChannelSendError, CloseCode, CloseMode, CloseReason, ConnectionId, ConnectionStats,
    Credential, Framing, QueueTotals, QuicServer, SendError, ServerError, ServerEvent,
    ServerNotice, ShutdownReport, Welcome, MOVEMENT_WIRE_VERSION,
};

use crate::checkpoint::CheckpointError;
use crate::host::{
    is_narrowing, normalize_control, normalize_fact_grants, normalize_grants, AdmissionResult,
    AdvanceReport, AiBudget, AiReport, ControlGrant, DeliveryKind, DisclosureGrants, FactGrants,
    GrantUpdate, Host, HostError, IngestDisposition, MoveAdmission, PeerHandle, PumpSummary,
    ResumeAnswer, TransferDisposition, TransferRefusal,
};

// ---------------------------------------------------------------------------
// Constants and identities (B§3.1).
// ---------------------------------------------------------------------------

/// Exact v1 invitation credential length (256 bits), inside ADR-0025's
/// 16..=128 transport bounds.
pub const INVITATION_CREDENTIAL_BYTES: usize = 32;
/// Registered invitations per adapter (live and unbound together).
pub const MAX_INVITATIONS: usize = 64;
/// Frames taken from one connection by one pump (references
/// [`POLICY_INGRESS_FRAMES_PER_PEER`](crpg_net::protocol::POLICY_INGRESS_FRAMES_PER_PEER), 128).
pub const MAX_DRAIN_PER_CONNECTION: usize = POLICY_INGRESS_FRAMES_PER_PEER;
/// Injected-time budget for a connection whose transport queue refuses
/// delivery before it is fenced as a slow consumer.
pub const STALL_TIMEOUT_MS: u64 = 10_000;

// The transport's lane-1 outbound frame cap must hold one host take of
// receipts (T037's `POLICY_EGRESS_FRAMES_PER_PEER`, Reading 12 of T150).
const _: () = assert!(
    crpg_net_quic::LANE1_OUTBOUND_FRAMES >= crpg_net::protocol::POLICY_EGRESS_FRAMES_PER_PEER
);

/// Never reused within one `QuicHost`; starts at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct InvitationId(u64);

impl InvitationId {
    /// The raw id.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// One operator-provisioned invitation (D03). `Debug` shows the credential
/// only as `Credential(<redacted>)` (its own `Debug`).
#[derive(Debug, Clone)]
pub struct Invitation {
    /// Exactly [`INVITATION_CREDENTIAL_BYTES`] bytes of operator secret.
    pub credential: Credential,
    /// The actors this principal may act as.
    pub control: ControlGrant,
    /// What this principal may learn.
    pub grants: DisclosureGrants,
}

/// An operator-provisioned v4 principal (Reading 11): an [`Invitation`]'s
/// fields plus what `Host::bind_v4_peer` needs. `Debug` shows the
/// credential only as `Credential(<redacted>)`.
#[derive(Debug, Clone)]
pub struct V4Invitation {
    /// Exactly [`INVITATION_CREDENTIAL_BYTES`] bytes of operator secret.
    pub credential: Credential,
    /// The actors this principal may act as.
    pub control: ControlGrant,
    /// What this principal may learn.
    pub grants: DisclosureGrants,
    /// The v3/v4 fact grants.
    pub facts: FactGrants,
    /// The principal's own pause rights.
    pub rights: PauseRights,
}

// ---------------------------------------------------------------------------
// Reports (B§3.3).
// ---------------------------------------------------------------------------

/// What one `pump` did, in call order. Bounded per pump: at most
/// `max_connections` hello decisions and closes, at most
/// `MAX_PEERS × MAX_DRAIN_PER_CONNECTION` ingest calls plus one held retry
/// per peer, plus at most `MAX_DRAIN_PER_CONNECTION` lane-1 ingests and one
/// held lane-1 retry per movement binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuicPumpReport {
    /// Every host call this pump made, in order: the equivalence trace.
    pub calls: Vec<HostCall>,
    /// Every lane-1 host call this pump made, in order, with its place in
    /// `calls` (T150c Reading 13).
    pub lane1: Vec<Lane1Trace>,
    /// One per `HelloReceived` polled.
    pub hellos: Vec<HelloDecision>,
    /// Bindings fenced by this pump.
    pub fences: Vec<Fence>,
    /// `ServerEvent::Closed` events polled (bound, retiring or unmapped).
    pub transport_closes: Vec<(ConnectionId, CloseReason)>,
    /// Delivery entries accepted by the transport, on any channel.
    pub frames_sent: usize,
    /// Inbound frames dropped after a terminal ingest refusal.
    pub frames_refused: usize,
    /// Frames discarded, on any channel: drained from retiring connections,
    /// held frames of fenced or closed bindings, and delivery entries for
    /// closed transports.
    pub frames_discarded: usize,
    /// Bound connections holding a refused frame when the pump ended.
    pub held: usize,
    /// Receipts accepted by `try_send_lane1` this pump.
    pub lane1_sent: usize,
    /// Samples accepted by `try_send_datagram` this pump.
    pub datagrams_sent: usize,
    /// Lane-1 frames dropped after a terminal `ingest_move` refusal.
    pub lane1_refused: usize,
    /// Lane-1 frames, receipts and samples discarded for retiring, fenced
    /// or closed connections.
    pub lane1_discarded: usize,
    /// Bound connections holding a refused lane-1 frame when the pump ended.
    pub lane1_held: usize,
    /// One per `ServerNotice::Resume` polled, in poll order.
    pub resumes: Vec<ResumeReport>,
}

impl QuicPumpReport {
    fn new() -> Self {
        Self {
            calls: Vec::new(),
            lane1: Vec::new(),
            hellos: Vec::new(),
            fences: Vec::new(),
            transport_closes: Vec::new(),
            frames_sent: 0,
            frames_refused: 0,
            frames_discarded: 0,
            held: 0,
            lane1_sent: 0,
            datagrams_sent: 0,
            lane1_refused: 0,
            lane1_discarded: 0,
            lane1_held: 0,
            resumes: Vec::new(),
        }
    }

    /// Every rejection in this report, in the order of B-1: refused hellos,
    /// refused resume openings, then the `calls` in order (T040a). This is
    /// the only place a rejected client message is defined (Reading 5).
    pub fn rejections(&self) -> Vec<Rejection> {
        let mut out = Vec::new();
        for decision in &self.hellos {
            if let HelloOutcome::Refused { code } = &decision.outcome {
                out.push(Rejection::Hello {
                    conn: decision.conn,
                    code: *code,
                });
            }
        }
        for resume in &self.resumes {
            if let ResumeOutcome::Refused { code } = &resume.outcome {
                out.push(Rejection::Resume {
                    conn: resume.conn,
                    code: *code,
                });
            }
        }
        for call in &self.calls {
            match call {
                HostCall::Ingest {
                    conn,
                    peer,
                    frame,
                    len,
                    result:
                        IngestDisposition::Refused {
                            status: ReceiptStatus::Rejected(code),
                        },
                } if *code != RejectionCode::QueueFull => out.push(Rejection::Ingest {
                    conn: *conn,
                    peer: *peer,
                    frame: *frame,
                    len: *len,
                    code: *code,
                }),
                HostCall::IngestTransfer {
                    conn,
                    peer,
                    frame,
                    len,
                    result: TransferDisposition::Refused(refusal),
                } => out.push(Rejection::Transfer {
                    conn: *conn,
                    peer: *peer,
                    frame: *frame,
                    len: *len,
                    refusal: *refusal,
                }),
                HostCall::CheckFrame {
                    conn,
                    peer,
                    result: Err(error),
                    ..
                } => out.push(Rejection::Frame {
                    conn: *conn,
                    peer: *peer,
                    error: error.clone(),
                }),
                HostCall::Pump { summary } => {
                    for result in &summary.results {
                        if let ReceiptStatus::Rejected(code) = result.status {
                            if !result.retained_in_ingress {
                                out.push(Rejection::Admission {
                                    peer: result.peer,
                                    seq: result.seq,
                                    code,
                                    cached: result.cached,
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// One rejected client message, read from a pump report (Reading 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    /// A hello refused with `code`.
    Hello {
        /// The connection the hello arrived on.
        conn: ConnectionId,
        /// The close code it was refused with.
        code: CloseCode,
    },
    /// A resume opening refused with `code`.
    Resume {
        /// The connection the resume arrived on.
        conn: ConnectionId,
        /// The close code it was refused with.
        code: CloseCode,
    },
    /// A lane-0 frame `ingest` refused (never `QueueFull`).
    Ingest {
        /// The connection the frame came from.
        conn: ConnectionId,
        /// Its binding.
        peer: PeerHandle,
        /// The frame's ordinal on the connection.
        frame: u64,
        /// The frame's length in bytes.
        len: usize,
        /// The rejection code `ingest` returned.
        code: RejectionCode,
    },
    /// A transfer-channel frame `ingest_transfer` refused.
    Transfer {
        /// The connection the frame came from.
        conn: ConnectionId,
        /// Its binding.
        peer: PeerHandle,
        /// The frame's ordinal on the connection.
        frame: u64,
        /// The frame's length in bytes.
        len: usize,
        /// Why `ingest_transfer` refused it.
        refusal: TransferRefusal,
    },
    /// A command the admission pump finalized as rejected.
    Admission {
        /// The submitting binding.
        peer: PeerHandle,
        /// The command's sequence, when decoding got that far.
        seq: Option<u64>,
        /// The rejection code.
        code: RejectionCode,
        /// Answered from the retry cache without re-execution.
        cached: bool,
    },
    /// A frame `check_frame` refused.
    Frame {
        /// The connection the frame arrived on.
        conn: ConnectionId,
        /// Its binding.
        peer: PeerHandle,
        /// What `check_frame` returned.
        error: HostError,
    },
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hello { conn, code } => {
                write!(f, "hello conn={} code={}", conn.get(), *code as u32)
            }
            Self::Resume { conn, code } => {
                write!(f, "resume conn={} code={}", conn.get(), *code as u32)
            }
            Self::Ingest {
                conn,
                peer,
                frame,
                len,
                code,
            } => write!(
                f,
                "ingest conn={} peer={} frame={} len={} code={}",
                conn.get(),
                peer.get(),
                frame,
                len,
                *code as u8
            ),
            Self::Transfer {
                conn,
                peer,
                frame,
                len,
                refusal,
            } => write!(
                f,
                "transfer conn={} peer={} frame={} len={} refusal={:?}",
                conn.get(),
                peer.get(),
                frame,
                len,
                refusal
            ),
            Self::Admission {
                peer,
                seq,
                code,
                cached,
            } => {
                write!(f, "admission peer={} seq=", peer.get())?;
                match seq {
                    Some(seq) => write!(f, "{seq}")?,
                    None => f.write_str("-")?,
                }
                write!(f, " code={} cached={}", *code as u8, cached)
            }
            Self::Frame { conn, peer, error } => write!(
                f,
                "frame conn={} peer={} error={}",
                conn.get(),
                peer.get(),
                error
            ),
        }
    }
}

/// One host call, with what it returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCall {
    /// `Host::bind_peer` for a hello presenting `invitation` on a
    /// `crpg-lane0/1` connection. On `/2`, `Host::bind_snapshot_peer` and its
    /// `BindSnapshotPeer` call.
    BindPeer {
        /// The connection the hello arrived on.
        conn: ConnectionId,
        /// The matched invitation.
        invitation: InvitationId,
        /// What `bind_peer` returned.
        result: Result<PeerHandle, HostError>,
    },
    /// `Host::bind_snapshot_peer` for a hello on a `crpg-lane0/2`
    /// connection.
    BindSnapshotPeer {
        /// The connection the hello arrived on.
        conn: ConnectionId,
        /// The matched invitation.
        invitation: InvitationId,
        /// What `bind_snapshot_peer` returned.
        result: Result<PeerHandle, HostError>,
    },
    /// `Host::unbind_peer`.
    UnbindPeer {
        /// The unbound handle.
        peer: PeerHandle,
    },
    /// `Host::ingest`. `frame` is the 1-based ordinal of the frame on its
    /// connection, counted over every channel in `try_recv_channel` order; a
    /// held frame's retries repeat its ordinal.
    Ingest {
        /// The connection the frame came from.
        conn: ConnectionId,
        /// Its binding.
        peer: PeerHandle,
        /// The frame's ordinal on the connection.
        frame: u64,
        /// The frame's length in bytes.
        len: usize,
        /// What `ingest` returned.
        result: IngestDisposition,
    },
    /// `Host::ingest_transfer`. `frame` is the 1-based ordinal of the frame
    /// on its connection, counted over every channel in `try_recv_channel`
    /// order.
    IngestTransfer {
        /// The connection the frame came from.
        conn: ConnectionId,
        /// Its binding.
        peer: PeerHandle,
        /// The frame's ordinal on the connection.
        frame: u64,
        /// The frame's length in bytes.
        len: usize,
        /// What `ingest_transfer` returned.
        result: TransferDisposition,
    },
    /// `Host::pump`.
    Pump {
        /// Its summary.
        summary: PumpSummary,
    },
    /// `Host::take_delivery_entries`. `Ok(n)`: it returned `n` entries.
    TakeDelivery {
        /// The binding.
        peer: PeerHandle,
        /// The entry count, or the error.
        result: Result<usize, HostError>,
    },
    /// `Host::acknowledge_delivery`.
    AcknowledgeDelivery {
        /// The binding.
        peer: PeerHandle,
        /// The acknowledged delivery ordinal.
        through: u64,
    },
    /// `Host::issue_resume_ticket` (T025b-q N-3). The ticket bytes are never
    /// traced.
    IssueResumeTicket {
        /// The binding the ticket belongs to.
        peer: PeerHandle,
        /// What `issue_resume_ticket` returned.
        result: Result<(), HostError>,
    },
    /// `Host::check_frame` (T025b-q N-4).
    CheckFrame {
        /// The connection the frame arrived on.
        conn: ConnectionId,
        /// Its binding.
        peer: PeerHandle,
        /// The connection's fence tag.
        fence: ConnectionFence,
        /// What `check_frame` returned.
        result: Result<(), HostError>,
    },
    /// `Host::begin_grace` (T025b-q N-7).
    BeginGrace {
        /// The disconnected binding.
        peer: PeerHandle,
        /// The grace deadline, or the error.
        result: Result<u64, HostError>,
    },
    /// `Host::resume_peer` (T025b-q N-5). Never carries proof or credential
    /// bytes.
    ResumePeer {
        /// The connection the resume arrived on.
        conn: ConnectionId,
        /// What `resume_peer` decided.
        result: Result<ResumeAnswer, HostError>,
    },
}

/// One lane-1 host call (T150c), with what it returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lane1Call {
    /// `Host::bind_v4_peer` for a `crpg-lane0/3` hello presenting
    /// `invitation`.
    BindV4Peer {
        /// The connection the hello arrived on.
        conn: ConnectionId,
        /// The matched invitation.
        invitation: InvitationId,
        /// What `bind_v4_peer` returned.
        result: Result<PeerHandle, HostError>,
    },
    /// `Host::ingest_move`. `frame` is the 1-based ordinal of the frame on
    /// the connection's lane-1 stream; a held frame's retries repeat it.
    IngestMove {
        /// The connection the frame came from.
        conn: ConnectionId,
        /// Its binding.
        peer: PeerHandle,
        /// The frame's ordinal on the connection's lane-1 stream.
        frame: u64,
        /// The frame's length in bytes.
        len: usize,
        /// What `ingest_move` returned.
        result: MoveAdmission,
    },
    /// `Host::take_move_receipts`. `Ok(n)`: it returned `n` receipts.
    TakeMoveReceipts {
        /// The binding.
        peer: PeerHandle,
        /// The receipt count, or the error.
        result: Result<usize, HostError>,
    },
    /// `Host::take_samples`. `Ok(n)`: it returned `n` datagrams.
    TakeSamples {
        /// The binding.
        peer: PeerHandle,
        /// The datagram count, or the error.
        result: Result<usize, HostError>,
    },
}

/// A lane-1 call and its place in the pump's call order (Reading 13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane1Trace {
    /// `calls.len()` when the call was made: it came after `calls[at - 1]`
    /// and before `calls[at]`; equal `at`s keep list order.
    pub at: usize,
    /// The call.
    pub call: Lane1Call,
}

/// The decision made for one `HelloReceived`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelloDecision {
    /// The connection the hello arrived on.
    pub conn: ConnectionId,
    /// What happened to it.
    pub outcome: HelloOutcome,
}

/// How a hello ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelloOutcome {
    /// Bound and welcomed.
    Accepted {
        /// The matched invitation.
        invitation: InvitationId,
        /// The new binding.
        peer: PeerHandle,
    },
    /// The close code passed to `QuicServer::refuse` (5, 6, 7 or 1).
    Refused {
        /// The close code.
        code: CloseCode,
    },
    /// `accept` failed because the connection had already closed.
    Lost {
        /// The transport's close reason.
        reason: CloseReason,
    },
}

/// One fenced binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fence {
    /// The connection closed with `SessionFenced`.
    pub conn: ConnectionId,
    /// The binding that ended.
    pub peer: PeerHandle,
    /// Why.
    pub cause: FenceCause,
}

/// Why a binding was fenced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenceCause {
    /// `disconnect`.
    Disconnected,
    /// `revoke_invitation`.
    Revoked,
    /// Narrowing `set_invitation_grants`.
    Narrowed,
    /// A newer connection presented the same invitation.
    Superseded,
    /// The host retired the binding itself (`SeqConflict`), or a host
    /// delivery call failed for it.
    EpochClosed,
    /// Delivery refused by the transport for [`STALL_TIMEOUT_MS`].
    SlowConsumer,
    /// A resume moved the session to another connection; the binding did not
    /// end (T025b-q).
    Resumed,
}

/// The decision made for one `ServerNotice::Resume` (T025b-q).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeReport {
    /// The connection the resume arrived on.
    pub conn: ConnectionId,
    /// How it ended.
    pub outcome: ResumeOutcome,
}

/// How a resume opening ended (T025b-q).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeOutcome {
    /// Answered `Accepted`; the connection now holds `granted`.
    Accepted {
        /// The matched invitation.
        invitation: InvitationId,
        /// The resumed handle.
        peer: PeerHandle,
        /// The fence the new connection holds.
        granted: ConnectionFence,
    },
    /// Refused with this close code (5 or 7).
    Refused {
        /// The close code.
        code: CloseCode,
    },
    /// Accepted by the host, but the connection was gone (Reading 19).
    Lost {
        /// The transport's close reason.
        reason: CloseReason,
    },
}

/// What `set_invitation_grants` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantChange {
    /// No live binding: stored for the next bind.
    Stored,
    /// Widening applied to the live binding (`GrantUpdate::Unchanged`).
    Widened,
    /// Narrowing: stored, and the live binding fenced.
    Fenced(Fence),
    /// Narrowing applied to a live `crpg-lane0/2` binding: stored and a
    /// snapshot staged (`GrantUpdate::SnapshotStaged`); no fence.
    SnapshotStaged,
}

/// What `shutdown` did (B§9).
#[derive(Debug)]
pub struct QuicHostShutdown {
    /// The closed host: authority, captures and save remain available.
    pub host: Host,
    /// `Host::shutdown`'s refusals of staged work (`SessionExpired`, FIFO).
    pub refused: Vec<AdmissionResult>,
    /// Adapter-held frames never staged.
    pub held_discarded: usize,
    /// Host delivery entries the transport could not take before shutdown,
    /// both channels counted.
    pub undelivered_frames: usize,
    /// Lane-1 receipts the transport could not take before shutdown.
    pub undelivered_receipts: usize,
    /// `Some` when requested: `save_checkpoint` taken after the host fence
    /// and before the transport closes.
    pub checkpoint: Option<Result<Vec<u8>, CheckpointError>>,
    /// `QuicServer::shutdown`'s report.
    pub transport: ShutdownReport,
}

/// `start` failure: everything handed in comes back.
#[derive(Debug)]
pub struct StartFailure {
    /// The host, unchanged.
    pub host: Host,
    /// The server, unchanged.
    pub server: QuicServer,
    /// Why `start` refused.
    pub error: QuicHostError,
}

// ---------------------------------------------------------------------------
// Error (B§3.4).
// ---------------------------------------------------------------------------

/// Every adapter failure. `Display` is `<VariantName> at host/quic`; no
/// source. A credential never appears in any variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuicHostError {
    /// `start` was given a shut-down host.
    HostClosed,
    /// `start` was given a host with a binding or staged ingress.
    HostInUse,
    /// `now_ms` below the last accepted `now_ms`.
    TimeRegression,
    /// An earlier host failure poisoned the adapter; only `shutdown` remains.
    Poisoned,
    /// Credential length is not [`INVITATION_CREDENTIAL_BYTES`].
    InvalidCredential,
    /// The credential equals a registered one.
    DuplicateCredential,
    /// [`MAX_INVITATIONS`] are registered.
    InvitationLimit,
    /// No such registered invitation.
    UnknownInvitation,
    /// No such bound connection.
    UnknownConnection,
    /// A host error passed through.
    Host(HostError),
}

impl fmt::Display for QuicHostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::HostClosed => "HostClosed",
            Self::HostInUse => "HostInUse",
            Self::TimeRegression => "TimeRegression",
            Self::Poisoned => "Poisoned",
            Self::InvalidCredential => "InvalidCredential",
            Self::DuplicateCredential => "DuplicateCredential",
            Self::InvitationLimit => "InvitationLimit",
            Self::UnknownInvitation => "UnknownInvitation",
            Self::UnknownConnection => "UnknownConnection",
            Self::Host(_) => "Host",
        };
        write!(f, "{name} at host/quic")
    }
}

impl std::error::Error for QuicHostError {}

// ---------------------------------------------------------------------------
// Private state (B§3.5).
// ---------------------------------------------------------------------------

/// One registered invitation: the credential, normalized grants, and the
/// live connection if bound.
struct InvitationRecord {
    credential: Credential,
    control: ControlGrant,
    grants: DisclosureGrants,
    live: Option<ConnectionId>,
    /// A session held through its grace (T025b-q), keyed by its handle.
    graced: Option<PeerHandle>,
    /// The fact grants and pause rights a `crpg-lane0/3` binding needs;
    /// `None` from `register_invitation` (Reading 11).
    v4: Option<(FactGrants, PauseRights)>,
}

/// A bound connection.
struct Bound {
    peer: PeerHandle,
    epoch: [u8; 16],
    invitation: InvitationId,
    /// The negotiated framing, fixed for the connection's life.
    framing: Framing,
    /// Whether the connection negotiated `crpg-lane0/3`.
    movement: bool,
    /// The connection's fence tag (T025b-q): `FIRST` for a hello binding,
    /// the granted fence for a resume binding.
    fence: ConnectionFence,
    /// The ordinal of the last frame taken with `try_recv_channel`, over
    /// every channel.
    frames: u64,
    /// The ordinal of the last fresh frame taken with `try_recv_lane1`.
    lane1_frames: u64,
    /// One lane-0 frame refused with `QueueFull`, retried before any later
    /// frame.
    held: Option<Vec<u8>>,
    /// One lane-1 frame refused with `QueueFull`, retried before any later
    /// lane-1 frame.
    held_lane1: Option<Vec<u8>>,
    stalled_since: Option<u64>,
    transport_closed: Option<CloseReason>,
    /// `try_recv` reported the end of the inbound stream.
    drained: bool,
    /// `try_recv_lane1` reported the end of the lane-1 stream.
    lane1_drained: bool,
    /// The connection's `Closed` event has been polled.
    event_seen: bool,
}

/// A connection this adapter closed, or whose binding ended, still drained
/// and discarded so the transport can forget its record.
struct Retiring {
    event_seen: bool,
    /// `try_recv_lane1` reported an error for this retiring entry.
    lane1_gone: bool,
}

enum Conn {
    Bound(Bound),
    Retiring(Retiring),
}

/// Host errors that leave the adapter's view of the host uncertain (B§12).
fn poisons(error: &HostError) -> bool {
    matches!(
        error,
        HostError::Invariant(_) | HostError::Closed | HostError::TimeRegression
    )
}

/// N-10 (Question 1 (b)): the session-bound resume proof is the keyed BLAKE3
/// derivation over the invitation credential and the session epoch. The
/// credential must be exactly [`INVITATION_CREDENTIAL_BYTES`] long.
fn resume_proof(credential: &Credential, epoch: [u8; 16]) -> Result<ResumeProof, HostError> {
    let key = credential.as_bytes();
    let key: &[u8; 32] = key
        .try_into()
        .map_err(|_| HostError::Invariant("invitation credential is not 32 bytes"))?;
    let mut message = Vec::with_capacity(b"crpg-resume-proof/1".len() + 16);
    message.extend_from_slice(b"crpg-resume-proof/1");
    message.extend_from_slice(&epoch);
    let digest = blake3::keyed_hash(key, &message);
    ResumeProof::new(digest.as_bytes().to_vec())
        .map_err(|_| HostError::Invariant("resume proof derivation failed"))
}

// ---------------------------------------------------------------------------
// The adapter (B§3.2).
// ---------------------------------------------------------------------------

/// The real-transport adapter: one `Host` plus one `QuicServer`, driven by a
/// synchronous `pump`. Neither `Clone` nor `Copy`. `Debug` is hand-written
/// (local address, binding and invitation counts) and never prints a
/// credential. `Send`; no `Sync` requirement.
///
/// Dropping a `QuicHost` drops the `QuicServer`, whose own `Drop` closes
/// every connection with `Shutdown` in `Discard` mode, and drops the `Host`
/// without a checkpoint.
pub struct QuicHost {
    host: Host,
    server: QuicServer,
    last_now_ms: u64,
    poisoned: bool,
    next_invitation: u64,
    invitations: BTreeMap<InvitationId, InvitationRecord>,
    conns: BTreeMap<ConnectionId, Conn>,
    /// Whether this adapter came from `start_resumable` (T025b-q).
    resumable: bool,
}

impl fmt::Debug for QuicHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bindings = self
            .conns
            .values()
            .filter(|conn| matches!(conn, Conn::Bound(_)))
            .count();
        f.debug_struct("QuicHost")
            .field("local_addr", &self.server.local_addr())
            .field("bindings", &bindings)
            .field("invitations", &self.invitations.len())
            .field("poisoned", &self.poisoned)
            .finish()
    }
}

impl QuicHost {
    /// Takes ownership of a host and a bound server. Precedence:
    /// `HostClosed` → `HostInUse` (any binding or staged ingress) →
    /// `TimeRegression` (`now_ms` below the host's last accepted time).
    /// On failure, both are returned unchanged.
    ///
    /// The server must come from `QuicServer::bind`, `QuicServer::bind_dual`
    /// or `QuicServer::bind_movement`, never `QuicServer::bind_resumable`:
    /// the adapter polls only `poll_event`, which stops for good at a resume
    /// notice, and no public call tells a resumable server apart. T025b lifts
    /// this precondition.
    pub fn start(
        host: Host,
        server: QuicServer,
        now_ms: u64,
    ) -> Result<QuicHost, Box<StartFailure>> {
        Self::start_impl(host, server, now_ms, false)
    }

    /// As [`start`](Self::start) (same precedence and failure), for a server
    /// from any `QuicServer::bind*`, `bind_resumable` included (N-1). The
    /// adapter polls `poll_notice`, so a resumable server's resume openings
    /// are handled, and `/2` hellos are followed by a resume ticket.
    pub fn start_resumable(
        host: Host,
        server: QuicServer,
        now_ms: u64,
    ) -> Result<QuicHost, Box<StartFailure>> {
        Self::start_impl(host, server, now_ms, true)
    }

    /// Whether this adapter came from [`start_resumable`](Self::start_resumable).
    pub fn is_resumable(&self) -> bool {
        self.resumable
    }

    /// The shared constructor: `start`'s precedence (`HostClosed` →
    /// `HostInUse` — any binding, staged ingress or held grace — →
    /// `TimeRegression`), returning both on failure.
    fn start_impl(
        host: Host,
        server: QuicServer,
        now_ms: u64,
        resumable: bool,
    ) -> Result<QuicHost, Box<StartFailure>> {
        let error = if host.is_closed() {
            Some(QuicHostError::HostClosed)
        } else if host.session_count() > 0 || host.has_staged_ingress() {
            Some(QuicHostError::HostInUse)
        } else if now_ms < host.last_now_ms() {
            Some(QuicHostError::TimeRegression)
        } else {
            None
        };
        if let Some(error) = error {
            return Err(Box::new(StartFailure {
                host,
                server,
                error,
            }));
        }
        Ok(QuicHost {
            host,
            server,
            last_now_ms: now_ms,
            poisoned: false,
            next_invitation: 1,
            invitations: BTreeMap::new(),
            conns: BTreeMap::new(),
            resumable,
        })
    }

    /// One serial step (B§5). Errors: `Poisoned`; `TimeRegression` (checked
    /// first, nothing changes); `Host(e)` for an unreachable host failure,
    /// which poisons the adapter (B§12).
    pub fn pump(&mut self, now_ms: u64) -> Result<QuicPumpReport, QuicHostError> {
        // 0. Preconditions.
        if self.poisoned {
            return Err(QuicHostError::Poisoned);
        }
        if now_ms < self.last_now_ms {
            return Err(QuicHostError::TimeRegression);
        }
        self.last_now_ms = now_ms;
        let mut report = QuicPumpReport::new();
        let result = self
            .run_phases(now_ms, &mut report)
            .and_then(|()| self.flush_phases(now_ms, &mut report));
        match result {
            Ok(()) => Ok(report),
            Err(error) => {
                self.poisoned = true;
                Err(QuicHostError::Host(error))
            }
        }
    }

    /// Phases 4–7 of [`pump`](Self::pump) alone (T154d, B-15): reconcile,
    /// deliver, retire closed peers and report, without polling the
    /// transport, draining a connection or admitting. Errors: `Poisoned`;
    /// `TimeRegression` below the adapter's last accepted `now_ms` (nothing
    /// changes); otherwise the phases run and a host error poisons as in
    /// `pump`.
    pub fn flush(&mut self, now_ms: u64) -> Result<QuicPumpReport, QuicHostError> {
        if self.poisoned {
            return Err(QuicHostError::Poisoned);
        }
        if now_ms < self.last_now_ms {
            return Err(QuicHostError::TimeRegression);
        }
        self.last_now_ms = now_ms;
        let mut report = QuicPumpReport::new();
        match self.flush_phases(now_ms, &mut report) {
            Ok(()) => Ok(report),
            Err(error) => {
                self.poisoned = true;
                Err(QuicHostError::Host(error))
            }
        }
    }

    /// `pump`'s phases 1–3: events and notices, drain, admit.
    fn run_phases(&mut self, now_ms: u64, report: &mut QuicPumpReport) -> Result<(), HostError> {
        // 1. Events and notices.
        if self.resumable {
            while let Some(notice) = self.server.poll_notice() {
                match notice {
                    ServerNotice::Event(event) => self.on_event(event, now_ms, report)?,
                    ServerNotice::Resume(received) => {
                        self.on_resume(received.conn, &received.request, now_ms, report)?;
                    }
                }
            }
        } else {
            while let Some(event) = self.server.poll_event() {
                self.on_event(event, now_ms, report)?;
            }
        }
        // 2. Drain.
        let ids: Vec<ConnectionId> = self.conns.keys().copied().collect();
        for conn in ids {
            match self.conns.get(&conn) {
                Some(Conn::Bound(_)) => {
                    self.drain_bound(conn, now_ms, report)?;
                    // C-3: lane 1 drains after lane 0, on the same connection.
                    if self.bound(conn).is_some_and(|bound| bound.movement) {
                        self.drain_lane1(conn, now_ms, report)?;
                    }
                }
                Some(Conn::Retiring(_)) => self.drain_retiring(conn, report),
                None => {}
            }
        }
        // 3. Admit.
        let summary = self.host.pump(now_ms)?;
        report.calls.push(HostCall::Pump { summary });
        Ok(())
    }

    /// `pump`'s phases 4–7: reconcile, deliver, retire closed peers, report.
    /// Shared body with [`flush`](Self::flush).
    fn flush_phases(&mut self, now_ms: u64, report: &mut QuicPumpReport) -> Result<(), HostError> {
        // 4. Reconcile.
        for conn in self.bound_ids() {
            let Some((peer, epoch)) = self.bound(conn).map(|bound| (bound.peer, bound.epoch))
            else {
                continue;
            };
            if self.host.epoch(peer) != Ok(epoch) {
                self.fence(conn, FenceCause::EpochClosed, Some(report));
            }
        }
        // N-6: a grace the host swept away is no longer held (read-only,
        // untraced).
        if self.resumable {
            let swept: Vec<InvitationId> = self
                .invitations
                .iter()
                .filter_map(|(id, record)| {
                    let peer = record.graced?;
                    (self.host.grace_deadline(peer).is_none()).then_some(*id)
                })
                .collect();
            for id in swept {
                if let Some(record) = self.invitations.get_mut(&id) {
                    record.graced = None;
                }
            }
        }
        // 5. Deliver.
        for conn in self.bound_ids() {
            self.deliver(conn, now_ms, report)?;
            // C-4/C-5: lane-1 receipts and samples, after lane 0's delivery.
            if self.bound(conn).is_some_and(|bound| bound.movement) {
                self.deliver_lane1(conn, report)?;
            }
        }
        // 6. Retire closed peers.
        for conn in self.bound_ids() {
            let Some(bound) = self.bound(conn) else {
                continue;
            };
            let lane1_pending =
                bound.movement && (!bound.lane1_drained || bound.held_lane1.is_some());
            if bound.transport_closed.is_none() || !bound.drained || bound.held.is_some() {
                continue;
            }
            if lane1_pending {
                continue;
            }
            let (peer, invitation, event_seen, framing, reason) = (
                bound.peer,
                bound.invitation,
                bound.event_seen,
                bound.framing,
                bound.transport_closed.expect("checked above"),
            );
            // N-7: a silent loss of a `/2` connection starts a grace; every
            // other binding unbinds exactly as today.
            let silent_loss = self.resumable
                && framing == Framing::V2
                && matches!(
                    reason,
                    CloseReason::IdleTimeout | CloseReason::Transport | CloseReason::Reset
                );
            if silent_loss {
                let result = self.host.begin_grace(peer, now_ms);
                report.calls.push(HostCall::BeginGrace {
                    peer,
                    result: result.clone(),
                });
                match result {
                    Ok(_deadline) => {
                        if let Some(record) = self.invitations.get_mut(&invitation) {
                            record.graced = Some(peer);
                        }
                    }
                    Err(HostError::NotResumable) => {
                        self.host.unbind_peer(peer);
                        report.calls.push(HostCall::UnbindPeer { peer });
                    }
                    Err(error) => return Err(error),
                }
            } else {
                self.host.unbind_peer(peer);
                report.calls.push(HostCall::UnbindPeer { peer });
            }
            self.clear_live(invitation, conn);
            if event_seen {
                self.conns.remove(&conn);
            } else {
                self.conns.insert(
                    conn,
                    Conn::Retiring(Retiring {
                        event_seen: false,
                        lane1_gone: false,
                    }),
                );
            }
        }
        // 7. Report.
        report.held = self
            .conns
            .values()
            .filter(|entry| matches!(entry, Conn::Bound(bound) if bound.held.is_some()))
            .count();
        report.lane1_held = self
            .conns
            .values()
            .filter(|entry| matches!(entry, Conn::Bound(bound) if bound.held_lane1.is_some()))
            .count();
        Ok(())
    }

    /// One `ServerEvent`, as phase 1 polled it.
    fn on_event(
        &mut self,
        event: ServerEvent,
        now_ms: u64,
        report: &mut QuicPumpReport,
    ) -> Result<(), HostError> {
        match event {
            ServerEvent::HelloReceived { conn, hello, .. } => {
                self.on_hello(conn, hello.wire_version, &hello.credential, now_ms, report)
            }
            ServerEvent::Closed { conn, reason } => {
                match self.conns.get_mut(&conn) {
                    Some(Conn::Bound(bound)) => {
                        bound.transport_closed.get_or_insert(reason);
                        bound.event_seen = true;
                    }
                    Some(Conn::Retiring(retiring)) => retiring.event_seen = true,
                    None => {}
                }
                report.transport_closes.push((conn, reason));
                Ok(())
            }
        }
    }

    /// N-5: one `ServerNotice::Resume`, in exact order.
    fn on_resume(
        &mut self,
        conn: ConnectionId,
        request_bytes: &[u8],
        now_ms: u64,
        report: &mut QuicPumpReport,
    ) -> Result<(), HostError> {
        // (1) Decode; a malformed request is refused with no host call.
        let request = match codec_resume::decode_request(request_bytes) {
            Ok(request) => request,
            Err(_) => {
                self.refuse_resume(conn, CloseCode::AuthRefused, report);
                return Ok(());
            }
        };
        // (2) Decide.
        let result = self.host.resume_peer(&request, now_ms);
        report.calls.push(HostCall::ResumePeer {
            conn,
            result: result.clone(),
        });
        let answer = match result {
            Ok(answer) => answer,
            Err(error) => {
                self.refuse_resume(conn, CloseCode::Shutdown, report);
                return Err(error);
            }
        };
        match answer {
            // (3) Refused: hang up at once (Question 2 (b)).
            ResumeAnswer::Rejected(rejected) => {
                let code = match rejected.reason {
                    ResumeRejection::Denied => CloseCode::AuthRefused,
                    ResumeRejection::Busy => CloseCode::ServerBusy,
                };
                self.refuse_resume(conn, code, report);
            }
            // (4) Accepted.
            ResumeAnswer::Accepted { peer, accepted } => {
                // The invitation: a bound connection whose peer matches,
                // else the record whose grace holds it.
                let mut old_conn: Option<ConnectionId> = None;
                let mut invitation: Option<InvitationId> = None;
                for (c, entry) in &self.conns {
                    if let Conn::Bound(bound) = entry {
                        if bound.peer == peer {
                            old_conn = Some(*c);
                            invitation = Some(bound.invitation);
                            break;
                        }
                    }
                }
                if invitation.is_none() {
                    invitation = self
                        .invitations
                        .iter()
                        .find_map(|(id, record)| (record.graced == Some(peer)).then_some(*id));
                }
                let invitation =
                    invitation.ok_or(HostError::Invariant("resumed session has no invitation"))?;
                // H5: retire the old connection before the answer.
                if let Some(old) = old_conn {
                    self.retire_resumed(old, Some(report));
                }
                let answer = codec_resume::encode_down(&ResumeDown::Answer(
                    ResumeResult::Accepted(accepted.clone()),
                ))
                .map_err(|_| HostError::Invariant("resume answer failed to encode"))?;
                let bound = Bound {
                    peer,
                    epoch: accepted.epoch,
                    invitation,
                    framing: Framing::V2,
                    movement: false,
                    fence: accepted.granted,
                    frames: 0,
                    lane1_frames: 0,
                    held: None,
                    held_lane1: None,
                    stalled_since: None,
                    transport_closed: None,
                    drained: false,
                    lane1_drained: false,
                    event_seen: false,
                };
                match self.server.accept_resume(conn, &answer) {
                    Ok(()) => {
                        self.conns.insert(conn, Conn::Bound(bound));
                        if let Some(record) = self.invitations.get_mut(&invitation) {
                            record.live = Some(conn);
                            record.graced = None;
                        }
                        report.resumes.push(ResumeReport {
                            conn,
                            outcome: ResumeOutcome::Accepted {
                                invitation,
                                peer,
                                granted: accepted.granted,
                            },
                        });
                    }
                    Err(ServerError::Closed(reason)) => {
                        self.conns.insert(
                            conn,
                            Conn::Bound(Bound {
                                transport_closed: Some(reason),
                                ..bound
                            }),
                        );
                        if let Some(record) = self.invitations.get_mut(&invitation) {
                            record.live = Some(conn);
                            record.graced = None;
                        }
                        report.resumes.push(ResumeReport {
                            conn,
                            outcome: ResumeOutcome::Lost { reason },
                        });
                    }
                    Err(_) => {
                        self.host.unbind_peer(peer);
                        report.calls.push(HostCall::UnbindPeer { peer });
                        if let Some(record) = self.invitations.get_mut(&invitation) {
                            record.graced = None;
                        }
                        report.resumes.push(ResumeReport {
                            conn,
                            outcome: ResumeOutcome::Lost {
                                reason: CloseReason::EndpointFailed,
                            },
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// N-5: refuse a resume opening and record its outcome.
    fn refuse_resume(&mut self, conn: ConnectionId, code: CloseCode, report: &mut QuicPumpReport) {
        let _ = self.server.refuse(conn, code);
        report.resumes.push(ResumeReport {
            conn,
            outcome: ResumeOutcome::Refused { code },
        });
    }

    /// B§4.3: one `HelloReceived`, in exact precedence.
    fn on_hello(
        &mut self,
        conn: ConnectionId,
        wire_version: u8,
        credential: &Credential,
        now_ms: u64,
        report: &mut QuicPumpReport,
    ) -> Result<(), HostError> {
        let refuse = |server: &mut QuicServer, report: &mut QuicPumpReport, code: CloseCode| {
            // The connection may already have ended; its `Closed` event is
            // polled and reported later either way.
            let _ = server.refuse(conn, code);
            report.hellos.push(HelloDecision {
                conn,
                outcome: HelloOutcome::Refused { code },
            });
        };
        // 1. Credential (before the version, B§16 Q8).
        let Some(invitation) = self.match_credential(credential) else {
            refuse(&mut self.server, report, CloseCode::AuthRefused);
            return Ok(());
        };
        // 2. The connection's mapping (T150c C-2): a `crpg-lane0/3`
        // connection is only visible until its `Closed` is polled.
        let movement = match self.server.movement(conn) {
            Ok(movement) => movement,
            Err(_) => {
                report.hellos.push(HelloDecision {
                    conn,
                    outcome: HelloOutcome::Lost {
                        reason: CloseReason::EndpointFailed,
                    },
                });
                return Ok(());
            }
        };
        // 3. Version: `/3` carries wire version 4, `/1` and `/2` as today.
        let version_ok = if movement {
            wire_version == MOVEMENT_WIRE_VERSION
        } else {
            wire_version == self.host.protocol().as_u8()
        };
        if !version_ok {
            refuse(&mut self.server, report, CloseCode::VersionRefused);
            return Ok(());
        }
        // 4. A movement hello needs a v4 invitation (Reading 11): no
        // supersession and no bind for one registered without facts.
        if movement
            && self
                .invitations
                .get(&invitation)
                .and_then(|record| record.v4.as_ref())
                .is_none()
        {
            refuse(&mut self.server, report, CloseCode::AuthRefused);
            return Ok(());
        }
        // N-8: a new hello for a graced invitation ends the grace first.
        if let Some(graced) = self
            .invitations
            .get(&invitation)
            .and_then(|record| record.graced)
        {
            self.host.unbind_peer(graced);
            report.calls.push(HostCall::UnbindPeer { peer: graced });
            if let Some(record) = self.invitations.get_mut(&invitation) {
                record.graced = None;
            }
        }
        // 5. Supersession: the newest connection wins (B§16 Q3).
        let live = self
            .invitations
            .get(&invitation)
            .and_then(|record| record.live);
        if let Some(old) = live {
            self.fence(old, FenceCause::Superseded, Some(report));
        }
        // 6. Bind, by the connection's negotiated framing. The record stays
        // visible until its `Closed` is polled, so `framing` cannot fail; if
        // it ever does, the hello is lost with no bind call.
        let framing = if movement {
            // A `/3` connection's lane stream is `/2`'s (T150 Reading 2).
            Framing::V2
        } else {
            match self.server.framing(conn) {
                Ok(framing) => framing,
                Err(_) => {
                    report.hellos.push(HelloDecision {
                        conn,
                        outcome: HelloOutcome::Lost {
                            reason: CloseReason::EndpointFailed,
                        },
                    });
                    return Ok(());
                }
            }
        };
        let record = self
            .invitations
            .get(&invitation)
            .ok_or(HostError::Invariant("matched invitation vanished"))?;
        let result = if movement {
            let (facts, rights) = record.v4.clone().ok_or(HostError::Invariant(
                "a movement hello matched a v3 invitation",
            ))?;
            let result = self.host.bind_v4_peer(
                record.control.clone(),
                record.grants.clone(),
                facts,
                rights,
                now_ms,
            );
            report.lane1.push(Lane1Trace {
                at: report.calls.len(),
                call: Lane1Call::BindV4Peer {
                    conn,
                    invitation,
                    result: result.clone(),
                },
            });
            result
        } else {
            let result = match framing {
                Framing::V2 => self.host.bind_snapshot_peer(
                    record.control.clone(),
                    record.grants.clone(),
                    now_ms,
                ),
                Framing::V1 => {
                    self.host
                        .bind_peer(record.control.clone(), record.grants.clone(), now_ms)
                }
            };
            report.calls.push(match framing {
                Framing::V2 => HostCall::BindSnapshotPeer {
                    conn,
                    invitation,
                    result: result.clone(),
                },
                Framing::V1 => HostCall::BindPeer {
                    conn,
                    invitation,
                    result: result.clone(),
                },
            });
            result
        };
        let peer = match result {
            Ok(peer) => peer,
            Err(HostError::ServerBusy | HostError::QueueFull | HostError::CounterExhausted) => {
                refuse(&mut self.server, report, CloseCode::ServerBusy);
                return Ok(());
            }
            Err(HostError::InvalidControl | HostError::InvalidGrants) => {
                refuse(&mut self.server, report, CloseCode::AuthRefused);
                return Ok(());
            }
            Err(error) => {
                refuse(&mut self.server, report, CloseCode::Shutdown);
                return Err(error);
            }
        };
        // 7. Welcome.
        let epoch = self.host.epoch(peer)?;
        let welcome = Welcome {
            wire_version: if movement {
                MOVEMENT_WIRE_VERSION
            } else {
                self.host.protocol().as_u8()
            },
            epoch,
        };
        // 8. Result.
        match self.server.accept(conn, welcome) {
            Ok(()) => {
                self.conns.insert(
                    conn,
                    Conn::Bound(Bound {
                        peer,
                        epoch,
                        invitation,
                        framing,
                        movement,
                        fence: ConnectionFence::FIRST,
                        frames: 0,
                        lane1_frames: 0,
                        held: None,
                        held_lane1: None,
                        stalled_since: None,
                        transport_closed: None,
                        drained: false,
                        lane1_drained: false,
                        event_seen: false,
                    }),
                );
                if let Some(record) = self.invitations.get_mut(&invitation) {
                    record.live = Some(conn);
                }
                report.hellos.push(HelloDecision {
                    conn,
                    outcome: HelloOutcome::Accepted { invitation, peer },
                });
                // N-3: a `/2` hello is followed by a resume ticket, the first
                // frame after the welcome.
                if self.resumable && framing == Framing::V2 {
                    let proof = {
                        let record = self
                            .invitations
                            .get(&invitation)
                            .ok_or(HostError::Invariant("matched invitation vanished"))?;
                        resume_proof(&record.credential, epoch)?
                    };
                    let result = self.host.issue_resume_ticket(peer, proof);
                    report.calls.push(HostCall::IssueResumeTicket {
                        peer,
                        result: result.as_ref().map(|_| ()).map_err(Clone::clone),
                    });
                    let bytes = result?;
                    if self
                        .server
                        .try_send_channel(conn, Channel::Resume, &bytes)
                        .is_err()
                    {
                        report.frames_discarded += 1;
                    }
                }
            }
            Err(ServerError::Closed(reason)) => {
                self.host.unbind_peer(peer);
                report.calls.push(HostCall::UnbindPeer { peer });
                report.hellos.push(HelloDecision {
                    conn,
                    outcome: HelloOutcome::Lost { reason },
                });
            }
            Err(
                ServerError::UnknownConnection | ServerError::NotPending | ServerError::NotAccepted,
            ) => {
                self.host.unbind_peer(peer);
                report.calls.push(HostCall::UnbindPeer { peer });
                report.hellos.push(HelloDecision {
                    conn,
                    outcome: HelloOutcome::Lost {
                        reason: CloseReason::EndpointFailed,
                    },
                });
            }
            Err(ServerError::InvalidWelcome) => {
                self.host.unbind_peer(peer);
                report.calls.push(HostCall::UnbindPeer { peer });
                refuse(&mut self.server, report, CloseCode::VersionRefused);
            }
        }
        Ok(())
    }

    /// B§4.1: `ct_eq` against every registered credential in ascending id
    /// order, flags folded with `|` and no early exit; the match is selected
    /// after the loop. The length check reads only the public length.
    fn match_credential(&self, credential: &Credential) -> Option<InvitationId> {
        if credential.as_bytes().len() != INVITATION_CREDENTIAL_BYTES {
            return None;
        }
        let flags: Vec<(InvitationId, bool)> = self
            .invitations
            .iter()
            .map(|(id, record)| (*id, credential.ct_eq(record.credential.as_bytes())))
            .collect();
        let any = flags.iter().fold(false, |acc, (_, flag)| acc | *flag);
        if !any {
            return None;
        }
        flags.iter().find(|(_, flag)| *flag).map(|(id, _)| *id)
    }

    /// B§6: the held retry, then up to `MAX_DRAIN_PER_CONNECTION` fresh frames.
    fn drain_bound(
        &mut self,
        conn: ConnectionId,
        now_ms: u64,
        report: &mut QuicPumpReport,
    ) -> Result<(), HostError> {
        // 1. Held retry.
        let held = self.bound_mut(conn).and_then(|bound| bound.held.take());
        if let Some(frame) = held {
            match self.ingest_one(conn, &frame, now_ms, report)? {
                Disposition::Continue => {}
                Disposition::Hold => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound.held = Some(frame);
                    }
                    return Ok(());
                }
                Disposition::Stop => return Ok(()),
            }
        }
        // 2. Fresh frames, on every channel in `try_recv_channel` order.
        for _ in 0..MAX_DRAIN_PER_CONNECTION {
            match self.server.try_recv_channel(conn) {
                Ok(Some((channel, frame))) => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound.frames += 1;
                    }
                    // N-4: fence-tag each fresh frame before routing it (a
                    // held retry is exempt).
                    if self.resumable {
                        let Some((peer, fence)) =
                            self.bound(conn).map(|bound| (bound.peer, bound.fence))
                        else {
                            return Ok(());
                        };
                        let check = self.host.check_frame(peer, fence);
                        report.calls.push(HostCall::CheckFrame {
                            conn,
                            peer,
                            fence,
                            result: check.clone(),
                        });
                        match check {
                            Ok(()) => {}
                            Err(HostError::SessionFenced) => {
                                report.frames_refused += 1;
                                self.retire_resumed(conn, Some(report));
                                return Ok(());
                            }
                            Err(HostError::UnknownPeer) => {
                                report.frames_refused += 1;
                                self.fence(conn, FenceCause::EpochClosed, Some(report));
                                return Ok(());
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    let disposition = match channel {
                        Channel::Lane0 => self.ingest_one(conn, &frame, now_ms, report)?,
                        Channel::Transfer => {
                            self.ingest_transfer_one(conn, &frame, now_ms, report)?
                        }
                        Channel::Resume => {
                            // Unreachable (Q§8.2): the server's reader closes
                            // an upstream 0x91 with `ProtocolViolation` before
                            // queueing it. No host call, no trace; the arm
                            // keeps the match exhaustive with no panic path.
                            report.frames_refused += 1;
                            self.fence(conn, FenceCause::EpochClosed, Some(report));
                            Disposition::Stop
                        }
                    };
                    match disposition {
                        Disposition::Continue => {}
                        Disposition::Hold => {
                            if let Some(bound) = self.bound_mut(conn) {
                                bound.held = Some(frame);
                            }
                            return Ok(());
                        }
                        Disposition::Stop => return Ok(()),
                    }
                }
                Ok(None) => return Ok(()),
                Err(ServerError::Closed(reason)) => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound.transport_closed.get_or_insert(reason);
                        bound.drained = true;
                    }
                    return Ok(());
                }
                Err(ServerError::UnknownConnection) => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound
                            .transport_closed
                            .get_or_insert(CloseReason::EndpointFailed);
                        bound.drained = true;
                    }
                    return Ok(());
                }
                Err(
                    ServerError::NotAccepted
                    | ServerError::NotPending
                    | ServerError::InvalidWelcome,
                ) => {
                    self.fence(conn, FenceCause::EpochClosed, Some(report));
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// C-3: the held lane-1 retry, then up to `MAX_DRAIN_PER_CONNECTION`
    /// fresh lane-1 frames, after the connection's lane-0 drain.
    fn drain_lane1(
        &mut self,
        conn: ConnectionId,
        now_ms: u64,
        report: &mut QuicPumpReport,
    ) -> Result<(), HostError> {
        // 1. Held retry.
        let held = self
            .bound_mut(conn)
            .and_then(|bound| bound.held_lane1.take());
        if let Some(frame) = held {
            match self.ingest_move_one(conn, &frame, now_ms, report)? {
                Disposition::Continue => {}
                Disposition::Hold => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound.held_lane1 = Some(frame);
                    }
                    return Ok(());
                }
                Disposition::Stop => return Ok(()),
            }
        }
        // 2. Fresh frames.
        for _ in 0..MAX_DRAIN_PER_CONNECTION {
            match self.server.try_recv_lane1(conn) {
                Ok(Some(frame)) => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound.lane1_frames += 1;
                    }
                    match self.ingest_move_one(conn, &frame, now_ms, report)? {
                        Disposition::Continue => {}
                        Disposition::Hold => {
                            if let Some(bound) = self.bound_mut(conn) {
                                bound.held_lane1 = Some(frame);
                            }
                            return Ok(());
                        }
                        Disposition::Stop => return Ok(()),
                    }
                }
                Ok(None) => return Ok(()),
                Err(ServerError::Closed(reason)) => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound.transport_closed.get_or_insert(reason);
                        bound.lane1_drained = true;
                    }
                    return Ok(());
                }
                Err(ServerError::UnknownConnection) => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound
                            .transport_closed
                            .get_or_insert(CloseReason::EndpointFailed);
                        bound.lane1_drained = true;
                    }
                    return Ok(());
                }
                Err(
                    ServerError::NotAccepted
                    | ServerError::NotPending
                    | ServerError::InvalidWelcome,
                ) => {
                    self.fence(conn, FenceCause::EpochClosed, Some(report));
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// One traced `ingest_move` and its C-3 disposition.
    fn ingest_move_one(
        &mut self,
        conn: ConnectionId,
        frame: &[u8],
        now_ms: u64,
        report: &mut QuicPumpReport,
    ) -> Result<Disposition, HostError> {
        let Some((peer, ordinal)) = self
            .bound(conn)
            .map(|bound| (bound.peer, bound.lane1_frames))
        else {
            return Ok(Disposition::Stop);
        };
        let result = self.host.ingest_move(peer, frame, now_ms)?;
        report.lane1.push(Lane1Trace {
            at: report.calls.len(),
            call: Lane1Call::IngestMove {
                conn,
                peer,
                frame: ordinal,
                len: frame.len(),
                result,
            },
        });
        Ok(match result {
            MoveAdmission::Admitted { .. }
            | MoveAdmission::Rejected { .. }
            | MoveAdmission::Duplicate { .. } => Disposition::Continue,
            MoveAdmission::Refused {
                code: CodeV3::QueueFull,
                ..
            } => Disposition::Hold,
            MoveAdmission::Refused {
                code: CodeV3::Unauthenticated,
                ..
            } => {
                report.lane1_refused += 1;
                self.fence(conn, FenceCause::EpochClosed, Some(report));
                Disposition::Stop
            }
            MoveAdmission::Refused { .. } => {
                report.lane1_refused += 1;
                Disposition::Continue
            }
        })
    }

    /// One traced `ingest` and its B§6 step-3 disposition.
    fn ingest_one(
        &mut self,
        conn: ConnectionId,
        frame: &[u8],
        now_ms: u64,
        report: &mut QuicPumpReport,
    ) -> Result<Disposition, HostError> {
        let Some((peer, ordinal)) = self.bound(conn).map(|bound| (bound.peer, bound.frames)) else {
            return Ok(Disposition::Stop);
        };
        let result = self.host.ingest(peer, frame, now_ms)?;
        report.calls.push(HostCall::Ingest {
            conn,
            peer,
            frame: ordinal,
            len: frame.len(),
            result: result.clone(),
        });
        Ok(match result {
            IngestDisposition::Staged => Disposition::Continue,
            IngestDisposition::Refused {
                status: ReceiptStatus::Rejected(RejectionCode::QueueFull),
            } => Disposition::Hold,
            IngestDisposition::Refused {
                status: ReceiptStatus::Rejected(RejectionCode::Unauthenticated),
            } => {
                report.frames_refused += 1;
                self.fence(conn, FenceCause::EpochClosed, Some(report));
                Disposition::Stop
            }
            IngestDisposition::Refused { .. } => {
                report.frames_refused += 1;
                Disposition::Continue
            }
        })
    }

    /// One traced `ingest_transfer` and its S§15 item 2 disposition. An
    /// `Err` poisons exactly as `ingest_one`'s; a transfer frame never holds.
    fn ingest_transfer_one(
        &mut self,
        conn: ConnectionId,
        frame: &[u8],
        now_ms: u64,
        report: &mut QuicPumpReport,
    ) -> Result<Disposition, HostError> {
        let Some((peer, ordinal)) = self.bound(conn).map(|bound| (bound.peer, bound.frames)) else {
            return Ok(Disposition::Stop);
        };
        let result = self.host.ingest_transfer(peer, frame, now_ms)?;
        report.calls.push(HostCall::IngestTransfer {
            conn,
            peer,
            frame: ordinal,
            len: frame.len(),
            result: result.clone(),
        });
        Ok(match result {
            TransferDisposition::Published
            | TransferDisposition::Resynced
            | TransferDisposition::Stale => Disposition::Continue,
            TransferDisposition::Refused(TransferRefusal::Unbound) => {
                report.frames_refused += 1;
                self.fence(conn, FenceCause::EpochClosed, Some(report));
                Disposition::Stop
            }
            TransferDisposition::Refused(_) => {
                report.frames_refused += 1;
                Disposition::Continue
            }
        })
    }

    /// B§5 phase 2 for a retiring connection: drain and discard every frame,
    /// whatever its channel and lane (C-6).
    fn drain_retiring(&mut self, conn: ConnectionId, report: &mut QuicPumpReport) {
        let mut lane0_gone = false;
        for _ in 0..MAX_DRAIN_PER_CONNECTION {
            match self.server.try_recv_channel(conn) {
                Ok(Some(_)) => report.frames_discarded += 1,
                Ok(None) => break,
                Err(_) => {
                    lane0_gone = true;
                    break;
                }
            }
        }
        let mut lane1_gone = matches!(
            self.conns.get(&conn),
            Some(Conn::Retiring(Retiring {
                lane1_gone: true,
                ..
            }))
        );
        if !lane1_gone {
            for _ in 0..MAX_DRAIN_PER_CONNECTION {
                match self.server.try_recv_lane1(conn) {
                    Ok(Some(_)) => report.lane1_discarded += 1,
                    Ok(None) => break,
                    Err(_) => {
                        lane1_gone = true;
                        break;
                    }
                }
            }
        }
        if lane1_gone {
            if let Some(Conn::Retiring(retiring)) = self.conns.get_mut(&conn) {
                retiring.lane1_gone = true;
            }
        }
        let seen = matches!(
            self.conns.get(&conn),
            Some(Conn::Retiring(Retiring {
                event_seen: true,
                ..
            }))
        );
        if lane0_gone && lane1_gone && seen {
            self.conns.remove(&conn);
        }
    }

    /// B§5 phase 5 for one bound connection.
    fn deliver(
        &mut self,
        conn: ConnectionId,
        now_ms: u64,
        report: &mut QuicPumpReport,
    ) -> Result<(), HostError> {
        let Some(peer) = self.bound(conn).map(|bound| bound.peer) else {
            return Ok(());
        };
        let taken = self.host.take_delivery_entries(peer);
        report.calls.push(HostCall::TakeDelivery {
            peer,
            result: taken.as_ref().map(Vec::len).map_err(Clone::clone),
        });
        let entries = match taken {
            Ok(entries) => entries,
            Err(error) if poisons(&error) => return Err(error),
            Err(_) => {
                self.fence(conn, FenceCause::EpochClosed, Some(report));
                return Ok(());
            }
        };
        let Some(bound) = self.bound(conn) else {
            return Ok(());
        };
        if bound.transport_closed.is_some() {
            // A departed peer's delivery is discarded and acknowledged so it
            // never blocks admission for the rest (B§8). The acknowledgement
            // is the last taken entry's ordinal: after a clear, a count-based
            // ack would fall short (T024h S§6.2).
            let n = entries.len();
            if n > 0 {
                let through = entries[n - 1].ordinal;
                self.acknowledge(conn, peer, through, report)?;
                report.frames_discarded += n;
            }
            if let Some(bound) = self.bound_mut(conn) {
                bound.stalled_since = None;
            }
            return Ok(());
        }
        let mut accepted = 0usize;
        let mut last_accepted = 0u64;
        let mut fence = false;
        for entry in &entries {
            let channel = match entry.kind {
                DeliveryKind::Delta => Channel::Lane0,
                DeliveryKind::Transfer => Channel::Transfer,
            };
            match self.server.try_send_channel(conn, channel, &entry.bytes) {
                Ok(()) => {
                    accepted += 1;
                    last_accepted = entry.ordinal;
                }
                Err(ChannelSendError::Send(SendError::QueueFull)) => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound.stalled_since.get_or_insert(now_ms);
                    }
                    break;
                }
                Err(ChannelSendError::Send(SendError::Closed(reason))) => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound.transport_closed.get_or_insert(reason);
                    }
                    break;
                }
                Err(ChannelSendError::Send(SendError::UnknownConnection)) => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound
                            .transport_closed
                            .get_or_insert(CloseReason::EndpointFailed);
                    }
                    break;
                }
                Err(ChannelSendError::Send(
                    SendError::EmptyFrame | SendError::FrameTooLarge | SendError::NotAccepted,
                )) => {
                    fence = true;
                    break;
                }
                Err(ChannelSendError::ChannelUnavailable) => {
                    // Cannot occur: only a `/2` session holds transfer
                    // entries, and lane 0 is available on every framing.
                    fence = true;
                    break;
                }
            }
        }
        report.frames_sent += accepted;
        if accepted > 0 {
            self.acknowledge(conn, peer, last_accepted, report)?;
        }
        if fence {
            self.fence(conn, FenceCause::EpochClosed, Some(report));
            return Ok(());
        }
        if accepted == entries.len() {
            if let Some(bound) = self.bound_mut(conn) {
                bound.stalled_since = None;
            }
        }
        // Slow consumer.
        let stalled = self
            .bound(conn)
            .and_then(|bound| bound.stalled_since)
            .is_some_and(|since| now_ms.saturating_sub(since) >= STALL_TIMEOUT_MS);
        if stalled {
            self.fence(conn, FenceCause::SlowConsumer, Some(report));
        }
        Ok(())
    }

    /// A traced `acknowledge_delivery` by delivery ordinal.
    fn acknowledge(
        &mut self,
        conn: ConnectionId,
        peer: PeerHandle,
        through: u64,
        report: &mut QuicPumpReport,
    ) -> Result<(), HostError> {
        let result = self.host.acknowledge_delivery(peer, through);
        report
            .calls
            .push(HostCall::AcknowledgeDelivery { peer, through });
        match result {
            Ok(()) => Ok(()),
            Err(error) if poisons(&error) => Err(error),
            Err(_) => {
                self.fence(conn, FenceCause::EpochClosed, Some(report));
                Ok(())
            }
        }
    }

    /// C-4/C-5: lane-1 receipts and samples for one still-bound movement
    /// connection, after its lane-0 delivery.
    fn deliver_lane1(
        &mut self,
        conn: ConnectionId,
        report: &mut QuicPumpReport,
    ) -> Result<(), HostError> {
        let Some(peer) = self.bound(conn).map(|bound| bound.peer) else {
            return Ok(());
        };
        let departed = self
            .bound(conn)
            .is_some_and(|bound| bound.transport_closed.is_some());
        // (1) Receipts, only when the transport's lane-1 queue is empty
        // (Reading 12): one full take then fits the outbound cap.
        let take_receipts = departed
            || self
                .server
                .lane1_stats(conn)
                .is_ok_and(|stats| stats.outbound_frames == 0);
        if take_receipts {
            let taken = self.host.take_move_receipts(peer);
            report.lane1.push(Lane1Trace {
                at: report.calls.len(),
                call: Lane1Call::TakeMoveReceipts {
                    peer,
                    result: taken.as_ref().map(Vec::len).map_err(Clone::clone),
                },
            });
            let receipts = match taken {
                Ok(receipts) => receipts,
                Err(error) if poisons(&error) => return Err(error),
                Err(_) => {
                    self.fence(conn, FenceCause::EpochClosed, Some(report));
                    return Ok(());
                }
            };
            if departed {
                report.lane1_discarded += receipts.len();
            } else {
                let mut refused = false;
                for receipt in &receipts {
                    if refused {
                        report.lane1_discarded += 1;
                        continue;
                    }
                    match self.server.try_send_lane1(conn, receipt) {
                        Ok(()) => report.lane1_sent += 1,
                        Err(ChannelSendError::Send(SendError::Closed(reason))) => {
                            if let Some(bound) = self.bound_mut(conn) {
                                bound.transport_closed.get_or_insert(reason);
                            }
                            refused = true;
                            report.lane1_discarded += 1;
                        }
                        Err(ChannelSendError::Send(SendError::UnknownConnection)) => {
                            if let Some(bound) = self.bound_mut(conn) {
                                bound
                                    .transport_closed
                                    .get_or_insert(CloseReason::EndpointFailed);
                            }
                            refused = true;
                            report.lane1_discarded += 1;
                        }
                        Err(_) => {
                            self.fence(conn, FenceCause::EpochClosed, Some(report));
                            return Ok(());
                        }
                    }
                }
            }
        }
        // (2) Samples, every pump (Reading 15): never held, never retried.
        let Some(peer) = self.bound(conn).map(|bound| bound.peer) else {
            return Ok(());
        };
        let taken = self.host.take_samples(peer);
        report.lane1.push(Lane1Trace {
            at: report.calls.len(),
            call: Lane1Call::TakeSamples {
                peer,
                result: taken.as_ref().map(Vec::len).map_err(Clone::clone),
            },
        });
        let samples = match taken {
            Ok(samples) => samples,
            Err(error) if poisons(&error) => return Err(error),
            Err(_) => {
                self.fence(conn, FenceCause::EpochClosed, Some(report));
                return Ok(());
            }
        };
        let discard = departed
            || self
                .bound(conn)
                .is_some_and(|bound| bound.transport_closed.is_some());
        if discard {
            report.lane1_discarded += samples.len();
            return Ok(());
        }
        for sample in &samples {
            match self.server.try_send_datagram(conn, sample) {
                Ok(()) => report.datagrams_sent += 1,
                Err(ChannelSendError::Send(SendError::Closed(reason))) => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound.transport_closed.get_or_insert(reason);
                    }
                    report.lane1_discarded += 1;
                }
                Err(ChannelSendError::Send(SendError::UnknownConnection)) => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound
                            .transport_closed
                            .get_or_insert(CloseReason::EndpointFailed);
                    }
                    report.lane1_discarded += 1;
                }
                Err(_) => {
                    self.fence(conn, FenceCause::EpochClosed, Some(report));
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// B§7.1: unbind (unless the host already did), close with
    /// `SessionFenced` in `Discard` mode, discard a held frame, clear the
    /// invitation's live binding, retire the entry and record the fence.
    /// `report` is `Some` only inside a pump, which traces the unbind.
    fn fence(
        &mut self,
        conn: ConnectionId,
        cause: FenceCause,
        report: Option<&mut QuicPumpReport>,
    ) -> Option<Fence> {
        let Some(Conn::Bound(bound)) = self.conns.remove(&conn) else {
            return None;
        };
        let mut report = report;
        if self.host.epoch(bound.peer) == Ok(bound.epoch) {
            self.host.unbind_peer(bound.peer);
            if let Some(report) = report.as_deref_mut() {
                report.calls.push(HostCall::UnbindPeer { peer: bound.peer });
            }
        }
        let _ = self
            .server
            .close(conn, CloseCode::SessionFenced, CloseMode::Discard);
        self.clear_live(bound.invitation, conn);
        self.conns.insert(
            conn,
            Conn::Retiring(Retiring {
                event_seen: bound.event_seen,
                lane1_gone: false,
            }),
        );
        let fence = Fence {
            conn,
            peer: bound.peer,
            cause,
        };
        if let Some(report) = report {
            if bound.held.is_some() {
                report.frames_discarded += 1;
            }
            // C-7: a held lane-1 frame is discarded too.
            if bound.held_lane1.is_some() {
                report.lane1_discarded += 1;
            }
            report.fences.push(fence);
        }
        Some(fence)
    }

    fn clear_live(&mut self, invitation: InvitationId, conn: ConnectionId) {
        if let Some(record) = self.invitations.get_mut(&invitation) {
            if record.live == Some(conn) {
                record.live = None;
            }
        }
    }

    /// As [`fence`](Self::fence) without the unbind (T025b-q N-5, H5): the
    /// session moved to another connection, so the host keeps the peer. Closes
    /// with `SessionFenced` in `Discard` mode, discards a held frame, clears
    /// the invitation's live binding, retires the entry and records the fence
    /// with cause `Resumed`.
    fn retire_resumed(
        &mut self,
        conn: ConnectionId,
        report: Option<&mut QuicPumpReport>,
    ) -> Option<Fence> {
        let Some(Conn::Bound(bound)) = self.conns.remove(&conn) else {
            return None;
        };
        let _ = self
            .server
            .close(conn, CloseCode::SessionFenced, CloseMode::Discard);
        self.clear_live(bound.invitation, conn);
        self.conns.insert(
            conn,
            Conn::Retiring(Retiring {
                event_seen: bound.event_seen,
                lane1_gone: false,
            }),
        );
        let fence = Fence {
            conn,
            peer: bound.peer,
            cause: FenceCause::Resumed,
        };
        if let Some(report) = report {
            if bound.held.is_some() {
                report.frames_discarded += 1;
            }
            report.fences.push(fence);
        }
        Some(fence)
    }

    /// N-8: an operator change (`set_invitation_grants`,
    /// `set_invitation_control`) ends the grace it touches, untraced.
    fn end_grace_for(&mut self, id: InvitationId) {
        let graced = self.invitations.get(&id).and_then(|record| record.graced);
        if let Some(graced) = graced {
            self.host.unbind_peer(graced);
            if let Some(record) = self.invitations.get_mut(&id) {
                record.graced = None;
            }
        }
    }

    fn bound_ids(&self) -> Vec<ConnectionId> {
        self.conns
            .iter()
            .filter(|(_, entry)| matches!(entry, Conn::Bound(_)))
            .map(|(conn, _)| *conn)
            .collect()
    }

    fn bound(&self, conn: ConnectionId) -> Option<&Bound> {
        match self.conns.get(&conn) {
            Some(Conn::Bound(bound)) => Some(bound),
            _ => None,
        }
    }

    fn bound_mut(&mut self, conn: ConnectionId) -> Option<&mut Bound> {
        match self.conns.get_mut(&conn) {
            Some(Conn::Bound(bound)) => Some(bound),
            _ => None,
        }
    }

    /// `QuicServer::wait`: blocks up to `timeout` until a transport event or
    /// inbound frame is queued. Optional; a pump on its own cadence never
    /// needs it.
    pub fn wait(&self, timeout: Duration) -> bool {
        self.server.wait(timeout)
    }

    /// Precedence: `Poisoned` → `InvitationLimit` → `InvalidCredential`
    /// (length ≠ 32) → `Host(InvalidControl)` → `Host(InvalidGrants)` →
    /// `DuplicateCredential`. Control and grants are stored normalized.
    pub fn register_invitation(
        &mut self,
        invitation: Invitation,
    ) -> Result<InvitationId, QuicHostError> {
        if self.poisoned {
            return Err(QuicHostError::Poisoned);
        }
        if self.invitations.len() >= MAX_INVITATIONS {
            return Err(QuicHostError::InvitationLimit);
        }
        if invitation.credential.as_bytes().len() != INVITATION_CREDENTIAL_BYTES {
            return Err(QuicHostError::InvalidCredential);
        }
        let actors = normalize_control(invitation.control).map_err(QuicHostError::Host)?;
        let grants = normalize_grants(invitation.grants).map_err(QuicHostError::Host)?;
        let duplicate = self
            .invitations
            .values()
            .map(|record| invitation.credential.ct_eq(record.credential.as_bytes()))
            .fold(false, |acc, flag| acc | flag);
        if duplicate {
            return Err(QuicHostError::DuplicateCredential);
        }
        let id = InvitationId(self.next_invitation);
        self.next_invitation += 1;
        self.invitations.insert(
            id,
            InvitationRecord {
                credential: invitation.credential,
                control: ControlGrant { actors },
                grants,
                live: None,
                graced: None,
                v4: None,
            },
        );
        Ok(id)
    }

    /// As [`register_invitation`](Self::register_invitation), storing the
    /// fact grants and pause rights a `crpg-lane0/3` binding needs
    /// (Reading 11). Precedence: `Poisoned` → `InvitationLimit` →
    /// `InvalidCredential` (length ≠ 32) → `Host(InvalidControl)` →
    /// `Host(InvalidGrants)` (the disclosure grants, then the fact grants) →
    /// `DuplicateCredential`.
    pub fn register_v4_invitation(
        &mut self,
        invitation: V4Invitation,
    ) -> Result<InvitationId, QuicHostError> {
        if self.poisoned {
            return Err(QuicHostError::Poisoned);
        }
        if self.invitations.len() >= MAX_INVITATIONS {
            return Err(QuicHostError::InvitationLimit);
        }
        if invitation.credential.as_bytes().len() != INVITATION_CREDENTIAL_BYTES {
            return Err(QuicHostError::InvalidCredential);
        }
        let actors = normalize_control(invitation.control).map_err(QuicHostError::Host)?;
        let grants = normalize_grants(invitation.grants).map_err(QuicHostError::Host)?;
        let facts = normalize_fact_grants(invitation.facts).map_err(QuicHostError::Host)?;
        let duplicate = self
            .invitations
            .values()
            .map(|record| invitation.credential.ct_eq(record.credential.as_bytes()))
            .fold(false, |acc, flag| acc | flag);
        if duplicate {
            return Err(QuicHostError::DuplicateCredential);
        }
        let id = InvitationId(self.next_invitation);
        self.next_invitation += 1;
        self.invitations.insert(
            id,
            InvitationRecord {
                credential: invitation.credential,
                control: ControlGrant { actors },
                grants,
                live: None,
                graced: None,
                v4: Some((facts, invitation.rights)),
            },
        );
        Ok(id)
    }

    /// Removes the invitation forever and fences its live binding
    /// (`Revoked`). Later hellos with it get `AuthRefused`. Errors:
    /// `Poisoned`, `UnknownInvitation`.
    pub fn revoke_invitation(&mut self, id: InvitationId) -> Result<Option<Fence>, QuicHostError> {
        if self.poisoned {
            return Err(QuicHostError::Poisoned);
        }
        let record = self
            .invitations
            .remove(&id)
            .ok_or(QuicHostError::UnknownInvitation)?;
        // N-8: revoking an invitation ends the grace it holds.
        if let Some(graced) = record.graced {
            self.host.unbind_peer(graced);
        }
        Ok(record
            .live
            .and_then(|conn| self.fence(conn, FenceCause::Revoked, None)))
    }

    /// Replaces the stored grants (B§7.2). Errors: `Poisoned`,
    /// `UnknownInvitation`, `Host(InvalidGrants)`, `Host(QueueFull)`
    /// (widening without room; nothing changes).
    pub fn set_invitation_grants(
        &mut self,
        id: InvitationId,
        grants: DisclosureGrants,
    ) -> Result<GrantChange, QuicHostError> {
        if self.poisoned {
            return Err(QuicHostError::Poisoned);
        }
        // N-8: changing the grants of a graced invitation ends the grace.
        self.end_grace_for(id);
        let record = self
            .invitations
            .get(&id)
            .ok_or(QuicHostError::UnknownInvitation)?;
        let grants = normalize_grants(grants).map_err(QuicHostError::Host)?;
        let live = record.live.and_then(|conn| {
            self.bound(conn)
                .map(|bound| (conn, bound.peer, bound.framing))
        });
        let Some((conn, peer, framing)) = live else {
            self.store_grants(id, grants);
            return Ok(GrantChange::Stored);
        };
        if is_narrowing(&record.grants, &grants) {
            if framing == Framing::V1 {
                // A `/1` binding cannot carry a transfer frame: fence as
                // today.
                self.store_grants(id, grants);
                return Ok(self.narrowed(conn));
            }
            return match self.host.set_grants(peer, grants.clone()) {
                Ok(GrantUpdate::SnapshotStaged) => {
                    self.store_grants(id, grants);
                    Ok(GrantChange::SnapshotStaged)
                }
                Ok(_) => Err(self.host_error(HostError::Invariant(
                    "snapshot session narrowed without a snapshot",
                ))),
                Err(error) => Err(self.host_error(error)),
            };
        }
        match self.host.set_grants(peer, grants.clone()) {
            Ok(GrantUpdate::Unchanged) => {
                self.store_grants(id, grants);
                Ok(GrantChange::Widened)
            }
            Ok(GrantUpdate::RebindRequired) => {
                self.store_grants(id, grants);
                Ok(self.narrowed(conn))
            }
            Ok(GrantUpdate::SnapshotStaged) => {
                self.store_grants(id, grants);
                Ok(GrantChange::SnapshotStaged)
            }
            Err(error) => Err(self.host_error(error)),
        }
    }

    fn narrowed(&mut self, conn: ConnectionId) -> GrantChange {
        match self.fence(conn, FenceCause::Narrowed, None) {
            Some(fence) => GrantChange::Fenced(fence),
            None => GrantChange::Stored,
        }
    }

    fn store_grants(&mut self, id: InvitationId, grants: DisclosureGrants) {
        if let Some(record) = self.invitations.get_mut(&id) {
            record.grants = grants;
        }
    }

    /// Replaces the stored control grant and, when bound, calls
    /// `Host::set_control` (future admission only; staged commands are
    /// judged at pump time). Errors: `Poisoned`, `UnknownInvitation`,
    /// `Host(InvalidControl)`.
    pub fn set_invitation_control(
        &mut self,
        id: InvitationId,
        control: ControlGrant,
    ) -> Result<(), QuicHostError> {
        if self.poisoned {
            return Err(QuicHostError::Poisoned);
        }
        // N-8: changing the control of a graced invitation ends the grace.
        self.end_grace_for(id);
        let record = self
            .invitations
            .get(&id)
            .ok_or(QuicHostError::UnknownInvitation)?;
        let actors = normalize_control(control).map_err(QuicHostError::Host)?;
        let control = ControlGrant { actors };
        let peer = record
            .live
            .and_then(|conn| self.bound(conn).map(|bound| bound.peer));
        if let Some(peer) = peer {
            if let Err(error) = self.host.set_control(peer, control.clone()) {
                return Err(self.host_error(error));
            }
        }
        if let Some(record) = self.invitations.get_mut(&id) {
            record.control = control;
        }
        Ok(())
    }

    /// Operator kick: fences a bound connection (`Disconnected`). Errors:
    /// `Poisoned`, `UnknownConnection` (not bound).
    pub fn disconnect(&mut self, conn: ConnectionId) -> Result<Fence, QuicHostError> {
        if self.poisoned {
            return Err(QuicHostError::Poisoned);
        }
        self.fence(conn, FenceCause::Disconnected, None)
            .ok_or(QuicHostError::UnknownConnection)
    }

    /// Trusted pass-through: one `Host::tick`.
    pub fn tick(&mut self) -> Result<(), QuicHostError> {
        if self.poisoned {
            return Err(QuicHostError::Poisoned);
        }
        self.host.tick().map_err(|error| self.host_error(error))
    }

    /// Trusted pass-through: one `Host::advance` (Reading 16). Precedence:
    /// `Poisoned` → `TimeRegression` (below the adapter's last `now_ms`;
    /// nothing changes) → the adapter records `now_ms` → the host call, an
    /// error through [`host_error`](Self::host_error) (an `Invariant`
    /// poisons). Never traced.
    pub fn advance(&mut self, now_ms: u64) -> Result<AdvanceReport, QuicHostError> {
        if self.poisoned {
            return Err(QuicHostError::Poisoned);
        }
        if now_ms < self.last_now_ms {
            return Err(QuicHostError::TimeRegression);
        }
        self.last_now_ms = now_ms;
        self.host
            .advance(now_ms)
            .map_err(|error| self.host_error(error))
    }

    /// Trusted pass-through: `Host::drive_ai` (T153). `Poisoned` first, then
    /// the host call, an error through [`host_error`](Self::host_error) (an
    /// `Invariant` poisons). Never traced.
    pub fn drive_ai(
        &mut self,
        sides: &crpg_ai::Sides,
        actors: &[EntityId],
        budget: AiBudget,
    ) -> Result<AiReport, QuicHostError> {
        if self.poisoned {
            return Err(QuicHostError::Poisoned);
        }
        self.host
            .drive_ai(sides, actors, budget)
            .map_err(|error| self.host_error(error))
    }

    /// Trusted pass-through: `Host::pause`. `Poisoned` first, then the host
    /// call (which records `now_ms` and refuses a regression), an error
    /// through [`host_error`](Self::host_error).
    pub fn pause(&mut self, now_ms: u64) -> Result<(), QuicHostError> {
        if self.poisoned {
            return Err(QuicHostError::Poisoned);
        }
        self.host
            .pause(now_ms)
            .map_err(|error| self.host_error(error))
    }

    /// Trusted pass-through: `Host::unpause`. `Poisoned` first, then the host
    /// call (which records `now_ms` and refuses a regression), an error
    /// through [`host_error`](Self::host_error).
    pub fn unpause(&mut self, now_ms: u64) -> Result<(), QuicHostError> {
        if self.poisoned {
            return Err(QuicHostError::Poisoned);
        }
        self.host
            .unpause(now_ms)
            .map_err(|error| self.host_error(error))
    }

    /// Trusted pass-through: `Host::acknowledge_captures`.
    pub fn acknowledge_captures(&mut self, through: u64) -> Result<(), QuicHostError> {
        if self.poisoned {
            return Err(QuicHostError::Poisoned);
        }
        self.host
            .acknowledge_captures(through)
            .map_err(|error| self.host_error(error))
    }

    /// A host error outside a pump: `Invariant` poisons (B§12); the rest
    /// (`QueueFull`, `FutureCaptureCursor`, `Closed` from `tick`, …) pass
    /// through as normal errors.
    fn host_error(&mut self, error: HostError) -> QuicHostError {
        if matches!(error, HostError::Invariant(_)) {
            self.poisoned = true;
        }
        QuicHostError::Host(error)
    }

    /// Read-only access for trusted oracles, captures and checkpoints.
    pub fn host(&self) -> &Host {
        &self.host
    }

    /// The binding of a bound connection.
    pub fn binding(&self, conn: ConnectionId) -> Option<PeerHandle> {
        self.bound(conn).map(|bound| bound.peer)
    }

    /// The live binding of a registered invitation. Errors:
    /// `UnknownInvitation`.
    pub fn invitation_binding(
        &self,
        id: InvitationId,
    ) -> Result<Option<(ConnectionId, PeerHandle)>, QuicHostError> {
        let record = self
            .invitations
            .get(&id)
            .ok_or(QuicHostError::UnknownInvitation)?;
        Ok(record
            .live
            .and_then(|conn| self.bound(conn).map(|bound| (conn, bound.peer))))
    }

    /// The bound UDP address.
    pub fn local_addr(&self) -> SocketAddr {
        self.server.local_addr()
    }

    /// The transport's host-wide totals.
    pub fn transport_totals(&self) -> QueueTotals {
        self.server.totals()
    }

    /// The transport's snapshot of one connection's queues.
    pub fn connection_stats(&self, conn: ConnectionId) -> Option<ConnectionStats> {
        self.server.stats(conn).ok()
    }

    /// Graceful D02 shutdown (B§9). Infallible; consumes the adapter.
    pub fn shutdown(self, checkpoint: bool) -> QuicHostShutdown {
        let QuicHost {
            mut host,
            mut server,
            conns,
            ..
        } = self;
        // 1. Stop admissions: no event is polled, no ingest or pump runs.
        // 2. Flush committed delivery to every open transport, on each
        // entry's channel and acknowledged by ordinal.
        for (conn, entry) in &conns {
            let Conn::Bound(bound) = entry else {
                continue;
            };
            if bound.transport_closed.is_some() {
                continue;
            }
            let Ok(entries) = host.take_delivery_entries(bound.peer) else {
                continue;
            };
            let mut accepted = 0usize;
            let mut last_accepted = 0u64;
            for entry in &entries {
                let channel = match entry.kind {
                    DeliveryKind::Delta => Channel::Lane0,
                    DeliveryKind::Transfer => Channel::Transfer,
                };
                if server
                    .try_send_channel(*conn, channel, &entry.bytes)
                    .is_err()
                {
                    break;
                }
                accepted += 1;
                last_accepted = entry.ordinal;
            }
            if accepted > 0 {
                let _ = host.acknowledge_delivery(bound.peer, last_accepted);
            }
        }
        // 2b. Flush released lane-1 receipts to every open movement
        // transport (C-8); no sample is taken.
        let mut undelivered_receipts = 0usize;
        for (conn, entry) in &conns {
            let Conn::Bound(bound) = entry else {
                continue;
            };
            if !bound.movement || bound.transport_closed.is_some() {
                continue;
            }
            let Ok(receipts) = host.take_move_receipts(bound.peer) else {
                continue;
            };
            let mut refused = false;
            for receipt in &receipts {
                if refused {
                    undelivered_receipts += 1;
                    continue;
                }
                if server.try_send_lane1(*conn, receipt).is_err() {
                    refused = true;
                    undelivered_receipts += 1;
                }
            }
        }
        // 3. Count what could not be sent (both kinds).
        let undelivered_frames = conns
            .values()
            .filter_map(|entry| match entry {
                Conn::Bound(bound) => host
                    .take_delivery_entries(bound.peer)
                    .ok()
                    .map(|entries| entries.len()),
                Conn::Retiring(_) => None,
            })
            .sum();
        // 4. Refuse staged work and close every binding.
        let refused = host.shutdown();
        // 5. Discard held frames.
        let held_discarded = conns
            .values()
            .filter(|entry| matches!(entry, Conn::Bound(bound) if bound.held.is_some()))
            .count();
        // 6. Checkpoint at the fenced, immutable boundary.
        let checkpoint = checkpoint.then(|| host.save_checkpoint());
        // 7. Close transports.
        let transport = server.shutdown();
        QuicHostShutdown {
            host,
            refused,
            held_discarded,
            undelivered_frames,
            undelivered_receipts,
            checkpoint,
            transport,
        }
    }
}

/// What the drain does after one ingest.
enum Disposition {
    /// Staged or terminally dropped: keep draining.
    Continue,
    /// Retryable `QueueFull`: hold the frame and stop this connection.
    Hold,
    /// The binding was fenced: stop this connection.
    Stop,
}

// ---------------------------------------------------------------------------
// Fix-round unit tests (F3, F5, F6): arms the public rig cannot race into.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::net::SocketAddr;
    use std::time::Instant;

    use crpg_net::protocol_resume::ResumeRequest;
    use crpg_net_quic::{
        ClientConfig, ClientLimits, QuicClient, ServerConfig, ServerIdentity, ServerLimits,
    };
    use crpg_sim::HistoryWorld;

    use crate::host::{HostConfig, ProtocolSelection};

    const NOW: u64 = 1_000;
    const WAIT: Duration = Duration::from_secs(30);
    const FIXTURE: &str = include_str!("../tests/fixtures/three_combatants.json");
    const CERT: &[u8] = include_bytes!("../../crpg-net-quic/tests/fixtures/server_a.cert.der");
    const KEY: &[u8] = include_bytes!("../../crpg-net-quic/tests/fixtures/server_a.key.pk8.der");
    const CREDENTIAL: [u8; 32] = [0xA1; 32];

    fn fixture() -> HistoryWorld {
        let mut history: HistoryWorld =
            serde_json::from_str(FIXTURE).expect("fixture loads through validated serde");
        history
            .acknowledge(history.last_sequence())
            .expect("acknowledges its own range");
        history
    }

    fn server_identity() -> ServerIdentity {
        ServerIdentity::new(CERT.to_vec(), KEY.to_vec())
    }

    fn client_config() -> ClientConfig {
        ClientConfig {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            pin: server_identity().pin(),
            limits: ClientLimits::v1(),
        }
    }

    fn credential() -> Credential {
        Credential::new(CREDENTIAL.to_vec()).expect("credential length in transport bounds")
    }

    /// A resumable adapter over the fixture host, with A registered.
    fn adapter() -> (QuicHost, u64) {
        let host = Host::from_history(
            fixture(),
            HostConfig {
                protocol: ProtocolSelection::V2,
                incarnation: 7,
            },
            NOW,
        )
        .expect("wraps an acknowledged authority");
        let server = QuicServer::bind_resumable(ServerConfig {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            identity: server_identity(),
            limits: ServerLimits::v1(),
        })
        .expect("bind resumable server");
        let qh = QuicHost::start_resumable(host, server, NOW).expect("starts over a fresh host");
        let mut qh = qh;
        qh.register_invitation(Invitation {
            credential: credential(),
            control: ControlGrant { actors: vec![] },
            grants: DisclosureGrants::default(),
        })
        .expect("registers A");
        (qh, NOW)
    }

    /// Pumps at `+100 ms` until `cond` holds; guards only.
    fn pump_until(
        qh: &mut QuicHost,
        now: &mut u64,
        what: &str,
        mut cond: impl FnMut(&QuicPumpReport) -> bool,
    ) -> QuicPumpReport {
        let start = Instant::now();
        loop {
            *now += 100;
            let report = qh.pump(*now).expect("pump");
            if cond(&report) {
                return report;
            }
            assert!(start.elapsed() < WAIT, "deadline: {what}");
            qh.wait(Duration::from_millis(5));
        }
    }

    /// Connects and accepts a `/2` hello of A through the adapter.
    fn connect_v2(qh: &mut QuicHost, now: &mut u64) -> (QuicClient, ConnectionId, PeerHandle) {
        let addr = qh.local_addr();
        let hello = crpg_net_quic::Hello {
            wire_version: 2,
            credential: credential(),
        };
        let pending = std::thread::spawn(move || {
            QuicClient::connect_framed(client_config(), addr, &hello, Framing::V2)
        });
        let report = pump_until(qh, now, "hello decided", |report| !report.hellos.is_empty());
        assert_eq!(report.hellos.len(), 1, "{:?}", report.hellos);
        let decision = report.hellos[0].clone();
        let HelloOutcome::Accepted { peer, .. } = decision.outcome else {
            panic!("expected Accepted, got {decision:?}");
        };
        let client = pending.join().expect("client thread").expect("connect");
        (client, decision.conn, peer)
    }

    fn request(epoch: [u8; 16], proof_bytes: Vec<u8>) -> ResumeRequest {
        ResumeRequest {
            epoch,
            fence: ConnectionFence::new(1).expect("first fence"),
            proof: ResumeProof::new(proof_bytes).expect("32-byte proof"),
            claims: Vec::new(),
        }
    }

    /// F5a (C4): a frame on a connection whose session fence moved on is
    /// refused and retired as `Resumed`; the host session stays bound.
    #[test]
    fn a_fenced_frame_retires_without_unbinding() {
        let (mut qh, mut now) = adapter();
        let (mut client, conn, peer) = connect_v2(&mut qh, &mut now);
        let epoch = qh.host.epoch(peer).expect("bound");
        // Move the session fence behind the adapter, as a live resume would.
        let proof = resume_proof(&credential(), epoch).expect("proof");
        let answer = qh
            .host
            .resume_peer(&request(epoch, proof.as_bytes().to_vec()), now)
            .expect("resume decides");
        assert!(matches!(answer, ResumeAnswer::Accepted { .. }));

        client.try_send(&[2, 1, 0, 0]).expect("client lane-0 send");
        let report = pump_until(&mut qh, &mut now, "frame refused", |report| {
            report.frames_refused > 0
        });
        assert_eq!(report.frames_refused, 1);
        assert_eq!(
            report.fences,
            vec![Fence {
                conn,
                peer,
                cause: FenceCause::Resumed,
            }]
        );
        assert!(
            report
                .calls
                .iter()
                .all(|call| !matches!(call, HostCall::UnbindPeer { .. })),
            "the resumed binding is not unbound: {:?}",
            report.calls
        );
        assert_eq!(qh.host().epoch(peer), Ok(epoch), "the peer stays bound");
    }

    /// F5b (C6): a frame for a session the host no longer has is refused and
    /// fenced as `EpochClosed`, not `Resumed`.
    #[test]
    fn a_frame_for_an_unbound_peer_fences_as_epoch_closed() {
        let (mut qh, mut now) = adapter();
        let (mut client, conn, peer) = connect_v2(&mut qh, &mut now);
        qh.host.unbind_peer(peer);
        client.try_send(&[2, 1, 0, 0]).expect("client lane-0 send");
        // Queue the frame before the pump: phase 2's fence check must see it
        // before phase 4's epoch reconcile fences the connection.
        let start = Instant::now();
        while qh
            .connection_stats(conn)
            .map_or(0, |stats| stats.inbound_frames)
            == 0
        {
            assert!(start.elapsed() < WAIT, "deadline: frame queued");
            qh.wait(Duration::from_millis(5));
        }
        let report = pump_until(&mut qh, &mut now, "frame refused", |report| {
            report.frames_refused > 0
        });
        assert_eq!(report.frames_refused, 1);
        assert_eq!(
            report.fences,
            vec![Fence {
                conn,
                peer,
                cause: FenceCause::EpochClosed,
            }]
        );
        assert!(
            report
                .calls
                .iter()
                .all(|call| !matches!(call, HostCall::UnbindPeer { .. })),
            "an unbound peer is not unbound again: {:?}",
            report.calls
        );
        assert_eq!(qh.host().epoch(peer), Err(HostError::UnknownPeer));
    }

    /// X5: a held frame is discarded when a resumed binding retires. No
    /// check-frame refusal can coincide with a held frame (the held retry
    /// always runs first and stops the drain), so the private discard arm is
    /// reached directly.
    #[test]
    fn a_held_frame_is_discarded_when_a_resumed_binding_retires() {
        let (mut qh, mut now) = adapter();
        let (client, conn, peer) = connect_v2(&mut qh, &mut now);
        let _client = client;
        match qh.conns.get_mut(&conn) {
            Some(Conn::Bound(bound)) => bound.held = Some(vec![1, 2, 3]),
            _ => panic!("expected a bound connection"),
        }
        let mut report = QuicPumpReport::new();
        let fence = qh
            .retire_resumed(conn, Some(&mut report))
            .expect("a bound entry retires");
        assert_eq!(
            fence,
            Fence {
                conn,
                peer,
                cause: FenceCause::Resumed,
            }
        );
        assert_eq!(report.frames_discarded, 1);
        assert_eq!(report.fences, vec![fence]);
    }

    /// F6 (G2, G3): `Transport` and `Reset` losses start a grace too.
    #[test]
    fn transport_and_reset_losses_start_a_grace() {
        for reason in [CloseReason::Transport, CloseReason::Reset] {
            let (mut qh, mut now) = adapter();
            let (_client, conn, peer) = connect_v2(&mut qh, &mut now);
            match qh.conns.get_mut(&conn) {
                Some(Conn::Bound(bound)) => {
                    bound.transport_closed = Some(reason);
                    bound.drained = true;
                }
                _ => panic!("expected a bound connection"),
            }
            let report = pump_until(&mut qh, &mut now, "grace from a silent loss", |report| {
                report
                    .calls
                    .iter()
                    .any(|call| matches!(call, HostCall::BeginGrace { result: Ok(_), .. }))
            });
            assert!(
                report.calls.iter().any(|call| matches!(
                    call,
                    HostCall::BeginGrace {
                        peer: p,
                        result: Ok(_)
                    } if *p == peer
                )),
                "a {reason:?} close is a silent loss: {:?}",
                report.calls
            );
            assert!(qh.host().grace_deadline(peer).is_some());
        }
    }

    /// F4b (O2): `end_grace_for` clears the grace record, not just the host
    /// deadline. Phase 4's N-6 sweep clears a stale record on the next pump,
    /// so the public hello path cannot distinguish the mutant; assert the
    /// state directly after the public change, with no pump in between.
    #[test]
    fn a_grant_change_clears_the_grace_record() {
        let (mut qh, mut now) = adapter();
        let (client, conn, peer) = connect_v2(&mut qh, &mut now);
        let _client = client;
        match qh.conns.get_mut(&conn) {
            Some(Conn::Bound(bound)) => {
                bound.transport_closed = Some(CloseReason::IdleTimeout);
                bound.drained = true;
            }
            _ => panic!("expected a bound connection"),
        }
        pump_until(&mut qh, &mut now, "begin grace", |report| {
            report
                .calls
                .iter()
                .any(|call| matches!(call, HostCall::BeginGrace { result: Ok(_), .. }))
        });
        let invitation = qh
            .invitations
            .iter()
            .find_map(|(id, record)| (record.graced == Some(peer)).then_some(*id))
            .expect("the grace is recorded");
        qh.set_invitation_grants(invitation, DisclosureGrants::default())
            .expect("a valid grant change");
        assert_eq!(qh.host().grace_deadline(peer), None);
        assert!(
            qh.invitations
                .get(&invitation)
                .expect("known invitation")
                .graced
                .is_none(),
            "the ended grace's record is cleared"
        );
    }

    /// T040a T2: `rejections()` lists only a rejected admission that is not
    /// the retained head, in order, and each `Display` is B-2's text.
    #[test]
    fn admissions_filter_and_display() {
        let mut host = Host::from_history(
            fixture(),
            HostConfig {
                protocol: ProtocolSelection::V1,
                incarnation: 7,
            },
            NOW,
        )
        .expect("wraps an acknowledged authority");
        let mut peer = None;
        for _ in 0..3 {
            peer = Some(
                host.bind_peer(
                    ControlGrant { actors: vec![] },
                    DisclosureGrants::default(),
                    NOW,
                )
                .expect("binds"),
            );
        }
        let peer = peer.expect("three binds");
        assert_eq!(peer.get(), 3);

        let (c1, c2) = (RejectionCode::IllegalAction, RejectionCode::NotAuthorized);
        let result = |seq, status, cached, retained| AdmissionResult {
            peer,
            seq,
            status,
            cached,
            retained_in_ingress: retained,
        };
        let rejected = ReceiptStatus::Rejected;
        let report_of = |results: Vec<AdmissionResult>| {
            let mut report = QuicPumpReport::new();
            report.calls.push(HostCall::Pump {
                summary: PumpSummary {
                    admitted: 0,
                    applied: 0,
                    rejected: 0,
                    backpressured: 0,
                    dropped_replies: 0,
                    results,
                },
            });
            report
        };
        let applied = result(Some(6), ReceiptStatus::Applied, false, false);
        let first = result(Some(7), rejected(c1), false, false);
        let head = result(Some(8), rejected(RejectionCode::QueueFull), false, true);
        let second = result(None, rejected(c2), true, false);

        let report = report_of(vec![
            applied.clone(),
            first.clone(),
            head.clone(),
            second.clone(),
        ]);
        let found = report.rejections();
        assert_eq!(
            found,
            vec![
                Rejection::Admission {
                    peer,
                    seq: Some(7),
                    code: c1,
                    cached: false
                },
                Rejection::Admission {
                    peer,
                    seq: None,
                    code: c2,
                    cached: true
                },
            ]
        );
        assert_eq!(
            found[0].to_string(),
            format!("admission peer=3 seq=7 code={} cached=false", c1 as u8)
        );
        assert_eq!(
            found[1].to_string(),
            format!("admission peer=3 seq=- code={} cached=true", c2 as u8)
        );

        // Control: the same head, not retained, is a rejection like any other.
        let mut loose = head;
        loose.retained_in_ingress = false;
        let control = report_of(vec![applied, first, loose, second]).rejections();
        assert_eq!(
            control.len(),
            3,
            "a non-retained QueueFull result is listed"
        );
        assert!(matches!(
            control[1],
            Rejection::Admission {
                seq: Some(8),
                code: RejectionCode::QueueFull,
                ..
            }
        ));
    }

    /// Fix round A (T040a B-1, B-2): one hand-built report holding every
    /// rejection source; `rejections()` lists them in B-1's order (hellos,
    /// resumes, then calls) with the right connection, peer and code in each,
    /// and each `Display` is B-2's text. Connection ids come from real refused
    /// hellos (`ConnectionId` has no public constructor); peers from a host
    /// whose first six handles were unbound, so no peer number equals a
    /// connection number and a swap of the two shows.
    #[test]
    fn fx_a1_rejections_list_every_source_in_order() {
        let (mut qh, mut now) = adapter();
        let addr = qh.local_addr();
        let mut conns = Vec::new();
        for _ in 0..5 {
            let hello = crpg_net_quic::Hello {
                wire_version: 2,
                credential: Credential::new(vec![0xB2; 32]).expect("32-byte credential"),
            };
            let pending = std::thread::spawn(move || {
                QuicClient::connect_framed(client_config(), addr, &hello, Framing::V2)
            });
            let report = pump_until(&mut qh, &mut now, "hello decided", |report| {
                !report.hellos.is_empty()
            });
            assert_eq!(report.hellos.len(), 1, "{:?}", report.hellos);
            assert_eq!(
                report.hellos[0].outcome,
                HelloOutcome::Refused {
                    code: CloseCode::AuthRefused
                }
            );
            conns.push(report.hellos[0].conn);
            assert!(
                pending.join().expect("client thread").is_err(),
                "the unknown credential is refused"
            );
        }
        for (i, a) in conns.iter().enumerate() {
            assert!(a.get() < 7, "connection ids stay below the peer numbers");
            assert!(
                conns[i + 1..].iter().all(|b| a != b),
                "distinct connections"
            );
        }

        let mut host = Host::from_history(
            fixture(),
            HostConfig {
                protocol: ProtocolSelection::V1,
                incarnation: 7,
            },
            NOW,
        )
        .expect("wraps an acknowledged authority");
        let bind = |host: &mut Host| {
            host.bind_peer(
                ControlGrant { actors: vec![] },
                DisclosureGrants::default(),
                NOW,
            )
            .expect("binds")
        };
        let early: Vec<PeerHandle> = (0..6).map(|_| bind(&mut host)).collect();
        for peer in early {
            host.unbind_peer(peer);
        }
        let (p7, p8, p9) = (bind(&mut host), bind(&mut host), bind(&mut host));
        assert_eq!((p7.get(), p8.get(), p9.get()), (7, 8, 9));

        let mut report = QuicPumpReport::new();
        report.hellos.push(HelloDecision {
            conn: conns[0],
            outcome: HelloOutcome::Refused {
                code: CloseCode::AuthRefused,
            },
        });
        report.resumes.push(ResumeReport {
            conn: conns[1],
            outcome: ResumeOutcome::Lost {
                reason: CloseReason::Transport,
            },
        });
        report.resumes.push(ResumeReport {
            conn: conns[2],
            outcome: ResumeOutcome::Refused {
                code: CloseCode::ServerBusy,
            },
        });
        report.calls.push(HostCall::Ingest {
            conn: conns[4],
            peer: p7,
            frame: 11,
            len: 13,
            result: IngestDisposition::Refused {
                status: ReceiptStatus::Rejected(RejectionCode::Malformed),
            },
        });
        report.calls.push(HostCall::IngestTransfer {
            conn: conns[3],
            peer: p8,
            frame: 5,
            len: 9,
            result: TransferDisposition::Published,
        });
        report.calls.push(HostCall::IngestTransfer {
            conn: conns[3],
            peer: p8,
            frame: 6,
            len: 21,
            result: TransferDisposition::Refused(TransferRefusal::RateLimited),
        });
        report.calls.push(HostCall::CheckFrame {
            conn: conns[4],
            peer: p9,
            fence: ConnectionFence::FIRST,
            result: Ok(()),
        });
        report.calls.push(HostCall::CheckFrame {
            conn: conns[1],
            peer: p8,
            fence: ConnectionFence::FIRST,
            result: Err(HostError::UnknownPeer),
        });
        report.calls.push(HostCall::Pump {
            summary: PumpSummary {
                admitted: 0,
                applied: 0,
                rejected: 1,
                backpressured: 0,
                dropped_replies: 0,
                results: vec![AdmissionResult {
                    peer: p9,
                    seq: Some(3),
                    status: ReceiptStatus::Rejected(RejectionCode::IllegalAction),
                    cached: false,
                    retained_in_ingress: false,
                }],
            },
        });

        let found = report.rejections();
        assert_eq!(
            found,
            vec![
                Rejection::Hello {
                    conn: conns[0],
                    code: CloseCode::AuthRefused
                },
                Rejection::Resume {
                    conn: conns[2],
                    code: CloseCode::ServerBusy
                },
                Rejection::Ingest {
                    conn: conns[4],
                    peer: p7,
                    frame: 11,
                    len: 13,
                    code: RejectionCode::Malformed
                },
                Rejection::Transfer {
                    conn: conns[3],
                    peer: p8,
                    frame: 6,
                    len: 21,
                    refusal: TransferRefusal::RateLimited
                },
                Rejection::Frame {
                    conn: conns[1],
                    peer: p8,
                    error: HostError::UnknownPeer
                },
                Rejection::Admission {
                    peer: p9,
                    seq: Some(3),
                    code: RejectionCode::IllegalAction,
                    cached: false
                },
            ]
        );
        assert_eq!(
            found[0].to_string(),
            format!("hello conn={} code=5", conns[0].get())
        );
        assert_eq!(
            found[1].to_string(),
            format!("resume conn={} code=7", conns[2].get())
        );
        assert_eq!(
            found[3].to_string(),
            format!(
                "transfer conn={} peer=8 frame=6 len=21 refusal=RateLimited",
                conns[3].get()
            )
        );
        assert_eq!(
            found[4].to_string(),
            format!(
                "frame conn={} peer=8 error={}",
                conns[1].get(),
                HostError::UnknownPeer
            )
        );
    }
}
