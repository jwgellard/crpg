#![allow(dead_code)]

//! The T056b AI fixture: authored encounter documents for the decision suite.
//!
//! Copied value for value from sim's own suites —
//! `crates/crpg-sim/tests/support/mod.rs`'s `Fixture::standard()` (the combat
//! documents and `start`/`start_mode`) and
//! `crates/crpg-sim/tests/support/movement_schedule.rs`'s `move_fixture` (the
//! movement rules, paces, placement 112's position and creature/placement
//! 113) — then edited by the functions below. Parsing the authored JSON
//! through `crpg-data`'s types keeps the fixture honest about the wire
//! contract; every edit after that is a typed change, so no numeric policy
//! hides in a string.
//!
//! Two suites include this module, each using only part of it, so the file
//! carries `#![allow(dead_code)]` exactly as
//! `crates/crpg-sim/tests/support/movement_schedule.rs` does:
//! `cargo clippy --all-targets -- -D warnings` would otherwise fail on the
//! unused part.

use std::collections::BTreeMap;

use crpg_ai::{host_route, SideId, Sides};
use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_data::{
    Ability, AbilityCost, ActionPoolTemplate, Creature, Effect, Encounter, EncounterParticipant,
    LengthWire, MovementWire, OutcomeTable as AuthoredOutcomeTable, Placement, RefreshWire,
    Ruleset, StatDecl, StatKindWire, TurnMovementWire,
};
use crpg_nav::{bake_box, NavMesh, NavRect};
use crpg_sim::{
    perform_move, register_creature, start_encounter_in_mode, start_encounter_with_members,
    CombatAction, CombatError, CreatureSpec, EncounterMode, EncounterSpec, Entrant, MoveOutcome,
    PlacementAndArea, World,
};
use serde_json::json;

/// 1.5 m in raw units: ability 103's range in most cases (`R15`).
pub const R15: i32 = 98_304;
/// 3 m in raw units: the twin ability's range in G20 (`R3`).
pub const R3: i32 = 196_608;
const _: () = assert!(R15 == 98_304);
const _: () = assert!(R3 == 196_608);

/// Whole fixed-point raw value for an integer stat.
pub fn fx_raw(value: i32) -> i32 {
    Fx16_16::from_int(value).to_raw()
}

/// Canonical ULID text for fixture identities.
pub fn uid(value: u128) -> String {
    Ulid::from_u128(value).to_string()
}

/// The ULID of fixture identity `value`.
pub fn ulid(value: u128) -> Ulid {
    Ulid::from_u128(value)
}

/// `p(x, z)`: whole metres on the ground plane.
pub fn p(x: i32, z: i32) -> [Fx16_16; 3] {
    [Fx16_16::from_int(x), Fx16_16::ZERO, Fx16_16::from_int(z)]
}

/// `rp(x, y, z)`: the point with those raw values.
pub fn rp(x: i32, y: i32, z: i32) -> [Fx16_16; 3] {
    [
        Fx16_16::from_raw(x),
        Fx16_16::from_raw(y),
        Fx16_16::from_raw(z),
    ]
}

/// `rect(x0, z0, x1, z1)`: a whole-metre rectangle in `x`/`z`.
pub fn rect(x0: i32, z0: i32, x1: i32, z1: i32) -> NavRect {
    NavRect {
        min_x: Fx16_16::from_int(x0),
        min_z: Fx16_16::from_int(z0),
        max_x: Fx16_16::from_int(x1),
        max_z: Fx16_16::from_int(z1),
    }
}

/// `M`: one cell, so every path on it is a straight two-point line.
pub fn m() -> NavMesh {
    bake_box(Fx16_16::ZERO, rect(-5, -5, 25, 25), &[]).expect("M bakes")
}

/// `MS`: `M` with a full-depth wall at `x = 4..6`, so the halves do not touch.
pub fn ms() -> NavMesh {
    bake_box(Fx16_16::ZERO, rect(-5, -5, 25, 25), &[rect(4, -5, 6, 25)]).expect("MS bakes")
}

/// `rev(m)`: the revision a mesh's canonical bytes hash to.
pub fn rev(mesh: &NavMesh) -> crpg_sim::NavRevision {
    crpg_sim::NavRevision::of_bytes(&mesh.to_canonical_bytes())
}

/// `bind(world, m)`: bind the mesh's revision into the world.
pub fn bind(world: &mut World, mesh: &NavMesh) {
    world.bind_nav(rev(mesh));
}

