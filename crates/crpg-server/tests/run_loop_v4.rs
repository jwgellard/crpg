//! T154c acceptance suite (`tasks/T154.md` V1–V4): lane 1 on the local link,
//! public API only.
//!
//! `V4` is the host and `bind_v4_peer` setup of T037d's suite
//! (`host_lane1.rs`): the real-time `MV3rt` authority with `M10` bound, `P`
//! controlling `[A]`, drained and published; `mv(seq)` is its lane-1 `MoveTo`
//! for A. The twin adds `ingest_move`, `take_move_receipts` and
//! `take_samples` in B-11..B-13's places.
//!
//! Expected values come from the contract, never from the implementation.
//! The schedule is stepped by hand (`HostLoop::step`), so no case asserts a
//! wall-clock duration.

use std::num::NonZeroUsize;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_nav::{bake_box, NavMesh, NavRect};
use crpg_net::codec_v2;
use crpg_net::codec_v4;
use crpg_net::facts_v3::EncounterGrants;
use crpg_net::movement_v4::{
    decode_down, encode_intent, MoveBody, MoveDown, MoveIntent, MoveOutcome, MOVEMENT_VERSION,
};
use crpg_net::pool_v4::{
    GateMode, ReassemblyPoolV4, ReceiverIdV4, ReplicaUpdateV4, TransferLimitsV3,
};
use crpg_net::protocol::{IntentBody, IntentFrame, NetId, LANE_COMBAT};
use crpg_net::protocol_transfer::{TransferId, TransferUp};
use crpg_net::protocol_v3::{PauseRights, RejectionCode as CodeV3};
use crpg_net::protocol_v4::SessionEpoch;
use crpg_net::protocol_v4::{IntentBody as IntentBodyV4, IntentFrame as IntentFrameV4, Point};
use crpg_net::replica_v4::ReplicaWorldV4;
use crpg_net::transfer_v3;
use crpg_net::transport::Transport;
use crpg_server::host::{
    ControlGrant, DeliveryKind, DisclosureGrants, EntityDisclosure, EntityFactGrant, FactGrants,
    Host, HostConfig, HostError, MoveAdmission, PeerHandle, ProtocolSelection, TransferDisposition,
    DEFAULT_HOST_GRANTS,
};
use crpg_server::run::{HostLoop, LocalHost};
use crpg_sim::{HistoryWorld, NavRevision};
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");

// ---------------------------------------------------------------------------
// Identities, fixtures and grants (copied from `host_lane1.rs`).
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
fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}
/// A raw wire point `[x, y, z]`.
fn pt(x: i32, y: i32, z: i32) -> Point {
    Point::new([x, y, z]).expect("within the wire bound")
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
         {"position": [3.0, 0.0, 1.0], "velocity": [0.0, 0.0, 1.0]}],
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

fn cap(capacity: usize) -> NonZeroUsize {
    NonZeroUsize::new(capacity).expect("nonzero")
}

fn revision() -> NavRevision {
    NavRevision::of_bytes(&m10().to_canonical_bytes())
}

// ---------------------------------------------------------------------------
// Intent bytes.
// ---------------------------------------------------------------------------

/// `mv(seq)`: the lane-1 `MoveTo` for A T037d's suite sends (`0xA1` first).
fn mv(host: &Host, peer: PeerHandle, seq: u64) -> Vec<u8> {
    let bytes = encode_intent(&MoveIntent {
        epoch: host.epoch(peer).expect("bound"),
        seq,
        actor: n(1),
        body: MoveBody::MoveTo {
            destination: pt(262144, 0, 65536),
        },
    })
    .expect("encodes");
    assert_eq!(bytes.first(), Some(&MOVEMENT_VERSION));
    bytes
}

