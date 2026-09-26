//! End-to-end tests for `crpgc lock` (T013).
//!
//! These drive the built binary — `env!("CARGO_BIN_EXE_crpgc")` — as a black
//! box through `std::process::Command`, asserting exit codes and exact stream
//! bytes. The explicit flat-catalog adapter reads `campaign.json`,
//! `assets/assets.lock`, and the supplied catalog in that order, resolves
//! through the existing `make_campaign_lock`, and creates, replaces, or
//! no-ops `campaign.lock`. Expected lock bytes and digests are pinned
//! through real data calls; reordered-catalog equality proves forwarding
//! with no CLI resolver oracle. Fixtures are read-only inputs; every case
//! operates on private temp directories.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
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

/// The `crpg-cli` crate directory, without depending on the working dir.
fn cli_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The `crpg-data` crate directory: the clean fixture lives here.
fn data_dir() -> PathBuf {
    cli_dir()
        .join("..")
        .join("crpg-data")
        .canonicalize()
        .expect("crpg-data crate dir must exist")
}

/// T010's clean fixture campaign root, consumed read-only except for copies.
fn valid_root() -> PathBuf {
    data_dir()
        .join("tests")
        .join("fixtures")
        .join("one_area_one_creature")
}

/// A fresh unique temp directory for one test.
fn temp_root(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("crpg-lock-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp root must create");
    path
}

/// Recursively copies `src` to `dst` with `std` only.
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

/// Maps every file under `root` to its bytes.
fn snapshot_dir(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut map = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("snapshot source must list")
            .map(|entry| entry.expect("snapshot entry").path())
            .collect();
        entries.sort();
        for entry in entries {
            if entry.is_dir() {
                stack.push(entry);
            } else {
                let relative = entry.strip_prefix(root).expect("prefix").to_path_buf();
                map.insert(
                    relative,
                    std::fs::read(&entry).expect("snapshot file must read"),
                );
            }
        }
    }
    map
}

const LOCK_USAGE: &str = "crpgc: usage: crpgc lock [<campaign-root>] --catalog <catalog-path>\n";
const EXPECTED_CAMPAIGN: &str = "crpgc lock: expected campaign document\n";
const INVALID_CATALOG: &str = "crpgc lock: invalid catalog\n";

const ZERO_CHECKSUM: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Minimal campaign manifest with the given `requires` JSON fragment.
/// Typed shape only; the lock operation never validates references.
fn campaign_json(requires: &str) -> Vec<u8> {
    format!(
        "{{\"engine\":\">=0.1.0\",\"entry\":{{\"area\":\"00000000000000000000000003\",\"spawn\":\"00000000000000000000000005\",\"world\":\"00000000000000000000000002\"}},\"id\":\"00000000000000000000000001\",\"name\":\"fixture.campaign\",\"package\":\"fixture.one-area\",\"requires\":{requires},\"schema\":\"crpg.campaign/1\",\"slug\":\"campaign\",\"version\":\"0.1.0\"}}"
    )
    .into_bytes()
}

const ASSETS_LOCK: &str = "{\"assets\":{},\"schema\":\"crpg.assets-lock/1\"}";

/// Writes the two lock inputs plus the catalog into a fresh temp campaign.
fn lock_inputs(name: &str, campaign: &[u8], assets: &[u8], catalog: &[u8]) -> (PathBuf, PathBuf) {
    let root = temp_root(name);
    let trial = root.join("campaign");
    std::fs::create_dir_all(trial.join("assets")).expect("dirs must create");
    std::fs::write(trial.join("campaign.json"), campaign).expect("campaign must write");
    std::fs::write(trial.join("assets").join("assets.lock"), assets).expect("assets must write");
    let catalog_path = root.join("catalog.json");
    std::fs::write(&catalog_path, catalog).expect("catalog must write");
    (trial, catalog_path)
}

/// Runs `lock` with Unicode path spellings.
fn run_lock(root: &Path, catalog: &Path) -> Output {
    run(&[
        "lock",
        root.to_str().expect("unicode"),
        "--catalog",
        catalog.to_str().expect("unicode"),
    ])
}

