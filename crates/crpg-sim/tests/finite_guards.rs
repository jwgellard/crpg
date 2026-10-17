//! Finite-guard coverage for state_hash, history_hash, and the area-transfer
//! preflight (audit leads #8, #9, #10). Tests only: no source changes.

#[allow(dead_code)]
mod support;

use crpg_core::Ulid;
use crpg_sim::{
    history_hash, state_hash, transfer_entity, AreaError, EntityMeta, HistoryWorld, Transform,
    World,
};

fn placed(position: [f32; 3], velocity: [f32; 3]) -> Transform {
    Transform { position, velocity }
}

fn area_a() -> Ulid {
    Ulid::from_u128(131)
}

fn area_b() -> Ulid {
    Ulid::from_u128(132)
}

fn bytes(world: &World) -> Vec<u8> {
    serde_json::to_vec(world).expect("the world serializes")
}

// ---------------------------------------------------------------------------
// Lead #8: state_hash guards every Transform component.
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "only finite floats")]
fn state_hash_rejects_nan_position_x() {
    let mut world = World::new(31);
    let id = world.spawn(EntityMeta {});
    world
        .transforms_mut()
        .insert(id, placed([f32::NAN, 0.0, 0.0], [0.0; 3]));
    let _ = state_hash(&world);
}

#[test]
#[should_panic(expected = "only finite floats")]
fn state_hash_rejects_infinite_position_y() {
    let mut world = World::new(32);
    let id = world.spawn(EntityMeta {});
    world
        .transforms_mut()
        .insert(id, placed([0.0, f32::INFINITY, 0.0], [0.0; 3]));
    let _ = state_hash(&world);
}

#[test]
#[should_panic(expected = "only finite floats")]
fn state_hash_rejects_negative_infinity_position_z() {
    let mut world = World::new(33);
    let id = world.spawn(EntityMeta {});
    world
        .transforms_mut()
        .insert(id, placed([0.0, 0.0, f32::NEG_INFINITY], [0.0; 3]));
    let _ = state_hash(&world);
}

#[test]
#[should_panic(expected = "only finite floats")]
fn state_hash_rejects_nan_velocity_x() {
    let mut world = World::new(34);
    let id = world.spawn(EntityMeta {});
    world
        .transforms_mut()
        .insert(id, placed([0.0; 3], [f32::NAN, 0.0, 0.0]));
    let _ = state_hash(&world);
}

#[test]
#[should_panic(expected = "only finite floats")]
fn state_hash_rejects_infinite_velocity_y() {
    let mut world = World::new(35);
    let id = world.spawn(EntityMeta {});
    world
        .transforms_mut()
        .insert(id, placed([0.0; 3], [0.0, f32::INFINITY, 0.0]));
    let _ = state_hash(&world);
}

#[test]
#[should_panic(expected = "only finite floats")]
fn state_hash_rejects_negative_infinity_velocity_z() {
    let mut world = World::new(36);
    let id = world.spawn(EntityMeta {});
    world
        .transforms_mut()
        .insert(id, placed([0.0; 3], [0.0, 0.0, f32::NEG_INFINITY]));
    let _ = state_hash(&world);
}

// ---------------------------------------------------------------------------
// Lead #9: history_hash guards every Transform component, through the public
// (de)serialization surface.
// ---------------------------------------------------------------------------

fn history_with_transform(transform: serde_json::Value) -> HistoryWorld {
    let mut history = HistoryWorld::new(0);
    let _ = history.spawn(EntityMeta {}).expect("spawn");
    let mut value = serde_json::to_value(&history).expect("serialize");
    value["world"]["transforms"] = serde_json::json!([[{"index": 0, "generation": 1}, transform]]);
    serde_json::from_value(value).expect("loads")
}

#[test]
#[should_panic(expected = "only finite floats")]
fn history_hash_rejects_nan_position_x() {
    let history = history_with_transform(serde_json::json!({
        "position": [1e40, 0.0, 0.0],
        "velocity": [0.0, 0.0, 0.0]
    }));
    let _ = history_hash(&history);
}

