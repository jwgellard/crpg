//! T052 two-keys suite: persisted script, dialogue and quest state.
//!
//! Written independently from the contract text alone (`tasks/T052.md`,
//! Readings 1-10, Behaviour B-1..B-12 and the Errors table; B-13 is T052b and
//! not testable from `crpg-sim`). Expected values are contract literals or
//! hand arithmetic over them; nothing here was read off the implementation.
//! Test names carry the contract item they prove, and every error case has an
//! accepted control beside it. Refusals compare a complete world snapshot
//! (`state_hash` plus the three stores) before and after the call.

use std::collections::BTreeSet;

use crpg_core::{EntityId, Ulid};
use crpg_sim::{
    end_turn, history_hash, state_hash, tick, tick_with_movement, transfer_entity,
    transfer_history_entity, AreaError, ContinuationError, EntityMeta, HistoryError, HistoryEvent,
    HistoryWorld, OpenDialogue, QuestProgress, ScriptContinuation, SimEvent, Transform, World,
};
use serde::Serialize;
use serde_json::{json, Value};

// ---------------------------------------------------------------- fixtures

/// `u(n)`: the ULID parsed from 25 `0`s followed by the digit `n`.
fn u(n: u128) -> Ulid {
    let text = format!("{}{}", "0".repeat(25), n);
    text.parse().expect("a canonical ULID")
}

fn g() -> Ulid {
    u(1)
}
fn w() -> Ulid {
    u(2)
}
fn w2() -> Ulid {
    u(3)
}
fn d() -> Ulid {
    u(4)
}
fn n1() -> Ulid {
    u(5)
}
fn n2() -> Ulid {
    u(6)
}
fn q() -> Ulid {
    u(7)
}
fn q2() -> Ulid {
    u(8)
}
fn q3() -> Ulid {
    u(9)
}
fn s1() -> Ulid {
    u(5)
}
fn s2() -> Ulid {
    u(6)
}

/// `k(r)`.
fn k(remaining: u64) -> ScriptContinuation {
    ScriptContinuation {
        graph: g(),
        node: w(),
        remaining,
    }
}

fn dialogue(node: Ulid) -> OpenDialogue {
    OpenDialogue {
        dialogue: d(),
        node,
    }
}

/// `e(i)`, generation 1 (the sim's arena mints generations from 1).
fn e(index: u32) -> EntityId {
    e_gen(index, 1)
}

fn e_gen(index: u32, generation: u32) -> EntityId {
    serde_json::from_value(json!({"index": index, "generation": generation}))
        .expect("a well-formed entity id")
}

/// The top-level JSON keys of a serialized `World`.
fn keys(world: &World) -> BTreeSet<String> {
    let value = serde_json::to_value(world).expect("a world serializes");
    value
        .as_object()
        .expect("the world is a JSON object")
        .keys()
        .cloned()
        .collect()
}

fn stored<T: Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string(value).expect("a store serializes")
}

/// `(state_hash, continuations, dialogues, quests)` — a complete snapshot of
/// the authoritative state a refusal must not move.
type Snapshot = ([u8; 32], String, String, String);

fn snapshot(world: &World) -> Snapshot {
    (
        state_hash(world),
        stored(world.continuations()),
        stored(world.dialogues()),
        stored(world.quests()),
    )
}

fn history_snapshot(history: &HistoryWorld) -> ([u8; 32], String) {
    (
        history_hash(history),
        serde_json::to_string(history).expect("a wrapper serializes"),
    )
}

fn drained(world: &mut World) -> Vec<(u64, SimEvent)> {
    world
        .events_mut()
        .drain()
        .into_iter()
        .map(|envelope| (envelope.tick.get(), envelope.payload))
        .collect()
}

fn remaining(world: &World, entity: EntityId) -> u64 {
    world
        .continuations()
        .get(entity)
        .expect("a continuation holder")
        .remaining
}

fn history_payloads(history: &HistoryWorld) -> Vec<HistoryEvent> {
    history
        .read_after(history.acknowledged(), 256)
        .expect("a full read fits one page")
        .into_iter()
        .map(|envelope| envelope.payload)
        .collect()
}

// ------------------------------------------------------------------ B-1/C1

