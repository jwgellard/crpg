//! T156a: `open_v4_ai`, the server AI as the non-player driver (A-1..A-5) on
//! the `combat_basic` fixture. Every case but A7 drives a manual clock and
//! `call` rendezvous; A7 is the one wall-clock case (the public wrapper).

use std::rc::Rc;
use std::time::{Duration, Instant};

use crpg_ai::SideId;
use crpg_core::GenerationalArena;

use super::rt_tests::{basic_root, call, settle, ManualClock};
use super::{ai_sides, AiPacer, AI_STEP_DELAY_MS};
use crate::core::client::{SessionView, ViewEvent, ViewReceipt};
use crate::core::{BridgeCode, BridgeCore};

/// The AI's per-step delay (a multiple of 50 under every D78b option).
const D: u64 = AI_STEP_DELAY_MS;

/// The `combat_basic` root as text.
fn basic() -> String {
    basic_root().to_str().expect("fixture path").to_string()
}

/// Opens BASIC as an AI session with `side` as the player's.
fn open_ai(core: &mut BridgeCore, clock: &ManualClock, side: &str) -> BridgeCode {
    core.open_v4_ai_with(&basic(), "", side, 1, clock.clone())
}

/// Opens BASIC as a peer-NPC v4 session with `side` as the player's.
fn open_peer(core: &mut BridgeCore, clock: &ManualClock, side: &str) -> BridgeCode {
    core.open_v4_with(&basic(), "", side, 1, clock.clone())
}

/// `host.executions()` read on the loop thread.
fn executions(core: &BridgeCore) -> u64 {
    call(core, |l| l.hosting().host().executions())
}

/// Every view's events, concatenated in order.
fn events_of(views: &[Rc<SessionView>]) -> Vec<ViewEvent> {
    views
        .iter()
        .flat_map(|view| view.events().iter().cloned())
        .collect()
}

/// One settle at `t`, asserted clean, returning the events its views hold.
fn step(core: &mut BridgeCore, clock: &ManualClock, t: u64) -> Vec<ViewEvent> {
    let (code, views) = settle(core, clock, t);
    assert_eq!(code, BridgeCode::Ok, "settle({t})");
    events_of(&views)
}

/// Every 50 ms from `from` to `to` inclusive, each step's events by time.
fn every_50(
    core: &mut BridgeCore,
    clock: &ManualClock,
    from: u64,
    to: u64,
) -> Vec<(u64, Vec<ViewEvent>)> {
    (from..=to)
        .step_by(50)
        .map(|t| (t, step(core, clock, t)))
        .collect()
}

fn is_action(event: &ViewEvent) -> bool {
    matches!(event, ViewEvent::ActionResolved { .. })
}

fn is_action_by(event: &ViewEvent, who: i64) -> bool {
    matches!(event, ViewEvent::ActionResolved { actor, .. } if *actor == who)
}

fn is_turn_of(event: &ViewEvent, who: i64) -> bool {
    matches!(event, ViewEvent::TurnStarted { actor, .. } if *actor == who)
}

/// The events of the step at `t` in `log`.
fn at(log: &[(u64, Vec<ViewEvent>)], t: u64) -> &[ViewEvent] {
    &log.iter().find(|(time, _)| *time == t).expect("step run").1
}

