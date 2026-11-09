//! T139c: the `Joined` and `Left` history events and the three journaled
//! wrapper methods (Behaviour B35-B38; tests H1-H5).
//!
//! Every journal below is worked out by hand from the contract: a start
//! journals one `Spawned` per spawned participant (members and entering
//! participants are not spawned) then the initial `TurnStarted`; a join
//! journals exactly one `Joined`; a leave journals `Left` then the transition
//! the leave actually took. Records never upgrade the wrapper (D12).

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Ulid};
use crpg_data::LateJoinWire;
use crpg_sim::{
    history_hash, CombatAction, CombatError, EncounterMode, EntityMeta, Entrant, HistoryError,
    HistoryEvent, HistoryWorld, HISTORY_VERSION, MAX_HISTORY_EVENTS,
};
use serde_json::Value;
use support::creature::{add_outsider, set_hooks};
use support::draws::{draw, oracle, set_initiative_rule};
use support::rt::{capable, capable_trio};
use support::Fixture;

const O: u128 = 131;
const J: u128 = 133;
const K: u128 = 134;
const L: u128 = 135;

fn id(value: u128) -> Ulid {
    Ulid::from_u128(value)
}

fn entrant(entity: EntityId, initiative: i32) -> Entrant {
    Entrant { entity, initiative }
}

/// A wrapper bound to the fixture's area, so that placements register.
fn bound(fixture: &Fixture, seed: u64) -> HistoryWorld {
    HistoryWorld::new_in_area(seed, fixture.area)
}

fn register(fixture: &Fixture, history: &mut HistoryWorld, placement: u128) -> EntityId {
    fixture
        .register_history(history, id(placement))
        .expect("the outsider registers")
}

fn begin_in(
    fixture: &Fixture,
    history: &mut HistoryWorld,
    mode: EncounterMode,
    members: &[Entrant],
) -> Result<(), HistoryError> {
    fixture.with_spec(|spec| history.start_encounter_with_members(spec, mode, members))
}

/// A turn-based start with no members.
fn begin(fixture: &Fixture, history: &mut HistoryWorld) {
    begin_in(fixture, history, EncounterMode::TurnBased, &[]).expect("the encounter starts");
}

fn combatant_ids(history: &HistoryWorld) -> Vec<EntityId> {
    history
        .world()
        .combatants()
        .iter()
        .map(|(entity, _)| entity)
        .collect()
}

/// The retained payloads with a sequence above `after`, oldest first.
fn since(history: &HistoryWorld, after: u64) -> Vec<HistoryEvent> {
    history
        .read_after(after, 256)
        .expect("a page of at most 256 fits")
        .into_iter()
        .map(|envelope| envelope.payload)
        .collect()
}

/// Every retained payload, paging past the 256 cap.
fn all_events(history: &HistoryWorld) -> Vec<HistoryEvent> {
    let mut out = Vec::new();
    let mut after = history.acknowledged();
    loop {
        let page = history.read_after(after, 256).expect("a page reads");
        if page.is_empty() {
            return out;
        }
        after += page.len() as u64;
        out.extend(page.into_iter().map(|envelope| envelope.payload));
    }
}

/// Runs `operation` and returns what it journaled.
fn journaled<R>(
    history: &mut HistoryWorld,
    operation: impl FnOnce(&mut HistoryWorld) -> Result<R, HistoryError>,
) -> Vec<HistoryEvent> {
    let mark = history.last_sequence();
    operation(history).expect("the operation succeeds");
    since(history, mark)
}

fn text(history: &HistoryWorld) -> String {
    serde_json::to_string(history).expect("the wrapper serializes")
}

fn value(history: &HistoryWorld) -> Value {
    serde_json::to_value(history).expect("the wrapper serializes")
}

fn load(value: Value) -> Result<HistoryWorld, serde_json::Error> {
    serde_json::from_value(value)
}

fn turn(actor: EntityId, round: u64) -> HistoryEvent {
    HistoryEvent::TurnStarted { actor, round }
}

fn end_turn(history: &mut HistoryWorld, actor: EntityId) {
    history
        .perform_action(&CombatAction::EndTurn { actor })
        .expect("the turn ends");
}

fn encounter_id() -> Ulid {
    id(101)
}

/// Two distinct entities minted in a scratch wrapper: `(index 0, index 1)`.
fn two_ids() -> (EntityId, EntityId) {
    let mut history = HistoryWorld::new(0);
    let first = history.spawn(EntityMeta {}).expect("spawns");
    let second = history.spawn(EntityMeta {}).expect("spawns");
    (first, second)
}

/// The error text of decoding `input` as a history event.
fn decode_error(input: &str) -> String {
    serde_json::from_str::<HistoryEvent>(input)
        .expect_err("the input must fail")
        .to_string()
}

/// Standard fixture with outsiders J, K and L.
fn outsiders() -> Fixture {
    let mut fixture = Fixture::standard();
    add_outsider(&mut fixture, J, 143);
    add_outsider(&mut fixture, K, 144);
    add_outsider(&mut fixture, L, 145);
    fixture
}

/// A failed operation that must leave the whole wrapper (journal, counters,
/// world, RNG) exactly as it was.
fn assert_rejected<R: std::fmt::Debug>(
    history: &mut HistoryWorld,
    expected: CombatError,
    operation: impl FnOnce(&mut HistoryWorld) -> Result<R, HistoryError>,
) {
    let before = history.clone();
    let hash = history_hash(history);
    let error = operation(history).expect_err("the operation is rejected");
    assert_eq!(error, HistoryError::Combat(expected));
    assert_eq!(*history, before, "a rejection committed something");
    assert_eq!(history_hash(history), hash);
}

// ---------------------------------------------------------------- H1

