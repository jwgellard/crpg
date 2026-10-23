//! Resource pool refill amount and clamping maximum setter (T135).
//!
//! Neutral vocabulary only. Counts are whole; no floats, no dice.

use core::num::NonZeroU32;

use crpg_core::{Tick, Ulid};
use crpg_rules::{
    RefreshEvent, RefreshTrigger, ResourcePool, ResourcePoolId, RestId, RulesErrorCode,
};
use proptest::prelude::*;

fn pool_id(n: u128) -> ResourcePoolId {
    ResourcePoolId(Ulid::from_u128(n))
}

fn rest_id(n: u128) -> RestId {
    RestId(Ulid::from_u128(n))
}

fn nz(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap()
}

fn partial(max: u32, current: u32, refresh: RefreshTrigger, refill: u32) -> ResourcePool {
    ResourcePool::with_refill(pool_id(1), max, current, refresh, Some(nz(refill))).unwrap()
}

fn full(max: u32, current: u32, refresh: RefreshTrigger) -> ResourcePool {
    ResourcePool::with_refill(pool_id(1), max, current, refresh, None).unwrap()
}

fn all_triggers() -> [RefreshTrigger; 5] {
    [
        RefreshTrigger::OnTurnStart,
        RefreshTrigger::OnRoundStart,
        RefreshTrigger::OnRest(rest_id(3)),
        RefreshTrigger::OnTick(5),
        RefreshTrigger::Never,
    ]
}

// R2 (Behaviour 1): construction.
#[test]
fn new_equals_with_refill_none_for_every_trigger() {
    for trigger in all_triggers() {
        let old = ResourcePool::new(pool_id(1), 5, 3, trigger).unwrap();
        let new = ResourcePool::with_refill(pool_id(1), 5, 3, trigger, None).unwrap();
        assert_eq!(old, new);
        assert_eq!(
            serde_json::to_string(&old).unwrap(),
            serde_json::to_string(&new).unwrap()
        );
        assert_eq!(new.refill(), None);
    }
}

#[test]
fn with_refill_validates_in_the_same_order_as_new() {
    let some = Some(nz(2));
    let error =
        ResourcePool::with_refill(pool_id(1), 5, 6, RefreshTrigger::Never, some).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidResource);
    assert_eq!(error.location, "/resource/current");
    let error =
        ResourcePool::with_refill(pool_id(1), 5, 5, RefreshTrigger::OnTick(0), some).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidResource);
    assert_eq!(error.location, "/resource/refresh");
    // Both faults together report the first in order.
    let error =
        ResourcePool::with_refill(pool_id(1), 5, 6, RefreshTrigger::OnTick(0), some).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidResource);
    assert_eq!(error.location, "/resource/current");
    // The refill amount adds no failure: above max, zero max, and Never.
    assert_eq!(
        partial(3, 1, RefreshTrigger::Never, 100).refill(),
        Some(nz(100))
    );
    assert_eq!(partial(0, 0, RefreshTrigger::OnTurnStart, 1).max(), 0);
}

// R3 (Behaviour 2): the refresh table.
#[test]
fn partial_refresh_table_on_turn_start() {
    let cases: [(u32, u32, u32, u32, bool); 6] = [
        // (max, current, refill, expected current, expected changed)
        (6, 1, 2, 3, true),
        (6, 5, 2, 6, true),
        (6, 6, 2, 6, false),
        (0, 0, 1, 0, false),
        (6, 1, 10, 6, true),
        (u32::MAX, u32::MAX - 1, u32::MAX, u32::MAX, true),
    ];
    for (max, current, refill, want, changed) in cases {
        let mut pool = partial(max, current, RefreshTrigger::OnTurnStart, refill);
        assert_eq!(
            pool.refresh(RefreshEvent::TurnStart),
            changed,
            "({max}, {current}, {refill})"
        );
        assert_eq!(pool.current(), want, "({max}, {current}, {refill})");
        assert_eq!(pool.max(), max);
    }
}

#[test]
fn partial_refresh_ignores_non_matching_events() {
    let mut pool = partial(6, 1, RefreshTrigger::OnTurnStart, 2);
    let before = pool.clone();
    assert!(!pool.refresh(RefreshEvent::RoundStart));
    assert!(!pool.refresh(RefreshEvent::Rest(rest_id(3))));
    assert!(!pool.refresh(RefreshEvent::Tick(Tick::new(5))));
    assert_eq!(pool, before);
}