/// One lane-0 `Pause` for A, accepted under the suite's pause rights.
fn pause_frame(host: &Host, peer: PeerHandle, seq: u64) -> Vec<u8> {
    codec_v4::encode_intent(&IntentFrameV4 {
        epoch: host.epoch(peer).expect("bound"),
        lane: LANE_COMBAT,
        seq,
        observed_tick: host.server_tick(),
        actor: n(1),
        body: IntentBodyV4::Pause,
    })
    .expect("encodes")
}

// ---------------------------------------------------------------------------
// The v4 client (T026p5), for the publish half of the setup.
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

/// The suite setup: real-time movers, `M10`, `P` controlling `[A]`, drained
/// and published.
fn setup() -> (Host, PeerHandle) {
    let mut host = Host::from_history(history_of(&mv3rt_value()), config(), 0).expect("wraps");
    let binding = host.bind_nav_mesh(m10()).expect("binds");
    assert_eq!(binding.revision, revision());
    let peer = host
        .bind_v4_peer(
            control(&[a()]),
            full_grants(&[a(), b(), c()]),
            facts(),
            r1(),
            0,
        )
        .expect("binds");
    let mut client = Client::new(host.epoch(peer).expect("bound"));
    client.drain(&mut host, peer, 0);
    assert_eq!(
        client.publish(&mut host, peer, 0),
        TransferDisposition::Published
    );
    (host, peer)
}

/// The suite setup without `Published`: the transfer stays in flight, so
/// tick publications are throttled and the delivery log stays empty.
fn setup_quiet() -> (Host, PeerHandle) {
    let mut host = Host::from_history(history_of(&mv3rt_value()), config(), 0).expect("wraps");
    let binding = host.bind_nav_mesh(m10()).expect("binds");
    assert_eq!(binding.revision, revision());
    let peer = host
        .bind_v4_peer(
            control(&[a()]),
            full_grants(&[a(), b(), c()]),
            facts(),
            r1(),
            0,
        )
        .expect("binds");
    let entries = host.take_delivery_entries(peer).expect("bound");
    if let Some(last) = entries.last() {
        host.acknowledge_delivery(peer, last.ordinal).expect("acks");
    }
    (host, peer)
}
/// One twin cycle in B-11..B-13's order: `ingest_move` per frame, `pump`,
/// `advance`, then delivery, receipts, samples, and retired captures. Returns
/// every frame the client would have received, in deliver order.
fn twin_cycle(host: &mut Host, peer: PeerHandle, moves: &[Vec<u8>], now: u64) -> Vec<Vec<u8>> {
    for frame in moves {
        assert!(
            matches!(
                host.ingest_move(peer, frame, now),
                Ok(MoveAdmission::Admitted { .. })
            ),
            "the suite setup admits every sent move"
        );
    }
    host.pump(now).expect("pumps");
    host.advance(now).expect("advances");
    let mut out = Vec::new();
    let entries = host.take_delivery_entries(peer).expect("bound");
    if let Some(last) = entries.last() {
        host.acknowledge_delivery(peer, last.ordinal).expect("acks");
    }
    out.extend(entries.into_iter().map(|entry| entry.bytes));
    out.extend(host.take_move_receipts(peer).expect("receipts"));
    out.extend(host.take_samples(peer).expect("samples"));
    host.acknowledge_captures(host.last_capture())
        .expect("acks captures");
    out
}

/// Reads every frame the client's down queue holds right now.
fn drain(client: &mut impl Transport) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    while let Some(frame) = client.recv() {
        frames.push(frame);
    }
    frames
}

/// The receipt seqs in `frames`, in order; anything else is skipped.
fn receipts_of(frames: &[Vec<u8>]) -> Vec<u64> {
    frames
        .iter()
        .filter_map(|bytes| match decode_down(bytes) {
            Ok(MoveDown::Receipt(receipt)) => Some(receipt.seq),
            _ => None,
        })
        .collect()
}

/// Whether `frames` hold at least one sample datagram.
fn has_sample(frames: &[Vec<u8>]) -> bool {
    frames
        .iter()
        .any(|bytes| matches!(decode_down(bytes), Ok(MoveDown::Sample(_))))
}

