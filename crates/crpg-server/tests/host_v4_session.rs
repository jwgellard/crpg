//! T037b acceptance suite (`tasks/T037.md`, tests B1 to B6 and B9; B7 and B8
//! are the unit tests in `src/host_v4.rs` and `src/host.rs`): protocol v4
//! sessions, positions, pages and `Moved`, public API only.
//!
//! Oracles are independent of the host: a test-owned mirror `HistoryWorld`
//! loaded from the same JSON, and hand-written expected frames, rows and
//! error values. The v4 client is the T026p5 one: a `ReassemblyPoolV4` gate
//! feeding a `ReplicaWorldV4`, never the v1/v2 receiver; the v3 twin uses
//! `ReassemblyPoolV3` and `ReplicaWorldV3`. The fixtures are
//! `tests/fixtures/three_combatants.json` as saved-game JSON, edited in
//! memory (no new fixture file):
//!
//! - `MV3`: `TB3` with A/B/C movers at `m(1, 1)`, `m(3, 1)`, `m(1, 5)`, their
//!   `f32` mirrors and the definition's movement rules.
//! - `MV3rt`: the same edits on the real-time JSON.
//!
//! `P` is `bind_v4_peer([A], FULL(A,B,C), FACTS, R1, ..)` drained and
//! published unless a case says otherwise; `Q` is the same binding by
//! `bind_v3_peer`. Every refusal is compared against the host state taken
//! just before the call and sits beside an accepted control.

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_net::codec_v3;
use crpg_net::codec_v4;
use crpg_net::facts_v3::EncounterGrants;
use crpg_net::pool_v3::{ReassemblyPoolV3, ReceiverIdV3, ReplicaUpdateV3};
use crpg_net::pool_v4::{
    DeltaOutcome, GateMode, ReassemblyPoolV4, ReceiverIdV4, ReplicaUpdateV4, TransferLimitsV3,
    TransferOutcome,
};
use crpg_net::protocol::LANE_COMBAT;
use crpg_net::protocol::{IntentBody as IntentBodyV1, NetId};
use crpg_net::protocol_transfer::{
    HealthFact, ResyncReason, TransferDown, TransferId, TransferUp, TurnFact,
};
use crpg_net::protocol_v3::{
    AutoPauseToggles, DeltaFrame as DeltaFrameV3, DeltaOp as DeltaOpV3, FactSet, HostFacts,
    IntentBody as IntentBodyV3, IntentFrame as IntentFrameV3, OrderFact, PauseRights, PoolFact,
    ReceiptStatus as ReceiptV3, StatFact, StatValue,
};
use crpg_net::protocol_v4::{
    DeltaFrame, DeltaOp, IntentBody as IntentBodyV4, IntentFrame as IntentFrameV4, Point,
    PositionFact, SessionEpoch,
};
use crpg_net::replica_v3::{ReplicaEntityV3, ReplicaWorldV3};
use crpg_net::replica_v4::{PositionState, ReplicaWorldV4};
use crpg_net::transfer_v3;
use crpg_server::capture::{CaptureEntry, CapturedRecord};
use crpg_server::host::{
    ControlGrant, DeliveryEntry, DeliveryKind, DisclosureGrants, EntityDisclosure, EntityFactGrant,
    FactGrants, GrantUpdate, Host, HostConfig, HostError, IngestDisposition, PeerHandle,
    ProtocolSelection, TransferDisposition, TransferState, DEFAULT_HOST_GRANTS,
};
use crpg_sim::EncounterMode;
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");

// ---------------------------------------------------------------------------
// Identities, geometry and fixtures.
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
fn smite() -> Ulid {
    Ulid::from_u128(604)
}
fn whiff() -> Ulid {
    Ulid::from_u128(605)
}
fn pool_jy() -> Ulid {
    Ulid::from_u128(606)
}
fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}
fn tid(raw: u64) -> TransferId {
    TransferId::new(raw).expect("nonzero")
}
/// A raw wire point `[x, y, z]`.
fn p(x: i32, y: i32, z: i32) -> Point {
    Point::new([x, y, z]).expect("within the wire bound")
}
/// A raw sim point `[x, y, z]`.
fn fp(x: i32, y: i32, z: i32) -> [Fx16_16; 3] {
    [
        Fx16_16::from_raw(x),
        Fx16_16::from_raw(y),
        Fx16_16::from_raw(z),
    ]
}
/// A whole-metre floor point `[x, 0, z]`.
fn m(x: i32, z: i32) -> [Fx16_16; 3] {
    [Fx16_16::from_int(x), Fx16_16::ZERO, Fx16_16::from_int(z)]
}

fn tb3_value() -> Value {
    serde_json::from_str(FIXTURE).expect("fixture parses")
}

