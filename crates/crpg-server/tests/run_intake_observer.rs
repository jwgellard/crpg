//! T040a acceptance suite (`tasks/T040.md` T1–T5, B-1–B-3): the rejections a
//! pump report names and the intake observer a `QuicHosting` calls.
//!
//! Public API only, over real `crpg-net-quic` loopback (the rig in
//! `tests/support/quic_host_rig.rs`). Expected values come from the contract
//! (B-1, B-2, B-3) and from the trace the adapter itself reports; every wait
//! is a guard that panics `"deadline"` after 30 s of real time.

#[path = "support/quic_host_rig.rs"]
mod rig;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crpg_net::protocol::{ReceiptStatus, RejectionCode};
use crpg_net_quic::{
    ClientLimits, CloseCode, ConnectError, Framing, Hello, QuicClient, QuicServer, ServerConfig,
    ServerLimits,
};
use crpg_server::host::{IngestDisposition, PeerHandle, ProtocolSelection};
use crpg_server::quic::{
    HelloOutcome, HostCall, QuicHost, QuicPumpReport, Rejection, STALL_TIMEOUT_MS,
};
use crpg_server::run::{HostLoop, IntakeObserver, QuicHosting};
use rig::*;

/// The credential the invitation holds: 32 bytes of `0x11`.
fn x() -> Vec<u8> {
    vec![0x11; 32]
}

/// A credential no invitation holds: 32 bytes of `0x22`.
fn y() -> Vec<u8> {
    vec![0x22; 32]
}

/// A V2 host started resumable at 1,000 with invitation `X` registered.
fn start_resumable_x() -> (QuicHost, TestClock) {
    let clock = TestClock::new();
    let mut qh = QuicHost::start_resumable(
        new_host(V2),
        QuicServer::bind_resumable(ServerConfig {
            bind: loopback(),
            identity: identity(),
            limits: ServerLimits::v1(),
        })
        .expect("bind resumable server"),
        clock.now(),
    )
    .expect("starts over a fresh host");
    qh.register_invitation(invitation(&x(), all_actors(), full_grants()))
        .expect("registers X");
    (qh, clock)
}

type Connecting = std::thread::JoinHandle<Result<QuicClient, ConnectError>>;

/// A `/2` hello with `credential_bytes` on its own thread, so the caller pumps.
fn hello_async(server: SocketAddr, credential_bytes: &[u8]) -> Connecting {
    let config = client_config(ClientLimits::v1());
    let hello = Hello {
        wire_version: version_byte(V2),
        credential: credential(credential_bytes),
    };
    std::thread::spawn(move || QuicClient::connect_framed(config, server, &hello, Framing::V2))
}

/// Pumps until a hello is decided; returns that pump's report.
fn pump_hello(qh: &mut QuicHost, clock: &mut TestClock) -> QuicPumpReport {
    let steps = pump_until(qh, clock, 100, "hello decided", |_, report| {
        !report.hellos.is_empty()
    });
    steps.into_iter().last().expect("one pump").1
}

/// Connects an accepted `X` client; returns it, its connection, its binding
/// and its epoch.
fn connect_x(
    qh: &mut QuicHost,
    clock: &mut TestClock,
) -> (
    QuicClient,
    crpg_net_quic::ConnectionId,
    PeerHandle,
    [u8; 16],
) {
    let pending = hello_async(qh.local_addr(), &x());
    let report = pump_hello(qh, clock);
    assert_eq!(report.hellos.len(), 1, "{:?}", report.hellos);
    let HelloOutcome::Accepted { peer, .. } = report.hellos[0].outcome else {
        panic!("expected Accepted, got {:?}", report.hellos[0]);
    };
    assert!(
        report.rejections().is_empty(),
        "an accepted hello is no rejection"
    );
    let client = pending.join().expect("client thread").expect("connect");
    let epoch = qh.host().epoch(peer).expect("bound");
    (client, report.hellos[0].conn, peer, epoch)
}

// ---------------------------------------------------------------------------
// T1.
// ---------------------------------------------------------------------------

/// T1 (B-1, B-2): a refused hello is one `Hello` rejection with the close
/// code and B-2's text; the control, an accepted hello, lists nothing.
#[test]
fn refused_hello_is_one_rejection() {
    let (mut qh, mut clock) = start_resumable_x();
    let pending = hello_async(qh.local_addr(), &y());
    let report = pump_hello(&mut qh, &mut clock);
    assert_eq!(report.hellos.len(), 1, "{:?}", report.hellos);
    let conn = report.hellos[0].conn;
    assert_eq!(
        report.hellos[0].outcome,
        HelloOutcome::Refused {
            code: CloseCode::AuthRefused
        }
    );
    let found = report.rejections();
    assert_eq!(
        found,
        vec![Rejection::Hello {
            conn,
            code: CloseCode::AuthRefused
        }]
    );
    assert_eq!(
        found[0].to_string(),
        format!("hello conn={} code=5", conn.get())
    );
    match pending.join().expect("client thread") {
        Err(ConnectError::Refused { code: 5 }) => {}
        Err(other) => panic!("expected a refusal with code 5, got {other:?}"),
        Ok(_) => panic!("expected a refusal, got a connection"),
    }

    // Control: the same adapter accepts `X`, and that report lists nothing.
    let (_client, _conn, _peer, _epoch) = connect_x(&mut qh, &mut clock);
}

