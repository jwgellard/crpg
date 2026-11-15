//! T037a acceptance suite (`tasks/T037.md`, tests A1..A10): navigation
//! binding, the one route computation and the trusted movement operations,
//! public API only.
//!
//! Oracles are independent of the host: a test-owned mirror `HistoryWorld`
//! loaded from the same JSON, driven with the same public sim calls and
//! acknowledged after each, and routes computed by calling `crpg_nav`
//! directly on the same mesh. The fixtures are
//! `tests/fixtures/three_combatants.json` as saved-game JSON, edited in
//! memory (no new fixture file):
//!
//! - `MV3`: `TB3`'s JSON with movers for A, B and C, their `f32` mirrors and
//!   the definition's `movement` rules (speed 1 m/tick, one `JY` unit per
//!   started 10 m).
//! - `MV3rt`: the same edits on `RT3`'s JSON.
//! - `MV3-C`: `MV3rt` without C's mover and transform.
//!
//! `A` stands at `m(1, 1)`, `B` at `m(3, 1)`, `C` at `m(1, 5)`.
//!
//! Every refusal is compared against the host state taken just before the
//! call and sits beside an accepted control.

use crpg_core::{EntityId, Fx16_16};
use crpg_nav::{bake_box, BakeError, NavError, NavMesh, NavRect};
use crpg_net::codec_v2;
use crpg_net::protocol::{IntentBody, IntentFrame, NetId, ReceiptStatus, LANE_COMBAT};
use crpg_net::protocol_v3::PauseRights;
use crpg_server::capture::{CaptureEntry, HostOp, HostRecord};
use crpg_server::checkpoint::load_checkpoint;
use crpg_server::host::{
    ControlGrant, DeliveryEntry, DisclosureGrants, EntityDisclosure, FactGrants, Host, HostConfig,
    HostError, NavBinding, PeerHandle, ProtocolSelection, HOST_ROUTE_TAG,
};
use crpg_sim::{
    history_hash, CombatAction, CombatError, EncounterMode, HistoryWorld, MoveError, MoveOutcome,
    MoveRoute, NavRevision, RouteTag,
};
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");

// ---------------------------------------------------------------------------
// Identities and geometry.
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
fn e7() -> EntityId {
    eid(7)
}

/// A raw point, `[x, y, z]` in 1/65,536 m.
fn p(x: i32, y: i32, z: i32) -> [Fx16_16; 3] {
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

fn rect(x0: i32, z0: i32, x1: i32, z1: i32) -> NavRect {
    NavRect {
        min_x: Fx16_16::from_int(x0),
        min_z: Fx16_16::from_int(z0),
        max_x: Fx16_16::from_int(x1),
        max_z: Fx16_16::from_int(z1),
    }
}

fn m10() -> NavMesh {
    bake_box(Fx16_16::ZERO, rect(0, 0, 10, 10), &[]).expect("bakes")
}
fn m4() -> NavMesh {
    bake_box(Fx16_16::ZERO, rect(0, 0, 4, 4), &[]).expect("bakes")
}
fn mh() -> NavMesh {
    bake_box(Fx16_16::ZERO, rect(0, 0, 10, 10), &[rect(4, 0, 6, 8)]).expect("bakes")
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

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

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

/// The movement edits of `MV3`/`MV3rt`: three movers at 1 m/tick with a 10 m
/// step, their mirrors, and the definition's one-`JY`-per-10 m rules.
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

/// `MV3rt` without C's mover and transform.
fn mv3_c_value() -> Value {
    let mut value = mv3rt_value();
    for key in ["movers", "transforms"] {
        let pairs = value["world"][key].as_array_mut().expect("pair list");
        pairs.retain(|pair| pair[0]["index"].as_u64() != Some(2));
    }
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
        abilities: Vec::new(),
        encounters: Vec::new(),
        turn_round: true,
        encounter_end: true,
        turn_state: true,
    }
}

fn control(actors: &[EntityId]) -> ControlGrant {
    ControlGrant {
        actors: actors.to_vec(),
    }
}

fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}

// ---------------------------------------------------------------------------
// Rig: one host and its mirror authority.
// ---------------------------------------------------------------------------

/// A refusal leaves all of this as it was.
#[derive(Debug, PartialEq, Eq)]
struct State {
    hash: [u8; 32],
    last_capture: u64,
    executions: u64,
    nav: Option<NavRevision>,
    deliveries: Vec<(PeerHandle, Vec<DeliveryEntry>)>,
}

