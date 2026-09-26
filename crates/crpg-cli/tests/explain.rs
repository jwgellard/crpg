//! End-to-end tests for `crpgc explain` (T013).
//!
//! These drive the built binary — `env!("CARGO_BIN_EXE_crpgc")` — as a black
//! box through `std::process::Command`, asserting exit codes and exact stream
//! bytes. The landed T013a `explain_object` owns every reference semantic;
//! the CLI owns argument parsing, collection, the one load call, and the
//! process treatment of `None`. Fixtures are read-only inputs; mutating
//! cases operate on private copies.

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

/// The `crpg-data` crate directory: fixtures and snapshots live here.
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

/// The fifteen-finding T011a fixture campaign root, consumed read-only.
fn broken_root() -> PathBuf {
    data_dir()
        .join("tests")
        .join("fixtures")
        .join("broken_references")
}

/// The authoritative old T012a campaign root, consumed read-only.
fn migration_v1_root() -> PathBuf {
    data_dir()
        .join("tests")
        .join("fixtures")
        .join("migration_v1")
        .join("campaign")
}

/// A fresh unique temp directory for one test.
fn temp_root(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("crpg-explain-{}-{name}", std::process::id()));
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

/// Maps every file under `root` to its bytes, for read-only proofs.
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

const EXPLAIN_USAGE: &str = "crpgc: usage: crpgc explain <id> [--root <campaign-root>]\n";

const CREATURE_ID: &str = "00000000000000000000000004";
const PLACEMENT_ID: &str = "00000000000000000000000005";
const CAMPAIGN_ID: &str = "00000000000000000000000001";
const UNKNOWN_ID: &str = "00000000000000000000000099";

/// Independently authored expected report for the fixture creature: the
/// T013a shape (canonical key order, uppercase id, logical file, empty
/// object pointer, current object shape, one valid inbound placement edge,
/// empty outbound) through canonical JSON with one final LF.
const EXPECTED_CREATURE_REPORT: &str = "{\n  \"file\": \"creatures/creature.json\",\n  \"id\": \"00000000000000000000000004\",\n  \"inbound\": [\n    {\n      \"file\": \"areas/start/placements.json\",\n      \"pointer\": \"/placements/0/prefab\",\n      \"source\": \"00000000000000000000000005\",\n      \"target\": \"00000000000000000000000004\",\n      \"target_location\": {\n        \"file\": \"creatures/creature.json\",\n        \"kind\": \"creature\",\n        \"pointer\": \"\"\n      }\n    }\n  ],\n  \"kind\": \"creature\",\n  \"object\": {\n    \"faction\": null,\n    \"id\": \"00000000000000000000000004\",\n    \"inventory\": [],\n    \"name\": \"fixture.creature\",\n    \"schema\": \"crpg.creature/1\",\n    \"slug\": \"creature\",\n    \"stats\": {\n      \"health\": 655360\n    },\n    \"tags\": [\n      \"fixture\"\n    ]\n  },\n  \"outbound\": [],\n  \"pointer\": \"\"\n}\n";

#[test]
fn explain_usage_matrix_reports_one_line_with_exit_2() {
    let cases: Vec<Vec<&str>> = vec![
        vec!["explain"],
        vec!["explain", CREATURE_ID, CREATURE_ID],
        vec!["explain", "--root"],
        vec!["explain", CREATURE_ID, "--root"],
        vec!["explain", "--root", "a", "--root", "b", CREATURE_ID],
        vec!["explain", "--bogus", CREATURE_ID],
        vec!["explain", CREATURE_ID, "--bogus"],
        vec!["explain", "--", CREATURE_ID],
        vec!["explain", "short"],
        vec!["explain", "0000000000000000000000001!"],
        vec!["explain", "80000000000000000000000000"],
        vec!["explain", " 00000000000000000000000004"],
    ];
    for argv in &cases {
        let output = run(argv);
        assert_eq!(output.status.code(), Some(2), "argv {argv:?}");
        assert!(output.stdout.is_empty(), "argv {argv:?}");
        assert_eq!(output.stderr, EXPLAIN_USAGE.as_bytes(), "argv {argv:?}");
    }
}

