//! Two-keys contract tests for T139b: members at start, join and leave
//! (B21-B34), written from `tasks/T139.md` alone.
//!
//! Every test names the Behaviour items it proves. A rejection test always
//! checks that the whole world is untouched (hash, store, `combat`, next RNG
//! draw, full equality), and most tests carry a control that differs in one
//! input.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Ulid};
use crpg_data::{
    ActionPoolTemplate, CreaturePersistenceWire, Effect, EffectsOnExitWire, EncounterParticipant,
    LateJoinWire, RefreshWire,
};
use crpg_rules::ResourcePool;
use crpg_sim::{
    end_encounter, join_encounter, leave_encounter, perform_action, start_encounter_in_mode,
    start_encounter_with_members, state_hash, tick, ActionOutcome, CombatAction, CombatError,
    CombatState, Combatant, EncounterMode, EntityMeta, Entrant, HistoryEvent, SimEvent, Transform,
    World, COMBAT_ROLL_STREAM, MAX_COMBATANTS,
};
use serde_json::{json, Value};
use support::creature::{
    add_outsider, edit_combatant, set_hooks, try_with_json, with_json, with_record,
};
use support::draws::{draw, grant, oracle, set_initiative_rule};
use support::rt::{capable, capable_trio, elapsed, make_lethal, pool_of, ticks, timeline_of};
use support::surface::{
    add_derived, attach_to_ability, binary, edit_ability, effect_doc, lit, modifier, stat,
};
use support::{uid, Fixture};

/// A named record edit.
type Named = (&'static str, fn(&mut Value));

const SEED: u64 = 7;
const TB: EncounterMode = EncounterMode::TurnBased;
const RT: EncounterMode = EncounterMode::RealTime;

// ---------------------------------------------------------------- helpers

fn ulid(value: u128) -> Ulid {
    Ulid::from_u128(value)
}

fn entrant(entity: EntityId, initiative: i32) -> Entrant {
    Entrant { entity, initiative }
}

fn ids(world: &World) -> Vec<EntityId> {
    world.combatants().iter().map(|(id, _)| id).collect()
}

fn combat(world: &World) -> &CombatState {
    world.combat().expect("an encounter")
}

fn active(world: &World) -> Option<EntityId> {
    combat(world).active
}

fn combatant(world: &World, id: EntityId) -> &Combatant {
    world.combatants().get(id).expect("a combatant")
}

fn events(world: &mut World) -> Vec<SimEvent> {
    world
        .events_mut()
        .drain()
        .into_iter()
        .map(|envelope| envelope.payload)
        .collect()
}

fn saved(world: &World) -> Value {
    serde_json::to_value(world).expect("the world serializes")
}

fn reloaded(world: &World) -> World {
    serde_json::from_slice(&serde_json::to_vec(world).expect("serializes")).expect("reloads")
}

/// The next draw on the combat stream of a clone of `world`.
fn next_draw(world: &World) -> u32 {
    world
        .clone()
        .rng_mut()
        .stream(COMBAT_ROLL_STREAM)
        .next_u32()
}

fn assert_untouched(before: &World, after: &World) {
    assert_eq!(state_hash(before), state_hash(after), "state_hash");
    assert_eq!(before.creatures(), after.creatures(), "creatures");
    assert_eq!(before.combat(), after.combat(), "combat");
    assert_eq!(next_draw(before), next_draw(after), "next draw");
    assert_eq!(before, after, "the whole world");
}

fn start_with(
    fixture: &Fixture,
    world: &mut World,
    mode: EncounterMode,
    members: &[Entrant],
) -> Result<(), CombatError> {
    fixture.with_spec(|spec| start_encounter_with_members(world, spec, mode, members))
}

fn start_rejects(fixture: &Fixture, world: &World, members: &[Entrant], expected: CombatError) {
    let mut probe = world.clone();
    assert_eq!(start_with(fixture, &mut probe, TB, members), Err(expected));
    assert_untouched(world, &probe);
}

fn join_rejects(world: &World, joiner: Entrant, expected: CombatError) {
    let mut probe = world.clone();
    assert_eq!(join_encounter(&mut probe, &joiner), Err(expected));
    assert_untouched(world, &probe);
}

fn leave_rejects(world: &World, entity: EntityId, expected: CombatError) {
    let mut probe = world.clone();
    assert_eq!(leave_encounter(&mut probe, entity), Err(expected));
    assert_untouched(world, &probe);
}

fn stale_id(world: &mut World) -> EntityId {
    let id = world.spawn(EntityMeta {});
    assert!(world.despawn(id));
    id
}

fn atk(
    world: &mut World,
    actor: EntityId,
    target: EntityId,
) -> Result<Option<ActionOutcome>, CombatError> {
    perform_action(
        world,
        &CombatAction::UseAbility {
            actor,
            ability: ulid(103),
            target,
        },
    )
}

fn end(world: &mut World, actor: EntityId) {
    perform_action(world, &CombatAction::EndTurn { actor }).expect("the turn ends");
}

fn zero_pools(record: &mut Value) {
    for pool in record["pools"].as_array_mut().expect("pools") {
        pool["current"] = json!(0);
    }
}

fn pools_of(c: &Combatant) -> Vec<ResourcePool> {
    let mut pools = vec![c.action_pool().clone()];
    pools.extend(c.extra_pools().iter().cloned());
    pools
}

/// A second pool refreshed at round start, so a rollover refresh shows.
fn round_pool(fixture: &mut Fixture) {
    fixture.ruleset.pools.push(ActionPoolTemplate {
        id: ulid(106),
        max: 3,
        refresh: RefreshWire::OnRoundStart,
    });
}

fn granting(id: u128, tags: &[&str]) -> Effect {
    grant(
        effect_doc(
            id,
            "target",
            "probe-type",
            "stack_all",
            vec![modifier(id + 1, json!({"type": "roll"}), "add", 1)],
            2,
        ),
        tags,
    )
}

/// Effect `601`, which the standard ability applies, so definitions list it.
fn with_effect(fixture: &mut Fixture) {
    attach_to_ability(
        fixture,
        effect_doc(
            601,
            "target",
            "probe-type",
            "stack_all",
            vec![modifier(602, json!({"type": "roll"}), "add", 1)],
            3,
        ),
    );
}

fn tag_names(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("tag-{index:04}")).collect()
}

/// The standard fixture, an outsider `O` (placement 131) registered with
/// every pool at zero, and a fresh world holding only the record.
fn member_setup() -> (Fixture, World, EntityId) {
    let mut f = Fixture::standard();
    let mut w = World::new(SEED);
    add_outsider(&mut f, 131, 141);
    let o = f.register(&mut w, ulid(131)).expect("registers");
    let mut w = with_record(&w, o, zero_pools);
    events(&mut w);
    (f, w, o)
}

/// A running encounter plus its fixture; outsiders registered through it
/// come after the start, so their entities sort after every participant.
struct Play {
    f: Fixture,
    w: World,
    a: EntityId,
    b: EntityId,
    next: u128,
}

impl Play {
    fn new(f: Fixture) -> Self {
        Self::in_mode(f, TB)
    }

    fn in_mode(f: Fixture, mode: EncounterMode) -> Self {
        let mut w = World::new(SEED);
        start_with(&f, &mut w, mode, &[]).expect("starts");
        let v = ids(&w);
        Play {
            f,
            w,
            a: v[0],
            b: v[1],
            next: 1,
        }
    }

    fn outsider(&mut self) -> EntityId {
        let n = self.next;
        self.next += 1;
        add_outsider(&mut self.f, 200 + n, 300 + n);
        self.f
            .register(&mut self.w, ulid(200 + n))
            .expect("registers")
    }

    /// An outsider whose every pool is at zero.
    fn zeroed(&mut self) -> EntityId {
        let e = self.outsider();
        self.edit(e, zero_pools);
        e
    }

    fn edit(&mut self, entity: EntityId, edit: impl FnOnce(&mut Value)) {
        self.w = with_record(&self.w, entity, edit);
    }

    fn quiet(&mut self) {
        self.w.events_mut().drain();
    }

    fn join(&mut self, entity: EntityId, key: i32) -> Result<(), CombatError> {
        join_encounter(&mut self.w, &entrant(entity, key))
    }

    fn tl(&self) -> Vec<(i32, EntityId)> {
        timeline_of(&self.w)
    }

    fn pending(&self) -> Vec<EntityId> {
        combat(&self.w).pending_joins.clone()
    }
}

fn after_active() -> Fixture {
    let mut f = Fixture::standard();
    set_hooks(&mut f, None, Some(LateJoinWire::AfterActive), None);
    f
}

/// Round 1 of the standard TB encounter: B and A end their turns.
fn into_round_one(p: &mut Play) {
    let (a, b) = (p.a, p.b);
    end(&mut p.w, b);
    end(&mut p.w, a);
    assert_eq!(combat(&p.w).round, 1);
    assert_eq!(active(&p.w), Some(b));
}

// ------------------------------------------------- B21, B22: members

/// B21, B22 (E1): a member publishes after the participants, keyed by its
/// entrant key, competes for the first turn and is refreshed as head.
#[test]
fn b22_member_competes_for_the_first_turn() {
    let (f, mut w, o) = member_setup();
    start_with(&f, &mut w, TB, &[entrant(o, 5)]).expect("starts");
    let v = ids(&w);
    assert_eq!(v.len(), 3);
    let (a, b) = (v[0], v[1]);
    assert_eq!(v[2], o, "members follow the participants in the store");
    assert_eq!(timeline_of(&w), [(5, o), (9, b), (10, a)]);
    assert_eq!(active(&w), Some(o));
    assert_eq!(combat(&w).round, 0);
    assert!(w.creatures().is_empty(), "the record is removed");
    assert_eq!(w.len(), 3, "a member is not spawned");
    assert_eq!(
        events(&mut w),
        [
            SimEvent::Spawned { entity: a },
            SimEvent::Spawned { entity: b }
        ]
    );
    let c = combatant(&w, o);
    assert!(c.entered() && !c.dead());
    assert_eq!(c.initiative(), 5);
    assert_eq!(pool_of(&w, o), 1, "the head is refreshed at turn start");
    assert!(!combatant(&w, a).entered());

    // Control: key 20 sorts last, so the member is no head and stays at 0.
    let (f, mut w, o) = member_setup();
    start_with(&f, &mut w, TB, &[entrant(o, 20)]).expect("starts");
    let (a, b) = (ids(&w)[0], ids(&w)[1]);
    assert_eq!(timeline_of(&w), [(9, b), (10, a), (20, o)]);
    assert_eq!(active(&w), Some(b));
    assert_eq!(pool_of(&w, o), 0);
}

/// B22: two members publish in slice order after the participants.
#[test]
fn b22_members_publish_in_slice_order() {
    let (mut f, mut w, o1) = member_setup();
    add_outsider(&mut f, 132, 142);
    let o2 = f.register(&mut w, ulid(132)).expect("registers");
    start_with(&f, &mut w, TB, &[entrant(o2, 6), entrant(o1, 7)]).expect("starts");
    let v = ids(&w);
    assert_eq!(&v[2..], [o2, o1], "slice order, not entity order");
    // Control: the other slice order flips the store order.
    let (mut f, mut w, o1) = member_setup();
    add_outsider(&mut f, 132, 142);
    let o2 = f.register(&mut w, ulid(132)).expect("registers");
    start_with(&f, &mut w, TB, &[entrant(o1, 7), entrant(o2, 6)]).expect("starts");
    assert_eq!(&ids(&w)[2..], [o1, o2]);
}

