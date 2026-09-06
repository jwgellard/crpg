//! Hash-sequence harness: [`run_hash_sequence`], golden files, [`Mismatch`].
//!
//! The three operations every behavioural task needs — run a scripted
//! scenario to a hash sequence, write it down, compare against it later —
//! built once, here, so they cannot fork across crates. T009's replay, T016's
//! combat and every golden file after them consume this module; none of them
//! reimplements it.
//!
//! ## The fixed interleaving
//!
//! Every tick runs script step, then `tick`, then `state_hash`, in that
//! order, for every consumer. That is what makes sequences comparable across
//! tasks: a hash at index *n* always means "the world after the *n*-th
//! scripted step and tick." Consumers do not reorder it.
//!
//! ## Scope discipline belongs to CI, not here
//!
//! Golden files carry `#`-comment scope headers this module writes
//! (`std::env::consts` OS/arch) and skips on compare — headers never
//! compare. But the toolchain and profile are not knowable to a library at
//! runtime, so the full ADR-0009 scope (toolchain + platform + profile, in
//! the filename, compared on canonical Linux) is enforced by the CI job
//! (E020's sequencing), not by the compare function. Tolerant comparison —
//! fuzzy hashes, per-platform goldens chosen at runtime — is rejected here
//! on purpose: it would re-admit exactly the ambiguity ADR-0009 removed. Do
//! not "fix" a cross-platform mismatch inside [`verify_golden`]; file it
//! against the CI job.
//!
//! ## Error shape
//!
//! [`verify_golden`] returns `Result<(), HarnessError>`: `Io` for filesystem
//! failures, `Mismatch` for content divergence (including length mismatch,
//! reported at the shorter length, malformed lines, and non-UTF-8 files;
//! missing files surface as `Io` with kind `NotFound`). The two are
//! distinguishable without matching on strings, both call sites stay
//! single-pathed, and neither costs a dependency.
//!
//! [`Mismatch`] is an enum so each side is present only when it exists: a
//! missing or unparsable side is `None`/its own variant, never a zeroed
//! hash (ADR-0010).

use std::fmt;
use std::io::{self, Write};
use std::path::Path;

use crpg_sim::{state_hash, tick, World};

/// One scripted step: mutate the world before the harness ticks it.
///
/// The step sees the world only through the public sim API (`spawn`,
/// stores, `timeline_mut`, `rng_mut`). A script that needs a private seam
/// is proof the sim API is missing something — file it, do not reach
/// around.
pub type ScriptStep = Box<dyn FnMut(&mut World)>;

/// First-diverging-tick report from [`verify_golden`].
///
/// Each variant carries only the sides that exist: absent or unparsable
/// content is `None` or its own variant, never a zeroed hash (ADR-0010).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mismatch {
    /// Same length, different hashes at `tick`.
    Diverged {
        /// Index into the hash sequence where comparison stopped.
        tick: usize,
        /// The approved hash at `tick`.
        expected: [u8; 32],
        /// The hash the run produced at `tick`.
        actual: [u8; 32],
    },
    /// The run produced more hashes than the golden holds.
    GoldenShort {
        /// First index with no approved hash (the golden length).
        tick: usize,
        /// The hash the run produced at `tick`.
        actual: [u8; 32],
    },
    /// The golden holds more hashes than the run produced.
    RunShort {
        /// First index with no produced hash (the run length).
        tick: usize,
        /// The approved hash at `tick`.
        expected: [u8; 32],
    },
    /// A golden line exists but is not 64 lowercase hex chars.
    Malformed {
        /// Index of the bad line among hash lines.
        tick: usize,
        /// The produced hash, when the run reaches `tick`.
        actual: Option<[u8; 32]>,
        /// The raw golden line.
        line: String,
    },
    /// The golden file is not valid UTF-8.
    InvalidUtf8,
}

impl Mismatch {
    /// Index where comparison stopped, when the variant has one.
    pub fn tick(&self) -> Option<usize> {
        match self {
            Mismatch::Diverged { tick, .. }
            | Mismatch::GoldenShort { tick, .. }
            | Mismatch::RunShort { tick, .. }
            | Mismatch::Malformed { tick, .. } => Some(*tick),
            Mismatch::InvalidUtf8 => None,
        }
    }
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mismatch::Diverged {
                tick,
                expected,
                actual,
            } => write!(
                f,
                "diverged at tick {}: expected {}, got {}",
                tick,
                hex(expected),
                hex(actual)
            ),
            Mismatch::GoldenShort { tick, actual } => write!(
                f,
                "run outlived golden at tick {}: got {}, no approved hash",
                tick,
                hex(actual)
            ),
            Mismatch::RunShort { tick, expected } => write!(
                f,
                "golden outlived run at tick {}: expected {}, run ended",
                tick,
                hex(expected)
            ),
            Mismatch::Malformed { tick, actual, line } => match actual {
                Some(hash) => write!(
                    f,
                    "malformed golden at tick {}: got {}, line {line:?}",
                    tick,
                    hex(hash)
                ),
                None => write!(
                    f,
                    "malformed golden at tick {tick}: line {line:?}, run ended"
                ),
            },
            Mismatch::InvalidUtf8 => write!(f, "golden file is not valid UTF-8"),
        }
    }
}

