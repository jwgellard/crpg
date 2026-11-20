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
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crpg_ai::{SideId, Sides};
use crpg_core::Ulid;
use crpg_net::codec_v2;
use crpg_net::facts_v3::EncounterGrants;
use crpg_net::protocol::{IntentBody, IntentFrame, LANE_COMBAT};
use crpg_net::protocol_transfer::{TransferId, TransferUp};
use crpg_net::protocol_v3::{PauseReason, PauseRights};
use crpg_net::transfer_v3;
use crpg_net::transport::Transport;
use crpg_net_quic::{
    Channel, ClientLimits, CloseReason, ConnectError, Framing, Hello, QuicClient, QuicServer,
    ServerConfig, ServerLimits, MOVEMENT_WIRE_VERSION,
};
use crpg_server::host::{
    AiBudget, DeliveryKind, DisclosureGrants, EntityFactGrant, FactGrants, Host, HostConfig,
    HostError, HostSettings, PeerHandle, ProtocolSelection, TransferDisposition,
    DEFAULT_HOST_GRANTS, MVP_AI_BUDGET,
};
use crpg_server::quic::{
    GrantChange, HelloOutcome, HostCall, InvitationId, QuicHost, QuicHostError, V4Invitation,
};
use crpg_server::run::{HostLoop, LocalHost, LoopError, QuicAiHook, QuicHosting};
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

// ---------------------------------------------------------------------------
// Fix round A (T154d-fix-a): the test gaps the mutant run found. Each case is
// named `fa_<n>_<what>` after its brief item and pins one contract item.
// ---------------------------------------------------------------------------

/// A fresh adapter over `TB3` and a bound server: the refusal cases below
/// need nothing but the adapter's own time and the host's.
fn fresh_adapter() -> QuicHost {
    QuicHost::start(tb3(0), bind_dual(), 0).expect("starts")
}

/// fa_1 (QF4, B-15): `flush` records its `now_ms`, so a later pump or flush
/// below it is a time regression.
#[test]
fn fa_1_flush_records_now_ms() {
    let mut qh = fresh_adapter();
    qh.flush(10).expect("flushes at 10");
    let before = host_state(qh.host());
    assert_eq!(
        qh.pump(9),
        Err(QuicHostError::TimeRegression),
        "flush(10) recorded 10"
    );
    assert_eq!(
        qh.flush(9),
        Err(QuicHostError::TimeRegression),
        "flush(10) recorded 10"
    );
    assert_eq!(host_state(qh.host()), before, "a refusal changes nothing");
    qh.pump(10).expect("10 is still the last accepted time");
}

/// fa_2 (QF5, B-15): a refused `flush` leaves the last accepted time alone,
/// so the same regression is still refused afterwards.
#[test]
fn fa_2_a_refused_flush_changes_nothing() {
    let mut qh = fresh_adapter();
    qh.flush(10).expect("flushes at 10");
    let before = host_state(qh.host());
    assert_eq!(qh.flush(9), Err(QuicHostError::TimeRegression));
    assert_eq!(
        qh.pump(9),
        Err(QuicHostError::TimeRegression),
        "the refused flush did not record 9"
    );
    assert_eq!(
        qh.flush(9),
        Err(QuicHostError::TimeRegression),
        "the refused flush did not record 9"
    );
    assert_eq!(host_state(qh.host()), before, "a refusal changes nothing");
    qh.pump(10).expect("the refusal did not poison");
}

/// fa_3 (QD3, B-16): `drive_ai` passes the caller's budget through. A budget
/// of zero decisions per turn forces the turn cap, which the MVP budget does
/// not do on the same authority.
#[test]
fn fa_3_drive_ai_passes_the_budget_through() {
    let sides =
        Sides::new(vec![(a(), SideId(1)), (b(), SideId(2)), (c(), SideId(1))]).expect("sides");
    let zero = AiBudget {
        decisions_per_turn: 0,
        actors_per_drive: 8,
    };
    let mut qh = fresh_adapter();
    let mut twin = tb3(0);
    let through = qh
        .drive_ai(&sides, &[a(), b(), c()], zero)
        .expect("passes through");
    let same = twin
        .drive_ai(&sides, &[a(), b(), c()], zero)
        .expect("decides");
    assert_eq!(through, same, "the twin given the same budget agrees");
    assert!(through.capped, "zero decisions per turn forces the cap");
    assert_eq!(through.steps.len(), 1, "the cap's one forced step");

    let mut mvp = tb3(0);
    let default = mvp
        .drive_ai(&sides, &[a(), b(), c()], MVP_AI_BUDGET)
        .expect("decides");
    assert_ne!(through, default, "the caller's budget, never MVP_AI_BUDGET");
}

