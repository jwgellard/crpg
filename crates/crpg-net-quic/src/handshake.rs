//! Wire constants, the length-prefixed frame header, and the hello/welcome
//! handshake payloads (ADR-0025, `tasks/T023.md` E§4).
//!
//! Nothing here touches a socket: these are pure byte functions shared by
//! the server, the client and the tests.

use std::fmt;

use crpg_net::{protocol, protocol_resume, protocol_transfer, protocol_v2};

/// ALPN identifier: lane-0 framing + hello/welcome layout version 1.
pub const ALPN_LANE0_V1: &[u8] = b"crpg-lane0/1";
/// ALPN identifier: lane framing version 2 (byte 0 of each lane-frame
/// header is a channel selector). Hello/welcome layout as version 1; a
/// `bind_resumable` server also accepts a resume opening.
pub const ALPN_LANE0_V2: &[u8] = b"crpg-lane0/2";
/// SNI sent by clients. Ignored by verification (the pin is the trust root).
pub const TLS_SERVER_NAME: &str = "crpg.invalid";
/// Length-prefix size: one big-endian u32.
pub const FRAME_HEADER_BYTES: usize = 4;
/// Credential length floor (128-bit entropy floor; content is host-defined).
pub const MIN_CREDENTIAL_BYTES: usize = 16;
/// Credential length ceiling.
pub const MAX_CREDENTIAL_BYTES: usize = 128;
/// Hello payload ceiling: wire version + length byte + credential.
pub const MAX_HELLO_BYTES: usize = 2 + MAX_CREDENTIAL_BYTES; // 130
/// Exact welcome payload size: wire version + 16-byte session epoch.
pub const WELCOME_BYTES: usize = 1 + 16; // 17

/// A frame length the framing rejects before any allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// A zero-length frame (every lane-0 frame has at least a version byte).
    Empty,
    /// A length above the direction's cap.
    TooLarge,
    /// A channel selector that is not `0x00`, `0x81` or `0x91`
    /// (`parse_channel_frame_header` only; the `/1` header never returns it).
    UnknownChannel,
}

/// The 4-byte header for a `len`-byte payload under `cap`.
pub fn frame_header(len: usize, cap: usize) -> Result<[u8; FRAME_HEADER_BYTES], FrameError> {
    if len == 0 {
        return Err(FrameError::Empty);
    }
    if len > cap {
        return Err(FrameError::TooLarge);
    }
    let len = u32::try_from(len).map_err(|_| FrameError::TooLarge)?;
    Ok(len.to_be_bytes())
}

/// The payload length a header declares, checked against `cap` (no allocation).
pub fn parse_frame_header(
    header: [u8; FRAME_HEADER_BYTES],
    cap: usize,
) -> Result<usize, FrameError> {
    let len = usize::try_from(u32::from_be_bytes(header)).map_err(|_| FrameError::TooLarge)?;
    if len == 0 {
        return Err(FrameError::Empty);
    }
    if len > cap {
        return Err(FrameError::TooLarge);
    }
    Ok(len)
}

/// The lane framing a connection negotiated by ALPN; fixed for its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    /// `crpg-lane0/1`: 4-byte length header, lane 0 only.
    V1,
    /// `crpg-lane0/2`: `[channel][24-bit length]` header; lane 0,
    /// transfer and resume channels.
    V2,
}

impl Framing {
    /// `ALPN_LANE0_V1` or `ALPN_LANE0_V2`.
    pub fn alpn(self) -> &'static [u8] {
        match self {
            Framing::V1 => ALPN_LANE0_V1,
            Framing::V2 => ALPN_LANE0_V2,
        }
    }

    /// Exact match against the two ALPNs, else `None`.
    pub fn from_alpn(alpn: &[u8]) -> Option<Framing> {
        if alpn == ALPN_LANE0_V1 {
            Some(Framing::V1)
        } else if alpn == ALPN_LANE0_V2 {
            Some(Framing::V2)
        } else {
            None
        }
    }
}

/// A `crpg-lane0/2` lane channel. The value is the header's first byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Channel {
    /// Lane-0 intents and deltas (wire version fixed by the hello).
    Lane0 = 0x00,
    /// Snapshot-transfer frames (`crpg_net::codec_transfer`, version 0x81).
    Transfer = 0x81,
    /// Resume frames (`crpg_net::codec_resume`, version 0x91): the opening
    /// request upstream; the answer and tickets downstream.
    Resume = 0x91,
}

