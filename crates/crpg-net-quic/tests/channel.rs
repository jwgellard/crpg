//! T024q transfer and resume channel suite: the 21 cases of
//! `tasks/T024q.md` S§10, all through the public `crpg_net_quic` API over
//! real QUIC on IPv4 loopback, plus a local raw quinn server for the
//! branches this crate's server never produces (cases 20–21).
//!
//! Waits are the rig's 30 s failure guards, never oracles. Every refusal has
//! a positive control on the same endpoint.

#[path = "support/quic_rig.rs"]
mod quic_rig;

use std::sync::Arc;
use std::time::Duration;

use crpg_net::codec_resume;
use crpg_net::protocol::{MAX_DELTA_FRAME_BYTES, MAX_INTENT_FRAME_BYTES};
use crpg_net::protocol_resume::{
    ConnectionFence, LaneClaim, LaneProgress, ResumeAccepted, ResumeDown, ResumeProof,
    ResumeRejected, ResumeRejection, ResumeRequest, ResumeResult, ResumeTicket,
    MAX_RESUME_FRAME_BYTES, RESUME_VERSION,
};
use crpg_net::protocol_transfer::{TransferChunk, TransferDown, TransferId};
use crpg_net::transport::Transport;
use crpg_net_quic::{
    channel_frame_header, frame_header, parse_channel_frame_header, Channel, ChannelSendError,
    CloseCode, CloseMode, CloseReason, ConnectError, ConnectionId, ConnectionStats, FrameError,
    Framing, Hello, QuicClient, QuicServer, ResumeReceived, SendError, ServerConfig, ServerError,
    ServerEvent, ServerLimits, ServerNotice, Welcome, ALPN_LANE0_V1, ALPN_LANE0_V2,
    MAX_HELLO_BYTES, TLS_SERVER_NAME,
};
use quic_rig::*;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

const INTENT_CAP: usize = MAX_INTENT_FRAME_BYTES;
const DELTA_CAP: usize = MAX_DELTA_FRAME_BYTES;

/// The session a resumed connection keeps (epoch and wire version, D39d Q4).
const SESSION: Welcome = Welcome {
    wire_version: 2,
    epoch: [0; 16],
};

fn queue_fields(stats: ConnectionStats) -> (usize, usize, usize, usize, bool) {
    (
        stats.inbound_frames,
        stats.inbound_bytes,
        stats.outbound_frames,
        stats.outbound_bytes,
        stats.reader_parked,
    )
}

fn connect_framed_async(
    config: crpg_net_quic::ClientConfig,
    server: std::net::SocketAddr,
    hello: Hello,
    framing: Framing,
) -> std::thread::JoinHandle<Result<QuicClient, ConnectError>> {
    std::thread::spawn(move || QuicClient::connect_framed(config, server, &hello, framing))
}

fn connect_resume_async(
    config: crpg_net_quic::ClientConfig,
    server: std::net::SocketAddr,
    session: Welcome,
    request: Vec<u8>,
) -> std::thread::JoinHandle<Result<(QuicClient, Vec<u8>), ConnectError>> {
    std::thread::spawn(move || QuicClient::connect_resume(config, server, session, &request))
}

/// Guard: the next server notice.
fn next_notice(server: &mut QuicServer) -> ServerNotice {
    let start = std::time::Instant::now();
    loop {
        if let Some(notice) = server.poll_notice() {
            return notice;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: server notice");
        server.wait(Duration::from_millis(20));
    }
}

fn next_resume(server: &mut QuicServer) -> ResumeReceived {
    match next_notice(server) {
        ServerNotice::Resume(received) => received,
        other => panic!("expected Resume, got {other:?}"),
    }
}

/// Guard: the next `HelloReceived`, skipping `Closed` notices left by
/// connections earlier in the case.
fn next_hello_any(server: &mut QuicServer) -> (ConnectionId, std::net::SocketAddr, Hello) {
    loop {
        match next_event(server) {
            ServerEvent::HelloReceived {
                conn,
                remote,
                hello,
            } => return (conn, remote, hello),
            ServerEvent::Closed { .. } => {}
        }
    }
}

fn client_recv_channel(client: &mut QuicClient) -> (Channel, Vec<u8>) {
    let start = std::time::Instant::now();
    loop {
        match client.try_recv_channel() {
            Ok(Some(pair)) => return pair,
            Ok(None) => {}
            Err(reason) => panic!("client closed: {reason:?}"),
        }
        assert!(start.elapsed() < DEADLINE, "deadline: client channel frame");
        client.wait(Duration::from_millis(20));
    }
}

fn client_err_channel(client: &mut QuicClient) -> CloseReason {
    let start = std::time::Instant::now();
    loop {
        match client.try_recv_channel() {
            Err(reason) => return reason,
            Ok(Some(_)) => continue,
            Ok(None) => {}
        }
        assert!(start.elapsed() < DEADLINE, "deadline: client channel close");
        client.wait(Duration::from_millis(20));
    }
}

fn server_recv_channel(server: &mut QuicServer, conn: ConnectionId) -> (Channel, Vec<u8>) {
    let start = std::time::Instant::now();
    loop {
        match server.try_recv_channel(conn) {
            Ok(Some(pair)) => return pair,
            Ok(None) => {}
            Err(error) => panic!("server closed: {error:?}"),
        }
        assert!(start.elapsed() < DEADLINE, "deadline: server channel frame");
        server.wait(Duration::from_millis(20));
    }
}

fn server_err_channel(server: &mut QuicServer, conn: ConnectionId) -> ServerError {
    let start = std::time::Instant::now();
    loop {
        match server.try_recv_channel(conn) {
            Err(error) => return error,
            Ok(Some(_)) => continue,
            Ok(None) => {}
        }
        assert!(start.elapsed() < DEADLINE, "deadline: server channel close");
        server.wait(Duration::from_millis(20));
    }
}

/// Connects with an explicit framing and accepts the hello on `server`.
fn connect_framed_accepted(
    server: &mut QuicServer,
    limits: crpg_net_quic::ClientLimits,
    wire_version: u8,
    framing: Framing,
) -> (QuicClient, ConnectionId) {
    let pending = connect_framed_async(
        client_config(limits),
        server.local_addr(),
        hello(wire_version),
        framing,
    );
    let (conn, _, _) = next_hello_any(server);
    assert_eq!(server.framing(conn), Ok(framing));
    server
        .accept(
            conn,
            Welcome {
                wire_version,
                epoch: epoch(1),
            },
        )
        .expect("accept");
    let client = pending.join().expect("client thread").expect("connect");
    (client, conn)
}

fn connect_resume_accepted(
    server: &mut QuicServer,
    limits: crpg_net_quic::ClientLimits,
    request: &[u8],
) -> (QuicClient, ConnectionId) {
    let pending = connect_resume_async(
        client_config(limits),
        server.local_addr(),
        SESSION,
        request.to_vec(),
    );
    let received = next_resume(server);
    assert_eq!(server.framing(received.conn), Ok(Framing::V2));
    let answer = l4_accepted();
    server
        .accept_resume(received.conn, &answer)
        .expect("accept_resume");
    let (client, got) = pending
        .join()
        .expect("client thread")
        .expect("connect_resume");
    assert_eq!(got, answer);
    (client, received.conn)
}

/// A connection of the given framing that has been accepted, closed by the
/// server and whose `Closed` was polled: the record is forgotten.
fn forgotten(server: &mut QuicServer, framing: Framing) -> ConnectionId {
    let (client, conn) =
        connect_framed_accepted(server, crpg_net_quic::ClientLimits::v1(), 1, framing);
    server
        .close(conn, CloseCode::Normal, CloseMode::Discard)
        .expect("close");
    loop {
        let (closed, _) = next_closed(server);
        if closed == conn {
            break;
        }
    }
    drop(client);
    conn
}

fn l1_request() -> Vec<u8> {
    let request = ResumeRequest {
        epoch: [0; 16],
        fence: ConnectionFence::FIRST,
        proof: ResumeProof::new(vec![0xAA; 16]).expect("proof"),
        claims: vec![LaneClaim {
            lane: 0,
            receipted_through: 0,
        }],
    };
    let bytes = codec_resume::encode_request(&request).expect("encode request");
    assert_eq!(bytes.len(), 40);
    bytes
}

fn l4_accepted() -> Vec<u8> {
    let accepted = ResumeAccepted {
        epoch: [0; 16],
        granted: ConnectionFence::new(2).expect("fence"),
        progress: vec![LaneProgress {
            lane: 0,
            finalized_through: 0,
        }],
    };
    let bytes = codec_resume::encode_down(&ResumeDown::Answer(ResumeResult::Accepted(accepted)))
        .expect("encode accepted");
    assert_eq!(bytes.len(), 23);
    bytes
}

fn l3_ticket() -> Vec<u8> {
    let ticket = ResumeTicket {
        epoch: [0; 16],
        proof: ResumeProof::new(vec![0xBB; 16]).expect("proof"),
    };
    let bytes = codec_resume::encode_down(&ResumeDown::Ticket(ticket)).expect("encode ticket");
    assert_eq!(bytes.len(), 36);
    bytes
}

fn rejected(reason: ResumeRejection) -> Vec<u8> {
    let bytes = codec_resume::encode_down(&ResumeDown::Answer(ResumeResult::Rejected(
        ResumeRejected {
            epoch: [0; 16],
            reason,
        },
    )))
    .expect("encode rejected");
    assert_eq!(bytes.len(), 20);
    bytes
}

fn l6_denied() -> Vec<u8> {
    rejected(ResumeRejection::Denied)
}

fn l7_busy() -> Vec<u8> {
    rejected(ResumeRejection::Busy)
}

/// Raw read of exactly `n` bytes under the 30 s guard.
fn raw_read_exact(rt: &RawRuntime, recv: &mut quinn::RecvStream, n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    rt.run(async {
        tokio::time::timeout(DEADLINE, recv.read_exact(&mut buf))
            .await
            .expect("deadline: raw read")
            .expect("raw read");
    });
    buf
}

fn hello_frame_of(hello: &Hello) -> Vec<u8> {
    frame_bytes(&hello.encode().expect("hello encode"))
}

// ---------------------------------------------------------------------------
// A raw quinn server (cases 20–21): the bars this crate's server never writes.
// ---------------------------------------------------------------------------

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

/// A raw quinn client offering an ordered ALPN list (the rig's client offers
/// exactly one).
fn raw_connect_alpns(
    server: std::net::SocketAddr,
    alpns: &[&[u8]],
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
    tls.alpn_protocols = alpns.iter().map(|alpn| alpn.to_vec()).collect();
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls).expect("quic tls");
    let config = quinn::ClientConfig::new(Arc::new(crypto));
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

/// A raw quinn server on `loopback()` with the fixture cert, TLS 1.3/ring
/// and `alpns` in order.
struct RawServer {
    rt: RawRuntime,
    endpoint: quinn::Endpoint,
}

fn raw_server(alpns: &[&[u8]]) -> RawServer {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cert = CertificateDer::from(SERVER_A_CERT.to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(SERVER_A_KEY.to_vec()));
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("tls13")
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .expect("raw server identity");
    tls.alpn_protocols = alpns.iter().map(|alpn| alpn.to_vec()).collect();
    let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls).expect("quic tls");
    let config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    let rt = RawRuntime::new();
    let endpoint = rt.run(async move {
        quinn::Endpoint::server(config, loopback()).expect("raw server endpoint")
    });
    RawServer { rt, endpoint }
}

impl RawServer {
    fn accept_lane(&self) -> (quinn::Connection, quinn::SendStream, quinn::RecvStream) {
        self.rt.run(async {
            let incoming = self.endpoint.accept().await.expect("raw accept");
            let connection = incoming.await.expect("raw incoming");
            let (send, recv) = connection.accept_bi().await.expect("raw bi");
            (connection, send, recv)
        })
    }

    fn read_exact(&self, recv: &mut quinn::RecvStream, n: usize) -> Vec<u8> {
        raw_read_exact(&self.rt, recv, n)
    }

