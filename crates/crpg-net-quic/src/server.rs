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
use crpg_net::transport::Transport;
use quinn::{Connection, EndpointConfig, Incoming, RecvStream, SendStream, TokioRuntime};

use crate::client::CloseReport;
use crate::close::{CloseCode, CloseMode, CloseReason, ConnectionStats, SendError};
use crate::handshake::{
    HandshakeError, Hello, Welcome, ALPN_LANE0_V1, MAX_HELLO_BYTES, WELCOME_BYTES,
};
use crate::limits::{ConfigError, ServerConfig, ServerLimits};
use crate::queue::{
    close_quinn, deadline_after, framed, lane_control, lane_reader, lane_writer, race, read_frame,
    reason_from, wait_stop, Conn, ConnWake, Either, FailGuard, FlushRequest, Phase, ReadFail,
    Shared, State, Stop, SYNC_GRACE,
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

/// What the server reports through `poll_event`.
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
    /// `HelloReceived`.
    Closed {
        /// The connection that ended.
        conn: ConnectionId,
        /// Why it ended.
        reason: CloseReason,
    },
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
    /// A welcome whose version differs from the hello's, or is not 1 or 2.
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

fn failed() -> ServerError {
    ServerError::Closed(CloseReason::EndpointFailed)
}

impl QuicServer {
    /// Validates the config, binds the UDP socket on the caller's thread,
    /// and starts the `crpg-quic-server` I/O thread. Returns once the
    /// endpoint is listening.
    pub fn bind(config: ServerConfig) -> Result<QuicServer, BindError> {
        let limits = config.limits;
        limits.validate().map_err(BindError::Config)?;
        let quinn_config = tls::server_config(
            &config.identity,
            limits.idle_timeout_ms,
            limits.keep_alive_ms,
            limits.max_connections,
        )
        .ok_or(BindError::Identity)?;
        let socket = UdpSocket::bind(config.bind).map_err(|e| BindError::Io(e.kind()))?;
        let local_addr = socket.local_addr().map_err(|e| BindError::Io(e.kind()))?;
        let shared = Arc::new(Shared::new(State::new(
            limits.inbound,
            limits.outbound,
            true,
        )));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread_shared = shared.clone();
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

    /// The next event, if any. Polling a `Closed` event releases the
    /// connection's slot.
    pub fn poll_event(&mut self) -> Option<ServerEvent> {
        let mut state = self.shared.lock_any()?;
        let event = state.events.pop_front()?;
        if let ServerEvent::Closed { conn, .. } = &event {
            state.live = state.live.saturating_sub(1);
            let id = conn.get();
            let forget = state.conns.get_mut(&id).is_some_and(|record| {
                record.event_polled = true;
                record.closed_reported || record.drained()
            });
            if forget {
                state.conns.remove(&id);
            }
        }
        Some(event)
    }

    /// Blocks up to `timeout` until an event or any inbound frame is queued;
    /// `false` on timeout. Never needed by a pump that polls on its own tick.
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
    /// frames flow. Precedence: `UnknownConnection` → `Closed` →
    /// `NotPending` → `InvalidWelcome` (nothing is sent).
    pub fn accept(&mut self, conn: ConnectionId, welcome: Welcome) -> Result<(), ServerError> {
        let mut state = self.shared.lock().ok_or_else(failed)?;
        let record = visible(&mut state, conn)?;
        if let Some(reason) = record.close_state() {
            return Err(ServerError::Closed(reason));
        }
        if record.phase != Phase::Pending {
            return Err(ServerError::NotPending);
        }
        if welcome.wire_version != record.hello_version || welcome.encode().is_err() {
            return Err(ServerError::InvalidWelcome);
        }
        record.phase = Phase::Accepted;
        record.welcome = Some(welcome);
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

    /// The next complete inbound frame. Precedence: `UnknownConnection` →
    /// `NotAccepted` → a queued frame → `Closed` once closed and drained →
    /// `Ok(None)`.
    pub fn try_recv(&mut self, conn: ConnectionId) -> Result<Option<Vec<u8>>, ServerError> {
        let mut state = self.shared.lock().ok_or_else(failed)?;
        let record = visible(&mut state, conn)?;
        if record.phase != Phase::Accepted {
            return Err(ServerError::NotAccepted);
        }
        let id = conn.get();
        if let Some(frame) = state.pop_inbound(id) {
            return Ok(Some(frame));
        }
        let Some(record) = state.conns.get_mut(&id) else {
            return Err(ServerError::UnknownConnection);
        };
        if record.published {
            let reason = record.reason.unwrap_or(CloseReason::EndpointFailed);
            record.closed_reported = true;
            if record.event_polled {
                state.conns.remove(&id);
            }
            return Err(ServerError::Closed(reason));
        }
        Ok(None)
    }

    /// Queues one lane frame (≤ 65,536 bytes). Precedence:
    /// `UnknownConnection` → `NotAccepted` → `Closed` → `EmptyFrame` →
    /// `FrameTooLarge` → `QueueFull` (retryable; nothing queued).
    pub fn try_send(&mut self, conn: ConnectionId, frame: &[u8]) -> Result<(), SendError> {
        let mut state = self
            .shared
            .lock()
            .ok_or(SendError::Closed(CloseReason::EndpointFailed))?;
        let record = visible(&mut state, conn).map_err(|_| SendError::UnknownConnection)?;
        if record.phase != Phase::Accepted {
            return Err(SendError::NotAccepted);
        }
        if let Some(reason) = record.close_state() {
            return Err(SendError::Closed(reason));
        }
        if frame.is_empty() {
            return Err(SendError::EmptyFrame);
        }
        if frame.len() > MAX_DELTA_FRAME_BYTES {
            return Err(SendError::FrameTooLarge);
        }
        state.queue_outbound(conn.get(), frame)
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
        Some(record) if record.phase != Phase::Hello => Ok(record),
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
            Either::Left(Some(incoming)) => admit(&shared, incoming, limits),
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
fn admit(shared: &Arc<Shared>, incoming: Incoming, limits: ServerLimits) {
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
        tokio::spawn(serve_connection(shared.clone(), incoming, limits));
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

fn alpn_matches(connection: &Connection) -> bool {
    connection
        .handshake_data()
        .and_then(|data| data.downcast::<quinn::crypto::rustls::HandshakeData>().ok())
        .is_some_and(|data| data.protocol.as_deref() == Some(ALPN_LANE0_V1))
}

enum HelloFail {
    Code(CloseCode),
    Lost,
}

/// E§5 step 3: the client's first bidi stream and one hello frame (cap 130).
async fn read_hello(connection: &Connection) -> Result<(SendStream, RecvStream, Hello), HelloFail> {
    let (send, mut recv) = connection.accept_bi().await.map_err(|_| HelloFail::Lost)?;
    let payload = match read_frame(&mut recv, MAX_HELLO_BYTES).await {
        Ok(Some(payload)) => payload,
        // FIN before a complete hello.
        Ok(None) => return Err(HelloFail::Code(CloseCode::ProtocolViolation)),
        Err(ReadFail::Code(code)) => return Err(HelloFail::Code(code)),
        Err(ReadFail::Lost) => return Err(HelloFail::Lost),
    };
    match Hello::decode(&payload) {
        Ok(hello) => Ok((send, recv, hello)),
        Err(HandshakeError::UnsupportedWireVersion) => {
            Err(HelloFail::Code(CloseCode::VersionRefused))
        }
        Err(HandshakeError::Malformed | HandshakeError::CredentialLength) => {
            Err(HelloFail::Code(CloseCode::ProtocolViolation))
        }
    }
}

async fn serve_connection(shared: Arc<Shared>, incoming: Incoming, limits: ServerLimits) {
    // E§5 step 2: TLS 1.3 handshake.
    let connection = match incoming.accept() {
        Ok(connecting) => connecting.await.ok(),
        Err(_) => None,
    };
    let Some(connection) = connection else {
        end_handshake(&shared, None);
        return;
    };
    if !alpn_matches(&connection) {
        close_quinn(&connection, CloseCode::ProtocolViolation);
        end_handshake(&shared, None);
        return;
    }
    // E§5 step 3: assign the id, read the hello under its timer.
    let wake = Arc::new(ConnWake::default());
    let id = match shared.lock_any() {
        Some(mut state) => {
            let id = state.next_id;
            state.next_id += 1;
            state.conns.insert(
                id,
                Conn::new(Phase::Hello, connection.clone(), wake.clone()),
            );
            id
        }
        None => return,
    };
    let hello_timeout = Duration::from_millis(u64::from(limits.hello_timeout_ms));
    let (send, recv, hello) =
        match tokio::time::timeout(hello_timeout, read_hello(&connection)).await {
            Ok(Ok(parts)) => parts,
            Ok(Err(HelloFail::Code(code))) => {
                close_quinn(&connection, code);
                end_handshake(&shared, Some(id));
                return;
            }
            Ok(Err(HelloFail::Lost)) => {
                end_handshake(&shared, Some(id));
                return;
            }
            Err(_elapsed) => {
                close_quinn(&connection, CloseCode::HelloTimeout);
                end_handshake(&shared, Some(id));
                return;
            }
        };
    // E§5 step 4: HelloReceived, then the decision timer.
    if let Some(mut state) = shared.lock_any() {
        if let Some(record) = state.conns.get_mut(&id) {
            record.phase = Phase::Pending;
            record.hello_version = hello.wire_version;
        }
        state.events.push_back(ServerEvent::HelloReceived {
            conn: ConnectionId(id),
            remote: connection.remote_address(),
            hello,
        });
    }
    shared.notify_sync();
    let welcome = await_decision(&shared, id, &connection, &wake, limits).await;
    // E§5 step 5: on accept, the welcome first, then the lane tasks.
    match welcome {
        Some(welcome) => start_lane(&shared, id, &connection, &wake, send, recv, welcome).await,
        None => {
            // Closed while pending (or between `accept` and the welcome): no
            // reader will run, so release its claim and publish the close.
            if let Some(mut state) = shared.lock_any() {
                state.reader_finished(id);
            }
            shared.notify_sync();
        }
    }
    lane_control(shared, id, connection, wake).await;
}

/// Waits for `accept`/`refuse` up to `decision_timeout_ms`; on expiry closes
/// with `AuthTimeout`. `Some(welcome)` once accepted.
async fn await_decision(
    shared: &Shared,
    id: u64,
    connection: &Connection,
    wake: &ConnWake,
    limits: ServerLimits,
) -> Option<Welcome> {
    let deadline =
        tokio::time::Instant::now() + Duration::from_millis(u64::from(limits.decision_timeout_ms));
    loop {
        {
            let mut state = shared.lock_any()?;
            let record = state.conns.get_mut(&id)?;
            if record.reason.is_some() {
                return None;
            }
            if let Some(welcome) = record.welcome.take() {
                return Some(welcome);
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

/// Writes the welcome frame, then starts the lane writer and reader. If the
/// welcome cannot be written the connection is closed and the reader's
/// claim released.
async fn start_lane(
    shared: &Arc<Shared>,
    id: u64,
    connection: &Connection,
    wake: &Arc<ConnWake>,
    mut send: SendStream,
    recv: RecvStream,
    welcome: Welcome,
) {
    let written = match welcome.encode() {
        Ok(bytes) => match framed(&bytes, WELCOME_BYTES) {
            Ok(frame) => send.write_all(&frame).await,
            Err(_) => Err(quinn::WriteError::ClosedStream),
        },
        Err(_) => Err(quinn::WriteError::ClosedStream),
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
                MAX_INTENT_FRAME_BYTES,
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
