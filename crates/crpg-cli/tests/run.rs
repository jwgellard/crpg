//! End-to-end tests for `crpgc run` (T013).
//!
//! These drive the built binary — `env!("CARGO_BIN_EXE_crpgc")` — as a black
//! box through `std::process::Command`, asserting exit codes and exact stream
//! bytes. `run` is a bounded harness wrapper, not campaign execution: it
//! calls `run_hash_sequence` once with the empty world's no-op script and
//! emits only the sampled `M, 2M, ... <= N` hash lines. Sample equality with
//! the native harness is asserted through real `crpg_testkit` calls; the
//! target-scoped replay tests remain the independent baseline gates.

use std::path::PathBuf;
use std::process::{Command, Output};

/// The built `crpgc` binary path, provided by cargo for integration tests.
fn crpgc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_crpgc"))
}

/// Runs `crpgc` with Unicode `args`, returning the captured output.
fn run(args: &[&str]) -> Output {
    Command::new(crpgc())
        .args(args)
        .output()
        .expect("spawning crpgc must succeed")
}

const RUN_USAGE: &str = "crpgc: usage: crpgc run --ticks N --hash-every M [--seed S]\n";

/// Lowercase hex rendering shared with the CLI's hand-rolled encoder.
fn hex32(bytes: &[u8; 32]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Expected sampled output for the no-op script through the native harness.
fn expected_samples(ticks: usize, hash_every: usize, seed: u64) -> Vec<u8> {
    let hashes = crpg_testkit::run_hash_sequence(seed, ticks, Box::new(|_| {}));
    let mut out = Vec::new();
    let mut tick = hash_every;
    while tick <= ticks {
        out.extend_from_slice(format!("{tick} {}\n", hex32(&hashes[tick - 1])).as_bytes());
        tick += hash_every;
    }
    out
}

#[test]
fn run_usage_matrix_reports_one_line_with_exit_2() {
    let cases: Vec<Vec<&str>> = vec![
        vec!["run"],
        vec!["run", "--ticks", "10"],
        vec!["run", "--hash-every", "2"],
        vec!["run", "--ticks", "10", "--hash-every", "2", "--seed"],
        vec!["run", "--ticks", "10", "--ticks", "11", "--hash-every", "2"],
        vec![
            "run",
            "--ticks",
            "10",
            "--hash-every",
            "2",
            "--hash-every",
            "3",
        ],
        vec!["run", "--ticks", "10", "--bogus", "2"],
        vec!["run", "--ticks", "10", "--hash-every", "2", "extra"],
        vec!["run", "--ticks=10", "--hash-every", "2"],
        vec!["run", "--ticks", "+10", "--hash-every", "2"],
        vec!["run", "--ticks", "-1", "--hash-every", "2"],
        vec!["run", "--ticks", "0x10", "--hash-every", "2"],
        vec!["run", "--ticks", "1_0", "--hash-every", "2"],
        vec!["run", "--ticks", "", "--hash-every", "2"],
        vec!["run", "--ticks", "1000001", "--hash-every", "1"],
        vec!["run", "--ticks", "10", "--hash-every", "0"],
        vec!["run", "--ticks", "10", "--hash-every", "1000001"],
        vec![
            "run",
            "--ticks",
            "18446744073709551616",
            "--hash-every",
            "1",
        ],
        vec!["run", "--help", "--ticks", "1", "--hash-every", "1"],
    ];
    for argv in &cases {
        let output = run(argv);
        assert_eq!(output.status.code(), Some(2), "argv {argv:?}");
        assert!(output.stdout.is_empty(), "argv {argv:?}");
        assert_eq!(output.stderr, RUN_USAGE.as_bytes(), "argv {argv:?}");
    }
}

#[test]
fn run_zero_ticks_produces_empty_stdout_with_exit_0() {
    let output = run(&["run", "--ticks", "0", "--hash-every", "1"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn run_hash_every_one_samples_every_tick() {
    let output = run(&["run", "--ticks", "4", "--hash-every", "1"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, expected_samples(4, 1, 0));
    assert_eq!(
        String::from_utf8(output.stdout)
            .expect("utf-8")
            .lines()
            .count(),
        4
    );
}

#[test]
fn run_hash_every_beyond_ticks_produces_empty_stdout() {
    let output = run(&["run", "--ticks", "3", "--hash-every", "9"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn run_nondivisible_intervals_sample_count_is_floor_n_over_m() {
    // N=7, M=3 samples ticks 3 and 6 only: no forced final partial sample.
    let output = run(&["run", "--ticks", "7", "--hash-every", "3"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, expected_samples(7, 3, 0));
    assert_eq!(
        String::from_utf8(output.stdout)
            .expect("utf-8")
            .lines()
            .count(),
        2
    );
}

#[test]
fn run_sample_labels_and_hex_shape_are_exact() {
    let output = run(&["run", "--ticks", "6", "--hash-every", "2", "--seed", "11"]);
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8(output.stdout).expect("utf-8");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    for (index, line) in lines.iter().enumerate() {
        let tick = (index + 1) * 2;
        let (label, hash) = line.split_once(' ').expect("label and hash");
        assert_eq!(
            label,
            tick.to_string(),
            "sample label uses completed tick k"
        );
        assert_eq!(hash.len(), 64, "hash is 64 hex chars");
        assert!(
            hash.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "hash is lowercase hex"
        );
    }
    // No header, no initial-state sample: the first label is M, not 0.
    assert!(lines[0].starts_with("2 "));
}

#[test]
fn run_explicit_seed_zero_matches_the_default_seed() {
    let default = run(&["run", "--ticks", "5", "--hash-every", "2"]);
    let explicit = run(&["run", "--ticks", "5", "--hash-every", "2", "--seed", "0"]);
    assert_eq!(default.status.code(), Some(0));
    assert_eq!(explicit.stdout, default.stdout);
    assert_eq!(explicit.stdout, expected_samples(5, 2, 0));
}

#[test]
fn run_repeats_identically_and_matches_the_native_harness() {
    let first = run(&["run", "--ticks", "9", "--hash-every", "4", "--seed", "42"]);
    let second = run(&["run", "--ticks", "9", "--hash-every", "4", "--seed", "42"]);
    assert_eq!(first.status.code(), Some(0));
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(first.stdout, expected_samples(9, 4, 42));
    // A different seed diverges: the seed reaches the harness.
    let other = run(&["run", "--ticks", "9", "--hash-every", "4", "--seed", "43"]);
    assert_ne!(other.stdout, first.stdout);
}

#[test]
fn run_uses_index_k_minus_1_not_k() {
    // The sample for completed tick k is the harness index k-1: with M=1
    // the first line is the post-tick-1 hash, never an initial state.
    let hashes = crpg_testkit::run_hash_sequence(7, 2, Box::new(|_| {}));
    let output = run(&["run", "--ticks", "2", "--hash-every", "1", "--seed", "7"]);
    let text = String::from_utf8(output.stdout).expect("utf-8");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], format!("1 {}", hex32(&hashes[0])));
    assert_eq!(lines[1], format!("2 {}", hex32(&hashes[1])));
}

#[test]
fn run_accepts_u64_max_seed_with_small_ticks() {
    // The u64::MAX seed is accepted without million-tick work: small N
    // proves the boundary parses and reaches the harness.
    let output = run(&[
        "run",
        "--ticks",
        "2",
        "--hash-every",
        "1",
        "--seed",
        "18446744073709551615",
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, expected_samples(2, 1, u64::MAX));
}