// ---------------------------------------------------------------------------
// T3a and T3b (erratum T040-E1).
// ---------------------------------------------------------------------------

/// Pumps (+100 ms each) until `cond` holds for the latest report; returns it.
fn pump_report(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    what: &str,
    cond: impl FnMut(&QuicHost, &QuicPumpReport) -> bool,
) -> QuicPumpReport {
    pump_until(qh, clock, 100, what, cond)
        .into_iter()
        .last()
        .expect("one pump")
        .1
}

/// T3a (B-1, B-2): the one-byte frame `[0xFF]` is staged by `ingest` (which
/// never decodes), and the pump that admits it rejects it as `UnsupportedVersion`:
/// exactly one `Admission` rejection and no `Ingest` rejection. The control, a
/// valid `EndTurn` frame, adds no rejection.
#[test]
fn bad_frame_is_one_admission_rejection() {
    let (mut qh, mut clock) = start_resumable_x();
    let (mut client, conn, peer, epoch) = connect_x(&mut qh, &mut clock);

    client.try_send(&[0xFF]).expect("client send");
    let report = pump_report(&mut qh, &mut clock, "bad frame ingested", |_, report| {
        report
            .calls
            .iter()
            .any(|call| matches!(call, HostCall::Ingest { len: 1, .. }))
    });
    let ingests: Vec<&HostCall> = report
        .calls
        .iter()
        .filter(|call| matches!(call, HostCall::Ingest { len: 1, .. }))
        .collect();
    assert_eq!(ingests.len(), 1, "exactly one Ingest call: {ingests:?}");
    assert!(
        matches!(
            ingests[0],
            HostCall::Ingest {
                conn: c,
                peer: p,
                result: IngestDisposition::Staged,
                ..
            } if *c == conn && *p == peer
        ),
        "ingest stages the undecodable frame: {:?}",
        ingests[0]
    );
    let found = report.rejections();
    let listed = Rejection::Admission {
        peer,
        seq: None,
        code: RejectionCode::UnsupportedVersion,
        cached: false,
    };
    assert_eq!(found, vec![listed.clone()], "one admission, no ingest");
    assert_eq!(
        listed.to_string(),
        format!("admission peer={} seq=- code=3 cached=false", peer.get())
    );

    // Control: a valid frame adds no rejection.
    let valid = end_turn_for(version_byte(V2), epoch, 1);
    client.try_send(&valid).expect("client send");
    let report = pump_report(&mut qh, &mut clock, "valid frame ingested", |_, r| {
        r.calls
            .iter()
            .any(|call| matches!(call, HostCall::Ingest { len, .. } if *len == valid.len()))
    });
    assert!(report.calls.iter().any(|call| matches!(
        call,
        HostCall::Ingest {
            len,
            result: IngestDisposition::Staged,
            ..
        } if *len == valid.len()
    )));
    assert_eq!(
        report.rejections(),
        Vec::new(),
        "a valid frame is no rejection"
    );
}

