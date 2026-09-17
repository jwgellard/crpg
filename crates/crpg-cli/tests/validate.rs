//! End-to-end tests for `crpgc validate` (T011b).
//!
//! These drive the built binary — `env!("CARGO_BIN_EXE_crpgc")` — as a black
//! box through `std::process::Command`, asserting exit codes and exact stream
//! bytes, never validation internals. The T011a fixture contract (valid
//! campaign, fifteen-finding broken campaign, data-owned `expected.json`
//! manifest and byte snapshot) is consumed as test data and never edited.
//! Real-symlink and non-Unicode process cases are compile-time cfg-gated to
//! platforms that can produce them; the collector's injected seam in
//! `main.rs` covers the same shapes everywhere without runtime skips.

use std::collections::BTreeSet;
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

/// The `crpg-data` crate directory: fixtures, manifest, and snapshot live here.
fn data_dir() -> PathBuf {
    cli_dir()
        .join("..")
        .join("crpg-data")
        .canonicalize()
        .expect("crpg-data crate dir must exist")
}

/// T011a's fixture campaigns, consumed read-only except for temp copies.
fn fixtures_dir() -> PathBuf {
    data_dir().join("tests").join("fixtures")
}

/// The clean T010 fixture campaign root.
fn valid_root() -> PathBuf {
    fixtures_dir().join("one_area_one_creature")
}

/// The fifteen-finding T011a fixture campaign root.
fn broken_root() -> PathBuf {
    fixtures_dir().join("broken_references")
}

