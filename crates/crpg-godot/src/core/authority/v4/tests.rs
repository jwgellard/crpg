//! T151b: the v4 authority half and `BridgeCore`'s v4 session on T154's run
//! loop (B-B1..B-B9). Every case drives a manual clock and `call` rendezvous,
//! never wall time (T154 R-3).

use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crpg_net::protocol_v3::PauseRights;
use crpg_net::transport::Transport;
use crpg_server::run::{HostLoop, LocalHost};
use crpg_server::runner::{Clock, RunnerGone};

use super::read_options;
use crate::core::client::{SessionView, ViewEvent, ViewMoveReceipt, ViewReceipt};
use crate::core::link::{LocalLink, LINK_FRAMES};
use crate::core::{BridgeCode, BridgeCore, Probe};

// ---------------------------------------------------------------------------
// Test-local helpers.
// ---------------------------------------------------------------------------

/// A manually stepped `Clock`: tests store the time they want the runner to
/// read, so a run-loop step happens exactly at the chosen `now_ms`.
#[derive(Clone)]
struct ManualClock {
    now: Arc<AtomicU64>,
}

impl ManualClock {
    fn new() -> Self {
        Self {
            now: Arc::new(AtomicU64::new(0)),
        }
    }

    fn set(&self, t: u64) {
        self.now.store(t, Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now.load(Ordering::Relaxed)
    }
}

/// The `combat_basic` fixture root.
fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("campaigns")
        .join("fixtures")
        .join("combat_basic")
}

/// A copied campaign tree removed on drop (copied from `core/tests.rs`).
struct TempCampaign {
    root: PathBuf,
}

impl TempCampaign {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "crpg-godot-t151b-{}-{unique}-{label}",
            std::process::id()
        ));
        if root.exists() {
            let _ = fs::remove_dir_all(&root);
        }
        copy_tree(&fixture_root(), &root);
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
    for entry in fs::read_dir(source).expect("read fixture") {
        let entry = entry.expect("fixture entry");
        let from = entry.path();
        let to = target.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to);
        } else {
            fs::copy(&from, &to).expect("copy fixture file");
        }
    }
}

/// The MOVE variant: BASIC plus the T037e movement rules on `minimal-d6`.
fn move_campaign() -> TempCampaign {
    const MOVE_LINE: &str = "  \"movement\": {\"speed\": {\"type\": \"constant\", \"value\": 65536}, \"turn\": {\"pool\": \"0000000000000000000000000P\", \"step\": {\"type\": \"constant\", \"value\": 65536}, \"step_cost\": 1}},";
    let campaign = TempCampaign::new("move");
    let ruleset = campaign.path().join("rulesets").join("minimal-d6.json");
    let text = fs::read_to_string(&ruleset).expect("ruleset");
    let needle = "  \"name\": \"fixture.minimal-d6\",";
    assert_eq!(
        text.matches(needle).count(),
        1,
        "the name line appears once"
    );
    let replaced = text.replacen(needle, &format!("{MOVE_LINE}\n{needle}"), 1);
    fs::write(&ruleset, replaced).expect("write ruleset");
    campaign
}

/// BASIC with `areas/start/area.json`'s `max` made degenerate (B-B13).
fn bad_area_campaign() -> TempCampaign {
    let campaign = TempCampaign::new("bad-area");
    let area = campaign
        .path()
        .join("areas")
        .join("start")
        .join("area.json");
    let content = "{\n  \"ambience\": null,\n  \"bounds\": {\n    \"max\": [\n      0,\n      65536,\n      65536\n    ],\n    \"min\": [\n      0,\n      0,\n      0\n    ]\n  },\n  \"id\": \"0000000000000000000000000D\",\n  \"name\": \"fixture.start\",\n  \"neighbours\": [],\n  \"schema\": \"crpg.area/1\",\n  \"slug\": \"start\"\n}";
    fs::write(&area, content).expect("write area");
    campaign
}

