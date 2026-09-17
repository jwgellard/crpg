#![forbid(unsafe_code)]
mod support;
use crpg_core::Ulid;
use crpg_data::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

type Files = BTreeMap<SourcePath, Vec<u8>>;
fn engine() -> semver::Version {
    "0.1.0".parse().unwrap()
}
fn path(s: &str) -> SourcePath {
    s.parse().unwrap()
}
fn id(n: u128) -> Ulid {
    Ulid::from_u128(n)
}
fn valid_loaded() -> LoadedCampaign {
    load_campaign(&support::fixture_files(), &engine()).unwrap()
}
fn broken_files() -> Files {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/broken_references");
    [
        "campaign.json",
        "campaign.lock",
        "worlds/world.json",
        "areas/start/area.json",
        "areas/start/placements.json",
        "areas/start/triggers.json",
        "creatures/creature.json",
        "creatures/z-duplicate.json",
        "dialogue/broken.json",
        "quests/broken.json",
        "variables/campaign_state.json",
        "assets/assets.lock",
        "locale/en.json",
    ]
    .into_iter()
    .map(|name| {
        (
            path(name),
            fs::read(root.join(name)).expect("broken fixture file"),
        )
    })
    .collect()
}
fn edit(files: &mut Files, name: &str, change: impl FnOnce(&mut Value)) {
    let key = path(name);
    let mut value = serde_json::from_slice(&files[&key]).unwrap();
    change(&mut value);
    files.insert(key, canonical_json(&value).unwrap());
}
fn only_codes(diagnostics: &[Diagnostic]) -> Vec<DiagnosticCode> {
    diagnostics.iter().map(|d| d.code).collect()
}
fn has(files: &Files, file: &str, pointer: &str, code: DiagnosticCode) -> bool {
    validate(&load_campaign(files, &engine()).unwrap())
        .iter()
        .any(|d| {
            d.file.as_ref().map(SourcePath::as_str) == Some(file)
                && d.pointer == pointer
                && d.code == code
        })
}

#[test]
fn valid_fixture_validates_clean_with_stable_index() {
    let files = support::fixture_files();
    let campaign = load_campaign(&files, &engine()).unwrap();
    assert_eq!(validate(&campaign), Vec::new());
    assert_eq!(validate_files(&files, &engine()), Vec::new());
    assert_eq!(campaign.index.len(), 7);
    assert_eq!(serialize_campaign(&campaign).unwrap(), files);
}

#[test]
fn broken_fixture_loads_and_matches_checked_in_snapshot() {
    let files = broken_files();
    assert_eq!(files.len(), 13);
    let campaign = load_campaign(&files, &engine()).unwrap();
    let diagnostics = validate(&campaign);
    assert_eq!(diagnostics.len(), 15);
    let expected = [
        (
            "areas/start/area.json",
            "/ambience",
            DiagnosticCode::MissingAsset,
        ),
        (
            "areas/start/area.json",
            "/neighbours/0",
            DiagnosticCode::DanglingReference,
        ),
        (
            "areas/start/placements.json",
            "/area",
            DiagnosticCode::AggregateOwnerMismatch,
        ),
        (
            "areas/start/placements.json",
            "/placements/0/overrides/broken/value",
            DiagnosticCode::DanglingReference,
        ),
        (
            "areas/start/placements.json",
            "/placements/0/prefab",
            DiagnosticCode::WrongReferenceKind,
        ),
        (
            "areas/start/triggers.json",
            "/graphs/0/edges/0/port",
            DiagnosticCode::GraphPortMismatch,
        ),
        (
            "campaign.json",
            "/entry/area",
            DiagnosticCode::DanglingReference,
        ),
        (
            "campaign.json",
            "/entry/world",
            DiagnosticCode::WrongReferenceKind,
        ),
        (
            "creatures/creature.json",
            "/faction",
            DiagnosticCode::WrongReferenceKind,
        ),
        (
            "creatures/creature.json",
            "/inventory/0",
            DiagnosticCode::DanglingReference,
        ),
        (
            "creatures/z-duplicate.json",
            "/slug",
            DiagnosticCode::DuplicateSlug,
        ),
        (
            "dialogue/broken.json",
            "/nodes/0/body/text_key",
            DiagnosticCode::MissingLocaleKey,
        ),
        (
            "dialogue/broken.json",
            "/nodes/1",
            DiagnosticCode::UnreachableNode,
        ),
        (
            "quests/broken.json",
            "/entry",
            DiagnosticCode::QuestNoCompletionPath,
        ),
        (
            "variables/campaign_state.json",
            "/variables/0/default",
            DiagnosticCode::ValueTypeMismatch,
        ),
    ];
    for (found, (file, pointer, code)) in diagnostics.iter().zip(expected.iter()) {
        assert_eq!(found.file.as_ref().map(SourcePath::as_str), Some(*file));
        assert_eq!(found.pointer, *pointer);
        assert_eq!(found.code, *code);
        assert_eq!(found.severity, Severity::Error);
    }
    // Sorted order is part of the contract, not an accident of discovery.
    let mut sorted = diagnostics.clone();
    sorted.sort_by(|a, b| {
        (
            &a.file,
            &a.pointer,
            a.code.as_str(),
            a.severity,
            &a.message,
            &a.suggested_fix,
        )
            .cmp(&(
                &b.file,
                &b.pointer,
                b.code.as_str(),
                b.severity,
                &b.message,
                &b.suggested_fix,
            ))
    });
    assert_eq!(sorted, diagnostics);
    let snapshot_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots/broken_references.diagnostics.json");
    assert_eq!(
        canonical_json(&diagnostics).unwrap(),
        fs::read(snapshot_path).unwrap()
    );
    assert_eq!(validate_files(&files, &engine()), diagnostics);
    // The data-owned gate manifest lists exactly three roots in sorted order.
    let manifest: Value = serde_json::from_slice(
        &fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/expected.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        manifest,
        json!([
            {"expect": "diagnostics", "root": "broken_references",
             "snapshot": "tests/snapshots/broken_references.diagnostics.json"},
            {"expect": "clean", "root": "migration_v1/campaign", "snapshot": null},
            {"expect": "clean", "root": "one_area_one_creature", "snapshot": null},
        ])
    );
}

