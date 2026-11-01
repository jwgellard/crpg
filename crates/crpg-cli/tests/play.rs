//! Black-box acceptance for `crpgc play` (T146).
//!
//! Drives the built binary (`env!("CARGO_BIN_EXE_crpgc")`) as a real process
//! over the checked-in `combat_basic` and `combat_srd` fixtures, piping stdin
//! and asserting exit codes plus stdout bytes, never play internals. The
//! final-health clause is cross-checked by an independent route: this file
//! loads the campaign through production `crpg-data`, assembles the encounter
//! spec itself, and drives the same choices through public `crpg-sim`
//! operations. Checked-in inputs stay read-only.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

use crpg_core::Ulid;
use crpg_data::{Document, LoadedCampaign, SourcePath};
use crpg_sim::{CombatAction, EncounterSpec, PlacementAndArea, World};

/// The two checked-in combat fixtures every behaviour test runs over.
const FIXTURES: [&str; 2] = ["combat_basic", "combat_srd"];

/// The built `crpgc` binary path, provided by cargo for integration tests.
fn crpgc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_crpgc"))
}

/// The `crpg-cli` crate directory, without depending on the working dir.
fn cli_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The repository root, two levels above the crate.
fn repo_root() -> PathBuf {
    cli_dir().join("..").join("..")
}

/// One checked-in fixture campaign root (read-only).
fn campaign(slug: &str) -> PathBuf {
    repo_root().join("campaigns").join("fixtures").join(slug)
}

/// The checked-in replay matching each fixture, for the header comparison.
fn replay(slug: &str) -> PathBuf {
    cli_dir()
        .join("..")
        .join("crpg-testkit")
        .join("fixtures")
        .join(format!("{slug}.replay"))
}

/// Runs `crpgc` with Unicode `args` and no stdin input.
fn run(args: &[&str]) -> Output {
    Command::new(crpgc())
        .args(args)
        .output()
        .expect("spawning crpgc must succeed")
}

/// Runs `crpgc play` with `stdin` fully piped in and then closed.
fn play_with(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(crpgc())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawning crpgc must succeed");
    child
        .stdin
        .take()
        .expect("stdin pipe")
        .write_all(stdin.as_bytes())
        .expect("stdin writes");
    child.wait_with_output().expect("child waits")
}

/// Runs `play` over one fixture with optional extra flags.
fn play(slug: &str, extra: &[&str], stdin: &str) -> Output {
    let root = campaign(slug);
    let root = root.to_str().expect("utf-8");
    let mut args = vec!["play", "--campaign", root];
    args.extend_from_slice(extra);
    play_with(&args, stdin)
}

