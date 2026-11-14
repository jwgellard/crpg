//! T141e acceptance suite (`tasks/T141.md`, tests E1 to E4; E5 is the unit test
//! in `src/host.rs`): v3 timing, public API only.
//!
//! The client is the D54 client: a `ReassemblyPoolV3` gate feeding a
//! `ReplicaWorldV3`, never the v1/v2 receiver. Oracles are independent of the
//! host: a test-owned mirror `HistoryWorld` loaded from the same JSON and
//! driven with the same sim calls (acknowledged after each), and hand-written
//! expected frames and ops. The fixtures are `tests/fixtures/three_combatants.json`
//! as saved-game JSON, edited in memory:
//!
//! - `TB3`: the file as is (A, B, C act in keys 1, 2, 3; A first).
//! - `TB3c`: `TB3` at history version 2 with a round clock (real-time capable
//!   while turn-based).
//! - `RT3`: `TB3c` in real-time mode, no active head, every timeline key 0.
//!
//! Every tick of `RT3` advances the round clock by one of four, so round
//! `r` starts at tick `4r` and every tick's page holds an `Encounter` op.

use crpg_core::{EntityId, Ulid};
use crpg_net::codec_v3;
use crpg_net::facts_v3::EncounterGrants;
use crpg_net::pool_v3::{
    DeltaOutcome, GateMode, ReassemblyPoolV3, ReceiverIdV3, ReplicaUpdateV3, TransferLimitsV3,
    TransferOutcome,
};
use crpg_net::protocol::{ReceiptStatus as StatusV1, LANE_COMBAT};
use crpg_net::protocol_transfer::{ResyncReason, TransferDown, TransferId, TransferUp};
use crpg_net::protocol_v3::{
    ClockFact, DeltaFrame, DeltaOp, EncounterFacts, EncounterMode as NetMode, IntentBody,
    IntentFrame, NetId, PauseReason, PauseRights, ReceiptStatus, RejectionCode, SessionEpoch,
};
use crpg_net::replica_v3::{PauseState, ReplicaWorldV3};
use crpg_net::transfer_v3;
use crpg_server::host::{
    AdmissionResult, ControlGrant, DeliveryEntry, DeliveryKind, DisclosureGrants, EntityDisclosure,
    EntityFactGrant, FactGrants, Host, HostConfig, IngestDisposition, PeerHandle,
    ProtocolSelection, TransferDisposition, TransferState, DEFAULT_HOST_GRANTS,
};
use crpg_sim::{history_hash, CombatAction, HistoryWorld};
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");

