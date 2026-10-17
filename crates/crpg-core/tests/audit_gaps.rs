//! Audit-gap tests (T123). Tests only.

use core::num::NonZeroU32;

use crpg_core::{CoreError, DeterministicRng, EventQueue, Interners, RoundCount, Tick, Ulid};
use proptest::prelude::*;
#[test]
fn round_count_serializes_as_a_bare_number() {
    assert_eq!(serde_json::to_string(&RoundCount::new(7)).unwrap(), "7");
}

#[test]
fn ulid_display_is_exactly_26_chars() {
    for value in [
        0u128,
        1,
        u128::MAX,
        0x8000_0000_0000_0000_0000_0000_0000_0000,
    ] {
        let ulid = Ulid::from_u128(value);
        assert_eq!(ulid.to_string().len(), 26);
    }
}

proptest! {
    #[test]
    fn ulid_display_is_26_chars_arbitrary(value in any::<u128>()) {
        prop_assert_eq!(Ulid::from_u128(value).to_string().len(), 26);
    }
}

#[test]
fn ulid_overflow_rejected_for_first_chars_above_7() {
    let suffix = "0".repeat(25);
    // Crockford alphabet valid chars with value >= 8.
    let high_valid = [
        '8', '9', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'J', 'K', 'M', 'N', 'P', 'Q', 'R', 'S',
        'T', 'V', 'W', 'X', 'Y', 'Z',
    ];
    for c in high_valid {
        let input = format!("{c}{suffix}");
        let err = input.parse::<Ulid>().unwrap_err();
        assert!(matches!(err, CoreError::UlidOverflow), "char {c}: {err:?}");
    }
}

#[test]
fn ulid_serde_rejects_invalid_strings() {
    // wrong length
    assert!(serde_json::from_str::<Ulid>(r#""0000000000000000000000000""#).is_err());
    assert!(serde_json::from_str::<Ulid>(r#""000000000000000000000000000""#).is_err());
    // invalid character
    assert!(serde_json::from_str::<Ulid>(r#""0000000000000U000000000000""#).is_err());
    // overflow
    assert!(serde_json::from_str::<Ulid>(r#""80000000000000000000000000""#).is_err());
}

#[test]
fn rng_bounded_draws_stay_in_bounds() {
    let bounds = [3u32, 5, 9, (1u32 << 31) + 1, u32::MAX];
    for &b in &bounds {
        let bound = NonZeroU32::new(b).unwrap();
        let mut rng = DeterministicRng::from_seed(0xfeed_face_dead_beef);
        for _ in 0..2000 {
            assert!(rng.stream("bounded").gen_range_u32(bound) < b);
        }
    }
}

#[test]
fn interners_round_trip_handles_resolve() {
    let mut interners = Interners::new();
    let stats = ["hp", "ac", "speed"];
    let tags = ["poisoned", "frightened"];
    let stat_handles: Vec<_> = stats.iter().map(|s| interners.intern_stat(s)).collect();
    let tag_handles: Vec<_> = tags.iter().map(|s| interners.intern_tag(s)).collect();
    let json = serde_json::to_string(&interners).unwrap();
    let restored: Interners = serde_json::from_str(&json).unwrap();
    for (handle, value) in stat_handles.iter().zip(stats.iter()) {
        assert_eq!(restored.resolve_stat(*handle), Some(*value));
    }
    for (handle, value) in tag_handles.iter().zip(tags.iter()) {
        assert_eq!(restored.resolve_tag(*handle), Some(*value));
    }
}

// EventQueue saturation tests (leads #19, #80)
#[test]
fn event_queue_accepts_saturated_next_seq() {
    // Saturated case: queued max seq is u64::MAX and next_seq == u64::MAX.
    let value = serde_json::json!({
        "envelopes": [{"tick": 0u64, "seq": u64::MAX, "payload": 1i32}],
        "next_seq": u64::MAX
    });
    let loaded: Result<EventQueue<i32>, _> = serde_json::from_value(value);
    assert!(loaded.is_ok(), "saturated next_seq must load");
}

#[test]
fn event_queue_rejects_non_saturated_next_seq_at_max_minus_one() {
    // next_seq not at saturation and not exceeding queued max -> rejected.
    let value = serde_json::json!({
        "envelopes": [{"tick": 0u64, "seq": u64::MAX, "payload": 1i32}],
        "next_seq": u64::MAX - 1
    });
    let loaded: Result<EventQueue<i32>, _> = serde_json::from_value(value);
    assert!(loaded.is_err(), "non-saturated next_seq <= max must fail");
    // next_seq == queued max (non-MAX) rejected too.
    let value = serde_json::json!({
        "envelopes": [{"tick": 0u64, "seq": 5u64, "payload": 1i32}],
        "next_seq": 5u64
    });
    let loaded: Result<EventQueue<i32>, _> = serde_json::from_value(value);
    assert!(loaded.is_err(), "next_seq == max must fail");
}

#[test]
fn event_queue_push_saturates_near_max() {
    // Push at next_seq MAX-1, then push again should saturate (seq MAX), not wrap.
    let value = serde_json::json!({
        "envelopes": [],
        "next_seq": u64::MAX - 1
    });
    let mut q: EventQueue<i32> = serde_json::from_value(value).unwrap();
    q.push(Tick::new(0), 1);
    q.push(Tick::new(0), 2);
    let d = q.drain();
    // seqs should be MAX-1 and MAX (no wrap to 0)
    assert_eq!(d[0].seq, u64::MAX - 1);
    assert_eq!(d[1].seq, u64::MAX);
    // next push saturates at MAX again
    q.push(Tick::new(0), 3);
    let d2 = q.drain();
    assert_eq!(d2[0].seq, u64::MAX);
}

#[test]
fn event_queue_push_at_max_stays() {
    let value = serde_json::json!({
        "envelopes": [{"tick": 0u64, "seq": u64::MAX, "payload": 1i32}],
        "next_seq": u64::MAX
    });
    let mut q: EventQueue<i32> = serde_json::from_value(value).unwrap();
    q.push(Tick::new(0), 2);
    let d = q.drain();
    // find pushed one (seq MAX)
    assert!(d.iter().any(|e| e.seq == u64::MAX && e.payload == 2));
    // no wrap
    assert!(!d.iter().any(|e| e.seq == 0 && e.payload == 2));
}
