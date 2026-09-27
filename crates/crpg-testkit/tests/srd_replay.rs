#![forbid(unsafe_code)]
//! Headless srd replay integration (T017e).
//!
//! Cross-layer proof that authored `combat_srd` content drives the generalized
//! B3 controller through the unmodified generic replay harness: production
//! loading, a caller-owned srd adapter, the checked-in `combat_srd.replay`
//! artifact, independently generated native goldens, and
//! trace/state/terminal/rollback coverage. The trace carries observed sim
//! results only; expected arithmetic is recomputed from pinned faces and
//! literal authored values. No downstream CLI change.

#[path = "support/srd.rs"]
mod srd;

use crpg_core::Ulid;
use crpg_rules::Outcome;
use crpg_sim::{state_hash, tick, World};
use crpg_testkit::{
    play_and_verify, play_replay, read_replay, verify_golden, write_golden, Mismatch, Replay,
    ReplayInput, REPLAY_FORMAT_VERSION,
};
use serde_json::json;
use srd::{
    srd_apply, srd_replay_path, srd_temp_file, ParticipantObservation, SrdAdapter, SrdContent,
    SrdStateObservation, SrdTraceEntry, SRD_CAMPAIGN_ID, SRD_CAMPAIGN_VERSION, SRD_ENGINE_VERSION,
};

/// Pinned seed (first satisfying seed in `0..=1023`, probed with a temporary
/// test through the public combat API; loser groups 0..=107 no-failure,
/// 3..=277 no-shift, 2..=282 terminal-at-5).
const SEED: u64 = 285;
/// Terminal action tick (`K`, with `7 <= K <= 12`).
const K: u64 = 9;
/// Trailing input-free ticks.
const T: u64 = 4;
/// Total ticks (`K + 1 + T`).
const TOTAL_TICKS: u64 = K + 1 + T;
/// Alternate seed with a different tick-1 focus face (prefix 0..=2 valid
/// under both seeds; winner draws `[5]`, alternate draws otherwise).
const ALT_SEED: u64 = 286;

