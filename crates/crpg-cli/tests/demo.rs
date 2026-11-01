//! Black-box acceptance for `crpgc demo` (T145).
//!
//! Drives the built binary (`env!("CARGO_BIN_EXE_crpgc")`) as a real process
//! over the checked-in `combat_basic` and `combat_srd` fixtures, asserting
//! exit codes and stdout bytes, never demo internals. Final healths are
//! cross-checked by an independent route: this file loads the campaign
//! through production `crpg-data`, assembles the encounter spec itself, and
//! plays the same replay through public `crpg-sim` operations. Checked-in
//! inputs stay read-only; the one mutation test uses a private temp copy.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crpg_core::Ulid;
use crpg_data::{Document, LoadedCampaign, SourcePath};
use crpg_sim::{CombatAction, EncounterSpec, PlacementAndArea, World};
use crpg_testkit::Replay;

/// Authored identities that must never appear when documents name them.
const RAW_IDS: &[&str] = &[
    "0000000000000000000000000B",
    "0000000000000000000000000E",
    "0000000000000000000000000F",
    "0000000000000000000000000G",
    "0000000000000000000000000H",
    "0000000000000000000000000K",
    "0000000000000000000000000M",
    "0000000000000000000000000N",
    "00000000000000000000000012",
    "00000000000000000000000013",
    "00000000000000000000000014",
    "00000000000000000000000015",
    "00000000000000000000000017",
    "00000000000000000000000018",
    "0000000000000000000000001C",
];

/// The built `crpgc` binary path, provided by cargo for integration tests.
fn crpgc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_crpgc"))
}

/// Runs `crpgc` with Unicode `args`.
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

/// The repository root, two levels above the crate.
fn repo_root() -> PathBuf {
    cli_dir().join("..").join("..")
}

/// The checked-in portable combat replay fixture (read-only).
fn combat_replay() -> PathBuf {
    cli_dir()
        .join("..")
        .join("crpg-testkit")
        .join("fixtures")
        .join("combat_basic.replay")
}

/// The checked-in second-ruleset replay fixture (read-only).
fn srd_replay() -> PathBuf {
    cli_dir()
        .join("..")
        .join("crpg-testkit")
        .join("fixtures")
        .join("combat_srd.replay")
}

/// One checked-in fixture campaign root (read-only).
fn campaign(slug: &str) -> PathBuf {
    repo_root().join("campaigns").join("fixtures").join(slug)
}

