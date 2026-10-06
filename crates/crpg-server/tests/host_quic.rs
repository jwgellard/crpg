//! T023b acceptance suite (`tasks/T023b.md` B§14 as amended by A§3–A§4):
//! public API only, over real `crpg-net-quic` loopback against the T022
//! host.
//!
//! Oracles are independent of the adapter:
//!
//! - **Authority oracle**: a test-owned mirror `HistoryWorld` loaded from
//!   the same fixture, given every action the test expects to be accepted;
//!   its `history_hash` must equal the host's `authority_hash`.
//! - **Permitted-state oracle**: per client, a replica built only from the
//!   bytes that client received over QUIC, compared with the mirror's public
//!   reads plus hand-assigned replica ids.
//! - **Trace replay**: every pump's `calls`, fed with the clients' own send
//!   logs to a fresh in-memory host, must reproduce every result, the final
//!   authority, the captures and the delivered bytes (B§11).
//!
//! Every wait is a guard that panics `"deadline"` after 30 s of real time;
//! no case sleeps as an oracle or asserts on wall-clock duration. Injected
//! time comes only from `TestClock`.

#[path = "support/quic_host_rig.rs"]
mod rig;

use std::collections::{BTreeMap, VecDeque};

use crpg_net::protocol::{ReceiptStatus, RejectionCode};
use crpg_net_quic::{
    ClientLimits, CloseCode, CloseMode, CloseReason, ConnectError, ConnectionId, ServerLimits,
    Welcome,
};
use crpg_server::capture::{CapturedRecord, MAX_CAPTURE_PAGE};
use crpg_server::checkpoint::load_checkpoint;
use crpg_server::host::{
    AdmissionResult, ControlGrant, DisclosureGrants, Host, HostError, IngestDisposition,
    PeerHandle, ProtocolSelection, PumpSummary,
};
use crpg_server::quic::{
    Fence, FenceCause, GrantChange, HelloOutcome, HostCall, InvitationId, QuicHost, QuicHostError,
    QuicPumpReport, INVITATION_CREDENTIAL_BYTES, MAX_DRAIN_PER_CONNECTION, MAX_INVITATIONS,
    STALL_TIMEOUT_MS,
};
use crpg_sim::{history_hash, CombatAction, HistoryWorld};
use rig::*;

/// Runs one named case body under both selected versions, as
/// `<case>::v1` and `<case>::v2`.
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

const APPLIED: ReceiptStatus = ReceiptStatus::Applied;

fn rejected(code: RejectionCode) -> ReceiptStatus {
    ReceiptStatus::Rejected(code)
}

fn result(peer: PeerHandle, seq: Option<u64>, status: ReceiptStatus) -> AdmissionResult {
    AdmissionResult {
        peer,
        seq,
        status,
        cached: false,
        retained_in_ingress: false,
    }
}

fn empty_summary() -> PumpSummary {
    PumpSummary {
        admitted: 0,
        applied: 0,
        rejected: 0,
        backpressured: 0,
        dropped_replies: 0,
        results: Vec::new(),
    }
}

/// The `Pump` summary of one report.
fn summary(report: &QuicPumpReport) -> PumpSummary {
    report
        .calls
        .iter()
        .find_map(|call| match call {
            HostCall::Pump { summary } => Some(summary.clone()),
            _ => None,
        })
        .expect("every pump report traces one host pump")
}

/// `take_delivery`'s count for `peer` in one report.
fn taken(report: &QuicPumpReport, peer: PeerHandle) -> Option<usize> {
    report.calls.iter().find_map(|call| match call {
        HostCall::TakeDelivery {
            peer: p,
            result: Ok(n),
        } if *p == peer => Some(*n),
        _ => None,
    })
}