/// T011a's checked-in canonical diagnostic snapshot for the broken fixture.
fn snapshot_path() -> PathBuf {
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
    path.push(format!("crpg-validate-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp root must create");
    path
}

/// Recursively copies `src` to `dst` with `std` only. Fixture copies let
/// structural tests mutate freely without touching the checked-in tree.
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

/// Copies the clean fixture to a temp root for mutation.
fn copy_fixture(name: &str) -> PathBuf {
    let dst = temp_root(name);
    copy_dir(&valid_root(), &dst);
    dst
}

/// Best-effort cleanup of a temp root after a test.
fn cleanup(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

/// Asserts `stdout` is a canonical diagnostic array and returns the parsed
/// diagnostics: no BOM, LF only, no trailing whitespace, exactly one final
/// LF, and portable `/` paths. Deserializing into data's own `Diagnostic`
/// pins the six-field shape, unknown-field rejection, and snake_case
/// severity/code spellings without a second shape in this crate.
fn assert_canonical_array(stdout: &[u8]) -> Vec<crpg_data::Diagnostic> {
    assert!(!stdout.is_empty(), "json output must not be empty");
    assert_eq!(stdout[0], b'[', "must start with `[` (no BOM, no prose)");
    assert!(!stdout.contains(&b'\r'), "LF endings only");
    assert_eq!(stdout.last(), Some(&b'\n'), "exactly one final LF");
    assert!(!stdout.ends_with(b"\n\n"), "exactly one final LF");
    for (index, line) in stdout.split(|byte| *byte == b'\n').enumerate() {
        assert!(
            !line.ends_with(b" ") && !line.ends_with(b"\t"),
            "no trailing whitespace on line {index}"
        );
    }
    let diagnostics: Vec<crpg_data::Diagnostic> =
        serde_json::from_slice(stdout).expect("stdout must parse as diagnostics");
    for diagnostic in &diagnostics {
        if let Some(file) = &diagnostic.file {
            assert!(
                !file.as_str().contains('\\'),
                "portable `/` paths: {}",
                file.as_str()
            );
        }
    }
    diagnostics
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

#[test]
fn validate_missing_root_is_usage_exit_2() {
    assert_usage(&run(&["validate"]));
    assert_usage(&run(&["validate", "--json"]));
}

#[test]
fn validate_duplicate_json_is_usage_exit_2() {
    let root = valid_root();
    let root = root.to_str().expect("utf-8 fixture path");
    assert_usage(&run(&["validate", "--json", "--json", root]));
    assert_usage(&run(&["validate", root, "--json", "--json"]));
}

#[test]
fn validate_extra_positional_is_usage_exit_2() {
    let root = valid_root();
    let root = root.to_str().expect("utf-8 fixture path");
    assert_usage(&run(&["validate", root, "extra"]));
}

#[test]
fn validate_unknown_flag_is_usage_exit_2() {
    let root = valid_root();
    let root = root.to_str().expect("utf-8 fixture path");
    assert_usage(&run(&["validate", "--bogus", root]));
    assert_usage(&run(&["validate", root, "--bogus"]));
}

#[test]
fn replay_first_position_flag_is_usage_exit_2() {
    assert_usage(&run(&["replay", "--bogus"]));
    assert_usage(&run(&["replay", "--golden", "g"]));
}

#[test]
fn validate_valid_plain_is_silent_exit_0() {
    let root = valid_root();
    let out = run(&["validate", root.to_str().expect("utf-8 fixture path")]);
    assert_eq!(out.status.code(), Some(0), "clean must exit 0: {out:?}");
    assert!(out.stdout.is_empty(), "plain stdout stays empty: {out:?}");
    assert!(
        out.stderr.is_empty(),
        "clean plain stderr stays empty: {out:?}"
    );
}

#[test]
fn validate_valid_json_flag_after_root_is_empty_array() {
    let root = valid_root();
    let out = run(&[
        "validate",
        root.to_str().expect("utf-8 fixture path"),
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(0), "clean must exit 0: {out:?}");
    assert_eq!(
        out.stdout,
        b"[]\n".to_vec(),
        "clean json is exactly `[]\\n`"
    );
    assert!(out.stderr.is_empty(), "json stderr stays empty: {out:?}");
}

#[test]
fn validate_valid_json_flag_before_root_is_empty_array() {
    let root = valid_root();
    let out = run(&[
        "validate",
        "--json",
        root.to_str().expect("utf-8 fixture path"),
    ]);
    assert_eq!(out.status.code(), Some(0), "clean must exit 0: {out:?}");
    assert_eq!(
        out.stdout,
        b"[]\n".to_vec(),
        "clean json is exactly `[]\\n`"
    );
    assert!(out.stderr.is_empty(), "json stderr stays empty: {out:?}");
}

#[test]
fn broken_plain_prints_fifteen_data_owned_display_lines() {
    let root = broken_root();
    let out = run(&["validate", root.to_str().expect("utf-8 fixture path")]);
    assert_eq!(out.status.code(), Some(1), "broken must exit 1: {out:?}");
    assert!(out.stdout.is_empty(), "plain stdout stays empty: {out:?}");
    let snapshot = std::fs::read(snapshot_path()).expect("snapshot must exist");
    let diagnostics: Vec<crpg_data::Diagnostic> =
        serde_json::from_slice(&snapshot).expect("snapshot must parse");
    assert_eq!(
        diagnostics.len(),
        15,
        "fixture contract is fifteen findings"
    );
    let mut expected = String::new();
    for diagnostic in &diagnostics {
        expected.push_str(&diagnostic.to_string());
        expected.push('\n');
    }
    assert_eq!(
        out.stderr,
        expected.as_bytes(),
        "plain stderr is the data-owned lines"
    );
}

#[test]
fn broken_json_byte_equals_checked_in_snapshot() {
    let root = broken_root();
    let out = run(&[
        "validate",
        root.to_str().expect("utf-8 fixture path"),
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(1), "broken must exit 1: {out:?}");
    assert!(out.stderr.is_empty(), "json stderr stays empty: {out:?}");
    let snapshot = std::fs::read(snapshot_path()).expect("snapshot must exist");
    assert_eq!(
        out.stdout, snapshot,
        "json must byte-equal the T011a snapshot"
    );
    assert_canonical_array(&out.stdout);
}

#[test]
fn malformed_document_is_single_structural_diagnostic() {
    let root = copy_fixture("malformed-document");
    std::fs::write(
        root.join("creatures").join("creature.json"),
        b"this is not json{{{",
    )
    .expect("corrupt fixture copy");
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["validate", &arg, "--json"]);
    assert_eq!(out.status.code(), Some(1), "malformed must exit 1: {out:?}");
    assert!(out.stderr.is_empty(), "json stderr stays empty: {out:?}");
    let diagnostics = assert_canonical_array(&out.stdout);
    assert_eq!(
        diagnostics.len(),
        1,
        "structural failures stay fail-fast: {diagnostics:?}"
    );
    assert_eq!(diagnostics[0].code, crpg_data::DiagnosticCode::Malformed);
    assert_eq!(
        diagnostics[0].file.as_ref().map(|file| file.as_str()),
        Some("creatures/creature.json")
    );
    cleanup(&root);
}

#[test]
fn unsupported_schema_is_single_structural_diagnostic() {
    let root = copy_fixture("unsupported-schema");
    let path = root.join("creatures").join("creature.json");
    let text = std::fs::read_to_string(&path).expect("fixture copy must read");
    assert!(
        text.contains("\"schema\": \"crpg.creature/1\""),
        "fixture pins its schema tag"
    );
    std::fs::write(
        &path,
        text.replace(
            "\"schema\": \"crpg.creature/1\"",
            "\"schema\": \"crpg.creature/99\"",
        ),
    )
    .expect("rewrite schema tag");
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["validate", &arg, "--json"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "unsupported schema must exit 1: {out:?}"
    );
    let diagnostics = assert_canonical_array(&out.stdout);
    assert_eq!(
        diagnostics.len(),
        1,
        "structural failures stay fail-fast: {diagnostics:?}"
    );
    assert_eq!(
        diagnostics[0].code,
        crpg_data::DiagnosticCode::UnsupportedSchema
    );
    assert_eq!(
        diagnostics[0].file.as_ref().map(|file| file.as_str()),
        Some("creatures/creature.json")
    );
    cleanup(&root);
}

#[test]
fn missing_required_file_is_single_layout_diagnostic() {
    let root = copy_fixture("missing-required-file");
    std::fs::remove_file(root.join("campaign.lock")).expect("remove lock from copy");
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["validate", &arg, "--json"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "missing file must exit 1: {out:?}"
    );
    let diagnostics = assert_canonical_array(&out.stdout);
    assert_eq!(
        diagnostics.len(),
        1,
        "structural failures stay fail-fast: {diagnostics:?}"
    );
    assert_eq!(diagnostics[0].code, crpg_data::DiagnosticCode::Layout);
    assert!(diagnostics[0].file.is_none());
    assert!(
        diagnostics[0].message.contains("missing required file"),
        "layout names the missing file: {}",
        diagnostics[0].message
    );
    cleanup(&root);
}

#[test]
fn impossible_engine_is_single_engine_incompatible_diagnostic() {
    let root = copy_fixture("impossible-engine");
    let path = root.join("campaign.json");
    let text = std::fs::read_to_string(&path).expect("fixture copy must read");
    assert!(
        text.contains(">=0.1.0, <1.0.0"),
        "fixture pins its engine range"
    );
    std::fs::write(&path, text.replace(">=0.1.0, <1.0.0", ">=99.0.0"))
        .expect("rewrite engine range");
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["validate", &arg, "--json"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "engine mismatch must exit 1: {out:?}"
    );
    let diagnostics = assert_canonical_array(&out.stdout);
    assert_eq!(
        diagnostics.len(),
        1,
        "structural failures stay fail-fast: {diagnostics:?}"
    );
    assert_eq!(
        diagnostics[0].code,
        crpg_data::DiagnosticCode::EngineIncompatible
    );
    assert!(diagnostics[0].file.is_none());
    cleanup(&root);
}

#[test]
fn garbage_lock_is_single_malformed_diagnostic() {
    let root = copy_fixture("garbage-lock");
    std::fs::write(root.join("campaign.lock"), b"\x00\x01 not json").expect("corrupt lock copy");
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["validate", &arg, "--json"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "garbage lock must exit 1: {out:?}"
    );
    let diagnostics = assert_canonical_array(&out.stdout);
    assert_eq!(
        diagnostics.len(),
        1,
        "structural failures stay fail-fast: {diagnostics:?}"
    );
    assert_eq!(diagnostics[0].code, crpg_data::DiagnosticCode::Malformed);
    assert_eq!(
        diagnostics[0].file.as_ref().map(|file| file.as_str()),
        Some("campaign.lock")
    );
    cleanup(&root);
}

#[test]
fn accepted_family_bad_name_is_invalid_path_not_io() {
    let root = copy_fixture("invalid-path");
    std::fs::write(root.join("creatures").join("bad name!.json"), b"ignored")
        .expect("bad-name file must create");
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["validate", &arg, "--json"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "invalid path must exit 1: {out:?}"
    );
    let diagnostics = assert_canonical_array(&out.stdout);
    assert_eq!(
        diagnostics.len(),
        1,
        "classifier failure is one diagnostic: {diagnostics:?}"
    );
    assert_eq!(diagnostics[0].code, crpg_data::DiagnosticCode::InvalidPath);
    assert!(diagnostics[0].file.is_none());
    let plain = run(&["validate", &arg]);
    assert_eq!(plain.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&plain.stderr);
    assert!(
        stderr.contains("invalid_path"),
        "classifier code preserved: {stderr}"
    );
    assert!(
        !stderr.contains("[io]"),
        "classifier failure is never io: {stderr}"
    );
    cleanup(&root);
}

#[test]
fn missing_root_is_exit_1_not_found() {
    let mut missing = std::env::temp_dir();
    missing.push(format!("crpg-validate-{}-no-such-root", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);
    let arg = missing.to_str().expect("utf-8 temp path").to_owned();

    let plain = run(&["validate", &arg]);
    assert_eq!(
        plain.status.code(),
        Some(1),
        "missing root must exit 1: {plain:?}"
    );
    assert!(plain.stdout.is_empty());
    assert_eq!(
        plain.stderr,
        b"<campaign>: error[io]: cannot open root <campaign-root>: not_found\n".to_vec()
    );

    let json = run(&["validate", &arg, "--json"]);
    assert_eq!(json.status.code(), Some(1));
    assert!(json.stderr.is_empty());
    let diagnostics = assert_canonical_array(&json.stdout);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, crpg_data::DiagnosticCode::Io);
    assert!(diagnostics[0].file.is_none());
    assert_eq!(
        diagnostics[0].message,
        "cannot open root <campaign-root>: not_found"
    );
}

#[test]
fn file_as_root_is_exit_1_not_a_directory() {
    let dir = temp_root("file-as-root");
    let file = dir.join("root.json");
    std::fs::write(&file, b"{}").expect("temp file must write");
    let arg = file.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["validate", &arg]);
    assert_eq!(out.status.code(), Some(1), "file root must exit 1: {out:?}");
    assert!(out.stdout.is_empty());
    assert_eq!(
        out.stderr,
        b"<campaign>: error[io]: cannot open root <campaign-root>: not_a_directory\n".to_vec()
    );
    cleanup(&dir);
}

#[test]
fn ignored_files_are_never_read() {
    let root = copy_fixture("ignored-files");
    // Garbage content in every ignored shape: if the walker read or parsed
    // any of these, validation could not stay clean.
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
    let arg = root.to_str().expect("utf-8 temp path").to_owned();

    let plain = run(&["validate", &arg]);
    assert_eq!(
        plain.status.code(),
        Some(0),
        "ignored junk stays clean: {plain:?}"
    );
    assert!(plain.stdout.is_empty());
    assert!(plain.stderr.is_empty());

    let json = run(&["validate", &arg, "--json"]);
    assert_eq!(json.status.code(), Some(0));
    assert_eq!(json.stdout, b"[]\n".to_vec());
    assert!(json.stderr.is_empty());
    cleanup(&root);
}

/// Real black-box symlink case, compile-time gated to platforms where
/// symlink creation is guaranteed. Windows without privileges cannot create
/// one, so the same shape is pinned there through the collector's injected
/// metadata seam instead — never a runtime skip.
#[cfg(unix)]
#[test]
fn symlink_at_document_path_is_exit_1() {
    use std::os::unix::fs::symlink;
    let dir = temp_root("symlink-document");
    symlink(
        valid_root().join("campaign.json"),
        dir.join("campaign.json"),
    )
    .expect("symlink must create");
    let arg = dir.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["validate", &arg, "--json"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "document symlink must exit 1: {out:?}"
    );
    assert!(out.stderr.is_empty());
    let diagnostics = assert_canonical_array(&out.stdout);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, crpg_data::DiagnosticCode::Io);
    assert_eq!(
        diagnostics[0].file.as_ref().map(|file| file.as_str()),
        Some("campaign.json")
    );
    assert_eq!(
        diagnostics[0].message,
        "cannot classify campaign.json: symlink_at_document_path"
    );
    cleanup(&dir);
}

/// A symlink at an ignored path is ignored, never followed and never read:
/// the empty collection then fails downstream as one layout diagnostic.
#[cfg(unix)]
#[test]
fn symlink_at_ignored_path_is_ignored() {
    use std::os::unix::fs::symlink;
    let dir = temp_root("symlink-ignored");
    symlink(valid_root().join("campaign.json"), dir.join("notes.txt"))
        .expect("symlink must create");
    let arg = dir.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["validate", &arg, "--json"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "empty collection must exit 1: {out:?}"
    );
    let diagnostics = assert_canonical_array(&out.stdout);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, crpg_data::DiagnosticCode::Layout);
    assert!(
        diagnostics[0].message.contains("missing required file"),
        "ignored symlink leaves an empty map: {}",
        diagnostics[0].message
    );
    cleanup(&dir);
}

/// Non-Unicode roots reach the binary through `args_os` and fail as `io`,
/// never as usage and never as a panic. Unix-only: only Unix can spell such
/// a path for a child process portably.
#[cfg(unix)]
#[test]
fn non_unicode_root_is_exit_1_io() {
    use std::os::unix::ffi::OsStringExt;
    let bad = OsString::from_vec(b"/tmp/crpg-validate-\xff-root".to_vec());
    let out = run_os(&[OsString::from("validate"), bad]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "non-Unicode root must exit 1: {out:?}"
    );
    assert!(out.stdout.is_empty());
    assert_eq!(
        out.stderr,
        b"<campaign>: error[io]: cannot open root <campaign-root>: non_unicode_component\n"
            .to_vec()
    );
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
    let arg = dir.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["validate", &arg]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "non-Unicode entry must exit 1: {out:?}"
    );
    assert!(out.stdout.is_empty());
    assert_eq!(
        out.stderr,
        b"<campaign>: error[io]: cannot list <campaign-root>: non_unicode_component\n".to_vec()
    );
    cleanup(&dir);
}

