//! T153 acceptance suite (`tasks/T153.md`, tests H1..H11): the MVP AI host
//! driver, public API only.
//!
//! Every case runs beside a mirror `HistoryWorld` loaded from the same JSON,
//! driven with the same sim calls and acknowledged after each (the `Rig` of
//! `host_nav.rs`, copied). Every mirrored move uses the route
//! `crpg_ai::host_route(&mesh, mirror.world(), actor, destination)`; after each
//! call `authority_hash() == history_hash(&mirror)`, and before each applied
//! `Act` the action is asserted to be in `legal_actions(mirror.world(), actor)`.
//!
//! The fixture is `tests/fixtures/three_combatants.json` edited in memory, as
//! `host_nav.rs` does.

use crpg_ai::{host_route, Decision, SideId, Sides};
use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_nav::{bake_box, NavMesh, NavRect};
use crpg_net::codec_v2;
use crpg_net::protocol::{IntentBody, IntentFrame, NetId, ReceiptStatus, LANE_COMBAT};
use crpg_net::protocol_v3::PauseReason;
use crpg_server::checkpoint::load_checkpoint;
use crpg_server::host::{
    AiBudget, AiReport, AiStep, ControlGrant, DeliveryEntry, DisclosureGrants, EntityDisclosure,
    Host, HostConfig, HostError, PeerHandle, ProtocolSelection, MVP_AI_BUDGET,
};
use crpg_sim::{
    history_hash, legal_actions, CombatAction, HistoryWorld, MoveRoute, NavRevision, RouteTag,
    StandingOrder,
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

/// A whole-metre floor point `[x, 0, z]`.
fn m(x: i32, z: i32) -> [Fx16_16; 3] {
    [Fx16_16::from_int(x), Fx16_16::ZERO, Fx16_16::from_int(z)]
}

/// A raw point, `[x, y, z]` in 1/65,536 m.
fn rp(x: i32, y: i32, z: i32) -> [Fx16_16; 3] {
    [
        Fx16_16::from_raw(x),
        Fx16_16::from_raw(y),
        Fx16_16::from_raw(z),
    ]
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

fn r(mesh: &NavMesh) -> NavRevision {
    NavRevision::of_bytes(&mesh.to_canonical_bytes())
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

/// The movement edits of `MV3`/`MV3rt` (`host_nav.rs`): three movers at
/// 1 m/tick with a 10 m step, their mirrors, and the one-`JY`-per-10 m rules.
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

/// The range-and-pool edits shared by every `AI3` fixture.
fn range_and_pool(mut value: Value, pool_max: u32) -> Value {
    for ability in value["world"]["combat"]["definition"]["abilities"]
        .as_array_mut()
        .expect("abilities")
    {
        ability["range"] = json!(98304);
    }
    value["world"]["combat"]["definition"]["pool_max"] = json!(pool_max);
    value["world"]["combat"]["definition"]["pools"][0]["max"] = json!(pool_max);
    for combatant in value["world"]["combatants"]
        .as_array_mut()
        .expect("combatants")
    {
        combatant[1]["action_pool"]["max"] = json!(pool_max);
        combatant[1]["action_pool"]["current"] = json!(pool_max);
    }
    value
}

/// `AI3`: `movement_edits(TB3)` with every ability at range 98,304 and pool
/// `JY` at max 2.
fn ai3_value() -> Value {
    range_and_pool(movement_edits(tb3_value()), 2)
}

/// `AI3¹`: the same edits without the pool edit (max 1).
fn ai3_once_value() -> Value {
    let mut value = movement_edits(tb3_value());
    for ability in value["world"]["combat"]["definition"]["abilities"]
        .as_array_mut()
        .expect("abilities")
    {
        ability["range"] = json!(98304);
    }
    value
}

/// `AI3rt`: the same edits on `rt3`.
fn ai3rt_value() -> Value {
    range_and_pool(movement_edits(rt3_value()), 2)
}

/// `AI3rt†`: `AI3rt` with C dead and removed from the timeline.
fn ai3rt_dagger_value() -> Value {
    let mut value = ai3rt_value();
    for combatant in value["world"]["combatants"]
        .as_array_mut()
        .expect("combatants")
    {
        if combatant[0]["index"].as_u64() == Some(2) {
            combatant[1]["health"] = json!(0);
            combatant[1]["dead"] = json!(true);
        }
    }
    value["world"]["timeline"]
        .as_array_mut()
        .expect("timeline")
        .retain(|entry| entry[1]["index"].as_u64() != Some(2));
    value
}

/// `FS3`: `TB3` with ability `JV` keeping the turn, pool `JY` at max 20, and
/// B's health raised to 1000.
fn fs3_value() -> Value {
    let mut value = tb3_value();
    for ability in value["world"]["combat"]["definition"]["abilities"]
        .as_array_mut()
        .expect("abilities")
    {
        if ability["ability"].as_str() == Some("000000000000000000000000JV") {
            ability["ends_turn"] = json!(false);
        }
    }
    value["world"]["combat"]["definition"]["pool_max"] = json!(20);
    value["world"]["combat"]["definition"]["pools"][0]["max"] = json!(20);
    for combatant in value["world"]["combatants"]
        .as_array_mut()
        .expect("combatants")
    {
        combatant[1]["action_pool"]["max"] = json!(20);
        combatant[1]["action_pool"]["current"] = json!(20);
        if combatant[0]["index"].as_u64() == Some(1) {
            combatant[1]["health"] = json!(1000);
            combatant[1]["max_health"] = json!(1000);
        }
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
// Sides, actions and report shorthands.
// ---------------------------------------------------------------------------

fn split(entries: &[(EntityId, u32)]) -> Sides {
    Sides::new(
        entries
            .iter()
            .map(|(entity, side)| (*entity, SideId(*side)))
            .collect(),
    )
    .expect("sides")
}

fn jv() -> Ulid {
    "000000000000000000000000JV".parse().expect("ulid")
}

fn use_ability(actor: EntityId, ability: Ulid, target: EntityId) -> CombatAction {
    CombatAction::UseAbility {
        actor,
        ability,
        target,
    }
}

fn end_turn(actor: EntityId) -> CombatAction {
    CombatAction::EndTurn { actor }
}

fn ap(actor: EntityId, decision: Decision) -> AiStep {
    AiStep::Applied { actor, decision }
}

fn rf(actor: EntityId, decision: Decision, error: HostError) -> AiStep {
    AiStep::Refused {
        actor,
        decision,
        error,
    }
}

fn report(steps: Vec<AiStep>, capped: bool) -> Result<AiReport, HostError> {
    Ok(AiReport { steps, capped })
}

/// The route `crpg_ai::host_route` builds for `destination` on the mirror.
fn route_to(
    mesh: &NavMesh,
    world: &HistoryWorld,
    actor: EntityId,
    destination: [Fx16_16; 3],
) -> MoveRoute {
    host_route(mesh, world.world(), actor, destination).expect("the AI's route")
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

    /// The mirror's `bind_nav`, acknowledged.
    fn mirror_bind_nav(&mut self, mesh: &NavMesh) {
        self.oracle.bind_nav(r(mesh)).expect("the mirror binds");
        self.mirror_ack();
    }

    fn mirror_place(&mut self, entity: EntityId, position: [Fx16_16; 3]) {
        self.oracle
            .place(entity, position)
            .expect("the mirror places");
        self.mirror_ack();
    }

    /// The mirror's `set_route`, acknowledged.
    fn mirror_set_route(&mut self, actor: EntityId, route: MoveRoute) {
        self.oracle
            .set_route(actor, route)
            .expect("the mirror sets");
        self.mirror_ack();
    }

    /// The mirror's `clear_route`, acknowledged; returns the cleared tag.
    fn mirror_clear_route(&mut self, actor: EntityId) -> Option<RouteTag> {
        let cleared = self.oracle.clear_route(actor).expect("the mirror clears");
        self.mirror_ack();
        cleared
    }

    fn mirror_tick(&mut self) {
        self.oracle.tick().expect("the mirror ticks");
        self.mirror_ack();
    }

    /// The mirror's `perform_move`, acknowledged.
    fn mirror_move(&mut self, actor: EntityId, route: MoveRoute) {
        self.oracle
            .perform_move(actor, &route)
            .expect("the mirror moves");
        self.mirror_ack();
    }

    /// The mirror's `perform_action`, acknowledged.
    fn mirror_action(&mut self, action: &CombatAction) {
        assert!(
            legal_actions(self.oracle.world(), action_actor(action))
                .expect("the actor may act")
                .contains(action),
            "the applied act is legal: {action:?}"
        );
        self.oracle
            .perform_action(action)
            .expect("the mirror applies");
        self.mirror_ack();
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

    /// One `advance` (pause-aware tick), mirrored and checked.
    fn advance(&mut self) {
        self.now += 100;
        self.host.advance(self.now).expect("advances");
        self.mirror_tick();
        self.assert_authority();
    }

    /// Binds control for A, B and C; every `FULL` flag.
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

/// The actor of a `CombatAction`, for the legality check.
fn action_actor(action: &CombatAction) -> EntityId {
    match action {
        CombatAction::UseAbility { actor, .. }
        | CombatAction::EndTurn { actor }
        | CombatAction::Stop { actor } => *actor,
    }
}

/// 4,096 `EndTurn` commands through the host and the mirror, so the capture
/// journal is full (`host_nav.rs` A7's pattern).
fn fill_journal(rig: &mut Rig, peer: PeerHandle) {
    for seq in 1..=4096_u64 {
        let active = rig
            .oracle
            .world()
            .combat()
            .expect("an encounter")
            .active
            .expect("a head");
        assert_eq!(rig.end_turn(peer, seq, active), ReceiptStatus::Applied);
        rig.oracle
            .perform_action(&CombatAction::EndTurn { actor: active })
            .expect("the mirror ends the same turn");
        rig.mirror_ack();
        rig.drain(peer);
    }
    assert_eq!(rig.host.last_capture(), 4096);
}

// ---------------------------------------------------------------------------
// H1.
// ---------------------------------------------------------------------------

#[test]
fn h1_tb_one_step_for_the_head() {
    let sides = split(&[(a(), 1), (b(), 2), (c(), 1)]);
    let mut rig = Rig::new(&tb3_value());

    let report1 = rig.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET);
    assert_eq!(
        report1,
        report(
            vec![ap(a(), Decision::Act(use_ability(a(), jv(), b())))],
            false
        )
    );
    rig.mirror_action(&use_ability(a(), jv(), b()));
    rig.assert_authority();
    assert_eq!(rig.host.awaiting_orders(), vec![b()]);

    // A second call: the head is B, which is not controlled.
    let before = rig.state(&[]);
    assert_eq!(
        rig.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(vec![], false)
    );
    rig.assert_unchanged(&before, &[]);

    // Control: `actors [B, A]` (unsorted) decides B now.
    assert_eq!(
        rig.host.drive_ai(&sides, &[b(), a()], MVP_AI_BUDGET),
        report(
            vec![ap(b(), Decision::Act(use_ability(b(), jv(), a())))],
            false
        )
    );
    rig.mirror_action(&use_ability(b(), jv(), a()));
    rig.assert_authority();

    // Control: a fresh `TB3` with everyone on one side ends the turn.
    let all_same = split(&[(a(), 1), (b(), 1), (c(), 1)]);
    let mut rig2 = Rig::new(&tb3_value());
    assert_eq!(
        rig2.host.drive_ai(&all_same, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Act(end_turn(a())))], false)
    );
    rig2.mirror_action(&end_turn(a()));
    rig2.assert_authority();
}

// ---------------------------------------------------------------------------
// H2.
// ---------------------------------------------------------------------------

#[test]
fn h2_tb_walk_then_strike() {
    let sides = split(&[(a(), 1), (b(), 2), (c(), 1)]);
    let mut rig = Rig::new(&ai3_value());
    rig.host.bind_nav_mesh(m10()).expect("binds");
    rig.mirror_bind_nav(&m10());
    rig.assert_authority();

    // Call 1: the move toward B.
    let dest = rp(98368, 0, 65536);
    assert_eq!(
        rig.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(
            vec![ap(
                a(),
                Decision::Move {
                    target: b(),
                    destination: dest,
                }
            )],
            false
        )
    );
    rig.mirror_move(a(), route_to(&m10(), &rig.oracle, a(), dest));
    rig.assert_authority();
    assert_eq!(rig.host.awaiting_orders(), vec![a()]);

    // Call 2: the strike.
    assert_eq!(
        rig.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(
            vec![ap(a(), Decision::Act(use_ability(a(), jv(), b())))],
            false
        )
    );
    rig.mirror_action(&use_ability(a(), jv(), b()));
    rig.assert_authority();
    assert_eq!(rig.host.awaiting_orders(), vec![b()]);

    // A twin host given the same calls returns equal reports and hashes.
    let mut twin = Rig::new(&ai3_value());
    twin.host.bind_nav_mesh(m10()).expect("binds");
    twin.mirror_bind_nav(&m10());
    assert_eq!(
        twin.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(
            vec![ap(
                a(),
                Decision::Move {
                    target: b(),
                    destination: dest,
                }
            )],
            false
        )
    );
    twin.mirror_move(a(), route_to(&m10(), &twin.oracle, a(), dest));
    assert_eq!(
        twin.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(
            vec![ap(a(), Decision::Act(use_ability(a(), jv(), b())))],
            false
        )
    );
    twin.mirror_action(&use_ability(a(), jv(), b()));
    twin.assert_authority();
    assert_eq!(twin.host.authority_hash(), rig.host.authority_hash());

    // Control: `AI3¹` (pool max 1) leaves no step, so the turn ends.
    let mut once = Rig::new(&ai3_once_value());
    once.host.bind_nav_mesh(m10()).expect("binds");
    once.mirror_bind_nav(&m10());
    assert_eq!(
        once.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Act(end_turn(a())))], false)
    );
    once.mirror_action(&end_turn(a()));
    once.assert_authority();

    // Control: `AI3` with no mesh ends the turn.
    let mut bare = Rig::new(&ai3_value());
    assert_eq!(
        bare.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Act(end_turn(a())))], false)
    );
    bare.mirror_action(&end_turn(a()));
    bare.assert_authority();
}

