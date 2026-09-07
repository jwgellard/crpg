//! Portable T009a replay tests: format round-trip, playback ordering,
//! validation, typed errors, and golden-divergence behaviour. All run on
//! every platform; independent Windows/MSVC and Linux/GNU comparisons live
//! in `replay_golden.rs` behind compile-time gates (ADR-0012).
//!
//! The tests own their payload vocabulary (see `support/mod.rs`) — those shapes
//! are caller-side fixtures, not crate vocabulary.

#![forbid(unsafe_code)]

mod support;

use crpg_sim::{state_hash, tick, World};
use crpg_testkit::{
    play_and_verify, play_replay, read_replay, run_hash_sequence, write_golden, write_replay,
    Mismatch, Replay, ReplayError, ReplayInput, MAX_REPLAY_INPUTS, MAX_REPLAY_TICKS,
    REPLAY_FORMAT_VERSION,
};
use serde_json::json;
use support::{basic_replay, temp_file, test_apply};

fn remove_silently(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

fn test_replay(seed: u64, total_ticks: u64, inputs: Vec<ReplayInput>) -> Replay {
    Replay::new(
        REPLAY_FORMAT_VERSION,
        seed,
        "testkit-test".to_string(),
        "0.1.0".to_string(),
        "0.1.0".to_string(),
        total_ticks,
        inputs,
    )
    .expect("test replay must validate")
}

fn input(tick: u64, payload: serde_json::Value) -> ReplayInput {
    ReplayInput { tick, payload }
}

#[test]
fn replay_round_trip_has_stable_output() {
    let path = temp_file("replay-roundtrip.replay");
    let nested = temp_file("replay-roundtrip-nested/a/b/c.replay");
    remove_silently(&path);
    let replay = basic_replay();
    write_replay(&path, &replay).unwrap();
    write_replay(&nested, &replay).unwrap();
    let first = std::fs::read(&path).unwrap();
    // Writing the same replay twice yields identical bytes, and nested
    // parents are created by the writer.
    write_replay(&path, &replay).unwrap();
    assert_eq!(first, std::fs::read(&path).unwrap());
    assert_eq!(first, std::fs::read(&nested).unwrap());
    // Reading and re-writing yields identical bytes and an equal value.
    let back = read_replay(&path).unwrap();
    assert_eq!(back, replay);
    let rewritten = temp_file("replay-roundtrip-rewritten.replay");
    write_replay(&rewritten, &back).unwrap();
    assert_eq!(first, std::fs::read(&rewritten).unwrap());
    // Pretty JSON with a trailing newline: reviewable fixture format.
    let text = String::from_utf8(first).unwrap();
    assert!(text.ends_with('\n'));
    assert!(text.contains("\"format_version\": 1"));
    remove_silently(&path);
    remove_silently(&nested);
    remove_silently(&rewritten);
}

#[test]
fn same_replay_plays_identically() {
    let replay = basic_replay();
    let first = play_replay(&replay, test_apply()).unwrap();
    let second = play_replay(&replay, test_apply()).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.len(), replay.total_ticks as usize);
}

#[test]
fn scheduled_inputs_apply_before_tick() {
    // Manual input-before-tick loop must match playback; input-after-tick
    // must not (mirrors the T008b order-pinning test for the script step).
    let replay = basic_replay();
    let played = play_replay(&replay, test_apply()).unwrap();

    let mut world = World::new(replay.seed);
    let mut apply = test_apply();
    let mut script_first = Vec::new();
    for _ in 0..replay.total_ticks {
        let now = world.tick().get();
        for input in replay.inputs.iter().filter(|i| i.tick == now) {
            apply(&mut world, &input.payload).unwrap();
        }
        tick(&mut world);
        script_first.push(state_hash(&world));
    }
    assert_eq!(played, script_first);

    let mut world = World::new(replay.seed);
    let mut apply = test_apply();
    let mut tick_first = Vec::new();
    for _ in 0..replay.total_ticks {
        let now = world.tick().get();
        tick(&mut world);
        for input in replay.inputs.iter().filter(|i| i.tick == now) {
            apply(&mut world, &input.payload).unwrap();
        }
        tick_first.push(state_hash(&world));
    }
    assert_ne!(played, tick_first);
}

