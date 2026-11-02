//! K15-K30 and the open half of K35: the bridge lifecycle, the campaign
//! errors, the revision/ack rules, the NPC driver and the panic fence.

use std::collections::BTreeMap;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crpg_core::{EntityId, Ulid};
use crpg_data::Document;
use crpg_net::codec_v2::encode_intent;
use crpg_net::protocol::{IntentBody, IntentFrame, NetId, LANE_COMBAT, SEQ_FIRST};
use crpg_net::transport::Transport;
use crpg_server::host::PeerHandle;
use crpg_sim::{CombatAction, HistoryWorld};

use super::authority::{collect_campaign, first_encounter, Presentation, RealFs, SpecParts};
use super::{BridgeCore, Live, Probe, State};
use crate::core::errors::BridgeCode;
use crate::core::link::{LocalLink, LINK_FRAMES};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("campaigns")
        .join("fixtures")
        .join("combat_basic")
}

fn live(core: &BridgeCore) -> Option<&Live> {
    match &core.state {
        State::Open(live) => Some(live),
        State::Closed | State::Fenced(_) => None,
    }
}

fn live_mut(core: &mut BridgeCore) -> Option<&mut Live> {
    match &mut core.state {
        State::Open(live) => Some(live),
        State::Closed | State::Fenced(_) => None,
    }
}

fn is_fenced(core: &BridgeCore, code: BridgeCode) -> bool {
    matches!(core.state, State::Fenced(fenced) if fenced == code)
}

/// A copied campaign tree removed on drop.
struct TempCampaign {
    root: PathBuf,
}

impl TempCampaign {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "crpg-godot-t047-{}-{unique}-{label}",
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