fn u(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

fn hero() -> Ulid {
    Ulid::from_u128(34)
}

fn goblin() -> Ulid {
    Ulid::from_u128(35)
}

fn heavy() -> Ulid {
    Ulid::from_u128(39)
}

fn focus() -> Ulid {
    Ulid::from_u128(40)
}

fn encounter() -> Ulid {
    Ulid::from_u128(44)
}

fn effect() -> Ulid {
    Ulid::from_u128(43)
}

fn init_payload() -> serde_json::Value {
    json!({
        "combat": 1,
        "op": "init",
        "encounter": encounter().to_string(),
    })
}

fn attack_payload(actor: Ulid, ability: Ulid, target: Ulid) -> serde_json::Value {
    json!({
        "combat": 1,
        "op": "attack",
        "actor": actor.to_string(),
        "ability": ability.to_string(),
        "target": target.to_string(),
    })
}

fn end_payload(actor: Ulid) -> serde_json::Value {
    json!({
        "combat": 1,
        "op": "end",
        "actor": actor.to_string(),
    })
}

/// The authored schedule: tick 0 init; ticks 1-2 goblin self-focus; tick 3
/// goblin heavy on hero; tick 4 hero end; tick 5 goblin heavy on hero;
/// tick 6 hero end; odd ticks 7, 9 goblin heavy on hero; even ticks 8, 10,
/// 12 hero heavy on goblin; terminal prefix ends at tick 9 with the hero
/// dead, then four trailing ticks.
fn srd_inputs() -> Vec<ReplayInput> {
    let g = goblin();
    let h = hero();
    vec![
        ReplayInput {
            tick: 0,
            payload: init_payload(),
        },
        ReplayInput {
            tick: 1,
            payload: attack_payload(g, focus(), g),
        },
        ReplayInput {
            tick: 2,
            payload: attack_payload(g, focus(), g),
        },
        ReplayInput {
            tick: 3,
            payload: attack_payload(g, heavy(), h),
        },
        ReplayInput {
            tick: 4,
            payload: end_payload(h),
        },
        ReplayInput {
            tick: 5,
            payload: attack_payload(g, heavy(), h),
        },
        ReplayInput {
            tick: 6,
            payload: end_payload(h),
        },
        ReplayInput {
            tick: 7,
            payload: attack_payload(g, heavy(), h),
        },
        ReplayInput {
            tick: 8,
            payload: attack_payload(h, heavy(), g),
        },
        ReplayInput {
            tick: 9,
            payload: attack_payload(g, heavy(), h),
        },
    ]
}

/// The in-memory replay the checked-in fixture must equal.
fn srd_replay() -> Replay {
    Replay::new(
        REPLAY_FORMAT_VERSION,
        SEED,
        SRD_CAMPAIGN_ID.to_string(),
        SRD_CAMPAIGN_VERSION.to_string(),
        SRD_ENGINE_VERSION.to_string(),
        TOTAL_TICKS,
        srd_inputs(),
    )
    .expect("srd replay must validate")
}

/// Reviewed literal trace (seed 285): faces pinned by the probe, every other
/// field independently recomputed in `independent_arithmetic_from_faces`.
fn expected_attacks() -> Vec<SrdTraceEntry> {
    let g = goblin();
    let h = hero();
    vec![
        SrdTraceEntry {
            tick: 1,
            actor: g,
            target: g,
            ability: focus(),
            faces: vec![5],
            total: 5,
            margin: -1,
            outcome: Outcome::Success,
            damage: 0,
            target_died: false,
            target_health: 6,
            active: Some(g),
        },
        SrdTraceEntry {
            tick: 2,
            actor: g,
            target: g,
            ability: focus(),
            faces: vec![6],
            total: 6,
            margin: 3,
            outcome: Outcome::Failure,
            damage: 0,
            target_died: false,
            target_health: 6,
            active: Some(g),
        },
        SrdTraceEntry {
            tick: 3,
            actor: g,
            target: h,
            ability: heavy(),
            faces: vec![4, 1],
            total: 5,
            margin: 1,
            outcome: Outcome::Success,
            damage: 3,
            target_died: false,
            target_health: 7,
            active: Some(h),
        },
        SrdTraceEntry {
            tick: 5,
            actor: g,
            target: h,
            ability: heavy(),
            faces: vec![5, 6],
            total: 11,
            margin: 7,
            outcome: Outcome::CriticalSuccess,
            damage: 5,
            target_died: false,
            target_health: 2,
            active: Some(h),
        },
        SrdTraceEntry {
            tick: 7,
            actor: g,
            target: h,
            ability: heavy(),
            faces: vec![1, 1],
            total: 2,
            margin: -5,
            outcome: Outcome::Failure,
            damage: 0,
            target_died: false,
            target_health: 2,
            active: Some(h),
        },
        SrdTraceEntry {
            tick: 8,
            actor: h,
            target: g,
            ability: heavy(),
            faces: vec![4, 4],
            total: 8,
            margin: 3,
            outcome: Outcome::CriticalSuccess,
            damage: 5,
            target_died: false,
            target_health: 1,
            active: Some(g),
        },
        SrdTraceEntry {
            tick: 9,
            actor: g,
            target: h,
            ability: heavy(),
            faces: vec![6, 2],
            total: 8,
            margin: 1,
            outcome: Outcome::CriticalSuccess,
            damage: 5,
            target_died: true,
            target_health: 0,
            active: None,
        },
    ]
}

fn participant(
    placement: Ulid,
    health: u32,
    dead: bool,
    primary: u32,
    second: u32,
    attached: Vec<(Ulid, u32)>,
) -> ParticipantObservation {
    ParticipantObservation {
        placement,
        health,
        dead,
        pools: vec![(u(46), primary), (u(47), second)],
        attached,
    }
}

fn state(
    round: Option<u32>,
    active: Option<Ulid>,
    hero: ParticipantObservation,
    goblin: ParticipantObservation,
) -> SrdStateObservation {
    SrdStateObservation {
        round,
        active,
        participants: vec![hero, goblin],
    }
}

fn live(placement: Ulid, health: u32, primary: u32, second: u32) -> ParticipantObservation {
    participant(placement, health, false, primary, second, vec![])
}

/// Expected before/after state per input index (0..=9), derived from the
/// pinned faces plus literal authored values (health 10/6, pools 1/2,
/// attachment expiry at round 2, round rollovers on empty timelines).
fn expected_states() -> Vec<(SrdStateObservation, SrdStateObservation)> {
    let h = hero();
    let g = goblin();
    let e43 = (effect(), 2);
    let empty = SrdStateObservation {
        round: None,
        active: None,
        participants: vec![],
    };
    vec![
        // 0: init (pre-init observation carries no participants).
        (
            empty,
            state(Some(0), Some(g), live(h, 10, 1, 2), live(g, 6, 1, 2)),
        ),
        // 1: G focus self — pool 47 2→1, effect attached expiring at 2.
        (
            state(Some(0), Some(g), live(h, 10, 1, 2), live(g, 6, 1, 2)),
            state(
                Some(0),
                Some(g),
                live(h, 10, 1, 2),
                participant(g, 6, false, 1, 1, vec![e43]),
            ),
        ),
        // 2: G focus self — pool 47 1→0, single attachment refreshed at 2.
        (
            state(
                Some(0),
                Some(g),
                live(h, 10, 1, 2),
                participant(g, 6, false, 1, 1, vec![e43]),
            ),
            state(
                Some(0),
                Some(g),
                live(h, 10, 1, 2),
                participant(g, 6, false, 1, 0, vec![e43]),
            ),
        ),
        // 3: G heavy→H — pool 46 spent, 3 damage, turn passes to H.
        (
            state(
                Some(0),
                Some(g),
                live(h, 10, 1, 2),
                participant(g, 6, false, 1, 0, vec![e43]),
            ),
            state(
                Some(0),
                Some(h),
                live(h, 7, 1, 2),
                participant(g, 6, false, 0, 0, vec![e43]),
            ),
        ),
        // 4: H end — timeline empties, round 1, both pools refreshed.
        (
            state(
                Some(0),
                Some(h),
                live(h, 7, 1, 2),
                participant(g, 6, false, 0, 0, vec![e43]),
            ),
            state(
                Some(1),
                Some(g),
                live(h, 7, 1, 2),
                participant(g, 6, false, 1, 2, vec![e43]),
            ),
        ),
        // 5: G heavy→H — 5 damage, turn passes to H.
        (
            state(
                Some(1),
                Some(g),
                live(h, 7, 1, 2),
                participant(g, 6, false, 1, 2, vec![e43]),
            ),
            state(
                Some(1),
                Some(h),
                live(h, 2, 1, 2),
                participant(g, 6, false, 0, 2, vec![e43]),
            ),
        ),
        // 6: H end — timeline empties, round 2, attachment expires, pools set.
        (
            state(
                Some(1),
                Some(h),
                live(h, 2, 1, 2),
                participant(g, 6, false, 0, 2, vec![e43]),
            ),
            state(Some(2), Some(g), live(h, 2, 1, 2), live(g, 6, 1, 2)),
        ),
        // 7: G heavy→H — no bonus, no damage, turn passes to H.
        (
            state(Some(2), Some(g), live(h, 2, 1, 2), live(g, 6, 1, 2)),
            state(
                Some(2),
                Some(h),
                live(h, 2, 1, 2),
                participant(g, 6, false, 0, 2, vec![]),
            ),
        ),
        // 8: H heavy→G — 5 damage, round 3, turn passes to G.
        (
            state(
                Some(2),
                Some(h),
                live(h, 2, 1, 2),
                participant(g, 6, false, 0, 2, vec![]),
            ),
            state(
                Some(3),
                Some(g),
                live(h, 2, 0, 2),
                participant(g, 1, false, 1, 2, vec![]),
            ),
        ),
        // 9: G heavy→H — lethal, terminal, no advance.
        (
            state(
                Some(3),
                Some(g),
                live(h, 2, 0, 2),
                participant(g, 1, false, 1, 2, vec![]),
            ),
            state(
                Some(3),
                None,
                participant(h, 0, true, 0, 2, vec![]),
                participant(g, 1, false, 0, 2, vec![]),
            ),
        ),
    ]
}

fn remove_silently(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

fn snapshot(world: &World) -> Vec<u8> {
    serde_json::to_vec(world).expect("the world serializes")
}

#[test]
fn production_content_loads_and_initializes() {
    let content = SrdContent::load().expect("combat_srd loads");
    assert_eq!(content.files.len(), 17);
    assert_eq!(content.loaded.index.len(), 14);
    assert!(crpg_data::validate(&content.loaded).is_empty());
    // Canonical parity is asserted inside load; spot-check the pins here.
    let ruleset = content
        .loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            crpg_data::Document::Ruleset(v) => Some(v),
            _ => None,
        })
        .expect("ruleset present");
    assert_eq!(ruleset.attributes, vec!["might", "guile", "resolve"]);
    assert_eq!(ruleset.health_stat, "health");
    assert_eq!(ruleset.pools.len(), 2);
    assert_eq!(ruleset.abilities.len(), 2);
    let effect = content
        .loaded
        .documents
        .values()
        .find_map(|doc| match doc {
            crpg_data::Document::Effect(v) => Some(v),
            _ => None,
        })
        .expect("effect present");
    assert_eq!(effect.duration_rounds, 2);
    assert_eq!(effect.modifiers.len(), 2);

    let content = SrdContent::load().expect("reloads");
    let (apply, _) = srd_apply(content);
    // Play only the init input to pin publication.
    let init_only = Replay::new(
        REPLAY_FORMAT_VERSION,
        SEED,
        SRD_CAMPAIGN_ID.to_string(),
        SRD_CAMPAIGN_VERSION.to_string(),
        SRD_ENGINE_VERSION.to_string(),
        1,
        vec![srd_inputs()[0].clone()],
    )
    .expect("init replay validates");
    let hashes = play_replay(&init_only, apply).expect("init plays");
    assert_eq!(hashes.len(), 1);
    // Re-drive init directly to inspect the published world.
    let content = SrdContent::load().expect("reloads");
    let mut adapter = SrdAdapter::new(content);
    let mut world = World::new(SEED);
    let out = adapter
        .apply(&mut world, &init_payload())
        .expect("init applies");
    assert!(out.is_none());
    assert!(adapter.initialized);
    assert_eq!(adapter.entities.len(), 2);
    let ids: Vec<_> = world.combatants().iter().map(|(id, _)| id).collect();
    assert_eq!(ids.len(), 2);
    // Authored participant order: hero (34) then goblin (35).
    assert_eq!(
        world.combatants().get(ids[0]).expect("hero").placement(),
        hero()
    );
    assert_eq!(
        world.combatants().get(ids[1]).expect("goblin").placement(),
        goblin()
    );
    assert_eq!(
        world.combatants().get(ids[0]).expect("hero").initiative(),
        10
    );
    assert_eq!(
        world.combatants().get(ids[1]).expect("goblin").initiative(),
        9
    );
    assert_eq!(world.combatants().get(ids[0]).expect("hero").health(), 10);
    assert_eq!(world.combatants().get(ids[1]).expect("goblin").health(), 6);
    assert_eq!(
        world
            .combatants()
            .get(ids[0])
            .expect("hero")
            .action_pool()
            .current(),
        1
    );
    assert_eq!(
        world
            .combatants()
            .get(ids[1])
            .expect("goblin")
            .action_pool()
            .current(),
        1
    );
    assert_eq!(
        world.combatants().get(ids[0]).expect("hero").extra_pools()[0].current(),
        2
    );
    assert_eq!(
        world
            .combatants()
            .get(ids[1])
            .expect("goblin")
            .extra_pools()[0]
            .current(),
        2
    );
    let combat = world.combat().expect("encounter active");
    assert_eq!(combat.round, 0);
    // Lower initiative acts first: the goblin holds the turn.
    assert_eq!(combat.active, Some(ids[1]));
    assert_eq!(combat.definition.ability, heavy());
    assert_eq!(combat.definition.abilities.len(), 2);
    assert_eq!(combat.definition.pools.len(), 2);
    assert_eq!(combat.definition.effects.len(), 1);
}

