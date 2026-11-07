//! T139b1: record-holding members at encounter start (Behaviour B21, B22 and
//! B25 for members; tests E1-E3 and E4's member cases). T139b2 adds joiners
//! and exit in `creature_join.rs` and `creature_exit.rs`.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Ulid};
use crpg_data::Effect;
use crpg_rules::MAX_TAGS;
use crpg_sim::{
    start_encounter_in_mode, start_encounter_with_members, state_hash, AttachedEffect, CombatError,
    EncounterMode, EntityMeta, Entrant, SimEvent, World,
};
use serde_json::json;
use support::creature::{add_outsider, with_record};
use support::draws::{draw, grant, oracle, set_initiative_rule, set_tags};
use support::rt::{capable, timeline_of};
use support::surface::{add_derived, attach_to_ability, binary, effect_doc, lit, modifier, stat};
use support::{uid, Fixture};

const O: u128 = 131;
const P: u128 = 132;

fn id(value: u128) -> Ulid {
    Ulid::from_u128(value)
}

fn bound(fixture: &Fixture) -> World {
    World::new_in_area(5, fixture.area)
}

/// Everything a failed start must leave alone: the bytes, the hash and the
/// RNG (the world's serialization also holds interners and events).
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

fn start_with(
    fixture: &Fixture,
    world: &mut World,
    mode: EncounterMode,
    members: &[Entrant],
) -> Result<(), CombatError> {
    fixture.with_spec(|spec| start_encounter_with_members(world, spec, mode, members))
}

/// A turn-based start with members that must succeed.
fn begin(fixture: &Fixture, world: &mut World, members: &[Entrant]) {
    start_with(fixture, world, EncounterMode::TurnBased, members).expect("the encounter starts");
}

/// A start that must fail with `expected` and leave `world` exactly as it was.
fn begin_fails(fixture: &Fixture, world: &mut World, members: &[Entrant], expected: CombatError) {
    let before = snapshot(world);
    assert_eq!(
        start_with(fixture, world, EncounterMode::TurnBased, members),
        Err(expected.clone())
    );
    assert_eq!(
        snapshot(world),
        before,
        "{expected:?} mutated the world, its RNG or its hash"
    );
    assert!(world.combat().is_none(), "{expected:?} published a combat");
}

/// Registers outsider `placement` and returns its entity.
fn register(fixture: &Fixture, world: &mut World, placement: u128) -> EntityId {
    fixture
        .register(world, id(placement))
        .expect("the outsider registers")
}

/// The standard fixture plus outsiders O (131) and P (132).
fn two_outsiders() -> Fixture {
    let mut fixture = Fixture::standard();
    add_outsider(&mut fixture, O, 141);
    add_outsider(&mut fixture, P, 142);
    fixture
}

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

/// E1 (B21, B22): a member competes for the first turn; it is stored after
/// the participants, keyed by its entrant initiative, loses its record and
/// enqueues nothing. Control: at initiative 20 it is not head and its pool is
/// untouched.
#[test]
fn e1_members_compete() {
    let mut fixture = Fixture::standard();
    add_outsider(&mut fixture, O, 141);
    let mut world = bound(&fixture);
    let o = register(&fixture, &mut world, O);
    let mut world = with_record(&world, o, |record| {
        record["pools"][0]["current"] = json!(0);
    });
    events_of(&mut world);
    assert_eq!(world.creatures().get(o).expect("O").pools()[0].current(), 0);
    let before_hash = state_hash(&world);

    begin(&fixture, &mut world, &[entrant(o, 5)]);
    assert_ne!(state_hash(&world), before_hash);

    let ids = combatant_ids(&world);
    assert_eq!(ids.len(), 3);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(ids[2], o, "the store order is [A, B, O]");
    assert_ne!(a, o);
    assert_ne!(b, o);
    assert_eq!(
        world.combatants().get(a).expect("A").placement(),
        fixture.placement_a
    );
    assert_eq!(
        world.combatants().get(b).expect("B").placement(),
        fixture.placement_b
    );
    assert_eq!(timeline_of(&world), [(5, o), (9, b), (10, a)]);
    assert_eq!(world.combat().expect("combat").active, Some(o));
    assert!(world.creatures().get(o).is_none(), "O has no record");
    assert_eq!(
        events_of(&mut world),
        [
            SimEvent::Spawned { entity: a },
            SimEvent::Spawned { entity: b }
        ],
        "only the two spawns are enqueued"
    );
    let member = world.combatants().get(o).expect("O");
    assert!(member.entered());
    assert_eq!(member.initiative(), 5);
    assert_eq!(member.action_pool().current(), 1, "refreshed as head");
    assert_eq!(
        member.placement(),
        id(O),
        "the member keeps its own placement"
    );

    // Control: at initiative 20 O is not head, so its pool stays 0.
    let mut world = bound(&fixture);
    let o = register(&fixture, &mut world, O);
    let mut world = with_record(&world, o, |record| {
        record["pools"][0]["current"] = json!(0);
    });
    begin(&fixture, &mut world, &[entrant(o, 20)]);
    let ids = combatant_ids(&world);
    assert_eq!(timeline_of(&world), [(9, ids[1]), (10, ids[0]), (20, o)]);
    assert_eq!(world.combat().expect("combat").active, Some(ids[1]));
    assert_eq!(
        world
            .combatants()
            .get(o)
            .expect("O")
            .action_pool()
            .current(),
        0
    );
    assert!(world.creatures().get(o).is_none());
}

