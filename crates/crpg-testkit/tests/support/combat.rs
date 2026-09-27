//! Tests-only combat adapter for T016c (never ships, never in `src/`).
//!
//! Loads repo-root `campaigns/fixtures/combat_basic/` through production
//! `load_campaign` (engine `0.1.0`), decodes the versioned
//! `{"combat": 1, "op": "init" | "attack"}` grammar onto the public sim
//! operations, and observes through shared reads plus read-only snapshots.
//! The generic harness and replay stay payload-agnostic; T016d writes its
//! own CLI-owned adapter against the same grammar.

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
pub const COMBAT_CAMPAIGN_ID: &str = "combat-basic";
/// Campaign version recorded in the replay envelope.
pub const COMBAT_CAMPAIGN_VERSION: &str = "0.1.0";
/// Engine version recorded in the replay envelope.
pub const COMBAT_ENGINE_VERSION: &str = "0.1.0";

/// Repo-root `campaigns/fixtures/combat_basic/` directory.
pub fn combat_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("campaigns")
        .join("fixtures")
        .join("combat_basic")
}

/// Repo-root `rulesets/minimal-d6/` canonical source directory.
pub fn ruleset_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("rulesets")
        .join("minimal-d6")
}

/// Path of the checked-in portable combat replay fixture.
pub fn combat_replay_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("combat_basic.replay")
}

/// Primary Windows combat baseline; unavailable outside its target/profile.
#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    debug_assertions
))]
pub fn combat_target_golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("goldens")
        .join("combat_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden")
}

/// Supported Linux combat baseline; unavailable outside its target/profile.
#[cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu",
    debug_assertions
))]
pub fn combat_target_golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("goldens")
        .join("combat_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden")
}

/// Temporary file under the OS temp dir, unique per process and name.
pub fn combat_temp_file(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("crpg-combat-{}-{name}", std::process::id()));
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

