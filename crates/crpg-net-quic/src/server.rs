//! The server endpoint: one UDP socket, one I/O thread, one current-thread
//! tokio runtime, and a synchronous, non-blocking API over bounded queues
//! (E§3, E§5, E§6, E§7).

use std::fmt;
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crpg_net::protocol::{MAX_DELTA_FRAME_BYTES, MAX_INTENT_FRAME_BYTES};
use crpg_net::protocol_resume::MAX_RESUME_FRAME_BYTES;
use crpg_net::transport::Transport;
use quinn::{Connection, EndpointConfig, Incoming, RecvStream, SendStream, TokioRuntime};

use crate::client::CloseReport;
use crate::close::{
    ChannelSendError, CloseCode, CloseMode, CloseReason, ConnectionStats, SendError,
};
use crate::handshake::{
    parse_channel_frame_header, parse_frame_header, Channel, Framing, HandshakeError, Hello,
    Welcome, ALPN_LANE0_V1, ALPN_LANE0_V2, FRAME_HEADER_BYTES, MAX_HELLO_BYTES, WELCOME_BYTES,
};
use crate::limits::{ConfigError, ServerConfig, ServerLimits};
use crate::queue::{
    close_quinn, deadline_after, frame_error_code, framed, framed_channel, lane_control,
    lane_reader, lane_writer, race, read_fail, reason_from, wait_stop, Answer, Conn, ConnWake,
    Either, FailGuard, FlushRequest, LaneDirection, Opening, Phase, ReadFail, ReadSpec, Shared,
    State, Stop, SYNC_GRACE,
};
use crate::tls;

/// Never reused within one `QuicServer`; starts at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConnectionId(u64);

impl ConnectionId {
    /// The raw id.
    pub fn get(self) -> u64 {
        self.0
    }

    pub(crate) fn from_raw(raw: u64) -> ConnectionId {
        ConnectionId(raw)
    }
}

/// What the server reports through `poll_event` for a hello opening.
#[derive(Debug, Clone)]
pub enum ServerEvent {
    /// TLS done and a well-formed hello read; awaiting `accept`/`refuse`.
    HelloReceived {
        /// The new connection.
        conn: ConnectionId,
        /// The peer's UDP address.
        remote: SocketAddr,
        /// The hello as sent (credential format-checked only).
        hello: Hello,
    },
    /// The connection ended. Emitted once, only for connections that produced
    /// `HelloReceived` or a `ServerNotice::Resume`.
    Closed {
        /// The connection that ended.
        conn: ConnectionId,
        /// Why it ended.
        reason: CloseReason,
    },
}

/// A resume opening read on a `bind_resumable` server, awaiting
/// `accept_resume` or `refuse`. No `PartialEq`: the request carries a
/// resume proof. `Debug` is hand-written and redacts `request`.
#[derive(Clone)]
pub struct ResumeReceived {
    /// The new connection.
    pub conn: ConnectionId,
    /// The peer's UDP address.
    pub remote: SocketAddr,
    /// The request payload as sent: 1..=4,096 opaque bytes, never inspected here.
    pub request: Vec<u8>,
}

impl fmt::Debug for ResumeReceived {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ResumeReceived {{ conn: {:?}, remote: {:?}, request: <redacted> }}",
            self.conn, self.remote
        )
    }
}

/// Everything `poll_notice` reports, in the one order the I/O side queued it.
#[derive(Debug, Clone)]
pub enum ServerNotice {
    /// A `ServerEvent`, exactly as `poll_event` would return it.
    Event(ServerEvent),
    /// A resume opening (`bind_resumable` servers only).
    Resume(ResumeReceived),
}

/// Why a per-connection server call did nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerError {
    /// No such connection (never visible, or its record was forgotten).
    UnknownConnection,
    /// `accept`/`refuse` on a connection that is not pending.
    NotPending,
    /// The connection was never accepted.
    NotAccepted,
    /// A welcome whose version differs from the hello's, or is not 1 or 2;
    /// on a resume opening, an answer to the wrong opening, or an answer of
    /// 0 or more than 65,536 bytes (`accept_resume`, S§4.5–S§4.6 of
    /// `tasks/T024q.md`). Nothing is sent.
    InvalidWelcome,
    /// The connection is closing or closed.
    Closed(CloseReason),
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            ServerError::UnknownConnection => "UnknownConnection",
            ServerError::NotPending => "NotPending",
            ServerError::NotAccepted => "NotAccepted",
            ServerError::InvalidWelcome => "InvalidWelcome",
            ServerError::Closed(_) => "Closed",
        };
        write!(f, "{name} at quic/server")
    }
}

impl std::error::Error for ServerError {}

/// Why `QuicServer::bind` failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindError {
    /// The limits failed validation.
    Config(ConfigError),
    /// The certificate or key did not load, or do not match.
    Identity,
    /// The UDP socket could not be bound or registered.
    Io(std::io::ErrorKind),
    /// The I/O thread or its runtime could not start.
    Runtime,
}

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            BindError::Config(_) => "Config",
            BindError::Identity => "Identity",
            BindError::Io(_) => "Io",
            BindError::Runtime => "Runtime",
        };
        write!(f, "{name} at quic/bind")
    }
}