/// Runs `demo` over one fixture and returns stdout after asserting success.
fn demo(campaign_slug: &str, replay: &Path) -> String {
    let out = run(&[
        "demo",
        replay.to_str().expect("utf-8"),
        "--campaign",
        campaign(campaign_slug).to_str().expect("utf-8"),
    ]);
    assert_eq!(out.status.code(), Some(0), "demo must exit 0: {out:?}");
    assert!(
        out.stderr.is_empty(),
        "success prints no stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("stdout utf-8")
}

/// Every fixture the behaviour tests run over.
fn fixtures() -> [(&'static str, PathBuf); 2] {
    [
        ("combat_basic", combat_replay()),
        ("combat_srd", srd_replay()),
    ]
}

/// A fresh unique temp directory for one test.
fn temp_root(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("crpg-demo-cli-{}-{name}", std::process::id()));
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

/// Collects a campaign tree through the production path classifier.
fn collect(root: &Path) -> BTreeMap<SourcePath, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<SourcePath, Vec<u8>>) {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .expect("campaign directory must list")
            .map(|entry| entry.expect("campaign entry").path())
            .collect();
        entries.sort();
        for entry in &entries {
            if entry.is_dir() {
                walk(entry, root, out);
            } else {
                let relative = entry
                    .strip_prefix(root)
                    .expect("entry under root")
                    .to_string_lossy()
                    .replace('\\', "/");
                match crpg_data::campaign_document_path(&relative) {
                    Ok(Some(path)) => {
                        let bytes = std::fs::read(entry).expect("campaign file must read");
                        out.insert(path, bytes);
                    }
                    Ok(None) => {}
                    Err(error) => panic!("classifier rejected {relative}: {error:?}"),
                }
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut files);
    files
}

/// Loads a fixture campaign through production `crpg-data`.
fn load(root: &Path) -> LoadedCampaign {
    let files = collect(root);
    let engine = "0.1.0".parse().expect("engine version parses");
    let loaded = crpg_data::load_campaign(&files, &engine).expect("campaign loads");
    assert!(
        crpg_data::validate(&loaded).is_empty(),
        "fixture campaign must validate clean"
    );
    loaded
}

/// Parses one required ULID field from a replay payload.
fn payload_ulid(payload: &serde_json::Value, field: &str) -> Ulid {
    payload
        .get(field)
        .and_then(|value| value.as_str())
        .unwrap_or_else(|| panic!("payload has {field}"))
        .parse()
        .unwrap_or_else(|_| panic!("payload {field} is a ULID"))
}

/// The independent sim route's observed end state.
struct SimFight {
    /// Authored participant placements in encounter order.
    participants: Vec<Ulid>,
    /// Final `(health, max_health, dead)` by placement.
    final_health: BTreeMap<Ulid, (u32, u32, bool)>,
    /// Completed rounds at the end of playback.
    round: u32,
}

/// Replays the same inputs through public `crpg-sim` API only.
///
/// This is deliberately a second route: it assembles the encounter spec from
/// the loaded documents itself, binds placements by authored order, and
/// applies the payloads directly. It never reads the demo's stdout.
fn sim_fight(root: &Path, replay: &Replay) -> SimFight {
    let loaded = load(root);
    let encounter_id = replay
        .inputs
        .iter()
        .find_map(|input| {
            if input.payload.get("op").and_then(|value| value.as_str()) == Some("init") {
                Some(payload_ulid(&input.payload, "encounter"))
            } else {
                None
            }
        })
        .expect("replay initializes an encounter");
    let encounter = loaded
        .documents
        .values()
        .find_map(|document| match document {
            Document::Encounter(value) if value.id == encounter_id => Some(value),
            _ => None,
        })
        .expect("encounter document exists");
    let ruleset = loaded
        .documents
        .values()
        .find_map(|document| match document {
            Document::Ruleset(value) if value.id == encounter.ruleset => Some(value),
            _ => None,
        })
        .expect("ruleset document exists");
    let mut abilities = BTreeMap::new();
    let mut outcome_tables = BTreeMap::new();
    let mut placements = BTreeMap::new();
    let mut creatures = BTreeMap::new();
    let mut effects = BTreeMap::new();
    for document in loaded.documents.values() {
        match document {
            Document::Ability(value) => {
                abilities.insert(value.id, value);
            }
            Document::OutcomeTable(value) => {
                outcome_tables.insert(value.id, value);
            }
            Document::Placements(value) => {
                for placement in &value.placements {
                    placements.insert(
                        placement.id,
                        PlacementAndArea {
                            placement,
                            area: value.area,
                        },
                    );
                }
            }
            Document::Creature(value) => {
                creatures.insert(value.id, value);
            }
            Document::Effect(value) => {
                effects.insert(value.id, value);
            }
            _ => {}
        }
    }
    let spec = EncounterSpec {
        encounter,
        ruleset,
        abilities: &abilities,
        outcome_tables: &outcome_tables,
        placements: &placements,
        creatures: &creatures,
        effects: &effects,
    };
    let mut world = World::new(replay.seed);
    crpg_sim::start_encounter(&mut world, &spec).expect("encounter starts");
    let participants: Vec<Ulid> = world
        .combatants()
        .iter()
        .map(|(_, combatant)| combatant.placement())
        .collect();
    let entities: BTreeMap<Ulid, crpg_core::EntityId> = world
        .combatants()
        .iter()
        .map(|(entity, combatant)| (combatant.placement(), entity))
        .collect();
    let mut cursor = 0usize;
    for _ in 0..replay.total_ticks {
        while cursor < replay.inputs.len() && replay.inputs[cursor].tick == world.tick().get() {
            let payload = &replay.inputs[cursor].payload;
            match payload.get("op").and_then(|value| value.as_str()) {
                Some("init") => {}
                Some("attack") => {
                    let actor = entities[&payload_ulid(payload, "actor")];
                    let target = entities[&payload_ulid(payload, "target")];
                    let ability = payload_ulid(payload, "ability");
                    crpg_sim::perform_action(
                        &mut world,
                        &CombatAction::UseAbility {
                            actor,
                            ability,
                            target,
                        },
                    )
                    .expect("attack applies");
                }
                Some("end") => {
                    let actor = entities[&payload_ulid(payload, "actor")];
                    crpg_sim::perform_action(&mut world, &CombatAction::EndTurn { actor })
                        .expect("end applies");
                }
                other => panic!("unexpected replay op {other:?}"),
            }
            cursor += 1;
        }
        crpg_sim::tick(&mut world);
    }
    let final_health = world
        .combatants()
        .iter()
        .map(|(_, combatant)| {
            (
                combatant.placement(),
                (combatant.health(), combatant.max_health(), combatant.dead()),
            )
        })
        .collect();
    let round = world.combat().map_or(0, |combat| combat.round);
    SimFight {
        participants,
        final_health,
        round,
    }
}

/// The header's combatant names in printed order.
fn header_names(stdout: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_block = false;
    for line in stdout.lines() {
        if line == "Combatants:" {
            in_block = true;
            continue;
        }
        if in_block {
            let Some(rest) = line.strip_prefix("  ") else {
                break;
            };
            let name = rest.split(" | side ").next().expect("name field");
            names.push(name.to_string());
        }
    }
    names
}

/// The header's starting `(health, max)` per printed combatant, in order.
fn header_health(stdout: &str) -> Vec<(u32, u32)> {
    let mut health = Vec::new();
    let mut in_block = false;
    for line in stdout.lines() {
        if line == "Combatants:" {
            in_block = true;
            continue;
        }
        if in_block {
            let Some(rest) = line.strip_prefix("  ") else {
                break;
            };
            let tail = rest.rsplit_once(" | ").expect("health field").1;
            let numbers = tail.strip_suffix(" hp").expect("hp suffix");
            let (current, max) = numbers.split_once('/').expect("current/max");
            health.push((
                current.parse().expect("current health"),
                max.parse().expect("max health"),
            ));
        }
    }
    health
}

/// The summary's `Final health` entries by combatant name.
fn summary_health(stdout: &str) -> BTreeMap<String, u32> {
    let mut health = BTreeMap::new();
    let mut in_block = false;
    for line in stdout.lines() {
        if line == "Final health:" {
            in_block = true;
            continue;
        }
        if in_block {
            let Some(rest) = line.strip_prefix("  ") else {
                break;
            };
            let rest = rest.strip_suffix(" (dead)").unwrap_or(rest);
            let rest = rest.strip_suffix(" hp").expect("hp suffix");
            let (name, numbers) = rest.rsplit_once(' ').expect("name and health");
            let (current, _) = numbers.split_once('/').expect("current/max");
            health.insert(name.to_string(), current.parse().expect("health"));
        }
    }
    health
}

/// One summary line's value, by prefix.
fn summary_line<'a>(stdout: &'a str, prefix: &str) -> &'a str {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("summary line {prefix:?} present: {stdout}"))
}

