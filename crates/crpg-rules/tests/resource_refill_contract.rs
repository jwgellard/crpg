//! Independent two-keys contract tests for T135 (pool refill amount and the
//! maximum setter), written from `tasks/T135.md` §Interface, §Behaviour and
//! §Tests R2–R8 alone.
//!
//! Neutral vocabulary only. No floats, no dice, no hash maps, no randomness.

use core::num::NonZeroU32;

use crpg_core::{Tick, Ulid};
use crpg_rules::{
    RefreshEvent, RefreshTrigger, ResourcePool, ResourcePoolId, RestId, RulesErrorCode,
};
use proptest::prelude::*;

const REST_A: u128 = 21;
const REST_B: u128 = 22;

fn pool_id(n: u128) -> ResourcePoolId {
    ResourcePoolId(Ulid::from_u128(n))
}

fn rest_id(n: u128) -> RestId {
    RestId(Ulid::from_u128(n))
}

fn nz(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap()
}

/// Behaviour 5: the invariant every public operation and decode must hold.
fn invariant(pool: &ResourcePool) {
    assert!(
        pool.current() <= pool.max(),
        "current {} exceeds max {}",
        pool.current(),
        pool.max()
    );
}

/// Behaviour 1: `new` returns exactly `with_refill(.., None)` for every one of
/// the five trigger kinds, in both value and encoded bytes.
#[test]
fn construction_new_equals_with_refill_none() {
    let triggers = [
        RefreshTrigger::OnTurnStart,
        RefreshTrigger::OnRoundStart,
        RefreshTrigger::OnRest(rest_id(3)),
        RefreshTrigger::OnTick(7),
        RefreshTrigger::Never,
    ];
    for trigger in triggers {
        let via_refill = ResourcePool::with_refill(pool_id(2), 5, 3, trigger, None).unwrap();
        assert!(via_refill.refill().is_none());
        let via_new = ResourcePool::new(pool_id(2), 5, 3, via_refill.refresh_trigger()).unwrap();
        assert_eq!(via_new, via_refill);
        assert_eq!(
            serde_json::to_string(&via_new).unwrap(),
            serde_json::to_string(&via_refill).unwrap()
        );
    }
}

/// Behaviour 1: the same validation in the same order with a refill present,
/// and the refill value itself adds no failure.
#[test]
fn construction_faults_order_and_refill_adds_no_failure() {
    let error = ResourcePool::with_refill(pool_id(1), 5, 6, RefreshTrigger::Never, Some(nz(2)))
        .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidResource);
    assert_eq!(error.location, "/resource/current");
    let error = ResourcePool::with_refill(pool_id(1), 5, 5, RefreshTrigger::OnTick(0), Some(nz(2)))
        .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidResource);
    assert_eq!(error.location, "/resource/refresh");
    // Both faults together report the first in contract order.
    let error = ResourcePool::with_refill(pool_id(1), 5, 6, RefreshTrigger::OnTick(0), Some(nz(2)))
        .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidResource);
    assert_eq!(error.location, "/resource/current");
    // `with_refill(.., None)` fails exactly as `new` does.
    let via_refill =
        ResourcePool::with_refill(pool_id(1), 5, 6, RefreshTrigger::Never, None).unwrap_err();
    let via_new = ResourcePool::new(pool_id(1), 5, 6, RefreshTrigger::Never).unwrap_err();
    assert_eq!(via_refill.code, via_new.code);
    assert_eq!(via_refill.location, via_new.location);
    // The refill value adds no failure: above max, and on a `Never` pool.
    ResourcePool::with_refill(
        pool_id(1),
        0,
        0,
        RefreshTrigger::OnTurnStart,
        Some(nz(u32::MAX)),
    )
    .unwrap();
    ResourcePool::with_refill(pool_id(1), 5, 3, RefreshTrigger::Never, Some(nz(9))).unwrap();
}

