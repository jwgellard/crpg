#![forbid(unsafe_code)]
mod support;
use crpg_core::{Fx16_16, Ulid};
use crpg_data::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;

type Files = BTreeMap<SourcePath, Vec<u8>>;
fn engine() -> semver::Version {
    "0.1.0".parse().unwrap()
}
fn path(s: &str) -> SourcePath {
    s.parse().unwrap()
}
fn edit(files: &mut Files, name: &str, change: impl FnOnce(&mut Value)) {
    let key = path(name);
    let mut value = serde_json::from_slice(&files[&key]).unwrap();
    change(&mut value);
    files.insert(key, canonical_json(&value).unwrap());
}
fn loaded() -> LoadedCampaign {
    load_campaign(&support::fixture_files(), &engine()).unwrap()
}
fn set_requirements(files: &mut Files) {
    edit(files, "campaign.json", |v| {
        v["requires"] = json!([{"kind":"module","package":"a","version":"^1"}])
    });
}
fn set_lock(files: &mut Files, kind: &str, version: &str) {
    edit(
        files,
        "campaign.lock",
        |v| v["packages"] = json!([{"kind":kind,"package":"a","version":version,"checksum":Digest::from_bytes([1;32])}]),
    );
}

#[test]
fn exact_fixture_index_bytes_and_models() {
    let files = support::fixture_files();
    assert_eq!(files.len(), 10);
    let campaign = load_campaign(&files, &engine()).unwrap();
    let entries = [
        (1, ObjectKind::Campaign, "campaign.json", ""),
        (2, ObjectKind::World, "worlds/world.json", ""),
        (3, ObjectKind::Area, "areas/start/area.json", ""),
        (4, ObjectKind::Creature, "creatures/creature.json", ""),
        (
            5,
            ObjectKind::Placement,
            "areas/start/placements.json",
            "/placements/0",
        ),
        (
            6,
            ObjectKind::Graph,
            "areas/start/triggers.json",
            "/graphs/0",
        ),
        (
            7,
            ObjectKind::Node,
            "areas/start/triggers.json",
            "/graphs/0/nodes/0",
        ),
    ];
    let expected = entries
        .into_iter()
        .map(|(id, kind, p, pointer)| {
            (
                Ulid::from_u128(id),
                IndexEntry {
                    kind,
                    path: path(p),
                    pointer: pointer.into(),
                },
            )
        })
        .collect();
    assert_eq!(campaign.index, expected);
    assert_eq!(serialize_campaign(&campaign).unwrap(), files);
    assert_eq!(
        serialize_campaign(
            &load_campaign(&serialize_campaign(&campaign).unwrap(), &engine()).unwrap()
        )
        .unwrap(),
        files
    );
    let Document::Campaign(manifest) = &campaign.documents[&path("campaign.json")] else {
        panic!()
    };
    assert_eq!(manifest.package.as_str(), "fixture.one-area");
    assert_eq!(manifest.version.to_string(), "0.1.0");
    assert_eq!(manifest.engine.to_string(), ">=0.1.0, <1.0.0");
    assert_eq!(manifest.note.as_deref(), Some("T010 canonical fixture"));
    assert_eq!(
        manifest.entry,
        EntryPoint {
            world: Ulid::from_u128(2),
            area: Ulid::from_u128(3),
            spawn: Ulid::from_u128(5)
        }
    );
    let Document::Creature(creature) = &campaign.documents[&path("creatures/creature.json")] else {
        panic!()
    };
    assert_eq!(
        creature.stats,
        BTreeMap::from([("health".into(), Fx16_16::from_int(10))])
    );
    assert_eq!(creature.tags, ["fixture"]);
    assert_eq!(creature.faction, None);
    assert!(creature.inventory.is_empty());
    let Document::Placements(placements) =
        &campaign.documents[&path("areas/start/placements.json")]
    else {
        panic!()
    };
    assert_eq!(placements.area, Ulid::from_u128(3));
    assert_eq!(placements.placements[0].prefab, Ulid::from_u128(4));
    assert_eq!(
        placements.placements[0].transform,
        Transform {
            position: [Fx16_16::ZERO; 3],
            rotation: [Fx16_16::ZERO; 3],
            scale: [Fx16_16::ONE; 3]
        }
    );
    let Document::Triggers(triggers) = &campaign.documents[&path("areas/start/triggers.json")]
    else {
        panic!()
    };
    assert_eq!(triggers.graphs[0].entry, Trigger::AreaEnter);
    assert_eq!(triggers.graphs[0].start, Ulid::from_u128(7));
    assert_eq!(
        triggers.graphs[0].nodes[0].body,
        NodeBody::Wait { ticks: 1 }
    );
    assert!(triggers.graphs[0].edges.is_empty());
    assert!(triggers.graphs[0].locals.is_empty());
}