// ---------------------------------------------------------------------------
// V1–V4.
// ---------------------------------------------------------------------------

/// V1 (B-11–B-13). `mv(1)` through the loop equals the twin, over two cycles.
#[test]
fn lane1_round_trip_equals_direct_calls() {
    let (host, p) = setup();
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    let frame = mv(host_loop.hosting().host(), p, 1);
    client.send(&frame).expect("sends");

    let (mut twin, q) = setup();
    assert_eq!(q, p);
    let expected = twin_cycle(&mut twin, q, std::slice::from_ref(&frame), 0);
    host_loop.step(0).expect("runs");
    let first = drain(&mut client);
    assert_eq!(first, expected);

    let expected_next = twin_cycle(&mut twin, q, &[], 50);
    host_loop.step(50).expect("runs");
    let second = drain(&mut client);
    assert_eq!(second, expected_next);
    assert_eq!(
        host_loop.hosting().host().authority_hash(),
        twin.authority_hash()
    );

    // Control: with no frame the client receives a different first cycle.
    let (host, p) = setup();
    let mut quiet = HostLoop::new(LocalHost::new(host), 0);
    let mut idle = quiet.hosting_mut().attach(p, cap(64)).expect("attaches");
    quiet.step(0).expect("runs");
    assert_ne!(drain(&mut idle), first);
}

/// V2 (B-11). A held lane-1 frame waits front first; a lane-0 frame behind
/// the hold is still received, and the hold clears without a refusal.
#[test]
fn lane1_hold_does_not_stop_lane0() {
    let (host, p) = setup();
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    // Twelve moves exceed the in-flight window of eight, so the ninth holds
    // and the rest queue behind it; the pause then proves lane 0 still flows.
    for seq in 1..=12 {
        client
            .send(&mv(host_loop.hosting().host(), p, seq))
            .expect("sends");
    }
    let pause = pause_frame(host_loop.hosting().host(), p, 1);
    client.send(&pause).expect("sends");

    host_loop.step(0).expect("runs");
    let stats = host_loop.hosting().link_stats(p).expect("link");
    assert_eq!(stats.received, 13);
    assert_eq!(stats.refused, 0);
    assert!(stats.held_moves >= 1);

    // The decision window frees the hold within ten more cycles, and no move
    // is refused while it drains.
    for now in (1..=10).map(|i| i * 50) {
        host_loop.step(now).expect("runs");
        assert_eq!(host_loop.hosting().link_stats(p).expect("link").refused, 0);
        if host_loop.hosting().link_stats(p).expect("link").held_moves == 0 {
            break;
        }
    }
    assert_eq!(
        host_loop.hosting().link_stats(p).expect("link").held_moves,
        0
    );
    assert_eq!(host_loop.hosting().link_stats(p).expect("link").refused, 0);

    // Control: three moves fit the window, so nothing holds.
    let (host, p) = setup();
    let mut quiet = HostLoop::new(LocalHost::new(host), 0);
    let mut idle = quiet.hosting_mut().attach(p, cap(64)).expect("attaches");
    for seq in 1..=3 {
        idle.send(&mv(quiet.hosting().host(), p, seq))
            .expect("sends");
    }
    let pause = pause_frame(quiet.hosting().host(), p, 1);
    idle.send(&pause).expect("sends");
    quiet.step(0).expect("runs");
    let stats = quiet.hosting().link_stats(p).expect("link");
    assert_eq!(stats.received, 4);
    assert_eq!(stats.refused, 0);
    assert_eq!(stats.held_moves, 0);
}

