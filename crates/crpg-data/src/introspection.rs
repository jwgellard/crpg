//! Data-owned object introspection for the future `crpgc explain` command.
//!
//! [`explain_object`] reports one object plus its inbound and outbound typed
//! references as canonical JSON bytes. Identity locations and reference
//! occurrences come from the shared [`inventory`](crate::inventory), never from
//! the caller-mutable index or from diagnostics.

use crate::{DataError, LoadedCampaign, ObjectKind, SourcePath};
use crpg_core::Ulid;
use serde::Serialize;
use std::collections::BTreeMap;

/// Stable report spelling for each object kind.
fn kind_spelling(kind: ObjectKind) -> &'static str {
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
        ObjectKind::DialogueNode => "dialogue_node",
        ObjectKind::QuestState => "quest_state",
    }
}

/// Location of an existing target object inside the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TargetLocation {
    /// Report spelling of the target kind.
    kind: &'static str,
    /// Logical source file of the target.
    file: SourcePath,
    /// RFC 6901 pointer to the target object.
    pointer: String,
}

/// One typed reference occurrence inside the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct RefReport {
    /// Logical source file containing the reference value.
    file: SourcePath,
    /// RFC 6901 pointer to the reference value.
    pointer: String,
    /// Nearest enclosing authored object, or null outside any identified object.
    source: Option<Ulid>,
    /// Authored target identity.
    target: Ulid,
    /// Location of the target when it exists, otherwise null.
    target_location: Option<TargetLocation>,
}

/// Complete canonical report for one queried object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Report {
    /// Queried uppercase identity.
    id: Ulid,
    /// Report spelling of the queried kind.
    kind: &'static str,
    /// Logical source file of the queried object.
    file: SourcePath,
    /// RFC 6901 pointer to the queried object.
    pointer: String,
    /// Current serialized object JSON at that pointer.
    object: serde_json::Value,
    /// All occurrences whose target equals the queried id, in stable order.
    inbound: Vec<RefReport>,
    /// All occurrences physically within the selected subtree, in stable order.
    outbound: Vec<RefReport>,
}

/// Unescapes one RFC 6901 reference token (`~1` then `~0`).
fn unescape_token(token: &str) -> String {
    token.replace("~1", "/").replace("~0", "~")
}

/// Navigates a canonical document value to the subtree at `pointer.
///
/// `pointer` is empty for a document root or a `/`-joined sequence of
/// unescaped object keys and decimal array indices.
fn navigate(value: &serde_json::Value, pointer: &str) -> Option<serde_json::Value> {
    if pointer.is_empty() {
        return Some(value.clone());
    }
    let mut current = value;
    for raw in pointer.split('/').skip(1) {
        let token = unescape_token(raw);
        match current {
            serde_json::Value::Object(map) => current = map.get(&token)?,
            serde_json::Value::Array(items) => {
                let index: usize = token.parse().ok()?;
                current = items.get(index)?;
            }
            _ => return None,
        }
    }
    Some(current.clone())
}

/// Whether `candidate` lies within the subtree rooted at `root`.
///
/// Matches RFC 6901 component boundaries: `/nodes/1` contains
/// `/nodes/1/body/next` but not `/nodes/10/body/next`. The empty root
/// contains every pointer in its file.
fn within_subtree(root: &str, candidate: &str) -> bool {
    if root.is_empty() {
        return true;
    }
    candidate == root || candidate.starts_with(&format!("{root}/"))
}