/// fa_4 (QA3, B-16): `pause` hands the host its own `now_ms`, so the host
/// records it and later refuses an earlier time.
#[test]
fn fa_4_pause_records_now_ms() {
    let mut qh = fresh_adapter();
    qh.pause(5).expect("pauses");
    assert_eq!(qh.host().paused(), Some(PauseReason::Operator));
    assert_eq!(
        qh.unpause(4),
        Err(QuicHostError::Host(HostError::TimeRegression)),
        "the host recorded the pause's 5"
    );
    assert_eq!(qh.host().paused(), Some(PauseReason::Operator));
    qh.unpause(5).expect("the recorded time itself is accepted");
    assert_eq!(qh.host().paused(), None);
}

/// fa_5 (QU4, B-16): `unpause` returns the host's error instead of
/// swallowing it.
#[test]
fn fa_5_unpause_returns_the_host_error() {
    let mut qh = fresh_adapter();
    qh.pause(5).expect("pauses");
    let before = host_state(qh.host());
    assert_eq!(
        qh.unpause(4),
        Err(QuicHostError::Host(HostError::TimeRegression)),
        "a host error is returned, not swallowed"
    );
    assert_eq!(host_state(qh.host()), before, "the refusal changed nothing");
    assert_eq!(qh.host().paused(), Some(PauseReason::Operator));
    qh.unpause(5)
        .expect("still unpausable at the recorded time");
}

/// fa_6 (RI3, B-14): the intake pump's failure is the step's error. The
/// host's accepted time is ahead of the step, so `Host::pump` inside the
/// intake pump fails and poisons the adapter; a swallowed intake would let
/// the cycle reach `advance`, whose poisoned check reports `Poisoned`
/// instead.
#[test]
fn fa_6_intake_error_is_the_loop_error() {
    let mut qh = fresh_adapter();
    qh.pause(1_000)
        .expect("moves the host's time ahead of the step");
    let mut host_loop = HostLoop::new(QuicHosting::new(qh), 0);
    assert_eq!(
        host_loop.step(100),
        Err(LoopError::Hosting(QuicHostError::Host(
            HostError::TimeRegression
        ))),
        "the intake pump's host error is the step's error"
    );
    assert_eq!(
        host_loop.hosting().quic().host().server_tick(),
        0,
        "no later stage ran"
    );
}

/// fa_7 (RA1, RA2, RS1; B-14 and `set_ai`): the `drive_ai` stage is the hook
/// when one is set, once per step, given the cycle's `now_ms`.
#[test]
fn fa_7_the_ai_hook_runs_once_per_step() {
    let seen: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&seen);
    let hook: QuicAiHook = Box::new(move |quic, now_ms| {
        recorded.lock().expect("lock").push(now_ms);
        let sides =
            Sides::new(vec![(a(), SideId(1)), (b(), SideId(2)), (c(), SideId(1))]).expect("sides");
        quic.drive_ai(&sides, &[a(), b(), c()], MVP_AI_BUDGET)
            .map(|_report| ())
    });
    let mut hosting = QuicHosting::new(fresh_adapter());
    hosting.set_ai(Some(hook));
    let mut host_loop = HostLoop::new(hosting, 100);
    host_loop.step(100).expect("steps");
    host_loop.step(150).expect("steps");
    assert_eq!(
        *seen.lock().expect("lock"),
        vec![100, 150],
        "once per step, in cycle order, with the cycle's now_ms"
    );

    // Control: a hosting with no hook runs the cycle and records nothing.
    let after = seen.lock().expect("lock").clone();
    let mut no_hook = HostLoop::new(QuicHosting::new(fresh_adapter()), 100);
    no_hook.step(100).expect("steps");
    no_hook.step(150).expect("steps");
    assert_eq!(*seen.lock().expect("lock"), after, "no hook, no call");
}