#[test]
fn same_tick_inputs_keep_file_order() {
    let first = input(0, json!({"spawn_at": [7, 0, 0]}));
    let second = input(0, json!({"spawn_at": [0, 8, 0]}));
    let in_order = test_replay(11, 3, vec![first.clone(), second.clone()]);
    let swapped = test_replay(11, 3, vec![second, first]);
    let played = play_replay(&in_order, test_apply()).unwrap();

    // Playback equals the manual loop applying same-tick inputs in file
    // order — and file order is observable, because the two spawn positions
    // land on different arena ids depending on who spawns first.
    let mut world = World::new(11);
    let mut apply = test_apply();
    let mut expected = Vec::new();
    for _ in 0..3 {
        let now = world.tick().get();
        for input in in_order.inputs.iter().filter(|i| i.tick == now) {
            apply(&mut world, &input.payload).unwrap();
        }
        tick(&mut world);
        expected.push(state_hash(&world));
    }
    assert_eq!(played, expected);
    assert_ne!(played, play_replay(&swapped, test_apply()).unwrap());
}

#[test]
fn gaps_trailing_and_empty_ticks_replay() {
    // The gate-shaped replay has gap ticks (1, 4) and trailing ticks (6, 7)
    // after the final input at tick 5; all of them hash.
    let replay = basic_replay();
    assert!(replay.inputs.iter().all(|i| i.tick <= 5));
    assert_eq!(replay.total_ticks, 8);
    let played = play_replay(&replay, test_apply()).unwrap();
    assert_eq!(played.len(), 8);

    // A zero-input replay is a plain tick loop — and matches the T008b
    // harness with a no-op script, proving the interleavings coincide.
    let empty = test_replay(0xE11CE, 16, vec![]);
    let played_empty = play_replay(&empty, test_apply()).unwrap();
    let harness_empty = run_hash_sequence(0xE11CE, 16, Box::new(|_| {}));
    assert_eq!(played_empty, harness_empty);

    // Zero ticks is degenerate but valid: an empty sequence, not an error.
    let zero = test_replay(1, 0, vec![]);
    assert_eq!(
        play_replay(&zero, test_apply()).unwrap(),
        Vec::<[u8; 32]>::new()
    );
}

#[test]
fn altered_payload_or_seed_diverges() {
    let replay = basic_replay();
    let baseline = play_replay(&replay, test_apply()).unwrap();

    let mut altered = replay.clone();
    altered.inputs[0].payload = json!({"spawn_at": [9, 9, 9]});
    let changed = play_replay(&altered, test_apply()).unwrap();
    assert_ne!(baseline, changed);
    // The altered input applies before tick 0's hash, so index 0 diverges.
    let first_diff = baseline
        .iter()
        .zip(changed.iter())
        .position(|(a, b)| a != b)
        .unwrap();
    assert_eq!(first_diff, 0);

    let reseeded = test_replay(
        replay.seed.wrapping_add(1),
        replay.total_ticks,
        replay.inputs.clone(),
    );
    assert_ne!(baseline, play_replay(&reseeded, test_apply()).unwrap());
}

#[test]
fn tampered_golden_names_the_exact_tick() {
    let replay_path = temp_file("replay-tamper.replay");
    let golden_path = temp_file("replay-tamper.golden");
    remove_silently(&replay_path);
    remove_silently(&golden_path);
    let replay = basic_replay();
    write_replay(&replay_path, &replay).unwrap();
    let hashes = play_replay(&replay, test_apply()).unwrap();
    write_golden(&golden_path, &hashes).unwrap();
    play_and_verify(&replay_path, &golden_path, test_apply()).unwrap();

    // Flip one hex char in the hash at index 3.
    let text = std::fs::read_to_string(&golden_path).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let target = 2 + 3; // two `#` header lines, then hash lines.
    let mut chars: Vec<char> = lines[target].chars().collect();
    chars[0] = if chars[0] == '0' { '1' } else { '0' };
    lines[target] = chars.into_iter().collect();
    std::fs::write(&golden_path, lines.join("\n") + "\n").unwrap();

    match play_and_verify(&replay_path, &golden_path, test_apply()) {
        Err(ReplayError::Divergence(d)) => {
            assert_eq!(d.campaign_id, "testkit-basic");
            assert_eq!(d.seed, replay.seed);
            assert_eq!(d.tick(), Some(3));
            match &d.mismatch {
                Mismatch::Diverged {
                    tick,
                    expected,
                    actual,
                } => {
                    assert_eq!(*tick, 3);
                    assert_eq!(*actual, hashes[3]);
                    assert_ne!(*expected, *actual);
                }
                other => panic!("expected a tick-3 divergence, got {other:?}"),
            }
            let shown = d.to_string();
            assert!(shown.contains("testkit-basic"), "{shown}");
            assert!(shown.contains("tick 3"), "{shown}");
        }
        other => panic!("expected a tick-3 divergence, got {other:?}"),
    }
    remove_silently(&replay_path);
    remove_silently(&golden_path);
}