fn tb3c_value() -> Value {
    let mut value = tb3_value();
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

/// The movement edits of `MV3`/`MV3rt` (T037a's fixture, unchanged).
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

fn mv3_value() -> Value {
    movement_edits(tb3_value())
}

#[allow(dead_code)]
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

fn config() -> HostConfig {
    HostConfig {
        protocol: ProtocolSelection::V2,
        incarnation: 1,
    }
}

fn host_of(value: &Value) -> Host {
    Host::from_history(history_of(value), config(), 0).expect("wraps")
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

/// `FULL(es)`: every flag true for each entity in `es`.
fn full_grants(entities: &[EntityId]) -> DisclosureGrants {
    DisclosureGrants {
        entities: entities.iter().copied().map(disclosure).collect(),
        abilities: vec![strike(), smite(), whiff()],
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

/// `FACTS`: A, B, C all four flags, every encounter grant, default host grants.
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

/// `FACTS` with A's pools withheld (B2's one fact change).
fn facts_without_a_pools() -> FactGrants {
    let mut facts = facts();
    facts.entities[0].pools = false;
    facts
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

fn toggles(bits: u8) -> AutoPauseToggles {
    AutoPauseToggles::from_bits(bits).expect("defined bits")
}

/// `P`: the v4 session under test, controlling `[A]`.
fn bind_p(host: &mut Host, now: u64) -> PeerHandle {
    host.bind_v4_peer(
        control(&[a()]),
        full_grants(&[a(), b(), c()]),
        facts(),
        r1(),
        now,
    )
    .expect("binds")
}

/// `Q`: the same binding by `bind_v3_peer` (the byte-for-byte control).
fn bind_q(host: &mut Host, now: u64) -> PeerHandle {
    host.bind_v3_peer(
        control(&[a()]),
        full_grants(&[a(), b(), c()]),
        facts(),
        r1(),
        now,
    )
    .expect("binds")
}

fn assert_authority(host: &Host, oracle: &HistoryWorld) {
    assert_eq!(host.authority_hash(), crpg_sim::history_hash(oracle));
}

// ---------------------------------------------------------------------------
// Expected ops (hand-written, never read back from the host).
// ---------------------------------------------------------------------------

fn pos(entity: u64, position: Option<(Point, bool)>) -> DeltaOp {
    DeltaOp::Position {
        entity: n(entity),
        position: position.map(|(point, moving)| PositionFact { point, moving }),
    }
}

fn clock(paused: Option<crpg_net::protocol_v3::PauseReason>) -> DeltaOpV3 {
    DeltaOpV3::Clock { paused }
}

fn host_facts(rights: PauseRights) -> DeltaOpV3 {
    DeltaOpV3::HostFacts(HostFacts {
        rights,
        auto_pause: Some(toggles(0)),
        turn_timer_ms: Some(None),
    })
}

/// The fact set of one fixture row under `FACTS`: the action pool at
/// `pools`, `order` as given, `alpha` 8, no effects and no tags.
fn fs(pools: Option<(u32, u32)>, order: Option<Option<OrderFact>>) -> FactSet {
    FactSet {
        pools: pools.map(|(current, max)| {
            vec![PoolFact {
                pool: pool_jy(),
                current,
                max,
            }]
        }),
        effects: Some(vec![]),
        order,
        stats: Some(vec![StatFact {
            name: "alpha".to_owned(),
            value: StatValue::Int(8),
        }]),
        tags: Some(vec![]),
    }
}

fn facts_op(entity: u64, facts: FactSet) -> DeltaOp {
    DeltaOp::V3(DeltaOpV3::EntityFacts {
        entity: n(entity),
        facts,
    })
}

fn row(id: u64, health: HealthFact, facts: FactSet) -> ReplicaEntityV3 {
    ReplicaEntityV3 {
        net_id: n(id),
        health: Some(health),
        facts,
    }
}

fn position(id: u64, raw: [i32; 3], moving: bool) -> (NetId, PositionState) {
    (
        n(id),
        PositionState {
            tick: 0,
            point: Point::new(raw).expect("within the wire bound"),
            moving,
        },
    )
}

// ---------------------------------------------------------------------------
// The v4 client (T026p5).
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Transfer(TransferOutcome),
    Delta(DeltaOutcome),
}

struct Drained {
    entries: Vec<DeliveryEntry>,
    outcomes: Vec<Outcome>,
    deltas: Vec<DeltaFrame>,
}

struct Client {
    epoch: [u8; 16],
    pool: ReassemblyPoolV4,
    rid: ReceiverIdV4,
    replica: ReplicaWorldV4,
    last_snapshot: Option<TransferId>,
}

impl Client {
    fn new(epoch: [u8; 16]) -> Self {
        let mut pool = ReassemblyPoolV4::new(TransferLimitsV3::v3()).expect("v3 limits");
        let rid = pool
            .add_receiver(epoch, GateMode::AwaitSnapshot)
            .expect("receiver");
        Self {
            epoch,
            pool,
            rid,
            replica: ReplicaWorldV4::new(SessionEpoch(epoch)),
            last_snapshot: None,
        }
    }

    fn drain(&mut self, host: &mut Host, peer: PeerHandle, now: u64) -> Drained {
        let entries = host.take_delivery_entries(peer).expect("bound");
        let mut outcomes = Vec::new();
        for entry in &entries {
            outcomes.push(match entry.kind {
                DeliveryKind::Transfer => Outcome::Transfer(
                    self.pool
                        .offer_transfer(self.rid, &entry.bytes, now)
                        .expect("the gate accepts the frame"),
                ),
                DeliveryKind::Delta => Outcome::Delta(
                    self.pool
                        .offer_delta(self.rid, &entry.bytes, now)
                        .expect("the gate accepts the frame"),
                ),
            });
        }
        if let Some(last) = entries.last() {
            host.acknowledge_delivery(peer, last.ordinal).expect("acks");
        }
        let mut deltas = Vec::new();
        while let Some(update) = self.pool.pop_update(self.rid) {
            match update {
                ReplicaUpdateV4::Snapshot(publication) => {
                    self.last_snapshot = Some(publication.transfer);
                    self.replica.apply_snapshot(&publication).expect("applies");
                }
                ReplicaUpdateV4::Delta(frame) => {
                    self.replica.apply_delta(&frame).expect("applies");
                    deltas.push(frame);
                }
            }
        }
        Drained {
            entries,
            outcomes,
            deltas,
        }
    }

    fn publish(&self, host: &mut Host, peer: PeerHandle, now: u64) -> TransferDisposition {
        let bytes = transfer_v3::encode_up(&TransferUp::Published {
            epoch: self.epoch,
            transfer: self.last_snapshot.expect("a snapshot was published"),
        })
        .expect("encodes");
        host.ingest_transfer(peer, &bytes, now).expect("open")
    }
}

/// The v3 twin: the same gate and replica as T141's suite.
struct Twin {
    epoch: [u8; 16],
    pool: ReassemblyPoolV3,
    rid: ReceiverIdV3,
    replica: ReplicaWorldV3,
    last_snapshot: Option<TransferId>,
}

impl Twin {
    fn new(epoch: [u8; 16]) -> Self {
        let mut pool = ReassemblyPoolV3::new(TransferLimitsV3::v3()).expect("v3 limits");
        let rid = pool
            .add_receiver(epoch, GateMode::AwaitSnapshot)
            .expect("receiver");
        Self {
            epoch,
            pool,
            rid,
            replica: ReplicaWorldV3::new(SessionEpoch(epoch)),
            last_snapshot: None,
        }
    }

    fn drain(&mut self, host: &mut Host, peer: PeerHandle, now: u64) -> Vec<DeliveryEntry> {
        let entries = host.take_delivery_entries(peer).expect("bound");
        for entry in &entries {
            match entry.kind {
                DeliveryKind::Transfer => {
                    self.pool
                        .offer_transfer(self.rid, &entry.bytes, now)
                        .expect("the gate accepts the frame");
                }
                DeliveryKind::Delta => {
                    self.pool
                        .offer_delta(self.rid, &entry.bytes, now)
                        .expect("the gate accepts the frame");
                }
            }
        }
        if let Some(last) = entries.last() {
            host.acknowledge_delivery(peer, last.ordinal).expect("acks");
        }
        while let Some(update) = self.pool.pop_update(self.rid) {
            match update {
                ReplicaUpdateV3::Snapshot(publication) => {
                    self.last_snapshot = Some(publication.transfer);
                    self.replica.apply_snapshot(&publication).expect("applies");
                }
                ReplicaUpdateV3::Delta(frame) => {
                    self.replica.apply_delta(&frame).expect("applies");
                }
            }
        }
        entries
    }

    fn publish(&mut self, host: &mut Host, peer: PeerHandle, now: u64) -> TransferDisposition {
        let bytes = transfer_v3::encode_up(&TransferUp::Published {
            epoch: self.epoch,
            transfer: self.last_snapshot.expect("a snapshot was published"),
        })
        .expect("encodes");
        host.ingest_transfer(peer, &bytes, now).expect("open")
    }
}

fn decoded_transfer(entry: &DeliveryEntry) -> TransferDown {
    transfer_v3::decode_down(&entry.bytes).expect("a 0x82 frame")
}

/// The ordinal and kind of each entry.
fn shape(entries: &[DeliveryEntry]) -> Vec<(u64, DeliveryKind)> {
    entries
        .iter()
        .map(|entry| (entry.ordinal, entry.kind))
        .collect()
}

/// Takes and acknowledges every pending entry of `peer`.
fn take(host: &mut Host, peer: PeerHandle) -> Vec<DeliveryEntry> {
    let entries = host.take_delivery_entries(peer).expect("bound");
    if let Some(last) = entries.last() {
        host.acknowledge_delivery(peer, last.ordinal).expect("acks");
    }
    entries
}

fn entry_ops(entry: &DeliveryEntry) -> Vec<DeltaOp> {
    codec_v4::decode_delta(&entry.bytes)
        .expect("a v4 delta")
        .ops
}

/// The ops of the one delta frame among `entries`.
fn delta_ops(entries: &[DeliveryEntry]) -> Vec<DeltaOp> {
    let deltas: Vec<&DeliveryEntry> = entries
        .iter()
        .filter(|entry| entry.kind == DeliveryKind::Delta)
        .collect();
    assert_eq!(deltas.len(), 1, "exactly one delta: {entries:?}");
    entry_ops(deltas[0])
}

/// Drains (and publishes) a fresh v4 binding, so its transfer is retired.
fn drained_v4(host: &mut Host, peer: PeerHandle, now: u64) -> Client {
    let mut client = Client::new(host.epoch(peer).expect("bound"));
    client.drain(host, peer, now);
    assert_eq!(
        client.publish(host, peer, now),
        TransferDisposition::Published
    );
    client
}

/// Drains (and publishes) a fresh v3 binding.
fn drained_v3(host: &mut Host, peer: PeerHandle, now: u64) -> Twin {
    let mut twin = Twin::new(host.epoch(peer).expect("bound"));
    twin.drain(host, peer, now);
    assert_eq!(
        twin.publish(host, peer, now),
        TransferDisposition::Published
    );
    twin
}

// ---------------------------------------------------------------------------
// State for refusals.
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
struct State {
    authority: [u8; 32],
    last_capture: u64,
    executions: u64,
    peers: Vec<(Option<TransferState>, Vec<DeliveryEntry>)>,
}

fn state(host: &mut Host, peers: &[PeerHandle]) -> State {
    let peers = peers
        .iter()
        .map(|peer| {
            (
                host.transfer_state(*peer).expect("bound"),
                host.take_delivery_entries(*peer).expect("bound"),
            )
        })
        .collect();
    State {
        authority: host.authority_hash(),
        last_capture: host.last_capture(),
        executions: host.executions(),
        peers,
    }
}

fn assert_unchanged(host: &mut Host, before: &State, peers: &[PeerHandle]) {
    assert_eq!(host.authority_hash(), before.authority);
    assert_eq!(host.last_capture(), before.last_capture);
    assert_eq!(host.executions(), before.executions);
    for (peer, (_, entries)) in peers.iter().zip(before.peers.iter()) {
        assert_eq!(host.take_delivery_entries(*peer), Ok(entries.clone()));
    }
}

// ---------------------------------------------------------------------------
// Intents.
// ---------------------------------------------------------------------------

fn v3_intent(host: &Host, peer: PeerHandle, seq: u64, actor: u64, body: IntentBodyV3) -> Vec<u8> {
    codec_v3::encode_intent(&IntentFrameV3 {
        epoch: host.epoch(peer).expect("bound"),
        lane: LANE_COMBAT,
        seq,
        observed_tick: host.server_tick(),
        actor: n(actor),
        body,
    })
    .expect("encodes")
}

#[allow(dead_code)]
fn v4_intent(host: &Host, peer: PeerHandle, seq: u64, actor: u64, body: IntentBodyV4) -> Vec<u8> {
    codec_v4::encode_intent(&IntentFrameV4 {
        epoch: host.epoch(peer).expect("bound"),
        lane: LANE_COMBAT,
        seq,
        observed_tick: host.server_tick(),
        actor: n(actor),
        body,
    })
    .expect("encodes")
}

/// One v3 intent through `ingest` then `pump`.
fn send_v3(
    host: &mut Host,
    peer: PeerHandle,
    seq: u64,
    actor: u64,
    body: IntentBodyV3,
    now: u64,
) -> crpg_server::host::AdmissionResult {
    let bytes = v3_intent(host, peer, seq, actor, body);
    assert_eq!(
        host.ingest(peer, &bytes, now),
        Ok(IngestDisposition::Staged)
    );
    let summary = host.pump(now).expect("pump");
    assert_eq!(summary.results.len(), 1, "{summary:?}");
    summary.results[0].clone()
}

fn resync_bytes(epoch: [u8; 16]) -> Vec<u8> {
    transfer_v3::encode_up(&TransferUp::Resync {
        epoch,
        after: None,
        reason: ResyncReason::Expired,
    })
    .expect("encodes")
}

// ---------------------------------------------------------------------------
// B1.
// ---------------------------------------------------------------------------

/// B1: a v4 binding's first delivery is a `0x82` snapshot at cursor 0
/// followed by the anchoring page whose ops are the three positions then the
/// v3 `[Clock, HostFacts]` pair, every one carried by v4; a v3 binding of the
/// same arguments has the same entries without the positions.
#[test]
fn b1_bind_v4_first_delivery() {
    let mut host = host_of(&mv3_value());
    let oracle = history_of(&mv3_value());
    let p = bind_p(&mut host, 100);
    assert_eq!(p.get(), 1);
    assert_eq!(host.session_wire(p), Ok(4));
    assert_eq!(
        host.transfer_state(p),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: Some(1),
        }))
    );
    let epoch = host.epoch(p).expect("bound");
    let mut client = Client::new(epoch);
    let drained = client.drain(&mut host, p, 100);
    assert_eq!(
        shape(&drained.entries),
        vec![
            (1, DeliveryKind::Transfer),
            (2, DeliveryKind::Transfer),
            (3, DeliveryKind::Delta),
        ]
    );
    assert_eq!(
        drained.outcomes,
        vec![
            Outcome::Transfer(TransferOutcome::Begun),
            Outcome::Transfer(TransferOutcome::Published {
                transfer: tid(1),
                released: 0,
            }),
            Outcome::Delta(DeltaOutcome::Released { count: 1 }),
        ]
    );
    let TransferDown::Begin(begin) = decoded_transfer(&drained.entries[0]) else {
        panic!("the first entry is a Begin");
    };
    assert_eq!(begin.epoch, epoch);
    assert_eq!(begin.transfer, tid(1));
    assert_eq!(begin.cursor, 0);
    assert_eq!(begin.entity_count, 3);
    assert_eq!(
        drained.deltas,
        vec![DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 0,
            event_seq: 1,
            ops: vec![
                pos(1, Some((p(65536, 0, 65536), false))),
                pos(2, Some((p(196608, 0, 65536), false))),
                pos(3, Some((p(65536, 0, 327680), false))),
                DeltaOp::V3(clock(None)),
                DeltaOp::V3(host_facts(r1())),
            ],
        }]
    );
    // The v3 replica behind the v4 one is T141 C1's.
    let replica = client.replica.v3();
    assert_eq!(
        replica.entities(),
        &[
            row(
                1,
                HealthFact {
                    health: 10,
                    max_health: 10,
                    dead: false,
                },
                fs(Some((1, 1)), Some(None)),
            ),
            row(
                2,
                HealthFact {
                    health: 10,
                    max_health: 10,
                    dead: false,
                },
                fs(Some((1, 1)), None),
            ),
            row(
                3,
                HealthFact {
                    health: 3,
                    max_health: 3,
                    dead: false,
                },
                fs(Some((1, 1)), None),
            ),
        ]
    );
    assert_eq!(
        replica.turn(),
        Some(&TurnFact {
            active: Some(n(1)),
            round: 0,
        })
    );
    assert_eq!(
        replica.host_facts(),
        Some(&HostFacts {
            rights: r1(),
            auto_pause: Some(toggles(0)),
            turn_timer_ms: Some(None),
        })
    );
    assert_eq!(replica.next_event_seq(), 2);
    assert_eq!(replica.server_tick(), 0);
    assert_eq!(
        client.replica.positions().clone(),
        [
            position(1, [65536, 0, 65536], false),
            position(2, [196608, 0, 65536], false),
            position(3, [65536, 0, 327680], false),
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(
        client.publish(&mut host, p, 100),
        TransferDisposition::Published
    );
    assert_eq!(
        host.transfer_state(p),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: None,
        }))
    );

    // Control: the same binding by `bind_v3_peer` gives the same entries,
    // with the three `Position` ops absent and every op unwrapped.
    let q = bind_q(&mut host, 101);
    let mut twin = Twin::new(host.epoch(q).expect("bound"));
    let entries = twin.drain(&mut host, q, 101);
    assert_eq!(
        shape(&entries),
        vec![
            (1, DeliveryKind::Transfer),
            (2, DeliveryKind::Transfer),
            (3, DeliveryKind::Delta),
        ]
    );
    assert_eq!(
        decoded_transfer(&entries[0]),
        decoded_transfer(&drained.entries[0])
    );
    assert_eq!(
        decoded_transfer(&entries[1]),
        decoded_transfer(&drained.entries[1])
    );
    assert_eq!(
        codec_v3::decode_delta(&entries[2].bytes).expect("a v3 delta"),
        DeltaFrameV3 {
            lane: LANE_COMBAT,
            server_tick: 0,
            event_seq: 1,
            ops: vec![clock(None), host_facts(r1())],
        }
    );
    assert_eq!(
        twin.publish(&mut host, q, 101),
        TransferDisposition::Published
    );
    assert_eq!(twin.replica, *client.replica.v3());
    assert_authority(&host, &oracle);
}