/// Behaviour 1: the header names the campaign and every combatant with its
/// authored name, side, and starting health.
#[test]
fn header_names_campaign_and_combatants() {
    for (slug, replay) in fixtures() {
        let stdout = demo(slug, &replay);
        let expected_campaign = match slug {
            "combat_basic" => "Combat Basic",
            _ => "Combat Srd",
        };
        assert!(
            stdout.contains(&format!("== Combat demo: {expected_campaign} ==\n")),
            "{slug} header names the campaign: {stdout}"
        );
        let sim = sim_fight(
            &campaign(slug),
            &crpg_testkit::read_replay(&replay).expect("replay"),
        );
        let names = header_names(&stdout);
        assert_eq!(
            names.len(),
            sim.participants.len(),
            "{slug} header lists every combatant"
        );
        let health = header_health(&stdout);
        assert_eq!(health.len(), sim.participants.len());
        for (index, placement) in sim.participants.iter().enumerate() {
            let (_, max, _) = sim.final_health[placement];
            assert!(
                stdout.contains(&format!("  {} | side ", names[index])),
                "{slug} header names {}: {stdout}",
                names[index]
            );
            assert_eq!(
                health[index],
                (max, max),
                "{slug} {} starts at full health",
                names[index]
            );
        }
    }
}