// ---------------------------------------------------------------------------
// H3.
// ---------------------------------------------------------------------------

#[test]
fn h3_tb_turn_cap_counts_across_calls() {
    let sides = split(&[(a(), 1), (b(), 2), (c(), 1)]);
    let budget = AiBudget {
        decisions_per_turn: 1,
        actors_per_drive: 8,
    };
    let mut rig = Rig::new(&ai3_value());
    rig.host.bind_nav_mesh(m10()).expect("binds");
    rig.mirror_bind_nav(&m10());

    // Call 1: the move (H2's), uncapped.
    let dest = rp(98368, 0, 65536);
    assert_eq!(
        rig.host.drive_ai(&sides, &[a(), b()], budget),
        report(
            vec![ap(
                a(),
                Decision::Move {
                    target: b(),
                    destination: dest,
                }
            )],
            false
        )
    );
    rig.mirror_move(a(), route_to(&m10(), &rig.oracle, a(), dest));
    rig.assert_authority();

    // Call 2: at the cap, the head's turn ends.
    assert_eq!(
        rig.host.drive_ai(&sides, &[a(), b()], budget),
        report(vec![ap(a(), Decision::Act(end_turn(a())))], true)
    );
    rig.mirror_action(&end_turn(a()));
    rig.assert_authority();
    assert_eq!(rig.host.awaiting_orders(), vec![b()]);

    // Call 3: B's turn, a fresh count; B strikes A.
    assert_eq!(
        rig.host.drive_ai(&sides, &[a(), b()], budget),
        report(
            vec![ap(b(), Decision::Act(use_ability(b(), jv(), a())))],
            false
        )
    );
    rig.mirror_action(&use_ability(b(), jv(), a()));
    rig.assert_authority();

    // Control: budget {2, 8} gives H2's two steps uncapped.
    let budget2 = AiBudget {
        decisions_per_turn: 2,
        actors_per_drive: 8,
    };
    let mut wide = Rig::new(&ai3_value());
    wide.host.bind_nav_mesh(m10()).expect("binds");
    wide.mirror_bind_nav(&m10());
    assert_eq!(
        wide.host.drive_ai(&sides, &[a(), b()], budget2),
        report(
            vec![ap(
                a(),
                Decision::Move {
                    target: b(),
                    destination: dest,
                }
            )],
            false
        )
    );
    wide.mirror_move(a(), route_to(&m10(), &wide.oracle, a(), dest));
    assert_eq!(
        wide.host.drive_ai(&sides, &[a(), b()], budget2),
        report(
            vec![ap(a(), Decision::Act(use_ability(a(), jv(), b())))],
            false
        )
    );
    wide.mirror_action(&use_ability(a(), jv(), b()));
    wide.assert_authority();

    // Control: budget {0, 8} on a fresh host ends the turn at once.
    let zero = AiBudget {
        decisions_per_turn: 0,
        actors_per_drive: 8,
    };
    let mut fresh = Rig::new(&ai3_value());
    fresh.host.bind_nav_mesh(m10()).expect("binds");
    fresh.mirror_bind_nav(&m10());
    assert_eq!(
        fresh.host.drive_ai(&sides, &[a(), b()], zero),
        report(vec![ap(a(), Decision::Act(end_turn(a())))], true)
    );
    fresh.mirror_action(&end_turn(a()));
    fresh.assert_authority();
    assert_eq!(
        fresh
            .oracle
            .world()
            .movers()
            .get(a())
            .expect("mover")
            .position(),
        m(1, 1),
        "A has not moved"
    );
}