// ---------------------------------------------------------------------------
// Identities and fixtures.
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
fn tid(raw: u64) -> TransferId {
    TransferId::new(raw).expect("nonzero")
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

/// The authority from `value`, its four start envelopes acknowledged.
fn history_of(value: &Value) -> HistoryWorld {
    let mut history: HistoryWorld =
        serde_json::from_value(value.clone()).expect("fixture loads through validated serde");
    history
        .acknowledge(history.last_sequence())
        .expect("acknowledges its own range");
    history
}

fn host_of(value: &Value) -> Host {
    Host::from_history(
        history_of(value),
        HostConfig {
            protocol: ProtocolSelection::V2,
            incarnation: 1,
        },
        0,
    )
    .expect("wraps")
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

/// `FULL(A, B, C)`: every flag true for the three combatants.
fn full_grants() -> DisclosureGrants {
    DisclosureGrants {
        entities: [a(), b(), c()].into_iter().map(disclosure).collect(),
        abilities: vec![strike(), Ulid::from_u128(604), Ulid::from_u128(605)],
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
// Expected ops (hand-written, never read back from the host).
// ---------------------------------------------------------------------------

/// `Encounter` with the real-time mode and the clock `{ 4, elapsed }` disclosed.
fn rt_encounter(elapsed: u64) -> DeltaOp {
    DeltaOp::Encounter(EncounterFacts {
        mode: Some(NetMode::RealTime),
        pending_mode: Some(None),
        clock: Some(Some(ClockFact {
            round_ticks: 4,
            elapsed,
        })),
    })
}

fn clock_running() -> DeltaOp {
    DeltaOp::Clock { paused: None }
}

fn clock_paused(reason: PauseReason) -> DeltaOp {
    DeltaOp::Clock {
        paused: Some(Some(reason)),
    }
}

fn turn_none(round: u64) -> DeltaOp {
    DeltaOp::Turn {
        active: None,
        round,
    }
}

fn applied(epoch: [u8; 16], seq: u64, tick: u64) -> DeltaOp {
    DeltaOp::Receipt {
        epoch,
        lane: LANE_COMBAT,
        seq,
        processed_tick: tick,
        status: ReceiptStatus::Applied,
    }
}

fn rejected(epoch: [u8; 16], seq: u64, code: RejectionCode) -> DeltaOp {
    DeltaOp::Receipt {
        epoch,
        lane: LANE_COMBAT,
        seq,
        processed_tick: 0,
        status: ReceiptStatus::Rejected(code),
    }
}

// ---------------------------------------------------------------------------
// The mirror oracle and the v3 client (D54).
// ---------------------------------------------------------------------------

/// A test-owned `HistoryWorld` driven with the same sim calls as the host and
/// acknowledged after each.
struct Mirror(HistoryWorld);

impl Mirror {
    fn new(value: &Value) -> Self {
        Self(history_of(value))
    }

    fn ack(&mut self) {
        let last = self.0.last_sequence();
        self.0.acknowledge(last).expect("acknowledges");
    }

    fn act(&mut self, action: CombatAction) {
        self.0.perform_action(&action).expect("the sim accepts it");
        self.ack();
    }

    fn tick(&mut self) {
        self.0.tick().expect("the sim ticks");
        self.ack();
    }

    /// The host's authority equals the mirror's.
    fn check(&self, host: &Host) {
        assert_eq!(host.authority_hash(), history_hash(&self.0));
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Transfer(TransferOutcome),
    Delta(DeltaOutcome),
}

struct Drained {
    outcomes: Vec<Outcome>,
    deltas: Vec<DeltaFrame>,
}

impl Drained {
    /// The op lists of the decoded delta frames, in order.
    fn pages(&self) -> Vec<Vec<DeltaOp>> {
        self.deltas.iter().map(|frame| frame.ops.clone()).collect()
    }
}

struct Client {
    epoch: [u8; 16],
    pool: ReassemblyPoolV3,
    rid: ReceiverIdV3,
    replica: ReplicaWorldV3,
    last_snapshot: Option<TransferId>,
}

impl Client {
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

    /// Takes every pending entry, offers each to the gate in order,
    /// acknowledges the last ordinal, applies every released update and
    /// returns the decoded delta frames.
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
                ReplicaUpdateV3::Snapshot(publication) => {
                    self.last_snapshot = Some(publication.transfer);
                    self.replica.apply_snapshot(&publication).expect("applies");
                }
                ReplicaUpdateV3::Delta(frame) => {
                    self.replica.apply_delta(&frame).expect("applies");
                    deltas.push(frame);
                }
            }
        }
        Drained { outcomes, deltas }
    }

    /// `Published` for the last popped snapshot, through `ingest_transfer`.
    fn publish(&self, host: &mut Host, peer: PeerHandle, now: u64) -> TransferDisposition {
        let bytes = transfer_v3::encode_up(&TransferUp::Published {
            epoch: self.epoch,
            transfer: self.last_snapshot.expect("a snapshot was published"),
        })
        .expect("encodes");
        host.ingest_transfer(peer, &bytes, now).expect("open")
    }
}

/// `P = bind_v3_peer([A], FULL(A,B,C), FACTS, R1, now)` with its first
/// delivery drained and not yet published: the transfer is in flight.
fn bind_unpublished(host: &mut Host, now: u64) -> (PeerHandle, Client) {
    let peer = host
        .bind_v3_peer(control(&[a()]), full_grants(), facts(), r1(), now)
        .expect("binds");
    let mut client = Client::new(host.epoch(peer).expect("bound"));
    let first = client.drain(host, peer, now);
    assert_eq!(first.deltas.len(), 1, "C-6's host-facts delta");
    assert_eq!(
        host.transfer_state(peer),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: Some(1),
        }))
    );
    (peer, client)
}

