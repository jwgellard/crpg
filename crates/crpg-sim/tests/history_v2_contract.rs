//! Two-keys contract tests for history v2 (T138b, ADR-0029 follow-on N5).
//!
//! Written independently from the contract text (`tasks/T138.md`, Behaviour
//! items H1–H6) through the public `HistoryWorld` API only. The implementer's
//! own tests (`history_v2.rs`, `history_v2_golden.rs`,
//! `support/history_v2_schedule.rs`, `src/history/tests.rs`) and the new
//! `src/` code were not read; only the public signatures needed to call were.
//!
//! Every test names the Behaviour item(s) it proves, and uses the exact JSON
//! encodings, error messages and version numbers the contract gives.

#[allow(dead_code)]
mod support;

use crpg_core::Ulid;
use crpg_sim::{
    history_hash, CombatAction, CombatError, EncounterMode, EntityMeta, HistoryError, HistoryEvent,
    HistoryWorld, HISTORY_VERSION,
};
use support::{rt, Fixture};

/// The retained payload sequence, oldest first.
fn payloads(history: &HistoryWorld) -> Vec<HistoryEvent> {
    history
        .read_after(history.acknowledged(), 256)
        .expect("a full read fits one page")
        .into_iter()
        .map(|envelope| envelope.payload)
        .collect()
}

/// Complete canonical wrapper bytes for rollback comparisons.
fn snapshot(history: &HistoryWorld) -> Vec<u8> {
    serde_json::to_vec(history).expect("the wrapper serializes")
}

/// The standard encounter identity both fixtures share.
fn encounter_id() -> Ulid {
    Ulid::from_u128(101)
}

/// Replaces the first occurrence of `needle` with `replacement` in `base`,
/// proves the edit changed the wrapper, then asserts the load fails with a
/// message containing `expected`.
fn load_expect(base: &str, needle: &str, replacement: &str, expected: &str) {
    let swapped = base.replacen(needle, replacement, 1);
    assert_ne!(swapped, base, "the fault must actually change the wrapper");
    let err =
        serde_json::from_str::<HistoryWorld>(&swapped).expect_err("the fault must be rejected");
    assert!(
        err.to_string().contains(expected),
        "expected {expected:?}, got: {err}"
    );
}

/// H1: the three new payloads encode exactly as shown and round-trip.
#[test]
fn h1_payload_encodings_and_round_trip() {
    assert_eq!(
        serde_json::to_string(&HistoryEvent::RoundStarted { round: 3 }).expect("serializes"),
        "{\"type\":\"round_started\",\"value\":{\"round\":3}}"
    );
    assert_eq!(
        serde_json::to_string(&HistoryEvent::ModeChanged {
            mode: EncounterMode::RealTime,
            round: 3,
        })
        .expect("serializes"),
        "{\"type\":\"mode_changed\",\"value\":{\"mode\":\"real_time\",\"round\":3}}"
    );

    // A freshly spawned id at slot 1, generation 1 matches the contract's
    // actor encoding.
    let mut world = HistoryWorld::new(0);
    let _first = world.spawn(EntityMeta {}).expect("spawn");
    let second = world.spawn(EntityMeta {}).expect("spawn");
    assert_eq!((second.index(), second.generation()), (1, 1));
    assert_eq!(
        serde_json::to_string(&HistoryEvent::OrderEnded { actor: second }).expect("serializes"),
        "{\"type\":\"order_ended\",\"value\":{\"actor\":{\"index\":1,\"generation\":1}}}"
    );

    // Each payload round-trips through the public serde surface.
    let events = [
        HistoryEvent::RoundStarted { round: 3 },
        HistoryEvent::ModeChanged {
            mode: EncounterMode::RealTime,
            round: 3,
        },
        HistoryEvent::OrderEnded { actor: second },
    ];
    for event in events {
        let json = serde_json::to_value(&event).expect("serializes");
        let back: HistoryEvent = serde_json::from_value(json).expect("deserializes");
        assert_eq!(back, event, "the payload round-trips");
    }
}