/// Filesystem failure versus content divergence from [`verify_golden`].
#[derive(Debug)]
pub enum HarnessError {
    /// Reading or writing the golden file failed.
    Io(io::Error),
    /// The file read fine; the hashes differ.
    Mismatch(Mismatch),
}

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HarnessError::Io(e) => write!(f, "golden file I/O failed: {e}"),
            HarnessError::Mismatch(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for HarnessError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HarnessError::Io(e) => Some(e),
            HarnessError::Mismatch(_) => None,
        }
    }
}

/// Runs `script` for `ticks` ticks from `seed`, hashing after every tick.
///
/// Builds `World::new(seed)`, then per tick applies the script step, calls
/// `tick`, and records `state_hash`. Fully deterministic for a fixed script:
/// same seed in, same sequence out.
pub fn run_hash_sequence(seed: u64, ticks: usize, mut script: ScriptStep) -> Vec<[u8; 32]> {
    let mut world = World::new(seed);
    (0..ticks)
        .map(|_| {
            script(&mut world);
            tick(&mut world);
            state_hash(&world)
        })
        .collect()
}

/// Writes `hashes` as lowercase hex, one per line, under `#`-comment scope
/// header lines. Creates parent directories. The file ends with a newline.
pub fn write_golden(path: &Path, hashes: &[[u8; 32]]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut file = std::fs::File::create(path)?;
    writeln!(
        file,
        "# scope: {}/{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )?;
    writeln!(file, "# generator: crpg-testkit")?;
    for hash in hashes {
        writeln!(file, "{}", hex(hash))?;
    }
    Ok(())
}

/// Reads a golden file and compares it against `hashes`.
///
/// `#` lines are skipped, so headers never compare. Returns `Ok(())` on a
/// full match, `Err(HarnessError::Mismatch)` at the first diverging tick —
/// a length mismatch reports at the shorter length — and
/// `Err(HarnessError::Io)` when the file cannot be read (a missing golden
/// and a wrong golden stay distinguishable). Non-UTF-8 files are content
/// divergence (`Mismatch::InvalidUtf8`), not I/O failures.
pub fn verify_golden(path: &Path, hashes: &[[u8; 32]]) -> Result<(), HarnessError> {
    let bytes = std::fs::read(path).map_err(HarnessError::Io)?;
    let text =
        String::from_utf8(bytes).map_err(|_| HarnessError::Mismatch(Mismatch::InvalidUtf8))?;
    let mut expected = text.lines().filter(|line| !line.starts_with('#'));
    for (tick, actual) in hashes.iter().enumerate() {
        match expected.next() {
            Some(line) => match unhex(line) {
                Some(baseline) => {
                    if baseline != *actual {
                        return Err(HarnessError::Mismatch(Mismatch::Diverged {
                            tick,
                            expected: baseline,
                            actual: *actual,
                        }));
                    }
                }
                None => {
                    return Err(HarnessError::Mismatch(Mismatch::Malformed {
                        tick,
                        actual: Some(*actual),
                        line: line.to_string(),
                    }));
                }
            },
            // The run outlived the file: length mismatch at the shorter side.
            None => {
                return Err(HarnessError::Mismatch(Mismatch::GoldenShort {
                    tick,
                    actual: *actual,
                }));
            }
        }
    }
    // The file outlived the run: same rule, reported at the shorter length.
    match expected.next() {
        None => Ok(()),
        Some(line) => match unhex(line) {
            Some(baseline) => Err(HarnessError::Mismatch(Mismatch::RunShort {
                tick: hashes.len(),
                expected: baseline,
            })),
            None => Err(HarnessError::Mismatch(Mismatch::Malformed {
                tick: hashes.len(),
                actual: None,
                line: line.to_string(),
            })),
        },
    }
}

/// Lowercase hex encoding. Hand-rolled: a dozen lines, not a dependency.
fn hex(bytes: &[u8; 32]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Parses exactly 64 lowercase hex chars into 32 bytes. Anything else —
/// wrong length, uppercase, non-hex, surrounding whitespace — is `None`,
/// which the caller reports as a content mismatch at that tick.
fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return None;
    }
    // Length is verified above, so every 2-byte window below is in bounds.
    let bytes = text.as_bytes();
    let mut out = [0u8; 32];
    for i in 0..32 {
        let pair = std::str::from_utf8(&bytes[2 * i..2 * i + 2]).ok()?;
        out[i] = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}