/// Polls until the runner's end is reported (A6), then returns its code
/// (`authority/v4/tests.rs`'s `poll_until_end`, copied).
fn poll_until_end(core: &mut BridgeCore) -> BridgeCode {
    let started = Instant::now();
    loop {
        let code = core.poll(100);
        if code != BridgeCode::Ok {
            return code;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "ai_session_shutdown_is_host_closed: no poll reported the runner's end within 5 s"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// What a played-out session leaves for A2's comparison.
struct Played {
    events: Vec<ViewEvent>,
    receipts: Vec<ViewReceipt>,
    health: Vec<i64>,
    dead: Vec<u8>,
    round: i64,
    ended: bool,
    executions: u64,
}

/// B12's loop: `settle(600 k)` for `k = 0 ..= 400`, option 0 whenever a new
/// revision has options.
fn play(core: &mut BridgeCore, clock: &ManualClock) -> Played {
    let mut events = Vec::new();
    let mut receipts = Vec::new();
    let mut chosen = -1i64;
    for k in 0..=400u64 {
        let (code, views) = settle(core, clock, 600 * k);
        assert_eq!(code, BridgeCode::Ok);
        for view in &views {
            events.extend_from_slice(view.events());
            receipts.extend_from_slice(view.receipts());
        }
        let view = core.view();
        if view.revision() != chosen && !view.option_labels().is_empty() {
            assert_eq!(core.submit_option(view.revision(), 0), BridgeCode::Ok);
            chosen = view.revision();
        }
    }
    let view = core.view();
    Played {
        events,
        receipts,
        health: view.health().to_vec(),
        dead: view.dead().to_vec(),
        round: view.round(),
        ended: view.ended(),
        executions: executions(core),
    }
}

// ---------------------------------------------------------------------------
// A1 the AI plays the Goblin on the server (A-1, A-3, A-4).
// ---------------------------------------------------------------------------

#[test]
fn ai_plays_the_goblin_on_the_server() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_ai(&mut core, &clock, ""), BridgeCode::Ok);

    // Nothing before the delay.
    let log = every_50(&mut core, &clock, 0, D - 50);
    for (t, events) in &log {
        assert!(
            !events.iter().any(is_action),
            "an action in settle({t}) before the delay"
        );
    }

    // One Strike at D, from the Goblin (2) on the Hero (1); no host command.
    let at_d = step(&mut core, &clock, D);
    let actions: Vec<&ViewEvent> = at_d.iter().filter(|e| is_action(e)).collect();
    assert_eq!(actions.len(), 1, "exactly one action in settle(D)");
    match actions[0] {
        ViewEvent::ActionResolved {
            actor,
            target,
            ability,
            ..
        } => {
            assert_eq!(*actor, 2);
            assert_eq!(*target, 1);
            assert_eq!(ability, "Strike");
        }
        other => panic!("not an action: {other:?}"),
    }
    assert_eq!(executions(&core), 0);

    // The Strike ends the Goblin's turn (`ends_turn` in the fixture's
    // ruleset), so the Hero's turn starts in the same cycle and the Hero's
    // options show; the AI then has no second action to take.
    assert_eq!(
        at_d.iter().filter(|e| is_turn_of(e, 1)).count(),
        1,
        "the Hero's TurnStarted in settle(D)"
    );
    let log = every_50(&mut core, &clock, D + 50, 2 * D + 100);
    for (t, events) in &log {
        assert!(!events.iter().any(is_action), "a second action in {t}");
        assert!(
            !events.iter().any(|e| is_turn_of(e, 1) || is_turn_of(e, 2)),
            "a turn change in settle({t})"
        );
    }
    assert_eq!(
        core.view().option_labels(),
        ["Strike -> Goblin".to_string(), "End turn".to_string()]
    );
    assert_eq!(executions(&core), 0);

    // Control: the peer NPC acts at its fixed 600 ms, through the player's
    // peer-command path, so `executions` moves.
    let mut peer = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_peer(&mut peer, &clock, ""), BridgeCode::Ok);
    let log = every_50(&mut peer, &clock, 0, 650);
    for (t, events) in &log {
        if *t < 650 {
            assert!(
                !events.iter().any(is_action),
                "the peer NPC acted in settle({t})"
            );
        }
    }
    assert_eq!(
        at(&log, 650).iter().filter(|e| is_action_by(e, 2)).count(),
        1
    );
    assert_eq!(executions(&peer), 1);
}

// ---------------------------------------------------------------------------
// A2 the AI playthrough equals the peer NPC's (A-4, A-5).
// ---------------------------------------------------------------------------

