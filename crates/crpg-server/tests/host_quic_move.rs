//! T150c acceptance suite (`tasks/T150.md` C-1..C-11, Q1..Q11): the QUIC v4
//! route (`crpg-lane0/3`) over real loopback transport, public API only.
//!
//! The authority is the T037d movement fixture: `tests/fixtures/three_combatants.json`
//! edited in memory into `MV3rt` (movers and the movement definition) with
//! `M10` bound through `bind_nav_mesh`; no new fixture file. The host is
//! `Host::from_history` with `M10`, started on a `QuicServer::bind_movement`
//! loopback server with the rig's identity, at `CLOCK_START`. `INV` is the v4
//! invitation the client presents.
//!
//! Oracles are independent of the adapter: a **twin** in-memory `Host` from
//! the same JSON with the same mesh, driven by replaying the pump reports
//! (`calls` and `lane1` interleaved by `at`); hand-written expected receipts
//! and rule-based expected counts. Every wait is a 30 s failure guard, never
//! an oracle; no case sleeps and no case asserts a count that depends on read
//! chunking.

#[path = "support/quic_host_rig.rs"]
mod rig;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_nav::{bake_box, NavMesh, NavRect};
use crpg_net::codec_v4;
use crpg_net::facts_v3::EncounterGrants;
use crpg_net::movement_v4::{
    decode_down, encode_down, encode_intent, MoveBody, MoveDown, MoveIntent, MoveOutcome,
    MoveReceipt,
};
use crpg_net::outbox_v4::MoveOutbox;
use crpg_net::protocol::{NetId, LANE_COMBAT};
use crpg_net::protocol_v3::{PauseRights, RejectionCode as CodeV3};
use crpg_net::protocol_v4::{IntentBody, IntentFrame, Point};
use crpg_net_quic::{
    Channel, ClientLimits, CloseCode, CloseMode, CloseReason, ConnectError, ConnectionId,
    Credential, Framing, QuicClient, QuicServer, ServerConfig, ServerLimits, Welcome,
    MOVEMENT_WIRE_VERSION,
};
use crpg_server::host::{
    ControlGrant, DeliveryKind, DisclosureGrants, EntityDisclosure, EntityFactGrant, FactGrants,
    Host, HostError, IngestDisposition, MoveAdmission, PeerHandle, DEFAULT_HOST_GRANTS,
    MAX_CONTROLLED_PER_PEER,
};
use crpg_server::quic::{
    Fence, FenceCause, GrantChange, HelloDecision, HelloOutcome, HostCall, Invitation,
    InvitationId, Lane1Call, Lane1Trace, QuicHost, QuicHostError, QuicPumpReport, V4Invitation,
    MAX_DRAIN_PER_CONNECTION, MAX_INVITATIONS,
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
fn strike() -> Ulid {
    Ulid::from_u128(603)
}
fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}
/// The raw wire point `[x, y, z]`.
fn p(x: i32, y: i32, z: i32) -> Point {
    Point::new([x, y, z]).expect("within the wire bound")
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
    /// Every lane-0 frame sent, in order: frame ordinal k is `lane0[k - 1]`.
    lane0: Vec<Vec<u8>>,
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
    fn send_lane0(&mut self, bytes: Vec<u8>) {
        self.quic
            .try_send_channel(Channel::Lane0, &bytes)
            .expect("client lane-0 send");
        self.lane0.push(bytes);
    }

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
        lane0: Vec::new(),
        lane1: Vec::new(),
        entries: Vec::new(),
        receipts: Vec::new(),
        datagrams: Vec::new(),
        closed: None,
    };
    (client, decision, steps)
}

