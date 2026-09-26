//! Table-driven modifier coverage: 100+ named, genuinely distinct rows.
//!
//! Each row pins an independently specified expected value with exact
//! trace/contribution statuses and ordering, or an exact error code and
//! location. Numbers are hand-computed, never produced by the pipeline.

#[allow(dead_code)]
#[path = "common/mod.rs"]
mod common;

use std::collections::BTreeSet;

use crpg_core::{Fx16_16, StatId, Ulid};
use crpg_rules::{
    ConditionExpr, ContributionStatus, Expr, ModOp, Modifier, ModifierBreakdown, ModifierPipeline,
    RulesError, RulesErrorCode, StackingPolicy, StatKind, StatValue, TagSet,
};

use common::{
    block, context, def, enum_kind, enum_value, fixture, int_def, modifier, policies, raw_fx,
    tag_set, uid, Fixture, TYPE_A, TYPE_B, TYPE_C,
};

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

/// One Int stat (`alpha`) with `StackAll` over `TYPE_A`.
fn solo() -> (Fixture, ModifierPipeline) {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![int_def(fx.stats[0])],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    (fx, pipeline)
}

/// Queries `alpha` holding an `Int` base with an empty tag set.
fn ask(
    pipeline: &ModifierPipeline,
    fx: &Fixture,
    base: i32,
    mods: &[Modifier],
) -> Result<(StatValue, ModifierBreakdown), RulesError> {
    let stored = block(&[(fx.stats[0], StatValue::Int(base))]);
    let tags = TagSet::new();
    let ctx = context(fx.entity, &stored, &tags, mods);
    pipeline.query(fx.entity, fx.stats[0], &ctx)
}

/// Extracts ordered `(id, status)` pairs from one trace.
fn contribs(breakdown: &ModifierBreakdown, stat: StatId) -> Vec<(Ulid, ContributionStatus)> {
    breakdown.traces[&stat]
        .contributions
        .iter()
        .map(|entry| (entry.modifier.id, entry.status.clone()))
        .collect()
}

fn applied(before: StatValue, after: StatValue) -> ContributionStatus {
    ContributionStatus::Applied { before, after }
}

fn int(value: i32) -> StatValue {
    StatValue::Int(value)
}

/// An `Add(Int)` modifier on `TYPE_A` with default source and priority.
fn add(id: u128, stat: StatId, amount: i32) -> Modifier {
    modifier(
        id,
        "sk",
        id,
        stat,
        ModOp::Add(int(amount)),
        TYPE_A,
        None,
        None,
        0,
    )
}

fn expect_err(
    result: Result<(StatValue, ModifierBreakdown), RulesError>,
    code: RulesErrorCode,
    location: &str,
) {
    let error = result.unwrap_err();
    assert_eq!(error.code, code);
    assert_eq!(error.location, location);
}

// ---------------------------------------------------------------------------
// Section A: StackAll basics, phases, ordering, integer arithmetic.
// ---------------------------------------------------------------------------

fn r_set_int() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        1,
        "sk",
        1,
        target,
        ModOp::Set(int(20)),
        TYPE_A,
        None,
        None,
        0,
    )];
    let (value, breakdown) = ask(&pipeline, &fx, 10, &mods).unwrap();
    assert_eq!(value, int(20));
    assert_eq!(breakdown.entity, fx.entity);
    assert_eq!(breakdown.stat, target);
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(1), applied(int(10), int(20)))]
    );
}

fn r_set_fixed() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Fixed, None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], StatValue::Fixed(raw_fx(65536)))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        1,
        "sk",
        1,
        fx.stats[0],
        ModOp::Set(StatValue::Fixed(raw_fx(131072))),
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (value, breakdown) = pipeline.query(fx.entity, fx.stats[0], &ctx).unwrap();
    assert_eq!(value, StatValue::Fixed(raw_fx(131072)));
    assert_eq!(
        contribs(&breakdown, fx.stats[0]),
        vec![(
            uid(1),
            applied(
                StatValue::Fixed(raw_fx(65536)),
                StatValue::Fixed(raw_fx(131072))
            )
        )]
    );
}

fn r_set_bool() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Bool, None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], StatValue::Bool(true))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        1,
        "sk",
        1,
        fx.stats[0],
        ModOp::Set(StatValue::Bool(false)),
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (value, breakdown) = pipeline.query(fx.entity, fx.stats[0], &ctx).unwrap();
    assert_eq!(value, StatValue::Bool(false));
    assert_eq!(
        contribs(&breakdown, fx.stats[0]),
        vec![(
            uid(1),
            applied(StatValue::Bool(true), StatValue::Bool(false))
        )]
    );
}

fn r_set_enum() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], enum_kind("ed", &["ex", "ey"]), None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], enum_value("ed", "ex"))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        1,
        "sk",
        1,
        fx.stats[0],
        ModOp::Set(enum_value("ed", "ey")),
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (value, breakdown) = pipeline.query(fx.entity, fx.stats[0], &ctx).unwrap();
    assert_eq!(value, enum_value("ed", "ey"));
    assert_eq!(
        contribs(&breakdown, fx.stats[0]),
        vec![(
            uid(1),
            applied(enum_value("ed", "ex"), enum_value("ed", "ey"))
        )]
    );
}

fn r_set_tags() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Tags, None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let before = StatValue::Tags(tag_set(&fx.tags[..1]));
    let after = StatValue::Tags(tag_set(&fx.tags[1..3]));
    let stored = block(&[(fx.stats[0], before.clone())]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        1,
        "sk",
        1,
        fx.stats[0],
        ModOp::Set(after.clone()),
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (value, breakdown) = pipeline.query(fx.entity, fx.stats[0], &ctx).unwrap();
    assert_eq!(value, after);
    assert_eq!(
        contribs(&breakdown, fx.stats[0]),
        vec![(uid(1), applied(before, after))]
    );
}

fn r_tags_set_empty() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Tags, None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let before = StatValue::Tags(tag_set(&fx.tags[..1]));
    let after = StatValue::Tags(TagSet::new());
    let stored = block(&[(fx.stats[0], before.clone())]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        1,
        "sk",
        1,
        fx.stats[0],
        ModOp::Set(after.clone()),
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (value, _) = pipeline.query(fx.entity, fx.stats[0], &ctx).unwrap();
    assert_eq!(value, after);
}

fn r_add_int_sum() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![add(1, target, 3), add(2, target, 4)];
    let (value, breakdown) = ask(&pipeline, &fx, 10, &mods).unwrap();
    assert_eq!(value, int(17));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), applied(int(10), int(13))),
            (uid(2), applied(int(13), int(17))),
        ]
    );
}

fn r_add_fixed() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Fixed, None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], StatValue::Fixed(raw_fx(65536)))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        1,
        "sk",
        1,
        fx.stats[0],
        ModOp::Add(StatValue::Fixed(raw_fx(32768))),
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (value, breakdown) = pipeline.query(fx.entity, fx.stats[0], &ctx).unwrap();
    assert_eq!(value, StatValue::Fixed(raw_fx(98304)));
    assert_eq!(
        contribs(&breakdown, fx.stats[0]),
        vec![(
            uid(1),
            applied(
                StatValue::Fixed(raw_fx(65536)),
                StatValue::Fixed(raw_fx(98304))
            )
        )]
    );
}

fn r_multiply_int() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        1,
        "sk",
        1,
        target,
        ModOp::Multiply(raw_fx(98304)),
        TYPE_A,
        None,
        None,
        0,
    )];
    let (value, breakdown) = ask(&pipeline, &fx, 10, &mods).unwrap();
    assert_eq!(value, int(15));
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(1), applied(int(10), int(15)))]
    );
}

fn r_multiply_fixed() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Fixed, None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], StatValue::Fixed(raw_fx(131072)))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        1,
        "sk",
        1,
        fx.stats[0],
        ModOp::Multiply(raw_fx(98304)),
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (value, _) = pipeline.query(fx.entity, fx.stats[0], &ctx).unwrap();
    assert_eq!(value, StatValue::Fixed(raw_fx(196608)));
}

fn r_clamp_below() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        1,
        "sk",
        1,
        target,
        ModOp::Clamp {
            min: int(0),
            max: int(10),
        },
        TYPE_A,
        None,
        None,
        0,
    )];
    let (value, breakdown) = ask(&pipeline, &fx, -5, &mods).unwrap();
    assert_eq!(value, int(0));
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(1), applied(int(-5), int(0)))]
    );
}

fn r_clamp_within() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        1,
        "sk",
        1,
        target,
        ModOp::Clamp {
            min: int(0),
            max: int(10),
        },
        TYPE_A,
        None,
        None,
        0,
    )];
    let (value, breakdown) = ask(&pipeline, &fx, 5, &mods).unwrap();
    assert_eq!(value, int(5));
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(1), applied(int(5), int(5)))]
    );
}

fn r_clamp_above() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        1,
        "sk",
        1,
        target,
        ModOp::Clamp {
            min: int(0),
            max: int(10),
        },
        TYPE_A,
        None,
        None,
        0,
    )];
    let (value, breakdown) = ask(&pipeline, &fx, 50, &mods).unwrap();
    assert_eq!(value, int(10));
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(1), applied(int(50), int(10)))]
    );
}

