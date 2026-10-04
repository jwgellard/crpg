//! Application close codes, close modes and reasons, the send error, and
//! per-connection stats (E§3, E§8.1).

use std::fmt;

/// Exact `u32` application close codes, sent as QUIC varints with the
/// [`CloseCode::name`] reason phrase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CloseCode {
    /// Deliberate end of session (either side).
    Normal = 0,
    /// Server shutting down (`shutdown`, `Drop`).
    Shutdown = 1,
    /// Bad framing, zero-length frame, malformed hello/welcome, FIN/reset
    /// mid-frame, `RESET_STREAM`/`STOP_SENDING` on the lane (either side).
    ProtocolViolation = 2,
    /// Length prefix over the direction's cap (either side).
    FrameTooLarge = 3,
    /// No complete hello within `hello_timeout_ms` (server).
    HelloTimeout = 4,
    /// Credential not accepted (server, adapter decision).
    AuthRefused = 5,
    /// Wire version unknown or not the host's selection (server).
    VersionRefused = 6,
    /// No binding slot (server, adapter decision).
    ServerBusy = 7,
    /// No accept/refuse decision within `decision_timeout_ms` (server).
    AuthTimeout = 8,
    /// The host fenced the binding (server, adapter decision).
    SessionFenced = 9,
}

impl CloseCode {
    /// The numeric code.
    pub fn as_u32(self) -> u32 {
        self as u32
    }

    /// `None` for any value outside 0..=9 (peer codes are QUIC varints).
    pub fn from_u64(raw: u64) -> Option<CloseCode> {
        Some(match raw {
            0 => CloseCode::Normal,
            1 => CloseCode::Shutdown,
            2 => CloseCode::ProtocolViolation,
            3 => CloseCode::FrameTooLarge,
            4 => CloseCode::HelloTimeout,
            5 => CloseCode::AuthRefused,
            6 => CloseCode::VersionRefused,
            7 => CloseCode::ServerBusy,
            8 => CloseCode::AuthTimeout,
            9 => CloseCode::SessionFenced,
            _ => return None,
        })
    }

    /// The variant name; also sent as the QUIC reason phrase (ASCII, ≤ 17 bytes).
    pub fn name(self) -> &'static str {
        match self {
            CloseCode::Normal => "Normal",
            CloseCode::Shutdown => "Shutdown",
            CloseCode::ProtocolViolation => "ProtocolViolation",
            CloseCode::FrameTooLarge => "FrameTooLarge",
            CloseCode::HelloTimeout => "HelloTimeout",
            CloseCode::AuthRefused => "AuthRefused",
            CloseCode::VersionRefused => "VersionRefused",
            CloseCode::ServerBusy => "ServerBusy",
            CloseCode::AuthTimeout => "AuthTimeout",
            CloseCode::SessionFenced => "SessionFenced",
        }
    }
}

/// How a local close treats outbound frames still queued in this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseMode {
    /// Write every queued outbound frame, FIN, wait for the peer's ack up to
    /// `close_flush_timeout_ms`, then CONNECTION_CLOSE.
    Flush,
    /// Discard queued outbound frames not yet handed to QUIC; close at once.
    Discard,
}

/// Why a connection ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// This side closed with this code.
    Local(CloseCode),
    /// The peer sent an application close (raw varint; see `CloseCode::from_u64`).
    Peer {
        /// The peer's raw application error code.
        code: u64,
    },
    /// QUIC idle timeout.
    IdleTimeout,
    /// Any QUIC transport/crypto error, or a peer CONNECTION_CLOSE of transport type.
    Transport,
    /// Stateless reset or connection lost.
    Reset,
    /// The endpoint's I/O thread ended unexpectedly (panic or runtime failure).
    EndpointFailed,
}

/// Why `try_send` queued nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// No such connection (never existed, or its record was forgotten).
    UnknownConnection,
    /// The connection is still pending the host's decision.
    NotAccepted,
    /// The connection is closing or closed.
    Closed(CloseReason),
    /// A zero-length frame.
    EmptyFrame,
    /// A frame above the direction's send cap.
    FrameTooLarge,
    /// Retryable: nothing was queued; retry after the peer drains.
    QueueFull,
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            SendError::UnknownConnection => "UnknownConnection",
            SendError::NotAccepted => "NotAccepted",
            SendError::Closed(_) => "Closed",
            SendError::EmptyFrame => "EmptyFrame",
            SendError::FrameTooLarge => "FrameTooLarge",
            SendError::QueueFull => "QueueFull",
        };
        write!(f, "{name} at quic/send")
    }
}

impl std::error::Error for SendError {}

/// A per-connection snapshot of this crate's queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionStats {
    /// Complete inbound frames queued for `try_recv`.
    pub inbound_frames: usize,
    /// Payload bytes of those frames.
    pub inbound_bytes: usize,
    /// Outbound frames queued and not yet handed to QUIC.
    pub outbound_frames: usize,
    /// Payload bytes of those frames.
    pub outbound_bytes: usize,
    /// One complete inbound frame is held, waiting for queue room.
    pub reader_parked: bool,
    /// QUIC PING frames this side sent (quinn `frame_tx.ping`).
    pub keepalive_pings_sent: u64,
}