/// BASIC grown to three combatants: two goblins on the `Goblin` side
/// (initiatives 9 and 10, the first acting) and the Hero (11). The first
/// goblin's options therefore open with an attack on its ally (the second
/// goblin) before the attack on the Hero (T151b fix round A, B15).
fn two_goblins_campaign() -> TempCampaign {
    let campaign = TempCampaign::new("two-goblins");
    let placements = campaign
        .path()
        .join("areas")
        .join("start")
        .join("placements.json");
    let content = r#"{
  "area": "0000000000000000000000000D",
  "placements": [
    {
      "id": "0000000000000000000000000E",
      "name": "fixture.hero-spawn",
      "overrides": {},
      "prefab": "0000000000000000000000000G",
      "slug": "hero-spawn",
      "transform": {
        "position": [0, 0, 0],
        "rotation": [0, 0, 0],
        "scale": [65536, 65536, 65536]
      }
    },
    {
      "id": "0000000000000000000000000F",
      "name": "fixture.goblin-spawn",
      "overrides": {},
      "prefab": "0000000000000000000000000H",
      "slug": "goblin-spawn",
      "transform": {
        "position": [0, 0, 0],
        "rotation": [0, 0, 0],
        "scale": [65536, 65536, 65536]
      }
    },
    {
      "id": "0000000000000000000000000Q",
      "name": "fixture.goblin-spawn-b",
      "overrides": {},
      "prefab": "0000000000000000000000000H",
      "slug": "goblin-spawn-b",
      "transform": {
        "position": [0, 0, 0],
        "rotation": [0, 0, 0],
        "scale": [65536, 65536, 65536]
      }
    }
  ],
  "schema": "crpg.placements/1"
}"#;
    fs::write(&placements, content).expect("write placements");

    let encounter = campaign.path().join("encounters").join("first-blood.json");
    let content = r#"{
  "id": "0000000000000000000000000N",
  "name": "fixture.first-blood",
  "participants": [
    {
      "initiative": 9,
      "placement": "0000000000000000000000000F"
    },
    {
      "initiative": 10,
      "placement": "0000000000000000000000000Q"
    },
    {
      "initiative": 11,
      "placement": "0000000000000000000000000E"
    }
  ],
  "ruleset": "0000000000000000000000000J",
  "schema": "crpg.encounter/1",
  "slug": "first-blood"
}"#;
    fs::write(&encounter, content).expect("write encounter");

    // The new placement's locale key, so semantic validation resolves it.
    let locale = campaign.path().join("locale").join("en.json");
    let text = fs::read_to_string(&locale).expect("locale");
    let needle = "    \"fixture.goblin\": \"Goblin\",\n";
    assert_eq!(text.matches(needle).count(), 1, "one goblin locale line");
    let replaced = text.replacen(
        needle,
        "    \"fixture.goblin\": \"Goblin\",\n    \"fixture.goblin-spawn-b\": \"Goblin Spawn B\",\n",
        1,
    );
    fs::write(&locale, replaced).expect("write locale");
    campaign
}

/// Opens BASIC as a v4 session with the clock at 0.
fn open_v4(core: &mut BridgeCore, clock: &ManualClock, side: &str) -> BridgeCode {
    core.open_v4_with(
        fixture_root().to_str().expect("fixture path"),
        "",
        side,
        1,
        clock.clone(),
    )
}

/// Opens BASIC as a v2 session and returns its link.
fn open_v2(core: &mut BridgeCore, side: &str) -> LocalLink {
    let link = LocalLink::new(LINK_FRAMES);
    let dir = fixture_root();
    assert_eq!(
        core.open_on(dir.to_str().expect("fixture path"), "", side, 1, &link),
        BridgeCode::Ok
    );
    link
}

/// `core.runner().unwrap().call(f)`, returning the raw result.
fn call_result<R, F>(core: &BridgeCore, f: F) -> Result<R, RunnerGone>
where
    R: Send + 'static,
    F: FnOnce(&mut HostLoop<LocalHost>) -> R + Send + 'static,
{
    core.runner().expect("a live v4 session").call(f)
}

/// `core.runner().unwrap().call(f).unwrap()`.
fn call<R, F>(core: &BridgeCore, f: F) -> R
where
    R: Send + 'static,
    F: FnOnce(&mut HostLoop<LocalHost>) -> R + Send + 'static,
{
    call_result(core, f).expect("runner alive")
}

/// Runs one between-cycles rendezvous and two polls (B-B4, T154 R-3).
///
/// Returns the first non-zero of the two polls (else 0) and the views after
/// each poll whose revision rose.
fn settle(
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

/// Every view member but `revision`, `events`, `receipts` and
/// `move_receipts`.
fn assert_same_content(a: &SessionView, b: &SessionView) {
    assert_eq!(a.net_ids(), b.net_ids());
    assert_eq!(a.names(), b.names());
    assert_eq!(a.sides(), b.sides());
    assert_eq!(a.placements(), b.placements());
    assert_eq!(a.health(), b.health());
    assert_eq!(a.max_health(), b.max_health());
    assert_eq!(a.health_known(), b.health_known());
    assert_eq!(a.dead(), b.dead());
    assert_eq!(a.active(), b.active());
    assert_eq!(a.round(), b.round());
    assert_eq!(a.ended(), b.ended());
    assert_eq!(a.option_labels(), b.option_labels());
    assert_eq!(a.option_targets(), b.option_targets());
    assert_eq!(a.positions(), b.positions());
    assert_eq!(a.position_known(), b.position_known());
    assert_eq!(a.moving(), b.moving());
}

fn events(views: &[Rc<SessionView>]) -> Vec<ViewEvent> {
    views
        .iter()
        .flat_map(|view| view.events().iter().cloned())
        .collect()
}

fn receipts(views: &[Rc<SessionView>]) -> Vec<ViewReceipt> {
    views
        .iter()
        .flat_map(|view| view.receipts().iter().cloned())
        .collect()
}

fn move_receipts(views: &[Rc<SessionView>]) -> Vec<ViewMoveReceipt> {
    views
        .iter()
        .flat_map(|view| view.move_receipts().iter().cloned())
        .collect()
}

// ---------------------------------------------------------------------------
// B-B9's source scan (the K31 comment stripper, copied).
// ---------------------------------------------------------------------------

fn strip_line_comments(text: &str) -> String {
    let mut kept = String::new();
    for line in text.lines() {
        match line.find("//") {
            Some(index) => kept.push_str(&line[..index]),
            None => kept.push_str(line),
        }
        kept.push('\n');
    }
    kept
}

fn tokens(text: &str) -> Vec<&str> {
    text.split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .filter(|token| !token.is_empty())
        .collect()
}

// ---------------------------------------------------------------------------
// B1 open_v4 binds v4 and the area and runs the loop (B-B1, B-B7).
// ---------------------------------------------------------------------------

#[test]
fn open_v4_binds_v4_and_the_area_and_runs_the_loop() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut core, &clock, ""), BridgeCode::Ok);

    let (p, q) = core.v4_peers().expect("live v4");
    let tuple = call(&core, move |l| {
        let host = l.hosting().host();
        (
            host.session_wire(p),
            host.session_wire(q),
            host.nav_revision().is_some(),
            l.hosting().link_stats(p).is_some(),
        )
    });
    assert_eq!(tuple, (Ok(4), Ok(2), true, true));
    assert!(call(&core, |l| l.cycles()) >= 1, "the loop has run a cycle");

    // Control: BASIC opened v2 binds no mesh and spawns no runner; its
    // player is a v2 session (through the existing Authority test seams).
    let mut v2 = BridgeCore::new();
    let _link = open_v2(&mut v2, "");
    assert!(v2.runner().is_none());
    let live = match &v2.state {
        crate::core::State::Open(live) => live,
        _ => unreachable!("v2 open"),
    };
    assert_eq!(
        live.authority.host().session_wire(live.authority.player()),
        Ok(2)
    );
    assert!(live.authority.host().nav_revision().is_none());
}