// ---------------------------------------------------------------------------
// H4.
// ---------------------------------------------------------------------------

#[test]
fn h4_mvp_turn_cap_is_sixteen() {
    let sides = split(&[(a(), 1), (b(), 2), (c(), 1)]);
    let mut rig = Rig::new(&fs3_value());

    for _ in 0..16 {
        assert_eq!(
            rig.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
            report(
                vec![ap(a(), Decision::Act(use_ability(a(), jv(), b())))],
                false
            )
        );
        rig.mirror_action(&use_ability(a(), jv(), b()));
        rig.assert_authority();
        assert_eq!(rig.host.awaiting_orders(), vec![a()]);
    }

    assert_eq!(
        rig.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Act(end_turn(a())))], true)
    );
    rig.mirror_action(&end_turn(a()));
    rig.assert_authority();
    assert_eq!(rig.host.awaiting_orders(), vec![b()]);

    let b_state = rig.oracle.world().combatants().get(b()).expect("B exists");
    assert_eq!(b_state.health(), 1000 - 16 * 3);
    let a_state = rig.oracle.world().combatants().get(a()).expect("A exists");
    assert_eq!(a_state.action_pool().current(), 4);
}

// ---------------------------------------------------------------------------
// H5.
// ---------------------------------------------------------------------------

