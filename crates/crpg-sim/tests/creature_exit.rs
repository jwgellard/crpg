//! T139b2: leaving a live encounter and the entry/exit lifecycle around it
//! (Behaviour B16, B29-B32 and B34; tests E12-E16). Joining is in
//! `creature_join.rs`.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Ulid};
use crpg_data::{CreaturePersistenceWire, EffectsOnExitWire, LateJoinWire};
use crpg_sim::{
    end_encounter, join_encounter, leave_encounter, perform_action, start_encounter_with_members,
    state_hash, tick, CombatAction, CombatError, Combatant, EncounterMode, EntityMeta, Entrant,
    SimEvent, SuspendedEffect, Transform, World,
};
use serde_json::{json, Value};
use support::creature::{add_outsider, edit_combatant, set_hooks, with_json, with_record};
use support::draws::set_damage;
use support::rt::{
    capable, capable_trio, elapsed, make_lethal, pool_of, ticks, timeline_of, use_action,
};
use support::surface::{always_success, attach_to_ability, effect_doc, end, modifier};
use support::{uid, Fixture};

const O: u128 = 131;
const J: u128 = 133;
const K: u128 = 134;

fn id(value: u128) -> Ulid {
    Ulid::from_u128(value)
}

fn bound(fixture: &Fixture) -> World {
    World::new_in_area(5, fixture.area)
}

/// Everything a failed operation must leave alone.
fn snapshot(world: &World) -> (Vec<u8>, [u8; 32], crpg_core::DeterministicRng) {
    let mut copy = world.clone();
    (
        serde_json::to_vec(world).expect("the world serializes"),
        state_hash(world),
        copy.rng_mut().clone(),
    )
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

fn entrant(entity: EntityId, initiative: i32) -> Entrant {
    Entrant { entity, initiative }
}

fn register(fixture: &Fixture, world: &mut World, placement: u128) -> EntityId {
    fixture
        .register(world, id(placement))
        .expect("the outsider registers")
}

fn begin_in(fixture: &Fixture, world: &mut World, mode: EncounterMode, members: &[Entrant]) {
    fixture
        .with_spec(|spec| start_encounter_with_members(world, spec, mode, members))
        .expect("the encounter starts");
}

fn begin(fixture: &Fixture, world: &mut World, members: &[Entrant]) {
    begin_in(fixture, world, EncounterMode::TurnBased, members);
}

fn effect_501(duration: u32) -> crpg_data::Effect {
    effect_doc(
        501,
        "target",
        "probe-type",
        "stack_all",
        vec![modifier(601, json!({"type": "roll"}), "add", 1)],
        duration,
    )
}

fn leave_fails(world: &mut World, entity: EntityId, expected: CombatError) {
    let before = snapshot(world);
    assert_eq!(leave_encounter(world, entity), Err(expected.clone()));
    assert_eq!(
        snapshot(world),
        before,
        "{expected:?} mutated the world, its RNG or its hash"
    );
}

/// Standard fixture with outsiders O, J and K and the effect of the exit
/// cases attached to the standard ability.
fn exit_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    attach_to_ability(&mut fixture, effect_501(3));
    add_outsider(&mut fixture, O, 141);
    add_outsider(&mut fixture, J, 143);
    add_outsider(&mut fixture, K, 144);
    fixture
}

/// The record a combatant becomes, written out from its getters (B16).
fn expected_record(combatant: &Combatant, ruleset: Ulid, suspended: &[SuspendedEffect]) -> Value {
    let mut pools = vec![combatant.action_pool().clone()];
    pools.extend(combatant.extra_pools().iter().cloned());
    let mut record = json!({
        "placement": combatant.placement(),
        "ruleset": ruleset,
        "attributes": combatant.attributes(),
        "health": combatant.health(),
        "max_health": combatant.max_health(),
        "pools": pools,
    });
    if !combatant.fixed().is_empty() {
        record["fixed"] = serde_json::to_value(combatant.fixed()).expect("fixed");
    }
    if !suspended.is_empty() {
        record["suspended"] = serde_json::to_value(suspended).expect("suspended");
    }
    if !combatant.tags().is_empty() {
        record["tags"] = json!(combatant.tags());
    }
    record
}

fn record_json(world: &World, entity: EntityId) -> Value {
    serde_json::to_value(world.creatures().get(entity).expect("a record")).expect("serializes")
}

