//! Collected semantic diagnostics over a loaded campaign.
//!
//! T010 structural acceptance stays fail-fast: one typed [`DataError`] for the
//! first defect in fixed phase order. [`validate`] runs only after
//! [`load_campaign`](crate::load_campaign) succeeds and returns every
//! independent semantic finding — dangling references, wrong kinds, duplicate
//! slugs, aggregate ownership, graph consistency, reachability, locale
//! coverage, asset references, and variable default types — as positioned
//! [`Diagnostic`] values in deterministic order. [`validate_files`] is the
//! file-map entry point that unifies both paths for thin consumers, and
//! [`campaign_document_path`] is the data-owned classifier between a caller's
//! directory walk and the loader's accepted layout.

use crate::{
    DataError, DataValue, DialogueBody, Document, EventGraph, LoadedCampaign, NodeBody, ObjectKind,
    Port, SourcePath, ValueType, VarDecl,
};
use crpg_core::Ulid;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Severity of a single diagnostic finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// A failure; validation with any error diagnostic is unsuccessful.
    Error,
    /// Reserved non-fatal information; never fails validation.
    Warning,
}

/// Machine-readable code identifying one diagnostic finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    /// Filesystem collection failure owned by the caller, never this crate.
    Io,
    /// Invalid standalone package coordinate.
    InvalidPackageId,
    /// Invalid standalone logical path.
    InvalidPath,
    /// Invalid standalone lowercase digest.
    InvalidDigest,
    /// Invalid JSON, primitive text or typed document shape.
    Malformed,
    /// Unrecognized schema id or version.
    UnsupportedSchema,
    /// Missing file, case collision or misplaced document.
    Layout,
    /// The caller's engine version does not satisfy the campaign.
    EngineIncompatible,
    /// An object id was published more than once.
    DuplicateId,
    /// Requirements disagree about the kind of a package.
    PackageKindConflict,
    /// An exact candidate version has inconsistent kind or checksum.
    CandidateConflict,
    /// No supplied candidate satisfies all requirements.
    UnresolvedPackage,
    /// A lock violates local or campaign coverage invariants.
    InvalidLock,
    /// Canonical asset-lock bytes do not match the campaign lock.
    AssetsLockMismatch,
    /// A slug repeats within its object kind.
    DuplicateSlug,
    /// A reference names an object id that does not exist.
    DanglingReference,
    /// A reference names an object of an unexpected kind.
    WrongReferenceKind,
    /// A local reference names an object owned by another container.
    ForeignReference,
    /// An aggregate owner does not match its sibling or root owner.
    AggregateOwnerMismatch,
    /// A graph edge uses a port its source node body does not allow.
    GraphPortMismatch,
    /// Two edges share one source node and output port.
    DuplicateGraphPort,
    /// A node cannot be reached from its container entry.
    UnreachableNode,
    /// A quest has no reachable terminal state.
    QuestNoCompletionPath,
    /// An asset key is absent from the assets lock.
    MissingAsset,
    /// A locale key is absent from a locale table.
    MissingLocaleKey,
    /// A variable default does not match its declared type.
    ValueTypeMismatch,
}

impl DiagnosticCode {
    /// Stable snake_case wire spelling of this code.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Io => "io",
            Self::InvalidPackageId => "invalid_package_id",
            Self::InvalidPath => "invalid_path",
            Self::InvalidDigest => "invalid_digest",
            Self::Malformed => "malformed",
            Self::UnsupportedSchema => "unsupported_schema",
            Self::Layout => "layout",
            Self::EngineIncompatible => "engine_incompatible",
            Self::DuplicateId => "duplicate_id",
            Self::PackageKindConflict => "package_kind_conflict",
            Self::CandidateConflict => "candidate_conflict",
            Self::UnresolvedPackage => "unresolved_package",
            Self::InvalidLock => "invalid_lock",
            Self::AssetsLockMismatch => "assets_lock_mismatch",
            Self::DuplicateSlug => "duplicate_slug",
            Self::DanglingReference => "dangling_reference",
            Self::WrongReferenceKind => "wrong_reference_kind",
            Self::ForeignReference => "foreign_reference",
            Self::AggregateOwnerMismatch => "aggregate_owner_mismatch",
            Self::GraphPortMismatch => "graph_port_mismatch",
            Self::DuplicateGraphPort => "duplicate_graph_port",
            Self::UnreachableNode => "unreachable_node",
            Self::QuestNoCompletionPath => "quest_no_completion_path",
            Self::MissingAsset => "missing_asset",
            Self::MissingLocaleKey => "missing_locale_key",
            Self::ValueTypeMismatch => "value_type_mismatch",
        }
    }
}

/// One positioned validation finding with a stable machine shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    /// Logical source of the containing object, if one truthfully exists.
    pub file: Option<SourcePath>,
    /// RFC 6901 pointer into the file; empty means the document root.
    pub pointer: String,
    /// Error fails validation; warning never does.
    pub severity: Severity,
    /// Machine-readable finding code.
    pub code: DiagnosticCode,
    /// Human-readable detail naming the offending value.
    pub message: String,
    /// Actionable repair hint, when one is known.
    pub suggested_fix: Option<String>,
}

impl fmt::Display for Severity {
    /// Renders the stable lowercase wire spelling.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Error => f.write_str("error"),
            Self::Warning => f.write_str("warning"),
        }
    }
}

impl fmt::Display for DiagnosticCode {
    /// Delegates to [`DiagnosticCode::as_str`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for Diagnostic {
    /// Renders one stable positioned line, with a fix suffix only when set.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let file = self
            .file
            .as_ref()
            .map(SourcePath::as_str)
            .unwrap_or("<campaign>");
        match &self.suggested_fix {
            Some(fix) => write!(
                f,
                "{}{}: {}[{}]: {}; suggested fix: {}",
                file, self.pointer, self.severity, self.code, self.message, fix
            ),
            None => write!(
                f,
                "{}{}: {}[{}]: {}",
                file, self.pointer, self.severity, self.code, self.message
            ),
        }
    }
}

