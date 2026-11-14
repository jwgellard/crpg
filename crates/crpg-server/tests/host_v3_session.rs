//! T141c acceptance suite (`tasks/T141.md`, tests C1 to C8 and C10; C9 is the
//! pacing unit test in `src/host.rs`, with the staging-bound cases beside it):
//! v3 sessions and the `0x82` snapshot sender, public API only.
//!
//! The client is the D54 client: a `ReassemblyPoolV3` gate feeding a
//! `ReplicaWorldV3`, never the v1/v2 receiver. Oracles are independent of the
//! host: a test-owned mirror `HistoryWorld` loaded from the same JSON, and
//! hand-written expected rows, frames and error values. The fixtures are
//! `tests/fixtures/three_combatants.json` as saved-game JSON, edited in
//! memory:
//!
//! - `TB3`: the file as is (A, B, C act in keys 1, 2, 3; A first).
//! - `TB3c`: `TB3` at history version 2 with a round clock (real-time capable
//!   while turn-based).
//! - `RT3`: `TB3c` in real-time mode, no active head, every timeline key 0.
//!
//! Every call after a binding uses a `now_ms` at or above that binding's. Each
//! refusal is compared against a `State` taken just before the call and sits
//! beside an accepted control. Test rule for T141c/T141d: `publish` follows
//! the first `drain` of every v3 session, and no op list later than 99 ticks
//! after a staging is pinned.

