//! Persisted area identity and explicit entity transfer (T027a, ADR-0020).
//!
//! Every fact is observed through public transitions on two bound worlds —
//! spawn, despawn, encounter start/death/release, and the transfer
//! producers — never through fake viewer permissions (those belong to the
//! host and network owners). Every rejection has a positive control and
//! compares both serialized authorities (and hashes, where finite) before and
//! after.

#[allow(dead_code)]
mod support;

use std::error::Error as _;

use crpg_core::{EntityId, Ulid};
use crpg_sim::{
    end_encounter, history_hash, perform_action, start_encounter, state_hash, tick,
    transfer_entity, transfer_history_entity, AreaError, CombatAction, CombatError, EncounterSpec,
    EntityMeta, HistoryError, HistoryEvent, HistoryWorld, InitiativeKey, SimEvent, Transform,
    World,
};
use support::Fixture;

fn area_a() -> Ulid {
    Ulid::from_u128(131)
}

fn area_b() -> Ulid {
    Ulid::from_u128(132)
}

fn bytes(world: &World) -> Vec<u8> {
    serde_json::to_vec(world).expect("the world serializes")
}

fn history_bytes(history: &HistoryWorld) -> Vec<u8> {
    serde_json::to_vec(history).expect("the history world serializes")
}

/// Both authorities' bytes and hashes, for complete-state equality.
fn pair(source: &World, destination: &World) -> (Vec<u8>, Vec<u8>, [u8; 32], [u8; 32]) {
    (
        bytes(source),
        bytes(destination),
        state_hash(source),
        state_hash(destination),
    )
}

fn placed(position: [f32; 3], velocity: [f32; 3]) -> Transform {
    Transform { position, velocity }
}

/// Starts `fixture` in `world`, returning the combatant ids in authored order.
fn start_in(fixture: &Fixture, world: &mut World) -> Result<Vec<EntityId>, CombatError> {
    let bundle = fixture.bundle();
    let spec = EncounterSpec {
        encounter: &fixture.encounter,
        ruleset: &fixture.ruleset,
        abilities: &bundle.abilities,
        outcome_tables: &bundle.tables,
        placements: &bundle.placements,
        creatures: &bundle.creatures,
        effects: &bundle.effects,
    };
    start_encounter(world, &spec)?;
    Ok(world.combatants().iter().map(|(id, _)| id).collect())
}

/// The standard fixture made lethal: every roll succeeds and B (health 6)
/// dies to the first 6-damage hit.
fn lethal() -> Fixture {
    let mut fixture = Fixture::standard();
    fixture.set_alpha(100, 100);
    let id = fixture.ability;
    let ability = fixture.abilities.get_mut(&id).expect("ability");
    for entry in &mut ability.damage {
        entry.amount = 6;
    }
    fixture
}

/// The queued legacy events, read from a clone so the world is untouched.
fn queued(world: &World) -> Vec<(u64, SimEvent)> {
    world
        .clone()
        .events_mut()
        .drain()
        .into_iter()
        .map(|envelope| (envelope.tick.get(), envelope.payload))
        .collect()
}

/// The last queued legacy event, if any.
fn last_event(world: &World) -> Option<(u64, SimEvent)> {
    queued(world).pop()
}

#[test]
fn bound_spawn_and_despawn() {
    let mut world = World::new_in_area(3, area_a());
    assert_eq!(world.area(), Some(area_a()));
    let entity = world.spawn(EntityMeta {});
    assert_eq!(
        world.area_of(entity),
        Some(area_a()),
        "spawn acquires the area"
    );
    assert!(world.despawn(entity));
    assert_eq!(world.area_of(entity), None, "despawn loses it immediately");

    // Valid control: an unbound world never reports an area, even for a
    // live entity.
    let mut legacy = World::new(3);
    let live = legacy.spawn(EntityMeta {});
    assert!(legacy.contains(live));
    assert_eq!(legacy.area(), None);
    assert_eq!(legacy.area_of(live), None);

    // Membership comes from liveness in this world, not from the id alone.
    let mut other = World::new_in_area(3, area_b());
    let foreign = other.spawn(EntityMeta {});
    let stranger = World::new_in_area(3, area_a());
    assert_eq!(stranger.area_of(foreign), None);
    assert_eq!(other.area_of(foreign), Some(area_b()));
}