/// E1's equality control (B22): `members == &[]` is byte-equal to
/// `start_encounter_in_mode`, including the RNG and the events, in both modes
/// and with records, a draw rule and a granting effect in play.
#[test]
fn e1_empty_members_equal_start_in_mode() {
    for mode in [EncounterMode::TurnBased, EncounterMode::RealTime] {
        for rolled in [false, true] {
            let mut fixture = capable(8);
            add_outsider(&mut fixture, O, 141);
            attach_to_ability(&mut fixture, grant(effect_501(3), &["g"]));
            if rolled {
                set_initiative_rule(&mut fixture, "1d6", &["alpha"]);
            }
            let make = || {
                let mut world = bound(&fixture);
                let a = register(&fixture, &mut world, 111);
                register(&fixture, &mut world, O);
                events_of(&mut world);
                (world, a)
            };

            let (mut plain, _) = make();
            fixture
                .with_spec(|spec| start_encounter_in_mode(&mut plain, spec, mode))
                .expect("starts");
            let (mut members, _) = make();
            start_with(&fixture, &mut members, mode, &[]).expect("starts");

            assert_eq!(
                serde_json::to_vec(&plain).expect("serializes"),
                serde_json::to_vec(&members).expect("serializes"),
                "{mode:?} rolled {rolled}"
            );
            assert_eq!(state_hash(&plain), state_hash(&members));
            assert_eq!(plain.rng_mut().clone(), members.rng_mut().clone());
            assert_eq!(events_of(&mut plain), events_of(&mut members));
            assert_eq!(plain.interners(), members.interners());
        }
    }
}

/// The M0 world: `count` registered outsiders next to the two authored
/// participants.
fn crowd(fixture: &mut Fixture, count: u128) -> (World, Vec<Entrant>) {
    for index in 0..count {
        add_outsider(fixture, 10_000 + index, 20_000 + index);
    }
    let mut world = bound(fixture);
    let mut members = Vec::new();
    for index in 0..count {
        let entity = register(fixture, &mut world, 10_000 + index);
        members.push(entrant(entity, 1));
    }
    (world, members)
}

fn tag_names(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("t{index:05}")).collect()
}

/// The fixture every E2 case shares: a derived `ratio = alpha / (gamma - 4)`
/// that fails at gamma 4, a rolled initiative (so that a failed start would
/// leave an RNG trace if it drew early), an effect that grants `zzz-new`, and
/// outsider O.
fn e2_fixture() -> Fixture {
    let mut fixture = Fixture::standard();
    add_derived(
        &mut fixture,
        "ratio",
        binary(
            "divide",
            stat("alpha"),
            binary("subtract", stat("gamma"), lit(4)),
        ),
    );
    set_initiative_rule(&mut fixture, "1d6", &["alpha"]);
    attach_to_ability(&mut fixture, grant(effect_501(3), &["zzz-new"]));
    add_outsider(&mut fixture, O, 141);
    fixture
}

