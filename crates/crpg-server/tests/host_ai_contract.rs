//! Two-keys contract suite for T153 (`tasks/T153.md` Behaviour D-1..D-12 and
//! Errors), written from the contract text alone. The rig is copied from
//! `tests/host_nav.rs`. Every mirrored move uses
//! `crpg_ai::host_route` on the mirror, so a divergent host route splits the
//! authority hash. Expected values are the contract literals; the H11 numbers
//! are the session 77 erratum (call 3 strike, call 4 strike, call 5 EndTurn,
//! B health 988, A pool 16).

use crpg_ai::{Decision, SideId, Sides, AI_ROUTE_TAG};
use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_nav::{bake_box, NavMesh, NavRect};
use crpg_net::codec_v2;
use crpg_net::protocol::{IntentBody, IntentFrame, NetId, ReceiptStatus, LANE_COMBAT};
use crpg_net::protocol_v3::PauseReason;
use crpg_server::checkpoint::load_checkpoint;
use crpg_server::host::{
    AiBudget, AiReport, AiStep, ControlGrant, DisclosureGrants, EntityDisclosure, Host, HostConfig,
    HostError, PeerHandle, ProtocolSelection, HOST_ROUTE_TAG, MVP_AI_BUDGET,
};
use crpg_sim::{
    history_hash, legal_actions, CombatAction, CombatError, HistoryWorld, LegalActionsError,
    MoveRoute, NavRevision, RouteTag, RT_CLOCK_MAX,
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
fn strike() -> Ulid {
    Ulid::from_u128(603)
}

/// A whole-metre floor point `[x, 0, z]`.
fn m(x: i32, z: i32) -> [Fx16_16; 3] {
    [Fx16_16::from_int(x), Fx16_16::ZERO, Fx16_16::from_int(z)]
}

/// Raw point `[x, y, z]` in 1/65536 m.
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

// ---------------------------------------------------------------------------
// Fixtures (in-memory edits of the saved-game JSON).
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

/// The movement edits of `host_nav.rs`: three movers at 1 m per tick with a
/// 10 m step, their mirrors, and the one-unit-per-10 m rule.
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

fn set_ability_range(value: &mut Value, range: u64) {
    let abilities = value["world"]["combat"]["definition"]["abilities"]
        .as_array_mut()
        .expect("abilities");
    for ability in abilities.iter_mut() {
        ability["range"] = json!(range);
    }
}

fn set_pool(value: &mut Value, max: u32, current: u32) {
    value["world"]["combat"]["definition"]["pool_max"] = json!(max);
    value["world"]["combat"]["definition"]["pools"][0]["max"] = json!(max);
    let combatants = value["world"]["combatants"]
        .as_array_mut()
        .expect("combatants");
    for pair in combatants.iter_mut() {
        pair[1]["action_pool"]["max"] = json!(max);
        pair[1]["action_pool"]["current"] = json!(current);
    }
}

/// `AI3`: movement plus range 98304 and pool JY at max 2.
fn ai3_value() -> Value {
    let mut value = movement_edits(tb3_value());
    set_ability_range(&mut value, 98304);
    set_pool(&mut value, 2, 2);
    value
}

/// `AI3-1`: the same without the pool edit (max 1).
fn ai3_1_value() -> Value {
    let mut value = movement_edits(tb3_value());
    set_ability_range(&mut value, 98304);
    value
}

/// `AI3rt`: the same edits on `rt3`.
fn ai3rt_value() -> Value {
    let mut value = movement_edits(rt3_value());
    set_ability_range(&mut value, 98304);
    set_pool(&mut value, 2, 2);
    value
}

/// `AI3rt-dagger`: `AI3rt` with C dead and off the timeline.
fn ai3rt_dagger_value() -> Value {
    let mut value = ai3rt_value();
    value["world"]["combatants"][2][1]["health"] = json!(0);
    value["world"]["combatants"][2][1]["dead"] = json!(true);
    let timeline = value["world"]["timeline"].as_array_mut().expect("timeline");
    timeline.retain(|entry| entry[1]["index"].as_u64() != Some(2));
    value
}

/// `AI3rt` at a chosen clock value (the save's `clock.elapsed`).
fn ai3rt_elapsed_value(elapsed: u64) -> Value {
    let mut value = ai3rt_value();
    value["world"]["combat"]["clock"]["elapsed"] = json!(elapsed);
    value
}

/// `AI3rt-solo`: B and C dead and off the timeline, so exactly one combatant
/// is alive (the real-time timeline lists exactly the live combatants).
fn ai3rt_solo_value() -> Value {
    let mut value = ai3rt_value();
    for slot in [1_usize, 2] {
        value["world"]["combatants"][slot][1]["health"] = json!(0);
        value["world"]["combatants"][slot][1]["dead"] = json!(true);
    }
    let timeline = value["world"]["timeline"]
        .as_array()
        .expect("timeline")
        .iter()
        .filter(|entry| entry[1]["index"].as_u64() == Some(0))
        .cloned()
        .collect::<Vec<Value>>();
    value["world"]["timeline"] = json!(timeline);
    value
}

/// `AI3rt-swing`: a real-time world whose swings land on every tick
/// (`round_ticks` 1) and in which B survives the 4096 swings that fill the
/// capture journal (10 → 20000 health, pool JY large enough for every order).
fn ai3rt_swing_value() -> Value {
    let mut value = ai3rt_value();
    value["world"]["combat"]["clock"] = json!({"round_ticks": 1, "elapsed": 0});
    value["world"]["combatants"][1][1]["health"] = json!(20_000);
    value["world"]["combatants"][1][1]["max_health"] = json!(20_000);
    set_pool(&mut value, 5000, 5000);
    value
}

/// `FS3`: `TB3` with a non-ending strike, pool JY at max 20, B at 1000 health.
fn fs3_value() -> Value {
    let mut value = tb3_value();
    value["world"]["combat"]["definition"]["abilities"][0]["ends_turn"] = json!(false);
    set_pool(&mut value, 20, 20);
    value["world"]["combatants"][1][1]["health"] = json!(1000);
    value["world"]["combatants"][1][1]["max_health"] = json!(1000);
    value
}

fn sides_abc() -> Sides {
    Sides::new(vec![(a(), SideId(1)), (b(), SideId(2)), (c(), SideId(1))]).expect("sides")
}

fn sides_all_same() -> Sides {
    Sides::new(vec![(a(), SideId(1)), (b(), SideId(1)), (c(), SideId(1))]).expect("sides")
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

// ---------------------------------------------------------------------------
// Rig: one host and its mirror authority.
// ---------------------------------------------------------------------------

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

fn m10() -> NavMesh {
    bake_box(Fx16_16::ZERO, rect(0, 0, 10, 10), &[]).expect("bakes")
}

fn r(mesh: &NavMesh) -> NavRevision {
    NavRevision::of_bytes(&mesh.to_canonical_bytes())
}

/// Everything a refused or empty drive must leave alone.
#[derive(Debug, PartialEq, Eq)]
struct State {
    hash: [u8; 32],
    last_capture: u64,
    executions: u64,
    nav: Option<NavRevision>,
}

fn state_of(host: &mut Host) -> State {
    State {
        hash: host.authority_hash(),
        last_capture: host.last_capture(),
        executions: host.executions(),
        nav: host.nav_revision(),
    }
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

    fn mirror_bind_nav(&mut self, mesh: &NavMesh) {
        let cleared = self
            .oracle
            .bind_nav(r(mesh))
            .expect("the mirror binds the revision");
        let _ = cleared;
        self.mirror_ack();
    }

    fn mirror_place(&mut self, entity: EntityId, position: [Fx16_16; 3]) {
        self.oracle
            .place(entity, position)
            .expect("the mirror places");
        self.mirror_ack();
    }

    fn mirror_set_route(&mut self, actor: EntityId, route: MoveRoute) {
        self.oracle
            .set_route(actor, route)
            .expect("the mirror accepts the route");
        self.mirror_ack();
    }

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

    fn mirror_move(&mut self, actor: EntityId, route: MoveRoute) {
        self.oracle
            .perform_move(actor, &route)
            .expect("the mirror moves");
        self.mirror_ack();
    }

    fn mirror_act(&mut self, action: &CombatAction) {
        self.oracle.perform_action(action).expect("the mirror acts");
        self.mirror_ack();
    }

    /// Assert `action` is an element of the mirror's legal options, then act.
    fn mirror_checked_act(&mut self, actor: EntityId, action: &CombatAction) {
        let options = legal_actions(self.oracle.world(), actor).expect("actor may act");
        assert!(options.contains(action));
        self.mirror_act(action);
    }

    fn assert_authority(&self) {
        assert_eq!(self.host.authority_hash(), history_hash(&self.oracle));
    }

    fn assert_state(&mut self, before: &State) {
        assert_eq!(self.host.authority_hash(), before.hash);
        assert_eq!(self.host.last_capture(), before.last_capture);
        assert_eq!(self.host.executions(), before.executions);
        assert_eq!(self.host.nav_revision(), before.nav);
    }

    fn mover_position(&self, entity: EntityId) -> [Fx16_16; 3] {
        self.oracle
            .world()
            .movers()
            .get(entity)
            .expect("the mirror holds the mover")
            .position()
    }

    fn bind_v(&mut self) -> PeerHandle {
        let peer = self
            .host
            .bind_peer(
                control(&[a(), b(), c()]),
                full_grants(&[a(), b(), c()]),
                self.now,
            )
            .expect("binds");
        let entries = self.host.take_delivery_entries(peer).expect("bound");
        if let Some(last) = entries.last() {
            self.host
                .acknowledge_delivery(peer, last.ordinal)
                .expect("acks");
        }
        peer
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

/// 4096 `EndTurn` commands through host and mirror: the capture journal is full.
fn fill_journal(rig: &mut Rig, peer: PeerHandle) {
    use crpg_net::protocol::ReceiptStatus as RS;
    for seq in 1..=4096_u64 {
        let active = rig
            .oracle
            .world()
            .combat()
            .expect("an encounter")
            .active
            .expect("a head");
        assert_eq!(rig.end_turn(peer, seq, active), RS::Applied);
        rig.oracle
            .perform_action(&CombatAction::EndTurn { actor: active })
            .expect("the mirror ends the same turn");
        rig.mirror_ack();
        let entries = rig.host.take_delivery_entries(peer).expect("bound");
        if let Some(last) = entries.last() {
            rig.host
                .acknowledge_delivery(peer, last.ordinal)
                .expect("acks");
        }
    }
    assert_eq!(rig.host.last_capture(), 4096);
}

fn drive(
    rig: &mut Rig,
    sides: &Sides,
    actors: &[EntityId],
    budget: AiBudget,
) -> Result<AiReport, HostError> {
    rig.host.drive_ai(sides, actors, budget)
}

// ---------------------------------------------------------------------------
// Constants: MVP budget and route tags (D-12, U2).
// ---------------------------------------------------------------------------

#[test]
fn constants_match_the_contract() {
    assert_eq!(
        MVP_AI_BUDGET,
        AiBudget {
            decisions_per_turn: 16,
            actors_per_drive: 8
        }
    );
    assert_eq!(AI_ROUTE_TAG, RouteTag(0));
    assert_eq!(HOST_ROUTE_TAG, RouteTag(0));
    assert_eq!(AI_ROUTE_TAG, HOST_ROUTE_TAG);
}

// ---------------------------------------------------------------------------
// H1: turn-based one step for the head (D-3, D-4, D-5, D-10).
// ---------------------------------------------------------------------------

#[test]
fn h1_tb_one_step_for_the_head() {
    let sides = sides_abc();
    let mut rig = Rig::new(&tb3_value());
    let expected = use_ability(a(), strike(), b());

    // C is A's ally and nothing moves, so the least hostile target (B) and
    // the least ability (strike) win.
    assert_eq!(
        drive(&mut rig, &sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Act(expected))], false)
    );
    rig.mirror_checked_act(a(), &expected);
    rig.assert_authority();
    assert_eq!(rig.host.awaiting_orders(), vec![b()]);

    // A second call: B holds the turn and is not controlled.
    let before = state_of(&mut rig.host);
    assert_eq!(
        drive(&mut rig, &sides, &[a()], MVP_AI_BUDGET),
        report(vec![], false)
    );
    rig.assert_state(&before);
    rig.assert_authority();

    // Control: the unsorted input decides the same way for the new head B.
    // B's hostiles A and C tie, so least A wins.
    let b_strike = use_ability(b(), strike(), a());
    assert_eq!(
        drive(&mut rig, &sides, &[b(), a()], MVP_AI_BUDGET),
        report(vec![ap(b(), Decision::Act(b_strike))], false)
    );
    rig.mirror_checked_act(b(), &b_strike);
    rig.assert_authority();
    assert_eq!(rig.host.awaiting_orders(), vec![c()]);

    // Control: nobody is hostile, so the head ends its turn.
    let mut calm = Rig::new(&tb3_value());
    let end = end_turn(a());
    assert_eq!(
        drive(&mut calm, &sides_all_same(), &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Act(end))], false)
    );
    calm.mirror_checked_act(a(), &end);
    calm.assert_authority();
    assert_eq!(calm.host.awaiting_orders(), vec![b()]);
}