#[test]
fn partial_refresh_on_tick_matches_positive_multiples() {
    let mut pool = partial(20, 0, RefreshTrigger::OnTick(5), 2);
    assert!(!pool.refresh(RefreshEvent::Tick(Tick::ZERO)));
    assert!(!pool.refresh(RefreshEvent::Tick(Tick::new(4))));
    assert_eq!(pool.current(), 0);
    assert!(pool.refresh(RefreshEvent::Tick(Tick::new(5))));
    assert_eq!(pool.current(), 2);
    assert!(pool.refresh(RefreshEvent::Tick(Tick::new(10))));
    assert_eq!(pool.current(), 4);
}

#[test]
fn partial_refresh_on_rest_matches_exact_rest() {
    let rest = rest_id(3);
    let mut pool = partial(9, 1, RefreshTrigger::OnRest(rest), 2);
    assert!(!pool.refresh(RefreshEvent::Rest(rest_id(4))));
    assert_eq!(pool.current(), 1);
    assert!(pool.refresh(RefreshEvent::Rest(rest)));
    assert_eq!(pool.current(), 3);
}

#[test]
fn partial_never_pool_ignores_every_event() {
    let mut pool = partial(9, 1, RefreshTrigger::Never, 2);
    for event in [
        RefreshEvent::TurnStart,
        RefreshEvent::RoundStart,
        RefreshEvent::Rest(rest_id(3)),
        RefreshEvent::Tick(Tick::new(10)),
    ] {
        assert!(!pool.refresh(event));
        assert_eq!(pool.current(), 1);
    }
}

#[test]
fn full_refill_rows_agree_with_the_t015_cases() {
    let mut turn = full(4, 1, RefreshTrigger::OnTurnStart);
    assert!(!turn.refresh(RefreshEvent::RoundStart));
    assert_eq!(turn.current(), 1);
    assert!(turn.refresh(RefreshEvent::TurnStart));
    assert_eq!(turn.current(), 4);
    assert!(!turn.refresh(RefreshEvent::TurnStart));
    let mut tick = full(9, 2, RefreshTrigger::OnTick(3));
    assert!(!tick.refresh(RefreshEvent::Tick(Tick::ZERO)));
    assert!(tick.refresh(RefreshEvent::Tick(Tick::new(3))));
    assert_eq!(tick.current(), 9);
    assert!(!tick.refresh(RefreshEvent::Tick(Tick::new(6))));
    let mut never = full(4, 1, RefreshTrigger::Never);
    assert!(!never.refresh(RefreshEvent::TurnStart));
    assert_eq!(never.current(), 1);
}

// R4 (Behaviour 3): repeated delivery is not idempotent for partial pools.
#[test]
fn repeated_delivery_gains_until_full() {
    let mut pool = partial(6, 0, RefreshTrigger::OnTurnStart, 2);
    for want in [2, 4, 6] {
        assert!(pool.refresh(RefreshEvent::TurnStart));
        assert_eq!(pool.current(), want);
    }
    assert!(!pool.refresh(RefreshEvent::TurnStart));
    assert_eq!(pool.current(), 6);
}

// R5 (Behaviour 4): set_max clamps.
#[test]
fn set_max_clamps_and_never_raises_current() {
    for refill in [None, Some(nz(2))] {
        let make = |max: u32, current: u32| {
            ResourcePool::with_refill(
                pool_id(1),
                max,
                current,
                RefreshTrigger::OnTurnStart,
                refill,
            )
            .unwrap()
        };
        let mut lower = make(6, 5);
        assert!(lower.set_max(3));
        assert_eq!((lower.max(), lower.current()), (3, 3));

        let mut above = make(6, 2);
        assert!(above.set_max(4));
        assert_eq!((above.max(), above.current()), (4, 2));

        let mut raise = make(4, 2);
        assert!(raise.set_max(9));
        assert_eq!((raise.max(), raise.current()), (9, 2));
        let before = raise.clone();
        assert!(!raise.set_max(9));
        assert_eq!(raise, before);
        for pool in [&lower, &above, &raise] {
            assert_eq!(pool.refill(), refill);
            assert_eq!(pool.refresh_trigger(), RefreshTrigger::OnTurnStart);
        }

        let mut zero = make(6, 4);
        assert!(zero.set_max(0));
        assert_eq!((zero.max(), zero.current()), (0, 0));
        assert_eq!(zero.refill(), refill);

        // A matching refresh after (9, 2): full refill reaches 9, partial adds 2.
        assert!(raise.refresh(RefreshEvent::TurnStart));
        assert_eq!(raise.current(), if refill.is_none() { 9 } else { 4 });
    }
}

