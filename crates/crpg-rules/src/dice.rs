//! Bounded dice expressions, parsing, and named-stream evaluation.
//!
//! A [`DiceExpr`] is one dice group plus an optional integer offset: a die
//! count, a side count, a keep/drop selection, and a signed offset. The
//! grammar is ASCII and case-sensitive, with no whitespace, omitted counts,
//! or general arithmetic. Evaluation draws every die in authored index order
//! from a caller-selected named stream of the caller's [`DeterministicRng`],
//! ranks keep/drop selection with earlier indices winning value ties, keeps
//! the trace in draw order, and clamps the kept sum plus offset to `i32`
//! once. Querying a dice-valued stat returns the expression; only explicit
//! evaluation (here) or resolution consumes randomness.
//!
//! [`DeterministicRng`]: crpg_core::DeterministicRng

use core::fmt;
use core::num::NonZeroU32;
use core::str::FromStr;

use crpg_core::DeterministicRng;

use crate::error::{RulesError, RulesErrorCode};
use crate::{MAX_DICE_COUNT, MAX_DICE_INPUT_BYTES, MAX_DIE_SIDES, MAX_RNG_STREAM_BYTES};

/// Which dice of a group contribute to the total.
///
/// Selection ranks values high or low as requested; equal values rank
/// earlier original indices first, including when selecting dice to drop.
/// The trace always stays in draw order regardless of selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiceSelection {
    /// Every drawn die is kept.
    All,
    /// Keeps the `n` highest dice; `n` must be `1..=count`.
    KeepHighest(u32),
    /// Keeps the `n` lowest dice; `n` must be `1..=count`.
    KeepLowest(u32),
    /// Drops the `n` highest dice; `n` must be `0..count` so at least one
    /// die is retained.
    DropHighest(u32),
    /// Drops the `n` lowest dice; `n` must be `0..count` so at least one die
    /// is retained.
    DropLowest(u32),
}

/// One bounded dice expression: a group plus an optional integer offset.
///
/// Equality is structural over the validated fields, not distributional.
/// The canonical notation uses minimal decimal digits, lowercase syntax,
/// preserves the selection variant (even a redundant keep/drop), and omits
/// a zero offset, so `parse(display(expr)) == expr`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiceExpr {
    count: u32,
    sides: u32,
    selection: DiceSelection,
    offset: i32,
}

impl DiceExpr {
    /// Builds an expression from validated parts.
    ///
    /// Checks `/dice/count` (must be `1..=MAX_DICE_COUNT`), then
    /// `/dice/sides` (must be `1..=MAX_DIE_SIDES`), then `/dice/selection`
    /// (keep values `1..=count`, drop values `0..count`).
    pub fn new(
        count: u32,
        sides: u32,
        selection: DiceSelection,
        offset: i32,
    ) -> Result<Self, RulesError> {
        if count == 0 || count as usize > MAX_DICE_COUNT {
            return Err(RulesError::at(
                RulesErrorCode::InvalidDice,
                String::from("/dice/count"),
            ));
        }
        if sides == 0 || sides > MAX_DIE_SIDES {
            return Err(RulesError::at(
                RulesErrorCode::InvalidDice,
                String::from("/dice/sides"),
            ));
        }
        let selected = match selection {
            DiceSelection::All => true,
            DiceSelection::KeepHighest(kept) | DiceSelection::KeepLowest(kept) => {
                kept >= 1 && kept <= count
            }
            DiceSelection::DropHighest(dropped) | DiceSelection::DropLowest(dropped) => {
                dropped < count
            }
        };
        if !selected {
            return Err(RulesError::at(
                RulesErrorCode::InvalidDice,
                String::from("/dice/selection"),
            ));
        }
        Ok(Self {
            count,
            sides,
            selection,
            offset,
        })
    }

    /// Returns the number of dice drawn.
    #[must_use]
    pub const fn count(&self) -> u32 {
        self.count
    }

    /// Returns the number of sides on each die.
    #[must_use]
    pub const fn sides(&self) -> u32 {
        self.sides
    }