/// The highest delivery acknowledgement for `peer` across `steps`.
fn acked_through(steps: &[Step], peer: PeerHandle) -> u64 {
    steps
        .iter()
        .flat_map(|(_, report)| report.calls.iter())
        .filter_map(|call| match call {
            HostCall::AcknowledgeDelivery { peer: p, through } if *p == peer => Some(*through),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

/// Ordinals of `conn`'s frames that ingest staged, in trace order.
fn staged_ordinals(steps: &[Step], conn: ConnectionId) -> Vec<u64> {
    steps
        .iter()
        .flat_map(|(_, report)| report.calls.iter())
        .filter_map(|call| match call {
            HostCall::Ingest {
                conn: c,
                frame,
                result: IngestDisposition::Staged,
                ..
            } if *c == conn => Some(*frame),
            _ => None,
        })
        .collect()
}

/// Sends one frame and pumps (+100 ms each) until the adapter ingested it;
/// returns the report of that pump (which also admitted it).
fn exchange(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    client: &mut Client,
    bytes: Vec<u8>,
) -> QuicPumpReport {
    let conn = client.conn;
    client.send(bytes);
    let ordinal = client.sent.len() as u64;
    let steps = pump_until(qh, clock, 100, "frame ingested", |_, report| {
        report.calls.iter().any(|call| {
            matches!(call, HostCall::Ingest { conn: c, frame, .. } if *c == conn && *frame == ordinal)
        })
    });
    steps.into_iter().last().expect("one pump").1
}

fn mirror(oracle: &mut HistoryWorld, action: CombatAction) {
    oracle
        .perform_action(&action)
        .expect("the mirror accepts the same action");
    oracle
        .acknowledge(oracle.last_sequence())
        .expect("mirror acks");
}

fn register(
    qh: &mut QuicHost,
    bytes: &[u8],
    control: ControlGrant,
    grants: DisclosureGrants,
) -> InvitationId {
    qh.register_invitation(invitation(bytes, control, grants))
        .expect("registers")
}

/// Every retained capture record, paged.
fn all_captures(host: &Host) -> Vec<CapturedRecord> {
    let mut out = Vec::new();
    let mut after = host.capture_acknowledged();
    loop {
        let page = host
            .read_captures(after, MAX_CAPTURE_PAGE)
            .expect("readable");
        if page.is_empty() {
            return out;
        }
        after = page.last().expect("non-empty").capture_seq;
        out.extend(page);
    }
}

/// Pumps until every client has received every frame the adapter
/// acknowledged for it, reading as it goes.
fn settle(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    step_ms: u64,
    steps: &mut Vec<Step>,
    clients: &mut [Client],
) {
    let more = pump_until(qh, clock, step_ms, "clients settle", |_, _| true);
    steps.extend(more);
    for client in clients.iter_mut() {
        let through = acked_through(steps, client.peer);
        client.read_until("client holds every acknowledged frame", |c| {
            c.frames.len() as u64 == through
        });
    }
}

// ---------------------------------------------------------------------------
// B§14.2 — conformance cases moved from T023.
// ---------------------------------------------------------------------------

fn quic_malicious_frames_rejected_without_mutation(protocol: ProtocolSelection) {
    let (mut qh, mut clock) = start(protocol, ServerLimits::v1());
    register(&mut qh, &cred_bytes(0), control(&[a()]), full_grants());
    register(&mut qh, &cred_bytes(1), control(&[b()]), full_grants());
    let addr = qh.local_addr();
    let (mut ca, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        protocol,
        &cred_bytes(0),
    );
    let (mut cb, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        protocol,
        &cred_bytes(1),
    );
    ca.read_until("A initial state", |c| !c.frames.is_empty());
    cb.read_until("B initial state", |c| !c.frames.is_empty());
    let mut oracle = fixture();
    let v = ca.version;
    let (pa, ea, eb) = (ca.peer, ca.epoch, cb.epoch);

    // Setup (positive control for A): A ends its own turn, so B is active.
    let report = exchange(&mut qh, &mut clock, &mut ca, end_turn(v, ea, 1, 0, 1));
    assert_eq!(summary(&report).results, vec![result(pa, Some(1), APPLIED)]);
    mirror(&mut oracle, CombatAction::EndTurn { actor: a() });
    assert_eq!(qh.host().authority_hash(), history_hash(&oracle));
    ca.read_until("A's first receipt", |c| c.has_receipt(1));

    // Terminal rejections with a receipt.
    let receipted: [(Vec<u8>, u64, RejectionCode); 4] = [
        // An unmapped NetId as actor.
        (
            declare(v, ea, 2, 0, 777, strike(), 2),
            2,
            RejectionCode::NotAuthorized,
        ),
        // B's actor as A's own.
        (
            declare(v, ea, 3, 0, 2, strike(), 1),
            3,
            RejectionCode::NotAuthorized,
        ),
        // A hidden (never-mapped) target.
        (
            declare(v, ea, 4, 0, 1, strike(), 999),
            4,
            RejectionCode::NotAuthorized,
        ),
        // observed_tick above the server tick.
        (
            declare(v, ea, 5, 5, 1, strike(), 2),
            5,
            RejectionCode::FutureTick,
        ),
    ];
    for (bytes, seq, code) in receipted {
        let before = snapshot(qh.host());
        let report = exchange(&mut qh, &mut clock, &mut ca, bytes);
        assert_eq!(
            summary(&report).results,
            vec![result(pa, Some(seq), rejected(code))]
        );
        assert_eq!(taken(&report, pa), Some(1), "one receipt frame");
        assert_eq!(snapshot(qh.host()), before, "complete state unchanged");
        ca.read_until("rejection receipt", |c| c.has_receipt(seq));
        let last = ca.receipts.last().expect("receipt");
        assert_eq!((last.0, last.1), (seq, rejected(code)));
    }

    // Rejections with no reply: nothing staged for delivery, no seq used.
    let mut cut = declare(v, ea, 6, 0, 1, strike(), 2);
    cut.truncate(cut.len() - 5);
    let other = match protocol {
        ProtocolSelection::V1 => 2,
        ProtocolSelection::V2 => 1,
    };
    let mut unknown_tag = intent_header(v, ea, 6, 0, 1);
    unknown_tag.push(2);
    let silent: [(Vec<u8>, Option<u64>, RejectionCode); 4] = [
        (
            end_turn(v, eb, 6, 0, 1),
            Some(6),
            RejectionCode::SessionExpired,
        ),
        (cut, None, RejectionCode::Malformed),
        (
            end_turn(other, ea, 6, 0, 1),
            None,
            RejectionCode::UnsupportedVersion,
        ),
        (unknown_tag, None, RejectionCode::UnknownMessage),
    ];
    for (bytes, seq, code) in silent {
        let before = snapshot(qh.host());
        let report = exchange(&mut qh, &mut clock, &mut ca, bytes);
        assert_eq!(
            summary(&report).results,
            vec![result(pa, seq, rejected(code))]
        );
        assert_eq!(taken(&report, pa), Some(0), "no reply");
        assert_eq!(snapshot(qh.host()), before, "complete state unchanged");
    }

    // A replayed seq with different bytes: SeqConflict, the host fences A
    // and the adapter closes it with SessionFenced.
    let before = snapshot(qh.host());
    let report = exchange(&mut qh, &mut clock, &mut ca, end_turn(v, ea, 2, 0, 1));
    assert_eq!(
        summary(&report).results,
        vec![result(pa, Some(2), rejected(RejectionCode::SeqConflict))]
    );
    assert_eq!(
        report.fences,
        vec![Fence {
            conn: ca.conn,
            peer: pa,
            cause: FenceCause::EpochClosed
        }]
    );
    assert!(
        !report.calls.contains(&HostCall::UnbindPeer { peer: pa }),
        "the host already fenced"
    );
    assert_eq!(qh.host().epoch(pa), Err(HostError::UnknownPeer));
    assert_eq!(snapshot(qh.host()), before);
    assert_eq!(ca.read_to_close(), CloseReason::Peer { code: 9 });

    // Positive control: B's legal whiff applies and its facts arrive.
    let report = exchange(
        &mut qh,
        &mut clock,
        &mut cb,
        declare(v, eb, 1, 0, 2, whiff(), 1),
    );
    assert_eq!(
        summary(&report).results,
        vec![result(cb.peer, Some(1), APPLIED)]
    );
    mirror(
        &mut oracle,
        CombatAction::UseAbility {
            actor: b(),
            ability: whiff(),
            target: a(),
        },
    );
    assert_eq!(qh.host().authority_hash(), history_hash(&oracle));
    assert_eq!(qh.host().executions(), 2);
    cb.read_until("B's receipt", |c| c.has_receipt(1));
    assert_eq!(cb.receipts.last().map(|r| (r.0, r.1)), Some((1, APPLIED)));
    let events = cb.replica().events;
    assert_eq!(
        events,
        for_version(
            protocol,
            vec![
                started(2, 0),
                resolved(2, 1, whiff(), "failure", 0),
                started(3, 0),
            ]
        )
    );
}
both_versions!(quic_malicious_frames_rejected_without_mutation);

/// B's grants for cases 2 and 3: observer, with A present but its health
/// hidden (the original conformance shape).
fn hidden_health_observer() -> DisclosureGrants {
    let mut grants = observer_grants();
    grants.entities[0].health = false;
    grants
}

/// A (all actors, full grants) and B (no control, observer without A's
/// health), both connected and holding their initial state.
fn filtered_setup(protocol: ProtocolSelection) -> (QuicHost, TestClock, Client, Client) {
    let (mut qh, mut clock) = start(protocol, ServerLimits::v1());
    register(&mut qh, &cred_bytes(0), all_actors(), full_grants());
    register(
        &mut qh,
        &cred_bytes(1),
        control(&[]),
        hidden_health_observer(),
    );
    let addr = qh.local_addr();
    let (mut ca, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        protocol,
        &cred_bytes(0),
    );
    let (mut cb, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        protocol,
        &cred_bytes(1),
    );
    ca.read_until("A initial state", |c| !c.frames.is_empty());
    cb.read_until("B initial state", |c| !c.frames.is_empty());
    (qh, clock, ca, cb)
}

/// Plays `turns` turns: the active actor's controller (A) strikes B when A
/// is active, else ends the active actor's turn. Returns every step.
fn play_turns(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    ca: &mut Client,
    oracle: &mut HistoryWorld,
    turns: u64,
) -> Vec<Step> {
    let ids = standard_ids();
    let mut steps = Vec::new();
    for seq in 1..=turns {
        let active = oracle
            .world()
            .combat()
            .and_then(|combat| combat.active)
            .expect("an active actor");
        let (bytes, action) = if active == a() {
            (
                declare(ca.version, ca.epoch, seq, 0, 1, strike(), 2),
                CombatAction::UseAbility {
                    actor: a(),
                    ability: strike(),
                    target: b(),
                },
            )
        } else {
            (
                end_turn(ca.version, ca.epoch, seq, 0, ids[&active]),
                CombatAction::EndTurn { actor: active },
            )
        };
        let report = exchange(qh, clock, ca, bytes);
        assert_eq!(
            summary(&report).results,
            vec![result(ca.peer, Some(seq), APPLIED)]
        );
        mirror(oracle, action);
        steps.push((clock.now(), report));
    }
    steps
}

fn quic_filtered_replica_converges_per_peer(protocol: ProtocolSelection) {
    let (mut qh, mut clock, mut ca, mut cb) = filtered_setup(protocol);
    let mut oracle = fixture();
    let mut steps = play_turns(&mut qh, &mut clock, &mut ca, &mut oracle, 6);
    assert_eq!(qh.host().authority_hash(), history_hash(&oracle));
    settle(
        &mut qh,
        &mut clock,
        100,
        &mut steps,
        std::slice::from_mut(&mut ca),
    );
    let through_b = acked_through(&steps, cb.peer);
    cb.read_until("B holds every frame", |c| {
        c.frames.len() as u64 == through_b
    });

    // Replicas from QUIC bytes only, gapless from event_seq 1.
    let replica_a = ca.replica();
    let replica_b = cb.replica();
    assert_eq!(ca.frames[0].event_seq, 1);
    assert_eq!(cb.frames[0].event_seq, 1);
    assert_eq!(
        replica_a.state,
        expected_state(&oracle, &full_grants(), &standard_ids())
    );
    assert_eq!(
        replica_b.state,
        expected_state(&oracle, &hidden_health_observer(), &standard_ids())
    );
    // B holds A's presence but never A's health.
    assert!(replica_b.state.entered.contains(&1));
    assert!(!replica_b.state.health.contains_key(&1));
    assert!(cb
        .frames
        .iter()
        .all(|frame| !frame.ops.iter().any(|op| *op == health(1, 10, 10, false))));
    // The ordered event oracle (V2; V1 carries none).
    assert_eq!(
        replica_a.events,
        for_version(
            protocol,
            vec![
                resolved(1, 2, strike(), "success", 3),
                started(2, 0),
                started(3, 0),
                started(1, 1),
                resolved(1, 2, strike(), "success", 3),
                started(2, 1),
                started(3, 1),
                started(1, 2),
            ]
        )
    );
    assert!(replica_b.events.is_empty());
    assert_eq!(replica_a.receipts.len(), 6);
    assert!(replica_b.receipts.is_empty());
}
both_versions!(quic_filtered_replica_converges_per_peer);

fn quic_desync_oracle_fails_on_drop_and_corruption(protocol: ProtocolSelection) {
    let (mut qh, mut clock, mut ca, mut cb) = filtered_setup(protocol);
    let mut oracle = fixture();
    let mut steps = play_turns(&mut qh, &mut clock, &mut ca, &mut oracle, 3);
    settle(
        &mut qh,
        &mut clock,
        100,
        &mut steps,
        std::slice::from_mut(&mut ca),
    );
    let frames = ca.frames.clone();
    assert_eq!(frames.len(), 4, "initial state plus three command frames");
    let expected = expected_state(&oracle, &full_grants(), &standard_ids());

    // Positive control: the unmodified stream converges.
    assert_eq!(ca.replica().state, expected);

    // Drop: the strike frame (event_seq 2) never reaches the replica.
    let mut replica = Replica::new();
    replica.try_apply(&frames[0]).expect("in order");
    assert_eq!(
        replica.try_apply(&frames[2]),
        Err(Gap {
            expected: 2,
            got: 3
        })
    );
    assert_ne!(replica.state, expected, "a dropped update diverges");
    // Feeding the frame back reconverges.
    for frame in &frames[1..] {
        replica.try_apply(frame).expect("refilled in order");
    }
    assert_eq!(replica.state, expected, "redelivery reconverges");

    // Corruption: the last frame with its last byte flipped either fails to
    // decode or diverges from the oracle.
    let mut tampered = frames[3].bytes.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xFF;
    match try_decode_frame(protocol, &tampered) {
        None => {}
        Some(frame) => {
            let mut replica = Replica::new();
            replica.apply(&frames[..3]);
            match replica.try_apply(&frame) {
                Err(_) => {}
                Ok(()) => assert_ne!(replica.state, expected, "corrupted facts diverge"),
            }
        }
    }

    // B, the filtered peer, converges on its own permitted view too.
    let through_b = acked_through(&steps, cb.peer);
    cb.read_until("B holds every frame", |c| {
        c.frames.len() as u64 == through_b
    });
    assert_eq!(
        cb.replica().state,
        expected_state(&oracle, &hidden_health_observer(), &standard_ids())
    );
}
both_versions!(quic_desync_oracle_fails_on_drop_and_corruption);

fn quic_lost_receipt_retry_executes_once(protocol: ProtocolSelection) {
    let (mut qh, mut clock) = start(protocol, ServerLimits::v1());
    register(&mut qh, &cred_bytes(0), all_actors(), full_grants());
    let addr = qh.local_addr();
    let (mut ca, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        protocol,
        &cred_bytes(0),
    );
    ca.read_until("initial state", |c| !c.frames.is_empty());
    let mut oracle = fixture();
    let v = ca.version;
    let bytes = declare(v, ca.epoch, 1, 0, 1, whiff(), 2);
    let report = exchange(&mut qh, &mut clock, &mut ca, bytes.clone());
    assert_eq!(
        summary(&report).results,
        vec![result(ca.peer, Some(1), APPLIED)]
    );
    mirror(
        &mut oracle,
        CombatAction::UseAbility {
            actor: a(),
            ability: whiff(),
            target: b(),
        },
    );
    // The receipt arrives and the test discards it (a lost reply).
    ca.read_until("the first receipt", |c| c.has_receipt(1));
    assert_eq!(ca.frames.last().map(|f| f.event_seq), Some(2));
    ca.receipts.clear();
    // A trusted tick moves the server tick; the retry is the identical bytes.
    qh.tick().expect("trusted tick");
    oracle.tick().expect("mirror tick");
    let report = exchange(&mut qh, &mut clock, &mut ca, bytes);
    assert_eq!(
        summary(&report).results,
        vec![AdmissionResult {
            cached: true,
            ..result(ca.peer, Some(1), APPLIED)
        }]
    );
    ca.read_until("the fresh receipt", |c| c.has_receipt(1));
    let fresh = ca.frames.last().expect("frame").clone();
    assert_eq!(fresh.event_seq, 3);
    assert_eq!(fresh.server_tick, 1);
    assert_eq!(fresh.ops, vec![receipt(ca.epoch, 1, 0, APPLIED)]);
    assert_eq!(qh.host().executions(), 1);
    assert_eq!(qh.host().authority_hash(), history_hash(&oracle));
    // Control: seq 2 executes.
    let bytes = declare(v, ca.epoch, 2, 1, 2, whiff(), 1);
    let report = exchange(&mut qh, &mut clock, &mut ca, bytes);
    assert_eq!(
        summary(&report).results,
        vec![result(ca.peer, Some(2), APPLIED)]
    );
    mirror(
        &mut oracle,
        CombatAction::UseAbility {
            actor: b(),
            ability: whiff(),
            target: a(),
        },
    );
    assert_eq!(qh.host().executions(), 2);
    assert_eq!(qh.host().authority_hash(), history_hash(&oracle));
}
both_versions!(quic_lost_receipt_retry_executes_once);

// -- Case 5: the admitted schedule over QUIC equals the in-memory one. ------

/// What a replay or a run ends with.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    hash: [u8; 32],
    executions: u64,
    last_capture: u64,
    captures: Vec<CapturedRecord>,
    delivered: BTreeMap<PeerHandle, Vec<Vec<u8>>>,
}

fn outcome_of(host: &Host, delivered: BTreeMap<PeerHandle, Vec<Vec<u8>>>) -> Outcome {
    Outcome {
        hash: host.authority_hash(),
        executions: host.executions(),
        last_capture: host.last_capture(),
        captures: all_captures(host),
        delivered,
    }
}

type Grants = BTreeMap<InvitationId, (ControlGrant, DisclosureGrants)>;

/// B§11: replays every traced host call against a fresh in-memory host
/// (same fixture and config), with each ingest's bytes recovered from the
/// sending client's log by ordinal. `Err` names the first divergence.
fn replay(
    protocol: ProtocolSelection,
    steps: &[Step],
    sent: &BTreeMap<ConnectionId, Vec<Vec<u8>>>,
    grants: &Grants,
) -> Result<Outcome, String> {
    let mut host = new_host(protocol);
    let mut last_take: BTreeMap<PeerHandle, Vec<Vec<u8>>> = BTreeMap::new();
    let mut acked: BTreeMap<PeerHandle, u64> = BTreeMap::new();
    let mut delivered: BTreeMap<PeerHandle, Vec<Vec<u8>>> = BTreeMap::new();
    for (index, (now, report)) in steps.iter().enumerate() {
        for call in &report.calls {
            match call {
                HostCall::BindPeer {
                    invitation, result, ..
                } => {
                    let (control, grants) = grants.get(invitation).ok_or("unknown invitation")?;
                    let got = host.bind_peer(control.clone(), grants.clone(), *now);
                    if got != *result {
                        return Err(format!("step {index}: bind {got:?} != {result:?}"));
                    }
                }
                HostCall::UnbindPeer { peer } => host.unbind_peer(*peer),
                HostCall::Ingest {
                    conn,
                    peer,
                    frame,
                    len,
                    result,
                } => {
                    let bytes = sent
                        .get(conn)
                        .and_then(|log| log.get(*frame as usize - 1))
                        .ok_or(format!("step {index}: no frame {frame} on {conn:?}"))?;
                    if bytes.len() != *len {
                        return Err(format!("step {index}: length differs"));
                    }
                    let got = host.ingest(*peer, bytes, *now);
                    if got != Ok(result.clone()) {
                        return Err(format!("step {index}: ingest {got:?} != {result:?}"));
                    }
                }
                HostCall::Pump { summary } => {
                    let got = host.pump(*now);
                    if got.as_ref() != Ok(summary) {
                        return Err(format!("step {index}: pump {got:?} != {summary:?}"));
                    }
                }
                HostCall::TakeDelivery { peer, result } => {
                    let got = host.take_delivery(*peer);
                    let count = got.as_ref().map(Vec::len).map_err(Clone::clone);
                    if count != *result {
                        return Err(format!("step {index}: take {count:?} != {result:?}"));
                    }
                    if let Ok(frames) = got {
                        last_take.insert(*peer, frames);
                    }
                }
                HostCall::AcknowledgeDelivery { peer, through } => {
                    host.acknowledge_delivery(*peer, *through)
                        .map_err(|e| format!("step {index}: ack {e:?}"))?;
                    let before = acked.get(peer).copied().unwrap_or(0);
                    let k = (*through - before) as usize;
                    let frames = last_take.get(peer).ok_or("ack without take")?;
                    delivered
                        .entry(*peer)
                        .or_default()
                        .extend(frames[..k].iter().cloned());
                    acked.insert(*peer, *through);
                }
            }
        }
    }
    Ok(outcome_of(&host, delivered))
}

const IMPAIRED: RelayConfig = RelayConfig {
    drop_every: 13,
    reorder_depth: 4,
    seed: 23,
};

/// Three clients (A, B, C each controlling its own actor, full grants),
/// each through its own relay.
fn three_clients(
    protocol: ProtocolSelection,
    relay: RelayConfig,
    step_ms: u64,
) -> (
    QuicHost,
    TestClock,
    Vec<UdpRelay>,
    Vec<Client>,
    Grants,
    Vec<Step>,
) {
    let (mut qh, mut clock) = start(protocol, ServerLimits::v1());
    let actors = [a(), b(), c()];
    let mut grants = Grants::new();
    let mut relays = Vec::new();
    let mut clients = Vec::new();
    let mut steps = Vec::new();
    for (index, actor) in actors.iter().enumerate() {
        let id = register(
            &mut qh,
            &cred_bytes(index as u8),
            control(&[*actor]),
            full_grants(),
        );
        grants.insert(id, (control(&[*actor]), full_grants()));
    }
    for index in 0..3u8 {
        let relay_handle = UdpRelay::start(qh.local_addr(), relay);
        let (client, more) = connect(
            &mut qh,
            &mut clock,
            step_ms,
            relay_handle.addr(),
            ClientLimits::v1(),
            protocol,
            &cred_bytes(index),
        );
        assert_eq!(client.peer.get(), u64::from(index) + 1);
        steps.extend(more);
        relays.push(relay_handle);
        clients.push(client);
    }
    (qh, clock, relays, clients, grants, steps)
}

/// The lockstep rotation's command `k` (1-based): actor index, and whether
/// it is a whiff (else `EndTurn`). `swap` turns that command's whiff into
/// an `EndTurn` (the negative control).
fn lockstep_command(k: u64, swap: Option<u64>) -> (usize, bool) {
    let actor = ((k - 1) % 3) as usize;
    let whiff = k.is_multiple_of(2) && swap != Some(k);
    (actor, whiff)
}

fn lockstep_bytes(version: u8, epoch: [u8; 16], seq: u64, actor: usize, whiff_it: bool) -> Vec<u8> {
    let net = actor as u64 + 1;
    if whiff_it {
        declare(version, epoch, seq, 0, net, whiff(), net % 3 + 1)
    } else {
        end_turn(version, epoch, seq, 0, net)
    }
}

const LOCKSTEP_COMMANDS: u64 = 300;

/// The lockstep run over QUIC through `relay`.
fn lockstep_quic(protocol: ProtocolSelection, relay: RelayConfig) -> (Outcome, u64) {
    let (mut qh, mut clock, relays, mut clients, _, mut steps) = three_clients(protocol, relay, 10);
    let mut seqs = [0u64; 3];
    for k in 1..=LOCKSTEP_COMMANDS {
        let (actor, whiff_it) = lockstep_command(k, None);
        seqs[actor] += 1;
        let seq = seqs[actor];
        let client = &mut clients[actor];
        let bytes = lockstep_bytes(client.version, client.epoch, seq, actor, whiff_it);
        client.send(bytes);
        let more = pump_until(&mut qh, &mut clock, 10, "lockstep receipt", |_, _| {
            for client in clients.iter_mut() {
                client.read_available();
            }
            clients[actor].has_receipt(seq)
        });
        steps.extend(more);
        let last = clients[actor].receipts.last().expect("receipt");
        assert_eq!((last.0, last.1), (seq, APPLIED), "lockstep command {k}");
    }
    settle(&mut qh, &mut clock, 10, &mut steps, &mut clients);
    let delivered = clients
        .iter()
        .map(|client| (client.peer, client.frame_bytes()))
        .collect();
    let dropped = relays.iter().map(UdpRelay::dropped).sum();
    (outcome_of(qh.host(), delivered), dropped)
}

/// The lockstep run directly against an in-memory host, no adapter.
fn lockstep_memory(protocol: ProtocolSelection, swap: Option<u64>) -> Outcome {
    let mut host = new_host(protocol);
    let mut now = CLOCK_START;
    let actors = [a(), b(), c()];
    let peers: Vec<PeerHandle> = actors
        .iter()
        .map(|actor| {
            host.bind_peer(control(&[*actor]), full_grants(), now)
                .expect("binds")
        })
        .collect();
    let mut delivered: BTreeMap<PeerHandle, Vec<Vec<u8>>> = BTreeMap::new();
    let mut acked: BTreeMap<PeerHandle, u64> = BTreeMap::new();
    let drain = |host: &mut Host,
                 delivered: &mut BTreeMap<PeerHandle, Vec<Vec<u8>>>,
                 acked: &mut BTreeMap<PeerHandle, u64>| {
        for peer in &peers {
            let frames = host.take_delivery(*peer).expect("bound");
            let through = acked.get(peer).copied().unwrap_or(0) + frames.len() as u64;
            if !frames.is_empty() {
                host.acknowledge_delivery(*peer, through).expect("acks");
            }
            acked.insert(*peer, through);
            delivered.entry(*peer).or_default().extend(frames);
        }
    };
    drain(&mut host, &mut delivered, &mut acked);
    let version = version_byte(protocol);
    let mut seqs = [0u64; 3];
    for k in 1..=LOCKSTEP_COMMANDS {
        let (actor, whiff_it) = lockstep_command(k, swap);
        seqs[actor] += 1;
        let epoch = host.epoch(peers[actor]).expect("bound");
        let bytes = lockstep_bytes(version, epoch, seqs[actor], actor, whiff_it);
        now += 100;
        assert_eq!(
            host.ingest(peers[actor], &bytes, now),
            Ok(IngestDisposition::Staged)
        );
        let summary = host.pump(now).expect("pump");
        assert_eq!(summary.applied, 1, "command {k}");
        drain(&mut host, &mut delivered, &mut acked);
    }
    outcome_of(&host, delivered)
}

fn decoded(
    protocol: ProtocolSelection,
    delivered: &BTreeMap<PeerHandle, Vec<Vec<u8>>>,
) -> BTreeMap<PeerHandle, Vec<Frame>> {
    delivered
        .iter()
        .map(|(peer, frames)| {
            (
                *peer,
                frames
                    .iter()
                    .map(|bytes| decode_frame(protocol, bytes))
                    .collect(),
            )
        })
        .collect()
}

fn quic_admitted_schedule_matches_in_memory(protocol: ProtocolSelection) {
    // Trace run: three scripted clients through impairing relays, no waiting.
    let (mut qh, mut clock, relays, mut clients, grants, mut steps) =
        three_clients(protocol, IMPAIRED, 10);
    for (index, client) in clients.iter_mut().enumerate() {
        for seq in 1..=60u64 {
            let bytes = lockstep_bytes(client.version, client.epoch, seq, index, seq % 2 == 0);
            client.send(bytes);
        }
    }
    let peers: Vec<PeerHandle> = clients.iter().map(|client| client.peer).collect();
    let mut decided: BTreeMap<PeerHandle, Vec<u64>> = BTreeMap::new();
    let more = pump_until(
        &mut qh,
        &mut clock,
        10,
        "every scripted seq decided",
        |_, report| {
            for client in clients.iter_mut() {
                client.read_available();
            }
            for result in summary(report).results {
                if !result.retained_in_ingress {
                    decided
                        .entry(result.peer)
                        .or_default()
                        .push(result.seq.expect("scripted frames decode"));
                }
            }
            peers
                .iter()
                .all(|peer| decided.get(peer).is_some_and(|seqs| seqs.len() == 60))
        },
    );
    steps.extend(more);
    for peer in &peers {
        assert_eq!(
            decided[peer],
            (1..=60).collect::<Vec<u64>>(),
            "each seq decided once, in order"
        );
    }
    settle(&mut qh, &mut clock, 10, &mut steps, &mut clients);
    let dropped: u64 = relays.iter().map(UdpRelay::dropped).sum();
    assert!(dropped >= 1, "the relays impaired the run");
    let illegal = steps
        .iter()
        .flat_map(|(_, report)| summary(report).results)
        .filter(|r| r.status == rejected(RejectionCode::IllegalAction))
        .count();
    assert!(
        illegal > 0,
        "the unsynchronized scripts include illegal commands"
    );

    // Replay every call into a fresh in-memory host.
    let sent: BTreeMap<ConnectionId, Vec<Vec<u8>>> = clients
        .iter()
        .map(|client| (client.conn, client.sent.clone()))
        .collect();
    let replayed = replay(protocol, &steps, &sent, &grants).expect("the trace replays exactly");
    let received: BTreeMap<PeerHandle, Vec<Vec<u8>>> = clients
        .iter()
        .map(|client| (client.peer, client.frame_bytes()))
        .collect();
    assert_eq!(replayed, outcome_of(qh.host(), received));

    // Negative controls: one flipped ingested byte, one removed ingest.
    let first_staged = steps
        .iter()
        .flat_map(|(_, report)| report.calls.iter())
        .find_map(|call| match call {
            HostCall::Ingest {
                conn,
                frame,
                result: IngestDisposition::Staged,
                ..
            } => Some((*conn, *frame)),
            _ => None,
        })
        .expect("a staged ingest");
    let mut flipped = sent.clone();
    let bytes = &mut flipped.get_mut(&first_staged.0).expect("log")[first_staged.1 as usize - 1];
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    assert!(
        replay(protocol, &steps, &flipped, &grants).map_or(true, |outcome| outcome != replayed),
        "a flipped byte diverges"
    );
    let mut removed = steps.clone();
    'remove: for (_, report) in removed.iter_mut() {
        if let Some(index) = report.calls.iter().position(|call| {
            matches!(
                call,
                HostCall::Ingest {
                    result: IngestDisposition::Staged,
                    ..
                }
            )
        }) {
            report.calls.remove(index);
            break 'remove;
        }
    }
    assert!(
        replay(protocol, &removed, &sent, &grants).map_or(true, |outcome| outcome != replayed),
        "a dropped ingest diverges"
    );
    drop(clients);
    drop(relays);

    // Lockstep: impaired QUIC, then in memory, then a clean-relay rerun.
    let (impaired, impaired_dropped) = lockstep_quic(protocol, IMPAIRED);
    assert!(impaired_dropped >= 1);
    let memory = lockstep_memory(protocol, None);
    assert_eq!(memory.executions, LOCKSTEP_COMMANDS);
    assert_eq!(impaired.hash, memory.hash);
    assert_eq!(impaired.captures, memory.captures);
    assert_eq!(
        decoded(protocol, &impaired.delivered),
        decoded(protocol, &memory.delivered)
    );
    assert_eq!(impaired, memory);
    let (clean, _) = lockstep_quic(protocol, RelayConfig::clean());
    assert_eq!(clean, memory);
    // Negative control: one whiff replaced by EndTurn.
    let swapped = lockstep_memory(protocol, Some(150));
    assert_ne!(swapped.hash, memory.hash);
}
both_versions!(quic_admitted_schedule_matches_in_memory);

// ---------------------------------------------------------------------------
// B§14.3 — adapter cases.
// ---------------------------------------------------------------------------

#[test]
fn start_and_pump_validate_host_and_clock() {
    fn assert_send<T: Send>() {}
    assert_send::<QuicHost>();

    let server = bind_server(ServerLimits::v1());
    // A shut-down host.
    let mut host = new_host(V1);
    host.shutdown();
    let hash = host.authority_hash();
    let failure = QuicHost::start(host, server, CLOCK_START).expect_err("closed host");
    assert_eq!(failure.error, QuicHostError::HostClosed);
    assert_eq!(failure.host.authority_hash(), hash);
    assert!(failure.host.is_closed());
    let server = failure.server;
    // A host holding one in-memory binding.
    let mut host = new_host(V1);
    host.bind_peer(all_actors(), full_grants(), CLOCK_START)
        .expect("binds");
    let hash = host.authority_hash();
    let failure = QuicHost::start(host, server, CLOCK_START).expect_err("bound host");
    assert_eq!(failure.error, QuicHostError::HostInUse);
    assert_eq!(failure.host.authority_hash(), hash);
    let server = failure.server;
    // A host with staged ingress and no binding.
    let mut host = new_host(V1);
    let peer = host
        .bind_peer(all_actors(), full_grants(), CLOCK_START)
        .expect("binds");
    let epoch = host.epoch(peer).expect("bound");
    assert_eq!(
        host.ingest(peer, &end_turn(1, epoch, 1, 0, 1), CLOCK_START),
        Ok(IngestDisposition::Staged)
    );
    host.unbind_peer(peer);
    let failure = QuicHost::start(host, server, CLOCK_START).expect_err("staged host");
    assert_eq!(failure.error, QuicHostError::HostInUse);
    let server = failure.server;
    // `now_ms` below the host's.
    let host = new_host(V1);
    let hash = host.authority_hash();
    let failure = QuicHost::start(host, server, CLOCK_START - 1).expect_err("time regression");
    assert_eq!(failure.error, QuicHostError::TimeRegression);
    assert_eq!(failure.host.authority_hash(), hash);
    // The returned host and server are usable.
    let mut clock = TestClock::new();
    let mut qh =
        QuicHost::start(failure.host, failure.server, clock.now()).expect("starts after failures");
    register(&mut qh, &cred_bytes(0), all_actors(), full_grants());

    // A time-regressing pump consumes no transport event.
    let pending = connect_async(ClientLimits::v1(), qh.local_addr(), 1, &cred_bytes(0));
    until("the hello is pending", || {
        qh.transport_totals().pending == 1
    });
    assert_eq!(qh.pump(clock.now() - 1), Err(QuicHostError::TimeRegression));
    assert_eq!(qh.transport_totals().pending, 1, "nothing was decided");
    clock.advance(100);
    let report = qh.pump(clock.now()).expect("pump");
    assert!(
        matches!(
            report.hellos.as_slice(),
            [decision] if matches!(decision.outcome, HelloOutcome::Accepted { .. })
        ),
        "{report:?}"
    );
    let client = pending.join().expect("thread").expect("connects");
    assert_eq!(client.welcome().epoch, epoch_of(INCARNATION, 1));
}

#[test]
fn invitation_registration_validates_and_redacts() {
    let (mut qh, mut clock) = start(V1, ServerLimits::v1());
    for len in [16usize, 31, 33, 128] {
        assert_eq!(
            qh.register_invitation(invitation(&vec![0x5A; len], all_actors(), full_grants())),
            Err(QuicHostError::InvalidCredential),
            "length {len}"
        );
    }
    assert_eq!(INVITATION_CREDENTIAL_BYTES, 32);
    let first = register(&mut qh, &cred_bytes(0), all_actors(), full_grants());
    assert_eq!(first.get(), 1);
    assert_eq!(
        qh.register_invitation(invitation(
            &cred_bytes(0),
            control(&[a()]),
            observer_grants()
        )),
        Err(QuicHostError::DuplicateCredential)
    );
    let many: Vec<_> = (0..1_025).map(eid).collect();
    let too_many = control(&many);
    assert_eq!(
        qh.register_invitation(invitation(&cred_bytes(1), too_many.clone(), full_grants())),
        Err(QuicHostError::Host(HostError::InvalidControl))
    );
    let mut duplicate_entity = full_grants();
    duplicate_entity.entities.push(full(a()));
    assert_eq!(
        qh.register_invitation(invitation(
            &cred_bytes(1),
            all_actors(),
            duplicate_entity.clone()
        )),
        Err(QuicHostError::Host(HostError::InvalidGrants))
    );
    // Combined precedence: length → control → grants → duplicate.
    assert_eq!(
        qh.register_invitation(invitation(
            &[0xA1; 16],
            too_many.clone(),
            duplicate_entity.clone()
        )),
        Err(QuicHostError::InvalidCredential)
    );
    assert_eq!(
        qh.register_invitation(invitation(
            &cred_bytes(0),
            too_many.clone(),
            duplicate_entity.clone()
        )),
        Err(QuicHostError::Host(HostError::InvalidControl))
    );
    assert_eq!(
        qh.register_invitation(invitation(&cred_bytes(0), all_actors(), duplicate_entity)),
        Err(QuicHostError::Host(HostError::InvalidGrants))
    );
    // Ids are never reused after revocation.
    let second = register(&mut qh, &cred_bytes(1), all_actors(), full_grants());
    assert_eq!(second.get(), 2);
    assert_eq!(qh.revoke_invitation(second), Ok(None));
    assert_eq!(
        qh.revoke_invitation(second),
        Err(QuicHostError::UnknownInvitation)
    );
    assert_eq!(
        qh.invitation_binding(second),
        Err(QuicHostError::UnknownInvitation)
    );
    let third = register(&mut qh, &cred_bytes(2), all_actors(), full_grants());
    assert_eq!(third.get(), 3);
    // Fill to MAX_INVITATIONS; the 65th is refused before anything else.
    for fill in 1..=(MAX_INVITATIONS as u8 - 2) {
        register(&mut qh, &[fill; 32], control(&[]), observer_grants());
    }
    assert_eq!(
        qh.register_invitation(invitation(&[0x77; 32], all_actors(), full_grants())),
        Err(QuicHostError::InvitationLimit)
    );
    assert_eq!(
        qh.register_invitation(invitation(&[0x77; 16], too_many, full_grants())),
        Err(QuicHostError::InvitationLimit)
    );

    // Redaction: nothing printed carries the credential's bytes.
    let addr = qh.local_addr();
    let (client, steps) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        V1,
        &cred_bytes(0),
    );
    let printed_invitation = format!(
        "{:?}",
        invitation(&cred_bytes(0), all_actors(), full_grants())
    );
    assert!(
        printed_invitation.contains("Credential(<redacted>)"),
        "{printed_invitation}"
    );
    let mut printed = vec![printed_invitation, format!("{qh:?}")];
    printed.extend(
        hellos(&steps)
            .iter()
            .map(|decision| format!("{decision:?}")),
    );
    printed.extend(steps.iter().map(|(_, report)| format!("{report:?}")));
    let raw = format!("{:?}", cred_bytes(0));
    for text in &printed {
        for needle in [raw.as_str(), "161, 161", "a1a1", "A1A1", "0xa1", "0xA1"] {
            assert!(!text.contains(needle), "credential bytes printed: {text}");
        }
    }
    drop(client);

    // Every Display string is exact.
    let displays = [
        (QuicHostError::HostClosed, "HostClosed at host/quic"),
        (QuicHostError::HostInUse, "HostInUse at host/quic"),
        (QuicHostError::TimeRegression, "TimeRegression at host/quic"),
        (QuicHostError::Poisoned, "Poisoned at host/quic"),
        (
            QuicHostError::InvalidCredential,
            "InvalidCredential at host/quic",
        ),
        (
            QuicHostError::DuplicateCredential,
            "DuplicateCredential at host/quic",
        ),
        (
            QuicHostError::InvitationLimit,
            "InvitationLimit at host/quic",
        ),
        (
            QuicHostError::UnknownInvitation,
            "UnknownInvitation at host/quic",
        ),
        (
            QuicHostError::UnknownConnection,
            "UnknownConnection at host/quic",
        ),
        (
            QuicHostError::Host(HostError::QueueFull),
            "Host at host/quic",
        ),
    ];
    for (error, text) in displays {
        assert_eq!(error.to_string(), text);
    }
}