/// H1 (B35): exact `joined` and `left` bytes, round trips, and the key faults.
///
/// **Fails if** a tag is misspelled or swapped, the payload key is not
/// `entity`, the encoding drops the generation, or either decoder arm accepts
/// a duplicate, unknown or missing key or a `value` before `type`.
#[test]
fn h1_payloads() {
    let (first, second) = two_ids();
    assert_eq!((first.index(), first.generation()), (0, 1));
    assert_eq!((second.index(), second.generation()), (1, 1));
    let cases = [
        (
            HistoryEvent::Joined { entity: second },
            r#"{"type":"joined","value":{"entity":{"index":1,"generation":1}}}"#,
        ),
        (
            HistoryEvent::Left { entity: second },
            r#"{"type":"left","value":{"entity":{"index":1,"generation":1}}}"#,
        ),
        (
            HistoryEvent::Joined { entity: first },
            r#"{"type":"joined","value":{"entity":{"index":0,"generation":1}}}"#,
        ),
        (
            HistoryEvent::Left { entity: first },
            r#"{"type":"left","value":{"entity":{"index":0,"generation":1}}}"#,
        ),
    ];
    for (event, expected) in cases {
        let encoded = serde_json::to_string(&event).expect("encodes");
        assert_eq!(encoded, expected);
        let decoded: HistoryEvent = serde_json::from_str(expected).expect("decodes");
        assert_eq!(decoded, event, "round trip for {expected}");
        assert_eq!(
            serde_json::to_string(&decoded).expect("re-encodes"),
            expected
        );
    }
    // The two tags are distinct events.
    assert_ne!(
        HistoryEvent::Joined { entity: second },
        HistoryEvent::Left { entity: second }
    );

    // The four key faults, for both tags.
    let entity = r#"{"index":1,"generation":1}"#;
    for tag in ["joined", "left"] {
        let wrap = |body: &str| format!("{{\"type\":\"{tag}\",\"value\":{{{body}}}}}");
        serde_json::from_str::<HistoryEvent>(&wrap(&format!("\"entity\":{entity}")))
            .expect("the valid shape decodes");
        let duplicate = decode_error(&wrap(&format!("\"entity\":{entity},\"entity\":{entity}")));
        assert!(
            duplicate.contains("duplicate history payload field `entity`"),
            "{tag}: {duplicate}"
        );
        let unknown = decode_error(&wrap(&format!("\"entity\":{entity},\"surprise\":1")));
        assert!(
            unknown.contains("unknown history payload field"),
            "{tag}: {unknown}"
        );
        // A key of a neighbouring payload is unknown here.
        let wrong = decode_error(&wrap(&format!("\"actor\":{entity}")));
        assert!(
            wrong.contains("unknown history payload field"),
            "{tag}: {wrong}"
        );
        let missing = decode_error(&wrap(""));
        assert!(
            missing.contains("history payload misses `entity`"),
            "{tag}: {missing}"
        );
        let reversed = format!("{{\"value\":{{\"entity\":{entity}}},\"type\":\"{tag}\"}}");
        let message = decode_error(&reversed);
        assert!(
            message.contains("requires `type` before `value`"),
            "{tag}: {message}"
        );
        let trailing =
            format!("{{\"type\":\"{tag}\",\"value\":{{\"entity\":{entity}}},\"extra\":1}}");
        assert!(decode_error(&trailing).contains("unknown history payload field"));
    }
    // A payload key may stay in any order around `entity`, and an unknown
    // tag keeps its existing message.
    assert!(decode_error(r#"{"type":"joins","value":{}}"#).contains("unknown history event type"));

    // Control: the `spawned` encoding is unchanged.
    for (event, expected) in [
        (
            HistoryEvent::Spawned { entity: second },
            r#"{"type":"spawned","value":{"entity":{"index":1,"generation":1}}}"#,
        ),
        (
            HistoryEvent::Despawned { entity: second },
            r#"{"type":"despawned","value":{"entity":{"index":1,"generation":1}}}"#,
        ),
        (
            HistoryEvent::Died { entity: second },
            r#"{"type":"died","value":{"entity":{"index":1,"generation":1}}}"#,
        ),
    ] {
        assert_eq!(serde_json::to_string(&event).expect("encodes"), expected);
        assert_eq!(
            serde_json::from_str::<HistoryEvent>(expected).expect("decodes"),
            event
        );
    }

    // In a journal the payload sits inside the envelope with its tick and
    // sequence.
    let fixture = outsiders();
    let mut history = bound(&fixture, 5);
    let j = register(&fixture, &mut history, J);
    begin(&fixture, &mut history);
    let mark = history.last_sequence();
    history.join_encounter(&entrant(j, 12)).expect("J joins");
    let envelope = &history.read_after(mark, 1).expect("reads")[0];
    let expected = format!(
        "{{\"tick\":{},\"seq\":{},\"payload\":{{\"type\":\"joined\",\"value\":{{\"entity\":{{\"index\":{},\"generation\":{}}}}}}}}}",
        history.world().tick().get(),
        mark + 1,
        j.index(),
        j.generation()
    );
    assert_eq!(serde_json::to_string(envelope).expect("encodes"), expected);
}

// ---------------------------------------------------------------- H2

/// H2 (B36), starts: a member journals no event, and neither does an entering
/// participant; the initial `TurnStarted` is journaled in TB and absent in RT.
///
/// **Fails if** a member or an entering participant is journaled as
/// `Spawned` or `Joined`, the initial turn is missing or names another head,
/// an RT start journals a turn, or an empty `members` differs from
/// `start_encounter_in_mode`.
fn h2_emission_orders_starts() {
    // TB with a member keyed ahead of both authored participants.
    let mut fixture = Fixture::standard();
    add_outsider(&mut fixture, O, 141);
    let mut history = bound(&fixture, 5);
    let o = register(&fixture, &mut history, O);
    assert_eq!((o.index(), o.generation()), (0, 1));
    let events = journaled(&mut history, |history| {
        begin_in(
            &fixture,
            history,
            EncounterMode::TurnBased,
            &[entrant(o, 5)],
        )
    });
    let ids = combatant_ids(&history);
    assert_eq!(ids.len(), 3);
    assert!(ids.contains(&o), "the member is a combatant");
    let spawned: Vec<EntityId> = ids.iter().copied().filter(|entity| *entity != o).collect();
    let (a, b) = (spawned[0], spawned[1]);
    assert_eq!((a.index(), b.index()), (1, 2));
    assert_eq!(
        events,
        [
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
            turn(o, 0),
        ],
        "the member is keyed at 5, so it is the head, and journals nothing"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            HistoryEvent::Spawned { entity } | HistoryEvent::Joined { entity } if *entity == o
        )),
        "a member is neither spawned nor joined"
    );

    // Control: no members, the same fixture: the head is B (9).
    let mut plain = bound(&fixture, 5);
    register(&fixture, &mut plain, O);
    let plain_events = journaled(&mut plain, |history| {
        begin_in(&fixture, history, EncounterMode::TurnBased, &[])
    });
    let plain_ids = combatant_ids(&plain);
    assert_eq!(plain_ids.len(), 2);
    assert_eq!(
        plain_events,
        [
            HistoryEvent::Spawned {
                entity: plain_ids[0]
            },
            HistoryEvent::Spawned {
                entity: plain_ids[1]
            },
            turn(plain_ids[1], 0),
        ]
    );
    // Control: an empty `members` is `start_encounter_in_mode` exactly.
    let mut legacy = bound(&fixture, 5);
    register(&fixture, &mut legacy, O);
    fixture
        .with_spec(|spec| legacy.start_encounter_in_mode(spec, EncounterMode::TurnBased))
        .expect("the legacy start");
    assert_eq!(legacy, plain);
    assert_eq!(history_hash(&legacy), history_hash(&plain));

    // RT with a member: the Spawned events only; no schedule, no turn.
    let mut rt = capable(8);
    add_outsider(&mut rt, O, 141);
    let mut history = bound(&rt, 5);
    let o = register(&rt, &mut history, O);
    let events = journaled(&mut history, |history| {
        begin_in(&rt, history, EncounterMode::RealTime, &[entrant(o, 5)])
    });
    let ids = combatant_ids(&history);
    assert_eq!(ids.len(), 3);
    let spawned: Vec<EntityId> = ids.iter().copied().filter(|entity| *entity != o).collect();
    assert_eq!(
        events,
        [
            HistoryEvent::Spawned { entity: spawned[0] },
            HistoryEvent::Spawned { entity: spawned[1] },
        ]
    );

    // An authored Enter participant: A's placement already holds a record, so
    // A enters from it and only B is spawned.
    let fixture = Fixture::standard();
    let mut history = bound(&fixture, 5);
    let a = register(&fixture, &mut history, 111);
    let events = journaled(&mut history, |history| {
        begin_in(&fixture, history, EncounterMode::TurnBased, &[])
    });
    let ids = combatant_ids(&history);
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], a, "A kept its entity");
    assert!(history.world().combatants().get(a).expect("A").entered());
    let b = ids[1];
    assert_eq!(
        events,
        [HistoryEvent::Spawned { entity: b }, turn(b, 0)],
        "no Spawned for the entering participant"
    );
    assert!(!events.contains(&HistoryEvent::Spawned { entity: a }));
}