fn r_oracle_transitions() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![
        add(1, target, 3),
        modifier(
            2,
            "sk",
            2,
            target,
            ModOp::Multiply(raw_fx(98304)),
            TYPE_A,
            None,
            None,
            0,
        ),
        modifier(
            3,
            "sk",
            3,
            target,
            ModOp::Clamp {
                min: int(0),
                max: int(18),
            },
            TYPE_A,
            None,
            None,
            0,
        ),
    ];
    let (value, breakdown) = ask(&pipeline, &fx, 10, &mods).unwrap();
    assert_eq!(value, int(18));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), applied(int(10), int(13))),
            (uid(2), applied(int(13), int(19))),
            (uid(3), applied(int(19), int(18))),
        ]
    );
}

fn r_negative_floor_scale() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        1,
        "sk",
        1,
        target,
        ModOp::Multiply(raw_fx(32768)),
        TYPE_A,
        None,
        None,
        0,
    )];
    let (value, _) = ask(&pipeline, &fx, -3, &mods).unwrap();
    assert_eq!(value, int(-2));
}

fn r_full_range_scale() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        1,
        "sk",
        1,
        target,
        ModOp::Multiply(raw_fx(65536)),
        TYPE_A,
        None,
        None,
        0,
    )];
    let (value, _) = ask(&pipeline, &fx, 100000, &mods).unwrap();
    assert_eq!(value, int(100000));
}

fn r_zero_factor() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        1,
        "sk",
        1,
        target,
        ModOp::Multiply(raw_fx(0)),
        TYPE_A,
        None,
        None,
        0,
    )];
    let (value, breakdown) = ask(&pipeline, &fx, 7, &mods).unwrap();
    assert_eq!(value, int(0));
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(1), applied(int(7), int(0)))]
    );
}

fn r_negative_factor() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        1,
        "sk",
        1,
        target,
        ModOp::Multiply(raw_fx(-65536)),
        TYPE_A,
        None,
        None,
        0,
    )];
    let (value, _) = ask(&pipeline, &fx, 10, &mods).unwrap();
    assert_eq!(value, int(-10));
}

fn r_int_add_saturate_top() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![add(1, target, 1)];
    let (value, breakdown) = ask(&pipeline, &fx, i32::MAX, &mods).unwrap();
    assert_eq!(value, int(i32::MAX));
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(1), applied(int(i32::MAX), int(i32::MAX)))]
    );
}

fn r_int_add_saturate_bottom() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![add(1, target, -1)];
    let (value, _) = ask(&pipeline, &fx, i32::MIN, &mods).unwrap();
    assert_eq!(value, int(i32::MIN));
}

fn r_min_times_neg_one() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        1,
        "sk",
        1,
        target,
        ModOp::Multiply(raw_fx(-65536)),
        TYPE_A,
        None,
        None,
        0,
    )];
    let (value, breakdown) = ask(&pipeline, &fx, i32::MIN, &mods).unwrap();
    assert_eq!(value, int(i32::MAX));
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(1), applied(int(i32::MIN), int(i32::MAX)))]
    );
}

fn r_phase_precedence_despite_input_order() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    // Add arrives first in the slice; Set still applies first.
    let mods = vec![
        add(1, target, 3),
        modifier(
            2,
            "sk",
            2,
            target,
            ModOp::Set(int(20)),
            TYPE_A,
            None,
            None,
            0,
        ),
    ];
    let (value, breakdown) = ask(&pipeline, &fx, 10, &mods).unwrap();
    assert_eq!(value, int(23));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(2), applied(int(10), int(20))),
            (uid(1), applied(int(20), int(23))),
        ]
    );
}

fn r_cross_type_global_phase_order() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![int_def(fx.stats[0])],
        policies(&[
            (TYPE_A, StackingPolicy::StackAll),
            (TYPE_B, StackingPolicy::StackAll),
        ]),
    )
    .unwrap();
    let target = fx.stats[0];
    // High-priority Add on one type still runs after a low-priority Set.
    let mods = vec![
        modifier(
            1,
            "sk",
            1,
            target,
            ModOp::Add(int(5)),
            TYPE_A,
            None,
            None,
            100,
        ),
        modifier(
            2,
            "sk",
            2,
            target,
            ModOp::Set(int(0)),
            TYPE_B,
            None,
            None,
            -100,
        ),
    ];
    let stored = block(&[(target, int(10))]);
    let tags = TagSet::new();
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (value, breakdown) = pipeline.query(fx.entity, target, &ctx).unwrap();
    assert_eq!(value, int(5));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(2), applied(int(10), int(0))),
            (uid(1), applied(int(0), int(5))),
        ]
    );
}

fn r_priority_orders_within_phase() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![
        modifier(
            1,
            "sk",
            1,
            target,
            ModOp::Add(int(1)),
            TYPE_A,
            None,
            None,
            9,
        ),
        modifier(
            2,
            "sk",
            2,
            target,
            ModOp::Add(int(10)),
            TYPE_A,
            None,
            None,
            5,
        ),
    ];
    let (value, breakdown) = ask(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(11));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(2), applied(int(0), int(10))),
            (uid(1), applied(int(10), int(11))),
        ]
    );
}

fn r_id_breaks_priority_ties() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![add(5, target, 1), add(3, target, 10)];
    let (value, breakdown) = ask(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(11));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(3), applied(int(0), int(10))),
            (uid(5), applied(int(10), int(11))),
        ]
    );
}

fn r_repeated_sets_last_wins() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![
        modifier(
            1,
            "sk",
            1,
            target,
            ModOp::Set(int(1)),
            TYPE_A,
            None,
            None,
            0,
        ),
        modifier(
            2,
            "sk",
            2,
            target,
            ModOp::Set(int(2)),
            TYPE_A,
            None,
            None,
            0,
        ),
    ];
    let (value, breakdown) = ask(&pipeline, &fx, 10, &mods).unwrap();
    assert_eq!(value, int(2));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), applied(int(10), int(1))),
            (uid(2), applied(int(1), int(2))),
        ]
    );
}

fn r_higher_priority_set_wins() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![
        modifier(
            1,
            "sk",
            1,
            target,
            ModOp::Set(int(1)),
            TYPE_A,
            None,
            None,
            5,
        ),
        modifier(
            2,
            "sk",
            2,
            target,
            ModOp::Set(int(2)),
            TYPE_A,
            None,
            None,
            0,
        ),
    ];
    let (value, _) = ask(&pipeline, &fx, 10, &mods).unwrap();
    assert_eq!(value, int(1));
}

fn r_saturated_still_applied() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![add(1, target, 5), add(2, target, -1)];
    let (value, breakdown) = ask(&pipeline, &fx, i32::MAX, &mods).unwrap();
    assert_eq!(value, int(i32::MAX - 1));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), applied(int(i32::MAX), int(i32::MAX))),
            (uid(2), applied(int(i32::MAX), int(i32::MAX - 1))),
        ]
    );
}

fn r_canonical_saturation_oracle_both_orders() {
    // Either input ordering folds canonically to MAX - 1.
    for order in [vec![1_u128, 2_u128], vec![2_u128, 1_u128]] {
        let (fx, pipeline) = solo();
        let target = fx.stats[0];
        let mods: Vec<Modifier> = order
            .iter()
            .map(|id| {
                let amount = if *id == 1 { 1 } else { -1 };
                add(*id, target, amount)
            })
            .collect();
        let (value, _) = ask(&pipeline, &fx, i32::MAX, &mods).unwrap();
        assert_eq!(value, int(i32::MAX - 1));
    }
}

fn r_removal_restores_exact() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let stored = block(&[(target, int(i32::MAX))]);
    let tags = TagSet::new();
    let base_mods = vec![add(2, target, -1)];
    let ctx = context(fx.entity, &stored, &tags, &base_mods);
    let original = pipeline.query(fx.entity, target, &ctx).unwrap();
    assert_eq!(original.0, int(i32::MAX - 1));
    // Augment from a fresh source, then remove exactly that source.
    let mut augmented_mods = base_mods.clone();
    augmented_mods.push(modifier(
        1,
        "extra-kind",
        41,
        target,
        ModOp::Add(int(1)),
        TYPE_A,
        None,
        None,
        0,
    ));
    let ctx = context(fx.entity, &stored, &tags, &augmented_mods);
    let augmented = pipeline.query(fx.entity, target, &ctx).unwrap();
    assert!(augmented.1.traces[&target].contributions.len() == 2);
    let surviving: Vec<Modifier> = augmented_mods
        .iter()
        .filter(|modifier| !(modifier.source.kind == "extra-kind" && modifier.source.id == uid(41)))
        .cloned()
        .collect();
    assert_eq!(surviving, base_mods);
    let ctx = context(fx.entity, &stored, &tags, &surviving);
    let restored = pipeline.query(fx.entity, target, &ctx).unwrap();
    assert_eq!(restored, original);
}

fn r_stackall_zero_add_applies() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![add(1, target, 0)];
    let (value, breakdown) = ask(&pipeline, &fx, 4, &mods).unwrap();
    assert_eq!(value, int(4));
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(1), applied(int(4), int(4)))]
    );
}

fn r_empty_modifiers() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let (value, breakdown) = ask(&pipeline, &fx, 9, &[]).unwrap();
    assert_eq!(value, int(9));
    let trace = &breakdown.traces[&target];
    assert_eq!(trace.base, int(9));
    assert_eq!(trace.value, int(9));
    assert!(trace.dependencies.is_empty());
    assert!(trace.contributions.is_empty());
    assert_eq!(breakdown.traces.len(), 1);
}