/// Behaviour 2: the refresh match rule with a partial refill, exactly the
/// table of contract §Tests R3.
#[test]
fn refresh_partial_refill_table() {
    let rows = vec![
        // (max, current, refill, trigger, event, want current, want changed)
        (
            6,
            1,
            Some(nz(2)),
            RefreshTrigger::OnTurnStart,
            RefreshEvent::TurnStart,
            3,
            true,
        ),
        (
            6,
            5,
            Some(nz(2)),
            RefreshTrigger::OnTurnStart,
            RefreshEvent::TurnStart,
            6,
            true,
        ),
        (
            6,
            6,
            Some(nz(2)),
            RefreshTrigger::OnTurnStart,
            RefreshEvent::TurnStart,
            6,
            false,
        ),
        (
            0,
            0,
            Some(nz(1)),
            RefreshTrigger::OnTurnStart,
            RefreshEvent::TurnStart,
            0,
            false,
        ),
        (
            6,
            1,
            Some(nz(10)),
            RefreshTrigger::OnTurnStart,
            RefreshEvent::TurnStart,
            6,
            true,
        ),
        (
            u32::MAX,
            u32::MAX - 1,
            Some(nz(u32::MAX)),
            RefreshTrigger::OnTurnStart,
            RefreshEvent::TurnStart,
            u32::MAX,
            true,
        ),
        // A non-matching event leaves the pool unchanged and returns false.
        (
            6,
            1,
            Some(nz(2)),
            RefreshTrigger::OnTurnStart,
            RefreshEvent::RoundStart,
            1,
            false,
        ),
        // Rest triggers match their exact rest only.
        (
            6,
            1,
            Some(nz(2)),
            RefreshTrigger::OnRest(rest_id(3)),
            RefreshEvent::Rest(rest_id(4)),
            1,
            false,
        ),
        (
            6,
            1,
            Some(nz(2)),
            RefreshTrigger::OnRest(rest_id(3)),
            RefreshEvent::Rest(rest_id(3)),
            3,
            true,
        ),
        // `Never` matches no event at all.
        (
            6,
            1,
            Some(nz(2)),
            RefreshTrigger::Never,
            RefreshEvent::TurnStart,
            1,
            false,
        ),
        (
            6,
            1,
            Some(nz(2)),
            RefreshTrigger::Never,
            RefreshEvent::RoundStart,
            1,
            false,
        ),
        (
            6,
            1,
            Some(nz(2)),
            RefreshTrigger::Never,
            RefreshEvent::Rest(rest_id(3)),
            1,
            false,
        ),
        (
            6,
            1,
            Some(nz(2)),
            RefreshTrigger::Never,
            RefreshEvent::Tick(Tick::new(7)),
            1,
            false,
        ),
    ];
    for (max, current, refill, trigger, event, want, changed) in rows {
        let mut pool =
            ResourcePool::with_refill(pool_id(1), max, current, trigger, refill).unwrap();
        assert_eq!(pool.refresh(event), changed);
        assert_eq!(pool.current(), want);
        invariant(&pool);
    }
    // `OnTick(5)` with `Some(2)`: ticks 0 and 4 give false; 5 and 10 add 2.
    let mut ticks =
        ResourcePool::with_refill(pool_id(1), 10, 1, RefreshTrigger::OnTick(5), Some(nz(2)))
            .unwrap();
    assert!(!ticks.refresh(RefreshEvent::Tick(Tick::ZERO)));
    assert!(!ticks.refresh(RefreshEvent::Tick(Tick::new(4))));
    assert_eq!(ticks.current(), 1);
    assert!(ticks.refresh(RefreshEvent::Tick(Tick::new(5))));
    assert_eq!(ticks.current(), 3);
    assert!(ticks.refresh(RefreshEvent::Tick(Tick::new(10))));
    assert_eq!(ticks.current(), 5);
}

