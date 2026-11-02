//! K4-K7, K34 and the direct half of K35: the hand-off, the NPC chooser and
//! pacer, the pump result fold, the campaign walk and the start-order binding.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crpg_core::Ulid;
use crpg_net::protocol::{IntentBody, NetId, ReceiptStatus, RejectionCode};
use crpg_net::transport::Transport;
use crpg_server::host::{
    AdmissionResult, ControlGrant, DeliveryEntry, DeliveryKind, DisclosureGrants, Host, HostConfig,
    ProtocolSelection,
};
use crpg_sim::{EntityMeta, World};

use super::{
    check_binding, choose_npc, collect_campaign, hand_off, npc_results, EntryKind, NpcPacer, WalkFs,
};
use crate::core::errors::BridgeCode;
use crate::core::link::LocalLink;

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
/// regular-file control.
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
    assert_eq!(collected.len(), 1);
    let (path, bytes) = collected.iter().next().expect("one entry");
    assert_eq!(path.as_str(), "creatures/a.json");
    assert_eq!(bytes, b"a");

    let mut root_link = FakeFs::default();
    root_link.kind("campaign", EntryKind::Symlink);
    assert_eq!(
        collect_campaign(&root_link, Path::new("campaign"))
            .unwrap_err()
            .code,
        BridgeCode::CampaignIo
    );

    let mut ancestor = FakeFs::default();
    ancestor.kind("alias", EntryKind::Symlink);
    assert_eq!(
        collect_campaign(&ancestor, Path::new("alias/campaign"))
            .unwrap_err()
            .code,
        BridgeCode::CampaignIo
    );

    let mut file_root = FakeFs::default();
    file_root.kind("campaign", EntryKind::File);
    assert_eq!(
        collect_campaign(&file_root, Path::new("campaign"))
            .unwrap_err()
            .code,
        BridgeCode::CampaignIo
    );

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
    document_link.kind("campaign/creatures/x.json", EntryKind::Symlink);
    assert_eq!(
        collect_campaign(&document_link, Path::new("campaign"))
            .unwrap_err()
            .code,
        BridgeCode::CampaignIo
    );

    let mut unreadable = FakeFs::default();
    unreadable.dir("campaign", &["creatures"]);
    unreadable.dir("campaign/creatures", &["a.json"]);
    unreadable.kind("campaign/creatures/a.json", EntryKind::File);
    assert_eq!(
        collect_campaign(&unreadable, Path::new("campaign"))
            .unwrap_err()
            .code,
        BridgeCode::CampaignIo
    );

    let mut classifier = FakeFs::default();
    classifier.dir("campaign", &["creatures"]);
    classifier.dir("campaign/creatures", &["bad name.json"]);
    classifier.file("campaign/creatures/bad name.json", b"x");
    assert_eq!(
        collect_campaign(&classifier, Path::new("campaign"))
            .unwrap_err()
            .code,
        BridgeCode::CampaignInvalid
    );

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
    assert_eq!(
        check_binding(&bound, &[ull(15), ull(14)]).unwrap_err().code,
        BridgeCode::StartFailed
    );
    assert_eq!(
        check_binding(&bound[..1], &[ull(14), ull(15)])
            .unwrap_err()
            .code,
        BridgeCode::StartFailed
    );
}
