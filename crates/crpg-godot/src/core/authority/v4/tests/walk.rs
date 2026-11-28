//! T157e: `open_v4_walk`, `request_mode` and the movement columns (K-1..K-6)
//! on the `campaigns/demo/adventure` campaign. Every case drives a manual
//! clock and `call` rendezvous, never wall time.

use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use crpg_core::Fx16_16;
use crpg_net::protocol_v3::PauseRights;
use crpg_sim::seg_len;

use super::rt_tests::{basic_root, call, settle, ManualClock};
use super::AI_STEP_DELAY_MS;
use crate::core::client::{SessionView, ViewEvent, ViewMoveReceipt, ViewReceipt};
use crate::core::{BridgeCode, BridgeCore, Probe};

/// The AI's per-step delay.
const D: u64 = AI_STEP_DELAY_MS;

/// `<CARGO_MANIFEST_DIR>/../../campaigns/demo/adventure` as text.
fn adventure() -> String {
    adventure_root()
        .to_str()
        .expect("adventure path")
        .to_string()
}

fn adventure_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("campaigns")
        .join("demo")
        .join("adventure")
}

/// The `combat_basic` root as text.
fn basic() -> String {
    basic_root().to_str().expect("fixture path").to_string()
}

/// A copied campaign tree removed on drop (`authority/v4/tests.rs`, copied).
struct TempCampaign {
    root: PathBuf,
}

impl TempCampaign {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "crpg-godot-t157e-{}-{unique}-{label}",
            std::process::id()
        ));
        if root.exists() {
            let _ = fs::remove_dir_all(&root);
        }
        copy_tree(&adventure_root(), &root);
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TempCampaign {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("create temp root");
    for entry in fs::read_dir(source).expect("read campaign") {
        let entry = entry.expect("campaign entry");
        let from = entry.path();
        let to = target.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to);
        } else {
            fs::copy(&from, &to).expect("copy campaign file");
        }
    }
}

/// NO-ENGAGE: the adventure whose encounter says `"engage": []`.
fn no_engage_campaign() -> TempCampaign {
    let campaign = TempCampaign::new("no-engage");
    let file = campaign.path().join("encounters").join("first-blood.json");
    let text = fs::read_to_string(&file).expect("encounter");
    let start = text.find("\"engage\": [").expect("the engage key");
    let end = start + text[start..].find("],").expect("the end of engage") + 2;
    let replaced = format!("{}\"engage\": [],{}", &text[..start], &text[end..]);
    assert!(replaced.contains("\"engage\": [],"));
    assert!(!replaced.contains("proximity"));
    fs::write(&file, replaced).expect("write encounter");
    campaign
}

/// What a run has collected: every event with the time of its settle, every
/// lane-0 receipt, every move receipt and every view's `active()`.
#[derive(Default)]
struct Seen {
    events: Vec<(u64, ViewEvent)>,
    receipts: Vec<ViewReceipt>,
    moves: Vec<ViewMoveReceipt>,
    actives: Vec<i64>,
}

impl Seen {
    fn collect(&mut self, t: u64, views: &[Rc<SessionView>]) {
        for view in views {
            self.events
                .extend(view.events().iter().cloned().map(|event| (t, event)));
            self.receipts.extend_from_slice(view.receipts());
            self.moves.extend_from_slice(view.move_receipts());
            self.actives.push(view.active());
        }
    }

    fn all_active_zero(&self) -> bool {
        self.actives.iter().all(|active| *active == 0)
    }

    fn applied_receipt(&self) -> bool {
        self.receipts
            .iter()
            .any(|receipt| receipt.applied && receipt.code == 0)
    }

    fn refused_with(&self, code: i64) -> bool {
        self.receipts
            .iter()
            .any(|receipt| !receipt.applied && receipt.code == code)
    }

    fn turn_of(&self, who: i64) -> bool {
        self.events.iter().any(
            |(_, event)| matches!(event, ViewEvent::TurnStarted { actor, .. } if *actor == who),
        )
    }

    fn moved(&self, who: i64) -> Vec<(u64, Vec<i64>)> {
        self.events
            .iter()
            .filter_map(|(t, event)| match event {
                ViewEvent::Moved { entity, points } if *entity == who => Some((*t, points.clone())),
                _ => None,
            })
            .collect()
    }

