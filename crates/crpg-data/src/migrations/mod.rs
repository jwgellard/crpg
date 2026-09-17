//! Per-document-type migration chains loaded in memory.
//!
//! Old source documents become current values on load without mutating caller
//! bytes or touching the filesystem. Each of the 15 document families starts
//! at version 1; [`schema_versions`] exposes the immutable production registry
//! sorted by schema type, and [`migrate_document`] drives a complete
//! registered historical chain through pure value-to-value steps. Steps never
//! run gameplay, expressions or semantic validation, and locks are covered by
//! the registry without shipping a lock edge.

mod item;

#[cfg(test)]
use crate as gate_api;
#[cfg(test)]
#[path = "../../tests/support/migration_gate.rs"]
mod gate;

use crate::DataError;
use serde_json::Value;
use std::collections::BTreeSet;

/// One document family and its current schema version.
///
/// The type name carries no slash or version suffix; the canonical wire tag
/// is `<schema_type>/<current>` in decimal without leading zeros.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaVersion {
    /// Registered document type name such as `crpg.item`.
    pub schema_type: &'static str,
    /// Current version for this family.
    pub current: u32,
}

/// Pure value transformation changing the tag to the destination.
type Step = fn(&mut Value) -> Result<(), DataError>;

/// Single registered upgrade edge within one family.
struct Edge {
    /// Source version.
    from: u32,
    /// Destination version, always `from + 1`.
    to: u32,
    /// Pure value transformation changing the tag to the destination.
    step: Step,
}

/// One document family with its complete upgrade edges.
struct Family {
    /// Registered type name.
    schema_type: &'static str,
    /// Current version.
    current: u32,
    /// Upgrade edges, each `(n) -> (n + 1)`.
    edges: &'static [Edge],
}

macro_rules! define_registry {
    ( $( $type:literal => $current:literal, edges: $edges:expr );* $(;)? ) => {
        /// Single production registry backing dispatch and the public view.
        static REGISTRY: &[Family] = &[
            $( Family { schema_type: $type, current: $current, edges: $edges } ),*
        ];
        /// Public version view derived from the same registry definition.
        static VERSIONS: &[SchemaVersion] = &[
            $( SchemaVersion { schema_type: $type, current: $current } ),*
        ];
    };
}

// Fifteen families sorted by schema type, including both locks. Only the
// item family has advanced past version 1.
define_registry! {
    "crpg.area" => 1, edges: &[];
    "crpg.assets-lock" => 1, edges: &[];
    "crpg.campaign" => 1, edges: &[];
    "crpg.campaign-lock" => 1, edges: &[];
    "crpg.creature" => 1, edges: &[];
    "crpg.dialogue" => 1, edges: &[];
    "crpg.faction" => 1, edges: &[];
    "crpg.graph" => 1, edges: &[];
    "crpg.item" => 2, edges: &[Edge { from: 1, to: 2, step: item::v1_to_v2 }];
    "crpg.locale" => 1, edges: &[];
    "crpg.placements" => 1, edges: &[];
    "crpg.quest" => 1, edges: &[];
    "crpg.triggers" => 1, edges: &[];
    "crpg.variables" => 1, edges: &[];
    "crpg.world" => 1, edges: &[]
}

/// Exposes the immutable production registry's document families.
///
/// Returns fifteen entries sorted by schema type, including both locks, with
/// names such as `crpg.item` carrying no slash or version suffix.
pub fn schema_versions() -> &'static [SchemaVersion] {
    VERSIONS
}

/// Rejects non-integer numbers recursively before schema dispatch.
///
/// Accepts JSON null, booleans, strings, arrays and objects whose numbers are
/// all `i64` or `u64`; any floating representation fails as malformed using
/// the existing integer-only policy.
fn reject_non_integers(value: &Value) -> Result<(), DataError> {
    match value {
        Value::Null | Value::Bool(_) | Value::String(_) => Ok(()),
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                Ok(())
            } else {
                Err(crate::error::malformed("non-integer number"))
            }
        }
        Value::Array(items) => {
            for item in items {
                reject_non_integers(item)?;
            }
            Ok(())
        }
        Value::Object(map) => {
            for item in map.values() {
                reject_non_integers(item)?;
            }
            Ok(())
        }
    }
}