/// Behaviour 2 and its test clause: one action block per attack intent, a
/// line for every round and turn change, exactly one death line per death,
/// and a summary.
#[test]
fn action_blocks_cover_every_attack_intent() {
    for (slug, replay_path) in fixtures() {
        let stdout = demo(slug, &replay_path);
        let replay = crpg_testkit::read_replay(&replay_path).expect("replay reads");
        let sim = sim_fight(&campaign(slug), &replay);
        let names: BTreeMap<Ulid, String> = sim
            .participants
            .iter()
            .copied()
            .zip(header_names(&stdout))
            .collect();
        // Every attack intent prints one block with its actor and target.
        let lines: Vec<&str> = stdout.lines().collect();
        let blocks: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.starts_with('[') && line.contains(" -> "))
            .map(|(index, _)| index)
            .collect();
        let attacks: Vec<&serde_json::Value> = replay
            .inputs
            .iter()
            .map(|input| &input.payload)
            .filter(|payload| payload.get("op").and_then(|value| value.as_str()) == Some("attack"))
            .collect();
        assert_eq!(
            blocks.len(),
            attacks.len(),
            "{slug} prints one block per attack intent: {stdout}"
        );
        for (index, payload) in blocks.iter().zip(&attacks) {
            let block = lines[*index];
            let actor = &names[&payload_ulid(payload, "actor")];
            let target = &names[&payload_ulid(payload, "target")];
            assert!(
                block.contains(&format!("] {actor} | ")),
                "{slug} block names actor {actor}: {block}"
            );
            assert!(
                block.ends_with(&format!(" -> {target}")),
                "{slug} block names target {target}: {block}"
            );
            let detail = lines.get(*index + 1).copied().unwrap_or_default();
            assert!(
                detail.contains("dice ") || detail.contains("no immediate result"),
                "{slug} block prints the sim result: {detail}"
            );
            if detail.contains("dice ") {
                let damage = lines.get(*index + 2).copied().unwrap_or_default();
                assert!(
                    damage.trim_start().starts_with("damage "),
                    "{slug} block prints damage: {damage}"
                );
                let health = lines.get(*index + 3).copied().unwrap_or_default();
                assert!(
                    health.trim_start().contains(" hp"),
                    "{slug} block prints target health after: {health}"
                );
            }
        }
        // Round and turn changes are announced.
        assert!(stdout.contains("-- Round "), "{slug} prints round changes");
        assert!(stdout.contains("\nTurn: "), "{slug} prints turn changes");
        // Exactly one death line per combatant that dies, none for survivors.
        for (placement, (_, _, dead)) in &sim.final_health {
            let name = &names[placement];
            let deaths = stdout.matches(&format!("{name} dies.")).count();
            assert_eq!(
                deaths,
                usize::from(*dead),
                "{slug} death line count for {name}: {stdout}"
            );
        }
        assert!(
            stdout.contains("== Summary ==\n"),
            "{slug} prints a summary"
        );
    }
}

/// Behaviour 2 (health after) and the test's independent-route clause: the
/// summary's final health equals what `crpg-sim` reaches on the same replay.
#[test]
fn summary_health_matches_independent_sim_run() {
    for (slug, replay_path) in fixtures() {
        let stdout = demo(slug, &replay_path);
        let replay = crpg_testkit::read_replay(&replay_path).expect("replay reads");
        let sim = sim_fight(&campaign(slug), &replay);
        let names: BTreeMap<Ulid, String> = sim
            .participants
            .iter()
            .copied()
            .zip(header_names(&stdout))
            .collect();
        let reported = summary_health(&stdout);
        assert_eq!(
            reported.len(),
            sim.final_health.len(),
            "{slug} summary lists every combatant: {stdout}"
        );
        for (placement, (health, _max, _dead)) in &sim.final_health {
            let name = &names[placement];
            assert_eq!(
                reported.get(name),
                Some(health),
                "{slug} summary health for {name} matches crpg-sim: {stdout}"
            );
        }
    }
}

