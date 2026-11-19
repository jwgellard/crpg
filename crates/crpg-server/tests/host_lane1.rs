//! T037d acceptance suite (`tasks/T037.md`, tests D1 to D12; D10's largest
//! receipt is a unit test in `src/host_v4.rs`): lane-1 admission, the decision
//! window, receipts, the duplicate cache and HM-10, public API only.
//!
//! Oracles are independent of the host: a test-owned mirror `HistoryWorld`
//! loaded from the same JSON and driven with the same public sim calls (routes
//! computed by calling `crpg_nav` directly on the same mesh), and hand-written
//! expected receipts and samples. Intents are built with the client's
//! `MoveOutbox` or by hand and encoded with `movement_v4::encode_intent`;
//! receipts and samples are decoded with `movement_v4::decode_down`. The v4
//! client is the T026p5 one (`ReassemblyPoolV4` feeding a `ReplicaWorldV4`).
//! The fixtures are `tests/fixtures/three_combatants.json` as saved-game JSON,
//! edited in memory (no new fixture file):
//!
//! - `MV3`: A/B/C movers at `m(1, 1)`, `m(3, 1)`, `m(1, 5)`, their `f32`
//!   mirrors and the definition's movement rules (speed 1 m/tick, one `JY`
//!   unit per started 10 m); turn-based.
//! - `MV3rt`: the same edits on the real-time JSON.
//! - `MV3-C`: `MV3rt` without `C`'s mover and transform.
//!
//! `P` is `bind_v4_peer([A], FULL(A,B,C), FACTS, R1, ..)` drained and published
//! unless a case says otherwise. Every refusal is compared against the host
//! state taken just before the call and sits beside an accepted control.

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_nav::{bake_box, NavMesh, NavRect};
use crpg_net::codec_v4;
use crpg_net::facts_v3::EncounterGrants;
use crpg_net::movement_v4::{
    decode_down, encode_intent, MoveBody, MoveDown, MoveIntent, MoveOutcome as MoveVerdict,
    MoveReceipt, MoveSample, SampleEntry,
};
use crpg_net::outbox_v4::MoveOutbox;
use crpg_net::pool_v4::{
    GateMode, ReassemblyPoolV4, ReceiverIdV4, ReplicaUpdateV4, TransferLimitsV3,
};
use crpg_net::protocol::{
    NetId, ReceiptStatus, LANE_COMBAT, POLICY_INGRESS_BYTES_PER_PEER,
    POLICY_INGRESS_FRAMES_PER_PEER,
};
use crpg_net::protocol_transfer::{TransferId, TransferUp};
use crpg_net::protocol_v3::{PauseRights, RejectionCode as CodeV3};
use crpg_net::protocol_v4::SessionEpoch;
use crpg_net::protocol_v4::{IntentBody as IntentBodyV4, IntentFrame as IntentFrameV4, Point};
use crpg_net::replica_v4::ReplicaWorldV4;
use crpg_net::transfer_v3;
use crpg_server::capture::CaptureEntry;
use crpg_server::host::{
    AdvanceReport, ControlGrant, DeliveryEntry, DisclosureGrants, EntityDisclosure,
    EntityFactGrant, FactGrants, GrantUpdate, Host, HostConfig, HostError, IngestDisposition,
    MoveAdmission, PeerHandle, ProtocolSelection, TransferDisposition, TransferState,
    DEFAULT_HOST_GRANTS,
};
use crpg_sim::{
    history_hash, CombatAction, HistoryEvent, HistoryWorld, MoveRoute, NavRevision, RouteTag,
};
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
fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}
/// A raw wire point `[x, y, z]`.
fn pt(x: i32, y: i32, z: i32) -> Point {
    Point::new([x, y, z]).expect("within the wire bound")
}
/// A whole-metre floor point `[x, 0, z]`.
fn m(x: i32, z: i32) -> [Fx16_16; 3] {
    [Fx16_16::from_int(x), Fx16_16::ZERO, Fx16_16::from_int(z)]
}

fn rect(x0: i32, z0: i32, x1: i32, z1: i32) -> NavRect {
    NavRect {
        min_x: Fx16_16::from_int(x0),
        min_z: Fx16_16::from_int(z0),
        max_x: Fx16_16::from_int(x1),
        max_z: Fx16_16::from_int(z1),
    }
}

/// `M10`: the 10 m square that holds `MV3`'s three movers.
fn m10() -> NavMesh {
    bake_box(Fx16_16::ZERO, rect(0, 0, 10, 10), &[]).expect("bakes")
}

