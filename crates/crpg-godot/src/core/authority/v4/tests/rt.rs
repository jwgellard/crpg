//! T050a: the real-time v4 open, `controlled()` and `render_positions()`
//! (W-1..W-4) on the walk campaign. Every case drives a manual clock and
//! `call` rendezvous, never wall time. The helpers are `pub(crate)` so
//! T049a's tests can import them.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crpg_server::run::{HostLoop, LocalHost};
use crpg_server::runner::Clock;

use crate::core::client::{SessionView, ViewMoveReceipt};
use crate::core::link::{LocalLink, LINK_FRAMES};
use crate::core::{BridgeCode, BridgeCore};

// ---------------------------------------------------------------------------
// Helpers (authority/v4/tests.rs, copied; the originals are private there).
// ---------------------------------------------------------------------------

/// A manually stepped runner clock (`authority/v4/tests.rs:26–49`, copied).
#[derive(Clone)]
pub(crate) struct ManualClock {
    now: Arc<AtomicU64>,
}

impl ManualClock {
    /// A clock at 0.
    pub(crate) fn new() -> Self {
        Self {
            now: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Stores `t`; the runner reads it on its next step.
    pub(crate) fn set(&self, t: u64) {
        self.now.store(t, Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now.load(Ordering::Relaxed)
    }
}

/// `<CARGO_MANIFEST_DIR>/../../campaigns/demo/walk`.
pub(crate) fn walk_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("campaigns")
        .join("demo")
        .join("walk")
}

/// `<CARGO_MANIFEST_DIR>/../../campaigns/fixtures/combat_basic`.
pub(crate) fn basic_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("campaigns")
        .join("fixtures")
        .join("combat_basic")
}

/// `core.runner().expect(..).call(f).expect(..)` (`tests.rs:253–269`).
pub(crate) fn call<R, F>(core: &BridgeCore, f: F) -> R
where
    R: Send + 'static,
    F: FnOnce(&mut HostLoop<LocalHost>) -> R + Send + 'static,
{
    core.runner()
        .expect("a live v4 session")
        .call(f)
        .expect("runner alive")
}

/// One rendezvous and two polls at `t` (`tests.rs:291–323`, copied).
///
/// Returns the first non-zero of the two polls (else 0) and the views after
/// each poll whose revision rose.
pub(crate) fn settle(
    core: &mut BridgeCore,
    clock: &ManualClock,
    t: u64,
) -> (BridgeCode, Vec<Rc<SessionView>>) {
    clock.set(t);
    call(core, |_| ());
    let mut views = Vec::new();
    let mut previous = core.view().revision();
    let first = core.poll(t);
    if core.view().revision() != previous {
        views.push(core.view());
        previous = core.view().revision();
    }
    call(core, |_| ());
    let second = core.poll(t);
    if core.view().revision() != previous {
        views.push(core.view());
    }
    let code = if first != BridgeCode::Ok {
        first
    } else {
        second
    };
    (code, views)
}

/// Every view's `move_receipts()`, concatenated in order.
pub(crate) fn move_receipts(views: &[Rc<SessionView>]) -> Vec<ViewMoveReceipt> {
    views
        .iter()
        .flat_map(|view| view.move_receipts().iter().cloned())
        .collect()
}

// ---------------------------------------------------------------------------
// Test-local helpers.
// ---------------------------------------------------------------------------

/// Opens WALK as a real-time v4 session (the player's side is the first
/// row's, `"Hero"`).
fn open_rt(core: &mut BridgeCore, clock: &ManualClock) -> BridgeCode {
    core.open_v4_rt_with(
        walk_root().to_str().expect("walk path"),
        "",
        "",
        1,
        clock.clone(),
    )
}

/// Opens WALK as a turn-based v4 session.
fn open_tb(core: &mut BridgeCore, clock: &ManualClock) -> BridgeCode {
    core.open_v4_with(
        walk_root().to_str().expect("walk path"),
        "",
        "",
        1,
        clock.clone(),
    )
}

/// A receipt without its tick fields: `(seq, actor, outcome, code)`.
fn shape(receipts: &[ViewMoveReceipt]) -> Vec<(i64, i64, &'static str, i64)> {
    receipts
        .iter()
        .map(|r| (r.seq, r.actor, r.outcome, r.code))
        .collect()
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

// ---------------------------------------------------------------------------
// W1.
// ---------------------------------------------------------------------------

#[test]
fn rt_open_publishes_the_walk_world() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_rt(&mut core, &clock), BridgeCode::Ok);
    let (code, _views) = settle(&mut core, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);

