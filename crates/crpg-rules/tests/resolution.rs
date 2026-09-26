//! Outcome tables, roll/DC modifier folds, and generic resolution.
//!
//! Neutral vocabulary only. Fixed seeds pin exact faces; failing calls must
//! preserve the complete RNG, including created streams.

#[allow(dead_code)]
#[path = "common/mod.rs"]
mod common;

use std::collections::BTreeMap;

use crpg_core::{DeterministicRng, EntityId};
use crpg_rules::{
    Against, AgainstBreakdown, DiceExpr, ModOp, Modifier, ModifierPipeline, ModifierTarget,
    NaturalEffect, NaturalRule, NumericModifierBreakdown, Outcome, OutcomeBand, OutcomeTable,
    OutcomeTableId, QueryContext, ResolutionContext, ResolutionRequest, RollRequest, RollSpec,
    RollTag, RulesErrorCode, StackingPolicy, StatBlock, StatValue, TagSet as RulesTagSet,
};

use common::{block, context, fixture, int_def, numeric_modifier, policies, raw_fx, tag_set, uid};

fn table_id(n: u128) -> OutcomeTableId {
    OutcomeTableId(uid(n))
}

fn roll_tag(fx: &common::Fixture, index: usize) -> RollTag {
    RollTag(fx.tags[index])
}

/// Four bands over MIN/-10/0/10 with shifts on faces 1 and 6.
fn four_band() -> OutcomeTable {
    OutcomeTable::new(
        table_id(1),
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
            NaturalRule {
                face: 6,
                effect: NaturalEffect::Shift(1),
            },
            NaturalRule {
                face: 1,
                effect: NaturalEffect::Shift(-1),
            },
        ],
    )
    .unwrap()
}

/// Two bands with a natural override on face 6.
fn two_band() -> OutcomeTable {
    OutcomeTable::new(
        table_id(2),
        vec![
            OutcomeBand {
                min_margin: i64::MIN,
                outcome: Outcome::Failure,
            },
            OutcomeBand {
                min_margin: 0,
                outcome: Outcome::Success,
            },
        ],
        vec![NaturalRule {
            face: 6,
            effect: NaturalEffect::Override(Outcome::CriticalSuccess),
        }],
    )
    .unwrap()
}

/// An alternative configuration with custom labels and other faces.
fn custom_table() -> OutcomeTable {
    OutcomeTable::new(
        table_id(3),
        vec![
            OutcomeBand {
                min_margin: i64::MIN,
                outcome: Outcome::Custom(1),
            },
            OutcomeBand {
                min_margin: 5,
                outcome: Outcome::Custom(2),
            },
            OutcomeBand {
                min_margin: 9,
                outcome: Outcome::Success,
            },
        ],
        vec![
            NaturalRule {
                face: 2,
                effect: NaturalEffect::Shift(-2),
            },
            NaturalRule {
                face: 5,
                effect: NaturalEffect::Shift(2),
            },
        ],
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
        tags: RulesTagSet::new(),
    }
}

fn dice(spec: &str) -> RollSpec {
    RollSpec::Dice(spec.parse::<DiceExpr>().unwrap())
}

fn int_pipeline(fx: &common::Fixture) -> ModifierPipeline {
    ModifierPipeline::new(
        vec![int_def(fx.stats[0]), int_def(fx.stats[1])],
        policies(&[
            (common::TYPE_A, StackingPolicy::StackAll),
            (common::TYPE_B, StackingPolicy::HighestBonusWorstPenalty),
            (common::TYPE_C, StackingPolicy::HighestPriorityPerName),
        ]),
    )
    .unwrap()
}

fn assert_rng_untouched(before: &DeterministicRng, after: &DeterministicRng, stream: &str) {
    assert_eq!(before, after, "a failed resolution must preserve the RNG");
    assert!(
        !after.has_stream(stream),
        "a failed resolution creates no stream"
    );
}

// Tables: selection, shifts, and validation.

#[test]
fn four_band_thresholds_and_extremes() {
    let table = four_band();
    let cases = [
        (i64::MIN, 0, Outcome::CriticalFailure),
        (-11, 0, Outcome::CriticalFailure),
        (-10, 1, Outcome::Failure),
        (-1, 1, Outcome::Failure),
        (0, 2, Outcome::Success),
        (9, 2, Outcome::Success),
        (10, 3, Outcome::CriticalSuccess),
        (i64::MAX, 3, Outcome::CriticalSuccess),
    ];
    for (margin, band, outcome) in cases {
        let decision = table.evaluate(margin, None);
        assert_eq!(decision.band_index, band, "margin {margin}");
        assert_eq!(decision.natural_rule_index, None, "margin {margin}");
        assert_eq!(decision.outcome, outcome, "margin {margin}");
    }
}

#[test]
fn natural_shifts_move_the_band_index_and_clamp() {
    let table = four_band();
    // Face 6 shifts toward higher margins: band 1 reads band 2's outcome.
    let decision = table.evaluate(-5, Some(6));
    assert_eq!(decision.band_index, 1);
    assert_eq!(decision.natural_rule_index, Some(0));
    assert_eq!(decision.outcome, Outcome::Success);
    // Face 1 shifts toward lower margins: band 1 reads band 0's outcome.
    let decision = table.evaluate(-5, Some(1));
    assert_eq!(decision.band_index, 1);
    assert_eq!(decision.natural_rule_index, Some(1));
    assert_eq!(decision.outcome, Outcome::CriticalFailure);
    // Shifts clamp at both ends instead of leaving the table.
    let decision = table.evaluate(50, Some(6));
    assert_eq!(decision.band_index, 3);
    assert_eq!(decision.outcome, Outcome::CriticalSuccess);
    let decision = table.evaluate(i64::MIN, Some(1));
    assert_eq!(decision.band_index, 0);
    assert_eq!(decision.outcome, Outcome::CriticalFailure);
    // An unmatched face leaves the band outcome alone.
    let decision = table.evaluate(-5, Some(3));
    assert_eq!(decision.band_index, 1);
    assert_eq!(decision.natural_rule_index, None);
    assert_eq!(decision.outcome, Outcome::Failure);
}

#[test]
fn two_band_override_and_custom_labels() {
    let table = two_band();
    let decision = table.evaluate(-5, None);
    assert_eq!(decision.outcome, Outcome::Failure);
    let decision = table.evaluate(-5, Some(6));
    assert_eq!(decision.band_index, 0);
    assert_eq!(decision.natural_rule_index, Some(0));
    assert_eq!(decision.outcome, Outcome::CriticalSuccess);
    let custom = custom_table();
    assert_eq!(custom.evaluate(0, None).outcome, Outcome::Custom(1));
    assert_eq!(custom.evaluate(5, None).outcome, Outcome::Custom(2));
    assert_eq!(custom.evaluate(9, None).outcome, Outcome::Success);
    // Face 5 shifts band 0 past band 1 into band 2.
    let decision = custom.evaluate(0, Some(5));
    assert_eq!(decision.band_index, 0);
    assert_eq!(decision.natural_rule_index, Some(1));
    assert_eq!(decision.outcome, Outcome::Success);
    // Face 2 shifts down and clamps at the first band.
    let decision = custom.evaluate(6, Some(2));
    assert_eq!(decision.band_index, 1);
    assert_eq!(decision.outcome, Outcome::Custom(1));
}