// ---------------------------------------------------------------------------
// H2: turn-based walk then strike (D-5, D-7, D-10, D-11, D-12).
// ---------------------------------------------------------------------------

#[test]
fn h2_tb_walk_then_strike() {
    let sides = sides_abc();
    let mesh = m10();
    let dest = rp(98368, 0, 65536);
    let mut rig = Rig::new(&ai3_value());
    rig.host.bind_nav_mesh(mesh.clone()).expect("binds");
    rig.mirror_bind_nav(&mesh);

    // Call 1: the approach stops one standoff inside range; the pool of 2
    // less the strike reserve 1 leaves one step, so the walk stands.
    let walk = Decision::Move {
        target: b(),
        destination: dest,
    };
    assert_eq!(
        drive(&mut rig, &sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), walk)], false)
    );
    let route = crpg_ai::host_route(&mesh, rig.oracle.world(), a(), dest).expect("a route");
    assert_eq!(route.points, vec![m(1, 1), dest]);
    assert_eq!(route.tag, RouteTag(0));
    rig.mirror_move(a(), route);
    rig.assert_authority();
    assert_eq!(rig.host.awaiting_orders(), vec![a()]);

    // Call 2: A now stands 98240 raw from B, inside 98304, so it strikes.
    let hit = use_ability(a(), strike(), b());
    assert_eq!(
        drive(&mut rig, &sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Act(hit))], false)
    );
    rig.mirror_checked_act(a(), &hit);
    rig.assert_authority();
    assert_eq!(rig.host.awaiting_orders(), vec![b()]);

    // A twin host given the same calls returns equal reports and hashes.
    let mut twin = Rig::new(&ai3_value());
    twin.host.bind_nav_mesh(mesh.clone()).expect("binds");
    twin.mirror_bind_nav(&mesh);
    let r1 = drive(&mut twin, &sides, &[a()], MVP_AI_BUDGET).expect("drives");
    assert_eq!(
        r1,
        AiReport {
            steps: vec![ap(a(), walk)],
            capped: false
        }
    );
    let route = crpg_ai::host_route(&mesh, twin.oracle.world(), a(), dest).expect("a route");
    twin.mirror_move(a(), route);
    let r2 = drive(&mut twin, &sides, &[a()], MVP_AI_BUDGET).expect("drives");
    assert_eq!(
        r2,
        AiReport {
            steps: vec![ap(a(), Decision::Act(hit))],
            capped: false
        }
    );
    twin.mirror_checked_act(a(), &hit);
    assert_eq!(twin.host.authority_hash(), rig.host.authority_hash());

    // Control: with pool max 1 the reserve leaves no step, so EndTurn.
    let mut once = Rig::new(&ai3_1_value());
    once.host.bind_nav_mesh(mesh.clone()).expect("binds");
    once.mirror_bind_nav(&mesh);
    let end = end_turn(a());
    assert_eq!(
        drive(&mut once, &sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Act(end))], false)
    );
    once.mirror_checked_act(a(), &end);
    once.assert_authority();
    assert_eq!(once.mover_position(a()), m(1, 1));

    // Control: with no mesh bound the no-move fallback ends the turn.
    let mut bare = Rig::new(&ai3_value());
    let end = end_turn(a());
    assert_eq!(
        drive(&mut bare, &sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Act(end))], false)
    );
    bare.mirror_checked_act(a(), &end);
    bare.assert_authority();
    assert_eq!(bare.mover_position(a()), m(1, 1));
}

