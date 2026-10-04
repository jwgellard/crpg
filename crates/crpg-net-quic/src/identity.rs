//! The pinned server identity (D03, ADR-0025): a SHA-256 certificate pin
//! for clients and a certificate + key pair for servers.

use std::fmt;

use crate::limits::ConfigError;
use crate::tls;

/// SHA-256 of a server certificate's DER. `Debug` prints `CertificatePin(<64 hex>)`.
///
/// The pin, distributed out of band with an invitation, is the whole trust
/// decision: there is no hostname, validity-period or revocation check.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CertificatePin([u8; 32]);

impl CertificatePin {
    /// Wraps an already computed SHA-256 digest.
    pub fn from_sha256(digest: [u8; 32]) -> CertificatePin {
        CertificatePin(digest)
    }

    /// Hashes `cert_der` with the ring provider's SHA-256 (via rustls).
    pub fn of_certificate(cert_der: &[u8]) -> CertificatePin {
        CertificatePin(tls::sha256(cert_der))
    }

    /// The 32 digest bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Exactly 64 lowercase hex digits.
    pub fn to_hex(&self) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            out.push(char::from(DIGITS[usize::from(byte >> 4)]));
            out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
        }
        out
    }

    /// Exactly 64 lowercase hex digits, else `ConfigError::InvalidPin`.
    pub fn from_hex(text: &str) -> Result<CertificatePin, ConfigError> {
        let bytes = text.as_bytes();
        if bytes.len() != 64 {
            return Err(ConfigError::InvalidPin);
        }
        let mut digest = [0u8; 32];
        for (i, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
            let hi = lower_hex_value(pair[0]).ok_or(ConfigError::InvalidPin)?;
            let lo = lower_hex_value(pair[1]).ok_or(ConfigError::InvalidPin)?;
            digest[i] = (hi << 4) | lo;
        }
        Ok(CertificatePin(digest))
    }
}

fn lower_hex_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    }
}

impl fmt::Debug for CertificatePin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CertificatePin({})", self.to_hex())
    }
}

/// Self-signed leaf certificate DER + PKCS#8 private key DER, operator-supplied.
/// Validated at `QuicServer::bind`. `Debug` prints the pin and `key: <redacted>`.
#[derive(Clone)]
pub struct ServerIdentity {
    cert_der: Vec<u8>,
    key_pkcs8_der: Vec<u8>,
}

impl ServerIdentity {
    /// Stores the DER bytes unchecked; `QuicServer::bind` validates them.
    pub fn new(cert_der: Vec<u8>, key_pkcs8_der: Vec<u8>) -> ServerIdentity {
        ServerIdentity {
            cert_der,
            key_pkcs8_der,
        }
    }

    /// The pin clients must hold for this certificate.
    pub fn pin(&self) -> CertificatePin {
        CertificatePin::of_certificate(&self.cert_der)
    }

    pub(crate) fn cert_der(&self) -> &[u8] {
        &self.cert_der
    }

    pub(crate) fn key_pkcs8_der(&self) -> &[u8] {
        &self.key_pkcs8_der
    }
}

impl fmt::Debug for ServerIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerIdentity")
            .field("pin", &self.pin())
            .field("key", &format_args!("<redacted>"))
            .finish()
    }
}
