//! Black-box acceptance for `crpgc replay --campaign` over `combat_srd`.
//!
//! These drive the built binary (`env!("CARGO_BIN_EXE_crpgc")`) as a real
//! process, asserting exit codes and exact stream bytes, never combat or
//! replay internals. The checked-in srd replay and both scoped goldens stay
//! read-only; mutation uses private temporary copies with std-only temp
//! conventions and cleanup. No test writes a new expected golden. Native
//! success/divergence tests select the existing scoped baseline at compile
//! time with T009c's cfg guards; portable usage/collection/payload/error
//! tests never read either baseline, never detect the platform at runtime,
//! and never assert cross-platform hash equality.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Authored identities from the checked-in srd fixture (T017a final map).
const HERO: &str = "00000000000000000000000012";
const GOBLIN: &str = "00000000000000000000000013";
const HEAVY: &str = "00000000000000000000000017";
const FOCUS: &str = "00000000000000000000000018";
const ENCOUNTER: &str = "0000000000000000000000001C";
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

/// The `crpg-cli` crate directory, without depending on the working dir.
fn cli_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The `crpg-testkit` crate directory holding replay fixtures and goldens.
fn testkit_dir() -> PathBuf {
    cli_dir().join("..").join("crpg-testkit")
}

/// The checked-in portable srd replay fixture (read-only).
fn srd_replay() -> PathBuf {
    testkit_dir().join("fixtures").join("combat_srd.replay")
}

/// The checked-in `combat_srd` campaign root (read-only).
fn srd_campaign() -> PathBuf {
    cli_dir()
        .join("..")
        .join("..")
        .join("campaigns")
        .join("fixtures")
        .join("combat_srd")
}

/// The checked-in `combat_basic` campaign root (read-only, same-command run).
fn basic_campaign() -> PathBuf {
    cli_dir()
        .join("..")
        .join("..")
        .join("campaigns")
        .join("fixtures")
        .join("combat_basic")
}

/// The checked-in portable basic replay fixture (read-only, same-command).
fn basic_replay() -> PathBuf {
    testkit_dir().join("fixtures").join("combat_basic.replay")
}

/// The supported native srd golden, when this build is one of the two
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
            .join("srd_lite_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden")
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
            .join("srd_lite_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden")
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

/// The supported native basic golden for the same-command run.
fn basic_target_golden() -> PathBuf {
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
        panic!("basic_target_golden is only callable on a supported target")
    }
}

/// A fresh unique temp directory for one test. Tests run in parallel, so the
/// name carries the test's own slug; stale leftovers are removed first.
fn temp_root(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("crpg-srd-cli-{}-{name}", std::process::id()));
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