impl std::error::Error for BindError {}

/// Host-wide counters and queue totals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueTotals {
    /// Connection slots held against `max_connections` (TLS, hello and
    /// pending included; released when a `Closed` event is polled).
    pub connections: usize,
    /// Connections awaiting `accept`/`refuse`.
    pub pending: usize,
    /// Inbound frames queued across connections.
    pub inbound_frames: usize,
    /// Inbound payload bytes queued across connections.
    pub inbound_bytes: usize,
    /// Outbound frames queued across connections.
    pub outbound_frames: usize,
    /// Outbound payload bytes queued across connections.
    pub outbound_bytes: usize,
    /// Connections that ended before `HelloReceived` (TLS/ALPN/pin failures
    /// and hello-stage closes). They produce no event.
    pub refused_handshakes: u64,
    /// Incoming connections refused at `max_connections` (QUIC CONNECTION_REFUSED).
    pub refused_at_limit: u64,
}

/// What `shutdown` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownReport {
    /// Pending and accepted connections closed by the shutdown.
    pub connections_closed: usize,
    /// Outbound frames not handed to QUIC by the flush deadline.
    pub outbound_frames_discarded: usize,
    /// Inbound frames never drained (queued or parked).
    pub inbound_frames_discarded: usize,
}

/// Owns one UDP socket, one I/O thread and one current-thread tokio runtime.
/// `Debug` is hand-written (local address + totals).
pub struct QuicServer {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    local_addr: SocketAddr,
    limits: ServerLimits,
}

/// The endpoint options `bind`/`bind_dual`/`bind_resumable` select.
#[derive(Clone, Copy)]
struct Setup {
    /// The framings the negotiated ALPN may name.
    framings: &'static [Framing],
    /// A `/2` connection may open with a resume request (`bind_resumable`).
    resumable: bool,
}

fn failed() -> ServerError {
    ServerError::Closed(CloseReason::EndpointFailed)
}

impl QuicServer {
    /// Validates the config, binds the UDP socket on the caller's thread,
    /// and starts the `crpg-quic-server` I/O thread. Returns once the
    /// endpoint is listening.
    pub fn bind(config: ServerConfig) -> Result<QuicServer, BindError> {
        Self::bind_setup(config, &[Framing::V1], false)
    }

    /// As `bind`, but advertising `[crpg-lane0/2, crpg-lane0/1]`. Every
    /// opening is a hello.
    pub fn bind_dual(config: ServerConfig) -> Result<QuicServer, BindError> {
        Self::bind_setup(config, &[Framing::V2, Framing::V1], false)
    }

    /// As `bind_dual`, and a `crpg-lane0/2` connection may open with a
    /// resume request (S§4.4 of `tasks/T024q.md`). Drive it with
    /// `poll_notice`.
    pub fn bind_resumable(config: ServerConfig) -> Result<QuicServer, BindError> {
        Self::bind_setup(config, &[Framing::V2, Framing::V1], true)
    }