/// Maps every structural [`DataError`] to its same-named diagnostic.
///
/// Uses the most precise path the error carries; [`DataError::DuplicateId`]
/// reports the second path at the document root because the error retains no
/// nested pointer. Structural diagnostics carry no suggested fix.
pub fn diagnostic_for_data_error(error: &DataError) -> Diagnostic {
    let (file, code) = match error {
        DataError::InvalidPackageId { .. } => (None, DiagnosticCode::InvalidPackageId),
        DataError::InvalidPath { .. } => (None, DiagnosticCode::InvalidPath),
        DataError::InvalidDigest { .. } => (None, DiagnosticCode::InvalidDigest),
        DataError::Malformed { path, .. } => (path.clone(), DiagnosticCode::Malformed),
        DataError::UnsupportedSchema { path, .. } => {
            (path.clone(), DiagnosticCode::UnsupportedSchema)
        }
        DataError::Layout { path, .. } => (path.clone(), DiagnosticCode::Layout),
        DataError::EngineIncompatible { .. } => (None, DiagnosticCode::EngineIncompatible),
        DataError::DuplicateId { second, .. } => {
            (Some(second.clone()), DiagnosticCode::DuplicateId)
        }
        DataError::PackageKindConflict { .. } => (None, DiagnosticCode::PackageKindConflict),
        DataError::CandidateConflict { .. } => (None, DiagnosticCode::CandidateConflict),
        DataError::UnresolvedPackage { .. } => (None, DiagnosticCode::UnresolvedPackage),
        DataError::InvalidLock { .. } => (None, DiagnosticCode::InvalidLock),
        DataError::AssetsLockMismatch { .. } => (None, DiagnosticCode::AssetsLockMismatch),
    };
    Diagnostic {
        file,
        pointer: String::new(),
        severity: Severity::Error,
        code,
        message: error.to_string(),
        suggested_fix: None,
    }
}

/// Validates a file map without trusting a previous load.
///
/// Calls [`load_campaign`](crate::load_campaign) exactly once: a structural
/// failure yields its single converted diagnostic, success yields
/// [`validate`] over the loaded campaign.
pub fn validate_files(
    files: &BTreeMap<SourcePath, Vec<u8>>,
    engine_version: &Version,
) -> Vec<Diagnostic> {
    match crate::load_campaign(files, engine_version) {
        Ok(loaded) => validate(&loaded),
        Err(error) => vec![diagnostic_for_data_error(&error)],
    }
}

/// Classifies a campaign-root-relative logical path for a directory walk.
///
/// The input is already joined with `/`, never an OS path. Returns `Some`
/// for the exact T010 document path families and required lock/root paths,
/// `None` for source assets, copied schemas, Lua, replays, build output, and
/// unrelated files, and [`DataError::InvalidPath`] for an accepted-family
/// candidate whose logical text violates [`SourcePath`]. Shares its predicates
/// with the loader layout check; classification is pure and case-sensitive.
pub fn campaign_document_path(value: &str) -> Result<Option<SourcePath>, DataError> {
    use crate::loader::{area_file, family, locale_shape};
    let candidate = value == "campaign.json"
        || value == "campaign.lock"
        || value == "assets/assets.lock"
        || value == "variables/campaign_state.json"
        || family(value, "worlds/")
        || family(value, "creatures/")
        || family(value, "items/")
        || family(value, "dialogue/")
        || family(value, "quests/")
        || family(value, "factions/")
        || family(value, "scripts/graphs/")
        || area_file(value, "area.json")
        || area_file(value, "placements.json")
        || area_file(value, "triggers.json")
        || locale_shape(value);
    if !candidate {
        return Ok(None);
    }
    value.parse().map(Some)
}

/// Stable lowercase kind word used inside diagnostic messages.
fn kind_name(kind: ObjectKind) -> &'static str {
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
fn escape(segment: &str) -> String {
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

/// Stable wire word for a declared value category.
fn type_name(value_type: &ValueType) -> &'static str {
    match value_type {
        ValueType::Bool => "bool",
        ValueType::Integer => "integer",
        ValueType::Unsigned => "unsigned",
        ValueType::Fixed => "fixed",
        ValueType::Text => "text",
        ValueType::ObjectRef => "object_ref",
        ValueType::List => "list",
        ValueType::Map => "map",
    }
}

/// Stable wire word for a tagged value's outer variant.
fn value_kind_name(value: &DataValue) -> &'static str {
    match value {
        DataValue::Bool(_) => "bool",
        DataValue::Integer(_) => "integer",
        DataValue::Unsigned(_) => "unsigned",
        DataValue::Fixed(_) => "fixed",
        DataValue::Text(_) => "text",
        DataValue::ObjectRef(_) => "object_ref",
        DataValue::List(_) => "list",
        DataValue::Map(_) => "map",
    }
}

/// Whether a declaration's outer value variant matches its declared type.
///
/// `List` and `Map` describe only the outer container; nested values may be
/// heterogeneous, so no element rule applies here.
fn var_outer_matches(decl: &VarDecl) -> bool {
    matches!(
        (&decl.value_type, &decl.default),
        (ValueType::Bool, DataValue::Bool(_))
            | (ValueType::Integer, DataValue::Integer(_))
            | (ValueType::Unsigned, DataValue::Unsigned(_))
            | (ValueType::Fixed, DataValue::Fixed(_))
            | (ValueType::Text, DataValue::Text(_))
            | (ValueType::ObjectRef, DataValue::ObjectRef(_))
            | (ValueType::List, DataValue::List(_))
            | (ValueType::Map, DataValue::Map(_))
    )
}

/// Stable word for a graph output port used inside messages.
fn port_name(port: &Port) -> String {
    match port {
        Port::Next => "next".into(),
        Port::True => "true".into(),
        Port::False => "false".into(),
        Port::Case { index } => format!("case {index}"),
    }
}