/// E12 (B16, B30): under the default, spawned A leaves and becomes a record.
/// Under `respawns = Some(true)`, spawned A leaves as a bare live entity with
/// its transform, while member O leaves and becomes a record. The entity
/// stays live in every case, keeps its `Transform`, and the leave enqueues
/// neither `Died` nor `Despawned`.
///
/// **Fails if** a leave despawns, drops the transform, writes back a spawned
/// combatant under `respawns`, or skips an entered one.
#[test]
fn e12_leave_write_back() {
    // The default: a spawned combatant leaves and becomes a record.
    let fixture = exit_fixture();
    let mut world = bound(&fixture);
    begin(&fixture, &mut world, &[]);
    let ids = combatant_ids(&world);
    let (a, b) = (ids[0], ids[1]);
    let spot = Transform {
        position: [1.0, 2.0, 3.0],
        velocity: [0.0, 0.0, 0.0],
    };
    world.transforms_mut().insert(a, spot);
    events_of(&mut world);
    let left = world.combatants().get(a).expect("A").clone();
    leave_encounter(&mut world, a).expect("A leaves");
    assert!(world.contains(a), "the entity stays live");
    assert!(world.combatants().get(a).is_none());
    assert!(!world.timeline().contains(a));
    assert_eq!(
        world.transforms().get(a),
        Some(&spot),
        "the transform stays"
    );
    assert_eq!(
        record_json(&world, a),
        expected_record(&left, id(102), &[]),
        "the record holds the combatant's values"
    );
    assert_eq!(world.creatures().get(a).expect("record").health(), 10);
    assert_eq!(world.combat().expect("combat").active, Some(b));
    assert!(events_of(&mut world).is_empty(), "no Died, no Despawned");

    // Under respawns: the spawned combatant leaves bare; the member leaves
    // as a record.
    let mut fixture = exit_fixture();
    fixture.encounter.respawns = Some(true);
    let mut world = bound(&fixture);
    let o = register(&fixture, &mut world, O);
    begin(&fixture, &mut world, &[entrant(o, 5)]);
    let ids = combatant_ids(&world);
    let (a, b) = (ids[0], ids[1]);
    world.transforms_mut().insert(a, spot);
    events_of(&mut world);
    leave_encounter(&mut world, a).expect("A leaves");
    assert!(world.contains(a));
    assert!(
        world.creatures().get(a).is_none(),
        "a spawned combatant is bare"
    );
    assert_eq!(world.transforms().get(a), Some(&spot));
    assert!(world.combatants().get(a).is_none());
    let member = world.combatants().get(o).expect("O").clone();
    assert!(member.entered());
    leave_encounter(&mut world, o).expect("O leaves");
    assert!(world.contains(o));
    assert_eq!(
        record_json(&world, o),
        expected_record(&member, id(102), &[])
    );
    assert_eq!(
        world.creatures().len(),
        1,
        "only the member is written back"
    );
    assert!(events_of(&mut world).is_empty());
    let _ = b;

    // The same under a registered ruleset (encounter `respawns` absent).
    let mut registered = exit_fixture();
    set_hooks(
        &mut registered,
        Some(CreaturePersistenceWire::Registered),
        None,
        None,
    );
    let mut world = bound(&registered);
    let o = register(&registered, &mut world, O);
    begin(&registered, &mut world, &[entrant(o, 5)]);
    let a = combatant_ids(&world)[0];
    leave_encounter(&mut world, a).expect("A leaves");
    assert!(world.creatures().get(a).is_none());
    leave_encounter(&mut world, o).expect("O leaves");
    assert!(world.creatures().get(o).is_some());

    // Control: a round trip with fixed rounds, under the default and with no
    // initiative rule. O holds `e` with `expires_round == 3`, leaves at round
    // 1, so its record has `suspended == [{e, 2}]`. The encounter rolls over
    // to round 2, and O joins again with the same entrant initiative.
    let fixture = exit_fixture();
    let mut world = bound(&fixture);
    let o = register(&fixture, &mut world, O);
    let mut world = with_record(&world, o, |record| {
        record["suspended"] = json!([{"effect": uid(501), "remaining_rounds": 3}]);
    });
    begin(&fixture, &mut world, &[entrant(o, 5)]);
    let ids = combatant_ids(&world);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        world.combatants().get(o).expect("O").attached()[0].expires_round,
        3
    );
    end(&mut world, o);
    end(&mut world, b);
    end(&mut world, a);
    let combat = world.combat().expect("combat");
    assert_eq!((combat.round, combat.active), (1, Some(o)));
    let at_leave = world.combatants().get(o).expect("O").clone();
    assert_eq!(at_leave.attached()[0].expires_round, 3);
    leave_encounter(&mut world, o).expect("O leaves");
    assert_eq!(
        world.creatures().get(o).expect("record").suspended(),
        [SuspendedEffect {
            effect: id(501),
            remaining_rounds: 2
        }]
    );
    assert_eq!(world.combat().expect("combat").active, Some(b));
    end(&mut world, b);
    end(&mut world, a);
    assert_eq!(world.combat().expect("combat").round, 2);
    join_encounter(&mut world, &entrant(o, 5)).expect("O joins back");
    let mut expected = at_leave.clone();
    let rejoined = world.combatants().get(o).expect("O").clone();
    assert_eq!(
        rejoined.attached(),
        [crpg_sim::AttachedEffect {
            effect: id(501),
            expires_round: 4
        }],
        "round 2 + remaining 2, not 2 (bare), 3 (the old expiry) or 1 + 2"
    );
    // Equal to the leave-time combatant except for the attachment.
    let json_of = |combatant: &Combatant| serde_json::to_value(combatant).expect("serializes");
    let mut expected_json = json_of(&expected);
    expected_json["attached"] = json!([{"effect": uid(501), "expires_round": 4}]);
    assert_eq!(json_of(&rejoined), expected_json);
    expected = rejoined;
    assert!(expected.entered());
}

