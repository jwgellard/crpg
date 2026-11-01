//! T139a1: records across areas and the journaled registration (Behaviour
//! B8 and B10; tests A7 and A9). T139a2 extends it with the lifecycle at
//! start and release (B11-B20; tests A7's release sequence, A10-A16 and
//! A19-A25).

#[allow(dead_code)]
mod support;

use std::error::Error as _;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_data::{CreaturePersistenceWire, Effect, EffectsOnExitWire};
use crpg_rules::MAX_TAGS;
use crpg_sim::{
    end_encounter, history_hash, perform_action, state_hash, transfer_entity,
    transfer_history_entity, AreaError, AttachedEffect, CombatAction, CombatError, Combatant,
    EncounterSpec, EntityMeta, HistoryError, HistoryEvent, HistoryWorld, SimEvent, SuspendedEffect,
    Transform, World,
};
use serde_json::{json, Value};
use support::creature::{add_outsider, edit_combatant, record, set_hooks, with_json, with_record};
use support::draws::{draw, grant, oracle, set_initiative_rule, set_tags, start_into};
use support::rt::{tick_pools, timeline_of, value_of};
use support::surface::{
    act, add_derived, add_stat, always_success, attach_to_ability, binary, edit_ability,
    effect_doc, end, lit, modifier, set_stat, set_stat_raw, stat,
};
use support::{uid, Fixture};

fn area_x(fixture: &Fixture) -> Ulid {
    fixture.area
}

fn area_y() -> Ulid {
    Ulid::from_u128(132)
}

/// Two bound worlds, as plain worlds or as history wrappers, so that every
/// transfer case runs through both functions.
enum Duo {
    Plain(World, World),
    History(HistoryWorld, HistoryWorld),
}

impl Duo {
    fn new(fixture: &Fixture, history: bool) -> Self {
        if history {
            Self::History(
                HistoryWorld::new_in_area(1, area_x(fixture)),
                HistoryWorld::new_in_area(2, area_y()),
            )
        } else {
            Self::Plain(
                World::new_in_area(1, area_x(fixture)),
                World::new_in_area(2, area_y()),
            )
        }
    }

    fn x(&self) -> &World {
        match self {
            Self::Plain(x, _) => x,
            Self::History(x, _) => x.world(),
        }
    }

    fn y(&self) -> &World {
        match self {
            Self::Plain(_, y) => y,
            Self::History(_, y) => y.world(),
        }
    }

    /// Both authorities' bytes, journals included for the history pair.
    fn bytes(&self) -> (Vec<u8>, Vec<u8>) {
        match self {
            Self::Plain(x, y) => (
                serde_json::to_vec(x).expect("serializes"),
                serde_json::to_vec(y).expect("serializes"),
            ),
            Self::History(x, y) => (
                serde_json::to_vec(x).expect("serializes"),
                serde_json::to_vec(y).expect("serializes"),
            ),
        }
    }

    fn register_x(&mut self, fixture: &Fixture, placement: Ulid) -> EntityId {
        match self {
            Self::Plain(x, _) => fixture.register(x, placement).expect("registers"),
            Self::History(x, _) => fixture.register_history(x, placement).expect("registers"),
        }
    }

    fn spawn_bare_x(&mut self) -> EntityId {
        match self {
            Self::Plain(x, _) => x.spawn(EntityMeta {}),
            Self::History(x, _) => x.spawn(EntityMeta {}).expect("spawns"),
        }
    }

    fn start_x(&mut self, fixture: &Fixture) -> Vec<EntityId> {
        fixture.with_spec(|spec: &EncounterSpec<'_>| match self {
            Self::Plain(x, _) => crpg_sim::start_encounter(x, spec).expect("starts"),
            Self::History(x, _) => x.start_encounter(spec).expect("starts"),
        });
        self.x().combatants().iter().map(|(id, _)| id).collect()
    }

    fn forward(&mut self, entity: EntityId) -> Result<EntityId, AreaError> {
        match self {
            Self::Plain(x, y) => transfer_entity(x, y, entity),
            Self::History(x, y) => transfer_history_entity(x, y, entity),
        }
    }

    fn back(&mut self, entity: EntityId) -> Result<EntityId, AreaError> {
        match self {
            Self::Plain(x, y) => transfer_entity(y, x, entity),
            Self::History(x, y) => transfer_history_entity(y, x, entity),
        }
    }

    /// Replaces the transform of the entity in X with a raw JSON one.
    fn set_transform_x(&mut self, entity: EntityId, transform: serde_json::Value) {
        let edit = |value: &mut serde_json::Value, at: &[&str]| {
            let mut slot = value;
            for key in at {
                slot = &mut slot[*key];
            }
            *slot = json!([[serde_json::to_value(entity).expect("id"), transform]]);
        };
        match self {
            Self::Plain(x, _) => {
                let mut value = serde_json::to_value(&*x).expect("serializes");
                edit(&mut value, &["transforms"]);
                *x = serde_json::from_value(value).expect("loads");
            }
            Self::History(x, _) => {
                let mut value = serde_json::to_value(&*x).expect("serializes");
                edit(&mut value, &["world", "transforms"]);
                *x = serde_json::from_value(value).expect("loads");
            }
        }
    }

    /// Releases the encounter in X.
    fn release_x(&mut self) {
        match self {
            Self::Plain(x, _) => {
                end_encounter(x).expect("releases");
            }
            Self::History(x, _) => {
                x.end_encounter().expect("releases");
            }
        }
    }

    /// Performs one accepted action in X.
    fn act_x(&mut self, action: &crpg_sim::CombatAction) {
        match self {
            Self::Plain(x, _) => {
                crpg_sim::perform_action(x, action).expect("the action is accepted");
            }
            Self::History(x, _) => {
                x.perform_action(action).expect("the action is accepted");
            }
        }
    }

    fn rejects(&mut self, entity: EntityId, forward: bool, expected: AreaError) {
        let before = self.bytes();
        let result = if forward {
            self.forward(entity)
        } else {
            self.back(entity)
        };
        assert_eq!(result, Err(expected.clone()));
        assert_eq!(
            self.bytes(),
            before,
            "{expected:?} mutated a world or journal"
        );
    }
}