    /// The shared constructor: the framings to offer in order and the
    /// `resumable` flag.
    fn bind_setup(
        config: ServerConfig,
        framings: &'static [Framing],
        resumable: bool,
    ) -> Result<QuicServer, BindError> {
        let alpns: &'static [&'static [u8]] = if framings.len() == 1 {
            &[ALPN_LANE0_V1]
        } else {
            &[ALPN_LANE0_V2, ALPN_LANE0_V1]
        };
        let limits = config.limits;
        limits.validate().map_err(BindError::Config)?;
        let quinn_config =
            tls::server_config(&config.identity, alpns, &limits).ok_or(BindError::Identity)?;
        let socket = UdpSocket::bind(config.bind).map_err(|e| BindError::Io(e.kind()))?;
        let local_addr = socket.local_addr().map_err(|e| BindError::Io(e.kind()))?;
        let shared = Arc::new(Shared::new(State::new(
            limits.inbound,
            limits.outbound,
            true,
        )));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread_shared = shared.clone();
        let setup = Setup {
            framings,
            resumable,
        };
        let thread = std::thread::Builder::new()
            .name("crpg-quic-server".into())
            .spawn(move || {
                let guard = FailGuard::new(thread_shared.clone());
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .enable_time()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(_) => {
                        let _ = ready_tx.send(Err(BindError::Runtime));
                        guard.clean();
                        return;
                    }
                };
                runtime.block_on(server_main(
                    thread_shared,
                    socket,
                    quinn_config,
                    limits,
                    setup,
                    ready_tx,
                ));
                guard.clean();
            })
            .map_err(|_| BindError::Runtime)?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(QuicServer {
                shared,
                thread: Some(thread),
                local_addr,
                limits,
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(BindError::Runtime)
            }
        }
    }

    /// The bound UDP address.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The next event, if any: a view over the notice queue. While a
    /// `ServerNotice::Resume` is at the head, it returns `None` and leaves
    /// every notice queued (only `poll_notice` may take it, S§8.3 of
    /// `tasks/T024q.md`). Polling a `Closed` event releases the connection's
    /// slot.
    pub fn poll_event(&mut self) -> Option<ServerEvent> {
        let mut state = self.shared.lock_any()?;
        if !matches!(state.events.front(), Some(ServerNotice::Event(_))) {
            return None;
        }
        let Some(ServerNotice::Event(event)) = state.events.pop_front() else {
            return None;
        };
        if let ServerEvent::Closed { conn, .. } = &event {
            state.live = state.live.saturating_sub(1);
            let id = conn.get();
            let forget = state.conns.get_mut(&id).is_some_and(|record| {
                record.event_polled = true;
                if record.held_frames {
                    record.closed_reported
                } else {
                    true
                }
            });
            if forget {
                state.release(id);
            }
        }
        Some(event)
    }

    /// The next notice, if any: every event and resume arrival in the one
    /// order the I/O side queued it. Polling `Event(Closed { .. })` releases
    /// the connection's slot exactly as `poll_event` does (S§8.5).
    pub fn poll_notice(&mut self) -> Option<ServerNotice> {
        let mut state = self.shared.lock_any()?;
        let notice = state.events.pop_front()?;
        if let ServerNotice::Event(ServerEvent::Closed { conn, .. }) = &notice {
            state.live = state.live.saturating_sub(1);
            let id = conn.get();
            let forget = state.conns.get_mut(&id).is_some_and(|record| {
                record.event_polled = true;
                if record.held_frames {
                    record.closed_reported
                } else {
                    true
                }
            });
            if forget {
                state.release(id);
            }
        }
        Some(notice)
    }

    /// Blocks up to `timeout` until an event, a notice or any inbound frame
    /// is queued; `false` on timeout. Never needed by a pump that polls on
    /// its own tick.
    pub fn wait(&self, timeout: Duration) -> bool {
        let now = std::time::Instant::now();
        let deadline = now.checked_add(timeout).unwrap_or(now);
        let Some(state) = self.shared.lock_any() else {
            return false;
        };
        let ready = |state: &State| !state.events.is_empty() || state.inbound_frames > 0;
        match self.shared.wait_until(state, deadline, ready) {
            Some(state) => ready(&state),
            None => false,
        }
    }

    /// Accepts a pending connection: the welcome is written first, then lane
    /// frames flow. On a resume opening this is refused with
    /// `InvalidWelcome` (use `accept_resume`), after the same first three
    /// checks. Precedence: `UnknownConnection` → `Closed` → `NotPending` →
    /// `InvalidWelcome` (nothing is sent).
    pub fn accept(&mut self, conn: ConnectionId, welcome: Welcome) -> Result<(), ServerError> {
        let mut state = self.shared.lock().ok_or_else(failed)?;
        let record = visible(&mut state, conn)?;
        if let Some(reason) = record.close_state() {
            return Err(ServerError::Closed(reason));
        }
        if record.phase != Phase::Pending {
            return Err(ServerError::NotPending);
        }
        if record.opening != Opening::Hello {
            return Err(ServerError::InvalidWelcome);
        }
        if welcome.wire_version != record.hello_version || welcome.encode().is_err() {
            return Err(ServerError::InvalidWelcome);
        }
        record.phase = Phase::Accepted;
        record.answer = Some(Answer::Welcome(welcome));
        record.reader_active = true;
        record.wake.control.notify_one();
        Ok(())
    }

    /// Accepts a pending resume opening: the answer is written as the first
    /// downstream `Resume` frame, in place of the welcome (S§4.5), then lane
    /// frames flow. Nothing is sent on a refusal. Precedence:
    /// `UnknownConnection` → `Closed` → `NotPending` → `InvalidWelcome`
    /// (the opening was a hello, or the answer is empty or above 65,536
    /// bytes).
    pub fn accept_resume(&mut self, conn: ConnectionId, answer: &[u8]) -> Result<(), ServerError> {
        let mut state = self.shared.lock().ok_or_else(failed)?;
        let record = visible(&mut state, conn)?;
        if let Some(reason) = record.close_state() {
            return Err(ServerError::Closed(reason));
        }
        if record.phase != Phase::Pending {
            return Err(ServerError::NotPending);
        }
        if record.opening != Opening::Resume
            || answer.is_empty()
            || answer.len() > MAX_DELTA_FRAME_BYTES
        {
            return Err(ServerError::InvalidWelcome);
        }
        record.phase = Phase::Accepted;
        record.answer = Some(Answer::Resume(answer.to_vec()));
        record.reader_active = true;
        record.wake.control.notify_one();
        Ok(())
    }

    /// Refuses a pending connection: it closes at once with `code`, and
    /// `Closed { Local(code) }` follows. Precedence: `UnknownConnection` →
    /// `Closed` → `NotPending`.
    pub fn refuse(&mut self, conn: ConnectionId, code: CloseCode) -> Result<(), ServerError> {
        let mut state = self.shared.lock().ok_or_else(failed)?;
        let record = visible(&mut state, conn)?;
        if let Some(reason) = record.close_state() {
            return Err(ServerError::Closed(reason));
        }
        if record.phase != Phase::Pending {
            return Err(ServerError::NotPending);
        }
        record.quinn_close = Some(code);
        state.record_close(conn.get(), CloseReason::Local(code));
        drop(state);
        self.shared.notify_sync();
        Ok(())
    }

    /// The next inbound lane-0 frame: a view over the same queue. While a
    /// transfer or resume frame is at the head it returns `Ok(None)` and
    /// leaves it queued (never reports `Closed` while such a frame is
    /// queued); only `try_recv_channel` can take it (S§8.3). Precedence:
    /// `UnknownConnection` → `NotAccepted` → a queued lane-0 frame →
    /// `Closed` once closed and drained → `Ok(None)`.
    pub fn try_recv(&mut self, conn: ConnectionId) -> Result<Option<Vec<u8>>, ServerError> {
        self.recv_view(conn, true)
            .map(|next| next.map(|(_, frame)| frame))
    }

    /// As `try_recv`, with the frame's channel. On a `/1` connection every
    /// frame is `Channel::Lane0`; on the server no frame is ever
    /// `Channel::Resume` (S§8.2).
    pub fn try_recv_channel(
        &mut self,
        conn: ConnectionId,
    ) -> Result<Option<(Channel, Vec<u8>)>, ServerError> {
        self.recv_view(conn, false)
    }

    /// The shared body of `try_recv` (head-is-lane-0 view) and
    /// `try_recv_channel` (any channel).
    fn recv_view(
        &mut self,
        conn: ConnectionId,
        lane0_only: bool,
    ) -> Result<Option<(Channel, Vec<u8>)>, ServerError> {
        let mut state = self.shared.lock().ok_or_else(failed)?;
        let record = visible(&mut state, conn)?;
        if record.phase != Phase::Accepted {
            return Err(ServerError::NotAccepted);
        }
        let id = conn.get();
        if let Some(next) = if lane0_only {
            state
                .pop_inbound_lane0(id)
                .map(|frame| (Channel::Lane0, frame))
        } else {
            state.pop_inbound(id)
        } {
            return Ok(Some(next));
        }
        let Some(record) = state.conns.get_mut(&id) else {
            return Err(ServerError::UnknownConnection);
        };
        // While a non-lane-0 frame is at the head, the close waits for it
        // (S§8.3).
        if record
            .inbound_head()
            .is_some_and(|channel| channel != Channel::Lane0)
        {
            return Ok(None);
        }
        if record.published {
            let reason = record.reason.unwrap_or(CloseReason::EndpointFailed);
            record.closed_reported = true;
            if record.event_polled {
                state.release(id);
            }
            return Err(ServerError::Closed(reason));
        }
        Ok(None)
    }

    /// Queues one lane frame (≤ 65,536 bytes): `try_send_channel` on
    /// `Channel::Lane0`, whose `ChannelUnavailable` cannot occur.
    pub fn try_send(&mut self, conn: ConnectionId, frame: &[u8]) -> Result<(), SendError> {
        match self.try_send_channel(conn, Channel::Lane0, frame) {
            Ok(()) => Ok(()),
            Err(ChannelSendError::Send(error)) => Err(error),
            // Lane 0 is available on every framing; treat the impossible arm
            // as an endpoint-level failure, not a panic.
            Err(ChannelSendError::ChannelUnavailable) => {
                Err(SendError::Closed(CloseReason::EndpointFailed))
            }
        }
    }

    /// Queues one lane frame (≤ 65,536 bytes) on `channel`. Precedence:
    /// `Send(UnknownConnection)` → `Send(NotAccepted)` → `Send(Closed(_))` →
    /// `ChannelUnavailable` (`Transfer` or `Resume` on a `/1` connection) →
    /// `Send(EmptyFrame)` → `Send(FrameTooLarge)` → `Send(QueueFull)`
    /// (retryable; nothing queued).
    pub fn try_send_channel(
        &mut self,
        conn: ConnectionId,
        channel: Channel,
        frame: &[u8],
    ) -> Result<(), ChannelSendError> {
        let mut state = self
            .shared
            .lock()
            .ok_or(ChannelSendError::Send(SendError::Closed(
                CloseReason::EndpointFailed,
            )))?;
        let record = visible(&mut state, conn)
            .map_err(|_| ChannelSendError::Send(SendError::UnknownConnection))?;
        if record.phase != Phase::Accepted {
            return Err(ChannelSendError::Send(SendError::NotAccepted));
        }
        if let Some(reason) = record.close_state() {
            return Err(ChannelSendError::Send(SendError::Closed(reason)));
        }
        if record.framing == Framing::V1 && channel != Channel::Lane0 {
            return Err(ChannelSendError::ChannelUnavailable);
        }
        if frame.is_empty() {
            return Err(ChannelSendError::Send(SendError::EmptyFrame));
        }
        if frame.len() > MAX_DELTA_FRAME_BYTES {
            return Err(ChannelSendError::Send(SendError::FrameTooLarge));
        }
        state
            .queue_outbound(conn.get(), channel, frame)
            .map_err(ChannelSendError::Send)
    }

    /// The connection's negotiated framing. Error: `UnknownConnection` (the
    /// same visibility rule as `stats`: hello stage and forgotten records
    /// are unknown).
    pub fn framing(&self, conn: ConnectionId) -> Result<Framing, ServerError> {
        let mut state = self.shared.lock().ok_or_else(failed)?;
        let record = visible(&mut state, conn)?;
        Ok(record.framing)
    }

    /// Closes a pending or accepted connection. Inbound frames stay
    /// drainable after a server-side close, so the report's
    /// `inbound_frames_discarded` is always 0 here. `Flush` blocks up to
    /// `close_flush_timeout_ms`. Precedence: `UnknownConnection` →
    /// `Closed` (already closing or closed).
    pub fn close(
        &mut self,
        conn: ConnectionId,
        code: CloseCode,
        mode: CloseMode,
    ) -> Result<CloseReport, ServerError> {
        let mut state = self.shared.lock().ok_or_else(failed)?;
        let id = conn.get();
        let record = visible(&mut state, conn)?;
        if let Some(reason) = record.close_state() {
            return Err(ServerError::Closed(reason));
        }
        let before = record.discarded;
        let mut report = CloseReport {
            outbound_frames_discarded: 0,
            inbound_frames_discarded: 0,
        };
        if record.phase == Phase::Pending || mode == CloseMode::Discard {
            record.quinn_close = Some(code);
            state.record_close(id, CloseReason::Local(code));
            report.outbound_frames_discarded = discarded_since(&state, id, before);
            drop(state);
            self.shared.notify_sync();
            return Ok(report);
        }
        let deadline = deadline_after(self.limits.close_flush_timeout_ms);
        record.closing = Some(code);
        record.flush = Some(FlushRequest {
            code,
            deadline,
            started: false,
        });
        record.wake.control.notify_one();
        let guard_deadline = deadline.checked_add(SYNC_GRACE).unwrap_or(deadline);
        let state = self.shared.wait_until(state, guard_deadline, |state| {
            state
                .conns
                .get(&id)
                .is_none_or(|record| record.close_done || record.control_done)
        });
        if let Some(state) = state {
            report.outbound_frames_discarded = discarded_since(&state, id, before);
        }
        Ok(report)
    }

    /// A snapshot of one connection's queues.
    pub fn stats(&self, conn: ConnectionId) -> Result<ConnectionStats, ServerError> {
        let (stats, connection) = {
            let mut state = self.shared.lock().ok_or_else(failed)?;
            let record = visible(&mut state, conn)?;
            (record.stats(0), record.connection.clone())
        };
        Ok(ConnectionStats {
            keepalive_pings_sent: connection.stats().frame_tx.ping,
            ..stats
        })
    }

    /// Host-wide totals and counters.
    pub fn totals(&self) -> QueueTotals {
        match self.shared.lock_any() {
            Some(state) => QueueTotals {
                connections: state.live,
                pending: state
                    .conns
                    .values()
                    .filter(|record| record.phase == Phase::Pending && record.reason.is_none())
                    .count(),
                inbound_frames: state.inbound_frames,
                inbound_bytes: state.inbound_bytes,
                outbound_frames: state.outbound_frames,
                outbound_bytes: state.outbound_bytes,
                refused_handshakes: state.refused_handshakes,
                refused_at_limit: state.refused_at_limit,
            },
            None => QueueTotals {
                connections: 0,
                pending: 0,
                inbound_frames: 0,
                inbound_bytes: 0,
                outbound_frames: 0,
                outbound_bytes: 0,
                refused_handshakes: 0,
                refused_at_limit: 0,
            },
        }
    }

    /// One accepted connection as a single-channel `Transport`.
    pub fn lane(&mut self, conn: ConnectionId) -> Result<ServerLane<'_>, ServerError> {
        {
            let mut state = self.shared.lock().ok_or_else(failed)?;
            let record = visible(&mut state, conn)?;
            if record.phase != Phase::Accepted {
                return Err(ServerError::NotAccepted);
            }
        }
        Ok(ServerLane { server: self, conn })
    }

    /// Graceful: stop accepting, close pending with `Shutdown`, flush accepted
    /// connections (`Flush`, one shared deadline), close all with `Shutdown`,
    /// wait for the endpoint to go idle (bounded by the same deadline), join.
    pub fn shutdown(mut self) -> ShutdownReport {
        let mut report = ShutdownReport {
            connections_closed: 0,
            outbound_frames_discarded: 0,
            inbound_frames_discarded: 0,
        };
        let deadline = deadline_after(self.limits.close_flush_timeout_ms);
        if let Some(mut state) = self.shared.lock() {
            state.stopping = true;
            let ids: Vec<u64> = state.conns.keys().copied().collect();
            let mut flushing: Vec<(u64, usize)> = Vec::new();
            for id in ids {
                let Some(record) = state.conns.get_mut(&id) else {
                    continue;
                };
                if record.phase == Phase::Hello || record.close_state().is_some() {
                    continue;
                }
                report.connections_closed += 1;
                if record.phase == Phase::Pending {
                    record.quinn_close = Some(CloseCode::Shutdown);
                    state.record_close(id, CloseReason::Local(CloseCode::Shutdown));
                } else {
                    record.closing = Some(CloseCode::Shutdown);
                    record.flush = Some(FlushRequest {
                        code: CloseCode::Shutdown,
                        deadline,
                        started: false,
                    });
                    record.wake.control.notify_one();
                    flushing.push((id, record.discarded));
                }
            }
            let guard_deadline = deadline.checked_add(SYNC_GRACE).unwrap_or(deadline);
            let waited = self.shared.wait_until(state, guard_deadline, |state| {
                flushing.iter().all(|(id, _)| {
                    state
                        .conns
                        .get(id)
                        .is_none_or(|record| record.close_done || record.control_done)
                })
            });
            if let Some(mut state) = waited {
                for (id, before) in &flushing {
                    report.outbound_frames_discarded += discarded_since(&state, *id, *before);
                }
                report.inbound_frames_discarded =
                    state.conns.values().map(Conn::held_inbound_frames).sum();
                state.stop = Some(Stop {
                    code: CloseCode::Shutdown,
                    deadline,
                });
            }
        }
        self.shared.endpoint_wake.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        report
    }
}

