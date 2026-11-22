//! T157d acceptance suite for the host's side rules (`tasks/T157.md`, tests
//! SD1 and SD2; SD3 is appended to `host_lane1.rs`, SD4 is every existing
//! suite): the host's "is the fight still on?" gates ask
//! `crpg_sim::fight_is_live` instead of counting living combatants, and
//! `advance` starts the turn-based fight when `crpg_sim::engagement_due` says
//! two opposed combatants met.
//!
//! Every case runs beside a mirror `HistoryWorld` loaded from the same JSON and
//! driven with the same public sim calls (the `Rig` of `host_lane1.rs`,
//! copied), so a host that engaged when the sim would not, or that skipped a
//! request the sim asks for, fails on `authority_hash()`.
//!
//! The fixture is `tests/fixtures/three_combatants.json` edited in memory, as
//! `host_lane1.rs` does: `MV3rt` (A/B/C movers at `m(1, 1)`, `m(3, 1)`,
//! `m(1, 5)`, 1 m a tick, `round_ticks` 4, real time). `ENGAGE_RT` adds A and B
//! on faction `F1`, C on `F2`, and one 2 m `proximity` rule.

use crpg_ai::{SideId, Sides};
use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_nav::{bake_box, NavMesh, NavRect};
use crpg_net::protocol_v3::PauseReason;
use crpg_server::host::{AdvanceReport, Host, HostConfig, ProtocolSelection, MVP_AI_BUDGET};
use crpg_sim::{
    history_hash, CombatAction, EncounterMode, HistoryWorld, MoveRoute, NavRevision, RouteTag,
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
/// `T157a`'s first faction.
fn f1() -> Ulid {
    Ulid::from_u128(131)
}
/// `T157a`'s second faction.
fn f2() -> Ulid {
    Ulid::from_u128(132)
}
/// The fixture's always-critical 4 000 000 000-damage ability.
fn kill() -> Ulid {
    Ulid::from_u128(604)
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

/// `M10`: the 10 m square that holds `MV3rt`'s three movers.
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

/// The movement edits of `MV3`/`MV3rt` (`host_lane1.rs`, unchanged).
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

/// `ENGAGE_RT` without its factions and rule: A and B alone are still opposed.
fn rt_value() -> Value {
    mv3rt_value()
}

/// `ENGAGE_RT` without the rule: A and B on `F1`, C on `F2`.
fn faction_rt_value() -> Value {
    let mut value = rt_value();
    value["world"]["combatants"][0][1]["faction"] = json!(f1().to_string());
    value["world"]["combatants"][1][1]["faction"] = json!(f1().to_string());
    value["world"]["combatants"][2][1]["faction"] = json!(f2().to_string());
    value
}

/// The contract's `ENGAGE_RT`: factions as `faction_rt_value`, plus one 2 m
/// `proximity` rule in the full-shape definition.
fn engage_rt_value() -> Value {
    let mut value = faction_rt_value();
    value["world"]["combat"]["definition"]["engage"] =
        json!([{"type": "proximity", "value": {"distance": 131072}}]);
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

/// The contract's `sides`: A and B on `SideId(0)`, C on `SideId(1)`.
fn sides() -> Sides {
    Sides::new(vec![(a(), SideId(0)), (b(), SideId(0)), (c(), SideId(1))])
        .expect("distinct entities")
}

// ---------------------------------------------------------------------------
// Rig: one host and its mirror authority (`host_lane1.rs`, copied).
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
        let binding = self.host.bind_nav_mesh(mesh).expect("binds");
        assert_eq!(binding.cleared, Vec::new());
        assert_eq!(binding.revision, r(&m10()));
        self.oracle
            .bind_nav(r(&m10()))
            .expect("the mirror binds the revision");
        self.mirror_ack();
    }

    /// The mirror's `set_route` for `actor`, computed by `crpg_nav` directly
    /// (the host's `plan_route` is the same two calls), tagged `tag`;
    /// acknowledged.
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

    /// One `advance`, its tick mirrored when it ticked.
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
}

// ---------------------------------------------------------------------------
// SD1.
// ---------------------------------------------------------------------------

/// SD1: `advance` asks for the turn-based fight on the first tick after two
/// opposed combatants come within the rule's distance, then the pending switch
/// applies at the next round boundary; a paused `advance` and a bare `tick`
/// never ask. The first case fails if `advance` does not engage, the controls
/// if it engages where the sim says it must not.
#[test]
fn sd1_the_host_engages_when_opposed_combatants_meet() {
    let mut rig = Rig::new(&engage_rt_value());
    rig.bind_nav();
    rig.host.move_to(a(), m(1, 3)).expect("a trusted walk");
    rig.mirror_route(a(), m(1, 3), 0);

    // Tick 1: A at (1, 2), 3 m from C. No rule holds, no request.
    rig.assert_ticked(50);
    let combat = rig.oracle.world().combat().expect("an encounter");
    assert_eq!(combat.mode, EncounterMode::RealTime);
    assert_eq!(combat.pending_mode, None);
    assert_eq!(rig.host.awaiting_orders(), vec![a(), b(), c()]);

    // Tick 2: A at (1, 3), exactly 2 m from C, so the host requested in this
    // same advance. The mirror holds no request yet, so the authorities differ
    // until the test asks for it too.
    let report = rig.advance(100);
    assert!(report.ticked, "{report:?}");
    assert_ne!(rig.host.authority_hash(), history_hash(&rig.oracle));
    rig.oracle
        .request_mode(EncounterMode::TurnBased)
        .expect("the mirror requests");
    rig.mirror_ack();
    rig.assert_authority();
    assert_eq!(
        rig.oracle
            .world()
            .combat()
            .expect("an encounter")
            .pending_mode,
        Some(EncounterMode::TurnBased)
    );

    // Tick 3: still real time; the switch waits for the boundary.
    rig.assert_ticked(150);
    assert_eq!(
        rig.oracle.world().combat().expect("an encounter").mode,
        EncounterMode::RealTime
    );

    // Tick 4: the boundary applies the switch; the turn head is A (initiative
    // 1, the lowest).
    rig.assert_ticked(200);
    assert_eq!(
        rig.oracle.world().combat().expect("an encounter").mode,
        EncounterMode::TurnBased
    );
    assert_eq!(rig.host.awaiting_orders(), vec![a()]);

    // Control (a): the same walk without `engage`; through 40 advances no
    // request is made and the fight stays real time, so `awaiting_orders` is
    // never the single turn head.
    let mut no_engage = Rig::new(&faction_rt_value());
    no_engage.bind_nav();
    no_engage
        .host
        .move_to(a(), m(1, 3))
        .expect("a trusted walk");
    no_engage.mirror_route(a(), m(1, 3), 0);
    for k in 1..=40u64 {
        no_engage.assert_ticked(50 * k);
        let combat = no_engage.oracle.world().combat().expect("an encounter");
        assert_eq!(combat.mode, EncounterMode::RealTime);
        assert_eq!(combat.pending_mode, None);
        assert_ne!(no_engage.host.awaiting_orders().len(), 1);
    }

    // Control (b): paused before the meeting advance, `advance` does not tick
    // and nothing is requested.
    let mut paused = Rig::new(&engage_rt_value());
    paused.bind_nav();
    paused.host.move_to(a(), m(1, 3)).expect("a trusted walk");
    paused.mirror_route(a(), m(1, 3), 0);
    paused.assert_ticked(50);
    assert_eq!(paused.host.pause(60), Ok(()));
    let report = paused.advance(100);
    assert!(!report.ticked, "{report:?}");
    assert_eq!(report.paused, Some(PauseReason::Operator));
    paused.assert_authority();
    let combat = paused.oracle.world().combat().expect("an encounter");
    assert_eq!(combat.mode, EncounterMode::RealTime);
    assert_eq!(combat.pending_mode, None);

    // Control (c): the meeting tick driven by `host.tick()` instead of
    // `advance`; no request.
    let mut bare = Rig::new(&engage_rt_value());
    bare.bind_nav();
    bare.host.move_to(a(), m(1, 3)).expect("a trusted walk");
    bare.mirror_route(a(), m(1, 3), 0);
    bare.assert_ticked(50);
    bare.host.tick().expect("the operator's tick");
    bare.mirror_tick();
    bare.assert_authority();
    let combat = bare.oracle.world().combat().expect("an encounter");
    assert_eq!(combat.mode, EncounterMode::RealTime);
    assert_eq!(combat.pending_mode, None);
}

// ---------------------------------------------------------------------------
// SD2.
// ---------------------------------------------------------------------------

/// SD2: the real-time `awaiting_orders` gate and `drive_ai`'s real-time plan
/// ask `fight_is_live`, not the living count. With A and B on one faction, C's
/// death ends the fight although two combatants live; with no faction A and B
/// are opposed, so both stay due.
#[test]
fn sd2_gates_follow_sides() {
    // Factions: A and B on F1, C on F2. A's standing order kills C.
    let mut rig = Rig::new(&faction_rt_value());
    rig.bind_nav();
    let order = CombatAction::UseAbility {
        actor: a(),
        ability: kill(),
        target: c(),
    };
    rig.host.host_action(&order).expect("an RT standing order");
    rig.mirror_perform(&order);
    rig.assert_authority();

    // Tick 1: A's order executes and C dies. Two combatants live, but one side
    // stands, so the fight is over.
    rig.assert_ticked(10);
    assert!(rig
        .oracle
        .world()
        .combatants()
        .get(c())
        .expect("a combatant")
        .dead());
    assert_eq!(rig.host.awaiting_orders(), Vec::<EntityId>::new());
    let report = rig
        .host
        .drive_ai(&sides(), &[a(), b()], MVP_AI_BUDGET)
        .expect("the host is open");
    assert!(report.steps.is_empty(), "{report:?}");
    assert!(!report.capped, "{report:?}");
    rig.assert_authority();

    // Control: no factions, so A and B alone are still opposed. The same kill
    // leaves both due and unordered.
    let mut control = Rig::new(&rt_value());
    control.bind_nav();
    control
        .host
        .host_action(&order)
        .expect("an RT standing order");
    control.mirror_perform(&order);
    control.assert_ticked(10);
    assert!(control
        .oracle
        .world()
        .combatants()
        .get(c())
        .expect("a combatant")
        .dead());
    assert_eq!(control.host.awaiting_orders(), vec![b()]);

    // A was re-keyed to elapsed + recovery (1 + 4 = 5); ticking on to it makes
    // both due, in ascending (key, EntityId) order: B (0) before A (5).
    for k in 2..=5u64 {
        control.assert_ticked(10 * k);
    }
    assert_eq!(control.host.awaiting_orders(), vec![b(), a()]);
}