#[test]
fn h5_rt_walk_strike_hold_and_replan() {
    let sides = split(&[(a(), 1), (b(), 2), (c(), 1)]);
    let mut rig = Rig::new(&ai3rt_value());
    rig.host.bind_nav_mesh(m10()).expect("binds");
    rig.mirror_bind_nav(&m10());
    rig.assert_authority();

    // Call 1: move_to toward B.
    let dest = rp(98368, 0, 65536);
    assert_eq!(
        rig.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(
            vec![ap(
                a(),
                Decision::Move {
                    target: b(),
                    destination: dest,
                }
            )],
            false
        )
    );
    rig.mirror_set_route(a(), route_to(&m10(), &rig.oracle, a(), dest));
    rig.assert_authority();

    // One advance: A arrives and its route ends.
    rig.advance();

    // Call 2: the strike order.
    assert_eq!(
        rig.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(
            vec![ap(a(), Decision::Act(use_ability(a(), jv(), b())))],
            false
        )
    );
    rig.mirror_action(&use_ability(a(), jv(), b()));
    rig.assert_authority();
    assert_eq!(
        rig.oracle
            .world()
            .combatants()
            .get(a())
            .expect("A exists")
            .order(),
        Some(StandingOrder {
            ability: jv(),
            target: b(),
            repeat: false,
        })
    );

    // Call 3: the order stands, so A holds.
    let before = rig.state(&[]);
    assert_eq!(
        rig.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Hold)], false)
    );
    rig.assert_unchanged(&before, &[]);

    // B walks out of range; A re-plans a move toward it.
    rig.host.place(b(), m(9, 1)).expect("places");
    rig.mirror_place(b(), m(9, 1));
    rig.assert_authority();
    assert!(!rig.host.awaiting_orders().contains(&a()));
    let replan = rp(491584, 0, 65536);
    assert_eq!(
        rig.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(
            vec![ap(
                a(),
                Decision::Move {
                    target: b(),
                    destination: replan,
                }
            )],
            false
        )
    );
    rig.mirror_set_route(a(), route_to(&m10(), &rig.oracle, a(), replan));
    rig.assert_authority();
}