/// H2 (B36), joins: exactly one `Joined` for every schedule, and for a rolled
/// key; a rejection commits nothing.
///
/// **Fails if** a join journals nothing, two events, a transition, or the
/// wrong entity, or a rejected join commits a sequence or a draw.
fn h2_emission_orders_joins() {
    let fixture = outsiders();
    let mut history = bound(&fixture, 5);
    let j = register(&fixture, &mut history, J);
    let k = register(&fixture, &mut history, K);
    begin(&fixture, &mut history);

    // After the head: inserted at once.
    let mark = history.last_sequence();
    history.join_encounter(&entrant(j, 12)).expect("J joins");
    assert_eq!(history.last_sequence(), mark + 1);
    assert!(history.world().timeline().contains(j));
    let page = history.read_after(mark, 8).expect("reads");
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].payload, HistoryEvent::Joined { entity: j });
    assert_eq!(page[0].seq, mark + 1);
    assert_eq!(page[0].tick, history.world().tick());

    // Before the head, default `late_join`: it waits, unscheduled, and still
    // journals the same single event.
    let events = journaled(&mut history, |history| {
        history.join_encounter(&entrant(k, 3))
    });
    assert!(!history.world().timeline().contains(k));
    assert_eq!(events, [HistoryEvent::Joined { entity: k }]);

    // A rolled key adds no event.
    let mut rolled = outsiders();
    set_initiative_rule(&mut rolled, "1d6", &["alpha"]);
    let mut history = bound(&rolled, 5);
    let j = register(&rolled, &mut history, J);
    begin(&rolled, &mut history);
    let events = journaled(&mut history, |history| {
        history.join_encounter(&entrant(j, 100))
    });
    assert_eq!(events, [HistoryEvent::Joined { entity: j }]);

    // RT: keyed at the elapsed tick, one `Joined`.
    let mut rt = capable(8);
    add_outsider(&mut rt, J, 143);
    let mut history = bound(&rt, 5);
    let j = register(&rt, &mut history, J);
    begin_in(&rt, &mut history, EncounterMode::RealTime, &[]).expect("starts");
    history.tick().expect("ticks");
    history.tick().expect("ticks");
    let events = journaled(&mut history, |history| {
        history.join_encounter(&entrant(j, 4))
    });
    assert_eq!(events, [HistoryEvent::Joined { entity: j }]);
    assert!(history.world().timeline().contains(j));

    // Control: rejections commit nothing.
    let fixture = outsiders();
    let mut idle = bound(&fixture, 5);
    let j = register(&fixture, &mut idle, J);
    assert_rejected(&mut idle, CombatError::NoEncounter, |history| {
        history.join_encounter(&entrant(j, 12))
    });
    let mut live = bound(&fixture, 5);
    let j = register(&fixture, &mut live, J);
    begin(&fixture, &mut live);
    let a = combatant_ids(&live)[0];
    assert_rejected(
        &mut live,
        CombatError::AlreadyParticipant { entity: a },
        |history| history.join_encounter(&entrant(a, 12)),
    );
    let plain = live.spawn(EntityMeta {}).expect("spawns");
    assert_rejected(
        &mut live,
        CombatError::NotCreature { entity: plain },
        |history| history.join_encounter(&entrant(plain, 12)),
    );
    // Control: the same wrapper then joins J.
    let events = journaled(&mut live, |history| history.join_encounter(&entrant(j, 12)));
    assert_eq!(events, [HistoryEvent::Joined { entity: j }]);
}