#[test]
fn malformed_and_unsupported_replays_are_typed() {
    let dir = temp_file("replay-malformed-dir");
    let _ = std::fs::create_dir_all(&dir);
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    };

    // Not JSON at all.
    let path = write("bad-json.replay", b"{ not json");
    assert!(matches!(read_replay(&path), Err(ReplayError::Malformed(_))));

    // Missing required metadata.
    let path = write(
        "missing-field.replay",
        br#"{"format_version":1,"seed":1,"campaign_version":"0.1.0","engine_version":"0.1.0","total_ticks":1,"inputs":[]}"#,
    );
    assert!(matches!(read_replay(&path), Err(ReplayError::Malformed(_))));

    // Empty identity counts as malformed, not merely unusual.
    let bad = Replay {
        campaign_id: String::new(),
        ..basic_replay()
    };
    assert!(matches!(
        crpg_testkit::validate_replay(&bad),
        Err(ReplayError::Malformed(_))
    ));

    // Wrong format version names the found version.
    let bad = Replay {
        format_version: 999,
        ..basic_replay()
    };
    match crpg_testkit::validate_replay(&bad) {
        Err(ReplayError::UnsupportedVersion { found }) => assert_eq!(found, 999),
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }

    // A negative tick cannot be a u64: parse failure, typed Malformed.
    let path = write(
        "negative-tick.replay",
        br#"{"format_version":1,"seed":1,"campaign_id":"x","campaign_version":"0.1.0","engine_version":"0.1.0","total_ticks":2,"inputs":[{"tick":-1,"payload":null}]}"#,
    );
    assert!(matches!(read_replay(&path), Err(ReplayError::Malformed(_))));

    // A tick overflowing u64 is a parse failure, not a schedule error.
    let path = write(
        "overflow-tick.replay",
        br#"{"format_version":1,"seed":1,"campaign_id":"x","campaign_version":"0.1.0","engine_version":"0.1.0","total_ticks":2,"inputs":[{"tick":18446744073709551616,"payload":null}]}"#,
    );
    assert!(matches!(read_replay(&path), Err(ReplayError::Malformed(_))));

    // Non-UTF-8 replay bytes are malformed content, not an I/O failure.
    let path = write("non-utf8.replay", &[0xff, 0xfe, 0x00, 0x2a]);
    assert!(matches!(read_replay(&path), Err(ReplayError::Malformed(_))));

    // The caller's apply failure aborts with tick, index, and reason.
    let replay = test_replay(5, 2, vec![input(1, json!({"boom": true}))]);
    match play_replay(&replay, test_apply()) {
        Err(ReplayError::ApplyFailed {
            tick,
            index,
            reason,
        }) => {
            assert_eq!((tick, index), (1, 0));
            assert!(reason.contains("unknown test payload"), "{reason}");
        }
        other => panic!("expected ApplyFailed, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unordered_out_of_range_and_huge_schedules_rejected() {
    // Tick equal to the count is outside 0..total_ticks.
    let bad = Replay {
        inputs: vec![input(2, json!(null))],
        ..test_replay(1, 2, vec![])
    };
    match crpg_testkit::validate_replay(&bad) {
        Err(ReplayError::OutOfRange {
            index,
            tick,
            total_ticks,
        }) => {
            assert_eq!((index, tick, total_ticks), (0, 2, 2));
        }
        other => panic!("expected OutOfRange, got {other:?}"),
    }

    // A decreasing pair names the later index and both ticks.
    let bad = Replay {
        inputs: vec![input(2, json!(null)), input(0, json!(null))],
        ..test_replay(1, 8, vec![])
    };
    match crpg_testkit::validate_replay(&bad) {
        Err(ReplayError::UnorderedSchedule {
            index,
            tick,
            prev_tick,
        }) => {
            assert_eq!((index, tick, prev_tick), (1, 0, 2));
        }
        other => panic!("expected UnorderedSchedule, got {other:?}"),
    }

    // Equal same-tick entries are valid (played here with a no-op
    // applier; ordering of real payloads is pinned above).
    let ok = test_replay(1, 8, vec![input(4, json!(null)), input(4, json!(null))]);
    let noop: crpg_testkit::ApplyInput = Box::new(|_, _| Ok(()));
    play_replay(&ok, noop).unwrap();

    // Impossible counts trip their bounds before any schedule scan.
    let bad = Replay {
        total_ticks: MAX_REPLAY_TICKS + 1,
        ..test_replay(1, 1, vec![])
    };
    match crpg_testkit::validate_replay(&bad) {
        Err(ReplayError::TooLarge { what, value }) => {
            assert_eq!(what, "total_ticks");
            assert_eq!(value, MAX_REPLAY_TICKS + 1);
        }
        other => panic!("expected TooLarge, got {other:?}"),
    }
    let bad = Replay {
        inputs: vec![input(0, json!(null)); MAX_REPLAY_INPUTS + 1],
        ..test_replay(1, 1, vec![])
    };
    match crpg_testkit::validate_replay(&bad) {
        Err(ReplayError::TooLarge { what, value }) => {
            assert_eq!(what, "inputs");
            assert_eq!(value, MAX_REPLAY_INPUTS as u64 + 1);
        }
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

#[test]
fn missing_files_stay_distinguishable_from_divergence() {
    let replay_path = temp_file("replay-missing.replay");
    let golden_path = temp_file("replay-missing.golden");
    remove_silently(&replay_path);
    remove_silently(&golden_path);

    // Missing replay: I/O NotFound, before playback is even attempted.
    write_replay(&replay_path, &basic_replay()).unwrap();
    let hashes = play_replay(&basic_replay(), test_apply()).unwrap();
    write_golden(&golden_path, &hashes).unwrap();
    remove_silently(&replay_path);
    match play_and_verify(&replay_path, &golden_path, test_apply()) {
        Err(ReplayError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
        other => panic!("expected NotFound for a missing replay, got {other:?}"),
    }

    // Missing golden: I/O NotFound, not Divergence — "write one" versus
    // "behaviour changed" stay different answers.
    write_replay(&replay_path, &basic_replay()).unwrap();
    remove_silently(&golden_path);
    match play_and_verify(&replay_path, &golden_path, test_apply()) {
        Err(ReplayError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
        other => panic!("expected NotFound for a missing golden, got {other:?}"),
    }

    // Non-UTF-8 golden bytes are content divergence (ADR-0010), with no tick.
    std::fs::write(&golden_path, [0xff, 0xfe, 0x00, 0x2a]).unwrap();
    match play_and_verify(&replay_path, &golden_path, test_apply()) {
        Err(ReplayError::Divergence(d)) => {
            assert_eq!(d.tick(), None);
            assert_eq!(d.mismatch, Mismatch::InvalidUtf8);
        }
        other => panic!("expected InvalidUtf8 divergence, got {other:?}"),
    }
    remove_silently(&replay_path);
    remove_silently(&golden_path);
}

#[test]
fn altered_in_memory_input_fails_usefully() {
    // No production source is touched: the checked-in replay is cloned in
    // memory, altered there, and the failure names the exact tick.
    let replay_path = temp_file("replay-altered.replay");
    let golden_path = temp_file("replay-altered.golden");
    remove_silently(&replay_path);
    remove_silently(&golden_path);
    let replay = read_replay(&support::fixture_replay_path()).unwrap();
    let hashes = play_replay(&replay, test_apply()).unwrap();
    write_golden(&golden_path, &hashes).unwrap();

    let mut altered = replay.clone();
    altered.inputs[1].payload = json!({"spawn_at": [9, 9, 9]});
    write_replay(&replay_path, &altered).unwrap();
    match play_and_verify(&replay_path, &golden_path, test_apply()) {
        Err(ReplayError::Divergence(d)) => {
            // The altered input applies before tick 0's hash.
            assert_eq!(d.tick(), Some(0));
            let shown = d.to_string();
            assert!(shown.contains("testkit-basic"), "{shown}");
            assert!(shown.contains("tick 0"), "{shown}");
            assert!(shown.contains("20260906"), "{shown}");
        }
        other => panic!("expected a tick-0 divergence, got {other:?}"),
    }
    remove_silently(&replay_path);
    remove_silently(&golden_path);
}

#[test]
fn checked_in_fixture_parses_and_plays() {
    // Portable shape check over the gate fixture itself: reviewable size,
    // more than one tick and input, a same-tick pair, a gap, trailing ticks.
    let replay = read_replay(&support::fixture_replay_path()).unwrap();
    assert_eq!(replay.format_version, REPLAY_FORMAT_VERSION);
    assert_eq!(replay.total_ticks, 8);
    assert_eq!(replay.inputs.len(), 5);
    assert_eq!(replay.inputs[0].tick, 0);
    assert_eq!(replay.inputs[1].tick, 0);
    let hashes = play_replay(&replay, test_apply()).unwrap();
    assert_eq!(hashes.len(), 8);
    assert_eq!(hashes, play_replay(&replay, test_apply()).unwrap());
}
