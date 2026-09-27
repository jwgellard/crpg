//! Black-box acceptance for `crpgc replay --campaign` (T016d).
//!
//! These drive the built binary (`env!("CARGO_BIN_EXE_crpgc")`) as a real
//! process, asserting exit codes and exact stream bytes, never combat or
//! replay internals. Checked-in inputs stay read-only; mutation uses private
//! temporary copies with std-only temp conventions and cleanup. No test
//! writes a new expected golden. Native success/divergence tests select the
//! existing scoped baseline at compile time with T009c's cfg guards;
//! portable usage/collection/payload/error tests never read either baseline,
//! never detect the platform at runtime, and never assert cross-platform
//! hash equality.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Authored placement/replay identities from the checked-in fixture.
const HERO: &str = "0000000000000000000000000E";
const GOBLIN: &str = "0000000000000000000000000F";
const ABILITY: &str = "0000000000000000000000000K";
const ENCOUNTER: &str = "0000000000000000000000000N";
/// Syntactically valid but unbound identity for unknown-identity rows.
const STRANGER: &str = "00000000000000000000000000";

/// The built `crpgc` binary path, provided by cargo for integration tests.
fn crpgc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_crpgc"))
}

/// Runs `crpgc` with Unicode `args` in the current working directory.
fn run(args: &[&str]) -> Output {
    Command::new(crpgc())
        .args(args)
        .output()
        .expect("spawning crpgc must succeed")
}

/// Runs `crpgc` with Unicode `args` in `dir` as the working directory.
fn run_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(crpgc())
        .current_dir(dir)
        .args(args)
        .output()
        .expect("spawning crpgc must succeed")
}

/// Runs `crpgc` with OS-string `args` for non-Unicode process cases.
#[cfg(unix)]
fn run_os(args: &[std::ffi::OsString]) -> Output {
    Command::new(crpgc())
        .args(args)
        .output()
        .expect("spawning crpgc must succeed")
}

/// The `crpg-cli` crate directory, without depending on the working dir.
fn cli_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The `crpg-testkit` crate directory holding replay fixtures and goldens.
fn testkit_dir() -> PathBuf {
    cli_dir().join("..").join("crpg-testkit")
}

/// The checked-in portable combat replay fixture (read-only).
fn combat_replay() -> PathBuf {
    testkit_dir().join("fixtures").join("combat_basic.replay")
}

/// The checked-in `combat_basic` campaign root (read-only).
fn combat_campaign() -> PathBuf {
    cli_dir()
        .join("..")
        .join("..")
        .join("campaigns")
        .join("fixtures")
        .join("combat_basic")
}

/// The checked-in portable reference replay fixture (read-only).
fn reference_replay() -> PathBuf {
    testkit_dir().join("fixtures").join("replay_basic.replay")
}

/// The supported native combat golden, when this build is one of the two
/// ADR-0012 targets and profiles. The cfg guards match testkit exactly;
/// elsewhere portable tests still run without reading either baseline.
fn target_golden() -> PathBuf {
    #[cfg(all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc",
        debug_assertions
    ))]
    {
        testkit_dir()
            .join("goldens")
            .join("combat_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden")
    }
    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu",
        debug_assertions
    ))]
    {
        testkit_dir()
            .join("goldens")
            .join("combat_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden")
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

/// The supported native reference golden for the legacy regression.
fn reference_target_golden() -> PathBuf {
    #[cfg(all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc",
        debug_assertions
    ))]
    {
        testkit_dir()
            .join("goldens")
            .join("replay_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden")
    }
    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu",
        debug_assertions
    ))]
    {
        testkit_dir()
            .join("goldens")
            .join("replay_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden")
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
        panic!("reference_target_golden is only callable on a supported target")
    }
}

/// A fresh unique temp directory for one test. Tests run in parallel, so the
/// name carries the test's own slug; stale leftovers are removed first.
fn temp_root(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("crpg-combat-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp root must create");
    path
}

/// Recursively copies `src` to `dst` with std only.
fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("copy destination must create");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(src)
        .expect("copy source must list")
        .map(|entry| entry.expect("copy entry").path())
        .collect();
    entries.sort();
    for entry in &entries {
        let target = dst.join(entry.file_name().expect("file name"));
        if entry.is_dir() {
            copy_dir(entry, &target);
        } else {
            std::fs::copy(entry, &target).expect("copy file must succeed");
        }
    }
}

