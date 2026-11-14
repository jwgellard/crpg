//! T141d acceptance suite (`tasks/T141.md`, tests D1 to D4, D6 to D12 and D15;
//! D5, D13 and D14 are the unit tests in `src/host.rs`): v3 publication, public
//! API only.
//!
//! The client is the D54 client: a `ReassemblyPoolV3` gate feeding a
//! `ReplicaWorldV3`, never the v1/v2 receiver. Oracles are independent of the
//! host: a test-owned mirror `HistoryWorld` loaded from the same JSON and
//! driven with the same sim calls (acknowledged after each), and hand-written
//! expected frames, ops and error values. The fixtures are
//! `tests/fixtures/three_combatants.json` as saved-game JSON, edited in memory:
//!
//! - `TB3`: the file as is (A, B, C act in keys 1, 2, 3; A first).
//! - `TB3c`: `TB3` at history version 2 with a round clock (real-time capable
//!   while turn-based).
//! - `RT3`: `TB3c` in real-time mode, no active head, every timeline key 0.
//!
//! Test rule for T141c/T141d: `publish` follows the first `drain` of every v3
//! session (no throttle), and no op list later than 99 ticks after a staging is
//! pinned (no heartbeat).

use crpg_core::{EntityId, Ulid};
use crpg_net::codec_v2;
use crpg_net::codec_v3;
use crpg_net::facts_v3::{EncounterGrants, HostGrants};
use crpg_net::pool_v3::{
    DeltaOutcome, GateMode, ReassemblyPoolV3, ReceiverIdV3, ReplicaUpdateV3, TransferLimitsV3,
    TransferOutcome,
};
use crpg_net::protocol::{
    DeltaOp as OpV1, ReceiptStatus as StatusV1, RejectionCode as CodeV1, LANE_COMBAT,
};
use crpg_net::protocol_transfer::{TransferDown, TransferId, TransferUp, TurnFact};
use crpg_net::protocol_v2::DeltaOp as OpV2;
use crpg_net::protocol_v3::{
    AutoPauseToggles, AutoPauseTrigger, ClockFact, DeltaFrame, DeltaOp, EncounterFacts,
    EncounterMode as NetMode, FactSet, HostFacts, IntentBody, IntentFrame, NetId, OrderFact,
    PauseReason, PauseRights, PoolFact, ReceiptStatus, RejectionCode, SessionEpoch, StatFact,
    StatValue, TimelineEntry,
};
use crpg_net::replica_v3::{PauseState, ReplicaWorldV3};
use crpg_net::transfer_v3;
use crpg_server::capture::{CaptureEntry, CapturedHealth, CapturedTurn, CapturedView};
use crpg_server::host::{
    AdmissionResult, ControlGrant, DeliveryEntry, DeliveryKind, DisclosureGrants, EntityDisclosure,
    EntityFactGrant, FactGrants, GrantUpdate, Host, HostConfig, HostError, HostSettings,
    IngestDisposition, PeerHandle, ProtocolSelection, TransferDisposition, TransferState,
    DEFAULT_HOST_GRANTS,
};
use crpg_sim::{history_hash, CombatAction, CombatError, EncounterMode, HistoryWorld};
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
fn smite() -> Ulid {
    Ulid::from_u128(604)
}
/// The fixture pool `000000000000000000000000JY`.
fn pool_jy() -> Ulid {
    Ulid::from_u128(606)
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
        abilities: vec![strike(), smite(), Ulid::from_u128(605)],
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

fn r0() -> PauseRights {
    PauseRights::default()
}

fn control(actors: &[EntityId]) -> ControlGrant {
    ControlGrant {
        actors: actors.to_vec(),
    }
}

fn toggles(bits: u8) -> AutoPauseToggles {
    AutoPauseToggles::from_bits(bits).expect("defined bits")
}

fn settings(bits: u8, timer: Option<u32>) -> HostSettings {
    HostSettings {
        initial_mode: EncounterMode::TurnBased,
        auto_pause: toggles(bits),
        turn_timer_ms: timer,
    }
}

// ---------------------------------------------------------------------------
// Expected-op constructors (hand-written, never read back from the host).
// ---------------------------------------------------------------------------

/// The fact set of one fixture row under `FACTS`: the action pool at
/// `pools` (`None` withholds it), `order` as given, `alpha` 8, no effects and
/// no tags.
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
    DeltaOp::EntityFacts {
        entity: n(entity),
        facts,
    }
}

fn order(ability: Ulid, target: u64) -> Option<Option<OrderFact>> {
    Some(Some(OrderFact {
        ability,
        target: n(target),
        repeat: false,
    }))
}

fn health_op(entity: u64, health: u32, max_health: u32, dead: bool) -> DeltaOp {
    DeltaOp::Health {
        entity: n(entity),
        health,
        max_health,
        dead,
    }
}

fn timeline(entries: &[(i32, u64)]) -> DeltaOp {
    DeltaOp::Timeline {
        entries: Some(
            entries
                .iter()
                .map(|(key, entity)| TimelineEntry {
                    key: *key,
                    entity: n(*entity),
                })
                .collect(),
        ),
    }
}

/// `Encounter` with the clock `{ 4, elapsed }` disclosed.
fn encounter(mode: Option<NetMode>, pending: Option<Option<NetMode>>, elapsed: u64) -> DeltaOp {
    DeltaOp::Encounter(EncounterFacts {
        mode,
        pending_mode: pending,
        clock: Some(Some(ClockFact {
            round_ticks: 4,
            elapsed,
        })),
    })
}

fn rt_encounter(elapsed: u64) -> DeltaOp {
    encounter(Some(NetMode::RealTime), Some(None), elapsed)
}

fn receipt(epoch: [u8; 16], seq: u64, tick: u64, status: ReceiptStatus) -> DeltaOp {
    DeltaOp::Receipt {
        epoch,
        lane: LANE_COMBAT,
        seq,
        processed_tick: tick,
        status,
    }
}

fn applied(epoch: [u8; 16], seq: u64) -> DeltaOp {
    receipt(epoch, seq, 0, ReceiptStatus::Applied)
}

fn rejected(epoch: [u8; 16], seq: u64, code: RejectionCode) -> DeltaOp {
    receipt(epoch, seq, 0, ReceiptStatus::Rejected(code))
}

fn clock_running() -> DeltaOp {
    DeltaOp::Clock { paused: None }
}

fn clock_paused(reason: PauseReason) -> DeltaOp {
    DeltaOp::Clock {
        paused: Some(Some(reason)),
    }
}

