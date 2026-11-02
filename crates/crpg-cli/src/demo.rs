//! `crpgc demo` — the terminal combat viewer (T145), plus the block printer
//! `crpgc play` (T146) reuses.
//!
//! Reads a combat replay through `crpg-testkit` (`read_replay`/`play_replay`,
//! so validation, input ordering, and the tick interleaving stay there),
//! plays it through the private `combat_apply` decode/bind glue, and prints
//! the fight as it happens. It is a viewer: no rules, no RNG, no replay
//! semantics, and no sim state live here. Every printed fact is read from a
//! campaign document or from existing public `crpg-sim` API.
//!
//! [`Show`] is the shared presentation: `demo` drives it from replay payloads
//! and `play` from chosen legal actions, so both commands print byte-identical
//! header, action, turn, death, and summary blocks. Only visibility moved for
//! T146; demo's bytes are unchanged.
//!
//! Output is streamed and flushed per block so `--delay-ms` lets a person
//! follow along; the bytes are deterministic for a fixed replay and campaign.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use crpg_core::{EntityId, Ulid};
use crpg_data::{Document, LoadedCampaign};
use crpg_sim::{ActionOutcome, World};

use crate::combat_apply::{Applied, CombatAdapter};
use crate::Outcome;

/// Exact stderr bytes when the compile-time engine version does not parse as
/// semver. A release-process bug, never user input.
const ENGINE_FAILURE: &str = "crpgc demo: internal engine version failure\n";

/// Plays `replay_path` through `campaign_root`, printing the fight.
///
/// Error precedence matches `crpgc replay --campaign`: campaign collection →
/// engine/load → campaign semantic findings → replay read/validation →
/// application. Every campaign and replay failure keeps the code and line
/// `crpgc replay` gives for it. Because the show streams, an apply failure
/// after the first printed block leaves that output on stdout.
pub(crate) fn run(
    replay_path: &Path,
    campaign_root: &Path,
    delay_ms: u64,
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
    let replay = match crpg_testkit::read_replay(replay_path) {
        Ok(replay) => replay,
        Err(error) => return Outcome::cli_error(&crate::CliError::Replay(error)),
    };
    let show = Rc::new(RefCell::new(Show::from_loaded(
        &loaded,
        Duration::from_millis(delay_ms),
    )));
    let shared = Rc::clone(&show);
    let mut adapter = CombatAdapter::new(loaded);
    let apply: crpg_testkit::ApplyInput = Box::new(move |world: &mut World, payload| {
        let applied = adapter.step(world, payload)?;
        let mut show = shared.borrow_mut();
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
        Ok(())
    });
    if let Err(error) = crpg_testkit::play_replay(&replay, apply) {
        return Outcome::cli_error(&crate::CliError::Replay(error));
    }
    let show = show.borrow();
    if show.initialized {
        show.summary();
    }
    Outcome::success()
}

/// Authored display facts resolved from the loaded campaign documents.
///
/// Document `name` fields are locale keys, so a key is resolved through the
/// campaign's locale table when one carries it; otherwise the key itself is
/// printed (still the document's authored name). A raw ULID is printed only
/// when no document names the identity.
struct Names {
    /// Campaign document name key.
    campaign: String,
    /// Merged locale keys to display text.
    strings: BTreeMap<String, String>,
    /// Encounter identity to name key.
    encounters: BTreeMap<Ulid, String>,
    /// Ability identity to name key.
    abilities: BTreeMap<Ulid, String>,
    /// Placement identity to its authored name key and prefab.
    placements: BTreeMap<Ulid, (String, Ulid)>,
    /// Creature prefab identity to its authored name key and faction.
    creatures: BTreeMap<Ulid, (String, Option<Ulid>)>,
    /// Faction identity to name key.
    factions: BTreeMap<Ulid, String>,
}