#[test]
fn b1_absent_state_is_not_serialized() {
    let mut world = World::new(0);
    let empty: BTreeSet<String> = [
        "entities",
        "transforms",
        "timeline",
        "events",
        "rng",
        "tick",
    ]
    .iter()
    .map(|key| (*key).to_owned())
    .collect();
    assert_eq!(keys(&world), empty, "an empty world keeps today's keys");

    let a = world.spawn_continuation(k(1));
    assert!(keys(&world).contains("continuations"));
    assert!(world.despawn(a));
    assert!(
        !keys(&world).contains("continuations"),
        "the key goes again"
    );

    let p = world.spawn(EntityMeta {});
    assert_eq!(world.open_dialogue(p, dialogue(n1())), Ok(()));
    assert!(keys(&world).contains("dialogues"));
    assert_eq!(world.close_dialogue(p), Ok(dialogue(n1())));
    assert!(!keys(&world).contains("dialogues"), "the key goes again");

    assert_eq!(world.set_quest_state(q(), s1()), None);
    assert!(keys(&world).contains("quests"));
    assert_eq!(world.remove_quest(q()), Some(s1()));
    assert!(!keys(&world).contains("quests"), "the key goes again");
}

// ------------------------------------------------------------------ B-2/C2

#[test]
fn b2_spawn_continuation_mints_like_spawn() {
    let mut world = World::new(3);
    let minted = world.spawn_continuation(k(2));
    assert_eq!(minted, e(0), "the first spawn takes index 0, generation 1");
    assert_eq!(world.len(), 1);
    assert_eq!(world.continuations().get(minted), Some(&k(2)));
    assert!(world.transforms().is_empty(), "no transform");
    assert!(world.timeline().is_empty(), "no timeline entry");
    assert!(world.creatures().is_empty(), "no creature record");
    assert!(world.movers().is_empty(), "no mover");

    let events = drained(&mut world);
    assert_eq!(
        events,
        vec![(0, SimEvent::Spawned { entity: e(0) })],
        "one Spawned at the current tick"
    );

    // Control: one input (the tick) differs, and the stamp follows it.
    let mut late = World::new(3);
    for _ in 0..3 {
        tick(&mut late);
    }
    let later = late.spawn_continuation(k(1));
    assert_eq!(later, e(0));
    assert_eq!(
        drained(&mut late),
        vec![(3, SimEvent::Spawned { entity: later })]
    );
}

// ------------------------------------------------------------------ B-3/C2

#[test]
fn b3_wait_counts_down_and_becomes_due() {
    let mut world = World::new(1);
    let a = world.spawn_continuation(k(3));
    let b = world.spawn_continuation(k(0));
    assert_eq!((a, b), (e(0), e(1)));

    let spawned = drained(&mut world);
    assert_eq!(
        spawned,
        vec![
            (0, SimEvent::Spawned { entity: a }),
            (0, SimEvent::Spawned { entity: b }),
        ],
        "the queue holds exactly the two spawns, at tick 0"
    );
    assert_eq!(world.due_continuations(), vec![b], "zero is due");

    tick(&mut world);
    assert_eq!((remaining(&world, a), remaining(&world, b)), (2, 0));
    assert_eq!(world.due_continuations(), vec![b]);

    // Control: `end_turn` never touches the stores.
    end_turn(&mut world);
    assert_eq!((remaining(&world, a), remaining(&world, b)), (2, 0));
    assert_eq!(world.due_continuations(), vec![b]);

    tick(&mut world);
    assert_eq!(remaining(&world, a), 1);
    tick(&mut world);
    assert_eq!(remaining(&world, a), 0, "0 first at tick t + r");
    assert_eq!(world.due_continuations(), vec![a, b], "ascending EntityId");
    assert!(world.events().is_empty(), "the countdown enqueues nothing");

    // A due record stays, and tick four changes neither.
    tick(&mut world);
    assert_eq!((remaining(&world, a), remaining(&world, b)), (0, 0));
    assert_eq!(world.due_continuations(), vec![a, b]);
    assert!(world.events().is_empty());

    // Control: `u64::MAX` after one tick is `u64::MAX - 1` (no wraparound).
    let mut big = World::new(1);
    let max = big.spawn_continuation(k(u64::MAX));
    tick(&mut big);
    assert_eq!(remaining(&big, max), u64::MAX - 1);
    assert!(big.due_continuations().is_empty());
}

#[test]
fn b3_tick_with_movement_and_history_tick_count_down() {
    let mut world = World::new(1);
    let holder = world.spawn_continuation(k(2));
    let facts = tick_with_movement(&mut world);
    assert!(facts.is_empty(), "nothing moved");
    assert_eq!(remaining(&world, holder), 1);

    // `HistoryWorld::tick` counts down too (C13).
    let mut history = HistoryWorld::new(1);
    let holder = history.spawn_continuation(k(2)).expect("mints");
    history
        .acknowledge(history.last_sequence())
        .expect("drains");
    history.tick().expect("ticks");
    assert_eq!(remaining(history.world(), holder), 1);
    history.tick().expect("ticks");
    assert_eq!(remaining(history.world(), holder), 0);
}

