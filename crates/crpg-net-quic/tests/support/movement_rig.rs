#![allow(dead_code)] // Each test binary that includes this rig uses a different subset.
//! Test-only rig for `tests/movement.rs` (T150b): a movement server and
//! client over loopback, a raw quinn client offering exactly `crpg-lane0/3`
//! with its own exact-certificate verifier, and the byte builders the lane-1
//! and datagram cases share.
//!
//! It builds on `support/quic_rig.rs` (included by the test crate root as
//! `quic_rig`). Waits are the rig's 30 s failure guards, never oracles.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crpg_net::movement_v4::{
    encode_down, encode_intent, MoveBody, MoveDown, MoveIntent, MoveOutcome, MoveReceipt,
};
use crpg_net::protocol::NetId;
use crpg_net::protocol_v4::Point;
use crpg_net_quic::{
    ClientLimits, ConnectionId, Hello, QuicClient, QuicServer, ServerConfig, ServerLimits, Welcome,
    ALPN_LANE0_V3, TLS_SERVER_NAME,
};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

use crate::quic_rig::{
    client_config, credential, epoch, identity_a, loopback, next_hello, until, RawClient,
    RawRuntime, DEADLINE, SERVER_A_CERT,
};

/// `bind_movement` on loopback with the rig's identity.
pub fn bind_movement_server(limits: ServerLimits) -> QuicServer {
    QuicServer::bind_movement(ServerConfig {
        bind: loopback(),
        identity: identity_a(),
        limits,
    })
    .expect("bind movement server")
}

/// A version-4 hello with a 32-byte credential of `fill`.
pub fn hello4(fill: u8) -> Hello {
    Hello {
        wire_version: 4,
        credential: credential(32, fill),
    }
}

/// Runs `QuicClient::connect_movement` on its own thread, so the caller can
/// drive the server's decision meanwhile.
pub fn connect_movement_async(
    limits: ClientLimits,
    server: SocketAddr,
    hello: Hello,
) -> std::thread::JoinHandle<Result<QuicClient, crpg_net_quic::ConnectError>> {
    std::thread::spawn(move || QuicClient::connect_movement(client_config(limits), server, &hello))
}

/// Connects a movement client with `hello4(fill)` and accepts it with
/// `Welcome { 4, epoch }`.
pub fn accept_movement(
    server: &mut QuicServer,
    limits: ClientLimits,
    fill: u8,
    epoch: [u8; 16],
) -> (QuicClient, ConnectionId) {
    let pending = connect_movement_async(limits, server.local_addr(), hello4(fill));
    let (conn, _, _) = next_hello(server);
    server
        .accept(
            conn,
            Welcome {
                wire_version: 4,
                epoch,
            },
        )
        .expect("accept movement");
    let client = pending.join().expect("client thread").expect("connect");
    (client, conn)
}

/// The raw client's own pin check (`quic_rig`'s is private to that module):
/// the presented certificate must be byte-identical to the expected DER.
#[derive(Debug)]
struct ExactCertificate {
    expected: Vec<u8>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for ExactCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if intermediates.is_empty() && end_entity.as_ref() == self.expected.as_slice() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
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
            rustls::PeerIncompatible::Tls13RequiredForQuic,
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

/// A raw quinn connection to `server` offering exactly `crpg-lane0/3`, whose
/// transport configuration has `datagram_receive_buffer_size(receive_buffer)`
/// and quinn's defaults otherwise. An `Err` is a TLS or handshake failure; a
/// server that closes the connection after TLS still gives `Ok`.
pub fn raw_movement(
    server: SocketAddr,
    receive_buffer: Option<usize>,
) -> Result<RawClient, quinn::ConnectionError> {
    let rt = RawRuntime::new();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("tls13")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(ExactCertificate {
            expected: SERVER_A_CERT.to_vec(),
            provider,
        }))
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN_LANE0_V3.to_vec()];
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls).expect("quic tls");
    let mut config = quinn::ClientConfig::new(Arc::new(crypto));
    let mut transport = quinn::TransportConfig::default();
    transport.datagram_receive_buffer_size(receive_buffer);
    config.transport_config(Arc::new(transport));
    let (endpoint, connection) = rt.run(async move {
        let endpoint = quinn::Endpoint::client(loopback()).expect("raw endpoint");
        let connecting = endpoint
            .connect_with(config, server, TLS_SERVER_NAME)
            .expect("raw connect");
        let connection = connecting.await;
        (endpoint, connection)
    });
    let connection = connection?;
    Ok(RawClient {
        rt,
        endpoint,
        connection,
    })
}

/// The 38-byte hello frame `[0, 0, 0, 34, 4, 32]` followed by 32 bytes of
/// `fill`.
pub fn raw_hello4(fill: u8) -> Vec<u8> {
    let mut out = vec![0, 0, 0, 34, 4, 32];
    out.extend_from_slice(&[fill; 32]);
    out
}

/// Reads the 21 raw bytes of a welcome frame (`quic_rig`'s `read_welcome`
/// decodes with the `/1` rule, which refuses version 4).
pub fn read_raw_welcome4(raw: &RawClient, recv: &mut quinn::RecvStream) -> [u8; 21] {
    let mut frame = [0u8; 21];
    raw.rt
        .run(async { recv.read_exact(&mut frame).await })
        .expect("welcome bytes");
    frame
}

