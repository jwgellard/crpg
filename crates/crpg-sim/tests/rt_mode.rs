//! Encounter mode, clock publication and mode requests (T138a, ADR-0029
//! T1-T2, T9, T11, T13): R1, R2, R3 and R4.
//!
//! Every expectation is derived from the contract by hand: keys, clocks and
//! pending requests are read back through the public surface, and whole-world
//! byte comparisons prove "nothing else changed".

#[allow(dead_code)]
mod support;

use crpg_core::Ulid;
use crpg_sim::{
    end_encounter, perform_action, request_mode, start_encounter, start_encounter_in_mode, tick,
    CombatAction, CombatError, EncounterClock, EncounterMode, EncounterSpec, World, MAX_COMBATANTS,
};
use support::rt::{
    bytes_of, capable, edit_ability, json_keys, make_lethal, pool_of, ticks, timeline_of,
    use_action, value_of, RT_KEYS,
};
use support::Fixture;

fn spec_of<'a>(fixture: &'a Fixture, bundle: &'a support::SpecBundle<'a>) -> EncounterSpec<'a> {
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

/// R1 (B1, B28): a TB start and an RT start publish the documented shapes,
/// and `start_encounter` is turn-based even on an RT-capable ruleset.
#[test]
fn r1_start_modes_publish_their_shapes() {
    let fixture = capable(4);

    // start_encounter on an RT-capable ruleset is turn-based.
    let (world, ids) = fixture.start(1);
    let combat = world.combat().expect("encounter");
    assert_eq!(combat.mode, EncounterMode::TurnBased);
    assert_eq!(
        combat.clock,
        Some(EncounterClock {
            round_ticks: 4,
            elapsed: 0
        })
    );
    assert_eq!(combat.pending_mode, None);
    assert_eq!(combat.active, Some(ids[1]), "B (initiative 9) acts first");
    assert_eq!(timeline_of(&world), vec![(9, ids[1]), (10, ids[0])]);

    // The explicit TB mode is the same publication, byte for byte.
    let (explicit, _) = fixture
        .start_mode(1, EncounterMode::TurnBased)
        .expect("TB start");
    assert_eq!(bytes_of(&world), bytes_of(&explicit));

    // An RT start: every key 0, no active turn, a clock at 0.
    let (rt, ids) = fixture
        .start_mode(1, EncounterMode::RealTime)
        .expect("RT start");
    let combat = rt.combat().expect("encounter");
    assert_eq!(combat.mode, EncounterMode::RealTime);
    assert_eq!(combat.active, None);
    assert_eq!(combat.round, 0);
    assert_eq!(combat.clock.map(|clock| clock.elapsed), Some(0));
    assert_eq!(timeline_of(&rt), vec![(0, ids[0]), (0, ids[1])]);
    for id in &ids {
        assert!(rt
            .combatants()
            .get(*id)
            .expect("combatant")
            .order()
            .is_none());
        assert_eq!(pool_of(&rt, *id), 1, "pools start at max");
    }
    // Only the participants' `Spawned` events were queued.
    let queued = rt.clone().events_mut().drain();
    assert_eq!(queued.len(), 2);
}

/// R1 (B1): release drops the mode, clock and pending request with the
/// state, and the released world carries none of the new keys.
#[test]
fn r1_release_drops_mode_clock_and_pending() {
    let fixture = capable(4);
    let (mut world, _) = fixture.start_rt(2, EncounterMode::RealTime);
    request_mode(&mut world, EncounterMode::TurnBased).expect("request");
    assert!(world.combat().expect("encounter").pending_mode.is_some());
    end_encounter(&mut world).expect("release");
    assert!(world.combat().is_none());
    let keys = json_keys(&value_of(&world));
    for name in RT_KEYS {
        assert!(!keys.contains(name), "{name} survived release");
    }
    // A second encounter starts from a clean slate.
    let bundle = fixture.bundle();
    let spec = spec_of(&fixture, &bundle);
    start_encounter(&mut world, &spec).expect("restart");
    let combat = world.combat().expect("encounter");
    assert_eq!(combat.mode, EncounterMode::TurnBased);
    assert_eq!(combat.pending_mode, None);
    assert_eq!(combat.clock.map(|clock| clock.elapsed), Some(0));
}