#[test]
fn explain_flag_shaped_root_values_are_usage_before_filesystem_access() {
    // Flag-shaped `--root` values never reach I/O: usage exit 2 even when
    // the root does not exist, proving parsing completes before any
    // filesystem access.
    for value in [
        "--help",
        "--version",
        "--",
        "-x",
        "--root",
        "--catalog",
        "--bogus",
    ] {
        let output = run(&["explain", CREATURE_ID, "--root", value]);
        assert_eq!(output.status.code(), Some(2), "value {value:?}");
        assert!(output.stdout.is_empty(), "value {value:?}");
        assert_eq!(output.stderr, EXPLAIN_USAGE.as_bytes(), "value {value:?}");
        // Missing-root form proves usage wins before I/O.
        let output = run(&["explain", "00000000000000000000000004", "--root", value]);
        assert_eq!(output.status.code(), Some(2), "value {value:?}");
        assert_eq!(output.stderr, EXPLAIN_USAGE.as_bytes());
    }
    // An explicit relative prefix escapes a dash-leading path: it reaches
    // I/O (exit 1, not usage) instead of failing as usage.
    let output = run(&["explain", CREATURE_ID, "--root", "./-dash-root"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
    assert_ne!(output.stderr, EXPLAIN_USAGE.as_bytes());
}

#[test]
fn explain_malformed_id_beats_a_missing_root() {
    // Parsing failure wins before I/O: the root does not exist, yet the
    // malformed id still reports usage, not a collection failure.
    let output = run(&["explain", "bogus", "--root", "/definitely/not/here"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, EXPLAIN_USAGE.as_bytes());
}

#[test]
fn explain_top_level_report_matches_independently_specified_bytes() {
    let root = valid_root();
    let output = run(&[
        "explain",
        CREATURE_ID,
        "--root",
        root.to_str().expect("fixture root is unicode"),
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, EXPECTED_CREATURE_REPORT.as_bytes());
}

#[test]
fn explain_binary_output_equals_the_data_api_bytes() {
    // Transport assertion alongside the independent oracle above: the
    // binary writes the exact `explain_object` bytes, not a reserialized
    // copy. Files are collected with the data classifier, loaded with the
    // package engine version, and queried once.
    use std::collections::BTreeMap;
    let root = valid_root();
    let output = run(&[
        "explain",
        CREATURE_ID,
        "--root",
        root.to_str().expect("unicode"),
    ]);
    assert_eq!(output.status.code(), Some(0));
    let mut files: BTreeMap<crpg_data::SourcePath, Vec<u8>> = BTreeMap::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("must list")
            .map(|entry| entry.expect("entry").path())
            .collect();
        entries.sort();
        for entry in entries {
            if entry.is_dir() {
                stack.push(entry);
                continue;
            }
            let relative = entry
                .strip_prefix(&root)
                .expect("prefix")
                .to_str()
                .expect("unicode")
                .replace('\\', "/");
            if let Some(path) = crpg_data::campaign_document_path(&relative)
                .expect("classifier must not fail on fixture")
            {
                files.insert(path, std::fs::read(&entry).expect("must read"));
            }
        }
    }
    let engine = env!("CARGO_PKG_VERSION").parse().expect("engine parses");
    let loaded = crpg_data::load_campaign(&files, &engine).expect("fixture loads");
    let id: crpg_core::Ulid = CREATURE_ID.parse().expect("id parses");
    let expected = crpg_data::explain_object(&loaded, id)
        .expect("explain must not fail")
        .expect("creature must exist");
    assert_eq!(output.stdout, expected);
}

#[test]
fn explain_root_may_lead_and_repeats_identically() {
    let root = valid_root();
    let root_text = root.to_str().expect("fixture root is unicode");
    let first = run(&["explain", "--root", root_text, CREATURE_ID]);
    let second = run(&["explain", CREATURE_ID, "--root", root_text]);
    assert_eq!(first.status.code(), Some(0));
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(first.stdout, EXPECTED_CREATURE_REPORT.as_bytes());
}

#[test]
fn explain_core_aliases_normalize_through_the_api() {
    // Lowercase `i` decodes as `1` in core; the CLI performs no report
    // rewrite, so the aliased invocation returns byte-identical output with
    // the canonical uppercase id inside.
    let root = valid_root();
    let root_text = root.to_str().expect("fixture root is unicode");
    let aliased = CAMPAIGN_ID.replace('1', "i");
    assert_ne!(aliased, CAMPAIGN_ID);
    let output = run(&["explain", &aliased, "--root", root_text]);
    assert_eq!(output.status.code(), Some(0));
    let canonical = run(&["explain", CAMPAIGN_ID, "--root", root_text]);
    assert_eq!(output.stdout, canonical.stdout);
    assert!(String::from_utf8(output.stdout)
        .expect("utf-8")
        .contains(CAMPAIGN_ID));
}

#[test]
fn explain_nested_placement_and_root_campaign_shapes() {
    let root = valid_root();
    let root_text = root.to_str().expect("fixture root is unicode");
    // The aggregate-owned placement is a nested identified object: its
    // report keeps the nested shape (no schema envelope) with its prefab
    // outbound edge intact.
    let output = run(&["explain", PLACEMENT_ID, "--root", root_text]);
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("report must parse");
    assert_eq!(
        report["kind"],
        serde_json::Value::String("placement".into())
    );
    assert_eq!(
        report["file"],
        serde_json::Value::String("areas/start/placements.json".into())
    );
    assert!(
        report["object"].get("schema").is_none(),
        "nested shape keeps no envelope"
    );
    assert_eq!(
        report["object"]["prefab"],
        serde_json::Value::String(CREATURE_ID.into())
    );
    let outbound = report["outbound"].as_array().expect("outbound is an array");
    assert_eq!(outbound.len(), 1, "prefab edge must be outbound");
    assert_eq!(
        outbound[0]["target"],
        serde_json::Value::String(CREATURE_ID.into())
    );
    // The campaign root keeps its envelope and carries entry outbound edges.
    let output = run(&["explain", CAMPAIGN_ID, "--root", root_text]);
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("report must parse");
    assert_eq!(report["kind"], serde_json::Value::String("campaign".into()));
    assert!(
        report["object"].get("schema").is_some(),
        "root keeps its envelope"
    );
    assert!(
        report["outbound"].as_array().expect("array").len() >= 3,
        "entry world/area/spawn must be outbound"
    );
}

#[test]
fn explain_unknown_id_reports_not_found_with_canonical_id() {
    let root = valid_root();
    let output = run(&[
        "explain",
        UNKNOWN_ID,
        "--root",
        root.to_str().expect("unicode"),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        format!("crpgc explain: object not found: {UNKNOWN_ID}\n").into_bytes()
    );
}

#[test]
fn explain_structural_failure_wins_over_absence() {
    // A valid but unknown id does not bypass structural failure: this
    // directory misses the required files, so the data diagnostic wins over
    // the not-found line.
    let root = temp_root("structural-wins");
    let output = run(&[
        "explain",
        UNKNOWN_ID,
        "--root",
        root.to_str().expect("unicode"),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        !output.stderr.is_empty()
            && output.stderr
                != format!("crpgc explain: object not found: {UNKNOWN_ID}\n").into_bytes(),
        "structural failure must win: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn explain_broken_references_still_explains_with_dangling_outbound() {
    let root = broken_root();
    // The broken fixture's first creature keeps its inbound/outbound
    // inventory despite semantic findings; no validation gate runs first.
    // Discover one existing object id from the fixture's creature file.
    let creature_bytes =
        std::fs::read(root.join("creatures").join("creature.json")).expect("creature must read");
    let creature: serde_json::Value =
        serde_json::from_slice(&creature_bytes).expect("creature must parse");
    let id = creature["id"]
        .as_str()
        .expect("creature has an id")
        .to_owned();
    let output = run(&["explain", &id, "--root", root.to_str().expect("unicode")]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("report must parse");
    assert_eq!(report["id"], serde_json::Value::String(id));
    // At least one outbound edge dangles: its target location is null while
    // the edge itself is retained.
    let outbound = report["outbound"].as_array().expect("outbound is an array");
    assert!(
        outbound
            .iter()
            .any(|edge| edge["target_location"].is_null()),
        "broken input must retain a dangling outbound edge"
    );
}

#[test]
fn explain_historical_input_reports_current_shape_without_writes() {
    let root = temp_root("historical");
    let trial = root.join("campaign");
    copy_dir(&migration_v1_root(), &trial);
    let before = snapshot_dir(&trial);
    // The old campaign's creature-equivalent: read one id from a historical
    // creature file and explain it through migration-aware loading.
    let mut creature_id = None;
    let mut stack = vec![trial.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("must list") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .parent()
                .and_then(|parent| parent.file_name())
                .is_some_and(|name| name == "creatures")
            {
                let value: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(&path).expect("must read"))
                        .expect("must parse");
                if let Some(id) = value.get("id").and_then(|id| id.as_str()) {
                    creature_id = Some(id.to_owned());
                    break;
                }
            }
        }
    }
    let id = creature_id.expect("historical campaign has a creature");
    let output = run(&["explain", &id, "--root", trial.to_str().expect("unicode")]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("report must parse");
    assert_eq!(report["id"], serde_json::Value::String(id));
    // An old item whose envelope demonstrably changes (v1 `crpg.item/1` on
    // disk) reports the current migrated shape (`crpg.item/2`) without
    // rewriting historical files.
    const OLD_ITEM_ID: &str = "00000000000000000000000008";
    let item_output = run(&[
        "explain",
        OLD_ITEM_ID,
        "--root",
        trial.to_str().expect("unicode"),
    ]);
    assert_eq!(
        item_output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&item_output.stderr)
    );
    let item_report: serde_json::Value =
        serde_json::from_slice(&item_output.stdout).expect("item report must parse");
    assert_eq!(
        item_report["id"],
        serde_json::Value::String(OLD_ITEM_ID.to_owned())
    );
    assert_eq!(
        item_report["kind"],
        serde_json::Value::String("item".into())
    );
    assert_eq!(
        item_report["object"]["schema"],
        serde_json::Value::String("crpg.item/2".into())
    );
    // Migration-aware loading reports the current shape without rewriting
    // historical files.
    assert_eq!(snapshot_dir(&trial), before, "explain must be read-only");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn explain_is_read_only_on_the_checked_in_tree() {
    let root = valid_root();
    let before = snapshot_dir(&root);
    let output = run(&[
        "explain",
        CREATURE_ID,
        "--root",
        root.to_str().expect("unicode"),
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(snapshot_dir(&root), before, "explain must not write");
}

#[test]
fn explain_missing_root_is_domain_failure_not_usage() {
    let missing = temp_root("missing-root");
    let _ = std::fs::remove_dir_all(&missing);
    let output = run(&[
        "explain",
        CREATURE_ID,
        "--root",
        missing.to_str().expect("unicode"),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}