// ---------------------------------------------------------------------------
// B2 first settle equals v2 and replies Published (B-A2, B-A3, B-B3, B-B4).
// ---------------------------------------------------------------------------

#[test]
fn first_settle_equals_v2_and_replies_published() {
    let mut v4 = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut v4, &clock, ""), BridgeCode::Ok);
    let (code, _views) = settle(&mut v4, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);

    let mut v2 = BridgeCore::new();
    let _link = open_v2(&mut v2, "");
    assert_eq!(v2.poll(0), BridgeCode::Ok);

    assert_same_content(&v4.view(), &v2.view());
    assert_eq!(v4.view().net_ids(), &[1, 2]);
    assert_eq!(v4.view().active(), 2);
    assert_eq!(v4.view().positions(), &[0; 6]);
    assert_eq!(v4.view().position_known(), &[0, 0]);

    let (p, _q) = v4.v4_peers().expect("live v4");
    assert_eq!(
        call(&v4, move |l| l
            .hosting()
            .host()
            .transfer_state(p)
            .unwrap()
            .unwrap()
            .in_flight),
        Some(1)
    );

    let (code, _views) = settle(&mut v4, &clock, 50);
    assert_eq!(code, BridgeCode::Ok);
    assert_eq!(
        call(&v4, move |l| l
            .hosting()
            .host()
            .transfer_state(p)
            .unwrap()
            .unwrap()
            .in_flight),
        None
    );
}

// ---------------------------------------------------------------------------
// B3 the move fixture anchors both (B-A6).
// ---------------------------------------------------------------------------

#[test]
fn move_fixture_anchors_both() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    let campaign = move_campaign();
    let dir = campaign.path().to_str().expect("temp path is UTF-8");
    assert_eq!(
        core.open_v4_with(dir, "", "Goblin", 1, clock.clone()),
        BridgeCode::Ok
    );
    let (code, _views) = settle(&mut core, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);
    let view = core.view();
    assert_eq!(view.positions(), &[0; 6]);
    assert_eq!(view.position_known(), &[1, 1]);
    assert_eq!(view.moving(), &[0, 0]);
    assert!(!view.option_labels().is_empty());
    drop(view);

    // Control: BASIC under the same open discloses no position.
    let mut basic = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut basic, &clock, "Goblin"), BridgeCode::Ok);
    let (code, _views) = settle(&mut basic, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);
    assert_eq!(basic.view().position_known(), &[0, 0]);
}

// ---------------------------------------------------------------------------
// B4 declare_move round trip (B-B4).
// ---------------------------------------------------------------------------

#[test]
fn declare_move_round_trip() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    let campaign = move_campaign();
    let dir = campaign.path().to_str().expect("temp path is UTF-8");
    assert_eq!(
        core.open_v4_with(dir, "", "Goblin", 1, clock.clone()),
        BridgeCode::Ok
    );
    let (code, _views) = settle(&mut core, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);
    let revision = core.view().revision();

    assert_eq!(core.declare_move(revision, 32768, 0, 32768), BridgeCode::Ok);
    let (code, views) = settle(&mut core, &clock, 50);
    assert_eq!(code, BridgeCode::Ok);
    assert_eq!(
        events(&views),
        vec![ViewEvent::Moved {
            entity: 2,
            points: vec![0, 0, 0, 32768, 0, 32768],
        }]
    );
    assert_eq!(
        receipts(&views),
        vec![ViewReceipt {
            seq: 1,
            applied: true,
            code: 0,
        }]
    );
    let view = core.view();
    assert_eq!(view.positions(), &[0, 0, 0, 32768, 0, 32768]);
    assert_eq!(view.moving(), &[0, 0]);
    drop(view);
    assert_eq!(call(&core, |l| l.hosting().host().server_tick()), 2);

    // A second move spends the pool: rejected IllegalAction (18), no motion.
    let revision = core.view().revision();
    assert_eq!(core.declare_move(revision, 0, 0, 0), BridgeCode::Ok);
    let (code, views) = settle(&mut core, &clock, 100);
    assert_eq!(code, BridgeCode::Ok);
    assert_eq!(
        receipts(&views),
        vec![ViewReceipt {
            seq: 2,
            applied: false,
            code: 18,
        }]
    );
    assert_eq!(core.view().positions(), &[0, 0, 0, 32768, 0, 32768]);
}