// ---------------------------------------------------------------------------
// H6.
// ---------------------------------------------------------------------------

#[test]
fn h6_rt_halt_before_strike() {
    let sides = split(&[(a(), 1), (b(), 2), (c(), 1)]);
    let mut rig = Rig::new(&ai3rt_value());
    rig.host.bind_nav_mesh(m10()).expect("binds");
    rig.mirror_bind_nav(&m10());
    rig.host.place(b(), m(2, 1)).expect("places");
    rig.mirror_place(b(), m(2, 1));
    rig.host.move_to(a(), m(1, 4)).expect("routes");
    rig.mirror_set_route(a(), route_to(&m10(), &rig.oracle, a(), m(1, 4)));
    rig.assert_authority();

    // Call 1: halt clears the route.
    assert_eq!(
        rig.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Halt)], false)
    );
    assert_eq!(rig.mirror_clear_route(a()), Some(RouteTag(0)));
    rig.assert_authority();
    assert!(rig
        .oracle
        .world()
        .movers()
        .get(a())
        .expect("mover")
        .route()
        .is_none());

    // Call 2: the strike (65,536 raw is inside 98,304).
    assert_eq!(
        rig.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(
            vec![ap(a(), Decision::Act(use_ability(a(), jv(), b())))],
            false
        )
    );
    rig.mirror_action(&use_ability(a(), jv(), b()));
    rig.assert_authority();
}