// ------------------------------------------------------------------ B-4/C3

#[test]
fn b4_due_order_is_entity_order() {
    let mut world = World::new(2);
    let first = world.spawn_continuation(k(0));
    let second = world.spawn_continuation(k(0));
    let third = world.spawn_continuation(k(0));
    assert!(world.despawn(first));
    // The reused slot is last in store order, first in EntityId order.
    let reused = world.spawn_continuation(k(0));
    assert_eq!(reused, e_gen(0, 2));

    let store_order: Vec<EntityId> = world
        .continuations()
        .iter()
        .map(|(entity, _)| entity)
        .collect();
    assert_eq!(
        store_order,
        vec![second, third, reused],
        "store is insertion order unsorted"
    );

    let before = state_hash(&world);
    let due = world.due_continuations();
    assert_eq!(
        due,
        vec![reused, second, third],
        "ascending EntityId, not store order"
    );
    let indices: Vec<u32> = due.iter().map(|entity| entity.index()).collect();
    assert_eq!(indices, vec![0, 1, 2]);
    assert_eq!(state_hash(&world), before, "the query changes nothing");
}

// ------------------------------------------------------------------ B-5/C4

#[test]
fn b5_set_continuation_replaces() {
    let mut world = World::new(4);
    let a = world.spawn_continuation(k(5));
    let plain = world.spawn(EntityMeta {});
    let replacement = ScriptContinuation {
        graph: g(),
        node: w2(),
        remaining: 9,
    };

    let queued = world.events().len();
    assert_eq!(
        world.set_continuation(a, replacement),
        Ok(k(5)),
        "the previous record is returned"
    );
    assert_eq!(world.continuations().get(a), Some(&replacement));
    assert_eq!(world.events().len(), queued, "nothing is enqueued");

    // A plain entity, an id that was never live, and a dead holder all refuse.
    for (target, name) in [(plain, "a plain entity"), (e(7), "a never-live id")] {
        let before = snapshot(&world);
        assert_eq!(
            world.set_continuation(target, replacement),
            Err(ContinuationError::NotContinuation { entity: target }),
            "{name} holds no continuation"
        );
        assert_eq!(
            snapshot(&world),
            before,
            "a refusal leaves the world unchanged"
        );
    }
    assert!(world.despawn(a));
    let before = snapshot(&world);
    assert_eq!(
        world.set_continuation(a, replacement),
        Err(ContinuationError::NotContinuation { entity: a }),
        "a despawned holder is a non-holder"
    );
    assert_eq!(snapshot(&world), before);
}

// ------------------------------------------------------------------ B-6/C5

#[test]
fn b6_dialogue_lifecycle() {
    let mut world = World::new(5);
    let p = world.spawn(EntityMeta {});
    let p2 = world.spawn(EntityMeta {});
    assert_eq!(drained(&mut world).len(), 2, "only the two Spawned");

    // None open: both node moves and close refuse.
    for _ in 0..2 {
        let before = snapshot(&world);
        assert_eq!(
            world.set_dialogue_node(p, n2()),
            Err(ContinuationError::NoDialogue { entity: p })
        );
        assert_eq!(snapshot(&world), before);
    }
    let before = snapshot(&world);
    assert_eq!(
        world.close_dialogue(p),
        Err(ContinuationError::NoDialogue { entity: p })
    );
    assert_eq!(snapshot(&world), before);
    let before = snapshot(&world);
    assert_eq!(
        world.open_dialogue(e(7), dialogue(n1())),
        Err(ContinuationError::NotLive { entity: e(7) })
    );
    assert_eq!(snapshot(&world), before, "a dead id is not live");

    // Open, refuse a second, then open one per participant.
    assert_eq!(world.open_dialogue(p, dialogue(n1())), Ok(()));
    let before = snapshot(&world);
    assert_eq!(
        world.open_dialogue(p, dialogue(n1())),
        Err(ContinuationError::DialogueOpen { entity: p })
    );
    assert_eq!(
        snapshot(&world),
        before,
        "a second dialogue changes nothing"
    );
    assert_eq!(world.open_dialogue(p2, dialogue(n1())), Ok(()));
    assert_eq!(world.dialogues().len(), 2, "two participants talk at once");

    // Move, close, and refuse afterwards.
    let before = snapshot(&world);
    assert_eq!(world.set_dialogue_node(p, n2()), Ok(n1()));
    assert_eq!(world.dialogues().get(p), Some(&dialogue(n2())));
    assert_ne!(snapshot(&world), before);
    assert_eq!(world.close_dialogue(p), Ok(dialogue(n2())));
    let before = snapshot(&world);
    assert_eq!(
        world.close_dialogue(p),
        Err(ContinuationError::NoDialogue { entity: p })
    );
    assert_eq!(snapshot(&world), before);
    assert_eq!(
        world.set_dialogue_node(p, n1()),
        Err(ContinuationError::NoDialogue { entity: p })
    );
    assert_eq!(snapshot(&world), before);
    assert!(world.events().is_empty(), "nothing is ever enqueued");
}