/// Checks the 11-id index with kinds, paths, and pointers (T016a §Tests).
fn assert_index(loaded: &LoadedCampaign) -> Result<(), String> {
    if loaded.index.len() != 11 {
        return Err(format!(
            "expected 11 indexed ids, found {}",
            loaded.index.len()
        ));
    }
    let expected: Vec<(Ulid, ObjectKind, &str, &str)> = vec![
        (id(11), ObjectKind::Campaign, "campaign.json", ""),
        (id(12), ObjectKind::World, "worlds/world.json", ""),
        (id(13), ObjectKind::Area, "areas/start/area.json", ""),
        (
            id(14),
            ObjectKind::Placement,
            "areas/start/placements.json",
            "/placements/0",
        ),
        (
            id(15),
            ObjectKind::Placement,
            "areas/start/placements.json",
            "/placements/1",
        ),
        (id(16), ObjectKind::Creature, "creatures/hero.json", ""),
        (id(17), ObjectKind::Creature, "creatures/goblin.json", ""),
        (id(18), ObjectKind::Ruleset, "rulesets/minimal-d6.json", ""),
        (id(19), ObjectKind::Ability, "abilities/strike.json", ""),
        (
            id(20),
            ObjectKind::OutcomeTable,
            "outcome_tables/roll-under.json",
            "",
        ),
        (
            id(21),
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
        if !(11..=21).contains(&n) {
            return Err(format!("index holds unexpected id {found}"));
        }
    }
    Ok(())
}

/// Checks the three campaign combat documents equal the canonical parses.
fn assert_parity(loaded: &LoadedCampaign) -> Result<(), String> {
    let source = read_tree(&ruleset_root())?;
    if source.len() != 3 {
        return Err(format!(
            "canonical source: expected 3 files, found {}",
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
    let by_id = |n: u128| {
        loaded
            .documents
            .values()
            .find(|doc| {
                matches!(doc, Document::Ruleset(v) if v.id == id(n))
                    || matches!(doc, Document::Ability(v) if v.id == id(n))
                    || matches!(doc, Document::OutcomeTable(v) if v.id == id(n))
            })
            .cloned()
            .ok_or_else(|| format!("campaign id {n} missing"))
    };
    let ruleset_key: SourcePath = "ruleset.json"
        .parse()
        .map_err(|e: crpg_data::DataError| format!("source key: {e:?}"))?;
    let strike_key: SourcePath = "strike.json"
        .parse()
        .map_err(|e: crpg_data::DataError| format!("source key: {e:?}"))?;
    let table_key: SourcePath = "roll_under.json"
        .parse()
        .map_err(|e: crpg_data::DataError| format!("source key: {e:?}"))?;
    let expect_ruleset = crpg_data::read_document(
        source
            .get(&ruleset_key)
            .ok_or_else(|| "canonical ruleset.json missing".to_string())?,
    )
    .map_err(|e| format!("canonical ruleset read: {e:?}"))?;
    let expect_strike = crpg_data::read_document(
        source
            .get(&strike_key)
            .ok_or_else(|| "canonical strike.json missing".to_string())?,
    )
    .map_err(|e| format!("canonical strike read: {e:?}"))?;
    let expect_table = crpg_data::read_document(
        source
            .get(&table_key)
            .ok_or_else(|| "canonical roll_under.json missing".to_string())?,
    )
    .map_err(|e| format!("canonical table read: {e:?}"))?;
    if by_id(18)? != expect_ruleset {
        return Err("ruleset 18 diverges from canonical source".to_string());
    }
    if by_id(19)? != expect_strike {
        return Err("ability 19 diverges from canonical source".to_string());
    }
    if by_id(20)? != expect_table {
        return Err("outcome table 20 diverges from canonical source".to_string());
    }
    Ok(())
}

/// Owned production content: the 14-file map plus the loaded campaign.
pub struct CombatContent {
    /// The raw 14-file map (asserted count 14).
    pub files: BTreeMap<SourcePath, Vec<u8>>,
    /// The loaded campaign (11-id index, empty validate, parity pinned).
    pub loaded: LoadedCampaign,
}

impl CombatContent {
    /// Loads `combat_basic` through the production data path, in order:
    /// 14-file tree, `load_campaign` (engine `0.1.0`), empty `validate`,
    /// the 11-id index, and canonical-source parity.
    pub fn load() -> Result<Self, String> {
        let files = read_tree(&combat_root())?;
        if files.len() != 14 {
            return Err(format!(
                "read_tree combat_basic: expected 14 files, found {}",
                files.len()
            ));
        }
        let engine = "0.1.0"
            .parse()
            .map_err(|e| format!("engine version parse: {e}"))?;
        let loaded = crpg_data::load_campaign(&files, &engine)
            .map_err(|e| format!("load_campaign combat_basic: {e:?}"))?;
        let findings = crpg_data::validate(&loaded);
        if !findings.is_empty() {
            return Err(format!(
                "validate combat_basic: expected empty, found {}",
                findings.len()
            ));
        }
        assert_index(&loaded)?;
        assert_parity(&loaded)?;
        Ok(Self { files, loaded })
    }
}

/// One accepted attack observed post-call (never an instruction).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceEntry {
    /// Replay tick the attack was scheduled at.
    pub tick: u64,
    /// Authored actor placement ULID.
    pub actor: Ulid,
    /// Authored target placement ULID.
    pub target: Ulid,
    /// Authored ability ULID.
    pub ability: Ulid,
    /// Raw die faces in draw order.
    pub faces: Vec<u32>,
    /// Kept sum plus offset.
    pub total: i32,
    /// Kernel margin (`total - might`).
    pub margin: i64,
    /// Table-selected outcome.
    pub outcome: Outcome,
    /// Flat damage applied.
    pub damage: u32,
    /// Whether this action killed the target.
    pub target_died: bool,
    /// Target health after the action.
    pub target_health: u32,
    /// Actor pool balance after the action.
    pub actor_pool: u32,
}

/// Observations collected by the `ApplyInput` wrapper (never fed back).
#[derive(Debug, Default)]
pub struct Observations {
    /// One entry per accepted attack, in playback order.
    pub trace: Vec<TraceEntry>,
    /// Snapshot bytes after every accepted attack.
    pub snapshots: Vec<Vec<u8>>,
    /// Detached clone after the latest successful input.
    pub last_world: Option<World>,
}

/// Tests-only combat adapter: owns content, binds placement identities.
pub struct CombatAdapter {
    /// Owned production documents.
    pub content: CombatContent,
    /// Authored placement ULID to runtime entity, bound at init.
    pub entities: BTreeMap<Ulid, EntityId>,
    /// Whether `init` has been accepted.
    pub initialized: bool,
}

impl CombatAdapter {
    /// Wraps loaded production content in an uninitialized adapter.
    pub fn new(content: CombatContent) -> Self {
        Self {
            content,
            entities: BTreeMap::new(),
            initialized: false,
        }
    }

    /// Decodes one payload and executes only the public sim operation,
    /// returning the optional trace. The wrapper observes successful calls.
    pub fn apply(
        &mut self,
        world: &mut World,
        payload: &serde_json::Value,
    ) -> Result<Option<TraceEntry>, String> {
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
        if op != "init" && op != "attack" {
            return Err("unknown combat op".to_string());
        }
        let allowed: &[&str] = if op == "init" {
            &["combat", "op", "encounter"]
        } else {
            &["combat", "op", "actor", "ability", "target"]
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
        let ability_id = decode_ulid(object, op, "ability")?;
        let target_id = decode_ulid(object, op, "target")?;
        self.apply_attack(world, actor_id, ability_id, target_id)
    }

    fn apply_init(
        &mut self,
        world: &mut World,
        encounter_id: Ulid,
    ) -> Result<Option<TraceEntry>, String> {
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
    ) -> Result<Option<TraceEntry>, String> {
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
        let actor_pool = world
            .combatants()
            .get(actor)
            .ok_or_else(|| "attack failed: missing actor after action".to_string())?
            .action_pool()
            .current();
        Ok(Some(TraceEntry {
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
            actor_pool,
        }))
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
/// borrowing a stack-local adapter.
pub fn combat_apply(content: CombatContent) -> (ApplyInput, Rc<RefCell<Observations>>) {
    let observations = Rc::new(RefCell::new(Observations {
        trace: Vec::new(),
        snapshots: Vec::new(),
        last_world: None,
    }));
    let mut adapter = CombatAdapter::new(content);
    let handle = Rc::clone(&observations);
    let apply: ApplyInput = Box::new(move |world: &mut World, payload: &serde_json::Value| {
        let entry = adapter.apply(world, payload)?;
        if let Some(trace) = entry {
            let bytes =
                serde_json::to_vec(world).map_err(|e| format!("snapshot serialize: {e}"))?;
            {
                let mut guard = handle.borrow_mut();
                guard.trace.push(trace);
                guard.snapshots.push(bytes);
                guard.last_world = Some(world.clone());
            }
        } else {
            let mut guard = handle.borrow_mut();
            guard.last_world = Some(world.clone());
        }
        Ok(())
    });
    (apply, observations)
}