impl Channel {
    /// The channel's selector byte.
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// `Some` only for 0x00, 0x81 and 0x91.
    pub fn from_u8(raw: u8) -> Option<Channel> {
        match raw {
            0x00 => Some(Channel::Lane0),
            0x81 => Some(Channel::Transfer),
            0x91 => Some(Channel::Resume),
            _ => None,
        }
    }
}

/// The 4-byte `/2` header `[channel][24-bit length]`, checked against `cap`.
///
/// Precedence (`tasks/T024q.md` S§5.4): `Empty`, then `TooLarge` (either
/// above `cap` or above the 24-bit maximum), then the channel byte.
pub fn channel_frame_header(
    channel: Channel,
    len: usize,
    cap: usize,
) -> Result<[u8; FRAME_HEADER_BYTES], FrameError> {
    if len == 0 {
        return Err(FrameError::Empty);
    }
    if len > cap || len > 0x00FF_FFFF {
        return Err(FrameError::TooLarge);
    }
    Ok([
        channel.as_u8(),
        ((len >> 16) & 0xFF) as u8,
        ((len >> 8) & 0xFF) as u8,
        (len & 0xFF) as u8,
    ])
}

/// The channel and payload length a `/2` header declares, checked against
/// `cap` (no allocation). The channel is checked first, so a header with an
/// unknown selector is `UnknownChannel` even when its length exceeds `cap`.
pub fn parse_channel_frame_header(
    header: [u8; FRAME_HEADER_BYTES],
    cap: usize,
) -> Result<(Channel, usize), FrameError> {
    let channel = Channel::from_u8(header[0]).ok_or(FrameError::UnknownChannel)?;
    let len = usize::try_from(u32::from_be_bytes([0, header[1], header[2], header[3]]))
        .map_err(|_| FrameError::TooLarge)?;
    if len == 0 {
        return Err(FrameError::Empty);
    }
    if len > cap {
        return Err(FrameError::TooLarge);
    }
    Ok((channel, len))
}

// The S§5.4 channel/protocol agreements, checked at compile time.
const _: () = assert!(Channel::Transfer as u8 == protocol_transfer::TRANSFER_VERSION);
const _: () = assert!(Channel::Resume as u8 == protocol_resume::RESUME_VERSION);
const _: () = assert!(protocol_transfer::MAX_DOWN_FRAME_BYTES == protocol::MAX_DELTA_FRAME_BYTES);
const _: () = assert!(protocol_transfer::MAX_UP_FRAME_BYTES == protocol::MAX_INTENT_FRAME_BYTES);
const _: () = assert!(protocol_resume::MAX_RESUME_FRAME_BYTES == protocol::MAX_INTENT_FRAME_BYTES);
const _: () = assert!(protocol::MAX_DELTA_FRAME_BYTES <= 0x00FF_FFFF);

/// An invitation credential: opaque bytes, `MIN..=MAX_CREDENTIAL_BYTES` long.
///
/// No `PartialEq`, no `Display`; `Debug` prints exactly `Credential(<redacted>)`.
/// Judging validity belongs to the host adapter (T023b); this type checks
/// only the length.
#[derive(Clone)]
pub struct Credential(Vec<u8>);

impl Credential {
    /// Wraps `bytes` when their length is inside the credential bounds.
    pub fn new(bytes: Vec<u8>) -> Result<Credential, CredentialError> {
        if bytes.len() < MIN_CREDENTIAL_BYTES {
            return Err(CredentialError::TooShort);
        }
        if bytes.len() > MAX_CREDENTIAL_BYTES {
            return Err(CredentialError::TooLong);
        }
        Ok(Credential(bytes))
    }

    /// The opaque credential bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Constant-time equality over equal lengths (unequal lengths: `false`
    /// without inspecting content). XOR-fold over every byte, no early exit,
    /// result passed through `core::hint::black_box`.
    pub fn ct_eq(&self, expected: &[u8]) -> bool {
        if self.0.len() != expected.len() {
            return false;
        }
        let mut acc = 0u8;
        for (a, b) in self.0.iter().zip(expected) {
            acc |= a ^ b;
        }
        core::hint::black_box(acc) == 0
    }
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credential(<redacted>)")
    }
}

/// A credential length outside `MIN..=MAX_CREDENTIAL_BYTES`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialError {
    /// Fewer than [`MIN_CREDENTIAL_BYTES`] bytes.
    TooShort,
    /// More than [`MAX_CREDENTIAL_BYTES`] bytes.
    TooLong,
}