    fn text(&self) -> String {
        self.root.to_str().expect("temp path is UTF-8").to_string()
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

fn open_fixture(core: &mut BridgeCore) -> LocalLink {
    let link = LocalLink::new(LINK_FRAMES);
    let dir = fixture_root();
    let code = core.open_on(dir.to_str().expect("fixture path"), "", "", 1, &link);
    assert_eq!(code, BridgeCode::Ok);
    link
}

fn open_and_poll(core: &mut BridgeCore) -> LocalLink {
    let link = open_fixture(core);
    assert_eq!(core.poll(0), BridgeCode::Ok);
    link
}

fn open_side(core: &mut BridgeCore, side: &str) -> LocalLink {
    let link = LocalLink::new(LINK_FRAMES);
    let dir = fixture_root();
    let code = core.open_on(dir.to_str().expect("fixture path"), "", side, 1, &link);
    assert_eq!(code, BridgeCode::Ok);
    link
}

fn open_and_poll_side(core: &mut BridgeCore, side: &str) -> LocalLink {
    let link = open_side(core, side);
    assert_eq!(core.poll(0), BridgeCode::Ok);
    link
}

fn id(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}

fn ull(raw: u128) -> Ulid {
    Ulid::from_u128(raw)
}

/// K15 / B11, B12: the first poll's exact view, and each placement is the one
/// the player's replica map names.
#[test]
fn fixture_view_is_exact_and_rows_use_the_player_replica_map() {
    let mut core = BridgeCore::new();
    let _link = open_fixture(&mut core);
    assert_eq!(core.poll(0), BridgeCode::Ok);
    let view = core.view();
    assert_eq!(view.revision(), 1);
    assert_eq!(view.net_ids(), &[1, 2]);
    assert_eq!(view.names(), &["Hero".to_string(), "Goblin".to_string()]);
    assert_eq!(view.sides(), &["Hero".to_string(), "Goblin".to_string()]);
    assert_eq!(
        view.placements(),
        &[ull(14).to_string(), ull(15).to_string()]
    );
    assert_eq!(view.health_known(), &[1, 1]);
    // The goblin's initiative 9 sorts before the hero's 10, so the goblin is
    // the opening actor (the CLI adapter records the same opening turn).
    assert_eq!(view.active(), 2);
    assert_eq!(view.round(), 0);
    assert!(view.events().is_empty());
    // The default player side is the first combatant's (Hero), which is not
    // the active actor, so it has no options on this poll.
    assert!(view.option_labels().is_empty());
    drop(view);

    let live = live(&core).expect("open");
    let host = live.authority.host();
    let player = live.authority.player();
    assert_ne!(player, live.authority.npc());
    for (entity, placement) in live.authority.bound() {
        let replica = host
            .replica_id(player, *entity)
            .expect("host")
            .expect("mapped");
        let row = core
            .view()
            .net_ids()
            .iter()
            .position(|net| *net == i64::try_from(replica.get()).expect("fits"))
            .expect("row");
        assert_eq!(core.view().placements()[row], placement.to_string());
    }
}

/// K16 / B1: every open failure code, the two precedence pairs, and the fact
/// that no failure leaves a `Live`.
#[test]
fn open_failure_codes_and_precedence_leave_no_session() {
    let mut missing = BridgeCore::new();
    assert_eq!(
        missing.open("definitely/missing", "", "", 1),
        BridgeCode::CampaignIo
    );
    assert!(live(&missing).is_none());

    let truncated = TempCampaign::new("truncated");
    fs::write(
        truncated.path().join("encounters").join("first-blood.json"),
        b"{",
    )
    .expect("truncate");
    assert_eq!(
        BridgeCore::new().open(&truncated.text(), "", "", 1),
        BridgeCode::CampaignInvalid
    );

    let no_encounters = TempCampaign::new("no-encounters");
    fs::remove_dir_all(no_encounters.path().join("encounters")).expect("remove encounters");
    let code = BridgeCore::new().open(&no_encounters.text(), "", "", 1);
    assert_eq!(code, BridgeCode::NoEncounter);

    let mut core = BridgeCore::new();
    let dir = fixture_root();
    let dir = dir.to_str().expect("fixture path");
    assert_eq!(
        core.open(dir, "01ARZ3NDEKTSV4RRFFQ69G5FAV", "", 1),
        BridgeCode::UnknownEncounter
    );
    assert!(live(&core).is_none());
    assert_eq!(core.open(dir, "", "Nobody", 1), BridgeCode::UnknownSide);
    assert!(live(&core).is_none());
    assert_eq!(core.open(dir, "", "", -1), BridgeCode::BadArgument);
    assert!(live(&core).is_none());
    assert_eq!(core.open(dir, "xyz", "", 1), BridgeCode::BadArgument);
    assert!(live(&core).is_none());

    assert_eq!(core.open(dir, "", "", 1), BridgeCode::Ok);
    assert_eq!(core.open(dir, "", "", 1), BridgeCode::AlreadyOpen);
    // Argument checks precede the state check.
    assert_eq!(core.open(dir, "", "", -1), BridgeCode::BadArgument);
    // The state check precedes the walk.
    assert_eq!(core.open("missing", "", "", 1), BridgeCode::AlreadyOpen);
    assert!(live(&core).is_some());

    // Fence the session, then the state check still beats a missing dir.
    let link = LocalLink::new(LINK_FRAMES);
    let mut fenced = BridgeCore::new();
    assert_eq!(fenced.open_on(dir, "", "", 1, &link), BridgeCode::Ok);
    assert_eq!(fenced.poll(0), BridgeCode::Ok);
    link.authority_end().send(&[0xff]).expect("room");
    assert_eq!(fenced.poll(1), BridgeCode::FrameUndecodable);
    assert_eq!(fenced.open("missing", "", "", 1), BridgeCode::AlreadyOpen);
    assert!(live(&fenced).is_none());

    // A valid open still works after every failure.
    let mut fresh = BridgeCore::new();
    assert_eq!(fresh.open(dir, "", "", 1), BridgeCode::Ok);
}

/// K17 / B8: one NPC ingest per poll, and the first action is exactly 600 ms
/// after its options appear.
#[test]
fn npc_fires_once_at_the_delay_and_never_twice_in_a_poll() {
    let mut core = BridgeCore::new();
    let _link = open_and_poll_side(&mut core, "Goblin");
    let view = core.view();
    let end_turn = view
        .option_labels()
        .iter()
        .position(|label| label == "End turn")
        .expect("end turn offered");
    assert_eq!(
        core.submit_option(view.revision(), end_turn as i64),
        BridgeCode::Ok
    );
    assert_eq!(core.poll(10), BridgeCode::Ok);
    assert_eq!(live(&core).expect("open").authority.host().executions(), 1);
    assert_eq!(core.poll(20), BridgeCode::Ok);
    assert_eq!(core.poll(619), BridgeCode::Ok);
    assert_eq!(live(&core).expect("open").authority.host().executions(), 1);
    assert_eq!(core.poll(620), BridgeCode::Ok);
    assert_eq!(live(&core).expect("open").authority.host().executions(), 2);

    let mut submitted = live(&core).expect("open").authority.npc_submitted();
    for step in 1..=40u64 {
        let now = 1000 + step * 1000;
        let view = core.view();
        if !view.ended() {
            if let Some(index) = view
                .option_labels()
                .iter()
                .position(|label| label == "End turn")
            {
                let revision = view.revision();
                drop(view);
                assert_eq!(core.submit_option(revision, index as i64), BridgeCode::Ok);
            }
        }
        assert_eq!(core.poll(now), BridgeCode::Ok);
        let after = live(&core).expect("open").authority.npc_submitted();
        assert!(after - submitted <= 1, "at most one NPC ingest per poll");
        submitted = after;
    }

    // Control: the player never ends its turn, so the NPC never gets one.
    let mut idle = BridgeCore::new();
    let _idle_link = open_and_poll_side(&mut idle, "Goblin");
    assert_eq!(idle.poll(620), BridgeCode::Ok);
    assert_eq!(live(&idle).expect("open").authority.host().executions(), 0);
}

/// K18 / B7: an idle poll changes nothing; one End turn changes hash,
/// executions and revision.
#[test]
fn idle_poll_changes_nothing() {
    let mut core = BridgeCore::new();
    let _link = open_and_poll_side(&mut core, "Goblin");
    let hash = live(&core).expect("open").authority.host().authority_hash();
    let executions = live(&core).expect("open").authority.host().executions();
    let revision = core.view().revision();
    assert_eq!(core.poll(5), BridgeCode::Ok);
    assert_eq!(
        live(&core).expect("open").authority.host().authority_hash(),
        hash
    );
    assert_eq!(
        live(&core).expect("open").authority.host().executions(),
        executions
    );
    assert_eq!(core.view().revision(), revision);

    let view = core.view();
    let index = view
        .option_labels()
        .iter()
        .position(|label| label == "End turn")
        .expect("end turn");
    assert_eq!(
        core.submit_option(view.revision(), index as i64),
        BridgeCode::Ok
    );
    assert_eq!(core.poll(10), BridgeCode::Ok);
    assert_ne!(
        live(&core).expect("open").authority.host().authority_hash(),
        hash
    );
    assert_eq!(
        live(&core).expect("open").authority.host().executions(),
        executions + 1
    );
    assert_eq!(core.view().revision(), revision + 1);
}

/// K19 / B4: a full link stops the hand-off; only accepted frames are
/// acknowledged, so the next frame arrives after the reader drains.
#[test]
fn a_full_link_defers_the_next_frame_until_it_is_read() {
    let link = LocalLink::new(1);
    let dir = fixture_root();
    let dir = dir.to_str().expect("fixture path");
    let mut core = BridgeCore::new();
    assert_eq!(core.open_on(dir, "", "Goblin", 1, &link), BridgeCode::Ok);
    let mut client = link.client_end();

    {
        let live = live_mut(&mut core).expect("open");
        live.authority.step(0).expect("step");
    }
    assert_eq!(link.queued().1, 1);

    let (epoch, actor) = {
        let live = live(&core).expect("open");
        let host = live.authority.host();
        let player = live.authority.player();
        let options = host.legal_intents(player).expect("options");
        let actor = options
            .iter()
            .find(|option| matches!(option.body, IntentBody::EndTurn))
            .expect("end turn")
            .actor;
        (host.epoch(player).expect("epoch"), actor)
    };
    let frame = encode_intent(&IntentFrame {
        epoch,
        lane: LANE_COMBAT,
        seq: SEQ_FIRST,
        observed_tick: 0,
        actor,
        body: IntentBody::EndTurn,
    })
    .expect("intent");
    client.send(&frame).expect("up room");

    {
        let live = live_mut(&mut core).expect("open");
        live.authority.step(1).expect("step");
    }
    assert_eq!(link.queued().1, 1, "frame 2 is refused while frame 1 waits");
    let bytes = client.recv().expect("frame 1");
    assert_eq!(
        crpg_net::codec_v2::decode_delta(&bytes)
            .expect("frame")
            .event_seq,
        1
    );

    {
        let live = live_mut(&mut core).expect("open");
        live.authority.step(2).expect("step");
    }
    assert_eq!(link.queued().1, 1);
    let bytes = client.recv().expect("frame 2");
    assert_eq!(
        crpg_net::codec_v2::decode_delta(&bytes)
            .expect("frame")
            .event_seq,
        2
    );
    {
        let live = live_mut(&mut core).expect("open");
        live.authority.step(3).expect("step");
    }
    assert_eq!(link.queued().1, 0);

    // Control: a deeper link queues both frames by the second step.
    let deep = LocalLink::new(LINK_FRAMES);
    let mut deep_core = BridgeCore::new();
    assert_eq!(
        deep_core.open_on(dir, "", "Goblin", 1, &deep),
        BridgeCode::Ok
    );
    {
        let live = live_mut(&mut deep_core).expect("open");
        live.authority.step(0).expect("step");
    }
    {
        let live = live(&deep_core).expect("open");
        let host = live.authority.host();
        let player = live.authority.player();
        let actor = host
            .legal_intents(player)
            .expect("options")
            .iter()
            .find(|option| matches!(option.body, IntentBody::EndTurn))
            .expect("end turn")
            .actor;
        let frame = encode_intent(&IntentFrame {
            epoch: host.epoch(player).expect("epoch"),
            lane: LANE_COMBAT,
            seq: SEQ_FIRST,
            observed_tick: 0,
            actor,
            body: IntentBody::EndTurn,
        })
        .expect("intent");
        deep.client_end().send(&frame).expect("up room");
    }
    {
        let live = live_mut(&mut deep_core).expect("open");
        live.authority.step(1).expect("step");
    }
    assert_eq!(deep.queued().1, 2);
}

/// K20 / B3(5): every poll watermarks captures, and the first action creates
/// one.
#[test]
fn captures_are_acknowledged_every_poll() {
    let mut core = BridgeCore::new();
    let _link = open_and_poll_side(&mut core, "Goblin");
    {
        let host = live(&core).expect("open").authority.host();
        assert_eq!(host.capture_acknowledged(), host.last_capture());
        assert_eq!(host.last_capture(), 0);
    }
    let view = core.view();
    let index = view
        .option_labels()
        .iter()
        .position(|label| label == "End turn")
        .expect("end turn");
    assert_eq!(
        core.submit_option(view.revision(), index as i64),
        BridgeCode::Ok
    );
    assert_eq!(core.poll(10), BridgeCode::Ok);
    {
        let host = live(&core).expect("open").authority.host();
        assert_eq!(host.capture_acknowledged(), host.last_capture());
        assert!(host.last_capture() > 0);
    }
    assert_eq!(core.poll(20), BridgeCode::Ok);
    {
        let host = live(&core).expect("open").authority.host();
        assert_eq!(host.capture_acknowledged(), host.last_capture());
    }
}

/// K21 / B3: an independent `HistoryWorld` oracle stays hash-equal through a
/// mixed player/NPC run.
#[test]
fn the_authority_hash_matches_an_independent_oracle() {
    let mut core = BridgeCore::new();
    let _link = open_and_poll_side(&mut core, "Goblin");

    let files = collect_campaign(&RealFs, &fixture_root()).expect("fixture collects");
    let engine = env!("CARGO_PKG_VERSION").parse().expect("engine");
    let loaded = crpg_data::load_campaign(&files, &engine).expect("fixture loads");
    assert!(crpg_data::validate(&loaded).is_empty());
    let encounter_id = first_encounter(&loaded).expect("encounter");
    let encounter = loaded
        .documents
        .values()
        .find_map(|document| match document {
            Document::Encounter(value) if value.id == encounter_id => Some(value),
            _ => None,
        })
        .expect("encounter document");
    let parts = SpecParts::load(&loaded, encounter).expect("spec parts");
    let mut oracle = HistoryWorld::new(1);
    oracle
        .start_encounter(&parts.spec(encounter))
        .expect("oracle starts");
    oracle
        .acknowledge(oracle.last_sequence())
        .expect("start ack");
    let presentation = Presentation::from_loaded(&loaded);

    let reverse: BTreeMap<NetId, EntityId> = {
        let live = live(&core).expect("open");
        let host = live.authority.host();
        let player = live.authority.player();
        live.authority
            .bound()
            .iter()
            .filter_map(|(entity, _)| {
                host.replica_id(player, *entity)
                    .expect("host")
                    .map(|net| (net, *entity))
            })
            .collect()
    };

    for step in 0..40u64 {
        let now = 1000 + step * 1000;
        let mut mirror_player = None;
        let view = core.view();
        if !view.ended() {
            let offered = {
                let live = live(&core).expect("open");
                live.authority
                    .host()
                    .legal_intents(live.authority.player())
                    .expect("options")
            };
            if !offered.is_empty() {
                let revision = view.revision();
                if core.submit_option(revision, 0) == BridgeCode::Ok {
                    mirror_player = Some((offered[0].actor, offered[0].body));
                }
            }
        }
        drop(view);
        let before = live(&core).expect("open").authority.npc_submitted();
        let code = core.poll(now);
        assert!(
            code == BridgeCode::Ok || code == BridgeCode::IntentRefused,
            "poll {step}: {code:?}"
        );

        if let Some((actor, body)) = mirror_player {
            let action = match body {
                IntentBody::DeclareAction { ability, target } => CombatAction::UseAbility {
                    actor: reverse[&actor],
                    ability,
                    target: reverse[&target],
                },
                IntentBody::EndTurn => CombatAction::EndTurn {
                    actor: reverse[&actor],
                },
            };
            if oracle.perform_action(&action).is_ok() {
                oracle
                    .acknowledge(oracle.last_sequence())
                    .expect("player ack");
            }
        }
        if live(&core).expect("open").authority.npc_submitted() > before {
            mirror_npc(&mut oracle, &presentation);
        }
        assert_eq!(
            live(&core).expect("open").authority.host().authority_hash(),
            crpg_sim::history_hash(&oracle),
            "poll {step}"
        );
    }
}

/// Mirrors the NPC's B10 choice on the oracle.
fn mirror_npc(oracle: &mut HistoryWorld, presentation: &Presentation) {
    let Some(active) = oracle.world().combat().and_then(|combat| combat.active) else {
        return;
    };
    let actions = crpg_sim::legal_actions(oracle.world(), active).expect("legal actions");
    let actor_side = oracle
        .world()
        .combatants()
        .get(active)
        .map(|combatant| presentation.combatant(combatant.placement()).1);
    let chosen = actions
        .iter()
        .find(|action| match action {
            CombatAction::UseAbility { target, .. } => oracle
                .world()
                .combatants()
                .get(*target)
                .map(|combatant| presentation.combatant(combatant.placement()).1)
                .is_some_and(|side| Some(side) != actor_side),
            _ => false,
        })
        .or_else(|| actions.first());
    if let Some(action) = chosen {
        if oracle.perform_action(action).is_ok() {
            oracle.acknowledge(oracle.last_sequence()).expect("NPC ack");
        }
    }
}

/// K22 / B14: the client-owned order and the link count after each refusal.
#[test]
fn submit_precedence_and_a_successful_send() {
    let mut core = BridgeCore::new();
    assert_eq!(core.submit_option(0, 0), BridgeCode::NotOpen);

    let link = open_and_poll_side(&mut core, "Goblin");
    let revision = core.view().revision();
    assert_eq!(
        core.submit_option(revision + 1, 0),
        BridgeCode::StaleOptions
    );
    assert_eq!(link.queued().0, 0);
    assert_eq!(core.submit_option(revision, 99), BridgeCode::NoSuchOption);
    assert_eq!(link.queued().0, 0);
    assert_eq!(core.submit_option(revision, 0), BridgeCode::Ok);
    assert_eq!(link.queued().0, 1);
    assert_eq!(core.submit_option(revision, 0), BridgeCode::IntentPending);
    assert_eq!(link.queued().0, 1);
}

/// K22b / B14: each neighbouring pair, in both directions.
#[test]
fn submit_precedence_pairs() {
    let mut closed = BridgeCore::new();
    assert_eq!(closed.submit_option(-1, 0), BridgeCode::BadArgument);
    assert_eq!(closed.submit_option(99, 0), BridgeCode::NotOpen);

    let mut core = BridgeCore::new();
    let link = open_and_poll_side(&mut core, "Goblin");
    let revision = core.view().revision();
    assert_eq!(
        core.submit_option(revision + 1, 99),
        BridgeCode::StaleOptions
    );
    assert_eq!(core.submit_option(revision, 0), BridgeCode::Ok);
    assert_eq!(core.submit_option(revision, 99), BridgeCode::NoSuchOption);
    assert_eq!(link.queued().0, 1);

    let mut fenced = BridgeCore::new();
    let fence_link = open_and_poll(&mut fenced);
    let fenced_revision = fenced.view().revision();
    fence_link.authority_end().send(&[0xff]).expect("room");
    assert_eq!(fenced.poll(1), BridgeCode::FrameUndecodable);
    assert_eq!(fenced.submit_option(-1, 0), BridgeCode::BadArgument);
    assert_eq!(
        fenced.submit_option(fenced_revision, 99),
        BridgeCode::FrameUndecodable
    );
}

/// A crafted intent with the session's epoch, in the player's replica ids.
fn crafted(host: &crpg_server::host::Host, player: PeerHandle, seq: u64, actor: NetId) -> Vec<u8> {
    encode_intent(&IntentFrame {
        epoch: host.epoch(player).expect("epoch"),
        lane: LANE_COMBAT,
        seq,
        observed_tick: host.server_tick(),
        actor,
        body: IntentBody::EndTurn,
    })
    .expect("intent")
}

/// K23 / BR-12: a gameplay rejection is a receipt, not a fence.
#[test]
fn gameplay_rejection_is_a_receipt() {
    let mut core = BridgeCore::new();
    let link = open_and_poll(&mut core);
    let frame = {
        let live = live(&core).expect("open");
        crafted(live.authority.host(), live.authority.player(), 1, id(99))
    };
    link.client_end().send(&frame).expect("up room");
    assert_eq!(core.poll(10), BridgeCode::Ok);
    assert!(live(&core).is_some());
    let view = core.view();
    assert_eq!(
        view.receipts(),
        &[crate::core::ViewReceipt {
            seq: 1,
            applied: false,
            code: 17,
        }]
    );

    // Control: a real option applies.
    let mut valid = BridgeCore::new();
    let valid_link = open_and_poll_side(&mut valid, "Goblin");
    let frame = {
        let live = live(&valid).expect("open");
        let host = live.authority.host();
        let player = live.authority.player();
        let option = host.legal_intents(player).expect("options")[0];
        encode_intent(&IntentFrame {
            epoch: host.epoch(player).expect("epoch"),
            lane: LANE_COMBAT,
            seq: 1,
            observed_tick: host.server_tick(),
            actor: option.actor,
            body: option.body,
        })
        .expect("intent")
    };
    valid_link.client_end().send(&frame).expect("up room");
    assert_eq!(valid.poll(10), BridgeCode::Ok);
    assert_eq!(
        valid.view().receipts(),
        &[crate::core::ViewReceipt {
            seq: 1,
            applied: true,
            code: 0,
        }]
    );
}

/// K24 / B3: the failure budget drops one reply; the poll reports 30 and the
/// other ten receipts still arrive.
#[test]
fn dropped_replies_are_intent_refused_not_fatal() {
    let mut core = BridgeCore::new();
    let link = open_and_poll(&mut core);
    {
        let live = live(&core).expect("open");
        let host = live.authority.host();
        let player = live.authority.player();
        for seq in 1..=11 {
            let frame = crafted(host, player, seq, id(99));
            link.client_end().send(&frame).expect("up room");
        }
    }
    assert_eq!(core.poll(10), BridgeCode::IntentRefused);
    assert!(live(&core).is_some());
    let view = core.view();
    let receipts = view.receipts();
    assert_eq!(receipts.len(), 10);
    for (index, receipt) in receipts.iter().enumerate() {
        assert_eq!(receipt.seq, i64::try_from(index + 1).expect("fits"));
        assert!(!receipt.applied);
        assert_eq!(receipt.code, 17);
    }

    // Control: ten frames fit the budget and report 0.
    let mut ten = BridgeCore::new();
    let ten_link = open_and_poll(&mut ten);
    {
        let live = live(&ten).expect("open");
        let host = live.authority.host();
        let player = live.authority.player();
        for seq in 1..=10 {
            let frame = crafted(host, player, seq, id(99));
            ten_link.client_end().send(&frame).expect("up room");
        }
    }
    assert_eq!(ten.poll(10), BridgeCode::Ok);
    assert_eq!(ten.view().receipts().len(), 10);
}

/// K25 / B3: an ingress refusal is 30, leaves the session open, and does not
/// block the next submit.
#[test]
fn oversized_frames_are_refused_without_fencing() {
    let mut core = BridgeCore::new();
    let link = open_and_poll_side(&mut core, "Goblin");
    link.client_end().send(&vec![0u8; 4097]).expect("room");
    assert_eq!(core.poll(10), BridgeCode::IntentRefused);
    assert!(live(&core).is_some());
    let revision = core.view().revision();
    assert_eq!(core.submit_option(revision, 0), BridgeCode::Ok);
    assert_eq!(link.queued().0, 1);
}

/// K26 / BR-13, B16: a bad downstream frame fences, keeps the view, and only
/// `close` clears it.
#[test]
fn corrupt_frame_fences_and_close_resets() {
    let mut core = BridgeCore::new();
    let link = open_and_poll(&mut core);
    let revision = core.view().revision();
    link.authority_end().send(&[0xff]).expect("room");
    assert_eq!(core.poll(10), BridgeCode::FrameUndecodable);
    assert!(is_fenced(&core, BridgeCode::FrameUndecodable));
    assert_eq!(core.view().revision(), revision);
    assert_eq!(core.poll(20), BridgeCode::FrameUndecodable);
    assert_eq!(
        core.submit_option(revision, 0),
        BridgeCode::FrameUndecodable
    );

    core.close();
    assert_eq!(core.view().revision(), 0);
    assert_eq!(core.poll(30), BridgeCode::NotOpen);
    let dir = fixture_root();
    assert_eq!(
        core.open(dir.to_str().expect("path"), "", "", 1),
        BridgeCode::Ok
    );
}

/// K27 / B16: a regressing clock is a fatal host fault.
#[test]
fn time_regression_fences() {
    let mut core = BridgeCore::new();
    let _link = open_fixture(&mut core);
    assert_eq!(core.poll(100), BridgeCode::Ok);
    assert_eq!(core.poll(50), BridgeCode::HostFailed);
    assert!(is_fenced(&core, BridgeCode::HostFailed));
    assert!(core.last_error().starts_with("HOST_FAILED"));

    let mut monotonic = BridgeCore::new();
    let _mono = open_fixture(&mut monotonic);
    assert_eq!(monotonic.poll(100), BridgeCode::Ok);
    assert_eq!(monotonic.poll(100), BridgeCode::Ok);
}

/// K28 / BR-18: argument checks run before any `Live` code or probe.
#[test]
fn argument_checks_run_before_the_session() {
    let mut core = BridgeCore::new();
    let link = LocalLink::new(LINK_FRAMES);
    assert_eq!(
        core.open_on("missing", "", "", -1, &link),
        BridgeCode::BadArgument
    );
    assert!(live(&core).is_none());
    assert_eq!(core.submit_option(-1, 0), BridgeCode::BadArgument);
    assert_eq!(core.submit_option(0, -1), BridgeCode::BadArgument);
    assert_eq!(link.queued().0, 0);

    let mut probed = BridgeCore::new();
    probed.probe = Some(Probe::Open);
    assert_eq!(
        probed.open_on("missing", "", "", -1, &link),
        BridgeCode::BadArgument
    );
    probed.probe = Some(Probe::Submit);
    assert_eq!(probed.submit_option(-1, 0), BridgeCode::BadArgument);
    assert_eq!(probed.submit_option(0, -1), BridgeCode::BadArgument);

    // Control: a real seed and index work.
    let mut ok = BridgeCore::new();
    ok.probe = None;
    let dir = fixture_root();
    assert_eq!(
        ok.open(dir.to_str().expect("path"), "", "Goblin", 0),
        BridgeCode::Ok
    );
    assert_eq!(ok.poll(0), BridgeCode::Ok);
    let revision = ok.view().revision();
    assert_eq!(ok.submit_option(revision, 0), BridgeCode::Ok);
}

/// K29 / BR-19, B20: every guarded path turns a panic into 55 (or, for
/// `close`, into `Closed` plus the text), and a teardown panic is ignored.
#[test]
fn panics_are_caught_at_every_boundary() {
    let mut polled = BridgeCore::new();
    polled.probe = Some(Probe::Poll);
    assert_eq!(polled.poll(0), BridgeCode::Panicked);
    assert!(is_fenced(&polled, BridgeCode::Panicked));
    assert!(polled.last_error().starts_with("PANICKED"));
    assert!(live(&polled).is_none());
    assert_eq!(polled.poll(1), BridgeCode::Panicked);

    let mut submitted = BridgeCore::new();
    submitted.probe = Some(Probe::Submit);
    assert_eq!(submitted.submit_option(0, 0), BridgeCode::Panicked);
    assert!(is_fenced(&submitted, BridgeCode::Panicked));
    assert!(submitted.last_error().starts_with("PANICKED"));

    let mut opened = BridgeCore::new();
    opened.probe = Some(Probe::Open);
    let dir = fixture_root();
    assert_eq!(
        opened.open(dir.to_str().expect("path"), "", "", 1),
        BridgeCode::Panicked
    );
    assert!(is_fenced(&opened, BridgeCode::Panicked));

    let mut closed = BridgeCore::new();
    let _link = open_and_poll(&mut closed);
    closed.probe = Some(Probe::Close);
    closed.close();
    assert!(matches!(closed.state, State::Closed));
    assert!(closed.last_error().starts_with("PANICKED"));
    assert_eq!(closed.poll(1), BridgeCode::NotOpen);

    // A Live-drop panic during a fatal fence keeps the fault code and text.
    let mut fenced = BridgeCore::new();
    fenced.probe = Some(Probe::LiveDrop);
    let link = open_fixture(&mut fenced);
    assert_eq!(fenced.poll(0), BridgeCode::Ok);
    link.authority_end().send(&[0xff]).expect("room");
    assert_eq!(fenced.poll(1), BridgeCode::FrameUndecodable);
    assert!(is_fenced(&fenced, BridgeCode::FrameUndecodable));
    assert!(fenced.last_error().starts_with("FRAME_UNDECODABLE"));

    // A Live-drop panic in close is reported but does not fence.
    let mut dropped = BridgeCore::new();
    dropped.probe = Some(Probe::LiveDrop);
    let _drop_link = open_fixture(&mut dropped);
    dropped.close();
    assert!(matches!(dropped.state, State::Closed));
    assert!(dropped.last_error().starts_with("PANICKED"));

    // Dropping an open core never escapes a panic.
    let mut dropped_open = BridgeCore::new();
    dropped_open.probe = Some(Probe::LiveDrop);
    let _drop_open_link = open_fixture(&mut dropped_open);
    assert!(catch_unwind(AssertUnwindSafe(|| drop(dropped_open))).is_ok());

    // Control: no probe still polls cleanly.
    let mut clean = BridgeCore::new();
    let _clean = open_and_poll(&mut clean);
    assert_eq!(clean.poll(5), BridgeCode::Ok);
}

/// K30 / B21: `close` is idempotent, keeps the last error, and resets the
/// view; the next `poll` is `NOT_OPEN`.
#[test]
fn close_is_idempotent_and_keeps_the_error_text() {
    let mut core = BridgeCore::new();
    let _link = open_and_poll(&mut core);
    let dir = fixture_root();
    assert_eq!(
        core.open(dir.to_str().expect("path"), "", "", 1),
        BridgeCode::AlreadyOpen
    );
    let revision = core.view().revision();
    assert_eq!(core.submit_option(revision, 99), BridgeCode::NoSuchOption);
    assert!(core.last_error().starts_with("NO_SUCH_OPTION"));
    core.close();
    assert!(core.last_error().starts_with("NO_SUCH_OPTION"));
    core.close();
    assert_eq!(core.view().revision(), 0);
    assert_eq!(core.poll(0), BridgeCode::NotOpen);
    assert!(core.last_error().starts_with("NOT_OPEN"));
}

/// K35 (open half): an encounter naming an absent ruleset is invalid. The
/// finding comes from `validate`, not `load_campaign`.
#[test]
fn an_absent_ruleset_is_campaign_invalid() {
    let temp = TempCampaign::new("absent-ruleset");
    let path = temp.path().join("encounters").join("first-blood.json");
    let text = fs::read_to_string(&path).expect("read encounter");
    let text = text.replace(
        "\"ruleset\": \"0000000000000000000000000J\"",
        "\"ruleset\": \"0000000000000000000000000Z\"",
    );
    fs::write(&path, text).expect("write encounter");
    // `load_campaign` succeeds on a dangling ruleset reference; the semantic
    // `validate` pass is what reports it, so the open result is 21.
    let files = collect_campaign(&RealFs, temp.path()).expect("temp campaign collects");
    let engine = env!("CARGO_PKG_VERSION").parse().expect("engine");
    let loaded = crpg_data::load_campaign(&files, &engine).expect("load passes");
    assert!(
        !crpg_data::validate(&loaded).is_empty(),
        "validate reports it"
    );
    let mut core = BridgeCore::new();
    assert_eq!(
        core.open(&temp.text(), "", "", 1),
        BridgeCode::CampaignInvalid
    );
    assert!(live(&core).is_none());
}