/// E13 (B16): A leaves at round 1 holding `e` (expires 3): `suspend` and
/// `strip_at_release` give `suspended == [{e, 2}]`, and `strip` gives `[]`.
/// Control: at release, `suspend` keeps `[{e, 2}]` while both strip policies
/// store none.
///
/// **Fails if** the policies are swapped, `strip_at_release` strips at leave,
/// or the remaining rounds are counted from the wrong round.
#[test]
fn e13_effects_at_leave() {
    let stored = [SuspendedEffect {
        effect: id(501),
        remaining_rounds: 2,
    }];
    for (policy, at_leave, at_release) in [
        (None, &stored[..], &stored[..]),
        (Some(EffectsOnExitWire::Suspend), &stored[..], &stored[..]),
        (Some(EffectsOnExitWire::Strip), &[][..], &[][..]),
        (
            Some(EffectsOnExitWire::StripAtRelease),
            &stored[..],
            &[][..],
        ),
    ] {
        for release in [false, true] {
            let mut fixture = exit_fixture();
            set_hooks(&mut fixture, None, None, policy);
            let mut world = bound(&fixture);
            begin(&fixture, &mut world, &[]);
            let ids = combatant_ids(&world);
            let (a, b) = (ids[0], ids[1]);
            end(&mut world, b);
            end(&mut world, a);
            assert_eq!(world.combat().expect("combat").round, 1);
            let mut world = with_json(&world, |save| {
                edit_combatant(save, a, |combatant| {
                    combatant["attached"] = json!([{"effect": uid(501), "expires_round": 3}]);
                });
            });
            if release {
                end_encounter(&mut world).expect("release");
            } else {
                leave_encounter(&mut world, a).expect("A leaves");
            }
            assert_eq!(
                world.creatures().get(a).expect("record").suspended(),
                if release { at_release } else { at_leave },
                "{policy:?}, release: {release}"
            );
        }
    }
}

/// A two-round world in which B (9) is active in round 0 with A (10) after
/// it, optionally with a third combatant C (8) ahead of both.
fn trio_world() -> (Fixture, World, [EntityId; 3]) {
    let mut fixture = Fixture::trio();
    add_outsider(&mut fixture, K, 144);
    let mut world = bound(&fixture);
    begin(&fixture, &mut world, &[]);
    let ids = combatant_ids(&world);
    (fixture, world, [ids[0], ids[1], ids[2]])
}

