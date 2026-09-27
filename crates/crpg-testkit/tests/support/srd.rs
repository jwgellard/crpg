//! Tests-only srd adapter for T017e (never ships, never in `src/`).
//!
//! Loads repo-root `campaigns/fixtures/combat_srd/` through production
//! `load_campaign` (engine `0.1.0`), decodes the versioned
//! `{"combat": 1, "op": "init" | "attack" | "end"}` grammar onto the public
//! sim operations, and observes through shared reads plus read-only
//! snapshots. Accepted inputs only: a rejected input aborts playback, so
//! rejections are covered by direct-adapter tests, never replay inputs. The
//! trace carries observed sim results only; expected arithmetic lives in the
//! suite. `support/combat.rs` and every `combat_basic` artifact stay
//! untouched.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str::FromStr;

use crpg_core::{EntityId, Ulid};
use crpg_data::{Document, LoadedCampaign, ObjectKind, SourcePath};
use crpg_rules::Outcome;
use crpg_sim::{perform_action, start_encounter, EncounterSpec, PlacementAndArea, World};
use crpg_testkit::ApplyInput;

/// Campaign identity recorded in the replay envelope (never a path).
pub const SRD_CAMPAIGN_ID: &str = "combat-srd";
/// Campaign version recorded in the replay envelope.
pub const SRD_CAMPAIGN_VERSION: &str = "0.1.0";
/// Engine version recorded in the replay envelope.
pub const SRD_ENGINE_VERSION: &str = "0.1.0";

/// Repo-root `campaigns/fixtures/combat_srd/` directory.
pub fn srd_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("campaigns")
        .join("fixtures")
        .join("combat_srd")
}

/// Repo-root `rulesets/srd-lite/` canonical source directory.
pub fn srd_ruleset_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("rulesets")
        .join("srd-lite")
}

/// Path of the checked-in portable srd replay fixture.
pub fn srd_replay_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("combat_srd.replay")
}

/// Primary Windows srd baseline; unavailable outside its target/profile.
#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    debug_assertions
))]
pub fn srd_target_golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("goldens")
        .join("srd_lite_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden")
}

/// Supported Linux srd baseline; unavailable outside its target/profile.
#[cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu",
    debug_assertions
))]
pub fn srd_target_golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("goldens")
        .join("srd_lite_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden")
}

/// Temporary file under the OS temp dir, unique per process and name.
pub fn srd_temp_file(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("crpg-srd-{}-{name}", std::process::id()));
    path
}

/// Reads a campaign tree to a logical-path map (recurse, skip
/// `.gitattributes`, forward-slash logical paths).
fn read_tree(root: &Path) -> Result<BTreeMap<SourcePath, Vec<u8>>, String> {
    fn walk(
        dir: &Path,
        root: &Path,
        out: &mut BTreeMap<SourcePath, Vec<u8>>,
    ) -> Result<(), String> {
        let entries =
            std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("dir entry {}: {e}", dir.display()))?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|e| format!("file type {}: {e}", path.display()))?;
            if kind.is_dir() {
                walk(&path, root, out)?;
            } else if kind.is_file() {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| format!("non-utf8 file name {}", path.display()))?;
                if name == ".gitattributes" {
                    continue;
                }
                let relative = path
                    .strip_prefix(root)
                    .map_err(|e| format!("strip prefix {}: {e}", path.display()))?
                    .to_string_lossy()
                    .replace('\\', "/");
                let key: SourcePath = relative
                    .parse()
                    .map_err(|e: crpg_data::DataError| format!("logical path {relative}: {e:?}"))?;
                let bytes =
                    std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
                out.insert(key, bytes);
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