/// [`bind_unpublished`], then `Published` right after the first drain (the
/// T141c/T141d rule): no throttle.
fn bind_p(host: &mut Host, now: u64) -> (PeerHandle, Client) {
    let (peer, client) = bind_unpublished(host, now);
    assert_eq!(
        client.publish(host, peer, now),
        TransferDisposition::Published
    );
    assert_eq!(
        host.transfer_state(peer),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: None,
        }))
    );
    (peer, client)
}

/// One tick of the mirror and of the host's pause-aware driver, then the
/// session's drained frames. `now` is a constant: no turn timer is armed.
fn step(
    host: &mut Host,
    mirror: &mut Mirror,
    client: &mut Client,
    peer: PeerHandle,
) -> Vec<DeltaFrame> {
    let report = host.advance(100).expect("advances");
    assert!(report.ticked);
    mirror.tick();
    client.drain(host, peer, 100).deltas
}

/// Asserts one tick produced no frame for the session.
fn assert_quiet(frames: &[DeltaFrame], tick: u64) {
    assert_eq!(frames, &[] as &[DeltaFrame], "tick {tick}");
}

/// Asserts `frames` is exactly one delta frame with these fields.
fn assert_one_frame(frames: &[DeltaFrame], tick: u64, event_seq: u64, ops: &[DeltaOp]) {
    assert_eq!(frames.len(), 1, "tick {tick}");
    assert_eq!(frames[0].lane, LANE_COMBAT, "tick {tick}");
    assert_eq!(frames[0].server_tick, tick, "tick {tick}");
    assert_eq!(frames[0].event_seq, event_seq, "tick {tick}");
    assert_eq!(frames[0].ops, ops, "tick {tick}");
}

fn clock_ops(frames: &[DeltaFrame]) -> usize {
    frames
        .iter()
        .flat_map(|frame| frame.ops.iter())
        .filter(|op| matches!(op, DeltaOp::Clock { .. }))
        .count()
}

/// The v3 intent bytes of one command.
fn intent_bytes(host: &Host, peer: PeerHandle, seq: u64, actor: u64, body: IntentBody) -> Vec<u8> {
    codec_v3::encode_intent(&IntentFrame {
        epoch: host.epoch(peer).expect("bound"),
        lane: LANE_COMBAT,
        seq,
        observed_tick: host.server_tick(),
        actor: n(actor),
        body,
    })
    .expect("encodes")
}

/// Stages `bytes` through `ingest` and pumps them.
fn pump_bytes(host: &mut Host, peer: PeerHandle, bytes: &[u8], now: u64) -> AdmissionResult {
    assert_eq!(host.ingest(peer, bytes, now), Ok(IngestDisposition::Staged));
    let summary = host.pump(now).expect("pump");
    assert_eq!(summary.results.len(), 1, "{summary:?}");
    assert_eq!(summary.dropped_replies, 0, "no failure budget is spent");
    summary.results[0].clone()
}

fn entries_of(host: &mut Host, peer: PeerHandle) -> Vec<DeliveryEntry> {
    host.take_delivery_entries(peer).expect("bound")
}

// ---------------------------------------------------------------------------
// E1, E2: the heartbeat.
// ---------------------------------------------------------------------------