#[test]
fn ai_playthrough_equals_the_peer_npc() {
    let mut ai = BridgeCore::new();
    let ai_clock = ManualClock::new();
    assert_eq!(open_ai(&mut ai, &ai_clock, ""), BridgeCode::Ok);
    let ai_played = play(&mut ai, &ai_clock);

    let mut peer = BridgeCore::new();
    let peer_clock = ManualClock::new();
    assert_eq!(open_peer(&mut peer, &peer_clock, ""), BridgeCode::Ok);
    let peer_played = play(&mut peer, &peer_clock);

    assert!(ai_played.ended, "the AI fight ends");
    assert!(peer_played.ended, "the peer fight ends");
    assert!(!ai_played.events.is_empty());
    assert_eq!(ai_played.events, peer_played.events);
    let ai_flags: Vec<bool> = ai_played.receipts.iter().map(|r| r.applied).collect();
    let peer_flags: Vec<bool> = peer_played.receipts.iter().map(|r| r.applied).collect();
    assert!(!ai_flags.is_empty());
    assert_eq!(ai_flags, peer_flags);
    assert_eq!(ai_played.health, peer_played.health);
    assert_eq!(ai_played.dead, peer_played.dead);
    assert_eq!(ai_played.round, peer_played.round);
    assert_eq!(ai_played.ended, peer_played.ended);
    // The AI's actions are host operations; only the player's commands count.
    assert!(
        ai_played.executions < peer_played.executions,
        "AI {} vs peer {}",
        ai_played.executions,
        peer_played.executions
    );
}

// ---------------------------------------------------------------------------
// A3 the AI never plays the player (A-1, A-3).
// ---------------------------------------------------------------------------

#[test]
fn ai_never_plays_the_player() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    // The player is the Goblin (opens); the AI controls the Hero.
    assert_eq!(open_ai(&mut core, &clock, "Goblin"), BridgeCode::Ok);

    let log = every_50(&mut core, &clock, 0, 1000);
    for (t, events) in &log {
        assert!(
            !events.iter().any(is_action),
            "the AI acted in settle({t}) on the player's turn"
        );
        if *t == 100 {
            assert_eq!(
                core.view().option_labels(),
                ["Strike -> Hero".to_string(), "End turn".to_string()]
            );
        }
    }

    // The player's Strike (which ends the Goblin's turn) is the one command.
    assert_eq!(
        core.submit_option(core.view().revision(), 0),
        BridgeCode::Ok
    );
    let mut t = 1000u64;
    let turn_at = loop {
        t += 50;
        assert!(t < 3000, "the Hero's turn never started");
        let events = step(&mut core, &clock, t);
        assert!(
            !events.iter().any(|e| is_action_by(e, 1)),
            "the Hero acted in settle({t}) before its turn"
        );
        if events.iter().any(|e| is_turn_of(e, 1)) {
            break t;
        }
    };

    // The AI's turn starts at T; its Strike waits the delay.
    let log = every_50(&mut core, &clock, turn_at + 50, turn_at + D);
    for (time, events) in &log {
        if *time < turn_at + D {
            assert!(
                !events.iter().any(|e| is_action_by(e, 1)),
                "the Hero acted in settle({time}), before T + D"
            );
        }
    }
    let hero_actions: Vec<&ViewEvent> = at(&log, turn_at + D)
        .iter()
        .filter(|e| is_action_by(e, 1))
        .collect();
    assert_eq!(hero_actions.len(), 1, "one Hero action in settle(T + D)");
    match hero_actions[0] {
        ViewEvent::ActionResolved { target, .. } => assert_eq!(*target, 2),
        other => panic!("not an action: {other:?}"),
    }
    // Only the player's Strike was a command.
    assert_eq!(executions(&core), 1);
}

// ---------------------------------------------------------------------------
// A4 `ai_sides` numbers side strings (A-2).
// ---------------------------------------------------------------------------

