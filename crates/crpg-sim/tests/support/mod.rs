//! Shared combat test fixtures (T016b).
//!
//! Hand-built authored documents with neutral vocabulary, parsed through
//! the production data shapes. Parsing (rather than struct literals) keeps
//! the fixtures honest about the wire contract and avoids naming a version
//! type this crate does not depend on. No minimal-ruleset identifiers,
//! attribute names, or numeric policy appear here as engine knowledge:
//! they are fixture data consumed read-only by the adapter under test.

use std::collections::BTreeMap;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_data::{
    Ability, Creature, Effect, Encounter, EncounterParticipant,
    OutcomeTable as AuthoredOutcomeTable, Placement, Ruleset,
};
use crpg_sim::{start_encounter, EncounterSpec, PlacementAndArea, World};
use serde_json::json;

/// Whole fixed-point raw value for an integer stat.
pub fn fx_raw(value: i32) -> i32 {
    Fx16_16::from_int(value).to_raw()
}

/// Canonical ULID text for fixture identities.
pub fn uid(value: u128) -> String {
    Ulid::from_u128(value).to_string()
}

/// Authored fixture documents with neutral vocabulary.
pub struct Fixture {
    /// Encounter 101: participants 111 (initiative 10) then 112 (9).
    pub encounter: Encounter,
    /// Ruleset 102: stats alpha/beta/gamma/health, pool max 1.
    pub ruleset: Ruleset,
    /// Ability 103 keyed by identity.
    pub abilities: BTreeMap<Ulid, Ability>,
    /// Outcome table 104 keyed by identity.
    pub tables: BTreeMap<Ulid, AuthoredOutcomeTable>,
    /// Effect documents by identity (empty for the single-ability fixture).
    pub effects: BTreeMap<Ulid, Effect>,
    /// Placements 111/112 with owning areas.
    pub placements: BTreeMap<Ulid, (Placement, Ulid)>,
    /// Creatures 121/122 keyed by identity.
    pub creatures: BTreeMap<Ulid, Creature>,
    /// Identity shortcuts.
    pub ability: Ulid,
    /// Identity shortcuts.
    pub area: Ulid,
    /// Identity shortcuts.
    pub placement_a: Ulid,
    /// Identity shortcuts.
    pub placement_b: Ulid,
    /// Identity shortcuts.
    pub creature_a: Ulid,
    /// Identity shortcuts.
    pub creature_b: Ulid,
}

/// Reference maps assembled from one fixture for [`EncounterSpec`].
pub struct SpecBundle<'a> {
    /// Ability references by identity.
    pub abilities: BTreeMap<Ulid, &'a Ability>,
    /// Table references by identity.
    pub tables: BTreeMap<Ulid, &'a AuthoredOutcomeTable>,
    /// Placements with areas by placement identity.
    pub placements: BTreeMap<Ulid, PlacementAndArea<'a>>,
    /// Creature references by identity.
    pub creatures: BTreeMap<Ulid, &'a Creature>,
    /// Effect references by identity.
    pub effects: BTreeMap<Ulid, &'a Effect>,
}