fn host_facts(rights: PauseRights, auto_pause: Option<u8>, timer: Option<Option<u32>>) -> DeltaOp {
    DeltaOp::HostFacts(HostFacts {
        rights,
        auto_pause: auto_pause.map(toggles),
        turn_timer_ms: timer,
    })
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

    fn mode(&mut self, mode: EncounterMode) {
        self.0.request_mode(mode).expect("the sim accepts it");
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

/// Binds a v3 session, drains its first delivery and publishes it (the rule).
fn bind3(
    host: &mut Host,
    actors: &[EntityId],
    grants: DisclosureGrants,
    facts: FactGrants,
    rights: PauseRights,
    now: u64,
) -> (PeerHandle, Client) {
    let peer = host
        .bind_v3_peer(control(actors), grants, facts, rights, now)
        .expect("binds");
    let mut client = Client::new(host.epoch(peer).expect("bound"));
    client.drain(host, peer, now);
    assert_eq!(
        client.publish(host, peer, now),
        TransferDisposition::Published
    );
    (peer, client)
}

/// `P = bind_v3_peer([A], FULL(A,B,C), FACTS, R1, now)`, published.
fn bind_p(host: &mut Host, now: u64) -> (PeerHandle, Client) {
    bind3(
        host,
        &[a()],
        full_grants(&[a(), b(), c()]),
        facts(),
        r1(),
        now,
    )
}

/// Everything a refusal must leave alone (the contract's list).
#[derive(Debug, PartialEq, Eq)]
struct State {
    authority: [u8; 32],
    last_capture: u64,
    capture_acknowledged: u64,
    executions: u64,
    paused: Option<PauseReason>,
    remaining: Option<u64>,
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
        capture_acknowledged: host.capture_acknowledged(),
        executions: host.executions(),
        paused: host.paused(),
        remaining: host.turn_timer_remaining_ms(),
        peers,
    }
}

/// The v3 intent bytes of one command.
fn intent_bytes(
    host: &Host,
    peer: PeerHandle,
    seq: u64,
    actor: u64,
    observed_tick: u64,
    body: IntentBody,
) -> Vec<u8> {
    codec_v3::encode_intent(&IntentFrame {
        epoch: host.epoch(peer).expect("bound"),
        lane: LANE_COMBAT,
        seq,
        observed_tick,
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
    summary.results[0].clone()
}

/// One v3 intent at the server's current tick, through `ingest` then `pump`.
fn send3(
    host: &mut Host,
    peer: PeerHandle,
    seq: u64,
    actor: u64,
    body: IntentBody,
    now: u64,
) -> AdmissionResult {
    let tick = host.server_tick();
    let bytes = intent_bytes(host, peer, seq, actor, tick, body);
    pump_bytes(host, peer, &bytes, now)
}

fn declare(ability: Ulid, target: u64) -> IntentBody {
    IntentBody::DeclareAction {
        ability,
        target: n(target),
    }
}

fn assert_applied(result: &AdmissionResult, seq: u64) {
    assert_eq!(result.seq, Some(seq));
    assert_eq!(result.status, StatusV1::Applied);
    assert!(!result.cached);
    assert!(!result.retained_in_ingress);
}

fn assert_rejected(result: &AdmissionResult, seq: u64, wire: CodeV1) {
    assert_eq!(result.seq, Some(seq));
    assert_eq!(result.status, StatusV1::Rejected(wire));
    assert!(!result.cached);
    assert!(!result.retained_in_ingress);
}

/// Decodes a v2 delta's ops.
fn v2_ops(entry: &DeliveryEntry) -> Vec<OpV2> {
    codec_v2::decode_delta(&entry.bytes)
        .expect("a v2 frame")
        .ops
}

fn decoded(entry: &DeliveryEntry) -> TransferDown {
    transfer_v3::decode_down(&entry.bytes).expect("a 0x82 frame")
}

/// The drained pages of one session: every frame's ops, in order.
fn pages(host: &mut Host, client: &mut Client, peer: PeerHandle, now: u64) -> Vec<Vec<DeltaOp>> {
    client.drain(host, peer, now).pages()
}

/// The replica's health row for `net`.
fn replica_health(client: &Client, net: u64) -> (u32, u32, bool) {
    let health = client
        .replica
        .entity(n(net))
        .and_then(|row| row.health.clone())
        .expect("a health fact");
    (health.health, health.max_health, health.dead)
}

// ---------------------------------------------------------------------------
// D1, D8.
// ---------------------------------------------------------------------------

/// D1: one turn-based command is one page for a v3 session, in the Reading 11
/// layout, and one unchanged v2 frame for a v2 session.
#[test]
fn d1_tb_command_page() {
    let mut host = host_of(&tb3_value());
    let mut mirror = Mirror::new(&tb3_value());
    let v = host
        .bind_peer(control(&[]), full_grants(&[a(), b(), c()]), 0)
        .expect("binds");
    let first = host.take_delivery_entries(v).expect("bound");
    host.acknowledge_delivery(v, first.last().expect("a frame").ordinal)
        .expect("acks");
    let (p, mut client) = bind_p_all(&mut host, 0);
    let epoch = host.epoch(p).expect("bound");

    let result = send3(&mut host, p, 1, 1, declare(strike(), 2), 10);
    mirror.act(CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    });
    assert_applied(&result, 1);
    assert_eq!(host.executions(), 1);
    assert_eq!(host.last_capture(), 1);
    mirror.check(&host);

    let drained = client.drain(&mut host, p, 10);
    assert_eq!(
        drained.deltas,
        vec![DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 0,
            event_seq: 2,
            ops: vec![
                applied(epoch, 1),
                DeltaOp::ActionResolved {
                    actor: n(1),
                    target: n(2),
                    ability: strike(),
                    outcome: "success".to_owned(),
                    damage: 3,
                },
                DeltaOp::TurnStarted {
                    actor: n(2),
                    round: 0,
                },
                health_op(2, 7, 10, false),
                DeltaOp::Turn {
                    active: Some(n(2)),
                    round: 0,
                },
                timeline(&[(2, 2), (3, 3)]),
                facts_op(1, fs(Some((0, 1)), Some(None))),
            ],
        }]
    );
    assert_eq!(replica_health(&client, 2), (7, 10, false));
    assert_eq!(client.replica.next_event_seq(), 3);

    // The v2 session gets exactly its unchanged frame, no receipt, no v3 op.
    let entries = host.take_delivery_entries(v).expect("bound");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, DeliveryKind::Delta);
    let frame = codec_v2::decode_delta(&entries[0].bytes).expect("a v2 frame");
    assert_eq!(frame.event_seq, 2);
    assert_eq!(
        frame.ops,
        vec![
            OpV2::ActionResolved {
                actor: n(1),
                target: n(2),
                ability: strike(),
                outcome: "success".to_owned(),
                damage: 3,
            },
            OpV2::TurnStarted {
                actor: n(2),
                round: 0,
            },
            OpV2::Legacy(OpV1::Health {
                entity: n(1),
                health: 10,
                max_health: 10,
                dead: false,
            }),
            OpV2::Legacy(OpV1::Health {
                entity: n(2),
                health: 7,
                max_health: 10,
                dead: false,
            }),
            OpV2::Legacy(OpV1::Turn {
                active: Some(n(2)),
                round: 0,
            }),
        ]
    );

    // A retry of the acknowledged, applied seq 1 is a cache hit that re-sends
    // the receipt alone (D-1).
    let tick = host.server_tick();
    let bytes = intent_bytes(&host, p, 1, 1, tick, declare(strike(), 2));
    let retry = pump_bytes(&mut host, p, &bytes, 11);
    assert_eq!(retry.status, StatusV1::Applied);
    assert!(retry.cached);
    assert_eq!(host.executions(), 1);
    assert_eq!(
        pages(&mut host, &mut client, p, 11),
        vec![vec![applied(epoch, 1)]]
    );
    mirror.check(&host);

    // Control: a v2-format intent into the v3 session is refused with no
    // change; the v3 intent that follows is accepted.
    let v2_bytes = codec_v2::encode_intent(&crpg_net::protocol::IntentFrame {
        epoch,
        lane: LANE_COMBAT,
        seq: 2,
        observed_tick: 0,
        actor: n(2),
        body: crpg_net::protocol::IntentBody::EndTurn,
    })
    .expect("encodes");
    let before = state(&mut host, &[p, v]);
    let refused = pump_bytes(&mut host, p, &v2_bytes, 12);
    assert_eq!(refused.seq, None);
    assert_eq!(
        refused.status,
        StatusV1::Rejected(CodeV1::UnsupportedVersion)
    );
    assert_eq!(state(&mut host, &[p, v]), before);
    assert_applied(&send3(&mut host, p, 2, 2, IntentBody::EndTurn, 13), 2);
}

/// `bind_p` for a session controlling A, B and C (D1, D8, D9).
fn bind_p_all(host: &mut Host, now: u64) -> (PeerHandle, Client) {
    bind3(
        host,
        &[a(), b(), c()],
        full_grants(&[a(), b(), c()]),
        facts(),
        r1(),
        now,
    )
}

