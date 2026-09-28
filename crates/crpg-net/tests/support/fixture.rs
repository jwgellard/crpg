//! T018c conformance fixtures: authored encounter documents (test-only).
//!
//! Mirrors the proven `crpg-sim` probe fixture shape (neutral vocabulary,
//! parsed through the production data shapes) with fresh identities, plus a
//! second ability whose combined spend exceeds its pool maximum: the adapter
//! accepts it per-amount, so the first use fails closed at runtime with
//! `InsufficientAction` — the malicious matrix's insufficient-pool case.

use std::collections::BTreeMap;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_data::{
    Ability, Creature, Effect, Encounter, OutcomeTable as AuthoredOutcomeTable, Placement, Ruleset,
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
    /// Encounter 501: participants 511 (initiative 10) then 512 (9).
    pub encounter: Encounter,
    /// Ruleset 502: stats alpha/beta/gamma/health, pool max 1.
    pub ruleset: Ruleset,
    /// Abilities 503 (affordable) and 506 (combined spend 2 > max 1).
    pub abilities: BTreeMap<Ulid, Ability>,
    /// Outcome table 504.
    pub tables: BTreeMap<Ulid, AuthoredOutcomeTable>,
    /// Placements 511/512 with owning area 531.
    pub placements: BTreeMap<Ulid, (Placement, Ulid)>,
    /// Creatures 521/522.
    pub creatures: BTreeMap<Ulid, Creature>,
    /// Affordable attack ability.
    pub ability: Ulid,
    /// Unaffordable attack ability (combined spend exceeds the pool).
    pub pricey_ability: Ulid,
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
    /// Effect references by identity (empty: no effects in this fixture).
    pub effects: BTreeMap<Ulid, &'a Effect>,
}

impl Fixture {
    /// The standard two-combatant fixture: creature A (alpha 8, health 10)
    /// before creature B (alpha 6, health 6), B acting first on lower
    /// initiative, two-band roll-under table with flat damage
    /// `{success: 2, failure: 0}` and cost 1.
    pub fn standard() -> Self {
        let ability = Ulid::from_u128(503);
        let pricey_ability = Ulid::from_u128(506);
        let area = Ulid::from_u128(531);
        let placement_a = Ulid::from_u128(511);
        let placement_b = Ulid::from_u128(512);
        let creature_a = Ulid::from_u128(521);
        let creature_b = Ulid::from_u128(522);

        let ruleset: Ruleset = serde_json::from_value(json!({
            "id": uid(502),
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
                    "id": uid(505),
                    "max": 1,
                    "refresh": {"type": "on_turn_start"}
                },
                {
                    "id": uid(507),
                    "max": 1,
                    "refresh": {"type": "never"}
                }
            ],
            "abilities": [uid(503), uid(506)]
        }))
        .expect("the standard ruleset is valid");

        let attack: Ability = serde_json::from_value(json!({
            "id": uid(503),
            "slug": "probe-attack",
            "name": "probe.attack",
            "dice": "2d6",
            "attribute": "alpha",
            "outcome_table": uid(504),
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

        let pricey: Ability = serde_json::from_value(json!({
            "id": uid(506),
            "slug": "probe-pricey",
            "name": "probe.pricey",
            "dice": "2d6",
            "attribute": "alpha",
            "outcome_table": uid(504),
            "damage": [
                {"outcome": {"type": "success"}, "amount": 2},
                {"outcome": {"type": "failure"}, "amount": 0}
            ],
            "cost": 1,
            "extra_costs": [{"pool": uid(507), "amount": 1}],
            "ends_turn": true,
            "defense": {"type": "actor_attribute"},
            "requires_target": true,
            "allow_self_target": false
        }))
        .expect("the pricey ability is valid");

        let table: AuthoredOutcomeTable = serde_json::from_value(json!({
            "id": uid(504),
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
            "id": uid(501),
            "slug": "probe-encounter",
            "name": "probe.encounter",
            "ruleset": uid(502),
            "participants": [
                {"placement": uid(511), "initiative": 10},
                {"placement": uid(512), "initiative": 9}
            ]
        }))
        .expect("the standard encounter is valid");

        let placement_doc_a: Placement = serde_json::from_value(json!({
            "id": uid(511),
            "slug": "probe-a",
            "name": "probe.a",
            "prefab": uid(521),
            "transform": {
                "position": [0, 0, 0],
                "rotation": [0, 0, 0],
                "scale": [65536, 65536, 65536]
            },
            "overrides": {}
        }))
        .expect("placement A is valid");
        let placement_doc_b: Placement = serde_json::from_value(json!({
            "id": uid(512),
            "slug": "probe-b",
            "name": "probe.b",
            "prefab": uid(522),
            "transform": {
                "position": [0, 0, 0],
                "rotation": [0, 0, 0],
                "scale": [65536, 65536, 65536]
            },
            "overrides": {}
        }))
        .expect("placement B is valid");

        let hero: Creature = serde_json::from_value(json!({
            "id": uid(521),
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
            "id": uid(522),
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
        abilities.insert(pricey_ability, pricey);
        let mut tables = BTreeMap::new();
        tables.insert(Ulid::from_u128(504), table);
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
            placements,
            creatures,
            ability,
            pricey_ability,
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
        let effects = BTreeMap::new();
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
}
