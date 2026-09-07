//! End-to-end tests for the `crpgc replay` binary (T009b).
//!
//! These drive the built binary — `env!("CARGO_BIN_EXE_crpgc")` — as a black
//! box through `std::process::Command`, asserting exit codes and single-pathed
//! stderr diagnostics, never replay scene internals. The success cases are
//! cfg-gated to the ADR-0012 supported targets because the fixture's golden
//! is target-scoped (independent Windows/MSVC and Linux/GNU baselines; never
//! compared to each other). The portable cases — divergence, malformed
//! replay, failed apply, missing files, usage — run on every target.

use std::path::PathBuf;
use std::process::{Command, Output};

/// The built `crpgc` binary path, provided by cargo for integration tests.
fn crpgc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_crpgc"))
}

/// Runs `crpgc` with `args`, returning the captured output.
fn run(args: &[&str]) -> Output {
    Command::new(crpgc())
        .args(args)
        .output()
        .expect("spawning crpgc must succeed")
}

/// The checked-in portable replay fixture, as a canonical absolute path.
fn fixture_replay() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("crpg-testkit")
        .join("fixtures")
        .join("replay_basic.replay")
        .canonicalize()
        .expect("fixture replay must exist")
}

/// Targets the ADR-0012 supported golden, when this build is one of those
/// targets and profiles. The cfg guards match
/// `crpg-testkit/tests/support/mod.rs` exactly; elsewhere neither golden
/// exists and portable tests below still run.
fn target_golden() -> PathBuf {
    #[cfg(all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc",
        debug_assertions
    ))]
    {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("crpg-testkit")
            .join("goldens")
            .join("replay_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden")
            .canonicalize()
            .expect("windows golden must exist on the windows target")
    }
    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu",
        debug_assertions
    ))]
    {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("crpg-testkit")
            .join("goldens")
            .join("replay_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden")
            .canonicalize()
            .expect("linux golden must exist on the linux target")
    }
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
    {
        panic!("target_golden is only callable on an ADR-0012 supported target")
    }
}

/// A unique per-process temp path, e.g. for a corrupted test golden.
fn temp_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("crpg-cli-it-{}-{name}", std::process::id()));
    path
}

/// Writes `text`, returning the path. Removes any prior file at that path so
/// stale leftovers from a crashed run cannot fake a success.
fn write_temp(name: &str, text: &str) -> PathBuf {
    let path = temp_path(name);
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, text).expect("writing temp file must succeed");
    path
}

/// Cleans up a file written by [`write_temp`] after a test.
fn remove_temp(path: &PathBuf) {
    let _ = std::fs::remove_file(path);
}

/// A valid but content-wrong golden: one header and one all-zero hash line.
/// The fixture plays eight ticks, so verification diverges at tick 0.
const WRONG_GOLDEN: &str = "\
# scope: crpg-cli integration test (deliberately wrong)
0000000000000000000000000000000000000000000000000000000000000000
";

/// A replay that passes validation but whose payload the reference apply
/// does not understand — reaching the `ApplyFailed` path.
const UNKNOWN_PAYLOAD_REPLAY: &str = "\
{
  \"format_version\": 1,
  \"seed\": 1,
  \"campaign_id\": \"cli-apply-fail\",
  \"campaign_version\": \"0.1.0\",
  \"engine_version\": \"0.1.0\",
  \"total_ticks\": 1,
  \"inputs\": [
    {
      \"tick\": 0,
      \"payload\": {
        \"launch\": true
      }
    }
  ]
}";

#[test]
fn replay_matches_native_golden_on_windows() {
    #[cfg(all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc",
        debug_assertions
    ))]
    {
        let out = run(&[
            "replay",
            fixture_replay().to_str().expect("utf-8 path"),
            "--golden",
            target_golden().to_str().expect("utf-8 path"),
        ]);
        assert!(out.status.success(), "windows golden must match: {out:?}");
        assert!(out.stdout.is_empty(), "success must print nothing: {out:?}");
        assert!(out.stderr.is_empty(), "success must print nothing: {out:?}");
    }
}

#[test]
fn replay_matches_native_golden_on_linux() {
    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu",
        debug_assertions
    ))]
    {
        let out = run(&[
            "replay",
            fixture_replay().to_str().expect("utf-8 path"),
            "--golden",
            target_golden().to_str().expect("utf-8 path"),
        ]);
        assert!(out.status.success(), "linux golden must match: {out:?}");
        assert!(out.stdout.is_empty(), "success must print nothing: {out:?}");
        assert!(out.stderr.is_empty(), "success must print nothing: {out:?}");
    }
}

