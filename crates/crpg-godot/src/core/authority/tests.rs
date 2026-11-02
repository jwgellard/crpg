//! K4-K7, K34 and the direct half of K35: the hand-off, the NPC chooser and
//! pacer, the pump result fold, the campaign walk and the start-order binding.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crpg_core::Ulid;
use crpg_data::Document;
use crpg_net::codec_v2::encode_intent;
use crpg_net::protocol::{
    IntentBody, IntentFrame, NetId, ReceiptStatus, RejectionCode, LANE_COMBAT, SEQ_FIRST,
};
use crpg_net::transport::Transport;
use crpg_server::host::{
    AdmissionResult, ControlGrant, DeliveryEntry, DeliveryKind, DisclosureGrants, Host, HostConfig,
    ProtocolSelection,
};
use crpg_sim::{EntityMeta, HistoryWorld, World};

use super::{
    check_binding, choose_npc, collect_campaign, first_encounter, grants, hand_off, npc_results,
    presentation_tables, ruleset_abilities, Authority, EntryKind, NpcPacer, Presentation, RealFs,
    SpecParts, WalkFs,
};
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

/// Builds the authority half the way `open` does, so the private host can be
/// read after a step (the `BridgeCore` seam cannot expose a mutable host).
fn fixture_authority(link: &LocalLink) -> Authority {
    let files = collect_campaign(&RealFs, &fixture_root()).expect("fixture collects");
    let engine = env!("CARGO_PKG_VERSION").parse().expect("engine");
    let loaded = crpg_data::load_campaign(&files, &engine).expect("fixture loads");
    let encounter = first_encounter(&loaded).expect("encounter");
    let encounter = loaded
        .documents
        .values()
        .find_map(|document| match document {
            Document::Encounter(value) if value.id == encounter => Some(value),
            _ => None,
        })
        .expect("encounter document");
    let parts = SpecParts::load(&loaded, encounter).expect("spec parts");
    let mut history = HistoryWorld::new(1);
    history
        .start_encounter(&parts.spec(encounter))
        .expect("start");
    let bound: Vec<_> = history
        .world()
        .combatants()
        .iter()
        .map(|(entity, combatant)| (entity, combatant.placement()))
        .collect();
    let presentation = Presentation::from_loaded(&loaded);
    history
        .acknowledge(history.last_sequence())
        .expect("start ack");
    let mut host = Host::from_history(
        history,
        HostConfig {
            protocol: ProtocolSelection::V2,
            incarnation: 1,
        },
        0,
    )
    .expect("host");
    let disclosure = grants(
        &bound,
        &ruleset_abilities(&loaded, encounter.ruleset),
        encounter.id,
    );
    let mut player_actors = Vec::new();
    let mut npc_actors = Vec::new();
    for (entity, placement) in &bound {
        if presentation.combatant(*placement).1 == "Goblin" {
            player_actors.push(*entity);
        } else {
            npc_actors.push(*entity);
        }
    }
    let player = host
        .bind_snapshot_peer(
            ControlGrant {
                actors: player_actors,
            },
            disclosure.clone(),
            0,
        )
        .expect("player");
    let npc = host
        .bind_peer(ControlGrant { actors: npc_actors }, disclosure, 0)
        .expect("npc");
    let (_, npc_sides) =
        presentation_tables(&host, player, npc, &bound, &presentation).expect("tables");
    Authority::new(host, player, npc, link.authority_end(), npc_sides, bound)
}

fn id(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}

fn ull(raw: u128) -> Ulid {
    Ulid::from_u128(raw)
}

fn entry(ordinal: u64, kind: DeliveryKind, bytes: &[u8]) -> DeliveryEntry {
    DeliveryEntry {
        ordinal,
        kind,
        bytes: bytes.to_vec(),
    }
}