fn r_sequential_disjoint_clamps() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![
        modifier(
            1,
            "sk",
            1,
            target,
            ModOp::Clamp {
                min: int(0),
                max: int(10),
            },
            TYPE_A,
            None,
            None,
            0,
        ),
        modifier(
            2,
            "sk",
            2,
            target,
            ModOp::Clamp {
                min: int(20),
                max: int(30),
            },
            TYPE_A,
            None,
            None,
            1,
        ),
    ];
    let (value, breakdown) = ask(&pipeline, &fx, 5, &mods).unwrap();
    assert_eq!(value, int(20));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), applied(int(5), int(5))),
            (uid(2), applied(int(5), int(20))),
        ]
    );
}

fn r_contradictory_clamps_later_wins() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![
        modifier(
            1,
            "sk",
            1,
            target,
            ModOp::Clamp {
                min: int(0),
                max: int(5),
            },
            TYPE_A,
            None,
            None,
            0,
        ),
        modifier(
            2,
            "sk",
            2,
            target,
            ModOp::Clamp {
                min: int(10),
                max: int(20),
            },
            TYPE_A,
            None,
            None,
            0,
        ),
    ];
    let (value, _) = ask(&pipeline, &fx, 100, &mods).unwrap();
    assert_eq!(value, int(10));
}

fn r_equal_bounds_pin() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        1,
        "sk",
        1,
        target,
        ModOp::Clamp {
            min: int(3),
            max: int(3),
        },
        TYPE_A,
        None,
        None,
        0,
    )];
    let (value, _) = ask(&pipeline, &fx, 7, &mods).unwrap();
    assert_eq!(value, int(3));
}

fn r_int_sub_via_negative_add() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let (value, _) = ask(&pipeline, &fx, 10, &[add(1, target, -4)]).unwrap();
    assert_eq!(value, int(6));
}

fn r_multiply_then_add_phase_order() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![
        modifier(
            1,
            "sk",
            1,
            target,
            ModOp::Multiply(raw_fx(131072)),
            TYPE_A,
            None,
            None,
            0,
        ),
        modifier(
            2,
            "sk",
            2,
            target,
            ModOp::Add(int(1)),
            TYPE_A,
            None,
            None,
            0,
        ),
    ];
    let (value, breakdown) = ask(&pipeline, &fx, 5, &mods).unwrap();
    assert_eq!(value, int(12));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(2), applied(int(5), int(6))),
            (uid(1), applied(int(6), int(12))),
        ]
    );
}

fn r_unrelated_targets_absent_from_trace() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![int_def(fx.stats[0]), int_def(fx.stats[1])],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], int(10)), (fx.stats[1], int(1))]);
    let tags = TagSet::new();
    let mods = vec![add(1, fx.stats[1], 100), add(2, fx.stats[0], 1)];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (first, breakdown) = pipeline.query(fx.entity, fx.stats[0], &ctx).unwrap();
    assert_eq!(first, int(11));
    assert_eq!(
        contribs(&breakdown, fx.stats[0]),
        vec![(uid(2), applied(int(10), int(11)))]
    );
    assert!(!breakdown.traces.contains_key(&fx.stats[1]));
    let (second, breakdown) = pipeline.query(fx.entity, fx.stats[1], &ctx).unwrap();
    assert_eq!(second, int(101));
    assert_eq!(
        contribs(&breakdown, fx.stats[1]),
        vec![(uid(1), applied(int(1), int(101)))]
    );
}

fn r_zero_ulid_accepted() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let (value, _) = ask(&pipeline, &fx, 1, &[add(0, target, 2)]).unwrap();
    assert_eq!(value, int(3));
}

fn r_fixed_add_saturates() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Fixed, None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], StatValue::Fixed(Fx16_16::MAX))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        1,
        "sk",
        1,
        fx.stats[0],
        ModOp::Add(StatValue::Fixed(Fx16_16::EPSILON)),
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (value, breakdown) = pipeline.query(fx.entity, fx.stats[0], &ctx).unwrap();
    assert_eq!(value, StatValue::Fixed(Fx16_16::MAX));
    assert_eq!(
        contribs(&breakdown, fx.stats[0]),
        vec![(
            uid(1),
            applied(
                StatValue::Fixed(Fx16_16::MAX),
                StatValue::Fixed(Fx16_16::MAX)
            )
        )]
    );
}

fn r_fixed_clamp_bounds() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Fixed, None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], StatValue::Fixed(raw_fx(327680)))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        1,
        "sk",
        1,
        fx.stats[0],
        ModOp::Clamp {
            min: StatValue::Fixed(raw_fx(65536)),
            max: StatValue::Fixed(raw_fx(131072)),
        },
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (value, _) = pipeline.query(fx.entity, fx.stats[0], &ctx).unwrap();
    assert_eq!(value, StatValue::Fixed(raw_fx(131072)));
}

fn r_priority_extremes_order() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![
        modifier(
            1,
            "sk",
            1,
            target,
            ModOp::Add(int(1)),
            TYPE_A,
            None,
            None,
            i16::MAX,
        ),
        modifier(
            2,
            "sk",
            2,
            target,
            ModOp::Add(int(2)),
            TYPE_A,
            None,
            None,
            i16::MIN,
        ),
    ];
    let (value, breakdown) = ask(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(3));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(2), applied(int(0), int(2))),
            (uid(1), applied(int(2), int(3))),
        ]
    );
}

// ---------------------------------------------------------------------------
// Section B: HighestBonusWorstPenalty selection and rejection.
// ---------------------------------------------------------------------------

/// One Int stat (`alpha`) with the bonus/penalty policy over `TYPE_B`.
fn bonus() -> (Fixture, ModifierPipeline) {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![int_def(fx.stats[0])],
        policies(&[(TYPE_B, StackingPolicy::HighestBonusWorstPenalty)]),
    )
    .unwrap();
    (fx, pipeline)
}

fn bonus_add(id: u128, stat: StatId, amount: i32, priority: i16) -> Modifier {
    modifier(
        id,
        "sk",
        id,
        stat,
        ModOp::Add(int(amount)),
        TYPE_B,
        None,
        None,
        priority,
    )
}

fn ask_bonus(
    pipeline: &ModifierPipeline,
    fx: &Fixture,
    base: i32,
    mods: &[Modifier],
) -> Result<(StatValue, ModifierBreakdown), RulesError> {
    let stored = block(&[(fx.stats[0], int(base))]);
    let tags = TagSet::new();
    let ctx = context(fx.entity, &stored, &tags, mods);
    pipeline.query(fx.entity, fx.stats[0], &ctx)
}

fn r_bonus_keeps_both_signs() {
    let (fx, pipeline) = bonus();
    let target = fx.stats[0];
    let mods = vec![bonus_add(1, target, 5, 0), bonus_add(2, target, -3, 0)];
    let (value, breakdown) = ask_bonus(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(2));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), applied(int(0), int(5))),
            (uid(2), applied(int(5), int(2))),
        ]
    );
}

fn r_bonus_pos_competition() {
    let (fx, pipeline) = bonus();
    let target = fx.stats[0];
    let mods = vec![bonus_add(1, target, 5, 0), bonus_add(2, target, 9, 0)];
    let (value, breakdown) = ask_bonus(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(9));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), ContributionStatus::Suppressed { winner: uid(2) }),
            (uid(2), applied(int(0), int(9))),
        ]
    );
}

fn r_bonus_neg_competition() {
    let (fx, pipeline) = bonus();
    let target = fx.stats[0];
    let mods = vec![bonus_add(1, target, -5, 0), bonus_add(2, target, -9, 0)];
    let (value, breakdown) = ask_bonus(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(-9));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), ContributionStatus::Suppressed { winner: uid(2) }),
            (uid(2), applied(int(0), int(-9))),
        ]
    );
}

fn r_bonus_pos_tie_priority() {
    let (fx, pipeline) = bonus();
    let target = fx.stats[0];
    let mods = vec![bonus_add(1, target, 5, 3), bonus_add(2, target, 5, 7)];
    let (value, breakdown) = ask_bonus(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(5));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), ContributionStatus::Suppressed { winner: uid(2) }),
            (uid(2), applied(int(0), int(5))),
        ]
    );
}

fn r_bonus_pos_tie_id() {
    let (fx, pipeline) = bonus();
    let target = fx.stats[0];
    let mods = vec![bonus_add(4, target, 5, 0), bonus_add(6, target, 5, 0)];
    let (value, breakdown) = ask_bonus(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(5));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(4), ContributionStatus::Suppressed { winner: uid(6) }),
            (uid(6), applied(int(0), int(5))),
        ]
    );
}

fn r_bonus_neg_tie() {
    let (fx, pipeline) = bonus();
    let target = fx.stats[0];
    let mods = vec![bonus_add(4, target, -5, 0), bonus_add(9, target, -5, 9)];
    let (value, breakdown) = ask_bonus(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(-5));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(4), ContributionStatus::Suppressed { winner: uid(9) }),
            (uid(9), applied(int(0), int(-5))),
        ]
    );
}

fn r_bonus_zero_is_zeroadd() {
    let (fx, pipeline) = bonus();
    let target = fx.stats[0];
    let mods = vec![bonus_add(1, target, 0, 0), bonus_add(2, target, 4, 0)];
    let (value, breakdown) = ask_bonus(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(4));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), ContributionStatus::ZeroAdd),
            (uid(2), applied(int(0), int(4))),
        ]
    );
}