/// Stable word for a node body used inside messages.
fn body_name(body: &NodeBody) -> &'static str {
    match body {
        NodeBody::Condition { .. } => "condition",
        NodeBody::Action { .. } => "action",
        NodeBody::Branch { .. } => "branch",
        NodeBody::Sequence { .. } => "sequence",
        NodeBody::Wait { .. } => "wait",
        NodeBody::CallScript { .. } => "call_script",
        NodeBody::CallGraph { .. } => "call_graph",
    }
}

/// Whether an edge port is allowed by its resolved local source node body.
///
/// Conditions take true/false, branches take in-range zero-based cases, and
/// every other body takes next.
fn port_allowed(body: &NodeBody, port: &Port) -> bool {
    match (body, port) {
        (NodeBody::Condition { .. }, Port::True) | (NodeBody::Condition { .. }, Port::False) => {
            true
        }
        (NodeBody::Branch { cases, .. }, Port::Case { index }) => (*index as usize) < cases.len(),
        (NodeBody::Condition { .. }, _) | (NodeBody::Branch { .. }, _) => false,
        (_, Port::Next) => true,
        (_, _) => false,
    }
}

/// Discriminator for duplicate-port detection within one graph.
fn port_key(port: &Port) -> String {
    match port {
        Port::Next => "next".into(),
        Port::True => "true".into(),
        Port::False => "false".into(),
        Port::Case { index } => format!("case:{index}"),
    }
}

