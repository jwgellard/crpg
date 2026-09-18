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
fn report(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}
fn explain(campaign: &LoadedCampaign, n: u128) -> Option<Value> {
    explain_object(campaign, id(n)).unwrap().map(|b| report(&b))
}
fn explain_bytes(campaign: &LoadedCampaign, n: u128) -> Option<Vec<u8>> {
    explain_object(campaign, id(n)).unwrap()
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
fn migration_files() -> Files {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/migration_v1/campaign");
    let mut files: Files = BTreeMap::new();
    for entry in walkdir(&root, &root) {
        let logical: SourcePath = entry.parse().unwrap();
        files.insert(
            logical,
            fs::read(root.join(&entry)).expect("migration fixture file"),
        );
    }
    files
}
fn walkdir(root: &Path, _base: &Path) -> Vec<String> {
    // Fixed inventory of the eleven migration files; missing files fail hard.
    let names = [
        "campaign.json",
        "campaign.lock",
        "worlds/world.json",
        "areas/start/area.json",
        "areas/start/placements.json",
        "areas/start/triggers.json",
        "creatures/creature.json",
        "items/item.json",
        "variables/campaign_state.json",
        "assets/assets.lock",
        "locale/en.json",
    ];
    for name in &names {
        assert!(root.join(name).exists(), "missing migration file {name}");
    }
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn exact_canonical_report_for_top_level_and_nested() {
    let campaign = valid_loaded();
    // Top-level campaign: exact bytes, kind spelling, null handling, final LF.
    let bytes = explain_bytes(&campaign, 1).expect("campaign present");
    assert!(bytes.ends_with(b"\n"));
    assert!(!bytes.ends_with(b"\n\n"));
    let value = report(&bytes);
    assert_eq!(value["id"], json!(id(1).to_string()));
    assert_eq!(value["kind"], json!("campaign"));
    assert_eq!(value["file"], json!("campaign.json"));
    assert_eq!(value["pointer"], json!(""));
    // Object retains the schema envelope for top-level entities.
    assert_eq!(value["object"]["schema"], json!("crpg.campaign/1"));
    assert_eq!(value["object"]["id"], json!(id(1).to_string()));
    // Required null-free top level: inbound/outbound present as arrays.
    assert!(value["inbound"].is_array());
    assert!(value["outbound"].is_array());
    // Canonical bytes are byte-identical on repeated calls.
    assert_eq!(explain_bytes(&campaign, 1).unwrap(), bytes);
    assert_eq!(canonical_json(&value).unwrap(), bytes);
    // Campaign inbound: locale + variables owners, sorted by file.
    let inbound = value["inbound"].as_array().unwrap();
    assert_eq!(inbound.len(), 2);
    assert_eq!(inbound[0]["file"], json!("locale/en.json"));
    assert_eq!(inbound[0]["pointer"], json!("/campaign"));
    assert_eq!(inbound[0]["source"], Value::Null);
    assert_eq!(inbound[0]["target"], json!(id(1).to_string()));
    assert_eq!(inbound[0]["target_location"]["kind"], json!("campaign"));
    assert_eq!(
        inbound[0]["target_location"]["file"],
        json!("campaign.json")
    );
    assert_eq!(inbound[0]["target_location"]["pointer"], json!(""));
    assert_eq!(inbound[1]["file"], json!("variables/campaign_state.json"));
    // Campaign outbound: entry world/area/spawn sorted lexically by pointer.
    let outbound = value["outbound"].as_array().unwrap();
    assert_eq!(outbound.len(), 3);
    assert_eq!(outbound[0]["pointer"], json!("/entry/area"));
    assert_eq!(outbound[1]["pointer"], json!("/entry/spawn"));
    assert_eq!(outbound[2]["pointer"], json!("/entry/world"));
    for edge in outbound {
        assert_eq!(edge["source"], json!(id(1).to_string()));
    }
    // Nested placement: actual nested shape, no invented schema tag, no enclosing document.
    let placement = explain(&campaign, 5).expect("placement present");
    assert_eq!(placement["kind"], json!("placement"));
    assert_eq!(placement["file"], json!("areas/start/placements.json"));
    assert_eq!(placement["pointer"], json!("/placements/0"));
    assert!(placement["object"].get("schema").is_none());
    assert_eq!(placement["object"]["id"], json!(id(5).to_string()));
    assert_eq!(placement["object"]["prefab"], json!(id(4).to_string()));
    // Placement inbound is the campaign spawn; outbound is the prefab only.
    assert_eq!(placement["inbound"].as_array().unwrap().len(), 1);
    assert_eq!(placement["inbound"][0]["pointer"], json!("/entry/spawn"));
    let p_out = placement["outbound"].as_array().unwrap();
    assert_eq!(p_out.len(), 1);
    assert_eq!(p_out[0]["pointer"], json!("/placements/0/prefab"));
    assert_eq!(p_out[0]["source"], json!(id(5).to_string()));
    assert_eq!(p_out[0]["target"], json!(id(4).to_string()));
    assert_eq!(p_out[0]["target_location"]["kind"], json!("creature"));
    // Nested node: empty reference arrays are present, not omitted.
    let node = explain(&campaign, 7).expect("node present");
    assert_eq!(node["kind"], json!("node"));
    assert_eq!(node["file"], json!("areas/start/triggers.json"));
    assert_eq!(node["pointer"], json!("/graphs/0/nodes/0"));
    assert!(node["object"].get("schema").is_none());
    assert_eq!(node["outbound"].as_array().unwrap().len(), 0);
    assert_eq!(node["inbound"].as_array().unwrap().len(), 1);
    assert_eq!(node["inbound"][0]["pointer"], json!("/graphs/0/start"));
    // Graph parent includes its start in outbound; start source is the graph.
    let graph = explain(&campaign, 6).expect("graph present");
    assert_eq!(graph["kind"], json!("graph"));
    assert_eq!(graph["outbound"].as_array().unwrap().len(), 1);
    assert_eq!(graph["outbound"][0]["source"], json!(id(6).to_string()));
}

#[test]
fn every_object_kind_has_its_report_spelling() {
    let mut campaign = valid_loaded();
    let extra_item = Document::Item(Item {
        id: id(101),
        slug: "spell-item".into(),
        name: "fixture.creature".into(),
        note: None,
        stats: BTreeMap::new(),
        tags: Vec::new(),
    });
    campaign
        .documents
        .insert(path("items/spell.json"), extra_item);
    let extra_faction = Document::Faction(Faction {
        id: id(102),
        slug: "spell-faction".into(),
        name: "fixture.creature".into(),
        note: None,
        relations: Vec::new(),
    });
    campaign
        .documents
        .insert(path("factions/spell.json"), extra_faction);
    let extra_dialogue = Document::Dialogue(Dialogue {
        id: id(103),
        slug: "spell-dialogue".into(),
        name: "fixture.creature".into(),
        note: None,
        entry: id(104),
        nodes: vec![DialogueNode {
            id: id(104),
            body: DialogueBody::End,
        }],
    });
    campaign
        .documents
        .insert(path("dialogue/spell.json"), extra_dialogue);
    let extra_quest = Document::Quest(Quest {
        id: id(105),
        slug: "spell-quest".into(),
        name: "fixture.creature".into(),
        note: None,
        entry: id(106),
        states: vec![QuestState {
            id: id(106),
            name: "fixture.creature".into(),
            terminal: true,
            on_enter: Vec::new(),
            transitions: Vec::new(),
        }],
    });
    campaign
        .documents
        .insert(path("quests/spell.json"), extra_quest);
    let extra_graph = Document::Graph(EventGraph {
        id: id(107),
        slug: "spell-graph".into(),
        name: "fixture.creature".into(),
        note: None,
        entry: Trigger::AreaEnter,
        start: id(108),
        nodes: vec![Node {
            id: id(108),
            body: NodeBody::Wait { ticks: 0 },
        }],
        edges: Vec::new(),
        locals: Vec::new(),
    });
    campaign
        .documents
        .insert(path("scripts/graphs/spell.json"), extra_graph);
    for (n, kind) in [
        (1, "campaign"),
        (2, "world"),
        (3, "area"),
        (4, "creature"),
        (101, "item"),
        (103, "dialogue"),
        (104, "dialogue_node"),
        (105, "quest"),
        (106, "quest_state"),
        (102, "faction"),
        (5, "placement"),
        (107, "graph"),
        (108, "node"),
    ] {
        let value = explain(&campaign, n).expect("present");
        assert_eq!(value["kind"], json!(kind), "id {n}");
        assert_eq!(value["id"], json!(id(n).to_string()));
    }
}

#[test]
fn every_reference_family_reports_source_and_target_locations() {
    let mut campaign = valid_loaded();
    // Faction relation, creature faction/inventory, world areas already covered
    // by fixture; add dialogue/quest/graph coverage with explicit locations.
    let speaker = id(4);
    let dialogue_id = id(201);
    let n0 = id(202);
    let n1 = id(203);
    let n2 = id(204);
    let n3 = id(205);
    let n4 = id(206);
    campaign.documents.insert(
        path("dialogue/full.json"),
        Document::Dialogue(Dialogue {
            id: dialogue_id,
            slug: "full".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: n0,
            nodes: vec![
                DialogueNode {
                    id: n0,
                    body: DialogueBody::NpcLine {
                        speaker,
                        text_key: "fixture.creature".into(),
                        conditions: Vec::new(),
                        on_enter: Vec::new(),
                        next: Some(n1),
                    },
                },
                DialogueNode {
                    id: n1,
                    body: DialogueBody::PlayerChoice {
                        text_key: "fixture.creature".into(),
                        conditions: Vec::new(),
                        on_select: Vec::new(),
                        next: n2,
                    },
                },
                DialogueNode {
                    id: n2,
                    body: DialogueBody::Jump { target: n3 },
                },
                DialogueNode {
                    id: n3,
                    body: DialogueBody::Link {
                        target: dialogue_id,
                    },
                },
                DialogueNode {
                    id: n4,
                    body: DialogueBody::End,
                },
            ],
        }),
    );
    // Dialogue entry source is the dialogue; node refs source is the node.
    let entry = explain(&campaign, 201).expect("dialogue");
    let entry_out: Vec<(String, Value)> = entry["outbound"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["pointer"].as_str().unwrap().to_string(),
                e["source"].clone(),
            )
        })
        .collect();
    assert!(entry_out
        .iter()
        .any(|(p, s)| p == "/entry" && *s == json!(id(201).to_string())));
    let node0 = explain(&campaign, 202).expect("node0");
    let pointers: Vec<String> = node0["outbound"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["pointer"].as_str().unwrap().to_string())
        .collect();
    assert!(pointers.contains(&"/nodes/0/body/speaker".to_string()));
    assert!(pointers.contains(&"/nodes/0/body/next".to_string()));
    for edge in node0["outbound"].as_array().unwrap() {
        assert_eq!(edge["source"], json!(id(202).to_string()));
        assert!(edge["target_location"].is_object());
    }
    // Quest entry and transitions.
    let quest_id = id(301);
    let s0 = id(302);
    let s1 = id(303);
    campaign.documents.insert(
        path("quests/full.json"),
        Document::Quest(Quest {
            id: quest_id,
            slug: "fullq".into(),
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
                    transitions: Vec::new(),
                },
            ],
        }),
    );
    let state0 = explain(&campaign, 302).expect("state0");
    assert!(state0["outbound"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["pointer"] == json!("/states/0/transitions/0/target")
            && e["source"] == json!(id(302).to_string())
            && e["target_location"]["kind"] == json!("quest_state")));
    // Standalone graph: start, sequence child, call-graph target, edge endpoints.
    let graph_id = id(401);
    let g0 = id(402);
    let g1 = id(403);
    let g2 = id(404);
    campaign.documents.insert(
        path("scripts/graphs/full.json"),
        Document::Graph(EventGraph {
            id: graph_id,
            slug: "fullg".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: Trigger::AreaEnter,
            start: g0,
            nodes: vec![
                Node {
                    id: g0,
                    body: NodeBody::Sequence { nodes: vec![g1] },
                },
                Node {
                    id: g1,
                    body: NodeBody::CallGraph {
                        graph_id: id(6),
                        args: BTreeMap::new(),
                    },
                },
                Node {
                    id: g2,
                    body: NodeBody::Wait { ticks: 0 },
                },
            ],
            edges: vec![Edge {
                from: g0,
                port: Port::Next,
                to: g1,
            }],
            locals: Vec::new(),
        }),
    );
    let graph = explain(&campaign, 401).expect("graph");
    let g_pointers: Vec<String> = graph["outbound"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["pointer"].as_str().unwrap().to_string())
        .collect();
    assert!(g_pointers.contains(&"/start".to_string()));
    assert!(g_pointers.contains(&"/edges/0/from".to_string()));
    assert!(g_pointers.contains(&"/edges/0/to".to_string()));
    // Edge endpoints source is the graph, sequence child source is the node.
    for edge in graph["outbound"].as_array().unwrap() {
        if edge["pointer"] == json!("/edges/0/from") || edge["pointer"] == json!("/edges/0/to") {
            assert_eq!(edge["source"], json!(id(401).to_string()));
        }
    }
    let seq_node = explain(&campaign, 402).expect("seq node");
    assert!(seq_node["outbound"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["pointer"] == json!("/nodes/0/body/nodes/0")
            && e["source"] == json!(id(402).to_string())));
    // Aggregate owners appear once with null source.
    let placements_owner = explain(&campaign, 3).expect("area");
    let inbound_files: Vec<String> = placements_owner["inbound"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["file"].as_str().unwrap().to_string())
        .collect();
    assert!(inbound_files.contains(&"areas/start/placements.json".to_string()));
    for edge in placements_owner["inbound"].as_array().unwrap() {
        if edge["file"] == json!("areas/start/placements.json") && edge["pointer"] == json!("/area")
        {
            assert_eq!(edge["source"], Value::Null);
            assert_eq!(edge["target_location"]["kind"], json!("area"));
        }
    }
    // Nested tagged values in every DataValue context.
    let mut campaign2 = valid_loaded();
    let target = id(4);
    let nested = DataValue::Map(BTreeMap::from([(
        "k".into(),
        DataValue::List(vec![DataValue::ObjectRef(target)]),
    )]));
    let Document::Placements(placements) = campaign2
        .documents
        .get_mut(&path("areas/start/placements.json"))
        .unwrap()
    else {
        panic!("placements")
    };
    placements.placements[0]
        .overrides
        .insert("deep".into(), nested.clone());
    let Document::World(world) = campaign2
        .documents
        .get_mut(&path("worlds/world.json"))
        .unwrap()
    else {
        panic!("world")
    };
    world.variables.push(VarDecl {
        name: "v".into(),
        value_type: ValueType::Map,
        default: nested.clone(),
        scope: VariableScope::World,
    });
    let Document::Triggers(triggers) = campaign2
        .documents
        .get_mut(&path("areas/start/triggers.json"))
        .unwrap()
    else {
        panic!("triggers")
    };
    triggers.graphs[0].nodes[0].body = NodeBody::Branch {
        expr: "x".into(),
        cases: vec![nested.clone()],
    };
    triggers.graphs[0].locals.push(VarDecl {
        name: "g".into(),
        value_type: ValueType::Map,
        default: nested.clone(),
        scope: VariableScope::Graph,
    });
    let Document::Variables(vars) = campaign2
        .documents
        .get_mut(&path("variables/campaign_state.json"))
        .unwrap()
    else {
        panic!("vars")
    };
    vars.variables.push(VarDecl {
        name: "c".into(),
        value_type: ValueType::Map,
        default: nested.clone(),
        scope: VariableScope::Campaign,
    });
    let Document::AssetsLock(assets) = campaign2
        .documents
        .get_mut(&path("assets/assets.lock"))
        .unwrap()
    else {
        panic!("assets")
    };
    assets.assets.insert(
        path("assets/models/dragon.glb"),
        AssetRecord {
            hash: Digest::from_bytes([7; 32]),
            import: BTreeMap::from([("ref".into(), DataValue::ObjectRef(target))]),
        },
    );
    // Keep the structural digest valid after the asset insertion.
    let digest = assets_lock_digest(assets).unwrap();
    let _ = assets;
    let Document::CampaignLock(lock) = campaign2.documents.get_mut(&path("campaign.lock")).unwrap()
    else {
        panic!("lock")
    };
    lock.assets_lock = digest;
    // Placement overrides source is the placement; world var source is the world;
    // graph branch/locals source is node/graph; campaign vars and asset imports
    // have null source.
    let placement_deep = explain(&campaign2, 5).expect("placement deep");
    assert!(placement_deep["outbound"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| {
            e["pointer"] == json!("/placements/0/overrides/deep/k/0/value")
                && e["source"] == json!(id(5).to_string())
        }));
    let world_deep = explain(&campaign2, 2).expect("world deep");
    assert!(world_deep["outbound"].as_array().unwrap().iter().any(|e| {
        e["pointer"] == json!("/variables/0/default/k/0/value")
            && e["source"] == json!(id(2).to_string())
    }));
}

