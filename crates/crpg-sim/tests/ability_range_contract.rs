//! T149b two-keys suite: ability range persistence (X1 in sim), admission (X2)
//! and real-time retention (X3).
//!
//! Written independently from the contract text alone (`tasks/T149.md`,
//! Behaviour B-S1..B-S5 and test items R-1..R-9). Every expected value is a
//! contract literal or hand arithmetic over one; nothing here was read off
//! the implementation. Test names carry the item they prove.

#[allow(dead_code)]
mod support;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_sim::*;
use serde_json::{json, Value};
use support::creature::{add_outsider, enlist};
use support::rt::{
    bytes_of, edit_ability, elapsed, json_keys, make_lethal, pool_of, timeline_of, value_of,
};
use support::surface::{
    add_derived, always_success, attach_to_ability, binary, effect_doc, modifier, stat, stat_target,
};
use support::{uid, Fixture};

// ----------------------------------------------------------------- fixtures

const M: i32 = 65_536;
const TEXT: &str = "OutOfRange at combat/target";

fn p(x: i32, z: i32) -> [Fx16_16; 3] {
    [Fx16_16::from_int(x), Fx16_16::ZERO, Fx16_16::from_int(z)]
}
fn rp(x: i32, y: i32, z: i32) -> [Fx16_16; 3] {
    [
        Fx16_16::from_raw(x),
        Fx16_16::from_raw(y),
        Fx16_16::from_raw(z),
    ]
}
fn pl(n: u128) -> Ulid {
    Ulid::from_u128(n)
}
fn edit_ruleset(f: &mut Fixture, edit: impl FnOnce(&mut Value)) {
    let mut v = serde_json::to_value(&f.ruleset).unwrap();
    edit(&mut v);
    f.ruleset = serde_json::from_value(v).unwrap();
}
fn set_position(f: &mut Fixture, placement: u128, position: [Fx16_16; 3]) {
    f.placements
        .get_mut(&pl(placement))
        .unwrap()
        .0
        .transform
        .position = position;
}
fn set_pace(f: &mut Fixture, creature: u128, raw: i32) {
    f.creatures
        .get_mut(&pl(creature))
        .unwrap()
        .stats
        .insert("pace".to_owned(), Fx16_16::from_raw(raw));
}
/// T036's MOVE fixture: A at `p(0,0)`, B at `p(10,0)`, C (a non-participant)
/// at `p(0,10)`.
fn move_fixture() -> Fixture {
    let mut fx = Fixture::standard();
    edit_ruleset(&mut fx, |r| {
        r["stats"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "pace", "kind": "fixed"}));
        r["pools"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": uid(106), "max": 6, "refresh": {"type": "on_turn_start"}}));
        r["movement"] = json!({"speed": {"type": "stat", "value": "pace"},
            "turn": {"pool": uid(106), "step": {"type": "constant", "value": 98304}, "step_cost": 1}});
    });
    set_pace(&mut fx, 121, M);
    set_pace(&mut fx, 122, M / 2);
    set_position(&mut fx, 112, rp(655_360, 0, 0));
    add_outsider(&mut fx, 113, 123);
    set_position(&mut fx, 113, rp(0, 0, 655_360));
    for name in ["alpha", "beta", "gamma"] {
        fx.creatures
            .get_mut(&pl(123))
            .unwrap()
            .stats
            .insert(name.to_owned(), Fx16_16::from_int(5));
    }
    fx.creatures
        .get_mut(&pl(123))
        .unwrap()
        .stats
        .insert("health".to_owned(), Fx16_16::from_int(4));
    set_pace(&mut fx, 123, M);
    fx
}
fn move_rt() -> Fixture {
    let mut fx = move_fixture();
    fx.ruleset.round_ticks = Some(4);
    fx
}
fn set_range(fx: &mut Fixture, raw: i32) {
    edit_ability(fx, |a| a.range = Some(Fx16_16::from_raw(raw)));
}
fn ranged(mut fx: Fixture, raw: i32) -> Fixture {
    set_range(&mut fx, raw);
    fx
}
fn use_ability(actor: EntityId, target: EntityId) -> CombatAction {
    CombatAction::UseAbility {
        actor,
        ability: pl(103),
        target,
    }
}
/// A turn-based world (B holds the first turn) and the ids `(A, B)`.
fn tb(fx: &Fixture) -> (World, EntityId, EntityId) {
    let (w, ids) = fx.start(7);
    (w, ids[0], ids[1])
}
fn rt(fx: &Fixture) -> (World, EntityId, EntityId) {
    let (w, ids) = fx.start_rt(7, EncounterMode::RealTime);
    (w, ids[0], ids[1])
}
fn put(w: &mut World, e: EntityId, position: [Fx16_16; 3]) {
    w.place(e, position).expect("the entity is placed");
}
fn order_of(w: &World, e: EntityId) -> Option<StandingOrder> {
    w.combatants().get(e).expect("a combatant").order()
}
fn standing(target: EntityId, repeat: bool) -> StandingOrder {
    StandingOrder {
        ability: pl(103),
        target,
        repeat,
    }
}