/// E14 (B29-B31): V1-V4 one by one, unchanged; then four leaves.
///
/// **Fails if** a check is missing or reordered, a non-head leave moves the
/// turn, the head's leave skips the next head's `TurnStart`, the last leave
/// keeps `combat`, or an RT leave below two alive keeps the clock running.
#[test]
fn e14_leave_settlement() {
    let (fixture, world, [a, b, c]) = trio_world();
    let mut dead_fixture = fixture;
    always_success(&mut dead_fixture);
    make_lethal(&mut dead_fixture, 10);
    // V1: no encounter, which also beats V2 (a live record holder).
    let mut idle = bound(&dead_fixture);
    let k = register(&dead_fixture, &mut idle, K);
    leave_fails(&mut idle, k, CombatError::NoEncounter);
    // V2: live, not a combatant.
    let mut with_k = world.clone();
    let k = register(&dead_fixture, &mut with_k, K);
    leave_fails(&mut with_k, k, CombatError::NotParticipant { entity: k });
    // V3: not live (V3 follows V2: a dead id is not "live, not a combatant").
    let mut with_gone = world.clone();
    let gone = with_gone.spawn(EntityMeta {});
    assert!(with_gone.despawn(gone));
    leave_fails(
        &mut with_gone,
        gone,
        CombatError::AbsentActor { actor: gone },
    );
    // V4: a dead combatant awaits release or despawn.
    let mut killed = bound(&dead_fixture);
    begin(&dead_fixture, &mut killed, &[]);
    let [a2, b2, c2] = {
        let ids = combatant_ids(&killed);
        [ids[0], ids[1], ids[2]]
    };
    end(&mut killed, c2);
    end(&mut killed, b2);
    perform_action(&mut killed, &use_action(a2, dead_fixture.ability, b2)).expect("A attacks B");
    assert!(killed.combatants().get(b2).expect("B").dead());
    leave_fails(&mut killed, b2, CombatError::DeadActor { actor: b2 });
    // Control: a live combatant of the same encounter leaves.
    leave_encounter(&mut killed.clone(), a2).expect("A leaves");

    // A non-head leaves, and the turn is untouched. C (8) is head; B leaves.
    let mut run = world.clone();
    events_of(&mut run);
    let before = run.combat().expect("combat").clone();
    let pool_c = pool_of(&run, c);
    leave_encounter(&mut run, b).expect("B leaves");
    let after = run.combat().expect("combat");
    assert_eq!(after.active, before.active);
    assert_eq!(after.round, before.round);
    assert_eq!(timeline_of(&run), [(8, c), (10, a)]);
    assert_eq!(pool_of(&run, c), pool_c);
    assert!(events_of(&mut run).is_empty());

    // The head leaves, and the next head gets a TurnStart refresh. B's pool
    // is emptied first so that the refresh shows.
    let mut leave_run = with_json(&world, |save| {
        edit_combatant(save, b, |combatant| {
            combatant["action_pool"]["current"] = json!(0);
        });
    });
    let mut despawn_run = leave_run.clone();
    events_of(&mut leave_run);
    leave_encounter(&mut leave_run, c).expect("the head leaves");
    assert_eq!(leave_run.combat().expect("combat").active, Some(b));
    assert_eq!(pool_of(&leave_run, b), 1, "the new head is refreshed");
    assert_eq!(timeline_of(&leave_run), [(9, b), (10, a)]);
    assert!(
        events_of(&mut leave_run).is_empty(),
        "no Died, no Despawned"
    );
    // Control: the same despawn gives an equal combat and timeline.
    assert!(despawn_run.despawn(c));
    assert_eq!(despawn_run.combat(), leave_run.combat());
    assert_eq!(timeline_of(&despawn_run), timeline_of(&leave_run));
    assert_eq!(
        serde_json::to_value(despawn_run.combatants()).expect("serializes"),
        serde_json::to_value(leave_run.combatants()).expect("serializes")
    );

    // The last combatant leaves, and `combat` is `None`; the RNG, interners,
    // events and tick are retained.
    let mut last = world.clone();
    leave_encounter(&mut last, a).expect("A leaves");
    leave_encounter(&mut last, b).expect("B leaves");
    assert!(last.combat().is_some());
    events_of(&mut last);
    let interners = last.interners().clone();
    let rng = last.rng_mut().clone();
    leave_encounter(&mut last, c).expect("C leaves");
    assert!(last.combat().is_none());
    assert!(last.combatants().is_empty());
    assert!(last.timeline().is_empty());
    assert_eq!(*last.interners(), interners);
    assert_eq!(*last.rng_mut(), rng);
    assert!(events_of(&mut last).is_empty());
    for entity in [a, b, c] {
        assert!(last.contains(entity));
        assert!(last.creatures().get(entity).is_some());
    }

    // In RT, a leave that drops the alive count to one ends the encounter:
    // the clock stops and a join is refused. Control: a leave that leaves two
    // alive does not.
    let rt = capable(8);
    let mut world = bound(&rt);
    begin_in(&rt, &mut world, EncounterMode::RealTime, &[]);
    let ids = combatant_ids(&world);
    ticks(&mut world, 2);
    assert_eq!(elapsed(&world), 2);
    events_of(&mut world);
    leave_encounter(&mut world, ids[0]).expect("A leaves");
    assert!(events_of(&mut world).is_empty(), "no Died, no Despawned");
    ticks(&mut world, 5);
    assert_eq!(elapsed(&world), 2, "one alive: the clock is frozen");
    join_fails(
        &mut world,
        entrant(ids[0], 1),
        CombatError::EncounterTerminal,
    );
    let trio_rt = capable_trio(8);
    let mut world = bound(&trio_rt);
    begin_in(&trio_rt, &mut world, EncounterMode::RealTime, &[]);
    let ids = combatant_ids(&world);
    ticks(&mut world, 2);
    leave_encounter(&mut world, ids[0]).expect("A leaves");
    ticks(&mut world, 3);
    assert_eq!(elapsed(&world), 5, "two alive: the clock runs");
}

