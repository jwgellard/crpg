//! End-to-end tests for `crpgc new` and `crpgc schema` (T013).
//!
//! These drive the built binary — `env!("CARGO_BIN_EXE_crpgc")` — as a black
//! box through `std::process::Command`, asserting exit codes and exact stream
//! bytes. Scaffold expectations are independently specified from the T013
//! template table plus canonical-JSON rules, and every scaffold is installed
//! in a private copy of the clean `one_area_one_creature` fixture with
//! explicit locale support, requiring binary `validate --json` to return
//! exact `[]\n`. Schema bytes are compared read-only against the checked-in
//! `schemas/` directory. The checked-in tree is never mutated; tests copy
//! fixtures to private temporary directories.

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

/// The repository root: the CLI crate lives at `<root>/crates/crpg-cli`.
fn repo_root() -> PathBuf {
    cli_dir()
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must exist")
}

/// The `crpg-data` crate directory: fixtures live here.
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

/// A fresh unique temp directory for one test. Tests run in parallel, so the
/// name carries the test's own slug; stale leftovers are removed first so a
/// crashed run cannot fake a result.
fn temp_root(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("crpg-scaffolding-{}-{name}", std::process::id()));
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

const NEW_USAGE: &str = "crpgc: usage: crpgc new <type> --slug <s> --id <id> [--entry-id <id>]\n";
const SCHEMA_USAGE: &str = "crpgc: usage: crpgc schema <type>\n";

/// Fresh caller-supplied identities, disjoint from the fixture's
/// `...000001`–`...000005` range.
const CREATURE_ID: &str = "00000000000000000000000011";
const ITEM_ID: &str = "00000000000000000000000012";
const DIALOGUE_ID: &str = "00000000000000000000000013";
const DIALOGUE_ENTRY_ID: &str = "00000000000000000000000014";
const QUEST_ID: &str = "00000000000000000000000015";
const QUEST_ENTRY_ID: &str = "00000000000000000000000016";

/// Independently specified expected canonical bytes for the creature
/// scaffold: the T013 template (empty stats/tags/inventory, null faction,
/// absent notes) through canonical JSON (lexical keys, two spaces, one LF).
const EXPECTED_CREATURE: &str = "{\n  \"faction\": null,\n  \"id\": \"00000000000000000000000011\",\n  \"inventory\": [],\n  \"name\": \"creature.gadwall.name\",\n  \"schema\": \"crpg.creature/1\",\n  \"slug\": \"gadwall\",\n  \"stats\": {},\n  \"tags\": []\n}\n";

/// Independently specified expected canonical bytes for the item scaffold.
const EXPECTED_ITEM: &str = "{\n  \"id\": \"00000000000000000000000012\",\n  \"name\": \"item.baton.name\",\n  \"schema\": \"crpg.item/2\",\n  \"slug\": \"baton\",\n  \"stats\": {},\n  \"tags\": []\n}\n";

/// Independently specified expected canonical bytes for the dialogue
/// scaffold: one node carrying the entry id with an `end` body.
const EXPECTED_DIALOGUE: &str = "{\n  \"entry\": \"00000000000000000000000014\",\n  \"id\": \"00000000000000000000000013\",\n  \"name\": \"dialogue.parley.name\",\n  \"nodes\": [\n    {\n      \"body\": {\n        \"kind\": \"end\"\n      },\n      \"id\": \"00000000000000000000000014\"\n    }\n  ],\n  \"schema\": \"crpg.dialogue/1\",\n  \"slug\": \"parley\"\n}\n";

/// Independently specified expected canonical bytes for the quest scaffold:
/// one terminal state carrying the entry id with empty on-enter and
/// transitions.
const EXPECTED_QUEST: &str = "{\n  \"entry\": \"00000000000000000000000016\",\n  \"id\": \"00000000000000000000000015\",\n  \"name\": \"quest.courier.name\",\n  \"schema\": \"crpg.quest/1\",\n  \"slug\": \"courier\",\n  \"states\": [\n    {\n      \"id\": \"00000000000000000000000016\",\n      \"name\": \"quest.courier.state.done\",\n      \"on_enter\": [],\n      \"terminal\": true,\n      \"transitions\": []\n    }\n  ]\n}\n";

