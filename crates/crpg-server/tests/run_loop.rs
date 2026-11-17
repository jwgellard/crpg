//! T154a acceptance suite (`tasks/T154.md` K1–K17): the run loop's cycle
//! order and cadence, `LocalHost`'s in-memory links, retirement records and
//! `LocalClient`, public API only.
//!
//! Expected values come from the contract, never from the implementation.
//! The schedule is stepped by hand (`HostLoop::step`), so no case asserts a
//! wall-clock duration. A "twin" is a second host built the same way and
//! driven by direct calls in the cycle order (`ingest` per frame, `pump`,
//! `advance`, `take_delivery_entries` then `acknowledge_delivery` through the
//! last, `acknowledge_captures(last_capture())`).

use std::cell::RefCell;
use std::num::{NonZeroU64, NonZeroUsize};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crpg_core::{EntityId, Ulid};
use crpg_net::codec_transfer;
use crpg_net::codec_v2;
use crpg_net::facts_v3::EncounterGrants;
use crpg_net::protocol::{IntentBody, IntentFrame, NetId, LANE_COMBAT};
use crpg_net::protocol_transfer::{ResyncReason, TransferId, TransferUp};
use crpg_net::protocol_v3::{PauseReason, PauseRights};
use crpg_net::transfer_v3;
use crpg_net::transport::Transport;
use crpg_server::host::{
    AdvanceReport, ControlGrant, DisclosureGrants, EntityDisclosure, EntityFactGrant, FactGrants,
    GrantUpdate, Host, HostConfig, HostError, IngestDisposition, PeerHandle, ProtocolSelection,
    TransferState, DEFAULT_HOST_GRANTS, MAX_PEERS,
};
use crpg_server::run::{
    AiHook, CycleReport, HostLoop, Hosting, LinkError, LinkStats, LocalClient, LocalError,
    LocalHost, LoopError, StepReport, MAX_RETIRED_RECORDS, TICK_PERIOD_MS,
};
use crpg_sim::{CombatAction, HistoryWorld};
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");

// ---------------------------------------------------------------------------
// Identities and fixtures (copied from `host_pause.rs`).
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
fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}

fn tb3_value() -> Value {
    serde_json::from_str(FIXTURE).expect("fixture parses")
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
        abilities: vec![strike(), smite(), whiff()],
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

fn config() -> HostConfig {
    HostConfig {
        protocol: ProtocolSelection::V2,
        incarnation: 1,
    }
}

fn end(actor: EntityId) -> CombatAction {
    CombatAction::EndTurn { actor }
}

/// `TB3`: turn based, A then B then C, A active.
fn tb3() -> Host {
    Host::from_history(history_of(&tb3_value()), config(), 0).expect("wraps")
}

/// `P`: control of A, B and C with every disclosure flag, bound at 0.
fn bind(host: &mut Host) -> PeerHandle {
    host.bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds")
}

fn cap(capacity: usize) -> NonZeroUsize {
    NonZeroUsize::new(capacity).expect("nonzero")
}

fn advance_report() -> AdvanceReport {
    AdvanceReport {
        ticked: true,
        timer_ended_turn: None,
        paused: None,
    }
}

/// One lane-0 intent: an `EndTurn` for A in `peer`'s ids, at the host's tick.
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

/// An `EndTurn` for `actor` in `peer`'s ids, at the host's tick.
fn actor_end_frame(host: &Host, peer: PeerHandle, seq: u64, actor: u64) -> Vec<u8> {
    codec_v2::encode_intent(&IntentFrame {
        epoch: host.epoch(peer).expect("bound"),
        lane: LANE_COMBAT,
        seq,
        observed_tick: host.server_tick(),
        actor: n(actor),
        body: IntentBody::EndTurn,
    })
    .expect("encodes")
}

fn published(epoch: [u8; 16], transfer: u64) -> Vec<u8> {
    codec_transfer::encode_up(&TransferUp::Published {
        epoch,
        transfer: TransferId::new(transfer).expect("nonzero"),
    })
    .expect("encodes")
}

fn resync(epoch: [u8; 16], transfer: Option<u64>, reason: ResyncReason) -> Vec<u8> {
    codec_transfer::encode_up(&TransferUp::Resync {
        epoch,
        after: transfer.and_then(TransferId::new),
        reason,
    })
    .expect("encodes")
}

/// One twin cycle: `ingest` per frame, `pump`, `advance`, then delivery
/// through the last accepted ordinal and every capture retired. Returns the
/// entries the client would have received, in order.
fn twin_cycle(host: &mut Host, peer: PeerHandle, frames: &[Vec<u8>], now: u64) -> Vec<Vec<u8>> {
    for frame in frames {
        assert_eq!(host.ingest(peer, frame, now), Ok(IngestDisposition::Staged));
    }
    host.pump(now).expect("pumps");
    host.advance(now).expect("advances");
    let entries = host.take_delivery_entries(peer).expect("bound");
    if let Some(last) = entries.last() {
        host.acknowledge_delivery(peer, last.ordinal).expect("acks");
    }
    host.acknowledge_captures(host.last_capture())
        .expect("acks captures");
    entries.into_iter().map(|entry| entry.bytes).collect()
}

/// Reads every frame the client's down queue holds right now.
fn drain(client: &mut impl Transport) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    while let Some(frame) = client.recv() {
        frames.push(frame);
    }
    frames
}

// ---------------------------------------------------------------------------
// `Rec`: the test-local hosting of K1–K5.
// ---------------------------------------------------------------------------

/// Appends one entry per stage and fails the one stage it is told to, once.
struct Rec {
    log: Rc<RefCell<Vec<String>>>,
    fail_at: Option<usize>,
}

impl Rec {
    fn new() -> (Self, Rc<RefCell<Vec<String>>>) {
        let log = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                log: Rc::clone(&log),
                fail_at: None,
            },
            log,
        )
    }

    fn failing_at(stage: usize) -> Self {
        Self {
            log: Rc::new(RefCell::new(Vec::new())),
            fail_at: Some(stage),
        }
    }

    fn stage(&mut self, index: usize, entry: String) -> Result<(), HostError> {
        self.log.borrow_mut().push(entry);
        if self.fail_at == Some(index) {
            self.fail_at = None;
            return Err(HostError::QueueFull);
        }
        Ok(())
    }
}

impl Hosting for Rec {
    type Error = HostError;

    fn intake(&mut self, now_ms: u64) -> Result<(), HostError> {
        self.stage(0, format!("intake@{now_ms}"))
    }

    fn drive_ai(&mut self, now_ms: u64) -> Result<(), HostError> {
        self.stage(1, format!("ai@{now_ms}"))
    }

    fn advance(&mut self, now_ms: u64) -> Result<AdvanceReport, HostError> {
        self.stage(2, format!("advance@{now_ms}"))?;
        Ok(advance_report())
    }

    fn deliver(&mut self, now_ms: u64) -> Result<(), HostError> {
        self.stage(3, format!("deliver@{now_ms}"))
    }

    fn retire_captures(&mut self) -> Result<(), HostError> {
        self.stage(4, "retire".to_owned())
    }
}

const FIRST_CYCLE: [&str; 5] = ["intake@0", "ai@0", "advance@0", "deliver@0", "retire"];

// ---------------------------------------------------------------------------
// B-1, B-2, B-3: the loop.
// ---------------------------------------------------------------------------

/// K1 (B-1). The grid, the period, `last_ms` and at most one cycle per step.
#[test]
fn cadence_on_the_grid() {
    let (rec, log) = Rec::new();
    let mut host_loop = HostLoop::new(rec, 1_000);
    assert_eq!(TICK_PERIOD_MS, 50);
    assert_eq!(host_loop.period_ms(), 50);
    assert_eq!(host_loop.next_due_ms(), 1_000);
    assert_eq!(host_loop.last_ms(), 1_000);
    assert_eq!(host_loop.cycles(), 0);
    assert_eq!(host_loop.missed(), 0);

    let step = host_loop.step(1_000).expect("runs");
    assert_eq!(
        step,
        StepReport {
            cycle: Some(CycleReport {
                now_ms: 1_000,
                advance: advance_report(),
            }),
            missed: 0,
        }
    );
    assert_eq!(host_loop.next_due_ms(), 1_050);

    let step = host_loop.step(1_049).expect("accepted");
    assert_eq!(
        step,
        StepReport {
            cycle: None,
            missed: 0,
        }
    );
    assert_eq!(host_loop.last_ms(), 1_049);

    let step = host_loop.step(1_050).expect("runs");
    assert_eq!(step.missed, 0);
    assert_eq!(step.cycle.map(|cycle| cycle.now_ms), Some(1_050));
    assert_eq!(host_loop.next_due_ms(), 1_100);

    assert!(host_loop.step(1_100).expect("runs").cycle.is_some());
    assert_eq!(host_loop.cycles(), 3);
    assert_eq!(host_loop.missed(), 0);
    assert_eq!(host_loop.last_ms(), 1_100);
    assert_eq!(log.borrow().len(), 15);

    // Control: an explicit period shifts every slot.
    let (rec, log) = Rec::new();
    let mut host_loop = HostLoop::with_period(rec, 1_000, NonZeroU64::new(25).expect("nonzero"));
    assert_eq!(host_loop.period_ms(), 25);
    assert!(host_loop.step(1_000).expect("runs").cycle.is_some());
    assert!(host_loop.step(1_024).expect("accepted").cycle.is_none());
    assert!(host_loop.step(1_025).expect("runs").cycle.is_some());
    assert_eq!(host_loop.cycles(), 2);
    assert_eq!(log.borrow().len(), 10);
}

/// K2 (B-1). A late step runs one cycle and counts the slots it skipped.
#[test]
fn missed_slots_are_skipped() {
    let (rec, log) = Rec::new();
    let mut host_loop = HostLoop::new(rec, 0);
    assert!(host_loop.step(0).expect("runs").cycle.is_some());

    let step = host_loop.step(210).expect("runs");
    assert_eq!(step.missed, 3);
    assert_eq!(step.cycle.map(|cycle| cycle.now_ms), Some(210));
    assert_eq!(host_loop.next_due_ms(), 250);
    assert_eq!(host_loop.missed(), 3);
    assert_eq!(host_loop.cycles(), 2);

    assert!(host_loop.step(249).expect("accepted").cycle.is_none());
    let step = host_loop.step(250).expect("runs");
    assert_eq!(step.missed, 0);
    assert_eq!(host_loop.next_due_ms(), 300);
    assert_eq!(log.borrow().len(), 15);

    // Control: a step on every slot skips none.
    let (rec, _log) = Rec::new();
    let mut host_loop = HostLoop::new(rec, 0);
    for slot in 0..=5u64 {
        assert!(host_loop.step(50 * slot).expect("runs").cycle.is_some());
    }
    assert_eq!(host_loop.cycles(), 6);
    assert_eq!(host_loop.missed(), 0);
}

/// K3 (B-2). Time never runs backwards; a regression changes nothing.
#[test]
fn time_regression_changes_nothing() {
    let (rec, log) = Rec::new();
    let mut host_loop = HostLoop::new(rec, 100);

    let error = host_loop.step(99).expect_err("regresses");
    assert_eq!(
        error,
        LoopError::TimeRegression {
            last_ms: 100,
            now_ms: 99,
        }
    );
    assert_eq!(error.to_string(), "TimeRegression at run/step");
    assert!(log.borrow().is_empty());
    assert_eq!(host_loop.next_due_ms(), 100);
    assert_eq!(host_loop.last_ms(), 100);
    assert_eq!(host_loop.cycles(), 0);

    assert!(host_loop.step(100).expect("runs").cycle.is_some());
    // `now == last` is accepted and is not a slot.
    assert!(host_loop.step(100).expect("accepted").cycle.is_none());

    let error = host_loop.step(99).expect_err("regresses");
    assert_eq!(
        error,
        LoopError::TimeRegression {
            last_ms: 100,
            now_ms: 99,
        }
    );
    assert_eq!(log.borrow().len(), 5);
    assert_eq!(host_loop.next_due_ms(), 150);

    // The other `Display` text: a stage's failure through the loop.
    let mut host_loop = HostLoop::new(Rec::failing_at(0), 0);
    let error = host_loop.step(0).expect_err("fails");
    assert_eq!(error, LoopError::Hosting(HostError::QueueFull));
    assert_eq!(
        error.to_string(),
        "Hosting at run/step: QueueFull at host/admission"
    );
}