/// Behaviour 2: the `None` rows repeat the T015 full-refill cases and must
/// agree with them.
#[test]
fn refresh_full_refill_rows_agree_with_t015() {
    let mut turn =
        ResourcePool::with_refill(pool_id(1), 4, 1, RefreshTrigger::OnTurnStart, None).unwrap();
    assert!(turn.refresh(RefreshEvent::TurnStart));
    assert_eq!(turn.current(), 4);
    assert!(!turn.refresh(RefreshEvent::TurnStart));
    assert_eq!(turn.current(), 4);
    assert!(!turn.refresh(RefreshEvent::RoundStart));
    // Positive tick multiples match, never tick zero.
    let mut ticks =
        ResourcePool::with_refill(pool_id(1), 9, 2, RefreshTrigger::OnTick(3), None).unwrap();
    assert!(!ticks.refresh(RefreshEvent::Tick(Tick::ZERO)));
    assert!(!ticks.refresh(RefreshEvent::Tick(Tick::new(1))));
    assert!(!ticks.refresh(RefreshEvent::Tick(Tick::new(2))));
    assert!(ticks.refresh(RefreshEvent::Tick(Tick::new(3))));
    assert_eq!(ticks.current(), 9);
    assert!(!ticks.refresh(RefreshEvent::Tick(Tick::new(6))));
    ticks.spend(4).unwrap();
    assert!(ticks.refresh(RefreshEvent::Tick(Tick::new(6))));
    assert_eq!(ticks.current(), 9);
    // Rest triggers match their exact rest only.
    let mut rest =
        ResourcePool::with_refill(pool_id(1), 4, 1, RefreshTrigger::OnRest(rest_id(3)), None)
            .unwrap();
    assert!(!rest.refresh(RefreshEvent::Rest(rest_id(4))));
    assert!(rest.refresh(RefreshEvent::Rest(rest_id(3))));
    assert_eq!(rest.current(), 4);
    // `Never` matches nothing.
    let mut never =
        ResourcePool::with_refill(pool_id(1), 4, 1, RefreshTrigger::Never, None).unwrap();
    assert!(!never.refresh(RefreshEvent::TurnStart));
    assert_eq!(never.current(), 1);
}

/// Behaviour 3: repeated deliveries are not idempotent for a partial-refill
/// pool; each matching delivery adds the amount until the pool is full.
#[test]
fn repeated_delivery_is_not_idempotent_for_partial_refill() {
    let mut pool =
        ResourcePool::with_refill(pool_id(1), 6, 0, RefreshTrigger::OnTurnStart, Some(nz(2)))
            .unwrap();
    let expected = [(2, true), (4, true), (6, true), (6, false)];
    for &(want, changed) in &expected {
        assert_eq!(pool.refresh(RefreshEvent::TurnStart), changed);
        assert_eq!(pool.current(), want);
        invariant(&pool);
    }
}

/// Behaviour 4: `set_max` sets the maximum, clamps `current` down, never
/// raises `current`, leaves `refill`/`refresh` untouched, and returns true
/// exactly when `max` changed.
#[test]
fn set_max_clamps_current_and_reports_change() {
    // Clamping down changes both max and current.
    let mut pool =
        ResourcePool::with_refill(pool_id(1), 6, 5, RefreshTrigger::OnTurnStart, Some(nz(2)))
            .unwrap();
    assert!(pool.set_max(3));
    assert_eq!((pool.max(), pool.current()), (3, 3));
    assert_eq!(pool.refill(), Some(nz(2)));
    assert_eq!(pool.refresh_trigger(), RefreshTrigger::OnTurnStart);
    // Raising the maximum never raises current.
    let mut pool =
        ResourcePool::with_refill(pool_id(1), 6, 2, RefreshTrigger::OnTurnStart, Some(nz(2)))
            .unwrap();
    assert!(pool.set_max(4));
    assert_eq!((pool.max(), pool.current()), (4, 2));
    assert_eq!(pool.refill(), Some(nz(2)));
    assert_eq!(pool.refresh_trigger(), RefreshTrigger::OnTurnStart);
    // A second identical call changes nothing and returns false.
    let mut pool =
        ResourcePool::with_refill(pool_id(1), 4, 2, RefreshTrigger::OnTurnStart, Some(nz(2)))
            .unwrap();
    assert!(pool.set_max(9));
    assert_eq!((pool.max(), pool.current()), (9, 2));
    assert!(!pool.set_max(9));
    assert_eq!((pool.max(), pool.current()), (9, 2));
    assert_eq!(pool.refill(), Some(nz(2)));
    assert_eq!(pool.refresh_trigger(), RefreshTrigger::OnTurnStart);
    // A zero maximum is valid and clamps current down to zero.
    let mut pool =
        ResourcePool::with_refill(pool_id(1), 6, 5, RefreshTrigger::OnTurnStart, Some(nz(2)))
            .unwrap();
    assert!(pool.set_max(0));
    assert_eq!((pool.max(), pool.current()), (0, 0));
    assert_eq!(pool.refill(), Some(nz(2)));
    assert_eq!(pool.refresh_trigger(), RefreshTrigger::OnTurnStart);
    // After (max 9, current 2) a matching refresh fills with `None`...
    let mut full =
        ResourcePool::with_refill(pool_id(1), 9, 2, RefreshTrigger::OnTurnStart, None).unwrap();
    assert!(full.refresh(RefreshEvent::TurnStart));
    assert_eq!(full.current(), 9);
    // ...and adds the refill amount with `Some(2)`.
    let mut part =
        ResourcePool::with_refill(pool_id(1), 9, 2, RefreshTrigger::OnTurnStart, Some(nz(2)))
            .unwrap();
    assert!(part.refresh(RefreshEvent::TurnStart));
    assert_eq!(part.current(), 4);
}