impl Fixture {
    /// The standard two-combatant fixture: creature A (alpha 8, health 10)
    /// is listed before creature B (alpha 6, health 6), but B's lower
    /// initiative (9 vs 10) acts first, on a two-band roll-under table
    /// with flat damage `{success: 2, failure: 0}` and cost 1.
    pub fn standard() -> Self {
        let ability = Ulid::from_u128(103);
        let area = Ulid::from_u128(131);
        let placement_a = Ulid::from_u128(111);
        let placement_b = Ulid::from_u128(112);
        let creature_a = Ulid::from_u128(121);
        let creature_b = Ulid::from_u128(122);

        let ruleset: Ruleset = serde_json::from_value(json!({
            "id": uid(102),
            "slug": "probe-ruleset",
            "name": "probe.ruleset",
            "package": "probe-pack",
            "version": "0.1.0",
            "stats": [
                {"name": "alpha", "kind": "int"},
                {"name": "beta", "kind": "int"},
                {"name": "gamma", "kind": "int"},
                {"name": "health", "kind": "int"}
            ],
            "health_stat": "health",
            "attributes": ["alpha", "beta", "gamma"],
            "pools": [
                {
                    "id": uid(105),
                    "max": 1,
                    "refresh": {"type": "on_turn_start"}
                }
            ],
            "abilities": [uid(103)]
        }))
        .expect("the standard ruleset is valid");

        let attack: Ability = serde_json::from_value(json!({
            "id": uid(103),
            "slug": "probe-attack",
            "name": "probe.attack",
            "dice": "2d6",
            "attribute": "alpha",
            "outcome_table": uid(104),
            "damage": [
                {"outcome": {"type": "success"}, "amount": 2},
                {"outcome": {"type": "failure"}, "amount": 0}
            ],
            "cost": 1,
            "extra_costs": [],
            "ends_turn": true,
            "defense": {"type": "actor_attribute"},
            "requires_target": true,
            "allow_self_target": false
        }))
        .expect("the standard ability is valid");

        let table: AuthoredOutcomeTable = serde_json::from_value(json!({
            "id": uid(104),
            "slug": "probe-table",
            "name": "probe.table",
            "bands": [
                {"min_margin": i64::MIN, "outcome": {"type": "success"}},
                {"min_margin": 1, "outcome": {"type": "failure"}}
            ],
            "natural_rules": []
        }))
        .expect("the standard table is valid");

        let encounter: Encounter = serde_json::from_value(json!({
            "id": uid(101),
            "slug": "probe-encounter",
            "name": "probe.encounter",
            "ruleset": uid(102),
            "participants": [
                {"placement": uid(111), "initiative": 10},
                {"placement": uid(112), "initiative": 9}
            ]
        }))
        .expect("the standard encounter is valid");

        let placement_doc_a: Placement = serde_json::from_value(json!({
            "id": uid(111),
            "slug": "probe-a",
            "name": "probe.a",
            "prefab": uid(121),
            "transform": {
                "position": [0, 0, 0],
                "rotation": [0, 0, 0],
                "scale": [65536, 65536, 65536]
            },
            "overrides": {}
        }))
        .expect("placement A is valid");
        let placement_doc_b: Placement = serde_json::from_value(json!({
            "id": uid(112),
            "slug": "probe-b",
            "name": "probe.b",
            "prefab": uid(122),
            "transform": {
                "position": [0, 0, 0],
                "rotation": [0, 0, 0],
                "scale": [65536, 65536, 65536]
            },
            "overrides": {}
        }))
        .expect("placement B is valid");

        let hero: Creature = serde_json::from_value(json!({
            "id": uid(121),
            "slug": "probe-creature-a",
            "name": "probe.creature-a",
            "stats": {
                "alpha": fx_raw(8),
                "beta": fx_raw(6),
                "gamma": fx_raw(7),
                "health": fx_raw(10)
            },
            "tags": [],
            "faction": null,
            "inventory": []
        }))
        .expect("creature A is valid");
        let rival: Creature = serde_json::from_value(json!({
            "id": uid(122),
            "slug": "probe-creature-b",
            "name": "probe.creature-b",
            "stats": {
                "alpha": fx_raw(6),
                "beta": fx_raw(7),
                "gamma": fx_raw(5),
                "health": fx_raw(6)
            },
            "tags": [],
            "faction": null,
            "inventory": []
        }))
        .expect("creature B is valid");

        let mut abilities = BTreeMap::new();
        abilities.insert(ability, attack);
        let mut tables = BTreeMap::new();
        tables.insert(Ulid::from_u128(104), table);
        let mut placements = BTreeMap::new();
        placements.insert(placement_a, (placement_doc_a, area));
        placements.insert(placement_b, (placement_doc_b, area));
        let mut creatures = BTreeMap::new();
        creatures.insert(creature_a, hero);
        creatures.insert(creature_b, rival);

        Self {
            encounter,
            ruleset,
            abilities,
            tables,
            effects: BTreeMap::new(),
            placements,
            creatures,
            ability,
            area,
            placement_a,
            placement_b,
            creature_a,
            creature_b,
        }
    }