    fn write(&self, send: &mut quinn::SendStream, bytes: &[u8]) {
        self.rt
            .run(async { send.write_all(bytes).await })
            .expect("raw write");
    }

    fn finish(&self, send: &mut quinn::SendStream) {
        send.finish().expect("finish");
    }

    fn close(&self, conn: &quinn::Connection, code: u32) {
        conn.close(quinn::VarInt::from_u32(code), b"raw");
        let endpoint = self.endpoint.clone();
        self.rt.run(async move {
            let _ = tokio::time::timeout(Duration::from_secs(2), endpoint.wait_idle()).await;
        });
    }

    fn closed_code(&self, conn: &quinn::Connection) -> Option<u64> {
        let connection = conn.clone();
        let error = self.rt.run(async move {
            tokio::time::timeout(DEADLINE, connection.closed())
                .await
                .expect("deadline: raw close")
        });
        match error {
            quinn::ConnectionError::ApplicationClosed(close) => Some(close.error_code.into_inner()),
            _ => None,
        }
    }
}

/// Reads a `/1` hello frame and writes a `/1` welcome with its version.
fn accept_raw_hello(raw: &RawServer, recv: &mut quinn::RecvStream, send: &mut quinn::SendStream) {
    let header = raw.read_exact(recv, 4);
    let len = usize::try_from(u32::from_be_bytes([
        header[0], header[1], header[2], header[3],
    ]))
    .expect("hello length");
    let body = raw.read_exact(recv, len);
    let hello = Hello::decode(&body).expect("raw hello");
    let welcome = Welcome {
        wire_version: hello.wire_version,
        epoch: epoch(7),
    };
    raw.write(send, &frame_bytes(&welcome.encode().expect("welcome")));
}

// ---------------------------------------------------------------------------
// Case 1: wire constants and literal header bytes (no I/O).
// ---------------------------------------------------------------------------

#[test]
fn channel_wire_constants_and_header_bytes() {
    assert_eq!(ALPN_LANE0_V2, b"crpg-lane0/2");
    assert_eq!(ALPN_LANE0_V1, b"crpg-lane0/1");
    assert_eq!(Framing::V1.alpn(), ALPN_LANE0_V1);
    assert_eq!(Framing::V2.alpn(), ALPN_LANE0_V2);
    assert_eq!(Framing::from_alpn(ALPN_LANE0_V1), Some(Framing::V1));
    assert_eq!(Framing::from_alpn(ALPN_LANE0_V2), Some(Framing::V2));
    assert_eq!(Framing::from_alpn(b"crpg-lane0/3"), None);
    assert_eq!(Framing::from_alpn(b""), None);

    assert_eq!(Channel::Lane0.as_u8(), 0x00);
    assert_eq!(
        Channel::Transfer.as_u8(),
        crpg_net::protocol_transfer::TRANSFER_VERSION
    );
    assert_eq!(Channel::Transfer.as_u8(), 0x81);
    assert_eq!(
        Channel::Resume.as_u8(),
        crpg_net::protocol_resume::RESUME_VERSION
    );
    assert_eq!(Channel::Resume.as_u8(), RESUME_VERSION);

    for raw in 0..=255u8 {
        let known = raw == 0x00 || raw == 0x81 || raw == 0x91;
        assert_eq!(
            Channel::from_u8(raw).is_some(),
            known,
            "from_u8({raw:#04x})"
        );
        assert_eq!(
            parse_channel_frame_header([raw, 0, 0, 1], INTENT_CAP).is_ok(),
            known,
            "parse({raw:#04x})"
        );
    }

    // S§5.5 literal vectors.
    assert_eq!(
        channel_frame_header(Channel::Lane0, 3, INTENT_CAP),
        Ok([0x00, 0x00, 0x00, 0x03])
    );
    assert_eq!(
        channel_frame_header(Channel::Lane0, 65_536, DELTA_CAP),
        Ok([0x00, 0x01, 0x00, 0x00])
    );
    assert_eq!(
        channel_frame_header(Channel::Transfer, 30, INTENT_CAP),
        Ok([0x81, 0x00, 0x00, 0x1E])
    );
    assert_eq!(
        channel_frame_header(Channel::Transfer, 32_815, DELTA_CAP),
        Ok([0x81, 0x00, 0x80, 0x2F])
    );
    assert_eq!(
        channel_frame_header(Channel::Transfer, 65_536, DELTA_CAP),
        Ok([0x81, 0x01, 0x00, 0x00])
    );
    assert_eq!(
        channel_frame_header(Channel::Resume, 40, INTENT_CAP),
        Ok([0x91, 0x00, 0x00, 0x28])
    );
    assert_eq!(
        channel_frame_header(Channel::Resume, 23, DELTA_CAP),
        Ok([0x91, 0x00, 0x00, 0x17])
    );
    assert_eq!(
        channel_frame_header(Channel::Resume, 4_096, INTENT_CAP),
        Ok([0x91, 0x00, 0x10, 0x00])
    );
    assert_eq!(
        channel_frame_header(Channel::Resume, 4_097, INTENT_CAP),
        Err(FrameError::TooLarge)
    );
    assert_eq!(
        channel_frame_header(Channel::Lane0, 16_777_215, usize::MAX),
        Ok([0x00, 0xFF, 0xFF, 0xFF])
    );
    assert_eq!(
        channel_frame_header(Channel::Lane0, 16_777_216, usize::MAX),
        Err(FrameError::TooLarge)
    );
    assert_eq!(
        channel_frame_header(Channel::Transfer, 0, INTENT_CAP),
        Err(FrameError::Empty)
    );
    assert_eq!(
        channel_frame_header(Channel::Transfer, 4_097, INTENT_CAP),
        Err(FrameError::TooLarge)
    );
    assert_eq!(
        channel_frame_header(Channel::Resume, 0, INTENT_CAP),
        Err(FrameError::Empty)
    );

    assert_eq!(
        parse_channel_frame_header([0x81, 0x00, 0x00, 0x1E], INTENT_CAP),
        Ok((Channel::Transfer, 30))
    );
    assert_eq!(
        parse_channel_frame_header([0x00, 0x00, 0x00, 0x03], INTENT_CAP),
        Ok((Channel::Lane0, 3))
    );
    assert_eq!(
        parse_channel_frame_header([0x91, 0x00, 0x00, 0x28], INTENT_CAP),
        Ok((Channel::Resume, 40))
    );
    assert_eq!(
        parse_channel_frame_header([0x81, 0x01, 0x00, 0x00], DELTA_CAP),
        Ok((Channel::Transfer, 65_536))
    );
    assert_eq!(
        parse_channel_frame_header([0x81, 0x01, 0x00, 0x01], DELTA_CAP),
        Err(FrameError::TooLarge)
    );
    assert_eq!(
        parse_channel_frame_header([0x91, 0x01, 0x00, 0x00], DELTA_CAP),
        Ok((Channel::Resume, 65_536))
    );
    assert_eq!(
        parse_channel_frame_header([0x91, 0x00, 0x10, 0x01], INTENT_CAP),
        Err(FrameError::TooLarge)
    );
    for raw in [0x01u8, 0x02, 0x80, 0x82, 0x90, 0x92, 0xFF] {
        assert_eq!(
            parse_channel_frame_header([raw, 0, 0, 1], INTENT_CAP),
            Err(FrameError::UnknownChannel)
        );
    }
    assert_eq!(
        parse_channel_frame_header([0x05, 0, 0, 0], INTENT_CAP),
        Err(FrameError::UnknownChannel)
    );
    assert_eq!(
        parse_channel_frame_header([0x05, 0xFF, 0xFF, 0xFF], INTENT_CAP),
        Err(FrameError::UnknownChannel)
    );
    for header in [[0x00, 0, 0, 0], [0x81, 0, 0, 0], [0x91, 0, 0, 0]] {
        assert_eq!(
            parse_channel_frame_header(header, INTENT_CAP),
            Err(FrameError::Empty)
        );
    }
    assert_eq!(
        parse_channel_frame_header([0x00, 0x00, 0x10, 0x01], INTENT_CAP),
        Err(FrameError::TooLarge)
    );
    assert_eq!(
        parse_channel_frame_header([0x81, 0x00, 0x10, 0x01], INTENT_CAP),
        Err(FrameError::TooLarge)
    );

    // Lane-0 frames are the `/1` header for every valid length.
    for n in [1usize, 2, 255, 256, 4_095, 4_096] {
        assert_eq!(
            channel_frame_header(Channel::Lane0, n, INTENT_CAP),
            frame_header(n, INTENT_CAP)
        );
    }
    for n in [1usize, 2, 255, 256, 65_535, 65_536] {
        assert_eq!(
            channel_frame_header(Channel::Lane0, n, DELTA_CAP),
            frame_header(n, DELTA_CAP)
        );
    }

    assert_eq!(
        FrameError::UnknownChannel.to_string(),
        "UnknownChannel at quic/frame"
    );
    assert_eq!(FrameError::Empty.to_string(), "Empty at quic/frame");
    assert_eq!(FrameError::TooLarge.to_string(), "TooLarge at quic/frame");
    assert_eq!(
        ChannelSendError::ChannelUnavailable.to_string(),
        "ChannelUnavailable at quic/channel"
    );
    assert_eq!(
        ChannelSendError::Send(SendError::QueueFull).to_string(),
        "Send at quic/channel"
    );
    assert_eq!(MAX_HELLO_BYTES, 130);
    assert_eq!(MAX_RESUME_FRAME_BYTES, INTENT_CAP);
}

// ---------------------------------------------------------------------------
// Case 2: `/2` round trip on both channels and both directions.
// ---------------------------------------------------------------------------

#[test]
fn v2_round_trip_both_channels_both_directions() {
    let mut server = QuicServer::bind_dual(server_config(ServerLimits::v1())).expect("bind_dual");
    let (mut client, conn) = connect_framed_accepted(&mut server, client_limits(), 1, Framing::V2);
    assert_eq!(server.framing(conn), Ok(Framing::V2));
    assert_eq!(client.framing(), Framing::V2);
    assert_eq!(
        client.welcome(),
        Welcome {
            wire_version: 1,
            epoch: epoch(1),
        }
    );

    for (channel, size, cap) in [
        (Channel::Lane0, 1, INTENT_CAP),
        (Channel::Transfer, 1, INTENT_CAP),
        (Channel::Lane0, INTENT_CAP, INTENT_CAP),
        (Channel::Transfer, INTENT_CAP, INTENT_CAP),
    ] {
        let frame = pattern(3, size as u32, size);
        client
            .try_send_channel(channel, &frame)
            .unwrap_or_else(|error| panic!("client send {channel:?} {size}: {error:?}"));
        assert_eq!(server_recv_channel(&mut server, conn), (channel, frame));
        assert!(size <= cap);
    }
    for (channel, size) in [
        (Channel::Lane0, 1),
        (Channel::Transfer, 1),
        (Channel::Resume, 1),
        (Channel::Lane0, DELTA_CAP),
        (Channel::Transfer, DELTA_CAP),
        (Channel::Resume, DELTA_CAP),
    ] {
        let frame = pattern(4, size as u32, size);
        server
            .try_send_channel(conn, channel, &frame)
            .unwrap_or_else(|error| panic!("server send {channel:?} {size}: {error:?}"));
        assert_eq!(client_recv_channel(&mut client), (channel, frame));
    }

    // A 32,815-byte transfer frame built by the transfer codec.
    let transfer = crpg_net::codec_transfer::encode_down(&TransferDown::Chunk(TransferChunk {
        epoch: [0; 16],
        transfer: TransferId::new(u64::MAX).expect("id"),
        index: 0,
        bytes: vec![0x2A; 32_768],
    }))
    .expect("encode chunk");
    assert_eq!(transfer.len(), 32_815);
    server
        .try_send_channel(conn, Channel::Transfer, &transfer)
        .expect("server transfer");
    assert_eq!(
        client_recv_channel(&mut client),
        (Channel::Transfer, transfer)
    );
}

fn server_config(limits: ServerLimits) -> ServerConfig {
    ServerConfig {
        bind: loopback(),
        identity: identity_a(),
        limits,
    }
}

