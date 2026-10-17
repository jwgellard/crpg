//! T127: targets for the `crpg-cli` gaps the 2026-10-16 crate audit left
//! (tests only).
//!
//! Every target here drives the built `crpgc` binary as a black box — the
//! same `env!("CARGO_BIN_EXE_crpgc")` process discipline the existing suites
//! use — and asserts exit codes plus exact stream bytes. No source, fixture or
//! golden is touched; temporary inputs live under the process temp directory
//! at run time, never in the repository. Portable on both ADR-0012 natives.
//!
//! Lead map (finding # -> test):
//! - #16/#75 `run` sampling/hex are re-derived through a different encoder and
//!   a different indexing property than the CLI's own loop.
//! - #18 combat-mode semantic findings are pinned byte-for-byte in returned
//!   order, not merely "some error, and not the later ones".

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

/// A fresh unique temp directory for one test; stale leftovers are removed so
/// a crashed earlier run cannot fake a success.
fn temp_dir(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("crpg-cli-audit-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp dir must create");
    path
}

/// The `crpg-data` broken-reference fixture (read-only), whose semantic
/// findings are data-owned and already pinned by `crpg-data`'s own suite.
fn broken_references_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("crpg-data")
        .join("tests")
        .join("fixtures")
        .join("broken_references")
}

/// One `init` input against the `combat_basic` encounter identity. The replay
/// never reaches application in the #18 test because the campaign fails
/// semantic validation first.
const COMBAT_INIT_REPLAY: &str = "\
{
  \"format_version\": 1,
  \"seed\": 0,
  \"campaign_id\": \"combat-basic\",
  \"campaign_version\": \"0.1.0\",
  \"engine_version\": \"0.1.0\",
  \"total_ticks\": 1,
  \"inputs\": [
    {
      \"tick\": 0,
      \"payload\": { \"combat\": 1, \"op\": \"init\", \"encounter\": \"0000000000000000000000000N\" }
    }
  ]
}
";

/// Lowercase hex for a 32-byte hash using the standard formatter, deliberately
/// a different code path than the CLI's hand-rolled nibble encoder.
fn std_hex(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// #16, #75 — `crpgc run` output must be the harness sample stream, not a
/// mirror of the CLI's own encoder or loop.
///
/// Independent derivation:
/// - the hash text is re-encoded with `{byte:02x}` (std) rather than a copy of
///   the CLI's hex table, and each published hash is decoded back with
///   `u8::from_str_radix` and compared to the harness bytes;
/// - sample `k` is asserted to be the *last* state of a `k`-tick harness run
///   (`run_hash_sequence(seed, k)[k - 1]`), which pins index `k - 1` without
///   reusing the CLI's `hashes[tick - 1]` loop, and pins the label sequence
///   `M, 2M, ... <= N` with no header and no forced final partial sample.
#[test]
fn run_samples_are_independently_reencoded_and_use_index_k_minus_1() {
    const TICKS: usize = 7;
    const EVERY: usize = 3;
    const SEED: u64 = 9;

    let out = run(&["run", "--ticks", "7", "--hash-every", "3", "--seed", "9"]);
    assert_eq!(out.status.code(), Some(0), "run must succeed: {out:?}");
    assert!(out.stderr.is_empty(), "run is silent on stderr: {out:?}");
    let text = String::from_utf8(out.stdout.clone()).expect("stdout is utf-8");

    // Independent expected text: labels 3 and 6 only, each the final state of
    // a run that stops at that label.
    let mut expected = String::new();
    let mut label = EVERY;
    while label <= TICKS {
        let short = crpg_testkit::run_hash_sequence(SEED, label, Box::new(|_| {}));
        expected.push_str(&format!("{label} {}\n", std_hex(&short[label - 1])));
        label += EVERY;
    }
    assert_eq!(
        text, expected,
        "sampled bytes must match independent derivation"
    );

    // Independent decoders and the full harness sequence: no header, no
    // initial-state line, and every published hash equals the harness hash for
    // the completed tick.
    let full = crpg_testkit::run_hash_sequence(SEED, TICKS, Box::new(|_| {}));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), TICKS / EVERY, "floor(N/M) samples only");
    for (index, line) in lines.iter().enumerate() {
        let label = (index + 1) * EVERY;
        let (rendered_label, hash) = line.split_once(' ').expect("label and hash");
        assert_eq!(rendered_label, label.to_string(), "sample label is tick k");
        assert_eq!(hash.len(), 64, "hash is 64 hex chars");
        let decoded: Vec<u8> = hash
            .as_bytes()
            .chunks(2)
            .map(|pair| {
                u8::from_str_radix(std::str::from_utf8(pair).expect("ascii"), 16)
                    .expect("lowercase hex decodes")
            })
            .collect();
        assert_eq!(
            decoded.as_slice(),
            &full[label - 1][..],
            "sample k is the post-tick-k hash at index k-1"
        );
        assert!(
            hash.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "lowercase hex only: {hash}"
        );
    }
}

