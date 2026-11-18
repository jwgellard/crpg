//! T025b-i acceptance suite: authenticated reconnect grace (`host.rs`).
//!
//! Public API only. Notation copied from `host_transfer.rs` (helpers at
//! `:42-470`, `Rig` at `:656-836`) and the v3 helpers from
//! `host_v3_session.rs` (`:56-468`). Every deadline is driven by the test's
//! `now_ms`; no test sleeps.

use crpg_core::{EntityId, Ulid};
use crpg_net::codec;
use crpg_net::codec_transfer;
use crpg_net::codec_v2;
use crpg_net::protocol::{
    self, IntentBody, IntentFrame, NetId, ReceiptStatus, RejectionCode, LANE_COMBAT,
};
use crpg_net::protocol_resume::{
    ConnectionFence, LaneClaim, LaneProgress, ResumeAccepted, ResumeProof, ResumeRejected,
    ResumeRejection, ResumeRequest,
};
use crpg_net::protocol_transfer::{
    BeginTransfer, CancelReason, ResyncReason, TransferDown, TransferId, TransferUp,
};
use crpg_net::protocol_v2::{self, DeltaOp};
use crpg_server::checkpoint::load_checkpoint;
use crpg_server::host::{
    ControlGrant, DisclosureGrants, EntityDisclosure, GrantUpdate, Host, HostConfig, HostError,
    IngestDisposition, PeerHandle, ProtocolSelection, ResumeAnswer, TransferState,
    POLICY_RECONNECT_GRACE_MS,
};
use crpg_sim::HistoryWorld;
use serde_json::json;

// ---------------------------------------------------------------------------
// Fixture identities and grants (from host_transfer.rs:42-204).
// ---------------------------------------------------------------------------

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");

const V1: ProtocolSelection = ProtocolSelection::V1;
const V2: ProtocolSelection = ProtocolSelection::V2;

/// Runs one named case body under both selected versions.
macro_rules! both_versions {
    ($name:ident) => {
        mod $name {
            #[test]
            fn v1() {
                super::$name(super::V1);
            }
            #[test]
            fn v2() {
                super::$name(super::V2);
            }
        }
    };
}

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
fn whiff() -> Ulid {
    Ulid::from_u128(605)
}
fn encounter() -> Ulid {
    Ulid::from_u128(601)
}

fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}

fn fixture() -> HistoryWorld {
    let mut history: HistoryWorld =
        serde_json::from_str(FIXTURE).expect("fixture loads through validated serde");
    history
        .acknowledge(history.last_sequence())
        .expect("acknowledges its own range");
    history
}