/// Two locale documents colliding under ASCII case-folding both classify, so
/// the collision reaches data and reports `layout`. Linux-only: Windows
/// cannot create both names in one directory.
#[cfg(target_os = "linux")]
#[test]
fn ascii_case_collision_reaches_data_as_layout() {
    let root = copy_fixture("case-collision");
    let text = std::fs::read_to_string(root.join("locale").join("en.json"))
        .expect("locale copy must read");
    assert!(text.contains("\"locale\": \"en\""), "locale pins its tag");
    std::fs::write(
        root.join("locale").join("EN.json"),
        text.replace("\"locale\": \"en\"", "\"locale\": \"EN\""),
    )
    .expect("colliding locale must write");
    let arg = root.to_str().expect("utf-8 temp path").to_owned();
    let out = run(&["validate", &arg, "--json"]);
    assert_eq!(out.status.code(), Some(1), "collision must exit 1: {out:?}");
    assert!(out.stderr.is_empty());
    let diagnostics = assert_canonical_array(&out.stdout);
    assert_eq!(
        diagnostics.len(),
        1,
        "collision is one diagnostic: {diagnostics:?}"
    );
    assert_eq!(diagnostics[0].code, crpg_data::DiagnosticCode::Layout);
    cleanup(&root);
}

/// Recursively discovers campaign roots under `fixtures`: directories that
/// directly contain `campaign.json`, reported as `/`-joined text relative to
/// `fixtures` and sorted lexically. An enclosing version directory without
/// its own `campaign.json` is never a root.
fn discover_campaign_roots(fixtures: &Path) -> Vec<String> {
    let mut roots = Vec::new();
    let mut stack = vec![fixtures.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|_| panic!("must list {}", dir.display()))
            .map(|entry| entry.expect("fixture entry").path())
            .collect();
        entries.sort();
        for entry in &entries {
            if entry.is_dir() {
                stack.push(entry.clone());
            }
        }
        if dir != fixtures && dir.join("campaign.json").is_file() {
            let relative = dir.strip_prefix(fixtures).expect("under fixtures");
            let text = relative
                .components()
                .map(|component| component.as_os_str().to_str().expect("utf-8 fixture names"))
                .collect::<Vec<_>>()
                .join("/");
            roots.push(text);
        }
    }
    roots.sort();
    roots
}