/// Runs `play` and returns stdout after asserting a clean success.
fn play_stdout(slug: &str, extra: &[&str], stdin: &str) -> String {
    let out = play(slug, extra, stdin);
    assert_eq!(out.status.code(), Some(0), "play must exit 0: {out:?}");
    assert!(
        out.stderr.is_empty(),
        "success prints no stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("stdout utf-8")
}

/// A fresh stdin script: still enough choices to finish either fixture.
fn ones() -> String {
    "1\n".repeat(200)
}

/// The header block: every line up to and including the first blank line.
fn header_block(stdout: &str) -> String {
    let mut block = String::new();
    for line in stdout.lines() {
        if line.is_empty() {
            break;
        }
        block.push_str(line);
        block.push('\n');
    }
    block
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

/// The header's printed side per combatant name.
fn header_sides(stdout: &str) -> BTreeMap<String, String> {
    let mut sides = BTreeMap::new();
    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("  ") else {
            continue;
        };
        let Some((name, tail)) = rest.split_once(" | side ") else {
            continue;
        };
        let Some((side, _)) = tail.split_once(" | ") else {
            continue;
        };
        sides.insert(name.to_string(), side.to_string());
    }
    sides
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

/// The sides with at least one living combatant, from the summary block.
fn surviving_sides(stdout: &str) -> Vec<String> {
    let sides = header_sides(stdout);
    let mut dead = BTreeMap::new();
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
            let is_dead = rest.ends_with(" (dead)");
            let rest = rest.strip_suffix(" (dead)").unwrap_or(rest);
            let rest = rest.strip_suffix(" hp").expect("hp suffix");
            let (name, _) = rest.rsplit_once(' ').expect("name and health");
            dead.insert(name.to_string(), is_dead);
        }
    }
    let mut alive: BTreeMap<String, bool> = BTreeMap::new();
    for (name, side) in &sides {
        let entry = alive.entry(side.clone()).or_insert(false);
        *entry |= !dead.get(name).copied().unwrap_or(true);
    }
    alive
        .into_iter()
        .filter(|(_, alive)| *alive)
        .map(|(side, _)| side)
        .collect()
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

/// The independent sim route's observed end state.
/// (Both scripted routes use the first option, the same choice as stdin `1`.)
struct DirectFight {
    /// Authored participant placements in encounter order.
    participants: Vec<Ulid>,
    /// Final `(health, max_health, dead)` by placement.
    final_health: BTreeMap<Ulid, (u32, u32, bool)>,
}

/// Plays the fixture through public `crpg-sim` API only, taking the first
/// listed option on the first participant's turns and the same computer rule
/// as the command.
///
/// This is deliberately a second route: it assembles the encounter spec from
/// the loaded documents itself and never reads the command's stdout. The
/// fixtures are one combatant per side with distinct prefabs and no factions,
/// so "the first participant's side" and "every other living combatant" are
/// exactly the sides the command reports.
fn direct_fight(root: &Path, seed: u64) -> DirectFight {
    let loaded = load(root);
    let encounter = loaded
        .documents
        .values()
        .find_map(|document| match document {
            Document::Encounter(value) => Some(value),
            _ => None,
        })
        .expect("fixture has an encounter");
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
    let mut world = World::new(seed);
    crpg_sim::start_encounter(&mut world, &spec).expect("encounter starts");
    let participants: Vec<Ulid> = world
        .combatants()
        .iter()
        .map(|(_, combatant)| combatant.placement())
        .collect();
    let player = participants[0];
    let mut guard = 0usize;
    while let Some(active) = world.combat().and_then(|combat| combat.active) {
        guard += 1;
        assert!(guard < 1000, "the scripted fight must terminate");
        let active_placement = world
            .combatants()
            .get(active)
            .expect("the active entity is a combatant")
            .placement();
        let options = crpg_sim::legal_actions(&world, active).expect("options list");
        let action = if active_placement == player {
            options[0]
        } else {
            options
                .iter()
                .copied()
                .find(|option| match *option {
                    CombatAction::UseAbility { target, .. } => {
                        world.combatants().get(target).is_some_and(|combatant| {
                            !combatant.dead() && combatant.placement() != active_placement
                        })
                    }
                    _ => false,
                })
                .unwrap_or(options[0])
        };
        crpg_sim::perform_action(&mut world, &action).expect("listed action applies");
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
    DirectFight {
        participants,
        final_health,
    }
}

/// Whether `line` is one numbered option line (`  3. Heavy -> Goblin`).
fn is_option_line(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("  ") else {
        return false;
    };
    let Some((number, _)) = rest.split_once(". ") else {
        return false;
    };
    !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
}

/// Spawns `play` and answers every player prompt with the last listed option.
///
/// Choosing the last option requires knowing each turn's option count, so the
/// process is driven interactively: the answers are read from the printed
/// going, so the run proves every last-listed choice performs.
fn play_last_option(slug: &str) -> (Child, String, String, usize) {
    let root = campaign(slug);
    let mut child = Command::new(crpgc())
        .args(["play", "--campaign", root.to_str().expect("utf-8")])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawning crpgc must succeed");
    let mut stdin = child.stdin.take().expect("stdin pipe");
    let stdout = child.stdout.take().expect("stdout pipe");
    let mut reader = BufReader::new(stdout);
    let mut printed = String::new();
    let mut options = 0usize;
    let mut turns = 0usize;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).expect("stdout reads") == 0 {
            break;
        }
        printed.push_str(&line);
        if line == "Options:\n" {
            options = 0;
        } else if is_option_line(&line) {
            options += 1;
        } else if line == "  q. quit\n" {
            assert!(options >= 1, "every prompt lists at least one option");
            turns += 1;
            writeln!(stdin, "{options}").expect("stdin writes");
            stdin.flush().expect("stdin flush");
        }
    }
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr pipe")
        .read_to_string(&mut stderr)
        .expect("stderr reads");
    (child, printed, stderr, turns)
}