// ------------------------------------------------------------------ B-7/C6

#[test]
fn b7_quests_stay_ascending() {
    let mut world = World::new(6);
    assert_eq!(world.set_quest_state(q2(), s1()), None);
    assert_eq!(world.set_quest_state(q(), s1()), None);
    assert_eq!(
        world.quests(),
        &[
            QuestProgress {
                quest: q(),
                state: s1()
            },
            QuestProgress {
                quest: q2(),
                state: s1()
            }
        ][..],
        "ascending by quest, not insertion order"
    );

    assert_eq!(world.set_quest_state(q(), s2()), Some(s1()));
    let before = state_hash(&world);
    assert_eq!(
        world.set_quest_state(q(), s2()),
        Some(s2()),
        "the previous state, even when equal"
    );
    assert_eq!(state_hash(&world), before, "an equal write changes nothing");
    assert_eq!(world.quest_state(q()), Some(s2()));
    assert_eq!(world.remove_quest(q2()), Some(s1()));
    assert_eq!(world.remove_quest(q2()), None);
    assert!(world.events().is_empty(), "nothing is enqueued");

    // Boundaries: inserted before, between and after existing quests.
    let mut ordered = World::new(6);
    let before_id = u(6);
    assert_eq!(ordered.set_quest_state(q3(), s1()), None);
    assert_eq!(ordered.set_quest_state(q(), s1()), None);
    assert_eq!(ordered.set_quest_state(before_id, s1()), None);
    assert_eq!(ordered.set_quest_state(q2(), s1()), None);
    let quest_order: Vec<Ulid> = ordered.quests().iter().map(|entry| entry.quest).collect();
    assert_eq!(quest_order, vec![before_id, q(), q2(), q3()]);
    assert_eq!(ordered.quest_state(q2()), Some(s1()));
    assert_eq!(ordered.quest_state(Ulid::NIL), None);
    assert_eq!(
        ordered.remove_quest(Ulid::NIL),
        None,
        "liveness is not involved"
    );
}

// ------------------------------------------------------------------ B-8/C7

#[test]
fn b8_despawn_strips_script_state() {
    let mut world = World::new(7);
    let holder = world.spawn_continuation(k(1));
    assert_eq!(world.open_dialogue(holder, dialogue(n1())), Ok(()));
    assert_eq!(world.set_quest_state(q(), s1()), None);
    let quests = world.quests().to_vec();
    let queued = world.events().len();

    assert!(world.despawn(holder), "a live holder despawns");
    assert!(world.continuations().is_empty());
    assert!(world.dialogues().is_empty());
    assert_eq!(world.quests(), &quests[..], "quests are untouched");
    assert_eq!(world.events().len(), queued + 1, "exactly one Despawned");
    assert_eq!(
        drained(&mut world),
        vec![
            (0, SimEvent::Spawned { entity: holder }),
            (0, SimEvent::Despawned { entity: holder }),
        ]
    );
    assert!(!world.despawn(holder), "a dead id despawns nothing");

    // Control: a dead id appends nothing.
    let mut control = World::new(7);
    assert!(!control.despawn(e(7)));
    assert!(control.events().is_empty());
}

// ------------------------------------------------------------------ B-9/C8