// ---------------------------------------------------------------------------
// H3: the turn cap counts across calls (D-5, D-11).
// ---------------------------------------------------------------------------

#[test]
fn h3_tb_turn_cap_counts_across_calls() {
    let sides = sides_abc();
    let mesh = m10();
    let dest = rp(98368, 0, 65536);
    let budget = AiBudget {
        decisions_per_turn: 1,
        actors_per_drive: 8,
    };
    let mut rig = Rig::new(&ai3_value());
    rig.host.bind_nav_mesh(mesh.clone()).expect("binds");
    rig.mirror_bind_nav(&mesh);

    // Call 1: the walk, uncapped.
    let walk = Decision::Move {
        target: b(),
        destination: dest,
    };
    assert_eq!(
        drive(&mut rig, &sides, &[a(), b()], budget),
        report(vec![ap(a(), walk)], false)
    );
    let route = crpg_ai::host_route(&mesh, rig.oracle.world(), a(), dest).expect("a route");
    rig.mirror_move(a(), route);
    rig.assert_authority();

    // Call 2: the count (1) reached the budget, so the forced EndTurn.
    let end_a = end_turn(a());
    assert_eq!(
        drive(&mut rig, &sides, &[a(), b()], budget),
        report(vec![ap(a(), Decision::Act(end_a))], true)
    );
    rig.mirror_checked_act(a(), &end_a);
    rig.assert_authority();
    assert_eq!(rig.host.awaiting_orders(), vec![b()]);

    // Call 3: B holds a new turn with a fresh count, and strikes A, who now
    // stands 98240 raw away.
    let hit = use_ability(b(), strike(), a());
    assert_eq!(
        drive(&mut rig, &sides, &[a(), b()], budget),
        report(vec![ap(b(), Decision::Act(hit))], false)
    );
    rig.mirror_checked_act(b(), &hit);
    rig.assert_authority();

    // Control: budget {2, 8} gives both H2 steps uncapped.
    let wide = AiBudget {
        decisions_per_turn: 2,
        actors_per_drive: 8,
    };
    let mut two = Rig::new(&ai3_value());
    two.host.bind_nav_mesh(mesh.clone()).expect("binds");
    two.mirror_bind_nav(&mesh);
    assert_eq!(
        drive(&mut two, &sides, &[a(), b()], wide),
        report(vec![ap(a(), walk)], false)
    );
    let route = crpg_ai::host_route(&mesh, two.oracle.world(), a(), dest).expect("a route");
    two.mirror_move(a(), route);
    let hit_a = use_ability(a(), strike(), b());
    assert_eq!(
        drive(&mut two, &sides, &[a(), b()], wide),
        report(vec![ap(a(), Decision::Act(hit_a))], false)
    );
    two.mirror_checked_act(a(), &hit_a);
    two.assert_authority();

    // Control: budget {0, 8} caps before the first decision; A has not moved.
    let zero = AiBudget {
        decisions_per_turn: 0,
        actors_per_drive: 8,
    };
    let mut fresh = Rig::new(&ai3_value());
    fresh.host.bind_nav_mesh(mesh.clone()).expect("binds");
    fresh.mirror_bind_nav(&mesh);
    let end = end_turn(a());
    assert_eq!(
        drive(&mut fresh, &sides, &[a(), b()], zero),
        report(vec![ap(a(), Decision::Act(end))], true)
    );
    fresh.mirror_checked_act(a(), &end);
    fresh.assert_authority();
    assert_eq!(fresh.mover_position(a()), m(1, 1));
}

