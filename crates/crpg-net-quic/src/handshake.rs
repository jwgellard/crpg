//! Wire constants, the length-prefixed frame header, and the hello/welcome
//! handshake payloads (ADR-0025, `tasks/T023.md` E§4).
//!
//! Nothing here touches a socket: these are pure byte functions shared by
//! the server, the client and the tests.

use std::fmt;

use crpg_net::{protocol, protocol_v2};

/// ALPN identifier: lane-0 framing + hello/welcome layout version 1.
pub const ALPN_LANE0_V1: &[u8] = b"crpg-lane0/1";
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