#[test]
fn move_changes_only_index_path_and_dangling_refs_still_load() {
    let files = support::fixture_files();
    let before = load_campaign(&files, &engine()).unwrap();
    let mut moved = files;
    let bytes = moved.remove(&path("creatures/creature.json")).unwrap();
    moved.insert(path("creatures/nested/creature.json"), bytes);
    let after = load_campaign(&moved, &engine()).unwrap();
    let mut expected = before.index;
    expected.get_mut(&Ulid::from_u128(4)).unwrap().path = path("creatures/nested/creature.json");
    assert_eq!(after.index, expected);
    assert_eq!(
        after.documents[&path("creatures/nested/creature.json")],
        before.documents[&path("creatures/creature.json")]
    );
    assert_eq!(
        after.documents[&path("areas/start/placements.json")],
        before.documents[&path("areas/start/placements.json")]
    );
    edit(&mut moved, "campaign.json", |v| {
        v["entry"]["world"] = json!(Ulid::from_u128(999))
    });
    edit(&mut moved, "areas/start/placements.json", |v| {
        v["placements"][0]["prefab"] = json!(Ulid::from_u128(998))
    });
    assert!(load_campaign(&moved, &engine()).is_ok());
}

fn extra_entities(files: &mut Files) {
    let id = Ulid::from_u128;
    for (p, value) in [
        (
            "items/nested/item.json",
            json!({"schema":"crpg.item/2","id":id(10),"slug":"i","name":"i","stats":{},"tags":[]}),
        ),
        (
            "factions/faction.json",
            json!({"schema":"crpg.faction/1","id":id(11),"slug":"f","name":"f","relations":[]}),
        ),
        (
            "dialogue/dialogue.json",
            json!({"schema":"crpg.dialogue/1","id":id(12),"slug":"d","name":"d","entry":id(13),"nodes":[{"id":id(13),"body":{"kind":"end"}}]}),
        ),
        (
            "quests/quest.json",
            json!({"schema":"crpg.quest/1","id":id(14),"slug":"q","name":"q","entry":id(15),"states":[{"id":id(15),"name":"s","terminal":true,"on_enter":[],"transitions":[]}]}),
        ),
        (
            "scripts/graphs/nested/graph.json",
            json!({"schema":"crpg.graph/1","id":id(16),"slug":"g","name":"g","entry":{"kind":"timer","ticks":0},"start":id(17),"nodes":[{"id":id(17),"body":{"kind":"wait","ticks":0}}],"edges":[],"locals":[]}),
        ),
    ] {
        files.insert(path(p), canonical_json(&value).unwrap());
    }
}

#[test]
fn independently_indexes_every_root_and_nested_kind() {
    let mut files = support::fixture_files();
    extra_entities(&mut files);
    let campaign = load_campaign(&files, &engine()).unwrap();
    assert_eq!(campaign.index.len(), 15);
    for (id, kind, p, pointer) in [
        (10, ObjectKind::Item, "items/nested/item.json", ""),
        (11, ObjectKind::Faction, "factions/faction.json", ""),
        (12, ObjectKind::Dialogue, "dialogue/dialogue.json", ""),
        (
            13,
            ObjectKind::DialogueNode,
            "dialogue/dialogue.json",
            "/nodes/0",
        ),
        (14, ObjectKind::Quest, "quests/quest.json", ""),
        (15, ObjectKind::QuestState, "quests/quest.json", "/states/0"),
        (
            16,
            ObjectKind::Graph,
            "scripts/graphs/nested/graph.json",
            "",
        ),
        (
            17,
            ObjectKind::Node,
            "scripts/graphs/nested/graph.json",
            "/nodes/0",
        ),
    ] {
        assert_eq!(
            campaign.index[&Ulid::from_u128(id)],
            IndexEntry {
                kind,
                path: path(p),
                pointer: pointer.into()
            }
        );
    }
    assert_eq!(serialize_campaign(&campaign).unwrap(), files);
}