fn r(mesh: &NavMesh) -> NavRevision {
    NavRevision::of_bytes(&mesh.to_canonical_bytes())
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

fn mv3rt_value() -> Value {
    movement_edits(rt3_value())
}

/// `MV3-C`: `MV3rt` without `C`'s mover and transform.
fn mv3c_value() -> Value {
    let mut value = mv3rt_value();
    value["world"]["movers"]
        .as_array_mut()
        .expect("movers")
        .pop();
    value["world"]["transforms"]
        .as_array_mut()
        .expect("transforms")
        .pop();
    value
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

// ---------------------------------------------------------------------------
// Intent bytes.
// ---------------------------------------------------------------------------

fn halt_intent(epoch: [u8; 16], seq: u64, actor: u64) -> MoveIntent {
    MoveIntent {
        epoch,
        seq,
        actor: n(actor),
        body: MoveBody::Halt,
    }
}

fn move_to_intent(epoch: [u8; 16], seq: u64, actor: u64, to: (i32, i32, i32)) -> MoveIntent {
    MoveIntent {
        epoch,
        seq,
        actor: n(actor),
        body: MoveBody::MoveTo {
            destination: pt(to.0, to.1, to.2),
        },
    }
}

fn bytes(intent: &MoveIntent) -> Vec<u8> {
    encode_intent(intent).expect("encodes")
}

fn halt_bytes(epoch: [u8; 16], seq: u64, actor: u64) -> Vec<u8> {
    bytes(&halt_intent(epoch, seq, actor))
}

fn move_to_bytes(epoch: [u8; 16], seq: u64, actor: u64, to: (i32, i32, i32)) -> Vec<u8> {
    bytes(&move_to_intent(epoch, seq, actor, to))
}

// ---------------------------------------------------------------------------
// Rig: one host and its mirror authority.
// ---------------------------------------------------------------------------

struct Rig {
    host: Host,
    oracle: HistoryWorld,
    now: u64,
}

impl Rig {
    fn new(value: &Value) -> Self {
        let host = Host::from_history(history_of(value), config(), 0).expect("wraps");
        Self {
            host,
            oracle: history_of(value),
            now: 0,
        }
    }

    fn epoch(&self, peer: PeerHandle) -> [u8; 16] {
        self.host.epoch(peer).expect("bound")
    }

    fn mirror_ack(&mut self) {
        self.oracle
            .acknowledge(self.oracle.last_sequence())
            .expect("acknowledges its own range");
    }

    /// Binds `M10` on the host and its revision on the mirror.
    fn bind_nav(&mut self) {
        let mesh = m10();
        let binding = self.host.bind_nav_mesh(mesh).expect("binds");
        assert_eq!(binding.cleared, Vec::new());
        assert_eq!(binding.revision, r(&m10()));
        self.oracle
            .bind_nav(r(&m10()))
            .expect("the mirror binds the revision");
        self.mirror_ack();
    }

    /// The mirror's `set_route` for `actor` from its current position to
    /// `destination`, computed by `crpg_nav` directly (the host's `plan_route`
    /// is the same two calls), tagged `tag`; acknowledged.
    fn mirror_route(&mut self, actor: EntityId, destination: [Fx16_16; 3], tag: u64) {
        let mesh = m10();
        let goal = mesh.nearest_on_mesh(destination);
        let from = self
            .oracle
            .world()
            .movers()
            .get(actor)
            .expect("a mover")
            .position();
        let path = mesh
            .find_path(from, goal)
            .expect("the mirror finds the path");
        self.oracle
            .set_route(
                actor,
                MoveRoute {
                    revision: r(&mesh),
                    tag: RouteTag(tag),
                    points: path.into_points(),
                },
            )
            .expect("the mirror sets the route");
        self.mirror_ack();
    }

    /// The mirror's `clear_route`, acknowledged.
    fn mirror_halt(&mut self, actor: EntityId) {
        self.oracle
            .clear_route(actor)
            .expect("the mirror clears the route");
        self.mirror_ack();
    }

    fn mirror_perform(&mut self, action: &CombatAction) {
        self.oracle
            .perform_action(action)
            .expect("the mirror performs");
        self.mirror_ack();
    }

    fn mirror_tick(&mut self) {
        self.oracle.tick().expect("the mirror ticks");
        self.mirror_ack();
    }

    fn assert_authority(&self) {
        assert_eq!(self.host.authority_hash(), history_hash(&self.oracle));
    }

    /// `P`: the v4 session under test, controlling exactly `actors`.
    fn bind_p_control(&mut self, actors: &[EntityId]) -> PeerHandle {
        self.host
            .bind_v4_peer(
                control(actors),
                full_grants(&[a(), b(), c()]),
                facts(),
                r1(),
                self.now,
            )
            .expect("binds")
    }

    fn bind_p(&mut self) -> PeerHandle {
        self.bind_p_control(&[a()])
    }

    fn bind_v3(&mut self) -> PeerHandle {
        self.host
            .bind_v3_peer(
                control(&[a()]),
                full_grants(&[a(), b(), c()]),
                facts(),
                r1(),
                self.now,
            )
            .expect("binds")
    }

    /// Drains and publishes a fresh v4 binding, returning its client.
    fn publish(&mut self, peer: PeerHandle) -> Client {
        let mut client = Client::new(self.epoch(peer));
        client.drain(&mut self.host, peer, self.now);
        assert_eq!(
            client.publish(&mut self.host, peer, self.now),
            TransferDisposition::Published
        );
        client
    }

    fn ingest_move(&mut self, peer: PeerHandle, bytes: &[u8], now: u64) -> MoveAdmission {
        self.now = now;
        self.host.ingest_move(peer, bytes, now).expect("open")
    }

    fn receipts(&mut self, peer: PeerHandle) -> Vec<MoveReceipt> {
        receipts(&mut self.host, peer)
    }

    fn samples(&mut self, peer: PeerHandle) -> Vec<MoveSample> {
        samples(&mut self.host, peer)
    }

    /// One `advance`, its tick mirrored.
    fn advance(&mut self, now: u64) -> AdvanceReport {
        self.now = now;
        let report = self.host.advance(now).expect("advances");
        if report.ticked {
            self.mirror_tick();
        }
        report
    }

    fn assert_ticked(&mut self, now: u64) {
        let report = self.advance(now);
        assert!(report.ticked, "{report:?}");
        assert_eq!(report.paused, None);
        self.assert_authority();
    }

    fn assert_paused(&mut self, now: u64) {
        let report = self.advance(now);
        assert!(!report.ticked, "{report:?}");
        self.assert_authority();
    }

    /// Drains and acknowledges a fresh v4 binding without `Published`:
    /// publishing one transfer spends a shared intent-bucket token
    /// (`ingest_transfer`), which the 80-frame budget cases must not lose.
    fn settle(&mut self, peer: PeerHandle) {
        let mut client = Client::new(self.epoch(peer));
        client.drain(&mut self.host, peer, self.now);
    }

    fn bind_log(&mut self, peer: PeerHandle) -> Vec<DeliveryEntry> {
        self.host.take_delivery_entries(peer).expect("bound")
    }
}

fn receipts(host: &mut Host, peer: PeerHandle) -> Vec<MoveReceipt> {
    host.take_move_receipts(peer)
        .expect("a v4 session")
        .iter()
        .map(
            |bytes| match decode_down(bytes).expect("a move-down datagram") {
                MoveDown::Receipt(receipt) => receipt,
                other => panic!("expected a receipt, got {other:?}"),
            },
        )
        .collect()
}

fn samples(host: &mut Host, peer: PeerHandle) -> Vec<MoveSample> {
    host.take_samples(peer)
        .expect("a v4 session")
        .iter()
        .map(
            |bytes| match decode_down(bytes).expect("a move-down datagram") {
                MoveDown::Sample(sample) => sample,
                other => panic!("expected a sample, got {other:?}"),
            },
        )
        .collect()
}

/// A refusal leaves all of this as it was.
#[derive(Debug, PartialEq, Eq)]
struct State {
    hash: [u8; 32],
    last_capture: u64,
    executions: u64,
    nav: Option<NavRevision>,
}

/// The state a refusal must leave as it was; every peer's delivery log,
/// samples and receipts are emptied first.
fn state_of(rig: &mut Rig, peers: &[PeerHandle]) -> State {
    for peer in peers {
        drain(&mut rig.host, *peer);
        let _ = rig.host.take_samples(*peer);
        let _ = rig.host.take_move_receipts(*peer);
    }
    State {
        hash: rig.host.authority_hash(),
        last_capture: rig.host.last_capture(),
        executions: rig.host.executions(),
        nav: rig.host.nav_revision(),
    }
}

fn assert_unchanged(rig: &mut Rig, before: &State, peers: &[PeerHandle]) {
    assert_eq!(rig.host.authority_hash(), before.hash);
    assert_eq!(rig.host.last_capture(), before.last_capture);
    assert_eq!(rig.host.executions(), before.executions);
    assert_eq!(rig.host.nav_revision(), before.nav);
    for peer in peers {
        assert_eq!(rig.host.take_delivery_entries(*peer), Ok(Vec::new()));
        assert_v4_queues_empty(&mut rig.host, *peer);
    }
}

/// The authority, captures and counters a rejection must leave as they were,
/// without claiming its (legitimate) receipt queue is empty.
fn assert_world_unchanged(rig: &Rig, before: &State) {
    assert_eq!(rig.host.authority_hash(), before.hash);
    assert_eq!(rig.host.last_capture(), before.last_capture);
    assert_eq!(rig.host.executions(), before.executions);
    assert_eq!(rig.host.nav_revision(), before.nav);
}

/// A v4 session has nothing queued; a v1–v3 session has no lane 1 at all.
fn assert_v4_queues_empty(host: &mut Host, peer: PeerHandle) {
    match host.take_samples(peer) {
        Ok(samples) => assert!(samples.is_empty(), "{samples:?}"),
        Err(HostError::NotV4Peer) => {}
        Err(error) => panic!("unexpected samples error: {error:?}"),
    }
    match host.take_move_receipts(peer) {
        Ok(receipts) => assert!(receipts.is_empty(), "{receipts:?}"),
        Err(HostError::NotV4Peer) => {}
        Err(error) => panic!("unexpected receipt error: {error:?}"),
    }
}

/// Takes and acknowledges every pending entry of `peer`.
fn drain(host: &mut Host, peer: PeerHandle) -> Vec<DeliveryEntry> {
    let entries = host.take_delivery_entries(peer).expect("bound");
    if let Some(last) = entries.last() {
        host.acknowledge_delivery(peer, last.ordinal).expect("acks");
    }
    entries
}

/// Decides every pending slot in a paused `advance` (D-5, D-7): the world may
/// change, but the decision makes no capture record and publishes no lane-0
/// frame (Reading 21). The pause's own publication is drained first, so what
/// is compared is only what the decision emits; `before` comes from
/// [`state_of`], whose drain proves the log started empty.
fn decide_paused(rig: &mut Rig, peer: PeerHandle, before: &State, now: u64) {
    assert_eq!(rig.host.pause(now), Ok(()));
    drain(&mut rig.host, peer);
    rig.assert_paused(now);
    assert_eq!(rig.host.last_capture(), before.last_capture);
    let delivered = drain(&mut rig.host, peer);
    assert!(delivered.is_empty(), "{delivered:?}");
    assert_eq!(rig.host.unpause(now), Ok(()));
}

// ---------------------------------------------------------------------------
// The v4 client (T026p5).
// ---------------------------------------------------------------------------

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

    fn drain(&mut self, host: &mut Host, peer: PeerHandle, now: u64) {
        let entries = host.take_delivery_entries(peer).expect("bound");
        for entry in &entries {
            match entry.kind {
                crpg_server::host::DeliveryKind::Transfer => {
                    self.pool
                        .offer_transfer(self.rid, &entry.bytes, now)
                        .expect("the gate accepts the frame");
                }
                crpg_server::host::DeliveryKind::Delta => {
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
                ReplicaUpdateV4::Snapshot(publication) => {
                    self.last_snapshot = Some(publication.transfer);
                    self.replica.apply_snapshot(&publication).expect("applies");
                }
                ReplicaUpdateV4::Delta(frame) => {
                    self.replica.apply_delta(&frame).expect("applies");
                }
            }
        }
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

/// A v4 rig with `M10` bound, `control` on its v4 session, published.
fn lane1_rig_control(value: &Value, controlled: &[EntityId]) -> (Rig, PeerHandle) {
    let mut rig = Rig::new(value);
    rig.bind_nav();
    let peer = rig.bind_p_control(controlled);
    rig.publish(peer);
    (rig, peer)
}

fn lane1_rig(value: &Value) -> (Rig, PeerHandle) {
    lane1_rig_control(value, &[a()])
}

/// A published-free settled v4 rig: the 80-token intent budget must be intact.
fn quiet_lane1_rig(value: &Value) -> (Rig, PeerHandle) {
    let mut rig = Rig::new(value);
    rig.bind_nav();
    let peer = rig.bind_p();
    rig.settle(peer);
    (rig, peer)
}

// ---------------------------------------------------------------------------
// D1.
// ---------------------------------------------------------------------------

/// D1: a `MoveTo` is admitted at once, decided (and receipted) by the next
/// `advance`, and starts walking on that call's tick; a paused `advance`
/// decides without ticking, and the walk starts on the first tick after
/// unpause. Samples echo the receipted `seq`.
#[test]
fn d1_move_to_accepted_and_walks() {
    let mut rig = Rig::new(&mv3rt_value());
    rig.bind_nav();
    let p = rig.bind_p();
    rig.publish(p);
    let epoch = rig.epoch(p);
    let mut outbox = MoveOutbox::new(epoch);

    let intent = outbox
        .send(
            n(1),
            MoveBody::MoveTo {
                destination: pt(262144, 0, 65536),
            },
        )
        .expect("sends");
    assert_eq!(intent.seq, 1);
    assert_eq!(
        rig.ingest_move(p, &bytes(&intent), 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    assert_eq!(rig.receipts(p), Vec::new());

    // The decision sets `RouteTag(1)` from `m(1, 1)` to `m(4, 1)`; the same
    // advance ticks once, moving A 1 m.
    rig.mirror_route(a(), m(4, 1), 1);
    rig.assert_ticked(20);
    let got = rig.receipts(p);
    assert_eq!(
        got,
        vec![MoveReceipt {
            epoch,
            seq: 1,
            actor: n(1),
            processed_tick: 0,
            outcome: MoveVerdict::Accepted { start_tick: 1 },
        }]
    );
    assert_eq!(outbox.receive(&got[0]), Ok(()));
    assert_eq!(
        rig.samples(p),
        vec![MoveSample {
            server_tick: 1,
            processed_seq: 1,
            entries: vec![SampleEntry {
                entity: n(1),
                point: pt(131072, 0, 65536),
                moving: true,
            }],
        }]
    );

    rig.assert_ticked(70);
    assert_eq!(
        rig.samples(p),
        vec![MoveSample {
            server_tick: 2,
            processed_seq: 1,
            entries: vec![SampleEntry {
                entity: n(1),
                point: pt(196608, 0, 65536),
                moving: true,
            }],
        }]
    );
    rig.assert_ticked(120);
    assert_eq!(
        rig.samples(p),
        vec![MoveSample {
            server_tick: 3,
            processed_seq: 1,
            entries: vec![SampleEntry {
                entity: n(1),
                point: pt(262144, 0, 65536),
                moving: false,
            }],
        }]
    );
    assert_eq!(rig.receipts(p), Vec::new());

    // Paused: the window decides, but no tick runs and no sample appears.
    assert_eq!(rig.host.pause(200), Ok(()));
    rig.now = 200;
    let second = outbox
        .send(
            n(1),
            MoveBody::MoveTo {
                destination: pt(65536, 0, 262144),
            },
        )
        .expect("sends");
    assert_eq!(second.seq, 2);
    assert_eq!(
        rig.ingest_move(p, &bytes(&second), 210),
        MoveAdmission::Admitted { seq: 2 }
    );
    rig.mirror_route(a(), m(1, 4), 2);
    rig.assert_paused(250);
    assert_eq!(
        rig.receipts(p),
        vec![MoveReceipt {
            epoch,
            seq: 2,
            actor: n(1),
            processed_tick: 3,
            outcome: MoveVerdict::Accepted { start_tick: 4 },
        }]
    );
    assert_eq!(rig.samples(p), Vec::new());

    assert_eq!(rig.host.unpause(260), Ok(()));
    rig.now = 260;
    rig.assert_ticked(270);
    assert_eq!(
        rig.samples(p),
        vec![MoveSample {
            server_tick: 4,
            processed_seq: 2,
            entries: vec![SampleEntry {
                entity: n(1),
                point: pt(215804, 0, 111876),
                moving: true,
            }],
        }]
    );
}

// ---------------------------------------------------------------------------
// D2.
// ---------------------------------------------------------------------------

/// D2: within one window only the last admitted intent per actor wins, every
/// earlier one is `Superseded`, and the order is host-wide admission order,
/// never handle or `seq` order. Receipts leave each session in `seq` order.
#[test]
fn d2_supersede_and_order() {
    let mut rig = Rig::new(&mv3rt_value());
    rig.bind_nav();
    let p = rig.bind_p_control(&[a(), b()]);
    rig.publish(p);
    let p2 = rig.bind_p_control(&[a(), b()]);
    rig.publish(p2);
    assert_eq!(rig.host.set_control(p, control(&[a()])), Ok(()));
    drain(&mut rig.host, p);
    let epoch_p = rig.epoch(p);
    let epoch_2 = rig.epoch(p2);

    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch_p, 1, 1, (262144, 0, 65536)), 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch_p, 2, 2, (65536, 0, 262144)), 10),
        MoveAdmission::Rejected {
            seq: 2,
            code: CodeV3::NotAuthorized,
        }
    );
    assert_eq!(rig.receipts(p), Vec::new(), "held behind seq 1");
    assert_eq!(
        rig.ingest_move(p2, &move_to_bytes(epoch_2, 1, 1, (65536, 0, 262144)), 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    assert_eq!(
        rig.ingest_move(p2, &halt_bytes(epoch_2, 2, 2), 10),
        MoveAdmission::Admitted { seq: 2 }
    );
    // The winning route is P2's seq 1: A walks to m(1, 4).
    rig.mirror_route(a(), m(1, 4), 1);
    rig.mirror_halt(b());
    rig.assert_ticked(10);

    assert_eq!(
        rig.receipts(p),
        vec![
            MoveReceipt {
                epoch: epoch_p,
                seq: 1,
                actor: n(1),
                processed_tick: 0,
                outcome: MoveVerdict::Superseded,
            },
            MoveReceipt {
                epoch: epoch_p,
                seq: 2,
                actor: n(2),
                processed_tick: 0,
                outcome: MoveVerdict::Rejected(CodeV3::NotAuthorized),
            },
        ]
    );
    assert_eq!(
        rig.receipts(p2),
        vec![
            MoveReceipt {
                epoch: epoch_2,
                seq: 1,
                actor: n(1),
                processed_tick: 0,
                outcome: MoveVerdict::Accepted { start_tick: 1 },
            },
            MoveReceipt {
                epoch: epoch_2,
                seq: 2,
                actor: n(2),
                processed_tick: 0,
                outcome: MoveVerdict::Accepted { start_tick: 1 },
            },
        ]
    );

    // With nothing pending, a rejected intent is receipted at once.
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch_p, 3, 2, (65536, 0, 262144)), 20),
        MoveAdmission::Rejected {
            seq: 3,
            code: CodeV3::NotAuthorized,
        }
    );
    assert_eq!(
        rig.receipts(p),
        vec![MoveReceipt {
            epoch: epoch_p,
            seq: 3,
            actor: n(2),
            processed_tick: 1,
            outcome: MoveVerdict::Rejected(CodeV3::NotAuthorized),
        }]
    );

    // Admission order, not handle or seq: P2 (handle 2) admits first, so its
    // A intent is superseded by P's later A intent, which has the lower
    // handle and the lower seq.
    assert_eq!(
        rig.ingest_move(p2, &halt_bytes(epoch_2, 3, 2), 20),
        MoveAdmission::Admitted { seq: 3 }
    );
    assert_eq!(
        rig.ingest_move(p2, &halt_bytes(epoch_2, 4, 2), 20),
        MoveAdmission::Admitted { seq: 4 }
    );
    assert_eq!(
        rig.ingest_move(p2, &move_to_bytes(epoch_2, 5, 1, (196608, 0, 262144)), 20),
        MoveAdmission::Admitted { seq: 5 }
    );
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch_p, 4, 1, (262144, 0, 65536)), 20),
        MoveAdmission::Admitted { seq: 4 }
    );
    rig.mirror_halt(b());
    rig.mirror_route(a(), m(4, 1), 4);
    rig.assert_ticked(20);

    let got = rig.receipts(p2);
    assert_eq!(got.len(), 3, "{got:?}");
    assert_eq!(got[0].seq, 3);
    assert_eq!(got[0].outcome, MoveVerdict::Superseded);
    assert_eq!(got[1].seq, 4);
    assert_eq!(got[1].processed_tick, 1);
    assert!(matches!(
        got[1].outcome,
        MoveVerdict::Accepted { start_tick: 2 }
    ));
    assert_eq!(got[2].seq, 5);
    assert_eq!(got[2].outcome, MoveVerdict::Superseded);
    let got = rig.receipts(p);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].seq, 4);
    assert_eq!(got[0].processed_tick, 1);
    assert!(matches!(
        got[0].outcome,
        MoveVerdict::Accepted { start_tick: 2 }
    ));
    let active = rig
        .oracle
        .world()
        .movers()
        .get(a())
        .expect("a mover")
        .route()
        .expect("a route");
    assert_eq!(active.route().points.last(), Some(&m(4, 1)));
    rig.assert_authority();
}

