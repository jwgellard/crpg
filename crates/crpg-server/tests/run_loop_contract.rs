//! T154a two-keys contract tests (`tasks/T154.md`, D66d approved rev3).
//!
//! Independent of the implementer's `tests/run_loop.rs` and of `src/run.rs`
//! bodies: every expectation below comes from the contract's Behaviour
//! (§"T154a — the loop", §"T154a — `LocalHost`"), Errors table and the K1–K17
//! conventions (literal 20 Hz grid, skip-not-burst, exact records).
//! Helpers (`eid`, `control`, `full_grants`, `config`, `history_of`, `end`)
//! are copied from `tests/host_pause.rs`.

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
    HostError, PeerHandle, ProtocolSelection, TransferState, MAX_PEERS,
};
use crpg_server::run::{
    AiHook, HostLoop, Hosting, LinkError, LinkStats, LocalClient, LocalError, LocalHost, LoopError,
    MAX_RETIRED_RECORDS, TICK_PERIOD_MS,
};
use crpg_sim::{CombatAction, HistoryWorld};
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");

// ---------------------------------------------------------------------------
// Fixtures (copied helpers).
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
fn history_of(value: &Value) -> HistoryWorld {
    let mut history: HistoryWorld = serde_json::from_value(value.clone()).expect("fixture loads");
    history
        .acknowledge(history.last_sequence())
        .expect("acks its own range");
    history
}
fn tb3_host() -> Host {
    Host::from_history(history_of(&tb3_value()), config(), 0).expect("wraps")
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
fn cap(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("nonzero capacity")
}
fn end_frame(host: &Host, peer: PeerHandle, seq: u64) -> Vec<u8> {
    let frame = IntentFrame {
        epoch: host.epoch(peer).expect("bound"),
        lane: LANE_COMBAT,
        seq,
        observed_tick: host.server_tick(),
        actor: n(1),
        body: IntentBody::EndTurn,
    };
    codec_v2::encode_intent(&frame).expect("encodes")
}
fn published(epoch: [u8; 16], transfer: u64) -> Vec<u8> {
    codec_transfer::encode_up(&TransferUp::Published {
        epoch,
        transfer: TransferId::new(transfer).expect("nonzero"),
    })
    .expect("encodes")
}
fn drain_down(client: &mut LocalClient) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(frame) = client.recv() {
        out.push(frame);
    }
    out
}
fn assert_send<T: Send>() {}

type Seen = Arc<Mutex<Vec<(u64, u64, Vec<EntityId>)>>>;

// ---------------------------------------------------------------------------
// Test-local `Hosting` (contract notation `Rec`).
// ---------------------------------------------------------------------------

struct Rec {
    log: Rc<RefCell<Vec<String>>>,
    fail_at: Option<usize>,
    failed: bool,
}
impl Rec {
    fn ok(log: Rc<RefCell<Vec<String>>>) -> Self {
        Self {
            log,
            fail_at: None,
            failed: false,
        }
    }
    fn failing_at(log: Rc<RefCell<Vec<String>>>, stage: usize) -> Self {
        Self {
            log,
            fail_at: Some(stage),
            failed: false,
        }
    }
    fn fail(&mut self, stage: usize, now: u64, label: &str) -> Result<(), HostError> {
        self.log.borrow_mut().push(format!("{label}@{now}"));
        if self.fail_at == Some(stage) && !self.failed {
            self.failed = true;
            return Err(HostError::Closed);
        }
        Ok(())
    }
}
impl Hosting for Rec {
    type Error = HostError;
    fn intake(&mut self, now_ms: u64) -> Result<(), HostError> {
        self.fail(0, now_ms, "intake")
    }
    fn drive_ai(&mut self, now_ms: u64) -> Result<(), HostError> {
        self.fail(1, now_ms, "ai")
    }
    fn advance(&mut self, now_ms: u64) -> Result<AdvanceReport, HostError> {
        self.fail(2, now_ms, "advance")?;
        Ok(AdvanceReport {
            ticked: true,
            timer_ended_turn: None,
            paused: None,
        })
    }
    fn deliver(&mut self, now_ms: u64) -> Result<(), HostError> {
        self.fail(3, now_ms, "deliver")
    }
    fn retire_captures(&mut self) -> Result<(), HostError> {
        self.log.borrow_mut().push("retire".to_owned());
        if self.fail_at == Some(4) && !self.failed {
            self.failed = true;
            return Err(HostError::Closed);
        }
        Ok(())
    }
}
fn ok_report() -> AdvanceReport {
    AdvanceReport {
        ticked: true,
        timer_ended_turn: None,
        paused: None,
    }
}

// ---------------------------------------------------------------------------
// B-1 cadence.
// ---------------------------------------------------------------------------