/// A join that must fail with `expected` and leave `world` unchanged.
fn join_fails(world: &mut World, joiner: Entrant, expected: CombatError) {
    let before = snapshot(world);
    assert_eq!(join_encounter(world, &joiner), Err(expected.clone()));
    assert_eq!(snapshot(world), before, "{expected:?} mutated the world");
}
/// E14 (B31): the four ways a combatant stops being one, side by side.
///
/// **Fails if** a leave enqueues `Died` or `Despawned`, a leave keeps the
/// combatant or its entry, a death removes the entity or writes a record
/// early, or a release skips the dead.
#[test]
fn e14_b31_table() {
    let mut fixture = Fixture::trio();
    always_success(&mut fixture);
    make_lethal(&mut fixture, 10);
    let mut base = bound(&fixture);
    begin(&fixture, &mut base, &[]);
    let ids = combatant_ids(&base);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    events_of(&mut base);

    // Death: C (the head) kills B.
    let mut death = base.clone();
    perform_action(&mut death, &use_action(c, fixture.ability, b)).expect("C attacks B");
    assert!(death.contains(b));
    assert!(death.combatants().get(b).expect("kept").dead());
    assert!(!death.timeline().contains(b));
    assert!(
        death.creatures().get(b).is_none(),
        "a record comes at release"
    );
    assert_eq!(events_of(&mut death), [SimEvent::Died { entity: b }]);

    // Leave.
    let mut leave = base.clone();
    leave_encounter(&mut leave, b).expect("B leaves");
    assert!(leave.contains(b));
    assert!(leave.combatants().get(b).is_none());
    assert!(!leave.timeline().contains(b));
    assert!(leave.creatures().get(b).is_some());
    assert!(events_of(&mut leave).is_empty());
    assert!(leave.combat().is_some(), "the encounter continues");

    // Despawn.
    let mut despawn = base.clone();
    assert!(despawn.despawn(b));
    assert!(!despawn.contains(b));
    assert!(despawn.combatants().get(b).is_none());
    assert!(!despawn.timeline().contains(b));
    assert!(despawn.creatures().get(b).is_none());
    assert_eq!(events_of(&mut despawn), [SimEvent::Despawned { entity: b }]);
    assert_eq!(despawn.combat(), leave.combat(), "both settle alike");
    assert_eq!(timeline_of(&despawn), timeline_of(&leave));

    // Release, after a death: the dead are written back too.
    let mut release = death.clone();
    events_of(&mut release);
    end_encounter(&mut release).expect("release");
    for entity in [a, b, c] {
        assert!(release.contains(entity));
        assert!(release.combatants().get(entity).is_none());
    }
    assert!(release.timeline().is_empty());
    assert!(release.combat().is_none());
    assert_eq!(release.creatures().len(), 3);
    assert_eq!(release.creatures().get(b).expect("record").health(), 0);
    assert!(events_of(&mut release).is_empty());
}