    let view = core.view();
    assert_eq!(view.net_ids(), &[1, 2, 3]);
    assert_eq!(
        view.names(),
        strings(&["Hero", "Hero", "Goblin"]).as_slice()
    );
    assert_eq!(
        view.sides(),
        strings(&["Hero", "Hero", "Goblin"]).as_slice()
    );
    assert_eq!(view.position_known(), &[1, 1, 1]);
    assert_eq!(
        view.positions(),
        &[196608, 0, 327680, 196608, 0, 458752, 786432, 0, 393216]
    );
    assert_eq!(view.moving(), &[0, 0, 0]);
    assert_eq!(view.active(), 0);
    assert!(view.option_labels().is_empty());
    assert_eq!(core.controlled(), vec![1, 2]);
    assert_eq!(core.render_positions(), view.positions().to_vec());
    assert_eq!(
        core.render_positions(),
        vec![196608, 0, 327680, 196608, 0, 458752, 786432, 0, 393216]
    );
    drop(view);
    core.close();

    // Control: the same campaign opened turn-based has an active actor.
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_tb(&mut core, &clock), BridgeCode::Ok);
    let (code, _views) = settle(&mut core, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);
    assert_ne!(core.view().active(), 0);
    assert_eq!(core.controlled(), vec![1, 2]);
    core.close();
}

// ---------------------------------------------------------------------------
// W2.
// ---------------------------------------------------------------------------

#[test]
fn rt_move_to_walks_to_the_destination() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_rt(&mut core, &clock), BridgeCode::Ok);
    let (code, _views) = settle(&mut core, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);

    assert_eq!(core.move_to(1, 327680, 0, 327680), BridgeCode::Ok);
    let mut views = Vec::new();
    for k in 1..=40u64 {
        let (code, mut step) = settle(&mut core, &clock, 50 * k);
        assert_eq!(code, BridgeCode::Ok, "settle({})", 50 * k);
        views.append(&mut step);
    }
    let receipts = move_receipts(&views);
    assert_eq!(receipts.len(), 1);
    let first = receipts[0];
    assert_eq!(first.seq, 1);
    assert_eq!(first.actor, 1);
    assert_eq!(first.outcome, "accepted");
    assert_eq!(first.code, 0);
    assert_eq!(first.start_tick, first.processed_tick + 1);

    assert!(
        views.iter().any(|view| view.moving()[0] == 1
            && view.positions()[0] > 196608
            && view.positions()[0] < 327680),
        "no view showed the hero mid-walk"
    );
    let last = views.last().expect("views were published");
    assert_eq!(&last.positions()[0..3], &[327680, 0, 327680]);
    assert_eq!(last.moving()[0], 0);
    assert_eq!(&last.positions()[3..6], &[196608, 0, 458752]);
    assert_eq!(&last.positions()[6..9], &[786432, 0, 393216]);
    assert_eq!(last.moving(), &[0, 0, 0]);
    assert_eq!(core.render_positions(), core.view().positions().to_vec());

    // Control A: the Goblin is not the player's.
    assert_eq!(core.move_to(3, 0, 0, 0), BridgeCode::Ok);
    let (code_a, mut views) = settle(&mut core, &clock, 2050);
    assert_eq!(code_a, BridgeCode::Ok);
    let (code_b, mut more) = settle(&mut core, &clock, 2100);
    assert_eq!(code_b, BridgeCode::Ok);
    views.append(&mut more);
    assert_eq!(shape(&move_receipts(&views)), vec![(2, 3, "rejected", 17)]);
    assert_eq!(&core.view().positions()[6..9], &[786432, 0, 393216]);
    core.close();

    // Control B: turn-based refuses lane 1 with WrongMode.
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_tb(&mut core, &clock), BridgeCode::Ok);
    let (code, _views) = settle(&mut core, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);
    assert_eq!(core.move_to(1, 327680, 0, 327680), BridgeCode::Ok);
    let (code_a, mut views) = settle(&mut core, &clock, 50);
    assert_eq!(code_a, BridgeCode::Ok);
    let (code_b, mut more) = settle(&mut core, &clock, 100);
    assert_eq!(code_b, BridgeCode::Ok);
    views.append(&mut more);
    assert_eq!(shape(&move_receipts(&views)), vec![(1, 1, "rejected", 22)]);
    assert_eq!(&core.view().positions()[0..3], &[196608, 0, 327680]);
    core.close();
}