fn id(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

/// Checks the 14-id index with kinds, paths, and pointers (T017a §Tests).
fn assert_index(loaded: &LoadedCampaign) -> Result<(), String> {
    if loaded.index.len() != 14 {
        return Err(format!(
            "expected 14 indexed ids, found {}",
            loaded.index.len()
        ));
    }
    let expected: Vec<(Ulid, ObjectKind, &str, &str)> = vec![
        (id(31), ObjectKind::Campaign, "campaign.json", ""),
        (id(32), ObjectKind::World, "worlds/world.json", ""),
        (id(33), ObjectKind::Area, "areas/start/area.json", ""),
        (
            id(34),
            ObjectKind::Placement,
            "areas/start/placements.json",
            "/placements/0",
        ),
        (
            id(35),
            ObjectKind::Placement,
            "areas/start/placements.json",
            "/placements/1",
        ),
        (id(36), ObjectKind::Creature, "creatures/hero.json", ""),
        (id(37), ObjectKind::Creature, "creatures/goblin.json", ""),
        (id(38), ObjectKind::Ruleset, "rulesets/srd-lite.json", ""),
        (id(39), ObjectKind::Ability, "abilities/heavy.json", ""),
        (id(40), ObjectKind::Ability, "abilities/focus.json", ""),
        (
            id(41),
            ObjectKind::OutcomeTable,
            "outcome_tables/heavy_table.json",
            "",
        ),
        (
            id(42),
            ObjectKind::OutcomeTable,
            "outcome_tables/focus_table.json",
            "",
        ),
        (id(43), ObjectKind::Effect, "effects/focusing.json", ""),
        (
            id(44),
            ObjectKind::Encounter,
            "encounters/first-blood.json",
            "",
        ),
    ];
    for (want_id, want_kind, want_path, want_pointer) in expected {
        let entry = loaded
            .index
            .get(&want_id)
            .ok_or_else(|| format!("index missing id {want_id}"))?;
        if entry.kind != want_kind {
            return Err(format!(
                "index id {want_id}: kind {kind:?}",
                kind = entry.kind
            ));
        }
        if entry.path.as_str() != want_path {
            return Err(format!("index id {want_id}: path {}", entry.path.as_str()));
        }
        if entry.pointer != want_pointer {
            return Err(format!("index id {want_id}: pointer {}", entry.pointer));
        }
    }
    for found in loaded.index.keys() {
        let n = found.to_u128();
        if !(31..=44).contains(&n) {
            return Err(format!("index holds unexpected id {found}"));
        }
    }
    Ok(())
}

/// Checks the six campaign combat documents equal the canonical parses.
fn assert_parity(loaded: &LoadedCampaign) -> Result<(), String> {
    let source = read_tree(&srd_ruleset_root())?;
    if source.len() != 6 {
        return Err(format!(
            "canonical source: expected 6 files, found {}",
            source.len()
        ));
    }
    for (logical, bytes) in &source {
        let doc = crpg_data::read_document(bytes)
            .map_err(|e| format!("canonical read {logical}: {e:?}"))?;
        if !loaded.documents.values().any(|candidate| candidate == &doc) {
            return Err(format!("canonical {logical} has no campaign copy"));
        }
    }
    Ok(())
}

/// Owned production content: the 17-file map plus the loaded campaign.
pub struct SrdContent {
    /// The raw 17-file map (asserted count 17).
    pub files: BTreeMap<SourcePath, Vec<u8>>,
    /// The loaded campaign (14-id index, empty validate, parity pinned).
    pub loaded: LoadedCampaign,
}

impl SrdContent {
    /// Loads `combat_srd` through the production data path, in order:
    /// 17-file tree, `load_campaign` (engine `0.1.0`), empty `validate`,
    /// the 14-id index, and canonical-source parity.
    pub fn load() -> Result<Self, String> {
        let files = read_tree(&srd_root())?;
        if files.len() != 17 {
            return Err(format!(
                "read_tree combat_srd: expected 17 files, found {}",
                files.len()
            ));
        }
        let engine = "0.1.0"
            .parse()
            .map_err(|e| format!("engine version parse: {e}"))?;
        let loaded = crpg_data::load_campaign(&files, &engine)
            .map_err(|e| format!("load_campaign combat_srd: {e:?}"))?;
        let findings = crpg_data::validate(&loaded);
        if !findings.is_empty() {
            return Err(format!(
                "validate combat_srd: expected empty, found {}",
                findings.len()
            ));
        }
        assert_index(&loaded)?;
        assert_parity(&loaded)?;
        Ok(Self { files, loaded })
    }
}

/// One accepted attack observed post-call: sim results only, never
/// test-computed expectations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrdTraceEntry {
    /// Replay tick the attack was scheduled at.
    pub tick: u64,
    /// Authored actor placement ULID.
    pub actor: Ulid,
    /// Authored target placement ULID.
    pub target: Ulid,
    /// Authored ability ULID (39 or 40).
    pub ability: Ulid,
    /// Raw die faces in draw order.
    pub faces: Vec<u32>,
    /// Kept sum plus offset.
    pub total: i32,
    /// Actual sim margin, compared against independent arithmetic.
    pub margin: i64,
    /// Table-selected outcome.
    pub outcome: Outcome,
    /// Flat damage applied.
    pub damage: u32,
    /// Whether this action killed the target.
    pub target_died: bool,
    /// Target health after the action.
    pub target_health: u32,
    /// Post-action turn holder; `None` after a terminal action.
    pub active: Option<Ulid>,
}

