#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Test utilities shared across crates: deterministic harnesses and golden
//! files first, fixtures and contract conformance suites as their tasks
//! arrive. Each addition names its task in its module docs; the crate grows
//! by named need, not by anticipation.

pub mod harness;

pub use harness::{
    run_hash_sequence, verify_golden, write_golden, HarnessError, Mismatch, ScriptStep,
};
