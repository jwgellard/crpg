#![allow(dead_code)] // Test support: each case uses a different subset of these helpers.
//! Test-only rig for `tests/host_quic.rs` (T023b B§14.1 and A§3): loopback
//! builders on `127.0.0.1:0`, a `QuicHost` starter, the synthetic
//! `TestClock`, failure-guard waits with a hard 30 s deadline, a test
//! client that records what it sent and received, copies of
//! `host_capture.rs`'s fixture loader, grant builders, intent encoders,
//! `decode_frame`, `Replica` and `expected_state`, the stall-case limits
//! and the `drive`/`StallMirror` machinery, and `UdpRelay`, copied from
//! `crpg-net-quic`'s rig.
//!
//! Guards panic `"deadline"`; they are never oracles. Every assertion is
//! made on observed values: pump reports, host reads and client bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crpg_core::{EntityId, Ulid};
use crpg_net::codec;
use crpg_net::codec_v2;
use crpg_net::protocol::{self, NetId, ReceiptStatus, LANE_COMBAT};
use crpg_net::protocol_v2::DeltaOp;
use crpg_net_quic::{
    ClientConfig, ClientLimits, CloseReason, ConnectError, ConnectionId, Credential, Hello,
    QuicClient, QuicServer, ServerConfig, ServerIdentity, ServerLimits,
};
use crpg_server::host::{
    ControlGrant, DisclosureGrants, EntityDisclosure, Host, HostConfig, PeerHandle,
    ProtocolSelection,
};
use crpg_server::quic::{
    HelloDecision, HelloOutcome, HostCall, Invitation, InvitationId, QuicHost, QuicPumpReport,
};
use crpg_sim::HistoryWorld;
use serde_json::json;

// ---------------------------------------------------------------------------
// Fixtures and identities.
// ---------------------------------------------------------------------------

const FIXTURE: &str = include_str!("../fixtures/three_combatants.json");

/// T023's test server identity, read in place (B§16 Q10): byte-exact under
/// `.gitattributes`, so no copy and no root edit.
pub const SERVER_A_CERT: &[u8] =
    include_bytes!("../../../crpg-net-quic/tests/fixtures/server_a.cert.der");
pub const SERVER_A_KEY: &[u8] =
    include_bytes!("../../../crpg-net-quic/tests/fixtures/server_a.key.pk8.der");
/// T023's recorded pin of `server_a.cert.der`; drift fails here.
pub const SERVER_A_PIN_HEX: &str =
    "9da1730bd38407644e8677d66f182438b9eec028f4d2b538e14d352a26d7696b";

/// Hard bound on every guard wait.
pub const DEADLINE: Duration = Duration::from_secs(30);
/// The host incarnation every case uses.
pub const INCARNATION: u64 = 7;
/// `TestClock`'s start.
pub const CLOCK_START: u64 = 1_000;

pub const V1: ProtocolSelection = ProtocolSelection::V1;
pub const V2: ProtocolSelection = ProtocolSelection::V2;

pub fn eid(index: u32) -> EntityId {
    serde_json::from_value(json!({"index": index, "generation": 1})).expect("valid entity id")
}

pub fn a() -> EntityId {
    eid(0)
}
pub fn b() -> EntityId {
    eid(1)
}
pub fn c() -> EntityId {
    eid(2)
}

pub fn strike() -> Ulid {
    Ulid::from_u128(603)
}
pub fn smite() -> Ulid {
    Ulid::from_u128(604)
}
pub fn whiff() -> Ulid {
    Ulid::from_u128(605)
}
pub fn encounter() -> Ulid {
    Ulid::from_u128(601)
}

pub fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}

/// The fixture authority after the embedding acknowledged its start
/// envelopes (as `host_capture`'s `fixture()`).
pub fn fixture() -> HistoryWorld {
    let mut history: HistoryWorld =
        serde_json::from_str(FIXTURE).expect("fixture loads through validated serde");
    history
        .acknowledge(history.last_sequence())
        .expect("acknowledges its own range");
    history
}