/// `validate_action` and `perform_action` on a clone both give `error`, the
/// clone's bytes and the original's stay unchanged.
fn rejects(w: &World, action: &CombatAction, error: CombatError) {
    let before = bytes_of(w);
    assert_eq!(validate_action(w, action), Err(error.clone()));
    let mut copy = w.clone();
    assert_eq!(perform_action(&mut copy, action), Err(error));
    assert_eq!(bytes_of(&copy), before, "a rejection changes nothing");
    assert_eq!(bytes_of(w), before, "validation changes nothing");
}
fn rejects_range(w: &World, action: &CombatAction) {
    rejects(w, action, CombatError::OutOfRange);
}
fn admits(w: &World, action: &CombatAction) {
    assert_eq!(validate_action(w, action), Ok(()));
    let mut copy = w.clone();
    assert!(perform_action(&mut copy, action).is_ok());
}
fn events_after(h: &HistoryWorld, after: u64) -> Vec<HistoryEvent> {
    h.read_after(after, 256)
        .expect("the journal reads")
        .into_iter()
        .map(|envelope| envelope.payload)
        .collect()
}
fn resolved(events: &[HistoryEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, HistoryEvent::ActionResolved { .. }))
        .count()
}
fn ended(events: &[HistoryEvent]) -> Vec<EntityId> {
    events
        .iter()
        .filter_map(|e| match e {
            HistoryEvent::OrderEnded { actor } => Some(*actor),
            _ => None,
        })
        .collect()
}
fn remove_mover(save: &mut Value, entity: EntityId) {
    let key = serde_json::to_value(entity).expect("the id serializes");
    save["movers"]
        .as_array_mut()
        .expect("a mover list")
        .retain(|entry| entry[0] != key);
}
fn reload(value: Value) -> World {
    serde_json::from_value(value).expect("the save loads")
}

// ---------------------------------------------------- R-1 / B-S2 the formula

#[test]
fn r1_within_range_formula() {
    let big = 1 << 29;
    let cases: Vec<([Fx16_16; 3], [Fx16_16; 3], i32, bool)> = vec![
        (p(0, 0), p(10, 0), 655_360, true),
        (p(0, 0), p(10, 0), 655_359, false),
        (p(0, 0), p(3, 4), 327_680, true),
        (p(0, 0), p(3, 4), 327_679, false),
        // The root rounds up: sqrt(2) raw is 1.41.. so 2.
        (rp(0, 0, 0), rp(1, 0, 1), 2, true),
        (rp(0, 0, 0), rp(1, 0, 1), 1, false),
        (p(0, 0), rp(M, 0, M), 92_682, true),
        (p(0, 0), rp(M, 0, M), 92_681, false),
        // Height never counts.
        (rp(0, 0, 0), rp(0, 327_680, 0), 0, true),
        (rp(0, 0, 0), rp(0, 327_680, 0), -1, false),
        (rp(0, 0, 0), rp(0, big, 0), 0, true),
        // `y` must still be inside the coordinate box.
        (rp(0, 0, 0), rp(0, big + 1, 0), i32::MAX, false),
        // The box diagonal.
        (rp(-big, 0, -big), rp(big, 0, big), 1_518_500_250, true),
        (rp(-big, 0, -big), rp(big, 0, big), 1_518_500_249, false),
        (rp(-big, 0, -big), rp(big, 0, big), i32::MAX, true),
        // One raw past the coordinate box is never within range.
        (p(0, 0), rp(big + 1, 0, 0), i32::MAX, false),
        (rp(big + 1, 0, 0), p(0, 0), i32::MAX, false),
        (p(0, 0), rp(0, 0, -big - 1), i32::MAX, false),
        (p(0, 0), rp(i32::MIN, 0, 0), i32::MAX, false),
        // The box edge itself is inside.
        (p(0, 0), rp(big, 0, 0), i32::MAX, true),
        (p(0, 0), rp(big, 0, 0), big, true),
        (p(0, 0), rp(big, 0, 0), big - 1, false),
        // A point to itself: zero is in, one is in, a negative range is out.
        (p(3, 3), p(3, 3), 0, true),
        (p(3, 3), p(3, 3), 1, true),
        (p(3, 3), p(3, 3), -1, false),
        (p(3, 3), p(3, 3), i32::MIN, false),
        (rp(0, 0, 0), rp(0, 0, 0), i32::MIN, false),
        // Raw range 1 against one raw of distance and two.
        (rp(0, 0, 0), rp(1, 0, 0), 1, true),
        (rp(0, 0, 0), rp(1, 0, 0), 0, false),
        (rp(0, 0, 0), rp(2, 0, 0), 1, false),
        (rp(0, 0, 0), rp(2, 0, 0), 2, true),
    ];
    for (from, to, range, expected) in cases {
        let r = Fx16_16::from_raw(range);
        assert_eq!(
            within_range(Measure::Euclidean, r, from, to),
            expected,
            "{from:?} -> {to:?} range {range}"
        );
        if expected || range >= 0 {
            // Distance is symmetric.
            assert_eq!(within_range(Measure::Euclidean, r, to, from), expected);
        }
    }
}