#[test]
fn ai_sides_numbers_side_strings() {
    let mut arena = GenerationalArena::<()>::new();
    let e0 = arena.insert(());
    let e1 = arena.insert(());
    let e2 = arena.insert(());
    let e3 = arena.insert(());

    let sides = ai_sides(&[
        (e0, "Hero".to_string()),
        (e1, "Goblin".to_string()),
        (e2, "Goblin".to_string()),
        (e3, "Ally".to_string()),
    ])
    .expect("distinct entities");
    assert_eq!(
        sides.entries(),
        [
            (e0, SideId(2)),
            (e1, SideId(1)),
            (e2, SideId(1)),
            (e3, SideId(0)),
        ]
    );
    assert!(sides.hostile(e0, e1));
    assert!(!sides.hostile(e1, e2));

    let empty = ai_sides(&[]).expect("no rows");
    assert!(empty.entries().is_empty());

    let fault =
        ai_sides(&[(e0, "A".to_string()), (e0, "B".to_string())]).expect_err("a duplicate entity");
    assert_eq!(fault.code, BridgeCode::StartFailed);
    assert!(
        fault.detail.starts_with("Duplicate at sides/"),
        "detail: {}",
        fault.detail
    );
}

// ---------------------------------------------------------------------------
// A5 the AI hook drains the NPC delivery (A-4 (2)).
// ---------------------------------------------------------------------------

#[test]
fn ai_hook_drains_the_npc_delivery() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_ai(&mut core, &clock, "Goblin"), BridgeCode::Ok);
    assert_eq!(settle(&mut core, &clock, 0).0, BridgeCode::Ok);
    let (_player, npc) = core.v4_peers().expect("live v4");
    let revision = core.view().revision();
    assert_eq!(core.submit_option(revision, 0), BridgeCode::Ok);
    assert_eq!(settle(&mut core, &clock, 50).0, BridgeCode::Ok);
    assert_eq!(
        call(&core, move |l| l
            .hosting_mut()
            .host_mut()
            .take_delivery_entries(npc)
            .unwrap()
            .len()),
        0
    );

    // Control: with the hook cleared the NPC's delivery is not drained.
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_ai(&mut core, &clock, "Goblin"), BridgeCode::Ok);
    assert_eq!(settle(&mut core, &clock, 0).0, BridgeCode::Ok);
    call(&core, |l| l.hosting_mut().set_ai(None));
    let (_player, npc) = core.v4_peers().expect("live v4");
    let revision = core.view().revision();
    assert_eq!(core.submit_option(revision, 0), BridgeCode::Ok);
    assert_eq!(settle(&mut core, &clock, 50).0, BridgeCode::Ok);
    assert!(
        call(&core, move |l| l
            .hosting_mut()
            .host_mut()
            .take_delivery_entries(npc)
            .unwrap()
            .len())
            > 0
    );
}

// ---------------------------------------------------------------------------
// A6 a shut-down host ends the AI session as HOST_CLOSED (A-4 (1)).
// ---------------------------------------------------------------------------

#[test]
fn ai_session_shutdown_is_host_closed() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_ai(&mut core, &clock, "Goblin"), BridgeCode::Ok);
    assert_eq!(settle(&mut core, &clock, 0).0, BridgeCode::Ok);
    assert_eq!(settle(&mut core, &clock, 50).0, BridgeCode::Ok);
    call(&core, |l| l.hosting_mut().host_mut().shutdown());
    clock.set(100);
    assert_eq!(poll_until_end(&mut core), BridgeCode::HostClosed);
    assert_eq!(core.last_error(), "HOST_CLOSED: Closed at host/admission");
    assert!(core.runner().is_none());
}

// ---------------------------------------------------------------------------
// A7 the public wrapper opens an AI session (A-1).
// ---------------------------------------------------------------------------

