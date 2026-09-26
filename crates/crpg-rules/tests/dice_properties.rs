//! Dice evaluation properties over arbitrary valid expressions.
//!
//! Integer-only throughout: bounded faces, kept counts, trace arithmetic,
//! seed/stream determinism, and stream independence.

use crpg_core::DeterministicRng;
use crpg_rules::{DiceExpr, DiceSelection};
use proptest::prelude::*;

/// Offsets biased toward saturation edges.
fn edge_offset() -> impl Strategy<Value = i32> {
    prop_oneof![Just(i32::MIN), Just(i32::MAX), -100..100i32, any::<i32>(),]
}

fn selection_strategy(count: u32) -> impl Strategy<Value = DiceSelection> {
    prop_oneof![
        Just(DiceSelection::All),
        (1..=count).prop_map(DiceSelection::KeepHighest),
        (1..=count).prop_map(DiceSelection::KeepLowest),
        (0..count).prop_map(DiceSelection::DropHighest),
        (0..count).prop_map(DiceSelection::DropLowest),
    ]
}

/// Arbitrary valid expressions with small groups and wide side coverage.
fn expr_strategy() -> impl Strategy<Value = DiceExpr> {
    (
        1..=6u32,
        prop_oneof![
            Just(1u32),
            Just(2),
            Just(4),
            Just(6),
            Just(8),
            Just(12),
            Just(100),
            Just(1_000_000)
        ],
        edge_offset(),
    )
        .prop_flat_map(|(count, sides, offset)| {
            (
                Just(count),
                Just(sides),
                selection_strategy(count),
                Just(offset),
            )
        })
        .prop_map(|(count, sides, selection, offset)| {
            DiceExpr::new(count, sides, selection, offset).expect("generated expression is valid")
        })
}

fn stream_strategy() -> impl Strategy<Value = String> {
    prop_oneof![Just("prop".to_owned()), "[a-z]{1,16}".prop_map(|name| name),]
}

fn expected_kept(count: u32, selection: DiceSelection) -> usize {
    match selection {
        DiceSelection::All => count as usize,
        DiceSelection::KeepHighest(kept) | DiceSelection::KeepLowest(kept) => kept as usize,
        DiceSelection::DropHighest(dropped) | DiceSelection::DropLowest(dropped) => {
            count as usize - dropped as usize
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, ..ProptestConfig::default() })]

    #[test]
    fn traces_are_bounded_with_matching_totals(expr in expr_strategy(), seed in any::<u64>(), stream in stream_strategy()) {
        let mut rng = DeterministicRng::from_seed(seed);
        let roll = expr.evaluate(&mut rng, &stream).expect("generated inputs evaluate");
        prop_assert_eq!(&roll.expression, &expr);
        prop_assert_eq!(roll.dice.len(), expr.count() as usize);
        for die in &roll.dice {
            prop_assert!((1..=expr.sides()).contains(&die.value), "face {} out of bounds", die.value);
        }
        let kept: Vec<&crpg_rules::DieResult> = roll.dice.iter().filter(|die| die.kept).collect();
        prop_assert_eq!(kept.len(), expected_kept(expr.count(), expr.selection()));
        let mut sum = i64::from(expr.offset());
        for die in &kept {
            sum += i64::from(die.value);
        }
        prop_assert_eq!(roll.total, sum.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
        prop_assert_eq!(&roll.expression.to_string().parse::<DiceExpr>().unwrap(), &expr);
    }

    #[test]
    fn equal_seed_stream_and_input_give_equal_traces(expr in expr_strategy(), seed in any::<u64>(), stream in stream_strategy()) {
        let mut first = DeterministicRng::from_seed(seed);
        let mut second = DeterministicRng::from_seed(seed);
        let left = expr.evaluate(&mut first, &stream).expect("first evaluates");
        let right = expr.evaluate(&mut second, &stream).expect("second evaluates");
        prop_assert_eq!(&left, &right);
        prop_assert_eq!(&first, &second, "post-draw RNG states match");
    }

    #[test]
    fn unrelated_streams_do_not_interfere(
        expr in expr_strategy(),
        seed in any::<u64>(),
        streams in (stream_strategy(), stream_strategy()).prop_filter("distinct", |(left, right)| left != right),
    ) {
        let (first_name, second_name) = streams;
        let mut ordered = DeterministicRng::from_seed(seed);
        let first_a = expr.evaluate(&mut ordered, &first_name).expect("ordered first");
        let first_b = expr.evaluate(&mut ordered, &second_name).expect("ordered second");
        let mut swapped = DeterministicRng::from_seed(seed);
        let second_b = expr.evaluate(&mut swapped, &second_name).expect("swapped first");
        let second_a = expr.evaluate(&mut swapped, &first_name).expect("swapped second");
        prop_assert_eq!(&first_a, &second_a);
        prop_assert_eq!(&first_b, &second_b);
    }
}
