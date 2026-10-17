//! Audit-gap tests for crpg-script (T124).
//!
//! Each test here proves one contract behaviour named by the crate audit's
//! unverified leads. Helpers are copied from `action_bindings.rs`.

use std::cell::Cell;
use std::collections::BTreeMap;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_data::{
    Ability, ActionBundleIdentity, ActionCall, ActionParameter, ActionSignature,
    ActionSignatureStore, Creature, DataValue, Effect, Encounter,
    OutcomeTable as AuthoredOutcomeTable, Placement, Ruleset, SignatureErrorCode, ValueType,
    MAX_ACTION_SIGNATURES,
};
use crpg_script::{
    ActionBinding, ActionBindings, ActionHandler, BindingError, HandlerError, InvocationContext,
};
use crpg_sim::{
    perform_action, start_encounter, state_hash, CombatAction, CombatError, EncounterSpec, EntityMeta,
    PlacementAndArea, World,
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
// Combat fixture (copied)
// ---------------------------------------------------------------------------

const ATTACK: u128 = 103;
const JAB: u128 = 201;
const SLAY: u128 = 202;
const WHIFF: u128 = 203;
const BUNDLE: u128 = 900;

fn uid(value: u128) -> String {
    Ulid::from_u128(value).to_string()
}

fn fx_raw(value: i32) -> i32 {
    Fx16_16::from_int(value).to_raw()
}

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

// ---------------------------------------------------------------------------
// Declarations and trusted handlers
// ---------------------------------------------------------------------------

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

fn build(
    store: &ActionSignatureStore,
    entries: &[(ActionSignature, ActionHandler)],
) -> Result<ActionBindings, BindingError> {
    ActionBindings::new(store.clone(), store.identity().clone(), bind(entries))
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
        (signature("tripwire", Vec::new()), tripwire),
        (signature("wait", Vec::new()), wait),
        (signature("heavy", Vec::new()), heavy_foreign),
    ]
}

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
    call("tripwire", vec![])
}