/// T3b (B-1, B-2): valid-size frames at one fixed clock spend the per-peer
/// rate budget until `ingest` refuses with `RateLimited`; every such `Ingest`
/// call gives exactly one `Ingest` rejection built from it, and no
/// `QueueFull` call gives one. The control is T3a's: a single valid frame adds
/// none (checked here on the first batch, before the budget is spent).
#[test]
fn rate_limited_frame_is_one_ingest_rejection() {
    const BATCH: usize = 20;
    // burst_frames (80, T023b A§2.2) + per_peer_frames (128) + 1.
    const MAX_FRAMES: usize = 80 + 128 + 1;
    let (mut qh, mut clock) = start_resumable_x();
    let (mut client, conn, peer, epoch) = connect_x(&mut qh, &mut clock);
    let now = clock.now();
    let version = version_byte(V2);

    let mut sent = 0usize;
    let mut limited = 0usize;
    while limited == 0 {
        assert!(sent < MAX_FRAMES, "the budget is spent within the bound");
        for _ in 0..BATCH {
            sent += 1;
            client
                .try_send(&end_turn_for(version, epoch, sent as u64))
                .expect("client send");
        }
        until("the batch is queued at the server", || {
            qh.connection_stats(conn)
                .is_some_and(|stats| stats.inbound_frames == BATCH)
        });
        let report = qh.pump(now).expect("pump");
        let mut expected = Vec::new();
        for call in &report.calls {
            let HostCall::Ingest {
                conn: c,
                peer: p,
                frame,
                len,
                result:
                    IngestDisposition::Refused {
                        status: ReceiptStatus::Rejected(code),
                    },
            } = call
            else {
                continue;
            };
            if *code == RejectionCode::RateLimited {
                expected.push(Rejection::Ingest {
                    conn: *c,
                    peer: *p,
                    frame: *frame,
                    len: *len,
                    code: *code,
                });
            }
        }
        let listed: Vec<Rejection> = report
            .rejections()
            .into_iter()
            .filter(|r| matches!(r, Rejection::Ingest { .. }))
            .collect();
        assert_eq!(listed, expected, "every RateLimited call, once, in order");
        if sent == BATCH {
            assert!(
                expected.is_empty(),
                "the first batch is inside the burst: {expected:?}"
            );
        }
        limited += expected.len();
        if let Some(first) = listed.first() {
            let Rejection::Ingest {
                conn: listed_conn,
                peer: listed_peer,
                frame,
                len,
                code,
            } = first
            else {
                unreachable!("filtered to Ingest");
            };
            assert_eq!(
                (*listed_conn, *listed_peer, *code),
                (conn, peer, RejectionCode::RateLimited)
            );
            assert_eq!(
                first.to_string(),
                format!(
                    "ingest conn={} peer={} frame={} len={} code={}",
                    conn.get(),
                    peer.get(),
                    frame,
                    len,
                    RejectionCode::RateLimited as u8
                )
            );
        }
    }
    assert!(sent <= MAX_FRAMES, "refused within the bound");
}

// ---------------------------------------------------------------------------
// T4 (the stall set-up and fill are copied from `tests/host_quic.rs`).
// ---------------------------------------------------------------------------

/// A (all actors, full grants, reading, connected first) and B (no
/// control, observer grants, stalled client, not reading).
fn stall_setup(protocol: ProtocolSelection) -> (QuicHost, TestClock, Client, Client) {
    let (mut qh, mut clock) = start(protocol, stall_server_limits());
    qh.register_invitation(invitation(&cred_bytes(0), all_actors(), full_grants()))
        .expect("registers");
    qh.register_invitation(invitation(&cred_bytes(1), control(&[]), observer_grants()))
        .expect("registers");
    let addr = qh.local_addr();
    let (mut ca, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        protocol,
        &cred_bytes(0),
    );
    let (cb, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        stalled_client_limits(),
        protocol,
        &cred_bytes(1),
    );
    assert!(ca.conn < cb.conn, "A is drained first");
    ca.read_until("A initial state", |c| !c.frames.is_empty());
    (qh, clock, ca, cb)
}

/// The A§3.4 fill: c-pumps until `stop`, tracking B's stall with the mirror.
struct Fill {
    seq: u64,
    mirror: StallMirror,
    /// `(c-pump, step)`.
    steps: Vec<(u64, Step)>,
}

impl Fill {
    fn new() -> Self {
        Self {
            seq: 1,
            mirror: StallMirror::new(),
            steps: Vec::new(),
        }
    }

    fn run(
        &mut self,
        qh: &mut QuicHost,
        clock: &mut TestClock,
        ca: &mut Client,
        b: PeerHandle,
        mut stop: impl FnMut(&QuicPumpReport) -> bool,
    ) {
        loop {
            let index = self.steps.len() as u64 + 1;
            assert!(index <= 2_000, "the fill reaches its stop");
            let step = drive_one(qh, clock, ca, &mut self.seq, PACE);
            self.mirror.feed(step.0, &step.1, b);
            if let Some(since) = self.mirror.since {
                assert!(
                    step.0 - since < STALL_TIMEOUT_MS,
                    "no slow-consumer timer yet"
                );
            }
            assert!(step.1.fences.is_empty());
            let done = stop(&step.1);
            self.steps.push((index, step));
            if done {
                return;
            }
        }
    }
}