// ---------------------------------------------------------------------------
// D3.
// ---------------------------------------------------------------------------

/// D3: HM-4's decision refusals, in order: `WrongMode` for a turn-based
/// combatant, then `Superseded`, then `IllegalAction` for a missing mesh,
/// an already-there destination and a missing mover; the world is unchanged.
#[test]
fn d3_decision_refusals() {
    // Turn-based: `MoveTo` and `Halt` are both `WrongMode`.
    let mut rig = Rig::new(&mv3_value());
    rig.bind_nav();
    let p = rig.bind_p();
    rig.publish(p);
    let epoch = rig.epoch(p);
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch, 1, 1, (262144, 0, 65536)), 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    let state = state_of(&mut rig, &[p]);
    decide_paused(&mut rig, p, &state, 10);
    rig.assert_ticked(10);
    assert_eq!(
        rig.receipts(p),
        vec![MoveReceipt {
            epoch,
            seq: 1,
            actor: n(1),
            processed_tick: 0,
            outcome: MoveVerdict::Rejected(CodeV3::WrongMode),
        }]
    );

    assert_eq!(
        rig.ingest_move(p, &halt_bytes(epoch, 2, 1), 20),
        MoveAdmission::Admitted { seq: 2 }
    );
    let state = state_of(&mut rig, &[p]);
    decide_paused(&mut rig, p, &state, 20);
    rig.assert_ticked(20);
    assert_eq!(
        rig.receipts(p),
        vec![MoveReceipt {
            epoch,
            seq: 2,
            actor: n(1),
            processed_tick: 1,
            outcome: MoveVerdict::Rejected(CodeV3::WrongMode),
        }]
    );

    // Two `MoveTo` for A in one window: the first is superseded, the second
    // refused for the mode.
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch, 3, 1, (262144, 0, 65536)), 30),
        MoveAdmission::Admitted { seq: 3 }
    );
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch, 4, 1, (196608, 0, 65536)), 30),
        MoveAdmission::Admitted { seq: 4 }
    );
    let state = state_of(&mut rig, &[p]);
    decide_paused(&mut rig, p, &state, 30);
    rig.assert_ticked(30);
    assert_eq!(
        rig.receipts(p),
        vec![
            MoveReceipt {
                epoch,
                seq: 3,
                actor: n(1),
                processed_tick: 2,
                outcome: MoveVerdict::Superseded,
            },
            MoveReceipt {
                epoch,
                seq: 4,
                actor: n(1),
                processed_tick: 2,
                outcome: MoveVerdict::Rejected(CodeV3::WrongMode),
            },
        ]
    );

    // Unbound: no mesh is `IllegalAction`.
    let mut unbound = Rig::new(&mv3rt_value());
    let p = unbound.bind_p();
    unbound.publish(p);
    let epoch = unbound.epoch(p);
    assert_eq!(
        unbound.ingest_move(p, &move_to_bytes(epoch, 1, 1, (262144, 0, 65536)), 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    let state = state_of(&mut unbound, &[p]);
    decide_paused(&mut unbound, p, &state, 10);
    unbound.assert_ticked(10);
    assert_eq!(
        unbound.receipts(p),
        vec![MoveReceipt {
            epoch,
            seq: 1,
            actor: n(1),
            processed_tick: 0,
            outcome: MoveVerdict::Rejected(CodeV3::IllegalAction),
        }]
    );

    // Bound, destination is the actor's own position: `AlreadyThere`.
    let mut bound = Rig::new(&mv3rt_value());
    bound.bind_nav();
    let p = bound.bind_p();
    bound.publish(p);
    let epoch = bound.epoch(p);
    assert_eq!(
        bound.ingest_move(p, &move_to_bytes(epoch, 1, 1, (65536, 0, 65536)), 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    let state = state_of(&mut bound, &[p]);
    decide_paused(&mut bound, p, &state, 10);
    bound.assert_ticked(10);
    assert_eq!(
        bound.receipts(p),
        vec![MoveReceipt {
            epoch,
            seq: 1,
            actor: n(1),
            processed_tick: 0,
            outcome: MoveVerdict::Rejected(CodeV3::IllegalAction),
        }]
    );
    // Control: the same host accepts its next intent's route — the decision
    // changes the world, while `decide_paused` proves it makes no record and
    // publishes no lane-0 frame.
    assert_eq!(
        bound.ingest_move(p, &move_to_bytes(epoch, 2, 1, (262144, 0, 65536)), 20),
        MoveAdmission::Admitted { seq: 2 }
    );
    let state = state_of(&mut bound, &[p]);
    let before_route = bound.host.authority_hash();
    bound.mirror_route(a(), m(4, 1), 2);
    decide_paused(&mut bound, p, &state, 20);
    assert_ne!(bound.host.authority_hash(), before_route);
    bound.assert_ticked(20);
    assert_eq!(
        bound.receipts(p)[0].outcome,
        MoveVerdict::Accepted { start_tick: 2 }
    );

    // A combatant without a mover: `Halt` is `IllegalAction`.
    let mut meshless = Rig::new(&mv3c_value());
    meshless.bind_nav();
    let p = meshless.bind_p_control(&[a(), b(), c()]);
    meshless.publish(p);
    let epoch = meshless.epoch(p);
    assert_eq!(
        meshless.ingest_move(p, &halt_bytes(epoch, 1, 3), 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    let state = state_of(&mut meshless, &[p]);
    decide_paused(&mut meshless, p, &state, 10);
    meshless.assert_ticked(10);
    assert_eq!(
        meshless.receipts(p),
        vec![MoveReceipt {
            epoch,
            seq: 1,
            actor: n(3),
            processed_tick: 0,
            outcome: MoveVerdict::Rejected(CodeV3::IllegalAction),
        }]
    );
}

// ---------------------------------------------------------------------------
// D4.
// ---------------------------------------------------------------------------

/// D4: every HM-2 admission refusal, each an otherwise-valid frame with one
/// fault, beside the same bytes with the fault removed (`Admitted` at the same
/// `seq`); no refusal consumes a sequence or queues a receipt.
#[test]
fn d4_admission_refusals() {
    // First byte 0xA0.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let valid = halt_bytes(epoch, 1, 1);
        let mut bad = valid.clone();
        bad[0] = 0xA0;
        let state = state_of(&mut rig, &[p]);
        assert_eq!(
            rig.ingest_move(p, &bad, 10),
            MoveAdmission::Refused {
                seq: None,
                code: CodeV3::UnsupportedVersion,
            }
        );
        assert_unchanged(&mut rig, &state, &[p]);
        assert_eq!(
            rig.ingest_move(p, &valid, 10),
            MoveAdmission::Admitted { seq: 1 }
        );
    }
    // Lane byte 0.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let valid = halt_bytes(epoch, 1, 1);
        let mut bad = valid.clone();
        bad[17] = 0;
        let state = state_of(&mut rig, &[p]);
        assert_eq!(
            rig.ingest_move(p, &bad, 10),
            MoveAdmission::Refused {
                seq: None,
                code: CodeV3::WrongDirection,
            }
        );
        assert_unchanged(&mut rig, &state, &[p]);
        assert_eq!(
            rig.ingest_move(p, &valid, 10),
            MoveAdmission::Admitted { seq: 1 }
        );
    }
    // Last byte cut.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let valid = halt_bytes(epoch, 1, 1);
        let bad = valid[..valid.len() - 1].to_vec();
        let state = state_of(&mut rig, &[p]);
        assert_eq!(
            rig.ingest_move(p, &bad, 10),
            MoveAdmission::Refused {
                seq: None,
                code: CodeV3::Malformed,
            }
        );
        assert_unchanged(&mut rig, &state, &[p]);
        assert_eq!(
            rig.ingest_move(p, &valid, 10),
            MoveAdmission::Admitted { seq: 1 }
        );
    }
    // A lane-0 v4 intent.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let lane0 = codec_v4::encode_intent(&IntentFrameV4 {
            epoch,
            lane: LANE_COMBAT,
            seq: 1,
            observed_tick: 0,
            actor: n(1),
            body: IntentBodyV4::DeclareMove {
                destination: pt(262144, 0, 65536),
            },
        })
        .expect("encodes");
        let state = state_of(&mut rig, &[p]);
        assert_eq!(
            rig.ingest_move(p, &lane0, 10),
            MoveAdmission::Refused {
                seq: None,
                code: CodeV3::UnsupportedVersion,
            }
        );
        assert_unchanged(&mut rig, &state, &[p]);
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 1, 1), 10),
            MoveAdmission::Admitted { seq: 1 }
        );
    }
    // Wrong epoch.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let valid = halt_bytes(epoch, 1, 1);
        let bad = halt_bytes([9; 16], 1, 1);
        let state = state_of(&mut rig, &[p]);
        assert_eq!(
            rig.ingest_move(p, &bad, 10),
            MoveAdmission::Refused {
                seq: Some(1),
                code: CodeV3::SessionExpired,
            }
        );
        assert_unchanged(&mut rig, &state, &[p]);
        assert_eq!(
            rig.ingest_move(p, &valid, 10),
            MoveAdmission::Admitted { seq: 1 }
        );
    }
    // Seq gap: 3 when 1 is next.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let valid = halt_bytes(epoch, 1, 1);
        let state = state_of(&mut rig, &[p]);
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 3, 1), 10),
            MoveAdmission::Refused {
                seq: Some(3),
                code: CodeV3::SeqGap,
            }
        );
        assert_unchanged(&mut rig, &state, &[p]);
        assert_eq!(
            rig.ingest_move(p, &valid, 10),
            MoveAdmission::Admitted { seq: 1 }
        );
    }
    // Same seq, other bytes, while pending: a fence.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 1, 1), 10),
            MoveAdmission::Admitted { seq: 1 }
        );
        assert_eq!(
            rig.ingest_move(p, &move_to_bytes(epoch, 1, 1, (262144, 0, 65536)), 10),
            MoveAdmission::Refused {
                seq: Some(1),
                code: CodeV3::SeqConflict,
            }
        );
        assert_eq!(rig.host.session_wire(p), Err(HostError::UnknownPeer));
        // Control: a fresh session admits the valid bytes at the same seq.
        let (mut control, q) = lane1_rig(&mv3rt_value());
        assert_eq!(
            control.ingest_move(q, &halt_bytes(control.epoch(q), 1, 1), 10),
            MoveAdmission::Admitted { seq: 1 }
        );
    }
    // 4,097 bytes: the valid 21-byte `Halt` plus 4,076 zero bytes.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let valid = halt_bytes(epoch, 1, 1);
        assert_eq!(valid.len(), 21);
        let mut bad = valid.clone();
        bad.resize(4_097, 0);
        let state = state_of(&mut rig, &[p]);
        assert_eq!(
            rig.ingest_move(p, &bad, 10),
            MoveAdmission::Refused {
                seq: None,
                code: CodeV3::FrameTooLarge,
            }
        );
        assert_unchanged(&mut rig, &state, &[p]);
        assert_eq!(
            rig.ingest_move(p, &valid, 10),
            MoveAdmission::Admitted { seq: 1 }
        );
    }
    // A v3 session and an unbound handle.
    {
        let mut rig = Rig::new(&mv3rt_value());
        rig.bind_nav();
        let q = rig.bind_v3();
        let epoch = rig.epoch(q);
        let state = state_of(&mut rig, &[q]);
        assert_eq!(
            rig.ingest_move(q, &halt_bytes(epoch, 1, 1), 10),
            MoveAdmission::Refused {
                seq: None,
                code: CodeV3::UnsupportedVersion,
            }
        );
        assert_unchanged(&mut rig, &state, &[q]);
        let p = rig.bind_p();
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(rig.epoch(p), 1, 1), 20),
            MoveAdmission::Admitted { seq: 1 }
        );

        let mut rig = Rig::new(&mv3rt_value());
        rig.bind_nav();
        let p = rig.bind_p();
        let epoch = rig.epoch(p);
        rig.host.unbind_peer(p);
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 1, 1), 10),
            MoveAdmission::Refused {
                seq: None,
                code: CodeV3::Unauthenticated,
            }
        );
        let q = rig.bind_p();
        assert_eq!(
            rig.ingest_move(q, &halt_bytes(rig.epoch(q), 1, 1), 20),
            MoveAdmission::Admitted { seq: 1 }
        );
    }
    // Closed and time regression are errors.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 1, 1), 20),
            MoveAdmission::Admitted { seq: 1 }
        );
        assert_eq!(
            rig.host.ingest_move(p, &halt_bytes(epoch, 2, 1), 10),
            Err(HostError::TimeRegression)
        );
        assert_eq!(
            rig.host.ingest_move(p, &halt_bytes(epoch, 2, 1), 20),
            Ok(MoveAdmission::Admitted { seq: 2 })
        );
        rig.host.shutdown();
        assert_eq!(
            rig.host.ingest_move(p, &halt_bytes(epoch, 3, 1), 20),
            Err(HostError::Closed)
        );
    }
    // In flight: the ninth unreceipted intent is `QueueFull`; untaken
    // receipts still count after the window releases them.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        for seq in 1..=8 {
            assert_eq!(
                rig.ingest_move(p, &halt_bytes(epoch, seq, 1), 5),
                MoveAdmission::Admitted { seq }
            );
        }
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 9, 1), 5),
            MoveAdmission::Refused {
                seq: Some(9),
                code: CodeV3::QueueFull,
            }
        );
        rig.mirror_halt(a());
        rig.assert_ticked(5);
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 9, 1), 5),
            MoveAdmission::Refused {
                seq: Some(9),
                code: CodeV3::QueueFull,
            }
        );
        assert_eq!(rig.receipts(p).len(), 8);
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 9, 1), 5),
            MoveAdmission::Admitted { seq: 9 }
        );
    }
    // The move bucket: 40 at one `now_ms`, the 41st refused; 50 ms later
    // admitted.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        for window in 0..5u64 {
            for step in 0..8u64 {
                let seq = window * 8 + step + 1;
                assert_eq!(
                    rig.ingest_move(p, &halt_bytes(epoch, seq, 1), 5),
                    MoveAdmission::Admitted { seq }
                );
            }
            rig.mirror_halt(a());
            rig.assert_ticked(5);
            assert_eq!(rig.receipts(p).len(), 8);
        }
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 41, 1), 5),
            MoveAdmission::Refused {
                seq: Some(41),
                code: CodeV3::RateLimited,
            }
        );
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 41, 1), 55),
            MoveAdmission::Admitted { seq: 41 }
        );
    }
    // Lane-0 ingress full: 128 lane-0 frames staged, one per 25 ms.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let mut now = 5;
        for seq in 1..=128u64 {
            let frame = codec_v4::encode_intent(&IntentFrameV4 {
                epoch,
                lane: LANE_COMBAT,
                seq,
                observed_tick: 0,
                actor: n(1),
                body: IntentBodyV4::DeclareAction {
                    ability: strike(),
                    target: n(2),
                },
            })
            .expect("encodes");
            assert_eq!(
                rig.host.ingest(p, &frame, now),
                Ok(crpg_server::host::IngestDisposition::Staged)
            );
            now += 25;
        }
        let state = state_of(&mut rig, &[p]);
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 1, 1), now),
            MoveAdmission::Refused {
                seq: None,
                code: CodeV3::QueueFull,
            }
        );
        assert_unchanged(&mut rig, &state, &[p]);
        // Control: once `pump` has drained the lane-0 ingress, the same
        // bytes are `Admitted`.
        rig.host.pump(now).expect("pumps the staged lane-0 frames");
        drain(&mut rig.host, p);
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 1, 1), now),
            MoveAdmission::Admitted { seq: 1 }
        );
    }
    // Stale: 257 decided and taken, so seq 1 left the 256-entry cache.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let mut now = 5;
        // 32 windows of eight plus one intent = 257.
        for window in 0..33u64 {
            let count = if window == 32 { 1 } else { 8 };
            for step in 0..count {
                let seq = window * 8 + step + 1;
                assert_eq!(
                    rig.ingest_move(p, &halt_bytes(epoch, seq, 1), now),
                    MoveAdmission::Admitted { seq }
                );
            }
            rig.mirror_halt(a());
            rig.assert_ticked(now);
            assert_eq!(rig.receipts(p).len() as u64, count);
            now += 400;
        }
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 1, 1), now),
            MoveAdmission::Refused {
                seq: Some(1),
                code: CodeV3::StaleSeq,
            }
        );
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 2, 1), now),
            MoveAdmission::Duplicate {
                seq: 2,
                resent: true,
            }
        );
    }
}