/// R1 (B2): a terminal RT encounter keeps its mode and pending request until
/// it is released; becoming terminal rewrites neither.
#[test]
fn r1_terminal_rt_keeps_mode_and_pending() {
    let mut fixture = capable(4);
    make_lethal(&mut fixture, 100);
    let (mut world, ids) = fixture.start_rt(3, EncounterMode::RealTime);
    request_mode(&mut world, EncounterMode::TurnBased).expect("request");
    perform_action(&mut world, &use_action(ids[0], fixture.ability, ids[1])).expect("order");
    tick(&mut world);
    assert!(world.combatants().get(ids[1]).expect("B").dead(), "B died");
    // The pass ended the fight; run across a boundary and beyond.
    ticks(&mut world, 10);
    let combat = world.combat().expect("still held until released");
    assert_eq!(combat.mode, EncounterMode::RealTime);
    assert_eq!(combat.pending_mode, Some(EncounterMode::TurnBased));
    assert_eq!(combat.clock.map(|clock| clock.elapsed), Some(1));
    end_encounter(&mut world).expect("release");
    assert!(world.combat().is_none());
}

/// R1 (B2): a terminal TB encounter keeps its mode and pending request too.
#[test]
fn r1_terminal_tb_keeps_mode_and_pending() {
    let mut fixture = capable(4);
    make_lethal(&mut fixture, 100);
    let (mut world, ids) = fixture.start(4);
    request_mode(&mut world, EncounterMode::RealTime).expect("request");
    // B acts first and kills A.
    perform_action(&mut world, &use_action(ids[1], fixture.ability, ids[0])).expect("kill");
    let combat = world.combat().expect("encounter");
    assert_eq!(combat.active, None, "terminal");
    assert_eq!(combat.mode, EncounterMode::TurnBased);
    assert_eq!(combat.pending_mode, Some(EncounterMode::RealTime));
    ticks(&mut world, 20);
    let combat = world.combat().expect("encounter");
    assert_eq!(combat.mode, EncounterMode::TurnBased);
    assert_eq!(combat.pending_mode, Some(EncounterMode::RealTime));
}

/// R1 (B30, B4): a second start fails `EncounterActive` before
/// `ModeUnsupported`, whatever the mode or the ruleset.
#[test]
fn r1_encounter_active_beats_mode_unsupported() {
    // A TB-only ruleset: RT is unsupported, but a live encounter wins.
    let fixture = Fixture::standard();
    let (mut world, _) = fixture.start(5);
    let bundle = fixture.bundle();
    let spec = spec_of(&fixture, &bundle);
    let before = bytes_of(&world);
    assert_eq!(
        start_encounter_in_mode(&mut world, &spec, EncounterMode::RealTime),
        Err(CombatError::EncounterActive)
    );
    assert_eq!(before, bytes_of(&world));

    // An RT-capable world: both modes fail the same way.
    let capable_fixture = capable(4);
    let (mut world, _) = capable_fixture.start_rt(5, EncounterMode::RealTime);
    let bundle = capable_fixture.bundle();
    let spec = spec_of(&capable_fixture, &bundle);
    for mode in [EncounterMode::TurnBased, EncounterMode::RealTime] {
        assert_eq!(
            start_encounter_in_mode(&mut world, &spec, mode),
            Err(CombatError::EncounterActive)
        );
    }
}