fn admission_binds_invitation_and_hands_off_epoch(protocol: ProtocolSelection) {
    let (mut qh, mut clock) = start(protocol, ServerLimits::v1());
    let id = register(&mut qh, &cred_bytes(0), all_actors(), full_grants());
    let addr = qh.local_addr();
    let (mut ca, steps) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        protocol,
        &cred_bytes(0),
    );
    let decision = hellos(&steps).pop().expect("decided");
    assert_eq!(
        decision.outcome,
        HelloOutcome::Accepted {
            invitation: id,
            peer: ca.peer
        }
    );
    assert_eq!(ca.peer.get(), 1);
    let welcome = Welcome {
        wire_version: version_byte(protocol),
        epoch: epoch_of(INCARNATION, 1),
    };
    assert_eq!(ca.quic.welcome(), welcome);
    assert_eq!(qh.host().epoch(ca.peer), Ok(welcome.epoch));
    assert_eq!(qh.binding(ca.conn), Some(ca.peer));
    assert_eq!(qh.invitation_binding(id), Ok(Some((ca.conn, ca.peer))));
    ca.read_until("initial state", |c| !c.frames.is_empty());
    assert_eq!(ca.frames[0].event_seq, 1);
    assert_eq!(ca.frames[0].ops, initial_full_state());
    let (_, deciding) = steps.last().expect("deciding pump");
    assert_eq!(
        deciding.calls,
        vec![
            HostCall::BindPeer {
                conn: ca.conn,
                invitation: id,
                result: Ok(ca.peer)
            },
            HostCall::Pump {
                summary: empty_summary()
            },
            HostCall::TakeDelivery {
                peer: ca.peer,
                result: Ok(1)
            },
            HostCall::AcknowledgeDelivery {
                peer: ca.peer,
                through: 1
            },
        ]
    );
    assert_eq!(deciding.frames_sent, 1);
}
both_versions!(admission_binds_invitation_and_hands_off_epoch);