fn full(entity: EntityId) -> EntityDisclosure {
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

fn full_grants() -> DisclosureGrants {
    DisclosureGrants {
        entities: vec![full(a()), full(b()), full(c())],
        abilities: vec![strike(), Ulid::from_u128(604), whiff()],
        encounters: vec![encounter()],
        turn_round: true,
        encounter_end: true,
        turn_state: true,
    }
}

/// Full grants with B not present (`B_hidden`).
fn hide_b(mut grants: DisclosureGrants) -> DisclosureGrants {
    grants.entities[1].present = false;
    grants
}

fn b_hidden() -> Vec<u8> {
    vec![
        0x02, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x03, 0x01, 0x03, 0x03, 0x00, 0x01, 0x00, 0x00,
    ]
}

fn full_after_1() -> Vec<u8> {
    vec![
        0x03, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x02, 0x01, 0x07, 0x0A, 0x00, 0x03, 0x01, 0x03, 0x03,
        0x00, 0x01, 0x01, 0x02, 0x00,
    ]
}

fn control(actors: &[EntityId]) -> ControlGrant {
    ControlGrant {
        actors: actors.to_vec(),
    }
}

fn all_actors() -> ControlGrant {
    control(&[a(), b(), c()])
}

fn epoch_of(incarnation: u64, handle: u64) -> [u8; 16] {
    let mut epoch = [0u8; 16];
    epoch[..8].copy_from_slice(&incarnation.to_le_bytes());
    epoch[8..].copy_from_slice(&handle.to_le_bytes());
    epoch
}

// ---------------------------------------------------------------------------
// Expected-op constructors and decoded delivery (from host_transfer.rs).
// ---------------------------------------------------------------------------

fn legacy(op: protocol::DeltaOp) -> DeltaOp {
    DeltaOp::Legacy(op)
}

fn receipt(epoch: [u8; 16], seq: u64, tick: u64, status: ReceiptStatus) -> DeltaOp {
    legacy(protocol::DeltaOp::Receipt {
        epoch,
        lane: LANE_COMBAT,
        seq,
        processed_tick: tick,
        status,
    })
}

fn for_version(protocol: ProtocolSelection, ops: Vec<DeltaOp>) -> Vec<DeltaOp> {
    match protocol {
        ProtocolSelection::V2 => ops,
        ProtocolSelection::V1 => ops
            .into_iter()
            .filter(|op| {
                matches!(
                    op,
                    DeltaOp::Legacy(
                        protocol::DeltaOp::Receipt { .. }
                            | protocol::DeltaOp::Health { .. }
                            | protocol::DeltaOp::Turn { .. }
                            | protocol::DeltaOp::EntityEnter { .. }
                            | protocol::DeltaOp::EntityLeave { .. }
                    )
                )
            })
            .collect(),
    }
}

const APPLIED: ReceiptStatus = ReceiptStatus::Applied;

fn decode_frame(protocol: ProtocolSelection, bytes: &[u8]) -> protocol_v2::DeltaFrame {
    match protocol {
        ProtocolSelection::V1 => {
            let frame = codec::decode_delta(bytes).expect("v1 delivery decodes");
            protocol_v2::DeltaFrame {
                lane: frame.lane,
                server_tick: frame.server_tick,
                event_seq: frame.event_seq,
                ops: frame.ops.into_iter().map(DeltaOp::Legacy).collect(),
            }
        }
        ProtocolSelection::V2 => codec_v2::decode_delta(bytes).expect("v2 delivery decodes"),
    }
}

fn decode_transfer(bytes: &[u8]) -> TransferDown {
    codec_transfer::decode_down(bytes).expect("transfer frame decodes")
}

fn decode_begin(bytes: &[u8]) -> BeginTransfer {
    match decode_transfer(bytes) {
        TransferDown::Begin(begin) => begin,
        other => panic!("expected Begin, got {other:?}"),
    }
}

fn decode_chunk(bytes: &[u8]) -> Vec<u8> {
    match decode_transfer(bytes) {
        TransferDown::Chunk(chunk) => chunk.bytes,
        other => panic!("expected Chunk, got {other:?}"),
    }
}

fn decode_cancel(bytes: &[u8]) -> (TransferId, CancelReason) {
    match decode_transfer(bytes) {
        TransferDown::Cancel {
            transfer, reason, ..
        } => (transfer, reason),
        other => panic!("expected Cancel, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Harness (from host_transfer.rs:656-856).
// ---------------------------------------------------------------------------

struct Rig {
    host: Host,
    protocol: ProtocolSelection,
    now: u64,
}

impl Rig {
    fn new(protocol: ProtocolSelection) -> Self {
        let host = Host::from_history(
            fixture(),
            HostConfig {
                protocol,
                incarnation: 1,
            },
            0,
        )
        .expect("wraps an acknowledged authority");
        Self {
            host,
            protocol,
            now: 0,
        }
    }

    fn epoch(&self, peer: PeerHandle) -> [u8; 16] {
        self.host.epoch(peer).expect("bound")
    }

    fn tick(&self) -> u64 {
        self.host.server_tick()
    }

    fn bind(&mut self, control: ControlGrant, grants: DisclosureGrants) -> PeerHandle {
        self.host
            .bind_peer(control, grants, self.now)
            .expect("binds")
    }

    fn bind_snapshot(&mut self, control: ControlGrant, grants: DisclosureGrants) -> PeerHandle {
        self.host
            .bind_snapshot_peer(control, grants, self.now)
            .expect("binds")
    }

    fn advance(&mut self, ms: u64) {
        self.now += ms;
    }

    fn ingest(&mut self, peer: PeerHandle, bytes: &[u8]) {
        self.advance(100);
        assert_eq!(
            self.host.ingest(peer, bytes, self.now),
            Ok(IngestDisposition::Staged)
        );
    }

    fn pump(&mut self) -> crpg_server::host::PumpSummary {
        self.host.pump(self.now).expect("pump")
    }

    fn send(&mut self, peer: PeerHandle, bytes: &[u8]) -> crpg_server::host::AdmissionResult {
        self.ingest(peer, bytes);
        let summary = self.pump();
        assert_eq!(summary.results.len(), 1, "{summary:?}");
        summary.results[0].clone()
    }

    fn intent(&self, peer: PeerHandle, seq: u64, actor: u64, body: IntentBody) -> Vec<u8> {
        let frame = IntentFrame {
            epoch: self.epoch(peer),
            lane: LANE_COMBAT,
            seq,
            observed_tick: self.tick(),
            actor: n(actor),
            body,
        };
        match self.protocol {
            ProtocolSelection::V1 => codec::encode_intent(&frame),
            ProtocolSelection::V2 => codec_v2::encode_intent(&frame),
        }
        .expect("encodes")
    }

    fn declare(
        &self,
        peer: PeerHandle,
        seq: u64,
        actor: u64,
        ability: Ulid,
        target: u64,
    ) -> Vec<u8> {
        self.intent(
            peer,
            seq,
            actor,
            IntentBody::DeclareAction {
                ability,
                target: n(target),
            },
        )
    }

    fn entries(&mut self, peer: PeerHandle) -> Vec<crpg_server::host::DeliveryEntry> {
        self.host.take_delivery_entries(peer).expect("bound")
    }

    fn ack(&mut self, peer: PeerHandle, through: u64) {
        self.host.acknowledge_delivery(peer, through).expect("acks");
    }

    fn ack_all(&mut self, peer: PeerHandle) {
        let entries = self.entries(peer);
        if let Some(last) = entries.last() {
            let ordinal = last.ordinal;
            self.ack(peer, ordinal);
        }
    }

    #[allow(dead_code)]
    fn transfer_state(&self, peer: PeerHandle) -> Option<TransferState> {
        self.host.transfer_state(peer).expect("bound")
    }

    fn complete(&self) -> CompleteState {
        CompleteState {
            hash: self.host.authority_hash(),
            executions: self.host.executions(),
            last_capture: self.host.last_capture(),
            capture_acknowledged: self.host.capture_acknowledged(),
        }
    }

    fn evidence(&mut self, peer: PeerHandle) -> Evidence {
        let entries = self.host.take_delivery_entries(peer);
        let state = self.host.transfer_state(peer);
        Evidence {
            entries,
            state,
            complete: self.complete(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompleteState {
    hash: [u8; 32],
    executions: u64,
    last_capture: u64,
    capture_acknowledged: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Evidence {
    entries: Result<Vec<crpg_server::host::DeliveryEntry>, HostError>,
    state: Result<Option<TransferState>, HostError>,
    complete: CompleteState,
}

fn resync(epoch: [u8; 16], after: Option<u64>, reason: ResyncReason) -> Vec<u8> {
    codec_transfer::encode_up(&TransferUp::Resync {
        epoch,
        after: after.and_then(TransferId::new),
        reason,
    })
    .expect("encodes")
}

// ---------------------------------------------------------------------------
// Resume notation (tasks/T025b.md Tests).
// ---------------------------------------------------------------------------

fn e1() -> [u8; 16] {
    epoch_of(1, 1)
}

fn proof_p() -> ResumeProof {
    ResumeProof::new(vec![0xA5; 32]).expect("valid proof")
}

fn proof_p2() -> ResumeProof {
    ResumeProof::new(vec![0xB7; 32]).expect("valid proof")
}

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

fn denied(epoch: [u8; 16]) -> Result<ResumeAnswer, HostError> {
    Ok(ResumeAnswer::Rejected(ResumeRejected {
        epoch,
        reason: ResumeRejection::Denied,
    }))
}

/// `S(protocol)`: handle 1 bound snapshot, ticket `P`, acked, one applied
/// strike at `now` 100.
fn s(protocol: ProtocolSelection) -> (Rig, PeerHandle, Vec<u8>) {
    let mut rig = Rig::new(protocol);
    let q = rig.bind_snapshot(all_actors(), full_grants());
    rig.host.issue_resume_ticket(q, proof_p()).expect("ticket");
    rig.ack_all(q);
    let bytes = rig.declare(q, 1, 1, strike(), 2);
    let result = rig.send(q, &bytes);
    assert_eq!(result.status, APPLIED);
    (rig, q, bytes)
}

// ---------------------------------------------------------------------------
// v3 helpers (from host_v3_session.rs:56-468, renamed to avoid clashes).
// ---------------------------------------------------------------------------

use crpg_net::facts_v3::EncounterGrants;
use crpg_net::pool_v3::{
    GateMode, ReassemblyPoolV3, ReceiverIdV3, ReplicaUpdateV3, TransferLimitsV3,
};
use crpg_net::protocol_v3::SessionEpoch as SessionEpochV3;
use crpg_net::protocol_v3::{DeltaFrame as DeltaFrameV3, PauseRights};
use crpg_net::replica_v3::ReplicaWorldV3;
use crpg_net::transfer_v3;
use crpg_server::host::{EntityFactGrant, FactGrants, DEFAULT_HOST_GRANTS};

fn tb3_value() -> serde_json::Value {
    serde_json::from_str(FIXTURE).expect("fixture parses")
}

fn history_of_v3(value: &serde_json::Value) -> HistoryWorld {
    let mut history: HistoryWorld =
        serde_json::from_value(value.clone()).expect("fixture loads through validated serde");
    history
        .acknowledge(history.last_sequence())
        .expect("acknowledges its own range");
    history
}

fn config_v2() -> HostConfig {
    HostConfig {
        protocol: ProtocolSelection::V2,
        incarnation: 1,
    }
}

fn host_of_v3(value: &serde_json::Value) -> Host {
    Host::from_history(history_of_v3(value), config_v2(), 0).expect("wraps")
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

fn facts_v3() -> FactGrants {
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

fn full_grants_v3(entities: &[EntityId]) -> DisclosureGrants {
    DisclosureGrants {
        entities: entities.iter().copied().map(full).collect(),
        abilities: vec![strike(), Ulid::from_u128(604), whiff()],
        encounters: vec![encounter()],
        turn_round: true,
        encounter_end: true,
        turn_state: true,
    }
}

fn r1() -> PauseRights {
    PauseRights {
        pause: true,
        unpause: true,
    }
}

fn bind_p_v3(host: &mut Host, now: u64) -> PeerHandle {
    host.bind_v3_peer(
        control(&[a()]),
        full_grants_v3(&[a(), b(), c()]),
        facts_v3(),
        r1(),
        now,
    )
    .expect("binds")
}

struct ClientV3 {
    pool: ReassemblyPoolV3,
    rid: ReceiverIdV3,
    replica: ReplicaWorldV3,
    last_snapshot: Option<TransferId>,
}

impl ClientV3 {
    fn new(epoch: [u8; 16]) -> Self {
        let mut pool = ReassemblyPoolV3::new(TransferLimitsV3::v3()).expect("v3 limits");
        let rid = pool
            .add_receiver(epoch, GateMode::AwaitSnapshot)
            .expect("receiver");
        Self {
            pool,
            rid,
            replica: ReplicaWorldV3::new(SessionEpochV3(epoch)),
            last_snapshot: None,
        }
    }

    fn drain(&mut self, host: &mut Host, peer: PeerHandle, now: u64) -> DrainedV3 {
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
        DrainedV3 { entries, deltas }
    }
}

#[allow(dead_code)]
struct DrainedV3 {
    entries: Vec<crpg_server::host::DeliveryEntry>,
    deltas: Vec<DeltaFrameV3>,
}

fn decoded_v3(entry: &crpg_server::host::DeliveryEntry) -> TransferDown {
    transfer_v3::decode_down(&entry.bytes).expect("a 0x82 frame")
}

fn is_begin_of(entry: &crpg_server::host::DeliveryEntry, transfer: u64) -> bool {
    matches!(decoded_v3(entry), TransferDown::Begin(begin) if begin.transfer == TransferId::new(transfer).expect("nonzero"))
}

fn is_cancel(
    entry: &crpg_server::host::DeliveryEntry,
    epoch: [u8; 16],
    transfer: u64,
    why: CancelReason,
) -> bool {
    decoded_v3(entry)
        == TransferDown::Cancel {
            epoch,
            transfer: TransferId::new(transfer).expect("nonzero"),
            reason: why,
        }
}

fn shape_v3(
    entries: &[crpg_server::host::DeliveryEntry],
) -> Vec<(u64, crpg_server::host::DeliveryKind)> {
    entries
        .iter()
        .map(|entry| (entry.ordinal, entry.kind))
        .collect()
}

// ---------------------------------------------------------------------------
// G1 ticket_binds_the_proof (G-2).
// ---------------------------------------------------------------------------

#[test]
fn ticket_binds_the_proof() {
    assert_eq!(POLICY_RECONNECT_GRACE_MS, 30_000);
    let mut rig = Rig::new(V2);
    let q = rig.bind_snapshot(all_actors(), full_grants());
    let ticket = rig.host.issue_resume_ticket(q, proof_p()).expect("ticket");
    let mut expected = vec![0x91, 0x03, 0x30];
    expected.extend_from_slice(&e1());
    expected.push(0x20);
    expected.extend_from_slice(&[0xA5; 32]);
    assert_eq!(ticket.len(), 52);
    assert_eq!(ticket, expected);
    // A second ticket replaces the proof.
    rig.host
        .issue_resume_ticket(q, proof_p2())
        .expect("second ticket");
    assert_eq!(rig.host.begin_grace(q, 100), Ok(30_100));
    assert_eq!(
        rig.host.resume_peer(&req(e1(), 1, proof_p()), 200),
        denied(e1())
    );
    assert_eq!(
        rig.host.resume_peer(&req(e1(), 1, proof_p2()), 300),
        acc(q, e1(), 2, 0)
    );
    // The proof never reaches Debug or a checkpoint.
    let debug = format!("{:?}", rig.host);
    assert!(debug.contains("ResumeProof(<redacted>)"));
    assert!(!debug.contains("165, 165"));
    let mut twin = Rig::new(V2);
    let _ = twin.bind_snapshot(all_actors(), full_grants());
    assert_eq!(
        rig.host.save_checkpoint().expect("saves"),
        twin.host.save_checkpoint().expect("saves")
    );
    // Precedence: an unbound handle -> UnknownPeer.
    let mut rig2 = Rig::new(V2);
    let q2 = rig2.bind_snapshot(all_actors(), full_grants());
    let ghost = rig2.bind(control(&[]), full_grants());
    rig2.host.unbind_peer(ghost);
    assert_eq!(
        rig2.host.issue_resume_ticket(ghost, proof_p()),
        Err(HostError::UnknownPeer)
    );
    // A bind_peer session is not resumable.
    let lane = rig2.bind(control(&[]), full_grants());
    assert_eq!(
        rig2.host.issue_resume_ticket(lane, proof_p()),
        Err(HostError::NotResumable)
    );
    // A graced handle is UnknownPeer.
    rig2.host
        .issue_resume_ticket(q2, proof_p())
        .expect("ticket");
    rig2.host.begin_grace(q2, 0).expect("grace");
    assert_eq!(
        rig2.host.issue_resume_ticket(q2, proof_p()),
        Err(HostError::UnknownPeer)
    );
    // After shutdown -> Closed.
    rig2.host.shutdown();
    assert_eq!(
        rig2.host.issue_resume_ticket(lane, proof_p()),
        Err(HostError::Closed)
    );
}

// ---------------------------------------------------------------------------
// G2 lost_receipt_survives_the_grace (G-3, G-6, G-7, G-8; both versions).
// ---------------------------------------------------------------------------

fn lost_receipt_survives_the_grace(protocol: ProtocolSelection) {
    let (mut rig, q, bytes) = s(protocol);
    assert_eq!(rig.host.begin_grace(q, 100), Ok(30_100));
    assert_eq!(rig.host.grace_deadline(q), Some(30_100));
    assert_eq!(rig.host.epoch(q), Err(HostError::UnknownPeer));
    assert_eq!(
        rig.host.resume_peer(&req(e1(), 1, proof_p()), 30_099),
        acc(q, e1(), 2, 1)
    );
    assert_eq!(rig.host.grace_deadline(q), None);
    let entries = rig.host.take_delivery_entries(q).expect("bound");
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![
            (3, crpg_server::host::DeliveryKind::Transfer),
            (4, crpg_server::host::DeliveryKind::Transfer)
        ]
    );
    let begin = decode_begin(&entries[0].bytes);
    assert_eq!(begin.epoch, e1());
    assert_eq!(begin.transfer, TransferId::new(1).expect("nonzero"));
    assert_eq!(begin.cursor, 2);
    assert_eq!(begin.base_tick, rig.host.server_tick());
    assert_eq!(decode_chunk(&entries[1].bytes), full_after_1());
    assert_eq!(
        rig.host.transfer_state(q),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: Some(1),
        }))
    );
    // Resend the seq-1 bytes: cached APPLIED, no new execution.
    rig.advance(99);
    assert_eq!(
        rig.host.ingest(q, &bytes, 30_199),
        Ok(IngestDisposition::Staged)
    );
    let summary = rig.host.pump(30_199).expect("pump");
    assert_eq!(summary.results.len(), 1);
    assert_eq!(summary.results[0].status, APPLIED);
    assert!(summary.results[0].cached);
    assert_eq!(rig.host.executions(), 1);
    let entries = rig.host.take_delivery_entries(q).expect("bound");
    let last = entries.last().expect("a receipt delta");
    assert_eq!(last.ordinal, 5);
    assert_eq!(last.kind, crpg_server::host::DeliveryKind::Delta);
    let frame = decode_frame(protocol, &last.bytes);
    assert_eq!(frame.event_seq, 3);
    assert_eq!(
        frame.ops,
        for_version(protocol, vec![receipt(e1(), 1, 0, APPLIED)])
    );
    // Controls: unbind twin is Denied; never-disconnected twin is cached too.
    let (mut unbound, _, _) = s(protocol);
    unbound.host.unbind_peer(q);
    assert_eq!(
        unbound.host.resume_peer(&req(e1(), 1, proof_p()), 200),
        denied(e1())
    );
    let (mut live, lq, lbytes) = s(protocol);
    live.advance(99);
    assert_eq!(
        live.host.ingest(lq, &lbytes, 30_199),
        Ok(IngestDisposition::Staged)
    );
    let summary = live.host.pump(30_199).expect("pump");
    assert!(summary.results[0].cached);
}

both_versions!(lost_receipt_survives_the_grace);

// ---------------------------------------------------------------------------
// G3 the_boundary_is_exact (G-5, G-6).
// ---------------------------------------------------------------------------

#[test]
fn the_boundary_is_exact() {
    // (a) Denied at 30_098 keeps the grace; Accepted at 30_099.
    let (mut rig, q, _) = s(V2);
    rig.host.begin_grace(q, 100).expect("grace");
    assert_eq!(
        rig.host.resume_peer(&req(e1(), 1, proof_p2()), 30_098),
        denied(e1())
    );
    assert_eq!(rig.host.grace_deadline(q), Some(30_100));
    assert_eq!(
        rig.host.resume_peer(&req(e1(), 1, proof_p()), 30_099),
        acc(q, e1(), 2, 1)
    );
    // (b) Twin: wrong proof at 30_099 is Denied; correct proof at 30_100 is
    // Denied by expiry; the seat is free.
    let (mut rig, q, _) = s(V2);
    rig.host.begin_grace(q, 100).expect("grace");
    assert_eq!(
        rig.host.resume_peer(&req(e1(), 1, proof_p2()), 30_099),
        denied(e1())
    );
    assert_eq!(
        rig.host.resume_peer(&req(e1(), 1, proof_p()), 30_100),
        denied(e1())
    );
    assert_eq!(rig.host.grace_deadline(q), None);
    let h2 = rig
        .host
        .bind_snapshot_peer(control(&[]), full_grants(), 30_100)
        .expect("seat free");
    assert_eq!(h2.get(), 2);
    // (c) Twin: the deadline reads before the sweep; the pump sweeps it.
    let (mut rig, q, _) = s(V2);
    rig.host.begin_grace(q, 100).expect("grace");
    assert_eq!(rig.host.grace_deadline(q), Some(30_100));
    rig.host.pump(30_100).expect("pump");
    assert_eq!(rig.host.grace_deadline(q), None);
}

// ---------------------------------------------------------------------------
// G4 forged_and_misdirected_requests (G-6).
// ---------------------------------------------------------------------------

#[test]
fn forged_and_misdirected_requests() {
    let (mut rig, q, _) = s(V2);
    let q2 = rig.bind_snapshot(control(&[]), full_grants());
    rig.host
        .issue_resume_ticket(q2, proof_p2())
        .expect("ticket");
    rig.host.begin_grace(q, 100).expect("grace");
    let before = rig.evidence(q2);
    let bad_proof = ResumeProof::new(vec![0xA6; 32]).expect("valid");
    let short = ResumeProof::new(vec![0xA5; 16]).expect("valid");
    let cases: Vec<ResumeRequest> = vec![
        req(e1(), 1, bad_proof),
        req(e1(), 1, short),
        req(e1(), 1, proof_p2()),
        req(epoch_of(1, 2), 1, proof_p()),
        req(epoch_of(1, 99), 1, proof_p()),
        req(epoch_of(2, 1), 1, proof_p()),
        req(e1(), 2, proof_p()),
    ];
    for bad in &cases {
        assert_eq!(
            rig.host.resume_peer(bad, 200),
            Ok(ResumeAnswer::Rejected(ResumeRejected {
                epoch: bad.epoch,
                reason: ResumeRejection::Denied,
            })),
            "request {bad:?}"
        );
        assert_eq!(rig.evidence(q2), before, "evidence unchanged");
    }
    // Control: the correct request still resumes with granted 2.
    assert_eq!(
        rig.host.resume_peer(&req(e1(), 1, proof_p()), 200),
        acc(q, e1(), 2, 1)
    );
}

// ---------------------------------------------------------------------------
// G5 frozen_during_grace (G-3, G-4, G-8).
// ---------------------------------------------------------------------------

#[test]
fn frozen_during_grace() {
    let mut rig = Rig::new(V2);
    let q = rig.bind_snapshot(all_actors(), full_grants());
    rig.host.issue_resume_ticket(q, proof_p()).expect("ticket");
    rig.ack_all(q);
    let bytes = rig.declare(q, 1, 1, strike(), 2);
    rig.host.ingest(q, &bytes, 100).expect("staged");
    rig.host.begin_grace(q, 100).expect("grace");
    let summary = rig.host.pump(100).expect("pump");
    assert_eq!(summary.admitted, 0);
    assert_eq!(rig.host.executions(), 0);
    // Frozen: every per-peer method answers as for an unbound handle.
    assert_eq!(
        rig.host.ingest(q, &bytes, 200),
        Ok(IngestDisposition::Refused {
            status: ReceiptStatus::Rejected(RejectionCode::Unauthenticated),
        })
    );
    assert_eq!(
        rig.host
            .ingest_transfer(q, &resync(e1(), None, ResyncReason::Overflow), 200),
        Ok(crpg_server::host::TransferDisposition::Refused(
            crpg_server::host::TransferRefusal::Unbound
        ))
    );
    assert!(rig.host.take_delivery_entries(q).is_err());
    assert_eq!(rig.host.transfer_state(q), Err(HostError::UnknownPeer));
    assert_eq!(rig.host.epoch(q), Err(HostError::UnknownPeer));
    assert_eq!(rig.host.session_wire(q), Err(HostError::UnknownPeer));
    assert!(rig.host.set_grants(q, full_grants()).is_err());
    assert!(rig.host.set_control(q, control(&[])).is_err());
    assert!(rig.host.acknowledge_delivery(q, 1).is_err());
    assert_eq!(
        rig.host.check_frame(q, ConnectionFence::FIRST),
        Err(HostError::UnknownPeer)
    );
    assert_eq!(
        rig.host.issue_resume_ticket(q, proof_p()),
        Err(HostError::UnknownPeer)
    );
    assert_eq!(rig.host.begin_grace(q, 200), Err(HostError::UnknownPeer));
    // Resume admits the purged bytes as new (finalized_through 0).
    assert_eq!(
        rig.host.resume_peer(&req(e1(), 1, proof_p()), 300),
        acc(q, e1(), 2, 0)
    );
    assert_eq!(
        rig.host.ingest(q, &bytes, 300),
        Ok(IngestDisposition::Staged)
    );
    let summary = rig.host.pump(300).expect("pump");
    assert_eq!(summary.results.len(), 1);
    assert_eq!(summary.results[0].status, APPLIED);
    assert!(!summary.results[0].cached);
    assert_eq!(rig.host.executions(), 1);
}

// ---------------------------------------------------------------------------
// G6 live_resume_moves_the_fence (G-1, G-7).
// ---------------------------------------------------------------------------

#[test]
fn live_resume_moves_the_fence() {
    let (mut rig, q, _) = s(V2);
    assert_eq!(rig.host.check_frame(q, ConnectionFence::FIRST), Ok(()));
    assert_eq!(
        rig.host.resume_peer(&req(e1(), 1, proof_p()), 200),
        acc(q, e1(), 2, 1)
    );
    assert_eq!(rig.host.epoch(q), Ok(e1()));
    assert_eq!(
        rig.host.check_frame(q, ConnectionFence::FIRST),
        Err(HostError::SessionFenced)
    );
    assert_eq!(
        rig.host
            .check_frame(q, ConnectionFence::new(3).expect("nonzero")),
        Err(HostError::SessionFenced)
    );
    assert_eq!(
        rig.host
            .check_frame(q, ConnectionFence::new(2).expect("nonzero")),
        Ok(())
    );
    let entries = rig.host.take_delivery_entries(q).expect("bound");
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![
            (3, crpg_server::host::DeliveryKind::Transfer),
            (4, crpg_server::host::DeliveryKind::Transfer)
        ]
    );
    let begin = decode_begin(&entries[0].bytes);
    assert_eq!(begin.transfer, TransferId::new(1).expect("nonzero"));
    assert_eq!(begin.cursor, 2);
}

// ---------------------------------------------------------------------------
// G7 racing_resumes_never_share_a_grant (G-1, G-6).
// ---------------------------------------------------------------------------

#[test]
fn racing_resumes_never_share_a_grant() {
    let (mut rig, q, _) = s(V2);
    rig.host.begin_grace(q, 100).expect("grace");
    let request = req(e1(), 1, proof_p());
    assert_eq!(rig.host.resume_peer(&request, 200), acc(q, e1(), 2, 1));
    assert_eq!(rig.host.resume_peer(&request, 300), acc(q, e1(), 3, 1));
    let entries = rig.host.take_delivery_entries(q).expect("bound");
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![
            (5, crpg_server::host::DeliveryKind::Transfer),
            (6, crpg_server::host::DeliveryKind::Transfer),
            (7, crpg_server::host::DeliveryKind::Transfer),
        ]
    );
    assert_eq!(
        decode_cancel(&entries[0].bytes),
        (
            TransferId::new(1).expect("nonzero"),
            CancelReason::Superseded
        )
    );
    let begin = decode_begin(&entries[1].bytes);
    assert_eq!(begin.transfer, TransferId::new(2).expect("nonzero"));
    assert_eq!(begin.cursor, 2);
    assert_eq!(
        rig.host.transfer_state(q),
        Ok(Some(TransferState {
            next_transfer: 3,
            in_flight: Some(2),
        }))
    );
    assert_eq!(
        rig.host
            .check_frame(q, ConnectionFence::new(2).expect("nonzero")),
        Err(HostError::SessionFenced)
    );
    assert_eq!(
        rig.host
            .check_frame(q, ConnectionFence::new(3).expect("nonzero")),
        Ok(())
    );
    // A stale fence never resumes again; the confirmed one does.
    assert_eq!(rig.host.resume_peer(&request, 400), denied(e1()));
    assert_eq!(
        rig.host.resume_peer(&req(e1(), 2, proof_p()), 400),
        denied(e1())
    );
    assert_eq!(
        rig.host.resume_peer(&req(e1(), 3, proof_p()), 500),
        acc(q, e1(), 4, 1)
    );
}

// ---------------------------------------------------------------------------
// G8 the_snapshot_uses_current_grants (G-7; both versions).
// ---------------------------------------------------------------------------

fn the_snapshot_uses_current_grants(protocol: ProtocolSelection) {
    let mut rig = Rig::new(protocol);
    let w = rig.bind_snapshot(control(&[]), full_grants());
    let p = rig.bind(all_actors(), full_grants());
    rig.host.issue_resume_ticket(w, proof_p()).expect("ticket");
    rig.ack_all(p);
    let script = rig.declare(p, 1, 1, strike(), 2);
    assert_eq!(rig.send(p, &script).status, APPLIED);
    assert_eq!(
        rig.host.set_grants(w, hide_b(full_grants())),
        Ok(GrantUpdate::SnapshotStaged)
    );
    let now = rig.now;
    rig.host.begin_grace(w, now).expect("grace");
    assert_eq!(
        rig.host
            .resume_peer(&req(epoch_of(1, 1), 1, proof_p()), now + 1),
        acc(w, epoch_of(1, 1), 2, 0)
    );
    let entries = rig.host.take_delivery_entries(w).expect("bound");
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![
            (5, crpg_server::host::DeliveryKind::Transfer),
            (6, crpg_server::host::DeliveryKind::Transfer),
            (7, crpg_server::host::DeliveryKind::Transfer),
        ]
    );
    assert_eq!(
        decode_cancel(&entries[0].bytes),
        (
            TransferId::new(1).expect("nonzero"),
            CancelReason::Superseded
        )
    );
    let begin = decode_begin(&entries[1].bytes);
    assert_eq!(begin.transfer, TransferId::new(2).expect("nonzero"));
    assert_eq!(begin.cursor, 2);
    assert_eq!(decode_chunk(&entries[2].bytes), b_hidden());
    assert_eq!(
        rig.host.transfer_state(w),
        Ok(Some(TransferState {
            next_transfer: 3,
            in_flight: Some(2),
        }))
    );
    // Control: without narrowing the resume replays the full body.
    let mut plain = Rig::new(protocol);
    let w2 = plain.bind_snapshot(control(&[]), full_grants());
    let p2 = plain.bind(all_actors(), full_grants());
    plain
        .host
        .issue_resume_ticket(w2, proof_p())
        .expect("ticket");
    plain.ack_all(p2);
    let script2 = plain.declare(p2, 1, 1, strike(), 2);
    assert_eq!(plain.send(p2, &script2).status, APPLIED);
    let now2 = plain.now;
    plain.host.begin_grace(w2, now2).expect("grace");
    assert_eq!(
        plain
            .host
            .resume_peer(&req(epoch_of(1, 1), 1, proof_p()), now2 + 1),
        acc(w2, epoch_of(1, 1), 2, 0)
    );
    let entries = plain.host.take_delivery_entries(w2).expect("bound");
    assert_eq!(entries.len(), 2);
    assert_eq!(decode_chunk(&entries[1].bytes), full_after_1());
}

both_versions!(the_snapshot_uses_current_grants);

// ---------------------------------------------------------------------------
// G9 v3_resume_keeps_its_views (G-7).
// ---------------------------------------------------------------------------

#[test]
fn v3_resume_keeps_its_views() {
    let mut host = host_of_v3(&tb3_value());
    let p = bind_p_v3(&mut host, 100);
    host.issue_resume_ticket(p, proof_p()).expect("ticket");
    host.set_grants(p, full_grants_v3(&[a(), c()]))
        .expect("narrow");
    let epoch = host.epoch(p).expect("bound");
    let mut client = ClientV3::new(epoch);
    let drained = client.drain(&mut host, p, 100);
    let payload_n = drained
        .entries
        .iter()
        .filter(|entry| entry.kind == crpg_server::host::DeliveryKind::Transfer)
        .filter_map(|entry| match decoded_v3(entry) {
            TransferDown::Chunk(chunk) => Some(chunk.bytes.clone()),
            _ => None,
        })
        .next_back()
        .expect("a chunk was staged");
    host.begin_grace(p, 100).expect("grace");
    assert_eq!(
        host.resume_peer(&req(epoch_of(1, 1), 1, proof_p()), 200),
        acc(p, epoch_of(1, 1), 2, 0)
    );
    let entries = host.take_delivery_entries(p).expect("bound");
    assert_eq!(
        shape_v3(&entries),
        vec![
            (8, crpg_server::host::DeliveryKind::Transfer),
            (9, crpg_server::host::DeliveryKind::Transfer),
            (10, crpg_server::host::DeliveryKind::Transfer),
            (11, crpg_server::host::DeliveryKind::Delta),
        ]
    );
    assert!(is_cancel(
        &entries[0],
        epoch_of(1, 1),
        2,
        CancelReason::Superseded
    ));
    assert!(is_begin_of(&entries[1], 3));
    assert_eq!(
        match decoded_v3(&entries[2]) {
            TransferDown::Chunk(chunk) => chunk.bytes,
            other => panic!("expected Chunk, got {other:?}"),
        },
        payload_n
    );
    // The delta carries event_seq 3.
    let delta = crpg_net::codec_v3::decode_delta(&entries[3].bytes).expect("v3 delta decodes");
    assert_eq!(delta.event_seq, 3);
    assert_eq!(
        host.transfer_state(p),
        Ok(Some(TransferState {
            next_transfer: 4,
            in_flight: Some(3),
        }))
    );
}

// ---------------------------------------------------------------------------
// G10 the_world_continues (G-9; V2).
// ---------------------------------------------------------------------------

#[test]
fn the_world_continues() {
    let mut rig = Rig::new(V2);
    let q = rig.bind_snapshot(control(&[]), full_grants());
    rig.host.issue_resume_ticket(q, proof_p()).expect("ticket");
    rig.ack_all(q);
    rig.host.begin_grace(q, 0).expect("grace");
    let p = rig.bind(all_actors(), full_grants());
    let bytes = rig.declare(p, 1, 1, strike(), 2);
    assert_eq!(rig.send(p, &bytes).status, APPLIED);
    let first = rig.host.advance(200).expect("advance");
    let second = rig.host.advance(5_000).expect("advance");
    assert_eq!(
        rig.host.resume_peer(&req(e1(), 1, proof_p()), 6_000),
        acc(q, e1(), 2, 0)
    );
    let entries = rig.host.take_delivery_entries(q).expect("bound");
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![
            (2, crpg_server::host::DeliveryKind::Transfer),
            (3, crpg_server::host::DeliveryKind::Transfer),
        ]
    );
    let begin = decode_begin(&entries[0].bytes);
    assert_eq!(begin.transfer, TransferId::new(1).expect("nonzero"));
    assert_eq!(begin.cursor, 1);
    assert_eq!(begin.base_tick, rig.host.server_tick());
    // Twin without a grace makes the same calls, then resyncs.
    let mut twin = Rig::new(V2);
    let tq = twin.bind_snapshot(control(&[]), full_grants());
    twin.ack_all(tq);
    let tp = twin.bind(all_actors(), full_grants());
    let tbytes = twin.declare(tp, 1, 1, strike(), 2);
    assert_eq!(twin.send(tp, &tbytes).status, APPLIED);
    let tfirst = twin.host.advance(200).expect("advance");
    let tsecond = twin.host.advance(5_000).expect("advance");
    assert_eq!(first, tfirst);
    assert_eq!(second, tsecond);
    assert_eq!(
        twin.host.ingest_transfer(
            tq,
            &resync(twin.epoch(tq), None, ResyncReason::Overflow),
            6_000
        ),
        Ok(crpg_server::host::TransferDisposition::Resynced)
    );
    assert_eq!(rig.host.authority_hash(), twin.host.authority_hash());
    assert_eq!(rig.host.executions(), twin.host.executions());
    assert_eq!(rig.host.server_tick(), twin.host.server_tick());
    assert_eq!(rig.host.awaiting_orders(), twin.host.awaiting_orders());
    let resumed = rig.host.take_delivery_entries(q).expect("bound");
    let resynced = twin.host.take_delivery_entries(tq).expect("bound");
    assert_eq!(
        decode_chunk(&resumed[1].bytes),
        decode_chunk(&resynced[1].bytes)
    );
}

// ---------------------------------------------------------------------------
// G11 budgets_are_not_refilled (G-10).
// ---------------------------------------------------------------------------

#[test]
fn budgets_are_not_refilled() {
    let mut rig = Rig::new(V2);
    let q = rig.bind_snapshot(all_actors(), full_grants());
    rig.host.issue_resume_ticket(q, proof_p()).expect("ticket");
    for _ in 0..80 {
        assert_eq!(
            rig.host.ingest(q, &[0x00], 100),
            Ok(IngestDisposition::Staged)
        );
    }
    assert_eq!(
        rig.host.ingest(q, &[0x00], 100),
        Ok(IngestDisposition::Refused {
            status: ReceiptStatus::Rejected(RejectionCode::RateLimited),
        })
    );
    rig.host.begin_grace(q, 100).expect("grace");
    assert_eq!(
        rig.host.resume_peer(&req(e1(), 1, proof_p()), 100),
        acc(q, e1(), 2, 0)
    );
    assert_eq!(
        rig.host.ingest(q, &[0x00], 100),
        Ok(IngestDisposition::Refused {
            status: ReceiptStatus::Rejected(RejectionCode::RateLimited),
        })
    );
    // Control: a fresh binding at the same time admits.
    let mut twin = Rig::new(V2);
    let tq = twin.bind_snapshot(all_actors(), full_grants());
    twin.host.unbind_peer(tq);
    let fresh = twin
        .host
        .bind_snapshot_peer(all_actors(), full_grants(), 100)
        .expect("binds");
    assert_eq!(
        twin.host.ingest(fresh, &[0x00], 100),
        Ok(IngestDisposition::Staged)
    );
}

// ---------------------------------------------------------------------------
// G12 graces_hold_seats (G-4, G-5).
// ---------------------------------------------------------------------------

#[test]
fn graces_hold_seats() {
    let mut rig = Rig::new(V2);
    let mut handles = Vec::new();
    for i in 0..8u8 {
        let peer = rig.bind_snapshot(control(&[]), full_grants());
        rig.host
            .issue_resume_ticket(peer, ResumeProof::new(vec![0x10 + i; 32]).expect("proof"))
            .expect("ticket");
        handles.push(peer);
    }
    assert_eq!(
        handles.iter().map(|h| h.get()).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6, 7, 8]
    );
    for peer in &handles {
        assert_eq!(rig.host.begin_grace(*peer, 100), Ok(30_100));
    }
    assert_eq!(
        rig.host.bind_peer(control(&[]), full_grants(), 100),
        Err(HostError::ServerBusy)
    );
    assert_eq!(
        rig.host.bind_peer(control(&[]), full_grants(), 30_099),
        Err(HostError::ServerBusy)
    );
    let ninth = rig
        .host
        .bind_peer(control(&[]), full_grants(), 30_100)
        .expect("seat free");
    assert_eq!(ninth.get(), 9);
    for peer in &handles {
        assert_eq!(rig.host.grace_deadline(*peer), None);
    }
}

// ---------------------------------------------------------------------------
// G13 not_resumable_and_precedence (G-1, G-2, G-3).
// ---------------------------------------------------------------------------

#[test]
fn not_resumable_and_precedence() {
    let mut rig = Rig::new(V2);
    // A bind_peer session: ticket and grace are NotResumable.
    let lane = rig.bind(control(&[]), full_grants());
    assert_eq!(
        rig.host.issue_resume_ticket(lane, proof_p()),
        Err(HostError::NotResumable)
    );
    assert_eq!(rig.host.begin_grace(lane, 0), Err(HostError::NotResumable));
    assert!(rig.host.epoch(lane).is_ok());
    assert_eq!(rig.host.check_frame(lane, ConnectionFence::FIRST), Ok(()));
    assert_eq!(
        rig.host.resume_peer(&req(rig.epoch(lane), 1, proof_p()), 0),
        denied(rig.epoch(lane))
    );
    // A snapshot session without a ticket: grace is NotResumable.
    let bare = rig.bind_snapshot(control(&[]), full_grants());
    assert_eq!(rig.host.begin_grace(bare, 0), Err(HostError::NotResumable));
    // A v4 session: ticket is NotResumable.
    let mut rig_v4 = Rig::new(V2);
    let v4 = rig_v4
        .host
        .bind_v4_peer(control(&[a()]), full_grants(), facts_v3(), r1(), 0)
        .expect("v4 binds");
    assert_eq!(
        rig_v4.host.issue_resume_ticket(v4, proof_p()),
        Err(HostError::NotResumable)
    );
    // begin_grace on an unbound handle -> UnknownPeer.
    let retired = rig.bind(control(&[]), full_grants());
    rig.host.unbind_peer(retired);
    assert_eq!(
        rig.host.begin_grace(retired, 0),
        Err(HostError::UnknownPeer)
    );
    // Time regression changes nothing.
    rig.advance(100);
    let _ = rig.bind(control(&[]), full_grants());
    rig.now -= 1;
    let before = rig.complete();
    assert_eq!(
        rig.host.begin_grace(lane, rig.now),
        Err(HostError::TimeRegression)
    );
    assert_eq!(
        rig.host
            .resume_peer(&req(rig.epoch(lane), 1, proof_p()), rig.now),
        Err(HostError::TimeRegression)
    );
    assert_eq!(rig.complete(), before);
    // After shutdown everything is Closed.
    rig.host.shutdown();
    assert_eq!(rig.host.begin_grace(lane, 200), Err(HostError::Closed));
    assert_eq!(
        rig.host.resume_peer(&req(e1(), 1, proof_p()), 200),
        Err(HostError::Closed)
    );
    assert_eq!(
        rig.host.issue_resume_ticket(lane, proof_p()),
        Err(HostError::Closed)
    );
    assert_eq!(
        rig.host.check_frame(lane, ConnectionFence::FIRST),
        Err(HostError::Closed)
    );
    // The two Display texts are exact.
    assert_eq!(
        HostError::NotResumable.to_string(),
        "NotResumable at host/admission"
    );
    assert_eq!(
        HostError::SessionFenced.to_string(),
        "SessionFenced at host/admission"
    );
}

// ---------------------------------------------------------------------------
// G14 restart_unbind_and_shutdown_drop_graces (G-11, G-12).
// ---------------------------------------------------------------------------

#[test]
fn restart_unbind_and_shutdown_drop_graces() {
    let (mut rig, q, bytes) = s(V2);
    rig.host.begin_grace(q, 100).expect("grace");
    let twin = {
        let (twin, _, _) = s(V2);
        twin
    };
    assert_eq!(
        rig.host.save_checkpoint().expect("saves"),
        twin.host.save_checkpoint().expect("never-graced saves")
    );
    let saved = rig.host.save_checkpoint().expect("saves");
    let mut loaded = load_checkpoint(&saved, 2, 200).expect("loads");
    assert_eq!(loaded.grace_deadline(q), None);
    assert_eq!(
        loaded.resume_peer(&req(e1(), 1, proof_p()), 200),
        denied(e1())
    );
    let fresh = loaded
        .bind_snapshot_peer(all_actors(), full_grants(), 200)
        .expect("binds");
    assert_eq!(loaded.epoch(fresh), Ok(epoch_of(2, 1)));
    assert_eq!(
        loaded.ingest(fresh, &bytes, 200),
        Ok(IngestDisposition::Staged)
    );
    let summary = loaded.pump(200).expect("pump");
    assert_eq!(summary.results.len(), 1);
    assert_eq!(
        summary.results[0].status,
        ReceiptStatus::Rejected(RejectionCode::SessionExpired)
    );
    // Twin: unbind drops the grace.
    let (mut rig2, q2, _) = s(V2);
    rig2.host.begin_grace(q2, 100).expect("grace");
    rig2.host.unbind_peer(q2);
    assert_eq!(rig2.host.grace_deadline(q2), None);
    assert_eq!(
        rig2.host.resume_peer(&req(e1(), 1, proof_p()), 200),
        denied(e1())
    );
    // Twin: shutdown drops every grace.
    let (mut rig3, q3, _) = s(V2);
    rig3.host.begin_grace(q3, 100).expect("grace");
    rig3.host.shutdown();
    assert_eq!(rig3.host.grace_deadline(q3), None);
    assert_eq!(
        rig3.host.resume_peer(&req(e1(), 1, proof_p()), 200),
        Err(HostError::Closed)
    );
    let _ = twin;
}