    fn move_receipt(&self, actor: i64, outcome: &str, code: Option<i64>) -> bool {
        self.moves.iter().any(|receipt| {
            receipt.actor == actor
                && receipt.outcome == outcome
                && code.is_none_or(|code| receipt.code == code)
        })
    }
}

/// One session on a manual clock; `t` is the time of the last settle.
struct Session {
    core: BridgeCore,
    clock: ManualClock,
    t: u64,
}

impl Session {
    /// "Opened walk": the adventure through `open_v4_walk_with`, then `settle(0)`.
    fn opened_walk() -> Self {
        Self::opened(|core, clock| core.open_v4_walk_with(&adventure(), "", "", 1, clock))
    }

    fn opened(open: impl FnOnce(&mut BridgeCore, ManualClock) -> BridgeCode) -> Self {
        let mut core = BridgeCore::new();
        let clock = ManualClock::new();
        assert_eq!(open(&mut core, clock.clone()), BridgeCode::Ok);
        let (code, _) = settle(&mut core, &clock, 0);
        assert_eq!(code, BridgeCode::Ok, "settle(0)");
        Self { core, clock, t: 0 }
    }

    /// "Start the fight": opened walk, `request_mode(0)`, run until
    /// `active() == 3`, at most to 1000.
    fn started_fight() -> (Self, Seen) {
        let mut session = Self::opened_walk();
        assert_eq!(session.core.request_mode(0), BridgeCode::Ok);
        let mut seen = Seen::default();
        session.run_until(1000, &mut seen, "active() == 3", |core, _| {
            core.view().active() == 3
        });
        (session, seen)
    }

    /// One settle at `t + 50`, asserted clean; the views it produced.
    fn step(&mut self) -> Vec<Rc<SessionView>> {
        self.t += 50;
        let (code, views) = settle(&mut self.core, &self.clock, self.t);
        assert_eq!(code, BridgeCode::Ok, "settle({})", self.t);
        views
    }

    /// "Run until P, at most to L".
    fn run_until(
        &mut self,
        limit: u64,
        seen: &mut Seen,
        what: &str,
        predicate: impl Fn(&BridgeCore, &Seen) -> bool,
    ) {
        loop {
            let views = self.step();
            seen.collect(self.t, &views);
            if predicate(&self.core, seen) {
                return;
            }
            assert!(self.t < limit, "never reached: {what} (t = {})", self.t);
        }
    }

    /// `host.executions()`, read on the loop thread.
    fn executions(&self) -> u64 {
        call(&self.core, |l| l.hosting().host().executions())
    }

    /// Submits, at the view's revision, the option whose target is `target`.
    fn submit_target(&mut self, target: i64) {
        let view = self.core.view();
        let index = view
            .option_targets()
            .iter()
            .position(|entry| *entry == target)
            .unwrap_or_else(|| panic!("no option targets {target}: {:?}", view.option_labels()));
        assert_eq!(
            self.core
                .submit_option(view.revision(), i64::try_from(index).expect("index")),
            BridgeCode::Ok
        );
    }
}

/// `seg_len` of rows `i` and `j` of the view's positions.
fn dist(view: &SessionView, i: usize, j: usize) -> u64 {
    let at = |row: usize| {
        let p = &view.positions()[3 * row..3 * row + 3];
        [
            Fx16_16::from_raw(i32::try_from(p[0]).expect("x")),
            Fx16_16::from_raw(i32::try_from(p[1]).expect("y")),
            Fx16_16::from_raw(i32::try_from(p[2]).expect("z")),
        ]
    };
    seg_len(at(i), at(j))
}

fn is_turn_started(event: &ViewEvent, who: i64) -> bool {
    matches!(event, ViewEvent::TurnStarted { actor, .. } if *actor == who)
}

// ---------------------------------------------------------------------------
// K1 the walk open (K-1, K-2, K-5, K-6).
// ---------------------------------------------------------------------------

