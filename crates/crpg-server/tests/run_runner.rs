//! T154b acceptance suite (`tasks/T154.md` U1–U7, U9–U10): the background
//! runner's clock, cadence, rendezvous, stopping, failures and respawn.
//!
//! Expected values come from the contract, never from the implementation.
//! The runner is driven by `ManualClock` (a test-local `Clock` behind an
//! `Arc<AtomicU64>`) and `call` rendezvous (R-3); no case asserts a
//! wall-clock duration, except U7's second half and U9's second half, which
//! use `SystemClock` with a 30 s failure deadline, as
//! `tests/support/quic_host_rig.rs:57` does. A "twin" is a second host built
//! the same way and driven by direct calls in the cycle order (`ingest` per
//! frame, `pump`, `advance`, `take_delivery_entries` then
//! `acknowledge_delivery` through the last, `acknowledge_captures(last_capture())`).

use std::fmt;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crpg_core::{EntityId, Ulid};
use crpg_net::codec_v2;
use crpg_net::protocol::{IntentBody, IntentFrame, NetId, LANE_COMBAT};
use crpg_net::transport::Transport;
use crpg_server::host::{
    AdvanceReport, ControlGrant, DisclosureGrants, EntityDisclosure, Host, HostConfig, HostError,
    IngestDisposition, PeerHandle, ProtocolSelection,
};
use crpg_server::run::{HostLoop, LocalHost, LoopError};
use crpg_server::runner::{spawn, Clock, RunnerGone, RunnerHandle, RunnerPanicked, SystemClock};
use crpg_sim::HistoryWorld;
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");

// ---------------------------------------------------------------------------
// Identities and fixtures (copied from `tests/run_loop.rs`).
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
        abilities: vec![
            Ulid::from_u128(603),
            Ulid::from_u128(604),
            Ulid::from_u128(605),
        ],
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

