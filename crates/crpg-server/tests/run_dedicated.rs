//! T154d acceptance suite (`tasks/T154.md` Q1–Q5): the dedicated hosting —
//! `QuicHost::flush` between pumps, the `drive_ai`/`pause`/`unpause`
//! pass-throughs, and a single-player save running on the dedicated loop.
//!
//! Public API only, over real `crpg-net-quic` loopback (the rig in
//! `tests/support/quic_host_rig.rs`). Expected values come from the
//! contract: the frames `flush` sends are the ones a twin direct `Host`
//! stages, and the save-into-dedicated hashes are compared between two
//! independent loads in one process. Every wait is a guard that panics
//! `"deadline"` after 30 s of real time.

#[path = "support/quic_host_rig.rs"]
mod rig;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crpg_ai::{SideId, Sides};
use crpg_core::Ulid;
use crpg_net::codec_v2;
use crpg_net::protocol::{IntentBody, IntentFrame, LANE_COMBAT};
use crpg_net::protocol_v3::PauseReason;
use crpg_net::transport::Transport;
use crpg_net_quic::{
    Channel, ClientLimits, CloseReason, ConnectError, Framing, Hello, QuicClient, QuicServer,
    ServerConfig, ServerLimits,
};
use crpg_server::host::{
    DeliveryKind, DisclosureGrants, Host, HostConfig, HostError, PeerHandle, ProtocolSelection,
    MVP_AI_BUDGET,
};
use crpg_server::quic::{
    GrantChange, HelloOutcome, HostCall, InvitationId, QuicHost, QuicHostError,
};
use crpg_server::run::{HostLoop, LocalHost, LoopError, QuicHosting};
use crpg_server::runner::{spawn, SystemClock};
use crpg_server::save::{load_host_file, save_host_file, SaveIdentity};
use rig::*;

const V2: ProtocolSelection = ProtocolSelection::V2;

// ---------------------------------------------------------------------------
// The contract's `Ident`, `TB3` and `end_frame` (T154 §Tests helpers).
// ---------------------------------------------------------------------------

fn ident() -> SaveIdentity {
    SaveIdentity {
        campaign_id: Ulid::from_u128(1),
        campaign_version: "1.0.0".into(),
    }
}

fn config() -> HostConfig {
    HostConfig {
        protocol: V2,
        incarnation: 1,
    }
}

/// `TB3`: the fixture as is (A then B then C, A active) at `now`.
fn tb3(now: u64) -> Host {
    Host::from_history(fixture(), config(), now).expect("wraps an acknowledged authority")
}

fn end_frame(host: &Host, peer: PeerHandle, seq: u64) -> Vec<u8> {
    let frame = IntentFrame {
        epoch: host.epoch(peer).expect("bound"),
        lane: LANE_COMBAT,
        seq,
        observed_tick: host.server_tick(),
        actor: n(1),
        body: IntentBody::EndTurn,
    };
    codec_v2::encode_intent(&frame).expect("encodes")
}

// ---------------------------------------------------------------------------
// Loopback servers and `/2` clients.
// ---------------------------------------------------------------------------

fn bind_dual() -> QuicServer {
    QuicServer::bind_dual(ServerConfig {
        bind: loopback(),
        identity: identity(),
        limits: ServerLimits::v1(),
    })
    .expect("binds a dual loopback server")
}

fn connect_snapshot_async(
    server: std::net::SocketAddr,
    credential_bytes: &[u8],
) -> std::thread::JoinHandle<Result<QuicClient, ConnectError>> {
    let config = client_config(ClientLimits::v1());
    let hello = Hello {
        wire_version: 2,
        credential: credential(credential_bytes),
    };
    std::thread::spawn(move || QuicClient::connect_framed(config, server, &hello, Framing::V2))
}

/// A `/2` client: every frame on its channel, lane-0 deltas decoded so their
/// receipts are observable. A fresh snapshot session's first state is a lane-0
/// delta, not a transfer (only a narrowing stages one).
struct ChannelClient {
    quic: QuicClient,
    peer: PeerHandle,
    epoch: [u8; 16],
    invitation: InvitationId,
    recv: Vec<(Channel, Vec<u8>)>,
    receipts: Vec<u64>,
    closed: Option<CloseReason>,
}