/// B22: an empty member list is exactly `start_encounter_in_mode`, in both
/// modes, with and without drawn initiative.
#[test]
fn b22_empty_members_equal_the_plain_start() {
    for mode in [TB, RT] {
        for rolled in [false, true] {
            let mut f = capable(10);
            if rolled {
                set_initiative_rule(&mut f, "1d6", &["alpha"]);
            }
            let mut x = World::new(SEED);
            f.with_spec(|spec| start_encounter_in_mode(&mut x, spec, mode))
                .expect("starts");
            let mut y = World::new(SEED);
            start_with(&f, &mut y, mode, &[]).expect("starts");
            assert_eq!(
                serde_json::to_vec(&x).unwrap(),
                serde_json::to_vec(&y).unwrap()
            );
            assert_eq!(x, y, "events and RNG included");
        }
    }
}

/// B22: `join_after_active` copies `late_join` at every start, members or not.
#[test]
fn b22_join_after_active_is_published_at_every_start() {
    for (hook, expected) in [
        (None, false),
        (Some(LateJoinWire::NextRound), false),
        (Some(LateJoinWire::AfterActive), true),
    ] {
        for members in [false, true] {
            let (mut f, mut w, o) = member_setup();
            set_hooks(&mut f, None, hook, None);
            let list = [entrant(o, 5)];
            start_with(&f, &mut w, TB, if members { &list } else { &[] }).expect("starts");
            assert_eq!(combat(&w).lifecycle.join_after_active, expected);
            let lifecycle = &saved(&w)["combat"]["lifecycle"];
            if expected {
                assert_eq!(lifecycle["join_after_active"], json!(true));
            } else {
                assert!(lifecycle.is_null(), "default lifecycle writes no key");
            }
        }
    }
}

/// B22: a real-time start keys every member at 0 and schedules no head.
#[test]
fn b22_rt_start_keys_a_member_at_zero() {
    let mut f = capable(10);
    let mut w = World::new(SEED);
    add_outsider(&mut f, 131, 141);
    let o = f.register(&mut w, ulid(131)).expect("registers");
    start_with(&f, &mut w, RT, &[entrant(o, 5)]).expect("starts");
    let t = timeline_of(&w);
    assert_eq!(t.len(), 3);
    assert!(t.iter().all(|(key, _)| *key == 0));
    assert!(t.iter().any(|(_, id)| *id == o));
    assert_eq!(active(&w), None);
    assert_eq!(combatant(&w, o).initiative(), 5);
    assert!(combatant(&w, o).entered());
}

/// B22, B25 (E3): member draws come after every participant's draw, in slice
/// order, with the record's attributes; controls change one input.
#[test]
fn b25_member_draws_follow_the_participants() {
    let mut f = Fixture::standard();
    set_initiative_rule(&mut f, "1d20", &["alpha"]);
    let mut w = World::new(SEED);
    add_outsider(&mut f, 131, 141);
    add_outsider(&mut f, 132, 142);
    let o1 = f.register(&mut w, ulid(131)).expect("registers");
    let o2 = f.register(&mut w, ulid(132)).expect("registers");
    let w = with_record(&w, o1, |r| {
        r["attributes"] = json!([["alpha", 3], ["beta", 6], ["gamma", 7]]);
    });
    let (mut rng, mut seen) = (oracle(SEED), Vec::new());
    for _ in 0..4 {
        seen.push(draw(&mut rng, "1d20").total);
    }
    assert!(seen[2] != seen[3], "the fixture must tell the draws apart");

    let mut run = w.clone();
    start_with(&f, &mut run, TB, &[entrant(o1, 5), entrant(o2, 6)]).expect("starts");
    let v = ids(&run);
    assert_eq!(combatant(&run, v[0]).initiative(), seen[0] + 8 + 10);
    assert_eq!(combatant(&run, v[1]).initiative(), seen[1] + 6 + 9);
    assert_eq!(combatant(&run, o1).initiative(), seen[2] + 3 + 5);
    assert_eq!(combatant(&run, o2).initiative(), seen[3] + 8 + 6);
    assert_eq!(
        next_draw(&run),
        rng.stream(COMBAT_ROLL_STREAM).next_u32(),
        "exactly four draws"
    );

    // Control: swapping the slice swaps which member gets which draw.
    let mut swap = w.clone();
    start_with(&f, &mut swap, TB, &[entrant(o2, 6), entrant(o1, 5)]).expect("starts");
    assert_eq!(combatant(&swap, o2).initiative(), seen[2] + 8 + 6);
    assert_eq!(combatant(&swap, o1).initiative(), seen[3] + 3 + 5);

    // Control: with no rule nothing is drawn and the keys are the bases.
    let (f, mut quiet, o) = member_setup();
    start_with(&f, &mut quiet, TB, &[entrant(o, 5)]).expect("starts");
    assert_eq!(next_draw(&quiet), next_draw(&World::new(SEED)));
    assert_eq!(combatant(&quiet, o).initiative(), 5);
}

/// B22, B25: in RT the draws are stored but every timeline key is 0.
#[test]
fn b25_rt_member_stores_the_drawn_key() {
    let mut f = capable(10);
    set_initiative_rule(&mut f, "1d20", &["alpha"]);
    let mut w = World::new(SEED);
    add_outsider(&mut f, 131, 141);
    let o = f.register(&mut w, ulid(131)).expect("registers");
    start_with(&f, &mut w, RT, &[entrant(o, 5)]).expect("starts");
    let mut rng = oracle(SEED);
    let rolls: Vec<i32> = (0..3).map(|_| draw(&mut rng, "1d20").total).collect();
    assert_eq!(combatant(&w, o).initiative(), rolls[2] + 8 + 5);
    assert!(timeline_of(&w).iter().all(|(key, _)| *key == 0));
}

// ------------------------------------------------- B21: member checks

/// B21 M0: the bound counts participants plus members and runs before any
/// per-member check.
#[test]
fn m0_participants_plus_members_are_bounded() {
    let mut f = Fixture::standard();
    let mut w = World::new(SEED);
    let mut members = Vec::new();
    for n in 0..(MAX_COMBATANTS - 1) as u128 {
        add_outsider(&mut f, 1000 + n, 2000 + n);
        let e = f.register(&mut w, ulid(1000 + n)).expect("registers");
        members.push(entrant(e, n as i32 + 20));
    }
    assert_eq!(members.len(), MAX_COMBATANTS - 1);
    start_rejects(&f, &w, &members, CombatError::TooManyParticipants);
    // M0 beats M1 and M2: absent ids and one repeated id.
    let gone = stale_id(&mut w);
    start_rejects(
        &f,
        &w,
        &vec![entrant(gone, 0); MAX_COMBATANTS - 1],
        CombatError::TooManyParticipants,
    );
    // Control: one fewer member is exactly MAX_COMBATANTS combatants.
    members.pop();
    let mut ok = w.clone();
    start_with(&f, &mut ok, TB, &members).expect("the bound is inclusive");
    assert_eq!(ok.combatants().len(), MAX_COMBATANTS);
}

/// B21 M1, M2, M3, M4: one fault each, world untouched; controls start.
#[test]
fn m1_to_m4_member_faults() {
    let (f, mut w, o) = member_setup();
    let gone = stale_id(&mut w);
    let bare = w.spawn(EntityMeta {});
    start_rejects(
        &f,
        &w,
        &[entrant(gone, 1)],
        CombatError::AbsentActor { actor: gone },
    );
    start_rejects(
        &f,
        &w,
        &[entrant(o, 5), entrant(o, 6)],
        CombatError::AlreadyParticipant { entity: o },
    );
    start_rejects(
        &f,
        &w,
        &[entrant(bare, 1)],
        CombatError::NotCreature { entity: bare },
    );
    let dead = with_record(&w, o, |r| r["health"] = json!(0));
    start_rejects(
        &f,
        &dead,
        &[entrant(o, 5)],
        CombatError::DeadActor { actor: o },
    );
    // Control: the intact record starts.
    start_with(&f, &mut w, TB, &[entrant(o, 5)]).expect("starts");
}

/// B21 M5 (as S1): ruleset id, attribute order and each pool field.
#[test]
fn m5_ruleset_mismatch() {
    let edits: [Named; 7] = [
        ("ruleset id", |r| r["ruleset"] = json!(uid(999))),
        ("attribute order", |r| {
            r["attributes"] = json!([["beta", 6], ["alpha", 8], ["gamma", 7]]);
        }),
        ("attribute name", |r| {
            r["attributes"] = json!([["alpha", 8], ["beta", 6], ["delta", 7]]);
        }),
        ("pool max", |r| r["pools"][0]["max"] = json!(2)),
        ("pool id", |r| r["pools"][0]["id"] = json!(uid(999))),
        ("pool refresh", |r| {
            r["pools"][0]["refresh"] = json!({"type": "never"});
        }),
        ("pool count", |r| {
            let extra = json!({"current": 1, "id": uid(998), "max": 1,
                               "refresh": {"type": "never"}});
            r["pools"].as_array_mut().unwrap().push(extra);
        }),
    ];
    for (name, edit) in edits {
        let (f, w, o) = member_setup();
        let w = with_record(&w, o, edit);
        let mut probe = w.clone();
        assert_eq!(
            start_with(&f, &mut probe, TB, &[entrant(o, 5)]),
            Err(CombatError::RulesetMismatch { entity: o }),
            "{name}"
        );
        assert_untouched(&w, &probe);
    }
}

/// B21 M6: a member whose placement an authored participant holds, with and
/// without `respawns`; a different placement starts.
#[test]
fn m6_placement_in_use() {
    for respawns in [Some(true), None] {
        let mut f = Fixture::standard();
        f.encounter.respawns = respawns;
        let mut w = World::new(SEED);
        let held = f.register(&mut w, f.placement_a).expect("registers");
        events(&mut w);
        start_rejects(
            &f,
            &w,
            &[entrant(held, 5)],
            CombatError::PlacementInUse {
                placement: f.placement_a,
            },
        );
        // Control: an outsider placement is fine.
        add_outsider(&mut f, 131, 141);
        let o = f.register(&mut w, ulid(131)).expect("registers");
        let mut ok = w.clone();
        start_with(&f, &mut ok, TB, &[entrant(o, 5)]).expect("starts");
    }
}

fn derived_ratio(f: &mut Fixture) {
    add_derived(
        f,
        "ratio",
        binary(
            "divide",
            stat("alpha"),
            binary("subtract", stat("gamma"), lit(4)),
        ),
    );
}

/// B21 M7 (as S2): a derived stat that fails for the record's attributes.
#[test]
fn m7_invalid_stat_value() {
    let (mut f, w, o) = member_setup();
    derived_ratio(&mut f);
    let bad = with_record(&w, o, |r| {
        r["attributes"] = json!([["alpha", 8], ["beta", 6], ["gamma", 4]]);
    });
    start_rejects(
        &f,
        &bad,
        &[entrant(o, 5)],
        CombatError::InvalidStatValue {
            stat: "ratio".to_owned(),
        },
    );
    // Control: gamma 5 divides.
    let mut ok = with_record(&w, o, |r| {
        r["attributes"] = json!([["alpha", 8], ["beta", 6], ["gamma", 5]]);
    });
    start_with(&f, &mut ok, TB, &[entrant(o, 5)]).expect("starts");
}

/// B21 M8 (as S3): the record's tags plus the definition's grants.
#[test]
fn m8_too_many_tags() {
    let (mut f, w, o) = member_setup();
    attach_to_ability(&mut f, granting(701, &["zzz-new"]));
    let full = tag_names(1024);
    let packed = with_record(&w, o, |r| r["tags"] = json!(full));
    start_rejects(&f, &packed, &[entrant(o, 5)], CombatError::TooManyTags);
    // Control: 1023 tags plus one new grant is exactly the bound.
    let ok = with_record(&w, o, |r| r["tags"] = json!(tag_names(1023)));
    let mut probe = ok.clone();
    start_with(&f, &mut probe, TB, &[entrant(o, 5)]).expect("1024 distinct tags fit");
    // Control: a grant the record already holds adds nothing.
    let (mut f, w, o) = member_setup();
    attach_to_ability(&mut f, granting(701, &["tag-0000"]));
    let mut dup = with_record(&w, o, |r| r["tags"] = json!(tag_names(1024)));
    start_with(&f, &mut dup, TB, &[entrant(o, 5)]).expect("a duplicate grant fits");
}