#[test]
fn walk_open() {
    let session = Session::opened_walk();
    let view = session.core.view();
    assert_eq!(view.net_ids(), &[1, 2, 3]);
    assert_eq!(view.names(), &["Hero", "Hero", "Goblin"]);
    assert_eq!(view.sides(), &["Heroes", "Heroes", "Goblins"]);
    assert_eq!(
        view.positions(),
        &[196608, 0, 327680, 196608, 0, 458752, 786432, 0, 393216]
    );
    assert_eq!(view.active(), 0);
    assert!(view.option_labels().is_empty());
    assert_eq!(session.core.controlled(), vec![1, 2]);
    let (player, _) = session.core.v4_peers().expect("a v4 session");
    assert_eq!(
        call(&session.core, move |l| l
            .hosting()
            .host()
            .pause_rights(player)),
        Ok(PauseRights {
            pause: true,
            unpause: true
        })
    );
    assert_eq!(view.movement(), &[6, 6, -1]);
    assert_eq!(view.movement_max(), &[6, 6, -1]);
    // K-6: the hero's health is 20 (1310720 raw); the goblin's is 6 (393216).
    assert_eq!(view.health(), &[20, 20, 6]);
    assert_eq!(view.max_health(), &[20, 20, 6]);
    assert_eq!(view.health_known(), &[1, 1, 1]);

    // Control: the real-time open of the same campaign binds neither the
    // pause rights nor the pool grants.
    let rt = Session::opened(|core, clock| core.open_v4_rt_with(&adventure(), "", "", 1, clock));
    let (player, _) = rt.core.v4_peers().expect("a v4 session");
    assert_eq!(
        call(&rt.core, move |l| l.hosting().host().pause_rights(player)),
        Ok(PauseRights::default())
    );
    assert_eq!(rt.core.view().movement(), &[-1, -1, -1]);

    // Control: a ruleset with no movement declares no pool.
    let ai = Session::opened(|core, clock| core.open_v4_ai_with(&basic(), "", "", 1, clock));
    assert_eq!(ai.core.view().movement(), &[-1, -1]);
}

// ---------------------------------------------------------------------------
// K2 the walk open's precedence (K-1).
// ---------------------------------------------------------------------------

#[test]
fn walk_open_precedence() {
    let clock = ManualClock::new();
    let mut core = BridgeCore::new();
    assert_eq!(
        core.open_v4_walk_with(&adventure(), "", "", -1, clock.clone()),
        BridgeCode::BadArgument
    );
    assert_eq!(core.last_error(), "BAD_ARGUMENT: bad argument");
    assert!(core.runner().is_none());
    assert_eq!(
        core.open_v4_walk_with(&adventure(), "nope", "", 1, clock.clone()),
        BridgeCode::BadArgument
    );
    assert!(core.runner().is_none());
    assert_eq!(
        core.open_v4_walk_with(&adventure(), "", "Nobody", 1, clock.clone()),
        BridgeCode::UnknownSide
    );
    assert!(core.runner().is_none());

    let walk_code = core.open_v4_walk_with(&basic(), "", "", 1, clock.clone());
    assert_eq!(walk_code, BridgeCode::StartFailed);
    let walk_error = core.last_error().to_string();
    assert!(core.runner().is_none());
    let mut second = BridgeCore::new();
    assert_eq!(
        second.open_v4_rt_with(&basic(), "", "", 1, clock.clone()),
        walk_code
    );
    assert_eq!(second.last_error(), walk_error);
    assert!(second.runner().is_none());

    let mut session = Session::opened_walk();
    assert_eq!(
        session
            .core
            .open_v4_walk_with(&adventure(), "", "", 1, session.clock.clone()),
        BridgeCode::AlreadyOpen
    );
    assert_eq!(
        session.core.last_error(),
        "ALREADY_OPEN: session already open"
    );
    assert_eq!(
        session
            .core
            .open_v4_rt_with(&adventure(), "", "", 1, session.clock.clone()),
        BridgeCode::AlreadyOpen
    );
    let (code, _) = settle(&mut session.core, &session.clock, 50);
    assert_eq!(code, BridgeCode::Ok);
    session.core.close();
    assert_eq!(
        session
            .core
            .open_v4_walk_with(&adventure(), "", "", 1, session.clock.clone()),
        BridgeCode::Ok
    );
}

// ---------------------------------------------------------------------------
// K3 the goblin waits while you walk (Reading 5).
// ---------------------------------------------------------------------------