/// Behaviour 3: the summary reports rounds played, survivors, and the side
/// that won.
#[test]
fn summary_reports_rounds_survivors_and_winner() {
    for (slug, replay_path) in fixtures() {
        let stdout = demo(slug, &replay_path);
        let replay = crpg_testkit::read_replay(&replay_path).expect("replay reads");
        let sim = sim_fight(&campaign(slug), &replay);
        let names: BTreeMap<Ulid, String> = sim
            .participants
            .iter()
            .copied()
            .zip(header_names(&stdout))
            .collect();
        // The demo numbers rounds from one; the sim counts completed rounds.
        assert_eq!(
            summary_line(&stdout, "Rounds played: "),
            (sim.round + 1).to_string(),
            "{slug} rounds played: {stdout}"
        );
        let survivors: Vec<String> = sim
            .participants
            .iter()
            .filter(|placement| !sim.final_health[placement].2)
            .map(|placement| names[placement].clone())
            .collect();
        let expected = if survivors.is_empty() {
            "none".to_string()
        } else {
            survivors.join(", ")
        };
        assert_eq!(
            summary_line(&stdout, "Survivors: "),
            expected,
            "{slug} survivors: {stdout}"
        );
        // One surviving side wins; anything else is a draw.
        let sides: BTreeMap<String, bool> = stdout
            .lines()
            .filter_map(|line| {
                let rest = line.strip_prefix("  ")?;
                let (name, tail) = rest.split_once(" | side ")?;
                let (side, _) = tail.split_once(" | ")?;
                let dead = sim
                    .participants
                    .iter()
                    .find(|placement| names[placement] == name)
                    .is_some_and(|placement| sim.final_health[placement].2);
                Some((side.to_string(), !dead))
            })
            .fold(BTreeMap::new(), |mut sides, (side, alive)| {
                let entry = sides.entry(side).or_insert(false);
                *entry |= alive;
                sides
            });
        let winners: Vec<String> = sides
            .iter()
            .filter(|(_, alive)| **alive)
            .map(|(side, _)| side.clone())
            .collect();
        let expected_winner = match winners.as_slice() {
            [only] => only.clone(),
            _ => "draw".to_string(),
        };
        assert_eq!(
            summary_line(&stdout, "Winner: "),
            expected_winner,
            "{slug} winner: {stdout}"
        );
    }
}

/// Behaviour 4: names come from campaign documents (the locale table
/// resolves their keys); a raw identity never appears while a document names
/// the combatant.
#[test]
fn names_come_from_documents_not_raw_ids() {
    for (slug, replay) in fixtures() {
        let stdout = demo(slug, &replay);
        assert!(stdout.contains("Hero"), "{slug} names the hero: {stdout}");
        assert!(
            stdout.contains("Goblin"),
            "{slug} names the goblin: {stdout}"
        );
        for raw in RAW_IDS {
            assert!(
                !stdout.contains(raw),
                "{slug} prints no raw identity {raw}: {stdout}"
            );
        }
    }
    // Display names come from the locale document, not from the authored
    // name key and not from any hard-coded text: editing the table changes
    // the printed name, and the raw identity still never appears.
    let copy = temp_root("renamed-locale");
    copy_dir(&campaign("combat_basic"), &copy);
    let locale_path = copy.join("locale").join("en.json");
    let locale = std::fs::read_to_string(&locale_path).expect("locale reads");
    let renamed = locale.replace(
        "\"fixture.hero\": \"Hero\"",
        "\"fixture.hero\": \"Sir Hero\"",
    );
    assert_ne!(renamed, locale, "the locale edit must land");
    std::fs::write(&locale_path, renamed).expect("locale writes");
    let out = run(&[
        "demo",
        combat_replay().to_str().expect("utf-8"),
        "--campaign",
        copy.to_str().expect("utf-8"),
    ]);
    assert_eq!(out.status.code(), Some(0), "renamed demo: {out:?}");
    let stdout = String::from_utf8(out.stdout).expect("stdout utf-8");
    assert!(
        stdout.contains("Sir Hero"),
        "display name comes from the locale document: {stdout}"
    );
    assert!(
        !stdout.contains("fixture.hero |"),
        "the raw name key is not printed when the locale resolves it: {stdout}"
    );
    for raw in RAW_IDS {
        assert!(
            !stdout.contains(raw),
            "renamed run still prints no raw identity {raw}: {stdout}"
        );
    }
    let _ = std::fs::remove_dir_all(&copy);
}

/// Two runs over the same inputs give byte-identical stdout.
#[test]
fn two_runs_are_byte_identical() {
    for (slug, replay) in fixtures() {
        let first = demo(slug, &replay);
        let second = demo(slug, &replay);
        assert_eq!(first, second, "{slug} output must be deterministic");
    }
}