pub fn full(entity: EntityId) -> EntityDisclosure {
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

pub fn full_grants() -> DisclosureGrants {
    DisclosureGrants {
        entities: vec![full(a()), full(b()), full(c())],
        abilities: vec![strike(), smite(), whiff()],
        encounters: vec![encounter()],
        turn_round: true,
        encounter_end: true,
        turn_state: true,
    }
}

/// Presence plus health plus turn state only: no action/turn/death facts.
pub fn observer_grants() -> DisclosureGrants {
    let observer = |entity| EntityDisclosure {
        entity,
        present: true,
        health: true,
        spawn: false,
        despawn: false,
        died: false,
        action_actor: false,
        action_target: false,
        action_outcome: false,
        action_damage: false,
        turn_start: false,
    };
    DisclosureGrants {
        entities: vec![observer(a()), observer(b()), observer(c())],
        turn_state: true,
        ..DisclosureGrants::default()
    }
}

pub fn control(actors: &[EntityId]) -> ControlGrant {
    ControlGrant {
        actors: actors.to_vec(),
    }
}

pub fn all_actors() -> ControlGrant {
    control(&[a(), b(), c()])
}

pub fn standard_ids() -> BTreeMap<EntityId, u64> {
    BTreeMap::from([(a(), 1), (b(), 2), (c(), 3)])
}

// ---------------------------------------------------------------------------
// Hand-encoded wire (independent of the codec).
// ---------------------------------------------------------------------------

pub fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

pub fn text(out: &mut Vec<u8>, value: &str) {
    varint(out, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
}

pub fn version_byte(protocol: ProtocolSelection) -> u8 {
    match protocol {
        ProtocolSelection::V1 => 1,
        ProtocolSelection::V2 => 2,
    }
}

pub fn intent_header(version: u8, epoch: [u8; 16], seq: u64, observed: u64, actor: u64) -> Vec<u8> {
    let mut out = vec![version];
    out.extend_from_slice(&epoch);
    out.push(LANE_COMBAT);
    varint(&mut out, seq);
    varint(&mut out, observed);
    varint(&mut out, actor);
    out
}

/// Hand-encoded `DeclareAction` intent bytes.
pub fn declare(
    version: u8,
    epoch: [u8; 16],
    seq: u64,
    observed: u64,
    actor: u64,
    ability: Ulid,
    target: u64,
) -> Vec<u8> {
    let mut out = intent_header(version, epoch, seq, observed, actor);
    out.push(0);
    text(&mut out, &ability.to_string());
    varint(&mut out, target);
    out
}

/// Hand-encoded `EndTurn` intent bytes.
pub fn end_turn(version: u8, epoch: [u8; 16], seq: u64, observed: u64, actor: u64) -> Vec<u8> {
    let mut out = intent_header(version, epoch, seq, observed, actor);
    out.push(1);
    out
}

/// A§3.3: `EndTurn` seq `seq` for actor NetId `(seq − 1) % 3 + 1`,
/// `observed_tick` 0.
pub fn end_turn_for(version: u8, epoch: [u8; 16], seq: u64) -> Vec<u8> {
    end_turn(version, epoch, seq, 0, (seq - 1) % 3 + 1)
}

pub fn epoch_of(incarnation: u64, handle: u64) -> [u8; 16] {
    let mut epoch = [0u8; 16];
    epoch[..8].copy_from_slice(&incarnation.to_le_bytes());
    epoch[8..].copy_from_slice(&handle.to_le_bytes());
    epoch
}

// ---------------------------------------------------------------------------
// Expected-op constructors.
// ---------------------------------------------------------------------------

pub fn legacy(op: protocol::DeltaOp) -> DeltaOp {
    DeltaOp::Legacy(op)
}
pub fn enter(id: u64) -> DeltaOp {
    legacy(protocol::DeltaOp::EntityEnter { entity: n(id) })
}
pub fn health(id: u64, health: u32, max_health: u32, dead: bool) -> DeltaOp {
    legacy(protocol::DeltaOp::Health {
        entity: n(id),
        health,
        max_health,
        dead,
    })
}
pub fn turn(active: Option<u64>, round: u64) -> DeltaOp {
    legacy(protocol::DeltaOp::Turn {
        active: active.map(n),
        round,
    })
}
pub fn receipt(epoch: [u8; 16], seq: u64, tick: u64, status: ReceiptStatus) -> DeltaOp {
    legacy(protocol::DeltaOp::Receipt {
        epoch,
        lane: LANE_COMBAT,
        seq,
        processed_tick: tick,
        status,
    })
}
pub fn resolved(actor: u64, target: u64, ability: Ulid, outcome: &str, damage: u32) -> DeltaOp {
    DeltaOp::ActionResolved {
        actor: n(actor),
        target: n(target),
        ability,
        outcome: outcome.to_owned(),
        damage,
    }
}
pub fn started(actor: u64, round: u64) -> DeltaOp {
    DeltaOp::TurnStarted {
        actor: n(actor),
        round,
    }
}

/// V1 carries receipts and state ops only, never event ops.
pub fn for_version(protocol: ProtocolSelection, ops: Vec<DeltaOp>) -> Vec<DeltaOp> {
    match protocol {
        ProtocolSelection::V2 => ops,
        ProtocolSelection::V1 => ops
            .into_iter()
            .filter(|op| matches!(op, DeltaOp::Legacy(_)))
            .collect(),
    }
}

pub fn initial_full_state() -> Vec<DeltaOp> {
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
// Decoded delivery and the replica.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub event_seq: u64,
    pub server_tick: u64,
    pub ops: Vec<DeltaOp>,
    pub bytes: Vec<u8>,
}

/// `None` when the bytes do not decode under `protocol`.
pub fn try_decode_frame(protocol: ProtocolSelection, bytes: &[u8]) -> Option<Frame> {
    match protocol {
        ProtocolSelection::V1 => codec::decode_delta(bytes).ok().map(|frame| Frame {
            event_seq: frame.event_seq,
            server_tick: frame.server_tick,
            ops: frame.ops.into_iter().map(DeltaOp::Legacy).collect(),
            bytes: bytes.to_vec(),
        }),
        ProtocolSelection::V2 => codec_v2::decode_delta(bytes).ok().map(|frame| Frame {
            event_seq: frame.event_seq,
            server_tick: frame.server_tick,
            ops: frame.ops,
            bytes: bytes.to_vec(),
        }),
    }
}

pub fn decode_frame(protocol: ProtocolSelection, bytes: &[u8]) -> Frame {
    try_decode_frame(protocol, bytes).expect("delivery decodes")
}

/// Permitted replica state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    pub entered: BTreeSet<u64>,
    pub health: BTreeMap<u64, (u32, u32, bool)>,
    pub turn: Option<(Option<u64>, u64)>,
}

/// A delivery gap: `expected` was due, `got` arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gap {
    pub expected: u64,
    pub got: u64,
}