fn r_bonus_all_zero() {
    let (fx, pipeline) = bonus();
    let target = fx.stats[0];
    let mods = vec![bonus_add(1, target, 0, 0), bonus_add(2, target, 0, 0)];
    let (value, breakdown) = ask_bonus(&pipeline, &fx, 3, &mods).unwrap();
    assert_eq!(value, int(3));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), ContributionStatus::ZeroAdd),
            (uid(2), ContributionStatus::ZeroAdd),
        ]
    );
}

fn r_bonus_false_winner_candidate() {
    let (fx, pipeline) = bonus();
    let target = fx.stats[0];
    let hidden = modifier(
        1,
        "sk",
        1,
        target,
        ModOp::Add(int(100)),
        TYPE_B,
        None,
        Some(ConditionExpr::HasTag(fx.tags[3])),
        0,
    );
    let mods = vec![hidden, bonus_add(2, target, 5, 0)];
    let stored = block(&[(target, int(0))]);
    let tags = tag_set(&fx.tags[..1]);
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (value, breakdown) = pipeline.query(fx.entity, target, &ctx).unwrap();
    assert_eq!(value, int(5));
    // The false condition wins precedence: no suppression, no selection.
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), ContributionStatus::ConditionFalse),
            (uid(2), applied(int(0), int(5))),
        ]
    );
}

fn r_illegal_bonus_set() {
    let (fx, pipeline) = bonus();
    let target = fx.stats[0];
    let mods = vec![modifier(
        7,
        "sk",
        7,
        target,
        ModOp::Set(int(1)),
        TYPE_B,
        None,
        None,
        0,
    )];
    expect_err(
        ask_bonus(&pipeline, &fx, 0, &mods),
        RulesErrorCode::InvalidOperation,
        &format!("/modifiers/{}/op", uid(7)),
    );
}

fn r_illegal_bonus_multiply() {
    let (fx, pipeline) = bonus();
    let target = fx.stats[0];
    let mods = vec![modifier(
        7,
        "sk",
        7,
        target,
        ModOp::Multiply(raw_fx(65536)),
        TYPE_B,
        None,
        None,
        0,
    )];
    expect_err(
        ask_bonus(&pipeline, &fx, 0, &mods),
        RulesErrorCode::InvalidOperation,
        &format!("/modifiers/{}/op", uid(7)),
    );
}

fn r_illegal_bonus_clamp() {
    let (fx, pipeline) = bonus();
    let target = fx.stats[0];
    let mods = vec![modifier(
        7,
        "sk",
        7,
        target,
        ModOp::Clamp {
            min: int(0),
            max: int(1),
        },
        TYPE_B,
        None,
        None,
        0,
    )];
    expect_err(
        ask_bonus(&pipeline, &fx, 0, &mods),
        RulesErrorCode::InvalidOperation,
        &format!("/modifiers/{}/op", uid(7)),
    );
}

// ---------------------------------------------------------------------------
// Section C: HighestPriorityPerName selection and rejection.
// ---------------------------------------------------------------------------

/// One Int stat (`alpha`) with the per-name policy over `TYPE_C`.
fn pername() -> (Fixture, ModifierPipeline) {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![int_def(fx.stats[0])],
        policies(&[(TYPE_C, StackingPolicy::HighestPriorityPerName)]),
    )
    .unwrap();
    (fx, pipeline)
}

fn named(id: u128, stat: StatId, op: ModOp, name: Option<&str>, priority: i16) -> Modifier {
    modifier(id, "sk", id, stat, op, TYPE_C, name, None, priority)
}

fn ask_pername(
    pipeline: &ModifierPipeline,
    fx: &Fixture,
    base: i32,
    mods: &[Modifier],
) -> Result<(StatValue, ModifierBreakdown), RulesError> {
    let stored = block(&[(fx.stats[0], int(base))]);
    let tags = TagSet::new();
    let ctx = context(fx.entity, &stored, &tags, mods);
    pipeline.query(fx.entity, fx.stats[0], &ctx)
}

fn r_pername_distinct_names_both_apply() {
    let (fx, pipeline) = pername();
    let target = fx.stats[0];
    let mods = vec![
        named(1, target, ModOp::Add(int(3)), Some("n1"), 0),
        named(2, target, ModOp::Add(int(4)), Some("n2"), 0),
    ];
    let (value, breakdown) = ask_pername(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(7));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), applied(int(0), int(3))),
            (uid(2), applied(int(3), int(7))),
        ]
    );
}

fn r_pername_same_name_suppresses() {
    let (fx, pipeline) = pername();
    let target = fx.stats[0];
    let mods = vec![
        named(1, target, ModOp::Add(int(3)), Some("n"), 0),
        named(2, target, ModOp::Add(int(4)), Some("n"), 0),
    ];
    let (value, breakdown) = ask_pername(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(4));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), ContributionStatus::Suppressed { winner: uid(2) }),
            (uid(2), applied(int(0), int(4))),
        ]
    );
}

fn r_pername_priority_beats_id() {
    let (fx, pipeline) = pername();
    let target = fx.stats[0];
    let mods = vec![
        named(1, target, ModOp::Add(int(3)), Some("n"), 9),
        named(2, target, ModOp::Add(int(4)), Some("n"), 0),
    ];
    let (value, breakdown) = ask_pername(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(3));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(2), ContributionStatus::Suppressed { winner: uid(1) }),
            (uid(1), applied(int(0), int(3))),
        ]
    );
}

fn r_pername_across_sources() {
    let (fx, pipeline) = pername();
    let target = fx.stats[0];
    let first = modifier(
        1,
        "kind-one",
        11,
        target,
        ModOp::Add(int(3)),
        TYPE_C,
        Some("shared"),
        None,
        0,
    );
    let second = modifier(
        2,
        "kind-two",
        22,
        target,
        ModOp::Add(int(4)),
        TYPE_C,
        Some("shared"),
        None,
        0,
    );
    let mods = vec![first, second];
    let (value, breakdown) = ask_pername(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(4));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), ContributionStatus::Suppressed { winner: uid(2) }),
            (uid(2), applied(int(0), int(4))),
        ]
    );
}

fn r_pername_phase_isolation() {
    let (fx, pipeline) = pername();
    let target = fx.stats[0];
    let mods = vec![
        named(1, target, ModOp::Set(int(50)), Some("n"), 0),
        named(2, target, ModOp::Add(int(5)), Some("n"), 0),
    ];
    let (value, breakdown) = ask_pername(&pipeline, &fx, 10, &mods).unwrap();
    assert_eq!(value, int(55));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), applied(int(10), int(50))),
            (uid(2), applied(int(50), int(55))),
        ]
    );
}

fn r_pername_zero_add_applies() {
    let (fx, pipeline) = pername();
    let target = fx.stats[0];
    let mods = vec![named(1, target, ModOp::Add(int(0)), Some("n"), 0)];
    let (value, breakdown) = ask_pername(&pipeline, &fx, 7, &mods).unwrap();
    assert_eq!(value, int(7));
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(1), applied(int(7), int(7)))]
    );
}

fn r_pername_missing_name_rejected() {
    let (fx, pipeline) = pername();
    let target = fx.stats[0];
    let mods = vec![named(8, target, ModOp::Add(int(1)), None, 0)];
    expect_err(
        ask_pername(&pipeline, &fx, 0, &mods),
        RulesErrorCode::InvalidOperation,
        &format!("/modifiers/{}/name", uid(8)),
    );
}

fn r_pername_case_sensitive() {
    let (fx, pipeline) = pername();
    let target = fx.stats[0];
    let mods = vec![
        named(1, target, ModOp::Add(int(3)), Some("Buff"), 0),
        named(2, target, ModOp::Add(int(4)), Some("buff"), 0),
    ];
    let (value, _) = ask_pername(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(7));
}

fn r_pername_set_suppression() {
    let (fx, pipeline) = pername();
    let target = fx.stats[0];
    let mods = vec![
        named(1, target, ModOp::Set(int(1)), Some("n"), 0),
        named(2, target, ModOp::Set(int(2)), Some("n"), 0),
    ];
    let (value, breakdown) = ask_pername(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(2));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), ContributionStatus::Suppressed { winner: uid(2) }),
            (uid(2), applied(int(0), int(2))),
        ]
    );
}

fn r_pername_order_invariance() {
    // Either input order selects the greater id and folds identically.
    for order in [vec![2_u128, 7_u128], vec![7_u128, 2_u128]] {
        let (fx, pipeline) = pername();
        let target = fx.stats[0];
        let mods: Vec<Modifier> = order
            .iter()
            .map(|id| {
                let amount = if *id == 2 { 4 } else { 3 };
                named(*id, target, ModOp::Add(int(amount)), Some("n"), 0)
            })
            .collect();
        let (value, breakdown) = ask_pername(&pipeline, &fx, 0, &mods).unwrap();
        assert_eq!(value, int(3));
        assert_eq!(
            contribs(&breakdown, target),
            vec![
                (uid(2), ContributionStatus::Suppressed { winner: uid(7) }),
                (uid(7), applied(int(0), int(3))),
            ]
        );
    }
}

// ---------------------------------------------------------------------------
// Section D: conditions read only the supplied tag set.
// ---------------------------------------------------------------------------