/// K4 / B4: only the last accepted ordinal is returned, a transfer is 54, and
/// an empty list accepts nothing.
#[test]
fn hand_off_stops_at_the_first_full_link() {
    let link = LocalLink::new(2);
    let mut end = link.authority_end();
    let entries = [
        entry(1, DeliveryKind::Delta, b"one"),
        entry(2, DeliveryKind::Delta, b"two"),
        entry(3, DeliveryKind::Delta, b"three"),
    ];
    assert_eq!(hand_off(&entries, &mut end), Ok(Some(2)));
    assert_eq!(link.queued(), (0, 2));
    let mut client = link.client_end();
    assert_eq!(client.recv(), Some(b"one".to_vec()));
    assert_eq!(client.recv(), Some(b"two".to_vec()));

    let transfer = [
        entry(1, DeliveryKind::Delta, b"one"),
        entry(2, DeliveryKind::Transfer, b"two"),
    ];
    assert_eq!(
        hand_off(&transfer, &mut end).unwrap_err().code,
        BridgeCode::UnexpectedTransfer
    );
    assert_eq!(hand_off(&[], &mut end), Ok(None));

    // Control: a deeper link accepts the third entry too.
    let deep = LocalLink::new(3);
    let mut deep_end = deep.authority_end();
    assert_eq!(hand_off(&entries, &mut deep_end), Ok(Some(3)));
    assert_eq!(deep.queued().1, 3);
}

/// K5 / B10: the first cross-side attack, else the first option, else none.
#[test]
fn choose_npc_prefers_a_cross_side_attack() {
    let sides = BTreeMap::from([(id(1), "Hero".to_string()), (id(2), "Goblin".to_string())]);
    let options = [
        crpg_server::host::LegalIntent {
            actor: id(1),
            body: IntentBody::EndTurn,
        },
        crpg_server::host::LegalIntent {
            actor: id(1),
            body: IntentBody::DeclareAction {
                ability: ull(10),
                target: id(1),
            },
        },
        crpg_server::host::LegalIntent {
            actor: id(1),
            body: IntentBody::DeclareAction {
                ability: ull(10),
                target: id(2),
            },
        },
    ];
    assert_eq!(choose_npc(&options, &sides), Some(2));

    let own_side = [
        crpg_server::host::LegalIntent {
            actor: id(1),
            body: IntentBody::DeclareAction {
                ability: ull(10),
                target: id(1),
            },
        },
        crpg_server::host::LegalIntent {
            actor: id(1),
            body: IntentBody::DeclareAction {
                ability: ull(11),
                target: id(77),
            },
        },
    ];
    assert_eq!(choose_npc(&own_side, &sides), Some(0));
    assert_eq!(choose_npc(&[], &sides), None);
}

/// K6 / B8: the pacer's first matching arm, both directions.
#[test]
fn pacer_arms_then_fires_only_at_the_due_time() {
    let mut pacer = NpcPacer::default();
    assert!(!pacer.step(20, true, false), "arms at 620");
    assert!(!pacer.step(619, true, false));
    assert!(pacer.step(620, true, false), "fires exactly at due");
    assert!(!pacer.step(621, true, false), "re-arms to 1221");
    assert!(!pacer.step(1221, false, false), "no options clears the due");
    // A stale `due` would have fired at 1221 and again at 1300; the cleared
    // one only arms a fresh 1900.
    assert!(
        !pacer.step(1300, true, false),
        "the due cleared at 1221 is not stale at 1300"
    );
    assert!(!pacer.step(1899, true, false));
    assert!(pacer.step(1900, true, false));

    // Control: armed, then the intent is in flight at 620 -> cleared.
    let mut held = NpcPacer::default();
    assert!(!held.step(20, true, false));
    assert!(!held.step(620, true, true));
    assert!(!held.step(700, true, false), "re-arms at 1300");
    assert!(held.step(1300, true, false));
}