/// B21 precedence pairs and D44a: spec checks win, M1/M2 run as a pass over
/// the whole list first, then M3-M8 member by member, S4 counts members.
#[test]
fn member_check_precedence() {
    // A spec fault beats a bad member.
    let (mut f, mut w, _o) = member_setup();
    let bare = w.spawn(EntityMeta {});
    start_rejects(
        &f,
        &w,
        &[entrant(bare, 1)],
        CombatError::NotCreature { entity: bare },
    );
    f.encounter.participants.push(EncounterParticipant {
        placement: ulid(999),
        initiative: 1,
    });
    start_rejects(
        &f,
        &w,
        &[entrant(bare, 1)],
        CombatError::MissingPlacement {
            placement: ulid(999),
        },
    );

    let (f, mut w, o) = member_setup();
    let bare = w.spawn(EntityMeta {});
    let gone = stale_id(&mut w);
    // M2 beats M3 for a bare entity listed twice.
    start_rejects(
        &f,
        &w,
        &[entrant(bare, 1), entrant(bare, 2)],
        CombatError::AlreadyParticipant { entity: bare },
    );
    // M4 beats M5.
    let dead = with_record(&w, o, |r| {
        r["health"] = json!(0);
        r["ruleset"] = json!(uid(999));
    });
    start_rejects(
        &f,
        &dead,
        &[entrant(o, 5)],
        CombatError::DeadActor { actor: o },
    );
    // D44a: an absent id later in the list is reported before an earlier
    // member's dead record.
    let dead = with_record(&w, o, |r| r["health"] = json!(0));
    start_rejects(
        &f,
        &dead,
        &[entrant(o, 5), entrant(gone, 6)],
        CombatError::AbsentActor { actor: gone },
    );
    // A repeat later in the list is reported before an earlier dead record.
    start_rejects(
        &f,
        &dead,
        &[entrant(o, 5), entrant(bare, 6), entrant(bare, 7)],
        CombatError::AlreadyParticipant { entity: bare },
    );
    // Control: with no M1/M2 fault the first member in list order wins.
    start_rejects(
        &f,
        &dead,
        &[entrant(bare, 6), entrant(o, 5)],
        CombatError::NotCreature { entity: bare },
    );
    start_rejects(
        &f,
        &dead,
        &[entrant(o, 5), entrant(bare, 6)],
        CombatError::DeadActor { actor: o },
    );
}

/// B21: S1 (an authored participant's record) wins over a bad member.
#[test]
fn authored_entry_checks_precede_member_checks() {
    let f = Fixture::standard();
    let mut w = World::new(SEED);
    let held = f.register(&mut w, f.placement_a).expect("registers");
    let gone = stale_id(&mut w);
    let w = with_record(&w, held, |r| r["ruleset"] = json!(uid(999)));
    start_rejects(
        &f,
        &w,
        &[entrant(gone, 1)],
        CombatError::RulesetMismatch { entity: held },
    );
}

/// B21, S4: members count toward "someone enters".
#[test]
fn members_count_against_no_entrants() {
    let mut f = Fixture::standard();
    let mut w = World::new(SEED);
    let ra = f.register(&mut w, f.placement_a).expect("registers");
    let rb = f.register(&mut w, f.placement_b).expect("registers");
    add_outsider(&mut f, 131, 141);
    let o = f.register(&mut w, ulid(131)).expect("registers");
    let w = with_record(&w, ra, |r| r["health"] = json!(0));
    let w = with_record(&w, rb, |r| r["health"] = json!(0));
    start_rejects(&f, &w, &[], CombatError::NoEntrants);
    let mut ok = w.clone();
    start_with(&f, &mut ok, TB, &[entrant(o, 5)]).expect("a member starts the fight");
    assert_eq!(ids(&ok), [o]);
    assert_eq!(timeline_of(&ok), [(5, o)]);
    assert_eq!(active(&ok), Some(o));
    assert_eq!(ok.creatures().len(), 2, "the skipped dead stay records");
}

// ------------------------------------------------- B23: join checks

/// B23 J1: no encounter, before the entity is even looked at.
#[test]
fn j1_no_encounter() {
    let mut w = World::new(SEED);
    let gone = stale_id(&mut w);
    let live = w.spawn(EntityMeta {});
    join_rejects(&w, entrant(live, 0), CombatError::NoEncounter);
    join_rejects(&w, entrant(gone, 0), CombatError::NoEncounter);
}

fn terminal_play() -> Play {
    let mut f = Fixture::standard();
    make_lethal(&mut f, 100);
    let mut p = Play::new(f);
    let (a, b) = (p.a, p.b);
    atk(&mut p.w, b, a).expect("the kill");
    assert_eq!(active(&p.w), None, "one alive: terminal");
    p
}

/// B23 J2: terminal TB and RT; control joins a live encounter.
#[test]
fn j2_terminal_encounter() {
    let mut p = terminal_play();
    let j = p.outsider();
    join_rejects(&p.w, entrant(j, 0), CombatError::EncounterTerminal);

    // RT with fewer than two alive: an order kills B on the next tick.
    let mut f = capable(10);
    make_lethal(&mut f, 100);
    let mut rt = Play::in_mode(f, RT);
    let (a, b) = (rt.a, rt.b);
    let j = rt.outsider();
    rt.join(j, 0).expect("two alive: joins");
    assert_eq!(atk(&mut rt.w, a, b), Ok(None));
    ticks(&mut rt.w, 1);
    assert!(combatant(&rt.w, b).dead());
    let k = rt.outsider();
    // J (alive) and A are alive: still two alive, so joining stays legal.
    rt.join(k, 0).expect("A and J are alive");
    assert_eq!(atk(&mut rt.w, j, a), Ok(None));
    ticks(&mut rt.w, 1);
    assert!(combatant(&rt.w, a).dead());
    let alive = rt.w.combatants().iter().filter(|(_, c)| !c.dead()).count();
    assert_eq!(alive, 2, "J and K");
    // Kill down to one: K orders J dead.
    assert_eq!(atk(&mut rt.w, k, j), Ok(None));
    ticks(&mut rt.w, 1);
    let l = rt.outsider();
    join_rejects(&rt.w, entrant(l, 0), CombatError::EncounterTerminal);
}

/// B23 J3 and its pair with J2: 1024 combatants cannot take a joiner, and a
/// terminal encounter reports J2 first.
#[test]
fn j3_too_many_participants() {
    let mut f = Fixture::standard();
    let mut w = World::new(SEED);
    let mut members = Vec::new();
    for n in 0..(MAX_COMBATANTS - 2) as u128 {
        add_outsider(&mut f, 1000 + n, 2000 + n);
        let e = f.register(&mut w, ulid(1000 + n)).expect("registers");
        members.push(entrant(e, n as i32 + 20));
    }
    start_with(&f, &mut w, TB, &members).expect("1024 combatants start");
    add_outsider(&mut f, 5000, 6000);
    let extra = f.register(&mut w, ulid(5000)).expect("registers");
    join_rejects(&w, entrant(extra, 0), CombatError::TooManyParticipants);

    // J2 beats J3: the same full house, terminal with one alive combatant.
    let terminal = with_json(&w, |save| {
        save["combat"]["active"] = Value::Null;
        save["timeline"] = json!([]);
        for (index, entry) in save["combatants"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            if index > 0 {
                entry[1]["health"] = json!(0);
                entry[1]["dead"] = json!(true);
            }
        }
    });
    join_rejects(&terminal, entrant(extra, 0), CombatError::EncounterTerminal);
}

/// B23 J4-J7 and the J1/J7 pairs.
#[test]
fn j4_to_j7_entity_faults() {
    let mut p = Play::new(Fixture::standard());
    let (a, b) = (p.a, p.b);
    let gone = stale_id(&mut p.w);
    let bare = p.w.spawn(EntityMeta {});
    let o = p.outsider();
    join_rejects(
        &p.w,
        entrant(gone, 1),
        CombatError::AbsentActor { actor: gone },
    );
    join_rejects(
        &p.w,
        entrant(a, 1),
        CombatError::AlreadyParticipant { entity: a },
    );
    join_rejects(
        &p.w,
        entrant(b, 1),
        CombatError::AlreadyParticipant { entity: b },
    );
    join_rejects(
        &p.w,
        entrant(bare, 1),
        CombatError::NotCreature { entity: bare },
    );
    p.edit(o, |r| r["health"] = json!(0));
    join_rejects(&p.w, entrant(o, 1), CombatError::DeadActor { actor: o });
    // J7 beats J8: a dead record of another ruleset.
    p.edit(o, |r| r["ruleset"] = json!(uid(999)));
    join_rejects(&p.w, entrant(o, 1), CombatError::DeadActor { actor: o });
    // Control: a live, matching record joins.
    let ok = p.outsider();
    p.join(ok, 1).expect("joins");
}

/// B23 J8 (as S1): the same mismatches as M5.
#[test]
fn j8_ruleset_mismatch() {
    let edits: [Named; 4] = [
        ("ruleset id", |r| r["ruleset"] = json!(uid(999))),
        ("attribute order", |r| {
            r["attributes"] = json!([["beta", 6], ["alpha", 8], ["gamma", 7]]);
        }),
        ("pool max", |r| r["pools"][0]["max"] = json!(2)),
        ("pool refresh", |r| {
            r["pools"][0]["refresh"] = json!({"type": "never"});
        }),
    ];
    for (name, edit) in edits {
        let mut p = Play::new(Fixture::standard());
        let o = p.outsider();
        p.edit(o, edit);
        let mut probe = p.w.clone();
        assert_eq!(
            join_encounter(&mut probe, &entrant(o, 1)),
            Err(CombatError::RulesetMismatch { entity: o }),
            "{name}"
        );
        assert_untouched(&p.w, &probe);
    }
}

/// B23 J9: a current combatant, dead or alive, holds the record's placement.
#[test]
fn j9_placement_in_use() {
    let mut f = Fixture::trio();
    f.encounter.respawns = Some(true);
    make_lethal(&mut f, 100);
    let mut w = World::new(SEED);
    let held = f.register(&mut w, f.placement_a).expect("registers");
    start_with(&f, &mut w, TB, &[]).expect("starts");
    let v = ids(&w);
    let (a, c) = (v[0], v[2]);
    assert!(
        w.creatures().get(held).is_some(),
        "respawns keeps the record"
    );
    join_rejects(
        &w,
        entrant(held, 1),
        CombatError::PlacementInUse {
            placement: f.placement_a,
        },
    );
    // A dead combatant still holds its placement.
    atk(&mut w, c, a).expect("C kills A");
    assert!(combatant(&w, a).dead());
    join_rejects(
        &w,
        entrant(held, 1),
        CombatError::PlacementInUse {
            placement: f.placement_a,
        },
    );
    // Control: a respawned A that leaves frees the placement.
    let mut f = Fixture::trio();
    f.encounter.respawns = Some(true);
    let mut w = World::new(SEED);
    let held = f.register(&mut w, f.placement_a).expect("registers");
    start_with(&f, &mut w, TB, &[]).expect("starts");
    let a = ids(&w)[0];
    leave_encounter(&mut w, a).expect("A leaves");
    join_encounter(&mut w, &entrant(held, 1)).expect("J9 clears");
    assert!(combatant(&w, held).entered());
}