// ---------------------------------------------------------------------------
// H4: the MVP turn cap is sixteen (D-5).
// ---------------------------------------------------------------------------

#[test]
fn h4_mvp_turn_cap_is_sixteen() {
    let sides = sides_abc();
    let mut rig = Rig::new(&fs3_value());
    let hit = use_ability(a(), strike(), b());
    for _ in 0..16 {
        assert_eq!(
            drive(&mut rig, &sides, &[a()], MVP_AI_BUDGET),
            report(vec![ap(a(), Decision::Act(hit))], false)
        );
        rig.mirror_checked_act(a(), &hit);
        rig.assert_authority();
        assert_eq!(rig.host.awaiting_orders(), vec![a()]);
    }
    let end = end_turn(a());
    assert_eq!(
        drive(&mut rig, &sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Act(end))], true)
    );
    rig.mirror_checked_act(a(), &end);
    rig.assert_authority();
    assert_eq!(rig.host.awaiting_orders(), vec![b()]);
    let b_state = rig.oracle.world().combatants().get(b()).expect("B exists");
    assert_eq!(b_state.health(), 1000 - 16 * 3);
    let a_state = rig.oracle.world().combatants().get(a()).expect("A exists");
    assert_eq!(a_state.action_pool().current(), 4);
}

// ---------------------------------------------------------------------------
// H5: real-time walk, strike, hold and re-plan (D-6, D-7, D-10, D-12).
// ---------------------------------------------------------------------------

#[test]
fn h5_rt_walk_strike_hold_and_replan() {
    let sides = sides_abc();
    let mesh = m10();
    let dest = rp(98368, 0, 65536);
    let mut rig = Rig::new(&ai3rt_value());
    rig.host.bind_nav_mesh(mesh.clone()).expect("binds");
    rig.mirror_bind_nav(&mesh);

    // Call 1: the same approach as H2, stored as a route by `move_to`.
    let walk = Decision::Move {
        target: b(),
        destination: dest,
    };
    assert_eq!(
        drive(&mut rig, &sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), walk)], false)
    );
    let route = crpg_ai::host_route(&mesh, rig.oracle.world(), a(), dest).expect("a route");
    assert_eq!(route.points, vec![m(1, 1), dest]);
    rig.mirror_set_route(a(), route);
    rig.assert_authority();

    // One tick on each side: A arrives and its route ends.
    assert_eq!(rig.host.tick(), Ok(()));
    rig.mirror_tick();
    rig.assert_authority();
    assert_eq!(rig.mover_position(a()), dest);

    // Call 2: in range, so the strike, stored as a standing order with no
    // capture entry.
    let before = state_of(&mut rig.host);
    let hit = use_ability(a(), strike(), b());
    assert_eq!(
        drive(&mut rig, &sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Act(hit))], false)
    );
    rig.mirror_checked_act(a(), &hit);
    rig.assert_authority();
    assert_eq!(rig.host.last_capture(), before.last_capture);
    assert!(!rig.host.awaiting_orders().contains(&a()));

    // Call 3: the order stands and the target is in range, so Hold, changing
    // nothing.
    let before = state_of(&mut rig.host);
    assert_eq!(
        drive(&mut rig, &sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Hold)], false)
    );
    rig.assert_state(&before);
    rig.assert_authority();

    // B walks away; A still holds an order, yet the next drive re-plans.
    rig.host.place(b(), m(9, 1)).expect("places");
    rig.mirror_place(b(), m(9, 1));
    rig.assert_authority();
    assert!(!rig.host.awaiting_orders().contains(&a()));
    let far = rp(491584, 0, 65536);
    let chase = Decision::Move {
        target: b(),
        destination: far,
    };
    assert_eq!(
        drive(&mut rig, &sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), chase)], false)
    );
    let route = crpg_ai::host_route(&mesh, rig.oracle.world(), a(), far).expect("a route");
    rig.mirror_set_route(a(), route);
    rig.assert_authority();
}

// ---------------------------------------------------------------------------
// H6: real-time halt before the strike (D-7).
// ---------------------------------------------------------------------------

#[test]
fn h6_rt_halt_before_strike() {
    let sides = sides_abc();
    let mesh = m10();
    let mut rig = Rig::new(&ai3rt_value());
    rig.host.bind_nav_mesh(mesh.clone()).expect("binds");
    rig.mirror_bind_nav(&mesh);
    rig.host.place(b(), m(2, 1)).expect("places");
    rig.mirror_place(b(), m(2, 1));
    rig.host.move_to(a(), m(1, 4)).expect("routes");
    rig.mirror_set_route(
        a(),
        MoveRoute {
            revision: r(&mesh),
            tag: RouteTag(0),
            points: vec![m(1, 1), m(1, 4)],
        },
    );
    rig.assert_authority();

    // Call 1: A walks while B stands in range, so Halt clears the route.
    assert_eq!(
        drive(&mut rig, &sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Halt)], false)
    );
    assert_eq!(rig.mirror_clear_route(a()), Some(RouteTag(0)));
    rig.assert_authority();

    // Call 2: with no route and B one metre away, the strike lands.
    let hit = use_ability(a(), strike(), b());
    assert_eq!(
        drive(&mut rig, &sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Act(hit))], false)
    );
    rig.mirror_checked_act(a(), &hit);
    rig.assert_authority();
}

