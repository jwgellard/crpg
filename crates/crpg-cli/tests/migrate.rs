//! End-to-end tests for `crpgc migrate` (T012b).
//!
//! These drive the built binary — `env!("CARGO_BIN_EXE_crpgc")` — as a black
//! box through `std::process::Command`, asserting exit codes and exact stream
//! bytes, never migration internals. T012a's data-owned migration framework,
//! schemas, fixtures, and goldens are consumed as test data and never edited.
//! The checked-in source tree is never migrated; tests copy fixtures to
//! private temporary directories and compare full resulting file-to-byte maps
//! with the checked-in golden. Real-symlink and non-Unicode process cases are
//! compile-time cfg-gated to platforms that can produce them; the rewrite
//! seam in `main.rs` covers the same shapes everywhere without runtime skips.

use std::collections::{BTreeMap, BTreeSet};
#[cfg(unix)]
use std::ffi::OsString;
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

/// Runs `crpgc` with OS-string `args` for non-Unicode process cases.
#[cfg(unix)]
fn run_os(args: &[OsString]) -> Output {
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

/// A fresh unique temp directory for one test. Tests run in parallel, so the
/// name carries the test's own slug; stale leftovers are removed first so a
/// crashed run cannot fake a result.
fn temp_root(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("crpg-migrate-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp root must create");
    path
}

/// Recursively copies `src` to `dst` with `std` only. Fixture copies let
/// migration tests mutate freely without touching the checked-in tree.
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

/// Copies the v1 campaign to a temp root for migration.
fn copy_v1_fixture(name: &str) -> PathBuf {
    let dst = temp_root(name);
    copy_dir(&migration_v1_root(), &dst);
    dst
}

/// Copies the canonical-current fixture to a temp root for mutation.
fn copy_valid_fixture(name: &str) -> PathBuf {
    let dst = temp_root(name);
    copy_dir(&valid_root(), &dst);
    dst
}

/// Copies the broken fixture to a temp root for migration.
fn copy_broken_fixture(name: &str) -> PathBuf {
    let dst = temp_root(name);
    copy_dir(&broken_root(), &dst);
    dst
}

/// Best-effort cleanup of a temp root after a test.
fn cleanup(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

/// Recursively snapshots every UTF-8-named file under `root` as a `/`-joined
/// relative path to byte map. Used to prove preflight performs zero writes
/// and second migrations are byte-identical. Non-Unicode entries (present
/// only in the dedicated non-Unicode test) are skipped rather than
/// lossy-converted, so the snapshot never panics and still pins that all
/// UTF-8 files are unchanged.
fn snapshot_all(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, map: &mut BTreeMap<String, Vec<u8>>) {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap_or_else(|_| panic!("must list {}", dir.display()))
            .map(|entry| entry.expect("snapshot entry").path())
            .collect();
        entries.sort();
        for entry in &entries {
            if entry.is_dir() {
                walk(entry, root, map);
            } else {
                let relative = entry.strip_prefix(root).expect("under root");
                let mut parts = Vec::new();
                let mut utf8 = true;
                for component in relative.components() {
                    match component.as_os_str().to_str() {
                        Some(text) => parts.push(text.to_owned()),
                        None => {
                            utf8 = false;
                            break;
                        }
                    }
                }
                if !utf8 {
                    continue;
                }
                let text = parts.join("/");
                let bytes = std::fs::read(entry).expect("snapshot file must read");
                map.insert(text, bytes);
            }
        }
    }

    let mut map = BTreeMap::new();
    walk(root, root, &mut map);
    map
}

/// Reads the checked-in T012a golden: a canonical JSON object mapping each
/// logical source path to its complete canonical output text string,
/// including final LF. Decodes each string as UTF-8 bytes for an exact
/// file-map comparison. Fails hard on a missing or malformed golden; tests
/// never bless outputs.
fn read_golden_map() -> BTreeMap<String, Vec<u8>> {
    let bytes = std::fs::read(migration_expected_path()).expect("migration golden must exist");
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).expect("golden must parse as JSON");
    let obj = value.as_object().expect("golden must be an object");
    assert!(!obj.is_empty(), "golden must not be empty");
    let mut map = BTreeMap::new();
    for (logical, text) in obj {
        let string = text
            .as_str()
            .unwrap_or_else(|| panic!("golden entry {logical} must be a string"));
        map.insert(logical.clone(), string.as_bytes().to_vec());
    }
    map
}