/// Parses one canonical tag against the given registry.
///
/// Returns the matched family index and numeric version. Any unknown type,
/// malformed version text, zero, overflow or otherwise non-canonical tag
/// fails as unsupported with the original tag preserved in `found`.
fn parse_tag(tag: &str, registry: &[Family]) -> Result<(usize, u32), DataError> {
    let unsupported = || DataError::UnsupportedSchema {
        path: None,
        found: tag.into(),
    };
    if tag.matches('/').count() != 1 {
        return Err(unsupported());
    }
    let (type_part, version_text) = tag.split_once('/').ok_or_else(unsupported)?;
    let index = registry
        .iter()
        .position(|family| family.schema_type == type_part)
        .ok_or_else(unsupported)?;
    if version_text.is_empty() || !version_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(unsupported());
    }
    if version_text.len() > 1 && version_text.starts_with('0') {
        return Err(unsupported());
    }
    // Ten digits already exceed u32::MAX unless the value fits; eleven or
    // more digits always overflow.
    if version_text.len() > 10 {
        return Err(unsupported());
    }
    let mut version: u32 = 0;
    for digit in version_text.bytes() {
        let place = u32::from(digit - b'0');
        version = version
            .checked_mul(10)
            .and_then(|base| base.checked_add(place))
            .ok_or_else(unsupported)?;
    }
    if version == 0 {
        return Err(unsupported());
    }
    Ok((index, version))
}

/// Validates one family's edge topology without applying anything.
///
/// Every edge must satisfy `to == from + 1` with no arithmetic wrap, stay
/// within `1..=current`, and use each source version at most once. Any
/// duplicate, branch, jump, wrap or beyond-current edge fails as unsupported
/// rather than panicking or looping.
fn validate_family(family: &Family, original_tag: &str) -> Result<(), DataError> {
    let unsupported = || DataError::UnsupportedSchema {
        path: None,
        found: original_tag.into(),
    };
    if family.current < 1 {
        return Err(unsupported());
    }
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    for edge in family.edges {
        if edge.from < 1 {
            return Err(unsupported());
        }
        let expected = edge.from.checked_add(1).ok_or_else(unsupported)?;
        if edge.to != expected {
            return Err(unsupported());
        }
        if edge.to > family.current {
            return Err(unsupported());
        }
        if !seen.insert(edge.from) {
            return Err(unsupported());
        }
    }
    Ok(())
}

/// Applies the complete registered chain for one value without final decode.
///
/// Checks integer-only content, the object/string envelope, tag selection and
/// the entire `(type, n) -> (type, n + 1)` chain before mutating anything,
/// then applies each step in order while checking the object/tag
/// postcondition after every step. Current-version input succeeds without
/// mutation. Failures leave the supplied value unchanged only when the caller
/// works on a clone; this helper mutates in place and reports step
/// shape, precondition and postcondition failures as malformed.
fn dispatch(value: &mut Value, registry: &[Family]) -> Result<(), DataError> {
    reject_non_integers(value)?;
    let original_tag = {
        let object = value
            .as_object()
            .ok_or_else(|| crate::error::malformed("expected object with string schema"))?;
        let tag = object
            .get("schema")
            .and_then(|entry| entry.as_str())
            .ok_or_else(|| crate::error::malformed("expected object with string schema"))?;
        tag.to_string()
    };
    let (index, version) = parse_tag(&original_tag, registry)?;
    let family = &registry[index];
    validate_family(family, &original_tag)?;
    if version > family.current {
        return Err(DataError::UnsupportedSchema {
            path: None,
            found: original_tag,
        });
    }
    if version == family.current {
        return Ok(());
    }
    // Collect the full chain before applying anything.
    let mut steps: Vec<Step> = Vec::new();
    let mut next = version;
    while next < family.current {
        let edge = family
            .edges
            .iter()
            .find(|edge| edge.from == next)
            .ok_or_else(|| DataError::UnsupportedSchema {
                path: None,
                found: original_tag.clone(),
            })?;
        if edge.to != next + 1 {
            return Err(DataError::UnsupportedSchema {
                path: None,
                found: original_tag.clone(),
            });
        }
        steps.push(edge.step);
        next = edge.to;
    }
    for (offset, step) in steps.into_iter().enumerate() {
        let destination = version + offset as u32 + 1;
        step(value).map_err(|_| crate::error::malformed("migration step failed"))?;
        let expected = std::format!("{}/{}", family.schema_type, destination);
        let actual = value
            .as_object()
            .and_then(|object| object.get("schema"))
            .and_then(|entry| entry.as_str())
            .ok_or_else(|| crate::error::malformed("migration broke the schema tag"))?;
        if actual != expected {
            return Err(crate::error::malformed("migration broke the schema tag"));
        }
    }
    Ok(())
}