#[test]
fn insertion_permutation_and_corrupt_index_do_not_change_results() {
    let files = broken_files();
    let baseline = validate(&load_campaign(&files, &engine()).unwrap());
    // BTreeMap iteration is sorted, but rebuild in reverse insertion order anyway.
    let reversed: Files = files
        .iter()
        .rev()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    assert_eq!(
        validate(&load_campaign(&reversed, &engine()).unwrap()),
        baseline
    );
    // The public index is caller-mutable and must not influence validation.
    let mut corrupted = load_campaign(&files, &engine()).unwrap();
    corrupted.index.clear();
    corrupted.index.insert(
        id(4242),
        IndexEntry {
            kind: ObjectKind::Item,
            path: path("items/fake.json"),
            pointer: String::new(),
        },
    );
    assert_eq!(validate(&corrupted), baseline);
    let mut valid = valid_loaded();
    valid.index.clear();
    assert_eq!(validate(&valid), Vec::new());
}

#[test]
fn direct_document_edits_pin_duplicate_id_defense() {
    let mut campaign = valid_loaded();
    let Document::Creature(creature) = campaign
        .documents
        .get_mut(&path("creatures/creature.json"))
        .unwrap()
    else {
        panic!("creature doc")
    };
    creature.id = id(3);
    let diagnostics = validate(&campaign);
    // The duplicate is positioned at its own id field naming the first
    // location; the orphaned placement prefab reference fails alongside it.
    assert!(diagnostics
        .iter()
        .any(|d| d.code == DiagnosticCode::DuplicateId
            && d.file.as_ref().map(SourcePath::as_str) == Some("creatures/creature.json")
            && d.pointer == "/id"
            && d.message.contains("areas/start/area.json")));
    assert!(diagnostics
        .iter()
        .any(|d| d.code == DiagnosticCode::DanglingReference));
    // T010 still reports the same defect first at load time.
    let mut files = support::fixture_files();
    edit(&mut files, "creatures/creature.json", |v| {
        v["id"] = json!(id(3));
    });
    assert_eq!(validate_files(&files, &engine()).len(), 1);
    assert_eq!(
        validate_files(&files, &engine())[0].code,
        DiagnosticCode::DuplicateId
    );
}

#[test]
fn duplicate_slugs_fail_for_every_slug_bearing_kind() {
    // Root entities, inserted directly so layout never interferes.
    for (kind, file, slug) in [
        (ObjectKind::Campaign, "campaign.json", "campaign"),
        (ObjectKind::World, "worlds/world.json", "world"),
        (ObjectKind::Area, "areas/start/area.json", "start"),
        (ObjectKind::Creature, "creatures/creature.json", "creature"),
    ] {
        let mut campaign = valid_loaded();
        let mut extra = campaign.documents[&path(file)].clone();
        let (new_id, extra_path) = match &mut extra {
            Document::Campaign(v) => {
                v.id = id(9101);
                v.entry = EntryPoint {
                    world: id(2),
                    area: id(3),
                    spawn: id(5),
                };
                (id(9101), "campaigns/z-extra.json")
            }
            Document::World(v) => {
                v.id = id(9102);
                v.areas = Vec::new();
                (id(9102), "worlds/z-extra.json")
            }
            Document::Area(v) => {
                v.id = id(9103);
                v.neighbours = Vec::new();
                (id(9103), "areas/z-extra/area.json")
            }
            Document::Creature(v) => {
                v.id = id(9104);
                v.faction = None;
                v.inventory = Vec::new();
                (id(9104), "creatures/z-extra.json")
            }
            _ => panic!("root doc"),
        };
        let _ = (kind, new_id);
        campaign.documents.insert(path(extra_path), extra);
        let diagnostics = validate(&campaign);
        assert_eq!(diagnostics.len(), 1, "slug {slug}");
        assert_eq!(diagnostics[0].code, DiagnosticCode::DuplicateSlug);
        assert_eq!(
            diagnostics[0].file.as_ref().map(SourcePath::as_str),
            Some(extra_path)
        );
        assert_eq!(diagnostics[0].pointer, "/slug");
        assert!(diagnostics[0].message.contains(file));
    }
    // Pair-inserted kinds with no fixture presence also collide within kind.
    // Each pair uses an existing locale key so no missing-locale diagnostic
    // leaks in alongside the duplicate slug.
    let item_pair = |n: u128, slug: &str| {
        Document::Item(Item {
            id: id(n),
            slug: slug.into(),
            name: "fixture.creature".into(),
            note: None,
            stats: BTreeMap::new(),
            tags: Vec::new(),
        })
    };
    let mut campaign = valid_loaded();
    campaign
        .documents
        .insert(path("items/a.json"), item_pair(9301, "dup-item"));
    campaign
        .documents
        .insert(path("items/z-extra.json"), item_pair(9302, "dup-item"));
    let diagnostics = validate(&campaign);
    assert_eq!(
        only_codes(&diagnostics),
        vec![DiagnosticCode::DuplicateSlug]
    );
    assert_eq!(
        diagnostics[0].file.as_ref().map(SourcePath::as_str),
        Some("items/z-extra.json")
    );
    assert_eq!(diagnostics[0].pointer, "/slug");
    let faction_pair = |n: u128, slug: &str| {
        Document::Faction(Faction {
            id: id(n),
            slug: slug.into(),
            name: "fixture.creature".into(),
            note: None,
            relations: Vec::new(),
        })
    };
    let mut campaign = valid_loaded();
    campaign
        .documents
        .insert(path("factions/a.json"), faction_pair(9311, "dup-faction"));
    campaign.documents.insert(
        path("factions/z-extra.json"),
        faction_pair(9312, "dup-faction"),
    );
    let diagnostics = validate(&campaign);
    assert_eq!(
        only_codes(&diagnostics),
        vec![DiagnosticCode::DuplicateSlug]
    );
    assert_eq!(
        diagnostics[0].file.as_ref().map(SourcePath::as_str),
        Some("factions/z-extra.json")
    );
    assert_eq!(diagnostics[0].pointer, "/slug");
    let dialogue_pair = |n: u128, node: u128, slug: &str| {
        Document::Dialogue(Dialogue {
            id: id(n),
            slug: slug.into(),
            name: "fixture.creature".into(),
            note: None,
            entry: id(node),
            nodes: vec![DialogueNode {
                id: id(node),
                body: DialogueBody::End,
            }],
        })
    };
    let mut campaign = valid_loaded();
    campaign.documents.insert(
        path("dialogue/a.json"),
        dialogue_pair(9321, 9323, "dup-dialogue"),
    );
    campaign.documents.insert(
        path("dialogue/z-extra.json"),
        dialogue_pair(9322, 9324, "dup-dialogue"),
    );
    let diagnostics = validate(&campaign);
    assert_eq!(
        only_codes(&diagnostics),
        vec![DiagnosticCode::DuplicateSlug]
    );
    assert_eq!(
        diagnostics[0].file.as_ref().map(SourcePath::as_str),
        Some("dialogue/z-extra.json")
    );
    assert_eq!(diagnostics[0].pointer, "/slug");
    let quest_pair = |n: u128, state: u128, slug: &str| {
        Document::Quest(Quest {
            id: id(n),
            slug: slug.into(),
            name: "fixture.creature".into(),
            note: None,
            entry: id(state),
            states: vec![QuestState {
                id: id(state),
                name: "fixture.creature".into(),
                terminal: true,
                on_enter: Vec::new(),
                transitions: Vec::new(),
            }],
        })
    };
    let mut campaign = valid_loaded();
    campaign
        .documents
        .insert(path("quests/a.json"), quest_pair(9331, 9333, "dup-quest"));
    campaign.documents.insert(
        path("quests/z-extra.json"),
        quest_pair(9332, 9334, "dup-quest"),
    );
    let diagnostics = validate(&campaign);
    assert_eq!(
        only_codes(&diagnostics),
        vec![DiagnosticCode::DuplicateSlug]
    );
    assert_eq!(
        diagnostics[0].file.as_ref().map(SourcePath::as_str),
        Some("quests/z-extra.json")
    );
    assert_eq!(diagnostics[0].pointer, "/slug");
    let graph_pair = |n: u128, node: u128, slug: &str| {
        Document::Graph(EventGraph {
            id: id(n),
            slug: slug.into(),
            name: "fixture.creature".into(),
            note: None,
            entry: Trigger::AreaEnter,
            start: id(node),
            nodes: vec![Node {
                id: id(node),
                body: NodeBody::Wait { ticks: 0 },
            }],
            edges: Vec::new(),
            locals: Vec::new(),
        })
    };
    let mut campaign = valid_loaded();
    campaign.documents.insert(
        path("scripts/graphs/a.json"),
        graph_pair(9341, 9343, "dup-graph"),
    );
    campaign.documents.insert(
        path("scripts/graphs/z-extra.json"),
        graph_pair(9342, 9344, "dup-graph"),
    );
    let diagnostics = validate(&campaign);
    assert_eq!(
        only_codes(&diagnostics),
        vec![DiagnosticCode::DuplicateSlug]
    );
    assert_eq!(
        diagnostics[0].file.as_ref().map(SourcePath::as_str),
        Some("scripts/graphs/z-extra.json")
    );
    assert_eq!(diagnostics[0].pointer, "/slug");
    // Embedded placement and graph entries participate in the same rule.
    let mut campaign = valid_loaded();
    let Document::Placements(placements) = campaign
        .documents
        .get_mut(&path("areas/start/placements.json"))
        .unwrap()
    else {
        panic!("placements doc")
    };
    let mut second = placements.placements[0].clone();
    second.id = id(9201);
    placements.placements.push(second);
    let diagnostics = validate(&campaign);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, DiagnosticCode::DuplicateSlug);
    assert_eq!(diagnostics[0].pointer, "/placements/1/slug");
    let mut campaign = valid_loaded();
    let Document::Triggers(triggers) = campaign
        .documents
        .get_mut(&path("areas/start/triggers.json"))
        .unwrap()
    else {
        panic!("triggers doc")
    };
    let mut graph = triggers.graphs[0].clone();
    graph.id = id(9202);
    graph.nodes[0].id = id(9203);
    graph.start = id(9203);
    triggers.graphs.push(graph);
    let diagnostics = validate(&campaign);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, DiagnosticCode::DuplicateSlug);
    assert_eq!(diagnostics[0].pointer, "/graphs/1/slug");
    // Equal text in different kinds stays valid.
    let mut campaign = valid_loaded();
    let Document::World(world) = campaign
        .documents
        .get_mut(&path("worlds/world.json"))
        .unwrap()
    else {
        panic!("world doc")
    };
    world.slug = "creature".into();
    assert_eq!(validate(&campaign), Vec::new());
}