/// K4 (B-3). One cycle runs the five stages in the one order.
#[test]
fn cycle_order() {
    let (rec, log) = Rec::new();
    let mut host_loop = HostLoop::new(rec, 0);
    let step = host_loop.step(0).expect("runs");
    assert_eq!(*log.borrow(), FIRST_CYCLE.to_vec());
    assert_eq!(
        step,
        StepReport {
            cycle: Some(CycleReport {
                now_ms: 0,
                advance: advance_report(),
            }),
            missed: 0,
        }
    );

    host_loop.step(50).expect("runs");
    let mut expected = FIRST_CYCLE.to_vec();
    expected.extend(["intake@50", "ai@50", "advance@50", "deliver@50", "retire"]);
    assert_eq!(*log.borrow(), expected);
}

/// K5 (B-3). The first failing stage ends the cycle; no later stage runs.
#[test]
fn first_error_stops_the_cycle() {
    for index in 0..FIRST_CYCLE.len() {
        let rec = Rec::failing_at(index);
        let log = Rc::clone(&rec.log);
        let mut host_loop = HostLoop::new(rec, 0);
        let error = host_loop.step(0).expect_err("fails");
        assert_eq!(error, LoopError::Hosting(HostError::QueueFull));
        assert_eq!(*log.borrow(), FIRST_CYCLE[..=index].to_vec());
        assert_eq!(host_loop.next_due_ms(), 50);
        assert_eq!(host_loop.cycles(), 1);
        assert_eq!(host_loop.missed(), 0);

        // The failure was once; the next cycle runs every stage.
        host_loop.step(50).expect("runs");
        assert_eq!(host_loop.cycles(), 2);
        assert_eq!(host_loop.missed(), 0);
        let mut expected = FIRST_CYCLE[..=index].to_vec();
        expected.extend(["intake@50", "ai@50", "advance@50", "deliver@50", "retire"]);
        assert_eq!(*log.borrow(), expected);
    }
}

// ---------------------------------------------------------------------------
// B-4, B-8: pause and the AI stage.
// ---------------------------------------------------------------------------

type Seen = Arc<Mutex<Vec<(u64, Vec<EntityId>)>>>;

/// A hook that records `(server_tick, awaiting_orders)` and ends the first
/// awaiting combatant's turn.
fn observing_hook(seen: Seen) -> AiHook {
    Box::new(move |host: &mut Host, _now_ms: u64| {
        let awaiting = host.awaiting_orders();
        seen.lock()
            .expect("recorder")
            .push((host.server_tick(), awaiting.clone()));
        if let Some(head) = awaiting.first() {
            host.host_action(&end(*head))?;
        }
        Ok(())
    })
}

/// K6 (B-4, B-8). A paused host runs every stage; only `advance` declines.
#[test]
fn pause_runs_every_stage_and_the_ai_hook() {
    let mut host = tb3();
    host.pause(0).expect("pauses");
    let mut hosting = LocalHost::new(host);
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    hosting.set_ai(Some(observing_hook(Arc::clone(&seen))));
    let mut host_loop = HostLoop::new(hosting, 0);

    let step = host_loop.step(0).expect("runs");
    assert_eq!(
        step.cycle.expect("runs a cycle").advance,
        AdvanceReport {
            ticked: false,
            timer_ended_turn: None,
            paused: Some(PauseReason::Operator),
        }
    );
    assert_eq!(*seen.lock().expect("recorder"), vec![(0u64, vec![a()])]);
    assert_eq!(host_loop.hosting().host().awaiting_orders(), vec![b()]);
    assert_eq!(host_loop.hosting().host().server_tick(), 0);

    // Control: running, the hook decides before `advance` and the tick runs.
    let mut hosting = LocalHost::new(tb3());
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    hosting.set_ai(Some(observing_hook(Arc::clone(&seen))));
    let mut host_loop = HostLoop::new(hosting, 0);
    let step = host_loop.step(0).expect("runs");
    assert_eq!(step.cycle.expect("runs a cycle").advance, advance_report());
    assert_eq!(*seen.lock().expect("recorder"), vec![(0u64, vec![a()])]);
    assert_eq!(host_loop.hosting().host().server_tick(), 1);

    // Control: no hook, nothing is decided.
    let mut host = tb3();
    host.pause(0).expect("pauses");
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    host_loop.step(0).expect("runs");
    assert_eq!(host_loop.hosting().host().awaiting_orders(), vec![a()]);
    assert_eq!(host_loop.hosting().host().server_tick(), 0);
}

// ---------------------------------------------------------------------------
// B-5 to B-10: `LocalHost` and `LocalClient`.
// ---------------------------------------------------------------------------

/// K7 (B-5). An unknown handle outranks a link; a second attach is refused.
#[test]
fn attach_precedence() {
    let mut host = tb3();
    let p = bind(&mut host);
    let q = host
        .bind_peer(control(&[a()]), full_grants(), 0)
        .expect("binds");
    host.unbind_peer(q);

    let mut hosting = LocalHost::new(host);
    assert_eq!(hosting.link_stats(q), None);
    assert_eq!(
        hosting.attach(q, cap(4)).err(),
        Some(LocalError::UnknownPeer)
    );
    assert_eq!(
        LocalError::UnknownPeer.to_string(),
        "UnknownPeer at run/attach"
    );

    let client = hosting.attach(p, cap(4)).expect("attaches");
    assert_eq!(
        hosting.attach(p, cap(4)).err(),
        Some(LocalError::AlreadyAttached)
    );
    assert_eq!(
        LocalError::AlreadyAttached.to_string(),
        "AlreadyAttached at run/attach"
    );
    assert_eq!(hosting.link_stats(p), Some(LinkStats::default()));
    assert_eq!(hosting.link_stats(q), None);
    drop(client);
}

/// K8 (B-6, B-7, B-8). One framed command through the loop equals the twin.
#[test]
fn round_trip_equals_direct_calls() {
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    let frame = end_frame(host_loop.hosting().host(), p, 1);
    client.send(&frame).expect("sends");

    let mut twin = tb3();
    assert_eq!(bind(&mut twin), p);
    let expected = twin_cycle(&mut twin, p, std::slice::from_ref(&frame), 0);

    let step = host_loop.step(0).expect("runs");
    assert_eq!(step.cycle.map(|cycle| cycle.now_ms), Some(0));
    let received = drain(&mut client);
    assert_eq!(received, expected);
    assert_eq!(
        host_loop.hosting().host().authority_hash(),
        twin.authority_hash()
    );
    assert_eq!(host_loop.hosting().host().server_tick(), 1);
    assert_eq!(twin.server_tick(), 1);
    assert_eq!(
        host_loop.hosting().host().capture_acknowledged(),
        host_loop.hosting().host().last_capture()
    );
    assert_eq!(twin.capture_acknowledged(), twin.last_capture());

    let stats = host_loop.hosting().link_stats(p).expect("link");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    assert!(!stats.held);
    assert_eq!(stats.sent as usize, expected.len());

    // The next cycle sends no frame and still equals the twin.
    let expected_next = twin_cycle(&mut twin, p, &[], 50);
    host_loop.step(50).expect("runs");
    assert_eq!(drain(&mut client), expected_next);
    assert_eq!(
        host_loop.hosting().host().authority_hash(),
        twin.authority_hash()
    );

    // Control: with no frame the client receives a different first cycle.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut control_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut quiet = control_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    control_loop.step(0).expect("runs");
    assert_ne!(drain(&mut quiet), received);
}

/// K9 (B-6). A transfer frame's first byte routes it to `ingest_transfer`.
#[test]
fn transfer_frames_route_to_ingest_transfer() {
    let mut host = tb3();
    let w = host
        .bind_snapshot_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    assert_eq!(
        host.transfer_state(w),
        Ok(Some(TransferState {
            next_transfer: 1,
            in_flight: None,
        }))
    );
    let mut narrowed = full_grants();
    narrowed.abilities = vec![strike()];
    assert_eq!(
        host.set_grants(w, narrowed),
        Ok(GrantUpdate::SnapshotStaged)
    );
    assert_eq!(
        host.transfer_state(w),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: Some(1),
        }))
    );
    let epoch = host.epoch(w).expect("bound");
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(w, cap(64))
        .expect("attaches");
    client.send(&published(epoch, 1)).expect("sends");

    host_loop.step(0).expect("runs");
    assert_eq!(
        host_loop.hosting().host().transfer_state(w),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: None,
        }))
    );
    let stats = host_loop.hosting().link_stats(w).expect("link");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);

    // Control: no frame leaves the transfer in flight.
    let mut host = tb3();
    let w = host
        .bind_snapshot_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let mut narrowed = full_grants();
    narrowed.abilities = vec![strike()];
    assert_eq!(
        host.set_grants(w, narrowed),
        Ok(GrantUpdate::SnapshotStaged)
    );
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let _client = host_loop
        .hosting_mut()
        .attach(w, cap(64))
        .expect("attaches");
    host_loop.step(0).expect("runs");
    assert_eq!(
        host_loop.hosting().host().transfer_state(w),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: Some(1),
        }))
    );

    // Control: an unknown transfer is refused, counted and dropped.
    let mut host = tb3();
    let w = host
        .bind_snapshot_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let mut narrowed = full_grants();
    narrowed.abilities = vec![strike()];
    assert_eq!(
        host.set_grants(w, narrowed),
        Ok(GrantUpdate::SnapshotStaged)
    );
    let epoch = host.epoch(w).expect("bound");
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(w, cap(64))
        .expect("attaches");
    client.send(&published(epoch, 9)).expect("sends");
    host_loop.step(0).expect("runs");
    let stats = host_loop.hosting().link_stats(w).expect("link stays");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 1);
    assert_eq!(
        host_loop.hosting().host().transfer_state(w),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: Some(1),
        }))
    );
}

/// K10 (B-6). Admission refusals are counted and dropped, never held.
#[test]
fn refusals_are_counted_and_dropped() {
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(200))
        .expect("attaches");
    let frames: Vec<Vec<u8>> = (1..=130)
        .map(|seq| end_frame(host_loop.hosting().host(), p, seq))
        .collect();
    for frame in &frames {
        client.send(frame).expect("sends");
    }

    host_loop.step(0).expect("runs");
    let stats = host_loop.hosting().link_stats(p).expect("link");
    assert_eq!(stats.received, 128);
    assert_eq!(stats.refused, 48);
    assert!(!stats.held);

    host_loop.step(50).expect("runs");
    let stats = host_loop.hosting().link_stats(p).expect("link");
    assert_eq!(stats.received, 130);
    assert_eq!(stats.refused, 48);

    // Control: the burst allowance alone refuses nothing.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(200))
        .expect("attaches");
    let frames: Vec<Vec<u8>> = (1..=80)
        .map(|seq| end_frame(host_loop.hosting().host(), p, seq))
        .collect();
    for frame in &frames {
        client.send(frame).expect("sends");
    }
    host_loop.step(0).expect("runs");
    let stats = host_loop.hosting().link_stats(p).expect("link");
    assert_eq!(stats.received, 80);
    assert_eq!(stats.refused, 0);
}