/// H1: duplicate, unknown and missing keys fail, and so does `value` before
/// `type`; an unknown `type` fails with the existing message.
#[test]
fn h1_payload_load_faults() {
    // A version-2 wrapper (an RT start) whose journal we mutate with raw v2
    // payloads, so the loader's manual visitors are what decide.
    let fixture = rt::capable(4);
    let (history, ids) = fixture
        .start_history(1, EncounterMode::RealTime)
        .expect("starts");
    assert_eq!(history.version(), 2, "an RT start upgrades the wrapper");
    let base = serde_json::to_string(&history).expect("serializes");
    let journal = history.read_after(0, 256).expect("read");
    let spawned = serde_json::to_string(&journal[0].payload).expect("payload");
    let actor = serde_json::to_string(&ids[1]).expect("id");

    // Each payload swaps in and loads with its exact value.
    let round_started = "{\"type\":\"round_started\",\"value\":{\"round\":3}}".to_owned();
    let loaded: HistoryWorld =
        serde_json::from_str(&base.replacen(&spawned, &round_started, 1)).expect("loads");
    assert!(matches!(
        loaded.read_after(0, 256).expect("read")[0].payload,
        HistoryEvent::RoundStarted { round: 3 }
    ));

    let mode_changed =
        "{\"type\":\"mode_changed\",\"value\":{\"mode\":\"real_time\",\"round\":3}}".to_owned();
    let loaded: HistoryWorld =
        serde_json::from_str(&base.replacen(&spawned, &mode_changed, 1)).expect("loads");
    assert!(matches!(
        loaded.read_after(0, 256).expect("read")[0].payload,
        HistoryEvent::ModeChanged {
            mode: EncounterMode::RealTime,
            round: 3
        }
    ));

    let order_ended = format!("{{\"type\":\"order_ended\",\"value\":{{\"actor\":{actor}}}}}");
    let loaded: HistoryWorld =
        serde_json::from_str(&base.replacen(&spawned, &order_ended, 1)).expect("loads");
    assert!(matches!(
        loaded.read_after(0, 256).expect("read")[0].payload,
        HistoryEvent::OrderEnded { actor } if actor == ids[1]
    ));

    // Faults on round_started (the `round` visitor).
    load_expect(
        &base,
        &spawned,
        "{\"value\":{\"round\":3},\"type\":\"round_started\"}",
        "requires `type` before `value`",
    );
    load_expect(
        &base,
        &spawned,
        "{\"type\":\"round_startedx\",\"value\":{\"round\":3}}",
        "unknown history event type",
    );
    load_expect(
        &base,
        &spawned,
        "{\"type\":\"round_started\",\"value\":{\"round\":3,\"round\":4}}",
        "duplicate history payload field",
    );
    load_expect(
        &base,
        &spawned,
        "{\"type\":\"round_started\",\"value\":{\"round\":3,\"roundx\":4}}",
        "unknown history payload field",
    );
    load_expect(
        &base,
        &spawned,
        "{\"type\":\"round_started\",\"value\":{}}",
        "misses `round`",
    );

    // Faults on mode_changed (both fields are required).
    load_expect(
        &base,
        &spawned,
        "{\"type\":\"mode_changed\",\"value\":{\"round\":3}}",
        "misses `mode`",
    );
    load_expect(
        &base,
        &spawned,
        "{\"type\":\"mode_changed\",\"value\":{\"mode\":\"real_time\"}}",
        "misses `round`",
    );

    // Faults on order_ended (its own `actor` visitor).
    load_expect(
        &base,
        &spawned,
        "{\"type\":\"order_ended\",\"value\":{}}",
        "misses `actor`",
    );
    load_expect(
        &base,
        &spawned,
        &format!("{{\"value\":{{\"actor\":{actor}}},\"type\":\"order_ended\"}}"),
        "requires `type` before `value`",
    );
    load_expect(
        &base,
        &spawned,
        &format!("{{\"type\":\"order_ended\",\"value\":{{\"actor\":{actor},\"actor\":{actor}}}}}"),
        "duplicate history payload field",
    );
}