/// V3 (B-12). A capacity-1 link keeps every receipt in the backlog: all three
/// arrive, seq 1, 2, 3, within twenty cycles of one read each. The session is
/// unpublished, so throttled tick publications leave the delivery log empty
/// and the receipts reach the one-slot queue at once.
#[test]
fn receipts_are_never_dropped() {
    let (host, p) = setup_quiet();
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop.hosting_mut().attach(p, cap(1)).expect("attaches");
    let mut sent = 0;
    let mut seqs = Vec::new();
    for now in (0..20).map(|i| i * 50) {
        if sent < 3 {
            sent += 1;
            client
                .send(&mv(host_loop.hosting().host(), p, sent))
                .expect("sends");
        }
        host_loop.step(now).expect("runs");
        if let Some(frame) = client.recv() {
            if let Ok(MoveDown::Receipt(receipt)) = decode_down(&frame) {
                seqs.push(receipt.seq);
            }
        }
        if seqs.len() == 3 {
            break;
        }
    }
    assert_eq!(seqs, vec![1, 2, 3]);
    assert!(host_loop.hosting().link_stats(p).is_some());

    // Control: capacity 64 delivers all three receipts in the first cycle.
    let (host, p) = setup();
    let mut wide = HostLoop::new(LocalHost::new(host), 0);
    let mut roomy = wide.hosting_mut().attach(p, cap(64)).expect("attaches");
    for seq in 1..=3 {
        roomy
            .send(&mv(wide.hosting().host(), p, seq))
            .expect("sends");
    }
    wide.step(0).expect("runs");
    assert_eq!(receipts_of(&drain(&mut roomy)), vec![1, 2, 3]);
}

/// `mv` with an explicit actor net id: naming B (`n(2)`) while the peer
/// controls A is step 9's `Rejected(NotAuthorized)`.
fn mv_actor(host: &Host, peer: PeerHandle, seq: u64, actor: u64) -> Vec<u8> {
    let bytes = encode_intent(&MoveIntent {
        epoch: host.epoch(peer).expect("bound"),
        seq,
        actor: n(actor),
        body: MoveBody::MoveTo {
            destination: pt(262144, 0, 65536),
        },
    })
    .expect("encodes");
    assert_eq!(bytes.first(), Some(&MOVEMENT_VERSION));
    bytes
}

/// `mv` under a foreign epoch: step 2's `Refused(SessionExpired)`, which
/// consumes nothing.
fn wrong_epoch_mv(seq: u64) -> Vec<u8> {
    encode_intent(&MoveIntent {
        epoch: [9; 16],
        seq,
        actor: n(1),
        body: MoveBody::MoveTo {
            destination: pt(262144, 0, 65536),
        },
    })
    .expect("encodes")
}

/// One lane-0 `EndTurn` for A on a v1/v2 session.
fn end_frame(host: &Host, peer: PeerHandle, seq: u64) -> Vec<u8> {
    codec_v2::encode_intent(&IntentFrame {
        epoch: host.epoch(peer).expect("bound"),
        lane: LANE_COMBAT,
        seq,
        observed_tick: host.server_tick(),
        actor: n(1),
        body: IntentBody::EndTurn,
    })
    .expect("encodes")
}

/// The `(seq, outcome)` of every receipt in `frames`, in order.
fn receipt_outcomes(frames: &[Vec<u8>]) -> Vec<(u64, MoveOutcome)> {
    frames
        .iter()
        .filter_map(|bytes| match decode_down(bytes) {
            Ok(MoveDown::Receipt(receipt)) => Some((receipt.seq, receipt.outcome)),
            _ => None,
        })
        .collect()
}