/// D-1 step 5: the 4,096-byte frame cap is exclusive ("more than
/// `MAX_INTENT_FRAME_BYTES`): a 4,096-byte frame passes the cap and is
/// refused by decode instead, while a 4,097-byte frame never reaches decode.
#[test]
fn d4_frame_cap_boundary() {
    let (mut rig, p) = lane1_rig(&mv3rt_value());
    let epoch = rig.epoch(p);
    let valid = halt_bytes(epoch, 1, 1);
    assert_eq!(valid.len(), 21);
    // Exactly the cap: passes step 5, fails decode.
    let mut at_cap = valid.clone();
    at_cap.resize(4_096, 0);
    assert_eq!(at_cap.len(), 4_096);
    let state = state_of(&mut rig, &[p]);
    assert_eq!(
        rig.ingest_move(p, &at_cap, 10),
        MoveAdmission::Refused {
            seq: None,
            code: CodeV3::Malformed,
        }
    );
    assert_unchanged(&mut rig, &state, &[p]);
    // One byte more: refused before decode.
    let mut over_cap = valid.clone();
    over_cap.resize(4_097, 0);
    assert_eq!(
        rig.ingest_move(p, &over_cap, 10),
        MoveAdmission::Refused {
            seq: None,
            code: CodeV3::FrameTooLarge,
        }
    );
    assert_unchanged(&mut rig, &state, &[p]);
    assert_eq!(
        rig.ingest_move(p, &valid, 10),
        MoveAdmission::Admitted { seq: 1 }
    );
}

/// D-1 step 4: frames refused by the cap or decode still spend the shared
/// byte bucket first. Thirty-two 4,096-byte frames (each `Malformed`) spend
/// exactly the 128 KiB byte burst, so the next valid frame is `RateLimited`;
/// after 31 such frames the next valid frame is admitted.
/// (Reading: 4,096-byte frames, not 4,097: with a 128 KiB burst the 32nd
/// 4,097-byte frame is itself `RateLimited` and spends nothing, so no later
/// frame could tell 32 attempts from 31; thirty-two frames of exactly the
/// cap spend the burst to the last byte.)
#[test]
fn d4_shared_byte_bucket_spends() {
    // Case: 32 capped frames spend the whole burst.
    {
        let (mut rig, p) = quiet_lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let valid = halt_bytes(epoch, 1, 1);
        assert_eq!(valid.len(), 21);
        let mut padded = valid.clone();
        padded.resize(4_096, 0);
        let state = state_of(&mut rig, &[p]);
        for _ in 0..32 {
            assert_eq!(
                rig.ingest_move(p, &padded, 5),
                MoveAdmission::Refused {
                    seq: None,
                    code: CodeV3::Malformed,
                }
            );
        }
        assert_unchanged(&mut rig, &state, &[p]);
        // The byte bucket is empty: the next valid frame is `RateLimited`.
        assert_eq!(
            rig.ingest_move(p, &valid, 5),
            MoveAdmission::Refused {
                seq: None,
                code: CodeV3::RateLimited,
            }
        );
    }
    // Control: 31 such frames leave room, so the next valid frame is admitted.
    {
        let (mut rig, p) = quiet_lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let valid = halt_bytes(epoch, 1, 1);
        let mut padded = valid.clone();
        padded.resize(4_096, 0);
        for _ in 0..31 {
            assert_eq!(
                rig.ingest_move(p, &padded, 5),
                MoveAdmission::Refused {
                    seq: None,
                    code: CodeV3::Malformed,
                }
            );
        }
        assert_eq!(
            rig.ingest_move(p, &valid, 5),
            MoveAdmission::Admitted { seq: 1 }
        );
    }
}

// ---------------------------------------------------------------------------
// D5.
// ---------------------------------------------------------------------------

/// D5: each adjacent precedence pair, both faults present, and the pair that
/// cannot be built is named.
#[test]
fn d5_precedence_pairs() {
    // Closed beats a regressed `now_ms`: the clock is advanced first, so the
    // regression is real and only `Closed` can answer it.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        assert_eq!(rig.host.pause(10), Ok(()));
        rig.host.shutdown();
        assert_eq!(
            rig.host.ingest_move(p, &halt_bytes(epoch, 1, 1), 0),
            Err(HostError::Closed)
        );
    }
    // Control: the same regression alone, on an open host, is
    // `TimeRegression`.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        assert_eq!(rig.host.pause(10), Ok(()));
        assert_eq!(
            rig.host.ingest_move(p, &halt_bytes(epoch, 1, 1), 0),
            Err(HostError::TimeRegression)
        );
    }
    // A regression beats an unbound handle.
    {
        let mut rig = Rig::new(&mv3rt_value());
        rig.bind_nav();
        let p = rig.bind_p();
        rig.host.unbind_peer(p);
        assert_eq!(rig.host.pause(10), Ok(()));
        assert_eq!(
            rig.host.ingest_move(p, &halt_bytes([0; 16], 1, 1), 5),
            Err(HostError::TimeRegression)
        );
    }
    // Spent intent buckets beat the frame cap.
    {
        let (mut rig, p) = quiet_lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        for _ in 0..80 {
            assert_eq!(
                rig.ingest_move(p, &[0xA0], 5),
                MoveAdmission::Refused {
                    seq: None,
                    code: CodeV3::UnsupportedVersion,
                }
            );
        }
        let mut oversized = halt_bytes(epoch, 1, 1);
        oversized.resize(4_097, 0);
        assert_eq!(
            rig.ingest_move(p, &oversized, 5),
            MoveAdmission::Refused {
                seq: None,
                code: CodeV3::RateLimited,
            }
        );
    }
    // The frame cap beats full ingress.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let mut now = 5;
        for seq in 1..=128u64 {
            let frame = codec_v4::encode_intent(&IntentFrameV4 {
                epoch,
                lane: LANE_COMBAT,
                seq,
                observed_tick: 0,
                actor: n(1),
                body: IntentBodyV4::DeclareAction {
                    ability: strike(),
                    target: n(2),
                },
            })
            .expect("encodes");
            assert_eq!(
                rig.host.ingest(p, &frame, now),
                Ok(crpg_server::host::IngestDisposition::Staged)
            );
            now += 25;
        }
        let mut oversized = halt_bytes(epoch, 1, 1);
        oversized.resize(4_097, 0);
        assert_eq!(
            rig.ingest_move(p, &oversized, now),
            MoveAdmission::Refused {
                seq: None,
                code: CodeV3::FrameTooLarge,
            }
        );
        // Full ingress then beats the version byte.
        assert_eq!(
            rig.ingest_move(p, &[0xA0], now),
            MoveAdmission::Refused {
                seq: None,
                code: CodeV3::QueueFull,
            }
        );
    }
    // The first byte beats the epoch.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let mut bad = halt_bytes([9; 16], 1, 1);
        bad[0] = 0xA0;
        assert_eq!(
            rig.ingest_move(p, &bad, 10),
            MoveAdmission::Refused {
                seq: None,
                code: CodeV3::UnsupportedVersion,
            }
        );
    }
    // The epoch beats a cached seq with other bytes: `SessionExpired`, and no
    // fence.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 1, 1), 10),
            MoveAdmission::Admitted { seq: 1 }
        );
        rig.mirror_halt(a());
        rig.assert_ticked(10);
        assert_eq!(rig.receipts(p).len(), 1);
        assert_eq!(
            rig.ingest_move(p, &move_to_bytes([9; 16], 1, 1, (262144, 0, 65536)), 20),
            MoveAdmission::Refused {
                seq: Some(1),
                code: CodeV3::SessionExpired,
            }
        );
        assert_eq!(rig.host.session_wire(p), Ok(4));
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 1, 1), 20),
            MoveAdmission::Duplicate {
                seq: 1,
                resent: true,
            }
        );
    }
    // A pending seq below `next_seq` is `Duplicate`, not `StaleSeq`.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 1, 1), 10),
            MoveAdmission::Admitted { seq: 1 }
        );
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 2, 1), 10),
            MoveAdmission::Admitted { seq: 2 }
        );
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 1, 1), 10),
            MoveAdmission::Duplicate {
                seq: 1,
                resent: false,
            }
        );
    }
    // The sequence range beats the (then empty) move bucket.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let mut now = 5;
        for window in 0..33u64 {
            let count = if window == 32 { 1 } else { 8 };
            for step in 0..count {
                let seq = window * 8 + step + 1;
                assert_eq!(
                    rig.ingest_move(p, &halt_bytes(epoch, seq, 1), now),
                    MoveAdmission::Admitted { seq }
                );
            }
            rig.mirror_halt(a());
            rig.assert_ticked(now);
            assert_eq!(rig.receipts(p).len() as u64, count);
            now += 400;
        }
        // Spend the rest of the bucket at one `now_ms`.
        for window in 0..5u64 {
            for step in 0..8u64 {
                let seq = 258 + window * 8 + step;
                assert_eq!(
                    rig.ingest_move(p, &halt_bytes(epoch, seq, 1), now),
                    MoveAdmission::Admitted { seq }
                );
            }
            rig.mirror_halt(a());
            rig.assert_ticked(now);
            assert_eq!(rig.receipts(p).len(), 8);
        }
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 1, 1), now),
            MoveAdmission::Refused {
                seq: Some(1),
                code: CodeV3::StaleSeq,
            }
        );
    }
    // The move bucket beats the in-flight window.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let now = 5;
        for window in 0..4u64 {
            for step in 0..8u64 {
                let seq = window * 8 + step + 1;
                assert_eq!(
                    rig.ingest_move(p, &halt_bytes(epoch, seq, 1), now),
                    MoveAdmission::Admitted { seq }
                );
            }
            rig.mirror_halt(a());
            rig.assert_ticked(now);
            assert_eq!(rig.receipts(p).len(), 8);
        }
        for step in 0..8u64 {
            let seq = 33 + step;
            assert_eq!(
                rig.ingest_move(p, &halt_bytes(epoch, seq, 1), now),
                MoveAdmission::Admitted { seq }
            );
        }
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 41, 1), now),
            MoveAdmission::Refused {
                seq: Some(41),
                code: CodeV3::RateLimited,
            }
        );
    }
    // The in-flight window beats actor resolution.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        for seq in 1..=8 {
            assert_eq!(
                rig.ingest_move(p, &halt_bytes(epoch, seq, 1), 5),
                MoveAdmission::Admitted { seq }
            );
        }
        assert_eq!(
            rig.ingest_move(p, &move_to_bytes(epoch, 9, 9, (262144, 0, 65536)), 5),
            MoveAdmission::Refused {
                seq: Some(9),
                code: CodeV3::QueueFull,
            }
        );
    }
    // Not buildable: unbound with empty intent buckets (no session, no
    // buckets); `SeqConflict` with `SeqExhausted` and `SeqGap` with `StaleSeq`
    // need 2^64 intents or one value to be two.
}