/// Client → server, first frame on the lane stream.
#[derive(Debug, Clone)]
pub struct Hello {
    /// Lane-0 wire version the client speaks: 1 (`protocol`) or 2 (`protocol_v2`).
    pub wire_version: u8,
    /// The invitation credential (format-checked only).
    pub credential: Credential,
}

impl Hello {
    /// `[wire_version][cred_len][credential]`.
    pub fn encode(&self) -> Result<Vec<u8>, HandshakeError> {
        check_wire_version(self.wire_version)?;
        let cred = self.credential.as_bytes();
        let cred_len = u8::try_from(cred.len()).map_err(|_| HandshakeError::CredentialLength)?;
        let mut out = Vec::with_capacity(2 + cred.len());
        out.push(self.wire_version);
        out.push(cred_len);
        out.extend_from_slice(cred);
        Ok(out)
    }

    /// Decodes a hello payload with the E§4 precedence: length < 2 →
    /// `Malformed`; unknown version → `UnsupportedWireVersion`; credential
    /// length out of bounds → `CredentialLength`; total ≠ `2 + cred_len` →
    /// `Malformed`.
    pub fn decode(bytes: &[u8]) -> Result<Hello, HandshakeError> {
        if bytes.len() < 2 {
            return Err(HandshakeError::Malformed);
        }
        let wire_version = bytes[0];
        check_wire_version(wire_version)?;
        let cred_len = usize::from(bytes[1]);
        if !(MIN_CREDENTIAL_BYTES..=MAX_CREDENTIAL_BYTES).contains(&cred_len) {
            return Err(HandshakeError::CredentialLength);
        }
        if bytes.len() != 2 + cred_len {
            return Err(HandshakeError::Malformed);
        }
        let credential =
            Credential::new(bytes[2..].to_vec()).map_err(|_| HandshakeError::CredentialLength)?;
        Ok(Hello {
            wire_version,
            credential,
        })
    }
}

/// Server → client, first frame on the lane stream after the host accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Welcome {
    /// Must equal the hello's `wire_version`.
    pub wire_version: u8,
    /// The host-issued session epoch the client puts in every intent.
    pub epoch: [u8; 16],
}

impl Welcome {
    /// `[wire_version][epoch: 16]`.
    pub fn encode(&self) -> Result<[u8; WELCOME_BYTES], HandshakeError> {
        check_wire_version(self.wire_version)?;
        let mut out = [0u8; WELCOME_BYTES];
        out[0] = self.wire_version;
        out[1..].copy_from_slice(&self.epoch);
        Ok(out)
    }

    /// Length ≠ 17 → `Malformed`; then unknown version → `UnsupportedWireVersion`.
    pub fn decode(bytes: &[u8]) -> Result<Welcome, HandshakeError> {
        if bytes.len() != WELCOME_BYTES {
            return Err(HandshakeError::Malformed);
        }
        check_wire_version(bytes[0])?;
        let mut epoch = [0u8; 16];
        epoch.copy_from_slice(&bytes[1..]);
        Ok(Welcome {
            wire_version: bytes[0],
            epoch,
        })
    }
}

/// A hello or welcome payload the handshake rejects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeError {
    /// Wrong length, truncated, or trailing bytes.
    Malformed,
    /// A wire version other than 1 or 2.
    UnsupportedWireVersion,
    /// A credential length byte outside `MIN..=MAX_CREDENTIAL_BYTES`.
    CredentialLength,
}

fn check_wire_version(version: u8) -> Result<(), HandshakeError> {
    if version == protocol::PROTOCOL_VERSION || version == protocol_v2::PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(HandshakeError::UnsupportedWireVersion)
    }
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            FrameError::Empty => "Empty",
            FrameError::TooLarge => "TooLarge",
            FrameError::UnknownChannel => "UnknownChannel",
        };
        write!(f, "{name} at quic/frame")
    }
}

impl std::error::Error for FrameError {}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            CredentialError::TooShort => "TooShort",
            CredentialError::TooLong => "TooLong",
        };
        write!(f, "{name} at quic/credential")
    }
}

impl std::error::Error for CredentialError {}

impl fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            HandshakeError::Malformed => "Malformed",
            HandshakeError::UnsupportedWireVersion => "UnsupportedWireVersion",
            HandshakeError::CredentialLength => "CredentialLength",
        };
        write!(f, "{name} at quic/handshake")
    }
}

impl std::error::Error for HandshakeError {}