// ---------------------------------------------------------------------------
// B2.
// ---------------------------------------------------------------------------

/// B-3: `v3_entries` and `v4_entries` are the same frames, the v4 deltas with
/// their `Position` ops removed and every op wrapped.
fn assert_v4_wraps_v3(v3_entries: &[DeliveryEntry], v4_entries: &[DeliveryEntry]) {
    assert_eq!(
        shape(v3_entries),
        shape(v4_entries),
        "the same entries: {v3_entries:?} vs {v4_entries:?}"
    );
    for (v3_entry, v4_entry) in v3_entries.iter().zip(v4_entries.iter()) {
        match v3_entry.kind {
            DeliveryKind::Transfer => assert_eq!(v3_entry.bytes, v4_entry.bytes),
            DeliveryKind::Delta => {
                let v3_frame = codec_v3::decode_delta(&v3_entry.bytes).expect("a v3 delta");
                let v4_frame = codec_v4::decode_delta(&v4_entry.bytes).expect("a v4 delta");
                assert_eq!(v3_frame.server_tick, v4_frame.server_tick);
                assert_eq!(v3_frame.event_seq, v4_frame.event_seq);
                let unwrapped: Vec<DeltaOpV3> = v4_frame
                    .ops
                    .iter()
                    .filter(|op| !matches!(op, DeltaOp::Position { .. }))
                    .map(|op| match op {
                        DeltaOp::V3(inner) => inner.clone(),
                        other => panic!("a v4 page op that is not v3: {other:?}"),
                    })
                    .collect();
                assert_eq!(v3_frame.ops, unwrapped);
            }
        }
    }
}