#[test]
fn table_validation_reports_exact_paths() {
    // Empty and oversized band collections.
    let error = OutcomeTable::new(table_id(1), vec![], vec![]).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidOutcomeTable);
    assert_eq!(error.location, "/outcome_table/bands");
    let mut bands = Vec::new();
    for index in 0..257 {
        bands.push(OutcomeBand {
            min_margin: i64::MIN + index as i64,
            outcome: Outcome::Failure,
        });
    }
    let error = OutcomeTable::new(table_id(1), bands, vec![]).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidOutcomeTable);
    assert_eq!(error.location, "/outcome_table/bands");
    // The first band must start at the extreme.
    let error = OutcomeTable::new(
        table_id(1),
        vec![OutcomeBand {
            min_margin: 0,
            outcome: Outcome::Failure,
        }],
        vec![],
    )
    .unwrap_err();
    assert_eq!(error.location, "/outcome_table/bands/0/min_margin");
    // Bounds never sort silently: duplicates and decreases fail in order.
    let error = OutcomeTable::new(
        table_id(1),
        vec![
            OutcomeBand {
                min_margin: i64::MIN,
                outcome: Outcome::Failure,
            },
            OutcomeBand {
                min_margin: 0,
                outcome: Outcome::Failure,
            },
            OutcomeBand {
                min_margin: 0,
                outcome: Outcome::Success,
            },
        ],
        vec![],
    )
    .unwrap_err();
    assert_eq!(error.location, "/outcome_table/bands/2/min_margin");
    // Natural-rule limits precede contents; faces validate in authored order.
    let mut rules = Vec::new();
    for face in 1..=257 {
        rules.push(NaturalRule {
            face: face as u32,
            effect: NaturalEffect::Shift(0),
        });
    }
    let error = OutcomeTable::new(
        table_id(1),
        vec![OutcomeBand {
            min_margin: i64::MIN,
            outcome: Outcome::Failure,
        }],
        rules,
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidOutcomeTable);
    assert_eq!(error.location, "/outcome_table/natural_rules");
    // A zero face fails at its own position.
    let error = OutcomeTable::new(
        table_id(1),
        vec![OutcomeBand {
            min_margin: i64::MIN,
            outcome: Outcome::Failure,
        }],
        vec![NaturalRule {
            face: 0,
            effect: NaturalEffect::Shift(0),
        }],
    )
    .unwrap_err();
    assert_eq!(error.location, "/outcome_table/natural_rules/0/face");
    // An oversized face fails at its own later position.
    let error = OutcomeTable::new(
        table_id(1),
        vec![OutcomeBand {
            min_margin: i64::MIN,
            outcome: Outcome::Failure,
        }],
        vec![
            NaturalRule {
                face: 3,
                effect: NaturalEffect::Shift(0),
            },
            NaturalRule {
                face: 1_000_001,
                effect: NaturalEffect::Shift(0),
            },
        ],
    )
    .unwrap_err();
    assert_eq!(error.location, "/outcome_table/natural_rules/1/face");
    // Repeated faces stay unambiguous by rejection at the second occurrence.
    let error = OutcomeTable::new(
        table_id(1),
        vec![OutcomeBand {
            min_margin: i64::MIN,
            outcome: Outcome::Failure,
        }],
        vec![
            NaturalRule {
                face: 4,
                effect: NaturalEffect::Shift(1),
            },
            NaturalRule {
                face: 4,
                effect: NaturalEffect::Shift(-1),
            },
        ],
    )
    .unwrap_err();
    assert_eq!(error.location, "/outcome_table/natural_rules/1/face");
    // Largest-valid collections build.
    let mut bands = Vec::new();
    for index in 0..256 {
        bands.push(OutcomeBand {
            min_margin: i64::MIN + index as i64,
            outcome: Outcome::Custom((index % 7) as u8),
        });
    }
    let mut rules = Vec::new();
    for face in 1..=256 {
        rules.push(NaturalRule {
            face: face as u32,
            effect: NaturalEffect::Shift(0),
        });
    }
    OutcomeTable::new(table_id(1), bands, rules).unwrap();
}

#[test]
fn table_wire_shapes_round_trip_and_reject_unknown_fields() {
    let table = OutcomeTable::new(
        table_id(9),
        vec![
            OutcomeBand {
                min_margin: i64::MIN,
                outcome: Outcome::CriticalFailure,
            },
            OutcomeBand {
                min_margin: 0,
                outcome: Outcome::Custom(7),
            },
        ],
        vec![
            NaturalRule {
                face: 6,
                effect: NaturalEffect::Shift(2),
            },
            NaturalRule {
                face: 1,
                effect: NaturalEffect::Override(Outcome::Success),
            },
        ],
    )
    .unwrap();
    let wire = serde_json::to_string(&table).unwrap();
    assert_eq!(
        wire,
        format!(
            "{{\"id\":\"{}\",\"bands\":[\
             {{\"min_margin\":-9223372036854775808,\"outcome\":{{\"type\":\"critical_failure\"}}}},\
             {{\"min_margin\":0,\"outcome\":{{\"type\":\"custom\",\"value\":7}}}}],\
             \"natural_rules\":[\
             {{\"face\":6,\"effect\":{{\"type\":\"shift\",\"value\":2}}}},\
             {{\"face\":1,\"effect\":{{\"type\":\"override\",\"value\":{{\"type\":\"success\"}}}}}}]}}",
            uid(9)
        )
    );
    let loaded: OutcomeTable = serde_json::from_str(&wire).unwrap();
    assert_eq!(loaded, table);
    assert_eq!(loaded.id(), table.id());
    assert_eq!(loaded.bands(), table.bands());
    assert_eq!(loaded.natural_rules(), table.natural_rules());
    // Unknown fields fail on the table, band, and rule objects.
    let bad = wire.replace("\"bands\"", "\"extra\":0,\"bands\"");
    assert!(serde_json::from_str::<OutcomeTable>(&bad).is_err());
    let bad = wire.replace("\"min_margin\"", "\"min_margin\",\"extra\":0,");
    assert!(serde_json::from_str::<OutcomeTable>(&bad).is_err());
    let bad = wire.replace("\"face\"", "\"face\",\"extra\":0,");
    assert!(serde_json::from_str::<OutcomeTable>(&bad).is_err());
    // Invalid decoded state runs constructor validation on decode.
    let bad = wire.replace("-9223372036854775808", "0");
    assert!(serde_json::from_str::<OutcomeTable>(&bad).is_err());
}