fn client_limits() -> crpg_net_quic::ClientLimits {
    crpg_net_quic::ClientLimits::v1()
}

// ---------------------------------------------------------------------------
// Case 3: `/2` wire bytes are exact; `/1` is unchanged.
// ---------------------------------------------------------------------------

#[test]
fn v2_wire_bytes_are_exact() {
    let mut server = QuicServer::bind_dual(server_config(ServerLimits::v1())).expect("bind_dual");
    let raw = raw_connect(server.local_addr(), ALPN_LANE0_V2).expect("raw /2");
    let (send, mut recv) = raw.open_and_write(&hello_frame_of(&hello(1)));
    let (conn, _, _) = next_hello_any(&mut server);
    server
        .accept(
            conn,
            Welcome {
                wire_version: 1,
                epoch: epoch(1),
            },
        )
        .expect("accept");
    raw.read_welcome(&mut recv);
    server
        .try_send_channel(conn, Channel::Transfer, b"xyz")
        .expect("transfer");
    server
        .try_send_channel(conn, Channel::Lane0, b"ab")
        .expect("lane0");
    server
        .try_send_channel(conn, Channel::Resume, b"t")
        .expect("resume");
    let expected = vec![
        0x81, 0x00, 0x00, 0x03, 0x78, 0x79, 0x7A, 0x00, 0x00, 0x00, 0x02, 0x61, 0x62, 0x91, 0x00,
        0x00, 0x01, 0x74,
    ];
    assert_eq!(raw_read_exact(&raw.rt, &mut recv, 18), expected);
    drop(send);

    // Control: a `/1` raw client on the same dual server reads the `/1`
    // bytes, byte-identical to a `bind` server.
    let dual_bytes = raw_lane_lane0_reply(&mut server);
    let mut plain = bind_server(ServerLimits::v1());
    let plain_bytes = raw_lane_lane0_reply(&mut plain);
    assert_eq!(dual_bytes, vec![0x00, 0x00, 0x00, 0x02, 0x61, 0x62]);
    assert_eq!(dual_bytes, plain_bytes);
}

fn raw_lane_lane0_reply(server: &mut QuicServer) -> Vec<u8> {
    let raw = raw_lane(server.local_addr());
    let (_send, mut recv) = raw.open_and_write(&hello_frame_of(&hello(1)));
    let (conn, _, _) = next_hello_any(server);
    server
        .accept(
            conn,
            Welcome {
                wire_version: 1,
                epoch: epoch(1),
            },
        )
        .expect("accept");
    raw.read_welcome(&mut recv);
    server.try_send(conn, &[0x61, 0x62]).expect("send");
    raw_read_exact(&raw.rt, &mut recv, 6)
}

// ---------------------------------------------------------------------------
// Case 4: the negotiation matrix never downgrades.
// ---------------------------------------------------------------------------

#[test]
fn negotiation_matrix_never_downgrades() {
    // (a) `connect` → `bind_dual`: V1 on both sides.
    let mut dual = QuicServer::bind_dual(server_config(ServerLimits::v1())).expect("bind_dual");
    let (mut client, conn) = connect_framed_accepted(&mut dual, client_limits(), 1, Framing::V1);
    assert_eq!(client.framing(), Framing::V1);
    assert_eq!(dual.framing(conn), Ok(Framing::V1));
    client.try_send(b"x").expect("lane0");
    assert_eq!(server_recv(&mut dual, conn), Ok(b"x".to_vec()));
    let before = queue_fields(dual.stats(conn).expect("stats"));
    assert_eq!(
        dual.try_send_channel(conn, Channel::Transfer, b"x"),
        Err(ChannelSendError::ChannelUnavailable)
    );
    assert_eq!(queue_fields(dual.stats(conn).expect("stats")), before);
    assert_eq!(
        dual.try_send_channel(conn, Channel::Transfer, b""),
        Err(ChannelSendError::ChannelUnavailable)
    );
    assert_eq!(queue_fields(dual.stats(conn).expect("stats")), before);
    let cbefore = queue_fields(client.stats());
    assert_eq!(
        client.try_send_channel(Channel::Transfer, b"x"),
        Err(ChannelSendError::ChannelUnavailable)
    );
    assert_eq!(queue_fields(client.stats()), cbefore);
    assert_eq!(
        client.try_send_channel(Channel::Transfer, b""),
        Err(ChannelSendError::ChannelUnavailable)
    );
    assert_eq!(queue_fields(client.stats()), cbefore);

    // (b) `connect_framed(V2)` → `bind`: TLS fails, counted, no event.
    let mut plain = bind_server(ServerLimits::v1());
    let refused = plain.totals().refused_handshakes;
    let attempt = connect_framed_async(
        client_config(client_limits()),
        plain.local_addr(),
        hello(1),
        Framing::V2,
    );
    assert!(matches!(
        attempt.join().expect("thread"),
        Err(ConnectError::Handshake)
    ));
    until("refused handshake", || {
        plain.totals().refused_handshakes == refused + 1
    });
    assert!(plain.poll_event().is_none());
    assert!(plain.poll_notice().is_none());

    // (c) `connect_framed(V2)` → `bind_dual`: V2.
    let (client_v2, conn_v2) = connect_framed_accepted(&mut dual, client_limits(), 1, Framing::V2);
    assert_eq!(client_v2.framing(), Framing::V2);
    assert_eq!(dual.framing(conn_v2), Ok(Framing::V2));

    // (d) `connect` → `bind`: V1 (control).
    let (client_v1, conn_v1) = connect_accepted(&mut plain, client_limits(), 1, epoch(1));
    assert_eq!(client_v1.framing(), Framing::V1);
    assert_eq!(plain.framing(conn_v1), Ok(Framing::V1));

    // (e) a foreign ALPN fails and is counted; both `[/2, /1]` gives V2.
    let refused = dual.totals().refused_handshakes;
    assert!(raw_connect(dual.local_addr(), b"crpg-lane0/3").is_err());
    until("foreign ALPN counted", || {
        dual.totals().refused_handshakes == refused + 1
    });
    assert!(dual.poll_notice().is_none());
    let raw =
        raw_connect_alpns(dual.local_addr(), &[ALPN_LANE0_V2, ALPN_LANE0_V1]).expect("both ALPNs");
    let (_send, mut recv) = raw.open_and_write(&hello_frame_of(&hello(1)));
    let (conn, _, _) = next_hello_any(&mut dual);
    assert_eq!(dual.framing(conn), Ok(Framing::V2));
    dual.accept(
        conn,
        Welcome {
            wire_version: 1,
            epoch: epoch(1),
        },
    )
    .expect("accept");
    raw.read_welcome(&mut recv);
}

// ---------------------------------------------------------------------------
// Case 5: the `/2` server-reader close map.
// ---------------------------------------------------------------------------

#[test]
fn v2_reader_close_codes() {
    let mut server = QuicServer::bind_dual(server_config(ServerLimits::v1())).expect("bind_dual");

    let probes: [(&[u8], u64); 13] = [
        (&[0x01, 0x00, 0x00, 0x01], 2),
        (&[0x02, 0x00, 0x00, 0x01], 2),
        (&[0x80, 0x00, 0x00, 0x01], 2),
        (&[0x82, 0x00, 0x00, 0x01], 2),
        (&[0x90, 0x00, 0x00, 0x01], 2),
        (&[0x92, 0x00, 0x00, 0x01], 2),
        (&[0xFF, 0x00, 0x00, 0x01], 2),
        (&[0x05, 0xFF, 0xFF, 0xFF], 2),
        (&[0x91, 0x00, 0x00, 0x01], 2),
        (&[0x91, 0x00, 0x10, 0x01], 2),
        (&[0x81, 0x00, 0x00, 0x00], 2),
        (&[0x81, 0x00, 0x10, 0x01], 3),
        (&[0x00, 0x00, 0x10, 0x01], 3),
    ];
    for (probe, code) in probes {
        let raw = raw_connect(server.local_addr(), ALPN_LANE0_V2).expect("raw /2");
        let (mut send, mut recv) = raw.open_and_write(&hello_frame_of(&hello(1)));
        let (conn, _, _) = next_hello_any(&mut server);
        server
            .accept(
                conn,
                Welcome {
                    wire_version: 1,
                    epoch: epoch(1),
                },
            )
            .expect("accept");
        raw.read_welcome(&mut recv);
        raw.write(&mut send, &[0x00, 0x00, 0x00, 0x02, 0x61, 0x62]);
        raw.write(&mut send, &[0x81, 0x00, 0x00, 0x03, 0x78, 0x79, 0x7A]);
        assert_eq!(
            server_recv_channel(&mut server, conn),
            (Channel::Lane0, b"ab".to_vec())
        );
        assert_eq!(
            server_recv_channel(&mut server, conn),
            (Channel::Transfer, b"xyz".to_vec())
        );
        raw.write(&mut send, probe);
        assert_eq!(
            server_err_channel(&mut server, conn),
            ServerError::Closed(CloseReason::Local(CloseCode::from_u64(code).expect("code")))
        );
        assert_eq!(raw.closed_code(), Some(code));
    }

    // A body cut short on a transfer frame.
    {
        let raw = raw_connect(server.local_addr(), ALPN_LANE0_V2).expect("raw /2");
        let (mut send, mut recv) = raw.open_and_write(&hello_frame_of(&hello(1)));
        let (conn, _, _) = next_hello_any(&mut server);
        server
            .accept(
                conn,
                Welcome {
                    wire_version: 1,
                    epoch: epoch(1),
                },
            )
            .expect("accept");
        raw.read_welcome(&mut recv);
        raw.write(&mut send, &[0x81, 0x00, 0x00, 0x05]);
        raw.write(&mut send, &[0x41, 0x42]);
        raw.finish(&mut send);
        assert_eq!(
            server_err_channel(&mut server, conn),
            ServerError::Closed(CloseReason::Local(CloseCode::ProtocolViolation))
        );
        assert_eq!(raw.closed_code(), Some(2));
    }

    // Controls: `/1` on the same server maps `81` and `91` headers to 3.
    for probe in [[0x81u8, 0x00, 0x00, 0x1E], [0x91, 0x00, 0x00, 0x01]] {
        let raw = raw_lane(server.local_addr());
        let (mut send, mut recv) = raw.open_and_write(&hello_frame_of(&hello(1)));
        let (conn, _, _) = next_hello_any(&mut server);
        server
            .accept(
                conn,
                Welcome {
                    wire_version: 1,
                    epoch: epoch(1),
                },
            )
            .expect("accept");
        raw.read_welcome(&mut recv);
        raw.write(&mut send, &probe);
        assert_eq!(
            server_err_channel(&mut server, conn),
            ServerError::Closed(CloseReason::Local(CloseCode::FrameTooLarge))
        );
        assert_eq!(raw.closed_code(), Some(3));
    }

    // A well-behaved `/2` client keeps working throughout.
    let (mut good, good_conn) =
        connect_framed_accepted(&mut server, client_limits(), 1, Framing::V2);
    good.try_send_channel(Channel::Transfer, b"ok")
        .expect("send");
    assert_eq!(
        server_recv_channel(&mut server, good_conn),
        (Channel::Transfer, b"ok".to_vec())
    );
}

// ---------------------------------------------------------------------------
// Case 6: send precedence and caps.
// ---------------------------------------------------------------------------