/// K11 (B-6). A `QueueFull` holds the frame and resumes once room returns.
///
/// The contract's "within 3 cycles" release is one short of what B-6/B-7
/// imply once S (capacity 1) is read: its down queue accepts and
/// acknowledges exactly one log entry per `deliver`, and `intake` (which
/// pumps) runs before `deliver`. Cycle 1 is still blocked (S's log is 128
/// before that deliver); cycle 2 frees one ingress slot; cycle 3 admits the
/// held frame but the frame still queued behind it is refused and holds
/// again; cycle 4 admits it. Hence four, derived from the cycle order and
/// B-7's "acknowledge only what was accepted", not read off a run.
#[test]
fn queue_full_holds_and_resumes() {
    // S is never read: its full log blocks admission, so P's staged frames
    // reach the per-peer ingress cap and one frame is held.
    let mut host = tb3();
    let p = bind(&mut host);
    let s = host
        .bind_peer(control(&[]), full_grants(), 0)
        .expect("binds");
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut p_client = host_loop
        .hosting_mut()
        .attach(p, cap(4_096))
        .expect("attaches");
    let mut s_client = host_loop.hosting_mut().attach(s, cap(1)).expect("attaches");

    let mut sent = 0u64;
    let mut cycle = 0u64;
    let mut held = false;
    while cycle < 2_000 {
        drain(&mut p_client);
        for _ in 0..2 {
            let actor = 1 + (sent % 3);
            let frame = actor_end_frame(host_loop.hosting().host(), p, sent + 1, actor);
            p_client.send(&frame).expect("sends");
            sent += 1;
        }
        host_loop.step(50 * cycle).expect("runs");
        cycle += 1;
        let stats = host_loop.hosting().link_stats(p).expect("link");
        if stats.held {
            assert_eq!(stats.refused, 0);
            assert!(
                stats.received == sent || stats.received + 1 == sent,
                "received {} of {sent}",
                stats.received
            );
            held = true;
            break;
        }
    }
    assert!(held, "no hold in 2,000 cycles");

    // P stops; S's client is read dry each cycle, and the hold clears.
    let mut released = false;
    for _ in 0..4 {
        drain(&mut s_client);
        drain(&mut p_client);
        host_loop.step(50 * cycle).expect("runs");
        cycle += 1;
        let stats = host_loop.hosting().link_stats(p).expect("link");
        if !stats.held && stats.received == sent {
            assert_eq!(stats.refused, 0);
            released = true;
            break;
        }
    }
    assert!(released, "the hold did not clear within 4 cycles");

    // Control: S read every cycle never blocks admission. Its queue holds
    // the two commands a cycle P sends, so reading it keeps the log drained;
    // capacity 1 accepts only one entry per deliver and would still fill S's
    // log (the contract's control cannot hold for 300 cycles at capacity 1).
    let mut host = tb3();
    let p = bind(&mut host);
    let s = host
        .bind_peer(control(&[]), full_grants(), 0)
        .expect("binds");
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut p_client = host_loop
        .hosting_mut()
        .attach(p, cap(4_096))
        .expect("attaches");
    let mut s_client = host_loop.hosting_mut().attach(s, cap(2)).expect("attaches");
    let mut sent = 0u64;
    for cycle in 0..300u64 {
        drain(&mut p_client);
        drain(&mut s_client);
        for _ in 0..2 {
            let actor = 1 + (sent % 3);
            let frame = actor_end_frame(host_loop.hosting().host(), p, sent + 1, actor);
            p_client.send(&frame).expect("sends");
            sent += 1;
        }
        host_loop.step(50 * cycle).expect("runs");
        assert!(
            !host_loop.hosting().link_stats(p).expect("link").held,
            "held in control cycle {cycle}"
        );
        assert_eq!(
            host_loop.hosting().link_stats(p).expect("link").refused,
            0,
            "refused in control cycle {cycle}"
        );
    }
}

/// K12 (B-6, B-7, B-9). A link ends only by retirement, and always records it.
#[test]
fn links_end_only_by_retirement() {
    // (a) A dropped client is seen at intake.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    drop(client);
    assert!(host_loop.step(0).expect("runs").cycle.is_some());
    assert_eq!(host_loop.hosting().link_stats(p), None);
    assert_eq!(
        host_loop.hosting().host().epoch(p),
        Err(HostError::UnknownPeer)
    );
    let retired = host_loop.hosting_mut().take_retired();
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].0, p);
    assert_eq!(retired[0].1.received, 0);
    assert!(host_loop.hosting_mut().take_retired().is_empty());

    // (b) An unbound peer's frame is refused `Unauthenticated`, then retired.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    host_loop.step(0).expect("runs");
    assert!(host_loop.hosting_mut().take_retired().is_empty());
    let frame = end_frame(host_loop.hosting().host(), p, 1);
    let queued = drain(&mut client);
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
    // The frames sent before the unbind are still readable, in order.
    assert!(!queued.is_empty());
    assert_eq!(client.recv(), None);
    assert!(client.is_closed());
    assert_eq!(client.send(&frame), Err(LinkError::Closed));

    // (c) Without a frame the link retires at deliver (`UnknownPeer`).
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    host_loop.step(0).expect("runs");
    host_loop.hosting_mut().host_mut().unbind_peer(p);
    host_loop.step(50).expect("runs");
    assert_eq!(host_loop.hosting().link_stats(p), None);
    let retired = host_loop.hosting_mut().take_retired();
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].0, p);
    assert_eq!(retired[0].1.received, 0);
    assert_eq!(retired[0].1.refused, 0);
    drop(client);

    // Control: an attended, bound link stays and records nothing.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let _client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    for slot in 0..3u64 {
        host_loop.step(50 * slot).expect("runs");
        assert!(host_loop.hosting().link_stats(p).is_some());
        assert!(host_loop.hosting_mut().take_retired().is_empty());
    }
}

/// K13 (B-7). A full down queue acknowledges only what it accepted.
#[test]
fn full_down_queue_acknowledges_only_what_was_sent() {
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop.hosting_mut().attach(p, cap(1)).expect("attaches");
    let frame = end_frame(host_loop.hosting().host(), p, 1);
    client.send(&frame).expect("sends");

    let mut twin = tb3();
    assert_eq!(bind(&mut twin), p);
    let expected = twin_cycle(&mut twin, p, std::slice::from_ref(&frame), 0);
    let count = expected.len();
    assert!(count >= 2, "the twin has {count} entries");

    host_loop.step(0).expect("runs");
    assert_eq!(host_loop.hosting().link_stats(p).expect("link").sent, 1);
    assert_eq!(client.recv(), Some(expected[0].clone()));

    for slot in 1..count as u64 {
        host_loop.step(50 * slot).expect("runs");
        assert_eq!(client.recv(), Some(expected[slot as usize].clone()));
    }
    let stats = host_loop.hosting().link_stats(p).expect("link");
    assert_eq!(stats.sent as usize, count);

    // Control: a deep queue takes every entry in one cycle.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    let control_frame = end_frame(host_loop.hosting().host(), p, 1);
    client.send(&control_frame).expect("sends");
    host_loop.step(0).expect("runs");
    assert_eq!(drain(&mut client), expected);
    assert_eq!(
        host_loop.hosting().link_stats(p).expect("link").sent as usize,
        count
    );
}

/// K14 (B-10). The client's queue errors and the closed flag.
#[test]
fn local_client_errors() {
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop.hosting_mut().attach(p, cap(2)).expect("attaches");

    let first = end_frame(host_loop.hosting().host(), p, 1);
    let second = actor_end_frame(host_loop.hosting().host(), p, 2, 2);
    assert_eq!(client.send(&first), Ok(()));
    assert_eq!(client.send(&second), Ok(()));
    assert_eq!(client.send(&first), Err(LinkError::Full));
    assert_eq!(LinkError::Full.to_string(), "Full at run/link");
    assert_eq!(LinkError::Closed.to_string(), "Closed at run/link");

    host_loop.step(0).expect("runs");
    drop(host_loop);
    assert!(client.recv().is_some());
    assert!(client.recv().is_some());
    assert_eq!(client.recv(), None);
    assert!(client.is_closed());
    assert_eq!(client.send(b"later"), Err(LinkError::Closed));
}

/// K15 (Reading 14). Loop state is never saved; the loop is `Send`.
#[test]
fn loop_state_is_not_saved() {
    fn assert_send<T: Send>() {}
    assert_send::<LocalHost>();
    assert_send::<LocalClient>();
    assert_send::<HostLoop<LocalHost>>();

    let mut host = tb3();
    let p = bind(&mut host);
    let mut hosting = LocalHost::new(host);
    let _client = hosting.attach(p, cap(64)).expect("attaches");
    hosting.set_ai(Some(Box::new(|_host: &mut Host, _now_ms: u64| Ok(()))));
    let mut host_loop = HostLoop::new(hosting, 0);
    for slot in 0..3u64 {
        host_loop.step(50 * slot).expect("runs");
    }
    let saved = host_loop.hosting().host().save_checkpoint().expect("saves");

    let mut twin = tb3();
    assert_eq!(bind(&mut twin), p);
    for slot in 0..3u64 {
        twin.pump(50 * slot).expect("pumps");
        twin.advance(50 * slot).expect("advances");
        if let Some(last) = twin.take_delivery_entries(p).expect("bound").last() {
            twin.acknowledge_delivery(p, last.ordinal).expect("acks");
        }
        twin.acknowledge_captures(twin.last_capture())
            .expect("acks captures");
    }
    assert_eq!(saved, twin.save_checkpoint().expect("saves"));
}