impl ChannelClient {
    fn read_available(&mut self) -> usize {
        let mut count = 0;
        if self.closed.is_some() {
            return 0;
        }
        loop {
            match self.quic.try_recv_channel() {
                Ok(Some((channel, bytes))) => {
                    if channel == Channel::Lane0 {
                        let frame = decode_frame(V2, &bytes);
                        for op in &frame.ops {
                            if let crpg_net::protocol_v2::DeltaOp::Legacy(
                                crpg_net::protocol::DeltaOp::Receipt { seq, .. },
                            ) = op
                            {
                                self.receipts.push(*seq);
                            }
                        }
                    }
                    self.recv.push((channel, bytes));
                    count += 1;
                }
                Ok(None) => break,
                Err(reason) => {
                    self.closed = Some(reason);
                    break;
                }
            }
        }
        count
    }

    fn read_until(&mut self, what: &str, mut cond: impl FnMut(&ChannelClient) -> bool) {
        let start = Instant::now();
        loop {
            self.read_available();
            if cond(self) {
                return;
            }
            assert!(start.elapsed() < DEADLINE, "deadline: {what}");
            self.quic.wait(Duration::from_millis(5));
        }
    }
}

/// Connects a `/2` client, pumping at `now` until the hello is decided.
fn connect_snapshot(qh: &mut QuicHost, now: u64, credential_bytes: &[u8]) -> ChannelClient {
    let pending = connect_snapshot_async(qh.local_addr(), credential_bytes);
    let start = Instant::now();
    let decision = loop {
        let report = qh.pump(now).expect("pump");
        if let Some(decision) = report.hellos.into_iter().next() {
            break decision;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: hello decided");
        qh.wait(Duration::from_millis(5));
    };
    let HelloOutcome::Accepted { invitation, peer } = decision.outcome else {
        panic!("expected Accepted, got {decision:?}");
    };
    let quic = pending.join().expect("client thread").expect("connect");
    assert_eq!(quic.framing(), Framing::V2, "a `/2` connection");
    let epoch = qh.host().epoch(peer).expect("bound");
    ChannelClient {
        quic,
        peer,
        epoch,
        invitation,
        recv: Vec::new(),
        receipts: Vec::new(),
        closed: None,
    }
}

// ---------------------------------------------------------------------------
// Save-into-dedicated helpers Q3, Q4 and Q5 share.
// ---------------------------------------------------------------------------

fn temp_path(case: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("crpg-server")
        .join("run_dedicated")
        .join(case);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temporary directory");
    dir.join("campaign.hsave")
}

/// A `LocalHost` loop on `TB3` with `P` and one `end_frame`: cycles at 0, 50
/// and 100, then `save_host_file`. Returns the save path.
fn embedded_save(case: &str) -> PathBuf {
    let path = temp_path(case);
    let mut host = tb3(0);
    let peer = host
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let mut local = LocalHost::new(host);
    let mut client = local
        .attach(peer, std::num::NonZeroUsize::new(64).expect("nonzero"))
        .expect("attaches");
    let frame = end_frame(local.host(), peer, 1);
    client.send(&frame).expect("queued");
    let mut host_loop = HostLoop::new(local, 0);
    for now in [0, 50, 100] {
        let step = host_loop.step(now).expect("steps");
        assert!(step.cycle.is_some(), "a cycle at {now}");
    }
    assert_eq!(
        host_loop.hosting().host().executions(),
        1,
        "A's EndTurn applied"
    );
    save_host_file(&path, host_loop.hosting().host(), &ident()).expect("quiescent save");
    path
}

/// Loads `path` at `now` under `incarnation` and starts a `QuicHost`.
fn load_dedicated(path: &Path, incarnation: u64, now: u64) -> QuicHost {
    let loaded = load_host_file(path, &ident(), incarnation, now).expect("loads");
    QuicHost::start(loaded.host, bind_dual(), now).expect("starts over a loaded host")
}

// ---------------------------------------------------------------------------
// Q1.
// ---------------------------------------------------------------------------

fn a_only_grants() -> DisclosureGrants {
    let mut grants = full_grants();
    grants.entities.retain(|entry| entry.entity == a());
    grants
}

fn channel_of(kind: DeliveryKind) -> Channel {
    match kind {
        DeliveryKind::Delta => Channel::Lane0,
        DeliveryKind::Transfer => Channel::Transfer,
    }
}

fn host_state(host: &Host) -> ([u8; 32], u64, u64, u64, u64) {
    (
        host.authority_hash(),
        host.server_tick(),
        host.executions(),
        host.last_capture(),
        host.capture_acknowledged(),
    )
}

#[test]
fn q1_flush_delivers_between_pumps() {
    let mut qh = QuicHost::start(tb3(0), bind_dual(), 0).expect("starts");
    let invitation = qh
        .register_invitation(invitation(&cred_bytes(0), all_actors(), a_only_grants()))
        .expect("registers");
    let mut client = connect_snapshot(&mut qh, 1, &cred_bytes(0));
    assert_eq!(client.invitation, invitation);
    // The bind's first state went out in the hello pump's own phases 4–7.
    client.read_until("bind state", |c| !c.recv.is_empty());

    // Widen to all. The twin is the same `/2` bind driven directly: its
    // staged entries after the widening are the oracle.
    let mut twin = tb3(1);
    let twin_peer = twin
        .bind_snapshot_peer(control(&[a(), b(), c()]), a_only_grants(), 1)
        .expect("binds");
    let initial = twin.take_delivery_entries(twin_peer).expect("staged");
    twin.acknowledge_delivery(twin_peer, initial.last().expect("staged").ordinal)
        .expect("acknowledged");
    twin.set_grants(twin_peer, full_grants()).expect("widens");
    let expected = twin.take_delivery_entries(twin_peer).expect("staged");
    assert!(
        !expected.is_empty(),
        "widening from A-only stages the newly permitted state"
    );

    assert_eq!(
        qh.set_invitation_grants(invitation, full_grants()),
        Ok(GrantChange::Widened)
    );
    // Control: with no flush, nothing arrives before the next pump.
    assert_eq!(client.read_available(), 0, "not delivered yet");

    let start = client.recv.len();
    let report = qh.flush(10).expect("flushes");
    assert_eq!(report.frames_sent, expected.len());
    assert_eq!(
        report.calls,
        vec![
            HostCall::TakeDelivery {
                peer: client.peer,
                result: Ok(expected.len()),
            },
            HostCall::AcknowledgeDelivery {
                peer: client.peer,
                through: expected.last().expect("staged").ordinal,
            },
        ],
        "phases 4–7 alone: no Pump call"
    );
    let want: Vec<(Channel, Vec<u8>)> = expected
        .iter()
        .map(|entry| (channel_of(entry.kind), entry.bytes.clone()))
        .collect();
    client.read_until("widening delivered", |c| c.recv.len() >= start + want.len());
    assert_eq!(client.recv[start..].to_vec(), want);

    // `pump(10)` then `flush(9)` is a regression and changes nothing.
    qh.pump(10).expect("pump at 10");
    let before = host_state(qh.host());
    let received = client.recv.len();
    assert_eq!(qh.flush(9), Err(QuicHostError::TimeRegression));
    client.read_available();
    assert_eq!(client.recv.len(), received, "nothing was delivered");
    assert_eq!(host_state(qh.host()), before, "nothing changed");
    assert_eq!(qh.host().epoch(client.peer), Ok(client.epoch));
    qh.pump(10).expect("not poisoned by the regression");
}

// ---------------------------------------------------------------------------
// Q2.
// ---------------------------------------------------------------------------

#[test]
fn q2_pass_throughs() {
    let mut qh = QuicHost::start(tb3(0), bind_server(ServerLimits::v1()), 0).expect("starts");
    assert_eq!(qh.host().paused(), None);
    qh.pause(1).expect("pauses");
    assert_eq!(qh.host().paused(), Some(PauseReason::Operator));
    qh.unpause(2).expect("unpauses");
    assert_eq!(qh.host().paused(), None);

    // The host records `now_ms`, so the adapter's earlier time still bounds it.
    let before = host_state(qh.host());
    assert_eq!(
        qh.pause(0),
        Err(QuicHostError::Host(HostError::TimeRegression))
    );
    assert_eq!(host_state(qh.host()), before, "the refusal changed nothing");
    assert_eq!(qh.host().paused(), None);

    // `drive_ai` equals the same call on a twin driven directly.
    let sides =
        Sides::new(vec![(a(), SideId(1)), (b(), SideId(2)), (c(), SideId(1))]).expect("sides");
    let mut twin = tb3(0);
    let through = qh
        .drive_ai(&sides, &[a()], MVP_AI_BUDGET)
        .expect("passes through");
    let direct = twin
        .drive_ai(&sides, &[a()], MVP_AI_BUDGET)
        .expect("decides");
    assert_eq!(through, direct);
    assert!(!through.steps.is_empty(), "A decides");
    assert_eq!(qh.host().authority_hash(), twin.authority_hash());
}

// ---------------------------------------------------------------------------
// Q3.
// ---------------------------------------------------------------------------

#[test]
fn q3_embedded_save_runs_dedicated() {
    let path = embedded_save("q3");
    let mut quic_loop = HostLoop::new(QuicHosting::new(load_dedicated(&path, 2, 100)), 100);
    let loaded = load_host_file(&path, &ident(), 2, 100).expect("loads");
    let mut local_loop = HostLoop::new(LocalHost::new(loaded.host), 100);

    // Step 1: both loads are `Restore`-paused and hold tick 3.
    let quic_step = quic_loop.step(100).expect("steps").cycle.expect("a cycle");
    assert!(!quic_step.advance.ticked);
    assert_eq!(quic_step.advance.paused, Some(PauseReason::Restore));
    assert_eq!(quic_loop.hosting().quic().host().server_tick(), 3);
    let local_step = local_loop.step(100).expect("steps").cycle.expect("a cycle");
    assert_eq!(local_step.advance.paused, Some(PauseReason::Restore));
    assert_eq!(
        quic_loop.hosting().quic().host().authority_hash(),
        local_loop.hosting().host().authority_hash()
    );

    // Step 2: unpaused, both tick once.
    quic_loop
        .hosting_mut()
        .quic_mut()
        .unpause(100)
        .expect("unpauses");
    let quic_step = quic_loop.step(150).expect("steps").cycle.expect("a cycle");
    assert!(quic_step.advance.ticked);
    assert_eq!(quic_step.advance.paused, None);
    assert_eq!(quic_loop.hosting().quic().host().server_tick(), 4);
    local_loop
        .hosting_mut()
        .host_mut()
        .unpause(100)
        .expect("unpauses");
    let local_step = local_loop.step(150).expect("steps").cycle.expect("a cycle");
    assert!(local_step.advance.ticked);
    assert_eq!(
        quic_loop.hosting().quic().host().authority_hash(),
        local_loop.hosting().host().authority_hash()
    );

    // Control: no unpause, so the loaded host never ticks past 3.
    let mut still = HostLoop::new(QuicHosting::new(load_dedicated(&path, 4, 100)), 100);
    still.step(100).expect("steps");
    assert_eq!(still.hosting().quic().host().server_tick(), 3);
    still.step(150).expect("steps");
    assert_eq!(still.hosting().quic().host().server_tick(), 3);
}

// ---------------------------------------------------------------------------
// Q4.
// ---------------------------------------------------------------------------

#[test]
fn q4_a_client_joins_the_loaded_campaign() {
    let path = embedded_save("q4");
    let loaded = load_host_file(&path, &ident(), 2, 100).expect("loads");
    let mut quic = QuicHost::start(loaded.host, bind_dual(), 100).expect("starts");
    let invitation = quic
        .register_invitation(invitation(&cred_bytes(0), all_actors(), full_grants()))
        .expect("registers");
    let runner = spawn(
        HostLoop::new(QuicHosting::new(quic), 100),
        SystemClock::starting_at(100),
    )
    .expect("spawns");
    assert_eq!(
        runner
            .call(|l| {
                let now = l.last_ms();
                l.hosting_mut().quic_mut().unpause(now)
            })
            .expect("alive"),
        Ok(())
    );

    let addr = runner
        .call(|l| l.hosting().quic().local_addr())
        .expect("alive");
    let pending = connect_snapshot_async(addr, &cred_bytes(0));
    let start = Instant::now();
    let (peer, epoch) = loop {
        let found = runner
            .call(move |l| l.hosting().quic().invitation_binding(invitation))
            .expect("alive")
            .expect("registered");
        if let Some((_conn, peer)) = found {
            let epoch = runner
                .call(move |l| l.hosting().quic().host().epoch(peer))
                .expect("alive")
                .expect("bound");
            break (peer, epoch);
        }
        assert!(start.elapsed() < DEADLINE, "deadline: client bound");
        std::thread::sleep(Duration::from_millis(2));
    };
    let quic = pending.join().expect("client thread").expect("connect");
    let mut client = ChannelClient {
        quic,
        peer,
        epoch,
        invitation,
        recv: Vec::new(),
        receipts: Vec::new(),
        closed: None,
    };
    // The bind's first state arrives in the runner's pump; hold it.
    client.read_until("first state", |c| !c.recv.is_empty());

    let frame = IntentFrame {
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: n(1),
        body: IntentBody::EndTurn,
    };
    let bytes = codec_v2::encode_intent(&frame).expect("encodes");
    client
        .quic
        .try_send_channel(Channel::Lane0, &bytes)
        .expect("client sends");
    client.read_until("EndTurn receipt", |c| c.receipts.contains(&1));
    assert!(client.closed.is_none(), "the connection lives");

    let exit = runner.join().expect("joined");
    assert_eq!(exit.error, None);
    assert!(exit.host_loop.cycles() >= 1);
    assert_eq!(
        exit.host_loop.hosting().quic().host().epoch(peer),
        Ok(epoch)
    );
}

// ---------------------------------------------------------------------------
// Q4's control.
// ---------------------------------------------------------------------------

#[test]
fn q4_control_a_clock_at_zero_ends_the_runner() {
    let path = embedded_save("q4_control");
    let quic = load_dedicated(&path, 2, 100);
    // `SystemClock::new()` reads 0, below the loop's and host's 100.
    let runner = spawn(
        HostLoop::new(QuicHosting::new(quic), 100),
        SystemClock::new(),
    )
    .expect("spawns");
    let start = Instant::now();
    while !runner.is_finished() {
        assert!(start.elapsed() < DEADLINE, "deadline: the runner ends");
        std::thread::sleep(Duration::from_millis(2));
    }
    let exit = runner.join().expect("joined");
    match exit.error {
        Some(LoopError::TimeRegression { last_ms, now_ms }) => {
            assert_eq!(last_ms, 100);
            assert!(now_ms < 100, "the clock's first read is below 100");
        }
        other => panic!("expected a step TimeRegression, got {other:?}"),
    }
    assert_eq!(exit.host_loop.cycles(), 0);
}

// ---------------------------------------------------------------------------
// Q5.
// ---------------------------------------------------------------------------

#[test]
fn q5_captures_are_retired() {
    let path = embedded_save("q5");
    let mut host_loop = HostLoop::new(QuicHosting::new(load_dedicated(&path, 2, 100)), 100);
    host_loop.step(100).expect("steps");
    host_loop
        .hosting_mut()
        .quic_mut()
        .unpause(100)
        .expect("unpauses");
    host_loop.step(150).expect("steps");
    let host = host_loop.hosting().quic().host();
    assert!(
        host.last_capture() > 0,
        "the save carried a capture journal"
    );
    assert_eq!(host.capture_acknowledged(), host.last_capture());
}