/// `Empty`: no encounter, incarnation 7, last time 0.
fn empty() -> Host {
    Host::new(7, config(), 0).expect("empty")
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
// The injected clock and the loop peek.
// ---------------------------------------------------------------------------

/// Test-local `Clock`: the test steps time by hand, never wall time.
#[derive(Debug, Clone)]
struct ManualClock {
    now: Arc<AtomicU64>,
}

impl ManualClock {
    fn new(now: u64) -> Self {
        Self {
            now: Arc::new(AtomicU64::new(now)),
        }
    }

    fn set(&self, now: u64) {
        self.now.store(now, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
}

/// `peek` (`tasks/T154.md`): one rendezvous read of the loop's schedule.
fn peek(host_loop: &HostLoop<LocalHost>) -> (u64, u64, u64, u64) {
    (
        host_loop.cycles(),
        host_loop.hosting().host().server_tick(),
        host_loop.missed(),
        host_loop.next_due_ms(),
    )
}

fn spawn_empty(clock: ManualClock) -> (RunnerHandle<LocalHost>, ManualClock) {
    let host_loop = HostLoop::new(LocalHost::new(empty()), 0);
    let handle = spawn(host_loop, clock.clone()).expect("spawns");
    (handle, clock)
}

// ---------------------------------------------------------------------------
// U1 `system_clock_is_monotonic` (R-1).
// ---------------------------------------------------------------------------

/// `SystemClock` never decreases and adds its origin without wrapping.
#[test]
fn system_clock_is_monotonic() {
    let wall = SystemClock::new();
    let first = wall.now_ms();
    let second = wall.now_ms();
    assert!(second >= first);
    assert!(first < 60_000);
    assert!(second < 60_000);

    let late = SystemClock::starting_at(5_000);
    let first = late.now_ms();
    let second = late.now_ms();
    assert!(second >= first);
    assert!((5_000..65_000).contains(&first));
    assert!((5_000..65_000).contains(&second));

    // `Default` is `new()`.
    let defaulted = SystemClock::default();
    assert!(defaulted.now_ms() < 60_000);

    assert_eq!(SystemClock::starting_at(u64::MAX).now_ms(), u64::MAX);
}

// ---------------------------------------------------------------------------
// U2 `rendezvous_and_cadence` (R-2, R-3).
// ---------------------------------------------------------------------------

/// A closure submitted after the clock reads `t` runs after a step at
/// `now >= t`; the thread keeps the grid, skipping late slots.
#[test]
fn rendezvous_and_cadence() {
    let clock = ManualClock::new(0);
    let (handle, clock) = spawn_empty(clock);

    assert_eq!(handle.call(|host_loop| peek(host_loop)), Ok((1, 1, 0, 50)));
    assert_eq!(handle.call(|host_loop| peek(host_loop)), Ok((1, 1, 0, 50)));
    clock.set(49);
    assert_eq!(handle.call(|host_loop| peek(host_loop)), Ok((1, 1, 0, 50)));
    clock.set(50);
    assert_eq!(handle.call(|host_loop| peek(host_loop)), Ok((2, 2, 0, 100)));
    clock.set(175);
    assert_eq!(handle.call(|host_loop| peek(host_loop)), Ok((3, 3, 1, 200)));
    let exit = handle.join().expect("joins");
    assert!(exit.error.is_none());

    // Control: the same loop stepped by hand at 0, 49, 50, 175.
    let mut direct = HostLoop::new(LocalHost::new(empty()), 0);
    direct.step(0).expect("runs");
    assert_eq!(peek(&direct), (1, 1, 0, 50));
    direct.step(0).expect("waits");
    assert_eq!(peek(&direct), (1, 1, 0, 50));
    direct.step(49).expect("waits");
    assert_eq!(peek(&direct), (1, 1, 0, 50));
    direct.step(50).expect("runs");
    assert_eq!(peek(&direct), (2, 2, 0, 100));
    let report = direct.step(175).expect("runs");
    assert_eq!(report.missed, 1);
    assert_eq!(peek(&direct), (3, 3, 1, 200));
}

// ---------------------------------------------------------------------------
// U3 `client_round_trip_on_the_thread` (R-3).
// ---------------------------------------------------------------------------

/// Frames sent through a `LocalClient` run on the thread; the thread's
/// delivery and hash equal a twin's.
#[test]
fn client_round_trip_on_the_thread() {
    let mut host = tb3();
    let peer = bind(&mut host);
    let mut local = LocalHost::new(host);
    let mut client = local.attach(peer, cap(64)).expect("attaches");
    let clock = ManualClock::new(0);
    let handle = spawn(HostLoop::new(local, 0), clock.clone()).expect("spawns");

    assert_eq!(handle.call(|host_loop| host_loop.cycles()), Ok(1));

    let mut twin = tb3();
    let twin_peer = bind(&mut twin);
    let bind_frames = twin_cycle(&mut twin, twin_peer, &[], 0);
    assert_eq!(drain(&mut client), bind_frames);

    let frame = end_frame(&twin, twin_peer, 1);
    client.send(&frame).expect("sends");
    let twin_second = twin_cycle(&mut twin, twin_peer, &[frame], 50);

    clock.set(50);
    assert_eq!(handle.call(|host_loop| host_loop.cycles()), Ok(2));
    assert_eq!(drain(&mut client), twin_second);
    let (hash, tick) = handle
        .call(|host_loop| {
            (
                host_loop.hosting().host().authority_hash(),
                host_loop.hosting().host().server_tick(),
            )
        })
        .expect("peeks");
    assert_eq!(hash, twin.authority_hash());
    assert_eq!(tick, twin.server_tick());
    assert_eq!(tick, 2);
    let exit = handle.join().expect("joins");
    assert!(exit.error.is_none());
}

// ---------------------------------------------------------------------------
// U4 `stop_join_and_drop` (R-4).
// ---------------------------------------------------------------------------

/// `stop` ends the runner at its wait with no error and no further step;
/// dropping a handle stops and joins the same way.
#[test]
fn stop_join_and_drop() {
    let clock = ManualClock::new(0);
    let (handle, _clock) = spawn_empty(clock);
    handle.stop();
    handle.stop();
    let exit = handle.join().expect("joins");
    assert_eq!(exit.error, None);
    assert_eq!(exit.host_loop.cycles(), 1);

    // A dropped handle over a `P` link: the rendezvous still returns, the
    // client reads what was queued, then the queue ends and closes.
    let mut host = tb3();
    let peer = bind(&mut host);
    let mut local = LocalHost::new(host);
    let mut client = local.attach(peer, cap(64)).expect("attaches");
    let clock = ManualClock::new(0);
    let handle = spawn(HostLoop::new(local, 0), clock).expect("spawns");
    let rendezvous = handle.submit(|host_loop| host_loop.cycles());
    drop(handle);
    assert_eq!(rendezvous.recv(), Ok(1));

    let mut twin = tb3();
    let twin_peer = bind(&mut twin);
    let bind_frames = twin_cycle(&mut twin, twin_peer, &[], 0);
    assert_eq!(drain(&mut client), bind_frames);
    assert_eq!(client.recv(), None);
    assert!(client.is_closed());
    let frame = end_frame(&twin, twin_peer, 1);
    assert_eq!(
        client.send(&frame),
        Err(crpg_server::run::LinkError::Closed)
    );
}

// ---------------------------------------------------------------------------
// U5 `step_error_ends_the_runner` (R-5).
// ---------------------------------------------------------------------------

/// A step error ends the runner; a pending closure is dropped, so its `call`
/// and every later one return `RunnerGone`.
#[test]
fn step_error_ends_the_runner() {
    let clock = ManualClock::new(0);
    let (handle, clock) = spawn_empty(clock);
    handle
        .call(|host_loop| host_loop.hosting_mut().host_mut().shutdown())
        .expect("shuts down");
    clock.set(50);
    assert_eq!(handle.call(|host_loop| peek(host_loop)), Err(RunnerGone));
    assert_eq!(handle.call(|host_loop| peek(host_loop)), Err(RunnerGone));
    let exit = handle.join().expect("joins");
    assert_eq!(exit.error, Some(LoopError::Hosting(HostError::Closed)));
    assert_eq!(exit.host_loop.cycles(), 2);
}

// ---------------------------------------------------------------------------
// U6 `panic_is_reported` (R-5).
// ---------------------------------------------------------------------------

/// A panic on the thread drops the closure (`RunnerGone`) and reports
/// `RunnerPanicked` from `join`.
#[test]
fn panic_is_reported() {
    let clock = ManualClock::new(0);
    let (handle, _clock) = spawn_empty(clock);
    assert_eq!(
        handle.call(|_| panic!("U6 panics on the loop thread")),
        Err(RunnerGone)
    );
    let Err(error) = handle.join() else {
        panic!("join must report the panic")
    };
    assert_eq!(error, RunnerPanicked);
}

// ---------------------------------------------------------------------------
// U7 `wall_clock_liveness` (R-2).
// ---------------------------------------------------------------------------

/// On `SystemClock` the thread steps on its own: three ticks, then a clean
/// stop.
#[test]
fn wall_clock_liveness() {
    let host_loop = HostLoop::new(LocalHost::new(empty()), 0);
    let handle = spawn(host_loop, SystemClock::new()).expect("spawns");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let tick = handle
            .call(|host_loop| host_loop.hosting().host().server_tick())
            .expect("peeks");
        if tick >= 3 {
            break;
        }
        assert!(Instant::now() < deadline, "the thread must tick on its own");
    }
    let exit = handle.join().expect("joins");
    assert_eq!(exit.error, None);
    assert!(exit.host_loop.cycles() >= 3);
}

// ---------------------------------------------------------------------------
// U9 `clock_origin_runs_a_late_started_host` (R-1, R-2; Reading 13).
// ---------------------------------------------------------------------------

/// The clock's origin must equal the loop's start: a host whose last time is
/// 1,000 runs on a clock at 1,000 and ends at once on a clock at 0.
#[test]
fn clock_origin_runs_a_late_started_host() {
    let late = || {
        HostLoop::new(
            LocalHost::new(Host::new(7, config(), 1_000).expect("late")),
            1_000,
        )
    };

    let clock = ManualClock::new(1_000);
    let handle = spawn(late(), clock.clone()).expect("spawns");
    assert_eq!(
        handle.call(|host_loop| peek(host_loop)),
        Ok((1, 1, 0, 1_050))
    );
    clock.set(1_050);
    assert_eq!(
        handle.call(|host_loop| peek(host_loop)),
        Ok((2, 2, 0, 1_100))
    );
    let exit = handle.join().expect("joins");
    assert_eq!(exit.error, None);

    // Control: the same loop on a clock at 0, as `SystemClock::new()` would
    // read: the first step is a `TimeRegression` and the runner ends.
    let handle = spawn(late(), ManualClock::new(0)).expect("spawns");
    assert_eq!(handle.call(|host_loop| host_loop.cycles()), Err(RunnerGone));
    let exit = handle.join().expect("joins");
    assert_eq!(
        exit.error,
        Some(LoopError::TimeRegression {
            last_ms: 1_000,
            now_ms: 0
        })
    );
    assert_eq!(exit.host_loop.cycles(), 0);

    // The wall-clock half: `starting_at` the load time runs a loaded host.
    let handle = spawn(late(), SystemClock::starting_at(1_000)).expect("spawns");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let tick = handle
            .call(|host_loop| host_loop.hosting().host().server_tick())
            .expect("peeks");
        if tick >= 2 {
            break;
        }
        assert!(Instant::now() < deadline, "the thread must tick on its own");
    }
    let exit = handle.join().expect("joins");
    assert_eq!(exit.error, None);
}

// ---------------------------------------------------------------------------
// U10 `stop_and_respawn_keeps_the_local_client` (R-4; Reading 10).
// ---------------------------------------------------------------------------

/// `join` returns the loop with its links and schedule; respawning it keeps
/// the local client's channels.
#[test]
fn stop_and_respawn_keeps_the_local_client() {
    let mut host = tb3();
    let peer = bind(&mut host);
    let mut local = LocalHost::new(host);
    let mut client = local.attach(peer, cap(64)).expect("attaches");
    let clock = ManualClock::new(0);
    let handle = spawn(HostLoop::new(local, 0), clock.clone()).expect("spawns");
    assert_eq!(handle.call(|host_loop| host_loop.cycles()), Ok(1));

    let mut twin = tb3();
    let twin_peer = bind(&mut twin);
    let bind_frames = twin_cycle(&mut twin, twin_peer, &[], 0);
    assert_eq!(drain(&mut client), bind_frames);

    let exit = handle.join().expect("joins");
    assert_eq!(exit.error, None);
    let last = exit.host_loop.last_ms();
    assert_eq!(last, 0);
    assert!(!client.is_closed());
    // Reading: the clock is set before the respawn, so the respawned
    // thread's first step deterministically reads 50 (one cycle, `missed`
    // 1) instead of racing the test thread between 0 and 50. Every asserted
    // value is the contract's: the loop restarts its grid at `last`, and a
    // step at 0 would tick once more (server tick 3); only a first step at
    // 50 runs exactly the command cycle the twin ran.
    let frame = end_frame(&twin, twin_peer, 1);
    client.send(&frame).expect("sends");
    clock.set(50);
    let respawned = spawn(
        HostLoop::new(exit.host_loop.into_hosting(), last),
        clock.clone(),
    )
    .expect("respawns");

    let twin_second = twin_cycle(&mut twin, twin_peer, &[frame], 50);
    assert_eq!(
        respawned.call(|host_loop| host_loop.hosting().host().server_tick()),
        Ok(2)
    );
    assert_eq!(drain(&mut client), twin_second);
    let (hash, stats) = respawned
        .call(move |host_loop| {
            (
                host_loop.hosting().host().authority_hash(),
                host_loop.hosting().link_stats(peer),
            )
        })
        .expect("peeks");
    assert_eq!(hash, twin.authority_hash());
    let stats = stats.expect("link kept");
    assert_eq!(stats.received, 1);
    assert_eq!(stats.refused, 0);
    let exit = respawned.join().expect("joins");
    assert!(exit.error.is_none());

    // Control: rebuilding the hosting from the bare host drops the links, so
    // the client's queue ends and closes after what was already queued.
    let mut host = tb3();
    let peer = bind(&mut host);
    let mut local = LocalHost::new(host);
    let mut client = local.attach(peer, cap(64)).expect("attaches");
    let handle = spawn(HostLoop::new(local, 0), ManualClock::new(0)).expect("spawns");
    assert_eq!(handle.call(|host_loop| host_loop.cycles()), Ok(1));
    let queued = drain(&mut client);
    assert!(!queued.is_empty());
    let exit = handle.join().expect("joins");
    let host = exit.host_loop.into_hosting().into_host();
    drop(LocalHost::new(host));
    assert_eq!(client.recv(), None);
    assert!(client.is_closed());
}

// ---------------------------------------------------------------------------
// Fix round: the thread steps on its own (t07/t08). Every `call` forces a
// step, so U2/U9 never show a self-driven one. This case advances the
// injected clock past the period and spins, with a 30 s deadline and no
// `call`, until a step-counting `Hosting` shows a cycle ran.
// ---------------------------------------------------------------------------

/// A `Hosting` that counts every `advance`, so the test sees cycles without
/// any `call` rendezvous forcing a step.
struct StepCountingHosting {
    steps: Arc<AtomicU64>,
}

#[derive(Debug, PartialEq, Eq)]
struct StepCountingError;

impl fmt::Display for StepCountingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StepCountingError")
    }
}