use crpg_core::{EntityId, Ulid};
use crpg_net::codec_transfer;
use crpg_net::codec_v2;
use crpg_net::facts_v3::{EncounterGrants, HostGrants};
use crpg_net::pool_v3::{
    DeltaOutcome, GateMode, ReassemblyPoolV3, ReceiverIdV3, ReplicaUpdateV3, TransferLimitsV3,
    TransferOutcome,
};
use crpg_net::protocol::{IntentBody, IntentFrame, NetId, LANE_COMBAT};
use crpg_net::protocol_transfer::{
    CancelReason, HealthFact, ResyncReason, TransferDown, TransferId, TransferUp, TurnFact,
};
use crpg_net::protocol_v3::{
    AutoPauseToggles, DeltaFrame, DeltaOp, EncounterFacts, EncounterMode as NetMode, FactSet,
    HostFacts, OrderFact, PauseRights, PoolFact, SessionEpoch, StatFact, StatValue, TimelineEntry,
};
use crpg_net::replica_v3::{PauseState, ReplicaEntityV3, ReplicaWorldV3};
use crpg_net::transfer_v3;
use crpg_server::host::{
    ControlGrant, DeliveryEntry, DeliveryKind, DisclosureGrants, EntityDisclosure, EntityFactGrant,
    FactGrants, GrantUpdate, Host, HostConfig, HostError, IngestDisposition, PeerHandle,
    ProtocolSelection, TransferDisposition, TransferRefusal, TransferState, DEFAULT_HOST_GRANTS,
    HOST_UNBOUND_AREA,
};
use crpg_sim::{history_hash, CombatAction, EncounterMode, HistoryWorld};
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
fn whiff() -> Ulid {
    Ulid::from_u128(605)
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

fn bind_p(host: &mut Host, now: u64) -> PeerHandle {
    host.bind_v3_peer(
        control(&[a()]),
        full_grants(&[a(), b(), c()]),
        facts(),
        r1(),
        now,
    )
    .expect("binds")
}

/// A host's authority equals a mirror driven with the same (here: no) sim
/// calls.
fn assert_authority(host: &Host, oracle: &HistoryWorld) {
    assert_eq!(host.authority_hash(), history_hash(oracle));
}

fn health(health: u32, max_health: u32, dead: bool) -> HealthFact {
    HealthFact {
        health,
        max_health,
        dead,
    }
}

/// The fact set of one fixture row under `FACTS`.
fn row_facts(order: Option<Option<OrderFact>>) -> FactSet {
    FactSet {
        pools: Some(vec![PoolFact {
            pool: pool_jy(),
            current: 1,
            max: 1,
        }]),
        effects: Some(vec![]),
        order,
        stats: Some(vec![StatFact {
            name: "alpha".to_owned(),
            value: StatValue::Int(8),
        }]),
        tags: Some(vec![]),
    }
}

fn row(id: u64, health: HealthFact, facts: FactSet) -> ReplicaEntityV3 {
    ReplicaEntityV3 {
        net_id: n(id),
        health: Some(health),
        facts,
    }
}

// ---------------------------------------------------------------------------
// The v3 client (D54).
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
        Drained {
            entries,
            outcomes,
            deltas,
        }
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

fn resync_bytes(epoch: [u8; 16], after: Option<u64>) -> Vec<u8> {
    transfer_v3::encode_up(&TransferUp::Resync {
        epoch,
        after: after.map(tid),
        reason: ResyncReason::Expired,
    })
    .expect("encodes")
}

/// `Resync` up, then a fresh client's drain of what the host staged.
fn resync_and_drain(
    host: &mut Host,
    peer: PeerHandle,
    after: Option<u64>,
    now: u64,
) -> (Client, Drained) {
    let epoch = host.epoch(peer).expect("bound");
    assert_eq!(
        host.ingest_transfer(peer, &resync_bytes(epoch, after), now),
        Ok(TransferDisposition::Resynced)
    );
    let mut client = Client::new(epoch);
    let drained = client.drain(host, peer, now);
    (client, drained)
}

fn decoded(entry: &DeliveryEntry) -> TransferDown {
    transfer_v3::decode_down(&entry.bytes).expect("a 0x82 frame")
}

fn is_begin_of(entry: &DeliveryEntry, transfer: u64) -> bool {
    matches!(decoded(entry), TransferDown::Begin(begin) if begin.transfer == tid(transfer))
}

fn is_chunk(entry: &DeliveryEntry) -> bool {
    matches!(decoded(entry), TransferDown::Chunk(_))
}

fn is_cancel(entry: &DeliveryEntry, epoch: [u8; 16], transfer: u64, why: CancelReason) -> bool {
    decoded(entry)
        == TransferDown::Cancel {
            epoch,
            transfer: tid(transfer),
            reason: why,
        }
}

/// The ordinal and kind of each entry.
fn shape(entries: &[DeliveryEntry]) -> Vec<(u64, DeliveryKind)> {
    entries
        .iter()
        .map(|entry| (entry.ordinal, entry.kind))
        .collect()
}

/// Everything a refusal must leave alone.
#[derive(Debug, PartialEq, Eq)]
struct State {
    authority: [u8; 32],
    last_capture: u64,
    capture_acknowledged: u64,
    executions: u64,
    paused: Option<crpg_net::protocol_v3::PauseReason>,
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

/// One v2 intent through `ingest` then `pump`.
fn send(
    host: &mut Host,
    peer: PeerHandle,
    seq: u64,
    actor: u64,
    body: IntentBody,
    now: u64,
) -> crpg_server::host::AdmissionResult {
    let frame = IntentFrame {
        epoch: host.epoch(peer).expect("bound"),
        lane: LANE_COMBAT,
        seq,
        observed_tick: host.server_tick(),
        actor: n(actor),
        body,
    };
    let bytes = codec_v2::encode_intent(&frame).expect("encodes");
    assert_eq!(
        host.ingest(peer, &bytes, now),
        Ok(IngestDisposition::Staged)
    );
    let summary = host.pump(now).expect("pump");
    assert_eq!(summary.results.len(), 1, "{summary:?}");
    summary.results[0].clone()
}

// ---------------------------------------------------------------------------
// C1 to C8.
// ---------------------------------------------------------------------------

/// C1: a v3 binding's first delivery is a `0x82` snapshot at cursor 0 plus one
/// `[Clock, HostFacts]` delta; every fact reads back exactly.
#[test]
fn c1_bind_v3_first_delivery() {
    let mut host = host_of(&tb3_value());
    let oracle = history_of(&tb3_value());
    let p = bind_p(&mut host, 100);
    assert_eq!(p.get(), 1);
    assert_eq!(host.session_wire(p), Ok(3));
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
    // The Begin: transfer 1 at cursor 0, one chunk of eight segments.
    let TransferDown::Begin(begin) = decoded(&drained.entries[0]) else {
        panic!("the first entry is a Begin");
    };
    assert_eq!(begin.epoch, epoch);
    assert_eq!(begin.transfer, tid(1));
    assert_eq!(begin.cursor, 0);
    assert_eq!(begin.base_tick, 0);
    assert_eq!(begin.entity_count, 3);
    assert_eq!(begin.segments_per_chunk, 8);
    assert_eq!(begin.chunk_count, 1);
    assert!(is_chunk(&drained.entries[1]));
    // The delta: event_seq 1, exactly Clock then HostFacts.
    assert_eq!(
        drained.deltas,
        vec![DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 0,
            event_seq: 1,
            ops: vec![
                DeltaOp::Clock { paused: None },
                DeltaOp::HostFacts(HostFacts {
                    rights: r1(),
                    auto_pause: Some(toggles(0)),
                    turn_timer_ms: Some(None),
                }),
            ],
        }]
    );
    let replica = &client.replica;
    assert_eq!(
        replica.entities(),
        &[
            row(1, health(10, 10, false), row_facts(Some(None))),
            row(2, health(10, 10, false), row_facts(None)),
            row(3, health(3, 3, false), row_facts(None)),
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
        replica.encounter(),
        &EncounterFacts {
            mode: Some(NetMode::TurnBased),
            pending_mode: Some(None),
            clock: Some(None),
        }
    );
    assert_eq!(
        replica.timeline(),
        Some(
            &[
                TimelineEntry {
                    key: 1,
                    entity: n(1)
                },
                TimelineEntry {
                    key: 2,
                    entity: n(2)
                },
                TimelineEntry {
                    key: 3,
                    entity: n(3)
                },
            ][..]
        )
    );
    assert_eq!(replica.pause(), Some(PauseState::Running));
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
    // Publication retires the transfer.
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
    assert_authority(&host, &oracle);
}

/// C2: nothing ungranted appears; a presence-only grant has no health.
#[test]
fn c2_bind_withheld() {
    let mut host = host_of(&tb3_value());
    let oracle = history_of(&tb3_value());
    let only = |health: bool| DisclosureGrants {
        entities: vec![EntityDisclosure {
            entity: a(),
            present: true,
            health,
            spawn: false,
            despawn: false,
            died: false,
            action_actor: false,
            action_target: false,
            action_outcome: false,
            action_damage: false,
            turn_start: false,
        }],
        abilities: vec![],
        encounters: vec![],
        turn_round: false,
        encounter_end: false,
        turn_state: false,
    };
    let p = host
        .bind_v3_peer(control(&[]), only(true), FactGrants::default(), r0(), 0)
        .expect("binds");
    let mut client = Client::new(host.epoch(p).expect("bound"));
    client.drain(&mut host, p, 0);
    let replica = &client.replica;
    assert_eq!(
        replica.entities(),
        &[ReplicaEntityV3 {
            net_id: n(1),
            health: Some(health(10, 10, false)),
            facts: FactSet::default(),
        }]
    );
    assert_eq!(replica.turn(), None);
    assert_eq!(replica.encounter(), &EncounterFacts::default());
    assert_eq!(replica.timeline(), None);
    assert_eq!(replica.pause(), Some(PauseState::Running));
    assert_eq!(
        replica.host_facts(),
        Some(&HostFacts {
            rights: r0(),
            auto_pause: None,
            turn_timer_ms: None,
        })
    );
    // The health grant alone decides the health fact.
    let q = host
        .bind_v3_peer(control(&[]), only(false), FactGrants::default(), r0(), 0)
        .expect("binds");
    let mut quiet = Client::new(host.epoch(q).expect("bound"));
    quiet.drain(&mut host, q, 0);
    assert_eq!(
        quiet.replica.entities(),
        &[ReplicaEntityV3 {
            net_id: n(1),
            health: None,
            facts: FactSet::default(),
        }]
    );
    assert_authority(&host, &oracle);
}

/// C2 (order branch): a standing order is a fact only for the controlling
/// session, and its target is named by that session's id.
#[test]
fn c2_order_fact_follows_control() {
    let mut host = host_of(&rt3_value());
    let mut oracle = history_of(&rt3_value());
    let v = host
        .bind_peer(control(&[a(), b(), c()]), full_grants(&[a(), b(), c()]), 0)
        .expect("binds");
    let first = host.take_delivery_entries(v).expect("bound");
    host.acknowledge_delivery(v, first[0].ordinal)
        .expect("acks");
    let result = send(
        &mut host,
        v,
        1,
        1,
        IntentBody::DeclareAction {
            ability: strike(),
            target: n(2),
        },
        10,
    );
    assert_eq!(result.status, crpg_net::protocol::ReceiptStatus::Applied);
    oracle
        .perform_action(&CombatAction::UseAbility {
            actor: a(),
            ability: strike(),
            target: b(),
        })
        .expect("the mirror accepts the order");
    oracle
        .acknowledge(oracle.last_sequence())
        .expect("acknowledges");
    assert_authority(&host, &oracle);

    let order = OrderFact {
        ability: strike(),
        target: n(2),
        repeat: false,
    };
    let p = host
        .bind_v3_peer(
            control(&[a()]),
            full_grants(&[a(), b(), c()]),
            facts(),
            r1(),
            20,
        )
        .expect("binds");
    let mut client = Client::new(host.epoch(p).expect("bound"));
    client.drain(&mut host, p, 20);
    let rows = client.replica.entities();
    assert_eq!(rows[0].facts.order, Some(Some(order)));
    assert_eq!(rows[1].facts.order, None);
    assert_eq!(rows[2].facts.order, None);
    // B's own order (none) is invisible to P, and a peer that controls B
    // sees `Some(None)` there.
    let q = host
        .bind_v3_peer(
            control(&[b()]),
            full_grants(&[a(), b(), c()]),
            facts(),
            r1(),
            20,
        )
        .expect("binds");
    let mut other = Client::new(host.epoch(q).expect("bound"));
    other.drain(&mut host, q, 20);
    let rows = other.replica.entities();
    assert_eq!(rows[0].facts.order, None);
    assert_eq!(rows[1].facts.order, Some(None));
}

/// C3: the read API under v3 ids.
#[test]
fn c3_read_api() {
    let mut host = host_of(&tb3_value());
    let oracle = history_of(&tb3_value());
    let v = host
        .bind_peer(control(&[a()]), full_grants(&[a(), b(), c()]), 100)
        .expect("binds");
    let p = bind_p(&mut host, 100);
    let from_v2 = host.legal_intents(v).expect("reads");
    assert!(!from_v2.is_empty());
    assert_eq!(host.legal_intents(p), Ok(from_v2));
    assert_eq!(host.replica_id(p, a()), Ok(Some(n(1))));
    assert_eq!(host.replica_id(p, c()), Ok(Some(n(3))));
    assert_eq!(host.replica_id(p, eid(99)), Ok(None));
    // A peer that does not control the head has no options, v2 or v3.
    let q = host
        .bind_v3_peer(
            control(&[b()]),
            full_grants(&[a(), b(), c()]),
            facts(),
            r1(),
            100,
        )
        .expect("binds");
    assert_eq!(host.legal_intents(q), Ok(Vec::new()));
    assert_authority(&host, &oracle);

    let mut rt_host = host_of(&rt3_value());
    let rt = bind_p(&mut rt_host, 0);
    assert_eq!(rt_host.legal_intents(rt), Ok(Vec::new()));
}

/// C4: narrowing a v3 session stages a fresh `0x82` snapshot; widening is
/// quiet; ids are never reused.
#[test]
fn c4_narrowing_restages() {
    let mut host = host_of(&tb3_value());
    let oracle = history_of(&tb3_value());
    let p = bind_p(&mut host, 100);
    let epoch = host.epoch(p).expect("bound");
    // Before publish: entries 1 to 3 are unacknowledged.
    assert_eq!(
        host.set_grants(p, full_grants(&[a(), b()])),
        Ok(GrantUpdate::SnapshotStaged)
    );
    assert_eq!(
        host.transfer_state(p),
        Ok(Some(TransferState {
            next_transfer: 3,
            in_flight: Some(2),
        }))
    );
    let mut client = Client::new(epoch);
    let drained = client.drain(&mut host, p, 100);
    assert_eq!(
        shape(&drained.entries),
        vec![
            (4, DeliveryKind::Transfer),
            (5, DeliveryKind::Transfer),
            (6, DeliveryKind::Transfer),
            (7, DeliveryKind::Delta),
        ]
    );
    assert!(is_cancel(
        &drained.entries[0],
        epoch,
        1,
        CancelReason::Aborted
    ));
    assert!(is_begin_of(&drained.entries[1], 2));
    assert!(is_chunk(&drained.entries[2]));
    assert_eq!(drained.deltas.len(), 1);
    assert_eq!(drained.deltas[0].event_seq, 2);
    let ids: Vec<NetId> = client
        .replica
        .entities()
        .iter()
        .map(|entity| entity.net_id)
        .collect();
    assert_eq!(ids, vec![n(1), n(2)]);

    // Widening back is quiet and pins no entry (T141d adds a page).
    assert_eq!(
        host.set_grants(p, full_grants(&[a(), b(), c()])),
        Ok(GrantUpdate::Unchanged)
    );
    // The next snapshot reads the widened grants; C gets a new id, never 3.
    let (again, redo) = resync_and_drain(&mut host, p, Some(2), 101);
    assert!(is_cancel(
        &redo.entries[0],
        epoch,
        2,
        CancelReason::Superseded
    ));
    let ids: Vec<NetId> = again
        .replica
        .entities()
        .iter()
        .map(|entity| entity.net_id)
        .collect();
    assert_eq!(ids, vec![n(1), n(2), n(4)]);

    // The same narrowing after publish has no Cancel.
    let mut host = host_of(&tb3_value());
    let p = bind_p(&mut host, 100);
    let mut client = Client::new(host.epoch(p).expect("bound"));
    client.drain(&mut host, p, 100);
    assert_eq!(
        client.publish(&mut host, p, 100),
        TransferDisposition::Published
    );
    assert_eq!(
        host.set_grants(p, full_grants(&[a(), b()])),
        Ok(GrantUpdate::SnapshotStaged)
    );
    let drained = client.drain(&mut host, p, 100);
    assert_eq!(
        shape(&drained.entries),
        vec![
            (4, DeliveryKind::Transfer),
            (5, DeliveryKind::Transfer),
            (6, DeliveryKind::Delta),
        ]
    );
    assert!(is_begin_of(&drained.entries[0], 2));
    assert_eq!(drained.deltas[0].event_seq, 2);
    assert_authority(&host, &oracle);
}

/// C5: `ingest_transfer` on a v3 session speaks `0x82` only.
#[test]
fn c5_transfer_up() {
    let mut host = host_of(&tb3_value());
    let oracle = history_of(&tb3_value());
    let p = bind_p(&mut host, 100);
    let epoch = host.epoch(p).expect("bound");
    let mut client = Client::new(epoch);
    client.drain(&mut host, p, 100);
    let published = |epoch: [u8; 16], transfer: u64| {
        transfer_v3::encode_up(&TransferUp::Published {
            epoch,
            transfer: tid(transfer),
        })
        .expect("encodes")
    };
    // A 0x81 frame is malformed here.
    let v1_frame = codec_transfer::encode_up(&TransferUp::Published {
        epoch,
        transfer: tid(1),
    })
    .expect("encodes");
    let before = state(&mut host, &[p]);
    assert_eq!(
        host.ingest_transfer(p, &v1_frame, 101),
        Ok(TransferDisposition::Refused(TransferRefusal::Malformed))
    );
    assert_eq!(
        host.ingest_transfer(p, &published([9; 16], 1), 101),
        Ok(TransferDisposition::Refused(TransferRefusal::WrongEpoch))
    );
    assert_eq!(
        host.ingest_transfer(p, &published(epoch, 5), 101),
        Ok(TransferDisposition::Refused(
            TransferRefusal::UnknownTransfer
        ))
    );
    assert_eq!(state(&mut host, &[p]), before);
    // Control: the same frame in 0x82 publishes; again is stale.
    assert_eq!(
        host.ingest_transfer(p, &published(epoch, 1), 101),
        Ok(TransferDisposition::Published)
    );
    assert_eq!(
        host.ingest_transfer(p, &published(epoch, 1), 101),
        Ok(TransferDisposition::Stale)
    );
    // Resync after the published transfer: no Cancel.
    let (_, drained) = resync_and_drain(&mut host, p, Some(1), 102);
    assert_eq!(
        shape(&drained.entries),
        vec![
            (4, DeliveryKind::Transfer),
            (5, DeliveryKind::Transfer),
            (6, DeliveryKind::Delta),
        ]
    );
    assert!(is_begin_of(&drained.entries[0], 2));
    assert!(is_chunk(&drained.entries[1]));
    // An unissued `after` is refused and changes nothing.
    let before = state(&mut host, &[p]);
    assert_eq!(
        host.ingest_transfer(p, &resync_bytes(epoch, Some(9)), 103),
        Ok(TransferDisposition::Refused(
            TransferRefusal::UnknownTransfer
        ))
    );
    assert_eq!(state(&mut host, &[p]), before);
    // A second Resync while transfer 2 is in flight cancels it first.
    let (_, drained) = resync_and_drain(&mut host, p, Some(2), 104);
    assert!(is_cancel(
        &drained.entries[0],
        epoch,
        2,
        CancelReason::Superseded
    ));
    assert!(is_begin_of(&drained.entries[1], 3));
    assert_eq!(drained.entries.len(), 4);
    assert_authority(&host, &oracle);
}

/// C6: errors and bounds.
#[test]
fn c6_errors_and_bounds() {
    let mut host = host_of(&tb3_value());
    let oracle = history_of(&tb3_value());
    let v = host
        .bind_peer(control(&[a()]), full_grants(&[a(), b(), c()]), 100)
        .expect("binds");
    // v3-only calls on a v2 session.
    assert_eq!(host.set_fact_grants(v, facts()), Err(HostError::NotV3Peer));
    assert_eq!(host.set_pause_rights(v, r1()), Err(HostError::NotV3Peer));
    assert_eq!(host.pause_rights(v), Err(HostError::NotV3Peer));
    assert_eq!(host.session_wire(v), Ok(2));
    // An unknown handle.
    let p = bind_p(&mut host, 100);
    let gone = bind_p(&mut host, 100);
    host.unbind_peer(gone);
    assert_eq!(
        host.set_fact_grants(gone, facts()),
        Err(HostError::UnknownPeer)
    );
    assert_eq!(
        host.set_pause_rights(gone, r1()),
        Err(HostError::UnknownPeer)
    );
    assert_eq!(host.pause_rights(gone), Err(HostError::UnknownPeer));
    assert_eq!(host.session_wire(gone), Err(HostError::UnknownPeer));
    assert_eq!(
        host.take_delivery_entries(gone),
        Err(HostError::UnknownPeer)
    );
    // Controls: the v3 session answers.
    assert_eq!(host.session_wire(p), Ok(3));
    assert_eq!(host.pause_rights(p), Ok(r1()));
    assert_eq!(host.set_pause_rights(p, r0()), Ok(()));
    assert_eq!(host.pause_rights(p), Ok(r0()));
    assert_eq!(host.set_fact_grants(p, FactGrants::default()), Ok(()));

    // Fact-grant bounds on bind: 1,025 entries and a duplicate are refused;
    // 1,024 distinct entities bind.
    let mut fresh = host_of(&tb3_value());
    let entries = |count: u32| -> Vec<EntityFactGrant> {
        (0..count).map(|index| all_fact_flags(eid(index))).collect()
    };
    let with = |entities: Vec<EntityFactGrant>| FactGrants {
        entities,
        ..FactGrants::default()
    };
    let bind = |host: &mut Host, grants: FactGrants| {
        host.bind_v3_peer(control(&[a()]), full_grants(&[a()]), grants, r1(), 5)
    };
    assert_eq!(
        bind(&mut fresh, with(entries(1025))),
        Err(HostError::InvalidGrants)
    );
    assert_eq!(
        bind(
            &mut fresh,
            with(vec![all_fact_flags(a()), all_fact_flags(a())])
        ),
        Err(HostError::InvalidGrants)
    );
    let wide = bind(&mut fresh, with(entries(1024))).expect("1,024 distinct entities bind");
    assert_eq!(wide.get(), 1, "the refused bindings consumed no handle");
    // The same bounds on set_fact_grants and set_grants.
    let before = state(&mut fresh, &[wide]);
    assert_eq!(
        fresh.set_fact_grants(wide, with(entries(1025))),
        Err(HostError::InvalidGrants)
    );
    assert_eq!(
        fresh.set_fact_grants(wide, with(vec![all_fact_flags(b()), all_fact_flags(b())])),
        Err(HostError::InvalidGrants)
    );
    let mut duplicate = full_grants(&[a(), a()]);
    assert_eq!(
        fresh.set_grants(wide, duplicate.clone()),
        Err(HostError::InvalidGrants)
    );
    assert_eq!(state(&mut fresh, &[wide]), before);
    assert_eq!(fresh.set_fact_grants(wide, with(entries(1024))), Ok(()));
    duplicate.entities.pop();
    assert_eq!(
        fresh.set_grants(wide, duplicate),
        Ok(GrantUpdate::Unchanged)
    );

    // The ninth binding is refused; the eighth was fine.
    let mut full_house = host_of(&tb3_value());
    let mut peers = Vec::new();
    for _ in 0..8 {
        peers.push(bind_p(&mut full_house, 7));
    }
    let before = state(&mut full_house, &peers);
    assert_eq!(
        full_house.bind_v3_peer(control(&[a()]), full_grants(&[a()]), facts(), r1(), 7),
        Err(HostError::ServerBusy)
    );
    assert_eq!(state(&mut full_house, &peers), before);
    full_house.unbind_peer(peers[0]);
    assert!(full_house
        .bind_v3_peer(control(&[a()]), full_grants(&[a()]), facts(), r1(), 7)
        .is_ok());

    // After shutdown: Closed for the setters, UnknownPeer for the readers.
    host.shutdown();
    assert_eq!(host.set_fact_grants(p, facts()), Err(HostError::Closed));
    assert_eq!(host.set_pause_rights(p, r1()), Err(HostError::Closed));
    assert_eq!(host.pause_rights(p), Err(HostError::UnknownPeer));
    assert_eq!(host.session_wire(p), Err(HostError::UnknownPeer));
    assert_eq!(
        host.bind_v3_peer(control(&[a()]), full_grants(&[a()]), facts(), r1(), 200),
        Err(HostError::Closed)
    );
    assert_authority(&host, &oracle);
}

/// C7: real-time encounter facts under full grants.
#[test]
fn c7_rt_encounter_facts() {
    let mut host = host_of(&rt3_value());
    let oracle = history_of(&rt3_value());
    let p = bind_p(&mut host, 0);
    let mut client = Client::new(host.epoch(p).expect("bound"));
    client.drain(&mut host, p, 0);
    let replica = &client.replica;
    assert_eq!(
        replica.encounter(),
        &EncounterFacts {
            mode: Some(NetMode::RealTime),
            pending_mode: Some(None),
            clock: Some(Some(crpg_net::protocol_v3::ClockFact {
                round_ticks: 4,
                elapsed: 0,
            })),
        }
    );
    assert_eq!(
        replica.timeline(),
        Some(
            &[
                TimelineEntry {
                    key: 0,
                    entity: n(1)
                },
                TimelineEntry {
                    key: 0,
                    entity: n(2)
                },
                TimelineEntry {
                    key: 0,
                    entity: n(3)
                },
            ][..]
        )
    );
    assert_eq!(
        replica.turn(),
        Some(&TurnFact {
            active: None,
            round: 0,
        })
    );
    assert_authority(&host, &oracle);
}

/// C7 (pending mode): a requested mode reads as the pending mode.
#[test]
fn c7_pending_mode_is_a_fact() {
    let mut host = host_of(&tb3c_value());
    let mut oracle = history_of(&tb3c_value());
    host.request_mode(EncounterMode::RealTime)
        .expect("accepted");
    oracle
        .request_mode(EncounterMode::RealTime)
        .expect("the mirror accepts the same request");
    let p = bind_p(&mut host, 0);
    let mut client = Client::new(host.epoch(p).expect("bound"));
    client.drain(&mut host, p, 0);
    assert_eq!(
        client.replica.encounter(),
        &EncounterFacts {
            mode: Some(NetMode::TurnBased),
            pending_mode: Some(Some(NetMode::RealTime)),
            clock: Some(Some(crpg_net::protocol_v3::ClockFact {
                round_ticks: 4,
                elapsed: 0,
            })),
        }
    );
    assert_authority(&host, &oracle);
}

/// C8: a revoked fact does not survive the next snapshot.
#[test]
fn c8_revoke_after_disclosure() {
    let mut host = host_of(&tb3_value());
    let oracle = history_of(&tb3_value());
    let p = bind_p(&mut host, 100);
    let mut client = Client::new(host.epoch(p).expect("bound"));
    client.drain(&mut host, p, 100);
    assert_eq!(
        client.publish(&mut host, p, 100),
        TransferDisposition::Published
    );

    // A's pools withheld, host grants all off: the next snapshot says so.
    let mut revoked = facts();
    revoked.entities[0].pools = false;
    revoked.host = HostGrants::default();
    assert_eq!(host.set_fact_grants(p, revoked), Ok(()));
    let (fresh, drained) = resync_and_drain(&mut host, p, Some(1), 101);
    assert_eq!(fresh.replica.entities()[0].facts.pools, None);
    assert_eq!(
        fresh.replica.entities()[1].facts.pools,
        row_facts(None).pools,
        "B's pools are still granted"
    );
    assert_eq!(
        drained.deltas[0].ops[1],
        DeltaOp::HostFacts(HostFacts {
            rights: r1(),
            auto_pause: None,
            turn_timer_ms: None,
        })
    );

    // Pause: the reason is withheld under the revoked host grant.
    host.pause(101).expect("pauses");
    let (paused, drained) = resync_and_drain(&mut host, p, Some(2), 102);
    assert_eq!(
        drained.deltas[0].ops[0],
        DeltaOp::Clock { paused: Some(None) }
    );
    assert_eq!(
        paused.replica.pause(),
        Some(PauseState::Paused { reason: None })
    );
    host.unpause(102).expect("unpauses");

    // Control: with the pause-reason grant back the reason shows.
    let mut regranted = facts();
    regranted.host = HostGrants {
        pause_reason: true,
        ..HostGrants::default()
    };
    host.set_fact_grants(p, regranted).expect("stores");
    host.pause(103).expect("pauses");
    let (_, drained) = resync_and_drain(&mut host, p, Some(3), 104);
    assert_eq!(
        drained.deltas[0].ops[0],
        DeltaOp::Clock {
            paused: Some(Some(crpg_net::protocol_v3::PauseReason::Operator))
        }
    );
    host.unpause(104).expect("unpauses");

    // Control loss hides the order fact, which follows control.
    host.set_fact_grants(p, facts()).expect("stores");
    let (before, _) = resync_and_drain(&mut host, p, Some(4), 105);
    assert_eq!(before.replica.entities()[0].facts.order, Some(None));
    host.set_control(p, control(&[])).expect("stores");
    let (after, _) = resync_and_drain(&mut host, p, Some(5), 106);
    assert_eq!(after.replica.entities()[0].facts.order, None);

    // Clearing turn_state is a narrowing; the body's turn is withheld.
    let mut no_turn = full_grants(&[a(), b(), c()]);
    no_turn.turn_state = false;
    assert_eq!(host.set_grants(p, no_turn), Ok(GrantUpdate::SnapshotStaged));
    let mut last = Client::new(host.epoch(p).expect("bound"));
    last.drain(&mut host, p, 107);
    assert_eq!(last.replica.turn(), None);
    assert_authority(&host, &oracle);
}

/// C10: precedence at every adjacent pair with both faults present.
#[test]
fn c10_precedence_pairs() {
    let too_many_control = || control(&(0..1025).map(eid).collect::<Vec<_>>());
    let too_many_facts = || FactGrants {
        entities: (0..1025).map(|index| all_fact_flags(eid(index))).collect(),
        ..FactGrants::default()
    };
    let duplicate_facts = || FactGrants {
        entities: vec![all_fact_flags(a()), all_fact_flags(a())],
        ..FactGrants::default()
    };

    // bind_v3_peer: Closed over TimeRegression.
    let mut host = host_of(&tb3_value());
    bind_p(&mut host, 100);
    assert_eq!(
        host.bind_v3_peer(control(&[a()]), full_grants(&[a()]), facts(), r1(), 50),
        Err(HostError::TimeRegression),
        "control: a regressed clock alone is TimeRegression"
    );
    host.shutdown();
    assert_eq!(
        host.bind_v3_peer(control(&[a()]), full_grants(&[a()]), facts(), r1(), 50),
        Err(HostError::Closed)
    );
    // TimeRegression over InvalidControl.
    let mut host = host_of(&tb3_value());
    let first = bind_p(&mut host, 100);
    let before = state(&mut host, &[first]);
    assert_eq!(
        host.bind_v3_peer(too_many_control(), full_grants(&[a()]), facts(), r1(), 50),
        Err(HostError::TimeRegression)
    );
    assert_eq!(
        host.bind_v3_peer(too_many_control(), full_grants(&[a()]), facts(), r1(), 100),
        Err(HostError::InvalidControl),
        "control: the same grant at a fine time is InvalidControl"
    );
    assert_eq!(state(&mut host, &[first]), before);
    // InvalidControl over InvalidGrants.
    assert_eq!(
        host.bind_v3_peer(
            too_many_control(),
            full_grants(&[a()]),
            too_many_facts(),
            r1(),
            100
        ),
        Err(HostError::InvalidControl)
    );
    assert_eq!(
        host.bind_v3_peer(
            control(&[a()]),
            full_grants(&[a()]),
            too_many_facts(),
            r1(),
            100
        ),
        Err(HostError::InvalidGrants),
        "control: the same fact grants with a fine control list"
    );
    assert_eq!(state(&mut host, &[first]), before);
    // InvalidGrants over ServerBusy. (ServerBusy over CounterExhausted
    // cannot be built through the public API: it needs 2^64 handles.)
    let mut busy = host_of(&tb3_value());
    let mut peers = Vec::new();
    for _ in 0..8 {
        peers.push(bind_p(&mut busy, 10));
    }
    let before = state(&mut busy, &peers);
    assert_eq!(
        busy.bind_v3_peer(
            control(&[a()]),
            full_grants(&[a()]),
            duplicate_facts(),
            r1(),
            10
        ),
        Err(HostError::InvalidGrants)
    );
    assert_eq!(state(&mut busy, &peers), before);
    assert_eq!(
        busy.bind_v3_peer(control(&[a()]), full_grants(&[a()]), facts(), r1(), 10),
        Err(HostError::ServerBusy),
        "control: the same bind with valid grants reaches ServerBusy"
    );

    // set_fact_grants: Closed over UnknownPeer.
    let mut host = host_of(&tb3_value());
    let p = bind_p(&mut host, 100);
    let v = host
        .bind_peer(control(&[a()]), full_grants(&[a()]), 100)
        .expect("binds");
    let unknown = p;
    host.unbind_peer(unknown);
    assert_eq!(
        host.set_fact_grants(unknown, too_many_facts()),
        Err(HostError::UnknownPeer),
        "control: an unbound handle on an open host"
    );
    // UnknownPeer over NotV3Peer cannot both hold for one handle: a handle
    // is either unbound or a v2 session. NotV3Peer over InvalidGrants:
    assert_eq!(
        host.set_fact_grants(v, too_many_facts()),
        Err(HostError::NotV3Peer)
    );
    let q = bind_p(&mut host, 100);
    let before = state(&mut host, &[v, q]);
    assert_eq!(
        host.set_fact_grants(v, too_many_facts()),
        Err(HostError::NotV3Peer)
    );
    assert_eq!(
        host.set_fact_grants(q, too_many_facts()),
        Err(HostError::InvalidGrants)
    );
    assert_eq!(
        host.set_fact_grants(q, duplicate_facts()),
        Err(HostError::InvalidGrants)
    );
    assert_eq!(state(&mut host, &[v, q]), before);
    assert_eq!(host.set_fact_grants(q, facts()), Ok(()));
    host.shutdown();
    assert_eq!(
        host.set_fact_grants(unknown, too_many_facts()),
        Err(HostError::Closed)
    );
    // Closed over NotV3Peer cannot be built: shutdown unbinds the v2 session
    // too, so no handle is a v2 session on a closed host.

    // set_pause_rights shares the first three steps.
    let mut host = host_of(&tb3_value());
    let p = bind_p(&mut host, 100);
    let v = host
        .bind_peer(control(&[a()]), full_grants(&[a()]), 100)
        .expect("binds");
    assert_eq!(host.set_pause_rights(v, r1()), Err(HostError::NotV3Peer));
    assert_eq!(host.set_pause_rights(p, r0()), Ok(()));
    host.unbind_peer(p);
    assert_eq!(host.set_pause_rights(p, r0()), Err(HostError::UnknownPeer));
    host.shutdown();
    assert_eq!(host.set_pause_rights(p, r0()), Err(HostError::Closed));
}

/// The unbound area is the zero Ulid (Reading 8).
#[test]
fn unbound_area_is_zero() {
    assert_eq!(HOST_UNBOUND_AREA, Ulid::from_u128(0));
    assert_eq!(
        DEFAULT_HOST_GRANTS,
        HostGrants {
            pause_reason: true,
            auto_pause: true,
            turn_timer: true,
        }
    );
}

/// C2 (fact families): every family reads from the world exactly: stats are
/// `Int` attributes then `Fixed` raws, tags are as stored, a creature row has
/// `effects` and `order` withheld and no health, and a granted entity that is
/// neither a combatant nor a creature has no health and no fact row.
#[test]
fn c2_fact_families_and_non_combatants() {
    let mut value = tb3_value();
    // A: two tags and one Fixed stat (1.5 is raw 98_304).
    let combatant_a = &mut value["world"]["combatants"][0][1];
    combatant_a["tags"] = json!(["alpha", "beta"]);
    combatant_a["fixed"] = json!([["speed", 98_304]]);
    // Entity index 3: a creature outside the encounter. Entity index 4: bare.
    let slots = value["world"]["entities"]["slots"]
        .as_array_mut()
        .expect("slots");
    slots.push(json!({"generation": 1, "value": {}}));
    slots.push(json!({"generation": 1, "value": {}}));
    value["world"]["creatures"] = json!([[
        {"index": 3, "generation": 1},
        {
            "placement": Ulid::from_u128(700).to_string(),
            "ruleset": Ulid::from_u128(602).to_string(),
            "attributes": [["beta", 5]],
            "fixed": [["reach", 65_536]],
            "health": 4,
            "max_health": 6,
            "pools": [{
                "id": pool_jy().to_string(),
                "max": 2,
                "current": 1,
                "refresh": {"type": "on_turn_start"}
            }],
            "tags": ["x", "y"]
        }
    ]]);
    let mut host = host_of(&value);
    let oracle = history_of(&value);
    let d = eid(3);
    let bare = eid(4);
    let mut grants = full_grants(&[a(), d, bare]);
    grants.entities.sort_by_key(|entity| entity.entity);
    let mut fact_grants = facts();
    fact_grants.entities = vec![all_fact_flags(a()), all_fact_flags(d), all_fact_flags(bare)];
    let p = host
        .bind_v3_peer(control(&[a(), d]), grants, fact_grants, r1(), 0)
        .expect("binds");
    let mut client = Client::new(host.epoch(p).expect("bound"));
    client.drain(&mut host, p, 0);
    let rows = client.replica.entities();
    assert_eq!(rows.len(), 3);
    // A (n(1)).
    assert_eq!(rows[0].net_id, n(1));
    assert_eq!(rows[0].health, Some(health(10, 10, false)));
    assert_eq!(
        rows[0].facts.stats,
        Some(vec![
            StatFact {
                name: "alpha".to_owned(),
                value: StatValue::Int(8),
            },
            StatFact {
                name: "speed".to_owned(),
                value: StatValue::Fixed(98_304),
            },
        ])
    );
    assert_eq!(
        rows[0].facts.tags,
        Some(vec!["alpha".to_owned(), "beta".to_owned()])
    );
    assert_eq!(rows[0].facts.effects, Some(vec![]));
    assert_eq!(rows[0].facts.order, Some(None));
    // The creature (n(2)): pools, stats and tags, but no effects and no
    // order, although it is in the control list.
    assert_eq!(rows[1].net_id, n(2));
    // `health` is the combatant's: a creature has none (C-2).
    assert_eq!(rows[1].health, None);
    assert_eq!(
        rows[1].facts,
        FactSet {
            pools: Some(vec![PoolFact {
                pool: pool_jy(),
                current: 1,
                max: 2,
            }]),
            effects: None,
            order: None,
            stats: Some(vec![
                StatFact {
                    name: "beta".to_owned(),
                    value: StatValue::Int(5),
                },
                StatFact {
                    name: "reach".to_owned(),
                    value: StatValue::Fixed(65_536),
                },
            ]),
            tags: Some(vec!["x".to_owned(), "y".to_owned()]),
        }
    );
    // The bare entity (n(3)): present, so a row, with no health and no facts.
    assert_eq!(
        rows[2],
        ReplicaEntityV3 {
            net_id: n(3),
            health: None,
            facts: FactSet::default(),
        }
    );
    assert_authority(&host, &oracle);
}

/// C-6: the host-facts delta reads the host: the armed timer and the
/// auto-pause toggles reach a session that is granted them and are withheld
/// from one that is not; a paused host's reason reaches the same split.
#[test]
fn c6_host_facts_delta_reads_the_host() {
    use crpg_server::host::HostSettings;
    let mut host = host_of(&tb3_value());
    let settings = HostSettings {
        initial_mode: EncounterMode::TurnBased,
        auto_pause: toggles(0x19),
        turn_timer_ms: Some(1000),
    };
    host.set_settings(settings, 10).expect("accepted");
    assert_eq!(host.turn_timer_remaining_ms(), Some(1000));
    let p = bind_p(&mut host, 10);
    let mut withheld = facts();
    withheld.host = HostGrants {
        pause_reason: true,
        auto_pause: false,
        turn_timer: false,
    };
    let q = host
        .bind_v3_peer(
            control(&[]),
            full_grants(&[a(), b(), c()]),
            withheld,
            r0(),
            10,
        )
        .expect("binds");
    let mut granted = Client::new(host.epoch(p).expect("bound"));
    let granted_drain = granted.drain(&mut host, p, 10);
    assert_eq!(
        granted_drain.deltas[0].ops,
        vec![
            DeltaOp::Clock { paused: None },
            DeltaOp::HostFacts(HostFacts {
                rights: r1(),
                auto_pause: Some(toggles(0x19)),
                turn_timer_ms: Some(Some(1000)),
            }),
        ]
    );
    let mut denied = Client::new(host.epoch(q).expect("bound"));
    let denied_drain = denied.drain(&mut host, q, 10);
    assert_eq!(
        denied_drain.deltas[0].ops[1],
        DeltaOp::HostFacts(HostFacts {
            rights: r0(),
            auto_pause: None,
            turn_timer_ms: None,
        })
    );
    // Paused: the frozen remainder and the reason.
    host.pause(20).expect("pauses");
    let (paused, drained) = resync_and_drain(&mut host, p, Some(1), 20);
    assert_eq!(
        drained.deltas[0].ops,
        vec![
            DeltaOp::Clock {
                paused: Some(Some(crpg_net::protocol_v3::PauseReason::Operator)),
            },
            DeltaOp::HostFacts(HostFacts {
                rights: r1(),
                auto_pause: Some(toggles(0x19)),
                turn_timer_ms: Some(Some(990)),
            }),
        ]
    );
    assert_eq!(
        paused.replica.pause(),
        Some(PauseState::Paused {
            reason: Some(crpg_net::protocol_v3::PauseReason::Operator),
        })
    );
}