/// E16 (B16, B17): under `respawns = Some(true)`, member O enters at start, A
/// and B spawn, outsider J joins at round 0, and A kills J. At release,
/// `creatures()` holds exactly O's record, equal to its release-time
/// combatant values, and J's record with `health == 0`; A and B are bare live
/// entities. The same script under `creature_persistence: registered`
/// (encounter `respawns` absent) gives the same records. Control: under the
/// default all four are written back.
///
/// **Fails if** the release writes back on `!lifecycle.respawns` alone (no
/// record for O or J), writes back every combatant (A and B get records), or
/// skips a dead entered combatant (no record for J).
#[test]
fn e16_entered_written_back_at_release() {
    #[derive(Clone, Copy, Debug)]
    enum Policy {
        EncounterRespawns,
        RulesetRegistered,
        Default,
    }
    for policy in [
        Policy::EncounterRespawns,
        Policy::RulesetRegistered,
        Policy::Default,
    ] {
        let mut fixture = exit_fixture();
        always_success(&mut fixture);
        make_lethal(&mut fixture, 10);
        match policy {
            Policy::EncounterRespawns => fixture.encounter.respawns = Some(true),
            Policy::RulesetRegistered => set_hooks(
                &mut fixture,
                Some(CreaturePersistenceWire::Registered),
                None,
                None,
            ),
            Policy::Default => {}
        }
        let mut world = bound(&fixture);
        let o = register(&fixture, &mut world, O);
        let j = register(&fixture, &mut world, J);
        // O is a member behind both participants (key 20).
        begin(&fixture, &mut world, &[entrant(o, 20)]);
        let ids = combatant_ids(&world);
        let (a, b) = (ids[0], ids[1]);
        assert_eq!(world.combat().expect("combat").active, Some(b));
        join_encounter(&mut world, &entrant(j, 12)).expect("J joins at round 0");
        end(&mut world, b);
        perform_action(&mut world, &use_action(a, fixture.ability, j)).expect("A attacks J");
        let dead = world.combatants().get(j).expect("J").clone();
        // A's hit attached the effect (duration 3, round 0), so the dead
        // combatant carries it into its record under the default policy.
        let killed_with = [SuspendedEffect {
            effect: id(501),
            remaining_rounds: 3,
        }];
        assert!(dead.dead() && dead.entered());
        let member = world.combatants().get(o).expect("O").clone();
        assert_eq!(world.combat().expect("combat").active, Some(o));
        let spawned_a = world.combatants().get(a).expect("A").clone();
        let spawned_b = world.combatants().get(b).expect("B").clone();
        end_encounter(&mut world).expect("release");

        for entity in [a, b, o, j] {
            assert!(world.contains(entity), "{policy:?}");
        }
        let held: Vec<EntityId> = world.creatures().iter().map(|(e, _)| e).collect();
        match policy {
            Policy::EncounterRespawns | Policy::RulesetRegistered => {
                assert_eq!(held, [o, j], "{policy:?}: exactly the entered ones");
                assert_eq!(
                    record_json(&world, o),
                    expected_record(&member, id(102), &[]),
                    "{policy:?}: O's record equals its release-time values"
                );
                assert_eq!(world.creatures().get(j).expect("J").health(), 0);
                assert!(world.creatures().get(j).expect("J").dead());
                assert_eq!(
                    record_json(&world, j),
                    expected_record(&dead, id(102), &killed_with)
                );
                assert!(world.creatures().get(a).is_none(), "{policy:?}: A is bare");
                assert!(world.creatures().get(b).is_none(), "{policy:?}: B is bare");
            }
            Policy::Default => {
                assert_eq!(held, [a, b, o, j], "all four are written back");
                assert_eq!(
                    record_json(&world, a),
                    expected_record(&spawned_a, id(102), &[])
                );
                assert_eq!(
                    record_json(&world, b),
                    expected_record(&spawned_b, id(102), &[])
                );
                assert_eq!(world.creatures().get(j).expect("J").health(), 0);
            }
        }
    }
}