    /// Returns the keep/drop selection.
    #[must_use]
    pub const fn selection(&self) -> DiceSelection {
        self.selection
    }

    /// Returns the integer offset added to the kept sum.
    #[must_use]
    pub const fn offset(&self) -> i32 {
        self.offset
    }

    /// Evaluates the expression against one named RNG stream.
    ///
    /// Draws every die in authored index order with core's
    /// `gen_range_u32(NonZeroU32(sides)) + 1`, including discarded dice and
    /// one-sided dice (which still consume a range draw). Selection ranks
    /// values as [`DiceSelection`] describes; the kept sum plus offset
    /// accumulates in `i64` and clamps once to `i32`.
    ///
    /// Fails with [`RulesErrorCode::InvalidStream`] at `/stream` for an
    /// empty stream name, or [`RulesErrorCode::LimitExceeded`] there when
    /// the name exceeds the stream byte limit. The stream is validated
    /// before any draw is requested.
    pub fn evaluate(
        &self,
        rng: &mut DeterministicRng,
        stream: &str,
    ) -> Result<DiceRoll, RulesError> {
        check_stream(stream)?;
        // Sides are at least one by construction, so the bound is nonzero.
        let bound = NonZeroU32::new(self.sides).expect("dice sides are at least one");
        let drawn = rng.stream(stream);
        let mut dice = Vec::with_capacity(self.count as usize);
        for _ in 0..self.count {
            dice.push(DieResult {
                value: drawn.gen_range_u32(bound) + 1,
                kept: true,
            });
        }
        apply_selection(&mut dice, self.selection);
        let mut sum = i64::from(self.offset);
        for die in &dice {
            if die.kept {
                sum += i64::from(die.value);
            }
        }
        Ok(DiceRoll {
            expression: self.clone(),
            dice,
            total: sum.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        })
    }
}

/// Validates a stream name: nonempty UTF-8 within the stream byte limit.
///
/// Byte length is the UTF-8 length of the `&str`. Reports
/// [`RulesErrorCode::InvalidStream`] for an empty name and
/// [`RulesErrorCode::LimitExceeded`] past the limit, both at `/stream`.
/// Callers that embed stream validation in a larger location remap the
/// returned error's location while preserving its code.
pub(crate) fn check_stream(stream: &str) -> Result<(), RulesError> {
    if stream.is_empty() {
        return Err(RulesError::at(
            RulesErrorCode::InvalidStream,
            String::from("/stream"),
        ));
    }
    if stream.len() > MAX_RNG_STREAM_BYTES {
        return Err(RulesError::at(
            RulesErrorCode::LimitExceeded,
            String::from("/stream"),
        ));
    }
    Ok(())
}

/// Marks kept/dropped dice in place, preserving draw order.
fn apply_selection(dice: &mut [DieResult], selection: DiceSelection) {
    let mut order: Vec<usize> = (0..dice.len()).collect();
    let keep_count = match selection {
        DiceSelection::All => return,
        DiceSelection::KeepHighest(kept) => {
            order.sort_by(|left, right| {
                dice[*right]
                    .value
                    .cmp(&dice[*left].value)
                    .then_with(|| left.cmp(right))
            });
            kept as usize
        }
        DiceSelection::KeepLowest(kept) => {
            order.sort_by(|left, right| {
                dice[*left]
                    .value
                    .cmp(&dice[*right].value)
                    .then_with(|| left.cmp(right))
            });
            kept as usize
        }
        DiceSelection::DropHighest(dropped) => {
            order.sort_by(|left, right| {
                dice[*right]
                    .value
                    .cmp(&dice[*left].value)
                    .then_with(|| left.cmp(right))
            });
            let dropped = dropped as usize;
            for index in order.into_iter().take(dropped) {
                dice[index].kept = false;
            }
            return;
        }
        DiceSelection::DropLowest(dropped) => {
            order.sort_by(|left, right| {
                dice[*left]
                    .value
                    .cmp(&dice[*right].value)
                    .then_with(|| left.cmp(right))
            });
            let dropped = dropped as usize;
            for index in order.into_iter().take(dropped) {
                dice[index].kept = false;
            }
            return;
        }
    };
    for index in order.into_iter().skip(keep_count) {
        dice[index].kept = false;
    }
}