/// K7 / B9: a hold is not a rejection; every other NPC rejection is 51.
#[test]
fn npc_results_keep_a_held_intent_and_fence_a_rejection() {
    let mut host = Host::new(
        7,
        HostConfig {
            protocol: ProtocolSelection::V2,
            incarnation: 1,
        },
        0,
    )
    .expect("host");
    let npc = host
        .bind_peer(
            ControlGrant { actors: vec![] },
            DisclosureGrants::default(),
            0,
        )
        .expect("npc");
    let player = host
        .bind_peer(
            ControlGrant { actors: vec![] },
            DisclosureGrants::default(),
            0,
        )
        .expect("player");

    let applied = AdmissionResult {
        peer: npc,
        seq: Some(3),
        status: ReceiptStatus::Applied,
        cached: false,
        retained_in_ingress: false,
    };
    assert_eq!(npc_results(&[applied], npc, Some(3)), Ok(None));

    // An applied NPC intent that is not the one in flight changes nothing.
    let other = AdmissionResult {
        peer: npc,
        seq: Some(4),
        status: ReceiptStatus::Applied,
        cached: false,
        retained_in_ingress: false,
    };
    assert_eq!(npc_results(&[other], npc, Some(3)), Ok(Some(3)));

    let rejected = AdmissionResult {
        peer: npc,
        seq: Some(3),
        status: ReceiptStatus::Rejected(RejectionCode::IllegalAction),
        cached: false,
        retained_in_ingress: false,
    };
    assert_eq!(
        npc_results(&[rejected], npc, Some(3)).unwrap_err().code,
        BridgeCode::HostFailed
    );

    let held = AdmissionResult {
        peer: npc,
        seq: Some(3),
        status: ReceiptStatus::Rejected(RejectionCode::QueueFull),
        cached: false,
        retained_in_ingress: true,
    };
    assert_eq!(
        npc_results(std::slice::from_ref(&held), npc, Some(3)),
        Ok(Some(3))
    );

    // Control: the same held result belongs to the player and is ignored.
    assert_eq!(
        npc_results(
            &[AdmissionResult {
                peer: player,
                ..held
            }],
            npc,
            Some(3)
        ),
        Ok(Some(3))
    );
}

/// The fake tree used by the walker cases.
#[derive(Default)]
struct FakeFs {
    kinds: BTreeMap<PathBuf, EntryKind>,
    dirs: BTreeMap<PathBuf, Vec<OsString>>,
    files: BTreeMap<PathBuf, Vec<u8>>,
}

impl FakeFs {
    fn dir(&mut self, path: &str, names: &[&str]) {
        self.kinds.insert(PathBuf::from(path), EntryKind::Directory);
        self.dirs.insert(
            PathBuf::from(path),
            names.iter().map(OsString::from).collect(),
        );
    }

    fn file(&mut self, path: &str, bytes: &[u8]) {
        self.kinds.insert(PathBuf::from(path), EntryKind::File);
        self.files.insert(PathBuf::from(path), bytes.to_vec());
    }

    fn kind(&mut self, path: &str, kind: EntryKind) {
        self.kinds.insert(PathBuf::from(path), kind);
    }

    /// Gives a directory real children without inventing a second kind for it.
    fn listing(&mut self, path: &str, names: &[&str]) {
        self.dirs.insert(
            PathBuf::from(path),
            names.iter().map(OsString::from).collect(),
        );
    }
}

impl WalkFs for FakeFs {
    fn metadata(&self, path: &Path) -> std::io::Result<EntryKind> {
        self.kinds
            .get(path)
            .copied()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
    }

    fn read_dir_names(&self, path: &Path) -> std::io::Result<Vec<OsString>> {
        self.dirs
            .get(path)
            .cloned()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
    }

    fn read_file(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
    }
}

#[cfg(unix)]
fn non_unicode_name() -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(vec![0x66, 0x6f, 0x80, 0x6f])
}

#[cfg(windows)]
fn non_unicode_name() -> OsString {
    use std::os::windows::ffi::OsStringExt;
    OsString::from_wide(&[0x0066, 0xD800])
}