/// B23 J10 and J11 (as S2, S3), and J9 beats J10, J10 beats J11.
#[test]
fn j10_j11_stat_and_tag_checks() {
    let mut f = Fixture::standard();
    derived_ratio(&mut f);
    attach_to_ability(&mut f, granting(701, &["zzz-new"]));
    let mut p = Play::new(f);
    let o = p.outsider();
    p.edit(o, |r| {
        r["attributes"] = json!([["alpha", 8], ["beta", 6], ["gamma", 4]]);
    });
    join_rejects(
        &p.w,
        entrant(o, 1),
        CombatError::InvalidStatValue {
            stat: "ratio".to_owned(),
        },
    );
    // J10 beats J11.
    p.edit(o, |r| r["tags"] = json!(tag_names(1024)));
    join_rejects(
        &p.w,
        entrant(o, 1),
        CombatError::InvalidStatValue {
            stat: "ratio".to_owned(),
        },
    );
    // J11 alone.
    p.edit(o, |r| {
        r["attributes"] = json!([["alpha", 8], ["beta", 6], ["gamma", 5]]);
    });
    join_rejects(&p.w, entrant(o, 1), CombatError::TooManyTags);
    // Control: 1023 tags plus the grant fit.
    p.edit(o, |r| r["tags"] = json!(tag_names(1023)));
    p.join(o, 1).expect("joins");
}

// ----------------------------------------- B24, B25, B26, B27: TB joins

/// B24, B26 (E5): round 0, default late join. A key after the head is
/// inserted at once; a key before it waits for the rollover.
#[test]
fn b26_round_zero_join() {
    let mut f = Fixture::standard();
    round_pool(&mut f);
    let mut p = Play::new(f);
    let (a, b) = (p.a, p.b);
    let late = p.zeroed();
    let early = p.zeroed();
    p.quiet();
    let before = p.w.clone();
    p.join(late, 12).expect("joins");
    assert_eq!(p.tl(), [(9, b), (10, a), (12, late)]);
    p.join(early, 5).expect("joins");
    assert_eq!(
        p.tl(),
        [(9, b), (10, a), (12, late)],
        "no entry for a passed slot"
    );
    assert!(p.pending().is_empty());
    assert!(saved(&p.w)["combat"]["pending_joins"].is_null());
    assert_eq!(active(&p.w), Some(b));
    assert_eq!(combat(&p.w).round, 0);
    assert!(events(&mut p.w).is_empty(), "no SimEvent");
    assert_eq!(p.w.interners(), before.interners());
    assert!(p.w.creatures().is_empty(), "both records were consumed");
    for j in [late, early] {
        let c = combatant(&p.w, j);
        assert!(c.entered() && !c.dead());
        assert_eq!(
            pools_of(c)
                .iter()
                .map(ResourcePool::current)
                .collect::<Vec<_>>(),
            [0, 0]
        );
    }
    assert_eq!(combatant(&p.w, early).initiative(), 5);

    // The rollover inserts `early` at its key with one RoundStart refresh.
    end(&mut p.w, b);
    end(&mut p.w, a);
    end(&mut p.w, late);
    assert_eq!(combat(&p.w).round, 1);
    assert_eq!(p.tl()[0], (5, early));
    assert_eq!(p.tl().len(), 4);
    assert_eq!(active(&p.w), Some(early));
    assert_eq!(combatant(&p.w, early).extra_pools()[0].current(), 3);
    assert_eq!(pool_of(&p.w, early), 1, "head: TurnStart refresh");
}

/// B26: the full sort key is (key, entity): an equal key sorts after the
/// head only for a higher entity.
#[test]
fn b26_equal_key_breaks_by_entity() {
    let mut f = Fixture::standard();
    let mut w = World::new(SEED);
    add_outsider(&mut f, 131, 141);
    let low = f.register(&mut w, ulid(131)).expect("registers");
    start_with(&f, &mut w, TB, &[]).expect("starts");
    add_outsider(&mut f, 132, 142);
    let high = f.register(&mut w, ulid(132)).expect("registers");
    let (a, b) = (ids(&w)[0], ids(&w)[1]);
    assert!(low < b && b < high);
    join_encounter(&mut w, &entrant(low, 9)).expect("joins");
    assert_eq!(
        timeline_of(&w),
        [(9, b), (10, a)],
        "(9, low) sorts before B"
    );
    join_encounter(&mut w, &entrant(high, 9)).expect("joins");
    assert_eq!(timeline_of(&w), [(9, b), (9, high), (10, a)]);
}

/// B26 (E6): from round 1 a joiner always waits, under both policies, and it
/// counts as alive.
#[test]
fn b26_later_round_joiner_waits() {
    for hook in [
        None,
        Some(LateJoinWire::NextRound),
        Some(LateJoinWire::AfterActive),
    ] {
        let mut f = Fixture::standard();
        set_hooks(&mut f, None, hook, None);
        let mut p = Play::new(f);
        into_round_one(&mut p);
        let (a, b) = (p.a, p.b);
        let j = p.outsider();
        p.quiet();
        p.join(j, 100).expect("joins");
        assert_eq!(p.tl(), [(9, b), (10, a)]);
        assert!(p.pending().is_empty());
        assert!(combatant(&p.w, j).entered());
        end(&mut p.w, b);
        end(&mut p.w, a);
        assert_eq!(combat(&p.w).round, 2);
        assert_eq!(
            p.tl(),
            [(9, b), (10, a), (100, j)],
            "enters at the rollover"
        );
    }
    // A early key waits as well.
    let mut p = Play::new(Fixture::standard());
    into_round_one(&mut p);
    let j = p.outsider();
    p.join(j, -50).expect("joins");
    assert_eq!(p.tl().len(), 2);

    // The waiting joiner counts as alive: the round rolls over instead of
    // ending. Control: without the joiner the same kill is terminal.
    for with_joiner in [true, false] {
        let mut f = Fixture::standard();
        make_lethal(&mut f, 100);
        let mut p = Play::new(f);
        into_round_one(&mut p);
        let (a, b) = (p.a, p.b);
        let j = p.outsider();
        if with_joiner {
            p.join(j, 100).expect("joins");
        }
        end(&mut p.w, b);
        atk(&mut p.w, a, b).expect("A kills B");
        if with_joiner {
            assert_eq!(combat(&p.w).round, 2);
            assert_eq!(active(&p.w), Some(a));
            assert_eq!(p.tl(), [(10, a), (100, j)]);
        } else {
            assert_eq!(active(&p.w), None);
        }
    }
}

