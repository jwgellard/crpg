//! Shared sim-local surface golden schedule (T137c).
//!
//! Compiled twice from one source: by `tests/surface_golden.rs` and by
//! `examples/generate_surface_golden.rs` (via `#[path]`), so the authoring
//! example and the verification test can never drift apart (the T020
//! pattern). It builds a three-combatant, turn-based encounter in memory
//! with neutral vocabulary that exercises every T137 shape at once:
//! derived and typed stats, all five defences (including `Opposed`),
//! bonus stats, damage dice, rolled initiative, and a `Stat`-target
//! effect that grants a tag. It runs a start, one action per defence, a
//! save/load mid-script (continuing from the loaded world), one more
//! action and `end_encounter`, recording one `state_hash` per step.
//!
//! The command schedule, seed, identities, stats and acknowledgement below
//! are pinned: changing any step, seed, identity, stat or acknowledgement
//! requires golden review, because every target-scoped golden file
//! records the resulting hashes.

use std::collections::BTreeMap;

use crpg_core::{EntityId, Ulid};
use crpg_data::{
    Ability, Creature, Effect, Encounter, OutcomeTable as AuthoredOutcomeTable, Placement, Ruleset,
};
use crpg_sim::{state_hash, CombatAction, EncounterSpec, PlacementAndArea, World};
use serde_json::json;

/// The pinned golden seed.
pub const GOLDEN_SEED: u64 = 42;

/// The number of hashes the schedule records: step zero through the last.
pub const GOLDEN_STEPS: usize = 10;

/// Canonical ULID text for golden identities.
fn uid(value: u128) -> String {
    Ulid::from_u128(value).to_string()
}

/// Whole fixed-point raw value for an integer stat.
fn fx_raw(value: i32) -> i32 {
    crpg_core::Fx16_16::from_int(value).to_raw()
}

/// The golden authored documents with neutral vocabulary.
pub struct GoldenFixture {
    /// Encounter 401: A(10), B(9), C(8) in authored order.
    pub encounter: Encounter,
    /// Ruleset 402: typed and derived stats, one pool, rolled initiative.
    pub ruleset: Ruleset,
    /// Abilities 411..415, one per defence.
    pub abilities: BTreeMap<Ulid, Ability>,
    /// Table 404 (always success) by identity.
    pub tables: BTreeMap<Ulid, AuthoredOutcomeTable>,
    /// Placements 421/422/423 with owning area 431.
    pub placements: BTreeMap<Ulid, (Placement, Ulid)>,
    /// Creatures 441/442/443 by identity.
    pub creatures: BTreeMap<Ulid, Creature>,
    /// Effect documents by identity (one granting effect).
    pub effects: BTreeMap<Ulid, Effect>,
    /// Identity shortcuts.
    pub ability_actor_attribute: Ulid,
    pub ability_constant: Ulid,
    pub ability_actor_stat: Ulid,
    pub ability_target_stat: Ulid,
    pub ability_opposed: Ulid,
}

