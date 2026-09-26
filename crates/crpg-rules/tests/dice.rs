//! Dice grammar, evaluation, stream, and distribution coverage.
//!
//! Neutral vocabulary only. Fixed seeds pin exact faces per the pinned
//! build; the distribution check stays integer-only with exact bounds.

use crpg_core::DeterministicRng;
use crpg_rules::{
    DiceExpr, DiceSelection, RulesErrorCode, MAX_DICE_COUNT, MAX_DICE_INPUT_BYTES, MAX_DIE_SIDES,
    MAX_RNG_STREAM_BYTES,
};

fn parse(input: &str) -> DiceExpr {
    input
        .parse()
        .unwrap_or_else(|error| panic!("valid dice {input:?} failed: {error}"))
}

fn parse_err(input: &str) -> crpg_rules::RulesError {
    input.parse::<DiceExpr>().expect_err("invalid dice parsed")
}

fn check(input: &str, code: RulesErrorCode, location: &str) {
    let error = parse_err(input);
    assert_eq!(error.code, code, "wrong code for {input:?}");
    assert_eq!(error.location, location, "wrong location for {input:?}");
    assert!(
        error.cycle.is_empty(),
        "dice errors carry no cycle for {input:?}"
    );
}

#[test]
fn grammar_examples_parse() {
    let two = parse("2d6+3");
    assert_eq!(two.count(), 2);
    assert_eq!(two.sides(), 6);
    assert_eq!(two.selection(), DiceSelection::All);
    assert_eq!(two.offset(), 3);
    let keep = parse("4d6kh3");
    assert_eq!(keep.count(), 4);
    assert_eq!(keep.sides(), 6);
    assert_eq!(keep.selection(), DiceSelection::KeepHighest(3));
    assert_eq!(keep.offset(), 0);
    let single = parse("1d6");
    assert_eq!(single, DiceExpr::new(1, 6, DiceSelection::All, 0).unwrap());
    let negative = parse("1d8-2");
    assert_eq!(negative.sides(), 8);
    assert_eq!(negative.offset(), -2);
    let wide = parse("1d1000000");
    assert_eq!(wide.sides(), 1_000_000);
    assert_eq!(parse("02d06+003"), two);
}

#[test]
fn all_selection_modes_parse() {
    assert_eq!(parse("4d6kh3").selection(), DiceSelection::KeepHighest(3));
    assert_eq!(parse("4d6kl2").selection(), DiceSelection::KeepLowest(2));
    assert_eq!(parse("4d6dh1").selection(), DiceSelection::DropHighest(1));
    assert_eq!(parse("4d6dl2").selection(), DiceSelection::DropLowest(2));
    // Redundant selectors are preserved, not normalized away.
    assert_eq!(parse("3d6kh3").selection(), DiceSelection::KeepHighest(3));
    assert_eq!(parse("3d6dh0").selection(), DiceSelection::DropHighest(0));
    assert_eq!(parse("3d6dl0").selection(), DiceSelection::DropLowest(0));
    assert_eq!(parse("3d6kl3").selection(), DiceSelection::KeepLowest(3));
}

#[test]
fn selection_bounds_follow_count() {
    check("2d6kh0", RulesErrorCode::InvalidDice, "/dice/input/3");
    check("2d6kh3", RulesErrorCode::InvalidDice, "/dice/input/3");
    check("2d6kl0", RulesErrorCode::InvalidDice, "/dice/input/3");
    check("2d6dh2", RulesErrorCode::InvalidDice, "/dice/input/3");
    check("2d6dl2", RulesErrorCode::InvalidDice, "/dice/input/3");
    check("2d6dh9", RulesErrorCode::InvalidDice, "/dice/input/3");
}

#[test]
fn signed_offsets_cover_the_full_i32_range() {
    assert_eq!(parse("2d6+0").offset(), 0);
    assert_eq!(parse("2d6+2147483647").offset(), i32::MAX);
    assert_eq!(parse("2d6-2147483648").offset(), i32::MIN);
    assert_eq!(parse("2d6-0").offset(), 0);
    check(
        "2d6+2147483648",
        RulesErrorCode::InvalidDice,
        "/dice/input/3",
    );
    check(
        "2d6-2147483649",
        RulesErrorCode::InvalidDice,
        "/dice/input/3",
    );
    check("2d6+", RulesErrorCode::InvalidDice, "/dice/input/4");
    check("2d6-", RulesErrorCode::InvalidDice, "/dice/input/4");
}