/// r02 (B-11). A `Rejected` lane-1 frame continues the drain, never holds:
/// the rejected seq 1 and the valid seq 2 behind it are both decided in the
/// same cycle and receipted together, with nothing held.
#[test]
fn lane1_rejected_continues_not_hold() {
    let (host, p) = setup();
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    client
        .send(&mv_actor(host_loop.hosting().host(), p, 1, 2))
        .expect("sends");
    client
        .send(&mv(host_loop.hosting().host(), p, 2))
        .expect("sends");
    host_loop.step(0).expect("runs");
    let stats = host_loop.hosting().link_stats(p).expect("link");
    assert_eq!(stats.received, 2);
    assert_eq!(stats.refused, 0);
    assert_eq!(stats.held_moves, 0);
    let outcomes = receipt_outcomes(&drain(&mut client));
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].0, 1);
    assert!(matches!(
        outcomes[0].1,
        MoveOutcome::Rejected(CodeV3::NotAuthorized)
    ));
    assert_eq!(outcomes[1].0, 2);

    // Control: the same two seqs, both for the controlled actor, receipt
    // both as decided intents with nothing refused.
    let (host, p) = setup();
    let mut quiet = HostLoop::new(LocalHost::new(host), 0);
    let mut idle = quiet.hosting_mut().attach(p, cap(64)).expect("attaches");
    idle.send(&mv(quiet.hosting().host(), p, 1)).expect("sends");
    idle.send(&mv(quiet.hosting().host(), p, 2)).expect("sends");
    quiet.step(0).expect("runs");
    let stats = quiet.hosting().link_stats(p).expect("link");
    assert_eq!(stats.received, 2);
    assert_eq!(stats.refused, 0);
    assert_eq!(stats.held_moves, 0);
    assert_eq!(receipts_of(&drain(&mut idle)), vec![1, 2]);
}

/// r04 (B-11). `RateLimited` is held like `QueueFull`: thirty-two capped
/// frames spend exactly the 128 KiB shared byte burst, so the valid frame
/// behind them holds (`held_moves == 1`, `refused == 32`) and is admitted on
/// a later cycle once the rate allows, receipted as seq 1.
#[test]
fn lane1_rate_limited_holds_like_queue_full() {
    let (host, p) = setup_quiet();
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    let valid = mv(host_loop.hosting().host(), p, 1);
    let mut padded = valid.clone();
    padded.resize(4096, 0);
    assert_eq!(padded.first(), Some(&MOVEMENT_VERSION));
    for _ in 0..32 {
        client.send(&padded).expect("sends");
    }
    client.send(&valid).expect("sends");
    host_loop.step(0).expect("runs");
    let stats = host_loop.hosting().link_stats(p).expect("link");
    assert_eq!(stats.received, 33);
    assert_eq!(stats.refused, 32);
    assert_eq!(stats.held_moves, 1);
    host_loop.step(50).expect("runs");
    let stats = host_loop.hosting().link_stats(p).expect("link");
    assert_eq!(stats.received, 33);
    assert_eq!(stats.refused, 32);
    assert_eq!(stats.held_moves, 0);
    assert_eq!(receipts_of(&drain(&mut client)), vec![1]);

    // Control: the valid frame alone is admitted at once.
    let (host, p) = setup_quiet();
    let mut quiet = HostLoop::new(LocalHost::new(host), 0);
    let mut idle = quiet.hosting_mut().attach(p, cap(64)).expect("attaches");
    idle.send(&mv(quiet.hosting().host(), p, 1)).expect("sends");
    quiet.step(0).expect("runs");
    let stats = quiet.hosting().link_stats(p).expect("link");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    assert_eq!(stats.held_moves, 0);
    assert_eq!(receipts_of(&drain(&mut idle)), vec![1]);
}

/// r06, r07 (B-11, Errors). `Refused(Unauthenticated)` retires the link and
/// counts: after the unbind the frame is received and refused, and the link
/// is gone with one retired record.
#[test]
fn lane1_unauthenticated_retires_and_counts() {
    let (host, p) = setup();
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    host_loop.step(0).expect("runs");
    assert!(host_loop.hosting_mut().take_retired().is_empty());
    let frame = mv(host_loop.hosting().host(), p, 1);
    host_loop.hosting_mut().host_mut().unbind_peer(p);
    client.send(&frame).expect("sends");
    host_loop.step(50).expect("runs");
    assert_eq!(host_loop.hosting().link_stats(p), None);
    assert_eq!(
        host_loop.hosting().host().epoch(p),
        Err(HostError::UnknownPeer)
    );
    let retired = host_loop.hosting_mut().take_retired();
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].0, p);
    assert_eq!(retired[0].1.received, 1);
    assert_eq!(retired[0].1.refused, 1);

    // Control: the bound peer's frame is received, refused by none, and the
    // link lives.
    let (host, p) = setup();
    let mut quiet = HostLoop::new(LocalHost::new(host), 0);
    let mut idle = quiet.hosting_mut().attach(p, cap(64)).expect("attaches");
    idle.send(&mv(quiet.hosting().host(), p, 1)).expect("sends");
    quiet.step(0).expect("runs");
    let stats = quiet.hosting().link_stats(p).expect("link");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    assert!(quiet.hosting_mut().take_retired().is_empty());
}