/// E1: a turn-based host sends `[Clock]` exactly every 100 ticks of a running
/// session (99 none, 100 one, 101 none), the pause and unpause pages restart
/// the count, and a paused host sends no heartbeat.
#[test]
fn e1_heartbeat_tb() {
    let mut host = host_of(&tb3_value());
    let mut mirror = Mirror::new(&tb3_value());
    let (p, mut client) = bind_p(&mut host, 100);
    assert_eq!(client.replica.server_tick(), 0);

    // Ticks 1 to 99: nothing (the 99-tick side of the bound).
    for tick in 1..=99u64 {
        let frames = step(&mut host, &mut mirror, &mut client, p);
        assert_quiet(&frames, tick);
    }
    // Tick 100: one frame holding only `Clock` (event_seq 2: C-6's frame was 1).
    let frames = step(&mut host, &mut mirror, &mut client, p);
    assert_one_frame(&frames, 100, 2, &[clock_running()]);
    // Tick 101 to 199: nothing; tick 200: the next heartbeat.
    for tick in 101..=199u64 {
        let frames = step(&mut host, &mut mirror, &mut client, p);
        assert_quiet(&frames, tick);
    }
    let frames = step(&mut host, &mut mirror, &mut client, p);
    assert_one_frame(&frames, 200, 3, &[clock_running()]);
    for tick in 201..=250u64 {
        let frames = step(&mut host, &mut mirror, &mut client, p);
        assert_quiet(&frames, tick);
    }
    assert_eq!(host.server_tick(), 250);

    // The operator's pause at tick 250 sends its own `Clock`.
    host.pause(100).expect("pauses");
    let frames = client.drain(&mut host, p, 100).deltas;
    assert_one_frame(&frames, 250, 4, &[clock_paused(PauseReason::Operator)]);
    // While paused `advance` does not tick and sends nothing, however long.
    for _ in 0..150 {
        let report = host.advance(100).expect("advances");
        assert!(!report.ticked);
        assert_eq!(report.paused, Some(PauseReason::Operator));
    }
    assert_eq!(host.server_tick(), 250);
    assert_quiet(&client.drain(&mut host, p, 100).deltas, 250);
    // The unpause sends `Clock` and restarts the count from tick 250.
    host.unpause(100).expect("unpauses");
    let frames = client.drain(&mut host, p, 100).deltas;
    assert_one_frame(&frames, 250, 5, &[clock_running()]);
    for tick in 251..=349u64 {
        let frames = step(&mut host, &mut mirror, &mut client, p);
        assert_quiet(&frames, tick);
    }
    let frames = step(&mut host, &mut mirror, &mut client, p);
    assert_one_frame(&frames, 350, 6, &[clock_running()]);
    let frames = step(&mut host, &mut mirror, &mut client, p);
    assert_quiet(&frames, 351);
    assert_eq!(client.replica.pause(), Some(PauseState::Running));
    mirror.check(&host);
}

/// E1 (reading): the heartbeat belongs to a running host. A direct `tick()` on
/// a paused host (it ignores pause, B-4) sends none, even 100 ticks after the
/// pause's own `Clock`; the control is the same ticks on a running host.
#[test]
fn e1_paused_host_sends_no_heartbeat() {
    let mut paused = host_of(&tb3_value());
    let mut paused_mirror = Mirror::new(&tb3_value());
    let (p, mut paused_client) = bind_p(&mut paused, 100);
    paused.pause(100).expect("pauses");
    assert_one_frame(
        &paused_client.drain(&mut paused, p, 100).deltas,
        0,
        2,
        &[clock_paused(PauseReason::Operator)],
    );
    for _ in 0..150 {
        paused.tick().expect("ticks");
        paused_mirror.tick();
    }
    assert_eq!(paused.server_tick(), 150);
    assert_quiet(&paused_client.drain(&mut paused, p, 100).deltas, 150);
    paused_mirror.check(&paused);

    // Control: the same 150 ticks on a running host reach the heartbeat.
    let mut running = host_of(&tb3_value());
    let mut running_mirror = Mirror::new(&tb3_value());
    let (q, mut running_client) = bind_p(&mut running, 100);
    for _ in 0..150 {
        running.tick().expect("ticks");
        running_mirror.tick();
    }
    let frames = running_client.drain(&mut running, q, 100).deltas;
    assert_eq!(clock_ops(&frames), 1);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].server_tick, 100);
    running_mirror.check(&running);
}

