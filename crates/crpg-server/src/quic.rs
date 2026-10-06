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
//! peers, and the binding's delivery log is handed to the transport and
//! acknowledged when `try_send` accepts each frame.
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

use crpg_net::protocol::{ReceiptStatus, RejectionCode, POLICY_INGRESS_FRAMES_PER_PEER};
use crpg_net_quic::{
    CloseCode, CloseMode, CloseReason, ConnectionId, ConnectionStats, Credential, QueueTotals,
    QuicServer, SendError, ServerError, ServerEvent, ShutdownReport, Welcome,
};

use crate::checkpoint::CheckpointError;
use crate::host::{
    is_narrowing, normalize_control, normalize_grants, AdmissionResult, ControlGrant,
    DisclosureGrants, GrantUpdate, Host, HostError, IngestDisposition, PeerHandle, PumpSummary,
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

// ---------------------------------------------------------------------------
// Reports (B§3.3).
// ---------------------------------------------------------------------------

/// What one `pump` did, in call order. Bounded per pump: at most
/// `max_connections` hello decisions and closes, and at most
/// `MAX_PEERS × MAX_DRAIN_PER_CONNECTION` ingest calls plus one held retry
/// per peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuicPumpReport {
    /// Every host call this pump made, in order: the equivalence trace.
    pub calls: Vec<HostCall>,
    /// One per `HelloReceived` polled.
    pub hellos: Vec<HelloDecision>,
    /// Bindings fenced by this pump.
    pub fences: Vec<Fence>,
    /// `ServerEvent::Closed` events polled (bound, retiring or unmapped).
    pub transport_closes: Vec<(ConnectionId, CloseReason)>,
    /// Delivery frames accepted by `try_send`.
    pub frames_sent: usize,
    /// Inbound frames dropped after a terminal ingest refusal.
    pub frames_refused: usize,
    /// Frames discarded: drained from retiring connections, held frames of
    /// fenced or closed bindings, and delivery frames for closed transports.
    pub frames_discarded: usize,
    /// Bound connections holding a refused frame when the pump ended.
    pub held: usize,
}

impl QuicPumpReport {
    fn new() -> Self {
        Self {
            calls: Vec::new(),
            hellos: Vec::new(),
            fences: Vec::new(),
            transport_closes: Vec::new(),
            frames_sent: 0,
            frames_refused: 0,
            frames_discarded: 0,
            held: 0,
        }
    }
}

/// One host call, with what it returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCall {
    /// `Host::bind_peer` for a hello presenting `invitation` on `conn`.
    BindPeer {
        /// The connection the hello arrived on.
        conn: ConnectionId,
        /// The matched invitation.
        invitation: InvitationId,
        /// What `bind_peer` returned.
        result: Result<PeerHandle, HostError>,
    },
    /// `Host::unbind_peer`.
    UnbindPeer {
        /// The unbound handle.
        peer: PeerHandle,
    },
    /// `Host::ingest`. `frame` is the 1-based ordinal of the frame on its
    /// connection (`try_recv` order); a held frame's retries repeat its
    /// ordinal.
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
    /// `Host::pump`.
    Pump {
        /// Its summary.
        summary: PumpSummary,
    },
    /// `Host::take_delivery`. `Ok(n)`: it returned `n` frames.
    TakeDelivery {
        /// The binding.
        peer: PeerHandle,
        /// The frame count, or the error.
        result: Result<usize, HostError>,
    },
    /// `Host::acknowledge_delivery`.
    AcknowledgeDelivery {
        /// The binding.
        peer: PeerHandle,
        /// The acknowledged `event_seq`.
        through: u64,
    },
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
    /// Host delivery frames the transport could not take before shutdown.
    pub undelivered_frames: usize,
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
}

/// A bound connection.
struct Bound {
    peer: PeerHandle,
    epoch: [u8; 16],
    invitation: InvitationId,
    /// The last delivery `event_seq` acknowledged (0 at bind).
    acked: u64,
    /// The ordinal of the last frame taken with `try_recv`.
    frames: u64,
    /// One frame refused with `QueueFull`, retried before any later frame.
    held: Option<Vec<u8>>,
    stalled_since: Option<u64>,
    transport_closed: Option<CloseReason>,
    /// `try_recv` reported the end of the inbound stream.
    drained: bool,
    /// The connection's `Closed` event has been polled.
    event_seen: bool,
}

