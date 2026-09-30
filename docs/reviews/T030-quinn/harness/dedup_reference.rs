//! Independent reference-model regression harness for `Dedup` (crpg T030).
//!
//! The model is an ordered set of received packet numbers plus the documented
//! window contract, written without any bit arithmetic:
//!
//! - `insert(p)` returns "might be a duplicate": false for a packet newer than
//!   every received one; for a packet within `window` of the highest (age =
//!   highest - p < window) true exactly when it was already received; true for
//!   anything older (left of the window).
//! - `smallest_missing_in_interval(lower, upper)` for received `lower <= upper`
//!   is the smallest never-received `p` with `lower < p < upper` whose age is
//!   at most `window - 1`; older packets count as received.
//!
//! Tests prefixed `control_` stay within ages <= 128, where the original
//! 129-packet window and the 2049-packet target agree: they must pass on both
//! the unpatched and the patched source. Tests prefixed `target_` exercise the
//! 2049-packet target (2048 history bits plus the separately stored highest
//! packet) and are expected to FAIL on unpatched quinn-proto 0.11.19.
//!
//! Hooked into `src/connection/spaces.rs` as a child test module so it can
//! reach the private `Dedup` API; it imports nothing else from the crate.

use std::collections::BTreeSet;

use super::Dedup;

/// Effective window targeted by the patch: highest + 2048 history packets.
const TARGET_WINDOW: u64 = 2049;

struct Model {
    received: BTreeSet<u64>,
    window: u64,
}

impl Model {
    fn new(window: u64) -> Self {
        Self {
            received: BTreeSet::new(),
            window,
        }
    }

    fn highest(&self) -> Option<u64> {
        self.received.last().copied()
    }

    fn insert(&mut self, packet: u64) -> bool {
        match self.highest() {
            None => {
                self.received.insert(packet);
                false
            }
            Some(highest) if packet > highest => {
                self.received.insert(packet);
                false
            }
            Some(highest) if highest - packet < self.window => !self.received.insert(packet),
            Some(_) => true,
        }
    }

    fn smallest_missing_in_interval(&self, lower: u64, upper: u64) -> Option<u64> {
        let highest = self.highest().expect("queries need a received packet");
        (lower.saturating_add(1)..upper)
            .find(|packet| !self.received.contains(packet) && highest - packet < self.window)
    }
}

/// Deterministic SplitMix64: reproducible schedules without dependencies.
struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

/// Drives both implementation and model, asserting every insert agrees.
struct Pair {
    dedup: Dedup,
    model: Model,
}

impl Pair {
    fn new(window: u64) -> Self {
        Self {
            dedup: Dedup::new(),
            model: Model::new(window),
        }
    }

    fn insert(&mut self, packet: u64) {
        let expected = self.model.insert(packet);
        let actual = self.dedup.insert(packet);
        assert_eq!(
            actual,
            expected,
            "insert({packet}) with highest {:?}: implementation says duplicate={actual}, model says {expected}",
            self.model.highest(),
        );
    }

    /// Compares missing-packet queries for every received (lower, upper) pair
    /// drawn from `bounds`.
    fn check_queries(&self, bounds: &[u64]) {
        for &lower in bounds {
            for &upper in bounds {
                if lower > upper
                    || !self.model.received.contains(&lower)
                    || !self.model.received.contains(&upper)
                {
                    continue;
                }
                let expected = self.model.smallest_missing_in_interval(lower, upper);
                assert_eq!(
                    self.dedup.smallest_missing_in_interval(lower, upper),
                    expected,
                    "smallest_missing_in_interval({lower}, {upper}) with highest {:?}",
                    self.model.highest(),
                );
                assert_eq!(
                    self.dedup.missing_in_interval(lower, upper),
                    expected.is_some(),
                    "missing_in_interval({lower}, {upper})",
                );
            }
        }
    }

    /// Every received packet within `span` of the highest, plus the highest.
    fn recent_received(&self, span: u64) -> Vec<u64> {
        let highest = self.model.highest().expect("received");
        self.model
            .received
            .range(highest.saturating_sub(span)..=highest)
            .copied()
            .collect()
    }
}

#[test]
fn control_empty_first_and_identical_duplicate() {
    for first in [0, 1, 1_000, (1 << 62) - 1] {
        let mut pair = Pair::new(TARGET_WINDOW);
        pair.insert(first);
        pair.insert(first);
        if first >= 5 {
            pair.insert(first - 5);
            pair.insert(first - 5);
        }
    }
}

#[test]
fn control_reordering_within_original_window() {
    let mut pair = Pair::new(TARGET_WINDOW);
    for packet in (0..100).step_by(2) {
        pair.insert(packet);
    }
    pair.insert(300);
    for packet in (173..300).rev() {
        pair.insert(packet);
    }
    for packet in [300, 250, 173] {
        pair.insert(packet);
    }
    pair.check_queries(&pair.recent_received(128));
}

#[test]
fn control_ages_127_and_128() {
    for age in [127, 128] {
        let mut pair = Pair::new(TARGET_WINDOW);
        pair.insert(5_000);
        pair.insert(5_000 - age);
        pair.insert(5_000 - age);
    }
}

#[test]
fn target_age_129_is_inside_the_window() {
    // Age 129 is the first age the original 129-packet window discards.
    let mut pair = Pair::new(TARGET_WINDOW);
    pair.insert(5_000);
    pair.insert(5_000 - 129);
}

