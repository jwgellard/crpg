//! Versioned replay format, validation, and playback: [`Replay`],
//! [`play_replay`], [`play_and_verify`], [`ReplayError`].
//!
//! A replay turns behaviour into a regression-testable artifact: a
//! deterministic seed plus an ordered input list plays to a hash sequence,
//! tick for tick, and that sequence compares against a checked-in golden
//! with an exact-tick report on divergence. T009b's `crpgc replay` and
//! T016's headless combat consume this module; neither reimplements it.
//!
//! ## Payloads are opaque; ordering is not
//!
//! Testkit never interprets a payload. Each [`ReplayInput`] carries a
//! `serde_json::Value` that the caller's [`ApplyInput`] maps onto public
//! `World` operations — and that mapping lives with the caller, because
//! concrete player intents do not exist yet and a speculative command enum
//! here is how game churn reaches the harness. What testkit owns is
//! everything around the payload: the file schema, the validation
//! invariants, the input → tick → hash interleaving, and the divergence
//! report.
//!
//! ## The fixed interleaving
//!
//! Playback is the T008b interleaving with inputs in place of the script
//! step. For every tick `t` in `0..total_ticks`: apply every input scheduled
//! for `t` in file order, call `tick`, call `state_hash`, record. A replay
//! hash at index *n* therefore means exactly what a harness hash at index
//! *n* means — the world after the *n*-th input step and tick — so replay
//! sequences and harness sequences stay comparable. Consumers do not reorder
//! it.
//!
//! ## Scope discipline belongs to CI, not here
//!
//! Same replay in, same sequence out — over the exact build (ADR-0009).
//! Cross-platform and cross-build sameness are explicitly not owed, so the
//! independent Windows/MSVC and Linux/GNU golden comparisons are selected at
//! compile time (ADR-0012; see `tests/replay_golden.rs`), never by runtime platform
//! detection or tolerant comparison. Do not "fix" a cross-platform mismatch
//! inside [`play_and_verify`]; file it against the CI job.
//!
//! ## Error shape
//!
//! [`ReplayError`] keeps every failure mode distinguishable without string
//! matching: `Io` for filesystem failures (a missing replay or golden is an
//! I/O failure, not a divergence), `Malformed` / `UnsupportedVersion` /
//! `UnorderedSchedule` / `OutOfRange` / `TooLarge` for rejected replay data,
//! `ApplyFailed` for a caller payload failure, and `Divergence` for content
//! divergence carrying the truthful [`Mismatch`] sides (ADR-0010). The hash
//! golden format and comparison are the T008b [`verify_golden`](crate::verify_golden)
//! semantics, reused — not reinvented — with replay identity attached.

use std::fmt;
use std::io;
use std::path::Path;

use crpg_sim::{state_hash, tick, World};

use crate::{verify_golden, HarnessError, Mismatch};

/// Replay format version this crate reads and writes.
///
/// Files carrying any other version fail with
/// [`ReplayError::UnsupportedVersion`], never with a parse error or a silent
/// reinterpretation.
pub const REPLAY_FORMAT_VERSION: u32 = 1;

/// Largest `total_ticks` accepted.
///
/// Absurd files are rejected before playback; this is a CI-practicality cap,
/// not a performance claim, and a later task changes it only with a stated
/// reason.
pub const MAX_REPLAY_TICKS: u64 = 1_000_000;

/// Largest input list accepted.
///
/// A file may schedule many inputs per tick, but an unbounded list is an
/// unbounded allocation before the first validation result. Same standing as
/// [`MAX_REPLAY_TICKS`].
pub const MAX_REPLAY_INPUTS: usize = 1_000_000;

/// One scheduled input: applied before the tick it names, in file order
/// among same-tick inputs.
///
/// The payload is opaque to testkit — see the module docs. The caller owns
/// what a payload means; testkit owns when it is applied.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReplayInput {
    /// Tick the payload is applied before (`< total_ticks`).
    pub tick: u64,
    /// Caller-interpreted deterministic payload.
    pub payload: serde_json::Value,
}