/// Gate 8: T011a's data-owned manifest drives the shipped binary over every
/// current fixture campaign. Fails on a missing/malformed/unsorted manifest,
/// on any discovered root the manifest does not list (or vice versa), and on
/// any expectation mismatch. Cannot pass vacuously: an empty manifest or an
/// empty discovery both fail.
#[test]
fn gate_8_manifest_drives_every_fixture_campaign() {
    let fixtures = fixtures_dir();
    let manifest_bytes =
        std::fs::read(fixtures.join("expected.json")).expect("T011a manifest must exist");
    let manifest: serde_json::Value =
        serde_json::from_slice(&manifest_bytes).expect("manifest must parse as JSON");
    let entries = manifest.as_array().expect("manifest must be an array");
    assert!(
        !entries.is_empty(),
        "manifest must list at least one fixture"
    );

    let mut manifest_roots = Vec::new();
    let mut expectations = std::collections::BTreeMap::new();
    for entry in entries {
        let obj = entry.as_object().expect("manifest entries must be objects");
        let keys: BTreeSet<&String> = obj.keys().collect();
        let wanted: BTreeSet<String> = ["expect", "root", "snapshot"]
            .iter()
            .map(|key| (*key).to_owned())
            .collect();
        let found: BTreeSet<String> = keys.iter().map(|key| (*key).clone()).collect();
        assert_eq!(
            found, wanted,
            "manifest entries hold exactly root/expect/snapshot"
        );
        let root = obj["root"]
            .as_str()
            .expect("root must be a string")
            .to_owned();
        let expect = obj["expect"]
            .as_str()
            .expect("expect must be a string")
            .to_owned();
        assert!(
            expect == "clean" || expect == "diagnostics",
            "unknown expectation for {root}: {expect}"
        );
        let snapshot = match &obj["snapshot"] {
            serde_json::Value::Null => None,
            serde_json::Value::String(text) => Some(text.clone()),
            other => panic!("snapshot for {root} must be a string or null: {other}"),
        };
        if expect == "diagnostics" {
            assert!(
                snapshot.is_some(),
                "diagnostics entry {root} must name a snapshot"
            );
        }
        manifest_roots.push(root.clone());
        expectations.insert(root, (expect, snapshot));
    }
    assert!(
        manifest_roots.windows(2).all(|pair| pair[0] < pair[1]),
        "manifest entries must be sorted by root: {manifest_roots:?}"
    );

    let discovered = discover_campaign_roots(&fixtures);
    assert!(
        !discovered.is_empty(),
        "must discover at least one fixture campaign"
    );
    assert_eq!(
        discovered, manifest_roots,
        "manifest must list exactly the discovered campaign roots"
    );

    for root in &manifest_roots {
        let (expect, snapshot) = &expectations[root];
        let mut campaign = fixtures.clone();
        for part in root.split('/') {
            campaign.push(part);
        }
        let arg = campaign.to_str().expect("utf-8 fixture path").to_owned();
        let out = run(&["validate", &arg, "--json"]);
        match expect.as_str() {
            "clean" => {
                assert_eq!(
                    out.status.code(),
                    Some(0),
                    "{root} must validate clean: {out:?}"
                );
                assert_eq!(
                    out.stdout,
                    b"[]\n".to_vec(),
                    "{root} clean json is exactly `[]\\n`"
                );
                assert!(
                    out.stderr.is_empty(),
                    "{root} json stderr stays empty: {out:?}"
                );
            }
            "diagnostics" => {
                assert_eq!(out.status.code(), Some(1), "{root} must fail: {out:?}");
                assert!(
                    out.stderr.is_empty(),
                    "{root} json stderr stays empty: {out:?}"
                );
                let expected =
                    std::fs::read(data_dir().join(snapshot.as_ref().expect("snapshot required")))
                        .expect("snapshot must exist");
                assert_eq!(
                    out.stdout, expected,
                    "{root} json must byte-equal its snapshot"
                );
                assert_canonical_array(&out.stdout);
            }
            other => panic!("unknown expectation for {root}: {other}"),
        }
    }
}