#[test]
fn b9_transfer_refuses_holders() {
    let mut west = World::new_in_area(0, u(1));
    let mut east = World::new_in_area(0, u(2));

    // A continuation holder is refused; neither side is published.
    let holder = west.spawn_continuation(k(1));
    let before = (state_hash(&west), state_hash(&east));
    assert_eq!(
        transfer_entity(&mut west, &mut east, holder),
        Err(AreaError::ScriptState { entity: holder })
    );
    assert_eq!((state_hash(&west), state_hash(&east)), before);
    assert!(west.continuations().contains(holder));
    assert_eq!(east.len(), 0);

    // A dialogue participant with a transform is refused the same way.
    let talker = west.spawn(EntityMeta {});
    west.transforms_mut().insert(
        talker,
        Transform {
            position: [1.0, 2.0, 3.0],
            velocity: [0.0; 3],
        },
    );
    assert_eq!(west.open_dialogue(talker, dialogue(n1())), Ok(()));
    let before = (state_hash(&west), state_hash(&east));
    assert_eq!(
        transfer_entity(&mut west, &mut east, talker),
        Err(AreaError::ScriptState { entity: talker })
    );
    assert_eq!((state_hash(&west), state_hash(&east)), before);
    assert!(east.dialogues().is_empty(), "no side receives the dialogue");

    // Control: once closed, the same participant transfers.
    assert_eq!(west.close_dialogue(talker), Ok(dialogue(n1())));
    let moved = transfer_entity(&mut west, &mut east, talker).expect("moves");
    assert!(!west.contains(talker));
    assert!(east.contains(moved));
    assert!(east.dialogues().is_empty());
}

#[test]
fn b9_precedence_and_history_refusal() {
    // An earlier check (`InvalidTransform`) keeps winning over `ScriptState`.
    let mut west = World::new_in_area(0, u(1));
    let mut east = World::new_in_area(0, u(2));
    let nan = west.spawn(EntityMeta {});
    west.transforms_mut().insert(
        nan,
        Transform {
            position: [f32::NAN, 0.0, 0.0],
            velocity: [0.0; 3],
        },
    );
    assert_eq!(west.open_dialogue(nan, dialogue(n1())), Ok(()));
    // `state_hash` panics on non-finite input, so compare the JSON instead.
    let before = (
        serde_json::to_string(&west).expect("serializes"),
        serde_json::to_string(&east).expect("serializes"),
    );
    assert_eq!(
        transfer_entity(&mut west, &mut east, nan),
        Err(AreaError::InvalidTransform { entity: nan })
    );
    let after = (
        serde_json::to_string(&west).expect("serializes"),
        serde_json::to_string(&east).expect("serializes"),
    );
    assert_eq!(after, before);

    // The history pair refuses both kinds and publishes neither side.
    let mut source = HistoryWorld::new_in_area(0, u(1));
    let mut destination = HistoryWorld::new_in_area(0, u(2));
    let holder = source.spawn_continuation(k(1)).expect("mints");
    let before = (history_hash(&source), history_hash(&destination));
    assert_eq!(
        transfer_history_entity(&mut source, &mut destination, holder),
        Err(AreaError::ScriptState { entity: holder })
    );
    assert_eq!((history_hash(&source), history_hash(&destination)), before);

    let talker = source.spawn(EntityMeta {}).expect("spawns");
    source.open_dialogue(talker, dialogue(n1())).expect("opens");
    let before = (history_hash(&source), history_hash(&destination));
    assert_eq!(
        transfer_history_entity(&mut source, &mut destination, talker),
        Err(AreaError::ScriptState { entity: talker })
    );
    assert_eq!((history_hash(&source), history_hash(&destination)), before);

    // Control: closed, the participant transfers.
    source.close_dialogue(talker).expect("closes");
    let moved = transfer_history_entity(&mut source, &mut destination, talker).expect("moves");
    assert!(destination.world().contains(moved));
    assert!(destination.world().dialogues().is_empty());
}

// ----------------------------------------------------------------- B-10/C9

fn load_error(value: Value) -> String {
    let error = serde_json::from_value::<World>(value).expect_err("the save is refused");
    error.to_string()
}

