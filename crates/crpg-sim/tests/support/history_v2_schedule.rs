//! Shared sim-local history v2 golden schedule (T138b, ADR-0029 N5).
//!
//! Compiled twice from one source: by `tests/history_v2_golden.rs` and by
//! `examples/generate_history_v2_golden.rs` (via `#[path]`), so the authoring
//! example and the verification test can never drift apart (the T020
//! pattern). It builds a three-combatant, RT-capable encounter in memory with
//! neutral vocabulary and runs the pinned seed-42 schedule: an RT start,
//! standing orders, ticks through two boundaries, a switch to turn-based, one
//! turn-based round, a switch back at the rollover, an invalid order, and a
//! natural end, followed by an inert read, an acknowledgement and a read.
//!
//! The outcome table is "always success" and damage is flat, so every
//! expected event is a function of the schedule alone, never of a roll. The
//! command schedule, ack schedule and fixture below are pinned: changing any
//! step, seed, identity, stat or acknowledgement requires golden review,
//! because every target-scoped golden file records the resulting hashes.

use std::collections::BTreeMap;

use crpg_core::{EntityId, Ulid};
use crpg_data::{
    Ability, AbilityCost, ActionPoolTemplate, Creature, Effect, Encounter,
    OutcomeTable as AuthoredOutcomeTable, Placement, RefreshWire, Ruleset, TickPeriod,
};
use crpg_sim::{
    history_hash, CombatAction, EncounterMode, EncounterSpec, HistoryEnvelope, HistoryWorld,
    PlacementAndArea,
};
use serde_json::json;

/// The pinned golden seed.
pub const GOLDEN_SEED: u64 = 42;

/// The pinned acknowledgement: the first twelve sequences retire.
pub const GOLDEN_ACK: u64 = 12;

/// The number of hashes the schedule records: step zero through the last.
pub const GOLDEN_STEPS: usize = 31;

/// The ruleset's round length in ticks.
pub const ROUND_TICKS: u32 = 4;

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
    /// Encounter 301: A(10), B(9), C(8) in authored order.
    pub encounter: Encounter,
    /// Ruleset 302: `round_ticks` 4, a primary pool (max 2, on turn start)
    /// and an extra pool (max 3, `OnTick(3)`).
    pub ruleset: Ruleset,
    /// Abilities 303 (strike) and 307 (finisher) by identity.
    pub abilities: BTreeMap<Ulid, Ability>,
    /// Table 304 (always success) by identity.
    pub tables: BTreeMap<Ulid, AuthoredOutcomeTable>,
    /// Placements 311/312/313 with owning area 331.
    pub placements: BTreeMap<Ulid, (Placement, Ulid)>,
    /// Creatures 321/322/323 by identity.
    pub creatures: BTreeMap<Ulid, Creature>,
    /// Effect documents by identity (empty: no effects in this schedule).
    pub effects: BTreeMap<Ulid, Effect>,
    /// Identity shortcuts: the repeating two-tick strike for 2 damage.
    pub strike: Ulid,
    /// Identity shortcuts: the one-shot finisher for 99 damage.
    pub finisher: Ulid,
    /// Identity shortcuts.
    pub encounter_id: Ulid,
}