#[test]
fn death_and_release_retain_presence() {
    let fixture = lethal();
    let mut world = World::new_in_area(9, area_a());
    let ids = start_in(&fixture, &mut world).expect("the fixture area matches");
    let (a, b) = (ids[0], ids[1]);
    for id in [a, b] {
        assert_eq!(world.area_of(id), Some(area_a()));
    }
    // B acts first; A must survive, so B hits A, then A kills B.
    perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        },
    )
    .expect("hit");
    perform_action(
        &mut world,
        &CombatAction::UseAbility {
            actor: a,
            ability: fixture.ability,
            target: b,
        },
    )
    .expect("kill");
    assert!(world.combatants().get(b).expect("B").dead());
    assert_eq!(
        world.area_of(b),
        Some(area_a()),
        "death is not despawn: the dead combatant is still present"
    );
    let summary = end_encounter(&mut world).expect("release");
    assert_eq!(summary.results.len(), 2);
    for id in [a, b] {
        assert!(world.contains(id), "release keeps every entity");
        assert_eq!(world.area_of(id), Some(area_a()), "release keeps presence");
    }
    assert_eq!(world.area(), Some(area_a()), "the world stays bound");
}

#[test]
fn unbound_legacy_bytes_unchanged() {
    // An unbound world writes no area key at all.
    let mut legacy = World::new(5);
    let entity = legacy.spawn(EntityMeta {});
    legacy.transforms_mut().insert(entity, Transform::default());
    tick(&mut legacy);
    let value = serde_json::to_value(&legacy).expect("serialize");
    assert!(value.get("area").is_none(), "legacy worlds omit area");
    let history = serde_json::to_value(HistoryWorld::new(5)).expect("serialize");
    assert!(history["world"].get("area").is_none());

    // The same operations on a bound world differ only by the area field.
    let mut bound = World::new_in_area(5, area_a());
    let twin = bound.spawn(EntityMeta {});
    bound.transforms_mut().insert(twin, Transform::default());
    tick(&mut bound);
    let mut stripped = serde_json::to_value(&bound).expect("serialize");
    assert_eq!(
        stripped["area"],
        serde_json::Value::String(area_a().to_string()),
        "a present area persists as ULID text"
    );
    stripped
        .as_object_mut()
        .expect("object")
        .remove("area")
        .expect("present");
    assert_eq!(stripped, value);

    // Missing and explicit null both load as unbound and reserialize with
    // the field omitted; canonical legacy bytes round-trip exactly.
    let reloaded: World = serde_json::from_value(value.clone()).expect("loads");
    assert_eq!(reloaded.area(), None);
    assert_eq!(bytes(&reloaded), bytes(&legacy));
    let mut nulled = value;
    nulled["area"] = serde_json::Value::Null;
    let reloaded: World = serde_json::from_value(nulled).expect("null loads as unbound");
    assert_eq!(reloaded.area(), None);
    assert_eq!(bytes(&reloaded), bytes(&legacy));
    assert_eq!(state_hash(&reloaded), state_hash(&legacy));

    // Default and `new` stay the same unbound shape.
    assert_eq!(bytes(&World::default()), bytes(&World::new(0)));
}