/// A client replica assembled only from delivery bytes.
#[derive(Debug, Default, Clone)]
pub struct Replica {
    pub next: u64,
    pub state: State,
    pub events: Vec<DeltaOp>,
    pub receipts: Vec<DeltaOp>,
}

impl Replica {
    pub fn new() -> Self {
        Self {
            next: 1,
            ..Self::default()
        }
    }

    /// Applies new frames in order; duplicates are skipped, gaps fail.
    pub fn apply(&mut self, frames: &[Frame]) {
        for frame in frames {
            self.try_apply(frame).expect("delivery is gapless");
        }
    }

    /// Applies one frame, or reports the gap without changing anything.
    pub fn try_apply(&mut self, frame: &Frame) -> Result<(), Gap> {
        if frame.event_seq < self.next {
            return Ok(());
        }
        if frame.event_seq != self.next {
            return Err(Gap {
                expected: self.next,
                got: frame.event_seq,
            });
        }
        self.next += 1;
        for op in &frame.ops {
            match op {
                DeltaOp::Legacy(protocol::DeltaOp::EntityEnter { entity }) => {
                    self.state.entered.insert(entity.get());
                }
                DeltaOp::Legacy(protocol::DeltaOp::EntityLeave { entity }) => {
                    self.state.entered.remove(&entity.get());
                    self.state.health.remove(&entity.get());
                }
                DeltaOp::Legacy(protocol::DeltaOp::Health {
                    entity,
                    health,
                    max_health,
                    dead,
                }) => {
                    self.state
                        .health
                        .insert(entity.get(), (*health, *max_health, *dead));
                }
                DeltaOp::Legacy(protocol::DeltaOp::Turn { active, round }) => {
                    self.state.turn = Some((active.map(NetId::get), *round));
                }
                DeltaOp::Legacy(protocol::DeltaOp::Receipt { .. }) => {
                    self.receipts.push(op.clone());
                }
                other => self.events.push(other.clone()),
            }
        }
        Ok(())
    }
}