// ---------------------------------------------------------------------------
// B5 lane-1 receipts come back (B-B4).
// ---------------------------------------------------------------------------

#[test]
fn lane1_receipts_come_back() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut core, &clock, "Goblin"), BridgeCode::Ok);
    let (code, _views) = settle(&mut core, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);

    assert_eq!(core.move_to(2, 0, 0, 0), BridgeCode::Ok);
    let (code, views) = settle(&mut core, &clock, 50);
    assert_eq!(code, BridgeCode::Ok);
    assert_eq!(
        move_receipts(&views),
        vec![ViewMoveReceipt {
            seq: 1,
            actor: 2,
            outcome: "rejected",
            code: 22,
            processed_tick: 1,
            start_tick: 0,
        }]
    );

    assert_eq!(core.halt(2), BridgeCode::Ok);
    let (code, views) = settle(&mut core, &clock, 100);
    assert_eq!(code, BridgeCode::Ok);
    assert_eq!(
        move_receipts(&views),
        vec![ViewMoveReceipt {
            seq: 2,
            actor: 2,
            outcome: "rejected",
            code: 22,
            processed_tick: 2,
            start_tick: 0,
        }]
    );

    // Control: a move by an actor the player does not control is NotAuthorized.
    assert_eq!(core.move_to(1, 0, 0, 0), BridgeCode::Ok);
    let (code, views) = settle(&mut core, &clock, 150);
    assert_eq!(code, BridgeCode::Ok);
    assert_eq!(
        move_receipts(&views),
        vec![ViewMoveReceipt {
            seq: 3,
            actor: 1,
            outcome: "rejected",
            code: 17,
            processed_tick: 3,
            start_tick: 0,
        }]
    );
}

// ---------------------------------------------------------------------------
// B6 a refused player frame is INTENT_REFUSED (B-B3, B-B4, B-B6).
// ---------------------------------------------------------------------------

#[test]
fn refused_player_frame_is_intent_refused() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut core, &clock, "Goblin"), BridgeCode::Ok);
    let (code, _views) = settle(&mut core, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);

    assert_eq!(
        core.v4_link().expect("live v4").send(&vec![
            0x04u8;
            crpg_net::protocol::MAX_INTENT_FRAME_BYTES + 1
        ]),
        Ok(())
    );
    let (p, _q) = core.v4_peers().expect("live v4");
    let (code, _views) = settle(&mut core, &clock, 50);
    assert_eq!(code, BridgeCode::IntentRefused);
    assert_eq!(core.last_error(), "INTENT_REFUSED: player intent refused");

    assert_eq!(
        call(&core, move |l| l
            .hosting()
            .link_stats(p)
            .map(|s| (s.refused, s.held))),
        Some((1, false))
    );

    let (code, _views) = settle(&mut core, &clock, 100);
    assert_eq!(code, BridgeCode::Ok, "the session stays open");
    assert!(core.runner().is_some());
    assert_eq!(
        call(&core, move |l| l
            .hosting()
            .link_stats(p)
            .map(|s| (s.refused, s.held))),
        Some((1, false))
    );

    // Control: no frame -> the next settle is clean.
    let mut clean = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut clean, &clock, "Goblin"), BridgeCode::Ok);
    assert_eq!(settle(&mut clean, &clock, 0).0, BridgeCode::Ok);
    assert_eq!(settle(&mut clean, &clock, 50).0, BridgeCode::Ok);
}

// ---------------------------------------------------------------------------
// B7 the NPC acts through the hook one cycle after v2 (B-B2).
// ---------------------------------------------------------------------------

#[test]
fn npc_acts_through_the_hook_one_cycle_after_v2() {
    let mut v4 = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut v4, &clock, ""), BridgeCode::Ok);

    let mut v2 = BridgeCore::new();
    let _link = open_v2(&mut v2, "");
    assert_eq!(v2.poll(0), BridgeCode::Ok);
    assert_eq!(v2.poll(550), BridgeCode::Ok);
    assert_eq!(v2.poll(600), BridgeCode::Ok);
    let v2_action = v2.view().events().iter().find_map(|event| match event {
        ViewEvent::ActionResolved { actor, .. } => Some(*actor),
        _ => None,
    });

    let mut v4_actions = Vec::new();
    for t in [0u64, 550, 600, 650] {
        let (code, views) = settle(&mut v4, &clock, t);
        assert_eq!(code, BridgeCode::Ok);
        v4_actions.extend(events(&views).into_iter().filter_map(|event| match event {
            ViewEvent::ActionResolved { actor, .. } => Some(actor),
            _ => None,
        }));
    }
    // No action through 600; exactly one at 650, equal to v2's at 600.
    let v4_through_600 = v4_actions.iter().filter(|actor| **actor == 2).count();
    assert_eq!(v4_actions.len(), 1, "exactly one action by 650");
    assert_eq!(v4_through_600, 1, "the action's actor is the goblin");
    assert_eq!(
        v4_actions.first().copied(),
        v2_action,
        "the same action as v2's poll(600)"
    );
    assert_eq!(v4_actions[0], 2);

    // Control: with the hook cleared, no action appears by settle(1300).
    let mut clear = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut clear, &clock, ""), BridgeCode::Ok);
    assert_eq!(settle(&mut clear, &clock, 0).0, BridgeCode::Ok);
    call(&clear, |l| l.hosting_mut().set_ai(None));
    let mut actions = 0usize;
    for k in 1..=26u64 {
        let (code, views) = settle(&mut clear, &clock, 50 * k);
        assert_eq!(code, BridgeCode::Ok);
        actions += events(&views)
            .iter()
            .filter(|e| matches!(e, ViewEvent::ActionResolved { .. }))
            .count();
    }
    assert_eq!(actions, 0, "no NPC action without the hook");
}