/// Behaviour 1: the command prints the same header as `crpgc demo`.
#[test]
fn header_is_the_demo_header() {
    for slug in FIXTURES {
        let out = run(&[
            "demo",
            replay(slug).to_str().expect("utf-8"),
            "--campaign",
            campaign(slug).to_str().expect("utf-8"),
        ]);
        assert_eq!(out.status.code(), Some(0), "demo must exit 0: {out:?}");
        let demo = String::from_utf8(out.stdout).expect("stdout utf-8");
        let played = play_stdout(slug, &[], "q\n");
        assert_eq!(
            header_block(&played),
            header_block(&demo),
            "{slug} play must print demo's header"
        );
    }
}

/// Behaviour 2: a player turn prints the round, the acting combatant,
/// everyone's health, the numbered `legal_actions` list by ability and target
/// name, and `q` to quit.
#[test]
fn prompt_lists_round_health_and_legal_actions_by_name() {
    let expected: [(&str, &[&str]); 2] = [
        ("combat_basic", &["Strike -> Goblin", "End turn"]),
        (
            "combat_srd",
            &[
                "Heavy -> Goblin",
                "Focus -> Hero",
                "Focus -> Goblin",
                "End turn",
            ],
        ),
    ];
    for (slug, options) in expected {
        let stdout = play_stdout(slug, &[], "q\n");
        assert!(stdout.contains("-- Round 1 --\n"), "{slug}: {stdout}");
        assert!(
            stdout.contains("Options:\n"),
            "{slug} prints an option block: {stdout}"
        );
        let listed: Vec<String> = stdout
            .lines()
            .filter(|line| is_option_line(line))
            .map(|line| {
                line.strip_prefix("  ")
                    .expect("two-space indent")
                    .split_once(". ")
                    .expect("number and text")
                    .1
                    .to_string()
            })
            .collect();
        assert_eq!(listed, options, "{slug} lists legal_actions in order");
        assert!(stdout.contains("  q. quit\n"), "{slug} offers quit");
        // The acting combatant and everyone's health are printed.
        assert!(
            stdout.contains("Turn: Hero\n-- Round 1 --"),
            "{slug} names the acting combatant after the computer's turn: {stdout}"
        );
        for name in header_names(&stdout) {
            assert!(
                stdout.contains(&format!("  {name} ")) && stdout.contains(" hp\n"),
                "{slug} prints health for {name}: {stdout}"
            );
        }
    }
}

/// Behaviour 3: a number in range performs that action and prints demo's
/// block; `0`, an out-of-range number, and non-numeric text print one range
/// line, ask again, and change nothing.
#[test]
fn invalid_input_prints_the_range_line_and_changes_nothing() {
    for slug in FIXTURES {
        let baseline = play_stdout(slug, &[], &ones());
        for bad in ["0", "99", "abc"] {
            let stdin = format!("{bad}\n{}", ones());
            let mutated = play_stdout(slug, &[], &stdin);
            let range_lines: Vec<&str> = mutated
                .lines()
                .filter(|line| line.starts_with("Enter a number from 1 to "))
                .collect();
            assert_eq!(
                range_lines.len(),
                1,
                "{slug} {bad:?} prints exactly one range line: {mutated}"
            );
            assert!(
                range_lines[0].ends_with(", or q to quit."),
                "{slug} {bad:?} names the range: {}",
                range_lines[0]
            );
            let without: String = mutated
                .lines()
                .filter(|line| !line.starts_with("Enter a number from 1 to "))
                .map(|line| format!("{line}\n"))
                .collect();
            assert_eq!(
                without, baseline,
                "{slug} {bad:?} leaves the following output unchanged"
            );
        }
    }
}

/// Behaviour 3: every listed option performs, checked by choosing the last
/// option on each player turn in one run.
#[test]
fn last_option_on_every_turn_performs_to_the_end() {
    for slug in FIXTURES {
        let (mut child, printed, stderr, turns) = play_last_option(slug);
        let status = child.wait().expect("child waits");
        assert!(
            status.success(),
            "{slug} last-option run exits 0: {printed}"
        );
        assert!(stderr.is_empty(), "{slug} prints no stderr: {stderr}");
        assert!(turns >= 1, "{slug} reached at least one player turn");
        assert!(
            printed.contains("== Summary ==\n"),
            "{slug} reaches the demo summary: {printed}"
        );
        assert!(
            summary_line(&printed, "Winner: ") != "draw",
            "{slug} names a winner: {printed}"
        );
    }
}