/// The independent permitted-state oracle: mirror reads plus hand-assigned
/// replica ids.
pub fn expected_state(
    oracle: &HistoryWorld,
    grants: &DisclosureGrants,
    ids: &BTreeMap<EntityId, u64>,
) -> State {
    let world = oracle.world();
    let mut state = State::default();
    let present: BTreeSet<EntityId> = grants
        .entities
        .iter()
        .filter(|entry| entry.present)
        .map(|entry| entry.entity)
        .collect();
    for entry in &grants.entities {
        if !entry.present {
            continue;
        }
        let id = ids[&entry.entity];
        state.entered.insert(id);
        if entry.health {
            if let Some(combatant) = world.combatants().get(entry.entity) {
                state.health.insert(
                    id,
                    (combatant.health(), combatant.max_health(), combatant.dead()),
                );
            }
        }
    }
    if grants.turn_state {
        if let Some(combat) = world.combat() {
            let active = combat
                .active
                .filter(|entity| present.contains(entity))
                .map(|entity| ids[&entity]);
            state.turn = Some((active, u64::from(combat.round)));
        }
    }
    state
}

/// Complete-state-unchanged evidence: authority hash, executions, captures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub hash: [u8; 32],
    pub executions: u64,
    pub last_capture: u64,
    pub capture_acknowledged: u64,
}

pub fn snapshot(host: &Host) -> Snapshot {
    Snapshot {
        hash: host.authority_hash(),
        executions: host.executions(),
        last_capture: host.last_capture(),
        capture_acknowledged: host.capture_acknowledged(),
    }
}

// ---------------------------------------------------------------------------
// Loopback builders, the clock and the adapter.
// ---------------------------------------------------------------------------

pub fn loopback() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

pub fn identity() -> ServerIdentity {
    let identity = ServerIdentity::new(SERVER_A_CERT.to_vec(), SERVER_A_KEY.to_vec());
    assert_eq!(
        identity.pin().to_hex(),
        SERVER_A_PIN_HEX,
        "fixture pin drift"
    );
    identity
}

pub fn bind_server(limits: ServerLimits) -> QuicServer {
    QuicServer::bind(ServerConfig {
        bind: loopback(),
        identity: identity(),
        limits,
    })
    .expect("bind loopback server")
}

pub fn client_config(limits: ClientLimits) -> ClientConfig {
    ClientConfig {
        bind: loopback(),
        pin: identity().pin(),
        limits,
    }
}

pub fn host_config(protocol: ProtocolSelection) -> HostConfig {
    HostConfig {
        protocol,
        incarnation: INCARNATION,
    }
}

/// The fixture host at `CLOCK_START`.
pub fn new_host(protocol: ProtocolSelection) -> Host {
    Host::from_history(fixture(), host_config(protocol), CLOCK_START)
        .expect("wraps an acknowledged authority")
}

/// A synthetic monotonic clock: starts at 1,000 ms, moves only when told.
#[derive(Debug, Clone, Copy)]
pub struct TestClock {
    now: u64,
}

impl TestClock {
    pub fn new() -> Self {
        Self { now: CLOCK_START }
    }

