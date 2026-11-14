//! The read-only action preview (T143, ADR-0029 U9): exact odds of one
//! ability use, counted rather than sampled.
//!
//! This module holds the counting layer, built first (T143b): [`PreviewCount`],
//! an exact unsigned integer of any size, and the dice distributions with
//! their pinned cost formulas. Every count is an exact integer over a stated
//! total; no float and no rounding appear anywhere in it. The query that
//! uses them arrives in T143c.

mod count;
#[allow(dead_code)] // used by T143c, which removes this allow
mod dist;

pub use count::PreviewCount;