/// Two hosts from the same JSON, one v3 session and one v4 session, driven in
/// lockstep.
struct Pair {
    x: Host,
    q: PeerHandle,
    cx: Twin,
    y: Host,
    p: PeerHandle,
    cy: Client,
    now: u64,
}

impl Pair {
    fn new() -> Self {
        let mut x = host_of(&mv3_value());
        let mut y = host_of(&mv3_value());
        let q = x
            .bind_v3_peer(
                control(&[a(), b(), c()]),
                full_grants(&[a(), b(), c()]),
                facts(),
                r1(),
                0,
            )
            .expect("binds");
        let p = y
            .bind_v4_peer(
                control(&[a(), b(), c()]),
                full_grants(&[a(), b(), c()]),
                facts(),
                r1(),
                0,
            )
            .expect("binds");
        let mut cx = Twin::new(x.epoch(q).expect("bound"));
        let mut cy = Client::new(y.epoch(p).expect("bound"));
        cx.drain(&mut x, q, 0);
        cy.drain(&mut y, p, 0);
        assert_eq!(cx.publish(&mut x, q, 0), TransferDisposition::Published);
        assert_eq!(cy.publish(&mut y, p, 0), TransferDisposition::Published);
        Self {
            x,
            q,
            cx,
            y,
            p,
            cy,
            now: 0,
        }
    }

