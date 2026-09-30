//! Shared sim-local history golden schedule (T020, ADR-0017).
//!
//! This file is compiled twice from one source: by
//! `tests/history_golden.rs` and by `examples/generate_history_golden.rs`
//! (via `#[path]`), so the authoring example and the verification test can
//! never drift apart. It builds its authored combat input in memory with
//! neutral vocabulary — no testkit edge — and runs the pinned seed-42
//! schedule: start, non-ending actions, a failed action, `EndTurn`,
//! round rollover, death, natural end, release, a second encounter,
//! despawn (non-active then active), tick, read, and ack.
//!
//! The command schedule and the ack schedule below are pinned: changing any
//! step, seed, identity, stat, or acknowledgement requires golden review,
//! because every target-scoped golden file records the resulting hashes.

use std::collections::BTreeMap;

use crpg_core::Ulid;
use crpg_data::{
    Ability, Creature, Effect, Encounter, OutcomeTable as AuthoredOutcomeTable, Placement, Ruleset,
};
use crpg_sim::{
    history_hash, CombatAction, EncounterSpec, HistoryEnvelope, HistoryEvent, HistoryWorld,
    PlacementAndArea,
};
use serde_json::json;

/// The pinned golden seed.
pub const GOLDEN_SEED: u64 = 42;

/// The pinned acknowledgement: the first ten sequences retire at step 16.
pub const GOLDEN_ACK: u64 = 10;

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
    /// Encounter 201: A(10), B(9), C(8) in authored order.
    pub encounter: Encounter,
    /// Ruleset 202: stats alpha/beta/health, one pool of max 10.
    pub ruleset: Ruleset,
    /// Abilities 203 (non-ending probe) and 207 (lethal) by identity.
    pub abilities: BTreeMap<Ulid, Ability>,
    /// Tables 204 (mixed) and 208 (always success) by identity.
    pub tables: BTreeMap<Ulid, AuthoredOutcomeTable>,
    /// Placements 211/212/213 with owning area 231.
    pub placements: BTreeMap<Ulid, (Placement, Ulid)>,
    /// Creatures 221/222/223 by identity.
    pub creatures: BTreeMap<Ulid, Creature>,
    /// Effect documents by identity (empty: no effects in this schedule).
    pub effects: BTreeMap<Ulid, Effect>,
    /// Identity shortcuts.
    pub ability: Ulid,
    /// Identity shortcuts.
    pub lethal: Ulid,
    /// Identity shortcuts.
    pub encounter_id: Ulid,
}