#[test]
fn the_goblin_waits_while_you_walk() {
    let mut session = Session::opened_walk();
    let mut seen = Seen::default();
    while session.t < 3000 {
        let views = session.step();
        seen.collect(session.t, &views);
        let view = session.core.view();
        assert_eq!(view.active(), 0, "t = {}", session.t);
        assert_eq!(
            &view.positions()[6..9],
            &[786432, 0, 393216],
            "t = {}",
            session.t
        );
    }
    assert!(seen.all_active_zero());
    assert!(
        !seen.events.iter().any(|(_, event)| matches!(
            event,
            ViewEvent::ActionResolved { .. }
                | ViewEvent::Moved { .. }
                | ViewEvent::TurnStarted { .. }
        )),
        "{:?}",
        seen.events
    );
    assert_eq!(session.executions(), 0);

    assert_eq!(session.core.move_to(1, 196608, 0, 196608), BridgeCode::Ok);
    let mut walked = Seen::default();
    session.run_until(6000, &mut walked, "the hero arrives", |core, _| {
        let view = core.view();
        view.positions()[0..3] == [196608, 0, 196608] && view.moving()[0] == 0
    });
    assert!(walked.move_receipt(1, "accepted", Some(0)));
    assert_eq!(session.core.view().active(), 0);
}

// ---------------------------------------------------------------------------
// K4 a request starts the fight (K-3, K-4).
// ---------------------------------------------------------------------------

#[test]
fn request_mode_starts_the_fight() {
    let mut session = Session::opened_walk();
    let _ = session.step();
    let _ = session.step();
    assert_eq!(session.t, 100);
    let x = session.executions();
    assert_eq!(session.core.request_mode(0), BridgeCode::Ok);

    let mut seen = Seen::default();
    session.run_until(300, &mut seen, "the mode receipt", |_, seen| {
        seen.applied_receipt()
    });
    assert!(
        seen.all_active_zero(),
        "no view before the receipt is active"
    );

    session.run_until(800, &mut seen, "active() == 3", |core, _| {
        core.view().active() == 3
    });
    let turn_start = session.t;
    assert!(
        seen.events
            .iter()
            .any(|(t, event)| *t == turn_start && is_turn_started(event, 3)),
        "the goblin's turn starts in the settle that shows it active"
    );
    assert_eq!(session.executions(), x + 1);

    // The goblin waits `D` before its first step.
    while session.t <= turn_start + D - 150 {
        let views = session.step();
        seen.collect(session.t, &views);
    }
    assert!(seen.moved(3).is_empty(), "{:?}", seen.moved(3));
    session.run_until(
        turn_start + D + 100,
        &mut seen,
        "the goblin moves",
        |_, seen| !seen.moved(3).is_empty(),
    );
    let moved = seen.moved(3);
    let points = &moved[0].1;
    assert_eq!(&points[0..3], &[786432, 0, 393216]);
    let end = &points[points.len() - 3..];
    let reach = |metres: i128| (metres * 65536) * (metres * 65536);
    let d2 = i128::from(end[0] - 786432).pow(2) + i128::from(end[2] - 393216).pow(2);
    assert!(reach(5) < d2 && d2 <= reach(6), "end {end:?}, squared {d2}");

    // Hero 1's turn: nothing in reach but the end of the turn.
    session.run_until(turn_start + 3 * D, &mut seen, "hero 1's turn", |_, seen| {
        seen.turn_of(1)
    });
    assert_eq!(session.core.view().option_labels(), &["End turn"]);
    assert_eq!(session.executions(), x + 1);
}

// ---------------------------------------------------------------------------
// K5 the host engages when you walk near (S-D2 end to end).
// ---------------------------------------------------------------------------

