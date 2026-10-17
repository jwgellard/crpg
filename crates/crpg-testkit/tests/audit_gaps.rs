//! Audit-gap tests (T121): close untested paths named by the 2026-10-16 crate
//! audit. Tests only; no source or fixture changes.

#![forbid(unsafe_code)]

mod support;

use crpg_sim::World;
use crpg_testkit::{
    play_replay, read_replay, run_hash_sequence, verify_golden, write_golden, ApplyInput,
    HarnessError, Mismatch, Replay, ReplayError, ReplayInput, MAX_REPLAY_TICKS,
};
use serde_json::json;

fn remove_silently(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

// #11 — malformed golden line after the run ends carries no produced hash.
#[test]
fn malformed_golden_trailing_line_has_no_produced_hash() {
    let path = support::temp_file("audit-malformed-trailing.golden");
    remove_silently(&path);
    let hashes = run_hash_sequence(0xFACE, 5, Box::new(|_: &mut World| {}));
    write_golden(&path, &hashes).unwrap();
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "zzzz").unwrap();
    }
    match verify_golden(&path, &hashes) {
        Err(HarnessError::Mismatch(Mismatch::Malformed { tick, actual, line })) => {
            assert_eq!(tick, 5);
            assert_eq!(actual, None);
            assert_eq!(line, "zzzz");
        }
        other => panic!("expected trailing malformed line, got {other:?}"),
    }
    remove_silently(&path);
}

// #69 — an extra unknown field is ignored on read and plays identically.
#[test]
fn replay_with_unknown_field_reads_and_plays_identically() {
    let path = support::temp_file("audit-unknown-field.replay");
    remove_silently(&path);
    let replay = support::basic_replay();
    let baseline_hashes = play_replay(&replay, support::test_apply()).unwrap();

    // Top-level unknown field.
    let mut value = serde_json::to_value(&replay).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("extra_unknown".to_string(), json!(42));
    std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap() + "\n").unwrap();
    let read_back = read_replay(&path).unwrap();
    assert_eq!(read_back, replay);
    assert_eq!(
        play_replay(&read_back, support::test_apply()).unwrap(),
        baseline_hashes
    );

    // Input-level unknown field.
    let path2 = support::temp_file("audit-unknown-field-input.replay");
    remove_silently(&path2);
    let mut value2 = serde_json::to_value(&replay).unwrap();
    value2["inputs"][0]
        .as_object_mut()
        .unwrap()
        .insert("bogus".to_string(), json!("x"));
    std::fs::write(
        &path2,
        serde_json::to_string_pretty(&value2).unwrap() + "\n",
    )
    .unwrap();
    let read_back2 = read_replay(&path2).unwrap();
    assert_eq!(read_back2, replay);
    assert_eq!(
        play_replay(&read_back2, support::test_apply()).unwrap(),
        baseline_hashes
    );

    remove_silently(&path);
    remove_silently(&path2);
}

// #70 — identity is recorded, never enforced; differing identity still plays
// and yields the same hashes.
#[test]
fn replay_identity_is_recorded_not_enforced() {
    let base = support::basic_replay();
    let mut other = base.clone();
    other.campaign_id = "other-campaign".to_string();
    other.campaign_version = "9.9.9".to_string();
    other.engine_version = "8.8.8".to_string();
    crpg_testkit::validate_replay(&other).unwrap();
    let base_hashes = play_replay(&base, support::test_apply()).unwrap();
    let other_hashes = play_replay(&other, support::test_apply()).unwrap();
    assert_eq!(base_hashes, other_hashes);
}

// #71 — validation reports the first failure in fixed order: version, metadata,
// counts, schedule scan. One test per adjacent pair.

#[test]
fn validation_reports_version_before_metadata() {
    let bad = Replay {
        format_version: 999,
        campaign_id: String::new(),
        ..support::basic_replay()
    };
    match crpg_testkit::validate_replay(&bad) {
        Err(ReplayError::UnsupportedVersion { found }) => assert_eq!(found, 999),
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }
}

#[test]
fn validation_reports_metadata_before_counts() {
    let bad = Replay {
        campaign_id: String::new(),
        total_ticks: MAX_REPLAY_TICKS + 1,
        ..support::basic_replay()
    };
    match crpg_testkit::validate_replay(&bad) {
        Err(ReplayError::Malformed(_)) => {}
        other => panic!("expected Malformed, got {other:?}"),
    }
}

#[test]
fn validation_reports_counts_before_schedule() {
    let bad = Replay {
        total_ticks: MAX_REPLAY_TICKS + 1,
        inputs: vec![
            ReplayInput {
                tick: 2,
                payload: json!(null),
            },
            ReplayInput {
                tick: 0,
                payload: json!(null),
            },
        ],
        ..support::basic_replay()
    };
    match crpg_testkit::validate_replay(&bad) {
        Err(ReplayError::TooLarge { what, value }) => {
            assert_eq!(what, "total_ticks");
            assert_eq!(value, MAX_REPLAY_TICKS + 1);
        }
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

// #72 — an invalid replay is rejected before its apply callback runs.
#[test]
fn invalid_replay_does_not_invoke_apply() {
    let replay = Replay {
        inputs: vec![ReplayInput {
            tick: 99,
            payload: json!(null),
        }],
        ..support::basic_replay()
    };
    use std::cell::Cell;
    use std::rc::Rc;
    let calls = Rc::new(Cell::new(0usize));
    let calls_for_apply = calls.clone();
    let apply: ApplyInput = Box::new(move |_, _| {
        calls_for_apply.set(calls_for_apply.get() + 1);
        Ok(())
    });
    let result = play_replay(&replay, apply);
    assert!(result.is_err());
    assert_eq!(calls.get(), 0);
}