/// E2: in a real-time fight the heartbeat rides in the tick's own page, last,
/// even though every tick of the fight has a page.
#[test]
fn e2_heartbeat_rt() {
    let mut host = host_of(&rt3_value());
    let mut mirror = Mirror::new(&rt3_value());
    let (p, mut client) = bind_p(&mut host, 100);

    for tick in 1..=99u64 {
        let frames = step(&mut host, &mut mirror, &mut client, p);
        assert_eq!(frames.len(), 1, "tick {tick}: one page");
        assert_eq!(clock_ops(&frames), 0, "tick {tick}: no heartbeat");
    }
    // Tick 100 (round 25): the page, then `Clock`, in one frame.
    let frames = step(&mut host, &mut mirror, &mut client, p);
    assert_one_frame(
        &frames,
        100,
        101,
        &[
            DeltaOp::RoundStarted { round: 25 },
            turn_none(25),
            rt_encounter(100),
            clock_running(),
        ],
    );
    // Tick 101: the page alone; the next heartbeat is at tick 200.
    let frames = step(&mut host, &mut mirror, &mut client, p);
    assert_one_frame(&frames, 101, 102, &[rt_encounter(101)]);
    for tick in 102..=199u64 {
        let frames = step(&mut host, &mut mirror, &mut client, p);
        assert_eq!(clock_ops(&frames), 0, "tick {tick}");
    }
    let frames = step(&mut host, &mut mirror, &mut client, p);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].server_tick, 200);
    assert_eq!(
        frames[0].ops,
        vec![
            DeltaOp::RoundStarted { round: 50 },
            turn_none(50),
            rt_encounter(200),
            clock_running(),
        ]
    );
    mirror.check(&host);
}

// ---------------------------------------------------------------------------
// E3, E4: the in-flight throttle.
// ---------------------------------------------------------------------------

/// E3: while the transfer is in flight a page is allowed every tenth tick, no
/// event ops or notices reach it, the changes fold into the next allowed page,
/// and `Published` ends the throttle at once.
#[test]
fn e3_throttle_folds() {
    let mut host = host_of(&rt3_value());
    let mut mirror = Mirror::new(&rt3_value());
    let (p, mut client) = bind_unpublished(&mut host, 100);

    for tick in 1..=30u64 {
        let frames = step(&mut host, &mut mirror, &mut client, p);
        match tick {
            10 => assert_one_frame(&frames, 10, 2, &[turn_none(2), rt_encounter(10)]),
            20 => assert_one_frame(&frames, 20, 3, &[turn_none(5), rt_encounter(20)]),
            30 => assert_one_frame(&frames, 30, 4, &[turn_none(7), rt_encounter(30)]),
            _ => assert_quiet(&frames, tick),
        }
    }
    assert_eq!(
        client.replica.encounter().clock,
        Some(Some(ClockFact {
            round_ticks: 4,
            elapsed: 30
        }))
    );
    // `Published` at tick 30 lifts the throttle: tick 31 is a page.
    assert_eq!(
        client.publish(&mut host, p, 100),
        TransferDisposition::Published
    );
    let frames = step(&mut host, &mut mirror, &mut client, p);
    assert_one_frame(&frames, 31, 5, &[rt_encounter(31)]);
    // Round 8 starts at tick 32, and its notice now arrives.
    let frames = step(&mut host, &mut mirror, &mut client, p);
    assert_one_frame(
        &frames,
        32,
        6,
        &[
            DeltaOp::RoundStarted { round: 8 },
            turn_none(8),
            rt_encounter(32),
        ],
    );
    assert_eq!(
        client.replica.encounter().clock,
        Some(Some(ClockFact {
            round_ticks: 4,
            elapsed: 32
        }))
    );
    assert_eq!(client.replica.turn().map(|turn| turn.round), Some(8));
    mirror.check(&host);
}

/// E3 control: the same ticks with `Published` first are a page every tick,
/// each with its `RoundStarted`, so the E3 drops are the throttle's doing.
#[test]
fn e3_control_published_session_gets_every_page() {
    let mut host = host_of(&rt3_value());
    let mut mirror = Mirror::new(&rt3_value());
    let (p, mut client) = bind_p(&mut host, 100);
    for tick in 1..=12u64 {
        let frames = step(&mut host, &mut mirror, &mut client, p);
        assert_eq!(frames.len(), 1, "tick {tick}");
        let started = frames[0]
            .ops
            .iter()
            .any(|op| matches!(op, DeltaOp::RoundStarted { .. }));
        assert_eq!(started, tick % 4 == 0, "tick {tick}");
    }
    mirror.check(&host);
}