/// Abrupt: every connection closes with `Shutdown` in `Discard` mode, then the
/// endpoint is given up to `close_flush_timeout_ms` to transmit the
/// CONNECTION_CLOSE frames (`wait_idle`), then the thread is joined.
impl Drop for QuicServer {
    fn drop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        let deadline = deadline_after(self.limits.close_flush_timeout_ms);
        if let Some(mut state) = self.shared.lock_any() {
            state.stopping = true;
            if state.stop.is_none() {
                state.stop = Some(Stop {
                    code: CloseCode::Shutdown,
                    deadline,
                });
            }
        }
        self.shared.endpoint_wake.notify_one();
        let _ = thread.join();
    }
}

impl fmt::Debug for QuicServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuicServer")
            .field("local_addr", &self.local_addr)
            .field("totals", &self.totals())
            .finish()
    }
}

/// One accepted connection viewed as the net-local single-channel `Transport`.
#[derive(Debug)]
pub struct ServerLane<'a> {
    server: &'a mut QuicServer,
    conn: ConnectionId,
}

impl Transport for ServerLane<'_> {
    type Error = SendError;

    /// `QuicServer::try_send` on this connection.
    fn send(&mut self, bytes: &[u8]) -> Result<(), SendError> {
        self.server.try_send(self.conn, bytes)
    }

    /// `QuicServer::try_recv` on this connection, errors folded into `None`.
    fn recv(&mut self) -> Option<Vec<u8>> {
        self.server.try_recv(self.conn).ok().flatten()
    }
}