// Numeric roll/DC folds.

fn numeric_query(
    pipeline: &ModifierPipeline,
    fx: &common::Fixture,
    target: ModifierTarget,
    base: i32,
    modifiers: &[Modifier],
) -> Result<(i32, NumericModifierBreakdown), crpg_rules::RulesError> {
    let stored = StatBlock::new();
    let tags = RulesTagSet::new();
    let ctx = context(fx.entity, &stored, &tags, modifiers);
    pipeline.query_numeric(fx.entity, target, base, &ctx)
}

#[test]
fn numeric_queries_reject_stat_targets_after_the_entity_check() {
    let mut fx = fixture();
    let pipeline = int_pipeline(&fx);
    let stored = StatBlock::new();
    let tags = RulesTagSet::new();
    let mods: Vec<Modifier> = vec![];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let error = pipeline
        .query_numeric(fx.entity, ModifierTarget::Stat(fx.stats[0]), 3, &ctx)
        .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidTarget);
    assert_eq!(error.location, "/target");
    // Entity mismatch precedes the target-kind check; the other entity
    // shares this arena so its identity is genuinely distinct.
    let other = fx.arena.insert(());
    let error = pipeline
        .query_numeric(other, ModifierTarget::Stat(fx.stats[0]), 3, &ctx)
        .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::EntityMismatch);
}

#[test]
fn numeric_folds_share_stacking_ordering_and_traces() {
    let fx = fixture();
    let pipeline = int_pipeline(&fx);
    let roll = ModifierTarget::Roll(roll_tag(&fx, 0));
    // Deliberately unordered input: clamp, add, set, multiply.
    let modifiers = vec![
        numeric_modifier(
            4,
            "sk",
            1,
            roll,
            ModOp::Clamp {
                min: StatValue::Int(0),
                max: StatValue::Int(18),
            },
            common::TYPE_A,
            None,
            None,
            0,
        ),
        numeric_modifier(
            2,
            "sk",
            1,
            roll,
            ModOp::Add(StatValue::Int(3)),
            common::TYPE_A,
            None,
            None,
            0,
        ),
        numeric_modifier(
            1,
            "sk",
            1,
            roll,
            ModOp::Set(StatValue::Int(10)),
            common::TYPE_A,
            None,
            None,
            0,
        ),
        numeric_modifier(
            3,
            "sk",
            1,
            roll,
            ModOp::Multiply(raw_fx(98304)),
            common::TYPE_A,
            None,
            None,
            0,
        ),
    ];
    let (value, breakdown) = numeric_query(&pipeline, &fx, roll, 0, &modifiers).unwrap();
    // Set -> Add -> Multiply(1.5) -> Clamp(0, 18): 10 -> 13 -> 19 -> 18.
    assert_eq!(value, 18);
    assert_eq!(breakdown.entity, fx.entity);
    assert_eq!(breakdown.target, roll);
    assert_eq!(breakdown.base, 0);
    assert_eq!(breakdown.value, 18);
    let order: Vec<u128> = breakdown
        .contributions
        .iter()
        .map(|contribution| contribution.modifier.id.to_u128())
        .collect();
    assert_eq!(
        order,
        vec![1, 2, 3, 4]
            .into_iter()
            .map(uid)
            .map(|id| id.to_u128())
            .collect::<Vec<_>>()
    );
    // HighestBonusWorstPenalty keeps the best bonus and worst penalty.
    let dc = ModifierTarget::Dc(roll_tag(&fx, 1));
    let modifiers = vec![
        numeric_modifier(
            1,
            "sk",
            1,
            dc,
            ModOp::Add(StatValue::Int(2)),
            common::TYPE_B,
            None,
            None,
            0,
        ),
        numeric_modifier(
            2,
            "sk",
            1,
            dc,
            ModOp::Add(StatValue::Int(5)),
            common::TYPE_B,
            None,
            None,
            5,
        ),
        numeric_modifier(
            3,
            "sk",
            1,
            dc,
            ModOp::Add(StatValue::Int(-1)),
            common::TYPE_B,
            None,
            None,
            0,
        ),
        numeric_modifier(
            4,
            "sk",
            1,
            dc,
            ModOp::Add(StatValue::Int(-4)),
            common::TYPE_B,
            None,
            None,
            0,
        ),
        numeric_modifier(
            5,
            "sk",
            1,
            dc,
            ModOp::Add(StatValue::Int(0)),
            common::TYPE_B,
            None,
            None,
            0,
        ),
    ];
    let (value, breakdown) = numeric_query(&pipeline, &fx, dc, 10, &modifiers).unwrap();
    assert_eq!(value, 11);
    assert_eq!(breakdown.contributions.len(), 5);
    // HighestPriorityPerName keeps one winner per exact name.
    let modifiers = vec![
        numeric_modifier(
            1,
            "sk",
            1,
            dc,
            ModOp::Add(StatValue::Int(1)),
            common::TYPE_C,
            Some("nm-a"),
            None,
            0,
        ),
        numeric_modifier(
            2,
            "sk",
            2,
            dc,
            ModOp::Add(StatValue::Int(9)),
            common::TYPE_C,
            Some("nm-a"),
            None,
            0,
        ),
        numeric_modifier(
            3,
            "sk",
            1,
            dc,
            ModOp::Add(StatValue::Int(4)),
            common::TYPE_C,
            Some("nm-b"),
            None,
            0,
        ),
    ];
    let (value, breakdown) = numeric_query(&pipeline, &fx, dc, 0, &modifiers).unwrap();
    assert_eq!(value, 13);
    let suppressed: Vec<bool> = breakdown
        .contributions
        .iter()
        .map(|contribution| {
            matches!(
                contribution.status,
                crpg_rules::ContributionStatus::Suppressed { .. }
            )
        })
        .collect();
    assert_eq!(suppressed, vec![true, false, false]);
}