#[test]
fn every_typed_reference_distinguishes_dangling_wrong_and_foreign() {
    // Dangling references at representative pointers across all families.
    let dangling: &[(&str, &str, &str)] = &[
        ("campaign.json", "entry", "world"),
        ("campaign.json", "entry", "area"),
        ("campaign.json", "entry", "spawn"),
        ("worlds/world.json", "areas", "0"),
        ("areas/start/area.json", "neighbours", "0"),
        ("creatures/creature.json", "faction", ""),
        ("creatures/creature.json", "inventory", "0"),
    ];
    for (file, field, index) in dangling {
        let (file, field, index): (&str, &str, &str) = (*file, *field, *index);
        let mut files = support::fixture_files();
        edit(&mut files, file, |v| {
            if index.is_empty() {
                v[field] = json!(id(9000));
            } else if let Ok(i) = index.parse::<usize>() {
                if v[field].as_array().is_some_and(|a| a.is_empty()) {
                    v[field] = json!([id(9000)]);
                } else {
                    v[field][i] = json!(id(9000));
                }
            } else {
                v[field][index] = json!(id(9000));
            }
        });
        let pointer = if index.is_empty() {
            format!("/{field}")
        } else {
            format!("/{field}/{index}")
        };
        assert!(
            has(&files, file, &pointer, DiagnosticCode::DanglingReference),
            "{file}{pointer}"
        );
    }
    // Wrong kinds reuse existing ids of an unexpected category.
    let mut files = support::fixture_files();
    edit(&mut files, "worlds/world.json", |v| {
        v["areas"] = json!([id(4)])
    });
    assert!(has(
        &files,
        "worlds/world.json",
        "/areas/0",
        DiagnosticCode::WrongReferenceKind
    ));
    let mut files = support::fixture_files();
    edit(&mut files, "creatures/creature.json", |v| {
        v["inventory"] = json!([id(3)])
    });
    assert!(has(
        &files,
        "creatures/creature.json",
        "/inventory/0",
        DiagnosticCode::WrongReferenceKind
    ));
    // Foreign local children: a second dialogue owns node 9001, the first points at it.
    let mut campaign = valid_loaded();
    campaign.documents.insert(
        path("dialogue/other.json"),
        Document::Dialogue(Dialogue {
            id: id(9301),
            slug: "other".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: id(9002),
            nodes: vec![DialogueNode {
                id: id(9001),
                body: DialogueBody::End,
            }],
        }),
    );
    // Entry is the jump below, so both nodes stay reachable and clean.
    let Document::Dialogue(first) = campaign
        .documents
        .get_mut(&path("dialogue/other.json"))
        .unwrap()
    else {
        panic!("dialogue doc")
    };
    first.nodes.push(DialogueNode {
        id: id(9002),
        body: DialogueBody::Jump { target: id(9001) },
    });
    // Self-owned jump is clean; retarget across dialogues is foreign.
    assert_eq!(validate(&campaign), Vec::new());
    campaign.documents.insert(
        path("dialogue/third.json"),
        Document::Dialogue(Dialogue {
            id: id(9302),
            slug: "third".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: id(9003),
            nodes: vec![DialogueNode {
                id: id(9003),
                body: DialogueBody::Jump { target: id(9001) },
            }],
        }),
    );
    let diagnostics = validate(&campaign);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, DiagnosticCode::ForeignReference);
    assert_eq!(diagnostics[0].pointer, "/nodes/0/body/target");
    // Pointer escaping covers hostile map keys exactly once.
    let mut campaign = valid_loaded();
    let Document::Placements(placements) = campaign
        .documents
        .get_mut(&path("areas/start/placements.json"))
        .unwrap()
    else {
        panic!("placements doc")
    };
    placements.placements[0]
        .overrides
        .insert("a/b~c".into(), DataValue::ObjectRef(id(9000)));
    let diagnostics = validate(&campaign);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, DiagnosticCode::DanglingReference);
    assert_eq!(
        diagnostics[0].pointer,
        "/placements/0/overrides/a~1b~0c/value"
    );
}