/// The deterministic step of the E15 scripts: the active combatant (TB) or
/// every orderless combatant (RT) attacks the first other living combatant;
/// a TB actor that cannot ends its turn; with no turn, the world ticks.
fn step(world: &mut World, ability: Ulid) {
    let Some(combat) = world.combat() else {
        tick(world);
        return;
    };
    let mode = combat.mode;
    let active = combat.active;
    let living: Vec<EntityId> = world
        .combatants()
        .iter()
        .filter(|(_, combatant)| !combatant.dead())
        .map(|(entity, _)| entity)
        .collect();
    let target_for = |actor: EntityId| living.iter().copied().find(|other| *other != actor);
    match (mode, active) {
        (EncounterMode::TurnBased, Some(actor)) => {
            let used = target_for(actor).is_some_and(|target| {
                perform_action(
                    world,
                    &CombatAction::UseAbility {
                        actor,
                        ability,
                        target,
                    },
                )
                .is_ok()
            });
            if !used {
                let _ = perform_action(world, &CombatAction::EndTurn { actor });
            }
        }
        (EncounterMode::TurnBased, None) => tick(world),
        (EncounterMode::RealTime, _) => {
            for actor in living.clone() {
                let orderless = world
                    .combatants()
                    .get(actor)
                    .is_some_and(|combatant| combatant.order().is_none());
                if let (true, Some(target)) = (orderless, target_for(actor)) {
                    let _ = perform_action(
                        world,
                        &CombatAction::UseAbility {
                            actor,
                            ability,
                            target,
                        },
                    );
                }
            }
            tick(world);
        }
    }
}

fn script_fixture(rt: bool) -> Fixture {
    let mut fixture = if rt { capable(4) } else { Fixture::standard() };
    set_hooks(&mut fixture, None, Some(LateJoinWire::AfterActive), None);
    set_damage(
        &mut fixture,
        json!([
            {"outcome": {"type": "success"}, "amount": 1},
            {"outcome": {"type": "failure"}, "amount": 0}
        ]),
    );
    attach_to_ability(&mut fixture, effect_501(2));
    add_outsider(&mut fixture, O, 141);
    add_outsider(&mut fixture, J, 143);
    add_outsider(&mut fixture, K, 144);
    add_outsider(&mut fixture, 135, 145);
    fixture
}

/// What a script observed, so a vacuous run cannot pass.
#[derive(Debug, Default, PartialEq, Eq)]
struct Seen {
    inserted_at_once: bool,
    pending: bool,
    waited: bool,
    left: bool,
}

/// start(members) -> join (each schedule) -> leave -> 30 actions or ticks ->
/// release, hashing after every step.
fn script(seed: u64, rt: bool) -> (Vec<[u8; 32]>, Seen) {
    let fixture = script_fixture(rt);
    let mut world = World::new_in_area(seed, fixture.area);
    let mut hashes = Vec::new();
    let mut seen = Seen::default();
    let o = register(&fixture, &mut world, O);
    let j1 = register(&fixture, &mut world, J);
    let j2 = register(&fixture, &mut world, K);
    let j3 = register(&fixture, &mut world, 135);
    hashes.push(state_hash(&world));
    let mode = if rt {
        EncounterMode::RealTime
    } else {
        EncounterMode::TurnBased
    };
    begin_in(&fixture, &mut world, mode, &[entrant(o, 5)]);
    hashes.push(state_hash(&world));
    if rt {
        ticks(&mut world, 2);
        hashes.push(state_hash(&world));
    }
    // After the head (TB: behind O at 5), and in RT keyed at `elapsed`.
    join_encounter(&mut world, &entrant(j1, 20)).expect("J1 joins");
    seen.inserted_at_once = world.timeline().contains(j1);
    hashes.push(state_hash(&world));
    // Before the head (TB): pending under after_active.
    join_encounter(&mut world, &entrant(j2, 3)).expect("J2 joins");
    seen.pending = !world.combat().expect("combat").pending_joins.is_empty();
    hashes.push(state_hash(&world));
    let mut joined_late = false;
    for index in 0..30 {
        step(&mut world, fixture.ability);
        hashes.push(state_hash(&world));
        if index == 4 && world.combatants().get(j1).is_some_and(|c| !c.dead()) {
            leave_encounter(&mut world, j1).expect("J1 leaves");
            seen.left = true;
            hashes.push(state_hash(&world));
        }
        let late_round = world.combat().is_some_and(|combat| {
            combat.mode == EncounterMode::TurnBased && combat.round >= 1 && combat.active.is_some()
        });
        if !joined_late && !rt && late_round {
            join_encounter(&mut world, &entrant(j3, 12)).expect("J3 joins");
            seen.waited = !world.timeline().contains(j3)
                && world.combat().expect("combat").pending_joins.is_empty();
            joined_late = true;
            hashes.push(state_hash(&world));
        }
    }
    if world.combat().is_some() {
        end_encounter(&mut world).expect("release");
    }
    hashes.push(state_hash(&world));
    (hashes, seen)
}