// ----------------------------------------------- R-2 / B-S3 the boundary

#[test]
fn r2_admission_at_the_boundary() {
    let action = |a, b| use_ability(b, a);
    // B at p(10,0), A at p(0,0): 10 m.
    let (w, a, b) = tb(&ranged(move_fixture(), 655_360));
    assert_eq!(w.combat().unwrap().active, Some(b));
    admits(&w, &action(a, b));
    let (w, a, b) = tb(&ranged(move_fixture(), 655_359));
    rejects_range(&w, &action(a, b));

    // 3-4-5 style: dx 7 m, dz 4 m, sqrt(65) m = 528368.1.. raw, rounded up.
    let (mut w, a, b) = tb(&ranged(move_fixture(), 528_369));
    put(&mut w, a, p(3, 4));
    admits(&w, &action(a, b));
    let (mut w, a, b) = tb(&ranged(move_fixture(), 528_368));
    put(&mut w, a, p(3, 4));
    rejects_range(&w, &action(a, b));

    // Height is ignored.
    let (mut w, a, b) = tb(&ranged(move_fixture(), 655_360));
    put(&mut w, a, rp(0, 327_680, 0));
    admits(&w, &action(a, b));
    let (mut w, a, b) = tb(&ranged(move_fixture(), 655_359));
    put(&mut w, a, rp(0, 327_680, 0));
    rejects_range(&w, &action(a, b));

    // Coincident targets: zero admits, a negative range admits nothing.
    let (mut w, a, b) = tb(&ranged(move_fixture(), 0));
    put(&mut w, a, p(10, 0));
    admits(&w, &action(a, b));
    let (mut w, a, b) = tb(&ranged(move_fixture(), -1));
    put(&mut w, a, p(10, 0));
    rejects_range(&w, &action(a, b));
    let (w, a, b) = tb(&ranged(move_fixture(), 0));
    rejects_range(&w, &action(a, b));

    // One raw of distance: range 1 is in, range 0 is out.
    let (mut w, a, b) = tb(&ranged(move_fixture(), 1));
    put(&mut w, a, rp(655_359, 0, 0));
    admits(&w, &action(a, b));
    let (mut w, a, b) = tb(&ranged(move_fixture(), 0));
    put(&mut w, a, rp(655_359, 0, 0));
    rejects_range(&w, &action(a, b));
    let (w, a, b) = tb(&ranged(move_fixture(), 1));
    rejects_range(&w, &action(a, b));

    // The largest range is unlimited in effect, within the box.
    let (mut w, a, b) = tb(&ranged(move_fixture(), i32::MAX));
    put(&mut w, a, rp(-(1 << 29), 0, -(1 << 29)));
    admits(&w, &action(a, b));

    // No range: Ok at any distance, even the box edge.
    let (mut w, a, b) = tb(&move_fixture());
    admits(&w, &action(a, b));
    put(&mut w, a, rp(1 << 29, 0, 0));
    admits(&w, &action(a, b));
}

#[test]
fn r2_the_range_applies_whatever_requires_target_says() {
    for requires_target in [true, false] {
        let mut fx = ranged(move_fixture(), 655_359);
        edit_ability(&mut fx, |a| a.requires_target = requires_target);
        let (w, a, b) = tb(&fx);
        rejects_range(&w, &use_ability(b, a));
        let mut fx = ranged(move_fixture(), 655_360);
        edit_ability(&mut fx, |a| a.requires_target = requires_target);
        let (w, a, b) = tb(&fx);
        admits(&w, &use_ability(b, a));
    }
}

#[test]
fn r2_a_self_target_measures_zero() {
    let mut fx = ranged(move_fixture(), 0);
    edit_ability(&mut fx, |a| a.allow_self_target = true);
    let (w, _, b) = tb(&fx);
    admits(&w, &use_ability(b, b));
    let mut fx = ranged(move_fixture(), -1);
    edit_ability(&mut fx, |a| a.allow_self_target = true);
    let (w, _, b) = tb(&fx);
    rejects_range(&w, &use_ability(b, b));
}