#[test]
fn encounter_area_mismatch_precedence() {
    // Positive control: the bound area matches the fixture's resolved area.
    let fixture = Fixture::standard();
    let mut world = World::new_in_area(1, area_a());
    start_in(&fixture, &mut world).expect("matching area starts");

    // Mismatch rejects without mutation.
    let mut world = World::new_in_area(1, area_b());
    let before = bytes(&world);
    let hash = state_hash(&world);
    let error = start_in(&fixture, &mut world).expect_err("mismatch");
    assert_eq!(
        error,
        CombatError::AreaMismatch {
            expected: area_b(),
            found: area_a(),
        }
    );
    assert_eq!(error.to_string(), "AreaMismatch at spec/areas");
    assert_eq!(bytes(&world), before);
    assert_eq!(state_hash(&world), hash);

    // An unbound world keeps its legacy behavior for the same spec.
    let mut legacy = World::new(1);
    start_in(&fixture, &mut legacy).expect("unbound worlds do not check areas");

    // MissingPlacement and MixedArea precede the mismatch.
    let mut missing = Fixture::standard();
    let placement_b = missing.placement_b;
    missing.placements.remove(&placement_b);
    assert_eq!(
        start_in(&missing, &mut World::new_in_area(1, area_b())),
        Err(CombatError::MissingPlacement {
            placement: placement_b,
        })
    );
    let mut mixed = Fixture::standard();
    mixed.placements.get_mut(&placement_b).expect("B").1 = Ulid::from_u128(999);
    assert_eq!(
        start_in(&mixed, &mut World::new_in_area(1, area_b())),
        Err(CombatError::MixedArea {
            area_a: area_a(),
            area_b: Ulid::from_u128(999),
        })
    );
    // The mismatch precedes creature and stat checks.
    let mut creatureless = Fixture::standard();
    creatureless.creatures.clear();
    assert_eq!(
        start_in(&creatureless, &mut World::new_in_area(1, area_b())),
        Err(CombatError::AreaMismatch {
            expected: area_b(),
            found: area_a(),
        })
    );
    assert!(matches!(
        start_in(&creatureless, &mut World::new_in_area(1, area_a())),
        Err(CombatError::MissingCreature { .. })
    ));

    // The history wrapper reports it transactionally as a combat error.
    let mut history = HistoryWorld::new_in_area(1, area_b());
    let before = history_bytes(&history);
    let bundle = fixture.bundle();
    let spec = EncounterSpec {
        encounter: &fixture.encounter,
        ruleset: &fixture.ruleset,
        abilities: &bundle.abilities,
        outcome_tables: &bundle.tables,
        placements: &bundle.placements,
        creatures: &bundle.creatures,
        effects: &bundle.effects,
    };
    assert_eq!(
        history.start_encounter(&spec),
        Err(HistoryError::Combat(CombatError::AreaMismatch {
            expected: area_b(),
            found: area_a(),
        }))
    );
    assert_eq!(history_bytes(&history), before);
    let mut history = HistoryWorld::new_in_area(1, area_a());
    history
        .start_encounter(&spec)
        .expect("matching history start");
    assert_eq!(history.world().area(), Some(area_a()));
}

#[test]
fn transfer_copies_supported_state() {
    let mut source = World::new_in_area(11, area_a());
    let mut destination = World::new_in_area(22, area_b());
    // Distinct clocks: each side stamps its own tick.
    tick(&mut source);
    tick(&mut source);
    tick(&mut destination);
    let _ = source.events_mut().drain();
    let _ = destination.events_mut().drain();
    let occupant = destination.spawn(EntityMeta {});
    let moving = source.spawn(EntityMeta {});
    let bare = source.spawn(EntityMeta {});
    let transform = placed([1.5, -2.0, 3.25], [0.5, 0.0, -1.0]);
    source.transforms_mut().insert(moving, transform);
    let rng_before = (
        serde_json::to_value(&source).expect("serialize")["rng"].clone(),
        serde_json::to_value(&destination).expect("serialize")["rng"].clone(),
    );

    let moved = transfer_entity(&mut source, &mut destination, moving).expect("transfer");
    assert!(!source.contains(moving));
    assert_eq!(source.area_of(moving), None);
    assert!(source.transforms().get(moving).is_none());
    assert_eq!(destination.area_of(moved), Some(area_b()));
    assert_ne!(moved, occupant);
    assert_eq!(destination.transforms().get(moved), Some(&transform));
    assert_eq!(
        last_event(&source),
        Some((2, SimEvent::Despawned { entity: moving }))
    );
    assert_eq!(
        last_event(&destination),
        Some((1, SimEvent::Spawned { entity: moved }))
    );
    assert_eq!(source.tick().get(), 2, "ticks do not migrate");
    assert_eq!(destination.tick().get(), 1);
    let rng_after = (
        serde_json::to_value(&source).expect("serialize")["rng"].clone(),
        serde_json::to_value(&destination).expect("serialize")["rng"].clone(),
    );
    assert_eq!(rng_before, rng_after, "RNG does not migrate or change");

    // An absent transform stays absent: no synthetic zero transform.
    let moved_bare = transfer_entity(&mut source, &mut destination, bare).expect("transfer");
    assert!(destination.contains(moved_bare));
    assert!(destination.transforms().get(moved_bare).is_none());
    assert!(source.is_empty());
    assert_eq!(destination.len(), 3);
}