/// A recorded run: deterministic seed, campaign/engine identity, tick count,
/// and the ordered input list.
///
/// The only way to build a `Replay` that has not been validated is to
/// deserialize one — and every entry point that consumes one
/// ([`play_replay`], [`play_and_verify`], [`read_replay`]) validates before
/// use. Prefer [`Replay::new`], which validates up front.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Replay {
    /// Must equal [`REPLAY_FORMAT_VERSION`].
    pub format_version: u32,
    /// Seed passed to `World::new` at playback.
    pub seed: u64,
    /// Campaign identity, recorded never enforced. Non-empty.
    pub campaign_id: String,
    /// Campaign version, recorded never enforced. Non-empty.
    pub campaign_version: String,
    /// Engine version, recorded never enforced. Non-empty. Playback does not
    /// compare it against the running build: an old replay must still parse
    /// and play, and version-equality would turn every release into a replay
    /// migration.
    pub engine_version: String,
    /// Exact tick count played, including trailing ticks after the final
    /// input. `<= MAX_REPLAY_TICKS`.
    pub total_ticks: u64,
    /// Inputs in non-decreasing tick order; same-tick entries keep file
    /// order. Length `<= MAX_REPLAY_INPUTS`.
    pub inputs: Vec<ReplayInput>,
}

impl Replay {
    /// Builds a replay, running the full [`validate_replay`] check.
    ///
    /// Fails with the same typed errors playback would fail with, so a
    /// constructed replay that passes here plays without a validation
    /// failure (payload application itself can still fail per input).
    pub fn new(
        format_version: u32,
        seed: u64,
        campaign_id: String,
        campaign_version: String,
        engine_version: String,
        total_ticks: u64,
        inputs: Vec<ReplayInput>,
    ) -> Result<Self, ReplayError> {
        let replay = Self {
            format_version,
            seed,
            campaign_id,
            campaign_version,
            engine_version,
            total_ticks,
            inputs,
        };
        validate_replay(&replay)?;
        Ok(replay)
    }
}

/// Caller-supplied payload application: testkit owns ordering and hashing,
/// the caller owns what a payload *means*.
///
/// Runs through public `World` APIs only — an input needing a private seam
/// proves the sim API is missing something (file it, do not reach around).
/// Returning `Err(reason)` aborts playback with
/// [`ReplayError::ApplyFailed`] at that tick and input index, carrying the
/// reason verbatim.
pub type ApplyInput = Box<dyn FnMut(&mut World, &serde_json::Value) -> Result<(), String>>;

/// First-diverging-tick report from [`play_and_verify`].
///
/// Replay identity plus the truthful [`Mismatch`] — never a fabricated
/// world-state diff. A hash-only golden carries no expected world state, so
/// there is no expected-vs-actual state diff to print; identity, exact tick,
/// and the present hash sides are the whole honest report (ADR-0010).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayDivergence {
    /// The replay's campaign id.
    pub campaign_id: String,
    /// The replay's campaign version.
    pub campaign_version: String,
    /// The replay's engine version.
    pub engine_version: String,
    /// The replay's seed.
    pub seed: u64,
    /// The golden comparison failure, with only-present sides.
    pub mismatch: Mismatch,
}

impl ReplayDivergence {
    /// First divergent tick, when the mismatch variant has one (`None` for
    /// [`Mismatch::InvalidUtf8`]).
    pub fn tick(&self) -> Option<usize> {
        self.mismatch.tick()
    }
}

impl fmt::Display for ReplayDivergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "replay '{}' v{} (engine {}, seed {}): {}",
            self.campaign_id, self.campaign_version, self.engine_version, self.seed, self.mismatch
        )
    }
}

