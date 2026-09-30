//! T030 impairment probe: real UDP/QUIC over loopback through a seeded
//! impairment relay, measuring injected versus observed loss.
//!
//! ```text
//! cargo run --release --locked -- --mode quic|udp --profile clean|reorder|impaired \
//!     --rate <datagrams/s> --seconds <s> --payload <bytes> --seed <u64> --label <text>
//! ```
//!
//! Profiles (applied independently to each direction by the relay):
//! - `clean`: forward immediately, no loss.
//! - `reorder`: 75 ms +/- 15 ms one-way delay (150 ms nominal RTT), no loss.
//! - `impaired`: `reorder` plus 3% scheduled loss.
//!
//! Loss is *scheduled*: the i-th datagram arriving in a direction is dropped
//! iff the direction's SplitMix64 stream (seeded from `--seed`) draws < 30/1000
//! at step i, so the injected count is exact and reported. Delay per datagram
//! is drawn from the same stream. Real network timing is an observation with
//! provenance, not a deterministic replay.
//!
//! Attribution reported per run:
//! - injected: datagrams the relay dropped on purpose;
//! - socket/kernel: relay-forwarded client->server datagrams minus the server
//!   connection's `udp_rx.datagrams`;
//! - window accounting: `quinn_proto` "discarding possible duplicate packet"
//!   events, counted by an in-process tracing subscriber (both endpoints);
//! - application progress: unique datagram sequence numbers the server app read.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::time::Instant;

// ---------------------------------------------------------------- arguments

struct Args {
    mode: String,
    profile: String,
    rate: u64,
    seconds: u64,
    payload: usize,
    seed: u64,
    label: String,
}

fn parse_args() -> Args {
    let mut args = Args {
        mode: "quic".into(),
        profile: "impaired".into(),
        rate: 30,
        seconds: 20,
        payload: 1000,
        seed: 1,
        label: "unlabelled".into(),
    };
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < raw.len() {
        let value = raw.get(i + 1).cloned().expect("every option takes a value");
        match raw[i].as_str() {
            "--mode" => args.mode = value,
            "--profile" => args.profile = value,
            "--rate" => args.rate = value.parse().expect("rate"),
            "--seconds" => args.seconds = value.parse().expect("seconds"),
            "--payload" => args.payload = value.parse().expect("payload"),
            "--seed" => args.seed = value.parse().expect("seed"),
            "--label" => args.label = value,
            other => panic!("unknown option {other}"),
        }
        i += 2;
    }
    assert!(args.payload >= 8, "payload carries an 8-byte sequence number");
    args
}

// ------------------------------------------------------------- impairment

#[derive(Clone, Copy)]
struct Impairment {
    base_ms: u64,
    jitter_ms: u64,
    loss_per_mille: u64,
}

fn impairment(profile: &str) -> Impairment {
    match profile {
        "clean" => Impairment { base_ms: 0, jitter_ms: 0, loss_per_mille: 0 },
        "reorder" => Impairment { base_ms: 75, jitter_ms: 15, loss_per_mille: 0 },
        "impaired" => Impairment { base_ms: 75, jitter_ms: 15, loss_per_mille: 30 },
        other => panic!("unknown profile {other}"),
    }
}

struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[derive(Default, Clone, Copy)]
struct DirStats {
    arrived: u64,
    dropped: u64,
    forwarded: u64,
    large_arrived: u64,
    large_dropped: u64,
    large_forwarded: u64,
    /// Forwarded datagrams that left after a later-arriving one.
    inversions: u64,
    /// Largest (highest arrival index forwarded so far - this arrival index).
    max_depth: u64,
    highest_forwarded: Option<u64>,
}

impl DirStats {
    fn json(&self) -> String {
        format!(
            "{{\"arrived\":{},\"dropped\":{},\"forwarded\":{},\"large_arrived\":{},\"large_dropped\":{},\"large_forwarded\":{},\"inversions\":{},\"max_reorder_depth\":{}}}",
            self.arrived,
            self.dropped,
            self.forwarded,
            self.large_arrived,
            self.large_dropped,
            self.large_forwarded,
            self.inversions,
            self.max_depth
        )
    }
}

type Peer = Arc<Mutex<Option<SocketAddr>>>;

