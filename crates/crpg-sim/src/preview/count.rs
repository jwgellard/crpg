//! An exact unsigned integer of any size, for counting equally likely draws.
//!
//! The preview's counts (`sides^count` and the sums over them) overflow any
//! fixed-width integer long before the dice limits, so this is a hand-written
//! big integer: little-endian base-2^64 limbs, schoolbook arithmetic, no
//! dependency. Only what counting needs is here: add, checked subtract,
//! multiply, small multiply, exact small division, power, compare and decimal
//! display.

use core::cmp::Ordering;
use core::fmt;

/// An exact count of equally likely draws, of any size (T143).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviewCount {
    // Little-endian base-2^64 limbs with no trailing zero limb; zero is empty.
    limbs: Vec<u64>,
}

/// The largest power of ten below 2^64, and its digit count, for display.
const DECIMAL_CHUNK: u64 = 10_000_000_000_000_000_000;
const DECIMAL_CHUNK_DIGITS: usize = 19;

impl PreviewCount {
    /// Zero.
    #[must_use]
    pub fn zero() -> Self {
        Self { limbs: Vec::new() }
    }

    /// Whether this is zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    /// The value when it fits `u64`.
    #[must_use]
    pub fn to_u64(&self) -> Option<u64> {
        match self.limbs.as_slice() {
            [] => Some(0),
            [low] => Some(*low),
            _ => None,
        }
    }

    /// The value when it fits `u128`.
    #[must_use]
    pub fn to_u128(&self) -> Option<u128> {
        match self.limbs.as_slice() {
            [] => Some(0),
            [low] => Some(u128::from(*low)),
            [low, high] => Some(u128::from(*low) | (u128::from(*high) << 64)),
            _ => None,
        }
    }

    /// Drops trailing zero limbs so equal values compare equal.
    fn trim(&mut self) {
        while self.limbs.last() == Some(&0) {
            self.limbs.pop();
        }
    }

    /// Divides in place by `divisor` (nonzero) and returns the remainder.
    fn divmod_small(&mut self, divisor: u64) -> u64 {
        let divisor = u128::from(divisor);
        let mut rest: u128 = 0;
        for limb in self.limbs.iter_mut().rev() {
            let current = (rest << 64) | u128::from(*limb);
            *limb = (current / divisor) as u64;
            rest = current % divisor;
        }
        self.trim();
        rest as u64
    }
}

#[allow(dead_code)] // used by T143c, which removes this allow
impl PreviewCount {
    /// Adds `other` into `self`.
    pub(crate) fn add_assign(&mut self, other: &Self) {
        if self.limbs.len() < other.limbs.len() {
            self.limbs.resize(other.limbs.len(), 0);
        }
        let mut carry = false;
        for (index, limb) in self.limbs.iter_mut().enumerate() {
            let addend = other.limbs.get(index).copied().unwrap_or(0);
            if index >= other.limbs.len() && !carry {
                break;
            }
            let (sum, first) = limb.overflowing_add(addend);
            let (sum, second) = sum.overflowing_add(u64::from(carry));
            *limb = sum;
            carry = first || second;
        }
        if carry {
            self.limbs.push(1);
        }
    }

    /// `self - other`, or `None` when `other` is larger.
    pub(crate) fn checked_sub(&self, other: &Self) -> Option<Self> {
        if self < other {
            return None;
        }
        let mut limbs = self.limbs.clone();
        let mut borrow = false;
        for (index, limb) in limbs.iter_mut().enumerate() {
            let subtrahend = other.limbs.get(index).copied().unwrap_or(0);
            if index >= other.limbs.len() && !borrow {
                break;
            }
            let (diff, first) = limb.overflowing_sub(subtrahend);
            let (diff, second) = diff.overflowing_sub(u64::from(borrow));
            *limb = diff;
            borrow = first || second;
        }
        let mut result = Self { limbs };
        result.trim();
        Some(result)
    }

    /// `self * other`.
    pub(crate) fn mul(&self, other: &Self) -> Self {
        if self.is_zero() || other.is_zero() {
            return Self::zero();
        }
        let mut limbs = vec![0u64; self.limbs.len() + other.limbs.len()];
        for (i, &a) in self.limbs.iter().enumerate() {
            let mut carry: u128 = 0;
            for (j, &b) in other.limbs.iter().enumerate() {
                let cell = u128::from(a) * u128::from(b) + u128::from(limbs[i + j]) + carry;
                limbs[i + j] = cell as u64;
                carry = cell >> 64;
            }
            limbs[i + other.limbs.len()] = carry as u64;
        }
        let mut result = Self { limbs };
        result.trim();
        result
    }