/// One participant's observed state, keyed by placement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticipantObservation {
    /// Authored placement ULID.
    pub placement: Ulid,
    /// Current health.
    pub health: u32,
    /// Whether dead.
    pub dead: bool,
    /// Pool balances by template identity, identity-sorted.
    pub pools: Vec<(Ulid, u32)>,
    /// Attachments by effect identity, identity-sorted.
    pub attached: Vec<(Ulid, u32)>,
}

/// Observed encounter state around one input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrdStateObservation {
    /// Completed rounds; `None` only before init.
    pub round: Option<u32>,
    /// Turn holder placement; `None` before init and after terminal actions.
    pub active: Option<Ulid>,
    /// Per-participant state, placement-sorted.
    pub participants: Vec<ParticipantObservation>,
}

/// One successful input with its before/after observations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputObservation {
    /// Scheduled zero-based replay tick.
    pub tick: u64,
    /// Index of the input in the replay schedule.
    pub input_index: usize,
    /// Observed state before the input.
    pub before: SrdStateObservation,
    /// Observed state after the input, still before the tick advance.
    pub after: SrdStateObservation,
    /// Full world bytes before the input.
    pub before_bytes: Vec<u8>,
    /// Full world bytes after the input.
    pub after_bytes: Vec<u8>,
    /// Attack trace; `None` for `init`/`end`.
    pub attack: Option<SrdTraceEntry>,
}

/// Observations collected by the `ApplyInput` wrapper (never fed back).
#[derive(Debug, Default)]
pub struct SrdObservations {
    /// One entry per successful input, in playback order.
    pub inputs: Vec<InputObservation>,
    /// Detached clone after the latest successful input.
    pub last_world: Option<World>,
}

/// Reads one placement-keyed state observation from a world. Runtime IDs
/// resolve to placements through the adapter bindings; an unknown ID is an
/// observation failure, never a fabricated identity.
fn observe_state(
    world: &World,
    entities: &BTreeMap<Ulid, EntityId>,
) -> Result<SrdStateObservation, String> {
    let Some(combat) = world.combat() else {
        return Ok(SrdStateObservation {
            round: None,
            active: None,
            participants: Vec::new(),
        });
    };
    let placement_of = |entity: EntityId| {
        entities
            .iter()
            .find(|(_, bound)| **bound == entity)
            .map(|(placement, _)| *placement)
            .ok_or_else(|| "observation failed: entity outside bindings".to_string())
    };
    let active = combat.active.map(placement_of).transpose()?;
    let mut participants = Vec::new();
    for (entity, combatant) in world.combatants().iter() {
        let mut pools = vec![(
            combatant.action_pool().id().0,
            combatant.action_pool().current(),
        )];
        for pool in combatant.extra_pools() {
            pools.push((pool.id().0, pool.current()));
        }
        pools.sort();
        let mut attached: Vec<(Ulid, u32)> = combatant
            .attached()
            .iter()
            .map(|entry| (entry.effect, entry.expires_round))
            .collect();
        attached.sort();
        participants.push(ParticipantObservation {
            placement: placement_of(entity)?,
            health: combatant.health(),
            dead: combatant.dead(),
            pools,
            attached,
        });
    }
    participants.sort_by_key(|entry| entry.placement);
    Ok(SrdStateObservation {
        round: Some(combat.round),
        active,
        participants,
    })
}