// ---------------------------------------------------------------------------
// H7.
// ---------------------------------------------------------------------------

#[test]
fn h7_rt_candidates_and_rotation() {
    let sides = split(&[(a(), 1), (b(), 2), (c(), 1)]);
    let actors = [c(), e7(), a(), b()];

    // No mesh: every candidate's answer is `Hold`; e7 is never decided.
    let mut rig = Rig::new(&ai3rt_value());
    assert_eq!(
        rig.host.drive_ai(&sides, &actors, MVP_AI_BUDGET),
        report(
            vec![
                ap(a(), Decision::Hold),
                ap(b(), Decision::Hold),
                ap(c(), Decision::Hold)
            ],
            false
        )
    );
    rig.assert_authority();

    // Budget {16, 1}: one candidate per call, rotating with the world tick.
    let budget1 = AiBudget {
        decisions_per_turn: 16,
        actors_per_drive: 1,
    };
    let mut rotated = Rig::new(&ai3rt_value());
    let mut seen = Vec::new();
    for _ in 0..4 {
        let step = rotated
            .host
            .drive_ai(&sides, &actors, budget1)
            .expect("drives");
        assert!(step.capped);
        assert_eq!(step.steps.len(), 1);
        match &step.steps[0] {
            AiStep::Applied {
                actor,
                decision: Decision::Hold,
            } => {
                let name = if *actor == a() {
                    "A"
                } else if *actor == b() {
                    "B"
                } else if *actor == c() {
                    "C"
                } else {
                    panic!("unexpected actor {actor:?}");
                };
                seen.push(name);
            }
            other => panic!("expected an applied Hold, got {other:?}"),
        }
        rotated.advance();
    }
    assert_eq!(seen, vec!["A", "B", "C", "A"]);

    // Budget {16, 2} at tick 2 starts at C and wraps to A.
    let budget2 = AiBudget {
        decisions_per_turn: 16,
        actors_per_drive: 2,
    };
    let mut tick2 = Rig::new(&ai3rt_value());
    tick2.advance();
    tick2.advance();
    assert_eq!(
        tick2.host.drive_ai(&sides, &actors, budget2),
        report(vec![ap(c(), Decision::Hold), ap(a(), Decision::Hold)], true)
    );

    // `AI3rt†`: C is dead, so it is not decided.
    let mut dagger = Rig::new(&ai3rt_dagger_value());
    assert_eq!(
        dagger
            .host
            .drive_ai(&sides, &[a(), b(), c()], MVP_AI_BUDGET),
        report(
            vec![ap(a(), Decision::Hold), ap(b(), Decision::Hold)],
            false
        )
    );

    // Control: `TB3` with the same actors decides only the head.
    let mut tb = Rig::new(&tb3_value());
    assert_eq!(
        tb.host.drive_ai(&sides, &actors, MVP_AI_BUDGET),
        report(
            vec![ap(a(), Decision::Act(use_ability(a(), jv(), b())))],
            false
        )
    );
}