/// Copies the checked-in combat campaign to a private temp root for mutation.
fn copy_campaign(name: &str) -> PathBuf {
    let dst = temp_root(name);
    copy_dir(&combat_campaign(), &dst);
    dst
}

/// Best-effort cleanup of a temp root after a test.
fn cleanup(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

/// Snapshots every file under `root` as forward-slash relative path bytes.
fn snapshot_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .expect("snapshot source must list")
            .map(|entry| entry.expect("snapshot entry").path())
            .collect();
        entries.sort();
        for entry in &entries {
            if entry.is_dir() {
                walk(entry, root, out);
            } else {
                let relative = entry
                    .strip_prefix(root)
                    .expect("snapshot entry under root")
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes = std::fs::read(entry).expect("snapshot file must read");
                out.insert(relative, bytes);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// Writes a replay envelope with the combat identity and the given inputs.
fn write_replay(path: &Path, total_ticks: u64, inputs: &str) {
    let text = format!(
        "{{\n  \"format_version\": 1,\n  \"seed\": 0,\n  \"campaign_id\": \"combat-basic\",\n  \"campaign_version\": \"0.1.0\",\n  \"engine_version\": \"0.1.0\",\n  \"total_ticks\": {total_ticks},\n  \"inputs\": [{inputs}\n  ]\n}}\n"
    );
    std::fs::write(path, text).expect("replay write must succeed");
}

/// One replay input entry with a raw JSON payload.
fn input(tick: u64, payload: &str) -> String {
    format!("\n    {{\n      \"tick\": {tick},\n      \"payload\": {payload}\n    }}")
}

/// Joins input entries with commas.
fn join_inputs(entries: &[String]) -> String {
    entries.join(",")
}

fn init_payload(encounter: &str) -> String {
    format!("{{\"combat\": 1, \"op\": \"init\", \"encounter\": \"{encounter}\"}}")
}

fn attack_payload(actor: &str, ability: &str, target: &str) -> String {
    format!(
        "{{\"combat\": 1, \"op\": \"attack\", \"actor\": \"{actor}\", \"ability\": \"{ability}\", \"target\": \"{target}\"}}"
    )
}

/// Asserts the exact single-line `crpgc replay:` failure shape.
fn assert_replay_failure(out: &Output, tick: u64, index: usize, reason: &str) {
    assert_eq!(
        out.status.code(),
        Some(1),
        "apply failure must exit 1: {out:?}"
    );
    assert!(out.stdout.is_empty(), "diagnostics go to stderr: {out:?}");
    let stderr = String::from_utf8(out.stderr.clone()).expect("stderr utf-8");
    assert_eq!(
        stderr,
        format!(
            "crpgc replay: replay input at tick {tick} (index {index}) failed to apply: {reason}\n"
        ),
        "exact tick/index/reason: {stderr}"
    );
}

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
fn combat_matches_native_golden_with_explicit_paths() {
    let out = run(&[
        "replay",
        combat_replay().to_str().expect("utf-8"),
        "--campaign",
        combat_campaign().to_str().expect("utf-8"),
        "--golden",
        target_golden().to_str().expect("utf-8"),
    ]);
    assert!(out.status.success(), "combat golden must match: {out:?}");
    assert!(out.stdout.is_empty(), "success must print nothing: {out:?}");
    assert!(out.stderr.is_empty(), "success must print nothing: {out:?}");
}

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
fn combat_default_golden_sibling_beside_private_replay() {
    let dir = temp_root("default-golden");
    let replay = dir.join("combat_copy.replay");
    let golden = dir.join("combat_copy.golden");
    std::fs::copy(combat_replay(), &replay).expect("replay copy must succeed");
    std::fs::copy(target_golden(), &golden).expect("golden copy must succeed");
    let out = run(&[
        "replay",
        replay.to_str().expect("utf-8"),
        "--campaign",
        combat_campaign().to_str().expect("utf-8"),
    ]);
    assert!(
        out.status.success(),
        "default sibling golden must match: {out:?}"
    );
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
    cleanup(&dir);
}

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
fn combat_option_orders_both_match() {
    let replay = combat_replay().to_str().expect("utf-8").to_owned();
    let campaign = combat_campaign().to_str().expect("utf-8").to_owned();
    let golden = target_golden().to_str().expect("utf-8").to_owned();
    for args in [
        vec![
            "replay",
            replay.as_str(),
            "--golden",
            golden.as_str(),
            "--campaign",
            campaign.as_str(),
        ],
        vec![
            "replay",
            replay.as_str(),
            "--campaign",
            campaign.as_str(),
            "--golden",
            golden.as_str(),
        ],
    ] {
        let out = run(&args);
        assert!(
            out.status.success(),
            "both option orders must match: {out:?}"
        );
        assert!(out.stdout.is_empty() && out.stderr.is_empty());
    }
}

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
fn combat_spaces_renamed_campaign_and_working_directory() {
    // Relocate the campaign to a renamed directory with spaces: no
    // repository-root or directory-name dependency.
    let base = temp_root("spaces base");
    let campaign = base.join("renamed campaign");
    copy_dir(&combat_campaign(), &campaign);
    let replay = base.join("re play.replay");
    let golden = base.join("re play.golden");
    std::fs::copy(combat_replay(), &replay).expect("replay copy must succeed");
    std::fs::copy(target_golden(), &golden).expect("golden copy must succeed");
    // From an unrelated working directory with absolute paths.
    let elsewhere = temp_root("unrelated cwd");
    let out = run_in(
        &elsewhere,
        &[
            "replay",
            replay.to_str().expect("utf-8"),
            "--campaign",
            campaign.to_str().expect("utf-8"),
            "--golden",
            golden.to_str().expect("utf-8"),
        ],
    );
    assert!(
        out.status.success(),
        "spaced absolute paths must match: {out:?}"
    );
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    // Then with paths relative to that directory.
    let work = temp_root("relative cwd");
    let work_campaign = work.join("campaign copy");
    copy_dir(&combat_campaign(), &work_campaign);
    std::fs::copy(combat_replay(), work.join("fight.replay")).expect("copy must succeed");
    std::fs::copy(target_golden(), work.join("fight.golden")).expect("copy must succeed");
    let out = run_in(
        &work,
        &["replay", "fight.replay", "--campaign", "campaign copy"],
    );
    assert!(
        out.status.success(),
        "relative paths must resolve against cwd: {out:?}"
    );
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    cleanup(&base);
    cleanup(&elsewhere);
    cleanup(&work);
}

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
fn legacy_reference_still_passes_without_campaign() {
    let out = run(&[
        "replay",
        reference_replay().to_str().expect("utf-8"),
        "--golden",
        reference_target_golden().to_str().expect("utf-8"),
    ]);
    assert!(
        out.status.success(),
        "legacy replay must still pass: {out:?}"
    );
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
}

#[test]
fn combat_without_flag_fails_reference_decoding() {
    let out = run(&["replay", combat_replay().to_str().expect("utf-8")]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "combat without flag must exit 1: {out:?}"
    );
    assert!(out.stdout.is_empty());
    assert!(stderr.starts_with("crpgc replay: "), "{stderr}");
    assert!(stderr.contains("failed to apply"), "{stderr}");
    assert!(stderr.contains("unknown intent payload"), "{stderr}");
    assert!(stderr.contains("tick 0 (index 0)"), "{stderr}");
}

#[test]
fn reference_with_flag_fails_combat_version_decoding() {
    let out = run(&[
        "replay",
        reference_replay().to_str().expect("utf-8"),
        "--campaign",
        combat_campaign().to_str().expect("utf-8"),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "reference with flag must exit 1: {out:?}"
    );
    assert!(out.stdout.is_empty());
    assert!(
        stderr.contains("unsupported combat payload version"),
        "{stderr}"
    );
    assert!(stderr.contains("tick 0 (index 0)"), "{stderr}");
}

#[test]
fn combat_attack_in_legacy_run_never_switches_mode() {
    let dir = temp_root("legacy-mixed");
    let replay = dir.join("mixed.replay");
    let entries = vec![input(0, &attack_payload(GOBLIN, ABILITY, HERO))];
    write_replay(&replay, 1, &join_inputs(&entries));
    let out = run(&["replay", replay.to_str().expect("utf-8")]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "mixed legacy run must exit 1: {out:?}"
    );
    assert!(stderr.contains("unknown intent payload"), "{stderr}");
    assert!(!stderr.contains("combat not initialized"), "{stderr}");
    cleanup(&dir);
}

#[test]
fn campaign_usage_errors_are_exact_and_precede_io() {
    // Missing value and duplicate carry exact bytes.
    for args in [
        vec!["replay", "a.replay", "--campaign"],
        vec!["replay", "a.replay", "--campaign", "--golden", "g"],
    ] {
        let out = run(&args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "missing campaign value: {out:?}"
        );
        assert!(out.stdout.is_empty());
        assert_eq!(out.stderr, b"crpgc: --campaign needs a value\n");
    }
    let out = run(&["replay", "a.replay", "--campaign", "c", "--campaign", "d"]);
    assert_eq!(out.status.code(), Some(2), "duplicate campaign: {out:?}");
    assert_eq!(out.stderr, b"crpgc: --campaign given more than once\n");
    // Remaining bad syntax stays usage exit 2 before any filesystem work:
    // nonexistent paths still report usage, never I/O.
    let missing = temp_root("usage-missing");
    let absent = missing.join("absent.replay");
    let absent_text = absent.to_str().expect("utf-8").to_owned();
    let cases: Vec<Vec<&str>> = vec![
        vec!["replay", absent_text.as_str(), "extra"],
        vec!["replay", absent_text.as_str(), "--campaign", "c", "extra"],
        vec!["replay", absent_text.as_str(), "--campaign=c"],
        vec!["replay", absent_text.as_str(), "--", "--campaign", "c"],
        vec!["replay", absent_text.as_str(), "--write", "--campaign", "c"],
        vec!["replay", absent_text.as_str(), "--campaign", "c", "--bogus"],
        vec!["replay", "--campaign", "c", absent_text.as_str()],
    ];
    for args in &cases {
        let out = run(args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "usage must precede I/O for {args:?}: {out:?}"
        );
        assert!(out.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&out.stderr).starts_with("crpgc: "),
            "{args:?}"
        );
        assert!(
            !String::from_utf8_lossy(&out.stderr).contains("cannot "),
            "usage must not leak I/O diagnostics for {args:?}"
        );
    }
    cleanup(&missing);
}

#[test]
fn missing_campaign_root_and_file_root_are_exit_1() {
    let missing = temp_root("missing-root").join("absent");
    let out = run(&[
        "replay",
        combat_replay().to_str().expect("utf-8"),
        "--campaign",
        missing.to_str().expect("utf-8"),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "missing root must exit 1: {out:?}"
    );
    assert!(out.stdout.is_empty());
    assert!(
        stderr.contains("cannot open root <campaign-root>: not_found"),
        "{stderr}"
    );
    // A regular file is not a campaign root.
    let dir = temp_root("file-root");
    let file = dir.join("file.json");
    std::fs::write(&file, b"{}").expect("temp file must write");
    let out = run(&[
        "replay",
        combat_replay().to_str().expect("utf-8"),
        "--campaign",
        file.to_str().expect("utf-8"),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "file root must exit 1: {out:?}");
    assert!(
        stderr.contains("cannot open root <campaign-root>: not_a_directory"),
        "{stderr}"
    );
    cleanup(&dir);
}

#[test]
fn malformed_campaign_stops_before_replay_read() {
    let campaign = copy_campaign("malformed-campaign");
    std::fs::write(campaign.join("campaign.json"), b"{ broken").expect("corrupt must write");
    let absent_replay = campaign.join("absent.replay");
    let out = run(&[
        "replay",
        absent_replay.to_str().expect("utf-8"),
        "--campaign",
        campaign.to_str().expect("utf-8"),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "malformed campaign must exit 1: {out:?}"
    );
    assert!(out.stdout.is_empty());
    assert!(
        !stderr.contains("replay file"),
        "campaign beats missing replay: {stderr}"
    );
    assert!(!stderr.is_empty());
    cleanup(&campaign);
}

#[test]
fn invalid_action_beats_missing_golden() {
    let dir = temp_root("action-beats-golden");
    let replay = dir.join("bad.replay");
    let entries = vec![input(0, "{\"combat\": 1, \"op\": \"retreat\"}")];
    write_replay(&replay, 1, &join_inputs(&entries));
    let missing_golden = dir.join("absent.golden");
    let out = run(&[
        "replay",
        replay.to_str().expect("utf-8"),
        "--campaign",
        combat_campaign().to_str().expect("utf-8"),
        "--golden",
        missing_golden.to_str().expect("utf-8"),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "invalid action must exit 1: {out:?}"
    );
    assert!(stderr.contains("unknown combat op"), "{stderr}");
    assert!(!stderr.contains("replay file I/O failed"), "{stderr}");
    cleanup(&dir);
}

#[test]
fn generic_campaign_without_encounter_is_unknown_encounter() {
    let generic = cli_dir()
        .join("..")
        .join("crpg-data")
        .join("tests")
        .join("fixtures")
        .join("one_area_one_creature");
    let dir = temp_root("generic-campaign");
    let replay = dir.join("init.replay");
    let entries = vec![input(0, &init_payload(ENCOUNTER))];
    write_replay(&replay, 1, &join_inputs(&entries));
    let out = run(&[
        "replay",
        replay.to_str().expect("utf-8"),
        "--campaign",
        generic.to_str().expect("utf-8"),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "generic campaign must exit 1: {out:?}"
    );
    assert!(
        stderr.contains(&format!("unknown encounter {ENCOUNTER}")),
        "{stderr}"
    );
    assert!(!stderr.contains("expected 14"), "{stderr}");
    assert!(!stderr.contains("minimal-d6"), "{stderr}");
    cleanup(&dir);
}

#[test]
fn semantically_invalid_campaign_reports_findings_first() {
    let broken = cli_dir()
        .join("..")
        .join("crpg-data")
        .join("tests")
        .join("fixtures")
        .join("broken_references");
    let dir = temp_root("broken-campaign");
    let replay = dir.join("init.replay");
    let entries = vec![input(0, &init_payload(ENCOUNTER))];
    write_replay(&replay, 1, &join_inputs(&entries));
    let out = run(&[
        "replay",
        replay.to_str().expect("utf-8"),
        "--campaign",
        broken.to_str().expect("utf-8"),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "broken campaign must exit 1: {out:?}"
    );
    assert!(out.stdout.is_empty());
    assert!(!stderr.contains("unknown encounter"), "{stderr}");
    assert!(!stderr.contains("replay file"), "{stderr}");
    assert!(!stderr.is_empty());
    cleanup(&dir);
}

#[test]
fn replay_and_golden_file_categories_stay_distinct() {
    let campaign = combat_campaign().to_str().expect("utf-8").to_owned();
    // Missing replay is I/O, never divergence.
    let absent = temp_root("absent-replay").join("absent.replay");
    let out = run(&[
        "replay",
        absent.to_str().expect("utf-8"),
        "--campaign",
        campaign.as_str(),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "missing replay: {out:?}");
    assert!(stderr.contains("replay file I/O failed"), "{stderr}");
    assert!(!stderr.contains("diverged"), "{stderr}");
    // Malformed replay is a parse failure with the single prefix.
    let dir = temp_root("malformed-replay");
    let bad = dir.join("bad.replay");
    std::fs::write(&bad, b"this is not json").expect("write must succeed");
    let out = run(&[
        "replay",
        bad.to_str().expect("utf-8"),
        "--campaign",
        campaign.as_str(),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "malformed replay: {out:?}");
    assert!(stderr.starts_with("crpgc replay: "), "{stderr}");
    assert!(stderr.contains("malformed replay"), "{stderr}");
    assert_eq!(stderr.matches("crpgc replay:").count(), 1, "{stderr}");
    // Missing golden is I/O, never divergence.
    let out = run(&[
        "replay",
        combat_replay().to_str().expect("utf-8"),
        "--campaign",
        campaign.as_str(),
        "--golden",
        dir.join("absent.golden").to_str().expect("utf-8"),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "missing golden: {out:?}");
    assert!(stderr.contains("replay file I/O failed"), "{stderr}");
    assert!(!stderr.contains("diverged"), "{stderr}");
    // A content-wrong golden diverges at tick 0 with identity attached.
    let wrong = dir.join("wrong.golden");
    std::fs::write(
        &wrong,
        "# scope: combat cli test (deliberately wrong)\n0000000000000000000000000000000000000000000000000000000000000000\n",
    )
    .expect("write must succeed");
    let out = run(&[
        "replay",
        combat_replay().to_str().expect("utf-8"),
        "--campaign",
        campaign.as_str(),
        "--golden",
        wrong.to_str().expect("utf-8"),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "wrong golden: {out:?}");
    assert!(stderr.contains("replay 'combat-basic'"), "{stderr}");
    assert!(stderr.contains("diverged at tick 0"), "{stderr}");
    // A truncated golden reports the short side, not I/O.
    let short = dir.join("short.golden");
    std::fs::write(
        &short,
        "# scope: short\n0000000000000000000000000000000000000000000000000000000000000000\n",
    )
    .expect("write must succeed");
    let out = run(&[
        "replay",
        combat_replay().to_str().expect("utf-8"),
        "--campaign",
        campaign.as_str(),
        "--golden",
        short.to_str().expect("utf-8"),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "short golden: {out:?}");
    assert!(stderr.contains("replay 'combat-basic'"), "{stderr}");
    assert!(!stderr.contains("replay file I/O failed"), "{stderr}");
    // An overlong golden reports the run ending first, not I/O.
    let mut long_text = String::from("# scope: long\n");
    for _ in 0..12 {
        long_text.push_str("0000000000000000000000000000000000000000000000000000000000000000\n");
    }
    let long = dir.join("long.golden");
    std::fs::write(&long, long_text).expect("write must succeed");
    let out = run(&[
        "replay",
        combat_replay().to_str().expect("utf-8"),
        "--campaign",
        campaign.as_str(),
        "--golden",
        long.to_str().expect("utf-8"),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "long golden: {out:?}");
    assert!(stderr.contains("replay 'combat-basic'"), "{stderr}");
    assert!(!stderr.contains("replay file I/O failed"), "{stderr}");
    cleanup(&dir);
}

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
fn delayed_lethal_attack_diverges_at_index_6() {
    // Delay only the lethal attack from tick 6 to tick 7; total_ticks stays
    // 11. The run stays valid but first diverges at index 6 against the
    // unchanged target golden.
    let text = std::fs::read_to_string(combat_replay()).expect("fixture must read");
    let mutated = text.replacen("\"tick\": 6", "\"tick\": 7", 1);
    assert_ne!(text, mutated, "the lethal tick must move");
    let dir = temp_root("delayed-lethal");
    let replay = dir.join("delayed.replay");
    std::fs::write(&replay, mutated).expect("mutated replay must write");
    let golden = target_golden();
    let out = run(&[
        "replay",
        replay.to_str().expect("utf-8"),
        "--campaign",
        combat_campaign().to_str().expect("utf-8"),
        "--golden",
        golden.to_str().expect("utf-8"),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "delayed lethal must exit 1: {out:?}"
    );
    assert!(out.stdout.is_empty());
    assert!(stderr.contains("replay 'combat-basic'"), "{stderr}");
    assert!(stderr.contains("diverged at tick 6"), "{stderr}");
    cleanup(&dir);
}

#[test]
fn payload_error_matrix_names_tick_index_and_reason() {
    let campaign = combat_campaign().to_str().expect("utf-8").to_owned();
    let dir = temp_root("payload-matrix");
    let mut seq = 0;
    let mut check =
        |payloads: Vec<(u64, String)>, total: u64, tick: u64, index: usize, reason: &str| {
            seq += 1;
            let replay = dir.join(format!("case-{seq}.replay"));
            let entries: Vec<String> = payloads
                .iter()
                .map(|(at, payload)| input(*at, payload))
                .collect();
            write_replay(&replay, total, &join_inputs(&entries));
            let out = run(&[
                "replay",
                replay.to_str().expect("utf-8"),
                "--campaign",
                campaign.as_str(),
            ]);
            assert_replay_failure(&out, tick, index, reason);
        };
    // Non-object; missing/wrong version; unknown op.
    check(
        vec![(0, "[1, 2]".to_string())],
        1,
        0,
        0,
        "malformed combat payload: expected object",
    );
    check(
        vec![(0, "{\"op\": \"init\"}".to_string())],
        1,
        0,
        0,
        "unsupported combat payload version",
    );
    check(
        vec![(0, "{\"combat\": 2, \"op\": \"init\"}".to_string())],
        1,
        0,
        0,
        "unsupported combat payload version",
    );
    check(
        vec![(0, "{\"combat\": 1, \"op\": \"retreat\"}".to_string())],
        1,
        0,
        0,
        "unknown combat op",
    );
    // Unknown field (lexically smallest) beats missing required fields.
    check(
        vec![(
            0,
            "{\"combat\": 1, \"op\": \"init\", \"zz\": 0, \"aa\": 0}".to_string(),
        )],
        1,
        0,
        0,
        "malformed combat init: unknown field aa",
    );
    // Missing and malformed ULIDs in order.
    check(
        vec![(0, "{\"combat\": 1, \"op\": \"init\"}".to_string())],
        1,
        0,
        0,
        "malformed combat init: encounter must be a ULID string",
    );
    check(
        vec![(
            0,
            "{\"combat\": 1, \"op\": \"init\", \"encounter\": \"bad\"}".to_string(),
        )],
        1,
        0,
        0,
        "malformed combat init: invalid encounter",
    );
    // Attack before init.
    check(
        vec![(0, attack_payload(GOBLIN, ABILITY, HERO))],
        1,
        0,
        0,
        "combat not initialized",
    );
    // Unknown encounter identity.
    check(
        vec![(0, init_payload(STRANGER))],
        1,
        0,
        0,
        &format!("unknown encounter {STRANGER}"),
    );
    // Duplicate init names tick 1 index 1.
    check(
        vec![(0, init_payload(ENCOUNTER)), (1, init_payload(ENCOUNTER))],
        2,
        1,
        1,
        "duplicate combat init",
    );
    // Unknown actor before unknown target.
    check(
        vec![
            (0, init_payload(ENCOUNTER)),
            (1, attack_payload(STRANGER, ABILITY, HERO)),
        ],
        2,
        1,
        1,
        &format!("unknown combat identity actor: {STRANGER}").to_string(),
    );
    check(
        vec![
            (0, init_payload(ENCOUNTER)),
            (1, attack_payload(GOBLIN, ABILITY, STRANGER)),
        ],
        2,
        1,
        1,
        &format!("unknown combat identity target: {STRANGER}").to_string(),
    );
    // Valid-but-unknown ability reaches sim validation verbatim.
    check(
        vec![
            (0, init_payload(ENCOUNTER)),
            (1, attack_payload(GOBLIN, STRANGER, HERO)),
        ],
        2,
        1,
        1,
        "attack failed: UnknownAbility at combat/ability",
    );
    // The hero acts out of turn on the opening tick.
    check(
        vec![
            (0, init_payload(ENCOUNTER)),
            (1, attack_payload(HERO, ABILITY, GOBLIN)),
        ],
        2,
        1,
        1,
        "attack failed: OutOfTurn at combat/active",
    );
    cleanup(&dir);
}

#[test]
fn payload_multifault_rows_pin_precedence() {
    let campaign = combat_campaign().to_str().expect("utf-8").to_owned();
    let dir = temp_root("payload-multifault");
    let mut seq = 0;
    let mut check =
        |payloads: Vec<(u64, String)>, total: u64, tick: u64, index: usize, reason: &str| {
            seq += 1;
            let replay = dir.join(format!("multi-{seq}.replay"));
            let entries: Vec<String> = payloads
                .iter()
                .map(|(at, payload)| input(*at, payload))
                .collect();
            write_replay(&replay, total, &join_inputs(&entries));
            let out = run(&[
                "replay",
                replay.to_str().expect("utf-8"),
                "--campaign",
                campaign.as_str(),
            ]);
            assert_replay_failure(&out, tick, index, reason);
        };
    // Unknown field beats a missing required ULID.
    check(
        vec![(
            0,
            "{\"combat\": 1, \"op\": \"attack\", \"actor\": \"x\", \"extra\": 0}".to_string(),
        )],
        1,
        0,
        0,
        "malformed combat attack: unknown field extra",
    );
    // Missing actor beats a missing ability/target.
    check(
        vec![(0, "{\"combat\": 1, \"op\": \"attack\"}".to_string())],
        1,
        0,
        0,
        "malformed combat attack: actor must be a ULID string",
    );
    // Duplicate init beats the unknown-encounter identity on the second init.
    check(
        vec![(0, init_payload(ENCOUNTER)), (1, init_payload(STRANGER))],
        2,
        1,
        1,
        "duplicate combat init",
    );
    // Attack-before-init beats unknown identities without initialization.
    check(
        vec![(0, attack_payload(STRANGER, ABILITY, HERO))],
        1,
        0,
        0,
        "combat not initialized",
    );
    // Actor identity resolves before target identity.
    check(
        vec![
            (0, init_payload(ENCOUNTER)),
            (1, attack_payload(STRANGER, ABILITY, STRANGER)),
        ],
        2,
        1,
        1,
        &format!("unknown combat identity actor: {STRANGER}").to_string(),
    );
    cleanup(&dir);
}

#[test]
fn verify_only_leaves_inputs_and_baselines_untouched() {
    let campaign = copy_campaign("verify-only-campaign");
    let dir = temp_root("verify-only-files");
    let replay = dir.join("check.replay");
    std::fs::copy(combat_replay(), &replay).expect("replay copy must succeed");
    let before_campaign = snapshot_tree(&campaign);
    let before_replay = std::fs::read(&replay).expect("replay must read");
    // A representative failure (unknown op) mutates nothing.
    let bad = dir.join("bad.replay");
    let entries = vec![input(0, "{\"combat\": 1, \"op\": \"retreat\"}")];
    write_replay(&bad, 1, &join_inputs(&entries));
    let out = run(&[
        "replay",
        bad.to_str().expect("utf-8"),
        "--campaign",
        campaign.to_str().expect("utf-8"),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        snapshot_tree(&campaign),
        before_campaign,
        "failures must not rewrite the campaign"
    );
    assert_eq!(
        std::fs::read(&replay).expect("read"),
        before_replay,
        "failures must not rewrite the replay"
    );
    assert!(
        !campaign.join("check.replay").exists(),
        "no output artifact may appear in the campaign"
    );
    assert!(
        !dir.join("check.golden").exists(),
        "no golden may be generated beside the replay"
    );
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
        let golden = dir.join("check.golden");
        std::fs::copy(target_golden(), &golden).expect("golden copy must succeed");
        let before_golden = std::fs::read(&golden).expect("golden must read");
        let out = run(&[
            "replay",
            replay.to_str().expect("utf-8"),
            "--campaign",
            campaign.to_str().expect("utf-8"),
        ]);
        assert!(out.status.success(), "success must exit 0: {out:?}");
        assert_eq!(
            snapshot_tree(&campaign),
            before_campaign,
            "success must not rewrite the campaign"
        );
        assert_eq!(
            std::fs::read(&replay).expect("read"),
            before_replay,
            "success must not rewrite the replay"
        );
        assert_eq!(
            std::fs::read(&golden).expect("read"),
            before_golden,
            "success must not replace the golden"
        );
    }
    cleanup(&campaign);
    cleanup(&dir);
}

/// Real black-box symlink case, compile-time gated to platforms where
/// symlink creation is guaranteed. The injected `WalkFs` seam in `main.rs`
/// covers the same shapes everywhere without runtime skips.
#[cfg(unix)]
#[test]
fn campaign_document_symlink_is_exit_1() {
    use std::os::unix::fs::symlink;
    let campaign = copy_campaign("symlink-document");
    let target = campaign.join("campaign.json");
    let staged = campaign.join("campaign.staged.json");
    std::fs::rename(&target, &staged).expect("stage must succeed");
    symlink(&staged, &target).expect("symlink must create");
    let out = run(&[
        "replay",
        combat_replay().to_str().expect("utf-8"),
        "--campaign",
        campaign.to_str().expect("utf-8"),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "document symlink must exit 1: {out:?}"
    );
    assert!(
        stderr.contains("cannot classify campaign.json: symlink_at_document_path"),
        "{stderr}"
    );
    cleanup(&campaign);
}

/// Non-Unicode campaign roots reach the binary through `args_os` and fail as
/// `io`, never as usage.
#[cfg(unix)]
#[test]
fn non_unicode_campaign_root_is_exit_1_io() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let bad = OsString::from_vec(vec![0x2f, 0x74, 0x6d, 0x70, 0x2f, 0x80]);
    let replay = combat_replay();
    let args = vec![
        OsString::from("replay"),
        replay.into_os_string(),
        OsString::from("--campaign"),
        bad,
    ];
    let out = run_os(&args);
    assert_eq!(
        out.status.code(),
        Some(1),
        "non-Unicode root must exit 1: {out:?}"
    );
    assert_eq!(
        out.stderr,
        b"<campaign>: error[io]: cannot open root <campaign-root>: non_unicode_component\n"
    );
}
