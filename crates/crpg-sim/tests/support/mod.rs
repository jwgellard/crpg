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