fn cond_ask(
    pipeline: &ModifierPipeline,
    fx: &Fixture,
    present: &[usize],
    mods: &[Modifier],
) -> Result<(StatValue, ModifierBreakdown), RulesError> {
    let target = fx.stats[0];
    let stored = block(&[(target, int(0))]);
    let tags: TagSet = present.iter().map(|index| fx.tags[*index]).collect();
    let ctx = context(fx.entity, &stored, &tags, mods);
    pipeline.query(fx.entity, target, &ctx)
}

fn gated(id: u128, stat: StatId, condition: ConditionExpr) -> Modifier {
    modifier(
        id,
        "sk",
        id,
        stat,
        ModOp::Add(int(5)),
        TYPE_A,
        None,
        Some(condition),
        0,
    )
}

fn r_cond_hastag_present() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![gated(1, target, ConditionExpr::HasTag(fx.tags[0]))];
    let (value, breakdown) = cond_ask(&pipeline, &fx, &[0], &mods).unwrap();
    assert_eq!(value, int(5));
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(1), applied(int(0), int(5)))]
    );
}

fn r_cond_hastag_missing() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![gated(1, target, ConditionExpr::HasTag(fx.tags[3]))];
    let (value, breakdown) = cond_ask(&pipeline, &fx, &[0], &mods).unwrap();
    assert_eq!(value, int(0));
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(1), ContributionStatus::ConditionFalse)]
    );
}

fn r_cond_not_true() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![gated(
        1,
        target,
        ConditionExpr::Not(Box::new(ConditionExpr::HasTag(fx.tags[3]))),
    )];
    let (value, _) = cond_ask(&pipeline, &fx, &[0], &mods).unwrap();
    assert_eq!(value, int(5));
}

fn r_cond_not_false() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![gated(
        1,
        target,
        ConditionExpr::Not(Box::new(ConditionExpr::HasTag(fx.tags[0]))),
    )];
    let (value, breakdown) = cond_ask(&pipeline, &fx, &[0], &mods).unwrap();
    assert_eq!(value, int(0));
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(1), ContributionStatus::ConditionFalse)]
    );
}

fn r_cond_all_true() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![gated(
        1,
        target,
        ConditionExpr::All(vec![
            ConditionExpr::HasTag(fx.tags[0]),
            ConditionExpr::HasTag(fx.tags[1]),
        ]),
    )];
    let (value, _) = cond_ask(&pipeline, &fx, &[0, 1], &mods).unwrap();
    assert_eq!(value, int(5));
}

fn r_cond_all_false() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![gated(
        1,
        target,
        ConditionExpr::All(vec![
            ConditionExpr::HasTag(fx.tags[0]),
            ConditionExpr::HasTag(fx.tags[3]),
        ]),
    )];
    let (value, breakdown) = cond_ask(&pipeline, &fx, &[0, 1], &mods).unwrap();
    assert_eq!(value, int(0));
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(1), ContributionStatus::ConditionFalse)]
    );
}

fn r_cond_any_true() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![gated(
        1,
        target,
        ConditionExpr::Any(vec![
            ConditionExpr::HasTag(fx.tags[3]),
            ConditionExpr::HasTag(fx.tags[1]),
        ]),
    )];
    let (value, _) = cond_ask(&pipeline, &fx, &[1], &mods).unwrap();
    assert_eq!(value, int(5));
}

fn r_cond_any_false() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![gated(
        1,
        target,
        ConditionExpr::Any(vec![
            ConditionExpr::HasTag(fx.tags[2]),
            ConditionExpr::HasTag(fx.tags[3]),
        ]),
    )];
    let (value, breakdown) = cond_ask(&pipeline, &fx, &[0], &mods).unwrap();
    assert_eq!(value, int(0));
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(1), ContributionStatus::ConditionFalse)]
    );
}

fn r_cond_empty_all_true() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![gated(1, target, ConditionExpr::All(vec![]))];
    let (value, _) = cond_ask(&pipeline, &fx, &[], &mods).unwrap();
    assert_eq!(value, int(5));
}

fn r_cond_empty_any_false() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![gated(1, target, ConditionExpr::Any(vec![]))];
    let (value, breakdown) = cond_ask(&pipeline, &fx, &[0], &mods).unwrap();
    assert_eq!(value, int(0));
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(1), ContributionStatus::ConditionFalse)]
    );
}

fn r_cond_nested() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![
        gated(
            1,
            target,
            ConditionExpr::Not(Box::new(ConditionExpr::Any(vec![
                ConditionExpr::HasTag(fx.tags[2]),
                ConditionExpr::HasTag(fx.tags[3]),
            ]))),
        ),
        gated(
            2,
            target,
            ConditionExpr::All(vec![
                ConditionExpr::HasTag(fx.tags[0]),
                ConditionExpr::Not(Box::new(ConditionExpr::HasTag(fx.tags[3]))),
            ]),
        ),
    ];
    let (value, breakdown) = cond_ask(&pipeline, &fx, &[0], &mods).unwrap();
    assert_eq!(value, int(10));
    assert_eq!(
        contribs(&breakdown, target),
        vec![
            (uid(1), applied(int(0), int(5))),
            (uid(2), applied(int(5), int(10))),
        ]
    );
}

fn r_cond_reads_context_not_stat_tags() {
    // The stat holds a Tags value containing ta, but the query tag set is
    // empty, so HasTag(ta) is false: conditions never read stat values.
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Tags, None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let base = StatValue::Tags(tag_set(&fx.tags[..1]));
    let stored = block(&[(fx.stats[0], base.clone())]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        1,
        "sk",
        1,
        fx.stats[0],
        ModOp::Set(StatValue::Tags(TagSet::new())),
        TYPE_A,
        None,
        Some(ConditionExpr::HasTag(fx.tags[0])),
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (value, breakdown) = pipeline.query(fx.entity, fx.stats[0], &ctx).unwrap();
    assert_eq!(value, base);
    assert_eq!(
        contribs(&breakdown, fx.stats[0]),
        vec![(uid(1), ContributionStatus::ConditionFalse)]
    );
}

fn r_cond_false_still_validated() {
    // A false condition does not excuse an invalid operation: Add on Bool
    // fails validation before any condition evaluates.
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Bool, None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], StatValue::Bool(true))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        9,
        "sk",
        9,
        fx.stats[0],
        ModOp::Add(int(1)),
        TYPE_A,
        None,
        Some(ConditionExpr::HasTag(fx.tags[0])),
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, fx.stats[0], &ctx),
        RulesErrorCode::InvalidOperation,
        &format!("/modifiers/{}/op", uid(9)),
    );
}

// ---------------------------------------------------------------------------
// Section E: query errors, precedence, limits, removal, invariance.
// ---------------------------------------------------------------------------

fn r_entity_mismatch() {
    let (mut fx, pipeline) = solo();
    let other = fx.arena.insert(());
    let stored = block(&[(fx.stats[0], int(0))]);
    let tags = TagSet::new();
    let mods: Vec<Modifier> = vec![];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(other, fx.stats[0], &ctx),
        RulesErrorCode::EntityMismatch,
        "/context/entity",
    );
}

fn r_unknown_queried_stat() {
    let (fx, pipeline) = solo();
    let stored = block(&[(fx.stats[0], int(0))]);
    let tags = TagSet::new();
    let mods: Vec<Modifier> = vec![];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, fx.stats[1], &ctx),
        RulesErrorCode::UnknownStat,
        &format!("/stats/{}", fx.stats[1].index()),
    );
}

fn r_unknown_modifier_target() {
    let (fx, pipeline) = solo();
    let mods = vec![add(3, fx.stats[4], 1)];
    expect_err(
        ask(&pipeline, &fx, 0, &mods),
        RulesErrorCode::UnknownStat,
        &format!("/modifiers/{}/target", uid(3)),
    );
}

fn r_unknown_mod_type() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        3,
        "sk",
        3,
        target,
        ModOp::Add(int(1)),
        "no-such-type",
        None,
        None,
        0,
    )];
    expect_err(
        ask(&pipeline, &fx, 0, &mods),
        RulesErrorCode::UnknownModifierType,
        &format!("/modifiers/{}/mod_type", uid(3)),
    );
}

fn r_modtype_case_sensitive() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        3,
        "sk",
        3,
        target,
        ModOp::Add(int(1)),
        "TA-TYPE",
        None,
        None,
        0,
    )];
    expect_err(
        ask(&pipeline, &fx, 0, &mods),
        RulesErrorCode::UnknownModifierType,
        &format!("/modifiers/{}/mod_type", uid(3)),
    );
}

fn r_empty_mod_type() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        3,
        "sk",
        3,
        target,
        ModOp::Add(int(1)),
        "",
        None,
        None,
        0,
    )];
    expect_err(
        ask(&pipeline, &fx, 0, &mods),
        RulesErrorCode::InvalidName,
        &format!("/modifiers/{}/mod_type", uid(3)),
    );
}

fn r_empty_source_kind() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        3,
        "",
        3,
        target,
        ModOp::Add(int(1)),
        TYPE_A,
        None,
        None,
        0,
    )];
    expect_err(
        ask(&pipeline, &fx, 0, &mods),
        RulesErrorCode::InvalidName,
        &format!("/modifiers/{}/source", uid(3)),
    );
}