#[test]
fn the_host_engages_when_you_walk_near() {
    let mut session = Session::opened_walk();
    assert_eq!(session.core.move_to(1, 589824, 0, 393216), BridgeCode::Ok);
    let mut seen = Seen::default();
    session.run_until(10_000, &mut seen, "the host starts the fight", |core, _| {
        core.view().active() == 3
    });
    let view = session.core.view();
    assert_eq!(view.moving()[0], 0);
    assert!(dist(&view, 0, 2) <= 4 * 65536, "{}", dist(&view, 0, 2));

    // Control: with `"engage": []` the same walk never starts a fight.
    let campaign = no_engage_campaign();
    let root = campaign.path().to_str().expect("temp path").to_string();
    let mut session =
        Session::opened(|core, clock| core.open_v4_walk_with(&root, "", "", 1, clock));
    assert_eq!(session.core.move_to(1, 589824, 0, 393216), BridgeCode::Ok);
    let mut seen = Seen::default();
    session.run_until(10_000, &mut seen, "the hero arrives", |core, _| {
        let view = core.view();
        view.positions()[0..3] == [589824, 0, 393216] && view.moving()[0] == 0
    });
    while session.t < 10_000 {
        let views = session.step();
        seen.collect(session.t, &views);
    }
    assert!(seen.all_active_zero());
    assert_eq!(session.core.view().active(), 0);
}

// ---------------------------------------------------------------------------
// K6 a request without the pause right is refused (K-2, K-3).
// ---------------------------------------------------------------------------

#[test]
fn a_request_without_the_pause_right_is_refused() {
    let mut session =
        Session::opened(|core, clock| core.open_v4_rt_with(&adventure(), "", "", 1, clock));
    assert_eq!(session.core.request_mode(0), BridgeCode::Ok);
    let mut seen = Seen::default();
    while session.t < 1500 {
        let views = session.step();
        seen.collect(session.t, &views);
        assert_eq!(session.core.view().active(), 0);
    }
    assert!(seen.refused_with(17), "{:?}", seen.receipts);
    assert!(seen.all_active_zero());
}

// ---------------------------------------------------------------------------
// K7 the request's argument precedence (K-3, K-4).
// ---------------------------------------------------------------------------

#[test]
fn request_mode_precedence() {
    let mut closed = BridgeCore::new();
    assert_eq!(closed.request_mode(2), BridgeCode::BadArgument);
    assert_eq!(closed.last_error(), "BAD_ARGUMENT: bad mode");
    assert_eq!(closed.request_mode(-1), BridgeCode::BadArgument);
    assert_eq!(closed.request_mode(0), BridgeCode::NotOpen);

    let mut v2 = BridgeCore::new();
    assert_eq!(v2.open(&basic(), "", "", 1), BridgeCode::Ok);
    assert_eq!(v2.request_mode(5), BridgeCode::BadArgument);
    assert_eq!(v2.last_error(), "BAD_ARGUMENT: bad mode");
    assert_eq!(v2.request_mode(0), BridgeCode::BadArgument);
    assert_eq!(v2.last_error(), "BAD_ARGUMENT: not a v4 session");

    let mut session = Session::opened_walk();
    assert_eq!(session.core.request_mode(1), BridgeCode::Ok);
    assert_eq!(session.core.request_mode(0), BridgeCode::IntentPending);
    assert_eq!(
        session.core.last_error(),
        "INTENT_PENDING: an intent is outstanding"
    );
    let mut seen = Seen::default();
    session.run_until(300, &mut seen, "the receipt", |_, seen| {
        !seen.receipts.is_empty()
    });
    assert!(seen.applied_receipt(), "{:?}", seen.receipts);
    assert_eq!(session.core.request_mode(0), BridgeCode::Ok);

    let mut probed = Session::opened_walk();
    probed.core.probe = Some(Probe::Submit);
    assert_eq!(probed.core.request_mode(0), BridgeCode::Panicked);
    assert_eq!(probed.core.request_mode(0), BridgeCode::Panicked);
}

// ---------------------------------------------------------------------------
// K8 the party wins and walks on (S-A4, S-C3, S-D3).
// ---------------------------------------------------------------------------