/// Asserts a usage failure: exit 2, empty stdout, exactly one `crpgc: ...`
/// line on stderr.
fn assert_usage(out: &Output) {
    assert_eq!(out.status.code(), Some(2), "usage must exit 2: {out:?}");
    assert!(out.stdout.is_empty(), "usage prints no stdout: {out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 1, "usage is one line: {stderr}");
    assert!(lines[0].starts_with("crpgc: "), "usage prefix: {stderr}");
}

/// Asserts a migrate domain failure: exit 1, empty stdout, exactly one
/// diagnostic line on stderr with portable `/` paths and no absolute paths.
fn assert_migrate_failure(out: &Output) -> String {
    assert_eq!(out.status.code(), Some(1), "domain must exit 1: {out:?}");
    assert!(out.stdout.is_empty(), "migrate stdout stays empty: {out:?}");
    let stderr = String::from_utf8(out.stderr.clone()).expect("stderr must be UTF-8");
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 1, "one diagnostic line: {stderr}");
    assert!(
        !stderr.contains('\\'),
        "portable `/` paths, no native separators: {stderr}"
    );
    assert!(
        !stderr.contains("rollback"),
        "must not claim rollback: {stderr}"
    );
    stderr
}

/// Asserts migrate success is silent: exit 0, both streams empty.
fn assert_migrate_success(out: &Output) {
    assert_eq!(out.status.code(), Some(0), "success must exit 0: {out:?}");
    assert!(out.stdout.is_empty(), "migrate stdout stays empty: {out:?}");
    assert!(out.stderr.is_empty(), "success stderr stays empty: {out:?}");
}

#[test]
fn migrate_missing_root_is_usage_exit_2() {
    assert_usage(&run(&["migrate"]));
}

#[test]
fn migrate_extra_positional_is_usage_exit_2() {
    let root = migration_v1_root();
    let root = root.to_str().expect("utf-8 fixture path");
    assert_usage(&run(&["migrate", root, "extra"]));
}

#[test]
fn migrate_any_flag_is_usage_exit_2() {
    let root = migration_v1_root();
    let root = root.to_str().expect("utf-8 fixture path");
    for args in [
        vec!["migrate", "--json", root],
        vec!["migrate", root, "--json"],
        vec!["migrate", "--check", root],
        vec!["migrate", "--dry-run", root],
        vec!["migrate", "--to", root],
        vec!["migrate", "--golden", root],
        vec!["migrate", root, "--golden", "g"],
        vec!["migrate", "--bogus", root],
        vec!["migrate", root, "--bogus"],
    ] {
        assert_usage(&run(&args));
    }
}