#[test]
fn duplicate_roots_entries_and_children_fail_in_authored_order() {
    for (p, pointer, id) in [
        ("creatures/creature.json", "/id", 3),
        ("areas/start/placements.json", "/placements/0/id", 3),
        ("areas/start/triggers.json", "/graphs/0/id", 5),
        ("areas/start/triggers.json", "/graphs/0/nodes/0/id", 6),
        ("dialogue/dialogue.json", "/nodes/0/id", 12),
        ("quests/quest.json", "/states/0/id", 14),
        ("scripts/graphs/nested/graph.json", "/nodes/0/id", 16),
    ] {
        let mut files = support::fixture_files();
        extra_entities(&mut files);
        edit(&mut files, p, |v| {
            *v.pointer_mut(pointer).unwrap() = json!(Ulid::from_u128(id))
        });
        assert!(
            matches!(load_campaign(&files,&engine()), Err(DataError::DuplicateId { id:actual, second, .. }) if actual == Ulid::from_u128(id) && second == path(p))
        );
    }
    let mut files = support::fixture_files();
    edit(&mut files, "areas/start/placements.json", |v| {
        let p = v["placements"][0].clone();
        v["placements"].as_array_mut().unwrap().push(p);
    });
    assert!(
        matches!(load_campaign(&files,&engine()), Err(DataError::DuplicateId { first, second, .. }) if first == second && first == path("areas/start/placements.json"))
    );
    let mut files = support::fixture_files();
    let trigger: Value =
        serde_json::from_slice(&files[&path("areas/start/triggers.json")]).unwrap();
    let mut graph = trigger["graphs"][0].clone();
    graph["schema"] = json!("crpg.graph/1");
    files.insert(
        path("scripts/graphs/copy.json"),
        canonical_json(&graph).unwrap(),
    );
    assert!(
        matches!(load_campaign(&files,&engine()), Err(DataError::DuplicateId { id, first, second }) if id == Ulid::from_u128(6) && first == path("areas/start/triggers.json") && second == path("scripts/graphs/copy.json"))
    );
}

#[test]
fn read_and_load_phase_precedence() {
    let mut files = support::fixture_files();
    files.insert(path("CREATURES/creature.json"), b"invalid".to_vec());
    files.remove(&path("campaign.json"));
    assert!(
        matches!(load_campaign(&files,&engine()), Err(DataError::Layout { path:Some(p), .. }) if p == path("creatures/creature.json"))
    );
    for missing in ["campaign.json", "campaign.lock", "assets/assets.lock"] {
        let mut files = support::fixture_files();
        files.remove(&path(missing));
        files.insert(path("z.json"), b"invalid".to_vec());
        assert!(
            matches!(load_campaign(&files,&engine()), Err(DataError::Layout { path:None, message }) if message.contains(missing))
        );
    }
    assert!(
        matches!(load_campaign(&BTreeMap::new(),&engine()), Err(DataError::Layout { message, .. }) if message.contains("campaign.json"))
    );
    assert!(
        matches!(read_document(b"{\"schema\":\"crpg.creature/2\",\"id\":false}"), Err(DataError::UnsupportedSchema { path:None, found }) if found == "crpg.creature/2")
    );
    let mut files = support::fixture_files();
    files.insert(
        path("unknown.json"),
        files[&path("creatures/creature.json")].clone(),
    );
    files.insert(path("worlds/world.json"), b"invalid".to_vec());
    assert!(
        matches!(load_campaign(&files,&engine()), Err(DataError::Malformed { path:Some(p), .. }) if p == path("worlds/world.json"))
    );
    let mut files = support::fixture_files();
    files.insert(
        path("unknown.json"),
        files[&path("creatures/creature.json")].clone(),
    );
    assert!(matches!(
        load_campaign(&files, &"2.0.0".parse().unwrap()),
        Err(DataError::Layout { .. })
    ));
    let mut files = support::fixture_files();
    edit(&mut files, "creatures/creature.json", |v| {
        v["id"] = json!(Ulid::from_u128(3))
    });
    assert!(
        matches!(load_campaign(&files,&"2.0.0".parse().unwrap()), Err(DataError::EngineIncompatible { required,actual }) if required == ">=0.1.0, <1.0.0" && actual == "2.0.0")
    );
    set_requirements(&mut files);
    assert!(matches!(
        load_campaign(&files, &engine()),
        Err(DataError::DuplicateId { .. })
    ));
    edit(&mut files, "creatures/creature.json", |v| {
        v["id"] = json!(Ulid::from_u128(4))
    });
    edit(&mut files, "campaign.lock", |v| {
        v["assets_lock"] = json!(Digest::from_bytes([0; 32]))
    });
    assert!(matches!(
        load_campaign(&files, &engine()),
        Err(DataError::InvalidLock { .. })
    ));
    set_lock(&mut files, "module", "1.0.0");
    assert!(
        matches!(load_campaign(&files,&engine()), Err(DataError::AssetsLockMismatch { expected, actual }) if expected == Digest::from_bytes([0;32]) && actual.to_string() == "957bc137f1abb3cde6cee277d10c099b1dcd2f8814a5fd0e29b1df3506ee44fb")
    );
}