/// T4 (B-1): in the pump where A's frame 3,025 is held on `QueueFull`,
/// `rejections()` lists no `Ingest` for it. The control flips the one input:
/// the same call answered `Malformed` is listed.
#[test]
fn queue_full_is_not_a_rejection() {
    let (mut qh, mut clock, mut ca, cb) = stall_setup(V1);
    let mut fill = Fill::new();
    fill.run(&mut qh, &mut clock, &mut ca, cb.peer, |report| {
        report.held == 1
    });
    let (_, (_, held_step)) = fill.steps.last().expect("steps").clone();
    let held_call = HostCall::Ingest {
        conn: ca.conn,
        peer: ca.peer,
        frame: 3_025,
        len: ca.sent[3_024].len(),
        result: IngestDisposition::Refused {
            status: ReceiptStatus::Rejected(RejectionCode::QueueFull),
        },
    };
    assert!(
        held_step.calls.contains(&held_call),
        "the held pump traces the QueueFull refusal of frame 3,025"
    );
    assert!(
        !held_step
            .rejections()
            .iter()
            .any(|r| matches!(r, Rejection::Ingest { frame: 3_025, .. })),
        "{:?}",
        held_step.rejections()
    );

    // Control: the same call refused with a terminal code is a rejection.
    let mut flipped = held_step.clone();
    for call in &mut flipped.calls {
        if *call == held_call {
            *call = HostCall::Ingest {
                conn: ca.conn,
                peer: ca.peer,
                frame: 3_025,
                len: ca.sent[3_024].len(),
                result: IngestDisposition::Refused {
                    status: ReceiptStatus::Rejected(RejectionCode::Malformed),
                },
            };
        }
    }
    assert!(flipped.rejections().contains(&Rejection::Ingest {
        conn: ca.conn,
        peer: ca.peer,
        frame: 3_025,
        len: ca.sent[3_024].len(),
        code: RejectionCode::Malformed,
    }));
}

// ---------------------------------------------------------------------------
// T5.
// ---------------------------------------------------------------------------

/// What the T5 scenario saw: the observer's record and the authority hash
/// after each of the four steps.
struct Seen {
    record: Vec<(u64, usize)>,
    hashes: Vec<[u8; 32]>,
}

/// T1's host in a `QuicHosting`, a loop stepped by hand at 1,000, 1,050 and
/// 1,100 with the `Y` hello arriving before the second step, then, with the
/// observer cleared, 1,150.
fn scenario(observe: bool) -> Seen {
    let (qh, _clock) = start_resumable_x();
    let record: Arc<Mutex<Vec<(u64, usize)>>> = Arc::new(Mutex::new(Vec::new()));
    let mut hosting = QuicHosting::new(qh);
    if observe {
        let sink = Arc::clone(&record);
        let observer: IntakeObserver = Box::new(move |now_ms, report| {
            sink.lock()
                .expect("not poisoned")
                .push((now_ms, report.hellos.len()));
        });
        hosting.set_intake_observer(Some(observer));
    }
    let addr = hosting.quic().local_addr();
    let mut host_loop = HostLoop::new(hosting, 1_000);
    let mut hashes = Vec::new();

    host_loop.step(1_000).expect("steps");
    hashes.push(host_loop.hosting().quic().host().authority_hash());

    let pending = hello_async(addr, &y());
    let start = Instant::now();
    while !host_loop.hosting().quic().wait(Duration::from_millis(5)) {
        assert!(start.elapsed() < DEADLINE, "deadline: hello arrives");
    }
    host_loop.step(1_050).expect("steps");
    hashes.push(host_loop.hosting().quic().host().authority_hash());
    host_loop.step(1_100).expect("steps");
    hashes.push(host_loop.hosting().quic().host().authority_hash());
    match pending.join().expect("client thread") {
        Err(ConnectError::Refused { code: 5 }) => {}
        other => panic!("expected the Y hello refused with code 5, got {other:?}"),
    }

    if observe {
        let seen = record.lock().expect("not poisoned").len();
        assert_eq!(seen, 3, "three intakes seen so far");
    }
    host_loop.hosting_mut().set_intake_observer(None);
    host_loop.step(1_150).expect("steps");
    hashes.push(host_loop.hosting().quic().host().authority_hash());

    let record = record.lock().expect("not poisoned").clone();
    Seen { record, hashes }
}

/// T5 (B-3): the observer sees every intake exactly once with its `now_ms`
/// and report, none after it is cleared, and changes no host state (the
/// control: the same loop with no observer has the same authority hash after
/// each step).
#[test]
fn observer_sees_every_intake_once() {
    let observed = scenario(true);
    let times: Vec<u64> = observed.record.iter().map(|(now, _)| *now).collect();
    assert_eq!(times, vec![1_000, 1_050, 1_100], "{:?}", observed.record);
    let hellos: Vec<usize> = observed.record.iter().map(|(_, n)| *n).collect();
    assert_eq!(hellos.iter().sum::<usize>(), 1, "the refused hello once");
    assert_eq!(
        hellos.iter().filter(|n| **n == 1).count(),
        1,
        "in exactly one intake: {:?}",
        observed.record
    );
    assert_eq!(
        observed.record.len(),
        3,
        "cleared: the 1,150 step adds none"
    );

    let control = scenario(false);
    assert!(control.record.is_empty(), "no observer, no record");
    assert_eq!(
        observed.hashes, control.hashes,
        "the observer changes no authority state"
    );
}
