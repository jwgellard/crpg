//! Success path for HistoryWorld::end_encounter (audit lead #68).

#[allow(dead_code)]
mod support;

use crpg_core::EntityId;
use crpg_sim::{end_encounter, start_encounter, state_hash, EncounterSpec, HistoryWorld, World};
use support::Fixture;

fn spec_for<'a>(fixture: &'a Fixture, bundle: &'a support::SpecBundle<'a>) -> EncounterSpec<'a> {
    EncounterSpec {
        encounter: &fixture.encounter,
        ruleset: &fixture.ruleset,
        abilities: &bundle.abilities,
        outcome_tables: &bundle.tables,
        placements: &bundle.placements,
        creatures: &bundle.creatures,
        effects: &bundle.effects,
    }
}

#[test]
fn history_end_encounter_success_matches_plain_world() {
    let fixture = Fixture::standard();
    let bundle = fixture.bundle();
    let spec = spec_for(&fixture, &bundle);

    // History wrapper path.
    let mut history = HistoryWorld::new(7);
    history.start_encounter(&spec).expect("wrapper starts");
    let ids: Vec<EntityId> = history
        .world()
        .combatants()
        .iter()
        .map(|(id, _)| id)
        .collect();
    let pending_before = history.pending_len();
    let summary_h = history.end_encounter().expect("wrapper releases");
    let pending_after = history.pending_len();

    // Plain world path, identical seed and spec.
    let mut world = World::new(7);
    start_encounter(&mut world, &spec).expect("legacy starts");
    let summary_w = end_encounter(&mut world).expect("legacy releases");

    // The encounter is gone on both sides.
    assert!(history.world().combat().is_none());
    assert!(world.combat().is_none());

    // Explicit release journals nothing: the journal did not grow.
    assert_eq!(pending_after, pending_before);
    // The start journaled exactly its three envelopes (2 spawns + turn start).
    assert_eq!(pending_before, 3);

    // Every entity stays live on both sides.
    for id in &ids {
        assert!(history.world().contains(*id));
    }
    for id in world.ids() {
        assert!(world.contains(id));
    }

    // The retained summary matches the plain-world result.
    assert_eq!(format!("{:?}", summary_h), format!("{:?}", summary_w));

    // Hashes of the underlying worlds agree once the legacy queue is drained.
    let mut drained = world.clone();
    let _ = drained.events_mut().drain();
    assert_eq!(state_hash(&drained), state_hash(history.world()));
}