#[test]
fn r2_real_time_admission_at_the_boundary() {
    let (mut w, a, b) = rt(&ranged(move_rt(), 655_360));
    admits(&w, &use_ability(a, b));
    put(&mut w, b, rp(655_361, 0, 0));
    rejects_range(&w, &use_ability(a, b));
    put(&mut w, b, rp(655_360, 0, 0));
    admits(&w, &use_ability(a, b));
    // Reversed roles: the distance is the same from B's side.
    put(&mut w, a, rp(-1, 0, 0));
    rejects_range(&w, &use_ability(b, a));
    // An accepted order is stored; a rejected one is not.
    let (mut w, a, b) = rt(&ranged(move_rt(), 655_360));
    assert_eq!(perform_action(&mut w, &use_ability(a, b)), Ok(None));
    assert_eq!(order_of(&w, a), Some(standing(b, false)));
}

// ------------------------------------------------- R-3 / B-S3 missing mover

#[test]
fn r3_missing_mover() {
    // No `movement`, so no movers: a range can never be met.
    let (w, ids) = ranged(Fixture::standard(), i32::MAX).start(7);
    assert!(w.movers().get(ids[0]).is_none());
    rejects_range(&w, &use_ability(ids[1], ids[0]));
    let (w, ids) = Fixture::standard().start(7);
    admits(&w, &use_ability(ids[1], ids[0]));

    // A save missing one side's mover.
    let (w, a, b) = tb(&ranged(move_fixture(), 655_360));
    admits(&w, &use_ability(b, a));
    for gone in [a, b] {
        let mut save = serde_json::to_value(&w).unwrap();
        remove_mover(&mut save, gone);
        let loaded = reload(save);
        assert!(loaded.movers().get(gone).is_none());
        rejects_range(&loaded, &use_ability(b, a));
    }
    // Without a range the same saves still admit.
    for gone in [a, b] {
        let (w, a, b) = tb(&move_fixture());
        let mut save = serde_json::to_value(&w).unwrap();
        remove_mover(&mut save, gone);
        admits(&reload(save), &use_ability(b, a));
    }
}

// ------------------------------------------------ R-4 / B-S3 the last stage

#[test]
fn r4_out_of_turn_beats_the_range() {
    let (w, a, b) = tb(&ranged(move_fixture(), 1));
    assert_eq!(
        validate_action(&w, &use_ability(a, b)),
        Err(CombatError::OutOfTurn {
            actor: a,
            active: Some(b)
        })
    );
    rejects(
        &w,
        &use_ability(a, b),
        CombatError::OutOfTurn {
            actor: a,
            active: Some(b),
        },
    );
    // Control: the active actor with the same range hits the range stage.
    rejects_range(&w, &use_ability(b, a));
}

#[test]
fn r4_self_target_beats_the_range() {
    let (w, _, b) = tb(&ranged(move_fixture(), -1));
    rejects(&w, &use_ability(b, b), CombatError::SelfTarget);
    let mut fx = ranged(move_fixture(), -1);
    edit_ability(&mut fx, |a| a.allow_self_target = true);
    let (w, _, b) = tb(&fx);
    rejects_range(&w, &use_ability(b, b));
}

#[test]
fn r4_insufficient_action_beats_the_range() {
    let mut fx = ranged(move_fixture(), 655_360);
    edit_ability(&mut fx, |a| a.ends_turn = false);
    let (mut w, a, b) = tb(&fx);
    // Control: affordable and out of range reports the range.
    let mut far = w.clone();
    put(&mut far, a, p(-1, 0));
    rejects_range(&far, &use_ability(b, a));
    assert!(perform_action(&mut w, &use_ability(b, a)).is_ok());
    assert_eq!(w.combat().unwrap().active, Some(b));
    put(&mut w, a, p(-1, 0));
    rejects(
        &w,
        &use_ability(b, a),
        CombatError::InsufficientAction {
            pool: pl(105),
            cost: 1,
            current: 0,
        },
    );
}

#[test]
fn r4_dead_target_beats_the_range() {
    let mut fx = ranged(move_fixture(), 655_360);
    always_success(&mut fx);
    make_lethal(&mut fx, 99);
    let (mut w, a, b) = tb(&fx);
    assert!(perform_action(&mut w, &use_ability(b, a)).is_ok());
    assert!(w.combatants().get(a).unwrap().dead());
    put(&mut w, a, p(-1, 0));
    rejects(
        &w,
        &use_ability(b, a),
        CombatError::DeadTarget { target: a },
    );
}