// ---------------------------------------------------------------------------
// B8 the hook drains the NPC's delivery (B-B2).
// ---------------------------------------------------------------------------

#[test]
fn npc_delivery_is_drained() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut core, &clock, "Goblin"), BridgeCode::Ok);
    let (code, _views) = settle(&mut core, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);

    let (_p, q) = core.v4_peers().expect("live v4");
    let revision = core.view().revision();
    assert_eq!(core.submit_option(revision, 0), BridgeCode::Ok);
    assert_eq!(settle(&mut core, &clock, 50).0, BridgeCode::Ok);
    assert_eq!(
        call(&core, move |l| l
            .hosting_mut()
            .host_mut()
            .take_delivery_entries(q)
            .unwrap()
            .len()),
        0
    );

    // Control: with the hook cleared the NPC's delivery is not drained.
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut core, &clock, "Goblin"), BridgeCode::Ok);
    assert_eq!(settle(&mut core, &clock, 0).0, BridgeCode::Ok);
    let (_p, q) = core.v4_peers().expect("live v4");
    call(&core, |l| l.hosting_mut().set_ai(None));
    let revision = core.view().revision();
    assert_eq!(core.submit_option(revision, 0), BridgeCode::Ok);
    assert_eq!(settle(&mut core, &clock, 50).0, BridgeCode::Ok);
    assert!(
        call(&core, move |l| l
            .hosting_mut()
            .host_mut()
            .take_delivery_entries(q)
            .unwrap()
            .len())
            > 0
    );
}

// ---------------------------------------------------------------------------
// B9 the bridge never ticks (B-B9).
// ---------------------------------------------------------------------------

#[test]
fn the_bridge_never_ticks() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("core");
    for file in ["authority/v4.rs", "mod.rs"] {
        let path = root.join(file);
        let text = strip_line_comments(&fs::read_to_string(&path).expect("read source"));
        let found = tokens(&text);
        for banned in [
            "ingest_move",
            "ingest_transfer",
            "take_move_receipts",
            "take_samples",
            "acknowledge_captures",
            "Instant",
            "SystemTime",
        ] {
            assert!(
                !found.contains(&banned),
                "{} names {banned}",
                path.display()
            );
        }
        for banned in [".advance(", ".pump(", "thread::spawn"] {
            assert!(
                !text.contains(banned),
                "{} contains {banned}",
                path.display()
            );
        }
    }

    // Control: the v2 authority half really does pump.
    let authority =
        strip_line_comments(&fs::read_to_string(root.join("authority.rs")).expect("read source"));
    assert!(authority.contains(".pump("));
}

// ---------------------------------------------------------------------------
// B10 server-side endings fence (B-B5).
// ---------------------------------------------------------------------------