/// Behaviour 5: `current <= max` after every public operation and after every
/// successful decode.
#[test]
fn invariant_current_never_exceeds_max() {
    let mut pool =
        ResourcePool::with_refill(pool_id(1), 6, 4, RefreshTrigger::OnTurnStart, Some(nz(2)))
            .unwrap();
    invariant(&pool);
    pool.spend(1).unwrap();
    invariant(&pool);
    pool.refresh(RefreshEvent::TurnStart);
    invariant(&pool);
    pool.set_max(2);
    invariant(&pool);
    pool.set_max(9);
    invariant(&pool);
    assert!(pool.spend(5).is_err());
    invariant(&pool);
    pool.spend(2).unwrap();
    invariant(&pool);
    pool.refresh(RefreshEvent::TurnStart);
    invariant(&pool);
    pool.set_max(0);
    invariant(&pool);
    pool.set_max(7);
    invariant(&pool);
    pool.refresh(RefreshEvent::TurnStart);
    invariant(&pool);
    // Successful decode preserves the invariant, including the edges.
    let wire = serde_json::to_string(&pool).unwrap();
    invariant(&serde_json::from_str::<ResourcePool>(&wire).unwrap());
    let ulid = Ulid::from_u128(2);
    for wire in [
        format!("{{\"id\":\"{ulid}\",\"max\":0,\"current\":0,\"refresh\":{{\"type\":\"never\"}}}}"),
        format!(
            "{{\"id\":\"{ulid}\",\"max\":4294967295,\"current\":4294967295,\"refresh\":{{\"type\":\"never\"}},\"refill\":1}}"
        ),
    ] {
        invariant(&serde_json::from_str::<ResourcePool>(&wire).unwrap());
    }
    invariant(&ResourcePool::new(pool_id(1), 0, 0, RefreshTrigger::Never).unwrap());
    invariant(&ResourcePool::new(pool_id(1), u32::MAX, u32::MAX, RefreshTrigger::Never).unwrap());
}

/// Behaviour 6: equality is the derived equality and includes `refill`, and
/// `Debug` gained a `refill` field (its exact shape is not pinned).
#[test]
fn equality_and_debug_include_refill() {
    let plain = ResourcePool::new(pool_id(1), 5, 3, RefreshTrigger::OnTurnStart).unwrap();
    let none =
        ResourcePool::with_refill(pool_id(1), 5, 3, RefreshTrigger::OnTurnStart, None).unwrap();
    let two = ResourcePool::with_refill(pool_id(1), 5, 3, RefreshTrigger::OnTurnStart, Some(nz(2)))
        .unwrap();
    let two_again =
        ResourcePool::with_refill(pool_id(1), 5, 3, RefreshTrigger::OnTurnStart, Some(nz(2)))
            .unwrap();
    let three =
        ResourcePool::with_refill(pool_id(1), 5, 3, RefreshTrigger::OnTurnStart, Some(nz(3)))
            .unwrap();
    assert_eq!(plain, none);
    assert_eq!(two, two_again);
    assert_ne!(none, two);
    assert_ne!(two, three);
    assert!(format!("{two:?}").contains("refill"));
    assert!(format!("{none:?}").contains("refill"));
}

/// Behaviour 7 (H1): without a refill the encoding is byte-identical to the
/// four-field object.
#[test]
fn encode_without_refill_is_the_four_field_object() {
    let cases = [
        (
            ResourcePool::new(pool_id(2), 5, 3, RefreshTrigger::OnTurnStart).unwrap(),
            format!(
                "{{\"id\":\"{}\",\"max\":5,\"current\":3,\"refresh\":{{\"type\":\"on_turn_start\"}}}}",
                Ulid::from_u128(2)
            ),
        ),
        (
            ResourcePool::new(pool_id(5), 5, 3, RefreshTrigger::OnTick(7)).unwrap(),
            format!(
                "{{\"id\":\"{}\",\"max\":5,\"current\":3,\"refresh\":{{\"type\":\"on_tick\",\"value\":7}}}}",
                Ulid::from_u128(5)
            ),
        ),
        (
            ResourcePool::new(pool_id(6), 0, 0, RefreshTrigger::Never).unwrap(),
            format!(
                "{{\"id\":\"{}\",\"max\":0,\"current\":0,\"refresh\":{{\"type\":\"never\"}}}}",
                Ulid::from_u128(6)
            ),
        ),
    ];
    for (pool, wire) in &cases {
        let encoded = serde_json::to_string(pool).unwrap();
        assert_eq!(encoded, *wire);
        assert!(!encoded.contains("refill"));
    }
}