    pub fn now(&self) -> u64 {
        self.now
    }

    pub fn advance(&mut self, ms: u64) -> u64 {
        self.now += ms;
        self.now
    }

    pub fn set(&mut self, now: u64) {
        assert!(now >= self.now, "the test clock never goes back");
        self.now = now;
    }
}

/// A started adapter over the fixture host plus its clock.
pub fn start(protocol: ProtocolSelection, limits: ServerLimits) -> (QuicHost, TestClock) {
    let clock = TestClock::new();
    let qh = QuicHost::start(new_host(protocol), bind_server(limits), clock.now())
        .expect("starts over a fresh host");
    (qh, clock)
}

pub fn credential(bytes: &[u8]) -> Credential {
    Credential::new(bytes.to_vec()).expect("credential length in transport bounds")
}

/// Client A, B, C, … credentials: `[0xA1; 32]`, `[0xB2; 32]`, `[0xC3; 32]`, ….
pub fn cred_bytes(index: u8) -> Vec<u8> {
    vec![0xA1 + index * 0x11; 32]
}

pub fn invitation(bytes: &[u8], control: ControlGrant, grants: DisclosureGrants) -> Invitation {
    Invitation {
        credential: credential(bytes),
        control,
        grants,
    }
}

/// One pump with its injected time.
pub type Step = (u64, QuicPumpReport);

/// Pumps at `+step_ms` each time, then `wait(5 ms)`, until `cond` holds for
/// the latest report. Panics `"deadline"` after 30 s of real time: a failure
/// guard, never an oracle.
pub fn pump_until(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    step_ms: u64,
    what: &str,
    mut cond: impl FnMut(&QuicHost, &QuicPumpReport) -> bool,
) -> Vec<Step> {
    let start = Instant::now();
    let mut steps = Vec::new();
    loop {
        clock.advance(step_ms);
        let report = qh.pump(clock.now()).expect("pump");
        let done = cond(qh, &report);
        steps.push((clock.now(), report));
        if done {
            return steps;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: {what}");
        qh.wait(Duration::from_millis(5));
    }
}

/// Guard without pumping: polls `cond`, yielding, then parking briefly.
pub fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    let mut spins = 0u32;
    while !cond() {
        assert!(start.elapsed() < DEADLINE, "deadline: {what}");
        spins += 1;
        if spins < 2_000 {
            std::thread::yield_now();
        } else {
            std::thread::park_timeout(Duration::from_millis(1));
        }
    }
}

pub fn calls(steps: &[Step]) -> Vec<HostCall> {
    steps
        .iter()
        .flat_map(|(_, report)| report.calls.iter().cloned())
        .collect()
}

pub fn hellos(steps: &[Step]) -> Vec<HelloDecision> {
    steps
        .iter()
        .flat_map(|(_, report)| report.hellos.iter().cloned())
        .collect()
}

// ---------------------------------------------------------------------------
// The test client.
// ---------------------------------------------------------------------------

/// A real `QuicClient` plus everything it sent and received.
pub struct Client {
    pub quic: QuicClient,
    pub conn: ConnectionId,
    pub peer: PeerHandle,
    pub invitation: InvitationId,
    pub epoch: [u8; 16],
    pub protocol: ProtocolSelection,
    pub version: u8,
    /// Every frame sent, in order: frame ordinal k is `sent[k - 1]`.
    pub sent: Vec<Vec<u8>>,
    /// Every frame received, decoded, in arrival order.
    pub frames: Vec<Frame>,
    /// `(seq, status, processed_tick)` of every receipt, in arrival order.
    pub receipts: Vec<(u64, ReceiptStatus, u64)>,
    /// Set once `try_recv` reports the end of the connection.
    pub closed: Option<CloseReason>,
}

impl Client {
    /// Sends one frame (must be accepted) and logs it.
    pub fn send(&mut self, bytes: Vec<u8>) {
        self.quic.try_send(&bytes).expect("client send");
        self.sent.push(bytes);
    }