#[test]
fn r4_stat_evaluation_beats_the_range() {
    // The zeroing scenario: B's hit sets A's `beta` to zero for three rounds,
    // so A's `ratio` divides by zero when B next targets A.
    let mut fx = Fixture::standard();
    add_derived(
        &mut fx,
        "ratio",
        binary("divide", stat("alpha"), stat("beta")),
    );
    always_success(&mut fx);
    edit_ability(&mut fx, |ability| {
        ability.defense = crpg_data::DefenseWire::TargetStat {
            stat: "ratio".to_owned(),
        };
        ability.allow_self_target = true;
        ability.ends_turn = false;
    });
    attach_to_ability(
        &mut fx,
        effect_doc(
            141,
            "target",
            "probe-type",
            "stack_all",
            vec![modifier(142, stat_target("beta"), "set", 0)],
            3,
        ),
    );
    let (mut w, ids) = fx.start(17);
    let (a, b) = (ids[0], ids[1]);
    assert!(perform_action(&mut w, &use_ability(b, a)).is_ok());
    perform_action(&mut w, &CombatAction::EndTurn { actor: b }).unwrap();
    perform_action(&mut w, &CombatAction::EndTurn { actor: a }).unwrap();
    assert_eq!(w.combat().unwrap().active, Some(b));
    let evaluation = CombatError::StatEvaluation {
        stat: "ratio".to_owned(),
    };
    assert_eq!(
        validate_action(&w, &use_ability(b, a)),
        Err(evaluation.clone())
    );
    // Control: before the range, B on B is legal.
    admits(&w, &use_ability(b, b));

    let mut save = serde_json::to_value(&w).unwrap();
    save["combat"]["definition"]["abilities"][0]["range"] = json!(M);
    let ranged_world = reload(save);
    rejects(&ranged_world, &use_ability(b, a), evaluation);
    // No movers in the standard fixture, so the range stage fails B on B.
    rejects_range(&ranged_world, &use_ability(b, b));
}

// ---------------------------------------------- R-5 / B-S4 legal_actions

#[test]
fn r5_legal_actions_omit_out_of_range_targets() {
    let end = |b| CombatAction::EndTurn { actor: b };
    let (w, a, b) = tb(&ranged(move_fixture(), 655_360));
    assert_eq!(legal_actions(&w, b), Ok(vec![use_ability(b, a), end(b)]));
    let (w, a, b) = tb(&ranged(move_fixture(), 655_359));
    assert_eq!(legal_actions(&w, b), Ok(vec![end(b)]));
    let _ = a;

    let mut fx = ranged(move_fixture(), 655_359);
    edit_ability(&mut fx, |x| x.allow_self_target = true);
    let (w, _, b) = tb(&fx);
    assert_eq!(legal_actions(&w, b), Ok(vec![use_ability(b, b), end(b)]));

    // Control: a range that reaches both lists both, A (the lower id) first.
    let mut fx = ranged(move_fixture(), 655_360);
    edit_ability(&mut fx, |x| x.allow_self_target = true);
    let (w, a, b) = tb(&fx);
    assert_eq!(
        legal_actions(&w, b),
        Ok(vec![use_ability(b, a), use_ability(b, b), end(b)])
    );

    // No range: unchanged.
    let (w, a, b) = tb(&move_fixture());
    assert_eq!(legal_actions(&w, b), Ok(vec![use_ability(b, a), end(b)]));

    // Every listed candidate validates and every omitted one does not.
    let (mut w, a, b) = tb(&ranged(move_fixture(), 655_360));
    put(&mut w, a, p(-1, 0));
    assert_eq!(legal_actions(&w, b), Ok(vec![end(b)]));
    put(&mut w, a, p(0, 0));
    assert_eq!(legal_actions(&w, b), Ok(vec![use_ability(b, a), end(b)]));
}

#[test]
fn r5_real_time_legal_actions() {
    let stop = |a| CombatAction::Stop { actor: a };
    let (mut w, a, b) = rt(&ranged(move_rt(), 655_360));
    put(&mut w, b, rp(655_361, 0, 0));
    assert_eq!(legal_actions(&w, a), Ok(vec![stop(a)]));
    put(&mut w, b, rp(655_360, 0, 0));
    assert_eq!(legal_actions(&w, a), Ok(vec![use_ability(a, b), stop(a)]));
}

// ------------------------------------------ R-6 / B-S4 rejections are inert

