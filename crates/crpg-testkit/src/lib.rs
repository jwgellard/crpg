#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Test utilities shared across crates: deterministic harnesses, golden
//! files, and versioned replays first, fixtures and contract conformance
//! suites as their tasks arrive. Each addition names its task in its module
//! docs; the crate grows by named need, not by anticipation.

pub mod harness;
pub mod replay;

pub use harness::{
    run_hash_sequence, verify_golden, write_golden, HarnessError, Mismatch, ScriptStep,
};
pub use replay::{
    play_and_verify, play_replay, read_replay, validate_replay, write_replay, ApplyInput, Replay,
    ReplayDivergence, ReplayError, ReplayInput, MAX_REPLAY_INPUTS, MAX_REPLAY_TICKS,
    REPLAY_FORMAT_VERSION,
};