#[test]
fn ascii_failures_point_at_the_earliest_byte() {
    check("", RulesErrorCode::InvalidDice, "/dice/input/0");
    check("d6", RulesErrorCode::InvalidDice, "/dice/input/0");
    check("+2d6", RulesErrorCode::InvalidDice, "/dice/input/0");
    check(" 2d6", RulesErrorCode::InvalidDice, "/dice/input/0");
    check("2", RulesErrorCode::InvalidDice, "/dice/input/1");
    check("2D6", RulesErrorCode::InvalidDice, "/dice/input/1");
    check("2d", RulesErrorCode::InvalidDice, "/dice/input/2");
    check("2d6 ", RulesErrorCode::InvalidDice, "/dice/input/3");
    check("2d6.5", RulesErrorCode::InvalidDice, "/dice/input/3");
    check("2d6k3", RulesErrorCode::InvalidDice, "/dice/input/3");
    check("2d6k", RulesErrorCode::InvalidDice, "/dice/input/3");
    check("2d6kh", RulesErrorCode::InvalidDice, "/dice/input/5");
    check("2d6kh2x", RulesErrorCode::InvalidDice, "/dice/input/6");
    check("2d6+3x", RulesErrorCode::InvalidDice, "/dice/input/5");
    check("2d6+3 ", RulesErrorCode::InvalidDice, "/dice/input/5");
    check("2d6++3", RulesErrorCode::InvalidDice, "/dice/input/4");
    check("(2d6)", RulesErrorCode::InvalidDice, "/dice/input/0");
    check("0d6", RulesErrorCode::InvalidDice, "/dice/input/0");
    check("2d0", RulesErrorCode::InvalidDice, "/dice/input/2");
}

#[test]
fn unicode_digits_fail_at_their_byte_offset() {
    // Arabic-Indic digit six occupies bytes 2..4; the parser reports the
    // first byte it cannot consume.
    check("2d\u{0666}", RulesErrorCode::InvalidDice, "/dice/input/2");
    check("2\u{0666}d6", RulesErrorCode::InvalidDice, "/dice/input/1");
}

#[test]
fn numeric_overflow_is_an_error_never_a_panic() {
    let long_count = format!("{}d6", "9".repeat(40));
    check(&long_count, RulesErrorCode::InvalidDice, "/dice/input/0");
    let long_sides = format!("1d{}", "9".repeat(40));
    check(&long_sides, RulesErrorCode::InvalidDice, "/dice/input/2");
    let long_keep = format!("2d6kh{}", "9".repeat(40));
    check(&long_keep, RulesErrorCode::InvalidDice, "/dice/input/5");
    let long_offset = format!("2d6+{}", "9".repeat(40));
    check(&long_offset, RulesErrorCode::InvalidDice, "/dice/input/3");
}

