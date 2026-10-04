//! The client endpoint: one connection, one UDP socket, one I/O thread, and
//! a synchronous, non-blocking API over bounded queues (E§3, E§5, E§6).

use std::fmt;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crpg_net::protocol::{MAX_DELTA_FRAME_BYTES, MAX_INTENT_FRAME_BYTES};
use crpg_net::sim::QueueCaps;
use crpg_net::transport::Transport;
use quinn::{
    Connection, ConnectionError, EndpointConfig, ReadError, ReadExactError, RecvStream, SendStream,
    TokioRuntime, TransportErrorCode, WriteError,
};

use crate::close::{CloseCode, CloseMode, CloseReason, ConnectionStats, SendError};
use crate::handshake::{
    Hello, Welcome, FRAME_HEADER_BYTES, MAX_HELLO_BYTES, TLS_SERVER_NAME, WELCOME_BYTES,
};
use crate::limits::{ClientConfig, ClientLimits, ConfigError};
use crate::queue::{
    close_quinn, deadline_after, framed, lane_control, lane_reader, lane_writer, race, reason_from,
    wait_stop, Conn, ConnWake, Either, FailGuard, FlushRequest, Phase, Shared, State, Stop,
    SYNC_GRACE,
};
use crate::tls;

/// The client's single connection record id.
const LANE: u64 = 1;

/// Why `QuicClient::connect` failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectError {
    /// The limits failed validation.
    Config(ConfigError),
    /// The UDP socket could not be bound, or the server address is unusable.
    Io(std::io::ErrorKind),
    /// The I/O thread or its runtime could not start.
    Runtime,
    /// The certificate's SHA-256 differs from the pin, or the server sent a chain.
    PinMismatch,
    /// Any other TLS/QUIC handshake failure (ALPN, version, crypto).
    Handshake,
    /// QUIC CONNECTION_REFUSED (server at `max_connections`).
    EndpointFull,
    /// The server closed with an application code before the welcome.
    Refused {
        /// The server's raw application close code.
        code: u64,
    },
    /// The connection ended otherwise before the welcome.
    Lost(CloseReason),
    /// Welcome malformed or with a different wire version (client then closes
    /// with `ProtocolViolation`).
    InvalidWelcome,
    /// `connect_timeout_ms` elapsed.
    TimedOut,
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            ConnectError::Config(_) => "Config",
            ConnectError::Io(_) => "Io",
            ConnectError::Runtime => "Runtime",
            ConnectError::PinMismatch => "PinMismatch",
            ConnectError::Handshake => "Handshake",
            ConnectError::EndpointFull => "EndpointFull",
            ConnectError::Refused { .. } => "Refused",
            ConnectError::Lost(_) => "Lost",
            ConnectError::InvalidWelcome => "InvalidWelcome",
            ConnectError::TimedOut => "TimedOut",
        };
        write!(f, "{name} at quic/connect")
    }
}

impl std::error::Error for ConnectError {}

/// What a local close discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseReport {
    /// Outbound frames never handed to QUIC.
    pub outbound_frames_discarded: usize,
    /// Inbound frames never drained (queued or parked).
    pub inbound_frames_discarded: usize,
}

/// One connection, one UDP socket, one I/O thread. `Debug` is hand-written.
pub struct QuicClient {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    local_addr: SocketAddr,
    welcome: Welcome,
    limits: ClientLimits,
}

impl QuicClient {
    /// Blocks until the welcome arrives or a `ConnectError`, bounded by `connect_timeout_ms`.
    pub fn connect(
        config: ClientConfig,
        server: SocketAddr,
        hello: &Hello,
    ) -> Result<QuicClient, ConnectError> {
        let limits = config.limits;
        limits.validate().map_err(ConnectError::Config)?;
        // A hello that cannot be encoded (wire version not 1 or 2) is a
        // handshake that cannot start.
        let hello_frame = hello
            .encode()
            .ok()
            .and_then(|payload| framed(&payload, MAX_HELLO_BYTES).ok())
            .ok_or(ConnectError::Handshake)?;
        let wire_version = hello.wire_version;
        let mismatch = Arc::new(AtomicBool::new(false));
        let quinn_config = tls::client_config(
            config.pin,
            mismatch.clone(),
            limits.idle_timeout_ms,
            limits.keep_alive_ms,
        )
        .ok_or(ConnectError::Handshake)?;
        let socket = UdpSocket::bind(config.bind).map_err(|e| ConnectError::Io(e.kind()))?;
        let local_addr = socket
            .local_addr()
            .map_err(|e| ConnectError::Io(e.kind()))?;
        let inbound = QueueCaps {
            per_peer_frames: limits.inbound_frames,
            per_peer_bytes: limits.inbound_bytes,
            host_frames: limits.inbound_frames,
            host_bytes: limits.inbound_bytes,
        };
        let outbound = QueueCaps {
            per_peer_frames: limits.outbound_frames,
            per_peer_bytes: limits.outbound_bytes,
            host_frames: limits.outbound_frames,
            host_bytes: limits.outbound_bytes,
        };
        let shared = Arc::new(Shared::new(State::new(inbound, outbound, false)));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread_shared = shared.clone();
        let attempt = Attempt {
            config: quinn_config,
            server,
            hello_frame,
            wire_version,
            mismatch,
            limits,
        };
        let thread = std::thread::Builder::new()
            .name("crpg-quic-client".into())
            .spawn(move || {
                let guard = FailGuard::new(thread_shared.clone());
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .enable_time()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(_) => {
                        let _ = ready_tx.send(Err(ConnectError::Runtime));
                        guard.clean();
                        return;
                    }
                };
                runtime.block_on(client_main(thread_shared, socket, attempt, ready_tx));
                guard.clean();
            })
            .map_err(|_| ConnectError::Runtime)?;
        match ready_rx.recv() {
            Ok(Ok(welcome)) => Ok(QuicClient {
                shared,
                thread: Some(thread),
                local_addr,
                welcome,
                limits,
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(ConnectError::Runtime)
            }
        }
    }