#[test]
fn v2_send_precedence_and_caps() {
    let mut server = QuicServer::bind_dual(server_config(ServerLimits::v1())).expect("bind_dual");
    let limits = client_limits();

    // Unknown: a forgotten record.
    let f1 = forgotten(&mut server, Framing::V1);
    assert_eq!(server.framing(f1), Err(ServerError::UnknownConnection));
    for (channel, frame) in [
        (Channel::Lane0, b"x".as_slice()),
        (Channel::Transfer, b"x".as_slice()),
        (Channel::Resume, b"".as_slice()),
    ] {
        assert_eq!(
            server.try_send_channel(f1, channel, frame),
            Err(ChannelSendError::Send(SendError::UnknownConnection))
        );
    }

    // Pending: `/1` P1 and `/2` P2.
    let p1_thread = connect_async(client_config(limits), server.local_addr(), hello(1));
    let (p1, _, _) = next_hello_any(&mut server);
    let p2_thread = connect_framed_async(
        client_config(limits),
        server.local_addr(),
        hello(1),
        Framing::V2,
    );
    let (p2, _, _) = next_hello_any(&mut server);
    let big = vec![0u8; 65_537];
    let p1_stats = queue_fields(server.stats(p1).expect("stats p1"));
    let p2_stats = queue_fields(server.stats(p2).expect("stats p2"));
    for (channel, frame) in [
        (Channel::Transfer, b"x".as_slice()),
        (Channel::Resume, b"".as_slice()),
        (Channel::Transfer, big.as_slice()),
    ] {
        assert_eq!(
            server.try_send_channel(p1, channel, frame),
            Err(ChannelSendError::Send(SendError::NotAccepted))
        );
        assert_eq!(queue_fields(server.stats(p1).expect("stats p1")), p1_stats);
    }
    assert_eq!(
        server.try_send_channel(p2, Channel::Lane0, b"x"),
        Err(ChannelSendError::Send(SendError::NotAccepted))
    );
    assert_eq!(queue_fields(server.stats(p2).expect("stats p2")), p2_stats);
    server.refuse(p1, CloseCode::Normal).expect("refuse p1");
    server.refuse(p2, CloseCode::Normal).expect("refuse p2");
    let _ = next_closed(&mut server);
    let _ = next_closed(&mut server);
    drop(p1_thread.join().expect("p1 thread"));
    drop(p2_thread.join().expect("p2 thread"));

    // Closed: accepted `/1` C1 and `/2` C2 before their `Closed` is polled.
    let (c1_client, c1) = connect_framed_accepted(&mut server, limits, 1, Framing::V1);
    let (c2_client, c2) = connect_framed_accepted(&mut server, limits, 1, Framing::V2);
    server
        .close(c1, CloseCode::Normal, CloseMode::Discard)
        .expect("close c1");
    server
        .close(c2, CloseCode::Normal, CloseMode::Discard)
        .expect("close c2");
    let closed_reason = CloseReason::Local(CloseCode::Normal);
    let c1_stats = queue_fields(server.stats(c1).expect("stats c1"));
    let c2_stats = queue_fields(server.stats(c2).expect("stats c2"));
    for (channel, frame) in [
        (Channel::Transfer, b"x".as_slice()),
        (Channel::Resume, b"x".as_slice()),
        (Channel::Transfer, b"".as_slice()),
    ] {
        assert_eq!(
            server.try_send_channel(c1, channel, frame),
            Err(ChannelSendError::Send(SendError::Closed(closed_reason)))
        );
        assert_eq!(queue_fields(server.stats(c1).expect("stats c1")), c1_stats);
    }
    assert_eq!(
        server.try_send_channel(c2, Channel::Lane0, b"x"),
        Err(ChannelSendError::Send(SendError::Closed(closed_reason)))
    );
    assert_eq!(queue_fields(server.stats(c2).expect("stats c2")), c2_stats);
    let _ = next_closed(&mut server);
    let _ = next_closed(&mut server);
    drop(c1_client);
    drop(c2_client);

    // Channel before size, on an accepted `/1` connection.
    let (mut c3_client, c3) = connect_framed_accepted(&mut server, limits, 1, Framing::V1);
    let c3_before = queue_fields(server.stats(c3).expect("stats"));
    let big = vec![0u8; 65_537];
    for channel in [Channel::Transfer, Channel::Resume] {
        for frame in [b"".as_slice(), big.as_slice()] {
            assert_eq!(
                server.try_send_channel(c3, channel, frame),
                Err(ChannelSendError::ChannelUnavailable)
            );
            assert_eq!(queue_fields(server.stats(c3).expect("stats")), c3_before);
        }
    }
    server
        .try_send_channel(c3, Channel::Lane0, b"x")
        .expect("control");
    c3_client.try_send(b"i").expect("client lane0");
    assert_eq!(server_recv(&mut server, c3), Ok(b"i".to_vec()));

    // Size on `/2`.
    let (mut c4_client, c4) = connect_framed_accepted(&mut server, limits, 1, Framing::V2);
    let empty_before = queue_fields(server.stats(c4).expect("stats"));
    for channel in [Channel::Lane0, Channel::Transfer, Channel::Resume] {
        assert_eq!(
            server.try_send_channel(c4, channel, b""),
            Err(ChannelSendError::Send(SendError::EmptyFrame))
        );
        assert_eq!(queue_fields(server.stats(c4).expect("stats")), empty_before);
    }
    let max = vec![0u8; 65_536];
    let too_big = vec![0u8; 65_537];
    for channel in [Channel::Lane0, Channel::Transfer, Channel::Resume] {
        assert_eq!(
            server.try_send_channel(c4, channel, &too_big),
            Err(ChannelSendError::Send(SendError::FrameTooLarge))
        );
        server
            .try_send_channel(c4, channel, &max)
            .unwrap_or_else(|error| panic!("{channel:?} max: {error:?}"));
    }
    for channel in [Channel::Lane0, Channel::Transfer, Channel::Resume] {
        assert_eq!(client_recv_channel(&mut c4_client), (channel, max.clone()));
    }

    // Client, open.
    let (mut client_v2, _) = connect_framed_accepted(&mut server, limits, 1, Framing::V2);
    for channel in [Channel::Lane0, Channel::Transfer] {
        let before = queue_fields(client_v2.stats());
        assert_eq!(
            client_v2.try_send_channel(channel, b""),
            Err(ChannelSendError::Send(SendError::EmptyFrame))
        );
        assert_eq!(queue_fields(client_v2.stats()), before);
        let too_big = vec![0u8; 4_097];
        assert_eq!(
            client_v2.try_send_channel(channel, &too_big),
            Err(ChannelSendError::Send(SendError::FrameTooLarge))
        );
        assert_eq!(queue_fields(client_v2.stats()), before);
        client_v2
            .try_send_channel(channel, &vec![0u8; 4_096])
            .expect("max");
    }
    let before = queue_fields(client_v2.stats());
    let resume_too_big = vec![0u8; 4_097];
    for frame in [b"".as_slice(), resume_too_big.as_slice()] {
        assert_eq!(
            client_v2.try_send_channel(Channel::Resume, frame),
            Err(ChannelSendError::ChannelUnavailable)
        );
        assert_eq!(queue_fields(client_v2.stats()), before);
    }
    let (mut client_v1, _) = connect_framed_accepted(&mut server, limits, 1, Framing::V1);
    let before = queue_fields(client_v1.stats());
    assert_eq!(
        client_v1.try_send_channel(Channel::Transfer, &vec![0u8; 4_097]),
        Err(ChannelSendError::ChannelUnavailable)
    );
    assert_eq!(queue_fields(client_v1.stats()), before);

    // Client, closed.
    let (mut closed_v2, closed_v2_conn) =
        connect_framed_accepted(&mut server, limits, 1, Framing::V2);
    let (mut closed_v1, closed_v1_conn) =
        connect_framed_accepted(&mut server, limits, 1, Framing::V1);
    server
        .close(closed_v2_conn, CloseCode::Normal, CloseMode::Flush)
        .expect("flush close v2");
    server
        .close(closed_v1_conn, CloseCode::Normal, CloseMode::Flush)
        .expect("flush close v1");
    until("client v2 closed", || closed_v2.close_reason().is_some());
    until("client v1 closed", || closed_v1.close_reason().is_some());
    let peer_zero = CloseReason::Peer { code: 0 };
    for frame in [b"x".as_slice(), b"".as_slice()] {
        let channel = if frame.is_empty() {
            Channel::Transfer
        } else {
            Channel::Resume
        };
        assert_eq!(
            closed_v2.try_send_channel(channel, frame),
            Err(ChannelSendError::Send(SendError::Closed(peer_zero)))
        );
    }
    assert_eq!(
        closed_v1.try_send_channel(Channel::Transfer, b"x"),
        Err(ChannelSendError::Send(SendError::Closed(peer_zero)))
    );
}

// ---------------------------------------------------------------------------
// Case 7: one queue per direction across all channels.
// ---------------------------------------------------------------------------

#[test]
fn v2_queues_shared_across_channels() {
    // Outbound: a non-reading client stalls the server queue at its cap.
    let mut slow_reader = client_limits();
    slow_reader.inbound_frames = 1;
    slow_reader.inbound_bytes = DELTA_CAP;
    let limits = ServerLimits::v1();
    let frames_cap = limits.outbound.per_peer_frames;
    let bytes_cap = limits.outbound.per_peer_bytes;
    let mut server = QuicServer::bind_dual(server_config(limits)).expect("bind_dual");
    let (mut client, conn) = connect_framed_accepted(&mut server, slow_reader, 1, Framing::V2);
    let mut next_id = 1u32;
    let cap = (frames_cap.min(bytes_cap / DELTA_CAP), bytes_cap);
    let accepted = {
        let server_cell = std::cell::RefCell::new(&mut server);
        fill_until_stable_queue_full(
            |frame| match server_cell
                .borrow_mut()
                .try_send_channel(conn, Channel::Transfer, frame)
            {
                Err(ChannelSendError::Send(error)) => Err(error),
                Err(other) => panic!("send: {other:?}"),
                Ok(()) => Ok(()),
            },
            || {
                let stats = server_cell.borrow().stats(conn).expect("stats");
                (stats.outbound_frames, stats.outbound_bytes)
            },
            (cap.0, cap.0 * DELTA_CAP),
            DELTA_CAP,
            &mut next_id,
        )
    };
    let full = queue_fields(server.stats(conn).expect("stats"));
    for (channel, frame) in [
        (Channel::Lane0, b"x".to_vec()),
        (Channel::Resume, b"t".to_vec()),
        (Channel::Transfer, vec![0u8; DELTA_CAP]),
    ] {
        assert_eq!(
            server.try_send_channel(conn, channel, &frame),
            Err(ChannelSendError::Send(SendError::QueueFull))
        );
        assert_eq!(queue_fields(server.stats(conn).expect("stats")), full);
    }

    // Drain: the accepted transfer frames come first, then one frame on each
    // of the three channels arrives in send order.
    let mut received = Vec::new();
    for _ in 0..accepted.len() {
        received.push(client_recv_channel(&mut client));
    }
    let extra = [
        (Channel::Lane0, pattern(10, 1, 7)),
        (Channel::Transfer, pattern(10, 2, 9)),
        (Channel::Resume, pattern(10, 3, 11)),
    ];
    for (channel, frame) in &extra {
        server
            .try_send_channel(conn, *channel, frame)
            .unwrap_or_else(|error| panic!("room after drain {channel:?}: {error:?}"));
    }
    let expected: Vec<(Channel, Vec<u8>)> = accepted
        .iter()
        .map(|id| (Channel::Transfer, pattern(9, *id, DELTA_CAP)))
        .chain(extra.iter().cloned())
        .collect();
    for _ in &expected[accepted.len()..] {
        received.push(client_recv_channel(&mut client));
    }
    assert_eq!(received, expected);

    // Inbound: one per-connection queue across channels, parked at two.
    let mut limits = ServerLimits::v1();
    limits.inbound.per_peer_frames = 2;
    let mut server = QuicServer::bind_dual(server_config(limits)).expect("bind_dual");
    let (mut client, conn) = connect_framed_accepted(&mut server, client_limits(), 1, Framing::V2);
    let frames = [
        (Channel::Transfer, b"t1".to_vec()),
        (Channel::Transfer, b"t2".to_vec()),
        (Channel::Lane0, b"l1".to_vec()),
        (Channel::Lane0, b"l2".to_vec()),
    ];
    for (channel, frame) in &frames {
        client
            .try_send_channel(*channel, frame)
            .expect("client send");
    }
    until("two queued + parked", || {
        let stats = server.stats(conn).expect("stats");
        stats.inbound_frames == 2 && stats.reader_parked
    });
    let mut received = Vec::new();
    for _ in 0..frames.len() {
        received.push(server_recv_channel(&mut server, conn));
    }
    assert_eq!(received, frames.to_vec());
}