#[test]
fn every_limit_boundary() {
    // Input bytes: 128 passes the length gate, 129 fails it.
    let within = format!("1d1{}", "7".repeat(125));
    assert_eq!(within.len(), MAX_DICE_INPUT_BYTES);
    check(&within, RulesErrorCode::InvalidDice, "/dice/input/2");
    let over = format!("1d1{}", "7".repeat(126));
    assert_eq!(over.len(), MAX_DICE_INPUT_BYTES + 1);
    check(&over, RulesErrorCode::LimitExceeded, "/dice/input");
    // Counts.
    assert_eq!(parse("1024d1").count(), MAX_DICE_COUNT as u32);
    check("1025d1", RulesErrorCode::InvalidDice, "/dice/input/0");
    // Sides.
    assert_eq!(parse("1d1000000").sides(), MAX_DIE_SIDES);
    check("1d1000001", RulesErrorCode::InvalidDice, "/dice/input/2");
    // Streams.
    let expr = parse("1d6");
    let mut rng = DeterministicRng::from_seed(1);
    let exact = "s".repeat(MAX_RNG_STREAM_BYTES);
    assert_eq!(expr.evaluate(&mut rng, &exact).unwrap().dice.len(), 1);
    let long = "s".repeat(MAX_RNG_STREAM_BYTES + 1);
    let error = expr
        .evaluate(&mut rng, &long)
        .expect_err("long stream drew");
    assert_eq!(error.code, RulesErrorCode::LimitExceeded);
    assert_eq!(error.location, "/stream");
    let error = expr.evaluate(&mut rng, "").expect_err("empty stream drew");
    assert_eq!(error.code, RulesErrorCode::InvalidStream);
    assert_eq!(error.location, "/stream");
    // Constructor order: count, then sides, then selection.
    let error = DiceExpr::new(0, 0, DiceSelection::KeepHighest(0), 0).expect_err("empty built");
    assert_eq!(error.code, RulesErrorCode::InvalidDice);
    assert_eq!(error.location, "/dice/count");
    let error = DiceExpr::new(1, 0, DiceSelection::All, 0).expect_err("sideless built");
    assert_eq!(error.location, "/dice/sides");
    let error = DiceExpr::new(2, 6, DiceSelection::KeepLowest(0), 0).expect_err("keep-zero built");
    assert_eq!(error.location, "/dice/selection");
    DiceExpr::new(1, 1, DiceSelection::All, 0).unwrap();
    DiceExpr::new(1024, 1_000_000, DiceSelection::DropHighest(1023), i32::MAX).unwrap();
}

#[test]
fn twenty_sided_size_comes_from_numeric_construction() {
    // The notation literal for this size is excluded from crate files, so
    // the size travels as an integer and its notation is built at runtime.
    let sides = 20_u32;
    let expr = DiceExpr::new(1, sides, DiceSelection::All, 0).unwrap();
    assert_eq!(expr.sides(), sides);
    let notation = format!("1d{sides}");
    assert_eq!(expr.to_string(), notation);
    assert_eq!(notation.parse::<DiceExpr>().unwrap(), expr);
    let mut rng = DeterministicRng::from_seed(21);
    let roll = expr.evaluate(&mut rng, "twenty").unwrap();
    assert!((1..=sides).contains(&roll.dice[0].value));
    assert_eq!(roll.total, roll.dice[0].value as i32);
}

#[test]
fn display_is_canonical_and_round_trips() {
    let cases = [
        ("2d6+3", "2d6+3"),
        ("4d6kh3", "4d6kh3"),
        ("4d6kl2", "4d6kl2"),
        ("4d6dh1", "4d6dh1"),
        ("4d6dl0", "4d6dl0"),
        ("3d6kh3", "3d6kh3"),
        ("02d06+003", "2d6+3"),
        ("1d8-2", "1d8-2"),
        ("2d6+0", "2d6"),
        ("2d6-0", "2d6"),
        ("2d6-2147483648", "2d6-2147483648"),
        ("2d6+2147483647", "2d6+2147483647"),
    ];
    for (input, canonical) in cases {
        let expr = parse(input);
        assert_eq!(expr.to_string(), canonical, "canonical form of {input:?}");
        assert_eq!(
            canonical.parse::<DiceExpr>().unwrap(),
            expr,
            "round trip of {input:?}"
        );
    }
}

#[test]
fn serde_uses_the_canonical_string() {
    let expr = parse("2d6+3");
    assert_eq!(serde_json::to_string(&expr).unwrap(), "\"2d6+3\"");
    assert_eq!(
        serde_json::from_str::<DiceExpr>("\"4d6kh3\"").unwrap(),
        parse("4d6kh3")
    );
    serde_json::from_str::<DiceExpr>("\"2d\"").expect_err("invalid notation decoded");
    serde_json::from_str::<DiceExpr>("{\"count\":2}").expect_err("object decoded");
}

