//! Item-only dummy migration, tag-only compatibility exercise.
//!
//! The payload is exactly the T010 item shape; the step changes only the
//! schema tag from `crpg.item/1` to `crpg.item/2` and preserves every other
//! field, identifier, reference, array order and author note.

/// Migrates one item document from version 1 to version 2.
///
/// Changes only the exact schema tag; the payload is left untouched so a
/// later typed decode still rejects invalid version-1 content.
pub(super) fn v1_to_v2(doc: &mut serde_json::Value) -> Result<(), crate::DataError> {
    let object = doc
        .as_object_mut()
        .ok_or_else(|| crate::error::malformed("expected object with string schema"))?;
    let current = object
        .get("schema")
        .and_then(|value| value.as_str())
        .ok_or_else(|| crate::error::malformed("expected object with string schema"))?;
    if current != "crpg.item/1" {
        return Err(crate::error::malformed(
            "item migration expects schema crpg.item/1",
        ));
    }
    object.insert(
        "schema".into(),
        serde_json::Value::String("crpg.item/2".into()),
    );
    Ok(())
}