/// A7 (B8): the record moves with its entity, byte-equal.
#[test]
fn a7_transfer_carries_record() {
    for history in [false, true] {
        let mut fixture = Fixture::standard();
        add_outsider(&mut fixture, 131, 141);
        let mut duo = Duo::new(&fixture, history);
        let entity = duo.register_x(&fixture, Ulid::from_u128(131));
        let source_record = duo.x().creatures().get(entity).expect("record").clone();
        let moved = duo.forward(entity).expect("a record moves");
        assert_eq!(duo.y().creatures().get(moved), Some(&source_record));
        assert!(duo.x().creatures().is_empty(), "the source loses it");
        assert_eq!(duo.y().creatures().len(), 1);

        // Control: a bare entity puts nothing in the destination store.
        let bare = duo.spawn_bare_x();
        let moved_bare = duo.forward(bare).expect("a bare entity moves");
        assert!(duo.y().creatures().get(moved_bare).is_none());
        assert_eq!(duo.y().creatures().len(), 1);

        // Control: a combatant still fails `CombatParticipant`.
        let combatants = duo.start_x(&fixture);
        duo.rejects(
            combatants[0],
            true,
            AreaError::CombatParticipant {
                entity: combatants[0],
            },
        );
    }
}

/// A7 (B8, T1): a record collision is refused for both functions, with both
/// worlds and journals unchanged.
#[test]
fn a7_t1_record_collision() {
    for history in [false, true] {
        let fixture = Fixture::standard();
        let placement = fixture.placement_a;
        let mut duo = Duo::new(&fixture, history);
        let first = duo.register_x(&fixture, placement);
        duo.forward(first).expect("the first record moves");
        // R5 passes: X no longer holds the placement.
        let second = duo.register_x(&fixture, placement);
        let expected = AreaError::PlacementInUse { placement };
        duo.rejects(second, true, expected.clone());
        assert_eq!(expected.to_string(), "PlacementInUse at area/transfer");
        assert!(expected.source().is_none());
        assert_eq!(duo.y().creatures().len(), 1);
        assert_eq!(duo.x().creatures().len(), 1);

        // Precedence: a source holder with a non-finite transform fails
        // `InvalidTransform`, not `PlacementInUse`.
        duo.set_transform_x(
            second,
            json!({"position": [0.0, 0.0, 1e40], "velocity": [0.0, 0.0, 0.0]}),
        );
        duo.rejects(second, true, AreaError::InvalidTransform { entity: second });
    }
}

/// A7 (B8, T1): a combatant collision is refused for both functions.
#[test]
fn a7_t1_combatant_collision() {
    for history in [false, true] {
        let fixture = Fixture::standard();
        let placement = fixture.placement_a;
        let mut duo = Duo::new(&fixture, history);
        let record = duo.register_x(&fixture, placement);
        let moved = duo.forward(record).expect("the record moves");
        // X holds no record for A, so the start spawns A fresh there.
        let combatants = duo.start_x(&fixture);
        assert_eq!(combatants.len(), 2);
        duo.rejects(moved, false, AreaError::PlacementInUse { placement });
        assert_eq!(duo.y().creatures().len(), 1);

        // Control: after the combatant is gone from X, the record returns.
        match &mut duo {
            Duo::Plain(x, _) => assert!(x.despawn(combatants[0])),
            Duo::History(x, _) => assert!(x.despawn(combatants[0]).expect("despawns")),
        }
        duo.back(moved).expect("moves once the placement is free");
        assert_eq!(duo.x().creatures().len(), 1);
    }
}

/// A7 (B8, T1): a dead combatant still collides ("dead or alive").
#[test]
fn a7_t1_dead_combatant_collision() {
    for history in [false, true] {
        let mut fixture = Fixture::standard();
        support::rt::make_lethal(&mut fixture, 5);
        let creature_a = fixture.creature_a;
        support::surface::set_stat(&mut fixture, creature_a, "health", 2);
        let placement = fixture.placement_a;
        let mut duo = Duo::new(&fixture, history);
        let record = duo.register_x(&fixture, placement);
        let moved = duo.forward(record).expect("the record moves");
        let combatants = duo.start_x(&fixture);
        // B acts first and kills A.
        duo.act_x(&support::rt::use_action(
            combatants[1],
            fixture.ability,
            combatants[0],
        ));
        assert!(duo.x().combatants().get(combatants[0]).expect("A").dead());
        duo.rejects(moved, false, AreaError::PlacementInUse { placement });
        assert_eq!(duo.y().creatures().len(), 1);
    }
}

/// A9 (B10): journaled registration journals exactly one `Spawned`, keeps
/// the version, and a rejection commits nothing.
#[test]
fn a9_history_registration() {
    let mut fixture = Fixture::standard();
    add_outsider(&mut fixture, 131, 141);
    let mut history = HistoryWorld::new_in_area(5, fixture.area);
    let before = serde_json::to_vec(&history).expect("serializes");
    // A rejected registration (R3) leaves an empty journal.
    let rejected = fixture.with_creature_spec(Ulid::from_u128(131), |spec| {
        let spec = crpg_sim::CreatureSpec {
            creature: &fixture.creatures[&fixture.creature_b],
            ..*spec
        };
        history.register_creature(&spec)
    });
    assert_eq!(
        rejected,
        Err(HistoryError::Combat(CombatError::MissingCreature {
            prefab: Ulid::from_u128(141)
        }))
    );
    assert_eq!(serde_json::to_vec(&history).expect("serializes"), before);
    assert_eq!(history.last_sequence(), 0);

    let entity = fixture
        .register_history(&mut history, Ulid::from_u128(131))
        .expect("registers");
    let page = history.read_after(0, 16).expect("reads");
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].payload, HistoryEvent::Spawned { entity });
    assert_eq!(history.version(), 1, "records do not upgrade the wrapper");
    assert!(history.world().creatures().get(entity).is_some());
    assert!(
        history.world().events().is_empty(),
        "no inner queue residue"
    );

    // Control: a version-1 wrapper holding a record loads, re-serializes
    // byte for byte and hashes unchanged.
    let saved = serde_json::to_vec(&history).expect("serializes");
    let loaded: HistoryWorld = serde_json::from_slice(&saved).expect("a v1 wrapper loads");
    assert_eq!(loaded.version(), 1);
    assert_eq!(serde_json::to_vec(&loaded).expect("serializes"), saved);
    assert_eq!(history_hash(&loaded), history_hash(&history));
}
// ---------------------------------------------------------------------------
// T139a2: the lifecycle at start and release (B11-B20). Tests: A7's release
// sequence, A10-A16 and A19-A25. A17 and A18 are in `creature_state.rs`.
// ---------------------------------------------------------------------------