/// Attempts a movement connection the adapter refuses.
fn connect_movement_refused(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    credential: &Credential,
) -> (ConnectError, HelloDecision, Vec<Step>) {
    let addr = qh.local_addr();
    let hello = crpg_net_quic::Hello {
        wire_version: MOVEMENT_WIRE_VERSION,
        credential: credential.clone(),
    };
    let pending = std::thread::spawn(move || {
        QuicClient::connect_movement(client_config(ClientLimits::v1()), addr, &hello)
    });
    let (decision, steps) = decide_hello(qh, clock, 0);
    let error = match pending.join().expect("client thread") {
        Ok(_) => panic!("expected a refusal, got a connection ({decision:?})"),
        Err(error) => error,
    };
    (error, decision, steps)
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

// ---------------------------------------------------------------------------
// The twin: an in-memory host replayed from the reports (Reading 13).
// ---------------------------------------------------------------------------

struct Twin {
    host: Host,
    /// The v4 invitation's arguments, as `register_v4_invitation` stored them.
    control: ControlGrant,
    grants: DisclosureGrants,
    facts: FactGrants,
    rights: PauseRights,
    /// Lane-0 frames sent, per connection, by ordinal.
    lane0: BTreeMap<ConnectionId, Vec<Vec<u8>>>,
    /// Lane-1 frames sent, per connection, by ordinal.
    lane1: BTreeMap<ConnectionId, Vec<Vec<u8>>>,
    /// Every receipt `TakeMoveReceipts` returned, in order.
    receipts: Vec<Vec<u8>>,
    /// Every datagram `TakeSamples` returned, in order.
    samples: Vec<Vec<u8>>,
    /// The last take of each binding, by delivery ordinal source.
    last_take: BTreeMap<PeerHandle, Vec<Vec<u8>>>,
}

impl Twin {
    fn new() -> Self {
        Self {
            host: movement_host(),
            control: control(&[a(), b(), c()]),
            grants: full_grants(&[a(), b(), c()]),
            facts: facts(),
            rights: r1(),
            lane0: BTreeMap::new(),
            lane1: BTreeMap::new(),
            receipts: Vec::new(),
            samples: Vec::new(),
            last_take: BTreeMap::new(),
        }
    }

    fn note(&mut self, client: &MvClient) {
        self.lane0.insert(client.conn, client.lane0.clone());
        self.lane1.insert(client.conn, client.lane1.clone());
    }

    fn advance(&mut self, now: u64) -> Result<crpg_server::host::AdvanceReport, HostError> {
        self.host.advance(now)
    }

    fn apply(&mut self, now: u64, report: &QuicPumpReport) -> Result<(), String> {
        let mut next = 0usize;
        let mut index = 0usize;
        loop {
            while let Some(trace) = report.lane1.get(index) {
                if trace.at != next {
                    break;
                }
                self.apply_lane1(now, trace)?;
                index += 1;
            }
            match report.calls.get(next) {
                Some(call) => {
                    self.apply_call(now, call)?;
                    next += 1;
                }
                None => break,
            }
        }
        if let Some(trace) = report.lane1.get(index) {
            return Err(format!("lane-1 trace past the call list: {trace:?}"));
        }
        Ok(())
    }

    fn apply_call(&mut self, now: u64, call: &HostCall) -> Result<(), String> {
        match call {
            HostCall::BindPeer {
                invitation, result, ..
            } => {
                let got = self
                    .host
                    .bind_peer(self.control.clone(), self.grants.clone(), now);
                if got != *result {
                    let _ = invitation;
                    return Err(format!("bind {got:?} != {result:?}"));
                }
            }
            HostCall::BindSnapshotPeer {
                invitation, result, ..
            } => {
                let got =
                    self.host
                        .bind_snapshot_peer(self.control.clone(), self.grants.clone(), now);
                if got != *result {
                    let _ = invitation;
                    return Err(format!("snapshot bind {got:?} != {result:?}"));
                }
            }
            HostCall::UnbindPeer { peer } => self.host.unbind_peer(*peer),
            HostCall::Ingest {
                conn,
                peer,
                frame,
                len,
                result,
            } => {
                let bytes = self
                    .lane0
                    .get(conn)
                    .and_then(|log| log.get(*frame as usize - 1))
                    .ok_or(format!("no lane-0 frame {frame} on {conn:?}"))?;
                if bytes.len() != *len {
                    return Err("lane-0 length differs".to_owned());
                }
                let got = self.host.ingest(*peer, bytes, now);
                if got.as_ref() != Ok(result) {
                    return Err(format!("ingest {got:?} != {result:?}"));
                }
            }
            HostCall::IngestTransfer {
                conn,
                peer,
                frame,
                len,
                result,
            } => {
                let bytes = self
                    .lane0
                    .get(conn)
                    .and_then(|log| log.get(*frame as usize - 1))
                    .ok_or(format!("no lane-0 frame {frame} on {conn:?}"))?;
                if bytes.len() != *len {
                    return Err("lane-0 length differs".to_owned());
                }
                let got = self.host.ingest_transfer(*peer, bytes, now);
                if got.as_ref() != Ok(result) {
                    return Err(format!("transfer {got:?} != {result:?}"));
                }
            }
            HostCall::Pump { summary } => {
                let got = self.host.pump(now);
                if got.as_ref() != Ok(summary) {
                    return Err(format!("pump {got:?} != {summary:?}"));
                }
            }
            HostCall::TakeDelivery { peer, result } => {
                let got = self.host.take_delivery(*peer);
                let count = got.as_ref().map(Vec::len).map_err(Clone::clone);
                if count != *result {
                    return Err(format!("take {count:?} != {result:?}"));
                }
                if let Ok(entries) = got {
                    self.last_take.insert(*peer, entries);
                }
            }
            HostCall::AcknowledgeDelivery { peer, through } => {
                self.host
                    .acknowledge_delivery(*peer, *through)
                    .map_err(|error| format!("ack {error:?}"))?;
                let _ = self.last_take.get(peer);
            }
            HostCall::IssueResumeTicket { .. }
            | HostCall::CheckFrame { .. }
            | HostCall::BeginGrace { .. }
            | HostCall::ResumePeer { .. } => {
                return Err("resume call in a start trace".to_owned());
            }
        }
        Ok(())
    }

    fn apply_lane1(&mut self, now: u64, trace: &Lane1Trace) -> Result<(), String> {
        match &trace.call {
            Lane1Call::BindV4Peer {
                conn,
                invitation,
                result,
            } => {
                let _ = (conn, invitation);
                let got = self.host.bind_v4_peer(
                    self.control.clone(),
                    self.grants.clone(),
                    self.facts.clone(),
                    self.rights,
                    now,
                );
                if got != *result {
                    return Err(format!("bind_v4_peer {got:?} != {result:?}"));
                }
            }
            Lane1Call::IngestMove {
                conn,
                peer,
                frame,
                len,
                result,
            } => {
                let bytes = self
                    .lane1
                    .get(conn)
                    .and_then(|log| log.get(*frame as usize - 1))
                    .ok_or(format!("no lane-1 frame {frame} on {conn:?}"))?;
                if bytes.len() != *len {
                    return Err("lane-1 length differs".to_owned());
                }
                let got = self.host.ingest_move(*peer, bytes, now);
                if got.as_ref() != Ok(result) {
                    return Err(format!("ingest_move {got:?} != {result:?}"));
                }
            }
            Lane1Call::TakeMoveReceipts { peer, result } => {
                let got = self.host.take_move_receipts(*peer);
                let count = got.as_ref().map(Vec::len).map_err(Clone::clone);
                if count != *result {
                    return Err(format!("take receipts {count:?} != {result:?}"));
                }
                if let Ok(receipts) = got {
                    self.receipts.extend(receipts);
                }
            }
            Lane1Call::TakeSamples { peer, result } => {
                let got = self.host.take_samples(*peer);
                let count = got.as_ref().map(Vec::len).map_err(Clone::clone);
                if count != *result {
                    return Err(format!("take samples {count:?} != {result:?}"));
                }
                if let Ok(samples) = got {
                    self.samples.extend(samples);
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Lane-1 and lane-0 byte builders.
// ---------------------------------------------------------------------------

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

fn move_to_bytes(epoch: [u8; 16], seq: u64, actor: u64, to: Point) -> Vec<u8> {
    encode_intent(&MoveIntent {
        epoch,
        seq,
        actor: n(actor),
        body: MoveBody::MoveTo { destination: to },
    })
    .expect("encodes")
}

/// A v4 lane-0 `DeclareAction { strike, n(2) }` by `n(1)`, seq 1.
fn declare_strike_bytes(epoch: [u8; 16]) -> Vec<u8> {
    codec_v4::encode_intent(&IntentFrame {
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: n(1),
        body: IntentBody::DeclareAction {
            ability: strike(),
            target: n(2),
        },
    })
    .expect("encodes")
}

/// A v4 lane-0 `EndTurn` for `n(1)`, seq 1.
fn end_turn_bytes(epoch: [u8; 16]) -> Vec<u8> {
    codec_v4::encode_intent(&IntentFrame {
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: n(1),
        body: IntentBody::EndTurn,
    })
    .expect("encodes")
}

/// Decodes one lane-1 frame as a receipt.
fn receipt_of(bytes: &[u8]) -> MoveReceipt {
    match decode_down(bytes).expect("decodes") {
        MoveDown::Receipt(receipt) => receipt,
        other => panic!("expected a receipt, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Q1: v4 binds over the movement endpoint.
// ---------------------------------------------------------------------------

#[test]
fn q1_v4_binds_over_movement_endpoint() {
    let (mut qh, mut clock, invitation) = movement_adapter(true);
    let invitation = invitation.expect("INV registered");
    let (mut client, decision, steps) = connect_movement(&mut qh, &mut clock, &inv().credential);
    let conn = client.conn;
    let peer = client.peer;
    assert_eq!(decision.conn, conn);
    let report = &steps.last().expect("a hello pump").1;

    // The hello bound a v4 session.
    assert_eq!(
        report.hellos,
        vec![HelloDecision {
            conn,
            outcome: HelloOutcome::Accepted { invitation, peer },
        }]
    );
    assert_eq!(qh.host().session_wire(peer), Ok(4));
    assert_eq!(
        client.quic.welcome(),
        Welcome {
            wire_version: 4,
            epoch: e(1),
        }
    );

    // The twin binds the same way and yields the first delivery.
    let mut twin = Twin::new();
    let twin_peer = twin
        .host
        .bind_v4_peer(
            control(&[a(), b(), c()]),
            full_grants(&[a(), b(), c()]),
            facts(),
            r1(),
            CLOCK_START,
        )
        .expect("twin binds");
    let twin_entries = twin
        .host
        .take_delivery_entries(twin_peer)
        .expect("twin takes its first delivery");

    // `calls` is the lane-0 summary, with no bind on it.
    assert_eq!(report.calls.len(), 3, "{:?}", report.calls);
    assert!(matches!(report.calls[0], HostCall::Pump { .. }));
    match &report.calls[1] {
        HostCall::TakeDelivery { peer: p, result } => {
            assert_eq!(*p, peer);
            assert_eq!(*result, Ok(twin_entries.len()));
        }
        other => panic!("expected TakeDelivery, got {other:?}"),
    }
    assert!(matches!(
        &report.calls[2],
        HostCall::AcknowledgeDelivery { peer: p, .. } if *p == peer
    ));
    assert!(
        !report.calls.iter().any(|call| matches!(
            call,
            HostCall::BindPeer { .. } | HostCall::BindSnapshotPeer { .. }
        )),
        "a /3 hello binds nothing on lane 0: {:?}",
        report.calls
    );

    // Lane 1 is the side trace: the bind before `calls[0]`, the takes after
    // the delivery's two calls.
    assert_eq!(
        report.lane1,
        vec![
            Lane1Trace {
                at: 0,
                call: Lane1Call::BindV4Peer {
                    conn,
                    invitation,
                    result: Ok(peer),
                },
            },
            Lane1Trace {
                at: 3,
                call: Lane1Call::TakeMoveReceipts {
                    peer,
                    result: Ok(0),
                },
            },
            Lane1Trace {
                at: 3,
                call: Lane1Call::TakeSamples {
                    peer,
                    result: Ok(0),
                },
            },
        ]
    );

    // Every byte the client reads equals the twin's first delivery.
    client.read_until("initial delivery", |client| {
        client.entries.len() >= twin_entries.len()
    });
    assert_eq!(
        client.entries.len(),
        twin_entries.len(),
        "{:?}",
        client.entries
    );
    for (got, expected) in client.entries.iter().zip(&twin_entries) {
        let channel = match expected.kind {
            DeliveryKind::Delta => Channel::Lane0,
            DeliveryKind::Transfer => Channel::Transfer,
        };
        assert_eq!(got, &(channel, expected.bytes.clone()));
    }
}

// ---------------------------------------------------------------------------
// Q2: hello and invitation precedence.
// ---------------------------------------------------------------------------

#[test]
fn q2_hello_and_invitation_precedence() {
    // Not buildable: a `/3` hello naming a version other than 4 —
    // `connect_movement` refuses a non-4 hello with `Handshake` before any
    // socket is bound (T150b N-4), so no public client sends one.
    // Not buildable: `Poisoned` beside another fault — the adapter poisons
    // only on a host `Invariant`, which no fixture triggers.
    // (a) A 32-byte credential nobody registered.
    {
        let (mut qh, mut clock, _) = movement_adapter(true);
        let unregistered = credential(&[0x99; 32]);
        let (error, decision, steps) = connect_movement_refused(&mut qh, &mut clock, &unregistered);
        assert_eq!(error, ConnectError::Refused { code: 5 });
        assert_eq!(
            decision.outcome,
            HelloOutcome::Refused {
                code: CloseCode::AuthRefused
            }
        );
        assert!(steps.last().expect("a hello pump").1.lane1.is_empty());
    }
    // (b) A credential registered with `register_invitation` never binds on
    // `/3` (Reading 11): the v4 record is missing.
    {
        let (mut qh, mut clock, _) = movement_adapter(false);
        let plain = credential(&[0x71; 32]);
        qh.register_invitation(Invitation {
            credential: plain.clone(),
            control: control(&[a()]),
            grants: full_grants(&[a()]),
        })
        .expect("registers a v3 invitation");
        let (error, decision, steps) = connect_movement_refused(&mut qh, &mut clock, &plain);
        assert_eq!(error, ConnectError::Refused { code: 5 });
        assert_eq!(
            decision.outcome,
            HelloOutcome::Refused {
                code: CloseCode::AuthRefused
            }
        );
        let report = &steps.last().expect("a hello pump").1;
        assert!(report.lane1.is_empty());
        assert!(
            !report.calls.iter().any(|call| matches!(
                call,
                HostCall::BindPeer { .. } | HostCall::BindSnapshotPeer { .. }
            )),
            "no bind: {:?}",
            report.calls
        );
    }
    // (c) A second client presenting `INV` fences the first as Superseded.
    {
        let (mut qh, mut clock, invitation) = movement_adapter(true);
        let invitation = invitation.expect("INV registered");
        let (mut first, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
        let (second, _, steps) = connect_movement(&mut qh, &mut clock, &inv().credential);
        let report = &steps.last().expect("a hello pump").1;
        assert_eq!(
            report.fences,
            vec![Fence {
                conn: first.conn,
                peer: first.peer,
                cause: FenceCause::Superseded,
            }]
        );
        let binds: Vec<&Lane1Trace> = report
            .lane1
            .iter()
            .filter(|trace| matches!(trace.call, Lane1Call::BindV4Peer { .. }))
            .collect();
        assert_eq!(binds.len(), 1, "{:?}", report.lane1);
        assert!(matches!(
            &binds[0].call,
            Lane1Call::BindV4Peer {
                invitation: i,
                result: Ok(peer),
                ..
            } if *i == invitation && *peer == second.peer
        ));
        // C-2 (6)/H07: the bind trace's `at` is the call list's length at the
        // bind, after the supersession's `UnbindPeer` (calls[0]).
        assert_eq!(binds[0].at, 1, "{:?}", report.lane1);
        assert!(
            matches!(report.calls.first(), Some(HostCall::UnbindPeer { .. })),
            "the supersession is calls[0]: {:?}",
            report.calls
        );
        // The superseded client is closed with `SessionFenced`.
        let reason = client_close_reason(&mut first);
        assert_eq!(reason, CloseReason::Peer { code: 9 });
    }
    // (d) `register_v4_invitation`'s precedence pairs.
    let too_many = control(
        &(0..=MAX_CONTROLLED_PER_PEER as u32)
            .map(eid)
            .collect::<Vec<_>>(),
    );
    let ok_control = control(&[a()]);
    let ok_grants = full_grants(&[a(), b(), c()]);
    let mut duplicate_grants = ok_grants.clone();
    duplicate_grants.entities.push(disclosure(a()));
    let mut duplicate_facts = facts();
    duplicate_facts.entities.push(all_fact_flags(a()));
    let ok_facts = facts();

    // Limit before the credential length.
    {
        let (mut qh, _, _) = movement_adapter(false);
        for index in 0..MAX_INVITATIONS as u8 {
            qh.register_v4_invitation(v4_inv(
                &[0x10 + index; 32],
                ok_control.clone(),
                ok_grants.clone(),
                ok_facts.clone(),
            ))
            .expect("registers up to the limit");
        }
        assert_eq!(
            qh.register_v4_invitation(v4_inv(
                &[0xEE; 31],
                ok_control.clone(),
                ok_grants.clone(),
                ok_facts.clone(),
            )),
            Err(QuicHostError::InvitationLimit)
        );
    }
    // Credential length before control.
    {
        let (mut qh, _, _) = movement_adapter(false);
        assert_eq!(
            qh.register_v4_invitation(v4_inv(
                &[0xEE; 31],
                too_many.clone(),
                ok_grants.clone(),
                ok_facts.clone(),
            )),
            Err(QuicHostError::InvalidCredential)
        );
    }
    // Control before grants.
    {
        let (mut qh, _, _) = movement_adapter(false);
        assert_eq!(
            qh.register_v4_invitation(v4_inv(
                &[0x21; 32],
                too_many.clone(),
                duplicate_grants.clone(),
                ok_facts.clone(),
            )),
            Err(QuicHostError::Host(HostError::InvalidControl))
        );
    }
    // Disclosure grants before fact grants.
    {
        let (mut qh, _, _) = movement_adapter(false);
        assert_eq!(
            qh.register_v4_invitation(v4_inv(
                &[0x22; 32],
                ok_control.clone(),
                duplicate_grants.clone(),
                duplicate_facts.clone(),
            )),
            Err(QuicHostError::Host(HostError::InvalidGrants))
        );
        // The control: the duplicated fact entity alone is `InvalidGrants`.
        assert_eq!(
            qh.register_v4_invitation(v4_inv(
                &[0x23; 32],
                ok_control.clone(),
                ok_grants.clone(),
                duplicate_facts.clone(),
            )),
            Err(QuicHostError::Host(HostError::InvalidGrants))
        );
    }
    // C-1: the operator calls work on a v4 record unchanged; a live `/3`
    // binding is `V2`, so narrowing stages a snapshot instead of fencing.
    {
        let (mut qh, mut clock, invitation) = movement_adapter(true);
        let invitation = invitation.expect("INV registered");
        let (_client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
        assert_eq!(
            qh.set_invitation_grants(invitation, DisclosureGrants::default()),
            Ok(GrantChange::SnapshotStaged)
        );
        assert!(qh
            .invitation_binding(invitation)
            .expect("known invitation")
            .is_some());
        let fence = qh
            .revoke_invitation(invitation)
            .expect("revokes")
            .expect("a live binding is fenced");
        assert_eq!(fence.cause, FenceCause::Revoked);
        assert_eq!(
            qh.invitation_binding(invitation),
            Err(QuicHostError::UnknownInvitation)
        );
    }
    // Fact grants before the duplicate credential.
    {
        let (mut qh, _, _) = movement_adapter(false);
        let registered = credential(&[0x51; 32]);
        qh.register_v4_invitation(v4_inv(
            registered.as_bytes(),
            ok_control.clone(),
            ok_grants.clone(),
            ok_facts.clone(),
        ))
        .expect("registers INV");
        assert_eq!(
            qh.register_v4_invitation(v4_inv(
                registered.as_bytes(),
                ok_control.clone(),
                ok_grants.clone(),
                duplicate_facts.clone(),
            )),
            Err(QuicHostError::Host(HostError::InvalidGrants))
        );
        assert_eq!(
            qh.register_v4_invitation(v4_inv(
                registered.as_bytes(),
                ok_control.clone(),
                ok_grants.clone(),
                ok_facts.clone(),
            )),
            Err(QuicHostError::DuplicateCredential)
        );
    }
}

/// Reads a client's lane 1 to the end of the connection.
fn client_close_reason(client: &mut MvClient) -> CloseReason {
    let start = Instant::now();
    loop {
        match client.quic.try_recv_lane1() {
            Ok(Some(frame)) => client.receipts.push(frame),
            Ok(None) => {}
            Err(reason) => return reason,
        }
        assert!(start.elapsed() < DEADLINE, "deadline: lane-1 close");
        client.quic.wait(Duration::from_millis(5));
    }
}

// ---------------------------------------------------------------------------
// Q3/Q4: the move round trip and the trace replay.
// ---------------------------------------------------------------------------

/// Applies every report of a step list to the twin.
fn replay_steps(twin: &mut Twin, steps: &[Step]) {
    for (now, report) in steps {
        twin.apply(*now, report).expect("the twin replays the pump");
    }
}

/// Applies a pump batch at one `now` to the twin.
fn replay_reports(twin: &mut Twin, now: u64, reports: &[QuicPumpReport]) {
    for report in reports {
        twin.apply(now, report).expect("the twin replays the pump");
    }
}

/// Q3's script, shared with Q4: one `MoveTo` admitted at `+10`, decided at
/// `+20`, its receipt and sample delivered, then ticks to tick 40 with the
/// twin in step. Returns the receipt bytes the client read.
fn q3_script(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    client: &mut MvClient,
    twin: &mut Twin,
) -> Vec<u8> {
    let mut outbox = MoveOutbox::new(e(1));
    let i1 = outbox
        .send(
            n(1),
            MoveBody::MoveTo {
                destination: p(262_144, 0, 65_536),
            },
        )
        .expect("mints seq 1");
    let i1_bytes = encode_intent(&i1).expect("encodes");
    client.send_lane1(i1_bytes.clone());
    twin.note(client);

    let now = CLOCK_START + 10;
    let (reports, traces) = pump_ingests(qh, clock, now, 1);
    replay_reports(twin, now, &reports);
    assert_eq!(traces.len(), 1, "{traces:?}");
    assert_eq!(
        traces[0].call,
        Lane1Call::IngestMove {
            conn: client.conn,
            peer: client.peer,
            frame: 1,
            len: i1_bytes.len(),
            result: MoveAdmission::Admitted { seq: 1 },
        }
    );

    let now = CLOCK_START + 20;
    let advance = qh.advance(now).expect("advance decides the window");
    assert!(advance.ticked, "{advance:?}");
    assert_eq!(
        twin.advance(now).expect("twin advance"),
        advance,
        "the twin advances the same way"
    );
    clock.set(now);
    let report = qh.pump(now).expect("pump");
    twin.apply(now, &report).expect("replay");
    assert!(
        report.lane1.iter().any(|trace| trace.call
            == Lane1Call::TakeMoveReceipts {
                peer: client.peer,
                result: Ok(1),
            }),
        "{:?}",
        report.lane1
    );
    assert!(
        report.lane1.iter().any(|trace| trace.call
            == Lane1Call::TakeSamples {
                peer: client.peer,
                result: Ok(1),
            }),
        "{:?}",
        report.lane1
    );
    assert_eq!(report.lane1_sent, 1);
    assert_eq!(report.datagrams_sent, 1);

    let expected = encode_down(&MoveDown::Receipt(MoveReceipt {
        epoch: e(1),
        seq: 1,
        actor: n(1),
        processed_tick: 0,
        outcome: MoveOutcome::Accepted { start_tick: 1 },
    }))
    .expect("encodes");
    client.read_until("the receipt", |client| !client.receipts.is_empty());
    assert_eq!(client.receipts[0], expected);
    let receipt = receipt_of(&client.receipts[0]);
    outbox.receive(&receipt).expect("the outbox accepts it");
    assert_eq!(outbox.processed_seq(), 1);

    // C-4 (2): two decisions between pumps still replace the sample queue
    // (T037c C6), and the pump sends every datagram the queue holds.
    let now = clock.now() + 20;
    let advance = qh.advance(now).expect("advance");
    assert!(advance.ticked, "{advance:?}");
    assert_eq!(twin.advance(now).expect("twin advance"), advance);
    let now = now + 20;
    let advance = qh.advance(now).expect("advance");
    assert!(advance.ticked, "{advance:?}");
    assert_eq!(twin.advance(now).expect("twin advance"), advance);
    clock.set(now);
    let report = qh.pump(now).expect("pump");
    twin.apply(now, &report).expect("replay");
    let samples = report
        .lane1
        .iter()
        .find_map(|trace| match trace.call {
            Lane1Call::TakeSamples { result: Ok(n), .. } => Some(n),
            _ => None,
        })
        .expect("a sample take");
    assert_eq!(
        samples, 1,
        "the later tick's sample replaced the first's: {report:?}"
    );
    assert_eq!(report.datagrams_sent, samples, "{report:?}");
    let receipts = report
        .lane1
        .iter()
        .find_map(|trace| match trace.call {
            Lane1Call::TakeMoveReceipts { result: Ok(n), .. } => Some(n),
            _ => None,
        })
        .expect("a receipt take");
    assert_eq!(report.lane1_sent, receipts, "{report:?}");
    client.read_available();

    while qh.host().server_tick() < 40 {
        let now = clock.now() + 20;
        let advance = qh.advance(now).expect("advance");
        assert!(advance.ticked, "{advance:?}");
        assert_eq!(twin.advance(now).expect("twin advance"), advance);
        clock.set(now);
        let report = qh.pump(now).expect("pump");
        twin.apply(now, &report).expect("replay");
        client.read_available();
    }
    client.read_until("a sample", |client| !client.datagrams.is_empty());
    for datagram in &client.datagrams {
        assert!(
            twin.samples.contains(datagram),
            "a datagram differs from the twin's: {datagram:?}"
        );
    }
    client.receipts[0].clone()
}

#[test]
fn q3_move_round_trip() {
    let (mut qh, mut clock, _) = movement_adapter(true);
    let (mut client, _, steps) = connect_movement(&mut qh, &mut clock, &inv().credential);
    let mut twin = Twin::new();
    twin.note(&client);
    replay_steps(&mut twin, &steps);
    q3_script(&mut qh, &mut clock, &mut client, &mut twin);
}

#[test]
fn q4_trace_replays() {
    let (mut qh, mut clock, _) = movement_adapter(true);
    let (mut client, _, steps) = connect_movement(&mut qh, &mut clock, &inv().credential);
    let mut twin = Twin::new();
    twin.note(&client);
    replay_steps(&mut twin, &steps);
    q3_script(&mut qh, &mut clock, &mut client, &mut twin);

    // A lane-0 `DeclareAction { strike, n(2) }` by `n(1)` and a lane-1 `Halt`.
    client.send_lane0(declare_strike_bytes(e(1)));
    client.send_lane1(halt_bytes(e(1), 2, 1));
    twin.note(&client);
    let now = clock.now() + 10;
    clock.set(now);
    let start = Instant::now();
    let mut reports = Vec::new();
    let mut lane0 = false;
    let mut lane1 = false;
    loop {
        let report = qh.pump(now).expect("pump");
        lane0 |= report
            .calls
            .iter()
            .any(|call| matches!(call, HostCall::Ingest { .. }));
        lane1 |= report.lane1.iter().any(|trace| {
            matches!(
                &trace.call,
                Lane1Call::IngestMove {
                    frame: 2,
                    result: MoveAdmission::Admitted { seq: 2 },
                    ..
                }
            )
        });
        reports.push(report);
        if lane0 && lane1 {
            break;
        }
        assert!(
            start.elapsed() < DEADLINE,
            "deadline: command frames {reports:#?}"
        );
        qh.wait(Duration::from_millis(5));
    }
    replay_reports(&mut twin, now, &reports);

    // Decide the halt's slot and take its receipt.
    let now = now + 20;
    let advance = qh.advance(now).expect("advance");
    assert_eq!(twin.advance(now).expect("twin advance"), advance);
    clock.set(now);
    let report = qh.pump(now).expect("pump");
    twin.apply(now, &report).expect("replay");

    client.read_until("every receipt", |client| {
        client.receipts.len() >= twin.receipts.len()
    });
    assert_eq!(client.receipts, twin.receipts, "the receipts, in order");
    assert_eq!(qh.host().authority_hash(), twin.host.authority_hash());
}

// ---------------------------------------------------------------------------
// Q5: a refused lane-1 frame is held and retried before any later one.
// ---------------------------------------------------------------------------

/// Pump 1 of Q5: nine `Halt`s sent before any advance; frame 1--8 admitted,
/// frame 9 `QueueFull` and held, all in one pump (C-3's per-pump drain bound).
/// Returns the nine frames' bytes.
fn q5_held_state(qh: &mut QuicHost, clock: &mut TestClock, client: &mut MvClient) -> Vec<Vec<u8>> {
    let expected: Vec<Vec<u8>> = (1..=9).map(|seq| halt_bytes(e(1), seq, 1)).collect();
    for bytes in &expected {
        client.send_lane1(bytes.clone());
    }
    settle_lane1(qh, client);
    let now = CLOCK_START + 10;
    clock.set(now);
    let report = qh.pump(now).expect("pump");
    let traces: Vec<Lane1Trace> = report
        .lane1
        .iter()
        .filter(|trace| matches!(trace.call, Lane1Call::IngestMove { .. }))
        .cloned()
        .collect();
    assert_eq!(
        traces.len(),
        9,
        "the whole lane-1 backlog drains in the first pump: {report:?}"
    );
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
                conn: client.conn,
                peer: client.peer,
                frame: seq,
                len: expected[index].len(),
                result,
            }
        );
    }
    assert_eq!(report.lane1_held, 1);
    expected
}

#[test]
fn q5_queue_full_is_held_and_retried() {
    let (mut qh, mut clock, _) = movement_adapter(true);
    let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
    let expected = q5_held_state(&mut qh, &mut clock, &mut client);

    // Pump 2: the held frame is the first lane-1 trace, still refused.
    let now = CLOCK_START + 20;
    clock.set(now);
    let report = qh.pump(now).expect("pump");
    let first = &report.lane1[0];
    assert!(
        matches!(
            &first.call,
            Lane1Call::IngestMove {
                frame: 9,
                result: MoveAdmission::Refused {
                    seq: Some(9),
                    code: CodeV3::QueueFull,
                },
                ..
            }
        ),
        "{first:?}"
    );
    assert_eq!(first.at, 0);
    assert_eq!(report.lane1_held, 1);

    // Pump 3: eight receipts were released but not taken, so frame 9 still
    // does not fit; then the eight are taken.
    let now = CLOCK_START + 30;
    assert!(qh.advance(now).expect("advance").ticked);
    clock.set(now);
    let report = qh.pump(now).expect("pump");
    assert!(
        matches!(
            &report.lane1[0].call,
            Lane1Call::IngestMove {
                frame: 9,
                result: MoveAdmission::Refused {
                    code: CodeV3::QueueFull,
                    ..
                },
                ..
            }
        ),
        "{:?}",
        report.lane1[0]
    );
    assert!(
        report.lane1.iter().any(|trace| trace.call
            == Lane1Call::TakeMoveReceipts {
                peer: client.peer,
                result: Ok(8),
            }),
        "{:?}",
        report.lane1
    );
    assert_eq!(report.lane1_sent, 8);

    // Pump 4: the retried frame now fits.
    let now = CLOCK_START + 40;
    clock.set(now);
    let report = qh.pump(now).expect("pump");
    assert!(
        report.lane1.iter().any(|trace| matches!(
            &trace.call,
            Lane1Call::IngestMove {
                frame: 9,
                result: MoveAdmission::Admitted { seq: 9 },
                len,
                ..
            } if *len == expected[8].len()
        )),
        "{:?}",
        report.lane1
    );
    assert_eq!(report.lane1_held, 0);

    // One more decision window releases the ninth receipt. The take waits for
    // an empty lane-1 queue (Reading 12), so pump until the transport has
    // handed the earlier eight to QUIC and the client holds all nine.
    let now = CLOCK_START + 60;
    assert!(qh.advance(now).expect("advance").ticked);
    clock.set(now);
    let start = Instant::now();
    loop {
        let _ = qh.pump(now).expect("pump");
        client.read_available();
        if client.receipts.len() >= 9 {
            break;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: nine receipts");
        qh.wait(Duration::from_millis(5));
    }
    let seqs: Vec<u64> = client
        .receipts
        .iter()
        .map(|bytes| receipt_of(bytes).seq)
        .collect();
    assert_eq!(seqs, (1..=9).collect::<Vec<_>>());
}

/// Q5's held state, then `frames` sent behind the hold and settled over three
/// pumps: the adapter retries the held frame first and reads nothing fresh, so
/// every `frames` entry is in the transport's lane-1 queue when the hold is
/// released. Returns the nine held frames' bytes.
fn held_with_queued(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    client: &mut MvClient,
    frames: &[Vec<u8>],
) -> Vec<Vec<u8>> {
    let expected = q5_held_state(qh, clock, client);
    for bytes in frames {
        client.send_lane1(bytes.clone());
    }
    settle_lane1(qh, client);
    let now = clock.now();
    for step in 1..=3 {
        let now = now + step * 10;
        clock.set(now);
        let report = qh.pump(now).expect("pump");
        let ingests = report
            .lane1
            .iter()
            .filter(|trace| matches!(trace.call, Lane1Call::IngestMove { .. }))
            .count();
        assert_eq!(
            ingests, 1,
            "only the held retry is read while the hold persists: {report:?}"
        );
        assert_eq!(report.lane1_held, 1, "{report:?}");
        qh.wait(Duration::from_millis(5));
    }
    expected
}

/// Releases Q5's hold: one pump after the decision window (the held frame
/// still refuses and the released receipts are taken), then the pump that
/// admits the held frame and reads every frame queued behind it.
fn release_held(qh: &mut QuicHost, clock: &mut TestClock) -> QuicPumpReport {
    let now = clock.now() + 10;
    assert!(qh.advance(now).expect("advance").ticked);
    clock.set(now);
    let report = qh.pump(now).expect("pump");
    assert_eq!(
        report
            .lane1
            .iter()
            .filter(|trace| matches!(trace.call, Lane1Call::IngestMove { .. }))
            .count(),
        1,
        "the held frame still does not fit in the drain: {report:?}"
    );
    let now = clock.now() + 10;
    clock.set(now);
    qh.pump(now).expect("pump")
}

/// The lane-1 `IngestMove` traces of one pump, in order.
fn ingests_of(report: &QuicPumpReport) -> Vec<Lane1Trace> {
    report
        .lane1
        .iter()
        .filter(|trace| matches!(trace.call, Lane1Call::IngestMove { .. }))
        .cloned()
        .collect()
}

/// Q5 (C-3 (1)--(2), Reading 14): nothing behind a held frame is read, and
/// once the hold clears the next frame is drained in the same pump.
#[test]
fn q5_nothing_after_the_held_frame() {
    let (mut qh, mut clock, _) = movement_adapter(true);
    let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
    let frame10 = halt_bytes(e(1), 10, 1);
    let expected = held_with_queued(
        &mut qh,
        &mut clock,
        &mut client,
        std::slice::from_ref(&frame10),
    );

    // With the hold still in place, each pump retries frame 9 only.
    let now = clock.now() + 10;
    clock.set(now);
    let report = qh.pump(now).expect("pump");
    let traces = ingests_of(&report);
    assert_eq!(traces.len(), 1, "{:?}", report.lane1);
    assert_eq!(
        traces[0].call,
        Lane1Call::IngestMove {
            conn: client.conn,
            peer: client.peer,
            frame: 9,
            len: expected[8].len(),
            result: MoveAdmission::Refused {
                seq: Some(9),
                code: CodeV3::QueueFull,
            },
        }
    );
    assert_eq!(report.lane1_held, 1);
    assert!(
        !report
            .lane1
            .iter()
            .any(|trace| matches!(&trace.call, Lane1Call::IngestMove { frame: 10, .. })),
        "frame 10 overtook the held frame: {:?}",
        report.lane1
    );

    // Released: frame 9 is admitted, then frame 10 in the same pump.
    let report = release_held(&mut qh, &mut clock);
    let traces = ingests_of(&report);
    assert_eq!(traces.len(), 2, "{:?}", report.lane1);
    assert_eq!(
        traces[0].call,
        Lane1Call::IngestMove {
            conn: client.conn,
            peer: client.peer,
            frame: 9,
            len: expected[8].len(),
            result: MoveAdmission::Admitted { seq: 9 },
        }
    );
    assert_eq!(
        traces[1].call,
        Lane1Call::IngestMove {
            conn: client.conn,
            peer: client.peer,
            frame: 10,
            len: frame10.len(),
            result: MoveAdmission::Admitted { seq: 10 },
        }
    );
    assert_eq!(report.lane1_held, 0);
}

/// Q5 (C-3 (2)): the fresh frame that is held stops its connection's drain in
/// that same pump -- a later frame queued behind it is never read.
#[test]
fn q5_held_frame_stops_the_drain() {
    let (mut qh, mut clock, _) = movement_adapter(true);
    let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
    let expected: Vec<Vec<u8>> = (1..=10).map(|seq| halt_bytes(e(1), seq, 1)).collect();
    for bytes in &expected {
        client.send_lane1(bytes.clone());
    }
    settle_lane1(&qh, &mut client);
    let now = CLOCK_START + 10;
    clock.set(now);
    let report = qh.pump(now).expect("pump");
    let traces = ingests_of(&report);
    assert_eq!(
        traces.len(),
        9,
        "frame 10 is read after the held frame: {:?}",
        report.lane1
    );
    assert_eq!(
        traces[8].call,
        Lane1Call::IngestMove {
            conn: client.conn,
            peer: client.peer,
            frame: 9,
            len: expected[8].len(),
            result: MoveAdmission::Refused {
                seq: Some(9),
                code: CodeV3::QueueFull,
            },
        }
    );
    assert_eq!(report.lane1_held, 1);
}

/// Q5 (C-3 `MAX_DRAIN_PER_CONNECTION`): a pump drains every frame one
/// connection's transport can hold, and not one connection's bound in total.
/// The transport's lane-1 inbound queue holds 64 frames per connection, so
/// "more than the bound queued" is three movement connections.
#[test]
fn q5_drain_bound_is_per_connection() {
    let (mut qh, mut clock, invitation) = movement_adapter(true);
    let _ = invitation.expect("INV registered");
    let credentials = [
        credential(&[0x51; 32]),
        credential(&[0x61; 32]),
        credential(&[0x62; 32]),
    ];
    for extra in &credentials[1..] {
        let mut invitation = inv();
        invitation.credential = extra.clone();
        qh.register_v4_invitation(invitation).expect("registers");
    }
    let mut clients = Vec::new();
    for bytes in &credentials {
        let (client, _, _) = connect_movement(&mut qh, &mut clock, bytes);
        clients.push(client);
    }

    const PER_CONNECTION: usize = 45;
    assert!(
        credentials.len() * PER_CONNECTION > MAX_DRAIN_PER_CONNECTION,
        "the case must queue more than one connection's bound"
    );
    let stale = encode_intent(&MoveIntent {
        epoch: [9; 16],
        seq: 1,
        actor: n(1),
        body: MoveBody::Halt,
    })
    .expect("encodes");
    for client in &mut clients {
        for _ in 0..PER_CONNECTION {
            client.send_lane1(stale.clone());
        }
    }
    for client in &mut clients {
        settle_lane1(&qh, client);
    }

    let now = CLOCK_START + 10;
    clock.set(now);
    let mut total = 0usize;
    let mut refused = 0usize;
    let mut per_connection: BTreeMap<ConnectionId, usize> = BTreeMap::new();
    let start = Instant::now();
    let wanted = credentials.len() * PER_CONNECTION;
    while total < wanted {
        let report = qh.pump(now).expect("pump");
        let mut counts: BTreeMap<ConnectionId, usize> = BTreeMap::new();
        for trace in &report.lane1 {
            if let Lane1Call::IngestMove {
                conn, len, result, ..
            } = &trace.call
            {
                assert_eq!(*len, stale.len());
                assert!(matches!(
                    result,
                    MoveAdmission::Refused {
                        code: CodeV3::SessionExpired,
                        ..
                    }
                ));
                *counts.entry(*conn).or_default() += 1;
            }
        }
        total += counts.values().sum::<usize>();
        refused += report.lane1_refused;
        assert_eq!(report.lane1_held, 0);
        for (conn, count) in counts {
            let best = per_connection.entry(conn).or_default();
            *best = (*best).max(count);
        }
        assert!(start.elapsed() < DEADLINE, "deadline: the lane-1 drain");
        qh.wait(Duration::from_millis(5));
    }
    assert_eq!(total, wanted);
    assert_eq!(refused, wanted);
    // C-3's bound is per connection and per pump: each connection's queue of
    // `PER_CONNECTION` frames leaves in one pump, never one frame per pump.
    assert_eq!(per_connection.len(), credentials.len());
    for (conn, most) in &per_connection {
        assert_eq!(
            *most, PER_CONNECTION,
            "{conn:?} did not drain its queue in one pump: {per_connection:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Q6: terminal refusals are dropped or fenced, never held.
// ---------------------------------------------------------------------------

#[test]
fn q6_refusals_dropped_or_fenced() {
    // (a) A stale epoch is refused and dropped; no sequence is consumed and
    // nothing is fenced.
    {
        let (mut qh, mut clock, _) = movement_adapter(true);
        let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
        let stale = encode_intent(&MoveIntent {
            epoch: [9; 16],
            seq: 1,
            actor: n(1),
            body: MoveBody::Halt,
        })
        .expect("encodes");
        client.send_lane1(stale.clone());
        let now = CLOCK_START + 10;
        let (reports, traces) = pump_ingests(&mut qh, &mut clock, now, 1);
        assert_eq!(traces.len(), 1);
        assert_eq!(
            traces[0].call,
            Lane1Call::IngestMove {
                conn: client.conn,
                peer: client.peer,
                frame: 1,
                len: stale.len(),
                result: MoveAdmission::Refused {
                    seq: Some(1),
                    code: CodeV3::SessionExpired,
                },
            }
        );
        let report = reports.last().expect("a pump");
        assert_eq!(report.lane1_refused, 1);
        assert!(report.fences.is_empty(), "{:?}", report.fences);
        assert_eq!(report.lane1_held, 0);

        client.send_lane1(halt_bytes(e(1), 1, 1));
        let (_, traces) = pump_ingests(&mut qh, &mut clock, now + 10, 1);
        assert_eq!(
            traces.last().expect("the control").call,
            Lane1Call::IngestMove {
                conn: client.conn,
                peer: client.peer,
                frame: 2,
                len: halt_bytes(e(1), 1, 1).len(),
                result: MoveAdmission::Admitted { seq: 1 },
            }
        );
    }
    // (b) A same-seq different-bytes retry is a `SeqConflict`, and the same
    // pump's reconcile fences the binding.
    {
        let (mut qh, mut clock, _) = movement_adapter(true);
        let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
        client.send_lane1(halt_bytes(e(1), 1, 1));
        let now = CLOCK_START + 10;
        let _ = pump_ingests(&mut qh, &mut clock, now, 1);
        let conflict = move_to_bytes(e(1), 1, 1, p(262_144, 0, 65_536));
        client.send_lane1(conflict.clone());
        let now = now + 10;
        let (reports, traces) = pump_ingests(&mut qh, &mut clock, now, 1);
        assert_eq!(
            traces.last().expect("the conflict").call,
            Lane1Call::IngestMove {
                conn: client.conn,
                peer: client.peer,
                frame: 2,
                len: conflict.len(),
                result: MoveAdmission::Refused {
                    seq: Some(1),
                    code: CodeV3::SeqConflict,
                },
            }
        );
        let report = reports.last().expect("a pump");
        assert_eq!(report.lane1_refused, 1);
        assert_eq!(
            report.fences,
            vec![Fence {
                conn: client.conn,
                peer: client.peer,
                cause: FenceCause::EpochClosed,
            }]
        );
        assert_eq!(
            client_close_reason(&mut client),
            CloseReason::Peer { code: 9 }
        );
    }
    // (c) A held lane-1 frame is discarded when the session is fenced.
    {
        let (mut qh, mut clock, _) = movement_adapter(true);
        let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
        q5_held_state(&mut qh, &mut clock, &mut client);
        client.send_lane0(end_turn_bytes(e(1)));
        client.send_lane0(declare_strike_bytes(e(1)));
        let steps = pump_until(
            &mut qh,
            &mut clock,
            10,
            "the lane-0 conflict fences",
            |_, report| !report.fences.is_empty(),
        );
        let report = &steps.last().expect("a pump").1;
        assert_eq!(
            report.fences,
            vec![Fence {
                conn: client.conn,
                peer: client.peer,
                cause: FenceCause::EpochClosed,
            }]
        );
        assert!(report.lane1_discarded >= 1, "{report:?}");
        assert_eq!(qh.binding(client.conn), None);
    }
}

/// Q6 (C-3): a stale-epoch refusal is dropped and the drain continues, so a
/// valid frame queued behind it is ingested in the same pump.
#[test]
fn q6_stale_then_valid_same_pump() {
    let (mut qh, mut clock, _) = movement_adapter(true);
    let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
    let stale = encode_intent(&MoveIntent {
        epoch: [9; 16],
        seq: 10,
        actor: n(1),
        body: MoveBody::Halt,
    })
    .expect("encodes");
    let valid = halt_bytes(e(1), 10, 1);
    let _ = held_with_queued(
        &mut qh,
        &mut clock,
        &mut client,
        &[stale.clone(), valid.clone()],
    );
    let report = release_held(&mut qh, &mut clock);
    let traces = ingests_of(&report);
    assert_eq!(traces.len(), 3, "{:?}", report.lane1);
    assert_eq!(
        traces[1].call,
        Lane1Call::IngestMove {
            conn: client.conn,
            peer: client.peer,
            frame: 10,
            len: stale.len(),
            result: MoveAdmission::Refused {
                seq: Some(10),
                code: CodeV3::SessionExpired,
            },
        }
    );
    assert_eq!(
        traces[2].call,
        Lane1Call::IngestMove {
            conn: client.conn,
            peer: client.peer,
            frame: 11,
            len: valid.len(),
            result: MoveAdmission::Admitted { seq: 10 },
        }
    );
    assert_eq!(report.lane1_refused, 1);
    assert!(report.fences.is_empty(), "{:?}", report.fences);
}

/// Q6 (C-3): a duplicate resend is answered and the drain continues.
#[test]
fn q6_duplicate_continues() {
    let (mut qh, mut clock, _) = movement_adapter(true);
    let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
    let duplicate = halt_bytes(e(1), 1, 1);
    let next = halt_bytes(e(1), 10, 1);
    let _ = held_with_queued(
        &mut qh,
        &mut clock,
        &mut client,
        &[duplicate.clone(), next.clone()],
    );
    let report = release_held(&mut qh, &mut clock);
    let traces = ingests_of(&report);
    assert_eq!(traces.len(), 3, "{:?}", report.lane1);
    assert!(
        matches!(
            &traces[1].call,
            Lane1Call::IngestMove {
                frame: 10,
                len,
                result: MoveAdmission::Duplicate { seq: 1, .. },
                ..
            } if *len == duplicate.len()
        ),
        "{:?}",
        traces[1]
    );
    assert_eq!(
        traces[2].call,
        Lane1Call::IngestMove {
            conn: client.conn,
            peer: client.peer,
            frame: 11,
            len: next.len(),
            result: MoveAdmission::Admitted { seq: 10 },
        }
    );
    assert_eq!(report.lane1_refused, 0);
    assert!(report.fences.is_empty(), "{:?}", report.fences);
}

/// Q6 (C-3): a `Rejected` admission is terminal for its frame alone, and the
/// drain continues to the next one.
#[test]
fn q6_rejected_continues() {
    let (mut qh, mut clock, _) = movement_adapter(true);
    let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
    // `INV` controls `n(1)`--`n(3)`: `n(4)` resolves to no actor.
    let rejected = halt_bytes(e(1), 10, 4);
    let next = halt_bytes(e(1), 11, 1);
    let _ = held_with_queued(
        &mut qh,
        &mut clock,
        &mut client,
        &[rejected.clone(), next.clone()],
    );
    let report = release_held(&mut qh, &mut clock);
    let traces = ingests_of(&report);
    assert_eq!(traces.len(), 3, "{:?}", report.lane1);
    assert_eq!(
        traces[1].call,
        Lane1Call::IngestMove {
            conn: client.conn,
            peer: client.peer,
            frame: 10,
            len: rejected.len(),
            result: MoveAdmission::Rejected {
                seq: 10,
                code: CodeV3::NotAuthorized,
            },
        }
    );
    assert_eq!(
        traces[2].call,
        Lane1Call::IngestMove {
            conn: client.conn,
            peer: client.peer,
            frame: 11,
            len: next.len(),
            result: MoveAdmission::Admitted { seq: 11 },
        }
    );
    assert_eq!(report.lane1_refused, 0);
    assert!(report.fences.is_empty(), "{:?}", report.fences);
}

/// Q6 (C-3): after a `SeqConflict` closes the session, the next frame is
/// `Unauthenticated`, both are counted, the drain fences `EpochClosed` and
/// stops -- a third queued frame is never read.
#[test]
fn q6_seq_conflict_then_unauthenticated() {
    let (mut qh, mut clock, _) = movement_adapter(true);
    let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
    // Frame 1's `Halt` is cached, so different bytes at seq 1 conflict.
    let conflict = move_to_bytes(e(1), 1, 1, p(262_144, 0, 65_536));
    let after_first = halt_bytes(e(1), 10, 1);
    let after_second = halt_bytes(e(1), 11, 1);
    let _ = held_with_queued(
        &mut qh,
        &mut clock,
        &mut client,
        &[conflict.clone(), after_first.clone(), after_second],
    );
    let report = release_held(&mut qh, &mut clock);
    let traces = ingests_of(&report);
    assert_eq!(traces.len(), 3, "{:?}", report.lane1);
    assert_eq!(
        traces[1].call,
        Lane1Call::IngestMove {
            conn: client.conn,
            peer: client.peer,
            frame: 10,
            len: conflict.len(),
            result: MoveAdmission::Refused {
                seq: Some(1),
                code: CodeV3::SeqConflict,
            },
        }
    );
    assert!(
        matches!(
            &traces[2].call,
            Lane1Call::IngestMove {
                conn,
                peer,
                frame: 11,
                len,
                result: MoveAdmission::Refused {
                    code: CodeV3::Unauthenticated,
                    ..
                },
            } if *conn == client.conn && *peer == client.peer && *len == after_first.len()
        ),
        "{:?}",
        traces[2]
    );
    assert_eq!(report.lane1_refused, 2);
    assert_eq!(
        report.fences,
        vec![Fence {
            conn: client.conn,
            peer: client.peer,
            cause: FenceCause::EpochClosed,
        }]
    );
}

// ---------------------------------------------------------------------------
// Q7: the two lanes drain independently.
// ---------------------------------------------------------------------------
#[test]
fn q7_lanes_drain_independently() {
    let (mut qh, mut clock, _) = movement_adapter(true);
    let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
    q5_held_state(&mut qh, &mut clock, &mut client);

    client.send_lane0(end_turn_bytes(e(1)));
    await_transport(&qh, "the lane-0 frame is queued");
    let now = CLOCK_START + 20;
    clock.set(now);
    let report = qh.pump(now).expect("pump");
    assert!(
        matches!(
            report.calls.first(),
            Some(HostCall::Ingest {
                conn,
                peer,
                frame: 1,
                result: IngestDisposition::Staged,
                ..
            }) if *conn == client.conn && *peer == client.peer
        ),
        "{:?}",
        report.calls.first()
    );
    assert_eq!(report.lane1[0].at, 1);
    assert!(
        matches!(
            &report.lane1[0].call,
            Lane1Call::IngestMove {
                frame: 9,
                result: MoveAdmission::Refused {
                    code: CodeV3::QueueFull,
                    ..
                },
                ..
            }
        ),
        "{:?}",
        report.lane1[0]
    );
    let summary = match &report.calls[1] {
        HostCall::Pump { summary } => summary,
        other => panic!("expected Pump, got {other:?}"),
    };
    assert!(summary.admitted >= 1, "{summary:?}");
    assert_eq!(report.lane1_held, 1);
}

// ---------------------------------------------------------------------------
// Q8: a departed peer's receipts are discarded, and retirement waits for both
// lanes.
// ---------------------------------------------------------------------------

#[test]
fn q8_departed_peer_and_retirement() {
    // (a) Both frames drained before the unbind, whatever pump sees the close.
    {
        let (mut qh, mut clock, _) = movement_adapter(true);
        let (client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
        let conn = client.conn;
        let peer = client.peer;
        let mut client = client;
        client.send_lane1(halt_bytes(e(1), 1, 1));
        client.send_lane1(halt_bytes(e(1), 2, 1));
        let MvClient { quic, .. } = client;
        let _closed = quic.close(CloseCode::Normal, CloseMode::Flush);
        let steps = pump_until(&mut qh, &mut clock, 100, "the unbind", |_, report| {
            report
                .calls
                .iter()
                .any(|call| matches!(call, HostCall::UnbindPeer { peer: p } if *p == peer))
        });

        // C-6: the unbind lands in the same pump that reports the transport
        // close -- a binding never retires a pump late.
        let close_step = steps
            .iter()
            .position(|(_, report)| {
                report
                    .transport_closes
                    .iter()
                    .any(|(closed, _)| *closed == conn)
            })
            .expect("the transport close is reported");
        let unbind_step = steps
            .iter()
            .position(|(_, report)| {
                report
                    .calls
                    .iter()
                    .any(|call| matches!(call, HostCall::UnbindPeer { peer: p } if *p == peer))
            })
            .expect("the binding unbound");
        assert_eq!(
            unbind_step, close_step,
            "the unbind lands in the pump that reports the close: {steps:#?}"
        );

        // Both lane-1 ingests happened before the unbind, in (step, at) order.
        let mut unbind: Option<(usize, usize)> = None;
        let mut admits: Vec<(usize, usize)> = Vec::new();
        for (step, (_, report)) in steps.iter().enumerate() {
            for trace in &report.lane1 {
                if matches!(
                    &trace.call,
                    Lane1Call::IngestMove {
                        result: MoveAdmission::Admitted { .. },
                        ..
                    }
                ) {
                    admits.push((step, trace.at));
                }
            }
            for (index, call) in report.calls.iter().enumerate() {
                if matches!(call, HostCall::UnbindPeer { peer: p } if *p == peer) {
                    unbind = Some((step, index));
                }
            }
        }
        let unbind = unbind.expect("the binding unbound");
        assert_eq!(admits.len(), 2, "{admits:?}");
        for admit in &admits {
            assert!(*admit < unbind, "an ingest after the unbind: {admit:?}");
        }

        // The unbind's own step: lane 0's delivery, then both lane-1 takes
        // before the unbind call.
        let (_, report) = &steps[unbind.0];
        assert_eq!(report.calls.len(), 3, "{:?}", report.calls);
        assert!(matches!(report.calls[0], HostCall::Pump { .. }));
        assert_eq!(
            report.calls[1],
            HostCall::TakeDelivery {
                peer,
                result: Ok(0),
            }
        );
        assert_eq!(report.calls[2], HostCall::UnbindPeer { peer });
        let takes: Vec<&Lane1Trace> = report
            .lane1
            .iter()
            .filter(|trace| {
                matches!(
                    trace.call,
                    Lane1Call::TakeMoveReceipts { .. } | Lane1Call::TakeSamples { .. }
                )
            })
            .collect();
        assert_eq!(
            takes,
            vec![
                &Lane1Trace {
                    at: 2,
                    call: Lane1Call::TakeMoveReceipts {
                        peer,
                        result: Ok(0),
                    },
                },
                &Lane1Trace {
                    at: 2,
                    call: Lane1Call::TakeSamples {
                        peer,
                        result: Ok(0),
                    },
                },
            ]
        );
    }
    // (b) A released receipt of a departed peer is taken and discarded, never
    // held, and the binding retires.
    {
        let (mut qh, mut clock, _) = movement_adapter(true);
        let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
        let conn = client.conn;
        let peer = client.peer;
        client.send_lane1(move_to_bytes(e(1), 1, 1, p(262_144, 0, 65_536)));
        let now = CLOCK_START + 10;
        let _ = pump_ingests(&mut qh, &mut clock, now, 1);
        let now = CLOCK_START + 20;
        assert!(qh.advance(now).expect("advance").ticked);
        let MvClient { quic, .. } = client;
        let _ = quic.close(CloseCode::Normal, CloseMode::Discard);
        until("the close is published", || {
            qh.wait(Duration::from_millis(100))
        });
        let report = qh.pump(now).expect("one pump");
        assert!(
            report.lane1.iter().any(|trace| trace.call
                == Lane1Call::TakeMoveReceipts {
                    peer,
                    result: Ok(1),
                }),
            "{:?}",
            report.lane1
        );
        assert_eq!(report.lane1_sent, 0);
        // C-5: every discarded receipt and sample is counted, exactly.
        let receipts = report
            .lane1
            .iter()
            .find_map(|trace| match trace.call {
                Lane1Call::TakeMoveReceipts { result: Ok(n), .. } => Some(n),
                _ => None,
            })
            .expect("a receipt take");
        let samples = report
            .lane1
            .iter()
            .find_map(|trace| match trace.call {
                Lane1Call::TakeSamples { result: Ok(n), .. } => Some(n),
                _ => None,
            })
            .expect("a sample take");
        assert!(samples >= 1, "the advance queued a sample: {report:?}");
        assert_eq!(report.lane1_discarded, receipts + samples, "{report:?}");
        assert!(report.calls.contains(&HostCall::UnbindPeer { peer }));
        assert_eq!(qh.binding(conn), None);
        assert_eq!(qh.host().session_wire(peer), Err(HostError::UnknownPeer));
    }
}

/// Q8 (C-6, C-3): a held lane-1 frame blocks retirement until it is retried
/// and admitted; the departed peer's released receipts are discarded.
#[test]
fn q8_held_frame_delays_retirement() {
    let (mut qh, mut clock, _) = movement_adapter(true);
    let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
    let conn = client.conn;
    let peer = client.peer;
    q5_held_state(&mut qh, &mut clock, &mut client);
    let MvClient { quic, .. } = client;
    let _ = quic.close(CloseCode::Normal, CloseMode::Discard);
    until("the close is published", || {
        qh.wait(Duration::from_millis(100))
    });

    // The close is seen, but the hold keeps the binding alive.
    let now = CLOCK_START + 20;
    clock.set(now);
    let report = qh.pump(now).expect("pump");
    assert_eq!(report.lane1_held, 1, "{report:?}");
    assert_eq!(qh.binding(conn), Some(peer));
    assert!(
        !report
            .calls
            .iter()
            .any(|call| matches!(call, HostCall::UnbindPeer { .. })),
        "{:?}",
        report.calls
    );

    // The decision window releases eight receipts; the departed peer's take
    // discards them (C-5), and the held frame still does not fit.
    let now = CLOCK_START + 30;
    assert!(qh.advance(now).expect("advance").ticked);
    clock.set(now);
    let report = qh.pump(now).expect("pump");
    assert_eq!(report.lane1_held, 1, "{report:?}");
    assert_eq!(qh.binding(conn), Some(peer));
    assert!(
        !report
            .calls
            .iter()
            .any(|call| matches!(call, HostCall::UnbindPeer { .. })),
        "{:?}",
        report.calls
    );
    let receipts = report
        .lane1
        .iter()
        .find_map(|trace| match trace.call {
            Lane1Call::TakeMoveReceipts { result: Ok(n), .. } => Some(n),
            _ => None,
        })
        .expect("a receipt take");
    assert_eq!(receipts, 8, "{:?}", report.lane1);
    let samples = report
        .lane1
        .iter()
        .find_map(|trace| match trace.call {
            Lane1Call::TakeSamples { result: Ok(n), .. } => Some(n),
            _ => None,
        })
        .expect("a sample take");
    assert_eq!(report.lane1_sent, 0);
    assert_eq!(report.lane1_discarded, receipts + samples, "{report:?}");

    // With the receipts gone the retry fits; the same pump drains the closed
    // lane 1 and retires the binding.
    let now = CLOCK_START + 40;
    clock.set(now);
    let report = qh.pump(now).expect("pump");
    assert!(
        report.lane1.iter().any(|trace| trace.call
            == Lane1Call::IngestMove {
                conn,
                peer,
                frame: 9,
                len: halt_bytes(e(1), 9, 1).len(),
                result: MoveAdmission::Admitted { seq: 9 },
            }),
        "{:?}",
        report.lane1
    );
    assert_eq!(report.lane1_held, 0);
    assert!(report.calls.contains(&HostCall::UnbindPeer { peer }));
    assert_eq!(qh.binding(conn), None);
}

/// Q8 (C-6): a retiring connection's queued lane-1 frames are drained and
/// counted as discarded.
#[test]
fn q8_retiring_discards_lane1_frames() {
    let (mut qh, mut clock, _) = movement_adapter(true);
    let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
    let conn = client.conn;
    const FRAMES: usize = 3;
    for seq in 1..=FRAMES as u64 {
        client.send_lane1(halt_bytes(e(1), seq, 1));
    }
    settle_lane1(&qh, &mut client);
    let fence = qh.disconnect(conn).expect("disconnects");
    assert_eq!(fence.cause, FenceCause::Disconnected);

    let now = CLOCK_START + 10;
    clock.set(now);
    let report = qh.pump(now).expect("pump");
    assert_eq!(
        report.lane1_discarded, FRAMES,
        "every queued lane-1 frame of a retiring connection is discarded: {report:?}"
    );
    assert!(report.lane1.is_empty(), "{report:?}");
}

// ---------------------------------------------------------------------------
// Q9: shutdown flushes released receipts.
// ---------------------------------------------------------------------------

#[test]
fn q9_shutdown_flushes_receipts() {
    let (mut qh, mut clock, _) = movement_adapter(true);
    let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
    client.send_lane1(halt_bytes(e(1), 1, 1));
    let now = CLOCK_START + 10;
    let _ = pump_ingests(&mut qh, &mut clock, now, 1);
    let now = CLOCK_START + 20;
    assert!(qh.advance(now).expect("advance").ticked);
    let shutdown = qh.shutdown(false);
    assert_eq!(shutdown.undelivered_receipts, 0);

    let start = Instant::now();
    let mut frames = Vec::new();
    let reason = loop {
        match client.quic.try_recv_lane1() {
            Ok(Some(frame)) => frames.push(frame),
            Ok(None) => {}
            Err(reason) => break reason,
        }
        assert!(
            start.elapsed() < DEADLINE,
            "deadline: the receipt and close"
        );
        client.quic.wait(Duration::from_millis(5));
    };
    assert_eq!(frames.len(), 1, "one receipt");
    assert_eq!(receipt_of(&frames[0]).seq, 1);
    assert_eq!(reason, CloseReason::Peer { code: 1 });
}

/// Q9 (C-8): a receipt the transport can no longer take is counted as
/// undelivered, and every receipt behind the first failure counts too.
#[test]
fn q9_undelivered_receipts_counted() {
    // One released receipt behind a discarded close.
    {
        let (mut qh, mut clock, _) = movement_adapter(true);
        let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
        client.send_lane1(halt_bytes(e(1), 1, 1));
        let now = CLOCK_START + 10;
        let _ = pump_ingests(&mut qh, &mut clock, now, 1);
        let now = CLOCK_START + 20;
        assert!(qh.advance(now).expect("advance").ticked);
        let MvClient { quic, .. } = client;
        let _ = quic.close(CloseCode::Normal, CloseMode::Discard);
        until("the close is published", || {
            qh.wait(Duration::from_millis(100))
        });
        assert_eq!(qh.shutdown(false).undelivered_receipts, 1);
    }
    // Several released receipts: every one after the first failure counts.
    {
        const RECEIPTS: usize = 3;
        let (mut qh, mut clock, _) = movement_adapter(true);
        let (mut client, _, _) = connect_movement(&mut qh, &mut clock, &inv().credential);
        for seq in 1..=RECEIPTS as u64 {
            client.send_lane1(halt_bytes(e(1), seq, 1));
        }
        let now = CLOCK_START + 10;
        let _ = pump_ingests(&mut qh, &mut clock, now, RECEIPTS);
        let now = CLOCK_START + 20;
        assert!(qh.advance(now).expect("advance").ticked);
        let MvClient { quic, .. } = client;
        let _ = quic.close(CloseCode::Normal, CloseMode::Discard);
        until("the close is published", || {
            qh.wait(Duration::from_millis(100))
        });
        assert_eq!(qh.shutdown(false).undelivered_receipts, RECEIPTS);
    }
}

// ---------------------------------------------------------------------------
// Q10: `/1` and `/2` sessions are untouched.
// ---------------------------------------------------------------------------

#[test]
fn q10_legacy_sessions_unchanged() {
    let clock = TestClock::new();
    let server = QuicServer::bind_dual(ServerConfig {
        bind: loopback(),
        identity: identity(),
        limits: ServerLimits::v1(),
    })
    .expect("bind dual server");
    let qh = QuicHost::start(new_host(V2), server, clock.now()).expect("starts");
    let mut qh = qh;
    let mut clock = clock;
    qh.register_invitation(Invitation {
        credential: credential(&cred_bytes(0)),
        control: all_actors(),
        grants: full_grants(&[a(), b(), c()]),
    })
    .expect("registers A");

    let addr = qh.local_addr();
    let hello = crpg_net_quic::Hello {
        wire_version: 2,
        credential: credential(&cred_bytes(0)),
    };
    let pending = std::thread::spawn(move || {
        QuicClient::connect_framed(client_config(ClientLimits::v1()), addr, &hello, Framing::V2)
    });
    let (decision, mut steps) = decide_hello(&mut qh, &mut clock, 0);
    let HelloOutcome::Accepted { peer, .. } = decision.outcome else {
        panic!("expected Accepted, got {decision:?}");
    };
    let conn = decision.conn;
    let mut client = pending.join().expect("client thread").expect("connect");
    let epoch = qh.host().epoch(peer).expect("bound");
    assert_eq!(qh.binding(conn), Some(peer));

    client
        .try_send(&end_turn(2, epoch, 1, 0, 1))
        .expect("lane-0 intent");
    let more = pump_until(&mut qh, &mut clock, 100, "the ingest", |_, report| {
        report
            .calls
            .iter()
            .any(|call| matches!(call, HostCall::Ingest { .. }))
    });
    steps.extend(more);
    let _ = client.close(CloseCode::Normal, CloseMode::Flush);
    let more = pump_until(&mut qh, &mut clock, 100, "the unbind", |_, report| {
        report
            .calls
            .iter()
            .any(|call| matches!(call, HostCall::UnbindPeer { peer: p } if *p == peer))
    });
    steps.extend(more);

    for (_, report) in &steps {
        assert!(report.lane1.is_empty(), "{report:?}");
        assert_eq!(report.lane1_sent, 0);
        assert_eq!(report.datagrams_sent, 0);
        assert_eq!(report.lane1_refused, 0);
        assert_eq!(report.lane1_discarded, 0);
        assert_eq!(report.lane1_held, 0);
    }
    // The leading pre-hello `[Pump]` steps vary in count with transport
    // timing, so every step's list must be one of the `/2` route's exact
    // shapes, and the bind, ingest and retirement pumps are pinned exactly.
    for (_, report) in &steps {
        let names: Vec<&str> = report.calls.iter().map(call_name).collect();
        assert!(
            matches!(
                names.as_slice(),
                ["Pump"]
                    | ["Pump", "TakeDelivery"]
                    | [
                        "BindSnapshotPeer",
                        "Pump",
                        "TakeDelivery",
                        "AcknowledgeDelivery"
                    ]
                    | ["Ingest", "Pump", "TakeDelivery", "AcknowledgeDelivery"]
                    | ["Pump", "TakeDelivery", "UnbindPeer"]
            ),
            "a call list beside the `/2` route: {names:?}"
        );
        if !names.contains(&"UnbindPeer") {
            assert!(
                report.transport_closes.is_empty(),
                "only the retirement pump reports the close: {:?}",
                report.transport_closes
            );
        }
    }
    let hello = steps
        .iter()
        .find(|(_, report)| {
            report
                .calls
                .iter()
                .any(|call| matches!(call, HostCall::BindSnapshotPeer { .. }))
        })
        .expect("the snapshot bind");
    assert_eq!(
        hello.1.calls.iter().map(call_name).collect::<Vec<_>>(),
        vec![
            "BindSnapshotPeer",
            "Pump",
            "TakeDelivery",
            "AcknowledgeDelivery"
        ]
    );
    let ingest = steps
        .iter()
        .find(|(_, report)| {
            report
                .calls
                .iter()
                .any(|call| matches!(call, HostCall::Ingest { .. }))
        })
        .expect("the lane-0 ingest");
    assert_eq!(
        ingest.1.calls.iter().map(call_name).collect::<Vec<_>>(),
        vec!["Ingest", "Pump", "TakeDelivery", "AcknowledgeDelivery"]
    );
    // C-6: the close is published in the pump that retires the binding.
    let (_, retire) = steps.last().expect("the retirement pump");
    assert_eq!(
        retire.calls.iter().map(call_name).collect::<Vec<_>>(),
        vec!["Pump", "TakeDelivery", "UnbindPeer"]
    );
    assert_eq!(
        retire.transport_closes,
        vec![(conn, CloseReason::Peer { code: 0 })]
    );
    assert_eq!(qh.binding(conn), None);
    assert_eq!(
        steps
            .iter()
            .filter(|(_, report)| report
                .calls
                .iter()
                .any(|call| matches!(call, HostCall::BindSnapshotPeer { .. })))
            .count(),
        1
    );
    assert_eq!(
        steps
            .iter()
            .filter(|(_, report)| report
                .calls
                .iter()
                .any(|call| matches!(call, HostCall::Ingest { .. })))
            .count(),
        1
    );
    assert_eq!(
        steps
            .iter()
            .filter(|(_, report)| report
                .calls
                .iter()
                .any(|call| matches!(call, HostCall::UnbindPeer { .. })))
            .count(),
        1
    );
}

/// A call's variant name, for sequence assertions.
fn call_name(call: &HostCall) -> &'static str {
    match call {
        HostCall::BindPeer { .. } => "BindPeer",
        HostCall::BindSnapshotPeer { .. } => "BindSnapshotPeer",
        HostCall::UnbindPeer { .. } => "UnbindPeer",
        HostCall::Ingest { .. } => "Ingest",
        HostCall::IngestTransfer { .. } => "IngestTransfer",
        HostCall::Pump { .. } => "Pump",
        HostCall::TakeDelivery { .. } => "TakeDelivery",
        HostCall::AcknowledgeDelivery { .. } => "AcknowledgeDelivery",
        HostCall::IssueResumeTicket { .. } => "IssueResumeTicket",
        HostCall::CheckFrame { .. } => "CheckFrame",
        HostCall::BeginGrace { .. } => "BeginGrace",
        HostCall::ResumePeer { .. } => "ResumePeer",
    }
}

// ---------------------------------------------------------------------------
// Q11: `advance` is a checked pass-through.
// ---------------------------------------------------------------------------

#[test]
fn q11_advance_pass_through() {
    let (mut qh, mut clock, _) = movement_adapter(true);
    let now = CLOCK_START + 50;
    clock.set(now);
    let _ = qh.pump(now).expect("pump");
    let before = qh.host().authority_hash();
    assert_eq!(
        qh.advance(CLOCK_START + 49),
        Err(QuicHostError::TimeRegression)
    );
    assert_eq!(qh.host().authority_hash(), before, "nothing changed");

    let mut twin = movement_host();
    let twin_report = twin.advance(CLOCK_START + 60).expect("twin advance");
    let report = qh.advance(CLOCK_START + 60).expect("advance");
    assert_eq!(report, twin_report);
    assert_eq!(qh.host().authority_hash(), twin.authority_hash());

    // C-9: `advance` records its `now_ms` for the next call's regression
    // check, and an equal time is not a regression.
    assert_eq!(
        qh.pump(CLOCK_START + 55),
        Err(QuicHostError::TimeRegression)
    );
    assert_eq!(
        qh.host().authority_hash(),
        twin.authority_hash(),
        "a refused pump changes nothing"
    );
    assert!(
        qh.advance(CLOCK_START + 60).is_ok(),
        "an equal time is not below the last"
    );
}
