#![forbid(unsafe_code)]
//! Godot-free core of the bridge: errors, the local link and the client half.
//!
//! Nothing here may name the engine, spawn a thread or read a clock. The
//! authority half (`BridgeCore`, `Authority`, the campaign walk) lands with
//! T047-2 and consumes these modules unchanged.

// T047-1 lands the client half ahead of its only non-test consumer, T047-2's
// `BridgeCore`. Until that consumer exists these crate-private items are
// exercised by the unit tests only, so the library target would report every
// one as dead code. Remove this allowance with T047-2.
#[allow(dead_code)]
mod client;
#[allow(dead_code)]
mod errors;
#[allow(dead_code)]
mod link;

pub use client::{SessionView, ViewEvent, ViewReceipt, ViewValue};
pub use errors::BridgeCode;
