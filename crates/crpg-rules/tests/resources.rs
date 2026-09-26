//! Resource pools: construction bounds, spending, refresh matching, and wire shapes.
//!
//! Neutral vocabulary only. Tick arithmetic uses core accessors; no floats.

use crpg_core::{Tick, Ulid};
use crpg_rules::{
    RefreshEvent, RefreshTrigger, ResourcePool, ResourcePoolId, RestId, RulesErrorCode,
};

fn pool_id(n: u128) -> ResourcePoolId {
    ResourcePoolId(Ulid::from_u128(n))
}

fn rest_id(n: u128) -> RestId {
    RestId(Ulid::from_u128(n))
}

fn pool(max: u32, current: u32) -> ResourcePool {
    ResourcePool::new(pool_id(1), max, current, RefreshTrigger::Never).unwrap()
}

fn pool_with(max: u32, current: u32, refresh: RefreshTrigger) -> ResourcePool {
    ResourcePool::new(pool_id(1), max, current, refresh).unwrap()
}

#[test]
fn max_current_boundaries() {
    pool(5, 5);
    pool(0, 0);
    pool(u32::MAX, u32::MAX);
    let error = ResourcePool::new(pool_id(1), 5, 6, RefreshTrigger::Never).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidResource);
    assert_eq!(error.location, "/resource/current");
    let error = ResourcePool::new(pool_id(1), 0, 1, RefreshTrigger::Never).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidResource);
    assert_eq!(error.location, "/resource/current");
}

#[test]
fn zero_max_and_zero_spend() {
    let mut empty = pool(0, 0);
    assert!(empty.can_afford(0));
    assert!(!empty.can_afford(1));
    empty.spend(0).unwrap();
    assert_eq!(empty.current(), 0);
    let error = empty.spend(1).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InsufficientResource);
    assert_eq!(error.location, "/resource/amount");
}

#[test]
fn full_u32_spends_and_insufficient_funds_roll_back() {
    let mut full = pool(u32::MAX, u32::MAX);
    assert!(full.can_afford(u32::MAX));
    assert!(!pool(7, 7).can_afford(8));
    full.spend(u32::MAX).unwrap();
    assert_eq!(full.current(), 0);
    let seven = pool(7, 7);
    let mut partial = seven;
    partial.spend(3).unwrap();
    assert_eq!(partial.current(), 4);
    let before = partial.clone();
    let error = partial.spend(5).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InsufficientResource);
    assert_eq!(error.location, "/resource/amount");
    assert_eq!(partial, before);
    assert_eq!(partial.current(), 4);
}

#[test]
fn every_trigger_matches_only_its_event_kind() {
    let rest = rest_id(3);
    let other = rest_id(4);
    // Turn triggers match turn events only.
    let mut turn = pool_with(4, 1, RefreshTrigger::OnTurnStart);
    assert!(!turn.refresh(RefreshEvent::RoundStart));
    assert_eq!(turn.current(), 1);
    assert!(turn.refresh(RefreshEvent::TurnStart));
    assert_eq!(turn.current(), 4);
    // Round triggers match round events only.
    let mut round = pool_with(4, 1, RefreshTrigger::OnRoundStart);
    assert!(!round.refresh(RefreshEvent::TurnStart));
    assert!(round.refresh(RefreshEvent::RoundStart));
    // Rest triggers match their exact rest only.
    let mut rest_pool = pool_with(4, 1, RefreshTrigger::OnRest(rest));
    assert!(!rest_pool.refresh(RefreshEvent::Rest(other)));
    assert_eq!(rest_pool.current(), 1);
    assert!(rest_pool.refresh(RefreshEvent::Rest(rest)));
    assert_eq!(rest_pool.current(), 4);
    // Never matches nothing.
    let mut never = pool_with(4, 1, RefreshTrigger::Never);
    assert!(!never.refresh(RefreshEvent::TurnStart));
    assert!(!never.refresh(RefreshEvent::RoundStart));
    assert!(!never.refresh(RefreshEvent::Rest(rest)));
    assert!(!never.refresh(RefreshEvent::Tick(Tick::new(12))));
    assert_eq!(never.current(), 1);
}

#[test]
fn tick_triggers_match_positive_multiples_but_never_zero() {
    let mut pool = pool_with(9, 2, RefreshTrigger::OnTick(3));
    assert!(!pool.refresh(RefreshEvent::Tick(Tick::ZERO)));
    assert!(!pool.refresh(RefreshEvent::Tick(Tick::new(1))));
    assert!(!pool.refresh(RefreshEvent::Tick(Tick::new(2))));
    assert_eq!(pool.current(), 2);
    assert!(pool.refresh(RefreshEvent::Tick(Tick::new(3))));
    assert_eq!(pool.current(), 9);
    // A repeated multiple is idempotent until spending reopens a refill.
    assert!(!pool.refresh(RefreshEvent::Tick(Tick::new(6))));
    pool.spend(4).unwrap();
    assert!(pool.refresh(RefreshEvent::Tick(Tick::new(6))));
    // A period of one matches every positive tick but still not zero.
    let mut every = pool_with(9, 0, RefreshTrigger::OnTick(1));
    assert!(!every.refresh(RefreshEvent::Tick(Tick::ZERO)));
    assert!(every.refresh(RefreshEvent::Tick(Tick::new(1))));
    // Neighbours of a multiple do not match.
    let mut wide = pool_with(9, 0, RefreshTrigger::OnTick(u64::MAX));
    assert!(!wide.refresh(RefreshEvent::Tick(Tick::new(u64::MAX - 1))));
    assert!(wide.refresh(RefreshEvent::Tick(Tick::new(u64::MAX))));
}

