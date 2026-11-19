//! T042b: helpers shared by the hostile-server suites. No clock, no thread,
//! no I/O, no unseeded RNG, no `HashMap`/`HashSet`, no float.
#![allow(dead_code)]

use std::ops::Range;

use crpg_core::Ulid;
use crpg_net::codec::CodecError;
use crpg_net::protocol::NetId;
use crpg_net::protocol_transfer::TransferId;

/// `[0x01, 0x02, …, 0x10]` (16 bytes, byte `k` is `k + 1`).
pub const EPOCH: [u8; 16] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
];

/// The pinned ability ULID `ABILITY` (`Ulid::from_parts(0x0123_4567_89AB,
/// 0x0CDE_F012_3456_789A)`, text `014D2PF2DB0000SQQG28T5CY4T`).
pub fn ability() -> Ulid {
    Ulid::from_parts(0x0123_4567_89AB, 0x0CDE_F012_3456_789A)
}

/// The pinned ability ULID `ABILITY_B` (`Ulid::from_parts(0x0123_4567_89AB,
/// 0x0CDE_F012_3456_789B)`, text `014D2PF2DB0000SQQG28T5CY4V`).
pub fn ability_b() -> Ulid {
    Ulid::from_parts(0x0123_4567_89AB, 0x0CDE_F012_3456_789B)
}

/// `NetId::new(n).unwrap()`.
pub fn net(n: u64) -> NetId {
    NetId::new(n).unwrap()
}

/// `TransferId::new(n).unwrap()`.
pub fn tid(n: u64) -> TransferId {
    TransferId::new(n).unwrap()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The decoder refused the bytes with this error.
    Refused(CodecError),
    /// The decoder accepted them; the canonical re-encoding of the value.
    Accepted(Vec<u8>),
}

/// `Err(e)` is `Refused(e)`; `Ok(v)` is `Accepted(encode(&v))`. If `encode`
/// refuses a decoded value this panics with `who`, `bytes` (`{:02X?}`) and the
/// error: that is a decoder defect (Reading 9, S-1).
pub fn reencoded<T>(
    who: &str,
    bytes: &[u8],
    decoded: Result<T, CodecError>,
    encode: fn(&T) -> Result<Vec<u8>, CodecError>,
) -> Verdict {
    match decoded {
        Err(error) => Verdict::Refused(error),
        Ok(value) => match encode(&value) {
            Ok(canonical) => Verdict::Accepted(canonical),
            Err(error) => panic!(
                "{who}: encoder refused a decoded value for input {:02X?}: {error:?}",
                bytes
            ),
        },
    }
}

/// `bytes` with `bytes[at]` replaced by `value`; every other byte unchanged.
pub fn with_byte(bytes: &[u8], at: usize, value: u8) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out[at] = value;
    out
}

/// `bytes` followed by `0x00` up to `len` (`len >= bytes.len()`).
pub fn padded(bytes: &[u8], len: usize) -> Vec<u8> {
    assert!(len >= bytes.len());
    let mut out = bytes.to_vec();
    out.resize(len, 0);
    out
}

/// The concatenation of `parts`, for literals.
pub fn cat(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for part in parts {
        out.extend_from_slice(part);
    }
    out
}