#[test]
fn cadence_grid_and_period() {
    assert_eq!(TICK_PERIOD_MS, 50);
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut loop_ = HostLoop::new(Rec::ok(Rc::clone(&log)), 1_000);
    assert_eq!(loop_.next_due_ms(), 1_000);
    assert_eq!(loop_.last_ms(), 1_000);
    assert_eq!((loop_.cycles(), loop_.missed()), (0, 0));
    let r = loop_.step(1_000).expect("on the grid runs");
    assert_eq!(
        (r.cycle.as_ref().expect("cycle").now_ms, r.missed),
        (1_000, 0)
    );
    assert_eq!(loop_.next_due_ms(), 1_050);
    let r = loop_.step(1_049).expect("one ms before waits");
    assert!(r.cycle.is_none() && r.missed == 0);
    assert_eq!(loop_.last_ms(), 1_049);
    let r = loop_.step(1_050).expect("exactly on the tick runs");
    assert!(r.cycle.is_some() && r.missed == 0);
    assert_eq!(loop_.next_due_ms(), 1_100);
    loop_.step(1_100).expect("runs");
    assert_eq!(
        (loop_.cycles(), loop_.missed(), loop_.last_ms()),
        (3, 0, 1_100)
    );
    assert_eq!(log.borrow().len(), 15);
    // Control: a 25 ms period runs at 1000 and 1025, not at 1024.
    let log2 = Rc::new(RefCell::new(Vec::new()));
    let mut narrow =
        HostLoop::with_period(Rec::ok(log2), 1_000, NonZeroU64::new(25).expect("nonzero"));
    assert_eq!(narrow.period_ms(), 25);
    assert!(narrow.step(1_000).expect("runs").cycle.is_some());
    assert!(narrow.step(1_024).expect("waits").cycle.is_none());
    assert!(narrow.step(1_025).expect("runs").cycle.is_some());
}

#[test]
fn missed_slots_skipped_saturating() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut loop_ = HostLoop::new(Rec::ok(Rc::clone(&log)), 0);
    loop_.step(0).expect("runs");
    let r = loop_.step(210).expect("skips, never bursts");
    assert_eq!(
        (r.cycle.as_ref().expect("one cycle").now_ms, r.missed),
        (210, 3)
    );
    assert_eq!(loop_.next_due_ms(), 250);
    assert_eq!((loop_.cycles(), loop_.missed()), (2, 3));
    assert!(loop_.step(249).expect("waits").cycle.is_none());
    let r = loop_.step(250).expect("runs");
    assert_eq!(r.missed, 0);
    assert_eq!(loop_.next_due_ms(), 300);
    assert_eq!(log.borrow().len(), 15);
    // Control: every 50 ms runs six cycles with nothing missed.
    let log2 = Rc::new(RefCell::new(Vec::new()));
    let mut steady = HostLoop::new(Rec::ok(log2), 0);
    for now in [0, 50, 100, 150, 200, 250] {
        let r = steady.step(now).expect("runs");
        assert_eq!(r.missed, 0);
        assert!(r.cycle.is_some());
    }
    assert_eq!((steady.cycles(), steady.missed()), (6, 0));
    // u64 edge: the schedule saturates instead of wrapping.
    let log3 = Rc::new(RefCell::new(Vec::new()));
    let mut edge = HostLoop::new(Rec::ok(log3), u64::MAX - 100);
    let r = edge.step(u64::MAX).expect("runs");
    assert_eq!(r.missed, 2);
    assert_eq!(edge.next_due_ms(), u64::MAX);
    assert_eq!(edge.last_ms(), u64::MAX);
}

// ---------------------------------------------------------------------------
// B-2 time + LoopError texts.
// ---------------------------------------------------------------------------

#[test]
fn time_regression_changes_nothing() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut loop_ = HostLoop::new(Rec::ok(Rc::clone(&log)), 100);
    let err = loop_.step(99).expect_err("regression");
    assert_eq!(
        err,
        LoopError::TimeRegression {
            last_ms: 100,
            now_ms: 99
        }
    );
    assert!(log.borrow().is_empty());
    assert_eq!((loop_.next_due_ms(), loop_.cycles()), (100, 0));
    assert_eq!(format!("{err}"), "TimeRegression at run/step");
    loop_.step(100).expect("equal time accepted");
    assert!(loop_.step(100).expect("same time waits").cycle.is_none());
    let err = loop_.step(99).expect_err("last moved, still regression");
    assert_eq!(format!("{err}"), "TimeRegression at run/step");
    assert_eq!(log.borrow().len(), 5);
    assert_eq!(loop_.next_due_ms(), 150);
    // Hosting display wraps its source; the source is the stage error.
    let hosting: LoopError<HostError> = LoopError::Hosting(HostError::Closed);
    assert_eq!(
        format!("{hosting}"),
        format!("Hosting at run/step: {}", HostError::Closed)
    );
    assert_eq!(
        std::error::Error::source(&hosting).map(|s| s.to_string()),
        Some(HostError::Closed.to_string())
    );
}

// ---------------------------------------------------------------------------
// B-3 cycle order.
// ---------------------------------------------------------------------------

