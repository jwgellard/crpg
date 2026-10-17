//! Audit-gap tests for crpg-rules (T126).

#[allow(dead_code)]
#[path = "common/mod.rs"]
mod common;

use std::collections::BTreeMap;

use crpg_core::{DeterministicRng, EntityId, GenerationalArena};
use crpg_rules::{
    Against, AgainstBreakdown, DiceExpr, DiceSelection, DieResult, KernelHook, ModOp, Modifier,
    ModifierPipeline, ModifierTarget, Outcome, OutcomeBand, OutcomeTable, OutcomeTableId,
    ResolutionContext, ResolutionRequest, RollRequest, RollSpec, RollTag, RulesErrorCode,
    SerializableStatValue, StackingPolicy, StatBlock, StatValue,
};

use common::{
    block, context, fixture, int_def, numeric_modifier, policies, tag_set, uid, TYPE_A, TYPE_B,
    TYPE_C,
};

// ---------------------------------------------------------------------------
// Helpers (copied from resolution.rs patterns; no source edits).
// ---------------------------------------------------------------------------

fn four_band() -> OutcomeTable {
    OutcomeTable::new(
        OutcomeTableId(uid(1)),
        vec![
            OutcomeBand {
                min_margin: i64::MIN,
                outcome: Outcome::CriticalFailure,
            },
            OutcomeBand {
                min_margin: -10,
                outcome: Outcome::Failure,
            },
            OutcomeBand {
                min_margin: 0,
                outcome: Outcome::Success,
            },
            OutcomeBand {
                min_margin: 10,
                outcome: Outcome::CriticalSuccess,
            },
        ],
        vec![
            crpg_rules::NaturalRule {
                face: 6,
                effect: crpg_rules::NaturalEffect::Shift(1),
            },
            crpg_rules::NaturalRule {
                face: 1,
                effect: crpg_rules::NaturalEffect::Shift(-1),
            },
        ],
    )
    .unwrap()
}

fn int_pipeline(fx: &common::Fixture) -> ModifierPipeline {
    ModifierPipeline::new(
        vec![int_def(fx.stats[0]), int_def(fx.stats[1])],
        policies(&[
            (TYPE_A, StackingPolicy::StackAll),
            (TYPE_B, StackingPolicy::HighestBonusWorstPenalty),
            (TYPE_C, StackingPolicy::HighestPriorityPerName),
        ]),
    )
    .unwrap()
}

fn roll_request(
    actor: EntityId,
    spec: RollSpec,
    bonus: RollTag,
    natural: Option<u32>,
    stream: &str,
) -> RollRequest {
    RollRequest {
        actor,
        roll: spec,
        bonus_target: bonus,
        natural_die: natural,
        stream: stream.to_owned(),
        tags: crpg_rules::TagSet::new(),
    }
}

fn resolve_request(
    roll: RollRequest,
    against: Against,
    table: OutcomeTableId,
) -> ResolutionRequest {
    ResolutionRequest {
        roll,
        target: None,
        against,
        outcome_table: table,
    }
}

fn dice(spec: &str) -> RollSpec {
    RollSpec::Dice(spec.parse::<DiceExpr>().unwrap())
}

fn arena_entity() -> (GenerationalArena<()>, EntityId) {
    let mut arena = GenerationalArena::new();
    let entity = arena.insert(());
    (arena, entity)
}

// ---------------------------------------------------------------------------
// #54 value-kind serde round trip covers Dice as well as the five T014 kinds.
// ---------------------------------------------------------------------------

#[test]
fn value_kind_serde_round_trips_all_six_kinds() {
    use crpg_core::Fx16_16;
    let raw = Fx16_16::from_raw(32768);
    let cases = [
        SerializableStatValue::Int(7),
        SerializableStatValue::Fixed(raw),
        SerializableStatValue::Bool(true),
        SerializableStatValue::Enum(crpg_rules::EnumValue {
            enum_id: String::from("ed"),
            variant: String::from("ex"),
        }),
        SerializableStatValue::Tags(vec![String::from("ta"), String::from("tb")]),
        SerializableStatValue::Dice(String::from("2d6+3")),
    ];
    for value in &cases {
        let json = serde_json::to_string(value).unwrap();
        let loaded: SerializableStatValue = serde_json::from_str(&json).unwrap();
        assert_eq!(&loaded, value);
    }
}

// ---------------------------------------------------------------------------
// #55 hook decoding rejects a missing correlation ULID and a missing field
// for every resolution variant.
// ---------------------------------------------------------------------------

