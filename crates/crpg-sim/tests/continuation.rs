//! T052: persisted script, dialogue and quest state in `World` (Behaviour
//! B-1..B-12; cases C1..C13, plus the error-text and tick-entry-point checks
//! that B-3 and the Errors table name).
//!
//! Notation (the contract's): `u(n)` is the ULID of 25 `0`s then the digit `n`;
//! `G = u(1)`, `W = u(2)`, `W2 = u(3)`, `D = u(4)`, `N1 = u(5)`, `N2 = u(6)`,
//! `Q = u(7)`, `Q2 = u(8)`, `S1 = u(5)`, `S2 = u(6)`; `k(r)` is a continuation
//! with `remaining = r`; `e(i)` is an entity of index `i`, generation 1.

use std::collections::BTreeSet;

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Ulid};
use crpg_sim::{
    end_turn, history_hash, state_hash, tick, tick_with_movement, transfer_entity,
    transfer_history_entity, AreaError, ContinuationError, EntityMeta, HistoryError, HistoryEvent,
    HistoryWorld, OpenDialogue, QuestProgress, ScriptContinuation, SimEvent, Transform, World,
};
use serde_json::{json, Value};

use support::Fixture;

fn u(n: u8) -> Ulid {
    format!("{}{n}", "0".repeat(25))
        .parse()
        .expect("26 characters of a valid ULID")
}

fn k(remaining: u64) -> ScriptContinuation {
    ScriptContinuation {
        graph: u(1),
        node: u(2),
        remaining,
    }
}

fn dlg(node: u8) -> OpenDialogue {
    OpenDialogue {
        dialogue: u(4),
        node: u(node),
    }
}

fn eg(index: u32, generation: u32) -> EntityId {
    serde_json::from_value(json!({"index": index, "generation": generation})).unwrap()
}

fn e(index: u32) -> EntityId {
    eg(index, 1)
}

/// The legacy queue as drained from a clone, leaving the world untouched.
fn queue(world: &World) -> Vec<(u64, SimEvent)> {
    world
        .clone()
        .events_mut()
        .drain()
        .into_iter()
        .map(|envelope| (envelope.tick.get(), envelope.payload))
        .collect()
}

fn keys(world: &World) -> BTreeSet<String> {
    match serde_json::to_value(world).unwrap() {
        Value::Object(map) => map.keys().cloned().collect(),
        other => panic!("a world is a JSON object, not {other}"),
    }
}