/// Tests-only srd adapter: owns content, binds placement identities.
pub struct SrdAdapter {
    /// Owned production documents.
    pub content: SrdContent,
    /// Authored placement ULID to runtime entity, bound at init.
    pub entities: BTreeMap<Ulid, EntityId>,
    /// Whether `init` has been accepted.
    pub initialized: bool,
}

impl SrdAdapter {
    /// Wraps loaded production content in an uninitialized adapter.
    pub fn new(content: SrdContent) -> Self {
        Self {
            content,
            entities: BTreeMap::new(),
            initialized: false,
        }
    }

    /// Rebuilds placement bindings from a live world's combatants without
    /// publishing anything: every encounter participant must resolve to a
    /// live combatant and every combatant to a participant. Bindings only,
    /// never edits to world or replay semantics.
    pub fn rebind(content: SrdContent, world: &World) -> Result<Self, String> {
        let combat = world
            .combat()
            .ok_or_else(|| "rebind failed: no active encounter".to_string())?;
        let encounter_id = combat.definition.encounter;
        let encounter = content
            .loaded
            .documents
            .values()
            .find_map(|doc| match doc {
                Document::Encounter(value) if value.id == encounter_id => Some(value),
                _ => None,
            })
            .ok_or_else(|| format!("rebind failed: unknown encounter {encounter_id}"))?;
        let mut entities = BTreeMap::new();
        for participant in &encounter.participants {
            let (entity, _) = world
                .combatants()
                .iter()
                .find(|(_, combatant)| combatant.placement() == participant.placement)
                .ok_or_else(|| {
                    format!(
                        "rebind failed: placement {} without combatant",
                        participant.placement
                    )
                })?;
            entities.insert(participant.placement, entity);
        }
        for (entity, combatant) in world.combatants().iter() {
            if !entities.values().any(|bound| *bound == entity) {
                return Err(format!(
                    "rebind failed: combatant outside participants: {}",
                    combatant.placement()
                ));
            }
        }
        Ok(Self {
            content,
            entities,
            initialized: true,
        })
    }

    /// Decodes one payload and executes only the public sim operation,
    /// returning the optional trace (`init`/`end` record none). The wrapper
    /// observes successful calls.
    pub fn apply(
        &mut self,
        world: &mut World,
        payload: &serde_json::Value,
    ) -> Result<Option<SrdTraceEntry>, String> {
        let object = payload
            .as_object()
            .ok_or_else(|| "malformed combat payload: expected object".to_string())?;
        if object.get("combat").and_then(|v| v.as_u64()) != Some(1) {
            return Err("unsupported combat payload version".to_string());
        }
        let op = object
            .get("op")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if op != "init" && op != "attack" && op != "end" {
            return Err("unknown combat op".to_string());
        }
        let allowed: &[&str] = if op == "init" {
            &["combat", "op", "encounter"]
        } else if op == "attack" {
            &["combat", "op", "actor", "ability", "target"]
        } else {
            &["combat", "op", "actor"]
        };
        let mut extra: Vec<&String> = object
            .keys()
            .filter(|key| !allowed.contains(&key.as_str()))
            .collect();
        extra.sort();
        if let Some(field) = extra.first() {
            return Err(format!("malformed combat {op}: unknown field {field}"));
        }
        if op == "init" {
            let encounter_id = decode_ulid(object, op, "encounter")?;
            return self.apply_init(world, encounter_id);
        }
        let actor_id = decode_ulid(object, op, "actor")?;
        if op == "end" {
            return self.apply_end(world, actor_id);
        }
        let ability_id = decode_ulid(object, op, "ability")?;
        let target_id = decode_ulid(object, op, "target")?;
        self.apply_attack(world, actor_id, ability_id, target_id)
    }