#[test]
fn migrate_missing_path_is_domain_exit_1_not_usage() {
    let mut missing = std::env::temp_dir();
    missing.push(format!("crpg-migrate-{}-no-such-root", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);
    let arg = missing.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    let stderr = assert_migrate_failure(&out);
    assert!(stderr.contains("error[io]"), "missing root is io: {stderr}");
    assert!(
        stderr.contains("cannot open root <campaign-root>: not_found"),
        "stable kind: {stderr}"
    );
}

#[test]
fn migrate_file_as_root_is_exit_1_not_a_directory() {
    let dir = temp_root("file-as-root");
    let file = dir.join("root.json");
    std::fs::write(&file, b"{}").expect("temp file must write");
    let arg = file.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    let stderr = assert_migrate_failure(&out);
    assert_eq!(
        out.stderr,
        b"<campaign>: error[io]: cannot open root <campaign-root>: not_a_directory\n".to_vec()
    );
    assert!(stderr.contains("not_a_directory"));
    cleanup(&dir);
}

#[test]
fn copied_v1_campaign_migrates_to_complete_data_golden() {
    let root = copy_v1_fixture("v1-to-golden");
    // Ignored sentinel files in every ignored shape: garbage content that
    // must remain byte-identical because the writer only replaces recognized
    // differing documents at their existing logical paths.
    let ignored: Vec<(&str, &[u8])> = vec![
        ("notes.txt", b"this is not json{{{"),
        ("run.lua", b"{{{garbage"),
        ("data.replay", b"\x00\x01"),
        (".gitattributes", b"* text=auto"),
        ("schemas/v1.json", b"{{{garbage"),
        ("build/output.bin", b"\x00\x01"),
        ("areas/start/notes.txt", b"{{{garbage"),
    ];
    let mut ignored_originals = BTreeMap::new();
    for (relative, bytes) in &ignored {
        let mut path = root.clone();
        for part in relative.split('/') {
            path.push(part);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("ignored dir must create");
        }
        std::fs::write(&path, bytes).expect("ignored file must write");
        ignored_originals.insert((*relative).to_owned(), (*bytes).to_vec());
    }
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    assert_migrate_success(&out);

    // The full resulting recognized file-to-byte map must equal the
    // checked-in T012a golden; the golden is reviewed output, never computed
    // from the actual result inside this test.
    let golden = read_golden_map();
    assert_eq!(golden.len(), 11, "golden pins eleven documents");
    for (logical, expected_bytes) in &golden {
        let mut path = root.clone();
        for part in logical.split('/') {
            path.push(part);
        }
        let actual =
            std::fs::read(&path).unwrap_or_else(|_| panic!("migrated file must exist: {logical}"));
        assert_eq!(
            &actual, expected_bytes,
            "migrated bytes must equal golden for {logical}"
        );
    }
    // Ignored content remains byte-identical.
    for (relative, expected) in &ignored_originals {
        let mut path = root.clone();
        for part in relative.split('/') {
            path.push(part);
        }
        let actual = std::fs::read(&path).expect("ignored file must still exist");
        assert_eq!(
            &actual, expected,
            "ignored file must remain byte-identical: {relative}"
        );
    }
    // The checked-in source fixture itself is unchanged.
    let source_item = std::fs::read_to_string(migration_v1_root().join("items").join("item.json"))
        .expect("source fixture must read");
    assert!(
        source_item.contains("\"schema\": \"crpg.item/1\""),
        "source fixture stays at v1: {source_item}"
    );
    cleanup(&root);
}

#[test]
fn second_migration_is_byte_identical_silent_success() {
    let root = copy_v1_fixture("second-migration");
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let first = run(&["migrate", &arg]);
    assert_migrate_success(&first);
    let after_first = snapshot_all(&root);
    let golden = read_golden_map();
    for (logical, expected) in &golden {
        assert_eq!(
            after_first.get(logical),
            Some(expected),
            "first migration must match golden for {logical}"
        );
    }
    let second = run(&["migrate", &arg]);
    assert_migrate_success(&second);
    let after_second = snapshot_all(&root);
    assert_eq!(
        after_first, after_second,
        "second migration must be byte-identical"
    );
    // Zero writes are pinned through the private writer seam in `main.rs`
    // (`rewrite_empty_updates_perform_zero_writes`), not via timestamps here.
    cleanup(&root);
}

#[test]
fn canonical_current_fixture_is_zero_write_success() {
    let root = copy_valid_fixture("canonical-current");
    let before = snapshot_all(&root);
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    assert_migrate_success(&out);
    let after = snapshot_all(&root);
    assert_eq!(before, after, "canonical-current must not change");
    cleanup(&root);
}

#[test]
fn noncanonical_current_canonicalizes_only_on_explicit_migrate() {
    let root = copy_valid_fixture("noncanonical-current");
    let target = root.join("creatures").join("creature.json");
    let canonical = std::fs::read(&target).expect("fixture copy must read");
    // Compact JSON differs in whitespace from the canonical pretty output but
    // carries identical semantics; it must load cleanly.
    let text = String::from_utf8(canonical.clone()).expect("canonical must be UTF-8");
    let value: serde_json::Value = serde_json::from_str(&text).expect("canonical must parse");
    let mut compact = serde_json::to_string(&value).expect("compact must serialize");
    compact.push('\n');
    std::fs::write(&target, compact.as_bytes()).expect("noncanonical must write");
    assert_ne!(
        std::fs::read(&target).expect("must read"),
        canonical,
        "test must actually de-canonicalize"
    );
    let arg = root.to_str().expect("utf-8 temp path").to_owned();

    // Validate before migrate is read-only.
    let validate_before = run(&["validate", &arg]);
    assert_eq!(
        validate_before.status.code(),
        Some(0),
        "noncanonical-current still validates clean: {validate_before:?}"
    );
    assert_eq!(
        std::fs::read(&target).expect("must read"),
        compact.as_bytes(),
        "validate must not rewrite"
    );

    let migrate = run(&["migrate", &arg]);
    assert_migrate_success(&migrate);
    assert_eq!(
        std::fs::read(&target).expect("must read"),
        canonical,
        "explicit migrate canonicalizes to the checked-in bytes"
    );
    cleanup(&root);
}

#[test]
fn unsupported_future_tag_leaves_entire_map_unchanged() {
    let root = copy_valid_fixture("unsupported-future-tag");
    let path = root.join("creatures").join("creature.json");
    let text = std::fs::read_to_string(&path).expect("fixture copy must read");
    assert!(text.contains("\"schema\": \"crpg.creature/1\""));
    std::fs::write(
        &path,
        text.replace(
            "\"schema\": \"crpg.creature/1\"",
            "\"schema\": \"crpg.creature/99\"",
        ),
    )
    .expect("rewrite schema tag");
    let before = snapshot_all(&root);
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    let stderr = assert_migrate_failure(&out);
    assert!(
        stderr.contains("error[unsupported_schema]"),
        "data-owned code: {stderr}"
    );
    assert!(
        stderr.contains("creatures/creature.json"),
        "data-owned position: {stderr}"
    );
    assert_eq!(snapshot_all(&root), before, "preflight must not write");
    cleanup(&root);
}

#[test]
fn malformed_document_leaves_entire_map_unchanged() {
    let root = copy_valid_fixture("malformed-document");
    std::fs::write(
        root.join("creatures").join("creature.json"),
        b"this is not json{{{",
    )
    .expect("corrupt fixture copy");
    let before = snapshot_all(&root);
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    let stderr = assert_migrate_failure(&out);
    assert!(stderr.contains("error[malformed]"), "{stderr}");
    assert!(stderr.contains("creatures/creature.json"), "{stderr}");
    assert_eq!(snapshot_all(&root), before, "preflight must not write");
    cleanup(&root);
}

#[test]
fn missing_required_file_leaves_entire_map_unchanged() {
    let root = copy_valid_fixture("missing-required-file");
    std::fs::remove_file(root.join("campaign.lock")).expect("remove lock from copy");
    let before = snapshot_all(&root);
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    let stderr = assert_migrate_failure(&out);
    assert!(stderr.contains("error[layout]"), "{stderr}");
    assert!(
        stderr.starts_with("<campaign>:"),
        "missing file has no logical file: {stderr}"
    );
    assert_eq!(snapshot_all(&root), before, "preflight must not write");
    cleanup(&root);
}

#[test]
fn layout_kind_mismatch_leaves_entire_map_unchanged() {
    let root = copy_valid_fixture("layout-kind-mismatch");
    let world_bytes =
        std::fs::read(root.join("worlds").join("world.json")).expect("world must read");
    std::fs::write(root.join("creatures").join("extra.json"), world_bytes)
        .expect("kind-mismatched file must write");
    let before = snapshot_all(&root);
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    let stderr = assert_migrate_failure(&out);
    assert!(stderr.contains("error[layout]"), "{stderr}");
    assert!(
        stderr.contains("creatures/extra.json"),
        "layout names the offending path: {stderr}"
    );
    assert_eq!(snapshot_all(&root), before, "preflight must not write");
    cleanup(&root);
}

#[test]
fn impossible_engine_leaves_entire_map_unchanged() {
    let root = copy_valid_fixture("impossible-engine");
    let path = root.join("campaign.json");
    let text = std::fs::read_to_string(&path).expect("fixture copy must read");
    assert!(text.contains(">=0.1.0, <1.0.0"));
    std::fs::write(&path, text.replace(">=0.1.0, <1.0.0", ">=99.0.0"))
        .expect("rewrite engine range");
    let before = snapshot_all(&root);
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    let stderr = assert_migrate_failure(&out);
    assert!(stderr.contains("error[engine_incompatible]"), "{stderr}");
    assert!(
        stderr.starts_with("<campaign>:"),
        "engine failure has no logical file: {stderr}"
    );
    assert_eq!(snapshot_all(&root), before, "preflight must not write");
    cleanup(&root);
}

#[test]
fn duplicate_id_leaves_entire_map_unchanged() {
    let root = copy_valid_fixture("duplicate-id");
    let world_text =
        std::fs::read_to_string(root.join("worlds").join("world.json")).expect("world must read");
    // World id is the second ULID; copy it over the creature id to force a
    // duplicate-id index failure during preflight.
    let world_value: serde_json::Value =
        serde_json::from_str(&world_text).expect("world must parse");
    let world_id = world_value
        .get("id")
        .and_then(|id| id.as_str())
        .expect("world has id")
        .to_owned();
    let creature_path = root.join("creatures").join("creature.json");
    let creature_text = std::fs::read_to_string(&creature_path).expect("creature must read");
    let mut creature_value: serde_json::Value =
        serde_json::from_str(&creature_text).expect("creature must parse");
    creature_value["id"] = serde_json::Value::String(world_id);
    let rewritten = serde_json::to_string_pretty(&creature_value).expect("must serialize");
    std::fs::write(&creature_path, format!("{rewritten}\n")).expect("duplicate id must write");
    let before = snapshot_all(&root);
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    let stderr = assert_migrate_failure(&out);
    assert!(stderr.contains("error[duplicate_id]"), "{stderr}");
    assert!(
        stderr.contains("worlds/world.json"),
        "duplicate reports the second path: {stderr}"
    );
    assert_eq!(snapshot_all(&root), before, "preflight must not write");
    cleanup(&root);
}

#[test]
fn invalid_lock_leaves_entire_map_unchanged() {
    let root = copy_valid_fixture("invalid-lock");
    let path = root.join("campaign.json");
    let text = std::fs::read_to_string(&path).expect("fixture copy must read");
    assert!(text.contains("\"requires\": []"));
    std::fs::write(
        &path,
        text.replace(
            "\"requires\": []",
            "\"requires\": [{\"kind\": \"module\", \"package\": \"a\", \"version\": \"^1\"}]",
        ),
    )
    .expect("add requirement without lock entry");
    let before = snapshot_all(&root);
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    let stderr = assert_migrate_failure(&out);
    assert!(
        stderr.contains("error[invalid_lock]"),
        "data-owned lock code: {stderr}"
    );
    assert!(
        stderr.starts_with("<campaign>:"),
        "lock failure has no logical file: {stderr}"
    );
    assert_eq!(snapshot_all(&root), before, "preflight must not write");
    cleanup(&root);
}

#[test]
fn digest_mismatch_leaves_entire_map_unchanged() {
    let root = copy_valid_fixture("digest-mismatch");
    let path = root.join("campaign.lock");
    let text = std::fs::read_to_string(&path).expect("lock copy must read");
    // Replace the stored digest with 64 zeros: valid hex shape, wrong value,
    // so the assets-lock digest check fails during preflight.
    let start = text.find("\"assets_lock\": \"").expect("lock pins digest");
    let prefix_len = "\"assets_lock\": \"".len();
    let digest_start = start + prefix_len;
    let mut rewritten = text.clone();
    rewritten.replace_range(digest_start..digest_start + 64, &"0".repeat(64));
    std::fs::write(&path, rewritten).expect("wrong digest must write");
    let before = snapshot_all(&root);
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    let stderr = assert_migrate_failure(&out);
    assert!(
        stderr.contains("error[assets_lock_mismatch]"),
        "digest failure: {stderr}"
    );
    assert!(
        stderr.starts_with("<campaign>:"),
        "digest failure has no logical file: {stderr}"
    );
    assert_eq!(snapshot_all(&root), before, "preflight must not write");
    cleanup(&root);
}

#[test]
fn early_migratable_plus_late_failing_proves_no_streaming_writes() {
    let root = copy_v1_fixture("early-migratable-late-failing");
    // `items/item.json` (v1, migratable) sorts before `worlds/world.json`
    // (corrupted late). A streaming writer would already have replaced the
    // early file before hitting the late read failure; correct preflight
    // leaves the entire map unchanged.
    std::fs::write(
        root.join("worlds").join("world.json"),
        b"this is not json{{{",
    )
    .expect("corrupt late file");
    let before = snapshot_all(&root);
    let early_before = before
        .get("items/item.json")
        .expect("early file must be snapshotted")
        .clone();
    assert!(
        String::from_utf8_lossy(&early_before).contains("crpg.item/1"),
        "early file starts unmigrated"
    );
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    let stderr = assert_migrate_failure(&out);
    assert!(stderr.contains("error[malformed]"), "{stderr}");
    assert!(stderr.contains("worlds/world.json"), "{stderr}");
    let after = snapshot_all(&root);
    assert_eq!(after, before, "no streaming writes");
    assert!(
        String::from_utf8_lossy(after.get("items/item.json").expect("early still there"))
            .contains("crpg.item/1"),
        "early migratable file must not have been rewritten"
    );
    cleanup(&root);
}

#[test]
fn broken_fixture_migrates_structurally_and_keeps_validate_snapshot() {
    let root = copy_broken_fixture("broken-migration");
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let migrate = run(&["migrate", &arg]);
    assert_migrate_success(&migrate);
    // Migration performs no semantic validation and no repair: the exact
    // subsequent `validate --json` output still byte-equals the checked-in
    // T011a snapshot.
    let validate = run(&["validate", &arg, "--json"]);
    assert_eq!(
        validate.status.code(),
        Some(1),
        "broken still fails validate: {validate:?}"
    );
    let snapshot = std::fs::read(broken_snapshot_path()).expect("broken snapshot must exist");
    assert_eq!(
        validate.stdout, snapshot,
        "post-migration validate must byte-equal its snapshot"
    );
    cleanup(&root);
}

#[test]
fn accepted_family_bad_name_is_invalid_path_not_io() {
    let root = copy_valid_fixture("migrate-invalid-path");
    std::fs::write(root.join("creatures").join("bad name!.json"), b"ignored")
        .expect("bad-name file must create");
    let before = snapshot_all(&root);
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    let stderr = assert_migrate_failure(&out);
    assert!(
        stderr.contains("error[invalid_path]"),
        "classifier code preserved: {stderr}"
    );
    assert!(!stderr.contains("[io]"), "never io: {stderr}");
    assert_eq!(
        snapshot_all(&root),
        before,
        "collection failure must not write"
    );
    cleanup(&root);
}

#[test]
fn ignored_files_are_never_read_or_written() {
    let root = copy_valid_fixture("migrate-ignored-files");
    std::fs::write(root.join("notes.txt"), b"this is not json{{{").expect("junk must write");
    std::fs::write(root.join("run.lua"), b"{{{garbage").expect("junk must write");
    std::fs::write(root.join("data.replay"), b"\x00\x01").expect("junk must write");
    std::fs::write(root.join(".gitattributes"), b"* text=auto").expect("junk must write");
    std::fs::create_dir_all(root.join("schemas")).expect("junk dir must create");
    std::fs::write(root.join("schemas").join("v1.json"), b"{{{garbage").expect("junk must write");
    std::fs::create_dir_all(root.join("build")).expect("junk dir must create");
    std::fs::write(root.join("build").join("output.bin"), b"\x00\x01").expect("junk must write");
    std::fs::write(
        root.join("areas").join("start").join("notes.txt"),
        b"{{{garbage",
    )
    .expect("nested junk must write");
    let before = snapshot_all(&root);
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    assert_migrate_success(&out);
    assert_eq!(
        snapshot_all(&root),
        before,
        "ignored junk stays byte-identical on a no-op migrate"
    );
    cleanup(&root);
}

/// Real black-box symlink case, compile-time gated to platforms where
/// symlink creation is guaranteed. Windows without privileges cannot create
/// one, so the same shape is pinned there through the rewrite seam in
/// `main.rs` instead — never a runtime skip.
#[cfg(unix)]
#[test]
fn symlink_at_document_path_is_exit_1() {
    use std::os::unix::fs::symlink;
    let dir = temp_root("symlink-document");
    copy_dir(&valid_root(), &dir);
    std::fs::remove_file(dir.join("campaign.json")).expect("remove for symlink");
    symlink(
        valid_root().join("campaign.json"),
        dir.join("campaign.json"),
    )
    .expect("symlink must create");
    let before = snapshot_all(&dir);
    let arg = dir.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    let stderr = assert_migrate_failure(&out);
    assert!(stderr.contains("error[io]"), "{stderr}");
    assert!(
        stderr.contains("campaign.json") && stderr.contains("symlink_at_document_path"),
        "{stderr}"
    );
    // The collector fails before any rewrite, so the live tree (apart from
    // the intentionally placed symlink) is unchanged. The snapshot includes
    // the symlink target bytes read through the link, which the collector
    // never reads, so compare only that no regular file changed: re-list
    // regular files excluding the symlinked path.
    let after = snapshot_all(&dir);
    // `snapshot_all` follows the symlink when reading, so both snapshots read
    // the same target bytes; the assertion is that migrate performed no
    // writes to any other file.
    for (logical, bytes) in &before {
        if logical == "campaign.json" {
            continue;
        }
        assert_eq!(
            after.get(logical),
            Some(bytes),
            "no writes beyond the symlinked path: {logical}"
        );
    }
    cleanup(&dir);
}

/// Non-Unicode roots reach the binary through `args_os` and fail as `io`,
/// never as usage and never as a panic. Unix-only: only Unix can spell such
/// a path for a child process portably.
#[cfg(unix)]
#[test]
fn non_unicode_root_is_exit_1_io() {
    use std::os::unix::ffi::OsStringExt;
    let bad = OsString::from_vec(b"/tmp/crpg-migrate-\xff-root".to_vec());
    let out = run_os(&[OsString::from("migrate"), bad]);
    let stderr = assert_migrate_failure(&out);
    assert_eq!(
        out.stderr,
        b"<campaign>: error[io]: cannot open root <campaign-root>: non_unicode_component\n"
            .to_vec()
    );
    assert!(stderr.contains("non_unicode_component"));
}

/// A non-Unicode entry inside a walked tree fails its entry with a portable
/// message that never lossy-converts the component.
#[cfg(unix)]
#[test]
fn non_unicode_component_is_exit_1_io() {
    use std::os::unix::ffi::OsStringExt;
    let dir = temp_root("non-unicode-component");
    copy_dir(&valid_root(), &dir);
    let bad = OsString::from_vec(vec![0xff, 0x62, 0x61, 0x64]);
    std::fs::write(dir.join(&bad), b"ignored").expect("non-Unicode file must create");
    let before = snapshot_all(&dir);
    let arg = dir.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    let stderr = assert_migrate_failure(&out);
    assert_eq!(
        out.stderr,
        b"<campaign>: error[io]: cannot list <campaign-root>: non_unicode_component\n".to_vec()
    );
    assert!(stderr.contains("non_unicode_component"));
    // The walker fails before any rewrite; regular files are unchanged. The
    // non-Unicode entry is not part of the UTF-8 snapshot on either side.
    assert_eq!(snapshot_all(&dir), before, "no writes on list failure");
    cleanup(&dir);
}

/// Two locale documents colliding under ASCII case-folding both classify, so
/// the collision reaches data and reports `layout`. Linux-only: Windows
/// cannot create both names in one directory.
#[cfg(target_os = "linux")]
#[test]
fn ascii_case_collision_reaches_data_as_layout() {
    let root = copy_valid_fixture("case-collision");
    let text = std::fs::read_to_string(root.join("locale").join("en.json"))
        .expect("locale copy must read");
    assert!(text.contains("\"locale\": \"en\""), "locale pins its tag");
    std::fs::write(
        root.join("locale").join("EN.json"),
        text.replace("\"locale\": \"en\"", "\"locale\": \"EN\""),
    )
    .expect("colliding locale must write");
    let before = snapshot_all(&root);
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["migrate", &arg]);
    let stderr = assert_migrate_failure(&out);
    assert!(stderr.contains("error[layout]"), "{stderr}");
    assert_eq!(snapshot_all(&root), before, "preflight must not write");
    cleanup(&root);
}

