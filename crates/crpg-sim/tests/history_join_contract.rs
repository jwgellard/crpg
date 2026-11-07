//! Two-keys contract tests for the T139c journal (`tasks/T139.md`, Behaviour
//! items B35-B38).
//!
//! Written independently from the contract text through the public
//! `HistoryWorld` API only. The implementer's own tests
//! (`history_join.rs`, the unit tests in `src/history.rs`) and the T139c
//! code in `src/` were not read; only the public signatures needed to call
//! were. Every test names the Behaviour items it proves and carries a
//! control. Every rejection also asserts that nothing was committed: the
//! canonical wrapper bytes, the journal length and byte total, both hashes,
//! the version and the next draw of a cloned RNG are unchanged.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Ulid};
use crpg_data::LateJoinWire;
use crpg_sim::{
    history_hash, state_hash, CombatAction, CombatError, EncounterMode, EntityMeta, Entrant,
    HistoryEnvelope, HistoryError, HistoryEvent, HistoryWorld, COMBAT_ROLL_STREAM,
    MAX_HISTORY_EVENTS,
};
use serde_json::{json, Value};
use support::creature::{add_outsider, set_hooks};
use support::draws::{draw, set_initiative_rule};
use support::rt::{capable, capable_trio, make_lethal, timeline_of};
use support::surface::always_success;
use support::Fixture;

const TB: EncounterMode = EncounterMode::TurnBased;
const RT: EncounterMode = EncounterMode::RealTime;

// ----------------------------------------------------------------- fixtures

/// Adds the outsiders 213..=216 (creatures 223..=226) to `fixture`.
fn with_outsiders(mut fixture: Fixture) -> Fixture {
    for n in 213..=216 {
        add_outsider(&mut fixture, n, n + 10);
    }
    fixture
}

/// Standard fixture (TB only, A 10 / B 9) with outsiders.
fn tb_only() -> Fixture {
    with_outsiders(Fixture::standard())
}

/// RT-capable standard fixture with outsiders.
fn capable_two() -> Fixture {
    with_outsiders(capable(4))
}

/// Registers outsider `placement` and returns its entity.
fn reg(history: &mut HistoryWorld, fixture: &Fixture, placement: u128) -> EntityId {
    fixture
        .register_history(history, Ulid::from_u128(placement))
        .expect("the outsider registers")
}

fn entrant(entity: EntityId, initiative: i32) -> Entrant {
    Entrant { entity, initiative }
}

fn start(
    history: &mut HistoryWorld,
    fixture: &Fixture,
    mode: EncounterMode,
    members: &[Entrant],
) -> Result<(), HistoryError> {
    fixture.with_spec(|spec| history.start_encounter_with_members(spec, mode, members))
}

/// A fresh wrapper with the fixture's encounter started and no members.
fn started(fixture: &Fixture, seed: u64, mode: EncounterMode) -> HistoryWorld {
    let mut history = HistoryWorld::new(seed);
    start(&mut history, fixture, mode, &[]).expect("the encounter starts");
    history
}

/// The combatant that was built from `placement`.
fn combatant_of(history: &HistoryWorld, placement: u128) -> EntityId {
    let wanted = Ulid::from_u128(placement);
    history
        .world()
        .combatants()
        .iter()
        .find(|(_, combatant)| combatant.placement() == wanted)
        .map(|(id, _)| id)
        .expect("a combatant of that placement")
}

fn end_turn(history: &mut HistoryWorld) {
    let actor = history
        .world()
        .combat()
        .expect("an encounter")
        .active
        .expect("an active head");
    history
        .perform_action(&CombatAction::EndTurn { actor })
        .expect("the turn ends");
}

fn mark(history: &HistoryWorld) -> u64 {
    history.last_sequence()
}

/// The payloads journaled after `mark`.
fn tail(history: &HistoryWorld, mark: u64) -> Vec<HistoryEvent> {
    assert!(
        history.world().events().is_empty(),
        "the legacy queue is drained by every wrapper operation"
    );
    history
        .read_after(mark, 256)
        .expect("a page")
        .into_iter()
        .map(|envelope| envelope.payload)
        .collect()
}

fn combat_err(error: CombatError) -> HistoryError {
    HistoryError::Combat(error)
}

fn encounter_id() -> Ulid {
    Ulid::from_u128(101)
}

/// The next combat-stream draw of a clone of the wrapper's RNG.
fn next_draw(history: &HistoryWorld) -> u32 {
    let mut world = history.world().clone();
    let mut rng = world.rng_mut().clone();
    rng.stream(COMBAT_ROLL_STREAM).next_u32()
}

/// Everything a rejected operation must leave alone.
struct Snap {
    bytes: Vec<u8>,
    len: usize,
    journal_bytes: usize,
    last: u64,
    acknowledged: u64,
    state: [u8; 32],
    history: [u8; 32],
    draw: u32,
    version: u32,
}

fn snap(history: &HistoryWorld) -> Snap {
    Snap {
        bytes: serde_json::to_vec(history).expect("the wrapper serializes"),
        len: history.pending_len(),
        journal_bytes: history.pending_bytes(),
        last: history.last_sequence(),
        acknowledged: history.acknowledged(),
        state: state_hash(history.world()),
        history: history_hash(history),
        draw: next_draw(history),
        version: history.version(),
    }
}

fn assert_unchanged(before: &Snap, history: &HistoryWorld, what: &str) {
    let after = snap(history);
    assert!(before.bytes == after.bytes, "{what}: wrapper bytes changed");
    assert_eq!(before.len, after.len, "{what}: journal length");
    assert_eq!(
        before.journal_bytes, after.journal_bytes,
        "{what}: journal bytes"
    );
    assert_eq!(before.last, after.last, "{what}: last sequence");
    assert_eq!(before.acknowledged, after.acknowledged, "{what}: watermark");
    assert_eq!(before.state, after.state, "{what}: state_hash");
    assert_eq!(before.history, after.history, "{what}: history_hash");
    assert_eq!(before.draw, after.draw, "{what}: next RNG draw");
    assert_eq!(before.version, after.version, "{what}: version");
}

fn fill_to(history: &mut HistoryWorld, count: usize) {
    while history.pending_len() < count {
        history.spawn(EntityMeta {}).expect("a filler spawns");
    }
    assert_eq!(history.pending_len(), count);
}