/// D8: the record of an accepted v3 command holds the session's capture view,
/// named by the session's own ids; a session that gets no frame adds none.
#[test]
fn d8_capture_views() {
    let mut host = host_of(&tb3_value());
    let (p, _client) = bind_p_all(&mut host, 0);
    let epoch = host.epoch(p).expect("bound");
    assert_applied(&send3(&mut host, p, 1, 1, declare(strike(), 2), 10), 1);
    let entries = host.read_capture_entries(0, 16).expect("reads");
    assert_eq!(entries.len(), 1);
    let CaptureEntry::Command(record) = &entries[0] else {
        panic!("a command entry");
    };
    assert_eq!(record.seq, 1);
    assert_eq!(record.history_start, 4);
    assert_eq!(record.history_end, 6);
    assert_eq!(
        record.views,
        vec![CapturedView {
            epoch,
            health: vec![
                CapturedHealth {
                    entity: n(1),
                    health: 10,
                    max_health: 10,
                    dead: false,
                },
                CapturedHealth {
                    entity: n(2),
                    health: 7,
                    max_health: 10,
                    dead: false,
                },
            ],
            turn: Some(CapturedTurn {
                active: Some(n(2)),
                round: 0,
            }),
        }]
    );

    // Control: a v2 session beside it adds its own view with its own ids, and
    // the two views stay ascending by epoch.
    let v = host
        .bind_peer(control(&[]), full_grants(&[a(), b(), c()]), 20)
        .expect("binds");
    assert_applied(&send3(&mut host, p, 2, 2, IntentBody::EndTurn, 21), 2);
    let entries = host.read_capture_entries(1, 16).expect("reads");
    let CaptureEntry::Command(second) = &entries[0] else {
        panic!("a command entry");
    };
    let epochs: Vec<[u8; 16]> = second.views.iter().map(|view| view.epoch).collect();
    assert_eq!(epochs, vec![epoch, host.epoch(v).expect("bound")]);
}

/// D8 (no frame, no view): a session whose page is empty adds no view.
#[test]
fn d8_a_session_with_no_frame_adds_no_view() {
    let mut host = host_of(&rt3_value());
    // P controls A and sees A's order; Q controls B and never sees it.
    let (p, _p_client) = bind_p(&mut host, 0);
    let (q, _q_client) = bind3(
        &mut host,
        &[b()],
        full_grants(&[a(), b(), c()]),
        facts(),
        r0(),
        0,
    );
    assert_applied(&send3(&mut host, p, 1, 1, declare(strike(), 2), 10), 1);
    let entries = host.read_capture_entries(0, 16).expect("reads");
    let CaptureEntry::Command(record) = &entries[0] else {
        panic!("a command entry");
    };
    let epochs: Vec<[u8; 16]> = record.views.iter().map(|view| view.epoch).collect();
    assert_eq!(epochs, vec![host.epoch(p).expect("bound")]);
    assert!(!epochs.contains(&host.epoch(q).expect("bound")));
}

// ---------------------------------------------------------------------------
// D2.
// ---------------------------------------------------------------------------

/// Asserts that `peer` gets exactly `expected` frames and `other` gets none.
fn assert_pages(
    host: &mut Host,
    (peer, client): (PeerHandle, &mut Client),
    other: (PeerHandle, &mut Client),
    now: u64,
    expected: Vec<Vec<DeltaOp>>,
) {
    assert_eq!(pages(host, client, peer, now), expected);
    assert_eq!(
        pages(host, other.1, other.0, now),
        Vec::<Vec<DeltaOp>>::new()
    );
}

/// D2: each setter publishes one frame to its own session; a revoked grant is
/// re-sent as withheld; only the settings reach other sessions.
#[test]
fn d2_setters_and_revocation() {
    let mut host = host_of(&tb3_value());
    let mirror = Mirror::new(&tb3_value());
    let (p, mut pc) = bind3(
        &mut host,
        &[a()],
        full_grants(&[a(), b()]),
        facts(),
        r1(),
        0,
    );
    let (q, mut qc) = bind3(&mut host, &[], full_grants(&[a(), b()]), facts(), r0(), 0);

    // A widening grant: the new entity, its health, the timeline and its facts.
    assert_eq!(
        host.set_grants(p, full_grants(&[a(), b(), c()])),
        Ok(GrantUpdate::Unchanged)
    );
    assert_pages(
        &mut host,
        (p, &mut pc),
        (q, &mut qc),
        1,
        vec![vec![
            DeltaOp::EntityEnter { entity: n(3) },
            health_op(3, 3, 3, false),
            timeline(&[(1, 1), (2, 2), (3, 3)]),
            facts_op(3, fs(Some((1, 1)), None)),
        ]],
    );
    assert_eq!(replica_health(&pc, 3), (3, 3, false));

    // A's pools withheld: the facts are re-sent with the family withheld.
    let mut fg = facts();
    fg.entities[0].pools = false;
    assert_eq!(host.set_fact_grants(p, fg.clone()), Ok(()));
    assert_pages(
        &mut host,
        (p, &mut pc),
        (q, &mut qc),
        2,
        vec![vec![facts_op(1, fs(None, Some(None)))]],
    );

    // The host grants withheld: `HostFacts` alone (the clock image is the same
    // for a running host).
    fg.host = HostGrants::default();
    assert_eq!(host.set_fact_grants(p, fg.clone()), Ok(()));
    assert_pages(
        &mut host,
        (p, &mut pc),
        (q, &mut qc),
        3,
        vec![vec![host_facts(r1(), None, None)]],
    );

    // The mode withheld.
    fg.encounter.mode = false;
    assert_eq!(host.set_fact_grants(p, fg.clone()), Ok(()));
    assert_pages(
        &mut host,
        (p, &mut pc),
        (q, &mut qc),
        4,
        vec![vec![DeltaOp::Encounter(EncounterFacts {
            mode: None,
            pending_mode: Some(None),
            clock: Some(None),
        })]],
    );

    // Control withdrawn: the order fact is withheld.
    assert_eq!(host.set_control(p, control(&[])), Ok(()));
    assert_pages(
        &mut host,
        (p, &mut pc),
        (q, &mut qc),
        5,
        vec![vec![facts_op(1, fs(None, None))]],
    );

    // The viewer's own rights.
    assert_eq!(host.set_pause_rights(p, r0()), Ok(()));
    assert_pages(
        &mut host,
        (p, &mut pc),
        (q, &mut qc),
        6,
        vec![vec![host_facts(r0(), None, None)]],
    );

    // Settings: P is not granted the toggles, so P gets nothing; Q is.
    assert_eq!(host.set_settings(settings(0x19, None), 7), Ok(()));
    assert_eq!(pages(&mut host, &mut pc, p, 7), Vec::<Vec<DeltaOp>>::new());
    assert_eq!(
        pages(&mut host, &mut qc, q, 7),
        vec![vec![host_facts(r0(), Some(0x19), Some(None))]]
    );

    // Restoring the host grants re-sends the toggles to P alone.
    fg.host = DEFAULT_HOST_GRANTS;
    assert_eq!(host.set_fact_grants(p, fg), Ok(()));
    assert_pages(
        &mut host,
        (p, &mut pc),
        (q, &mut qc),
        8,
        vec![vec![host_facts(r0(), Some(0x19), Some(None))]],
    );

    // Settings back to none: both sessions.
    assert_eq!(host.set_settings(settings(0x00, None), 9), Ok(()));
    assert_eq!(
        pages(&mut host, &mut pc, p, 9),
        vec![vec![host_facts(r0(), Some(0), Some(None))]]
    );
    assert_eq!(
        pages(&mut host, &mut qc, q, 9),
        vec![vec![host_facts(r0(), Some(0), Some(None))]]
    );
    mirror.check(&host);

    // Controls: an unchanged setter publishes nothing; the setters keep their
    // error precedence (T141c), and the replica is exact after all of it.
    let before = state(&mut host, &[p, q]);
    assert_eq!(host.set_settings(settings(0x00, None), 10), Ok(()));
    assert_eq!(state(&mut host, &[p, q]), before);
    assert_eq!(
        pc.replica.host_facts(),
        Some(&HostFacts {
            rights: r0(),
            auto_pause: Some(toggles(0)),
            turn_timer_ms: Some(None),
        })
    );
    let gone = retired_handle(&mut host, 10);
    assert_eq!(
        host.set_pause_rights(gone, r1()),
        Err(HostError::UnknownPeer)
    );
}