/// `S(A:1, B:2, ...)`: the caller-supplied sides of the contract.
pub fn sides(entries: &[(EntityId, u32)]) -> Sides {
    Sides::new(
        entries
            .iter()
            .map(|&(entity, side)| (entity, SideId(side)))
            .collect(),
    )
    .expect("the fixture lists each entity once")
}

/// `use(x, a, t)`: ability `a` from `x` onto `t`.
pub fn use_ability(actor: EntityId, ability: u128, target: EntityId) -> CombatAction {
    CombatAction::UseAbility {
        actor,
        ability: ulid(ability),
        target,
    }
}

/// `apply_move(world, mesh, actor, destination)`: the host's move on the
/// fixture's mesh — turn-based `perform_move` over `host_route`, or the
/// real-time `set_route`.
pub fn apply_move(
    world: &mut World,
    mesh: &NavMesh,
    actor: EntityId,
    destination: [Fx16_16; 3],
) -> Option<MoveOutcome> {
    let route = host_route(mesh, world, actor, destination).expect("the destination routes");
    if world
        .combat()
        .is_some_and(|state| state.mode == EncounterMode::RealTime)
    {
        world.set_route(actor, route).expect("the route validates");
        None
    } else {
        Some(perform_move(world, actor, &route).expect("the move is affordable"))
    }
}

/// Authored fixture documents with neutral vocabulary.
pub struct Fixture {
    /// Encounter 101: participants 111 (initiative 10) then 112 (9).
    pub encounter: Encounter,
    /// Ruleset 102: stats alpha/beta/gamma/health, pool max 1.
    pub ruleset: Ruleset,
    /// Ability documents keyed by identity.
    pub abilities: BTreeMap<Ulid, Ability>,
    /// Outcome table 104 keyed by identity.
    pub tables: BTreeMap<Ulid, AuthoredOutcomeTable>,
    /// Effect documents by identity (empty for these fixtures).
    pub effects: BTreeMap<Ulid, Effect>,
    /// Placements with owning areas.
    pub placements: BTreeMap<Ulid, (Placement, Ulid)>,
    /// Creature prefabs keyed by identity.
    pub creatures: BTreeMap<Ulid, Creature>,
    /// Identity shortcut: ability 103.
    pub ability: Ulid,
    /// Identity shortcut: area 131.
    pub area: Ulid,
    /// Identity shortcut: placement 111.
    pub placement_a: Ulid,
    /// Identity shortcut: placement 112.
    pub placement_b: Ulid,
    /// Identity shortcut: creature 121.
    pub creature_a: Ulid,
    /// Identity shortcut: creature 122.
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
        let ability = ulid(103);
        let area = ulid(131);
        let placement_a = ulid(111);
        let placement_b = ulid(112);
        let creature_a = ulid(121);
        let creature_b = ulid(122);

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
        tables.insert(ulid(104), table);
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

    /// Starts the fixture's encounter in `mode`, returning the world and the
    /// combatant ids in authored participant order.
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

    /// Starts the encounter turn-based; asserts B (initiative 9) acts first.
    pub fn start(&self, seed: u64) -> (World, Vec<EntityId>) {
        let (world, ids) = self
            .start_mode(seed, EncounterMode::TurnBased)
            .expect("the fixture starts");
        let active = world.combat().and_then(|state| state.active);
        assert_eq!(active, Some(ids[1]), "B's initiative 9 is least");
        (world, ids)
    }

    /// Starts the encounter in real time; the fixture fixes the mode.
    pub fn start_rt(&self, seed: u64) -> (World, Vec<EntityId>) {
        self.start_mode(seed, EncounterMode::RealTime)
            .expect("the fixture starts in real time")
    }

    /// Starts the encounter with placement 113's creature as a member that
    /// enters from a registered record, after A and B are published.
    ///
    /// With `with_movement` the record is registered under the fixture's own
    /// movement rules and gains a mover. Without, it is registered under a
    /// copy of the ruleset whose movement rules are absent, so it enters as a
    /// combatant with no mover — the mixed pair Reading 5 names ("a pair
    /// without a mover on both sides has distance `u64::MAX`").
    pub fn start_with_c_record(&self, seed: u64, with_movement: bool) -> (World, Vec<EntityId>) {
        let mut ruleset = self.ruleset.clone();
        if !with_movement {
            ruleset.movement = None;
        }
        let (placement, area) = self
            .placements
            .get(&ulid(113))
            .expect("the moving fixture has placement 113")
            .clone();
        let creature = self
            .creatures
            .get(&ulid(123))
            .expect("the moving fixture has creature 123")
            .clone();
        let mut world = World::new(seed);
        let member = register_creature(
            &mut world,
            &CreatureSpec {
                ruleset: &ruleset,
                placement: &placement,
                area,
                creature: &creature,
            },
        )
        .expect("the member registers");
        assert_eq!(
            world.movers().contains(member),
            with_movement,
            "registration creates a mover exactly when the ruleset moves"
        );
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
        start_encounter_with_members(
            &mut world,
            &spec,
            EncounterMode::TurnBased,
            &[Entrant {
                entity: member,
                initiative: 11,
            }],
        )
        .expect("the encounter starts with the member");
        let ids: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
        assert_eq!(ids.len(), 3, "A, B and the record member");
        assert_eq!(ids[2], member, "the member publishes last");
        let active = world.combat().and_then(|state| state.active);
        assert_eq!(active, Some(ids[1]), "B's initiative 9 is least");
        (world, ids)
    }
}