impl std::error::Error for StepCountingError {}

impl crpg_server::run::Hosting for StepCountingHosting {
    type Error = StepCountingError;

    fn intake(&mut self, _now_ms: u64) -> Result<(), Self::Error> {
        Ok(())
    }

    fn drive_ai(&mut self, _now_ms: u64) -> Result<(), Self::Error> {
        Ok(())
    }

    fn advance(&mut self, _now_ms: u64) -> Result<AdvanceReport, Self::Error> {
        self.steps.fetch_add(1, Ordering::SeqCst);
        Ok(AdvanceReport {
            ticked: true,
            timer_ended_turn: None,
            paused: None,
        })
    }

    fn deliver(&mut self, _now_ms: u64) -> Result<(), Self::Error> {
        Ok(())
    }

    fn retire_captures(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// The loop thread keeps the grid without any caller rendezvous: a wait made
/// huge (t07/t08) would leave this spin short of its second cycle.
#[test]
fn thread_steps_on_its_own() {
    let steps = Arc::new(AtomicU64::new(0));
    let clock = ManualClock::new(0);
    let hosting = StepCountingHosting {
        steps: steps.clone(),
    };
    let handle = spawn(HostLoop::new(hosting, 0), clock.clone()).expect("spawns");
    let deadline = Instant::now() + Duration::from_secs(30);
    // The first step runs on its own right after `spawn`, with no `call`.
    while steps.load(Ordering::SeqCst) < 1 {
        assert!(Instant::now() < deadline, "the thread must step on its own");
        std::thread::yield_now();
    }
    // Advance the clock past the period; the next cycle must run on its own.
    clock.set(500);
    while steps.load(Ordering::SeqCst) < 2 {
        assert!(Instant::now() < deadline, "the thread must step on its own");
        std::thread::yield_now();
    }
    let exit = handle.join().expect("joins");
    assert_eq!(exit.error, None);
}

// ---------------------------------------------------------------------------
// Fix round: `stop` stops on its own (h01) and `is_finished` reports it
// (h02/h03). After `stop()` alone the test spins on `is_finished()` with a
// 30 s deadline until it is true, then `join()` gives `error: None`.
// ---------------------------------------------------------------------------

/// `stop()` ends the runner without a further rendezvous, and `is_finished`
/// is false while running and true after the stop.
#[test]
fn stop_ends_the_runner_on_its_own() {
    let clock = ManualClock::new(0);
    let (handle, _clock) = spawn_empty(clock);
    assert!(!handle.is_finished());
    handle.stop();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !handle.is_finished() {
        assert!(Instant::now() < deadline, "stop must end the thread");
        std::thread::yield_now();
    }
    assert!(handle.is_finished());
    let exit = handle.join().expect("joins");
    assert_eq!(exit.error, None);
}

// ---------------------------------------------------------------------------
// Fix round: dropping the handle joins (h06). The loop owns a `Clock` whose
// `Drop` sets a shared flag; the flag is set immediately after `drop`
// returns only when `Drop` joined instead of detaching.
// ---------------------------------------------------------------------------

/// A `Clock` that reports its own drop, so the test sees the loop thread
/// end when the handle is dropped.
struct DropNotifyingClock {
    now: Arc<AtomicU64>,
    dropped: Arc<AtomicBool>,
}

impl Drop for DropNotifyingClock {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl Clock for DropNotifyingClock {
    fn now_ms(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
}

/// Dropping an unjoined handle stops and joins the thread (R-4): the clock
/// the thread owns is dropped before `drop(handle)` returns.
#[test]
fn dropping_the_handle_joins_the_thread() {
    let dropped = Arc::new(AtomicBool::new(false));
    let handle = spawn(
        HostLoop::new(LocalHost::new(empty()), 0),
        DropNotifyingClock {
            now: Arc::new(AtomicU64::new(0)),
            dropped: dropped.clone(),
        },
    )
    .expect("spawns");
    drop(handle);
    assert!(dropped.load(Ordering::SeqCst));
}

// ---------------------------------------------------------------------------
// Fix round: `Stop` runs no further step (t14). With a 3_600_000 ms period
// no timeout can fire, so `cycles() == 1` after `stop()` + `join()` proves
// the stop itself stepped nothing.
// ---------------------------------------------------------------------------

/// `stop()` ends the runner at its wait with no further step.
#[test]
fn stop_runs_no_further_step() {
    let clock = ManualClock::new(0);
    let period = NonZeroU64::new(3_600_000).expect("nonzero");
    let handle = spawn(
        HostLoop::with_period(LocalHost::new(empty()), 0, period),
        clock.clone(),
    )
    .expect("spawns");
    // Pin the first cycle at clock 0, so a step at the period is a second cycle.
    assert_eq!(handle.call(|host_loop| host_loop.cycles()), Ok(1));
    clock.set(3_600_000);
    handle.stop();
    let exit = handle.join().expect("joins");
    assert_eq!(exit.error, None);
    assert_eq!(exit.host_loop.cycles(), 1);
}

// ---------------------------------------------------------------------------
// Fix round: the loop thread's contract name (t01, R-2).
// ---------------------------------------------------------------------------

/// The background thread is named exactly `crpg-server-loop`.
#[test]
fn loop_thread_has_the_contract_name() {
    let clock = ManualClock::new(0);
    let (handle, _clock) = spawn_empty(clock);
    assert_eq!(
        handle.call(|_| std::thread::current().name().map(str::to_owned)),
        Ok(Some("crpg-server-loop".to_owned()))
    );
    let exit = handle.join().expect("joins");
    assert_eq!(exit.error, None);
}

// ---------------------------------------------------------------------------
// Fix round: `SystemClock` measures whole milliseconds (s04/s05/s08), and
// `Default` is `starting_at(0)`.
// ---------------------------------------------------------------------------

/// `new()`, `default()` and `starting_at(5_000)` read whole milliseconds
/// since construction, bracketed above by the test's own `Instant`.
#[test]
fn system_clock_measures_whole_milliseconds() {
    let t0 = Instant::now();
    let clock = SystemClock::new();
    let t1 = Instant::now();
    while t1.elapsed() < Duration::from_millis(30) {}
    let now = clock.now_ms();
    assert!(now >= 30);
    assert!(now <= t0.elapsed().as_millis() as u64);

    let t0 = Instant::now();
    let clock = SystemClock::default();
    let t1 = Instant::now();
    while t1.elapsed() < Duration::from_millis(30) {}
    let now = clock.now_ms();
    assert!(now >= 30);
    assert!(now <= t0.elapsed().as_millis() as u64);

    let t0 = Instant::now();
    let clock = SystemClock::starting_at(5_000);
    let t1 = Instant::now();
    while t1.elapsed() < Duration::from_millis(30) {}
    let now = clock.now_ms();
    assert!(now >= 5_030);
    assert!(now <= 5_000 + t0.elapsed().as_millis() as u64);
}

// ---------------------------------------------------------------------------
// Fix round: the millisecond addition saturates (s02).
// ---------------------------------------------------------------------------

/// `starting_at(u64::MAX)` still reads `u64::MAX` after time passes.
#[test]
fn system_clock_saturates_at_max() {
    let clock = SystemClock::starting_at(u64::MAX);
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(2) {}
    assert_eq!(clock.now_ms(), u64::MAX);
}

// ---------------------------------------------------------------------------
// Fix round: the runner error `Display` texts (d01/d02, Errors table).
// ---------------------------------------------------------------------------

/// `RunnerGone` and `RunnerPanicked` carry exactly the contract's texts.
#[test]
fn runner_error_display_texts() {
    assert_eq!(RunnerGone.to_string(), "RunnerGone at run/runner");
    assert_eq!(RunnerPanicked.to_string(), "RunnerPanicked at run/runner");
}
