//! Shared typed identity and reference enumeration for validation and introspection.
//!
//! This private module is the single authority for which object identities exist,
//! where they live, and which typed fields carry object references. Both
//! [`validate`](crate::validate) and
//! [`explain_object`](crate::explain_object) consume these helpers so the two
//! consumers cannot grow independent field lists. Validation retains its
//! diagnostic messages, precedence, reachability, and sort policy; introspection
//! retains its report shape, subtree, and ordering rules.

use crate::{DataValue, Document, EventGraph, ObjectKind, SourcePath};
use crpg_core::Ulid;
use std::collections::BTreeMap;

/// Every indexed object kind; untyped object references accept any of them.
pub(crate) const ALL_KINDS: [ObjectKind; 13] = [
    ObjectKind::Campaign,
    ObjectKind::World,
    ObjectKind::Area,
    ObjectKind::Creature,
    ObjectKind::Item,
    ObjectKind::Dialogue,
    ObjectKind::Quest,
    ObjectKind::Faction,
    ObjectKind::Placement,
    ObjectKind::Graph,
    ObjectKind::Node,
    ObjectKind::DialogueNode,
    ObjectKind::QuestState,
];

static ALLOWED_WORLD: &[ObjectKind] = &[ObjectKind::World];
static ALLOWED_AREA: &[ObjectKind] = &[ObjectKind::Area];
static ALLOWED_PLACEMENT: &[ObjectKind] = &[ObjectKind::Placement];
static ALLOWED_FACTION: &[ObjectKind] = &[ObjectKind::Faction];
static ALLOWED_ITEM: &[ObjectKind] = &[ObjectKind::Item];
static ALLOWED_CREATURE_ITEM: &[ObjectKind] = &[ObjectKind::Creature, ObjectKind::Item];
static ALLOWED_CREATURE_PLACEMENT: &[ObjectKind] = &[ObjectKind::Creature, ObjectKind::Placement];
static ALLOWED_DIALOGUE_NODE: &[ObjectKind] = &[ObjectKind::DialogueNode];
static ALLOWED_DIALOGUE: &[ObjectKind] = &[ObjectKind::Dialogue];
static ALLOWED_QUEST_STATE: &[ObjectKind] = &[ObjectKind::QuestState];
static ALLOWED_NODE: &[ObjectKind] = &[ObjectKind::Node];
static ALLOWED_GRAPH: &[ObjectKind] = &[ObjectKind::Graph];
static ALLOWED_OBJECT: &[ObjectKind] = &ALL_KINDS;

/// Stable lowercase kind word used inside diagnostic messages.
pub(crate) fn diagnostic_kind_name(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Campaign => "campaign",
        ObjectKind::World => "world",
        ObjectKind::Area => "area",
        ObjectKind::Creature => "creature",
        ObjectKind::Item => "item",
        ObjectKind::Dialogue => "dialogue",
        ObjectKind::Quest => "quest",
        ObjectKind::Faction => "faction",
        ObjectKind::Placement => "placement",
        ObjectKind::Graph => "graph",
        ObjectKind::Node => "node",
        ObjectKind::DialogueNode => "dialogue node",
        ObjectKind::QuestState => "quest state",
    }
}

/// Escapes one RFC 6901 reference-token segment.
pub(crate) fn escape_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for c in segment.chars() {
        match c {
            '~' => out.push_str("~0"),
            '/' => out.push_str("~1"),
            _ => out.push(c),
        }
    }
    out
}

/// One authored identity with its rebuilt location.
pub(crate) struct ObjectOccurrence {
    /// Authored identity.
    pub id: Ulid,
    /// Object category.
    pub kind: ObjectKind,
    /// Logical source file.
    pub path: SourcePath,
    /// RFC 6901 pointer to the object, not its id field.
    pub pointer: String,
}

