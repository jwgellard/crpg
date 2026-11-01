//! T024h2 acceptance suite (`tasks/T024h.md` S§16): the `crpg-lane0/2`
//! transfer adapter, public API only, over real `crpg-net-quic` loopback.
//!
//! The `/2` client stand-in routes every received frame by its channel into a
//! `crpg_net::reassembly::ReassemblyPool` (`TransferLimits::v1()`, the host's
//! `DeltaWire`, `GateMode::Live`), sends `Published` after every publication,
//! and keeps a test-local replica built only from the pool's updates. Oracles
//! are hand-written expected facts, independent of `PeerView` and the host's
//! own state projection.
//!
//! Every wait is a guard that panics `"deadline"` after 30 s of real time; no
//! case sleeps as an oracle or asserts on wall-clock duration. Injected time
//! comes only from the rig's `TestClock`.

#[path = "support/quic_host_rig.rs"]
mod rig;

use std::collections::BTreeMap;
use std::time::Duration;

use crpg_core::Ulid;
use crpg_net::codec;
use crpg_net::codec_transfer;
use crpg_net::codec_v2;
use crpg_net::protocol::{self, NetId, ReceiptStatus, RejectionCode};
use crpg_net::protocol_transfer::{
    BeginTransfer, CancelReason, HealthFact, ResyncReason, SnapshotBody, SnapshotEntity,
    TransferDown, TransferId, TransferUp, TurnFact,
};
use crpg_net::protocol_v2::{self, DeltaOp};
use crpg_net::reassembly::{
    DeltaWire, GateMode, GatedDelta, ReassemblyPool, ReceiverId, ReplicaUpdate, TransferLimits,
};
use crpg_net_quic::{
    Channel, ClientLimits, CloseCode, CloseMode, CloseReason, ConnectError, ConnectionId, Framing,
    Hello, QuicClient, QuicServer, ServerConfig, ServerLimits,
};
use crpg_server::host::{
    AdmissionResult, ControlGrant, DisclosureGrants, GrantUpdate, PeerHandle, ProtocolSelection,
    PumpSummary, TransferDisposition, TransferState,
};
use crpg_server::quic::{
    FenceCause, GrantChange, HelloOutcome, HostCall, InvitationId, QuicHost, QuicPumpReport,
};
use rig::*;

/// Runs one named case body under both selected versions, as
/// `<case>::v1` and `<case>::v2`.
macro_rules! both_versions {
    ($name:ident) => {
        mod $name {
            #[test]
            fn v1() {
                super::$name(super::V1);
            }
            #[test]
            fn v2() {
                super::$name(super::V2);
            }
        }
    };
}

const APPLIED: ReceiptStatus = ReceiptStatus::Applied;

// ---------------------------------------------------------------------------
// Fixture identities and grants.
// ---------------------------------------------------------------------------

/// Full grants with A's health off (`A_no_health_b`).
fn a_no_health_b(mut grants: DisclosureGrants) -> DisclosureGrants {
    grants.entities[0].health = false;
    grants
}

/// B not present and A's health off (`A_no_health`).
fn a_no_health(mut grants: DisclosureGrants) -> DisclosureGrants {
    grants.entities[0].health = false;
    grants.entities[1].present = false;
    grants
}

// ---------------------------------------------------------------------------
// Literal T024 S§5.4 bodies.
// ---------------------------------------------------------------------------

fn a_no_health_b_body() -> Vec<u8> {
    vec![
        0x03, 0x01, 0x00, 0x02, 0x01, 0x07, 0x0A, 0x00, 0x03, 0x01, 0x03, 0x03, 0x00, 0x01, 0x01,
        0x02, 0x00,
    ]
}

type EntityFacts = (u64, Option<(u32, u32, bool)>);

fn body(entities: &[EntityFacts], turn: Option<(Option<u64>, u64)>) -> SnapshotBody {
    SnapshotBody {
        entities: entities
            .iter()
            .map(|(entity, health)| SnapshotEntity {
                entity: NetId::new(*entity).expect("nonzero"),
                health: health.map(|(health, max_health, dead)| HealthFact {
                    health,
                    max_health,
                    dead,
                }),
            })
            .collect(),
        turn: turn.map(|(active, round)| TurnFact {
            active: active.and_then(NetId::new),
            round,
        }),
    }
}

fn a_no_health_b_snapshot() -> SnapshotBody {
    body(
        &[
            (1, None),
            (2, Some((7, 10, false))),
            (3, Some((3, 3, false))),
        ],
        Some((Some(2), 0)),
    )
}

// ---------------------------------------------------------------------------
// Expected-op constructors and the hand-written V1 projection rule.
// ---------------------------------------------------------------------------

fn legacy(op: protocol::DeltaOp) -> DeltaOp {
    DeltaOp::Legacy(op)
}
fn enter(id: u64) -> DeltaOp {
    legacy(protocol::DeltaOp::EntityEnter {
        entity: NetId::new(id).expect("nonzero"),
    })
}
fn health(id: u64, health: u32, max_health: u32, dead: bool) -> DeltaOp {
    legacy(protocol::DeltaOp::Health {
        entity: NetId::new(id).expect("nonzero"),
        health,
        max_health,
        dead,
    })
}
fn turn(active: Option<u64>, round: u64) -> DeltaOp {
    legacy(protocol::DeltaOp::Turn {
        active: active.and_then(NetId::new),
        round,
    })
}
fn died(id: u64) -> DeltaOp {
    legacy(protocol::DeltaOp::Died {
        entity: NetId::new(id).expect("nonzero"),
    })
}
fn resolved(actor: u64, target: u64, ability: Ulid, outcome: &str, damage: u32) -> DeltaOp {
    DeltaOp::ActionResolved {
        actor: NetId::new(actor).expect("nonzero"),
        target: NetId::new(target).expect("nonzero"),
        ability,
        outcome: outcome.to_owned(),
        damage,
    }
}
fn started(actor: u64, round: u64) -> DeltaOp {
    DeltaOp::TurnStarted {
        actor: NetId::new(actor).expect("nonzero"),
        round,
    }
}

fn for_version(protocol: ProtocolSelection, ops: Vec<DeltaOp>) -> Vec<DeltaOp> {
    match protocol {
        ProtocolSelection::V2 => ops,
        ProtocolSelection::V1 => ops
            .into_iter()
            .filter(|op| {
                matches!(
                    op,
                    DeltaOp::Legacy(
                        protocol::DeltaOp::Receipt { .. }
                            | protocol::DeltaOp::Health { .. }
                            | protocol::DeltaOp::Turn { .. }
                            | protocol::DeltaOp::EntityEnter { .. }
                            | protocol::DeltaOp::EntityLeave { .. }
                    )
                )
            })
            .collect(),
    }
}

fn initial_full_state() -> Vec<DeltaOp> {
    vec![
        enter(1),
        enter(2),
        enter(3),
        health(1, 10, 10, false),
        health(2, 10, 10, false),
        health(3, 3, 3, false),
        turn(Some(1), 0),
    ]
}

// ---------------------------------------------------------------------------
// Decoded hosts.
// ---------------------------------------------------------------------------

fn decode_frame(protocol: ProtocolSelection, bytes: &[u8]) -> protocol_v2::DeltaFrame {
    match protocol {
        ProtocolSelection::V1 => {
            let frame = codec::decode_delta(bytes).expect("v1 delivery decodes");
            protocol_v2::DeltaFrame {
                lane: frame.lane,
                server_tick: frame.server_tick,
                event_seq: frame.event_seq,
                ops: frame.ops.into_iter().map(DeltaOp::Legacy).collect(),
            }
        }
        ProtocolSelection::V2 => codec_v2::decode_delta(bytes).expect("v2 delivery decodes"),
    }
}

fn decode_transfer(bytes: &[u8]) -> TransferDown {
    codec_transfer::decode_down(bytes).expect("transfer frame decodes")
}

fn decode_begin(bytes: &[u8]) -> BeginTransfer {
    match decode_transfer(bytes) {
        TransferDown::Begin(begin) => begin,
        other => panic!("expected Begin, got {other:?}"),
    }
}