/// Expected lock bytes through real data calls: the assets digest and the
/// resolution come from data, so equality proves forwarding.
fn expected_lock_bytes(candidates: &[crpg_data::PackageCandidate]) -> Vec<u8> {
    let assets = crpg_data::read_assets_lock(ASSETS_LOCK.as_bytes()).expect("test assets parse");
    let campaign =
        match crpg_data::read_document(&campaign_json("[]")).expect("test campaign parses") {
            crpg_data::Document::Campaign(campaign) => campaign,
            other => panic!("expected campaign: {other:?}"),
        };
    let lock = crpg_data::make_campaign_lock(&campaign.requires, candidates, &assets)
        .expect("test lock resolves");
    // The new lock's note is the data constructor's None.
    assert!(lock.note.is_none());
    crpg_data::write_campaign_lock(&lock).expect("test lock writes")
}

#[test]
fn lock_usage_matrix_reports_one_line_with_exit_2() {
    let cases: Vec<Vec<&str>> = vec![
        vec!["lock"],
        vec!["lock", "some/root"],
        vec!["lock", "--catalog"],
        vec!["lock", "--catalog", "a", "--catalog", "b"],
        vec!["lock", "a", "b", "--catalog", "c"],
        vec!["lock", "--bogus", "--catalog", "c"],
        vec!["lock", "--catalog", "c", "--bogus"],
        vec!["lock", "--", "--catalog", "c"],
        vec!["lock", "--catalog=c", "root"],
    ];
    for argv in &cases {
        let output = run(argv);
        assert_eq!(output.status.code(), Some(2), "argv {argv:?}");
        assert!(output.stdout.is_empty(), "argv {argv:?}");
        assert_eq!(output.stderr, LOCK_USAGE.as_bytes(), "argv {argv:?}");
    }
}

