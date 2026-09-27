#![forbid(unsafe_code)]
//! Headless combat replay integration (T016c).
//!
//! Cross-layer proof that authored `combat_basic` content drives the T016b
//! controller through the unmodified generic replay harness: production
//! loading, a caller-owned combat adapter, the checked-in
//! `combat_basic.replay` artifact, independently generated native goldens,
//! and terminal/trailing/rollback/failure coverage. No downstream CLI change.

#[path = "support/combat.rs"]
mod combat;

use combat::{
    combat_apply, combat_replay_path, combat_temp_file, CombatAdapter, CombatContent, TraceEntry,
    COMBAT_CAMPAIGN_ID, COMBAT_CAMPAIGN_VERSION, COMBAT_ENGINE_VERSION,
};
use crpg_core::Ulid;
use crpg_rules::Outcome;
use crpg_sim::{state_hash, tick, World};
use crpg_testkit::{
    play_and_verify, play_replay, read_replay, verify_golden, write_golden, write_replay, Mismatch,
    Replay, ReplayError, ReplayInput, REPLAY_FORMAT_VERSION,
};
use serde_json::json;

/// Pinned seed (first satisfying seed in `0..=1023`, probed with a temporary
/// test through the public combat API).
const SEED: u64 = 0;
/// Bounded lethal action count (`K <= 12`).
const K: u64 = 6;
/// Trailing input-free ticks (`T >= 4`).
const T: u64 = 4;
/// Total ticks (`K + 1 + T`).
const TOTAL_TICKS: u64 = 11;

fn placement_hero() -> Ulid {
    Ulid::from_u128(14)
}

fn placement_goblin() -> Ulid {
    Ulid::from_u128(15)
}

fn ability() -> Ulid {
    Ulid::from_u128(19)
}

fn encounter() -> Ulid {
    Ulid::from_u128(21)
}

fn init_payload() -> serde_json::Value {
    json!({
        "combat": 1,
        "op": "init",
        "encounter": encounter().to_string(),
    })
}

fn attack_payload(actor: Ulid, target: Ulid) -> serde_json::Value {
    json!({
        "combat": 1,
        "op": "attack",
        "actor": actor.to_string(),
        "ability": ability().to_string(),
        "target": target.to_string(),
    })
}

/// The authored schedule: tick 0 init, ticks `1..=K` alternating goblin
/// first (initiative 9) and hero (initiative 10), stopping at the lethal
/// action. Fixed authored order; the winner is recorded, not chosen.
fn combat_inputs() -> Vec<ReplayInput> {
    let hero = placement_hero();
    let goblin = placement_goblin();
    vec![
        ReplayInput {
            tick: 0,
            payload: init_payload(),
        },
        ReplayInput {
            tick: 1,
            payload: attack_payload(goblin, hero),
        },
        ReplayInput {
            tick: 2,
            payload: attack_payload(hero, goblin),
        },
        ReplayInput {
            tick: 3,
            payload: attack_payload(goblin, hero),
        },
        ReplayInput {
            tick: 4,
            payload: attack_payload(hero, goblin),
        },
        ReplayInput {
            tick: 5,
            payload: attack_payload(goblin, hero),
        },
        ReplayInput {
            tick: 6,
            payload: attack_payload(hero, goblin),
        },
    ]
}

/// The in-memory replay the checked-in fixture must equal.
fn combat_replay() -> Replay {
    Replay::new(
        REPLAY_FORMAT_VERSION,
        SEED,
        COMBAT_CAMPAIGN_ID.to_string(),
        COMBAT_CAMPAIGN_VERSION.to_string(),
        COMBAT_ENGINE_VERSION.to_string(),
        TOTAL_TICKS,
        combat_inputs(),
    )
    .expect("combat replay must validate")
}

