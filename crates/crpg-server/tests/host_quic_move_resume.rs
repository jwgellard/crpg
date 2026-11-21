//! T155c acceptance suite (`tasks/T155.md` C-1..C-6, QR1..QR4): the resumed
//! movement (`crpg-lane0/3`) connection through the adapter, over real
//! loopback transport, public API only. QR5 is a unit test in `quic.rs`.
//!
//! The rig, fixtures and client are `tests/host_quic_move.rs`'s (T150c); the
//! resume helpers are `tests/host_quic_resume.rs`'s (T025b-q). `INV` is the v4
//! invitation (credential `[0x51; 32]`, control over a/b/c) every case but QR4
//! presents; its first binding is handle 1, so `E = e(1)`.
//!
//! Oracles are hand-written bytes and the host's own state. Every wait is a
//! 30 s failure guard; no case sleeps as an oracle, and every grace is timed
//! by the test's injected `now_ms`.

#[path = "support/quic_host_rig.rs"]
mod rig;

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_nav::{bake_box, NavMesh, NavRect};
use crpg_net::codec_resume;
use crpg_net::facts_v3::EncounterGrants;
use crpg_net::movement_v4::{
    encode_down, encode_intent, MoveBody, MoveDown, MoveIntent, MoveOutcome, MoveReceipt,
};
use crpg_net::protocol::NetId;
use crpg_net::protocol_resume::{ConnectionFence, LaneClaim, ResumeProof, ResumeRequest};
use crpg_net::protocol_transfer::{TransferDown, TransferUp};
use crpg_net::protocol_v3::{PauseRights, RejectionCode as CodeV3};
use crpg_net::transfer_v3;
use crpg_net_quic::{
    Channel, ClientLimits, CloseReason, ConnectError, ConnectionId, Credential, QuicClient,
    QuicServer, ServerConfig, ServerLimits, Welcome, MOVEMENT_WIRE_VERSION,
};
use crpg_server::host::{
    ControlGrant, DisclosureGrants, EntityDisclosure, EntityFactGrant, FactGrants, Host,
    MoveAdmission, PeerHandle, TransferDisposition, DEFAULT_HOST_GRANTS,
};
use crpg_server::quic::{
    Fence, FenceCause, HelloDecision, HelloOutcome, HostCall, InvitationId, Lane1Call, Lane1Trace,
    QuicHost, QuicPumpReport, Rejection, ResumeOutcome, ResumeReport, V4Invitation,
};
use crpg_sim::HistoryWorld;
use serde_json::{json, Value};

use rig::*;

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");

// ---------------------------------------------------------------------------
// Identities, geometry and fixtures (`MV3rt`, `M10`; as `tests/host_lane1.rs`).
// ---------------------------------------------------------------------------

fn eid(index: u32) -> EntityId {
    serde_json::from_value(json!({"index": index, "generation": 1})).expect("valid entity id")
}
fn a() -> EntityId {
    eid(0)
}
fn b() -> EntityId {
    eid(1)
}
fn c() -> EntityId {
    eid(2)
}
fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}
fn rect(x0: i32, z0: i32, x1: i32, z1: i32) -> NavRect {
    NavRect {
        min_x: Fx16_16::from_int(x0),
        min_z: Fx16_16::from_int(z0),
        max_x: Fx16_16::from_int(x1),
        max_z: Fx16_16::from_int(z1),
    }
}

/// `M10`: the 10 m square that holds `MV3rt`'s three movers.
fn m10() -> NavMesh {
    bake_box(Fx16_16::ZERO, rect(0, 0, 10, 10), &[]).expect("bakes")
}

fn tb3c_value() -> Value {
    let mut value: Value = serde_json::from_str(FIXTURE).expect("fixture parses");
    value["version"] = json!(2);
    value["world"]["combat"]["clock"] = json!({"round_ticks": 4, "elapsed": 0});
    value
}

fn rt3_value() -> Value {
    let mut value = tb3c_value();
    value["world"]["combat"]["mode"] = json!("real_time");
    value["world"]["combat"]["active"] = Value::Null;
    for entry in value["world"]["timeline"].as_array_mut().expect("timeline") {
        entry[0] = json!(0);
    }
    value
}

fn movement_edits(mut value: Value) -> Value {
    value["world"]["movers"] = json!([
        [{"index": 0, "generation": 1},
         {"position": [65536, 0, 65536], "speed": 65536, "step": 655360}],
        [{"index": 1, "generation": 1},
         {"position": [196608, 0, 65536], "speed": 65536, "step": 655360}],
        [{"index": 2, "generation": 1},
         {"position": [65536, 0, 327680], "speed": 65536, "step": 655360}]
    ]);
    value["world"]["transforms"] = json!([
        [{"index": 0, "generation": 1},
         {"position": [1.0, 0.0, 1.0], "velocity": [0.0, 0.0, 0.0]}],
        [{"index": 1, "generation": 1},
         {"position": [3.0, 0.0, 1.0], "velocity": [0.0, 0.0, 0.0]}],
        [{"index": 2, "generation": 1},
         {"position": [1.0, 0.0, 5.0], "velocity": [0.0, 0.0, 0.0]}]
    ]);
    value["world"]["combat"]["definition"]["movement"] = json!({
        "measure": "euclidean",
        "pool": "000000000000000000000000JY",
        "step_cost": 1
    });
    value
}