/// Usage errors exit 2 with one stderr line and empty stdout.
#[test]
fn usage_errors_exit_2() {
    let replay = combat_replay();
    let replay = replay.to_str().expect("utf-8");
    let campaign = campaign("combat_basic");
    let campaign = campaign.to_str().expect("utf-8");
    let cases: Vec<Vec<&str>> = vec![
        // Missing --campaign entirely.
        vec!["demo", replay],
        // Unknown flag.
        vec!["demo", replay, "--campaign", campaign, "--bogus"],
        // Non-number delay.
        vec!["demo", replay, "--campaign", campaign, "--delay-ms", "soon"],
        vec!["demo", replay, "--campaign", campaign, "--delay-ms", "-5"],
        // Missing values and extra positionals.
        vec!["demo", replay, "--campaign"],
        vec!["demo", replay, "--campaign", campaign, "extra"],
        vec!["demo", "--campaign", campaign, replay],
        vec!["demo"],
    ];
    for args in &cases {
        let out = run(args);
        assert_eq!(out.status.code(), Some(2), "usage must exit 2: {args:?}");
        assert!(out.stdout.is_empty(), "usage prints no stdout: {args:?}");
        let stderr = String::from_utf8(out.stderr).expect("stderr utf-8");
        assert_eq!(
            stderr.matches('\n').count(),
            1,
            "usage is one stderr line: {args:?} -> {stderr:?}"
        );
        assert!(
            stderr.starts_with("crpgc: "),
            "usage carries the shared prefix: {args:?} -> {stderr:?}"
        );
    }
}

/// A missing campaign root and a missing replay keep `crpgc replay`'s exit 1
/// lines.
#[test]
fn missing_campaign_and_replay_keep_replay_lines() {
    let replay = combat_replay();
    let replay = replay.to_str().expect("utf-8");
    let absent = temp_root("absent").join("absent");
    let out = run(&[
        "demo",
        replay,
        "--campaign",
        absent.to_str().expect("utf-8"),
    ]);
    assert_eq!(out.status.code(), Some(1), "missing root: {out:?}");
    assert_eq!(
        out.stderr,
        b"<campaign>: error[io]: cannot open root <campaign-root>: not_found\n"
    );
    let campaign_root = campaign("combat_basic");
    let out = run(&[
        "demo",
        absent.join("absent.replay").to_str().expect("utf-8"),
        "--campaign",
        campaign_root.to_str().expect("utf-8"),
    ]);
    assert_eq!(out.status.code(), Some(1), "missing replay: {out:?}");
    let stderr = String::from_utf8(out.stderr).expect("stderr utf-8");
    assert_eq!(
        stderr.matches("crpgc replay: ").count(),
        1,
        "missing replay keeps the replay line: {stderr:?}"
    );
    assert!(stderr.contains("replay file I/O failed"), "{stderr}");
    let _ = std::fs::remove_dir_all(absent.parent().expect("temp root"));
}

/// The `replay` command's output on the same inputs is unchanged.
#[test]
fn replay_output_is_unchanged() {
    // Portable pin: combat content without `--campaign` still fails exactly
    // as it did before this task, through the reference adapter.
    let replay = combat_replay();
    let out = run(&["replay", replay.to_str().expect("utf-8")]);
    assert_eq!(out.status.code(), Some(1), "legacy failure: {out:?}");
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8(out.stderr).expect("stderr utf-8");
    assert_eq!(
        stderr.matches("crpgc replay: ").count(),
        1,
        "single-pathed replay line: {stderr:?}"
    );
    assert!(stderr.contains("unknown intent payload"), "{stderr}");
    assert!(stderr.contains("tick 0 (index 0)"), "{stderr}");
    // Native pin: the checked-in combat golden still verifies silently.
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
        let golden = native_combat_golden();
        let out = run(&[
            "replay",
            replay.to_str().expect("utf-8"),
            "--campaign",
            campaign("combat_basic").to_str().expect("utf-8"),
            "--golden",
            golden.to_str().expect("utf-8"),
        ]);
        assert!(
            out.status.success(),
            "combat golden must still match: {out:?}"
        );
        assert!(out.stdout.is_empty() && out.stderr.is_empty());
    }
}

/// The supported native combat golden for this target and profile.
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
fn native_combat_golden() -> PathBuf {
    let goldens = cli_dir().join("..").join("crpg-testkit").join("goldens");
    #[cfg(all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc",
        debug_assertions
    ))]
    {
        goldens.join("combat_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden")
    }
    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu",
        debug_assertions
    ))]
    {
        goldens.join("combat_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden")
    }
}