/// `std()`: the standard fixture, turn-based, with no movement rules.
pub fn std() -> Fixture {
    Fixture::standard()
}

/// Sets one base `Fixed` stat of a creature to a raw value.
pub fn set_creature_stat(fixture: &mut Fixture, creature: u128, name: &str, raw: i32) {
    fixture
        .creatures
        .get_mut(&ulid(creature))
        .expect("the creature exists")
        .stats
        .insert(name.to_owned(), Fx16_16::from_raw(raw));
}

/// `moving()`: `std()` plus the MOVE fixture's movement rules — stat `pace`,
/// pool 106 max 6, movement `{speed: pace, turn: {pool: 106, step: 98304,
/// step_cost: 1}}`, A pace 65536, B pace 32768, placement 112 at
/// `rp(655360, 0, 0)`, creature 123 and placement 113 at `rp(0, 0, 655360)`.
pub fn moving() -> Fixture {
    let mut fixture = Fixture::standard();
    fixture.ruleset.stats.push(StatDecl {
        name: "pace".to_owned(),
        kind: StatKindWire::Fixed,
        derived: None,
    });
    fixture.ruleset.pools.push(ActionPoolTemplate {
        id: ulid(106),
        max: 6,
        refresh: RefreshWire::OnTurnStart,
    });
    fixture.ruleset.movement = Some(Box::new(MovementWire {
        speed: LengthWire::Stat("pace".to_owned()),
        measure: None,
        turn: TurnMovementWire {
            pool: ulid(106),
            step: LengthWire::Constant(Fx16_16::from_raw(98_304)),
            step_cost: 1,
        },
    }));
    set_creature_stat(&mut fixture, 121, "pace", 65_536);
    set_creature_stat(&mut fixture, 122, "pace", 32_768);
    fixture
        .placements
        .get_mut(&ulid(112))
        .expect("placement B")
        .0
        .transform
        .position = rp(655_360, 0, 0);

    // Creature 123 and placement 113 (not a participant yet).
    let mut creature_c = fixture.creatures[&ulid(121)].clone();
    creature_c.id = ulid(123);
    creature_c.slug = "probe-creature-c".to_owned();
    creature_c.name = "probe.creature-c".to_owned();
    for stat in ["alpha", "beta", "gamma"] {
        creature_c
            .stats
            .insert(stat.to_owned(), Fx16_16::from_int(5));
    }
    creature_c
        .stats
        .insert("health".to_owned(), Fx16_16::from_int(4));
    creature_c
        .stats
        .insert("pace".to_owned(), Fx16_16::from_raw(65_536));
    creature_c.tags.clear();
    fixture.creatures.insert(ulid(123), creature_c);
    let mut doc_c = fixture.placements[&ulid(111)].0.clone();
    doc_c.id = ulid(113);
    doc_c.slug = "probe-c".to_owned();
    doc_c.name = "probe.c".to_owned();
    doc_c.prefab = ulid(123);
    doc_c.transform.position = rp(0, 0, 655_360);
    let area = fixture.area;
    fixture.placements.insert(ulid(113), (doc_c, area));
    fixture
}

/// `moving_rt()`: `moving()` with `round_ticks: 4`, so real time may start.
pub fn moving_rt() -> Fixture {
    let mut fixture = moving();
    fixture.ruleset.round_ticks = Some(4);
    fixture
}

/// `ranged(f, r)`: ability 103's `range = Some(raw r)`.
pub fn ranged(mut fixture: Fixture, range: i32) -> Fixture {
    fixture
        .abilities
        .get_mut(&ulid(103))
        .expect("ability 103 exists")
        .range = Some(Fx16_16::from_raw(range));
    fixture
}