/// A handle that was bound and then retired: it never binds again.
fn retired_handle(host: &mut Host, now: u64) -> PeerHandle {
    let peer = host
        .bind_peer(control(&[]), full_grants(&[a()]), now)
        .expect("binds");
    host.unbind_peer(peer);
    peer
}

// ---------------------------------------------------------------------------
// D3.
// ---------------------------------------------------------------------------

/// D3: a real-time order, a tick's events, the clock, the timeline and the
/// refresh reach a v3 session in order.
#[test]
fn d3_rt_tick_pages() {
    let mut host = host_of(&rt3_value());
    let mut mirror = Mirror::new(&rt3_value());
    let (p, mut client) = bind_p(&mut host, 100);

    let order_action = CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    };
    assert_eq!(host.host_action(&order_action), Ok(None));
    mirror.act(order_action);
    assert_eq!(
        pages(&mut host, &mut client, p, 100),
        vec![vec![facts_op(1, fs(Some((1, 1)), order(strike(), 2)))]]
    );
    mirror.check(&host);

    host.tick().expect("ticks");
    mirror.tick();
    assert_eq!(
        pages(&mut host, &mut client, p, 100),
        vec![vec![
            DeltaOp::ActionResolved {
                actor: n(1),
                target: n(2),
                ability: strike(),
                outcome: "success".to_owned(),
                damage: 3,
            },
            health_op(2, 7, 10, false),
            rt_encounter(1),
            timeline(&[(0, 2), (0, 3), (5, 1)]),
            facts_op(1, fs(Some((0, 1)), Some(None))),
        ]]
    );
    mirror.check(&host);

    for elapsed in 2..=3 {
        host.tick().expect("ticks");
        mirror.tick();
        assert_eq!(
            pages(&mut host, &mut client, p, 100),
            vec![vec![rt_encounter(elapsed)]]
        );
    }
    host.tick().expect("ticks");
    mirror.tick();
    assert_eq!(
        pages(&mut host, &mut client, p, 100),
        vec![vec![
            DeltaOp::RoundStarted { round: 1 },
            DeltaOp::Turn {
                active: None,
                round: 1,
            },
            rt_encounter(4),
            facts_op(1, fs(Some((1, 1)), Some(None))),
        ]]
    );
    mirror.check(&host);
    // The host operations are never peer commands, and the replica is exact.
    assert_eq!(host.executions(), 0);
    assert_eq!(
        client.replica.turn(),
        Some(&TurnFact {
            active: None,
            round: 1,
        })
    );
    assert_eq!(client.replica.server_tick(), 4);
}

// ---------------------------------------------------------------------------
// D4.
// ---------------------------------------------------------------------------

/// D4: the v3 verbs, rights, receipt codes, the exact-status cache and the
/// retry, in the contract's order.
#[test]
fn d4_v3_intents() {
    let mut host = host_of(&rt3_value());
    let mut mirror = Mirror::new(&rt3_value());
    let (p, mut pc) = bind_p(&mut host, 0);
    let (q, mut qc) = bind3(
        &mut host,
        &[b()],
        full_grants(&[a(), b(), c()]),
        facts(),
        r0(),
        0,
    );
    let pe = host.epoch(p).expect("bound");
    let qe = host.epoch(q).expect("bound");

    // 1. A real-time declare sets the standing order.
    let result = send3(&mut host, p, 1, 1, declare(strike(), 2), 10);
    mirror.act(CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    });
    assert_applied(&result, 1);
    assert_eq!(
        pages(&mut host, &mut pc, p, 10),
        vec![vec![
            applied(pe, 1),
            facts_op(1, fs(Some((1, 1)), order(strike(), 2))),
        ]]
    );
    assert_eq!(pages(&mut host, &mut qc, q, 10), Vec::<Vec<DeltaOp>>::new());

    // 2. `Stop` clears it.
    let result = send3(&mut host, p, 2, 1, IntentBody::Stop, 11);
    mirror.act(CombatAction::Stop { actor: a() });
    assert_applied(&result, 2);
    assert_eq!(
        pages(&mut host, &mut pc, p, 11),
        vec![vec![
            applied(pe, 2),
            facts_op(1, fs(Some((1, 1)), Some(None)))
        ]]
    );
    assert_eq!(pages(&mut host, &mut qc, q, 11), Vec::<Vec<DeltaOp>>::new());

    // 3. `EndTurn` in real time: code 22 on the wire, 18 in the trusted API.
    let tick = host.server_tick();
    let bytes3 = intent_bytes(&host, p, 3, 1, tick, IntentBody::EndTurn);
    let result = pump_bytes(&mut host, p, &bytes3, 12);
    assert_rejected(&result, 3, CodeV1::IllegalAction);
    assert_eq!(
        host.cached_status(p, 3),
        Ok(Some(StatusV1::Rejected(CodeV1::IllegalAction)))
    );
    assert_eq!(
        host.cached_status_v3(p, 3),
        Ok(Some(ReceiptStatus::Rejected(RejectionCode::WrongMode)))
    );
    assert_eq!(
        host.last_combat_error(),
        Some(CombatError::WrongMode {
            mode: EncounterMode::RealTime,
        })
    );
    assert_eq!(
        pages(&mut host, &mut pc, p, 12),
        vec![vec![rejected(pe, 3, RejectionCode::WrongMode)]]
    );
    assert_eq!(host.executions(), 2);
    mirror.check(&host);

    // Retry: the exact bytes again after the frame was acknowledged. The
    // trusted result keeps the v1 code, the frame the exact one (finding 8).
    let retry = pump_bytes(&mut host, p, &bytes3, 13);
    assert_eq!(retry.seq, Some(3));
    assert_eq!(retry.status, StatusV1::Rejected(CodeV1::IllegalAction));
    assert!(retry.cached);
    assert_eq!(
        pages(&mut host, &mut pc, p, 13),
        vec![vec![rejected(pe, 3, RejectionCode::WrongMode)]]
    );
    assert_eq!(host.executions(), 2);

    // 4. A mode request from a peer with the `pause` right: both sessions see
    // the pending mode, P after its receipt.
    let result = send3(
        &mut host,
        p,
        4,
        1,
        IntentBody::RequestMode {
            mode: NetMode::TurnBased,
        },
        14,
    );
    mirror.mode(EncounterMode::TurnBased);
    assert_applied(&result, 4);
    let pending = encounter(Some(NetMode::RealTime), Some(Some(NetMode::TurnBased)), 0);
    assert_eq!(
        pages(&mut host, &mut pc, p, 14),
        vec![vec![applied(pe, 4), pending.clone()]]
    );
    assert_eq!(pages(&mut host, &mut qc, q, 14), vec![vec![pending]]);

    // 5. The same request without the right is `NotAuthorized` (Question 3).
    let result = send3(
        &mut host,
        q,
        1,
        2,
        IntentBody::RequestMode {
            mode: NetMode::RealTime,
        },
        15,
    );
    assert_rejected(&result, 1, CodeV1::NotAuthorized);
    assert_eq!(
        pages(&mut host, &mut qc, q, 15),
        vec![vec![rejected(qe, 1, RejectionCode::NotAuthorized)]]
    );

    // 6. `Pause` without the right: code 30 on the wire, 17 in the trusted API.
    let result = send3(&mut host, q, 2, 2, IntentBody::Pause, 16);
    assert_rejected(&result, 2, CodeV1::NotAuthorized);
    assert_eq!(
        host.cached_status(q, 2),
        Ok(Some(StatusV1::Rejected(CodeV1::NotAuthorized)))
    );
    assert_eq!(
        host.cached_status_v3(q, 2),
        Ok(Some(ReceiptStatus::Rejected(RejectionCode::NoPauseRight)))
    );
    assert_eq!(
        pages(&mut host, &mut qc, q, 16),
        vec![vec![rejected(qe, 2, RejectionCode::NoPauseRight)]]
    );
    assert_eq!(host.paused(), None);

    // 7. Control is checked before the right.
    let result = send3(&mut host, q, 3, 1, IntentBody::Pause, 17);
    assert_rejected(&result, 3, CodeV1::NotAuthorized);
    assert_eq!(
        pages(&mut host, &mut qc, q, 17),
        vec![vec![rejected(qe, 3, RejectionCode::NotAuthorized)]]
    );
    assert_eq!(host.paused(), None);

    // 8. `Pause` with the right: the player's reason reaches both sessions.
    let result = send3(&mut host, p, 5, 1, IntentBody::Pause, 18);
    assert_applied(&result, 5);
    assert_eq!(host.paused(), Some(PauseReason::Player));
    assert_eq!(
        pages(&mut host, &mut pc, p, 18),
        vec![vec![applied(pe, 5), clock_paused(PauseReason::Player)]]
    );
    assert_eq!(
        pages(&mut host, &mut qc, q, 18),
        vec![vec![clock_paused(PauseReason::Player)]]
    );

    // 9. A second `Pause` changes nothing and answers with the receipt alone.
    let result = send3(&mut host, p, 6, 1, IntentBody::Pause, 19);
    assert_applied(&result, 6);
    assert_eq!(host.paused(), Some(PauseReason::Player));
    assert_eq!(pages(&mut host, &mut pc, p, 19), vec![vec![applied(pe, 6)]]);
    assert_eq!(pages(&mut host, &mut qc, q, 19), Vec::<Vec<DeltaOp>>::new());

    // 10. `Unpause`.
    let result = send3(&mut host, p, 7, 1, IntentBody::Unpause, 20);
    assert_applied(&result, 7);
    assert_eq!(host.paused(), None);
    assert_eq!(
        pages(&mut host, &mut pc, p, 20),
        vec![vec![applied(pe, 7), clock_running()]]
    );
    assert_eq!(
        pages(&mut host, &mut qc, q, 20),
        vec![vec![clock_running()]]
    );

    // Steps 1, 2, 4, 8, 9 and 10 executed; the rejections did not.
    assert_eq!(host.executions(), 6);
    assert_eq!(host.last_capture(), 6);
    mirror.check(&host);

    // An `Unpause` without its right is refused the same way; the control is
    // step 10 above.
    let result = send3(&mut host, q, 4, 2, IntentBody::Unpause, 21);
    assert_rejected(&result, 4, CodeV1::NotAuthorized);
    assert_eq!(
        host.cached_status_v3(q, 4),
        Ok(Some(ReceiptStatus::Rejected(RejectionCode::NoPauseRight)))
    );
    assert_eq!(host.executions(), 6);

    // `cached_status_v3`: the applied status is exact too, an unknown seq is
    // `None`, and the call is v3-only.
    assert_eq!(
        host.cached_status_v3(p, 1),
        Ok(Some(ReceiptStatus::Applied))
    );
    assert_eq!(host.cached_status_v3(p, 99), Ok(None));
    assert_eq!(host.cached_status(p, 1), Ok(Some(StatusV1::Applied)));
    let v = host
        .bind_peer(control(&[a()]), full_grants(&[a(), b(), c()]), 22)
        .expect("binds");
    assert_eq!(host.cached_status_v3(v, 1), Err(HostError::NotV3Peer));
    let gone = retired_handle(&mut host, 22);
    assert_eq!(host.cached_status_v3(gone, 1), Err(HostError::UnknownPeer));

    // A v2 session's out-of-mode `EndTurn` is still `IllegalAction` on the
    // wire: the exact code is a v3 fact.
    let first = host.take_delivery_entries(v).expect("bound");
    host.acknowledge_delivery(v, first.last().expect("a frame").ordinal)
        .expect("acks");
    let bytes = codec_v2::encode_intent(&crpg_net::protocol::IntentFrame {
        epoch: host.epoch(v).expect("bound"),
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: n(1),
        body: crpg_net::protocol::IntentBody::EndTurn,
    })
    .expect("encodes");
    let result = pump_bytes(&mut host, v, &bytes, 23);
    assert_rejected(&result, 1, CodeV1::IllegalAction);
    let entries = host.take_delivery_entries(v).expect("bound");
    assert_eq!(
        v2_ops(&entries[0]),
        vec![OpV2::Legacy(OpV1::Receipt {
            epoch: host.epoch(v).expect("bound"),
            lane: LANE_COMBAT,
            seq: 1,
            processed_tick: 0,
            status: StatusV1::Rejected(CodeV1::IllegalAction),
        })]
    );
    assert_eq!(
        host.cached_status(v, 1),
        Ok(Some(StatusV1::Rejected(CodeV1::IllegalAction)))
    );
    assert_eq!(
        host.last_combat_error(),
        Some(CombatError::WrongMode {
            mode: EncounterMode::RealTime,
        })
    );
}