/// Migrates one owned value to its current tag without final typed decode.
///
/// Shared by the byte reader and the public value API so both use the same
/// dispatcher rather than maintaining a second loader.
pub(crate) fn migrate_to_current(value: &mut Value) -> Result<(), DataError> {
    dispatch(value, REGISTRY)
}

/// Migrates one document value to its current schema version.
///
/// Works on a private clone and publishes it only after the full chain and
/// current typed and local validation succeed; failure leaves the caller's
/// value exactly unchanged. Current-version input is validated but never
/// rewritten or normalized, and repeated successful migration is idempotent.
/// Non-integer numbers fail as malformed before schema dispatch, while unknown
/// types, bad, zero, overflow, future or unavailable versions fail as
/// unsupported with the original tag preserved.
pub fn migrate_document(doc: &mut Value) -> Result<(), DataError> {
    reject_non_integers(doc)?;
    let mut working = doc.clone();
    dispatch(&mut working, REGISTRY)?;
    crate::document::decode_current(working.clone())?;
    if working != *doc {
        *doc = working;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{dispatch, validate_family, Edge, Family, REGISTRY, VERSIONS};
    use crate::DataError;
    use serde_json::{json, Value};

    fn item_v1() -> Value {
        json!({
            "_note": "T012 migration fixture",
            "id": "00000000000000000000000008",
            "name": "fixture.creature",
            "schema": "crpg.item/1",
            "slug": "item",
            "stats": {},
            "tags": []
        })
    }

    fn item_v2() -> Value {
        json!({
            "_note": "T012 migration fixture",
            "id": "00000000000000000000000008",
            "name": "fixture.creature",
            "schema": "crpg.item/2",
            "slug": "item",
            "stats": {},
            "tags": []
        })
    }

    fn ok_step_to(tag: &'static str) -> fn(&mut Value) -> Result<(), DataError> {
        let _ = tag;
        |_| Ok(())
    }

    #[test]
    fn production_registry_is_sorted_fifteen_with_item_two() {
        assert_eq!(VERSIONS.len(), 15);
        assert_eq!(REGISTRY.len(), 15);
        let names: Vec<&str> = VERSIONS.iter().map(|entry| entry.schema_type).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        for entry in VERSIONS {
            assert!(!entry.schema_type.contains('/'));
            assert!(entry.current >= 1);
        }
        let item = VERSIONS
            .iter()
            .find(|entry| entry.schema_type == "crpg.item")
            .unwrap();
        assert_eq!(item.current, 2);
        for entry in VERSIONS {
            if entry.schema_type != "crpg.item" {
                assert_eq!(entry.current, 1);
            }
        }
        assert!(VERSIONS
            .iter()
            .any(|entry| entry.schema_type == "crpg.campaign-lock"));
        assert!(VERSIONS
            .iter()
            .any(|entry| entry.schema_type == "crpg.assets-lock"));
        // Registry and public view agree by construction.
        for (family, version) in REGISTRY.iter().zip(VERSIONS.iter()) {
            assert_eq!(family.schema_type, version.schema_type);
            assert_eq!(family.current, version.current);
        }
        // Production topology validates for the real tags.
        for family in REGISTRY {
            let tag = std::format!("{}/{}", family.schema_type, family.current);
            validate_family(family, &tag).unwrap();
        }
        let _ = ok_step_to("crpg.item/2");
        let _ = names;
    }

    #[test]
    fn topology_rejects_duplicate_branch_jump_wrap_and_beyond() {
        fn noop(_: &mut Value) -> Result<(), DataError> {
            Ok(())
        }
        // Duplicate source.
        let duplicate = Family {
            schema_type: "test.dup",
            current: 2,
            edges: &[
                Edge {
                    from: 1,
                    to: 2,
                    step: noop,
                },
                Edge {
                    from: 1,
                    to: 2,
                    step: noop,
                },
            ],
        };
        assert!(validate_family(&duplicate, "test.dup/1").is_err());
        // Jump over a version.
        let jump = Family {
            schema_type: "test.jump",
            current: 3,
            edges: &[Edge {
                from: 1,
                to: 3,
                step: noop,
            }],
        };
        assert!(validate_family(&jump, "test.jump/1").is_err());
        // Beyond current.
        let beyond = Family {
            schema_type: "test.beyond",
            current: 2,
            edges: &[Edge {
                from: 2,
                to: 3,
                step: noop,
            }],
        };
        assert!(validate_family(&beyond, "test.beyond/2").is_err());
        // Arithmetic wrap from the maximum version.
        let wrap = Family {
            schema_type: "test.wrap",
            current: u32::MAX,
            edges: &[Edge {
                from: u32::MAX,
                to: 0,
                step: noop,
            }],
        };
        assert!(validate_family(&wrap, "test.wrap/1").is_err());
        // Backward edge modelling a cycle.
        let cycle = Family {
            schema_type: "test.cycle",
            current: 2,
            edges: &[
                Edge {
                    from: 1,
                    to: 2,
                    step: noop,
                },
                Edge {
                    from: 2,
                    to: 1,
                    step: noop,
                },
            ],
        };
        assert!(validate_family(&cycle, "test.cycle/1").is_err());
        // Missing chain dispatches as unsupported without mutating.
        let gap = Family {
            schema_type: "test.gap",
            current: 3,
            edges: &[Edge {
                from: 2,
                to: 3,
                step: noop,
            }],
        };
        let registry = [gap];
        let mut value = json!({"schema": "test.gap/1"});
        let before = value.clone();
        assert!(matches!(
            dispatch(&mut value, &registry),
            Err(DataError::UnsupportedSchema { .. })
        ));
        assert_eq!(value, before);
    }

    #[test]
    fn per_edge_postcondition_and_cross_type_fail_malformed() {
        fn no_tag_change(_: &mut Value) -> Result<(), DataError> {
            Ok(())
        }
        fn wrong_type(doc: &mut Value) -> Result<(), DataError> {
            doc["schema"] = json!("other.type/2");
            Ok(())
        }
        let missing_post = Family {
            schema_type: "test.post",
            current: 2,
            edges: &[Edge {
                from: 1,
                to: 2,
                step: no_tag_change,
            }],
        };
        let mut value = json!({"schema": "test.post/1"});
        assert!(matches!(
            dispatch(&mut value, &[missing_post]),
            Err(DataError::Malformed { .. })
        ));
        let cross = Family {
            schema_type: "test.cross",
            current: 2,
            edges: &[Edge {
                from: 1,
                to: 2,
                step: wrong_type,
            }],
        };
        let mut value = json!({"schema": "test.cross/1"});
        assert!(matches!(
            dispatch(&mut value, &[cross]),
            Err(DataError::Malformed { .. })
        ));
    }

    fn widget_step_one(doc: &mut Value) -> Result<(), DataError> {
        doc["schema"] = json!("test.widget/2");
        let log = doc.get_mut("log").and_then(|entry| entry.as_array_mut());
        match log {
            Some(items) => items.push(json!(1)),
            None => doc["log"] = json!([1]),
        }
        Ok(())
    }

    fn widget_step_two(doc: &mut Value) -> Result<(), DataError> {
        doc["schema"] = json!("test.widget/3");
        let log = doc.get_mut("log").and_then(|entry| entry.as_array_mut());
        match log {
            Some(items) => items.push(json!(2)),
            None => doc["log"] = json!([1, 2]),
        }
        Ok(())
    }

    fn widget_step_two_fails(doc: &mut Value) -> Result<(), DataError> {
        doc["schema"] = json!("test.widget/3");
        Err(crate::error::malformed("second step fails"))
    }

    fn widget_step_one_fails(_: &mut Value) -> Result<(), DataError> {
        Err(crate::error::malformed("first step fails"))
    }

    fn widget_registry_two() -> [Family; 1] {
        [Family {
            schema_type: "test.widget",
            current: 3,
            edges: &[
                Edge {
                    from: 1,
                    to: 2,
                    step: widget_step_one,
                },
                Edge {
                    from: 2,
                    to: 3,
                    step: widget_step_two,
                },
            ],
        }]
    }

    fn clone_then_dispatch(doc: &mut Value, registry: &[Family]) -> Result<(), DataError> {
        let mut working = doc.clone();
        dispatch(&mut working, registry)?;
        if working != *doc {
            *doc = working;
        }
        Ok(())
    }

    #[test]
    fn two_edge_registry_composes_in_order_with_rollback() {
        // Ordered composition through both steps.
        let registry = widget_registry_two();
        let mut value = json!({"schema": "test.widget/1", "log": []});
        clone_then_dispatch(&mut value, &registry).unwrap();
        assert_eq!(value["schema"], json!("test.widget/3"));
        assert_eq!(value["log"], json!([1, 2]));
        // Late failure leaves the caller exactly unchanged.
        let failing = [Family {
            schema_type: "test.widget",
            current: 3,
            edges: &[
                Edge {
                    from: 1,
                    to: 2,
                    step: widget_step_one,
                },
                Edge {
                    from: 2,
                    to: 3,
                    step: widget_step_two_fails,
                },
            ],
        }];
        let mut value = json!({"schema": "test.widget/1", "log": []});
        let before = value.clone();
        assert!(clone_then_dispatch(&mut value, &failing).is_err());
        assert_eq!(value, before);
        // Early failure also rolls back.
        let early = [Family {
            schema_type: "test.widget",
            current: 3,
            edges: &[
                Edge {
                    from: 1,
                    to: 2,
                    step: widget_step_one_fails,
                },
                Edge {
                    from: 2,
                    to: 3,
                    step: widget_step_two,
                },
            ],
        }];
        let mut value = json!({"schema": "test.widget/1", "log": []});
        let before = value.clone();
        assert!(clone_then_dispatch(&mut value, &early).is_err());
        assert_eq!(value, before);
    }

    #[test]
    fn item_edge_is_exact_tag_only_oracle() {
        let mut value = item_v1();
        let before = value.clone();
        super::item::v1_to_v2(&mut value).unwrap();
        assert_eq!(value, item_v2());
        // Every non-tag field is preserved byte for byte.
        for key in ["id", "slug", "name", "_note", "stats", "tags"] {
            assert_eq!(value[key], before[key], "{key}");
        }
    }

    #[test]
    fn production_edges_match_manifest_and_checked_in_single_step_oracles() {
        use super::gate;
        let files = gate::snapshot().unwrap();
        gate::check_tags(
            VERSIONS,
            &crate::generated_schemas().unwrap(),
            &gate::representatives(&files).unwrap(),
        )
        .unwrap();
        let edges: Vec<_> = REGISTRY
            .iter()
            .flat_map(|family| {
                family
                    .edges
                    .iter()
                    .map(|edge| (family.schema_type.to_string(), edge.from, edge.to))
            })
            .collect();
        gate::check_edges(VERSIONS, &edges).unwrap();
        let rows = gate::check_manifest(VERSIONS, &files).unwrap();
        let mut row_edges: Vec<_> = rows.iter().map(gate::Entry::key).collect();
        let mut sorted_edges = edges.clone();
        row_edges.sort();
        sorted_edges.sort();
        assert_eq!(sorted_edges, row_edges);
        let oracles = gate::check_fixtures(VERSIONS, &files).unwrap();
        gate::check_steps(&oracles, |(name, from, to), value| {
            let family = REGISTRY
                .iter()
                .find(|family| family.schema_type == name)
                .ok_or("missing family")?;
            let edge = family
                .edges
                .iter()
                .find(|edge| edge.from == *from && edge.to == *to)
                .ok_or("missing edge")?;
            (edge.step)(value).map_err(|e| e.to_string())
        })
        .unwrap();
        // Mutate the actual production metadata, not a manifest stand-in.
        let mut duplicate = edges.clone();
        duplicate.push(edges[0].clone());
        assert!(gate::check_edges(VERSIONS, &duplicate).is_err());
        assert!(gate::check_edges(VERSIONS, &[]).is_err());
        let mut bumped = VERSIONS.to_vec();
        bumped
            .iter_mut()
            .find(|v| v.schema_type == "crpg.item")
            .unwrap()
            .current = 3;
        assert!(gate::check_edges(&bumped, &edges).is_err());
        for edge in [
            ("crpg.world".into(), 1, 2),
            ("unknown".into(), 1, 2),
            ("crpg.item".into(), 1, 3),
            ("crpg.item".into(), 2, 1),
        ] {
            let mut invalid = edges.clone();
            invalid.push(edge);
            assert!(gate::check_edges(VERSIONS, &invalid).is_err());
        }
        // A successful callback that fails to transform the value cannot pass.
        assert!(gate::check_steps(&oracles, |_, _| Ok(())).is_err());
    }

    #[test]
    fn superseded_edge_inventory_and_single_steps_are_checked_independently() {
        use super::gate;
        let mut files = gate::snapshot().unwrap();
        let mut versions = VERSIONS.to_vec();
        versions
            .iter_mut()
            .find(|v| v.schema_type == "crpg.item")
            .unwrap()
            .current = 3;
        let old_path = "migration_v1/campaign/items/item.json";
        let second_path = "migration_v2/campaign/items/item.json";
        let step_path = "migration_steps/crpg.item/1-to-2.json";
        let golden_path = "migration_v2/expected.json";
        // Synthetic metadata and values stay in memory. Production remains /2.
        let first: Value = serde_json::from_slice(&files[old_path]).unwrap();
        let golden: std::collections::BTreeMap<String, String> =
            serde_json::from_slice(&files["migration_v1/expected.json"]).unwrap();
        let second: Value = serde_json::from_str(&golden["items/item.json"]).unwrap();
        let mut third = second.clone();
        third["schema"] = json!("crpg.item/3");
        let encode = |v: &Value| crate::canonical_json(v).unwrap();
        files.insert(second_path.into(), encode(&second));
        files.insert(step_path.into(), encode(&second));
        files.insert(
            golden_path.into(),
            encode(&json!({"items/item.json": String::from_utf8(encode(&third)).unwrap()})),
        );
        let mut rows: Value = serde_json::from_slice(&files["migrations.json"]).unwrap();
        let mut upper = rows[0].clone();
        upper["from"] = json!(2);
        upper["to"] = json!(3);
        upper["root"] = json!("migration_v2/campaign");
        upper["golden"] = json!(golden_path);
        rows.as_array_mut().unwrap().push(upper);
        files.insert("migrations.json".into(), encode(&rows));
        let oracles = gate::check_oracle_inventory(&versions, &files).unwrap();
        assert_eq!(oracles.len(), 2);
        assert_eq!(oracles[0].before, first);
        assert_eq!(oracles[0].after, second);
        assert_eq!(oracles[1].before, second);
        assert_eq!(oracles[1].after, third);
        fn to_three(value: &mut Value) -> Result<(), DataError> {
            if value["schema"] != "crpg.item/2" {
                return Err(crate::error::malformed("expected second step input"));
            }
            value["schema"] = json!("crpg.item/3");
            Ok(())
        }
        let registry = [Family {
            schema_type: "crpg.item",
            current: 3,
            edges: &[
                Edge {
                    from: 1,
                    to: 2,
                    step: super::item::v1_to_v2,
                },
                Edge {
                    from: 2,
                    to: 3,
                    step: to_three,
                },
            ],
        }];
        gate::check_steps(&oracles, |(_, from, _), value| {
            let edge = registry[0]
                .edges
                .iter()
                .find(|edge| edge.from == *from)
                .unwrap();
            (edge.step)(value).map_err(|e| e.to_string())
        })
        .unwrap();
        // Using the whole chain for the first oracle would incorrectly reach /3.
        assert!(
            gate::check_steps(&oracles, |_, value| dispatch(value, &registry)
                .map_err(|e| e.to_string()))
            .is_err()
        );
        let mut missing = files.clone();
        missing.remove(step_path);
        assert!(gate::check_oracle_inventory(&versions, &missing).is_err());
        for bytes in [Vec::new(), b"{invalid}".to_vec(), encode(&third)] {
            let mut bad = files.clone();
            bad.insert(step_path.into(), bytes);
            assert!(gate::check_oracle_inventory(&versions, &bad).is_err());
        }
        let mut wrong = files.clone();
        let mut wrong_second = second.clone();
        wrong_second["slug"] = json!("wrong");
        wrong.insert(step_path.into(), encode(&wrong_second));
        let wrong_oracles = gate::check_oracle_inventory(&versions, &wrong).unwrap();
        assert!(gate::check_steps(&wrong_oracles, |(_, from, _), value| {
            let edge = registry[0]
                .edges
                .iter()
                .find(|edge| edge.from == *from)
                .unwrap();
            (edge.step)(value).map_err(|e| e.to_string())
        })
        .is_err());
        let mut extra = files.clone();
        extra.insert(
            "migration_steps/crpg.item/2-to-3.json".into(),
            encode(&third),
        );
        assert!(gate::check_oracle_inventory(&versions, &extra).is_err());
    }
}