// ---------------------------------------------------------------------------
// H7: real-time candidates and rotation (D-3, D-6).
// ---------------------------------------------------------------------------

#[test]
fn h7_rt_candidates_and_rotation() {
    let sides = sides_abc();

    // No mesh and every hostile out of range: each answer is Hold. The input
    // order never matters and e7 is never decided.
    let mut rig = Rig::new(&ai3rt_value());
    assert_eq!(
        drive(&mut rig, &sides, &[c(), e7(), a(), b()], MVP_AI_BUDGET),
        report(
            vec![
                ap(a(), Decision::Hold),
                ap(b(), Decision::Hold),
                ap(c(), Decision::Hold),
            ],
            false
        )
    );
    rig.assert_authority();

    // Budget {16, 1} with one tick between calls rotates through A, B, C.
    let one = AiBudget {
        decisions_per_turn: 16,
        actors_per_drive: 1,
    };
    let mut rot = Rig::new(&ai3rt_value());
    for (tick, actor) in [a(), b(), c(), a()].into_iter().enumerate() {
        assert_eq!(rot.host.server_tick(), tick as u64);
        assert_eq!(
            drive(&mut rot, &sides, &[c(), e7(), a(), b()], one),
            report(vec![ap(actor, Decision::Hold)], true)
        );
        rot.assert_authority();
        assert_eq!(rot.host.tick(), Ok(()));
        rot.mirror_tick();
        rot.assert_authority();
    }

    // Budget {16, 2} at tick 2 starts at cand[2] and wraps.
    let two = AiBudget {
        decisions_per_turn: 16,
        actors_per_drive: 2,
    };
    let mut wrap = Rig::new(&ai3rt_value());
    assert_eq!(wrap.host.tick(), Ok(()));
    wrap.mirror_tick();
    assert_eq!(wrap.host.tick(), Ok(()));
    wrap.mirror_tick();
    assert_eq!(wrap.host.server_tick(), 2);
    assert_eq!(
        drive(&mut wrap, &sides, &[c(), e7(), a(), b()], two),
        report(vec![ap(c(), Decision::Hold), ap(a(), Decision::Hold)], true)
    );
    wrap.assert_authority();

    // The dead C is skipped, never Undecided.
    let mut dagger = Rig::new(&ai3rt_dagger_value());
    let out = drive(&mut dagger, &sides, &[a(), b(), c()], MVP_AI_BUDGET).expect("drives");
    assert_eq!(
        out,
        AiReport {
            steps: vec![ap(a(), Decision::Hold), ap(b(), Decision::Hold)],
            capped: false,
        }
    );
    for step in &out.steps {
        match step {
            AiStep::Applied { .. } => {}
            AiStep::Refused { .. } => panic!("no refusal expected"),
            AiStep::Undecided { .. } => panic!("the dead are skipped, not Undecided"),
        }
    }
    dagger.assert_authority();

    // Control: the same actors on TB3 decide only the head.
    let mut tb = Rig::new(&tb3_value());
    let out = drive(&mut tb, &sides, &[c(), e7(), a(), b()], MVP_AI_BUDGET).expect("drives");
    assert_eq!(out.steps.len(), 1);
    match &out.steps[0] {
        AiStep::Applied { actor, .. } => assert_eq!(*actor, a()),
        other => panic!("head only, got {other:?}"),
    }
    assert!(!out.capped);
}

// ---------------------------------------------------------------------------
// H8: a refusal is a step, never a retry (D-5, D-7, D-8; Errors).
// ---------------------------------------------------------------------------

#[test]
fn h8_refusal_is_a_step() {
    let sides = sides_abc();
    let mut rig = Rig::new(&tb3_value());
    let peer = rig.bind_v();
    fill_journal(&mut rig, peer);
    rig.assert_authority();
    // 4096 EndTurns leave B holding the turn.
    assert_eq!(
        rig.oracle.world().combat().expect("an encounter").active,
        Some(b())
    );

    // The strike is refused CaptureFull; a second call repeats the refusal.
    let hit = use_ability(b(), strike(), a());
    assert_eq!(
        drive(&mut rig, &sides, &[b()], MVP_AI_BUDGET),
        report(
            vec![rf(b(), Decision::Act(hit), HostError::CaptureFull)],
            false
        )
    );
    rig.assert_authority();
    assert_eq!(
        drive(&mut rig, &sides, &[b()], MVP_AI_BUDGET),
        report(
            vec![rf(b(), Decision::Act(hit), HostError::CaptureFull)],
            false
        )
    );
    rig.assert_authority();

    // Control: after acknowledging, the same drive applies.
    rig.host.acknowledge_captures(4096).expect("acks");
    let hit = use_ability(b(), strike(), a());
    assert_eq!(
        drive(&mut rig, &sides, &[b()], MVP_AI_BUDGET),
        report(vec![ap(b(), Decision::Act(hit))], false)
    );
    rig.mirror_checked_act(b(), &hit);
    rig.assert_authority();

    // Cap case: budget {0, 8} forces EndTurn, refused, yet capped is true.
    let mut capped = Rig::new(&tb3_value());
    let peer = capped.bind_v();
    fill_journal(&mut capped, peer);
    let zero = AiBudget {
        decisions_per_turn: 0,
        actors_per_drive: 8,
    };
    let end_b = end_turn(b());
    assert_eq!(
        drive(&mut capped, &sides, &[b()], zero),
        report(
            vec![rf(b(), Decision::Act(end_b), HostError::CaptureFull)],
            true
        )
    );
    capped.assert_authority();
    assert_eq!(capped.host.awaiting_orders(), vec![b()]);
    assert_eq!(
        drive(&mut capped, &sides, &[b()], zero),
        report(
            vec![rf(b(), Decision::Act(end_b), HostError::CaptureFull)],
            true
        )
    );
    capped.assert_authority();

    // Control: after acknowledging, the forced EndTurn applies and ends B.
    capped.host.acknowledge_captures(4096).expect("acks");
    let end_b = end_turn(b());
    assert_eq!(
        drive(&mut capped, &sides, &[b()], zero),
        report(vec![ap(b(), Decision::Act(end_b))], true)
    );
    capped.mirror_checked_act(b(), &end_b);
    capped.assert_authority();
    assert_eq!(capped.host.awaiting_orders(), vec![c()]);
}