#[test]
fn server_side_endings_fence() {
    // (a) the player is unbound -> the link closes.
    {
        let mut core = BridgeCore::new();
        let clock = ManualClock::new();
        assert_eq!(open_v4(&mut core, &clock, "Goblin"), BridgeCode::Ok);
        assert_eq!(settle(&mut core, &clock, 0).0, BridgeCode::Ok);
        assert_eq!(settle(&mut core, &clock, 50).0, BridgeCode::Ok);
        let (p, _q) = core.v4_peers().expect("live v4");
        call(&core, move |l| l.hosting_mut().host_mut().unbind_peer(p));
        clock.set(100);
        call(&core, |_| ());
        assert_eq!(core.poll(100), BridgeCode::HostFailed);
        assert_eq!(core.last_error(), "HOST_FAILED: server closed the link");
        assert!(core.runner().is_none());
        assert_eq!(core.poll(100), BridgeCode::HostFailed);
    }

    // (b) the host shuts down -> the loop ends and the bridge fences
    // `HOST_CLOSED` (50), not a host failure.
    {
        let mut core = BridgeCore::new();
        let clock = ManualClock::new();
        assert_eq!(open_v4(&mut core, &clock, "Goblin"), BridgeCode::Ok);
        assert_eq!(settle(&mut core, &clock, 0).0, BridgeCode::Ok);
        assert_eq!(settle(&mut core, &clock, 50).0, BridgeCode::Ok);
        call(&core, |l| l.hosting_mut().host_mut().shutdown());
        clock.set(100);
        assert!(call_result(&core, |_| ()).is_err());
        let first = core.poll(100);
        assert!(
            matches!(first, BridgeCode::Ok | BridgeCode::HostClosed),
            "the first poll sees the end asynchronously"
        );
        assert_eq!(core.poll(100), BridgeCode::HostClosed);
        assert_eq!(core.last_error(), "HOST_CLOSED: Closed at host/admission");
        assert!(core.runner().is_none());
        assert_eq!(core.poll(100), BridgeCode::HostClosed);
    }

    // (c) a panicking closure on the loop thread fences as 55.
    {
        let mut core = BridgeCore::new();
        let clock = ManualClock::new();
        assert_eq!(open_v4(&mut core, &clock, "Goblin"), BridgeCode::Ok);
        assert_eq!(settle(&mut core, &clock, 0).0, BridgeCode::Ok);
        assert_eq!(settle(&mut core, &clock, 50).0, BridgeCode::Ok);
        assert!(call_result(&core, |_| panic!("boom")).is_err());
        let mut code = core.poll(100);
        if code != BridgeCode::Panicked {
            code = core.poll(100);
        }
        assert_eq!(code, BridgeCode::Panicked);
        assert_eq!(core.last_error(), "PANICKED: server loop panicked");
        assert!(core.runner().is_none());
    }

    // Control: nothing done -> the session stays open.
    {
        let mut core = BridgeCore::new();
        let clock = ManualClock::new();
        assert_eq!(open_v4(&mut core, &clock, "Goblin"), BridgeCode::Ok);
        assert_eq!(settle(&mut core, &clock, 0).0, BridgeCode::Ok);
        assert_eq!(settle(&mut core, &clock, 50).0, BridgeCode::Ok);
        assert_eq!(settle(&mut core, &clock, 100).0, BridgeCode::Ok);
        assert!(core.runner().is_some());
    }
}

// ---------------------------------------------------------------------------
// B11 move entry precedence (B-B8).
// ---------------------------------------------------------------------------

#[test]
fn move_entry_precedence() {
    // A closed core: argument checks win, then NotOpen.
    let mut closed = BridgeCore::new();
    assert_eq!(closed.declare_move(-1, 0, 0, 0), BridgeCode::BadArgument);
    assert_eq!(closed.move_to(0, 0, 0, 0), BridgeCode::BadArgument);
    assert_eq!(
        closed.move_to(1, 536_870_913, 0, 0),
        BridgeCode::BadArgument
    );
    assert_eq!(closed.halt(1), BridgeCode::NotOpen);

    // A v2 session refuses every move with 3.
    let mut v2 = BridgeCore::new();
    let _link = open_v2(&mut v2, "");
    assert_eq!(v2.move_to(1, 0, 0, 0), BridgeCode::BadArgument);
    assert_eq!(v2.last_error(), "BAD_ARGUMENT: not a v4 session");

    // A v4 session with no options has no actor to move.
    let mut v4 = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut v4, &clock, ""), BridgeCode::Ok);
    assert_eq!(settle(&mut v4, &clock, 0).0, BridgeCode::Ok);
    let revision = v4.view().revision();
    assert_eq!(v4.declare_move(revision, 0, 0, 0), BridgeCode::NoSuchOption);

    // A panic at the Submit boundary fences as 55 and ends the runner.
    let mut probed = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut probed, &clock, "Goblin"), BridgeCode::Ok);
    assert_eq!(settle(&mut probed, &clock, 0).0, BridgeCode::Ok);
    let revision = probed.view().revision();
    probed.probe = Some(Probe::Submit);
    assert_eq!(probed.declare_move(revision, 0, 0, 0), BridgeCode::Panicked);
    assert_eq!(probed.poll(50), BridgeCode::Panicked);
    assert!(probed.runner().is_none());
}

// ---------------------------------------------------------------------------
// B12 v4 playthrough equals v2 (B-B4, B-B7).
// ---------------------------------------------------------------------------

#[test]
fn v4_playthrough_equals_v2() {
    let dir = fixture_root();
    let dir = dir.to_str().expect("fixture path");

    // v2 twin.
    let mut v2 = BridgeCore::new();
    let _link = open_v2(&mut v2, "");
    let mut v2_events = Vec::new();
    let mut v2_receipts = Vec::new();
    let mut v2_chosen = -1i64;
    let mut v2_previous = -1i64;
    for k in 0..=200u64 {
        assert_eq!(v2.poll(600 * k), BridgeCode::Ok);
        let view = v2.view();
        if view.revision() != v2_previous {
            v2_events.extend_from_slice(view.events());
            v2_receipts.extend_from_slice(view.receipts());
            v2_previous = view.revision();
        }
        if view.revision() != v2_chosen && !view.option_labels().is_empty() {
            assert_eq!(v2.submit_option(view.revision(), 0), BridgeCode::Ok);
            v2_chosen = view.revision();
        }
    }
    assert!(v2.view().ended());

    // v4 twin.
    let mut v4 = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(
        v4.open_v4_with(dir, "", "", 1, clock.clone()),
        BridgeCode::Ok
    );
    let mut v4_events = Vec::new();
    let mut v4_receipts = Vec::new();
    let mut v4_chosen = -1i64;
    for k in 0..=400u64 {
        let (code, views) = settle(&mut v4, &clock, 600 * k);
        assert_eq!(code, BridgeCode::Ok);
        for view in &views {
            v4_events.extend_from_slice(view.events());
            v4_receipts.extend_from_slice(view.receipts());
        }
        let view = v4.view();
        if view.revision() != v4_chosen && !view.option_labels().is_empty() {
            assert_eq!(v4.submit_option(view.revision(), 0), BridgeCode::Ok);
            v4_chosen = view.revision();
        }
    }
    assert!(v4.view().ended());

    assert_eq!(v4_events, v2_events);
    let v2_flags: Vec<bool> = v2_receipts.iter().map(|r| r.applied).collect();
    let v4_flags: Vec<bool> = v4_receipts.iter().map(|r| r.applied).collect();
    assert_eq!(v4_flags, v2_flags);
    assert_eq!(v4.view().health(), v2.view().health());
    assert_eq!(v4.view().dead(), v2.view().dead());
    assert_eq!(v4.view().round(), v2.view().round());
    assert_eq!(v4.view().ended(), v2.view().ended());
}