fn r_empty_optional_name() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        3,
        "sk",
        3,
        target,
        ModOp::Add(int(1)),
        TYPE_A,
        Some(""),
        None,
        0,
    )];
    expect_err(
        ask(&pipeline, &fx, 0, &mods),
        RulesErrorCode::InvalidName,
        &format!("/modifiers/{}/name", uid(3)),
    );
}

fn r_unknown_stored_stat() {
    let (fx, pipeline) = solo();
    let stored = block(&[(fx.stats[0], int(0)), (fx.stats[1], int(0))]);
    let tags = TagSet::new();
    let mods: Vec<Modifier> = vec![];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, fx.stats[0], &ctx),
        RulesErrorCode::UnknownStat,
        &format!("/stats/{}", fx.stats[1].index()),
    );
}

fn r_derived_base_rejected() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Int, Some(Expr::Literal(int(5))))],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], int(0))]);
    let tags = TagSet::new();
    let mods: Vec<Modifier> = vec![];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, fx.stats[0], &ctx),
        RulesErrorCode::DerivedBase,
        &format!("/stats/{}", fx.stats[0].index()),
    );
}

fn r_duplicate_modifier_ids_across_targets() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![int_def(fx.stats[0]), int_def(fx.stats[1])],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], int(0)), (fx.stats[1], int(0))]);
    let tags = TagSet::new();
    let mods = vec![add(6, fx.stats[0], 1), add(6, fx.stats[1], 2)];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, fx.stats[0], &ctx),
        RulesErrorCode::DuplicateModifier,
        &format!("/modifiers/{}", uid(6)),
    );
}

fn r_duplicate_modifier_ids_smallest_reported() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![
        add(9, target, 1),
        add(9, target, 2),
        add(5, target, 3),
        add(5, target, 4),
    ];
    expect_err(
        ask(&pipeline, &fx, 0, &mods),
        RulesErrorCode::DuplicateModifier,
        &format!("/modifiers/{}", uid(5)),
    );
}

fn r_modifiers_over_limit() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods: Vec<Modifier> = (1..=4097_u128).map(|id| add(id, target, 0)).collect();
    assert_eq!(mods.len(), 4097);
    expect_err(
        ask(&pipeline, &fx, 0, &mods),
        RulesErrorCode::LimitExceeded,
        "/modifiers",
    );
}

fn r_tags_over_limit() {
    let (mut fx, pipeline) = solo();
    for index in 0..1021 {
        fx.interners.intern_tag(&format!("overflow-{index:04}"));
    }
    let stored = block(&[(fx.stats[0], int(0))]);
    let mut tags = TagSet::new();
    for index in 0..1021 {
        let tag = fx.interners.tag(&format!("overflow-{index:04}")).unwrap();
        tags.insert(tag);
    }
    for tag in fx.tags.iter() {
        tags.insert(*tag);
    }
    assert_eq!(tags.len(), 1025);
    let mods: Vec<Modifier> = vec![];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, fx.stats[0], &ctx),
        RulesErrorCode::LimitExceeded,
        "/context",
    );
}

fn r_policies_over_limit() {
    let fx = fixture();
    let mut pairs: Vec<(String, StackingPolicy)> = Vec::new();
    for index in 0..1025 {
        pairs.push((format!("bulk-{index:04}"), StackingPolicy::StackAll));
    }
    let map: std::collections::BTreeMap<crpg_rules::ModTypeId, StackingPolicy> = pairs
        .into_iter()
        .map(|(name, policy)| (crpg_rules::ModTypeId(name), policy))
        .collect();
    assert_eq!(map.len(), 1025);
    let error = ModifierPipeline::new(vec![int_def(fx.stats[0])], map).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::LimitExceeded);
    assert_eq!(error.location, "/policies");
}

fn r_policies_at_capacity_query_succeeds() {
    let fx = fixture();
    // 1024 policies: the largest valid count; the query still succeeds.
    let mut pairs: Vec<(String, StackingPolicy)> = Vec::new();
    for index in 0..1024 {
        pairs.push((format!("bulk-{index:04}"), StackingPolicy::StackAll));
    }
    let map: std::collections::BTreeMap<crpg_rules::ModTypeId, StackingPolicy> = pairs
        .into_iter()
        .map(|(name, policy)| (crpg_rules::ModTypeId(name), policy))
        .collect();
    assert_eq!(map.len(), 1024);
    let pipeline = ModifierPipeline::new(vec![int_def(fx.stats[0])], map).unwrap();
    let empty: Vec<Modifier> = vec![];
    let stored = block(&[(fx.stats[0], int(3))]);
    let tags = TagSet::new();
    let ctx = context(fx.entity, &stored, &tags, &empty);
    let (value, _) = pipeline.query(fx.entity, fx.stats[0], &ctx).unwrap();
    assert_eq!(value, int(3));
}

fn r_modifiers_at_capacity_apply() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    // 4096 modifiers: the largest valid count; every one applies in order.
    let mods: Vec<Modifier> = (1..=4096_u128).map(|id| add(id, target, 1)).collect();
    assert_eq!(mods.len(), 4096);
    let (value, breakdown) = ask(&pipeline, &fx, 0, &mods).unwrap();
    assert_eq!(value, int(4096));
    assert_eq!(breakdown.traces[&target].contributions.len(), 4096);
}

fn r_duplicate_stat_definitions() {
    let fx = fixture();
    let target = fx.stats[2];
    let error = ModifierPipeline::new(
        vec![int_def(fx.stats[0]), int_def(target), int_def(target)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::DuplicateStat);
    assert_eq!(error.location, format!("/definitions/{}", target.index()));
}

fn r_empty_policy_name() {
    let fx = fixture();
    let error = ModifierPipeline::new(
        vec![int_def(fx.stats[0])],
        policies(&[("", StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidName);
    assert_eq!(error.location, "/policies/");
}

fn r_precedence_entity_before_stat() {
    let (mut fx, pipeline) = solo();
    let other = fx.arena.insert(());
    let stored = block(&[(fx.stats[0], int(0))]);
    let tags = TagSet::new();
    let mods: Vec<Modifier> = vec![];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(other, fx.stats[5], &ctx),
        RulesErrorCode::EntityMismatch,
        "/context/entity",
    );
}

fn r_precedence_stat_before_limits() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods: Vec<Modifier> = (1..=4097_u128).map(|id| add(id, target, 0)).collect();
    let stored = block(&[(target, int(0))]);
    let tags = TagSet::new();
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, fx.stats[5], &ctx),
        RulesErrorCode::UnknownStat,
        &format!("/stats/{}", fx.stats[5].index()),
    );
}

fn r_precedence_limits_before_stored() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods: Vec<Modifier> = (1..=4097_u128).map(|id| add(id, target, 0)).collect();
    let stored = block(&[(target, int(0)), (fx.stats[5], int(0))]);
    let tags = TagSet::new();
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, target, &ctx),
        RulesErrorCode::LimitExceeded,
        "/modifiers",
    );
}

fn r_precedence_stored_before_dup_ids() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let stored = block(&[(target, int(0)), (fx.stats[5], int(0))]);
    let tags = TagSet::new();
    let mods = vec![add(4, target, 1), add(4, target, 2)];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, target, &ctx),
        RulesErrorCode::UnknownStat,
        &format!("/stats/{}", fx.stats[5].index()),
    );
}

fn r_precedence_dup_before_modifier() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let stored = block(&[(target, int(0))]);
    let tags = TagSet::new();
    let mods = vec![
        add(4, target, 1),
        add(4, target, 2),
        modifier(
            9,
            "sk",
            9,
            target,
            ModOp::Add(int(1)),
            "missing-type",
            None,
            None,
            0,
        ),
    ];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, target, &ctx),
        RulesErrorCode::DuplicateModifier,
        &format!("/modifiers/{}", uid(4)),
    );
}

fn r_precedence_modifier_id_order() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let stored = block(&[(target, int(0))]);
    let tags = TagSet::new();
    // id 9 fails on its unknown type, id 3 on its operation; ascending id 3 wins.
    let bad_op = modifier(
        3,
        "sk",
        3,
        target,
        ModOp::Set(StatValue::Fixed(raw_fx(0))),
        TYPE_A,
        None,
        None,
        0,
    );
    let bad_type = modifier(
        9,
        "sk",
        9,
        target,
        ModOp::Add(int(1)),
        "missing-type",
        None,
        None,
        0,
    );
    let mods = vec![bad_type, bad_op];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, target, &ctx),
        RulesErrorCode::TypeMismatch,
        &format!("/modifiers/{}/op", uid(3)),
    );
}

fn r_no_mutation_on_error() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let stored = block(&[(target, int(0))]);
    let stored_before = stored.clone();
    let tags = TagSet::new();
    let mods = vec![add(4, target, 1), add(4, target, 2)];
    let mods_before = mods.clone();
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let result = pipeline.query(fx.entity, target, &ctx);
    assert!(result.is_err());
    assert_eq!(stored, stored_before);
    assert_eq!(mods, mods_before);
}

fn r_add_fixed_on_int() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        3,
        "sk",
        3,
        target,
        ModOp::Add(StatValue::Fixed(raw_fx(0))),
        TYPE_A,
        None,
        None,
        0,
    )];
    expect_err(
        ask(&pipeline, &fx, 0, &mods),
        RulesErrorCode::TypeMismatch,
        &format!("/modifiers/{}/op", uid(3)),
    );
}

