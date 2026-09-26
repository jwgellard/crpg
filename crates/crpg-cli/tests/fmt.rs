//! End-to-end tests for `crpgc fmt` (T013).
//!
//! These drive the built binary — `env!("CARGO_BIN_EXE_crpgc")` — as a black
//! box through `std::process::Command`, asserting exit codes and exact stream
//! bytes. `fmt` shares migrate's collect -> load -> serialize -> key-set
//! check -> byte-diff plan and the T012b writer discipline; `--check` is the
//! same preflight without opening files for writing. Fixtures are read-only
//! inputs; every mutating case operates on a private copy and the migration
//! golden is consumed read-only, never generated from actual output.

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

/// The `crpg-data` crate directory: fixtures, manifest, and goldens live here.
fn data_dir() -> PathBuf {
    cli_dir()
        .join("..")
        .join("crpg-data")
        .canonicalize()
        .expect("crpg-data crate dir must exist")
}

/// T012a's fixture campaigns, consumed read-only except for temp copies.
fn fixtures_dir() -> PathBuf {
    data_dir().join("tests").join("fixtures")
}

/// The canonical-current T010 fixture campaign root.
fn valid_root() -> PathBuf {
    fixtures_dir().join("one_area_one_creature")
}

/// The authoritative old T012a campaign root.
fn migration_v1_root() -> PathBuf {
    fixtures_dir().join("migration_v1").join("campaign")
}

/// The T012a expected canonical output map.
fn migration_expected_path() -> PathBuf {
    fixtures_dir().join("migration_v1").join("expected.json")
}

/// The fifteen-finding T011a fixture campaign root.
fn broken_root() -> PathBuf {
    fixtures_dir().join("broken_references")
}

/// T011a's checked-in canonical diagnostic snapshot for the broken fixture.
fn broken_snapshot_path() -> PathBuf {
    data_dir()
        .join("tests")
        .join("snapshots")
        .join("broken_references.diagnostics.json")
}

/// A fresh unique temp directory for one test.
fn temp_root(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("crpg-fmt-{}-{name}", std::process::id()));
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

/// Copies the v1 campaign to a temp root.
fn copy_v1_fixture(name: &str) -> PathBuf {
    let dst = temp_root(name).join("campaign");
    copy_dir(&migration_v1_root(), &dst);
    dst
}

/// Reads the T012a expected canonical output map.
fn migration_expected() -> BTreeMap<String, String> {
    let bytes = std::fs::read(migration_expected_path()).expect("expected.json must read");
    serde_json::from_slice(&bytes).expect("expected.json must parse")
}

const FMT_USAGE: &str = "crpgc: usage: crpgc fmt [<campaign-root>] [--check]\n";

#[test]
fn fmt_usage_matrix_reports_one_line_with_exit_2() {
    let cases: Vec<Vec<&str>> = vec![
        vec!["fmt", "a", "b"],
        vec!["fmt", "--check", "--check"],
        vec!["fmt", "--bogus"],
        vec!["fmt", "a", "--bogus"],
        vec!["fmt", "--"],
        vec!["fmt", "--help"],
    ];
    for argv in &cases {
        let output = run(argv);
        assert_eq!(output.status.code(), Some(2), "argv {argv:?}");
        assert!(output.stdout.is_empty(), "argv {argv:?}");
        assert_eq!(output.stderr, FMT_USAGE.as_bytes(), "argv {argv:?}");
    }
}