#[test]
fn lock_flag_shaped_catalog_values_are_usage_before_filesystem_access() {
    // The reviewed `lock --catalog --help` case plus version/terminator/
    // short-flag/duplicate forms: usage exit 2 even when the root does not
    // exist, proving parsing completes before any filesystem access.
    for value in [
        "--help",
        "--version",
        "--",
        "-x",
        "--catalog",
        "--root",
        "--bogus",
    ] {
        let output = run(&["lock", "--catalog", value]);
        assert_eq!(output.status.code(), Some(2), "value {value:?}");
        assert!(output.stdout.is_empty(), "value {value:?}");
        assert_eq!(output.stderr, LOCK_USAGE.as_bytes(), "value {value:?}");
    }
    let output = run(&["lock", "--catalog", "--help"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stderr, LOCK_USAGE.as_bytes());
    // An explicit relative prefix escapes a dash-leading catalog path: it
    // reaches I/O (exit 1, not usage) instead of failing as usage.
    let (trial, _) = lock_inputs(
        "dash-catalog",
        &campaign_json("[]"),
        ASSETS_LOCK.as_bytes(),
        b"[]",
    );
    let dash = "./-dash-catalog.json";
    let output = run_lock(&trial, Path::new(dash));
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_ne!(output.stderr, LOCK_USAGE.as_bytes());
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
}

#[cfg(target_os = "linux")]
#[test]
fn lock_rejects_symlinked_catalog_ancestor_without_traversal() {
    // Real Linux ancestor case (compile-time selected, no runtime skip):
    // a catalog reached through a symlinked parent is rejected with the
    // portable `<catalog>` I/O diagnostic, never `invalid catalog`. This
    // mirrors the reviewed `/proc/self/cwd` traversal, which also reads
    // through a symlinked ancestor.
    use std::os::unix::fs::symlink;
    let (trial, _) = lock_inputs(
        "symlink-ancestor",
        &campaign_json("[]"),
        ASSETS_LOCK.as_bytes(),
        b"[]",
    );
    let outer = trial.parent().expect("parent").to_path_buf();
    let real = outer.join("real");
    std::fs::create_dir_all(&real).expect("real dir must create");
    let catalog_real = real.join("catalog.json");
    std::fs::write(&catalog_real, b"[]").expect("catalog must write");
    let link = outer.join("link");
    symlink(&real, &link).expect("symlink creation is guaranteed on Linux");
    let via_link = link.join("catalog.json");
    assert!(via_link.exists(), "symlinked-ancestor file must exist");
    let output = run_lock(&trial, &via_link);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("utf-8");
    assert!(
        stderr.contains("cannot read <catalog>: symlink_at_document_path"),
        "must reject the symlinked ancestor, got: {stderr}"
    );
    assert!(
        !trial.join("campaign.lock").exists(),
        "symlinked ancestor must not create output"
    );
    let _ = std::fs::remove_dir_all(&outer);
}

#[cfg(target_os = "linux")]
#[test]
fn lock_rejects_proc_self_cwd_catalog_ancestor() {
    // The exact reviewed read-only reproduction shape: the catalog path
    // traverses `/proc/self/cwd` (itself a symlink) and must be rejected
    // with the portable diagnostic instead of reaching `invalid catalog`.
    let (trial, _) = lock_inputs(
        "proc-ancestor",
        &campaign_json("[]"),
        ASSETS_LOCK.as_bytes(),
        b"[]",
    );
    let via_proc = PathBuf::from("/proc/self/cwd/tests/inputs/llm-trials/item.json");
    assert!(
        via_proc.exists(),
        "reviewed /proc/self/cwd catalog must exist when tests run in the crate dir"
    );
    let output = run_lock(&trial, &via_proc);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("utf-8");
    assert!(
        stderr.contains("cannot read <catalog>: symlink_at_document_path"),
        "must reject /proc/self/cwd traversal, got: {stderr}"
    );
    assert!(!trial.join("campaign.lock").exists());
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
}

#[test]
fn lock_creates_absent_output_with_expected_bytes() {
    let (trial, catalog) = lock_inputs(
        "create",
        &campaign_json("[]"),
        ASSETS_LOCK.as_bytes(),
        b"[]",
    );
    assert!(!trial.join("campaign.lock").exists());
    let output = run_lock(&trial, &catalog);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    let created = std::fs::read(trial.join("campaign.lock")).expect("lock must be created");
    assert_eq!(created, expected_lock_bytes(&[]));
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
}

#[test]
fn lock_replaces_malformed_output_and_no_ops_on_equality() {
    let (trial, catalog) = lock_inputs(
        "replace",
        &campaign_json("[]"),
        ASSETS_LOCK.as_bytes(),
        b"[]",
    );
    std::fs::write(trial.join("campaign.lock"), b"{ not a lock").expect("must plant malformed");
    let output = run_lock(&trial, &catalog);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let replaced = std::fs::read(trial.join("campaign.lock")).expect("must read");
    assert_eq!(replaced, expected_lock_bytes(&[]));
    // A second run over identical bytes is a silent no-op.
    let again = run_lock(&trial, &catalog);
    assert_eq!(again.status.code(), Some(0));
    assert!(again.stdout.is_empty() && again.stderr.is_empty());
    assert_eq!(
        std::fs::read(trial.join("campaign.lock")).expect("must read"),
        replaced
    );
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
}

#[test]
fn lock_replaces_stale_output_after_requirement_change() {
    // A lock created for empty requirements is stale once the manifest
    // requires a package; with a satisfying catalog the output is replaced.
    let root = temp_root("stale");
    let trial = root.join("campaign");
    std::fs::create_dir_all(trial.join("assets")).expect("dirs must create");
    std::fs::write(trial.join("campaign.json"), campaign_json("[]")).expect("must write");
    std::fs::write(trial.join("assets").join("assets.lock"), ASSETS_LOCK).expect("must write");
    let catalog_path = root.join("catalog.json");
    std::fs::write(&catalog_path, b"[]").expect("must write");
    assert_eq!(run_lock(&trial, &catalog_path).status.code(), Some(0));
    let stale = std::fs::read(trial.join("campaign.lock")).expect("must read");
    std::fs::write(
        trial.join("campaign.json"),
        campaign_json("[{\"kind\":\"module\",\"package\":\"test.dep\",\"version\":\">=1.0.0\"}]"),
    )
    .expect("must write");
    let candidate = format!(
        "[{{\"kind\":\"module\",\"package\":\"test.dep\",\"version\":\"1.5.0\",\"checksum\":\"{ZERO_CHECKSUM}\"}}]"
    );
    std::fs::write(&catalog_path, candidate).expect("must write");
    let output = run_lock(&trial, &catalog_path);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let replaced = std::fs::read(trial.join("campaign.lock")).expect("must read");
    assert_ne!(replaced, stale, "stale output must be replaced");
    let lock: serde_json::Value = serde_json::from_slice(&replaced).expect("must parse");
    assert_eq!(
        lock["packages"][0]["version"],
        serde_json::Value::String("1.5.0".into())
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn lock_resolves_repeated_ranges_and_build_metadata_ties() {
    // Repeated ranges intersect: only 1.5.0 satisfies both.
    let requires = "[{\"kind\":\"module\",\"package\":\"test.dep\",\"version\":\">=1.0.0\"},{\"kind\":\"module\",\"package\":\"test.dep\",\"version\":\"<2.0.0\"}]";
    let catalog = format!(
        "[{{\"kind\":\"module\",\"package\":\"test.dep\",\"version\":\"2.0.0\",\"checksum\":\"{ZERO_CHECKSUM}\"}},{{\"kind\":\"module\",\"package\":\"test.dep\",\"version\":\"1.5.0\",\"checksum\":\"{ZERO_CHECKSUM}\"}}]"
    );
    let (trial, catalog_path) = lock_inputs(
        "ranges",
        &campaign_json(requires),
        ASSETS_LOCK.as_bytes(),
        catalog.as_bytes(),
    );
    let output = run_lock(&trial, &catalog_path);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lock: serde_json::Value =
        serde_json::from_slice(&std::fs::read(trial.join("campaign.lock")).expect("must read"))
            .expect("must parse");
    assert_eq!(
        lock["packages"][0]["version"],
        serde_json::Value::String("1.5.0".into())
    );
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
    // Build-metadata precedence ties select the lexically greatest complete
    // version text.
    let requires = "[{\"kind\":\"module\",\"package\":\"test.dep\",\"version\":\">=1.0.0\"}]";
    let catalog = format!(
        "[{{\"kind\":\"module\",\"package\":\"test.dep\",\"version\":\"1.0.0+aaa\",\"checksum\":\"{ZERO_CHECKSUM}\"}},{{\"kind\":\"module\",\"package\":\"test.dep\",\"version\":\"1.0.0+zzz\",\"checksum\":\"{ZERO_CHECKSUM}\"}}]"
    );
    let (trial, catalog_path) = lock_inputs(
        "tie",
        &campaign_json(requires),
        ASSETS_LOCK.as_bytes(),
        catalog.as_bytes(),
    );
    let output = run_lock(&trial, &catalog_path);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lock: serde_json::Value =
        serde_json::from_slice(&std::fs::read(trial.join("campaign.lock")).expect("must read"))
            .expect("must parse");
    assert_eq!(
        lock["packages"][0]["version"],
        serde_json::Value::String("1.0.0+zzz".into())
    );
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
}

#[test]
fn lock_reordered_catalog_yields_identical_bytes() {
    let requires = "[{\"kind\":\"module\",\"package\":\"test.dep\",\"version\":\">=1.0.0\"},{\"kind\":\"module\",\"package\":\"test.other\",\"version\":\">=2.0.0\"}]";
    let first = format!(
        "[{{\"kind\":\"module\",\"package\":\"test.dep\",\"version\":\"1.5.0\",\"checksum\":\"{ZERO_CHECKSUM}\"}},{{\"kind\":\"module\",\"package\":\"test.other\",\"version\":\"2.1.0\",\"checksum\":\"{ZERO_CHECKSUM}\"}}]"
    );
    let second = format!(
        "[{{\"kind\":\"module\",\"package\":\"test.other\",\"version\":\"2.1.0\",\"checksum\":\"{ZERO_CHECKSUM}\"}},{{\"kind\":\"module\",\"package\":\"test.dep\",\"version\":\"1.5.0\",\"checksum\":\"{ZERO_CHECKSUM}\"}}]"
    );
    let (trial, catalog_path) = lock_inputs(
        "reorder-a",
        &campaign_json(requires),
        ASSETS_LOCK.as_bytes(),
        first.as_bytes(),
    );
    assert_eq!(run_lock(&trial, &catalog_path).status.code(), Some(0));
    let a = std::fs::read(trial.join("campaign.lock")).expect("must read");
    let root_a = trial.parent().expect("parent").to_path_buf();
    let (trial, catalog_path) = lock_inputs(
        "reorder-b",
        &campaign_json(requires),
        ASSETS_LOCK.as_bytes(),
        second.as_bytes(),
    );
    assert_eq!(run_lock(&trial, &catalog_path).status.code(), Some(0));
    let b = std::fs::read(trial.join("campaign.lock")).expect("must read");
    assert_eq!(a, b, "catalog order must not affect the lock");
    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
}

#[test]
fn lock_unsatisfied_constraints_fail_without_output_writes() {
    let requires = "[{\"kind\":\"module\",\"package\":\"test.dep\",\"version\":\">=1.0.0\"}]";
    let (trial, catalog) = lock_inputs(
        "unsatisfied",
        &campaign_json(requires),
        ASSETS_LOCK.as_bytes(),
        b"[]",
    );
    let before = snapshot_dir(&trial);
    let output = run_lock(&trial, &catalog);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    // Exact forwarded resolver diagnostic, not merely nonempty stderr.
    assert_eq!(
        output.stderr,
        b"<campaign>: error[unresolved_package]: unresolved test.dep: >=1.0.0\n"
    );
    assert!(
        !trial.join("campaign.lock").exists(),
        "no output on resolution failure"
    );
    assert_eq!(snapshot_dir(&trial), before, "inputs must be unchanged");
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
}

#[test]
fn lock_conflicting_candidates_fail_without_output_writes() {
    let requires = "[{\"kind\":\"module\",\"package\":\"test.dep\",\"version\":\">=1.0.0\"}]";
    let catalog = format!(
        "[{{\"kind\":\"module\",\"package\":\"test.dep\",\"version\":\"1.5.0\",\"checksum\":\"{ZERO_CHECKSUM}\"}},{{\"kind\":\"campaign\",\"package\":\"test.dep\",\"version\":\"1.5.0\",\"checksum\":\"{ZERO_CHECKSUM}\"}}]"
    );
    let (trial, catalog_path) = lock_inputs(
        "conflict",
        &campaign_json(requires),
        ASSETS_LOCK.as_bytes(),
        catalog.as_bytes(),
    );
    let output = run_lock(&trial, &catalog_path);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    // Exact forwarded conflict diagnostic, not merely nonempty stderr.
    assert_eq!(
        output.stderr,
        b"<campaign>: error[candidate_conflict]: conflicting candidate test.dep@1.5.0\n"
    );
    assert!(!trial.join("campaign.lock").exists());
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
}

#[test]
fn lock_empty_output_matches_independently_specified_bytes_and_digest() {
    // Independently specified canonical bytes for the empty catalog/no
    // dependency case, alongside the API-forwarding comparison: the digest
    // is the checked-in empty-assets digest from `migration_v1/expected.json`
    // (not blessed from actuals), keys are canonical lexical order, one LF.
    const EXPECTED_EMPTY_LOCK: &str = "{\n  \"assets_lock\": \"957bc137f1abb3cde6cee277d10c099b1dcd2f8814a5fd0e29b1df3506ee44fb\",\n  \"packages\": [],\n  \"schema\": \"crpg.campaign-lock/1\"\n}\n";
    let (trial, catalog) = lock_inputs(
        "empty-bytes",
        &campaign_json("[]"),
        ASSETS_LOCK.as_bytes(),
        b"[]",
    );
    let output = run_lock(&trial, &catalog);
    assert_eq!(output.status.code(), Some(0));
    let created = std::fs::read(trial.join("campaign.lock")).expect("must read");
    assert_eq!(created, EXPECTED_EMPTY_LOCK.as_bytes());
    // The digest matches the data-owned assets-lock digest, proving the
    // hash authority is forwarded, not reimplemented.
    let assets = crpg_data::read_assets_lock(ASSETS_LOCK.as_bytes()).expect("assets parse");
    let digest = crpg_data::assets_lock_digest(&assets).expect("digest computes");
    assert_eq!(
        format!("{digest}"),
        "957bc137f1abb3cde6cee277d10c099b1dcd2f8814a5fd0e29b1df3506ee44fb"
    );
    let lock: serde_json::Value = serde_json::from_slice(&created).expect("lock must parse");
    assert_eq!(
        lock["assets_lock"],
        serde_json::Value::String(format!("{digest}"))
    );
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
}

#[test]
fn lock_rejects_bad_inputs_with_exact_lines_and_zero_writes() {
    // Wrong campaign document kind.
    let (trial, catalog) = lock_inputs(
        "wrong-kind",
        br#"{"areas":[],"id":"00000000000000000000000002","name":"k","schema":"crpg.world/1","slug":"world","variables":[]}"#,
        ASSETS_LOCK.as_bytes(),
        b"[]",
    );
    let output = run_lock(&trial, &catalog);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, EXPECTED_CAMPAIGN.as_bytes());
    assert!(!trial.join("campaign.lock").exists());
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
    // Catalog syntax, non-array, unknown-field, and duplicate-field shapes
    // share the single invalid-catalog line without serde prose.
    let bad_catalogs: Vec<Vec<u8>> = vec![
        b"{".to_vec(),
        b"{}".to_vec(),
        b"null".to_vec(),
        b"[}".to_vec(),
        format!(
            "[{{\"kind\":\"module\",\"package\":\"test.dep\",\"version\":\"1.0.0\",\"checksum\":\"{ZERO_CHECKSUM}\",\"extra\":1}}]"
        )
        .into_bytes(),
        format!(
            "[{{\"kind\":\"module\",\"kind\":\"campaign\",\"package\":\"test.dep\",\"version\":\"1.0.0\",\"checksum\":\"{ZERO_CHECKSUM}\"}}]"
        )
        .into_bytes(),
        b"\"[]\"".to_vec(),
    ];
    for (index, bad) in bad_catalogs.iter().enumerate() {
        let (trial, catalog) = lock_inputs(
            &format!("bad-catalog-{index}"),
            &campaign_json("[]"),
            ASSETS_LOCK.as_bytes(),
            bad,
        );
        let output = run_lock(&trial, &catalog);
        assert_eq!(output.status.code(), Some(1), "catalog {bad:?}");
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, INVALID_CATALOG.as_bytes(), "catalog {bad:?}");
        assert!(!trial.join("campaign.lock").exists(), "catalog {bad:?}");
        let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
    }
    // Malformed campaign and assets inputs fail as data diagnostics with no
    // output writes.
    let (trial, catalog) = lock_inputs("bad-manifest", b"{ broken", ASSETS_LOCK.as_bytes(), b"[]");
    let output = run_lock(&trial, &catalog);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty() && !output.stderr.is_empty());
    assert!(!trial.join("campaign.lock").exists());
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
    let (trial, catalog) = lock_inputs("bad-assets", &campaign_json("[]"), b"{ broken", b"[]");
    let output = run_lock(&trial, &catalog);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty() && !output.stderr.is_empty());
    assert!(!trial.join("campaign.lock").exists());
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
}