/// `far(f)`: placement 112 at `rp(1310720, 0, 0)`.
pub fn far(mut fixture: Fixture) -> Fixture {
    fixture
        .placements
        .get_mut(&ulid(112))
        .expect("placement B")
        .0
        .transform
        .position = rp(1_310_720, 0, 0);
    fixture
}

/// `near(f)`: placement 112 at `rp(65536, 0, 0)`.
pub fn near(mut fixture: Fixture) -> Fixture {
    fixture
        .placements
        .get_mut(&ulid(112))
        .expect("placement B")
        .0
        .transform
        .position = rp(65_536, 0, 0);
    fixture
}

/// `trio(f)`: participants gain `{placement 113, initiative 11}`.
pub fn trio(mut fixture: Fixture) -> Fixture {
    assert!(
        fixture.placements.contains_key(&ulid(113)),
        "trio needs the moving fixture"
    );
    fixture.encounter.participants.push(EncounterParticipant {
        placement: ulid(113),
        initiative: 11,
    });
    fixture
}

/// `lethal(f)`: ability 103's two damage amounts become 99.
pub fn lethal(mut fixture: Fixture) -> Fixture {
    let ability = fixture
        .abilities
        .get_mut(&ulid(103))
        .expect("ability 103 exists");
    for entry in &mut ability.damage {
        entry.amount = 99;
    }
    fixture
}

/// `shared(f)`: pool 105 max 7, and the movement pool also 105.
pub fn shared(mut fixture: Fixture) -> Fixture {
    fixture
        .ruleset
        .pools
        .iter_mut()
        .find(|pool| pool.id == ulid(105))
        .expect("pool 105 exists")
        .max = 7;
    fixture
        .ruleset
        .movement
        .as_mut()
        .expect("the fixture moves")
        .turn
        .pool = ulid(105);
    fixture
}

/// `twin(f)`: ability 107, a copy of 103 (as it is now), listed after 103.
pub fn twin(mut fixture: Fixture) -> Fixture {
    let mut copy = fixture
        .abilities
        .get(&ulid(103))
        .expect("ability 103 exists")
        .clone();
    copy.id = ulid(107);
    copy.slug = "probe-attack-2".to_owned();
    copy.name = "probe.attack-2".to_owned();
    fixture.abilities.insert(copy.id, copy);
    fixture.ruleset.abilities.push(ulid(107));
    fixture
}

/// `tune(f, ability, r, cost)`: that ability's range and primary-pool cost.
pub fn tune(mut fixture: Fixture, ability: u128, range: i32, cost: u32) -> Fixture {
    let entry = fixture
        .abilities
        .get_mut(&ulid(ability))
        .expect("the ability exists");
    entry.range = Some(Fx16_16::from_raw(range));
    entry.cost = cost;
    let max = fixture
        .ruleset
        .pools
        .first()
        .expect("the ruleset lists a pool")
        .max;
    assert!(
        (1..=max).contains(&cost),
        "a primary-pool cost must fit the pool"
    );
    fixture
}

/// `reversed(f)`: the ruleset lists its abilities in reverse order, so the
/// larger ULID is first in `definition.abilities` (M-5 sorts by ULID).
pub fn reversed_abilities(mut fixture: Fixture) -> Fixture {
    fixture.ruleset.abilities.reverse();
    fixture
}

/// `step_cost(f, cost)`: the movement rules' per-step pool cost.
pub fn step_cost(mut fixture: Fixture, cost: u32) -> Fixture {
    let turn = &mut fixture
        .ruleset
        .movement
        .as_mut()
        .expect("the fixture moves")
        .turn;
    assert!(cost > 0, "a step costs at least one pool unit");
    turn.step_cost = cost;
    fixture
}

/// `extra_cost(f, ability, pool, amount)`: one non-primary pool cost entry.
pub fn extra_cost(mut fixture: Fixture, ability: u128, pool: u128, amount: u32) -> Fixture {
    fixture
        .abilities
        .get_mut(&ulid(ability))
        .expect("the ability exists")
        .extra_costs
        .push(AbilityCost {
            pool: ulid(pool),
            amount,
        });
    fixture
}

/// `place_c(f, x, z)`: placement 113 at `p(x, z)`.
pub fn place_c(mut fixture: Fixture, x: i32, z: i32) -> Fixture {
    fixture
        .placements
        .get_mut(&ulid(113))
        .expect("the moving fixture has placement 113")
        .0
        .transform
        .position = p(x, z);
    fixture
}