    /// Drains both sessions, compares (B-3) and the replicas, then publishes.
    fn sync(&mut self) {
        let v3 = self.cx.drain(&mut self.x, self.q, self.now);
        let v4 = self.cy.drain(&mut self.y, self.p, self.now);
        assert_v4_wraps_v3(&v3, &v4.entries);
        assert_eq!(&self.cx.replica, self.cy.replica.v3());
        assert_eq!(
            self.cx.publish(&mut self.x, self.q, self.now),
            TransferDisposition::Published
        );
        assert_eq!(
            self.cy.publish(&mut self.y, self.p, self.now),
            TransferDisposition::Published
        );
    }

    /// One intent at the next tick, sent to both sessions; the statuses agree.
    fn send(&mut self, seq: u64, actor: u64, body: IntentBodyV3) {
        self.now += 10;
        let status = send_v3(&mut self.x, self.q, seq, actor, body.clone(), self.now);
        let bytes = v3_intent(&self.y, self.p, seq, actor, body);
        assert_eq!(
            self.y.ingest(self.p, &bytes, self.now),
            Ok(IngestDisposition::Staged)
        );
        let summary = self.y.pump(self.now).expect("pump");
        assert_eq!(summary.results.len(), 1);
        assert_eq!(summary.results[0].status, status.status);
        self.sync();
    }
}

/// B2: the same script on a v3 and a v4 session; after every step B-3 holds,
/// the retry-cache statuses agree and the v3 replica behind the v4 one equals
/// the v3 replica.
#[test]
fn b2_v4_wraps_v3() {
    let mut pair = Pair::new();
    pair.sync();
    pair.send(
        1,
        1,
        IntentBodyV3::DeclareAction {
            ability: strike(),
            target: n(2),
        },
    );
    assert_eq!(
        pair.x.cached_status_v3(pair.q, 1),
        Ok(Some(ReceiptV3::Applied))
    );
    assert_eq!(
        pair.y.cached_status_v3(pair.p, 1),
        Ok(Some(ReceiptV3::Applied))
    );
    pair.send(2, 1, IntentBodyV3::EndTurn);
    assert_eq!(
        pair.x.cached_status_v3(pair.q, 2),
        Ok(Some(ReceiptV3::Rejected(
            crpg_net::protocol_v3::RejectionCode::IllegalAction
        )))
    );
    assert_eq!(
        pair.x.cached_status_v3(pair.q, 2),
        pair.y.cached_status_v3(pair.p, 2)
    );
    // A fact change withholds A's pools from both sessions.
    pair.now += 10;
    assert_eq!(
        pair.x.set_fact_grants(pair.q, facts_without_a_pools()),
        Ok(())
    );
    assert_eq!(
        pair.y.set_fact_grants(pair.p, facts_without_a_pools()),
        Ok(())
    );
    pair.sync();
    // Pause, unpause and a tick publish a `Clock` page.
    pair.now += 10;
    assert_eq!(pair.x.pause(pair.now), Ok(()));
    assert_eq!(pair.y.pause(pair.now), Ok(()));
    pair.sync();
    pair.now += 10;
    assert_eq!(pair.x.unpause(pair.now), Ok(()));
    assert_eq!(pair.y.unpause(pair.now), Ok(()));
    pair.sync();
    assert_eq!(pair.x.tick(), Ok(()));
    assert_eq!(pair.y.tick(), Ok(()));
    pair.sync();
    // A narrowing restages both sessions under the new grants.
    pair.now += 10;
    assert_eq!(
        pair.x.set_grants(pair.q, full_grants(&[a(), b()])),
        Ok(GrantUpdate::SnapshotStaged)
    );
    assert_eq!(
        pair.y.set_grants(pair.p, full_grants(&[a(), b()])),
        Ok(GrantUpdate::SnapshotStaged)
    );
    pair.sync();
    assert_authority(&pair.x, &history_of(&mv3_value()));
    assert_authority(&pair.y, &history_of(&mv3_value()));
}

// ---------------------------------------------------------------------------
// B3.
// ---------------------------------------------------------------------------