impl Names {
    /// Collects the naming documents in deterministic source-path order.
    fn from_loaded(loaded: &LoadedCampaign) -> Self {
        let mut names = Self {
            campaign: String::new(),
            strings: BTreeMap::new(),
            encounters: BTreeMap::new(),
            abilities: BTreeMap::new(),
            placements: BTreeMap::new(),
            creatures: BTreeMap::new(),
            factions: BTreeMap::new(),
        };
        for document in loaded.documents.values() {
            match document {
                Document::Campaign(campaign) => names.campaign = campaign.name.clone(),
                Document::Locale(locale) => {
                    for (key, text) in &locale.strings {
                        names.strings.insert(key.clone(), text.clone());
                    }
                }
                Document::Encounter(encounter) => {
                    names
                        .encounters
                        .insert(encounter.id, encounter.name.clone());
                }
                Document::Ability(ability) => {
                    names.abilities.insert(ability.id, ability.name.clone());
                }
                Document::Placements(placements) => {
                    for placement in &placements.placements {
                        names
                            .placements
                            .insert(placement.id, (placement.name.clone(), placement.prefab));
                    }
                }
                Document::Creature(creature) => {
                    names
                        .creatures
                        .insert(creature.id, (creature.name.clone(), creature.faction));
                }
                Document::Faction(faction) => {
                    names.factions.insert(faction.id, faction.name.clone());
                }
                _ => {}
            }
        }
        names
    }

    /// Resolves one document name key through the locale table.
    fn text(&self, key: &str) -> String {
        self.strings
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_owned())
    }

    /// The campaign's display name, or its identity when unnamed.
    fn campaign(&self) -> String {
        if self.campaign.is_empty() {
            "unnamed campaign".to_string()
        } else {
            self.text(&self.campaign)
        }
    }

    /// One encounter's display name.
    fn encounter(&self, encounter: Ulid) -> String {
        match self.encounters.get(&encounter) {
            Some(key) => self.text(key),
            None => encounter.to_string(),
        }
    }

    /// One ability's display name.
    fn ability(&self, ability: Ulid) -> String {
        match self.abilities.get(&ability) {
            Some(key) => self.text(key),
            None => ability.to_string(),
        }
    }

    /// One combatant's display name and side.
    ///
    /// The name is the prefab (creature) name, falling back to the placement
    /// name and then the raw placement identity. The side is the creature's
    /// faction name when it names one, else the creature name, so combatants
    /// instantiated from the same prefab share a side.
    fn combatant(&self, placement: Ulid) -> (String, String) {
        let placed = self.placements.get(&placement);
        let creature = placed.and_then(|(_, prefab)| self.creatures.get(prefab));
        let name = match creature {
            Some((key, _)) => self.text(key),
            None => match placed {
                Some((key, _)) => self.text(key),
                None => placement.to_string(),
            },
        };
        let side = match creature {
            Some((key, faction)) => match faction.and_then(|id| self.factions.get(&id)) {
                Some(faction_key) => self.text(faction_key),
                None => self.text(key),
            },
            None => name.clone(),
        };
        (name, side)
    }
}

/// One combatant's observed health at the latest applied payload.
#[derive(Clone, Copy)]
struct Health {
    health: u32,
    max: u32,
    dead: bool,
}

/// The printed show: name tables plus the facts the summary needs after
/// `play_replay` has dropped its world.
///
/// `demo` and `play` share it: names and sides resolve from campaign
/// documents, and each printed block is flushed as it is produced.
pub(crate) struct Show {
    names: Names,
    delay: Duration,
    initialized: bool,
    actions: usize,
    round: u32,
    active: Option<EntityId>,
    /// Authored participant order, as the store published it at init.
    order: Vec<Ulid>,
    /// Latest observed health by placement.
    health: BTreeMap<Ulid, Health>,
    /// Runtime entity to authored placement, for turn-head naming.
    entities: BTreeMap<EntityId, Ulid>,
    /// The last `-- Round N --` display number printed.
    printed_round: Option<u32>,
    /// The last `Turn: ...` line printed.
    printed_turn: Option<String>,
}

impl Show {
    /// Builds an uninitialized show with its name tables and pause.
    fn new(names: Names, delay: Duration) -> Self {
        Self {
            names,
            delay,
            initialized: false,
            actions: 0,
            round: 0,
            active: None,
            order: Vec::new(),
            health: BTreeMap::new(),
            entities: BTreeMap::new(),
            printed_round: None,
            printed_turn: None,
        }
    }

