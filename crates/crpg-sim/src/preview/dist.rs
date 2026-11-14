//! Exact dice distributions and their cost formulas (T143 B3, B6).
//!
//! Nothing here enumerates a face tuple. Plain sums are counted one die at a
//! time by a sliding window over the reachable totals; keep and drop are
//! counted by a dynamic program over how many dice show each face value; a
//! natural face is counted by holding the selected die at each rule face and
//! subtracting those counts from the full distribution. Every count is an
//! exact [`PreviewCount`].
//!
//! The cost formulas are a contract: [`cost`] charges what these algorithms
//! do, so changing an algorithm means changing its formula in the same edit.

use std::collections::BTreeMap;

use crpg_rules::{DiceExpr, DiceSelection};

use super::count::PreviewCount;

/// One distribution: (raw total, natural tag) to count. The tag is the
/// natural die's face when that face is one of `rule_faces`, else `None`.
pub(crate) type Distribution = BTreeMap<(i32, Option<u32>), PreviewCount>;

/// The number of binary digits of `value` (`bitlen(1)` is 1).
fn bitlen(value: u32) -> u64 {
    u64::from(32 - value.leading_zeros())
}

/// The kept count `k`: `n` for `All`, `k` for keep, `n - d` for drop.
fn kept_count(expr: &DiceExpr) -> u32 {
    match expr.selection() {
        DiceSelection::All => expr.count(),
        DiceSelection::KeepHighest(kept) | DiceSelection::KeepLowest(kept) => kept,
        DiceSelection::DropHighest(dropped) | DiceSelection::DropLowest(dropped) => {
            expr.count() - dropped
        }
    }
}

/// The distinct natural-rule faces in `1..=sides`, ascending; empty unless a
/// natural face is read from `expr`.
fn natural_faces(expr: &DiceExpr, natural_die: Option<u32>, rule_faces: &[u32]) -> Vec<u32> {
    if natural_die.is_none() {
        return Vec::new();
    }
    let mut faces: Vec<u32> = rule_faces
        .iter()
        .copied()
        .filter(|face| (1..=expr.sides()).contains(face))
        .collect();
    faces.sort_unstable();
    faces.dedup();
    faces
}

/// `T(i)`: the number of sums of `i` dice of `sides` sides.
fn span(dice: u64, sides: u64) -> u64 {
    dice * (sides - 1) + 1
}

/// The limb width charged for counts of `expr` (B3).
pub(crate) fn limbs(expr: &DiceExpr) -> u64 {
    (u64::from(expr.count()) * bitlen(expr.sides())).div_ceil(64)
}

/// The number of rows `expr` can produce (B3).
pub(crate) fn rows(expr: &DiceExpr, natural_die: Option<u32>, rule_faces: &[u32]) -> u64 {
    let r = natural_faces(expr, natural_die, rule_faces).len() as u64;
    let n = u64::from(expr.count());
    let s = u64::from(expr.sides());
    match expr.selection() {
        DiceSelection::All => span(n, s) + r * span(n - 1, s),
        _ => {
            let base = span(u64::from(kept_count(expr)), s);
            if r > 0 {
                base * (r + 1)
            } else {
                base
            }
        }
    }
}

/// The cost of `expr` (B3); `None` when it overflows `u64`.
pub(crate) fn cost(expr: &DiceExpr, natural_die: Option<u32>, rule_faces: &[u32]) -> Option<u64> {
    let r = natural_faces(expr, natural_die, rule_faces).len() as u64;
    let n = u64::from(expr.count());
    let s = u64::from(expr.sides());
    let width = limbs(expr);
    match expr.selection() {
        DiceSelection::All => {
            // n + (s - 1) * n * (n + 1) / 2, plus 2 * r * T(n - 1).
            let pairs = n.checked_mul(n.checked_add(1)?)? / 2;
            let mut steps = n.checked_add((s - 1).checked_mul(pairs)?)?;
            if r > 0 {
                steps = steps.checked_add(2u64.checked_mul(r)?.checked_mul(span(n - 1, s))?)?;
            }
            steps.checked_mul(width)
        }
        _ => {
            // s * (n + 1)^2 * (k * s + 1), times (2r + 1), times the limb
            // widths of the count and of the binomial it is multiplied by.
            let k = u64::from(kept_count(expr));
            let square = (n + 1).checked_mul(n + 1)?;
            let mut steps = s
                .checked_mul(square)?
                .checked_mul(k.checked_mul(s)?.checked_add(1)?)?;
            if r > 0 {
                steps = steps.checked_mul(2 * r + 1)?;
            }
            steps.checked_mul(width)?.checked_mul(n.div_ceil(64))
        }
    }
}