/// B3: revoking a position after disclosure withdraws it and stops the
/// anchors, restoring it re-anchors it, and an unchanged list sends nothing; a
/// resync under the withheld list anchors only the granted entities. The v3
/// session beside it receives nothing.
#[test]
fn b3_withhold_after_disclosure() {
    let mut host = host_of(&mv3_value());
    let p = bind_p(&mut host, 0);
    let mut client = drained_v4(&mut host, p, 0);
    let q = bind_q(&mut host, 1);
    let mut twin = drained_v3(&mut host, q, 1);
    assert_eq!(client.replica.positions().len(), 3);

    // Withhold B: exactly one Position withdrawal.
    assert_eq!(host.set_position_withheld(p, vec![b()]), Ok(()));
    let drained = client.drain(&mut host, p, 2);
    assert_eq!(
        drained.deltas,
        vec![DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 0,
            event_seq: 2,
            ops: vec![pos(2, None)],
        }]
    );
    assert_eq!(client.replica.position(n(2)), None);
    assert!(client.replica.position(n(1)).is_some());
    assert!(client.replica.position(n(3)).is_some());
    assert!(twin.drain(&mut host, q, 2).is_empty());

    // The same list again: no page, no frame.
    assert_eq!(host.set_position_withheld(p, vec![b()]), Ok(()));
    assert!(client.drain(&mut host, p, 3).entries.is_empty());
    assert!(twin.drain(&mut host, q, 3).is_empty());

    // Clearing the list re-anchors B.
    assert_eq!(host.set_position_withheld(p, Vec::new()), Ok(()));
    let drained = client.drain(&mut host, p, 4);
    assert_eq!(
        drained.deltas,
        vec![DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 0,
            event_seq: 3,
            ops: vec![pos(2, Some((p(196608, 0, 65536), false)))],
        }]
    );
    assert!(twin.drain(&mut host, q, 4).is_empty());

    // A resync under `[B]` withheld anchors only n(1) and n(3).
    assert_eq!(host.set_position_withheld(p, vec![b()]), Ok(()));
    client.drain(&mut host, p, 5);
    let epoch = host.epoch(p).expect("bound");
    assert_eq!(
        host.ingest_transfer(p, &resync_bytes(epoch), 6),
        Ok(TransferDisposition::Resynced)
    );
    let mut fresh = Client::new(epoch);
    let drained = fresh.drain(&mut host, p, 6);
    assert_eq!(
        shape(&drained.entries),
        vec![
            (6, DeliveryKind::Transfer),
            (7, DeliveryKind::Transfer),
            (8, DeliveryKind::Delta),
        ]
    );
    let TransferDown::Begin(begin) = decoded_transfer(&drained.entries[0]) else {
        panic!("the first entry is a Begin");
    };
    assert_eq!(begin.transfer, tid(2));
    assert_eq!(
        entry_ops(&drained.entries[2]),
        vec![
            pos(1, Some((p(65536, 0, 65536), false))),
            pos(3, Some((p(65536, 0, 327680), false))),
            DeltaOp::V3(clock(None)),
            DeltaOp::V3(host_facts(r1())),
        ]
    );
    assert_eq!(fresh.replica.position(n(2)), None);
    assert!(twin.drain(&mut host, q, 6).is_empty());
    assert_authority(&host, &history_of(&mv3_value()));
}

// ---------------------------------------------------------------------------
// B4.
// ---------------------------------------------------------------------------

/// B4: a presence narrowing restages and anchors only the still-granted
/// entities; the same grants again are `Unchanged` and send no `Position` op.
#[test]
fn b4_presence_narrowing_reanchors() {
    let mut host = host_of(&mv3_value());
    let p = bind_p(&mut host, 0);
    let mut client = drained_v4(&mut host, p, 0);
    assert_eq!(client.replica.positions().len(), 3);

    assert_eq!(
        host.set_grants(p, full_grants(&[a(), c()])),
        Ok(GrantUpdate::SnapshotStaged)
    );
    let drained = client.drain(&mut host, p, 1);
    assert_eq!(
        shape(&drained.entries),
        vec![
            (3, DeliveryKind::Transfer),
            (4, DeliveryKind::Transfer),
            (5, DeliveryKind::Delta),
        ]
    );
    let TransferDown::Begin(begin) = decoded_transfer(&drained.entries[0]) else {
        panic!("the first entry is a Begin");
    };
    assert_eq!(begin.transfer, tid(2));
    assert!(matches!(
        decoded_transfer(&drained.entries[1]),
        TransferDown::Chunk(_)
    ));
    assert_eq!(
        entry_ops(&drained.entries[2]),
        vec![
            pos(1, Some((p(65536, 0, 65536), false))),
            pos(3, Some((p(65536, 0, 327680), false))),
            DeltaOp::V3(clock(None)),
            DeltaOp::V3(host_facts(r1())),
        ]
    );
    assert_eq!(
        client
            .replica
            .positions()
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![n(1), n(3)]
    );

    // Control: unchanged grants are `Unchanged` and send no `Position` op.
    let mut other = host_of(&mv3_value());
    let r = bind_p(&mut other, 0);
    let mut other_client = drained_v4(&mut other, r, 0);
    assert_eq!(
        other.set_grants(r, full_grants(&[a(), b(), c()])),
        Ok(GrantUpdate::Unchanged)
    );
    assert!(other_client.drain(&mut other, r, 1).entries.is_empty());
    assert_authority(&host, &history_of(&mv3_value()));
}

// ---------------------------------------------------------------------------
// B5.
// ---------------------------------------------------------------------------

/// B5: `Moved` follows the position grant — a viewer with it gets the notice
/// and its fact row, a viewer without it gets the fact row only, and the v3
/// session beside them never gets `Moved`.
#[test]
fn b5_moved_follows_the_position_grant() {
    let row = fs(Some((0, 1)), Some(None));
    let mut host = host_of(&mv3_value());
    assert_eq!(
        host.bind_area_bounds(fp(0, 0, 0), fp(655360, 655360, 655360)),
        Ok(crpg_server::host::NavBinding {
            revision: crpg_sim::NavRevision::of_bytes(
                &crpg_nav::bake_box(
                    Fx16_16::ZERO,
                    crpg_nav::NavRect {
                        min_x: Fx16_16::ZERO,
                        min_z: Fx16_16::ZERO,
                        max_x: Fx16_16::from_int(10),
                        max_z: Fx16_16::from_int(10),
                    },
                    &[],
                )
                .expect("bakes")
                .to_canonical_bytes()
            ),
            cleared: Vec::new(),
            placed: Vec::new(),
        })
    );
    let p = bind_p(&mut host, 0);
    let mut client = drained_v4(&mut host, p, 0);
    let q = host
        .bind_v3_peer(
            control(&[a(), b(), c()]),
            full_grants(&[a(), b(), c()]),
            facts(),
            r1(),
            1,
        )
        .expect("binds");
    let mut twin = drained_v3(&mut host, q, 1);

    assert_eq!(
        host.host_move(a(), m(4, 1)),
        Ok(crpg_sim::MoveOutcome {
            actor: a(),
            cost: 1,
            from: m(1, 1),
            to: m(4, 1),
        })
    );
    let drained = client.drain(&mut host, p, 2);
    assert_eq!(
        drained.deltas,
        vec![DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 0,
            event_seq: 2,
            ops: vec![
                DeltaOp::Moved {
                    entity: n(1),
                    points: vec![p(65536, 0, 65536), p(262144, 0, 65536)],
                },
                facts_op(1, row.clone()),
            ],
        }]
    );
    let entries = twin.drain(&mut host, q, 2);
    assert_eq!(
        codec_v3::decode_delta(&entries[0].bytes).expect("a v3 delta"),
        DeltaFrameV3 {
            lane: LANE_COMBAT,
            server_tick: 0,
            event_seq: 2,
            ops: vec![DeltaOpV3::EntityFacts {
                entity: n(1),
                facts: row.clone(),
            }],
        }
    );

    // Control withheld: the same move, no `Moved`.
    let mut other = host_of(&mv3_value());
    other
        .bind_area_bounds(fp(0, 0, 0), fp(655360, 655360, 655360))
        .expect("binds the mesh");
    let r = bind_p(&mut other, 0);
    let mut other_client = drained_v4(&mut other, r, 0);
    assert_eq!(other.set_position_withheld(r, vec![a()]), Ok(()));
    let first = other_client.drain(&mut other, r, 1);
    assert_eq!(delta_ops(&first.entries), vec![pos(1, None)]);
    assert_eq!(other.host_move(a(), m(4, 1)).map(|o| o.cost), Ok(1));
    let drained = other_client.drain(&mut other, r, 2);
    assert_eq!(
        drained.deltas,
        vec![DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 0,
            event_seq: 3,
            ops: vec![facts_op(1, row.clone())],
        }]
    );
}

