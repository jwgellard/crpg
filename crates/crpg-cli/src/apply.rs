//! Provisional reference-intents for `crpgc replay` (T009b).
//!
//! Testkit owns no payload vocabulary (invariant 8): the caller owns what a
//! payload means. Until a real game-intent task (T014/T016) defines one, the
//! binary's apply interprets the same miniature reference vocabulary the
//! T009a fixture was generated against (`spawn_at`, `timeline`, `draw`,
//! `despawn`) over public `World` APIs only. It exists so `crpgc replay`
//! can verify that fixture against its native-scoped golden; it is
//! superseded, not blessed, when real intents land, and nothing outside
//! this crate may import it.

use crpg_core::EntityId;
use crpg_sim::{EntityMeta, InitiativeKey, Transform, World};
use crpg_testkit::ApplyInput;

/// Plays reference payloads through public `World` APIs.
///
/// Semantics are intentionally identical to the caller-side example in
/// `crpg-testkit/tests/support/mod.rs`, byte for byte, so a replay generated
/// with one plays unchanged under the other. Spawned ids are tracked in
/// spawn order so later inputs can name them by `slot`.
pub(crate) fn reference_intents() -> ApplyInput {
    let mut spawned: Vec<EntityId> = Vec::new();
    Box::new(move |world: &mut World, payload: &serde_json::Value| {
        if let Some(pos) = payload.get("spawn_at") {
            let arr = pos
                .as_array()
                .ok_or_else(|| "spawn_at must be [x, y, z]".to_string())?;
            if arr.len() != 3 {
                return Err("spawn_at must be [x, y, z]".to_string());
            }
            let mut xyz = [0.0f32; 3];
            for (i, v) in arr.iter().enumerate() {
                xyz[i] = v
                    .as_f64()
                    .ok_or_else(|| "spawn_at coordinates must be numbers".to_string())?
                    as f32;
            }
            let id = world.spawn(EntityMeta {});
            world.transforms_mut().insert(
                id,
                Transform {
                    position: xyz,
                    velocity: [0.0, 0.0, 0.0],
                },
            );
            spawned.push(id);
            return Ok(());
        }
        if let Some(entry) = payload.get("timeline") {
            let slot = entry
                .get("slot")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| "timeline.slot must be a non-negative integer".to_string())?;
            let key = entry
                .get("key")
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| "timeline.key must be an integer".to_string())?;
            let id = spawned
                .get(slot as usize)
                .copied()
                .ok_or_else(|| format!("timeline.slot {slot} names nothing spawned so far"))?;
            if !world.contains(id) {
                return Err(format!("timeline.slot {slot} is no longer live"));
            }
            world.timeline_mut().insert(InitiativeKey(key as i32), id);
            return Ok(());
        }
        if let Some(name) = payload.get("draw") {
            let name = name
                .as_str()
                .ok_or_else(|| "draw must be a stream name string".to_string())?;
            let _ = world.rng_mut().stream(name).next_u32();
            return Ok(());
        }
        if let Some(entry) = payload.get("despawn") {
            let slot = entry
                .get("slot")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| "despawn.slot must be a non-negative integer".to_string())?;
            let id = spawned
                .get(slot as usize)
                .copied()
                .ok_or_else(|| format!("despawn.slot {slot} names nothing spawned so far"))?;
            if !world.despawn(id) {
                return Err(format!("despawn.slot {slot} is already dead"));
            }
            return Ok(());
        }
        Err(format!("unknown intent payload: {payload}"))
    })
}