/// D4 (turn-based): the sim's own codes reach a v3 receipt, and the accepted
/// control beside them succeeds.
#[test]
fn d4_tb_codes() {
    let mut host = host_of(&tb3_value());
    let mut mirror = Mirror::new(&tb3_value());
    let (p, mut client) = bind_p(&mut host, 100);
    let epoch = host.epoch(p).expect("bound");

    // `RequestMode { RealTime }` on a ruleset with no clock is refused by the
    // sim under the accepted reservation: code 21.
    let result = send3(
        &mut host,
        p,
        1,
        1,
        IntentBody::RequestMode {
            mode: NetMode::RealTime,
        },
        101,
    );
    assert_rejected(&result, 1, CodeV1::IllegalAction);
    assert_eq!(host.last_combat_error(), Some(CombatError::ModeUnsupported));
    assert_eq!(
        host.cached_status_v3(p, 1),
        Ok(Some(ReceiptStatus::Rejected(
            RejectionCode::ModeUnsupported
        )))
    );
    // `Stop` in turn-based mode: code 22.
    let result = send3(&mut host, p, 2, 1, IntentBody::Stop, 102);
    assert_rejected(&result, 2, CodeV1::IllegalAction);
    assert_eq!(
        host.cached_status_v3(p, 2),
        Ok(Some(ReceiptStatus::Rejected(RejectionCode::WrongMode)))
    );
    assert_eq!(
        pages(&mut host, &mut client, p, 102),
        vec![
            vec![rejected(epoch, 1, RejectionCode::ModeUnsupported)],
            vec![rejected(epoch, 2, RejectionCode::WrongMode)],
        ]
    );
    // Anything that is not one of the nine never reaches the sim: an actor this
    // session does not control is `NotAuthorized`.
    let result = send3(&mut host, p, 3, 2, IntentBody::EndTurn, 103);
    assert_rejected(&result, 3, CodeV1::NotAuthorized);
    assert_eq!(host.executions(), 0);
    // Control: the head's own `EndTurn` is accepted, with no refusal code.
    let result = send3(&mut host, p, 4, 1, IntentBody::EndTurn, 104);
    mirror.act(CombatAction::EndTurn { actor: a() });
    assert_applied(&result, 4);
    assert_eq!(host.executions(), 1);
    mirror.check(&host);
}

// ---------------------------------------------------------------------------
// D6.
// ---------------------------------------------------------------------------