#[test]
fn new_usage_matrix_reports_one_line_with_exit_2() {
    let cases: Vec<Vec<&str>> = vec![
        vec!["new"],
        vec!["new", "creature"],
        vec!["new", "creature", "--slug", "gadwall"],
        vec!["new", "creature", "--id", CREATURE_ID],
        vec!["new", "golem", "--slug", "gadwall", "--id", CREATURE_ID],
        vec![
            "new",
            "creature",
            "--slug",
            "gadwall",
            "--id",
            CREATURE_ID,
            "--slug",
            "orc",
        ],
        vec![
            "new",
            "creature",
            "a",
            "b",
            "--slug",
            "gadwall",
            "--id",
            CREATURE_ID,
        ],
        vec![
            "new",
            "creature",
            "--slug",
            "gadwall",
            "--id",
            CREATURE_ID,
            "--bogus",
        ],
        vec![
            "new",
            "creature",
            "--slug",
            "gadwall",
            "--id",
            CREATURE_ID,
            "--",
        ],
        vec![
            "new",
            "creature",
            "--slug",
            "gadwall",
            "--id",
            CREATURE_ID,
            "-s",
        ],
        vec!["new", "creature", "--slug=gadwall", "--id", CREATURE_ID],
        vec!["new", "dialogue", "--slug", "parley", "--id", DIALOGUE_ID],
        vec!["new", "quest", "--slug", "courier", "--id", QUEST_ID],
        vec![
            "new",
            "creature",
            "--slug",
            "gadwall",
            "--id",
            CREATURE_ID,
            "--entry-id",
            DIALOGUE_ENTRY_ID,
        ],
        vec![
            "new",
            "dialogue",
            "--slug",
            "parley",
            "--id",
            DIALOGUE_ID,
            "--entry-id",
            DIALOGUE_ID,
        ],
        vec!["new", "creature", "--slug", "Bad", "--id", CREATURE_ID],
        vec!["new", "creature", "--slug", "gadwall", "--id", "short"],
        vec![
            "new",
            "creature",
            "--slug",
            "gadwall",
            "--id",
            "80000000000000000000000000",
        ],
    ];
    for argv in &cases {
        let output = run(argv);
        assert_eq!(output.status.code(), Some(2), "argv {argv:?}");
        assert!(output.stdout.is_empty(), "argv {argv:?}");
        assert_eq!(output.stderr, NEW_USAGE.as_bytes(), "argv {argv:?}");
    }
}

#[test]
fn new_rejects_dash_paths_without_relative_prefix() {
    // A path-shaped positional starting with `-` is a flag, hence usage;
    // the relative prefix spelling parses as a positional instead (then
    // fails as an unsupported type, still with the one usage line).
    let output = run(&["new", "-gadwall", "--slug", "gadwall", "--id", CREATURE_ID]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stderr, NEW_USAGE.as_bytes());
    let output = run(&["new", "./gadwall", "--slug", "gadwall", "--id", CREATURE_ID]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stderr, NEW_USAGE.as_bytes());
}

#[test]
fn new_creature_scaffold_matches_independently_specified_bytes() {
    let output = run(&["new", "creature", "--slug", "gadwall", "--id", CREATURE_ID]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, EXPECTED_CREATURE.as_bytes());
    // Repeated calls with identical operands produce identical bytes.
    let again = run(&["new", "creature", "--slug", "gadwall", "--id", CREATURE_ID]);
    assert_eq!(again.stdout, output.stdout);
    // The current envelope tag is present; notes are absent.
    let text = String::from_utf8(output.stdout).expect("utf-8");
    assert!(text.contains("\"schema\": \"crpg.creature/1\""));
    assert!(!text.contains("_note"));
}

