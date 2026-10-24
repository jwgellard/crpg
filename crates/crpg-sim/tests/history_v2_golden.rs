//! Native history v2 golden verification (T138b, ADR-0029 N5): R20.
//!
//! The shared sim-local schedule runs from one source
//! (`support/history_v2_schedule.rs`, also compiled by the authoring example
//! via `#[path]`), so authoring and verification can never drift apart. The
//! hand-authored event oracle below runs *before* any hash comparison: exact
//! payloads with concrete identities, rounds, modes and ticks, plus
//! drop/duplicate/swap sensitivity. Only then do the per-step hashes meet the
//! target-scoped fixture `tests/goldens/history_v2.<target>.txt` - one
//! `<step> <64-lowercase-hex-hash>` line per step from zero with one final
//! LF. Target selection is literal and compile-time on the two supported
//! native targets under the normal test profile; a missing baseline fails,
//! never passes. Every other target or profile runs the same schedule and
//! oracle plus repeatability checks without reading either baseline.

// The schedule fixture is shared verbatim with the authoring example: one
// source, compiled here and there, so the two can never drift apart.
#[path = "support/history_v2_schedule.rs"]
mod history_v2_schedule;

use crpg_core::Ulid;
use crpg_sim::{EncounterMode, HistoryEvent};
use history_v2_schedule::{GoldenFixture, GoldenRun, GOLDEN_ACK, GOLDEN_STEPS};

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

/// The hand-authored oracle shared by the scoped golden comparison and the
/// portable fallback. Every event is worked out by hand from the contract
/// (B13, B25, B26, H2) and pinned with the world tick it was journaled under.
/// Runs before any hash comparison and never reads a baseline itself.
fn assert_oracle(run: &GoldenRun) {
    let [a, b, c] = run.ids;
    for (index, id) in run.ids.iter().enumerate() {
        assert_eq!(id.index(), index as u32, "arena order is authored order");
        assert_eq!(id.generation(), 1);
    }
    let strike = GoldenFixture::new().strike;
    let finisher = GoldenFixture::new().finisher;
    let encounter = GoldenFixture::new().encounter_id;
    assert_eq!(strike, Ulid::from_u128(303));
    assert_eq!(finisher, Ulid::from_u128(307));
    let resolved = |actor, target, ability, damage| HistoryEvent::ActionResolved {
        actor,
        target,
        ability,
        outcome: "success".to_owned(),
        damage,
    };
    // Two standing strikes land on every due tick: A on B and B on C.
    let pair = |tick: u64| {
        vec![
            (tick, resolved(a, b, strike, 2)),
            (tick, resolved(b, c, strike, 2)),
        ]
    };
    let round = |round| HistoryEvent::RoundStarted { round };
    let mut expected: Vec<(u64, HistoryEvent)> = vec![
        (0, HistoryEvent::Spawned { entity: a }),
        (0, HistoryEvent::Spawned { entity: b }),
        (0, HistoryEvent::Spawned { entity: c }),
    ];
    expected.extend(pair(1));
    expected.extend(pair(3));
    expected.push((4, round(1)));
    expected.extend(pair(5));
    expected.extend(pair(7));
    expected.push((8, round(2)));
    expected.extend(pair(9));
    expected.extend(pair(11));
    // The boundary at tick 12 applies the pending turn-based request.
    expected.push((
        12,
        HistoryEvent::ModeChanged {
            mode: EncounterMode::TurnBased,
            round: 3,
        },
    ));
    expected.push((12, HistoryEvent::TurnStarted { actor: c, round: 3 }));
    // One turn-based round: C, B, then A; A's turn ends the round and the
    // pending real-time request applies at the rollover.
    expected.push((12, resolved(c, a, strike, 2)));
    expected.push((12, HistoryEvent::TurnStarted { actor: b, round: 3 }));
    expected.push((12, resolved(b, a, strike, 2)));
    expected.push((12, HistoryEvent::TurnStarted { actor: a, round: 3 }));
    expected.push((12, resolved(a, b, strike, 2)));
    expected.push((
        12,
        HistoryEvent::ModeChanged {
            mode: EncounterMode::RealTime,
            round: 4,
        },
    ));
    expected.push((12, round(4)));
    // Back in real time: A strikes, C finishes B in the same pass.
    expected.push((13, resolved(a, b, strike, 2)));
    expected.push((13, resolved(c, b, finisher, 99)));
    expected.push((13, HistoryEvent::Died { entity: b }));
    // A's repeating order now names a dead target.
    expected.push((15, HistoryEvent::OrderEnded { actor: a }));
    // The boundary tick: the round starts, A finishes C, the encounter ends.
    expected.push((16, round(5)));
    expected.push((16, resolved(a, c, finisher, 99)));
    expected.push((16, HistoryEvent::Died { entity: c }));
    expected.push((
        16,
        HistoryEvent::EncounterEnded {
            encounter,
            round: 5,
        },
    ));

    let actual: Vec<(u64, HistoryEvent)> = run
        .journal
        .iter()
        .map(|envelope| (envelope.tick.get(), envelope.payload.clone()))
        .collect();
    assert_eq!(actual, expected, "the oracle precedes the hashes");
    assert_eq!(run.journal.len(), 34);
    // The oracle is sensitive: drop, duplication and swap fail it.
    let mut dropped = expected.clone();
    dropped.remove(20);
    assert_ne!(actual, dropped);
    let mut duplicated = expected.clone();
    duplicated.insert(20, expected[20].clone());
    assert_ne!(actual, duplicated);
    let mut swapped = expected.clone();
    swapped.swap(20, 21);
    assert_ne!(actual, swapped);
    // Coordinates: contiguous sequences from one.
    for (index, envelope) in run.journal.iter().enumerate() {
        assert_eq!(envelope.seq, index as u64 + 1);
    }
    // The wrapper upgraded at the RT-capable start and stayed there.
    assert_eq!(run.world.version(), 2);
    // The pinned ack schedule is part of the hashed input.
    assert_eq!(run.world.acknowledged(), GOLDEN_ACK);
    assert_eq!(run.world.pending_len(), 34 - GOLDEN_ACK as usize);
    assert_eq!(run.world.last_sequence(), 34);
    // The natural end left a terminal real-time encounter.
    let combat = run.world.world().combat().expect("terminal, not released");
    assert_eq!(combat.mode, EncounterMode::RealTime);
    assert_eq!(combat.round, 5);
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
fn history_v2_golden() {
    let run = history_v2_schedule::run_golden_schedule();
    assert_eq!(run.hashes.len(), GOLDEN_STEPS, "one hash per pinned step");
    assert_oracle(&run);

    // The target-scoped golden file: exact lines, exact hex, exact count.
    let path = format!("tests/goldens/history_v2.{GOLDEN_TARGET}.txt");
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
fn history_v2_portable_shape_and_repeatability() {
    let first = history_v2_schedule::run_golden_schedule();
    assert_eq!(first.hashes.len(), GOLDEN_STEPS, "one hash per pinned step");
    assert_oracle(&first);
    let second = history_v2_schedule::run_golden_schedule();
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
    assert_eq!(first.journal.len(), 34);
    assert_eq!(first.world, second.world);
}
