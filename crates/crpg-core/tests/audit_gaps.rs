//! Tests closing the audit's untested paths in `crpg-core` (T123).

use core::num::NonZeroU32;

use crpg_core::{CoreError, DeterministicRng, EventQueue, Interners, RoundCount, Tick, Ulid};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Lead #19 / #80 — EventQueue deserialization and push near u64::MAX
// ---------------------------------------------------------------------------

#[test]
fn event_deserialize_accepts_saturated_next_seq() {
    // A queue whose counter has saturated at u64::MAX (for example after a
    // push at u64::MAX) must still load, and a further push saturates.
    let mut queue: EventQueue<i32> =
        serde_json::from_value(serde_json::json!({"envelopes": [], "next_seq": u64::MAX})).unwrap();
    assert!(queue.is_empty());
    queue.push(Tick::new(1), 42);
    let drained = queue.drain();
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].seq, u64::MAX);
}

#[test]
fn event_deserialize_rejects_stale_next_seq_at_max() {
    // With a queued envelope at u64::MAX, a counter below MAX is stale and
    // must be rejected (saturation is the only way to represent "next").
    let value = serde_json::json!({
        "envelopes": [{"tick": 0, "seq": u64::MAX, "payload": 1}],
        "next_seq": u64::MAX - 1
    });
    let result: Result<EventQueue<i32>, _> = serde_json::from_value(value);
    assert!(result.is_err(), "stale next_seq at max must fail to load");
}

#[test]
fn event_push_near_u64_max_saturates() {
    let mut queue: EventQueue<i32> =
        serde_json::from_value(serde_json::json!({"envelopes": [], "next_seq": u64::MAX - 1}))
            .unwrap();
    queue.push(Tick::ZERO, 1);
    queue.push(Tick::ZERO, 2);
    let seqs: Vec<u64> = queue.drain().into_iter().map(|e| e.seq).collect();
    assert_eq!(seqs, vec![u64::MAX - 1, u64::MAX]);
}

// ---------------------------------------------------------------------------
// Lead #20 — RoundCount serializes as a bare integer, like Tick
// ---------------------------------------------------------------------------

#[test]
fn round_count_serializes_as_bare_integer() {
    assert_eq!(serde_json::to_string(&RoundCount::new(7)).unwrap(), "7");
}

// ---------------------------------------------------------------------------
// Lead #21 — Interners round-trips through serde preserving handle resolution
// ---------------------------------------------------------------------------

#[test]
fn interners_serde_round_trip_preserves_handle_resolution() {
    let mut interners = Interners::new();
    let stat = interners.intern_stat("hp");
    let tag = interners.intern_tag("foo");

    let json = serde_json::to_string(&interners).unwrap();
    let restored: Interners = serde_json::from_str(&json).unwrap();

    assert_eq!(restored, interners);
    assert_eq!(restored.resolve_stat(stat), Some("hp"));
    assert_eq!(restored.resolve_tag(tag), Some("foo"));
}

// ---------------------------------------------------------------------------
// Lead #22 — bounded draws stay in bounds and pin rejection sampling
// ---------------------------------------------------------------------------

#[test]
fn rng_bounded_draws_stay_in_bounds() {
    // In-bounds across bounds that exercise rejection sampling.
    for &bound in &[3u32, 5u32, 7u32, (1u32 << 31) + 1, u32::MAX] {
        let bound = NonZeroU32::new(bound).unwrap();
        let mut rng = DeterministicRng::from_seed(0xfeed_face_dead_beef);
        rng.stream("range");
        for _ in 0..200 {
            assert!(rng.stream("range").gen_range_u32(bound) < bound.get());
        }
    }

    // Pin the documented rejection rule for a bound where rejection is frequent.
    let bound = NonZeroU32::new((1u32 << 31) + 1).unwrap();
    let mut rng = DeterministicRng::from_seed(0x0123_4567_89ab_cdef);
    let mut raw_rng = rng.clone();
    rng.stream("range");
    raw_rng.stream("range");

    // threshold = (-bound) mod bound; raw draws below it are rejected.
    let threshold = bound.get().wrapping_neg() % bound.get();
    let mut rejected = 0usize;
    for _ in 0..100 {
        let expected = loop {
            let raw = raw_rng.stream("range").next_u32();
            if raw < threshold {
                rejected += 1;
            } else {
                break raw % bound.get();
            }
        };
        let actual = rng.stream("range").gen_range_u32(bound);
        assert_eq!(actual, expected);
    }
    assert!(rejected > 0, "rejection branch was not exercised");
    // Boundary values (threshold-1 rejected, threshold accepted) are not pinned:
    // no public API forces a raw draw to a chosen value without serde
    // construction, which is out of scope for these tests.
}

// ---------------------------------------------------------------------------
// Lead #81 — ULID overflow rejection for every high first character
// ---------------------------------------------------------------------------

#[test]
fn ulid_overflow_rejected_for_every_high_first_char() {
    // Crockford base32 digits above 7 that are valid first characters.
    let high = "89ABCDEFGHJKMNPQRSTVWXYZ";
    for c in high.chars() {
        let input = format!("{}{}", c, "0".repeat(25));
        let error = input.parse::<Ulid>().unwrap_err();
        assert!(
            matches!(error, CoreError::UlidOverflow),
            "char {} produced {:?}",
            c,
            error
        );
    }
}

// ---------------------------------------------------------------------------
// Lead #85 — Display of arbitrary ULIDs is exactly 26 characters
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn ulid_display_is_exactly_26_chars(value in any::<u128>()) {
        let ulid = Ulid::from_u128(value);
        prop_assert_eq!(ulid.to_string().len(), 26);
    }
}

// ---------------------------------------------------------------------------
// Lead #86 — ULID serde rejects invalid strings
// ---------------------------------------------------------------------------

#[test]
fn ulid_serde_rejects_invalid_strings() {
    let cases = [
        "\"\"",                            // wrong length (empty)
        "\"0000000000000000000000000\"",   // wrong length (25)
        "\"000000000000000000000000000\"", // wrong length (27)
        "\"0000000000000U000000000000\"",  // invalid character
        "\"80000000000000000000000000\"",  // overflow
    ];
    for input in cases {
        let result = serde_json::from_str::<Ulid>(input);
        assert!(result.is_err(), "expected rejection for {}", input);
    }
}