/// Copied from `tests/quic.rs:1328-1367` (S§10 case 7).
fn fill_until_stable_queue_full(
    mut send: impl FnMut(&[u8]) -> Result<(), SendError>,
    mut stats: impl FnMut() -> (usize, usize),
    cap: (usize, usize),
    size: usize,
    next_id: &mut u32,
) -> Vec<u32> {
    let mut accepted = Vec::new();
    let start = std::time::Instant::now();
    loop {
        assert!(start.elapsed() < DEADLINE, "deadline: QueueFull");
        let id = *next_id;
        *next_id += 1;
        match send(&pattern(9, id, size)) {
            Ok(()) => {
                accepted.push(id);
                continue;
            }
            Err(SendError::QueueFull) => {}
            Err(error) => panic!("send: {error:?}"),
        }
        let before = stats();
        let id = *next_id;
        *next_id += 1;
        match send(&pattern(9, id, size)) {
            Ok(()) => {
                accepted.push(id);
                continue;
            }
            Err(SendError::QueueFull) => {}
            Err(error) => panic!("send: {error:?}"),
        }
        let after = stats();
        if before == after && after == cap {
            return accepted;
        }
    }
}

// ---------------------------------------------------------------------------
// Case 8: interleaved channels keep one total order.
// ---------------------------------------------------------------------------

fn channel_rank(channel: Channel) -> u8 {
    match channel {
        Channel::Lane0 => 0,
        Channel::Transfer => 1,
        Channel::Resume => 2,
    }
}

type ChannelList = Vec<(Channel, Vec<u8>)>;

fn interleaved_run(
    server_addr: std::net::SocketAddr,
    relay: Option<&UdpRelay>,
    server: &mut QuicServer,
) -> (ChannelList, ChannelList) {
    let pending = connect_framed_async(
        client_config(client_limits()),
        server_addr,
        hello(1),
        Framing::V2,
    );
    let (conn, _, _) = next_hello_any(server);
    server
        .accept(
            conn,
            Welcome {
                wire_version: 1,
                epoch: epoch(1),
            },
        )
        .expect("accept");
    let mut client = pending.join().expect("thread").expect("connect");
    if let Some(relay) = relay {
        relay.set_impaired(true);
    }

    let mut down_expected = Vec::new();
    let mut down_received = Vec::new();
    for i in 0..200u32 {
        let channel = match i % 3 {
            0 => Channel::Transfer,
            1 => Channel::Resume,
            _ => Channel::Lane0,
        };
        let frame = pattern(1, i, cycling_size(i, DELTA_CAP));
        down_expected.push((channel, frame.clone()));
        loop {
            match server.try_send_channel(conn, channel, &frame) {
                Ok(()) => break,
                Err(ChannelSendError::Send(SendError::QueueFull)) => {
                    if let Ok(Some(pair)) = client.try_recv_channel() {
                        down_received.push(pair);
                    } else {
                        client.wait(Duration::from_millis(20));
                    }
                }
                Err(error) => panic!("server send: {error:?}"),
            }
        }
    }
    while down_received.len() < down_expected.len() {
        down_received.push(client_recv_channel(&mut client));
    }

    let mut up_expected = Vec::new();
    let mut up_received = Vec::new();
    for i in 0..200u32 {
        let channel = if i % 2 == 0 {
            Channel::Transfer
        } else {
            Channel::Lane0
        };
        let frame = pattern(2, i, cycling_size(i, INTENT_CAP));
        up_expected.push((channel, frame.clone()));
        loop {
            match client.try_send_channel(channel, &frame) {
                Ok(()) => break,
                Err(ChannelSendError::Send(SendError::QueueFull)) => {
                    if let Ok(Some(pair)) = server.try_recv_channel(conn) {
                        up_received.push(pair);
                    } else {
                        server.wait(Duration::from_millis(20));
                    }
                }
                Err(error) => panic!("client send: {error:?}"),
            }
        }
    }
    while up_received.len() < up_expected.len() {
        up_received.push(server_recv_channel(server, conn));
    }
    assert_eq!(down_received, down_expected, "downstream order");
    assert_eq!(up_received, up_expected, "upstream order");
    (down_received, up_received)
}

#[test]
fn v2_interleaved_channels_keep_one_total_order() {
    let mut server = QuicServer::bind_dual(server_config(ServerLimits::v1())).expect("bind_dual");
    let (down, up) = interleaved_run(server.local_addr(), None, &mut server);
    assert_eq!(down.len(), 200);
    assert_eq!(up.len(), 200);
    let mut partitioned = down.clone();
    partitioned.sort_by_key(|(channel, _)| channel_rank(*channel));
    assert_ne!(partitioned, down, "the stream really interleaves channels");

    // Impaired relay after the handshake.
    let mut server = QuicServer::bind_dual(server_config(ServerLimits::v1())).expect("bind_dual");
    let relay = UdpRelay::start(
        server.local_addr(),
        RelayConfig {
            drop_every: 13,
            reorder_depth: 4,
            seed: 23,
        },
    );
    let (relay_down, relay_up) = interleaved_run(relay.addr(), Some(&relay), &mut server);
    assert!(relay.dropped() >= 1, "relay dropped {}", relay.dropped());
    assert!(
        relay.reordered() >= 1,
        "relay reordered {}",
        relay.reordered()
    );
    assert_eq!(relay_down.len(), 200);
    assert_eq!(relay_up.len(), 200);
}

// ---------------------------------------------------------------------------
// Case 9: the legacy entry points are lane-0 views.
// ---------------------------------------------------------------------------

#[test]
fn v2_legacy_entry_points_are_lane0_views() {
    let mut server = QuicServer::bind_dual(server_config(ServerLimits::v1())).expect("bind_dual");
    let (mut client, conn) = connect_framed_accepted(&mut server, client_limits(), 1, Framing::V2);

    client.try_send(b"x").expect("client send");
    assert_eq!(
        server_recv_channel(&mut server, conn),
        (Channel::Lane0, b"x".to_vec())
    );

    let t = b"T".to_vec();
    let r = b"R".to_vec();
    let l = b"L".to_vec();
    server
        .try_send_channel(conn, Channel::Transfer, &t)
        .expect("transfer");
    server
        .try_send_channel(conn, Channel::Resume, &r)
        .expect("resume");
    server
        .try_send_channel(conn, Channel::Lane0, &l)
        .expect("lane0");
    until("three queued", || client.stats().inbound_frames == 3);
    assert_eq!(client.try_recv(), Ok(None));
    assert_eq!(client.try_recv_channel(), Ok(Some((Channel::Transfer, t))));
    assert_eq!(client.try_recv(), Ok(None));
    assert_eq!(client.try_recv_channel(), Ok(Some((Channel::Resume, r))));
    assert_eq!(client.try_recv(), Ok(Some(l)));

    // Server side, through `lane(conn)`'s `Transport::recv`.
    let t = b"T".to_vec();
    let l = b"L".to_vec();
    client.try_send_channel(Channel::Transfer, &t).expect("t");
    client.try_send_channel(Channel::Lane0, &l).expect("l");
    until("two queued at server", || {
        let stats = server.stats(conn).expect("stats");
        stats.inbound_frames == 2
    });
    {
        let mut lane = server.lane(conn).expect("lane");
        assert_eq!(lane.recv(), None);
    }
    assert_eq!(
        server.try_recv_channel(conn),
        Ok(Some((Channel::Transfer, t)))
    );
    {
        let mut lane = server.lane(conn).expect("lane");
        assert_eq!(lane.recv(), Some(l));
    }

    // Control: on `/1` every frame is lane 0.
    let mut plain = bind_server(ServerLimits::v1());
    let (mut client_v1, conn_v1) = connect_accepted(&mut plain, client_limits(), 1, epoch(1));
    client_v1.try_send(b"a").expect("send");
    assert_eq!(
        server_recv_channel(&mut plain, conn_v1),
        (Channel::Lane0, b"a".to_vec())
    );
}

// ---------------------------------------------------------------------------
// Case 10: the floor windows carry maximal frames on `/2`.
// ---------------------------------------------------------------------------

#[test]
fn v2_floor_windows_carry_maximal_frames() {
    let mut sl = ServerLimits::v1();
    sl.stream_window_bytes = 4_100;
    sl.connection_window_bytes = 4_100;
    sl.send_window_bytes = 65_540;
    let mut cl = client_limits();
    cl.stream_window_bytes = 65_540;
    cl.connection_window_bytes = 65_540;
    cl.send_window_bytes = 4_100;
    let mut server = QuicServer::bind_dual(ServerConfig {
        bind: loopback(),
        identity: identity_a(),
        limits: sl,
    })
    .expect("bind_dual");
    let (mut client, conn) = connect_framed_accepted(&mut server, cl, 1, Framing::V2);

    for channel in [Channel::Lane0, Channel::Transfer] {
        let frame = pattern(6, u32::from(channel.as_u8()), INTENT_CAP);
        client.try_send_channel(channel, &frame).expect("up");
        assert_eq!(server_recv_channel(&mut server, conn), (channel, frame));
    }
    for channel in [Channel::Lane0, Channel::Transfer, Channel::Resume] {
        let frame = pattern(7, u32::from(channel.as_u8()), DELTA_CAP);
        server
            .try_send_channel(conn, channel, &frame)
            .expect("down");
        assert_eq!(client_recv_channel(&mut client), (channel, frame));
    }
    for i in 0..100u32 {
        let up_channel = if i % 2 == 0 {
            Channel::Transfer
        } else {
            Channel::Lane0
        };
        let up = pattern(8, i, cycling_size(i, INTENT_CAP));
        client.try_send_channel(up_channel, &up).expect("up");
        assert_eq!(server_recv_channel(&mut server, conn), (up_channel, up));
        let down_channel = match i % 3 {
            0 => Channel::Lane0,
            1 => Channel::Transfer,
            _ => Channel::Resume,
        };
        let down = pattern(9, i, cycling_size(i, DELTA_CAP));
        server
            .try_send_channel(conn, down_channel, &down)
            .expect("down");
        assert_eq!(client_recv_channel(&mut client), (down_channel, down));
    }
}

// ---------------------------------------------------------------------------
// Case 11: a close keeps tagged frames drainable.
// ---------------------------------------------------------------------------

#[test]
fn v2_close_keeps_tagged_frames_drainable() {
    let mut server = QuicServer::bind_dual(server_config(ServerLimits::v1())).expect("bind_dual");
    let (mut client, conn) = connect_framed_accepted(&mut server, client_limits(), 1, Framing::V2);
    let t = b"T".to_vec();
    let l = b"L".to_vec();
    client.try_send_channel(Channel::Transfer, &t).expect("t");
    client.try_send_channel(Channel::Lane0, &l).expect("l");
    client.close(CloseCode::Normal, CloseMode::Flush);

    assert_eq!(
        server_recv_channel(&mut server, conn),
        (Channel::Transfer, t)
    );
    assert_eq!(server_recv_channel(&mut server, conn), (Channel::Lane0, l));
    assert_eq!(
        server_err_channel(&mut server, conn),
        ServerError::Closed(CloseReason::Peer { code: 0 })
    );
    assert_eq!(
        next_closed(&mut server),
        (conn, CloseReason::Peer { code: 0 })
    );
}

// ---------------------------------------------------------------------------
// Case 12: a resume opening round trip.
// ---------------------------------------------------------------------------