/// #18 — a semantically invalid campaign must render the data-owned findings,
/// in the order `crpg_data::validate` returned them, before any replay work.
///
/// The 15 lines below are the `broken_references` fixture's checked-in
/// findings as `crpg-data` owns them; the combat replay path must print exactly
/// these bytes on stderr, in this order, with empty stdout and exit 1. A
/// dropped, reordered, duplicated or reworded finding changes the bytes.
fn broken_references_findings() -> String {
    const LINES: &[&str] = &[
        "areas/start/area.json/ambience: error[missing_asset]: asset key \"assets/missing.ogg\" is not in assets.lock; suggested fix: add the asset to assets.lock or fix the key",
        "areas/start/area.json/neighbours/0: error[dangling_reference]: dangling area neighbour reference to 000000000000000000000000W5; suggested fix: point at an existing object id or create the target",
        "areas/start/placements.json/area: error[aggregate_owner_mismatch]: placements owner 00000000000000000000000002 does not match the required owner 00000000000000000000000003; suggested fix: set the owner to the sibling or root id",
        "areas/start/placements.json/placements/0/overrides/broken/value: error[dangling_reference]: dangling object reference to 000000000000000000000000W7; suggested fix: point at an existing object id or create the target",
        "areas/start/placements.json/placements/0/prefab: error[wrong_reference_kind]: wrong kind for placement prefab reference to 00000000000000000000000002: found world, expected creature or item; suggested fix: point at an object of the expected kind",
        "areas/start/triggers.json/graphs/0/edges/0/port: error[graph_port_mismatch]: edge port true is not allowed for wait source node; suggested fix: use the port allowed by the source node body",
        "campaign.json/entry/area: error[dangling_reference]: dangling entry area reference to 000000000000000000000000W4; suggested fix: point at an existing object id or create the target",
        "campaign.json/entry/world: error[wrong_reference_kind]: wrong kind for entry world reference to 00000000000000000000000003: found area, expected world; suggested fix: point at an object of the expected kind",
        "creatures/creature.json/faction: error[wrong_reference_kind]: wrong kind for creature faction reference to 00000000000000000000000004: found creature, expected faction; suggested fix: point at an object of the expected kind",
        "creatures/creature.json/inventory/0: error[dangling_reference]: dangling creature inventory item reference to 000000000000000000000000W6; suggested fix: point at an existing object id or create the target",
        "creatures/z-duplicate.json/slug: error[duplicate_slug]: duplicate slug \"creature\" for creature (first at creatures/creature.json/slug); suggested fix: rename the later slug to a unique value within its type",
        "dialogue/broken.json/nodes/0/body/text_key: error[missing_locale_key]: locale key \"fixture.missing-key\" is missing from locale \"en\"; suggested fix: add \"fixture.missing-key\" to locale \"en\"",
        "dialogue/broken.json/nodes/1: error[unreachable_node]: dialogue node 0000000000000000000000000P is unreachable from the entry; suggested fix: connect the node from the entry or remove it",
        "quests/broken.json/entry: error[quest_no_completion_path]: quest has no reachable terminal state; suggested fix: add a transition path to a terminal state",
        "variables/campaign_state.json/variables/0/default: error[value_type_mismatch]: variable \"broken\" declares integer but the default is bool; suggested fix: change the default value or the declared type",
    ];
    let mut out = String::new();
    for line in LINES {
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[test]
fn combat_semantic_findings_render_in_returned_order() {
    let dir = temp_dir("broken-campaign-order");
    let replay = dir.join("init.replay");
    std::fs::write(&replay, COMBAT_INIT_REPLAY).expect("replay write must succeed");

    let out = run(&[
        "replay",
        replay.to_str().expect("utf-8 path"),
        "--campaign",
        broken_references_fixture().to_str().expect("utf-8 path"),
    ]);

    assert_eq!(
        out.status.code(),
        Some(1),
        "semantic findings must exit 1: {out:?}"
    );
    assert!(
        out.stdout.is_empty(),
        "diagnostics go to stderr only: {out:?}"
    );
    assert_eq!(
        String::from_utf8(out.stderr.clone()).expect("stderr is utf-8"),
        broken_references_findings(),
        "findings render in returned order, exactly once each"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
