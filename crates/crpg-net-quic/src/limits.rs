//! Endpoint limits and configs (E§7, E§8.3, as amended by the 2026-10-04
//! decisions: 64 connections, 60 s idle, 15 s keep-alive).
//!
//! Every limit defaults to its v1 value and may only be tightened: a value
//! looser than v1 is [`ConfigError::LoosensPolicy`], a value below a floor
//! or inconsistent with another field is [`ConfigError::InvalidLimits`].

use std::fmt;
use std::net::SocketAddr;

use crpg_net::protocol::{
    MAX_DELTA_FRAME_BYTES, MAX_INTENT_FRAME_BYTES, POLICY_EGRESS_BYTES_PER_PEER,
    POLICY_EGRESS_FRAMES_PER_PEER, POLICY_INGRESS_BYTES_PER_PEER, POLICY_INGRESS_FRAMES_PER_PEER,
};
use crpg_net::sim::QueueCaps;

use crate::identity::{CertificatePin, ServerIdentity};

const V1_IDLE_TIMEOUT_MS: u32 = 60_000;
const MIN_IDLE_TIMEOUT_MS: u32 = 1_000;
const V1_KEEP_ALIVE_MS: u32 = 15_000;
const MIN_KEEP_ALIVE_MS: u32 = 100;
const V1_HELLO_TIMEOUT_MS: u32 = 5_000;
const V1_DECISION_TIMEOUT_MS: u32 = 5_000;
const V1_CONNECT_TIMEOUT_MS: u32 = 10_000;
const MIN_PHASE_TIMEOUT_MS: u32 = 100;
const V1_CLOSE_FLUSH_TIMEOUT_MS: u32 = 2_000;
const V1_MAX_CONNECTIONS: usize = 64;

/// Server endpoint limits. Defaults are [`ServerLimits::v1`]; tightening-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerLimits {
    /// QUIC idle timeout (`1_000..=60_000`).
    pub idle_timeout_ms: u32,
    /// QUIC keep-alive interval: 0 (off) or `100..idle_timeout_ms`.
    pub keep_alive_ms: u32,
    /// Time allowed for a complete hello after TLS (`100..=5_000`).
    pub hello_timeout_ms: u32,
    /// Time allowed for the host's accept/refuse decision (`100..=5_000`).
    pub decision_timeout_ms: u32,
    /// Flush and close-transmit budget for `Flush`, shutdown and drop (`0..=2_000`).
    pub close_flush_timeout_ms: u32,
    /// Live connections, pending (not yet accepted) included.
    pub max_connections: usize,
    /// Client → server queue (per connection + host-wide).
    pub inbound: QueueCaps,
    /// Server → client queue (per connection + host-wide).
    pub outbound: QueueCaps,
}

impl ServerLimits {
    /// The v1 envelope: 60 s idle, 15 s keep-alive, 5 s hello, 5 s decision,
    /// 2 s close flush, 64 connections, `QueueCaps::v1()` inbound and
    /// `QueueCaps::v1_egress()` outbound.
    pub fn v1() -> ServerLimits {
        ServerLimits {
            idle_timeout_ms: V1_IDLE_TIMEOUT_MS,
            keep_alive_ms: V1_KEEP_ALIVE_MS,
            hello_timeout_ms: V1_HELLO_TIMEOUT_MS,
            decision_timeout_ms: V1_DECISION_TIMEOUT_MS,
            close_flush_timeout_ms: V1_CLOSE_FLUSH_TIMEOUT_MS,
            max_connections: V1_MAX_CONNECTIONS,
            inbound: QueueCaps::v1(),
            outbound: QueueCaps::v1_egress(),
        }
    }