/// A trio world: C (8) heads, then B (9), then A (10).
fn trio_history(fixture: &Fixture) -> (HistoryWorld, [EntityId; 3]) {
    let mut history = bound(fixture, 5);
    begin(fixture, &mut history);
    let ids = combatant_ids(&history);
    let [a, b, c] = [ids[0], ids[1], ids[2]];
    assert_eq!(history.world().combat().expect("combat").active, Some(c));
    (history, [a, b, c])
}

/// H2 (B36), leaves: `Left`, then the transition the leave took.
///
/// **Fails if** `Left` is missing or comes after the transition, a leave
/// journals `Died` or `Despawned`, the head's leave misses `TurnStarted`, the
/// last of a nonterminal encounter misses `EncounterEnded`, a terminal one
/// journals a second end, or a pending switch applied by the leave is not
/// journaled in `ModeChanged`, `RoundStarted` order.
fn h2_emission_orders_leaves() {
    let fixture = Fixture::trio();
    let (base, [a, b, c]) = trio_history(&fixture);
    let encounter = encounter_id();

    // A non-head leaves: `Left` only.
    let mut history = base.clone();
    let events = journaled(&mut history, |history| history.leave_encounter(b));
    assert_eq!(events, [HistoryEvent::Left { entity: b }]);
    assert!(history.world().contains(b), "the entity survives");
    assert!(history.world().combatants().get(b).is_none());

    // The head leaves: `Left`, then the next head's turn in the same round.
    let mut history = base.clone();
    let events = journaled(&mut history, |history| history.leave_encounter(c));
    assert_eq!(events, [HistoryEvent::Left { entity: c }, turn(b, 0)]);

    // Control: a despawn of the same head journals `Despawned` and the same
    // turn, never `Left`.
    let mut despawned = base.clone();
    let events = journaled(&mut despawned, |history| history.despawn(c));
    assert_eq!(events, [HistoryEvent::Despawned { entity: c }, turn(b, 0)]);

    // The last combatant of a nonterminal encounter: `EncounterEnded`.
    let mut history = base.clone();
    assert_eq!(
        journaled(&mut history, |history| history.leave_encounter(a)),
        [HistoryEvent::Left { entity: a }]
    );
    assert_eq!(
        journaled(&mut history, |history| history.leave_encounter(b)),
        [HistoryEvent::Left { entity: b }]
    );
    assert!(history.world().combat().is_some());
    let events = journaled(&mut history, |history| history.leave_encounter(c));
    assert_eq!(
        events,
        [
            HistoryEvent::Left { entity: c },
            HistoryEvent::EncounterEnded {
                encounter,
                round: 0
            },
        ]
    );
    assert!(history.world().combat().is_none());

    // A terminal encounter: the head's leave ends it (one alive remains),
    // and the survivor's later leave journals `Left` and nothing more.
    let mut history = base.clone();
    assert_eq!(
        journaled(&mut history, |history| history.leave_encounter(c)),
        [HistoryEvent::Left { entity: c }, turn(b, 0)]
    );
    assert_eq!(
        journaled(&mut history, |history| history.leave_encounter(b)),
        [
            HistoryEvent::Left { entity: b },
            HistoryEvent::EncounterEnded {
                encounter,
                round: 0
            },
        ]
    );
    assert_eq!(history.world().combat().expect("combat").active, None);
    let events = journaled(&mut history, |history| history.leave_encounter(a));
    assert_eq!(
        events,
        [HistoryEvent::Left { entity: a }],
        "no second end for a terminal encounter"
    );

    // RT: a leave that drops the alive count below two ends the encounter.
    let rt = capable(8);
    let mut history = bound(&rt, 5);
    begin_in(&rt, &mut history, EncounterMode::RealTime, &[]).expect("starts");
    let leaver = combatant_ids(&history)[0];
    let events = journaled(&mut history, |history| history.leave_encounter(leaver));
    assert_eq!(
        events,
        [
            HistoryEvent::Left { entity: leaver },
            HistoryEvent::EncounterEnded {
                encounter,
                round: 0
            },
        ]
    );
}

/// H2 (B36): a head whose leave rolls over into a pending RT switch
/// journals `[Left, ModeChanged, RoundStarted]`.
///
/// **Fails if** the switch is dropped, comes before `Left`, or is journaled
/// as `RoundStarted` before `ModeChanged`.
fn h2_leave_rolls_over_into_a_pending_switch() {
    let trio = capable_trio(8);
    let mut history = bound(&trio, 5);
    begin(&trio, &mut history);
    let [a, b, c] = {
        let ids = combatant_ids(&history);
        [ids[0], ids[1], ids[2]]
    };
    end_turn(&mut history, c);
    end_turn(&mut history, b);
    assert_eq!(history.world().combat().expect("combat").active, Some(a));
    history
        .request_mode(EncounterMode::RealTime)
        .expect("requested");
    let events = journaled(&mut history, |history| history.leave_encounter(a));
    assert_eq!(
        events,
        [
            HistoryEvent::Left { entity: a },
            HistoryEvent::ModeChanged {
                mode: EncounterMode::RealTime,
                round: 1
            },
            HistoryEvent::RoundStarted { round: 1 },
        ]
    );
    assert_eq!(
        history.world().combat().expect("combat").mode,
        EncounterMode::RealTime
    );
}