#[test]
fn new_item_scaffold_matches_independently_specified_bytes() {
    let output = run(&["new", "item", "--id", ITEM_ID, "--slug", "baton"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, EXPECTED_ITEM.as_bytes());
    let text = String::from_utf8(output.stdout).expect("utf-8");
    assert!(text.contains("\"schema\": \"crpg.item/2\""));
    assert!(!text.contains("_note"));
}

#[test]
fn new_dialogue_scaffold_matches_independently_specified_bytes() {
    let output = run(&[
        "new",
        "dialogue",
        "--slug",
        "parley",
        "--entry-id",
        DIALOGUE_ENTRY_ID,
        "--id",
        DIALOGUE_ID,
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, EXPECTED_DIALOGUE.as_bytes());
}

#[test]
fn new_quest_scaffold_matches_independently_specified_bytes() {
    let output = run(&[
        "new",
        "quest",
        "--slug",
        "courier",
        "--id",
        QUEST_ID,
        "--entry-id",
        QUEST_ENTRY_ID,
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, EXPECTED_QUEST.as_bytes());
}

/// Installs one scaffold document plus its locale entries in a fresh private
/// copy of the clean fixture, then requires `validate --json` to accept it.
fn validate_scaffold(name: &str, relative_path: &str, document: &[u8], locale_keys: &[&str]) {
    let root = temp_root(name);
    let trial = root.join("trial");
    copy_dir(&valid_root(), &trial);
    let target = trial.join(relative_path);
    std::fs::create_dir_all(target.parent().expect("parent")).expect("dirs must create");
    std::fs::write(&target, document).expect("scaffold must install");
    let locale_path = trial.join("locale").join("en.json");
    let mut locale: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&locale_path).expect("locale must read"))
            .expect("locale must parse");
    let strings = locale
        .get_mut("strings")
        .and_then(|strings| strings.as_object_mut())
        .expect("locale has a strings table");
    for key in locale_keys {
        strings.insert(
            (*key).to_owned(),
            serde_json::Value::String((*key).to_owned()),
        );
    }
    // Canonical locale bytes preserve the data writer's shape; the file is
    // rewritten through parsed JSON with sorted keys and one final LF, which
    // is canonical for this string-only table.
    let mut locale_bytes = serde_json::to_string_pretty(&locale).expect("locale serializes");
    locale_bytes.push('\n');
    std::fs::write(&locale_path, locale_bytes).expect("locale must update");
    let output = run(&[
        "validate",
        trial.to_str().expect("temp root is unicode"),
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(0), "trial {name}");
    assert_eq!(output.stdout, b"[]\n", "trial {name}");
    assert!(output.stderr.is_empty(), "trial {name}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn new_creature_validates_in_a_private_campaign() {
    validate_scaffold(
        "creature",
        "creatures/scaffold-gadwall.json",
        EXPECTED_CREATURE.as_bytes(),
        &["creature.gadwall.name"],
    );
}

#[test]
fn new_item_validates_in_a_private_campaign() {
    validate_scaffold(
        "item",
        "items/scaffold-baton.json",
        EXPECTED_ITEM.as_bytes(),
        &["item.baton.name"],
    );
}

#[test]
fn new_dialogue_validates_in_a_private_campaign() {
    validate_scaffold(
        "dialogue",
        "dialogue/scaffold-parley.json",
        EXPECTED_DIALOGUE.as_bytes(),
        &["dialogue.parley.name"],
    );
}

#[test]
fn new_quest_validates_in_a_private_campaign() {
    validate_scaffold(
        "quest",
        "quests/scaffold-courier.json",
        EXPECTED_QUEST.as_bytes(),
        &["quest.courier.name", "quest.courier.state.done"],
    );
}

/// The classified dialogue directory is singular `dialogue/` (data owns the
/// `family(p, "dialogue/")` mapping); a `dialogues/` (plural) path is ignored
/// content, so installing there validates vacuously. This negative control
/// pins the classified path: malformed bytes at `dialogue/` must fail,
/// proving the dialogue scaffold and rerun cases above genuinely exercise
/// their documents instead of passing trivially.
#[test]
fn dialogue_install_path_is_classified_and_exercised() {
    let root = temp_root("dialogue-path-proof");
    let trial = root.join("campaign");
    copy_dir(&valid_root(), &trial);
    let target = trial.join("dialogue").join("bad.json");
    std::fs::create_dir_all(target.parent().expect("parent")).expect("dirs must create");
    std::fs::write(&target, b"{ not json").expect("bad document must install");
    let output = run(&["validate", trial.to_str().expect("unicode"), "--json"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "malformed dialogue must fail"
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("dialogue/bad.json"),
        "failure must name the classified path"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn new_identities_are_valid_and_distinct() {
    for id in [
        CREATURE_ID,
        ITEM_ID,
        DIALOGUE_ID,
        DIALOGUE_ENTRY_ID,
        QUEST_ID,
        QUEST_ENTRY_ID,
    ] {
        assert_eq!(id.len(), 26, "{id}");
        assert!(id.bytes().all(|b| b.is_ascii_alphanumeric()));
    }
    let mut sorted = vec![
        CREATURE_ID,
        ITEM_ID,
        DIALOGUE_ID,
        DIALOGUE_ENTRY_ID,
        QUEST_ID,
        QUEST_ENTRY_ID,
    ];
    sorted.sort_unstable();
    let before = sorted.len();
    sorted.dedup();
    assert_eq!(sorted.len(), before, "scaffold identities must be distinct");
}

const SCHEMA_STEMS: [&str; 17] = [
    "campaign",
    "world",
    "area",
    "creature",
    "item",
    "dialogue",
    "quest",
    "faction",
    "graph",
    "placements",
    "triggers",
    "locale",
    "variables",
    "campaign-lock",
    "assets-lock",
    "placement",
    "action-signature",
];

#[test]
fn schema_usage_matrix_reports_one_line_with_exit_2() {
    let cases: Vec<Vec<&str>> = vec![
        vec!["schema"],
        vec!["schema", "campaign", "world"],
        vec!["schema", "golem"],
        vec!["schema", "Campaign"],
        vec!["schema", "--check", "campaign"],
        vec!["schema", "campaign", "--check"],
        vec!["schema", "--"],
    ];
    for argv in &cases {
        let output = run(argv);
        assert_eq!(output.status.code(), Some(2), "argv {argv:?}");
        assert!(output.stdout.is_empty(), "argv {argv:?}");
        assert_eq!(output.stderr, SCHEMA_USAGE.as_bytes(), "argv {argv:?}");
    }
}

#[test]
fn schema_covers_all_stems_with_checked_in_bytes() {
    let schemas_dir = repo_root().join("schemas");
    for stem in SCHEMA_STEMS {
        let output = run(&["schema", stem]);
        assert_eq!(output.status.code(), Some(0), "stem {stem}");
        assert!(output.stderr.is_empty(), "stem {stem}");
        let checked_in = std::fs::read(schemas_dir.join(format!("{stem}.schema.json")))
            .unwrap_or_else(|_| panic!("checked-in schema for {stem} must exist"));
        assert_eq!(
            output.stdout, checked_in,
            "stem {stem} must match read-only bytes"
        );
        assert_eq!(output.stdout.last(), Some(&b'\n'), "stem {stem}");
        // Repeated output is identical.
        let again = run(&["schema", stem]);
        assert_eq!(again.stdout, output.stdout, "stem {stem}");
    }
}

#[test]
fn schema_runs_identically_outside_the_repository() {
    let root = temp_root("out-of-repo");
    for stem in ["creature", "campaign-lock", "action-signature"] {
        let output = Command::new(crpgc())
            .args(["schema", stem])
            .current_dir(&root)
            .output()
            .expect("spawning crpgc must succeed");
        assert_eq!(output.status.code(), Some(0), "stem {stem}");
        let checked_in = std::fs::read(
            repo_root()
                .join("schemas")
                .join(format!("{stem}.schema.json")),
        )
        .expect("checked-in schema must exist");
        assert_eq!(output.stdout, checked_in, "stem {stem}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// The four preserved literal schema-only LLM acceptance responses
/// (`tasks/T013.md` transcripts): output filename, locale keys derived from
/// the typed document's locale-key fields, and the installed relative path.
/// The live LLM call is recorded acceptance, not a network-dependent CI
/// test: this reruns their process validation on every native instead.
const LLM_TRIALS: [(&str, &str, &[u8]); 4] = [
    (
        "creature",
        "creatures/llm-trial-creature.json",
        b"{\"id\":\"00000000000000000000000021\",\"inventory\":[],\"name\":\"llm.creature.name\",\"schema\":\"crpg.creature/1\",\"slug\":\"llm-creature\",\"stats\":{},\"tags\":[],\"faction\":null}",
    ),
    (
        "item",
        "items/llm-trial-item.json",
        b"{\"id\":\"00000000000000000000000022\",\"name\":\"llm.item.name\",\"schema\":\"crpg.item/2\",\"slug\":\"llm-item\",\"stats\":{},\"tags\":[]}",
    ),
    (
        "dialogue",
        "dialogue/llm-trial-dialogue.json",
        b"{\"entry\":\"00000000000000000000000024\",\"id\":\"00000000000000000000000023\",\"name\":\"llm.dialogue.name\",\"nodes\":[{\"body\":{\"kind\":\"end\"},\"id\":\"00000000000000000000000024\"}],\"schema\":\"crpg.dialogue/1\",\"slug\":\"llm-dialogue\"}",
    ),
    (
        "quest",
        "quests/llm-trial-quest.json",
        b"{\"entry\":\"00000000000000000000000026\",\"id\":\"00000000000000000000000025\",\"name\":\"llm.quest.name\",\"schema\":\"crpg.quest/1\",\"slug\":\"llm-quest\",\"states\":[{\"id\":\"00000000000000000000000026\",\"name\":\"llm.quest.state.done\",\"on_enter\":[],\"terminal\":true,\"transitions\":[]}]}",
    ),
];

const LLM_TRIALS_R2: [(&str, &str, &[u8]); 4] = [
    (
        "creature",
        "creatures/llm-trial-r2-creature.json",
        b"{\"schema\":\"crpg.creature/1\",\"id\":\"00000000000000000000000000\",\"slug\":\"a\",\"name\":\"a\",\"stats\":{},\"tags\":[],\"faction\":null,\"inventory\":[]}",
    ),
    (
        "item",
        "items/llm-trial-r2-item.json",
        b"{\"id\":\"01ARZ3NDEKTSV4RRFFQ69G5FAV\",\"name\":\"item\",\"schema\":\"crpg.item/2\",\"slug\":\"item\",\"stats\":{},\"tags\":[]}",
    ),
    (
        "dialogue",
        "dialogue/llm-trial-r2-dialogue.json",
        b"{\n\"schema\": \"crpg.dialogue/1\",\n\"id\": \"01JAV8Q3K7ZB4N6X2M9PQRST00\",\n\"slug\": \"greeting\",\n\"name\": \"dialogue.greeting.name\",\n\"entry\": \"01JAV8Q3K7ZB4N6X2M9PQRST01\",\n\"nodes\": [\n{\n\"id\": \"01JAV8Q3K7ZB4N6X2M9PQRST01\",\n\"body\": {\n\"kind\": \"end\"\n}\n}\n]\n}",
    ),
    (
        "quest",
        "quests/llm-trial-r2-quest.json",
        b"{\"schema\":\"crpg.quest/1\",\"id\":\"01ARZ3NDEKTSV4RRFFQ69G5FAV\",\"slug\":\"q\",\"name\":\"q\",\"entry\":\"01ARZ3NDEKTSV4RRFFQ69G5FAW\",\"states\":[{\"id\":\"01ARZ3NDEKTSV4RRFFQ69G5FAW\",\"name\":\"s\",\"terminal\":true,\"on_enter\":[],\"transitions\":[]}]}",
    ),
];

/// Derives the locale keys the context adapter must provision from the
/// permitted typed document fields only: the document `name`, every quest
/// state `name`, and every dialogue `text_key` present in an npc_line or
/// player_choice body. Decoded via `crpg_data::read_document`, never by
/// probing JSON field names. This test support is not a production
/// reference walker: it reads only these known typed positions.
fn llm_trial_locale_keys(document: &crpg_data::Document) -> Vec<String> {
    let mut keys = Vec::new();
    match document {
        crpg_data::Document::Creature(creature) => {
            keys.push(creature.name.clone());
        }
        crpg_data::Document::Item(item) => {
            keys.push(item.name.clone());
        }
        crpg_data::Document::Dialogue(dialogue) => {
            keys.push(dialogue.name.clone());
            for node in &dialogue.nodes {
                match &node.body {
                    crpg_data::DialogueBody::NpcLine { text_key, .. } => {
                        keys.push(text_key.clone());
                    }
                    crpg_data::DialogueBody::PlayerChoice { text_key, .. } => {
                        keys.push(text_key.clone());
                    }
                    crpg_data::DialogueBody::Jump { .. }
                    | crpg_data::DialogueBody::Link { .. }
                    | crpg_data::DialogueBody::End => {}
                }
            }
        }
        crpg_data::Document::Quest(quest) => {
            keys.push(quest.name.clone());
            for state in &quest.states {
                keys.push(state.name.clone());
            }
        }
        other => panic!(
            "llm trial must be creature/item/dialogue/quest, got {}",
            match other {
                crpg_data::Document::Campaign(_) => "campaign",
                crpg_data::Document::World(_) => "world",
                crpg_data::Document::Area(_) => "area",
                crpg_data::Document::Faction(_) => "faction",
                crpg_data::Document::Graph(_) => "graph",
                crpg_data::Document::Placements(_) => "placements",
                crpg_data::Document::Triggers(_) => "triggers",
                crpg_data::Document::Locale(_) => "locale",
                crpg_data::Document::Variables(_) => "variables",
                _ => "other",
            }
        ),
    }
    keys.sort();
    keys.dedup();
    keys
}

#[test]
fn llm_trial_responses_validate_in_private_campaigns() {
    for (set, trials) in [("llm-trials", LLM_TRIALS), ("llm-trials-r2", LLM_TRIALS_R2)] {
        let inputs = cli_dir().join("tests").join("inputs").join(set);
        for (stem, relative, expected_bytes) in trials {
            // The preserved bytes are the unedited first-attempt trial
            // responses; the trial root is a fresh private fixture copy.
            let document_bytes =
                std::fs::read(inputs.join(format!("{stem}.json"))).expect("trial input must exist");
            assert_eq!(
                document_bytes, expected_bytes,
                "{set} {stem}: preserved response bytes must match the reviewed transcript pin"
            );
            // Typed decode enforces the protocol: invalid JSON, wrong envelope,
            // or unsupported external references fail here rather than causing a
            // retry or hidden adjustment.
            let document: crpg_data::Document =
                crpg_data::read_document(&document_bytes).expect("trial input must decode typed");
            let root = temp_root(&format!("{set}-{stem}"));
            let trial = root.join("campaign");
            copy_dir(&valid_root(), &trial);
            let target = trial.join(relative);
            std::fs::create_dir_all(target.parent().expect("parent")).expect("dirs must create");
            // Install the original response bytes unchanged.
            std::fs::write(&target, &document_bytes).expect("trial document must install");
            assert_eq!(
                std::fs::read(&target).expect("trial document must reread"),
                document_bytes,
                "llm trial {stem}: installed bytes must equal the preserved response"
            );
            let locale_path = trial.join("locale").join("en.json");
            let mut locale: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&locale_path).expect("locale must read"))
                    .expect("locale must parse");
            let strings = locale
                .get_mut("strings")
                .and_then(|strings| strings.as_object_mut())
                .expect("locale has a strings table");
            // Add-only locale provisioning: values equal keys, never silently
            // overwriting an existing entry (a collision fails the attempt).
            for key in llm_trial_locale_keys(&document) {
                assert!(
                    !strings.contains_key(&key),
                    "llm trial {stem}: locale key collision for {key:?}"
                );
                strings.insert(key.clone(), serde_json::Value::String(key));
            }
            let mut locale_bytes =
                serde_json::to_string_pretty(&locale).expect("locale serializes");
            locale_bytes.push('\n');
            std::fs::write(&locale_path, locale_bytes).expect("locale must update");
            let output = run(&["validate", trial.to_str().expect("unicode"), "--json"]);
            assert_eq!(output.status.code(), Some(0), "llm trial {stem}");
            assert_eq!(output.stdout, b"[]\n", "llm trial {stem}");
            assert!(output.stderr.is_empty(), "llm trial {stem}");
            let _ = std::fs::remove_dir_all(&root);
        }
    }
}