#[test]
fn b10_load_rejects_incoherent_state() {
    let mut world = World::new(9);
    world.spawn_continuation(k(1));
    let participant = world.spawn(EntityMeta {});
    assert_eq!(world.open_dialogue(participant, dialogue(n1())), Ok(()));
    assert_eq!(world.set_quest_state(q(), s1()), None);
    assert_eq!(world.set_quest_state(q2(), s2()), None);
    let value = serde_json::to_value(&world).expect("serializes");

    // Control: the unedited JSON loads and equals the original.
    let loaded: World = serde_json::from_value(value.clone()).expect("the control loads");
    assert_eq!(loaded, world);

    // L1: a continuation key not live.
    let mut edited = value.clone();
    edited["continuations"][0][0] = json!({"index": 9, "generation": 1});
    let message = load_error(edited);
    assert_eq!(
        message,
        format!("dangling continuation for non-live {:?}", e(9)),
        "L1 text"
    );

    // L2: a dialogue key not live.
    let mut edited = value.clone();
    edited["dialogues"][0][0] = json!({"index": 9, "generation": 1});
    let message = load_error(edited);
    assert_eq!(
        message,
        format!("dangling dialogue for non-live {:?}", e(9)),
        "L2 text"
    );

    // L3: two adjacent quests out of ascending order (swapped and equal).
    for quests in [
        json!([{"quest": q2(), "state": s2()}, {"quest": q(), "state": s1()}]),
        json!([{"quest": q(), "state": s1()}, {"quest": q(), "state": s1()}]),
    ] {
        let mut edited = value.clone();
        edited["quests"] = quests;
        let message = load_error(edited);
        assert_eq!(message, "quest progress breaks ascending order", "L3 text");
    }

    // A record field the shape does not know is refused.
    let mut edited = value.clone();
    edited["continuations"][0][1]["x"] = json!(1);
    assert!(serde_json::from_value::<World>(edited).is_err());

    // Order: L1, then L2, then L3. One save carries all three faults.
    let mut edited = value.clone();
    edited["continuations"][0][0] = json!({"index": 9, "generation": 1});
    edited["dialogues"][0][0] = json!({"index": 9, "generation": 1});
    edited["quests"] = json!([{"quest": q2(), "state": s2()}, {"quest": q(), "state": s1()}]);
    let message = load_error(edited.clone());
    assert!(
        message.contains("dangling continuation"),
        "L1 wins: {message}"
    );
    edited["continuations"][0][0] = json!({"index": 0, "generation": 1});
    let message = load_error(edited.clone());
    assert!(message.contains("dangling dialogue"), "then L2: {message}");
    edited["dialogues"][0][0] = json!({"index": 1, "generation": 1});
    let message = load_error(edited.clone());
    assert!(
        message.contains("quest progress breaks ascending order"),
        "then L3: {message}"
    );

    // Every existing check keeps winning over L1-L3.
    let mut transform_fault = edited;
    transform_fault["continuations"][0][0] = json!({"index": 9, "generation": 1});
    let mut only_transform = transform_fault.clone();
    only_transform["continuations"][0][0] = json!({"index": 0, "generation": 1});
    only_transform["quests"] = value["quests"].clone();
    // A transform for a non-live entity is one existing check.
    let dangling = serde_json::to_value(Transform::default()).expect("serializes");
    transform_fault["transforms"]
        .as_array_mut()
        .expect("a list")
        .push(json!([{"index": 8, "generation": 1}, dangling.clone()]));
    only_transform["transforms"]
        .as_array_mut()
        .expect("a list")
        .push(json!([{"index": 8, "generation": 1}, dangling]));
    let with_script_faults = load_error(transform_fault);
    let without = load_error(only_transform);
    assert!(
        without.contains("transform") || without.contains("non-live"),
        "the existing check is the transform one: {without}"
    );
    assert_eq!(
        with_script_faults, without,
        "an existing check's error replaces L1"
    );
}

// ---------------------------------------------------------------- B-11/C10

fn full_world(seed: u64, remaining: u64) -> World {
    let mut world = World::new(seed);
    world.spawn_continuation(k(remaining));
    world.spawn_continuation(k(0));
    let participant = world.spawn(EntityMeta {});
    assert_eq!(world.open_dialogue(participant, dialogue(n1())), Ok(()));
    assert_eq!(world.set_quest_state(q(), s1()), None);
    assert_eq!(world.set_quest_state(q2(), s2()), None);
    world
}

#[test]
fn b11_round_trip_keeps_everything() {
    let world = full_world(5, 4);
    let json = serde_json::to_string(&world).expect("serializes");
    let loaded: World = serde_json::from_str(&json).expect("loads");
    assert_eq!(loaded, world, "an equal value");
    assert_eq!(state_hash(&loaded), state_hash(&world));
    assert_eq!(loaded.due_continuations(), world.due_continuations());
    assert_eq!(loaded.quests(), world.quests());
    assert_eq!(loaded.continuations().len(), 2);
    assert_eq!(loaded.dialogues().len(), 1);

    // Control: one input (`remaining`) differs and the hash moves.
    let control = full_world(5, 5);
    assert_ne!(state_hash(&control), state_hash(&world));
}

// ---------------------------------------------------------------- B-11/C11

fn seeded_world(seed: u64, remaining: u64) -> (World, EntityId) {
    let mut world = World::new(seed);
    let holder = world.spawn_continuation(k(remaining));
    world.spawn_continuation(k(0));
    let participant = world.spawn(EntityMeta {});
    assert_eq!(world.open_dialogue(participant, dialogue(n1())), Ok(()));
    assert_eq!(world.set_quest_state(q(), s1()), None);
    (world, holder)
}

