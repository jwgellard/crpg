//! Public acceptance for trusted action bindings (T029b, D17).
//!
//! Every assertion goes through public APIs: real T029a declarations
//! (`crpg-data`), real combat started and checked through the public
//! `crpg-sim` World/controller surface, and hand-authored trusted handlers.
//! Every rejection is paired with a valid positive control and checked
//! against complete-World bytes plus `state_hash`. Expected simulation
//! results come from an independent oracle: `perform_action` applied
//! directly to a public clone of the same world.
//!
//! Handlers here bump a thread-local invocation counter. That is test
//! instrumentation (each test runs on its own thread), not a pattern for
//! production handlers, whose reviewed contract forbids globals.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::error::Error as _;
use std::panic::{catch_unwind, AssertUnwindSafe};

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_data::{
    Ability, ActionBundleIdentity, ActionCall, ActionParameter, ActionSignature,
    ActionSignatureStore, Creature, DataValue, Effect, Encounter,
    OutcomeTable as AuthoredOutcomeTable, Placement, Ruleset, SignatureErrorCode, SignatureLimit,
    ValueType, MAX_ACTION_SIGNATURES,
};
use crpg_script::{
    ActionBinding, ActionBindings, ActionHandler, BindingError, HandlerError, InvocationContext,
    MAX_BINDING_PROPOSALS,
};
use crpg_sim::{
    perform_action, start_encounter, state_hash, validate_action, ActionOutcome, CombatAction,
    CombatError, EncounterSpec, EntityMeta, PlacementAndArea, SimEvent, World,
};
use serde_json::json;

// ---------------------------------------------------------------------------
// Instrumentation
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
// Combat fixture: neutral authored documents parsed through production shapes
// ---------------------------------------------------------------------------

const ATTACK: u128 = 103;
const JAB: u128 = 201;
const SLAY: u128 = 202;
const WHIFF: u128 = 203;

fn uid(value: u128) -> String {
    Ulid::from_u128(value).to_string()
}

fn fx_raw(value: i32) -> i32 {
    Fx16_16::from_int(value).to_raw()
}

/// Authored documents for a two-combatant fight.
///
/// Creature A (placement 111, initiative 10, health 10) and creature B
/// (placement 112, initiative 9, health 6); B acts first. Every `alpha` is
/// 100, so `alpha` rolls always succeed; every `gamma` is 1, so `gamma` rolls
/// always fail. One action pool (max 40, refreshed on turn start).
///
/// - 103 attack: `alpha`, damage 2/0, cost 1, ends the turn.
/// - 201 jab: `alpha`, damage 0/0, cost 1, does not end the turn.
/// - 202 slay: `alpha`, damage 100/100, cost 1, does not end the turn.
/// - 203 whiff: `gamma`, damage 2/0, cost 1, ends the turn.
struct Fixture {
    encounter: Encounter,
    ruleset: Ruleset,
    abilities: BTreeMap<Ulid, Ability>,
    tables: BTreeMap<Ulid, AuthoredOutcomeTable>,
    effects: BTreeMap<Ulid, Effect>,
    placements: BTreeMap<Ulid, (Placement, Ulid)>,
    creatures: BTreeMap<Ulid, Creature>,
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
    .expect("the fixture ability is valid")
}

fn placement_doc(id: u128, prefab: u128) -> Placement {
    serde_json::from_value(json!({
        "id": uid(id),
        "slug": format!("probe-placement-{id}"),
        "name": format!("probe.placement-{id}"),
        "prefab": uid(prefab),
        "transform": {
            "position": [0, 0, 0],
            "rotation": [0, 0, 0],
            "scale": [65536, 65536, 65536]
        },
        "overrides": {}
    }))
    .expect("the fixture placement is valid")
}

fn creature_doc(id: u128, health: i32) -> Creature {
    serde_json::from_value(json!({
        "id": uid(id),
        "slug": format!("probe-creature-{id}"),
        "name": format!("probe.creature-{id}"),
        "stats": {
            "alpha": fx_raw(100),
            "beta": fx_raw(6),
            "gamma": fx_raw(1),
            "health": fx_raw(health)
        },
        "tags": [],
        "faction": null,
        "inventory": []
    }))
    .expect("the fixture creature is valid")
}

impl Fixture {
    fn new() -> Self {
        let ruleset: Ruleset = serde_json::from_value(json!({
            "id": uid(102),
            "slug": "probe-ruleset",
            "name": "probe.ruleset",
            "package": "probe-pack",
            "version": "0.1.0",
            "stats": [
                {"name": "alpha", "kind": "int"},
                {"name": "beta", "kind": "int"},
                {"name": "gamma", "kind": "int"},
                {"name": "health", "kind": "int"}
            ],
            "health_stat": "health",
            "attributes": ["alpha", "beta", "gamma"],
            "pools": [
                {"id": uid(105), "max": 40, "refresh": {"type": "on_turn_start"}}
            ],
            "abilities": [uid(ATTACK), uid(JAB), uid(SLAY), uid(WHIFF)]
        }))
        .expect("the fixture ruleset is valid");
        let table: AuthoredOutcomeTable = serde_json::from_value(json!({
            "id": uid(104),
            "slug": "probe-table",
            "name": "probe.table",
            "bands": [
                {"min_margin": i64::MIN, "outcome": {"type": "success"}},
                {"min_margin": 1, "outcome": {"type": "failure"}}
            ],
            "natural_rules": []
        }))
        .expect("the fixture table is valid");
        let encounter: Encounter = serde_json::from_value(json!({
            "id": uid(101),
            "slug": "probe-encounter",
            "name": "probe.encounter",
            "ruleset": uid(102),
            "participants": [
                {"placement": uid(111), "initiative": 10},
                {"placement": uid(112), "initiative": 9}
            ]
        }))
        .expect("the fixture encounter is valid");

        let mut abilities = BTreeMap::new();
        for ability in [
            ability_doc(ATTACK, "alpha", 2, 0, true),
            ability_doc(JAB, "alpha", 0, 0, false),
            ability_doc(SLAY, "alpha", 100, 100, false),
            ability_doc(WHIFF, "gamma", 2, 0, true),
        ] {
            abilities.insert(ability.id, ability);
        }
        let mut tables = BTreeMap::new();
        tables.insert(Ulid::from_u128(104), table);
        let area = Ulid::from_u128(131);
        let mut placements = BTreeMap::new();
        placements.insert(Ulid::from_u128(111), (placement_doc(111, 121), area));
        placements.insert(Ulid::from_u128(112), (placement_doc(112, 122), area));
        let mut creatures = BTreeMap::new();
        creatures.insert(Ulid::from_u128(121), creature_doc(121, 10));
        creatures.insert(Ulid::from_u128(122), creature_doc(122, 6));
        Self {
            encounter,
            ruleset,
            abilities,
            tables,
            effects: BTreeMap::new(),
            placements,
            creatures,
        }
    }