/// A connection this adapter closed, or whose binding ended, still drained
/// and discarded so the transport can forget its record.
struct Retiring {
    event_seen: bool,
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
    pub fn start(
        host: Host,
        server: QuicServer,
        now_ms: u64,
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
        let result = self.run_phases(now_ms, &mut report);
        match result {
            Ok(()) => Ok(report),
            Err(error) => {
                self.poisoned = true;
                Err(QuicHostError::Host(error))
            }
        }
    }

    fn run_phases(&mut self, now_ms: u64, report: &mut QuicPumpReport) -> Result<(), HostError> {
        // 1. Events.
        while let Some(event) = self.server.poll_event() {
            match event {
                ServerEvent::HelloReceived { conn, hello, .. } => {
                    self.on_hello(conn, hello.wire_version, &hello.credential, now_ms, report)?;
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
                }
            }
        }
        // 2. Drain.
        let ids: Vec<ConnectionId> = self.conns.keys().copied().collect();
        for conn in ids {
            match self.conns.get(&conn) {
                Some(Conn::Bound(_)) => self.drain_bound(conn, now_ms, report)?,
                Some(Conn::Retiring(_)) => self.drain_retiring(conn, report),
                None => {}
            }
        }
        // 3. Admit.
        let summary = self.host.pump(now_ms)?;
        report.calls.push(HostCall::Pump { summary });
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
        // 5. Deliver.
        for conn in self.bound_ids() {
            self.deliver(conn, now_ms, report)?;
        }
        // 6. Retire closed peers.
        for conn in self.bound_ids() {
            let Some(bound) = self.bound(conn) else {
                continue;
            };
            if bound.transport_closed.is_none() || !bound.drained || bound.held.is_some() {
                continue;
            }
            let (peer, invitation, event_seen) = (bound.peer, bound.invitation, bound.event_seen);
            self.host.unbind_peer(peer);
            report.calls.push(HostCall::UnbindPeer { peer });
            self.clear_live(invitation, conn);
            if event_seen {
                self.conns.remove(&conn);
            } else {
                self.conns
                    .insert(conn, Conn::Retiring(Retiring { event_seen: false }));
            }
        }
        // 7. Report.
        report.held = self
            .conns
            .values()
            .filter(|entry| matches!(entry, Conn::Bound(bound) if bound.held.is_some()))
            .count();
        Ok(())
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
        // 2. Version.
        if wire_version != self.host.protocol().as_u8() {
            refuse(&mut self.server, report, CloseCode::VersionRefused);
            return Ok(());
        }
        // 3. Supersession: the newest connection wins (B§16 Q3).
        let live = self
            .invitations
            .get(&invitation)
            .and_then(|record| record.live);
        if let Some(old) = live {
            self.fence(old, FenceCause::Superseded, Some(report));
        }
        // 4. Bind.
        let record = self
            .invitations
            .get(&invitation)
            .ok_or(HostError::Invariant("matched invitation vanished"))?;
        let result = self
            .host
            .bind_peer(record.control.clone(), record.grants.clone(), now_ms);
        report.calls.push(HostCall::BindPeer {
            conn,
            invitation,
            result: result.clone(),
        });
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
        // 5. Welcome.
        let epoch = self.host.epoch(peer)?;
        let welcome = Welcome {
            wire_version: self.host.protocol().as_u8(),
            epoch,
        };
        // 6. Result.
        match self.server.accept(conn, welcome) {
            Ok(()) => {
                self.conns.insert(
                    conn,
                    Conn::Bound(Bound {
                        peer,
                        epoch,
                        invitation,
                        acked: 0,
                        frames: 0,
                        held: None,
                        stalled_since: None,
                        transport_closed: None,
                        drained: false,
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
        // 2. Fresh frames.
        for _ in 0..MAX_DRAIN_PER_CONNECTION {
            match self.server.try_recv(conn) {
                Ok(Some(frame)) => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound.frames += 1;
                    }
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

    /// B§5 phase 2 for a retiring connection: drain and discard.
    fn drain_retiring(&mut self, conn: ConnectionId, report: &mut QuicPumpReport) {
        let mut gone = false;
        for _ in 0..MAX_DRAIN_PER_CONNECTION {
            match self.server.try_recv(conn) {
                Ok(Some(_)) => report.frames_discarded += 1,
                Ok(None) => break,
                Err(_) => {
                    gone = true;
                    break;
                }
            }
        }
        let seen = matches!(
            self.conns.get(&conn),
            Some(Conn::Retiring(Retiring { event_seen: true }))
        );
        if gone && seen {
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
        let taken = self.host.take_delivery(peer);
        report.calls.push(HostCall::TakeDelivery {
            peer,
            result: taken.as_ref().map(Vec::len).map_err(Clone::clone),
        });
        let frames = match taken {
            Ok(frames) => frames,
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
            // never blocks admission for the rest (B§8).
            let n = frames.len();
            if n > 0 {
                let through = bound.acked + n as u64;
                self.acknowledge(conn, peer, through, report)?;
                report.frames_discarded += n;
            }
            if let Some(bound) = self.bound_mut(conn) {
                bound.stalled_since = None;
            }
            return Ok(());
        }
        let mut accepted = 0usize;
        let mut fence = false;
        for frame in &frames {
            match self.server.try_send(conn, frame) {
                Ok(()) => accepted += 1,
                Err(SendError::QueueFull) => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound.stalled_since.get_or_insert(now_ms);
                    }
                    break;
                }
                Err(SendError::Closed(reason)) => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound.transport_closed.get_or_insert(reason);
                    }
                    break;
                }
                Err(SendError::UnknownConnection) => {
                    if let Some(bound) = self.bound_mut(conn) {
                        bound
                            .transport_closed
                            .get_or_insert(CloseReason::EndpointFailed);
                    }
                    break;
                }
                Err(SendError::EmptyFrame | SendError::FrameTooLarge | SendError::NotAccepted) => {
                    fence = true;
                    break;
                }
            }
        }
        report.frames_sent += accepted;
        if accepted > 0 {
            let through = self.bound(conn).map_or(0, |bound| bound.acked) + accepted as u64;
            self.acknowledge(conn, peer, through, report)?;
        }
        if fence {
            self.fence(conn, FenceCause::EpochClosed, Some(report));
            return Ok(());
        }
        if accepted == frames.len() {
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

    /// A traced `acknowledge_delivery` that keeps `acked` equal to the
    /// host's watermark.
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
            Ok(()) => {
                if let Some(bound) = self.bound_mut(conn) {
                    bound.acked = through;
                }
                Ok(())
            }
            Err(error) if poisons(&error) => Err(error),
            Err(_) => {
                self.fence(conn, FenceCause::EpochClosed, Some(report));
                Ok(())
            }
        }
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
        let record = self
            .invitations
            .get(&id)
            .ok_or(QuicHostError::UnknownInvitation)?;
        let grants = normalize_grants(grants).map_err(QuicHostError::Host)?;
        let live = record
            .live
            .and_then(|conn| self.bound(conn).map(|bound| (conn, bound.peer)));
        let Some((conn, peer)) = live else {
            self.store_grants(id, grants);
            return Ok(GrantChange::Stored);
        };
        if is_narrowing(&record.grants, &grants) {
            self.store_grants(id, grants);
            return Ok(self.narrowed(conn));
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
        // 2. Flush committed delivery to every open transport.
        for (conn, entry) in &conns {
            let Conn::Bound(bound) = entry else {
                continue;
            };
            if bound.transport_closed.is_some() {
                continue;
            }
            let Ok(frames) = host.take_delivery(bound.peer) else {
                continue;
            };
            let accepted = frames
                .iter()
                .take_while(|frame| server.try_send(*conn, frame).is_ok())
                .count();
            if accepted > 0 {
                let _ = host.acknowledge_delivery(bound.peer, bound.acked + accepted as u64);
            }
        }
        // 3. Count what could not be sent.
        let undelivered_frames = conns
            .values()
            .filter_map(|entry| match entry {
                Conn::Bound(bound) => host.take_delivery(bound.peer).ok().map(|f| f.len()),
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