/// Every replay failure mode, distinguishable without string matching.
///
/// `Display` is single-pathed per variant; both call sites stay
/// single-pathed on this enum, and neither costs a dependency.
#[derive(Debug)]
pub enum ReplayError {
    /// Either file could not be read or written. A missing replay or a
    /// missing golden surfaces here (with kind `NotFound`) — missing
    /// baseline and wrong baseline stay distinct variants on purpose.
    Io(io::Error),
    /// Not JSON, not UTF-8, or missing/malformed required metadata. An empty
    /// campaign or engine identity counts as malformed. Carries detail.
    Malformed(String),
    /// `format_version` is not [`REPLAY_FORMAT_VERSION`].
    UnsupportedVersion {
        /// The version the replay carries.
        found: u32,
    },
    /// `inputs[i].tick < inputs[i-1].tick`. Equal same-tick entries are
    /// valid and keep file order.
    UnorderedSchedule {
        /// Index of the offending input in file order.
        index: usize,
        /// Its tick.
        tick: u64,
        /// The previous input's tick.
        prev_tick: u64,
    },
    /// `inputs[i].tick >= total_ticks`.
    OutOfRange {
        /// Index of the offending input in file order.
        index: usize,
        /// Its tick.
        tick: u64,
        /// The replay's declared tick count.
        total_ticks: u64,
    },
    /// `total_ticks` or `inputs.len()` exceeds its `MAX_` bound. A JSON
    /// number overflowing its target integer fails earlier, at parse time,
    /// as `Malformed`; this variant is for in-range-but-impossible counts.
    TooLarge {
        /// Which bound tripped: `"total_ticks"` or `"inputs"`.
        what: &'static str,
        /// The offending value.
        value: u64,
    },
    /// The caller's apply function failed. Carries the tick, the input index
    /// in file order, and the caller's reason verbatim.
    ApplyFailed {
        /// Tick whose input failed.
        tick: u64,
        /// Index of the failing input in file order.
        index: usize,
        /// The reason the apply function returned.
        reason: String,
    },
    /// Playback succeeded; the golden comparison diverged. Boxed: the
    /// identity strings would otherwise push this error over clippy's
    /// `result_large_err` threshold.
    Divergence(Box<ReplayDivergence>),
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplayError::Io(e) => write!(f, "replay file I/O failed: {e}"),
            ReplayError::Malformed(detail) => write!(f, "malformed replay: {detail}"),
            ReplayError::UnsupportedVersion { found } => write!(
                f,
                "unsupported replay format version {found}: this crate reads version {REPLAY_FORMAT_VERSION}"
            ),
            ReplayError::UnorderedSchedule { index, tick, prev_tick } => write!(
                f,
                "replay inputs out of order at index {index}: tick {tick} follows tick {prev_tick}"
            ),
            ReplayError::OutOfRange { index, tick, total_ticks } => write!(
                f,
                "replay input at index {index} names tick {tick} outside 0..{total_ticks}"
            ),
            ReplayError::TooLarge { what, value } => {
                write!(f, "replay {what} count {value} exceeds its bound")
            }
            ReplayError::ApplyFailed { tick, index, reason } => write!(
                f,
                "replay input at tick {tick} (index {index}) failed to apply: {reason}"
            ),
            ReplayError::Divergence(d) => write!(f, "{d}"),
        }
    }
}

impl std::error::Error for ReplayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ReplayError::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// Validates without playing.
///
/// Check order is fixed so one malformed replay always reports one error:
/// version, then metadata, then counts, then the per-input schedule scan in
/// file order (first failing index wins).
pub fn validate_replay(replay: &Replay) -> Result<(), ReplayError> {
    if replay.format_version != REPLAY_FORMAT_VERSION {
        return Err(ReplayError::UnsupportedVersion {
            found: replay.format_version,
        });
    }
    if replay.campaign_id.is_empty() {
        return Err(ReplayError::Malformed(
            "campaign_id must not be empty".to_string(),
        ));
    }
    if replay.campaign_version.is_empty() {
        return Err(ReplayError::Malformed(
            "campaign_version must not be empty".to_string(),
        ));
    }
    if replay.engine_version.is_empty() {
        return Err(ReplayError::Malformed(
            "engine_version must not be empty".to_string(),
        ));
    }
    if replay.total_ticks > MAX_REPLAY_TICKS {
        return Err(ReplayError::TooLarge {
            what: "total_ticks",
            value: replay.total_ticks,
        });
    }
    if replay.inputs.len() > MAX_REPLAY_INPUTS {
        return Err(ReplayError::TooLarge {
            what: "inputs",
            value: replay.inputs.len() as u64,
        });
    }
    let mut prev_tick: Option<u64> = None;
    for (index, input) in replay.inputs.iter().enumerate() {
        if let Some(prev) = prev_tick {
            if input.tick < prev {
                return Err(ReplayError::UnorderedSchedule {
                    index,
                    tick: input.tick,
                    prev_tick: prev,
                });
            }
        }
        if input.tick >= replay.total_ticks {
            return Err(ReplayError::OutOfRange {
                index,
                tick: input.tick,
                total_ticks: replay.total_ticks,
            });
        }
        prev_tick = Some(input.tick);
    }
    Ok(())
}

