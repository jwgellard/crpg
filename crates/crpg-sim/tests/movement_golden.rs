//! Native movement golden verification (T036b, ADR-0036 W1-W3): B-8.
//!
//! The shared sim-local schedule runs from one source
//! (`support/movement_schedule.rs`, also compiled by the authoring example via
//! `#[path]`), so authoring and verification can never drift apart. The
//! hand-authored oracle below runs *before* any hash comparison: the exact
//! position of A, B and C after every tick, the route each still holds, the
//! facts of every tick and the mode, all worked out by hand from the contract.
//! Only then do the per-step `state_hash` lines meet the target-scoped fixture
//! `tests/goldens/movement_v1.<target>.txt` - one `<step> <64-lowercase-hex>`
//! line per step from zero with one final LF. Target selection is literal and
//! compile-time on the two supported native targets under the normal test
//! profile; a missing baseline fails, never passes. Every other target or
//! profile runs the same schedule and oracle plus repeatability checks without
//! reading either baseline.

#[allow(dead_code)]
mod support;

// The schedule fixture is shared verbatim with the authoring example: one
// source, compiled here and there, so the two can never drift apart.
#[path = "support/movement_schedule.rs"]
mod movement_schedule;

use crpg_sim::{EncounterMode, MovementFact, RouteTag};
use movement_schedule::{p, rp, GoldenRun, GOLDEN_STEPS, GOLDEN_TICKS};

/// The Windows/MSVC golden fixture selected at compile time.
///
/// Full target and profile scope per ADR-0012: x86-64 Windows/MSVC under the
/// normal test profile only.
#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    debug_assertions
))]
const GOLDEN_TARGET: &str = "x86_64-pc-windows-msvc";
/// The Linux/GNU golden fixture selected at compile time.
///
/// Full target and profile scope per ADR-0012: x86-64 Linux/GNU under the
/// normal test profile only.
#[cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu",
    debug_assertions
))]
const GOLDEN_TARGET: &str = "x86_64-unknown-linux-gnu";

/// The hand-authored oracle shared by the scoped golden comparison and the
/// portable fallback. Runs before any hash comparison and never reads a
/// baseline.
fn assert_oracle(run: &GoldenRun) {
    let [a, b, c] = run.ids;
    // A = {1, 1}, B = {2, 1}, C = {0, 1}.
    assert_eq!((c.index(), c.generation()), (0, 1));
    assert_eq!((a.index(), a.generation()), (1, 1));
    assert_eq!((b.index(), b.generation()), (2, 1));
    assert_eq!(run.positions.len(), GOLDEN_TICKS);
    assert_eq!(run.routes.len(), GOLDEN_TICKS);
    assert_eq!(run.facts.len(), GOLDEN_TICKS);
    assert_eq!(run.modes.len(), GOLDEN_TICKS);

    // A after ticks 1-5 as B-1, then at rest on p(3, 4).
    let a_path = [
        rp(39321, 0, 52428),
        rp(78643, 0, 104857),
        rp(117964, 0, 157286),
        rp(157286, 0, 209715),
        p(3, 4),
    ];
    // B after ticks 1-7; the tick-8 switch stops it short of p(8, 2).
    let b_path = [
        rp(655360, 0, 32768),
        rp(655360, 0, 65536),
        rp(655360, 0, 98304),
        rp(655360, 0, 131072),
        rp(622592, 0, 131072),
        rp(589824, 0, 131072),
        rp(557056, 0, 131072),
    ];
    for (index, positions) in run.positions.iter().enumerate() {
        let tick = index + 1;
        let want_a = if tick <= 5 { a_path[tick - 1] } else { p(3, 4) };
        let want_b = if tick <= 7 {
            b_path[tick - 1]
        } else {
            rp(557056, 0, 131072)
        };
        let want_c = if tick <= 11 {
            rp(65536 * tick as i32, 0, 655360)
        } else {
            p(12, 10)
        };
        assert_eq!(positions[0], want_a, "A after tick {tick}");
        assert_eq!(positions[1], want_b, "B after tick {tick}");
        assert_eq!(positions[2], want_c, "C after tick {tick}");
    }

    // The routes still held after each tick.
    for (index, routes) in run.routes.iter().enumerate() {
        let tick = index + 1;
        let want_a = (tick < 5).then_some(1);
        let want_b = (tick < 8).then_some(2);
        let want_c = (tick < 12).then_some(3);
        assert_eq!(
            *routes,
            [want_a, want_b, want_c],
            "routes after tick {tick}"
        );
    }

    // Facts: A arrives at tick 5, C at tick 12, nothing else.
    for (index, facts) in run.facts.iter().enumerate() {
        let tick = index + 1;
        let want: Vec<MovementFact> = match tick {
            5 => vec![MovementFact::Arrived {
                entity: a,
                tag: RouteTag(1),
            }],
            12 => vec![MovementFact::Arrived {
                entity: c,
                tag: RouteTag(3),
            }],
            _ => Vec::new(),
        };
        assert_eq!(*facts, want, "facts of tick {tick}");
    }

    // The switch applies at the tick-8 boundary.
    for (index, mode) in run.modes.iter().enumerate() {
        let tick = index + 1;
        let want = if tick < 8 {
            EncounterMode::RealTime
        } else {
            EncounterMode::TurnBased
        };
        assert_eq!(*mode, want, "mode after tick {tick}");
    }
    assert_eq!(run.world.nav(), Some(movement_schedule::golden_revision()));
    // The oracle is sensitive: a shifted position fails it.
    assert_ne!(run.positions[0][0], run.positions[1][0]);
}

/// Scoped golden comparison: only on the two supported native targets under
/// the normal test profile. The oracle runs first; only then do the per-step
/// hashes meet the target-scoped fixture. A missing baseline fails.
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
fn movement_golden() {
    let run = movement_schedule::run_golden_schedule();
    assert_eq!(run.hashes.len(), GOLDEN_STEPS, "one hash per pinned step");
    assert_oracle(&run);

    // The target-scoped golden file: exact lines, exact hex, exact count.
    let path = format!("tests/goldens/movement_v1.{GOLDEN_TARGET}.txt");
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

/// Portable shape and repeatability: every other target or profile runs the
/// same schedule and oracle plus identical-input repeatability, without
/// reading either scoped baseline.
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
fn movement_portable_shape_and_repeatability() {
    let first = movement_schedule::run_golden_schedule();
    assert_eq!(first.hashes.len(), GOLDEN_STEPS, "one hash per pinned step");
    assert_oracle(&first);
    let second = movement_schedule::run_golden_schedule();
    assert_eq!(
        first.hashes, second.hashes,
        "identical inputs repeat identically"
    );
    assert_eq!(first.positions, second.positions);
    assert_eq!(first.world, second.world);
}