/// Writes a replay envelope with the srd identity and the given inputs.
fn write_replay(path: &Path, total_ticks: u64, inputs: &str) {
    let text = format!(
        "{{\n  \"format_version\": 1,\n  \"seed\": 285,\n  \"campaign_id\": \"combat-srd\",\n  \"campaign_version\": \"0.1.0\",\n  \"engine_version\": \"0.1.0\",\n  \"total_ticks\": {total_ticks},\n  \"inputs\": [{inputs}\n  ]\n}}\n"
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

fn end_payload(actor: &str) -> String {
    format!("{{\"combat\": 1, \"op\": \"end\", \"actor\": \"{actor}\"}}")
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

#[test]
fn srd_matches_native_golden_with_explicit_paths() {
    let out = run(&[
        "replay",
        srd_replay().to_str().expect("utf-8"),
        "--campaign",
        srd_campaign().to_str().expect("utf-8"),
        "--golden",
        target_golden().to_str().expect("utf-8"),
    ]);
    assert!(out.status.success(), "srd golden must match: {out:?}");
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
fn srd_default_golden_sibling_beside_private_replay() {
    let dir = temp_root("default-golden");
    let replay = dir.join("srd_copy.replay");
    let golden = dir.join("srd_copy.golden");
    std::fs::copy(srd_replay(), &replay).expect("replay copy must succeed");
    std::fs::copy(target_golden(), &golden).expect("golden copy must succeed");
    let out = run(&[
        "replay",
        replay.to_str().expect("utf-8"),
        "--campaign",
        srd_campaign().to_str().expect("utf-8"),
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
fn srd_option_orders_both_match() {
    let replay = srd_replay().to_str().expect("utf-8").to_owned();
    let campaign = srd_campaign().to_str().expect("utf-8").to_owned();
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
fn srd_spaces_renamed_campaign_and_working_directory() {
    // Relocate the campaign to a renamed directory with spaces: no
    // repository-root or directory-name dependency.
    let base = temp_root("spaces base");
    let campaign = base.join("renamed campaign");
    copy_dir(&srd_campaign(), &campaign);
    let replay = base.join("re play.replay");
    let golden = base.join("re play.golden");
    std::fs::copy(srd_replay(), &replay).expect("replay copy must succeed");
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
    copy_dir(&srd_campaign(), &work_campaign);
    std::fs::copy(srd_replay(), work.join("fight.replay")).expect("copy must succeed");
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
fn same_command_verifies_both_campaigns_outside_repo() {
    // Both rulesets through the identical command shape, back to back,
    // from a working directory that is neither campaign's home.
    let dir = temp_root("both-campaigns");
    let elsewhere = temp_root("both-cwd");
    for (replay_src, golden_src, campaign_src, stem) in [
        (
            basic_replay(),
            basic_target_golden(),
            basic_campaign(),
            "basic",
        ),
        (srd_replay(), target_golden(), srd_campaign(), "srd"),
    ] {
        let replay = dir.join(format!("{stem}.replay"));
        let golden = dir.join(format!("{stem}.golden"));
        let campaign = dir.join(format!("{stem} campaign"));
        std::fs::copy(replay_src, &replay).expect("replay copy must succeed");
        std::fs::copy(golden_src, &golden).expect("golden copy must succeed");
        copy_dir(&campaign_src, &campaign);
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
            "{stem} campaign must verify through the same command: {out:?}"
        );
        assert!(out.stdout.is_empty() && out.stderr.is_empty());
    }
    cleanup(&dir);
    cleanup(&elsewhere);
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
fn delayed_lethal_attack_diverges_at_index_9() {
    // Delay only the lethal attack from tick 9 to tick 10; total_ticks stays
    // 14. The run stays valid but first diverges at index 9 against the
    // unchanged target golden.
    let text = std::fs::read_to_string(srd_replay()).expect("fixture must read");
    let mutated = text.replacen("\"tick\": 9", "\"tick\": 10", 1);
    assert_ne!(text, mutated, "the lethal tick must move");
    let dir = temp_root("delayed-lethal");
    let replay = dir.join("delayed.replay");
    std::fs::write(&replay, mutated).expect("mutated replay must write");
    let golden = target_golden();
    let out = run(&[
        "replay",
        replay.to_str().expect("utf-8"),
        "--campaign",
        srd_campaign().to_str().expect("utf-8"),
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
    assert!(stderr.contains("replay 'combat-srd'"), "{stderr}");
    assert!(stderr.contains("diverged at tick 9"), "{stderr}");
    cleanup(&dir);
}

#[test]
fn dead_actor_end_after_lethal_prefix_fails_application() {
    // All ten original inputs plus a hero end at tick 10 (input index 10):
    // the hero died at tick 9, so the end reports DeadActor, not
    // divergence and not OutOfTurn. No --golden flag: the default sibling
    // is absent, proving application fails before golden I/O.
    let text = std::fs::read_to_string(srd_replay()).expect("fixture must read");
    let value: serde_json::Value = serde_json::from_str(&text).expect("fixture parses");
    let inputs = value["inputs"].as_array().expect("input array").len();
    assert_eq!(inputs, 10);
    let dir = temp_root("dead-end");
    let replay = dir.join("dead.replay");
    let mut entries: Vec<String> = Vec::new();
    for (tick, payload) in [
        (0, init_payload(ENCOUNTER)),
        (1, attack_payload(GOBLIN, FOCUS, GOBLIN)),
        (2, attack_payload(GOBLIN, FOCUS, GOBLIN)),
        (3, attack_payload(GOBLIN, HEAVY, HERO)),
        (4, end_payload(HERO)),
        (5, attack_payload(GOBLIN, HEAVY, HERO)),
        (6, end_payload(HERO)),
        (7, attack_payload(GOBLIN, HEAVY, HERO)),
        (8, attack_payload(HERO, HEAVY, GOBLIN)),
        (9, attack_payload(GOBLIN, HEAVY, HERO)),
        (10, end_payload(HERO)),
    ] {
        entries.push(input(tick, &payload));
    }
    write_replay(&replay, 14, &join_inputs(&entries));
    let out = run(&[
        "replay",
        replay.to_str().expect("utf-8"),
        "--campaign",
        srd_campaign().to_str().expect("utf-8"),
    ]);
    assert_replay_failure(
        &out,
        10,
        10,
        "end failed: DeadActor at combatants/EntityId { index: 0, generation: 1 }",
    );
    cleanup(&dir);
}

#[test]
fn out_of_turn_end_names_tick_index_and_reason() {
    // The tick-4 end with the goblin as actor: the hero holds the turn
    // then, so the input fails application at tick/index 4, not divergence.
    // The needle pins the pretty-printed tick-4 payload, unique in the file.
    let text = std::fs::read_to_string(srd_replay()).expect("fixture must read");
    let needle =
        "\"tick\": 4,\n      \"payload\": {\n        \"actor\": \"00000000000000000000000012\"";
    let replacement =
        "\"tick\": 4,\n      \"payload\": {\n        \"actor\": \"00000000000000000000000013\"";
    let mutated = text.replacen(needle, replacement, 1);
    assert_ne!(text, mutated, "the tick-4 end actor must change");
    let dir = temp_root("out-of-turn-end");
    let replay = dir.join("turn.replay");
    std::fs::write(&replay, mutated).expect("mutated replay must write");
    let out = run(&[
        "replay",
        replay.to_str().expect("utf-8"),
        "--campaign",
        srd_campaign().to_str().expect("utf-8"),
    ]);
    assert_replay_failure(&out, 4, 4, "end failed: OutOfTurn at combat/active");
    cleanup(&dir);
}

#[test]
fn end_error_matrix_names_tick_index_and_reason() {
    let campaign = srd_campaign().to_str().expect("utf-8").to_owned();
    let dir = temp_root("end-matrix");
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
            // No --golden flag: the default sibling is absent, so a golden
            // read would fail as I/O — application must fail first.
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
        vec![(0, "{\"op\": \"end\"}".to_string())],
        1,
        0,
        0,
        "unsupported combat payload version",
    );
    check(
        vec![(0, "{\"combat\": 2, \"op\": \"end\"}".to_string())],
        1,
        0,
        0,
        "unsupported combat payload version",
    );
    check(
        vec![(0, "{\"combat\": 1, \"op\": \"rest\"}".to_string())],
        1,
        0,
        0,
        "unknown combat op",
    );
    // Unknown field (lexically smallest) beats a missing actor.
    check(
        vec![(
            0,
            "{\"combat\": 1, \"op\": \"end\", \"zz\": 0, \"aa\": 0}".to_string(),
        )],
        1,
        0,
        0,
        "malformed combat end: unknown field aa",
    );
    // Missing, non-string, and invalid actors in order.
    check(
        vec![(0, "{\"combat\": 1, \"op\": \"end\"}".to_string())],
        1,
        0,
        0,
        "malformed combat end: actor must be a ULID string",
    );
    check(
        vec![(
            0,
            "{\"combat\": 1, \"op\": \"end\", \"actor\": 12}".to_string(),
        )],
        1,
        0,
        0,
        "malformed combat end: actor must be a ULID string",
    );
    check(
        vec![(
            0,
            "{\"combat\": 1, \"op\": \"end\", \"actor\": \"bad\"}".to_string(),
        )],
        1,
        0,
        0,
        "malformed combat end: invalid actor",
    );
    // A valid actor before init fails the guard before binding lookup.
    check(
        vec![(0, end_payload(GOBLIN))],
        1,
        0,
        0,
        "combat not initialized",
    );
    // Initialized, syntactically valid but unbound identity.
    check(
        vec![(0, init_payload(ENCOUNTER)), (1, end_payload(STRANGER))],
        2,
        1,
        1,
        "unknown combat identity actor: 00000000000000000000000000",
    );
    // Initialized, bound actor out of turn (hero holds no turn at tick 1).
    check(
        vec![(0, init_payload(ENCOUNTER)), (1, end_payload(HERO))],
        2,
        1,
        1,
        "end failed: OutOfTurn at combat/active",
    );
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
fn verify_only_leaves_srd_trees_untouched() {
    // Snapshot the private inputs and baselines around a successful run:
    // verify-only means every tree is byte-identical afterwards.
    let dir = temp_root("verify-only");
    let replay = dir.join("srd.replay");
    let golden = dir.join("srd.golden");
    let campaign = dir.join("srd campaign");
    std::fs::copy(srd_replay(), &replay).expect("replay copy must succeed");
    std::fs::copy(target_golden(), &golden).expect("golden copy must succeed");
    copy_dir(&srd_campaign(), &campaign);
    let before = snapshot_tree(&dir);
    let out = run(&[
        "replay",
        replay.to_str().expect("utf-8"),
        "--campaign",
        campaign.to_str().expect("utf-8"),
        "--golden",
        golden.to_str().expect("utf-8"),
    ]);
    assert!(out.status.success(), "srd run must verify: {out:?}");
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    assert_eq!(
        before,
        snapshot_tree(&dir),
        "verify-only must change nothing"
    );
    cleanup(&dir);
}