// ---------------------------------------------------------------------------
// H9: pause does not stop the driver (D-9).
// ---------------------------------------------------------------------------

#[test]
fn h9_pause_does_not_stop_the_driver() {
    let sides = sides_abc();
    let expected = use_ability(a(), strike(), b());
    let mut paused = Rig::new(&tb3_value());
    assert_eq!(paused.host.pause(1), Ok(()));
    assert_eq!(
        drive(&mut paused, &sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Act(expected))], false)
    );
    paused.mirror_checked_act(a(), &expected);
    paused.assert_authority();
    assert_eq!(paused.host.paused(), Some(PauseReason::Operator));

    // A running host given the same call returns the same report and hash.
    let mut running = Rig::new(&tb3_value());
    assert_eq!(
        drive(&mut running, &sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Act(expected))], false)
    );
    running.mirror_checked_act(a(), &expected);
    assert_eq!(paused.host.authority_hash(), running.host.authority_hash());
}

// ---------------------------------------------------------------------------
// H10: closed and no fight (D-1, D-2; Errors).
// ---------------------------------------------------------------------------

#[test]
fn h10_closed_and_no_fight() {
    let sides = sides_abc();

    // No encounter: empty and uncapped.
    let mut fresh = Host::new(7, config(), 0).expect("new");
    assert_eq!(
        fresh.drive_ai(&sides, &[a()], MVP_AI_BUDGET),
        report(vec![], false)
    );

    // Control: before shutdown the head strike applies.
    let mut rig = Rig::new(&tb3_value());
    let hit = use_ability(a(), strike(), b());
    assert_eq!(
        drive(&mut rig, &sides, &[a()], MVP_AI_BUDGET),
        report(vec![ap(a(), Decision::Act(hit))], false)
    );
    rig.mirror_checked_act(a(), &hit);

    // After shutdown every drive is Closed, with the exact Display text.
    let before = state_of(&mut rig.host);
    rig.host.shutdown();
    assert_eq!(
        drive(&mut rig, &sides, &[a()], MVP_AI_BUDGET),
        Err(HostError::Closed)
    );
    assert_eq!(
        drive(&mut rig, &sides, &[], MVP_AI_BUDGET),
        Err(HostError::Closed)
    );
    assert_eq!(HostError::Closed.to_string(), "Closed at host/admission");
    assert_eq!(rig.host.authority_hash(), before.hash);
    assert_eq!(rig.host.last_capture(), before.last_capture);
    assert_eq!(rig.host.executions(), before.executions);
    rig.assert_authority();
}

// ---------------------------------------------------------------------------
// H11: a restore restarts the turn count (D-11; session 77 erratum).
// ---------------------------------------------------------------------------

#[test]
fn h11_restore_restarts_the_turn_count() {
    let sides = sides_abc();
    let budget = AiBudget {
        decisions_per_turn: 2,
        actors_per_drive: 8,
    };
    let mut rig = Rig::new(&fs3_value());
    let hit = use_ability(a(), strike(), b());
    for _ in 0..2 {
        assert_eq!(
            drive(&mut rig, &sides, &[a()], budget),
            report(vec![ap(a(), Decision::Act(hit))], false)
        );
        rig.mirror_checked_act(a(), &hit);
        rig.assert_authority();
        assert_eq!(rig.host.awaiting_orders(), vec![a()]);
    }

    // Restore mid-turn: the count restarts, so two more strikes fit before
    // the forced EndTurn on the fifth call.
    let saved = rig.host.save_checkpoint().expect("saves");
    let mut restored = load_checkpoint(&saved, 2, 0).expect("the save loads");
    assert_eq!(restored.nav_revision(), None);
    let mut mirror = rig.oracle;
    let check = |host: &mut Host, expected: Result<AiReport, HostError>| {
        assert_eq!(host.drive_ai(&sides, &[a()], budget), expected);
    };
    let hit2 = hit;
    check(
        &mut restored,
        report(vec![ap(a(), Decision::Act(hit2))], false),
    );
    mirror.perform_action(&hit2).expect("the mirror acts");
    mirror.acknowledge(mirror.last_sequence()).expect("acks");
    assert_eq!(restored.authority_hash(), history_hash(&mirror));
    assert_eq!(restored.awaiting_orders(), vec![a()]);
    check(
        &mut restored,
        report(vec![ap(a(), Decision::Act(hit2))], false),
    );
    mirror.perform_action(&hit2).expect("the mirror acts");
    mirror.acknowledge(mirror.last_sequence()).expect("acks");
    assert_eq!(restored.authority_hash(), history_hash(&mirror));
    let end = end_turn(a());
    check(
        &mut restored,
        report(vec![ap(a(), Decision::Act(end))], true),
    );
    mirror.perform_action(&end).expect("the mirror acts");
    mirror.acknowledge(mirror.last_sequence()).expect("acks");
    assert_eq!(restored.authority_hash(), history_hash(&mirror));
    assert_eq!(restored.awaiting_orders(), vec![b()]);
    let b_state = mirror.world().combatants().get(b()).expect("B exists");
    assert_eq!(b_state.health(), 1000 - 4 * 3);
    let a_state = mirror.world().combatants().get(a()).expect("A exists");
    assert_eq!(a_state.action_pool().current(), 16);

    // Control: with no restore the third call is the capped EndTurn.
    let mut plain = Rig::new(&fs3_value());
    plain.mirror_checked_act(a(), &hit);
    assert_eq!(
        plain.host.drive_ai(&sides, &[a()], budget),
        report(vec![ap(a(), Decision::Act(hit))], false)
    );
    plain.mirror_checked_act(a(), &hit);
    assert_eq!(
        plain.host.drive_ai(&sides, &[a()], budget),
        report(vec![ap(a(), Decision::Act(hit))], false)
    );
    let end = end_turn(a());
    assert_eq!(
        plain.host.drive_ai(&sides, &[a()], budget),
        report(vec![ap(a(), Decision::Act(end))], true)
    );
    plain.mirror_checked_act(a(), &end);
    plain.assert_authority();
    assert_eq!(plain.host.awaiting_orders(), vec![b()]);
}