#[test]
fn recursive_object_refs_are_checked_in_every_value_context() {
    fn nested() -> DataValue {
        DataValue::Map(BTreeMap::from([(
            "l0".into(),
            DataValue::List(vec![DataValue::Map(BTreeMap::from([(
                "l1".into(),
                DataValue::ObjectRef(id(9000)),
            )]))]),
        )]))
    }
    // Placement overrides, world variables, branch cases, action args,
    // script args, graph-call args, dialogue/quest actions, asset imports.
    let mut campaign = valid_loaded();
    let Document::Placements(placements) = campaign
        .documents
        .get_mut(&path("areas/start/placements.json"))
        .unwrap()
    else {
        panic!("placements doc")
    };
    placements.placements[0]
        .overrides
        .insert("deep".into(), nested());
    let Document::World(world) = campaign
        .documents
        .get_mut(&path("worlds/world.json"))
        .unwrap()
    else {
        panic!("world doc")
    };
    world.variables.push(VarDecl {
        name: "v".into(),
        value_type: ValueType::Map,
        default: nested(),
        scope: VariableScope::World,
    });
    let Document::Triggers(triggers) = campaign
        .documents
        .get_mut(&path("areas/start/triggers.json"))
        .unwrap()
    else {
        panic!("triggers doc")
    };
    triggers.graphs[0].nodes[0].body = NodeBody::Branch {
        expr: "x".into(),
        cases: vec![nested()],
    };
    triggers.graphs[0].nodes.push(Node {
        id: id(9401),
        body: NodeBody::Action {
            call: ActionCall {
                action_id: "act".into(),
                args: BTreeMap::from([("arg".into(), nested())]),
            },
        },
    });
    triggers.graphs[0].nodes.push(Node {
        id: id(9402),
        body: NodeBody::CallScript {
            script_id: "s".into(),
            args: BTreeMap::from([("arg".into(), nested())]),
        },
    });
    triggers.graphs[0].nodes.push(Node {
        id: id(9403),
        body: NodeBody::CallGraph {
            graph_id: id(6),
            args: BTreeMap::from([("arg".into(), nested())]),
        },
    });
    // Chain every new node behind the start so reachability stays silent and
    // only the six nested references fail. Node 7 now holds the branch body,
    // so its outbound edge uses a valid zero-based case port.
    triggers.graphs[0].edges.push(Edge {
        from: id(7),
        port: Port::Case { index: 0 },
        to: id(9401),
    });
    for (from, to) in [(id(9401), id(9402)), (id(9402), id(9403))] {
        triggers.graphs[0].edges.push(Edge {
            from,
            port: Port::Next,
            to,
        });
    }
    let diagnostics = validate(&campaign);
    assert_eq!(diagnostics.len(), 6);
    assert!(
        diagnostics
            .iter()
            .all(|d| d.code == DiagnosticCode::DanglingReference),
        "{diagnostics:?}"
    );
    let pointers: Vec<&str> = diagnostics.iter().map(|d| d.pointer.as_str()).collect();
    assert!(pointers.contains(&"/placements/0/overrides/deep/l0/0/l1/value"));
    assert!(pointers.contains(&"/variables/0/default/l0/0/l1/value"));
    assert!(pointers.contains(&"/graphs/0/nodes/0/body/cases/0/l0/0/l1/value"));
    assert!(pointers.contains(&"/graphs/0/nodes/1/body/call/args/arg/l0/0/l1/value"));
    assert!(pointers.contains(&"/graphs/0/nodes/2/body/args/arg/l0/0/l1/value"));
    assert!(pointers.contains(&"/graphs/0/nodes/3/body/args/arg/l0/0/l1/value"));
    // Asset import settings participate with no double emission.
    let mut campaign = valid_loaded();
    let Document::AssetsLock(assets) = campaign
        .documents
        .get_mut(&path("assets/assets.lock"))
        .unwrap()
    else {
        panic!("assets doc")
    };
    assets.assets.insert(
        path("assets/models/dragon.glb"),
        AssetRecord {
            hash: Digest::from_bytes([7; 32]),
            import: BTreeMap::from([("ref".into(), DataValue::ObjectRef(id(9000)))]),
        },
    );
    let diagnostics = validate(&campaign);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, DiagnosticCode::DanglingReference);
    assert_eq!(
        diagnostics[0].pointer,
        "/assets/assets~1models~1dragon.glb/import/ref/value"
    );
}