fn id(value: u128) -> Ulid {
    Ulid::from_u128(value)
}

fn bytes(world: &World) -> Vec<u8> {
    serde_json::to_vec(world).expect("the world serializes")
}

/// The whole world, for complete-state equality.
fn state(world: &World) -> (Vec<u8>, [u8; 32]) {
    (bytes(world), state_hash(world))
}

fn bound(fixture: &Fixture) -> World {
    World::new_in_area(5, fixture.area)
}

fn events_of(world: &mut World) -> Vec<SimEvent> {
    world
        .events_mut()
        .drain()
        .into_iter()
        .map(|envelope| envelope.payload)
        .collect()
}

fn combatant_ids(world: &World) -> Vec<EntityId> {
    world
        .combatants()
        .iter()
        .map(|(entity, _)| entity)
        .collect()
}

/// Starts the fixture's encounter into `world` and unwraps.
fn begin(fixture: &Fixture, world: &mut World) {
    start_into(fixture, world).expect("the encounter starts");
}

/// A start that must fail with `expected`, leaving `world` unchanged.
fn begin_fails(fixture: &Fixture, world: &mut World, expected: CombatError) {
    let before = state(world);
    assert_eq!(start_into(fixture, world), Err(expected.clone()));
    assert_eq!(state(world), before, "{expected:?} mutated the world");
}

/// Gives every creature of the fixture a base `Fixed` stat `speed` (D6).
fn with_speed(fixture: &mut Fixture) {
    add_stat(fixture, json!({"name": "speed", "kind": "fixed"}));
    let creatures: Vec<Ulid> = fixture.creatures.keys().copied().collect();
    for creature in creatures {
        set_stat_raw(fixture, creature, "speed", 98_304);
    }
}

/// The effect (identity 501) that the standard ability applies to its target.
fn effect_501(duration: u32) -> Effect {
    effect_doc(
        501,
        "target",
        "probe-type",
        "stack_all",
        vec![modifier(601, json!({"type": "roll"}), "add", 1)],
        duration,
    )
}

/// A11's fixture: creature A has health 2.
fn a_dies_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    let creature_a = fixture.creature_a;
    set_stat(&mut fixture, creature_a, "health", 2);
    fixture
}

/// A11's setup: `fixture.start(3)` gives `[A, B]`, and B kills A.
fn a_dies(fixture: &Fixture) -> (World, Vec<EntityId>) {
    let (mut world, ids) = fixture.start(3);
    let kill = CombatAction::UseAbility {
        actor: ids[1],
        ability: fixture.ability,
        target: ids[0],
    };
    let outcome = perform_action(&mut world, &kill)
        .expect("the kill is accepted")
        .expect("an ability use reports an outcome");
    assert!(outcome.target_died);
    (world, ids)
}

/// Asserts that `entity`'s record is exactly `before`'s combatant values.
fn assert_written_back(world: &World, entity: EntityId, before: &Combatant, ruleset: Ulid) {
    let record = world.creatures().get(entity).expect("a record");
    assert_eq!(record.placement(), before.placement());
    assert_eq!(record.ruleset(), ruleset);
    assert_eq!(record.attributes(), before.attributes());
    assert_eq!(record.fixed(), before.fixed());
    assert_eq!(
        (record.health(), record.max_health()),
        (before.health(), before.max_health())
    );
    let mut pools = vec![before.action_pool().clone()];
    pools.extend(before.extra_pools().iter().cloned());
    assert_eq!(record.pools(), pools);
    let mut tags = before.tags().to_vec();
    tags.sort();
    tags.dedup();
    assert_eq!(record.tags(), tags);
}

/// A7 (B8, T1): the review's sequence, for both functions. After the first
/// record moved to Y, an encounter in X spawns A fresh, its release writes A
/// back in X, and moving that record to Y is refused with both worlds and
/// journals unchanged.
#[test]
fn a7_t1_release_sequence() {
    for history in [false, true] {
        let fixture = Fixture::standard();
        let placement = fixture.placement_a;
        let mut duo = Duo::new(&fixture, history);
        let first = duo.register_x(&fixture, placement);
        duo.forward(first).expect("the first record moves");
        // X holds no record for A, so the start spawns A fresh.
        let combatants = duo.start_x(&fixture);
        assert_eq!(combatants.len(), 2);
        duo.release_x();
        let written = duo
            .x()
            .creatures()
            .get(combatants[0])
            .expect("A is written back");
        assert_eq!(written.placement(), placement);
        assert_eq!(duo.x().creatures().len(), 2, "both survivors are records");
        duo.rejects(combatants[0], true, AreaError::PlacementInUse { placement });
        assert_eq!(duo.y().creatures().len(), 1);
        assert_eq!(duo.x().creatures().len(), 2);
    }
}