/// H2: one RT tick with two killing resolutions — each drained `Died` follows
/// its own (k-th) killing `ActionResolved`, never a trailing batch.
#[test]
fn h2_rt_tick_two_kills_each_died_follows_its_resolution() {
    let mut fixture = rt::capable_trio(4);
    rt::make_lethal(&mut fixture, 10);
    let (mut history, ids) = fixture
        .start_history(42, EncounterMode::RealTime)
        .expect("starts");
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    // Order A -> C and B -> A; both are due at key 0 on the next tick.
    history
        .perform_action(&CombatAction::UseAbility {
            actor: a,
            ability: fixture.ability,
            target: c,
        })
        .expect("order stored");
    history
        .perform_action(&CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        })
        .expect("order stored");
    // Setting orders journals nothing.
    assert_eq!(
        payloads(&history),
        vec![
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
            HistoryEvent::Spawned { entity: c },
        ]
    );
    history.tick().expect("tick");
    assert_eq!(
        payloads(&history),
        vec![
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
            HistoryEvent::Spawned { entity: c },
            HistoryEvent::ActionResolved {
                actor: a,
                target: c,
                ability: fixture.ability,
                outcome: "success".to_owned(),
                damage: 10,
            },
            HistoryEvent::Died { entity: c },
            HistoryEvent::ActionResolved {
                actor: b,
                target: a,
                ability: fixture.ability,
                outcome: "success".to_owned(),
                damage: 10,
            },
            HistoryEvent::Died { entity: a },
            HistoryEvent::EncounterEnded {
                encounter: encounter_id(),
                round: 0,
            },
        ]
    );
}

/// H2: a tick applying the pending RT→TB switch journals `ModeChanged` then
/// `TurnStarted` (no `RoundStarted`).
#[test]
fn h2_rt_tick_boundary_mode_changed_then_turn_started() {
    let fixture = rt::capable(4);
    let (mut history, ids) = fixture
        .start_history(1, EncounterMode::RealTime)
        .expect("starts");
    let (a, b) = (ids[0], ids[1]);
    history
        .request_mode(EncounterMode::TurnBased)
        .expect("request");
    assert_eq!(
        payloads(&history),
        vec![
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
        ],
        "a mode request journals nothing"
    );
    for _ in 0..4 {
        history.tick().expect("tick");
    }
    assert_eq!(
        payloads(&history),
        vec![
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
            HistoryEvent::ModeChanged {
                mode: EncounterMode::TurnBased,
                round: 1,
            },
            HistoryEvent::TurnStarted { actor: b, round: 1 },
        ]
    );
}

/// H2: a steady RT boundary journals `RoundStarted` with the new round.
#[test]
fn h2_rt_tick_steady_boundary_round_started() {
    let fixture = rt::capable(4);
    let (mut history, ids) = fixture
        .start_history(1, EncounterMode::RealTime)
        .expect("starts");
    let (a, b) = (ids[0], ids[1]);
    for _ in 0..4 {
        history.tick().expect("tick");
    }
    assert_eq!(
        payloads(&history),
        vec![
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
            HistoryEvent::RoundStarted { round: 1 },
        ]
    );
}

/// H2: an RT start journals only its `Spawned` events; a TB start journals
/// `Spawned` plus the initial `TurnStarted`, as today.
#[test]
fn h2_rt_start_journals_only_spawned() {
    let fixture = rt::capable(4);
    let (history, ids) = fixture
        .start_history(1, EncounterMode::RealTime)
        .expect("starts");
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        payloads(&history),
        vec![
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
        ]
    );

    let (history, ids) = fixture
        .start_history(1, EncounterMode::TurnBased)
        .expect("starts");
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        payloads(&history),
        vec![
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
            HistoryEvent::TurnStarted { actor: b, round: 0 },
        ]
    );
}