#[test]
#[should_panic(expected = "only finite floats")]
fn history_hash_rejects_infinite_position_y() {
    let history = history_with_transform(serde_json::json!({
        "position": [0.0, 1e40, 0.0],
        "velocity": [0.0, 0.0, 0.0]
    }));
    let _ = history_hash(&history);
}

#[test]
#[should_panic(expected = "only finite floats")]
fn history_hash_rejects_negative_infinity_position_z() {
    let history = history_with_transform(serde_json::json!({
        "position": [0.0, 0.0, -1e40],
        "velocity": [0.0, 0.0, 0.0]
    }));
    let _ = history_hash(&history);
}

#[test]
#[should_panic(expected = "only finite floats")]
fn history_hash_rejects_nan_velocity_x() {
    let history = history_with_transform(serde_json::json!({
        "position": [0.0, 0.0, 0.0],
        "velocity": [1e40, 0.0, 0.0]
    }));
    let _ = history_hash(&history);
}

#[test]
#[should_panic(expected = "only finite floats")]
fn history_hash_rejects_infinite_velocity_y() {
    let history = history_with_transform(serde_json::json!({
        "position": [0.0, 0.0, 0.0],
        "velocity": [0.0, 1e40, 0.0]
    }));
    let _ = history_hash(&history);
}

#[test]
#[should_panic(expected = "only finite floats")]
fn history_hash_rejects_negative_infinity_velocity_z() {
    let history = history_with_transform(serde_json::json!({
        "position": [0.0, 0.0, 0.0],
        "velocity": [0.0, 0.0, -1e40]
    }));
    let _ = history_hash(&history);
}

// ---------------------------------------------------------------------------
// Lead #10: area-transfer preflight guards every Transform component.
// ---------------------------------------------------------------------------

#[test]
fn transfer_rejects_nonfinite_position_y() {
    let mut source = World::new_in_area(4, area_a());
    let mut destination = World::new_in_area(4, area_b());
    let loose = source.spawn(EntityMeta {});
    source
        .transforms_mut()
        .insert(loose, placed([0.0, f32::NAN, 0.0], [0.0; 3]));
    let before = (bytes(&source), bytes(&destination));
    assert_eq!(
        transfer_entity(&mut source, &mut destination, loose),
        Err(AreaError::InvalidTransform { entity: loose })
    );
    assert_eq!((bytes(&source), bytes(&destination)), before);
}

#[test]
fn transfer_rejects_nonfinite_position_z() {
    let mut source = World::new_in_area(4, area_a());
    let mut destination = World::new_in_area(4, area_b());
    let loose = source.spawn(EntityMeta {});
    source
        .transforms_mut()
        .insert(loose, placed([0.0, 0.0, f32::INFINITY], [0.0; 3]));
    let before = (bytes(&source), bytes(&destination));
    assert_eq!(
        transfer_entity(&mut source, &mut destination, loose),
        Err(AreaError::InvalidTransform { entity: loose })
    );
    assert_eq!((bytes(&source), bytes(&destination)), before);
}

#[test]
fn transfer_rejects_nonfinite_velocity_y() {
    let mut source = World::new_in_area(4, area_a());
    let mut destination = World::new_in_area(4, area_b());
    let loose = source.spawn(EntityMeta {});
    source
        .transforms_mut()
        .insert(loose, placed([0.0; 3], [0.0, f32::NEG_INFINITY, 0.0]));
    let before = (bytes(&source), bytes(&destination));
    assert_eq!(
        transfer_entity(&mut source, &mut destination, loose),
        Err(AreaError::InvalidTransform { entity: loose })
    );
    assert_eq!((bytes(&source), bytes(&destination)), before);
}

#[test]
fn transfer_rejects_nonfinite_velocity_z() {
    let mut source = World::new_in_area(4, area_a());
    let mut destination = World::new_in_area(4, area_b());
    let loose = source.spawn(EntityMeta {});
    source
        .transforms_mut()
        .insert(loose, placed([0.0; 3], [0.0, 0.0, f32::NAN]));
    let before = (bytes(&source), bytes(&destination));
    assert_eq!(
        transfer_entity(&mut source, &mut destination, loose),
        Err(AreaError::InvalidTransform { entity: loose })
    );
    assert_eq!((bytes(&source), bytes(&destination)), before);
}