/// E2 (B21): M0 to M8 one by one, each leaving the world unchanged, each with
/// its fixed control starting.
#[test]
fn e2_member_checks() {
    let fixture = e2_fixture();
    let mut base = bound(&fixture);
    let o = register(&fixture, &mut base, O);
    let good = [entrant(o, 5)];

    // The unedited member starts (the control every fault below corrects to).
    let mut control = base.clone();
    begin(&fixture, &mut control, &good);
    assert!(control.combatants().get(o).expect("O").entered());

    // M0: 1023 members and 2 participants is one over `MAX_COMBATANTS`.
    {
        let mut crowded = e2_fixture();
        let (mut world, members) = crowd(&mut crowded, 1023);
        assert_eq!(members.len(), 1023);
        begin_fails(
            &crowded,
            &mut world,
            &members,
            CombatError::TooManyParticipants,
        );
        // Control: one fewer fills the bound exactly.
        let mut ok = world.clone();
        begin(&crowded, &mut ok, &members[..1022]);
        assert_eq!(ok.combatants().len(), 1024);
    }

    // M1: a dead id.
    let mut with_gone = base.clone();
    let gone = with_gone.spawn(EntityMeta {});
    assert!(with_gone.despawn(gone));
    begin_fails(
        &fixture,
        &mut with_gone,
        &[entrant(gone, 5)],
        CombatError::AbsentActor { actor: gone },
    );
    // M1 comes before M2: a dead id listed twice is absent first.
    begin_fails(
        &fixture,
        &mut with_gone,
        &[entrant(gone, 5), entrant(gone, 6)],
        CombatError::AbsentActor { actor: gone },
    );
    begin(&fixture, &mut with_gone.clone(), &good);

    // M2: one member listed twice.
    let mut world = base.clone();
    begin_fails(
        &fixture,
        &mut world,
        &[entrant(o, 5), entrant(o, 6)],
        CombatError::AlreadyParticipant { entity: o },
    );
    begin(&fixture, &mut world, &[entrant(o, 5)]);

    // M3: a bare entity.
    let mut world = base.clone();
    let bare = world.spawn(EntityMeta {});
    begin_fails(
        &fixture,
        &mut world,
        &[entrant(bare, 5)],
        CombatError::NotCreature { entity: bare },
    );
    // Pair: M2 beats M3 (a bare entity listed twice).
    begin_fails(
        &fixture,
        &mut world,
        &[entrant(bare, 5), entrant(bare, 6)],
        CombatError::AlreadyParticipant { entity: bare },
    );
    begin(&fixture, &mut world, &[entrant(o, 5)]);

    // M4: health zero.
    let dead = with_record(&base, o, |record| record["health"] = json!(0));
    begin_fails(
        &fixture,
        &mut dead.clone(),
        &good,
        CombatError::DeadActor { actor: o },
    );
    // Pair: M4 beats M5 (a dead record with another ruleset id).
    let dead_and_foreign = with_record(&base, o, |record| {
        record["health"] = json!(0);
        record["ruleset"] = json!(uid(999));
    });
    begin_fails(
        &fixture,
        &mut dead_and_foreign.clone(),
        &good,
        CombatError::DeadActor { actor: o },
    );

    // M5: a ruleset id changed; an attribute order swapped; a pool changed.
    let mismatch = CombatError::RulesetMismatch { entity: o };
    let foreign = with_record(&base, o, |record| record["ruleset"] = json!(uid(999)));
    begin_fails(&fixture, &mut foreign.clone(), &good, mismatch.clone());
    let swapped = with_record(&base, o, |record| {
        record["attributes"] = json!([["beta", 6], ["alpha", 8], ["gamma", 7]]);
    });
    begin_fails(&fixture, &mut swapped.clone(), &good, mismatch.clone());
    let repooled = with_record(&base, o, |record| record["pools"][0]["max"] = json!(5));
    begin_fails(&fixture, &mut repooled.clone(), &good, mismatch);

    // M6: the member's placement is an authored participant's. Under
    // `respawns` the participants spawn, and the record of placement A is
    // still a member in use.
    let mut respawning = e2_fixture();
    respawning.encounter.respawns = Some(true);
    let mut world = bound(&respawning);
    let record_a = register(&respawning, &mut world, 111);
    let record_o = register(&respawning, &mut world, O);
    begin_fails(
        &respawning,
        &mut world,
        &[entrant(record_a, 5)],
        CombatError::PlacementInUse {
            placement: respawning.placement_a,
        },
    );
    // Control: the outsider's own placement is free.
    begin(&respawning, &mut world.clone(), &[entrant(record_o, 5)]);
    // M6 also holds without `respawns`, when the authored participant would
    // have entered from that very record.
    let mut world = bound(&fixture);
    let record_a = register(&fixture, &mut world, 111);
    begin_fails(
        &fixture,
        &mut world,
        &[entrant(record_a, 5)],
        CombatError::PlacementInUse {
            placement: fixture.placement_a,
        },
    );

    // M7: gamma 4 makes `ratio` divide by zero.
    let broken = with_record(&base, o, |record| {
        record["attributes"][2][1] = json!(4);
    });
    begin_fails(
        &fixture,
        &mut broken.clone(),
        &good,
        CombatError::InvalidStatValue {
            stat: "ratio".to_owned(),
        },
    );

    // M8: `MAX_TAGS` tags plus the effect's new grant. Control: one fewer
    // fits.
    let names = tag_names(MAX_TAGS);
    let full = with_record(&base, o, |record| record["tags"] = json!(&names));
    begin_fails(&fixture, &mut full.clone(), &good, CombatError::TooManyTags);
    let fits = with_record(&base, o, |record| {
        record["tags"] = json!(&names[..MAX_TAGS - 1]);
    });
    begin(&fixture, &mut fits.clone(), &good);

    // Pair: a `MissingPlacement` in the spec beats a bad member.
    let mut missing = e2_fixture();
    let placement_b = missing.placement_b;
    missing.placements.remove(&placement_b);
    let mut world = base.clone();
    let bare = world.spawn(EntityMeta {});
    begin_fails(
        &missing,
        &mut world,
        &[entrant(bare, 5)],
        CombatError::MissingPlacement {
            placement: placement_b,
        },
    );

    // Pair: S1 on an authored Enter participant beats a bad member.
    let mut world = bound(&fixture);
    let record_a = register(&fixture, &mut world, 111);
    let mut world = with_record(&world, record_a, |record| {
        record["ruleset"] = json!(uid(999));
    });
    let bare = world.spawn(EntityMeta {});
    begin_fails(
        &fixture,
        &mut world,
        &[entrant(bare, 5)],
        CombatError::RulesetMismatch { entity: record_a },
    );

    // Pair: S4 counts members. Both authored records dead, no member:
    // `NoEntrants`; one member: it starts alone.
    let mut world = bound(&fixture);
    let record_a = register(&fixture, &mut world, 111);
    let record_b = register(&fixture, &mut world, 112);
    let o = register(&fixture, &mut world, O);
    let world = with_record(&world, record_a, |record| record["health"] = json!(0));
    let world = with_record(&world, record_b, |record| record["health"] = json!(0));
    begin_fails(&fixture, &mut world.clone(), &[], CombatError::NoEntrants);
    let mut alone = world.clone();
    begin(&fixture, &mut alone, &[entrant(o, 5)]);
    assert_eq!(combatant_ids(&alone), [o]);
    let key = alone.combatants().get(o).expect("O").initiative();
    assert_eq!(timeline_of(&alone), [(key, o)]);
    assert_eq!(alone.combat().expect("combat").active, Some(o));

    // The new errors render without their fields.
    assert_eq!(
        CombatError::NotCreature { entity: o }.to_string(),
        "NotCreature at creatures"
    );
    assert_eq!(
        CombatError::AlreadyParticipant { entity: o }.to_string(),
        "AlreadyParticipant at combatants"
    );
}