#[test]
fn resume_opening_round_trip() {
    let mut server =
        QuicServer::bind_resumable(server_config(ServerLimits::v1())).expect("bind_resumable");
    let request = l1_request();
    let pending = connect_resume_async(
        client_config(client_limits()),
        server.local_addr(),
        SESSION,
        request.clone(),
    );
    until("one resume pending", || server.totals().pending == 1);
    assert!(server.wait(Duration::ZERO));
    assert!(server.poll_event().is_none());
    assert_eq!(server.totals().pending, 1);
    let received = next_resume(&mut server);
    assert_eq!(received.request, request);
    assert_eq!(received.request.len(), 40);
    assert_eq!(server.framing(received.conn), Ok(Framing::V2));
    let answer = l4_accepted();
    server
        .accept_resume(received.conn, &answer)
        .expect("accept_resume");
    let (mut client, got) = pending.join().expect("thread").expect("connect_resume");
    assert_eq!(got, answer);
    assert_eq!(client.framing(), Framing::V2);
    assert_eq!(client.welcome(), SESSION);
    assert_eq!(received.remote, client.local_addr());

    let ticket = l3_ticket();
    let (t, l) = (b"T".to_vec(), b"L".to_vec());
    server
        .try_send_channel(received.conn, Channel::Resume, &ticket)
        .expect("ticket");
    server
        .try_send_channel(received.conn, Channel::Transfer, &t)
        .expect("transfer");
    server
        .try_send_channel(received.conn, Channel::Lane0, &l)
        .expect("lane0");
    assert_eq!(client_recv_channel(&mut client), (Channel::Resume, ticket));
    assert_eq!(client_recv_channel(&mut client), (Channel::Transfer, t));
    assert_eq!(client_recv_channel(&mut client), (Channel::Lane0, l));

    client.try_send_channel(Channel::Lane0, b"i").expect("i");
    client.try_send_channel(Channel::Transfer, b"p").expect("p");
    assert_eq!(
        server_recv_channel(&mut server, received.conn),
        (Channel::Lane0, b"i".to_vec())
    );
    assert_eq!(
        server_recv_channel(&mut server, received.conn),
        (Channel::Transfer, b"p".to_vec())
    );

    let connections = server.totals().connections;
    client.close(CloseCode::Normal, CloseMode::Flush);
    match next_notice(&mut server) {
        ServerNotice::Event(ServerEvent::Closed { conn, reason }) => {
            assert_eq!(conn, received.conn);
            assert_eq!(reason, CloseReason::Peer { code: 0 });
        }
        other => panic!("expected Closed, got {other:?}"),
    }
    assert_eq!(server.totals().connections, connections - 1);
    assert_eq!(
        server.framing(received.conn),
        Err(ServerError::UnknownConnection)
    );
}

// ---------------------------------------------------------------------------
// Case 13: the resume opening's wire bytes are exact.
// ---------------------------------------------------------------------------

#[test]
fn resume_opening_wire_bytes_are_exact() {
    let mut server =
        QuicServer::bind_resumable(server_config(ServerLimits::v1())).expect("bind_resumable");
    let raw = raw_connect(server.local_addr(), ALPN_LANE0_V2).expect("raw /2");
    let (mut send, mut recv) = raw.open_and_write(&[0x91, 0x00, 0x00, 0x03, 0x41, 0x42, 0x43]);
    let received = next_resume(&mut server);
    assert_eq!(received.request, b"ABC");
    assert_eq!(
        format!("{received:?}"),
        format!(
            "ResumeReceived {{ conn: {:?}, remote: {:?}, request: <redacted> }}",
            received.conn, received.remote
        )
    );
    let notice = format!("{:?}", ServerNotice::Resume(received.clone()));
    assert!(notice.contains("request: <redacted>"));
    assert!(!notice.contains("65, 66, 67"));

    server
        .accept_resume(received.conn, b"xy")
        .expect("accept_resume");
    server
        .try_send_channel(received.conn, Channel::Resume, b"t")
        .expect("ticket");
    server
        .try_send_channel(received.conn, Channel::Lane0, b"ab")
        .expect("lane0");
    let expected = vec![
        0x91, 0x00, 0x00, 0x02, 0x78, 0x79, 0x91, 0x00, 0x00, 0x01, 0x74, 0x00, 0x00, 0x00, 0x02,
        0x61, 0x62,
    ];
    assert_eq!(raw_read_exact(&raw.rt, &mut recv, expected.len()), expected);

    raw.write(&mut send, &[0x00, 0x00, 0x00, 0x02, 0x61, 0x62]);
    raw.write(&mut send, &[0x91, 0x00, 0x00, 0x01, 0x7A]);
    assert_eq!(
        server_recv_channel(&mut server, received.conn),
        (Channel::Lane0, b"ab".to_vec())
    );
    assert_eq!(
        server_err_channel(&mut server, received.conn),
        ServerError::Closed(CloseReason::Local(CloseCode::ProtocolViolation))
    );
    assert_eq!(raw.closed_code(), Some(2));
}

// ---------------------------------------------------------------------------
// Case 14: the resume opening is opt-in and never downgrades.
// ---------------------------------------------------------------------------

#[test]
fn resume_opening_is_opt_in_and_never_downgrades() {
    // (a) `connect_resume` → `bind`.
    let mut plain = bind_server(ServerLimits::v1());
    let refused = plain.totals().refused_handshakes;
    let attempt = connect_resume_async(
        client_config(client_limits()),
        plain.local_addr(),
        SESSION,
        l1_request(),
    );
    assert!(matches!(
        attempt.join().expect("thread"),
        Err(ConnectError::Handshake)
    ));
    until("refused", || {
        plain.totals().refused_handshakes == refused + 1
    });
    assert!(plain.poll_notice().is_none());

    // (b) `connect_resume` → `bind_dual`: the opening is a hello header.
    let mut dual = QuicServer::bind_dual(server_config(ServerLimits::v1())).expect("bind_dual");
    let refused = dual.totals().refused_handshakes;
    let attempt = connect_resume_async(
        client_config(client_limits()),
        dual.local_addr(),
        SESSION,
        l1_request(),
    );
    assert!(matches!(
        attempt.join().expect("thread"),
        Err(ConnectError::Refused { code: 3 })
    ));
    until("refused", || {
        dual.totals().refused_handshakes == refused + 1
    });
    assert!(dual.poll_notice().is_none());
    assert!(dual.poll_event().is_none());

    // (c) a raw `/1` client on `bind_resumable` cannot open with `0x91`.
    let mut resumable =
        QuicServer::bind_resumable(server_config(ServerLimits::v1())).expect("bind_resumable");
    let refused = resumable.totals().refused_handshakes;
    let raw = raw_lane(resumable.local_addr());
    let (_send, _recv) = raw.open_and_write(&[0x91, 0x00, 0x00, 0x03, 0x41, 0x42, 0x43]);
    assert_eq!(raw.closed_code(), Some(3));
    until("refused", || {
        resumable.totals().refused_handshakes == refused + 1
    });
    assert!(resumable.poll_notice().is_none());

    // (d) on the same server, `connect` is V1 and `connect_framed(V2)` is V2.
    for framing in [Framing::V1, Framing::V2] {
        let pending = connect_framed_async(
            client_config(client_limits()),
            resumable.local_addr(),
            hello(1),
            framing,
        );
        let (conn, _, _) = next_hello_any(&mut resumable);
        assert_eq!(resumable.framing(conn), Ok(framing));
        resumable
            .accept(
                conn,
                Welcome {
                    wire_version: 1,
                    epoch: epoch(1),
                },
            )
            .expect("accept");
        let mut client = pending.join().expect("thread").expect("connect");
        assert_eq!(client.framing(), framing);
        client.try_send(b"i").expect("up");
        assert_eq!(server_recv(&mut resumable, conn), Ok(b"i".to_vec()));
        resumable.try_send(conn, b"o").expect("down");
        assert_eq!(
            client_recv_channel(&mut client),
            (Channel::Lane0, b"o".to_vec())
        );
    }

    // (e) the S§4.6 step-1 refusals never reach the network.
    let before = resumable.totals();
    let config = client_config(client_limits());
    for request in [Vec::new(), vec![0u8; 4_097]] {
        assert!(matches!(
            QuicClient::connect_resume(config.clone(), resumable.local_addr(), SESSION, &request),
            Err(ConnectError::Handshake)
        ));
    }
    let bad_session = Welcome {
        wire_version: 3,
        epoch: [0; 16],
    };
    assert!(matches!(
        QuicClient::connect_resume(
            config.clone(),
            resumable.local_addr(),
            bad_session,
            &l1_request()
        ),
        Err(ConnectError::Handshake)
    ));
    let mut bad_limits = client_limits();
    bad_limits.inbound_frames = 0;
    let bad_config = crpg_net_quic::ClientConfig {
        limits: bad_limits,
        ..config
    };
    assert!(matches!(
        QuicClient::connect_resume(bad_config, resumable.local_addr(), SESSION, &[]),
        Err(ConnectError::Config(_))
    ));
    assert_eq!(
        resumable.totals().refused_handshakes,
        before.refused_handshakes
    );
    assert_eq!(resumable.totals().connections, before.connections);
}

// ---------------------------------------------------------------------------
// Case 15: the resume opening close codes and its timeout.
// ---------------------------------------------------------------------------

#[test]
fn resume_opening_close_codes() {
    let mut server =
        QuicServer::bind_resumable(server_config(ServerLimits::v1())).expect("bind_resumable");
    let rows = [
        (&[0x91, 0x00, 0x00, 0x00], None, 2),
        (&[0x91, 0x00, 0x10, 0x01], None, 3),
        (&[0x91, 0x00, 0x00, 0x05], Some(&[0x41, 0x42]), 2),
        (&[0x81, 0x00, 0x00, 0x1E], None, 3),
        (&[0x00, 0x00, 0x00, 0x00], None, 2),
    ];
    for (header, body, code) in rows {
        let refused = server.totals().refused_handshakes;
        let raw = raw_connect(server.local_addr(), ALPN_LANE0_V2).expect("raw /2");
        let (mut send, _recv) = raw.open_and_write(header);
        if let Some(body) = body {
            raw.write(&mut send, body);
            raw.finish(&mut send);
        }
        assert_eq!(raw.closed_code(), Some(code));
        until("refused", || {
            server.totals().refused_handshakes == refused + 1
        });
        assert!(server.poll_notice().is_none());
    }

    // Boundary controls on the first server.
    let raw = raw_connect(server.local_addr(), ALPN_LANE0_V2).expect("raw /2");
    let (mut send, _recv) = raw.open_and_write(&[0x91, 0x00, 0x10, 0x00]);
    raw.write(&mut send, &vec![0x5A; 4_096]);
    let received = next_resume(&mut server);
    assert_eq!(received.request.len(), 4_096);
    server
        .refuse(received.conn, CloseCode::Normal)
        .expect("refuse");
    let _ = next_closed(&mut server);

    let pending = connect_resume_async(
        client_config(client_limits()),
        server.local_addr(),
        SESSION,
        vec![0x5A; 4_096],
    );
    let received = next_resume(&mut server);
    assert_eq!(received.request.len(), 4_096);
    server
        .accept_resume(received.conn, &l4_accepted())
        .expect("accept_resume");
    let _ = pending.join().expect("thread").expect("connect_resume");

    let hello_pending = connect_framed_async(
        client_config(client_limits()),
        server.local_addr(),
        hello(1),
        Framing::V2,
    );
    let (conn, _, _) = next_hello_any(&mut server);
    server
        .accept(
            conn,
            Welcome {
                wire_version: 1,
                epoch: epoch(1),
            },
        )
        .expect("accept");
    let _ = hello_pending.join().expect("thread").expect("connect");

    // A 200 ms hello timeout covers the partial header and short body.
    let mut limits = ServerLimits::v1();
    limits.hello_timeout_ms = 200;
    let mut slow = QuicServer::bind_resumable(server_config(limits)).expect("bind_resumable");
    let writes: [&[u8]; 3] = [&[], &[0x91, 0x00], &[0x91, 0x00, 0x00, 0x05, 0x41, 0x42]];
    for bytes in writes {
        let refused = slow.totals().refused_handshakes;
        let raw = raw_connect(slow.local_addr(), ALPN_LANE0_V2).expect("raw /2");
        let (_send, _recv) = raw.open_and_write(bytes);
        assert_eq!(raw.closed_code(), Some(4));
        until("timed out", || {
            slow.totals().refused_handshakes == refused + 1
        });
        assert!(slow.poll_notice().is_none());
    }
}

// ---------------------------------------------------------------------------
// Case 16: resume-pending decisions and their precedence.
// ---------------------------------------------------------------------------