/// B26, B27 (E7): `after_active` sends a passed round-0 slot to
/// `pending_joins`; the next advance schedules it before the head pick.
#[test]
fn b27_after_active_pending_flush() {
    let mut p = Play::new(after_active());
    let (a, b) = (p.a, p.b);
    let j = p.zeroed();
    p.quiet();
    p.join(j, 5).expect("joins");
    assert_eq!(p.pending(), [j]);
    assert_eq!(p.tl(), [(9, b), (10, a)], "pending joiners have no entry");
    assert_eq!(active(&p.w), Some(b));
    assert_eq!(
        saved(&p.w)["combat"]["pending_joins"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(pool_of(&p.w, j), 0);
    assert!(events(&mut p.w).is_empty());

    end(&mut p.w, b);
    assert_eq!(active(&p.w), Some(j), "the joiner acts right after B");
    assert_eq!(p.tl(), [(5, j), (10, a)]);
    assert!(p.pending().is_empty());
    assert!(saved(&p.w)["combat"]["pending_joins"].is_null());
    assert_eq!(pool_of(&p.w, j), 1, "its TurnStart refresh");
    assert_eq!(combat(&p.w).round, 0);
    end(&mut p.w, j);
    assert_eq!(active(&p.w), Some(a));
    assert_eq!(combat(&p.w).round, 0);

    // Control: under next_round the same join waits for the rollover.
    let mut p = Play::new(Fixture::standard());
    let (a, b) = (p.a, p.b);
    let j = p.zeroed();
    p.join(j, 5).expect("joins");
    assert!(p.pending().is_empty());
    end(&mut p.w, b);
    assert_eq!(active(&p.w), Some(a));
    end(&mut p.w, a);
    assert_eq!(combat(&p.w).round, 1);
    assert_eq!(active(&p.w), Some(j));
}

/// B27: pending joiners act in key order, not join order.
#[test]
fn b27_pending_joiners_act_in_key_order() {
    let mut p = Play::new(after_active());
    let (a, b) = (p.a, p.b);
    let j2 = p.outsider();
    let j1 = p.outsider();
    p.join(j2, 7).expect("joins");
    p.join(j1, 3).expect("joins");
    assert_eq!(p.pending(), [j2, j1], "join order");
    end(&mut p.w, b);
    assert_eq!(active(&p.w), Some(j1));
    assert!(p.pending().is_empty());
    end(&mut p.w, j1);
    assert_eq!(active(&p.w), Some(j2));
    end(&mut p.w, j2);
    assert_eq!(active(&p.w), Some(a));
    assert_eq!(combat(&p.w).round, 0);
}

/// B26 (E7 tail): under `after_active` a slot that has not passed is
/// inserted at once, and the result reloads.
#[test]
fn b26_after_active_inserts_a_slot_after_the_head() {
    let mut p = Play::new(after_active());
    let (a, b) = (p.a, p.b);
    let j = p.outsider();
    p.join(j, 12).expect("joins");
    assert_eq!(p.tl(), [(9, b), (10, a), (12, j)]);
    assert!(p.pending().is_empty());
    assert!(saved(&p.w)["combat"]["pending_joins"].is_null());
    let mut again = reloaded(&p.w);
    assert_eq!(state_hash(&again), state_hash(&p.w));
    end(&mut again, b);
    end(&mut p.w, b);
    assert_eq!(state_hash(&again), state_hash(&p.w));
}

/// B27 (E8): a pending joiner leaves, is killed, or is despawned.
#[test]
fn b27_pending_edges() {
    // Leaves: B's turn is untouched.
    let mut p = Play::new(after_active());
    let (a, b) = (p.a, p.b);
    let j = p.outsider();
    p.join(j, 5).unwrap();
    leave_encounter(&mut p.w, j).expect("leaves");
    assert!(p.pending().is_empty());
    assert_eq!(active(&p.w), Some(b));
    assert_eq!(p.tl(), [(9, b), (10, a)]);
    assert!(p.w.creatures().get(j).is_some(), "it becomes a record");

    // Killed: the flush drops it, and A is head.
    let mut f = after_active();
    make_lethal(&mut f, 100);
    let mut p = Play::new(f);
    let (a, b) = (p.a, p.b);
    let j = p.outsider();
    p.join(j, 5).unwrap();
    atk(&mut p.w, b, j).expect("a pending joiner is a target");
    assert!(combatant(&p.w, j).dead());
    assert!(p.pending().is_empty());
    assert_eq!(active(&p.w), Some(a));
    assert_eq!(p.tl(), [(10, a)]);

    // Killed without ending the turn: still pending and dead, a legal save;
    // the flush at B's EndTurn drops it.
    let mut f = after_active();
    make_lethal(&mut f, 100);
    edit_ability(&mut f, |ability| ability.ends_turn = false);
    let mut p = Play::new(f);
    let (a, b) = (p.a, p.b);
    let j = p.outsider();
    p.join(j, 5).unwrap();
    atk(&mut p.w, b, j).expect("kills J");
    assert_eq!(p.pending(), [j]);
    assert!(combatant(&p.w, j).dead());
    assert_eq!(state_hash(&reloaded(&p.w)), state_hash(&p.w));
    end(&mut p.w, b);
    assert_eq!(active(&p.w), Some(a));
    assert!(p.pending().is_empty());
    assert_eq!(p.tl(), [(10, a)]);

    // Despawned: removed from pending_joins, one Despawned.
    let mut p = Play::new(after_active());
    let j = p.outsider();
    p.join(j, 5).unwrap();
    p.quiet();
    assert!(p.w.despawn(j));
    assert!(p.pending().is_empty());
    assert_eq!(events(&mut p.w), [SimEvent::Despawned { entity: j }]);
    assert!(p.w.combatants().get(j).is_none());
}

/// B26: a pending joiner schedules rather than ending the encounter.
#[test]
fn b26_pending_joiner_prevents_terminal() {
    let mut f = after_active();
    make_lethal(&mut f, 100);
    let mut p = Play::new(f);
    let (a, b) = (p.a, p.b);
    let j = p.outsider();
    p.join(j, 5).unwrap();
    atk(&mut p.w, b, a).expect("B kills A");
    assert_eq!(active(&p.w), Some(j), "J and B are alive: J is scheduled");
    assert_eq!(combat(&p.w).round, 0);
    assert_eq!(p.tl(), [(5, j)]);
}

/// B25 (E11): a join makes exactly one draw on `combat.roll`, and the key
/// is roll + bonus (record values) + entrant base; no rule draws nothing.
#[test]
fn b25_join_initiative() {
    let mut f = Fixture::standard();
    set_initiative_rule(&mut f, "1d20", &["alpha", "vigor"]);
    add_derived(&mut f, "vigor", binary("add", stat("health"), lit(0)));
    let mut p = Play::new(f);
    let j = p.outsider();
    p.edit(j, |r| {
        r["attributes"] = json!([["alpha", 3], ["beta", 6], ["gamma", 7]]);
        r["health"] = json!(4);
    });
    let before = p.w.clone();
    let mut rng = before.clone().rng_mut().clone();
    let rolled = draw(&mut rng, "1d20").total;
    p.join(j, 100).expect("joins");
    assert_eq!(combatant(&p.w, j).initiative(), rolled + 3 + 4 + 100);
    assert!(
        p.tl().contains(&(rolled + 3 + 4 + 100, j)),
        "the timeline slot is the drawn key, not the base 100"
    );
    assert_eq!(
        next_draw(&p.w),
        rng.stream(COMBAT_ROLL_STREAM).next_u32(),
        "one draw"
    );
    assert_eq!(p.w.interners(), before.interners());

    // Control: no rule means the key is the base and RNG is unchanged.
    let mut p = Play::new(Fixture::standard());
    let j = p.outsider();
    let before = p.w.clone();
    p.join(j, 11).expect("joins");
    assert_eq!(combatant(&p.w, j).initiative(), 11);
    assert_eq!(p.w.rng_mut().clone(), before.clone().rng_mut().clone());
}

/// B25, B26 (E5, E11): the round-0 insert-versus-pending decision reads the
/// created entry key, never the entrant's base. A base that sorts after the
/// head with a drawn key before it must leave the joiner pending; a base
/// that sorts before the head with a drawn key after it must insert it.
#[test]
fn b25_join_key_decides_insert_versus_pending() {
    let mut f = after_active();
    set_initiative_rule(&mut f, "1d20", &["alpha"]);
    let mut p = Play::new(f);
    let (a, b) = (p.a, p.b);
    // A declared rule also rolls the participants' keys at start.
    let order = p.tl();
    let head = order[0].0;
    assert_eq!(order.len(), 2);
    assert_eq!(order[0].1, b);
    assert_eq!(order[1].1, a);
    assert_eq!(active(&p.w), Some(b));

    // Base 100 sorts after the head; the record's negative alpha pulls the
    // drawn key below it, so only the drawn key can send J to pending.
    let j = p.outsider();
    p.edit(j, |r| {
        r["attributes"] = json!([["alpha", -200], ["beta", 6], ["gamma", 7]]);
    });
    let mut rng = p.w.clone().rng_mut().clone();
    let rolled = draw(&mut rng, "1d20").total;
    let key = rolled - 200 + 100;
    assert!(key < head, "the drawn key {key} sorts before the head");
    let ordered = p.tl();
    p.join(j, 100).expect("joins");
    assert_eq!(combatant(&p.w, j).initiative(), key);
    assert_eq!(p.tl(), ordered, "no timeline entry yet");
    assert_eq!(p.pending(), [j], "the base 100 alone would have inserted");
    assert_eq!(active(&p.w), Some(b));

    // Control differing in one input: base 0 sorts before the head while the
    // positive alpha raises the drawn key above it, so K is inserted at once.
    let k = p.outsider();
    p.edit(k, |r| {
        r["attributes"] = json!([["alpha", 200], ["beta", 6], ["gamma", 7]]);
    });
    let mut rng = p.w.clone().rng_mut().clone();
    let rolled = draw(&mut rng, "1d20").total;
    let key = rolled + 200;
    assert!(key > head, "the drawn key {key} sorts after the head");
    p.join(k, 0).expect("joins");
    assert_eq!(combatant(&p.w, k).initiative(), key);
    let mut expect = ordered.clone();
    expect.push((key, k));
    assert_eq!(p.tl(), expect);
    assert_eq!(p.pending(), [j], "the control adds no pending entry");
}

// ------------------------------------------------------ B28: RT joins

/// B28 (E9): keyed at `elapsed` with no order; no pool refresh; it acts on
/// the first tick after an order; a join into one alive is terminal.
#[test]
fn b28_rt_join() {
    let mut f = capable(10);
    make_lethal(&mut f, 2);
    let mut p = Play::in_mode(f, RT);
    let b = p.b;
    ticks(&mut p.w, 3);
    assert_eq!(elapsed(&p.w), 3);
    let cold = p.zeroed();
    let warm = p.outsider();
    p.quiet();
    p.join(cold, 0).expect("joins");
    p.join(warm, 0).expect("joins");
    assert!(p.tl().contains(&(3, cold)) && p.tl().contains(&(3, warm)));
    assert_eq!(combatant(&p.w, cold).order(), None);
    assert_eq!(pool_of(&p.w, cold), 0, "entry refreshes nothing");
    assert_eq!(active(&p.w), None);
    assert!(events(&mut p.w).is_empty());
    let hp = combatant(&p.w, b).health();
    ticks(&mut p.w, 1);
    assert_eq!(
        combatant(&p.w, b).health(),
        hp,
        "no order: nothing executes"
    );
    // The unrefreshed joiner cannot pay; the full one can.
    let pool = ulid(105);
    assert_eq!(
        atk(&mut p.w, cold, b),
        Err(CombatError::InsufficientAction {
            pool,
            cost: 1,
            current: 0
        })
    );
    assert_eq!(atk(&mut p.w, warm, b), Ok(None));
    assert!(combatant(&p.w, warm).order().is_some());
    assert_eq!(combatant(&p.w, b).health(), hp, "stored, not executed");
    ticks(&mut p.w, 1);
    assert_eq!(combatant(&p.w, b).health(), hp - 2, "due on the first tick");
}

/// B25, B28: an RT joiner stores the drawn key but is scheduled at `elapsed`.
#[test]
fn b28_rt_join_stores_the_drawn_key() {
    let mut f = capable(10);
    set_initiative_rule(&mut f, "1d20", &["alpha"]);
    let mut p = Play::in_mode(f, RT);
    ticks(&mut p.w, 2);
    let j = p.outsider();
    let mut rng = p.w.clone().rng_mut().clone();
    let rolled = draw(&mut rng, "1d20").total;
    p.join(j, 4).expect("joins");
    assert_eq!(combatant(&p.w, j).initiative(), rolled + 8 + 4);
    assert!(p.tl().contains(&(2, j)));
    assert_eq!(next_draw(&p.w), rng.stream(COMBAT_ROLL_STREAM).next_u32());
}

// ------------------------------------------------ B29-B31: leave

/// B29 (E14): V1-V4 in order, each leaving the world untouched.
#[test]
fn b29_leave_checks() {
    let mut w = World::new(SEED);
    let gone = stale_id(&mut w);
    leave_rejects(&w, gone, CombatError::NoEncounter);

    let mut f = Fixture::trio();
    make_lethal(&mut f, 100);
    let mut p = Play::new(f);
    let (a, b, c) = (p.a, p.b, ids(&p.w)[2]);
    let bare = p.w.spawn(EntityMeta {});
    let gone = stale_id(&mut p.w);
    let rec = p.outsider();
    leave_rejects(&p.w, bare, CombatError::NotParticipant { entity: bare });
    leave_rejects(&p.w, rec, CombatError::NotParticipant { entity: rec });
    leave_rejects(&p.w, gone, CombatError::AbsentActor { actor: gone });
    atk(&mut p.w, c, a).expect("C kills A");
    leave_rejects(&p.w, a, CombatError::DeadActor { actor: a });
    leave_encounter(&mut p.w, b).expect("an alive combatant leaves");
}

fn trio_play() -> Play {
    Play::new(Fixture::trio())
}

/// B30 (E14): a non-head combatant leaves; the turn is untouched.
#[test]
fn b30_non_head_leave() {
    let mut p = trio_play();
    let (a, b, c) = (p.a, p.b, ids(&p.w)[2]);
    let snapshot = Transform {
        position: [1.0, 2.0, 3.0],
        velocity: [0.0, 0.0, 0.0],
    };
    p.w.transforms_mut().insert(a, snapshot);
    p.quiet();
    let before = p.w.clone();
    leave_encounter(&mut p.w, a).expect("leaves");
    assert_eq!(active(&p.w), Some(c));
    assert_eq!(combat(&p.w).round, 0);
    assert_eq!(p.tl(), [(8, c), (9, b)]);
    assert!(p.w.combatants().get(a).is_none());
    assert!(p.w.contains(a), "the entity stays live");
    assert_eq!(
        p.w.transforms().get(a),
        Some(&snapshot),
        "keeps its Transform"
    );
    assert!(events(&mut p.w).is_empty(), "no Died, no Despawned");
    assert_eq!(pool_of(&p.w, c), pool_of(&before, c));
    assert_eq!(p.w.interners(), before.interners());
}

/// B30 (E14): the head leaves; the next head gets `TurnStart`; the result
/// equals despawning the same combatant (control).
#[test]
fn b30_head_leave_matches_despawn() {
    let mut p = trio_play();
    let (a, b, c) = (p.a, p.b, ids(&p.w)[2]);
    p.w = with_json(&p.w, |save| {
        edit_combatant(save, b, |entry| entry["action_pool"]["current"] = json!(0));
    });
    p.quiet();
    let mut despawned = p.w.clone();
    leave_encounter(&mut p.w, c).expect("leaves");
    assert_eq!(active(&p.w), Some(b));
    assert_eq!(pool_of(&p.w, b), 1, "the new head's TurnStart refresh");
    assert_eq!(p.tl(), [(9, b), (10, a)]);
    assert!(events(&mut p.w).is_empty());
    assert!(p.w.contains(c) && p.w.creatures().get(c).is_some());

    assert!(despawned.despawn(c));
    let (x, y) = (saved(&p.w), saved(&despawned));
    for key in ["combat", "combatants", "timeline"] {
        assert_eq!(x[key], y[key], "{key} equal to despawn");
    }
    assert!(!despawned.contains(c));
}

/// B30, B31 (E14): the last combatant leaves; `combat` is `None` and the
/// interners, RNG and tick are retained; nothing is enqueued.
#[test]
fn b30_last_combatant_leaves() {
    let mut p = Play::new(Fixture::standard());
    let (a, b) = (p.a, p.b);
    leave_encounter(&mut p.w, a).expect("A leaves");
    assert_eq!(active(&p.w), Some(b), "the turn is untouched");
    p.quiet();
    let before = p.w.clone();
    leave_encounter(&mut p.w, b).expect("B leaves");
    assert!(p.w.combat().is_none());
    assert_eq!(p.w.interners(), before.interners());
    assert_eq!(p.w.tick(), before.tick());
    assert_eq!(next_draw(&p.w), next_draw(&before));
    assert!(events(&mut p.w).is_empty());
    assert_eq!(p.w.creatures().len(), 2, "both became records");
    assert!(p.w.contains(a) && p.w.contains(b));
}

/// B30: when the head leaves the pending flush runs first.
#[test]
fn b30_head_leave_flushes_pending() {
    let mut p = Play::new(after_active());
    let (a, b) = (p.a, p.b);
    let j = p.zeroed();
    p.join(j, 5).unwrap();
    leave_encounter(&mut p.w, b).expect("the head leaves");
    assert_eq!(active(&p.w), Some(j));
    assert_eq!(p.tl(), [(5, j), (10, a)]);
    assert!(p.pending().is_empty());
    assert_eq!(pool_of(&p.w, j), 1);
}

/// B30, B31 (E14): in RT a leave that drops the alive count to one ends the
/// encounter as a despawn does, and no `Died`/`Despawned` is enqueued.
#[test]
fn b30_rt_leave_to_one_alive_ends_the_encounter() {
    let mut p = Play::in_mode(capable(10), RT);
    let a = p.a;
    p.quiet();
    let mut despawned = p.w.clone();
    leave_encounter(&mut p.w, a).expect("leaves");
    assert!(events(&mut p.w).is_empty());
    assert!(despawned.despawn(a));
    let (x, y) = (saved(&p.w), saved(&despawned));
    for key in ["combat", "combatants", "timeline"] {
        assert_eq!(x[key], y[key], "{key} equal to despawn");
    }
    let j = p.outsider();
    join_rejects(&p.w, entrant(j, 0), CombatError::EncounterTerminal);
}

/// B30: another combatant's standing order against the leaver fails when due.
#[test]
fn b30_orders_against_a_leaver_end() {
    let mut f = capable_trio(10);
    make_lethal(&mut f, 2);
    let mut p = Play::in_mode(f, RT);
    let (a, c) = (p.a, ids(&p.w)[2]);
    assert_eq!(atk(&mut p.w, a, c), Ok(None));
    leave_encounter(&mut p.w, c).expect("C leaves");
    ticks(&mut p.w, 1);
    assert_eq!(combatant(&p.w, a).order(), None, "the order ended");
    assert_eq!(p.w.creatures().get(c).expect("C's record").health(), 10);
}

/// B31: death keeps a dead combatant and enqueues `Died`; leave enqueues
/// nothing.
#[test]
fn b31_leave_vs_death() {
    let mut f = Fixture::trio();
    make_lethal(&mut f, 100);
    let mut p = Play::new(f);
    let (a, b, c) = (p.a, p.b, ids(&p.w)[2]);
    p.quiet();
    atk(&mut p.w, c, a).expect("C kills A");
    assert_eq!(
        events(&mut p.w),
        [SimEvent::Died { entity: a }],
        "death enqueues Died"
    );
    assert!(combatant(&p.w, a).dead(), "the dead combatant is kept");
    assert!(p.w.creatures().get(a).is_none(), "a record only at release");
    assert!(!p.tl().iter().any(|(_, id)| *id == a));
    leave_encounter(&mut p.w, b).expect("B leaves");
    assert!(events(&mut p.w).is_empty());
    assert!(p.w.combatants().get(b).is_none());
    assert!(p.w.creatures().get(b).is_some());
}

/// B30, B16 (E12): write-back at leave under the default and `respawns`.
#[test]
fn b30_leave_write_back() {
    // Default: a spawned combatant becomes a record.
    let mut p = Play::new(Fixture::standard());
    let a = p.a;
    let held = combatant(&p.w, a).clone();
    leave_encounter(&mut p.w, a).expect("leaves");
    let r = p.w.creatures().get(a).expect("a record");
    assert_eq!(r.placement(), held.placement());
    assert_eq!(r.ruleset(), ulid(102));
    assert_eq!((r.health(), r.max_health()), (10, 10));
    assert_eq!(r.attributes(), held.attributes());
    assert_eq!(r.pools(), pools_of(&held));
    assert!(r.suspended().is_empty());

    // respawns: a spawned combatant stays a bare live entity with its
    // Transform, while an entered member becomes a record.
    let mut f = Fixture::standard();
    f.encounter.respawns = Some(true);
    add_outsider(&mut f, 131, 141);
    let mut w = World::new(SEED);
    let o = f.register(&mut w, ulid(131)).expect("registers");
    start_with(&f, &mut w, TB, &[entrant(o, 5)]).expect("starts");
    let a = ids(&w)[0];
    let spot = Transform {
        position: [4.0, 5.0, 6.0],
        velocity: [0.0, 0.0, 0.0],
    };
    w.transforms_mut().insert(a, spot);
    leave_encounter(&mut w, a).expect("A leaves");
    assert!(w.contains(a) && w.creatures().get(a).is_none());
    assert_eq!(w.transforms().get(a), Some(&spot));
    leave_encounter(&mut w, o).expect("O leaves");
    assert!(
        w.creatures().get(o).is_some(),
        "an entered member is written back"
    );
}

/// B30 (E12 control): a leave and a rejoin count the suspended rounds from
/// the join round.
#[test]
fn b30_leave_and_rejoin_resumes_effects() {
    let (mut f, mut w, o) = member_setup();
    with_effect(&mut f);
    let w2 = with_record(&w, o, |r| {
        r["suspended"] = json!([{"effect": uid(601), "remaining_rounds": 3}]);
    });
    w = w2;
    start_with(&f, &mut w, TB, &[entrant(o, 5)]).expect("starts");
    let v = ids(&w);
    let (a, b) = (v[0], v[1]);
    assert_eq!(combatant(&w, o).attached().len(), 1);
    assert_eq!(combatant(&w, o).attached()[0].expires_round, 3);
    for actor in [o, b, a] {
        end(&mut w, actor);
    }
    assert_eq!(combat(&w).round, 1);
    let at_leave = combatant(&w, o).clone();
    leave_encounter(&mut w, o).expect("leaves at round 1");
    let r = w.creatures().get(o).expect("a record");
    assert_eq!(r.suspended().len(), 1);
    assert_eq!(r.suspended()[0].remaining_rounds, 2);
    assert_eq!(r.suspended()[0].effect, ulid(601));
    end(&mut w, b);
    end(&mut w, a);
    assert_eq!(combat(&w).round, 2);
    join_encounter(&mut w, &entrant(o, 5)).expect("rejoins at round 2");
    let mut expected = serde_json::to_value(&at_leave).unwrap();
    expected["attached"] = json!([{"effect": uid(601), "expires_round": 4}]);
    assert_eq!(serde_json::to_value(combatant(&w, o)).unwrap(), expected);

    // Control: an effect the definition does not list is dropped on entry.
    let (f, w, o) = member_setup();
    let mut w = with_record(&w, o, |r| {
        r["suspended"] = json!([{"effect": uid(699), "remaining_rounds": 3}]);
    });
    start_with(&f, &mut w, TB, &[entrant(o, 5)]).expect("starts");
    assert!(combatant(&w, o).attached().is_empty());
}

/// B30, B16 (E13): effects at leave under each `effects_on_exit` value, and
/// the lapse boundary.
#[test]
fn b30_effects_at_leave() {
    for (hook, expected) in [
        (None, 2),
        (Some(EffectsOnExitWire::Suspend), 2),
        (Some(EffectsOnExitWire::StripAtRelease), 2),
        (Some(EffectsOnExitWire::Strip), 0),
    ] {
        let mut f = Fixture::standard();
        with_effect(&mut f);
        set_hooks(&mut f, None, None, hook);
        let mut p = Play::new(f);
        into_round_one(&mut p);
        let a = p.a;
        p.w = with_json(&p.w, |save| {
            edit_combatant(save, a, |c| {
                c["attached"] = json!([{"effect": uid(601), "expires_round": 3}]);
            });
        });
        leave_encounter(&mut p.w, a).expect("leaves");
        let suspended = p.w.creatures().get(a).expect("a record").suspended();
        assert_eq!(suspended.len(), usize::from(expected > 0), "{hook:?}");
        if expected > 0 {
            assert_eq!(suspended[0].effect, ulid(601));
            assert_eq!(suspended[0].remaining_rounds, expected, "{hook:?}");
        }
    }
    // Boundary: an attachment that lapses one round after the leave round
    // keeps exactly one round. (The loader refuses a lapsed attachment.)
    let mut f = Fixture::standard();
    with_effect(&mut f);
    let mut p = Play::new(f);
    into_round_one(&mut p);
    let a = p.a;
    p.w = with_json(&p.w, |save| {
        edit_combatant(save, a, |c| {
            c["attached"] = json!([{"effect": uid(601), "expires_round": 2}]);
        });
    });
    leave_encounter(&mut p.w, a).expect("leaves");
    let suspended = p.w.creatures().get(a).unwrap().suspended();
    assert_eq!(suspended.len(), 1);
    assert_eq!(suspended[0].remaining_rounds, 1);
}

/// B31, B16, B17 (E16): release writes back entered combatants and the dead,
/// under `respawns` and under `registered`; the default writes back all.
#[test]
fn b31_release_after_entry() {
    for mode in ["respawns", "registered", "default"] {
        let mut f = Fixture::standard();
        make_lethal(&mut f, 100);
        match mode {
            "respawns" => f.encounter.respawns = Some(true),
            "registered" => {
                set_hooks(
                    &mut f,
                    Some(CreaturePersistenceWire::Registered),
                    None,
                    None,
                );
            }
            _ => {}
        }
        let mut w = World::new(SEED);
        add_outsider(&mut f, 131, 141);
        add_outsider(&mut f, 132, 142);
        let o = f.register(&mut w, ulid(131)).expect("registers");
        let j = f.register(&mut w, ulid(132)).expect("registers");
        start_with(&f, &mut w, TB, &[entrant(o, 5)]).expect("starts");
        let (a, b) = (ids(&w)[0], ids(&w)[1]);
        join_encounter(&mut w, &entrant(j, 6)).expect("joins at round 0");
        end(&mut w, o);
        end(&mut w, j);
        end(&mut w, b);
        atk(&mut w, a, j).expect("A kills J");
        assert!(combatant(&w, j).dead());
        let held_o = combatant(&w, o).clone();
        end_encounter(&mut w).expect("releases");
        assert!(w.combat().is_none());
        let r = w.creatures().get(o).expect("O's record");
        assert_eq!(r.placement(), held_o.placement());
        assert_eq!(r.health(), held_o.health());
        assert_eq!(r.max_health(), held_o.max_health());
        assert_eq!(r.attributes(), held_o.attributes());
        assert_eq!(r.pools(), pools_of(&held_o));
        assert_eq!(r.ruleset(), ulid(102));
        let rj = w.creatures().get(j).expect("J's record");
        assert!(rj.dead() && rj.health() == 0);
        let written = |id: EntityId| w.creatures().get(id).is_some();
        assert!(w.contains(a) && w.contains(b));
        if mode == "default" {
            assert!(written(a) && written(b), "{mode}");
            assert_eq!(w.creatures().len(), 4);
        } else {
            assert!(!written(a) && !written(b), "{mode}");
            assert_eq!(w.creatures().len(), 2, "{mode}");
        }
    }
}

// ---------------------------------------------------- B32: despawn

/// B32: despawn is unchanged for an empty `pending_joins`: exactly one
/// `Despawned`, no `Died`, the same settlement, the record removed.
#[test]
fn b32_despawn_unchanged() {
    // Non-head, with a transform: everything is stripped.
    let mut p = trio_play();
    let (a, b, c) = (p.a, p.b, ids(&p.w)[2]);
    p.w.transforms_mut().insert(
        a,
        Transform {
            position: [1.0; 3],
            velocity: [0.0; 3],
        },
    );
    p.quiet();
    assert!(p.w.despawn(a));
    assert_eq!(events(&mut p.w), [SimEvent::Despawned { entity: a }]);
    assert_eq!(active(&p.w), Some(c));
    assert_eq!(p.tl(), [(8, c), (9, b)]);
    assert!(p.w.transforms().get(a).is_none() && p.w.creatures().get(a).is_none());
    assert!(!p.w.despawn(a), "a dead id returns false");
    assert!(events(&mut p.w).is_empty(), "and enqueues nothing");

    // Head: the next actor starts a turn.
    let mut p = trio_play();
    let (b, c) = (p.b, ids(&p.w)[2]);
    p.w = with_json(&p.w, |save| {
        edit_combatant(save, b, |e| e["action_pool"]["current"] = json!(0));
    });
    p.quiet();
    assert!(p.w.despawn(c));
    assert_eq!(active(&p.w), Some(b));
    assert_eq!(pool_of(&p.w, b), 1);
    assert_eq!(events(&mut p.w), [SimEvent::Despawned { entity: c }]);

    // Last combatant: the encounter is released and state retained.
    let mut p = Play::new(Fixture::standard());
    let (a, b) = (p.a, p.b);
    assert!(p.w.despawn(a));
    assert_eq!(active(&p.w), Some(b), "the turn is untouched");
    let before = p.w.clone();
    assert!(p.w.despawn(b));
    assert!(p.w.combat().is_none());
    assert_eq!(p.w.interners(), before.interners());
    assert_eq!(p.w.tick(), before.tick());

    // A record holder that is not a combatant: the record goes too.
    let mut p = Play::new(Fixture::standard());
    let o = p.outsider();
    assert!(p.w.despawn(o));
    assert!(p.w.creatures().get(o).is_none());
}

/// B32: a despawned combatant leaves no record, unlike a leaver.
#[test]
fn b32_despawn_writes_no_record() {
    let mut p = Play::new(Fixture::standard());
    let a = p.a;
    assert!(p.w.despawn(a));
    assert!(p.w.creatures().get(a).is_none() && !p.w.contains(a));
}

// -------------------------------------------------------- B33: load

fn pending_play() -> (Play, EntityId) {
    let mut p = Play::new(after_active());
    let j = p.zeroed();
    p.join(j, 5).unwrap();
    assert_eq!(p.pending(), [j]);
    (p, j)
}

/// B33: waiting and pending saves round-trip and continue identically.
#[test]
fn b33_round_trips() {
    let (mut p, j) = pending_play();
    let mut again = reloaded(&p.w);
    assert_eq!(again, p.w);
    let b = p.b;
    end(&mut p.w, b);
    end(&mut again, b);
    assert_eq!(state_hash(&p.w), state_hash(&again));
    assert_eq!(active(&again), Some(j));

    let mut p = Play::new(Fixture::standard());
    into_round_one(&mut p);
    let j = p.outsider();
    p.join(j, 100).unwrap();
    let mut again = reloaded(&p.w);
    assert_eq!(state_hash(&again), state_hash(&p.w));
    let (a, b) = (p.a, p.b);
    for w in [&mut p.w, &mut again] {
        end(w, b);
        end(w, a);
    }
    assert_eq!(state_hash(&p.w), state_hash(&again));
    assert_eq!(timeline_of(&again).last(), Some(&(100, j)));
}

/// B33 Q1-Q5, each by raw JSON with its exact message, and their order.
#[test]
fn b33_pending_joins_load_checks() {
    let (p, j) = pending_play();
    let (a, b) = (p.a, p.b);
    let mut q = p.w.clone();
    let x = {
        let mut f = after_active();
        add_outsider(&mut f, 900, 901);
        f.register(&mut q, ulid(900)).expect("registers")
    };
    let fail = |edit: &dyn Fn(&mut Value)| -> String {
        try_with_json(&q, |save| edit(save)).expect_err("the save must not load")
    };
    let id = |e: EntityId| serde_json::to_value(e).unwrap();

    let q1 = "combat pending joins outside a first turn-based round";
    let q2 = "combat pending joins repeat an entity";
    let q3 = "combat pending join names a non-combatant";
    let q4 = "combat pending join is scheduled";
    let q5 = "combat pending join sorts after the active head";
    assert!(fail(&|s| s["combat"]["round"] = json!(1)).contains(q1));
    assert!(fail(&|s| s["combat"]["pending_joins"] = json!([id(j), id(j)])).contains(q2));
    assert!(fail(&|s| s["combat"]["pending_joins"] = json!([id(x)])).contains(q3));
    assert!(fail(&|s| s["combat"]["pending_joins"] = json!([id(a)])).contains(q4));
    assert!(fail(&|s| edit_combatant(s, j, |c| c["initiative"] = json!(20))).contains(q5));
    // An equal key with a higher entity is not below the head either.
    assert!(fail(&|s| edit_combatant(s, j, |c| c["initiative"] = json!(9))).contains(q5));
    // Order: Q1 before Q2, Q2 before Q3, and Q4 before Q5 (A also sorts
    // after the head).
    assert!(fail(&|s| {
        s["combat"]["round"] = json!(1);
        s["combat"]["pending_joins"] = json!([id(j), id(j)]);
    })
    .contains(q1));
    assert!(fail(&|s| s["combat"]["pending_joins"] = json!([id(x), id(x)])).contains(q2));
    assert!(fail(&|s| s["combat"]["pending_joins"] = json!([id(b)])).contains(q4));
    // Controls: the unedited save loads, and so does a dead pending joiner.
    assert!(try_with_json(&q, |_| {}).is_ok());
    assert!(try_with_json(&p.w, |s| {
        edit_combatant(s, j, |c| {
            c["health"] = json!(0);
            c["dead"] = json!(true);
        });
    })
    .is_ok());
}

/// B33: an equal key with a lower entity sorts before the head, so a pending
/// joiner with it is a legal save.
#[test]
fn b33_equal_key_lower_entity_loads() {
    let mut f = after_active();
    let mut w = World::new(SEED);
    add_outsider(&mut f, 131, 141);
    let low = f.register(&mut w, ulid(131)).expect("registers");
    start_with(&f, &mut w, TB, &[]).expect("starts");
    join_encounter(&mut w, &entrant(low, 9)).expect("joins");
    assert_eq!(combat(&w).pending_joins, [low]);
    let again = reloaded(&w);
    assert_eq!(state_hash(&again), state_hash(&w));
}

/// B33 (D45a): a terminal turn-based save never holds pending joins and
/// round-trips, including with a joiner that was waiting or pending.
#[test]
fn b33_terminal_save_round_trips() {
    // A pending joiner killed with the last other combatant: terminal.
    let mut f = after_active();
    make_lethal(&mut f, 100);
    edit_ability(&mut f, |ability| ability.ends_turn = false);
    f.ruleset.pools[0].max = 2;
    let mut p = Play::new(f);
    let (a, b) = (p.a, p.b);
    let j = p.outsider();
    p.join(j, 5).unwrap();
    atk(&mut p.w, b, a).unwrap();
    assert_eq!(p.pending(), [j], "A is dead, J is still pending");
    atk(&mut p.w, b, j).unwrap();
    assert_eq!(active(&p.w), None, "terminal");
    assert!(p.pending().is_empty());
    assert!(saved(&p.w)["combat"]["pending_joins"].is_null());
    let again = reloaded(&p.w);
    assert_eq!(state_hash(&again), state_hash(&p.w));

    // A waiting joiner killed in round 1 with A despawned: terminal.
    let mut f = Fixture::standard();
    make_lethal(&mut f, 100);
    let mut p = Play::new(f);
    into_round_one(&mut p);
    let (a, b) = (p.a, p.b);
    let j = p.outsider();
    p.join(j, 100).unwrap();
    assert!(p.w.despawn(a));
    atk(&mut p.w, b, j).unwrap();
    assert_eq!(active(&p.w), None);
    assert_eq!(state_hash(&reloaded(&p.w)), state_hash(&p.w));
    let k = p.outsider();
    join_rejects(&p.w, entrant(k, 0), CombatError::EncounterTerminal);
}

// -------------------------------------------------- B34: determinism

#[derive(Clone, Copy, Debug)]
enum Step {
    Join(usize, i32),
    Leave(usize),
    Play,
    UntilRound(u32),
    Tick,
    Order(usize, usize),
    Release,
}

/// Builds the scripted world; `cast` is `[a, b, o, p, q, r, s]`.
fn scripted(rt: bool, seed: u64) -> (World, Vec<EntityId>) {
    let mut f = if rt { capable(10) } else { after_active() };
    if rt {
        make_lethal(&mut f, 1);
    }
    let mut w = World::new(seed);
    let mut outsiders = Vec::new();
    for n in 0..5u128 {
        add_outsider(&mut f, 400 + n, 500 + n);
        outsiders.push(f.register(&mut w, ulid(400 + n)).expect("registers"));
    }
    let mode = if rt { RT } else { TB };
    start_with(&f, &mut w, mode, &[entrant(outsiders[0], 5)]).expect("starts");
    let v = ids(&w);
    let mut cast = vec![v[0], v[1]];
    cast.extend(&outsiders);
    (w, cast)
}

fn apply(world: &mut World, cast: &[EntityId], step: Step) {
    match step {
        Step::Join(who, key) => {
            let _ = join_encounter(world, &entrant(cast[who], key));
        }
        Step::Leave(who) => {
            let _ = leave_encounter(world, cast[who]);
        }
        Step::Order(actor, target) => {
            let _ = atk(world, cast[actor], cast[target]);
        }
        Step::Tick => tick(world),
        Step::Release => {
            let _ = end_encounter(world);
        }
        Step::Play => play_once(world),
        Step::UntilRound(round) => {
            for _ in 0..40 {
                if world.combat().is_none_or(|c| c.round >= round) {
                    break;
                }
                match active(world) {
                    Some(actor) => end(world, actor),
                    None => tick(world),
                }
            }
        }
    }
}

fn play_once(world: &mut World) {
    let Some(actor) = world.combat().and_then(|c| c.active) else {
        tick(world);
        return;
    };
    let target = ids(world)
        .into_iter()
        .find(|t| *t != actor && !combatant(world, *t).dead());
    if !target.is_some_and(|t| atk(world, actor, t).is_ok()) {
        end(world, actor);
    }
}

fn run(script: &[Step], rt: bool, seed: u64, reload_at: Option<usize>) -> Vec<[u8; 32]> {
    let (mut w, cast) = scripted(rt, seed);
    let mut out = vec![state_hash(&w)];
    for (index, step) in script.iter().enumerate() {
        apply(&mut w, &cast, *step);
        if reload_at == Some(index) {
            w = reloaded(&w);
        }
        out.push(state_hash(&w));
    }
    out
}

fn world_after(script: &[Step], rt: bool, count: usize) -> (World, Vec<EntityId>) {
    let (mut w, cast) = scripted(rt, SEED);
    for step in &script[..count] {
        apply(&mut w, &cast, *step);
    }
    (w, cast)
}

fn tb_script() -> Vec<Step> {
    let mut s = vec![
        Step::Join(3, 12), // after the head: inserted
        Step::Join(4, 3),  // before the head: pending
        Step::Join(6, 11),
        Step::Play,
        Step::Play,
        Step::Leave(6),
        Step::Play,
        Step::Play,
        Step::UntilRound(1),
        Step::Join(5, 40), // round 1: waits
        Step::Leave(3),
    ];
    s.extend([Step::Play; 30]);
    s.push(Step::Release);
    s
}

fn rt_script() -> Vec<Step> {
    let mut s = vec![
        Step::Tick,
        Step::Tick,
        Step::Join(3, 0),
        Step::Order(3, 0),
        Step::Tick,
        Step::Join(4, 0),
        Step::Order(1, 4),
        Step::Tick,
        Step::Tick,
        Step::Leave(4),
        Step::Order(0, 1),
    ];
    s.extend([Step::Tick; 30]);
    s.push(Step::Release);
    s
}

/// B34 (E15): two runs of the script give equal per-step hashes, a different
/// seed does not, and the script really exercises all three join schedules.
#[test]
fn b34_two_runs_agree() {
    for (rt, script) in [(false, tb_script()), (true, rt_script())] {
        let first = run(&script, rt, SEED, None);
        assert_eq!(first, run(&script, rt, SEED, None), "rt={rt}");
        assert_eq!(first.len(), script.len() + 1);
        assert_ne!(first, run(&script, rt, SEED + 1, None), "rt={rt}");
        let distinct: std::collections::BTreeSet<_> = first.iter().collect();
        assert!(distinct.len() > script.len() / 2, "the hashes move");
    }
    // The TB script covers an immediate, a pending and a waiting join.
    let script = tb_script();
    let (w, cast) = world_after(&script, false, 2);
    assert_eq!(timeline_of(&w).iter().filter(|(k, _)| *k == 12).count(), 1);
    assert_eq!(combat(&w).pending_joins, [cast[4]]);
    let (w, cast) = world_after(&script, false, 10);
    let waiter = cast[5];
    assert_eq!(combat(&w).round, 1);
    assert!(combatant(&w, waiter).entered());
    assert!(!timeline_of(&w).iter().any(|(_, id)| *id == waiter));
    assert!(!combat(&w).pending_joins.contains(&waiter));
}

/// B34: a save and reload while a joiner is pending, and while one waits,
/// continues with equal hashes.
#[test]
fn b34_reload_while_pending_and_waiting() {
    let script = tb_script();
    let straight = run(&script, false, SEED, None);
    // After step 1 the pending joiner has not yet been scheduled.
    let (w, cast) = world_after(&script, false, 2);
    assert_eq!(combat(&w).pending_joins, [cast[4]]);
    assert_eq!(run(&script, false, SEED, Some(1)), straight, "pending");
    // After step 9 the round-1 joiner waits.
    let (w, cast) = world_after(&script, false, 10);
    assert!(!timeline_of(&w).iter().any(|(_, id)| *id == cast[5]));
    assert_eq!(run(&script, false, SEED, Some(9)), straight, "waiting");
    // And at every other point.
    for at in 0..script.len() {
        assert_eq!(run(&script, false, SEED, Some(at)), straight, "at {at}");
    }
    let rt = rt_script();
    let straight = run(&rt, true, SEED, None);
    for at in 0..rt.len() {
        assert_eq!(run(&rt, true, SEED, Some(at)), straight, "rt at {at}");
    }
}

// ------------------------------------------------ mutant-review fixes

/// A respawning fixture (so a registered record of an authored placement is
/// a legal member or joiner), with a derived `ratio` and a granting effect.
fn clash_fixture() -> (Fixture, World, EntityId) {
    let mut f = Fixture::standard();
    f.encounter.respawns = Some(true);
    derived_ratio(&mut f);
    attach_to_ability(&mut f, granting(701, &["zzz-new"]));
    let mut w = World::new(SEED);
    let held = f.register(&mut w, f.placement_a).expect("registers");
    events(&mut w);
    (f, w, held)
}

const GAMMA_FOUR: fn(&mut Value) =
    |r| r["attributes"] = json!([["alpha", 8], ["beta", 6], ["gamma", 4]]);

/// B21: each adjacent pair of M5, M6, M7, M8: a member failing both reports
/// the earlier check.
#[test]
fn m5_to_m8_adjacent_precedence() {
    let (f, w, held) = clash_fixture();
    let in_use = CombatError::PlacementInUse {
        placement: f.placement_a,
    };
    // Control: the clash alone is M6.
    start_rejects(&f, &w, &[entrant(held, 5)], in_use.clone());
    // M5 over M6.
    let both = with_record(&w, held, |r| r["ruleset"] = json!(uid(999)));
    start_rejects(
        &f,
        &both,
        &[entrant(held, 5)],
        CombatError::RulesetMismatch { entity: held },
    );
    // M6 over M7, and M6 over M8.
    let both = with_record(&w, held, GAMMA_FOUR);
    start_rejects(&f, &both, &[entrant(held, 5)], in_use.clone());
    let both = with_record(&w, held, |r| r["tags"] = json!(tag_names(1024)));
    start_rejects(&f, &both, &[entrant(held, 5)], in_use);
    // M7 over M8, on an outsider.
    let (mut f, mut w, _) = clash_fixture();
    add_outsider(&mut f, 131, 141);
    let o = f.register(&mut w, ulid(131)).expect("registers");
    let both = with_record(&w, o, |r| {
        GAMMA_FOUR(r);
        r["tags"] = json!(tag_names(1024));
    });
    start_rejects(
        &f,
        &both,
        &[entrant(o, 5)],
        CombatError::InvalidStatValue {
            stat: "ratio".to_owned(),
        },
    );
    // Control: M8 alone.
    let only = with_record(&w, o, |r| r["tags"] = json!(tag_names(1024)));
    start_rejects(&f, &only, &[entrant(o, 5)], CombatError::TooManyTags);
}

/// B23: J8 over J9, and J9 over J10.
#[test]
fn j8_j9_j10_precedence() {
    let (f, mut w, held) = clash_fixture();
    start_with(&f, &mut w, TB, &[]).expect("starts");
    let in_use = CombatError::PlacementInUse {
        placement: f.placement_a,
    };
    join_rejects(&w, entrant(held, 1), in_use.clone());
    let both = with_record(&w, held, |r| r["ruleset"] = json!(uid(999)));
    join_rejects(
        &both,
        entrant(held, 1),
        CombatError::RulesetMismatch { entity: held },
    );
    let both = with_record(&w, held, GAMMA_FOUR);
    join_rejects(&both, entrant(held, 1), in_use);
}

/// B23 J3: the bound is exclusive of the joiner's own slot: a join into 1023
/// combatants succeeds, and the next one is refused.
#[test]
fn j3_join_at_1023_succeeds() {
    let mut f = Fixture::standard();
    let mut w = World::new(SEED);
    let mut members = Vec::new();
    for n in 0..(MAX_COMBATANTS - 3) as u128 {
        add_outsider(&mut f, 1000 + n, 2000 + n);
        let e = f.register(&mut w, ulid(1000 + n)).expect("registers");
        members.push(entrant(e, n as i32 + 20));
    }
    start_with(&f, &mut w, TB, &members).expect("1023 combatants start");
    assert_eq!(w.combatants().len(), MAX_COMBATANTS - 1);
    add_outsider(&mut f, 5000, 6000);
    add_outsider(&mut f, 5001, 6001);
    let x = f.register(&mut w, ulid(5000)).expect("registers");
    let y = f.register(&mut w, ulid(5001)).expect("registers");
    join_encounter(&mut w, &entrant(x, 40)).expect("the 1024th combatant joins");
    assert_eq!(w.combatants().len(), MAX_COMBATANTS);
    join_rejects(&w, entrant(y, 41), CombatError::TooManyParticipants);
}

/// B30: the leave round is captured before the removal. The head leaves, the
/// round rolls over, and the suspended rounds count from the old round.
#[test]
fn b30_leave_captures_the_round_before_removal() {
    let mut f = Fixture::trio();
    with_effect(&mut f);
    let mut p = Play::new(f);
    let (a, b, c) = (p.a, p.b, ids(&p.w)[2]);
    p.w = with_json(&p.w, |save| {
        edit_combatant(save, a, |e| {
            e["attached"] = json!([{"effect": uid(601), "expires_round": 3}]);
        });
    });
    end(&mut p.w, c);
    end(&mut p.w, b);
    assert_eq!((active(&p.w), combat(&p.w).round), (Some(a), 0));
    leave_encounter(&mut p.w, a).expect("the last head of round 0 leaves");
    assert_eq!(combat(&p.w).round, 1, "the leave rolled the round over");
    let r = p.w.creatures().get(a).expect("a record");
    assert_eq!(r.suspended().len(), 1);
    assert_eq!(r.suspended()[0].remaining_rounds, 3, "3 - round 0");
}

/// B33 Q1: the real-time arm and the no-active-turn arm, with the exact
/// message; the loader half of D45a.
#[test]
fn b33_q1_other_arms() {
    let q1 = "combat pending joins outside a first turn-based round";
    let id = |e: EntityId| serde_json::to_value(e).unwrap();
    let rt = Play::in_mode(capable(10), RT);
    assert!(try_with_json(&rt.w, |_| {}).is_ok());
    let err = try_with_json(&rt.w, |s| {
        s["combat"]["pending_joins"] = json!([id(rt.a)]);
    })
    .expect_err("an RT save with pending joins must not load");
    assert!(err.contains(q1), "{err}");

    let t = terminal_play();
    assert!(try_with_json(&t.w, |_| {}).is_ok());
    let err = try_with_json(&t.w, |s| {
        s["combat"]["pending_joins"] = json!([id(t.b)]);
    })
    .expect_err("a terminal save with pending joins must not load");
    assert!(err.contains(q1), "{err}");
}

/// B33 Q1: the `mode == TurnBased` arm holds on its own. An RT save whose
/// `active` is the scheduled head passes the active-head check, so only the
/// mode arm can reject the pending entry; without it the load would fail
/// later on the RT active turn.
#[test]
fn b33_q1_mode_arm_with_active_head() {
    let q1 = "combat pending joins outside a first turn-based round";
    let rt = Play::in_mode(capable(10), RT);
    let head = timeline_of(&rt.w)[0].1;
    let id = |e: EntityId| serde_json::to_value(e).unwrap();
    // Control: the unedited RT save loads, and its head is the active one
    // this test installs.
    assert!(try_with_json(&rt.w, |_| {}).is_ok());
    let err = try_with_json(&rt.w, |s| {
        s["combat"]["active"] = id(head);
        s["combat"]["pending_joins"] = json!([id(rt.a)]);
    })
    .expect_err("an RT save with an active head and pending joins must not load");
    assert!(err.contains(q1), "{err}");
    assert!(
        !err.contains("real-time combat holds an active turn"),
        "the mode arm must win before the RT active-turn check: {err}"
    );
}

/// B30: an RT leave ends the encounter exactly when the alive count falls
/// from two or more to below two. The world does not show it (`combat` is
/// identical to a not-ended one), so this reads the journal's
/// `EncounterEnded`.
#[test]
fn b30_rt_leave_ends_only_below_two_alive() {
    let ended = |h: &crpg_sim::HistoryWorld| {
        h.read_after(0, 256)
            .expect("reads")
            .iter()
            .any(|e| matches!(e.payload, HistoryEvent::EncounterEnded { .. }))
    };
    let (mut h, v) = capable_trio(10).start_history(SEED, RT).expect("starts");
    h.leave_encounter(v[0]).expect("3 -> 2 alive");
    assert!(!ended(&h), "two alive: not ended");
    h.leave_encounter(v[1]).expect("2 -> 1 alive");
    assert!(ended(&h), "one alive: ended");

    let (mut h, v) = capable(10).start_history(SEED, RT).expect("starts");
    h.leave_encounter(v[0]).expect("2 -> 1 alive");
    assert!(ended(&h));
}