#[test]
fn cycle_order_and_report() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut loop_ = HostLoop::new(Rec::ok(Rc::clone(&log)), 0);
    let r = loop_.step(0).expect("runs");
    let cycle = r.cycle.expect("cycle");
    assert_eq!(cycle.now_ms, 0);
    assert_eq!(cycle.advance, ok_report());
    assert_eq!(
        log.borrow().iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        vec!["intake@0", "ai@0", "advance@0", "deliver@0", "retire"]
    );
    loop_.step(50).expect("runs");
    assert_eq!(log.borrow().len(), 10);
    assert_eq!(
        log.borrow().iter().map(|s| s.as_str()).collect::<Vec<_>>()[5..],
        ["intake@50", "ai@50", "advance@50", "deliver@50", "retire"]
    );
}

#[test]
fn first_error_stops_cycle() {
    let full = ["intake@0", "ai@0", "advance@0", "deliver@0", "retire"];
    for stage in 0..5 {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut loop_ = HostLoop::new(Rec::failing_at(Rc::clone(&log), stage), 0);
        let err = loop_.step(0).expect_err("first error wins");
        assert_eq!(
            format!("{err}"),
            format!("Hosting at run/step: {}", HostError::Closed)
        );
        {
            let borrowed = log.borrow();
            let got: Vec<&str> = borrowed.iter().map(|s| s.as_str()).collect();
            assert_eq!(got, full[..=stage]);
        }
        assert_eq!((loop_.cycles(), loop_.next_due_ms()), (1, 50));
        loop_.step(50).expect("fail-once consumed, full cycle");
        assert_eq!(log.borrow().len(), stage + 1 + 5);
    }
}

// ---------------------------------------------------------------------------
// B-4 pause + B-8 stages.
// ---------------------------------------------------------------------------

#[test]
fn pause_runs_every_stage_and_ai_hook() {
    let mut hosting = LocalHost::new(tb3_host());
    hosting.host_mut().pause(0).expect("pauses");
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let seen_hook = Arc::clone(&seen);
    let hook: AiHook = Box::new(move |host: &mut Host, now: u64| {
        let orders = host.awaiting_orders();
        seen_hook
            .lock()
            .expect("lock")
            .push((now, host.server_tick(), orders.clone()));
        if let Some(head) = orders.first() {
            let _ = host.host_action(&end(*head));
        }
        Ok(())
    });
    hosting.set_ai(Some(hook));
    let mut loop_ = HostLoop::new(hosting, 0);
    let r = loop_.step(0).expect("paused loop still cycles");
    let cycle = r.cycle.expect("cycle");
    assert_eq!(
        cycle.advance,
        AdvanceReport {
            ticked: false,
            timer_ended_turn: None,
            paused: Some(PauseReason::Operator),
        }
    );
    assert_eq!(seen.lock().expect("lock").clone(), vec![(0, 0, vec![a()])]);
    assert_eq!(loop_.hosting().host().awaiting_orders(), vec![b()]);
    assert_eq!(loop_.hosting().host().server_tick(), 0);
    // Control: unpaused ticks, the hook still ran first with the same record.
    let mut hosting = LocalHost::new(tb3_host());
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let seen_hook = Arc::clone(&seen);
    let hook: AiHook = Box::new(move |host: &mut Host, now: u64| {
        let orders = host.awaiting_orders();
        seen_hook
            .lock()
            .expect("lock")
            .push((now, host.server_tick(), orders.clone()));
        if let Some(head) = orders.first() {
            let _ = host.host_action(&end(*head));
        }
        Ok(())
    });
    hosting.set_ai(Some(hook));
    let mut loop_ = HostLoop::new(hosting, 0);
    let r = loop_.step(0).expect("runs");
    assert!(r.cycle.expect("cycle").advance.ticked);
    assert_eq!(seen.lock().expect("lock").clone(), vec![(0, 0, vec![a()])]);
    assert_eq!(loop_.hosting().host().server_tick(), 1);
    // Control: no hook leaves the head waiting.
    let mut loop_ = HostLoop::new(LocalHost::new(tb3_host()), 0);
    loop_.step(0).expect("runs");
    assert_eq!(loop_.hosting().host().awaiting_orders(), vec![a()]);
    // A failing hook is the cycle error and later stages do not run.
    let mut hosting = LocalHost::new(tb3_host());
    let hook: AiHook = Box::new(|_: &mut Host, _: u64| Err(HostError::Closed));
    hosting.set_ai(Some(hook));
    let mut loop_ = HostLoop::new(hosting, 0);
    let err = loop_.step(0).expect_err("hook error");
    assert_eq!(
        format!("{err}"),
        format!("Hosting at run/step: {}", HostError::Closed)
    );
}

// ---------------------------------------------------------------------------
// B-5 attach.
// ---------------------------------------------------------------------------