/// Enumerates every identified object in loader order.
///
/// Lexical document paths, root entity first, then identified array entries in
/// authored order (graph before its nodes, dialogue before nodes, quest before
/// states; placements or graphs followed by each graph's nodes).
pub(crate) fn object_occurrences(
    documents: &BTreeMap<SourcePath, Document>,
) -> Vec<ObjectOccurrence> {
    let mut out = Vec::new();
    for (path, document) in documents {
        let root = match document {
            Document::Campaign(v) => Some((v.id, ObjectKind::Campaign)),
            Document::World(v) => Some((v.id, ObjectKind::World)),
            Document::Area(v) => Some((v.id, ObjectKind::Area)),
            Document::Creature(v) => Some((v.id, ObjectKind::Creature)),
            Document::Item(v) => Some((v.id, ObjectKind::Item)),
            Document::Dialogue(v) => Some((v.id, ObjectKind::Dialogue)),
            Document::Quest(v) => Some((v.id, ObjectKind::Quest)),
            Document::Faction(v) => Some((v.id, ObjectKind::Faction)),
            Document::Graph(v) => Some((v.id, ObjectKind::Graph)),
            _ => None,
        };
        if let Some((id, kind)) = root {
            out.push(ObjectOccurrence {
                id,
                kind,
                path: path.clone(),
                pointer: String::new(),
            });
        }
        match document {
            Document::Placements(v) => {
                for (i, p) in v.placements.iter().enumerate() {
                    out.push(ObjectOccurrence {
                        id: p.id,
                        kind: ObjectKind::Placement,
                        path: path.clone(),
                        pointer: format!("/placements/{i}"),
                    });
                }
            }
            Document::Triggers(v) => {
                for (i, graph) in v.graphs.iter().enumerate() {
                    out.push(ObjectOccurrence {
                        id: graph.id,
                        kind: ObjectKind::Graph,
                        path: path.clone(),
                        pointer: format!("/graphs/{i}"),
                    });
                    for (j, node) in graph.nodes.iter().enumerate() {
                        out.push(ObjectOccurrence {
                            id: node.id,
                            kind: ObjectKind::Node,
                            path: path.clone(),
                            pointer: format!("/graphs/{i}/nodes/{j}"),
                        });
                    }
                }
            }
            Document::Graph(v) => {
                for (i, node) in v.nodes.iter().enumerate() {
                    out.push(ObjectOccurrence {
                        id: node.id,
                        kind: ObjectKind::Node,
                        path: path.clone(),
                        pointer: format!("/nodes/{i}"),
                    });
                }
            }
            Document::Dialogue(v) => {
                for (i, node) in v.nodes.iter().enumerate() {
                    out.push(ObjectOccurrence {
                        id: node.id,
                        kind: ObjectKind::DialogueNode,
                        path: path.clone(),
                        pointer: format!("/nodes/{i}"),
                    });
                }
            }
            Document::Quest(v) => {
                for (i, state) in v.states.iter().enumerate() {
                    out.push(ObjectOccurrence {
                        id: state.id,
                        kind: ObjectKind::QuestState,
                        path: path.clone(),
                        pointer: format!("/states/{i}"),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// Ownership tables mapping local child ids to their owning container id.
///
/// First occurrence wins in the same lexical-path, authored order as identity
/// enumeration.
pub(crate) fn ownership_tables(
    documents: &BTreeMap<SourcePath, Document>,
) -> (
    BTreeMap<Ulid, Ulid>,
    BTreeMap<Ulid, Ulid>,
    BTreeMap<Ulid, Ulid>,
) {
    let mut node_owner: BTreeMap<Ulid, Ulid> = BTreeMap::new();
    let mut dnode_owner: BTreeMap<Ulid, Ulid> = BTreeMap::new();
    let mut state_owner: BTreeMap<Ulid, Ulid> = BTreeMap::new();
    for document in documents.values() {
        match document {
            Document::Triggers(v) => {
                for graph in &v.graphs {
                    for node in &graph.nodes {
                        node_owner.entry(node.id).or_insert(graph.id);
                    }
                }
            }
            Document::Graph(v) => {
                for node in &v.nodes {
                    node_owner.entry(node.id).or_insert(v.id);
                }
            }
            Document::Dialogue(v) => {
                for node in &v.nodes {
                    dnode_owner.entry(node.id).or_insert(v.id);
                }
            }
            Document::Quest(v) => {
                for state in &v.states {
                    state_owner.entry(state.id).or_insert(v.id);
                }
            }
            _ => {}
        }
    }
    (node_owner, dnode_owner, state_owner)
}

/// Local-ownership expectation for one generic reference check.
pub(crate) struct OwnerExpectation {
    /// Owner recorded for the target id, if it is an identified local child.
    pub found: Option<Ulid>,
    /// Id of the container holding the reference.
    pub mine: Ulid,
    /// Kind word of the container used inside messages.
    pub container: &'static str,
}

/// How one enumerated reference participates in validation.
pub(crate) enum RefPolicy {
    /// A typed reference checked generically by validation.
    Generic {
        /// Allowed target kinds.
        allowed: &'static [ObjectKind],
        /// Local-ownership expectation, when the target must live in this container.
        owner: Option<OwnerExpectation>,
        /// Human-readable field description used inside messages.
        what: &'static str,
    },
    /// An aggregate owner field: introspection inventories it, validation
    /// reports it through its specialized ownership check instead.
    AggregateOwner {
        /// Human-readable aggregate description used inside messages.
        what: &'static str,
    },
    /// A graph edge endpoint: introspection inventories it, validation judges
    /// only its port and reachability from resolved endpoints.
    EdgeEndpoint,
}

/// One typed authored ULID reference occurrence.
pub(crate) struct RefOccurrence {
    /// Logical source file containing the reference value.
    pub file: SourcePath,
    /// RFC 6901 pointer to the reference value.
    pub pointer: String,
    /// Nearest enclosing authored object, or `None` outside any identified object.
    pub source: Option<Ulid>,
    /// Authored target identity.
    pub target: Ulid,
    /// Validation participation for this site.
    pub policy: RefPolicy,
}

#[allow(clippy::too_many_arguments)]
fn push_generic(
    out: &mut Vec<RefOccurrence>,
    file: &SourcePath,
    pointer: String,
    source: Option<Ulid>,
    target: Ulid,
    allowed: &'static [ObjectKind],
    owner: Option<OwnerExpectation>,
    what: &'static str,
) {
    out.push(RefOccurrence {
        file: file.clone(),
        pointer,
        source,
        target,
        policy: RefPolicy::Generic {
            allowed,
            owner,
            what,
        },
    });
}

fn push_owner(
    out: &mut Vec<RefOccurrence>,
    file: &SourcePath,
    pointer: &str,
    target: Ulid,
    what: &'static str,
) {
    out.push(RefOccurrence {
        file: file.clone(),
        pointer: pointer.into(),
        source: None,
        target,
        policy: RefPolicy::AggregateOwner { what },
    });
}

fn push_edge(
    out: &mut Vec<RefOccurrence>,
    file: &SourcePath,
    pointer: String,
    source: Ulid,
    target: Ulid,
) {
    out.push(RefOccurrence {
        file: file.clone(),
        pointer,
        source: Some(source),
        target,
        policy: RefPolicy::EdgeEndpoint,
    });
}

/// Recursively inventories object references inside one tagged value.
///
/// The occurrence sits at the tagged value's `/value` member; list and map
/// segments extend the base pointer with escaping, matching validation.
fn inventory_value(
    out: &mut Vec<RefOccurrence>,
    file: &SourcePath,
    base: String,
    value: &DataValue,
    source: Option<Ulid>,
) {
    match value {
        DataValue::ObjectRef(id) => {
            let mut pointer = base;
            pointer.push_str("/value");
            push_generic(
                out,
                file,
                pointer,
                source,
                *id,
                ALLOWED_OBJECT,
                None,
                "object",
            );
        }
        DataValue::List(items) => {
            for (i, item) in items.iter().enumerate() {
                inventory_value(out, file, format!("{base}/{i}"), item, source);
            }
        }
        DataValue::Map(entries) => {
            for (key, item) in entries {
                inventory_value(
                    out,
                    file,
                    format!("{base}/{}", escape_segment(key)),
                    item,
                    source,
                );
            }
        }
        _ => {}
    }
}

/// Inventories one named argument map of tagged values.
fn inventory_args(
    out: &mut Vec<RefOccurrence>,
    file: &SourcePath,
    base: String,
    args: &BTreeMap<String, DataValue>,
    source: Option<Ulid>,
) {
    for (key, value) in args {
        inventory_value(
            out,
            file,
            format!("{base}/{}", escape_segment(key)),
            value,
            source,
        );
    }
}

/// Inventories nested references inside one variable default.
fn inventory_var(
    out: &mut Vec<RefOccurrence>,
    file: &SourcePath,
    base: String,
    default: &DataValue,
    source: Option<Ulid>,
) {
    let mut at_default = base;
    at_default.push_str("/default");
    inventory_value(out, file, at_default, default, source);
}

/// Inventories references for one event graph at its object pointer.
///
/// `gbase` is the graph object's pointer within its file (empty for a
/// standalone graph document, `/graphs/{i}` when embedded in triggers).
fn inventory_graph(
    out: &mut Vec<RefOccurrence>,
    file: &SourcePath,
    gbase: String,
    graph: &EventGraph,
    node_owner: &BTreeMap<Ulid, Ulid>,
) {
    push_generic(
        out,
        file,
        format!("{gbase}/start"),
        Some(graph.id),
        graph.start,
        ALLOWED_NODE,
        Some(OwnerExpectation {
            found: node_owner.get(&graph.start).copied(),
            mine: graph.id,
            container: "graph",
        }),
        "graph start",
    );
    for (i, decl) in graph.locals.iter().enumerate() {
        inventory_var(
            out,
            file,
            format!("{gbase}/locals/{i}"),
            &decl.default,
            Some(graph.id),
        );
    }
    for (j, node) in graph.nodes.iter().enumerate() {
        let nbase = format!("{gbase}/nodes/{j}");
        match &node.body {
            crate::NodeBody::Condition { .. } | crate::NodeBody::Wait { .. } => {}
            crate::NodeBody::Action { call } => {
                inventory_args(
                    out,
                    file,
                    format!("{nbase}/body/call/args"),
                    &call.args,
                    Some(node.id),
                );
            }
            crate::NodeBody::Branch { cases, .. } => {
                for (k, case) in cases.iter().enumerate() {
                    inventory_value(
                        out,
                        file,
                        format!("{nbase}/body/cases/{k}"),
                        case,
                        Some(node.id),
                    );
                }
            }
            crate::NodeBody::Sequence { nodes } => {
                for (k, child) in nodes.iter().enumerate() {
                    push_generic(
                        out,
                        file,
                        format!("{nbase}/body/nodes/{k}"),
                        Some(node.id),
                        *child,
                        ALLOWED_NODE,
                        Some(OwnerExpectation {
                            found: node_owner.get(child).copied(),
                            mine: graph.id,
                            container: "graph",
                        }),
                        "sequence child",
                    );
                }
            }
            crate::NodeBody::CallScript { args, .. } => {
                inventory_args(out, file, format!("{nbase}/body/args"), args, Some(node.id));
            }
            crate::NodeBody::CallGraph { graph_id, args } => {
                push_generic(
                    out,
                    file,
                    format!("{nbase}/body/graph_id"),
                    Some(node.id),
                    *graph_id,
                    ALLOWED_GRAPH,
                    None,
                    "graph call",
                );
                inventory_args(out, file, format!("{nbase}/body/args"), args, Some(node.id));
            }
        }
    }
    for (i, edge) in graph.edges.iter().enumerate() {
        push_edge(
            out,
            file,
            format!("{gbase}/edges/{i}/from"),
            graph.id,
            edge.from,
        );
        push_edge(
            out,
            file,
            format!("{gbase}/edges/{i}/to"),
            graph.id,
            edge.to,
        );
    }
}

/// Collects every typed authored ULID reference occurrence.
///
/// Coverage is every typed object-reference field in the current documents,
/// including valid, dangling, wrong-kind, and foreign-owner edges, aggregate
/// ownership fields, graph edge endpoints, and recursively nested
/// `DataValue::ObjectRef` values. Identity declarations, containment itself,
/// package coordinates, digests, asset paths, locale keys, tags, notes, opaque
/// conditions, symbolic names, and ordinary strings are never edges. Null
/// optional references contribute no occurrence. Documents iterate in lexical
/// path order with authored array order preserved.
pub(crate) fn reference_occurrences(
    documents: &BTreeMap<SourcePath, Document>,
) -> Vec<RefOccurrence> {
    let (node_owner, dnode_owner, state_owner) = ownership_tables(documents);
    let mut out = Vec::new();
    for (path, document) in documents {
        match document {
            Document::Campaign(v) => {
                push_generic(
                    &mut out,
                    path,
                    "/entry/world".into(),
                    Some(v.id),
                    v.entry.world,
                    ALLOWED_WORLD,
                    None,
                    "entry world",
                );
                push_generic(
                    &mut out,
                    path,
                    "/entry/area".into(),
                    Some(v.id),
                    v.entry.area,
                    ALLOWED_AREA,
                    None,
                    "entry area",
                );
                push_generic(
                    &mut out,
                    path,
                    "/entry/spawn".into(),
                    Some(v.id),
                    v.entry.spawn,
                    ALLOWED_PLACEMENT,
                    None,
                    "entry spawn",
                );
            }
            Document::World(v) => {
                for (i, area) in v.areas.iter().enumerate() {
                    push_generic(
                        &mut out,
                        path,
                        format!("/areas/{i}"),
                        Some(v.id),
                        *area,
                        ALLOWED_AREA,
                        None,
                        "world area",
                    );
                }
                for (i, decl) in v.variables.iter().enumerate() {
                    inventory_var(
                        &mut out,
                        path,
                        format!("/variables/{i}"),
                        &decl.default,
                        Some(v.id),
                    );
                }
            }
            Document::Area(v) => {
                for (i, neighbour) in v.neighbours.iter().enumerate() {
                    push_generic(
                        &mut out,
                        path,
                        format!("/neighbours/{i}"),
                        Some(v.id),
                        *neighbour,
                        ALLOWED_AREA,
                        None,
                        "area neighbour",
                    );
                }
            }
            Document::Creature(v) => {
                if let Some(faction) = v.faction {
                    push_generic(
                        &mut out,
                        path,
                        "/faction".into(),
                        Some(v.id),
                        faction,
                        ALLOWED_FACTION,
                        None,
                        "creature faction",
                    );
                }
                for (i, item) in v.inventory.iter().enumerate() {
                    push_generic(
                        &mut out,
                        path,
                        format!("/inventory/{i}"),
                        Some(v.id),
                        *item,
                        ALLOWED_ITEM,
                        None,
                        "creature inventory item",
                    );
                }
            }
            Document::Item(_) => {}
            Document::Dialogue(v) => {
                push_generic(
                    &mut out,
                    path,
                    "/entry".into(),
                    Some(v.id),
                    v.entry,
                    ALLOWED_DIALOGUE_NODE,
                    Some(OwnerExpectation {
                        found: dnode_owner.get(&v.entry).copied(),
                        mine: v.id,
                        container: "dialogue",
                    }),
                    "dialogue entry",
                );
                for (i, node) in v.nodes.iter().enumerate() {
                    let nbase = format!("/nodes/{i}");
                    let owned = |id: &Ulid| OwnerExpectation {
                        found: dnode_owner.get(id).copied(),
                        mine: v.id,
                        container: "dialogue",
                    };
                    match &node.body {
                        crate::DialogueBody::NpcLine {
                            speaker,
                            on_enter,
                            next,
                            ..
                        } => {
                            push_generic(
                                &mut out,
                                path,
                                format!("{nbase}/body/speaker"),
                                Some(node.id),
                                *speaker,
                                ALLOWED_CREATURE_PLACEMENT,
                                None,
                                "dialogue speaker",
                            );
                            for (a, call) in on_enter.iter().enumerate() {
                                inventory_args(
                                    &mut out,
                                    path,
                                    format!("{nbase}/body/on_enter/{a}/args"),
                                    &call.args,
                                    Some(node.id),
                                );
                            }
                            if let Some(next) = next {
                                push_generic(
                                    &mut out,
                                    path,
                                    format!("{nbase}/body/next"),
                                    Some(node.id),
                                    *next,
                                    ALLOWED_DIALOGUE_NODE,
                                    Some(owned(next)),
                                    "dialogue next",
                                );
                            }
                        }
                        crate::DialogueBody::PlayerChoice {
                            on_select, next, ..
                        } => {
                            for (a, call) in on_select.iter().enumerate() {
                                inventory_args(
                                    &mut out,
                                    path,
                                    format!("{nbase}/body/on_select/{a}/args"),
                                    &call.args,
                                    Some(node.id),
                                );
                            }
                            push_generic(
                                &mut out,
                                path,
                                format!("{nbase}/body/next"),
                                Some(node.id),
                                *next,
                                ALLOWED_DIALOGUE_NODE,
                                Some(owned(next)),
                                "dialogue choice next",
                            );
                        }
                        crate::DialogueBody::Jump { target } => {
                            push_generic(
                                &mut out,
                                path,
                                format!("{nbase}/body/target"),
                                Some(node.id),
                                *target,
                                ALLOWED_DIALOGUE_NODE,
                                Some(owned(target)),
                                "dialogue jump",
                            );
                        }
                        crate::DialogueBody::Link { target } => {
                            push_generic(
                                &mut out,
                                path,
                                format!("{nbase}/body/target"),
                                Some(node.id),
                                *target,
                                ALLOWED_DIALOGUE,
                                None,
                                "dialogue link",
                            );
                        }
                        crate::DialogueBody::End => {}
                    }
                }
            }
            Document::Quest(v) => {
                push_generic(
                    &mut out,
                    path,
                    "/entry".into(),
                    Some(v.id),
                    v.entry,
                    ALLOWED_QUEST_STATE,
                    Some(OwnerExpectation {
                        found: state_owner.get(&v.entry).copied(),
                        mine: v.id,
                        container: "quest",
                    }),
                    "quest entry",
                );
                for (i, state) in v.states.iter().enumerate() {
                    let sbase = format!("/states/{i}");
                    for (a, call) in state.on_enter.iter().enumerate() {
                        inventory_args(
                            &mut out,
                            path,
                            format!("{sbase}/on_enter/{a}/args"),
                            &call.args,
                            Some(state.id),
                        );
                    }
                    for (j, transition) in state.transitions.iter().enumerate() {
                        push_generic(
                            &mut out,
                            path,
                            format!("{sbase}/transitions/{j}/target"),
                            Some(state.id),
                            transition.target,
                            ALLOWED_QUEST_STATE,
                            Some(OwnerExpectation {
                                found: state_owner.get(&transition.target).copied(),
                                mine: v.id,
                                container: "quest",
                            }),
                            "quest transition",
                        );
                    }
                }
            }
            Document::Faction(v) => {
                for (i, relation) in v.relations.iter().enumerate() {
                    push_generic(
                        &mut out,
                        path,
                        format!("/relations/{i}/faction"),
                        Some(v.id),
                        relation.faction,
                        ALLOWED_FACTION,
                        None,
                        "faction relation",
                    );
                }
            }
            Document::Placements(v) => {
                push_owner(&mut out, path, "/area", v.area, "placements");
                for (i, placement) in v.placements.iter().enumerate() {
                    let pbase = format!("/placements/{i}");
                    push_generic(
                        &mut out,
                        path,
                        format!("{pbase}/prefab"),
                        Some(placement.id),
                        placement.prefab,
                        ALLOWED_CREATURE_ITEM,
                        None,
                        "placement prefab",
                    );
                    for (key, value) in &placement.overrides {
                        inventory_value(
                            &mut out,
                            path,
                            format!("{pbase}/overrides/{}", escape_segment(key)),
                            value,
                            Some(placement.id),
                        );
                    }
                }
            }
            Document::Triggers(v) => {
                push_owner(&mut out, path, "/area", v.area, "triggers");
                for (i, graph) in v.graphs.iter().enumerate() {
                    inventory_graph(&mut out, path, format!("/graphs/{i}"), graph, &node_owner);
                }
            }
            Document::Graph(v) => {
                inventory_graph(&mut out, path, String::new(), v, &node_owner);
            }
            Document::Locale(v) => {
                push_owner(&mut out, path, "/campaign", v.campaign, "locale");
            }
            Document::Variables(v) => {
                push_owner(&mut out, path, "/campaign", v.campaign, "variables");
                for (i, decl) in v.variables.iter().enumerate() {
                    inventory_var(
                        &mut out,
                        path,
                        format!("/variables/{i}"),
                        &decl.default,
                        None,
                    );
                }
            }
            Document::CampaignLock(_) | Document::AssetsLock(_) => {}
        }
    }
    for (path, document) in documents {
        if let Document::AssetsLock(lock) = document {
            for (asset, record) in &lock.assets {
                inventory_args(
                    &mut out,
                    path,
                    format!("/assets/{}/import", escape_segment(asset.as_str())),
                    &record.import,
                    None,
                );
            }
        }
    }
    out
}