#[test]
fn the_party_wins_and_walks_on() {
    let (mut session, mut seen) = Session::started_fight();
    session.run_until(3000, &mut seen, "hero 1's turn", |_, seen| seen.turn_of(1));

    // Control: a lane-1 move by a live turn-based participant is refused.
    let before = session.core.view().positions()[0..3].to_vec();
    assert_eq!(session.core.move_to(1, 262144, 0, 327680), BridgeCode::Ok);
    let mut refused = Seen::default();
    session.run_until(session.t + 3000, &mut refused, "the refusal", |_, seen| {
        !seen.moves.is_empty()
    });
    assert!(
        refused.move_receipt(1, "rejected", Some(22)),
        "{:?}",
        refused.moves
    );
    assert_eq!(session.core.view().positions()[0..3], before[..]);

    // Play the fight out: strike the goblin when it is offered, else end the
    // turn.
    let mut awaiting = false;
    let mut played = Seen::default();
    while !session.core.view().ended() {
        assert!(session.t < 120_000, "the fight did not end");
        let views = session.step();
        played.collect(session.t, &views);
        if views.iter().any(|view| !view.receipts().is_empty()) {
            awaiting = false;
        }
        let view = session.core.view();
        if !awaiting && !view.option_labels().is_empty() {
            let target = if view.option_targets().contains(&3) {
                3
            } else {
                0
            };
            session.submit_target(target);
            awaiting = true;
        }
    }
    let view = session.core.view();
    assert_eq!(view.dead(), &[0, 0, 1]);
    assert_eq!(view.active(), 0);

    // The survivors walk.
    let (hx2, hz2) = (view.positions()[3], view.positions()[5]);
    assert_eq!(session.core.move_to(2, hx2 + 65536, 0, hz2), BridgeCode::Ok);
    let start = session.t;
    let mut walked = Seen::default();
    session.run_until(start + 3000, &mut walked, "hero 2 walks", |core, seen| {
        let view = core.view();
        seen.move_receipt(2, "accepted", None)
            && view.positions()[3..6] == [hx2 + 65536, 0, hz2]
            && view.moving()[1] == 0
    });
}

// ---------------------------------------------------------------------------
// K9 the movement readout follows the pool (K-5, K-6).
// ---------------------------------------------------------------------------

/// Runs K9's start: the fight, then hero 1's turn with only "End turn".
fn heroes_turn() -> (Session, Seen) {
    let (mut session, mut seen) = Session::started_fight();
    session.run_until(3000, &mut seen, "hero 1's turn", |core, seen| {
        seen.turn_of(1) && core.view().option_labels() == ["End turn"]
    });
    (session, seen)
}

/// `declare_move` at the current revision, then run until a lane-0 receipt.
fn declare(session: &mut Session, x: i64, z: i64) -> Seen {
    let revision = session.core.view().revision();
    assert_eq!(session.core.declare_move(revision, x, 0, z), BridgeCode::Ok);
    let mut seen = Seen::default();
    let limit = session.t + 3000;
    session.run_until(limit, &mut seen, "the move receipt", |_, seen| {
        !seen.receipts.is_empty()
    });
    seen
}

#[test]
fn movement_readout_follows_the_pool() {
    let (mut session, _) = heroes_turn();
    let (hx, hz) = (196608, 327680);
    let view = session.core.view();
    assert_eq!(view.movement(), &[6, 6, -1]);
    assert_eq!(view.movement_max(), &[6, 6, -1]);

    // (1) 1.5 m costs 2 of 6.
    let seen = declare(&mut session, hx, hz - 98304);
    assert!(seen.applied_receipt(), "{:?}", seen.receipts);
    let moved = seen.moved(1);
    assert_eq!(moved.len(), 1, "{moved:?}");
    assert_eq!(&moved[0].1[moved[0].1.len() - 3..], &[hx, 0, hz - 98304]);
    let view = session.core.view();
    assert_eq!(view.movement(), &[4, 6, -1]);
    assert_eq!(view.movement_max(), &[6, 6, -1]);

    // (2) 4.5 m costs 5 of 4.
    let before = session.core.view().positions()[0..3].to_vec();
    let seen = declare(&mut session, hx + 294912, hz - 98304);
    assert!(seen.refused_with(18), "{:?}", seen.receipts);
    let view = session.core.view();
    assert_eq!(view.positions()[0..3], before[..]);
    assert_eq!(view.movement()[0], 4);
    assert_eq!(view.movement_max(), &[6, 6, -1]);

    // (3) 3.5 m costs 4 of 4.
    let seen = declare(&mut session, hx + 229376, hz - 98304);
    assert!(seen.applied_receipt(), "{:?}", seen.receipts);
    let view = session.core.view();
    assert_eq!(view.movement()[0], 0);
    assert_eq!(view.movement_max(), &[6, 6, -1]);

    // (4) 0.5 m costs 1 of 0.
    let seen = declare(&mut session, hx + 229376, hz - 131072);
    assert!(seen.refused_with(18), "{:?}", seen.receipts);
    assert_eq!(session.core.view().movement_max(), &[6, 6, -1]);
}

