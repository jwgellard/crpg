//! Two-keys contract tests for T025b-i, written from `tasks/T025b.md` alone:
//! the reconnect-grace cases **G3** (`the_boundary_is_exact`), **G4**
//! (`forged_and_misdirected_requests`), **G6** (`live_resume_moves_the_fence`)
//! and **G7** (`racing_resumes_never_share_a_grant`), with independent
//! fixtures and values (incarnation 7, session handles 1 and 2, proofs
//! `0x3C..` and `0x9E..`, and a grace begun at `now_ms` 250).
//!
//! Everything is driven by the `now_ms` the test passes in; there are no
//! clocks, threads or sleeps, and no hand-computed hashes: where the resume
//! snapshot content matters, it is compared against the same public host API
//! (the deterministic `server_tick` and the known after-strike chunk) rather
//! than against any digest.

use crpg_core::{EntityId, Ulid};
use crpg_net::codec_transfer;
use crpg_net::codec_v2;
use crpg_net::protocol::{IntentBody, IntentFrame, NetId, ReceiptStatus, LANE_COMBAT};
use crpg_net::protocol_resume::{
    ConnectionFence, LaneClaim, LaneProgress, ResumeAccepted, ResumeProof, ResumeRejected,
    ResumeRejection, ResumeRequest,
};
use crpg_net::protocol_transfer::{BeginTransfer, CancelReason, TransferDown, TransferId};
use crpg_server::host::{
    ControlGrant, DeliveryEntry, DeliveryKind, DisclosureGrants, EntityDisclosure, Host,
    HostConfig, HostError, IngestDisposition, PeerHandle, ProtocolSelection, ResumeAnswer,
    TransferState,
};
use crpg_sim::HistoryWorld;
use serde_json::json;

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");
const V2: ProtocolSelection = ProtocolSelection::V2;

/// Own fixture values: a different incarnation than the contract's worked
/// example, so every epoch differs from `epoch_of(1, _)`.
const INCARNATION: u64 = 7;
/// The `now_ms` at which `begin_grace` is called (the contract uses 100).
const GRACE_AT: u64 = 250;
/// The exact deadline `begin_grace` must report.
const DEADLINE: u64 = GRACE_AT + 30_000;
/// The proof the session holds (the contract uses `0xA5..`).
const PROOF_A: [u8; 32] = [0x3C; 32];
/// A second, unrelated proof (the contract uses `0xB7..`).
const PROOF_B: [u8; 32] = [0x9E; 32];

// ---------------------------------------------------------------------------
// Fixture identities and grants (own helpers, same combat fixture as the
// rest of the crate: it is the only available authority).
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
        abilities: vec![strike(), Ulid::from_u128(604), Ulid::from_u128(605)],
        encounters: vec![Ulid::from_u128(601)],
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

fn all_actors() -> ControlGrant {
    control(&[a(), b(), c()])
}

fn epoch_of(incarnation: u64, handle: u64) -> [u8; 16] {
    let mut epoch = [0u8; 16];
    epoch[..8].copy_from_slice(&incarnation.to_le_bytes());
    epoch[8..].copy_from_slice(&handle.to_le_bytes());
    epoch
}

/// The deterministic snapshot body after A (id 1) strikes B (id 2): B is at
/// 7/10 and the turn is on B, round 0. Copied from the crate's own transfer
/// suite so the resumed chunk is compared to a public encoding, never a hash.
fn full_after_1() -> Vec<u8> {
    vec![
        0x03, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x02, 0x01, 0x07, 0x0A, 0x00, 0x03, 0x01, 0x03, 0x03,
        0x00, 0x01, 0x01, 0x02, 0x00,
    ]
}

fn proof(bytes: Vec<u8>) -> ResumeProof {
    ResumeProof::new(bytes).expect("valid proof")
}

fn fence(raw: u64) -> ConnectionFence {
    ConnectionFence::new(raw).expect("valid fence")
}

fn req(epoch: [u8; 16], fence_raw: u64, proof: ResumeProof) -> ResumeRequest {
    ResumeRequest {
        epoch,
        fence: fence(fence_raw),
        proof,
        claims: vec![LaneClaim {
            lane: LANE_COMBAT,
            receipted_through: 0,
        }],
    }
}