/// D6: `OrderEnded` reaches only the controlling session, and a round leaks
/// only with its grant.
#[test]
fn d6_event_grants() {
    let mut host = host_of(&rt3_value());
    let mut mirror = Mirror::new(&rt3_value());
    let (p, mut pc) = bind3(
        &mut host,
        &[c()],
        full_grants(&[a(), b(), c()]),
        facts(),
        r0(),
        0,
    );
    let mut no_round = full_grants(&[a(), b(), c()]);
    no_round.turn_round = false;
    let (q, mut qc) = bind3(&mut host, &[], no_round, facts(), r0(), 0);

    let smite_b = CombatAction::UseAbility {
        actor: a(),
        ability: smite(),
        target: b(),
    };
    assert_eq!(host.host_action(&smite_b), Ok(None));
    mirror.act(smite_b);
    let strike_b = CombatAction::UseAbility {
        actor: c(),
        ability: strike(),
        target: b(),
    };
    assert_eq!(host.host_action(&strike_b), Ok(None));
    mirror.act(strike_b);
    // P controls C, so it learns C's order; Q learns neither order.
    let drained = pages(&mut host, &mut pc, p, 1);
    assert_eq!(
        drained,
        vec![vec![facts_op(3, fs(Some((1, 1)), order(strike(), 2)))]]
    );
    assert_eq!(pages(&mut host, &mut qc, q, 1), Vec::<Vec<DeltaOp>>::new());

    host.tick().expect("ticks");
    mirror.tick();
    let shared = |ops: &mut Vec<DeltaOp>| {
        ops.push(health_op(2, 0, 10, true));
        ops.push(rt_encounter(1));
        ops.push(timeline(&[(0, 3), (5, 1)]));
        ops.push(facts_op(1, fs(Some((0, 1)), None)));
    };
    let resolved = DeltaOp::ActionResolved {
        actor: n(1),
        target: n(2),
        ability: smite(),
        outcome: "critical_success".to_owned(),
        damage: 4_000_000_000,
    };
    let mut expected_p = vec![resolved.clone(), DeltaOp::Died { entity: n(2) }];
    expected_p.push(DeltaOp::OrderEnded { actor: n(3) });
    shared(&mut expected_p);
    expected_p.push(facts_op(3, fs(Some((1, 1)), Some(None))));
    assert_eq!(pages(&mut host, &mut pc, p, 2), vec![expected_p]);
    let mut expected_q = vec![resolved, DeltaOp::Died { entity: n(2) }];
    shared(&mut expected_q);
    assert_eq!(pages(&mut host, &mut qc, q, 2), vec![expected_q]);
    mirror.check(&host);

    // Three more ticks: the fourth starts a round, which only P may be told.
    for _ in 0..2 {
        host.tick().expect("ticks");
        mirror.tick();
        pages(&mut host, &mut pc, p, 3);
        pages(&mut host, &mut qc, q, 3);
    }
    host.tick().expect("ticks");
    mirror.tick();
    let p_page = pages(&mut host, &mut pc, p, 4);
    let q_page = pages(&mut host, &mut qc, q, 4);
    assert_eq!(p_page.len(), 1);
    assert_eq!(p_page[0][0], DeltaOp::RoundStarted { round: 1 });
    assert!(!q_page[0].contains(&DeltaOp::RoundStarted { round: 1 }));
    // The `Turn` state op is its own grant, so Q still has the round there.
    assert!(q_page[0].contains(&DeltaOp::Turn {
        active: None,
        round: 1,
    }));
    mirror.check(&host);
}

// ---------------------------------------------------------------------------
// D7, D15.
// ---------------------------------------------------------------------------

/// Ticks `count` times, never draining.
fn tick_undrained(host: &mut Host, mirror: &mut Mirror, count: usize) {
    for _ in 0..count {
        host.tick().expect("ticks");
        mirror.tick();
    }
}

/// D7: a page that does not fit the log restages a snapshot instead of
/// blocking, dropping the page.
#[test]
fn d7_overflow_restages() {
    let mut host = host_of(&rt3_value());
    let mut mirror = Mirror::new(&rt3_value());
    let (p, mut client) = bind_p(&mut host, 100);
    tick_undrained(&mut host, &mut mirror, 128);
    let entries = host.take_delivery_entries(p).expect("bound");
    assert_eq!(entries.len(), 128);
    let ordinals: Vec<u64> = entries.iter().map(|entry| entry.ordinal).collect();
    assert_eq!(ordinals, (4..=131).collect::<Vec<u64>>());
    assert!(entries
        .iter()
        .all(|entry| entry.kind == DeliveryKind::Delta));
    let seqs: Vec<u64> = entries
        .iter()
        .map(|entry| codec_v3::decode_delta(&entry.bytes).expect("v3").event_seq)
        .collect();
    assert_eq!(seqs, (2..=129).collect::<Vec<u64>>());
    assert_eq!(
        host.transfer_state(p),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: None,
        }))
    );

    // Tick 129 does not fit: a snapshot replaces the whole log.
    host.tick().expect("the tick commits");
    mirror.tick();
    assert_eq!(
        host.transfer_state(p),
        Ok(Some(TransferState {
            next_transfer: 3,
            in_flight: Some(2),
        }))
    );
    let entries = host.take_delivery_entries(p).expect("bound");
    assert_eq!(entries.len(), 3);
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![
            (132, DeliveryKind::Transfer),
            (133, DeliveryKind::Transfer),
            (134, DeliveryKind::Delta),
        ]
    );
    assert!(matches!(decoded(&entries[0]), TransferDown::Begin(begin) if begin.transfer == tid(2)));
    assert!(matches!(decoded(&entries[1]), TransferDown::Chunk(_)));
    assert_eq!(
        codec_v3::decode_delta(&entries[2].bytes)
            .expect("v3")
            .event_seq,
        130
    );
    mirror.check(&host);

    // The client takes the replacement, publishes it and is exact.
    let drained = client.drain(&mut host, p, 100);
    assert_eq!(
        drained.outcomes[0],
        Outcome::Transfer(TransferOutcome::Begun)
    );
    assert!(drained.outcomes.iter().any(|outcome| matches!(
        outcome,
        Outcome::Transfer(TransferOutcome::Published { .. })
    )));
    assert_eq!(
        client.replica.encounter().clock,
        Some(Some(ClockFact {
            round_ticks: 4,
            elapsed: 129,
        }))
    );
    assert_eq!(
        client.replica.turn(),
        Some(&TurnFact {
            active: None,
            round: 32,
        })
    );
    assert_eq!(client.replica.next_event_seq(), 131);
    assert_eq!(
        client.publish(&mut host, p, 100),
        TransferDisposition::Published
    );

    // Control: with room, the next tick is an ordinary page, not a snapshot.
    host.tick().expect("ticks");
    mirror.tick();
    assert_eq!(
        pages(&mut host, &mut client, p, 100),
        vec![vec![rt_encounter(130)]]
    );
    assert_eq!(
        host.transfer_state(p),
        Ok(Some(TransferState {
            next_transfer: 3,
            in_flight: None,
        }))
    );
}

/// D15: a rejected v3 intent never blocks, whatever the egress room; its
/// receipt is dropped and recovered by the client's retry.
#[test]
fn d15_rejected_receipt_without_room() {
    let mut host = host_of(&rt3_value());
    let mut mirror = Mirror::new(&rt3_value());
    let (p, mut client) = bind_p(&mut host, 100);
    let epoch = host.epoch(p).expect("bound");
    tick_undrained(&mut host, &mut mirror, 128);
    let full = state(&mut host, &[p]);
    assert_eq!(full.peers[0].1.len(), 128);

    let bytes = intent_bytes(&host, p, 1, 1, host.server_tick(), IntentBody::EndTurn);
    assert_eq!(host.ingest(p, &bytes, 101), Ok(IngestDisposition::Staged));
    let summary = host.pump(101).expect("pump");
    assert_eq!(summary.backpressured, 0);
    assert_eq!(summary.admitted, 1);
    assert_eq!(summary.rejected, 1);
    assert_eq!(summary.dropped_replies, 0);
    assert_eq!(summary.results.len(), 1);
    assert_rejected(&summary.results[0], 1, CodeV1::IllegalAction);
    // No frame was added; the sequence and cache committed.
    let after = state(&mut host, &[p]);
    assert_eq!(after, full);
    assert_eq!(
        host.cached_status_v3(p, 1),
        Ok(Some(ReceiptStatus::Rejected(RejectionCode::WrongMode)))
    );
    assert_eq!(host.executions(), 0);

    // The client drains and acknowledges, then retries the same bytes.
    let drained = client.drain(&mut host, p, 102);
    assert_eq!(drained.deltas.len(), 128);
    let retry = pump_bytes(&mut host, p, &bytes, 102);
    assert_eq!(retry.seq, Some(1));
    assert_eq!(retry.status, StatusV1::Rejected(CodeV1::IllegalAction));
    assert!(retry.cached);
    assert!(!retry.retained_in_ingress);
    assert_eq!(
        pages(&mut host, &mut client, p, 102),
        vec![vec![receipt(
            epoch,
            1,
            128,
            ReceiptStatus::Rejected(RejectionCode::WrongMode)
        )]]
    );
    mirror.check(&host);

    // Control: an accepted intent with a full log is never blocked either: it
    // executes and the session is restaged.
    let mut host = host_of(&rt3_value());
    let mut mirror = Mirror::new(&rt3_value());
    let (p, _client) = bind_p(&mut host, 100);
    tick_undrained(&mut host, &mut mirror, 128);
    let result = send3(&mut host, p, 1, 1, declare(strike(), 2), 101);
    mirror.act(CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    });
    assert_applied(&result, 1);
    assert_eq!(host.executions(), 1);
    assert_eq!(
        host.transfer_state(p),
        Ok(Some(TransferState {
            next_transfer: 3,
            in_flight: Some(2),
        }))
    );
    mirror.check(&host);
}