/// K34 / B1(3): every walker refusal, the sorted first failure, and the
/// regular-file control. Every fake directory that a missing guard could walk
/// into is listed with real children, so only the guard can turn a case into
/// `Ok`.
#[test]
fn collect_campaign_refuses_the_same_shapes_as_the_cli_walker() {
    // Control: a regular tree collects exactly the classified bytes, and a
    // link at an unclassified path is ignored.
    let mut ok = FakeFs::default();
    ok.dir("campaign", &["creatures"]);
    ok.dir("campaign/creatures", &["a.json", "notes.txt"]);
    ok.file("campaign/creatures/a.json", b"a");
    ok.kind("campaign/creatures/notes.txt", EntryKind::Symlink);
    let collected = collect_campaign(&ok, Path::new("campaign")).expect("regular tree");
    let document = crpg_data::campaign_document_path("creatures/a.json")
        .expect("classifies")
        .expect("is a document");
    assert_eq!(collected, BTreeMap::from([(document, b"a".to_vec())]));

    // Root symlink: walking the listed root would collect nothing.
    let mut root_link = FakeFs::default();
    root_link.listing("campaign", &[]);
    root_link.kind("campaign", EntryKind::Symlink);
    let fault = collect_campaign(&root_link, Path::new("campaign")).unwrap_err();
    assert_eq!(fault.code, BridgeCode::CampaignIo);
    assert_eq!(fault.detail, "open root <campaign-root>");

    // Symlink ancestor: the root itself is a normal, listable directory.
    let mut ancestor = FakeFs::default();
    ancestor.kind("alias", EntryKind::Symlink);
    ancestor.kind("alias/campaign", EntryKind::Directory);
    ancestor.listing("alias/campaign", &[]);
    let fault = collect_campaign(&ancestor, Path::new("alias/campaign")).unwrap_err();
    assert_eq!(fault.code, BridgeCode::CampaignIo);
    assert_eq!(fault.detail, "open root <campaign-root>");

    // File root: likewise listable if the root check were skipped.
    let mut file_root = FakeFs::default();
    file_root.listing("campaign", &[]);
    file_root.kind("campaign", EntryKind::File);
    let fault = collect_campaign(&file_root, Path::new("campaign")).unwrap_err();
    assert_eq!(fault.code, BridgeCode::CampaignIo);
    assert_eq!(fault.detail, "open root <campaign-root>");

    let mut non_unicode = FakeFs::default();
    non_unicode
        .kinds
        .insert(PathBuf::from("campaign"), EntryKind::Directory);
    non_unicode
        .dirs
        .insert(PathBuf::from("campaign"), vec![non_unicode_name()]);
    assert_eq!(
        collect_campaign(&non_unicode, Path::new("campaign"))
            .unwrap_err()
            .code,
        BridgeCode::CampaignIo
    );

    let mut document_link = FakeFs::default();
    document_link.dir("campaign", &["creatures"]);
    document_link.dir("campaign/creatures", &["x.json"]);
    document_link.file("campaign/creatures/x.json", b"x");
    document_link.kind("campaign/creatures/x.json", EntryKind::Symlink);
    let fault = collect_campaign(&document_link, Path::new("campaign")).unwrap_err();
    assert_eq!(fault.code, BridgeCode::CampaignIo);
    assert_eq!(fault.detail, "classify creatures/x.json");

    let mut unreadable = FakeFs::default();
    unreadable.dir("campaign", &["creatures"]);
    unreadable.dir("campaign/creatures", &["a.json"]);
    unreadable.kind("campaign/creatures/a.json", EntryKind::File);
    let fault = collect_campaign(&unreadable, Path::new("campaign")).unwrap_err();
    assert_eq!(fault.code, BridgeCode::CampaignIo);
    assert_eq!(fault.detail, "read creatures/a.json");

    let mut classifier = FakeFs::default();
    classifier.dir("campaign", &["creatures"]);
    classifier.dir("campaign/creatures", &["bad name.json"]);
    classifier.file("campaign/creatures/bad name.json", b"x");
    let fault = collect_campaign(&classifier, Path::new("campaign")).unwrap_err();
    assert_eq!(fault.code, BridgeCode::CampaignInvalid);
    assert_eq!(fault.detail, "classify creatures/bad name.json");

    // Two failures: the byte-sorted first is the one reported.
    let mut unsorted = FakeFs::default();
    unsorted.dir("campaign", &["creatures"]);
    unsorted.dir("campaign/creatures", &["b.json", "a.json"]);
    unsorted.kind("campaign/creatures/a.json", EntryKind::File);
    unsorted.kind("campaign/creatures/b.json", EntryKind::File);
    let fault = collect_campaign(&unsorted, Path::new("campaign")).unwrap_err();
    assert_eq!(fault.code, BridgeCode::CampaignIo);
    assert_eq!(fault.detail, "read creatures/a.json");
}