#[test]
fn checked_in_replay_matches_authored_schedule() {
    let file = read_replay(&srd_replay_path()).expect("fixture parses");
    assert_eq!(file, srd_replay());
    assert_eq!(file.seed, SEED);
    assert_eq!(file.campaign_id, SRD_CAMPAIGN_ID);
    assert_eq!(file.total_ticks, TOTAL_TICKS);
    assert_eq!(file.inputs.len(), (K + 1) as usize);
    assert_eq!(file.inputs[0].tick, 0);
    for (step, input) in file.inputs.iter().enumerate().skip(1) {
        assert_eq!(input.tick, step as u64);
    }
}

#[test]
fn exact_action_trace_matches_reviewed_oracle() {
    let content = SrdContent::load().expect("content loads");
    let (apply, observations) = srd_apply(content);
    let replay = srd_replay();
    let hashes = play_replay(&replay, apply).expect("srd plays");
    assert_eq!(hashes.len(), TOTAL_TICKS as usize);
    let guard = observations.borrow();
    let attacks: Vec<SrdTraceEntry> = guard
        .inputs
        .iter()
        .filter_map(|input| input.attack.clone())
        .collect();
    assert_eq!(attacks, expected_attacks());
    assert_eq!(guard.inputs.len(), (K + 1) as usize);
    // Every input carries its tick, index, and before/after states.
    let expected = expected_states();
    assert_eq!(guard.inputs.len(), expected.len());
    for (index, input) in guard.inputs.iter().enumerate() {
        assert_eq!(input.tick, index as u64);
        assert_eq!(input.input_index, index);
        assert_eq!(input.before, expected[index].0, "input {index} before");
        assert_eq!(input.after, expected[index].1, "input {index} after");
        assert_eq!(
            input.attack,
            expected_attacks()
                .iter()
                .find(|entry| entry.tick == index as u64)
                .cloned(),
            "input {index} attack"
        );
    }
    // Snapshot continuity: after-bytes of one input equal before-bytes of
    // the next up to the harness tick advance, so playback hides nothing.
    for pair in guard.inputs.windows(2) {
        let after: serde_json::Value =
            serde_json::from_slice(&pair[0].after_bytes).expect("after parses");
        let before: serde_json::Value =
            serde_json::from_slice(&pair[1].before_bytes).expect("before parses");
        let mut after_map = after.as_object().cloned().expect("an object");
        let mut before_map = before.as_object().cloned().expect("an object");
        let after_tick = after_map.remove("tick").expect("a tick");
        let before_tick = before_map.remove("tick").expect("a tick");
        assert_eq!(after_map, before_map, "only the tick advances");
        assert_eq!(
            after_tick.as_u64().expect("a tick") + 1,
            before_tick.as_u64().expect("a tick")
        );
    }
    // Terminal identity: the hero died at tick 9, the goblin survived at 1.
    let last = attacks.last().expect("lethal trace");
    assert!(last.target_died);
    assert_eq!(last.actor, goblin());
    assert_eq!(last.target, hero());
    assert_eq!(last.target_health, 0);
    assert_eq!(last.active, None);
}

