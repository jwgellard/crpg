#![allow(dead_code)] // Each test binary that includes this rig uses a different subset.
//! Test-only rig for `tests/quic.rs` (T023 R8.1): loopback builders on
//! `127.0.0.1:0`, failure-guard waits with a hard 30 s deadline, a raw
//! quinn client with its own pin check, and `UdpRelay`, a std-only UDP
//! relay that drops, reorders or blackholes packets.
//!
//! The waits are guards that panic `"deadline"`; they are never oracles.
//! Every assertion is made on the observed values themselves.

use std::future::Future;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crpg_net_quic::{
    CertificatePin, ClientConfig, ClientLimits, CloseReason, ConnectError, ConnectionId,
    Credential, Hello, QuicClient, QuicServer, ServerConfig, ServerError, ServerEvent,
    ServerIdentity, ServerLimits, Welcome, ALPN_LANE0_V1, TLS_SERVER_NAME,
};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

pub const SERVER_A_CERT: &[u8] = include_bytes!("../fixtures/server_a.cert.der");
pub const SERVER_A_KEY: &[u8] = include_bytes!("../fixtures/server_a.key.pk8.der");
pub const SERVER_B_CERT: &[u8] = include_bytes!("../fixtures/server_b.cert.der");
pub const SERVER_B_KEY: &[u8] = include_bytes!("../fixtures/server_b.key.pk8.der");

/// Hard bound on every guard wait.
pub const DEADLINE: Duration = Duration::from_secs(30);

pub fn loopback() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

pub fn identity_a() -> ServerIdentity {
    ServerIdentity::new(SERVER_A_CERT.to_vec(), SERVER_A_KEY.to_vec())
}

pub fn pin_a() -> CertificatePin {
    CertificatePin::of_certificate(SERVER_A_CERT)
}

pub fn pin_b() -> CertificatePin {
    CertificatePin::of_certificate(SERVER_B_CERT)
}

pub fn bind_server(limits: ServerLimits) -> QuicServer {
    QuicServer::bind(ServerConfig {
        bind: loopback(),
        identity: identity_a(),
        limits,
    })
    .expect("bind loopback server")
}

pub fn client_config(limits: ClientLimits) -> ClientConfig {
    ClientConfig {
        bind: loopback(),
        pin: pin_a(),
        limits,
    }
}

pub fn credential(len: usize, fill: u8) -> Credential {
    Credential::new(vec![fill; len]).expect("credential length in bounds")
}

pub fn hello(wire_version: u8) -> Hello {
    Hello {
        wire_version,
        credential: credential(16, 0x11),
    }
}

pub fn epoch(fill: u8) -> [u8; 16] {
    [fill; 16]
}

/// Runs `QuicClient::connect` on its own thread so the caller can drive the
/// server's decision meanwhile.
pub fn connect_async(
    config: ClientConfig,
    server: SocketAddr,
    hello: Hello,
) -> JoinHandle<Result<QuicClient, ConnectError>> {
    std::thread::spawn(move || QuicClient::connect(config, server, &hello))
}

/// Guard: polls `cond` until true, panicking `"deadline"` after 30 s. The
/// short park between polls is pacing, not an oracle.
pub fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < DEADLINE, "deadline: {what}");
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