/// D-1 step 6: the ingress-room checks other than per-peer frames.
/// `POLICY_INGRESS_FRAMES_PER_PEER` − 1 staged lane-0 frames still admit the
/// next lane-1 frame; `POLICY_INGRESS_BYTES_PER_PEER` staged bytes refuse it
/// with `QueueFull`. (The eight-peer host caps need eight saturated peers;
/// skipped as too costly for the same code path.)
#[test]
fn d6_ingress_room_frames_and_bytes() {
    // 127 staged frames admit the next.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let mut now = 5;
        for seq in 1..=(POLICY_INGRESS_FRAMES_PER_PEER as u64 - 1) {
            let frame = codec_v4::encode_intent(&IntentFrameV4 {
                epoch,
                lane: LANE_COMBAT,
                seq,
                observed_tick: 0,
                actor: n(1),
                body: IntentBodyV4::DeclareAction {
                    ability: strike(),
                    target: n(2),
                },
            })
            .expect("encodes");
            assert_eq!(
                rig.host.ingest(p, &frame, now),
                Ok(crpg_server::host::IngestDisposition::Staged)
            );
            now += 25;
        }
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 1, 1), now),
            MoveAdmission::Admitted { seq: 1 }
        );
    }
    // About 256 KiB staged refuse the next with `QueueFull`.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let staged = vec![0xABu8; 4_096];
        let mut now = 5;
        for _ in 0..(POLICY_INGRESS_BYTES_PER_PEER / 4_096) {
            assert_eq!(
                rig.host.ingest(p, &staged, now),
                Ok(crpg_server::host::IngestDisposition::Staged)
            );
            now += 100;
        }
        let state = state_of(&mut rig, &[p]);
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 1, 1), now),
            MoveAdmission::Refused {
                seq: None,
                code: CodeV3::QueueFull,
            }
        );
        assert_unchanged(&mut rig, &state, &[p]);
    }
}

// ---------------------------------------------------------------------------
// D6.
// ---------------------------------------------------------------------------

/// D6: lane-1 attempts spend the shared intent buckets, and lane-1 rejection
/// receipts spend the shared failure buckets without ever being dropped.
#[test]
fn d6_aggregates() {
    // (a) 80 one-byte lane-1 frames empty the shared intent buckets.
    {
        let (mut rig, p) = quiet_lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        for _ in 0..80 {
            assert_eq!(
                rig.ingest_move(p, &[0xA0], 5),
                MoveAdmission::Refused {
                    seq: None,
                    code: CodeV3::UnsupportedVersion,
                }
            );
        }
        let frame = codec_v4::encode_intent(&IntentFrameV4 {
            epoch,
            lane: LANE_COMBAT,
            seq: 1,
            observed_tick: 0,
            actor: n(1),
            body: IntentBodyV4::EndTurn,
        })
        .expect("encodes");
        match rig.host.ingest(p, &frame, 5) {
            Ok(IngestDisposition::Refused { status }) => {
                assert_eq!(
                    status,
                    ReceiptStatus::Rejected(crpg_net::protocol::RejectionCode::RateLimited)
                )
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }
    // (b) Ten lane-1 rejections spend ten failure tokens; the eleventh lane-1
    // receipt still arrives; the next lane-0 rejection is dropped.
    {
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        for seq in 1..=10u64 {
            assert_eq!(
                rig.ingest_move(p, &move_to_bytes(epoch, seq, 2, (65536, 0, 262144)), 5),
                MoveAdmission::Rejected {
                    seq,
                    code: CodeV3::NotAuthorized,
                }
            );
            assert_eq!(
                rig.receipts(p),
                vec![MoveReceipt {
                    epoch,
                    seq,
                    actor: n(2),
                    processed_tick: 0,
                    outcome: MoveVerdict::Rejected(CodeV3::NotAuthorized),
                }]
            );
        }
        // The eleventh still gets its receipt.
        assert_eq!(
            rig.ingest_move(p, &move_to_bytes(epoch, 11, 2, (65536, 0, 262144)), 5),
            MoveAdmission::Rejected {
                seq: 11,
                code: CodeV3::NotAuthorized,
            }
        );
        assert_eq!(rig.receipts(p).len(), 1);

        let frame = codec_v4::encode_intent(&IntentFrameV4 {
            epoch,
            lane: LANE_COMBAT,
            seq: 1,
            observed_tick: 0,
            actor: n(2),
            body: IntentBodyV4::DeclareAction {
                ability: strike(),
                target: n(1),
            },
        })
        .expect("encodes");
        assert_eq!(
            rig.host.ingest(p, &frame, 5),
            Ok(crpg_server::host::IngestDisposition::Staged)
        );
        let summary = rig.host.pump(5).expect("pump");
        assert_eq!(summary.dropped_replies, 1, "{summary:?}");
        assert_eq!(summary.rejected, 1);

        // Control: no lane-1 traffic, so the failure budget holds.
        let mut twin = Rig::new(&mv3rt_value());
        twin.bind_nav();
        let q = twin.bind_p();
        twin.publish(q);
        let epoch = twin.epoch(q);
        let frame = codec_v4::encode_intent(&IntentFrameV4 {
            epoch,
            lane: LANE_COMBAT,
            seq: 1,
            observed_tick: 0,
            actor: n(2),
            body: IntentBodyV4::DeclareAction {
                ability: strike(),
                target: n(1),
            },
        })
        .expect("encodes");
        assert_eq!(
            twin.host.ingest(q, &frame, 5),
            Ok(crpg_server::host::IngestDisposition::Staged)
        );
        let summary = twin.host.pump(5).expect("pump");
        assert_eq!(summary.dropped_replies, 0, "{summary:?}");
        assert_eq!(summary.rejected, 1);
    }
}

/// D-3, D-8, Reading 15: only `Rejected` lane-1 receipts spend the shared
/// failure budget. Ten `Accepted`/`Superseded` receipts released at one
/// `now_ms` leave the budget full, so the next lane-0 rejection is kept;
/// ten `Rejected` receipts empty it, so the same rejection is dropped.
#[test]
fn d6_accepted_receipts_spare_failure_budget() {
    // Ten `Accepted`/`Superseded` receipts, all released at `now_ms` 5.
    let (mut rig, p) = lane1_rig(&mv3rt_value());
    let epoch = rig.epoch(p);
    // Window 1: two `Halt A` intents; the earlier one is superseded.
    assert_eq!(
        rig.ingest_move(p, &halt_bytes(epoch, 1, 1), 5),
        MoveAdmission::Admitted { seq: 1 }
    );
    assert_eq!(
        rig.ingest_move(p, &halt_bytes(epoch, 2, 1), 5),
        MoveAdmission::Admitted { seq: 2 }
    );
    rig.mirror_halt(a());
    rig.assert_ticked(5);
    assert_eq!(
        rig.receipts(p),
        vec![
            MoveReceipt {
                epoch,
                seq: 1,
                actor: n(1),
                processed_tick: 0,
                outcome: MoveVerdict::Superseded,
            },
            MoveReceipt {
                epoch,
                seq: 2,
                actor: n(1),
                processed_tick: 0,
                outcome: MoveVerdict::Accepted { start_tick: 1 },
            },
        ]
    );
    // Window 2: six more `Halt A` intents in one window; only the last wins.
    for seq in 3..=8u64 {
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, seq, 1), 5),
            MoveAdmission::Admitted { seq }
        );
    }
    rig.mirror_halt(a());
    rig.assert_ticked(5);
    let got = rig.receipts(p);
    assert_eq!(got.len(), 6);
    for (i, receipt) in got.iter().enumerate() {
        let seq = i as u64 + 3;
        assert_eq!(receipt.epoch, epoch);
        assert_eq!(receipt.seq, seq);
        assert_eq!(receipt.actor, n(1));
        assert_eq!(receipt.processed_tick, 1);
        assert_eq!(
            receipt.outcome,
            if seq == 8 {
                MoveVerdict::Accepted { start_tick: 2 }
            } else {
                MoveVerdict::Superseded
            }
        );
    }
    // Window 3: two more, the earlier superseded: ten receipts in all, none
    // of them `Rejected`.
    for seq in 9..=10u64 {
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, seq, 1), 5),
            MoveAdmission::Admitted { seq }
        );
    }
    rig.mirror_halt(a());
    rig.assert_ticked(5);
    assert_eq!(
        rig.receipts(p),
        vec![
            MoveReceipt {
                epoch,
                seq: 9,
                actor: n(1),
                processed_tick: 2,
                outcome: MoveVerdict::Superseded,
            },
            MoveReceipt {
                epoch,
                seq: 10,
                actor: n(1),
                processed_tick: 2,
                outcome: MoveVerdict::Accepted { start_tick: 3 },
            },
        ]
    );
    // The lane-0 rejection that follows is kept: the failure budget is full.
    let frame = codec_v4::encode_intent(&IntentFrameV4 {
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: n(2),
        body: IntentBodyV4::DeclareAction {
            ability: strike(),
            target: n(1),
        },
    })
    .expect("encodes");
    assert_eq!(
        rig.host.ingest(p, &frame, 5),
        Ok(crpg_server::host::IngestDisposition::Staged)
    );
    let summary = rig.host.pump(5).expect("pump");
    assert_eq!(summary.dropped_replies, 0, "{summary:?}");
    assert_eq!(summary.rejected, 1, "{summary:?}");

    // Control: ten `Rejected` receipts at one `now_ms` empty the budget, so
    // the same lane-0 rejection is dropped.
    let (mut control, q) = lane1_rig(&mv3rt_value());
    let epoch = control.epoch(q);
    for seq in 1..=10u64 {
        assert_eq!(
            control.ingest_move(q, &move_to_bytes(epoch, seq, 2, (65536, 0, 262144)), 5),
            MoveAdmission::Rejected {
                seq,
                code: CodeV3::NotAuthorized,
            }
        );
        assert_eq!(control.receipts(q).len(), 1);
    }
    let frame = codec_v4::encode_intent(&IntentFrameV4 {
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: n(2),
        body: IntentBodyV4::DeclareAction {
            ability: strike(),
            target: n(1),
        },
    })
    .expect("encodes");
    assert_eq!(
        control.host.ingest(q, &frame, 5),
        Ok(crpg_server::host::IngestDisposition::Staged)
    );
    let summary = control.host.pump(5).expect("pump");
    assert_eq!(summary.dropped_replies, 1, "{summary:?}");
    assert_eq!(summary.rejected, 1, "{summary:?}");
}