#[test]
fn numeric_queries_isolate_targets_and_reverse_by_removal() {
    let fx = fixture();
    let pipeline = int_pipeline(&fx);
    let roll = ModifierTarget::Roll(roll_tag(&fx, 0));
    let dc = ModifierTarget::Dc(roll_tag(&fx, 1));
    // A DC modifier never contributes to a roll fold and vice versa.
    let modifiers = vec![
        numeric_modifier(
            1,
            "sk",
            1,
            dc,
            ModOp::Add(StatValue::Int(100)),
            common::TYPE_A,
            None,
            None,
            0,
        ),
        numeric_modifier(
            2,
            "sk",
            1,
            roll,
            ModOp::Add(StatValue::Int(7)),
            common::TYPE_A,
            None,
            None,
            0,
        ),
    ];
    let (value, breakdown) = numeric_query(&pipeline, &fx, roll, 1, &modifiers).unwrap();
    assert_eq!(value, 8);
    assert_eq!(breakdown.contributions.len(), 1);
    // A stat query validates roll/DC modifiers but records no contribution.
    let stored = block(&[(fx.stats[0], StatValue::Int(3))]);
    let tags = RulesTagSet::new();
    let ctx = context(fx.entity, &stored, &tags, &modifiers);
    let (value, stat_breakdown) = pipeline.query(fx.entity, fx.stats[0], &ctx).unwrap();
    assert_eq!(value, StatValue::Int(3));
    assert!(stat_breakdown.traces[&fx.stats[0]].contributions.is_empty());
    // Removing the source restores the exact earlier value and breakdown.
    let augmented = vec![
        modifiers[0].clone(),
        modifiers[1].clone(),
        numeric_modifier(
            3,
            "fresh",
            7,
            roll,
            ModOp::Add(StatValue::Int(1000)),
            common::TYPE_A,
            None,
            None,
            0,
        ),
    ];
    let (before_value, before_breakdown) =
        numeric_query(&pipeline, &fx, roll, 1, &modifiers).unwrap();
    let (augmented_value, _) = numeric_query(&pipeline, &fx, roll, 1, &augmented).unwrap();
    assert_eq!(augmented_value, 1008);
    let survived: Vec<Modifier> = augmented
        .into_iter()
        .filter(|modifier| modifier.source.kind != "fresh")
        .collect();
    let (restored_value, restored_breakdown) =
        numeric_query(&pipeline, &fx, roll, 1, &survived).unwrap();
    assert_eq!(
        (restored_value, restored_breakdown),
        (before_value, before_breakdown)
    );
    // Saturation matches the canonical oracle: MAX + 1 - 1 is MAX - 1.
    let saturated = vec![
        numeric_modifier(
            1,
            "sk",
            1,
            roll,
            ModOp::Add(StatValue::Int(1)),
            common::TYPE_A,
            None,
            None,
            0,
        ),
        numeric_modifier(
            2,
            "sk",
            1,
            roll,
            ModOp::Add(StatValue::Int(-1)),
            common::TYPE_A,
            None,
            None,
            0,
        ),
    ];
    let (value, _) = numeric_query(&pipeline, &fx, roll, i32::MAX, &saturated).unwrap();
    assert_eq!(value, i32::MAX - 1);
    // Full-range integer scaling never passes through the fixed range.
    let scaled = vec![numeric_modifier(
        1,
        "sk",
        1,
        roll,
        ModOp::Multiply(raw_fx(65536)),
        common::TYPE_A,
        None,
        None,
        0,
    )];
    let (value, _) = numeric_query(&pipeline, &fx, roll, 100_000, &scaled).unwrap();
    assert_eq!(value, 100_000);
    let half = vec![numeric_modifier(
        1,
        "sk",
        1,
        roll,
        ModOp::Multiply(raw_fx(32768)),
        common::TYPE_A,
        None,
        None,
        0,
    )];
    let (value, _) = numeric_query(&pipeline, &fx, roll, -3, &half).unwrap();
    assert_eq!(value, -2);
}

#[test]
fn unrelated_malformed_modifiers_fail_numeric_queries() {
    let fx = fixture();
    let pipeline = int_pipeline(&fx);
    let roll = ModifierTarget::Roll(roll_tag(&fx, 0));
    // A malformed stat modifier fails a numeric query even though its target
    // differs; a mismatched Add operand on a roll target is a TypeMismatch.
    let modifiers = vec![
        numeric_modifier(
            1,
            "sk",
            1,
            ModifierTarget::Stat(fx.stats[0]),
            ModOp::Add(StatValue::Bool(true)),
            "no-such-type",
            None,
            None,
            0,
        ),
        numeric_modifier(
            2,
            "sk",
            1,
            roll,
            ModOp::Add(StatValue::Int(1)),
            common::TYPE_A,
            None,
            None,
            0,
        ),
    ];
    let error = numeric_query(&pipeline, &fx, roll, 0, &modifiers).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::UnknownModifierType);
    let modifiers = vec![numeric_modifier(
        2,
        "sk",
        1,
        roll,
        ModOp::Add(StatValue::Fixed(raw_fx(1))),
        common::TYPE_A,
        None,
        None,
        0,
    )];
    let error = numeric_query(&pipeline, &fx, roll, 0, &modifiers).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::TypeMismatch);
}

// Resolution flows.

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

fn single_context<'a>(
    entity: EntityId,
    stored: &'a StatBlock,
    tags: &'a RulesTagSet,
    modifiers: &'a [Modifier],
) -> (BTreeMap<EntityId, QueryContext<'a>>, QueryContext<'a>) {
    let view = context(entity, stored, tags, modifiers);
    let mut map = BTreeMap::new();
    map.insert(entity, view);
    (map, view)
}

#[test]
fn constant_dc_resolves_with_an_exact_pinned_trace() {
    let fx = fixture();
    let pipeline = int_pipeline(&fx);
    let table = four_band();
    // Seed 100 draws face 2 on stream "roll-a": raw total 4.
    let roll = roll_request(
        fx.entity,
        dice("1d6+2"),
        roll_tag(&fx, 0),
        Some(0),
        "roll-a",
    );
    let request = resolve_request(
        roll,
        Against::Dc {
            value: 4,
            owner: fx.entity,
            modifier_target: roll_tag(&fx, 1),
        },
        table.id(),
    );
    let stored = StatBlock::new();
    let tags = RulesTagSet::new();
    let mods: Vec<Modifier> = vec![];
    let (map, _) = single_context(fx.entity, &stored, &tags, &mods);
    let resolution = ResolutionContext {
        pipeline: &pipeline,
        entities: &map,
        outcome_table: &table,
    };
    let mut rng = DeterministicRng::from_seed(100);
    let result = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap();
    assert_eq!(result.actor.total, 4);
    assert_eq!(result.actor.natural, Some(2));
    assert_eq!(result.actor.roll.as_ref().unwrap().dice.len(), 1);
    assert!(result.actor.modifiers.contributions.is_empty());
    assert_eq!(result.against_value, 4);
    // Zero margin has no hidden tie winner: the table chooses Success.
    assert_eq!(result.margin, 0);
    assert_eq!(result.decision.band_index, 2);
    assert_eq!(result.decision.outcome, Outcome::Success);
    assert!(matches!(result.against, AgainstBreakdown::Dc(_)));
    // A successful roll commits its stream.
    assert!(rng.has_stream("roll-a"));
    assert_ne!(rng, DeterministicRng::from_seed(100));
}