#[test]
fn r6_rejections_change_nothing() {
    // Turn-based.
    let (mut w, a, b) = tb(&ranged(move_fixture(), 655_359));
    let before = bytes_of(&w);
    let hash = state_hash(&w);
    assert_eq!(
        perform_action(&mut w, &use_ability(b, a)),
        Err(CombatError::OutOfRange)
    );
    assert_eq!(bytes_of(&w), before);
    assert_eq!(state_hash(&w), hash);
    assert_eq!(pool_of(&w, b), 1);

    // Real-time: no order is stored.
    let (mut w, a, b) = rt(&ranged(move_rt(), 655_359));
    let before = bytes_of(&w);
    assert_eq!(
        perform_action(&mut w, &use_ability(a, b)),
        Err(CombatError::OutOfRange)
    );
    assert_eq!(order_of(&w, a), None);
    assert_eq!(bytes_of(&w), before);
    assert_eq!(pool_of(&w, a), 1);
}

#[test]
fn r6_history_rejections_change_nothing() {
    for (fx, mode, real_time) in [
        (
            ranged(move_fixture(), 655_359),
            EncounterMode::TurnBased,
            false,
        ),
        (ranged(move_rt(), 655_359), EncounterMode::RealTime, true),
    ] {
        let (mut h, ids) = fx.start_history(7, mode).expect("the history starts");
        let (a, b) = (ids[0], ids[1]);
        let action = if real_time {
            use_ability(a, b)
        } else {
            use_ability(b, a)
        };
        let (seq, len, bytes) = (h.last_sequence(), h.pending_len(), bytes_of(h.world()));
        let error = h.perform_action(&action).expect_err("out of range");
        assert_eq!(error, HistoryError::Combat(CombatError::OutOfRange));
        assert_eq!(error.to_string(), TEXT);
        assert_eq!(h.last_sequence(), seq);
        assert_eq!(h.pending_len(), len);
        assert_eq!(bytes_of(h.world()), bytes);
        assert_eq!(order_of(h.world(), a), None);
    }
}

#[test]
fn r6_the_error_texts() {
    assert_eq!(CombatError::OutOfRange.to_string(), TEXT);
    assert_eq!(
        HistoryError::Combat(CombatError::OutOfRange).to_string(),
        TEXT
    );
}

// -------------------------------------- R-7 / B-S5 an out-of-range order waits

#[test]
fn r7_out_of_range_order_stays_due() {
    let fx = ranged(move_rt(), 655_360);
    let (mut h, ids) = fx.start_history(7, EncounterMode::RealTime).unwrap();
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(timeline_of(h.world()), vec![(0, a), (0, b)]);
    assert_eq!(h.perform_action(&use_ability(a, b)), Ok(None));
    assert_eq!(order_of(h.world(), a), Some(standing(b, false)));
    let cursor = h.last_sequence();
    h.place(b, rp(655_361, 0, 0)).unwrap();
    for ticks in 1..=2u64 {
        h.tick().unwrap();
        assert_eq!(elapsed(h.world()), ticks);
        assert_eq!(order_of(h.world(), a), Some(standing(b, false)));
        assert_eq!(timeline_of(h.world()), vec![(0, a), (0, b)]);
        assert_eq!(pool_of(h.world(), a), 1);
        assert_eq!(pool_of(h.world(), b), 1);
        let events = events_after(&h, cursor);
        assert!(ended(&events).is_empty(), "{events:?}");
        assert_eq!(resolved(&events), 0, "{events:?}");
    }
    h.place(b, p(10, 0)).unwrap();
    let cursor = h.last_sequence();
    h.tick().unwrap();
    assert_eq!(elapsed(h.world()), 3);
    let events = events_after(&h, cursor);
    assert_eq!(resolved(&events), 1, "{events:?}");
    assert!(events.iter().any(|e| matches!(
        e,
        HistoryEvent::ActionResolved { actor, target, ability, .. }
            if *actor == a && *target == b && *ability == pl(103)
    )));
    assert!(ended(&events).is_empty());
    assert_eq!(order_of(h.world(), a), None);
    assert_eq!(pool_of(h.world(), a), 0);
    assert_eq!(timeline_of(h.world()), vec![(0, b), (7, a)]);
}

#[test]
fn r7_the_order_waits_across_a_round_boundary() {
    let fx = ranged(move_rt(), 655_360);
    let (mut h, ids) = fx.start_history(7, EncounterMode::RealTime).unwrap();
    let (a, b) = (ids[0], ids[1]);
    h.perform_action(&use_ability(a, b)).unwrap();
    h.place(b, rp(655_361, 0, 0)).unwrap();
    for _ in 0..6 {
        h.tick().unwrap();
        assert_eq!(order_of(h.world(), a), Some(standing(b, false)));
        assert_eq!(timeline_of(h.world()), vec![(0, a), (0, b)]);
        assert_eq!(pool_of(h.world(), a), 1);
    }
    assert_eq!(elapsed(h.world()), 6);
    assert_eq!(resolved(&events_after(&h, 0)), 0);
    assert!(ended(&events_after(&h, 0)).is_empty());
    h.place(b, rp(655_360, 0, 0)).unwrap();
    let cursor = h.last_sequence();
    h.tick().unwrap();
    assert_eq!(elapsed(h.world()), 7);
    assert_eq!(resolved(&events_after(&h, cursor)), 1);
    assert_eq!(order_of(h.world(), a), None);
    assert_eq!(timeline_of(h.world()), vec![(0, b), (11, a)]);
}