/// The record of a connection the API may name (hello stage excluded).
fn visible(state: &mut State, conn: ConnectionId) -> Result<&mut Conn, ServerError> {
    match state.conns.get_mut(&conn.get()) {
        Some(record) if record.phase != Phase::Hello && !record.released => Ok(record),
        _ => Err(ServerError::UnknownConnection),
    }
}

fn discarded_since(state: &State, id: u64, before: usize) -> usize {
    state
        .conns
        .get(&id)
        .map_or(0, |record| record.discarded.saturating_sub(before))
}

// ---------------------------------------------------------------------------
// I/O thread.
// ---------------------------------------------------------------------------

async fn server_main(
    shared: Arc<Shared>,
    socket: UdpSocket,
    config: quinn::ServerConfig,
    limits: ServerLimits,
    setup: Setup,
    ready: mpsc::SyncSender<Result<(), BindError>>,
) {
    let endpoint = match quinn::Endpoint::new(
        EndpointConfig::default(),
        Some(config),
        socket,
        Arc::new(TokioRuntime),
    ) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            let _ = ready.send(Err(BindError::Io(error.kind())));
            return;
        }
    };
    let _ = ready.send(Ok(()));
    let stop = loop {
        match race(endpoint.accept(), wait_stop(&shared)).await {
            Either::Left(Some(incoming)) => admit(&shared, incoming, limits, setup),
            Either::Left(None) => break wait_stop(&shared).await,
            Either::Right(stop) => break stop,
        }
    };
    close_quinn_endpoint(&endpoint, stop.code);
    let deadline = tokio::time::Instant::from_std(stop.deadline);
    let _ = tokio::time::timeout_at(deadline, endpoint.wait_idle()).await;
}