    /// Starts the encounter in `world`, returning `(a, b)`.
    fn start_in(&self, world: &mut World) -> (EntityId, EntityId) {
        let abilities: BTreeMap<Ulid, &Ability> =
            self.abilities.iter().map(|(id, doc)| (*id, doc)).collect();
        let tables: BTreeMap<Ulid, &AuthoredOutcomeTable> =
            self.tables.iter().map(|(id, doc)| (*id, doc)).collect();
        let placements: BTreeMap<Ulid, PlacementAndArea<'_>> = self
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
        let creatures: BTreeMap<Ulid, &Creature> =
            self.creatures.iter().map(|(id, doc)| (*id, doc)).collect();
        let effects: BTreeMap<Ulid, &Effect> =
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
        start_encounter(world, &spec).expect("the fixture starts");
        let fresh: Vec<EntityId> = world
            .combatants()
            .iter()
            .map(|(id, _)| id)
            .filter(|id| !before.contains(id))
            .collect();
        assert_eq!(fresh.len(), 2);
        let a = fresh
            .iter()
            .copied()
            .find(|id| world.combatants().get(*id).expect("combatant").initiative() == 10)
            .expect("A");
        let b = fresh
            .iter()
            .copied()
            .find(|id| world.combatants().get(*id).expect("combatant").initiative() == 9)
            .expect("B");
        (a, b)
    }

    /// A fresh started world, returning `(world, a, b)`.
    fn started(seed: u64) -> (World, EntityId, EntityId) {
        let mut world = World::new(seed);
        let (a, b) = Self::new().start_in(&mut world);
        (world, a, b)
    }
}

fn active(world: &World) -> EntityId {
    world
        .combat()
        .expect("combat")
        .active
        .expect("an active turn")
}

// ---------------------------------------------------------------------------
// Complete-state helpers and the independent oracle
// ---------------------------------------------------------------------------

/// Complete-World bytes plus the state hash.
fn snapshot(world: &World) -> (Vec<u8>, [u8; 32]) {
    (
        serde_json::to_vec(world).expect("the world serializes"),
        state_hash(world),
    )
}

fn assert_unchanged(world: &World, before: &(Vec<u8>, [u8; 32]), context: &str) {
    let after = snapshot(world);
    assert_eq!(before.0, after.0, "world bytes changed: {context}");
    assert_eq!(before.1, after.1, "state hash changed: {context}");
}

/// Oracle: applies `actions` directly to a clone, in order.
fn oracle(
    world: &World,
    actions: &[CombatAction],
) -> (World, Vec<Result<Option<ActionOutcome>, CombatError>>) {
    let mut probe = world.clone();
    let results = actions
        .iter()
        .map(|action| perform_action(&mut probe, action))
        .collect();
    (probe, results)
}

/// Drains a clone's event queue and returns its `Died` entities in order.
fn deaths(world: &World) -> Vec<EntityId> {
    let mut probe = world.clone();
    probe
        .events_mut()
        .drain()
        .into_iter()
        .filter_map(|envelope| match envelope.payload {
            SimEvent::Died { entity } => Some(entity),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Declarations and trusted handlers
// ---------------------------------------------------------------------------

const BUNDLE: u128 = 900;

fn parameter(name: &str, value_type: ValueType, required: bool) -> ActionParameter {
    ActionParameter {
        name: name.to_owned(),
        value_type,
        required,
    }
}

fn signature(action_id: &str, parameters: Vec<ActionParameter>) -> ActionSignature {
    ActionSignature {
        action_id: action_id.to_owned(),
        parameters,
    }
}

/// The trusted handler table, in declaration-lexical order.
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
            combo as ActionHandler,
        ),
        (
            signature(
                "flurry",
                vec![
                    parameter("ability", ValueType::ObjectRef, true),
                    parameter("count", ValueType::Unsigned, true),
                ],
            ),
            flurry,
        ),
        (signature("hijack", Vec::new()), hijack),
        (signature("pass", Vec::new()), pass),
        (signature("refuse", Vec::new()), refuse),
        (
            signature(
                "strike",
                vec![parameter("ability", ValueType::ObjectRef, true)],
            ),
            strike,
        ),
        (
            signature(
                "tripwire",
                vec![
                    parameter("label", ValueType::Text, true),
                    parameter("note", ValueType::Text, false),
                ],
            ),
            tripwire,
        ),
        (signature("wait", Vec::new()), wait),
    ]
}

fn store_of(table: &[(ActionSignature, ActionHandler)]) -> ActionSignatureStore {
    ActionSignatureStore::new(
        Ulid::from_u128(BUNDLE),
        table
            .iter()
            .map(|(signature, _)| signature.clone())
            .collect(),
    )
    .expect("the declarations are valid")
}

fn bind(table: &[(ActionSignature, ActionHandler)]) -> Vec<ActionBinding> {
    table
        .iter()
        .map(|(signature, handler)| ActionBinding {
            signature: signature.clone(),
            handler: *handler,
        })
        .collect()
}

/// Builds the trusted table from `entries` registered in the given order.
fn build(
    store: &ActionSignatureStore,
    entries: &[(ActionSignature, ActionHandler)],
) -> Result<ActionBindings, BindingError> {
    ActionBindings::new(store.clone(), store.identity().clone(), bind(entries))
}

/// The standard bindings, registered in declaration order.
fn standard() -> (ActionBindings, ActionBundleIdentity) {
    let entries = table();
    let store = store_of(&entries);
    let identity = store.identity().clone();
    let bindings = build(&store, &entries).expect("the standard table binds");
    (bindings, identity)
}

fn call(action_id: &str, args: Vec<(&str, DataValue)>) -> ActionCall {
    ActionCall {
        action_id: action_id.to_owned(),
        args: args
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    }
}

fn ability_ref(value: u128) -> DataValue {
    DataValue::ObjectRef(Ulid::from_u128(value))
}