// ---------------------------------------------------------------------------
// B6.
// ---------------------------------------------------------------------------

/// B6: `set_position_withheld`'s precedence and bounds, and a v4 binding's
/// time and peer ceiling; `cached_status_v3` accepts a v4 session.
#[test]
fn b6_errors_and_precedence() {
    let mut host = host_of(&mv3_value());
    let p = bind_p(&mut host, 0);
    let q = bind_q(&mut host, 0);
    let ghost = bind_p(&mut host, 0);
    host.unbind_peer(ghost);
    let peers = [p, q];
    let before = state(&mut host, &peers);

    // Unknown handle, then a v3 session, then a duplicate, then the bound.
    assert_eq!(
        host.set_position_withheld(ghost, vec![b()]),
        Err(HostError::UnknownPeer)
    );
    assert_unchanged(&mut host, &before, &peers);
    assert_eq!(
        host.set_position_withheld(q, vec![b()]),
        Err(HostError::NotV4Peer)
    );
    assert_unchanged(&mut host, &before, &peers);
    assert_eq!(
        host.set_position_withheld(p, vec![b(), b()]),
        Err(HostError::InvalidGrants)
    );
    assert_unchanged(&mut host, &before, &peers);
    let oversized: Vec<EntityId> = (0..1025).map(eid).collect();
    assert_eq!(
        host.set_position_withheld(p, oversized.clone()),
        Err(HostError::InvalidGrants)
    );
    assert_unchanged(&mut host, &before, &peers);
    // Pairs with both faults present: the earlier check wins.
    assert_eq!(
        host.set_position_withheld(ghost, oversized.clone()),
        Err(HostError::UnknownPeer)
    );
    assert_eq!(
        host.set_position_withheld(q, oversized),
        Err(HostError::NotV4Peer)
    );
    assert_unchanged(&mut host, &before, &peers);
    assert_eq!(
        HostError::NotV4Peer.to_string(),
        "NotV4Peer at host/admission"
    );
    // 1,024 distinct entries are accepted.
    let full: Vec<EntityId> = (0..1024).map(eid).collect();
    assert_eq!(host.set_position_withheld(p, full), Ok(()));
    assert!(host.cached_status_v3(p, 1) != Err(HostError::NotV3Peer));

    // `bind_v4_peer`: a regressed `now_ms` beats an oversized control list.
    let mut bound = host_of(&mv3_value());
    bind_p(&mut bound, 100);
    let oversized_control = ControlGrant {
        actors: (0..1025).map(eid).collect(),
    };
    assert_eq!(
        bound.bind_v4_peer(oversized_control, full_grants(&[a()]), facts(), r0(), 50,),
        Err(HostError::TimeRegression)
    );
    // The ninth binding is `ServerBusy`.
    let mut busy = host_of(&mv3_value());
    for _ in 0..8 {
        bind_p(&mut busy, 0);
    }
    assert_eq!(
        busy.bind_v4_peer(control(&[a()]), full_grants(&[a()]), facts(), r0(), 0),
        Err(HostError::ServerBusy)
    );

    // Closed beats an unknown handle.
    let mut closed = host_of(&mv3_value());
    let bound = bind_p(&mut closed, 0);
    let ghost = bind_p(&mut closed, 0);
    closed.unbind_peer(ghost);
    closed.shutdown();
    assert_eq!(
        closed.set_position_withheld(ghost, vec![b()]),
        Err(HostError::Closed)
    );
    assert_eq!(
        closed.set_position_withheld(bound, vec![b()]),
        Ok(()).or(Err(HostError::Closed))
    );
}

// ---------------------------------------------------------------------------
// B9.
// ---------------------------------------------------------------------------

/// The v2 and v3 entries of a session, for a byte comparison.
fn entries_of(host: &mut Host, peer: PeerHandle) -> Vec<DeliveryEntry> {
    take(host, peer)
}

/// The capture page with the v4 session's views removed.
fn captures_without(captures: Vec<CaptureEntry>, epoch: [u8; 16]) -> Vec<CaptureEntry> {
    captures
        .into_iter()
        .map(|entry| match entry {
            CaptureEntry::Host(record) => CaptureEntry::Host(record),
            CaptureEntry::Command(mut record) => {
                record.views.retain(|view| view.epoch != epoch);
                CaptureEntry::Command(record)
            }
        })
        .collect()
}

fn assert_same_entries(left: &[DeliveryEntry], right: &[DeliveryEntry]) {
    assert_eq!(
        shape(left),
        shape(right),
        "entry shape: {left:?} vs {right:?}"
    );
    for (l, r) in left.iter().zip(right.iter()) {
        assert_eq!(l.bytes, r.bytes);
    }
}