#[test]
fn target_ages_2047_2048_2049() {
    for age in [2047, 2048, 2049] {
        let mut pair = Pair::new(TARGET_WINDOW);
        pair.insert(10_000);
        pair.insert(10_000 - age);
        pair.insert(10_000 - age);
    }
}

#[test]
fn target_advance_by_zero_one_word_window_and_beyond() {
    for advance in [0, 1, 63, 64, 65, 127, 128, 129, 2047, 2048, 2049, 2050, 5_000] {
        let mut pair = Pair::new(TARGET_WINDOW);
        for packet in 100..160 {
            pair.insert(packet);
        }
        pair.insert(158 + advance);
        for age in [1, 2, 63, 64, 65, 128, 129, 2047, 2048, 2049, 2050] {
            if let Some(packet) = (158 + advance).checked_sub(age) {
                pair.insert(packet);
            }
        }
        pair.check_queries(&pair.recent_received(TARGET_WINDOW + 10));
    }
}

#[test]
fn target_packet_number_range_limits() {
    let top = (1u64 << 62) - 1;
    let mut pair = Pair::new(TARGET_WINDOW);
    pair.insert(top - 3_000);
    pair.insert(top);
    for age in [1, 129, 2048, 2049, 3_000] {
        pair.insert(top - age);
    }
    pair.check_queries(&pair.recent_received(3_100));
}

#[test]
fn target_missing_queries_whole_partial_full_and_disjoint() {
    let mut pair = Pair::new(TARGET_WINDOW);
    // Full run, a wholly missing stretch, partial holes and disjoint holes,
    // spread across more than one 64-bit word and past bit 128.
    for packet in 0..=400 {
        pair.insert(packet);
    }
    for packet in (600..=900).filter(|p| p % 7 != 0) {
        pair.insert(packet);
    }
    for packet in [1_000, 1_001, 1_130, 1_131, 1_700, 2_300] {
        pair.insert(packet);
    }
    let mut bounds = pair.recent_received(TARGET_WINDOW);
    bounds.retain(|p| p % 3 == 0 || *p >= 1_000 || *p == 400 || *p == 601);
    pair.check_queries(&bounds);
}

#[test]
fn target_duplicates_after_eviction() {
    let mut pair = Pair::new(TARGET_WINDOW);
    pair.insert(10);
    pair.insert(11);
    pair.insert(11 + 2_049);
    // Packet 10 has aged out: it is treated as received (duplicate) again.
    pair.insert(10);
    pair.insert(11);
    // Still-tracked older packets remain accurate.
    pair.insert(12);
    pair.insert(12);
}

#[test]
fn target_generated_schedules_fixed_seed() {
    for seed in [1u64, 42, 0xC0FFEE, 2710] {
        let mut rng = SplitMix(seed);
        let mut pair = Pair::new(TARGET_WINDOW);
        let mut next = rng.below(1_000);
        let mut pending: Vec<u64> = Vec::new();
        for step in 0..6_000 {
            match rng.below(10) {
                // Deliver the next packet in order, sometimes skipping (loss or delay).
                0..=5 => {
                    if rng.below(8) == 0 {
                        pending.push(next);
                    } else {
                        pair.insert(next);
                    }
                    next += 1 + rng.below(3);
                }
                // Deliver a delayed packet late, possibly far behind (reordering up to ~3000).
                6..=8 => {
                    if !pending.is_empty() {
                        let index = rng.below(pending.len() as u64) as usize;
                        let packet = pending.swap_remove(index);
                        pair.insert(packet);
                    }
                }
                // Replay something recent or ancient (duplicate).
                _ => {
                    if let Some(highest) = pair.model.highest() {
                        pair.insert(highest.saturating_sub(rng.below(3_000)));
                    }
                }
            }
            if step % 500 == 499 {
                let mut bounds = pair.recent_received(TARGET_WINDOW + 50);
                let stride = (bounds.len() / 40).max(1);
                bounds = bounds.into_iter().step_by(stride).collect();
                pair.check_queries(&bounds);
            }
        }
    }
}

#[test]
fn control_generated_schedules_shallow_reordering() {
    // Same generator shape, reordering depth capped at 100: both windows agree.
    for seed in [7u64, 99, 2711] {
        let mut rng = SplitMix(seed);
        let mut pair = Pair::new(TARGET_WINDOW);
        let mut next = 0;
        let mut pending: Vec<u64> = Vec::new();
        for _ in 0..4_000 {
            if rng.below(3) == 0 {
                pending.push(next);
            } else {
                pair.insert(next);
            }
            next += 1;
            // Anything delayed by 100 or more is lost, never delivered.
            pending.retain(|packet| next - packet < 100);
            if !pending.is_empty() && rng.below(2) == 0 {
                let index = rng.below(pending.len() as u64) as usize;
                let packet = pending.swap_remove(index);
                pair.insert(packet);
            }
        }
        pair.check_queries(&pair.recent_received(120));
    }
}

#[test]
fn measure_layout() {
    // Layout count only (no heap allocation is involved in either version).
    eprintln!(
        "T030_LAYOUT size_of::<Dedup>()={} align_of::<Dedup>()={}",
        std::mem::size_of::<Dedup>(),
        std::mem::align_of::<Dedup>()
    );
}