// R6 (Behaviour 7, H2): exact wire strings.
#[test]
fn present_refill_wire_shape_is_exact() {
    let turn =
        ResourcePool::with_refill(pool_id(2), 5, 3, RefreshTrigger::OnTurnStart, Some(nz(2)))
            .unwrap();
    let wire = format!(
        "{{\"id\":\"{}\",\"max\":5,\"current\":3,\"refresh\":{{\"type\":\"on_turn_start\"}},\"refill\":2}}",
        Ulid::from_u128(2)
    );
    assert_eq!(serde_json::to_string(&turn).unwrap(), wire);
    assert_eq!(serde_json::from_str::<ResourcePool>(&wire).unwrap(), turn);

    let tick = ResourcePool::with_refill(
        pool_id(3),
        5,
        3,
        RefreshTrigger::OnTick(7),
        Some(nz(u32::MAX)),
    )
    .unwrap();
    let wire = format!(
        "{{\"id\":\"{}\",\"max\":5,\"current\":3,\"refresh\":{{\"type\":\"on_tick\",\"value\":7}},\"refill\":4294967295}}",
        Ulid::from_u128(3)
    );
    assert_eq!(serde_json::to_string(&tick).unwrap(), wire);
    assert_eq!(serde_json::from_str::<ResourcePool>(&wire).unwrap(), tick);
}

#[test]
fn absent_refill_encodes_as_the_four_field_object() {
    let pool = full(5, 3, RefreshTrigger::Never);
    let wire = serde_json::to_string(&pool).unwrap();
    assert_eq!(
        wire,
        format!(
            "{{\"id\":\"{}\",\"max\":5,\"current\":3,\"refresh\":{{\"type\":\"never\"}}}}",
            Ulid::from_u128(1)
        )
    );
    assert!(!wire.contains("refill"));
}

// R7 (Behaviour 8, H3): decode.
fn wire_with(
    extra_before: &str,
    extra_after: &str,
    max: u32,
    current: u32,
    refresh: &str,
) -> String {
    format!(
        "{{{extra_before}\"id\":\"{}\",\"max\":{max},\"current\":{current},\"refresh\":{refresh}{extra_after}}}",
        Ulid::from_u128(2)
    )
}

#[test]
fn refill_may_appear_at_any_key_position() {
    let never = "{\"type\":\"never\"}";
    let first = wire_with("\"refill\":2,", "", 5, 3, never);
    let last = wire_with("", ",\"refill\":2", 5, 3, never);
    let expected =
        ResourcePool::with_refill(pool_id(2), 5, 3, RefreshTrigger::Never, Some(nz(2))).unwrap();
    assert_eq!(
        serde_json::from_str::<ResourcePool>(&first).unwrap(),
        expected
    );
    assert_eq!(
        serde_json::from_str::<ResourcePool>(&last).unwrap(),
        expected
    );
    // Absent refill decodes as None.
    let absent = wire_with("", "", 5, 3, never);
    assert_eq!(
        serde_json::from_str::<ResourcePool>(&absent)
            .unwrap()
            .refill(),
        None
    );
}

#[test]
fn invalid_refill_wire_is_rejected() {
    let never = "{\"type\":\"never\"}";
    let rejected = [
        "\"refill\":0",
        "\"refill\":null",
        "\"refill\":-1",
        "\"refill\":4294967296",
        "\"refill\":\"2\"",
        "\"refill\":2.0",
        "\"refill\":2,\"refill\":2",
        "\"refil\":2",
    ];
    for tail in rejected {
        let wire = wire_with("", &format!(",{tail}"), 5, 3, never);
        assert!(
            serde_json::from_str::<ResourcePool>(&wire).is_err(),
            "{tail} must be rejected"
        );
    }
    // Faults still fail when a valid refill is present.
    let over = wire_with("", ",\"refill\":2", 5, 6, never);
    assert!(serde_json::from_str::<ResourcePool>(&over).is_err());
    let zero_period = wire_with(
        "",
        ",\"refill\":2",
        5,
        5,
        "{\"type\":\"on_tick\",\"value\":0}",
    );
    assert!(serde_json::from_str::<ResourcePool>(&zero_period).is_err());
}