#[test]
fn graph_ports_case_bounds_duplicates_and_reachability() {
    fn graph_with(body: NodeBody, port: Port) -> (EventGraph, Ulid, Ulid) {
        let graph = id(9501);
        let node = id(9502);
        (
            EventGraph {
                id: graph,
                slug: "g".into(),
                name: "fixture.creature".into(),
                note: None,
                entry: Trigger::AreaEnter,
                start: node,
                nodes: vec![Node { id: node, body }],
                edges: vec![Edge {
                    from: node,
                    port,
                    to: node,
                }],
                locals: Vec::new(),
            },
            graph,
            node,
        )
    }
    fn check(body: NodeBody, port: Port) -> Vec<Diagnostic> {
        let (graph, _, _) = graph_with(body, port);
        let mut campaign = valid_loaded();
        campaign
            .documents
            .insert(path("scripts/graphs/t.json"), Document::Graph(graph));
        validate(&campaign)
    }
    // Allowed combinations stay silent (self-loop keeps the node reachable).
    assert_eq!(check(NodeBody::Wait { ticks: 0 }, Port::Next), Vec::new());
    assert_eq!(
        check(NodeBody::Condition { expr: "x".into() }, Port::True),
        Vec::new()
    );
    assert_eq!(
        check(
            NodeBody::Branch {
                expr: "x".into(),
                cases: vec![DataValue::Bool(true)]
            },
            Port::Case { index: 0 }
        ),
        Vec::new()
    );
    // Mismatches and out-of-range cases point at the port.
    for (body, port) in [
        (NodeBody::Wait { ticks: 0 }, Port::True),
        (NodeBody::Condition { expr: "x".into() }, Port::Next),
        (
            NodeBody::Branch {
                expr: "x".into(),
                cases: vec![DataValue::Bool(true)],
            },
            Port::Case { index: 1 },
        ),
        (
            NodeBody::Branch {
                expr: "x".into(),
                cases: Vec::new(),
            },
            Port::Case { index: 0 },
        ),
        (NodeBody::Sequence { nodes: Vec::new() }, Port::False),
    ] {
        let diagnostics = check(body, port);
        assert_eq!(
            only_codes(&diagnostics),
            vec![DiagnosticCode::GraphPortMismatch]
        );
        assert_eq!(diagnostics[0].pointer, "/edges/0/port");
    }
    // Duplicate (from, port) pairs fail on the later edge only.
    let (mut graph, _, node) = graph_with(NodeBody::Wait { ticks: 0 }, Port::Next);
    graph.edges.push(Edge {
        from: node,
        port: Port::Next,
        to: node,
    });
    let mut campaign = valid_loaded();
    campaign
        .documents
        .insert(path("scripts/graphs/t.json"), Document::Graph(graph));
    let diagnostics = validate(&campaign);
    assert_eq!(
        only_codes(&diagnostics),
        vec![DiagnosticCode::DuplicateGraphPort]
    );
    assert_eq!(diagnostics[0].pointer, "/edges/1/port");
    // Unreachable standalone and embedded nodes are positioned at the object.
    let (mut graph, _, _) = graph_with(NodeBody::Wait { ticks: 0 }, Port::Next);
    graph.nodes.push(Node {
        id: id(9503),
        body: NodeBody::Wait { ticks: 0 },
    });
    let mut campaign = valid_loaded();
    campaign
        .documents
        .insert(path("scripts/graphs/t.json"), Document::Graph(graph));
    let diagnostics = validate(&campaign);
    assert_eq!(
        only_codes(&diagnostics),
        vec![DiagnosticCode::UnreachableNode]
    );
    assert_eq!(diagnostics[0].pointer, "/nodes/1");
    // The same reachability rule applies inside an embedded triggers graph.
    let mut campaign = valid_loaded();
    let Document::Triggers(triggers) = campaign
        .documents
        .get_mut(&path("areas/start/triggers.json"))
        .unwrap()
    else {
        panic!("triggers doc")
    };
    triggers.graphs[0].nodes.push(Node {
        id: id(9504),
        body: NodeBody::Wait { ticks: 0 },
    });
    let diagnostics = validate(&campaign);
    assert_eq!(
        only_codes(&diagnostics),
        vec![DiagnosticCode::UnreachableNode]
    );
    assert_eq!(
        diagnostics[0].file.as_ref().map(SourcePath::as_str),
        Some("areas/start/triggers.json")
    );
    assert_eq!(diagnostics[0].pointer, "/graphs/0/nodes/1");
    // Sequence children count as reachable; an invalid start suppresses
    // reachability instead of cascading.
    let graph_id = id(9511);
    let (first, second, third) = (id(9512), id(9513), id(9514));
    let mut campaign = valid_loaded();
    campaign.documents.insert(
        path("scripts/graphs/t.json"),
        Document::Graph(EventGraph {
            id: graph_id,
            slug: "g".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: Trigger::AreaEnter,
            start: first,
            nodes: vec![
                Node {
                    id: first,
                    body: NodeBody::Sequence {
                        nodes: vec![second],
                    },
                },
                Node {
                    id: second,
                    body: NodeBody::Wait { ticks: 0 },
                },
                Node {
                    id: third,
                    body: NodeBody::Wait { ticks: 0 },
                },
            ],
            edges: Vec::new(),
            locals: Vec::new(),
        }),
    );
    let diagnostics = validate(&campaign);
    assert_eq!(
        only_codes(&diagnostics),
        vec![DiagnosticCode::UnreachableNode]
    );
    assert_eq!(diagnostics[0].pointer, "/nodes/2");
    let mut campaign = valid_loaded();
    campaign.documents.insert(
        path("scripts/graphs/t.json"),
        Document::Graph(EventGraph {
            id: graph_id,
            slug: "g".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: Trigger::AreaEnter,
            start: id(9000),
            nodes: vec![Node {
                id: first,
                body: NodeBody::Wait { ticks: 0 },
            }],
            edges: Vec::new(),
            locals: Vec::new(),
        }),
    );
    let diagnostics = validate(&campaign);
    assert_eq!(
        only_codes(&diagnostics),
        vec![DiagnosticCode::DanglingReference]
    );
    assert_eq!(diagnostics[0].pointer, "/start");
}