/// Exact envelope text of `payload_json` at (`tick`, `seq`).
fn envelope_text(tick: u64, seq: u64, payload_json: &str) -> String {
    format!(r#"{{"tick":{tick},"seq":{seq},"payload":{payload_json}}}"#)
}

fn id_json(entity: EntityId) -> String {
    serde_json::to_string(&entity).expect("an id serializes")
}

// --------------------------------------------------------------------- B35

/// B35: exact `joined` and `left` bytes, `type` before `value`, round trips.
/// Control: the `spawned` encoding is unchanged.
#[test]
fn b35_payload_bytes_and_round_trips() {
    let mut history = HistoryWorld::new(0);
    history.spawn(EntityMeta {}).expect("first");
    let entity = history.spawn(EntityMeta {}).expect("second");
    assert_eq!(id_json(entity), r#"{"index":1,"generation":1}"#);
    let cases = [
        (
            HistoryEvent::Joined { entity },
            r#"{"type":"joined","value":{"entity":{"index":1,"generation":1}}}"#,
        ),
        (
            HistoryEvent::Left { entity },
            r#"{"type":"left","value":{"entity":{"index":1,"generation":1}}}"#,
        ),
        (
            HistoryEvent::Spawned { entity },
            r#"{"type":"spawned","value":{"entity":{"index":1,"generation":1}}}"#,
        ),
    ];
    for (event, text) in cases {
        assert_eq!(serde_json::to_string(&event).expect("encodes"), text);
        let wrapped = envelope_text(4, 9, text);
        let envelope: HistoryEnvelope = serde_json::from_str(&wrapped).expect("decodes");
        assert_eq!(envelope.payload, event);
        assert_eq!(envelope.seq, 9);
        assert_eq!(
            serde_json::to_string(&envelope).expect("re-encodes"),
            wrapped
        );
    }
    // The two new tags are distinct variants of one entity.
    assert_ne!(
        HistoryEvent::Joined { entity },
        HistoryEvent::Left { entity }
    );
    // An unknown tag still fails with the existing text.
    let unknown = r#"{"type":"entered","value":{"entity":{"index":1,"generation":1}}}"#;
    let error = serde_json::from_str::<HistoryEvent>(unknown).expect_err("unknown type");
    assert!(error.to_string().starts_with("unknown history event type"));
}

/// The error text without serde_json's trailing position.
fn bare(error: serde_json::Error) -> String {
    let text = error.to_string();
    text.split(" at line ")
        .next()
        .expect("a message")
        .to_owned()
}

/// B35: a duplicate, unknown or missing `entity` key fails with the
/// `EntityRef` messages (as `spawned` does), and `value` before `type`
/// fails as today. Control: the same documents under `spawned`.
#[test]
fn b35_key_faults_match_spawned() {
    let e = r#"{"index":0,"generation":1}"#;
    let documents = [
        (
            format!(r#"{{"type":"TAG","value":{{"entity":{e},"entity":{e}}}}}"#),
            "duplicate history payload field `entity`",
        ),
        (
            format!(r#"{{"type":"TAG","value":{{"entity":{e},"extra":1}}}}"#),
            "unknown history payload field",
        ),
        (
            format!(r#"{{"type":"TAG","value":{{"other":{e}}}}}"#),
            "unknown history payload field",
        ),
        (
            r#"{"type":"TAG","value":{}}"#.to_owned(),
            "history payload misses `entity`",
        ),
        (
            format!(r#"{{"value":{{"entity":{e}}},"type":"TAG"}}"#),
            "history payload requires `type` before `value`",
        ),
        (
            format!(r#"{{"type":"TAG","value":{{"entity":{e}}},"extra":1}}"#),
            "unknown history payload field",
        ),
    ];
    for (template, expected) in documents {
        let control = bare(
            serde_json::from_str::<HistoryEvent>(&template.replace("TAG", "spawned"))
                .expect_err("the control fault fails"),
        );
        assert_eq!(control, expected, "spawned control for {template}");
        for tag in ["joined", "left"] {
            let text = template.replace("TAG", tag);
            let error = serde_json::from_str::<HistoryEvent>(&text)
                .expect_err("the fault must be rejected");
            assert_eq!(bare(error), expected, "{tag}: {text}");
        }
    }
    // Control: the unfaulted documents decode.
    for tag in ["joined", "left", "spawned"] {
        let ok = format!(r#"{{"type":"{tag}","value":{{"entity":{e}}}}}"#);
        serde_json::from_str::<HistoryEvent>(&ok).expect("a clean payload decodes");
    }
}

// --------------------------------------------------------------------- B36

/// B36: `start_encounter_with_members` journals `Spawned` per spawned
/// participant in drain order, then `TurnStarted` when a head was scheduled;
/// members and Enter participants journal nothing. Control: `members == []`
/// journals what `start_encounter_in_mode` does.
#[test]
fn b36_tb_start_with_members_orders() {
    let fixture = tb_only();

    // A member that is the head.
    let mut history = HistoryWorld::new(1);
    let m = reg(&mut history, &fixture, 213);
    let before = mark(&history);
    start(&mut history, &fixture, TB, &[entrant(m, 5)]).expect("starts");
    let a = combatant_of(&history, 111);
    let b = combatant_of(&history, 112);
    assert_eq!(
        tail(&history, before),
        vec![
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
            HistoryEvent::TurnStarted { actor: m, round: 0 },
        ]
    );
    assert!(history.world().combatants().contains(m));
    assert!(!history.world().creatures().contains(m));
    assert_eq!(history.last_sequence(), before + 3);

    // A member that is not the head: B (9) is.
    let mut history = HistoryWorld::new(1);
    let m = reg(&mut history, &fixture, 213);
    let before = mark(&history);
    start(&mut history, &fixture, TB, &[entrant(m, 20)]).expect("starts");
    let a = combatant_of(&history, 111);
    let b = combatant_of(&history, 112);
    assert_eq!(
        tail(&history, before),
        vec![
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
            HistoryEvent::TurnStarted { actor: b, round: 0 },
        ]
    );

    // Two members, in slice order, journal nothing of their own.
    let mut history = HistoryWorld::new(1);
    let m1 = reg(&mut history, &fixture, 213);
    let m2 = reg(&mut history, &fixture, 214);
    let before = mark(&history);
    start(
        &mut history,
        &fixture,
        TB,
        &[entrant(m1, 30), entrant(m2, 2)],
    )
    .expect("starts");
    let a = combatant_of(&history, 111);
    let b = combatant_of(&history, 112);
    assert_eq!(
        tail(&history, before),
        vec![
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
            HistoryEvent::TurnStarted {
                actor: m2,
                round: 0
            },
        ]
    );
}

/// B36: an Enter participant (its placement already holds a record) and a
/// member journal no `Spawned`. Control: the Spawn participant does.
#[test]
fn b36_enter_participant_and_member_journal_no_spawned() {
    let fixture = tb_only();
    let mut history = HistoryWorld::new(2);
    let enter = reg(&mut history, &fixture, 111);
    let m = reg(&mut history, &fixture, 213);
    let before = mark(&history);
    start(&mut history, &fixture, TB, &[entrant(m, 20)]).expect("starts");
    let b = combatant_of(&history, 112);
    assert!(history.world().combatants().contains(enter));
    assert_eq!(
        tail(&history, before),
        vec![
            HistoryEvent::Spawned { entity: b },
            HistoryEvent::TurnStarted { actor: b, round: 0 },
        ]
    );

    // The Enter participant as the head: `TurnStarted` names it.
    let mut fixture = tb_only();
    fixture.encounter.participants[0].initiative = 1;
    let mut history = HistoryWorld::new(2);
    let enter = reg(&mut history, &fixture, 111);
    let m = reg(&mut history, &fixture, 213);
    let before = mark(&history);
    start(&mut history, &fixture, TB, &[entrant(m, 20)]).expect("starts");
    let b = combatant_of(&history, 112);
    assert_eq!(
        tail(&history, before),
        vec![
            HistoryEvent::Spawned { entity: b },
            HistoryEvent::TurnStarted {
                actor: enter,
                round: 0
            },
        ]
    );

    // Both participants enter and a member joins: only the head is journaled.
    let fixture = tb_only();
    let mut history = HistoryWorld::new(2);
    let a = reg(&mut history, &fixture, 111);
    let b = reg(&mut history, &fixture, 112);
    let m = reg(&mut history, &fixture, 213);
    let before = mark(&history);
    start(&mut history, &fixture, TB, &[entrant(m, 20)]).expect("starts");
    assert_eq!(
        tail(&history, before),
        vec![HistoryEvent::TurnStarted { actor: b, round: 0 }]
    );
    assert!(history.world().combatants().contains(a));
}

/// B36 control: with no members the journal and the whole wrapper equal
/// `start_encounter_in_mode`'s.
#[test]
fn b36_empty_members_equals_start_in_mode() {
    for (fixture, mode) in [(tb_only(), TB), (capable_two(), TB), (capable_two(), RT)] {
        let mut plain = HistoryWorld::new(9);
        fixture
            .with_spec(|spec| plain.start_encounter_in_mode(spec, mode))
            .expect("starts");
        let mut members = HistoryWorld::new(9);
        start(&mut members, &fixture, mode, &[]).expect("starts");
        assert_eq!(tail(&plain, 0), tail(&members, 0));
        assert_eq!(history_hash(&plain), history_hash(&members));
        assert_eq!(plain.version(), members.version());
    }
}

/// B36: an RT start journals only its `Spawned` events (members and Enter
/// participants journal nothing), and no `TurnStarted`.
#[test]
fn b36_rt_start_with_members_orders() {
    let fixture = capable_two();
    let mut history = HistoryWorld::new(3);
    let m = reg(&mut history, &fixture, 213);
    let before = mark(&history);
    start(&mut history, &fixture, RT, &[entrant(m, 5)]).expect("starts");
    let a = combatant_of(&history, 111);
    let b = combatant_of(&history, 112);
    assert_eq!(
        tail(&history, before),
        vec![
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
        ]
    );
    assert!(history.world().combatants().contains(m));
    assert_eq!(history.version(), 2);

    // An Enter participant adds no `Spawned` in RT either.
    let mut history = HistoryWorld::new(3);
    let enter = reg(&mut history, &fixture, 111);
    let m = reg(&mut history, &fixture, 213);
    let before = mark(&history);
    start(&mut history, &fixture, RT, &[entrant(m, 5)]).expect("starts");
    let b = combatant_of(&history, 112);
    assert!(history.world().combatants().contains(enter));
    assert_eq!(
        tail(&history, before),
        vec![HistoryEvent::Spawned { entity: b }]
    );
}

/// B35 + B36: `join_encounter` journals exactly `[Joined]`, whichever
/// schedule the joiner takes (inserted, waiting, pending) and in RT.
/// Control: the timeline shows the schedule really differed.
#[test]
fn b36_join_journals_exactly_one_joined() {
    // TB, default: a late key is inserted at once, an early one waits.
    let fixture = tb_only();
    let mut history = started(&fixture, 4, TB);
    let late = reg(&mut history, &fixture, 213);
    let early = reg(&mut history, &fixture, 214);
    let before = mark(&history);
    history.join_encounter(&entrant(late, 20)).expect("joins");
    let mark_after = mark(&history);
    assert_eq!(
        tail(&history, before),
        vec![HistoryEvent::Joined { entity: late }]
    );
    assert!(timeline_of(history.world())
        .iter()
        .any(|&(_, id)| id == late));
    let envelope = &history.read_after(before, 1).expect("page")[0];
    assert_eq!(
        serde_json::to_string(envelope).expect("encodes"),
        envelope_text(
            0,
            before + 1,
            &format!(
                r#"{{"type":"joined","value":{{"entity":{}}}}}"#,
                id_json(late)
            )
        )
    );
    history.join_encounter(&entrant(early, 1)).expect("joins");
    assert_eq!(
        tail(&history, mark_after),
        vec![HistoryEvent::Joined { entity: early }]
    );
    assert!(!timeline_of(history.world())
        .iter()
        .any(|&(_, id)| id == early));
    assert!(history.world().combatants().contains(early));

    // TB, after_active: an early key is pending, still `[Joined]` only.
    let mut fixture = tb_only();
    set_hooks(&mut fixture, None, Some(LateJoinWire::AfterActive), None);
    let mut history = started(&fixture, 4, TB);
    let early = reg(&mut history, &fixture, 214);
    let before = mark(&history);
    history.join_encounter(&entrant(early, 1)).expect("joins");
    assert_eq!(
        tail(&history, before),
        vec![HistoryEvent::Joined { entity: early }]
    );
    assert_eq!(
        history.world().combat().expect("encounter").pending_joins,
        vec![early]
    );

    // RT.
    let fixture = capable_two();
    let mut history = started(&fixture, 4, RT);
    let joiner = reg(&mut history, &fixture, 213);
    let before = mark(&history);
    history.join_encounter(&entrant(joiner, 0)).expect("joins");
    assert_eq!(
        tail(&history, before),
        vec![HistoryEvent::Joined { entity: joiner }]
    );
    assert!(history.world().combatants().contains(joiner));
}

/// B36: a leave journals `Left`, then the trailing transition. A non-head
/// leaves quietly; a head hands the turn over; the last combatant of a
/// nonterminal TB encounter adds `EncounterEnded`. Control: a despawn of the
/// same head gives the same turn hand-over, with `Despawned` for `Left`.
#[test]
fn b36_leave_orders_non_head_head_and_last_nonterminal() {
    let fixture = with_outsiders(Fixture::trio());

    // The head leaves (two combatants remain): `[Left, TurnStarted { next }]`.
    let mut history = started(&fixture, 5, TB);
    let mut twin = started(&fixture, 5, TB);
    let b = combatant_of(&history, 112);
    let c = combatant_of(&history, 113);
    assert_eq!(history.world().combat().expect("encounter").active, Some(c));
    let before = mark(&history);
    history.leave_encounter(c).expect("c leaves");
    assert_eq!(
        tail(&history, before),
        vec![
            HistoryEvent::Left { entity: c },
            HistoryEvent::TurnStarted { actor: b, round: 0 },
        ]
    );
    assert_eq!(history.world().combat().expect("live").active, Some(b));
    let before = mark(&twin);
    twin.despawn(c).expect("c despawns");
    assert_eq!(
        tail(&twin, before),
        vec![
            HistoryEvent::Despawned { entity: c },
            HistoryEvent::TurnStarted { actor: b, round: 0 },
        ]
    );
    assert_eq!(history.world().combat(), twin.world().combat());
    assert_eq!(timeline_of(history.world()), timeline_of(twin.world()));

    // Non-heads leave: `[Left]`, the turn is untouched, the leaver is live.
    let mut history = started(&fixture, 5, TB);
    let a = combatant_of(&history, 111);
    let b = combatant_of(&history, 112);
    let c = combatant_of(&history, 113);
    let before = mark(&history);
    history.leave_encounter(a).expect("a leaves");
    assert_eq!(
        tail(&history, before),
        vec![HistoryEvent::Left { entity: a }]
    );
    assert_eq!(history.world().combat().expect("live").active, Some(c));
    assert!(
        history.world().contains(a),
        "the leaver stays a live entity"
    );
    assert!(!history.world().combatants().contains(a));
    let before = mark(&history);
    history.leave_encounter(b).expect("b leaves");
    assert_eq!(
        tail(&history, before),
        vec![HistoryEvent::Left { entity: b }]
    );
    assert_eq!(history.world().combat().expect("live").active, Some(c));

    // The last combatant of a nonterminal encounter: `[Left, EncounterEnded]`.
    let before = mark(&history);
    history.leave_encounter(c).expect("c leaves");
    assert_eq!(
        tail(&history, before),
        vec![
            HistoryEvent::Left { entity: c },
            HistoryEvent::EncounterEnded {
                encounter: encounter_id(),
                round: 0
            },
        ]
    );
    assert!(history.world().combat().is_none());
}

/// A two-combatant lethal fixture whose first action kills.
fn lethal() -> Fixture {
    let mut fixture = tb_only();
    make_lethal(&mut fixture, 100);
    always_success(&mut fixture);
    fixture
}

/// Drives `lethal()` to a terminal encounter: B killed A.
fn terminal_world(fixture: &Fixture) -> (HistoryWorld, EntityId, EntityId) {
    let mut history = started(fixture, 6, TB);
    let a = combatant_of(&history, 111);
    let b = combatant_of(&history, 112);
    history
        .perform_action(&CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        })
        .expect("the kill is accepted");
    assert!(history
        .world()
        .combat()
        .expect("encounter")
        .active
        .is_none());
    (history, a, b)
}

/// B36: the last combatant of a terminal TB encounter journals `[Left]` and
/// nothing more. Control: leaving a survivor of the terminal encounter
/// likewise adds nothing, and the dead combatant cannot leave.
#[test]
fn b36_leave_in_terminal_encounter_adds_no_second_end() {
    let fixture = lethal();

    // The survivor leaves a terminal encounter holding a dead body.
    let (mut history, a, b) = terminal_world(&fixture);
    let before = mark(&history);
    history.leave_encounter(b).expect("b leaves");
    assert_eq!(
        tail(&history, before),
        vec![HistoryEvent::Left { entity: b }]
    );
    assert!(history.world().combat().is_some());
    assert!(history.world().combatants().contains(a));

    // The last combatant of a terminal encounter.
    let (mut history, a, b) = terminal_world(&fixture);
    assert!(history.despawn(a).expect("the body despawns"));
    let before = mark(&history);
    history.leave_encounter(b).expect("b leaves");
    assert_eq!(
        tail(&history, before),
        vec![HistoryEvent::Left { entity: b }]
    );
    assert!(history.world().combat().is_none());

    // The dead cannot leave.
    let (mut history, a, _) = terminal_world(&fixture);
    let snapshot = snap(&history);
    assert_eq!(
        history.leave_encounter(a),
        Err(combat_err(CombatError::DeadActor { actor: a }))
    );
    assert_unchanged(&snapshot, &history, "a dead leaver");
}

/// B36: a head's leave that rolls the round over into a pending RT switch
/// journals `[Left, ModeChanged, RoundStarted]`. Control: without the
/// request the same leave journals `[Left, TurnStarted]` in round 1.
#[test]
fn b36_head_leave_rolls_into_pending_rt_switch() {
    let fixture = with_outsiders(capable_trio(4));
    let setup = |request: bool| {
        let mut history = started(&fixture, 7, TB);
        if request {
            history.request_mode(RT).expect("the request is taken");
        }
        let c = combatant_of(&history, 113);
        let b = combatant_of(&history, 112);
        let a = combatant_of(&history, 111);
        end_turn(&mut history);
        end_turn(&mut history);
        assert_eq!(history.world().combat().expect("encounter").active, Some(a));
        (history, a, b, c)
    };

    let (mut history, a, _, _) = setup(true);
    let before = mark(&history);
    history.leave_encounter(a).expect("the head leaves");
    assert_eq!(
        tail(&history, before),
        vec![
            HistoryEvent::Left { entity: a },
            HistoryEvent::ModeChanged { mode: RT, round: 1 },
            HistoryEvent::RoundStarted { round: 1 },
        ]
    );
    let state = history.world().combat().expect("encounter");
    assert_eq!(state.mode, RT);
    assert_eq!(state.round, 1);

    let (mut history, a, _, c) = setup(false);
    let before = mark(&history);
    history.leave_encounter(a).expect("the head leaves");
    assert_eq!(
        tail(&history, before),
        vec![
            HistoryEvent::Left { entity: a },
            HistoryEvent::TurnStarted { actor: c, round: 1 },
        ]
    );
}

/// B36: a head's leave that flushes a pending joiner J journals
/// `[Left, TurnStarted { J, 0 }]`. Control: under `next_round` the joiner
/// waits and the other participant is next.
#[test]
fn b36_head_leave_flushes_pending_joiner() {
    let mut fixture = tb_only();
    set_hooks(&mut fixture, None, Some(LateJoinWire::AfterActive), None);
    let mut history = started(&fixture, 8, TB);
    let a = combatant_of(&history, 111);
    let b = combatant_of(&history, 112);
    let j = reg(&mut history, &fixture, 213);
    history.join_encounter(&entrant(j, 1)).expect("joins");
    assert_eq!(
        history.world().combat().expect("encounter").pending_joins,
        vec![j]
    );
    let before = mark(&history);
    history.leave_encounter(b).expect("the head leaves");
    assert_eq!(
        tail(&history, before),
        vec![
            HistoryEvent::Left { entity: b },
            HistoryEvent::TurnStarted { actor: j, round: 0 },
        ]
    );
    assert!(history
        .world()
        .combat()
        .expect("encounter")
        .pending_joins
        .is_empty());
    assert_eq!(history.world().combat().expect("encounter").active, Some(j));
    assert!(history.world().combatants().contains(a));

    let fixture = tb_only();
    let mut history = started(&fixture, 8, TB);
    let a = combatant_of(&history, 111);
    let b = combatant_of(&history, 112);
    let j = reg(&mut history, &fixture, 213);
    history.join_encounter(&entrant(j, 1)).expect("joins");
    let before = mark(&history);
    history.leave_encounter(b).expect("the head leaves");
    assert_eq!(
        tail(&history, before),
        vec![
            HistoryEvent::Left { entity: b },
            HistoryEvent::TurnStarted { actor: a, round: 0 },
        ]
    );
}

/// B36: in RT a leave that drops the alive count from two or more to below
/// two journals `[Left, EncounterEnded]`; a leave that leaves two journals
/// `[Left]`.
#[test]
fn b36_rt_leave_orders() {
    let fixture = capable_two();
    let mut history = HistoryWorld::new(10);
    let m = reg(&mut history, &fixture, 213);
    start(&mut history, &fixture, RT, &[entrant(m, 5)]).expect("starts");
    let a = combatant_of(&history, 111);
    let b = combatant_of(&history, 112);

    let before = mark(&history);
    history.leave_encounter(m).expect("m leaves");
    assert_eq!(
        tail(&history, before),
        vec![HistoryEvent::Left { entity: m }]
    );

    let round = u64::from(history.world().combat().expect("encounter").round);
    let before = mark(&history);
    history.leave_encounter(a).expect("a leaves");
    assert_eq!(
        tail(&history, before),
        vec![
            HistoryEvent::Left { entity: a },
            HistoryEvent::EncounterEnded {
                encounter: encounter_id(),
                round
            },
        ]
    );
    assert!(history.world().combatants().contains(b));
    assert!(history.world().contains(a));
}

/// B36: `end_encounter` still journals nothing, even with records written.
/// Control: the records exist, the summary is returned, and a join/leave in
/// the same encounter did journal.
#[test]
fn b36_end_encounter_journals_nothing() {
    let fixture = tb_only();
    let mut history = HistoryWorld::new(11);
    let m = reg(&mut history, &fixture, 213);
    let j = reg(&mut history, &fixture, 214);
    start(&mut history, &fixture, TB, &[entrant(m, 5)]).expect("starts");
    history.join_encounter(&entrant(j, 30)).expect("joins");
    let a = combatant_of(&history, 111);
    history.leave_encounter(a).expect("a leaves");
    assert_eq!(
        history.world().creatures().len(),
        1,
        "the leaver is a record"
    );
    let before = snap(&history);
    let summary = history.end_encounter().expect("releases");
    assert_eq!(summary.encounter, encounter_id());
    assert_eq!(
        history.world().creatures().len(),
        4,
        "m, j, b and a hold records"
    );
    assert_eq!(history.pending_len(), before.len);
    assert_eq!(history.last_sequence(), before.last);
    assert_eq!(history.pending_bytes(), before.journal_bytes);
    assert!(history.world().combat().is_none());
}

/// B36: every gameplay rejection commits nothing, rolled draws included.
/// Control: the corrected call succeeds.
#[test]
fn b36_rejections_commit_nothing() {
    let mut fixture = tb_only();
    set_initiative_rule(&mut fixture, "1d6", &[]);

    // start_encounter_with_members.
    let mut history = HistoryWorld::new(12);
    let m = reg(&mut history, &fixture, 213);
    let gone = reg(&mut history, &fixture, 214);
    assert!(history.despawn(gone).expect("despawns"));
    let plain = history.spawn(EntityMeta {}).expect("a bare entity");
    let enter = reg(&mut history, &fixture, 111);
    let cases = [
        (
            vec![entrant(gone, 1)],
            CombatError::AbsentActor { actor: gone },
        ),
        (
            vec![entrant(m, 1), entrant(m, 2)],
            CombatError::AlreadyParticipant { entity: m },
        ),
        (
            vec![entrant(plain, 1)],
            CombatError::NotCreature { entity: plain },
        ),
        (
            vec![entrant(enter, 1)],
            CombatError::PlacementInUse {
                placement: Ulid::from_u128(111),
            },
        ),
    ];
    for (members, error) in cases {
        let snapshot = snap(&history);
        let result = start(&mut history, &fixture, TB, &members);
        assert_eq!(result, Err(combat_err(error.clone())), "{error:?}");
        assert_unchanged(&snapshot, &history, "a rejected start");
        assert!(history.world().combat().is_none());
    }
    start(&mut history, &fixture, TB, &[entrant(m, 4)]).expect("the control starts");
    let snapshot = snap(&history);
    assert_eq!(
        start(&mut history, &fixture, TB, &[entrant(m, 4)]),
        Err(combat_err(CombatError::EncounterActive))
    );
    assert_unchanged(&snapshot, &history, "a start over a live encounter");

    // join_encounter.
    let mut history = HistoryWorld::new(12);
    let j = reg(&mut history, &fixture, 213);
    let snapshot = snap(&history);
    assert_eq!(
        history.join_encounter(&entrant(j, 3)),
        Err(combat_err(CombatError::NoEncounter))
    );
    assert_unchanged(&snapshot, &history, "a join with no encounter");
    start(&mut history, &fixture, TB, &[]).expect("starts");
    let a = combatant_of(&history, 111);
    let gone = reg(&mut history, &fixture, 214);
    assert!(history.despawn(gone).expect("despawns"));
    let plain = history.spawn(EntityMeta {}).expect("a bare entity");
    let cases = [
        (entrant(gone, 3), CombatError::AbsentActor { actor: gone }),
        (entrant(a, 3), CombatError::AlreadyParticipant { entity: a }),
        (
            entrant(plain, 3),
            CombatError::NotCreature { entity: plain },
        ),
    ];
    for (joiner, error) in cases {
        let snapshot = snap(&history);
        assert_eq!(
            history.join_encounter(&joiner),
            Err(combat_err(error.clone())),
            "{error:?}"
        );
        assert_unchanged(&snapshot, &history, "a rejected join");
    }
    history
        .join_encounter(&entrant(j, 3))
        .expect("the control joins");
    assert!(history.world().combatants().contains(j));

    // join into a terminal encounter.
    let lethal = lethal();
    let (mut history, _, _) = terminal_world(&lethal);
    let j = reg(&mut history, &lethal, 213);
    let snapshot = snap(&history);
    assert_eq!(
        history.join_encounter(&entrant(j, 3)),
        Err(combat_err(CombatError::EncounterTerminal))
    );
    assert_unchanged(&snapshot, &history, "a join into a terminal encounter");

    // leave_encounter.
    let mut history = HistoryWorld::new(12);
    let m = reg(&mut history, &fixture, 213);
    let snapshot = snap(&history);
    assert_eq!(
        history.leave_encounter(m),
        Err(combat_err(CombatError::NoEncounter))
    );
    assert_unchanged(&snapshot, &history, "a leave with no encounter");
    start(&mut history, &fixture, TB, &[]).expect("starts");
    let gone = reg(&mut history, &fixture, 214);
    assert!(history.despawn(gone).expect("despawns"));
    for (entity, error) in [
        (m, CombatError::NotParticipant { entity: m }),
        (gone, CombatError::AbsentActor { actor: gone }),
    ] {
        let snapshot = snap(&history);
        assert_eq!(
            history.leave_encounter(entity),
            Err(combat_err(error.clone()))
        );
        assert_unchanged(&snapshot, &history, "a rejected leave");
    }
    let a = combatant_of(&history, 111);
    history.leave_encounter(a).expect("the control leaves");
    assert!(!history.world().combatants().contains(a));
}

/// B36: a full journal fails `start_encounter_with_members` with
/// `EventLimit`, commits nothing (the member keeps its record, no entity is
/// minted, no draw is taken) and succeeds after one acknowledgement.
#[test]
fn b36_event_limit_rollback_start_with_members() {
    let mut fixture = tb_only();
    set_initiative_rule(&mut fixture, "1d6", &[]);
    let mut history = HistoryWorld::new(13);
    let m = reg(&mut history, &fixture, 213);
    fill_to(&mut history, MAX_HISTORY_EVENTS);
    let snapshot = snap(&history);
    let entities = history.world().len();
    assert_eq!(
        start(&mut history, &fixture, TB, &[entrant(m, 5)]),
        Err(HistoryError::EventLimit)
    );
    assert_unchanged(&snapshot, &history, "a start on a full journal");
    assert!(history.world().combat().is_none());
    assert!(history.world().creatures().contains(m));
    assert_eq!(history.world().len(), entities);
    assert_eq!(history.version(), 1);

    // One free slot is still too few: the start journals three events.
    history
        .acknowledge(history.acknowledged() + 1)
        .expect("acks");
    let snapshot = snap(&history);
    assert_eq!(
        start(&mut history, &fixture, TB, &[entrant(m, 5)]),
        Err(HistoryError::EventLimit)
    );
    assert_unchanged(&snapshot, &history, "a start with one slot free");

    // Control: two more acknowledged sequences free the three slots.
    history
        .acknowledge(history.acknowledged() + 2)
        .expect("acks");
    let before = mark(&history);
    start(&mut history, &fixture, TB, &[entrant(m, 5)]).expect("now it fits");
    assert_eq!(tail(&history, before).len(), 3);
    assert!(history.world().combatants().contains(m));
    assert_ne!(next_draw(&history), snapshot.draw, "the start drew");
}

/// B36: a rolled `join_encounter` on a full journal fails `EventLimit`,
/// commits nothing (the draw is not taken) and succeeds after one
/// acknowledgement with exactly one `1d6` drawn. B37: the rejection does not
/// upgrade a version 1 wrapper; the success does.
#[test]
fn b36_event_limit_rollback_join_encounter() {
    let mut fixture = tb_only();
    set_initiative_rule(&mut fixture, "1d6", &[]);
    let mut history = started(&fixture, 14, TB);
    let j = reg(&mut history, &fixture, 213);
    fill_to(&mut history, MAX_HISTORY_EVENTS);
    assert_eq!(history.version(), 1);
    let snapshot = snap(&history);
    assert_eq!(
        history.join_encounter(&entrant(j, 100)),
        Err(HistoryError::EventLimit)
    );
    assert_unchanged(&snapshot, &history, "a join on a full journal");
    assert!(history.world().creatures().contains(j));
    assert!(!history.world().combatants().contains(j));
    assert_eq!(history.version(), 1, "a rejected join never upgrades");

    // The draw a successful join must take.
    let mut oracle = history.world().clone().rng_mut().clone();
    draw(&mut oracle, "1d6");
    let expected = oracle.stream(COMBAT_ROLL_STREAM).next_u32();
    assert_ne!(expected, snapshot.draw);

    history
        .acknowledge(history.acknowledged() + 1)
        .expect("acks");
    let before = mark(&history);
    history
        .join_encounter(&entrant(j, 100))
        .expect("now it fits");
    assert_eq!(
        tail(&history, before),
        vec![HistoryEvent::Joined { entity: j }]
    );
    assert_eq!(next_draw(&history), expected, "exactly one 1d6 was drawn");
    assert_eq!(history.version(), 2);
}

/// B36: `leave_encounter` on a full journal fails `EventLimit` and commits
/// nothing, for a one-event leave and for a two-event head leave, and
/// succeeds after one acknowledgement. Control: the head leave with only one
/// slot free still fails, because it needs two.
#[test]
fn b36_event_limit_rollback_leave_encounter() {
    let fixture = with_outsiders(Fixture::trio());

    // A non-head's leave journals one event.
    let mut history = started(&fixture, 15, TB);
    let a = combatant_of(&history, 111);
    fill_to(&mut history, MAX_HISTORY_EVENTS);
    let snapshot = snap(&history);
    assert_eq!(history.leave_encounter(a), Err(HistoryError::EventLimit));
    assert_unchanged(&snapshot, &history, "a leave on a full journal");
    assert!(history.world().combatants().contains(a));
    assert_eq!(history.version(), 1);
    history
        .acknowledge(history.acknowledged() + 1)
        .expect("acks");
    let before = mark(&history);
    history.leave_encounter(a).expect("now it fits");
    assert_eq!(
        tail(&history, before),
        vec![HistoryEvent::Left { entity: a }]
    );
    assert_eq!(history.version(), 2);

    // The head's leave journals two: one free slot is not enough.
    let mut history = started(&fixture, 15, TB);
    let b = combatant_of(&history, 112);
    let c = combatant_of(&history, 113);
    fill_to(&mut history, MAX_HISTORY_EVENTS);
    let snapshot = snap(&history);
    assert_eq!(history.leave_encounter(c), Err(HistoryError::EventLimit));
    assert_unchanged(&snapshot, &history, "a head leave on a full journal");
    history
        .acknowledge(history.acknowledged() + 1)
        .expect("acks");
    let snapshot = snap(&history);
    assert_eq!(history.leave_encounter(c), Err(HistoryError::EventLimit));
    assert_unchanged(&snapshot, &history, "a head leave with one slot free");
    assert!(history.world().combatants().contains(c));
    assert_eq!(history.version(), 1);
    history
        .acknowledge(history.acknowledged() + 1)
        .expect("acks");
    let before = mark(&history);
    history.leave_encounter(c).expect("now it fits");
    assert_eq!(
        tail(&history, before),
        vec![
            HistoryEvent::Left { entity: c },
            HistoryEvent::TurnStarted { actor: b, round: 0 },
        ]
    );
}

/// B36: gameplay errors win before history errors, even on a full journal.
#[test]
fn b36_gameplay_error_beats_event_limit() {
    let fixture = tb_only();
    let mut history = HistoryWorld::new(16);
    let gone = reg(&mut history, &fixture, 213);
    assert!(history.despawn(gone).expect("despawns"));
    fill_to(&mut history, MAX_HISTORY_EVENTS);
    assert_eq!(
        start(&mut history, &fixture, TB, &[entrant(gone, 1)]),
        Err(combat_err(CombatError::AbsentActor { actor: gone }))
    );
    assert_eq!(
        history.join_encounter(&entrant(gone, 1)),
        Err(combat_err(CombatError::NoEncounter))
    );
    assert_eq!(
        history.leave_encounter(gone),
        Err(combat_err(CombatError::NoEncounter))
    );

    let mut history = started(&fixture, 16, TB);
    let a = combatant_of(&history, 111);
    let plain = history.spawn(EntityMeta {}).expect("a bare entity");
    fill_to(&mut history, MAX_HISTORY_EVENTS);
    let snapshot = snap(&history);
    assert_eq!(
        history.join_encounter(&entrant(a, 1)),
        Err(combat_err(CombatError::AlreadyParticipant { entity: a }))
    );
    assert_eq!(
        history.leave_encounter(plain),
        Err(combat_err(CombatError::NotParticipant { entity: plain }))
    );
    assert_unchanged(&snapshot, &history, "gameplay errors on a full journal");
}

// --------------------------------------------------------------------- B37

/// B37: a join upgrades a version 1 wrapper to 2, and so does a leave.
/// Control: every step before the join/leave stays at version 1, and the
/// version never goes back (release, restart, acknowledgement).
#[test]
fn b37_join_and_leave_upgrade_a_v1_wrapper() {
    let fixture = tb_only();
    let mut history = HistoryWorld::new(20);
    assert_eq!(history.version(), 1);
    let m = reg(&mut history, &fixture, 213);
    let j = reg(&mut history, &fixture, 214);
    start(&mut history, &fixture, TB, &[entrant(m, 5)]).expect("starts");
    end_turn(&mut history);
    assert_eq!(history.version(), 1, "start, members and turns stay at v1");
    history.join_encounter(&entrant(j, 30)).expect("joins");
    assert_eq!(history.version(), 2);
    history.end_encounter().expect("releases");
    assert_eq!(history.version(), 2);
    start(&mut history, &fixture, TB, &[]).expect("restarts");
    history.acknowledge(history.last_sequence()).expect("acks");
    assert_eq!(history.version(), 2, "it never goes back");

    let mut history = HistoryWorld::new(20);
    start(&mut history, &fixture, TB, &[]).expect("starts");
    assert_eq!(history.version(), 1);
    let a = combatant_of(&history, 111);
    history.leave_encounter(a).expect("leaves");
    assert_eq!(history.version(), 2);
}

/// B37: records never upgrade a wrapper. A TB-only start with members,
/// release (records written) and restart (records entered) stay at v1.
/// Control: `Joined` in the same script does upgrade.
#[test]
fn b37_records_and_tb_only_cycle_stay_v1() {
    let fixture = tb_only();
    let mut history = HistoryWorld::new(21);
    let m = reg(&mut history, &fixture, 213);
    assert_eq!(history.version(), 1, "registration is a record");
    start(&mut history, &fixture, TB, &[entrant(m, 5)]).expect("starts");
    end_turn(&mut history);
    history.end_encounter().expect("releases");
    assert_eq!(history.world().creatures().len(), 3);
    assert_eq!(
        history.version(),
        1,
        "a release writing records stays at v1"
    );
    let before = mark(&history);
    start(&mut history, &fixture, TB, &[]).expect("restarts with records");
    assert_eq!(history.world().creatures().len(), 1, "A and B entered");
    assert_eq!(
        history.version(),
        1,
        "a restart entering records stays at v1"
    );
    assert!(
        tail(&history, before)
            .iter()
            .all(|event| matches!(event, HistoryEvent::TurnStarted { .. })),
        "Enter participants spawn nothing"
    );
    history.join_encounter(&entrant(m, 40)).expect("joins");
    assert_eq!(history.version(), 2);
}

/// B37: an RT-capable start upgrades at once, members or not, and a failed
/// start does not. Control: the TB start of the same ruleset with a declared
/// clock is RT-capable too.
#[test]
fn b37_rt_start_with_members_upgrades() {
    let fixture = capable_two();
    let mut history = HistoryWorld::new(22);
    let m = reg(&mut history, &fixture, 213);
    let gone = reg(&mut history, &fixture, 214);
    assert!(history.despawn(gone).expect("despawns"));
    assert_eq!(history.version(), 1);
    let snapshot = snap(&history);
    assert!(start(&mut history, &fixture, RT, &[entrant(gone, 1)]).is_err());
    assert_unchanged(&snapshot, &history, "a rejected RT start");
    assert_eq!(history.version(), 1);
    start(&mut history, &fixture, RT, &[entrant(m, 5)]).expect("starts");
    assert_eq!(history.version(), 2);
}

fn forged(version: u64, payloads: &[Value]) -> Value {
    let mut value = serde_json::to_value(HistoryWorld::new(0)).expect("a wrapper");
    value["version"] = json!(version);
    value["next_seq"] = json!(payloads.len() + 1);
    value["pending"] = Value::Array(
        payloads
            .iter()
            .enumerate()
            .map(|(i, payload)| json!({"tick": 0, "seq": i + 1, "payload": payload}))
            .collect(),
    );
    value
}

/// B37: a forged version 1 wrapper holding a `joined` or `left` payload
/// fails `history v1 journal holds a v2 event`. Control: version 2 loads and
/// re-encodes the same journal, and a version 1 wrapper of only `spawned`
/// loads.
#[test]
fn b37_forged_v1_with_v2_payload_fails() {
    let entity = json!({"entity": {"index": 0, "generation": 1}});
    let spawned = json!({"type": "spawned", "value": entity});
    for tag in ["joined", "left"] {
        let v2 = json!({"type": tag, "value": entity});
        for payloads in [vec![v2.clone()], vec![spawned.clone(), v2.clone()]] {
            let error = serde_json::from_value::<HistoryWorld>(forged(1, &payloads))
                .expect_err("v1 may not hold a v2 event");
            assert_eq!(error.to_string(), "history v1 journal holds a v2 event");

            let value = forged(2, &payloads);
            let loaded: HistoryWorld = serde_json::from_value(value.clone()).expect("v2 loads");
            assert_eq!(loaded.version(), 2);
            assert_eq!(serde_json::to_value(&loaded).expect("encodes"), value);
            assert_eq!(loaded.pending_len(), payloads.len());
        }
    }
    let ok: HistoryWorld =
        serde_json::from_value(forged(1, &[spawned])).expect("a v1 spawned journal loads");
    assert_eq!(ok.version(), 1);
}

// --------------------------------------------------------------------- B38

/// One scripted run, hashing the wrapper after every step. `reload_at`
/// saves and reloads the wrapper after that step.
struct Run {
    history: HistoryWorld,
    trace: Vec<[u8; 32]>,
    reload_at: Option<usize>,
}

impl Run {
    fn check(&mut self) {
        let hash = history_hash(&self.history);
        if self.reload_at == Some(self.trace.len()) {
            let bytes = serde_json::to_vec(&self.history).expect("saves");
            let back: HistoryWorld = serde_json::from_slice(&bytes).expect("reloads");
            assert!(back == self.history, "a reload changed the wrapper");
            assert_eq!(history_hash(&back), hash, "a reload changed the hash");
            self.history = back;
        }
        self.trace.push(hash);
    }
}

fn script_fixture(late_join: Option<LateJoinWire>) -> Fixture {
    let mut fixture = capable_two();
    set_hooks(&mut fixture, None, late_join, None);
    set_initiative_rule(&mut fixture, "1d6", &[]);
    fixture
}

/// start(members) -> joins of two schedules -> leaves -> turns -> an RT
/// switch -> ticks -> release -> restart -> turns.
fn run_script(seed: u64, late_join: Option<LateJoinWire>, reload_at: Option<usize>) -> Run {
    let fixture = script_fixture(late_join);
    let mut run = Run {
        history: HistoryWorld::new(seed),
        trace: Vec::new(),
        reload_at,
    };
    let m = reg(&mut run.history, &fixture, 213);
    run.check();
    let j1 = reg(&mut run.history, &fixture, 214);
    run.check();
    let j2 = reg(&mut run.history, &fixture, 215);
    run.check();
    let j3 = reg(&mut run.history, &fixture, 216);
    run.check();
    start(&mut run.history, &fixture, TB, &[entrant(m, 3)]).expect("starts");
    run.check();
    run.history
        .join_encounter(&entrant(j1, 40))
        .expect("late key joins");
    run.check();
    run.history
        .join_encounter(&entrant(j2, -100))
        .expect("early key joins");
    // The early joiner has no timeline entry: it waits, or is pending.
    let pending = run.history.world().combat().expect("encounter");
    assert_eq!(
        pending.pending_joins == vec![j2],
        late_join == Some(LateJoinWire::AfterActive)
    );
    assert!(!timeline_of(run.history.world())
        .iter()
        .any(|&(_, id)| id == j2));
    run.check();
    for _ in 0..3 {
        end_turn(&mut run.history);
        run.check();
    }
    run.history.leave_encounter(j1).expect("j1 leaves");
    run.check();
    run.history
        .join_encounter(&entrant(j3, 0))
        .expect("j3 joins");
    run.check();
    run.history.request_mode(RT).expect("requests RT");
    run.check();
    for _ in 0..40 {
        if run.history.world().combat().expect("encounter").mode == RT {
            break;
        }
        end_turn(&mut run.history);
        run.check();
    }
    assert_eq!(run.history.world().combat().expect("encounter").mode, RT);
    for _ in 0..5 {
        run.history.tick().expect("ticks");
        run.check();
    }
    run.history.leave_encounter(j2).expect("j2 leaves");
    run.check();
    run.history.end_encounter().expect("releases");
    run.check();
    start(&mut run.history, &fixture, TB, &[]).expect("restarts");
    run.check();
    end_turn(&mut run.history);
    run.check();
    run
}

/// B38: two runs of one script give equal per-step `history_hash`
/// sequences, under both late-join policies. Control: a different seed
/// diverges, and the steps themselves change the hash.
#[test]
fn b38_two_runs_give_equal_hash_sequences() {
    for late_join in [None, Some(LateJoinWire::AfterActive)] {
        let first = run_script(30, late_join, None);
        let second = run_script(30, late_join, None);
        assert!(first.trace.len() > 20);
        assert_eq!(first.trace, second.trace);
        assert!(
            first.history == second.history,
            "equal runs end in equal wrappers"
        );
        let other = run_script(31, late_join, None);
        assert_ne!(first.trace.last(), other.trace.last());
        assert_ne!(first.trace[0], first.trace[first.trace.len() - 1]);
    }
}

/// B38: a wrapper saved and reloaded at every step of the script, which
/// includes the steps while a TB joiner waits and while one is pending,
/// continues with equal hashes. Control: the waiting and the pending states
/// really occurred.
#[test]
fn b38_reload_mid_script_continues_equal() {
    for late_join in [None, Some(LateJoinWire::AfterActive)] {
        let baseline = run_script(32, late_join, None);
        for step in 0..baseline.trace.len() {
            let reloaded = run_script(32, late_join, Some(step));
            assert_eq!(reloaded.trace, baseline.trace, "reload after step {step}");
            assert!(reloaded.history == baseline.history);
        }
    }

    // The state saved at step 6 holds the early joiner either waiting
    // (no entry) or pending.
    for (late_join, pending) in [(None, false), (Some(LateJoinWire::AfterActive), true)] {
        let fixture = script_fixture(late_join);
        let mut history = HistoryWorld::new(32);
        let m = reg(&mut history, &fixture, 213);
        let j = reg(&mut history, &fixture, 214);
        start(&mut history, &fixture, TB, &[entrant(m, 3)]).expect("starts");
        history.join_encounter(&entrant(j, -100)).expect("joins");
        let state = history.world().combat().expect("encounter");
        assert_eq!(state.pending_joins == vec![j], pending);
        assert!(!timeline_of(history.world()).iter().any(|&(_, id)| id == j));
        let bytes = serde_json::to_vec(&history).expect("saves");
        let back: HistoryWorld = serde_json::from_slice(&bytes).expect("reloads");
        assert!(back == history);
    }
}

/// B35 + B38: the reloaded journal keeps `joined` and `left` payload bytes
/// (a save with both is byte-stable). Control: editing the stored tag to a
/// different event changes the hash.
#[test]
fn b35_saved_journal_with_joined_and_left_is_byte_stable() {
    let fixture = tb_only();
    let mut history = started(&fixture, 33, TB);
    let j = reg(&mut history, &fixture, 213);
    history.join_encounter(&entrant(j, 30)).expect("joins");
    let a = combatant_of(&history, 111);
    history.leave_encounter(a).expect("leaves");
    let events = tail(&history, 0);
    assert!(events.contains(&HistoryEvent::Joined { entity: j }));
    assert!(events.contains(&HistoryEvent::Left { entity: a }));
    let bytes = serde_json::to_vec(&history).expect("saves");
    let text = String::from_utf8(bytes.clone()).expect("utf8");
    assert!(text.contains(&format!(
        r#"{{"type":"joined","value":{{"entity":{}}}}}"#,
        id_json(j)
    )));
    assert!(text.contains(&format!(
        r#"{{"type":"left","value":{{"entity":{}}}}}"#,
        id_json(a)
    )));
    let back: HistoryWorld = serde_json::from_slice(&bytes).expect("reloads");
    assert_eq!(serde_json::to_vec(&back).expect("re-saves"), bytes);
    assert_eq!(history_hash(&back), history_hash(&history));

    let swapped = text.replacen(r#""type":"left""#, r#""type":"joined""#, 1);
    let tampered: HistoryWorld = serde_json::from_str(&swapped).expect("still loads");
    assert_ne!(history_hash(&tampered), history_hash(&history));
}