impl GoldenFixture {
    /// Builds the golden documents.
    pub fn new() -> Self {
        let ability = Ulid::from_u128(203);
        let lethal = Ulid::from_u128(207);
        let encounter_id = Ulid::from_u128(201);
        let area = Ulid::from_u128(231);

        let ruleset: Ruleset = serde_json::from_value(json!({
            "id": uid(202),
            "slug": "history-ruleset",
            "name": "history.ruleset",
            "package": "history-pack",
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
                    "id": uid(205),
                    "max": 10,
                    "refresh": {"type": "on_turn_start"}
                }
            ],
            "abilities": [uid(203), uid(207)]
        }))
        .expect("the golden ruleset is valid");

        let probe: Ability = serde_json::from_value(json!({
            "id": uid(203),
            "slug": "history-probe",
            "name": "history.probe",
            "dice": "2d6",
            "attribute": "alpha",
            "outcome_table": uid(204),
            "damage": [
                {"outcome": {"type": "success"}, "amount": 2},
                {"outcome": {"type": "failure"}, "amount": 0}
            ],
            "cost": 1,
            "extra_costs": [],
            "ends_turn": false,
            "defense": {"type": "actor_attribute"},
            "requires_target": true,
            "allow_self_target": false
        }))
        .expect("the probe ability is valid");

        let lethal_ability: Ability = serde_json::from_value(json!({
            "id": uid(207),
            "slug": "history-lethal",
            "name": "history.lethal",
            "dice": "2d6",
            "attribute": "alpha",
            "outcome_table": uid(208),
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
        .expect("the lethal ability is valid");

        let mixed: AuthoredOutcomeTable = serde_json::from_value(json!({
            "id": uid(204),
            "slug": "history-mixed",
            "name": "history.mixed",
            "bands": [
                {"min_margin": i64::MIN, "outcome": {"type": "success"}},
                {"min_margin": 1, "outcome": {"type": "failure"}}
            ],
            "natural_rules": []
        }))
        .expect("the mixed table is valid");

        let always: AuthoredOutcomeTable = serde_json::from_value(json!({
            "id": uid(208),
            "slug": "history-always",
            "name": "history.always",
            "bands": [
                {"min_margin": i64::MIN, "outcome": {"type": "success"}}
            ],
            "natural_rules": []
        }))
        .expect("the always table is valid");

        let encounter: Encounter = serde_json::from_value(json!({
            "id": uid(201),
            "slug": "history-encounter",
            "name": "history.encounter",
            "ruleset": uid(202),
            "participants": [
                {"placement": uid(211), "initiative": 10},
                {"placement": uid(212), "initiative": 9},
                {"placement": uid(213), "initiative": 8}
            ]
        }))
        .expect("the golden encounter is valid");

        let mut placements = BTreeMap::new();
        for (placement_id, creature_id, slug, name) in [
            (211u128, 221u128, "history-a", "history.a"),
            (212u128, 222u128, "history-b", "history.b"),
            (213u128, 223u128, "history-c", "history.c"),
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
        for (creature_id, alpha, slug, name) in [
            (221u128, 8, "history-creature-a", "history.creature-a"),
            (222u128, 6, "history-creature-b", "history.creature-b"),
            (223u128, 7, "history-creature-c", "history.creature-c"),
        ] {
            let creature: Creature = serde_json::from_value(json!({
                "id": uid(creature_id),
                "slug": slug,
                "name": name,
                "stats": {
                    "alpha": fx_raw(alpha),
                    "beta": fx_raw(5),
                    "health": fx_raw(10)
                },
                "tags": [],
                "faction": null,
                "inventory": []
            }))
            .expect("the golden creature is valid");
            creatures.insert(Ulid::from_u128(creature_id), creature);
        }

        let mut abilities = BTreeMap::new();
        abilities.insert(ability, probe);
        abilities.insert(lethal, lethal_ability);
        let mut tables = BTreeMap::new();
        tables.insert(Ulid::from_u128(204), mixed);
        tables.insert(Ulid::from_u128(208), always);

        Self {
            encounter,
            ruleset,
            abilities,
            tables,
            placements,
            creatures,
            effects: BTreeMap::new(),
            ability,
            lethal,
            encounter_id,
        }
    }
}

/// One completed golden run: per-step hashes plus the journal and world the
/// test oracle checks before any hash comparison.
pub struct GoldenRun {
    /// `history_hash` after each pinned step, step zero first.
    pub hashes: Vec<[u8; 32]>,
    /// The full journal captured before the acknowledgement steps.
    pub journal: Vec<HistoryEnvelope>,
    /// The final world after every pinned step.
    pub world: HistoryWorld,
}

/// Runs the pinned seed-42 schedule, recording one hash per step.
///
/// Steps: 0 fresh, 1 start, 2 non-ending probe, 3 non-ending probe, 4
/// `EndTurn`, 5 probe, 6 `EndTurn`, 7 `EndTurn` (round rollover), 8 lethal
/// (death plus turn), 9 lethal (natural end), 10 release, 11 second
/// encounter, 12 despawn non-active, 13 despawn active (terminal), 14 tick,
/// 15 read (inert), 16 acknowledge, 17 read (inert).
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

    // Step 1: start. Entities allocate in authored order A, B, C; C (lowest
    // initiative) holds the first turn.
    world.start_encounter(&spec).expect("step 1 starts");
    let ids: Vec<_> = world
        .world()
        .combatants()
        .iter()
        .map(|(id, _)| id)
        .collect();
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    hashes.push(history_hash(&world));

    // Steps 2-3: non-ending probes from the active turn holder.
    world
        .perform_action(&CombatAction::UseAbility {
            actor: c,
            ability: fixture.ability,
            target: a,
        })
        .expect("step 2 probes")
        .expect("attack");
    hashes.push(history_hash(&world));
    world
        .perform_action(&CombatAction::UseAbility {
            actor: c,
            ability: fixture.ability,
            target: b,
        })
        .expect("step 3 probes")
        .expect("attack");
    hashes.push(history_hash(&world));

    // Step 4: end C's turn.
    world
        .perform_action(&CombatAction::EndTurn { actor: c })
        .expect("step 4 ends");
    hashes.push(history_hash(&world));

    // Step 5: probe from B.
    world
        .perform_action(&CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        })
        .expect("step 5 probes")
        .expect("attack");
    hashes.push(history_hash(&world));

    // Steps 6-7: end B, then end A into the empty timeline: round rollover.
    world
        .perform_action(&CombatAction::EndTurn { actor: b })
        .expect("step 6 ends");
    hashes.push(history_hash(&world));
    world
        .perform_action(&CombatAction::EndTurn { actor: a })
        .expect("step 7 ends");
    hashes.push(history_hash(&world));

    // Step 8: lethal C onto B — death plus the next turn.
    world
        .perform_action(&CombatAction::UseAbility {
            actor: c,
            ability: fixture.lethal,
            target: b,
        })
        .expect("step 8 kills")
        .expect("attack");
    hashes.push(history_hash(&world));

    // Step 9: lethal A onto C — the natural end.
    world
        .perform_action(&CombatAction::UseAbility {
            actor: a,
            ability: fixture.lethal,
            target: c,
        })
        .expect("step 9 ends naturally")
        .expect("attack");
    hashes.push(history_hash(&world));

    // Step 10: release the terminal encounter.
    world.end_encounter().expect("step 10 releases");
    hashes.push(history_hash(&world));

    // Step 11: a second encounter reuses the content with fresh runtime ids.
    world.start_encounter(&spec).expect("step 11 starts");
    let second: Vec<_> = world
        .world()
        .combatants()
        .iter()
        .map(|(id, _)| id)
        .collect();
    let (a2, _b2, c2) = (second[0], second[1], second[2]);
    hashes.push(history_hash(&world));

    // Steps 12-13: despawn the non-active combatant, then the active one
    // into the terminal transition.
    assert!(world.despawn(a2).expect("step 12 despawns"));
    hashes.push(history_hash(&world));
    assert!(world.despawn(c2).expect("step 13 despawns"));
    hashes.push(history_hash(&world));

    // Step 14: tick advances time with no journal addition.
    world.tick().expect("step 14 ticks");
    hashes.push(history_hash(&world));

    // Capture the full journal before acknowledgement for the test oracle.
    let journal = world.read_after(0, 256).expect("the journal fits");

    // Step 15: a read is inert.
    let _ = world.read_after(0, 3).expect("step 15 reads");
    hashes.push(history_hash(&world));

    // Step 16: the pinned acknowledgement retires the first ten sequences.
    world.acknowledge(GOLDEN_ACK).expect("step 16 acks");
    hashes.push(history_hash(&world));

    // Step 17: reading past the watermark is inert.
    let _ = world.read_after(GOLDEN_ACK, 256).expect("step 17 reads");
    hashes.push(history_hash(&world));

    GoldenRun {
        hashes,
        journal,
        world,
    }
}

/// The hand-authored event oracle is written in the golden test, not here:
/// this module transports the schedule and its actuals, never the
/// expectations. The payload constructors below exist only so the example
/// stays free of oracle-shaped assertions.
#[allow(dead_code)]
pub fn journal_payloads(journal: &[HistoryEnvelope]) -> Vec<HistoryEvent> {
    journal.iter().map(|env| env.payload.clone()).collect()
}