// ---------------------------------------------------------------------------
// D9.
// ---------------------------------------------------------------------------

/// D9: a mode switch is a page with `ModeChanged` for a viewer with the mode
/// grant and without it for one without.
#[test]
fn d9_mode_changed_page() {
    let mut host = host_of(&tb3c_value());
    let mut mirror = Mirror::new(&tb3c_value());
    let (p, mut pc) = bind_p_all(&mut host, 0);
    let mut hidden_mode = facts();
    hidden_mode.encounter.mode = false;
    let (q, mut qc) = bind3(
        &mut host,
        &[],
        full_grants(&[a(), b(), c()]),
        hidden_mode,
        r0(),
        0,
    );

    host.request_mode(EncounterMode::RealTime)
        .expect("a request is accepted");
    mirror.mode(EncounterMode::RealTime);
    assert_eq!(
        pages(&mut host, &mut pc, p, 1),
        vec![vec![encounter(
            Some(NetMode::TurnBased),
            Some(Some(NetMode::RealTime)),
            0
        )]]
    );
    assert_eq!(
        pages(&mut host, &mut qc, q, 1),
        vec![vec![encounter(None, Some(Some(NetMode::RealTime)), 0)]]
    );

    for actor in [a(), b()] {
        let action = CombatAction::EndTurn { actor };
        assert!(host.host_action(&action).is_ok());
        mirror.act(action);
        pages(&mut host, &mut pc, p, 2);
        pages(&mut host, &mut qc, q, 2);
    }
    let action = CombatAction::EndTurn { actor: c() };
    assert!(host.host_action(&action).is_ok());
    mirror.act(action);
    assert_eq!(
        pages(&mut host, &mut pc, p, 3),
        vec![vec![
            DeltaOp::ModeChanged {
                mode: NetMode::RealTime,
                round: 1,
            },
            DeltaOp::RoundStarted { round: 1 },
            DeltaOp::Turn {
                active: None,
                round: 1,
            },
            rt_encounter(4),
            timeline(&[(4, 1), (4, 2), (4, 3)]),
        ]]
    );
    assert_eq!(
        pages(&mut host, &mut qc, q, 3),
        vec![vec![
            DeltaOp::RoundStarted { round: 1 },
            DeltaOp::Turn {
                active: None,
                round: 1,
            },
            encounter(None, Some(None), 4),
            timeline(&[(4, 1), (4, 2), (4, 3)]),
        ]]
    );
    mirror.check(&host);
    assert_eq!(host.executions(), 0);
}

// ---------------------------------------------------------------------------
// D10.
// ---------------------------------------------------------------------------

/// D10: an arm, a freeze, a resume, an expiry and a disarm each re-send the
/// timer; the countdown alone never does; a pause with no timer is `Clock`
/// alone.
#[test]
fn d10_timer_host_facts() {
    let mut host = host_of(&tb3_value());
    let mut mirror = Mirror::new(&tb3_value());
    let (p, mut client) = bind_p(&mut host, 100);

    assert_eq!(host.set_settings(settings(0, Some(1000)), 5000), Ok(()));
    assert_eq!(
        pages(&mut host, &mut client, p, 5000),
        vec![vec![host_facts(r1(), Some(0), Some(Some(1000)))]]
    );
    // The countdown alone re-sends nothing.
    let report = host.advance(5500).expect("advances");
    assert!(report.ticked);
    mirror.tick();
    assert_eq!(host.turn_timer_remaining_ms(), Some(500));
    assert_eq!(
        pages(&mut host, &mut client, p, 5500),
        Vec::<Vec<DeltaOp>>::new()
    );
    // A freeze.
    assert_eq!(host.pause(5600), Ok(()));
    assert_eq!(
        pages(&mut host, &mut client, p, 5600),
        vec![vec![
            clock_paused(PauseReason::Operator),
            host_facts(r1(), Some(0), Some(Some(400))),
        ]]
    );
    // A resume.
    assert_eq!(host.unpause(5700), Ok(()));
    assert_eq!(
        pages(&mut host, &mut client, p, 5700),
        vec![vec![
            clock_running(),
            host_facts(r1(), Some(0), Some(Some(400))),
        ]]
    );
    // The expiry ends the head's turn, and the re-armed timer is re-sent.
    let report = host.advance(6100).expect("advances");
    assert_eq!(report.timer_ended_turn, Some(a()));
    mirror.act(CombatAction::EndTurn { actor: a() });
    mirror.tick();
    assert_eq!(
        pages(&mut host, &mut client, p, 6100),
        vec![vec![
            DeltaOp::TurnStarted {
                actor: n(2),
                round: 0,
            },
            DeltaOp::Turn {
                active: Some(n(2)),
                round: 0,
            },
            timeline(&[(2, 2), (3, 3)]),
            host_facts(r1(), Some(0), Some(Some(1000))),
        ]]
    );
    // A disarm.
    assert_eq!(host.set_settings(settings(0, None), 6200), Ok(()));
    assert_eq!(
        pages(&mut host, &mut client, p, 6200),
        vec![vec![host_facts(r1(), Some(0), Some(None))]]
    );
    // A pause with no timer armed sends `Clock` only (finding 7).
    assert_eq!(host.pause(6300), Ok(()));
    assert_eq!(
        pages(&mut host, &mut client, p, 6300),
        vec![vec![clock_paused(PauseReason::Operator)]]
    );
    mirror.check(&host);
    assert_eq!(
        client.replica.pause(),
        Some(PauseState::Paused {
            reason: Some(PauseReason::Operator),
        })
    );
}

// ---------------------------------------------------------------------------
// D11.
// ---------------------------------------------------------------------------