fn r_add_int_on_fixed() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Fixed, None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], StatValue::Fixed(raw_fx(0)))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        3,
        "sk",
        3,
        fx.stats[0],
        ModOp::Add(int(1)),
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, fx.stats[0], &ctx),
        RulesErrorCode::TypeMismatch,
        &format!("/modifiers/{}/op", uid(3)),
    );
}

fn r_set_fixed_on_int() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        3,
        "sk",
        3,
        target,
        ModOp::Set(StatValue::Fixed(raw_fx(0))),
        TYPE_A,
        None,
        None,
        0,
    )];
    expect_err(
        ask(&pipeline, &fx, 0, &mods),
        RulesErrorCode::TypeMismatch,
        &format!("/modifiers/{}/op", uid(3)),
    );
}

fn r_set_on_bool_wrong_kind() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Bool, None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], StatValue::Bool(true))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        3,
        "sk",
        3,
        fx.stats[0],
        ModOp::Set(int(1)),
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, fx.stats[0], &ctx),
        RulesErrorCode::TypeMismatch,
        &format!("/modifiers/{}/op", uid(3)),
    );
}

fn r_add_on_bool() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Bool, None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], StatValue::Bool(true))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        3,
        "sk",
        3,
        fx.stats[0],
        ModOp::Add(int(1)),
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, fx.stats[0], &ctx),
        RulesErrorCode::InvalidOperation,
        &format!("/modifiers/{}/op", uid(3)),
    );
}

fn r_multiply_on_bool() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Bool, None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], StatValue::Bool(true))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        3,
        "sk",
        3,
        fx.stats[0],
        ModOp::Multiply(raw_fx(65536)),
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, fx.stats[0], &ctx),
        RulesErrorCode::InvalidOperation,
        &format!("/modifiers/{}/op", uid(3)),
    );
}

fn r_clamp_on_enum() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], enum_kind("ed", &["ex", "ey"]), None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], enum_value("ed", "ex"))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        3,
        "sk",
        3,
        fx.stats[0],
        ModOp::Clamp {
            min: enum_value("ed", "ex"),
            max: enum_value("ed", "ey"),
        },
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, fx.stats[0], &ctx),
        RulesErrorCode::InvalidOperation,
        &format!("/modifiers/{}/op", uid(3)),
    );
}

fn r_clamp_bounds_wrong_kind() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Fixed, None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], StatValue::Fixed(raw_fx(0)))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        3,
        "sk",
        3,
        fx.stats[0],
        ModOp::Clamp {
            min: int(0),
            max: int(1),
        },
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, fx.stats[0], &ctx),
        RulesErrorCode::TypeMismatch,
        &format!("/modifiers/{}/op", uid(3)),
    );
}

fn r_clamp_min_above_max() {
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![modifier(
        3,
        "sk",
        3,
        target,
        ModOp::Clamp {
            min: int(10),
            max: int(0),
        },
        TYPE_A,
        None,
        None,
        0,
    )];
    expect_err(
        ask(&pipeline, &fx, 5, &mods),
        RulesErrorCode::InvalidClamp,
        &format!("/modifiers/{}/op", uid(3)),
    );
}

fn r_enum_set_wrong_variant() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], enum_kind("ed", &["ex", "ey"]), None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], enum_value("ed", "ex"))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        3,
        "sk",
        3,
        fx.stats[0],
        ModOp::Set(enum_value("ed", "ez")),
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, fx.stats[0], &ctx),
        RulesErrorCode::InvalidEnum,
        &format!("/modifiers/{}/op", uid(3)),
    );
}

fn r_enum_set_wrong_domain() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], enum_kind("ed", &["ex", "ey"]), None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], enum_value("ed", "ex"))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        3,
        "sk",
        3,
        fx.stats[0],
        ModOp::Set(enum_value("other", "ex")),
        TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, fx.stats[0], &ctx),
        RulesErrorCode::InvalidEnum,
        &format!("/modifiers/{}/op", uid(3)),
    );
}