    /// Builds an uninitialized show whose names resolve against `loaded`.
    ///
    /// Shared by `demo` and `play`; the caller owns the pause (only `demo`
    /// has a `--delay-ms` flag).
    pub(crate) fn from_loaded(loaded: &LoadedCampaign, delay: Duration) -> Self {
        Self::new(Names::from_loaded(loaded), delay)
    }

    /// One combatant's display name for its authored placement.
    pub(crate) fn name(&self, placement: Ulid) -> String {
        self.names.combatant(placement).0
    }

    /// The side a combatant's authored placement belongs to.
    pub(crate) fn side(&self, placement: Ulid) -> String {
        self.names.combatant(placement).1
    }

    /// One ability's display name.
    pub(crate) fn ability_name(&self, ability: Ulid) -> String {
        self.names.ability(ability)
    }

    /// Writes one block to stdout and flushes it.
    pub(crate) fn emit(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let mut out = std::io::stdout();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }

    /// Sleeps after one printed intent when `--delay-ms` asked for it.
    fn pause(&self) {
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
    }

    /// Prints the campaign and combatant header after the encounter starts.
    pub(crate) fn header(&mut self, world: &World) {
        let campaign = self.names.campaign();
        let encounter = world.combat().map_or_else(String::new, |combat| {
            self.names.encounter(combat.definition.encounter)
        });
        let mut text = format!("== Combat demo: {campaign} ==\nEncounter: {encounter}\n");
        text.push_str("Combatants:\n");
        for (_, combatant) in world.combatants().iter() {
            let (name, side) = self.names.combatant(combatant.placement());
            text.push_str(&format!(
                "  {name} | side {side} | {}/{} hp\n",
                combatant.health(),
                combatant.max_health()
            ));
        }
        text.push('\n');
        self.initialized = true;
        self.emit(&text);
    }

    /// Prints one action block for an ability use.
    pub(crate) fn attack(
        &mut self,
        world: &World,
        actor: Ulid,
        ability: Ulid,
        target: Ulid,
        target_entity: EntityId,
        outcome: Option<&ActionOutcome>,
    ) {
        self.actions += 1;
        let (actor_name, _) = self.names.combatant(actor);
        let (target_name, _) = self.names.combatant(target);
        let ability_name = self.names.ability(ability);
        let mut text = format!(
            "[{}] {actor_name} | {ability_name} -> {target_name}\n",
            self.actions
        );
        if let Some(outcome) = outcome {
            let faces: Vec<String> = outcome
                .roll
                .dice
                .iter()
                .map(|die| die.value.to_string())
                .collect();
            text.push_str(&format!(
                "    dice {} = {} ({}); margin {:+} -> {}\n",
                outcome.roll.expression,
                outcome.roll.total,
                faces.join("+"),
                outcome.margin,
                verdict(outcome)
            ));
            text.push_str(&format!("    damage {}\n", outcome.damage));
        } else {
            text.push_str("    no immediate result\n");
        }
        if let Some(combatant) = world.combatants().get(target_entity) {
            text.push_str(&format!(
                "    {target_name}: {}/{} hp\n",
                combatant.health(),
                combatant.max_health()
            ));
        }
        self.emit(&text);
        self.pause();
    }

    /// Prints one block for a completed turn.
    pub(crate) fn end_turn(&mut self, actor: Ulid) {
        self.actions += 1;
        let (actor_name, _) = self.names.combatant(actor);
        let text = format!("[{}] {actor_name} | ends turn\n", self.actions);
        self.emit(&text);
        self.pause();
    }

    /// Prints the death, round-change, and turn-change lines after one step.
    /// Round and turn lines print only while a turn is in play and only when
    /// their value differs from the last one printed.
    pub(crate) fn after(&mut self, world: &World) {
        let previous_round = self.round;
        let previous_dead: Vec<Ulid> = self
            .health
            .iter()
            .filter(|(_, health)| health.dead)
            .map(|(placement, _)| *placement)
            .collect();
        self.snapshot(world);
        let mut text = String::new();
        for placement in &self.order {
            let newly_dead = self.health.get(placement).is_some_and(|health| health.dead)
                && !previous_dead.contains(placement);
            if newly_dead {
                let (name, _) = self.names.combatant(*placement);
                text.push_str(&format!("  {name} dies.\n"));
            }
        }
        if self.initialized && self.round != previous_round {
            text.push_str(&self.round_line(self.round + 1));
        }
        if let Some(turn) = self.turn_line() {
            text.push_str(&turn);
            text.push('\n');
        }
        self.emit(&text);
    }