    /// The welcome the server sent.
    pub fn welcome(&self) -> Welcome {
        self.welcome
    }

    /// The bound UDP address.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Queues one lane frame (≤ 4,096 bytes). Precedence: `Closed` →
    /// `EmptyFrame` → `FrameTooLarge` → `QueueFull` (retryable; nothing queued).
    pub fn try_send(&mut self, frame: &[u8]) -> Result<(), SendError> {
        let endpoint_failed = SendError::Closed(CloseReason::EndpointFailed);
        let mut state = self.shared.lock().ok_or(endpoint_failed)?;
        let record = state.conns.get(&LANE).ok_or(endpoint_failed)?;
        if let Some(reason) = record.close_state() {
            return Err(SendError::Closed(reason));
        }
        if frame.is_empty() {
            return Err(SendError::EmptyFrame);
        }
        if frame.len() > MAX_INTENT_FRAME_BYTES {
            return Err(SendError::FrameTooLarge);
        }
        state.queue_outbound(LANE, frame)
    }

    /// `Ok(None)` when empty; `Err(reason)` once closed **and** drained.
    pub fn try_recv(&mut self) -> Result<Option<Vec<u8>>, CloseReason> {
        let mut state = self.shared.lock().ok_or(CloseReason::EndpointFailed)?;
        if let Some(frame) = state.pop_inbound(LANE) {
            return Ok(Some(frame));
        }
        let record = state.conns.get(&LANE).ok_or(CloseReason::EndpointFailed)?;
        if record.published {
            return Err(record.reason.unwrap_or(CloseReason::EndpointFailed));
        }
        Ok(None)
    }

    /// Blocks up to `timeout` until an inbound frame is queued or the
    /// connection has ended; `false` on timeout.
    pub fn wait(&self, timeout: Duration) -> bool {
        let now = std::time::Instant::now();
        let deadline = now.checked_add(timeout).unwrap_or(now);
        let Some(state) = self.shared.lock_any() else {
            return false;
        };
        let ready = |state: &State| {
            state.inbound_frames > 0 || state.conns.get(&LANE).is_none_or(|record| record.published)
        };
        match self.shared.wait_until(state, deadline, ready) {
            Some(state) => ready(&state),
            None => false,
        }
    }

    /// A snapshot of the connection's queues.
    pub fn stats(&self) -> ConnectionStats {
        let snapshot = self.shared.lock_any().and_then(|state| {
            state
                .conns
                .get(&LANE)
                .map(|record| (record.stats(0), record.connection.clone()))
        });
        match snapshot {
            Some((stats, connection)) => ConnectionStats {
                keepalive_pings_sent: connection.stats().frame_tx.ping,
                ..stats
            },
            None => ConnectionStats {
                inbound_frames: 0,
                inbound_bytes: 0,
                outbound_frames: 0,
                outbound_bytes: 0,
                reader_parked: false,
                keepalive_pings_sent: 0,
            },
        }
    }

    /// Why the connection ended, once it has.
    pub fn close_reason(&self) -> Option<CloseReason> {
        if self.shared.is_failed() {
            return Some(CloseReason::EndpointFailed);
        }
        let state = self.shared.lock_any()?;
        state.conns.get(&LANE).and_then(|record| record.reason)
    }

    /// Closes the connection with `code` and ends the I/O thread. `Flush`
    /// blocks up to `close_flush_timeout_ms` for the peer's ack.
    pub fn close(mut self, code: CloseCode, mode: CloseMode) -> CloseReport {
        let report = self.stop(code, mode);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        report
    }