#[test]
fn refusals_map_to_close_codes() {
    let (mut qh, mut clock) = start(V1, ServerLimits::v1());
    let creds: Vec<Vec<u8>> = (0..9u8).map(|i| vec![0x40 + i; 32]).collect();
    for cred in &creds {
        register(&mut qh, cred, control(&[]), observer_grants());
    }
    let revoked = register(&mut qh, &[0x7E; 32], control(&[]), observer_grants());
    qh.revoke_invitation(revoked).expect("revokes");
    let before = snapshot(qh.host());
    let no_bind = |steps: &[Step]| {
        assert!(
            !calls(steps)
                .iter()
                .any(|call| matches!(call, HostCall::BindPeer { result: Ok(_), .. })),
            "a refusal creates no binding"
        );
    };
    let cases: [(Vec<u8>, u8, CloseCode); 5] = [
        (vec![0x99; 32], 1, CloseCode::AuthRefused),
        (vec![0x40; 16], 1, CloseCode::AuthRefused),
        (vec![0x40; 128], 1, CloseCode::AuthRefused),
        (vec![0x7E; 32], 1, CloseCode::AuthRefused),
        (creds[0].clone(), 2, CloseCode::VersionRefused),
    ];
    for (cred, version, code) in cases {
        let (error, decision, steps) =
            connect_refused(&mut qh, &mut clock, ClientLimits::v1(), version, &cred);
        assert_eq!(
            error,
            ConnectError::Refused {
                code: u64::from(code.as_u32())
            }
        );
        assert_eq!(decision.outcome, HelloOutcome::Refused { code });
        no_bind(&steps);
        assert!(
            calls(&steps)
                .iter()
                .all(|call| matches!(call, HostCall::Pump { .. })),
            "no host call but the pump"
        );
    }
    // Eight bound; the ninth gets ServerBusy without consuming a handle.
    let addr = qh.local_addr();
    let mut clients = Vec::new();
    for (index, cred) in creds.iter().take(8).enumerate() {
        let (client, _) = connect(&mut qh, &mut clock, 100, addr, ClientLimits::v1(), V1, cred);
        assert_eq!(client.peer.get(), index as u64 + 1);
        clients.push(client);
    }
    let (error, decision, steps) =
        connect_refused(&mut qh, &mut clock, ClientLimits::v1(), 1, &creds[8]);
    assert_eq!(error, ConnectError::Refused { code: 7 });
    assert_eq!(
        decision.outcome,
        HelloOutcome::Refused {
            code: CloseCode::ServerBusy
        }
    );
    no_bind(&steps);
    assert!(calls(&steps).iter().any(|call| matches!(
        call,
        HostCall::BindPeer {
            result: Err(HostError::ServerBusy),
            ..
        }
    )));
    assert_eq!(snapshot(qh.host()), before, "authority unchanged");
    // Slots return to the live count once the Closed events are polled.
    pump_until(&mut qh, &mut clock, 100, "slots released", |qh, _| {
        qh.transport_totals().connections == 8
    });
    // After one disconnect the ninth binds with handle 9.
    let first = clients.remove(0);
    let fence = qh.disconnect(first.conn).expect("bound");
    assert_eq!(fence.cause, FenceCause::Disconnected);
    let (ninth, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        V1,
        &creds[8],
    );
    assert_eq!(ninth.peer.get(), 9, "no refused hello consumed a handle");
    pump_until(&mut qh, &mut clock, 100, "slots released", |qh, _| {
        qh.transport_totals().connections == 8
    });
    assert_eq!(snapshot(qh.host()), before, "authority unchanged");
    drop(first);
}

