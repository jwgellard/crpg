//! T139a1: records across areas and the journaled registration (Behaviour
//! B8 and B10; tests A7 and A9). A7's release sequence and the lifecycle
//! at start and release are T139a2's.

#[allow(dead_code)]
mod support;

use std::error::Error as _;

use crpg_core::{EntityId, Ulid};
use crpg_sim::{
    history_hash, transfer_entity, transfer_history_entity, AreaError, CombatError, EncounterSpec,
    EntityMeta, HistoryError, HistoryEvent, HistoryWorld, World,
};
use serde_json::json;
use support::creature::add_outsider;
use support::Fixture;

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