#[test]
fn independent_arithmetic_from_faces() {
    // Literal authored values (T017a final map): might 8/6, ward 7/5,
    // health 10/6; heavy bands MIN/F, -2/S, 2/C with a face-6 shift on die
    // 0; focus bands MIN/S, 1/F; damage {F 0, S 3, C 5} else 0; E1 is
    // Roll +2 (priority 0) and +1 (priority 1), duration 2; pools 46 (max
    // 1, turn) and 47 (max 2, round). Attachment by round, tracked
    // independently: G attached ticks 1-6 (rounds 0-1), expired from
    // round 2 on; H never attached.
    // Tick 1's own focus rolls before its attachment lands, so the +3
    // applies to ticks 2..=6 only (rounds 0-1 attached, expired at 2).
    let attached = |tick: u64| (2..=6).contains(&tick);
    let content = SrdContent::load().expect("content loads");
    let (apply, observations) = srd_apply(content);
    play_replay(&srd_replay(), apply).expect("srd plays");
    let guard = observations.borrow();
    let attacks: Vec<SrdTraceEntry> = guard
        .inputs
        .iter()
        .filter_map(|input| input.attack.clone())
        .collect();
    assert_eq!(attacks.len(), expected_attacks().len());
    for entry in &attacks {
        let (defense, bonus) = if entry.ability == heavy() {
            let ward = if entry.target == hero() { 7 } else { 5 };
            (
                ward,
                if attached(entry.tick) && entry.actor == goblin() {
                    3
                } else {
                    0
                },
            )
        } else {
            // Focus defends on the actor's might; only the goblin focuses.
            (6, if attached(entry.tick) { 3 } else { 0 })
        };
        let expected_modified = entry.total + bonus;
        let expected_margin = i64::from(expected_modified) - i64::from(defense);
        assert_eq!(entry.margin, expected_margin, "tick {} margin", entry.tick);
        // Bands are per ability: heavy [{MIN:F}, {-2:S}, {2:C}] with a
        // face-6 shift on die 0; focus [{MIN:S}, {1:F}] with no shift.
        let (expected_outcome, _shifted) = if entry.ability == heavy() {
            let band = if expected_margin < -2 {
                0
            } else if expected_margin < 2 {
                1
            } else {
                2
            };
            let shifted = if entry.faces[0] == 6 {
                (band + 1).min(2)
            } else {
                band
            };
            let outcome = match shifted {
                0 => Outcome::Failure,
                1 => Outcome::Success,
                _ => Outcome::CriticalSuccess,
            };
            (outcome, shifted)
        } else {
            let outcome = if expected_margin < 1 {
                Outcome::Success
            } else {
                Outcome::Failure
            };
            (outcome, 0)
        };
        assert_eq!(
            entry.outcome, expected_outcome,
            "tick {} outcome",
            entry.tick
        );
        let expected_damage = match expected_outcome {
            Outcome::Failure => 0,
            Outcome::Success => {
                if entry.ability == heavy() {
                    3
                } else {
                    0
                }
            }
            Outcome::CriticalSuccess if entry.ability == heavy() => 5,
            _ => 0,
        };
        assert_eq!(entry.damage, expected_damage, "tick {} damage", entry.tick);
    }
    // The shift visibly changes tick 9 (Success would-become Critical), and
    // tick 7 proves the post-expiry margin carries no bonus.
    let tick9 = attacks
        .iter()
        .find(|entry| entry.tick == 9)
        .expect("tick 9");
    assert_eq!(tick9.faces[0], 6);
    assert_eq!(tick9.margin, 1);
    assert_eq!(tick9.outcome, Outcome::CriticalSuccess);
    let tick7 = attacks
        .iter()
        .find(|entry| entry.tick == 7)
        .expect("tick 7");
    assert_eq!(tick7.margin, i64::from(tick7.total) - 7);
}