// ---------------------------------------------------------------------------
// B13 v4 open failures (B-B1).
// ---------------------------------------------------------------------------

#[test]
fn v4_open_failures() {
    let campaign = bad_area_campaign();
    let dir = campaign.path().to_str().expect("temp path is UTF-8");

    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(
        core.open_v4_with(dir, "", "", 1, clock.clone()),
        BridgeCode::StartFailed
    );
    assert_eq!(core.last_error(), "START_FAILED: NavBake at host/admission");
    assert!(core.runner().is_none());
    assert!(matches!(&core.state, crate::core::State::Closed));

    // Control: the v2 open never bakes the area.
    let mut v2 = BridgeCore::new();
    let link = LocalLink::new(LINK_FRAMES);
    assert_eq!(v2.open_on(dir, "", "", 1, &link), BridgeCode::Ok);
}

// ---------------------------------------------------------------------------
// B14 close ends the loop (B-B9).
// ---------------------------------------------------------------------------

#[test]
fn close_ends_the_loop() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut core, &clock, ""), BridgeCode::Ok);
    assert_eq!(settle(&mut core, &clock, 0).0, BridgeCode::Ok);
    core.close();
    assert!(core.runner().is_none());

    // The same core reopens and reaches a real view.
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut core, &clock, ""), BridgeCode::Ok);
    let (code, _views) = settle(&mut core, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);
    assert!(core.view().revision() >= 1);
}

// ---------------------------------------------------------------------------
// T151b fix round A: the six cases the mutant run (reviews/T151b) left alive.
// ---------------------------------------------------------------------------

/// fa_1 (B04, B-B1, Reading 3): the player binds with the least-privilege
/// pause rights. `pause_rights` is the host's own read of the v4 session.
#[test]
fn fa_1_pause_rights_are_the_least_privilege_default() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut core, &clock, "Goblin"), BridgeCode::Ok);
    let (p, _q) = core.v4_peers().expect("live v4");
    assert_eq!(
        call(&core, move |l| l.hosting().host().pause_rights(p)),
        Ok(PauseRights::default())
    );
    assert_eq!(
        PauseRights::default(),
        PauseRights {
            pause: false,
            unpause: false
        }
    );

    // Control: a v2 session never binds v4 pause rights at all.
    let mut v2 = BridgeCore::new();
    let _link = open_v2(&mut v2, "Goblin");
    assert!(v2.v4_peers().is_none());
}

/// fa_2 (B14, B-B2): the NPC's `IntentFrame` carries the host's tick as its
/// `observed_tick`. The host reads it on admission: zero would be past
/// `POLICY_OBSERVED_TICK_WINDOW` (200 ticks) once the loop has ticked past
/// 200, so the NPC's intent is refused as `StaleTick` and no action lands.
#[test]
fn fa_2_npc_observed_tick_is_the_server_tick() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    // The player is the goblin, so the NPC hero has no options until the
    // goblin ends its turn; the hook cannot fire during the long run-up.
    assert_eq!(open_v4(&mut core, &clock, "Goblin"), BridgeCode::Ok);
    assert_eq!(settle(&mut core, &clock, 0).0, BridgeCode::Ok);

    // One cycle per 50 ms step, so 210 cycles put the host past tick 200.
    for k in 1..=209u64 {
        clock.set(50 * k);
        call(&core, |_| ());
    }
    let (code, _views) = settle(&mut core, &clock, 50 * 210);
    assert_eq!(code, BridgeCode::Ok);
    let tick = call(&core, |l| l.hosting().host().server_tick());
    assert!(
        tick > 200,
        "the host is past the observed-tick window: {tick}"
    );

    // End the goblin's turn; the hero's options appear in that cycle and the
    // pacer fires 600 ms later, when a zero observed tick is stale.
    let view = core.view();
    let end_turn = view
        .option_labels()
        .iter()
        .position(|label| label == "End turn")
        .expect("end turn offered");
    let revision = view.revision();
    drop(view);
    assert_eq!(
        core.submit_option(revision, end_turn as i64),
        BridgeCode::Ok
    );
    for k in 211..=230u64 {
        clock.set(50 * k);
        call(&core, |_| ());
    }
    let (code, views) = settle(&mut core, &clock, 50 * 231);
    assert_eq!(code, BridgeCode::Ok);
    let actions: Vec<(i64, i64)> = events(&views)
        .into_iter()
        .filter_map(|event| match event {
            ViewEvent::ActionResolved { actor, target, .. } => Some((actor, target)),
            _ => None,
        })
        .collect();
    assert_eq!(
        actions.len(),
        1,
        "the hero acts once when observed_tick is the host tick"
    );
    assert_eq!(actions[0].0, 1, "the hero, not the goblin, acted");
}