    /// `self * factor`.
    pub(crate) fn mul_small(&self, factor: u64) -> Self {
        if factor == 0 || self.is_zero() {
            return Self::zero();
        }
        let mut limbs = Vec::with_capacity(self.limbs.len() + 1);
        let mut carry: u128 = 0;
        for &limb in &self.limbs {
            let cell = u128::from(limb) * u128::from(factor) + carry;
            limbs.push(cell as u64);
            carry = cell >> 64;
        }
        if carry != 0 {
            limbs.push(carry as u64);
        }
        Self { limbs }
    }

    /// `self / divisor`, for a division that is exact.
    ///
    /// # Panics
    /// When `divisor` is zero or leaves a nonzero remainder.
    pub(crate) fn div_small_exact(&self, divisor: u64) -> Self {
        assert!(divisor != 0, "preview count division by zero");
        let mut quotient = self.clone();
        let rest = quotient.divmod_small(divisor);
        assert!(rest == 0, "preview count division was not exact");
        quotient
    }

    /// `base^exp`.
    pub(crate) fn pow_small(base: u64, exp: u32) -> Self {
        let mut result = Self::from(1);
        for _ in 0..exp {
            result = result.mul_small(base);
        }
        result
    }
}

impl From<u64> for PreviewCount {
    fn from(value: u64) -> Self {
        if value == 0 {
            Self::zero()
        } else {
            Self { limbs: vec![value] }
        }
    }
}

impl Ord for PreviewCount {
    fn cmp(&self, other: &Self) -> Ordering {
        self.limbs
            .len()
            .cmp(&other.limbs.len())
            .then_with(|| self.limbs.iter().rev().cmp(other.limbs.iter().rev()))
    }
}