/// A10 (B16, B17): survivors persist at release, exactly as their combatants
/// were. Controls: `respawns` and a `registered` ruleset keep nothing.
#[test]
fn a10_survivors_persist_at_release() {
    let mut fixture = Fixture::standard();
    always_success(&mut fixture);
    with_speed(&mut fixture);
    let (mut world, ids) = fixture.start(3);
    for entity in &ids {
        world.transforms_mut().insert(*entity, Transform::default());
    }
    // B damages A; A answers, spending its primary pool.
    act(&mut world, ids[1], fixture.ability, ids[0]);
    act(&mut world, ids[0], fixture.ability, ids[1]);
    let before: Vec<Combatant> = ids
        .iter()
        .map(|entity| world.combatants().get(*entity).expect("combatant").clone())
        .collect();
    assert_eq!(before[0].health(), 8);
    assert_eq!(before[0].action_pool().current(), 0, "A's pool is spent");
    assert_eq!(before[0].fixed().len(), 1);
    events_of(&mut world);

    let summary = end_encounter(&mut world).expect("releases");
    assert_eq!(summary.results.len(), 2);
    assert_eq!(world.creatures().len(), 2);
    for (entity, combatant) in ids.iter().zip(&before) {
        assert_written_back(&world, *entity, combatant, id(102));
        let record = world.creatures().get(*entity).expect("record");
        assert!(record.suspended().is_empty());
        assert!(world.contains(*entity), "the entity stays live");
        assert_eq!(
            world.transforms().get(*entity),
            Some(&Transform::default()),
            "its transform stays"
        );
    }
    assert!(world.combatants().is_empty() && world.combat().is_none());
    assert!(world.events().is_empty(), "release enqueues nothing");

    // Controls: an encounter that respawns, and a registered ruleset, keep
    // nothing.
    let mut respawning = Fixture::standard();
    respawning.encounter.respawns = Some(true);
    let (mut world, _) = respawning.start(3);
    end_encounter(&mut world).expect("releases");
    assert!(world.creatures().is_empty());
    let mut registered = Fixture::standard();
    set_hooks(
        &mut registered,
        Some(CreaturePersistenceWire::Registered),
        None,
        None,
    );
    let (mut world, _) = registered.start(3);
    end_encounter(&mut world).expect("releases");
    assert!(world.creatures().is_empty());
}

/// A11 (B13, B15, B18): the dead stay dead. Control: `respawns` spawns two
/// fresh entities and leaves both records untouched.
#[test]
fn a11_dead_stay_dead() {
    let fixture = a_dies_fixture();
    let (mut world, ids) = a_dies(&fixture);
    let summary = end_encounter(&mut world).expect("releases");
    assert_eq!(summary.results.len(), 2);
    let record_a = world.creatures().get(ids[0]).expect("A's record").clone();
    assert_eq!(record_a.health(), 0);
    assert!(record_a.dead());
    let record_b = world.creatures().get(ids[1]).expect("B's record").clone();
    assert_eq!(record_b.health(), 6);

    // Control: a respawning restart spawns two fresh entities and reads no
    // record.
    let mut respawning = a_dies_fixture();
    respawning.encounter.respawns = Some(true);
    let mut control = world.clone();
    events_of(&mut control);
    begin(&respawning, &mut control);
    let fresh = combatant_ids(&control);
    assert_eq!(fresh.len(), 2);
    assert!(fresh.iter().all(|entity| !ids.contains(entity)));
    assert_eq!(events_of(&mut control).len(), 2, "two Spawned");
    assert_eq!(control.creatures().get(ids[0]), Some(&record_a));
    assert_eq!(control.creatures().get(ids[1]), Some(&record_b));

    // The restart under the default: only B, with its old identity.
    events_of(&mut world);
    begin(&fixture, &mut world);
    assert_eq!(combatant_ids(&world), [ids[1]]);
    assert!(events_of(&mut world).is_empty(), "no Spawned is enqueued");
    let survivor = world.combatants().get(ids[1]).expect("B");
    assert_eq!(survivor.health(), 6);
    assert!(survivor.entered());
    assert_eq!(world.creatures().len(), 1, "A's dead record stays");
    assert_eq!(world.creatures().get(ids[0]), Some(&record_a));
    let summary = end_encounter(&mut world).expect("releases");
    assert_eq!(summary.results.len(), 1);
    assert_eq!(summary.results[0].entity, ids[1]);
}

/// A12 (B13): no entrant fails `NoEntrants`, unchanged. Control: with one
/// record alive the start succeeds with one combatant.
#[test]
fn a12_no_entrants() {
    let fixture = Fixture::standard();
    let mut world = bound(&fixture);
    let a = fixture
        .register(&mut world, fixture.placement_a)
        .expect("A");
    let b = fixture
        .register(&mut world, fixture.placement_b)
        .expect("B");
    let one_dead = with_record(&world, a, |record| record["health"] = json!(0));
    let mut both_dead = with_record(&one_dead, b, |record| record["health"] = json!(0));
    begin_fails(&fixture, &mut both_dead, CombatError::NoEntrants);

    let mut control = one_dead;
    begin(&fixture, &mut control);
    assert_eq!(combatant_ids(&control), [b]);
}

fn a13_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    tick_pools(&mut fixture, 4, &[3, 7]);
    with_speed(&mut fixture);
    fixture.effects.insert(id(501), effect_501(3));
    edit_ability(&mut fixture, |ability| ability.effect = Some(id(501)));
    fixture
}

/// Edits A's record as A13 describes.
fn a13_edit(world: &World, a: EntityId) -> World {
    with_record(world, a, |record| {
        record["health"] = json!(4);
        record["pools"][0]["current"] = json!(0);
        record["pools"][1]["current"] = json!(1);
        record["fixed"] = json!([["speed", 131_072]]);
        record["suspended"] = json!([
            {"effect": uid(501), "remaining_rounds": 2},
            {"effect": uid(502), "remaining_rounds": 1}
        ]);
    })
}

