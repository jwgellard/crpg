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
use crpg_net::protocol_resume::MAX_RESUME_FRAME_BYTES;
use crpg_net::sim::QueueCaps;
use crpg_net::transport::Transport;
use quinn::{
    Connection, ConnectionError, EndpointConfig, ReadError, ReadExactError, RecvStream, SendStream,
    TokioRuntime, TransportErrorCode, WriteError,
};

use crate::close::{
    ChannelSendError, CloseCode, CloseMode, CloseReason, ConnectionStats, SendError,
};
use crate::handshake::{
    parse_channel_frame_header, Channel, Framing, Hello, Welcome, ALPN_LANE0_V2,
    FRAME_HEADER_BYTES, MAX_HELLO_BYTES, TLS_SERVER_NAME, WELCOME_BYTES,
};
use crate::limits::{ClientConfig, ClientLimits, ConfigError};
use crate::queue::{
    close_quinn, framed, framed_channel, lane_control, lane_reader, lane_writer, race, reason_from,
    wait_stop, Conn, ConnWake, Either, FailGuard, FlushClose, LaneDirection, Phase, ReadSpec,
    Shared, State, Stop, SYNC_GRACE,
};
use crate::teardown::finish_endpoint_close;
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
    /// The connection ended otherwise before the welcome or answer.
    Lost(CloseReason),
    /// The welcome (or, on a resume connection, the answer frame) is
    /// malformed or with a different wire version; on a resume connection
    /// the answer's header is malformed (byte 0 not `0x91`, or a length of
    /// 0 or above 65,536) and the client closes with `ProtocolViolation`
    /// (S§4.6 of `tasks/T024q.md`).
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
    /// The framing this client asked for and negotiated.
    framing: Framing,
}

impl QuicClient {
    /// Blocks until the welcome arrives or a `ConnectError`, bounded by `connect_timeout_ms`.
    pub fn connect(
        config: ClientConfig,
        server: SocketAddr,
        hello: &Hello,
    ) -> Result<QuicClient, ConnectError> {
        Self::connect_framed(config, server, hello, Framing::V1)
    }