/// Behaviour 7 (H2): with a refill the same four fields in the same order are
/// followed by `"refill"` as a decimal `u32`, and both shapes decode back to
/// equal values (contract §Tests R6).
#[test]
fn encode_with_refill_adds_the_fifth_field() {
    let ulid = Ulid::from_u128(2);
    let turn =
        ResourcePool::with_refill(pool_id(2), 5, 3, RefreshTrigger::OnTurnStart, Some(nz(2)))
            .unwrap();
    let turn_wire = format!(
        "{{\"id\":\"{ulid}\",\"max\":5,\"current\":3,\"refresh\":{{\"type\":\"on_turn_start\"}},\"refill\":2}}"
    );
    assert_eq!(serde_json::to_string(&turn).unwrap(), turn_wire);
    let tick = ResourcePool::with_refill(
        pool_id(2),
        5,
        3,
        RefreshTrigger::OnTick(7),
        Some(nz(u32::MAX)),
    )
    .unwrap();
    let tick_wire = format!(
        "{{\"id\":\"{ulid}\",\"max\":5,\"current\":3,\"refresh\":{{\"type\":\"on_tick\",\"value\":7}},\"refill\":4294967295}}"
    );
    assert_eq!(serde_json::to_string(&tick).unwrap(), tick_wire);
    // Both decode back to equal values.
    let loaded = serde_json::from_str::<ResourcePool>(&turn_wire).unwrap();
    assert_eq!(loaded, turn);
    assert_eq!(loaded.refill(), Some(nz(2)));
    let loaded = serde_json::from_str::<ResourcePool>(&tick_wire).unwrap();
    assert_eq!(loaded, tick);
    assert_eq!(loaded.refill(), Some(nz(u32::MAX)));
}

/// Behaviour 8 (H3): `refill` may be absent or appear at any key position,
/// and a present value must be an integer in `1..=u32::MAX`.
#[test]
fn decode_accepts_refill_in_any_key_position() {
    let ulid = Ulid::from_u128(2);
    // First key.
    let wire = format!(
        "{{\"refill\":2,\"id\":\"{ulid}\",\"max\":5,\"current\":3,\"refresh\":{{\"type\":\"on_turn_start\"}}}}"
    );
    let loaded = serde_json::from_str::<ResourcePool>(&wire).unwrap();
    assert_eq!(
        loaded,
        ResourcePool::with_refill(pool_id(2), 5, 3, RefreshTrigger::OnTurnStart, Some(nz(2)))
            .unwrap()
    );
    // Middle key, smallest valid value.
    let wire = format!(
        "{{\"id\":\"{ulid}\",\"max\":5,\"refill\":1,\"current\":3,\"refresh\":{{\"type\":\"on_turn_start\"}}}}"
    );
    let loaded = serde_json::from_str::<ResourcePool>(&wire).unwrap();
    assert_eq!(loaded.refill(), Some(nz(1)));
    // Middle key, largest valid value.
    let wire = format!(
        "{{\"id\":\"{ulid}\",\"max\":5,\"refill\":4294967295,\"current\":3,\"refresh\":{{\"type\":\"on_turn_start\"}}}}"
    );
    let loaded = serde_json::from_str::<ResourcePool>(&wire).unwrap();
    assert_eq!(loaded.refill(), Some(nz(u32::MAX)));
}