#[test]
fn replay_default_golden_sibling_on_native_target() {
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
    {
        let dir = temp_path("default-golden-dir");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir must create");
        let replay = dir.join("replay_name.replay");
        let golden = dir.join("replay_name.golden");
        std::fs::copy(fixture_replay(), &replay).expect("fixture copy must succeed");
        std::fs::copy(target_golden(), &golden).expect("golden copy must succeed");
        let out = run(&["replay", replay.to_str().expect("utf-8 path")]);
        assert!(
            out.status.success(),
            "default sibling golden must match: {out:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn divergence_is_exit_1_with_exact_tick() {
    let replay = fixture_replay();
    let golden = write_temp("wrong.golden", WRONG_GOLDEN);
    let out = run(&[
        "replay",
        replay.to_str().expect("utf-8 path"),
        "--golden",
        golden.to_str().expect("utf-8 path"),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "divergence must exit 1: {out:?}"
    );
    assert!(
        stderr.contains("replay 'testkit-basic'"),
        "identity must print: {stderr}"
    );
    assert!(
        stderr.contains("diverged at tick 0"),
        "exact tick must print: {stderr}"
    );
    assert!(
        out.stdout.is_empty(),
        "diagnostics go to stderr only: {out:?}"
    );
    remove_temp(&golden);
}

#[test]
fn malformed_replay_is_exit_1() {
    let replay = write_temp("malformed.replay", "this is not json");
    let out = run(&["replay", replay.to_str().expect("utf-8 path")]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "malformed must exit 1: {out:?}");
    assert!(
        stderr.contains("malformed replay"),
        "diagnostic must print: {stderr}"
    );
    remove_temp(&replay);
}

#[test]
fn unknown_payload_is_exit_1_apply_failed() {
    let replay = write_temp("unknown-payload.replay", UNKNOWN_PAYLOAD_REPLAY);
    let out = run(&["replay", replay.to_str().expect("utf-8 path")]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "apply failure must exit 1: {out:?}"
    );
    assert!(
        stderr.contains("failed to apply"),
        "apply diagnostic must print: {stderr}"
    );
    remove_temp(&replay);
}

#[test]
fn missing_replay_is_exit_1_io() {
    let missing = temp_path("does-not-exist.replay");
    let out = run(&["replay", missing.to_str().expect("utf-8 path")]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "missing replay must exit 1: {out:?}"
    );
    assert!(
        stderr.contains("replay file I/O failed"),
        "Io diagnostic must print: {stderr}"
    );
}

#[test]
fn missing_golden_is_exit_1_io_not_divergence() {
    let replay = fixture_replay();
    let missing = temp_path("does-not-exist.golden");
    let out = run(&[
        "replay",
        replay.to_str().expect("utf-8 path"),
        "--golden",
        missing.to_str().expect("utf-8 path"),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "missing golden must exit 1: {out:?}"
    );
    assert!(
        stderr.contains("replay file I/O failed"),
        "Io diagnostic must print: {stderr}"
    );
    assert!(
        !stderr.contains("diverged"),
        "missing baseline must not look like a divergence: {stderr}"
    );
}

#[test]
fn no_arguments_is_usage_exit_2() {
    let out = run(&[]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "no args must exit 2: {out:?}");
    assert!(
        stderr.contains("missing subcommand"),
        "usage must print: {stderr}"
    );
}

#[test]
fn unknown_subcommand_is_usage_exit_2() {
    let out = run(&["migrate"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(2),
        "unknown subcommand must exit 2: {out:?}"
    );
    assert!(
        stderr.contains("unknown subcommand"),
        "usage must print: {stderr}"
    );
}

#[test]
fn replay_missing_path_is_usage_exit_2() {
    let out = run(&["replay"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(2),
        "missing path must exit 2: {out:?}"
    );
    assert!(
        stderr.contains("missing <replay-path>"),
        "usage must print: {stderr}"
    );
}

#[test]
fn unknown_flag_is_usage_exit_2() {
    let out = run(&["replay", "a.replay", "--nope"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(2),
        "unknown flag must exit 2: {out:?}"
    );
    assert!(
        stderr.contains("unknown flag"),
        "usage must print: {stderr}"
    );
}

#[test]
fn golden_without_value_is_usage_exit_2() {
    let out = run(&["replay", "a.replay", "--golden"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(2),
        "missing flag value must exit 2: {out:?}"
    );
    assert!(
        stderr.contains("--golden needs a value"),
        "usage must print: {stderr}"
    );
}