/// H2 (B36): a head whose leave flushes a pending joiner J journals
/// `[Left, TurnStarted { J, 0 }]`.
///
/// **Fails if** the flushed joiner's turn is journaled with the displaced
/// head's name, with the wrong round, or not at all.
fn h2_leave_flushes_a_pending_joiner() {
    let mut fixture = outsiders();
    set_hooks(&mut fixture, None, Some(LateJoinWire::AfterActive), None);
    let mut history = bound(&fixture, 5);
    let j = register(&fixture, &mut history, J);
    begin(&fixture, &mut history);
    let ids = combatant_ids(&history);
    let b = ids[1];
    assert_eq!(history.world().combat().expect("combat").active, Some(b));
    let events = journaled(&mut history, |history| {
        history.join_encounter(&entrant(j, 3))
    });
    assert_eq!(events, [HistoryEvent::Joined { entity: j }]);
    assert_eq!(history.world().combat().expect("combat").pending_joins, [j]);

    let events = journaled(&mut history, |history| history.leave_encounter(b));
    assert_eq!(
        events,
        [HistoryEvent::Left { entity: b }, turn(j, 0)],
        "the leave flushed J into the head"
    );
    assert!(history
        .world()
        .combat()
        .expect("combat")
        .pending_joins
        .is_empty());
    assert_eq!(history.world().combat().expect("combat").active, Some(j));

    // Control: under the default rule the same leave names the next in line.
    let plain = outsiders();
    let mut history = bound(&plain, 5);
    let j = register(&plain, &mut history, J);
    begin(&plain, &mut history);
    let ids = combatant_ids(&history);
    let (a, b) = (ids[0], ids[1]);
    history.join_encounter(&entrant(j, 3)).expect("J joins");
    let events = journaled(&mut history, |history| history.leave_encounter(b));
    assert_eq!(
        events,
        [HistoryEvent::Left { entity: b }, turn(a, 0)],
        "J waits for the rollover, so the next in line is A"
    );
}

/// H2 (B36): `end_encounter` with records written still journals nothing.
///
/// **Fails if** a release journals an `EncounterEnded`, a `Left` per
/// combatant, or consumes a sequence.
fn h2_end_encounter_journals_nothing_with_records_written() {
    let fixture = outsiders();
    let mut history = bound(&fixture, 5);
    let j = register(&fixture, &mut history, J);
    begin(&fixture, &mut history);
    history.join_encounter(&entrant(j, 12)).expect("J joins");
    let ids = combatant_ids(&history);
    assert!(history.world().creatures().is_empty(), "nothing yet");
    let (pending, last, version) = (
        history.pending_len(),
        history.last_sequence(),
        history.version(),
    );
    let events = journaled(&mut history, |history| history.end_encounter());
    assert!(events.is_empty());
    for entity in &ids {
        assert!(
            history.world().creatures().get(*entity).is_some(),
            "the release wrote a record for every survivor"
        );
    }
    assert_eq!(history.pending_len(), pending);
    assert_eq!(history.last_sequence(), last);
    assert_eq!(history.version(), version);
    assert!(history.world().combat().is_none());
}

/// H2 control: every start and leave rejection commits nothing.
///
/// **Fails if** a rejected start, leave or member check journals, advances
/// the sequence, spawns, or leaves a staged change behind.
fn h2_rejections_commit_nothing() {
    let mut fixture = outsiders();
    add_outsider(&mut fixture, O, 141);
    // Start rejections.
    let mut fresh = bound(&fixture, 5);
    let gone = fresh.spawn(EntityMeta {}).expect("spawns");
    fresh.despawn(gone).expect("despawns");
    assert_rejected(
        &mut fresh,
        CombatError::AbsentActor { actor: gone },
        |history| {
            begin_in(
                &fixture,
                history,
                EncounterMode::TurnBased,
                &[entrant(gone, 5)],
            )
        },
    );
    let mut live = bound(&fixture, 5);
    let o = register(&fixture, &mut live, O);
    begin_in(
        &fixture,
        &mut live,
        EncounterMode::TurnBased,
        &[entrant(o, 5)],
    )
    .expect("starts");
    assert_rejected(&mut live, CombatError::EncounterActive, |history| {
        begin_in(&fixture, history, EncounterMode::TurnBased, &[])
    });
    // Leave rejections.
    let mut idle = bound(&fixture, 5);
    let j = register(&fixture, &mut idle, J);
    assert_rejected(&mut idle, CombatError::NoEncounter, |history| {
        history.leave_encounter(j)
    });
    let mut live = bound(&fixture, 5);
    let j = register(&fixture, &mut live, J);
    begin(&fixture, &mut live);
    assert_rejected(
        &mut live,
        CombatError::NotParticipant { entity: j },
        |history| history.leave_encounter(j),
    );
    let gone = live.spawn(EntityMeta {}).expect("spawns");
    live.despawn(gone).expect("despawns");
    assert_rejected(
        &mut live,
        CombatError::AbsentActor { actor: gone },
        |history| history.leave_encounter(gone),
    );
    // Control: a participant leaves.
    let a = combatant_ids(&live)[0];
    assert_eq!(
        journaled(&mut live, |history| history.leave_encounter(a)),
        [HistoryEvent::Left { entity: a }]
    );
}