/// D-3, D-8, Reading 15: a `Rejected` lane-1 receipt spends the failure
/// budget refilled since. Ten rejections empty the ten-token budget at
/// `now_ms` 5; 100 ms later exactly one token has refilled, so one
/// `Rejected` resend spends it and the next lane-0 rejection is dropped.
/// Without that resend the refilled token keeps the rejection.
#[test]
fn d8_rejected_resend_spends_refilled_token() {
    let (mut rig, p) = lane1_rig(&mv3rt_value());
    let epoch = rig.epoch(p);
    for seq in 1..=10u64 {
        assert_eq!(
            rig.ingest_move(p, &move_to_bytes(epoch, seq, 2, (65536, 0, 262144)), 5),
            MoveAdmission::Rejected {
                seq,
                code: CodeV3::NotAuthorized,
            }
        );
        assert_eq!(rig.receipts(p).len(), 1);
    }
    // One refill token 100 ms later; the cached `Rejected` resend spends it.
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch, 10, 2, (65536, 0, 262144)), 105),
        MoveAdmission::Duplicate {
            seq: 10,
            resent: true,
        }
    );
    assert_eq!(
        rig.receipts(p),
        vec![MoveReceipt {
            epoch,
            seq: 10,
            actor: n(2),
            processed_tick: 0,
            outcome: MoveVerdict::Rejected(CodeV3::NotAuthorized),
        }]
    );
    // The lane-0 rejection that follows is dropped: the budget is empty.
    let frame = codec_v4::encode_intent(&IntentFrameV4 {
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: n(2),
        body: IntentBodyV4::DeclareAction {
            ability: strike(),
            target: n(1),
        },
    })
    .expect("encodes");
    assert_eq!(
        rig.host.ingest(p, &frame, 105),
        Ok(crpg_server::host::IngestDisposition::Staged)
    );
    let summary = rig.host.pump(105).expect("pump");
    assert_eq!(summary.dropped_replies, 1, "{summary:?}");
    assert_eq!(summary.rejected, 1, "{summary:?}");

    // Control: the same run without the resend keeps its one refilled token,
    // so the same lane-0 rejection is not dropped.
    let (mut control, q) = lane1_rig(&mv3rt_value());
    let epoch = control.epoch(q);
    for seq in 1..=10u64 {
        assert_eq!(
            control.ingest_move(q, &move_to_bytes(epoch, seq, 2, (65536, 0, 262144)), 5),
            MoveAdmission::Rejected {
                seq,
                code: CodeV3::NotAuthorized,
            }
        );
        assert_eq!(control.receipts(q).len(), 1);
    }
    let frame = codec_v4::encode_intent(&IntentFrameV4 {
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: n(2),
        body: IntentBodyV4::DeclareAction {
            ability: strike(),
            target: n(1),
        },
    })
    .expect("encodes");
    assert_eq!(
        control.host.ingest(q, &frame, 105),
        Ok(crpg_server::host::IngestDisposition::Staged)
    );
    let summary = control.host.pump(105).expect("pump");
    assert_eq!(summary.dropped_replies, 0, "{summary:?}");
    assert_eq!(summary.rejected, 1, "{summary:?}");
}

/// D-1 step 6 before HM-2 step 1 on a v3 session: with full ingress even a
/// version-mismatched frame is `QueueFull`; with room a lane-byte-0 frame is
/// `UnsupportedVersion`, because decode (which would say `WrongDirection`)
/// is never reached.
#[test]
fn d5_v3_queue_full_before_version() {
    // Full ingress: 128 staged lane-0 frames, one per 25 ms.
    let mut rig = Rig::new(&mv3rt_value());
    rig.bind_nav();
    let q = rig.bind_v3();
    let epoch = rig.epoch(q);
    let mut now = 5;
    for seq in 1..=128u64 {
        let frame = codec_v4::encode_intent(&IntentFrameV4 {
            epoch,
            lane: LANE_COMBAT,
            seq,
            observed_tick: 0,
            actor: n(1),
            body: IntentBodyV4::DeclareAction {
                ability: strike(),
                target: n(2),
            },
        })
        .expect("encodes");
        assert_eq!(
            rig.host.ingest(q, &frame, now),
            Ok(crpg_server::host::IngestDisposition::Staged)
        );
        now += 25;
    }
    let mut bad = halt_bytes(epoch, 1, 1);
    bad[17] = 0;
    let state = state_of(&mut rig, &[q]);
    assert_eq!(
        rig.ingest_move(q, &bad, now),
        MoveAdmission::Refused {
            seq: None,
            code: CodeV3::QueueFull,
        }
    );
    assert_unchanged(&mut rig, &state, &[q]);
    // Control: with room the same bytes are `UnsupportedVersion` ...
    let mut control = Rig::new(&mv3rt_value());
    control.bind_nav();
    let r = control.bind_v3();
    let epoch = control.epoch(r);
    let mut bad = halt_bytes(epoch, 1, 1);
    bad[17] = 0;
    assert_eq!(
        control.ingest_move(r, &bad, 10),
        MoveAdmission::Refused {
            seq: None,
            code: CodeV3::UnsupportedVersion,
        }
    );
    // ... while a v4 session with room reaches decode: `WrongDirection`.
    let (mut v4, p) = lane1_rig(&mv3rt_value());
    let epoch = v4.epoch(p);
    let mut bad = halt_bytes(epoch, 1, 1);
    bad[17] = 0;
    assert_eq!(
        v4.ingest_move(p, &bad, 10),
        MoveAdmission::Refused {
            seq: None,
            code: CodeV3::WrongDirection,
        }
    );
}

// ---------------------------------------------------------------------------
// D7.
// ---------------------------------------------------------------------------

/// D7 (HM-10): a lane-0 retry interleaved with lane-1 traffic is answered from
/// the lane-0 cache, its effect happens once, and the lane-1 move neither
/// blocks nor doubles it.
#[test]
fn d7_no_double_spend() {
    let mut rig = Rig::new(&mv3rt_value());
    rig.bind_nav();
    let p = rig.bind_p();
    let mut client = rig.publish(p);
    let epoch = rig.epoch(p);
    let action = codec_v4::encode_intent(&IntentFrameV4 {
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: n(1),
        body: IntentBodyV4::DeclareAction {
            ability: strike(),
            target: n(2),
        },
    })
    .expect("encodes");
    assert_eq!(
        rig.host.ingest(p, &action, 10),
        Ok(crpg_server::host::IngestDisposition::Staged)
    );
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch, 1, 1, (262144, 0, 65536)), 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    assert_eq!(
        rig.host.ingest(p, &action, 10),
        Ok(crpg_server::host::IngestDisposition::Staged)
    );
    let summary = rig.host.pump(10).expect("pump");
    assert_eq!(summary.admitted, 2, "{summary:?}");
    assert_eq!(
        summary
            .results
            .iter()
            .map(|result| result.status)
            .collect::<Vec<_>>(),
        vec![ReceiptStatus::Applied, ReceiptStatus::Applied]
    );
    assert!(summary.results[1].cached);

    // Mirror: the one command, then the decision's route, then the tick.
    rig.mirror_perform(&CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    });
    rig.mirror_route(a(), m(4, 1), 1);
    rig.assert_ticked(20);
    assert_eq!(
        rig.receipts(p),
        vec![MoveReceipt {
            epoch,
            seq: 1,
            actor: n(1),
            processed_tick: 0,
            outcome: MoveVerdict::Accepted { start_tick: 1 },
        }]
    );

    // The tick's host record holds exactly one `ActionResolved`.
    let entries = rig.host.read_capture_entries(0, 256).expect("readable");
    let record = entries
        .iter()
        .rev()
        .find_map(|entry| match entry {
            CaptureEntry::Host(record) => Some(record),
            CaptureEntry::Command(_) => None,
        })
        .expect("a host record");
    assert_eq!(
        record
            .events
            .iter()
            .filter(|envelope| matches!(envelope.payload, HistoryEvent::ActionResolved { .. }))
            .count(),
        1,
        "{:?}",
        record.events
    );
    assert!(record.events.iter().any(|envelope| matches!(
        &envelope.payload,
        HistoryEvent::ActionResolved { actor, target, ability, outcome, damage }
            if *actor == a()
                && *target == b()
                && *ability == strike()
                && outcome == "success"
                && *damage == 3
    )));

    // The page order is the lane-0 receipt, then the tick's page with B at 7.
    client.drain(&mut rig.host, p, rig.now);
    assert_eq!(
        client
            .replica
            .v3()
            .entity(n(2))
            .and_then(|entry| entry.health.as_ref())
            .map(|health| health.health),
        Some(7)
    );
    assert_eq!(
        rig.samples(p),
        vec![MoveSample {
            server_tick: 1,
            processed_seq: 1,
            entries: vec![SampleEntry {
                entity: n(1),
                point: pt(131072, 0, 65536),
                moving: true,
            }],
        }]
    );

    // Control: the lane-0 calls alone give the same journal and B health.
    let mut twin = Rig::new(&mv3rt_value());
    twin.bind_nav();
    let q = twin.bind_p();
    let mut twin_client = twin.publish(q);
    let epoch = twin.epoch(q);
    let action = codec_v4::encode_intent(&IntentFrameV4 {
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: n(1),
        body: IntentBodyV4::DeclareAction {
            ability: strike(),
            target: n(2),
        },
    })
    .expect("encodes");
    assert_eq!(
        twin.host.ingest(q, &action, 10),
        Ok(crpg_server::host::IngestDisposition::Staged)
    );
    assert_eq!(
        twin.host.ingest(q, &action, 10),
        Ok(crpg_server::host::IngestDisposition::Staged)
    );
    twin.host.pump(10).expect("pump");
    twin.mirror_perform(&CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    });
    twin.assert_ticked(20);
    twin_client.drain(&mut twin.host, q, twin.now);
    assert_eq!(
        twin_client
            .replica
            .v3()
            .entity(n(2))
            .and_then(|entry| entry.health.as_ref())
            .map(|health| health.health),
        Some(7)
    );
    let lane0_only = twin.host.read_capture_entries(0, 256).expect("readable");
    let with_lane1 = rig.host.read_capture_entries(0, 256).expect("readable");
    assert_eq!(with_lane1, lane0_only);
    rig.assert_authority();
}

// ---------------------------------------------------------------------------
// D8.
// ---------------------------------------------------------------------------

/// D8: an identical resend never re-decides; a cached receipt is queued again
/// when its egress room holds it, spending the failure budget for a rejection
/// and never dropped by it.
#[test]
fn d8_resend() {
    let (mut rig, p) = lane1_rig(&mv3rt_value());
    let epoch = rig.epoch(p);
    let intent = halt_intent(epoch, 1, 1);
    let bytes = bytes(&intent);
    assert_eq!(
        rig.ingest_move(p, &bytes, 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    rig.mirror_halt(a());
    rig.assert_ticked(10);
    let receipt = rig.receipts(p);
    assert_eq!(receipt.len(), 1);
    assert_eq!(
        rig.ingest_move(p, &bytes, 20),
        MoveAdmission::Duplicate {
            seq: 1,
            resent: true,
        }
    );
    assert_eq!(
        rig.ingest_move(p, &bytes, 20),
        MoveAdmission::Duplicate {
            seq: 1,
            resent: false,
        },
        "already queued"
    );
    assert_eq!(
        rig.receipts(p),
        vec![MoveReceipt {
            epoch,
            seq: 1,
            actor: n(1),
            processed_tick: 0,
            outcome: MoveVerdict::Accepted { start_tick: 1 },
        }],
        "the cached receipt, verbatim and only once"
    );

    // A pending seq resent: nothing is decided a second time.
    assert_eq!(
        rig.ingest_move(p, &halt_bytes(epoch, 2, 1), 20),
        MoveAdmission::Admitted { seq: 2 }
    );
    assert_eq!(
        rig.ingest_move(p, &halt_bytes(epoch, 2, 1), 20),
        MoveAdmission::Duplicate {
            seq: 2,
            resent: false,
        }
    );
    rig.mirror_halt(a());
    rig.assert_ticked(20);
    assert_eq!(rig.receipts(p).len(), 1);

    // A cached `Rejected` receipt spends the tenth failure token; seq 8 with
    // the buckets empty is still resent.
    let (mut rig, p) = lane1_rig(&mv3rt_value());
    let epoch = rig.epoch(p);
    for seq in 1..=9u64 {
        assert_eq!(
            rig.ingest_move(p, &move_to_bytes(epoch, seq, 2, (65536, 0, 262144)), 5),
            MoveAdmission::Rejected {
                seq,
                code: CodeV3::NotAuthorized,
            }
        );
        assert_eq!(rig.receipts(p).len(), 1);
    }
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch, 9, 2, (65536, 0, 262144)), 5),
        MoveAdmission::Duplicate {
            seq: 9,
            resent: true,
        }
    );
    assert_eq!(
        rig.receipts(p),
        vec![MoveReceipt {
            epoch,
            seq: 9,
            actor: n(2),
            processed_tick: 0,
            outcome: MoveVerdict::Rejected(CodeV3::NotAuthorized),
        }]
    );
    let frame = codec_v4::encode_intent(&IntentFrameV4 {
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: n(2),
        body: IntentBodyV4::DeclareAction {
            ability: strike(),
            target: n(1),
        },
    })
    .expect("encodes");
    assert_eq!(
        rig.host.ingest(p, &frame, 5),
        Ok(crpg_server::host::IngestDisposition::Staged)
    );
    assert_eq!(rig.host.pump(5).expect("pump").dropped_replies, 1);
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch, 8, 2, (65536, 0, 262144)), 5),
        MoveAdmission::Duplicate {
            seq: 8,
            resent: true,
        }
    );
    assert_eq!(
        rig.receipts(p),
        vec![MoveReceipt {
            epoch,
            seq: 8,
            actor: n(2),
            processed_tick: 0,
            outcome: MoveVerdict::Rejected(CodeV3::NotAuthorized),
        }]
    );

    // Control: a cached `Accepted` resend does not spend the failure budget,
    // so the same lane-0 rejection is not dropped.
    let (mut twin, q) = lane1_rig(&mv3rt_value());
    let epoch = twin.epoch(q);
    for seq in 1..=9u64 {
        assert_eq!(
            twin.ingest_move(q, &move_to_bytes(epoch, seq, 2, (65536, 0, 262144)), 5),
            MoveAdmission::Rejected {
                seq,
                code: CodeV3::NotAuthorized,
            }
        );
        assert_eq!(twin.receipts(q).len(), 1);
    }
    assert_eq!(
        twin.ingest_move(q, &halt_bytes(epoch, 10, 1), 5),
        MoveAdmission::Admitted { seq: 10 }
    );
    twin.mirror_halt(a());
    twin.assert_ticked(5);
    assert_eq!(twin.receipts(q).len(), 1);
    assert_eq!(
        twin.ingest_move(q, &halt_bytes(epoch, 10, 1), 5),
        MoveAdmission::Duplicate {
            seq: 10,
            resent: true,
        }
    );
    // Control: the same lane-0 rejection on a fresh lane 0 (`seq: 1`, not a
    // `SeqGap` refusal) is genuinely rejected and not dropped, because the
    // cached `Accepted` resend above spent no failure token.
    let frame = codec_v4::encode_intent(&IntentFrameV4 {
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: n(2),
        body: IntentBodyV4::DeclareAction {
            ability: strike(),
            target: n(1),
        },
    })
    .expect("encodes");
    assert_eq!(
        twin.host.ingest(q, &frame, 5),
        Ok(crpg_server::host::IngestDisposition::Staged)
    );
    let summary = twin.host.pump(5).expect("pump");
    assert_eq!(summary.dropped_replies, 0, "{summary:?}");
    assert_eq!(summary.rejected, 1, "{summary:?}");
}

