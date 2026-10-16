//! Signed 16.16 fixed point. See the doc comment on each item for its exact contract.
#![forbid(unsafe_code)]

/// A signed 16.16 fixed-point number: the real value is `self.0 / 65536`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fx(pub i32);

/// Why [`Fx::parse`] rejected its input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseFxError {
    /// The text does not match the grammar.
    Invalid,
    /// The text matches the grammar but its rounded value is outside the `i32` range of `Fx`.
    OutOfRange,
}

impl Fx {
    /// 1.0
    pub const ONE: Fx = Fx(1 << 16);
    /// Largest value, 32767.99998474...
    pub const MAX: Fx = Fx(i32::MAX);
    /// Smallest value, -32768.0
    pub const MIN: Fx = Fx(i32::MIN);

    /// Multiply. The exact real product is rounded to the nearest `Fx`; an exact
    /// tie (half a unit) rounds away from zero. A result outside the `i32` range
    /// saturates to [`Fx::MAX`] or [`Fx::MIN`]. Never panics.
    pub fn mul_round(self, rhs: Fx) -> Fx {
        todo!()
    }

    /// Divide. `None` when `rhs` is zero. Otherwise the exact real quotient is
    /// truncated toward zero to an `Fx`; a result outside the `i32` range
    /// saturates to [`Fx::MAX`] or [`Fx::MIN`]. Never panics.
    pub fn checked_div(self, rhs: Fx) -> Option<Fx> {
        todo!()
    }

    /// Parse decimal text. Grammar (ASCII only, nothing else allowed, no whitespace):
    /// an optional `-`, then one or more digits `0-9`, then optionally a `.` followed
    /// by between 1 and 18 digits. Leading zeros are allowed. Anything else is
    /// [`ParseFxError::Invalid`] (for example `+1`, `.5`, `5.`, `1e3`, ` 1`, or 19
    /// fractional digits).
    ///
    /// The exact decimal value is rounded to the nearest `Fx`; an exact tie rounds
    /// away from zero. If the rounded value does not fit in `i32` the result is
    /// [`ParseFxError::OutOfRange`], however many integer digits the text has.
    /// `-0` and values that round to zero give `Fx(0)`. Never panics.
    pub fn parse(s: &str) -> Result<Fx, ParseFxError> {
        todo!()
    }
}