#[test]
fn zero_tick_period_is_rejected() {
    let error = ResourcePool::new(pool_id(1), 5, 5, RefreshTrigger::OnTick(0)).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidResource);
    assert_eq!(error.location, "/resource/refresh");
}

#[test]
fn immediate_repeated_refresh_is_idempotent_until_spending() {
    let mut pool = pool_with(6, 2, RefreshTrigger::OnTurnStart);
    assert!(pool.refresh(RefreshEvent::TurnStart));
    assert!(!pool.refresh(RefreshEvent::TurnStart));
    assert_eq!(pool.current(), 6);
    pool.spend(1).unwrap();
    assert!(pool.refresh(RefreshEvent::TurnStart));
    assert_eq!(pool.current(), 6);
    assert!(!pool.refresh(RefreshEvent::TurnStart));
}

#[test]
fn pool_wire_shapes_round_trip() {
    let rest = rest_id(8);
    let cases = [
        (
            ResourcePool::new(pool_id(2), 5, 3, RefreshTrigger::OnTurnStart).unwrap(),
            format!(
                "{{\"id\":\"{}\",\"max\":5,\"current\":3,\"refresh\":{{\"type\":\"on_turn_start\"}}}}",
                Ulid::from_u128(2)
            ),
        ),
        (
            ResourcePool::new(pool_id(3), 5, 3, RefreshTrigger::OnRoundStart).unwrap(),
            format!(
                "{{\"id\":\"{}\",\"max\":5,\"current\":3,\"refresh\":{{\"type\":\"on_round_start\"}}}}",
                Ulid::from_u128(3)
            ),
        ),
        (
            ResourcePool::new(pool_id(4), 5, 3, RefreshTrigger::OnRest(rest)).unwrap(),
            format!(
                "{{\"id\":\"{}\",\"max\":5,\"current\":3,\"refresh\":{{\"type\":\"on_rest\",\"value\":\"{rest}\"}}}}",
                Ulid::from_u128(4),
                rest = Ulid::from_u128(8)
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
        assert_eq!(serde_json::to_string(pool).unwrap(), *wire);
        let loaded: ResourcePool = serde_json::from_str(wire).unwrap();
        assert_eq!(&loaded, pool);
        assert_eq!(loaded.id(), pool.id());
        assert_eq!(loaded.max(), pool.max());
        assert_eq!(loaded.current(), pool.current());
        assert_eq!(loaded.refresh_trigger(), pool.refresh_trigger());
    }
    // Refresh events round-trip with the same adjacent tagging.
    let event = RefreshEvent::Tick(Tick::new(9));
    let wire = serde_json::to_string(&event).unwrap();
    assert_eq!(wire, "{\"type\":\"tick\",\"value\":9}");
    assert_eq!(serde_json::from_str::<RefreshEvent>(&wire).unwrap(), event);
    let event = RefreshEvent::Rest(rest);
    let wire = serde_json::to_string(&event).unwrap();
    assert_eq!(
        wire,
        format!("{{\"type\":\"rest\",\"value\":\"{}\"}}", Ulid::from_u128(8))
    );
    assert_eq!(serde_json::from_str::<RefreshEvent>(&wire).unwrap(), event);
}

#[test]
fn invalid_pool_wire_is_rejected() {
    // Current above maximum fails construction on decode.
    let wire = format!(
        "{{\"id\":\"{}\",\"max\":5,\"current\":6,\"refresh\":{{\"type\":\"never\"}}}}",
        Ulid::from_u128(2)
    );
    assert!(serde_json::from_str::<ResourcePool>(&wire).is_err());
    // A zero tick period fails construction on decode.
    let wire = format!(
        "{{\"id\":\"{}\",\"max\":5,\"current\":5,\"refresh\":{{\"type\":\"on_tick\",\"value\":0}}}}",
        Ulid::from_u128(2)
    );
    assert!(serde_json::from_str::<ResourcePool>(&wire).is_err());
    // Unknown fields are rejected on pools, triggers, and events.
    let wire = format!(
        "{{\"id\":\"{}\",\"max\":5,\"current\":5,\"refresh\":{{\"type\":\"never\"}},\"extra\":0}}",
        Ulid::from_u128(2)
    );
    assert!(serde_json::from_str::<ResourcePool>(&wire).is_err());
    assert!(serde_json::from_str::<RefreshTrigger>("{\"type\":\"never\",\"extra\":0}").is_err());
    assert!(serde_json::from_str::<RefreshTrigger>("{\"type\":\"on_tick\"}").is_err());
    assert!(
        serde_json::from_str::<RefreshEvent>("{\"type\":\"tick\",\"value\":1,\"extra\":0}")
            .is_err()
    );
    assert!(serde_json::from_str::<RefreshTrigger>("{\"type\":\"on_pause\"}").is_err());
}
