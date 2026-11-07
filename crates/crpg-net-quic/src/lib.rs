#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Real lane-0 QUIC transport for `crpg-net`'s byte protocol (T023,
//! ADR-0024 placement, ADR-0025 wire and trust).
//!
//! One client-initiated bidirectional QUIC stream per connection carries a
//! hello, a welcome, and then every lane-0 frame in both directions, each
//! prefixed with a 4-byte big-endian length checked against the
//! direction's cap before any allocation. There are no other streams and no
//! DATAGRAM frames. The server is trusted by a SHA-256 pin of its
//! certificate (D03); the client presents an opaque invitation credential
//! that this crate checks for format only.
//!
//! Each [`QuicServer`] and [`QuicClient`] owns one UDP socket, one OS thread
//! and one current-thread tokio runtime. The caller's side is synchronous
//! and non-blocking (except `wait`, `connect`, `shutdown`, `close` and
//! `Drop`), reached only through bounded queues, so a host pump never names
//! an async, quinn or rustls type. Payload bytes are never decoded here:
//! intents and deltas stay `crpg-net`'s, and this crate re-exports nothing
//! from it.
//!
//! Under the `crpg-lane0/2` ALPN the first byte of each lane-frame header is
//! a channel selector (`0x00` lane 0, `0x81` transfer, `0x91` resume), and a
//! `bind_resumable` server also accepts a connection opened with a resume
//! request in place of the hello (`tasks/T024q.md` S§4–S§6).
//!
//! `crpg-net` keeps its no-I/O rule; sockets, threads, clocks, tokio, quinn
//! and rustls live only in this crate (ADR-0024).

mod client;
mod close;
mod handshake;
mod identity;
mod limits;
mod queue;
mod server;
mod teardown;
mod tls;

pub use client::{CloseReport, ConnectError, QuicClient};
pub use close::{ChannelSendError, CloseCode, CloseMode, CloseReason, ConnectionStats, SendError};
pub use handshake::{
    channel_frame_header, frame_header, parse_channel_frame_header, parse_frame_header, Channel,
    Credential, CredentialError, FrameError, Framing, HandshakeError, Hello, Welcome,
    ALPN_LANE0_V1, ALPN_LANE0_V2, FRAME_HEADER_BYTES, MAX_CREDENTIAL_BYTES, MAX_HELLO_BYTES,
    MIN_CREDENTIAL_BYTES, TLS_SERVER_NAME, WELCOME_BYTES,
};
pub use identity::{CertificatePin, ServerIdentity};
pub use limits::{ClientConfig, ClientLimits, ConfigError, ServerConfig, ServerLimits};
pub use server::{
    BindError, ConnectionId, QueueTotals, QuicServer, ResumeReceived, ServerError, ServerEvent,
    ServerLane, ServerNotice, ShutdownReport,
};