/// A13 (B14, B15): A enters from its record, B spawns. Control: with B
/// registered too, nothing spawns.
#[test]
fn a13_mixed_start_seeding() {
    let fixture = a13_fixture();
    let mut world = bound(&fixture);
    let a = fixture
        .register(&mut world, fixture.placement_a)
        .expect("A");
    let mut world = a13_edit(&world, a);
    events_of(&mut world);
    let recorded = world.creatures().get(a).expect("A's record").clone();
    begin(&fixture, &mut world);

    let ids = combatant_ids(&world);
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], a, "A keeps its entity and heads the store");
    let b = ids[1];
    assert_ne!(b, a);
    assert_eq!(timeline_of(&world), [(9, b), (10, a)]);
    assert_eq!(world.combat().expect("combat").active, Some(b));
    assert_eq!(events_of(&mut world), [SimEvent::Spawned { entity: b }]);

    let entered = world.combatants().get(a).expect("A");
    assert_eq!((entered.health(), entered.max_health()), (4, 10));
    assert_eq!(entered.action_pool(), &recorded.pools()[0]);
    assert_eq!(entered.extra_pools(), &recorded.pools()[1..]);
    assert_eq!(entered.action_pool().current(), 0, "unrefreshed");
    assert_eq!(
        entered.attached(),
        [AttachedEffect {
            effect: id(501),
            expires_round: 2
        }],
        "the effect the definition lacks is dropped"
    );
    assert!(entered.entered());
    assert_eq!(
        entered.fixed(),
        [("speed".to_owned(), Fx16_16::from_raw(131_072))]
    );
    assert!(world.creatures().is_empty(), "A has no record");
    let spawned = world.combatants().get(b).expect("B");
    assert!(!spawned.entered());
    assert_eq!(
        spawned.fixed(),
        [("speed".to_owned(), Fx16_16::from_raw(98_304))]
    );

    // Control: with B also registered first, no Spawned is enqueued.
    let mut world = bound(&fixture);
    let a = fixture
        .register(&mut world, fixture.placement_a)
        .expect("A");
    let b = fixture
        .register(&mut world, fixture.placement_b)
        .expect("B");
    let mut world = a13_edit(&world, a);
    events_of(&mut world);
    begin(&fixture, &mut world);
    assert_eq!(combatant_ids(&world), [a, b]);
    assert!(events_of(&mut world).is_empty());
}

/// A14 (B13): S1 and S2 refuse the entry, the world unchanged. Pair: a
/// `MissingPlacement` in the spec beats S1.
#[test]
fn a14_entry_checks() {
    let mut fixture = Fixture::standard();
    // `alpha / (health - 4)`: fine at the prefab's health, not at 4.
    add_derived(
        &mut fixture,
        "ratio",
        binary(
            "divide",
            stat("alpha"),
            binary("subtract", stat("health"), lit(4)),
        ),
    );
    let mut world = bound(&fixture);
    let a = fixture
        .register(&mut world, fixture.placement_a)
        .expect("A");

    // Control: the unedited record enters.
    let mut control = world.clone();
    begin(&fixture, &mut control);
    assert!(control.combatants().get(a).expect("A").entered());

    let mismatch = CombatError::RulesetMismatch { entity: a };
    type Edit = Box<dyn Fn(&mut Value)>;
    let mut cases: Vec<(Edit, CombatError)> = Vec::new();
    cases.push((
        Box::new(|record| record["ruleset"] = json!(uid(999))),
        mismatch.clone(),
    ));
    cases.push((
        Box::new(|record| {
            record["attributes"] = json!([["beta", 6], ["alpha", 8], ["gamma", 7]]);
        }),
        mismatch.clone(),
    ));
    cases.push((
        Box::new(|record| record["pools"][0]["max"] = json!(5)),
        mismatch,
    ));
    cases.push((
        Box::new(|record| record["health"] = json!(4)),
        CombatError::InvalidStatValue {
            stat: "ratio".to_owned(),
        },
    ));
    for (edit, expected) in cases {
        let mut edited = with_record(&world, a, |record| edit(record));
        begin_fails(&fixture, &mut edited, expected);
    }

    // Pair: a missing placement in the spec beats S1.
    let mut missing = Fixture::standard();
    let placement_b = missing.placement_b;
    missing.placements.remove(&placement_b);
    let mut edited = with_record(&world, a, |record| record["ruleset"] = json!(uid(999)));
    begin_fails(
        &missing,
        &mut edited,
        CombatError::MissingPlacement {
            placement: placement_b,
        },
    );
}

/// Runs one A15 script: B's attack applies effect 501 (`duration` rounds) to
/// A at round 0; with `second_round` A ends its turn and the round rolls
/// over. Returns the released world and its summary round.
fn release_with_effect(
    exit: Option<EffectsOnExitWire>,
    duration: u32,
    second_round: bool,
) -> (World, Vec<EntityId>, u32) {
    let mut fixture = Fixture::standard();
    always_success(&mut fixture);
    fixture.effects.insert(id(501), effect_501(duration));
    edit_ability(&mut fixture, |ability| ability.effect = Some(id(501)));
    set_hooks(&mut fixture, None, None, exit);
    let (mut world, ids) = fixture.start(3);
    act(&mut world, ids[1], fixture.ability, ids[0]);
    assert_eq!(
        world.combatants().get(ids[0]).expect("A").attached().len(),
        1
    );
    if second_round {
        end(&mut world, ids[0]);
    }
    let summary = end_encounter(&mut world).expect("releases");
    (world, ids, summary.round)
}