/// E4: a session in flight gets exactly 128 delta frames (C-6's frame and the
/// pages at ticks 10 to 1,270), none after, and a `Resync` restarts the
/// throttle.
#[test]
fn e4_throttle_budget() {
    let mut host = host_of(&rt3_value());
    let mut mirror = Mirror::new(&rt3_value());
    let (p, mut client) = bind_unpublished(&mut host, 100);
    let mut delivered: Vec<u64> = vec![0]; // C-6's frame, at tick 0.
    for tick in 1..=1290u64 {
        let frames = step(&mut host, &mut mirror, &mut client, p);
        let expected = tick % 10 == 0 && tick <= 1270;
        assert_eq!(frames.len(), usize::from(expected), "tick {tick}");
        delivered.extend(frames.iter().map(|frame| frame.server_tick));
    }
    assert_eq!(delivered.len(), 128);
    assert_eq!(delivered[1], 10);
    assert_eq!(delivered[127], 1270);
    assert!(!delivered.contains(&1280));
    assert!(!delivered.contains(&1290));
    assert_eq!(
        host.transfer_state(p),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: Some(1),
        }))
    );

    // The ended deltas hold a rejected receipt too (a real-time `EndTurn` is
    // `WrongMode`): nothing is logged, the status is cached for the retry.
    let bytes = intent_bytes(&host, p, 1, 1, IntentBody::EndTurn);
    let result = pump_bytes(&mut host, p, &bytes, 100);
    assert_eq!(
        result.status,
        StatusV1::Rejected(crpg_net::protocol::RejectionCode::IllegalAction)
    );
    assert_eq!(
        host.cached_status_v3(p, 1),
        Ok(Some(ReceiptStatus::Rejected(RejectionCode::WrongMode)))
    );
    assert_quiet(&client.drain(&mut host, p, 100).deltas, 1290);

    // A `Resync` at tick 1,290 stages a fresh transfer ...
    let epoch = host.epoch(p).expect("bound");
    let resync = transfer_v3::encode_up(&TransferUp::Resync {
        epoch,
        after: Some(tid(1)),
        reason: ResyncReason::Expired,
    })
    .expect("encodes");
    assert_eq!(
        host.ingest_transfer(p, &resync, 100),
        Ok(TransferDisposition::Resynced)
    );
    let entries = entries_of(&mut host, p);
    assert_eq!(entries.len(), 4);
    assert_eq!(
        transfer_v3::decode_down(&entries[0].bytes),
        Ok(TransferDown::Cancel {
            epoch,
            transfer: tid(1),
            reason: crpg_net::protocol_transfer::CancelReason::Superseded,
        })
    );
    assert!(matches!(
        transfer_v3::decode_down(&entries[1].bytes),
        Ok(TransferDown::Begin(begin)) if begin.transfer == tid(2)
    ));
    assert!(matches!(
        transfer_v3::decode_down(&entries[2].bytes),
        Ok(TransferDown::Chunk(chunk)) if chunk.transfer == tid(2)
    ));
    assert_eq!(entries[3].kind, DeliveryKind::Delta);
    let frame = codec_v3::decode_delta(&entries[3].bytes).expect("a v3 delta");
    assert_eq!(frame.server_tick, 1290);
    assert_eq!(frame.ops.len(), 2, "C-6: Clock and HostFacts");
    // A fresh client takes the new transfer and its delta.
    let mut fresh = Client::new(epoch);
    let first = fresh.drain(&mut host, p, 100);
    assert_eq!(first.deltas.len(), 1);
    assert_eq!(first.deltas[0].server_tick, 1290);

    // ... and the next page comes at tick 1,300, not before.
    for tick in 1291..=1299u64 {
        let frames = step(&mut host, &mut mirror, &mut fresh, p);
        assert_quiet(&frames, tick);
    }
    let frames = step(&mut host, &mut mirror, &mut fresh, p);
    assert_eq!(frames.len(), 1, "tick 1300");
    assert_eq!(frames[0].server_tick, 1300);
    // Still in flight: the page keeps the state and drops the round notice.
    assert_eq!(frames[0].ops, vec![turn_none(325), rt_encounter(1300)]);
    mirror.check(&host);
}