/// Every indexed object kind; object references accept any of them.
const ALL_KINDS: [ObjectKind; 13] = [
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

/// Lookup tables rebuilt from documents, never from the caller-mutable index.
struct Tables {
    /// First occurrence of each id in lexical-path, authored order.
    first: BTreeMap<Ulid, crate::IndexEntry>,
    /// Graph node id to owning graph id.
    node_owner: BTreeMap<Ulid, Ulid>,
    /// Dialogue node id to owning dialogue id.
    dnode_owner: BTreeMap<Ulid, Ulid>,
    /// Quest state id to owning quest id.
    state_owner: BTreeMap<Ulid, Ulid>,
}

/// Local-ownership expectation for one reference check.
struct OwnerCheck {
    /// Owner recorded for the id, if it is an identified local child at all.
    found: Option<Ulid>,
    /// Id of the container holding the reference.
    mine: Ulid,
    /// Kind word of the container used inside messages.
    container: &'static str,
}

/// Pushes one error diagnostic with an optional repair hint.
fn push(
    out: &mut Vec<Diagnostic>,
    file: &SourcePath,
    pointer: String,
    code: DiagnosticCode,
    message: String,
    fix: Option<&str>,
) {
    out.push(Diagnostic {
        file: Some(file.clone()),
        pointer,
        severity: Severity::Error,
        code,
        message,
        suggested_fix: fix.map(str::to_string),
    });
}

/// Checks one typed reference at its exact pointer.
///
/// A missing id is dangling, a present id outside `allowed` is the wrong
/// kind, and a present id of the right kind owned by another container is
/// foreign. At most one diagnostic is emitted per reference.
#[allow(clippy::too_many_arguments)]
fn check_ref(
    tables: &Tables,
    out: &mut Vec<Diagnostic>,
    file: &SourcePath,
    pointer: String,
    id: Ulid,
    allowed: &[ObjectKind],
    owner: Option<OwnerCheck>,
    what: &str,
) {
    let Some(entry) = tables.first.get(&id) else {
        push(
            out,
            file,
            pointer,
            DiagnosticCode::DanglingReference,
            format!("dangling {what} reference to {id}"),
            Some("point at an existing object id or create the target"),
        );
        return;
    };
    if !allowed.contains(&entry.kind) {
        let expected = allowed
            .iter()
            .map(|kind| kind_name(*kind))
            .collect::<Vec<_>>()
            .join(" or ");
        push(
            out,
            file,
            pointer,
            DiagnosticCode::WrongReferenceKind,
            format!(
                "wrong kind for {what} reference to {id}: found {}, expected {expected}",
                kind_name(entry.kind)
            ),
            Some("point at an object of the expected kind"),
        );
        return;
    }
    if let Some(check) = owner {
        if check.found != Some(check.mine) {
            let other = match check.found {
                Some(other) => format!("{} {other}", check.container),
                None => "no container".into(),
            };
            push(
                out,
                file,
                pointer,
                DiagnosticCode::ForeignReference,
                format!(
                    "{what} reference to {id} belongs to {other}, expected {} {}",
                    check.container, check.mine
                ),
                Some("point at an object owned by this container"),
            );
        }
    }
}

/// Recursively checks object references inside one tagged value.
///
/// The reference diagnostic sits at the tagged value's `/value` member; list
/// and map segments extend the base pointer with escaping.
fn walk_value(
    tables: &Tables,
    out: &mut Vec<Diagnostic>,
    file: &SourcePath,
    base: String,
    value: &DataValue,
) {
    match value {
        DataValue::ObjectRef(id) => {
            let mut pointer = base;
            pointer.push_str("/value");
            check_ref(tables, out, file, pointer, *id, &ALL_KINDS, None, "object");
        }
        DataValue::List(items) => {
            for (i, item) in items.iter().enumerate() {
                walk_value(tables, out, file, format!("{base}/{i}"), item);
            }
        }
        DataValue::Map(entries) => {
            for (key, item) in entries {
                walk_value(tables, out, file, format!("{base}/{}", escape(key)), item);
            }
        }
        _ => {}
    }
}

/// Checks one named argument map of tagged values.
fn walk_args(
    tables: &Tables,
    out: &mut Vec<Diagnostic>,
    file: &SourcePath,
    base: String,
    args: &BTreeMap<String, DataValue>,
) {
    for (key, value) in args {
        walk_value(tables, out, file, format!("{base}/{}", escape(key)), value);
    }
}

/// Checks one variable declaration's default type and nested references.
fn walk_var(
    tables: &Tables,
    out: &mut Vec<Diagnostic>,
    file: &SourcePath,
    base: String,
    decl: &VarDecl,
) {
    let mut at_default = base;
    at_default.push_str("/default");
    if !var_outer_matches(decl) {
        push(
            out,
            file,
            at_default.clone(),
            DiagnosticCode::ValueTypeMismatch,
            format!(
                "variable {:?} declares {} but the default is {}",
                decl.name,
                type_name(&decl.value_type),
                value_kind_name(&decl.default)
            ),
            Some("change the default value or the declared type"),
        );
    }
    walk_value(tables, out, file, at_default, &decl.default);
}

/// Validates a loaded campaign and returns every finding in stable order.
///
/// Pure, deterministic, and non-mutating: occurrence and ownership tables are
/// rebuilt from `documents` in lexical-path, authored order without trusting
/// the caller-mutable index. Findings sort by file, pointer, code, severity,
/// message, then suggested fix; a clean campaign yields an empty vector.
pub fn validate(campaign: &LoadedCampaign) -> Vec<Diagnostic> {
    let docs = &campaign.documents;
    // Occurrence order matches the loader: lexical paths, root first, then
    // identified array entries in authored order.
    let mut occurrences: Vec<(Ulid, ObjectKind, &SourcePath, String)> = Vec::new();
    for (path, document) in docs {
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
            occurrences.push((id, kind, path, String::new()));
        }
        match document {
            Document::Placements(v) => {
                for (i, p) in v.placements.iter().enumerate() {
                    occurrences.push((
                        p.id,
                        ObjectKind::Placement,
                        path,
                        format!("/placements/{i}"),
                    ));
                }
            }
            Document::Triggers(v) => {
                for (i, graph) in v.graphs.iter().enumerate() {
                    occurrences.push((graph.id, ObjectKind::Graph, path, format!("/graphs/{i}")));
                    for (j, node) in graph.nodes.iter().enumerate() {
                        occurrences.push((
                            node.id,
                            ObjectKind::Node,
                            path,
                            format!("/graphs/{i}/nodes/{j}"),
                        ));
                    }
                }
            }
            Document::Graph(v) => {
                for (i, node) in v.nodes.iter().enumerate() {
                    occurrences.push((node.id, ObjectKind::Node, path, format!("/nodes/{i}")));
                }
            }
            Document::Dialogue(v) => {
                for (i, node) in v.nodes.iter().enumerate() {
                    occurrences.push((
                        node.id,
                        ObjectKind::DialogueNode,
                        path,
                        format!("/nodes/{i}"),
                    ));
                }
            }
            Document::Quest(v) => {
                for (i, state) in v.states.iter().enumerate() {
                    occurrences.push((
                        state.id,
                        ObjectKind::QuestState,
                        path,
                        format!("/states/{i}"),
                    ));
                }
            }
            _ => {}
        }
    }

    let mut tables = Tables {
        first: BTreeMap::new(),
        node_owner: BTreeMap::new(),
        dnode_owner: BTreeMap::new(),
        state_owner: BTreeMap::new(),
    };
    let mut out: Vec<Diagnostic> = Vec::new();
    // First occurrence wins for lookup; every later one is a duplicate id at
    // its own `/id` pointer naming the first location.
    for (id, kind, path, pointer) in occurrences {
        match tables.first.get(&id) {
            Some(seen) => {
                let mut at_id = pointer;
                at_id.push_str("/id");
                push(
                    &mut out,
                    path,
                    at_id,
                    DiagnosticCode::DuplicateId,
                    format!(
                        "duplicate id {id} (first at {}{})",
                        seen.path.as_str(),
                        seen.pointer
                    ),
                    Some("assign a new ULID to the later object"),
                );
                let _ = kind;
            }
            None => {
                tables.first.insert(
                    id,
                    crate::IndexEntry {
                        kind,
                        path: path.clone(),
                        pointer,
                    },
                );
            }
        }
    }

    // Ownership tables, first wins in the same order.
    for document in docs.values() {
        match document {
            Document::Triggers(v) => {
                for graph in &v.graphs {
                    for node in &graph.nodes {
                        tables.node_owner.entry(node.id).or_insert(graph.id);
                    }
                }
            }
            Document::Graph(v) => {
                for node in &v.nodes {
                    tables.node_owner.entry(node.id).or_insert(v.id);
                }
            }
            Document::Dialogue(v) => {
                for node in &v.nodes {
                    tables.dnode_owner.entry(node.id).or_insert(v.id);
                }
            }
            Document::Quest(v) => {
                for state in &v.states {
                    tables.state_owner.entry(state.id).or_insert(v.id);
                }
            }
            _ => {}
        }
    }

    // Locale tables in lexical file order for coverage checks.
    let mut locales: Vec<(&SourcePath, &str, &BTreeMap<String, String>)> = Vec::new();
    for (path, document) in docs {
        if let Document::Locale(v) = document {
            locales.push((path, v.locale.as_str(), &v.strings));
        }
    }

    let root_campaign: Option<Ulid> = docs.values().find_map(|d| match d {
        Document::Campaign(v) => Some(v.id),
        _ => None,
    });
    // Sibling area id for an aggregate file, if its area.json holds an Area.
    let sibling_area = |file: &SourcePath, sibling: &str| -> Option<Ulid> {
        let stem = file
            .as_str()
            .strip_prefix("areas/")?
            .strip_suffix(sibling)?;
        let area_path: SourcePath = format!("areas/{stem}area.json").parse().ok()?;
        match docs.get(&area_path) {
            Some(Document::Area(area)) => Some(area.id),
            _ => None,
        }
    };
    // One specialized owner check with no generic reference diagnostic.
    let check_owner = |tables: &Tables,
                       out: &mut Vec<Diagnostic>,
                       file: &SourcePath,
                       pointer: &str,
                       id: Ulid,
                       expected: Option<Ulid>,
                       what: &str| {
        let _ = tables;
        match expected {
            Some(wanted) if wanted == id => {}
            Some(wanted) => push(
                out,
                file,
                pointer.into(),
                DiagnosticCode::AggregateOwnerMismatch,
                format!("{what} owner {id} does not match the required owner {wanted}"),
                Some("set the owner to the sibling or root id"),
            ),
            None => push(
                out,
                file,
                pointer.into(),
                DiagnosticCode::AggregateOwnerMismatch,
                format!("{what} owner {id} has no sibling or root document to match"),
                Some("set the owner to the sibling or root id"),
            ),
        }
    };

    // (kind, slug, file, slug pointer) for the later uniqueness pass.
    let mut slugs: Vec<(ObjectKind, String, SourcePath, String)> = Vec::new();
    // (file, pointer, key) for the later locale coverage pass.
    let mut locale_refs: Vec<(SourcePath, String, String)> = Vec::new();

    for (path, document) in docs {
        match document {
            Document::Campaign(v) => {
                let mut pointer = String::new();
                pointer.push_str("/slug");
                slugs.push((ObjectKind::Campaign, v.slug.clone(), path.clone(), pointer));
                locale_refs.push((path.clone(), "/name".into(), v.name.clone()));
                check_ref(
                    &tables,
                    &mut out,
                    path,
                    "/entry/world".into(),
                    v.entry.world,
                    &[ObjectKind::World],
                    None,
                    "entry world",
                );
                check_ref(
                    &tables,
                    &mut out,
                    path,
                    "/entry/area".into(),
                    v.entry.area,
                    &[ObjectKind::Area],
                    None,
                    "entry area",
                );
                check_ref(
                    &tables,
                    &mut out,
                    path,
                    "/entry/spawn".into(),
                    v.entry.spawn,
                    &[ObjectKind::Placement],
                    None,
                    "entry spawn",
                );
            }
            Document::World(v) => {
                slugs.push((
                    ObjectKind::World,
                    v.slug.clone(),
                    path.clone(),
                    "/slug".into(),
                ));
                locale_refs.push((path.clone(), "/name".into(), v.name.clone()));
                for (i, area) in v.areas.iter().enumerate() {
                    check_ref(
                        &tables,
                        &mut out,
                        path,
                        format!("/areas/{i}"),
                        *area,
                        &[ObjectKind::Area],
                        None,
                        "world area",
                    );
                }
                for (i, decl) in v.variables.iter().enumerate() {
                    walk_var(&tables, &mut out, path, format!("/variables/{i}"), decl);
                }
            }
            Document::Area(v) => {
                slugs.push((
                    ObjectKind::Area,
                    v.slug.clone(),
                    path.clone(),
                    "/slug".into(),
                ));
                locale_refs.push((path.clone(), "/name".into(), v.name.clone()));
                for (i, neighbour) in v.neighbours.iter().enumerate() {
                    check_ref(
                        &tables,
                        &mut out,
                        path,
                        format!("/neighbours/{i}"),
                        *neighbour,
                        &[ObjectKind::Area],
                        None,
                        "area neighbour",
                    );
                }
                if let Some(key) = &v.ambience {
                    let known = docs
                        .values()
                        .find_map(|d| match d {
                            Document::AssetsLock(lock) => Some(lock),
                            _ => None,
                        })
                        .is_some_and(|lock| lock.assets.keys().any(|p| p.as_str() == key));
                    if !known {
                        push(
                            &mut out,
                            path,
                            "/ambience".into(),
                            DiagnosticCode::MissingAsset,
                            format!("asset key {key:?} is not in assets.lock"),
                            Some("add the asset to assets.lock or fix the key"),
                        );
                    }
                }
            }
            Document::Creature(v) => {
                slugs.push((
                    ObjectKind::Creature,
                    v.slug.clone(),
                    path.clone(),
                    "/slug".into(),
                ));
                locale_refs.push((path.clone(), "/name".into(), v.name.clone()));
                if let Some(faction) = v.faction {
                    check_ref(
                        &tables,
                        &mut out,
                        path,
                        "/faction".into(),
                        faction,
                        &[ObjectKind::Faction],
                        None,
                        "creature faction",
                    );
                }
                for (i, item) in v.inventory.iter().enumerate() {
                    check_ref(
                        &tables,
                        &mut out,
                        path,
                        format!("/inventory/{i}"),
                        *item,
                        &[ObjectKind::Item],
                        None,
                        "creature inventory item",
                    );
                }
            }
            Document::Item(v) => {
                slugs.push((
                    ObjectKind::Item,
                    v.slug.clone(),
                    path.clone(),
                    "/slug".into(),
                ));
                locale_refs.push((path.clone(), "/name".into(), v.name.clone()));
            }
            Document::Dialogue(v) => {
                slugs.push((
                    ObjectKind::Dialogue,
                    v.slug.clone(),
                    path.clone(),
                    "/slug".into(),
                ));
                locale_refs.push((path.clone(), "/name".into(), v.name.clone()));
                check_ref(
                    &tables,
                    &mut out,
                    path,
                    "/entry".into(),
                    v.entry,
                    &[ObjectKind::DialogueNode],
                    Some(OwnerCheck {
                        found: tables.dnode_owner.get(&v.entry).copied(),
                        mine: v.id,
                        container: "dialogue",
                    }),
                    "dialogue entry",
                );
                for (i, node) in v.nodes.iter().enumerate() {
                    let nbase = format!("/nodes/{i}");
                    let owned = |id: &Ulid| OwnerCheck {
                        found: tables.dnode_owner.get(id).copied(),
                        mine: v.id,
                        container: "dialogue",
                    };
                    match &node.body {
                        DialogueBody::NpcLine {
                            speaker,
                            text_key,
                            on_enter,
                            next,
                            ..
                        } => {
                            check_ref(
                                &tables,
                                &mut out,
                                path,
                                format!("{nbase}/body/speaker"),
                                *speaker,
                                &[ObjectKind::Creature, ObjectKind::Placement],
                                None,
                                "dialogue speaker",
                            );
                            locale_refs.push((
                                path.clone(),
                                format!("{nbase}/body/text_key"),
                                text_key.clone(),
                            ));
                            for (a, call) in on_enter.iter().enumerate() {
                                walk_args(
                                    &tables,
                                    &mut out,
                                    path,
                                    format!("{nbase}/body/on_enter/{a}/args"),
                                    &call.args,
                                );
                            }
                            if let Some(next) = next {
                                check_ref(
                                    &tables,
                                    &mut out,
                                    path,
                                    format!("{nbase}/body/next"),
                                    *next,
                                    &[ObjectKind::DialogueNode],
                                    Some(owned(next)),
                                    "dialogue next",
                                );
                            }
                        }
                        DialogueBody::PlayerChoice {
                            text_key,
                            on_select,
                            next,
                            ..
                        } => {
                            locale_refs.push((
                                path.clone(),
                                format!("{nbase}/body/text_key"),
                                text_key.clone(),
                            ));
                            for (a, call) in on_select.iter().enumerate() {
                                walk_args(
                                    &tables,
                                    &mut out,
                                    path,
                                    format!("{nbase}/body/on_select/{a}/args"),
                                    &call.args,
                                );
                            }
                            check_ref(
                                &tables,
                                &mut out,
                                path,
                                format!("{nbase}/body/next"),
                                *next,
                                &[ObjectKind::DialogueNode],
                                Some(owned(next)),
                                "dialogue choice next",
                            );
                        }
                        DialogueBody::Jump { target } => {
                            check_ref(
                                &tables,
                                &mut out,
                                path,
                                format!("{nbase}/body/target"),
                                *target,
                                &[ObjectKind::DialogueNode],
                                Some(owned(target)),
                                "dialogue jump",
                            );
                        }
                        DialogueBody::Link { target } => {
                            check_ref(
                                &tables,
                                &mut out,
                                path,
                                format!("{nbase}/body/target"),
                                *target,
                                &[ObjectKind::Dialogue],
                                None,
                                "dialogue link",
                            );
                        }
                        DialogueBody::End => {}
                    }
                }
                walk_dialogue_reachability(&tables, &mut out, path, v);
            }
            Document::Quest(v) => {
                slugs.push((
                    ObjectKind::Quest,
                    v.slug.clone(),
                    path.clone(),
                    "/slug".into(),
                ));
                locale_refs.push((path.clone(), "/name".into(), v.name.clone()));
                check_ref(
                    &tables,
                    &mut out,
                    path,
                    "/entry".into(),
                    v.entry,
                    &[ObjectKind::QuestState],
                    Some(OwnerCheck {
                        found: tables.state_owner.get(&v.entry).copied(),
                        mine: v.id,
                        container: "quest",
                    }),
                    "quest entry",
                );
                for (i, state) in v.states.iter().enumerate() {
                    let sbase = format!("/states/{i}");
                    locale_refs.push((path.clone(), format!("{sbase}/name"), state.name.clone()));
                    for (a, call) in state.on_enter.iter().enumerate() {
                        walk_args(
                            &tables,
                            &mut out,
                            path,
                            format!("{sbase}/on_enter/{a}/args"),
                            &call.args,
                        );
                    }
                    for (j, transition) in state.transitions.iter().enumerate() {
                        check_ref(
                            &tables,
                            &mut out,
                            path,
                            format!("{sbase}/transitions/{j}/target"),
                            transition.target,
                            &[ObjectKind::QuestState],
                            Some(OwnerCheck {
                                found: tables.state_owner.get(&transition.target).copied(),
                                mine: v.id,
                                container: "quest",
                            }),
                            "quest transition",
                        );
                    }
                }
                walk_quest_completion(&tables, &mut out, path, v);
            }
            Document::Faction(v) => {
                slugs.push((
                    ObjectKind::Faction,
                    v.slug.clone(),
                    path.clone(),
                    "/slug".into(),
                ));
                locale_refs.push((path.clone(), "/name".into(), v.name.clone()));
                for (i, relation) in v.relations.iter().enumerate() {
                    check_ref(
                        &tables,
                        &mut out,
                        path,
                        format!("/relations/{i}/faction"),
                        relation.faction,
                        &[ObjectKind::Faction],
                        None,
                        "faction relation",
                    );
                }
            }
            Document::Placements(v) => {
                check_owner(
                    &tables,
                    &mut out,
                    path,
                    "/area",
                    v.area,
                    sibling_area(path, "placements.json"),
                    "placements",
                );
                for (i, placement) in v.placements.iter().enumerate() {
                    let pbase = format!("/placements/{i}");
                    slugs.push((
                        ObjectKind::Placement,
                        placement.slug.clone(),
                        path.clone(),
                        format!("{pbase}/slug"),
                    ));
                    locale_refs.push((
                        path.clone(),
                        format!("{pbase}/name"),
                        placement.name.clone(),
                    ));
                    check_ref(
                        &tables,
                        &mut out,
                        path,
                        format!("{pbase}/prefab"),
                        placement.prefab,
                        &[ObjectKind::Creature, ObjectKind::Item],
                        None,
                        "placement prefab",
                    );
                    for (key, value) in &placement.overrides {
                        walk_value(
                            &tables,
                            &mut out,
                            path,
                            format!("{pbase}/overrides/{}", escape(key)),
                            value,
                        );
                    }
                }
            }
            Document::Triggers(v) => {
                check_owner(
                    &tables,
                    &mut out,
                    path,
                    "/area",
                    v.area,
                    sibling_area(path, "triggers.json"),
                    "triggers",
                );
                for (i, graph) in v.graphs.iter().enumerate() {
                    walk_graph(
                        &tables,
                        &mut out,
                        &mut slugs,
                        &mut locale_refs,
                        path,
                        format!("/graphs/{i}"),
                        graph,
                    );
                }
            }
            Document::Graph(v) => {
                walk_graph(
                    &tables,
                    &mut out,
                    &mut slugs,
                    &mut locale_refs,
                    path,
                    String::new(),
                    v,
                );
            }
            Document::Locale(v) => {
                check_owner(
                    &tables,
                    &mut out,
                    path,
                    "/campaign",
                    v.campaign,
                    root_campaign,
                    "locale",
                );
            }
            Document::Variables(v) => {
                check_owner(
                    &tables,
                    &mut out,
                    path,
                    "/campaign",
                    v.campaign,
                    root_campaign,
                    "variables",
                );
                for (i, decl) in v.variables.iter().enumerate() {
                    walk_var(&tables, &mut out, path, format!("/variables/{i}"), decl);
                }
            }
            Document::CampaignLock(_) | Document::AssetsLock(_) => {}
        }
    }

    // Asset import settings may also carry object references.
    for (path, document) in docs {
        if let Document::AssetsLock(lock) = document {
            for (asset, record) in &lock.assets {
                walk_args(
                    &tables,
                    &mut out,
                    path,
                    format!("/assets/{}/import", escape(asset.as_str())),
                    &record.import,
                );
            }
        }
    }

    // Slugs are unique within kind: earliest (path, pointer) wins.
    slugs.sort_by(|a, b| (&a.2, &a.3).cmp(&(&b.2, &b.3)));
    let mut seen_slugs: BTreeMap<(ObjectKind, &str), (&SourcePath, &str)> = BTreeMap::new();
    for (kind, slug, file, pointer) in &slugs {
        match seen_slugs.get(&(*kind, slug.as_str())) {
            Some((first_path, first_pointer)) => push(
                &mut out,
                file,
                pointer.clone(),
                DiagnosticCode::DuplicateSlug,
                format!(
                    "duplicate slug {slug:?} for {} (first at {first_path}{first_pointer})",
                    kind_name(*kind)
                ),
                Some("rename the later slug to a unique value within its type"),
            ),
            None => {
                seen_slugs.insert((*kind, slug.as_str()), (file, pointer.as_str()));
            }
        }
    }

    // Locale coverage: every name and dialogue text key must exist in every
    // loaded locale table; unused strings are valid.
    if locales.is_empty() {
        let mut unique: BTreeSet<(&SourcePath, &str)> = BTreeSet::new();
        for (file, pointer, _) in &locale_refs {
            unique.insert((file, pointer.as_str()));
        }
        for (file, pointer) in unique {
            out.push(Diagnostic {
                file: Some(file.clone()),
                pointer: pointer.into(),
                severity: Severity::Error,
                code: DiagnosticCode::MissingLocaleKey,
                message: "locale key has no locale table to resolve against".into(),
                suggested_fix: Some("add a locale table containing the key".into()),
            });
        }
    } else {
        for (file, pointer, key) in &locale_refs {
            for (_, locale, strings) in &locales {
                if !strings.contains_key(key) {
                    out.push(Diagnostic {
                        file: Some(file.clone()),
                        pointer: pointer.clone(),
                        severity: Severity::Error,
                        code: DiagnosticCode::MissingLocaleKey,
                        message: format!("locale key {key:?} is missing from locale {locale:?}"),
                        suggested_fix: Some(format!("add {key:?} to locale {locale:?}")),
                    });
                }
            }
        }
    }

    // Deterministic order: file (absent first), pointer, code, severity,
    // message, then suggested fix.
    out.sort_by(|a, b| {
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
    out
}

/// Validates one event graph: references, edge ports, then reachability.
///
/// `gbase` is the graph object's pointer within its file (empty for a
/// standalone graph document, `/graphs/{i}` when embedded in triggers).
#[allow(clippy::too_many_arguments)]
fn walk_graph(
    tables: &Tables,
    out: &mut Vec<Diagnostic>,
    slugs: &mut Vec<(ObjectKind, String, SourcePath, String)>,
    locale_refs: &mut Vec<(SourcePath, String, String)>,
    file: &SourcePath,
    gbase: String,
    graph: &EventGraph,
) {
    slugs.push((
        ObjectKind::Graph,
        graph.slug.clone(),
        file.clone(),
        format!("{gbase}/slug"),
    ));
    locale_refs.push((file.clone(), format!("{gbase}/name"), graph.name.clone()));
    for (i, decl) in graph.locals.iter().enumerate() {
        walk_var(tables, out, file, format!("{gbase}/locals/{i}"), decl);
    }
    check_ref(
        tables,
        out,
        file,
        format!("{gbase}/start"),
        graph.start,
        &[ObjectKind::Node],
        Some(OwnerCheck {
            found: tables.node_owner.get(&graph.start).copied(),
            mine: graph.id,
            container: "graph",
        }),
        "graph start",
    );
    for (j, node) in graph.nodes.iter().enumerate() {
        let nbase = format!("{gbase}/nodes/{j}");
        match &node.body {
            NodeBody::Condition { .. } | NodeBody::Wait { .. } => {}
            NodeBody::Action { call } => {
                walk_args(
                    tables,
                    out,
                    file,
                    format!("{nbase}/body/call/args"),
                    &call.args,
                );
            }
            NodeBody::Branch { cases, .. } => {
                for (k, case) in cases.iter().enumerate() {
                    walk_value(tables, out, file, format!("{nbase}/body/cases/{k}"), case);
                }
            }
            NodeBody::Sequence { nodes } => {
                for (k, child) in nodes.iter().enumerate() {
                    check_ref(
                        tables,
                        out,
                        file,
                        format!("{nbase}/body/nodes/{k}"),
                        *child,
                        &[ObjectKind::Node],
                        Some(OwnerCheck {
                            found: tables.node_owner.get(child).copied(),
                            mine: graph.id,
                            container: "graph",
                        }),
                        "sequence child",
                    );
                }
            }
            NodeBody::CallScript { args, .. } => {
                walk_args(tables, out, file, format!("{nbase}/body/args"), args);
            }
            NodeBody::CallGraph { graph_id, args } => {
                check_ref(
                    tables,
                    out,
                    file,
                    format!("{nbase}/body/graph_id"),
                    *graph_id,
                    &[ObjectKind::Graph],
                    None,
                    "graph call",
                );
                walk_args(tables, out, file, format!("{nbase}/body/args"), args);
            }
        }
    }
    // Node lookup for port and reachability analysis, first wins.
    let mut nodes: BTreeMap<Ulid, &NodeBody> = BTreeMap::new();
    for node in &graph.nodes {
        nodes.entry(node.id).or_insert(&node.body);
    }
    let local = |tables: &Tables, id: &Ulid| -> Option<&NodeBody> {
        if tables.node_owner.get(id) == Some(&graph.id) {
            nodes.get(id).copied()
        } else {
            None
        }
    };
    // Edge ports: only edges from a resolved local source are judged; the
    // reference diagnostics above already cover unresolved endpoints.
    let mut used_ports: BTreeSet<(Ulid, String)> = BTreeSet::new();
    for (i, edge) in graph.edges.iter().enumerate() {
        let Some(body) = local(tables, &edge.from) else {
            continue;
        };
        let at_port = format!("{gbase}/edges/{i}/port");
        if !port_allowed(body, &edge.port) {
            push(
                out,
                file,
                at_port.clone(),
                DiagnosticCode::GraphPortMismatch,
                format!(
                    "edge port {} is not allowed for {} source node",
                    port_name(&edge.port),
                    body_name(body)
                ),
                Some("use the port allowed by the source node body"),
            );
        }
        if !used_ports.insert((edge.from, port_key(&edge.port))) {
            push(
                out,
                file,
                at_port,
                DiagnosticCode::DuplicateGraphPort,
                format!(
                    "duplicate edge port {} from {}",
                    port_name(&edge.port),
                    edge.from
                ),
                Some("remove the later edge or use an unused port"),
            );
        }
    }
    // Reachability from a valid start across local edges and sequence
    // children; an invalid start already has its own positioned diagnostic.
    if local(tables, &graph.start).is_none() {
        return;
    }
    let mut reachable: BTreeSet<Ulid> = BTreeSet::new();
    let mut stack = vec![graph.start];
    while let Some(id) = stack.pop() {
        if !reachable.insert(id) {
            continue;
        }
        if let Some(NodeBody::Sequence { nodes: children }) = local(tables, &id) {
            for child in children {
                if local(tables, child).is_some() {
                    stack.push(*child);
                }
            }
        }
        for edge in &graph.edges {
            if edge.from == id && local(tables, &edge.to).is_some() {
                stack.push(edge.to);
            }
        }
    }
    for (j, node) in graph.nodes.iter().enumerate() {
        if !reachable.contains(&node.id) {
            push(
                out,
                file,
                format!("{gbase}/nodes/{j}"),
                DiagnosticCode::UnreachableNode,
                format!("graph node {} is unreachable from the start", node.id),
                Some("connect the node from the entry or remove it"),
            );
        }
    }
}

/// Emits one diagnostic per dialogue node unreachable from a valid entry.
///
/// `End` and `NpcLine` without continuation stop; `Link` leaves the local
/// dialogue. An invalid entry already has its own diagnostic and suppresses
/// this analysis.
fn walk_dialogue_reachability(
    tables: &Tables,
    out: &mut Vec<Diagnostic>,
    file: &SourcePath,
    dialogue: &crate::Dialogue,
) {
    let mut by_id: BTreeMap<Ulid, &crate::DialogueNode> = BTreeMap::new();
    for node in &dialogue.nodes {
        by_id.entry(node.id).or_insert(node);
    }
    let local = |id: &Ulid| -> bool { tables.dnode_owner.get(id) == Some(&dialogue.id) };
    if !local(&dialogue.entry) || !by_id.contains_key(&dialogue.entry) {
        return;
    }
    let mut reachable: BTreeSet<Ulid> = BTreeSet::new();
    let mut stack = vec![dialogue.entry];
    while let Some(id) = stack.pop() {
        if !reachable.insert(id) {
            continue;
        }
        let Some(node) = by_id.get(&id) else {
            continue;
        };
        match &node.body {
            DialogueBody::NpcLine {
                next: Some(next), ..
            } => {
                if local(next) {
                    stack.push(*next);
                }
            }
            DialogueBody::PlayerChoice { next, .. } | DialogueBody::Jump { target: next } => {
                if local(next) {
                    stack.push(*next);
                }
            }
            DialogueBody::NpcLine { next: None, .. }
            | DialogueBody::Link { .. }
            | DialogueBody::End => {}
        }
    }
    for (j, node) in dialogue.nodes.iter().enumerate() {
        if !reachable.contains(&node.id) {
            push(
                out,
                file,
                format!("/nodes/{j}"),
                DiagnosticCode::UnreachableNode,
                format!("dialogue node {} is unreachable from the entry", node.id),
                Some("connect the node from the entry or remove it"),
            );
        }
    }
}

/// Emits one completion diagnostic when no reachable state is terminal.
///
/// Transitions with invalid targets keep their own reference diagnostics and
/// are not followed here; an invalid entry suppresses this analysis.
fn walk_quest_completion(
    tables: &Tables,
    out: &mut Vec<Diagnostic>,
    file: &SourcePath,
    quest: &crate::Quest,
) {
    let mut by_id: BTreeMap<Ulid, &crate::QuestState> = BTreeMap::new();
    for state in &quest.states {
        by_id.entry(state.id).or_insert(state);
    }
    let local = |id: &Ulid| -> bool { tables.state_owner.get(id) == Some(&quest.id) };
    if !local(&quest.entry) || !by_id.contains_key(&quest.entry) {
        return;
    }
    let mut reachable: BTreeSet<Ulid> = BTreeSet::new();
    let mut stack = vec![quest.entry];
    while let Some(id) = stack.pop() {
        if !reachable.insert(id) {
            continue;
        }
        let Some(state) = by_id.get(&id) else {
            continue;
        };
        for transition in &state.transitions {
            if local(&transition.target) {
                stack.push(transition.target);
            }
        }
    }
    let complete = reachable
        .iter()
        .any(|id| by_id.get(id).is_some_and(|state| state.terminal));
    if !complete {
        push(
            out,
            file,
            "/entry".into(),
            DiagnosticCode::QuestNoCompletionPath,
            "quest has no reachable terminal state".into(),
            Some("add a transition path to a terminal state"),
        );
    }
}