#[test]
fn superseding_connection_fences_previous() {
    let (mut qh, mut clock) = start(V1, ServerLimits::v1());
    let id = register(&mut qh, &cred_bytes(0), all_actors(), full_grants());
    let addr = qh.local_addr();
    let (mut a1, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        V1,
        &cred_bytes(0),
    );
    a1.read_until("initial state", |c| !c.frames.is_empty());
    // Block admission with the capture ledger (A§4 case 10).
    let mut seq = 1;
    for _ in 0..1_024 {
        let (_, report) = drive_one(&mut qh, &mut clock, &mut a1, &mut seq, PACE);
        let pumped = summary(&report);
        assert_eq!(pumped.applied, PACE as usize);
        assert_eq!(pumped.backpressured, 0);
    }
    assert_eq!(qh.host().last_capture(), 4_096);
    assert_eq!(qh.host().capture_acknowledged(), 0);
    let (_, report) = drive_one(&mut qh, &mut clock, &mut a1, &mut seq, 1);
    let head = AdmissionResult {
        peer: a1.peer,
        seq: Some(4_097),
        status: rejected(RejectionCode::QueueFull),
        cached: false,
        retained_in_ingress: true,
    };
    assert_eq!(
        report.calls[..2],
        [
            HostCall::Ingest {
                conn: a1.conn,
                peer: a1.peer,
                frame: 4_097,
                len: a1.sent[4_096].len(),
                result: IngestDisposition::Staged
            },
            HostCall::Pump {
                summary: PumpSummary {
                    backpressured: 1,
                    results: vec![head.clone()],
                    ..empty_summary()
                }
            },
        ]
    );
    // A1 holds every receipt before the supersession (a guard).
    a1.read_until("A1's receipts", |c| c.receipts.len() == 4_096);

    // A2 presents the same invitation.
    let pending = connect_async(ClientLimits::v1(), addr, 1, &cred_bytes(0));
    let (decision, steps) = decide_hello(&mut qh, &mut clock, 100);
    let HelloOutcome::Accepted { invitation, peer } = decision.outcome else {
        panic!("expected Accepted, got {decision:?}");
    };
    assert_eq!((invitation, peer.get()), (id, 2), "handle 2 as approved");
    let (_, deciding) = steps.last().expect("deciding pump");
    assert_eq!(
        deciding.fences,
        vec![Fence {
            conn: a1.conn,
            peer: a1.peer,
            cause: FenceCause::Superseded
        }]
    );
    let unbind = deciding
        .calls
        .iter()
        .position(|call| *call == HostCall::UnbindPeer { peer: a1.peer })
        .expect("unbind");
    let bind = deciding
        .calls
        .iter()
        .position(|call| matches!(call, HostCall::BindPeer { result: Ok(p), .. } if *p == peer))
        .expect("bind");
    assert!(unbind < bind);
    assert!(
        summary(deciding).results.contains(&result(
            a1.peer,
            Some(4_097),
            rejected(RejectionCode::SessionExpired)
        )),
        "the staged command is SessionExpired as approved"
    );
    for (_, waiting) in &steps[..steps.len() - 1] {
        let pumped = summary(waiting);
        assert_eq!(
            (pumped.backpressured, pumped.results),
            (1, vec![head.clone()])
        );
    }
    let quic = pending.join().expect("thread").expect("connects");
    let epoch = qh.host().epoch(peer).expect("bound");
    assert_eq!(epoch, epoch_of(INCARNATION, 2));
    let mut a2 = Client {
        quic,
        conn: decision.conn,
        peer,
        invitation,
        epoch,
        protocol: V1,
        version: 1,
        sent: Vec::new(),
        frames: Vec::new(),
        receipts: Vec::new(),
        closed: None,
    };
    assert_eq!(qh.invitation_binding(id), Ok(Some((a2.conn, peer))));

    // A1's view: receipts 1..=4096 in order, none for 4097, then code 9.
    assert_eq!(a1.read_to_close(), CloseReason::Peer { code: 9 });
    let expected: Vec<(u64, ReceiptStatus)> = (1..=4_096).map(|s| (s, APPLIED)).collect();
    let got: Vec<(u64, ReceiptStatus)> = a1.receipts.iter().map(|r| (r.0, r.1)).collect();
    assert_eq!(got, expected);

    // The new client's command applies once captures are acknowledged.
    qh.acknowledge_captures(4_096).expect("trusted ack");
    let report = exchange(&mut qh, &mut clock, &mut a2, end_turn(1, epoch, 1, 0, 2));
    assert_eq!(
        summary(&report).results,
        vec![result(peer, Some(1), APPLIED)]
    );
    a2.read_until("A2's receipt", |c| c.has_receipt(1));
    assert_eq!(qh.host().executions(), 4_097);
}