#[test]
fn transfer_rejects_combat_and_scheduled() {
    let fixture = Fixture::standard();
    let mut source = World::new_in_area(4, area_a());
    let ids = start_in(&fixture, &mut source).expect("start");
    let loose = source.spawn(EntityMeta {});
    let scheduled = source.spawn(EntityMeta {});
    source.timeline_mut().insert(InitiativeKey(50), scheduled);
    let gone = source.spawn(EntityMeta {});
    assert!(source.despawn(gone));
    let mut destination = World::new_in_area(4, area_b());
    let mut unbound = World::new(4);
    let mut same = World::new_in_area(4, area_a());

    let check = |source: &mut World, destination: &mut World, entity, expected: AreaError| {
        let before = pair(source, destination);
        assert_eq!(
            transfer_entity(source, destination, entity),
            Err(expected.clone())
        );
        assert_eq!(pair(source, destination), before, "{expected:?} mutated");
        assert_eq!(expected.to_string().split(' ').nth(1), Some("at"));
        assert!(expected.to_string().ends_with(" at area/transfer"));
    };
    // Pinned order: unbound source beats everything, then destination,
    // then same area, then liveness, combat, schedule, transform.
    let mut unbound_source = World::new(4);
    let orphan = unbound_source.spawn(EntityMeta {});
    check(
        &mut unbound_source,
        &mut unbound,
        orphan,
        AreaError::UnboundSource,
    );
    check(
        &mut source,
        &mut unbound,
        gone,
        AreaError::UnboundDestination,
    );
    check(
        &mut source,
        &mut same,
        gone,
        AreaError::SameArea { area: area_a() },
    );
    check(
        &mut source,
        &mut destination,
        gone,
        AreaError::AbsentEntity { entity: gone },
    );
    for id in &ids {
        check(
            &mut source,
            &mut destination,
            *id,
            AreaError::CombatParticipant { entity: *id },
        );
    }
    check(
        &mut source,
        &mut destination,
        scheduled,
        AreaError::ScheduledEntity { entity: scheduled },
    );
    assert_eq!(
        AreaError::UnboundSource.to_string(),
        "UnboundSource at area/transfer"
    );

    // A non-finite transform is refused (bytes only: hashing a non-finite
    // transform panics by design).
    source
        .transforms_mut()
        .insert(loose, placed([f32::NAN, 0.0, 0.0], [0.0; 3]));
    let before = (bytes(&source), bytes(&destination));
    assert_eq!(
        transfer_entity(&mut source, &mut destination, loose),
        Err(AreaError::InvalidTransform { entity: loose })
    );
    assert_eq!((bytes(&source), bytes(&destination)), before);
    source
        .transforms_mut()
        .insert(loose, placed([0.0; 3], [f32::INFINITY, 0.0, 0.0]));
    assert_eq!(
        transfer_entity(&mut source, &mut destination, loose),
        Err(AreaError::InvalidTransform { entity: loose })
    );
    // Positive control: a finite transform moves.
    source
        .transforms_mut()
        .insert(loose, placed([1.0, 2.0, 3.0], [0.0; 3]));
    transfer_entity(&mut source, &mut destination, loose).expect("finite moves");

    // Kill B, so even a dead, terminal participant is refused until release.
    let lethal = lethal();
    let mut arena = World::new_in_area(8, area_a());
    let ids = start_in(&lethal, &mut arena).expect("start");
    let (a, b) = (ids[0], ids[1]);
    perform_action(
        &mut arena,
        &CombatAction::UseAbility {
            actor: b,
            ability: lethal.ability,
            target: a,
        },
    )
    .expect("hit");
    perform_action(
        &mut arena,
        &CombatAction::UseAbility {
            actor: a,
            ability: lethal.ability,
            target: b,
        },
    )
    .expect("kill");
    assert_eq!(arena.combat().expect("terminal").active, None);
    check(
        &mut arena,
        &mut destination,
        b,
        AreaError::CombatParticipant { entity: b },
    );
    end_encounter(&mut arena).expect("release");
    let moved = transfer_entity(&mut arena, &mut destination, b).expect("released moves");
    assert_eq!(destination.area_of(moved), Some(area_b()));
    assert_eq!(arena.area_of(b), None);
}