// ---------------------------------------------------------------------------
// Errors that are steps: the decide error arm is unreachable here (D-8).
// ---------------------------------------------------------------------------

#[test]
fn undecided_is_the_decide_error_shape() {
    // `TooManyOptions` is the only `decide` error that can reach a live
    // controlled combatant, and it needs more than 4096 options, which three
    // combatants cannot supply. The skip rule (absent and dead actors are
    // never decided) is proved by H7; here the error value shape is pinned.
    let error = LegalActionsError::TooManyOptions { limit: 4096 };
    let step = AiStep::Undecided {
        actor: a(),
        error: error.clone(),
    };
    assert_eq!(step, AiStep::Undecided { actor: a(), error });
    let _ = CombatError::NoEncounter;
}

// ---------------------------------------------------------------------------
// D-3: a duplicated controlled actor is decided once (a02).
// ---------------------------------------------------------------------------

#[test]
fn d3_duplicate_actors_decide_once() {
    let sides = sides_abc();
    let plain = [a(), b()];
    let with_duplicates = [b(), a(), b(), a(), b()];

    // `ctl` is sorted with duplicates removed: each actor appears once, in
    // ascending order, even though the input repeats B and A.
    let mut dup = Rig::new(&ai3rt_value());
    assert_eq!(
        drive(&mut dup, &sides, &with_duplicates, MVP_AI_BUDGET),
        report(
            vec![ap(a(), Decision::Hold), ap(b(), Decision::Hold)],
            false
        )
    );
    dup.assert_authority();

    // Control: each actor listed once gives the same report and the same
    // authority.
    let mut once = Rig::new(&ai3rt_value());
    assert_eq!(
        drive(&mut once, &sides, &plain, MVP_AI_BUDGET),
        report(
            vec![ap(a(), Decision::Hold), ap(b(), Decision::Hold)],
            false
        )
    );
    once.assert_authority();
    assert_eq!(dup.host.authority_hash(), once.host.authority_hash());
}

// ---------------------------------------------------------------------------
// D-6: exactly `actors_per_drive` candidates is not capped (a19).
// ---------------------------------------------------------------------------

#[test]
fn d6_exact_cap_is_not_capped() {
    let sides = sides_abc();
    let actors = [c(), e7(), a(), b()];
    let at_tick_one = |rig: &mut Rig| {
        assert_eq!(rig.host.tick(), Ok(()));
        rig.mirror_tick();
        assert_eq!(rig.host.server_tick(), 1);
    };

    // n == k == 3 at tick 1: every candidate is decided, ascending. A rotated
    // order would start at `cand[1]` (B), so ascending proves no rotation.
    let mut exact = Rig::new(&ai3rt_value());
    at_tick_one(&mut exact);
    assert_eq!(
        drive(
            &mut exact,
            &sides,
            &actors,
            AiBudget {
                decisions_per_turn: 16,
                actors_per_drive: 3
            }
        ),
        report(
            vec![
                ap(a(), Decision::Hold),
                ap(b(), Decision::Hold),
                ap(c(), Decision::Hold)
            ],
            false
        )
    );
    exact.assert_authority();

    // Control: n == k + 1 is capped and rotated at the same tick.
    let mut over = Rig::new(&ai3rt_value());
    at_tick_one(&mut over);
    assert_eq!(
        drive(
            &mut over,
            &sides,
            &actors,
            AiBudget {
                decisions_per_turn: 16,
                actors_per_drive: 2
            }
        ),
        report(vec![ap(b(), Decision::Hold), ap(c(), Decision::Hold)], true)
    );
    over.assert_authority();

    // A cap above n never caps and never rotates either.
    let mut under = Rig::new(&ai3rt_value());
    at_tick_one(&mut under);
    assert_eq!(
        drive(
            &mut under,
            &sides,
            &actors,
            AiBudget {
                decisions_per_turn: 16,
                actors_per_drive: 4
            }
        ),
        report(
            vec![
                ap(a(), Decision::Hold),
                ap(b(), Decision::Hold),
                ap(c(), Decision::Hold)
            ],
            false
        )
    );
    under.assert_authority();
}

// ---------------------------------------------------------------------------
// D-5: a new round restarts the count for the same head (a10).
// ---------------------------------------------------------------------------

#[test]
fn d5_new_round_restarts_the_count() {
    let sides = sides_abc();
    let one = AiBudget {
        decisions_per_turn: 1,
        actors_per_drive: 8,
    };
    // `FS3`: the strike keeps the turn, so A's head survives its decision.
    let mut rig = Rig::new(&fs3_value());

    // Round r: A's one decision is the strike, which keeps its turn.
    let hit = use_ability(a(), strike(), b());
    assert_eq!(
        drive(&mut rig, &sides, &[a()], one),
        report(vec![ap(a(), Decision::Act(hit))], false)
    );
    rig.mirror_checked_act(a(), &hit);
    rig.assert_authority();
    assert_eq!(rig.host.awaiting_orders(), vec![a()]);
    let round = rig.oracle.world().combat().expect("an encounter").round;

    // Control: the same head in the same round is capped, so the count is not
    // per call.
    let end_a = end_turn(a());
    assert_eq!(
        drive(&mut rig, &sides, &[a()], one),
        report(vec![ap(a(), Decision::Act(end_a))], true)
    );
    rig.mirror_checked_act(a(), &end_a);
    rig.assert_authority();
    assert_eq!(rig.host.awaiting_orders(), vec![b()]);

    // B and C end their turns without the driver, so the stored counter still
    // names A and the round rolls over under it.
    let end_b = end_turn(b());
    assert_eq!(rig.host.host_action(&end_b), Ok(None));
    rig.mirror_checked_act(b(), &end_b);
    let end_c = end_turn(c());
    assert_eq!(rig.host.host_action(&end_c), Ok(None));
    rig.mirror_checked_act(c(), &end_c);
    rig.assert_authority();
    assert_eq!(
        rig.oracle.world().combat().expect("an encounter").round,
        round + 1
    );
    // The same head holds the new round, and the stored counter named it at
    // the cap.
    assert_eq!(rig.host.awaiting_orders(), vec![a()]);

    // The new round restarts the count: this call decides, it is not the
    // capped `EndTurn`.
    let hit2 = use_ability(a(), strike(), b());
    assert_eq!(
        drive(&mut rig, &sides, &[a()], one),
        report(vec![ap(a(), Decision::Act(hit2))], false)
    );
    rig.mirror_checked_act(a(), &hit2);
    rig.assert_authority();
}