fn state_of(host: &mut Host, peers: &[PeerHandle]) -> State {
    State {
        hash: host.authority_hash(),
        last_capture: host.last_capture(),
        executions: host.executions(),
        nav: host.nav_revision(),
        deliveries: peers
            .iter()
            .map(|peer| (*peer, host.take_delivery_entries(*peer).expect("bound")))
            .collect(),
    }
}

fn assert_unchanged(host: &mut Host, before: &State, peers: &[PeerHandle]) {
    assert_eq!(host.authority_hash(), before.hash);
    assert_eq!(host.last_capture(), before.last_capture);
    assert_eq!(host.executions(), before.executions);
    assert_eq!(host.nav_revision(), before.nav);
    for (peer, entries) in &before.deliveries {
        assert_eq!(host.take_delivery_entries(*peer), Ok(entries.clone()));
    }
    let _ = peers;
}

/// Takes and acknowledges every pending entry of `peer`.
fn drain(host: &mut Host, peer: PeerHandle) -> Vec<DeliveryEntry> {
    let entries = host.take_delivery_entries(peer).expect("bound");
    if let Some(last) = entries.last() {
        host.acknowledge_delivery(peer, last.ordinal).expect("acks");
    }
    entries
}

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

    /// The mirror's `bind_nav`, acknowledged; returns the cleared pairs.
    fn mirror_bind_nav(&mut self, mesh: &NavMesh) -> Vec<(EntityId, RouteTag)> {
        let cleared = self
            .oracle
            .bind_nav(r(mesh))
            .expect("the mirror binds the revision");
        self.mirror_ack();
        cleared
    }

    fn mirror_place(&mut self, entity: EntityId, position: [Fx16_16; 3]) {
        self.oracle
            .place(entity, position)
            .expect("the mirror places");
        self.mirror_ack();
    }

    /// The mirror's `set_route`, acknowledged; returns the replaced tag.
    fn mirror_set_route(&mut self, actor: EntityId, route: MoveRoute) -> Option<RouteTag> {
        let replaced = self
            .oracle
            .set_route(actor, route)
            .expect("the mirror accepts the route");
        self.mirror_ack();
        replaced
    }

    /// The mirror's `clear_route`, acknowledged; returns the cleared tag.
    fn mirror_clear_route(&mut self, actor: EntityId) -> Option<RouteTag> {
        let cleared = self
            .oracle
            .clear_route(actor)
            .expect("the mirror clears the route");
        self.mirror_ack();
        cleared
    }

    fn mirror_tick(&mut self) {
        self.oracle.tick().expect("the mirror ticks");
        self.mirror_ack();
    }

    /// The mirror's `perform_move`, acknowledged.
    fn mirror_move(&mut self, actor: EntityId, route: MoveRoute) -> MoveOutcome {
        let outcome = self
            .oracle
            .perform_move(actor, &route)
            .expect("the mirror moves");
        self.mirror_ack();
        outcome
    }

    fn state(&mut self, peers: &[PeerHandle]) -> State {
        state_of(&mut self.host, peers)
    }

    fn assert_unchanged(&mut self, before: &State, peers: &[PeerHandle]) {
        assert_unchanged(&mut self.host, before, peers);
    }

    fn assert_authority(&self) {
        assert_eq!(self.host.authority_hash(), history_hash(&self.oracle));
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

    fn save(&self) -> String {
        String::from_utf8(self.host.save_checkpoint().expect("saves")).expect("utf8")
    }

    /// Binds up to `limit` control entries for A, B and C; every `FULL` flag.
    fn bind_v(&mut self) -> PeerHandle {
        let peer = self
            .host
            .bind_peer(
                control(&[a(), b(), c()]),
                full_grants(&[a(), b(), c()]),
                self.now,
            )
            .expect("binds");
        self.drain(peer);
        peer
    }

    fn drain(&mut self, peer: PeerHandle) -> Vec<DeliveryEntry> {
        drain(&mut self.host, peer)
    }

    /// One v2 `EndTurn` intent through `ingest` then `pump`, +100 ms.
    fn end_turn(&mut self, peer: PeerHandle, seq: u64, actor: EntityId) -> ReceiptStatus {
        let frame = IntentFrame {
            epoch: self.host.epoch(peer).expect("bound"),
            lane: LANE_COMBAT,
            seq,
            observed_tick: self.host.server_tick(),
            actor: n(u64::from(actor.index()) + 1),
            body: IntentBody::EndTurn,
        };
        let bytes = codec_v2::encode_intent(&frame).expect("encodes");
        self.now += 100;
        assert_eq!(
            self.host.ingest(peer, &bytes, self.now),
            Ok(crpg_server::host::IngestDisposition::Staged)
        );
        let summary = self.host.pump(self.now).expect("pump");
        assert_eq!(summary.results.len(), 1, "{summary:?}");
        summary.results[0].status
    }
}