/// Polls on the wall clock until the Goblin's action arrives (at most 5 s),
/// returning the host's `executions()` at that point.
fn executions_at_goblin_action(core: &mut BridgeCore) -> u64 {
    let started = Instant::now();
    let mut previous = core.view().revision();
    loop {
        let now = started.elapsed().as_millis() as u64;
        assert_eq!(core.poll(now), BridgeCode::Ok);
        let view = core.view();
        if view.revision() != previous {
            previous = view.revision();
            if view.events().iter().any(|e| is_action_by(e, 2)) {
                return executions(core);
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "no Goblin action within 5 s"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn public_open_v4_ai_uses_the_server_ai() {
    let mut core = BridgeCore::new();
    assert_eq!(core.open_v4_ai(&basic(), "", "", 1), BridgeCode::Ok);
    assert_eq!(executions_at_goblin_action(&mut core), 0);
    core.close();

    // Control: the public `open_v4` plays the Goblin through the peer NPC.
    let mut peer = BridgeCore::new();
    assert_eq!(peer.open_v4(&basic(), "", "", 1), BridgeCode::Ok);
    assert_eq!(executions_at_goblin_action(&mut peer), 1);
    peer.close();
}

// ---------------------------------------------------------------------------
// A8 the AI open checks in `open_v4`'s order (A-1).
// ---------------------------------------------------------------------------

#[test]
fn ai_open_precedence() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(
        core.open_v4_ai_with(&basic(), "", "", -1, clock.clone()),
        BridgeCode::BadArgument
    );
    assert_eq!(core.last_error(), "BAD_ARGUMENT: bad argument");
    assert!(core.runner().is_none());
    assert_eq!(
        core.open_v4_ai_with(&basic(), "nope", "", 1, clock.clone()),
        BridgeCode::BadArgument
    );
    assert_eq!(core.last_error(), "BAD_ARGUMENT: bad argument");
    assert!(core.runner().is_none());
    assert_eq!(
        core.open_v4_ai_with(&basic(), "", "Nobody", 1, clock.clone()),
        BridgeCode::UnknownSide
    );
    assert!(core.runner().is_none());

    assert_eq!(open_ai(&mut core, &clock, ""), BridgeCode::Ok);
    assert_eq!(
        open_ai(&mut core, &clock, ""),
        BridgeCode::AlreadyOpen,
        "a second AI open"
    );
    assert_eq!(core.last_error(), "ALREADY_OPEN: session already open");
    assert_eq!(
        open_peer(&mut core, &clock, ""),
        BridgeCode::AlreadyOpen,
        "a peer open over an AI session"
    );
    assert_eq!(core.last_error(), "ALREADY_OPEN: session already open");
    assert_eq!(settle(&mut core, &clock, 50).0, BridgeCode::Ok);
    assert!(core.runner().is_some(), "the live session survived");

    core.close();
    assert_eq!(open_ai(&mut core, &clock, ""), BridgeCode::Ok);
    core.close();
}

// ---------------------------------------------------------------------------
// A9 the pacer's arm, fire, disarm and saturation rules (A-3).
// ---------------------------------------------------------------------------

#[test]
fn ai_pacer_steps() {
    let mut pacer = AiPacer::default();
    assert!(!pacer.step(0, true), "arms at 0");
    assert!(!pacer.step(D - 1, true), "not yet due");
    assert!(pacer.step(D, true), "fires when due");
    assert!(!pacer.step(D, true), "fire disarmed: re-arms at D");
    assert!(!pacer.step(2 * D - 1, true));
    assert!(pacer.step(2 * D, true), "fires one delay after re-arming");
    assert!(!pacer.step(2 * D + 1, false), "not the AI's turn disarms");
    assert!(!pacer.step(2 * D + 2, true), "re-arms at 2D + 2");
    assert!(!pacer.step(3 * D + 1, true));
    assert!(pacer.step(3 * D + 2, true));

    let mut saturated = AiPacer::default();
    assert!(!saturated.step(u64::MAX - 1, true), "arms at u64::MAX");
    assert!(saturated.step(u64::MAX, true), "due is saturated");
}