#[test]
fn identical_inputs_repeat_and_alt_seed_diverges() {
    let prefix: Vec<ReplayInput> = srd_inputs()[..=2].to_vec();
    let run = |seed: u64| {
        let replay = Replay::new(
            REPLAY_FORMAT_VERSION,
            seed,
            SRD_CAMPAIGN_ID.to_string(),
            SRD_CAMPAIGN_VERSION.to_string(),
            SRD_ENGINE_VERSION.to_string(),
            3,
            prefix.clone(),
        )
        .expect("prefix validates");
        let content = SrdContent::load().expect("loads");
        let (apply, observations) = srd_apply(content);
        let hashes = play_replay(&replay, apply).expect("prefix plays");
        let guard = observations.borrow();
        (
            hashes,
            guard
                .inputs
                .iter()
                .filter_map(|input| input.attack.clone())
                .collect::<Vec<_>>(),
        )
    };
    let (hashes, trace) = run(SEED);
    let (repeat_hashes, repeat_trace) = run(SEED);
    assert_eq!(hashes, repeat_hashes);
    assert_eq!(trace, repeat_trace);
    assert_eq!(trace[0].faces, vec![5]);
    let (alt_hashes, alt_trace) = run(ALT_SEED);
    assert_ne!(alt_hashes, hashes);
    assert_ne!(alt_trace[0].faces, trace[0].faces);
    // Both prefixes stay valid: two accepted focuses, turn held throughout.
    assert_eq!(alt_trace.len(), 2);
    assert_eq!(alt_trace[0].outcome, Outcome::Success);
    assert_eq!(alt_trace[0].active, Some(goblin()));
}