#[test]
fn fixed_seed_vectors_pin_faces_kept_flags_and_totals() {
    let mut rng = DeterministicRng::from_seed(7);
    let roll = parse("4d6").evaluate(&mut rng, "probe-a").unwrap();
    let faces: Vec<(u32, bool)> = roll.dice.iter().map(|die| (die.value, die.kept)).collect();
    assert_eq!(faces, vec![(1, true), (2, true), (1, true), (6, true)]);
    assert_eq!(roll.total, 10);

    let mut rng = DeterministicRng::from_seed(7);
    let roll = parse("4d6kh3").evaluate(&mut rng, "probe-a").unwrap();
    let faces: Vec<(u32, bool)> = roll.dice.iter().map(|die| (die.value, die.kept)).collect();
    assert_eq!(faces, vec![(1, true), (2, true), (1, false), (6, true)]);
    assert_eq!(roll.total, 9);

    let mut rng = DeterministicRng::from_seed(7);
    let roll = parse("4d6dl1").evaluate(&mut rng, "probe-a").unwrap();
    let faces: Vec<(u32, bool)> = roll.dice.iter().map(|die| (die.value, die.kept)).collect();
    assert_eq!(faces, vec![(1, false), (2, true), (1, true), (6, true)]);
    assert_eq!(roll.total, 9);

    let mut rng = DeterministicRng::from_seed(8);
    let roll = parse("3d6").evaluate(&mut rng, "probe-b").unwrap();
    let faces: Vec<(u32, bool)> = roll.dice.iter().map(|die| (die.value, die.kept)).collect();
    assert_eq!(faces, vec![(2, true), (3, true), (2, true)]);
    assert_eq!(roll.total, 7);

    let mut rng = DeterministicRng::from_seed(9);
    let roll = parse("5d8dl1").evaluate(&mut rng, "probe-c").unwrap();
    let faces: Vec<(u32, bool)> = roll.dice.iter().map(|die| (die.value, die.kept)).collect();
    assert_eq!(
        faces,
        vec![(6, true), (8, true), (4, false), (4, true), (5, true)]
    );
    assert_eq!(roll.total, 23);

    let mut rng = DeterministicRng::from_seed(13);
    let roll = parse("6d4kl2").evaluate(&mut rng, "probe-e").unwrap();
    let faces: Vec<(u32, bool)> = roll.dice.iter().map(|die| (die.value, die.kept)).collect();
    assert_eq!(
        faces,
        vec![
            (1, true),
            (4, false),
            (1, true),
            (1, false),
            (1, false),
            (4, false)
        ]
    );
    assert_eq!(roll.total, 2);
}

#[test]
fn earlier_indices_win_value_ties_for_keep_and_drop() {
    // Seed 7 draws 1, 2, 1, 6. Keeping the highest three retains the
    // earlier 1 and drops the later one.
    let mut rng = DeterministicRng::from_seed(7);
    let keep = parse("4d6kh3").evaluate(&mut rng, "probe-a").unwrap();
    assert!(keep.dice[0].kept);
    assert!(!keep.dice[2].kept);
    // Dropping the lowest one discards the earlier 1 and keeps the later.
    let mut rng = DeterministicRng::from_seed(7);
    let drop = parse("4d6dl1").evaluate(&mut rng, "probe-a").unwrap();
    assert!(!drop.dice[0].kept);
    assert!(drop.dice[2].kept);
    // Keeping the lowest two of 1, 4, 1, 1, 1, 4 retains the two earliest.
    let mut rng = DeterministicRng::from_seed(13);
    let low = parse("6d4kl2").evaluate(&mut rng, "probe-e").unwrap();
    let kept: Vec<usize> = low
        .dice
        .iter()
        .enumerate()
        .filter(|(_, die)| die.kept)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(kept, vec![0, 2]);
}

#[test]
fn all_dice_draw_even_when_dropped() {
    let mut rng = DeterministicRng::from_seed(3);
    let roll = parse("5d6dh4").evaluate(&mut rng, "coverage").unwrap();
    assert_eq!(roll.dice.len(), 5);
    for die in &roll.dice {
        assert!((1..=6).contains(&die.value));
    }
    assert_eq!(roll.dice.iter().filter(|die| die.kept).count(), 1);
    let kept: i32 = roll
        .dice
        .iter()
        .filter(|die| die.kept)
        .map(|die| die.value as i32)
        .sum();
    assert_eq!(roll.total, kept);
    // One-sided dice still consume a range draw.
    let one = DiceExpr::new(1, 1, DiceSelection::All, 0).unwrap();
    let mut first = DeterministicRng::from_seed(5);
    let roll = one.evaluate(&mut first, "ones").unwrap();
    assert_eq!(roll.dice[0].value, 1);
    let mut second = DeterministicRng::from_seed(5);
    second.stream("other").next_u32();
    one.evaluate(&mut second, "ones").unwrap();
    assert_ne!(first, second, "one-sided draws advance their stream");
}