// R8 (Behaviour 5, H1-H3): property against a two-integer reference model.
#[derive(Debug, Clone)]
enum Op {
    Spend(u32),
    Refresh(RefreshEvent),
    SetMax(u32),
}

fn count() -> impl Strategy<Value = u32> {
    prop_oneof![0u32..=10, any::<u32>()]
}

fn trigger_strategy() -> impl Strategy<Value = RefreshTrigger> {
    prop_oneof![
        Just(RefreshTrigger::OnTurnStart),
        Just(RefreshTrigger::OnRoundStart),
        (1u128..=2).prop_map(|n| RefreshTrigger::OnRest(rest_id(n))),
        (1u64..=16).prop_map(RefreshTrigger::OnTick),
        Just(RefreshTrigger::Never),
    ]
}

fn event_strategy() -> impl Strategy<Value = RefreshEvent> {
    prop_oneof![
        Just(RefreshEvent::TurnStart),
        Just(RefreshEvent::RoundStart),
        (1u128..=2).prop_map(|n| RefreshEvent::Rest(rest_id(n))),
        (0u64..=64).prop_map(|t| RefreshEvent::Tick(Tick::new(t))),
    ]
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        count().prop_map(Op::Spend),
        event_strategy().prop_map(Op::Refresh),
        count().prop_map(Op::SetMax),
    ]
}

fn matches(trigger: RefreshTrigger, event: RefreshEvent) -> bool {
    match (trigger, event) {
        (RefreshTrigger::OnTurnStart, RefreshEvent::TurnStart)
        | (RefreshTrigger::OnRoundStart, RefreshEvent::RoundStart) => true,
        (RefreshTrigger::OnRest(a), RefreshEvent::Rest(b)) => a == b,
        (RefreshTrigger::OnTick(period), RefreshEvent::Tick(tick)) => {
            tick.get() > 0 && tick.get() % period == 0
        }
        _ => false,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn pool_matches_the_reference_model(
        max in count(),
        selector in any::<u32>(),
        refill in proptest::option::of((1u32..=u32::MAX).prop_map(nz)),
        trigger in trigger_strategy(),
        ops in proptest::collection::vec(op_strategy(), 0..=32),
    ) {
        let start = u32::try_from(u64::from(selector) % (u64::from(max) + 1)).unwrap();
        let mut pool =
            ResourcePool::with_refill(pool_id(7), max, start, trigger, refill).unwrap();
        let mut model_max = max;
        let mut model_current = start;
        for op in ops {
            match op {
                Op::Spend(amount) => {
                    let result = pool.spend(amount);
                    if amount > model_current {
                        prop_assert!(result.is_err());
                    } else {
                        prop_assert!(result.is_ok());
                        model_current -= amount;
                    }
                }
                Op::Refresh(event) => {
                    let changed = pool.refresh(event);
                    let mut expected = false;
                    if matches(trigger, event) {
                        let next = match refill {
                            None => model_max,
                            Some(n) => u32::try_from(
                                (u64::from(model_current) + u64::from(n.get()))
                                    .min(u64::from(model_max)),
                            )
                            .unwrap(),
                        };
                        expected = next != model_current;
                        model_current = next;
                    }
                    prop_assert_eq!(changed, expected);
                }
                Op::SetMax(new_max) => {
                    let changed = pool.set_max(new_max);
                    prop_assert_eq!(changed, new_max != model_max);
                    model_max = new_max;
                    model_current = model_current.min(new_max);
                }
            }
            prop_assert!(pool.current() <= pool.max());
            prop_assert_eq!(pool.current(), model_current);
            prop_assert_eq!(pool.max(), model_max);
            prop_assert_eq!(pool.refill(), refill);
            prop_assert_eq!(pool.refresh_trigger(), trigger);
            let wire = serde_json::to_string(&pool).unwrap();
            prop_assert_eq!(wire.contains("\"refill\""), refill.is_some());
            let decoded: ResourcePool = serde_json::from_str(&wire).unwrap();
            prop_assert_eq!(decoded, pool.clone());
        }
    }
}