fn close_quinn_endpoint(endpoint: &quinn::Endpoint, code: CloseCode) {
    endpoint.close(
        quinn::VarInt::from_u32(code.as_u32()),
        code.name().as_bytes(),
    );
}

/// E§5 step 1: refuse at `max_connections` before any crypto.
fn admit(shared: &Arc<Shared>, incoming: Incoming, limits: ServerLimits, setup: Setup) {
    let admitted = match shared.lock_any() {
        Some(mut state) => {
            if state.stopping {
                false
            } else if state.live >= limits.max_connections {
                state.refused_at_limit += 1;
                false
            } else {
                state.live += 1;
                true
            }
        }
        None => false,
    };
    shared.notify_sync();
    if admitted {
        tokio::spawn(serve_connection(shared.clone(), incoming, limits, setup));
    } else {
        incoming.refuse();
    }
}

/// Ends a connection that never produced `HelloReceived`.
fn end_handshake(shared: &Shared, id: Option<u64>) {
    if let Some(mut state) = shared.lock_any() {
        if let Some(id) = id {
            state.conns.remove(&id);
        }
        state.live = state.live.saturating_sub(1);
        state.refused_handshakes += 1;
    }
    shared.notify_sync();
}

/// The framing the client offered; `None` for any other or no ALPN.
fn negotiated_framing(connection: &Connection) -> Option<Framing> {
    let protocol = connection
        .handshake_data()
        .and_then(|data| data.downcast::<quinn::crypto::rustls::HandshakeData>().ok())
        .and_then(|data| data.protocol.clone())?;
    Framing::from_alpn(&protocol)
}