fn object_arg(call: &ActionCall, name: &str) -> Result<Ulid, HandlerError> {
    match call.args.get(name) {
        Some(DataValue::ObjectRef(id)) => Ok(*id),
        _ => Err(HandlerError::Refused),
    }
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

fn tripwire(
    _context: &InvocationContext<'_>,
    _call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    panic!("tripwire handler invoked");
}

fn heavy_foreign(
    context: &InvocationContext<'_>,
    _call: &ActionCall,
) -> Result<Vec<CombatAction>, HandlerError> {
    record();
    let target = opponent(context)?;
    let mut out = Vec::with_capacity(33);
    for _ in 0..32 {
        out.push(CombatAction::UseAbility {
            actor: context.actor(),
            ability: Ulid::from_u128(ATTACK),
            target,
        });
    }
    out.push(CombatAction::EndTurn { actor: target });
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// Lead #56/#59: more than 32 proposals that also include a foreign actor
// report the error the pinned order puts first (ProposalLimit before
// ForeignActor).
#[test]
fn proposal_limit_precedes_foreign_actor() {
    // Control: a foreign actor alone is ForeignActor.
    let (bindings, identity) = standard();
    let (mut world, _, b) = Fixture::started(19);
    let before = snapshot(&world);
    let start = calls();
    let error = bindings
        .dispatch(&mut world, b, &identity, &call("hijack", Vec::new()))
        .expect_err("foreign actor");
    assert_eq!(error, BindingError::ForeignActor { index: 1 });
    assert_eq!(calls(), start + 1);
    assert_unchanged(&world, &before, "foreign actor control");

    // Over-count input that also contains a foreign actor: count wins.
    let entries = vec![(
        signature("heavy", Vec::new()),
        heavy_foreign as ActionHandler,
    )];
    let store = store_of(&entries);
    let identity = store.identity().clone();
    let bindings = build(&store, &entries).expect("heavy binds");
    let (mut world, _, b) = Fixture::started(19);
    let before = snapshot(&world);
    let start = calls();
    let error = bindings
        .dispatch(&mut world, b, &identity, &call("heavy", Vec::new()))
        .expect_err("over count with foreign actor");
    assert_eq!(error, BindingError::ProposalLimit);
    assert_eq!(calls(), start + 1);
    assert_unchanged(&world, &before, "proposal limit");
}

// Lead #57: Call-before-Actor precedence against each actor-error kind.
#[test]
fn call_precedes_every_actor_error_kind() {
    let (bindings, identity) = standard();
    let fixture = Fixture::new();

    // No encounter.
    let mut idle = World::new(4);
    let loner = idle.spawn(EntityMeta {});
    let before = snapshot(&idle);
    let error = bindings
        .dispatch(&mut idle, loner, &identity, &call("forged", Vec::new()))
        .expect_err("no encounter + bad call");
    assert!(matches!(
        &error,
        BindingError::Call(inner) if inner.code == SignatureErrorCode::UnknownAction
    ));
    assert_unchanged(&idle, &before, "no encounter");

    // Live non-participant, stale id, out-of-turn.
    let mut world = World::new(4);
    let outsider = world.spawn(EntityMeta {});
    let stale = world.spawn(EntityMeta {});
    world.despawn(stale);
    let (a, b) = fixture.start_in(&mut world);
    assert_eq!(active(&world), b);
    let cases = [outsider, stale, a];
    let before = snapshot(&world);
    for actor in &cases {
        let error = bindings
            .dispatch(&mut world, *actor, &identity, &call("forged", Vec::new()))
            .expect_err("actor + bad call");
        assert!(
            matches!(
                &error,
                BindingError::Call(inner) if inner.code == SignatureErrorCode::UnknownAction
            ),
            "actor {actor:?}"
        );
        assert_unchanged(&world, &before, "actor + bad call");
    }

    // Dead combatant.
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
        .dispatch(&mut killed, a, &identity, &call("forged", Vec::new()))
        .expect_err("dead + bad call");
    assert!(matches!(
        &error,
        BindingError::Call(inner) if inner.code == SignatureErrorCode::UnknownAction
    ));
    assert_unchanged(&killed, &before_dead, "dead + bad call");

    // Positive control: a valid call with the same actors still yields Actor errors.
    let error = bindings
        .dispatch(&mut world, outsider, &identity, &tripwire_call())
        .expect_err("non-participant");
    assert!(matches!(
        error,
        BindingError::Actor(CombatError::NotParticipant { .. })
    ));
}

// Lead #60: the Apply-rejection test's handler ran exactly once.
#[test]
fn apply_rejection_handler_runs_once() {
    let (bindings, identity) = standard();
    let (mut world, a, b) = Fixture::started(17);
    let before = snapshot(&world);
    let start = calls();
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
        .expect_err("apply rejection");
    assert_eq!(
        error,
        BindingError::Apply {
            index: 1,
            error: CombatError::DeadTarget { target: a },
        }
    );
    assert_eq!(calls(), start + 1, "handler ran exactly once");
    assert_unchanged(&world, &before, "apply rejection");
}

// Lead #101: each of the six rejection paths leaves the world unchanged.
#[test]
fn retry_rejections_each_leave_world_unchanged() {
    let (bindings, identity) = standard();
    let (mut world, _, b) = Fixture::started(29);
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
        call("strike", vec![("ability", ability_ref(999))]),
    ];
    for rejected_call in &rejected {
        let before = snapshot(&world);
        let _ = bindings
            .dispatch(&mut world, b, &identity, rejected_call)
            .expect_err("rejected");
        assert_unchanged(&world, &before, "rejection path");
    }
}

// Lead #102: the empty-proposals case asserts the handler ran.
#[test]
fn empty_proposals_handler_ran() {
    let (bindings, identity) = standard();
    let (mut world, _, b) = Fixture::started(13);
    let before = snapshot(&world);
    let start = calls();
    let outcomes = bindings
        .dispatch(&mut world, b, &identity, &flurry_call(JAB, 0))
        .expect("empty proposals");
    assert!(outcomes.is_empty());
    assert_eq!(calls(), start + 1, "handler ran");
    assert_unchanged(&world, &before, "empty proposals");
}

// Lead #62 (unknown): count bound precedes unknown ids.
#[test]
fn binding_count_precedes_unknown() {
    // Over-count input: one declared id plus 1024 unknown ids (1025 total),
    // no duplicate and no missing declared id.
    let declared = vec![(signature("zzzz", Vec::new()), pass as ActionHandler)];
    let store = store_of(&declared);
    let mut over: Vec<(ActionSignature, ActionHandler)> = vec![declared[0].clone()];
    for i in 0..MAX_ACTION_SIGNATURES {
        over.push((
            signature(&format!("u{i:04}"), Vec::new()),
            pass as ActionHandler,
        ));
    }
    assert_eq!(over.len(), MAX_ACTION_SIGNATURES + 1);
    let error = build(&store, &over).expect_err("over count");
    assert_eq!(error, BindingError::TooManyBindings);

    // Same input minus the excess (1024 bindings) still has an unknown id and
    // no missing declared id, so it fails as UnknownBinding on its own.
    let mut under: Vec<(ActionSignature, ActionHandler)> = vec![declared[0].clone()];
    for i in 0..(MAX_ACTION_SIGNATURES - 1) {
        under.push((
            signature(&format!("u{i:04}"), Vec::new()),
            pass as ActionHandler,
        ));
    }
    assert_eq!(under.len(), MAX_ACTION_SIGNATURES);
    let error = build(&store, &under).expect_err("unknown on its own");
    assert_eq!(
        error,
        BindingError::UnknownBinding {
            action_id: "u0000".into()
        }
    );
}