fn fencing_closes_with_session_fenced(protocol: ProtocolSelection) {
    let (mut qh, mut clock) = start(protocol, ServerLimits::v1());
    let addr = qh.local_addr();
    let v = version_byte(protocol);
    let i1 = register(&mut qh, &cred_bytes(0), control(&[a()]), full_grants());
    let i2 = register(&mut qh, &cred_bytes(1), control(&[a()]), full_grants());
    let i3 = register(&mut qh, &cred_bytes(2), control(&[a()]), full_grants());
    let i4 = register(&mut qh, &cred_bytes(3), control(&[a()]), full_grants());

    let check_fenced = |qh: &mut QuicHost, client: &mut Client, fence: Fence, cause: FenceCause| {
        assert_eq!(
            fence,
            Fence {
                conn: client.conn,
                peer: client.peer,
                cause
            }
        );
        assert_eq!(client.read_to_close(), CloseReason::Peer { code: 9 });
        assert_eq!(qh.host().epoch(client.peer), Err(HostError::UnknownPeer));
        assert_eq!(qh.binding(client.conn), None);
        let frames = client.frames.len();
        assert_eq!(client.read_available(), 0);
        assert!(client.quic.try_recv().is_err(), "no bytes after the close");
        assert_eq!(client.frames.len(), frames);
    };

    // disconnect.
    let (mut c1, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        protocol,
        &cred_bytes(0),
    );
    c1.read_until("initial", |c| !c.frames.is_empty());
    let fence = qh.disconnect(c1.conn).expect("bound");
    check_fenced(&mut qh, &mut c1, fence, FenceCause::Disconnected);
    assert_eq!(qh.invitation_binding(i1), Ok(None));
    assert_eq!(
        qh.disconnect(c1.conn),
        Err(QuicHostError::UnknownConnection)
    );

    // revoke_invitation.
    let (mut c2, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        protocol,
        &cred_bytes(1),
    );
    c2.read_until("initial", |c| !c.frames.is_empty());
    let fence = qh
        .revoke_invitation(i2)
        .expect("registered")
        .expect("bound");
    check_fenced(&mut qh, &mut c2, fence, FenceCause::Revoked);

    // Narrowing set_invitation_grants: clears A's health flag.
    let (mut c3, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        protocol,
        &cred_bytes(2),
    );
    c3.read_until("initial", |c| !c.frames.is_empty());
    let mut narrowed = full_grants();
    narrowed.entities[0].health = false;
    let change = qh.set_invitation_grants(i3, narrowed).expect("registered");
    let GrantChange::Fenced(fence) = change else {
        panic!("expected a fence, got {change:?}");
    };
    check_fenced(&mut qh, &mut c3, fence, FenceCause::Narrowed);
    // A reconnect with the same invitation gets a fresh state without A's health.
    let (mut c3b, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        protocol,
        &cred_bytes(2),
    );
    c3b.read_until("fresh state", |c| !c.frames.is_empty());
    assert_eq!(
        c3b.frames[0].ops,
        vec![
            enter(1),
            enter(2),
            enter(3),
            health(2, 10, 10, false),
            health(3, 3, 3, false),
            turn(Some(1), 0),
        ]
    );

    // Control: widening keeps the connection and sends the new Health op.
    assert_eq!(
        qh.set_invitation_grants(i3, full_grants()),
        Ok(GrantChange::Widened)
    );
    let through = c3b.frames.len() + 1;
    let steps = pump_until(
        &mut qh,
        &mut clock,
        100,
        "widened state sent",
        |_, report| report.frames_sent > 0,
    );
    assert!(steps.iter().all(|(_, report)| report.fences.is_empty()));
    c3b.read_until("widened state", |c| c.frames.len() == through);
    assert_eq!(c3b.frames[1].ops, vec![health(1, 10, 10, false)]);
    assert!(c3b.closed.is_none());
    // Control: an unbound invitation is only stored.
    assert_eq!(
        qh.set_invitation_grants(i4, observer_grants()),
        Ok(GrantChange::Stored)
    );
    // Control: removing A's actor makes the next command NotAuthorized, no fence.
    qh.set_invitation_control(i3, control(&[]))
        .expect("registered");
    let bytes = end_turn(v, c3b.epoch, 1, 0, 1);
    let report = exchange(&mut qh, &mut clock, &mut c3b, bytes);
    assert_eq!(
        summary(&report).results,
        vec![result(
            c3b.peer,
            Some(1),
            rejected(RejectionCode::NotAuthorized)
        )]
    );
    assert!(report.fences.is_empty());
    c3b.read_until("NotAuthorized receipt", |c| c.has_receipt(1));
    assert_eq!(
        c3b.receipts.last().map(|r| r.1),
        Some(rejected(RejectionCode::NotAuthorized))
    );
    assert!(c3b.closed.is_none());
    assert_eq!(qh.host().executions(), 0);
}
both_versions!(fencing_closes_with_session_fenced);

// -- Stall cases (A§4: 12, 14, 16). ------------------------------------------

/// A (all actors, full grants, reading, connected first) and B (no
/// control, observer grants, stalled client, not reading).
fn stall_setup(protocol: ProtocolSelection) -> (QuicHost, TestClock, Client, Client) {
    let (mut qh, mut clock) = start(protocol, stall_server_limits());
    register(&mut qh, &cred_bytes(0), all_actors(), full_grants());
    register(&mut qh, &cred_bytes(1), control(&[]), observer_grants());
    let addr = qh.local_addr();
    let (mut ca, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        protocol,
        &cred_bytes(0),
    );
    let (cb, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        stalled_client_limits(),
        protocol,
        &cred_bytes(1),
    );
    assert!(ca.conn < cb.conn, "A is drained first");
    ca.read_until("A initial state", |c| !c.frames.is_empty());
    (qh, clock, ca, cb)
}

/// The A§3.4 fill: c-pumps until `stop`, tracking B's stall with the mirror.
struct Fill {
    /// `T`: `now_ms` after the connects.
    t: u64,
    seq: u64,
    mirror: StallMirror,
    /// `(c-pump, step)`.
    steps: Vec<(u64, Step)>,
    first_stall: Option<u64>,
    first_backpressure: Option<u64>,
}

impl Fill {
    fn new(clock: &TestClock) -> Self {
        Self {
            t: clock.now(),
            seq: 1,
            mirror: StallMirror::new(),
            steps: Vec::new(),
            first_stall: None,
            first_backpressure: None,
        }
    }

    fn run(
        &mut self,
        qh: &mut QuicHost,
        clock: &mut TestClock,
        ca: &mut Client,
        b: PeerHandle,
        mut stop: impl FnMut(&QuicPumpReport) -> bool,
    ) {
        loop {
            let index = self.steps.len() as u64 + 1;
            assert!(index <= 2_000, "the fill reaches its stop");
            let step = drive_one(qh, clock, ca, &mut self.seq, PACE);
            self.mirror.feed(step.0, &step.1, b);
            if let Some(since) = self.mirror.since {
                self.first_stall.get_or_insert(index);
                assert!(
                    step.0 - since < STALL_TIMEOUT_MS,
                    "no slow-consumer timer yet"
                );
            }
            if summary(&step.1).backpressured == 1 {
                self.first_backpressure.get_or_insert(index);
            }
            assert!(step.1.fences.is_empty());
            let done = stop(&step.1);
            self.steps.push((index, step));
            if done {
                return;
            }
        }
    }

    fn step(&self, index: u64) -> &Step {
        &self.steps[index as usize - 1].1
    }

    fn all(&self) -> Vec<Step> {
        self.steps.iter().map(|(_, step)| step.clone()).collect()
    }