/// H2 (B36): the emission orders of the three wrapper methods, and the
/// control that each rejection commits nothing. The cases are plain
/// functions above, run in contract order.
#[test]
fn h2_emission_orders() {
    h2_emission_orders_starts();
    h2_emission_orders_joins();
    h2_emission_orders_leaves();
    h2_leave_rolls_over_into_a_pending_switch();
    h2_leave_flushes_a_pending_joiner();
    h2_end_encounter_journals_nothing_with_records_written();
    h2_rejections_commit_nothing();
}

// ---------------------------------------------------------------- H3

/// H3 (B37): a join upgrades a version-1 wrapper, and so does a leave;
/// a rejected join does not; the upgrade is sticky.
///
/// **Fails if** `Joined` or `Left` is missing from `is_v2`, a rejected
/// operation upgrades, or the wrapper goes back.
fn h3_version_upgrades_on_join_and_leave() {
    let fixture = outsiders();
    let mut history = bound(&fixture, 5);
    let j = register(&fixture, &mut history, J);
    begin(&fixture, &mut history);
    assert_eq!(history.version(), 1, "a TB-only start stays at version 1");
    assert!(text(&history).starts_with("{\"version\":1,"));
    // A rejected join upgrades nothing.
    let a = combatant_ids(&history)[0];
    assert!(history.join_encounter(&entrant(a, 1)).is_err());
    assert_eq!(history.version(), 1);
    let left_over = history.clone();
    history.join_encounter(&entrant(j, 12)).expect("J joins");
    assert_eq!(history.version(), 2);
    assert_eq!(HISTORY_VERSION, 3);
    assert!(text(&history).starts_with("{\"version\":2,"));
    // Control: the same operations on the clone give a different version only
    // through the join.
    assert_eq!(left_over.version(), 1);

    // A leave upgrades on its own.
    let mut leaver = left_over.clone();
    let b = combatant_ids(&leaver)[1];
    leaver.leave_encounter(b).expect("B leaves");
    assert_eq!(leaver.version(), 2);

    // Sticky: acknowledgement and release do not go back.
    history
        .acknowledge(history.last_sequence())
        .expect("acknowledges");
    assert_eq!(history.pending_len(), 0);
    history.end_encounter().expect("releases");
    assert_eq!(history.version(), 2);
    let reloaded: HistoryWorld = serde_json::from_str(&text(&history)).expect("reloads");
    assert_eq!(reloaded.version(), 2);
    assert_eq!(reloaded, history);
}

/// H3 (B37) control: records never upgrade (D12). A TB-only start with a
/// member, a release that writes records, and a restart that enters from
/// them stay at version 1.
///
/// **Fails if** `commit` upgrades on records, or the loader's v1 block
/// rejects a version-1 wrapper holding records.
fn h3_records_never_upgrade() {
    let mut fixture = Fixture::standard();
    add_outsider(&mut fixture, O, 141);
    let mut history = bound(&fixture, 5);
    let o = register(&fixture, &mut history, O);
    assert!(history.world().creatures().get(o).is_some());
    assert_eq!(history.version(), 1, "a record alone does not upgrade");
    begin_in(
        &fixture,
        &mut history,
        EncounterMode::TurnBased,
        &[entrant(o, 5)],
    )
    .expect("starts with a member");
    assert_eq!(history.version(), 1);
    assert!(history.world().creatures().is_empty(), "records consumed");
    history.end_encounter().expect("releases");
    assert!(
        history.world().creatures().get(o).is_some(),
        "the release wrote records"
    );
    assert_eq!(history.version(), 1);
    // The restart enters every participant from a record.
    let events = journaled(&mut history, |history| {
        begin_in(
            &fixture,
            history,
            EncounterMode::TurnBased,
            &[entrant(o, 5)],
        )
    });
    assert_eq!(events, [turn(o, 0)], "everyone entered; nobody spawned");
    assert_eq!(history.version(), 1);
    assert!(text(&history).starts_with("{\"version\":1,"));
    // A version-1 wrapper holding records loads and is unchanged.
    history.end_encounter().expect("releases");
    assert!(!history.world().creatures().is_empty());
    let loaded: HistoryWorld = serde_json::from_str(&text(&history)).expect("v1 loads");
    assert_eq!(loaded, history);
    assert_eq!(loaded.version(), 1);
    assert_eq!(history_hash(&loaded), history_hash(&history));
}

/// H3 (B37): a forged version-1 wrapper retaining a `joined` or `left`
/// payload fails the loader's v1 check, on both loader paths.
///
/// **Fails if** the loader's v1 block does not see either variant as v2.
fn h3_forged_version_one_is_rejected() {
    let fixture = outsiders();
    for variant in ["joined", "left"] {
        let mut history = bound(&fixture, 5);
        let j = register(&fixture, &mut history, J);
        begin(&fixture, &mut history);
        // Only the variant under test is retained: acknowledge the rest.
        history
            .acknowledge(history.last_sequence())
            .expect("acknowledges");
        if variant == "joined" {
            history.join_encounter(&entrant(j, 12)).expect("J joins");
        } else {
            let a = combatant_ids(&history)[0];
            history.leave_encounter(a).expect("A leaves");
        }
        let pending = since(&history, history.acknowledged());
        assert_eq!(pending.len(), 1);
        assert!(
            matches!(
                (&pending[0], variant),
                (HistoryEvent::Joined { .. }, "joined") | (HistoryEvent::Left { .. }, "left")
            ),
            "{variant}: {pending:?}"
        );
        assert_eq!(history.version(), 2);
        // Control: the genuine version-2 wrapper loads.
        assert_eq!(load(value(&history)).expect("v2 loads"), history);

        let mut forged = value(&history);
        forged["version"] = Value::from(1);
        let message = load(forged).expect_err("v1 with a v2 event").to_string();
        assert!(
            message.contains("history v1 journal holds a v2 event"),
            "{variant}: {message}"
        );
        let forged = text(&history).replacen("\"version\":2", "\"version\":1", 1);
        let message = serde_json::from_str::<HistoryWorld>(&forged)
            .expect_err("v1 with a v2 event")
            .to_string();
        assert!(
            message.contains("history v1 journal holds a v2 event"),
            "{variant}: {message}"
        );
    }
}