fn accepted(
    peer: PeerHandle,
    epoch: [u8; 16],
    granted: u64,
    finalized_through: u64,
) -> Result<ResumeAnswer, HostError> {
    Ok(ResumeAnswer::Accepted {
        peer,
        accepted: ResumeAccepted {
            epoch,
            granted: fence(granted),
            progress: vec![LaneProgress {
                lane: LANE_COMBAT,
                finalized_through,
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

// ---------------------------------------------------------------------------
// Decoded delivery (transfer frames only; the deltas are not inspected here).
// ---------------------------------------------------------------------------

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
// Harness.
// ---------------------------------------------------------------------------

struct Rig {
    host: Host,
    now: u64,
}

impl Rig {
    fn new() -> Self {
        let host = Host::from_history(
            fixture(),
            HostConfig {
                protocol: V2,
                incarnation: INCARNATION,
            },
            0,
        )
        .expect("wraps an acknowledged authority");
        Self { host, now: 0 }
    }

    fn epoch(&self, peer: PeerHandle) -> Result<[u8; 16], HostError> {
        self.host.epoch(peer)
    }

    fn bind_snapshot(&mut self, control: ControlGrant, grants: DisclosureGrants) -> PeerHandle {
        self.host
            .bind_snapshot_peer(control, grants, self.now)
            .expect("binds")
    }

    fn ticket(&mut self, peer: PeerHandle, proof: ResumeProof) {
        self.host
            .issue_resume_ticket(peer, proof)
            .expect("ticket issued");
    }

    fn declare(
        &self,
        peer: PeerHandle,
        seq: u64,
        actor: u64,
        ability: Ulid,
        target: u64,
    ) -> Vec<u8> {
        let frame = IntentFrame {
            epoch: self.epoch(peer).expect("bound"),
            lane: LANE_COMBAT,
            seq,
            observed_tick: self.host.server_tick(),
            actor: n(actor),
            body: IntentBody::DeclareAction {
                ability,
                target: n(target),
            },
        };
        codec_v2::encode_intent(&frame).expect("encodes")
    }

    fn send(&mut self, peer: PeerHandle, bytes: &[u8], now: u64) -> ReceiptStatus {
        self.now = now;
        assert_eq!(
            self.host.ingest(peer, bytes, now),
            Ok(IngestDisposition::Staged)
        );
        let summary = self.host.pump(now).expect("pump");
        assert_eq!(summary.results.len(), 1, "{summary:?}");
        summary.results[0].status
    }

    fn ack_all(&mut self, peer: PeerHandle) {
        let entries = self.host.take_delivery_entries(peer).expect("bound");
        if let Some(last) = entries.last() {
            let ordinal = last.ordinal;
            self.host.acknowledge_delivery(peer, ordinal).expect("acks");
        }
    }

    fn entries(&mut self, peer: PeerHandle) -> Vec<DeliveryEntry> {
        self.host.take_delivery_entries(peer).expect("bound")
    }

    fn transfer_state(&self, peer: PeerHandle) -> Option<TransferState> {
        self.host.transfer_state(peer).expect("bound")
    }

    fn tick(&self) -> u64 {
        self.host.server_tick()
    }

    /// A snapshot of everything a refused `resume_peer` must leave untouched:
    /// the complete-state (authority hash, executions, captures), the graced
    /// peer's deadline and the live peer's delivery log and transfer state.
    fn seal(&mut self, graced: PeerHandle, live: PeerHandle) -> Seal {
        Seal {
            hash: self.host.authority_hash(),
            executions: self.host.executions(),
            last_capture: self.host.last_capture(),
            capture_acknowledged: self.host.capture_acknowledged(),
            grace: self.host.grace_deadline(graced),
            live_entries: self.host.take_delivery_entries(live).expect("live"),
            live_state: self.host.transfer_state(live).expect("live"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Seal {
    hash: [u8; 32],
    executions: u64,
    last_capture: u64,
    capture_acknowledged: u64,
    grace: Option<u64>,
    live_entries: Vec<DeliveryEntry>,
    live_state: Option<TransferState>,
}

/// S: a snapshot session (handle 1) that has issued its ticket, acknowledged
/// its bind and applied A's strike on B at `now_ms` 100.
fn snapshot_session_after_strike() -> (Rig, PeerHandle) {
    let mut rig = Rig::new();
    let q = rig.bind_snapshot(all_actors(), full_grants());
    assert_eq!(q.get(), 1);
    rig.ticket(q, proof(PROOF_A.to_vec()));
    rig.ack_all(q);
    let bytes = rig.declare(q, 1, 1, strike(), 2);
    let status = rig.send(q, &bytes, 100);
    assert_eq!(status, ReceiptStatus::Applied);
    (rig, q)
}

// ---------------------------------------------------------------------------
// G3 — the boundary is exact.
// ---------------------------------------------------------------------------

#[test]
fn the_boundary_is_exact() {
    let e1 = epoch_of(INCARNATION, 1);

    // (a) One ms before expiry the grace is still held: a wrong proof is
    // denied without moving the deadline, and the right one resumes.
    let (mut rig, q) = snapshot_session_after_strike();
    assert_eq!(rig.host.begin_grace(q, GRACE_AT), Ok(DEADLINE));
    assert_eq!(rig.host.grace_deadline(q), Some(DEADLINE));
    assert_eq!(rig.epoch(q), Err(HostError::UnknownPeer));
    assert_eq!(
        rig.host
            .resume_peer(&req(e1, 1, proof(PROOF_B.to_vec())), DEADLINE - 2),
        denied(e1)
    );
    assert_eq!(rig.host.grace_deadline(q), Some(DEADLINE));
    assert_eq!(
        rig.host
            .resume_peer(&req(e1, 1, proof(PROOF_A.to_vec())), DEADLINE - 1),
        accepted(q, e1, 2, 1)
    );

    // (b) Exactly on expiry the grace is gone: the request is denied, the
    // deadline reads None and the freed seat admits a fresh binding.
    let (mut rig, q) = snapshot_session_after_strike();
    assert_eq!(rig.host.begin_grace(q, GRACE_AT), Ok(DEADLINE));
    assert_eq!(
        rig.host
            .resume_peer(&req(e1, 1, proof(PROOF_A.to_vec())), DEADLINE),
        denied(e1)
    );
    assert_eq!(rig.host.grace_deadline(q), None);
    let next = rig
        .host
        .bind_snapshot_peer(control(&[]), full_grants(), DEADLINE)
        .expect("seat freed at the boundary");
    assert_eq!(next.get(), 2);

    // (c) The sweep is lazy: `grace_deadline` still reports a past deadline
    // until a sweep runs, and `pump` is the sweep.
    let (mut rig, q) = snapshot_session_after_strike();
    assert_eq!(rig.host.begin_grace(q, GRACE_AT), Ok(DEADLINE));
    assert_eq!(rig.host.grace_deadline(q), Some(DEADLINE));
    rig.host.pump(DEADLINE).expect("pump sweeps");
    assert_eq!(rig.host.grace_deadline(q), None);

    // (d) One ms after expiry the grace is also gone.
    let (mut rig, q) = snapshot_session_after_strike();
    assert_eq!(rig.host.begin_grace(q, GRACE_AT), Ok(DEADLINE));
    assert_eq!(
        rig.host
            .resume_peer(&req(e1, 1, proof(PROOF_A.to_vec())), DEADLINE + 1),
        denied(e1)
    );
    assert_eq!(rig.host.grace_deadline(q), None);
}

// ---------------------------------------------------------------------------
// G4 — forged and misdirected requests are refused, state unchanged.
// ---------------------------------------------------------------------------

#[test]
fn forged_and_misdirected_requests_are_denied() {
    let (mut rig, q) = snapshot_session_after_strike();
    let e1 = epoch_of(INCARNATION, 1);
    let e2 = epoch_of(INCARNATION, 2);

    let q2 = rig.bind_snapshot(control(&[]), full_grants());
    assert_eq!(q2.get(), 2);
    rig.ticket(q2, proof(PROOF_B.to_vec()));
    assert_eq!(rig.host.begin_grace(q, GRACE_AT), Ok(DEADLINE));

    // Drain q2's bind entry once so the seal sees a stable, empty log.
    let _ = rig.entries(q2);
    let seal = rig.seal(q, q2);

    let now = 500;
    let forged: Vec<(ResumeRequest, [u8; 16])> = vec![
        // Wrong proof bytes (right length).
        (req(e1, 1, proof([0x4D; 32].to_vec())), e1),
        // Proof of the wrong length.
        (req(e1, 1, proof([0x3C; 16].to_vec())), e1),
        // q2's proof presented for q's epoch.
        (req(e1, 1, proof(PROOF_B.to_vec())), e1),
        // q's proof presented for q2's (live) epoch.
        (req(e2, 1, proof(PROOF_A.to_vec())), e2),
        // An epoch no session holds.
        (
            req(epoch_of(INCARNATION, 99), 1, proof(PROOF_A.to_vec())),
            epoch_of(INCARNATION, 99),
        ),
        // The right handle under a different incarnation.
        (
            req(epoch_of(2, 1), 1, proof(PROOF_A.to_vec())),
            epoch_of(2, 1),
        ),
        // A fence above the current one.
        (req(e1, 2, proof(PROOF_A.to_vec())), e1),
    ];

    for (request, rejected_epoch) in forged {
        assert_eq!(
            rig.host.resume_peer(&request, now),
            denied(rejected_epoch),
            "epoch {:?}",
            request.epoch
        );
        assert_eq!(rig.seal(q, q2), seal, "epoch {:?}", request.epoch);
    }

    // Control: the right request still resumes, so no refusal moved the fence
    // or the deadline.
    assert_eq!(
        rig.host
            .resume_peer(&req(e1, 1, proof(PROOF_A.to_vec())), now),
        accepted(q, e1, 2, 1)
    );
    assert_eq!(rig.host.grace_deadline(q), None);
}

// ---------------------------------------------------------------------------
// G6 — a live resume moves the fence.
// ---------------------------------------------------------------------------

#[test]
fn live_resume_moves_the_fence() {
    let (mut rig, q) = snapshot_session_after_strike();
    let e1 = epoch_of(INCARNATION, 1);
    let first = ConnectionFence::FIRST;

    // Control: before any resume, the FIRST fence is current.
    assert_eq!(rig.host.check_frame(q, first), Ok(()));
    assert_eq!(rig.epoch(q), Ok(e1));

    // Resume the live session: the fence moves to 2.
    assert_eq!(
        rig.host
            .resume_peer(&req(e1, 1, proof(PROOF_A.to_vec())), 200),
        accepted(q, e1, 2, 1)
    );
    assert_eq!(rig.epoch(q), Ok(e1));

    // The old fence and any future fence are refused; the new one passes.
    assert_eq!(
        rig.host.check_frame(q, first),
        Err(HostError::SessionFenced)
    );
    assert_eq!(
        rig.host.check_frame(q, fence(3)),
        Err(HostError::SessionFenced)
    );
    assert_eq!(rig.host.check_frame(q, fence(2)), Ok(()));

    // The resume staged a fresh snapshot above the old cursor.
    let entries = rig.entries(q);
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![(3, DeliveryKind::Transfer), (4, DeliveryKind::Transfer)]
    );
    let begin = decode_begin(&entries[0].bytes);
    assert_eq!(begin.transfer, TransferId::new(1).expect("nonzero"));
    assert_eq!(begin.cursor, 2);
    assert_eq!(begin.base_tick, rig.tick());
    assert_eq!(decode_chunk(&entries[1].bytes), full_after_1());
    assert_eq!(
        rig.transfer_state(q),
        Some(TransferState {
            next_transfer: 2,
            in_flight: Some(1),
        })
    );
}

// ---------------------------------------------------------------------------
// G7 — racing resumes never share a grant.
// ---------------------------------------------------------------------------

#[test]
fn racing_resumes_never_share_a_grant() {
    let (mut rig, q) = snapshot_session_after_strike();
    let e1 = epoch_of(INCARNATION, 1);
    assert_eq!(rig.host.begin_grace(q, GRACE_AT), Ok(DEADLINE));

    // The same request (fence 1, proof A) races twice: each gets a distinct
    // grant, so the two never share a fence.
    let race = req(e1, 1, proof(PROOF_A.to_vec()));
    assert_eq!(rig.host.resume_peer(&race, 400), accepted(q, e1, 2, 1));

    let first = rig.entries(q);
    assert_eq!(
        first
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![(3, DeliveryKind::Transfer), (4, DeliveryKind::Transfer)]
    );

    assert_eq!(rig.host.resume_peer(&race, 500), accepted(q, e1, 3, 1));
    let entries = rig.entries(q);
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.ordinal, entry.kind))
            .collect::<Vec<_>>(),
        vec![
            (5, DeliveryKind::Transfer),
            (6, DeliveryKind::Transfer),
            (7, DeliveryKind::Transfer),
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
    assert_eq!(decode_chunk(&entries[2].bytes), full_after_1());
    assert_eq!(
        rig.transfer_state(q),
        Some(TransferState {
            next_transfer: 3,
            in_flight: Some(2),
        })
    );

    // The superseded grant is fenced; the current one is not.
    assert_eq!(
        rig.host.check_frame(q, fence(2)),
        Err(HostError::SessionFenced)
    );
    assert_eq!(rig.host.check_frame(q, fence(3)), Ok(()));

    // Below the confirmed fence, the race and a stale fence are both denied.
    assert_eq!(rig.host.resume_peer(&race, 600), denied(e1));
    assert_eq!(
        rig.host
            .resume_peer(&req(e1, 2, proof(PROOF_A.to_vec())), 600),
        denied(e1)
    );

    // Control: the current fence still resumes with a fresh grant.
    assert_eq!(
        rig.host
            .resume_peer(&req(e1, 3, proof(PROOF_A.to_vec())), 700),
        accepted(q, e1, 4, 1)
    );
}