/// fa_8 (RA3, Errors): the hook's error is the loop's error and no later
/// stage runs.
#[test]
fn fa_8_the_ai_hook_error_is_the_loop_error() {
    let hook: QuicAiHook =
        Box::new(|_quic, _now_ms| Err(QuicHostError::Host(HostError::QueueFull)));
    let mut hosting = QuicHosting::new(fresh_adapter());
    hosting.set_ai(Some(hook));
    let mut host_loop = HostLoop::new(hosting, 0);
    assert_eq!(
        host_loop.step(0),
        Err(LoopError::Hosting(QuicHostError::Host(
            HostError::QueueFull
        )))
    );
    assert_eq!(
        host_loop.hosting().quic().host().server_tick(),
        0,
        "advance did not run"
    );
    assert_eq!(host_loop.cycles(), 1);
    assert_eq!(host_loop.next_due_ms(), 50);
}

/// fa_9 (RS2, `set_ai`): `set_ai(None)` clears the hook.
#[test]
fn fa_9_set_ai_none_clears_the_hook() {
    let seen: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&seen);
    let hook: QuicAiHook = Box::new(move |_quic, now_ms| {
        recorded.lock().expect("lock").push(now_ms);
        Ok(())
    });
    let mut hosting = QuicHosting::new(fresh_adapter());
    hosting.set_ai(Some(hook));
    let mut host_loop = HostLoop::new(hosting, 100);
    host_loop.step(100).expect("steps");
    assert_eq!(*seen.lock().expect("lock"), vec![100]);
    host_loop.hosting_mut().set_ai(None);
    host_loop.step(150).expect("steps");
    assert_eq!(
        *seen.lock().expect("lock"),
        vec![100],
        "set_ai(None) cleared the hook"
    );
}

// The `/3` client fa_10 needs. Only a v3/v4 session receives a tick's
// publication (a v1/v2 session gets frames only from its own admitted
// commands, which the intake pump's own phases 4–7 already send), so the
// deliver stage is observable there alone.

fn bind_movement_dual() -> QuicServer {
    QuicServer::bind_movement(ServerConfig {
        bind: loopback(),
        identity: identity(),
        limits: ServerLimits::v1(),
    })
    .expect("binds a movement loopback server")
}

fn v4_facts() -> FactGrants {
    let flags = |entity| EntityFactGrant {
        entity,
        pools: true,
        effects: true,
        stats: true,
        tags: true,
    };
    FactGrants {
        entities: vec![flags(a()), flags(b()), flags(c())],
        encounter: EncounterGrants {
            mode: true,
            pending_mode: true,
            clock: true,
            timeline: true,
        },
        host: DEFAULT_HOST_GRANTS,
    }
}

fn v4_invitation() -> V4Invitation {
    V4Invitation {
        credential: credential(&cred_bytes(0)),
        control: control(&[a(), b(), c()]),
        grants: full_grants(),
        facts: v4_facts(),
        rights: PauseRights {
            pause: true,
            unpause: true,
        },
    }
}

fn connect_v4_async(
    server: std::net::SocketAddr,
    credential_bytes: &[u8],
) -> std::thread::JoinHandle<Result<QuicClient, ConnectError>> {
    let config = client_config(ClientLimits::v1());
    let hello = Hello {
        wire_version: MOVEMENT_WIRE_VERSION,
        credential: credential(credential_bytes),
    };
    std::thread::spawn(move || QuicClient::connect_movement(config, server, &hello))
}

/// A `/3` client that reads frames whole: a v3 session's lane 0 is v3-encoded,
/// so it is not decoded here.
struct RawClient {
    quic: QuicClient,
    peer: PeerHandle,
    recv: Vec<(Channel, Vec<u8>)>,
    closed: Option<CloseReason>,
}