#[test]
fn lexical_read_and_layout_order_and_illegal_placements() {
    let mut files = support::fixture_files();
    files.insert(path("worlds/world.json"), b"invalid".to_vec());
    files.insert(
        path("areas/start/area.json"),
        b"{\"schema\":\"future/1\"}".to_vec(),
    );
    assert!(
        matches!(load_campaign(&files,&engine()), Err(DataError::UnsupportedSchema { path:Some(p), .. }) if p == path("areas/start/area.json"))
    );
    for (old, new) in [
        ("campaign.json", "worlds/extra.json"),
        ("campaign.lock", "worlds/extra.json"),
        ("assets/assets.lock", "assets/extra.lock"),
        ("areas/start/area.json", "areas/area.json"),
        ("areas/start/placements.json", "areas/start/instance.json"),
        ("locale/en.json", "locale/nested/en.json"),
        ("creatures/creature.json", "items/creature.json"),
        ("variables/campaign_state.json", "variables/other.json"),
    ] {
        let mut files = support::fixture_files();
        files.insert(path(new), files[&path(old)].clone());
        assert!(
            matches!(
                load_campaign(&files, &engine()),
                Err(DataError::Layout { .. })
            ),
            "{new}"
        );
    }
    let mut files = support::fixture_files();
    edit(&mut files, "locale/en.json", |v| v["locale"] = json!("fr"));
    assert!(matches!(
        load_campaign(&files, &engine()),
        Err(DataError::Layout { .. })
    ));
    let mut files = support::fixture_files();
    files.insert(
        path("campaign.lock"),
        files[&path("assets/assets.lock")].clone(),
    );
    assert!(
        matches!(load_campaign(&files,&engine()), Err(DataError::Layout { path:Some(p), .. }) if p == path("campaign.lock"))
    );
}