fn mv3rt_value() -> Value {
    movement_edits(rt3_value())
}

/// The authority from `value`, its four start envelopes acknowledged.
fn history_of(value: &Value) -> HistoryWorld {
    let mut history: HistoryWorld =
        serde_json::from_value(value.clone()).expect("fixture loads through validated serde");
    history
        .acknowledge(history.last_sequence())
        .expect("acknowledges its own range");
    history
}

fn disclosure(entity: EntityId) -> EntityDisclosure {
    EntityDisclosure {
        entity,
        present: true,
        health: true,
        spawn: true,
        despawn: true,
        died: true,
        action_actor: true,
        action_target: true,
        action_outcome: true,
        action_damage: true,
        turn_start: true,
    }
}

fn full_grants(entities: &[EntityId]) -> DisclosureGrants {
    DisclosureGrants {
        entities: entities.iter().copied().map(disclosure).collect(),
        abilities: vec![Ulid::from_u128(603), Ulid::from_u128(604)],
        encounters: vec![Ulid::from_u128(601)],
        turn_round: true,
        encounter_end: true,
        turn_state: true,
    }
}

fn all_fact_flags(entity: EntityId) -> EntityFactGrant {
    EntityFactGrant {
        entity,
        pools: true,
        effects: true,
        stats: true,
        tags: true,
    }
}

fn facts() -> FactGrants {
    FactGrants {
        entities: vec![
            all_fact_flags(a()),
            all_fact_flags(b()),
            all_fact_flags(c()),
        ],
        encounter: EncounterGrants {
            mode: true,
            pending_mode: true,
            clock: true,
            timeline: true,
        },
        host: DEFAULT_HOST_GRANTS,
    }
}

fn r1() -> PauseRights {
    PauseRights {
        pause: true,
        unpause: true,
    }
}

fn control(actors: &[EntityId]) -> ControlGrant {
    ControlGrant {
        actors: actors.to_vec(),
    }
}

/// The session epoch of handle `h` on incarnation 7.
fn e(h: u64) -> [u8; 16] {
    epoch_of(INCARNATION, h)
}

/// `INV`: the v4 principal every case presents.
fn inv() -> V4Invitation {
    V4Invitation {
        credential: credential(&[0x51; 32]),
        control: control(&[a(), b(), c()]),
        grants: full_grants(&[a(), b(), c()]),
        facts: facts(),
        rights: r1(),
    }
}

fn v4_inv(
    credential_bytes: &[u8],
    control_grant: ControlGrant,
    grants: DisclosureGrants,
    fact_grants: FactGrants,
) -> V4Invitation {
    V4Invitation {
        credential: credential(credential_bytes),
        control: control_grant,
        grants,
        facts: fact_grants,
        rights: r1(),
    }
}

/// A moved authority bound to `M10`.
fn movement_host() -> Host {
    let mut host = Host::from_history(history_of(&mv3rt_value()), host_config(V2), CLOCK_START)
        .expect("wraps an acknowledged authority");
    host.bind_nav_mesh(m10()).expect("binds M10");
    host
}

/// A `bind_movement` loopback server with the rig's identity.
fn bind_movement_server(limits: ServerLimits) -> QuicServer {
    QuicServer::bind_movement(ServerConfig {
        bind: loopback(),
        identity: identity(),
        limits,
    })
    .expect("bind movement server")
}

/// A started adapter over the movement host plus its clock, `INV` registered
/// when `register` is true.
fn movement_adapter(register: bool) -> (QuicHost, TestClock, Option<InvitationId>) {
    let clock = TestClock::new();
    let qh = QuicHost::start(
        movement_host(),
        bind_movement_server(ServerLimits::v1()),
        clock.now(),
    )
    .expect("starts over a fresh host");
    let mut qh = qh;
    let id = register.then(|| qh.register_v4_invitation(inv()).expect("registers INV"));
    (qh, clock, id)
}

// ---------------------------------------------------------------------------
// The movement client: lane-0 and lane-1 sends logged separately.
// ---------------------------------------------------------------------------

struct MvClient {
    quic: QuicClient,
    conn: ConnectionId,
    peer: PeerHandle,
    /// Every lane-1 frame sent, in order: frame ordinal k is `lane1[k - 1]`.
    lane1: Vec<Vec<u8>>,
    /// Every lane-0 delivery entry read, with its channel.
    entries: Vec<(Channel, Vec<u8>)>,
    /// Every lane-1 frame read, in order.
    receipts: Vec<Vec<u8>>,
    /// Every datagram read, in order.
    datagrams: Vec<Vec<u8>>,
    closed: Option<CloseReason>,
}

