//! `crpgc play` — playable terminal combat (T146).
//!
//! Starts one encounter from a campaign in a public `crpg-sim` world, then
//! alternates: on the chosen side's turn a person picks one numbered
//! `crpg_sim::legal_actions` option from stdin; on every other side's turn a
//! fixed rule picks an attack (the first legal action targeting a living
//! opponent, else the first legal action). Every change goes through
//! `crpg_sim::perform_action`; no rule, RNG, or combat state lives here.
//!
//! The printed header, action, turn, death, and summary blocks are `demo`'s
//! [`Show`], reused rather than copied, and the encounter binding is the
//! private `combat_apply` adapter's. This is the first version a person
//! plays rather than watches; it adds no vocabulary the sim did not already
//! list.

use std::path::Path;
use std::time::Duration;

use crpg_core::{EntityId, Ulid};
use crpg_data::{Document, LoadedCampaign};
use crpg_sim::{CombatAction, World};

use crate::combat_apply::{Applied, CombatAdapter};
use crate::demo::Show;
use crate::Outcome;

/// Exact stderr bytes when the compile-time engine version does not parse as
/// semver. A release-process bug, never user input.
const ENGINE_FAILURE: &str = "crpgc play: internal engine version failure\n";

/// One choice read from stdin at a player prompt.
enum Prompt {
    /// The zero-based index into the turn's `legal_actions` list.
    Action(usize),
    /// `q`, end of stdin, or an unreadable stdin.
    Quit,
}

/// Plays one encounter from `campaign_root` in the terminal.
///
/// Error precedence matches `crpgc replay --campaign`: campaign collection →
/// engine/load → campaign semantic findings, then the encounter selection.
/// A campaign failure keeps the code and line replay gives it. `--encounter`
/// naming no encounter document, and `--side` naming no combatant, are usage
/// errors (exit 2); a campaign with no encounter at all is the domain
/// failure replay reports for an invalid campaign (exit 1).
pub(crate) fn run(
    campaign_root: &Path,
    encounter: Option<Ulid>,
    side: Option<&str>,
    seed: u64,
    engine_text: &str,
) -> Outcome {
    let files = match crate::collect_campaign_files(campaign_root) {
        Ok(files) => files,
        Err(diagnostic) => return crate::io_outcome(&diagnostic),
    };
    let loaded = match crate::load_for_command(&files, engine_text, ENGINE_FAILURE) {
        Ok(loaded) => loaded,
        Err(outcome) => return outcome,
    };
    let findings = crpg_data::validate(&loaded);
    if !findings.is_empty() {
        return crate::combat_findings_outcome(&findings);
    }
    let encounter_id = match encounter {
        Some(id) => {
            if !has_encounter(&loaded, id) {
                return usage(format!("unknown encounter {id}"));
            }
            id
        }
        None => match first_encounter(&loaded) {
            Some(id) => id,
            None => return failure("campaign has no encounter"),
        },
    };
    let mut show = Show::from_loaded(&loaded, Duration::ZERO);
    let mut adapter = CombatAdapter::new(loaded);
    let mut world = World::new(seed);
    if let Err(message) = adapter.start(&mut world, encounter_id) {
        return failure(&message);
    }
    let player_side = match side {
        Some(name) => {
            if !world
                .combatants()
                .iter()
                .any(|(_, combatant)| show.side(combatant.placement()) == name)
            {
                return usage(format!("unknown side '{name}'"));
            }
            name.to_owned()
        }
        None => match world.combatants().iter().next() {
            Some((_, combatant)) => show.side(combatant.placement()),
            None => return failure("encounter has no combatant"),
        },
    };
    show.header(&world);
    show.after(&world);
    loop {
        let Some(active) = world.combat().and_then(|combat| combat.active) else {
            show.summary();
            return Outcome::success();
        };
        let Some(actor_placement) = placement_of(&world, active) else {
            return failure("active combatant is missing");
        };
        let options = match crpg_sim::legal_actions(&world, active) {
            Ok(options) => options,
            Err(error) => return failure(&error.to_string()),
        };
        let chosen = if show.side(actor_placement) == player_side {
            match prompt(&mut show, &world, actor_placement, &options) {
                Prompt::Action(index) => options[index],
                Prompt::Quit => {
                    show.emit("Quit.\n");
                    return Outcome::success();
                }
            }
        } else {
            match npc_action(&show, &world, actor_placement, &options) {
                Some(action) => action,
                None => return failure("no legal action"),
            }
        };
        match adapter.perform(&mut world, &chosen) {
            Ok(applied) => print_applied(&mut show, &world, applied),
            Err(message) => return failure(&message),
        }
    }
}