#[test]
fn migration_sits_inside_reads_with_fixed_precedence() {
    // Syntax wins over an old tag that would otherwise migrate.
    assert!(matches!(
        read_document(b"{\"schema\":\"crpg.item/1\",\"stats\":1.0}"),
        Err(DataError::Malformed { path: None, .. })
    ));
    // Unsupported still wins over typed-field errors; old invalid payloads
    // fail as malformed rather than silently repairing.
    assert!(matches!(
        read_document(b"{\"schema\":\"crpg.item/1\"}"),
        Err(DataError::Malformed { path: None, .. })
    ));
    // Required-file and collision checks beat migration errors.
    let mut files = support::fixture_files();
    files.remove(&path("campaign.json"));
    files.insert(
        path("items/item.json"),
        b"{\"schema\":\"crpg.item/1\"}".to_vec(),
    );
    assert!(matches!(
        load_campaign(&files, &engine()),
        Err(DataError::Layout { path: None, .. })
    ));
    // Lexical first read failure wins: items/... migrates badly before worlds/... parses badly.
    let mut files = support::fixture_files();
    files.insert(
        path("items/item.json"),
        b"{\"schema\":\"crpg.item/1\"}".to_vec(),
    );
    files.insert(path("worlds/world.json"), b"invalid".to_vec());
    assert!(matches!(
        load_campaign(&files, &engine()),
        Err(DataError::Malformed { path: Some(p), .. }) if p == path("items/item.json")
    ));
    // Migration failure beats a layout error whose own read succeeds.
    let mut files = support::fixture_files();
    files.insert(
        path("items/item.json"),
        b"{\"schema\":\"crpg.item/1\"}".to_vec(),
    );
    files.insert(
        path("unknown.json"),
        files[&path("creatures/creature.json")].clone(),
    );
    assert!(matches!(
        load_campaign(&files, &engine()),
        Err(DataError::Malformed { path: Some(p), .. }) if p == path("items/item.json")
    ));
    // Old and new item bytes converge through one dispatcher; locks keep current tags.
    let old = canonical_json(&json!({"schema":"crpg.item/1","id":Ulid::from_u128(8),"slug":"item","name":"fixture.creature","stats":{},"tags":[]})).unwrap();
    let Document::Item(migrated) = read_document(&old).unwrap() else {
        panic!("migrated item")
    };
    assert_eq!(migrated.id, Ulid::from_u128(8));
    let current = write_document(&Document::Item(migrated.clone())).unwrap();
    let current_value: Value = serde_json::from_slice(&current).unwrap();
    assert_eq!(current_value["schema"], json!("crpg.item/2"));
    assert_eq!(read_document(&current).unwrap(), Document::Item(migrated));
    let fixture = support::fixture_files();
    assert!(read_campaign_lock(&fixture[&path("campaign.lock")]).is_ok());
    assert!(read_assets_lock(&fixture[&path("assets/assets.lock")]).is_ok());
}

#[test]
fn exact_lock_coverage_kind_and_range_without_reresolution() {
    for (kind, version, valid) in [
        ("module", "1.0.0", true),
        ("module", "1.9.0", true),
        ("ruleset", "1.0.0", false),
        ("module", "2.0.0", false),
    ] {
        let mut files = support::fixture_files();
        set_requirements(&mut files);
        set_lock(&mut files, kind, version);
        let result = load_campaign(&files, &engine());
        if valid {
            assert_eq!(serialize_campaign(&result.unwrap()).unwrap(), files);
        } else {
            assert!(matches!(result, Err(DataError::InvalidLock { .. })));
        }
    }
    let mut files = support::fixture_files();
    set_lock(&mut files, "module", "1.0.0");
    assert!(matches!(
        load_campaign(&files, &engine()),
        Err(DataError::InvalidLock { .. })
    ));
    set_requirements(&mut files);
    edit(&mut files, "campaign.json", |v| {
        v["requires"]
            .as_array_mut()
            .unwrap()
            .push(json!({"kind":"module","package":"a","version":">=1.1"}))
    });
    assert!(matches!(
        load_campaign(&files, &engine()),
        Err(DataError::InvalidLock { .. })
    ));
}