// ---------------------------------------------------------------------------
// K10 the reach is one and a half metres (K-6).
// ---------------------------------------------------------------------------

#[test]
fn reach_is_one_and_a_half_metres() {
    let (mut session, _) = heroes_turn();
    let (hx, hz) = (196608, 327680);
    let _ = declare(&mut session, hx, hz - 98304);
    let _ = declare(&mut session, hx + 229376, hz - 98304);

    // Hero 1 ends its turn; hero 2 ends its turn.
    session.submit_target(0);
    let mut seen = Seen::default();
    session.run_until(
        session.t + 3000,
        &mut seen,
        "hero 2's turn",
        |core, seen| seen.turn_of(2) && !core.view().option_labels().is_empty(),
    );
    assert_eq!(session.core.view().option_labels(), &["End turn"]);
    let round = seen
        .events
        .iter()
        .find_map(|(_, event)| match event {
            ViewEvent::TurnStarted { actor: 2, round } => Some(*round),
            _ => None,
        })
        .expect("hero 2's turn");
    session.submit_target(0);

    // The goblin walks into reach of hero 1 and strikes it.
    let mut later = Seen::default();
    session.run_until(
        session.t + 10_000,
        &mut later,
        "hero 1's next turn",
        |_, seen| {
            seen.events.iter().any(|(_, event)| {
            matches!(event, ViewEvent::TurnStarted { actor: 1, round: r } if *r > round)
        })
        },
    );
    let strike = later.events.iter().position(|(_, event)| {
        matches!(
            event,
            ViewEvent::ActionResolved { actor: 3, target: 1, ability, .. } if ability == "Strike"
        )
    });
    let turn = later.events.iter().position(
        |(_, event)| matches!(event, ViewEvent::TurnStarted { actor: 1, round: r } if *r > round),
    );
    assert!(
        strike.is_some() && strike < turn,
        "the goblin's strike precedes hero 1's turn: {:?}",
        later.events
    );
    assert_eq!(
        session.core.view().option_labels(),
        &["Strike -> Goblin", "End turn"]
    );
}

// ---------------------------------------------------------------------------
// K11 the withheld columns and the demo data (K-5, K-6).
// ---------------------------------------------------------------------------

#[test]
fn v2_and_default_views_withhold_movement() {
    // K-5: the default view has both columns empty.
    let empty = BridgeCore::new();
    assert!(empty.view().movement().is_empty());
    assert!(empty.view().movement_max().is_empty());

    // K-5: a v2 session fills both with -1 per row.
    let mut core = BridgeCore::new();
    assert_eq!(core.open(&basic(), "", "", 1), BridgeCode::Ok);
    assert_eq!(core.poll(0), BridgeCode::Ok);
    let view = core.view();
    let rows = view.net_ids().len();
    assert!(rows > 0, "the v2 view has rows");
    assert_eq!(view.movement(), vec![-1; rows].as_slice());
    assert_eq!(view.movement_max(), vec![-1; rows].as_slice());
    core.close();
}

/// The adventure campaign as the bridge loads it.
fn loaded_adventure() -> crpg_data::LoadedCampaign {
    let files = crate::core::authority::collect_campaign(
        &crate::core::authority::RealFs,
        &adventure_root(),
    )
    .expect("collect the adventure campaign");
    let engine = env!("CARGO_PKG_VERSION").parse().expect("engine version");
    crpg_data::load_campaign(&files, &engine).expect("load the adventure campaign")
}

#[test]
fn demo_data_pins_the_strike_range() {
    // K-6: Strike's reach is 1.5 m (98304 raw).
    let loaded = loaded_adventure();
    let mut ranges = Vec::new();
    for document in loaded.documents.values() {
        if let crpg_data::Document::Ability(ability) = document {
            if ability.slug == "strike" {
                ranges.push(ability.range.map(|range| range.to_raw()));
            }
        }
    }
    assert_eq!(ranges, vec![Some(98304)]);
}