fn decode_chunk(bytes: &[u8]) -> Vec<u8> {
    match decode_transfer(bytes) {
        TransferDown::Chunk(chunk) => chunk.bytes,
        other => panic!("expected Chunk, got {other:?}"),
    }
}

fn decode_cancel(bytes: &[u8]) -> (TransferId, CancelReason) {
    match decode_transfer(bytes) {
        TransferDown::Cancel {
            transfer, reason, ..
        } => (transfer, reason),
        other => panic!("expected Cancel, got {other:?}"),
    }
}

fn transfer_id(raw: u64) -> TransferId {
    TransferId::new(raw).expect("nonzero")
}

fn published(epoch: [u8; 16], transfer: u64) -> Vec<u8> {
    codec_transfer::encode_up(&TransferUp::Published {
        epoch,
        transfer: transfer_id(transfer),
    })
    .expect("encodes")
}

fn resync(epoch: [u8; 16], after: Option<u64>, reason: ResyncReason) -> Vec<u8> {
    codec_transfer::encode_up(&TransferUp::Resync {
        epoch,
        after: after.and_then(TransferId::new),
        reason,
    })
    .expect("encodes")
}

// ---------------------------------------------------------------------------
// Replica.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Replica {
    health: BTreeMap<u64, Option<(u32, u32, bool)>>,
    turn: Option<(Option<u64>, u64)>,
}

fn expected(items: &[(u64, (u32, u32, bool))], turn: Option<(Option<u64>, u64)>) -> Replica {
    Replica {
        health: items
            .iter()
            .map(|(id, facts)| (*id, Some(*facts)))
            .collect(),
        turn,
    }
}

/// A's presence is disclosed but its health is not: the replica holds `None`.
fn expected_hidden_a(
    items: &[(u64, (u32, u32, bool))],
    turn: Option<(Option<u64>, u64)>,
) -> Replica {
    let mut replica = expected(items, turn);
    replica.health.insert(1, None);
    replica
}

// ---------------------------------------------------------------------------
// Loopback builders and clients.
// ---------------------------------------------------------------------------

/// A `bind_dual` server: `/2` preferred, `/1` accepted.
fn bind_dual_server(limits: ServerLimits) -> QuicServer {
    QuicServer::bind_dual(ServerConfig {
        bind: loopback(),
        identity: identity(),
        limits,
    })
    .expect("bind loopback dual server")
}

/// A started adapter over the fixture host and a `bind_dual` server.
fn start_dual(protocol: ProtocolSelection) -> (QuicHost, TestClock) {
    let clock = TestClock::new();
    let qh = QuicHost::start(
        new_host(protocol),
        bind_dual_server(ServerLimits::v1()),
        clock.now(),
    )
    .expect("starts over a fresh host");
    (qh, clock)
}

fn connect_framed_async(
    limits: ClientLimits,
    server: std::net::SocketAddr,
    version: u8,
    credential_bytes: &[u8],
    framing: Framing,
) -> std::thread::JoinHandle<Result<QuicClient, ConnectError>> {
    let config = client_config(limits);
    let hello = Hello {
        wire_version: version,
        credential: credential(credential_bytes),
    };
    std::thread::spawn(move || QuicClient::connect_framed(config, server, &hello, framing))
}

/// Connects a real client with an explicit framing and returns the rig's
/// recording client plus the pumps that decided its hello.
fn connect_raw(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    protocol: ProtocolSelection,
    credential_bytes: &[u8],
    framing: Framing,
) -> (Client, Vec<Step>) {
    let pending = connect_framed_async(
        ClientLimits::v1(),
        qh.local_addr(),
        version_byte(protocol),
        credential_bytes,
        framing,
    );
    let (decision, steps) = decide_hello(qh, clock, 100);
    let HelloOutcome::Accepted { invitation, peer } = decision.outcome else {
        panic!("expected Accepted, got {decision:?}");
    };
    let quic = pending.join().expect("client thread").expect("connect");
    assert_eq!(quic.framing(), framing);
    let epoch = qh.host().epoch(peer).expect("bound");
    let client = Client {
        quic,
        conn: decision.conn,
        peer,
        invitation,
        epoch,
        protocol,
        version: version_byte(protocol),
        sent: Vec::new(),
        frames: Vec::new(),
        receipts: Vec::new(),
        closed: None,
    };
    (client, steps)
}

/// A `/2` client plus the pool/replica stand-in.
struct TxClient {
    client: Client,
    recv: Vec<(Channel, Vec<u8>)>,
    pool: ReassemblyPool,
    rx: ReceiverId,
    replica: Replica,
    last_delta_seq: u64,
    published_cursor: u64,
    /// Feed received frames to the pool (off for raw reads).
    feed: bool,
    /// Record but do not offer the next `Chunk` (case 4's withheld chunk).
    withhold_chunk: bool,
}

impl TxClient {
    fn new(client: Client, protocol: ProtocolSelection) -> Self {
        let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("v1 limits");
        let wire = match protocol {
            ProtocolSelection::V1 => DeltaWire::V1,
            ProtocolSelection::V2 => DeltaWire::V2,
        };
        let rx = pool
            .add_receiver(client.epoch, wire, GateMode::Live)
            .expect("receiver");
        Self {
            client,
            recv: Vec::new(),
            pool,
            rx,
            replica: Replica::default(),
            last_delta_seq: 0,
            published_cursor: 0,
            feed: true,
            withhold_chunk: false,
        }
    }

    fn conn(&self) -> ConnectionId {
        self.client.conn
    }

    fn peer(&self) -> PeerHandle {
        self.client.peer
    }

    fn invitation(&self) -> InvitationId {
        self.client.invitation
    }

    fn epoch(&self) -> [u8; 16] {
        self.client.epoch
    }

    fn send_transfer(&mut self, bytes: Vec<u8>) {
        self.client
            .quic
            .try_send_channel(Channel::Transfer, &bytes)
            .expect("client transfer send");
        self.client.sent.push(bytes);
    }

    /// Reads every available frame, routing it by channel. Updates the
    /// replica and answers every publication with `Published`.
    fn read_available(&mut self, now: u64) -> usize {
        let mut count = 0;
        loop {
            match self.client.quic.try_recv_channel() {
                Ok(Some((channel, bytes))) => {
                    self.recv.push((channel, bytes.clone()));
                    if self.feed {
                        match channel {
                            Channel::Lane0 => {
                                let outcome = self
                                    .pool
                                    .offer_delta(self.rx, &bytes, now)
                                    .expect("pool accepts a lane-0 frame");
                                let _ = outcome;
                            }
                            Channel::Transfer => {
                                let withhold = self.withhold_chunk
                                    && matches!(decode_transfer(&bytes), TransferDown::Chunk(_));
                                if withhold {
                                    self.withhold_chunk = false;
                                } else {
                                    self.pool
                                        .offer_transfer(self.rx, &bytes, now)
                                        .expect("pool accepts a transfer frame");
                                }
                            }
                            Channel::Resume => panic!("no resume frame is ever delivered"),
                        }
                        self.drain_updates();
                    }
                    count += 1;
                }
                Ok(None) => break,
                Err(reason) => {
                    self.client.closed = Some(reason);
                    break;
                }
            }
        }
        count
    }

    fn drain_updates(&mut self) {
        while let Some(update) = self.pool.pop_update(self.rx) {
            self.apply(&update);
            if let ReplicaUpdate::Snapshot(publication) = &update {
                let bytes = published(self.client.epoch, publication.transfer.get());
                self.client
                    .quic
                    .try_send_channel(Channel::Transfer, &bytes)
                    .expect("client publishes");
                self.client.sent.push(bytes);
            }
        }
    }