#[test]
fn writer_rechecks_shared_policy_and_ignores_mutable_index() {
    let mut campaign = loaded();
    campaign.index.clear();
    assert_eq!(
        serialize_campaign(&campaign).unwrap(),
        support::fixture_files()
    );
    let Document::Campaign(v) = campaign.documents.get_mut(&path("campaign.json")).unwrap() else {
        panic!()
    };
    v.engine = ">=99".parse().unwrap();
    assert!(serialize_campaign(&campaign).is_ok());
    for defect in 0..7 {
        let mut campaign = loaded();
        match defect {
            0 => {
                campaign.documents.remove(&path("campaign.lock"));
            }
            1 => {
                campaign.documents.insert(
                    path("CREATURES/creature.json"),
                    campaign.documents[&path("creatures/creature.json")].clone(),
                );
            }
            2 => {
                campaign.documents.insert(
                    path("wrong.json"),
                    campaign.documents[&path("creatures/creature.json")].clone(),
                );
            }
            3 => {
                if let Document::Creature(v) = campaign
                    .documents
                    .get_mut(&path("creatures/creature.json"))
                    .unwrap()
                {
                    v.id = Ulid::from_u128(3);
                }
            }
            4 => {
                if let Document::CampaignLock(v) =
                    campaign.documents.get_mut(&path("campaign.lock")).unwrap()
                {
                    v.assets_lock = Digest::from_bytes([0; 32]);
                }
            }
            5 => {
                if let Document::Campaign(v) =
                    campaign.documents.get_mut(&path("campaign.json")).unwrap()
                {
                    v.requires.push(PackageRequirement {
                        kind: PackageKind::Module,
                        package: "missing".parse().unwrap(),
                        version: "*".parse().unwrap(),
                    });
                }
            }
            6 => {
                if let Document::AssetsLock(v) = campaign
                    .documents
                    .get_mut(&path("assets/assets.lock"))
                    .unwrap()
                {
                    v.assets.insert(
                        path("bad"),
                        AssetRecord {
                            hash: Digest::from_bytes([0; 32]),
                            import: BTreeMap::new(),
                        },
                    );
                }
            }
            _ => unreachable!(),
        }
        match (defect, serialize_campaign(&campaign).unwrap_err()) {
            (0..=2, DataError::Layout { .. })
            | (3, DataError::DuplicateId { .. })
            | (4, DataError::AssetsLockMismatch { .. })
            | (5..=6, DataError::InvalidLock { .. }) => {}
            (_, error) => panic!("defect {defect}: {error}"),
        }
    }

    let make_duplicate = |campaign: &mut LoadedCampaign| {
        let Document::Creature(creature) = campaign
            .documents
            .get_mut(&path("creatures/creature.json"))
            .unwrap()
        else {
            panic!()
        };
        creature.id = Ulid::from_u128(3);
    };
    let make_bad_assets = |campaign: &mut LoadedCampaign| {
        let Document::AssetsLock(lock) = campaign
            .documents
            .get_mut(&path("assets/assets.lock"))
            .unwrap()
        else {
            panic!()
        };
        lock.assets.insert(
            path("bad"),
            AssetRecord {
                hash: Digest::from_bytes([0; 32]),
                import: BTreeMap::new(),
            },
        );
    };
    let make_bad_coverage = |campaign: &mut LoadedCampaign| {
        let Document::Campaign(manifest) =
            campaign.documents.get_mut(&path("campaign.json")).unwrap()
        else {
            panic!()
        };
        manifest.requires.push(PackageRequirement {
            kind: PackageKind::Module,
            package: "missing".parse().unwrap(),
            version: "*".parse().unwrap(),
        });
    };
    let make_bad_digest = |campaign: &mut LoadedCampaign| {
        let Document::CampaignLock(lock) =
            campaign.documents.get_mut(&path("campaign.lock")).unwrap()
        else {
            panic!()
        };
        lock.assets_lock = Digest::from_bytes([0; 32]);
    };

    let mut campaign = loaded();
    campaign.documents.remove(&path("campaign.lock"));
    make_duplicate(&mut campaign);
    make_bad_assets(&mut campaign);
    assert!(matches!(
        serialize_campaign(&campaign),
        Err(DataError::Layout { .. })
    ));

    let mut campaign = loaded();
    make_bad_assets(&mut campaign);
    make_duplicate(&mut campaign);
    assert!(matches!(
        serialize_campaign(&campaign),
        Err(DataError::InvalidLock { .. })
    ));

    let mut campaign = loaded();
    make_duplicate(&mut campaign);
    make_bad_coverage(&mut campaign);
    make_bad_digest(&mut campaign);
    assert!(matches!(
        serialize_campaign(&campaign),
        Err(DataError::DuplicateId { .. })
    ));

    let mut campaign = loaded();
    make_bad_coverage(&mut campaign);
    make_bad_digest(&mut campaign);
    assert!(matches!(
        serialize_campaign(&campaign),
        Err(DataError::InvalidLock { .. })
    ));
}