#[test]
fn equal_ids_in_distinct_areas() {
    let mut west = World::new_in_area(1, area_a());
    let mut east = World::new_in_area(1, area_b());
    let here = west.spawn(EntityMeta {});
    let there = east.spawn(EntityMeta {});
    assert_eq!(here, there, "two worlds mint equal arena ids");
    assert_ne!(
        (west.area_of(here), here),
        (east.area_of(there), there),
        "identity is (area, EntityId)"
    );

    // A transfer into an empty slot can reissue the numerically equal id,
    // which still belongs to the destination area.
    let mut empty = World::new_in_area(1, Ulid::from_u128(133));
    let moved = transfer_entity(&mut west, &mut empty, here).expect("transfer");
    assert_eq!(moved, here);
    assert_eq!(west.area_of(here), None);
    assert_eq!(empty.area_of(moved), Some(Ulid::from_u128(133)));
    assert_eq!(east.area_of(there), Some(area_b()));
}

#[test]
fn slot_reuse_does_not_restore_membership() {
    let mut world = World::new_in_area(2, area_a());
    let first = world.spawn(EntityMeta {});
    assert!(world.despawn(first));
    let second = world.spawn(EntityMeta {});
    assert_eq!(first.index(), second.index(), "the slot is reused");
    assert_ne!(first, second);
    assert_eq!(world.area_of(first), None, "the old id stays absent");
    assert_eq!(world.area_of(second), Some(area_a()));

    // Transferred away, then the source slot is reused: the old id is
    // still absent in the source and was never present in the destination.
    let mut elsewhere = World::new_in_area(2, area_b());
    let _ = elsewhere.spawn(EntityMeta {});
    let moved = transfer_entity(&mut world, &mut elsewhere, second).expect("transfer");
    let reuse = world.spawn(EntityMeta {});
    assert_eq!(reuse.index(), second.index());
    assert_eq!(world.area_of(second), None);
    assert_eq!(world.area_of(reuse), Some(area_a()));
    assert_ne!(moved, second);
    assert_eq!(elsewhere.area_of(second), None);
}

#[test]
fn transfer_both_worlds_atomic() {
    let mut west = World::new_in_area(6, area_a());
    let mut east = World::new_in_area(7, area_b());
    let traveller = west.spawn(EntityMeta {});
    west.transforms_mut()
        .insert(traveller, placed([4.0, 5.0, 6.0], [1.0, 1.0, 1.0]));
    let blocker = west.spawn(EntityMeta {});
    west.timeline_mut().insert(InitiativeKey(1), blocker);

    // A rejected move leaves both worlds exactly as they were.
    let before = pair(&west, &east);
    assert_eq!(
        transfer_entity(&mut west, &mut east, blocker),
        Err(AreaError::ScheduledEntity { entity: blocker })
    );
    assert_eq!(pair(&west, &east), before);

    // A successful move changes both, together, and round-trips.
    let there = transfer_entity(&mut west, &mut east, traveller).expect("west to east");
    assert_ne!(pair(&west, &east).0, before.0);
    assert_ne!(pair(&west, &east).1, before.1);
    assert!(!west.contains(traveller) && east.contains(there));
    let back = transfer_entity(&mut east, &mut west, there).expect("east to west");
    assert!(!east.contains(there) && west.contains(back));
    assert_eq!(
        west.transforms().get(back),
        Some(&placed([4.0, 5.0, 6.0], [1.0, 1.0, 1.0]))
    );
    assert_eq!(west.area_of(back), Some(area_a()));
    // Each side saw exactly one removal and one arrival per move.
    let west_events: Vec<SimEvent> = queued(&west).into_iter().map(|(_, e)| e).collect();
    let east_events: Vec<SimEvent> = queued(&east).into_iter().map(|(_, e)| e).collect();
    assert_eq!(
        west_events,
        vec![
            SimEvent::Spawned { entity: traveller },
            SimEvent::Spawned { entity: blocker },
            SimEvent::Despawned { entity: traveller },
            SimEvent::Spawned { entity: back },
        ]
    );
    assert_eq!(
        east_events,
        vec![
            SimEvent::Spawned { entity: there },
            SimEvent::Despawned { entity: there },
        ]
    );
}