/// R2 (B3): the clock exists exactly when the ruleset declares
/// `round_ticks`; RT-capable abilities copy the authored fields.
#[test]
fn r2_clock_and_ability_fields_are_published() {
    // TB-only: no clock, whatever the abilities author.
    let mut fixture = Fixture::standard();
    edit_ability(&mut fixture, |ability| {
        ability.recovery_ticks = Some(3);
        ability.repeats_in_real_time = Some(true);
    });
    let (world, _) = fixture.start(1);
    let combat = world.combat().expect("encounter");
    assert_eq!(combat.clock, None);
    let entry = &combat.definition.abilities[0];
    assert_eq!(entry.recovery_ticks, None, "absent in a TB-only encounter");
    assert!(!entry.repeats_in_real_time, "absent in a TB-only encounter");

    // RT-capable: the authored values are copied (flag via unwrap_or(false)).
    let mut fixture = capable(4);
    edit_ability(&mut fixture, |ability| {
        ability.recovery_ticks = Some(3);
        ability.repeats_in_real_time = Some(true);
    });
    let (world, _) = fixture.start(1);
    let entry = &world.combat().expect("encounter").definition.abilities[0];
    assert_eq!(entry.recovery_ticks, Some(3));
    assert!(entry.repeats_in_real_time);

    for (authored, copied) in [(Some(false), false), (None, false)] {
        let mut fixture = capable(4);
        edit_ability(&mut fixture, |ability| {
            ability.repeats_in_real_time = authored;
        });
        let (world, _) = fixture.start(1);
        let entry = &world.combat().expect("encounter").definition.abilities[0];
        assert_eq!(entry.recovery_ticks, None);
        assert_eq!(entry.repeats_in_real_time, copied, "{authored:?}");
    }
}

/// R2 (B5): zero `round_ticks` is a data-owned precondition.
#[test]
#[should_panic(expected = "ruleset round_ticks is validated nonzero by data read-time checks")]
fn r2_zero_round_ticks_panics() {
    let mut fixture = Fixture::standard();
    fixture.ruleset.round_ticks = Some(0);
    let _ = fixture.start(1);
}

/// R2 (B5): zero `recovery_ticks` in an RT-capable encounter is too.
#[test]
#[should_panic(expected = "ability recovery_ticks is validated nonzero by data read-time checks")]
fn r2_zero_recovery_ticks_panics() {
    let mut fixture = capable(4);
    edit_ability(&mut fixture, |ability| ability.recovery_ticks = Some(0));
    let _ = fixture.start(1);
}

/// R2 (B5): a TB-only encounter never reads `recovery_ticks`.
#[test]
fn r2_zero_recovery_in_tb_only_is_ignored() {
    let mut fixture = Fixture::standard();
    edit_ability(&mut fixture, |ability| ability.recovery_ticks = Some(0));
    let (world, _) = fixture.start(1);
    assert!(world.combat().is_some());
}

/// R2 (B7, D13): an RT-capable single-ability encounter that declares an RT
/// field serializes the vectors and survives save then load; without one the
/// legacy shape is unchanged.
#[test]
fn r2_legacy_shape_save_load() {
    let definition_keys = |world: &World| -> Vec<String> {
        let value = value_of(world);
        value["combat"]["definition"]
            .as_object()
            .expect("definition object")
            .keys()
            .cloned()
            .collect()
    };

    // Neither field: legacy shape, fifteen keys.
    let plain = capable(4);
    let (world, _) = plain.start(1);
    let keys = definition_keys(&world);
    assert_eq!(keys.len(), 15);
    assert!(!keys.iter().any(|key| key == "abilities"));
    let saved = serde_json::to_string(&world).expect("serialize");
    let loaded: World = serde_json::from_str(&saved).expect("legacy shape loads");
    assert_eq!(loaded, world);

    // Each field alone forces the vectors.
    for (recovery, repeats) in [(Some(3), None), (None, Some(true)), (Some(2), Some(true))] {
        let mut fixture = capable(4);
        edit_ability(&mut fixture, |ability| {
            ability.recovery_ticks = recovery;
            ability.repeats_in_real_time = repeats;
        });
        let (world, _) = fixture.start(1);
        let keys = definition_keys(&world);
        assert_eq!(keys.len(), 18, "{recovery:?} {repeats:?}");
        for name in ["abilities", "pools", "effects"] {
            assert!(keys.iter().any(|key| key == name), "{name}");
        }
        let saved = serde_json::to_string(&world).expect("serialize");
        let loaded: World = serde_json::from_str(&saved).expect("vector shape loads");
        assert_eq!(loaded, world, "save then load keeps the RT fields");
        let entry = &loaded.combat().expect("encounter").definition.abilities[0];
        assert_eq!(entry.recovery_ticks, recovery);
        assert_eq!(entry.repeats_in_real_time, repeats.unwrap_or(false));
    }

    // `Some(false)` is the same as absent (D31i): the legacy shape.
    let mut fixture = capable(4);
    edit_ability(&mut fixture, |ability| {
        ability.repeats_in_real_time = Some(false);
    });
    let (world, _) = fixture.start(1);
    assert_eq!(definition_keys(&world).len(), 15);
}