    fn apply_init(
        &mut self,
        world: &mut World,
        encounter_id: Ulid,
    ) -> Result<Option<SrdTraceEntry>, String> {
        if self.initialized {
            return Err("duplicate combat init".to_string());
        }
        let encounter = self
            .content
            .loaded
            .documents
            .values()
            .find_map(|doc| match doc {
                Document::Encounter(value) if value.id == encounter_id => Some(value),
                _ => None,
            })
            .ok_or_else(|| format!("unknown encounter {encounter_id}"))?;
        let ruleset = self
            .content
            .loaded
            .documents
            .values()
            .find_map(|doc| match doc {
                Document::Ruleset(value) if value.id == encounter.ruleset => Some(value),
                _ => None,
            })
            .ok_or_else(|| format!("init failed: missing ruleset {}", encounter.ruleset))?;
        let mut abilities = BTreeMap::new();
        for doc in self.content.loaded.documents.values() {
            if let Document::Ability(value) = doc {
                abilities.insert(value.id, value);
            }
        }
        for listed in &ruleset.abilities {
            if !abilities.contains_key(listed) {
                return Err(format!("init failed: missing ability {listed}"));
            }
        }
        let mut outcome_tables = BTreeMap::new();
        for doc in self.content.loaded.documents.values() {
            if let Document::OutcomeTable(value) = doc {
                outcome_tables.insert(value.id, value);
            }
        }
        let mut placements: BTreeMap<Ulid, PlacementAndArea<'_>> = BTreeMap::new();
        for doc in self.content.loaded.documents.values() {
            if let Document::Placements(value) = doc {
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
        }
        let mut creatures = BTreeMap::new();
        for doc in self.content.loaded.documents.values() {
            if let Document::Creature(value) = doc {
                creatures.insert(value.id, value);
            }
        }
        let mut effects = BTreeMap::new();
        for doc in self.content.loaded.documents.values() {
            if let Document::Effect(value) = doc {
                effects.insert(value.id, value);
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
        start_encounter(world, &spec).map_err(|e| format!("init failed: {e}"))?;
        let bound: Vec<(EntityId, Ulid)> = world
            .combatants()
            .iter()
            .map(|(entity, combatant)| (entity, combatant.placement()))
            .collect();
        if bound.len() != encounter.participants.len() {
            return Err(format!(
                "init failed: placement binding mismatch ({} combatants, {} participants)",
                bound.len(),
                encounter.participants.len()
            ));
        }
        for (index, (entity, placement)) in bound.iter().enumerate() {
            let expected = encounter.participants[index].placement;
            if *placement != expected {
                return Err(format!(
                    "init failed: placement binding mismatch at index {index} (expected {expected}, found {placement})"
                ));
            }
            self.entities.insert(expected, *entity);
        }
        let definition_ability = world
            .combat()
            .ok_or_else(|| "init failed: missing encounter state".to_string())?
            .definition
            .ability;
        let expected_ability = ruleset
            .abilities
            .first()
            .ok_or_else(|| "init failed: ruleset lists no ability".to_string())?;
        if definition_ability != *expected_ability {
            return Err(format!(
                "init failed: ability binding mismatch (expected {expected_ability}, found {definition_ability})"
            ));
        }
        self.initialized = true;
        Ok(None)
    }

    fn apply_attack(
        &mut self,
        world: &mut World,
        actor_placement: Ulid,
        ability: Ulid,
        target_placement: Ulid,
    ) -> Result<Option<SrdTraceEntry>, String> {
        if !self.initialized {
            return Err("combat not initialized".to_string());
        }
        let actor = self
            .entities
            .get(&actor_placement)
            .copied()
            .ok_or_else(|| format!("unknown combat identity actor: {actor_placement}"))?;
        let target = self
            .entities
            .get(&target_placement)
            .copied()
            .ok_or_else(|| format!("unknown combat identity target: {target_placement}"))?;
        let tick = world.tick().get();
        let outcome = perform_action(
            world,
            &crpg_sim::CombatAction::UseAbility {
                actor,
                ability,
                target,
            },
        )
        .map_err(|e| format!("attack failed: {e}"))?
        .ok_or_else(|| "attack failed: missing outcome".to_string())?;
        let faces: Vec<u32> = outcome.roll.dice.iter().map(|die| die.value).collect();
        let target_health = world
            .combatants()
            .get(target)
            .ok_or_else(|| "attack failed: missing target after action".to_string())?
            .health();
        let active = world
            .combat()
            .ok_or_else(|| "attack failed: missing encounter state".to_string())?
            .active;
        let placement_of = |entity: EntityId| {
            self.entities
                .iter()
                .find(|(_, bound)| **bound == entity)
                .map(|(placement, _)| *placement)
                .ok_or_else(|| "attack failed: turn outside bindings".to_string())
        };
        let active = active.map(placement_of).transpose()?;
        if active.is_none() && !outcome.target_died {
            return Err("attack failed: terminal turn without death".to_string());
        }
        Ok(Some(SrdTraceEntry {
            tick,
            actor: actor_placement,
            target: target_placement,
            ability,
            faces,
            total: outcome.roll.total,
            margin: outcome.margin,
            outcome: outcome.outcome,
            damage: outcome.damage,
            target_died: outcome.target_died,
            target_health,
            active,
        }))
    }

    fn apply_end(
        &mut self,
        world: &mut World,
        actor_placement: Ulid,
    ) -> Result<Option<SrdTraceEntry>, String> {
        if !self.initialized {
            return Err("combat not initialized".to_string());
        }
        let actor = self
            .entities
            .get(&actor_placement)
            .copied()
            .ok_or_else(|| format!("unknown combat identity actor: {actor_placement}"))?;
        match perform_action(world, &crpg_sim::CombatAction::EndTurn { actor })
            .map_err(|e| format!("end failed: {e}"))?
        {
            None => Ok(None),
            Some(_) => Err("end failed: unexpected outcome".to_string()),
        }
    }
}

fn decode_ulid(
    object: &serde_json::Map<String, serde_json::Value>,
    op: &str,
    field: &str,
) -> Result<Ulid, String> {
    let text = object
        .get(field)
        .and_then(|value| value.as_str())
        .ok_or_else(|| format!("malformed combat {op}: {field} must be a ULID string"))?;
    Ulid::from_str(text).map_err(|_| format!("malformed combat {op}: invalid {field}"))
}

/// Builds the `'static` caller-owned apply closure plus its observation
/// handle. The closure owns the adapter and a clone of the handle, so it
/// satisfies the existing `ApplyInput` alias without changing it or
/// borrowing a stack-local adapter. Every successful input records its
/// before/after bytes and placement-keyed state; snapshots are always
/// post-input and pre-tick.
pub fn srd_apply(content: SrdContent) -> (ApplyInput, Rc<RefCell<SrdObservations>>) {
    let observations = Rc::new(RefCell::new(SrdObservations {
        inputs: Vec::new(),
        last_world: None,
    }));
    let mut adapter = SrdAdapter::new(content);
    let handle = Rc::clone(&observations);
    let apply: ApplyInput = Box::new(move |world: &mut World, payload: &serde_json::Value| {
        // During playback the world tick still names the scheduled tick:
        // inputs apply before the harness `tick` advance.
        let tick = world.tick().get();
        let before_bytes =
            serde_json::to_vec(&*world).map_err(|e| format!("snapshot serialize: {e}"))?;
        let before = observe_state(world, &adapter.entities)
            .map_err(|e| format!("observation failed: {e}"))?;
        let entry = adapter.apply(world, payload)?;
        let after_bytes =
            serde_json::to_vec(&*world).map_err(|e| format!("snapshot serialize: {e}"))?;
        let after = observe_state(world, &adapter.entities)
            .map_err(|e| format!("observation failed: {e}"))?;
        {
            let mut guard = handle.borrow_mut();
            let input_index = guard.inputs.len();
            guard.inputs.push(InputObservation {
                tick,
                input_index,
                before,
                after,
                before_bytes,
                after_bytes,
                attack: entry,
            });
            guard.last_world = Some(world.clone());
        }
        Ok(())
    });
    (apply, observations)
}