/// A bound history world holding one spawned, transformed entity whose
/// journal is optionally exhausted (acknowledged, then the sentinel).
fn history_with(seed: u64, area: Ulid, exhausted: bool) -> (HistoryWorld, EntityId, Transform) {
    let mut history = HistoryWorld::new_in_area(seed, area);
    let entity = history.spawn(EntityMeta {}).expect("spawn");
    let transform = placed([7.0, 8.0, 9.0], [0.0, -1.0, 0.0]);
    let mut value = serde_json::to_value(&history).expect("serialize");
    value["world"]["transforms"] =
        serde_json::to_value(vec![(entity, transform)]).expect("transform list");
    if exhausted {
        value["acknowledged"] = serde_json::Value::from(u64::MAX - 1);
        value["next_seq"] = serde_json::Value::from(u64::MAX);
        value["pending"] = serde_json::Value::Array(Vec::new());
    }
    let history: HistoryWorld = serde_json::from_value(value).expect("loads");
    (history, entity, transform)
}

#[test]
fn history_transfer_both_journals_atomic() {
    // Positive control: one Despawned in the source journal, one Spawned in
    // the destination journal, transform carried, no mutable-inner escape.
    let (mut source, entity, transform) = history_with(1, area_a(), false);
    let mut destination = HistoryWorld::new_in_area(2, area_b());
    destination.tick().expect("tick");
    let source_last = source.last_sequence();
    let moved = transfer_history_entity(&mut source, &mut destination, entity).expect("move");
    let source_page = source.read_after(source_last, 16).expect("read");
    assert_eq!(source_page.len(), 1);
    assert_eq!(source_page[0].payload, HistoryEvent::Despawned { entity });
    assert_eq!(source_page[0].tick.get(), 0);
    let destination_page = destination.read_after(0, 16).expect("read");
    assert_eq!(destination_page.len(), 1);
    assert_eq!(
        destination_page[0].payload,
        HistoryEvent::Spawned { entity: moved }
    );
    assert_eq!(
        destination_page[0].tick.get(),
        1,
        "each journal keeps its clock"
    );
    assert_eq!(
        destination.world().transforms().get(moved),
        Some(&transform)
    );
    assert_eq!(destination.world().area_of(moved), Some(area_b()));
    assert_eq!(source.world().area_of(entity), None);
    assert!(source.world().events().is_empty(), "no inner queue residue");
    assert!(destination.world().events().is_empty());

    let reject = |source: &mut HistoryWorld,
                  destination: &mut HistoryWorld,
                  entity: EntityId,
                  expected: AreaError| {
        let before = (
            history_bytes(source),
            history_bytes(destination),
            history_hash(source),
            history_hash(destination),
        );
        let error = transfer_history_entity(source, destination, entity).expect_err("rejects");
        assert_eq!(error, expected);
        let after = (
            history_bytes(source),
            history_bytes(destination),
            history_hash(source),
            history_hash(destination),
        );
        assert_eq!(after, before, "{expected:?} mutated a wrapper");
    };

    // Source-only capacity failure: the source journal cannot accept the
    // removal; neither wrapper (nor the source RNG/event counters) moves.
    let (mut full_source, entity, _) = history_with(3, area_a(), true);
    let mut roomy = HistoryWorld::new_in_area(4, area_b());
    reject(
        &mut full_source,
        &mut roomy,
        entity,
        AreaError::SourceHistory(HistoryError::SequenceExhausted),
    );
    // Destination-only: the staged source removal is discarded too.
    let (mut roomy_source, entity, _) = history_with(5, area_a(), false);
    let (mut full_destination, _, _) = history_with(6, area_b(), true);
    reject(
        &mut roomy_source,
        &mut full_destination,
        entity,
        AreaError::DestinationHistory(HistoryError::SequenceExhausted),
    );
    assert!(roomy_source.world().contains(entity));
    // Both full: the source is reported first.
    let (mut full_source, entity, _) = history_with(7, area_a(), true);
    let (mut full_destination, _, _) = history_with(8, area_b(), true);
    reject(
        &mut full_source,
        &mut full_destination,
        entity,
        AreaError::SourceHistory(HistoryError::SequenceExhausted),
    );
    let error = AreaError::DestinationHistory(HistoryError::SequenceExhausted);
    assert_eq!(error.to_string(), "DestinationHistory at area/transfer");
    assert_eq!(
        error.source().map(ToString::to_string),
        Some(HistoryError::SequenceExhausted.to_string())
    );
    assert!(AreaError::SameArea { area: area_a() }.source().is_none());

    // Preflight failures also publish nothing.
    let (mut source, entity, _) = history_with(9, area_a(), false);
    let mut same = HistoryWorld::new_in_area(10, area_a());
    reject(
        &mut source,
        &mut same,
        entity,
        AreaError::SameArea { area: area_a() },
    );
    let mut legacy = HistoryWorld::new(10);
    reject(
        &mut source,
        &mut legacy,
        entity,
        AreaError::UnboundDestination,
    );
}