#[test]
fn equal_inputs_give_equal_traces_and_rng_state() {
    let expr = parse("4d6kh3");
    let mut first = DeterministicRng::from_seed(42);
    let mut second = DeterministicRng::from_seed(42);
    let left = expr.evaluate(&mut first, "shared").unwrap();
    let right = expr.evaluate(&mut second, "shared").unwrap();
    assert_eq!(left, right);
    assert_eq!(first, second);
}

#[test]
fn unrelated_streams_do_not_interfere() {
    let expr = parse("2d6");
    let mut ordered = DeterministicRng::from_seed(42);
    let first_a = expr.evaluate(&mut ordered, "aa").unwrap();
    let first_b = expr.evaluate(&mut ordered, "bb").unwrap();
    let mut swapped = DeterministicRng::from_seed(42);
    let second_b = expr.evaluate(&mut swapped, "bb").unwrap();
    let second_a = expr.evaluate(&mut swapped, "aa").unwrap();
    assert_eq!(first_a, second_a);
    assert_eq!(first_b, second_b);
}

#[test]
fn failed_evaluation_preserves_the_whole_rng() {
    let expr = parse("2d6");
    let mut rng = DeterministicRng::from_seed(42);
    expr.evaluate(&mut rng, "warm").unwrap();
    let before = rng.clone();
    let error = expr.evaluate(&mut rng, "").expect_err("empty stream drew");
    assert_eq!(error.code, RulesErrorCode::InvalidStream);
    assert_eq!(rng, before);
    assert!(!rng.has_stream(""));
    let long = "s".repeat(MAX_RNG_STREAM_BYTES + 1);
    let error = expr
        .evaluate(&mut rng, &long)
        .expect_err("long stream drew");
    assert_eq!(error.code, RulesErrorCode::LimitExceeded);
    assert_eq!(rng, before);
    assert!(!rng.has_stream(&long));
}

#[test]
fn distribution_is_uniform_with_integer_only_bounds() {
    let expr = parse("1d6");
    let mut rng = DeterministicRng::from_seed(15);
    let mut counts = [0_usize; 6];
    for _ in 0..60_000 {
        let roll = expr.evaluate(&mut rng, "distribution").unwrap();
        counts[(roll.dice[0].value - 1) as usize] += 1;
        assert_eq!(roll.total, roll.dice[0].value as i32);
    }
    assert_eq!(counts, [9743, 9924, 10018, 10128, 9992, 10195]);
    for (face, count) in counts.iter().enumerate() {
        assert!(
            (9_000..=11_000).contains(count),
            "face {} drew {count} times",
            face + 1
        );
    }
}

#[test]
fn totals_match_exact_trace_arithmetic() {
    // Seed 7 keeps 1 + 2 + 6 with an offset of 5.
    let expr = DiceExpr::new(4, 6, DiceSelection::KeepHighest(3), 5).unwrap();
    let mut rng = DeterministicRng::from_seed(7);
    let roll = expr.evaluate(&mut rng, "probe-a").unwrap();
    assert_eq!(roll.total, 1 + 2 + 6 + 5);
    // Kept dice plus a saturating offset clamp once to i32::MAX.
    let expr = DiceExpr::new(3, 1, DiceSelection::All, i32::MAX).unwrap();
    let mut rng = DeterministicRng::from_seed(7);
    let roll = expr.evaluate(&mut rng, "probe-a").unwrap();
    assert_eq!(roll.dice.iter().map(|die| die.value).sum::<u32>(), 3);
    assert_eq!(roll.total, i32::MAX);
    // A minimum offset plus one pip lands one above the floor.
    let expr = DiceExpr::new(1, 1, DiceSelection::All, i32::MIN).unwrap();
    let mut rng = DeterministicRng::from_seed(7);
    let roll = expr.evaluate(&mut rng, "probe-a").unwrap();
    assert_eq!(roll.total, i32::MIN + 1);
}