/// K16 (R-6, Reading 5). `run.rs` names no clock, thread, socket, AI type or
/// hash collection, and never blocks.
#[test]
fn run_rs_source_rule() {
    let source = include_str!("../src/run.rs");
    let code: String = source
        .lines()
        .map(|line| match line.find("//") {
            Some(index) => &line[..index],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n");
    for token in [
        "Instant",
        "SystemTime",
        "thread::",
        "sleep",
        "recv_timeout",
        "UdpSocket",
        "TcpListener",
        ".recv()",
        ".send(",
        "crpg_ai",
        "HashMap",
        "HashSet",
    ] {
        assert!(!code.contains(token), "run.rs must not name {token}");
    }
}

/// K17 (B-9). Retirement records are bounded; the oldest is dropped first.
#[test]
fn retired_records_are_bounded() {
    assert_eq!(MAX_RETIRED_RECORDS, 64);
    assert_eq!(MAX_PEERS, 8);
    let mut host_loop = HostLoop::new(LocalHost::new(tb3()), 0);
    let mut handles = Vec::new();

    for round in 0..=8u64 {
        let count = if round <= 7 { 8 } else { 1 };
        let now = 50 * round;
        let mut round_peers = Vec::new();
        for _ in 0..count {
            let peer = host_loop
                .hosting_mut()
                .host_mut()
                .bind_peer(control(&[]), full_grants(), now)
                .expect("binds");
            let client = host_loop
                .hosting_mut()
                .attach(peer, cap(1))
                .expect("attaches");
            drop(client);
            handles.push(peer);
            round_peers.push(peer);
        }
        let step = host_loop.step(now).expect("runs");
        assert!(step.cycle.is_some());
        assert_eq!(step.missed, 0);
        for peer in round_peers {
            assert_eq!(host_loop.hosting().link_stats(peer), None);
            assert_eq!(
                host_loop.hosting().host().epoch(peer),
                Err(HostError::UnknownPeer)
            );
        }
        if round <= 7 {
            assert_eq!(host_loop.hosting().retired_dropped(), 0);
        } else {
            assert_eq!(host_loop.hosting().retired_dropped(), 1);
        }
    }

    let retired = host_loop.hosting_mut().take_retired();
    assert_eq!(retired.len(), 64);
    for (index, (peer, stats)) in retired.iter().enumerate() {
        assert_eq!(*peer, handles[index + 1]);
        assert_eq!(stats.received, 0);
        assert_eq!(stats.refused, 0);
    }
    assert_eq!(retired.first().expect("first").0, handles[1]);
    assert_eq!(retired.last().expect("last").0, handles[64]);
    assert!(host_loop.hosting_mut().take_retired().is_empty());
    assert_eq!(host_loop.hosting().retired_dropped(), 1);

    // Control: eight rounds fill exactly 64 records and drop none.
    let mut host_loop = HostLoop::new(LocalHost::new(tb3()), 0);
    let mut handles = Vec::new();
    for round in 0..=7u64 {
        let now = 50 * round;
        for _ in 0..8 {
            let peer = host_loop
                .hosting_mut()
                .host_mut()
                .bind_peer(control(&[]), full_grants(), now)
                .expect("binds");
            let client = host_loop
                .hosting_mut()
                .attach(peer, cap(1))
                .expect("attaches");
            drop(client);
            handles.push(peer);
        }
        host_loop.step(now).expect("runs");
    }
    let retired = host_loop.hosting_mut().take_retired();
    assert_eq!(retired.len(), 64);
    assert_eq!(retired.first().expect("first").0, handles[0]);
    assert_eq!(retired.last().expect("last").0, handles[63]);
    assert_eq!(host_loop.hosting().retired_dropped(), 0);
}

// ---------------------------------------------------------------------------
// T154a fix round 1, part A (gaps 1–9): one test per gap. Each case has a
// control beside it: the same setup with the one difference removed,
// asserting the other outcome.
// ---------------------------------------------------------------------------

/// Gap 1 (B-4; H). A paused host with a link still runs every stage: a frame
/// sent while paused is applied, delivery advances `sent`, and captures are
/// retired. Kills p02 (delivery skipped while paused), p03 (intake skipped),
/// p04 (captures skipped) and p05 (pump skipped).
#[test]
fn paused_link_still_intakes_delivers_and_retires_captures() {
    let mut host = tb3();
    let p = bind(&mut host);
    host.pause(0).expect("pauses");
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    let frame = end_frame(host_loop.hosting().host(), p, 1);
    client.send(&frame).expect("sends");
    let step = host_loop.step(0).expect("runs");
    assert_eq!(
        step.cycle.expect("runs a cycle").advance,
        AdvanceReport {
            ticked: false,
            timer_ended_turn: None,
            paused: Some(PauseReason::Operator),
        }
    );

    // The twin runs the same cycle by hand while paused.
    let mut twin = tb3();
    assert_eq!(bind(&mut twin), p);
    twin.pause(0).expect("pauses");
    assert_eq!(twin.ingest(p, &frame, 0), Ok(IngestDisposition::Staged));
    twin.pump(0).expect("pumps");
    assert_eq!(
        twin.advance(0).expect("advances"),
        AdvanceReport {
            ticked: false,
            timer_ended_turn: None,
            paused: Some(PauseReason::Operator),
        }
    );
    let entries = twin.take_delivery_entries(p).expect("bound");
    if let Some(last) = entries.last() {
        twin.acknowledge_delivery(p, last.ordinal).expect("acks");
    }
    twin.acknowledge_captures(twin.last_capture())
        .expect("acks captures");

    // The frame was applied (intake and pump ran): same post-state as the twin.
    assert_eq!(
        host_loop.hosting().host().authority_hash(),
        twin.authority_hash()
    );
    assert_eq!(host_loop.hosting().host().awaiting_orders(), vec![b()]);
    assert_eq!(twin.awaiting_orders(), vec![b()]);
    assert_eq!(host_loop.hosting().host().server_tick(), 0);
    // Delivery ran: the client receives the twin's entries, counted in `sent`.
    let received = drain(&mut client);
    assert_eq!(
        received,
        entries
            .into_iter()
            .map(|entry| entry.bytes)
            .collect::<Vec<_>>()
    );
    assert!(!received.is_empty());
    let stats = host_loop.hosting().link_stats(p).expect("link");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    assert_eq!(stats.sent as usize, received.len());
    // Captures were retired.
    assert_eq!(
        host_loop.hosting().host().capture_acknowledged(),
        host_loop.hosting().host().last_capture()
    );
    assert_eq!(twin.capture_acknowledged(), twin.last_capture());
    assert!(host_loop.hosting_mut().take_retired().is_empty());

    // Control: the same run unpaused ticks and delivers the same way.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut control = HostLoop::new(LocalHost::new(host), 0);
    let mut quiet = control.hosting_mut().attach(p, cap(64)).expect("attaches");
    let frame = end_frame(control.hosting().host(), p, 1);
    quiet.send(&frame).expect("sends");
    let step = control.step(0).expect("runs");
    assert_eq!(step.cycle.expect("runs a cycle").advance, advance_report());
    assert_eq!(control.hosting().host().server_tick(), 1);
    assert!(!drain(&mut quiet).is_empty());
    assert_eq!(
        control.hosting().host().capture_acknowledged(),
        control.hosting().host().last_capture()
    );
}

/// Gap 2 (B-6/B-9; H). A departure seen at intake still retires at deliver,
/// after the pump: a client that sends a command then drops still has its
/// command applied. Kills i18 (retire at intake, before the pump, so the
/// staged command is refused `SessionExpired`).
#[test]
fn dropped_client_command_survives_until_deliver() {
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    let frame = end_frame(host_loop.hosting().host(), p, 1);
    client.send(&frame).expect("sends");
    drop(client);
    host_loop.step(0).expect("runs");
    assert_eq!(host_loop.hosting().link_stats(p), None);
    let retired = host_loop.hosting_mut().take_retired();
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].0, p);
    assert_eq!(retired[0].1.received, 1);
    assert_eq!(retired[0].1.refused, 0);
    assert_eq!(retired[0].1.sent, 0);

    // The command took effect: the host matches a twin that received it.
    let mut twin = tb3();
    assert_eq!(bind(&mut twin), p);
    assert_eq!(twin.ingest(p, &frame, 0), Ok(IngestDisposition::Staged));
    twin.pump(0).expect("pumps");
    twin.advance(0).expect("advances");
    twin.unbind_peer(p);
    twin.acknowledge_captures(twin.last_capture())
        .expect("acks captures");
    assert_eq!(
        host_loop.hosting().host().authority_hash(),
        twin.authority_hash()
    );
    assert_eq!(host_loop.hosting().host().server_tick(), 1);
    assert_eq!(
        host_loop.hosting().host().awaiting_orders(),
        twin.awaiting_orders()
    );

    // Control: dropped with no command, the state is the command-less twin's.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut control = HostLoop::new(LocalHost::new(host), 0);
    let idle = control.hosting_mut().attach(p, cap(64)).expect("attaches");
    drop(idle);
    control.step(0).expect("runs");
    let retired = control.hosting_mut().take_retired();
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].1.received, 0);
    let mut twin = tb3();
    assert_eq!(bind(&mut twin), p);
    twin.pump(0).expect("pumps");
    twin.advance(0).expect("advances");
    twin.unbind_peer(p);
    twin.acknowledge_captures(twin.last_capture())
        .expect("acks captures");
    assert_eq!(
        control.hosting().host().authority_hash(),
        twin.authority_hash()
    );
    assert_ne!(
        control.hosting().host().authority_hash(),
        host_loop.hosting().host().authority_hash()
    );
}

/// Gap 3 (B-6/B-7; H). K12(a) drops before the first cycle, where the bind
/// entries still queued mask a missing departed mark. Drained first, the next
/// cycle carries no entries, so only the intake mark retires the link. Kills
/// i17 (the disconnect never marked) and d01 (no retire at deliver step 1).
#[test]
fn drained_then_dropped_link_retires_on_the_next_step() {
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    host_loop.step(0).expect("runs");
    assert!(host_loop.hosting().link_stats(p).is_some());
    drain(&mut client);
    drop(client);
    host_loop.step(50).expect("runs");
    assert_eq!(host_loop.hosting().link_stats(p), None);
    assert_eq!(
        host_loop.hosting().host().epoch(p),
        Err(HostError::UnknownPeer)
    );
    let retired = host_loop.hosting_mut().take_retired();
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].0, p);
    assert_eq!(retired[0].1.received, 0);
    assert_eq!(retired[0].1.refused, 0);

    // Control: drained but attended, the link stays and records nothing.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut control = HostLoop::new(LocalHost::new(host), 0);
    let mut quiet = control.hosting_mut().attach(p, cap(64)).expect("attaches");
    control.step(0).expect("runs");
    drain(&mut quiet);
    control.step(50).expect("runs");
    assert!(control.hosting().link_stats(p).is_some());
    assert!(control.hosting_mut().take_retired().is_empty());
}

/// Gap 4 (B-6(3); H). The pump runs even with no links: a command staged
/// straight into the host is applied by the step. Kills i20 (no pump when the
/// link table is empty).
#[test]
fn pump_runs_with_no_links_attached() {
    let mut host = tb3();
    let p = bind(&mut host);
    let frame = end_frame(&host, p, 1);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    assert_eq!(
        host_loop.hosting_mut().host_mut().ingest(p, &frame, 0),
        Ok(IngestDisposition::Staged)
    );
    host_loop.step(0).expect("runs");

    let mut twin = tb3();
    assert_eq!(bind(&mut twin), p);
    assert_eq!(twin.ingest(p, &frame, 0), Ok(IngestDisposition::Staged));
    twin.pump(0).expect("pumps");
    twin.advance(0).expect("advances");
    twin.acknowledge_captures(twin.last_capture())
        .expect("acks captures");
    assert_eq!(
        host_loop.hosting().host().authority_hash(),
        twin.authority_hash()
    );
    assert_eq!(host_loop.hosting().host().server_tick(), 1);

    // Control: nothing staged, the step ticks but applies nothing.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut control = HostLoop::new(LocalHost::new(host), 0);
    control.step(0).expect("runs");
    let mut twin = tb3();
    assert_eq!(bind(&mut twin), p);
    twin.pump(0).expect("pumps");
    twin.advance(0).expect("advances");
    twin.acknowledge_captures(twin.last_capture())
        .expect("acks captures");
    assert_eq!(
        control.hosting().host().authority_hash(),
        twin.authority_hash()
    );
    assert_ne!(
        control.hosting().host().authority_hash(),
        host_loop.hosting().host().authority_hash()
    );
}

/// Gap 5 (B-6; M). Intake walks the links in ascending `PeerHandle` order no
/// matter the attach order: two peers' commands match a twin driven by hand
/// in ascending order. Kills i24 (descending intake, which stages the frames
/// in the other order).
#[test]
fn intake_processes_peers_in_ascending_handle_order() {
    let mut host = tb3();
    let p = bind(&mut host);
    let q = host
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    assert!(p < q);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    // Attached descending: the intake order still governs.
    let mut q_client = host_loop
        .hosting_mut()
        .attach(q, cap(64))
        .expect("attaches");
    let mut p_client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    let p_frame = actor_end_frame(host_loop.hosting().host(), p, 1, 1);
    let q_frame = actor_end_frame(host_loop.hosting().host(), q, 1, 2);
    p_client.send(&p_frame).expect("sends");
    q_client.send(&q_frame).expect("sends");
    host_loop.step(0).expect("runs");

    // The twin ingests in ascending handle order, then pumps once.
    let mut twin = tb3();
    assert_eq!(bind(&mut twin), p);
    assert_eq!(
        twin.bind_peer(control(&[a(), b(), c()]), full_grants(), 0),
        Ok(q)
    );
    assert_eq!(twin.ingest(p, &p_frame, 0), Ok(IngestDisposition::Staged));
    assert_eq!(twin.ingest(q, &q_frame, 0), Ok(IngestDisposition::Staged));
    let summary = twin.pump(0).expect("pumps");
    assert_eq!(summary.admitted, 2);
    assert_eq!(summary.applied, 2);
    assert_eq!(summary.rejected, 0);
    twin.advance(0).expect("advances");
    let p_entries = twin.take_delivery_entries(p).expect("bound");
    if let Some(last) = p_entries.last() {
        twin.acknowledge_delivery(p, last.ordinal).expect("acks");
    }
    let q_entries = twin.take_delivery_entries(q).expect("bound");
    if let Some(last) = q_entries.last() {
        twin.acknowledge_delivery(q, last.ordinal).expect("acks");
    }
    twin.acknowledge_captures(twin.last_capture())
        .expect("acks captures");

    // The setup is order-sensitive: a twin driven descending disagrees.
    let mut flipped = tb3();
    assert_eq!(bind(&mut flipped), p);
    assert_eq!(
        flipped.bind_peer(control(&[a(), b(), c()]), full_grants(), 0),
        Ok(q)
    );
    assert_eq!(
        flipped.ingest(q, &q_frame, 0),
        Ok(IngestDisposition::Staged)
    );
    assert_eq!(
        flipped.ingest(p, &p_frame, 0),
        Ok(IngestDisposition::Staged)
    );
    flipped.pump(0).expect("pumps");
    flipped.advance(0).expect("advances");
    flipped
        .acknowledge_captures(flipped.last_capture())
        .expect("acks captures");
    assert_ne!(flipped.authority_hash(), twin.authority_hash());

    // Ascending, P ends A's turn and Q ends B's: both apply, nothing refused.
    let p_stats = host_loop.hosting().link_stats(p).expect("link");
    assert_eq!(p_stats.received, 1);
    assert_eq!(p_stats.refused, 0);
    let q_stats = host_loop.hosting().link_stats(q).expect("link");
    assert_eq!(q_stats.received, 1);
    assert_eq!(q_stats.refused, 0);
    assert_eq!(
        host_loop.hosting().host().authority_hash(),
        twin.authority_hash()
    );
    let p_got = drain(&mut p_client);
    assert_eq!(
        p_got,
        p_entries
            .into_iter()
            .map(|entry| entry.bytes)
            .collect::<Vec<_>>()
    );
    let q_got = drain(&mut q_client);
    assert_eq!(
        q_got,
        q_entries
            .into_iter()
            .map(|entry| entry.bytes)
            .collect::<Vec<_>>()
    );
    assert_ne!(p_got, q_got);

    // Control: attached ascending, the same frames give the same outcome.
    let mut host = tb3();
    let p = bind(&mut host);
    let q = host
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let mut control = HostLoop::new(LocalHost::new(host), 0);
    let mut p_client = control.hosting_mut().attach(p, cap(64)).expect("attaches");
    let mut q_client = control.hosting_mut().attach(q, cap(64)).expect("attaches");
    let p_frame = actor_end_frame(control.hosting().host(), p, 1, 1);
    let q_frame = actor_end_frame(control.hosting().host(), q, 1, 2);
    p_client.send(&p_frame).expect("sends");
    q_client.send(&q_frame).expect("sends");
    control.step(0).expect("runs");
    assert_eq!(
        control.hosting().host().authority_hash(),
        host_loop.hosting().host().authority_hash()
    );
    assert_eq!(drain(&mut p_client), p_got);
    assert_eq!(drain(&mut q_client), q_got);
}