/// Explains one authored object plus its inbound and outbound references.
///
/// Pure in-memory work with no filesystem use. Establishes the same
/// structural acceptance boundary and error precedence as
/// [`serialize_campaign`](crate::serialize_campaign) before lookup, rebuilds
/// authoritative locations from documents without trusting the caller-mutable
/// index, and reports semantic edges as an inventory even when validation
/// would flag them. Returns `None` when a structurally acceptable campaign
/// contains no object with that identity.
///
/// The returned bytes are canonical JSON with exactly one final LF.
pub fn explain_object(campaign: &LoadedCampaign, id: Ulid) -> Result<Option<Vec<u8>>, DataError> {
    crate::loader::structural_check(&campaign.documents)?;
    let documents = &campaign.documents;
    let mut locations: BTreeMap<Ulid, (ObjectKind, SourcePath, String)> = BTreeMap::new();
    for occurrence in crate::inventory::object_occurrences(documents) {
        locations.entry(occurrence.id).or_insert((
            occurrence.kind,
            occurrence.path,
            occurrence.pointer,
        ));
    }
    let Some((kind, file, pointer)) = locations.get(&id) else {
        return Ok(None);
    };
    let kind = *kind;
    let file = file.clone();
    let pointer = pointer.clone();
    let document = documents
        .get(&file)
        .expect("location rebuilt from documents");
    let canonical = crate::write_document(document)?;
    let document_value: serde_json::Value =
        serde_json::from_slice(&canonical).map_err(crate::error::malformed)?;
    let object = navigate(&document_value, &pointer).expect("rebuilt pointer resolves");
    let occurrences = crate::inventory::reference_occurrences(documents);
    let mut inbound = Vec::new();
    let mut outbound = Vec::new();
    for occurrence in &occurrences {
        let location =
            locations
                .get(&occurrence.target)
                .map(
                    |(target_kind, target_file, target_pointer)| TargetLocation {
                        kind: kind_spelling(*target_kind),
                        file: target_file.clone(),
                        pointer: target_pointer.clone(),
                    },
                );
        let report = RefReport {
            file: occurrence.file.clone(),
            pointer: occurrence.pointer.clone(),
            source: occurrence.source,
            target: occurrence.target,
            target_location: location,
        };
        if occurrence.target == id {
            inbound.push(report.clone());
        }
        if occurrence.file == file && within_subtree(&pointer, &occurrence.pointer) {
            outbound.push(report);
        }
    }
    let order = |a: &RefReport, b: &RefReport| {
        (&a.file, &a.pointer, &a.target).cmp(&(&b.file, &b.pointer, &b.target))
    };
    inbound.sort_by(order);
    outbound.sort_by(order);
    let report = Report {
        id,
        kind: kind_spelling(kind),
        file,
        pointer,
        object,
        inbound,
        outbound,
    };
    Ok(Some(crate::canonical_json(&report)?))
}

#[cfg(test)]
mod tests {
    use super::{kind_spelling, navigate, unescape_token, within_subtree};
    use crate::ObjectKind;

    #[test]
    fn kind_spellings_match_the_report_contract() {
        for (kind, text) in [
            (ObjectKind::Campaign, "campaign"),
            (ObjectKind::World, "world"),
            (ObjectKind::Area, "area"),
            (ObjectKind::Creature, "creature"),
            (ObjectKind::Item, "item"),
            (ObjectKind::Dialogue, "dialogue"),
            (ObjectKind::Quest, "quest"),
            (ObjectKind::Faction, "faction"),
            (ObjectKind::Placement, "placement"),
            (ObjectKind::Graph, "graph"),
            (ObjectKind::Node, "node"),
            (ObjectKind::DialogueNode, "dialogue_node"),
            (ObjectKind::QuestState, "quest_state"),
        ] {
            assert_eq!(kind_spelling(kind), text);
        }
    }

    #[test]
    fn rfc6901_unescape_orders_tilde_before_slash() {
        assert_eq!(unescape_token("a~1b~0c"), "a/b~c");
        assert_eq!(unescape_token("~01"), "~1");
    }

    #[test]
    fn subtree_matching_respects_component_boundaries() {
        assert!(within_subtree("", "/nodes/10/body/next"));
        assert!(within_subtree("/nodes/1", "/nodes/1"));
        assert!(within_subtree("/nodes/1", "/nodes/1/body/next"));
        assert!(!within_subtree("/nodes/1", "/nodes/10/body/next"));
        assert!(!within_subtree("/graphs/0", "/graphs/1/start"));
    }

    #[test]
    fn pointer_navigation_resolves_escaped_segments() {
        let value = serde_json::json!({"a/b~c": [{"d": 1}]});
        assert_eq!(navigate(&value, "/a~1b~0c/0/d"), Some(serde_json::json!(1)));
        assert_eq!(navigate(&value, ""), Some(value.clone()));
        assert_eq!(navigate(&value, "/missing"), None);
    }
}