#[test]
fn hook_decoding_rejects_missing_correlation_and_fields() {
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

    // BeforeRoll: missing required fields (target is optional).
    for json in [
        format!(r#"{{"type":"before_roll","target":{target_wire},"resolution":"{resolution}"}}"#),
        format!(r#"{{"type":"before_roll","actor":{entity_wire},"target":{target_wire}}}"#),
    ] {
        assert!(
            serde_json::from_str::<KernelHook>(&json).is_err(),
            "before_roll accepted: {json}"
        );
    }
    // AfterRoll: missing required fields (target is optional).
    for json in [
        format!(r#"{{"type":"after_roll","target":null,"resolution":"{resolution}","total":0}}"#),
        format!(r#"{{"type":"after_roll","actor":{entity_wire},"target":null,"total":0}}"#),
        format!(
            r#"{{"type":"after_roll","actor":{entity_wire},"target":null,"resolution":"{resolution}"}}"#
        ),
    ] {
        assert!(
            serde_json::from_str::<KernelHook>(&json).is_err(),
            "after_roll accepted: {json}"
        );
    }
    // BeforeDamage: missing each field.
    for json in [
        format!(
            r#"{{"type":"before_damage","target":{target_wire},"resolution":"{resolution}","amount":1}}"#
        ),
        format!(
            r#"{{"type":"before_damage","source":{entity_wire},"resolution":"{resolution}","amount":1}}"#
        ),
        format!(
            r#"{{"type":"before_damage","source":{entity_wire},"target":{target_wire},"amount":1}}"#
        ),
        format!(
            r#"{{"type":"before_damage","source":{entity_wire},"target":{target_wire},"resolution":"{resolution}"}}"#
        ),
    ] {
        assert!(
            serde_json::from_str::<KernelHook>(&json).is_err(),
            "before_damage accepted: {json}"
        );
    }
    // AfterDamage: missing each field.
    for json in [
        format!(
            r#"{{"type":"after_damage","target":{target_wire},"resolution":"{resolution}","amount":1}}"#
        ),
        format!(
            r#"{{"type":"after_damage","source":{entity_wire},"resolution":"{resolution}","amount":1}}"#
        ),
        format!(
            r#"{{"type":"after_damage","source":{entity_wire},"target":{target_wire},"amount":1}}"#
        ),
        format!(
            r#"{{"type":"after_damage","source":{entity_wire},"target":{target_wire},"resolution":"{resolution}"}}"#
        ),
    ] {
        assert!(
            serde_json::from_str::<KernelHook>(&json).is_err(),
            "after_damage accepted: {json}"
        );
    }
    // Missing correlation ULID specifically (resolution null/absent already
    // covered above); also reject a null resolution identity.
    let null_res = format!(
        r#"{{"type":"before_roll","actor":{entity_wire},"target":{target_wire},"resolution":null}}"#
    );
    assert!(serde_json::from_str::<KernelHook>(&null_res).is_err());
}

// ---------------------------------------------------------------------------
// #98 one-sided draw consumes exactly one range draw on the named stream.
// ---------------------------------------------------------------------------

#[test]
fn one_sided_die_consumes_one_range_draw() {
    let one = DiceExpr::new(1, 1, DiceSelection::All, 0).unwrap();
    let mut first = DeterministicRng::from_seed(5);
    let roll = one.evaluate(&mut first, "ones").unwrap();
    assert_eq!(roll.dice[0].value, 1);
    let mut second = DeterministicRng::from_seed(5);
    // Advance a fresh stream by exactly one range draw.
    let _ = second.stream("ones").next_u32();
    assert_eq!(first, second);
}

// ---------------------------------------------------------------------------
// #99 shared-stream opponent result checked against a hand-computed expectation.
// ---------------------------------------------------------------------------

#[test]
fn shared_stream_opponent_matches_hand_computed_expectation() {
    let mut fx = fixture();
    let second = fx.arena.insert(());
    let pipeline = int_pipeline(&fx);
    let table = four_band();
    let actor_roll = roll_request(
        fx.entity,
        dice("1d6"),
        RollTag(fx.tags[0]),
        Some(0),
        "shared",
    );
    let opposed_roll = roll_request(second, dice("1d6"), RollTag(fx.tags[0]), Some(0), "shared");
    let request = resolve_request(actor_roll, Against::Opposed(opposed_roll), table.id());
    let stored = StatBlock::new();
    let other_stored = StatBlock::new();
    let tags = crpg_rules::TagSet::new();
    let other_tags = crpg_rules::TagSet::new();
    let mods: Vec<Modifier> = vec![];
    let other_mods: Vec<Modifier> = vec![];
    let mut map = BTreeMap::new();
    map.insert(fx.entity, context(fx.entity, &stored, &tags, &mods));
    map.insert(
        second,
        context(second, &other_stored, &other_tags, &other_mods),
    );
    let resolution = ResolutionContext {
        pipeline: &pipeline,
        entities: &map,
        outcome_table: &table,
    };
    let mut rng = DeterministicRng::from_seed(100);
    let result = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap();
    assert_eq!(result.actor.natural, Some(3));
    let AgainstBreakdown::Opposed(opposed) = &result.against else {
        panic!("expected opposed breakdown");
    };
    // Hand-computed from the seeded stream: the second draw on "shared" is 6.
    let expected = vec![DieResult {
        value: 6,
        kept: true,
    }];
    assert_eq!(opposed.roll.as_ref().unwrap().dice, expected);
}

// ---------------------------------------------------------------------------
// #53 missing stacking policy fails rather than defaulting to StackAll.
// ---------------------------------------------------------------------------

#[test]
fn missing_stacking_policy_fails() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![int_def(fx.stats[0])],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], StatValue::Int(5))]);
    let tags = tag_set(&[]);
    let mods = vec![numeric_modifier(
        1,
        "src",
        1,
        ModifierTarget::Stat(fx.stats[0]),
        ModOp::Set(StatValue::Int(9)),
        TYPE_B,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let err = pipeline.query(fx.entity, fx.stats[0], &ctx).unwrap_err();
    assert_eq!(err.code, RulesErrorCode::UnknownModifierType);
}