/// `sides^count`.
pub(crate) fn tuples(expr: &DiceExpr) -> PreviewCount {
    PreviewCount::pow_small(u64::from(expr.sides()), expr.count())
}

/// The exact distribution (B6).
pub(crate) fn distribution(
    expr: &DiceExpr,
    natural_die: Option<u32>,
    rule_faces: &[u32],
) -> Distribution {
    let faces = natural_faces(expr, natural_die, rule_faces);
    let n = expr.count();
    let s = expr.sides();
    let offset = i64::from(expr.offset());
    let mut out = Distribution::new();

    // `untagged` starts as the full distribution and each tagged part is
    // subtracted from it. Both are counts indexed by the kept sum minus
    // `base`: the number of dice for a plain sum, zero for keep and drop.
    let (base, mut untagged, tagged) = match expr.selection() {
        DiceSelection::All => {
            let (previous, full) = sum_layers(n, s);
            // Holding one die at face `f`, the other `n - 1` dice sum as
            // `previous`, so its count sits `f - 1` above the full index.
            let tagged = faces
                .iter()
                .map(|&face| {
                    let mut lifted = vec![PreviewCount::zero(); face as usize - 1];
                    lifted.extend(previous.iter().cloned());
                    (face, lifted)
                })
                .collect::<Vec<_>>();
            (u64::from(n), full, tagged)
        }
        selection => {
            let k = kept_count(expr);
            let highest = matches!(
                selection,
                DiceSelection::KeepHighest(_) | DiceSelection::DropLowest(_)
            );
            let full = kept_counts(n, s, k, highest, None);
            let tagged = faces
                .iter()
                .map(|&face| (face, kept_counts(n, s, k, highest, Some(face))))
                .collect::<Vec<_>>();
            (0, full, tagged)
        }
    };

    for (face, counts) in &tagged {
        for (index, count) in counts.iter().enumerate() {
            bump(&mut out, offset, base + index as u64, Some(*face), count);
            untagged[index] = untagged[index]
                .checked_sub(count)
                .expect("tagged draws are a subset of all draws");
        }
    }
    for (index, count) in untagged.iter().enumerate() {
        bump(&mut out, offset, base + index as u64, None, count);
    }
    out
}

/// Adds `count` to the entry for the clamped raw total of `offset + sum`.
fn bump(out: &mut Distribution, offset: i64, sum: u64, tag: Option<u32>, count: &PreviewCount) {
    if count.is_zero() {
        return;
    }
    let raw = (offset + sum as i64).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    out.entry((raw, tag)).or_default().add_assign(count);
}

/// Sums of `n` dice, one die at a time. Returns the counts for `n - 1` dice
/// and for `n` dice, each indexed by the sum minus its dice count.
fn sum_layers(n: u32, sides: u32) -> (Vec<PreviewCount>, Vec<PreviewCount>) {
    let mut previous = Vec::new();
    let mut current = vec![PreviewCount::from(1)];
    for _ in 0..n {
        let next = add_die(&current, sides as usize);
        previous = std::mem::replace(&mut current, next);
    }
    (previous, current)
}

/// Adds one die of `sides` sides to counts indexed by sum minus dice count.
///
/// The count for a new total is the previous new total's count plus the old
/// count entering the window minus the old count leaving it: one add and one
/// subtract per new total, with no loop over faces.
fn add_die(old: &[PreviewCount], sides: usize) -> Vec<PreviewCount> {
    let mut next = Vec::with_capacity(old.len() + sides - 1);
    let mut running = PreviewCount::zero();
    for total in 0..old.len() + sides - 1 {
        if let Some(entering) = old.get(total) {
            running.add_assign(entering);
        }
        if total >= sides {
            running = running
                .checked_sub(&old[total - sides])
                .expect("a sliding window never leaves more than it holds");
        }
        next.push(running.clone());
    }
    next
}