    /// `connect` with an explicit framing; offers exactly `framing.alpn()`.
    /// Blocks as `connect`, bounded by `connect_timeout_ms`.
    pub fn connect_framed(
        config: ClientConfig,
        server: SocketAddr,
        hello: &Hello,
        framing: Framing,
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
        let mismatch = Arc::new(AtomicBool::new(false));
        let quinn_config =
            tls::client_config(config.pin, mismatch.clone(), framing.alpn(), &limits)
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
            kind: Kind::Hello {
                hello_frame,
                wire_version: hello.wire_version,
            },
            framing,
            mismatch,
            limits,
        };
        let thread = spawn_attempt(thread_shared, socket, attempt, ready_tx)?;
        match ready_rx.recv() {
            Ok(Ok(Connected {
                welcome,
                resume_answer,
            })) => {
                debug_assert!(resume_answer.is_none(), "a hello opening reads no answer");
                Ok(QuicClient {
                    shared,
                    thread: Some(thread),
                    local_addr,
                    welcome,
                    limits,
                    framing,
                })
            }
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

    /// A `crpg-lane0/2` connection opened with `request` (S§4.6 of
    /// `tasks/T024q.md`): TLS over `/2` only, then `[0x91][len24]` +
    /// `request` as the opening, then the server's answer read and checked
    /// for form only. The one timer covers TLS, the request and the whole
    /// answer read. Returns the client and the answer payload;
    /// [`welcome`](Self::welcome) then returns `session`. Before any I/O,
    /// limits invalid → `Config`, an unencodable `session` or an empty or
    /// over-4,096-byte `request` → `Handshake`, and no socket is bound.
    pub fn connect_resume(
        config: ClientConfig,
        server: SocketAddr,
        session: Welcome,
        request: &[u8],
    ) -> Result<(QuicClient, Vec<u8>), ConnectError> {
        let limits = config.limits;
        limits.validate().map_err(ConnectError::Config)?;
        // Resume keeps the session's epoch and wire version, so the caller
        // supplies the session's welcome in place of a received one (D39d
        // Q4); an unencodable session is a handshake that cannot start.
        session.encode().map_err(|_| ConnectError::Handshake)?;
        if request.is_empty() || request.len() > MAX_RESUME_FRAME_BYTES {
            return Err(ConnectError::Handshake);
        }
        let request_frame = framed_channel(Channel::Resume, request, MAX_RESUME_FRAME_BYTES)
            .map_err(|_| ConnectError::Handshake)?;
        let mismatch = Arc::new(AtomicBool::new(false));
        let quinn_config =
            tls::client_config(config.pin, mismatch.clone(), Framing::V2.alpn(), &limits)
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
            kind: Kind::Resume {
                request_frame,
                welcome: session,
            },
            framing: Framing::V2,
            mismatch,
            limits,
        };
        let thread = spawn_attempt(thread_shared, socket, attempt, ready_tx)?;
        match ready_rx.recv() {
            Ok(Ok(Connected {
                welcome,
                resume_answer,
            })) => match resume_answer {
                Some(answer) => Ok((
                    QuicClient {
                        shared,
                        thread: Some(thread),
                        local_addr,
                        welcome,
                        limits,
                        framing: Framing::V2,
                    },
                    answer,
                )),
                // A resume attempt always reads an answer (S§4.6).
                None => {
                    let _ = thread.join();
                    Err(ConnectError::Runtime)
                }
            },
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

    /// The framing this client asked for and negotiated.
    pub fn framing(&self) -> Framing {
        self.framing
    }

    /// The welcome the server sent; on a resumed connection the session
    /// welcome the caller passed (the transport does not decode the answer;
    /// T025a C4).
    pub fn welcome(&self) -> Welcome {
        self.welcome
    }

    /// The bound UDP address.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Queues one lane-0 frame (≤ 4,096 bytes): `try_send_channel` on
    /// `Channel::Lane0`, whose `ChannelUnavailable` cannot occur.
    pub fn try_send(&mut self, frame: &[u8]) -> Result<(), SendError> {
        match self.try_send_channel(Channel::Lane0, frame) {
            Ok(()) => Ok(()),
            Err(ChannelSendError::Send(error)) => Err(error),
            // Lane 0 is available on every framing; treat the impossible arm
            // as an endpoint-level failure, not a panic.
            Err(ChannelSendError::ChannelUnavailable) => {
                Err(SendError::Closed(CloseReason::EndpointFailed))
            }
        }
    }

    /// Queues one lane frame (≤ 4,096 bytes) on `channel`. Precedence:
    /// `Send(Closed(_))` → `ChannelUnavailable` (`Resume` always;
    /// `Transfer` on a `/1` connection) → `Send(EmptyFrame)` →
    /// `Send(FrameTooLarge)` → `Send(QueueFull)` (retryable; nothing
    /// queued).
    pub fn try_send_channel(
        &mut self,
        channel: Channel,
        frame: &[u8],
    ) -> Result<(), ChannelSendError> {
        let endpoint_failed =
            ChannelSendError::Send(SendError::Closed(CloseReason::EndpointFailed));
        let mut state = self.shared.lock().ok_or(endpoint_failed)?;
        let record = state.conns.get(&LANE).ok_or(endpoint_failed)?;
        if let Some(reason) = record.close_state() {
            return Err(ChannelSendError::Send(SendError::Closed(reason)));
        }
        // The resume channel is directional: upstream, a `0x91` frame is
        // legal only as the opening (S§5.2); and a `/1` connection has no
        // transfer channel.
        if channel == Channel::Resume
            || (channel == Channel::Transfer && record.framing == Framing::V1)
        {
            return Err(ChannelSendError::ChannelUnavailable);
        }
        if frame.is_empty() {
            return Err(ChannelSendError::Send(SendError::EmptyFrame));
        }
        if frame.len() > MAX_INTENT_FRAME_BYTES {
            return Err(ChannelSendError::Send(SendError::FrameTooLarge));
        }
        state
            .queue_outbound(LANE, channel, frame)
            .map_err(ChannelSendError::Send)
    }

    /// `Ok(None)` when empty; `Err(reason)` once closed **and** drained. A
    /// view: while a transfer or resume frame is at the head it returns
    /// `Ok(None)` and leaves it queued; only `try_recv_channel` can take it
    /// (S§8.3).
    pub fn try_recv(&mut self) -> Result<Option<Vec<u8>>, CloseReason> {
        let mut state = self.shared.lock().ok_or(CloseReason::EndpointFailed)?;
        if let Some(frame) = state.pop_inbound_lane0(LANE) {
            return Ok(Some(frame));
        }
        let record = state.conns.get(&LANE).ok_or(CloseReason::EndpointFailed)?;
        if record
            .inbound_head()
            .is_some_and(|channel| channel != Channel::Lane0)
        {
            return Ok(None);
        }
        if record.published {
            return Err(record.reason.unwrap_or(CloseReason::EndpointFailed));
        }
        Ok(None)
    }

    /// As `try_recv`, with the frame's channel: on a `crpg-lane0/2`
    /// connection all three channels arrive; on a `/1` connection every
    /// frame is `Channel::Lane0`.
    pub fn try_recv_channel(&mut self) -> Result<Option<(Channel, Vec<u8>)>, CloseReason> {
        let mut state = self.shared.lock().ok_or(CloseReason::EndpointFailed)?;
        if let Some(next) = state.pop_inbound(LANE) {
            return Ok(Some(next));
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
    /// blocks up to `close_flush_timeout_ms` for the peer's ack. The last
    /// `min(200 ms, budget / 4)` of the budget is kept for the CONNECTION_CLOSE.
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
        let close = FlushClose::flush_close(self.limits.close_flush_timeout_ms, code);
        if let Some(mut state) = self.shared.lock_any() {
            let open = state
                .conns
                .get(&LANE)
                .is_some_and(|record| record.close_state().is_none());
            let before = state.conns.get(&LANE).map_or(0, |record| record.discarded);
            if open && mode == CloseMode::Flush {
                if let Some(record) = state.conns.get_mut(&LANE) {
                    record.closing = Some(code);
                    record.flush = Some(close.request);
                    record.wake.control.notify_one();
                }
                let guard_deadline = close
                    .overall
                    .checked_add(SYNC_GRACE)
                    .unwrap_or(close.overall);
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
            state.stop = Some(Stop {
                code,
                deadline: close.overall,
            });
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

/// What a successful handshake hands the connecting call.
#[derive(Debug)]
struct Connected {
    welcome: Welcome,
    /// The resume answer payload, on a resume opening only (S§4.6).
    resume_answer: Option<Vec<u8>>,
}

/// Which opening the attempt presents on the lane stream.
enum Kind {
    /// A hello frame; answered with exactly one 17-byte welcome.
    Hello {
        hello_frame: Vec<u8>,
        wire_version: u8,
    },
    /// A resume request frame (an `0x91` channel frame); answered with the
    /// server's answer payload, checked for form only.
    Resume {
        request_frame: Vec<u8>,
        /// The session welcome the caller supplied as the client's welcome.
        welcome: Welcome,
    },
}

struct Attempt {
    config: quinn::ClientConfig,
    server: SocketAddr,
    kind: Kind,
    framing: Framing,
    mismatch: Arc<AtomicBool>,
    limits: ClientLimits,
}

/// The thread skeleton shared by `connect_framed` and `connect_resume`.
fn spawn_attempt(
    thread_shared: Arc<Shared>,
    socket: UdpSocket,
    attempt: Attempt,
    ready_tx: mpsc::SyncSender<Result<Connected, ConnectError>>,
) -> Result<JoinHandle<()>, ConnectError> {
    std::thread::Builder::new()
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
        .map_err(|_| ConnectError::Runtime)
}

async fn client_main(
    shared: Arc<Shared>,
    socket: UdpSocket,
    attempt: Attempt,
    ready: mpsc::SyncSender<Result<Connected, ConnectError>>,
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
    let (connection, send, recv, ready_outcome) = match outcome {
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
    let (welcome, resume_answer, framing) = ready_outcome;
    let wake = Arc::new(ConnWake::default());
    if let Some(mut state) = shared.lock_any() {
        let mut record = Conn::new(Phase::Accepted, connection.clone(), wake.clone());
        record.framing = framing;
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
        ReadSpec {
            framing,
            direction: LaneDirection::Downstream,
            cap: MAX_DELTA_FRAME_BYTES,
        },
        wake.clone(),
    ));
    let _ = ready.send(Ok(Connected {
        welcome,
        resume_answer,
    }));
    let stop = match race(
        lane_control(shared.clone(), LANE, connection, wake),
        wait_stop(&shared),
    )
    .await
    {
        Either::Left(()) => wait_stop(&shared).await,
        Either::Right(stop) => stop,
    };
    finish_endpoint_close(&endpoint, stop.code, stop.deadline).await;
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

/// E§5 client sequence with the S§4.3/S§4.6 additions: TLS with the pinned
/// verifier (plus the `/2` negotiated-ALPN re-check), open the bidi stream,
/// write the opening (hello, or `[0x91][len24]` + request), and read the
/// welcome, or the resume answer checked for form only. On success the
/// record's framing is returned for the lane reader.
async fn handshake(
    endpoint: &quinn::Endpoint,
    attempt: Attempt,
) -> Result<
    (
        Connection,
        SendStream,
        RecvStream,
        (Welcome, Option<Vec<u8>>, Framing),
    ),
    ConnectError,
> {
    let Attempt {
        config,
        server,
        kind,
        framing,
        mismatch,
        ..
    } = attempt;
    let connecting = endpoint
        .connect_with(config, server, TLS_SERVER_NAME)
        .map_err(|error| match error {
            quinn::ConnectError::InvalidRemoteAddress(_) => {
                ConnectError::Io(ErrorKind::InvalidInput)
            }
            _ => ConnectError::Handshake,
        })?;
    let connection = connecting
        .await
        .map_err(|error| handshake_error(&error, &mismatch))?;
    // S§4.3: under `/2`, a negotiated protocol other than `crpg-lane0/2` is
    // a protocol violation (defence in depth; the pinned rustls client
    // refuses it first).
    if framing == Framing::V2 {
        let negotiated = connection
            .handshake_data()
            .and_then(|data| data.downcast::<quinn::crypto::rustls::HandshakeData>().ok())
            .is_some_and(|data| data.protocol.as_deref() == Some(ALPN_LANE0_V2));
        if !negotiated {
            close_quinn(&connection, CloseCode::ProtocolViolation);
            return Err(ConnectError::Handshake);
        }
    }
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|error| lost_error(&error))?;
    match kind {
        Kind::Hello {
            hello_frame,
            wire_version,
        } => {
            send.write_all(&hello_frame)
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
            if welcome.wire_version != wire_version {
                return Err(invalid_welcome(&connection));
            }
            Ok((connection, send, recv, (welcome, None, framing)))
        }
        Kind::Resume {
            request_frame,
            welcome,
        } => {
            send.write_all(&request_frame)
                .await
                .map_err(|error| match error {
                    WriteError::ConnectionLost(lost) => lost_error(&lost),
                    _ => invalid_welcome(&connection),
                })?;
            // S§4.6 step 4: the answer header is a `/2` `[0x91][24-bit
            // length]` frame with 1..=65,536 bytes; anything else is an
            // `InvalidWelcome` after closing with code 2.
            let mut header = [0u8; FRAME_HEADER_BYTES];
            read_welcome_part(&connection, &mut recv, &mut header).await?;
            if header[0] != Channel::Resume.as_u8() {
                return Err(invalid_welcome(&connection));
            }
            let len = parse_answer_len(header).ok_or_else(|| invalid_welcome(&connection))?;
            let mut body = vec![0u8; len];
            read_welcome_part(&connection, &mut recv, &mut body).await?;
            Ok((connection, send, recv, (welcome, Some(body), framing)))
        }
    }
}

/// The `/2` answer header's payload length, `Ok` only for 1..=65,536 bytes
/// (S§4.6 step 4; the 24-bit length is checked against the direction cap).
fn parse_answer_len(header: [u8; FRAME_HEADER_BYTES]) -> Option<usize> {
    match parse_channel_frame_header(header, MAX_DELTA_FRAME_BYTES) {
        Ok((_, len)) => Some(len),
        Err(_) => None,
    }
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
