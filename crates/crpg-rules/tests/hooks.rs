//! Lifecycle hooks: wire shapes, queue round trips, and direct handlers.

#[allow(dead_code)]
#[path = "common/mod.rs"]
mod common;

use crpg_core::{EventQueue, GenerationalArena, Tick, Ulid};
use crpg_rules::{HookHandler, HookMutation, KernelHook, RulesError, RulesErrorCode, StatValue};

use common::{fixture, uid};

fn arena_entity() -> (GenerationalArena<()>, crpg_core::EntityId) {
    let mut arena = GenerationalArena::new();
    let entity = arena.insert(());
    (arena, entity)
}

#[test]
fn all_five_wire_shapes_round_trip() {
    let (_arena, entity) = arena_entity();
    let entity_wire = format!(
        "{{\"index\":{},\"generation\":{}}}",
        entity.index(),
        entity.generation()
    );
    let cases: Vec<(KernelHook, String)> = vec![
        (
            KernelHook::OnDeath { entity },
            format!("{{\"type\":\"on_death\",\"entity\":{entity_wire}}}"),
        ),
        (
            KernelHook::OnTurnStart { entity },
            format!("{{\"type\":\"on_turn_start\",\"entity\":{entity_wire}}}"),
        ),
        (
            KernelHook::OnTurnEnd { entity },
            format!("{{\"type\":\"on_turn_end\",\"entity\":{entity_wire}}}"),
        ),
        (
            KernelHook::OnRoundStart { round: 7 },
            String::from("{\"type\":\"on_round_start\",\"round\":7}"),
        ),
        (
            KernelHook::OnEncounterStart { encounter: uid(9) },
            format!(
                "{{\"type\":\"on_encounter_start\",\"encounter\":\"{}\"}}",
                uid(9)
            ),
        ),
    ];
    for (hook, wire) in &cases {
        assert_eq!(serde_json::to_string(hook).unwrap(), *wire);
        let loaded: KernelHook = serde_json::from_str(wire).unwrap();
        assert_eq!(&loaded, hook);
    }
}

#[test]
fn round_zero_and_zero_ulids_are_valid() {
    let hook = KernelHook::OnRoundStart { round: 0 };
    let wire = serde_json::to_string(&hook).unwrap();
    assert_eq!(wire, "{\"type\":\"on_round_start\",\"round\":0}");
    assert_eq!(serde_json::from_str::<KernelHook>(&wire).unwrap(), hook);
    let hook = KernelHook::OnEncounterStart {
        encounter: Ulid::NIL,
    };
    let loaded: KernelHook = serde_json::from_str(&serde_json::to_string(&hook).unwrap()).unwrap();
    assert_eq!(loaded, hook);
}

#[test]
fn unknown_hook_fields_and_types_are_rejected() {
    let (_arena, entity) = arena_entity();
    let hook = KernelHook::OnDeath { entity };
    let mut value = serde_json::to_value(hook).unwrap();
    value["extra"] = serde_json::Value::from(0);
    assert!(serde_json::from_value::<KernelHook>(value).is_err());
    assert!(serde_json::from_str::<KernelHook>("{\"type\":\"before_roll\"}").is_err());
    assert!(serde_json::from_str::<KernelHook>("{\"type\":\"on_death\"}").is_err());
}

#[test]
fn event_queue_round_trip_preserves_hook_order() {
    let (_arena, entity) = arena_entity();
    let mut queue = EventQueue::new();
    queue.push(Tick::new(2), KernelHook::OnDeath { entity });
    queue.push(Tick::new(1), KernelHook::OnRoundStart { round: 3 });
    queue.push(
        Tick::new(1),
        KernelHook::OnEncounterStart { encounter: uid(4) },
    );
    let json = serde_json::to_string(&queue).unwrap();
    let mut loaded: EventQueue<KernelHook> = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded, queue);
    let drained = loaded.drain();
    assert_eq!(drained.len(), 3);
    // Ascending (tick, seq): the two tick-1 hooks keep push order first.
    assert_eq!(drained[0].payload, KernelHook::OnRoundStart { round: 3 });
    assert_eq!(
        drained[1].payload,
        KernelHook::OnEncounterStart { encounter: uid(4) }
    );
    assert_eq!(drained[2].payload, KernelHook::OnDeath { entity });
    assert!(loaded.is_empty());
}

fn propose_nothing(_hook: &KernelHook) -> Result<Vec<HookMutation>, RulesError> {
    Ok(vec![])
}

fn propose_ordered(hook: &KernelHook) -> Result<Vec<HookMutation>, RulesError> {
    match *hook {
        KernelHook::OnDeath { entity } => {
            let fx = fixture();
            Ok(vec![
                HookMutation::RemoveSource {
                    entity,
                    source: crpg_rules::SourceRef {
                        kind: String::from("sk"),
                        id: uid(1),
                    },
                },
                HookMutation::SetBaseStat {
                    entity,
                    stat: fx.stats[0],
                    value: StatValue::Int(0),
                },
            ])
        }
        _ => Ok(vec![]),
    }
}