    fn stop(&mut self, code: CloseCode, mode: CloseMode) -> CloseReport {
        let mut report = CloseReport {
            outbound_frames_discarded: 0,
            inbound_frames_discarded: 0,
        };
        let deadline = deadline_after(self.limits.close_flush_timeout_ms);
        if let Some(mut state) = self.shared.lock_any() {
            let open = state
                .conns
                .get(&LANE)
                .is_some_and(|record| record.close_state().is_none());
            let before = state.conns.get(&LANE).map_or(0, |record| record.discarded);
            if open && mode == CloseMode::Flush {
                if let Some(record) = state.conns.get_mut(&LANE) {
                    record.closing = Some(code);
                    record.flush = Some(FlushRequest {
                        code,
                        deadline,
                        started: false,
                    });
                    record.wake.control.notify_one();
                }
                let guard_deadline = deadline.checked_add(SYNC_GRACE).unwrap_or(deadline);
                state = match self.shared.wait_until(state, guard_deadline, |state| {
                    state
                        .conns
                        .get(&LANE)
                        .is_none_or(|record| record.close_done || record.control_done)
                }) {
                    Some(state) => state,
                    None => return report,
                };
            } else if open {
                state.record_close(LANE, CloseReason::Local(code));
            }
            if let Some(record) = state.conns.get(&LANE) {
                report.outbound_frames_discarded = record.discarded.saturating_sub(before);
                report.inbound_frames_discarded = record.held_inbound_frames();
            }
            state.stop = Some(Stop { code, deadline });
        }
        self.shared.endpoint_wake.notify_one();
        report
    }
}

impl Transport for QuicClient {
    type Error = SendError;

    /// `QuicClient::try_send`.
    fn send(&mut self, bytes: &[u8]) -> Result<(), SendError> {
        self.try_send(bytes)
    }

    /// `QuicClient::try_recv`, errors folded into `None`.
    fn recv(&mut self) -> Option<Vec<u8>> {
        self.try_recv().ok().flatten()
    }
}

/// Closes with `Normal` in `Discard` mode, waits up to
/// `close_flush_timeout_ms` for the CONNECTION_CLOSE to go out, joins the thread.
impl Drop for QuicClient {
    fn drop(&mut self) {
        if self.thread.is_none() {
            return;
        }
        self.stop(CloseCode::Normal, CloseMode::Discard);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl fmt::Debug for QuicClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuicClient")
            .field("local_addr", &self.local_addr)
            .field("close_reason", &self.close_reason())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// I/O thread.
// ---------------------------------------------------------------------------

struct Attempt {
    config: quinn::ClientConfig,
    server: SocketAddr,
    hello_frame: Vec<u8>,
    wire_version: u8,
    mismatch: Arc<AtomicBool>,
    limits: ClientLimits,
}

async fn client_main(
    shared: Arc<Shared>,
    socket: UdpSocket,
    attempt: Attempt,
    ready: mpsc::SyncSender<Result<Welcome, ConnectError>>,
) {
    let endpoint = match quinn::Endpoint::new(
        EndpointConfig::default(),
        None,
        socket,
        Arc::new(TokioRuntime),
    ) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            let _ = ready.send(Err(ConnectError::Io(error.kind())));
            return;
        }
    };
    let limits = attempt.limits;
    let connect_timeout = Duration::from_millis(u64::from(limits.connect_timeout_ms));
    let outcome = tokio::time::timeout(connect_timeout, handshake(&endpoint, attempt)).await;
    let (connection, send, recv, welcome) = match outcome {
        Ok(Ok(parts)) => parts,
        Ok(Err(error)) => {
            finish_endpoint(&endpoint, CloseCode::Normal, limits).await;
            let _ = ready.send(Err(error));
            return;
        }
        Err(_elapsed) => {
            finish_endpoint(&endpoint, CloseCode::Normal, limits).await;
            let _ = ready.send(Err(ConnectError::TimedOut));
            return;
        }
    };
    let wake = Arc::new(ConnWake::default());
    if let Some(mut state) = shared.lock_any() {
        let mut record = Conn::new(Phase::Accepted, connection.clone(), wake.clone());
        record.reader_active = true;
        state.conns.insert(LANE, record);
    }
    tokio::spawn(lane_writer(
        shared.clone(),
        LANE,
        connection.clone(),
        send,
        wake.clone(),
    ));
    tokio::spawn(lane_reader(
        shared.clone(),
        LANE,
        connection.clone(),
        recv,
        MAX_DELTA_FRAME_BYTES,
        wake.clone(),
    ));
    let _ = ready.send(Ok(welcome));
    let stop = match race(
        lane_control(shared.clone(), LANE, connection, wake),
        wait_stop(&shared),
    )
    .await
    {
        Either::Left(()) => wait_stop(&shared).await,
        Either::Right(stop) => stop,
    };
    endpoint.close(
        quinn::VarInt::from_u32(stop.code.as_u32()),
        stop.code.name().as_bytes(),
    );
    let deadline = tokio::time::Instant::from_std(stop.deadline);
    let _ = tokio::time::timeout_at(deadline, endpoint.wait_idle()).await;
}