#[test]
fn constant_dc_applies_owner_modifiers_and_actor_bonuses() {
    let fx = fixture();
    let pipeline = int_pipeline(&fx);
    let table = four_band();
    // Raw 4 plus a roll bonus of 3 against DC 10 plus a DC bonus of 2.
    let roll = roll_request(fx.entity, dice("1d6+2"), roll_tag(&fx, 0), None, "roll-a");
    let request = resolve_request(
        roll,
        Against::Dc {
            value: 10,
            owner: fx.entity,
            modifier_target: roll_tag(&fx, 1),
        },
        table.id(),
    );
    let stored = StatBlock::new();
    let tags = RulesTagSet::new();
    let mods = vec![
        numeric_modifier(
            1,
            "sk",
            1,
            ModifierTarget::Roll(roll_tag(&fx, 0)),
            ModOp::Add(StatValue::Int(3)),
            common::TYPE_A,
            None,
            None,
            0,
        ),
        numeric_modifier(
            2,
            "sk",
            1,
            ModifierTarget::Dc(roll_tag(&fx, 1)),
            ModOp::Add(StatValue::Int(2)),
            common::TYPE_A,
            None,
            None,
            0,
        ),
    ];
    let (map, _) = single_context(fx.entity, &stored, &tags, &mods);
    let resolution = ResolutionContext {
        pipeline: &pipeline,
        entities: &map,
        outcome_table: &table,
    };
    let mut rng = DeterministicRng::from_seed(100);
    let result = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap();
    assert_eq!(result.actor.total, 7);
    assert_eq!(result.actor.natural, None);
    assert_eq!(result.against_value, 12);
    assert_eq!(result.margin, -5);
    assert_eq!(result.decision.outcome, Outcome::Failure);
    let AgainstBreakdown::Dc(dc) = &result.against else {
        panic!("expected a DC breakdown");
    };
    assert_eq!(dc.base, 10);
    assert_eq!(dc.value, 12);
}

#[test]
fn stat_dc_reuses_its_preflight_trace() {
    let fx = fixture();
    let pipeline = int_pipeline(&fx);
    let table = four_band();
    let roll = roll_request(fx.entity, RollSpec::None, roll_tag(&fx, 0), None, "");
    let request = resolve_request(
        roll,
        Against::Stat {
            entity: fx.entity,
            stat: fx.stats[0],
            modifier_target: roll_tag(&fx, 1),
        },
        table.id(),
    );
    // Stat 8 plus 2, then a DC bonus of 1: defence 11 against raw 0.
    let stored = block(&[
        (fx.stats[0], StatValue::Int(8)),
        (fx.stats[1], StatValue::Int(0)),
    ]);
    let tags = RulesTagSet::new();
    let mods = vec![
        common::modifier(
            1,
            "sk",
            1,
            fx.stats[0],
            ModOp::Add(StatValue::Int(2)),
            common::TYPE_A,
            None,
            None,
            0,
        ),
        numeric_modifier(
            2,
            "sk",
            1,
            ModifierTarget::Dc(roll_tag(&fx, 1)),
            ModOp::Add(StatValue::Int(1)),
            common::TYPE_A,
            None,
            None,
            0,
        ),
    ];
    let (map, _) = single_context(fx.entity, &stored, &tags, &mods);
    let resolution = ResolutionContext {
        pipeline: &pipeline,
        entities: &map,
        outcome_table: &table,
    };
    let mut rng = DeterministicRng::from_seed(100);
    let before = rng.clone();
    let result = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap();
    assert_eq!(result.actor.total, 0);
    assert_eq!(result.against_value, 11);
    assert_eq!(result.margin, -11);
    assert_eq!(result.decision.band_index, 0);
    assert_eq!(result.decision.outcome, Outcome::CriticalFailure);
    let AgainstBreakdown::Stat { stat, dc } = &result.against else {
        panic!("expected a stat breakdown");
    };
    assert_eq!(stat.traces[&fx.stats[0]].value, StatValue::Int(10));
    assert_eq!(dc.base, 10);
    assert_eq!(dc.value, 11);
    // No side rolled, so no RNG state changed.
    assert_eq!(rng, before);
}

#[test]
fn non_int_stat_dc_is_rejected() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![common::def(fx.stats[0], crpg_rules::StatKind::Bool, None)],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let table = four_band();
    let roll = roll_request(fx.entity, RollSpec::None, roll_tag(&fx, 0), None, "");
    let request = resolve_request(
        roll,
        Against::Stat {
            entity: fx.entity,
            stat: fx.stats[0],
            modifier_target: roll_tag(&fx, 1),
        },
        table.id(),
    );
    let stored = block(&[(fx.stats[0], StatValue::Bool(true))]);
    let tags = RulesTagSet::new();
    let mods: Vec<Modifier> = vec![];
    let (map, _) = single_context(fx.entity, &stored, &tags, &mods);
    let resolution = ResolutionContext {
        pipeline: &pipeline,
        entities: &map,
        outcome_table: &table,
    };
    let mut rng = DeterministicRng::from_seed(100);
    let before = rng.clone();
    let error = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::TypeMismatch);
    assert_eq!(error.location, "/resolution/against");
    assert_rng_untouched(&before, &rng, "roll-a");
}

#[test]
fn opposed_rolls_use_distinct_streams() {
    let mut fx = fixture();
    let second = fx.arena.insert(());
    let pipeline = int_pipeline(&fx);
    let table = four_band();
    // Seed 100: actor face 2 plus offset 2 is 4; opponent rolls 1 + 1 is 2.
    let actor_roll = roll_request(
        fx.entity,
        dice("1d6+2"),
        roll_tag(&fx, 0),
        Some(0),
        "roll-a",
    );
    let opposed_roll = roll_request(second, dice("2d6"), roll_tag(&fx, 0), None, "roll-b");
    let request = resolve_request(actor_roll, Against::Opposed(opposed_roll), table.id());
    let stored = StatBlock::new();
    let other_stored = StatBlock::new();
    let tags = RulesTagSet::new();
    let other_tags = RulesTagSet::new();
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
    assert_eq!(result.actor.total, 4);
    assert_eq!(result.actor.natural, Some(2));
    let AgainstBreakdown::Opposed(opposed) = &result.against else {
        panic!("expected opposed breakdown");
    };
    assert_eq!(opposed.total, 2);
    assert_eq!(result.against_value, 2);
    assert_eq!(result.margin, 2);
    assert_eq!(result.decision.outcome, Outcome::Success);
    assert!(rng.has_stream("roll-a"));
    assert!(rng.has_stream("roll-b"));
}