/// K35 (direct half): a swapped or missing placement is `START_FAILED`; the
/// fixture order is accepted.
#[test]
fn check_binding_requires_the_authored_order() {
    let mut world = World::new(1);
    let first = world.spawn(EntityMeta {});
    let second = world.spawn(EntityMeta {});
    let bound = [(first, ull(14)), (second, ull(15))];

    assert_eq!(check_binding(&bound, &[ull(14), ull(15)]), Ok(()));
    let swapped = check_binding(&bound, &[ull(15), ull(14)]).unwrap_err();
    assert_eq!(swapped.code, BridgeCode::StartFailed);
    assert_eq!(
        swapped.detail,
        "placement binding mismatch at index 0 (expected 0000000000000000000000000F, found 0000000000000000000000000E)"
    );
    // Both length directions: fewer and more bound entries than authored.
    assert_eq!(
        check_binding(&bound[..1], &[ull(14), ull(15)])
            .unwrap_err()
            .code,
        BridgeCode::StartFailed
    );
    assert_eq!(
        check_binding(&bound, &[ull(14), ull(15), ull(16)])
            .unwrap_err()
            .code,
        BridgeCode::StartFailed
    );
    // A too-high placement in the second slot: an order check written as `<`
    // would accept it, an equality check refuses it.
    assert_eq!(
        check_binding(&[(first, ull(14)), (second, ull(16))], &[ull(14), ull(15)])
            .unwrap_err()
            .code,
        BridgeCode::StartFailed
    );
    assert_eq!(check_binding(&[], &[]), Ok(()));
}

/// B3(4), B8: after the NPC acts, its delivery is acknowledged unread and its
/// in-flight intent is cleared by the arriving result.
#[test]
fn npc_delivery_is_acknowledged_and_its_inflight_clears() {
    let link = LocalLink::new(LINK_FRAMES);
    let mut authority = fixture_authority(&link);
    let mut client = link.client_end();

    authority.step(0).expect("step 0");
    // The player (Goblin) ends its turn so the hero becomes active.
    let (epoch, actor) = {
        let options = authority
            .host
            .legal_intents(authority.player)
            .expect("options");
        let actor = options
            .iter()
            .find(|option| matches!(option.body, IntentBody::EndTurn))
            .expect("end turn")
            .actor;
        (
            authority.host.epoch(authority.player).expect("epoch"),
            actor,
        )
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
    authority.step(10).expect("step 10");
    assert_eq!(authority.npc_submitted, 0);

    authority.step(1010).expect("arm at 1610");
    authority.step(2010).expect("fire at 1610");
    assert_eq!(authority.npc_submitted, 1);
    assert_eq!(
        authority.inflight, None,
        "the NPC's own result arrived and cleared it"
    );
    let npc = authority.npc;
    assert!(
        authority
            .host
            .take_delivery_entries(npc)
            .expect("npc entries")
            .is_empty(),
        "step 4 acknowledged the NPC delivery unread"
    );
    // The next poll only re-arms: no second ingest before the new delay.
    authority.step(2020).expect("re-arm");
    assert_eq!(authority.npc_submitted, 1);
}