    /// Checks every field in declaration order; the first failure wins, and
    /// `InvalidLimits` is reported before `LoosensPolicy` for one field.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let v1 = ServerLimits::v1();
        bounded(
            "idle_timeout_ms",
            self.idle_timeout_ms,
            MIN_IDLE_TIMEOUT_MS,
            v1.idle_timeout_ms,
        )?;
        keep_alive("keep_alive_ms", self.keep_alive_ms, self.idle_timeout_ms)?;
        bounded(
            "hello_timeout_ms",
            self.hello_timeout_ms,
            MIN_PHASE_TIMEOUT_MS,
            v1.hello_timeout_ms,
        )?;
        bounded(
            "decision_timeout_ms",
            self.decision_timeout_ms,
            MIN_PHASE_TIMEOUT_MS,
            v1.decision_timeout_ms,
        )?;
        bounded(
            "close_flush_timeout_ms",
            self.close_flush_timeout_ms,
            0,
            v1.close_flush_timeout_ms,
        )?;
        bounded(
            "max_connections",
            self.max_connections,
            1,
            v1.max_connections,
        )?;
        queue(
            &INBOUND_FIELDS,
            &self.inbound,
            &v1.inbound,
            MAX_INTENT_FRAME_BYTES,
        )?;
        queue(
            &OUTBOUND_FIELDS,
            &self.outbound,
            &v1.outbound,
            MAX_DELTA_FRAME_BYTES,
        )?;
        Ok(())
    }
}

/// Client endpoint limits. Defaults are [`ClientLimits::v1`]; tightening-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientLimits {
    /// QUIC idle timeout (`1_000..=60_000`).
    pub idle_timeout_ms: u32,
    /// QUIC keep-alive interval: 0 (off) or `100..idle_timeout_ms`.
    pub keep_alive_ms: u32,
    /// Bound on the whole `connect` sequence (`100..=10_000`).
    pub connect_timeout_ms: u32,
    /// Flush and close-transmit budget for `Flush` and drop (`0..=2_000`).
    pub close_flush_timeout_ms: u32,
    /// Queued server → client frames (`1..=128`).
    pub inbound_frames: usize,
    /// Queued server → client bytes (`65_536..=2_097_152`).
    pub inbound_bytes: usize,
    /// Queued client → server frames (`1..=128`).
    pub outbound_frames: usize,
    /// Queued client → server bytes (`4_096..=262_144`).
    pub outbound_bytes: usize,
}

impl ClientLimits {
    /// The v1 envelope: 60 s idle, 15 s keep-alive, 10 s connect, 2 s close
    /// flush, inbound 128 frames / 2 MiB, outbound 128 frames / 256 KiB.
    pub fn v1() -> ClientLimits {
        ClientLimits {
            idle_timeout_ms: V1_IDLE_TIMEOUT_MS,
            keep_alive_ms: V1_KEEP_ALIVE_MS,
            connect_timeout_ms: V1_CONNECT_TIMEOUT_MS,
            close_flush_timeout_ms: V1_CLOSE_FLUSH_TIMEOUT_MS,
            inbound_frames: POLICY_EGRESS_FRAMES_PER_PEER,
            inbound_bytes: POLICY_EGRESS_BYTES_PER_PEER,
            outbound_frames: POLICY_INGRESS_FRAMES_PER_PEER,
            outbound_bytes: POLICY_INGRESS_BYTES_PER_PEER,
        }
    }

    /// Checks every field in declaration order; the first failure wins, and
    /// `InvalidLimits` is reported before `LoosensPolicy` for one field.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let v1 = ClientLimits::v1();
        bounded(
            "idle_timeout_ms",
            self.idle_timeout_ms,
            MIN_IDLE_TIMEOUT_MS,
            v1.idle_timeout_ms,
        )?;
        keep_alive("keep_alive_ms", self.keep_alive_ms, self.idle_timeout_ms)?;
        bounded(
            "connect_timeout_ms",
            self.connect_timeout_ms,
            MIN_PHASE_TIMEOUT_MS,
            v1.connect_timeout_ms,
        )?;
        bounded(
            "close_flush_timeout_ms",
            self.close_flush_timeout_ms,
            0,
            v1.close_flush_timeout_ms,
        )?;
        bounded("inbound_frames", self.inbound_frames, 1, v1.inbound_frames)?;
        bounded(
            "inbound_bytes",
            self.inbound_bytes,
            MAX_DELTA_FRAME_BYTES,
            v1.inbound_bytes,
        )?;
        bounded(
            "outbound_frames",
            self.outbound_frames,
            1,
            v1.outbound_frames,
        )?;
        bounded(
            "outbound_bytes",
            self.outbound_bytes,
            MAX_INTENT_FRAME_BYTES,
            v1.outbound_bytes,
        )?;
        Ok(())
    }
}