/// Relays datagrams arriving on `from` to `to` -> `destination`, applying
/// the scheduled impairment. `learn` records the sender address for the
/// reverse direction.
async fn relay(
    from: Arc<UdpSocket>,
    to: Arc<UdpSocket>,
    destination: Peer,
    learn: Option<Peer>,
    imp: Impairment,
    seed: u64,
    large: usize,
    stats: Arc<Mutex<DirStats>>,
) {
    let mut rng = SplitMix(seed);
    let mut buf = vec![0u8; 65_536];
    let mut index: u64 = 0;
    loop {
        let (len, sender) = match from.recv_from(&mut buf).await {
            Ok(received) => received,
            Err(_) => continue,
        };
        if let Some(learn) = &learn {
            *learn.lock().unwrap() = Some(sender);
        }
        let this = index;
        index += 1;
        let is_large = len >= large;
        let drop_draw = rng.next() % 1000;
        let jitter_draw = rng.next();
        {
            let mut s = stats.lock().unwrap();
            s.arrived += 1;
            s.large_arrived += u64::from(is_large);
            if drop_draw < imp.loss_per_mille {
                s.dropped += 1;
                s.large_dropped += u64::from(is_large);
                continue;
            }
        }
        let mut delay_us = imp.base_ms * 1000;
        if imp.jitter_ms > 0 {
            let span = 2 * imp.jitter_ms * 1000 + 1;
            delay_us = delay_us + jitter_draw % span - imp.jitter_ms * 1000;
        }
        let data = buf[..len].to_vec();
        let to = Arc::clone(&to);
        let destination = Arc::clone(&destination);
        let stats = Arc::clone(&stats);
        let deliver_at = Instant::now() + Duration::from_micros(delay_us);
        tokio::spawn(async move {
            tokio::time::sleep_until(deliver_at).await;
            let target = *destination.lock().unwrap();
            if let Some(target) = target {
                if to.send_to(&data, target).await.is_ok() {
                    let mut s = stats.lock().unwrap();
                    s.forwarded += 1;
                    s.large_forwarded += u64::from(is_large);
                    match s.highest_forwarded {
                        Some(highest) if highest > this => {
                            s.inversions += 1;
                            s.max_depth = s.max_depth.max(highest - this);
                        }
                        _ => s.highest_forwarded = Some(this),
                    }
                }
            }
        });
    }
}

struct Relay {
    client_facing: SocketAddr,
    up: Arc<Mutex<DirStats>>,
    down: Arc<Mutex<DirStats>>,
}

async fn start_relay(server: SocketAddr, imp: Impairment, seed: u64, large: usize) -> Relay {
    let a = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind a"));
    let b = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind b"));
    let client_facing = a.local_addr().unwrap();
    let client: Peer = Arc::new(Mutex::new(None));
    let server_peer: Peer = Arc::new(Mutex::new(Some(server)));
    let up = Arc::new(Mutex::new(DirStats::default()));
    let down = Arc::new(Mutex::new(DirStats::default()));
    tokio::spawn(relay(
        Arc::clone(&a),
        Arc::clone(&b),
        server_peer,
        Some(Arc::clone(&client)),
        imp,
        seed ^ 0x5550,
        large,
        Arc::clone(&up),
    ));
    tokio::spawn(relay(b, a, client, None, imp, seed ^ 0xD0_4E, large, Arc::clone(&down)));
    Relay {
        client_facing,
        up,
        down,
    }
}

// ------------------------------------------------- duplicate-discard counter

/// Counts `quinn_proto` "discarding possible duplicate packet" events.
struct DupCounter {
    discards: AtomicU64,
    next_span: AtomicU64,
}

static COUNTER: DupCounter = DupCounter {
    discards: AtomicU64::new(0),
    next_span: AtomicU64::new(1),
};

struct MessageMatch(bool);

impl tracing::field::Visit for MessageMatch {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" && format!("{value:?}") == "discarding possible duplicate packet" {
            self.0 = true;
        }
    }
}

struct CounterSubscriber;