impl GoldenFixture {
    /// Builds the golden documents.
    pub fn new() -> Self {
        let ability_actor_attribute = Ulid::from_u128(411);
        let ability_constant = Ulid::from_u128(412);
        let ability_actor_stat = Ulid::from_u128(413);
        let ability_target_stat = Ulid::from_u128(414);
        let ability_opposed = Ulid::from_u128(415);
        let effect_id = Ulid::from_u128(451);
        let area = Ulid::from_u128(431);

        let mut ruleset: Ruleset = serde_json::from_value(json!({
            "id": uid(402),
            "slug": "surface-ruleset",
            "name": "surface.ruleset",
            "package": "surface-pack",
            "version": "0.1.0",
            "stats": [
                {"name": "alpha", "kind": "int"},
                {"name": "beta", "kind": "int"},
                {"name": "gamma", "kind": "int"},
                {"name": "health", "kind": "int"},
                {"name": "weight", "kind": "fixed"},
                {"name": "guard", "kind": "int", "derived": {
                    "type": "add", "value": {
                        "left": {"type": "stat", "value": "alpha"},
                        "right": {"type": "stat", "value": "beta"}
                    }
                }}
            ],
            "health_stat": "health",
            "attributes": ["alpha", "beta", "gamma", "guard"],
            "pools": [
                {
                    "id": uid(405),
                    "max": 10,
                    "refresh": {"type": "on_turn_start"}
                }
            ],
            "abilities": [uid(411), uid(412), uid(413), uid(414), uid(415)]
        }))
        .expect("the golden ruleset is valid");
        ruleset.initiative = Some(
            serde_json::from_value(json!({"dice": "1d20", "bonus_stats": ["alpha"]}))
                .expect("the initiative rule is valid"),
        );

        let always: AuthoredOutcomeTable = serde_json::from_value(json!({
            "id": uid(404),
            "slug": "surface-always",
            "name": "surface.always",
            "bands": [
                {"min_margin": i64::MIN, "outcome": {"type": "success"}}
            ],
            "natural_rules": []
        }))
        .expect("the always table is valid");

        // Ability 411: ActorAttribute defence, bonus stats, damage dice.
        let mut aa: Ability = serde_json::from_value(json!({
            "id": uid(411),
            "slug": "surface-actor-attribute",
            "name": "surface.actor_attribute",
            "dice": "2d6",
            "attribute": "alpha",
            "outcome_table": uid(404),
            "damage": [
                {"outcome": {"type": "success"}, "amount": 2, "dice": "1d4"}
            ],
            "cost": 1,
            "extra_costs": [],
            "ends_turn": true,
            "defense": {"type": "actor_attribute"},
            "requires_target": true,
            "allow_self_target": false
        }))
        .expect("the actor-attribute ability is valid");
        aa.bonus_stats = Some(vec!["guard".to_owned()]);
        aa.effect = Some(effect_id);

        let constant: Ability = serde_json::from_value(json!({
            "id": uid(412),
            "slug": "surface-constant",
            "name": "surface.constant",
            "dice": "2d6",
            "attribute": "alpha",
            "outcome_table": uid(404),
            "damage": [
                {"outcome": {"type": "success"}, "amount": 3}
            ],
            "cost": 1,
            "extra_costs": [],
            "ends_turn": true,
            "defense": {"type": "constant", "value": 12},
            "requires_target": true,
            "allow_self_target": false
        }))
        .expect("the constant ability is valid");

        let actor_stat: Ability = serde_json::from_value(json!({
            "id": uid(413),
            "slug": "surface-actor-stat",
            "name": "surface.actor_stat",
            "dice": "2d6",
            "attribute": "alpha",
            "outcome_table": uid(404),
            "damage": [
                {"outcome": {"type": "success"}, "amount": 4, "dice": "1d6"}
            ],
            "cost": 1,
            "extra_costs": [],
            "ends_turn": true,
            "defense": {"type": "actor_stat", "value": {"stat": "beta"}},
            "requires_target": true,
            "allow_self_target": false
        }))
        .expect("the actor-stat ability is valid");

        let target_stat: Ability = serde_json::from_value(json!({
            "id": uid(414),
            "slug": "surface-target-stat",
            "name": "surface.target_stat",
            "dice": "2d6",
            "attribute": "alpha",
            "outcome_table": uid(404),
            "damage": [
                {"outcome": {"type": "success"}, "amount": 5}
            ],
            "cost": 1,
            "extra_costs": [],
            "ends_turn": true,
            "defense": {"type": "target_stat", "value": {"stat": "guard"}},
            "requires_target": true,
            "allow_self_target": false
        }))
        .expect("the target-stat ability is valid");

        let opposed: Ability = serde_json::from_value(json!({
            "id": uid(415),
            "slug": "surface-opposed",
            "name": "surface.opposed",
            "dice": "2d6",
            "attribute": "alpha",
            "outcome_table": uid(404),
            "damage": [
                {"outcome": {"type": "success"}, "amount": 6, "dice": "1d8"}
            ],
            "cost": 1,
            "extra_costs": [],
            "ends_turn": true,
            "defense": {"type": "opposed", "value": {"dice": "1d8", "bonus_stats": ["beta"]}},
            "requires_target": true,
            "allow_self_target": false
        }))
        .expect("the opposed ability is valid");

        let encounter: Encounter = serde_json::from_value(json!({
            "id": uid(401),
            "slug": "surface-encounter",
            "name": "surface.encounter",
            "ruleset": uid(402),
            "participants": [
                {"placement": uid(421), "initiative": 10},
                {"placement": uid(422), "initiative": 9},
                {"placement": uid(423), "initiative": 8}
            ]
        }))
        .expect("the golden encounter is valid");

        let mut placements = BTreeMap::new();
        for (placement_id, creature_id, slug, name) in [
            (421u128, 441u128, "surface-a", "surface.a"),
            (422u128, 442u128, "surface-b", "surface.b"),
            (423u128, 443u128, "surface-c", "surface.c"),
        ] {
            let placement: Placement = serde_json::from_value(json!({
                "id": uid(placement_id),
                "slug": format!("{slug}-place"),
                "name": format!("{name}.place"),
                "prefab": uid(creature_id),
                "transform": {
                    "position": [0, 0, 0],
                    "rotation": [0, 0, 0],
                    "scale": [65536, 65536, 65536]
                },
                "overrides": {}
            }))
            .expect("the golden placement is valid");
            placements.insert(Ulid::from_u128(placement_id), (placement, area));
        }

        let mut creatures = BTreeMap::new();
        for (creature_id, alpha, beta, gamma, slug, name) in [
            (441u128, 8, 6, 7, "surface-creature-a", "surface.creature-a"),
            (442u128, 6, 7, 5, "surface-creature-b", "surface.creature-b"),
            (443u128, 7, 5, 6, "surface-creature-c", "surface.creature-c"),
        ] {
            let creature: Creature = serde_json::from_value(json!({
                "id": uid(creature_id),
                "slug": slug,
                "name": name,
                "stats": {
                    "alpha": fx_raw(alpha),
                    "beta": fx_raw(beta),
                    "gamma": fx_raw(gamma),
                    "health": fx_raw(20),
                    "weight": fx_raw(3)
                },
                "tags": ["pc"],
                "faction": null,
                "inventory": []
            }))
            .expect("the golden creature is valid");
            creatures.insert(Ulid::from_u128(creature_id), creature);
        }

        // Stat-target effect granting a tag.
        let effect: Effect = serde_json::from_value(json!({
            "id": uid(451),
            "slug": "surface-grant",
            "name": "surface.grant",
            "aim": {"type": "target"},
            "mod_type": "probe-type",
            "policy": {"type": "stack_all"},
            "modifiers": [
                {"id": uid(452), "target": {"type": "stat", "value": {"stat": "gamma"}},
                 "op": {"type": "add"}, "value": 2, "priority": 0, "name": null}
            ],
            "duration_rounds": 2,
            "grants": ["hasted"]
        }))
        .expect("the granting effect is valid");

        let mut abilities = BTreeMap::new();
        abilities.insert(ability_actor_attribute, aa);
        abilities.insert(ability_constant, constant);
        abilities.insert(ability_actor_stat, actor_stat);
        abilities.insert(ability_target_stat, target_stat);
        abilities.insert(ability_opposed, opposed);
        let mut tables = BTreeMap::new();
        tables.insert(Ulid::from_u128(404), always);
        let mut effects = BTreeMap::new();
        effects.insert(effect_id, effect);

        Self {
            encounter,
            ruleset,
            abilities,
            tables,
            placements,
            creatures,
            effects,
            ability_actor_attribute,
            ability_constant,
            ability_actor_stat,
            ability_target_stat,
            ability_opposed,
        }
    }
}