enum OpeningFail {
    Code(CloseCode),
    Lost,
}

/// What the client's first lane-stream frame was (`tasks/T024q.md` S§4.4).
enum OpeningPart {
    /// A hello; answered with `accept(welcome)`.
    Hello(Hello),
    /// A resume request (1..=4,096 opaque bytes); answered with
    /// `accept_resume(answer)`.
    Resume(Vec<u8>),
}

/// E§5 step 3 under T024q S§4.4: the client's first bidi stream, then the
/// 4-byte opening header, then either the resume branch (a `bind_resumable`
/// server over `/2` with `header[0] == 0x91`) or today's hello path.
async fn read_opening(
    connection: &Connection,
    framing: Framing,
    resumable: bool,
) -> Result<(SendStream, RecvStream, OpeningPart), OpeningFail> {
    let (send, mut recv) = connection
        .accept_bi()
        .await
        .map_err(|_| OpeningFail::Lost)?;
    let mut header = [0u8; FRAME_HEADER_BYTES];
    match recv.read_exact(&mut header).await {
        Ok(()) => {}
        // FIN before a complete header.
        Err(error) => return Err(status_err(read_fail(error))),
    }
    // S§4.4 step 2: the resume opening, only where it is legal.
    if resumable && framing == Framing::V2 && header[0] == Channel::Resume.as_u8() {
        let len = match parse_channel_frame_header(header, MAX_RESUME_FRAME_BYTES) {
            Ok((_, len)) => len,
            Err(error) => return Err(OpeningFail::Code(frame_error_code(error))),
        };
        let mut request = vec![0u8; len];
        match recv.read_exact(&mut request).await {
            Ok(()) => {}
            Err(error) => return Err(status_err(read_fail(error))),
        }
        return Ok((send, recv, OpeningPart::Resume(request)));
    }
    // S§4.4 step 3: the hello opening, today's code path.
    let len = match parse_frame_header(header, MAX_HELLO_BYTES) {
        Ok(len) => len,
        Err(error) => return Err(OpeningFail::Code(frame_error_code(error))),
    };
    let mut body = vec![0u8; len];
    match recv.read_exact(&mut body).await {
        Ok(()) => {}
        Err(error) => return Err(status_err(read_fail(error))),
    }
    let hello = match Hello::decode(&body) {
        Ok(hello) => hello,
        Err(HandshakeError::UnsupportedWireVersion) => {
            return Err(OpeningFail::Code(CloseCode::VersionRefused))
        }
        Err(HandshakeError::Malformed | HandshakeError::CredentialLength) => {
            return Err(OpeningFail::Code(CloseCode::ProtocolViolation))
        }
    };
    Ok((send, recv, OpeningPart::Hello(hello)))
}

fn status_err(fail: ReadFail) -> OpeningFail {
    match fail {
        ReadFail::Code(code) => OpeningFail::Code(code),
        ReadFail::Lost => OpeningFail::Lost,
    }
}

async fn serve_connection(
    shared: Arc<Shared>,
    incoming: Incoming,
    limits: ServerLimits,
    setup: Setup,
) {
    // E§5 step 2: TLS 1.3 handshake.
    let connection = match incoming.accept() {
        Ok(connecting) => connecting.await.ok(),
        Err(_) => None,
    };
    let Some(connection) = connection else {
        end_handshake(&shared, None);
        return;
    };
    let framing = match negotiated_framing(&connection) {
        Some(framing) if setup.framings.contains(&framing) => framing,
        _ => {
            close_quinn(&connection, CloseCode::ProtocolViolation);
            end_handshake(&shared, None);
            return;
        }
    };
    // E§5 step 3: assign the id, read the opening frame under its timer.
    let wake = Arc::new(ConnWake::default());
    let id = match shared.lock_any() {
        Some(mut state) => {
            let id = state.next_id;
            state.next_id += 1;
            let mut record = Conn::new(Phase::Hello, connection.clone(), wake.clone());
            // The negotiated framing is known here and never changes.
            record.framing = framing;
            state.conns.insert(id, record);
            id
        }
        None => return,
    };
    let hello_timeout = Duration::from_millis(u64::from(limits.hello_timeout_ms));
    let (send, recv, part) = match tokio::time::timeout(
        hello_timeout,
        read_opening(&connection, framing, setup.resumable),
    )
    .await
    {
        Ok(Ok(parts)) => parts,
        Ok(Err(OpeningFail::Code(code))) => {
            close_quinn(&connection, code);
            end_handshake(&shared, Some(id));
            return;
        }
        Ok(Err(OpeningFail::Lost)) => {
            end_handshake(&shared, Some(id));
            return;
        }
        Err(_elapsed) => {
            close_quinn(&connection, CloseCode::HelloTimeout);
            end_handshake(&shared, Some(id));
            return;
        }
    };
    // E§5 step 4: the opening notice (event or resume), then the decision
    // timer.
    match part {
        OpeningPart::Hello(hello) => {
            if let Some(mut state) = shared.lock_any() {
                if let Some(record) = state.conns.get_mut(&id) {
                    record.phase = Phase::Pending;
                    record.opening = Opening::Hello;
                    record.hello_version = hello.wire_version;
                }
                state
                    .events
                    .push_back(ServerNotice::Event(ServerEvent::HelloReceived {
                        conn: ConnectionId(id),
                        remote: connection.remote_address(),
                        hello,
                    }));
            }
        }
        OpeningPart::Resume(request) => {
            if let Some(mut state) = shared.lock_any() {
                if let Some(record) = state.conns.get_mut(&id) {
                    record.phase = Phase::Pending;
                    record.opening = Opening::Resume;
                }
                state.events.push_back(ServerNotice::Resume(ResumeReceived {
                    conn: ConnectionId(id),
                    remote: connection.remote_address(),
                    request,
                }));
            }
        }
    }
    shared.notify_sync();
    let answer = await_decision(&shared, id, &connection, &wake, limits).await;
    // E§5 step 5: on accept, the welcome or resume answer first, then the
    // lane tasks.
    match answer {
        Some(answer) => {
            start_lane(
                &shared,
                id,
                &connection,
                &wake,
                LaneStreams { send, recv },
                framing,
                answer,
            )
            .await
        }
        None => {
            // Closed while pending (or between the accept and the answer):
            // no reader will run, so release its claim and publish the close.
            if let Some(mut state) = shared.lock_any() {
                state.reader_finished(id);
            }
            shared.notify_sync();
        }
    }
    lane_control(shared, id, connection, wake).await;
}