#[test]
fn resume_pending_decisions() {
    let mut server =
        QuicServer::bind_resumable(server_config(ServerLimits::v1())).expect("bind_resumable");
    let r_thread = connect_resume_async(
        client_config(client_limits()),
        server.local_addr(),
        SESSION,
        l1_request(),
    );
    let r = next_resume(&mut server);
    let h_thread = connect_framed_async(
        client_config(client_limits()),
        server.local_addr(),
        hello(2),
        Framing::V2,
    );
    let (h, _, _) = next_hello_any(&mut server);
    let welcome = Welcome {
        wire_version: 2,
        epoch: epoch(2),
    };
    assert_eq!(
        server.accept(r.conn, welcome),
        Err(ServerError::InvalidWelcome)
    );
    assert_eq!(
        server.accept_resume(h, b"x"),
        Err(ServerError::InvalidWelcome)
    );
    assert_eq!(
        server.accept_resume(r.conn, b""),
        Err(ServerError::InvalidWelcome)
    );
    assert_eq!(
        server.accept_resume(r.conn, &vec![0u8; 65_537]),
        Err(ServerError::InvalidWelcome)
    );
    assert_eq!(
        server.try_send_channel(r.conn, Channel::Lane0, b"x"),
        Err(ChannelSendError::Send(SendError::NotAccepted))
    );
    assert_eq!(
        server.try_recv_channel(r.conn),
        Err(ServerError::NotAccepted)
    );
    assert_eq!(server.totals().pending, 2);
    let r_stats = queue_fields(server.stats(r.conn).expect("stats"));
    let h_stats = queue_fields(server.stats(h).expect("stats"));
    assert_eq!(queue_fields(server.stats(r.conn).expect("stats")), r_stats);
    assert_eq!(queue_fields(server.stats(h).expect("stats")), h_stats);

    let answer = vec![0x5A; 65_536];
    server
        .accept_resume(r.conn, &answer)
        .expect("accept_resume max answer");
    let (r_client, got) = r_thread.join().expect("thread").expect("connect_resume");
    assert_eq!(got, answer);

    // NotPending before InvalidWelcome.
    assert_eq!(
        server.accept_resume(r.conn, b"x"),
        Err(ServerError::NotPending)
    );
    assert_eq!(
        server.accept_resume(r.conn, b""),
        Err(ServerError::NotPending)
    );
    assert_eq!(server.accept(r.conn, welcome), Err(ServerError::NotPending));
    assert_eq!(
        server.refuse(r.conn, CloseCode::ServerBusy),
        Err(ServerError::NotPending)
    );
    server.accept(h, welcome).expect("accept h");
    let _h_client = h_thread.join().expect("thread").expect("connect");
    assert_eq!(server.accept_resume(h, b""), Err(ServerError::NotPending));

    // Closed before NotPending and InvalidWelcome.
    let r2_thread = connect_resume_async(
        client_config(client_limits()),
        server.local_addr(),
        SESSION,
        l1_request(),
    );
    let r2 = match next_notice(&mut server) {
        ServerNotice::Resume(received) => received,
        other => panic!("expected R2 Resume, got {other:?}"),
    };
    server
        .refuse(r2.conn, CloseCode::ServerBusy)
        .expect("refuse r2");
    assert!(matches!(
        r2_thread.join().expect("thread"),
        Err(ConnectError::Refused { code: 7 })
    ));
    let closed_busy = Err(ServerError::Closed(CloseReason::Local(
        CloseCode::ServerBusy,
    )));
    assert_eq!(server.accept_resume(r2.conn, b"x"), closed_busy);
    assert_eq!(server.accept_resume(r2.conn, b""), closed_busy);
    assert_eq!(server.accept(r2.conn, welcome), closed_busy);

    // The notices for R2 are its opening, then its close.
    match next_notice(&mut server) {
        ServerNotice::Event(ServerEvent::Closed { conn, reason }) => {
            assert_eq!(conn, r2.conn);
            assert_eq!(reason, CloseReason::Local(CloseCode::ServerBusy));
        }
        other => panic!("expected R2 Closed, got {other:?}"),
    }

    // UnknownConnection first, on a released record and a forgotten id.
    let unknown = Err(ServerError::UnknownConnection);
    assert_eq!(server.accept_resume(r2.conn, b"x"), unknown);
    assert_eq!(server.accept_resume(r2.conn, b""), unknown);
    assert_eq!(server.accept(r2.conn, welcome), unknown);

    let h2_thread = connect_framed_async(
        client_config(client_limits()),
        server.local_addr(),
        hello(2),
        Framing::V2,
    );
    let (h2, _, _) = next_hello_any(&mut server);
    server.refuse(h2, CloseCode::ServerBusy).expect("refuse h2");
    let _ = h2_thread.join().expect("thread");
    assert_eq!(
        server.accept_resume(h2, b"x"),
        Err(ServerError::Closed(CloseReason::Local(
            CloseCode::ServerBusy
        )))
    );

    server
        .close(r.conn, CloseCode::Normal, CloseMode::Discard)
        .expect("close r");
    assert_eq!(
        server.accept_resume(r.conn, b"x"),
        Err(ServerError::Closed(CloseReason::Local(CloseCode::Normal)))
    );

    let f = forgotten(&mut server, Framing::V2);
    assert_eq!(server.accept_resume(f, b"x"), unknown);

    // The decision timer applies to resume openings too.
    let mut limits = ServerLimits::v1();
    limits.decision_timeout_ms = 200;
    let mut slow = QuicServer::bind_resumable(server_config(limits)).expect("bind_resumable");
    let thread = connect_resume_async(
        client_config(client_limits()),
        slow.local_addr(),
        SESSION,
        l1_request(),
    );
    let received = next_resume(&mut slow);
    match next_notice(&mut slow) {
        ServerNotice::Event(ServerEvent::Closed { conn, reason }) => {
            assert_eq!(conn, received.conn);
            assert_eq!(reason, CloseReason::Local(CloseCode::AuthTimeout));
        }
        other => panic!("expected AuthTimeout Closed, got {other:?}"),
    }
    assert!(matches!(
        thread.join().expect("thread"),
        Err(ConnectError::Refused { code: 8 })
    ));
    drop(r_client);
}

// ---------------------------------------------------------------------------
// Case 17: an answer followed by a close reaches the client.
// ---------------------------------------------------------------------------

#[test]
fn resume_answer_then_close_reaches_client() {
    for (answer, code) in [
        (l6_denied(), CloseCode::AuthRefused),
        (l7_busy(), CloseCode::ServerBusy),
    ] {
        let mut server =
            QuicServer::bind_resumable(server_config(ServerLimits::v1())).expect("bind_resumable");
        let pending = connect_resume_async(
            client_config(client_limits()),
            server.local_addr(),
            SESSION,
            l1_request(),
        );
        let received = next_resume(&mut server);
        server
            .accept_resume(received.conn, &answer)
            .expect("accept_resume");
        server
            .close(received.conn, code, CloseMode::Flush)
            .expect("close");
        let (mut client, got) = pending.join().expect("thread").expect("connect_resume");
        assert_eq!(got, answer);
        assert_eq!(
            client_err_channel(&mut client),
            CloseReason::Peer {
                code: u64::from(code.as_u32())
            }
        );
        match next_notice(&mut server) {
            ServerNotice::Event(ServerEvent::Closed { conn, reason }) => {
                assert_eq!(conn, received.conn);
                assert_eq!(reason, CloseReason::Local(code));
            }
            other => panic!("expected Closed, got {other:?}"),
        }
    }

    // Control: `refuse` sends no answer.
    let mut server =
        QuicServer::bind_resumable(server_config(ServerLimits::v1())).expect("bind_resumable");
    let pending = connect_resume_async(
        client_config(client_limits()),
        server.local_addr(),
        SESSION,
        l1_request(),
    );
    let received = next_resume(&mut server);
    server
        .refuse(received.conn, CloseCode::AuthRefused)
        .expect("refuse");
    assert!(matches!(
        pending.join().expect("thread"),
        Err(ConnectError::Refused { code: 5 })
    ));
}

// ---------------------------------------------------------------------------
// Case 18: the resume channel's direction rules.
// ---------------------------------------------------------------------------

#[test]
fn resume_channel_direction_rules() {
    // A `/2` hello connection: downstream `0x91` is legal; upstream is not.
    let mut dual = QuicServer::bind_dual(server_config(ServerLimits::v1())).expect("bind_dual");
    let (mut client, conn) = connect_framed_accepted(&mut dual, client_limits(), 1, Framing::V2);
    dual.try_send_channel(conn, Channel::Resume, b"tk")
        .expect("down");
    assert_eq!(
        client_recv_channel(&mut client),
        (Channel::Resume, b"tk".to_vec())
    );
    let before = queue_fields(client.stats());
    for frame in [b"x".as_slice(), b"".as_slice()] {
        assert_eq!(
            client.try_send_channel(Channel::Resume, frame),
            Err(ChannelSendError::ChannelUnavailable)
        );
        assert_eq!(queue_fields(client.stats()), before);
    }
    client
        .try_send_channel(Channel::Lane0, b"ok")
        .expect("lane0 control");

    // A resumed connection refuses an upstream `0x91` too.
    let mut resumable =
        QuicServer::bind_resumable(server_config(ServerLimits::v1())).expect("bind_resumable");
    let (mut resumed, resumed_conn) =
        connect_resume_accepted(&mut resumable, client_limits(), &l1_request());
    assert_eq!(resumed.framing(), Framing::V2);
    let before = queue_fields(resumed.stats());
    assert_eq!(
        resumed.try_send_channel(Channel::Resume, b"x"),
        Err(ChannelSendError::ChannelUnavailable)
    );
    assert_eq!(queue_fields(resumed.stats()), before);
    resumed
        .try_send_channel(Channel::Lane0, b"i")
        .expect("resumed lane0");
    assert_eq!(
        server_recv_channel(&mut resumable, resumed_conn),
        (Channel::Lane0, b"i".to_vec())
    );

    // A `/1` connection: the server refuses `Resume` (and `Transfer`).
    let mut plain = bind_server(ServerLimits::v1());
    let (mut client_v1, conn_v1) = connect_accepted(&mut plain, client_limits(), 1, epoch(1));
    let before = queue_fields(plain.stats(conn_v1).expect("stats"));
    for frame in [b"x".as_slice(), b"".as_slice()] {
        assert_eq!(
            plain.try_send_channel(conn_v1, Channel::Resume, frame),
            Err(ChannelSendError::ChannelUnavailable)
        );
        assert_eq!(
            plain.try_send_channel(conn_v1, Channel::Transfer, frame),
            Err(ChannelSendError::ChannelUnavailable)
        );
    }
    assert_eq!(queue_fields(plain.stats(conn_v1).expect("stats")), before);
    plain.try_send(conn_v1, b"o").expect("lane0 control");
    assert_eq!(
        client_recv_channel(&mut client_v1),
        (Channel::Lane0, b"o".to_vec())
    );
    let before = queue_fields(client_v1.stats());
    assert_eq!(
        client_v1.try_send_channel(Channel::Resume, b"x"),
        Err(ChannelSendError::ChannelUnavailable)
    );
    assert_eq!(queue_fields(client_v1.stats()), before);
}

// ---------------------------------------------------------------------------
// Case 19: one notice order.
// ---------------------------------------------------------------------------