/// H2: RT order setting, `Stop` and `request_mode` journal nothing.
#[test]
fn h2_no_events_for_order_setting_stop_and_request() {
    let fixture = rt::capable(4);
    let (mut history, ids) = fixture
        .start_history(1, EncounterMode::RealTime)
        .expect("starts");
    let (a, b) = (ids[0], ids[1]);
    let before = payloads(&history);
    history
        .perform_action(&CombatAction::UseAbility {
            actor: a,
            ability: fixture.ability,
            target: b,
        })
        .expect("order stored");
    assert_eq!(payloads(&history), before, "order setting journals nothing");
    history
        .perform_action(&CombatAction::Stop { actor: a })
        .expect("stop");
    assert_eq!(payloads(&history), before, "Stop journals nothing");
    history
        .request_mode(EncounterMode::TurnBased)
        .expect("request");
    assert_eq!(payloads(&history), before, "request_mode journals nothing");
}

/// H2: an RT `despawn` records `EncounterEnded` exactly when it drops
/// `rt_alive` from two or more to below two.
#[test]
fn h2_rt_despawn_records_encounter_ended() {
    let fixture = rt::capable(4);
    let (mut history, ids) = fixture
        .start_history(1, EncounterMode::RealTime)
        .expect("starts");
    let (a, b) = (ids[0], ids[1]);
    assert!(history.despawn(a).expect("despawn"));
    assert_eq!(
        payloads(&history),
        vec![
            HistoryEvent::Spawned { entity: a },
            HistoryEvent::Spawned { entity: b },
            HistoryEvent::Despawned { entity: a },
            HistoryEvent::EncounterEnded {
                encounter: encounter_id(),
                round: 0,
            },
        ]
    );
}

/// H2: inside `perform_action`, a TB→RT switch trails the resolution with
/// `ModeChanged` then `RoundStarted` (no `TurnStarted`).
#[test]
fn h2_tb_to_rt_perform_action_trailing_transition() {
    let fixture = rt::capable(4);
    let (mut history, ids) = fixture
        .start_history(1, EncounterMode::TurnBased)
        .expect("starts");
    let (a, b) = (ids[0], ids[1]);
    history
        .request_mode(EncounterMode::RealTime)
        .expect("request");
    history
        .perform_action(&CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        })
        .expect("attack")
        .expect("attack");
    history
        .perform_action(&CombatAction::UseAbility {
            actor: a,
            ability: fixture.ability,
            target: b,
        })
        .expect("attack")
        .expect("attack");
    let ps = payloads(&history);
    assert_eq!(ps.len(), 8);
    assert!(
        matches!(ps[3], HistoryEvent::ActionResolved { actor, target, .. } if actor == b && target == a),
        "B's resolution: {ps:?}"
    );
    assert_eq!(ps[4], HistoryEvent::TurnStarted { actor: a, round: 0 });
    assert!(
        matches!(ps[5], HistoryEvent::ActionResolved { actor, target, .. } if actor == a && target == b),
        "A's resolution: {ps:?}"
    );
    assert_eq!(
        ps[6],
        HistoryEvent::ModeChanged {
            mode: EncounterMode::RealTime,
            round: 1,
        }
    );
    assert_eq!(ps[7], HistoryEvent::RoundStarted { round: 1 });
}