fn propose_failure(_hook: &KernelHook) -> Result<Vec<HookMutation>, RulesError> {
    Err(RulesError {
        code: RulesErrorCode::MissingBase,
        location: String::from("/stats/0"),
        cycle: vec![],
    })
}

#[test]
fn direct_handlers_return_ordered_proposals_or_errors() {
    let (_arena, entity) = arena_entity();
    let hook = KernelHook::OnDeath { entity };
    let nothing: HookHandler = propose_nothing;
    assert_eq!(nothing(&hook).unwrap(), vec![]);
    // The caller's block is untouched: handlers propose, never mutate.
    let fx = fixture();
    let stored = common::block(&[(fx.stats[0], StatValue::Int(5))]);
    let before = stored.clone();
    let ordered: HookHandler = propose_ordered;
    let proposals = ordered(&hook).unwrap();
    assert_eq!(proposals.len(), 2);
    assert!(matches!(proposals[0], HookMutation::RemoveSource { .. }));
    assert!(matches!(proposals[1], HookMutation::SetBaseStat { .. }));
    assert_eq!(stored, before);
    let failing: HookHandler = propose_failure;
    let error = failing(&hook).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::MissingBase);
}

#[test]
fn handler_results_do_not_touch_a_supplied_block() {
    // A caller-owned block passes through handler invocation unchanged even
    // when the handler proposes a base update for the same stat.
    let fx = fixture();
    let stored = common::block(&[(fx.stats[0], StatValue::Int(5))]);
    let before = stored.clone();
    let (_arena, entity) = arena_entity();
    let ordered: HookHandler = propose_ordered;
    let proposals = ordered(&KernelHook::OnDeath { entity }).unwrap();
    assert_eq!(proposals.len(), 2);
    assert_eq!(stored, before);
    assert_eq!(stored.get(fx.stats[0]), Some(&StatValue::Int(5)));
}

#[test]
fn resolution_hook_variants_round_trip_with_core_closed_fields() {
    let (_arena, entity) = arena_entity();
    let (_target_arena, target) = arena_entity();
    let entity_wire = format!(
        "{{\"index\":{},\"generation\":{}}}",
        entity.index(),
        entity.generation()
    );
    let target_wire = format!(
        "{{\"index\":{},\"generation\":{}}}",
        target.index(),
        target.generation()
    );
    let resolution = uid(21);
    let cases: Vec<(KernelHook, String)> = vec![
        (
            KernelHook::BeforeRoll {
                actor: entity,
                target: Some(target),
                resolution,
            },
            format!(
                "{{\"type\":\"before_roll\",\"actor\":{entity_wire},\"target\":{target_wire},\"resolution\":\"{resolution}\"}}"
            ),
        ),
        (
            KernelHook::AfterRoll {
                actor: entity,
                target: None,
                resolution,
                total: -4,
            },
            format!(
                "{{\"type\":\"after_roll\",\"actor\":{entity_wire},\"target\":null,\"resolution\":\"{resolution}\",\"total\":-4}}"
            ),
        ),
        (
            KernelHook::BeforeDamage {
                source: entity,
                target,
                resolution,
                amount: 9,
            },
            format!(
                "{{\"type\":\"before_damage\",\"source\":{entity_wire},\"target\":{target_wire},\"resolution\":\"{resolution}\",\"amount\":9}}"
            ),
        ),
        (
            KernelHook::AfterDamage {
                source: entity,
                target,
                resolution,
                amount: 0,
            },
            format!(
                "{{\"type\":\"after_damage\",\"source\":{entity_wire},\"target\":{target_wire},\"resolution\":\"{resolution}\",\"amount\":0}}"
            ),
        ),
    ];
    for (hook, wire) in &cases {
        assert_eq!(serde_json::to_string(hook).unwrap(), *wire);
        let loaded: KernelHook = serde_json::from_str(wire).unwrap();
        assert_eq!(&loaded, hook);
    }
    // Payloads stay core-closed: unknown fields and unknown types fail, and
    // a missing correlation identity fails with it.
    let (first_hook, _) = &cases[0];
    let (second_hook, _) = &cases[1];
    let (third_hook, _) = &cases[2];
    let mut value = serde_json::to_value(first_hook).unwrap();
    value["extra"] = serde_json::Value::from(0);
    assert!(serde_json::from_value::<KernelHook>(value).is_err());
    assert!(serde_json::from_str::<KernelHook>("{\"type\":\"before_damage\"}").is_err());
    assert!(serde_json::from_str::<KernelHook>("{\"type\":\"after_damage\"}").is_err());
    // Resolution hooks ride the generic queue like the lifecycle hooks.
    let mut queue = EventQueue::new();
    queue.push(Tick::new(1), *third_hook);
    queue.push(Tick::new(1), *second_hook);
    let json = serde_json::to_string(&queue).unwrap();
    let mut loaded: EventQueue<KernelHook> = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded, queue);
    let drained = loaded.drain();
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].payload, *third_hook);
    assert_eq!(drained[1].payload, *second_hook);
}