#[test]
fn r7_a_bare_world_tick_retains_the_order_too() {
    let (mut w, a, b) = rt(&ranged(move_rt(), 655_360));
    perform_action(&mut w, &use_ability(a, b)).unwrap();
    put(&mut w, b, rp(655_361, 0, 0));
    let queue = value_of(&w)["events"].clone();
    for _ in 0..3 {
        tick(&mut w);
        assert_eq!(order_of(&w, a), Some(standing(b, false)));
        assert_eq!(pool_of(&w, a), 1);
        assert_eq!(value_of(&w)["events"], queue, "nothing is enqueued");
    }
    put(&mut w, b, p(10, 0));
    tick(&mut w);
    assert_eq!(order_of(&w, a), None);
    assert_eq!(pool_of(&w, a), 0);
}

// ----------------------------- R-8 / B-S5 other failures still end the order

#[test]
fn r8_other_failures_still_end_the_order() {
    let mut fx = ranged(move_rt(), 655_360);
    edit_ability(&mut fx, |x| {
        x.repeats_in_real_time = Some(true);
        x.recovery_ticks = Some(1);
    });
    let (mut h, ids) = fx.start_history(7, EncounterMode::RealTime).unwrap();
    let (a, b) = (ids[0], ids[1]);
    h.perform_action(&use_ability(a, b)).unwrap();
    let cursor = h.last_sequence();
    h.tick().unwrap();
    assert_eq!(elapsed(h.world()), 1);
    assert_eq!(resolved(&events_after(&h, cursor)), 1);
    assert_eq!(pool_of(h.world(), a), 0);
    assert_eq!(order_of(h.world(), a), Some(standing(b, true)));
    assert_eq!(timeline_of(h.world()), vec![(0, b), (2, a)]);

    h.place(b, rp(655_361, 0, 0)).unwrap();
    let cursor = h.last_sequence();
    h.tick().unwrap();
    assert_eq!(elapsed(h.world()), 2);
    // Affordability fails before the range: the order ends.
    assert_eq!(order_of(h.world(), a), None);
    let events = events_after(&h, cursor);
    assert_eq!(ended(&events), vec![a], "{events:?}");
    assert_eq!(resolved(&events), 0);
}

#[test]
fn r8_a_dead_target_ends_the_order_even_out_of_range() {
    let mut fx = ranged(move_rt(), 655_360);
    enlist(&mut fx, 113, 8);
    always_success(&mut fx);
    make_lethal(&mut fx, 99);
    edit_ability(&mut fx, |x| {
        x.repeats_in_real_time = Some(true);
        x.recovery_ticks = Some(1);
    });
    let (mut h, ids) = fx.start_history(7, EncounterMode::RealTime).unwrap();
    let (a, b) = (ids[0], ids[1]);
    h.perform_action(&use_ability(a, b)).unwrap();
    h.tick().unwrap();
    assert!(h.world().combatants().get(b).unwrap().dead());
    assert_eq!(order_of(h.world(), a), Some(standing(b, true)));
    h.place(b, rp(655_361, 0, 0)).unwrap();
    // Make the actor able to pay again: only the dead target should fail.
    let cursor = h.last_sequence();
    h.tick().unwrap();
    let events = events_after(&h, cursor);
    assert_eq!(order_of(h.world(), a), None);
    assert_eq!(ended(&events), vec![a], "{events:?}");
}

// ----------------------------------------------- R-9 / B-S1 persistence

#[test]
fn r9_persistence() {
    let fx = ranged(Fixture::standard(), M);
    let (w, _) = fx.start(7);
    let save = serde_json::to_value(&w).unwrap();
    let definition = &save["combat"]["definition"];
    assert!(definition.get("abilities").is_some(), "non-legacy form");
    assert_eq!(definition["abilities"][0]["range"], json!(65_536));
    assert_eq!(
        w.combat().unwrap().definition.abilities[0].range,
        Some(Fx16_16::from_raw(65_536))
    );
    let loaded = reload(save.clone());
    assert_eq!(
        loaded.combat().unwrap().definition.abilities[0].range,
        Some(Fx16_16::from_raw(65_536))
    );
    assert_eq!(bytes_of(&loaded), bytes_of(&w));
    assert_eq!(state_hash(&loaded), state_hash(&w));
    assert_eq!(serde_json::to_value(&loaded).unwrap(), save);

    // Without a range: the legacy 15-field form and no `range` key anywhere.
    let (plain, _) = Fixture::standard().start(7);
    let save = serde_json::to_value(&plain).unwrap();
    assert!(save["combat"]["definition"].get("abilities").is_none());
    assert!(!json_keys(&save).contains("range"));
    assert_eq!(plain.combat().unwrap().definition.abilities[0].range, None);
    let loaded = reload(save);
    assert_eq!(bytes_of(&loaded), bytes_of(&plain));
    assert_eq!(loaded.combat().unwrap().definition.abilities[0].range, None);
    assert_ne!(state_hash(&plain), state_hash(&w));
}

