//! rustls/quinn configuration builders and the pinned certificate verifier
//! (E§5, E§8.3). Private: no rustls or quinn type leaves this crate.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{IdleTimeout, TransportConfig, VarInt};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, PeerIncompatible, SignatureScheme};

use crate::handshake::ALPN_LANE0_V1;
use crate::identity::{CertificatePin, ServerIdentity};

/// The ring crypto provider, built fresh (no process-wide default).
fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// SHA-256 through the hash provider of ring's `TLS13_AES_128_GCM_SHA256`
/// suite, so there is no direct `ring` edge.
pub(crate) fn sha256(data: &[u8]) -> [u8; 32] {
    let suite = rustls::crypto::ring::cipher_suite::TLS13_AES_128_GCM_SHA256
        .tls13()
        .expect("ring's TLS13_AES_128_GCM_SHA256 is a TLS 1.3 suite");
    let output = suite.common.hash_provider.hash(data);
    let mut digest = [0u8; 32];
    digest.copy_from_slice(output.as_ref());
    digest
}

/// The transport parameters fixed by E§8.3.
struct Windows {
    bidi_streams: u32,
    stream_receive_window: u32,
    receive_window: u32,
    send_window: u64,
}

const SERVER_WINDOWS: Windows = Windows {
    bidi_streams: 1,
    stream_receive_window: 65_536,
    receive_window: 131_072,
    send_window: 1_048_576,
};

const CLIENT_WINDOWS: Windows = Windows {
    bidi_streams: 0,
    stream_receive_window: 1_048_576,
    receive_window: 2_097_152,
    send_window: 262_144,
};

fn transport(windows: &Windows, idle_timeout_ms: u32, keep_alive_ms: u32) -> TransportConfig {
    let mut config = TransportConfig::default();
    config
        .max_concurrent_bidi_streams(VarInt::from_u32(windows.bidi_streams))
        .max_concurrent_uni_streams(VarInt::from_u32(0))
        .datagram_receive_buffer_size(None)
        .datagram_send_buffer_size(0)
        .stream_receive_window(VarInt::from_u32(windows.stream_receive_window))
        .receive_window(VarInt::from_u32(windows.receive_window))
        .send_window(windows.send_window)
        .max_idle_timeout(Some(IdleTimeout::from(VarInt::from_u32(idle_timeout_ms))))
        .keep_alive_interval(if keep_alive_ms == 0 {
            None
        } else {
            Some(Duration::from_millis(u64::from(keep_alive_ms)))
        });
    config
}

/// The server's quinn config: TLS 1.3 only, ring, no client auth, the
/// single identity certificate, ALPN `crpg-lane0/1`, no 0-RTT, no
/// migration, `max_incoming = max_connections`. `None` when the identity
/// does not load (bad DER, key not matching the certificate).
pub(crate) fn server_config(
    identity: &ServerIdentity,
    idle_timeout_ms: u32,
    keep_alive_ms: u32,
    max_connections: usize,
) -> Option<quinn::ServerConfig> {
    let cert = CertificateDer::from(identity.cert_der().to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.key_pkcs8_der().to_vec()));
    let mut tls = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .ok()?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .ok()?;
    tls.alpn_protocols = vec![ALPN_LANE0_V1.to_vec()];
    tls.max_early_data_size = 0;
    let crypto = QuicServerConfig::try_from(tls).ok()?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    config
        .transport_config(Arc::new(transport(
            &SERVER_WINDOWS,
            idle_timeout_ms,
            keep_alive_ms,
        )))
        .migration(false)
        .max_incoming(max_connections);
    Some(config)
}

/// The client's quinn config: TLS 1.3 only, ring, the pinned verifier, no
/// client auth, ALPN `crpg-lane0/1`, no 0-RTT and no session resumption.
pub(crate) fn client_config(
    pin: CertificatePin,
    mismatch: Arc<AtomicBool>,
    idle_timeout_ms: u32,
    keep_alive_ms: u32,
) -> Option<quinn::ClientConfig> {
    let provider = provider();
    let verifier = PinnedServerVerifier {
        pin,
        provider: provider.clone(),
        mismatch,
    };
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .ok()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN_LANE0_V1.to_vec()];
    tls.enable_early_data = false;
    tls.resumption = rustls::client::Resumption::disabled();
    let crypto = QuicClientConfig::try_from(tls).ok()?;
    let mut config = quinn::ClientConfig::new(Arc::new(crypto));
    config.transport_config(Arc::new(transport(
        &CLIENT_WINDOWS,
        idle_timeout_ms,
        keep_alive_ms,
    )));
    Some(config)
}

/// Accepts exactly one certificate (no intermediates) whose DER hashes to
/// the pin. Server name and time are ignored: the pin is the whole trust
/// decision (D03). The TLS 1.3 signature is still verified, so the server
/// must hold the pinned key.
#[derive(Debug)]
pub(crate) struct PinnedServerVerifier {
    pin: CertificatePin,
    provider: Arc<CryptoProvider>,
    mismatch: Arc<AtomicBool>,
}

impl ServerCertVerifier for PinnedServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let digest = sha256(end_entity.as_ref());
        let mut acc = 0u8;
        for (a, b) in digest.iter().zip(self.pin.as_bytes()) {
            acc |= a ^ b;
        }
        let pinned = core::hint::black_box(acc) == 0;
        if intermediates.is_empty() && pinned {
            Ok(ServerCertVerified::assertion())
        } else {
            self.mismatch.store(true, Ordering::SeqCst);
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            PeerIncompatible::Tls13RequiredForQuic,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