    /// Assembles the reference maps one [`EncounterSpec`] borrows.
    pub fn bundle(&self) -> SpecBundle<'_> {
        let mut abilities = BTreeMap::new();
        for (id, ability) in &self.abilities {
            abilities.insert(*id, ability);
        }
        let mut tables = BTreeMap::new();
        for (id, table) in &self.tables {
            tables.insert(*id, table);
        }
        let mut placements = BTreeMap::new();
        for (id, (placement, area)) in &self.placements {
            placements.insert(
                *id,
                PlacementAndArea {
                    placement,
                    area: *area,
                },
            );
        }
        let mut creatures = BTreeMap::new();
        for (id, creature) in &self.creatures {
            creatures.insert(*id, creature);
        }
        let mut effects = BTreeMap::new();
        for (id, effect) in &self.effects {
            effects.insert(*id, effect);
        }
        SpecBundle {
            abilities,
            tables,
            placements,
            creatures,
            effects,
        }
    }

    /// Starts the standard encounter in a fresh world, returning the world
    /// with combatant entities in authored participant order.
    pub fn start(&self, seed: u64) -> (World, Vec<EntityId>) {
        let bundle = self.bundle();
        let spec = EncounterSpec {
            encounter: &self.encounter,
            ruleset: &self.ruleset,
            abilities: &bundle.abilities,
            outcome_tables: &bundle.tables,
            placements: &bundle.placements,
            creatures: &bundle.creatures,
            effects: &bundle.effects,
        };
        let mut world = World::new(seed);
        start_encounter(&mut world, &spec).expect("the standard fixture starts");
        let ids: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
        (world, ids)
    }

    /// Sets both creatures' checked attribute to `value`.
    pub fn set_alpha(&mut self, alpha_a: i32, alpha_b: i32) {
        self.creatures
            .get_mut(&self.creature_a)
            .expect("creature A exists")
            .stats
            .insert("alpha".to_owned(), Fx16_16::from_int(alpha_a));
        self.creatures
            .get_mut(&self.creature_b)
            .expect("creature B exists")
            .stats
            .insert("alpha".to_owned(), Fx16_16::from_int(alpha_b));
    }

    /// The three-combatant fixture: `standard` plus creature C (an A clone
    /// with fresh identities, initiative 8), so C acts first and
    /// active-removal paths have two survivors to pass to.
    pub fn trio() -> Self {
        let mut fixture = Self::standard();
        let placement_c = Ulid::from_u128(113);
        let creature_c = Ulid::from_u128(123);
        let mut document = fixture
            .placements
            .get(&fixture.placement_a)
            .expect("placement A exists")
            .0
            .clone();
        document.id = placement_c;
        document.slug = "probe-c".to_owned();
        document.name = "probe.c".to_owned();
        document.prefab = creature_c;
        fixture
            .placements
            .insert(placement_c, (document, fixture.area));
        let mut creature = fixture
            .creatures
            .get(&fixture.creature_a)
            .expect("creature A exists")
            .clone();
        creature.id = creature_c;
        creature.slug = "probe-creature-c".to_owned();
        creature.name = "probe.creature-c".to_owned();
        fixture.creatures.insert(creature_c, creature);
        fixture.encounter.participants.push(EncounterParticipant {
            placement: placement_c,
            initiative: 8,
        });
        fixture
    }
}

/// Real-time fixtures and probes (T138, ADR-0029). Kept apart so that test
/// binaries that never use them stay free of dead-code warnings.
pub mod rt {
    #![allow(dead_code)]

    use std::collections::BTreeSet;

    use crpg_core::{EntityId, Ulid};
    use crpg_data::{AbilityCost, ActionPoolTemplate, RefreshWire, TickPeriod};
    use crpg_sim::{
        start_encounter_in_mode, CombatAction, CombatError, EncounterMode, EncounterSpec,
        HistoryError, HistoryWorld, World,
    };

    use super::{uid, Fixture};

    /// The standard two-combatant fixture with `round_ticks` declared.
    pub fn capable(round_ticks: u32) -> Fixture {
        let mut fixture = Fixture::standard();
        fixture.ruleset.round_ticks = Some(round_ticks);
        fixture
    }

    /// The three-combatant fixture with `round_ticks` declared.
    pub fn capable_trio(round_ticks: u32) -> Fixture {
        let mut fixture = Fixture::trio();
        fixture.ruleset.round_ticks = Some(round_ticks);
        fixture
    }

    /// Makes every roll succeed and every success deal `damage`.
    pub fn make_lethal(fixture: &mut Fixture, damage: u32) {
        fixture.set_alpha(100, 100);
        let id = fixture.ability;
        let ability = fixture.abilities.get_mut(&id).expect("ability");
        for entry in &mut ability.damage {
            if entry.amount > 0 {
                entry.amount = damage;
            }
        }
    }

    /// Edits the standard ability in place.
    pub fn edit_ability(fixture: &mut Fixture, edit: impl FnOnce(&mut crpg_data::Ability)) {
        let id = fixture.ability;
        edit(fixture.abilities.get_mut(&id).expect("ability"));
    }