/// Guard: the next server event.
pub fn next_event(server: &mut QuicServer) -> ServerEvent {
    let start = Instant::now();
    loop {
        if let Some(event) = server.poll_event() {
            return event;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: server event");
        server.wait(Duration::from_millis(20));
    }
}

/// Guard: the next event, which must be `HelloReceived`.
pub fn next_hello(server: &mut QuicServer) -> (ConnectionId, SocketAddr, Hello) {
    match next_event(server) {
        ServerEvent::HelloReceived {
            conn,
            remote,
            hello,
        } => (conn, remote, hello),
        other => panic!("expected HelloReceived, got {other:?}"),
    }
}

/// Guard: the next event, which must be `Closed`.
pub fn next_closed(server: &mut QuicServer) -> (ConnectionId, CloseReason) {
    match next_event(server) {
        ServerEvent::Closed { conn, reason } => (conn, reason),
        other => panic!("expected Closed, got {other:?}"),
    }
}

/// Connects a client and accepts it with `Welcome { wire_version, epoch }`.
pub fn connect_accepted(
    server: &mut QuicServer,
    limits: ClientLimits,
    wire_version: u8,
    epoch: [u8; 16],
) -> (QuicClient, ConnectionId) {
    let pending = connect_async(
        client_config(limits),
        server.local_addr(),
        hello(wire_version),
    );
    let (conn, _, _) = next_hello(server);
    server
        .accept(
            conn,
            Welcome {
                wire_version,
                epoch,
            },
        )
        .expect("accept");
    let client = pending.join().expect("client thread").expect("connect");
    (client, conn)
}

/// Guard: the server's next inbound frame on `conn`, or its error.
pub fn server_recv(server: &mut QuicServer, conn: ConnectionId) -> Result<Vec<u8>, ServerError> {
    let start = Instant::now();
    loop {
        match server.try_recv(conn) {
            Ok(Some(frame)) => return Ok(frame),
            Ok(None) => {}
            Err(error) => return Err(error),
        }
        assert!(start.elapsed() < DEADLINE, "deadline: server frame");
        server.wait(Duration::from_millis(20));
    }
}

/// Guard: the client's next inbound frame, or the close reason.
pub fn client_recv(client: &mut QuicClient) -> Result<Vec<u8>, CloseReason> {
    let start = Instant::now();
    loop {
        match client.try_recv() {
            Ok(Some(frame)) => return Ok(frame),
            Ok(None) => {}
            Err(reason) => return Err(reason),
        }
        assert!(start.elapsed() < DEADLINE, "deadline: client frame");
        client.wait(Duration::from_millis(20));
    }
}

/// A deterministic payload: the 4-byte big-endian counter (when it fits),
/// then bytes derived from the stream, counter and position.
pub fn pattern(stream: u8, counter: u32, size: usize) -> Vec<u8> {
    let mut out: Vec<u8> = (0..size)
        .map(|i| (u32::from(stream) * 131 + counter * 31 + u32::try_from(i).unwrap_or(0) * 7) as u8)
        .collect();
    let tag = counter.to_be_bytes();
    let n = size.min(4);
    out[..n].copy_from_slice(&tag[..n]);
    out
}

/// Size `i` of a cycle over `1..=cap` with a prime stride.
pub fn cycling_size(i: u32, cap: usize) -> usize {
    1 + (usize::try_from(i).unwrap_or(0) * 7919) % cap
}

// ---------------------------------------------------------------------------
// Raw quinn client (malformed hellos, foreign ALPN, early frames, aborts).
// ---------------------------------------------------------------------------

/// A current-thread tokio runtime driven by its own thread; `run` polls a
/// future on the caller's thread while that thread drives I/O and timers.
pub struct RawRuntime {
    handle: tokio::runtime::Handle,
    stop: Arc<tokio::sync::Notify>,
    thread: Option<JoinHandle<()>>,
}

impl RawRuntime {
    pub fn new() -> RawRuntime {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .expect("raw runtime");
        let handle = runtime.handle().clone();
        let stop = Arc::new(tokio::sync::Notify::new());
        let thread_stop = stop.clone();
        let thread = std::thread::spawn(move || {
            runtime.block_on(thread_stop.notified());
        });
        RawRuntime {
            handle,
            stop,
            thread: Some(thread),
        }
    }

    pub fn run<F: Future>(&self, future: F) -> F::Output {
        self.handle.block_on(future)
    }
}

impl Drop for RawRuntime {
    fn drop(&mut self) {
        self.stop.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The raw client's own pin check, written independently of the crate's:
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

pub struct RawClient {
    pub rt: RawRuntime,
    pub endpoint: quinn::Endpoint,
    pub connection: quinn::Connection,
}

/// A raw quinn connection to `server` offering exactly `alpn`.
pub fn raw_connect(server: SocketAddr, alpn: &[u8]) -> Result<RawClient, quinn::ConnectionError> {
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
    tls.alpn_protocols = vec![alpn.to_vec()];
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

/// The default lane ALPN for raw clients.
pub fn raw_lane(server: SocketAddr) -> RawClient {
    raw_connect(server, ALPN_LANE0_V1).expect("raw lane connection")
}

pub fn frame_bytes(payload: &[u8]) -> Vec<u8> {
    let mut out = u32::try_from(payload.len())
        .expect("u32 length")
        .to_be_bytes()
        .to_vec();
    out.extend_from_slice(payload);
    out
}

impl RawClient {
    /// Opens the lane stream and writes `bytes` raw.
    pub fn open_and_write(&self, bytes: &[u8]) -> (quinn::SendStream, quinn::RecvStream) {
        let connection = self.connection.clone();
        let bytes = bytes.to_vec();
        self.rt.run(async move {
            let (mut send, recv) = connection.open_bi().await.expect("open bi");
            send.write_all(&bytes).await.expect("write");
            (send, recv)
        })
    }

    pub fn write(&self, send: &mut quinn::SendStream, bytes: &[u8]) {
        self.rt
            .run(async { send.write_all(bytes).await })
            .expect("raw write");
    }

    pub fn finish(&self, send: &mut quinn::SendStream) {
        send.finish().expect("finish");
    }

    /// Reads the 17-byte welcome frame.
    pub fn read_welcome(&self, recv: &mut quinn::RecvStream) -> Welcome {
        let mut frame = [0u8; 4 + 17];
        self.rt
            .run(async { recv.read_exact(&mut frame).await })
            .expect("welcome bytes");
        assert_eq!(&frame[..4], &[0, 0, 0, 17]);
        Welcome::decode(&frame[4..]).expect("welcome")
    }

    /// The application close code the server ended the connection with.
    pub fn closed_code(&self) -> Option<u64> {
        let connection = self.connection.clone();
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

    pub fn close(&self, code: u32) {
        self.connection.close(quinn::VarInt::from_u32(code), b"raw");
        let endpoint = self.endpoint.clone();
        self.rt.run(async move {
            let _ = tokio::time::timeout(Duration::from_secs(2), endpoint.wait_idle()).await;
        });
    }
}

// ---------------------------------------------------------------------------
// UdpRelay.
// ---------------------------------------------------------------------------

/// Relay impairment: drop with probability `1/drop_every` (0 = never) from
/// a seeded SplitMix64, and rotate blocks of `reorder_depth` packets (≤ 1 =
/// no reordering).
#[derive(Debug, Clone, Copy)]
pub struct RelayConfig {
    pub drop_every: u64,
    pub reorder_depth: usize,
    pub seed: u64,
}

impl RelayConfig {
    pub fn clean() -> RelayConfig {
        RelayConfig {
            drop_every: 0,
            reorder_depth: 0,
            seed: 0,
        }
    }
}

#[derive(Default)]
struct Counters {
    forwarded: AtomicU64,
    dropped: AtomicU64,
    reordered: AtomicU64,
}

/// A client ↔ server UDP relay. Clients connect to `addr()`.
pub struct UdpRelay {
    addr: SocketAddr,
    counters: Arc<Counters>,
    blackhole: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

struct Direction {
    config: RelayConfig,
    rng: SplitMix64,
    block: Vec<Vec<u8>>,
    counters: Arc<Counters>,
    blackhole: Arc<AtomicBool>,
}

impl Direction {
    fn offer(&mut self, packet: Vec<u8>, send: &mut dyn FnMut(&[u8])) {
        if self.blackhole.load(Ordering::SeqCst) {
            self.counters.dropped.fetch_add(1, Ordering::SeqCst);
            return;
        }
        if self.config.drop_every > 0 && self.rng.next().is_multiple_of(self.config.drop_every) {
            self.counters.dropped.fetch_add(1, Ordering::SeqCst);
            return;
        }
        if self.config.reorder_depth <= 1 {
            send(&packet);
            self.counters.forwarded.fetch_add(1, Ordering::SeqCst);
            return;
        }
        self.block.push(packet);
        if self.block.len() == self.config.reorder_depth {
            // Block rotation: the first packet goes out last.
            self.block.rotate_left(1);
            for packet in self.block.drain(..) {
                send(&packet);
                self.counters.forwarded.fetch_add(1, Ordering::SeqCst);
            }
            self.counters.reordered.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Whether a partial reorder block is being held.
    fn holding(&self) -> bool {
        !self.block.is_empty()
    }

    /// A partial block is released in order as soon as no further packet is
    /// already queued (or on the idle read timeout), so reordering happens
    /// within a burst and never waits on the OS timer.
    fn release(&mut self, send: &mut dyn FnMut(&[u8])) {
        for packet in self.block.drain(..) {
            send(&packet);
            self.counters.forwarded.fetch_add(1, Ordering::SeqCst);
        }
    }
}

/// Receive one datagram. While a partial reorder block is held, only a
/// packet that is already queued is accepted (a non-blocking read), so the
/// block's release never depends on read-timeout granularity: Windows rounds
/// the 2 ms timeout up to its ~15.6 ms timer tick, which inflated the relayed
/// RTT roughly eightfold and starved the impaired transfer of its deadline.
/// With nothing held, the read blocks for the short idle timeout only so the
/// thread can observe `stop`.
fn recv_burst(
    socket: &UdpSocket,
    buf: &mut [u8],
    holding: bool,
) -> std::io::Result<(usize, SocketAddr)> {
    socket.set_nonblocking(holding)?;
    socket.recv_from(buf)
}

impl UdpRelay {
    pub fn start(server: SocketAddr, config: RelayConfig) -> UdpRelay {
        let front = UdpSocket::bind(loopback()).expect("relay front");
        let back = UdpSocket::bind(loopback()).expect("relay back");
        let read_timeout = Some(Duration::from_millis(2));
        front.set_read_timeout(read_timeout).expect("timeout");
        back.set_read_timeout(read_timeout).expect("timeout");
        let addr = front.local_addr().expect("relay addr");
        let counters = Arc::new(Counters::default());
        let blackhole = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let client: Arc<Mutex<Option<SocketAddr>>> = Arc::new(Mutex::new(None));

        let mut threads = Vec::new();
        // Client → server.
        {
            let front = front.try_clone().expect("clone");
            let back = back.try_clone().expect("clone");
            let client = client.clone();
            let stop = stop.clone();
            let mut direction = Direction {
                config,
                rng: SplitMix64(config.seed ^ 0x0C11_E47A),
                block: Vec::new(),
                counters: counters.clone(),
                blackhole: blackhole.clone(),
            };
            threads.push(std::thread::spawn(move || {
                let mut buf = vec![0u8; 65_536];
                let mut send = |packet: &[u8]| {
                    let _ = back.send_to(packet, server);
                };
                while !stop.load(Ordering::SeqCst) {
                    match recv_burst(&front, &mut buf, direction.holding()) {
                        Ok((n, from)) => {
                            if let Ok(mut known) = client.lock() {
                                *known = Some(from);
                            }
                            direction.offer(buf[..n].to_vec(), &mut send);
                        }
                        Err(_) => direction.release(&mut send),
                    }
                }
            }));
        }
        // Server → client.
        {
            let stop = stop.clone();
            let mut direction = Direction {
                config,
                rng: SplitMix64(config.seed ^ 0x5E4F_E4D0),
                block: Vec::new(),
                counters: counters.clone(),
                blackhole: blackhole.clone(),
            };
            threads.push(std::thread::spawn(move || {
                let mut buf = vec![0u8; 65_536];
                let mut send = |packet: &[u8]| {
                    let target = client.lock().ok().and_then(|known| *known);
                    if let Some(target) = target {
                        let _ = front.send_to(packet, target);
                    }
                };
                while !stop.load(Ordering::SeqCst) {
                    match recv_burst(&back, &mut buf, direction.holding()) {
                        Ok((n, _)) => direction.offer(buf[..n].to_vec(), &mut send),
                        Err(_) => direction.release(&mut send),
                    }
                }
            }));
        }
        UdpRelay {
            addr,
            counters,
            blackhole,
            stop,
            threads,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn forwarded(&self) -> u64 {
        self.counters.forwarded.load(Ordering::SeqCst)
    }

    pub fn dropped(&self) -> u64 {
        self.counters.dropped.load(Ordering::SeqCst)
    }

    pub fn reordered(&self) -> u64 {
        self.counters.reordered.load(Ordering::SeqCst)
    }

    pub fn set_blackhole(&self, on: bool) {
        self.blackhole.store(on, Ordering::SeqCst);
    }
}

impl Drop for UdpRelay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