fn seeded_history(seed: u64, remaining: u64) -> (HistoryWorld, EntityId) {
    let mut history = HistoryWorld::new(seed);
    let holder = history.spawn_continuation(k(remaining)).expect("mints");
    history.spawn_continuation(k(0)).expect("mints");
    let participant = history.spawn(EntityMeta {}).expect("spawns");
    history
        .open_dialogue(participant, dialogue(n1()))
        .expect("opens");
    history.set_quest_state(q(), s1()).expect("sets");
    (history, holder)
}

#[test]
fn b11_split_run_equals_uninterrupted() {
    // Spec :853 over `World`: 500 ticks, serialize, 500 more.
    let (mut run_a, holder_a) = seeded_world(42, 600);
    let mut hashes_a = Vec::new();
    let mut due_a = None;
    for step in 1..=1000 {
        tick(&mut run_a);
        hashes_a.push(state_hash(&run_a));
        if due_a.is_none() && run_a.due_continuations().contains(&holder_a) {
            due_a = Some(step);
        }
    }

    let (mut run_b, holder_b) = seeded_world(42, 600);
    let mut hashes_b = Vec::new();
    let mut due_b = None;
    for step in 1..=500 {
        tick(&mut run_b);
        hashes_b.push(state_hash(&run_b));
        if due_b.is_none() && run_b.due_continuations().contains(&holder_b) {
            due_b = Some(step);
        }
    }
    let json = serde_json::to_string(&run_b).expect("serializes at 500");
    run_b = serde_json::from_str(&json).expect("loads at 500");
    for step in 501..=1000 {
        tick(&mut run_b);
        hashes_b.push(state_hash(&run_b));
        if due_b.is_none() && run_b.due_continuations().contains(&holder_b) {
            due_b = Some(step);
        }
    }
    assert_eq!(hashes_a, hashes_b, "no pair differs across the split");
    assert_eq!((due_a, due_b), (Some(600), Some(600)));

    // Control: `k(601)` is first due after tick 601.
    let (mut control, holder) = seeded_world(42, 601);
    let mut control_due = None;
    for step in 1..=601 {
        tick(&mut control);
        if control_due.is_none() && control.due_continuations().contains(&holder) {
            control_due = Some(step);
        }
    }
    assert_eq!(control_due, Some(601));

    // The same over `HistoryWorld`, `history_hash` per tick.
    let (mut run_a, holder_a) = seeded_history(42, 600);
    let mut hashes_a = Vec::new();
    let mut due_a = None;
    for step in 1..=1000 {
        run_a.tick().expect("ticks");
        hashes_a.push(history_hash(&run_a));
        if due_a.is_none() && run_a.world().due_continuations().contains(&holder_a) {
            due_a = Some(step);
        }
    }

    let (mut run_b, holder_b) = seeded_history(42, 600);
    let mut hashes_b = Vec::new();
    let mut due_b = None;
    for step in 1..=500 {
        run_b.tick().expect("ticks");
        hashes_b.push(history_hash(&run_b));
        if due_b.is_none() && run_b.world().due_continuations().contains(&holder_b) {
            due_b = Some(step);
        }
    }
    let json = serde_json::to_string(&run_b).expect("serializes at 500");
    run_b = serde_json::from_str(&json).expect("loads at 500");
    for step in 501..=1000 {
        run_b.tick().expect("ticks");
        hashes_b.push(history_hash(&run_b));
        if due_b.is_none() && run_b.world().due_continuations().contains(&holder_b) {
            due_b = Some(step);
        }
    }
    assert_eq!(hashes_a, hashes_b, "no history pair differs");
    assert_eq!((due_a, due_b), (Some(600), Some(600)));
}

// ---------------------------------------------------------------- B-12/C12

