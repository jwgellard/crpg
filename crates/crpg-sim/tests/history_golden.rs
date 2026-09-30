//! Native history golden verification (T020, ADR-0017).
//!
//! The shared sim-local schedule runs from one source
//! (`support/history_schedule.rs`, also compiled by the authoring example
//! via `#[path]`), so authoring and verification can never drift apart. The
//! hand-authored event oracle below runs *before* any hash comparison:
//! exact payloads with concrete identities, rounds, symbolic outcomes, and
//! damage, plus drop/duplicate/swap sensitivity. Only then do the per-step
//! hashes meet the target-scoped fixture
//! `tests/goldens/history_v1.<target>.txt` — one
//! `<step> <64-lowercase-hex-hash>` line per step from zero with one final
//! LF. Target selection is literal and compile-time on the two supported
//! native targets under the normal test profile; a missing baseline fails,
//! never passes. Every other target or profile runs the same schedule and
//! oracle plus repeatability checks without reading either baseline.

// The schedule fixture is shared verbatim with the authoring example: one
// source, compiled here and there, so the two can never drift apart.
#[path = "support/history_schedule.rs"]
mod history_schedule;

use crpg_core::Ulid;
use crpg_sim::HistoryEvent;
use history_schedule::{GoldenFixture, GoldenRun, GOLDEN_ACK};

/// The Windows/MSVC golden fixture selected at compile time.
///
/// Full target and profile scope per ADR-0012: x86-64 Windows/MSVC under the
/// normal test profile only. Other Windows builds fall through to the
/// portable shape/repeatability check below and never read this baseline.
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
/// normal test profile only. Other Linux builds fall through to the
/// portable shape/repeatability check below and never read this baseline.
#[cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu",
    debug_assertions
))]
const GOLDEN_TARGET: &str = "x86_64-unknown-linux-gnu";

/// The hand-authored oracle shared by the scoped golden comparison and
/// the portable fallback: allocation order, turn/terminal transitions,
/// symbolic outcomes, reported damage, coordinates, and ack schedule.
/// Runs before any hash comparison and never reads a baseline itself.
fn assert_oracle(run: &GoldenRun) {
    // The hand-authored oracle: allocation order, turn/terminal transitions,
    // symbolic outcomes, and reported damage — all literal.
    let spawned: Vec<_> = run
        .journal
        .iter()
        .filter_map(|envelope| match envelope.payload {
            HistoryEvent::Spawned { entity } => Some(entity),
            _ => None,
        })
        .collect();
    assert_eq!(spawned.len(), 6, "two encounters of three");
    for (index, id) in spawned.iter().enumerate() {
        assert_eq!(id.index(), index as u32, "arena order is authored order");
        assert_eq!(id.generation(), 1);
    }
    let (a, b, c, a2, b2, c2) = (
        spawned[0], spawned[1], spawned[2], spawned[3], spawned[4], spawned[5],
    );
    let probe = Ulid::from_u128(203);
    let lethal = Ulid::from_u128(207);
    let encounter = GoldenFixture::new().encounter_id;
    let expected = vec![
        HistoryEvent::Spawned { entity: a },
        HistoryEvent::Spawned { entity: b },
        HistoryEvent::Spawned { entity: c },
        HistoryEvent::TurnStarted { actor: c, round: 0 },
        HistoryEvent::ActionResolved {
            actor: c,
            target: a,
            ability: probe,
            outcome: "success".to_owned(),
            damage: 2,
        },
        HistoryEvent::ActionResolved {
            actor: c,
            target: b,
            ability: probe,
            outcome: "failure".to_owned(),
            damage: 0,
        },
        HistoryEvent::TurnStarted { actor: b, round: 0 },
        HistoryEvent::ActionResolved {
            actor: b,
            target: a,
            ability: probe,
            outcome: "failure".to_owned(),
            damage: 0,
        },
        HistoryEvent::TurnStarted { actor: a, round: 0 },
        HistoryEvent::TurnStarted { actor: c, round: 1 },
        HistoryEvent::ActionResolved {
            actor: c,
            target: b,
            ability: lethal,
            outcome: "success".to_owned(),
            damage: 99,
        },
        HistoryEvent::Died { entity: b },
        HistoryEvent::TurnStarted { actor: a, round: 1 },
        HistoryEvent::ActionResolved {
            actor: a,
            target: c,
            ability: lethal,
            outcome: "success".to_owned(),
            damage: 99,
        },
        HistoryEvent::Died { entity: c },
        HistoryEvent::EncounterEnded {
            encounter,
            round: 1,
        },
        HistoryEvent::Spawned { entity: a2 },
        HistoryEvent::Spawned { entity: b2 },
        HistoryEvent::Spawned { entity: c2 },
        HistoryEvent::TurnStarted {
            actor: c2,
            round: 0,
        },
        HistoryEvent::Despawned { entity: a2 },
        HistoryEvent::Despawned { entity: c2 },
        HistoryEvent::EncounterEnded {
            encounter,
            round: 0,
        },
    ];
    let actual: Vec<_> = run.journal.iter().map(|env| env.payload.clone()).collect();
    assert_eq!(actual, expected, "the oracle precedes the hashes");
    // The oracle is sensitive: same-tick drop, duplication, and swap fail it.
    let mut dropped = expected.clone();
    dropped.remove(10);
    assert_ne!(actual, dropped);
    let mut duplicated = expected.clone();
    duplicated.insert(10, expected[10].clone());
    assert_ne!(actual, duplicated);
    let mut swapped = expected.clone();
    swapped.swap(10, 11);
    assert_ne!(actual, swapped);
    // Coordinates: contiguous sequences from one, everything at tick zero —
    // the tick step journals nothing.
    assert_eq!(run.journal.len(), 23);
    for (index, envelope) in run.journal.iter().enumerate() {
        assert_eq!(envelope.seq, index as u64 + 1);
        assert_eq!(envelope.tick.get(), 0);
    }
    // The pinned ack schedule is part of the hashed input.
    assert_eq!(run.world.acknowledged(), GOLDEN_ACK);
    assert_eq!(run.world.pending_len(), 23 - GOLDEN_ACK as usize);
    assert_eq!(run.world.last_sequence(), 23);
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
fn history_golden() {
    let run = history_schedule::run_golden_schedule();
    assert_eq!(run.hashes.len(), 18, "one hash per pinned step");
    assert_oracle(&run);

    // The target-scoped golden file: exact lines, exact hex, exact count.
    let path = format!("tests/goldens/history_v1.{GOLDEN_TARGET}.txt");
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
fn history_portable_shape_and_repeatability() {
    let first = history_schedule::run_golden_schedule();
    assert_eq!(first.hashes.len(), 18, "one hash per pinned step");
    assert_oracle(&first);
    let second = history_schedule::run_golden_schedule();
    assert_eq!(
        first.hashes, second.hashes,
        "identical inputs repeat identically"
    );
    let first_payloads: Vec<_> = first
        .journal
        .iter()
        .map(|envelope| envelope.payload.clone())
        .collect();
    let second_payloads: Vec<_> = second
        .journal
        .iter()
        .map(|envelope| envelope.payload.clone())
        .collect();
    assert_eq!(first_payloads, second_payloads);
    assert_eq!(first.journal.len(), 23);
    assert_eq!(first.world, second.world);
}