/// E3 (B22, B25): under an initiative rule the member draws come after the
/// participants' draws, in slice order, and each key is roll plus the
/// record's bonus plus the entrant's base. Controls: with no rule nothing is
/// drawn; a real-time start keys the member at 0, schedules no head and
/// stores the drawn key for a later rebuild.
#[test]
fn e3_member_draws() {
    let mut fixture = two_outsiders();
    set_initiative_rule(&mut fixture, "1d6", &["alpha"]);
    let mut world = bound(&fixture);
    let o = register(&fixture, &mut world, O);
    let p = register(&fixture, &mut world, P);
    // P has alpha 3 (the prefab says 8), so that the bonus reads the record
    // of the member in question.
    let world = with_record(&world, p, |record| {
        record["attributes"][0][1] = json!(3);
    });

    let mut started = world.clone();
    begin(&fixture, &mut started, &[entrant(p, 4), entrant(o, 5)]);
    let mut expected = oracle(5);
    let roll_a = draw(&mut expected, "1d6").total;
    let roll_b = draw(&mut expected, "1d6").total;
    let roll_p = draw(&mut expected, "1d6").total;
    let roll_o = draw(&mut expected, "1d6").total;
    let ids = combatant_ids(&started);
    assert_eq!(ids.len(), 4);
    assert_eq!(&ids[2..], [p, o], "members are stored in slice order");
    let key_a = roll_a + 8 + 10;
    let key_b = roll_b + 6 + 9;
    let key_p = roll_p + 3 + 4;
    let key_o = roll_o + 8 + 5;
    assert_eq!(
        started.combatants().get(ids[0]).expect("A").initiative(),
        key_a
    );
    assert_eq!(
        started.combatants().get(ids[1]).expect("B").initiative(),
        key_b
    );
    assert_eq!(started.combatants().get(p).expect("P").initiative(), key_p);
    assert_eq!(started.combatants().get(o).expect("O").initiative(), key_o);
    assert_eq!(*started.rng_mut(), expected, "exactly four draws, in order");
    let mut timeline = timeline_of(&started);
    timeline.sort();
    let mut keyed = vec![(key_a, ids[0]), (key_b, ids[1]), (key_p, p), (key_o, o)];
    keyed.sort();
    assert_eq!(timeline, keyed);

    // Control: no rule, so nothing is drawn and the keys are the bases.
    let mut plain = two_outsiders();
    plain.ruleset.initiative = None;
    let mut world = bound(&plain);
    let o = register(&plain, &mut world, O);
    let p = register(&plain, &mut world, P);
    let mut fresh = bound(&plain);
    begin(&plain, &mut world, &[entrant(p, 4), entrant(o, 5)]);
    assert_eq!(*world.rng_mut(), *fresh.rng_mut(), "nothing was drawn");
    assert_eq!(world.combatants().get(p).expect("P").initiative(), 4);
    assert_eq!(world.combatants().get(o).expect("O").initiative(), 5);
    let ids = combatant_ids(&world);
    assert_eq!(
        timeline_of(&world),
        [(4, p), (5, o), (9, ids[1]), (10, ids[0])]
    );

    // Control: a real-time start with no rule keys the member at 0, holds no
    // head and keeps the base key on the combatant.
    let mut capable_fixture = capable(8);
    add_outsider(&mut capable_fixture, O, 141);
    let mut world = bound(&capable_fixture);
    let o = register(&capable_fixture, &mut world, O);
    start_with(
        &capable_fixture,
        &mut world,
        EncounterMode::RealTime,
        &[entrant(o, 5)],
    )
    .expect("starts in real time");
    let state = world.combat().expect("combat");
    assert_eq!(state.active, None);
    let ids = combatant_ids(&world);
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[2], o);
    let mut seen = timeline_of(&world);
    seen.sort();
    let mut zero = vec![(0, ids[0]), (0, ids[1]), (0, o)];
    zero.sort();
    assert_eq!(seen, zero);
    assert_eq!(world.combatants().get(o).expect("O").initiative(), 5);
    assert_eq!(
        world
            .combatants()
            .get(o)
            .expect("O")
            .action_pool()
            .current(),
        1,
        "a real-time start refreshes nothing"
    );

    // Real time under a rule: still keyed 0, and the drawn key is stored.
    set_initiative_rule(&mut capable_fixture, "1d6", &["alpha"]);
    let mut world = bound(&capable_fixture);
    let o = register(&capable_fixture, &mut world, O);
    start_with(
        &capable_fixture,
        &mut world,
        EncounterMode::RealTime,
        &[entrant(o, 5)],
    )
    .expect("starts in real time");
    let mut expected = oracle(5);
    draw(&mut expected, "1d6");
    draw(&mut expected, "1d6");
    let roll_o = draw(&mut expected, "1d6").total;
    assert_eq!(*world.rng_mut(), expected);
    assert_eq!(world.combat().expect("combat").active, None);
    assert!(timeline_of(&world).iter().all(|(key, _)| *key == 0));
    assert_eq!(
        world.combatants().get(o).expect("O").initiative(),
        roll_o + 8 + 5
    );
}