/// H3 (B37): the version rules, with the records control and the forged
/// version-1 rejection. The cases are plain functions above.
#[test]
fn h3_version() {
    h3_version_upgrades_on_join_and_leave();
    h3_records_never_upgrade();
    h3_forged_version_one_is_rejected();
}

// ---------------------------------------------------------------- H4

/// Fills the journal with spawns until it holds `count` envelopes.
fn fill_to(history: &mut HistoryWorld, count: usize) {
    while history.pending_len() < count {
        history.spawn(EntityMeta {}).expect("the journal fills");
    }
}

/// H4 (B36): a rolled join on a full journal fails `EventLimit` and commits
/// nothing, not even its draw; after one acknowledgement it succeeds and
/// draws exactly once. A leave on a full journal behaves the same.
///
/// **Fails if** the staged RNG is published on failure, the join commits
/// before the journal check, or a failed leave removes the combatant.
#[test]
fn h4_event_limit_rollback() {
    let mut fixture = outsiders();
    set_initiative_rule(&mut fixture, "1d6", &["alpha"]);
    let mut history = bound(&fixture, 5);
    let j = register(&fixture, &mut history, J);
    begin(&fixture, &mut history);
    fill_to(&mut history, MAX_HISTORY_EVENTS);
    assert_eq!(history.pending_len(), MAX_HISTORY_EVENTS);

    let before = history.clone();
    let rng_before = history.world().clone().rng_mut().clone();
    let error = history
        .join_encounter(&entrant(j, 100))
        .expect_err("the journal is full");
    assert_eq!(error, HistoryError::EventLimit);
    assert_eq!(history, before, "nothing was committed");
    assert_eq!(history_hash(&history), history_hash(&before));
    let mut rng_after = history.world().clone().rng_mut().clone();
    assert_eq!(rng_after, rng_before);
    assert!(history.world().creatures().get(j).is_some());
    assert!(history.world().combatants().get(j).is_none());
    // The next draw is the one the failed join would have taken.
    let mut expected = oracle(5);
    draw(&mut expected, "1d6");
    draw(&mut expected, "1d6");
    let first = draw(&mut expected.clone(), "1d6").total;
    assert_eq!(draw(&mut rng_after, "1d6").total, first);
    assert_eq!(rng_before, expected, "only the start drew");

    // Control: one acknowledgement frees a slot, and the join then succeeds
    // with exactly one draw.
    history
        .acknowledge(history.acknowledged() + 1)
        .expect("acknowledges");
    let events = journaled(&mut history, |history| {
        history.join_encounter(&entrant(j, 100))
    });
    assert_eq!(events, [HistoryEvent::Joined { entity: j }]);
    draw(&mut expected, "1d6");
    assert_eq!(*history.world().clone().rng_mut(), expected);

    // A leave on a full journal fails the same way and keeps the combatant.
    fill_to(&mut history, MAX_HISTORY_EVENTS);
    let b = j;
    let before = history.clone();
    let error = history.leave_encounter(b).expect_err("the journal is full");
    assert_eq!(error, HistoryError::EventLimit);
    assert_eq!(history, before);
    assert!(history.world().combatants().get(b).is_some());
    history
        .acknowledge(history.acknowledged() + 1)
        .expect("acknowledges");
    let events = journaled(&mut history, |history| history.leave_encounter(b));
    assert_eq!(events, [HistoryEvent::Left { entity: b }]);

    // A gameplay error wins over the full journal.
    fill_to(&mut history, MAX_HISTORY_EVENTS);
    let before = history.clone();
    let a = combatant_ids(&history)[0];
    let error = history
        .join_encounter(&entrant(a, 100))
        .expect_err("A is already a participant");
    assert_eq!(
        error,
        HistoryError::Combat(CombatError::AlreadyParticipant { entity: a })
    );
    assert_eq!(history, before);
}

// ---------------------------------------------------------------- H5

fn script_fixture(rt: bool) -> Fixture {
    let mut fixture = if rt { capable(4) } else { Fixture::standard() };
    set_hooks(&mut fixture, None, Some(LateJoinWire::AfterActive), None);
    support::draws::set_damage(
        &mut fixture,
        serde_json::json!([
            {"outcome": {"type": "success"}, "amount": 1},
            {"outcome": {"type": "failure"}, "amount": 0}
        ]),
    );
    add_outsider(&mut fixture, O, 141);
    add_outsider(&mut fixture, J, 143);
    add_outsider(&mut fixture, K, 144);
    add_outsider(&mut fixture, L, 145);
    fixture
}

/// The deterministic step of the script: the active combatant (TB) or every
/// orderless combatant (RT) attacks the first other living combatant; a TB
/// actor that cannot ends its turn; with no turn, the wrapper ticks.
fn step(history: &mut HistoryWorld, ability: Ulid) {
    let Some(combat) = history.world().combat() else {
        history.tick().expect("ticks");
        return;
    };
    let (mode, active) = (combat.mode, combat.active);
    let living: Vec<EntityId> = history
        .world()
        .combatants()
        .iter()
        .filter(|(_, combatant)| !combatant.dead())
        .map(|(entity, _)| entity)
        .collect();
    let target_for = |actor: EntityId| living.iter().copied().find(|other| *other != actor);
    let use_ability = |actor, target| CombatAction::UseAbility {
        actor,
        ability,
        target,
    };
    match (mode, active) {
        (EncounterMode::TurnBased, Some(actor)) => {
            let used = target_for(actor)
                .is_some_and(|target| history.perform_action(&use_ability(actor, target)).is_ok());
            if !used {
                let _ = history.perform_action(&CombatAction::EndTurn { actor });
            }
        }
        (EncounterMode::TurnBased, None) => history.tick().expect("ticks"),
        (EncounterMode::RealTime, _) => {
            for actor in living.clone() {
                let orderless = history
                    .world()
                    .combatants()
                    .get(actor)
                    .is_some_and(|combatant| combatant.order().is_none());
                if let (true, Some(target)) = (orderless, target_for(actor)) {
                    let _ = history.perform_action(&use_ability(actor, target));
                }
            }
            history.tick().expect("ticks");
        }
    }
}