/// Gap 6 (B-6(1); M). K11 allows "sent or one fewer" at the hold. Held over
/// several cycles, `received` stays flat; on resume nothing is lost or
/// reordered. Kills i03 (re-held frame dropped), i04 (a re-held frame falls
/// through to the fresh drain) and i13 (a fresh hold keeps draining).
#[test]
fn held_frame_blocks_its_peer_until_room_returns() {
    // S is never read: its full log blocks admission, so P's staged frames
    // reach the per-peer ingress cap and one frame is held.
    let mut host = tb3();
    let p = bind(&mut host);
    let s = host
        .bind_peer(control(&[]), full_grants(), 0)
        .expect("binds");
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut p_client = host_loop
        .hosting_mut()
        .attach(p, cap(4_096))
        .expect("attaches");
    let mut s_client = host_loop.hosting_mut().attach(s, cap(1)).expect("attaches");

    let mut sent = 0u64;
    let mut cycle = 0u64;
    while cycle < 2_000 {
        drain(&mut p_client);
        for _ in 0..2 {
            let actor = 1 + (sent % 3);
            let frame = actor_end_frame(host_loop.hosting().host(), p, sent + 1, actor);
            p_client.send(&frame).expect("sends");
            sent += 1;
        }
        host_loop.step(50 * cycle).expect("runs");
        cycle += 1;
        if host_loop.hosting().link_stats(p).expect("link").held {
            break;
        }
    }
    let held_received = host_loop.hosting().link_stats(p).expect("link").received;
    assert!(host_loop.hosting().link_stats(p).expect("link").held);
    assert_eq!(host_loop.hosting().link_stats(p).expect("link").refused, 0);
    assert!(held_received == sent || held_received + 1 == sent);

    // While held, further cycles take nothing more: `received` is flat and
    // nothing is refused.
    for _ in 0..5 {
        drain(&mut p_client);
        for _ in 0..2 {
            let actor = 1 + (sent % 3);
            let frame = actor_end_frame(host_loop.hosting().host(), p, sent + 1, actor);
            p_client.send(&frame).expect("sends");
            sent += 1;
        }
        host_loop.step(50 * cycle).expect("runs");
        cycle += 1;
        let stats = host_loop.hosting().link_stats(p).expect("link");
        assert!(stats.held);
        assert_eq!(stats.received, held_received);
        assert_eq!(stats.refused, 0);
    }

    // Release: P stops and S is read dry. Every sent frame is taken exactly
    // once, in order, so the rotating `EndTurn`s stay valid: none refused.
    let mut released = false;
    for _ in 0..20 {
        drain(&mut s_client);
        drain(&mut p_client);
        host_loop.step(50 * cycle).expect("runs");
        cycle += 1;
        let stats = host_loop.hosting().link_stats(p).expect("link");
        if !stats.held && stats.received == sent {
            assert_eq!(stats.refused, 0);
            released = true;
            break;
        }
    }
    assert!(released, "the hold did not clear");

    // Control: S read every cycle never blocks admission, so nothing holds.
    let mut host = tb3();
    let p = bind(&mut host);
    let s = host
        .bind_peer(control(&[]), full_grants(), 0)
        .expect("binds");
    let mut control = HostLoop::new(LocalHost::new(host), 0);
    let mut p_client = control
        .hosting_mut()
        .attach(p, cap(4_096))
        .expect("attaches");
    let mut s_client = control.hosting_mut().attach(s, cap(2)).expect("attaches");
    let mut sent = 0u64;
    for cycle in 0..100u64 {
        drain(&mut p_client);
        drain(&mut s_client);
        for _ in 0..2 {
            let actor = 1 + (sent % 3);
            let frame = actor_end_frame(control.hosting().host(), p, sent + 1, actor);
            p_client.send(&frame).expect("sends");
            sent += 1;
        }
        control.step(50 * cycle).expect("runs");
        assert!(
            !control.hosting().link_stats(p).expect("link").held,
            "held in control cycle {cycle}"
        );
        assert_eq!(control.hosting().link_stats(p).expect("link").refused, 0);
    }
}

/// Gap 7 (B-1; M). `missed()` sums across skipping steps, a failing cycle
/// past several slots still moves the schedule and the total, and the grid
/// saturates at `u64::MAX`. Kills s16 (total overwritten), s25 (a failure
/// rolls the total back) and s11 (wrapping `next_due_ms`).
#[test]
fn missed_slots_sum_and_saturate() {
    // `missed()` sums over two skipping steps.
    let (rec, _) = Rec::new();
    let mut host_loop = HostLoop::new(rec, 0);
    host_loop.step(0).expect("runs");
    let step = host_loop.step(210).expect("runs");
    assert_eq!(step.missed, 3);
    let step = host_loop.step(420).expect("runs");
    assert_eq!(step.missed, 3);
    assert_eq!(host_loop.missed(), 6);
    assert_eq!(host_loop.cycles(), 3);
    assert_eq!(host_loop.next_due_ms(), 450);

    // Control: every slot stepped, nothing missed.
    let (rec, _) = Rec::new();
    let mut control = HostLoop::new(rec, 0);
    for slot in 0..3u64 {
        let step = control.step(50 * slot).expect("runs");
        assert_eq!(step.missed, 0);
    }
    assert_eq!(control.missed(), 0);
    assert_eq!(control.cycles(), 3);

    // A failing cycle past several slots still moves the schedule and total.
    let mut host_loop = HostLoop::new(Rec::failing_at(0), 0);
    let error = host_loop.step(210).expect_err("fails");
    assert_eq!(error, LoopError::Hosting(HostError::QueueFull));
    assert_eq!(host_loop.cycles(), 1);
    assert_eq!(host_loop.missed(), 4);
    assert_eq!(host_loop.next_due_ms(), 250);
    host_loop.step(250).expect("runs");
    assert_eq!(host_loop.cycles(), 2);
    assert_eq!(host_loop.missed(), 4);

    // Control: the same jump without a failure runs and counts the same skip.
    let (rec, _) = Rec::new();
    let mut control = HostLoop::new(rec, 0);
    let step = control.step(210).expect("runs");
    assert_eq!(step.missed, 4);
    assert_eq!(control.missed(), 4);

    // The grid saturates instead of wrapping past `u64::MAX`.
    let (rec, _) = Rec::new();
    let mut host_loop = HostLoop::new(rec, u64::MAX - 10);
    let step = host_loop.step(u64::MAX).expect("runs");
    assert_eq!(step.missed, 0);
    assert_eq!(step.cycle.map(|cycle| cycle.now_ms), Some(u64::MAX));
    assert_eq!(host_loop.next_due_ms(), u64::MAX);
    assert_eq!(host_loop.missed(), 0);

    // Control: the start slot itself runs and already saturates the next due.
    let (rec, _) = Rec::new();
    let mut control = HostLoop::new(rec, u64::MAX - 10);
    assert!(control.step(u64::MAX - 10).expect("runs").cycle.is_some());
    assert_eq!(control.next_due_ms(), u64::MAX);
}

/// Gap 8 (B-1; M). `with_period` starts `last_ms` at the given start, so an
/// earlier time regresses. Kills k05 (`last_ms` 0 in `with_period`).
#[test]
fn with_period_starts_last_ms_at_the_given_start() {
    let (rec, _) = Rec::new();
    let mut host_loop = HostLoop::with_period(rec, 1_000, NonZeroU64::new(25).expect("nonzero"));
    assert_eq!(host_loop.last_ms(), 1_000);
    assert_eq!(host_loop.next_due_ms(), 1_000);
    let error = host_loop.step(999).expect_err("regresses");
    assert_eq!(
        error,
        LoopError::TimeRegression {
            last_ms: 1_000,
            now_ms: 999,
        }
    );
    assert_eq!(host_loop.cycles(), 0);

    // Control: the start slot itself runs.
    let (rec, _) = Rec::new();
    let mut control = HostLoop::with_period(rec, 1_000, NonZeroU64::new(25).expect("nonzero"));
    let step = control.step(1_000).expect("runs");
    assert_eq!(step.cycle.map(|cycle| cycle.now_ms), Some(1_000));
    assert_eq!(control.last_ms(), 1_000);
}

/// Gap 9 (B-8; M). The AI hook sees each cycle's `now_ms`, runs every cycle,
/// and `set_ai(None)` stops it. Kills h01 (the hook gets time 0), h03 (the
/// hook consumed after the first call) and l03 (`set_ai` cannot clear).
#[test]
fn ai_hook_receives_every_cycle_time_and_can_be_cleared() {
    let seen: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&seen);
    let mut hosting = LocalHost::new(tb3());
    hosting.set_ai(Some(Box::new(move |_host: &mut Host, now_ms: u64| {
        probe.lock().expect("recorder").push(now_ms);
        Ok(())
    })));
    let mut host_loop = HostLoop::new(hosting, 0);
    host_loop.step(0).expect("runs");
    host_loop.step(600).expect("runs");
    host_loop.step(650).expect("runs");
    assert_eq!(*seen.lock().expect("recorder"), vec![0, 600, 650]);
    assert_eq!(host_loop.cycles(), 3);

    // Clearing the hook stops the stage: the next cycle records nothing.
    host_loop.hosting_mut().set_ai(None);
    host_loop.step(700).expect("runs");
    assert_eq!(*seen.lock().expect("recorder"), vec![0, 600, 650]);
    assert_eq!(host_loop.cycles(), 4);

    // Control: with no hook at all, cycles still run and record nothing.
    let mut control = HostLoop::new(LocalHost::new(tb3()), 0);
    control.step(0).expect("runs");
    control.step(600).expect("runs");
    assert_eq!(control.cycles(), 2);
}

// ---------------------------------------------------------------------------
// T154a fix round 1, part B (gaps 10–18): one test per gap. Each case has a
// control beside it: the same setup with the one difference removed,
// asserting the other outcome.
// ---------------------------------------------------------------------------

/// Gap 10 (B-5; M). An unbound handle outranks an existing link: after
/// `unbind_peer` a re-attach is `UnknownPeer`, not `AlreadyAttached`.
/// Kills a02 (the epoch check after the link check).
#[test]
fn reattach_after_unbind_is_unknown_peer() {
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let stale = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    host_loop.hosting_mut().host_mut().unbind_peer(p);
    // The stale link is still tabled, so a link-first check would answer
    // `AlreadyAttached`: the epoch check must come first.
    assert!(host_loop.hosting().link_stats(p).is_some());
    assert_eq!(
        host_loop.hosting_mut().attach(p, cap(64)).err(),
        Some(LocalError::UnknownPeer)
    );
    drop(stale);

    // Control: still bound, a second attach is refused as attached.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut hosting = LocalHost::new(host);
    let _client = hosting.attach(p, cap(64)).expect("attaches");
    assert_eq!(
        hosting.attach(p, cap(64)).err(),
        Some(LocalError::AlreadyAttached)
    );
}