#[test]
fn notices_keep_one_order() {
    let mut server =
        QuicServer::bind_resumable(server_config(ServerLimits::v1())).expect("bind_resumable");
    let a_thread = connect_framed_async(
        client_config(client_limits()),
        server.local_addr(),
        hello(1),
        Framing::V2,
    );
    until("A pending", || server.totals().pending == 1);
    let r_thread = connect_resume_async(
        client_config(client_limits()),
        server.local_addr(),
        SESSION,
        l1_request(),
    );
    until("R pending", || server.totals().pending == 2);
    let c_thread = connect_async(
        client_config(client_limits()),
        server.local_addr(),
        hello(1),
    );
    until("C pending", || server.totals().pending == 3);

    let a = match server.poll_event() {
        Some(ServerEvent::HelloReceived { conn, .. }) => conn,
        other => panic!("expected A HelloReceived, got {other:?}"),
    };
    assert!(server.poll_event().is_none());
    assert!(server.wait(Duration::ZERO));
    let r = match server.poll_notice() {
        Some(ServerNotice::Resume(received)) => received.conn,
        other => panic!("expected R Resume, got {other:?}"),
    };
    let c = match server.poll_event() {
        Some(ServerEvent::HelloReceived { conn, .. }) => conn,
        other => panic!("expected C HelloReceived, got {other:?}"),
    };
    server
        .accept(
            a,
            Welcome {
                wire_version: 1,
                epoch: epoch(1),
            },
        )
        .expect("accept A");
    server
        .accept(
            c,
            Welcome {
                wire_version: 1,
                epoch: epoch(1),
            },
        )
        .expect("accept C");
    let _a_client = a_thread.join().expect("A thread").expect("connect");
    let _c_client = c_thread.join().expect("C thread").expect("connect");

    let connections = server.totals().connections;
    server.refuse(r, CloseCode::ServerBusy).expect("refuse R");
    match server.poll_event() {
        Some(ServerEvent::Closed { conn, reason }) => {
            assert_eq!(conn, r);
            assert_eq!(reason, CloseReason::Local(CloseCode::ServerBusy));
        }
        other => panic!("expected R Closed, got {other:?}"),
    }
    assert_eq!(server.totals().connections, connections - 1);
    assert!(server.poll_event().is_none());
    let _ = r_thread.join().expect("R thread");

    // Control: on `bind`, the notice queue reports the same events.
    let mut plain = bind_server(ServerLimits::v1());
    let pending = connect_async(client_config(client_limits()), plain.local_addr(), hello(1));
    match next_notice(&mut plain) {
        ServerNotice::Event(ServerEvent::HelloReceived { conn, .. }) => {
            plain
                .accept(
                    conn,
                    Welcome {
                        wire_version: 1,
                        epoch: epoch(1),
                    },
                )
                .expect("accept");
        }
        other => panic!("expected HelloReceived, got {other:?}"),
    }
    let client = pending.join().expect("thread").expect("connect");
    client.close(CloseCode::Normal, CloseMode::Flush);
    match next_notice(&mut plain) {
        ServerNotice::Event(ServerEvent::Closed { reason, .. }) => {
            assert_eq!(reason, CloseReason::Peer { code: 0 });
        }
        other => panic!("expected Closed, got {other:?}"),
    }
    assert!(plain.poll_event().is_none());
}

// ---------------------------------------------------------------------------
// Case 20: the `/2` client-reader close map, through a raw quinn server.
// ---------------------------------------------------------------------------

fn raw_v2_connect(
    raw: &RawServer,
) -> (
    QuicClient,
    quinn::Connection,
    quinn::SendStream,
    quinn::RecvStream,
) {
    let pending = connect_framed_async(
        client_config(client_limits()),
        raw.endpoint.local_addr().expect("raw addr"),
        hello(1),
        Framing::V2,
    );
    let (conn, mut send, mut recv) = raw.accept_lane();
    accept_raw_hello(raw, &mut recv, &mut send);
    let client = pending.join().expect("thread").expect("connect");
    assert_eq!(client.framing(), Framing::V2);
    (client, conn, send, recv)
}

fn channel_frame(channel: Channel, payload: &[u8]) -> Vec<u8> {
    let mut out = channel_frame_header(channel, payload.len(), DELTA_CAP)
        .expect("header")
        .to_vec();
    out.extend_from_slice(payload);
    out
}

#[test]
fn v2_client_reader_close_codes() {
    let raw = raw_server(&[ALPN_LANE0_V2, ALPN_LANE0_V1]);
    for (probe, code) in [
        ([0x01u8, 0x00, 0x00, 0x01], 2u64),
        ([0x02, 0x00, 0x00, 0x01], 2),
        ([0x80, 0x00, 0x00, 0x01], 2),
        ([0x82, 0x00, 0x00, 0x01], 2),
        ([0x90, 0x00, 0x00, 0x01], 2),
        ([0x92, 0x00, 0x00, 0x01], 2),
        ([0xFF, 0x00, 0x00, 0x01], 2),
        ([0x05, 0xFF, 0xFF, 0xFF], 2),
        ([0x00, 0x00, 0x00, 0x00], 2),
        ([0x81, 0x00, 0x00, 0x00], 2),
        ([0x91, 0x00, 0x00, 0x00], 2),
        ([0x00, 0x01, 0x00, 0x01], 3),
        ([0x81, 0x01, 0x00, 0x01], 3),
        ([0x91, 0x01, 0x00, 0x01], 3),
    ] {
        let (mut client, conn, mut send, _recv) = raw_v2_connect(&raw);
        raw.write(&mut send, &[0x00, 0x00, 0x00, 0x02, 0x61, 0x62]);
        raw.write(&mut send, &[0x81, 0x00, 0x00, 0x03, 0x78, 0x79, 0x7A]);
        raw.write(&mut send, &[0x91, 0x00, 0x00, 0x01, 0x74]);
        raw.write(&mut send, &probe);
        assert_eq!(
            client_recv_channel(&mut client),
            (Channel::Lane0, b"ab".to_vec())
        );
        assert_eq!(
            client_recv_channel(&mut client),
            (Channel::Transfer, b"xyz".to_vec())
        );
        assert_eq!(
            client_recv_channel(&mut client),
            (Channel::Resume, b"t".to_vec())
        );
        assert_eq!(
            client_err_channel(&mut client),
            CloseReason::Local(CloseCode::from_u64(code).expect("code"))
        );
        assert_eq!(raw.closed_code(&conn), Some(code));
    }

    // A body cut short: FIN inside the body.
    {
        let (mut client, conn, mut send, _recv) = raw_v2_connect(&raw);
        raw.write(&mut send, &[0x00, 0x00, 0x00, 0x02, 0x61, 0x62]);
        raw.write(&mut send, &[0x81, 0x00, 0x00, 0x03, 0x78, 0x79, 0x7A]);
        raw.write(&mut send, &[0x91, 0x00, 0x00, 0x01, 0x74]);
        raw.write(&mut send, &[0x91, 0x00, 0x00, 0x05, 0x41, 0x42]);
        raw.finish(&mut send);
        let _ = client_recv_channel(&mut client);
        let _ = client_recv_channel(&mut client);
        let _ = client_recv_channel(&mut client);
        assert_eq!(
            client_err_channel(&mut client),
            CloseReason::Local(CloseCode::ProtocolViolation)
        );
        assert_eq!(raw.closed_code(&conn), Some(2));
    }

    // Controls: maximal downstream frames on all three channels.
    {
        let (mut client, conn, mut send, _recv) = raw_v2_connect(&raw);
        for channel in [Channel::Lane0, Channel::Transfer, Channel::Resume] {
            let payload = pattern(11, u32::from(channel.as_u8()), DELTA_CAP);
            raw.write(&mut send, &channel_frame(channel, &payload));
        }
        for channel in [Channel::Lane0, Channel::Transfer, Channel::Resume] {
            assert_eq!(
                client_recv_channel(&mut client),
                (channel, pattern(11, u32::from(channel.as_u8()), DELTA_CAP))
            );
        }
        raw.close(&conn, 0);
        assert_eq!(
            client_err_channel(&mut client),
            CloseReason::Peer { code: 0 }
        );
    }

    // Controls: the `/1` reader is unchanged.
    for probe in [[0x81u8, 0x00, 0x00, 0x1E], [0x91, 0x00, 0x00, 0x01]] {
        let pending = connect_async(
            client_config(client_limits()),
            raw.endpoint.local_addr().expect("raw addr"),
            hello(1),
        );
        let (conn, mut send, mut recv) = raw.accept_lane();
        accept_raw_hello(&raw, &mut recv, &mut send);
        let mut client = pending.join().expect("thread").expect("connect");
        raw.write(&mut send, &[0x00, 0x00, 0x00, 0x02, 0x61, 0x62]);
        assert_eq!(
            client_recv_channel(&mut client),
            (Channel::Lane0, b"ab".to_vec())
        );
        raw.write(&mut send, &probe);
        assert_eq!(
            client_err_channel(&mut client),
            CloseReason::Local(CloseCode::FrameTooLarge)
        );
        assert_eq!(raw.closed_code(&conn), Some(3));
    }
}

// ---------------------------------------------------------------------------
// Case 21: `connect_resume`'s answer checks and timeout, raw quinn server.
// ---------------------------------------------------------------------------

#[test]
fn connect_resume_answer_checks_and_timeout() {
    let request = l1_request();
    let mut welcome_bytes = vec![0x00, 0x00, 0x00, 0x11];
    welcome_bytes.extend_from_slice(
        &Welcome {
            wire_version: 1,
            epoch: epoch(3),
        }
        .encode()
        .expect("welcome"),
    );
    let malformed: [(Vec<u8>, bool); 6] = [
        (welcome_bytes, false),
        (vec![0x81, 0x00, 0x00, 0x01, 0x00], false),
        (vec![0x05, 0x00, 0x00, 0x01, 0x00], false),
        (vec![0x91, 0x00, 0x00, 0x00], false),
        (vec![0x91, 0x01, 0x00, 0x01], false),
        (vec![0x91, 0x00, 0x00, 0x05, 0x41, 0x42], true),
    ];
    for (answer, fin) in malformed {
        let raw = raw_server(&[ALPN_LANE0_V2]);
        let pending = connect_resume_async(
            client_config(client_limits()),
            raw.endpoint.local_addr().expect("raw addr"),
            SESSION,
            request.clone(),
        );
        let (conn, mut send, mut recv) = raw.accept_lane();
        let header = raw.read_exact(&mut recv, 4);
        assert_eq!(header, vec![0x91, 0x00, 0x00, 0x28]);
        assert_eq!(raw.read_exact(&mut recv, 40), request);
        raw.write(&mut send, &answer);
        if fin {
            raw.finish(&mut send);
        }
        assert!(matches!(
            pending.join().expect("thread"),
            Err(ConnectError::InvalidWelcome)
        ));
        assert_eq!(raw.closed_code(&conn), Some(2));
    }

    // Control: a valid answer, then a lane frame.
    {
        let raw = raw_server(&[ALPN_LANE0_V2]);
        let pending = connect_resume_async(
            client_config(client_limits()),
            raw.endpoint.local_addr().expect("raw addr"),
            SESSION,
            request.clone(),
        );
        let (_conn, mut send, mut recv) = raw.accept_lane();
        let header = raw.read_exact(&mut recv, 4);
        assert_eq!(header, vec![0x91, 0x00, 0x00, 0x28]);
        assert_eq!(raw.read_exact(&mut recv, 40), request);
        raw.write(&mut send, &[0x91, 0x00, 0x00, 0x02, 0x78, 0x79]);
        let (mut client, answer) = pending.join().expect("thread").expect("connect_resume");
        assert_eq!(answer, b"xy");
        assert_eq!(client.framing(), Framing::V2);
        assert_eq!(client.welcome(), SESSION);
        raw.write(&mut send, &[0x00, 0x00, 0x00, 0x01, 0x7A]);
        assert_eq!(
            client_recv_channel(&mut client),
            (Channel::Lane0, b"z".to_vec())
        );
    }

    // Timeout: the one timer covers the answer read.
    {
        let raw = raw_server(&[ALPN_LANE0_V2]);
        let mut limits = client_limits();
        limits.connect_timeout_ms = 300;
        let pending = connect_resume_async(
            client_config(limits),
            raw.endpoint.local_addr().expect("raw addr"),
            SESSION,
            request.clone(),
        );
        let (conn, _send, mut recv) = raw.accept_lane();
        let header = raw.read_exact(&mut recv, 4);
        assert_eq!(header, vec![0x91, 0x00, 0x00, 0x28]);
        assert_eq!(raw.read_exact(&mut recv, 40), request);
        assert!(matches!(
            pending.join().expect("thread"),
            Err(ConnectError::TimedOut)
        ));
        assert_eq!(raw.closed_code(&conn), Some(0));
    }
}