#[test]
fn opposed_shared_streams_consume_consecutive_draws() {
    let mut fx = fixture();
    let second = fx.arena.insert(());
    let pipeline = int_pipeline(&fx);
    let table = four_band();
    let actor_roll = roll_request(fx.entity, dice("1d6"), roll_tag(&fx, 0), Some(0), "shared");
    let opposed_roll = roll_request(second, dice("1d6"), roll_tag(&fx, 0), Some(0), "shared");
    let request = resolve_request(actor_roll, Against::Opposed(opposed_roll), table.id());
    let stored = StatBlock::new();
    let other_stored = StatBlock::new();
    let tags = RulesTagSet::new();
    let other_tags = RulesTagSet::new();
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
    // Seed 100 draws face 3 first on "shared": the actor consumes it.
    assert_eq!(result.actor.natural, Some(3));
    // The opponent consumes the next draw on the same stream.
    let mut parallel = DeterministicRng::from_seed(100);
    let expr: DiceExpr = "1d6".parse().unwrap();
    expr.evaluate(&mut parallel, "shared").unwrap();
    let expected = expr.evaluate(&mut parallel, "shared").unwrap();
    let AgainstBreakdown::Opposed(opposed) = &result.against else {
        panic!("expected opposed breakdown");
    };
    assert_eq!(opposed.roll.as_ref().unwrap().dice, expected.dice);
    assert_eq!(
        result.margin,
        i64::from(result.actor.total) - i64::from(result.against_value)
    );
    assert_eq!(
        result.decision,
        table.evaluate(result.margin, result.actor.natural)
    );
}

#[test]
fn same_entity_opposition_validates_once_and_succeeds() {
    let fx = fixture();
    let pipeline = int_pipeline(&fx);
    let table = four_band();
    let actor_roll = roll_request(fx.entity, dice("1d6+2"), roll_tag(&fx, 0), None, "roll-a");
    let opposed_roll = roll_request(fx.entity, dice("2d6"), roll_tag(&fx, 0), None, "roll-b");
    let request = resolve_request(actor_roll, Against::Opposed(opposed_roll), table.id());
    let stored = StatBlock::new();
    let tags = RulesTagSet::new();
    let mods: Vec<Modifier> = vec![];
    let (map, _) = single_context(fx.entity, &stored, &tags, &mods);
    let resolution = ResolutionContext {
        pipeline: &pipeline,
        entities: &map,
        outcome_table: &table,
    };
    let mut rng = DeterministicRng::from_seed(100);
    let result = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap();
    assert_eq!(result.actor.total, 4);
    assert_eq!(result.against_value, 2);
    assert_eq!(result.margin, 2);
}

#[test]
fn dropped_natural_selectors_read_raw_faces() {
    let fx = fixture();
    let pipeline = int_pipeline(&fx);
    let table = four_band();
    // Seed 101 draws 5, 6, 3, 1 and drops the 1: selecting index 3 still
    // reads the raw dropped face.
    let roll = roll_request(
        fx.entity,
        dice("4d6kh3"),
        roll_tag(&fx, 0),
        Some(3),
        "roll-a",
    );
    let request = resolve_request(
        roll,
        Against::Dc {
            value: 14,
            owner: fx.entity,
            modifier_target: roll_tag(&fx, 1),
        },
        table.id(),
    );
    let stored = StatBlock::new();
    let tags = RulesTagSet::new();
    let mods: Vec<Modifier> = vec![];
    let (map, _) = single_context(fx.entity, &stored, &tags, &mods);
    let resolution = ResolutionContext {
        pipeline: &pipeline,
        entities: &map,
        outcome_table: &table,
    };
    let mut rng = DeterministicRng::from_seed(101);
    let result = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap();
    assert_eq!(result.actor.total, 14);
    assert_eq!(result.actor.natural, Some(1));
    assert_eq!(result.margin, 0);
    assert_eq!(result.decision.band_index, 2);
}

#[test]
fn opposed_resolution_without_an_actor_roll_still_commits() {
    let mut fx = fixture();
    let second = fx.arena.insert(());
    let pipeline = int_pipeline(&fx);
    let table = four_band();
    let actor_roll = roll_request(fx.entity, RollSpec::None, roll_tag(&fx, 0), None, "");
    let opposed_roll = roll_request(second, dice("2d6"), roll_tag(&fx, 0), None, "roll-b");
    let request = resolve_request(actor_roll, Against::Opposed(opposed_roll), table.id());
    let stored = StatBlock::new();
    let other_stored = StatBlock::new();
    let tags = RulesTagSet::new();
    let other_tags = RulesTagSet::new();
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
    let before = rng.clone();
    let result = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap();
    assert_eq!(result.actor.total, 0);
    assert_eq!(result.against_value, 2);
    assert_eq!(result.margin, -2);
    assert_ne!(rng, before);
    assert!(!rng.has_stream("roll-a"));
    assert!(rng.has_stream("roll-b"));
}

// Failure precedence and RNG preservation.

#[test]
fn resolution_checks_run_in_contract_order() {
    let mut fx = fixture();
    let pipeline = int_pipeline(&fx);
    let table = four_band();
    // Removed ghosts are absent from every view map and unequal to every
    // live identity; fresh arenas would mint colliding positional ids.
    let ghost_actor = fx.arena.insert(());
    fx.arena.remove(ghost_actor);
    let ghost_defender = fx.arena.insert(());
    fx.arena.remove(ghost_defender);
    let stored = StatBlock::new();
    let tags = RulesTagSet::new();
    let mods: Vec<Modifier> = vec![];
    let (map, _) = single_context(fx.entity, &stored, &tags, &mods);
    let resolution = ResolutionContext {
        pipeline: &pipeline,
        entities: &map,
        outcome_table: &table,
    };
    let mut rng = DeterministicRng::from_seed(100);
    let before = rng.clone();
    // Table identity precedes actor existence.
    let roll = roll_request(ghost_actor, dice("1d6"), roll_tag(&fx, 0), None, "roll-a");
    let request = resolve_request(
        roll,
        Against::Dc {
            value: 1,
            owner: fx.entity,
            modifier_target: roll_tag(&fx, 1),
        },
        table_id(404),
    );
    let error = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidResolution);
    assert_eq!(error.location, "/resolution/outcome_table");
    assert_rng_untouched(&before, &rng, "roll-a");
    // Actor existence precedes defender existence.
    let request = resolve_request(
        roll_request(ghost_actor, dice("1d6"), roll_tag(&fx, 0), None, "roll-a"),
        Against::Dc {
            value: 1,
            owner: ghost_defender,
            modifier_target: roll_tag(&fx, 1),
        },
        table.id(),
    );
    let error = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap_err();
    assert_eq!(error.location, "/resolution/actor");
    assert_rng_untouched(&before, &rng, "roll-a");
    // Defender existence precedes roll-shape validation.
    let request = resolve_request(
        roll_request(fx.entity, dice("1d6"), roll_tag(&fx, 0), Some(9), "roll-a"),
        Against::Dc {
            value: 1,
            owner: ghost_defender,
            modifier_target: roll_tag(&fx, 1),
        },
        table.id(),
    );
    let error = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::MissingEntity);
    assert_eq!(error.location, "/resolution/against");
    assert_rng_untouched(&before, &rng, "roll-a");
    // Actor roll shape precedes opposing roll shape.
    let mut fx_two = fixture();
    let second = fx_two.arena.insert(());
    let pipeline_two = int_pipeline(&fx_two);
    let stored_two = StatBlock::new();
    let other_two = StatBlock::new();
    let tags_two = RulesTagSet::new();
    let other_tags_two = RulesTagSet::new();
    let mods_two: Vec<Modifier> = vec![];
    let other_mods_two: Vec<Modifier> = vec![];
    let mut map_two = BTreeMap::new();
    map_two.insert(
        fx_two.entity,
        context(fx_two.entity, &stored_two, &tags_two, &mods_two),
    );
    map_two.insert(
        second,
        context(second, &other_two, &other_tags_two, &other_mods_two),
    );
    let resolution_two = ResolutionContext {
        pipeline: &pipeline_two,
        entities: &map_two,
        outcome_table: &table,
    };
    let request = resolve_request(
        roll_request(
            fx_two.entity,
            dice("1d6"),
            roll_tag(&fx_two, 0),
            Some(9),
            "roll-a",
        ),
        Against::Opposed(roll_request(
            second,
            dice("1d6"),
            roll_tag(&fx_two, 0),
            Some(9),
            "roll-b",
        )),
        table.id(),
    );
    let error = crpg_rules::resolve(&request, &resolution_two, &mut rng).unwrap_err();
    assert_eq!(error.location, "/resolution/actor/roll/natural_die");
    assert_rng_untouched(&before, &rng, "roll-a");
    // An invalid opposing shape fails even though the actor would roll.
    let request = resolve_request(
        roll_request(
            fx_two.entity,
            dice("1d6"),
            roll_tag(&fx_two, 0),
            None,
            "roll-a",
        ),
        Against::Opposed(roll_request(
            second,
            dice("1d6"),
            roll_tag(&fx_two, 0),
            Some(9),
            "roll-b",
        )),
        table.id(),
    );
    let error = crpg_rules::resolve(&request, &resolution_two, &mut rng).unwrap_err();
    assert_eq!(error.location, "/resolution/against/roll/natural_die");
    assert_rng_untouched(&before, &rng, "roll-a");
}

