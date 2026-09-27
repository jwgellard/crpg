//! Ability `1 -> 2` migration: primary cost to plural costs with defaults.
//!
//! The payload is the T016a ability shape; the step preserves every field,
//! identifier, reference, array order and author note, keeping `cost` with
//! its refined primary-pool meaning and supplying all-local defaults for the
//! new shapes: empty `extra_costs`, ending turns, no effect, actor-attribute
//! defense, and no natural selection. The per-type chain is context-free, so
//! no pool ULID lookup occurs here. The schema tag advances from
//! `crpg.ability/1` to `crpg.ability/2`.

/// Migrates one ability document from version 1 to version 2.
///
/// Keeps `cost` and adds `extra_costs: []`, `ends_turn: true`,
/// `defense: {type: actor_attribute}`, with `effect` and `natural_die`
/// omitted (optional-missing); a later typed decode still rejects invalid
/// version-1 content.
pub(super) fn v1_to_v2(doc: &mut serde_json::Value) -> Result<(), crate::DataError> {
    let object = doc
        .as_object_mut()
        .ok_or_else(|| crate::error::malformed("expected object with string schema"))?;
    let current = object
        .get("schema")
        .and_then(|value| value.as_str())
        .ok_or_else(|| crate::error::malformed("expected object with string schema"))?;
    if current != "crpg.ability/1" {
        return Err(crate::error::malformed(
            "ability migration expects schema crpg.ability/1",
        ));
    }
    object.insert("extra_costs".into(), serde_json::Value::Array(Vec::new()));
    object.insert("ends_turn".into(), serde_json::Value::Bool(true));
    object.insert(
        "defense".into(),
        serde_json::json!({"type": "actor_attribute"}),
    );
    object.insert(
        "schema".into(),
        serde_json::Value::String("crpg.ability/2".into()),
    );
    Ok(())
}