/// Gap 11 (retire preamble; M). A retired record keeps the link's final
/// counters: a held link retired after delivery was accepted records
/// `sent > 0` and `held == true`. Kills r11 r12 (a record reset on retire).
#[test]
fn retired_record_keeps_sent_and_held() {
    // Reach a hold exactly like K11, reading P so its delivery is accepted.
    let mut host = tb3();
    let p = bind(&mut host);
    let s = host
        .bind_peer(control(&[]), full_grants(), 0)
        .expect("binds");
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut p_client = host_loop
        .hosting_mut()
        .attach(p, cap(4_096))
        .expect("attaches");
    let _s_client = host_loop.hosting_mut().attach(s, cap(1)).expect("attaches");
    let mut sent = 0u64;
    let mut cycle = 0u64;
    while cycle < 2_000 {
        drain(&mut p_client);
        for _ in 0..2 {
            let actor = 1 + (sent % 3);
            let frame = actor_end_frame(host_loop.hosting().host(), p, sent + 1, actor);
            p_client.send(&frame).expect("sends");
            sent += 1;
        }
        host_loop.step(50 * cycle).expect("runs");
        cycle += 1;
        if host_loop.hosting().link_stats(p).expect("link").held {
            break;
        }
    }
    assert!(host_loop.hosting().link_stats(p).expect("link").held);

    // Control (same setup, client kept): one more cycle retires nothing and
    // the hold and the accepted delivery are still on the link.
    drain(&mut p_client);
    host_loop.step(50 * cycle).expect("runs");
    cycle += 1;
    let before = host_loop.hosting().link_stats(p).expect("link");
    assert!(before.held);
    assert!(before.sent > 0);
    assert_eq!(before.refused, 0);
    assert!(host_loop.hosting_mut().take_retired().is_empty());

    // Dropped between cycles, the link retires with those counters intact:
    // the unbind makes the held frame's retry `Unauthenticated`, so intake
    // retires the link holding its final `sent` and `held`.
    host_loop.hosting_mut().host_mut().unbind_peer(p);
    host_loop.step(50 * cycle).expect("runs");
    assert_eq!(host_loop.hosting().link_stats(p), None);
    assert_eq!(
        host_loop.hosting().host().epoch(p),
        Err(HostError::UnknownPeer)
    );
    let retired = host_loop.hosting_mut().take_retired();
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].0, p);
    assert_eq!(retired[0].1.received, before.received);
    assert_eq!(retired[0].1.refused, before.refused + 1);
    assert_eq!(retired[0].1.sent, before.sent);
    assert!(retired[0].1.sent > 0);
    assert!(retired[0].1.held);
    assert!(host_loop.hosting_mut().take_retired().is_empty());
    assert_eq!(host_loop.hosting().retired_dropped(), 0);
}

/// Gap 12 (B-9; M). Sixty-six retirements drop the two oldest records:
/// the list holds Q2..Q65 in bind order and `retired_dropped == 2`, while
/// at most 8 peers are ever bound at once (K17 rev3). Kills r05 (the cap
/// keeps the newest or loses the count).
#[test]
fn retired_records_drop_oldest_beyond_the_cap() {
    assert_eq!(MAX_RETIRED_RECORDS, 64);
    assert_eq!(MAX_PEERS, 8);
    let mut host_loop = HostLoop::new(LocalHost::new(tb3()), 0);
    let mut handles = Vec::new();
    for round in 0..=8u64 {
        let count = if round <= 7 { 8 } else { 2 };
        let now = 50 * round;
        for _ in 0..count {
            let peer = host_loop
                .hosting_mut()
                .host_mut()
                .bind_peer(control(&[]), full_grants(), now)
                .expect("binds");
            let client = host_loop
                .hosting_mut()
                .attach(peer, cap(1))
                .expect("attaches");
            drop(client);
            handles.push(peer);
        }
        let step = host_loop.step(now).expect("runs");
        assert!(step.cycle.is_some());
        assert_eq!(step.missed, 0);
        for peer in handles.iter().skip(handles.len() - count as usize).copied() {
            assert_eq!(host_loop.hosting().link_stats(peer), None);
            assert_eq!(
                host_loop.hosting().host().epoch(peer),
                Err(HostError::UnknownPeer)
            );
        }
        if round <= 7 {
            assert_eq!(host_loop.hosting().retired_dropped(), 0);
        }
    }
    assert_eq!(handles.len(), 66);
    assert!(handles.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(host_loop.hosting().retired_dropped(), 2);
    let retired = host_loop.hosting_mut().take_retired();
    assert_eq!(retired.len(), 64);
    for (index, (peer, stats)) in retired.iter().enumerate() {
        assert_eq!(*peer, handles[index + 2]);
        assert_eq!(stats.received, 0);
        assert_eq!(stats.refused, 0);
    }
    assert_eq!(retired.first().expect("first").0, handles[2]);
    assert_eq!(retired.last().expect("last").0, handles[65]);
    assert!(host_loop.hosting_mut().take_retired().is_empty());
    assert_eq!(host_loop.hosting().retired_dropped(), 2);

    // Control: eight rounds fill exactly 64 records and drop none.
    let mut host_loop = HostLoop::new(LocalHost::new(tb3()), 0);
    let mut handles = Vec::new();
    for round in 0..=7u64 {
        let now = 50 * round;
        for _ in 0..8 {
            let peer = host_loop
                .hosting_mut()
                .host_mut()
                .bind_peer(control(&[]), full_grants(), now)
                .expect("binds");
            let client = host_loop
                .hosting_mut()
                .attach(peer, cap(1))
                .expect("attaches");
            drop(client);
            handles.push(peer);
        }
        host_loop.step(now).expect("runs");
    }
    let retired = host_loop.hosting_mut().take_retired();
    assert_eq!(retired.len(), 64);
    assert_eq!(retired.first().expect("first").0, handles[0]);
    assert_eq!(retired.last().expect("last").0, handles[63]);
    assert_eq!(host_loop.hosting().retired_dropped(), 0);
}

/// Gap 13 (B-6 Route; M). A `0x81` frame reaches `ingest_transfer`:
/// `Published`, `Stale` and `Resynced` continue with `refused == 0`, an
/// unbound peer's transfer frame is counted and retires the link, and the
/// same holds past the first cycle. Kills v01 v03 v04 v07 v08 v11 v22.
#[test]
fn transfer_frames_route_by_first_byte_through_ingest_transfer() {
    fn narrowed(host: &mut Host) -> (PeerHandle, [u8; 16]) {
        let w = host
            .bind_snapshot_peer(control(&[a(), b(), c()]), full_grants(), 0)
            .expect("binds");
        let mut grants = full_grants();
        grants.abilities = vec![strike()];
        assert_eq!(host.set_grants(w, grants), Ok(GrantUpdate::SnapshotStaged));
        let epoch = host.epoch(w).expect("bound");
        (w, epoch)
    }

    // (a) `Published` at step 50, not only the first cycle: in flight ends,
    // nothing refused, nothing held.
    let mut host = tb3();
    let (w, epoch) = narrowed(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 50);
    let mut client = host_loop
        .hosting_mut()
        .attach(w, cap(64))
        .expect("attaches");
    client.send(&published(epoch, 1)).expect("sends");
    let step = host_loop.step(50).expect("runs");
    assert_eq!(step.missed, 0);
    assert_eq!(
        host_loop.hosting().host().transfer_state(w),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: None,
        }))
    );
    let stats = host_loop.hosting().link_stats(w).expect("link");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    assert!(!stats.held);

    // (b) A repeat publish is `Stale`: still continued, still uncounted.
    let mut host = tb3();
    let (w, epoch) = narrowed(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(w, cap(64))
        .expect("attaches");
    client.send(&published(epoch, 1)).expect("sends");
    client.send(&published(epoch, 1)).expect("sends");
    host_loop.step(0).expect("runs");
    assert_eq!(
        host_loop.hosting().host().transfer_state(w),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: None,
        }))
    );
    let stats = host_loop.hosting().link_stats(w).expect("link");
    assert_eq!(stats.received, 2);
    assert_eq!(stats.refused, 0);
    assert!(!stats.held);

    // (c) A resync of the in-flight transfer is `Resynced`: continued,
    // uncounted, and the next transfer is staged.
    let mut host = tb3();
    let (w, epoch) = narrowed(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(w, cap(64))
        .expect("attaches");
    client
        .send(&resync(epoch, Some(1), ResyncReason::Expired))
        .expect("sends");
    host_loop.step(0).expect("runs");
    assert_eq!(
        host_loop.hosting().host().transfer_state(w),
        Ok(Some(TransferState {
            next_transfer: 3,
            in_flight: Some(2),
        }))
    );
    let stats = host_loop.hosting().link_stats(w).expect("link");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    assert!(!stats.held);

    // (d) An unbound peer's transfer frame is counted and retires the link.
    let mut host = tb3();
    let (w, epoch) = narrowed(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(w, cap(64))
        .expect("attaches");
    host_loop.step(0).expect("runs");
    assert!(host_loop.hosting_mut().take_retired().is_empty());
    host_loop.hosting_mut().host_mut().unbind_peer(w);
    client.send(&published(epoch, 1)).expect("sends");
    host_loop.step(50).expect("runs");
    assert_eq!(host_loop.hosting().link_stats(w), None);
    assert_eq!(
        host_loop.hosting().host().epoch(w),
        Err(HostError::UnknownPeer)
    );
    let retired = host_loop.hosting_mut().take_retired();
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].0, w);
    assert_eq!(retired[0].1.received, 1);
    assert_eq!(retired[0].1.refused, 1);

    // Control: an unknown transfer is counted and dropped, the link stays
    // unheld, and a second step changes nothing.
    let mut host = tb3();
    let (w, epoch) = narrowed(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(w, cap(64))
        .expect("attaches");
    client.send(&published(epoch, 9)).expect("sends");
    host_loop.step(0).expect("runs");
    let stats = host_loop.hosting().link_stats(w).expect("link stays");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 1);
    assert!(!stats.held);
    assert_eq!(
        host_loop.hosting().host().transfer_state(w),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: Some(1),
        }))
    );
    host_loop.step(50).expect("runs");
    let stats = host_loop.hosting().link_stats(w).expect("link stays");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 1);
    assert!(!stats.held);
    assert!(host_loop.hosting_mut().take_retired().is_empty());
    drop(client);
}

/// Gap 14 (B-6; M). Two frames queued behind an unbind count only the
/// first: the first retires the link and the drain stops. Kills v15 (the
/// drain continues past the retirement).
#[test]
fn two_frames_after_unbind_count_only_the_first() {
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    host_loop.step(0).expect("runs");
    assert!(host_loop.hosting_mut().take_retired().is_empty());
    let first = end_frame(host_loop.hosting().host(), p, 1);
    let second = end_frame(host_loop.hosting().host(), p, 2);
    let queued = drain(&mut client);
    host_loop.hosting_mut().host_mut().unbind_peer(p);
    client.send(&first).expect("sends");
    client.send(&second).expect("sends");
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
    // The frames sent before the unbind are still readable, in order.
    assert!(!queued.is_empty());
    assert_eq!(client.recv(), None);
    assert!(client.is_closed());
    assert_eq!(client.send(&first), Err(LinkError::Closed));

    // Control: still bound, both frames are taken and none refused.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut control = HostLoop::new(LocalHost::new(host), 0);
    let mut quiet = control.hosting_mut().attach(p, cap(64)).expect("attaches");
    let first = end_frame(control.hosting().host(), p, 1);
    let second = end_frame(control.hosting().host(), p, 2);
    quiet.send(&first).expect("sends");
    quiet.send(&second).expect("sends");
    control.step(0).expect("runs");
    let stats = control.hosting().link_stats(p).expect("link stays");
    assert_eq!(stats.received, 2);
    assert_eq!(stats.refused, 0);
    assert!(!stats.held);
}