fn host_record(entry: &CaptureEntry) -> &HostRecord {
    match entry {
        CaptureEntry::Host(record) => record,
        CaptureEntry::Command(record) => panic!("expected a host entry, got {record:?}"),
    }
}

fn mover_position(oracle: &HistoryWorld, entity: EntityId) -> [Fx16_16; 3] {
    oracle
        .world()
        .movers()
        .get(entity)
        .expect("the mirror holds the mover")
        .position()
}

fn mover_route(oracle: &HistoryWorld, entity: EntityId) -> Option<MoveRoute> {
    oracle
        .world()
        .movers()
        .get(entity)
        .expect("the mirror holds the mover")
        .route()
        .map(|active| active.route().clone())
}

// ---------------------------------------------------------------------------
// A1.
// ---------------------------------------------------------------------------

#[test]
fn a1_bind_area_and_mesh() {
    let value = mv3_value();
    let mut rig = Rig::new(&value);
    assert_eq!(rig.host.nav_revision(), None);

    let bound = rig
        .host
        .bind_area_bounds(p(0, 0, 0), p(655360, 655360, 655360));
    assert_eq!(
        bound,
        Ok(NavBinding {
            revision: r(&m10()),
            cleared: Vec::new(),
            placed: Vec::new(),
        })
    );
    assert_eq!(rig.host.nav_revision(), Some(r(&m10())));
    assert_eq!(rig.mirror_bind_nav(&m10()), Vec::new());
    rig.assert_authority();

    // Control: the same mesh bound directly returns the same binding and hash.
    let mut direct = Rig::new(&value);
    assert_eq!(direct.host.bind_nav_mesh(m10()), bound);
    assert_eq!(direct.mirror_bind_nav(&m10()), Vec::new());
    direct.assert_authority();
    assert_eq!(direct.host.authority_hash(), rig.host.authority_hash());

    // The floor is `min.y`: a raised bound bakes a different revision.
    let raised_mesh = bake_box(Fx16_16::from_int(1), rect(0, 0, 10, 10), &[]).expect("bakes");
    assert_ne!(r(&raised_mesh), r(&m10()));
    let mut raised = Rig::new(&value);
    assert_eq!(
        raised
            .host
            .bind_area_bounds(p(0, 65536, 0), p(655360, 655360, 655360)),
        Ok(NavBinding {
            revision: r(&raised_mesh),
            cleared: Vec::new(),
            placed: Vec::new(),
        })
    );
    raised.mirror_bind_nav(&raised_mesh);
    raised.assert_authority();

    // A failed bake is refused and changes nothing.
    let before = rig.state(&[]);
    assert_eq!(
        rig.host.bind_area_bounds(p(0, 0, 0), p(0, 655360, 655360)),
        Err(HostError::NavBake(BakeError::EmptyOuter))
    );
    assert_eq!(rig.host.nav_revision(), Some(r(&m10())));
    rig.assert_unchanged(&before, &[]);
    rig.assert_authority();

    // The four new `Display` texts, and `source()` is `None` for each.
    let cases = [
        (
            HostError::NavBake(BakeError::EmptyOuter),
            "NavBake at host/admission",
        ),
        (HostError::NoNavMesh, "NoNavMesh at host/admission"),
        (
            HostError::NoPath(NavError::AlreadyThere),
            "NoPath at host/admission",
        ),
        (
            HostError::Move(MoveError::NotMover { entity: a() }),
            "Move at host/admission",
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
        assert!(std::error::Error::source(&error).is_none());
    }

    // A closed host refuses both bindings.
    let mut closed = Rig::new(&value);
    closed.host.shutdown();
    assert_eq!(
        closed
            .host
            .bind_area_bounds(p(0, 0, 0), p(655360, 655360, 655360)),
        Err(HostError::Closed)
    );
    assert_eq!(closed.host.bind_nav_mesh(m10()), Err(HostError::Closed));
    closed.assert_authority();
}

// ---------------------------------------------------------------------------
// A2.
// ---------------------------------------------------------------------------

#[test]
fn a2_off_mesh_movers_are_placed() {
    let value = mv3_value();
    let mut rig = Rig::new(&value);
    assert_eq!(
        rig.host.bind_nav_mesh(m4()),
        Ok(NavBinding {
            revision: r(&m4()),
            cleared: Vec::new(),
            placed: vec![c()],
        })
    );
    assert_eq!(rig.mirror_bind_nav(&m4()), Vec::new());
    rig.mirror_place(c(), p(65536, 0, 262144));
    rig.assert_authority();
    assert_eq!(mover_position(&rig.oracle, c()), m(1, 4));
    assert_eq!(mover_position(&rig.oracle, a()), m(1, 1));
    assert_eq!(mover_position(&rig.oracle, b()), m(3, 1));

    // Control: `M10` contains every mover, so nobody is placed.
    let mut wide = Rig::new(&value);
    assert_eq!(
        wide.host.bind_nav_mesh(m10()),
        Ok(NavBinding {
            revision: r(&m10()),
            cleared: Vec::new(),
            placed: Vec::new(),
        })
    );
    wide.mirror_bind_nav(&m10());
    wide.assert_authority();
    assert_eq!(mover_position(&wide.oracle, c()), m(1, 5));
}

// ---------------------------------------------------------------------------
// A3.
// ---------------------------------------------------------------------------

#[test]
fn a3_rebind_keeps_or_clears_routes() {
    let value = mv3rt_value();
    let mut rig = Rig::new(&value);
    let v2 = rig
        .host
        .bind_peer(control(&[a(), b(), c()]), full_grants(&[a(), b(), c()]), 0)
        .expect("binds");
    assert!(!rig.drain(v2).is_empty());
    let v3 = rig
        .host
        .bind_v3_peer(
            control(&[a(), b(), c()]),
            full_grants(&[a(), b(), c()]),
            FactGrants::default(),
            PauseRights::default(),
            0,
        )
        .expect("binds");
    assert!(!rig.drain(v3).is_empty());

    assert_eq!(rig.host.bind_nav_mesh(m10()).unwrap().placed, Vec::new());
    assert_eq!(rig.mirror_bind_nav(&m10()), Vec::new());
    rig.assert_authority();

    assert_eq!(rig.host.move_to(a(), m(4, 1)), Ok(None));
    assert_eq!(
        rig.mirror_set_route(a(), route(&m10(), 0, &[m(1, 1), m(4, 1)])),
        None
    );
    rig.assert_authority();
    assert_eq!(rig.host.move_to(a(), m(1, 4)), Ok(Some(RouteTag(0))));
    assert_eq!(
        rig.mirror_set_route(a(), route(&m10(), 0, &[m(1, 1), m(1, 4)])),
        Some(RouteTag(0))
    );
    rig.assert_authority();
    assert_eq!(
        mover_route(&rig.oracle, a()).map(|route| route.points),
        Some(vec![m(1, 1), m(1, 4)])
    );

    // A same-revision rebind keeps the route and clears nothing.
    assert_eq!(
        rig.host.bind_nav_mesh(m10()),
        Ok(NavBinding {
            revision: r(&m10()),
            cleared: Vec::new(),
            placed: Vec::new(),
        })
    );
    assert_eq!(rig.mirror_bind_nav(&m10()), Vec::new());
    rig.assert_authority();
    assert!(mover_route(&rig.oracle, a()).is_some());

    // A new revision clears it.
    assert_eq!(
        rig.host.bind_nav_mesh(mh()),
        Ok(NavBinding {
            revision: r(&mh()),
            cleared: vec![(a(), RouteTag(0))],
            placed: Vec::new(),
        })
    );
    assert_eq!(rig.mirror_bind_nav(&mh()), vec![(a(), RouteTag(0))]);
    rig.assert_authority();
    assert_eq!(mover_route(&rig.oracle, a()), None);

    // No zero-event operation publishes a frame.
    assert_eq!(rig.host.take_delivery_entries(v2), Ok(Vec::new()));
    assert_eq!(rig.host.take_delivery_entries(v3), Ok(Vec::new()));
}

// ---------------------------------------------------------------------------
// A4.
// ---------------------------------------------------------------------------

#[test]
fn a4_move_to_walks() {
    let value = mv3rt_value();
    let mut rig = Rig::new(&value);
    rig.host.bind_nav_mesh(m10()).unwrap();
    rig.mirror_bind_nav(&m10());
    assert_eq!(rig.host.move_to(a(), m(4, 1)), Ok(None));
    rig.mirror_set_route(a(), route(&m10(), 0, &[m(1, 1), m(4, 1)]));
    rig.assert_authority();

    for (index, expected) in [m(2, 1), m(3, 1), m(4, 1)].into_iter().enumerate() {
        assert_eq!(rig.host.tick(), Ok(()));
        rig.mirror_tick();
        rig.assert_authority();
        assert_eq!(
            mover_position(&rig.oracle, a()),
            expected,
            "tick {}",
            index + 1
        );
    }
    assert_eq!(mover_route(&rig.oracle, a()), None);

    // The goal is S3's snap then nearest-on-mesh.
    let mut far = Rig::new(&value);
    far.host.bind_nav_mesh(m10()).unwrap();
    far.mirror_bind_nav(&m10());
    assert_eq!(far.host.move_to(a(), m(12, 1)), Ok(None));
    far.mirror_set_route(a(), route(&m10(), 0, &[m(1, 1), m(10, 1)]));
    far.assert_authority();
    assert_eq!(
        mover_route(&far.oracle, a()).map(|route| route.points),
        Some(vec![m(1, 1), m(10, 1)])
    );

    let mut raised = Rig::new(&value);
    raised.host.bind_nav_mesh(m10()).unwrap();
    raised.mirror_bind_nav(&m10());
    let high_destination = [
        Fx16_16::from_int(4),
        Fx16_16::from_int(5),
        Fx16_16::from_int(1),
    ];
    assert_eq!(raised.host.move_to(a(), high_destination), Ok(None));
    raised.mirror_set_route(a(), route(&m10(), 0, &[m(1, 1), m(4, 1)]));
    raised.assert_authority();
    assert_eq!(
        mover_route(&raised.oracle, a()).map(|route| route.points),
        Some(vec![m(1, 1), m(4, 1)])
    );

    // Differential: the wall's route is nav's own answer.
    let expected: Vec<[Fx16_16; 3]> = mh()
        .find_path(m(1, 1), m(9, 1))
        .expect("the wall's path exists")
        .into_points();
    assert!(expected.len() >= 3);
    let mut wall = Rig::new(&value);
    wall.host.bind_nav_mesh(mh()).unwrap();
    wall.mirror_bind_nav(&mh());
    assert_eq!(wall.host.move_to(a(), m(9, 1)), Ok(None));
    wall.mirror_set_route(a(), route(&mh(), 0, &expected));
    wall.assert_authority();
    assert_eq!(
        mover_route(&wall.oracle, a()).map(|route| route.points),
        Some(expected)
    );
}

// ---------------------------------------------------------------------------
// A5.
// ---------------------------------------------------------------------------

#[test]
fn a5_move_to_refusals() {
    let value = mv3rt_value();

    // Closed beats unbound; unbound beats an absent entity.
    let mut closed = Rig::new(&value);
    closed.host.shutdown();
    assert_eq!(closed.host.move_to(a(), m(4, 1)), Err(HostError::Closed));
    closed.assert_authority();
    let mut unbound = Rig::new(&value);
    let before = unbound.state(&[]);
    assert_eq!(
        unbound.host.move_to(a(), m(4, 1)),
        Err(HostError::NoNavMesh)
    );
    unbound.assert_unchanged(&before, &[]);
    assert_eq!(
        unbound.host.move_to(e7(), m(4, 1)),
        Err(HostError::NoNavMesh)
    );
    unbound.assert_unchanged(&before, &[]);
    unbound.assert_authority();

    let mut rig = Rig::new(&value);
    let bound = rig.host.bind_nav_mesh(m10());
    assert!(bound.is_ok());
    rig.mirror_bind_nav(&m10());

    let before = rig.state(&[]);
    assert_eq!(
        rig.host.move_to(e7(), m(4, 1)),
        Err(HostError::Move(MoveError::AbsentEntity { entity: e7() }))
    );
    rig.assert_unchanged(&before, &[]);

    // `AbsentEntity`/`NotMover` and `NotMover`/a path error are unbuildable
    // pairs: an entity without a mover has no start point, so it can reach no
    // path check.
    let before = rig.state(&[]);
    assert_eq!(
        rig.host.move_to(a(), m(1, 1)),
        Err(HostError::NoPath(NavError::AlreadyThere))
    );
    rig.assert_unchanged(&before, &[]);

    assert_eq!(rig.host.place(a(), m(11, 1)), Ok(()));
    rig.mirror_place(a(), m(11, 1));
    rig.assert_authority();
    let before_off_mesh = rig.state(&[]);
    assert_eq!(
        rig.host.move_to(a(), m(4, 1)),
        Err(HostError::NoPath(NavError::StartOffMesh))
    );
    rig.assert_unchanged(&before_off_mesh, &[]);

    // `MV3-C` has no mover for C.
    let mut no_c = Rig::new(&mv3_c_value());
    no_c.host.bind_nav_mesh(m10()).unwrap();
    no_c.mirror_bind_nav(&m10());
    let before = no_c.state(&[]);
    assert_eq!(
        no_c.host.move_to(c(), m(4, 1)),
        Err(HostError::Move(MoveError::NotMover { entity: c() }))
    );
    no_c.assert_unchanged(&before, &[]);

    // Turn-based: the sim refuses the route; the path check precedes it.
    let mut tb = Rig::new(&mv3_value());
    tb.host.bind_nav_mesh(m10()).unwrap();
    tb.mirror_bind_nav(&m10());
    let before = tb.state(&[]);
    assert_eq!(
        tb.host.move_to(a(), m(4, 1)),
        Err(HostError::Move(MoveError::TurnBased { entity: a() }))
    );
    tb.assert_unchanged(&before, &[]);
    let before = tb.state(&[]);
    assert_eq!(
        tb.host.move_to(a(), m(1, 1)),
        Err(HostError::NoPath(NavError::AlreadyThere))
    );
    tb.assert_unchanged(&before, &[]);

    // After shutdown.
    tb.host.shutdown();
    assert_eq!(tb.host.move_to(a(), m(4, 1)), Err(HostError::Closed));

    // Control: a bound real-time host accepts the same request.
    let mut control = Rig::new(&value);
    control.host.bind_nav_mesh(m10()).unwrap();
    control.mirror_bind_nav(&m10());
    assert_eq!(control.host.move_to(a(), m(4, 1)), Ok(None));
    control.mirror_set_route(a(), route(&m10(), 0, &[m(1, 1), m(4, 1)]));
    control.assert_authority();

    rig.assert_authority();
    no_c.assert_authority();
}

// ---------------------------------------------------------------------------
// A6.
// ---------------------------------------------------------------------------

#[test]
fn a6_halt_and_place() {
    let value = mv3rt_value();
    let mut rig = Rig::new(&value);
    rig.host.bind_nav_mesh(m10()).unwrap();
    rig.mirror_bind_nav(&m10());
    assert_eq!(rig.host.move_to(a(), m(4, 1)), Ok(None));
    rig.mirror_set_route(a(), route(&m10(), 0, &[m(1, 1), m(4, 1)]));

    assert_eq!(rig.host.halt(a()), Ok(Some(RouteTag(0))));
    assert_eq!(rig.mirror_clear_route(a()), Some(RouteTag(0)));
    rig.assert_authority();
    assert_eq!(rig.host.halt(a()), Ok(None));
    assert_eq!(rig.mirror_clear_route(a()), None);
    rig.assert_authority();

    let before = rig.state(&[]);
    assert_eq!(
        rig.host.halt(e7()),
        Err(HostError::Move(MoveError::AbsentEntity { entity: e7() }))
    );
    rig.assert_unchanged(&before, &[]);

    // A placement clears the route and needs no mesh.
    assert_eq!(rig.host.move_to(a(), m(4, 1)), Ok(None));
    rig.mirror_set_route(a(), route(&m10(), 0, &[m(1, 1), m(4, 1)]));
    assert_eq!(rig.host.place(a(), m(2, 2)), Ok(()));
    rig.mirror_place(a(), m(2, 2));
    rig.assert_authority();
    assert_eq!(mover_position(&rig.oracle, a()), m(2, 2));
    assert_eq!(mover_route(&rig.oracle, a()), None);

    let before = rig.state(&[]);
    assert_eq!(
        rig.host.place(a(), p(536870913, 0, 0)),
        Err(HostError::Move(MoveError::OutOfBounds { index: 0 }))
    );
    rig.assert_unchanged(&before, &[]);

    let mut no_c = Rig::new(&mv3_c_value());
    assert_eq!(
        no_c.host.place(c(), m(1, 1)),
        Err(HostError::Move(MoveError::NotMover { entity: c() }))
    );
    no_c.assert_authority();

    // A placement before any binding succeeds; the mirror places too.
    let mut bare = Rig::new(&value);
    assert_eq!(bare.host.place(a(), m(2, 2)), Ok(()));
    bare.mirror_place(a(), m(2, 2));
    bare.assert_authority();

    // Control: the same halt on an unbound host still clears.
    let mut unbound = Rig::new(&value);
    assert_eq!(unbound.host.halt(a()), Ok(None));
    assert_eq!(unbound.mirror_clear_route(a()), None);
    unbound.assert_authority();
}

// ---------------------------------------------------------------------------
// A7.
// ---------------------------------------------------------------------------

#[test]
fn a7_host_move_turn_based() {
    let value = mv3_value();
    let mut rig = Rig::new(&value);
    let v = rig.bind_v();
    assert_eq!(rig.host.bind_nav_mesh(m10()).unwrap().placed, Vec::new());
    rig.mirror_bind_nav(&m10());
    rig.assert_authority();

    // Out of turn: refused, and the reservation before it changed nothing.
    let before = rig.state(&[v]);
    assert_eq!(
        rig.host.host_move(b(), m(4, 1)),
        Err(HostError::Combat(CombatError::OutOfTurn {
            actor: b(),
            active: Some(a()),
        }))
    );
    rig.assert_unchanged(&before, &[v]);

    // Unbound: no mesh once the turn check passes; B is refused by the turn
    // check first.
    let mut bare = Rig::new(&value);
    assert_eq!(bare.host.host_move(a(), m(4, 1)), Err(HostError::NoNavMesh));
    assert_eq!(
        bare.host.host_move(b(), m(4, 1)),
        Err(HostError::Combat(CombatError::OutOfTurn {
            actor: b(),
            active: Some(a()),
        }))
    );
    bare.assert_authority();

    // Cost: 741,456 raw over a 655,360 step is two units of one.
    let before = rig.state(&[v]);
    assert_eq!(
        rig.host.host_move(a(), m(9, 9)),
        Err(HostError::Move(MoveError::CannotAfford {
            cost: 2,
            current: 1,
        }))
    );
    rig.assert_unchanged(&before, &[v]);

    let moved = rig.host.host_move(a(), m(4, 1));
    assert_eq!(
        moved,
        Ok(MoveOutcome {
            actor: a(),
            cost: 1,
            from: m(1, 1),
            to: m(4, 1),
        })
    );
    assert_eq!(
        rig.mirror_move(a(), route(&m10(), 0, &[m(1, 1), m(4, 1)])),
        MoveOutcome {
            actor: a(),
            cost: 1,
            from: m(1, 1),
            to: m(4, 1),
        }
    );
    rig.assert_authority();

    // One host record holding the one `Moved` envelope of the mirror's seq 5.
    let entries = rig.entries();
    assert_eq!(entries.len(), 1);
    let record = host_record(&entries[0]);
    assert_eq!(record.capture_seq, 1);
    assert_eq!(record.op, HostOp::Action);
    assert_eq!(record.history_start, 4);
    assert_eq!(record.history_end, 5);
    assert_eq!(record.processed_tick, 0);
    assert_eq!(record.events.len(), 1);
    assert_eq!(record.events[0].seq, 5);
    assert_eq!(
        record.events[0].payload,
        crpg_sim::HistoryEvent::Moved {
            entity: a(),
            points: vec![m(1, 1), m(4, 1)],
        }
    );
    assert_eq!(rig.host.executions(), 0);
    assert_eq!(rig.host.take_delivery_entries(v), Ok(Vec::new()));

    // The pool is spent to zero: the next move cannot afford one unit.
    let before = rig.state(&[v]);
    assert_eq!(
        rig.host.host_move(a(), m(4, 4)),
        Err(HostError::Move(MoveError::CannotAfford {
            cost: 1,
            current: 0,
        }))
    );
    rig.assert_unchanged(&before, &[v]);

    // Real time refuses the turn verb.
    let mut rt = Rig::new(&mv3rt_value());
    rt.host.bind_nav_mesh(m10()).unwrap();
    rt.mirror_bind_nav(&m10());
    rt.assert_authority();
    let before = rt.state(&[]);
    assert_eq!(
        rt.host.host_move(a(), m(4, 1)),
        Err(HostError::Combat(CombatError::WrongMode {
            mode: EncounterMode::RealTime,
        }))
    );
    rt.assert_unchanged(&before, &[]);

    // A full capture journal refuses the record's room before anything else.
    let mut full = Rig::new(&value);
    let v2 = full.bind_v();
    full.host.bind_nav_mesh(m10()).unwrap();
    full.mirror_bind_nav(&m10());
    for seq in 1..=4096_u64 {
        let active = full
            .oracle
            .world()
            .combat()
            .expect("an encounter")
            .active
            .expect("a head");
        assert_eq!(full.end_turn(v2, seq, active), ReceiptStatus::Applied);
        full.oracle
            .perform_action(&CombatAction::EndTurn { actor: active })
            .expect("the mirror ends the same turn");
        full.mirror_ack();
        full.drain(v2);
    }
    assert_eq!(full.host.last_capture(), 4096);
    full.assert_authority();
    let before = full.state(&[v2]);
    assert_eq!(
        full.host.host_move(b(), m(4, 1)),
        Err(HostError::CaptureFull)
    );
    full.assert_unchanged(&before, &[v2]);
}

// ---------------------------------------------------------------------------
// A8.
// ---------------------------------------------------------------------------

#[test]
fn a8_host_move_checkpoint() {
    let value = mv3_value();
    let mut rig = Rig::new(&value);
    rig.host.bind_nav_mesh(m10()).unwrap();
    rig.mirror_bind_nav(&m10());

    let before = rig.save();
    assert!(before.starts_with("{\"version\":1,"), "{before}");

    assert_eq!(
        rig.host.host_move(a(), m(4, 1)),
        Ok(MoveOutcome {
            actor: a(),
            cost: 1,
            from: m(1, 1),
            to: m(4, 1),
        })
    );
    rig.mirror_move(a(), route(&m10(), 0, &[m(1, 1), m(4, 1)]));
    rig.assert_authority();

    let after = rig.save();
    assert!(after.starts_with("{\"version\":2,"), "{after}");
    let loaded = load_checkpoint(after.as_bytes(), 2, 0).expect("the save loads");
    assert_eq!(loaded.authority_hash(), rig.host.authority_hash());
    assert_eq!(loaded.capture_acknowledged(), 0);
    assert_eq!(loaded.last_capture(), 1);
}

// ---------------------------------------------------------------------------
// A9.
// ---------------------------------------------------------------------------

#[test]
fn a9_restore_and_rebind() {
    let value = mv3rt_value();
    let mut rig = Rig::new(&value);
    rig.host.bind_nav_mesh(m10()).unwrap();
    rig.mirror_bind_nav(&m10());
    assert_eq!(rig.host.move_to(a(), m(4, 1)), Ok(None));
    rig.mirror_set_route(a(), route(&m10(), 0, &[m(1, 1), m(4, 1)]));
    assert_eq!(rig.host.tick(), Ok(()));
    rig.mirror_tick();
    rig.assert_authority();

    let saved = rig.save();
    assert!(saved.starts_with("{\"version\":1,"), "{saved}");
    let mut loaded = load_checkpoint(saved.as_bytes(), 2, 0).expect("the save loads");
    assert_eq!(loaded.nav_revision(), None);

    let before = state_of(&mut loaded, &[]);
    assert_eq!(loaded.move_to(a(), m(1, 4)), Err(HostError::NoNavMesh));
    assert_unchanged(&mut loaded, &before, &[]);
    assert_eq!(loaded.authority_hash(), rig.host.authority_hash());

    // The same mesh rebinds without clearing the route or placing anyone.
    assert_eq!(
        loaded.bind_nav_mesh(m10()),
        Ok(NavBinding {
            revision: r(&m10()),
            cleared: Vec::new(),
            placed: Vec::new(),
        })
    );
    assert_eq!(loaded.nav_revision(), Some(r(&m10())));

    // One more tick on each side: the loaded host continues the same walk.
    assert_eq!(loaded.tick(), Ok(()));
    assert_eq!(rig.host.tick(), Ok(()));
    rig.mirror_tick();
    rig.assert_authority();
    assert_eq!(loaded.authority_hash(), rig.host.authority_hash());
    assert_eq!(mover_position(&rig.oracle, a()), m(3, 1));
}

// ---------------------------------------------------------------------------
// A10.
// ---------------------------------------------------------------------------

#[test]
fn a10_shared_constants() {
    assert_eq!(crpg_nav::COORD_LIMIT_RAW, crpg_sim::COORD_LIMIT_RAW);
    assert_eq!(crpg_nav::MAX_PATH_POINTS, crpg_sim::MAX_ROUTE_POINTS);
    assert_eq!(
        crpg_nav::MAX_PATH_LENGTH_RAW,
        crpg_sim::MAX_ROUTE_LENGTH_RAW
    );
    assert_eq!(
        crpg_net::protocol_v4::COORD_LIMIT_RAW,
        crpg_sim::COORD_LIMIT_RAW
    );
    assert_eq!(
        crpg_net::protocol_v4::MAX_MOVED_POINTS,
        crpg_sim::MAX_ROUTE_POINTS
    );
    assert_eq!(HOST_ROUTE_TAG, RouteTag(0));
}