/// H3: the lazy, sticky wrapper version.
#[test]
fn h3_version_lifecycle() {
    assert_eq!(HISTORY_VERSION, 2, "the newest written version is 2");

    // A new wrapper starts at version 1.
    let fresh = HistoryWorld::new(0);
    assert_eq!(fresh.version(), 1);

    // A TB-only history stays at version 1 through start, action, tick and
    // release.
    let fixture = Fixture::standard();
    let (mut history, ids) = fixture
        .start_history(1, EncounterMode::TurnBased)
        .expect("starts");
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(history.version(), 1);
    history
        .perform_action(&CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        })
        .expect("attack")
        .expect("attack");
    history.tick().expect("tick");
    history.end_encounter().expect("release");
    assert_eq!(history.version(), 1, "a TB-only history never upgrades");

    // An RT-capable start (even in TB) upgrades to 2 and is sticky.
    let fixture = rt::capable(4);
    let (mut history, _ids) = fixture
        .start_history(1, EncounterMode::TurnBased)
        .expect("starts");
    assert_eq!(history.version(), 2, "an RT-capable encounter upgrades");
    history.end_encounter().expect("release");
    assert_eq!(history.version(), 2, "sticky across release");
    history.acknowledge(history.last_sequence()).expect("ack");
    assert_eq!(history.version(), 2, "sticky across acknowledgement");

    // A rejected commit does not upgrade.
    let fixture = Fixture::standard();
    let mut history = HistoryWorld::new(0);
    let result =
        fixture.with_spec(|spec| history.start_encounter_in_mode(spec, EncounterMode::RealTime));
    assert_eq!(
        result,
        Err(HistoryError::Combat(CombatError::ModeUnsupported))
    );
    assert_eq!(history.version(), 1, "a rejected commit leaves version 1");
    assert_eq!(history.pending_len(), 0);
}

/// H4: the loader rules under "Hash and history".
#[test]
fn h4_loader_rules() {
    // Version 3 is unknown.
    let fresh = serde_json::to_string(&HistoryWorld::new(0)).expect("serializes");
    let mut value: serde_json::Value = serde_json::from_str(&fresh).expect("parses");
    value["version"] = serde_json::Value::from(3);
    assert!(serde_json::from_value::<HistoryWorld>(value)
        .expect_err("unknown version fails")
        .to_string()
        .contains("unknown history version"));

    // A version-1 wrapper holding a v2 event is rejected.
    let fixture = Fixture::standard();
    let (history, _ids) = fixture
        .start_history(1, EncounterMode::TurnBased)
        .expect("starts");
    assert_eq!(history.version(), 1);
    let base = serde_json::to_string(&history).expect("serializes");
    let journal = history.read_after(0, 256).expect("read");
    let spawned = serde_json::to_string(&journal[0].payload).expect("payload");
    load_expect(
        &base,
        &spawned,
        "{\"type\":\"round_started\",\"value\":{\"round\":3}}",
        "history v1 journal holds a v2 event",
    );

    // A version-1 wrapper holding a real-time-capable encounter is rejected.
    let fixture = rt::capable(4);
    let (history, _ids) = fixture
        .start_history(1, EncounterMode::TurnBased)
        .expect("starts");
    let base = serde_json::to_string(&history).expect("serializes");
    let mut value: serde_json::Value = serde_json::from_str(&base).expect("parses");
    value["version"] = serde_json::Value::from(1);
    assert!(serde_json::from_value::<HistoryWorld>(value)
        .expect_err("v1 wrapper with a clock fails")
        .to_string()
        .contains("history v1 wrapper holds a real-time-capable encounter"));

    // A version-1 wrapper that is valid today loads byte-identically and
    // hashes unchanged.
    let fixture = Fixture::standard();
    let (mut history, ids) = fixture
        .start_history(1, EncounterMode::TurnBased)
        .expect("starts");
    let (a, b) = (ids[0], ids[1]);
    history
        .perform_action(&CombatAction::UseAbility {
            actor: b,
            ability: fixture.ability,
            target: a,
        })
        .expect("attack")
        .expect("attack");
    assert_eq!(history.version(), 1);
    let bytes = snapshot(&history);
    let hash = history_hash(&history);
    let loaded: HistoryWorld = serde_json::from_slice(&bytes).expect("loads");
    assert_eq!(loaded, history);
    assert_eq!(
        snapshot(&loaded),
        bytes,
        "re-serialization is byte-identical"
    );
    assert_eq!(history_hash(&loaded), hash);
    assert_eq!(loaded.version(), 1);
}