/// Reads one choice for the acting player-side combatant.
///
/// The block names the round, the acting combatant, and everyone's health,
/// then the `legal_actions` list in returned order by ability and target
/// name. Invalid input prints one line naming the valid range and asks again
/// without touching the world.
fn prompt(show: &mut Show, world: &World, actor: Ulid, options: &[CombatAction]) -> Prompt {
    let round = world.combat().map_or(0, |combat| combat.round) + 1;
    let mut text = format!("-- Round {round} --\nTurn: {}\n", show.name(actor));
    for (_, combatant) in world.combatants().iter() {
        text.push_str(&format!(
            "  {} {}/{} hp\n",
            show.name(combatant.placement()),
            combatant.health(),
            combatant.max_health()
        ));
    }
    text.push_str("Options:\n");
    for (index, option) in options.iter().enumerate() {
        text.push_str(&format!(
            "  {}. {}\n",
            index + 1,
            option_text(show, world, option)
        ));
    }
    text.push_str("  q. quit\n");
    show.emit(&text);
    let mut line = String::new();
    loop {
        line.clear();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => return Prompt::Quit,
            Ok(_) => {}
        }
        let text = line.trim();
        if text == "q" {
            return Prompt::Quit;
        }
        if let Ok(number) = text.parse::<usize>() {
            if (1..=options.len()).contains(&number) {
                return Prompt::Action(number - 1);
            }
        }
        show.emit(&format!(
            "Enter a number from 1 to {}, or q to quit.\n",
            options.len()
        ));
    }
}

/// Renders one legal option by ability name and target name.
fn option_text(show: &Show, world: &World, option: &CombatAction) -> String {
    match *option {
        CombatAction::UseAbility {
            ability, target, ..
        } => {
            // `legal_actions` lists only live combatant targets, so the
            // fallback cannot print for an option this module produced.
            match placement_of(world, target) {
                Some(placement) => {
                    format!("{} -> {}", show.ability_name(ability), show.name(placement))
                }
                None => show.ability_name(ability),
            }
        }
        CombatAction::EndTurn { .. } => "End turn".to_string(),
        CombatAction::Stop { .. } => "Stop".to_string(),
    }
}

/// The computer's choice: the first listed attack on a living opponent, else
/// the first listed action.
///
/// "Opponent" is a living combatant whose side differs from the acting
/// combatant's side. The list itself is the sim's, in its returned order.
fn npc_action(
    show: &Show,
    world: &World,
    actor: Ulid,
    options: &[CombatAction],
) -> Option<CombatAction> {
    let actor_side = show.side(actor);
    for option in options {
        if let CombatAction::UseAbility { target, .. } = *option {
            let opponent = world.combatants().get(target).is_some_and(|combatant| {
                !combatant.dead() && show.side(combatant.placement()) != actor_side
            });
            if opponent {
                return Some(*option);
            }
        }
    }
    options.first().copied()
}

/// Prints one applied action exactly as `demo` prints it, then its aftermath.
fn print_applied(show: &mut Show, world: &World, applied: Applied) {
    match applied {
        Applied::Init => show.header(world),
        Applied::Attack {
            actor,
            ability,
            target,
            target_entity,
            outcome,
        } => show.attack(
            world,
            actor,
            ability,
            target,
            target_entity,
            outcome.as_ref(),
        ),
        Applied::End { actor } => show.end_turn(actor),
    }
    show.after(world);
}

/// One bound runtime entity's authored placement.
fn placement_of(world: &World, entity: EntityId) -> Option<Ulid> {
    world
        .combatants()
        .get(entity)
        .map(|combatant| combatant.placement())
}

/// The first encounter document in source-path (document) order.
fn first_encounter(loaded: &LoadedCampaign) -> Option<Ulid> {
    loaded
        .documents
        .values()
        .find_map(|document| match document {
            Document::Encounter(encounter) => Some(encounter.id),
            _ => None,
        })
}

/// Whether `loaded` holds an encounter document with this identity.
fn has_encounter(loaded: &LoadedCampaign, id: Ulid) -> bool {
    loaded
        .documents
        .values()
        .any(|document| matches!(document, Document::Encounter(encounter) if encounter.id == id))
}

/// A usage failure: exit 2, one stderr line, empty stdout.
fn usage(message: String) -> Outcome {
    Outcome::cli_error(&crate::CliError::Usage(message))
}

/// A domain failure: exit 1 with one `crpgc play:` stderr line.
fn failure(message: &str) -> Outcome {
    Outcome {
        code: 1,
        stdout: Vec::new(),
        stderr: format!("crpgc play: {message}\n").into_bytes(),
    }
}