/// What a script did, so a vacuous run cannot pass.
#[derive(Debug, Default, PartialEq, Eq)]
struct Done {
    joins: usize,
    leaves: usize,
}

/// start(members) -> join (each schedule) -> leave -> 30 steps -> release,
/// hashing the wrapper after every step.
fn script(seed: u64, rt: bool) -> (Vec<[u8; 32]>, Done, HistoryWorld) {
    let fixture = script_fixture(rt);
    let mut history = bound(&fixture, seed);
    let mut hashes = Vec::new();
    let mut done = Done::default();
    let o = register(&fixture, &mut history, O);
    let j1 = register(&fixture, &mut history, J);
    let j2 = register(&fixture, &mut history, K);
    let j3 = register(&fixture, &mut history, L);
    hashes.push(history_hash(&history));
    let mode = if rt {
        EncounterMode::RealTime
    } else {
        EncounterMode::TurnBased
    };
    begin_in(&fixture, &mut history, mode, &[entrant(o, 5)]).expect("starts");
    hashes.push(history_hash(&history));
    if rt {
        history.tick().expect("ticks");
        history.tick().expect("ticks");
        hashes.push(history_hash(&history));
    }
    history.join_encounter(&entrant(j1, 20)).expect("J1 joins");
    done.joins += 1;
    hashes.push(history_hash(&history));
    history.join_encounter(&entrant(j2, 3)).expect("J2 joins");
    done.joins += 1;
    hashes.push(history_hash(&history));
    let mut joined_late = false;
    for index in 0..30 {
        step(&mut history, fixture.ability);
        hashes.push(history_hash(&history));
        if index == 4
            && history
                .world()
                .combatants()
                .get(j1)
                .is_some_and(|combatant| !combatant.dead())
        {
            history.leave_encounter(j1).expect("J1 leaves");
            done.leaves += 1;
            hashes.push(history_hash(&history));
        }
        let late_round = history.world().combat().is_some_and(|combat| {
            combat.mode == EncounterMode::TurnBased && combat.round >= 1 && combat.active.is_some()
        });
        if !joined_late && !rt && late_round {
            history.join_encounter(&entrant(j3, 12)).expect("J3 joins");
            done.joins += 1;
            joined_late = true;
            hashes.push(history_hash(&history));
        }
    }
    if history.world().combat().is_some() {
        history.end_encounter().expect("releases");
    }
    hashes.push(history_hash(&history));
    (hashes, done, history)
}

/// H5 (B38): two wrapper runs of the E15 script give equal per-step
/// `history_hash`, in TB and in RT; a different seed does not; and the
/// journal holds one `Joined` per join and one `Left` per leave.
///
/// **Fails if** a journaled join or leave depends on anything but the wrapper,
/// or a join or leave is journaled twice or not at all.
#[test]
fn h5_replay() {
    for rt in [false, true] {
        let (first, done, history) = script(7, rt);
        let (second, again, replayed) = script(7, rt);
        assert_eq!(first, second, "rt: {rt}: equal per-step hashes");
        assert_eq!(done, again);
        assert_eq!(history, replayed);
        assert!(first.len() > 30);
        assert_ne!(first, script(8, rt).0, "rt: {rt}: a seed difference shows");
        let events = all_events(&history);
        let joined = events
            .iter()
            .filter(|event| matches!(event, HistoryEvent::Joined { .. }))
            .count();
        let left = events
            .iter()
            .filter(|event| matches!(event, HistoryEvent::Left { .. }))
            .count();
        assert_eq!((joined, left), (done.joins, done.leaves), "rt: {rt}");
        assert!(done.joins >= 2 && done.leaves == 1, "rt: {rt}: {done:?}");
        if !rt {
            assert_eq!(done.joins, 3, "the TB script reached the late join");
        }
        assert_eq!(history.version(), 2);
    }

    // A save and reload while J waits (round 1), and while J is pending.
    let fixture = script_fixture(false);
    for pending in [false, true] {
        let mut history = bound(&fixture, 7);
        let j = register(&fixture, &mut history, J);
        begin(&fixture, &mut history);
        let ids = combatant_ids(&history);
        if pending {
            history.join_encounter(&entrant(j, 3)).expect("J joins");
            assert!(!history
                .world()
                .combat()
                .expect("combat")
                .pending_joins
                .is_empty());
        } else {
            end_turn(&mut history, ids[1]);
            end_turn(&mut history, ids[0]);
            history.join_encounter(&entrant(j, 12)).expect("J joins");
            assert!(!history.world().timeline().contains(j));
            assert!(history
                .world()
                .combat()
                .expect("combat")
                .pending_joins
                .is_empty());
        }
        let mut reloaded: HistoryWorld =
            serde_json::from_str(&text(&history)).expect("the save reloads");
        assert_eq!(reloaded, history);
        assert_eq!(history_hash(&reloaded), history_hash(&history));
        assert_eq!(reloaded.version(), 2);
        for index in 0..20 {
            step(&mut history, fixture.ability);
            step(&mut reloaded, fixture.ability);
            assert_eq!(
                history_hash(&reloaded),
                history_hash(&history),
                "pending: {pending}, step {index}"
            );
        }
        assert_eq!(reloaded, history);
        assert_eq!(all_events(&reloaded), all_events(&history));
    }
}