fn base_keys() -> BTreeSet<String> {
    [
        "entities",
        "transforms",
        "timeline",
        "events",
        "rng",
        "tick",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn with_key(name: &str) -> BTreeSet<String> {
    let mut set = base_keys();
    set.insert(name.to_string());
    set
}

fn remaining(world: &World, holder: EntityId) -> u64 {
    world
        .continuations()
        .get(holder)
        .expect("the holder is live")
        .remaining
}

fn text(world: &World) -> String {
    serde_json::to_string(world).unwrap()
}

// ------------------------------------------------------------------- C1 / B-1

#[test]
fn absent_state_is_not_serialized() {
    let mut world = World::new(0);
    assert_eq!(keys(&world), base_keys());

    let holder = world.spawn_continuation(k(1));
    assert_eq!(keys(&world), with_key("continuations"));
    assert!(world.despawn(holder));
    assert_eq!(keys(&world), base_keys());

    let p = world.spawn(EntityMeta {});
    world.open_dialogue(p, dlg(5)).unwrap();
    assert_eq!(keys(&world), with_key("dialogues"));
    world.close_dialogue(p).unwrap();
    assert_eq!(keys(&world), base_keys());

    world.set_quest_state(u(7), u(5));
    assert_eq!(keys(&world), with_key("quests"));
    world.remove_quest(u(7));
    assert_eq!(keys(&world), base_keys());

    // Control: the keys are present together when all three hold state.
    world.spawn_continuation(k(1));
    world.open_dialogue(p, dlg(5)).unwrap();
    world.set_quest_state(u(7), u(5));
    let all = keys(&world);
    for name in ["continuations", "dialogues", "quests"] {
        assert!(all.contains(name), "{name} is written when non-empty");
    }
}

// ------------------------------------------------------------------- C2 / B-2..B-4

#[test]
fn wait_counts_down_and_becomes_due() {
    let mut w0 = World::new(1);
    let a = w0.spawn_continuation(k(3));
    let b = w0.spawn_continuation(k(0));
    assert_eq!((a, b), (e(0), e(1)));
    let spawned = vec![
        (0, SimEvent::Spawned { entity: e(0) }),
        (0, SimEvent::Spawned { entity: e(1) }),
    ];
    assert_eq!(queue(&w0), spawned);
    assert_eq!(w0.due_continuations(), vec![b]);

    // B-2: the holder holds no other component.
    for holder in [a, b] {
        assert!(!w0.transforms().contains(holder));
        assert!(!w0.timeline().contains(holder));
        assert!(!w0.creatures().contains(holder));
        assert!(!w0.movers().contains(holder));
        assert!(!w0.combatants().contains(holder));
        assert!(!w0.dialogues().contains(holder));
    }

    // Control: an end_turn between ticks never touches the stores.
    let mut with_end_turn = w0.clone();
    for expected in [2_u64, 1, 0] {
        tick(&mut w0);
        assert_eq!(remaining(&w0, a), expected);
        assert_eq!(remaining(&w0, b), 0);

        assert_eq!(end_turn(&mut with_end_turn), None);
        tick(&mut with_end_turn);
        assert_eq!(remaining(&with_end_turn, a), expected);
        assert_eq!(remaining(&with_end_turn, b), 0);
    }
    assert_eq!(w0.due_continuations(), vec![a, b]);
    tick(&mut w0);
    assert_eq!(remaining(&w0, a), 0);
    assert_eq!(remaining(&w0, b), 0);
    assert_eq!(w0.due_continuations(), vec![a, b], "a due record is kept");
    assert_eq!(w0.continuations().len(), 2);
    assert_eq!(queue(&w0), spawned, "no tick enqueues anything");

    // Control: a large count moves by exactly one.
    let mut big = World::new(1);
    let holder = big.spawn_continuation(k(u64::MAX));
    tick(&mut big);
    assert_eq!(remaining(&big, holder), u64::MAX - 1);
}

#[test]
fn every_tick_entry_point_counts_down() {
    let mut world = World::new(1);
    let a = world.spawn_continuation(k(3));
    tick(&mut world);
    assert_eq!(remaining(&world, a), 2);
    let facts = tick_with_movement(&mut world);
    assert!(facts.is_empty());
    assert_eq!(remaining(&world, a), 1);

    let mut history = HistoryWorld::new(1);
    let b = history.spawn_continuation(k(2)).unwrap();
    history.tick().unwrap();
    assert_eq!(history.world().continuations().get(b).unwrap().remaining, 1);
    history.tick_with_movement().unwrap();
    assert_eq!(history.world().continuations().get(b).unwrap().remaining, 0);
}

// ------------------------------------------------------------------- C3 / B-4

#[test]
fn due_order_is_entity_order() {
    let mut world = World::new(0);
    for _ in 0..3 {
        world.spawn_continuation(k(0));
    }
    assert!(world.despawn(e(0)));
    let reused = world.spawn_continuation(k(0));
    assert_eq!(reused, eg(0, 2));
    // Store order is [1, 2, 0]; the contract is ascending id.
    let store_order: Vec<u32> = world
        .continuations()
        .iter()
        .map(|(id, _)| id.index())
        .collect();
    assert_eq!(store_order, vec![1, 2, 0]);
    let due = world.due_continuations();
    assert_eq!(
        due.iter().map(|id| id.index()).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(due, vec![eg(0, 2), e(1), e(2)]);

    // Control: the read changes nothing.
    assert_eq!(world.due_continuations(), due);
}

// ------------------------------------------------------------------- C4 / B-5

#[test]
fn set_continuation_replaces() {
    let mut world = World::new(0);
    let a = world.spawn_continuation(k(5));
    let before = queue(&world);
    let replacement = ScriptContinuation {
        graph: u(1),
        node: u(3),
        remaining: 9,
    };
    assert_eq!(world.set_continuation(a, replacement), Ok(k(5)));
    assert_eq!(world.continuations().get(a), Some(&replacement));
    assert_eq!(queue(&world), before, "a rewrite enqueues nothing");

    let plain = world.spawn(EntityMeta {});
    let hash = state_hash(&world);
    assert_eq!(
        world.set_continuation(plain, k(1)),
        Err(ContinuationError::NotContinuation { entity: plain })
    );
    assert_eq!(
        world.set_continuation(e(7), k(1)),
        Err(ContinuationError::NotContinuation { entity: e(7) })
    );
    assert!(!world.continuations().contains(plain));
    assert_eq!(state_hash(&world), hash);

    assert!(world.despawn(a));
    let hash = state_hash(&world);
    assert_eq!(
        world.set_continuation(a, k(1)),
        Err(ContinuationError::NotContinuation { entity: a })
    );
    assert_eq!(state_hash(&world), hash);
}

// ------------------------------------------------------------------- C5 / B-6

#[test]
fn dialogue_lifecycle() {
    let mut world = World::new(0);
    let p = world.spawn(EntityMeta {});
    let p2 = world.spawn(EntityMeta {});
    let spawned = queue(&world);
    assert_eq!(spawned.len(), 2);

    assert_eq!(world.open_dialogue(p, dlg(5)), Ok(()));
    let hash = state_hash(&world);
    assert_eq!(
        world.open_dialogue(p, dlg(6)),
        Err(ContinuationError::DialogueOpen { entity: p })
    );
    assert_eq!(state_hash(&world), hash);
    assert_eq!(world.dialogues().get(p), Some(&dlg(5)));

    assert_eq!(world.set_dialogue_node(p, u(6)), Ok(u(5)));
    assert_eq!(world.close_dialogue(p), Ok(dlg(6)));
    let hash = state_hash(&world);
    assert_eq!(
        world.close_dialogue(p),
        Err(ContinuationError::NoDialogue { entity: p })
    );
    assert_eq!(
        world.set_dialogue_node(p, u(5)),
        Err(ContinuationError::NoDialogue { entity: p })
    );
    assert_eq!(
        world.open_dialogue(e(7), dlg(5)),
        Err(ContinuationError::NotLive { entity: e(7) })
    );
    assert_eq!(state_hash(&world), hash);

    // Control: two participants hold dialogues at once.
    world.open_dialogue(p, dlg(5)).unwrap();
    world.open_dialogue(p2, dlg(6)).unwrap();
    assert_eq!(world.dialogues().len(), 2);
    assert_eq!(world.dialogues().get(p), Some(&dlg(5)));
    assert_eq!(world.dialogues().get(p2), Some(&dlg(6)));
    assert_eq!(queue(&world), spawned, "nothing is ever enqueued");

    // A despawned participant is not live (its id stays dead after reuse).
    assert!(world.despawn(p));
    assert_eq!(
        world.open_dialogue(p, dlg(5)),
        Err(ContinuationError::NotLive { entity: p })
    );
}

// ------------------------------------------------------------------- C6 / B-7

#[test]
fn quests_stay_ascending() {
    let mut world = World::new(0);
    let (q, q2, s1, s2) = (u(7), u(8), u(5), u(6));
    assert_eq!(world.set_quest_state(q2, s1), None);
    assert_eq!(world.set_quest_state(q, s1), None);
    assert_eq!(
        world.quests(),
        &[
            QuestProgress {
                quest: q,
                state: s1
            },
            QuestProgress {
                quest: q2,
                state: s1
            }
        ]
    );
    assert_eq!(world.set_quest_state(q, s2), Some(s1));
    let hash = state_hash(&world);
    assert_eq!(world.set_quest_state(q, s2), Some(s2));
    assert_eq!(state_hash(&world), hash, "an equal state changes nothing");
    assert_eq!(world.quests().len(), 2, "a replace adds no entry");
    assert_eq!(world.quest_state(q), Some(s2));
    assert_eq!(world.quest_state(u(1)), None);
    assert_eq!(world.remove_quest(q2), Some(s1));
    assert_eq!(world.remove_quest(q2), None);
    assert_eq!(world.quests().len(), 1);
    assert!(
        world.clone().events_mut().drain().is_empty(),
        "quests enqueue nothing and need no live entity"
    );
}

// ------------------------------------------------------------------- C7 / B-8

#[test]
fn despawn_strips_script_state() {
    let mut world = World::new(0);
    let holder = world.spawn_continuation(k(4));
    world.open_dialogue(holder, dlg(5)).unwrap();
    world.set_quest_state(u(7), u(5));
    let before = queue(&world);
    assert_eq!(before.len(), 1);

    assert!(world.despawn(holder));
    assert!(world.continuations().is_empty());
    assert!(world.dialogues().is_empty());
    assert_eq!(
        world.quests(),
        &[QuestProgress {
            quest: u(7),
            state: u(5)
        }]
    );
    let after = queue(&world);
    assert_eq!(after.len(), 2);
    assert_eq!(after[..1], before[..]);
    assert_eq!(after[1].1, SimEvent::Despawned { entity: holder });

    // Control: a never-live id despawns nothing and appends nothing.
    assert!(!world.despawn(e(7)));
    assert_eq!(queue(&world), after);
}

// ------------------------------------------------------------------- C8 / B-9

fn nan_transform() -> Transform {
    Transform {
        position: [f32::NAN, 0.0, 0.0],
        velocity: [0.0; 3],
    }
}

fn flat_transform() -> Transform {
    Transform {
        position: [1.0, 2.0, 3.0],
        velocity: [0.0; 3],
    }
}

#[test]
fn transfer_refuses_holders() {
    let mut source = World::new_in_area(0, u(1));
    let mut destination = World::new_in_area(0, u(2));

    let holder = source.spawn_continuation(k(3));
    let participant = source.spawn(EntityMeta {});
    source
        .transforms_mut()
        .insert(participant, flat_transform());
    source.open_dialogue(participant, dlg(5)).unwrap();
    let (source_text, destination_text) = (text(&source), text(&destination));

    assert_eq!(
        transfer_entity(&mut source, &mut destination, holder),
        Err(AreaError::ScriptState { entity: holder })
    );
    assert_eq!(
        transfer_entity(&mut source, &mut destination, participant),
        Err(AreaError::ScriptState {
            entity: participant
        })
    );
    assert_eq!(
        (text(&source), text(&destination)),
        (source_text, destination_text)
    );

    // Precedence: an earlier check (InvalidTransform) is not replaced.
    let corrupt = source.spawn(EntityMeta {});
    source.transforms_mut().insert(corrupt, nan_transform());
    source.open_dialogue(corrupt, dlg(5)).unwrap();
    let (source_text, destination_text) = (text(&source), text(&destination));
    assert_eq!(
        transfer_entity(&mut source, &mut destination, corrupt),
        Err(AreaError::InvalidTransform { entity: corrupt })
    );
    assert_eq!(
        (text(&source), text(&destination)),
        (source_text, destination_text)
    );

    // Control: after the dialogue closes the participant transfers.
    source.close_dialogue(participant).unwrap();
    let moved = transfer_entity(&mut source, &mut destination, participant)
        .expect("a participant with no open dialogue moves");
    assert!(destination.contains(moved));
    assert!(!source.contains(participant));
    assert_eq!(destination.transforms().get(moved), Some(&flat_transform()));
}

#[test]
fn transfer_history_refuses_holders() {
    let mut source = HistoryWorld::new_in_area(0, u(1));
    let mut destination = HistoryWorld::new_in_area(0, u(2));
    let holder = source.spawn_continuation(k(3)).unwrap();
    let participant = source.spawn(EntityMeta {}).unwrap();
    source.open_dialogue(participant, dlg(5)).unwrap();
    let (source_hash, destination_hash) = (history_hash(&source), history_hash(&destination));

    for entity in [holder, participant] {
        assert_eq!(
            transfer_history_entity(&mut source, &mut destination, entity),
            Err(AreaError::ScriptState { entity })
        );
    }
    assert_eq!(history_hash(&source), source_hash);
    assert_eq!(history_hash(&destination), destination_hash);

    // Control: after the dialogue closes the participant transfers.
    source.close_dialogue(participant).unwrap();
    let moved = transfer_history_entity(&mut source, &mut destination, participant)
        .expect("a participant with no open dialogue moves");
    assert!(destination.world().contains(moved));
    assert!(!source.world().contains(participant));
}

/// B-9 (Reading 10): the `ScriptState` check runs after the T139
/// `PlacementInUse` check. A holder whose transfer also collides with a
/// destination record is refused as `PlacementInUse`, in both functions, with
/// both sides unchanged. Controls: the same holder against a destination with
/// no collision is `ScriptState`, and after `close_dialogue` it moves.
#[test]
fn fa_1_transfer_placement_conflict_wins_over_script_state() {
    let fixture = Fixture::standard();
    let placement = fixture.placement_a;

    // Plain worlds: put a record of `placement` in the destination, then a
    // second record of it (with an open dialogue) in the source.
    let mut source = World::new_in_area(0, fixture.area);
    let mut destination = World::new_in_area(0, u(2));
    let first = fixture.register(&mut source, placement).expect("registers");
    transfer_entity(&mut source, &mut destination, first).expect("the first record moves");
    let holder = fixture.register(&mut source, placement).expect("registers");
    source.transforms_mut().insert(holder, flat_transform());
    source.open_dialogue(holder, dlg(5)).unwrap();

    let (source_text, destination_text) = (text(&source), text(&destination));
    assert_eq!(
        transfer_entity(&mut source, &mut destination, holder),
        Err(AreaError::PlacementInUse { placement })
    );
    assert_eq!(
        (text(&source), text(&destination)),
        (source_text, destination_text),
        "a refusal publishes neither side"
    );

    // Control: the same holder against a destination holding no record is
    // `ScriptState`, so the holder is still refused once placement is free.
    let mut free = World::new_in_area(0, u(3));
    assert_eq!(
        transfer_entity(&mut source, &mut free, holder),
        Err(AreaError::ScriptState { entity: holder })
    );

    // Control: after the dialogue closes the record moves.
    source.close_dialogue(holder).unwrap();
    let moved = transfer_entity(&mut source, &mut free, holder).expect("moves");
    assert!(free.contains(moved));
    assert!(!source.contains(holder));

    // The same precedence through the history pair.
    let mut source = HistoryWorld::new_in_area(0, fixture.area);
    let mut destination = HistoryWorld::new_in_area(0, u(2));
    let first = fixture
        .register_history(&mut source, placement)
        .expect("registers");
    transfer_history_entity(&mut source, &mut destination, first).expect("the first record moves");
    let holder = fixture
        .register_history(&mut source, placement)
        .expect("registers");
    source.open_dialogue(holder, dlg(5)).unwrap();

    let (source_text, destination_text) = (
        serde_json::to_string(&source).unwrap(),
        serde_json::to_string(&destination).unwrap(),
    );
    assert_eq!(
        transfer_history_entity(&mut source, &mut destination, holder),
        Err(AreaError::PlacementInUse { placement })
    );
    assert_eq!(
        (
            serde_json::to_string(&source).unwrap(),
            serde_json::to_string(&destination).unwrap()
        ),
        (source_text, destination_text),
        "a refusal publishes neither wrapper"
    );

    // Control: the same holder against a free destination is `ScriptState`.
    let mut free = HistoryWorld::new_in_area(0, u(3));
    assert_eq!(
        transfer_history_entity(&mut source, &mut free, holder),
        Err(AreaError::ScriptState { entity: holder })
    );

    // Control: after the dialogue closes the record moves.
    source.close_dialogue(holder).unwrap();
    let moved = transfer_history_entity(&mut source, &mut free, holder).expect("moves");
    assert!(free.world().contains(moved));
    assert!(!source.world().contains(holder));
}

// ------------------------------------------------------------------- C9 / B-10

/// A valid world: a holder `e(0)`, a participant `e(1)` with a dialogue and
/// two quests.
fn sample_world() -> World {
    let mut world = World::new(5);
    world.spawn_continuation(k(4));
    let p = world.spawn(EntityMeta {});
    world.open_dialogue(p, dlg(5)).unwrap();
    world.set_quest_state(u(7), u(5));
    world.set_quest_state(u(8), u(6));
    world
}

fn load(value: &Value) -> Result<World, String> {
    serde_json::from_value::<World>(value.clone()).map_err(|error| error.to_string())
}

#[test]
fn load_rejects_incoherent_state() {
    let world = sample_world();
    let valid = serde_json::to_value(&world).unwrap();
    // Control: the unedited JSON loads and equals the original.
    assert_eq!(load(&valid).as_ref(), Ok(&world));

    let ghost = json!({"index": 9, "generation": 1});

    let mut v = valid.clone();
    v["continuations"][0][0] = ghost.clone();
    let error = load(&v).unwrap_err();
    assert!(
        error.contains("dangling continuation for non-live"),
        "{error}"
    );

    let mut v = valid.clone();
    v["dialogues"][0][0] = ghost;
    let error = load(&v).unwrap_err();
    assert!(error.contains("dangling dialogue for non-live"), "{error}");

    let mut v = valid.clone();
    v["quests"].as_array_mut().unwrap().reverse();
    let error = load(&v).unwrap_err();
    assert!(
        error.contains("quest progress breaks ascending order"),
        "{error}"
    );

    let mut v = valid.clone();
    let first = v["quests"][0].clone();
    v["quests"] = json!([first.clone(), first]);
    let error = load(&v).unwrap_err();
    assert!(
        error.contains("quest progress breaks ascending order"),
        "{error}"
    );

    for (path, record) in [
        ("continuations", &valid["continuations"][0][1]),
        ("dialogues", &valid["dialogues"][0][1]),
        ("quests", &valid["quests"][0]),
    ] {
        let mut extra = record.clone();
        extra["x"] = json!(1);
        let mut v = valid.clone();
        match path {
            "continuations" => v["continuations"][0][1] = extra,
            "dialogues" => v["dialogues"][0][1] = extra,
            _ => v["quests"][0] = extra,
        }
        assert!(load(&v).is_err(), "an extra key in {path} is refused");
    }
}

#[test]
fn older_saves_without_the_keys_load_empty() {
    let mut value = serde_json::to_value(World::new(3)).unwrap();
    for name in ["continuations", "dialogues", "quests"] {
        assert!(value.get(name).is_none());
    }
    // Spawn something so the arena is not trivial, then drop nothing.
    let mut world = World::new(3);
    world.spawn(EntityMeta {});
    value = serde_json::to_value(&world).unwrap();
    let loaded = load(&value).unwrap();
    assert!(loaded.continuations().is_empty());
    assert!(loaded.dialogues().is_empty());
    assert!(loaded.quests().is_empty());
}

// ------------------------------------------------------------------- C10 / B-11

#[test]
fn round_trip_keeps_everything() {
    let mut world = World::new(5);
    world.spawn_continuation(k(4));
    world.spawn_continuation(k(0));
    let p = world.spawn(EntityMeta {});
    world.open_dialogue(p, dlg(5)).unwrap();
    world.set_quest_state(u(7), u(5));
    world.set_quest_state(u(8), u(6));

    let json = serde_json::to_string(&world).unwrap();
    let loaded: World = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded, world);
    assert_eq!(state_hash(&loaded), state_hash(&world));
    assert_eq!(loaded.due_continuations(), world.due_continuations());
    assert_eq!(loaded.due_continuations(), vec![e(1)]);
    assert_eq!(
        loaded.quests(),
        &[
            QuestProgress {
                quest: u(7),
                state: u(5)
            },
            QuestProgress {
                quest: u(8),
                state: u(6)
            }
        ]
    );

    // Control: one differing field hashes differently.
    let mut other = World::new(5);
    other.spawn_continuation(k(5));
    other.spawn_continuation(k(0));
    let p = other.spawn(EntityMeta {});
    other.open_dialogue(p, dlg(5)).unwrap();
    other.set_quest_state(u(7), u(5));
    other.set_quest_state(u(8), u(6));
    assert_ne!(state_hash(&other), state_hash(&world));

    // The same for a HistoryWorld and history_hash.
    let mut history = HistoryWorld::new(5);
    history.spawn_continuation(k(4)).unwrap();
    history.spawn_continuation(k(0)).unwrap();
    let p = history.spawn(EntityMeta {}).unwrap();
    history.open_dialogue(p, dlg(5)).unwrap();
    history.set_quest_state(u(7), u(5)).unwrap();
    let json = serde_json::to_string(&history).unwrap();
    let loaded: HistoryWorld = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded, history);
    assert_eq!(history_hash(&loaded), history_hash(&history));
    assert_eq!(
        loaded.world().due_continuations(),
        history.world().due_continuations()
    );
}

// ------------------------------------------------------------------- C11 / B-3, B-11

/// A world with the contract's mix, the `k(first)` holder first.
fn split_world(first: u64) -> (World, EntityId) {
    let mut world = World::new(42);
    let holder = world.spawn_continuation(k(first));
    world.spawn_continuation(k(0));
    let p = world.spawn(EntityMeta {});
    world.open_dialogue(p, dlg(5)).unwrap();
    world.set_quest_state(u(7), u(5));
    (world, holder)
}

/// Runs 1000 ticks, optionally through a save and load at 500, returning each
/// tick's hash and the first world tick at which `holder` was due.
fn run_world(mut world: World, holder: EntityId, reload: bool) -> (Vec<[u8; 32]>, Option<u64>) {
    let mut hashes = Vec::new();
    let mut first_due = None;
    for step in 1..=1000_u32 {
        tick(&mut world);
        if first_due.is_none() && world.due_continuations().contains(&holder) {
            first_due = Some(world.tick().get());
        }
        hashes.push(state_hash(&world));
        if reload && step == 500 {
            let json = serde_json::to_string(&world).unwrap();
            world = serde_json::from_str(&json).unwrap();
        }
    }
    (hashes, first_due)
}

fn run_history(
    mut history: HistoryWorld,
    holder: EntityId,
    reload: bool,
) -> (Vec<[u8; 32]>, Option<u64>) {
    let mut hashes = Vec::new();
    let mut first_due = None;
    for step in 1..=1000_u32 {
        history.tick().unwrap();
        if first_due.is_none() && history.world().due_continuations().contains(&holder) {
            first_due = Some(history.world().tick().get());
        }
        hashes.push(history_hash(&history));
        if reload && step == 500 {
            let json = serde_json::to_string(&history).unwrap();
            history = serde_json::from_str(&json).unwrap();
        }
    }
    (hashes, first_due)
}

#[test]
fn split_run_equals_uninterrupted() {
    let (world, holder) = split_world(600);
    let (whole, due_whole) = run_world(world.clone(), holder, false);
    let (split, due_split) = run_world(world, holder, true);
    assert_eq!(whole.len(), 1000);
    for (index, (a, b)) in whole.iter().zip(&split).enumerate() {
        assert_eq!(a, b, "tick {} differs between the two runs", index + 1);
    }
    assert_eq!(due_whole, Some(600));
    assert_eq!(due_split, Some(600));

    // Control: 601 is first due after tick 601.
    let (world, holder) = split_world(601);
    let (_, due) = run_world(world, holder, true);
    assert_eq!(due, Some(601));
}

fn history_split(first: u64) -> (HistoryWorld, EntityId) {
    let mut history = HistoryWorld::new(42);
    let holder = history.spawn_continuation(k(first)).unwrap();
    history.spawn_continuation(k(0)).unwrap();
    let p = history.spawn(EntityMeta {}).unwrap();
    history.open_dialogue(p, dlg(5)).unwrap();
    history.set_quest_state(u(7), u(5)).unwrap();
    (history, holder)
}

#[test]
fn history_split_run_equals_uninterrupted() {
    let (history, holder) = history_split(600);
    let (whole, due_whole) = run_history(history.clone(), holder, false);
    let (split, due_split) = run_history(history, holder, true);
    for (index, (a, b)) in whole.iter().zip(&split).enumerate() {
        assert_eq!(a, b, "tick {} differs between the two runs", index + 1);
    }
    assert_eq!(due_whole, Some(600));
    assert_eq!(due_split, Some(600));

    let (history, holder) = history_split(601);
    let (_, due) = run_history(history, holder, true);
    assert_eq!(due, Some(601));
}

// ------------------------------------------------------------------- C12 / B-12

#[test]
fn history_journals_only_spawns() {
    let mut h = HistoryWorld::new(3);
    let a = h.spawn_continuation(k(2)).unwrap();
    assert_eq!(a, e(0));
    assert_eq!(
        h.read_after(0, 256)
            .unwrap()
            .into_iter()
            .map(|envelope| envelope.payload)
            .collect::<Vec<_>>(),
        vec![HistoryEvent::Spawned { entity: e(0) }]
    );
    assert_eq!(h.version(), 1);

    let p = h.spawn(EntityMeta {}).unwrap();
    let (len, last) = (h.pending_len(), h.last_sequence());
    let mut hashes = vec![history_hash(&h)];
    macro_rules! step {
        ($call:expr) => {{
            $call.unwrap();
            assert_eq!(h.pending_len(), len, "a set journals nothing");
            assert_eq!(h.last_sequence(), last);
            assert_eq!(h.version(), 1);
            let hash = history_hash(&h);
            assert_ne!(hash, *hashes.last().unwrap(), "the call changed the hash");
            hashes.push(hash);
        }};
    }
    step!(h.set_continuation(a, k(5)));
    step!(h.open_dialogue(p, dlg(5)));
    step!(h.set_dialogue_node(p, u(6)));
    step!(h.close_dialogue(p));
    step!(h.set_quest_state(u(7), u(5)));
    step!(h.remove_quest(u(7)));

    // Return values pass through.
    assert_eq!(h.set_continuation(a, k(2)), Ok(k(5)));
    h.open_dialogue(p, dlg(5)).unwrap();
    assert_eq!(h.set_dialogue_node(p, u(6)), Ok(u(5)));
    assert_eq!(h.close_dialogue(p), Ok(dlg(6)));
    assert_eq!(h.set_quest_state(u(7), u(5)), Ok(None));
    assert_eq!(h.set_quest_state(u(7), u(6)), Ok(Some(u(5))));
    assert_eq!(h.remove_quest(u(7)), Ok(Some(u(6))));
    assert_eq!(h.remove_quest(u(7)), Ok(None));

    // A refusal is `Script(e)` and changes nothing.
    let hash = history_hash(&h);
    assert_eq!(
        h.set_continuation(e(7), k(1)),
        Err(HistoryError::Script(ContinuationError::NotContinuation {
            entity: e(7)
        }))
    );
    assert_eq!(
        h.open_dialogue(e(7), dlg(5)),
        Err(HistoryError::Script(ContinuationError::NotLive {
            entity: e(7)
        }))
    );
    h.open_dialogue(p, dlg(5)).unwrap();
    let hash_open = history_hash(&h);
    assert_eq!(
        h.open_dialogue(p, dlg(5)),
        Err(HistoryError::Script(ContinuationError::DialogueOpen {
            entity: p
        }))
    );
    h.close_dialogue(p).unwrap();
    assert_eq!(
        h.close_dialogue(p),
        Err(HistoryError::Script(ContinuationError::NoDialogue {
            entity: p
        }))
    );
    assert_eq!(
        h.set_dialogue_node(p, u(5)),
        Err(HistoryError::Script(ContinuationError::NoDialogue {
            entity: p
        }))
    );
    assert_ne!(hash, hash_open);
    assert_eq!(h.pending_len(), len);
}

#[test]
fn full_journal_blocks_only_the_spawn() {
    let mut h = HistoryWorld::new(3);
    for _ in 0..4096 {
        h.spawn(EntityMeta {}).unwrap();
    }
    let hash = history_hash(&h);
    assert_eq!(h.spawn_continuation(k(1)), Err(HistoryError::EventLimit));
    assert_eq!(history_hash(&h), hash, "a failed spawn commits nothing");

    // The six zero-event wrappers need no free journal slot.
    assert_eq!(h.set_quest_state(u(7), u(5)), Ok(None));
    assert_eq!(h.pending_len(), 4096);

    h.acknowledge(4096).unwrap();
    let minted = h.spawn_continuation(k(1)).unwrap();
    assert_eq!(minted, e(4096), "the refused call allocated nothing");
}

// ------------------------------------------------------------------- C13 / B-3, B-12

#[test]
fn history_tick_journals_nothing() {
    let mut h = HistoryWorld::new(3);
    let a = h.spawn_continuation(k(2)).unwrap();
    h.acknowledge(h.last_sequence()).unwrap();
    assert_eq!(h.pending_len(), 0);

    h.tick().unwrap();
    h.tick().unwrap();
    assert_eq!(h.pending_len(), 0);
    assert_eq!(h.version(), 1);
    assert_eq!(h.world().continuations().get(a).unwrap().remaining, 0);

    // Control: before the due tick the record was still positive.
    let mut early = HistoryWorld::new(3);
    let b = early.spawn_continuation(k(2)).unwrap();
    early.tick().unwrap();
    assert_eq!(early.world().continuations().get(b).unwrap().remaining, 1);
}

// ------------------------------------------------------------------- Errors table

#[test]
fn error_texts_and_sources() {
    let entity = e(0);
    for (error, text) in [
        (
            ContinuationError::NotContinuation { entity },
            "NotContinuation at script/state",
        ),
        (
            ContinuationError::NotLive { entity },
            "NotLive at script/state",
        ),
        (
            ContinuationError::DialogueOpen { entity },
            "DialogueOpen at script/state",
        ),
        (
            ContinuationError::NoDialogue { entity },
            "NoDialogue at script/state",
        ),
    ] {
        assert_eq!(error.to_string(), text);
        assert!(std::error::Error::source(&error).is_none());
        let wrapped = HistoryError::Script(error);
        assert_eq!(wrapped.to_string(), text);
        let source = std::error::Error::source(&wrapped).expect("Script wraps its error");
        assert_eq!(source.to_string(), text);
    }
    assert_eq!(
        AreaError::ScriptState { entity }.to_string(),
        "ScriptState at area/transfer"
    );
    assert!(std::error::Error::source(&AreaError::ScriptState { entity }).is_none());
}