#[test]
fn attach_precedence_and_stats_default() {
    assert!(NonZeroUsize::new(0).is_none(), "capacity 0 has no value");
    assert_eq!(MAX_PEERS, 8);
    let mut hosting = LocalHost::new(tb3_host());
    let p = hosting
        .host_mut()
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds P");
    let q = hosting
        .host_mut()
        .bind_peer(control(&[a()]), full_grants(), 0)
        .expect("binds Q");
    hosting.host_mut().unbind_peer(q);
    assert_eq!(
        hosting.attach(q, cap(4)).map(|_| ()),
        Err(LocalError::UnknownPeer)
    );
    let _client = hosting.attach(p, cap(4)).expect("attaches");
    assert_eq!(
        hosting.attach(p, cap(4)).map(|_| ()),
        Err(LocalError::AlreadyAttached)
    );
    let stats = hosting.link_stats(p).expect("linked");
    assert_eq!(
        (stats.received, stats.refused, stats.held, stats.sent),
        (0, 0, false, 0)
    );
    assert_eq!(stats, LinkStats::default());
    assert_eq!(hosting.link_stats(q), None);
    assert_eq!(
        format!("{}", LocalError::UnknownPeer),
        "UnknownPeer at run/attach"
    );
    assert_eq!(
        format!("{}", LocalError::AlreadyAttached),
        "AlreadyAttached at run/attach"
    );
}

#[test]
fn live_peer_cap_is_eight() {
    let mut hosting = LocalHost::new(tb3_host());
    let mut peers = Vec::new();
    for _ in 0..MAX_PEERS {
        peers.push(
            hosting
                .host_mut()
                .bind_peer(control(&[]), full_grants(), 0)
                .expect("slot free"),
        );
    }
    assert_eq!(
        hosting.host_mut().bind_peer(control(&[]), full_grants(), 0),
        Err(HostError::ServerBusy)
    );
    assert_eq!(peers.len(), 8);
}

// ---------------------------------------------------------------------------
// B-6/B-7/B-8 round trip against a directly driven twin.
// ---------------------------------------------------------------------------

#[test]
fn round_trip_matches_twin() {
    let mut hosting = LocalHost::new(tb3_host());
    let p = hosting
        .host_mut()
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let mut client = hosting.attach(p, cap(64)).expect("attaches");
    let frame = end_frame(hosting.host(), p, 1);
    client.send(frame.as_slice()).expect("queues");
    let mut loop_ = HostLoop::new(hosting, 0);
    loop_.step(0).expect("cycles");
    // Twin: ingest, pump, advance, take entries then ack last, ack captures.
    let mut twin = tb3_host();
    let tp = twin
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let tframe = end_frame(&twin, tp, 1);
    assert_eq!(frame, tframe);
    twin.ingest(tp, &tframe, 0).expect("staged");
    twin.pump(0).expect("pumps");
    let advanced = twin.advance(0).expect("advances");
    assert!(advanced.ticked);
    let entries = twin.take_delivery_entries(tp).expect("bound");
    assert!(!entries.is_empty());
    if let Some(last) = entries.last() {
        twin.acknowledge_delivery(tp, last.ordinal).expect("acks");
    }
    twin.acknowledge_captures(twin.last_capture())
        .expect("acks");
    let got = drain_down(&mut client);
    assert_eq!(
        got,
        entries.iter().map(|e| e.bytes.clone()).collect::<Vec<_>>()
    );
    let host = loop_.hosting().host();
    assert_eq!(host.authority_hash(), twin.authority_hash());
    assert_eq!(host.server_tick(), 1);
    assert_eq!(host.server_tick(), twin.server_tick());
    assert_eq!(host.capture_acknowledged(), host.last_capture());
    assert_eq!(host.capture_acknowledged(), twin.capture_acknowledged());
    let stats = loop_.hosting().link_stats(p).expect("linked");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    assert!(!stats.held);
    assert_eq!(stats.sent, entries.len() as u64);
    // Next cycle with no frame matches the twin's next cycle.
    twin.pump(50).expect("pumps");
    twin.advance(50).expect("advances");
    let entries2 = twin.take_delivery_entries(tp).expect("bound");
    if let Some(last) = entries2.last() {
        twin.acknowledge_delivery(tp, last.ordinal).expect("acks");
    }
    twin.acknowledge_captures(twin.last_capture())
        .expect("acks");
    loop_.step(50).expect("cycles");
    assert_eq!(
        drain_down(&mut client),
        entries2.iter().map(|e| e.bytes.clone()).collect::<Vec<_>>()
    );
    assert_eq!(
        loop_.hosting().host().authority_hash(),
        twin.authority_hash()
    );
    // Control: no frame sent delivers different bytes than the command run.
    let mut hosting = LocalHost::new(tb3_host());
    let p = hosting
        .host_mut()
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let mut client = hosting.attach(p, cap(64)).expect("attaches");
    let mut loop_ = HostLoop::new(hosting, 0);
    loop_.step(0).expect("cycles");
    assert_ne!(drain_down(&mut client), got);
}

// ---------------------------------------------------------------------------
// B-6 transfer routing.
// ---------------------------------------------------------------------------