/// E4 (B14, D5; member cases): a suspended effect resumes at
/// `round + remaining_rounds` (round 0 here); one the definition lacks is
/// dropped. A member's tags come from its record, not its prefab, and carry
/// no grant, with or without a granting effect.
///
/// **Fails if** entry stores bare `remaining_rounds` for a nonzero round (see
/// T139b2's joiner cases), gives a member no tags, takes its tags from its
/// prefab (P gets `["a","b"]`), adds a grant, or keeps B19's gate (no tags
/// without the grant).
#[test]
fn e4_member_entrant_seeding() {
    let held = json!([
        {"effect": uid(501), "remaining_rounds": 2},
        {"effect": uid(502), "remaining_rounds": 1}
    ]);

    // The definition knows effect 501 and not 502.
    let mut fixture = two_outsiders();
    attach_to_ability(&mut fixture, effect_501(3));
    let mut world = bound(&fixture);
    let o = register(&fixture, &mut world, O);
    let world = with_record(&world, o, |record| record["suspended"] = held.clone());
    let mut started = world.clone();
    begin(&fixture, &mut started, &[entrant(o, 5)]);
    assert_eq!(
        started.combatants().get(o).expect("O").attached(),
        [AttachedEffect {
            effect: id(501),
            expires_round: 2
        }],
        "501 resumes at round 0 + 2; 502 is dropped"
    );

    // Control: with no effect in the definition, nothing resumes.
    let bare_fixture = two_outsiders();
    let mut started = world.clone();
    begin(&bare_fixture, &mut started, &[entrant(o, 5)]);
    assert!(started
        .combatants()
        .get(o)
        .expect("O")
        .attached()
        .is_empty());

    // Tags: O's and P's prefab (a clone of creature A) has tags
    // `["b","a","a"]`, so both records hold `["a","b"]`; P's record then
    // holds `["c"]`.
    for granting in [true, false] {
        let mut fixture = two_outsiders();
        set_tags(&mut fixture, id(141), &["b", "a", "a"]);
        set_tags(&mut fixture, id(142), &["b", "a", "a"]);
        if granting {
            attach_to_ability(&mut fixture, grant(effect_501(3), &["g"]));
        }
        let mut world = bound(&fixture);
        let o = register(&fixture, &mut world, O);
        let p = register(&fixture, &mut world, P);
        assert_eq!(world.creatures().get(o).expect("O").tags(), ["a", "b"]);
        assert_eq!(world.creatures().get(p).expect("P").tags(), ["a", "b"]);
        let mut world = with_record(&world, p, |record| record["tags"] = json!(["c"]));
        begin(&fixture, &mut world, &[entrant(o, 5), entrant(p, 6)]);
        assert_eq!(
            world.combatants().get(o).expect("O").tags(),
            ["a", "b"],
            "granting: {granting}"
        );
        assert_eq!(
            world.combatants().get(p).expect("P").tags(),
            ["c"],
            "granting: {granting}"
        );
        let spawned = combatant_ids(&world)[0];
        assert!(
            world
                .combatants()
                .get(spawned)
                .expect("A")
                .tags()
                .is_empty(),
            "an authored participant of a tagless prefab carries none"
        );
    }

    // The rest of B14 for a member: balances, fixed stats, `entered`, no
    // order.
    let fixture = two_outsiders();
    let mut world = bound(&fixture);
    let o = register(&fixture, &mut world, O);
    let record = world.creatures().get(o).expect("O").clone();
    begin(&fixture, &mut world, &[entrant(o, 5)]);
    let member = world.combatants().get(o).expect("O");
    assert_eq!(member.attributes(), record.attributes());
    assert_eq!(member.fixed(), record.fixed());
    assert_eq!((member.health(), member.max_health()), (10, 10));
    assert_eq!(member.action_pool(), &record.pools()[0]);
    assert!(member.entered());
    assert!(!member.dead());
    assert!(member.order().is_none());
}