/// One completed golden run.
pub struct GoldenRun {
    /// `state_hash` after each pinned step, step zero first.
    pub hashes: Vec<[u8; 32]>,
    /// Rolled initiative keys, in authored combatant order.
    pub initiative_keys: Vec<i32>,
    /// Margin of each action, in step order.
    pub margins: Vec<i64>,
    /// Damage of each action, in step order.
    pub damages: Vec<u32>,
}

/// Runs the pinned seed-42 schedule.
pub fn run_golden_schedule() -> GoldenRun {
    let fixture = GoldenFixture::new();
    let mut abilities = BTreeMap::new();
    for (id, ability) in &fixture.abilities {
        abilities.insert(*id, ability);
    }
    let mut tables = BTreeMap::new();
    for (id, table) in &fixture.tables {
        tables.insert(*id, table);
    }
    let mut placements = BTreeMap::new();
    for (id, (placement, area)) in &fixture.placements {
        placements.insert(
            *id,
            PlacementAndArea {
                placement,
                area: *area,
            },
        );
    }
    let mut creatures = BTreeMap::new();
    for (id, creature) in &fixture.creatures {
        creatures.insert(*id, creature);
    }
    let mut effects = BTreeMap::new();
    for (id, effect) in &fixture.effects {
        effects.insert(*id, effect);
    }
    let spec = EncounterSpec {
        encounter: &fixture.encounter,
        ruleset: &fixture.ruleset,
        abilities: &abilities,
        outcome_tables: &tables,
        placements: &placements,
        creatures: &creatures,
        effects: &effects,
    };

    let mut world = World::new(GOLDEN_SEED);
    let mut hashes = vec![state_hash(&world)];
    let mut margins = Vec::new();
    let mut damages = Vec::new();

    // Step 1: start.
    crpg_sim::start_encounter(&mut world, &spec).expect("step 1 starts");
    let combatants: Vec<_> = world.combatants().iter().map(|(id, _)| id).collect();
    let ids = [combatants[0], combatants[1], combatants[2]];
    let initiative_keys: Vec<i32> = ids
        .iter()
        .map(|id| world.combatants().get(*id).expect("combatant").initiative())
        .collect();
    hashes.push(state_hash(&world));

    let ability_list = [
        fixture.ability_actor_attribute,
        fixture.ability_constant,
        fixture.ability_actor_stat,
        fixture.ability_target_stat,
        fixture.ability_opposed,
    ];

    fn alive_target(world: &World, actor: EntityId, ids: &[EntityId; 3]) -> EntityId {
        for id in ids {
            if *id != actor {
                if let Some(c) = world.combatants().get(*id) {
                    if !c.dead() {
                        return *id;
                    }
                }
            }
        }
        ids[0]
    }

    // Steps 2..6: one action per defence.
    for (index, ability) in ability_list.iter().enumerate() {
        let step = 2 + index;
        let actor = world.combat().expect("encounter").active.expect("active");
        let target = alive_target(&world, actor, &ids);
        let outcome = crpg_sim::perform_action(
            &mut world,
            &CombatAction::UseAbility {
                actor,
                ability: *ability,
                target,
            },
        )
        .expect("action accepted")
        .expect("ability use reports an outcome");
        margins.push(outcome.margin);
        damages.push(outcome.damage);
        hashes.push(state_hash(&world));
        let _ = step;
    }

    // Step 7: save/load mid-script, continue from the loaded world.
    let value = serde_json::to_value(&world).expect("the world serializes");
    let loaded: World = serde_json::from_value(value).expect("the save loads");
    world = loaded;
    hashes.push(state_hash(&world));

    // Step 8: one more action after the reload.
    let actor = world.combat().expect("encounter").active.expect("active");
    let target = alive_target(&world, actor, &ids);
    let outcome = crpg_sim::perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor,
            ability: fixture.ability_actor_attribute,
            target,
        },
    )
    .expect("action accepted")
    .expect("ability use reports an outcome");
    margins.push(outcome.margin);
    damages.push(outcome.damage);
    hashes.push(state_hash(&world));

    // Step 9: end the encounter.
    crpg_sim::end_encounter(&mut world).expect("end_encounter");
    hashes.push(state_hash(&world));

    GoldenRun {
        hashes,
        initiative_keys,
        margins,
        damages,
    }
}