#[test]
fn dialogue_reachability_covers_all_bodies_and_entry_suppression() {
    // A chain through every body variant with Link leaving stays silent.
    let (a, b, c, d, e) = (id(9601), id(9602), id(9603), id(9604), id(9605));
    let mut campaign = valid_loaded();
    campaign.documents.insert(
        path("dialogue/chain.json"),
        Document::Dialogue(Dialogue {
            id: id(9600),
            slug: "chain".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: a,
            nodes: vec![
                DialogueNode {
                    id: a,
                    body: DialogueBody::NpcLine {
                        speaker: id(4),
                        text_key: "fixture.creature".into(),
                        conditions: Vec::new(),
                        on_enter: Vec::new(),
                        next: Some(b),
                    },
                },
                DialogueNode {
                    id: b,
                    body: DialogueBody::PlayerChoice {
                        text_key: "fixture.creature".into(),
                        conditions: Vec::new(),
                        on_select: Vec::new(),
                        next: c,
                    },
                },
                DialogueNode {
                    id: c,
                    body: DialogueBody::Jump { target: d },
                },
                DialogueNode {
                    id: d,
                    body: DialogueBody::Link { target: id(9600) },
                },
                DialogueNode {
                    id: e,
                    body: DialogueBody::End,
                },
            ],
        }),
    );
    // Node e is unreferenced; the link target is the dialogue itself (dangling
    // would be a second finding, so point the link at this dialogue: valid).
    let diagnostics = validate(&campaign);
    assert_eq!(
        only_codes(&diagnostics),
        vec![DiagnosticCode::UnreachableNode]
    );
    assert_eq!(diagnostics[0].pointer, "/nodes/4");
    // An invalid entry suppresses reachability: exactly one diagnostic.
    let mut campaign = valid_loaded();
    campaign.documents.insert(
        path("dialogue/chain.json"),
        Document::Dialogue(Dialogue {
            id: id(9600),
            slug: "chain".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: id(9000),
            nodes: vec![DialogueNode {
                id: a,
                body: DialogueBody::End,
            }],
        }),
    );
    let diagnostics = validate(&campaign);
    assert_eq!(
        only_codes(&diagnostics),
        vec![DiagnosticCode::DanglingReference]
    );
}

#[test]
fn quest_completion_terminal_cycles_and_invalid_entries() {
    // Reachable terminal through a cycle stays silent.
    let (quest, s0, s1) = (id(9700), id(9701), id(9702));
    let mut campaign = valid_loaded();
    campaign.documents.insert(
        path("quests/loop.json"),
        Document::Quest(Quest {
            id: quest,
            slug: "loop".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: s0,
            states: vec![
                QuestState {
                    id: s0,
                    name: "fixture.creature".into(),
                    terminal: false,
                    on_enter: Vec::new(),
                    transitions: vec![QuestTransition {
                        condition: "x".into(),
                        target: s1,
                    }],
                },
                QuestState {
                    id: s1,
                    name: "fixture.creature".into(),
                    terminal: true,
                    on_enter: Vec::new(),
                    transitions: vec![QuestTransition {
                        condition: "x".into(),
                        target: s0,
                    }],
                },
            ],
        }),
    );
    assert_eq!(validate(&campaign), Vec::new());
    // A cycle with no terminal state fails once at the entry.
    let mut campaign = valid_loaded();
    campaign.documents.insert(
        path("quests/loop.json"),
        Document::Quest(Quest {
            id: quest,
            slug: "loop".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: s0,
            states: vec![QuestState {
                id: s0,
                name: "fixture.creature".into(),
                terminal: false,
                on_enter: Vec::new(),
                transitions: vec![QuestTransition {
                    condition: "x".into(),
                    target: s0,
                }],
            }],
        }),
    );
    let diagnostics = validate(&campaign);
    assert_eq!(
        only_codes(&diagnostics),
        vec![DiagnosticCode::QuestNoCompletionPath]
    );
    assert_eq!(diagnostics[0].pointer, "/entry");
    // A dangling transition keeps its own diagnostic and still leaves the
    // quest without a completion path.
    let mut campaign = valid_loaded();
    campaign.documents.insert(
        path("quests/loop.json"),
        Document::Quest(Quest {
            id: quest,
            slug: "loop".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: s0,
            states: vec![QuestState {
                id: s0,
                name: "fixture.creature".into(),
                terminal: false,
                on_enter: Vec::new(),
                transitions: vec![QuestTransition {
                    condition: "x".into(),
                    target: id(9000),
                }],
            }],
        }),
    );
    let diagnostics = validate(&campaign);
    // Sorted by pointer, the entry completion finding precedes the transition.
    assert_eq!(
        only_codes(&diagnostics),
        vec![
            DiagnosticCode::QuestNoCompletionPath,
            DiagnosticCode::DanglingReference
        ]
    );
    // An invalid entry suppresses the completion diagnostic.
    let mut campaign = valid_loaded();
    campaign.documents.insert(
        path("quests/loop.json"),
        Document::Quest(Quest {
            id: quest,
            slug: "loop".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: id(9000),
            states: vec![QuestState {
                id: s0,
                name: "fixture.creature".into(),
                terminal: false,
                on_enter: Vec::new(),
                transitions: Vec::new(),
            }],
        }),
    );
    let diagnostics = validate(&campaign);
    assert_eq!(
        only_codes(&diagnostics),
        vec![DiagnosticCode::DanglingReference]
    );
}

#[test]
fn locale_coverage_multiple_tables_no_tables_and_unused_strings() {
    // A second table with partial coverage fails per missing (reference, locale).
    let mut files = support::fixture_files();
    edit(&mut files, "locale/en.json", |v| {
        v["strings"]["fixture.extra"] = json!("Extra");
    });
    files.insert(
        path("locale/fr.json"),
        canonical_json(&json!({
            "schema": "crpg.locale/1",
            "campaign": id(1),
            "locale": "fr",
            "strings": {"fixture.campaign": "Campagne"}
        }))
        .unwrap(),
    );
    let diagnostics = validate(&load_campaign(&files, &engine()).unwrap());
    assert!(!diagnostics.is_empty());
    assert!(
        diagnostics
            .iter()
            .all(|d| d.code == DiagnosticCode::MissingLocaleKey),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("\"fr\"") && d.pointer == "/name"),
        "{diagnostics:?}"
    );
    // With no locale tables, each unique referring field fails once.
    let mut files = support::fixture_files();
    files.remove(&path("locale/en.json"));
    let diagnostics = validate(&load_campaign(&files, &engine()).unwrap());
    assert_eq!(diagnostics.len(), 6);
    assert!(diagnostics
        .iter()
        .all(|d| d.code == DiagnosticCode::MissingLocaleKey));
    assert!(diagnostics
        .iter()
        .all(|d| d.message.contains("no locale table")));
    // Unused strings never fail.
    let mut files = support::fixture_files();
    edit(&mut files, "locale/en.json", |v| {
        v["strings"]["fixture.unused"] = json!("Unused");
    });
    assert_eq!(
        validate(&load_campaign(&files, &engine()).unwrap()),
        Vec::new()
    );
    // Asset keys match exactly against the lock, without filesystem access.
    let mut files = support::fixture_files();
    edit(&mut files, "areas/start/area.json", |v| {
        v["ambience"] = json!("assets/song.ogg")
    });
    assert!(has(
        &files,
        "areas/start/area.json",
        "/ambience",
        DiagnosticCode::MissingAsset
    ));
    // Adding the lock entry clears the finding; typed mutation avoids the
    // lock-digest coupling that file edits would trip at load time.
    let mut campaign = valid_loaded();
    let Document::Area(area) = campaign
        .documents
        .get_mut(&path("areas/start/area.json"))
        .unwrap()
    else {
        panic!("area doc")
    };
    area.ambience = Some("assets/song.ogg".into());
    let Document::AssetsLock(lock) = campaign
        .documents
        .get_mut(&path("assets/assets.lock"))
        .unwrap()
    else {
        panic!("assets doc")
    };
    lock.assets.insert(
        path("assets/song.ogg"),
        AssetRecord {
            hash: Digest::from_bytes([3; 32]),
            import: BTreeMap::new(),
        },
    );
    assert_eq!(validate(&campaign), Vec::new());
}