#[test]
fn transfer_frames_route_to_ingest_transfer() {
    // Fresh snapshot bind stages nothing; narrowing the grants stages transfer 1.
    let mut hosting = LocalHost::new(tb3_host());
    let w = hosting
        .host_mut()
        .bind_snapshot_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds snapshot");
    let fresh: Option<TransferState> = hosting.host().transfer_state(w).expect("state");
    assert_eq!(
        fresh,
        Some(TransferState {
            next_transfer: 1,
            in_flight: None,
        })
    );
    let mut narrowed = full_grants();
    narrowed.abilities = vec![strike()];
    assert_eq!(
        hosting.host_mut().set_grants(w, narrowed),
        Ok(GrantUpdate::SnapshotStaged)
    );
    assert_eq!(
        hosting.host().transfer_state(w).expect("state"),
        Some(TransferState {
            next_transfer: 2,
            in_flight: Some(1),
        })
    );
    let epoch = hosting.host().epoch(w).expect("bound");
    let mut client = hosting.attach(w, cap(64)).expect("attaches");
    client.send(published(epoch, 1).as_slice()).expect("queues");
    let mut loop_ = HostLoop::new(hosting, 0);
    loop_.step(0).expect("cycles");
    let st: Option<TransferState> = loop_.hosting().host().transfer_state(w).expect("state");
    assert_eq!(
        st,
        Some(TransferState {
            next_transfer: 2,
            in_flight: None,
        })
    );
    let stats = loop_.hosting().link_stats(w).expect("linked");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    // Control: no frame leaves the staged transfer in flight.
    let mut hosting = LocalHost::new(tb3_host());
    let w = hosting
        .host_mut()
        .bind_snapshot_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let mut narrowed = full_grants();
    narrowed.abilities = vec![strike()];
    assert_eq!(
        hosting.host_mut().set_grants(w, narrowed),
        Ok(GrantUpdate::SnapshotStaged)
    );
    let _client = hosting.attach(w, cap(64)).expect("attaches");
    let mut loop_ = HostLoop::new(hosting, 0);
    loop_.step(0).expect("cycles");
    let st: Option<TransferState> = loop_.hosting().host().transfer_state(w).expect("state");
    assert_eq!(
        st,
        Some(TransferState {
            next_transfer: 2,
            in_flight: Some(1),
        })
    );
    // Control: an unknown transfer is refused, counted, and the link lives.
    let mut hosting = LocalHost::new(tb3_host());
    let w = hosting
        .host_mut()
        .bind_snapshot_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let mut narrowed = full_grants();
    narrowed.abilities = vec![strike()];
    assert_eq!(
        hosting.host_mut().set_grants(w, narrowed),
        Ok(GrantUpdate::SnapshotStaged)
    );
    let epoch = hosting.host().epoch(w).expect("bound");
    let mut client = hosting.attach(w, cap(64)).expect("attaches");
    client.send(published(epoch, 9).as_slice()).expect("queues");
    let mut loop_ = HostLoop::new(hosting, 0);
    loop_.step(0).expect("cycles");
    let stats = loop_.hosting().link_stats(w).expect("linked");
    assert_eq!(stats.refused, 1);
    let st: Option<TransferState> = loop_.hosting().host().transfer_state(w).expect("state");
    assert_eq!(
        st,
        Some(TransferState {
            next_transfer: 2,
            in_flight: Some(1),
        })
    );
}

// ---------------------------------------------------------------------------
// B-6 refusals counted, drain cap.
// ---------------------------------------------------------------------------

#[test]
fn refusals_counted_and_dropped() {
    let mut hosting = LocalHost::new(tb3_host());
    let p = hosting
        .host_mut()
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let mut client = hosting.attach(p, cap(200)).expect("attaches");
    for seq in 1..=130 {
        client
            .send(end_frame(hosting.host(), p, seq).as_slice())
            .expect("up queue holds 200");
    }
    let mut loop_ = HostLoop::new(hosting, 0);
    loop_.step(0).expect("cycles");
    let stats = loop_.hosting().link_stats(p).expect("linked");
    assert_eq!(
        (stats.received, stats.refused, stats.held),
        (128, 48, false)
    );
    loop_.step(50).expect("cycles");
    let stats = loop_.hosting().link_stats(p).expect("linked");
    assert_eq!(
        (stats.received, stats.refused, stats.held),
        (130, 48, false)
    );
    // Control: within the burst nothing is refused.
    let mut hosting = LocalHost::new(tb3_host());
    let p = hosting
        .host_mut()
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let mut client = hosting.attach(p, cap(200)).expect("attaches");
    for seq in 1..=80 {
        client
            .send(end_frame(hosting.host(), p, seq).as_slice())
            .expect("queues");
    }
    let mut loop_ = HostLoop::new(hosting, 0);
    loop_.step(0).expect("cycles");
    let stats = loop_.hosting().link_stats(p).expect("linked");
    assert_eq!((stats.received, stats.refused), (80, 0));
}

// ---------------------------------------------------------------------------
// B-6 QueueFull hold and resume.
// ---------------------------------------------------------------------------