/// D-2 step 3: duplicates compare canonical bytes. A cached `seq` resent
/// with a non-canonical but equivalent encoding is `Duplicate`; resent with
/// other content it is `SeqConflict` and fences the session.
#[test]
fn d2_canonical_duplicate_and_conflict() {
    // The same intent with `seq` 1 overlong (`0x81 0x00` for `0x01`; byte 18
    // is the varint `seq`) decodes to identical canonical bytes: `Duplicate`.
    let (mut rig, p) = lane1_rig(&mv3rt_value());
    let epoch = rig.epoch(p);
    let valid = halt_bytes(epoch, 1, 1);
    assert_eq!(
        rig.ingest_move(p, &valid, 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    rig.mirror_halt(a());
    rig.assert_ticked(10);
    assert_eq!(rig.receipts(p).len(), 1);
    let mut overlong = valid[..18].to_vec();
    overlong.extend_from_slice(&[0x81, 0x00]);
    overlong.extend_from_slice(&valid[19..]);
    assert_eq!(overlong.len(), 22);
    assert_eq!(
        rig.ingest_move(p, &overlong, 20),
        MoveAdmission::Duplicate {
            seq: 1,
            resent: true,
        }
    );
    assert_eq!(
        rig.ingest_move(p, &overlong, 20),
        MoveAdmission::Duplicate {
            seq: 1,
            resent: false,
        },
        "already queued"
    );
    assert_eq!(
        rig.receipts(p),
        vec![MoveReceipt {
            epoch,
            seq: 1,
            actor: n(1),
            processed_tick: 0,
            outcome: MoveVerdict::Accepted { start_tick: 1 },
        }],
        "the cached receipt, verbatim and only once"
    );
    // Control beside it: a cached `seq` with other content fences.
    let (mut rig, p) = lane1_rig(&mv3rt_value());
    let epoch = rig.epoch(p);
    assert_eq!(
        rig.ingest_move(p, &halt_bytes(epoch, 1, 1), 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    rig.mirror_halt(a());
    rig.assert_ticked(10);
    assert_eq!(rig.receipts(p).len(), 1);
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch, 1, 1, (262144, 0, 65536)), 20),
        MoveAdmission::Refused {
            seq: Some(1),
            code: CodeV3::SeqConflict,
        }
    );
    assert_eq!(rig.host.session_wire(p), Err(HostError::UnknownPeer));
}

// ---------------------------------------------------------------------------
// D9.
// ---------------------------------------------------------------------------

/// D9: `take_move_receipts`' precedence, and a fenced session's pending
/// intents are never decided.
#[test]
fn d9_lifecycle_and_errors() {
    let mut rig = Rig::new(&mv3rt_value());
    rig.bind_nav();
    let q = rig.bind_v3();
    drain(&mut rig.host, q);
    assert_eq!(rig.host.take_move_receipts(q), Err(HostError::NotV4Peer));
    let mut rig = Rig::new(&mv3rt_value());
    rig.bind_nav();
    let doomed = rig.bind_p();
    rig.host.unbind_peer(doomed);
    assert_eq!(
        rig.host.take_move_receipts(doomed),
        Err(HostError::UnknownPeer)
    );
    rig.host.shutdown();
    assert_eq!(
        rig.host.take_move_receipts(doomed),
        Err(HostError::UnknownPeer)
    );

    let mut rig = Rig::new(&mv3rt_value());
    rig.bind_nav();
    let p = rig.bind_p();
    rig.publish(p);
    let epoch = rig.epoch(p);
    assert_eq!(
        rig.ingest_move(p, &halt_bytes(epoch, 1, 1), 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch, 2, 1, (262144, 0, 65536)), 10),
        MoveAdmission::Admitted { seq: 2 }
    );
    state_of(&mut rig, &[p]);
    rig.host.unbind_peer(p);
    rig.assert_ticked(10);
    assert_eq!(
        rig.host.take_move_receipts(p),
        Err(HostError::UnknownPeer),
        "no receipt anywhere"
    );

    // Control: the same advance on a twin without those intents.
    let mut twin = Rig::new(&mv3rt_value());
    twin.bind_nav();
    let q = twin.bind_p();
    twin.publish(q);
    twin.assert_ticked(10);
    assert_eq!(rig.host.authority_hash(), twin.host.authority_hash());
}

// ---------------------------------------------------------------------------
// D10.
// ---------------------------------------------------------------------------

/// D10: lane-1 receipts share the egress caps with lane 0. Two reservations
/// fill P's frame budget, a third intent is `QueueFull`, an acknowledgement
/// frees the room, and the next window's page then does not fit, so P is
/// restaged; the control logs the page instead.
#[test]
fn d10_receipts_share_egress() {
    let mut rig = Rig::new(&mv3rt_value());
    rig.bind_nav();
    let p = rig.bind_p();
    rig.publish(p);
    let epoch = rig.epoch(p);
    for tick in 1..=126u64 {
        rig.assert_ticked(tick * 10);
    }
    let entries = rig.bind_log(p);
    assert_eq!(entries.len(), 126, "one page frame per tick");

    assert_eq!(
        rig.ingest_move(p, &halt_bytes(epoch, 1, 1), 2000),
        MoveAdmission::Admitted { seq: 1 }
    );
    assert_eq!(
        rig.ingest_move(p, &halt_bytes(epoch, 2, 1), 2000),
        MoveAdmission::Admitted { seq: 2 }
    );
    assert_eq!(
        rig.ingest_move(p, &halt_bytes(epoch, 3, 1), 2000),
        MoveAdmission::Refused {
            seq: Some(3),
            code: CodeV3::QueueFull,
        },
        "126 logged frames plus two reservations is the whole frame cap"
    );
    rig.host
        .acknowledge_delivery(p, entries[0].ordinal)
        .expect("acks");
    assert_eq!(
        rig.ingest_move(p, &halt_bytes(epoch, 3, 1), 2000),
        MoveAdmission::Admitted { seq: 3 }
    );

    rig.mirror_halt(a());
    rig.assert_ticked(2010);
    assert_eq!(rig.receipts(p).len(), 3);
    assert_eq!(
        rig.host.transfer_state(p),
        Ok(Some(TransferState {
            next_transfer: 3,
            in_flight: Some(2),
        })),
        "the tick's page no longer fits beside three receipts"
    );

    // Control: the same frames and acknowledgement, no lane-1 intent.
    let mut twin = Rig::new(&mv3rt_value());
    twin.bind_nav();
    let q = twin.bind_p();
    twin.publish(q);
    for tick in 1..=126u64 {
        twin.assert_ticked(tick * 10);
    }
    let entries = twin.bind_log(q);
    assert_eq!(entries.len(), 126);
    twin.host
        .acknowledge_delivery(q, entries[0].ordinal)
        .expect("acks");
    twin.assert_ticked(2010);
    assert_eq!(
        twin.host.transfer_state(q),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: None,
        })),
        "the page is logged and the session is not staged"
    );
}

// ---------------------------------------------------------------------------
// D11.
// ---------------------------------------------------------------------------

/// D11: the receipt queue never passes the egress frame cap; duplicates past
/// it are `resent: false`, and taking the receipts frees the room.
#[test]
fn d11_receipt_queue_bound() {
    let (mut rig, p) = lane1_rig(&mv3rt_value());
    let epoch = rig.epoch(p);
    let mut now = 5;
    let mut sent = Vec::new();
    for window in 0..17u64 {
        for step in 0..8u64 {
            let seq = window * 8 + step + 1;
            let bytes = halt_bytes(epoch, seq, 1);
            sent.push(bytes.clone());
            assert_eq!(
                rig.ingest_move(p, &bytes, now),
                MoveAdmission::Admitted { seq }
            );
        }
        rig.mirror_halt(a());
        rig.assert_ticked(now);
        assert_eq!(rig.receipts(p).len(), 8);
        drain(&mut rig.host, p);
        now += 400;
    }
    assert_eq!(sent.len(), 136);

    // 136 resends, one per 25 ms, without taking: 128 fit, the rest do not.
    let mut at = now;
    for (index, bytes) in sent.iter().enumerate() {
        let seq = index as u64 + 1;
        let expected = if index < 128 {
            MoveAdmission::Duplicate { seq, resent: true }
        } else {
            MoveAdmission::Duplicate { seq, resent: false }
        };
        assert_eq!(rig.ingest_move(p, bytes, at), expected);
        at += 25;
    }
    assert_eq!(rig.receipts(p).len(), 128);
    assert_eq!(
        rig.ingest_move(p, &sent[128], at),
        MoveAdmission::Duplicate {
            seq: 129,
            resent: true,
        },
        "the take freed one frame"
    );
}

// ---------------------------------------------------------------------------
// D12.
// ---------------------------------------------------------------------------