/// Gap 15 (B-6; L). An empty frame is not a transfer frame: it goes to
/// `host.ingest`, which stages it, so the link continues with nothing
/// refused and nothing held. Kills v20 (empty dropped or misrouted).
#[test]
fn empty_frame_routes_to_ingest_and_continues() {
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    client.send(&[]).expect("sends");
    host_loop.step(0).expect("runs");
    let stats = host_loop.hosting().link_stats(p).expect("link stays");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    assert!(!stats.held);

    // The frame reached the host's lane-0 path: the loop's delivery equals
    // a twin that staged the same empty bytes by hand.
    let mut twin = tb3();
    assert_eq!(bind(&mut twin), p);
    assert_eq!(twin.ingest(p, &[], 0), Ok(IngestDisposition::Staged));
    twin.pump(0).expect("pumps");
    twin.advance(0).expect("advances");
    let entries = twin.take_delivery_entries(p).expect("bound");
    if let Some(last) = entries.last() {
        twin.acknowledge_delivery(p, last.ordinal).expect("acks");
    }
    twin.acknowledge_captures(twin.last_capture())
        .expect("acks captures");
    assert_eq!(
        host_loop.hosting().host().authority_hash(),
        twin.authority_hash()
    );
    assert_eq!(
        drain(&mut client),
        entries
            .into_iter()
            .map(|entry| entry.bytes)
            .collect::<Vec<_>>()
    );
    assert!(host_loop.hosting_mut().take_retired().is_empty());

    // Control: a one-byte `0x81` frame on a lane-0 peer routes to
    // `ingest_transfer`, which refuses it as `NotSnapshotPeer`.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut control = HostLoop::new(LocalHost::new(host), 0);
    let mut quiet = control.hosting_mut().attach(p, cap(64)).expect("attaches");
    quiet.send(&[0x81]).expect("sends");
    control.step(0).expect("runs");
    let stats = control.hosting().link_stats(p).expect("link stays");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 1);
    assert!(!stats.held);
}

/// Gap 16 (B-3; L). A host ahead in time fails the cycle at intake: the
/// step is `Hosting(TimeRegression)`, the AI hook never runs, and the
/// drain stops after the first frame. Kills v23 v24 i25.
#[test]
fn host_time_regression_stops_the_cycle_mid_drain() {
    let mut host = tb3();
    let p = bind(&mut host);
    // The host's last accepted time moves to 100; the loop starts at 0.
    host.pump(100).expect("pumps");
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    let ran: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    let probe = Arc::clone(&ran);
    host_loop
        .hosting_mut()
        .set_ai(Some(Box::new(move |_host: &mut Host, _now_ms: u64| {
            *probe.lock().expect("recorder") = true;
            Ok(())
        })));
    let first = end_frame(host_loop.hosting().host(), p, 1);
    let second = end_frame(host_loop.hosting().host(), p, 2);
    client.send(&first).expect("sends");
    client.send(&second).expect("sends");

    let error = host_loop.step(0).expect_err("regresses");
    assert_eq!(error, LoopError::Hosting(HostError::TimeRegression));
    assert_eq!(
        error.to_string(),
        "Hosting at run/step: TimeRegression at host/admission"
    );
    // The hook never ran and only the first frame left the queue.
    assert!(!*ran.lock().expect("recorder"));
    let stats = host_loop.hosting().link_stats(p).expect("link stays");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    assert!(!stats.held);
    assert_eq!(host_loop.cycles(), 1);

    // Control: the same loop stepped at the host's time runs the hook and
    // takes the second frame.
    let step = host_loop.step(100).expect("runs");
    assert!(step.cycle.is_some());
    assert!(*ran.lock().expect("recorder"));
    let stats = host_loop.hosting().link_stats(p).expect("link stays");
    assert_eq!(stats.received, 2);
    assert_eq!(stats.refused, 0);
    assert_eq!(host_loop.cycles(), 2);
}

/// Gap 17 (B-10; L). The first `send` after the server end went away
/// reports `Closed` and sets the flag: no `recv` needs to come first.
/// Kills n04 n05 (the flag set only on the receive path).
#[test]
fn send_first_after_drop_reports_closed() {
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop.hosting_mut().attach(p, cap(2)).expect("attaches");
    let first = end_frame(host_loop.hosting().host(), p, 1);
    let second = actor_end_frame(host_loop.hosting().host(), p, 2, 2);
    assert_eq!(client.send(&first), Ok(()));
    assert_eq!(client.send(&second), Ok(()));
    assert_eq!(client.send(&first), Err(LinkError::Full));
    assert!(!client.is_closed());

    host_loop.step(0).expect("runs");
    drop(host_loop);
    // No `recv` first: the send itself finds the queue gone.
    assert_eq!(client.send(b"later"), Err(LinkError::Closed));
    assert!(client.is_closed());
    assert!(client.recv().is_some());
    assert!(client.recv().is_some());
    assert_eq!(client.recv(), None);
    assert!(client.is_closed());
    assert_eq!(client.send(b"later"), Err(LinkError::Closed));
    assert_eq!(LinkError::Closed.to_string(), "Closed at run/link");

    // Control: before the drop the client is open and the queue delivers.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut control = HostLoop::new(LocalHost::new(host), 0);
    let mut quiet = control.hosting_mut().attach(p, cap(2)).expect("attaches");
    let first = end_frame(control.hosting().host(), p, 1);
    quiet.send(&first).expect("sends");
    assert!(!quiet.is_closed());
    control.step(0).expect("runs");
    assert!(!quiet.is_closed());
    assert!(quiet.recv().is_some());
}

/// Gap 18 (B-7(3)/(5); L). The AI hook runs between intake and deliver: a
/// client it drops retires at deliver, with its staged command applied.
/// Kills d10 d11 (no retire on a deliver-time disconnect).
#[test]
fn ai_hook_dropped_client_retires_at_deliver() {
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    let frame = end_frame(host_loop.hosting().host(), p, 1);
    client.send(&frame).expect("sends");
    let slot: Arc<Mutex<Option<LocalClient>>> = Arc::new(Mutex::new(None));
    slot.lock().expect("slot").replace(client);
    let dropped = Arc::clone(&slot);
    let ran: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    let probe = Arc::clone(&ran);
    host_loop
        .hosting_mut()
        .set_ai(Some(Box::new(move |_host: &mut Host, _now_ms: u64| {
            *probe.lock().expect("recorder") = true;
            drop(dropped.lock().expect("slot").take());
            Ok(())
        })));
    host_loop.step(0).expect("runs");
    assert!(*ran.lock().expect("recorder"));

    // Intake took the frame, deliver found the queue gone: one record.
    assert_eq!(host_loop.hosting().link_stats(p), None);
    assert_eq!(
        host_loop.hosting().host().epoch(p),
        Err(HostError::UnknownPeer)
    );
    let retired = host_loop.hosting_mut().take_retired();
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].0, p);
    assert_eq!(retired[0].1.received, 1);
    assert_eq!(retired[0].1.refused, 0);
    assert!(!retired[0].1.held);
    assert_eq!(retired[0].1.sent, 0);
    // The staged command was still applied before the drop.
    assert_eq!(host_loop.hosting().host().server_tick(), 1);
    assert_eq!(host_loop.hosting().host().awaiting_orders(), vec![b()]);

    // Control: a hook that keeps the client retires nothing.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut control = HostLoop::new(LocalHost::new(host), 0);
    let mut quiet = control.hosting_mut().attach(p, cap(64)).expect("attaches");
    let frame = end_frame(control.hosting().host(), p, 1);
    quiet.send(&frame).expect("sends");
    control
        .hosting_mut()
        .set_ai(Some(Box::new(|_host: &mut Host, _now_ms: u64| Ok(()))));
    control.step(0).expect("runs");
    assert!(control.hosting().link_stats(p).is_some());
    assert!(control.hosting_mut().take_retired().is_empty());
    assert_eq!(control.hosting().host().server_tick(), 1);
}

// ---------------------------------------------------------------------------
// T154a fix round 2 (re-plant survivors s11, v01, v07, v20, v22, v24, i13,
// i25): one test per mutant. Each case has a control beside it: the same
// setup with the one difference removed, asserting the other outcome. d01 is
// equivalent (see the note at the end) and gets no test.
// ---------------------------------------------------------------------------

/// Fix round 2, s11 (B-1). A jump whose `(skipped + 1) * period_ms`
/// overflows `u64` saturates `next_due_ms` at `u64::MAX`. Kills s11
/// (wrapping arithmetic: the wrapped product lands at `u64::MAX - 1`).
#[test]
fn schedule_saturates_on_overflow() {
    let (rec, _) = Rec::new();
    let mut host_loop = HostLoop::with_period(rec, 0, NonZeroU64::new(u64::MAX).expect("nonzero"));
    let step = host_loop.step(u64::MAX).expect("runs");
    assert_eq!(step.missed, 1);
    assert_eq!(step.cycle.map(|cycle| cycle.now_ms), Some(u64::MAX));
    assert_eq!(host_loop.next_due_ms(), u64::MAX);
    assert_eq!(host_loop.missed(), 1);
    assert_eq!(host_loop.cycles(), 1);

    // Control: the start slot itself runs and already saturates the next due
    // with no overflow involved.
    let (rec, _) = Rec::new();
    let mut control = HostLoop::with_period(rec, 0, NonZeroU64::new(u64::MAX).expect("nonzero"));
    let step = control.step(0).expect("runs");
    assert_eq!(step.missed, 0);
    assert_eq!(step.cycle.map(|cycle| cycle.now_ms), Some(0));
    assert_eq!(control.next_due_ms(), u64::MAX);
    assert_eq!(control.missed(), 0);
}