impl tracing::Subscriber for CounterSubscriber {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target().starts_with("quinn_proto") && *metadata.level() <= tracing::Level::DEBUG
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(COUNTER.next_span.fetch_add(1, Ordering::Relaxed))
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut found = MessageMatch(false);
        event.record(&mut found);
        if found.0 {
            COUNTER.discards.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

// ------------------------------------------------------------------ helpers

fn payload(seq: u64, size: usize) -> Bytes {
    let mut data = vec![0xA5u8; size];
    data[..8].copy_from_slice(&seq.to_le_bytes());
    Bytes::from(data)
}

fn seq_of(data: &[u8]) -> Option<u64> {
    data.get(..8).map(|bytes| u64::from_le_bytes(bytes.try_into().unwrap()))
}

#[derive(Default)]
struct Received {
    unique: BTreeSet<u64>,
    duplicates: u64,
    highest: Option<u64>,
    late: u64,
}

impl Received {
    fn record(&mut self, seq: u64) {
        if !self.unique.insert(seq) {
            self.duplicates += 1;
        }
        match self.highest {
            Some(highest) if highest > seq => self.late += 1,
            _ => self.highest = Some(seq),
        }
    }
}

// -------------------------------------------------------------------- modes

async fn run_udp(args: &Args, imp: Impairment) -> String {
    let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let relay = start_relay(receiver.local_addr().unwrap(), imp, args.seed, args.payload).await;
    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let received = Arc::new(Mutex::new(Received::default()));
    let sink = Arc::clone(&received);
    tokio::spawn(async move {
        let mut buf = vec![0u8; 65_536];
        while let Ok((len, _)) = receiver.recv_from(&mut buf).await {
            if let Some(seq) = seq_of(&buf[..len]) {
                sink.lock().unwrap().record(seq);
            }
        }
    });
    let total = args.rate * args.seconds;
    let mut ticker = tokio::time::interval(Duration::from_micros(1_000_000 / args.rate));
    for seq in 0..total {
        ticker.tick().await;
        sender
            .send_to(&payload(seq, args.payload), relay.client_facing)
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let r = received.lock().unwrap();
    let up = *relay.up.lock().unwrap();
    format!(
        "{{\"mode\":\"udp\",\"label\":\"{}\",\"profile\":\"{}\",\"rate\":{},\"seconds\":{},\"payload\":{},\"seed\":{},\"app_sent\":{},\"relay_up\":{},\"app_received_unique\":{},\"app_duplicates\":{},\"app_late\":{},\"socket_loss\":{}}}",
        args.label,
        args.profile,
        args.rate,
        args.seconds,
        args.payload,
        args.seed,
        total,
        up.json(),
        r.unique.len(),
        r.duplicates,
        r.late,
        up.forwarded as i64 - r.unique.len() as i64 - r.duplicates as i64
    )
}

async fn run_quic(args: &Args, imp: Impairment) -> String {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let cert_der = rustls::pki_types::CertificateDer::from(certified.cert.der().to_vec());
    let key_der =
        rustls::pki_types::PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der());

    let mut transport = quinn::TransportConfig::default();
    transport.datagram_receive_buffer_size(Some(64 * 1024 * 1024));
    transport.datagram_send_buffer_size(16 * 1024 * 1024);
    let transport = Arc::new(transport);

    let mut server_config =
        quinn::ServerConfig::with_single_cert(vec![cert_der.clone()], key_der.into()).unwrap();
    server_config.transport_config(Arc::clone(&transport));
    let server = quinn::Endpoint::server(server_config, "127.0.0.1:0".parse().unwrap()).unwrap();
    let relay = start_relay(server.local_addr().unwrap(), imp, args.seed, args.payload).await;

    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert_der).unwrap();
    let mut client_config = quinn::ClientConfig::with_root_certificates(Arc::new(roots)).unwrap();
    client_config.transport_config(transport);
    let mut client = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
    client.set_default_client_config(client_config);

    let received = Arc::new(Mutex::new(Received::default()));
    let sink = Arc::clone(&received);
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let incoming = server.accept().await.expect("incoming");
        let connection = incoming.await.expect("handshake");
        let _ = tx.send(connection.clone());
        while let Ok(data) = connection.read_datagram().await {
            if let Some(seq) = seq_of(&data) {
                sink.lock().unwrap().record(seq);
            }
        }
    });

    let connection = client
        .connect(relay.client_facing, "localhost")
        .unwrap()
        .await
        .expect("client handshake");
    let server_connection = rx.await.expect("server connection");
    let discards_before = COUNTER.discards.load(Ordering::Relaxed);

    let total = args.rate * args.seconds;
    let mut send_errors = 0u64;
    let mut ticker = tokio::time::interval(Duration::from_micros(1_000_000 / args.rate));
    for seq in 0..total {
        ticker.tick().await;
        if connection.send_datagram(payload(seq, args.payload)).is_err() {
            send_errors += 1;
        }
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let client_stats = connection.stats();
    let server_stats = server_connection.stats();
    let discards = COUNTER.discards.load(Ordering::Relaxed) - discards_before;
    connection.close(0u32.into(), b"done");
    let r = received.lock().unwrap();
    let up = *relay.up.lock().unwrap();
    let down = *relay.down.lock().unwrap();
    format!(
        "{{\"mode\":\"quic\",\"label\":\"{}\",\"profile\":\"{}\",\"rate\":{},\"seconds\":{},\"payload\":{},\"seed\":{},\"app_sent\":{},\"send_errors\":{},\"client_datagram_frames_tx\":{},\"client_udp_tx\":{},\"client_lost_packets\":{},\"relay_up\":{},\"relay_down\":{},\"server_udp_rx\":{},\"server_datagram_frames_rx\":{},\"app_received_unique\":{},\"app_duplicates\":{},\"app_late\":{},\"dedup_discards_both_endpoints\":{},\"socket_loss_up\":{}}}",
        args.label,
        args.profile,
        args.rate,
        args.seconds,
        args.payload,
        args.seed,
        total,
        send_errors,
        client_stats.frame_tx.datagram,
        client_stats.udp_tx.datagrams,
        client_stats.path.lost_packets,
        up.json(),
        down.json(),
        server_stats.udp_rx.datagrams,
        server_stats.frame_rx.datagram,
        r.unique.len(),
        r.duplicates,
        r.late,
        discards,
        up.forwarded as i64 - server_stats.udp_rx.datagrams as i64
    )
}

fn main() {
    tracing::subscriber::set_global_default(CounterSubscriber).expect("subscriber");
    let args = parse_args();
    let imp = impairment(&args.profile);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let line = runtime.block_on(async {
        match args.mode.as_str() {
            "udp" => run_udp(&args, imp).await,
            "quic" => run_quic(&args, imp).await,
            other => panic!("unknown mode {other}"),
        }
    });
    println!("{line}");
}