/// Counts of the kept sum of `n` dice, indexed by that sum, where the
/// `k` highest (`highest`) or lowest dice are kept. With `fixed` one die is
/// held at that face and the other `n - 1` are free.
fn kept_counts(n: u32, sides: u32, k: u32, highest: bool, fixed: Option<u32>) -> Vec<PreviewCount> {
    // Keeping the lowest dice is keeping the highest of the mirrored faces
    // (`f -> sides + 1 - f`); the kept sum mirrors as `k * (sides + 1) - sum`.
    let mirror = |face: u32| sides + 1 - face;
    let ranked_fixed = fixed.map(|face| if highest { face } else { mirror(face) });
    let ranked = kept_highest(n, sides, k, ranked_fixed);
    if highest {
        return ranked;
    }
    let top = (k as usize) * (sides as usize + 1);
    let mut counts = vec![PreviewCount::zero(); ranked.len()];
    for (sum, count) in ranked.into_iter().enumerate() {
        if !count.is_zero() {
            counts[top - sum] = count;
        }
    }
    counts
}

/// The dynamic program for the `k` highest of `n` dice. It visits face values
/// from `sides` down to 1, choosing how many dice show each value and
/// weighting by the binomial count of which dice those are. Its state is the
/// number of free dice placed and the sum kept so far.
fn kept_highest(n: u32, sides: u32, k: u32, fixed: Option<u32>) -> Vec<PreviewCount> {
    let free = (n - u32::from(fixed.is_some())) as usize;
    let size = k as usize * sides as usize + 1;
    let blank = || vec![vec![PreviewCount::zero(); size]; free + 1];
    let mut state = blank();
    state[0][0] = PreviewCount::from(1);
    for face in (1..=sides).rev() {
        let fixed_here = u32::from(fixed == Some(face));
        let fixed_before = u32::from(fixed.is_some_and(|held| held > face));
        let mut next = blank();
        for placed in 0..=free {
            let remaining = (free - placed) as u64;
            // C(remaining, c) by the multiplicative recurrence.
            let mut binomial = PreviewCount::from(1);
            for c in 0..=remaining {
                if c > 0 {
                    binomial = binomial.mul_small(remaining - c + 1).div_small_exact(c);
                }
                let dice_before = placed as u32 + fixed_before;
                let kept_here = if dice_before >= k {
                    0
                } else {
                    (c as u32 + fixed_here).min(k - dice_before)
                };
                let gain = kept_here as usize * face as usize;
                for (sum, count) in state[placed].iter().enumerate() {
                    if count.is_zero() {
                        continue;
                    }
                    let weighted = match binomial.to_u64() {
                        Some(small) => count.mul_small(small),
                        None => count.mul(&binomial),
                    };
                    next[placed + c as usize][sum + gain].add_assign(&weighted);
                }
            }
        }
        state = next;
    }
    state.swap_remove(free)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dice(text: &str) -> DiceExpr {
        text.parse().unwrap()
    }

    fn built(count: u32, sides: u32, selection: DiceSelection, offset: i32) -> DiceExpr {
        DiceExpr::new(count, sides, selection, offset).unwrap()
    }

    /// U2 `costs`.
    #[test]
    fn costs() {
        // (expression, natural die, rule faces, cost, rows, limbs)
        type Case = (&'static str, Option<u32>, Vec<u32>, Option<u64>, u64, u64);
        let cases: Vec<Case> = vec![
            ("2d6", None, vec![], Some(17), 11, 1),
            ("1d1", None, vec![], Some(1), 1, 1),
            ("1d20", Some(0), vec![20, 1, 2], Some(26), 23, 1),
            ("1d20", Some(0), vec![21], Some(20), 20, 1),
            ("2d4kh1", Some(1), vec![4], Some(540), 8, 1),
            ("3d3kh1", None, vec![], Some(192), 3, 1),
            ("3d3kl1", None, vec![], Some(192), 3, 1),
            ("3d3dh1", None, vec![], Some(336), 5, 1),
            ("3d3dl1", None, vec![], Some(336), 5, 1),
            ("32d2", None, vec![], Some(560), 33, 1),
            ("33d2", None, vec![], Some(1188), 34, 2),
            ("100d6", None, vec![], Some(126_750), 501, 5),
            ("10d1000", None, vec![], Some(109_910), 9991, 2),
            ("10d10kh3", None, vec![], Some(37_510), 28, 1),
            ("2d1364kh1", None, vec![], Some(16_756_740), 1364, 1),
            ("2d1365kh1", None, vec![], Some(16_781_310), 1365, 1),
        ];
        for (text, natural, faces, want_cost, want_rows, want_limbs) in cases {
            let expr = dice(text);
            assert_eq!(cost(&expr, natural, &faces), want_cost, "cost of {text}");
            assert_eq!(rows(&expr, natural, &faces), want_rows, "rows of {text}");
            assert_eq!(limbs(&expr), want_limbs, "limbs of {text}");
        }
        // The overflowing case has a cost of `None` (rows and limbs are not
        // asserted for it).
        assert_eq!(cost(&dice("1024d1000000kh1000"), None, &[]), None);

        // Both neighbours of the 16,777,216 ceiling are tight: one die
        // size below the fit, one above.
        assert!(cost(&dice("2d1364kh1"), None, &[]).unwrap() <= 16_777_216);
        assert!(cost(&dice("2d1365kh1"), None, &[]).unwrap() > 16_777_216);
    }

    #[test]
    fn natural_faces_count_once_and_only_inside_the_die() {
        let plain = dice("1d20");
        // Duplicates, order and faces outside 1..=sides do not change r.
        assert_eq!(cost(&plain, Some(0), &[2, 1, 20, 1, 21, 0]), Some(26));
        assert_eq!(rows(&plain, Some(0), &[2, 1, 20, 1, 21, 0]), 23);
        // The faces are read only when a natural die is.
        assert_eq!(cost(&plain, None, &[1, 2, 20]), Some(20));
        assert_eq!(rows(&plain, None, &[1, 2, 20]), 20);
        // The kept arm reads them the same way.
        let kept = dice("2d4kh1");
        assert_eq!(cost(&kept, Some(0), &[4, 4, 5, 0]), Some(540));
        assert_eq!(rows(&kept, Some(0), &[4, 4, 5, 0]), 8);
        assert_eq!(cost(&kept, None, &[4]), Some(180));
        assert_eq!(rows(&kept, None, &[4]), 4);
        // Two kept faces: (2r + 1) = 5 and (r + 1) = 3.
        assert_eq!(cost(&kept, Some(1), &[1, 4]), Some(900));
        assert_eq!(rows(&kept, Some(1), &[1, 4]), 12);
    }

    #[test]
    fn tuples_is_sides_to_the_count() {
        assert_eq!(tuples(&dice("3d3")).to_u64(), Some(27));
        assert_eq!(tuples(&dice("3d3kh1")).to_u64(), Some(27));
        assert_eq!(tuples(&dice("1d1")).to_u64(), Some(1));
        assert_eq!(
            tuples(&dice("100d6")).to_string(),
            "653318623500070906096690267158057820537143710472954871543071966369497141477376"
        );
    }

    fn untagged(entries: &[(i32, u64)]) -> Distribution {
        entries
            .iter()
            .map(|&(raw, count)| ((raw, None), PreviewCount::from(count)))
            .collect()
    }

    fn from_counts(start: i32, counts: &[u64]) -> Distribution {
        untagged(
            &counts
                .iter()
                .enumerate()
                .map(|(i, &count)| (start + i as i32, count))
                .collect::<Vec<_>>(),
        )
    }

    fn tagged(start: i32, face: u32, counts: &[u64]) -> Distribution {
        counts
            .iter()
            .enumerate()
            .map(|(i, &count)| ((start + i as i32, Some(face)), PreviewCount::from(count)))
            .collect()
    }

    fn merged(parts: Vec<Distribution>) -> Distribution {
        parts.into_iter().flatten().collect()
    }

    /// U3 `literal_distributions`.
    #[test]
    fn literal_distributions() {
        let plain = |text: &str| distribution(&dice(text), None, &[]);
        assert_eq!(plain("3d3"), from_counts(3, &[1, 3, 6, 7, 6, 3, 1]));
        assert_eq!(plain("3d3kh1"), from_counts(1, &[1, 7, 19]));
        assert_eq!(plain("3d3kl1"), from_counts(1, &[19, 7, 1]));
        assert_eq!(plain("3d3dh1"), from_counts(2, &[7, 9, 7, 3, 1]));
        assert_eq!(plain("3d3kl2"), from_counts(2, &[7, 9, 7, 3, 1]));
        assert_eq!(plain("3d3dl1"), from_counts(2, &[1, 3, 7, 9, 7]));
        assert_eq!(plain("3d3kh2"), from_counts(2, &[1, 3, 7, 9, 7]));
        assert_eq!(plain("3d3-2"), from_counts(1, &[1, 3, 6, 7, 6, 3, 1]));
        assert_eq!(plain("1d3-3"), from_counts(-2, &[1, 1, 1]));
        assert_eq!(plain("1d4+2147483647"), untagged(&[(2_147_483_647, 4)]));

        // 3d3, die 2, faces {3}.
        assert_eq!(
            distribution(&dice("3d3"), Some(2), &[3]),
            merged(vec![
                tagged(5, 3, &[1, 2, 3, 2, 1]),
                from_counts(3, &[1, 3, 5, 5, 3, 1]),
            ])
        );

        // 2d4kh1, die 1, faces {4}.
        let mut want = untagged(&[(1, 1), (2, 3), (3, 5), (4, 3)]);
        want.insert((4, Some(4)), PreviewCount::from(4));
        assert_eq!(distribution(&dice("2d4kh1"), Some(1), &[4]), want);
    }

    #[test]
    fn literal_distribution_100d6() {
        let got = distribution(&dice("100d6"), None, &[]);
        assert_eq!(got.len(), 501);
        let counts: Vec<(i32, &PreviewCount)> = got
            .iter()
            .map(|((raw, tag), c)| {
                assert_eq!(*tag, None);
                (*raw, c)
            })
            .collect();
        assert_eq!(counts[0].0, 100);
        assert_eq!(counts[500].0, 600);
        let small = |i: usize| counts[i].1.to_u64().unwrap();
        assert_eq!([small(0), small(1), small(2)], [1, 100, 5050]);
        assert_eq!([small(498), small(499), small(500)], [5050, 100, 1]);
        let mut total = PreviewCount::zero();
        for (_, count) in &counts {
            total.add_assign(count);
        }
        assert_eq!(total, tuples(&dice("100d6")));
        assert_eq!(
            total.to_string(),
            "653318623500070906096690267158057820537143710472954871543071966369497141477376"
        );
    }

    #[test]
    fn literal_distribution_10d1000() {
        let got = distribution(&dice("10d1000"), None, &[]);
        assert_eq!(got.len(), 9991);
        let counts: Vec<(i32, &PreviewCount)> = got.iter().map(|((raw, _), c)| (*raw, c)).collect();
        assert_eq!((counts[0].0, counts[9990].0), (10, 10_000));
        let small = |i: usize| counts[i].1.to_u64().unwrap();
        assert_eq!([small(0), small(1), small(2)], [1, 10, 55]);
        assert_eq!([small(9988), small(9989), small(9990)], [55, 10, 1]);
        let mut total = PreviewCount::zero();
        for (_, count) in &counts {
            total.add_assign(count);
        }
        assert_eq!(total.to_string(), "1000000000000000000000000000000");
    }

    #[test]
    fn literal_distribution_10d10kh3() {
        let got = distribution(&dice("10d10kh3"), None, &[]);
        assert_eq!(got.len(), 28);
        let count_at = |raw: i32| got[&(raw, None)].to_u64().unwrap();
        assert_eq!(count_at(3), 1);
        assert_eq!(count_at(4), 10);
        assert_eq!(count_at(29), 1_182_127_725);
        assert_eq!(count_at(30), 701_908_264);
        assert_eq!(*got.keys().next().unwrap(), (3, None));
        assert_eq!(*got.keys().last().unwrap(), (30, None));
        let mut total = PreviewCount::zero();
        for count in got.values() {
            total.add_assign(count);
        }
        assert_eq!(total.to_u64(), Some(10_000_000_000));
    }

    /// The kept cost at counts with more than one limb: `steps * limbs *
    /// ceil(n / 64)`, each figure worked by hand from B3.
    #[test]
    fn kept_cost_charges_count_and_binomial_widths() {
        // (expression, steps, limbs, ceil(n / 64), cost)
        let cases = [
            // 2 * 34^2 * 3 = 6936
            ("33d2kh1", 6_936u64, 2u64, 1u64, 13_872u64),
            // 2 * 65^2 * 3 = 25350; limbs ceil(128 / 64) = 2
            ("64d2kh1", 25_350, 2, 1, 50_700),
            // 2 * 66^2 * 3 = 26136; limbs ceil(130 / 64) = 3
            ("65d2kh1", 26_136, 3, 2, 156_816),
            // 2 * 129^2 * 3 = 99846; limbs ceil(256 / 64) = 4
            ("128d2kh1", 99_846, 4, 2, 798_768),
            // drop arms: k = 64, 2 * 66^2 * (64 * 2 + 1) = 1123848
            ("65d2dh1", 1_123_848, 3, 2, 6_743_088),
            ("65d2dl1", 1_123_848, 3, 2, 6_743_088),
            ("65d2kl64", 1_123_848, 3, 2, 6_743_088),
        ];
        for (text, steps, width, binomial, want) in cases {
            let expr = dice(text);
            assert_eq!(limbs(&expr), width, "limbs of {text}");
            assert_eq!(steps * width * binomial, want, "hand figure of {text}");
            assert_eq!(cost(&expr, None, &[]), Some(want), "cost of {text}");
        }
        // The natural factor (2r + 1) multiplies before the widths:
        // 6936 * 3 * 2 * 1.
        assert_eq!(cost(&dice("33d2kh1"), Some(0), &[2]), Some(41_616));
    }

    /// Partial products of the kept cost, in u128 so no step can wrap.
    fn kept_steps(n: u64, s: u64, k: u64, r: u64, width: u64) -> [u128; 4] {
        let base = u128::from(s) * u128::from((n + 1) * (n + 1)) * u128::from(k * s + 1);
        let with_natural = if r > 0 {
            base * u128::from(2 * r + 1)
        } else {
            base
        };
        let with_width = with_natural * u128::from(width);
        let total = with_width * u128::from(n.div_ceil(64));
        [base, with_natural, with_width, total]
    }

    fn fits(value: u128) -> bool {
        value <= u128::from(u64::MAX)
    }

    #[test]
    fn kept_cost_overflows_at_the_natural_factor() {
        // 3d1000000kh1: one limb, so only the (2r + 1) product can overflow.
        let expr = dice("3d1000000kh1");
        assert_eq!(limbs(&expr), 1);
        let base = kept_steps(3, 1_000_000, 1, 0, 1)[0];
        let most = u128::from(u64::MAX) / base; // the largest 2r + 1
        let r_fits = ((most - 1) / 2) as u32;
        let faces_fit: Vec<u32> = (1..=r_fits).collect();
        let faces_over: Vec<u32> = (1..=r_fits + 1).collect();
        let fit = kept_steps(3, 1_000_000, 1, u64::from(r_fits), 1);
        let over = kept_steps(3, 1_000_000, 1, u64::from(r_fits) + 1, 1);
        assert!(fits(fit[3]) && !fits(over[1]) && fits(over[0]));
        assert_eq!(cost(&expr, Some(0), &faces_fit), Some(fit[3] as u64));
        assert_eq!(cost(&expr, Some(0), &faces_over), None);
    }

    #[test]
    fn kept_cost_overflows_at_the_limb_width() {
        // 64d1000000kh64: width 20, one binomial limb. With r = 2 the steps
        // still fit and only `* width` overflows; r = 1 is the control.
        let expr = dice("64d1000000kh64");
        assert_eq!(limbs(&expr), 20);
        let control = kept_steps(64, 1_000_000, 64, 1, 20);
        let over = kept_steps(64, 1_000_000, 64, 2, 20);
        assert!(fits(control[3]));
        assert!(fits(over[1]) && !fits(over[2]));
        assert_eq!(cost(&expr, Some(0), &[1]), Some(control[3] as u64));
        assert_eq!(cost(&expr, Some(0), &[1, 2]), None);
    }

    #[test]
    fn kept_cost_overflows_at_the_binomial_width() {
        // 1024d5000kh1024: width 208, binomial width 16. The steps times the
        // width fit and only `* ceil(n / 64)` overflows; kh100 is the control.
        let expr = dice("1024d5000kh1024");
        assert_eq!(limbs(&expr), 208);
        let over = kept_steps(1024, 5000, 1024, 0, 208);
        assert!(fits(over[2]) && !fits(over[3]));
        assert_eq!(cost(&expr, None, &[]), None);
        let control_expr = dice("1024d5000kh100");
        let control = kept_steps(1024, 5000, 100, 0, 208);
        assert!(fits(control[3]));
        assert_eq!(cost(&control_expr, None, &[]), Some(control[3] as u64));
    }

    #[test]
    fn negative_extreme_offset_is_not_clamped_or_shifted() {
        // Faces are positive, so the totals sit just above i32::MIN.
        let got = distribution(&built(1, 4, DiceSelection::All, i32::MIN), Some(0), &[1, 2]);
        let mut want = Distribution::new();
        want.insert((i32::MIN + 1, Some(1)), PreviewCount::from(1));
        want.insert((i32::MIN + 2, Some(2)), PreviewCount::from(1));
        want.insert((i32::MIN + 3, None), PreviewCount::from(1));
        want.insert((i32::MIN + 4, None), PreviewCount::from(1));
        assert_eq!(got, want);

        let kept = distribution(
            &built(2, 2, DiceSelection::KeepLowest(1), i32::MIN),
            None,
            &[],
        );
        let mut want = Distribution::new();
        want.insert((i32::MIN + 1, None), PreviewCount::from(3));
        want.insert((i32::MIN + 2, None), PreviewCount::from(1));
        assert_eq!(kept, want);
    }

    #[test]
    fn clamping_merges_totals_and_tags() {
        // Every total clamps to i32::MAX: the tagged and untagged parts
        // merge by tag.
        let got = distribution(&dice("1d4+2147483647"), Some(0), &[1, 2]);
        let mut want = Distribution::new();
        want.insert((i32::MAX, Some(1)), PreviewCount::from(1));
        want.insert((i32::MAX, Some(2)), PreviewCount::from(1));
        want.insert((i32::MAX, None), PreviewCount::from(2));
        assert_eq!(got, want);
    }

    // ---- U4: agreement with enumeration ----

    /// The test's own enumeration of every tuple.
    fn enumerate(expr: &DiceExpr, natural_die: Option<u32>, rule_faces: &[u32]) -> Distribution {
        let n = expr.count() as usize;
        let s = expr.sides();
        let mut out = Distribution::new();
        let mut tuple = vec![1u32; n];
        loop {
            let mut sorted = tuple.clone();
            sorted.sort_unstable();
            let kept: &[u32] = match expr.selection() {
                DiceSelection::All => &sorted,
                DiceSelection::KeepHighest(k) => &sorted[n - k as usize..],
                DiceSelection::KeepLowest(k) => &sorted[..k as usize],
                DiceSelection::DropHighest(d) => &sorted[..n - d as usize],
                DiceSelection::DropLowest(d) => &sorted[d as usize..],
            };
            let sum: i64 =
                kept.iter().map(|&f| i64::from(f)).sum::<i64>() + i64::from(expr.offset());
            let raw = sum.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
            let tag = natural_die
                .map(|j| tuple[j as usize])
                .filter(|face| rule_faces.contains(face));
            out.entry((raw, tag))
                .or_insert_with(PreviewCount::zero)
                .add_assign(&PreviewCount::from(1));
            // Odometer.
            let mut digit = 0;
            loop {
                if digit == n {
                    return out;
                }
                if tuple[digit] < s {
                    tuple[digit] += 1;
                    break;
                }
                tuple[digit] = 1;
                digit += 1;
            }
        }
    }

    fn selections(n: u32) -> Vec<DiceSelection> {
        let mut all = vec![DiceSelection::All];
        for k in 1..=n {
            all.push(DiceSelection::KeepHighest(k));
            all.push(DiceSelection::KeepLowest(k));
        }
        for d in 0..n {
            all.push(DiceSelection::DropHighest(d));
            all.push(DiceSelection::DropLowest(d));
        }
        all
    }

    fn check_against_enumeration(counts: &[(u32, u32)], offsets: &[i32]) -> usize {
        let mut checked = 0;
        for &(n, s) in counts {
            let rule_sets: Vec<Vec<u32>> = vec![vec![], vec![1], vec![s], vec![1, 2], vec![2, 1]];
            let mut dice_choices: Vec<Option<u32>> = vec![None];
            dice_choices.extend((0..n).map(Some));
            for selection in selections(n) {
                for &offset in offsets {
                    let expr = built(n, s, selection, offset);
                    for &natural in &dice_choices {
                        for faces in &rule_sets {
                            let got = distribution(&expr, natural, faces);
                            let want = enumerate(&expr, natural, faces);
                            let label = format!("{expr} die {natural:?} faces {faces:?}");
                            assert_eq!(got, want, "{label}");
                            let mut total = PreviewCount::zero();
                            for count in got.values() {
                                assert!(!count.is_zero(), "{label}: zero entry");
                                total.add_assign(count);
                            }
                            assert_eq!(total, tuples(&expr), "{label}: sum");
                            assert!(
                                got.len() as u64 <= rows(&expr, natural, faces),
                                "{label}: rows"
                            );
                            checked += 1;
                        }
                    }
                }
            }
        }
        checked
    }

    /// U4 `matches_enumeration`.
    #[test]
    fn matches_enumeration() {
        let mut counts = Vec::new();
        for n in 1..=3 {
            for s in 1..=5 {
                counts.push((n, s));
            }
        }
        counts.push((4, 3));
        let checked = check_against_enumeration(&counts, &[-3, 0, 2]);
        assert!(checked > 5000, "the sweep ran {checked} cases");
    }

    #[test]
    fn matches_enumeration_at_the_clamp() {
        let counts = [(1, 4), (2, 3), (3, 2), (3, 4)];
        check_against_enumeration(&counts, &[i32::MAX - 2, i32::MAX]);
    }

    #[test]
    fn rule_face_order_and_repeats_change_nothing() {
        let expr = dice("3d4kh2");
        let a = distribution(&expr, Some(1), &[1, 4]);
        let b = distribution(&expr, Some(1), &[4, 1, 4, 9, 0]);
        assert_eq!(a, b);
        // Rule faces are not read without a natural die.
        assert_eq!(
            distribution(&expr, None, &[1, 4]),
            distribution(&expr, None, &[])
        );
    }

    #[test]
    fn die_index_does_not_change_counts() {
        for text in ["3d4", "3d4kh2", "3d4kl1", "3d4dh1", "3d4dl2"] {
            let expr = dice(text);
            let first = distribution(&expr, Some(0), &[2, 3]);
            for j in 1..3 {
                assert_eq!(
                    distribution(&expr, Some(j), &[2, 3]),
                    first,
                    "{text} die {j}"
                );
            }
        }
    }

    #[test]
    fn counts_wider_than_a_limb_stay_exact() {
        // 33d2 needs two limbs; the sum of its distribution is 2^33 and the
        // middle count is C(33, 16).
        let got = distribution(&dice("33d2"), None, &[]);
        assert_eq!(got.len(), 34);
        assert_eq!(got[&(49, None)].to_u64(), Some(1_166_803_110));
        // 70 dice of four sides keep-highest-one: 4^70 does not fit u128.
        let wide = distribution(&dice("70d4kh1"), None, &[]);
        let mut total = PreviewCount::zero();
        for count in wide.values() {
            total.add_assign(count);
        }
        assert_eq!(total, tuples(&dice("70d4kh1")));
        assert_eq!(total.to_u128(), None);
        // Of those, one face tuple scores 1 (all ones).
        assert_eq!(wide[&(1, None)].to_u64(), Some(1));
    }
}