/// A15 (B16): effects at release follow `effects_on_exit`. Control: an
/// attachment that lapsed by the release round is not stored.
#[test]
fn a15_effects_at_release() {
    let held = SuspendedEffect {
        effect: id(501),
        remaining_rounds: 2,
    };
    for (exit, expected) in [
        (None, vec![held]),
        (Some(EffectsOnExitWire::Suspend), vec![held]),
        (Some(EffectsOnExitWire::Strip), vec![]),
        (Some(EffectsOnExitWire::StripAtRelease), vec![]),
    ] {
        let (world, ids, round) = release_with_effect(exit, 3, true);
        assert_eq!(round, 1);
        let stored = world
            .creatures()
            .get(ids[0])
            .expect("A's record")
            .suspended();
        assert_eq!(stored, expected, "{exit:?}");
    }
    // The boundary: one round left at round 0 is stored; at round 1 the
    // rollover has lapsed it, so nothing is stored.
    let (world, ids, round) = release_with_effect(None, 1, false);
    assert_eq!(round, 0);
    let stored = world.creatures().get(ids[0]).expect("record").suspended();
    assert_eq!(
        stored,
        [SuspendedEffect {
            effect: id(501),
            remaining_rounds: 1
        }]
    );
    let (world, ids, round) = release_with_effect(None, 1, true);
    assert_eq!(round, 1);
    assert!(world
        .creatures()
        .get(ids[0])
        .expect("record")
        .suspended()
        .is_empty());
}

/// A15's control, aimed at the write-back filter itself. The rollover removes
/// lapsed attachments, a save carrying one does not load, and a duration is at
/// least one round, so a lapsed attachment reaches `exit_record` only through
/// saturation: at round `u32::MAX` a new attachment expires at `u32::MAX`.
/// Control: the same script one round earlier stores the effect.
#[test]
fn a15_write_back_drops_lapsed_attachments() {
    for (round, duration, expected) in [
        (u32::MAX, 1, vec![]),
        (u32::MAX, 3, vec![]),
        (
            u32::MAX - 1,
            1,
            vec![SuspendedEffect {
                effect: id(501),
                remaining_rounds: 1,
            }],
        ),
    ] {
        let mut fixture = Fixture::standard();
        always_success(&mut fixture);
        fixture.effects.insert(id(501), effect_501(duration));
        edit_ability(&mut fixture, |ability| ability.effect = Some(id(501)));
        let (world, ids) = fixture.start(3);
        let mut world = with_json(&world, |save| save["combat"]["round"] = json!(round));
        act(&mut world, ids[1], fixture.ability, ids[0]);
        let attached = world.combatants().get(ids[0]).expect("A").attached();
        assert_eq!(attached.len(), 1, "the effect is attached");
        assert_eq!(attached[0].expires_round, round.saturating_add(duration));
        let summary = end_encounter(&mut world).expect("releases");
        assert_eq!(summary.round, round);
        let stored = world.creatures().get(ids[0]).expect("record").suspended();
        assert_eq!(stored, expected, "round {round}, duration {duration}");
    }
}