impl GoldenFixture {
    /// Builds the golden documents.
    pub fn new() -> Self {
        let strike = Ulid::from_u128(303);
        let finisher = Ulid::from_u128(307);
        let encounter_id = Ulid::from_u128(301);
        let area = Ulid::from_u128(331);

        let mut ruleset: Ruleset = serde_json::from_value(json!({
            "id": uid(302),
            "slug": "history-v2-ruleset",
            "name": "history.v2.ruleset",
            "package": "history-v2-pack",
            "version": "0.1.0",
            "stats": [
                {"name": "alpha", "kind": "int"},
                {"name": "beta", "kind": "int"},
                {"name": "health", "kind": "int"}
            ],
            "health_stat": "health",
            "attributes": ["alpha", "beta"],
            "pools": [
                {
                    "id": uid(305),
                    "max": 2,
                    "refresh": {"type": "on_turn_start"}
                }
            ],
            "abilities": [uid(303), uid(307)]
        }))
        .expect("the golden ruleset is valid");
        ruleset.round_ticks = Some(ROUND_TICKS);
        ruleset.pools.push(ActionPoolTemplate {
            id: Ulid::from_u128(306),
            max: 3,
            refresh: RefreshWire::OnTick(TickPeriod(3)),
        });

        let mut strike_ability: Ability = serde_json::from_value(json!({
            "id": uid(303),
            "slug": "history-v2-strike",
            "name": "history.v2.strike",
            "dice": "2d6",
            "attribute": "alpha",
            "outcome_table": uid(304),
            "damage": [
                {"outcome": {"type": "success"}, "amount": 2}
            ],
            "cost": 1,
            "extra_costs": [],
            "ends_turn": true,
            "defense": {"type": "actor_attribute"},
            "requires_target": true,
            "allow_self_target": false
        }))
        .expect("the strike ability is valid");
        strike_ability.extra_costs = vec![AbilityCost {
            pool: Ulid::from_u128(306),
            amount: 1,
        }];
        strike_ability.recovery_ticks = Some(2);
        strike_ability.repeats_in_real_time = Some(true);

        let finisher_ability: Ability = serde_json::from_value(json!({
            "id": uid(307),
            "slug": "history-v2-finisher",
            "name": "history.v2.finisher",
            "dice": "2d6",
            "attribute": "alpha",
            "outcome_table": uid(304),
            "damage": [
                {"outcome": {"type": "success"}, "amount": 99}
            ],
            "cost": 1,
            "extra_costs": [],
            "ends_turn": true,
            "defense": {"type": "actor_attribute"},
            "requires_target": true,
            "allow_self_target": false
        }))
        .expect("the finisher ability is valid");

        let always: AuthoredOutcomeTable = serde_json::from_value(json!({
            "id": uid(304),
            "slug": "history-v2-always",
            "name": "history.v2.always",
            "bands": [
                {"min_margin": i64::MIN, "outcome": {"type": "success"}}
            ],
            "natural_rules": []
        }))
        .expect("the always table is valid");

        let encounter: Encounter = serde_json::from_value(json!({
            "id": uid(301),
            "slug": "history-v2-encounter",
            "name": "history.v2.encounter",
            "ruleset": uid(302),
            "participants": [
                {"placement": uid(311), "initiative": 10},
                {"placement": uid(312), "initiative": 9},
                {"placement": uid(313), "initiative": 8}
            ]
        }))
        .expect("the golden encounter is valid");

        let mut placements = BTreeMap::new();
        for (placement_id, creature_id, slug, name) in [
            (311u128, 321u128, "history-v2-a", "history.v2.a"),
            (312u128, 322u128, "history-v2-b", "history.v2.b"),
            (313u128, 323u128, "history-v2-c", "history.v2.c"),
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
        for (creature_id, slug, name) in [
            (321u128, "history-v2-creature-a", "history.v2.creature-a"),
            (322u128, "history-v2-creature-b", "history.v2.creature-b"),
            (323u128, "history-v2-creature-c", "history.v2.creature-c"),
        ] {
            let creature: Creature = serde_json::from_value(json!({
                "id": uid(creature_id),
                "slug": slug,
                "name": name,
                "stats": {
                    "alpha": fx_raw(100),
                    "beta": fx_raw(5),
                    "health": fx_raw(20)
                },
                "tags": [],
                "faction": null,
                "inventory": []
            }))
            .expect("the golden creature is valid");
            creatures.insert(Ulid::from_u128(creature_id), creature);
        }

        let mut abilities = BTreeMap::new();
        abilities.insert(strike, strike_ability);
        abilities.insert(finisher, finisher_ability);
        let mut tables = BTreeMap::new();
        tables.insert(Ulid::from_u128(304), always);

        Self {
            encounter,
            ruleset,
            abilities,
            tables,
            placements,
            creatures,
            effects: BTreeMap::new(),
            strike,
            finisher,
            encounter_id,
        }
    }
}

/// One completed golden run: per-step hashes plus the journal and world the
/// test oracle checks before any hash comparison.
pub struct GoldenRun {
    /// `history_hash` after each pinned step, step zero first.
    pub hashes: Vec<[u8; 32]>,
    /// The full journal captured before the acknowledgement step.
    pub journal: Vec<HistoryEnvelope>,
    /// The final world after every pinned step.
    pub world: HistoryWorld,
    /// The combatants A, B and C, in authored order.
    pub ids: [EntityId; 3],
}

/// One `UseAbility` action.
fn use_ability(actor: EntityId, ability: Ulid, target: EntityId) -> CombatAction {
    CombatAction::UseAbility {
        actor,
        ability,
        target,
    }
}

/// Runs the pinned seed-42 schedule, recording one hash per step.
///
/// Steps: 0 fresh; 1 RT start; 2-3 orders (A strikes B, B strikes C); 4-11
/// eight ticks, boundaries at ticks 4 and 8; 12 request turn-based; 13-16
/// four ticks, the boundary at tick 12 switching to turn-based; 17 C strikes
/// A; 18 request real-time; 19 B strikes A; 20 A strikes B, which rolls the
/// round over into real time; 21-22 orders (A strikes B, C finishes B);
/// 23-25 three ticks, the last holding A's invalid order; 26 A's finishing
/// order; 27 the tick that ends the encounter; 28 read (inert); 29
/// acknowledge; 30 read (inert).
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