/// Behaviour 8 (H3): invalid `refill` values, a duplicate key, and the
/// near-miss key name are all rejected (contract §Tests R7).
#[test]
fn decode_rejects_invalid_refill_values() {
    let ulid = Ulid::from_u128(2);
    let bad_values = ["0", "null", "-1", "4294967296", "\"2\"", "2.0"];
    for value in bad_values {
        let wire = format!(
            "{{\"id\":\"{ulid}\",\"max\":5,\"current\":3,\"refresh\":{{\"type\":\"on_turn_start\"}},\"refill\":{value}}}"
        );
        assert!(
            serde_json::from_str::<ResourcePool>(&wire).is_err(),
            "accepted invalid refill {value}"
        );
    }
    // A duplicate `refill` key is rejected.
    let wire = format!(
        "{{\"id\":\"{ulid}\",\"max\":5,\"current\":3,\"refresh\":{{\"type\":\"on_turn_start\"}},\"refill\":2,\"refill\":2}}"
    );
    assert!(serde_json::from_str::<ResourcePool>(&wire).is_err());
    // The near-miss key name is unknown and stays rejected.
    let wire = format!(
        "{{\"id\":\"{ulid}\",\"max\":5,\"current\":3,\"refresh\":{{\"type\":\"on_turn_start\"}},\"refil\":2}}"
    );
    assert!(serde_json::from_str::<ResourcePool>(&wire).is_err());
}

/// Behaviour 8 (H3): the construction faults of item 1 still reject a pool
/// when `refill` is present.
#[test]
fn decode_rejects_invalid_pools_with_refill_present() {
    let ulid = Ulid::from_u128(2);
    let wire = format!(
        "{{\"id\":\"{ulid}\",\"max\":5,\"current\":6,\"refresh\":{{\"type\":\"never\"}},\"refill\":2}}"
    );
    assert!(serde_json::from_str::<ResourcePool>(&wire).is_err());
    let wire = format!(
        "{{\"id\":\"{ulid}\",\"max\":5,\"current\":5,\"refresh\":{{\"type\":\"on_tick\",\"value\":0}},\"refill\":2}}"
    );
    assert!(serde_json::from_str::<ResourcePool>(&wire).is_err());
}

/// Behaviour 9 (H4): old saves decode unchanged because the key is absent,
/// and re-encode to the identical legacy bytes.
#[test]
fn legacy_wire_without_the_key_decodes_unchanged() {
    let ulid = Ulid::from_u128(2);
    let wire = format!(
        "{{\"id\":\"{ulid}\",\"max\":5,\"current\":3,\"refresh\":{{\"type\":\"on_turn_start\"}}}}"
    );
    let loaded = serde_json::from_str::<ResourcePool>(&wire).unwrap();
    assert!(loaded.refill().is_none());
    assert_eq!(
        loaded,
        ResourcePool::new(pool_id(2), 5, 3, RefreshTrigger::OnTurnStart).unwrap()
    );
    assert_eq!(serde_json::to_string(&loaded).unwrap(), wire);
}

#[derive(Clone, Debug)]
enum TriggerSpec {
    TurnStart,
    RoundStart,
    Rest(u128),
    Tick(u64),
    Never,
}

#[derive(Clone, Debug)]
enum EventSpec {
    TurnStart,
    RoundStart,
    Rest(u128),
    Tick(u64),
}

#[derive(Debug)]
enum Op {
    Spend(u32),
    Refresh(EventSpec),
    SetMax(u32),
}

fn trigger_of(spec: &TriggerSpec) -> RefreshTrigger {
    match spec {
        TriggerSpec::TurnStart => RefreshTrigger::OnTurnStart,
        TriggerSpec::RoundStart => RefreshTrigger::OnRoundStart,
        TriggerSpec::Rest(n) => RefreshTrigger::OnRest(rest_id(*n)),
        TriggerSpec::Tick(p) => RefreshTrigger::OnTick(*p),
        TriggerSpec::Never => RefreshTrigger::Never,
    }
}

fn event_of(spec: &EventSpec) -> RefreshEvent {
    match spec {
        EventSpec::TurnStart => RefreshEvent::TurnStart,
        EventSpec::RoundStart => RefreshEvent::RoundStart,
        EventSpec::Rest(n) => RefreshEvent::Rest(rest_id(*n)),
        EventSpec::Tick(t) => RefreshEvent::Tick(Tick::new(*t)),
    }
}

/// The T015 match rule: kinds match exactly, rest matches its exact id, tick
/// matches positive multiples of the period, `Never` matches nothing.
fn matches(trigger: &TriggerSpec, event: &EventSpec) -> bool {
    match (trigger, event) {
        (TriggerSpec::TurnStart, EventSpec::TurnStart) => true,
        (TriggerSpec::RoundStart, EventSpec::RoundStart) => true,
        (TriggerSpec::Rest(a), EventSpec::Rest(b)) => a == b,
        (TriggerSpec::Tick(p), EventSpec::Tick(t)) => *t != 0 && t % p == 0,
        _ => false,
    }
}