/// H5: `request_mode` commits zero events (needs no free slot or sequence),
/// and gameplay errors win over history errors.
#[test]
fn h5_request_mode_zero_events_and_gameplay_errors_win() {
    // request_mode succeeds at sequence exhaustion: it needs no sequence.
    let fixture = rt::capable(4);
    let (mut history, _ids) = fixture
        .start_history(1, EncounterMode::RealTime)
        .expect("starts");
    history.acknowledge(history.last_sequence()).expect("drain");
    let mut value: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&history).expect("serializes"))
            .expect("parses");
    value["acknowledged"] = serde_json::Value::from(u64::MAX - 1);
    value["next_seq"] = serde_json::Value::from(u64::MAX);
    let mut history: HistoryWorld = serde_json::from_value(value).expect("exhaustion loads");
    history
        .request_mode(EncounterMode::TurnBased)
        .expect("request at exhaustion");
    assert_eq!(history.last_sequence(), u64::MAX - 1);
    assert_eq!(history.pending_len(), 0);

    // Gameplay errors win over history errors: a request with no encounter.
    let mut fresh = HistoryWorld::new(0);
    assert_eq!(
        fresh.request_mode(EncounterMode::TurnBased),
        Err(HistoryError::Combat(CombatError::NoEncounter))
    );

    // At a full journal, an invalid action still reports the combat error.
    let fixture = Fixture::standard();
    let (mut history, ids) = fixture
        .start_history(1, EncounterMode::TurnBased)
        .expect("starts");
    let (a, b) = (ids[0], ids[1]);
    for _ in 0..(4096 - history.pending_len()) {
        history.spawn(EntityMeta {}).expect("fill");
    }
    assert_eq!(history.pending_len(), 4096);
    assert_eq!(
        history.perform_action(&CombatAction::UseAbility {
            actor: a,
            ability: fixture.ability,
            target: b,
        }),
        Err(HistoryError::Combat(CombatError::OutOfTurn {
            actor: a,
            active: Some(b),
        }))
    );
}

/// H6: an over-cap RT tick commits nothing — no clock advance, no RNG draw.
#[test]
fn h6_over_cap_tick_commits_nothing() {
    let fixture = rt::capable(4);
    let (mut history, ids) = fixture
        .start_history(1, EncounterMode::RealTime)
        .expect("starts");
    let (a, b) = (ids[0], ids[1]);
    for _ in 0..(4096 - history.pending_len()) {
        history.spawn(EntityMeta {}).expect("fill");
    }
    assert_eq!(history.pending_len(), 4096);
    // An order that would resolve on the next tick.
    history
        .perform_action(&CombatAction::UseAbility {
            actor: a,
            ability: fixture.ability,
            target: b,
        })
        .expect("order stored");
    let before = snapshot(&history);
    let before_hash = history_hash(&history);
    let before_world_tick = history.world().tick().get();
    let before_elapsed = rt::elapsed(history.world());
    assert_eq!(history.tick(), Err(HistoryError::EventLimit));
    assert_eq!(snapshot(&history), before, "the tick commits nothing");
    assert_eq!(history_hash(&history), before_hash, "no RNG draw");
    assert_eq!(
        history.world().tick().get(),
        before_world_tick,
        "no clock advance"
    );
    assert_eq!(
        rt::elapsed(history.world()),
        before_elapsed,
        "no clock advance"
    );
    assert!(
        history
            .world()
            .combatants()
            .get(a)
            .expect("combatant")
            .order()
            .is_some(),
        "the standing order survives the rollback"
    );
}