/// B20: a history start journals `Spawned` for Spawn participants only, plus
/// the initial `TurnStarted`; a release with write-back journals nothing.
#[test]
fn b20_history_journal_at_start_and_release() {
    let fixture = Fixture::standard();
    let mut history = HistoryWorld::new_in_area(5, fixture.area);
    let a = fixture
        .register_history(&mut history, fixture.placement_a)
        .expect("A registers");
    let registered = history.last_sequence();
    assert_eq!(registered, 1, "registration journaled A's Spawned");

    fixture.with_spec(|spec: &EncounterSpec<'_>| history.start_encounter(spec).expect("starts"));
    let combatants = combatant_ids(history.world());
    assert_eq!(combatants.len(), 2);
    assert_eq!(combatants[0], a, "A enters under its own entity");
    let b = combatants[1];
    let active = history.world().combat().expect("combat").active;
    assert_eq!(active, Some(b));
    let page = history.read_after(registered, 16).expect("reads");
    let payloads: Vec<HistoryEvent> = page
        .iter()
        .map(|envelope| envelope.payload.clone())
        .collect();
    assert_eq!(
        payloads,
        [
            HistoryEvent::Spawned { entity: b },
            HistoryEvent::TurnStarted { actor: b, round: 0 }
        ],
        "B spawns, A (an Enter participant) does not, then the first turn"
    );

    let journaled = history.last_sequence();
    let pending = history.pending_len();
    history.end_encounter().expect("releases");
    assert_eq!(history.world().creatures().len(), 2, "write-back happened");
    assert_eq!(history.last_sequence(), journaled);
    assert_eq!(history.pending_len(), pending);
    assert!(history.read_after(journaled, 16).expect("reads").is_empty());
}

/// A16 (B15): an entered participant's initiative bonus reads its record;
/// a skipped participant draws nothing.
#[test]
fn a16_entry_initiative_uses_record() {
    let mut fixture = Fixture::standard();
    set_initiative_rule(&mut fixture, "1d6", &["alpha"]);
    let mut world = bound(&fixture);
    let a = fixture
        .register(&mut world, fixture.placement_a)
        .expect("A");
    let b = fixture
        .register(&mut world, fixture.placement_b)
        .expect("B");
    let world = with_record(&world, a, |record| {
        record["attributes"][0][1] = json!(3);
    });

    // A enters with alpha 3 (the prefab says 8); B enters with its own 6.
    let mut entering = world.clone();
    begin(&fixture, &mut entering);
    let mut expected = oracle(5);
    let roll_a = draw(&mut expected, "1d6");
    let roll_b = draw(&mut expected, "1d6");
    assert_eq!(
        entering.combatants().get(a).expect("A").initiative(),
        roll_a.total + 3 + 10,
        "roll + record alpha + authored key"
    );
    assert_eq!(
        entering.combatants().get(b).expect("B").initiative(),
        roll_b.total + 6 + 9
    );
    assert_eq!(*entering.rng_mut(), expected, "exactly two draws");

    // Control: B's record dead, so B is skipped and draws nothing.
    let mut skipping = with_record(&world, b, |record| record["health"] = json!(0));
    begin(&fixture, &mut skipping);
    let mut expected = oracle(5);
    draw(&mut expected, "1d6");
    assert_eq!(*skipping.rng_mut(), expected, "exactly one draw");
    assert_eq!(combatant_ids(&skipping), [a]);
}

/// A19 (B2, B11, B17): under `registered`, an outsider's record is untouched
/// by an encounter and the participants keep nothing. Control: under the
/// default A and B are written back and the outsider is still untouched.
#[test]
fn a19_registered_policy() {
    for registered in [true, false] {
        let mut fixture = Fixture::standard();
        always_success(&mut fixture);
        add_outsider(&mut fixture, 131, 141);
        if registered {
            set_hooks(
                &mut fixture,
                Some(CreaturePersistenceWire::Registered),
                None,
                None,
            );
        }
        let mut world = bound(&fixture);
        let outsider = fixture.register(&mut world, id(131)).expect("registers");
        let original = record(&world, outsider);
        begin(&fixture, &mut world);
        assert_eq!(record(&world, outsider), original, "after the start");
        let ids = combatant_ids(&world);
        assert_eq!(ids.len(), 2);
        act(&mut world, ids[1], fixture.ability, ids[0]);
        assert_eq!(record(&world, outsider), original, "after an action");
        end_encounter(&mut world).expect("releases");
        assert_eq!(record(&world, outsider), original, "after the release");
        if registered {
            assert_eq!(world.creatures().len(), 1, "only the outsider");
            assert_eq!(world.combat(), None, "the encounter is released");
        } else {
            assert_eq!(world.creatures().len(), 3, "A and B are written back");
        }
    }
}

/// Drives up to `count` accepted actions of the active combatant, alternating
/// an attack and a pass, and records a hash after each.
fn play(world: &mut World, fixture: &Fixture, count: usize, hashes: &mut Vec<[u8; 32]>) {
    for step in 0..count {
        let Some(actor) = world.combat().and_then(|combat| combat.active) else {
            break;
        };
        let action = if step % 2 == 0 {
            let target = combatant_ids(world)
                .into_iter()
                .find(|entity| *entity != actor)
                .expect("a target");
            CombatAction::UseAbility {
                actor,
                ability: fixture.ability,
                target,
            }
        } else {
            CombatAction::EndTurn { actor }
        };
        perform_action(world, &action).expect("the action is accepted");
        hashes.push(state_hash(world));
    }
}

/// One A20 script, optionally saving and reloading after the first release.
fn restart_script(fixture: &Fixture, reload: bool) -> Vec<[u8; 32]> {
    let mut hashes = Vec::new();
    let mut world = bound(fixture);
    begin(fixture, &mut world);
    hashes.push(state_hash(&world));
    play(&mut world, fixture, 10, &mut hashes);
    end_encounter(&mut world).expect("releases");
    hashes.push(state_hash(&world));
    if reload {
        world = serde_json::from_slice(&bytes(&world)).expect("the released world reloads");
        hashes.push(state_hash(&world));
    }
    begin(fixture, &mut world);
    hashes.push(state_hash(&world));
    play(&mut world, fixture, 10, &mut hashes);
    end_encounter(&mut world).expect("releases");
    hashes.push(state_hash(&world));
    hashes
}

/// A20 (B19): restarts are deterministic, and a save/reload after the first
/// release continues with equal hashes.
#[test]
fn a20_restart_determinism() {
    let mut fixture = Fixture::standard();
    for creature in [fixture.creature_a, fixture.creature_b] {
        set_stat(&mut fixture, creature, "health", 100);
    }
    let first = restart_script(&fixture, false);
    let second = restart_script(&fixture, false);
    assert!(first.len() >= 22, "the script is not trivially short");
    assert_eq!(first, second, "two runs give equal per-step hashes");

    let reloaded = restart_script(&fixture, true);
    // The reload step adds one hash; the rest equal the plain run.
    let mut without = reloaded.clone();
    let extra = without.remove(12);
    assert_eq!(extra, first[11], "reloading changes nothing");
    assert_eq!(without, first);
}

fn tagged_fixture(granting: bool) -> Fixture {
    let mut fixture = Fixture::standard();
    let creature_a = fixture.creature_a;
    set_tags(&mut fixture, creature_a, &["b", "a", "a"]);
    if granting {
        attach_to_ability(&mut fixture, grant(effect_501(3), &["g"]));
    }
    fixture
}

/// A21 (B14, D5): an authored Enter participant takes its tags from the
/// record: no prefab read, no grant. Control: with and without a granting
/// effect.
#[test]
fn a21_entered_tags_from_record() {
    for granting in [true, false] {
        let fixture = tagged_fixture(granting);
        let mut world = bound(&fixture);
        let a = fixture
            .register(&mut world, fixture.placement_a)
            .expect("A");
        assert_eq!(world.creatures().get(a).expect("record").tags(), ["a", "b"]);
        let mut world = with_record(&world, a, |record| record["tags"] = json!(["c"]));
        begin(&fixture, &mut world);
        let entered = world.combatants().get(a).expect("A");
        assert!(entered.entered());
        assert_eq!(entered.tags(), ["c"], "granting: {granting}");
    }
}

/// A22 (B16, B17): an entered combatant is written back even when its
/// encounter respawns; a spawned one is not. Control: without the edit both
/// are written back.
#[test]
fn a22_entered_written_back_under_respawns() {
    let fixture = Fixture::standard();
    for respawns in [true, false] {
        let mut world = bound(&fixture);
        let a = fixture
            .register(&mut world, fixture.placement_a)
            .expect("A");
        begin(&fixture, &mut world);
        let b = combatant_ids(&world)[1];
        assert!(world.combatants().get(a).expect("A").entered());
        assert!(!world.combatants().get(b).expect("B").entered());
        if respawns {
            // The state T139b reaches with members of a respawning
            // encounter; no load check rejects it.
            world = with_json(&world, |save| {
                save["combat"]["lifecycle"] = json!({"respawns": true});
            });
        }
        let before = world.combatants().get(a).expect("A").clone();
        end_encounter(&mut world).expect("releases");
        assert_written_back(&world, a, &before, id(102));
        assert!(world.contains(b));
        assert_eq!(
            world.creatures().get(b).is_some(),
            !respawns,
            "B, spawned, is written back only without respawns"
        );
    }
}

/// A23 (B4, B12, B14, B16, D5): tags persist: carried by every combatant,
/// stored sorted and distinct, read from the record at entry. Control: no
/// tags, no key.
#[test]
fn a23_tags_persist() {
    let mut fixture = Fixture::standard();
    let (creature_a, creature_b) = (fixture.creature_a, fixture.creature_b);
    set_tags(&mut fixture, creature_a, &["b", "a", "a"]);
    set_tags(&mut fixture, creature_b, &["solo"]);
    let (mut world, ids) = fixture.start(3);
    assert_eq!(
        world.combatants().get(ids[0]).expect("A").tags(),
        ["a", "b"]
    );
    assert_eq!(world.combatants().get(ids[1]).expect("B").tags(), ["solo"]);
    end_encounter(&mut world).expect("releases");
    assert_eq!(world.creatures().get(ids[0]).expect("A").tags(), ["a", "b"]);
    assert_eq!(world.creatures().get(ids[1]).expect("B").tags(), ["solo"]);

    // The prefab changes; the record wins at entry.
    set_tags(&mut fixture, creature_a, &["x"]);
    begin(&fixture, &mut world);
    assert_eq!(
        world.combatants().get(ids[0]).expect("A").tags(),
        ["a", "b"]
    );

    // A loaded save may hold a combatant's tags unsorted; write-back sorts.
    let mut world = with_json(&world, |save| {
        edit_combatant(save, ids[0], |combatant| {
            combatant["tags"] = json!(["z", "a", "z"]);
        });
    });
    end_encounter(&mut world).expect("releases");
    assert_eq!(world.creatures().get(ids[0]).expect("A").tags(), ["a", "z"]);
    let reloaded: World = serde_json::from_value(value_of(&world)).expect("the save reloads");
    assert_eq!(reloaded, world);

    // Control: no tags, no key on the combatant or the record.
    let plain = Fixture::standard();
    let (mut world, ids) = plain.start(3);
    assert!(!value_of(&world)["combatants"]
        .to_string()
        .contains("\"tags\""));
    end_encounter(&mut world).expect("releases");
    assert!(!record(&world, ids[0]).to_string().contains("tags"));
}

fn names(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("t{index:05}")).collect()
}