/// fa_3 (B15, B-B2): `choose_npc` is given the real sides table. On a
/// three-combatant fixture whose first option is an attack on an ally, the
/// empty-table `Some(0)` fallback would attack the ally; the real table
/// picks the first attack on another side.
#[test]
fn fa_3_npc_sides_table_chooses_a_cross_side_attack() {
    let campaign = two_goblins_campaign();
    let dir = campaign.path().to_str().expect("temp path is UTF-8");
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(
        core.open_v4_with(dir, "", "Hero", 1, clock.clone()),
        BridgeCode::Ok
    );
    for t in [0u64, 550, 600, 650] {
        let (code, _views) = settle(&mut core, &clock, t);
        assert_eq!(code, BridgeCode::Ok, "settle at {t}");
    }
    let view = core.view();
    let hero_row = view
        .names()
        .iter()
        .position(|name| name == "Hero")
        .expect("the hero is a row");
    let hero_id = view.net_ids()[hero_row];
    let target = view
        .events()
        .iter()
        .find_map(|event| match event {
            ViewEvent::ActionResolved { target, .. } => Some(*target),
            _ => None,
        })
        .expect("the goblin acted");
    assert_eq!(
        target, hero_id,
        "the sides table makes the goblin attack the hero, not its ally"
    );
}

/// fa_4 (B24, B-B4/B-B5): a read error in the between-cycles reply is
/// `host_fault(&e)`, not the loop-stopped text. The NPC peer has no local
/// link, so `read_options` answers `UnknownPeer` while the loop runs.
#[test]
fn fa_4_read_error_maps_through_host_fault() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut core, &clock, "Goblin"), BridgeCode::Ok);
    assert_eq!(settle(&mut core, &clock, 0).0, BridgeCode::Ok);
    let (_p, q) = core.v4_peers().expect("live v4");
    {
        let crate::core::State::OpenV4(live) = &mut core.state else {
            unreachable!("live v4");
        };
        live.reply = Some(live.runner.submit(move |l| read_options(l, q)));
    }
    call(&core, |_| ());
    let code = core.poll(50);
    assert_eq!(code, BridgeCode::HostFailed);
    assert_eq!(
        core.last_error(),
        "HOST_FAILED: UnknownPeer at host/admission"
    );
    assert!(core.runner().is_none());
    assert_eq!(core.poll(50), BridgeCode::HostFailed);
}

/// fa_5 (B28, B-B9, B14): `close` ends the loop and leaves no runner; a
/// client retained across the close sees no frame the close produced and
/// then ends. Only the `shutdown` call can change the host, and the host is
/// gone with the joined loop, so this is the strongest observable form.
#[test]
fn fa_5_close_ends_the_loop_without_a_live_host() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut core, &clock, ""), BridgeCode::Ok);
    assert_eq!(settle(&mut core, &clock, 0).0, BridgeCode::Ok);

    // Attach the NPC and keep the player's client outside the session, so
    // the close path can be read after the runner and host are gone.
    let (_p, q) = core.v4_peers().expect("live v4");
    let npc_client = call(&core, move |l| {
        l.hosting_mut()
            .attach(q, NonZeroUsize::new(LINK_FRAMES).expect("nonzero"))
            .expect("the NPC attaches")
    });
    let mut player_client = {
        let crate::core::State::OpenV4(live) = &mut core.state else {
            unreachable!("live v4");
        };
        std::mem::replace(&mut live.link, npc_client)
    };
    let quiescent = player_client.recv();
    assert!(quiescent.is_none(), "the client was drained by settle(0)");

    core.close();
    assert!(core.runner().is_none());
    assert_eq!(core.view().revision(), 0);
    assert_eq!(core.poll(50), BridgeCode::NotOpen);
    assert!(player_client.send(&[]).is_err(), "the link ended");
}

/// fa_6 (B31, B-B8): `revision < 0` is the bound, so revision 0 is accepted
/// and reaches the client's own checks (no options yet: 6).
#[test]
fn fa_6_declare_move_accepts_revision_zero() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(open_v4(&mut core, &clock, "Goblin"), BridgeCode::Ok);
    assert_eq!(core.view().revision(), 0);
    assert_eq!(core.declare_move(0, 0, 0, 0), BridgeCode::NoSuchOption);
    assert_eq!(core.last_error(), "NO_SUCH_OPTION: no actor to move");

    // Control: the negative bound still refuses -1 before the session.
    assert_eq!(core.declare_move(-1, 0, 0, 0), BridgeCode::BadArgument);
    assert_eq!(core.last_error(), "BAD_ARGUMENT: negative argument");
}