    /// Reads every frame available now; returns how many.
    pub fn read_available(&mut self) -> usize {
        let mut count = 0;
        if self.closed.is_some() {
            return 0;
        }
        loop {
            match self.quic.try_recv() {
                Ok(Some(bytes)) => {
                    let frame = decode_frame(self.protocol, &bytes);
                    for op in &frame.ops {
                        if let DeltaOp::Legacy(protocol::DeltaOp::Receipt {
                            seq,
                            processed_tick,
                            status,
                            ..
                        }) = op
                        {
                            self.receipts.push((*seq, *status, *processed_tick));
                        }
                    }
                    self.frames.push(frame);
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

    /// Guard: reads until `cond` holds.
    pub fn read_until(&mut self, what: &str, mut cond: impl FnMut(&Client) -> bool) {
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

    /// Guard: reads to the end of the connection and returns its reason.
    pub fn read_to_close(&mut self) -> CloseReason {
        self.read_until("client close", |client| client.closed.is_some());
        self.closed.expect("closed")
    }

    pub fn has_receipt(&self, seq: u64) -> bool {
        self.receipts.iter().any(|(s, _, _)| *s == seq)
    }

    pub fn replica(&self) -> Replica {
        let mut replica = Replica::new();
        replica.apply(&self.frames);
        replica
    }

    pub fn frame_bytes(&self) -> Vec<Vec<u8>> {
        self.frames
            .iter()
            .map(|frame| frame.bytes.clone())
            .collect()
    }
}

/// Runs `QuicClient::connect` on its own thread so the caller can pump.
pub fn connect_async(
    limits: ClientLimits,
    server: SocketAddr,
    version: u8,
    credential_bytes: &[u8],
) -> JoinHandle<Result<QuicClient, ConnectError>> {
    let config = client_config(limits);
    let hello = Hello {
        wire_version: version,
        credential: credential(credential_bytes),
    };
    std::thread::spawn(move || QuicClient::connect(config, server, &hello))
}

/// Pumps until one hello is decided; returns the decision and the steps.
pub fn decide_hello(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    step_ms: u64,
) -> (HelloDecision, Vec<Step>) {
    let steps = pump_until(qh, clock, step_ms, "hello decided", |_, report| {
        !report.hellos.is_empty()
    });
    let decided = hellos(&steps);
    assert_eq!(decided.len(), 1, "{decided:?}");
    (decided[0].clone(), steps)
}

/// Connects a client that the adapter accepts.
pub fn connect(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    step_ms: u64,
    server: SocketAddr,
    limits: ClientLimits,
    protocol: ProtocolSelection,
    credential_bytes: &[u8],
) -> (Client, Vec<Step>) {
    let version = version_byte(protocol);
    let pending = connect_async(limits, server, version, credential_bytes);
    let (decision, steps) = decide_hello(qh, clock, step_ms);
    let HelloOutcome::Accepted { invitation, peer } = decision.outcome else {
        panic!("expected Accepted, got {decision:?}");
    };
    let quic = pending.join().expect("client thread").expect("connect");
    let epoch = qh.host().epoch(peer).expect("bound");
    let client = Client {
        quic,
        conn: decision.conn,
        peer,
        invitation,
        epoch,
        protocol,
        version,
        sent: Vec::new(),
        frames: Vec::new(),
        receipts: Vec::new(),
        closed: None,
    };
    (client, steps)
}

/// Attempts a connection the adapter refuses; returns the client's error
/// and the adapter's decision.
pub fn connect_refused(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    limits: ClientLimits,
    version: u8,
    credential_bytes: &[u8],
) -> (ConnectError, HelloDecision, Vec<Step>) {
    let pending = connect_async(limits, qh.local_addr(), version, credential_bytes);
    let (decision, steps) = decide_hello(qh, clock, 100);
    let error = match pending.join().expect("client thread") {
        Ok(_) => panic!("expected a refusal, got a connection ({decision:?})"),
        Err(error) => error,
    };
    (error, decision, steps)
}

// ---------------------------------------------------------------------------
// Stall-case machinery (A§3).
// ---------------------------------------------------------------------------

/// Server for the stall cases (12, 14, 16): v1 except a tightened inbound
/// queue, so that the backlog behind a held frame (8 queued + 1 parked)
/// stays inside the 80-frame rate burst (T023b A§2.2). Outbound stays v1
/// egress: 128 frames / 2 MiB, the T022 per-peer log cap (A§2.1).
pub fn stall_server_limits() -> ServerLimits {
    let mut limits = ServerLimits::v1();
    limits.inbound.per_peer_frames = 8;
    limits.inbound.per_peer_bytes = 4_096; // MAX_INTENT_FRAME_BYTES, the floor
    limits
}

/// T023c C§10's non-reading observer B.
pub fn stalled_client_limits() -> ClientLimits {
    ClientLimits {
        inbound_frames: 1,
        inbound_bytes: 65_536,
        stream_window_bytes: 65_540,
        connection_window_bytes: 65_540,
        ..ClientLimits::v1()
    }
}

/// EndTurns per paced pump, and the injected step. 4 per 100 ms equals the
/// sustained 40 frames/s, so the 80-frame burst never drains (A§2.2).
pub const PACE: u64 = 4;
pub const PACE_STEP_MS: u64 = 100;
/// B's transport takes event_seq 1..=B_HANDED (initial + EndTurns 1..=2,768).
pub const B_HANDED: u64 = 2_769;

/// One c-pump (A§3.4): A sends the next `count` `EndTurn`s, a guard waits
/// (no pump) until all `count` are queued at the server, then the clock
/// advances `PACE_STEP_MS`, the adapter pumps once, and A reads every frame
/// it has.
pub fn drive_one(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    a: &mut Client,
    next_seq: &mut u64,
    count: u64,
) -> Step {
    for _ in 0..count {
        let bytes = end_turn_for(a.version, a.epoch, *next_seq);
        a.send(bytes);
        *next_seq += 1;
    }
    let conn = a.conn;
    let wanted = count as usize;
    until("A's frames queued at the server", || {
        qh.connection_stats(conn)
            .is_some_and(|stats| stats.inbound_frames == wanted)
    });
    clock.advance(PACE_STEP_MS);
    let report = qh.pump(clock.now()).expect("pump");
    a.read_available();
    (clock.now(), report)
}

/// A§3.5: B§5 phase 5's stall rule recomputed from the public trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StallMirror {
    pub since: Option<u64>,
    pub through: u64,
}

impl StallMirror {
    pub fn new() -> Self {
        Self {
            since: None,
            through: 0,
        }
    }

    /// Feeds one pump's trace for peer `b` at `now`.
    pub fn feed(&mut self, now: u64, report: &QuicPumpReport, b: PeerHandle) {
        let mut taken = None;
        let mut acked = self.through;
        for call in &report.calls {
            match call {
                HostCall::TakeDelivery {
                    peer,
                    result: Ok(n),
                } if *peer == b => taken = Some(*n as u64),
                HostCall::AcknowledgeDelivery { peer, through } if *peer == b => acked = *through,
                _ => {}
            }
        }
        let Some(n) = taken else {
            return;
        };
        let k = acked - self.through;
        self.through = acked;
        if k < n {
            self.since.get_or_insert(now);
        } else {
            self.since = None;
        }
    }
}

// Provenance (T023b B§14.1, T023 R8.3): everything below this line is copied
// verbatim from `crates/crpg-net-quic/tests/support/quic_rig.rs` (its
// "UdpRelay." section, including the monotonic-clock reorder hold that
// fixed the Windows burst release). Test-only and std-only; there is no
// shared runtime module and no testkit edge. Keep it byte-identical to the
// source; change the source first if it must change.

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
    forwarded_to_client: AtomicU64,
    dropped: AtomicU64,
    reordered: AtomicU64,
}

/// A client ↔ server UDP relay. Clients connect to `addr()`.
pub struct UdpRelay {
    addr: SocketAddr,
    counters: Arc<Counters>,
    blackhole: Arc<AtomicBool>,
    blackhole_to_server: Arc<AtomicBool>,
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
    /// When the oldest packet of the partial block arrived.
    held_since: Option<Instant>,
    counters: Arc<Counters>,
    blackhole: Arc<AtomicBool>,
    /// Blackholes this direction only.
    one_way: Arc<AtomicBool>,
}

impl Direction {
    fn offer(&mut self, packet: Vec<u8>, send: &mut dyn FnMut(&[u8])) {
        if self.blackhole.load(Ordering::SeqCst) || self.one_way.load(Ordering::SeqCst) {
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
        if self.block.is_empty() {
            self.held_since = Some(Instant::now());
        }
        self.block.push(packet);
        if self.block.len() == self.config.reorder_depth {
            self.held_since = None;
            // Block rotation: the first packet goes out last.
            self.block.rotate_left(1);
            for packet in self.block.drain(..) {
                send(&packet);
                self.counters.forwarded.fetch_add(1, Ordering::SeqCst);
            }
            self.counters.reordered.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// When the partial reorder block started being held, if one is.
    fn holding(&self) -> Option<Instant> {
        self.held_since
    }

    /// A partial block is released in order once it has been held for
    /// `HOLD` (measured on the monotonic clock, see `recv_burst`), or on the
    /// idle read timeout.
    fn release(&mut self, send: &mut dyn FnMut(&[u8])) {
        self.held_since = None;
        for packet in self.block.drain(..) {
            send(&packet);
            self.counters.forwarded.fetch_add(1, Ordering::SeqCst);
        }
    }
}

/// How long a partial reorder block may wait for the rest of its block.
const HOLD: Duration = Duration::from_millis(2);

/// Receive one datagram. While a partial reorder block is held, the socket is
/// polled non-blocking until a packet arrives or the block has been held for
/// `HOLD` on the monotonic clock. The hold therefore never depends on
/// read-timeout or sleep granularity: Windows rounds a 2 ms timeout up to its
/// ~15.6 ms tick, which inflated the relayed RTT about eightfold, while
/// releasing as soon as nothing was queued left small transfers on Windows
/// with no reordering at all. With nothing held, the read blocks for the
/// short idle timeout only so the thread can observe `stop`.
fn recv_burst(
    socket: &UdpSocket,
    buf: &mut [u8],
    held_since: Option<Instant>,
) -> std::io::Result<(usize, SocketAddr)> {
    let Some(since) = held_since else {
        socket.set_nonblocking(false)?;
        return socket.recv_from(buf);
    };
    socket.set_nonblocking(true)?;
    loop {
        match socket.recv_from(buf) {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if since.elapsed() >= HOLD {
                    return Err(error);
                }
                std::thread::yield_now();
            }
            other => return other,
        }
    }
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
        let blackhole_to_server = Arc::new(AtomicBool::new(false));
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
                held_since: None,
                counters: counters.clone(),
                blackhole: blackhole.clone(),
                one_way: blackhole_to_server.clone(),
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
                held_since: None,
                counters: counters.clone(),
                blackhole: blackhole.clone(),
                one_way: Arc::new(AtomicBool::new(false)),
            };
            let to_client = counters.clone();
            threads.push(std::thread::spawn(move || {
                let mut buf = vec![0u8; 65_536];
                let mut send = |packet: &[u8]| {
                    let target = client.lock().ok().and_then(|known| *known);
                    if let Some(target) = target {
                        let _ = front.send_to(packet, target);
                        to_client.forwarded_to_client.fetch_add(1, Ordering::SeqCst);
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
            blackhole_to_server,
            stop,
            threads,
        }
    }

    /// Server → client datagrams forwarded.
    pub fn forwarded_to_client(&self) -> u64 {
        self.counters.forwarded_to_client.load(Ordering::SeqCst)
    }

    /// Drops client → server datagrams only, so the server receives no ACKs.
    pub fn set_blackhole_to_server(&self, on: bool) {
        self.blackhole_to_server.store(on, Ordering::SeqCst);
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