/// A24 (B12, B13): the tag bound runs at every start, grants or not, and S3
/// reads the record. Controls: `MAX_TAGS` starts; a grant already among the
/// record's tags enters.
#[test]
fn a24_tag_bound_without_grants() {
    let full = names(MAX_TAGS + 1);
    let creature_tags =
        |count: usize| -> Vec<&str> { full[..count].iter().map(String::as_str).collect() };

    let mut fixture = Fixture::standard();
    let creature_a = fixture.creature_a;
    set_tags(&mut fixture, creature_a, &creature_tags(MAX_TAGS + 1));
    let mut world = World::new(1);
    begin_fails(&fixture, &mut world, CombatError::TooManyTags);
    set_tags(&mut fixture, creature_a, &creature_tags(MAX_TAGS));
    begin(&fixture, &mut World::new(1));

    // S3: a record holding `MAX_TAGS` tags and an encounter that grants one
    // more.
    let granting = |tag: &str| {
        let mut fixture = Fixture::standard();
        attach_to_ability(&mut fixture, grant(effect_501(3), &[tag]));
        fixture
    };
    let fixture = granting("zzz-new");
    let mut world = bound(&fixture);
    let a = fixture
        .register(&mut world, fixture.placement_a)
        .expect("A");
    let mut world = with_record(&world, a, |record| {
        record["tags"] = json!(&full[..MAX_TAGS]);
    });
    begin_fails(&fixture, &mut world, CombatError::TooManyTags);

    // Control: a grant already among the record's tags lets it enter.
    let fixture = granting("t00000");
    begin(&fixture, &mut world);
    let entered = world.combatants().get(a).expect("A");
    assert!(entered.entered());
    assert_eq!(entered.tags().len(), MAX_TAGS);
}

/// A25 (B7, B18, D3): a despawned dead record's placement spawns fresh at the
/// next start. Control: without the despawn the restart has only B (A11).
#[test]
fn a25_despawned_dead_record_respawns() {
    let fixture = a_dies_fixture();
    let (mut world, ids) = a_dies(&fixture);
    end_encounter(&mut world).expect("releases");
    assert!(world.creatures().get(ids[0]).expect("A").dead());
    events_of(&mut world);

    assert!(world.despawn(ids[0]));
    assert_eq!(world.creatures().len(), 1);
    assert!(world.creatures().get(ids[1]).is_some());
    assert_eq!(
        events_of(&mut world),
        [SimEvent::Despawned { entity: ids[0] }]
    );

    // Control: the same restart without the despawn has only B.
    let (mut control, control_ids) = a_dies(&fixture);
    end_encounter(&mut control).expect("releases");
    begin(&fixture, &mut control);
    assert_eq!(combatant_ids(&control), [control_ids[1]]);

    events_of(&mut world);
    begin(&fixture, &mut world);
    let restarted = combatant_ids(&world);
    assert_eq!(restarted.len(), 2);
    let fresh = restarted[0];
    assert_ne!(fresh, ids[0], "A's old identity stays dead");
    assert!(!world.contains(ids[0]));
    assert_eq!(restarted[1], ids[1], "B enters with its old identity");
    assert_eq!(events_of(&mut world), [SimEvent::Spawned { entity: fresh }]);
    let spawned = world.combatants().get(fresh).expect("A, fresh");
    assert!(!spawned.entered());
    assert_eq!((spawned.health(), spawned.max_health()), (2, 2));
    assert_eq!(
        spawned.action_pool().current(),
        spawned.action_pool().max(),
        "pools at max"
    );
    let entered = world.combatants().get(ids[1]).expect("B");
    assert!(entered.entered());
    assert_eq!(entered.health(), 6);
}
