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
use crpg_net::protocol::{IntentBody, IntentFrame, NetId, LANE_COMBAT};
use crpg_net::protocol_transfer::{TransferId, TransferUp};
use crpg_net::protocol_v3::PauseReason;
use crpg_net::transport::Transport;
use crpg_server::host::{
    AdvanceReport, ControlGrant, DisclosureGrants, EntityDisclosure, GrantUpdate, Host, HostConfig,
    HostError, IngestDisposition, PeerHandle, ProtocolSelection, TransferState, MAX_PEERS,
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