/// D12: lane 1 moves only an entity the peer may name as an actor (mapped,
/// granted `present && action_actor`, controlled); each refusal is
/// `NotAuthorized` with its receipt and the world unchanged, beside a control
/// that is admitted and accepted.
#[test]
fn d12_actor_resolution() {
    for body in 0..2 {
        // A never-minted actor.
        let (mut rig, p) = lane1_rig(&mv3rt_value());
        let epoch = rig.epoch(p);
        let bad = if body == 0 {
            move_to_bytes(epoch, 1, 9, (262144, 0, 65536))
        } else {
            halt_bytes(epoch, 1, 9)
        };
        let state = state_of(&mut rig, &[p]);
        assert_eq!(
            rig.ingest_move(p, &bad, 10),
            MoveAdmission::Rejected {
                seq: 1,
                code: CodeV3::NotAuthorized,
            }
        );
        assert_world_unchanged(&rig, &state);
        assert_eq!(
            rig.receipts(p),
            vec![MoveReceipt {
                epoch,
                seq: 1,
                actor: n(9),
                processed_tick: 0,
                outcome: MoveVerdict::Rejected(CodeV3::NotAuthorized),
            }]
        );
        // Control: the same intent by the controlled actor is accepted.
        let good = if body == 0 {
            move_to_bytes(epoch, 2, 1, (262144, 0, 65536))
        } else {
            halt_bytes(epoch, 2, 1)
        };
        assert_eq!(
            rig.ingest_move(p, &good, 10),
            MoveAdmission::Admitted { seq: 2 }
        );
        if body == 0 {
            rig.mirror_route(a(), m(4, 1), 2);
        } else {
            rig.mirror_halt(a());
        }
        rig.assert_ticked(10);
        assert_eq!(
            rig.receipts(p)[0].outcome,
            MoveVerdict::Accepted { start_tick: 1 }
        );

        // `A` granted with `action_actor: false`: not an actor it may name.
        let mut rig = Rig::new(&mv3rt_value());
        rig.bind_nav();
        let mut grants = full_grants(&[a(), b(), c()]);
        grants.entities[0].action_actor = false;
        let p = rig
            .host
            .bind_v4_peer(control(&[a()]), grants, facts(), r1(), 0)
            .expect("binds");
        rig.publish(p);
        let epoch = rig.epoch(p);
        let bad = if body == 0 {
            move_to_bytes(epoch, 1, 1, (262144, 0, 65536))
        } else {
            halt_bytes(epoch, 1, 1)
        };
        let state = state_of(&mut rig, &[p]);
        assert_eq!(
            rig.ingest_move(p, &bad, 10),
            MoveAdmission::Rejected {
                seq: 1,
                code: CodeV3::NotAuthorized,
            }
        );
        assert_world_unchanged(&rig, &state);
        assert_eq!(rig.receipts(p).len(), 1);
        // Control: with the flag set the same intent is admitted and accepted.
        let (mut twin, q) = lane1_rig(&mv3rt_value());
        let epoch = twin.epoch(q);
        let good = if body == 0 {
            move_to_bytes(epoch, 1, 1, (262144, 0, 65536))
        } else {
            halt_bytes(epoch, 1, 1)
        };
        assert_eq!(
            twin.ingest_move(q, &good, 10),
            MoveAdmission::Admitted { seq: 1 }
        );
        if body == 0 {
            twin.mirror_route(a(), m(4, 1), 1);
        } else {
            twin.mirror_halt(a());
        }
        twin.assert_ticked(10);
        assert_eq!(
            twin.receipts(q)[0].outcome,
            MoveVerdict::Accepted { start_tick: 1 }
        );

        // `n(2)`: admitted before the narrowing, refused after it retired the
        // id from the view (`set_grants` made B not `present`).
        let (mut rig, p) = lane1_rig_control(&mv3rt_value(), &[a(), b()]);
        let epoch = rig.epoch(p);
        let good = if body == 0 {
            move_to_bytes(epoch, 1, 2, (196608, 0, 196608))
        } else {
            halt_bytes(epoch, 1, 2)
        };
        assert_eq!(
            rig.ingest_move(p, &good, 10),
            MoveAdmission::Admitted { seq: 1 }
        );
        if body == 0 {
            rig.mirror_route(b(), m(3, 3), 1);
        } else {
            rig.mirror_halt(b());
        }
        rig.assert_ticked(10);
        assert_eq!(
            rig.receipts(p)[0].outcome,
            MoveVerdict::Accepted { start_tick: 1 }
        );
        assert_eq!(
            rig.host.set_grants(p, full_grants(&[a(), c()])),
            Ok(GrantUpdate::SnapshotStaged)
        );
        let bad = if body == 0 {
            move_to_bytes(epoch, 2, 2, (196608, 0, 196608))
        } else {
            halt_bytes(epoch, 2, 2)
        };
        let state = state_of(&mut rig, &[p]);
        assert_eq!(
            rig.ingest_move(p, &bad, 20),
            MoveAdmission::Rejected {
                seq: 2,
                code: CodeV3::NotAuthorized,
            }
        );
        assert_world_unchanged(&rig, &state);
        assert_eq!(rig.receipts(p).len(), 1);
        // Control: the still-controlled A is admitted after the narrowing.
        assert_eq!(
            rig.ingest_move(p, &halt_bytes(epoch, 3, 1), 20),
            MoveAdmission::Admitted { seq: 3 }
        );
    }
}

/// Gap 12 (mutant l03): the move bucket refills at 20 tokens per second, so
/// after the 40-burst is spent at one `now_ms` the 41st intent is
/// `RateLimited`, still `RateLimited` 49 ms later (49 ms refills 980
/// milli-tokens, less than one token), and admitted 50 ms later (one full
/// token). The admitted control then decides to `Accepted`.
#[test]
fn d4_move_bucket_refill_needs_a_full_period() {
    let (mut rig, p) = lane1_rig(&mv3rt_value());
    let epoch = rig.epoch(p);
    for window in 0..5u64 {
        for step in 0..8u64 {
            let seq = window * 8 + step + 1;
            assert_eq!(
                rig.ingest_move(p, &halt_bytes(epoch, seq, 1), 5),
                MoveAdmission::Admitted { seq }
            );
        }
        rig.mirror_halt(a());
        rig.assert_ticked(5);
        assert_eq!(rig.receipts(p).len(), 8);
    }
    assert_eq!(
        rig.ingest_move(p, &halt_bytes(epoch, 41, 1), 5),
        MoveAdmission::Refused {
            seq: Some(41),
            code: CodeV3::RateLimited,
        }
    );
    assert_eq!(
        rig.ingest_move(p, &halt_bytes(epoch, 41, 1), 54),
        MoveAdmission::Refused {
            seq: Some(41),
            code: CodeV3::RateLimited,
        },
        "49 ms refills less than one move token"
    );
    assert_eq!(
        rig.ingest_move(p, &halt_bytes(epoch, 41, 1), 55),
        MoveAdmission::Admitted { seq: 41 }
    );
    rig.mirror_halt(a());
    rig.assert_ticked(55);
    assert_eq!(
        rig.receipts(p)[0].outcome,
        MoveVerdict::Accepted { start_tick: 6 }
    );
}

/// Gap 15 (mutant d11): a resent receipt is appended behind the queue, not
/// put at the front. Taking seq 1, leaving seq 2 queued, then resending seq 1
/// yields release order `[2, 1]`.
#[test]
fn d11_resent_receipt_is_appended_behind_the_queue() {
    let (mut rig, p) = lane1_rig(&mv3rt_value());
    let epoch = rig.epoch(p);
    let first = halt_bytes(epoch, 1, 1);
    let second = halt_bytes(epoch, 2, 1);
    assert_eq!(
        rig.ingest_move(p, &first, 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    rig.mirror_halt(a());
    rig.assert_ticked(10);
    assert_eq!(
        rig.receipts(p)
            .iter()
            .map(|receipt| receipt.seq)
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert_eq!(
        rig.ingest_move(p, &second, 20),
        MoveAdmission::Admitted { seq: 2 }
    );
    rig.mirror_halt(a());
    rig.assert_ticked(20);
    // Seq 2's receipt is queued; resending seq 1 appends behind it.
    assert_eq!(
        rig.ingest_move(p, &first, 20),
        MoveAdmission::Duplicate {
            seq: 1,
            resent: true
        }
    );
    assert_eq!(
        rig.receipts(p)
            .iter()
            .map(|receipt| receipt.seq)
            .collect::<Vec<_>>(),
        vec![2, 1]
    );
}

/// Gap 14, part 1 (mutant w14, D-5): a direct `host.tick()` never decides.
/// The admitted intent stays pending through the tick (no receipt) and is
/// decided by the next `advance`, with `processed_tick` of the decision
/// point and `Accepted` starting on the following tick.
#[test]
fn tick_never_decides_lane1() {
    let (mut rig, p) = lane1_rig(&mv3rt_value());
    let epoch = rig.epoch(p);
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch, 1, 1, (262144, 0, 65536)), 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    rig.host.tick().expect("ticks without deciding");
    rig.mirror_tick();
    assert_eq!(rig.receipts(p), Vec::new(), "the tick decides nothing");
    rig.mirror_route(a(), m(4, 1), 1);
    rig.assert_ticked(20);
    assert_eq!(
        rig.receipts(p),
        vec![MoveReceipt {
            epoch,
            seq: 1,
            actor: n(1),
            processed_tick: 1,
            outcome: MoveVerdict::Accepted { start_tick: 2 },
        }]
    );
}

/// Gap 14, part 2 (mutant o01, D-7 (1)): in turn-based `MV3` a controlled
/// non-combatant mover is `Accepted`, beside a combatant control that is
/// `Rejected(WrongMode)`. Lane 1 is the real-time and exploration lane, so
/// the mode refusal applies to combatants only.
#[test]
fn non_combatant_mover_accepted_in_turn_based() {
    fn mv3_with_bystander() -> Value {
        let mut value = mv3_value();
        value["world"]["entities"]["slots"]
            .as_array_mut()
            .expect("entity slots")
            .push(json!({"generation": 1, "value": {}}));
        value["world"]["movers"]
            .as_array_mut()
            .expect("movers")
            .push(json!([
                {"index": 3, "generation": 1},
                {"position": [131072, 0, 131072], "speed": 65536, "step": 655360},
            ]));
        value["world"]["transforms"]
            .as_array_mut()
            .expect("transforms")
            .push(json!([
                {"index": 3, "generation": 1},
                {"position": [2.0, 0.0, 2.0], "velocity": [0.0, 0.0, 0.0]},
            ]));
        value
    }
    let mut rig = Rig::new(&mv3_with_bystander());
    rig.bind_nav();
    let bystander = eid(3);
    let p = rig
        .host
        .bind_v4_peer(
            control(&[a(), bystander]),
            full_grants(&[a(), b(), c(), bystander]),
            facts(),
            r1(),
            0,
        )
        .expect("binds");
    rig.publish(p);
    let epoch = rig.epoch(p);
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch, 1, 4, (262144, 0, 131072)), 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch, 2, 1, (262144, 0, 65536)), 10),
        MoveAdmission::Admitted { seq: 2 }
    );
    rig.mirror_route(bystander, m(4, 2), 1);
    rig.assert_ticked(10);
    assert_eq!(
        rig.receipts(p),
        vec![
            MoveReceipt {
                epoch,
                seq: 1,
                actor: n(4),
                processed_tick: 0,
                outcome: MoveVerdict::Accepted { start_tick: 1 },
            },
            MoveReceipt {
                epoch,
                seq: 2,
                actor: n(1),
                processed_tick: 0,
                outcome: MoveVerdict::Rejected(CodeV3::WrongMode),
            },
        ]
    );
}

/// Gap 13 (mutant e03, Reading 23, C-7): with 126 logged frames and two
/// lane-1 reservations the frame cap is full, so the next tick's page does
/// not fit and the session restages; its re-anchored tick sample is empty.
/// Control: the same 126 frames with the walk started by trusted `move_to`
/// (no lane-1 state) logs its page and queues the walking sample.
#[test]
fn samples_dropped_when_lane1_fills_egress() {
    // Test: 126 ticks, then two lane-1 `MoveTo` for A in one window.
    let mut rig = Rig::new(&mv3rt_value());
    rig.bind_nav();
    let p = rig.bind_p();
    rig.publish(p);
    let epoch = rig.epoch(p);
    for tick in 1..=126u64 {
        rig.assert_ticked(tick * 10);
    }
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch, 1, 1, (262144, 0, 65536)), 2000),
        MoveAdmission::Admitted { seq: 1 }
    );
    assert_eq!(
        rig.ingest_move(p, &move_to_bytes(epoch, 2, 1, (65536, 0, 262144)), 2000),
        MoveAdmission::Admitted { seq: 2 }
    );
    rig.mirror_route(a(), m(1, 4), 2);
    rig.assert_ticked(2010);
    assert_eq!(
        rig.receipts(p)
            .iter()
            .map(|receipt| (receipt.seq, receipt.outcome))
            .collect::<Vec<_>>(),
        vec![
            (1, MoveVerdict::Superseded),
            (2, MoveVerdict::Accepted { start_tick: 127 }),
        ]
    );
    assert_eq!(
        rig.host.transfer_state(p),
        Ok(Some(TransferState {
            next_transfer: 3,
            in_flight: Some(2),
        })),
        "the tick's page no longer fits beside two lane-1 frames"
    );
    assert_eq!(rig.samples(p), Vec::new());

    // Control: the same 126 frames, the same walk, no lane-1 state.
    let mut twin = Rig::new(&mv3rt_value());
    twin.bind_nav();
    let q = twin.bind_p();
    twin.publish(q);
    for tick in 1..=126u64 {
        twin.assert_ticked(tick * 10);
    }
    twin.host.move_to(a(), m(1, 4)).expect("trusted walk");
    twin.mirror_route(a(), m(1, 4), 0);
    twin.assert_ticked(2010);
    assert_eq!(
        twin.host.transfer_state(q),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: None,
        })),
        "the page is logged and the session is not staged"
    );
    assert_eq!(
        twin.samples(q),
        vec![MoveSample {
            server_tick: 127,
            processed_seq: 0,
            entries: vec![SampleEntry {
                entity: n(1),
                point: pt(65536, 0, 131072),
                moving: true,
            }],
        }]
    );
}