    let mut world = HistoryWorld::new(GOLDEN_SEED);
    let mut hashes = vec![history_hash(&world)];

    // Step 1: a real-time start. Entities allocate in authored order A, B, C
    // and no turn starts.
    world
        .start_encounter_in_mode(&spec, EncounterMode::RealTime)
        .expect("step 1 starts");
    let combatants: Vec<_> = world
        .world()
        .combatants()
        .iter()
        .map(|(id, _)| id)
        .collect();
    let (a, b, c) = (combatants[0], combatants[1], combatants[2]);
    hashes.push(history_hash(&world));

    // Steps 2-3: standing orders; both repeat, and neither journals.
    world
        .perform_action(&use_ability(a, fixture.strike, b))
        .expect("step 2 orders");
    hashes.push(history_hash(&world));
    world
        .perform_action(&use_ability(b, fixture.strike, c))
        .expect("step 3 orders");
    hashes.push(history_hash(&world));

    // Steps 4-11: eight ticks through the boundaries at ticks 4 and 8.
    for step in 4..=11 {
        world
            .tick()
            .unwrap_or_else(|error| panic!("step {step} ticks: {error}"));
        hashes.push(history_hash(&world));
    }

    // Step 12: ask for turn-based play; it applies at the next boundary.
    world
        .request_mode(EncounterMode::TurnBased)
        .expect("step 12 requests");
    hashes.push(history_hash(&world));

    // Steps 13-16: four ticks; the boundary at tick 12 switches to
    // turn-based and starts C's turn.
    for step in 13..=16 {
        world
            .tick()
            .unwrap_or_else(|error| panic!("step {step} ticks: {error}"));
        hashes.push(history_hash(&world));
    }

    // Step 17: C strikes A and ends its turn.
    world
        .perform_action(&use_ability(c, fixture.strike, a))
        .expect("step 17 strikes");
    hashes.push(history_hash(&world));

    // Step 18: ask for real-time play; it applies at the next rollover.
    world
        .request_mode(EncounterMode::RealTime)
        .expect("step 18 requests");
    hashes.push(history_hash(&world));

    // Step 19: B strikes A. Step 20: A strikes B; the round rolls over into
    // real time.
    world
        .perform_action(&use_ability(b, fixture.strike, a))
        .expect("step 19 strikes");
    hashes.push(history_hash(&world));
    world
        .perform_action(&use_ability(a, fixture.strike, b))
        .expect("step 20 strikes");
    hashes.push(history_hash(&world));

    // Steps 21-22: fresh orders, since the switch cleared every order.
    world
        .perform_action(&use_ability(a, fixture.strike, b))
        .expect("step 21 orders");
    hashes.push(history_hash(&world));
    world
        .perform_action(&use_ability(c, fixture.finisher, b))
        .expect("step 22 orders");
    hashes.push(history_hash(&world));

    // Steps 23-25: three ticks. The first kills B; the third finds A's
    // repeating order aimed at a dead target.
    for step in 23..=25 {
        world
            .tick()
            .unwrap_or_else(|error| panic!("step {step} ticks: {error}"));
        hashes.push(history_hash(&world));
    }

    // Step 26: A's finishing order.
    world
        .perform_action(&use_ability(a, fixture.finisher, c))
        .expect("step 26 orders");
    hashes.push(history_hash(&world));

    // Step 27: the boundary tick on which A finishes C, ending the encounter.
    world.tick().expect("step 27 ticks");
    hashes.push(history_hash(&world));

    // Capture the full journal before acknowledgement for the test oracle.
    let journal = world.read_after(0, 256).expect("the journal fits");

    // Step 28: a read is inert.
    let _ = world.read_after(0, 3).expect("step 28 reads");
    hashes.push(history_hash(&world));

    // Step 29: the pinned acknowledgement retires the first twelve sequences.
    world.acknowledge(GOLDEN_ACK).expect("step 29 acks");
    hashes.push(history_hash(&world));

    // Step 30: reading past the watermark is inert.
    let _ = world.read_after(GOLDEN_ACK, 256).expect("step 30 reads");
    hashes.push(history_hash(&world));

    GoldenRun {
        hashes,
        journal,
        world,
        ids: [a, b, c],
    }
}