/// R3 (B4): RT on a TB-only ruleset fails `ModeUnsupported`, checked after
/// `EncounterActive` and before the participant bound, with nothing changed.
#[test]
fn r3_rt_start_on_tb_only_is_rejected_cleanly() {
    let fixture = Fixture::standard();
    let bundle = fixture.bundle();
    let spec = spec_of(&fixture, &bundle);
    let mut world = World::new(9);
    let before = bytes_of(&world);
    assert_eq!(
        start_encounter_in_mode(&mut world, &spec, EncounterMode::RealTime),
        Err(CombatError::ModeUnsupported)
    );
    assert_eq!(before, bytes_of(&world), "no entity, event, stream or tag");
    assert_eq!(
        CombatError::ModeUnsupported.to_string(),
        "ModeUnsupported at combat/mode"
    );
    // The same world still starts a TB encounter afterwards.
    start_encounter_in_mode(&mut world, &spec, EncounterMode::TurnBased).expect("TB starts");

    // ModeUnsupported precedes TooManyParticipants; TB meets the bound.
    let mut crowded = Fixture::standard();
    let template = crowded.encounter.participants[0];
    while crowded.encounter.participants.len() <= MAX_COMBATANTS {
        crowded.encounter.participants.push(template);
    }
    let bundle = crowded.bundle();
    let spec = spec_of(&crowded, &bundle);
    let mut world = World::new(9);
    assert_eq!(
        start_encounter_in_mode(&mut world, &spec, EncounterMode::RealTime),
        Err(CombatError::ModeUnsupported)
    );
    assert_eq!(
        start_encounter_in_mode(&mut world, &spec, EncounterMode::TurnBased),
        Err(CombatError::TooManyParticipants)
    );
    // An RT-capable ruleset reaches the bound in RT as well.
    let mut crowded = capable(4);
    let template = crowded.encounter.participants[0];
    while crowded.encounter.participants.len() <= MAX_COMBATANTS {
        crowded.encounter.participants.push(template);
    }
    let bundle = crowded.bundle();
    let spec = spec_of(&crowded, &bundle);
    assert_eq!(
        start_encounter_in_mode(&mut world, &spec, EncounterMode::RealTime),
        Err(CombatError::TooManyParticipants)
    );
}

fn pending_of(world: &World) -> Option<EncounterMode> {
    world.combat().expect("encounter").pending_mode
}

/// The world as JSON without the pending request.
fn without_pending(world: &World) -> serde_json::Value {
    let mut value = value_of(world);
    value["combat"]
        .as_object_mut()
        .expect("combat")
        .remove("pending_mode");
    value
}

/// R3 (B23): the request matrix.
#[test]
fn r3_request_matrix() {
    // NoEncounter, for both modes, with nothing changed.
    let mut empty = World::new(1);
    let before = bytes_of(&empty);
    for mode in [EncounterMode::TurnBased, EncounterMode::RealTime] {
        assert_eq!(
            request_mode(&mut empty, mode),
            Err(CombatError::NoEncounter)
        );
    }
    assert_eq!(before, bytes_of(&empty));

    // TB-only: RT is unsupported; TB (the current mode) is accepted and clear.
    let (mut world, _) = Fixture::standard().start(1);
    let before = bytes_of(&world);
    assert_eq!(
        request_mode(&mut world, EncounterMode::RealTime),
        Err(CombatError::ModeUnsupported)
    );
    assert_eq!(before, bytes_of(&world));
    assert_eq!(request_mode(&mut world, EncounterMode::TurnBased), Ok(()));
    assert_eq!(
        before,
        bytes_of(&world),
        "requesting the current mode is a no-op"
    );

    // RT-capable, in TB.
    let fixture = capable(4);
    let (mut world, _) = fixture.start(1);
    let base = without_pending(&world);
    request_mode(&mut world, EncounterMode::RealTime).expect("other");
    assert_eq!(pending_of(&world), Some(EncounterMode::RealTime));
    assert_eq!(
        world.combat().expect("encounter").mode,
        EncounterMode::TurnBased
    );
    request_mode(&mut world, EncounterMode::RealTime).expect("repeat");
    assert_eq!(pending_of(&world), Some(EncounterMode::RealTime), "repeat");
    request_mode(&mut world, EncounterMode::TurnBased).expect("current clears");
    assert_eq!(pending_of(&world), None);
    request_mode(&mut world, EncounterMode::TurnBased).expect("current again");
    assert_eq!(pending_of(&world), None);
    assert_eq!(base, without_pending(&world), "nothing else changed");

    // RT-capable, in RT.
    let (mut world, _) = fixture.start_rt(1, EncounterMode::RealTime);
    let base = without_pending(&world);
    request_mode(&mut world, EncounterMode::TurnBased).expect("other");
    assert_eq!(pending_of(&world), Some(EncounterMode::TurnBased));
    request_mode(&mut world, EncounterMode::TurnBased).expect("repeat");
    assert_eq!(pending_of(&world), Some(EncounterMode::TurnBased));
    request_mode(&mut world, EncounterMode::RealTime).expect("current clears");
    assert_eq!(pending_of(&world), None);
    assert_eq!(base, without_pending(&world));
    assert_eq!(world.events().len(), 2, "no event beyond the two spawns");
}