fn r_stored_enum_wrong_member() {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![def(fx.stats[0], enum_kind("ed", &["ex", "ey"]), None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = block(&[(fx.stats[0], enum_value("ed", "ez"))]);
    let tags = TagSet::new();
    let mods: Vec<Modifier> = vec![];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    expect_err(
        pipeline.query(fx.entity, fx.stats[0], &ctx),
        RulesErrorCode::InvalidEnum,
        &format!("/stats/{}", fx.stats[0].index()),
    );
}

fn r_query_full_block_boundary() {
    // 1024 definitions with 1024 stored entries query successfully.
    let mut fx = fixture();
    let mut definitions = Vec::new();
    let mut entries = Vec::new();
    for index in 0..1024 {
        let stat = fx.interners.intern_stat(&format!("bulk-{index:04}"));
        definitions.push(int_def(stat));
        entries.push((stat, int(index)));
    }
    let pipeline =
        ModifierPipeline::new(definitions, policies(&[(TYPE_A, StackingPolicy::StackAll)]))
            .unwrap();
    let stored = block(&entries);
    assert_eq!(stored.len(), 1024);
    let tags = TagSet::new();
    let mods: Vec<Modifier> = vec![];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (first_stat, _) = entries[0];
    let (value, breakdown) = pipeline.query(fx.entity, first_stat, &ctx).unwrap();
    assert_eq!(value, int(0));
    assert_eq!(breakdown.traces.len(), 1);
}

fn r_condition_nodes_boundary_valid() {
    // All with 255 children holds 256 nodes: the largest valid shape.
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let children: Vec<ConditionExpr> = (0..255)
        .map(|_| ConditionExpr::HasTag(fx.tags[0]))
        .collect();
    let mods = vec![gated(1, target, ConditionExpr::All(children))];
    let (value, _) = cond_ask(&pipeline, &fx, &[0], &mods).unwrap();
    assert_eq!(value, int(5));
}

fn r_condition_nodes_over_limit() {
    // All with 256 children holds 257 nodes: the first over-limit shape.
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let children: Vec<ConditionExpr> = (0..256)
        .map(|_| ConditionExpr::HasTag(fx.tags[0]))
        .collect();
    let mods = vec![gated(1, target, ConditionExpr::All(children))];
    expect_err(
        cond_ask(&pipeline, &fx, &[0], &mods),
        RulesErrorCode::LimitExceeded,
        &format!("/modifiers/{}/condition", uid(1)),
    );
}

fn not_chain(tag: crpg_core::TagId, depth_nots: usize) -> ConditionExpr {
    let mut condition = ConditionExpr::HasTag(tag);
    for _ in 0..depth_nots {
        condition = ConditionExpr::Not(Box::new(condition));
    }
    condition
}

fn r_condition_depth_boundary_valid() {
    // 31 negations over one tag read as depth 32: the largest valid shape.
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![gated(1, target, not_chain(fx.tags[0], 31))];
    let (value, _) = cond_ask(&pipeline, &fx, &[0], &mods).unwrap();
    // Odd negations flip the present tag to false.
    assert_eq!(value, int(0));
}

fn r_condition_depth_over_limit() {
    // 32 negations read as depth 33: the first over-limit shape.
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let mods = vec![gated(1, target, not_chain(fx.tags[0], 32))];
    expect_err(
        cond_ask(&pipeline, &fx, &[0], &mods),
        RulesErrorCode::LimitExceeded,
        &format!("/modifiers/{}/condition", uid(1)),
    );
}

fn r_distinct_source_kinds_same_ulid() {
    // Same ULID under two kinds are distinct sources: removing one keeps
    // the other, and the survivor still applies.
    let (fx, pipeline) = solo();
    let target = fx.stats[0];
    let stored = block(&[(target, int(0))]);
    let tags = TagSet::new();
    let mods = vec![
        modifier(
            1,
            "kind-a",
            50,
            target,
            ModOp::Add(int(3)),
            TYPE_A,
            None,
            None,
            0,
        ),
        modifier(
            2,
            "kind-b",
            50,
            target,
            ModOp::Add(int(4)),
            TYPE_A,
            None,
            None,
            0,
        ),
    ];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (both, _) = pipeline.query(fx.entity, target, &ctx).unwrap();
    assert_eq!(both, int(7));
    let surviving: Vec<Modifier> = mods
        .iter()
        .filter(|modifier| !(modifier.source.kind == "kind-a" && modifier.source.id == uid(50)))
        .cloned()
        .collect();
    assert_eq!(surviving.len(), 1);
    let ctx = context(fx.entity, &stored, &tags, &surviving);
    let (value, breakdown) = pipeline.query(fx.entity, target, &ctx).unwrap();
    assert_eq!(value, int(4));
    assert_eq!(
        contribs(&breakdown, target),
        vec![(uid(2), applied(int(0), int(4)))]
    );
}

fn r_input_order_invariance_explicit() {
    let orders = [vec![1_u128, 2_u128, 3_u128], vec![3_u128, 1_u128, 2_u128]];
    let mut first: Option<(StatValue, ModifierBreakdown)> = None;
    for order in orders {
        let (fx, pipeline) = solo();
        let target = fx.stats[0];
        let mods: Vec<Modifier> = order.iter().map(|id| add(*id, target, 2)).collect();
        let (value, breakdown) = ask(&pipeline, &fx, 1, &mods).unwrap();
        assert_eq!(value, int(7));
        match first {
            None => first = Some((value, breakdown)),
            Some((ref first_value, ref first_breakdown)) => {
                assert_eq!(&value, first_value);
                assert_eq!(&breakdown, first_breakdown);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Registry: every row runs, names stay unique, and the count stays >= 100.
// ---------------------------------------------------------------------------

fn rows() -> Vec<(&'static str, fn())> {
    vec![
        ("set_int", r_set_int),
        ("set_fixed", r_set_fixed),
        ("set_bool", r_set_bool),
        ("set_enum", r_set_enum),
        ("set_tags", r_set_tags),
        ("tags_set_empty", r_tags_set_empty),
        ("add_int_sum", r_add_int_sum),
        ("add_fixed", r_add_fixed),
        ("multiply_int", r_multiply_int),
        ("multiply_fixed", r_multiply_fixed),
        ("clamp_below", r_clamp_below),
        ("clamp_within", r_clamp_within),
        ("clamp_above", r_clamp_above),
        ("oracle_transitions", r_oracle_transitions),
        ("negative_floor_scale", r_negative_floor_scale),
        ("full_range_scale", r_full_range_scale),
        ("zero_factor", r_zero_factor),
        ("negative_factor", r_negative_factor),
        ("int_add_saturate_top", r_int_add_saturate_top),
        ("int_add_saturate_bottom", r_int_add_saturate_bottom),
        ("min_times_neg_one", r_min_times_neg_one),
        (
            "phase_precedence_despite_input_order",
            r_phase_precedence_despite_input_order,
        ),
        (
            "cross_type_global_phase_order",
            r_cross_type_global_phase_order,
        ),
        (
            "priority_orders_within_phase",
            r_priority_orders_within_phase,
        ),
        ("id_breaks_priority_ties", r_id_breaks_priority_ties),
        ("repeated_sets_last_wins", r_repeated_sets_last_wins),
        ("higher_priority_set_wins", r_higher_priority_set_wins),
        ("saturated_still_applied", r_saturated_still_applied),
        (
            "canonical_saturation_oracle_both_orders",
            r_canonical_saturation_oracle_both_orders,
        ),
        ("removal_restores_exact", r_removal_restores_exact),
        ("stackall_zero_add_applies", r_stackall_zero_add_applies),
        ("empty_modifiers", r_empty_modifiers),
        ("sequential_disjoint_clamps", r_sequential_disjoint_clamps),
        (
            "contradictory_clamps_later_wins",
            r_contradictory_clamps_later_wins,
        ),
        ("equal_bounds_pin", r_equal_bounds_pin),
        ("int_sub_via_negative_add", r_int_sub_via_negative_add),
        (
            "multiply_then_add_phase_order",
            r_multiply_then_add_phase_order,
        ),
        (
            "unrelated_targets_absent_from_trace",
            r_unrelated_targets_absent_from_trace,
        ),
        ("zero_ulid_accepted", r_zero_ulid_accepted),
        ("fixed_add_saturates", r_fixed_add_saturates),
        ("fixed_clamp_bounds", r_fixed_clamp_bounds),
        ("priority_extremes_order", r_priority_extremes_order),
        ("bonus_keeps_both_signs", r_bonus_keeps_both_signs),
        ("bonus_pos_competition", r_bonus_pos_competition),
        ("bonus_neg_competition", r_bonus_neg_competition),
        ("bonus_pos_tie_priority", r_bonus_pos_tie_priority),
        ("bonus_pos_tie_id", r_bonus_pos_tie_id),
        ("bonus_neg_tie", r_bonus_neg_tie),
        ("bonus_zero_is_zeroadd", r_bonus_zero_is_zeroadd),
        ("bonus_all_zero", r_bonus_all_zero),
        (
            "bonus_false_winner_candidate",
            r_bonus_false_winner_candidate,
        ),
        ("illegal_bonus_set", r_illegal_bonus_set),
        ("illegal_bonus_multiply", r_illegal_bonus_multiply),
        ("illegal_bonus_clamp", r_illegal_bonus_clamp),
        (
            "pername_distinct_names_both_apply",
            r_pername_distinct_names_both_apply,
        ),
        (
            "pername_same_name_suppresses",
            r_pername_same_name_suppresses,
        ),
        ("pername_priority_beats_id", r_pername_priority_beats_id),
        ("pername_across_sources", r_pername_across_sources),
        ("pername_phase_isolation", r_pername_phase_isolation),
        ("pername_zero_add_applies", r_pername_zero_add_applies),
        (
            "pername_missing_name_rejected",
            r_pername_missing_name_rejected,
        ),
        ("pername_case_sensitive", r_pername_case_sensitive),
        ("pername_set_suppression", r_pername_set_suppression),
        ("pername_order_invariance", r_pername_order_invariance),
        ("cond_hastag_present", r_cond_hastag_present),
        ("cond_hastag_missing", r_cond_hastag_missing),
        ("cond_not_true", r_cond_not_true),
        ("cond_not_false", r_cond_not_false),
        ("cond_all_true", r_cond_all_true),
        ("cond_all_false", r_cond_all_false),
        ("cond_any_true", r_cond_any_true),
        ("cond_any_false", r_cond_any_false),
        ("cond_empty_all_true", r_cond_empty_all_true),
        ("cond_empty_any_false", r_cond_empty_any_false),
        ("cond_nested", r_cond_nested),
        (
            "cond_reads_context_not_stat_tags",
            r_cond_reads_context_not_stat_tags,
        ),
        ("cond_false_still_validated", r_cond_false_still_validated),
        ("entity_mismatch", r_entity_mismatch),
        ("unknown_queried_stat", r_unknown_queried_stat),
        ("unknown_modifier_target", r_unknown_modifier_target),
        ("unknown_mod_type", r_unknown_mod_type),
        ("modtype_case_sensitive", r_modtype_case_sensitive),
        ("empty_mod_type", r_empty_mod_type),
        ("empty_source_kind", r_empty_source_kind),
        ("empty_optional_name", r_empty_optional_name),
        ("unknown_stored_stat", r_unknown_stored_stat),
        ("derived_base_rejected", r_derived_base_rejected),
        (
            "duplicate_modifier_ids_across_targets",
            r_duplicate_modifier_ids_across_targets,
        ),
        (
            "duplicate_modifier_ids_smallest_reported",
            r_duplicate_modifier_ids_smallest_reported,
        ),
        ("modifiers_over_limit", r_modifiers_over_limit),
        ("modifiers_at_capacity_apply", r_modifiers_at_capacity_apply),
        ("tags_over_limit", r_tags_over_limit),
        ("policies_over_limit", r_policies_over_limit),
        (
            "policies_at_capacity_query_succeeds",
            r_policies_at_capacity_query_succeeds,
        ),
        ("duplicate_stat_definitions", r_duplicate_stat_definitions),
        ("empty_policy_name", r_empty_policy_name),
        (
            "precedence_entity_before_stat",
            r_precedence_entity_before_stat,
        ),
        (
            "precedence_stat_before_limits",
            r_precedence_stat_before_limits,
        ),
        (
            "precedence_limits_before_stored",
            r_precedence_limits_before_stored,
        ),
        (
            "precedence_stored_before_dup_ids",
            r_precedence_stored_before_dup_ids,
        ),
        (
            "precedence_dup_before_modifier",
            r_precedence_dup_before_modifier,
        ),
        (
            "precedence_modifier_id_order",
            r_precedence_modifier_id_order,
        ),
        ("no_mutation_on_error", r_no_mutation_on_error),
        ("add_fixed_on_int", r_add_fixed_on_int),
        ("add_int_on_fixed", r_add_int_on_fixed),
        ("set_fixed_on_int", r_set_fixed_on_int),
        ("set_on_bool_wrong_kind", r_set_on_bool_wrong_kind),
        ("add_on_bool", r_add_on_bool),
        ("multiply_on_bool", r_multiply_on_bool),
        ("clamp_on_enum", r_clamp_on_enum),
        ("clamp_bounds_wrong_kind", r_clamp_bounds_wrong_kind),
        ("clamp_min_above_max", r_clamp_min_above_max),
        ("enum_set_wrong_variant", r_enum_set_wrong_variant),
        ("enum_set_wrong_domain", r_enum_set_wrong_domain),
        ("stored_enum_wrong_member", r_stored_enum_wrong_member),
        ("query_full_block_boundary", r_query_full_block_boundary),
        (
            "condition_nodes_boundary_valid",
            r_condition_nodes_boundary_valid,
        ),
        ("condition_nodes_over_limit", r_condition_nodes_over_limit),
        (
            "condition_depth_boundary_valid",
            r_condition_depth_boundary_valid,
        ),
        ("condition_depth_over_limit", r_condition_depth_over_limit),
        (
            "distinct_source_kinds_same_ulid",
            r_distinct_source_kinds_same_ulid,
        ),
        (
            "input_order_invariance_explicit",
            r_input_order_invariance_explicit,
        ),
    ]
}

#[test]
fn table_runs_at_least_100_distinct_rows() {
    let table = rows();
    assert!(
        table.len() >= 100,
        "table holds {} rows, need at least 100",
        table.len()
    );
    let names: BTreeSet<&str> = table.iter().map(|(name, _)| *name).collect();
    assert_eq!(names.len(), table.len(), "row names must be unique");
    for (name, run) in table {
        run();
        let _ = name;
    }
}