#[test]
fn queue_full_holds_and_resumes() {
    let mut hosting = LocalHost::new(tb3_host());
    let p = hosting
        .host_mut()
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds P");
    let s = hosting
        .host_mut()
        .bind_peer(control(&[]), full_grants(), 0)
        .expect("binds S");
    let mut pc = hosting.attach(p, cap(4096)).expect("attaches P");
    let mut sc = hosting.attach(s, cap(1)).expect("attaches S");
    let mut loop_ = HostLoop::new(hosting, 0);
    let mut sent = 0u64;
    let mut held_at: Option<u64> = None;
    for k in 1..=2000u64 {
        for j in 0..2u64 {
            let seq = 2 * (k - 1) + 1 + j;
            let head = loop_.hosting().host().awaiting_orders();
            let actor = if head.contains(&b()) {
                2
            } else if head.contains(&c()) {
                3
            } else {
                1
            };
            let epoch = loop_.hosting().host().epoch(p).expect("bound");
            let tick = loop_.hosting().host().server_tick();
            let frame = IntentFrame {
                epoch,
                lane: LANE_COMBAT,
                seq,
                observed_tick: tick,
                actor: n(actor),
                body: IntentBody::EndTurn,
            };
            let bytes = codec_v2::encode_intent(&frame).expect("encodes");
            pc.send(bytes.as_slice()).expect("up holds 4096");
            sent += 1;
        }
        drain_down(&mut pc);
        loop_.step(50 * k).expect("cycles");
        let stats = loop_.hosting().link_stats(p).expect("linked");
        assert_eq!(stats.refused, 0);
        if stats.held {
            held_at = Some(k);
            break;
        }
    }
    let held_at = held_at.expect("hold engages within 2000 cycles");
    let stats = loop_.hosting().link_stats(p).expect("linked");
    assert!(stats.received == sent || stats.received + 1 == sent);
    // S is read dry: the hold releases with nothing refused.
    for k in (held_at + 1)..=(held_at + 3) {
        drain_down(&mut sc);
        drain_down(&mut pc);
        loop_.step(50 * k).expect("cycles");
        if !loop_.hosting().link_stats(p).expect("linked").held {
            break;
        }
    }
    let stats = loop_.hosting().link_stats(p).expect("linked");
    assert!(!stats.held);
    assert_eq!(stats.received, sent);
    assert_eq!(stats.refused, 0);
    // Control: S read every cycle never holds in 300 cycles.
    let mut hosting = LocalHost::new(tb3_host());
    let p = hosting
        .host_mut()
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds P");
    let s = hosting
        .host_mut()
        .bind_peer(control(&[]), full_grants(), 0)
        .expect("binds S");
    let mut pc = hosting.attach(p, cap(4096)).expect("attaches P");
    let mut sc = hosting.attach(s, cap(1)).expect("attaches S");
    let mut loop_ = HostLoop::new(hosting, 0);
    for k in 1..=300u64 {
        let head = loop_.hosting().host().awaiting_orders();
        let actor = if head.contains(&b()) {
            2
        } else if head.contains(&c()) {
            3
        } else {
            1
        };
        let epoch = loop_.hosting().host().epoch(p).expect("bound");
        let tick = loop_.hosting().host().server_tick();
        let frame = IntentFrame {
            epoch,
            lane: LANE_COMBAT,
            seq: k,
            observed_tick: tick,
            actor: n(actor),
            body: IntentBody::EndTurn,
        };
        let bytes = codec_v2::encode_intent(&frame).expect("encodes");
        pc.send(bytes.as_slice()).expect("queues");
        drain_down(&mut pc);
        drain_down(&mut sc);
        loop_.step(50 * k).expect("cycles");
        assert!(!loop_.hosting().link_stats(p).expect("linked").held);
    }
}

// ---------------------------------------------------------------------------
// B-6/B-7/B-9 retirement.
// ---------------------------------------------------------------------------

#[test]
fn links_end_only_by_retirement() {
    // (a) dropped client: seen at intake, retired at deliver.
    let mut hosting = LocalHost::new(tb3_host());
    let p = hosting
        .host_mut()
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let client = hosting.attach(p, cap(64)).expect("attaches");
    drop(client);
    let mut loop_ = HostLoop::new(hosting, 0);
    loop_.step(0).expect("cycles");
    assert_eq!(loop_.hosting().link_stats(p), None);
    assert_eq!(loop_.hosting().host().epoch(p), Err(HostError::UnknownPeer));
    let retired = loop_.hosting_mut().take_retired();
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].0, p);
    assert_eq!((retired[0].1.received, retired[0].1.refused), (0, 0));
    assert!(loop_.hosting_mut().take_retired().is_empty());
    // (b) unbound peer: the queued frame is refused before the link retires.
    let mut hosting = LocalHost::new(tb3_host());
    let p = hosting
        .host_mut()
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let mut client = hosting.attach(p, cap(64)).expect("attaches");
    let mut loop_ = HostLoop::new(hosting, 0);
    loop_.step(0).expect("cycles");
    assert!(loop_.hosting_mut().take_retired().is_empty());
    let frame = end_frame(loop_.hosting().host(), p, 1);
    loop_.hosting_mut().host_mut().unbind_peer(p);
    client.send(frame.as_slice()).expect("up still queued");
    loop_.step(50).expect("cycles");
    assert_eq!(loop_.hosting().link_stats(p), None);
    let retired = loop_.hosting_mut().take_retired();
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].0, p);
    assert_eq!((retired[0].1.received, retired[0].1.refused), (1, 1));
    let got = drain_down(&mut client);
    assert!(!got.is_empty(), "frames sent at 0 still deliver");
    assert_eq!(client.recv(), None);
    assert!(client.is_closed());
    assert_eq!(client.send(&[0x78]), Err(LinkError::Closed));
    // (c) unbound with no frame retires at deliver with empty stats.
    let mut hosting = LocalHost::new(tb3_host());
    let p = hosting
        .host_mut()
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let _client = hosting.attach(p, cap(64)).expect("attaches");
    let mut loop_ = HostLoop::new(hosting, 0);
    loop_.step(0).expect("cycles");
    assert!(loop_.hosting_mut().take_retired().is_empty());
    loop_.hosting_mut().host_mut().unbind_peer(p);
    loop_.step(50).expect("cycles");
    let retired = loop_.hosting_mut().take_retired();
    assert_eq!(retired.len(), 1);
    assert_eq!((retired[0].1.received, retired[0].1.refused), (0, 0));
    // Control: an idle link stays through 3 cycles with no record.
    let mut hosting = LocalHost::new(tb3_host());
    let p = hosting
        .host_mut()
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let _client = hosting.attach(p, cap(64)).expect("attaches");
    let mut loop_ = HostLoop::new(hosting, 0);
    for now in [0, 50, 100] {
        loop_.step(now).expect("cycles");
        assert!(loop_.hosting().link_stats(p).is_some());
        assert!(loop_.hosting_mut().take_retired().is_empty());
    }
}