/// E2/E3 reading (D-6, Reading 15): a rejected receipt is one delta frame, so
/// the throttle holds it while the transfer is in flight; the cached status is
/// still committed, and the retry recovers the receipt at the next allowed
/// tick.
#[test]
fn throttled_rejected_receipt_is_recovered_by_the_retry() {
    let mut host = host_of(&tb3_value());
    let mut mirror = Mirror::new(&tb3_value());
    let (p, mut client) = bind_unpublished(&mut host, 100);
    let epoch = host.epoch(p).expect("bound");
    // Tick 0: `n(2)` is not controlled by P, so seq 1 is rejected; the throttle
    // holds the receipt frame.
    let bytes = intent_bytes(
        &host,
        p,
        1,
        2,
        IntentBody::DeclareAction {
            ability: strike(),
            target: n(3),
        },
    );
    let result = pump_bytes(&mut host, p, &bytes, 100);
    assert_eq!(
        result.status,
        StatusV1::Rejected(crpg_net::protocol::RejectionCode::NotAuthorized)
    );
    assert!(!result.cached);
    assert_eq!(
        host.cached_status_v3(p, 1),
        Ok(Some(ReceiptStatus::Rejected(RejectionCode::NotAuthorized)))
    );
    assert_quiet(&client.drain(&mut host, p, 100).deltas, 0);
    // Tick 9: still held; tick 10: a retry is a cache hit that delivers.
    for tick in 1..=9u64 {
        let frames = step(&mut host, &mut mirror, &mut client, p);
        assert_quiet(&frames, tick);
    }
    let retry = pump_bytes(&mut host, p, &bytes, 100);
    assert!(retry.cached);
    assert_quiet(&client.drain(&mut host, p, 100).deltas, 9);
    let frames = step(&mut host, &mut mirror, &mut client, p);
    assert_quiet(&frames, 10);
    let retry = pump_bytes(&mut host, p, &bytes, 100);
    assert!(retry.cached);
    let frames = client.drain(&mut host, p, 100).deltas;
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].server_tick, 10);
    assert_eq!(
        frames[0].ops,
        vec![rejected(epoch, 1, RejectionCode::NotAuthorized)]
    );
    // The frame spent the tenth-tick allowance: an immediate retry is held.
    let retry = pump_bytes(&mut host, p, &bytes, 100);
    assert!(retry.cached);
    assert_quiet(&client.drain(&mut host, p, 100).deltas, 10);
    mirror.check(&host);
}

/// Reading 15: an accepted command's receipt and event ops are dropped for a
/// session in flight (its page keeps the state), and the retry recovers the
/// receipt once the allowance is open; a published session has them at once.
#[test]
fn throttled_accepted_receipt_and_events_are_dropped() {
    let mut host = host_of(&tb3_value());
    let mut mirror = Mirror::new(&tb3_value());
    let (p, mut client) = bind_unpublished(&mut host, 100);
    let epoch = host.epoch(p).expect("bound");
    for tick in 1..=10u64 {
        let frames = step(&mut host, &mut mirror, &mut client, p);
        assert_quiet(&frames, tick);
    }
    // Tick 10: the allowance is open. A's `EndTurn` is applied; the page keeps
    // the new head but holds no receipt and no event op.
    let bytes = intent_bytes(&host, p, 1, 1, IntentBody::EndTurn);
    let result = pump_bytes(&mut host, p, &bytes, 100);
    mirror.act(CombatAction::EndTurn { actor: a() });
    assert_eq!(result.status, StatusV1::Applied);
    assert_eq!(host.executions(), 1);
    assert_eq!(
        host.cached_status_v3(p, 1),
        Ok(Some(ReceiptStatus::Applied))
    );
    let frames = client.drain(&mut host, p, 100).deltas;
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].server_tick, 10);
    assert!(
        frames[0]
            .ops
            .iter()
            .all(|op| !matches!(op, DeltaOp::Receipt { .. } | DeltaOp::TurnStarted { .. })),
        "{:?}",
        frames[0].ops
    );
    assert!(frames[0]
        .ops
        .iter()
        .any(|op| matches!(op, DeltaOp::Turn { .. })));
    assert_eq!(
        client.replica.turn().map(|turn| turn.active),
        Some(Some(n(2)))
    );
    // The retry is held until tick 20, then delivers the exact receipt.
    let retry = pump_bytes(&mut host, p, &bytes, 100);
    assert!(retry.cached);
    assert_quiet(&client.drain(&mut host, p, 100).deltas, 10);
    for tick in 11..=19u64 {
        let frames = step(&mut host, &mut mirror, &mut client, p);
        assert_quiet(&frames, tick);
    }
    let frames = step(&mut host, &mut mirror, &mut client, p);
    assert_quiet(&frames, 20);
    let retry = pump_bytes(&mut host, p, &bytes, 100);
    assert!(retry.cached);
    let frames = client.drain(&mut host, p, 100).deltas;
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].ops, vec![applied(epoch, 1, 10)]);

    // Control: a published session's command has its receipt and events.
    let mut host = host_of(&tb3_value());
    let (q, mut client) = bind_p(&mut host, 100);
    let epoch = host.epoch(q).expect("bound");
    let bytes = intent_bytes(&host, q, 1, 1, IntentBody::EndTurn);
    let result = pump_bytes(&mut host, q, &bytes, 100);
    assert_eq!(result.status, StatusV1::Applied);
    let frames = client.drain(&mut host, q, 100).deltas;
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].ops[0], applied(epoch, 1, 0));
    assert!(frames[0]
        .ops
        .iter()
        .any(|op| matches!(op, DeltaOp::TurnStarted { .. })));
}