impl fmt::Display for DiceExpr {
    /// Renders the canonical notation: minimal digits, lowercase syntax,
    /// the selection variant preserved, and a zero offset omitted.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}d{}", self.count, self.sides)?;
        match self.selection {
            DiceSelection::All => {}
            DiceSelection::KeepHighest(kept) => write!(f, "kh{kept}")?,
            DiceSelection::KeepLowest(kept) => write!(f, "kl{kept}")?,
            DiceSelection::DropHighest(dropped) => write!(f, "dh{dropped}")?,
            DiceSelection::DropLowest(dropped) => write!(f, "dl{dropped}")?,
        }
        if self.offset > 0 {
            write!(f, "+{}", self.offset)?;
        } else if self.offset < 0 {
            write!(f, "-{}", -(i64::from(self.offset)))?;
        }
        Ok(())
    }
}

impl FromStr for DiceExpr {
    type Err = RulesError;

    /// Parses `count "d" sides [selection] [offset]`, ASCII case-sensitive.
    ///
    /// No whitespace, omitted counts, parentheses, trailing data, decimal
    /// points, Unicode digits, or leading signs on count/sides. Leading
    /// zeros are accepted. The offset accepts the full `i32` range,
    /// including its minimum through negative magnitude parsing. Overflow
    /// is an error, never a panic. Input length is measured in UTF-8 bytes
    /// and bounded by `MAX_DICE_INPUT_BYTES`.
    ///
    /// Failures report [`RulesErrorCode::InvalidDice`] at
    /// `/dice/input/<byte-offset>`: the earliest offending byte, with end
    /// of input reported as the input length. Valid numeric tokens outside
    /// a permitted range point at the token start. Input longer than the
    /// byte limit reports [`RulesErrorCode::LimitExceeded`] at `/dice/input`.
    fn from_str(input: &str) -> Result<Self, RulesError> {
        if input.len() > MAX_DICE_INPUT_BYTES {
            return Err(RulesError::at(
                RulesErrorCode::LimitExceeded,
                String::from("/dice/input"),
            ));
        }
        let bytes = input.as_bytes();
        let at = |offset: usize| {
            RulesError::at(RulesErrorCode::InvalidDice, format!("/dice/input/{offset}"))
        };
        let mut index = 0_usize;

        // Count: one or more ASCII digits, 1..=MAX_DICE_COUNT.
        let (count, next) = take_number(bytes, index, 0)?;
        let Some(count) = count else {
            return Err(at(index));
        };
        if count == 0 || count > MAX_DICE_COUNT as u64 {
            return Err(at(0));
        }
        index = next;

        // The literal lowercase separator.
        if index >= bytes.len() {
            return Err(at(index));
        }
        if bytes[index] != b'd' {
            return Err(at(index));
        }
        index += 1;

        // Sides: one or more ASCII digits, 1..=MAX_DIE_SIDES.
        let sides_start = index;
        let (sides, next) = take_number(bytes, index, sides_start)?;
        let Some(sides) = sides else {
            return Err(at(index));
        };
        if sides == 0 || sides > u64::from(MAX_DIE_SIDES) {
            return Err(at(sides_start));
        }
        index = next;

        // Optional keep/drop selection with its unsigned operand.
        let mut selection = DiceSelection::All;
        if index < bytes.len() && (bytes[index] == b'k' || bytes[index] == b'd') {
            let select_start = index;
            let mode = if index + 1 < bytes.len() {
                (bytes[index], bytes[index + 1])
            } else {
                (bytes[index], 0)
            };
            let keep = match mode {
                (b'k', b'h') => true,
                (b'k', b'l') => true,
                (b'd', b'h') => false,
                (b'd', b'l') => false,
                _ => return Err(at(select_start)),
            };
            index += 2;
            let (operand, next) = take_number(bytes, index, index)?;
            let Some(operand) = operand else {
                return Err(at(index));
            };
            let count_u64 = count;
            if keep {
                if operand < 1 || operand > count_u64 || operand > u64::from(u32::MAX) {
                    return Err(at(select_start));
                }
                let operand = operand as u32;
                selection = if mode == (b'k', b'h') {
                    DiceSelection::KeepHighest(operand)
                } else {
                    DiceSelection::KeepLowest(operand)
                };
            } else {
                if operand >= count_u64 || operand > u64::from(u32::MAX) {
                    return Err(at(select_start));
                }
                let operand = operand as u32;
                selection = if mode == (b'd', b'h') {
                    DiceSelection::DropHighest(operand)
                } else {
                    DiceSelection::DropLowest(operand)
                };
            }
            index = next;
        }

        // Optional signed offset accepting the full i32 range.
        let mut offset = 0_i32;
        if index < bytes.len() && (bytes[index] == b'+' || bytes[index] == b'-') {
            let sign_start = index;
            let negative = bytes[index] == b'-';
            index += 1;
            let (magnitude, next) = take_number(bytes, index, sign_start)?;
            let Some(magnitude) = magnitude else {
                return Err(at(index));
            };
            let limit = if negative {
                i32::MAX as u64 + 1
            } else {
                i32::MAX as u64
            };
            if magnitude > limit {
                return Err(at(sign_start));
            }
            offset = if negative {
                -(magnitude as i64) as i32
            } else {
                magnitude as i32
            };
            index = next;
        }

        if index != bytes.len() {
            return Err(at(index));
        }
        // Counts, sides, and the selection operand are range-checked above
        // against bounds that fit `u32`, so these conversions are exact.
        Self::new(count as u32, sides as u32, selection, offset).map_err(|constructor| {
            // The parser checks ranges in token order before delegating
            // to the constructor, so a constructor fault repeats the
            // parser's own verdict; keep the parser-rooted location.
            let _ = constructor;
            at(0)
        })
    }
}

