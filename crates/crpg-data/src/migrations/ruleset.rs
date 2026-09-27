//! Ruleset `1 -> 2` migration: single pool to plural pools.
//!
//! The payload is the T016a ruleset shape; the step preserves every field,
//! identifier, reference, array order and author note, replacing the single
//! `action_pool` template with the one-entry `pools` array carrying the same
//! template value. The schema tag advances from `crpg.ruleset/1` to
//! `crpg.ruleset/2`.

/// Migrates one ruleset document from version 1 to version 2.
///
/// Moves `action_pool` into `pools: [action_pool]` without touching any
/// other field; a later typed decode still rejects invalid version-1
/// content.
pub(super) fn v1_to_v2(doc: &mut serde_json::Value) -> Result<(), crate::DataError> {
    let object = doc
        .as_object_mut()
        .ok_or_else(|| crate::error::malformed("expected object with string schema"))?;
    let current = object
        .get("schema")
        .and_then(|value| value.as_str())
        .ok_or_else(|| crate::error::malformed("expected object with string schema"))?;
    if current != "crpg.ruleset/1" {
        return Err(crate::error::malformed(
            "ruleset migration expects schema crpg.ruleset/1",
        ));
    }
    let pool = object
        .remove("action_pool")
        .ok_or_else(|| crate::error::malformed("ruleset migration missing action_pool"))?;
    object.insert("pools".into(), serde_json::Value::Array(vec![pool]));
    object.insert(
        "schema".into(),
        serde_json::Value::String("crpg.ruleset/2".into()),
    );
    Ok(())
}
