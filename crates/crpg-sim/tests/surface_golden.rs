//! Native surface golden verification (T137c): R14.
//!
//! The shared sim-local schedule runs from one source
//! (`support/surface_schedule.rs`, also compiled by the authoring example
//! via `#[path]`), so authoring and verification can never drift apart. The
//! hand-authored value oracle below runs *before* any hash comparison: the
//! rolled initiative keys, each action's margin and each damage total, all
//! written as literal numbers derived from the contract and confirmed from
//! a run. Only then do the per-step `state_hash` lines meet the
//! target-scoped fixture `tests/goldens/surface_v1.<target>.txt` - one
//! `<step> <64-lowercase-hex-hash>` line per step from zero with one final
//! LF. Target selection is literal and compile-time on the two supported
//! native targets under the normal test profile; a missing baseline fails,
//! never passes. Every other target or profile runs the same schedule and
//! oracle plus repeatability checks without reading either baseline.

// The schedule fixture is shared verbatim with the authoring example: one
// source, compiled here and there, so the two can never drift apart.
#[path = "support/surface_schedule.rs"]
mod surface_schedule;

use surface_schedule::{run_golden_schedule, GOLDEN_STEPS};

/// The Windows/MSVC golden fixture selected at compile time.
#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    debug_assertions
))]
const GOLDEN_TARGET: &str = "x86_64-pc-windows-msvc";
/// The Linux/GNU golden fixture selected at compile time.
#[cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu",
    debug_assertions
))]
const GOLDEN_TARGET: &str = "x86_64-unknown-linux-gnu";

/// The hand-authored value oracle: initiative keys, margins and damages.
fn assert_oracle(run: &surface_schedule::GoldenRun) {
    assert_eq!(
        run.initiative_keys,
        vec![33, 23, 22],
        "rolled initiative keys"
    );
    assert_eq!(
        run.margins,
        vec![16, -3, 0, -12, 3, 13],
        "each action's margin"
    );
    assert_eq!(
        run.damages,
        vec![4, 3, 8, 5, 11, 4],
        "each action's damage total"
    );
}

/// Scoped golden comparison.
#[cfg(any(
    all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc",
        debug_assertions
    ),
    all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu",
        debug_assertions
    )
))]
#[test]
fn surface_golden() {
    let run = run_golden_schedule();
    assert_eq!(run.hashes.len(), GOLDEN_STEPS, "one hash per pinned step");
    assert_oracle(&run);

    let path = format!("tests/goldens/surface_v1.{GOLDEN_TARGET}.txt");
    let text = std::fs::read_to_string(&path)
        .expect("the target-scoped golden baseline exists; missing baselines fail");
    assert!(
        text.ends_with('\n') && !text.ends_with("\n\n"),
        "one final LF, no more"
    );
    let mut lines = text.split('\n');
    let trailing = lines.next_back().expect("the final LF splits");
    assert_eq!(trailing, "", "the file ends with exactly one LF");
    let lines: Vec<&str> = lines.collect();
    assert_eq!(lines.len(), run.hashes.len(), "one line per schedule step");
    for (step, line) in lines.iter().enumerate() {
        let (number, hex) = line.split_once(' ').expect("`<step> <hash>`");
        assert_eq!(number, step.to_string(), "steps count from zero");
        assert_eq!(hex.len(), 64, "256-bit hashes in hex");
        assert!(
            hex.bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
            "lowercase hex only"
        );
        let mut rendered = String::with_capacity(64);
        for byte in run.hashes[step] {
            rendered.push_str(&format!("{byte:02x}"));
        }
        assert_eq!(hex, rendered, "step {step} matches its golden hash");
    }
}

/// Portable shape and repeatability.
#[cfg(not(any(
    all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc",
        debug_assertions
    ),
    all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu",
        debug_assertions
    )
)))]
#[test]
fn surface_portable_shape_and_repeatability() {
    let first = run_golden_schedule();
    assert_eq!(first.hashes.len(), GOLDEN_STEPS, "one hash per pinned step");
    assert_oracle(&first);
    let second = run_golden_schedule();
    assert_eq!(
        first.hashes, second.hashes,
        "identical inputs repeat identically"
    );
    assert_eq!(first.initiative_keys, second.initiative_keys);
    assert_eq!(first.margins, second.margins);
    assert_eq!(first.damages, second.damages);
}