/// Fix round 2, v01 (B-6 Route). A `0x82` frame reaches `ingest_transfer`:
/// on a v3 session a v3 `Published` ends the in-flight transfer. Kills v01
/// (the `TRANSFER_VERSION_V3` arm removed, so the frame falls into lane-0
/// `ingest` and the transfer stays in flight).
#[test]
fn v3_transfer_frames_route_to_ingest_transfer() {
    fn facts() -> FactGrants {
        FactGrants {
            entities: vec![
                EntityFactGrant {
                    entity: a(),
                    pools: true,
                    effects: true,
                    stats: true,
                    tags: true,
                },
                EntityFactGrant {
                    entity: b(),
                    pools: true,
                    effects: true,
                    stats: true,
                    tags: true,
                },
                EntityFactGrant {
                    entity: c(),
                    pools: true,
                    effects: true,
                    stats: true,
                    tags: true,
                },
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
    fn published_v3(epoch: [u8; 16], transfer: u64) -> Vec<u8> {
        transfer_v3::encode_up(&TransferUp::Published {
            epoch,
            transfer: TransferId::new(transfer).expect("nonzero"),
        })
        .expect("encodes")
    }

    let mut host = tb3();
    let w = host
        .bind_v3_peer(control(&[a(), b(), c()]), full_grants(), facts(), r1(), 0)
        .expect("binds");
    let staged = host
        .transfer_state(w)
        .expect("bound")
        .expect("a v3 bind stages a snapshot");
    let in_flight = staged.in_flight.expect("staged at bind");
    let epoch = host.epoch(w).expect("bound");
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(w, cap(64))
        .expect("attaches");
    client.send(&published_v3(epoch, in_flight)).expect("sends");
    host_loop.step(0).expect("runs");
    assert_eq!(
        host_loop
            .hosting()
            .host()
            .transfer_state(w)
            .expect("bound")
            .expect("snapshot")
            .in_flight,
        None
    );
    let stats = host_loop.hosting().link_stats(w).expect("link stays");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    assert!(!stats.held);
    assert!(host_loop.hosting_mut().take_retired().is_empty());

    // Control: no frame leaves the transfer in flight, unheld and uncounted.
    let mut host = tb3();
    let w = host
        .bind_v3_peer(control(&[a(), b(), c()]), full_grants(), facts(), r1(), 0)
        .expect("binds");
    let staged = host
        .transfer_state(w)
        .expect("bound")
        .expect("a v3 bind stages a snapshot");
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let _client = host_loop
        .hosting_mut()
        .attach(w, cap(64))
        .expect("attaches");
    host_loop.step(0).expect("runs");
    assert_eq!(
        host_loop.hosting().host().transfer_state(w),
        Ok(Some(staged))
    );
    let stats = host_loop.hosting().link_stats(w).expect("link stays");
    assert_eq!(stats.received, 0);
    assert_eq!(stats.refused, 0);
    assert!(!stats.held);
}

/// Fix round 2, v07 (B-6 Route, transfer `Unbound` arm). An unbound peer's
/// transfer frame retires the link at once: the link is gone (one record)
/// after that step, and a second frame queued behind it is not taken.
/// Kills v07 (`Continue` instead of `Retire`: the link stays and the second
/// frame is taken).
#[test]
fn unbound_transfer_frame_retires_before_the_second_frame() {
    fn narrowed(host: &mut Host) -> (PeerHandle, [u8; 16]) {
        let w = host
            .bind_snapshot_peer(control(&[a(), b(), c()]), full_grants(), 0)
            .expect("binds");
        let mut grants = full_grants();
        grants.abilities = vec![strike()];
        assert_eq!(host.set_grants(w, grants), Ok(GrantUpdate::SnapshotStaged));
        let epoch = host.epoch(w).expect("bound");
        (w, epoch)
    }

    let mut host = tb3();
    let (w, epoch) = narrowed(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(w, cap(64))
        .expect("attaches");
    host_loop.step(0).expect("runs");
    assert!(host_loop.hosting_mut().take_retired().is_empty());
    host_loop.hosting_mut().host_mut().unbind_peer(w);
    client.send(&published(epoch, 1)).expect("sends");
    client.send(&published(epoch, 1)).expect("sends");
    host_loop.step(50).expect("runs");
    assert_eq!(host_loop.hosting().link_stats(w), None);
    assert_eq!(
        host_loop.hosting().host().epoch(w),
        Err(HostError::UnknownPeer)
    );
    let retired = host_loop.hosting_mut().take_retired();
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].0, w);
    assert_eq!(retired[0].1.received, 1);
    assert_eq!(retired[0].1.refused, 1);
    assert!(host_loop.hosting_mut().take_retired().is_empty());

    // Control: still bound, both frames are taken and the link stays.
    let mut host = tb3();
    let (w, epoch) = narrowed(&mut host);
    let mut control = HostLoop::new(LocalHost::new(host), 0);
    let mut quiet = control.hosting_mut().attach(w, cap(64)).expect("attaches");
    quiet.send(&published(epoch, 1)).expect("sends");
    quiet.send(&published(epoch, 1)).expect("sends");
    control.step(0).expect("runs");
    let stats = control.hosting().link_stats(w).expect("link stays");
    assert_eq!(stats.received, 2);
    assert_eq!(stats.refused, 0);
    assert!(!stats.held);
    assert!(control.hosting_mut().take_retired().is_empty());
}

/// Fix round 2, v20 (B-6 Route). An empty frame goes to `host.ingest`: on an
/// unbound peer that is `Unauthenticated`, counted and retired. Kills v20
/// (`None => Continue`: the frame is dropped, the link stays, nothing is
/// refused).
#[test]
fn empty_frame_on_unbound_peer_routes_to_ingest() {
    let mut host = tb3();
    let p = bind(&mut host);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    host_loop.step(0).expect("runs");
    assert!(host_loop.hosting_mut().take_retired().is_empty());
    let queued = drain(&mut client);
    host_loop.hosting_mut().host_mut().unbind_peer(p);
    client.send(&[]).expect("sends");
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
    // The frames sent before the unbind are still readable, in order.
    assert!(!queued.is_empty());
    assert_eq!(client.recv(), None);
    assert!(client.is_closed());

    // Control: bound, the empty frame is staged and the link stays unheld.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut control = HostLoop::new(LocalHost::new(host), 0);
    let mut quiet = control.hosting_mut().attach(p, cap(64)).expect("attaches");
    quiet.send(&[]).expect("sends");
    control.step(0).expect("runs");
    let stats = control.hosting().link_stats(p).expect("link stays");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    assert!(!stats.held);
}

/// Fix round 2, v22 (B-6 Route). A transfer frame reaches the host with the
/// step's `now_ms`: after the host's time moved to 50, a transfer at 100 is
/// accepted; passing 0 instead is a time regression. Kills v22 (the frame
/// passed with 0: the step fails with `Hosting(TimeRegression)`).
#[test]
fn transfer_frame_carries_the_step_time() {
    let mut host = tb3();
    let w = host
        .bind_snapshot_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let mut narrowed = full_grants();
    narrowed.abilities = vec![strike()];
    assert_eq!(
        host.set_grants(w, narrowed),
        Ok(GrantUpdate::SnapshotStaged)
    );
    let epoch = host.epoch(w).expect("bound");
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(w, cap(64))
        .expect("attaches");
    host_loop.step(50).expect("runs");
    assert!(host_loop.hosting_mut().take_retired().is_empty());
    client.send(&published(epoch, 1)).expect("sends");
    let step = host_loop.step(100).expect("runs");
    assert_eq!(step.missed, 0);
    assert_eq!(
        host_loop.hosting().host().transfer_state(w),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: None,
        }))
    );
    let stats = host_loop.hosting().link_stats(w).expect("link stays");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    assert!(!stats.held);

    // Control: a lane-0 frame at the same step is accepted and ticks.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut control = HostLoop::new(LocalHost::new(host), 0);
    let mut quiet = control.hosting_mut().attach(p, cap(64)).expect("attaches");
    control.step(50).expect("runs");
    let frame = end_frame(control.hosting().host(), p, 1);
    quiet.send(&frame).expect("sends");
    let step = control.step(100).expect("runs");
    assert_eq!(step.cycle.map(|cycle| cycle.now_ms), Some(100));
    assert_eq!(control.hosting().host().awaiting_orders(), vec![b()]);
}

/// Fix round 2, v24 (B-6 Route). An `Err` from `ingest_transfer` surfaces
/// from `step` as `Hosting`: with the host's time ahead at 100, a transfer
/// frame at 0 is a `TimeRegression`. Kills v24 (`unwrap_or(Published)`
/// swallows the error and the step runs).
#[test]
fn transfer_time_regression_surfaces_as_hosting_error() {
    let mut host = tb3();
    let w = host
        .bind_snapshot_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let mut narrowed = full_grants();
    narrowed.abilities = vec![strike()];
    assert_eq!(
        host.set_grants(w, narrowed),
        Ok(GrantUpdate::SnapshotStaged)
    );
    let epoch = host.epoch(w).expect("bound");
    // The host's last accepted time moves to 100; the loop starts at 0.
    host.pump(100).expect("pumps");
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let mut client = host_loop
        .hosting_mut()
        .attach(w, cap(64))
        .expect("attaches");
    client.send(&published(epoch, 1)).expect("sends");
    client.send(&published(epoch, 1)).expect("sends");

    let error = host_loop.step(0).expect_err("regresses");
    assert_eq!(error, LoopError::Hosting(HostError::TimeRegression));
    // The drain stops at the first frame; the link stays.
    let stats = host_loop.hosting().link_stats(w).expect("link stays");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    assert!(!stats.held);
    assert_eq!(host_loop.cycles(), 1);
    assert_eq!(host_loop.next_due_ms(), 50);

    // Control: the same loop stepped at the host's time publishes and runs.
    client.send(&published(epoch, 1)).expect("sends");
    let step = host_loop.step(100).expect("runs");
    assert!(step.cycle.is_some());
    assert_eq!(
        host_loop.hosting().host().transfer_state(w),
        Ok(Some(TransferState {
            next_transfer: 2,
            in_flight: None,
        }))
    );
    let stats = host_loop.hosting().link_stats(w).expect("link stays");
    assert_eq!(stats.received, 3);
    assert_eq!(stats.refused, 0);
}

/// Fix round 2, i13 (B-6(2)). A fresh `Hold` stops that peer's drain: with
/// the ingress cap full, the first of two queued frames is taken and held
/// and the second is not taken that cycle. Kills i13 (the `break` after a
/// fresh hold removed: the drain continues and takes the second frame).
#[test]
fn fresh_hold_stops_the_peer_drain() {
    // Fills P's ingress without pumping: one frame per 50 ms stays within
    // the 40/s sustained rate, so all 128 stage and the 129th is QueueFull.
    fn prestaged(count: usize, spacing: u64) -> (Host, PeerHandle) {
        let mut host = tb3();
        let p = bind(&mut host);
        for index in 1..=count as u64 {
            let actor = 1 + ((index - 1) % 3);
            let frame = actor_end_frame(&host, p, index, actor);
            assert_eq!(
                host.ingest(p, &frame, spacing * (index - 1)),
                Ok(IngestDisposition::Staged)
            );
        }
        (host, p)
    }

    let (host, p) = prestaged(128, 50);
    let mut host_loop = HostLoop::new(LocalHost::new(host), 6_350);
    let mut client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    let first = actor_end_frame(host_loop.hosting().host(), p, 129, 1);
    let second = actor_end_frame(host_loop.hosting().host(), p, 130, 2);
    client.send(&first).expect("sends");
    client.send(&second).expect("sends");
    host_loop.step(6_350).expect("runs");
    let stats = host_loop.hosting().link_stats(p).expect("link stays");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    assert!(stats.held);
    assert!(host_loop.hosting_mut().take_retired().is_empty());

    // Control: one slot below the cap, both frames stage and nothing holds.
    let (host, p) = prestaged(126, 50);
    let mut control = HostLoop::new(LocalHost::new(host), 6_250);
    let mut quiet = control.hosting_mut().attach(p, cap(64)).expect("attaches");
    let first = actor_end_frame(control.hosting().host(), p, 127, 1);
    let second = actor_end_frame(control.hosting().host(), p, 128, 2);
    quiet.send(&first).expect("sends");
    quiet.send(&second).expect("sends");
    control.step(6_250).expect("runs");
    let stats = control.hosting().link_stats(p).expect("link stays");
    assert_eq!(stats.received, 2);
    assert_eq!(stats.refused, 0);
    assert!(!stats.held);
}

/// Fix round 2, i25 (B-6(3)). An `Err` from the cycle's `pump` surfaces from
/// `step` even with no frame queued: with the host's time ahead at 100, the
/// empty intake's `pump(0)` is a `TimeRegression`, before the AI hook runs.
/// Kills i25 (`let _ = pump`: the error is dropped and the hook runs).
#[test]
fn pump_error_with_no_frames_stops_the_cycle() {
    let mut host = tb3();
    let p = bind(&mut host);
    // The host's last accepted time moves to 100; the loop starts at 0.
    host.pump(100).expect("pumps");
    let mut host_loop = HostLoop::new(LocalHost::new(host), 0);
    let _client = host_loop
        .hosting_mut()
        .attach(p, cap(64))
        .expect("attaches");
    let ran: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    let probe = Arc::clone(&ran);
    host_loop
        .hosting_mut()
        .set_ai(Some(Box::new(move |_host: &mut Host, _now_ms: u64| {
            *probe.lock().expect("recorder") = true;
            Ok(())
        })));

    let error = host_loop.step(0).expect_err("regresses");
    assert_eq!(error, LoopError::Hosting(HostError::TimeRegression));
    // The AI stage never ran, and the link is untouched.
    assert!(!*ran.lock().expect("recorder"));
    let stats = host_loop.hosting().link_stats(p).expect("link stays");
    assert_eq!(stats.received, 0);
    assert_eq!(stats.refused, 0);
    assert!(!stats.held);
    assert_eq!(host_loop.cycles(), 1);
    assert_eq!(host_loop.next_due_ms(), 50);

    // Control: without the host running ahead, the step runs the hook.
    let mut host = tb3();
    let p = bind(&mut host);
    let mut control = HostLoop::new(LocalHost::new(host), 0);
    let _client = control.hosting_mut().attach(p, cap(64)).expect("attaches");
    let ran: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    let probe = Arc::clone(&ran);
    control
        .hosting_mut()
        .set_ai(Some(Box::new(move |_host: &mut Host, _now_ms: u64| {
            *probe.lock().expect("recorder") = true;
            Ok(())
        })));
    assert!(control.step(0).expect("runs").cycle.is_some());
    assert!(*ran.lock().expect("recorder"));
}

// Fix round 2, d01 (B-7(1)): equivalent, no test. A link departed at the
// start of `deliver` retires at arm (5) of that same step however arm (1)
// reads: offering its entries hits a dropped down queue (`Disconnected`,
// still departed, nothing acknowledged) or finds no entries, and both paths
// reach the departed check before the next peer. No public-API state can
// tell arm (1) apart.