#[test]
fn fmt_check_is_silent_and_read_only_on_clean_input() {
    let root = temp_root("check-clean");
    let trial = root.join("campaign");
    copy_dir(&valid_root(), &trial);
    let before = snapshot_dir(&trial);
    let output = run(&["fmt", trial.to_str().expect("unicode"), "--check"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(snapshot_dir(&trial), before, "check must be read-only");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fmt_save_is_a_silent_no_op_on_clean_input() {
    let root = temp_root("save-clean");
    let trial = root.join("campaign");
    copy_dir(&valid_root(), &trial);
    let before = snapshot_dir(&trial);
    let output = run(&["fmt", trial.to_str().expect("unicode")]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(
        snapshot_dir(&trial),
        before,
        "clean save must change nothing"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fmt_check_lists_every_differing_file_in_lexical_order() {
    let root = temp_root("check-dirty");
    let trial = root.join("campaign");
    copy_dir(&valid_root(), &trial);
    // Noncanonical semantically-identical bytes: extra blank lines and
    // trailing spaces are not canonical, so both files differ after the
    // data writer round-trips them.
    for relative in ["campaign.json", "worlds/world.json"] {
        let path = trial.join(relative);
        let mut text = std::fs::read_to_string(&path).expect("must read");
        text.push_str("\n   \n");
        std::fs::write(&path, text).expect("must dirty");
    }
    let trial_text = trial.to_str().expect("unicode").to_owned();
    let output = run(&["fmt", "--check", &trial_text]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"crpgc fmt: noncanonical: campaign.json\ncrpgc fmt: noncanonical: worlds/world.json\n"
    );
    // The check opened nothing for writing: the dirt survives byte-identical.
    let after_check = run(&["fmt", &trial_text, "--check"]);
    assert_eq!(after_check.stderr, output.stderr);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fmt_save_canonicalizes_and_then_no_ops() {
    let root = temp_root("save-dirty");
    let trial = root.join("campaign");
    copy_dir(&valid_root(), &trial);
    let path = trial.join("creatures").join("creature.json");
    let mut text = std::fs::read_to_string(&path).expect("must read");
    text.push('\n');
    std::fs::write(&path, text).expect("must dirty");
    let trial_text = trial.to_str().expect("unicode").to_owned();
    let output = run(&["fmt", &trial_text]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    // The save normalized the file back to the checked-in canonical bytes.
    let saved = std::fs::read(&path).expect("must read");
    let checked_in = std::fs::read(valid_root().join("creatures").join("creature.json"))
        .expect("checked-in must read");
    assert_eq!(saved, checked_in);
    // A second save is a zero-write no-op: silent with identical bytes.
    let before = snapshot_dir(&trial);
    let again = run(&["fmt", &trial_text]);
    assert_eq!(again.status.code(), Some(0));
    assert!(again.stdout.is_empty() && again.stderr.is_empty());
    assert_eq!(snapshot_dir(&trial), before);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fmt_save_matches_the_checked_in_migration_golden() {
    let trial = copy_v1_fixture("golden");
    let trial_text = trial.to_str().expect("unicode").to_owned();
    let output = run(&["fmt", &trial_text]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    let expected = migration_expected();
    let after = snapshot_dir(&trial);
    // The complete expected output map/key set matches: no missing, no
    // extra, no renamed files. Ignored files stay byte-identical because
    // the writer only replaces recognized documents.
    let mut after_strings: std::collections::BTreeMap<String, Vec<u8>> =
        std::collections::BTreeMap::new();
    for (relative, bytes) in &after {
        let key = relative.to_str().expect("unicode").replace('\\', "/");
        after_strings.insert(key, bytes.clone());
    }
    let mut expected_keys: Vec<&String> = expected.keys().collect();
    expected_keys.sort();
    let mut after_keys: Vec<&String> = after_strings.keys().collect();
    after_keys.sort();
    // Every golden entry matches; the key sets are equal (no extra files
    // beyond the golden, which would hide an ignored-byte rewrite).
    for (logical, text) in &expected {
        let relative = PathBuf::from(logical);
        let actual = after
            .get(&relative)
            .unwrap_or_else(|| panic!("golden file {logical} must exist"));
        assert_eq!(actual, &text.as_bytes().to_vec(), "golden file {logical}");
    }
    assert_eq!(
        after_keys.len(),
        expected_keys.len(),
        "fmt must not create extra files beyond the golden"
    );
    // Ignored sentinel preservation: add an ignored file before saving on a
    // second copy and require it byte-identical after.
    let trial2 = copy_v1_fixture("golden-ignored");
    let sentinel = trial2.join("notes.txt");
    std::fs::write(&sentinel, b"author notes\n").expect("sentinel must write");
    let trial2_text = trial2.to_str().expect("unicode").to_owned();
    let output2 = run(&["fmt", &trial2_text]);
    assert_eq!(output2.status.code(), Some(0));
    assert_eq!(
        std::fs::read(&sentinel).expect("sentinel must reread"),
        b"author notes\n"
    );
    let _ = std::fs::remove_dir_all(trial2.parent().expect("parent"));
    // A second save is a silent no-op on the migrated tree.
    let before_second = snapshot_dir(&trial);
    let again = run(&["fmt", &trial_text]);
    assert_eq!(again.status.code(), Some(0));
    assert!(again.stdout.is_empty() && again.stderr.is_empty());
    assert_eq!(snapshot_dir(&trial), before_second);
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
}

#[test]
fn fmt_refuses_a_document_hard_linked_to_ignored_content() {
    let root = temp_root("hard-link-alias");
    let trial = root.join("campaign");
    copy_dir(&valid_root(), &trial);
    let document = trial.join("campaign.json");
    let ignored_alias = trial.join("author-backup.txt");
    std::fs::hard_link(&document, &ignored_alias).expect("hard link must create");
    let mut dirty = std::fs::read(&document).expect("document must read");
    dirty.extend_from_slice(b"\n");
    std::fs::write(&document, &dirty).expect("document must dirty");
    let output = run(&["fmt", trial.to_str().expect("unicode")]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"campaign.json: error[io]: cannot check campaign.json: source_changed\n"
    );
    assert_eq!(
        std::fs::read(&document).expect("document must reread"),
        dirty
    );
    assert_eq!(
        std::fs::read(&ignored_alias).expect("alias must reread"),
        dirty
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(target_os = "linux")]
#[test]
fn fmt_rejects_a_symlinked_root_ancestor_without_traversal() {
    use std::os::unix::fs::symlink;
    let root = temp_root("symlink-root-ancestor");
    let real_parent = root.join("real");
    let campaign = real_parent.join("campaign");
    copy_dir(&valid_root(), &campaign);
    let document = campaign.join("campaign.json");
    let mut dirty = std::fs::read(&document).expect("document must read");
    dirty.extend_from_slice(b"\n");
    std::fs::write(&document, &dirty).expect("document must dirty");
    let alias = root.join("alias");
    symlink(&real_parent, &alias).expect("Linux symlink must create");
    let via_alias = alias.join("campaign");
    let output = run(&["fmt", via_alias.to_str().expect("unicode")]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"<campaign>: error[io]: cannot open root <campaign-root>: symlink_at_document_path\n"
    );
    assert_eq!(
        std::fs::read(&document).expect("document must reread"),
        dirty
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fmt_check_on_historical_input_is_read_only_and_dirty() {
    let trial = copy_v1_fixture("historical-check");
    let before = snapshot_dir(&trial);
    let output = run(&["fmt", trial.to_str().expect("unicode"), "--check"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
    for line in String::from_utf8(output.stderr).expect("utf-8").lines() {
        assert!(line.starts_with("crpgc fmt: noncanonical: "), "{line}");
    }
    assert_eq!(
        snapshot_dir(&trial),
        before,
        "historical check must be read-only"
    );
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
}

#[test]
fn fmt_preflight_failure_writes_nothing() {
    // A genuine early-migratable file followed by a late failing file proves
    // no streaming writes: the historical v1 tree has an old item
    // (`items/item.json`, `crpg.item/1`) that would migrate, while the late
    // lexical file `worlds/world.json` is corrupted. The whole map fails
    // before any write starts, so the early file is untouched.
    let trial = copy_v1_fixture("preflight-genuine");
    std::fs::write(trial.join("worlds").join("world.json"), b"{ not json")
        .expect("must break late file");
    let before = snapshot_dir(&trial);
    let trial_text = trial.to_str().expect("unicode").to_owned();
    for argv in [
        vec!["fmt", &trial_text],
        vec!["fmt", "--check", &trial_text],
    ] {
        let output = run(&argv);
        assert_eq!(output.status.code(), Some(1), "argv {argv:?}");
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
    assert_eq!(
        snapshot_dir(&trial),
        before,
        "genuine early-migratable/late-failing preflight must write nothing"
    );
    let _ = std::fs::remove_dir_all(trial.parent().expect("parent"));
}

#[test]
fn fmt_broken_fixture_saves_structurally_and_keeps_its_snapshot() {
    let root = temp_root("broken");
    let trial = root.join("campaign");
    copy_dir(&broken_root(), &trial);
    let trial_text = trial.to_str().expect("unicode").to_owned();
    // Structurally valid input with semantic findings still formats.
    let output = run(&["fmt", &trial_text]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Its subsequent validate snapshot is retained exactly.
    let validate = run(&["validate", &trial_text, "--json"]);
    assert_eq!(validate.status.code(), Some(1));
    let snapshot = std::fs::read(broken_snapshot_path()).expect("snapshot must read");
    assert_eq!(validate.stdout, snapshot, "fmt must not repair semantics");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fmt_validate_before_save_is_read_only() {
    let root = temp_root("validate-read-only");
    let trial = root.join("campaign");
    copy_dir(&valid_root(), &trial);
    let path = trial.join("campaign.json");
    let mut text = std::fs::read_to_string(&path).expect("must read");
    text.push('\n');
    std::fs::write(&path, text).expect("must dirty");
    let before = snapshot_dir(&trial);
    let trial_text = trial.to_str().expect("unicode").to_owned();
    let validate = run(&["validate", &trial_text, "--json"]);
    assert_eq!(validate.status.code(), Some(0));
    assert_eq!(
        snapshot_dir(&trial),
        before,
        "validate before fmt is read-only"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fmt_missing_root_is_domain_failure_not_usage() {
    let missing = temp_root("missing-root");
    let _ = std::fs::remove_dir_all(&missing);
    let output = run(&["fmt", missing.to_str().expect("unicode")]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}

#[test]
fn fmt_runs_with_an_implicit_dot_root() {
    let root = temp_root("dot-root");
    let trial = root.join("campaign");
    copy_dir(&valid_root(), &trial);
    let output = Command::new(crpgc())
        .args(["fmt", "--check"])
        .current_dir(&trial)
        .output()
        .expect("spawning crpgc must succeed");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}