#[test]
fn b12_history_journals_only_spawns() {
    let mut history = HistoryWorld::new(3);
    let holder = history.spawn_continuation(k(2)).expect("mints");
    assert_eq!(holder, e(0));
    assert_eq!(
        history_payloads(&history),
        vec![HistoryEvent::Spawned { entity: e(0) }],
        "exactly one Spawned"
    );
    assert_eq!(history.pending_len(), 1);
    assert_eq!(history.version(), 1);
    let participant = history.spawn(EntityMeta {}).expect("spawns");
    assert_eq!(history.pending_len(), 2);

    // Each of the six other wrappers adds no envelope, keeps version 1 and
    // changes the wrapper hash.
    let mut quiet = |action: &mut dyn FnMut(&mut HistoryWorld)| {
        let pending = history.pending_len();
        let version = history.version();
        let before = history_hash(&history);
        action(&mut history);
        assert_eq!(history.pending_len(), pending, "no envelope");
        assert_eq!(history.version(), version, "the version does not move");
        assert_ne!(
            history_hash(&history),
            before,
            "the write changed the world"
        );
    };
    quiet(&mut |h| {
        let replacement = ScriptContinuation {
            graph: g(),
            node: w2(),
            remaining: 9,
        };
        assert_eq!(h.set_continuation(holder, replacement), Ok(k(2)));
    });
    quiet(&mut |h| {
        assert_eq!(h.open_dialogue(participant, dialogue(n1())), Ok(()));
    });
    quiet(&mut |h| {
        assert_eq!(h.set_dialogue_node(participant, n2()), Ok(n1()));
    });
    quiet(&mut |h| {
        assert_eq!(h.close_dialogue(participant), Ok(dialogue(n2())));
    });
    quiet(&mut |h| {
        assert_eq!(h.set_quest_state(q(), s1()), Ok(None));
    });
    quiet(&mut |h| {
        assert_eq!(h.remove_quest(q()), Ok(Some(s1())));
    });

    // A refusal is `Script(error)` and moves nothing.
    let before = history_snapshot(&history);
    assert_eq!(
        history.set_continuation(e(7), k(1)),
        Err(HistoryError::Script(ContinuationError::NotContinuation {
            entity: e(7)
        }))
    );
    assert_eq!(history_snapshot(&history), before);
}

#[test]
fn b12_full_journal_and_zero_event_writes() {
    let mut history = HistoryWorld::new(3);
    for _ in 0..4096 {
        history.spawn(EntityMeta {}).expect("the journal fills");
    }
    assert_eq!(history.pending_len(), 4096);

    // A zero-event write needs no free slot, even with the journal full.
    assert_eq!(history.set_quest_state(q(), s1()), Ok(None));
    assert_eq!(history.pending_len(), 4096);

    // An emitting spawn is refused and allocates nothing.
    let before = history_snapshot(&history);
    assert_eq!(
        history.spawn_continuation(k(1)),
        Err(HistoryError::EventLimit)
    );
    assert_eq!(history_snapshot(&history), before);
    assert_eq!(history.world().len(), 4096, "no entity was allocated");

    history.acknowledge(4096).expect("drains");
    let minted = history.spawn_continuation(k(1)).expect("mints");
    assert_eq!(
        (minted.index(), minted.generation()),
        (4096, 1),
        "the refused call took no index"
    );
}

// ---------------------------------------------------------------- B-12/C13

#[test]
fn c13_history_tick_journals_nothing() {
    let mut history = HistoryWorld::new(3);
    let holder = history.spawn_continuation(k(2)).expect("mints");
    history
        .acknowledge(history.last_sequence())
        .expect("drains");
    history.tick().expect("ticks");
    assert_eq!(history.pending_len(), 0, "the countdown journals nothing");
    history.tick().expect("ticks");
    assert_eq!(history.pending_len(), 0);
    assert_eq!(history.version(), 1);
    assert_eq!(remaining(history.world(), holder), 0);
}

// --------------------------------------------------------------- Errors

#[test]
fn errors_have_the_contract_variant_and_display() {
    let refusals: [(ContinuationError, &str); 4] = [
        (
            ContinuationError::NotContinuation { entity: e(0) },
            "NotContinuation",
        ),
        (ContinuationError::NotLive { entity: e(1) }, "NotLive"),
        (
            ContinuationError::DialogueOpen { entity: e(2) },
            "DialogueOpen",
        ),
        (ContinuationError::NoDialogue { entity: e(3) }, "NoDialogue"),
    ];
    for (error, name) in refusals {
        assert_eq!(error.to_string(), format!("{name} at script/state"));
        assert!(std::error::Error::source(&error).is_none(), "no source");
    }

    let area = AreaError::ScriptState { entity: e(4) };
    assert_eq!(area.to_string(), "ScriptState at area/transfer");

    // Through the wrapper: the same text, wrapped as `Script` with a source.
    let mut history = HistoryWorld::new(0);
    let wrapped = history
        .set_continuation(e(7), k(1))
        .expect_err("a non-holder is refused");
    assert_eq!(
        wrapped,
        HistoryError::Script(ContinuationError::NotContinuation { entity: e(7) })
    );
    assert_eq!(wrapped.to_string(), "NotContinuation at script/state");
    let source = std::error::Error::source(&wrapped).expect("a source");
    assert_eq!(source.to_string(), "NotContinuation at script/state");
}