/// A raw `/3` connection that wrote `raw_hello4(7)`, was accepted with
/// `Welcome { 4, epoch }` and read the welcome (the shape M7 starts from).
pub struct RawMovement {
    pub raw: RawClient,
    pub send: quinn::SendStream,
    pub recv: quinn::RecvStream,
    pub conn: ConnectionId,
}

/// Builds a `RawMovement` on `server`.
pub fn raw_accepted(
    server: &mut QuicServer,
    receive_buffer: usize,
    epoch: [u8; 16],
) -> RawMovement {
    let raw = raw_movement(server.local_addr(), Some(receive_buffer)).expect("raw /3");
    let (send, mut recv) = raw.open_and_write(&raw_hello4(7));
    let (conn, _, hello) = next_hello(server);
    assert_eq!(hello.wire_version, 4);
    server
        .accept(
            conn,
            Welcome {
                wire_version: 4,
                epoch,
            },
        )
        .expect("accept raw");
    let welcome = read_raw_welcome4(&raw, &mut recv);
    assert_eq!(&welcome[..5], &[0, 0, 0, 17, 4]);
    RawMovement {
        raw,
        send,
        recv,
        conn,
    }
}

impl RawMovement {
    /// Opens the raw client's second bidi stream and writes `bytes` on it.
    pub fn open_lane1(&self, bytes: &[u8]) -> (quinn::SendStream, quinn::RecvStream) {
        self.raw.open_and_write(bytes)
    }
}

/// `n` bytes, byte `i` = `(k as u8).wrapping_add(i as u8)`.
pub fn pat(k: usize, n: usize) -> Vec<u8> {
    (0..n).map(|i| (k as u8).wrapping_add(i as u8)).collect()
}

/// A real lane-1 `Halt` intent as a value.
pub fn h_intent(seq: u64) -> MoveIntent {
    MoveIntent {
        epoch: epoch(5),
        seq,
        actor: NetId::new(1).unwrap(),
        body: MoveBody::Halt,
    }
}

/// The bytes of `h_intent(seq)`: 21 bytes for `seq <= 127`.
pub fn h(seq: u64) -> Vec<u8> {
    encode_intent(&h_intent(seq)).unwrap()
}

/// A real lane-1 `MoveTo` intent.
pub fn m_intent(seq: u64) -> MoveIntent {
    MoveIntent {
        epoch: epoch(5),
        seq,
        actor: NetId::new(1).unwrap(),
        body: MoveBody::MoveTo {
            destination: Point::new([262_144, 0, 65_536]).unwrap(),
        },
    }
}

/// The bytes of `m_intent(seq)`.
pub fn m(seq: u64) -> Vec<u8> {
    encode_intent(&m_intent(seq)).unwrap()
}

/// A real receipt: 24 bytes.
pub fn r1() -> Vec<u8> {
    encode_down(&MoveDown::Receipt(MoveReceipt {
        epoch: epoch(5),
        seq: 1,
        actor: NetId::new(1).unwrap(),
        processed_tick: 0,
        outcome: MoveOutcome::Accepted { start_tick: 1 },
    }))
    .unwrap()
}

/// Sends `h(1)` on the client's lane 1 and waits until the server's
/// `try_recv_lane1(conn)` returns it: the server then has its lane-1 stream
/// and writer (N-3 (8), Reading 3). Every case that needs server lane-1
/// frames delivered primes first.
pub fn prime(client: &mut QuicClient, server: &mut QuicServer, conn: ConnectionId) {
    client.try_send_lane1(&h(1)).expect("prime send");
    let start = Instant::now();
    loop {
        match server.try_recv_lane1(conn) {
            Ok(Some(frame)) => {
                assert_eq!(frame, h(1), "prime frame");
                return;
            }
            Ok(None) => {}
            Err(error) => panic!("prime: {error:?}"),
        }
        assert!(start.elapsed() < DEADLINE, "deadline: prime");
        server.wait(Duration::from_millis(20));
    }
}

/// Sends `bytes` with `try_send_datagram`, polls `try_recv_datagram` with
/// `client.wait(100 ms)`, resends after each empty poll, asserts every
/// received datagram equals `bytes` and returns the first one that arrives.
pub fn datagram_until(
    server: &mut QuicServer,
    conn: ConnectionId,
    client: &mut QuicClient,
    bytes: &[u8],
) -> Vec<u8> {
    let start = Instant::now();
    loop {
        server
            .try_send_datagram(conn, bytes)
            .expect("send datagram");
        client.wait(Duration::from_millis(100));
        let mut first = None;
        while let Some(datagram) = client.try_recv_datagram().expect("datagram open") {
            assert_eq!(datagram, bytes, "datagram altered");
            first.get_or_insert(datagram);
        }
        if let Some(datagram) = first {
            return datagram;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: datagram");
    }
}

/// Guard: the server's next lane-1 frame on `conn`.
pub fn server_recv_lane1(server: &mut QuicServer, conn: ConnectionId) -> Vec<u8> {
    let mut out = None;
    until("server lane-1 frame", || {
        match server.try_recv_lane1(conn).expect("lane 1 open") {
            Some(frame) => {
                out = Some(frame);
                true
            }
            None => false,
        }
    });
    out.expect("frame")
}

/// Guard: the client's next lane-1 frame.
pub fn client_recv_lane1(client: &mut QuicClient) -> Vec<u8> {
    let mut out = None;
    until("client lane-1 frame", || {
        match client.try_recv_lane1().expect("lane 1 open") {
            Some(frame) => {
                out = Some(frame);
                true
            }
            None => false,
        }
    });
    out.expect("frame")
}