    /// Replaces the ruleset's pools with the primary pool `105` (max
    /// `primary_max`, `OnTurnStart`) followed by one extra pool per period,
    /// ids `106..`, each max 3 and `OnTick(period)`, and makes the standard
    /// ability cost 3 from every extra pool (and 1 from the primary).
    pub fn tick_pools(fixture: &mut Fixture, primary_max: u32, periods: &[u64]) {
        let mut pools = vec![ActionPoolTemplate {
            id: Ulid::from_u128(105),
            max: primary_max,
            refresh: RefreshWire::OnTurnStart,
        }];
        let mut costs = Vec::new();
        for (index, period) in periods.iter().enumerate() {
            let id = Ulid::from_u128(106 + index as u128);
            pools.push(ActionPoolTemplate {
                id,
                max: 3,
                refresh: RefreshWire::OnTick(TickPeriod(*period)),
            });
            costs.push(AbilityCost {
                pool: id,
                amount: 3,
            });
        }
        fixture.ruleset.pools = pools;
        edit_ability(fixture, |ability| ability.extra_costs = costs);
    }

    impl Fixture {
        /// Starts the fixture's encounter in `mode`, returning the world and
        /// the combatant ids in authored participant order.
        pub fn start_mode(
            &self,
            seed: u64,
            mode: EncounterMode,
        ) -> Result<(World, Vec<EntityId>), CombatError> {
            let bundle = self.bundle();
            let spec = EncounterSpec {
                encounter: &self.encounter,
                ruleset: &self.ruleset,
                abilities: &bundle.abilities,
                outcome_tables: &bundle.tables,
                placements: &bundle.placements,
                creatures: &bundle.creatures,
                effects: &bundle.effects,
            };
            let mut world = World::new(seed);
            start_encounter_in_mode(&mut world, &spec, mode)?;
            let ids: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
            Ok((world, ids))
        }

        /// Starts in `mode` and unwraps.
        pub fn start_rt(&self, seed: u64, mode: EncounterMode) -> (World, Vec<EntityId>) {
            self.start_mode(seed, mode)
                .expect("the fixture starts in the requested mode")
        }
    }

    impl Fixture {
        /// Runs `body` with the borrowed [`EncounterSpec`] of this fixture.
        pub fn with_spec<R>(&self, body: impl FnOnce(&EncounterSpec<'_>) -> R) -> R {
            let bundle = self.bundle();
            let spec = EncounterSpec {
                encounter: &self.encounter,
                ruleset: &self.ruleset,
                abilities: &bundle.abilities,
                outcome_tables: &bundle.tables,
                placements: &bundle.placements,
                creatures: &bundle.creatures,
                effects: &bundle.effects,
            };
            body(&spec)
        }

        /// Starts the fixture's encounter in a fresh history wrapper in
        /// `mode`, returning the wrapper and the combatant ids in authored
        /// order (T138b).
        pub fn start_history(
            &self,
            seed: u64,
            mode: EncounterMode,
        ) -> Result<(HistoryWorld, Vec<EntityId>), HistoryError> {
            let mut history = HistoryWorld::new(seed);
            self.with_spec(|spec| history.start_encounter_in_mode(spec, mode))?;
            let ids: Vec<EntityId> = history
                .world()
                .combatants()
                .iter()
                .map(|(id, _)| id)
                .collect();
            Ok((history, ids))
        }
    }

    /// Adds one more participant, a clone of creature A with fresh identities
    /// (placement `placement`, creature `creature`) at `initiative`.
    pub fn add_clone(fixture: &mut Fixture, placement: u128, creature: u128, initiative: i32) {
        let placement_id = Ulid::from_u128(placement);
        let creature_id = Ulid::from_u128(creature);
        let mut document = fixture.placements[&fixture.placement_a].0.clone();
        document.id = placement_id;
        document.slug = format!("probe-extra-{placement}");
        document.name = format!("probe.extra-{placement}");
        document.prefab = creature_id;
        fixture
            .placements
            .insert(placement_id, (document, fixture.area));
        let mut clone = fixture.creatures[&fixture.creature_a].clone();
        clone.id = creature_id;
        clone.slug = format!("probe-creature-extra-{creature}");
        clone.name = format!("probe.creature-extra-{creature}");
        fixture.creatures.insert(creature_id, clone);
        fixture
            .encounter
            .participants
            .push(crpg_data::EncounterParticipant {
                placement: placement_id,
                initiative,
            });
    }