/// r08, r09 (B-11). Any other `Refused` counts and continues: a
/// `SessionExpired` frame is refused without consuming its seq, and the
/// valid frame for that seq behind it is admitted in the same cycle.
#[test]
fn lane1_session_expired_counts_and_continues() {
    let (host, p) = setup();
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    client.send(&wrong_epoch_mv(1)).expect("sends");
    client
        .send(&mv(host_loop.hosting().host(), p, 1))
        .expect("sends");
    host_loop.step(0).expect("runs");
    let stats = host_loop.hosting().link_stats(p).expect("link");
    assert_eq!(stats.received, 2);
    assert_eq!(stats.refused, 1);
    assert_eq!(stats.held_moves, 0);
    assert_eq!(receipts_of(&drain(&mut client)), vec![1]);

    // Control: the valid frame alone is refused by nothing.
    let (host, p) = setup();
    let mut quiet = HostLoop::new(LocalHost::new(host), 0);
    let mut idle = quiet.hosting_mut().attach(p, cap(64)).expect("attaches");
    idle.send(&mv(quiet.hosting().host(), p, 1)).expect("sends");
    quiet.step(0).expect("runs");
    let stats = quiet.hosting().link_stats(p).expect("link");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    assert_eq!(stats.held_moves, 0);
    assert_eq!(receipts_of(&drain(&mut idle)), vec![1]);
}

/// r10, r11 (B-11). A v1 peer's `0xA1` frame counts and continues: the cycle
/// is `Ok`, `refused == 1`, the link is not retired, and a lane-0 frame from
/// the same peer is still ingested in that cycle.
#[test]
fn lane1_v1_peer_a1_counts_and_continues() {
    let mut host = Host::from_history(history_of(&mv3rt_value()), config(), 0).expect("wraps");
    host.bind_nav_mesh(m10()).expect("binds");
    let v = host
        .bind_peer(control(&[a(), b(), c()]), full_grants(&[a(), b(), c()]), 0)
        .expect("binds");
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(v, cap(64))
        .expect("attaches");
    let a1 = mv(host_loop.hosting().host(), v, 1);
    assert_eq!(a1.first(), Some(&MOVEMENT_VERSION));
    client.send(&a1).expect("sends");
    client
        .send(&end_frame(host_loop.hosting().host(), v, 1))
        .expect("sends");
    host_loop.step(0).expect("runs");
    let stats = host_loop.hosting().link_stats(v).expect("link");
    assert_eq!(stats.received, 2);
    assert_eq!(stats.refused, 1);
    assert_eq!(stats.held_moves, 0);

    // Control: the lane-0 frame alone is refused by nothing.
    let mut host = Host::from_history(history_of(&mv3rt_value()), config(), 0).expect("wraps");
    host.bind_nav_mesh(m10()).expect("binds");
    let v = host
        .bind_peer(control(&[a(), b(), c()]), full_grants(&[a(), b(), c()]), 0)
        .expect("binds");
    let mut quiet = HostLoop::new(LocalHost::new(host), 0);
    let mut idle = quiet.hosting_mut().attach(v, cap(64)).expect("attaches");
    idle.send(&end_frame(quiet.hosting().host(), v, 1))
        .expect("sends");
    quiet.step(0).expect("runs");
    let stats = quiet.hosting().link_stats(v).expect("link");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
}