/// The throttle is per session: a published session beside an in-flight one
/// keeps every page, and the in-flight one still holds the same state once its
/// allowance opens.
#[test]
fn throttle_is_per_session() {
    let mut host = host_of(&rt3_value());
    let mut mirror = Mirror::new(&rt3_value());
    let (slow, mut slow_client) = bind_unpublished(&mut host, 100);
    let (fast, mut fast_client) = bind_p(&mut host, 100);
    for tick in 1..=10u64 {
        let report = host.advance(100).expect("advances");
        assert!(report.ticked);
        mirror.tick();
        let fast_frames = fast_client.drain(&mut host, fast, 100).deltas;
        assert_eq!(fast_frames.len(), 1, "tick {tick}");
        let slow_frames = slow_client.drain(&mut host, slow, 100).deltas;
        assert_eq!(slow_frames.len(), usize::from(tick == 10), "tick {tick}");
    }
    assert_eq!(
        slow_client.replica.encounter(),
        fast_client.replica.encounter()
    );
    assert_eq!(
        slow_client.replica.turn().cloned(),
        fast_client.replica.turn().cloned()
    );
    mirror.check(&host);
}

// ---------------------------------------------------------------------------
// Nothing else changed.
// ---------------------------------------------------------------------------

/// The first delivery of a v3 session is unchanged by the throttle: three
/// entries, the last C-6's `[Clock, HostFacts]` at tick 0, and the throttle
/// does not hold a session whose transfer is not in flight (the default is
/// the T141c/T141d behaviour).
#[test]
fn first_delivery_and_published_session_are_unthrottled() {
    let mut host = host_of(&tb3_value());
    let peer = host
        .bind_v3_peer(control(&[a()]), full_grants(), facts(), r1(), 100)
        .expect("binds");
    let mut client = Client::new(host.epoch(peer).expect("bound"));
    let drained = client.drain(&mut host, peer, 100);
    assert_eq!(drained.outcomes.len(), 3);
    assert!(matches!(drained.outcomes[2], Outcome::Delta(_)));
    let pages = drained.pages();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].len(), 2);
    assert_eq!(pages[0][0], clock_running());
    assert_eq!(
        client.publish(&mut host, peer, 100),
        TransferDisposition::Published
    );
    // Published, an operator setter page is immediate (no throttle).
    host.pause(100).expect("pauses");
    assert_eq!(
        client.drain(&mut host, peer, 100).pages(),
        vec![vec![clock_paused(PauseReason::Operator)]]
    );
    assert_eq!(
        client.replica.pause(),
        Some(PauseState::Paused {
            reason: Some(PauseReason::Operator)
        })
    );
}