#[test]
fn lock_keeps_inputs_and_unrelated_bytes_unchanged() {
    let root = temp_root("unchanged");
    let trial = root.join("campaign");
    copy_dir(&valid_root(), &trial);
    std::fs::remove_file(trial.join("campaign.lock")).expect("must remove lock");
    let sentinel = trial.join("notes.txt");
    std::fs::write(&sentinel, b"unrelated").expect("must write sentinel");
    let before = snapshot_dir(&trial);
    let catalog_path = root.join("catalog.json");
    std::fs::write(&catalog_path, b"[]").expect("must write");
    let catalog_before = std::fs::read(&catalog_path).expect("must read");
    let output = run_lock(&trial, &catalog_path);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after = snapshot_dir(&trial);
    for (relative, bytes) in &before {
        if relative == &PathBuf::from("campaign.lock") {
            continue;
        }
        assert_eq!(after.get(relative), Some(bytes), "must keep {relative:?}");
    }
    assert_eq!(
        std::fs::read(&catalog_path).expect("must read"),
        catalog_before
    );
    assert_eq!(std::fs::read(&sentinel).expect("must read"), b"unrelated");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn lock_refuses_output_that_is_also_an_input() {
    let root = temp_root("input-output-alias");
    let trial = root.join("campaign");
    std::fs::create_dir_all(trial.join("assets")).expect("dirs must create");
    std::fs::write(trial.join("campaign.json"), campaign_json("[]")).expect("campaign must write");
    std::fs::write(trial.join("assets").join("assets.lock"), ASSETS_LOCK)
        .expect("assets must write");
    let output_path = trial.join("campaign.lock");
    std::fs::write(&output_path, b"[]").expect("catalog/output must write");
    let before = std::fs::read(&output_path).expect("alias must read");
    let output = run_lock(&trial, &output_path);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"campaign.lock: error[io]: cannot check campaign.lock: source_changed\n"
    );
    assert_eq!(
        std::fs::read(&output_path).expect("alias must reread"),
        before,
        "catalog bytes must not be replaced by output bytes"
    );
    let dotted_alias = trial.join(".").join("campaign.lock");
    let output = run_lock(&trial, &dotted_alias);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        output.stderr,
        b"campaign.lock: error[io]: cannot check campaign.lock: source_changed\n"
    );
    assert_eq!(
        std::fs::read(&output_path).expect("normalized alias must reread"),
        before
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn lock_refuses_hard_link_aliases_of_every_input() {
    for input in ["campaign", "assets", "catalog"] {
        let (trial, catalog) = lock_inputs(
            &format!("hard-link-{input}"),
            &campaign_json("[]"),
            ASSETS_LOCK.as_bytes(),
            b"[]",
        );
        let input_path = match input {
            "campaign" => trial.join("campaign.json"),
            "assets" => trial.join("assets").join("assets.lock"),
            "catalog" => catalog.clone(),
            _ => unreachable!(),
        };
        let output_path = trial.join("campaign.lock");
        std::fs::hard_link(&input_path, &output_path).expect("hard link must create");
        let before = std::fs::read(&input_path).expect("input must read");
        let output = run_lock(&trial, &catalog);
        assert_eq!(output.status.code(), Some(1), "input {input}");
        assert!(output.stdout.is_empty(), "input {input}");
        assert_eq!(
            output.stderr,
            b"campaign.lock: error[io]: cannot check campaign.lock: source_changed\n",
            "input {input}"
        );
        assert_eq!(
            std::fs::read(&input_path).expect("input must reread"),
            before,
            "input {input} must remain unchanged"
        );
        assert_eq!(
            std::fs::read(&output_path).expect("alias must reread"),
            before,
            "alias {input} must remain unchanged"
        );
        let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
    }
}

#[test]
fn lock_nonregular_output_is_a_check_failure() {
    let (trial, catalog) = lock_inputs(
        "dir-output",
        &campaign_json("[]"),
        ASSETS_LOCK.as_bytes(),
        b"[]",
    );
    std::fs::create_dir(trial.join("campaign.lock")).expect("must plant directory");
    let output = run_lock(&trial, &catalog);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("utf-8"),
        "campaign.lock: error[io]: cannot check campaign.lock: not_a_file\n"
    );
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
}

#[test]
fn lock_runs_with_an_implicit_dot_root() {
    let root = temp_root("dot-root");
    let trial = root.join("campaign");
    copy_dir(&valid_root(), &trial);
    std::fs::remove_file(trial.join("campaign.lock")).expect("must remove lock");
    let catalog_path = trial.join("catalog.json");
    std::fs::write(&catalog_path, b"[]").expect("must write");
    let output = Command::new(crpgc())
        .args(["lock", "--catalog", "catalog.json"])
        .current_dir(&trial)
        .output()
        .expect("spawning crpgc must succeed");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(trial.join("campaign.lock").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn lock_missing_inputs_are_domain_failures_not_usage() {
    let missing = temp_root("missing-root");
    let _ = std::fs::remove_dir_all(&missing);
    let catalog = temp_root("missing-catalog").join("catalog.json");
    let output = run(&[
        "lock",
        missing.to_str().expect("unicode"),
        "--catalog",
        catalog.to_str().expect("unicode"),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
    let _ = std::fs::remove_dir_all(catalog.parent().expect("parent"));
}