/// E15 (B32, B34): two runs of start(members) -> join (each schedule) ->
/// leave -> 30 actions or ticks -> release give equal `state_hash` per step,
/// in TB and in RT; a different seed does not. A save and reload while a TB
/// joiner waits, and again while one is pending, continue with equal hashes.
/// (The existing despawn tests pass unedited, which the full suite proves.)
///
/// **Fails if** any join, leave or flush depends on anything but the world,
/// or a reload loses the waiting or pending joiner.
#[test]
fn e15_unchanged_and_deterministic() {
    let (first, seen) = script(7, false);
    let (second, _) = script(7, false);
    assert_eq!(first, second, "TB: equal per-step hashes");
    assert_eq!(
        seen,
        Seen {
            inserted_at_once: true,
            pending: true,
            waited: true,
            left: true
        },
        "the TB script reached every schedule"
    );
    let (other, _) = script(8, false);
    assert_ne!(first, other, "a different seed differs");
    assert!(first.len() > 30);

    let (first, seen) = script(7, true);
    let (second, _) = script(7, true);
    assert_eq!(first, second, "RT: equal per-step hashes");
    assert!(seen.inserted_at_once && seen.left, "{seen:?}");
    assert_ne!(first, script(9, true).0);

    // A save and reload while J waits (round 1), and while J is pending.
    let fixture = script_fixture(false);
    for pending in [false, true] {
        let mut world = bound(&fixture);
        let j = register(&fixture, &mut world, J);
        begin(&fixture, &mut world, &[]);
        let ids = combatant_ids(&world);
        if pending {
            join_encounter(&mut world, &entrant(j, 3)).expect("J joins");
            assert!(!world.combat().expect("combat").pending_joins.is_empty());
        } else {
            end(&mut world, ids[1]);
            end(&mut world, ids[0]);
            join_encounter(&mut world, &entrant(j, 12)).expect("J joins");
            assert!(!world.timeline().contains(j));
            assert!(world.combat().expect("combat").pending_joins.is_empty());
        }
        let mut reloaded: World =
            serde_json::from_str(&serde_json::to_string(&world).expect("serializes"))
                .expect("the save reloads");
        assert_eq!(reloaded, world);
        assert_eq!(state_hash(&reloaded), state_hash(&world));
        for index in 0..20 {
            step(&mut world, fixture.ability);
            step(&mut reloaded, fixture.ability);
            assert_eq!(
                state_hash(&reloaded),
                state_hash(&world),
                "pending: {pending}, step {index}"
            );
        }
    }
}

/// B30: a leave that causes a rollover records the attachment against the
/// round the leave happened in, not the round it caused. The leaver is the
/// last timeline entry, so its leave rolls the round over.
///
/// **Fails if** `round` is captured after the removal (the record gets 2).
#[test]
fn e13_leave_rollover_uses_the_leave_round() {
    let mut fixture = Fixture::trio();
    attach_to_ability(&mut fixture, effect_501(3));
    let mut world = bound(&fixture);
    begin(&fixture, &mut world, &[]);
    let ids = combatant_ids(&world);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    end(&mut world, c);
    end(&mut world, b);
    assert_eq!(world.combat().expect("combat").active, Some(a));
    assert_eq!(timeline_of(&world), [(10, a)]);
    let mut world = with_json(&world, |save| {
        edit_combatant(save, a, |combatant| {
            combatant["attached"] = json!([{"effect": uid(501), "expires_round": 3}]);
        });
    });
    leave_encounter(&mut world, a).expect("A leaves");
    assert_eq!(
        world.combat().expect("combat").round,
        1,
        "the leave rolled over"
    );
    assert_eq!(
        world.creatures().get(a).expect("record").suspended(),
        [SuspendedEffect {
            effect: id(501),
            remaining_rounds: 3
        }]
    );
}