/// B9: a v4 session bound beside v2 and v3 ones changes no byte of theirs —
/// in delivery or in the capture journal — while it does receive `Position`
/// and `Moved`.
#[test]
fn b9_older_bytes_unchanged_beside_v4() {
    struct Rig {
        host: Host,
        v: PeerHandle,
        q: PeerHandle,
        twin: Twin,
        p: Option<(PeerHandle, Client)>,
        now: u64,
    }
    impl Rig {
        fn new(v4: bool) -> Self {
            let mut host = host_of(&mv3_value());
            let v = host
                .bind_peer(control(&[a()]), full_grants(&[a(), b(), c()]), 0)
                .expect("binds");
            let q = host
                .bind_v3_peer(
                    control(&[a()]),
                    full_grants(&[a(), b(), c()]),
                    facts(),
                    r1(),
                    0,
                )
                .expect("binds");
            let mut twin = Twin::new(host.epoch(q).expect("bound"));
            twin.drain(&mut host, q, 0);
            assert_eq!(
                twin.publish(&mut host, q, 0),
                TransferDisposition::Published
            );
            take(&mut host, v);
            let p = if v4 {
                let handle = host
                    .bind_v4_peer(
                        control(&[a(), b(), c()]),
                        full_grants(&[a(), b(), c()]),
                        facts(),
                        r1(),
                        0,
                    )
                    .expect("binds");
                let mut client = Client::new(host.epoch(handle).expect("bound"));
                client.drain(&mut host, handle, 0);
                assert_eq!(
                    client.publish(&mut host, handle, 0),
                    TransferDisposition::Published
                );
                Some((handle, client))
            } else {
                None
            };
            Self {
                host,
                v,
                q,
                twin,
                p,
                now: 0,
            }
        }

        /// Takes `V`'s and `Q`'s entries, returning them for comparison.
        fn take_older(&mut self) -> (Vec<DeliveryEntry>, Vec<DeliveryEntry>) {
            let v = entries_of(&mut self.host, self.v);
            let q = self.twin.drain(&mut self.host, self.q, self.now);
            (v, q)
        }

        fn p_entries(&mut self) -> Vec<DeliveryEntry> {
            match &mut self.p {
                Some((handle, client)) => client.drain(&mut self.host, *handle, self.now).entries,
                None => Vec::new(),
            }
        }
    }

    let mut x = Rig::new(false);
    let mut y = Rig::new(true);
    let p_epoch =
        y.p.as_ref()
            .map(|_| y.host.epoch(y.p.unwrap().0).expect("bound"));

    // The mesh, then a trusted move while A still holds the turn.
    for rig in [&mut x, &mut y] {
        assert_eq!(
            rig.host
                .bind_area_bounds(fp(0, 0, 0), fp(655360, 655360, 655360))
                .map(|binding| binding.revision),
            Ok(crpg_sim::NavRevision::of_bytes(
                &crpg_nav::bake_box(
                    Fx16_16::ZERO,
                    crpg_nav::NavRect {
                        min_x: Fx16_16::ZERO,
                        min_z: Fx16_16::ZERO,
                        max_x: Fx16_16::from_int(10),
                        max_z: Fx16_16::from_int(10),
                    },
                    &[],
                )
                .expect("bakes")
                .to_canonical_bytes()
            ))
        );
        assert_eq!(rig.host.host_move(a(), m(4, 1)).map(|o| o.cost), Ok(1));
        let (v, q) = rig.take_older();
        let p = rig.p_entries();
        if rig.p.is_none() {
            assert!(p.is_empty());
        }
        let _ = (v, q);
    }
    let (xv, xq) = x.take_older();
    let (yv, yq) = y.take_older();
    assert_same_entries(&xv, &yv);
    assert_same_entries(&xq, &yq);
    let xp = x.p_entries();
    let yp = y.p_entries();
    assert!(xp.is_empty());
    assert!(!yp.is_empty());
    assert_same_entries(&x.p_entries(), &xp);

    // B2's script, run on both hosts' v3 session.
    for rig in [&mut x, &mut y] {
        rig.now += 10;
        let now = rig.now;
        send_v3(
            &mut rig.host,
            rig.q,
            1,
            1,
            IntentBodyV3::DeclareAction {
                ability: strike(),
                target: n(2),
            },
            now,
        );
        let (v, q) = rig.take_older();
        let _ = (v, q);
        send_v3(&mut rig.host, rig.q, 2, 1, IntentBodyV3::EndTurn, now);
        rig.host
            .set_fact_grants(rig.q, facts_without_a_pools())
            .expect("v3");
        rig.host.pause(now).expect("pauses");
        rig.host.unpause(now).expect("unpauses");
        rig.host.tick().expect("ticks");
        assert_eq!(
            rig.host.set_grants(rig.q, full_grants(&[a(), b()])),
            Ok(GrantUpdate::SnapshotStaged)
        );
        let _ = rig.take_older();
        let _ = rig.p_entries();
    }
    let (xv, xq) = x.take_older();
    let (yv, yq) = y.take_older();
    assert_same_entries(&xv, &yv);
    assert_same_entries(&xq, &yq);
    let x_captures = captures_without(x.host.read_capture_entries(0, 256).expect("reads"), [0; 16]);
    let y_captures = captures_without(
        y.host.read_capture_entries(0, 256).expect("reads"),
        p_epoch.expect("a v4 session"),
    );
    assert_eq!(x_captures, y_captures);
    // Control: the v4 session did receive `Position` and `Moved` ops.
    let p_ops = y.p_entries();
    let mut saw_position = false;
    let mut saw_moved = false;
    for entry in &p_ops {
        if entry.kind != DeliveryKind::Delta {
            continue;
        }
        for op in entry_ops(entry) {
            match op {
                DeltaOp::Position { .. } => saw_position = true,
                DeltaOp::Moved { .. } => saw_moved = true,
                DeltaOp::V3(_) => {}
            }
        }
    }
    assert!(saw_position && saw_moved, "{p_ops:?}");
    let _ = xq.len();
    let _ = CapturedRecord {
        capture_seq: 0,
        epoch: [0; 16],
        lane: 0,
        seq: 0,
        history_start: 0,
        history_end: 0,
        outcome: None,
        processed_tick: 0,
        events: Vec::new(),
        views: Vec::new(),
    };
    let _: IntentBodyV1 = IntentBodyV1::EndTurn;
    let _ = (b(), c(), smite(), whiff(), EncounterMode::TurnBased);
}