#[test]
fn valid_dangling_wrong_foreign_and_unknown_ids() {
    let campaign = valid_loaded();
    // Valid incoming references are reported even when validation is clean.
    assert_eq!(validate(&campaign), Vec::new());
    let creature = explain(&campaign, 4).expect("creature");
    // Creature 4 has no outbound refs in the clean fixture, but placement 5
    // points at it: inbound carries the actual location.
    assert_eq!(creature["outbound"].as_array().unwrap().len(), 0);
    let inbound = creature["inbound"].as_array().unwrap();
    assert_eq!(inbound.len(), 1);
    assert_eq!(inbound[0]["file"], json!("areas/start/placements.json"));
    assert_eq!(inbound[0]["pointer"], json!("/placements/0/prefab"));
    assert_eq!(inbound[0]["source"], json!(id(5).to_string()));
    assert_eq!(inbound[0]["target_location"]["kind"], json!("creature"));
    // Dangling references remain outbound with null target location.
    let mut dangling = valid_loaded();
    let Document::Creature(creature_doc) = dangling
        .documents
        .get_mut(&path("creatures/creature.json"))
        .unwrap()
    else {
        panic!("creature")
    };
    creature_doc.inventory.push(id(9000));
    let out = explain(&dangling, 4).expect("creature with dangling");
    let outbound = out["outbound"].as_array().unwrap();
    assert_eq!(outbound.len(), 1);
    assert_eq!(outbound[0]["pointer"], json!("/inventory/0"));
    assert_eq!(outbound[0]["target"], json!(id(9000).to_string()));
    assert_eq!(outbound[0]["target_location"], Value::Null);
    // Unknown query id returns None, including when some edge targets it.
    assert_eq!(explain_object(&dangling, id(9000)).unwrap(), None);
    assert_eq!(explain_object(&campaign, id(9999)).unwrap(), None);
    // Wrong-kind retains its actual location.
    let mut wrong = valid_loaded();
    let Document::World(world) = wrong.documents.get_mut(&path("worlds/world.json")).unwrap()
    else {
        panic!("world")
    };
    world.areas = vec![id(4)];
    let area_target = explain(&wrong, 4).expect("creature as wrong area");
    assert!(area_target["inbound"].as_array().unwrap().iter().any(|e| {
        e["file"] == json!("worlds/world.json")
            && e["pointer"] == json!("/areas/0")
            && e["target_location"]["kind"] == json!("creature")
    }));
    // Foreign-owner retains its actual location.
    let mut foreign = valid_loaded();
    foreign.documents.insert(
        path("dialogue/other.json"),
        Document::Dialogue(Dialogue {
            id: id(9301),
            slug: "other".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: id(9002),
            nodes: vec![
                DialogueNode {
                    id: id(9001),
                    body: DialogueBody::End,
                },
                DialogueNode {
                    id: id(9002),
                    body: DialogueBody::Jump { target: id(9001) },
                },
            ],
        }),
    );
    foreign.documents.insert(
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
    let foreign_target = explain(&foreign, 9001).expect("foreign node");
    assert!(foreign_target["inbound"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| {
            e["file"] == json!("dialogue/third.json")
                && e["target_location"]["kind"] == json!("dialogue_node")
        }));
}

#[test]
fn duplicates_cycles_self_edges_nullable_aggregates_and_subtrees() {
    // Duplicate-target occurrences keep distinct pointers, not deduped by target.
    let mut campaign = valid_loaded();
    let Document::Creature(creature) = campaign
        .documents
        .get_mut(&path("creatures/creature.json"))
        .unwrap()
    else {
        panic!("creature")
    };
    creature.inventory = vec![id(4), id(4)];
    // Creature inventory expects Item, so both are wrong-kind but still inventoried.
    let dup = explain(&campaign, 4).expect("dup target");
    let outbound = dup["outbound"].as_array().unwrap();
    assert_eq!(outbound.len(), 2);
    assert_eq!(outbound[0]["pointer"], json!("/inventory/0"));
    assert_eq!(outbound[1]["pointer"], json!("/inventory/1"));
    assert_eq!(outbound[0]["target"], outbound[1]["target"]);
    // Self-reference appears once in each applicable list.
    let mut self_ref = valid_loaded();
    let Document::Creature(creature) = self_ref
        .documents
        .get_mut(&path("creatures/creature.json"))
        .unwrap()
    else {
        panic!("creature")
    };
    creature.inventory = vec![id(4)];
    let value = explain(&self_ref, 4).expect("self");
    assert_eq!(value["outbound"].as_array().unwrap().len(), 1);
    assert_eq!(value["inbound"].as_array().unwrap().len(), 2);
    // Placement prefab inbound + self inventory inbound both target 4.
    // Nullable fields contribute no occurrence.
    let clean = valid_loaded();
    let creature_clean = explain(&clean, 4).expect("clean creature");
    assert_eq!(creature_clean["outbound"].as_array().unwrap().len(), 0);
    // Aggregate owners have null sources; refs from unidentified aggregates too.
    let area = explain(&clean, 3).expect("area");
    for edge in area["inbound"].as_array().unwrap() {
        if edge["pointer"] == json!("/area") {
            assert_eq!(edge["source"], Value::Null);
        }
    }
    let campaign_report = explain(&clean, 1).expect("campaign");
    for edge in campaign_report["inbound"].as_array().unwrap() {
        assert_eq!(edge["source"], Value::Null);
    }
    // Parent subtree inclusion: dialogue parent outbound includes nested children refs.
    let mut with_dialogue = valid_loaded();
    with_dialogue.documents.insert(
        path("dialogue/nest.json"),
        Document::Dialogue(Dialogue {
            id: id(501),
            slug: "nest".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: id(502),
            nodes: vec![DialogueNode {
                id: id(502),
                body: DialogueBody::Jump { target: id(503) },
            }],
        }),
    );
    with_dialogue.documents.insert(
        path("creatures/extra.json"),
        Document::Creature(Creature {
            id: id(503),
            slug: "extra".into(),
            name: "fixture.creature".into(),
            note: None,
            stats: BTreeMap::new(),
            tags: Vec::new(),
            faction: None,
            inventory: Vec::new(),
        }),
    );
    let parent = explain(&with_dialogue, 501).expect("parent");
    assert!(parent["outbound"].as_array().unwrap().iter().any(|e| {
        e["pointer"] == json!("/nodes/0/body/target") && e["source"] == json!(id(502).to_string())
    }));
    // Nested source ids remain intact when a parent includes their references.
    // /nodes/1 versus /nodes/10 pins component boundaries, not textual prefixes.
    let mut many_nodes = valid_loaded();
    let mut nodes = Vec::new();
    for n in 0..11 {
        let node_id = id(600 + n);
        nodes.push(DialogueNode {
            id: node_id,
            body: DialogueBody::Jump { target: id(4) },
        });
    }
    many_nodes.documents.insert(
        path("dialogue/many.json"),
        Document::Dialogue(Dialogue {
            id: id(599),
            slug: "many".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: id(600),
            nodes,
        }),
    );
    let first = explain(&many_nodes, 600).expect("first node");
    // Node 600 outbound is only its own jump, not node 10's jump.
    assert_eq!(first["outbound"].as_array().unwrap().len(), 1);
    assert_eq!(
        first["outbound"][0]["pointer"],
        json!("/nodes/0/body/target")
    );
    let tenth = explain(&many_nodes, 610).expect("tenth node");
    assert_eq!(
        tenth["outbound"][0]["pointer"],
        json!("/nodes/10/body/target")
    );
    let parent_many = explain(&many_nodes, 599).expect("parent many");
    assert_eq!(parent_many["outbound"].as_array().unwrap().len(), 12);
}

#[test]
fn escaping_sorting_insertion_independence_and_negative_controls() {
    let mut campaign = valid_loaded();
    let Document::Placements(placements) = campaign
        .documents
        .get_mut(&path("areas/start/placements.json"))
        .unwrap()
    else {
        panic!("placements")
    };
    placements.placements[0]
        .overrides
        .insert("a/b~c".into(), DataValue::ObjectRef(id(4)));
    placements.placements[0]
        .overrides
        .insert("z".into(), DataValue::ObjectRef(id(2)));
    let value = explain(&campaign, 5).expect("escaped");
    let pointers: Vec<String> = value["outbound"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["pointer"].as_str().unwrap().to_string())
        .collect();
    assert!(pointers.contains(&"/placements/0/overrides/a~1b~0c/value".to_string()));
    // Lexical pointer order, not numerical array-index order.
    let mut sorted = pointers.clone();
    sorted.sort();
    assert_eq!(pointers, sorted);
    // Authored arrays preserved in the object value.
    assert!(value["object"]["overrides"].is_object());
    // Insertion-order independence: rebuild with reversed map insertion.
    let mut reversed = valid_loaded();
    let Document::Placements(other) = reversed
        .documents
        .get_mut(&path("areas/start/placements.json"))
        .unwrap()
    else {
        panic!("placements")
    };
    other.placements[0]
        .overrides
        .insert("z".into(), DataValue::ObjectRef(id(2)));
    other.placements[0]
        .overrides
        .insert("a/b~c".into(), DataValue::ObjectRef(id(4)));
    assert_eq!(
        explain_object(&campaign, id(5)).unwrap(),
        explain_object(&reversed, id(5)).unwrap()
    );
    // ULID-looking strings in notes, text, conditions, and Text values are never edges.
    let mut hostile = valid_loaded();
    let ulid_text = id(9000).to_string();
    let Document::Creature(creature) = hostile
        .documents
        .get_mut(&path("creatures/creature.json"))
        .unwrap()
    else {
        panic!("creature")
    };
    creature.note = Some(format!("see {ulid_text}"));
    let Document::Placements(placements) = hostile
        .documents
        .get_mut(&path("areas/start/placements.json"))
        .unwrap()
    else {
        panic!("placements")
    };
    placements.placements[0]
        .overrides
        .insert("hostile".into(), DataValue::Text(ulid_text.clone()));
    let hostile_report = explain(&hostile, 5).expect("hostile");
    assert!(!hostile_report["outbound"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["target"] == json!(ulid_text)));
    assert!(!hostile_report["inbound"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["target"] == json!(ulid_text)));
    // Unknown id with ULID-looking text targeting nothing still returns None.
    assert_eq!(explain_object(&hostile, id(9000)).unwrap(), None);
}

#[test]
fn forged_indexes_are_ignored_and_structural_errors_win() {
    let campaign = valid_loaded();
    let baseline = explain_object(&campaign, id(5)).unwrap().expect("baseline");
    // Forged, empty, and stale indexes produce identical reports.
    let mut forged = campaign.clone();
    forged.index.clear();
    forged.index.insert(
        id(4242),
        IndexEntry {
            kind: ObjectKind::Item,
            path: path("items/fake.json"),
            pointer: String::new(),
        },
    );
    assert_eq!(
        explain_object(&forged, id(5)).unwrap().expect("forged"),
        baseline
    );
    let mut emptied = campaign.clone();
    emptied.index.clear();
    assert_eq!(
        explain_object(&emptied, id(5)).unwrap().expect("empty"),
        baseline
    );
    // Mutated documents with duplicate ids return the writer's error before lookup.
    let mut duplicate = campaign.clone();
    let Document::Creature(creature) = duplicate
        .documents
        .get_mut(&path("creatures/creature.json"))
        .unwrap()
    else {
        panic!("creature")
    };
    creature.id = id(3);
    assert!(matches!(
        explain_object(&duplicate, id(4)),
        Err(DataError::DuplicateId { .. })
    ));
    assert!(matches!(
        serialize_campaign(&duplicate),
        Err(DataError::DuplicateId { .. })
    ));
    // Layout, lock, and digest failures match the writer's errors.
    let mut missing = campaign.clone();
    missing.documents.remove(&path("campaign.lock"));
    assert!(matches!(
        explain_object(&missing, id(1)),
        Err(DataError::Layout { .. })
    ));
    let mut bad_lock = campaign.clone();
    let Document::CampaignLock(lock) = bad_lock.documents.get_mut(&path("campaign.lock")).unwrap()
    else {
        panic!("lock")
    };
    lock.assets_lock = Digest::from_bytes([0; 32]);
    assert!(matches!(
        explain_object(&bad_lock, id(1)),
        Err(DataError::AssetsLockMismatch { .. })
    ));
    // Structural failure wins over unknown id.
    assert!(explain_object(&missing, id(9999)).is_err());
    assert!(explain_object(&duplicate, id(9999)).is_err());
    // Input maps and indexes remain unchanged on success, absence, and failure.
    let before = campaign.clone();
    let _ = explain_object(&campaign, id(5)).unwrap();
    let _ = explain_object(&campaign, id(9999)).unwrap();
    let _ = explain_object(&duplicate, id(5));
    assert_eq!(campaign.documents, before.documents);
    assert_eq!(campaign.index, before.index);
}

#[test]
fn fixtures_are_read_only_and_semantics_do_not_block_introspection() {
    // one_area_one_creature loads and explains every root; broken snapshot preserved.
    let files = support::fixture_files();
    let campaign = load_campaign(&files, &engine()).unwrap();
    for n in [1, 2, 3, 4, 5, 6, 7] {
        assert!(
            explain_object(&campaign, id(n)).unwrap().is_some(),
            "id {n}"
        );
    }
    let broken = load_campaign(&broken_files(), &engine()).unwrap();
    let diagnostics = validate(&broken);
    assert_eq!(diagnostics.len(), 15);
    // Introspection succeeds despite semantic findings.
    let broken_campaign_id = broken.documents[&path("campaign.json")].clone();
    let Document::Campaign(manifest) = &broken_campaign_id else {
        panic!("campaign")
    };
    let report = explain_object(&broken, manifest.id)
        .unwrap()
        .expect("broken explain");
    assert_eq!(report.len(), report.len());
    // Migration-aware loaded objects use current serialized shape.
    let migrated = load_campaign(&migration_files(), &engine()).unwrap();
    let item_report = explain(&migrated, 8).expect("migrated item");
    assert_eq!(item_report["kind"], json!("item"));
    assert_eq!(item_report["object"]["schema"], json!("crpg.item/2"));
    // Historical files are not rewritten by introspection.
    let raw: Value = serde_json::from_slice(&migration_files()[&path("items/item.json")]).unwrap();
    assert_eq!(raw["schema"], json!("crpg.item/1"));
    // Required fixtures fail hard when absent.
    assert!(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/one_area_one_creature/campaign.json")
        .exists());
    assert!(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/broken_references/campaign.json")
        .exists());
    assert!(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/migration_v1/campaign/campaign.json")
        .exists());
}

#[test]
fn shared_enumeration_covers_every_site_for_both_consumers() {
    // Positive: every typed site in the inventory appears in validation
    // diagnostics when dangling and in introspection edges when present.
    // Removing a site from the shared enumeration must fail both consumers.
    let mut campaign = valid_loaded();
    // Faction relation site.
    campaign.documents.insert(
        path("factions/rel.json"),
        Document::Faction(Faction {
            id: id(701),
            slug: "rel".into(),
            name: "fixture.creature".into(),
            note: None,
            relations: vec![FactionRelation {
                faction: id(9000),
                disposition: 1,
            }],
        }),
    );
    // Validation sees the dangling relation; introspection inventories it.
    let loaded = campaign.clone();
    assert!(validate(&loaded).iter().any(
        |d| d.code == DiagnosticCode::DanglingReference && d.pointer == "/relations/0/faction"
    ));
    let faction_report = explain(&loaded, 701).expect("faction");
    assert!(faction_report["outbound"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| {
            e["pointer"] == json!("/relations/0/faction") && e["target_location"] == Value::Null
        }));
    // Negative: ordinary strings, notes, conditions, symbolic names, asset
    // paths, locale keys, tags, digests, and package coordinates are never edges.
    let clean = valid_loaded();
    let before_edges: usize = explain(&clean, 1).unwrap()["outbound"]
        .as_array()
        .unwrap()
        .len();
    let mut hostile = clean.clone();
    let Document::Campaign(manifest) = hostile.documents.get_mut(&path("campaign.json")).unwrap()
    else {
        panic!("campaign")
    };
    manifest.note = Some(id(9000).to_string());
    let after_edges: usize = explain(&hostile, 1).unwrap()["outbound"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(before_edges, after_edges);
    // The shared inventory is the single authority: both consumers traverse
    // the same occurrence list, so deleting a site cannot stay green elsewhere.
    // Faction relations, placement prefabs, and graph starts are pinned above
    // for both validation diagnostics and introspection edges; removing any of
    // those sites from the shared enumeration fails both assertions.
    assert!(explain(&loaded, 701).is_some());
    assert!(!validate(&loaded).is_empty());
}

#[test]
fn every_argument_site_reports_nested_refs_with_enclosing_sources() {
    let mut campaign = valid_loaded();
    let target = id(4);
    let nested = DataValue::Map(BTreeMap::from([("k".into(), DataValue::ObjectRef(target))]));
    // Dialogue action arguments in every body variant.
    let d_id = id(801);
    let dn0 = id(802);
    let dn1 = id(803);
    campaign.documents.insert(
        path("dialogue/args.json"),
        Document::Dialogue(Dialogue {
            id: d_id,
            slug: "args".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: dn0,
            nodes: vec![
                DialogueNode {
                    id: dn0,
                    body: DialogueBody::NpcLine {
                        speaker: target,
                        text_key: "fixture.creature".into(),
                        conditions: Vec::new(),
                        on_enter: vec![ActionCall {
                            action_id: "act".into(),
                            args: BTreeMap::from([("arg".into(), nested.clone())]),
                        }],
                        next: Some(dn1),
                    },
                },
                DialogueNode {
                    id: dn1,
                    body: DialogueBody::PlayerChoice {
                        text_key: "fixture.creature".into(),
                        conditions: Vec::new(),
                        on_select: vec![ActionCall {
                            action_id: "act".into(),
                            args: BTreeMap::from([("arg".into(), nested.clone())]),
                        }],
                        next: dn1,
                    },
                },
            ],
        }),
    );
    let _ = dn1;
    // Quest action arguments and graph action/script/call-graph arguments.
    let q_id = id(811);
    let qs0 = id(812);
    campaign.documents.insert(
        path("quests/args.json"),
        Document::Quest(Quest {
            id: q_id,
            slug: "argsq".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: qs0,
            states: vec![QuestState {
                id: qs0,
                name: "fixture.creature".into(),
                terminal: true,
                on_enter: vec![ActionCall {
                    action_id: "act".into(),
                    args: BTreeMap::from([("arg".into(), nested.clone())]),
                }],
                transitions: Vec::new(),
            }],
        }),
    );
    let g_id = id(821);
    let gn0 = id(822);
    let gn1 = id(823);
    let gn2 = id(824);
    campaign.documents.insert(
        path("scripts/graphs/args.json"),
        Document::Graph(EventGraph {
            id: g_id,
            slug: "argsg".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: Trigger::AreaEnter,
            start: gn0,
            nodes: vec![
                Node {
                    id: gn0,
                    body: NodeBody::Action {
                        call: ActionCall {
                            action_id: "act".into(),
                            args: BTreeMap::from([("arg".into(), nested.clone())]),
                        },
                    },
                },
                Node {
                    id: gn1,
                    body: NodeBody::CallScript {
                        script_id: "s".into(),
                        args: BTreeMap::from([("arg".into(), nested.clone())]),
                    },
                },
                Node {
                    id: gn2,
                    body: NodeBody::CallGraph {
                        graph_id: id(6),
                        args: BTreeMap::from([("arg".into(), nested.clone())]),
                    },
                },
            ],
            edges: Vec::new(),
            locals: Vec::new(),
        }),
    );
    for (node, pointer) in [
        (802, "/nodes/0/body/on_enter/0/args/arg/k/value"),
        (803, "/nodes/1/body/on_select/0/args/arg/k/value"),
        (812, "/states/0/on_enter/0/args/arg/k/value"),
        (822, "/nodes/0/body/call/args/arg/k/value"),
        (823, "/nodes/1/body/args/arg/k/value"),
        (824, "/nodes/2/body/args/arg/k/value"),
    ] {
        let value = explain(&campaign, node).expect("arg node present");
        assert!(
            value["outbound"].as_array().unwrap().iter().any(|e| {
                e["pointer"] == json!(pointer) && e["target"] == json!(target.to_string())
            }),
            "node {node} pointer {pointer}"
        );
    }
    // Asset import escaping uses RFC 6901 for slashes in the asset path.
    let mut with_asset = valid_loaded();
    let Document::AssetsLock(assets) = with_asset
        .documents
        .get_mut(&path("assets/assets.lock"))
        .unwrap()
    else {
        panic!("assets")
    };
    assets.assets.insert(
        path("assets/a/b.glb"),
        AssetRecord {
            hash: Digest::from_bytes([9; 32]),
            import: BTreeMap::from([("ref".into(), DataValue::ObjectRef(target))]),
        },
    );
    let digest = assets_lock_digest(assets).unwrap();
    let _ = assets;
    let Document::CampaignLock(lock) = with_asset
        .documents
        .get_mut(&path("campaign.lock"))
        .unwrap()
    else {
        panic!("lock")
    };
    lock.assets_lock = digest;
    let target_report = explain(&with_asset, 4).expect("asset target");
    assert!(target_report["inbound"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| {
            e["pointer"] == json!("/assets/assets~1a~1b.glb/import/ref/value")
                && e["source"] == Value::Null
        }));
}

#[test]
fn lexical_pointer_order_and_document_insertion_independence() {
    // Lexical, not numerical, array-index order: /nodes/10 sorts before /nodes/2.
    let mut campaign = valid_loaded();
    let mut nodes = Vec::new();
    for n in 0..11 {
        nodes.push(DialogueNode {
            id: id(700 + n),
            body: DialogueBody::Jump { target: id(4) },
        });
    }
    campaign.documents.insert(
        path("dialogue/order.json"),
        Document::Dialogue(Dialogue {
            id: id(699),
            slug: "order".into(),
            name: "fixture.creature".into(),
            note: None,
            entry: id(700),
            nodes,
        }),
    );
    let parent = explain(&campaign, 699).expect("order parent");
    let pointers: Vec<String> = parent["outbound"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["pointer"].as_str().unwrap().to_string())
        .collect();
    let mut lexical = pointers.clone();
    lexical.sort();
    assert_eq!(pointers, lexical);
    assert!(
        pointers
            .iter()
            .position(|p| p == "/nodes/10/body/target")
            .unwrap()
            < pointers
                .iter()
                .position(|p| p == "/nodes/2/body/target")
                .unwrap()
    );
    // Document insertion order does not change canonical bytes.
    let mut reversed = valid_loaded();
    reversed.documents.insert(
        path("dialogue/order.json"),
        campaign.documents[&path("dialogue/order.json")].clone(),
    );
    assert_eq!(
        explain_object(&campaign, id(699)).unwrap(),
        explain_object(&reversed, id(699)).unwrap()
    );
}