/// h06 (B-11). A retry that retires stops that link's intake for the cycle:
/// after the unbind the held retry ends the link, and the frame queued
/// behind it is never ingested (`received` stays 12, `refused` counts the
/// retry alone).
#[test]
fn lane1_retry_retire_stops_intake() {
    let (host, p) = setup();
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    for seq in 1..=12 {
        client
            .send(&mv(host_loop.hosting().host(), p, seq))
            .expect("sends");
    }
    host_loop.step(0).expect("runs");
    assert_eq!(
        host_loop.hosting().link_stats(p).expect("link").held_moves,
        4
    );
    let queued = mv(host_loop.hosting().host(), p, 13);
    host_loop.hosting_mut().host_mut().unbind_peer(p);
    client.send(&queued).expect("sends");
    host_loop.step(50).expect("runs");
    assert_eq!(host_loop.hosting().link_stats(p), None);
    let retired = host_loop.hosting_mut().take_retired();
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].0, p);
    assert_eq!(retired[0].1.received, 12);
    assert_eq!(retired[0].1.refused, 1);

    // Control: while bound, the same retry is admitted and drains the hold
    // with nothing refused.
    let (host, p) = setup();
    let mut quiet = HostLoop::new(LocalHost::new(host), 0);
    let mut idle = quiet.hosting_mut().attach(p, cap(64)).expect("attaches");
    for seq in 1..=12 {
        idle.send(&mv(quiet.hosting().host(), p, seq))
            .expect("sends");
    }
    quiet.step(0).expect("runs");
    for now in (1..=10).map(|i| i * 50) {
        quiet.step(now).expect("runs");
        if quiet.hosting().link_stats(p).expect("link").held_moves == 0 {
            break;
        }
    }
    let stats = quiet.hosting().link_stats(p).expect("link");
    assert_eq!(stats.held_moves, 0);
    assert_eq!(stats.refused, 0);
}

/// V4 (B-13). An unread capacity-1 link to a moving A keeps no error, keeps
/// the link, and holds at most one frame; a v1 peer skips lane 1 entirely.
#[test]
fn samples_drop_and_v1_peers_skip_lane1() {
    let mut host = Host::from_history(history_of(&mv3rt_value()), config(), 0).expect("wraps");
    host.bind_nav_mesh(m10()).expect("binds");
    let p = host
        .bind_v4_peer(
            control(&[a()]),
            full_grants(&[a(), b(), c()]),
            facts(),
            r1(),
            0,
        )
        .expect("binds");
    let mut publisher = Client::new(host.epoch(p).expect("bound"));
    publisher.drain(&mut host, p, 0);
    assert_eq!(
        publisher.publish(&mut host, p, 0),
        TransferDisposition::Published
    );
    let v = host
        .bind_peer(control(&[b(), c()]), full_grants(&[a(), b(), c()]), 0)
        .expect("binds");
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut moving = host_loop.hosting_mut().attach(p, cap(1)).expect("attaches");
    let _v_client = host_loop
        .hosting_mut()
        .attach(v, cap(64))
        .expect("attaches");
    moving
        .send(&mv(host_loop.hosting().host(), p, 1))
        .expect("sends");

    for now in (0..5).map(|i| i * 50) {
        host_loop.step(now).expect("runs");
    }
    assert!(host_loop.hosting().link_stats(p).is_some());
    assert!(host_loop.hosting().link_stats(v).is_some());
    assert!(drain(&mut moving).len() <= 1);

    // Control: capacity 64 delivers a sample every ticking cycle.
    let (host, p) = setup();
    let mut wide = HostLoop::new(LocalHost::new(host), 0);
    let mut roomy = wide.hosting_mut().attach(p, cap(64)).expect("attaches");
    roomy.send(&mv(wide.hosting().host(), p, 1)).expect("sends");
    for now in (0..3).map(|i| i * 50) {
        wide.step(now).expect("runs");
        assert!(has_sample(&drain(&mut roomy)));
    }
}