/// Closes whatever the endpoint still holds and lets the close go out.
async fn finish_endpoint(endpoint: &quinn::Endpoint, code: CloseCode, limits: ClientLimits) {
    endpoint.close(
        quinn::VarInt::from_u32(code.as_u32()),
        code.name().as_bytes(),
    );
    let flush = Duration::from_millis(u64::from(limits.close_flush_timeout_ms));
    let _ = tokio::time::timeout(flush, endpoint.wait_idle()).await;
}

/// E§5 client sequence: TLS with the pinned verifier, open the bidi stream,
/// write the hello, read exactly one 17-byte welcome and check its version.
async fn handshake(
    endpoint: &quinn::Endpoint,
    attempt: Attempt,
) -> Result<(Connection, SendStream, RecvStream, Welcome), ConnectError> {
    let connecting = endpoint
        .connect_with(attempt.config, attempt.server, TLS_SERVER_NAME)
        .map_err(|error| match error {
            quinn::ConnectError::InvalidRemoteAddress(_) => {
                ConnectError::Io(ErrorKind::InvalidInput)
            }
            _ => ConnectError::Handshake,
        })?;
    let connection = connecting
        .await
        .map_err(|error| handshake_error(&error, &attempt.mismatch))?;
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|error| lost_error(&error))?;
    send.write_all(&attempt.hello_frame)
        .await
        .map_err(|error| match error {
            WriteError::ConnectionLost(lost) => lost_error(&lost),
            _ => invalid_welcome(&connection),
        })?;
    let mut header = [0u8; FRAME_HEADER_BYTES];
    read_welcome_part(&connection, &mut recv, &mut header).await?;
    if usize::try_from(u32::from_be_bytes(header)).ok() != Some(WELCOME_BYTES) {
        return Err(invalid_welcome(&connection));
    }
    let mut body = [0u8; WELCOME_BYTES];
    read_welcome_part(&connection, &mut recv, &mut body).await?;
    let welcome = Welcome::decode(&body).map_err(|_| invalid_welcome(&connection))?;
    if welcome.wire_version != attempt.wire_version {
        return Err(invalid_welcome(&connection));
    }
    Ok((connection, send, recv, welcome))
}

async fn read_welcome_part(
    connection: &Connection,
    recv: &mut RecvStream,
    buf: &mut [u8],
) -> Result<(), ConnectError> {
    match recv.read_exact(buf).await {
        Ok(()) => Ok(()),
        Err(ReadExactError::ReadError(ReadError::ConnectionLost(lost))) => Err(lost_error(&lost)),
        Err(_) => Err(invalid_welcome(connection)),
    }
}

fn invalid_welcome(connection: &Connection) -> ConnectError {
    close_quinn(connection, CloseCode::ProtocolViolation);
    ConnectError::InvalidWelcome
}

/// The connection ended after TLS but before the welcome.
fn lost_error(error: &ConnectionError) -> ConnectError {
    match error {
        ConnectionError::ApplicationClosed(close) => ConnectError::Refused {
            code: close.error_code.into_inner(),
        },
        other => ConnectError::Lost(reason_from(other)),
    }
}

/// The TLS/QUIC handshake failed.
fn handshake_error(error: &ConnectionError, mismatch: &AtomicBool) -> ConnectError {
    if mismatch.load(Ordering::SeqCst) {
        return ConnectError::PinMismatch;
    }
    match error {
        ConnectionError::ConnectionClosed(close)
            if close.error_code == TransportErrorCode::CONNECTION_REFUSED =>
        {
            ConnectError::EndpointFull
        }
        ConnectionError::ApplicationClosed(close) => ConnectError::Refused {
            code: close.error_code.into_inner(),
        },
        ConnectionError::TimedOut => ConnectError::Lost(CloseReason::IdleTimeout),
        ConnectionError::Reset => ConnectError::Lost(CloseReason::Reset),
        ConnectionError::LocallyClosed => ConnectError::Lost(CloseReason::Local(CloseCode::Normal)),
        ConnectionError::TransportError(_)
        | ConnectionError::ConnectionClosed(_)
        | ConnectionError::VersionMismatch
        | ConnectionError::CidsExhausted => ConnectError::Handshake,
    }
}