    fn apply(&mut self, update: &ReplicaUpdate) {
        match update {
            ReplicaUpdate::Snapshot(publication) => {
                self.replica.health.clear();
                self.replica.turn = publication
                    .body
                    .turn
                    .as_ref()
                    .map(|turn| (turn.active.map(NetId::get), turn.round));
                for entity in &publication.body.entities {
                    self.replica.health.insert(
                        entity.entity.get(),
                        entity
                            .health
                            .as_ref()
                            .map(|fact| (fact.health, fact.max_health, fact.dead)),
                    );
                }
                self.published_cursor = self.published_cursor.max(publication.cursor);
            }
            ReplicaUpdate::Delta(gated) => {
                let (seq, ops) = match gated {
                    GatedDelta::V1(frame) => (
                        frame.event_seq,
                        frame
                            .ops
                            .iter()
                            .cloned()
                            .map(DeltaOp::Legacy)
                            .collect::<Vec<_>>(),
                    ),
                    GatedDelta::V2(frame) => (frame.event_seq, frame.ops.clone()),
                };
                assert!(
                    seq > self.last_delta_seq,
                    "popped deltas have strictly increasing event_seq: {seq} after {}",
                    self.last_delta_seq
                );
                assert!(
                    seq > self.published_cursor,
                    "popped deltas are above the published cursor: {seq} vs {}",
                    self.published_cursor
                );
                self.last_delta_seq = seq;
                for op in &ops {
                    match op {
                        DeltaOp::Legacy(protocol::DeltaOp::EntityEnter { entity }) => {
                            self.replica.health.insert(entity.get(), None);
                        }
                        DeltaOp::Legacy(protocol::DeltaOp::Health {
                            entity,
                            health,
                            max_health,
                            dead,
                        }) => {
                            self.replica
                                .health
                                .insert(entity.get(), Some((*health, *max_health, *dead)));
                        }
                        DeltaOp::Legacy(protocol::DeltaOp::Turn { active, round }) => {
                            self.replica.turn = Some((active.map(NetId::get), *round));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Guard: reads until `cond` holds.
    fn read_until(&mut self, now: u64, what: &str, mut cond: impl FnMut(&TxClient) -> bool) {
        let start = std::time::Instant::now();
        loop {
            self.read_available(now);
            if cond(self) {
                return;
            }
            assert!(start.elapsed() < DEADLINE, "deadline: {what}");
            self.client.quic.wait(Duration::from_millis(5));
        }
    }

    /// All received payload bytes in arrival order.
    fn received_bytes(&self) -> Vec<Vec<u8>> {
        self.recv.iter().map(|(_, bytes)| bytes.clone()).collect()
    }
}

fn register(
    qh: &mut QuicHost,
    bytes: &[u8],
    control: ControlGrant,
    grants: DisclosureGrants,
) -> InvitationId {
    qh.register_invitation(invitation(bytes, control, grants))
        .expect("registers")
}

fn pump(qh: &mut QuicHost, clock: &mut TestClock) -> QuicPumpReport {
    clock.advance(100);
    qh.pump(clock.now()).expect("pump")
}

fn summary(report: &QuicPumpReport) -> PumpSummary {
    report
        .calls
        .iter()
        .find_map(|call| match call {
            HostCall::Pump { summary } => Some(summary.clone()),
            _ => None,
        })
        .expect("every pump report traces one host pump")
}

fn take_count(report: &QuicPumpReport, peer: PeerHandle) -> Option<usize> {
    report.calls.iter().find_map(|call| match call {
        HostCall::TakeDelivery {
            peer: p,
            result: Ok(n),
        } if *p == peer => Some(*n),
        _ => None,
    })
}

/// The highest acknowledgement traced for `peer`.
fn highest_ack(steps: &[Step], peer: PeerHandle) -> u64 {
    steps
        .iter()
        .flat_map(|(_, report)| report.calls.iter())
        .filter_map(|call| match call {
            HostCall::AcknowledgeDelivery { peer: p, through } if *p == peer => Some(*through),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

fn close_code(reason: CloseReason) -> Option<u64> {
    match reason {
        CloseReason::Peer { code } => Some(code),
        CloseReason::Local(code) => Some(u64::from(code.as_u32())),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// S§16 case 1.
// ---------------------------------------------------------------------------

fn v2_hello_binds_a_snapshot_session(protocol: ProtocolSelection) {
    let (mut qh, mut clock) = start_dual(protocol);
    let id = register(&mut qh, &cred_bytes(0), control(&[]), full_grants());
    let (mut watcher, steps) =
        connect_raw(&mut qh, &mut clock, protocol, &cred_bytes(0), Framing::V2);
    let deciding = &steps.last().expect("deciding pump").1;
    assert_eq!(
        deciding.calls,
        vec![
            HostCall::BindSnapshotPeer {
                conn: watcher.conn,
                invitation: id,
                result: Ok(watcher.peer),
            },
            HostCall::Pump {
                summary: empty_summary(),
            },
            HostCall::TakeDelivery {
                peer: watcher.peer,
                result: Ok(1),
            },
            HostCall::AcknowledgeDelivery {
                peer: watcher.peer,
                through: 1,
            },
        ]
    );
    assert_eq!(
        qh.host().transfer_state(watcher.peer),
        Ok(Some(TransferState {
            next_transfer: 1,
            in_flight: None,
        }))
    );
    watcher.read_until("state delta", |c| !c.frames.is_empty());
    assert_eq!(watcher.frames[0].event_seq, 1);
    assert_eq!(watcher.frames[0].ops, initial_full_state());

    // Control: a `/1` client on an equivalent dual server is a lane-0 session.
    let (mut lane_qh, mut lane_clock) = start_dual(protocol);
    register(&mut lane_qh, &cred_bytes(1), control(&[]), full_grants());
    let (lane, lane_steps) = connect_raw(
        &mut lane_qh,
        &mut lane_clock,
        protocol,
        &cred_bytes(1),
        Framing::V1,
    );
    let deciding = &lane_steps.last().expect("deciding pump").1;
    assert_eq!(
        deciding.calls,
        vec![
            HostCall::BindPeer {
                conn: lane.conn,
                invitation: lane.invitation,
                result: Ok(lane.peer),
            },
            HostCall::Pump {
                summary: empty_summary(),
            },
            HostCall::TakeDelivery {
                peer: lane.peer,
                result: Ok(1),
            },
            HostCall::AcknowledgeDelivery {
                peer: lane.peer,
                through: 1,
            },
        ]
    );
    assert_eq!(lane_qh.host().transfer_state(lane.peer), Ok(None));
}
both_versions!(v2_hello_binds_a_snapshot_session);

// ---------------------------------------------------------------------------
// S§16 case 2.
// ---------------------------------------------------------------------------

/// The decision `set_invitation_grants` made, for the replay mirror.
#[derive(Debug, Clone)]
struct Mirror {
    /// Applied after this many pump steps.
    after: usize,
    invitation: InvitationId,
    grants: DisclosureGrants,
    result: GrantUpdate,
}

fn empty_summary() -> PumpSummary {
    PumpSummary {
        admitted: 0,
        applied: 0,
        rejected: 0,
        backpressured: 0,
        dropped_replies: 0,
        results: Vec::new(),
    }
}

fn result(peer: PeerHandle, seq: Option<u64>, status: ReceiptStatus) -> AdmissionResult {
    AdmissionResult {
        peer,
        seq,
        status,
        cached: false,
        retained_in_ingress: false,
    }
}

/// Sends one lane-0 frame and pumps until the adapter ingested it by ordinal.
fn exchange(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    client: &mut Client,
    bytes: Vec<u8>,
) -> QuicPumpReport {
    let conn = client.conn;
    client.send(bytes);
    let ordinal = client.sent.len() as u64;
    let steps = pump_until(qh, clock, 100, "frame ingested", |_, report| {
        report.calls.iter().any(|call| {
            matches!(call, HostCall::Ingest { conn: c, frame, .. } if *c == conn && *frame == ordinal)
        })
    });
    let report = steps.last().expect("one pump").1.clone();
    assert!(report.fences.is_empty(), "{report:?}");
    report
}

/// Reads a `/2` client until it has at least `n` received frames.
fn read_n(tx: &mut TxClient, now: u64, n: usize, what: &str) {
    tx.read_until(now, what, |t| t.recv.len() >= n);
}

fn v2_narrowing_is_seamless(protocol: ProtocolSelection) {
    let (mut qh, mut clock) = start_dual(protocol);
    let watch_id = register(&mut qh, &cred_bytes(0), control(&[]), full_grants());
    register(&mut qh, &cred_bytes(1), all_actors(), full_grants());
    let (watcher, _) = connect_raw(&mut qh, &mut clock, protocol, &cred_bytes(0), Framing::V2);
    let (mut actor, _) = connect_raw(&mut qh, &mut clock, protocol, &cred_bytes(1), Framing::V1);
    let mut watcher = TxClient::new(watcher, protocol);
    read_n(&mut watcher, clock.now(), 1, "watcher state");
    actor.read_until("actor state", |c| !c.frames.is_empty());

    let seq1 = declare(actor.version, actor.epoch, 1, 0, 1, strike(), 2);
    let report = exchange(&mut qh, &mut clock, &mut actor, seq1);
    assert_eq!(
        summary(&report).results,
        vec![result(actor.peer, Some(1), APPLIED)]
    );
    read_n(&mut watcher, clock.now(), 2, "seq 1 delta");
    let delta = decode_frame(protocol, &watcher.recv[1].1);
    assert_eq!(delta.event_seq, 2);
    let cursor = watcher.last_delta_seq;
    assert_eq!(cursor, 2);

    // Narrowing stages a transfer instead of resetting or fencing.
    let narrowed = a_no_health_b(full_grants());
    assert_eq!(
        qh.set_invitation_grants(watch_id, narrowed),
        Ok(GrantChange::SnapshotStaged)
    );
    let transfer_steps = pump_until(
        &mut qh,
        &mut clock,
        100,
        "snapshot delivered",
        |_, report| report.frames_sent >= 2,
    );
    assert!(transfer_steps
        .iter()
        .all(|(_, report)| report.fences.is_empty()));
    read_n(&mut watcher, clock.now(), 4, "begin and chunk");
    let transfers: Vec<&(Channel, Vec<u8>)> = watcher
        .recv
        .iter()
        .filter(|(channel, _)| *channel == Channel::Transfer)
        .collect();
    assert_eq!(transfers.len(), 2, "{transfers:?}");
    let begin = decode_begin(&transfers[0].1);
    assert_eq!(begin.epoch, watcher.epoch());
    assert_eq!(begin.transfer, transfer_id(1));
    assert_eq!(begin.cursor, cursor);
    assert_eq!(begin.segments_per_chunk, 8);
    assert_eq!(begin.chunk_count, 1);
    assert_eq!(decode_chunk(&transfers[1].1), a_no_health_b_body());
    assert_eq!(
        codec_transfer::decode_body(&decode_chunk(&transfers[1].1)).expect("decodes"),
        a_no_health_b_snapshot()
    );
    assert_eq!(
        qh.host().transfer_state(watcher.peer()),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: Some(1),
        }))
    );

    // The publication is sent and traced.
    let publish_steps = pump_until(&mut qh, &mut clock, 100, "Published traced", |_, report| {
        report.calls.iter().any(|call| {
            matches!(
                call,
                HostCall::IngestTransfer {
                    peer,
                    result: TransferDisposition::Published,
                    ..
                } if *peer == watcher.peer()
            )
        })
    });
    assert!(publish_steps
        .iter()
        .all(|(_, report)| report.fences.is_empty()));
    assert_eq!(
        watcher.replica,
        expected_hidden_a(
            &[(2, (7, 10, false)), (3, (3, 3, false))],
            Some((Some(2), 0)),
        )
    );

    // The actor's seq 2 reaches the watcher above the snapshot cursor.
    let seq2 = declare(actor.version, actor.epoch, 2, 0, 2, strike(), 3);
    let report = exchange(&mut qh, &mut clock, &mut actor, seq2);
    assert_eq!(
        summary(&report).results,
        vec![result(actor.peer, Some(2), APPLIED)]
    );
    read_n(&mut watcher, clock.now(), 5, "seq 2 delta");
    assert_eq!(watcher.recv[4].0, Channel::Lane0);
    let delta = decode_frame(protocol, &watcher.recv[4].1);
    assert_eq!(delta.event_seq, 3);
    // A's health stays undisclosed, so only its health state op is omitted;
    // B and C remain fully visible.
    assert_eq!(
        delta.ops,
        for_version(
            protocol,
            vec![
                resolved(2, 3, strike(), "success", 3),
                died(3),
                started(1, 1),
                health(2, 7, 10, false),
                health(3, 0, 3, true),
                turn(Some(1), 1),
            ],
        )
    );
    assert_eq!(
        watcher.replica,
        expected_hidden_a(
            &[(2, (7, 10, false)), (3, (0, 3, true))],
            Some((Some(1), 1)),
        )
    );
    assert!(watcher.client.closed.is_none());
    assert_eq!(qh.binding(watcher.conn()), Some(watcher.peer()));

    // Negative control: a `/1` watcher fences with cause `Narrowed`, code 9.
    let (mut lane_qh, mut lane_clock) = start_dual(protocol);
    let lane_id = register(&mut lane_qh, &cred_bytes(2), control(&[]), full_grants());
    register(&mut lane_qh, &cred_bytes(3), all_actors(), full_grants());
    let (mut lane_watch, _) = connect_raw(
        &mut lane_qh,
        &mut lane_clock,
        protocol,
        &cred_bytes(2),
        Framing::V1,
    );
    let (mut lane_actor, _) = connect_raw(
        &mut lane_qh,
        &mut lane_clock,
        protocol,
        &cred_bytes(3),
        Framing::V1,
    );
    lane_watch.read_until("lane watcher state", |c| !c.frames.is_empty());
    lane_actor.read_until("lane actor state", |c| !c.frames.is_empty());
    let seq1 = declare(lane_actor.version, lane_actor.epoch, 1, 0, 1, strike(), 2);
    lane_actor.send(seq1);
    pump_until(
        &mut lane_qh,
        &mut lane_clock,
        100,
        "lane seq 1",
        |_, report| {
            report
                .calls
                .iter()
                .any(|call| matches!(call, HostCall::Ingest { frame: 1, .. }))
        },
    );
    let change = lane_qh
        .set_invitation_grants(lane_id, a_no_health_b(full_grants()))
        .expect("narrowing a live `/1` binding returns a fence");
    let GrantChange::Fenced(fence) = change else {
        panic!("expected a fence, got {change:?}");
    };
    assert_eq!(fence.cause, FenceCause::Narrowed);
    lane_watch.read_until("fenced", |c| c.closed.is_some());
    assert_eq!(close_code(lane_watch.closed.expect("closed")), Some(9));
}
both_versions!(v2_narrowing_is_seamless);

// ---------------------------------------------------------------------------
// S§16 case 3.
// ---------------------------------------------------------------------------

#[test]
fn v2_upstream_transfer_frames_are_traced_in_order() {
    let protocol = V2;
    let (mut qh, mut clock) = start_dual(protocol);
    register(&mut qh, &cred_bytes(0), all_actors(), full_grants());
    let (mut client, _) = connect_raw(&mut qh, &mut clock, protocol, &cred_bytes(0), Framing::V2);
    client.read_until("state", |c| !c.frames.is_empty());
    let epoch = client.epoch;
    let other = [0xFF; 16];

    client.send(end_turn_for(client.version, epoch, 1));
    client
        .quic
        .try_send_channel(Channel::Transfer, &published(epoch, 999))
        .expect("transfer send");
    client.sent.push(published(epoch, 999));
    client
        .quic
        .try_send_channel(Channel::Transfer, &[0x81, 0x02, 0x20])
        .expect("transfer send");
    client.sent.push(vec![0x81, 0x02, 0x20]);
    client
        .quic
        .try_send_channel(Channel::Transfer, &published(other, 1))
        .expect("transfer send");
    client.sent.push(published(other, 1));
    client.send(end_turn_for(client.version, epoch, 2));

    let conn = client.conn;
    until("five frames queued", || {
        qh.connection_stats(conn)
            .is_some_and(|stats| stats.inbound_frames == 5)
    });
    let report = pump(&mut qh, &mut clock);
    let ingest_calls: Vec<HostCall> = report
        .calls
        .iter()
        .filter(|call| {
            matches!(
                call,
                HostCall::Ingest { .. } | HostCall::IngestTransfer { .. }
            )
        })
        .cloned()
        .collect();
    assert_eq!(
        ingest_calls,
        vec![
            HostCall::Ingest {
                conn,
                peer: client.peer,
                frame: 1,
                len: client.sent[0].len(),
                result: crpg_server::host::IngestDisposition::Staged,
            },
            HostCall::IngestTransfer {
                conn,
                peer: client.peer,
                frame: 2,
                len: client.sent[1].len(),
                result: TransferDisposition::Refused(
                    crpg_server::host::TransferRefusal::UnknownTransfer
                ),
            },
            HostCall::IngestTransfer {
                conn,
                peer: client.peer,
                frame: 3,
                len: 3,
                result: TransferDisposition::Refused(crpg_server::host::TransferRefusal::Malformed),
            },
            HostCall::IngestTransfer {
                conn,
                peer: client.peer,
                frame: 4,
                len: client.sent[3].len(),
                result: TransferDisposition::Refused(
                    crpg_server::host::TransferRefusal::WrongEpoch
                ),
            },
            HostCall::Ingest {
                conn,
                peer: client.peer,
                frame: 5,
                len: client.sent[4].len(),
                result: crpg_server::host::IngestDisposition::Staged,
            },
        ]
    );
    assert_eq!(report.frames_refused, 3);
    assert!(report.fences.is_empty());
    let results = summary(&report).results;
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].status, APPLIED);
    assert_eq!(results[1].status, APPLIED);
    assert_eq!(results[0].seq, Some(1));
    assert_eq!(results[1].seq, Some(2));

    // Control: an honoured `Resync` is traced, then delivers Begin + Chunk on
    // the transfer channel.
    client
        .quic
        .try_send_channel(
            Channel::Transfer,
            &resync(epoch, None, ResyncReason::Overflow),
        )
        .expect("transfer send");
    client
        .sent
        .push(resync(epoch, None, ResyncReason::Overflow));
    let steps = pump_until(&mut qh, &mut clock, 100, "Resynced traced", |_, report| {
        report.calls.iter().any(|call| {
            matches!(
                call,
                HostCall::IngestTransfer {
                    result: TransferDisposition::Resynced,
                    ..
                }
            )
        })
    });
    assert!(steps.iter().all(|(_, report)| report.fences.is_empty()));
    // Read the transfer channel explicitly.
    let mut transfers = Vec::new();
    let start = std::time::Instant::now();
    while transfers.len() < 2 {
        match client.quic.try_recv_channel() {
            Ok(Some((Channel::Transfer, bytes))) => transfers.push(bytes),
            Ok(Some((Channel::Lane0, _))) => {}
            Ok(Some((Channel::Resume, _))) => panic!("no resume frame"),
            Ok(None) => {}
            Err(_) => panic!("closed before the snapshot arrived"),
        }
        assert!(start.elapsed() < DEADLINE, "deadline: snapshot frames");
        client.quic.wait(Duration::from_millis(5));
    }
    assert_eq!(transfers.len(), 2, "{transfers:?}");
    let begin = decode_begin(&transfers[0]);
    assert_eq!(begin.transfer, transfer_id(1));
    assert!(matches!(
        decode_transfer(&transfers[1]),
        TransferDown::Chunk(_)
    ));
}

// ---------------------------------------------------------------------------
// S§16 case 4.
// ---------------------------------------------------------------------------

#[test]
fn v2_resync_after_client_expiry_restores_the_replica() {
    let protocol = V2;
    let (mut qh, mut clock) = start_dual(protocol);
    register(&mut qh, &cred_bytes(0), control(&[]), full_grants());
    register(&mut qh, &cred_bytes(1), all_actors(), full_grants());
    let (watcher, _) = connect_raw(&mut qh, &mut clock, protocol, &cred_bytes(0), Framing::V2);
    let (mut actor, _) = connect_raw(&mut qh, &mut clock, protocol, &cred_bytes(1), Framing::V1);
    let mut watcher = TxClient::new(watcher, protocol);
    read_n(&mut watcher, clock.now(), 1, "watcher state");
    actor.read_until("actor state", |c| !c.frames.is_empty());

    let seq1 = declare(actor.version, actor.epoch, 1, 0, 1, strike(), 2);
    exchange(&mut qh, &mut clock, &mut actor, seq1);
    read_n(&mut watcher, clock.now(), 2, "seq 1 delta");

    let narrowing = a_no_health_b(full_grants());
    assert_eq!(
        qh.set_invitation_grants(watcher.invitation(), narrowing),
        Ok(GrantChange::SnapshotStaged)
    );
    watcher.withhold_chunk = true;
    pump_until(
        &mut qh,
        &mut clock,
        100,
        "snapshot delivered",
        |_, report| report.frames_sent >= 2,
    );
    read_n(&mut watcher, clock.now(), 4, "begin and withheld chunk");
    let begin = decode_begin(&watcher.recv[2].1);
    assert_eq!(begin.transfer, transfer_id(1));
    assert_eq!(watcher.recv[3].0, Channel::Transfer);
    assert!(matches!(
        decode_transfer(&watcher.recv[3].1),
        TransferDown::Chunk(_)
    ));
    assert!(watcher
        .pool
        .status(watcher.rx)
        .expect("receiver")
        .in_flight
        .is_some());

    // The client expires the incomplete transfer.
    let expired = watcher.pool.poll(clock.now() + 5_000);
    assert_eq!(expired, vec![watcher.rx]);
    let status = watcher.pool.status(watcher.rx).expect("receiver");
    assert_eq!(status.resync, Some(ResyncReason::Expired));

    watcher.send_transfer(resync(watcher.epoch(), Some(1), ResyncReason::Expired));
    let steps = pump_until(&mut qh, &mut clock, 100, "Resynced traced", |_, report| {
        report.calls.iter().any(|call| {
            matches!(
                call,
                HostCall::IngestTransfer {
                    peer,
                    result: TransferDisposition::Resynced,
                    ..
                } if *peer == watcher.peer()
            )
        })
    });
    assert!(steps.iter().all(|(_, report)| report.fences.is_empty()));
    read_n(&mut watcher, clock.now(), 7, "cancel and replacement");
    let cancel = decode_cancel(&watcher.recv[4].1);
    assert_eq!(cancel, (transfer_id(1), CancelReason::Superseded));
    let begin = decode_begin(&watcher.recv[5].1);
    assert_eq!(begin.transfer, transfer_id(2));
    assert_eq!(decode_chunk(&watcher.recv[6].1), a_no_health_b_body());
    assert_eq!(
        watcher.replica,
        expected_hidden_a(
            &[(2, (7, 10, false)), (3, (3, 3, false))],
            Some((Some(2), 0)),
        )
    );
    let steps = pump_until(
        &mut qh,
        &mut clock,
        100,
        "replacement published",
        |_, report| {
            report.calls.iter().any(|call| {
                matches!(
                    call,
                    HostCall::IngestTransfer {
                        peer,
                        result: TransferDisposition::Published,
                        ..
                    } if *peer == watcher.peer()
                )
            })
        },
    );
    assert!(steps.iter().all(|(_, report)| report.fences.is_empty()));

    // Control: without withholding, transfer 1 publishes and no Cancel is
    // sent.
    let (mut cqh, mut cclock) = start_dual(protocol);
    register(&mut cqh, &cred_bytes(0), control(&[]), full_grants());
    register(&mut cqh, &cred_bytes(1), all_actors(), full_grants());
    let (cwatch, _) = connect_raw(&mut cqh, &mut cclock, protocol, &cred_bytes(0), Framing::V2);
    let (mut cactor, _) = connect_raw(&mut cqh, &mut cclock, protocol, &cred_bytes(1), Framing::V1);
    let mut cwatch = TxClient::new(cwatch, protocol);
    read_n(&mut cwatch, cclock.now(), 1, "control state");
    cactor.read_until("control actor state", |c| !c.frames.is_empty());
    let seq1 = declare(cactor.version, cactor.epoch, 1, 0, 1, strike(), 2);
    exchange(&mut cqh, &mut cclock, &mut cactor, seq1);
    read_n(&mut cwatch, cclock.now(), 2, "control seq 1");
    assert_eq!(
        cqh.set_invitation_grants(cwatch.invitation(), a_no_health_b(full_grants())),
        Ok(GrantChange::SnapshotStaged)
    );
    pump_until(
        &mut cqh,
        &mut cclock,
        100,
        "control snapshot delivered",
        |_, report| report.frames_sent >= 2,
    );
    read_n(&mut cwatch, cclock.now(), 4, "control begin and chunk");
    let steps = pump_until(
        &mut cqh,
        &mut cclock,
        100,
        "control published",
        |_, report| {
            report.calls.iter().any(|call| {
                matches!(
                    call,
                    HostCall::IngestTransfer {
                        result: TransferDisposition::Published,
                        ..
                    }
                )
            })
        },
    );
    assert!(steps.iter().all(|(_, report)| report.fences.is_empty()));
    assert!(!cwatch.recv.iter().any(|(channel, bytes)| {
        *channel == Channel::Transfer
            && matches!(decode_transfer(bytes), TransferDown::Cancel { .. })
    }));
    assert_eq!(
        cwatch
            .pool
            .status(cwatch.rx)
            .expect("receiver")
            .published
            .map(|p| p.transfer),
        Some(transfer_id(1))
    );
}

// ---------------------------------------------------------------------------
// S§16 case 5.
// ---------------------------------------------------------------------------

fn read_to_acks(report: &QuicPumpReport, watcher: &mut TxClient, actor: &mut Client, now: u64) {
    let mut w_ack = 0;
    let mut a_ack = 0;
    for call in &report.calls {
        if let HostCall::AcknowledgeDelivery { peer, through } = call {
            if *peer == watcher.peer() {
                w_ack = w_ack.max(*through);
            }
            if *peer == actor.peer {
                a_ack = a_ack.max(*through);
            }
        }
    }
    if w_ack > 0 {
        watcher.read_until(now, "watcher holds its acknowledged frames", |t| {
            t.recv.len() as u64 >= w_ack
        });
    }
    if a_ack > 0 {
        actor.read_until("actor holds its acknowledged frames", |c| {
            c.frames.len() as u64 >= a_ack
        });
    }
    for call in &report.calls {
        if let HostCall::AcknowledgeDelivery { peer, through } = call {
            if *peer == watcher.peer() {
                assert_eq!(
                    *through,
                    watcher.recv.len() as u64,
                    "`/2` acknowledgement counts every channel"
                );
            }
            if *peer == actor.peer {
                assert_eq!(
                    *through,
                    actor.frames.len() as u64,
                    "`/1` acknowledgement counts deltas"
                );
            }
        }
    }
}

fn assert_take_empty(qh: &mut QuicHost, clock: &mut TestClock, peer: PeerHandle) {
    let report = pump(qh, clock);
    assert_eq!(take_count(&report, peer), Some(0), "{report:?}");
    assert_eq!(report.frames_sent, 0);
}

#[test]
fn v2_acknowledges_by_ordinal() {
    let protocol = V2;
    let (mut qh, mut clock) = start_dual(protocol);
    register(&mut qh, &cred_bytes(0), control(&[]), full_grants());
    register(&mut qh, &cred_bytes(1), all_actors(), full_grants());
    let (watcher, _) = connect_raw(&mut qh, &mut clock, protocol, &cred_bytes(0), Framing::V2);
    let (mut actor, _) = connect_raw(&mut qh, &mut clock, protocol, &cred_bytes(1), Framing::V1);
    let mut watcher = TxClient::new(watcher, protocol);
    read_n(&mut watcher, clock.now(), 1, "watcher state");
    actor.read_until("actor state", |c| !c.frames.is_empty());

    let seq1 = declare(actor.version, actor.epoch, 1, 0, 1, strike(), 2);
    let report = exchange(&mut qh, &mut clock, &mut actor, seq1);
    read_to_acks(&report, &mut watcher, &mut actor, clock.now());
    assert_take_empty(&mut qh, &mut clock, watcher.peer());

    assert_eq!(
        qh.set_invitation_grants(watcher.invitation(), a_no_health_b(full_grants())),
        Ok(GrantChange::SnapshotStaged)
    );
    let report = pump(&mut qh, &mut clock);
    read_to_acks(&report, &mut watcher, &mut actor, clock.now());
    assert_take_empty(&mut qh, &mut clock, watcher.peer());

    let seq2 = declare(actor.version, actor.epoch, 2, 0, 2, strike(), 3);
    let report = exchange(&mut qh, &mut clock, &mut actor, seq2);
    read_to_acks(&report, &mut watcher, &mut actor, clock.now());
    assert_take_empty(&mut qh, &mut clock, watcher.peer());
    assert!(watcher.client.closed.is_none());
}

// ---------------------------------------------------------------------------
// S§16 case 6.
// ---------------------------------------------------------------------------

#[test]
fn v2_held_lane0_frame_blocks_transfer_frames_behind_it() {
    let protocol = V2;
    let (mut qh, mut clock) = start_dual(protocol);
    register(&mut qh, &cred_bytes(0), all_actors(), full_grants());
    register(&mut qh, &cred_bytes(1), control(&[]), observer_grants());
    let (mut actor, _) = connect_raw(&mut qh, &mut clock, protocol, &cred_bytes(0), Framing::V2);
    let addr = qh.local_addr();
    let (mut observer, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        stalled_client_limits(),
        protocol,
        &cred_bytes(1),
    );
    actor.read_until("actor state", |c| !c.frames.is_empty());

    // Fill until one lane-0 frame is held behind host ingress backpressure.
    let mut seq = 1u64;
    let mut held_report = None;
    for _ in 0..2_000 {
        let step = drive_one(&mut qh, &mut clock, &mut actor, &mut seq, PACE);
        if step.1.held == 1 {
            held_report = Some(step.1);
            break;
        }
    }
    let held_report = held_report.expect("deadline: a frame is held");
    let held_ordinal = held_report
        .calls
        .iter()
        .find_map(|call| match call {
            HostCall::Ingest {
                frame,
                result:
                    crpg_server::host::IngestDisposition::Refused {
                        status: ReceiptStatus::Rejected(RejectionCode::QueueFull),
                    },
                ..
            } => Some(*frame),
            _ => None,
        })
        .expect("the held frame's ordinal");

    // The transfer frame queues behind the held lane-0 frame.
    let transfer = published(actor.epoch, 999);
    actor
        .quic
        .try_send_channel(Channel::Transfer, &transfer)
        .expect("transfer send");
    actor.sent.push(transfer);
    let transfer_ordinal = actor.sent.len() as u64;

    // While the lane-0 frame is held, no transfer frame is traced.
    let report = pump(&mut qh, &mut clock);
    assert!(!report
        .calls
        .iter()
        .any(|call| matches!(call, HostCall::IngestTransfer { .. })));
    assert!(report.calls.iter().any(|call| matches!(
        call,
        HostCall::Ingest { frame, result: crpg_server::host::IngestDisposition::Refused {
            status: ReceiptStatus::Rejected(RejectionCode::QueueFull)
        }, .. } if *frame == held_ordinal
    )));

    // Let the observer drain so admission can make room; the held frame then
    // stages and the transfer frame is taken immediately after it.
    let mut staged = None;
    let start = std::time::Instant::now();
    while staged.is_none() {
        observer.read_available();
        actor.read_available();
        clock.advance(100);
        let report = qh.pump(clock.now()).expect("pump");
        let calls = &report.calls;
        let transfer_at = calls.iter().position(|call| {
            matches!(call, HostCall::IngestTransfer { frame, .. } if *frame == transfer_ordinal)
        });
        let ingest_at = calls.iter().position(
            |call| matches!(call, HostCall::Ingest { frame, .. } if *frame == held_ordinal),
        );
        if let (Some(ingest_at), Some(transfer_at)) = (ingest_at, transfer_at) {
            assert!(ingest_at < transfer_at, "IngestTransfer follows its Ingest");
            staged = Some(report);
        }
        assert!(start.elapsed() < DEADLINE, "deadline: held frame stages");
        qh.wait(Duration::from_millis(5));
    }
    let report = staged.expect("staged");
    assert!(report.fences.is_empty());
}

// ---------------------------------------------------------------------------
// S§16 case 7: the schedule replays in memory.
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    hash: [u8; 32],
    executions: u64,
    last_capture: u64,
    delivered: BTreeMap<PeerHandle, Vec<Vec<u8>>>,
}

/// Replays every traced host call against a fresh in-memory host. `force_lane0`
/// makes every `BindSnapshotPeer` a `bind_peer` (negative control);
/// `check_mirrors` compares the out-of-pump `set_grants` results (also a
/// negative control when off).
fn replay(
    protocol: ProtocolSelection,
    steps: &[Step],
    sent: &BTreeMap<ConnectionId, Vec<Vec<u8>>>,
    grants: &BTreeMap<InvitationId, (ControlGrant, DisclosureGrants)>,
    mirrors: &[Mirror],
    force_lane0: bool,
    check_mirrors: bool,
) -> Result<Outcome, String> {
    let mut host = new_host(protocol);
    let mut invitation_peer: BTreeMap<InvitationId, PeerHandle> = BTreeMap::new();
    let mut pending: BTreeMap<PeerHandle, Vec<crpg_server::host::DeliveryEntry>> = BTreeMap::new();
    let mut delivered: BTreeMap<PeerHandle, Vec<Vec<u8>>> = BTreeMap::new();
    for (index, (now, report)) in steps.iter().enumerate() {
        for call in &report.calls {
            match call {
                HostCall::BindPeer {
                    invitation, result, ..
                } => {
                    let (control, grants) = grants.get(invitation).ok_or("unknown invitation")?;
                    let got = host.bind_peer(control.clone(), grants.clone(), *now);
                    if got != *result {
                        return Err(format!("step {index}: bind {got:?} != {result:?}"));
                    }
                    if let Ok(peer) = got {
                        invitation_peer.insert(*invitation, peer);
                    }
                }
                HostCall::BindSnapshotPeer {
                    invitation, result, ..
                } => {
                    let (control, grants) = grants.get(invitation).ok_or("unknown invitation")?;
                    let got = if force_lane0 {
                        host.bind_peer(control.clone(), grants.clone(), *now)
                    } else {
                        host.bind_snapshot_peer(control.clone(), grants.clone(), *now)
                    };
                    if got != *result {
                        return Err(format!("step {index}: snapshot bind {got:?} != {result:?}"));
                    }
                    if let Ok(peer) = got {
                        invitation_peer.insert(*invitation, peer);
                    }
                }
                HostCall::UnbindPeer { peer } => host.unbind_peer(*peer),
                HostCall::Ingest {
                    conn,
                    peer,
                    frame,
                    len,
                    result,
                } => {
                    let bytes = sent
                        .get(conn)
                        .and_then(|log| log.get(*frame as usize - 1))
                        .ok_or(format!("step {index}: no frame {frame} on {conn:?}"))?;
                    if bytes.len() != *len {
                        return Err(format!("step {index}: length differs"));
                    }
                    let got = host.ingest(*peer, bytes, *now);
                    if got != Ok(result.clone()) {
                        return Err(format!("step {index}: ingest {got:?} != {result:?}"));
                    }
                }
                HostCall::IngestTransfer {
                    conn,
                    peer,
                    frame,
                    len,
                    result,
                } => {
                    let bytes = sent
                        .get(conn)
                        .and_then(|log| log.get(*frame as usize - 1))
                        .ok_or(format!("step {index}: no frame {frame} on {conn:?}"))?;
                    if bytes.len() != *len {
                        return Err(format!("step {index}: length differs"));
                    }
                    let got = host.ingest_transfer(*peer, bytes, *now);
                    if got != Ok(result.clone()) {
                        return Err(format!("step {index}: transfer {got:?} != {result:?}"));
                    }
                }
                HostCall::Pump { summary } => {
                    let got = host.pump(*now);
                    if got.as_ref() != Ok(summary) {
                        return Err(format!("step {index}: pump {got:?} != {summary:?}"));
                    }
                }
                HostCall::TakeDelivery { peer, result } => {
                    let got = host.take_delivery_entries(*peer);
                    let count = got.as_ref().map(Vec::len).map_err(Clone::clone);
                    if count != *result {
                        return Err(format!("step {index}: take {count:?} != {result:?}"));
                    }
                    if let Ok(entries) = got {
                        pending.entry(*peer).or_default().extend(entries);
                    }
                }
                HostCall::AcknowledgeDelivery { peer, through } => {
                    host.acknowledge_delivery(*peer, *through)
                        .map_err(|e| format!("step {index}: ack {e:?}"))?;
                    if let Some(entries) = pending.get_mut(peer) {
                        let mut keep = Vec::new();
                        for entry in entries.drain(..) {
                            if entry.ordinal <= *through {
                                delivered.entry(*peer).or_default().push(entry.bytes);
                            } else {
                                keep.push(entry);
                            }
                        }
                        *entries = keep;
                    }
                }
            }
        }
        for mirror in mirrors.iter().filter(|mirror| mirror.after == index + 1) {
            let peer = invitation_peer
                .get(&mirror.invitation)
                .copied()
                .ok_or(format!("step {index}: mirror without a binding"))?;
            let got = host.set_grants(peer, mirror.grants.clone());
            if check_mirrors && got != Ok(mirror.result.clone()) {
                return Err(format!(
                    "step {index}: grants {got:?} != {:?}",
                    mirror.result
                ));
            }
        }
    }
    Ok(Outcome {
        hash: host.authority_hash(),
        executions: host.executions(),
        last_capture: host.last_capture(),
        delivered,
    })
}

/// Sends one lane-0 frame and returns every pump step up to its ingest.
fn exchange_all(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    client: &mut Client,
    bytes: Vec<u8>,
) -> Vec<Step> {
    let conn = client.conn;
    client.send(bytes);
    let ordinal = client.sent.len() as u64;
    let steps = pump_until(qh, clock, 100, "frame ingested", |_, report| {
        report.calls.iter().any(|call| {
            matches!(call, HostCall::Ingest { conn: c, frame, .. } if *c == conn && *frame == ordinal)
        })
    });
    assert!(steps.iter().all(|(_, report)| report.fences.is_empty()));
    steps
}

fn v2_schedule_replays_in_memory(protocol: ProtocolSelection) {
    let (mut qh, mut clock) = start_dual(protocol);
    let watch_id = register(&mut qh, &cred_bytes(0), control(&[]), full_grants());
    let actor_id = register(&mut qh, &cred_bytes(1), all_actors(), full_grants());
    let mut grants: BTreeMap<InvitationId, (ControlGrant, DisclosureGrants)> = BTreeMap::new();
    grants.insert(watch_id, (control(&[]), full_grants()));
    grants.insert(actor_id, (all_actors(), full_grants()));
    let mut steps = Vec::new();
    let (watcher, more) = connect_raw(&mut qh, &mut clock, protocol, &cred_bytes(0), Framing::V2);
    steps.extend(more);
    let (mut actor, more) = connect_raw(&mut qh, &mut clock, protocol, &cred_bytes(1), Framing::V1);
    steps.extend(more);
    let mut watcher = TxClient::new(watcher, protocol);
    read_n(&mut watcher, clock.now(), 1, "watcher state");
    actor.read_until("actor state", |c| !c.frames.is_empty());

    let seq1 = declare(actor.version, actor.epoch, 1, 0, 1, strike(), 2);
    steps.extend(exchange_all(&mut qh, &mut clock, &mut actor, seq1));
    read_n(&mut watcher, clock.now(), 2, "seq 1 delta");

    let narrowing = a_no_health_b(full_grants());
    let mirror = Mirror {
        after: steps.len(),
        invitation: watch_id,
        grants: narrowing.clone(),
        result: GrantUpdate::SnapshotStaged,
    };
    assert_eq!(
        qh.set_invitation_grants(watch_id, narrowing),
        Ok(GrantChange::SnapshotStaged)
    );
    let more = pump_until(
        &mut qh,
        &mut clock,
        100,
        "snapshot delivered",
        |_, report| report.frames_sent >= 2,
    );
    steps.extend(more);
    read_n(&mut watcher, clock.now(), 4, "begin and chunk");
    let more = pump_until(&mut qh, &mut clock, 100, "Published traced", |_, report| {
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
    steps.extend(more);

    let seq2 = declare(actor.version, actor.epoch, 2, 0, 2, strike(), 3);
    steps.extend(exchange_all(&mut qh, &mut clock, &mut actor, seq2));
    let actor_ack = highest_ack(&steps, actor.peer);
    let watch_ack = highest_ack(&steps, watcher.peer());
    actor.read_until("actor holds every acknowledged frame", |c| {
        c.frames.len() as u64 >= actor_ack
    });
    read_n(
        &mut watcher,
        clock.now(),
        watch_ack as usize,
        "watcher holds every acknowledged frame",
    );

    let sent = BTreeMap::from([
        (watcher.conn(), watcher.client.sent.clone()),
        (actor.conn, actor.sent.clone()),
    ]);
    let mut expected_delivered = BTreeMap::new();
    expected_delivered.insert(watcher.peer(), watcher.received_bytes());
    expected_delivered.insert(
        actor.peer,
        actor
            .frames
            .iter()
            .map(|frame| frame.bytes.clone())
            .collect(),
    );
    let mirrors = vec![mirror];

    let outcome = replay(protocol, &steps, &sent, &grants, &mirrors, false, true)
        .expect("the `/2` trace replays");
    assert_eq!(outcome.hash, qh.host().authority_hash());
    assert_eq!(outcome.executions, qh.host().executions());
    assert_eq!(outcome.last_capture, qh.host().last_capture());
    assert_eq!(outcome.delivered, expected_delivered);

    // Negative control 1: a lane-0 bind diverges at the mirrored narrowing.
    let error = replay(protocol, &steps, &sent, &grants, &mirrors, true, true)
        .expect_err("bind_peer in place of bind_snapshot_peer must diverge");
    assert!(error.contains("grants"), "{error}");

    // Negative control 2: with the grant check skipped, the trace alone
    // diverges at the next pump's watcher delivery count.
    let error = replay(protocol, &steps, &sent, &grants, &mirrors, true, false)
        .expect_err("a lane-0 session delivers one reset frame, not two");
    assert!(error.contains("take"), "{error}");
    assert!(!error.contains("transfer"), "{error}");
}
both_versions!(v2_schedule_replays_in_memory);

// ---------------------------------------------------------------------------
// S§16 case 8.
// ---------------------------------------------------------------------------

#[test]
fn v2_shutdown_flushes_transfer_entries_by_channel_and_ordinal() {
    let protocol = V2;
    let (mut qh, mut clock) = start_dual(protocol);
    register(&mut qh, &cred_bytes(0), control(&[]), full_grants());
    let (watcher, _) = connect_raw(&mut qh, &mut clock, protocol, &cred_bytes(0), Framing::V2);
    let mut watcher = TxClient::new(watcher, protocol);
    read_n(&mut watcher, clock.now(), 1, "state delta");

    assert_eq!(
        qh.set_invitation_grants(watcher.invitation(), a_no_health_b(full_grants())),
        Ok(GrantChange::SnapshotStaged)
    );
    assert_eq!(
        qh.set_invitation_grants(watcher.invitation(), a_no_health(full_grants())),
        Ok(GrantChange::SnapshotStaged)
    );

    let shutdown = qh.shutdown(false);
    assert_eq!(shutdown.undelivered_frames, 0);
    watcher.feed = false;
    watcher.read_until(clock.now(), "shutdown close", |t| t.client.closed.is_some());
    let received = &watcher.recv;
    assert_eq!(received.len(), 4, "{received:?}");
    let transfers: Vec<&(Channel, Vec<u8>)> = received[1..].iter().collect();
    assert_eq!(transfers[0].0, Channel::Transfer);
    assert_eq!(
        decode_cancel(&transfers[0].1),
        (transfer_id(1), CancelReason::Aborted)
    );
    let begin = decode_begin(&transfers[1].1);
    assert_eq!(begin.transfer, transfer_id(2));
    assert_eq!(begin.cursor, 1);
    let body = codec_transfer::decode_body(&decode_chunk(&transfers[2].1)).expect("decodes");
    assert!(body.entities.iter().all(|entity| entity.entity.get() != 2));
    assert!(body
        .entities
        .iter()
        .all(|entity| entity.entity.get() != 1 || entity.health.is_none()));
    assert!(!transfers.iter().any(|(_, bytes)| {
        matches!(decode_transfer(bytes), TransferDown::Begin(begin) if begin.transfer == transfer_id(1))
    }));
}

// ---------------------------------------------------------------------------
// S§16 case 9.
// ---------------------------------------------------------------------------

#[test]
fn v2_retiring_connection_drains_transfer_frames() {
    let protocol = V2;
    let (mut qh, mut clock) = start_dual(protocol);
    let id = register(&mut qh, &cred_bytes(0), all_actors(), full_grants());
    let (mut client, _) = connect_raw(&mut qh, &mut clock, protocol, &cred_bytes(0), Framing::V2);
    client.read_until("state", |c| !c.frames.is_empty());

    client
        .quic
        .try_send_channel(Channel::Transfer, &published(client.epoch, 999))
        .expect("transfer send");
    client.sent.push(published(client.epoch, 999));
    client.send(end_turn_for(client.version, client.epoch, 1));

    let conn = client.conn;
    until("two frames queued", || {
        qh.connection_stats(conn)
            .is_some_and(|stats| stats.inbound_frames == 2)
    });
    let fence = qh
        .revoke_invitation(id)
        .expect("registered")
        .expect("a live binding is fenced");
    assert_eq!(fence.cause, FenceCause::Revoked);

    let steps = pump_until(&mut qh, &mut clock, 100, "record released", |qh, _| {
        qh.connection_stats(conn).is_none()
    });
    assert_eq!(
        steps
            .iter()
            .map(|(_, report)| report.frames_discarded)
            .sum::<usize>(),
        2
    );
    assert!(!steps
        .iter()
        .any(|(_, report)| report.calls.iter().any(|call| {
            match call {
                HostCall::Ingest { conn: c, .. } | HostCall::IngestTransfer { conn: c, .. } => {
                    *c == conn
                }
                _ => false,
            }
        })));
    assert_eq!(qh.transport_totals().inbound_frames, 0);
}

// ---------------------------------------------------------------------------
// S§16 case 10.
// ---------------------------------------------------------------------------

#[test]
fn v2_departed_peer_acknowledges_through_the_last_ordinal() {
    let protocol = V2;
    let (mut qh, mut clock) = start_dual(protocol);
    register(&mut qh, &cred_bytes(0), control(&[]), full_grants());
    let (watcher, _) = connect_raw(&mut qh, &mut clock, protocol, &cred_bytes(0), Framing::V2);
    let mut watcher = TxClient::new(watcher, protocol);
    read_n(&mut watcher, clock.now(), 1, "state delta");

    // Quiet: every event polled, nothing queued.
    let peer = watcher.peer();
    let invitation = watcher.invitation();
    let _ = watcher
        .client
        .quic
        .close(CloseCode::Normal, CloseMode::Flush);
    assert!(
        qh.wait(Duration::from_secs(30)),
        "deadline: the queued Closed event"
    );

    assert_eq!(
        qh.set_invitation_grants(invitation, a_no_health_b(full_grants())),
        Ok(GrantChange::SnapshotStaged)
    );
    assert_eq!(
        qh.set_invitation_grants(invitation, a_no_health(full_grants())),
        Ok(GrantChange::SnapshotStaged)
    );

    let report = pump(&mut qh, &mut clock);
    let calls = &report.calls;
    let take_at = calls
        .iter()
        .position(
            |call| matches!(call, HostCall::TakeDelivery { peer: p, result: Ok(3) } if *p == peer),
        )
        .expect("the departed peer's three entries are taken");
    let ack_at = calls
        .iter()
        .position(|call| matches!(call, HostCall::AcknowledgeDelivery { peer: p, through: 6 } if *p == peer))
        .expect("acknowledged through the last ordinal");
    let unbind_at = calls
        .iter()
        .position(|call| matches!(call, HostCall::UnbindPeer { peer: p } if *p == peer))
        .expect("unbound in phase 6");
    assert!(take_at < ack_at && ack_at < unbind_at, "{calls:?}");
    assert_eq!(report.frames_discarded, 3);
    assert_eq!(report.frames_sent, 0);
}