/// Everything `QuicServer::bind` needs.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Local UDP address to bind.
    pub bind: SocketAddr,
    /// The certificate and key the server presents.
    pub identity: ServerIdentity,
    /// Tightening-only limits.
    pub limits: ServerLimits,
}

/// Everything `QuicClient::connect` needs besides the server address and hello.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Local UDP address to bind.
    pub bind: SocketAddr,
    /// SHA-256 pin of the server certificate.
    pub pin: CertificatePin,
    /// Tightening-only limits.
    pub limits: ClientLimits,
}

/// A limit or pin the configuration rejects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// Zero, out of range, inconsistent, or unable to hold one maximal frame.
    InvalidLimits {
        /// The failing field (`inbound.per_peer_frames` style for nested caps).
        field: &'static str,
    },
    /// Looser than the v1 envelope (tightening-only).
    LoosensPolicy {
        /// The failing field.
        field: &'static str,
    },
    /// A pin string that is not 64 lowercase hex digits.
    InvalidPin,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            ConfigError::InvalidLimits { .. } => "InvalidLimits",
            ConfigError::LoosensPolicy { .. } => "LoosensPolicy",
            ConfigError::InvalidPin => "InvalidPin",
        };
        write!(f, "{name} at quic/config")
    }
}

impl std::error::Error for ConfigError {}

/// `floor <= value <= v1`, reporting the floor first.
fn bounded<T: PartialOrd>(
    field: &'static str,
    value: T,
    floor: T,
    v1: T,
) -> Result<(), ConfigError> {
    if value < floor {
        return Err(ConfigError::InvalidLimits { field });
    }
    if value > v1 {
        return Err(ConfigError::LoosensPolicy { field });
    }
    Ok(())
}

/// Keep-alive is not a policy cap: 0 (off), or `MIN..idle` (strictly less).
fn keep_alive(field: &'static str, value: u32, idle: u32) -> Result<(), ConfigError> {
    if value != 0 && (value < MIN_KEEP_ALIVE_MS || value >= idle) {
        return Err(ConfigError::InvalidLimits { field });
    }
    Ok(())
}

struct QueueFields {
    per_peer_frames: &'static str,
    per_peer_bytes: &'static str,
    host_frames: &'static str,
    host_bytes: &'static str,
}

const INBOUND_FIELDS: QueueFields = QueueFields {
    per_peer_frames: "inbound.per_peer_frames",
    per_peer_bytes: "inbound.per_peer_bytes",
    host_frames: "inbound.host_frames",
    host_bytes: "inbound.host_bytes",
};

const OUTBOUND_FIELDS: QueueFields = QueueFields {
    per_peer_frames: "outbound.per_peer_frames",
    per_peer_bytes: "outbound.per_peer_bytes",
    host_frames: "outbound.host_frames",
    host_bytes: "outbound.host_bytes",
};

/// Frames `>= 1`, bytes `>=` one maximal frame of that direction, host-wide
/// `>=` per-connection, and nothing looser than v1.
fn queue(
    names: &QueueFields,
    caps: &QueueCaps,
    v1: &QueueCaps,
    frame_cap: usize,
) -> Result<(), ConfigError> {
    bounded(
        names.per_peer_frames,
        caps.per_peer_frames,
        1,
        v1.per_peer_frames,
    )?;
    bounded(
        names.per_peer_bytes,
        caps.per_peer_bytes,
        frame_cap,
        v1.per_peer_bytes,
    )?;
    bounded(
        names.host_frames,
        caps.host_frames,
        caps.per_peer_frames,
        v1.host_frames,
    )?;
    bounded(
        names.host_bytes,
        caps.host_bytes,
        caps.per_peer_bytes,
        v1.host_bytes,
    )?;
    Ok(())
}
