//! Integer length helpers (ADR-0036 U4).

use crpg_core::Fx16_16;

/// `ceil(sqrt(n))`: `s + (s * s < n) as u128` with `s = n.isqrt()`.
pub fn ceil_sqrt(n: u128) -> u128 {
    let s = n.isqrt();
    s + u128::from(s * s < n)
}

/// Horizontal U4 length: `ceil_sqrt(dx^2 + dz^2)` over raw values.
///
/// `y` is ignored. The squares are taken in `u128`, so no input overflows,
/// and the result always fits `u64` (the radicand is below 2^66).
pub fn seg_len(a: [Fx16_16; 3], b: [Fx16_16; 3]) -> u64 {
    let dx = u128::from((i64::from(a[0].to_raw()) - i64::from(b[0].to_raw())).unsigned_abs());
    let dz = u128::from((i64::from(a[2].to_raw()) - i64::from(b[2].to_raw())).unsigned_abs());
    // `ceil_sqrt` of a value below 2^66 is below 2^33, so the cast is lossless.
    ceil_sqrt(dx * dx + dz * dz) as u64
}