// ---------------------------------------------------------------------------
// B-7 full down queue.
// ---------------------------------------------------------------------------

#[test]
fn full_down_queue_acknowledges_only_what_was_sent() {
    // A fresh bind stages exactly one entry, so the run sends A's EndTurn first.
    let mut hosting = LocalHost::new(tb3_host());
    let p = hosting
        .host_mut()
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let mut client = hosting.attach(p, cap(1)).expect("attaches");
    let frame = end_frame(hosting.host(), p, 1);
    client.send(frame.as_slice()).expect("queues");
    let mut twin = tb3_host();
    let tp = twin
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let tframe = end_frame(&twin, tp, 1);
    assert_eq!(frame, tframe);
    twin.ingest(tp, &tframe, 0).expect("staged");
    twin.pump(0).expect("pumps");
    twin.advance(0).expect("advances");
    let want = twin.take_delivery_entries(tp).expect("bound");
    if let Some(last) = want.last() {
        twin.acknowledge_delivery(tp, last.ordinal).expect("acks");
    }
    twin.acknowledge_captures(twin.last_capture())
        .expect("acks");
    let n = want.len();
    assert!(n >= 2, "twin has the bind entry and the command delta");
    let mut loop_ = HostLoop::new(hosting, 0);
    loop_.step(0).expect("cycles");
    assert_eq!(loop_.hosting().link_stats(p).expect("linked").sent, 1);
    assert_eq!(client.recv().expect("first entry"), want[0].bytes);
    let mut got = vec![want[0].bytes.clone()];
    for k in 1..n {
        loop_.step(50 * k as u64).expect("cycles");
        got.push(client.recv().expect("next entry in order"));
    }
    assert_eq!(
        got,
        want.iter().map(|e| e.bytes.clone()).collect::<Vec<_>>()
    );
    assert_eq!(
        loop_.hosting().link_stats(p).expect("linked").sent,
        n as u64
    );
    // Control: a wide queue delivers everything in one cycle.
    let mut hosting = LocalHost::new(tb3_host());
    let p = hosting
        .host_mut()
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let mut client = hosting.attach(p, cap(64)).expect("attaches");
    let frame = end_frame(hosting.host(), p, 1);
    client.send(frame.as_slice()).expect("queues");
    let mut loop_ = HostLoop::new(hosting, 0);
    loop_.step(0).expect("cycles");
    assert_eq!(drain_down(&mut client).len(), n);
}

// ---------------------------------------------------------------------------
// B-10 LocalClient.
// ---------------------------------------------------------------------------

#[test]
fn local_client_errors() {
    let mut hosting = LocalHost::new(tb3_host());
    let p = hosting
        .host_mut()
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let mut client = hosting.attach(p, cap(2)).expect("attaches");
    client
        .send(end_frame(hosting.host(), p, 1).as_slice())
        .expect("first queues");
    client
        .send(end_frame(hosting.host(), p, 2).as_slice())
        .expect("second queues");
    assert_eq!(
        client.send(end_frame(hosting.host(), p, 3).as_slice()),
        Err(LinkError::Full)
    );
    assert_eq!(format!("{}", LinkError::Full), "Full at run/link");
    let mut loop_ = HostLoop::new(hosting, 0);
    loop_.step(0).expect("cycles");
    assert_eq!(loop_.hosting().link_stats(p).expect("linked").sent, 2);
    drop(loop_);
    let first = client.recv().expect("first frame still received");
    let second = client.recv().expect("second frame still received");
    assert_ne!(first, second);
    assert_eq!(client.recv(), None);
    assert!(client.is_closed());
    assert_eq!(client.send(&[0x78]), Err(LinkError::Closed));
    assert_eq!(format!("{}", LinkError::Closed), "Closed at run/link");
}