#[test]
fn roll_shape_failures_use_roll_field_paths() {
    let fx = fixture();
    let pipeline = int_pipeline(&fx);
    let table = four_band();
    let stored = StatBlock::new();
    let tags = RulesTagSet::new();
    let mods: Vec<Modifier> = vec![];
    let (map, _) = single_context(fx.entity, &stored, &tags, &mods);
    let resolution = ResolutionContext {
        pipeline: &pipeline,
        entities: &map,
        outcome_table: &table,
    };
    let mut rng = DeterministicRng::from_seed(100);
    let before = rng.clone();
    // None takes no selector and an empty stream.
    let request = resolve_request(
        roll_request(fx.entity, RollSpec::None, roll_tag(&fx, 0), Some(0), ""),
        Against::Dc {
            value: 1,
            owner: fx.entity,
            modifier_target: roll_tag(&fx, 1),
        },
        table.id(),
    );
    let error = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap_err();
    assert_eq!(error.location, "/resolution/actor/roll/natural_die");
    let request = resolve_request(
        roll_request(fx.entity, RollSpec::None, roll_tag(&fx, 0), None, "roll-a"),
        Against::Dc {
            value: 1,
            owner: fx.entity,
            modifier_target: roll_tag(&fx, 1),
        },
        table.id(),
    );
    let error = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap_err();
    assert_eq!(error.location, "/resolution/actor/roll/stream");
    // Dice takes a valid nonempty stream.
    let request = resolve_request(
        roll_request(fx.entity, dice("1d6"), roll_tag(&fx, 0), None, ""),
        Against::Dc {
            value: 1,
            owner: fx.entity,
            modifier_target: roll_tag(&fx, 1),
        },
        table.id(),
    );
    let error = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidStream);
    assert_eq!(error.location, "/resolution/actor/roll/stream");
    assert_rng_untouched(&before, &rng, "roll-a");
}

#[test]
fn participant_contexts_validate_once_inascending_entity_order() {
    // Defender sorts below the actor here, so its context fails first with
    // the against prefix; swapping the malformations swaps the report.
    let mut fx = fixture();
    let second = fx.arena.insert(());
    let pipeline = int_pipeline(&fx);
    let table = four_band();
    let dup = uid(50);
    let defender_mods = vec![
        numeric_modifier(
            50,
            "sk",
            1,
            ModifierTarget::Dc(roll_tag(&fx, 1)),
            ModOp::Add(StatValue::Int(1)),
            common::TYPE_A,
            None,
            None,
            0,
        ),
        numeric_modifier(
            50,
            "sk",
            1,
            ModifierTarget::Dc(roll_tag(&fx, 1)),
            ModOp::Add(StatValue::Int(2)),
            common::TYPE_A,
            None,
            None,
            0,
        ),
    ];
    let actor_mods = vec![numeric_modifier(
        60,
        "sk",
        1,
        ModifierTarget::Roll(roll_tag(&fx, 0)),
        ModOp::Add(StatValue::Int(1)),
        "no-such-type",
        None,
        None,
        0,
    )];
    let stored = StatBlock::new();
    let other_stored = StatBlock::new();
    let tags = RulesTagSet::new();
    let other_tags = RulesTagSet::new();
    let mut map = BTreeMap::new();
    map.insert(
        second,
        context(second, &other_stored, &other_tags, &actor_mods),
    );
    map.insert(
        fx.entity,
        context(fx.entity, &stored, &tags, &defender_mods),
    );
    let resolution = ResolutionContext {
        pipeline: &pipeline,
        entities: &map,
        outcome_table: &table,
    };
    // Actor is `second`; defender (DC owner) is the lower-sorted entity.
    let request = resolve_request(
        roll_request(second, RollSpec::None, roll_tag(&fx, 0), None, ""),
        Against::Dc {
            value: 1,
            owner: fx.entity,
            modifier_target: roll_tag(&fx, 1),
        },
        table.id(),
    );
    let mut rng = DeterministicRng::from_seed(100);
    let before = rng.clone();
    let error = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::DuplicateModifier);
    assert_eq!(
        error.location,
        format!("/resolution/against/modifiers/{dup}")
    );
    assert_rng_untouched(&before, &rng, "roll-a");
    // Swap the malformations: the actor side now fails first.
    let mut map = BTreeMap::new();
    map.insert(
        second,
        context(second, &other_stored, &other_tags, &defender_mods),
    );
    map.insert(fx.entity, context(fx.entity, &stored, &tags, &actor_mods));
    let resolution = ResolutionContext {
        pipeline: &pipeline,
        entities: &map,
        outcome_table: &table,
    };
    let request = resolve_request(
        roll_request(fx.entity, RollSpec::None, roll_tag(&fx, 0), None, ""),
        Against::Dc {
            value: 1,
            owner: second,
            modifier_target: roll_tag(&fx, 1),
        },
        table.id(),
    );
    let error = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::UnknownModifierType);
    assert_eq!(
        error.location,
        format!("/resolution/actor/modifiers/{}/mod_type", uid(60))
    );
    assert_rng_untouched(&before, &rng, "roll-a");
}