/// D11: the intent precedence at every adjacent pair, each beside an accepted
/// control.
#[test]
fn d11_intent_precedence() {
    let mut host = host_of(&rt3_value());
    let (p, mut client) = bind3(
        &mut host,
        &[a()],
        full_grants(&[a(), b(), c()]),
        facts(),
        r0(),
        0,
    );
    let epoch = host.epoch(p).expect("bound");

    // Control (actor) before the right.
    let result = send3(&mut host, p, 1, 2, IntentBody::Pause, 1);
    assert_rejected(&result, 1, CodeV1::NotAuthorized);
    assert_eq!(
        host.cached_status_v3(p, 1),
        Ok(Some(ReceiptStatus::Rejected(RejectionCode::NotAuthorized)))
    );
    // The right, with the actor controlled.
    let result = send3(&mut host, p, 2, 1, IntentBody::Pause, 2);
    assert_rejected(&result, 2, CodeV1::NotAuthorized);
    assert_eq!(
        host.cached_status_v3(p, 2),
        Ok(Some(ReceiptStatus::Rejected(RejectionCode::NoPauseRight)))
    );
    // An unmapped target is `NotAuthorized` even for a controlled actor.
    let result = send3(&mut host, p, 3, 1, declare(strike(), 9), 3);
    assert_rejected(&result, 3, CodeV1::NotAuthorized);
    assert_eq!(
        host.cached_status_v3(p, 3),
        Ok(Some(ReceiptStatus::Rejected(RejectionCode::NotAuthorized)))
    );
    // Freshness before resolution: a future `observed_tick` with no right.
    let bytes = intent_bytes(&host, p, 4, 1, 1, IntentBody::Pause);
    let result = pump_bytes(&mut host, p, &bytes, 4);
    assert_rejected(&result, 4, CodeV1::FutureTick);
    assert_eq!(
        host.cached_status_v3(p, 4),
        Ok(Some(ReceiptStatus::Rejected(RejectionCode::FutureTick)))
    );
    // The sequence before everything after it: a gap with an unmapped actor.
    let bytes = intent_bytes(&host, p, 9, 77, 0, IntentBody::Pause);
    let result = pump_bytes(&mut host, p, &bytes, 5);
    assert_eq!(result.seq, Some(9));
    assert_eq!(result.status, StatusV1::Rejected(CodeV1::SeqGap));
    assert_eq!(host.cached_status_v3(p, 9), Ok(None));
    assert_eq!(host.executions(), 0);
    assert_eq!(host.paused(), None);
    let drained = pages(&mut host, &mut client, p, 5);
    assert_eq!(
        drained,
        vec![
            vec![rejected(epoch, 1, RejectionCode::NotAuthorized)],
            vec![rejected(epoch, 2, RejectionCode::NoPauseRight)],
            vec![rejected(epoch, 3, RejectionCode::NotAuthorized)],
            vec![rejected(epoch, 4, RejectionCode::FutureTick)],
        ]
    );

    // Control: with the right the same `Pause` is accepted.
    assert_eq!(host.set_pause_rights(p, r1()), Ok(()));
    pages(&mut host, &mut client, p, 6);
    let result = send3(&mut host, p, 5, 1, IntentBody::Pause, 6);
    assert_applied(&result, 5);
    assert_eq!(host.paused(), Some(PauseReason::Player));
    assert_eq!(host.executions(), 1);
}

// ---------------------------------------------------------------------------
// D12.
// ---------------------------------------------------------------------------

/// D12: an auto-pause is in the page of the tick that caused it, with no later
/// publication.
#[test]
fn d12_auto_pause_reaches_the_page() {
    let mut host = host_of(&rt3_value());
    let mut mirror = Mirror::new(&rt3_value());
    assert_eq!(host.set_settings(settings(0x02, None), 100), Ok(()));
    let (p, mut client) = bind_p(&mut host, 100);

    for (step, elapsed) in (1..=3u64).zip(1..=3u64) {
        let report = host.advance(100 + step).expect("advances");
        assert_eq!(report.paused, None);
        mirror.tick();
        assert_eq!(
            pages(&mut host, &mut client, p, 100 + step),
            vec![vec![rt_encounter(elapsed)]]
        );
    }
    let report = host.advance(104).expect("advances");
    mirror.tick();
    assert!(report.ticked);
    assert_eq!(report.timer_ended_turn, None);
    assert_eq!(
        report.paused,
        Some(PauseReason::Auto(AutoPauseTrigger::RoundStarted))
    );
    assert_eq!(
        pages(&mut host, &mut client, p, 104),
        vec![vec![
            DeltaOp::RoundStarted { round: 1 },
            DeltaOp::Turn {
                active: None,
                round: 1,
            },
            rt_encounter(4),
            clock_paused(PauseReason::Auto(AutoPauseTrigger::RoundStarted)),
        ]]
    );
    // The next advance does not tick and logs nothing.
    let before = state(&mut host, &[p]);
    let report = host.advance(105).expect("advances");
    assert!(!report.ticked);
    assert_eq!(state(&mut host, &[p]), before);
    mirror.check(&host);
}

// ---------------------------------------------------------------------------
// D-1 (Blocked branch) and D-6 (failure budget).
// ---------------------------------------------------------------------------

/// D-1, D-5: an accepted v3 intent still reserves capture room, and the head
/// that stays staged is decoded with the session's own codec.
#[test]
fn d_capture_full_blocks_an_accepted_v3_intent_and_decodes_its_seq() {
    let mut host = host_of(&rt3_value());
    let mirror = Mirror::new(&rt3_value());
    let (p, _client) = bind_p(&mut host, 0);
    for seq in 1..=4096u64 {
        let result = send3(&mut host, p, seq, 1, IntentBody::Pause, 100 * seq);
        assert_applied(&result, seq);
        let entries = host.take_delivery_entries(p).expect("bound");
        let last = entries.last().expect("a receipt frame").ordinal;
        host.acknowledge_delivery(p, last).expect("acks");
    }
    assert_eq!(host.last_capture(), 4096);
    assert_eq!(host.executions(), 4096);
    assert_eq!(host.paused(), Some(PauseReason::Player));

    let now = 100 * 4097;
    let bytes = intent_bytes(&host, p, 4097, 1, 0, IntentBody::Unpause);
    let before = state(&mut host, &[p]);
    assert_eq!(host.ingest(p, &bytes, now), Ok(IngestDisposition::Staged));
    let summary = host.pump(now).expect("pump");
    assert_eq!(summary.backpressured, 1);
    assert_eq!(summary.admitted, 0);
    assert_eq!(
        summary.results,
        vec![AdmissionResult {
            peer: p,
            seq: Some(4097),
            status: StatusV1::Rejected(CodeV1::QueueFull),
            cached: false,
            retained_in_ingress: true,
        }]
    );
    assert_eq!(state(&mut host, &[p]), before);
    assert_eq!(host.cached_status_v3(p, 4097), Ok(None));

    // Control: with room the same head is admitted and applied.
    host.acknowledge_captures(1).expect("acknowledges");
    let summary = host.pump(now + 1).expect("pump");
    assert_eq!(summary.backpressured, 0);
    assert_eq!(summary.applied, 1);
    assert_eq!(host.paused(), None);
    assert_eq!(host.executions(), 4097);
    mirror.check(&host);
}

/// D-6: a rejected v3 receipt spends the failure budgets like any other; one
/// the budget refuses is dropped, counted, and recovered by the retry.
#[test]
fn d_rejected_v3_receipts_obey_the_failure_budget() {
    let mut host = host_of(&rt3_value());
    let mirror = Mirror::new(&rt3_value());
    let (p, mut client) = bind_p(&mut host, 0);
    let epoch = host.epoch(p).expect("bound");
    // Twelve rejected `EndTurn`s in one instant: the burst is ten.
    for seq in 1..=12u64 {
        let bytes = intent_bytes(&host, p, seq, 1, 0, IntentBody::EndTurn);
        assert_eq!(host.ingest(p, &bytes, 10), Ok(IngestDisposition::Staged));
    }
    let summary = host.pump(10).expect("pump");
    assert_eq!(summary.rejected, 12);
    assert_eq!(summary.backpressured, 0);
    assert_eq!(summary.dropped_replies, 2);
    let drained = pages(&mut host, &mut client, p, 10);
    let expected: Vec<Vec<DeltaOp>> = (1..=10)
        .map(|seq| vec![rejected(epoch, seq, RejectionCode::WrongMode)])
        .collect();
    assert_eq!(drained, expected);
    // The dropped replies were still committed to the cache.
    for seq in 11..=12 {
        assert_eq!(
            host.cached_status_v3(p, seq),
            Ok(Some(ReceiptStatus::Rejected(RejectionCode::WrongMode)))
        );
    }
    // The retry recovers one once the budget has refilled.
    let bytes = intent_bytes(&host, p, 11, 1, 0, IntentBody::EndTurn);
    let retry = pump_bytes(&mut host, p, &bytes, 20_000);
    assert!(retry.cached);
    assert_eq!(
        pages(&mut host, &mut client, p, 20_000),
        vec![vec![rejected(epoch, 11, RejectionCode::WrongMode)]]
    );
    mirror.check(&host);
}