/// Waits for `accept`/`accept_resume`/`refuse` up to `decision_timeout_ms`;
/// on expiry closes with `AuthTimeout`. `Some(answer)` once accepted.
async fn await_decision(
    shared: &Shared,
    id: u64,
    connection: &Connection,
    wake: &ConnWake,
    limits: ServerLimits,
) -> Option<Answer> {
    let deadline =
        tokio::time::Instant::now() + Duration::from_millis(u64::from(limits.decision_timeout_ms));
    loop {
        {
            let mut state = shared.lock_any()?;
            let record = state.conns.get_mut(&id)?;
            if record.reason.is_some() {
                return None;
            }
            if let Some(answer) = record.answer.take() {
                return Some(answer);
            }
        }
        let woke = race(
            race(wake.control.notified(), tokio::time::sleep_until(deadline)),
            connection.closed(),
        )
        .await;
        match woke {
            Either::Left(Either::Left(())) => {}
            Either::Left(Either::Right(())) => {
                let recorded = shared.lock_any().is_some_and(|mut state| {
                    let pending = state
                        .conns
                        .get(&id)
                        .is_some_and(|record| record.phase == Phase::Pending);
                    pending && state.record_close(id, CloseReason::Local(CloseCode::AuthTimeout))
                });
                shared.notify_sync();
                if recorded {
                    close_quinn(connection, CloseCode::AuthTimeout);
                }
            }
            Either::Right(error) => {
                if let Some(mut state) = shared.lock_any() {
                    state.record_close(id, reason_from(&error));
                }
                shared.notify_sync();
                return None;
            }
        }
    }
}

/// The lane stream pair an accepted connection hands to its two tasks.
struct LaneStreams {
    send: SendStream,
    recv: RecvStream,
}

/// Writes the welcome or the resume answer as the first downstream frame,
/// then starts the lane writer and reader. If the frame cannot be written
/// the connection is closed and the reader's claim released.
async fn start_lane(
    shared: &Arc<Shared>,
    id: u64,
    connection: &Connection,
    wake: &Arc<ConnWake>,
    streams: LaneStreams,
    framing: Framing,
    answer: Answer,
) {
    let LaneStreams { mut send, recv } = streams;
    let written = match &answer {
        Answer::Welcome(welcome) => match welcome.encode() {
            Ok(bytes) => match framed(&bytes, WELCOME_BYTES) {
                Ok(frame) => send.write_all(&frame).await,
                Err(_) => Err(quinn::WriteError::ClosedStream),
            },
            Err(_) => Err(quinn::WriteError::ClosedStream),
        },
        Answer::Resume(payload) => {
            match framed_channel(Channel::Resume, payload, MAX_DELTA_FRAME_BYTES) {
                Ok(frame) => send.write_all(&frame).await,
                Err(_) => Err(quinn::WriteError::ClosedStream),
            }
        }
    };
    match written {
        Ok(()) => {
            tokio::spawn(lane_writer(
                shared.clone(),
                id,
                connection.clone(),
                send,
                wake.clone(),
            ));
            tokio::spawn(lane_reader(
                shared.clone(),
                id,
                connection.clone(),
                recv,
                ReadSpec {
                    framing,
                    direction: LaneDirection::Upstream,
                    cap: MAX_INTENT_FRAME_BYTES,
                },
                wake.clone(),
            ));
        }
        Err(error) => {
            match error {
                quinn::WriteError::ConnectionLost(lost) => {
                    if let Some(mut state) = shared.lock_any() {
                        state.record_close(id, reason_from(&lost));
                    }
                }
                _ => {
                    crate::queue::local_close(shared, id, connection, CloseCode::ProtocolViolation)
                }
            }
            if let Some(mut state) = shared.lock_any() {
                state.reader_finished(id);
            }
            shared.notify_sync();
        }
    }
}