#[test]
fn tag_unions_are_bounded_and_never_leak_into_dc() {
    let mut fx = fixture();
    let pipeline = int_pipeline(&fx);
    let table = four_band();
    // 1021 fresh request tags plus 4 entity tags overflow the tag bound.
    let mut request_tags = RulesTagSet::new();
    for index in 0..1021 {
        request_tags.insert(fx.interners.intern_tag(&format!("union-{index:04}")));
    }
    let mut roll = roll_request(fx.entity, RollSpec::None, roll_tag(&fx, 0), None, "");
    roll.tags = request_tags;
    let request = resolve_request(
        roll,
        Against::Dc {
            value: 1,
            owner: fx.entity,
            modifier_target: roll_tag(&fx, 1),
        },
        table.id(),
    );
    let stored = StatBlock::new();
    let tags = tag_set(&fx.tags);
    let mods: Vec<Modifier> = vec![];
    let (map, _) = single_context(fx.entity, &stored, &tags, &mods);
    let resolution = ResolutionContext {
        pipeline: &pipeline,
        entities: &map,
        outcome_table: &table,
    };
    let mut rng = DeterministicRng::from_seed(100);
    let before = rng.clone();
    let error = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::LimitExceeded);
    assert_eq!(error.location, "/resolution/actor/roll/tags");
    assert_rng_untouched(&before, &rng, "roll-a");
    // Actor request tags do not leak into DC conditions: a DC modifier gated
    // on an actor-only tag stays ConditionFalse.
    let mut fx = fixture();
    let actor_only = fx.interners.intern_tag("actor-only");
    let pipeline = int_pipeline(&fx);
    let mut roll = roll_request(fx.entity, RollSpec::None, roll_tag(&fx, 0), None, "");
    let mut request_tags = RulesTagSet::new();
    request_tags.insert(actor_only);
    roll.tags = request_tags;
    let request = resolve_request(
        roll,
        Against::Dc {
            value: 5,
            owner: fx.entity,
            modifier_target: roll_tag(&fx, 1),
        },
        table.id(),
    );
    let stored = StatBlock::new();
    let tags = RulesTagSet::new();
    let mods = vec![numeric_modifier(
        1,
        "sk",
        1,
        ModifierTarget::Dc(roll_tag(&fx, 1)),
        ModOp::Add(StatValue::Int(100)),
        common::TYPE_A,
        None,
        Some(crpg_rules::ConditionExpr::HasTag(actor_only)),
        0,
    )];
    let (map, _) = single_context(fx.entity, &stored, &tags, &mods);
    let resolution = ResolutionContext {
        pipeline: &pipeline,
        entities: &map,
        outcome_table: &table,
    };
    let mut rng = DeterministicRng::from_seed(100);
    let result = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap();
    assert_eq!(result.against_value, 5);
    let AgainstBreakdown::Dc(dc) = &result.against else {
        panic!("expected a DC breakdown");
    };
    assert!(matches!(
        dc.contributions[0].status,
        crpg_rules::ContributionStatus::ConditionFalse
    ));
}

#[test]
fn target_identity_is_contextual_only() {
    let mut fx = fixture();
    let second = fx.arena.insert(());
    let pipeline = int_pipeline(&fx);
    let table = four_band();
    let roll = roll_request(fx.entity, dice("1d6+2"), roll_tag(&fx, 0), None, "roll-a");
    let against = Against::Dc {
        value: 4,
        owner: fx.entity,
        modifier_target: roll_tag(&fx, 1),
    };
    let stored = StatBlock::new();
    let other_stored = StatBlock::new();
    let tags = RulesTagSet::new();
    let other_tags = RulesTagSet::new();
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
    let plain = resolve_request(roll.clone(), against.clone(), table.id());
    let mut rng = DeterministicRng::from_seed(100);
    let without = crpg_rules::resolve(&plain, &resolution, &mut rng).unwrap();
    // Naming a target changes nothing about the computation.
    let mut targeted = plain.clone();
    targeted.target = Some(second);
    let mut rng = DeterministicRng::from_seed(100);
    let with = crpg_rules::resolve(&targeted, &resolution, &mut rng).unwrap();
    assert_eq!(without, with);
    // A missing target view fails; an unused mismatched target view is ignored.
    let mut missing = plain.clone();
    let ghost_target = fx.arena.insert(());
    fx.arena.remove(ghost_target);
    missing.target = Some(ghost_target);
    let mut rng = DeterministicRng::from_seed(100);
    let error = crpg_rules::resolve(&missing, &resolution, &mut rng).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::MissingEntity);
    assert_eq!(error.location, "/resolution/target");
    let mut map_bad = BTreeMap::new();
    map_bad.insert(fx.entity, context(fx.entity, &stored, &tags, &mods));
    map_bad.insert(
        second,
        context(fx.entity, &other_stored, &other_tags, &other_mods),
    );
    let resolution_bad = ResolutionContext {
        pipeline: &pipeline,
        entities: &map_bad,
        outcome_table: &table,
    };
    let mut rng = DeterministicRng::from_seed(100);
    let ignored = crpg_rules::resolve(&targeted, &resolution_bad, &mut rng).unwrap();
    assert_eq!(ignored.actor.total, with.actor.total);
    assert_eq!(ignored.margin, with.margin);
}

#[test]
fn alternative_table_configuration_resolves_through_six_sided_dice() {
    // The custom configuration (faces 2/5, custom labels) drives a full
    // resolution through the same code as the standard configuration: seed
    // 100 draws face 2 on "roll-a", matching the shift-down rule.
    let fx = fixture();
    let pipeline = int_pipeline(&fx);
    let table = custom_table();
    let roll = roll_request(fx.entity, dice("1d6"), roll_tag(&fx, 0), Some(0), "roll-a");
    let request = resolve_request(
        roll,
        Against::Dc {
            value: 3,
            owner: fx.entity,
            modifier_target: roll_tag(&fx, 1),
        },
        table.id(),
    );
    let stored = StatBlock::new();
    let tags = RulesTagSet::new();
    let mods: Vec<Modifier> = vec![];
    let (map, _) = single_context(fx.entity, &stored, &tags, &mods);
    let resolution = ResolutionContext {
        pipeline: &pipeline,
        entities: &map,
        outcome_table: &table,
    };
    let mut rng = DeterministicRng::from_seed(100);
    let result = crpg_rules::resolve(&request, &resolution, &mut rng).unwrap();
    assert_eq!(result.actor.total, 2);
    assert_eq!(result.actor.natural, Some(2));
    assert_eq!(result.margin, -1);
    assert_eq!(result.decision.band_index, 0);
    assert_eq!(result.decision.natural_rule_index, Some(0));
    assert_eq!(result.decision.outcome, Outcome::Custom(1));
}
