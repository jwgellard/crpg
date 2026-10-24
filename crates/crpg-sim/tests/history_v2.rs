//! History v2 (T138b, ADR-0029 N5): R16 (H1), R17 (H2), R18 (H3, H4) and R19
//! (H5, H6).
//!
//! Expected journals are worked out by hand from the contract: stage 2 runs
//! the clock and a boundary, stage 3 runs the due orders in ascending
//! `EntityId`, and every scenario states the tick on which each event must
//! appear.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_sim::{
    history_hash, CombatAction, CombatError, EncounterMode, EntityMeta, HistoryError, HistoryEvent,
    HistoryWorld, HISTORY_VERSION, MAX_HISTORY_EVENTS,
};
use support::rt::{add_clone, capable, capable_trio, make_lethal, use_action};
use support::Fixture;

/// The encounter identity the standard fixture publishes.
fn encounter_id() -> Ulid {
    Ulid::from_u128(101)
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

/// The wrapper's canonical text.
fn text(history: &HistoryWorld) -> String {
    serde_json::to_string(history).expect("the wrapper serializes")
}

/// The wrapper as a JSON value (object keys sorted, so `version` follows
/// `pending`).
fn value(history: &HistoryWorld) -> serde_json::Value {
    serde_json::to_value(history).expect("the wrapper serializes")
}

/// Loads a wrapper from a JSON value through the validating loader.
fn load(value: serde_json::Value) -> Result<HistoryWorld, serde_json::Error> {
    serde_json::from_value(value)
}

/// One `ActionResolved` with the symbolic `success` outcome.
fn resolved(actor: EntityId, target: EntityId, ability: Ulid, damage: u32) -> HistoryEvent {
    HistoryEvent::ActionResolved {
        actor,
        target,
        ability,
        outcome: "success".to_owned(),
        damage,
    }
}

/// Two distinct entities minted in a scratch wrapper: `(index 0, index 1)`.
fn two_ids() -> (EntityId, EntityId) {
    let mut history = HistoryWorld::new(0);
    let first = history.spawn(EntityMeta {}).expect("spawns");
    let second = history.spawn(EntityMeta {}).expect("spawns");
    (first, second)
}

/// Fills the journal with `count` spawn events.
fn fill(history: &mut HistoryWorld, count: usize) {
    for _ in 0..count {
        history.spawn(EntityMeta {}).expect("the journal fills");
    }
}

// ---------------------------------------------------------------- R16 (H1)

/// R16 (H1): the three payloads encode exactly as the contract shows.
#[test]
fn r16_exact_encodings() {
    let (_, second) = two_ids();
    assert_eq!(second.index(), 1);
    assert_eq!(second.generation(), 1);
    let cases = [
        (
            HistoryEvent::RoundStarted { round: 3 },
            r#"{"type":"round_started","value":{"round":3}}"#,
        ),
        (
            HistoryEvent::ModeChanged {
                mode: EncounterMode::RealTime,
                round: 3,
            },
            r#"{"type":"mode_changed","value":{"mode":"real_time","round":3}}"#,
        ),
        (
            HistoryEvent::ModeChanged {
                mode: EncounterMode::TurnBased,
                round: 0,
            },
            r#"{"type":"mode_changed","value":{"mode":"turn_based","round":0}}"#,
        ),
        (
            HistoryEvent::OrderEnded { actor: second },
            r#"{"type":"order_ended","value":{"actor":{"index":1,"generation":1}}}"#,
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
    // Payload keys may arrive in any order.
    let swapped: HistoryEvent =
        serde_json::from_str(r#"{"type":"mode_changed","value":{"round":3,"mode":"real_time"}}"#)
            .expect("payload key order is free");
    assert_eq!(
        swapped,
        HistoryEvent::ModeChanged {
            mode: EncounterMode::RealTime,
            round: 3
        }
    );
    // The u64 round survives its widest value.
    let widest = HistoryEvent::RoundStarted { round: u64::MAX };
    let encoded = serde_json::to_string(&widest).expect("encodes");
    assert_eq!(
        serde_json::from_str::<HistoryEvent>(&encoded).expect("decodes"),
        widest
    );
}

/// The error text of decoding `input` as a history event.
fn decode_error(input: &str) -> String {
    serde_json::from_str::<HistoryEvent>(input)
        .expect_err("the input must fail")
        .to_string()
}

/// R16 (H1): duplicate, unknown and missing keys fail for every payload, and
/// so does `value` before `type`.
#[test]
fn r16_key_faults() {
    let actor = r#"{"index":1,"generation":1}"#;
    // (tag, [(key, valid JSON value)]) for each new payload.
    let payloads: [(&str, Vec<(&str, String)>); 3] = [
        ("round_started", vec![("round", "3".to_owned())]),
        (
            "mode_changed",
            vec![
                ("mode", r#""real_time""#.to_owned()),
                ("round", "3".to_owned()),
            ],
        ),
        ("order_ended", vec![("actor", actor.to_owned())]),
    ];
    for (tag, fields) in &payloads {
        let join = |pairs: &[(&str, String)]| -> String {
            pairs
                .iter()
                .map(|(key, val)| format!("\"{key}\":{val}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        let wrap = |body: &str| format!("{{\"type\":\"{tag}\",\"value\":{{{body}}}}}");
        // The valid shape decodes.
        serde_json::from_str::<HistoryEvent>(&wrap(&join(fields))).expect("valid decodes");
        // A duplicate of each key fails.
        for (key, val) in fields {
            let mut doubled = fields.clone();
            doubled.push((key, val.clone()));
            let message = decode_error(&wrap(&join(&doubled)));
            assert!(
                message.contains(&format!("duplicate history payload field `{key}`")),
                "{tag} duplicate {key}: {message}"
            );
        }
        // An unknown key fails.
        let mut extra = fields.clone();
        extra.push(("surprise", "1".to_owned()));
        let message = decode_error(&wrap(&join(&extra)));
        assert!(
            message.contains("unknown history payload field"),
            "{tag} unknown: {message}"
        );
        // A missing key fails, one at a time.
        for (missing, _) in fields {
            let kept: Vec<(&str, String)> = fields
                .iter()
                .filter(|(key, _)| key != missing)
                .cloned()
                .collect();
            let message = decode_error(&wrap(&join(&kept)));
            assert!(
                message.contains(&format!("history payload misses `{missing}`")),
                "{tag} missing {missing}: {message}"
            );
        }
        // `value` before `type` fails.
        let reversed = format!("{{\"value\":{{{}}},\"type\":\"{tag}\"}}", join(fields));
        let message = decode_error(&reversed);
        assert!(
            message.contains("requires `type` before `value`"),
            "{tag} reversed: {message}"
        );
        // A trailing payload key beside `value` fails.
        let trailing = format!(
            "{{\"type\":\"{tag}\",\"value\":{{{}}},\"extra\":1}}",
            join(fields)
        );
        assert!(decode_error(&trailing).contains("unknown history payload field"));
    }
    // An unknown mode name fails inside the payload.
    assert!(serde_json::from_str::<HistoryEvent>(
        r#"{"type":"mode_changed","value":{"mode":"paused","round":1}}"#
    )
    .is_err());
    // `order_ended` is not an `entity` reference.
    assert!(decode_error(
        r#"{"type":"order_ended","value":{"entity":{"index":1,"generation":1}}}"#
    )
    .contains("unknown history payload field"));
    // An unknown type keeps its existing message.
    assert!(
        decode_error(r#"{"type":"round_ended","value":{"round":1}}"#)
            .contains("unknown history event type")
    );
}

// ---------------------------------------------------------------- R17 (H2)

/// R17 (H2): one RT tick with a boundary, two executions (one killing) and an
/// invalid order journals, in order, `RoundStarted`, each `ActionResolved`
/// followed at once by its `Died`, and `OrderEnded`; every event carries the
/// tick it ran under.
#[test]
fn r17_rt_tick_emission_order() {
    let mut fixture = capable(4);
    add_clone(&mut fixture, 113, 123, 8);
    add_clone(&mut fixture, 114, 124, 7);
    make_lethal(&mut fixture, 3);
    for creature in fixture.creatures.values_mut() {
        creature
            .stats
            .insert("alpha".to_owned(), Fx16_16::from_int(100));
    }
    fixture
        .creatures
        .get_mut(&fixture.creature_b)
        .expect("creature B")
        .stats
        .insert("health".to_owned(), Fx16_16::from_int(3));
    let (mut history, ids) = fixture
        .start_history(7, EncounterMode::RealTime)
        .expect("the RT fixture starts");
    let [a, b, c, d] = [ids[0], ids[1], ids[2], ids[3]];
    let mark = history.last_sequence();
    for _ in 0..3 {
        history.tick().expect("an idle tick");
    }
    assert_eq!(
        history.last_sequence(),
        mark,
        "idle RT ticks journal nothing"
    );

    // Orders journal nothing either.
    for action in [
        use_action(a, fixture.ability, b),
        use_action(c, fixture.ability, b),
        use_action(d, fixture.ability, a),
    ] {
        history.perform_action(&action).expect("an order is stored");
    }
    assert_eq!(history.last_sequence(), mark);

    // Tick 4 is a boundary. Due in ascending id: A kills B; B is dead; C's
    // order names the dead B and ends; D hits A without killing.
    history.tick().expect("the boundary tick");
    let ability = fixture.ability;
    assert_eq!(
        since(&history, mark),
        vec![
            HistoryEvent::RoundStarted { round: 1 },
            resolved(a, b, ability, 3),
            HistoryEvent::Died { entity: b },
            HistoryEvent::OrderEnded { actor: c },
            resolved(d, a, ability, 3),
        ]
    );
    for envelope in history.read_after(mark, 256).expect("reads") {
        assert_eq!(envelope.tick.get(), 4);
    }
    assert!(history
        .world()
        .combat()
        .expect("encounter")
        .active
        .is_none());
    assert_eq!(history.version(), 2);
}

/// R17 (H2): the tick that ends an RT encounter journals the kill, then
/// `EncounterEnded`, and nothing after; later ticks journal nothing.
#[test]
fn r17_rt_tick_terminal_edge() {
    let mut fixture = capable(4);
    make_lethal(&mut fixture, 99);
    let (mut history, ids) = fixture
        .start_history(3, EncounterMode::RealTime)
        .expect("starts");
    let (a, b) = (ids[0], ids[1]);
    history
        .perform_action(&use_action(a, fixture.ability, b))
        .expect("order");
    let mark = history.last_sequence();
    history.tick().expect("the kill");
    assert_eq!(
        since(&history, mark),
        vec![
            resolved(a, b, fixture.ability, 99),
            HistoryEvent::Died { entity: b },
            HistoryEvent::EncounterEnded {
                encounter: encounter_id(),
                round: 0,
            },
        ]
    );
    let after = history.last_sequence();
    for _ in 0..6 {
        history.tick().expect("a terminal tick");
    }
    assert_eq!(history.last_sequence(), after, "terminal ticks are silent");
}

/// R17 (H2): the RT to TB tick journals `ModeChanged` then `TurnStarted`, and
/// no `RoundStarted`.
#[test]
fn r17_rt_to_tb_tick_order() {
    let fixture = capable_trio(4);
    let (mut history, ids) = fixture
        .start_history(5, EncounterMode::RealTime)
        .expect("starts");
    let c = ids[2];
    history
        .request_mode(EncounterMode::TurnBased)
        .expect("requests");
    let mark = history.last_sequence();
    for _ in 0..3 {
        history.tick().expect("ticks");
    }
    assert_eq!(history.last_sequence(), mark, "nothing before the boundary");
    history.tick().expect("the boundary");
    assert_eq!(
        since(&history, mark),
        vec![
            HistoryEvent::ModeChanged {
                mode: EncounterMode::TurnBased,
                round: 1,
            },
            HistoryEvent::TurnStarted { actor: c, round: 1 },
        ]
    );
}

/// R17 (H2): TB to RT inside `perform_action` journals `ModeChanged` then
/// `RoundStarted`, with no `TurnStarted`.
#[test]
fn r17_tb_to_rt_in_perform_action() {
    let fixture = capable_trio(4);
    let (mut history, ids) = fixture
        .start_history(5, EncounterMode::TurnBased)
        .expect("starts");
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    assert_eq!(
        since(&history, 0),
        vec![
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
            HistoryEvent::Spawned { entity: c },
            HistoryEvent::TurnStarted { actor: c, round: 0 },
        ]
    );
    history
        .request_mode(EncounterMode::RealTime)
        .expect("requests");
    let mark = history.last_sequence();
    history
        .perform_action(&CombatAction::EndTurn { actor: c })
        .expect("C ends");
    history
        .perform_action(&CombatAction::EndTurn { actor: b })
        .expect("B ends");
    assert_eq!(
        since(&history, mark),
        vec![
            HistoryEvent::TurnStarted { actor: b, round: 0 },
            HistoryEvent::TurnStarted { actor: a, round: 0 },
        ]
    );
    let mark = history.last_sequence();
    history
        .perform_action(&CombatAction::EndTurn { actor: a })
        .expect("A ends into the rollover");
    assert_eq!(
        since(&history, mark),
        vec![
            HistoryEvent::ModeChanged {
                mode: EncounterMode::RealTime,
                round: 1,
            },
            HistoryEvent::RoundStarted { round: 1 },
        ]
    );
    assert_eq!(
        history.world().combat().expect("encounter").mode,
        EncounterMode::RealTime
    );
}

/// R17 (H2): TB to RT inside `despawn` keeps `Despawned` first, then
/// `ModeChanged` and `RoundStarted`.
#[test]
fn r17_tb_to_rt_in_despawn() {
    let fixture = capable_trio(4);
    let (mut history, ids) = fixture
        .start_history(5, EncounterMode::TurnBased)
        .expect("starts");
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    history
        .perform_action(&CombatAction::EndTurn { actor: c })
        .expect("C ends");
    history
        .perform_action(&CombatAction::EndTurn { actor: b })
        .expect("B ends");
    history
        .request_mode(EncounterMode::RealTime)
        .expect("requests");
    let mark = history.last_sequence();
    // A is the active head and the last entry: removing it empties the
    // timeline with two alive, which rolls the round over.
    assert!(history.despawn(a).expect("despawns"));
    assert_eq!(
        since(&history, mark),
        vec![
            HistoryEvent::Despawned { entity: a },
            HistoryEvent::ModeChanged {
                mode: EncounterMode::RealTime,
                round: 1,
            },
            HistoryEvent::RoundStarted { round: 1 },
        ]
    );
}

/// R17 (H2): RT order setting, `Stop` and mode requests journal nothing, and a
/// turn-based tick journals nothing either.
#[test]
fn r17_zero_event_operations() {
    let fixture = capable_trio(4);
    let (mut history, ids) = fixture
        .start_history(5, EncounterMode::RealTime)
        .expect("starts");
    let (a, b) = (ids[0], ids[1]);
    let mark = history.last_sequence();
    let pending = history.pending_len();
    history
        .perform_action(&use_action(a, fixture.ability, b))
        .expect("an RT order");
    history
        .perform_action(&CombatAction::Stop { actor: a })
        .expect("Stop clears it");
    history
        .perform_action(&CombatAction::Stop { actor: a })
        .expect("Stop without an order is a no-op");
    history
        .request_mode(EncounterMode::TurnBased)
        .expect("a request");
    history
        .request_mode(EncounterMode::RealTime)
        .expect("the current mode clears it");
    assert_eq!(history.last_sequence(), mark);
    assert_eq!(history.pending_len(), pending);

    let (mut tb, _) = fixture
        .start_history(5, EncounterMode::TurnBased)
        .expect("starts");
    let mark = tb.last_sequence();
    for _ in 0..9 {
        tb.tick().expect("a TB tick");
    }
    assert_eq!(tb.last_sequence(), mark);
}

/// R17 (H2): an RT start journals only its `Spawned` events; a TB start adds
/// the initial `TurnStarted`.
#[test]
fn r17_rt_start_journal() {
    let fixture = capable_trio(4);
    let (history, ids) = fixture
        .start_history(5, EncounterMode::RealTime)
        .expect("starts");
    assert_eq!(
        since(&history, 0),
        ids.iter()
            .map(|id| HistoryEvent::Spawned { entity: *id })
            .collect::<Vec<_>>()
    );
    let (tb, _) = fixture
        .start_history(5, EncounterMode::TurnBased)
        .expect("starts");
    assert_eq!(since(&tb, 0).len(), 4);
    assert!(matches!(
        since(&tb, 0)[3],
        HistoryEvent::TurnStarted { round: 0, .. }
    ));
}

/// R17 (H2): an RT `despawn` ends the encounter exactly when it drops the
/// alive count from two or more to below two.
#[test]
fn r17_rt_despawn_ends_the_encounter_once() {
    let fixture = capable_trio(4);
    let (mut history, ids) = fixture
        .start_history(5, EncounterMode::RealTime)
        .expect("starts");
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    let mark = history.last_sequence();
    assert!(history.despawn(a).expect("three to two"));
    assert_eq!(
        since(&history, mark),
        vec![HistoryEvent::Despawned { entity: a }]
    );
    let mark = history.last_sequence();
    assert!(history.despawn(b).expect("two to one"));
    assert_eq!(
        since(&history, mark),
        vec![
            HistoryEvent::Despawned { entity: b },
            HistoryEvent::EncounterEnded {
                encounter: encounter_id(),
                round: 0,
            },
        ]
    );
    let mark = history.last_sequence();
    assert!(history.despawn(c).expect("one to none"));
    assert_eq!(
        since(&history, mark),
        vec![HistoryEvent::Despawned { entity: c }],
        "no second end"
    );
    assert!(history.world().combat().is_none());
}

/// R17 (H2): despawning a dead RT combatant does not end the encounter again.
#[test]
fn r17_rt_despawn_of_a_corpse_is_not_an_end() {
    let mut fixture = capable_trio(4);
    make_lethal(&mut fixture, 99);
    let (mut history, ids) = fixture
        .start_history(5, EncounterMode::RealTime)
        .expect("starts");
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    history
        .perform_action(&use_action(a, fixture.ability, b))
        .expect("order");
    history.tick().expect("A kills B");
    let mark = history.last_sequence();
    // Alive: A and C. Removing the corpse leaves the count alone.
    assert!(history.despawn(b).expect("despawns the corpse"));
    assert_eq!(
        since(&history, mark),
        vec![HistoryEvent::Despawned { entity: b }]
    );
    let mark = history.last_sequence();
    assert!(history.despawn(c).expect("two to one"));
    assert_eq!(
        since(&history, mark),
        vec![
            HistoryEvent::Despawned { entity: c },
            HistoryEvent::EncounterEnded {
                encounter: encounter_id(),
                round: 0,
            },
        ]
    );
}

// -------------------------------------------------------- R18 (H3, H4)

/// R18 (H3): TB-only wrappers stay `"version":1` through every operation.
#[test]
fn r18_tb_only_histories_stay_version_one() {
    let fixture = Fixture::standard();
    assert!(text(&HistoryWorld::new(1)).starts_with("{\"version\":1,"));
    let (mut history, ids) = fixture
        .start_history(1, EncounterMode::TurnBased)
        .expect("a TB-only start");
    assert_eq!(history.version(), 1);
    history.tick().expect("ticks");
    history
        .perform_action(&CombatAction::EndTurn { actor: ids[1] })
        .expect("ends");
    history.spawn(EntityMeta {}).expect("spawns");
    history.acknowledge(1).expect("acks");
    history.end_encounter().expect("releases");
    assert_eq!(history.version(), 1);
    assert!(text(&history).starts_with("{\"version\":1,"));
    // An RT start on a TB-only ruleset is rejected and upgrades nothing.
    let mut fresh = HistoryWorld::new(1);
    let error = fixture
        .with_spec(|spec| fresh.start_encounter_in_mode(spec, EncounterMode::RealTime))
        .expect_err("ModeUnsupported");
    assert_eq!(error, HistoryError::Combat(CombatError::ModeUnsupported));
    assert_eq!(fresh.version(), 1);
    assert_eq!(HISTORY_VERSION, 2);
}

/// R18 (H3): the first commit that stages an RT-capable encounter upgrades the
/// wrapper, even for a TB start, and the upgrade is sticky.
#[test]
fn r18_upgrade_is_lazy_and_sticky() {
    let fixture = capable_trio(4);
    let mut history = HistoryWorld::new(9);
    history.spawn(EntityMeta {}).expect("spawns");
    assert_eq!(history.version(), 1, "a spawn alone does not upgrade");
    fixture
        .with_spec(|spec| history.start_encounter_in_mode(spec, EncounterMode::TurnBased))
        .expect("a TB start of an RT-capable ruleset");
    assert_eq!(history.version(), 2);
    assert!(text(&history).starts_with("{\"version\":2,"));
    history.end_encounter().expect("releases");
    assert!(history.world().combat().is_none());
    assert_eq!(history.version(), 2, "release does not downgrade");
    history.acknowledge(history.last_sequence()).expect("acks");
    assert_eq!(history.pending_len(), 0);
    assert_eq!(history.version(), 2, "acknowledgement does not downgrade");
    history.tick().expect("ticks");
    assert_eq!(history.version(), 2);
    let reloaded: HistoryWorld = serde_json::from_str(&text(&history)).expect("reloads");
    assert_eq!(reloaded.version(), 2);
    assert_eq!(reloaded, history);
}

/// R18 (H3): a rejected commit does not upgrade.
#[test]
fn r18_rejected_commit_does_not_upgrade() {
    let fixture = capable_trio(4);
    let mut history = HistoryWorld::new(2);
    fill(&mut history, MAX_HISTORY_EVENTS - 1);
    let before = text(&history);
    let error = fixture
        .with_spec(|spec| history.start_encounter_in_mode(spec, EncounterMode::TurnBased))
        .expect_err("three spawns do not fit one slot");
    assert_eq!(error, HistoryError::EventLimit);
    assert_eq!(history.version(), 1);
    assert_eq!(text(&history), before);
    // A gameplay rejection on an RT-capable ruleset changes nothing either.
    let mut fresh = HistoryWorld::new(2);
    fixture
        .with_spec(|spec| fresh.start_encounter_in_mode(spec, EncounterMode::TurnBased))
        .expect("starts");
    let snapshot = fresh.clone();
    let error = fixture
        .with_spec(|spec| fresh.start_encounter_in_mode(spec, EncounterMode::RealTime))
        .expect_err("EncounterActive");
    assert_eq!(error, HistoryError::Combat(CombatError::EncounterActive));
    assert_eq!(fresh, snapshot);
}

/// R18 (H4): a version-1 wrapper that is valid today loads, re-serializes
/// byte-identically and hashes unchanged; version 2 is also read.
#[test]
fn r18_v1_and_v2_wrappers_round_trip() {
    let fixture = Fixture::standard();
    let (mut v1, ids) = fixture
        .start_history(4, EncounterMode::TurnBased)
        .expect("starts");
    v1.perform_action(&CombatAction::EndTurn { actor: ids[1] })
        .expect("ends");
    assert_eq!(v1.version(), 1);
    let bytes = text(&v1);
    let loaded: HistoryWorld = serde_json::from_str(&bytes).expect("v1 loads");
    assert_eq!(loaded.version(), 1);
    assert_eq!(text(&loaded), bytes);
    assert_eq!(history_hash(&loaded), history_hash(&v1));
    // A TB-only wrapper may also carry version 2.
    let mut as_two = value(&v1);
    as_two["version"] = serde_json::Value::from(2);
    let loaded = load(as_two).expect("v2 loads");
    assert_eq!(loaded.version(), 2);
    assert!(text(&loaded).starts_with("{\"version\":2,"));

    // A real RT wrapper round-trips.
    let (mut rt, rt_ids) = capable_trio(4)
        .start_history(4, EncounterMode::RealTime)
        .expect("starts");
    rt.perform_action(&use_action(rt_ids[0], capable_trio(4).ability, rt_ids[1]))
        .expect("order");
    for _ in 0..4 {
        rt.tick().expect("ticks");
    }
    assert_eq!(rt.version(), 2);
    let reloaded: HistoryWorld = serde_json::from_str(&text(&rt)).expect("v2 reloads");
    assert_eq!(reloaded, rt);
    assert_eq!(history_hash(&reloaded), history_hash(&rt));
}

/// R18 (H4): the two version-1 rejections and the unknown versions.
#[test]
fn r18_loader_rejections() {
    // A journal holding a v2 event.
    let (mut rt, _) = capable_trio(4)
        .start_history(4, EncounterMode::RealTime)
        .expect("starts");
    for _ in 0..4 {
        rt.tick().expect("ticks");
    }
    assert!(since(&rt, 0)
        .iter()
        .any(|event| matches!(event, HistoryEvent::RoundStarted { .. })));
    // `serde_json::Value` objects are sorted, so `version` follows `pending`
    // here: payloads decode with both vocabularies whatever the version.
    let mut forged = value(&rt);
    forged["version"] = serde_json::Value::from(1);
    let message = load(forged).expect_err("v1 with a v2 event").to_string();
    assert!(
        message.contains("history v1 journal holds a v2 event"),
        "{message}"
    );
    // The same holds on the text path with `version` first.
    let forged = text(&rt).replacen("\"version\":2", "\"version\":1", 1);
    let message = serde_json::from_str::<HistoryWorld>(&forged)
        .expect_err("v1 with a v2 event")
        .to_string();
    assert!(message.contains("history v1 journal holds a v2 event"));

    // A world holding an RT-capable encounter, with no v2 event journaled.
    let (capable, _) = capable_trio(4)
        .start_history(4, EncounterMode::TurnBased)
        .expect("starts");
    assert!(since(&capable, 0).iter().all(|event| !matches!(
        event,
        HistoryEvent::RoundStarted { .. }
            | HistoryEvent::ModeChanged { .. }
            | HistoryEvent::OrderEnded { .. }
    )));
    let mut forged = value(&capable);
    forged["version"] = serde_json::Value::from(1);
    let message = load(forged).expect_err("v1 with a clock").to_string();
    assert!(
        message.contains("history v1 wrapper holds a real-time-capable encounter"),
        "{message}"
    );

    // Unknown versions.
    for unknown in [0_u64, 3, 4, u64::from(u32::MAX)] {
        let mut forged = value(&rt);
        forged["version"] = serde_json::Value::from(unknown);
        let message = load(forged).expect_err("unknown version").to_string();
        assert!(
            message.contains("unknown history version"),
            "{unknown}: {message}"
        );
    }
}

// -------------------------------------------------------- R19 (H5, H6)

/// R19 (H5): `request_mode` commits zero events, so it succeeds at sequence
/// exhaustion; an emitting operation does not. Gameplay errors win.
#[test]
fn r19_request_mode_at_sequence_exhaustion() {
    let fixture = capable_trio(4);
    let (mut history, _) = fixture
        .start_history(6, EncounterMode::TurnBased)
        .expect("starts");
    history.acknowledge(history.last_sequence()).expect("acks");
    let mut forged = value(&history);
    forged["acknowledged"] = serde_json::Value::from(u64::MAX - 1);
    forged["next_seq"] = serde_json::Value::from(u64::MAX);
    let mut exhausted = load(forged).expect("the exhausted sentinel loads");
    assert_eq!(exhausted.last_sequence(), u64::MAX - 1);
    exhausted
        .request_mode(EncounterMode::RealTime)
        .expect("zero events need no sequence");
    assert_eq!(
        exhausted.world().combat().expect("encounter").pending_mode,
        Some(EncounterMode::RealTime)
    );
    exhausted
        .request_mode(EncounterMode::TurnBased)
        .expect("the current mode clears the request");
    assert_eq!(
        exhausted.spawn(EntityMeta {}),
        Err(HistoryError::SequenceExhausted)
    );

    // Gameplay errors win and commit nothing.
    let mut empty = HistoryWorld::new(1);
    let before = text(&empty);
    assert_eq!(
        empty.request_mode(EncounterMode::TurnBased),
        Err(HistoryError::Combat(CombatError::NoEncounter))
    );
    assert_eq!(text(&empty), before);
    let (mut only, _) = Fixture::standard()
        .start_history(1, EncounterMode::TurnBased)
        .expect("starts");
    let before = text(&only);
    assert_eq!(
        only.request_mode(EncounterMode::RealTime),
        Err(HistoryError::Combat(CombatError::ModeUnsupported))
    );
    assert_eq!(text(&only), before);
    // A request upgrades nothing by itself on a v1 wrapper's TB-only world,
    // and it journals nothing on an RT-capable one.
    let last = history.last_sequence();
    history
        .request_mode(EncounterMode::RealTime)
        .expect("requests");
    assert_eq!(history.last_sequence(), last);
}

/// R19 (H6): a tick whose events would pass the cap commits nothing, and the
/// caps apply unchanged: the same tick fits once a slot frees.
#[test]
fn r19_over_cap_rt_tick_commits_nothing() {
    let mut fixture = capable_trio(4);
    make_lethal(&mut fixture, 99);
    let (mut base, ids) = fixture
        .start_history(8, EncounterMode::RealTime)
        .expect("starts");
    let (a, b) = (ids[0], ids[1]);
    base.perform_action(&use_action(a, fixture.ability, b))
        .expect("order");

    // Control: with two free slots the killing tick (ActionResolved + Died)
    // fits exactly.
    let mut fits = base.clone();
    fill(&mut fits, MAX_HISTORY_EVENTS - base.pending_len() - 2);
    assert_eq!(fits.pending_len(), MAX_HISTORY_EVENTS - 2);
    fits.tick().expect("exactly two slots remain");
    assert_eq!(fits.pending_len(), MAX_HISTORY_EVENTS);

    // One slot short: the tick returns the history error and nothing moves.
    let mut tight = base.clone();
    fill(&mut tight, MAX_HISTORY_EVENTS - base.pending_len() - 1);
    assert_eq!(tight.pending_len(), MAX_HISTORY_EVENTS - 1);
    let before = tight.clone();
    let before_hash = history_hash(&tight);
    assert_eq!(tight.tick(), Err(HistoryError::EventLimit));
    assert_eq!(tight, before, "no clock advance, no RNG draw, no events");
    assert_eq!(history_hash(&tight), before_hash);
    assert_eq!(tight.world().tick(), before.world().tick());

    // The failed tick left the order due: free a slot and the tick succeeds
    // with the same result as the control.
    tight.acknowledge(tight.acknowledged() + 1).expect("acks");
    tight.tick().expect("the tick fits once a slot frees");
    let kills = since(&tight, tight.last_sequence() - 2);
    assert_eq!(
        kills,
        vec![
            resolved(a, b, fixture.ability, 99),
            HistoryEvent::Died { entity: b }
        ]
    );
    // `HISTORY_VERSION` is the newest version this build writes.
    assert_eq!(tight.version(), HISTORY_VERSION);
}