/// Plays `replay` to a hash sequence.
///
/// Builds `World::new(seed)`, validates first, then per tick applies every
/// input scheduled for that tick in file order, calls `tick`, and records
/// `state_hash`. Fully deterministic for a fixed replay and apply function:
/// same replay in, same sequence out (exact-build scope, ADR-0009).
///
/// A non-finite float installed by the apply function panics inside
/// `state_hash` by sim design — that panic propagates, it is not a
/// `ReplayError`. Do not install one; fix the producer.
pub fn play_replay(replay: &Replay, mut apply: ApplyInput) -> Result<Vec<[u8; 32]>, ReplayError> {
    validate_replay(replay)?;
    let mut world = World::new(replay.seed);
    let mut hashes = Vec::with_capacity(replay.total_ticks as usize);
    // Inputs arrive in non-decreasing tick order (validated above), so one
    // cursor walks them while the tick loop advances — no regrouping pass
    // that could reorder same-tick entries.
    let mut cursor = 0usize;
    for _ in 0..replay.total_ticks {
        // `World::tick` reports the stamped counter (born 0, +1 per `tick`
        // call), so the count of ticks already played is exactly the counter
        // value. Inputs named `t` apply while the counter still reads `t`,
        // before the call that stamps `t + 1`.
        while cursor < replay.inputs.len() && replay.inputs[cursor].tick == world.tick().get() {
            let input = &replay.inputs[cursor];
            apply(&mut world, &input.payload).map_err(|reason| ReplayError::ApplyFailed {
                tick: input.tick,
                index: cursor,
                reason,
            })?;
            cursor += 1;
        }
        tick(&mut world);
        hashes.push(state_hash(&world));
    }
    Ok(hashes)
}

/// Writes `replay` as pretty JSON plus a trailing newline. Creates parent
/// directories.
///
/// Byte-stable: writing the same replay twice, or reading and re-writing
/// it, yields identical bytes (pinned by test, not by assertion).
pub fn write_replay(path: &Path, replay: &Replay) -> Result<(), ReplayError> {
    validate_replay(replay)?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(ReplayError::Io)?;
        }
    }
    let mut text = serde_json::to_string_pretty(replay)
        .map_err(|e| ReplayError::Malformed(format!("replay serialization failed: {e}")))?;
    text.push('\n');
    std::fs::write(path, text).map_err(ReplayError::Io)?;
    Ok(())
}

/// Reads and validates a `.replay` file.
///
/// A missing file is `ReplayError::Io` with kind `NotFound`; non-UTF-8 bytes
/// or bad JSON (including a missing required field or a number overflowing
/// its target integer) is `Malformed`; semantic violations are their typed
/// variants from [`validate_replay`].
pub fn read_replay(path: &Path) -> Result<Replay, ReplayError> {
    let bytes = std::fs::read(path).map_err(ReplayError::Io)?;
    let text = String::from_utf8(bytes)
        .map_err(|e| ReplayError::Malformed(format!("replay file is not valid UTF-8: {e}")))?;
    let replay: Replay = serde_json::from_str(&text)
        .map_err(|e| ReplayError::Malformed(format!("replay JSON malformed: {e}")))?;
    validate_replay(&replay)?;
    Ok(replay)
}

/// End-to-end gate: reads and validates the replay, plays it, and compares
/// the sequence against the golden with the T008b [`verify_golden`]
/// semantics.
///
/// Returns the played sequence on full match. A content divergence becomes
/// [`ReplayError::Divergence`] with the replay identity attached; a missing
/// or unreadable file stays [`ReplayError::Io`] — missing baseline and wrong
/// baseline remain distinguishable without string matching.
pub fn play_and_verify(
    replay_path: &Path,
    golden_path: &Path,
    apply: ApplyInput,
) -> Result<Vec<[u8; 32]>, ReplayError> {
    let replay = read_replay(replay_path)?;
    let hashes = play_replay(&replay, apply)?;
    match verify_golden(golden_path, &hashes) {
        Ok(()) => Ok(hashes),
        Err(HarnessError::Io(e)) => Err(ReplayError::Io(e)),
        Err(HarnessError::Mismatch(mismatch)) => {
            Err(ReplayError::Divergence(Box::new(ReplayDivergence {
                campaign_id: replay.campaign_id.clone(),
                campaign_version: replay.campaign_version.clone(),
                engine_version: replay.engine_version.clone(),
                seed: replay.seed,
                mismatch,
            })))
        }
    }
}