fn strike_call(ability: u128) -> ActionCall {
    call("strike", vec![("ability", ability_ref(ability))])
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

fn tripwire_call() -> ActionCall {
    call("tripwire", vec![("label", DataValue::Text("probe".into()))])
}

fn object_arg(call: &ActionCall, name: &str) -> Result<Ulid, HandlerError> {
    match call.args.get(name) {
        Some(DataValue::ObjectRef(id)) => Ok(*id),
        _ => Err(HandlerError::Refused),
    }
}

/// The lowest-id living combatant other than the actor.
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

fn strike(
    context: &InvocationContext<'_>,
    call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    Ok(vec![CombatAction::UseAbility {
        actor: context.actor(),
        ability: object_arg(call, "ability")?,
        target: opponent(context)?,
    }])
}

fn flurry(
    context: &InvocationContext<'_>,
    call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    let ability = object_arg(call, "ability")?;
    let count = match call.args.get("count") {
        Some(DataValue::Unsigned(count)) if *count <= 64 => *count,
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

fn combo(
    context: &InvocationContext<'_>,
    call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    let target = opponent(context)?;
    Ok(vec![
        CombatAction::UseAbility {
            actor: context.actor(),
            ability: object_arg(call, "first")?,
            target,
        },
        CombatAction::UseAbility {
            actor: context.actor(),
            ability: object_arg(call, "second")?,
            target,
        },
    ])
}

/// A legal own `EndTurn`, then an attempt to act as the opponent.
fn hijack(
    context: &InvocationContext<'_>,
    _call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    Ok(vec![
        CombatAction::EndTurn {
            actor: context.actor(),
        },
        CombatAction::EndTurn {
            actor: opponent(context)?,
        },
    ])
}

fn pass(
    context: &InvocationContext<'_>,
    _call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    Ok(vec![CombatAction::EndTurn {
        actor: context.actor(),
    }])
}

fn refuse(
    _context: &InvocationContext<'_>,
    _call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    Err(HandlerError::Refused)
}

fn wait(
    _context: &InvocationContext<'_>,
    _call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    Err(HandlerError::UnsupportedWait)
}

/// Must never run when validation is supposed to stop first.
fn tripwire(
    _context: &InvocationContext<'_>,
    _call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    panic!("tripwire handler invoked");
}

/// Every permutation of `0..n` (Heap's algorithm).
fn permutations(n: usize) -> Vec<Vec<usize>> {
    let mut items: Vec<usize> = (0..n).collect();
    let mut out = vec![items.clone()];
    let mut counters = vec![0usize; n];
    let mut i = 0;
    while i < n {
        if counters[i] < i {
            if i % 2 == 0 {
                items.swap(0, i);
            } else {
                items.swap(counters[i], i);
            }
            out.push(items.clone());
            counters[i] += 1;
            i = 0;
        } else {
            counters[i] = 0;
            i += 1;
        }
    }
    out
}

/// Asserts that every registration order of `entries` yields `expected`.
fn assert_every_order(
    store: &ActionSignatureStore,
    entries: &[(ActionSignature, ActionHandler)],
    expected: &BindingError,
) {
    let orders = permutations(entries.len());
    let mut factorial = 1;
    for k in 2..=entries.len() {
        factorial *= k;
    }
    assert_eq!(orders.len(), factorial);
    for order in orders {
        let shuffled: Vec<(ActionSignature, ActionHandler)> =
            order.iter().map(|&index| entries[index].clone()).collect();
        let error = build(store, &shuffled).expect_err("the table is rejected");
        assert_eq!(&error, expected, "registration order {order:?}");
    }
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

#[test]
fn exact_startup_succeeds_and_registration_order_changes_nothing() {
    let entries = table();
    let store = store_of(&entries);
    let identity = store.identity().clone();
    let (base, a, b) = Fixture::started(5);
    let mut reference: Option<(Vec<Option<ActionOutcome>>, Vec<u8>)> = None;
    let n = entries.len();
    let mut orders: Vec<Vec<usize>> = (0..n)
        .map(|shift| (0..n).map(|i| (i + shift) % n).collect())
        .collect();
    orders.push((0..n).rev().collect());
    orders.push(vec![5, 2, 7, 0, 3, 6, 1, 4]);
    for order in orders {
        let shuffled: Vec<(ActionSignature, ActionHandler)> =
            order.iter().map(|&index| entries[index].clone()).collect();
        let bindings = build(&store, &shuffled).expect("every order binds");
        assert_eq!(bindings.declarations(), &store);
        for (signature, _) in &entries {
            assert!(bindings.contains(&signature.action_id));
        }
        let mut world = base.clone();
        let outcomes = bindings
            .dispatch(&mut world, b, &identity, &strike_call(ATTACK))
            .expect("the strike dispatches");
        let bytes = snapshot(&world).0;
        match &reference {
            None => reference = Some((outcomes, bytes)),
            Some((outcomes0, bytes0)) => {
                assert_eq!(&outcomes, outcomes0, "order {order:?}");
                assert_eq!(&bytes, bytes0, "order {order:?}");
            }
        }
    }
    let (_, bytes) = reference.expect("at least one order ran");
    let (expected, _) = oracle(
        &base,
        &[CombatAction::UseAbility {
            actor: b,
            ability: Ulid::from_u128(ATTACK),
            target: a,
        }],
    );
    assert_eq!(bytes, snapshot(&expected).0);
}

#[test]
fn empty_declarations_with_empty_bindings_are_valid() {
    let store =
        ActionSignatureStore::new(Ulid::from_u128(BUNDLE), Vec::new()).expect("empty store");
    let identity = store.identity().clone();
    let bindings =
        ActionBindings::new(store.clone(), identity.clone(), Vec::new()).expect("empty binds");
    assert!(bindings.declarations().is_empty());
    assert!(!bindings.contains("pass"));
    let (mut world, _, b) = Fixture::started(3);
    let before = snapshot(&world);
    let error = bindings
        .dispatch(&mut world, b, &identity, &call("pass", Vec::new()))
        .expect_err("nothing is bound");
    assert!(matches!(
        &error,
        BindingError::Call(inner) if inner.code == SignatureErrorCode::UnknownAction
    ));
    assert_unchanged(&world, &before, "empty table");

    // An extra binding against empty declarations is unknown, not ignored.
    let error = ActionBindings::new(
        store,
        identity,
        bind(&[(signature("pass", Vec::new()), pass as ActionHandler)]),
    )
    .expect_err("undeclared binding");
    assert_eq!(
        error,
        BindingError::UnknownBinding {
            action_id: "pass".into()
        }
    );
}

#[test]
fn identity_mismatch_is_checked_first() {
    let entries = table();
    let store = store_of(&entries);
    // Same declarations, different bundle ULID.
    let other_bundle = ActionSignatureStore::new(
        Ulid::from_u128(BUNDLE + 1),
        entries.iter().map(|(s, _)| s.clone()).collect(),
    )
    .expect("store");
    // Same bundle ULID, different declarations: a different revision.
    let other_revision =
        ActionSignatureStore::new(Ulid::from_u128(BUNDLE), vec![signature("pass", Vec::new())])
            .expect("store");
    assert_eq!(
        other_revision.identity().bundle,
        store.identity().bundle,
        "only the revision differs"
    );
    for wrong in [other_bundle.identity(), other_revision.identity()] {
        // Even with every later failure present, identity wins.
        let mut hostile = bind(&entries);
        hostile.push(ActionBinding {
            signature: signature("pass", Vec::new()),
            handler: pass,
        });
        hostile.push(ActionBinding {
            signature: signature("zzz", Vec::new()),
            handler: pass,
        });
        let error = ActionBindings::new(store.clone(), wrong.clone(), hostile)
            .expect_err("identity mismatch");
        assert_eq!(error, BindingError::BundleMismatch);
    }
    // Positive control: the exact identity binds.
    build(&store, &entries).expect("the exact identity binds");
}

#[test]
fn binding_count_is_bounded_before_duplicates() {
    let entries: Vec<(ActionSignature, ActionHandler)> = (0..MAX_ACTION_SIGNATURES)
        .map(|index| {
            (
                signature(&format!("a{index:04}"), Vec::new()),
                pass as ActionHandler,
            )
        })
        .collect();
    let store = store_of(&entries);
    // Exactly the maximum binds.
    let bindings = build(&store, &entries).expect("1024 bindings bind");
    assert!(bindings.contains("a0000") && bindings.contains("a1023"));
    // One more — a duplicate — fails on the count, not the duplicate.
    let mut over = entries.clone();
    over.push(entries[0].clone());
    assert_eq!(over.len(), MAX_ACTION_SIGNATURES + 1);
    let error = build(&store, &over).expect_err("1025 bindings");
    assert_eq!(error, BindingError::TooManyBindings);
}

#[test]
fn duplicate_unknown_missing_precedence_is_lexical_in_every_order() {
    let declared = vec![
        (signature("pass", Vec::new()), pass as ActionHandler),
        (signature("refuse", Vec::new()), refuse as ActionHandler),
        (signature("wait", Vec::new()), wait as ActionHandler),
    ];
    let store = store_of(&declared);
    let undeclared = |id: &str| (signature(id, Vec::new()), pass as ActionHandler);

    // Two duplicated ids plus an unknown id: the lexically first duplicate.
    let entries = vec![
        declared[2].clone(),
        declared[0].clone(),
        declared[1].clone(),
        declared[2].clone(),
        declared[0].clone(),
        undeclared("aaa"),
    ];
    assert_every_order(
        &store,
        &entries,
        &BindingError::DuplicateBinding {
            action_id: "pass".into(),
        },
    );
    // A duplicate never silently overwrites, even with identical handlers.
    let entries = vec![
        declared[0].clone(),
        declared[1].clone(),
        declared[2].clone(),
        declared[2].clone(),
    ];
    assert_every_order(
        &store,
        &entries,
        &BindingError::DuplicateBinding {
            action_id: "wait".into(),
        },
    );

    // Two unknown ids plus a missing one: the lexically first unknown.
    let entries = vec![
        declared[0].clone(),
        declared[1].clone(),
        undeclared("zz"),
        undeclared("ab"),
    ];
    assert_every_order(
        &store,
        &entries,
        &BindingError::UnknownBinding {
            action_id: "ab".into(),
        },
    );

    // Two missing ids plus a mismatched one: the lexically first missing.
    let entries = vec![(
        signature("wait", vec![parameter("x", ValueType::Bool, true)]),
        wait as ActionHandler,
    )];
    assert_every_order(
        &store,
        &entries,
        &BindingError::MissingBinding {
            action_id: "pass".into(),
        },
    );
    let entries = vec![declared[0].clone(), declared[2].clone()];
    assert_every_order(
        &store,
        &entries,
        &BindingError::MissingBinding {
            action_id: "refuse".into(),
        },
    );

    // Positive control: the exact set binds in every order.
    for order in permutations(declared.len()) {
        let shuffled: Vec<_> = order.iter().map(|&i| declared[i].clone()).collect();
        build(&store, &shuffled).expect("the exact set binds");
    }
}

#[test]
fn signature_mismatch_covers_order_name_type_and_requirement() {
    let entries = table();
    let store = store_of(&entries);
    let index_of = |id: &str| {
        entries
            .iter()
            .position(|(s, _)| s.action_id == id)
            .expect("declared")
    };
    let with = |id: &str, parameters: Vec<ActionParameter>| {
        let mut changed = entries.clone();
        changed[index_of(id)].0.parameters = parameters;
        changed
    };

    // Parameter order alone is a mismatch.
    let swapped = with(
        "flurry",
        vec![
            parameter("count", ValueType::Unsigned, true),
            parameter("ability", ValueType::ObjectRef, true),
        ],
    );
    let expected = BindingError::SignatureMismatch {
        action_id: "flurry".into(),
    };
    assert_eq!(build(&store, &swapped).expect_err("order"), expected);
    let mut reversed = swapped.clone();
    reversed.reverse();
    assert_eq!(build(&store, &reversed).expect_err("order"), expected);

    // Name, category, required flag, extra and absent parameters.
    let variants = [
        vec![
            parameter("ability", ValueType::ObjectRef, true),
            parameter("total", ValueType::Unsigned, true),
        ],
        vec![
            parameter("ability", ValueType::ObjectRef, true),
            parameter("count", ValueType::Integer, true),
        ],
        vec![
            parameter("ability", ValueType::ObjectRef, true),
            parameter("count", ValueType::Unsigned, false),
        ],
        vec![
            parameter("ability", ValueType::ObjectRef, true),
            parameter("count", ValueType::Unsigned, true),
            parameter("extra", ValueType::Bool, false),
        ],
        vec![parameter("ability", ValueType::ObjectRef, true)],
    ];
    for parameters in variants {
        let changed = with("flurry", parameters);
        assert_eq!(build(&store, &changed).expect_err("mismatch"), expected);
    }

    // Two mismatches: the lexically first declaration wins in any order.
    let mut both = with(
        "flurry",
        vec![
            parameter("count", ValueType::Unsigned, true),
            parameter("ability", ValueType::ObjectRef, true),
        ],
    );
    both[index_of("combo")].0.parameters[1].required = false;
    let expected = BindingError::SignatureMismatch {
        action_id: "combo".into(),
    };
    assert_eq!(build(&store, &both).expect_err("two"), expected);
    both.reverse();
    assert_eq!(build(&store, &both).expect_err("two"), expected);
    both.rotate_left(3);
    assert_eq!(build(&store, &both).expect_err("two"), expected);

    // Positive control.
    build(&store, &entries).expect("the exact signatures bind");
}

// ---------------------------------------------------------------------------
// Dispatch: validation before the handler
// ---------------------------------------------------------------------------

#[test]
fn every_call_validation_failure_precedes_the_handler() {
    let (bindings, identity) = standard();
    let mut world = World::new(9);
    // A stale invocation actor too: call validation still comes first.
    let stale = world.spawn(EntityMeta {});
    world.despawn(stale);
    let (_, b) = Fixture::new().start_in(&mut world);
    let before = snapshot(&world);

    let other = ActionSignatureStore::new(
        Ulid::from_u128(BUNDLE + 7),
        vec![signature("tripwire", Vec::new())],
    )
    .expect("store");

    let mut deep = DataValue::Integer(0);
    for _ in 0..32 {
        deep = DataValue::List(vec![deep]);
    }
    let wide = DataValue::List((0..4096).map(DataValue::Integer).collect());
    let long_text = DataValue::Text("x".repeat(4097));
    let bulky = DataValue::List((0..17).map(|_| DataValue::Text("y".repeat(4000))).collect());
    let many_args: Vec<(String, DataValue)> = (0..33)
        .map(|index| (format!("arg{index:02}"), DataValue::Bool(true)))
        .collect();

    let cases: Vec<(&str, ActionBundleIdentity, ActionCall, SignatureErrorCode)> = vec![
        (
            "bundle mismatch",
            other.identity().clone(),
            tripwire_call(),
            SignatureErrorCode::BundleMismatch,
        ),
        (
            "empty action id",
            identity.clone(),
            call("", Vec::new()),
            SignatureErrorCode::InvalidIdentifier,
        ),
        (
            "empty argument name",
            identity.clone(),
            call(
                "tripwire",
                vec![
                    ("label", DataValue::Text("x".into())),
                    ("", DataValue::Bool(true)),
                ],
            ),
            SignatureErrorCode::InvalidIdentifier,
        ),
        (
            "too many arguments",
            identity.clone(),
            ActionCall {
                action_id: "tripwire".into(),
                args: many_args.into_iter().collect(),
            },
            SignatureErrorCode::LimitExceeded {
                kind: SignatureLimit::Arguments,
                limit: 32,
            },
        ),
        (
            "value depth",
            identity.clone(),
            call("tripwire", vec![("label", deep)]),
            SignatureErrorCode::LimitExceeded {
                kind: SignatureLimit::ValueDepth,
                limit: 32,
            },
        ),
        (
            "value nodes",
            identity.clone(),
            call("tripwire", vec![("label", wide)]),
            SignatureErrorCode::LimitExceeded {
                kind: SignatureLimit::ValueNodes,
                limit: 4096,
            },
        ),
        (
            "string bytes",
            identity.clone(),
            call("tripwire", vec![("label", long_text)]),
            SignatureErrorCode::LimitExceeded {
                kind: SignatureLimit::ValueStringBytes,
                limit: 4096,
            },
        ),
        (
            "call bytes",
            identity.clone(),
            call("tripwire", vec![("label", bulky)]),
            SignatureErrorCode::LimitExceeded {
                kind: SignatureLimit::CallBytes,
                limit: 65_536,
            },
        ),
        (
            "unknown action",
            identity.clone(),
            call("forged", Vec::new()),
            SignatureErrorCode::UnknownAction,
        ),
        (
            "extra argument",
            identity.clone(),
            call(
                "tripwire",
                vec![
                    ("label", DataValue::Text("x".into())),
                    ("bonus", DataValue::Bool(true)),
                ],
            ),
            SignatureErrorCode::ExtraArgument,
        ),
        (
            "missing argument",
            identity.clone(),
            call("tripwire", vec![("note", DataValue::Text("x".into()))]),
            SignatureErrorCode::MissingArgument,
        ),
        (
            "type mismatch",
            identity.clone(),
            call("tripwire", vec![("label", DataValue::Integer(1))]),
            SignatureErrorCode::TypeMismatch {
                expected: ValueType::Text,
                actual: ValueType::Integer,
            },
        ),
    ];
    for (name, case_identity, case_call, code) in cases {
        for actor in [b, stale] {
            let error = bindings
                .dispatch(&mut world, actor, &case_identity, &case_call)
                .expect_err(name);
            let expected = bindings
                .declarations()
                .validate_call(&case_identity, &case_call)
                .expect_err(name);
            assert_eq!(expected.code, code, "{name}");
            assert_eq!(error, BindingError::Call(expected), "{name}");
            assert_unchanged(&world, &before, name);
        }
    }

    // Positive control: the valid call does reach the tripwire.
    let reached = catch_unwind(AssertUnwindSafe(|| {
        let _ = bindings.dispatch(&mut world, b, &identity, &tripwire_call());
    }));
    assert!(reached.is_err(), "a valid call reaches its handler");
    assert_unchanged(&world, &before, "a panicking handler applies nothing");
}

#[test]
fn actor_errors_precede_the_handler_with_a_valid_control() {
    let (bindings, identity) = standard();
    let fixture = Fixture::new();

    // No encounter at all.
    let mut idle = World::new(4);
    let loner = idle.spawn(EntityMeta {});
    let before = snapshot(&idle);
    let error = bindings
        .dispatch(&mut idle, loner, &identity, &tripwire_call())
        .expect_err("no encounter");
    assert_eq!(error, BindingError::Actor(CombatError::NoEncounter));
    assert_unchanged(&idle, &before, "no encounter");

    // A live non-participant, a stale id, and an out-of-turn combatant.
    let mut world = World::new(4);
    let outsider = world.spawn(EntityMeta {});
    let stale = world.spawn(EntityMeta {});
    world.despawn(stale);
    let (a, b) = fixture.start_in(&mut world);
    assert_eq!(active(&world), b);
    let cases = [
        (outsider, CombatError::NotParticipant { entity: outsider }),
        (stale, CombatError::AbsentActor { actor: stale }),
        (
            a,
            CombatError::OutOfTurn {
                actor: a,
                active: Some(b),
            },
        ),
    ];
    let before = snapshot(&world);
    for (actor, expected) in &cases {
        let error = bindings
            .dispatch(&mut world, *actor, &identity, &tripwire_call())
            .expect_err("actor rejected");
        assert_eq!(error, BindingError::Actor(expected.clone()));
        assert_eq!(
            validate_action(&world, &CombatAction::EndTurn { actor: *actor }),
            Err(expected.clone()),
            "the T028 path agrees"
        );
        assert_unchanged(&world, &before, "actor rejected");
    }

    // A dead combatant.
    let mut killed = world.clone();
    perform_action(
        &mut killed,
        &CombatAction::UseAbility {
            actor: b,
            ability: Ulid::from_u128(SLAY),
            target: a,
        },
    )
    .expect("the slay is legal");
    let before_dead = snapshot(&killed);
    let error = bindings
        .dispatch(&mut killed, a, &identity, &tripwire_call())
        .expect_err("dead actor");
    assert_eq!(
        error,
        BindingError::Actor(CombatError::DeadActor { actor: a })
    );
    assert_unchanged(&killed, &before_dead, "dead actor");

    // Valid actor control: the handler runs exactly once and applies.
    let start = calls();
    let outcomes = bindings
        .dispatch(&mut world, b, &identity, &call("pass", Vec::new()))
        .expect("the active actor passes");
    assert_eq!(outcomes, vec![None]);
    assert_eq!(calls(), start + 1);
    assert_eq!(active(&world), a);
}

// ---------------------------------------------------------------------------
// Dispatch: accepted proposals
// ---------------------------------------------------------------------------

#[test]
fn accepted_attack_failed_attack_and_end_turn_match_the_controller() {
    let (bindings, identity) = standard();
    let (mut world, a, b) = Fixture::started(21);

    // One accepted, successful attack by B onto A.
    let attack = CombatAction::UseAbility {
        actor: b,
        ability: Ulid::from_u128(ATTACK),
        target: a,
    };
    let (expected_world, expected) = oracle(&world, &[attack]);
    let start = calls();
    let outcomes = bindings
        .dispatch(&mut world, b, &identity, &strike_call(ATTACK))
        .expect("the attack dispatches");
    assert_eq!(calls(), start + 1, "the handler ran exactly once");
    assert_eq!(outcomes, vec![expected[0].clone().expect("legal")]);
    let outcome = outcomes[0].as_ref().expect("an attack outcome");
    assert_eq!((outcome.actor, outcome.target), (b, a));
    assert_eq!(outcome.damage, 2);
    assert!(outcome.margin <= 0, "alpha 100 always succeeds");
    assert_eq!(world.combatants().get(a).expect("A").health(), 8);
    assert_eq!(snapshot(&world), snapshot(&expected_world));
    assert_eq!(active(&world), a, "the attack ended B's turn");

    // An accepted, legal attack whose roll fails: consumed, no damage.
    let whiff = CombatAction::UseAbility {
        actor: a,
        ability: Ulid::from_u128(WHIFF),
        target: b,
    };
    let before = snapshot(&world);
    let (expected_world, expected) = oracle(&world, &[whiff]);
    let outcomes = bindings
        .dispatch(&mut world, a, &identity, &strike_call(WHIFF))
        .expect("the failed attack is still accepted");
    assert_eq!(outcomes, vec![expected[0].clone().expect("legal")]);
    let outcome = outcomes[0].as_ref().expect("an attack outcome");
    assert_eq!(outcome.damage, 0);
    assert!(outcome.margin > 0, "gamma 1 always fails");
    assert_eq!(world.combatants().get(b).expect("B").health(), 6);
    assert_eq!(snapshot(&world), snapshot(&expected_world));
    assert_ne!(
        before,
        snapshot(&world),
        "the failed roll consumed RNG/turn"
    );
    assert_eq!(active(&world), b);

    // An explicit EndTurn.
    let (expected_world, expected) = oracle(&world, &[CombatAction::EndTurn { actor: b }]);
    let outcomes = bindings
        .dispatch(&mut world, b, &identity, &call("pass", Vec::new()))
        .expect("EndTurn dispatches");
    assert_eq!(outcomes, vec![None]);
    assert_eq!(expected, vec![Ok(None)]);
    assert_eq!(snapshot(&world), snapshot(&expected_world));
    assert_eq!(active(&world), a);
}

#[test]
fn thirty_two_proposals_apply_thirty_three_are_refused_and_empty_succeeds() {
    let (bindings, identity) = standard();
    let (base, a, b) = Fixture::started(13);
    assert_eq!(MAX_BINDING_PROPOSALS, 32);
    let jab = CombatAction::UseAbility {
        actor: b,
        ability: Ulid::from_u128(JAB),
        target: a,
    };

    // 33 legal non-ending proposals: refused by count before staging.
    let mut world = base.clone();
    let before = snapshot(&world);
    let (_, oracle_33) = oracle(&world, &[jab; 33]);
    assert!(
        oracle_33.iter().all(Result::is_ok),
        "each of 33 would be legal on its own"
    );
    let start = calls();
    let error = bindings
        .dispatch(&mut world, b, &identity, &flurry_call(JAB, 33))
        .expect_err("33 proposals");
    assert_eq!(error, BindingError::ProposalLimit);
    assert_eq!(calls(), start + 1, "the handler ran; nothing applied");
    assert_unchanged(&world, &before, "33 proposals");

    // Exactly 32: all applied sequentially, matching the controller.
    let (expected_world, expected) = oracle(&world, &[jab; 32]);
    let outcomes = bindings
        .dispatch(&mut world, b, &identity, &flurry_call(JAB, 32))
        .expect("32 proposals");
    assert_eq!(outcomes.len(), 32);
    let expected: Vec<Option<ActionOutcome>> = expected
        .into_iter()
        .map(|result| result.expect("legal"))
        .collect();
    assert_eq!(outcomes, expected);
    assert!(outcomes.iter().all(Option::is_some));
    assert_eq!(snapshot(&world), snapshot(&expected_world));
    assert_eq!(active(&world), b, "jabs never end the turn");
    let pool = world.combatants().get(b).expect("B").action_pool().clone();
    let probe = base.combatants().get(b).expect("B").action_pool().clone();
    assert_ne!(pool, probe, "32 costs were spent");

    // Empty proposals: success, nothing changes.
    let before = snapshot(&world);
    let outcomes = bindings
        .dispatch(&mut world, b, &identity, &flurry_call(JAB, 0))
        .expect("empty proposals");
    assert!(outcomes.is_empty());
    assert_unchanged(&world, &before, "empty proposals");
}

// ---------------------------------------------------------------------------
// Dispatch: rejection and atomicity
// ---------------------------------------------------------------------------

#[test]
fn a_rejected_second_proposal_rolls_back_the_whole_invocation() {
    let (bindings, identity) = standard();
    let (mut world, a, b) = Fixture::started(17);
    let before = snapshot(&world);
    let slay = CombatAction::UseAbility {
        actor: b,
        ability: Ulid::from_u128(SLAY),
        target: a,
    };
    let attack = CombatAction::UseAbility {
        actor: b,
        ability: Ulid::from_u128(ATTACK),
        target: a,
    };

    // The oracle shows what staging did before the rejection: the first
    // proposal drew RNG, killed A and queued its death.
    let (staged, results) = oracle(&world, &[slay, attack]);
    let first = results[0]
        .clone()
        .expect("the slay is legal")
        .expect("attack");
    assert!(first.target_died);
    assert_eq!(deaths(&staged), vec![a]);
    assert!(deaths(&world).is_empty());
    assert_ne!(
        snapshot(&staged),
        before,
        "staging changed RNG/events/health"
    );
    assert_eq!(results[1], Err(CombatError::DeadTarget { target: a }));

    let error = bindings
        .dispatch(
            &mut world,
            b,
            &identity,
            &call(
                "combo",
                vec![
                    ("first", ability_ref(SLAY)),
                    ("second", ability_ref(ATTACK)),
                ],
            ),
        )
        .expect_err("the second proposal fails");
    assert_eq!(
        error,
        BindingError::Apply {
            index: 1,
            error: CombatError::DeadTarget { target: a },
        }
    );
    assert_unchanged(&world, &before, "staged slay discarded");
    assert!(deaths(&world).is_empty(), "no Died escaped");
    assert_eq!(world.combatants().get(a).expect("A").health(), 10);

    // Positive control: a combo whose second step stays legal applies both.
    let jab = CombatAction::UseAbility {
        actor: b,
        ability: Ulid::from_u128(JAB),
        target: a,
    };
    let (expected_world, expected) = oracle(&world, &[jab, attack]);
    let outcomes = bindings
        .dispatch(
            &mut world,
            b,
            &identity,
            &call(
                "combo",
                vec![("first", ability_ref(JAB)), ("second", ability_ref(ATTACK))],
            ),
        )
        .expect("both proposals apply");
    let expected: Vec<Option<ActionOutcome>> =
        expected.into_iter().map(|r| r.expect("legal")).collect();
    assert_eq!(outcomes, expected);
    assert_eq!(snapshot(&world), snapshot(&expected_world));
}

#[test]
fn actor_substitution_is_rejected_before_staging() {
    let (bindings, identity) = standard();
    let (mut world, _, b) = Fixture::started(19);
    let before = snapshot(&world);
    let start = calls();
    let error = bindings
        .dispatch(&mut world, b, &identity, &call("hijack", Vec::new()))
        .expect_err("foreign actor");
    assert_eq!(error, BindingError::ForeignActor { index: 1 });
    assert_eq!(calls(), start + 1);
    // Index 0 (B's own legal EndTurn) was not applied either.
    assert_unchanged(&world, &before, "foreign actor");
    assert_eq!(active(&world), b);
}

#[test]
fn waiting_and_refusal_are_errors_never_success() {
    let (bindings, identity) = standard();
    let (mut world, _, b) = Fixture::started(23);
    let before = snapshot(&world);
    for (action, expected) in [
        ("wait", HandlerError::UnsupportedWait),
        ("refuse", HandlerError::Refused),
    ] {
        let start = calls();
        let error = bindings
            .dispatch(&mut world, b, &identity, &call(action, Vec::new()))
            .expect_err(action);
        assert_eq!(error, BindingError::Handler(expected));
        assert_eq!(calls(), start + 1);
        assert_unchanged(&world, &before, action);
    }
}

#[test]
fn retry_after_rejection_equals_a_fresh_invocation() {
    let (bindings, identity) = standard();
    let (fresh, _, b) = Fixture::started(29);
    let mut retried = fresh.clone();
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
        bindings
            .dispatch(&mut retried, b, &identity, rejected_call)
            .expect_err("rejected");
    }
    let mut fresh = fresh;
    let expected = bindings
        .dispatch(&mut fresh, b, &identity, &strike_call(ATTACK))
        .expect("fresh");
    let actual = bindings
        .dispatch(&mut retried, b, &identity, &strike_call(ATTACK))
        .expect("retry");
    assert_eq!(actual, expected);
    assert_eq!(snapshot(&retried), snapshot(&fresh));
}

/// A fixed invocation script: `(action, args)` for whoever holds the turn.
fn script() -> Vec<ActionCall> {
    vec![
        strike_call(ATTACK),
        flurry_call(JAB, 3),
        call("pass", Vec::new()),
        strike_call(WHIFF),
        call(
            "combo",
            vec![("first", ability_ref(JAB)), ("second", ability_ref(ATTACK))],
        ),
        strike_call(ATTACK),
    ]
}

#[test]
fn worlds_loaded_from_the_same_bytes_dispatch_identically() {
    let (origin, _, _) = Fixture::started(31);
    let bytes = serde_json::to_vec(&origin).expect("serializes");
    let mut left: World = serde_json::from_slice(&bytes).expect("loads");
    let mut right: World = serde_json::from_slice(&bytes).expect("loads");

    let entries = table();
    let store = store_of(&entries);
    let identity = store.identity().clone();
    let left_bindings = build(&store, &entries).expect("binds");
    let mut reversed = entries.clone();
    reversed.reverse();
    let right_bindings = build(&store, &reversed).expect("binds");

    for step in script() {
        let left_actor = active(&left);
        let right_actor = active(&right);
        assert_eq!(left_actor, right_actor);
        let l = left_bindings.dispatch(&mut left, left_actor, &identity, &step);
        let r = right_bindings.dispatch(&mut right, right_actor, &identity, &step);
        assert!(l.is_ok(), "{l:?}");
        assert_eq!(l, r);
        assert_eq!(snapshot(&left), snapshot(&right));
    }
}

#[test]
fn saving_the_world_between_invocations_resumes_identically() {
    let calls_script = script();
    let split = 3;

    // Uninterrupted run.
    let (bindings, identity) = standard();
    let (mut uninterrupted, _, _) = Fixture::started(37);
    let origin = uninterrupted.clone();
    let mut expected = Vec::new();
    for step in &calls_script {
        let actor = active(&uninterrupted);
        expected.push(
            bindings
                .dispatch(&mut uninterrupted, actor, &identity, step)
                .expect("uninterrupted"),
        );
    }
    drop(bindings);

    // Interrupted: run the first part, save only the World, drop everything.
    let saved = {
        let (bindings, identity) = standard();
        let mut world = origin;
        for (index, step) in calls_script[..split].iter().enumerate() {
            let actor = active(&world);
            let outcome = bindings
                .dispatch(&mut world, actor, &identity, step)
                .expect("first part");
            assert_eq!(outcome, expected[index]);
        }
        serde_json::to_vec(&world).expect("saves")
    };

    // Reload the World and rebuild identical trusted bindings from scratch.
    let mut resumed: World = serde_json::from_slice(&saved).expect("loads");
    assert_eq!(serde_json::to_vec(&resumed).expect("re-saves"), saved);
    let entries = table();
    let store = store_of(&entries);
    let mut shuffled = entries.clone();
    shuffled.rotate_left(5);
    let rebuilt = build(&store, &shuffled).expect("rebuilt");
    let identity = store.identity().clone();
    for (offset, step) in calls_script[split..].iter().enumerate() {
        let actor = active(&resumed);
        let outcome = rebuilt
            .dispatch(&mut resumed, actor, &identity, step)
            .expect("resumed");
        assert_eq!(outcome, expected[split + offset]);
    }
    assert_eq!(snapshot(&resumed), snapshot(&uninterrupted));
}

// ---------------------------------------------------------------------------
// Authored input cannot install code; error surface
// ---------------------------------------------------------------------------

#[test]
fn forged_authored_ids_cannot_install_handlers() {
    let (bindings, identity) = standard();
    let (mut world, a, b) = Fixture::started(41);
    let before = snapshot(&world);

    // Lookup is inert and exact; combat ability ids are not IR ids.
    for absent in [
        "forged",
        "Strike",
        "strike ",
        "",
        &Ulid::from_u128(ATTACK).to_string(),
    ] {
        assert!(!bindings.contains(absent), "{absent:?}");
    }
    assert_unchanged(&world, &before, "lookups");

    // A call naming an undeclared id is a validation error, nothing more.
    let error = bindings
        .dispatch(&mut world, b, &identity, &call("forged", Vec::new()))
        .expect_err("forged");
    assert!(matches!(
        &error,
        BindingError::Call(inner) if inner.code == SignatureErrorCode::UnknownAction
    ));
    // A bundle that does declare it cannot be used against this table.
    let forged = ActionSignatureStore::new(
        Ulid::from_u128(BUNDLE),
        vec![signature("forged", Vec::new())],
    )
    .expect("store");
    let error = bindings
        .dispatch(
            &mut world,
            b,
            forged.identity(),
            &call("forged", Vec::new()),
        )
        .expect_err("foreign bundle");
    assert!(matches!(
        &error,
        BindingError::Call(inner) if inner.code == SignatureErrorCode::BundleMismatch
    ));
    assert_unchanged(&world, &before, "forged calls");
    assert!(!bindings.contains("forged"));

    // An authored call decoded from bytes only selects the trusted handler.
    let bytes = bindings
        .declarations()
        .write_call(&identity, &strike_call(ATTACK))
        .expect("encodes");
    let decoded = bindings
        .declarations()
        .read_call(&identity, &bytes)
        .expect("decodes");
    let (expected_world, expected) = oracle(
        &world,
        &[CombatAction::UseAbility {
            actor: b,
            ability: Ulid::from_u128(ATTACK),
            target: a,
        }],
    );
    let outcomes = bindings
        .dispatch(&mut world, b, &identity, &decoded)
        .expect("decoded call dispatches");
    assert_eq!(outcomes, vec![expected[0].clone().expect("legal")]);
    assert_eq!(snapshot(&world), snapshot(&expected_world));
}

#[test]
fn errors_display_and_expose_sources_as_pinned() {
    let (bindings, identity) = standard();
    let (mut world, _, b) = Fixture::started(43);

    let call_error = bindings
        .dispatch(&mut world, b, &identity, &call("forged", Vec::new()))
        .expect_err("call");
    let BindingError::Call(inner) = &call_error else {
        panic!("expected Call, got {call_error:?}");
    };
    assert_eq!(call_error.to_string(), inner.to_string());
    assert_eq!(call_error.to_string(), "unknown_action at /action_id");
    assert!(call_error.source().is_none(), "delegates the inner source");

    let actor_error = BindingError::Actor(CombatError::NoEncounter);
    assert_eq!(
        actor_error.to_string(),
        CombatError::NoEncounter.to_string()
    );
    assert!(actor_error.source().is_none());

    for (handler_error, text) in [
        (HandlerError::Refused, "Refused"),
        (HandlerError::UnsupportedWait, "UnsupportedWait"),
    ] {
        assert_eq!(handler_error.to_string(), text);
        assert!(handler_error.source().is_none());
        let wrapped = BindingError::Handler(handler_error);
        assert_eq!(wrapped.to_string(), text);
        assert!(wrapped.source().is_none());
    }

    let apply = BindingError::Apply {
        index: 7,
        error: CombatError::NoEncounter,
    };
    assert_eq!(apply.to_string(), "Apply at proposals/7");
    let source = apply.source().expect("Apply exposes its CombatError");
    assert_eq!(source.to_string(), CombatError::NoEncounter.to_string());

    let id = || "secret_action".to_owned();
    for (error, text) in [
        (BindingError::BundleMismatch, "BundleMismatch at bindings"),
        (BindingError::TooManyBindings, "TooManyBindings at bindings"),
        (
            BindingError::DuplicateBinding { action_id: id() },
            "DuplicateBinding at bindings",
        ),
        (
            BindingError::UnknownBinding { action_id: id() },
            "UnknownBinding at bindings",
        ),
        (
            BindingError::MissingBinding { action_id: id() },
            "MissingBinding at bindings",
        ),
        (
            BindingError::SignatureMismatch { action_id: id() },
            "SignatureMismatch at bindings",
        ),
        (BindingError::ProposalLimit, "ProposalLimit at bindings"),
        (
            BindingError::ForeignActor { index: 3 },
            "ForeignActor at bindings",
        ),
    ] {
        assert_eq!(error.to_string(), text);
        assert!(error.source().is_none());
    }

    // Debug of the table never prints function addresses.
    let rendered = format!("{bindings:?}");
    assert!(rendered.contains("strike") && !rendered.contains("0x"));
}
