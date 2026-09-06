//! Ordering and persistence tests for the generic event substrate (T007,
//! ADR-0008). Payloads here are plain integers — a core-closed type — because
//! the substrate itself must never name a game concept, not even in tests.

use crpg_core::{EventQueue, Tick};
use proptest::prelude::*;

fn tick(v: u64) -> Tick {
    Tick::new(v)
}

proptest! {
    #[test]
    fn drain_is_ascending_tick_then_seq_and_clears(
        pushes in prop::collection::vec((any::<u64>(), any::<i32>()), 0..200),
    ) {
        let mut queue = EventQueue::new();
        for (t, payload) in &pushes {
            queue.push(tick(*t), *payload);
        }
        prop_assert_eq!(queue.len(), pushes.len());
        let drained = queue.drain();
        prop_assert_eq!(drained.len(), pushes.len());
        let mut last: Option<(Tick, u64)> = None;
        for envelope in &drained {
            let key = (envelope.tick, envelope.seq);
            if let Some(prev) = last {
                prop_assert!(prev <= key, "drain out of order: {prev:?} then {key:?}");
            }
            last = Some(key);
        }
        // Sequence numbers are push order: the k-th pushed envelope has seq k,
        // whatever tick it carries.
        let mut seqs: Vec<u64> = drained.iter().map(|e| e.seq).collect();
        seqs.sort_unstable();
        let expected: Vec<u64> = (0..pushes.len() as u64).collect();
        prop_assert_eq!(seqs, expected);
        // Draining clears: length drops and a second drain is empty.
        prop_assert!(queue.is_empty());
        prop_assert!(queue.drain().is_empty());
    }

    #[test]
    fn serde_round_trip_preserves_order_and_counter(
        pushes in prop::collection::vec((any::<u64>(), any::<i32>()), 0..100),
    ) {
        let mut queue = EventQueue::new();
        for (t, payload) in &pushes {
            queue.push(tick(*t), *payload);
        }
        // Drain-then-repush exercises the serialized sequence counter: the
        // re-pushed envelopes must not reuse sequence numbers.
        let drained = queue.drain();
        for envelope in &drained {
            queue.push(envelope.tick, envelope.payload);
        }
        let json = serde_json::to_string(&queue).unwrap();
        let mut loaded: EventQueue<i32> = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(loaded.clone(), queue.clone());
        prop_assert_eq!(loaded.drain(), queue.drain());
    }
}

#[test]
fn push_assigns_monotonic_sequence_numbers() {
    let mut queue = EventQueue::new();
    queue.push(Tick::ZERO, "first");
    queue.push(Tick::ZERO, "second");
    let drained = queue.drain();
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].seq, 0);
    assert_eq!(drained[1].seq, 1);
    // Same tick keeps push order: the sort is stable.
    assert_eq!(drained[0].payload, "first");
    assert_eq!(drained[1].payload, "second");
}

#[test]
fn drain_of_empty_queue_is_empty() {
    let mut queue: EventQueue<i32> = EventQueue::new();
    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);
    assert!(queue.drain().is_empty());
}

#[test]
fn deserialize_rejects_stale_next_seq() {
    let mut queue = EventQueue::new();
    queue.push(tick(0), 10);
    let mut value = serde_json::to_value(&queue).unwrap();
    // Envelope holds seq 0; rewinding the counter would reuse it on push.
    value["next_seq"] = serde_json::Value::from(0u64);
    // next_seq == max is also stale: the next push must exceed, not meet.
    let result: Result<EventQueue<i32>, _> = serde_json::from_value(value.clone());
    assert!(result.is_err(), "next_seq == max must fail to load");
    value["next_seq"] = serde_json::Value::from(0u64);
    // And a larger envelope makes the gap explicit: seq 0 with next 0 is
    // tested above; bump the envelope to seq 10 with next 0.
    let mut envelopes = value["envelopes"].as_array().unwrap().clone();
    envelopes[0]["seq"] = serde_json::Value::from(10u64);
    value["envelopes"] = serde_json::Value::Array(envelopes);
    let result: Result<EventQueue<i32>, _> = serde_json::from_value(value);
    assert!(result.is_err(), "next_seq below max must fail to load");
}

#[test]
fn deserialize_rejects_duplicate_seq() {
    let mut queue = EventQueue::new();
    queue.push(tick(0), 1);
    queue.push(tick(0), 2);
    let mut value = serde_json::to_value(&queue).unwrap();
    let mut envelopes = value["envelopes"].as_array().unwrap().clone();
    envelopes[1]["seq"] = envelopes[0]["seq"].clone();
    value["envelopes"] = serde_json::Value::Array(envelopes);
    let result: Result<EventQueue<i32>, _> = serde_json::from_value(value);
    assert!(result.is_err(), "duplicate seq must fail to load");
}

#[test]
fn deserialize_accepts_drained_counter_and_continues() {
    let mut queue = EventQueue::new();
    queue.push(tick(3), 1);
    queue.push(tick(4), 2);
    queue.drain();
    assert!(queue.is_empty());
    let json = serde_json::to_string(&queue).unwrap();
    let mut loaded: EventQueue<i32> = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded, queue);
    loaded.push(tick(5), 3);
    let drained = loaded.drain();
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].seq, 2);
}