// ---------------------------------------------------------------------------
// D-6: the real-time fight gate needs two live combatants (a05, a09).
// ---------------------------------------------------------------------------

#[test]
fn d6_two_live_combatants_gate() {
    let sides = sides_abc();

    // One live combatant (A), the encounter still present: the gate holds the
    // call back, and nothing changes.
    let mut solo = Rig::new(&ai3rt_solo_value());
    {
        let world = solo.oracle.world();
        assert!(world.combat().is_some(), "the encounter is still on");
        let alive = [a(), b(), c()]
            .iter()
            .filter(|entity| !world.combatants().get(**entity).expect("present").dead())
            .count();
        assert_eq!(alive, 1, "exactly one live combatant");
    }
    let before = state_of(&mut solo.host);
    assert_eq!(
        drive(&mut solo, &sides, &[a(), b(), c()], MVP_AI_BUDGET),
        report(vec![], false)
    );
    solo.assert_state(&before);
    solo.assert_authority();

    // Control: with two live combatants the same actors decide (B is alive,
    // C is dead and skipped).
    let mut pair = Rig::new(&ai3rt_dagger_value());
    assert_eq!(
        drive(&mut pair, &sides, &[a(), b(), c()], MVP_AI_BUDGET),
        report(
            vec![ap(a(), Decision::Hold), ap(b(), Decision::Hold)],
            false
        )
    );
    pair.assert_authority();
}

// ---------------------------------------------------------------------------
// D-6: the fight gate at `RT_CLOCK_MAX` (a06, a07).
// ---------------------------------------------------------------------------

#[test]
fn d6_rt_clock_bound() {
    let sides = sides_abc();
    let actors = [a(), b(), c()];
    assert_eq!(RT_CLOCK_MAX, 2_147_483_647);

    // At the bound the fight is over for the driver: empty and uncapped.
    let mut at = Rig::new(&ai3rt_elapsed_value(RT_CLOCK_MAX));
    {
        let combat = at.oracle.world().combat().expect("an encounter");
        assert_eq!(combat.clock.expect("a clock").elapsed, RT_CLOCK_MAX);
        for entity in [a(), b(), c()] {
            let combatant = at.oracle.world().combatants().get(entity).expect("present");
            assert!(!combatant.dead(), "the gate is the clock, not the dead");
        }
    }
    let before = state_of(&mut at.host);
    assert_eq!(
        drive(&mut at, &sides, &actors, MVP_AI_BUDGET),
        report(vec![], false)
    );
    at.assert_state(&before);
    at.assert_authority();

    // One tick below the bound every live candidate decides.
    let mut below = Rig::new(&ai3rt_elapsed_value(RT_CLOCK_MAX - 1));
    assert_eq!(
        drive(&mut below, &sides, &actors, MVP_AI_BUDGET),
        report(
            vec![
                ap(a(), Decision::Hold),
                ap(b(), Decision::Hold),
                ap(c(), Decision::Hold)
            ],
            false
        )
    );
    below.assert_authority();
}

// ---------------------------------------------------------------------------
// D-8: a refusal in real time does not stop the call (a29).
// ---------------------------------------------------------------------------

#[test]
fn d8_rt_refusal_does_not_stop_the_call() {
    let sides = sides_abc();
    let mut rig = Rig::new(&ai3rt_swing_value());
    // B stands one metre from A, inside the strike's range, and is not
    // controlled; C stands out of range and has no mesh, so it holds.
    rig.host.place(b(), m(2, 1)).expect("places");
    rig.mirror_place(b(), m(2, 1));

    // Fill the capture journal with swings: each order resolves on the next
    // tick (`round_ticks` 1) and appends one record, none acknowledged. The
    // first order the journal has no room for stops the fill.
    let swing_a = use_ability(a(), strike(), b());
    let mut filled = 0_u32;
    loop {
        match rig.host.host_action(&swing_a) {
            Ok(None) => {}
            Err(HostError::CaptureFull) => break,
            other => panic!("unexpected order result: {other:?}"),
        }
        rig.oracle
            .perform_action(&swing_a)
            .expect("the mirror orders");
        rig.mirror_ack();
        assert_eq!(rig.host.tick(), Ok(()));
        rig.mirror_tick();
        filled += 1;
        assert!(filled <= 4096, "the journal never filled");
    }
    assert!(filled > 0, "the journal filled before one record");
    assert_eq!(rig.host.last_capture(), 4096, "the capture journal is full");
    rig.assert_authority();

    // The first candidate's strike is refused by the full journal; C's `Hold`
    // is still applied in the same call, after it.
    let before = state_of(&mut rig.host);
    assert_eq!(
        drive(&mut rig, &sides, &[a(), c()], MVP_AI_BUDGET),
        report(
            vec![
                rf(a(), Decision::Act(swing_a), HostError::CaptureFull),
                ap(c(), Decision::Hold)
            ],
            false
        )
    );
    rig.assert_state(&before);
    rig.assert_authority();

    // Control: with room the same call applies both, in order.
    rig.host
        .acknowledge_captures(rig.host.last_capture())
        .expect("acks");
    assert_eq!(
        drive(&mut rig, &sides, &[a(), c()], MVP_AI_BUDGET),
        report(
            vec![ap(a(), Decision::Act(swing_a)), ap(c(), Decision::Hold)],
            false
        )
    );
    rig.oracle
        .perform_action(&swing_a)
        .expect("the mirror orders");
    rig.mirror_ack();
    rig.assert_authority();
}