/// Reads one run of ASCII digits from `bytes` at `index`.
///
/// Returns the checked `u64` accumulation (or `None` when no digit leads)
/// plus the first index past the run. Accumulation overflow reports
/// [`RulesErrorCode::InvalidDice`] at `token_start`, the start of the
/// numeric token, so over-long digit runs are errors rather than panics.
fn take_number(
    bytes: &[u8],
    index: usize,
    token_start: usize,
) -> Result<(Option<u64>, usize), RulesError> {
    let mut value = 0_u64;
    let mut next = index;
    while next < bytes.len() && bytes[next].is_ascii_digit() {
        let digit = u64::from(bytes[next] - b'0');
        value = value
            .checked_mul(10)
            .and_then(|scaled| scaled.checked_add(digit))
            .ok_or_else(|| {
                RulesError::at(
                    RulesErrorCode::InvalidDice,
                    format!("/dice/input/{token_start}"),
                )
            })?;
        next += 1;
    }
    if next == index {
        Ok((None, index))
    } else {
        Ok((Some(value), next))
    }
}

impl serde::Serialize for DiceExpr {
    /// Serializes the canonical notation string.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for DiceExpr {
    /// Deserializes through the parser; no unchecked field decoding.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let text = <String as serde::Deserialize>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// One drawn die: its raw face value and whether selection kept it.
///
/// Faces always lie in `1..=sides`. The trace holds every draw, kept or
/// dropped, in draw order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DieResult {
    /// The raw face value drawn for this die.
    pub value: u32,
    /// Whether keep/drop selection retained this die in the total.
    pub kept: bool,
}

/// The evaluated outcome of one [`DiceExpr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiceRoll {
    /// The expression that was evaluated.
    pub expression: DiceExpr,
    /// Every drawn die in authored index order, kept or dropped.
    pub dice: Vec<DieResult>,
    /// The kept sum plus offset, clamped once to `i32`.
    pub total: i32,
}
