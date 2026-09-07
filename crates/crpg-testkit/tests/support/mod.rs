//! Shared helpers for the T009a replay tests: fixture paths and the
//! miniature payload vocabulary the tests apply through public `World` APIs.
//!
//! These shapes (`spawn_at`, `timeline`, `draw`, `despawn`) belong to the
//! tests, not to `crpg-testkit` — they are what "caller owns what a payload
//! means" looks like in practice. Nothing outside `tests/` may use them.
//!
//! Compiled separately into each test binary, and each binary uses a
//! different subset (`replay_golden.rs` never writes temp files, `replay.rs`
//! never touches the target-scoped golden path), so unused
//! helpers here are cross-binary sharing, not dead code.
#![allow(dead_code)]

use std::path::PathBuf;

use crpg_sim::{EntityMeta, InitiativeKey, Transform, World};
use crpg_testkit::ApplyInput;

/// Path of the checked-in portable replay fixture.
pub fn fixture_replay_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("replay_basic.replay")
}

/// Primary Windows baseline; unavailable outside its ADR-0012 target/profile.
#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    debug_assertions
))]
pub fn target_golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("goldens")
        .join("replay_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden")
}

/// Supported Linux baseline; unavailable outside its ADR-0012 target/profile.
#[cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu",
    debug_assertions
))]
pub fn target_golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("goldens")
        .join("replay_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden")
}

/// Temporary file under the OS temp dir, unique per process and name.
pub fn temp_file(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("crpg-testkit-{}-{name}", std::process::id()));
    path
}

/// Caller-supplied payload application for the tests. Spawned ids are tracked
/// in spawn order so later inputs can name them by `slot` — deterministic
/// for a fixed input order, which is exactly what the ordering tests pin.
pub fn test_apply() -> ApplyInput {
    let mut spawned: Vec<crpg_core::EntityId> = Vec::new();
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
        Err(format!("unknown test payload: {payload}"))
    })
}

/// Builds the same replay the checked-in fixture holds, for tests that need
/// an in-memory replay with the gate's shape (ordering, gaps, trailing
/// ticks) without depending on the fixture file itself.
pub fn basic_replay() -> crpg_testkit::Replay {
    use crpg_testkit::{ReplayInput, REPLAY_FORMAT_VERSION};
    use serde_json::json;
    crpg_testkit::Replay::new(
        REPLAY_FORMAT_VERSION,
        20260906,
        "testkit-basic".to_string(),
        "0.1.0".to_string(),
        "0.1.0".to_string(),
        8,
        vec![
            ReplayInput {
                tick: 0,
                payload: json!({"spawn_at": [1, 0, 0]}),
            },
            ReplayInput {
                tick: 0,
                payload: json!({"spawn_at": [0, 2, 0]}),
            },
            ReplayInput {
                tick: 2,
                payload: json!({"timeline": {"slot": 0, "key": 3}}),
            },
            ReplayInput {
                tick: 3,
                payload: json!({"draw": "scout"}),
            },
            ReplayInput {
                tick: 5,
                payload: json!({"despawn": {"slot": 0}}),
            },
        ],
    )
    .expect("test replay must validate")
}