    /// One `UseAbility` action.
    pub fn use_action(actor: EntityId, ability: Ulid, target: EntityId) -> CombatAction {
        CombatAction::UseAbility {
            actor,
            ability,
            target,
        }
    }

    /// Every object key in a JSON value, at any depth.
    pub fn json_keys(value: &serde_json::Value) -> BTreeSet<String> {
        fn walk(value: &serde_json::Value, out: &mut BTreeSet<String>) {
            match value {
                serde_json::Value::Object(map) => {
                    for (key, inner) in map {
                        out.insert(key.clone());
                        walk(inner, out);
                    }
                }
                serde_json::Value::Array(items) => {
                    for item in items {
                        walk(item, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = BTreeSet::new();
        walk(value, &mut out);
        out
    }

    /// The six key names a turn-based-only world must never write.
    pub const RT_KEYS: [&str; 6] = [
        "mode",
        "clock",
        "pending_mode",
        "order",
        "recovery_ticks",
        "repeats_in_real_time",
    ];

    /// The timeline as `(key, entity)` pairs in ascending order.
    pub fn timeline_of(world: &World) -> Vec<(i32, EntityId)> {
        world
            .timeline()
            .iter()
            .map(|(key, id)| (key.0, id))
            .collect()
    }

    /// The world as JSON.
    pub fn value_of(world: &World) -> serde_json::Value {
        serde_json::to_value(world).expect("the world serializes")
    }

    /// Whole-world bytes.
    pub fn bytes_of(world: &World) -> Vec<u8> {
        serde_json::to_vec(world).expect("the world serializes")
    }

    /// Runs `count` ticks.
    pub fn ticks(world: &mut World, count: usize) {
        for _ in 0..count {
            crpg_sim::tick(world);
        }
    }

    /// The combatant's current primary-pool balance.
    pub fn pool_of(world: &World, id: EntityId) -> u32 {
        world
            .combatants()
            .get(id)
            .expect("combatant")
            .action_pool()
            .current()
    }

    /// The encounter clock's elapsed ticks.
    pub fn elapsed(world: &World) -> u64 {
        world
            .combat()
            .expect("encounter")
            .clock
            .expect("clock")
            .elapsed
    }

    /// A fresh ULID text for raw-JSON edits.
    pub fn ulid_text(value: u128) -> String {
        uid(value)
    }
}

/// Kernel-range surface fixtures (T137): typed and derived stats, defences,
/// bonus stats and `Stat` effects over the standard two-combatant fixture.
/// Kept apart so that test binaries that never use them stay free of
/// dead-code warnings.
pub mod surface {
    #![allow(dead_code)]

    use crpg_core::{EntityId, Ulid};
    use crpg_data::{Ability, Effect};
    use crpg_sim::{
        perform_action, ActionOutcome, CombatAction, CombatError, EncounterMode, World,
    };
    use serde_json::{json, Value};

    use super::{uid, Fixture};

    /// A `Literal(Int)` formula node.
    pub fn lit(value: i32) -> Value {
        json!({"type": "literal", "value": {"type": "int", "value": value}})
    }

    /// A `Stat(name)` formula node.
    pub fn stat(name: &str) -> Value {
        json!({"type": "stat", "value": name})
    }

    /// One binary formula node: `kind` is `add`, `subtract`, `multiply`,
    /// `divide`, `min` or `max`.
    pub fn binary(kind: &str, left: Value, right: Value) -> Value {
        json!({"type": kind, "value": {"left": left, "right": right}})
    }

    /// Appends one stat declaration given as JSON.
    pub fn add_stat(fixture: &mut Fixture, declaration: Value) {
        fixture
            .ruleset
            .stats
            .push(serde_json::from_value(declaration).expect("the stat declaration is valid"));
    }

    /// Appends one derived `Int` stat with the given formula JSON.
    pub fn add_derived(fixture: &mut Fixture, name: &str, formula: Value) {
        add_stat(
            fixture,
            json!({"name": name, "kind": "int", "derived": formula}),
        );
    }

    /// Declares the named stat an attribute of the ruleset too.
    pub fn add_attribute(fixture: &mut Fixture, name: &str) {
        fixture.ruleset.attributes.push(name.to_owned());
    }

    /// Sets one stat of the creature `which` to the whole value `value`.
    pub fn set_stat(fixture: &mut Fixture, which: Ulid, name: &str, value: i32) {
        fixture
            .creatures
            .get_mut(&which)
            .expect("the creature exists")
            .stats
            .insert(name.to_owned(), crpg_core::Fx16_16::from_int(value));
    }

    /// Sets one stat of the creature `which` to a raw fixed-point value.
    pub fn set_stat_raw(fixture: &mut Fixture, which: Ulid, name: &str, raw: i32) {
        fixture
            .creatures
            .get_mut(&which)
            .expect("the creature exists")
            .stats
            .insert(name.to_owned(), crpg_core::Fx16_16::from_raw(raw));
    }

    /// Removes one stat from the creature `which`.
    pub fn remove_stat(fixture: &mut Fixture, which: Ulid, name: &str) {
        fixture
            .creatures
            .get_mut(&which)
            .expect("the creature exists")
            .stats
            .remove(name);
    }

    /// One effect modifier JSON object (`target` is `{"type": ...}`).
    pub fn modifier(id: u128, target: Value, op: &str, value: i32) -> Value {
        json!({"id": uid(id), "target": target, "op": {"type": op},
               "value": value, "priority": 0, "name": null})
    }

    /// The `Stat` modifier target of `name`.
    pub fn stat_target(name: &str) -> Value {
        json!({"type": "stat", "value": {"stat": name}})
    }

    /// An effect document: `aim` is `slf` or `target`, `policy` is
    /// `stack_all`, `highest_bonus_worst_penalty` or
    /// `highest_priority_per_name`.
    pub fn effect_doc(
        id: u128,
        aim: &str,
        mod_type: &str,
        policy: &str,
        modifiers: Vec<Value>,
        duration: u32,
    ) -> Effect {
        serde_json::from_value(json!({
            "id": uid(id),
            "slug": format!("probe-effect-{id}"),
            "name": format!("probe.effect-{id}"),
            "aim": {"type": aim},
            "mod_type": mod_type,
            "policy": {"type": policy},
            "modifiers": modifiers,
            "duration_rounds": duration
        }))
        .expect("the effect document is valid")
    }

    /// Registers `effect` with the fixture and makes the standard ability
    /// apply it on success.
    pub fn attach_to_ability(fixture: &mut Fixture, effect: Effect) {
        let id = effect.id;
        fixture.effects.insert(id, effect);
        edit_ability(fixture, |ability| ability.effect = Some(id));
    }

    /// Edits the standard ability in place.
    pub fn edit_ability(fixture: &mut Fixture, edit: impl FnOnce(&mut Ability)) {
        let id = fixture.ability;
        edit(fixture.abilities.get_mut(&id).expect("the ability exists"));
    }

    /// Replaces the outcome table by one band that always succeeds.
    pub fn always_success(fixture: &mut Fixture) {
        let id = Ulid::from_u128(104);
        let table = serde_json::from_value(json!({
            "id": uid(104),
            "slug": "probe-table",
            "name": "probe.table",
            "bands": [{"min_margin": i64::MIN, "outcome": {"type": "success"}}],
            "natural_rules": []
        }))
        .expect("the always-success table is valid");
        fixture.tables.insert(id, table);
    }

    /// Starts the fixture turn-based, returning the combatant ids in
    /// authored order (creature A first; B acts first).
    pub fn try_start(fixture: &Fixture, seed: u64) -> Result<(World, Vec<EntityId>), CombatError> {
        fixture.start_mode(seed, EncounterMode::TurnBased)
    }

    /// Starts and unwraps.
    pub fn start(fixture: &Fixture, seed: u64) -> (World, Vec<EntityId>) {
        try_start(fixture, seed).expect("the surface fixture starts")
    }

    /// One accepted `UseAbility`, unwrapped to its outcome.
    pub fn act(
        world: &mut World,
        actor: EntityId,
        ability: Ulid,
        target: EntityId,
    ) -> ActionOutcome {
        perform_action(
            world,
            &CombatAction::UseAbility {
                actor,
                ability,
                target,
            },
        )
        .expect("the action is accepted")
        .expect("an ability use reports an outcome")
    }

    /// One accepted `EndTurn`.
    pub fn end(world: &mut World, actor: EntityId) {
        perform_action(world, &CombatAction::EndTurn { actor }).expect("the turn ends");
    }
}