#[test]
fn area_save_hash_continuation() {
    // Sensitivity: identical worlds except for area differ in bytes and in
    // both hashes.
    let mut left = World::new_in_area(12, area_a());
    let mut right = World::new_in_area(12, area_b());
    left.spawn(EntityMeta {});
    right.spawn(EntityMeta {});
    assert_ne!(bytes(&left), bytes(&right));
    assert_ne!(state_hash(&left), state_hash(&right));
    assert_ne!(
        history_hash(&HistoryWorld::new_in_area(12, area_a())),
        history_hash(&HistoryWorld::new_in_area(12, area_b()))
    );
    assert_ne!(
        history_hash(&HistoryWorld::new_in_area(12, area_a())),
        history_hash(&HistoryWorld::new(12))
    );

    // An invalid ULID fails loading.
    let mut corrupt = serde_json::to_value(&left).expect("serialize");
    corrupt["area"] = serde_json::Value::String("not-a-ulid".to_owned());
    assert!(serde_json::from_value::<World>(corrupt).is_err());

    // Save/load mid-run, then continue: identical to uninterrupted runs,
    // through spawn, transfer, and a combat action.
    let fixture = Fixture::standard();
    let run = |checkpoint: bool| {
        let mut west = World::new_in_area(40, area_a());
        let mut east = World::new_in_area(41, area_b());
        let ids = start_in(&fixture, &mut west).expect("start");
        let wanderer = west.spawn(EntityMeta {});
        west.transforms_mut()
            .insert(wanderer, placed([1.0, 0.0, 0.0], [0.0; 3]));
        if checkpoint {
            west = serde_json::from_slice(&bytes(&west)).expect("west reloads");
            east = serde_json::from_slice(&bytes(&east)).expect("east reloads");
            assert_eq!(west.area(), Some(area_a()));
        }
        let moved = transfer_entity(&mut west, &mut east, wanderer).expect("transfer");
        let extra = west.spawn(EntityMeta {});
        perform_action(
            &mut west,
            &CombatAction::UseAbility {
                actor: ids[1],
                ability: fixture.ability,
                target: ids[0],
            },
        )
        .expect("action");
        (bytes(&west), bytes(&east), moved, extra)
    };
    assert_eq!(run(true), run(false));

    // The same for history wrappers.
    let history_run = |checkpoint: bool| {
        let (mut west, entity, _) = history_with(50, area_a(), false);
        let mut east = HistoryWorld::new_in_area(51, area_b());
        if checkpoint {
            west = serde_json::from_slice(&history_bytes(&west)).expect("reloads");
            east = serde_json::from_slice(&history_bytes(&east)).expect("reloads");
        }
        let moved = transfer_history_entity(&mut west, &mut east, entity).expect("move");
        east.spawn(EntityMeta {}).expect("spawn");
        (history_bytes(&west), history_bytes(&east), moved)
    };
    assert_eq!(history_run(true), history_run(false));
}
