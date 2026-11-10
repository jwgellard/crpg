//! Native history v3 golden verification (T036c, ADR-0036 W4): C-7.
//!
//! The shared sim-local schedule runs from one source
//! (`support/history_v3_schedule.rs`, also compiled by the authoring example
//! via `#[path]`), so authoring and verification can never drift apart. The
//! hand-authored oracle below runs *before* any hash comparison: the exact
//! journal, the wrapper version after every step, both movement pools and C's
//! position, all worked out by hand from the contract. Only then do the
//! per-step `history_hash` lines meet the target-scoped fixture
//! `tests/goldens/history_v3.<target>.txt` - one `<step> <64-lowercase-hex>`
//! line per step from zero with one final LF. Target selection is literal and
//! compile-time on the two supported native targets under the normal test
//! profile; a missing baseline fails, never passes. Every other target or
//! profile runs the same schedule and oracle plus repeatability checks without
//! reading either baseline.

#[allow(dead_code)]
mod support;

// The MOVE fixture is shared with the movement tests and the authoring
// example: one source, compiled here and there.
#[path = "support/movement_schedule.rs"]
mod movement_schedule;

// The schedule is shared verbatim with the authoring example.
#[path = "support/history_v3_schedule.rs"]
mod history_v3_schedule;

use crpg_core::EntityId;
use crpg_sim::{HistoryEnvelope, HistoryEvent, HISTORY_VERSION};
use history_v3_schedule::{GoldenRun, GOLDEN_STEPS};
use movement_schedule::{p, rp};

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

/// One expected journal entry: `(seq, tick, event)`.
fn envelope(seq: u64, tick: u64, payload: HistoryEvent) -> HistoryEnvelope {
    HistoryEnvelope {
        tick: crpg_core::Tick::new(tick),
        seq,
        payload,
    }
}

/// The hand-authored oracle shared by the scoped golden comparison and the
/// portable fallback. Runs before any hash comparison and never reads a
/// baseline.
fn assert_oracle(run: &GoldenRun) {
    let [a, b, c] = run.ids;
    // C registers first (index 0), then the start spawns A and B.
    assert_eq!((c.index(), c.generation()), (0, 1));
    assert_eq!((a.index(), a.generation()), (1, 1));
    assert_eq!((b.index(), b.generation()), (2, 1));

    let spawned = |entity: EntityId| HistoryEvent::Spawned { entity };
    let expected = vec![
        envelope(1, 0, spawned(c)),
        envelope(2, 0, spawned(a)),
        envelope(3, 0, spawned(b)),
        envelope(4, 0, HistoryEvent::TurnStarted { actor: b, round: 0 }),
        envelope(
            5,
            0,
            HistoryEvent::Moved {
                entity: b,
                points: vec![
                    rp(655360, 0, 0),
                    rp(655360, 0, 196608),
                    rp(458752, 0, 196608),
                ],
            },
        ),
        envelope(6, 1, HistoryEvent::TurnStarted { actor: a, round: 0 }),
        envelope(
            7,
            1,
            HistoryEvent::Moved {
                entity: a,
                points: vec![p(0, 0), rp(196608, 0, 0)],
            },
        ),
    ];
    assert_eq!(run.journal, expected, "the journal");

    // The wrapper version: 1 for steps 0-3, 3 from step 4.
    assert_eq!(run.versions.len(), GOLDEN_STEPS);
    for (step, version) in run.versions.iter().enumerate() {
        let want = if step < 4 { 1 } else { 3 };
        assert_eq!(*version, want, "version after step {step}");
    }
    assert_eq!(HISTORY_VERSION, 3);

    // The moves: B pays 4 of 6, A pays 2 of 6.
    assert_eq!(run.outcomes[0].cost, 4);
    assert_eq!(run.outcomes[0].from, p(10, 0));
    assert_eq!(run.outcomes[0].to, p(7, 3));
    assert_eq!(run.outcomes[1].cost, 2);
    assert_eq!(run.outcomes[1].from, p(0, 0));
    assert_eq!(run.outcomes[1].to, p(3, 0));
    assert_eq!(run.b_pool_after_move, 2);
    assert_eq!(run.a_pool_after_move, 4);

    // C keeps its route through the turn-based start and walks 1 m per tick:
    // ticks at steps 5, 8, 9 and 10.
    assert_eq!(run.c_positions.len(), GOLDEN_STEPS);
    for step in 0..=4 {
        assert_eq!(run.c_positions[step], p(0, 10), "C after step {step}");
    }
    assert_eq!(run.c_positions[5], rp(65536, 0, 655360));
    assert_eq!(run.c_positions[6], rp(65536, 0, 655360));
    assert_eq!(run.c_positions[7], rp(65536, 0, 655360));
    assert_eq!(run.c_positions[8], rp(131072, 0, 655360));
    assert_eq!(run.c_positions[9], rp(196608, 0, 655360));
    assert_eq!(run.c_positions[10], p(4, 10));

    // The movers of A and B stand where their moves left them.
    let at = |entity: EntityId| {
        run.world
            .world()
            .movers()
            .get(entity)
            .expect("a mover")
            .position()
    };
    assert_eq!(at(a), p(3, 0));
    assert_eq!(at(b), p(7, 3));
    // The oracle is sensitive: distinct steps hash differently.
    assert_ne!(run.hashes[3], run.hashes[4]);
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
fn history_v3_golden() {
    let run = history_v3_schedule::run_golden_schedule();
    assert_eq!(run.hashes.len(), GOLDEN_STEPS, "one hash per pinned step");
    assert_oracle(&run);

    // The target-scoped golden file: exact lines, exact hex, exact count.
    let path = format!("tests/goldens/history_v3.{GOLDEN_TARGET}.txt");
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
fn history_v3_portable_shape_and_repeatability() {
    let first = history_v3_schedule::run_golden_schedule();
    assert_eq!(first.hashes.len(), GOLDEN_STEPS, "one hash per pinned step");
    assert_oracle(&first);
    let second = history_v3_schedule::run_golden_schedule();
    assert_eq!(
        first.hashes, second.hashes,
        "identical inputs repeat identically"
    );
    assert_eq!(first.journal, second.journal);
    assert_eq!(first.world, second.world);
}