/// Replay and validate keep their full byte/exit contracts after the migrate
/// addition: clean validate stays silent `[]\n`, broken validate still
/// byte-equals its snapshot, and replay usage still exits 2.
#[test]
fn replay_and_validate_contracts_are_preserved() {
    let root = valid_root();
    let arg = root.to_str().expect("utf-8 fixture path").to_owned();
    let plain = run(&["validate", &arg]);
    assert_eq!(plain.status.code(), Some(0), "{plain:?}");
    assert!(plain.stdout.is_empty());
    assert!(plain.stderr.is_empty());
    let json = run(&["validate", &arg, "--json"]);
    assert_eq!(json.status.code(), Some(0));
    assert_eq!(json.stdout, b"[]\n".to_vec());
    assert!(json.stderr.is_empty());

    let broken = broken_root();
    let broken_arg = broken.to_str().expect("utf-8 fixture path").to_owned();
    let broken_json = run(&["validate", &broken_arg, "--json"]);
    assert_eq!(broken_json.status.code(), Some(1));
    let snapshot = std::fs::read(broken_snapshot_path()).expect("snapshot must exist");
    assert_eq!(broken_json.stdout, snapshot);

    let usage = run(&["replay", "--bogus"]);
    assert_eq!(usage.status.code(), Some(2));
    assert!(usage.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&usage.stderr);
    assert_eq!(stderr.lines().count(), 1);
    assert!(stderr.starts_with("crpgc: "));
}