/// R3 (B24): a request never applies mid-round.
#[test]
fn r3_request_never_applies_mid_round() {
    // RT to TB: only at the boundary (tick 4 of round_ticks 4).
    let fixture = capable(4);
    let (mut world, _) = fixture.start_rt(1, EncounterMode::RealTime);
    request_mode(&mut world, EncounterMode::TurnBased).expect("request");
    for expected in 1..=3_u64 {
        tick(&mut world);
        let combat = world.combat().expect("encounter");
        assert_eq!(combat.mode, EncounterMode::RealTime, "tick {expected}");
        assert_eq!(combat.pending_mode, Some(EncounterMode::TurnBased));
    }
    tick(&mut world);
    let combat = world.combat().expect("encounter");
    assert_eq!(
        combat.mode,
        EncounterMode::TurnBased,
        "applied at the boundary"
    );
    assert_eq!(combat.pending_mode, None);

    // TB to RT: only at the rollover, not after B's turn alone.
    let (mut world, ids) = fixture.start(1);
    request_mode(&mut world, EncounterMode::RealTime).expect("request");
    perform_action(&mut world, &CombatAction::EndTurn { actor: ids[1] }).expect("B ends");
    let combat = world.combat().expect("encounter");
    assert_eq!(combat.mode, EncounterMode::TurnBased, "mid-round");
    assert_eq!(combat.pending_mode, Some(EncounterMode::RealTime));
    ticks(&mut world, 50);
    assert_eq!(
        world.combat().expect("encounter").mode,
        EncounterMode::TurnBased,
        "ticks never apply a TB to RT request"
    );
    perform_action(&mut world, &CombatAction::EndTurn { actor: ids[0] }).expect("A ends");
    let combat = world.combat().expect("encounter");
    assert_eq!(
        combat.mode,
        EncounterMode::RealTime,
        "applied at the rollover"
    );
    assert_eq!(combat.pending_mode, None);
}

/// R4 (B6): a TB-only world never writes any of the six new keys.
#[test]
fn r4_tb_only_worlds_write_no_new_keys() {
    for fixture in [Fixture::standard(), Fixture::trio()] {
        let (mut world, _) = fixture.start(11);
        let check = |world: &World, when: &str| {
            let keys = json_keys(&value_of(world));
            for name in RT_KEYS {
                assert!(!keys.contains(name), "{name} appeared {when}");
            }
        };
        check(&world, "before play");
        let ability: Ulid = fixture.ability;
        for step in 0..20 {
            if let Some(actor) = world.combat().and_then(|combat| combat.active) {
                let target = world
                    .combatants()
                    .iter()
                    .map(|(id, _)| id)
                    .find(|id| *id != actor && !world.combatants().get(*id).expect("c").dead())
                    .expect("a foe");
                let action = if step % 3 == 0 {
                    CombatAction::EndTurn { actor }
                } else {
                    use_action(actor, ability, target)
                };
                perform_action(&mut world, &action).expect("legal step");
            }
            tick(&mut world);
        }
        check(&world, "after 20 actions and ticks");
    }
}