#[test]
fn every_value_type_pair_matches_or_fails_at_default() {
    fn decl(value_type: ValueType, default: DataValue) -> VarDecl {
        VarDecl {
            name: "v".into(),
            value_type,
            default,
            scope: VariableScope::Campaign,
        }
    }
    let good = vec![
        decl(ValueType::Bool, DataValue::Bool(true)),
        decl(ValueType::Integer, DataValue::Integer(i64::MIN)),
        decl(ValueType::Unsigned, DataValue::Unsigned(u64::MAX)),
        decl(
            ValueType::Fixed,
            DataValue::Fixed(crpg_core::Fx16_16::from_int(-3)),
        ),
        decl(ValueType::Text, DataValue::Text("t".into())),
        decl(ValueType::ObjectRef, DataValue::ObjectRef(id(4))),
        decl(
            ValueType::List,
            DataValue::List(vec![DataValue::Bool(false), DataValue::Integer(1)]),
        ),
        decl(
            ValueType::Map,
            DataValue::Map(BTreeMap::from([("k".into(), DataValue::Unsigned(2))])),
        ),
    ];
    let mut campaign = valid_loaded();
    let Document::Variables(vars) = campaign
        .documents
        .get_mut(&path("variables/campaign_state.json"))
        .unwrap()
    else {
        panic!("variables doc")
    };
    vars.variables = good;
    assert_eq!(validate(&campaign), Vec::new());
    // Every cross pairing fails at its own default pointer.
    let bad = [
        (ValueType::Bool, DataValue::Integer(0)),
        (ValueType::Integer, DataValue::Bool(false)),
        (ValueType::Unsigned, DataValue::Integer(0)),
        (ValueType::Fixed, DataValue::Text("x".into())),
        (ValueType::Text, DataValue::List(Vec::new())),
        (ValueType::ObjectRef, DataValue::Map(BTreeMap::new())),
        (ValueType::List, DataValue::Map(BTreeMap::new())),
        (ValueType::Map, DataValue::List(Vec::new())),
    ];
    let mut campaign = valid_loaded();
    let Document::Variables(vars) = campaign
        .documents
        .get_mut(&path("variables/campaign_state.json"))
        .unwrap()
    else {
        panic!("variables doc")
    };
    vars.variables = bad
        .into_iter()
        .enumerate()
        .map(|(i, (value_type, default))| VarDecl {
            name: format!("v{i}"),
            value_type,
            default,
            scope: VariableScope::Campaign,
        })
        .collect();
    let diagnostics = validate(&campaign);
    assert_eq!(diagnostics.len(), 8);
    assert!(diagnostics
        .iter()
        .all(|d| d.code == DiagnosticCode::ValueTypeMismatch));
    for (i, diagnostic) in diagnostics.iter().enumerate() {
        assert_eq!(diagnostic.pointer, format!("/variables/{i}/default"));
    }
}

#[test]
fn every_data_error_maps_to_its_structural_code() {
    let package: PackageId = "a".parse().unwrap();
    let errors = [
        (
            DataError::InvalidPackageId { value: "A".into() },
            DiagnosticCode::InvalidPackageId,
            None,
        ),
        (
            DataError::InvalidPath { value: "x".into() },
            DiagnosticCode::InvalidPath,
            None,
        ),
        (
            DataError::InvalidDigest { value: "x".into() },
            DiagnosticCode::InvalidDigest,
            None,
        ),
        (
            DataError::Malformed {
                path: Some(path("worlds/world.json")),
                message: "m".into(),
            },
            DiagnosticCode::Malformed,
            Some("worlds/world.json"),
        ),
        (
            DataError::UnsupportedSchema {
                path: None,
                found: "x/9".into(),
            },
            DiagnosticCode::UnsupportedSchema,
            None,
        ),
        (
            DataError::Layout {
                path: None,
                message: "m".into(),
            },
            DiagnosticCode::Layout,
            None,
        ),
        (
            DataError::EngineIncompatible {
                required: ">=9".into(),
                actual: "0.1.0".into(),
            },
            DiagnosticCode::EngineIncompatible,
            None,
        ),
        (
            DataError::DuplicateId {
                id: id(1),
                first: path("a.json"),
                second: path("b.json"),
            },
            DiagnosticCode::DuplicateId,
            Some("b.json"),
        ),
        (
            DataError::PackageKindConflict {
                package: package.clone(),
            },
            DiagnosticCode::PackageKindConflict,
            None,
        ),
        (
            DataError::CandidateConflict {
                package: package.clone(),
                version: "1.0.0".into(),
            },
            DiagnosticCode::CandidateConflict,
            None,
        ),
        (
            DataError::UnresolvedPackage {
                package,
                requirements: vec!["^1".into()],
            },
            DiagnosticCode::UnresolvedPackage,
            None,
        ),
        (
            DataError::InvalidLock {
                message: "m".into(),
            },
            DiagnosticCode::InvalidLock,
            None,
        ),
        (
            DataError::AssetsLockMismatch {
                expected: Digest::from_bytes([1; 32]),
                actual: Digest::from_bytes([2; 32]),
            },
            DiagnosticCode::AssetsLockMismatch,
            None,
        ),
    ];
    assert_eq!(errors.len(), 13);
    for (error, code, file) in &errors {
        let diagnostic = diagnostic_for_data_error(error);
        assert_eq!(diagnostic.code, *code);
        assert_eq!(diagnostic.severity, Severity::Error);
        assert_eq!(diagnostic.file.as_ref().map(SourcePath::as_str), *file);
        assert_eq!(diagnostic.suggested_fix, None);
        assert!(!diagnostic.message.contains("Some("));
        assert!(!diagnostic.message.contains("None"));
    }
    // validate_files returns one load diagnostic or the semantic vector.
    let mut files = support::fixture_files();
    files.insert(path("worlds/world.json"), b"invalid".to_vec());
    let diagnostics = validate_files(&files, &engine());
    assert_eq!(only_codes(&diagnostics), vec![DiagnosticCode::Malformed]);
    assert_eq!(
        diagnostics[0].file.as_ref().map(SourcePath::as_str),
        Some("worlds/world.json")
    );
    assert_eq!(
        validate_files(&support::fixture_files(), &engine()),
        Vec::new()
    );
}