/// Behaviour 4: on another side's turn the computer takes the first listed
/// attack that targets a living opponent.
#[test]
fn computer_takes_the_first_attack_on_a_living_opponent() {
    // Making the goblin the player's side leaves the hero to the computer;
    // ending the goblin's turn hands the hero that automatic turn.
    let stdout = play_stdout("combat_basic", &["--side", "Goblin"], "2\nq\n");
    let auto = stdout
        .lines()
        .find(|line| line.contains("] Hero | "))
        .unwrap_or_else(|| panic!("the computer acts for the hero: {stdout}"));
    assert!(
        auto.ends_with(" -> Goblin"),
        "the computer targets the living opponent: {auto}"
    );
    assert!(
        auto.contains("Strike"),
        "the computer uses a listed attack: {auto}"
    );
    assert!(
        stdout.contains("  q. quit\n"),
        "control returns to the player's side after the automatic turn: {stdout}"
    );
}

/// Behaviour 5: a finished fight prints demo's summary and exits 0 with
/// exactly one surviving side.
#[test]
fn repeated_first_option_plays_to_the_end() {
    for slug in FIXTURES {
        let stdout = play_stdout(slug, &[], &ones());
        assert!(
            stdout.contains("== Summary ==\n"),
            "{slug} prints the demo summary: {stdout}"
        );
        let survivors = surviving_sides(&stdout);
        assert_eq!(
            survivors.len(),
            1,
            "{slug} ends with exactly one winning side: {stdout}"
        );
        assert_eq!(
            summary_line(&stdout, "Winner: "),
            survivors[0],
            "{slug} names the surviving side: {stdout}"
        );
    }
}

/// Behaviour 5: `q` and end of stdin print `Quit.`, exit 0, and print no
/// summary.
#[test]
fn q_and_end_of_stdin_quit_cleanly() {
    for slug in FIXTURES {
        for stdin in ["q\n", ""] {
            let stdout = play_stdout(slug, &[], stdin);
            assert!(
                stdout.ends_with("Quit.\n"),
                "{slug} {stdin:?} prints Quit.: {stdout}"
            );
            assert!(
                !stdout.contains("== Summary =="),
                "{slug} {stdin:?} prints no summary: {stdout}"
            );
            assert!(
                stdout.contains("  q. quit\n"),
                "{slug} {stdin:?} asked before quitting: {stdout}"
            );
        }
    }
}

/// The test's determinism clause: the same stdin and seed give byte-identical
/// stdout, while at least one of seeds 1..=9 differs from seed 0.
#[test]
fn same_inputs_are_byte_identical_and_a_seed_can_differ() {
    for slug in FIXTURES {
        let stdin = ones();
        let first = play_stdout(slug, &[], &stdin);
        let second = play_stdout(slug, &[], &stdin);
        assert_eq!(first, second, "{slug} output must be deterministic");
        let mut differs = false;
        for seed in 1..=9u64 {
            let seed_text = seed.to_string();
            let other = play_stdout(slug, &["--seed", &seed_text], &stdin);
            differs |= other != first;
        }
        assert!(
            differs,
            "{slug} at least one of seeds 1..=9 differs from seed 0"
        );
    }
}

/// The independent-route clause: the summary's final health equals what
/// public `crpg-sim` reaches with the same choices and seed.
#[test]
fn summary_health_matches_the_direct_sim_run() {
    for slug in FIXTURES {
        let seed = 7u64;
        let seed_text = seed.to_string();
        let stdout = play_stdout(slug, &["--seed", &seed_text], &ones());
        let sim = direct_fight(&campaign(slug), seed);
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

/// The usage clause: unknown `--side`, unknown `--encounter`, a missing
/// `--campaign`, and a non-number `--seed` each exit 2 with one stderr line.
#[test]
fn usage_errors_exit_2() {
    let root = campaign("combat_basic");
    let root = root.to_str().expect("utf-8");
    let unknown = "00000000000000000000000000";
    let cases: Vec<Vec<&str>> = vec![
        vec!["play", "--campaign", root, "--side", "Nobody"],
        vec!["play", "--campaign", root, "--encounter", unknown],
        vec!["play"],
        vec!["play", "--campaign", root, "--seed", "abc"],
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