    /// Asserts the A§4 fill counts through the first `backpressured: 1`.
    fn assert_stall_and_backpressure(&self, a: &Client, b: PeerHandle) -> u64 {
        let t0 = self.t + 69_300;
        assert_eq!(
            self.first_stall,
            Some(693),
            "B's event_seq 2,770 is the first refused"
        );
        let stall = self.step(693);
        assert_eq!(stall.0, t0);
        assert_eq!(taken(&stall.1, b), Some(4));
        assert!(!stall
            .1
            .calls
            .iter()
            .any(|call| matches!(call, HostCall::AcknowledgeDelivery { peer, .. } if *peer == b)));
        assert_eq!(
            acked_through(&self.all()[..692], b),
            B_HANDED,
            "B took event_seq 1..=2,769"
        );
        assert_eq!(self.first_backpressure, Some(725));
        let pressured = self.step(725);
        assert_eq!(pressured.0, t0 + 3_200);
        let pumped = summary(&pressured.1);
        assert_eq!(pumped.backpressured, 1);
        assert_eq!(
            pumped.results.last(),
            Some(&AdmissionResult {
                peer: a.peer,
                seq: Some(2_897),
                status: rejected(RejectionCode::QueueFull),
                cached: false,
                retained_in_ingress: true,
            })
        );
        t0
    }
}

fn admitted_seqs(steps: &[Step], peer: PeerHandle) -> Vec<u64> {
    steps
        .iter()
        .flat_map(|(_, report)| summary(report).results)
        .filter(|r| r.peer == peer && !r.retained_in_ingress && r.status == APPLIED)
        .map(|r| r.seq.expect("decoded"))
        .collect()
}

#[test]
fn ingest_queue_full_holds_frame_and_resumes() {
    let (mut qh, mut clock, mut ca, mut cb) = stall_setup(V1);
    let mut fill = Fill::new(&clock);
    // 1. Fill until a frame is held.
    fill.run(&mut qh, &mut clock, &mut ca, cb.peer, |report| {
        report.held == 1
    });
    let t0 = fill.assert_stall_and_backpressure(&ca, cb.peer);
    let (held_at, held_step) = fill.steps.last().expect("steps").clone();
    assert_eq!(held_at, 757);
    assert_eq!(held_step.0, t0 + 6_400);
    assert!(held_step.1.calls.contains(&HostCall::Ingest {
        conn: ca.conn,
        peer: ca.peer,
        frame: 3_025,
        len: ca.sent[3_024].len(),
        result: IngestDisposition::Refused {
            status: rejected(RejectionCode::QueueFull)
        },
    }));
    let all = fill.all();
    assert_eq!(
        staged_ordinals(&all, ca.conn),
        (1..=3_024).collect::<Vec<u64>>()
    );
    assert_eq!(
        admitted_seqs(&all, ca.peer),
        (1..=2_896).collect::<Vec<u64>>()
    );
    // A's staged ingress reached 128 (2,897..=3,024, staged, not admitted).
    assert_eq!(3_024 - 2_896, MAX_DRAIN_PER_CONNECTION as u64);

    // 2. Park A's reader (no pump): 8 queued plus 1 parked.
    let mut seq = fill.seq;
    assert_eq!(seq, 3_029);
    for _ in 0..6 {
        ca.send(end_turn_for(1, ca.epoch, seq));
        seq += 1;
    }
    let conn_a = ca.conn;
    until("A's reader parks", || {
        qh.connection_stats(conn_a)
            .is_some_and(|s| s.inbound_frames == 8 && s.reader_parked)
    });
    assert_eq!(seq - 1, 3_034);

    // 3. B reads (no pump) everything its transport took.
    let conn_b = cb.conn;
    until("B drains its transport", || {
        cb.read_available();
        cb.frames.len() as u64 == B_HANDED
            && qh
                .connection_stats(conn_b)
                .is_some_and(|s| s.outbound_frames == 0)
    });
    let b_seqs: Vec<u64> = cb.frames.iter().map(|f| f.event_seq).collect();
    assert_eq!(b_seqs, (1..=B_HANDED).collect::<Vec<u64>>());

    // 4. Resume: each pump after B and A have read and B's queue is empty.
    let mut mirror = fill.mirror;
    let mut resumed: Vec<Step> = Vec::new();
    while ca.receipts.len() < 3_034 {
        assert!(resumed.len() < 1_000, "resume converges");
        until("B's transport drained", || {
            cb.read_available();
            ca.read_available();
            qh.connection_stats(conn_b)
                .is_some_and(|s| s.outbound_frames == 0)
        });
        clock.advance(PACE_STEP_MS);
        let report = qh.pump(clock.now()).expect("pump");
        mirror.feed(clock.now(), &report, cb.peer);
        if let Some(since) = mirror.since {
            assert!(clock.now() - since < STALL_TIMEOUT_MS);
        }
        assert!(report.fences.is_empty());
        resumed.push((clock.now(), report));
        ca.read_available();
    }
    // Resume pump 1: the held retry is refused again, B takes its 128.
    let (now, first) = &resumed[0];
    assert_eq!(*now, t0 + 6_500);
    let retry = HostCall::Ingest {
        conn: ca.conn,
        peer: ca.peer,
        frame: 3_025,
        len: ca.sent[3_024].len(),
        result: IngestDisposition::Refused {
            status: rejected(RejectionCode::QueueFull),
        },
    };
    assert_eq!(first.calls[0], retry);
    assert_eq!(summary(first).backpressured, 1);
    assert_eq!(taken(first, cb.peer), Some(128));
    assert!(first.calls.contains(&HostCall::AcknowledgeDelivery {
        peer: cb.peer,
        through: B_HANDED + 128
    }));
    // Resume pump 2 admits 2,897..=3,024.
    let second = &resumed[1].1;
    assert_eq!(second.calls[0], retry);
    assert_eq!(
        admitted_seqs(&resumed[1..2], ca.peer),
        (2_897..=3_024).collect::<Vec<u64>>()
    );
    // Resume pump 3 stages the same ordinal and drains behind it.
    let third = &resumed[2].1;
    assert_eq!(
        third.calls[0],
        HostCall::Ingest {
            conn: ca.conn,
            peer: ca.peer,
            frame: 3_025,
            len: ca.sent[3_024].len(),
            result: IngestDisposition::Staged,
        }
    );
    assert!(staged_ordinals(&resumed[2..3], ca.conn).len() >= 9);
    assert_eq!(mirror.since, None);

    // Final assertions, as approved with n = 3,034.
    let mut everything = fill.all();
    everything.extend(resumed.iter().cloned());
    assert_eq!(
        staged_ordinals(&everything, ca.conn),
        (1..=3_034).collect::<Vec<u64>>()
    );
    let receipts: Vec<(u64, ReceiptStatus)> = ca.receipts.iter().map(|r| (r.0, r.1)).collect();
    assert_eq!(
        receipts,
        (1..=3_034).map(|s| (s, APPLIED)).collect::<Vec<_>>()
    );
    assert_eq!(qh.host().executions(), 3_034);
    let refused: usize = everything.iter().map(|(_, r)| r.frames_refused).sum();
    assert_eq!(refused, 0);
    assert!(everything.iter().all(|(_, r)| r.fences.is_empty()));
}

#[test]
fn terminal_ingest_refusals_dropped_not_held() {
    let (mut qh, mut clock) = start(V1, ServerLimits::v1());
    register(&mut qh, &cred_bytes(0), all_actors(), full_grants());
    let addr = qh.local_addr();
    let (mut ca, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        V1,
        &cred_bytes(0),
    );
    ca.read_until("initial", |c| !c.frames.is_empty());
    for seq in 1..=100 {
        ca.send(end_turn_for(1, ca.epoch, seq));
    }
    let conn = ca.conn;
    until("all 100 queued at the server", || {
        qh.connection_stats(conn)
            .is_some_and(|s| s.inbound_frames == 100)
    });
    // `now_ms` held: one pump at the same injected time.
    let report = qh.pump(clock.now()).expect("pump");
    let ingests: Vec<(u64, IngestDisposition)> = report
        .calls
        .iter()
        .filter_map(|call| match call {
            HostCall::Ingest { frame, result, .. } => Some((*frame, result.clone())),
            _ => None,
        })
        .collect();
    let expected: Vec<(u64, IngestDisposition)> = (1..=100)
        .map(|frame| {
            let result = if frame <= 80 {
                IngestDisposition::Staged
            } else {
                IngestDisposition::Refused {
                    status: rejected(RejectionCode::RateLimited),
                }
            };
            (frame, result)
        })
        .collect();
    assert_eq!(
        ingests, expected,
        "draining continues past terminal refusals"
    );
    assert_eq!(report.frames_refused, 20);
    assert_eq!(report.held, 0);
    assert_eq!(summary(&report).applied, 80);
    // After 1 s, A's resend of seq 81 is admitted normally.
    clock.advance(1_000);
    let bytes = end_turn_for(1, ca.epoch, 81);
    let report = exchange(&mut qh, &mut clock, &mut ca, bytes);
    assert_eq!(
        summary(&report).results,
        vec![result(ca.peer, Some(81), APPLIED)]
    );
    ca.read_until("81 receipts", |c| c.receipts.len() == 81);
    let receipts: Vec<(u64, ReceiptStatus)> = ca.receipts.iter().map(|r| (r.0, r.1)).collect();
    assert_eq!(receipts, (1..=81).map(|s| (s, APPLIED)).collect::<Vec<_>>());
    assert_eq!(qh.host().executions(), 81);
}

#[test]
fn slow_consumer_fenced_after_stall_timeout() {
    let (mut qh, mut clock, mut ca, mut cb) = stall_setup(V1);
    let mut fill = Fill::new(&clock);
    fill.run(&mut qh, &mut clock, &mut ca, cb.peer, |report| {
        summary(report).backpressured == 1
    });
    let t0 = fill.assert_stall_and_backpressure(&ca, cb.peer);
    assert_eq!(fill.steps.len(), 725);
    let mut mirror = fill.mirror;
    assert_eq!(mirror.since, Some(t0));
    // Just before the timeout: no fence.
    clock.set(t0 + STALL_TIMEOUT_MS - 1);
    let report = qh.pump(clock.now()).expect("pump");
    mirror.feed(clock.now(), &report, cb.peer);
    assert!(report.fences.is_empty());
    assert_eq!(mirror.since, Some(t0));
    // At the timeout: SlowConsumer.
    clock.set(t0 + STALL_TIMEOUT_MS);
    let report = qh.pump(clock.now()).expect("pump");
    assert_eq!(
        report.fences,
        vec![Fence {
            conn: cb.conn,
            peer: cb.peer,
            cause: FenceCause::SlowConsumer
        }]
    );
    assert!(report
        .calls
        .contains(&HostCall::UnbindPeer { peer: cb.peer }));
    assert_eq!(qh.host().epoch(cb.peer), Err(HostError::UnknownPeer));
    // Release: the next pump admits A's blocked head.
    clock.advance(PACE_STEP_MS);
    let report = qh.pump(clock.now()).expect("pump");
    let pumped = summary(&report);
    assert_eq!(
        (pumped.admitted, pumped.applied, pumped.backpressured),
        (4, 4, 0)
    );
    ca.read_until("A's new receipts", |c| c.has_receipt(2_900));
    let tail: Vec<u64> = ca.receipts.iter().map(|r| r.0).skip(2_896).collect();
    assert_eq!(tail, vec![2_897, 2_898, 2_899, 2_900]);
    // B's view: an in-order prefix from event_seq 1, then code 9.
    assert_eq!(cb.read_to_close(), CloseReason::Peer { code: 9 });
    let prefix: Vec<u64> = cb.frames.iter().map(|f| f.event_seq).collect();
    assert!(prefix.len() as u64 <= B_HANDED);
    assert_eq!(prefix, (1..=prefix.len() as u64).collect::<Vec<u64>>());
}