#[test]
fn rejected_spend_and_end_rejections_preserve_full_state() {
    let content = SrdContent::load().expect("loads");
    let mut adapter = SrdAdapter::new(content);
    let mut world = World::new(SEED);
    adapter.apply(&mut world, &init_payload()).expect("init");
    let focus_g = attack_payload(goblin(), focus(), goblin());
    adapter.apply(&mut world, &focus_g).expect("tick 1");
    adapter.apply(&mut world, &focus_g).expect("tick 2");
    // The third focus fails pool-47 insufficiency with zero mutation,
    // including the already-existing combat stream.
    assert!(world.rng_mut().has_stream("combat.roll"));
    let before = snapshot(&world);
    let bindings = adapter.entities.clone();
    assert_eq!(
        adapter.apply(&mut world, &focus_g),
        Err("attack failed: InsufficientAction at combat/pool".to_string())
    );
    assert_eq!(before, snapshot(&world));
    assert_eq!(bindings, adapter.entities);
    assert!(world.rng_mut().has_stream("combat.roll"));
    // Out-of-turn end by the hero while the goblin holds the turn.
    let early = end_payload(hero());
    let before = snapshot(&world);
    assert_eq!(
        adapter.apply(&mut world, &early),
        Err("end failed: OutOfTurn at combat/active".to_string())
    );
    assert_eq!(before, snapshot(&world));
    // Unknown-actor end names a live outsider.
    let outsider = world.spawn(crpg_sim::EntityMeta {});
    let intruder = end_payload(Ulid::from_u128(999));
    let before = snapshot(&world);
    assert!(adapter.apply(&mut world, &intruder).is_err());
    assert_eq!(before, snapshot(&world));
    assert!(world.despawn(outsider));
    // Dead-actor end on the terminal world: death outranks turn order.
    let content = SrdContent::load().expect("reloads");
    let (apply, _) = srd_apply(content);
    play_replay(&srd_replay(), apply).expect("full run plays");
    let content = SrdContent::load().expect("reloads");
    let mut adapter = SrdAdapter::new(content);
    let mut terminal = World::new(SEED);
    for input in srd_inputs() {
        adapter
            .apply(&mut terminal, &input.payload)
            .expect("replays");
        tick(&mut terminal);
    }
    let corpse = end_payload(hero());
    let before = snapshot(&terminal);
    let corpse_entity = adapter.entities.get(&hero()).copied().expect("hero bound");
    assert_eq!(
        adapter.apply(&mut terminal, &corpse),
        Err(format!(
            "end failed: DeadActor at combatants/{corpse_entity:?}"
        ))
    );
    assert_eq!(before, snapshot(&terminal));
}

#[test]
fn accepted_ends_transition_exactly() {
    let content = SrdContent::load().expect("loads");
    let (apply, observations) = srd_apply(content);
    play_replay(&srd_replay(), apply).expect("srd plays");
    let guard = observations.borrow();
    // Both ends return None with exact scheduling/round deltas.
    for index in [4usize, 6] {
        let input = &guard.inputs[index];
        assert_eq!(input.attack, None);
        assert_eq!(input.tick, index as u64);
    }
    assert_eq!(guard.inputs[4].before.round, Some(0));
    assert_eq!(guard.inputs[4].after.round, Some(1));
    assert_eq!(guard.inputs[4].before.active, Some(hero()));
    assert_eq!(guard.inputs[4].after.active, Some(goblin()));
    assert_eq!(guard.inputs[6].before.round, Some(1));
    assert_eq!(guard.inputs[6].after.round, Some(2));
    assert_eq!(guard.inputs[6].before.active, Some(hero()));
    assert_eq!(guard.inputs[6].after.active, Some(goblin()));
    // Tick 6 expires the attachment while refreshing both pools.
    assert_eq!(
        guard.inputs[6].before.participants[1].attached,
        vec![(effect(), 2)]
    );
    assert!(guard.inputs[6].after.participants[1].attached.is_empty());
    // Ends advance no RNG stream: the serialized RNG substate is identical.
    for index in [4usize, 6] {
        let before: serde_json::Value =
            serde_json::from_slice(&guard.inputs[index].before_bytes).expect("parses");
        let after: serde_json::Value =
            serde_json::from_slice(&guard.inputs[index].after_bytes).expect("parses");
        assert_eq!(
            before["rng"], after["rng"],
            "end input {index} spends no draw"
        );
    }
}