/// Independently reviewed oracle (seed 0, fixed `2d6` draws on
/// `combat.roll`): faces checked against inclusive roll-under
/// (`total <= might` succeeds; margin `= total - might`; bands
/// `[{MIN: success}, {1: failure}]`), damage against authored
/// `{success: 2, failure: 0}`. Hero might 8, goblin might 6; hero health
/// 10, goblin health 6. Both hit and miss appear; the hero wins.
fn expected_trace() -> Vec<TraceEntry> {
    let hero = placement_hero();
    let goblin = placement_goblin();
    let ab = ability();
    vec![
        TraceEntry {
            tick: 1,
            actor: goblin,
            target: hero,
            ability: ab,
            faces: vec![5, 6],
            total: 11,
            margin: 5,
            outcome: Outcome::Failure,
            damage: 0,
            target_died: false,
            target_health: 10,
            actor_pool: 0,
        },
        TraceEntry {
            tick: 2,
            actor: hero,
            target: goblin,
            ability: ab,
            faces: vec![3, 2],
            total: 5,
            margin: -3,
            outcome: Outcome::Success,
            damage: 2,
            target_died: false,
            target_health: 4,
            actor_pool: 0,
        },
        TraceEntry {
            tick: 3,
            actor: goblin,
            target: hero,
            ability: ab,
            faces: vec![2, 4],
            total: 6,
            margin: 0,
            outcome: Outcome::Success,
            damage: 2,
            target_died: false,
            target_health: 8,
            actor_pool: 0,
        },
        TraceEntry {
            tick: 4,
            actor: hero,
            target: goblin,
            ability: ab,
            faces: vec![2, 3],
            total: 5,
            margin: -3,
            outcome: Outcome::Success,
            damage: 2,
            target_died: false,
            target_health: 2,
            actor_pool: 0,
        },
        TraceEntry {
            tick: 5,
            actor: goblin,
            target: hero,
            ability: ab,
            faces: vec![2, 6],
            total: 8,
            margin: 2,
            outcome: Outcome::Failure,
            damage: 0,
            target_died: false,
            target_health: 8,
            actor_pool: 0,
        },
        TraceEntry {
            tick: 6,
            actor: hero,
            target: goblin,
            ability: ab,
            faces: vec![4, 2],
            total: 6,
            margin: -2,
            outcome: Outcome::Success,
            damage: 2,
            target_died: true,
            target_health: 0,
            actor_pool: 0,
        },
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
    let content = CombatContent::load().expect("combat_basic loads");
    assert_eq!(content.files.len(), 14);
    assert_eq!(content.loaded.index.len(), 11);
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

    let content = CombatContent::load().expect("reloads");
    let (apply, _) = combat_apply(content);
    // Play only the init input to pin publication.
    let init_only = Replay::new(
        REPLAY_FORMAT_VERSION,
        SEED,
        COMBAT_CAMPAIGN_ID.to_string(),
        COMBAT_CAMPAIGN_VERSION.to_string(),
        COMBAT_ENGINE_VERSION.to_string(),
        1,
        vec![combat_inputs()[0].clone()],
    )
    .expect("init replay validates");
    let hashes = play_replay(&init_only, apply).expect("init plays");
    assert_eq!(hashes.len(), 1);
    // Re-drive init directly to inspect the published world.
    let content = CombatContent::load().expect("reloads");
    let mut adapter = CombatAdapter::new(content);
    let mut world = World::new(SEED);
    let out = adapter
        .apply(&mut world, &init_payload())
        .expect("init applies");
    assert!(out.is_none());
    assert!(adapter.initialized);
    assert_eq!(adapter.entities.len(), 2);
    let ids: Vec<_> = world.combatants().iter().map(|(id, _)| id).collect();
    assert_eq!(ids.len(), 2);
    // Authored participant order: hero (14) then goblin (15).
    assert_eq!(
        world.combatants().get(ids[0]).expect("hero").placement(),
        placement_hero()
    );
    assert_eq!(
        world.combatants().get(ids[1]).expect("goblin").placement(),
        placement_goblin()
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
    let combat = world.combat().expect("encounter active");
    assert_eq!(combat.round, 0);
    // Lower initiative acts first: the goblin holds the turn.
    assert_eq!(combat.active, Some(ids[1]));
    assert_eq!(combat.definition.ability, ability());
}

#[test]
fn checked_in_replay_matches_authored_schedule() {
    let file = read_replay(&combat_replay_path()).expect("fixture parses");
    assert_eq!(file, combat_replay());
    assert_eq!(file.seed, SEED);
    assert_eq!(file.campaign_id, COMBAT_CAMPAIGN_ID);
    assert_eq!(file.total_ticks, TOTAL_TICKS);
    assert_eq!(file.inputs.len(), (K + 1) as usize);
    assert_eq!(file.inputs[0].tick, 0);
    for (step, input) in file.inputs.iter().enumerate().skip(1) {
        assert_eq!(input.tick, step as u64);
    }
}

#[test]
fn exact_action_trace_matches_reviewed_oracle() {
    let content = CombatContent::load().expect("content loads");
    let (apply, observations) = combat_apply(content);
    let replay = combat_replay();
    let hashes = play_replay(&replay, apply).expect("combat plays");
    assert_eq!(hashes.len(), TOTAL_TICKS as usize);
    let guard = observations.borrow();
    assert_eq!(guard.trace, expected_trace());
    assert_eq!(guard.snapshots.len(), K as usize);
    // Terminal identity: the goblin died, the hero survived with 8 health.
    let last = guard.trace.last().expect("lethal trace");
    assert!(last.target_died);
    assert_eq!(last.actor, placement_hero());
    assert_eq!(last.target, placement_goblin());
    assert_eq!(last.target_health, 0);
}

#[test]
fn identical_inputs_repeat_identically_and_reseed_diverges_on_prefix() {
    let first = play_replay(
        &combat_replay(),
        combat_apply(CombatContent::load().expect("loads")).0,
    )
    .expect("first plays");
    let second = play_replay(
        &combat_replay(),
        combat_apply(CombatContent::load().expect("loads")).0,
    )
    .expect("second plays");
    assert_eq!(first, second);
    assert_eq!(first.len(), TOTAL_TICKS as usize);

    // Reseeded prefix (init plus the first attack) is valid for both seeds
    // but must hash differently: the opening draws differ.
    let prefix_inputs = vec![combat_inputs()[0].clone(), combat_inputs()[1].clone()];
    let prefix_a = Replay::new(
        REPLAY_FORMAT_VERSION,
        SEED,
        COMBAT_CAMPAIGN_ID.to_string(),
        COMBAT_CAMPAIGN_VERSION.to_string(),
        COMBAT_ENGINE_VERSION.to_string(),
        2,
        prefix_inputs.clone(),
    )
    .expect("prefix validates");
    let prefix_b = Replay::new(
        REPLAY_FORMAT_VERSION,
        SEED + 1,
        COMBAT_CAMPAIGN_ID.to_string(),
        COMBAT_CAMPAIGN_VERSION.to_string(),
        COMBAT_ENGINE_VERSION.to_string(),
        2,
        prefix_inputs,
    )
    .expect("prefix validates");
    let hashes_a = play_replay(
        &prefix_a,
        combat_apply(CombatContent::load().expect("loads")).0,
    )
    .expect("prefix A plays");
    let hashes_b = play_replay(
        &prefix_b,
        combat_apply(CombatContent::load().expect("loads")).0,
    )
    .expect("prefix B plays");
    assert_eq!(hashes_a.len(), 2);
    assert_eq!(hashes_b.len(), 2);
    assert_ne!(hashes_a, hashes_b);
}

#[test]
fn delayed_lethal_attack_diverges_at_index_k() {
    let original = combat_replay();
    let original_hashes = play_replay(
        &original,
        combat_apply(CombatContent::load().expect("loads")).0,
    )
    .expect("original plays");
    // Delay the lethal attack from tick K to K+1, keeping earlier inputs
    // and total_ticks intact. The run stays valid; the old lethal tick now
    // holds no action.
    let mut delayed_inputs = combat_inputs();
    delayed_inputs.last_mut().expect("lethal").tick = K + 1;
    let delayed = Replay::new(
        REPLAY_FORMAT_VERSION,
        SEED,
        COMBAT_CAMPAIGN_ID.to_string(),
        COMBAT_CAMPAIGN_VERSION.to_string(),
        COMBAT_ENGINE_VERSION.to_string(),
        TOTAL_TICKS,
        delayed_inputs,
    )
    .expect("delayed validates");
    let delayed_hashes = play_replay(
        &delayed,
        combat_apply(CombatContent::load().expect("loads")).0,
    )
    .expect("delayed stays valid");
    assert_ne!(original_hashes, delayed_hashes);
    // First divergence is at index K (the old lethal tick), not K+1.
    let mut first = None;
    for (index, (a, b)) in original_hashes
        .iter()
        .zip(delayed_hashes.iter())
        .enumerate()
    {
        if a != b {
            first = Some(index);
            break;
        }
    }
    assert_eq!(first, Some(K as usize));

    // The same divergence surfaces truthfully through play_and_verify on
    // private copies: expected/actual sides match the independently
    // obtained sequences.
    let replay_path = combat_temp_file("delayed.replay");
    let golden_path = combat_temp_file("delayed.golden");
    remove_silently(&replay_path);
    remove_silently(&golden_path);
    write_replay(&replay_path, &delayed).expect("writes delayed replay");
    write_golden(&golden_path, &original_hashes).expect("writes original golden");
    let error = play_and_verify(
        &replay_path,
        &golden_path,
        combat_apply(CombatContent::load().expect("loads")).0,
    )
    .expect_err("delayed diverges");
    match error {
        ReplayError::Divergence(report) => {
            assert_eq!(report.tick(), Some(K as usize));
            match &report.mismatch {
                Mismatch::Diverged {
                    tick,
                    expected,
                    actual,
                } => {
                    assert_eq!(*tick, K as usize);
                    assert_eq!(*expected, original_hashes[K as usize]);
                    assert_eq!(*actual, delayed_hashes[K as usize]);
                }
                other => panic!("expected Diverged, got {other:?}"),
            }
        }
        other => panic!("expected Divergence, got {other:?}"),
    }
    remove_silently(&replay_path);
    remove_silently(&golden_path);
}

fn apply_fails(payload: serde_json::Value, tick: u64, index: usize) -> String {
    let replay = Replay::new(
        REPLAY_FORMAT_VERSION,
        SEED,
        COMBAT_CAMPAIGN_ID.to_string(),
        COMBAT_CAMPAIGN_VERSION.to_string(),
        COMBAT_ENGINE_VERSION.to_string(),
        tick + 1,
        vec![ReplayInput { tick, payload }],
    )
    .expect("probe replay validates");
    let content = CombatContent::load().expect("loads");
    let (apply, _) = combat_apply(content);
    match play_replay(&replay, apply).expect_err("must fail") {
        ReplayError::ApplyFailed {
            tick: got_tick,
            index: got_index,
            reason,
        } => {
            assert_eq!(got_tick, tick);
            assert_eq!(got_index, index);
            reason
        }
        other => panic!("expected ApplyFailed, got {other:?}"),
    }
}

#[test]
fn malformed_payloads_and_identities_fail_loudly() {
    // Non-object payload.
    assert_eq!(
        apply_fails(json!([1, 2]), 0, 0),
        "malformed combat payload: expected object"
    );
    // Missing combat version.
    assert_eq!(
        apply_fails(json!({"op": "init"}), 0, 0),
        "unsupported combat payload version"
    );
    // Wrong version (integer and non-integer).
    assert_eq!(
        apply_fails(json!({"combat": 2, "op": "init"}), 0, 0),
        "unsupported combat payload version"
    );
    assert_eq!(
        apply_fails(json!({"combat": "1", "op": "init"}), 0, 0),
        "unsupported combat payload version"
    );
    // Unknown op (including missing op).
    assert_eq!(
        apply_fails(json!({"combat": 1, "op": "retreat"}), 0, 0),
        "unknown combat op"
    );
    assert_eq!(apply_fails(json!({"combat": 1}), 0, 0), "unknown combat op");
    // Unknown field wins over missing required fields; lexically smallest.
    assert_eq!(
        apply_fails(json!({"combat": 1, "op": "init", "zz": 0, "aa": 0}), 0, 0),
        "malformed combat init: unknown field aa"
    );
    assert_eq!(
        apply_fails(
            json!({"combat": 1, "op": "attack", "actor": "x", "extra": 0}),
            1,
            0
        ),
        "malformed combat attack: unknown field extra"
    );
    // Missing required ULID fields.
    assert_eq!(
        apply_fails(json!({"combat": 1, "op": "init"}), 0, 0),
        "malformed combat init: encounter must be a ULID string"
    );
    assert_eq!(
        apply_fails(
            json!({"combat": 1, "op": "attack", "ability": ability().to_string(), "target": placement_hero().to_string()}),
            1,
            0
        ),
        "malformed combat attack: actor must be a ULID string"
    );
    // Non-string required field.
    assert_eq!(
        apply_fails(json!({"combat": 1, "op": "init", "encounter": 12}), 0, 0),
        "malformed combat init: encounter must be a ULID string"
    );
    // Unparseable ULID (no parser debug text).
    assert_eq!(
        apply_fails(
            json!({"combat": 1, "op": "init", "encounter": "not-a-ulid!!!!!!!!!!!!!!!!"}),
            0,
            0
        ),
        "malformed combat init: invalid encounter"
    );
    assert_eq!(
        apply_fails(
            json!({"combat": 1, "op": "attack", "actor": "bad", "ability": ability().to_string(), "target": placement_hero().to_string()}),
            1,
            0
        ),
        "malformed combat attack: invalid actor"
    );
    // Unknown encounter identity (canonical Display spelling).
    let strange = Ulid::from_u128(999).to_string();
    assert_eq!(
        apply_fails(
            json!({"combat": 1, "op": "init", "encounter": strange}),
            0,
            0
        ),
        format!("unknown encounter {strange}")
    );
    // Attack before init.
    assert_eq!(
        apply_fails(
            json!({"combat": 1, "op": "attack", "actor": placement_goblin().to_string(), "ability": ability().to_string(), "target": placement_hero().to_string()}),
            1,
            0
        ),
        "combat not initialized"
    );
    // Unknown placement identities (actor before target).
    let content = CombatContent::load().expect("loads");
    let mut adapter = CombatAdapter::new(content);
    let mut world = World::new(SEED);
    adapter
        .apply(&mut world, &init_payload())
        .expect("init applies");
    let unknown = Ulid::from_u128(999).to_string();
    let payload = json!({"combat": 1, "op": "attack", "actor": unknown, "ability": ability().to_string(), "target": placement_hero().to_string()});
    let before = snapshot(&world);
    let error = adapter
        .apply(&mut world, &payload)
        .expect_err("unknown actor");
    assert_eq!(error, format!("unknown combat identity actor: {unknown}"));
    assert_eq!(before, snapshot(&world));
    let payload = json!({"combat": 1, "op": "attack", "actor": placement_goblin().to_string(), "ability": ability().to_string(), "target": unknown});
    let error = adapter
        .apply(&mut world, &payload)
        .expect_err("unknown target");
    assert_eq!(error, format!("unknown combat identity target: {unknown}"));
    // Duplicate init names the failure with tick/index through playback.
    let replay = Replay::new(
        REPLAY_FORMAT_VERSION,
        SEED,
        COMBAT_CAMPAIGN_ID.to_string(),
        COMBAT_CAMPAIGN_VERSION.to_string(),
        COMBAT_ENGINE_VERSION.to_string(),
        2,
        vec![
            ReplayInput {
                tick: 0,
                payload: init_payload(),
            },
            ReplayInput {
                tick: 1,
                payload: init_payload(),
            },
        ],
    )
    .expect("duplicate replay validates");
    let (apply, _) = combat_apply(CombatContent::load().expect("loads"));
    match play_replay(&replay, apply).expect_err("duplicate must fail") {
        ReplayError::ApplyFailed {
            tick,
            index,
            reason,
        } => {
            assert_eq!(tick, 1);
            assert_eq!(index, 1);
            assert_eq!(reason, "duplicate combat init");
        }
        other => panic!("expected ApplyFailed, got {other:?}"),
    }
    // Unknown ability surfaces the sim Display verbatim.
    let replay = Replay::new(
        REPLAY_FORMAT_VERSION,
        SEED,
        COMBAT_CAMPAIGN_ID.to_string(),
        COMBAT_CAMPAIGN_VERSION.to_string(),
        COMBAT_ENGINE_VERSION.to_string(),
        2,
        vec![
            ReplayInput {
                tick: 0,
                payload: init_payload(),
            },
            ReplayInput {
                tick: 1,
                payload: json!({"combat": 1, "op": "attack", "actor": placement_goblin().to_string(), "ability": strange, "target": placement_hero().to_string()}),
            },
        ],
    )
    .expect("strange ability validates");
    let (apply, _) = combat_apply(CombatContent::load().expect("loads"));
    match play_replay(&replay, apply).expect_err("unknown ability must fail") {
        ReplayError::ApplyFailed {
            tick,
            index,
            reason,
        } => {
            assert_eq!(tick, 1);
            assert_eq!(index, 1);
            assert_eq!(reason, "attack failed: UnknownAbility at combat/ability");
        }
        other => panic!("expected ApplyFailed, got {other:?}"),
    }
}

#[test]
fn rejected_actions_preserve_full_state() {
    // Fresh-world pre-init attempt: identical bytes, no combat RNG stream.
    let content = CombatContent::load().expect("loads");
    let mut adapter = CombatAdapter::new(content);
    let mut fresh = World::new(SEED);
    let before = snapshot(&fresh);
    let payload = json!({"combat": 1, "op": "attack", "actor": placement_goblin().to_string(), "ability": ability().to_string(), "target": placement_hero().to_string()});
    assert_eq!(
        adapter.apply(&mut fresh, &payload).expect_err("pre-init"),
        "combat not initialized"
    );
    assert_eq!(before, snapshot(&fresh));
    assert!(
        !fresh.rng_mut().has_stream(crpg_sim::COMBAT_ROLL_STREAM),
        "rejected pre-init must not create the combat stream"
    );

    // Initialized world: out-of-turn and unknown-ability leave bytes
    // identical and create no combat stream on this path.
    let content = CombatContent::load().expect("loads");
    let mut adapter = CombatAdapter::new(content);
    let mut world = World::new(SEED);
    adapter
        .apply(&mut world, &init_payload())
        .expect("init applies");
    assert!(!world.rng_mut().has_stream(crpg_sim::COMBAT_ROLL_STREAM));
    // Out-of-turn: the hero acts while the goblin holds the turn.
    let early = json!({"combat": 1, "op": "attack", "actor": placement_hero().to_string(), "ability": ability().to_string(), "target": placement_goblin().to_string()});
    let before = snapshot(&world);
    let error = adapter.apply(&mut world, &early).expect_err("out of turn");
    assert_eq!(error, "attack failed: OutOfTurn at combat/active");
    assert_eq!(before, snapshot(&world));
    assert!(!world.rng_mut().has_stream(crpg_sim::COMBAT_ROLL_STREAM));
    // Unknown ability on the active turn.
    let strange = Ulid::from_u128(999).to_string();
    let weird = json!({"combat": 1, "op": "attack", "actor": placement_goblin().to_string(), "ability": strange, "target": placement_hero().to_string()});
    let before = snapshot(&world);
    let error = adapter
        .apply(&mut world, &weird)
        .expect_err("unknown ability");
    assert_eq!(error, "attack failed: UnknownAbility at combat/ability");
    assert_eq!(before, snapshot(&world));
    assert!(!world.rng_mut().has_stream(crpg_sim::COMBAT_ROLL_STREAM));
}

#[test]
fn terminal_state_trailing_ticks_and_exactly_once_death() {
    let content = CombatContent::load().expect("loads");
    let (apply, observations) = combat_apply(content);
    let replay = combat_replay();
    assert_eq!(replay.total_ticks, TOTAL_TICKS);
    assert_eq!(TOTAL_TICKS, K + 1 + T);
    let hashes = play_replay(&replay, apply).expect("combat plays");
    assert_eq!(hashes.len(), TOTAL_TICKS as usize);

    // Terminal facts come from the final callback snapshot (post-last
    // action, pre-tick): T016b's no-input-ticks-invent-nothing proves
    // no-input ticks change only the counter, so post-action combat state
    // equals terminal combat state; the golden pins every trailing hash.
    let guard = observations.borrow();
    let last_bytes = guard.snapshots.last().expect("lethal snapshot");
    let terminal: World = serde_json::from_slice(last_bytes).expect("snapshot parses");
    let combat = terminal.combat().expect("encounter state");
    assert_eq!(combat.active, None);
    assert!(combat.round >= 1);
    let mut dead = 0;
    let mut alive = 0;
    let mut dead_health = 0;
    for (_, combatant) in terminal.combatants().iter() {
        if combatant.dead() {
            dead += 1;
            assert_eq!(combatant.health(), 0);
        } else {
            alive += 1;
            assert!(combatant.health() > 0);
            assert!(!combatant.dead());
            dead_health = combatant.health();
        }
    }
    assert_eq!(dead, 1);
    assert_eq!(alive, 1);
    // The survivor is the hero with 8 health; the corpse is the goblin.
    assert_eq!(dead_health, 8);
    drop(guard);

    // Focused no-input continuation: the detached last world ticks exactly
    // T+1 times (the lethal input's own tick plus T trailing ticks). After
    // each step the hash matches replay indices K..=K+T, and the full
    // serialized state differs only in its tick field.
    let guard = observations.borrow();
    let mut continued: World = guard.last_world.clone().expect("last world");
    // last_world is post-last-action, pre-tick at tick K: its counter still
    // reads K, so the first tick() call produces replay index K.
    assert_eq!(continued.tick().get(), K);
    let terminal_bytes = serde_json::to_vec(&continued).expect("serializes");
    let mut terminal_value: serde_json::Value =
        serde_json::from_slice(&terminal_bytes).expect("parses");
    terminal_value
        .as_object_mut()
        .expect("an object")
        .remove("tick");
    for (step, expected) in hashes[(K as usize)..].iter().enumerate() {
        tick(&mut continued);
        assert_eq!(&state_hash(&continued), expected, "suffix step {step}");
        let mut value: serde_json::Value =
            serde_json::from_slice(&snapshot(&continued)).expect("parses");
        value.as_object_mut().expect("an object").remove("tick");
        assert_eq!(value, terminal_value, "suffix changes only the counter");
    }
    assert_eq!(continued.tick().get(), K + T + 1);
    drop(guard);

    // Death events: on a separate detached clone, drain and assert exactly
    // two Spawned plus one Died naming the dead combatant, sequences
    // 0/1/2, spawn tick 0, death tick K, next_seq 3 in the pre-drain
    // serialization. Never drain the authoritative replay world.
    let guard = observations.borrow();
    let mut probed: World = guard.last_world.clone().expect("last world");
    let pre: serde_json::Value = serde_json::from_slice(&snapshot(&probed)).expect("parses");
    assert_eq!(pre["events"]["next_seq"], json!(3));
    let envelopes = pre["events"]["envelopes"]
        .as_array()
        .expect("queue is envelopes plus next_seq, not an array at events");
    assert_eq!(envelopes.len(), 3);
    drop(guard);
    let drained = probed.events_mut().drain();
    assert_eq!(drained.len(), 3);
    assert_eq!(drained[0].seq, 0);
    assert_eq!(drained[1].seq, 1);
    assert_eq!(drained[2].seq, 2);
    assert_eq!(drained[0].tick.get(), 0);
    assert_eq!(drained[1].tick.get(), 0);
    assert_eq!(drained[2].tick.get(), K);
    match (
        &drained[0].payload,
        &drained[1].payload,
        &drained[2].payload,
    ) {
        (
            crpg_sim::SimEvent::Spawned { .. },
            crpg_sim::SimEvent::Spawned { .. },
            crpg_sim::SimEvent::Died { entity },
        ) => {
            // The dead entity is the goblin combatant (zero health).
            let guard = observations.borrow();
            let world: &World = guard.last_world.as_ref().expect("last world");
            let mut goblin_entity = None;
            for (id, combatant) in world.combatants().iter() {
                if combatant.placement() == placement_goblin() {
                    goblin_entity = Some(id);
                }
            }
            assert_eq!(Some(*entity), goblin_entity);
        }
        other => panic!("expected Spawned, Spawned, Died, got {other:?}"),
    }
    // Repeat after the terminal suffix on another clone: no new event.
    let guard = observations.borrow();
    let mut after: World = guard.last_world.clone().expect("last world");
    drop(guard);
    for _ in 0..(T + 1) {
        tick(&mut after);
    }
    assert_eq!(after.events().len(), 3);
}

#[test]
fn golden_negatives_fail_without_skips() {
    let hashes = play_replay(
        &combat_replay(),
        combat_apply(CombatContent::load().expect("loads")).0,
    )
    .expect("plays");
    // Missing golden is Io(NotFound), never a skip.
    let missing = combat_temp_file("missing.golden");
    remove_silently(&missing);
    match verify_golden(&missing, &hashes) {
        Err(crpg_testkit::HarnessError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
        other => panic!("expected Io(NotFound), got {other:?}"),
    }
    // Truncated golden is GoldenShort at the shorter length.
    let short_path = combat_temp_file("short.golden");
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
    // Malformed line is Malformed with the raw line.
    let bad_path = combat_temp_file("bad.golden");
    remove_silently(&bad_path);
    write_golden(&bad_path, &hashes).expect("writes golden");
    {
        let mut text = std::fs::read_to_string(&bad_path).expect("reads");
        let mut lines: Vec<&str> = text.lines().collect();
        let tamper = 2.min(lines.len().saturating_sub(1));
        // Replace the hash line at `tamper` (skipping headers) with text.
        let mut seen = 0;
        for line in lines.iter_mut() {
            if line.starts_with('#') {
                continue;
            }
            if seen == tamper {
                *line = "not-hex";
                break;
            }
            seen += 1;
        }
        text = lines.join("\n") + "\n";
        std::fs::write(&bad_path, text).expect("rewrites");
    }
    match verify_golden(&bad_path, &hashes) {
        Err(crpg_testkit::HarnessError::Mismatch(Mismatch::Malformed { .. })) => {}
        other => panic!("expected Malformed, got {other:?}"),
    }
    remove_silently(&bad_path);
    // Wrong hash is Diverged with truthful sides.
    let wrong_path = combat_temp_file("wrong.golden");
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
    // Extra valid hash line is RunShort at the run length.
    let long_path = combat_temp_file("long.golden");
    remove_silently(&long_path);
    let mut long = hashes.clone();
    long.push([7u8; 32]);
    write_golden(&long_path, &long).expect("writes long");
    match verify_golden(&long_path, &hashes) {
        Err(crpg_testkit::HarnessError::Mismatch(Mismatch::RunShort { tick, expected })) => {
            assert_eq!(tick, hashes.len());
            assert_eq!(expected, [7u8; 32]);
        }
        other => panic!("expected RunShort, got {other:?}"),
    }
    remove_silently(&long_path);

    // Tampered replay copy reports the truthful first divergence through
    // the replay layer (Diverged sides, never rewritten artifacts).
    let replay_path = combat_temp_file("tampered.replay");
    let golden_path = combat_temp_file("tampered.golden");
    remove_silently(&replay_path);
    remove_silently(&golden_path);
    let mut tampered_inputs = combat_inputs();
    // Flip the second attack's target: hero now strikes itself is rejected,
    // so instead delay it by moving it to the next tick (still valid, still
    // diverges at index 2).
    tampered_inputs[2].tick = 3;
    let tampered = Replay::new(
        REPLAY_FORMAT_VERSION,
        SEED,
        COMBAT_CAMPAIGN_ID.to_string(),
        COMBAT_CAMPAIGN_VERSION.to_string(),
        COMBAT_ENGINE_VERSION.to_string(),
        TOTAL_TICKS,
        tampered_inputs,
    )
    .expect("tampered validates");
    let tampered_hashes = play_replay(
        &tampered,
        combat_apply(CombatContent::load().expect("loads")).0,
    )
    .expect("tampered plays");
    write_replay(&replay_path, &tampered).expect("writes tampered");
    write_golden(&golden_path, &hashes).expect("writes golden");
    match play_and_verify(
        &replay_path,
        &golden_path,
        combat_apply(CombatContent::load().expect("loads")).0,
    ) {
        Err(ReplayError::Divergence(report)) => match &report.mismatch {
            Mismatch::Diverged {
                tick,
                expected,
                actual,
            } => {
                assert_eq!(*tick, 2);
                assert_eq!(*expected, hashes[2]);
                assert_eq!(*actual, tampered_hashes[2]);
            }
            other => panic!("expected Diverged, got {other:?}"),
        },
        other => panic!("expected Divergence, got {other:?}"),
    }
    remove_silently(&replay_path);
    remove_silently(&golden_path);
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
        &combat_replay_path(),
        &combat::combat_target_golden_path(),
        combat_apply(CombatContent::load().expect("loads")).0,
    )
    .expect("native target golden replay must match; missing baseline is an error");
    assert_eq!(hashes.len(), TOTAL_TICKS as usize);
}

#[test]
fn portable_repeatability_without_golden() {
    let replay = read_replay(&combat_replay_path()).expect("fixture parses");
    let first = play_replay(
        &replay,
        combat_apply(CombatContent::load().expect("loads")).0,
    )
    .expect("plays");
    let second = play_replay(
        &replay,
        combat_apply(CombatContent::load().expect("loads")).0,
    )
    .expect("plays");
    assert_eq!(first, second);
    assert_eq!(first.len(), replay.total_ticks as usize);
    assert_eq!(first.len(), TOTAL_TICKS as usize);
}