#[test]
fn document_path_classifier_shares_the_loader_layout() {
    for accepted in [
        "campaign.json",
        "campaign.lock",
        "assets/assets.lock",
        "variables/campaign_state.json",
        "worlds/world.json",
        "worlds/nested/deep.json",
        "creatures/goblin.json",
        "items/sword.json",
        "dialogue/intro.json",
        "quests/main.json",
        "factions/guild.json",
        "scripts/graphs/door.json",
        "scripts/graphs/nested/door.json",
        "areas/start/area.json",
        "areas/start/placements.json",
        "areas/start/triggers.json",
        "areas/deep/nest/area.json",
        "locale/en.json",
    ] {
        assert_eq!(
            campaign_document_path(accepted)
                .unwrap()
                .map(|p| p.as_str().to_string()),
            Some(accepted.into()),
            "{accepted}"
        );
    }
    // Every checked-in valid fixture path classifies without error.
    for file in support::fixture_files().keys() {
        assert!(
            campaign_document_path(file.as_str()).unwrap().is_some(),
            "{file}"
        );
    }
    for ignored in [
        "assets/models/dragon.glb",
        "schemas/creature.schema.json",
        "scripts/lua/greet.lua",
        "tests/smoke.replay",
        "build/output.bin",
        ".gitattributes",
        "unknown.json",
        "locale/nested/en.json",
        "areas/area.json",
    ] {
        assert_eq!(campaign_document_path(ignored).unwrap(), None, "{ignored}");
    }
    for ignored in ["areas//area.json", "locale/.json"] {
        assert_eq!(campaign_document_path(ignored).unwrap(), None, "{ignored}");
    }
    for invalid in ["worlds/a b.json", "areas/a b/area.json", "locale/a b.json"] {
        assert!(
            matches!(
                campaign_document_path(invalid),
                Err(DataError::InvalidPath { .. })
            ),
            "{invalid}"
        );
    }
    // Classification is case-sensitive throughout.
    for upper in [
        "Campaign.json",
        "WORLDS/world.json",
        "LOCALE/en.json",
        "Areas/start/area.json",
    ] {
        assert_eq!(campaign_document_path(upper).unwrap(), None, "{upper}");
    }
}

#[test]
fn diagnostic_wire_shape_spellings_and_display_are_pinned() {
    let full = Diagnostic {
        file: Some(path("areas/start/area.json")),
        pointer: "/neighbours/0".into(),
        severity: Severity::Error,
        code: DiagnosticCode::DanglingReference,
        message: "dangling area neighbour reference to 0000000000000000000000C008".into(),
        suggested_fix: Some("point at an existing object id or create the target".into()),
    };
    assert_eq!(
        full.to_string(),
        "areas/start/area.json/neighbours/0: error[dangling_reference]: dangling area neighbour reference to 0000000000000000000000C008; suggested fix: point at an existing object id or create the target"
    );
    let bare = Diagnostic {
        file: None,
        pointer: String::new(),
        severity: Severity::Warning,
        code: DiagnosticCode::Io,
        message: "cannot list <campaign-root>: not_found".into(),
        suggested_fix: None,
    };
    assert_eq!(
        bare.to_string(),
        "<campaign>: warning[io]: cannot list <campaign-root>: not_found"
    );
    assert_eq!(Severity::Error.to_string(), "error");
    let codes = [
        (DiagnosticCode::Io, "io"),
        (DiagnosticCode::InvalidPackageId, "invalid_package_id"),
        (DiagnosticCode::InvalidPath, "invalid_path"),
        (DiagnosticCode::InvalidDigest, "invalid_digest"),
        (DiagnosticCode::Malformed, "malformed"),
        (DiagnosticCode::UnsupportedSchema, "unsupported_schema"),
        (DiagnosticCode::Layout, "layout"),
        (DiagnosticCode::EngineIncompatible, "engine_incompatible"),
        (DiagnosticCode::DuplicateId, "duplicate_id"),
        (DiagnosticCode::PackageKindConflict, "package_kind_conflict"),
        (DiagnosticCode::CandidateConflict, "candidate_conflict"),
        (DiagnosticCode::UnresolvedPackage, "unresolved_package"),
        (DiagnosticCode::InvalidLock, "invalid_lock"),
        (DiagnosticCode::AssetsLockMismatch, "assets_lock_mismatch"),
        (DiagnosticCode::DuplicateSlug, "duplicate_slug"),
        (DiagnosticCode::DanglingReference, "dangling_reference"),
        (DiagnosticCode::WrongReferenceKind, "wrong_reference_kind"),
        (DiagnosticCode::ForeignReference, "foreign_reference"),
        (
            DiagnosticCode::AggregateOwnerMismatch,
            "aggregate_owner_mismatch",
        ),
        (DiagnosticCode::GraphPortMismatch, "graph_port_mismatch"),
        (DiagnosticCode::DuplicateGraphPort, "duplicate_graph_port"),
        (DiagnosticCode::UnreachableNode, "unreachable_node"),
        (
            DiagnosticCode::QuestNoCompletionPath,
            "quest_no_completion_path",
        ),
        (DiagnosticCode::MissingAsset, "missing_asset"),
        (DiagnosticCode::MissingLocaleKey, "missing_locale_key"),
        (DiagnosticCode::ValueTypeMismatch, "value_type_mismatch"),
    ];
    assert_eq!(codes.len(), 26);
    for (code, text) in codes {
        assert_eq!(code.as_str(), text);
        assert_eq!(code.to_string(), text);
    }
    // Exactly six keys with explicit nulls, round-tripping byte-identically.
    let value = serde_json::to_value(&bare).unwrap();
    let object = value.as_object().unwrap();
    assert_eq!(object.len(), 6);
    assert!(object["file"].is_null());
    assert!(object["suggested_fix"].is_null());
    assert_eq!(object["severity"], json!("warning"));
    assert_eq!(object["code"], json!("io"));
    let bytes = canonical_json(&bare).unwrap();
    let decoded: Diagnostic = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, bare);
    let decoded_full: Diagnostic = serde_json::from_slice(&canonical_json(&full).unwrap()).unwrap();
    assert_eq!(decoded_full, full);
}