/// Manifest sanity: the gate-8 manifest still lists exactly the discovered
/// campaign roots, so T012b consumed T012a's fixture extension without
/// editing it.
#[test]
fn manifest_still_lists_exactly_discovered_roots() {
    let fixtures = fixtures_dir();
    let manifest_bytes =
        std::fs::read(fixtures.join("expected.json")).expect("T011a manifest must exist");
    let manifest: serde_json::Value =
        serde_json::from_slice(&manifest_bytes).expect("manifest must parse");
    let entries = manifest.as_array().expect("manifest must be an array");
    assert!(!entries.is_empty());
    let mut manifest_roots = Vec::new();
    for entry in entries {
        let root = entry["root"].as_str().expect("root must be a string");
        manifest_roots.push(root.to_owned());
    }
    assert!(
        manifest_roots.windows(2).all(|pair| pair[0] < pair[1]),
        "manifest sorted: {manifest_roots:?}"
    );

    let mut discovered = Vec::new();
    let mut stack = vec![fixtures.clone()];
    while let Some(dir) = stack.pop() {
        let mut dir_entries: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|_| panic!("must list {}", dir.display()))
            .map(|entry| entry.expect("fixture entry").path())
            .collect();
        dir_entries.sort();
        for entry in &dir_entries {
            if entry.is_dir() {
                stack.push(entry.clone());
            }
        }
        if dir != fixtures && dir.join("campaign.json").is_file() {
            let relative = dir.strip_prefix(&fixtures).expect("under fixtures");
            let text = relative
                .components()
                .map(|component| component.as_os_str().to_str().expect("utf-8 fixture names"))
                .collect::<Vec<_>>()
                .join("/");
            discovered.push(text);
        }
    }
    discovered.sort();
    assert_eq!(
        discovered, manifest_roots,
        "manifest must list exactly discovered roots"
    );
    // T012a's three-root extension is consumed, not edited.
    assert_eq!(
        manifest_roots,
        vec![
            "broken_references".to_owned(),
            "migration_v1/campaign".to_owned(),
            "one_area_one_creature".to_owned()
        ]
    );
    let _ = BTreeSet::<String>::new();
}