#[test]
fn peer_close_drains_then_unbinds() {
    let (mut qh, mut clock) = start(V1, ServerLimits::v1());
    let id = register(&mut qh, &cred_bytes(0), all_actors(), full_grants());
    let addr = qh.local_addr();

    // (a) Three legal commands, then a Flush close: all three are admitted.
    let (mut ca, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        V1,
        &cred_bytes(0),
    );
    ca.read_until("initial", |c| !c.frames.is_empty());
    for seq in 1..=3 {
        ca.send(end_turn_for(1, ca.epoch, seq));
    }
    let Client {
        quic, conn, peer, ..
    } = ca;
    quic.close(CloseCode::Normal, CloseMode::Flush);
    let steps = pump_until(
        &mut qh,
        &mut clock,
        100,
        "unbound after drain",
        |_, report| report.calls.contains(&HostCall::UnbindPeer { peer }),
    );
    assert_eq!(staged_ordinals(&steps, conn), vec![1, 2, 3]);
    assert_eq!(admitted_seqs(&steps, peer), vec![1, 2, 3]);
    assert_eq!(qh.host().executions(), 3);
    let closes: Vec<(ConnectionId, CloseReason)> = steps
        .iter()
        .flat_map(|(_, report)| report.transport_closes.iter().copied())
        .collect();
    assert!(
        closes.contains(&(conn, CloseReason::Peer { code: 0 })),
        "{closes:?}"
    );
    assert_eq!(qh.binding(conn), None);
    assert_eq!(qh.host().epoch(peer), Err(HostError::UnknownPeer));
    pump_until(&mut qh, &mut clock, 100, "slot released", |qh, _| {
        qh.transport_totals().connections == 0
    });

    // (b) The invitation binds again; a client dropped after a receipt is
    // reported closed and unbound.
    let (mut cb, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        V1,
        &cred_bytes(0),
    );
    assert_eq!(qh.invitation_binding(id), Ok(Some((cb.conn, cb.peer))));
    let bytes = end_turn_for(1, cb.epoch, 1);
    let report = exchange(&mut qh, &mut clock, &mut cb, bytes);
    assert_eq!(
        summary(&report).results,
        vec![result(cb.peer, Some(1), APPLIED)]
    );
    cb.read_until("receipt", |c| c.has_receipt(1));
    let (conn, peer) = (cb.conn, cb.peer);
    drop(cb);
    let mut closed = false;
    let steps = pump_until(
        &mut qh,
        &mut clock,
        100,
        "dropped client unbound",
        |_, report| {
            closed |= report.transport_closes.iter().any(|(c, _)| *c == conn);
            closed && report.calls.contains(&HostCall::UnbindPeer { peer })
        },
    );
    assert!(calls(&steps).contains(&HostCall::UnbindPeer { peer }));
    pump_until(&mut qh, &mut clock, 100, "slot released", |qh, _| {
        qh.transport_totals().connections == 0
    });
    assert_eq!(qh.invitation_binding(id), Ok(None));

    // (c) Idle timeout through a blackholed relay.
    let limits = ServerLimits {
        idle_timeout_ms: 1_000,
        keep_alive_ms: 300,
        ..ServerLimits::v1()
    };
    let (mut qh2, mut clock2) = start(V1, limits);
    let id2 = register(&mut qh2, &cred_bytes(0), all_actors(), full_grants());
    let relay = UdpRelay::start(qh2.local_addr(), RelayConfig::clean());
    let (mut cc, _) = connect(
        &mut qh2,
        &mut clock2,
        100,
        relay.addr(),
        ClientLimits::v1(),
        V1,
        &cred_bytes(0),
    );
    cc.read_until("initial", |c| !c.frames.is_empty());
    let (conn, peer) = (cc.conn, cc.peer);
    relay.set_blackhole(true);
    let steps = pump_until(&mut qh2, &mut clock2, 100, "idle timeout", |_, report| {
        report.calls.contains(&HostCall::UnbindPeer { peer })
    });
    let closes: Vec<(ConnectionId, CloseReason)> = steps
        .iter()
        .flat_map(|(_, report)| report.transport_closes.iter().copied())
        .collect();
    assert!(
        closes.contains(&(conn, CloseReason::IdleTimeout)),
        "{closes:?}"
    );
    pump_until(&mut qh2, &mut clock2, 100, "slot released", |qh, _| {
        qh.transport_totals().connections == 0
    });
    drop(cc);
    let direct = qh2.local_addr();
    let (again, _) = connect(
        &mut qh2,
        &mut clock2,
        100,
        direct,
        ClientLimits::v1(),
        V1,
        &cred_bytes(0),
    );
    assert_eq!(
        qh2.invitation_binding(id2),
        Ok(Some((again.conn, again.peer)))
    );
}

fn graceful_shutdown_orders_refuse_checkpoint_close(protocol: ProtocolSelection) {
    let (mut qh, mut clock, mut ca, mut cb) = stall_setup(protocol);
    let mut fill = Fill::new(&clock);
    fill.run(&mut qh, &mut clock, &mut ca, cb.peer, |report| {
        summary(report).backpressured == 1
    });
    let t0 = fill.assert_stall_and_backpressure(&ca, cb.peer);
    // B stages two EndTurns it cannot control, after admission blocks.
    cb.send(end_turn(cb.version, cb.epoch, 1, 0, 1));
    cb.send(end_turn(cb.version, cb.epoch, 2, 0, 1));
    let conn_b = cb.conn;
    until("B's frames queued", || {
        qh.connection_stats(conn_b)
            .is_some_and(|s| s.inbound_frames == 2)
    });
    fill.run(&mut qh, &mut clock, &mut ca, cb.peer, |report| {
        report.held == 1
    });
    let (held_at, held_step) = fill.steps.last().expect("steps").clone();
    assert_eq!(held_at, 757);
    assert_eq!(held_step.0, t0 + 6_400);
    assert_eq!(held_step.1.held, 1);
    assert!(held_step.1.calls.iter().any(|call| matches!(
        call,
        HostCall::Ingest {
            frame: 3_025,
            result: IngestDisposition::Refused { .. },
            ..
        }
    )));
    let all = fill.all();
    assert_eq!(staged_ordinals(&all, cb.conn), vec![1, 2]);

    // FIFO of staged-but-never-admitted work, from the trace.
    let mut queue: VecDeque<PeerHandle> = VecDeque::new();
    for (_, report) in &all {
        for call in &report.calls {
            match call {
                HostCall::Ingest {
                    peer,
                    result: IngestDisposition::Staged,
                    ..
                } => queue.push_back(*peer),
                HostCall::Pump { summary } => {
                    for _ in 0..summary.admitted {
                        queue.pop_front();
                    }
                }
                _ => {}
            }
        }
    }
    let mut expected_peers = vec![ca.peer; 8];
    expected_peers.extend([cb.peer; 2]);
    expected_peers.extend(vec![ca.peer; 120]);
    assert_eq!(queue.iter().copied().collect::<Vec<_>>(), expected_peers);
    let b_through = acked_through(&all, cb.peer);
    assert_eq!(b_through, B_HANDED);
    let last_b_take = all
        .iter()
        .rev()
        .find_map(|(_, report)| taken(report, cb.peer))
        .expect("B takes");
    assert_eq!(last_b_take, 128);

    let done = qh.shutdown(true);
    assert_eq!(done.refused.len(), 130);
    assert!(done.refused.iter().all(|r| r.seq.is_none()
        && r.status == rejected(RejectionCode::SessionExpired)
        && !r.cached
        && !r.retained_in_ingress));
    assert_eq!(
        done.refused.iter().map(|r| r.peer).collect::<Vec<_>>(),
        expected_peers
    );
    assert_eq!(done.undelivered_frames, 128);
    assert_eq!(done.held_discarded, 1);
    assert_eq!(done.held_discarded, held_step.1.held);
    assert_eq!(done.transport.connections_closed, 2);
    assert!(done.host.is_closed());
    assert_eq!(done.host.last_capture(), 2_896);
    assert_eq!(
        done.host
            .read_captures(0, MAX_CAPTURE_PAGE)
            .expect("readable")
            .len(),
        MAX_CAPTURE_PAGE
    );
    let bytes = match &done.checkpoint {
        Some(Ok(bytes)) => bytes.clone(),
        other => panic!("expected a checkpoint, got {other:?}"),
    };
    let restored = load_checkpoint(&bytes, INCARNATION + 1, clock.now()).expect("restores");
    assert_eq!(restored.authority_hash(), done.host.authority_hash());

    // A received every committed receipt before the Shutdown close.
    assert_eq!(ca.read_to_close(), CloseReason::Peer { code: 1 });
    let receipts: Vec<(u64, ReceiptStatus)> = ca.receipts.iter().map(|r| (r.0, r.1)).collect();
    assert_eq!(
        receipts,
        (1..=2_896).map(|s| (s, APPLIED)).collect::<Vec<_>>()
    );
    drop(cb);
}
both_versions!(graceful_shutdown_orders_refuse_checkpoint_close);

#[test]
fn drop_closes_with_shutdown_code() {
    let (mut qh, mut clock) = start(V1, ServerLimits::v1());
    register(&mut qh, &cred_bytes(0), all_actors(), full_grants());
    let addr = qh.local_addr();
    let (mut ca, _) = connect(
        &mut qh,
        &mut clock,
        100,
        addr,
        ClientLimits::v1(),
        V1,
        &cred_bytes(0),
    );
    ca.read_until("initial", |c| !c.frames.is_empty());
    drop(qh);
    assert_eq!(ca.read_to_close(), CloseReason::Peer { code: 1 });
}