impl PartialOrd for PreviewCount {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for PreviewCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return f.write_str("0");
        }
        // Peel off 19 decimal digits at a time, least significant first.
        let mut rest = self.clone();
        let mut chunks = Vec::new();
        while !rest.is_zero() {
            chunks.push(rest.divmod_small(DECIMAL_CHUNK));
        }
        let mut chunks = chunks.into_iter().rev();
        if let Some(first) = chunks.next() {
            write!(f, "{first}")?;
        }
        for chunk in chunks {
            write!(f, "{chunk:0width$}", width = DECIMAL_CHUNK_DIGITS)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big(text: &str) -> PreviewCount {
        // Test-local decimal parser: independent of the code under test's
        // display path, so a wrong carry cannot cancel against itself.
        let mut value = PreviewCount::zero();
        for digit in text.bytes() {
            value = value.mul_small(10);
            value.add_assign(&PreviewCount::from(u64::from(digit - b'0')));
        }
        value
    }

    /// U1 `count_arithmetic`.
    #[test]
    fn count_arithmetic() {
        let max = PreviewCount::from(u64::MAX);
        assert_eq!(max.to_string(), "18446744073709551615");
        let mut plus_one = max.clone();
        plus_one.add_assign(&PreviewCount::from(1));
        assert_eq!(plus_one.to_string(), "18446744073709551616");
        assert_eq!(plus_one.to_u64(), None);
        assert_eq!(plus_one.to_u128(), Some(1u128 << 64));

        let square = plus_one.mul(&plus_one);
        assert_eq!(
            square.to_string(),
            "340282366920938463463374607431768211456"
        );
        assert_eq!(square.to_u128(), None);
        assert_eq!(square.to_u64(), None);

        // u64::MAX squared pins the carry of the widest limb product.
        assert_eq!(
            max.mul(&max).to_string(),
            "340282366920938463426481119284349108225"
        );
        assert_eq!(max.mul(&max).to_u128(), Some(u128::from(u64::MAX).pow(2)));

        assert_eq!(
            PreviewCount::pow_small(6, 100).to_string(),
            "653318623500070906096690267158057820537143710472954871543071966369497141477376"
        );
        assert_eq!(PreviewCount::pow_small(7, 0).to_string(), "1");
        assert_eq!(PreviewCount::pow_small(0, 3), PreviewCount::zero());

        // C(100, 50) by the multiplicative recurrence.
        let mut binomial = PreviewCount::from(1);
        for i in 1..=50u64 {
            binomial = binomial.mul_small(100 - 50 + i).div_small_exact(i);
        }
        assert_eq!(binomial.to_string(), "100891344545564193334812497256");

        // Zero.
        assert_eq!(PreviewCount::zero().to_string(), "0");
        assert_eq!(PreviewCount::from(0), PreviewCount::zero());
        assert_eq!(PreviewCount::default(), PreviewCount::zero());
        assert!(PreviewCount::zero().is_zero());
        assert!(!PreviewCount::from(1).is_zero());
        assert_eq!(PreviewCount::zero().to_u64(), Some(0));
        assert_eq!(PreviewCount::zero().to_u128(), Some(0));
        assert_eq!(PreviewCount::from(u64::MAX).to_u64(), Some(u64::MAX));

        // Ordering is numeric.
        assert!(plus_one > max);
        assert!(max < plus_one);
        assert!(PreviewCount::from(3) < PreviewCount::from(4));
        assert!(PreviewCount::zero() < PreviewCount::from(1));
        assert_eq!(
            PreviewCount::from(5).cmp(&PreviewCount::from(5)),
            Ordering::Equal
        );
        assert!(square > plus_one);
        // Same length, difference in the high limb only.
        assert!(big("36893488147419103232") > big("18446744073709551616"));
        // Same length, difference in the low limb only.
        assert!(big("18446744073709551617") > big("18446744073709551616"));

        // Subtraction.
        assert_eq!(plus_one.checked_sub(&max), Some(PreviewCount::from(1)));
        assert_eq!(max.checked_sub(&plus_one), None);
        assert_eq!(
            PreviewCount::from(3).checked_sub(&PreviewCount::from(3)),
            Some(PreviewCount::zero())
        );
        assert_eq!(
            PreviewCount::from(3).checked_sub(&PreviewCount::from(4)),
            None
        );
        assert_eq!(
            PreviewCount::zero().checked_sub(&PreviewCount::from(1)),
            None
        );
        // A borrow ripples through two zero limbs and the result is trimmed.
        let power128 = square.clone(); // 2^128
        assert_eq!(
            power128
                .checked_sub(&PreviewCount::from(1))
                .unwrap()
                .to_string(),
            "340282366920938463463374607431768211455"
        );
        assert_eq!(power128.checked_sub(&power128), Some(PreviewCount::zero()));
        assert_eq!(
            power128
                .checked_sub(&PreviewCount::from(1))
                .unwrap()
                .to_u128(),
            Some(u128::MAX)
        );
        // Borrow out of the low limb only: 2^64 - 1 leaves a single limb.
        assert_eq!(
            plus_one.checked_sub(&PreviewCount::from(1)),
            Some(max.clone())
        );

        // A carry ripples through a full limb pair.
        let mut ripple = PreviewCount::from(u64::MAX);
        ripple.add_assign(&PreviewCount::from(u64::MAX));
        assert_eq!(ripple.to_u128(), Some(2 * u128::from(u64::MAX)));
        let mut all_ones = big("340282366920938463463374607431768211455");
        all_ones.add_assign(&PreviewCount::from(1));
        assert_eq!(all_ones, square);
        // Adding a longer operand into a shorter one.
        let mut short = PreviewCount::from(1);
        short.add_assign(&square);
        assert_eq!(short.to_string(), "340282366920938463463374607431768211457");

        // Multiplication by zero, by one and across limbs.
        assert_eq!(square.mul(&PreviewCount::zero()), PreviewCount::zero());
        assert_eq!(PreviewCount::zero().mul(&square), PreviewCount::zero());
        assert_eq!(square.mul(&PreviewCount::from(1)), square);
        assert_eq!(square.mul_small(0), PreviewCount::zero());
        assert_eq!(
            max.mul_small(u64::MAX).to_string(),
            "340282366920938463426481119284349108225"
        );
        assert_eq!(
            square.mul(&square).to_string(),
            "115792089237316195423570985008687907853269984665640564039457584007913129639936"
        );

        // Exact division, including across limbs, and its panics.
        assert_eq!(square.div_small_exact(2).to_u128(), Some(1u128 << 127));
        assert_eq!(
            big("100891344545564193334812497256").div_small_exact(1),
            big("100891344545564193334812497256")
        );
        assert_eq!(
            PreviewCount::zero().div_small_exact(7),
            PreviewCount::zero()
        );
    }

    #[test]
    #[should_panic(expected = "preview count division was not exact")]
    fn div_small_exact_panics_on_a_remainder() {
        let _ = PreviewCount::from(7).div_small_exact(2);
    }

    #[test]
    #[should_panic(expected = "preview count division by zero")]
    fn div_small_exact_panics_on_zero() {
        let _ = PreviewCount::from(7).div_small_exact(0);
    }

    #[test]
    fn display_pads_inner_chunks_with_zeros() {
        // 10^19 is exactly one chunk boundary: "1" then nineteen zeros.
        assert_eq!(
            PreviewCount::pow_small(10, 19).to_string(),
            "10000000000000000000"
        );
        assert_eq!(
            PreviewCount::pow_small(10, 38).to_string(),
            "100000000000000000000000000000000000000"
        );
        assert_eq!(
            big("100000000000000000000000000000000000001").to_string(),
            "100000000000000000000000000000000000001"
        );
        assert_eq!(
            PreviewCount::from(9_999_999_999_999_999_999).to_string(),
            "9999999999999999999"
        );
        assert_eq!(PreviewCount::from(10).to_string(), "10");
        assert_eq!(PreviewCount::from(1).to_string(), "1");
    }
}
