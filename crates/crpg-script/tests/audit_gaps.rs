//! Audit-gap tests for crpg-script (T124).
//!
//! Each test guards a contract path the crate audit flagged as untested.
//! Helpers are copied from tests/action_bindings.rs; that file is untouched.

use std::cell::Cell;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_data::{
    Ability, ActionBundleIdentity, ActionCall, ActionParameter, ActionSignature,
    ActionSignatureStore, Creature, DataValue, Effect, Encounter,
    OutcomeTable as AuthoredOutcomeTable, Placement, Ruleset, ValueType, MAX_ACTION_SIGNATURES,
};
use crpg_script::{
    ActionBinding, ActionBindings, ActionHandler, BindingError, HandlerError, InvocationContext,
};
use crpg_sim::{
    perform_action, start_encounter, state_hash, CombatAction, CombatError, EncounterSpec,
    EntityMeta, PlacementAndArea, World,
};
use serde_json::json;

// ---------------------------------------------------------------------------
// Instrumentation (thread-local invocation counter)
// ---------------------------------------------------------------------------

thread_local! {
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

fn record() {
    CALLS.with(|calls| calls.set(calls.get() + 1));
}

fn calls() -> usize {
    CALLS.with(Cell::get)
}

// ---------------------------------------------------------------------------
// Combat fixture (copied, trimmed to what the tests need)
// ---------------------------------------------------------------------------

const ATTACK: u128 = 103;
const JAB: u128 = 201;
const SLAY: u128 = 202;

fn uid(value: u128) -> String {
    Ulid::from_u128(value).to_string()
}

fn fx_raw(value: i32) -> i32 {
    Fx16_16::from_int(value).to_raw()
}

struct Fixture {
    encounter: Encounter,
    ruleset: Ruleset,
    abilities: std::collections::BTreeMap<Ulid, Ability>,
    tables: std::collections::BTreeMap<Ulid, AuthoredOutcomeTable>,
    effects: std::collections::BTreeMap<Ulid, Effect>,
    placements: std::collections::BTreeMap<Ulid, (Placement, Ulid)>,
    creatures: std::collections::BTreeMap<Ulid, Creature>,
}

fn ability_doc(id: u128, attribute: &str, success: u32, failure: u32, ends_turn: bool) -> Ability {
    serde_json::from_value(json!({
        "id": uid(id),
        "slug": format!("probe-ability-{id}"),
        "name": format!("probe.ability-{id}"),
        "dice": "2d6",
        "attribute": attribute,
        "outcome_table": uid(104),
        "damage": [
            {"outcome": {"type": "success"}, "amount": success},
            {"outcome": {"type": "failure"}, "amount": failure}
        ],
        "cost": 1,
        "extra_costs": [],
        "ends_turn": ends_turn,
        "defense": {"type": "actor_attribute"},
        "requires_target": true,
        "allow_self_target": false
    }))
    .expect("ability")
}

fn placement_doc(id: u128, prefab: u128) -> Placement {
    serde_json::from_value(json!({
        "id": uid(id),
        "slug": format!("probe-placement-{id}"),
        "name": format!("probe.placement-{id}"),
        "prefab": uid(prefab),
        "transform": {"position": [0,0,0], "rotation": [0,0,0], "scale": [65536,65536,65536]},
        "overrides": {}
    }))
    .expect("placement")
}

fn creature_doc(id: u128, health: i32) -> Creature {
    serde_json::from_value(json!({
        "id": uid(id),
        "slug": format!("probe-creature-{id}"),
        "name": format!("probe.creature-{id}"),
        "stats": {"alpha": fx_raw(100), "beta": fx_raw(6), "gamma": fx_raw(1), "health": fx_raw(health)},
        "tags": [], "faction": null, "inventory": []
    }))
    .expect("creature")
}

impl Fixture {
    fn new() -> Self {
        let ruleset: Ruleset = serde_json::from_value(json!({
            "id": uid(102), "slug": "probe-ruleset", "name": "probe.ruleset",
            "package": "probe-pack", "version": "0.1.0",
            "stats": [
                {"name": "alpha", "kind": "int"},
                {"name": "beta", "kind": "int"},
                {"name": "gamma", "kind": "int"},
                {"name": "health", "kind": "int"}
            ],
            "health_stat": "health", "attributes": ["alpha","beta","gamma"],
            "pools": [{"id": uid(105), "max": 40, "refresh": {"type": "on_turn_start"}}],
            "abilities": [uid(ATTACK), uid(JAB), uid(SLAY)]
        }))
        .expect("ruleset");
        let table: AuthoredOutcomeTable = serde_json::from_value(json!({
            "id": uid(104), "slug": "probe-table", "name": "probe.table",
            "bands": [
                {"min_margin": i64::MIN, "outcome": {"type": "success"}},
                {"min_margin": 1, "outcome": {"type": "failure"}}
            ],
            "natural_rules": []
        }))
        .expect("table");
        let encounter: Encounter = serde_json::from_value(json!({
            "id": uid(101), "slug": "probe-encounter", "name": "probe.encounter",
            "ruleset": uid(102),
            "participants": [
                {"placement": uid(111), "initiative": 10},
                {"placement": uid(112), "initiative": 9}
            ]
        }))
        .expect("encounter");
        let mut abilities = std::collections::BTreeMap::new();
        for ability in [
            ability_doc(ATTACK, "alpha", 2, 0, true),
            ability_doc(201, "alpha", 0, 0, false),
            ability_doc(SLAY, "alpha", 100, 100, false),
        ] {
            abilities.insert(ability.id, ability);
        }
        let mut tables = std::collections::BTreeMap::new();
        tables.insert(Ulid::from_u128(104), table);
        let area = Ulid::from_u128(131);
        let mut placements = std::collections::BTreeMap::new();
        placements.insert(Ulid::from_u128(111), (placement_doc(111, 121), area));
        placements.insert(Ulid::from_u128(112), (placement_doc(112, 122), area));
        let mut creatures = std::collections::BTreeMap::new();
        creatures.insert(Ulid::from_u128(121), creature_doc(121, 10));
        creatures.insert(Ulid::from_u128(122), creature_doc(122, 6));
        Self {
            encounter,
            ruleset,
            abilities,
            tables,
            effects: std::collections::BTreeMap::new(),
            placements,
            creatures,
        }
    }

    fn start_in(&self, world: &mut World) -> (EntityId, EntityId) {
        let abilities: std::collections::BTreeMap<Ulid, &Ability> =
            self.abilities.iter().map(|(id, doc)| (*id, doc)).collect();
        let tables: std::collections::BTreeMap<Ulid, &AuthoredOutcomeTable> =
            self.tables.iter().map(|(id, doc)| (*id, doc)).collect();
        let placements: std::collections::BTreeMap<Ulid, PlacementAndArea<'_>> = self
            .placements
            .iter()
            .map(|(id, (placement, area))| {
                (
                    *id,
                    PlacementAndArea {
                        placement,
                        area: *area,
                    },
                )
            })
            .collect();
        let creatures: std::collections::BTreeMap<Ulid, &Creature> =
            self.creatures.iter().map(|(id, doc)| (*id, doc)).collect();
        let effects: std::collections::BTreeMap<Ulid, &Effect> =
            self.effects.iter().map(|(id, doc)| (*id, doc)).collect();
        let spec = EncounterSpec {
            encounter: &self.encounter,
            ruleset: &self.ruleset,
            abilities: &abilities,
            outcome_tables: &tables,
            placements: &placements,
            creatures: &creatures,
            effects: &effects,
        };
        let before: Vec<EntityId> = world.combatants().iter().map(|(id, _)| id).collect();
        start_encounter(world, &spec).expect("start");
        let fresh: Vec<EntityId> = world
            .combatants()
            .iter()
            .map(|(id, _)| id)
            .filter(|id| !before.contains(id))
            .collect();
        let a = fresh
            .iter()
            .copied()
            .find(|id| world.combatants().get(*id).expect("c").initiative() == 10)
            .expect("A");
        let b = fresh
            .iter()
            .copied()
            .find(|id| world.combatants().get(*id).expect("c").initiative() == 9)
            .expect("B");
        (a, b)
    }

    fn started(seed: u64) -> (World, EntityId, EntityId) {
        let mut world = World::new(seed);
        let (a, b) = Self::new().start_in(&mut world);
        (world, a, b)
    }
}

// ---------------------------------------------------------------------------
// State helpers
// ---------------------------------------------------------------------------

fn snapshot(world: &World) -> (Vec<u8>, [u8; 32]) {
    (serde_json::to_vec(world).expect("ser"), state_hash(world))
}

fn assert_unchanged(world: &World, before: &(Vec<u8>, [u8; 32]), ctx: &str) {
    let after = snapshot(world);
    assert_eq!(before.0, after.0, "bytes changed: {ctx}");
    assert_eq!(before.1, after.1, "hash changed: {ctx}");
}

fn opponent(context: &InvocationContext<'_>) -> Result<EntityId, HandlerError> {
    context
        .world()
        .combatants()
        .iter()
        .filter(|(id, combatant)| *id != context.actor() && !combatant.dead())
        .map(|(id, _)| id)
        .min()
        .ok_or(HandlerError::Refused)
}

// ---------------------------------------------------------------------------
// Binding helpers
// ---------------------------------------------------------------------------

fn signature(action_id: &str, parameters: Vec<ActionParameter>) -> ActionSignature {
    ActionSignature {
        action_id: action_id.to_owned(),
        parameters,
    }
}

fn parameter(name: &str, value_type: ValueType, required: bool) -> ActionParameter {
    ActionParameter {
        name: name.to_owned(),
        value_type,
        required,
    }
}

fn store_of(entries: &[(ActionSignature, ActionHandler)]) -> ActionSignatureStore {
    ActionSignatureStore::new(
        Ulid::from_u128(900),
        entries.iter().map(|(s, _)| s.clone()).collect(),
    )
    .expect("store")
}

fn bind(entries: &[(ActionSignature, ActionHandler)]) -> Vec<ActionBinding> {
    entries
        .iter()
        .map(|(s, h)| ActionBinding {
            signature: s.clone(),
            handler: *h,
        })
        .collect()
}

fn build(
    store: &ActionSignatureStore,
    entries: &[(ActionSignature, ActionHandler)],
) -> Result<ActionBindings, BindingError> {
    ActionBindings::new(store.clone(), store.identity().clone(), bind(entries))
}

fn standard() -> (ActionBindings, ActionBundleIdentity) {
    // Reuse a tiny table matching the audit's needs: combo-like via handlers
    // defined below. We instead build from the shared table() helper.
    let entries = table();
    let store = store_of(&entries);
    let identity = store.identity().clone();
    let bindings = build(&store, &entries).expect("standard");
    (bindings, identity)
}

fn table() -> Vec<(ActionSignature, ActionHandler)> {
    vec![
        (
            signature(
                "combo",
                vec![
                    parameter("first", ValueType::ObjectRef, true),
                    parameter("second", ValueType::ObjectRef, true),
                ],
            ),
            combo_handler as ActionHandler,
        ),
        (signature("hijack", Vec::new()), hijack_handler),
        (signature("pass", Vec::new()), pass_handler),
        (signature("refuse", Vec::new()), refuse_handler),
        (
            signature(
                "strike",
                vec![parameter("ability", ValueType::ObjectRef, true)],
            ),
            strike_handler,
        ),
        (signature("wait", Vec::new()), wait_handler),
        (
            signature(
                "flurry",
                vec![
                    parameter("ability", ValueType::ObjectRef, true),
                    parameter("count", ValueType::Unsigned, true),
                ],
            ),
            flurry_handler,
        ),
    ]
}

fn call(action_id: &str, args: Vec<(&str, DataValue)>) -> ActionCall {
    ActionCall {
        action_id: action_id.to_owned(),
        args: args.into_iter().map(|(n, v)| (n.to_owned(), v)).collect(),
    }
}

fn ability_ref(value: u128) -> DataValue {
    DataValue::ObjectRef(Ulid::from_u128(value))
}

fn flurry_call(ability: u128, count: u64) -> ActionCall {
    call(
        "flurry",
        vec![
            ("ability", ability_ref(ability)),
            ("count", DataValue::Unsigned(count)),
        ],
    )
}

fn strike_call(ability: u128) -> ActionCall {
    call("strike", vec![("ability", ability_ref(ability))])
}

// handlers
fn combo_handler(
    context: &InvocationContext<'_>,
    call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    let target = opponent(context)?;
    let first = match call.args.get("first") {
        Some(DataValue::ObjectRef(id)) => *id,
        _ => return Err(HandlerError::Refused),
    };
    let second = match call.args.get("second") {
        Some(DataValue::ObjectRef(id)) => *id,
        _ => return Err(HandlerError::Refused),
    };
    Ok(vec![
        CombatAction::UseAbility {
            actor: context.actor(),
            ability: first,
            target,
        },
        CombatAction::UseAbility {
            actor: context.actor(),
            ability: second,
            target,
        },
    ])
}

fn hijack_handler(
    context: &InvocationContext<'_>,
    _call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    let foreign = opponent(context)?;
    Ok(vec![
        CombatAction::EndTurn {
            actor: context.actor(),
        },
        CombatAction::EndTurn { actor: foreign },
    ])
}

fn pass_handler(
    context: &InvocationContext<'_>,
    _call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    Ok(vec![CombatAction::EndTurn {
        actor: context.actor(),
    }])
}

fn refuse_handler(
    _context: &InvocationContext<'_>,
    _call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    Err(HandlerError::Refused)
}

fn wait_handler(
    _context: &InvocationContext<'_>,
    _call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    Err(HandlerError::UnsupportedWait)
}

fn strike_handler(
    context: &InvocationContext<'_>,
    call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    let ability = match call.args.get("ability") {
        Some(DataValue::ObjectRef(id)) => *id,
        _ => return Err(HandlerError::Refused),
    };
    let target = opponent(context)?;
    Ok(vec![CombatAction::UseAbility {
        actor: context.actor(),
        ability,
        target,
    }])
}

fn flurry_handler(
    context: &InvocationContext<'_>,
    call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    let ability = match call.args.get("ability") {
        Some(DataValue::ObjectRef(id)) => *id,
        _ => return Err(HandlerError::Refused),
    };
    let count = match call.args.get("count") {
        Some(DataValue::Unsigned(c)) if *c <= 64 => *c,
        _ => return Err(HandlerError::Refused),
    };
    let target = opponent(context)?;
    Ok((0..count)
        .map(|_| CombatAction::UseAbility {
            actor: context.actor(),
            ability,
            target,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Handlers specific to audit gaps
// ---------------------------------------------------------------------------

fn many_with_foreign(
    context: &InvocationContext<'_>,
    _call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    let foreign = opponent(context)?;
    let mut proposals = Vec::with_capacity(33);
    for _ in 0..32 {
        proposals.push(CombatAction::EndTurn {
            actor: context.actor(),
        });
    }
    proposals.push(CombatAction::EndTurn { actor: foreign });
    Ok(proposals)
}

fn slay_then_attack(
    context: &InvocationContext<'_>,
    _call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    let target = opponent(context)?;
    Ok(vec![
        CombatAction::UseAbility {
            actor: context.actor(),
            ability: Ulid::from_u128(SLAY),
            target,
        },
        CombatAction::UseAbility {
            actor: context.actor(),
            ability: Ulid::from_u128(ATTACK),
            target,
        },
    ])
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// #56, #59: more than 32 proposals that also include a foreign actor report
// ProposalLimit first (pinned order: bound before substitute).
#[test]
fn proposal_limit_precedes_foreign_actor() {
    let entries = vec![(
        signature("many", Vec::new()),
        many_with_foreign as ActionHandler,
    )];
    let store = store_of(&entries);
    let identity = store.identity().clone();
    let bindings = build(&store, &entries).expect("binds");
    let (mut world, _a, b) = Fixture::started(5);
    let before = snapshot(&world);
    let start = calls();
    let error = bindings
        .dispatch(&mut world, b, &identity, &call("many", Vec::new()))
        .expect_err("33 incl foreign");
    assert_eq!(error, BindingError::ProposalLimit);
    assert_eq!(calls(), start + 1);
    assert_unchanged(&world, &before, "proposal limit");
}

// #57: Call validation precedes every actor-error kind.
#[test]
fn call_precedes_every_actor_error_kind() {
    let store = ActionSignatureStore::new(Ulid::from_u128(900), Vec::new()).expect("empty store");
    let identity = store.identity().clone();
    let bindings = ActionBindings::new(store.clone(), identity.clone(), Vec::new()).expect("empty");
    let forged = call("forged", Vec::new());

    // NoEncounter
    {
        let mut idle = World::new(4);
        let loner = idle.spawn(EntityMeta {});
        let before = snapshot(&idle);
        let error = bindings
            .dispatch(&mut idle, loner, &identity, &forged)
            .expect_err("no enc");
        assert!(matches!(error, BindingError::Call(_)));
        assert_unchanged(&idle, &before, "no encounter");
    }

    // Started world cases.
    let fixture = Fixture::new();
    let mut world = World::new(4);
    let outsider = world.spawn(EntityMeta {});
    let stale = world.spawn(EntityMeta {});
    world.despawn(stale);
    let (a, b) = fixture.start_in(&mut world);

    // NotParticipant
    {
        let before = snapshot(&world);
        let error = bindings
            .dispatch(&mut world, outsider, &identity, &forged)
            .expect_err("outsider");
        assert!(matches!(error, BindingError::Call(_)));
        assert_unchanged(&world, &before, "outsider");
    }
    // AbsentActor
    {
        let before = snapshot(&world);
        let error = bindings
            .dispatch(&mut world, stale, &identity, &forged)
            .expect_err("stale");
        assert!(matches!(error, BindingError::Call(_)));
        assert_unchanged(&world, &before, "stale");
    }
    // OutOfTurn
    {
        let before = snapshot(&world);
        let error = bindings
            .dispatch(&mut world, a, &identity, &forged)
            .expect_err("out of turn");
        assert!(matches!(error, BindingError::Call(_)));
        assert_unchanged(&world, &before, "out of turn");
    }
    // DeadActor
    {
        let mut killed = world.clone();
        perform_action(
            &mut killed,
            &CombatAction::UseAbility {
                actor: b,
                ability: Ulid::from_u128(SLAY),
                target: a,
            },
        )
        .expect("slay");
        let before = snapshot(&killed);
        let error = bindings
            .dispatch(&mut killed, a, &identity, &forged)
            .expect_err("dead");
        assert!(matches!(error, BindingError::Call(_)));
        assert_unchanged(&killed, &before, "dead");
    }
}

// #60: Apply-rejection path runs the handler exactly once.
#[test]
fn apply_rejection_runs_handler_once() {
    let entries = vec![(
        signature("combo2", Vec::new()),
        slay_then_attack as ActionHandler,
    )];
    let store = store_of(&entries);
    let identity = store.identity().clone();
    let bindings = build(&store, &entries).expect("binds");
    let (mut world, a, b) = Fixture::started(17);
    let before = snapshot(&world);
    let start = calls();
    let error = bindings
        .dispatch(&mut world, b, &identity, &call("combo2", Vec::new()))
        .expect_err("apply rejects");
    assert_eq!(
        error,
        BindingError::Apply {
            index: 1,
            error: CombatError::DeadTarget { target: a },
        }
    );
    assert_eq!(calls(), start + 1);
    assert_unchanged(&world, &before, "apply rejection");
}

// #62: binding count bound precedes unknown ids.
#[test]
fn binding_count_precedes_unknown() {
    let store = ActionSignatureStore::new(Ulid::from_u128(900), Vec::new()).expect("empty store");
    let mut entries = Vec::new();
    for i in 0..(MAX_ACTION_SIGNATURES + 1) {
        entries.push((
            signature(&format!("a{i:04}"), Vec::new()),
            pass_handler as ActionHandler,
        ));
    }
    let error = build(&store, &entries).expect_err("too many");
    assert_eq!(error, BindingError::TooManyBindings);
}

// #62: binding count bound precedes missing ids.
#[test]
fn binding_count_precedes_missing() {
    let declared = vec![
        (signature("pass", Vec::new()), pass_handler as ActionHandler),
        (
            signature("refuse", Vec::new()),
            refuse_handler as ActionHandler,
        ),
    ];
    let store = store_of(&declared);
    let mut entries = Vec::new();
    for _ in 0..(MAX_ACTION_SIGNATURES + 1) {
        entries.push(declared[0].clone());
    }
    let error = build(&store, &entries).expect_err("too many");
    assert_eq!(error, BindingError::TooManyBindings);
}

// #101: each rejection in the retry sequence leaves the world unchanged.
#[test]
fn retry_rejections_leave_world_unchanged() {
    let (bindings, identity) = standard();
    let (mut world, _a, b) = Fixture::started(29);
    let rejected = [
        call(
            "combo",
            vec![
                ("first", ability_ref(SLAY)),
                ("second", ability_ref(ATTACK)),
            ],
        ),
        call("hijack", Vec::new()),
        call("wait", Vec::new()),
        flurry_call(JAB, 33),
        call("forged", Vec::new()),
        strike_call(999),
    ];
    for rejected_call in &rejected {
        let before = snapshot(&world);
        bindings
            .dispatch(&mut world, b, &identity, rejected_call)
            .expect_err("rejected");
        assert_unchanged(&world, &before, "rejection");
    }
}

// #102: empty proposals assert the handler ran.
#[test]
fn empty_proposals_asserts_handler_ran() {
    let (bindings, identity) = standard();
    let (mut world, _a, b) = Fixture::started(23);
    let before = snapshot(&world);
    let start = calls();
    let outcomes = bindings
        .dispatch(&mut world, b, &identity, &flurry_call(JAB, 0))
        .expect("empty");
    assert!(outcomes.is_empty());
    assert_eq!(calls(), start + 1);
    assert_unchanged(&world, &before, "empty");
}