// ---------------------------------------------------------------------------
// H8.
// ---------------------------------------------------------------------------

#[test]
fn h8_refusal_is_a_step() {
    let sides = split(&[(a(), 1), (b(), 2), (c(), 1)]);

    // A full capture journal leaves B holding the turn.
    let mut rig = Rig::new(&tb3_value());
    let peer = rig.bind_v();
    fill_journal(&mut rig, peer);
    rig.assert_authority();
    assert_eq!(
        rig.oracle.world().combat().expect("an encounter").active,
        Some(b())
    );

    let before = rig.state(&[peer]);
    let expected = report(
        vec![rf(
            b(),
            Decision::Act(use_ability(b(), jv(), a())),
            HostError::CaptureFull,
        )],
        false,
    );
    assert_eq!(rig.host.drive_ai(&sides, &[b()], MVP_AI_BUDGET), expected);
    rig.assert_unchanged(&before, &[peer]);
    // A second call returns the same report, unchanged.
    assert_eq!(rig.host.drive_ai(&sides, &[b()], MVP_AI_BUDGET), expected);
    rig.assert_unchanged(&before, &[peer]);

    // Control: after acknowledging the journal, the strike applies.
    rig.host.acknowledge_captures(4096).expect("acks");
    assert_eq!(
        rig.host.drive_ai(&sides, &[b()], MVP_AI_BUDGET),
        report(
            vec![ap(b(), Decision::Act(use_ability(b(), jv(), a())))],
            false
        )
    );
    rig.mirror_action(&use_ability(b(), jv(), a()));
    rig.assert_authority();

    // Cap case (D-5(1)): a refused forced `EndTurn` reports `capped` true.
    let mut cap = Rig::new(&tb3_value());
    let cap_peer = cap.bind_v();
    fill_journal(&mut cap, cap_peer);
    let zero = AiBudget {
        decisions_per_turn: 0,
        actors_per_drive: 8,
    };
    let before_cap = cap.state(&[cap_peer]);
    let expected_cap = report(
        vec![rf(
            b(),
            Decision::Act(end_turn(b())),
            HostError::CaptureFull,
        )],
        true,
    );
    assert_eq!(cap.host.drive_ai(&sides, &[b()], zero), expected_cap);
    cap.assert_unchanged(&before_cap, &[cap_peer]);
    assert_eq!(cap.host.awaiting_orders(), vec![b()]);
    // The count was not incremented: a second call returns the same report.
    assert_eq!(cap.host.drive_ai(&sides, &[b()], zero), expected_cap);
    cap.assert_unchanged(&before_cap, &[cap_peer]);

    // Control: after acknowledging, the forced `EndTurn` applies.
    cap.host.acknowledge_captures(4096).expect("acks");
    assert_eq!(
        cap.host.drive_ai(&sides, &[b()], zero),
        report(vec![ap(b(), Decision::Act(end_turn(b())))], true)
    );
    cap.mirror_action(&end_turn(b()));
    cap.assert_authority();
    assert_eq!(cap.host.awaiting_orders(), vec![c()]);
}

// ---------------------------------------------------------------------------
// H9.
// ---------------------------------------------------------------------------

#[test]
fn h9_pause_does_not_stop_the_driver() {
    let sides = split(&[(a(), 1), (b(), 2), (c(), 1)]);
    let mut rig = Rig::new(&tb3_value());
    rig.host.pause(1).expect("pauses");

    assert_eq!(
        rig.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(
            vec![ap(a(), Decision::Act(use_ability(a(), jv(), b())))],
            false
        )
    );
    rig.mirror_action(&use_ability(a(), jv(), b()));
    rig.assert_authority();
    assert_eq!(rig.host.paused(), Some(PauseReason::Operator));
}

// ---------------------------------------------------------------------------
// H10.
// ---------------------------------------------------------------------------

