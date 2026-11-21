//! T155a acceptance suite: a protocol v4 (movement) session is resumable and a
//! grace or a resume un-admits its unanswered lane-1 intents (`tasks/T155.md`
//! H-1 to H-6, tests V1 to V6; U1 is a unit test in `src/host.rs`).
//!
//! Public API only. The rig, fixtures and client are copied from
//! `host_lane1.rs:25-681` (T037d); the resume helpers are `host_reconnect.rs`'s
//! (T025b-i). Every grace and resume is timed by the test's `now_ms`; no test
//! sleeps. Expected values are the contract's, never read back from the host.
//!
//! `P` is `lane1_rig(&mv3rt_value())`'s session (handle 1, control `[a]`) with
//! `issue_resume_ticket(p, P)` called right after; `E` is its epoch.

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_nav::{bake_box, NavMesh, NavRect};
use crpg_net::facts_v3::EncounterGrants;
use crpg_net::movement_v4::{
    decode_down, encode_down, encode_intent, MoveBody, MoveDown, MoveIntent,
    MoveOutcome as MoveVerdict, MoveReceipt, MoveSample,
};
use crpg_net::outbox_v4::MoveOutbox;
use crpg_net::pool_v4::{
    GateMode, ReassemblyPoolV4, ReceiverIdV4, ReplicaUpdateV4, TransferLimitsV3,
};
use crpg_net::protocol::NetId;
use crpg_net::protocol_resume::{
    ConnectionFence, LaneClaim, LaneProgress, ResumeAccepted, ResumeProof, ResumeRejected,
    ResumeRejection, ResumeRequest,
};
use crpg_net::protocol_transfer::{TransferId, TransferUp};
use crpg_net::protocol_v3::{PauseRights, RejectionCode as CodeV3};
use crpg_net::protocol_v4::Point;
use crpg_net::protocol_v4::SessionEpoch;
use crpg_net::replica_v4::ReplicaWorldV4;
use crpg_net::transfer_v3;
use crpg_server::host::{
    AdvanceReport, ControlGrant, DisclosureGrants, EntityDisclosure, EntityFactGrant, FactGrants,
    Host, HostConfig, HostError, MoveAdmission, PeerHandle, ProtocolSelection, ResumeAnswer,
    TransferDisposition, DEFAULT_HOST_GRANTS,
};
use crpg_sim::{HistoryWorld, NavRevision};
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

    fn mirror_tick(&mut self) {
        self.oracle.tick().expect("the mirror ticks");
        self.mirror_ack();
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

// ---------------------------------------------------------------------------
// Resume helpers (T025b, `tasks/T025b.md:511-515`).
// ---------------------------------------------------------------------------

/// `P`: the resume proof every case stores.
fn proof_p() -> ResumeProof {
    ResumeProof::new(vec![0xA5; 32]).expect("valid proof")
}

/// `req(e, f, proof)`: a lane-0 resume request at connection fence `f`.
fn req(epoch: [u8; 16], fence: u64, proof: ResumeProof) -> ResumeRequest {
    ResumeRequest {
        epoch,
        fence: ConnectionFence::new(fence).expect("nonzero fence"),
        proof,
        claims: vec![LaneClaim {
            lane: 0,
            receipted_through: 0,
        }],
    }
}

/// `ACC(peer, e, g, f)`: accepted at fence `g`, lane 0 finalized through `f`.
fn acc(
    peer: PeerHandle,
    epoch: [u8; 16],
    granted: u64,
    finalized: u64,
) -> Result<ResumeAnswer, HostError> {
    Ok(ResumeAnswer::Accepted {
        peer,
        accepted: ResumeAccepted {
            epoch,
            granted: ConnectionFence::new(granted).expect("nonzero fence"),
            progress: vec![LaneProgress {
                lane: 0,
                finalized_through: finalized,
            }],
        },
    })
}

/// `DENIED(e)`.
fn denied(epoch: [u8; 16]) -> Result<ResumeAnswer, HostError> {
    Ok(ResumeAnswer::Rejected(ResumeRejected {
        epoch,
        reason: ResumeRejection::Denied,
    }))
}

/// `halt(s, k)`: the canonical bytes of a `Halt` for `n(k)` at lane-1 seq `s`.
fn halt(epoch: [u8; 16], seq: u64, actor: u64) -> Vec<u8> {
    halt_bytes(epoch, seq, actor)
}

/// `rcpt(s, k, t, outcome)`.
fn rcpt(epoch: [u8; 16], seq: u64, actor: u64, tick: u64, outcome: MoveVerdict) -> MoveReceipt {
    MoveReceipt {
        epoch,
        seq,
        actor: n(actor),
        processed_tick: tick,
        outcome,
    }
}

/// The wire bytes of one receipt.
fn receipt_bytes(receipt: MoveReceipt) -> Vec<u8> {
    encode_down(&MoveDown::Receipt(receipt)).expect("encodes")
}

/// `lane1_rig(&mv3rt_value())` with `P` stored, keeping the first client (V4
/// and V5 need it; `lane1_rig` drops it).
fn ticketed_rig() -> (Rig, PeerHandle, [u8; 16], Client) {
    let mut rig = Rig::new(&mv3rt_value());
    rig.bind_nav();
    let p = rig.bind_p();
    let client = rig.publish(p);
    let epoch = rig.epoch(p);
    rig.host
        .issue_resume_ticket(p, proof_p())
        .expect("a v4 session takes a ticket");
    (rig, p, epoch, client)
}

/// The anchored positions of a replica, ascending by `NetId`.
fn positions_of(client: &Client) -> Vec<(NetId, Point, bool)> {
    client
        .replica
        .positions()
        .iter()
        .map(|(net, state)| (*net, state.point, state.moving))
        .collect()
}

fn assert_positions(client: &Client, expected: &[(u64, Point, bool)]) {
    let expected: Vec<(NetId, Point, bool)> = expected
        .iter()
        .map(|(raw, point, moving)| (n(*raw), *point, *moving))
        .collect();
    assert_eq!(positions_of(client), expected);
}

/// The start points of `MV3`'s three movers.
fn start_points() -> Vec<(u64, Point, bool)> {
    vec![
        (1, pt(65536, 0, 65536), false),
        (2, pt(196608, 0, 65536), false),
        (3, pt(65536, 0, 327680), false),
    ]
}

fn assert_ticked_at(rig: &mut Rig, now: u64) {
    let report = rig.advance(now);
    assert!(report.ticked, "{report:?}");
}

// ---------------------------------------------------------------------------
// V1.
// ---------------------------------------------------------------------------

/// V1 (H-1, H-6): a v4 session takes a ticket and a grace, and a graced one is
/// closed to every lane-1 call while the world goes on. Controls: a v4 session
/// with no ticket and a `bind_peer` session stay `NotResumable`.
#[test]
fn v1_v4_sessions_take_a_ticket_and_a_grace() {
    let mut rig = Rig::new(&mv3rt_value());
    rig.bind_nav();
    let p = rig.bind_p();
    rig.publish(p);
    let epoch = rig.epoch(p);
    let ticket = rig
        .host
        .issue_resume_ticket(p, proof_p())
        .expect("a v4 session takes a ticket");
    let mut expected = vec![0x91, 0x03, 0x30];
    expected.extend_from_slice(&epoch);
    expected.push(0x20);
    expected.extend_from_slice(&[0xA5; 32]);
    assert_eq!(expected.len(), 52);
    assert_eq!(ticket, expected);

    assert_eq!(rig.host.begin_grace(p, 100), Ok(30_100));
    assert_eq!(rig.host.grace_deadline(p), Some(30_100));
    assert_eq!(rig.host.epoch(p), Err(HostError::UnknownPeer));
    assert_eq!(rig.host.take_samples(p), Err(HostError::UnknownPeer));
    assert_eq!(rig.host.take_move_receipts(p), Err(HostError::UnknownPeer));
    assert_eq!(
        rig.host.set_position_withheld(p, vec![]),
        Err(HostError::UnknownPeer)
    );
    assert_eq!(
        rig.ingest_move(p, &halt(epoch, 1, 1), 200),
        MoveAdmission::Refused {
            seq: None,
            code: CodeV3::Unauthenticated,
        }
    );
    assert_ticked_at(&mut rig, 250);
    assert_eq!(rig.host.take_samples(p), Err(HostError::UnknownPeer));

    // Controls.
    let q = rig.bind_p_control(&[b()]);
    assert_eq!(rig.host.begin_grace(q, 300), Err(HostError::NotResumable));
    let plain = rig
        .host
        .bind_peer(control(&[a()]), full_grants(&[a(), b(), c()]), 300)
        .expect("binds");
    assert_eq!(
        rig.host.issue_resume_ticket(plain, proof_p()),
        Err(HostError::NotResumable)
    );
}

// ---------------------------------------------------------------------------
// V2 and V3.
// ---------------------------------------------------------------------------

/// What `drive_unanswered` leaves for the checks after it.
struct Driven {
    outbox: MoveOutbox,
    /// The bytes of seq 1 to 4, in order.
    sent: Vec<Vec<u8>>,
    /// The wire bytes of seq 1's receipt, `R1`.
    r1: Vec<u8>,
}

/// V2's first half: seq 1 answered and taken; seq 2 and 3 admitted; seq 4
/// refused `NotAuthorized` behind them (`b` is not controlled). With `grace`,
/// the session is then graced at 60 and the world ticks at 70; without it, the
/// same calls with no grace (the control).
fn drive_unanswered(rig: &mut Rig, p: PeerHandle, epoch: [u8; 16], grace: bool) -> Driven {
    let mut outbox = MoveOutbox::new(epoch);
    let mut sent = Vec::new();
    let first = outbox.send(n(1), MoveBody::Halt).expect("sends");
    sent.push(bytes(&first));
    assert_eq!(
        rig.ingest_move(p, &sent[0], 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    assert_ticked_at(rig, 20);
    let r1 = receipt_bytes(rcpt(
        epoch,
        1,
        1,
        0,
        MoveVerdict::Accepted { start_tick: 1 },
    ));
    assert_eq!(
        rig.host.take_move_receipts(p).expect("a v4 session"),
        vec![r1.clone()]
    );
    let decoded = match decode_down(&r1).expect("decodes") {
        MoveDown::Receipt(receipt) => receipt,
        other => panic!("{other:?}"),
    };
    assert_eq!(outbox.receive(&decoded), Ok(()));
    for (seq, now) in [(2u64, 30u64), (3, 40)] {
        let intent = outbox.send(n(1), MoveBody::Halt).expect("sends");
        sent.push(bytes(&intent));
        assert_eq!(
            rig.ingest_move(p, &sent[seq as usize - 1], now),
            MoveAdmission::Admitted { seq }
        );
    }
    let fourth = outbox.send(n(2), MoveBody::Halt).expect("sends");
    sent.push(bytes(&fourth));
    assert_eq!(
        rig.ingest_move(p, &sent[3], 50),
        MoveAdmission::Rejected {
            seq: 4,
            code: CodeV3::NotAuthorized,
        }
    );
    assert_eq!(rig.host.take_move_receipts(p), Ok(Vec::new()));
    if grace {
        assert_eq!(rig.host.begin_grace(p, 60), Ok(30_060));
    }
    assert_ticked_at(rig, 70);
    Driven { outbox, sent, r1 }
}

/// V2 (H-2 to H-5): a grace and a resume un-admit the unanswered intents, so
/// each resend is admitted as new and receipted once; the answered intent is
/// still a cached duplicate with its original bytes.
#[test]
fn v2_unanswered_moves_are_unadmitted() {
    let (mut rig, p, epoch, _client) = ticketed_rig();
    let Driven {
        mut outbox,
        sent,
        r1,
    } = drive_unanswered(&mut rig, p, epoch, true);
    assert_eq!(
        rig.host.resume_peer(&req(epoch, 1, proof_p()), 80),
        acc(p, epoch, 2, 0)
    );
    assert_eq!(rig.host.take_move_receipts(p), Ok(Vec::new()));
    assert_eq!(rig.host.take_samples(p), Ok(Vec::new()));
    // The client resends its pending intents in order.
    assert_eq!(
        rig.ingest_move(p, &sent[1], 90),
        MoveAdmission::Admitted { seq: 2 }
    );
    assert_eq!(
        rig.ingest_move(p, &sent[2], 90),
        MoveAdmission::Admitted { seq: 3 }
    );
    assert_eq!(
        rig.ingest_move(p, &sent[3], 90),
        MoveAdmission::Rejected {
            seq: 4,
            code: CodeV3::NotAuthorized,
        }
    );
    assert_eq!(
        rig.ingest_move(p, &sent[0], 90),
        MoveAdmission::Duplicate {
            seq: 1,
            resent: true,
        }
    );
    assert_eq!(rig.host.take_move_receipts(p), Ok(vec![r1]));
    let t = rig.host.server_tick();
    assert_ticked_at(&mut rig, 100);
    let got = rig.receipts(p);
    assert_eq!(
        got,
        vec![
            rcpt(epoch, 2, 1, t, MoveVerdict::Superseded),
            rcpt(epoch, 3, 1, t, MoveVerdict::Accepted { start_tick: t + 1 }),
            rcpt(epoch, 4, 2, t, MoveVerdict::Rejected(CodeV3::NotAuthorized)),
        ]
    );
    for receipt in &got {
        assert_eq!(outbox.receive(receipt), Ok(()));
    }
    assert_eq!(outbox.pending(), &[]);

    // Control: the same calls without a grace or a resume leave seq 2 a
    // duplicate whose receipt is still queued.
    let (mut twin, tp, tepoch, _client) = ticketed_rig();
    let driven = drive_unanswered(&mut twin, tp, tepoch, false);
    assert_eq!(
        twin.ingest_move(tp, &driven.sent[1], 90),
        MoveAdmission::Duplicate {
            seq: 2,
            resent: false,
        }
    );
}

/// V3 (H-2, H-3): a released, untaken receipt is dropped by the grace and the
/// resend is answered from the lane-1 cache with the original bytes.
#[test]
fn v3_released_receipts_are_dropped_and_served_from_the_cache() {
    let (mut rig, p, epoch, _client) = ticketed_rig();
    let one = halt(epoch, 1, 1);
    assert_eq!(
        rig.ingest_move(p, &one, 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    assert_ticked_at(&mut rig, 20);
    let r1 = receipt_bytes(rcpt(
        epoch,
        1,
        1,
        0,
        MoveVerdict::Accepted { start_tick: 1 },
    ));
    assert_eq!(rig.host.begin_grace(p, 30), Ok(30_030));
    assert_eq!(
        rig.host.resume_peer(&req(epoch, 1, proof_p()), 40),
        acc(p, epoch, 2, 0)
    );
    assert_eq!(rig.host.take_move_receipts(p), Ok(Vec::new()));
    assert_eq!(
        rig.ingest_move(p, &one, 50),
        MoveAdmission::Duplicate {
            seq: 1,
            resent: true,
        }
    );
    assert_eq!(rig.host.take_move_receipts(p), Ok(vec![r1.clone()]));

    // Control: with no grace the resend is not a re-queue and the take
    // returns the one untaken receipt once.
    let (mut twin, tp, tepoch, _client) = ticketed_rig();
    assert_eq!(
        twin.ingest_move(tp, &halt(tepoch, 1, 1), 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    assert_ticked_at(&mut twin, 20);
    assert_eq!(
        twin.ingest_move(tp, &halt(tepoch, 1, 1), 50),
        MoveAdmission::Duplicate {
            seq: 1,
            resent: false,
        }
    );
    assert_eq!(twin.host.take_move_receipts(tp), Ok(vec![r1]));
    assert_eq!(twin.host.take_move_receipts(tp), Ok(Vec::new()));
}

// ---------------------------------------------------------------------------
// V4.
// ---------------------------------------------------------------------------

/// V4 (H-4, H-5, H-6): a live resume drops the held slot, fences the old
/// connection, and re-anchors every mover on the staging page.
#[test]
fn v4_a_live_v4_resume_restages_and_reanchors() {
    let (mut rig, p, epoch, mut cl) = ticketed_rig();
    let to = move_to_bytes(epoch, 1, 1, (262144, 0, 65536));
    assert_eq!(
        rig.ingest_move(p, &to, 10),
        MoveAdmission::Admitted { seq: 1 }
    );
    assert_eq!(
        rig.host.resume_peer(&req(epoch, 1, proof_p()), 20),
        acc(p, epoch, 2, 0)
    );
    assert_eq!(
        rig.host.check_frame(p, ConnectionFence::FIRST),
        Err(HostError::SessionFenced)
    );
    assert_eq!(
        rig.host
            .check_frame(p, ConnectionFence::new(2).expect("nonzero")),
        Ok(())
    );
    cl.pool.rebind_receiver(cl.rid, 20).expect("rebinds");
    cl.drain(&mut rig.host, p, 20);
    assert_eq!(cl.last_snapshot, Some(TransferId::new(2).expect("nonzero")));
    assert_positions(&cl, &start_points());
    // The slot was dropped: the resend is admitted as new, not a duplicate.
    assert_eq!(
        rig.ingest_move(p, &to, 30),
        MoveAdmission::Admitted { seq: 1 }
    );
}

// ---------------------------------------------------------------------------
// V5.
// ---------------------------------------------------------------------------

/// The V5 scenario: `p` (control `[a]`, ticket) and `q` (control `[b]`), both
/// published; `p` is graced at 10 when `grace`. `q` walks `n(2)`; the world
/// ticks at 20, 70, 120 and 170.
fn walk_while_p_waits(grace: bool) -> (Rig, PeerHandle, [u8; 16], Client) {
    let (mut rig, p, epoch, mut cl) = ticketed_rig();
    let q = rig.bind_p_control(&[b()]);
    let q_epoch = rig.epoch(q);
    rig.publish(q);
    if grace {
        assert_eq!(rig.host.begin_grace(p, 10), Ok(30_010));
    }
    let walk = move_to_bytes(q_epoch, 1, 2, (196608, 0, 262144));
    assert_eq!(
        rig.ingest_move(q, &walk, 15),
        MoveAdmission::Admitted { seq: 1 }
    );
    for now in [20, 70, 120, 170] {
        assert_ticked_at(&mut rig, now);
        // A live `p` applies each tick's sample (a tick's sample replaces the
        // last, so the stop at the third tick is read before the fourth).
        if !grace {
            for sample in rig.samples(p) {
                cl.replica.apply_sample(&sample);
            }
        }
    }
    (rig, p, epoch, cl)
}

/// V5 (H-6): the world moves on while `p` is graced, and the resume snapshot
/// anchors the new position; a graced session changes nothing in the world.
#[test]
fn v5_the_world_moves_while_a_v4_session_is_graced() {
    let end = vec![
        (1, pt(65536, 0, 65536), false),
        (2, pt(196608, 0, 262144), false),
        (3, pt(65536, 0, 327680), false),
    ];
    let (mut rig, p, epoch, mut cl) = walk_while_p_waits(true);
    assert_eq!(
        rig.host.resume_peer(&req(epoch, 1, proof_p()), 200),
        acc(p, epoch, 2, 0)
    );
    cl.pool.rebind_receiver(cl.rid, 200).expect("rebinds");
    cl.drain(&mut rig.host, p, 200);
    assert_positions(&cl, &end);

    // Control: `p` never graced, same calls otherwise.
    let (mut twin, tp, _tepoch, mut tcl) = walk_while_p_waits(false);
    assert_eq!(twin.host.authority_hash(), rig.host.authority_hash());
    tcl.drain(&mut twin.host, tp, 200);
    assert_positions(&tcl, &end);
}

// ---------------------------------------------------------------------------
// V6.
// ---------------------------------------------------------------------------

/// V6 (H-6): a v4 grace expires at its deadline and is never in a checkpoint.
#[test]
fn v6_v4_graces_expire_and_never_reach_a_checkpoint() {
    let (mut rig, p, epoch, _client) = ticketed_rig();
    let (twin, _tp, _tepoch, _tclient) = ticketed_rig();
    assert_eq!(rig.host.begin_grace(p, 100), Ok(30_100));
    assert_eq!(
        rig.host.save_checkpoint().expect("saves"),
        twin.host.save_checkpoint().expect("saves")
    );
    assert_eq!(
        rig.host.resume_peer(&req(epoch, 1, proof_p()), 30_100),
        denied(epoch)
    );
    assert_eq!(rig.host.grace_deadline(p), None);

    // Control: one millisecond earlier it resumes.
    let (mut early, ep, eepoch, _client) = ticketed_rig();
    assert_eq!(early.host.begin_grace(ep, 100), Ok(30_100));
    assert_eq!(
        early.host.resume_peer(&req(eepoch, 1, proof_p()), 30_099),
        acc(ep, eepoch, 2, 0)
    );
}

// ---------------------------------------------------------------------------
// V7 and V8: what a grace and a resume leave alone (H-2).
// ---------------------------------------------------------------------------

/// `P` spends its whole lane-1 burst (`POLICY_MOVE_INTENT_BURST` = 40) at
/// `now_ms` 10, five rounds of eight halts, each round ticked and its receipts
/// taken so that the in-flight window never refuses. The 41st intent is then
/// refused by the rate limit: the bucket really is empty.
fn deplete_move_bucket(rig: &mut Rig, p: PeerHandle, epoch: [u8; 16]) {
    for round in 0..5u64 {
        for k in 1..=8u64 {
            let seq = round * 8 + k;
            assert_eq!(
                rig.ingest_move(p, &halt(epoch, seq, 1), 10),
                MoveAdmission::Admitted { seq }
            );
        }
        assert_ticked_at(rig, 10);
        let got = rig.receipts(p);
        assert_eq!(got.len(), 8);
        assert_eq!(got[7].seq, round * 8 + 8);
    }
    assert_eq!(
        rig.ingest_move(p, &halt(epoch, 41, 1), 10),
        MoveAdmission::Refused {
            seq: Some(41),
            code: CodeV3::RateLimited,
        }
    );
}

/// V7 (H-2, M14): the move-intent bucket is unchanged by a grace and by a
/// resume. `P` is depleted, reconnects at the same millisecond, and the next
/// intent is still refused by the rate limit; one refill interval later (50 ms
/// at 20 per second is exactly one token) it is admitted, as in the control
/// that never reconnects. Both the graced and the live resume are covered.
#[test]
fn v7_the_move_bucket_is_unchanged_by_grace_and_resume() {
    // Graced, then resumed.
    let (mut rig, p, epoch, _client) = ticketed_rig();
    deplete_move_bucket(&mut rig, p, epoch);
    assert_eq!(rig.host.begin_grace(p, 10), Ok(30_010));
    assert_eq!(
        rig.host.resume_peer(&req(epoch, 1, proof_p()), 10),
        acc(p, epoch, 2, 0)
    );
    assert_eq!(
        rig.ingest_move(p, &halt(epoch, 41, 1), 10),
        MoveAdmission::Refused {
            seq: Some(41),
            code: CodeV3::RateLimited,
        }
    );
    assert_eq!(
        rig.ingest_move(p, &halt(epoch, 41, 1), 60),
        MoveAdmission::Admitted { seq: 41 }
    );

    // Live resume with no grace.
    let (mut live, lp, lepoch, _client) = ticketed_rig();
    deplete_move_bucket(&mut live, lp, lepoch);
    assert_eq!(
        live.host.resume_peer(&req(lepoch, 1, proof_p()), 10),
        acc(lp, lepoch, 2, 0)
    );
    assert_eq!(
        live.ingest_move(lp, &halt(lepoch, 41, 1), 10),
        MoveAdmission::Refused {
            seq: Some(41),
            code: CodeV3::RateLimited,
        }
    );
    assert_eq!(
        live.ingest_move(lp, &halt(lepoch, 41, 1), 60),
        MoveAdmission::Admitted { seq: 41 }
    );

    // Control: no reconnect at all gives the same two answers.
    let (mut twin, tp, tepoch, _client) = ticketed_rig();
    deplete_move_bucket(&mut twin, tp, tepoch);
    assert_eq!(
        twin.ingest_move(tp, &halt(tepoch, 41, 1), 10),
        MoveAdmission::Refused {
            seq: Some(41),
            code: CodeV3::RateLimited,
        }
    );
    assert_eq!(
        twin.ingest_move(tp, &halt(tepoch, 41, 1), 60),
        MoveAdmission::Admitted { seq: 41 }
    );
}

/// V8 (H-2, M15): the withheld set is unchanged by a grace and by a resume. `b`
/// is withheld from `P`, so `P`'s replica holds the positions of `a` and `c`
/// only; after a grace and a resume (and after a live resume) the resume
/// snapshot still anchors `a` and `c` alone. Control: the unwithheld session
/// of the V4 test anchors all three movers.
#[test]
fn v8_the_withheld_set_is_unchanged_by_grace_and_resume() {
    let without_b = vec![
        (1, pt(65536, 0, 65536), false),
        (3, pt(65536, 0, 327680), false),
    ];
    // Graced, then resumed.
    let (mut rig, p, epoch, mut cl) = ticketed_rig();
    rig.host
        .set_position_withheld(p, vec![b()])
        .expect("withholds");
    cl.drain(&mut rig.host, p, 10);
    assert_positions(&cl, &without_b);
    assert_eq!(rig.host.begin_grace(p, 20), Ok(30_020));
    assert_eq!(
        rig.host.resume_peer(&req(epoch, 1, proof_p()), 30),
        acc(p, epoch, 2, 0)
    );
    cl.pool.rebind_receiver(cl.rid, 30).expect("rebinds");
    cl.drain(&mut rig.host, p, 30);
    assert_eq!(cl.last_snapshot, Some(TransferId::new(2).expect("nonzero")));
    assert_positions(&cl, &without_b);

    // Live resume with no grace.
    let (mut live, lp, lepoch, mut lcl) = ticketed_rig();
    live.host
        .set_position_withheld(lp, vec![b()])
        .expect("withholds");
    lcl.drain(&mut live.host, lp, 10);
    assert_positions(&lcl, &without_b);
    assert_eq!(
        live.host.resume_peer(&req(lepoch, 1, proof_p()), 30),
        acc(lp, lepoch, 2, 0)
    );
    lcl.pool.rebind_receiver(lcl.rid, 30).expect("rebinds");
    lcl.drain(&mut live.host, lp, 30);
    assert_positions(&lcl, &without_b);

    // Control: with nothing withheld the same resume anchors all three.
    let (mut twin, tp, tepoch, mut tcl) = ticketed_rig();
    assert_eq!(
        twin.host.resume_peer(&req(tepoch, 1, proof_p()), 30),
        acc(tp, tepoch, 2, 0)
    );
    tcl.pool.rebind_receiver(tcl.rid, 30).expect("rebinds");
    tcl.drain(&mut twin.host, tp, 30);
    assert_positions(&tcl, &start_points());
}