#[test]
fn effect_expiry_with_save_load_continuation() {
    // Reference hashes from the uninterrupted frozen replay.
    let content = SrdContent::load().expect("loads");
    let (apply, _) = srd_apply(content);
    let full = play_replay(&srd_replay(), apply).expect("full run plays");
    assert_eq!(full.len(), TOTAL_TICKS as usize);
    // Prefix replay through tick 4 must reproduce the reference prefix.
    let prefix = Replay::new(
        REPLAY_FORMAT_VERSION,
        SEED,
        SRD_CAMPAIGN_ID.to_string(),
        SRD_CAMPAIGN_VERSION.to_string(),
        SRD_ENGINE_VERSION.to_string(),
        5,
        srd_inputs()[..=4].to_vec(),
    )
    .expect("prefix validates");
    let content = SrdContent::load().expect("reloads");
    let (apply, observations) = srd_apply(content);
    let prefix_hashes = play_replay(&prefix, apply).expect("prefix plays");
    assert_eq!(prefix_hashes, full[..5]);
    // The observer's detached world is post-input tick 4, pre-tick.
    let guard = observations.borrow();
    let mut checkpoint = guard.last_world.clone().expect("a checkpoint");
    drop(guard);
    assert_eq!(checkpoint.tick().get(), 4);
    tick(&mut checkpoint);
    assert_eq!(checkpoint.tick().get(), 5);
    assert_eq!(checkpoint.combat().expect("combat").round, 1);
    assert_eq!(state_hash(&checkpoint), full[4]);
    let bytes = snapshot(&checkpoint);
    let mut resumed: World = serde_json::from_slice(&bytes).expect("checkpoint loads");
    assert_eq!(snapshot(&resumed), bytes);
    assert_eq!(state_hash(&resumed), state_hash(&checkpoint));
    // Continue both worlds over ticks 5..TOTAL_TICKS with the frozen inputs.
    let content_a = SrdContent::load().expect("reloads");
    let content_b = SrdContent::load().expect("reloads");
    let mut adapter_a = SrdAdapter::rebind(content_a, &checkpoint).expect("rebinds");
    let mut adapter_b = SrdAdapter::rebind(content_b, &resumed).expect("rebinds");
    let inputs = srd_inputs();
    for t in 5..TOTAL_TICKS {
        if let Some(input) = inputs.iter().find(|candidate| candidate.tick == t) {
            let first = adapter_a
                .apply(&mut checkpoint, &input.payload)
                .expect("continues");
            let second = adapter_b
                .apply(&mut resumed, &input.payload)
                .expect("resumed continues");
            assert_eq!(first, second, "tick {t} observations agree");
        }
        tick(&mut checkpoint);
        tick(&mut resumed);
        assert_eq!(
            state_hash(&checkpoint),
            full[t as usize],
            "tick {t} checkpoint"
        );
        assert_eq!(state_hash(&resumed), full[t as usize], "tick {t} resumed");
    }
    // Bonus at 5, expiry at 6, no bonus at 7 — re-observed on a fresh
    // full run below so the continued worlds above stay untouched.
    let content = SrdContent::load().expect("reloads");
    let (apply, observations) = srd_apply(content);
    play_replay(&srd_replay(), apply).expect("full run plays");
    let guard = observations.borrow();
    let attacks: Vec<SrdTraceEntry> = guard
        .inputs
        .iter()
        .filter_map(|input| input.attack.clone())
        .collect();
    let tick5 = attacks
        .iter()
        .find(|entry| entry.tick == 5)
        .expect("tick 5");
    assert_eq!(tick5.margin, i64::from(tick5.total) + 3 - 7);
    let tick7 = attacks
        .iter()
        .find(|entry| entry.tick == 7)
        .expect("tick 7");
    assert_eq!(tick7.margin, i64::from(tick7.total) - 7);
    let end6 = guard
        .inputs
        .iter()
        .find(|input| input.tick == 6)
        .expect("tick 6");
    assert!(end6.attack.is_none());
    assert!(end6
        .after
        .participants
        .iter()
        .all(|entry| entry.attached.is_empty()));
}

#[test]
fn terminal_trailing_exactly_once_death() {
    let content = SrdContent::load().expect("loads");
    let (apply, _) = srd_apply(content);
    let hashes = play_replay(&srd_replay(), apply).expect("srd plays");
    assert_eq!(hashes.len(), TOTAL_TICKS as usize);
    // Manual drive keeps the adapter for terminal probing.
    let content = SrdContent::load().expect("reloads");
    let mut adapter = SrdAdapter::new(content);
    let mut world = World::new(SEED);
    for input in srd_inputs() {
        adapter.apply(&mut world, &input.payload).expect("replays");
        tick(&mut world);
    }
    for _ in 0..T {
        tick(&mut world);
    }
    assert_eq!(state_hash(&world), hashes[(TOTAL_TICKS - 1) as usize]);
    let combat = world.combat().expect("combat");
    assert_eq!(combat.active, None);
    let dead: Vec<_> = world
        .combatants()
        .iter()
        .filter(|(_, state)| state.dead())
        .collect();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].1.placement(), hero());
    assert_eq!(dead[0].1.health(), 0);
    for (entity, state) in world.combatants().iter() {
        if entity != dead[0].0 {
            assert!(!state.dead());
            assert!(state.health() > 0);
        }
    }
    // Trailing attacks fail without mutation: corpse as target, then actor.
    // End the `dead` borrow before taking `&mut world` again.
    let corpse_entity = dead[0].0;
    std::mem::drop(dead);
    let survivor = adapter
        .entities
        .get(&goblin())
        .copied()
        .expect("goblin bound");
    let before = snapshot(&world);
    assert_eq!(
        adapter.apply(&mut world, &attack_payload(goblin(), heavy(), hero())),
        Err(format!(
            "attack failed: DeadTarget at combatants/{corpse_entity:?}"
        ))
    );
    assert_eq!(before, snapshot(&world));
    assert_eq!(
        adapter.apply(&mut world, &attack_payload(hero(), heavy(), goblin())),
        Err(format!(
            "attack failed: DeadActor at combatants/{corpse_entity:?}"
        ))
    );
    assert_eq!(before, snapshot(&world));
    let _ = survivor;
    // Death notification appears exactly once, observed via a detached clone.
    let mut clone = world.clone();
    let mut spawned = 0;
    let mut deaths = 0;
    for envelope in clone.events_mut().drain() {
        match envelope.payload {
            crpg_sim::SimEvent::Died { entity } => {
                deaths += 1;
                assert_eq!(entity, corpse_entity);
            }
            crpg_sim::SimEvent::Spawned { .. } => spawned += 1,
            other => panic!("unexpected event {other:?}"),
        }
    }
    assert_eq!(spawned, 2);
    assert_eq!(deaths, 1);
}