    /// Appends the round and turn lines for a prompt, each only when a turn
    /// is in play and its value differs from the last line printed.
    pub(crate) fn round_and_turn(&mut self) -> String {
        if self.active.is_none() {
            return String::new();
        }
        let mut text = self.round_line(self.round + 1);
        if let Some(turn) = self.turn_line() {
            text.push_str(&turn);
            text.push('\n');
        }
        text
    }

    /// Refreshes the observed health, round, and turn head from the world.
    fn snapshot(&mut self, world: &World) {
        self.order.clear();
        self.health.clear();
        self.entities.clear();
        for (entity, combatant) in world.combatants().iter() {
            let placement = combatant.placement();
            self.order.push(placement);
            self.entities.insert(entity, placement);
            self.health.insert(
                placement,
                Health {
                    health: combatant.health(),
                    max: combatant.max_health(),
                    dead: combatant.dead(),
                },
            );
        }
        self.round = world.combat().map_or(0, |combat| combat.round);
        self.active = world.combat().and_then(|combat| combat.active);
    }

    /// Renders the round line unless `displayed` was the last one printed.
    fn round_line(&mut self, displayed: u32) -> String {
        if self.printed_round == Some(displayed) {
            return String::new();
        }
        self.printed_round = Some(displayed);
        format!("-- Round {displayed} --\n")
    }

    /// Renders the current turn head by authored name, or `None` when there
    /// is no turn to show or it repeats the last line printed.
    fn turn_line(&mut self) -> Option<String> {
        let active = self.active?;
        let placement = *self.entities.get(&active)?;
        let (name, _) = self.names.combatant(placement);
        let line = format!("Turn: {name}");
        if self.printed_turn.as_deref() == Some(line.as_str()) {
            return None;
        }
        self.printed_turn = Some(line.clone());
        Some(line)
    }

    /// Prints the closing summary from the last observed snapshot.
    pub(crate) fn summary(&self) {
        let mut text = String::from("== Summary ==\n");
        text.push_str(&format!("Rounds played: {}\n", self.round + 1));
        text.push_str("Final health:\n");
        let mut survivors = Vec::new();
        let mut sides: BTreeMap<String, bool> = BTreeMap::new();
        for placement in &self.order {
            let Some(health) = self.health.get(placement) else {
                continue;
            };
            let (name, side) = self.names.combatant(*placement);
            let dead = if health.dead { " (dead)" } else { "" };
            text.push_str(&format!(
                "  {name} {}/{} hp{dead}\n",
                health.health, health.max
            ));
            if !health.dead {
                survivors.push(name);
            }
            let alive = sides.entry(side).or_insert(false);
            if !health.dead {
                *alive = true;
            }
        }
        let survivor_text = if survivors.is_empty() {
            "none".to_string()
        } else {
            survivors.join(", ")
        };
        text.push_str(&format!("Survivors: {survivor_text}\n"));
        let winners: Vec<String> = sides
            .iter()
            .filter(|(_, alive)| **alive)
            .map(|(side, _)| side.clone())
            .collect();
        let winner = match winners.as_slice() {
            [only] => only.clone(),
            _ => "draw".to_string(),
        };
        text.push_str(&format!("Winner: {winner}\n"));
        self.emit(&text);
    }
}

/// Renders the sim's outcome label as a hit/miss verdict plus its label.
///
/// The rules `Outcome` type is not nameable from this crate (no `crpg-rules`
/// edge), so the label is read through its public `Serialize` form, which is
/// the same adjacent `type`/`value` wire the data layer authors.
fn verdict(outcome: &ActionOutcome) -> String {
    let serialized = serde_json::to_value(outcome.outcome).ok();
    let label = serialized
        .as_ref()
        .and_then(|value| value.get("type"))
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_string();
    let hit = match label.as_str() {
        "success" | "critical_success" => "hit",
        "failure" | "critical_failure" => "miss",
        _ => "outcome",
    };
    format!("{hit} ({})", label.replace('_', " "))
}