// ---------------------------------------------------------------------------
// W3.
// ---------------------------------------------------------------------------

#[test]
fn rt_open_needs_round_ticks() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    let code = core.open_v4_rt_with(
        basic_root().to_str().expect("basic path"),
        "",
        "",
        1,
        clock.clone(),
    );
    assert_eq!(code, BridgeCode::StartFailed);
    assert_eq!(code as i64, 25);
    assert_eq!(
        core.last_error(),
        "START_FAILED: ModeUnsupported at combat/mode"
    );
    assert!(core.runner().is_none());
    assert_eq!(core.view().revision(), 0);
    assert!(core.controlled().is_empty());

    // Control: the same campaign opens turn-based.
    let mut core = BridgeCore::new();
    let code = core.open_v4_with(
        basic_root().to_str().expect("basic path"),
        "",
        "",
        1,
        clock.clone(),
    );
    assert_eq!(code, BridgeCode::Ok);
    assert!(core.runner().is_some());
    core.close();
}

// ---------------------------------------------------------------------------
// W4.
// ---------------------------------------------------------------------------

#[test]
fn controlled_and_render_positions_by_state() {
    let mut core = BridgeCore::new();
    assert!(core.controlled().is_empty());
    assert!(core.render_positions().is_empty());

    // Open (v2): no controlled list, zeros for positions.
    let link = LocalLink::new(LINK_FRAMES);
    let dir = basic_root();
    assert_eq!(
        core.open_on(dir.to_str().expect("basic path"), "", "", 1, &link),
        BridgeCode::Ok
    );
    assert_eq!(core.poll(0), BridgeCode::Ok);
    assert!(core.controlled().is_empty());
    assert_eq!(core.render_positions(), vec![0; 6]);
    core.close();

    // Open v4 as the Goblin: the Goblin's replica id only.
    let clock = ManualClock::new();
    assert_eq!(
        core.open_v4_with(
            basic_root().to_str().expect("basic path"),
            "",
            "Goblin",
            1,
            clock.clone()
        ),
        BridgeCode::Ok
    );
    let (code, _views) = settle(&mut core, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);
    assert_eq!(core.controlled(), vec![2]);
    assert_eq!(core.render_positions(), core.view().positions().to_vec());
    assert_eq!(core.render_positions().len(), 6);
    core.close();
    assert!(core.controlled().is_empty());
    assert!(core.render_positions().is_empty());
}

// ---------------------------------------------------------------------------
// W5.
// ---------------------------------------------------------------------------

#[test]
fn rt_open_precedence() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    let dir = walk_root();
    let dir = dir.to_str().expect("walk path");

    assert_eq!(
        core.open_v4_rt_with(dir, "", "", -1, clock.clone()),
        BridgeCode::BadArgument
    );
    assert_eq!(core.last_error(), "BAD_ARGUMENT: bad argument");
    assert!(core.runner().is_none());
    assert_eq!(
        core.open_v4_rt_with(dir, "nope", "", 1, clock.clone()),
        BridgeCode::BadArgument
    );
    assert_eq!(core.last_error(), "BAD_ARGUMENT: bad argument");
    assert!(core.runner().is_none());

    assert_eq!(open_rt(&mut core, &clock), BridgeCode::Ok);
    let (code, _views) = settle(&mut core, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);

    assert_eq!(
        core.open_v4_rt_with(dir, "", "", 1, clock.clone()),
        BridgeCode::AlreadyOpen
    );
    assert_eq!(core.last_error(), "ALREADY_OPEN: session already open");
    assert_eq!(
        core.open_v4_with(dir, "", "", 1, clock.clone()),
        BridgeCode::AlreadyOpen
    );
    let link = LocalLink::new(LINK_FRAMES);
    assert_eq!(core.open_on(dir, "", "", 1, &link), BridgeCode::AlreadyOpen);

    // The first session is untouched.
    let (code, _views) = settle(&mut core, &clock, 50);
    assert_eq!(code, BridgeCode::Ok);
    assert_eq!(core.controlled(), vec![1, 2]);
    assert_eq!(
        core.view().positions(),
        &[196608, 0, 327680, 196608, 0, 458752, 786432, 0, 393216]
    );

    // Control: after `close`, the open succeeds.
    core.close();
    assert_eq!(open_rt(&mut core, &clock), BridgeCode::Ok);
    core.close();
}