#[test]
fn golden_negatives_fail_without_skips() {
    let hashes = play_replay(
        &srd_replay(),
        srd_apply(SrdContent::load().expect("loads")).0,
    )
    .expect("plays");
    // Missing golden is Io(NotFound), never a skip.
    let missing = srd_temp_file("missing.golden");
    remove_silently(&missing);
    match verify_golden(&missing, &hashes) {
        Err(crpg_testkit::HarnessError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
        other => panic!("expected Io(NotFound), got {other:?}"),
    }
    // Truncated golden is GoldenShort at the shorter length.
    let short_path = srd_temp_file("short.golden");
    remove_silently(&short_path);
    write_golden(&short_path, &hashes[..hashes.len() - 1]).expect("writes short");
    match verify_golden(&short_path, &hashes) {
        Err(crpg_testkit::HarnessError::Mismatch(Mismatch::GoldenShort { tick, actual })) => {
            assert_eq!(tick, hashes.len() - 1);
            assert_eq!(actual, hashes[hashes.len() - 1]);
        }
        other => panic!("expected GoldenShort, got {other:?}"),
    }
    remove_silently(&short_path);
    // Wrong hash is Diverged with truthful sides.
    let wrong_path = srd_temp_file("wrong.golden");
    remove_silently(&wrong_path);
    let mut wrong = hashes.clone();
    wrong[1] = [9u8; 32];
    write_golden(&wrong_path, &wrong).expect("writes wrong");
    match verify_golden(&wrong_path, &hashes) {
        Err(crpg_testkit::HarnessError::Mismatch(Mismatch::Diverged {
            tick,
            expected,
            actual,
        })) => {
            assert_eq!(tick, 1);
            assert_eq!(expected, wrong[1]);
            assert_eq!(actual, hashes[1]);
        }
        other => panic!("expected Diverged, got {other:?}"),
    }
    remove_silently(&wrong_path);
}

/// Independent native baseline (compile-time selection, never runtime OS
/// detection, never cross-platform equality, never tolerance).
#[cfg(any(
    all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc",
        debug_assertions
    ),
    all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu",
        debug_assertions
    )
))]
#[test]
fn native_target_golden_replay() {
    let hashes = play_and_verify(
        &srd_replay_path(),
        &srd::srd_target_golden_path(),
        srd_apply(SrdContent::load().expect("loads")).0,
    )
    .expect("native target golden replay must match; missing baseline is an error");
    assert_eq!(hashes.len(), TOTAL_TICKS as usize);
}

#[test]
fn portable_repeatability_without_golden() {
    let replay = read_replay(&srd_replay_path()).expect("fixture parses");
    let first =
        play_replay(&replay, srd_apply(SrdContent::load().expect("loads")).0).expect("plays");
    let second =
        play_replay(&replay, srd_apply(SrdContent::load().expect("loads")).0).expect("plays");
    assert_eq!(first, second);
    assert_eq!(first.len(), replay.total_ticks as usize);
    assert_eq!(first.len(), TOTAL_TICKS as usize);
}

#[test]
fn envelope_and_grammar_boundaries() {
    // Out-of-range tick is rejected at construction.
    assert!(Replay::new(
        REPLAY_FORMAT_VERSION,
        SEED,
        SRD_CAMPAIGN_ID.to_string(),
        SRD_CAMPAIGN_VERSION.to_string(),
        SRD_ENGINE_VERSION.to_string(),
        TOTAL_TICKS,
        vec![ReplayInput {
            tick: TOTAL_TICKS,
            payload: init_payload(),
        }],
    )
    .is_err());
    // Unordered schedule is rejected at construction.
    let mut inputs = srd_inputs();
    inputs.swap(1, 2);
    assert!(Replay::new(
        REPLAY_FORMAT_VERSION,
        SEED,
        SRD_CAMPAIGN_ID.to_string(),
        SRD_CAMPAIGN_VERSION.to_string(),
        SRD_ENGINE_VERSION.to_string(),
        TOTAL_TICKS,
        inputs,
    )
    .is_err());
    // Unknown op, bad version, and extra fields fail loudly by op.
    let content = SrdContent::load().expect("loads");
    let mut adapter = SrdAdapter::new(content);
    let mut world = World::new(SEED);
    for (payload, _reason) in [
        (
            json!({"combat": 1, "op": "rest", "actor": "x"}),
            "unknown op",
        ),
        (
            json!({"combat": 2, "op": "init", "encounter": "x"}),
            "version",
        ),
        (
            json!({"combat": 1, "op": "end", "actor": hero().to_string(), "ability": "x"}),
            "extra field",
        ),
        (json!({"combat": 1, "op": "end"}), "missing actor"),
        (
            json!({"combat": 1, "op": "end", "actor": "not-a-ulid"}),
            "non-ULID actor",
        ),
        (
            json!({"combat": 1, "op": "attack", "actor": hero().to_string()}),
            "missing ability",
        ),
    ] {
        assert!(adapter.apply(&mut world, &payload).is_err(), "{_reason}");
    }
    // Duplicate init fails without mutation past the accepted init.
    assert!(adapter.apply(&mut world, &init_payload()).is_ok());
    let before = snapshot(&world);
    assert!(adapter.apply(&mut world, &init_payload()).is_err());
    assert_eq!(before, snapshot(&world));
    let content = SrdContent::load().expect("reloads");
    let mut adapter = SrdAdapter::new(content);
    let mut world = World::new(SEED);
    let before = snapshot(&world);
    assert!(adapter
        .apply(&mut world, &attack_payload(goblin(), heavy(), hero()))
        .is_err());
    assert_eq!(before, snapshot(&world));
}