fn trigger_spec() -> impl Strategy<Value = TriggerSpec> {
    prop_oneof![
        Just(TriggerSpec::TurnStart),
        Just(TriggerSpec::RoundStart),
        Just(TriggerSpec::Never),
        (1u64..=16).prop_map(TriggerSpec::Tick),
        prop_oneof![Just(REST_A), Just(REST_B)].prop_map(TriggerSpec::Rest),
    ]
}

fn event_spec() -> impl Strategy<Value = EventSpec> {
    prop_oneof![
        Just(EventSpec::TurnStart),
        Just(EventSpec::RoundStart),
        prop_oneof![Just(REST_A), Just(REST_B)].prop_map(EventSpec::Rest),
        (0u64..=64).prop_map(EventSpec::Tick),
    ]
}

fn refill_spec() -> impl Strategy<Value = Option<NonZeroU32>> {
    prop_oneof![
        1 => Just(None),
        1 => Just(Some(nz(1))),
        1 => Just(Some(nz(u32::MAX))),
        3 => any::<u32>().prop_map(|v| Some(nz(v.max(1)))),
    ]
}

fn op_spec() -> impl Strategy<Value = Op> {
    prop_oneof![
        any::<u32>().prop_map(Op::Spend),
        event_spec().prop_map(Op::Refresh),
        any::<u32>().prop_map(Op::SetMax),
    ]
}

// Behaviour 5 and contract §Tests R8: 256 cases of at most 32 operations each
// against a two-integer reference model, checking the invariant, the return
// values, the untouched getters, and the wire round trip after every step.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn pool_matches_the_two_integer_reference_model(
        bounds in any::<u32>().prop_flat_map(|max| (Just(max), 0..=max)),
        refill in refill_spec(),
        trigger in trigger_spec(),
        ops in proptest::collection::vec(op_spec(), 0..=32),
    ) {
        let (max, current) = bounds;
        let mut pool = ResourcePool::with_refill(
            pool_id(1), max, current, trigger_of(&trigger), refill,
        ).unwrap();
        let (mut model_max, mut model_current) = (max, current);
        for op in ops {
            match op {
                Op::Spend(k) => {
                    let expect_ok = k <= model_current;
                    let result = pool.spend(k);
                    prop_assert_eq!(result.is_ok(), expect_ok, "spend({})", k);
                    if let Err(error) = result {
                        prop_assert_eq!(error.code, RulesErrorCode::InsufficientResource);
                        prop_assert_eq!(error.location, "/resource/amount");
                    }
                    if expect_ok {
                        model_current -= k;
                    }
                }
                Op::Refresh(spec) => {
                    let expect_changed = if matches(&trigger, &spec) {
                        match refill {
                            None => {
                                let changed = model_current != model_max;
                                model_current = model_max;
                                changed
                            }
                            Some(n) => {
                                let target = model_current
                                    .saturating_add(n.get())
                                    .min(model_max);
                                let changed = target != model_current;
                                model_current = target;
                                changed
                            }
                        }
                    } else {
                        false
                    };
                    prop_assert_eq!(pool.refresh(event_of(&spec)), expect_changed);
                }
                Op::SetMax(new_max) => {
                    let expect_changed = new_max != model_max;
                    model_max = new_max;
                    if model_current > model_max {
                        model_current = model_max;
                    }
                    prop_assert_eq!(pool.set_max(new_max), expect_changed);
                }
            }
            // The two-integer model, the invariant, and the untouched getters.
            prop_assert!(model_current <= model_max);
            prop_assert!(pool.current() <= pool.max());
            prop_assert_eq!(pool.current(), model_current);
            prop_assert_eq!(pool.max(), model_max);
            prop_assert_eq!(pool.refill(), refill);
            prop_assert_eq!(pool.refresh_trigger(), trigger_of(&trigger));
            // Wire: the key appears exactly when a refill is present, and a
            // round trip returns an equal pool.
            let wire = serde_json::to_string(&pool).unwrap();
            prop_assert_eq!(wire.contains("\"refill\""), refill.is_some());
            let decoded = serde_json::from_str::<ResourcePool>(&wire).unwrap();
            prop_assert_eq!(&decoded, &pool);
        }
    }
}
