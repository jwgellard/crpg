//! T037c acceptance suite (`tasks/T037.md`, tests C1 to C7; C8 is the unit
//! test in `src/host.rs`): the turn-based `DeclareMove`, its retry, and the
//! per-tick movement samples, public API only.
//!
//! Oracles are independent of the host: a test-owned mirror `HistoryWorld`
//! loaded from the same JSON and driven with the same public sim calls
//! (routes computed by calling `crpg_nav` directly on the same mesh), and
//! hand-written expected frames, samples and capture fields. The v4 client is
//! the T026p5 one (`ReassemblyPoolV4` feeding a `ReplicaWorldV4`), and a sample
//! is decoded with `movement_v4::decode_down`. The fixtures are
//! `tests/fixtures/three_combatants.json` as saved-game JSON, edited in memory
//! (no new fixture file):
//!
//! - `MV3`: A/B/C movers at `m(1, 1)`, `m(3, 1)`, `m(1, 5)`, their `f32`
//!   mirrors and the definition's movement rules (speed 1 m/tick, one `JY`
//!   unit per started 10 m); turn-based.
//! - `MV3rt`: the same edits on the real-time JSON.
//!
//! `P` is `bind_v4_peer([A], FULL(A,B,C), FACTS, R1, ..)` drained and
//! published unless a case says otherwise; `V` is a v2 `bind_peer` session and
//! `Q` a v3 `bind_v3_peer`. Every refusal is compared against the host state
//! taken just before the call and sits beside an accepted control.

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_nav::{bake_box, NavMesh, NavRect};
use crpg_net::codec_v2;
use crpg_net::codec_v4;
use crpg_net::facts_v3::EncounterGrants;
use crpg_net::movement_v4::{decode_down, MoveDown, MoveSample};
use crpg_net::pool_v4::{
    DeltaOutcome, GateMode, ReassemblyPoolV4, ReceiverIdV4, ReplicaUpdateV4, TransferLimitsV3,
    TransferOutcome,
};
use crpg_net::protocol::{self as protocol_v1, NetId, ReceiptStatus, RejectionCode, LANE_COMBAT};
use crpg_net::protocol_transfer::{TransferId, TransferUp};
use crpg_net::protocol_v3::{
    DeltaOp as DeltaOpV3, FactSet, OrderFact, PauseRights, PoolFact, ReceiptStatus as ReceiptV3,
    RejectionCode as CodeV3, StatFact, StatValue,
};
use crpg_net::protocol_v4::{
    DeltaFrame, DeltaOp, IntentBody as IntentBodyV4, IntentFrame as IntentFrameV4, Point,
    SessionEpoch,
};
use crpg_net::replica_v4::{PositionState, ReplicaWorldV4};
use crpg_net::transfer_v3;
use crpg_server::capture::{CaptureEntry, CapturedHealth, CapturedTurn, CapturedView};
use crpg_server::checkpoint::load_checkpoint;
use crpg_server::host::{
    AdmissionResult, ControlGrant, DeliveryEntry, DisclosureGrants, EntityDisclosure,
    EntityFactGrant, FactGrants, Host, HostConfig, HostError, IngestDisposition, PeerHandle,
    ProtocolSelection, TransferDisposition, DEFAULT_HOST_GRANTS, POLICY_SAMPLE_REFRESH_TICKS,
};
use crpg_sim::{
    history_hash, validate_move, CombatError, HistoryWorld, MoveError, MoveRoute, NavRevision,
    RouteTag,
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
fn pool_jy() -> Ulid {
    Ulid::from_u128(606)
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

/// `MoveRoute { R(mesh), tag, points }`.
fn route(mesh: &NavMesh, tag: u64, points: &[[Fx16_16; 3]]) -> MoveRoute {
    MoveRoute {
        revision: r(mesh),
        tag: RouteTag(tag),
        points: points.to_vec(),
    }
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

/// `FULL(es)`: every flag true for each entity in `es`.
fn full_grants(entities: &[EntityId]) -> DisclosureGrants {
    DisclosureGrants {
        entities: entities.iter().copied().map(disclosure).collect(),
        abilities: vec![
            Ulid::from_u128(603),
            Ulid::from_u128(604),
            Ulid::from_u128(605),
        ],
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

    fn mirror_ack(&mut self) {
        self.oracle
            .acknowledge(self.oracle.last_sequence())
            .expect("acknowledges its own range");
    }

    /// Binds `M10` on the host and its revision on the mirror.
    fn bind_nav(&mut self) {
        let mesh = m10();
        let cleared = self.host.bind_nav_mesh(mesh).expect("binds");
        assert_eq!(cleared.cleared, Vec::new());
        assert_eq!(cleared.revision, r(&m10()));
        self.oracle
            .bind_nav(r(&m10()))
            .expect("the mirror binds the revision");
        self.mirror_ack();
    }

    /// The mirror's `set_route`, acknowledged; its tag is `RouteTag(0)`.
    fn mirror_route(&mut self, actor: EntityId, points: &[[Fx16_16; 3]]) {
        let route = route(&m10(), 0, points);
        self.oracle
            .set_route(actor, route)
            .expect("the mirror accepts the route");
        self.mirror_ack();
    }

    /// The mirror's `perform_move`, acknowledged, its route tagged `tag`
    /// (0 for a trusted host route, the intent's `seq` for a `DeclareMove`).
    fn mirror_move(&mut self, actor: EntityId, points: &[[Fx16_16; 3]], tag: u64) {
        let route = route(&m10(), tag, points);
        self.oracle
            .perform_move(actor, &route)
            .expect("the mirror moves");
        self.mirror_ack();
    }

    fn mirror_tick(&mut self) {
        self.oracle.tick().expect("the mirror ticks");
        self.mirror_ack();
    }

    fn assert_authority(&self) {
        assert_eq!(self.host.authority_hash(), history_hash(&self.oracle));
    }

    /// `V`: a v2 `bind_peer` session with an empty control list.
    fn bind_v(&mut self) -> PeerHandle {
        let peer = self
            .host
            .bind_peer(control(&[]), full_grants(&[a(), b(), c()]), self.now)
            .expect("binds");
        self.drain(peer);
        peer
    }

    /// `P`: the v4 session under test, controlling `[A]`.
    fn bind_p(&mut self) -> PeerHandle {
        self.host
            .bind_v4_peer(
                control(&[a()]),
                full_grants(&[a(), b(), c()]),
                facts(),
                r1(),
                self.now,
            )
            .expect("binds")
    }

    /// `Q`: the same binding by `bind_v3_peer`.
    fn bind_q(&mut self) -> PeerHandle {
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

    fn drain(&mut self, peer: PeerHandle) -> Vec<DeliveryEntry> {
        drain(&mut self.host, peer)
    }

    /// One `DeclareMove` through `ingest` then `pump`, +100 ms.
    fn declare_move(
        &mut self,
        peer: PeerHandle,
        seq: u64,
        actor: u64,
        destination: (i32, i32, i32),
    ) -> AdmissionResult {
        let bytes = declare_move_bytes(&self.host, peer, seq, actor, destination);
        self.send(peer, &bytes)
    }

    /// The host's pause, at `now`.
    fn pause(&mut self, now: u64) {
        self.now = now;
        assert_eq!(self.host.pause(now), Ok(()));
    }

    /// The host's unpause, at `now`.
    fn unpause(&mut self, now: u64) {
        self.now = now;
        assert_eq!(self.host.unpause(now), Ok(()));
    }

    fn send(&mut self, peer: PeerHandle, bytes: &[u8]) -> AdmissionResult {
        assert_eq!(
            self.host.ingest(peer, bytes, self.now),
            Ok(IngestDisposition::Staged)
        );
        let summary = self.host.pump(self.now).expect("pump");
        assert_eq!(summary.results.len(), 1, "{summary:?}");
        summary.results[0].clone()
    }

    /// One `advance`, its tick mirrored.
    fn advance(&mut self, now: u64) {
        self.now = now;
        let report = self.host.advance(now).expect("advances");
        assert!(report.ticked, "{report:?}");
        self.mirror_tick();
        self.assert_authority();
    }

    /// One `advance` whose tick is mirrored by the caller (a paused one).
    fn advance_paused(&mut self, now: u64) {
        self.now = now;
        let report = self.host.advance(now).expect("advances");
        assert!(!report.ticked, "{report:?}");
    }

    fn save(&self) -> String {
        String::from_utf8(self.host.save_checkpoint().expect("saves")).expect("utf8")
    }

    fn entries(&self) -> Vec<CaptureEntry> {
        let mut all = Vec::new();
        let mut after = self.host.capture_acknowledged();
        loop {
            let page = self
                .host
                .read_capture_entries(after, 256)
                .expect("readable");
            if page.is_empty() {
                return all;
            }
            after = page.last().expect("non-empty").capture_seq();
            all.extend(page);
        }
    }
}

/// A refusal leaves all of this as it was.
#[derive(Debug, PartialEq, Eq)]
struct State {
    hash: [u8; 32],
    last_capture: u64,
    executions: u64,
    nav: Option<NavRevision>,
}

/// The state a refusal must leave as it was; the peers' delivery logs and
/// samples are emptied first, so only the receipt a rejection logs is new.
fn state_of(host: &mut Host, peers: &[PeerHandle]) -> State {
    for peer in peers {
        drain(host, *peer);
        assert_eq!(host.take_samples(*peer), Ok(Vec::new()));
    }
    State {
        hash: host.authority_hash(),
        last_capture: host.last_capture(),
        executions: host.executions(),
        nav: host.nav_revision(),
    }
}

fn assert_unchanged(host: &mut Host, before: &State) {
    assert_eq!(host.authority_hash(), before.hash);
    assert_eq!(host.last_capture(), before.last_capture);
    assert_eq!(host.executions(), before.executions);
    assert_eq!(host.nav_revision(), before.nav);
}

fn assert_combat_error(host: &Host, expected: CombatError) {
    assert_eq!(host.last_combat_error(), Some(expected));
}

/// Takes and acknowledges every pending entry of `peer`.
fn drain(host: &mut Host, peer: PeerHandle) -> Vec<DeliveryEntry> {
    let entries = host.take_delivery_entries(peer).expect("bound");
    if let Some(last) = entries.last() {
        host.acknowledge_delivery(peer, last.ordinal).expect("acks");
    }
    entries
}

// ---------------------------------------------------------------------------
// The v4 client (T026p5).
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Transfer(TransferOutcome),
    Delta(DeltaOutcome),
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

    fn drain(
        &mut self,
        host: &mut Host,
        peer: PeerHandle,
        now: u64,
    ) -> (Vec<DeliveryEntry>, Vec<Outcome>, Vec<DeltaFrame>) {
        let entries = host.take_delivery_entries(peer).expect("bound");
        let mut outcomes = Vec::new();
        for entry in &entries {
            outcomes.push(match entry.kind {
                crpg_server::host::DeliveryKind::Transfer => Outcome::Transfer(
                    self.pool
                        .offer_transfer(self.rid, &entry.bytes, now)
                        .expect("the gate accepts the frame"),
                ),
                crpg_server::host::DeliveryKind::Delta => Outcome::Delta(
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
        (entries, outcomes, deltas)
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

/// Drains and publishes a fresh v4 binding.
fn bind_client(host: &mut Host, peer: PeerHandle, now: u64) -> Client {
    let mut client = Client::new(host.epoch(peer).expect("bound"));
    let (_, _, deltas) = client.drain(host, peer, now);
    assert_eq!(deltas.len(), 1, "the anchoring page");
    assert_eq!(
        client.publish(host, peer, now),
        TransferDisposition::Published
    );
    client
}

// ---------------------------------------------------------------------------
// Intents, samples and expected ops.
// ---------------------------------------------------------------------------

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

fn declare_move_bytes(
    host: &Host,
    peer: PeerHandle,
    seq: u64,
    actor: u64,
    (x, y, z): (i32, i32, i32),
) -> Vec<u8> {
    v4_intent(
        host,
        peer,
        seq,
        actor,
        IntentBodyV4::DeclareMove {
            destination: pt(x, y, z),
        },
    )
}

/// Every sample datagram of `peer`, decoded.
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

fn receipt(epoch: [u8; 16], seq: u64, processed_tick: u64, status: ReceiptV3) -> DeltaOpV3 {
    DeltaOpV3::Receipt {
        epoch,
        lane: LANE_COMBAT,
        seq,
        processed_tick,
        status,
    }
}

fn moved(entity: u64, points: &[(i32, i32, i32)]) -> DeltaOp {
    DeltaOp::Moved {
        entity: n(entity),
        points: points.iter().map(|(x, y, z)| pt(*x, *y, *z)).collect(),
    }
}

fn pos(entity: u64, position: Option<(Point, bool)>) -> DeltaOp {
    DeltaOp::Position {
        entity: n(entity),
        position: position
            .map(|(point, moving)| crpg_net::protocol_v4::PositionFact { point, moving }),
    }
}

/// The fact set of one fixture row under `FACTS`: A's pool at `pools`, the
/// controlling peer's order `order`, `alpha` 8, no effects and no tags.
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

fn entry_ops(entry: &DeliveryEntry) -> Vec<DeltaOp> {
    codec_v4::decode_delta(&entry.bytes)
        .expect("a v4 delta")
        .ops
}

fn view(epoch: [u8; 16]) -> CapturedView {
    CapturedView {
        epoch,
        health: vec![CapturedHealth {
            entity: n(1),
            health: 10,
            max_health: 10,
            dead: false,
        }],
        turn: Some(CapturedTurn {
            active: Some(n(1)),
            round: 0,
        }),
    }
}

// ---------------------------------------------------------------------------
// C1.
// ---------------------------------------------------------------------------

/// C1: a `DeclareMove` pays, journals one `Moved`, records an outcome-less
/// command and publishes one page per session; the next tick samples the new
/// position.
#[test]
fn c1_declare_move_page() {
    let mut rig = Rig::new(&mv3_value());
    rig.bind_nav();
    let v = rig.bind_v();
    assert_eq!(rig.host.session_wire(v), Ok(2));
    let p = rig.bind_p();
    assert_eq!(rig.host.session_wire(p), Ok(4));
    let mut client = bind_client(&mut rig.host, p, rig.now);
    let epoch_p = rig.host.epoch(p).expect("bound");
    let epoch_v = rig.host.epoch(v).expect("bound");

    let bytes = declare_move_bytes(&rig.host, p, 1, 1, (262144, 0, 65536));
    assert_eq!(
        bytes.len(),
        29,
        "a 22-byte header plus the point's seven bytes"
    );
    let result = rig.send(p, &bytes);
    assert_eq!(result.status, ReceiptStatus::Applied);
    assert_eq!(result.seq, Some(1));
    assert!(!result.cached);
    rig.mirror_move(a(), &[m(1, 1), m(4, 1)], 1);
    rig.assert_authority();
    assert_eq!(rig.host.executions(), 1);
    assert_eq!(rig.host.last_combat_error(), None);

    // P's one frame, `event_seq` 2.
    let (entries, _, deltas) = client.drain(&mut rig.host, p, rig.now);
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].event_seq, 2);
    assert_eq!(deltas[0].server_tick, 0);
    assert_eq!(
        entry_ops(&entries[0]),
        vec![
            DeltaOp::V3(receipt(epoch_p, 1, 0, ReceiptV3::Applied)),
            moved(1, &[(65536, 0, 65536), (262144, 0, 65536)]),
            facts_op(1, fs(Some((0, 1)), Some(None))),
        ]
    );
    // The replica applies the page: `Moved` is a notice, not a position.
    assert_eq!(
        client.replica.position(n(1)),
        Some(&PositionState {
            tick: 0,
            point: pt(65536, 0, 65536),
            moving: false,
        })
    );

    // V's v2 frame, `event_seq` 2: the permitted state of the involved actor.
    let v_entries = rig.drain(v);
    assert_eq!(v_entries.len(), 1, "{v_entries:?}");
    let v_frame = codec_v2::decode_delta(&v_entries[0].bytes).expect("a v2 delta");
    assert_eq!(v_frame.event_seq, 2);
    assert_eq!(
        v_frame.ops,
        vec![
            protocol_v1::DeltaOp::Health {
                entity: n(1),
                health: 10,
                max_health: 10,
                dead: false,
            },
            protocol_v1::DeltaOp::Turn {
                active: Some(n(1)),
                round: 0,
            },
        ]
        .into_iter()
        .map(crpg_net::protocol_v2::DeltaOp::Legacy)
        .collect::<Vec<_>>()
    );

    // Entry 1 is one command with the `Moved` envelope and no outcome.
    match &rig.entries()[0] {
        CaptureEntry::Command(record) => {
            assert_eq!(record.capture_seq, 1);
            assert_eq!(record.epoch, epoch_p);
            assert_eq!(record.lane, LANE_COMBAT);
            assert_eq!(record.seq, 1);
            assert_eq!(record.history_start, 4);
            assert_eq!(record.history_end, 5);
            assert_eq!(record.outcome, None);
            assert_eq!(record.events.len(), 1);
            assert_eq!(
                record.events[0].payload,
                crpg_sim::HistoryEvent::Moved {
                    entity: a(),
                    points: vec![m(1, 1), m(4, 1)],
                }
            );
            assert_eq!(record.views, vec![view(epoch_v), view(epoch_p)]);
        }
        other => panic!("expected a command entry, got {other:?}"),
    }

    // `advance(1)`: one sample, the moved position.
    rig.advance(1);
    assert_eq!(
        samples(&mut rig.host, p),
        vec![MoveSample {
            server_tick: 1,
            processed_seq: 0,
            entries: vec![crpg_net::movement_v4::SampleEntry {
                entity: n(1),
                point: pt(262144, 0, 65536),
                moving: false,
            }],
        }]
    );

    // The checkpoint keeps no host record, so it is version 1, and it loads.
    let saved = rig.save();
    assert!(saved.starts_with("{\"version\":1,"), "{saved}");
    let loaded = load_checkpoint(saved.as_bytes(), 2, 0).expect("the save loads");
    assert_eq!(loaded.authority_hash(), rig.host.authority_hash());
    assert_eq!(loaded.last_capture(), 1);
}

// ---------------------------------------------------------------------------
// C2.
// ---------------------------------------------------------------------------

/// C2: the turn check precedes navigation and validation; every refusal
/// consumes its sequence, journals nothing and leaves the pool alone.
#[test]
fn c2_declare_move_refusals() {
    // Out of turn: B is controlled but not the head.
    {
        let mut rig = Rig::new(&mv3_value());
        rig.bind_nav();
        let p = rig
            .host
            .bind_v4_peer(
                control(&[a(), b()]),
                full_grants(&[a(), b(), c()]),
                facts(),
                r1(),
                0,
            )
            .expect("binds");
        drain(&mut rig.host, p);
        let before = state_of(&mut rig.host, &[p]);
        let result = rig.declare_move(p, 1, 2, (262144, 0, 65536));
        assert_eq!(
            result.status,
            ReceiptStatus::Rejected(RejectionCode::IllegalAction)
        );
        assert_eq!(result.seq, Some(1));
        assert_combat_error(
            &rig.host,
            CombatError::OutOfTurn {
                actor: b(),
                active: Some(a()),
            },
        );
        assert_unchanged(&mut rig.host, &before);
        rig.assert_authority();
        // The sequence was consumed: the same bytes are a cached rejection.
        assert_eq!(
            rig.host.cached_status_v3(p, 1),
            Ok(Some(ReceiptV3::Rejected(CodeV3::IllegalAction)))
        );
        // Control: A, the head, is applied.
        let control = rig.declare_move(p, 2, 1, (262144, 0, 65536));
        assert_eq!(control.status, ReceiptStatus::Applied);
        rig.mirror_move(a(), &[m(1, 1), m(4, 1)], 2);
        rig.assert_authority();
    }

    // Real-time: `DeclareMove` is `WrongMode` on the wire, `IllegalAction` on
    // the trusted summary.
    {
        let mut rig = Rig::new(&mv3rt_value());
        rig.bind_nav();
        let p = rig.bind_p();
        let before = state_of(&mut rig.host, &[p]);
        let result = rig.declare_move(p, 1, 1, (262144, 0, 65536));
        assert_eq!(
            result.status,
            ReceiptStatus::Rejected(RejectionCode::IllegalAction)
        );
        assert_eq!(
            rig.host.cached_status_v3(p, 1),
            Ok(Some(ReceiptV3::Rejected(CodeV3::WrongMode)))
        );
        assert_unchanged(&mut rig.host, &before);
        rig.assert_authority();
    }

    // No mesh: the turn check passed and navigation refuses.
    {
        let mut rig = Rig::new(&mv3_value());
        let p = rig.bind_p();
        let before = state_of(&mut rig.host, &[p]);
        let result = rig.declare_move(p, 1, 1, (262144, 0, 65536));
        assert_eq!(
            result.status,
            ReceiptStatus::Rejected(RejectionCode::IllegalAction)
        );
        assert_eq!(rig.host.last_combat_error(), None);
        assert_unchanged(&mut rig.host, &before);
        rig.assert_authority();
    }

    // Already there, and unaffordable (`m(9, 9)` is cost 2 of 1). The pair
    // "no mesh with an unaffordable destination" is named, not built: both
    // read `IllegalAction` with `last_combat_error` unchanged.
    {
        let mut rig = Rig::new(&mv3_value());
        rig.bind_nav();
        let p = rig.bind_p();
        let before = state_of(&mut rig.host, &[p]);
        let here = rig.declare_move(p, 1, 1, (65536, 0, 65536));
        assert_eq!(
            here.status,
            ReceiptStatus::Rejected(RejectionCode::IllegalAction)
        );
        assert_eq!(rig.host.last_combat_error(), None);
        assert_unchanged(&mut rig.host, &before);
        let far = rig.declare_move(p, 2, 1, (589824, 0, 589824));
        assert_eq!(
            far.status,
            ReceiptStatus::Rejected(RejectionCode::IllegalAction)
        );
        assert_eq!(rig.host.last_combat_error(), None);
        assert_unchanged(&mut rig.host, &before);
        rig.assert_authority();
    }

    // A session controlling `[B]` only may not act as A.
    {
        let mut rig = Rig::new(&mv3_value());
        rig.bind_nav();
        let p = rig
            .host
            .bind_v4_peer(
                control(&[b()]),
                full_grants(&[a(), b(), c()]),
                facts(),
                r1(),
                0,
            )
            .expect("binds");
        drain(&mut rig.host, p);
        let before = state_of(&mut rig.host, &[p]);
        let result = rig.declare_move(p, 1, 1, (262144, 0, 65536));
        assert_eq!(
            result.status,
            ReceiptStatus::Rejected(RejectionCode::NotAuthorized)
        );
        assert_eq!(rig.host.last_combat_error(), None);
        assert_unchanged(&mut rig.host, &before);
        rig.assert_authority();
    }

    // The pair: a session controlling only C may not act as B, and B is also
    // out of turn; resolution wins with `NotAuthorized`.
    {
        let mut rig = Rig::new(&mv3_value());
        rig.bind_nav();
        let p = rig
            .host
            .bind_v4_peer(
                control(&[c()]),
                full_grants(&[a(), b(), c()]),
                facts(),
                r1(),
                0,
            )
            .expect("binds");
        drain(&mut rig.host, p);
        let before = state_of(&mut rig.host, &[p]);
        let result = rig.declare_move(p, 1, 2, (262144, 0, 65536));
        assert_eq!(
            result.status,
            ReceiptStatus::Rejected(RejectionCode::NotAuthorized)
        );
        assert_eq!(rig.host.last_combat_error(), None);
        assert_unchanged(&mut rig.host, &before);
        rig.assert_authority();
    }

    // The pair: out of turn with no mesh reports the turn error first.
    {
        let mut rig = Rig::new(&mv3_value());
        let p = rig
            .host
            .bind_v4_peer(
                control(&[a(), b()]),
                full_grants(&[a(), b(), c()]),
                facts(),
                r1(),
                0,
            )
            .expect("binds");
        drain(&mut rig.host, p);
        let before = state_of(&mut rig.host, &[p]);
        let result = rig.declare_move(p, 1, 2, (262144, 0, 65536));
        assert_eq!(
            result.status,
            ReceiptStatus::Rejected(RejectionCode::IllegalAction)
        );
        assert_combat_error(
            &rig.host,
            CombatError::OutOfTurn {
                actor: b(),
                active: Some(a()),
            },
        );
        assert_unchanged(&mut rig.host, &before);
        rig.assert_authority();
    }

    // Actor resolution: an unmapped id, then a granted-but-not-acting id.
    {
        let mut rig = Rig::new(&mv3_value());
        rig.bind_nav();
        let p = rig.bind_p();
        let before = state_of(&mut rig.host, &[p]);
        let ghost = rig.declare_move(p, 1, 9, (262144, 0, 65536));
        assert_eq!(
            ghost.status,
            ReceiptStatus::Rejected(RejectionCode::NotAuthorized)
        );
        assert_eq!(rig.host.last_combat_error(), None);
        assert_unchanged(&mut rig.host, &before);
        // Control: n(1) is applied.
        let control = rig.declare_move(p, 2, 1, (262144, 0, 65536));
        assert_eq!(control.status, ReceiptStatus::Applied);
        rig.mirror_move(a(), &[m(1, 1), m(4, 1)], 2);
        rig.assert_authority();
    }
    {
        let mut rig = Rig::new(&mv3_value());
        rig.bind_nav();
        let mut grants = full_grants(&[a(), b(), c()]);
        grants.entities[0].action_actor = false;
        let denied = rig
            .host
            .bind_v4_peer(control(&[a()]), grants, facts(), r1(), 0)
            .expect("binds");
        drain(&mut rig.host, denied);
        let before = state_of(&mut rig.host, &[denied]);
        let result = rig.declare_move(denied, 1, 1, (262144, 0, 65536));
        assert_eq!(
            result.status,
            ReceiptStatus::Rejected(RejectionCode::NotAuthorized)
        );
        assert_eq!(rig.host.last_combat_error(), None);
        assert_unchanged(&mut rig.host, &before);
        rig.assert_authority();
        // Control: the flag set, the same intent is applied.
        let allowed = rig
            .host
            .bind_v4_peer(
                control(&[a()]),
                full_grants(&[a(), b(), c()]),
                facts(),
                r1(),
                0,
            )
            .expect("binds");
        drain(&mut rig.host, allowed);
        let control = rig.declare_move(allowed, 1, 1, (262144, 0, 65536));
        assert_eq!(control.status, ReceiptStatus::Applied);
        rig.mirror_move(a(), &[m(1, 1), m(4, 1)], 1);
        rig.assert_authority();
    }

    // Actor resolution: B's id is retired by narrowing, so n(2) no longer
    // maps; before that call the same intent passes resolution and fails the
    // turn check.
    {
        let mut rig = Rig::new(&mv3_value());
        rig.bind_nav();
        let p = rig
            .host
            .bind_v4_peer(
                control(&[a(), b()]),
                full_grants(&[a(), b(), c()]),
                facts(),
                r1(),
                0,
            )
            .expect("binds");
        drain(&mut rig.host, p);
        let control = rig.declare_move(p, 1, 2, (262144, 0, 65536));
        assert_eq!(
            control.status,
            ReceiptStatus::Rejected(RejectionCode::IllegalAction)
        );
        assert_combat_error(
            &rig.host,
            CombatError::OutOfTurn {
                actor: b(),
                active: Some(a()),
            },
        );
        assert_eq!(
            rig.host.set_grants(p, full_grants(&[a(), c()])),
            Ok(crpg_server::host::GrantUpdate::SnapshotStaged)
        );
        drain(&mut rig.host, p);
        let before = state_of(&mut rig.host, &[p]);
        let retired = rig.declare_move(p, 2, 2, (262144, 0, 65536));
        assert_eq!(
            retired.status,
            ReceiptStatus::Rejected(RejectionCode::NotAuthorized)
        );
        assert_combat_error(
            &rig.host,
            CombatError::OutOfTurn {
                actor: b(),
                active: Some(a()),
            },
        );
        assert_unchanged(&mut rig.host, &before);
        rig.assert_authority();
    }
}

// ---------------------------------------------------------------------------
// C3.
// ---------------------------------------------------------------------------

/// C3: identical bytes are answered from the retry cache and never move twice;
/// a v3 session cannot decode the v4 move.
#[test]
fn c3_declare_move_retry() {
    let mut rig = Rig::new(&mv3_value());
    rig.bind_nav();
    let p = rig.bind_p();
    let mut client = bind_client(&mut rig.host, p, rig.now);
    let bytes = declare_move_bytes(&rig.host, p, 1, 1, (262144, 0, 65536));
    assert_eq!(rig.send(p, &bytes).status, ReceiptStatus::Applied);
    rig.mirror_move(a(), &[m(1, 1), m(4, 1)], 1);
    rig.assert_authority();
    // Drain and acknowledge the page, then resend the same bytes.
    client.drain(&mut rig.host, p, rig.now);
    client.publish(&mut rig.host, p, rig.now);
    let retry = rig.send(p, &bytes);
    assert_eq!(retry.status, ReceiptStatus::Applied);
    assert!(retry.cached);
    assert_eq!(retry.seq, Some(1));
    let entries = rig.drain(p);
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(
        entry_ops(&entries[0]),
        vec![DeltaOp::V3(receipt(
            rig.host.epoch(p).expect("bound"),
            1,
            0,
            ReceiptV3::Applied
        ))]
    );
    assert_eq!(rig.host.executions(), 1);
    assert_eq!(rig.host.last_capture(), 1);
    rig.assert_authority();
    // The pool was spent once (C-2 step 3): a later move to a reachable cell A
    // does not occupy passes `plan_route` and is refused by `validate_move` as
    // unaffordable, not by the route step as `AlreadyThere`.
    let before = state_of(&mut rig.host, &[p]);
    // The mirror judges this destination with `validate_move`, so it is a
    // routable cell and the only fault is the spent pool.
    assert_eq!(
        validate_move(
            rig.oracle.world(),
            a(),
            &route(&m10(), 2, &[m(4, 1), m(2, 1)])
        ),
        Err(MoveError::CannotAfford {
            cost: 1,
            current: 0
        })
    );
    let poor = rig.declare_move(p, 2, 1, (131072, 0, 65536));
    assert_eq!(
        poor.status,
        ReceiptStatus::Rejected(RejectionCode::IllegalAction)
    );
    // The turn check passed and steps 2 and 3 leave the error unchanged.
    assert_eq!(rig.host.last_combat_error(), None);
    assert_unchanged(&mut rig.host, &before);
    rig.assert_authority();

    // Control: the same actor standing on the same cell with the pool unspent
    // (A placed there, nothing paid) accepts the same destination, so the
    // refusal above is the spent pool: an uncharged first move would leave
    // `current` 1, accept this route, move A and raise `executions` to 2.
    {
        let mut fresh = Rig::new(&mv3_value());
        fresh.bind_nav();
        assert_eq!(fresh.host.place(a(), m(4, 1)), Ok(()));
        fresh.oracle.place(a(), m(4, 1)).expect("the mirror places");
        fresh.mirror_ack();
        let fp = fresh.bind_p();
        let control = fresh.declare_move(fp, 1, 1, (131072, 0, 65536));
        assert_eq!(control.status, ReceiptStatus::Applied);
        fresh.mirror_move(a(), &[m(4, 1), m(2, 1)], 1);
        fresh.assert_authority();
    }

    // A v3 session sending a v3 frame whose body tag is 6 (v4's move).
    let q = rig.bind_q();
    drain(&mut rig.host, q);
    let mut v4_bytes = declare_move_bytes(&rig.host, q, 1, 1, (262144, 0, 65536));
    v4_bytes[0] = 3;
    let unknown = rig.send(q, &v4_bytes);
    assert_eq!(unknown.seq, None);
    assert_eq!(
        unknown.status,
        ReceiptStatus::Rejected(RejectionCode::UnknownMessage)
    );
    // Control: the same body at the v3 tag None is refused for its version.
    let unsupported = rig.send(
        q,
        &declare_move_bytes(&rig.host, q, 1, 1, (262144, 0, 65536)),
    );
    assert_eq!(unsupported.seq, None);
    assert_eq!(
        unsupported.status,
        ReceiptStatus::Rejected(RejectionCode::UnsupportedVersion)
    );
    rig.assert_authority();
}

// ---------------------------------------------------------------------------
// C4.
// ---------------------------------------------------------------------------

/// C4: every committed tick samples the mover's new position and its
/// `moving` flag; a still entity is never sampled.
#[test]
fn c4_samples_walk() {
    let mut rig = Rig::new(&mv3rt_value());
    rig.bind_nav();
    let p = rig.bind_p();
    bind_client(&mut rig.host, p, rig.now);
    assert_eq!(rig.host.move_to(a(), m(4, 1)), Ok(None));
    rig.mirror_route(a(), &[m(1, 1), m(4, 1)]);
    rig.assert_authority();

    rig.advance(10);
    assert_eq!(
        samples(&mut rig.host, p),
        vec![MoveSample {
            server_tick: 1,
            processed_seq: 0,
            entries: vec![crpg_net::movement_v4::SampleEntry {
                entity: n(1),
                point: pt(131072, 0, 65536),
                moving: true,
            }],
        }]
    );
    rig.advance(20);
    assert_eq!(
        samples(&mut rig.host, p),
        vec![MoveSample {
            server_tick: 2,
            processed_seq: 0,
            entries: vec![crpg_net::movement_v4::SampleEntry {
                entity: n(1),
                point: pt(196608, 0, 65536),
                moving: true,
            }],
        }]
    );
    rig.advance(30);
    assert_eq!(
        samples(&mut rig.host, p),
        vec![MoveSample {
            server_tick: 3,
            processed_seq: 0,
            entries: vec![crpg_net::movement_v4::SampleEntry {
                entity: n(1),
                point: pt(262144, 0, 65536),
                moving: false,
            }],
        }]
    );
    // Tick 4: nothing changed.
    rig.advance(40);
    assert_eq!(samples(&mut rig.host, p), Vec::new());
}

// ---------------------------------------------------------------------------
// C5.
// ---------------------------------------------------------------------------

/// C5: the refresh interval is 20 ticks, and a withheld position is neither
/// sampled nor re-anchored until it is disclosed again.
#[test]
fn c5_refresh_and_revoke() {
    // The refresh: ticks 5..=19 send nothing, tick 20 re-sends all three.
    {
        let mut rig = Rig::new(&mv3rt_value());
        rig.bind_nav();
        let p = rig.bind_p();
        bind_client(&mut rig.host, p, rig.now);
        assert_eq!(rig.host.move_to(a(), m(4, 1)), Ok(None));
        rig.mirror_route(a(), &[m(1, 1), m(4, 1)]);
        rig.assert_authority();
        for now in [10, 20, 30, 40] {
            rig.advance(now);
            let _ = samples(&mut rig.host, p);
        }
        assert_eq!(POLICY_SAMPLE_REFRESH_TICKS, 20);
        let mut now = 40;
        for _ in 5..20 {
            now += 10;
            rig.advance(now);
            assert_eq!(samples(&mut rig.host, p), Vec::new(), "tick {now}");
        }
        now += 10;
        rig.advance(now);
        let refreshed = samples(&mut rig.host, p);
        assert_eq!(
            refreshed,
            vec![MoveSample {
                server_tick: 20,
                processed_seq: 0,
                entries: vec![
                    crpg_net::movement_v4::SampleEntry {
                        entity: n(1),
                        point: pt(262144, 0, 65536),
                        moving: false,
                    },
                    crpg_net::movement_v4::SampleEntry {
                        entity: n(2),
                        point: pt(196608, 0, 65536),
                        moving: false,
                    },
                    crpg_net::movement_v4::SampleEntry {
                        entity: n(3),
                        point: pt(65536, 0, 327680),
                        moving: false,
                    },
                ],
            }]
        );
    }

    // Revocation after tick 1's sample: no sample for A, and the withdrawal
    // and later re-anchor are pages.
    {
        let mut rig = Rig::new(&mv3rt_value());
        rig.bind_nav();
        let p = rig.bind_p();
        bind_client(&mut rig.host, p, rig.now);
        assert_eq!(rig.host.move_to(a(), m(4, 1)), Ok(None));
        rig.mirror_route(a(), &[m(1, 1), m(4, 1)]);
        rig.advance(10);
        assert_eq!(samples(&mut rig.host, p).len(), 1);
        // The tick's own page is drained first: the setter publishes alone.
        rig.drain(p);
        assert_eq!(rig.host.set_position_withheld(p, vec![a()]), Ok(()));
        let entries = rig.drain(p);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entry_ops(&entries[0]), vec![pos(1, None)]);
        rig.advance(20);
        assert_eq!(samples(&mut rig.host, p), Vec::new());
        rig.advance(30);
        assert_eq!(samples(&mut rig.host, p), Vec::new());
        let mut now = 30;
        for _ in 4..20 {
            now += 10;
            rig.advance(now);
            assert_eq!(samples(&mut rig.host, p), Vec::new(), "tick {now}");
        }
        now += 10;
        rig.advance(now);
        assert_eq!(now, 200);
        let refreshed = samples(&mut rig.host, p);
        assert_eq!(
            refreshed,
            vec![MoveSample {
                server_tick: 20,
                processed_seq: 0,
                entries: vec![
                    crpg_net::movement_v4::SampleEntry {
                        entity: n(2),
                        point: pt(196608, 0, 65536),
                        moving: false,
                    },
                    crpg_net::movement_v4::SampleEntry {
                        entity: n(3),
                        point: pt(65536, 0, 327680),
                        moving: false,
                    },
                ],
            }]
        );
        // Lifting the withdrawal re-anchors A at its current position.
        rig.drain(p);
        assert_eq!(rig.host.set_position_withheld(p, Vec::new()), Ok(()));
        let entries = rig.drain(p);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(
            entry_ops(&entries[0]),
            vec![pos(1, Some((pt(262144, 0, 65536), false)))]
        );
        rig.advance(210);
        assert_eq!(samples(&mut rig.host, p), Vec::new());
    }
}

// ---------------------------------------------------------------------------
// C6.
// ---------------------------------------------------------------------------

/// C6: untaken samples replace, never accumulate; a paused `advance` samples
/// nothing; the call's errors are `NotV4Peer` and `UnknownPeer`.
#[test]
fn c6_queue_and_errors() {
    let mut rig = Rig::new(&mv3rt_value());
    rig.bind_nav();
    let p = rig.bind_p();
    bind_client(&mut rig.host, p, rig.now);
    let q = rig.bind_q();
    drain(&mut rig.host, q);
    assert_eq!(rig.host.move_to(a(), m(4, 1)), Ok(None));
    rig.mirror_route(a(), &[m(1, 1), m(4, 1)]);
    rig.assert_authority();

    // Two ticks, no take: only the second survives.
    rig.advance(10);
    rig.advance(20);
    let taken = samples(&mut rig.host, p);
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].server_tick, 2);
    assert_eq!(taken[0].entries[0].point, pt(196608, 0, 65536));
    assert_eq!(samples(&mut rig.host, p), Vec::new());

    // A paused `advance` ticks nothing, so it samples nothing.
    rig.pause(30);
    rig.advance_paused(40);
    assert_eq!(samples(&mut rig.host, p), Vec::new());
    rig.unpause(50);

    // Errors: a v3 session, an unbound handle, and a closed host.
    assert_eq!(rig.host.take_samples(q), Err(HostError::NotV4Peer));
    let ghost = rig.bind_p();
    rig.host.unbind_peer(ghost);
    assert_eq!(rig.host.take_samples(ghost), Err(HostError::UnknownPeer));
    rig.host.shutdown();
    assert_eq!(rig.host.take_samples(p), Err(HostError::UnknownPeer));
    assert_eq!(rig.host.take_samples(q), Err(HostError::UnknownPeer));
}

// ---------------------------------------------------------------------------
// C7.
// ---------------------------------------------------------------------------

/// C7: a sample fits the T026p5 client and moves the replica's position.
#[test]
fn c7_samples_reach_the_replica() {
    let mut rig = Rig::new(&mv3rt_value());
    rig.bind_nav();
    let p = rig.bind_p();
    let mut client = bind_client(&mut rig.host, p, rig.now);
    assert_eq!(rig.host.move_to(a(), m(4, 1)), Ok(None));
    rig.mirror_route(a(), &[m(1, 1), m(4, 1)]);
    rig.assert_authority();

    let expected = [
        (1, pt(131072, 0, 65536), true),
        (2, pt(196608, 0, 65536), true),
        (3, pt(262144, 0, 65536), false),
    ];
    let mut now = 0;
    for (tick, point, moving) in expected {
        now += 10;
        rig.advance(now);
        let datagrams = rig.host.take_samples(p).expect("a v4 session");
        assert_eq!(datagrams.len(), 1);
        let sample = match decode_down(&datagrams[0]).expect("a move-down datagram") {
            MoveDown::Sample(sample) => sample,
            other => panic!("expected a sample, got {other:?}"),
        };
        assert_eq!(client.replica.apply_sample(&sample), 1);
        assert_eq!(
            client.replica.position(n(1)),
            Some(&PositionState {
                tick,
                point,
                moving,
            })
        );
    }
}