// ---------------------------------------------------------------------------
// Reading 14 + Send.
// ---------------------------------------------------------------------------

#[test]
fn loop_state_is_not_saved_and_send() {
    let mut hosting = LocalHost::new(tb3_host());
    let p = hosting
        .host_mut()
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    let _client = hosting.attach(p, cap(64)).expect("attaches");
    let hook: AiHook = Box::new(|_: &mut Host, _: u64| Ok(()));
    hosting.set_ai(Some(hook));
    let mut loop_ = HostLoop::new(hosting, 0);
    for now in [0, 50, 100] {
        loop_.step(now).expect("cycles");
    }
    let mut twin = tb3_host();
    let tp = twin
        .bind_peer(control(&[a(), b(), c()]), full_grants(), 0)
        .expect("binds");
    for now in [0, 50, 100] {
        twin.pump(now).expect("pumps");
        twin.advance(now).expect("advances");
        let entries = twin.take_delivery_entries(tp).expect("bound");
        if let Some(last) = entries.last() {
            twin.acknowledge_delivery(tp, last.ordinal).expect("acks");
        }
        twin.acknowledge_captures(twin.last_capture())
            .expect("acks");
    }
    assert_eq!(
        loop_.hosting().host().save_checkpoint().expect("saves"),
        twin.save_checkpoint().expect("saves")
    );
    assert_send::<LocalHost>();
    assert_send::<LocalClient>();
    assert_send::<HostLoop<LocalHost>>();
}

// ---------------------------------------------------------------------------
// B-9 retired list bound (K17).
// ---------------------------------------------------------------------------

#[test]
fn retired_records_are_bounded() {
    assert_eq!(MAX_RETIRED_RECORDS, 64);
    assert_eq!(MAX_PEERS, 8);
    // Control first: rounds 0..=7 hold exactly 64 records, oldest kept.
    let mut loop_ = HostLoop::new(LocalHost::new(tb3_host()), 0);
    let mut handles = Vec::new();
    for r in 0..=7u64 {
        let now = 50 * r;
        let mut round = Vec::new();
        for _ in 0..8 {
            let peer = loop_
                .hosting_mut()
                .host_mut()
                .bind_peer(control(&[]), full_grants(), now)
                .expect("slot free after last retirement");
            round.push(peer);
            let _client = loop_.hosting_mut().attach(peer, cap(1)).expect("attaches");
        }
        // Every client dropped before the step: departed at intake.
        let rpt = loop_.step(now).expect("cycles");
        assert!(rpt.cycle.is_some());
        assert_eq!(rpt.missed, 0);
        for peer in &round {
            assert_eq!(loop_.hosting().link_stats(*peer), None);
            assert_eq!(
                loop_.hosting().host().epoch(*peer),
                Err(HostError::UnknownPeer)
            );
        }
        handles.extend(round);
        assert_eq!(loop_.hosting().retired_dropped(), 0);
    }
    assert_eq!(handles.len(), 64);
    let mut ordered = handles.clone();
    ordered.sort();
    assert_eq!(handles, ordered, "handles grow in bind order");
    let retired = loop_.hosting_mut().take_retired();
    assert_eq!(retired.len(), 64);
    assert_eq!(retired.first().expect("records").0, handles[0]);
    assert_eq!(retired.last().expect("records").0, handles[63]);
    for record in &retired {
        assert_eq!((record.1.received, record.1.refused), (0, 0));
    }
    assert!(loop_.hosting_mut().take_retired().is_empty());
    assert_eq!(loop_.hosting().retired_dropped(), 0);
    // Full run: a 65th record drops the oldest and counts it.
    let mut loop_ = HostLoop::new(LocalHost::new(tb3_host()), 0);
    let mut handles = Vec::new();
    for r in 0..=8u64 {
        let now = 50 * r;
        let count = if r <= 7 { 8 } else { 1 };
        for _ in 0..count {
            let peer = loop_
                .hosting_mut()
                .host_mut()
                .bind_peer(control(&[]), full_grants(), now)
                .expect("slot free after last retirement");
            handles.push(peer);
            let _client = loop_.hosting_mut().attach(peer, cap(1)).expect("attaches");
        }
        let rpt = loop_.step(now).expect("cycles");
        assert!(rpt.cycle.is_some());
        assert_eq!(rpt.missed, 0);
        if r <= 7 {
            assert_eq!(loop_.hosting().retired_dropped(), 0);
        }
    }
    assert_eq!(handles.len(), 65);
    let mut ordered = handles.clone();
    ordered.sort();
    assert_eq!(handles, ordered);
    assert_eq!(loop_.hosting().retired_dropped(), 1);
    let retired = loop_.hosting_mut().take_retired();
    assert_eq!(retired.len(), 64);
    assert_eq!(retired.first().expect("records").0, handles[1]);
    assert_eq!(retired.last().expect("records").0, handles[64]);
    for record in &retired {
        assert_eq!((record.1.received, record.1.refused), (0, 0));
        assert!(!record.1.held);
    }
    assert!(loop_.hosting_mut().take_retired().is_empty());
    assert_eq!(loop_.hosting().retired_dropped(), 1);
}