/// Row edits (the "Row notation"): each `(at, remove, insert)` replaces
/// `bytes[at .. at + remove]` by `insert`. Panics if the edits are not
/// ascending by `at`, overlap, or reach past `bytes`.
pub fn edit(bytes: &[u8], edits: &[(usize, usize, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for &(at, remove, insert) in edits {
        assert!(at >= cursor, "edit at {at} is not ascending or overlaps");
        assert!(
            at + remove <= bytes.len(),
            "edit at {at} removing {remove} reaches past the input"
        );
        out.extend_from_slice(&bytes[cursor..at]);
        out.extend_from_slice(insert);
        cursor = at + remove;
    }
    out.extend_from_slice(&bytes[cursor..]);
    out
}

/// What a row's mutated input must give.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Want {
    /// The decoder must refuse with this exact error.
    Refused(CodecError),
    /// `Accepted(m)`: the mutated input is its own canonical form.
    Same,
    /// `Accepted(f)`: the canonical re-encoding is the original fixture.
    Orig,
}

/// One named edit of one or more fixtures.
pub struct Row {
    /// The row's name (its table key).
    pub id: &'static str,
    /// The fixtures it applies to, in table order.
    pub fixtures: Vec<&'static str>,
    /// The edits (ascending by `at`, into the unedited fixture).
    pub edits: Vec<(usize, usize, Vec<u8>)>,
    /// What the mutated input must give.
    pub want: Want,
}

/// Builds a [`Row`].
pub fn row(
    id: &'static str,
    fixtures: &[&'static str],
    edits: &[(usize, usize, &[u8])],
    want: Want,
) -> Row {
    Row {
        id,
        fixtures: fixtures.to_vec(),
        edits: edits.iter().map(|&(a, r, b)| (a, r, b.to_vec())).collect(),
        want,
    }
}

/// Asserts `got` against `row.want` for `original`/`mutated`; the message names
/// `who`, `row.id` and both byte strings (`{:02X?}`).
pub fn assert_row(who: &str, row: &Row, original: &[u8], mutated: &[u8], got: &Verdict) {
    let expected = match &row.want {
        Want::Refused(error) => Verdict::Refused(*error),
        Want::Same => Verdict::Accepted(mutated.to_vec()),
        Want::Orig => Verdict::Accepted(original.to_vec()),
    };
    assert_eq!(
        got, &expected,
        "{who} row {}: original {:02X?}, mutated {:02X?}",
        row.id, original, mutated
    );
}

/// Marsaglia xorshift64 (shifts 13, 7, 17), wrapping `u64` arithmetic:
/// `state ^= state << 13; state ^= state >> 7; state ^= state << 17;` then
/// returns the state.
pub struct XorShift64(pub u64);

impl XorShift64 {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// The fixed seed of the WA13/WB11/WC13/WD13 noise runs.
pub const SEED: u64 = 0x5EED_0000_0000_0042;

/// Rule of WA12: `Refused(_)`, or `Accepted(c)` with `verdict(&c) == Accepted(c)`.
pub fn stable(verdict: &dyn Fn(&[u8]) -> Verdict, bytes: &[u8]) -> bool {
    match verdict(bytes) {
        Verdict::Refused(_) => true,
        Verdict::Accepted(canonical) => verdict(&canonical) == Verdict::Accepted(canonical),
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Counts {
    pub refused: usize,
    pub exact: usize,
    pub alias: usize,
    pub shorter: usize,
}

/// Reading 4's rule over every index `i < original.len()` and every value
/// `v != original[i]`: with `m = with_byte(original, i, v)`, `verdict(&m)` is
/// (1) `Refused(e)` with `e != FrameTooLarge` (`refused`), or (2) `Accepted(m)`
/// (`exact`), or (3) `Accepted(c)` with `i` inside one of `ulid_text`,
/// `c.len() == m.len()` and `c[j] == m[j]` for every `j != i` (`alias`), or (4)
/// `Accepted(c)` with `c.len() < m.len()` and `verdict(&c) == Accepted(c)`
/// (`shorter`). Anything else panics with `who`, `i`, `v`, `m` and the verdict.
pub fn check_corruption(
    who: &str,
    verdict: &dyn Fn(&[u8]) -> Verdict,
    original: &[u8],
    ulid_text: &[Range<usize>],
) -> Counts {
    let mut counts = Counts::default();
    for (i, &original_byte) in original.iter().enumerate() {
        for v in 0..=255u8 {
            if v == original_byte {
                continue;
            }
            let mutated = with_byte(original, i, v);
            match verdict(&mutated) {
                Verdict::Refused(CodecError::FrameTooLarge) => panic!(
                    "{who}: byte {i} := {v:02X} reported FrameTooLarge: {:02X?}",
                    mutated
                ),
                Verdict::Refused(_) => counts.refused += 1,
                Verdict::Accepted(canonical) => {
                    if canonical == mutated {
                        counts.exact += 1;
                    } else if ulid_text.iter().any(|range| range.contains(&i))
                        && canonical.len() == mutated.len()
                        && canonical
                            .iter()
                            .zip(mutated.iter())
                            .enumerate()
                            .all(|(j, (&a, &b))| j == i || a == b)
                    {
                        counts.alias += 1;
                    } else if canonical.len() < mutated.len()
                        && verdict(&canonical) == Verdict::Accepted(canonical.clone())
                    {
                        counts.shorter += 1;
                    } else {
                        panic!(
                            "{who}: byte {i} := {v:02X} re-encoded unstably: {:02X?} -> {:02X?}",
                            mutated, canonical
                        );
                    }
                }
            }
        }
    }
    counts
}