impl MvClient {
    fn send_lane1(&mut self, bytes: Vec<u8>) {
        self.quic
            .try_send_lane1(&bytes)
            .expect("client lane-1 send");
        self.lane1.push(bytes);
    }

    /// Reads every lane-0 entry and lane-1 frame available now.
    fn read_available(&mut self) {
        if self.closed.is_some() {
            return;
        }
        loop {
            match self.quic.try_recv_channel() {
                Ok(Some(entry)) => self.entries.push(entry),
                Ok(None) => break,
                Err(reason) => {
                    self.closed = Some(reason);
                    return;
                }
            }
        }
        loop {
            match self.quic.try_recv_lane1() {
                Ok(Some(frame)) => self.receipts.push(frame),
                Ok(None) => break,
                Err(reason) => {
                    self.closed = Some(reason);
                    return;
                }
            }
        }
        loop {
            match self.quic.try_recv_datagram() {
                Ok(Some(datagram)) => self.datagrams.push(datagram),
                Ok(None) => break,
                Err(reason) => {
                    self.closed = Some(reason);
                    return;
                }
            }
        }
    }

    /// Guard: reads until `cond` holds, panicking `"deadline"` after 30 s.
    fn read_until(&mut self, what: &str, mut cond: impl FnMut(&MvClient) -> bool) {
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

/// Connects a movement client with the version-4 hello of `credential`.
fn connect_movement(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    credential: &Credential,
) -> (MvClient, HelloDecision, Vec<Step>) {
    let addr = qh.local_addr();
    connect_movement_via(qh, clock, addr, credential)
}

/// As [`connect_movement`], against an explicit address (a relay).
fn connect_movement_via(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    addr: SocketAddr,
    credential: &Credential,
) -> (MvClient, HelloDecision, Vec<Step>) {
    let hello = crpg_net_quic::Hello {
        wire_version: MOVEMENT_WIRE_VERSION,
        credential: credential.clone(),
    };
    let pending = std::thread::spawn(move || {
        QuicClient::connect_movement(client_config(ClientLimits::v1()), addr, &hello)
    });
    let (decision, steps) = decide_hello(qh, clock, 0);
    let quic = pending
        .join()
        .expect("client thread")
        .expect("connect_movement");
    let HelloOutcome::Accepted { invitation, peer } = decision.outcome else {
        panic!("expected Accepted, got {decision:?}");
    };
    let _ = invitation;
    let client = MvClient {
        quic,
        conn: decision.conn,
        peer,
        lane1: Vec::new(),
        entries: Vec::new(),
        receipts: Vec::new(),
        datagrams: Vec::new(),
        closed: None,
    };
    (client, decision, steps)
}

/// Waits (guard) until the transport has an event or a frame queued.
fn await_transport(qh: &QuicHost, what: &str) {
    until(what, || qh.wait(Duration::from_millis(5)));
}

/// Guard: the client's lane-1 writer has handed every queued frame to QUIC
/// and the server holds a lane-1 frame, then paces the server's reader with
/// the transport's own wait (the client has no queued inbound frame, so the
/// wait runs to its timeout). The adapter exposes no server-side lane-1
/// count, so "all frames arrived" is a guard wait, never an oracle; the pace
/// gives the reader the time a single-threaded pump would not.
fn settle_lane1(qh: &QuicHost, client: &mut MvClient) {
    until("the client wrote its lane-1 frames", || {
        client.quic.lane1_stats().outbound_frames == 0
    });
    await_transport(qh, "a lane-1 frame is queued at the server");
    client.read_available();
    if client.closed.is_none() {
        client.quic.wait(Duration::from_millis(20));
    }
}

/// Pumps at `now` until `want` `IngestMove` calls have been traced, without
/// advancing the clock; returns the last report and every ingest trace.
fn pump_ingests(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    now: u64,
    want: usize,
) -> (Vec<QuicPumpReport>, Vec<Lane1Trace>) {
    clock.set(now);
    let mut reports: Vec<QuicPumpReport> = Vec::new();
    let mut traces: Vec<Lane1Trace> = Vec::new();
    let start = Instant::now();
    loop {
        let report = qh.pump(now).expect("pump");
        traces.extend(
            report
                .lane1
                .iter()
                .filter(|trace| matches!(trace.call, Lane1Call::IngestMove { .. }))
                .cloned(),
        );
        reports.push(report);
        if traces.len() >= want {
            return (reports, traces);
        }
        assert!(start.elapsed() < DEADLINE, "deadline: lane-1 ingests");
        qh.wait(Duration::from_millis(5));
    }
}

fn halt_intent(epoch: [u8; 16], seq: u64, actor: u64) -> MoveIntent {
    MoveIntent {
        epoch,
        seq,
        actor: n(actor),
        body: MoveBody::Halt,
    }
}

fn halt_bytes(epoch: [u8; 16], seq: u64, actor: u64) -> Vec<u8> {
    encode_intent(&halt_intent(epoch, seq, actor)).expect("encodes")
}
// ---------------------------------------------------------------------------
// T155c: resume helpers (`tests/host_quic_resume.rs`, on a movement endpoint).
// ---------------------------------------------------------------------------

fn grace_limits() -> ServerLimits {
    ServerLimits {
        idle_timeout_ms: 1_000,
        keep_alive_ms: 300,
        ..ServerLimits::v1()
    }
}

/// The keyed BLAKE3 proof for `epoch` under `INV`'s credential.
fn proof4(epoch: [u8; 16]) -> Vec<u8> {
    let mut message = b"crpg-resume-proof/1".to_vec();
    message.extend_from_slice(&epoch);
    blake3::keyed_hash(&[0x51; 32], &message)
        .as_bytes()
        .to_vec()
}

/// The exact ticket bytes: `[0x91, 0x03, 0x30] ++ epoch ++ [0x20] ++ proof`.
fn ticket4(epoch: [u8; 16]) -> Vec<u8> {
    let mut out = vec![0x91, 0x03, 0x30];
    out.extend_from_slice(&epoch);
    out.push(0x20);
    out.extend_from_slice(&proof4(epoch));
    out
}

/// A resume request's canonical bytes.
fn encode_request(
    epoch: [u8; 16],
    fence: u64,
    proof_bytes: &[u8],
    claims: &[(u8, u64)],
) -> Vec<u8> {
    codec_resume::encode_request(&ResumeRequest {
        epoch,
        fence: ConnectionFence::new(fence).expect("nonzero fence"),
        proof: ResumeProof::new(proof_bytes.to_vec()).expect("valid proof"),
        claims: claims
            .iter()
            .map(|(lane, through)| LaneClaim {
                lane: *lane,
                receipted_through: *through,
            })
            .collect(),
    })
    .expect("encodes a resume request")
}

fn grant(raw: u64) -> ConnectionFence {
    ConnectionFence::new(raw).expect("nonzero fence")
}

/// The accepted answer for `epoch` at fence 2 with lane-0 progress 0.
fn accepted_answer(epoch: [u8; 16]) -> Vec<u8> {
    let mut answer = vec![0x91, 0x03, 0x31];
    answer.extend_from_slice(&epoch);
    answer.extend_from_slice(&[0x02, 0x01, 0x00, 0x00]);
    answer
}

/// A resumable movement adapter over the movement host, `INV` registered.
fn start_movement_resumable(limits: ServerLimits) -> (QuicHost, TestClock, InvitationId) {
    let clock = TestClock::new();
    let mut qh = QuicHost::start_resumable(
        movement_host(),
        QuicServer::bind_movement_resumable(ServerConfig {
            bind: loopback(),
            identity: identity(),
            limits,
        })
        .expect("bind movement resumable server"),
        clock.now(),
    )
    .expect("starts over a fresh host");
    let id = qh.register_v4_invitation(inv()).expect("registers INV");
    (qh, clock, id)
}

fn mv_client(quic: QuicClient, conn: ConnectionId, peer: PeerHandle) -> MvClient {
    MvClient {
        quic,
        conn,
        peer,
        lane1: Vec::new(),
        entries: Vec::new(),
        receipts: Vec::new(),
        datagrams: Vec::new(),
        closed: None,
    }
}

/// The outcome of `connect_movement_resume`, plus the pump steps observed
/// while the resume opening was decided.
type ResumeConnect = (Result<(QuicClient, Vec<u8>), ConnectError>, Vec<Step>);

/// Opens a movement resume connection and pumps until its notice is decided.
fn resume4(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    session: Welcome,
    request: &[u8],
) -> ResumeConnect {
    let addr = qh.local_addr();
    let config = client_config(ClientLimits::v1());
    let request = request.to_vec();
    let pending = std::thread::spawn(move || {
        QuicClient::connect_movement_resume(config, addr, session, &request)
    });
    let steps = pump_until(qh, clock, 0, "resume decided", |_, report| {
        !report.resumes.is_empty()
    });
    (pending.join().expect("client thread"), steps)
}

fn session4() -> Welcome {
    Welcome {
        wire_version: MOVEMENT_WIRE_VERSION,
        epoch: e(1),
    }
}

fn reports_of(steps: Vec<Step>) -> Vec<QuicPumpReport> {
    steps.into_iter().map(|(_, report)| report).collect()
}

/// One pump at `now`, recorded.
fn pump_at(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    now: u64,
    all: &mut Vec<QuicPumpReport>,
) -> usize {
    clock.set(now);
    all.push(qh.pump(now).expect("pump"));
    all.len() - 1
}

/// `advance(now)` (it must tick), then one recorded pump.
fn tick_at(qh: &mut QuicHost, clock: &mut TestClock, now: u64, all: &mut Vec<QuicPumpReport>) {
    assert!(qh.advance(now).expect("advance").ticked);
    pump_at(qh, clock, now, all);
}

/// The first transfer entry's `Begin`, once the whole snapshot has arrived,
/// then `Published` sent and pumped in, so no transfer is in flight.
fn publish_first_transfer(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    client: &mut MvClient,
    all: &mut Vec<QuicPumpReport>,
) {
    client.read_until("the first transfer", |client| {
        client
            .entries
            .iter()
            .any(|(channel, _)| *channel == Channel::Transfer)
    });
    let first = client
        .entries
        .iter()
        .find_map(|(channel, bytes)| (*channel == Channel::Transfer).then(|| bytes.clone()))
        .expect("a transfer frame");
    let TransferDown::Begin(begin) = transfer_v3::decode_down(&first).expect("decodes") else {
        panic!("the first transfer frame is a Begin");
    };
    let frames = 1 + usize::from(begin.chunk_count);
    client.read_until("the whole snapshot", |client| {
        client
            .entries
            .iter()
            .filter(|(channel, _)| *channel == Channel::Transfer)
            .count()
            >= frames
    });
    let published = transfer_v3::encode_up(&TransferUp::Published {
        epoch: begin.epoch,
        transfer: begin.transfer,
    })
    .expect("encodes");
    client
        .quic
        .try_send_channel(Channel::Transfer, &published)
        .expect("client transfer send");
    let steps = pump_until(qh, clock, 0, "published ingested", |_, report| {
        report.calls.iter().any(|call| {
            matches!(
                call,
                HostCall::IngestTransfer {
                    result: TransferDisposition::Published,
                    ..
                }
            )
        })
    });
    all.extend(reports_of(steps));
}

/// C-3's trace rule: every `IngestMove` trace is preceded in `calls` by the
/// `CheckFrame` of the same connection, binding and fence. Returns the
/// `(conn, fence)` of each, in order.
fn checked_ingests(reports: &[QuicPumpReport], peer: PeerHandle) -> Vec<(ConnectionId, u64)> {
    let mut out = Vec::new();
    for report in reports {
        for trace in &report.lane1 {
            let Lane1Call::IngestMove { conn, .. } = &trace.call else {
                continue;
            };
            assert!(trace.at > 0, "an ingest with no preceding call: {report:?}");
            let HostCall::CheckFrame {
                conn: checked,
                peer: checked_peer,
                fence,
                result,
            } = &report.calls[trace.at - 1]
            else {
                panic!("an ingest without a preceding CheckFrame: {report:?}");
            };
            assert_eq!(checked, conn);
            assert_eq!(*checked_peer, peer);
            assert_eq!(*result, Ok(()));
            out.push((*conn, fence.get()));
        }
    }
    out
}

fn receipt_bytes(seq: u64, processed_tick: u64, outcome: MoveOutcome) -> Vec<u8> {
    encode_down(&MoveDown::Receipt(MoveReceipt {
        epoch: e(1),
        seq,
        actor: n(1),
        processed_tick,
        outcome,
    }))
    .expect("encodes")
}

// ---------------------------------------------------------------------------
// QR1 `a_movement_hello_gets_a_ticket` (C-6, T155a H-1).
// ---------------------------------------------------------------------------

#[test]
fn a_movement_hello_gets_a_ticket() {
    let (mut qh, mut clock, invitation) = start_movement_resumable(ServerLimits::v1());
    let (mut client, decision, steps) = connect_movement(&mut qh, &mut clock, &inv().credential);
    let h1 = client.peer;
    assert_eq!(h1.get(), 1);
    assert_eq!(
        decision.outcome,
        HelloOutcome::Accepted {
            invitation,
            peer: h1
        }
    );

    // The first lane-stream frame after the welcome is the ticket, exactly.
    client.read_until("the ticket", |client| !client.entries.is_empty());
    let ticket = ticket4(e(1));
    assert_eq!(ticket.len(), 52);
    assert_eq!(client.entries[0], (Channel::Resume, ticket));

    // The hello pump bound a v4 session and issued the ticket.
    let report = &steps.last().expect("a hello pump").1;
    assert!(
        report.lane1.iter().any(|trace| trace.call
            == Lane1Call::BindV4Peer {
                conn: client.conn,
                invitation,
                result: Ok(h1),
            }),
        "{:?}",
        report.lane1
    );
    assert!(
        report.calls.contains(&HostCall::IssueResumeTicket {
            peer: h1,
            result: Ok(()),
        }),
        "{:?}",
        report.calls
    );

    // The adapter is not poisoned: the next pump is Ok.
    let now = clock.now() + 10;
    clock.set(now);
    assert!(qh.pump(now).is_ok());

    // Control: a `start` adapter over `bind_movement` sends no Resume frame.
    let (mut qh2, mut clock2, _) = movement_adapter(true);
    let (mut plain, _, _) = connect_movement(&mut qh2, &mut clock2, &inv().credential);
    plain.read_until("the initial transfer", |client| !client.entries.is_empty());
    assert!(
        plain
            .entries
            .iter()
            .all(|(channel, _)| *channel != Channel::Resume),
        "{:?}",
        plain.entries
    );
}

// ---------------------------------------------------------------------------
// QR2 `movement_silent_loss_then_resume` (C-1..C-3, C-6).
// ---------------------------------------------------------------------------

#[test]
fn movement_silent_loss_then_resume() {
    let (mut qh, mut clock, invitation) = start_movement_resumable(grace_limits());
    let relay = UdpRelay::start(qh.local_addr(), RelayConfig::clean());
    let (mut c1, _, steps) =
        connect_movement_via(&mut qh, &mut clock, relay.addr(), &inv().credential);
    let h1 = c1.peer;
    let mut all = reports_of(steps);
    c1.read_until("the ticket", |client| !client.entries.is_empty());
    assert_eq!(c1.entries[0], (Channel::Resume, ticket4(e(1))));
    publish_first_transfer(&mut qh, &mut clock, &mut c1, &mut all);

    // Seq 1 is admitted, decided and receipted; seq 2 is admitted only.
    let b1 = halt_bytes(e(1), 1, 1);
    let b2 = halt_bytes(e(1), 2, 1);
    c1.send_lane1(b1.clone());
    let now = clock.now() + 10;
    let (reports, traces) = pump_ingests(&mut qh, &mut clock, now, 1);
    all.extend(reports);
    assert_eq!(
        traces[0].call,
        Lane1Call::IngestMove {
            conn: c1.conn,
            peer: h1,
            frame: 1,
            len: b1.len(),
            result: MoveAdmission::Admitted { seq: 1 },
        }
    );
    tick_at(&mut qh, &mut clock, now + 10, &mut all);
    c1.read_until("R1", |client| !client.receipts.is_empty());
    let r1 = receipt_bytes(1, 0, MoveOutcome::Accepted { start_tick: 1 });
    assert_eq!(c1.receipts[0], r1);
    c1.send_lane1(b2.clone());
    let now = clock.now() + 10;
    let (reports, traces) = pump_ingests(&mut qh, &mut clock, now, 1);
    all.extend(reports);
    assert_eq!(
        traces[0].call,
        Lane1Call::IngestMove {
            conn: c1.conn,
            peer: h1,
            frame: 2,
            len: b2.len(),
            result: MoveAdmission::Admitted { seq: 2 },
        }
    );

    // The loss is silent: a grace, never an unbind.
    relay.set_blackhole(true);
    let steps = pump_until(&mut qh, &mut clock, 100, "begin grace", |_, report| {
        report
            .calls
            .iter()
            .any(|call| matches!(call, HostCall::BeginGrace { result: Ok(_), .. }))
    });
    let grace_calls = calls(&steps);
    let deadline = grace_calls
        .iter()
        .find_map(|call| match call {
            HostCall::BeginGrace {
                peer,
                result: Ok(d),
            } => {
                assert_eq!(*peer, h1);
                Some(*d)
            }
            _ => None,
        })
        .expect("a begin-grace deadline");
    assert!(
        grace_calls
            .iter()
            .all(|call| !matches!(call, HostCall::UnbindPeer { peer } if *peer == h1)),
        "a silent loss never unbinds"
    );
    all.extend(reports_of(steps));
    let t = deadline - 30_000;

    // Resume at T + 29_999, just inside the grace, on a movement connection.
    clock.set(t + 29_999);
    let request = encode_request(e(1), 1, &proof4(e(1)), &[(0, 0)]);
    let (resume, steps) = resume4(&mut qh, &mut clock, session4(), &request);
    let resumes: Vec<ResumeReport> = steps
        .iter()
        .flat_map(|(_, report)| report.resumes.clone())
        .collect();
    all.extend(reports_of(steps));
    let (quic, answer) = resume.expect("resume accepted");
    assert_eq!(answer, accepted_answer(e(1)));
    assert!(quic.movement());
    assert_eq!(resumes.len(), 1);
    let conn2 = resumes[0].conn;
    assert_eq!(
        resumes[0],
        ResumeReport {
            conn: conn2,
            outcome: ResumeOutcome::Accepted {
                invitation,
                peer: h1,
                granted: grant(2),
            },
        }
    );

    // The snapshot: the first transfer frame is a Begin with transfer id 2.
    let mut c2 = mv_client(quic, conn2, h1);
    c2.read_until("the snapshot", |client| {
        client
            .entries
            .iter()
            .any(|(channel, _)| *channel == Channel::Transfer)
    });
    let first = c2
        .entries
        .iter()
        .find_map(|(channel, bytes)| (*channel == Channel::Transfer).then(|| bytes.clone()))
        .expect("a transfer frame");
    let TransferDown::Begin(begin) = transfer_v3::decode_down(&first).expect("decodes") else {
        panic!("the first resume transfer frame is a Begin");
    };
    assert_eq!(begin.transfer.get(), 2);

    // The resumed binding is a movement binding: its lane 1 is drained,
    // fence-checked, and a resent intent already answered is a duplicate.
    c2.send_lane1(b2.clone());
    c2.send_lane1(b1.clone());
    let now = clock.now() + 10;
    let (reports, traces) = pump_ingests(&mut qh, &mut clock, now, 2);
    all.extend(reports);
    assert_eq!(
        traces[0].call,
        Lane1Call::IngestMove {
            conn: conn2,
            peer: h1,
            frame: 1,
            len: b2.len(),
            result: MoveAdmission::Admitted { seq: 2 },
        }
    );
    assert_eq!(
        traces[1].call,
        Lane1Call::IngestMove {
            conn: conn2,
            peer: h1,
            frame: 2,
            len: b1.len(),
            result: MoveAdmission::Duplicate {
                seq: 1,
                resent: true,
            },
        }
    );
    let tick = qh.host().server_tick();
    let now = clock.now() + 10;
    tick_at(&mut qh, &mut clock, now, &mut all);
    c2.read_until("R1 then the seq-2 receipt", |client| {
        client.receipts.len() >= 2
    });
    assert_eq!(c2.receipts[0], r1, "the cached receipt, byte for byte");
    assert_eq!(
        c2.receipts[1],
        receipt_bytes(
            2,
            tick,
            MoveOutcome::Accepted {
                start_tick: tick + 1
            }
        )
    );

    // A sample pass reaches the new connection.
    while qh.host().server_tick() % 20 != 0 {
        let now = clock.now() + 10;
        tick_at(&mut qh, &mut clock, now, &mut all);
    }
    c2.read_until("a datagram", |client| !client.datagrams.is_empty());

    // C-3: every lane-1 ingest follows a passing CheckFrame, fence 1 on the
    // old connection and 2 on the resumed one.
    let checked = checked_ingests(&all, h1);
    assert_eq!(
        checked,
        vec![(c1.conn, 1), (c1.conn, 1), (conn2, 2), (conn2, 2),]
    );
}

// ---------------------------------------------------------------------------
// QR3 `live_movement_resume_fences_the_old_connection` (C-1, C-2, C-4).
// ---------------------------------------------------------------------------

#[test]
fn live_movement_resume_fences_the_old_connection() {
    let (mut qh, mut clock, invitation) = start_movement_resumable(ServerLimits::v1());
    let (mut c1, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
    let h1 = c1.peer;
    c1.read_until("the ticket", |client| !client.entries.is_empty());
    assert_eq!(c1.entries[0], (Channel::Resume, ticket4(e(1))));

    // Nine intents: eight admitted, the ninth QueueFull and held.
    let frames: Vec<Vec<u8>> = (1..=9).map(|seq| halt_bytes(e(1), seq, 1)).collect();
    for bytes in &frames {
        c1.send_lane1(bytes.clone());
    }
    settle_lane1(&qh, &mut c1);
    let (reports, traces) = pump_ingests(&mut qh, &mut clock, CLOCK_START + 10, 9);
    assert_eq!(traces.len(), 9, "{traces:?}");
    for (index, trace) in traces.iter().enumerate() {
        let seq = index as u64 + 1;
        let result = if seq <= 8 {
            MoveAdmission::Admitted { seq }
        } else {
            MoveAdmission::Refused {
                seq: Some(seq),
                code: CodeV3::QueueFull,
            }
        };
        assert_eq!(
            trace.call,
            Lane1Call::IngestMove {
                conn: c1.conn,
                peer: h1,
                frame: seq,
                len: frames[index].len(),
                result,
            }
        );
    }
    let last = reports.last().expect("a pump");
    assert_eq!(last.lane1_held, 1);
    for report in &reports {
        assert!(
            report
                .rejections()
                .iter()
                .all(|rejection| !matches!(rejection, Rejection::Move { .. })),
            "a QueueFull is held, not rejected: {:?}",
            report.rejections()
        );
    }
    assert_eq!(qh.host().epoch(h1), Ok(e(1)));

    // A live resume with the ticket's fence retires the old connection,
    // discards its held lane-1 frame and never unbinds.
    let request = encode_request(e(1), 1, &proof4(e(1)), &[(0, 0)]);
    let (resume, steps) = resume4(&mut qh, &mut clock, session4(), &request);
    let (quic, answer) = resume.expect("resume accepted");
    assert_eq!(answer, accepted_answer(e(1)));
    let resumes: Vec<ResumeReport> = steps
        .iter()
        .flat_map(|(_, report)| report.resumes.clone())
        .collect();
    assert_eq!(resumes.len(), 1);
    let conn2 = resumes[0].conn;
    assert_eq!(
        resumes[0].outcome,
        ResumeOutcome::Accepted {
            invitation,
            peer: h1,
            granted: grant(2),
        }
    );
    let fences: Vec<Fence> = steps
        .iter()
        .flat_map(|(_, report)| report.fences.clone())
        .collect();
    assert_eq!(
        fences,
        vec![Fence {
            conn: c1.conn,
            peer: h1,
            cause: FenceCause::Resumed,
        }]
    );
    let discarded: usize = steps.iter().map(|(_, report)| report.lane1_discarded).sum();
    assert_eq!(discarded, 1, "the held lane-1 frame is discarded");
    assert_eq!(steps.last().expect("a pump").1.lane1_held, 0);
    assert!(
        calls(&steps)
            .iter()
            .all(|call| !matches!(call, HostCall::UnbindPeer { .. })),
        "a live resume never unbinds"
    );
    assert_eq!(qh.host().epoch(h1), Ok(e(1)));
    c1.read_until("the old connection's close", |client| {
        client.closed.is_some()
    });
    assert_eq!(c1.closed, Some(CloseReason::Peer { code: 9 }));

    // The resumed connection takes lane 1: the dropped slots are free again.
    let mut c2 = mv_client(quic, conn2, h1);
    c2.send_lane1(frames[0].clone());
    let now = clock.now() + 10;
    let (_, traces) = pump_ingests(&mut qh, &mut clock, now, 1);
    assert_eq!(
        traces[0].call,
        Lane1Call::IngestMove {
            conn: conn2,
            peer: h1,
            frame: 1,
            len: frames[0].len(),
            result: MoveAdmission::Admitted { seq: 1 },
        }
    );

    // Control: a fresh hello with `INV` supersedes and unbinds instead.
    let (mut qh2, mut clock2, _) = start_movement_resumable(ServerLimits::v1());
    let (d1, _, _) = connect_movement(&mut qh2, &mut clock2, &inv().credential);
    let (_, _, steps) = connect_movement(&mut qh2, &mut clock2, &inv().credential);
    let fences: Vec<Fence> = steps
        .iter()
        .flat_map(|(_, report)| report.fences.clone())
        .collect();
    assert_eq!(
        fences,
        vec![Fence {
            conn: d1.conn,
            peer: d1.peer,
            cause: FenceCause::Superseded,
        }]
    );
    assert!(
        calls(&steps)
            .iter()
            .any(|call| matches!(call, HostCall::UnbindPeer { peer } if *peer == d1.peer)),
        "a superseding hello unbinds"
    );
}

// ---------------------------------------------------------------------------
// QR4 `refused_lane1_frames_are_rejections` (C-5).
// ---------------------------------------------------------------------------

#[test]
fn refused_lane1_frames_are_rejections() {
    let clock = TestClock::new();
    let mut clock = clock;
    let mut qh = QuicHost::start(
        movement_host(),
        bind_movement_server(ServerLimits::v1()),
        clock.now(),
    )
    .expect("starts over a fresh host");
    qh.register_v4_invitation(v4_inv(
        &[0x52; 32],
        control(&[a()]),
        full_grants(&[a(), b(), c()]),
        facts(),
    ))
    .expect("registers the principal");
    let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &credential(&[0x52; 32]));
    let h1 = client.peer;

    let f1 = halt_bytes(e(1), 1, 2);
    let f2 = halt_bytes(epoch_of(7, 99), 7, 1);
    let f3 = vec![0xA1];
    let f4 = halt_bytes(e(1), 2, 1);
    for bytes in [&f1, &f2, &f3, &f4] {
        client.send_lane1(bytes.clone());
    }
    settle_lane1(&qh, &mut client);
    let (reports, traces) = pump_ingests(&mut qh, &mut clock, CLOCK_START + 10, 4);
    assert_eq!(traces.len(), 4, "{traces:?}");
    let Lane1Call::IngestMove {
        result: MoveAdmission::Refused { seq: None, code: k },
        ..
    } = traces[2].call
    else {
        panic!("frame 3 is refused with no sequence: {:?}", traces[2]);
    };
    assert_eq!(
        traces[0].call,
        Lane1Call::IngestMove {
            conn: client.conn,
            peer: h1,
            frame: 1,
            len: f1.len(),
            result: MoveAdmission::Rejected {
                seq: 1,
                code: CodeV3::NotAuthorized,
            },
        }
    );
    assert_eq!(
        traces[3].call,
        Lane1Call::IngestMove {
            conn: client.conn,
            peer: h1,
            frame: 4,
            len: f4.len(),
            result: MoveAdmission::Admitted { seq: 2 },
        }
    );

    let found: Vec<Rejection> = reports.iter().flat_map(|r| r.rejections()).collect();
    assert_eq!(
        found,
        vec![
            Rejection::Move {
                conn: client.conn,
                peer: h1,
                frame: 1,
                len: f1.len(),
                seq: Some(1),
                code: CodeV3::NotAuthorized,
            },
            Rejection::Move {
                conn: client.conn,
                peer: h1,
                frame: 2,
                len: f2.len(),
                seq: Some(7),
                code: CodeV3::SessionExpired,
            },
            Rejection::Move {
                conn: client.conn,
                peer: h1,
                frame: 3,
                len: 1,
                seq: None,
                code: k,
            },
        ]
    );
    let c = client.conn.get();
    assert_eq!(
        found[0].to_string(),
        format!(
            "move conn={c} peer=1 frame=1 len={} seq=1 code=17",
            f1.len()
        )
    );
    assert_eq!(
        found[1].to_string(),
        format!(
            "move conn={c} peer=1 frame=2 len={} seq=7 code=10",
            f2.len()
        )
    );
    assert_eq!(
        found[2].to_string(),
        format!("move conn={c} peer=1 frame=3 len=1 seq=- code={}", k as u8)
    );
}