impl RawClient {
    fn read_available(&mut self) -> usize {
        let mut count = 0;
        if self.closed.is_some() {
            return 0;
        }
        loop {
            match self.quic.try_recv_channel() {
                Ok(Some(frame)) => {
                    self.recv.push(frame);
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

    fn lane0_frames(&self) -> usize {
        self.recv
            .iter()
            .filter(|(channel, _)| *channel == Channel::Lane0)
            .count()
    }

    fn read_until(&mut self, what: &str, mut cond: impl FnMut(&RawClient) -> bool) {
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

/// Connects a `/3` client and waits for its bind delta (the transfer is
/// staged before it, so the delta is the bind's one lane-0 frame).
fn connect_v4(qh: &mut QuicHost, now: u64, credential_bytes: &[u8]) -> RawClient {
    let pending = connect_v4_async(qh.local_addr(), credential_bytes);
    let start = Instant::now();
    let decision = loop {
        let report = qh.pump(now).expect("pump");
        if let Some(decision) = report.hellos.into_iter().next() {
            break decision;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: hello decided");
        qh.wait(Duration::from_millis(5));
    };
    let HelloOutcome::Accepted { peer, .. } = decision.outcome else {
        panic!("expected Accepted, got {decision:?}");
    };
    let quic = pending.join().expect("client thread").expect("connect");
    let mut client = RawClient {
        quic,
        peer,
        recv: Vec::new(),
        closed: None,
    };
    client.read_until("bind delta", |c| c.lane0_frames() >= 1);
    client
}

/// Answers the bind's snapshot, so the session is no longer throttled (a v3
/// session with a transfer in flight logs a subset of its publications).
fn publish_bind_snapshot(qh: &mut QuicHost, client: &mut RawClient, now: u64) {
    let epoch = qh.host().epoch(client.peer).expect("bound");
    let bytes = transfer_v3::encode_up(&TransferUp::Published {
        epoch,
        transfer: TransferId::new(1).expect("nonzero"),
    })
    .expect("encodes");
    client
        .quic
        .try_send_channel(Channel::Transfer, &bytes)
        .expect("client publishes");
    let start = Instant::now();
    loop {
        let report = qh.pump(now).expect("pump");
        if report.calls.iter().any(|call| {
            matches!(
                call,
                HostCall::IngestTransfer {
                    result: TransferDisposition::Published,
                    ..
                }
            )
        }) {
            return;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: Published ingested");
        qh.wait(Duration::from_millis(5));
    }
}

/// fa_10 (RD1, B-14): the deliver stage is `flush(now)`. A turn timer ends
/// A's turn in the tick, whose page is staged after intake, so only the
/// cycle's own deliver sends it: no further pump runs.
#[test]
fn fa_10_deliver_sends_the_ticks_delta() {
    let mut host = tb3(0);
    host.set_settings(
        HostSettings {
            turn_timer_ms: Some(20),
            ..HostSettings::multiplayer()
        },
        0,
    )
    .expect("arms the turn timer");
    let mut qh = QuicHost::start(host, bind_movement_dual(), 0).expect("starts");
    qh.register_v4_invitation(v4_invitation())
        .expect("registers");
    let mut client = connect_v4(&mut qh, 0, &cred_bytes(0));
    publish_bind_snapshot(&mut qh, &mut client, 0);
    let before = client.lane0_frames();

    let mut host_loop = HostLoop::new(QuicHosting::new(qh), 0);
    let step = host_loop.step(50).expect("steps").cycle.expect("a cycle");
    assert!(step.advance.ticked);
    assert_eq!(
        step.advance.timer_ended_turn,
        Some(a()),
        "the timer ended A's turn"
    );
    client.read_until("the tick's page, before any further pump", |c| {
        c.lane0_frames() > before
    });
    assert!(client.closed.is_none(), "the connection lives");
}

/// A save whose capture journal has one unacknowledged record: the command is
/// admitted and applied on the host directly, so no loop's retire stage ran.
fn unacknowledged_save(case: &str) -> PathBuf {
    let path = temp_path(case);
    let mut host = tb3(0);
    let peer = host
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let frame = end_frame(&host, peer, 1);
    host.ingest(peer, &frame, 0).expect("stages");
    host.pump(0).expect("admits");
    host.advance(0).expect("advances");
    assert!(
        host.last_capture() > host.capture_acknowledged(),
        "the save carries an unacknowledged capture"
    );
    save_host_file(&path, &host, &ident()).expect("quiescent save");
    path
}

/// fa_11 (RC1, RC2, RC3; B-14): retire is
/// `acknowledge_captures(last_capture)`.
#[test]
fn fa_11_retire_acknowledges_the_last_capture() {
    let path = unacknowledged_save("fa_11");
    let mut host_loop = HostLoop::new(QuicHosting::new(load_dedicated(&path, 2, 100)), 100);
    let host = host_loop.hosting().quic().host();
    assert!(
        host.last_capture() > host.capture_acknowledged(),
        "the load keeps the record unacknowledged"
    );
    let last = host.last_capture();

    let step = host_loop.step(100).expect("steps").cycle.expect("a cycle");
    assert_eq!(step.advance.paused, Some(PauseReason::Restore));
    let host = host_loop.hosting().quic().host();
    assert_eq!(host.last_capture(), last, "no new capture");
    assert_eq!(
        host.capture_acknowledged(),
        host.last_capture(),
        "retire acknowledged through the last capture"
    );
}