#[test]
fn r9_a_missing_range_key_reads_as_none() {
    let (w, _) = ranged(Fixture::standard(), M).start(7);
    let mut save = serde_json::to_value(&w).unwrap();
    save["combat"]["definition"]["abilities"][0]
        .as_object_mut()
        .unwrap()
        .remove("range");
    let loaded = reload(save);
    assert_eq!(loaded.combat().unwrap().definition.abilities[0].range, None);
    assert!(!json_keys(&serde_json::to_value(&loaded).unwrap()).contains("range"));
}

#[test]
fn r9_every_raw_value_survives_a_round_trip() {
    for raw in [1, M, 655_360, i32::MAX, 0, -1, i32::MIN] {
        let (w, _) = ranged(move_fixture(), raw).start(7);
        let save = serde_json::to_value(&w).unwrap();
        assert_eq!(
            save["combat"]["definition"]["abilities"][0]["range"],
            json!(raw)
        );
        let loaded = reload(save);
        assert_eq!(
            loaded.combat().unwrap().definition.abilities[0].range,
            Some(Fx16_16::from_raw(raw))
        );
        assert_eq!(bytes_of(&loaded), bytes_of(&w));
    }
}

// ------------------------------------------- review survivors (fix round)

#[test]
fn r7_a_retained_order_does_not_stop_the_rest_of_the_pass() {
    // A's order waits (B is out of A's range); B's self-order measures 0 and
    // is due in the same pass, after A in timeline order.
    let mut fx = ranged(move_rt(), 655_360);
    edit_ability(&mut fx, |x| x.allow_self_target = true);
    let (mut h, ids) = fx.start_history(7, EncounterMode::RealTime).unwrap();
    let (a, b) = (ids[0], ids[1]);
    h.perform_action(&use_ability(a, b)).unwrap();
    h.perform_action(&use_ability(b, b)).unwrap();
    h.place(b, rp(655_361, 0, 0)).unwrap();
    let cursor = h.last_sequence();
    h.tick().unwrap();
    let events = events_after(&h, cursor);
    assert_eq!(resolved(&events), 1, "{events:?}");
    assert!(events.iter().any(|e| matches!(
        e,
        HistoryEvent::ActionResolved { actor, target, .. } if *actor == b && *target == b
    )));
    assert!(ended(&events).is_empty());
    assert_eq!(pool_of(h.world(), b), 0);
    assert_eq!(order_of(h.world(), b), None);
    assert_eq!(order_of(h.world(), a), Some(standing(b, false)));
    assert_eq!(pool_of(h.world(), a), 1);
}

#[test]
fn r7_a_retained_order_keeps_its_repeat_flag() {
    let mut fx = ranged(move_rt(), 655_360);
    edit_ability(&mut fx, |x| x.repeats_in_real_time = Some(true));
    let (mut w, a, b) = rt(&fx);
    perform_action(&mut w, &use_ability(a, b)).unwrap();
    assert_eq!(order_of(&w, a), Some(standing(b, true)));
    put(&mut w, b, rp(655_361, 0, 0));
    for _ in 0..2 {
        tick(&mut w);
        assert_eq!(order_of(&w, a), Some(standing(b, true)));
    }
    put(&mut w, b, p(10, 0));
    tick(&mut w);
    // A repeating order survives its execution.
    assert_eq!(pool_of(&w, a), 0);
    assert!(order_of(&w, a).is_some_and(|o| o.repeat));
}

#[test]
fn r9_any_some_range_makes_the_definition_non_legacy() {
    for raw in [0, -1] {
        let (w, _) = ranged(Fixture::standard(), raw).start(7);
        let save = serde_json::to_value(&w).unwrap();
        assert_eq!(
            save["combat"]["definition"]["abilities"][0]["range"],
            json!(raw)
        );
        let loaded = reload(save);
        assert_eq!(
            loaded.combat().unwrap().definition.abilities[0].range,
            Some(Fx16_16::from_raw(raw))
        );
        assert_eq!(bytes_of(&loaded), bytes_of(&w));
    }
}