#[test]
fn h10_closed_and_no_fight() {
    let sides = split(&[(a(), 1), (b(), 2), (c(), 1)]);

    // A host with no encounter: nothing to do.
    let mut empty = Host::new(7, config(), 0).expect("new");
    assert_eq!(
        empty.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(vec![], false)
    );

    // A shut-down host refuses, whatever the actors.
    let mut rig = Rig::new(&tb3_value());
    rig.host.shutdown();
    assert_eq!(
        rig.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        Err(HostError::Closed)
    );
    assert_eq!(
        rig.host.drive_ai(&sides, &[], MVP_AI_BUDGET),
        Err(HostError::Closed)
    );

    // Control: before shutdown, `actors [A]` is H1's report.
    let mut control = Rig::new(&tb3_value());
    assert_eq!(
        control.host.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(
            vec![ap(a(), Decision::Act(use_ability(a(), jv(), b())))],
            false
        )
    );
    control.mirror_action(&use_ability(a(), jv(), b()));
    control.assert_authority();
}

// ---------------------------------------------------------------------------
// H11.
// ---------------------------------------------------------------------------

#[test]
fn h11_restore_restarts_the_turn_count() {
    let sides = split(&[(a(), 1), (b(), 2), (c(), 1)]);
    let budget = AiBudget {
        decisions_per_turn: 2,
        actors_per_drive: 8,
    };
    let mut rig = Rig::new(&fs3_value());

    for _ in 0..2 {
        assert_eq!(
            rig.host.drive_ai(&sides, &[a()], budget),
            report(
                vec![ap(a(), Decision::Act(use_ability(a(), jv(), b())))],
                false
            )
        );
        rig.mirror_action(&use_ability(a(), jv(), b()));
        rig.assert_authority();
        assert_eq!(rig.host.awaiting_orders(), vec![a()]);
    }

    let saved = rig.host.save_checkpoint().expect("saves");
    let mut restored = load_checkpoint(&saved, 2, 0).expect("the save loads");

    // Call 3 on the restored host: the count restarted, so it strikes again.
    assert_eq!(
        restored.drive_ai(&sides, &[a()], budget),
        report(
            vec![ap(a(), Decision::Act(use_ability(a(), jv(), b())))],
            false
        )
    );
    rig.mirror_action(&use_ability(a(), jv(), b()));
    assert_eq!(restored.authority_hash(), history_hash(&rig.oracle));
    assert_eq!(restored.awaiting_orders(), vec![a()]);

    // Call 4: the restored count is 1, so it strikes a second time. D-5
    // restarts the restored count at zero, so with `decisions_per_turn: 2`
    // the cap is reached one call later than the contract's H11 literal
    // ("call 4 = EndTurn", three strikes): a reset count of two needs two
    // restored strikes before the forced `EndTurn`.
    assert_eq!(
        restored.drive_ai(&sides, &[a()], budget),
        report(
            vec![ap(a(), Decision::Act(use_ability(a(), jv(), b())))],
            false
        )
    );
    rig.mirror_action(&use_ability(a(), jv(), b()));
    assert_eq!(restored.authority_hash(), history_hash(&rig.oracle));
    assert_eq!(restored.awaiting_orders(), vec![a()]);

    // Call 5: the restored count reaches 2, so the turn ends.
    assert_eq!(
        restored.drive_ai(&sides, &[a()], budget),
        report(vec![ap(a(), Decision::Act(end_turn(a())))], true)
    );
    rig.mirror_action(&end_turn(a()));
    assert_eq!(restored.authority_hash(), history_hash(&rig.oracle));
    assert_eq!(restored.awaiting_orders(), vec![b()]);

    let b_state = rig.oracle.world().combatants().get(b()).expect("B exists");
    assert_eq!(b_state.health(), 1000 - 4 * 3);
    let a_state = rig.oracle.world().combatants().get(a()).expect("A exists");
    assert_eq!(a_state.action_pool().current(), 16);

    // Control: no restore, so call 3 hits the cap and ends the turn.
    let mut control = Rig::new(&fs3_value());
    for _ in 0..2 {
        assert_eq!(
            control.host.drive_ai(&sides, &[a()], budget),
            report(
                vec![ap(a(), Decision::Act(use_ability(a(), jv(), b())))],
                false
            )
        );
        control.mirror_action(&use_ability(a(), jv(), b()));
        control.assert_authority();
    }
    assert_eq!(
        control.host.drive_ai(&sides, &[a()], budget),
        report(vec![ap(a(), Decision::Act(end_turn(a())))], true)
    );
    control.mirror_action(&end_turn(a()));
    control.assert_authority();
    assert_eq!(control.host.awaiting_orders(), vec![b()]);
}
