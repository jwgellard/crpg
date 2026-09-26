//! Derived-stat graphs: references, modified inputs, diamonds, cycles,
//! operators, division, and expression/definition limits.

#[allow(dead_code)]
#[path = "common/mod.rs"]
mod common;

use std::collections::BTreeSet;

use crpg_core::StatId;
use crpg_rules::{
    ContributionStatus, Expr, ModOp, ModifierPipeline, RulesErrorCode, StackingPolicy, StatBlock,
    StatDefinition, StatKind, StatValue, TagSet,
};

use common::{
    block, context, def, enum_kind, enum_value, fixture, int_def, modifier, policies, raw_fx,
    tag_set, Fixture,
};

fn stack_pipeline(definitions: Vec<StatDefinition>) -> ModifierPipeline {
    ModifierPipeline::new(
        definitions,
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap()
}

fn lit(value: i32) -> Expr {
    Expr::Literal(StatValue::Int(value))
}

fn query_stat(
    pipeline: &ModifierPipeline,
    fx: &Fixture,
    stored: &StatBlock,
    stat: StatId,
) -> Result<(StatValue, crpg_rules::ModifierBreakdown), crpg_rules::RulesError> {
    let tags = TagSet::new();
    let mods: Vec<crpg_rules::Modifier> = vec![];
    let ctx = context(fx.entity, stored, &tags, &mods);
    pipeline.query(fx.entity, stat, &ctx)
}

#[test]
fn forward_references_are_legal() {
    let fx = fixture();
    // beta derives from alpha, declared first.
    let pipeline = stack_pipeline(vec![
        def(
            fx.stats[1],
            StatKind::Int,
            Some(Expr::Add(
                Box::new(Expr::Stat(fx.stats[0])),
                Box::new(lit(1)),
            )),
        ),
        int_def(fx.stats[0]),
    ]);
    let stored = block(&[(fx.stats[0], StatValue::Int(41))]);
    let (value, _) = query_stat(&pipeline, &fx, &stored, fx.stats[1]).unwrap();
    assert_eq!(value, StatValue::Int(42));
}

#[test]
fn derived_reads_fully_modified_inputs() {
    let fx = fixture();
    let pipeline = stack_pipeline(vec![
        int_def(fx.stats[0]),
        def(
            fx.stats[1],
            StatKind::Int,
            Some(Expr::Add(
                Box::new(Expr::Stat(fx.stats[0])),
                Box::new(lit(100)),
            )),
        ),
    ]);
    let stored = block(&[(fx.stats[0], StatValue::Int(1))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        1,
        "sk",
        1,
        fx.stats[0],
        ModOp::Add(StatValue::Int(9)),
        common::TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (value, breakdown) = pipeline.query(fx.entity, fx.stats[1], &ctx).unwrap();
    // alpha modifies 1 -> 10; beta sees 10 and adds 100.
    assert_eq!(value, StatValue::Int(110));
    let trace = &breakdown.traces[&fx.stats[1]];
    assert_eq!(trace.base, StatValue::Int(110));
    assert_eq!(trace.value, StatValue::Int(110));
    assert_eq!(trace.dependencies, vec![fx.stats[0]]);
    assert!(trace.contributions.is_empty());
    let inner = &breakdown.traces[&fx.stats[0]];
    assert_eq!(inner.base, StatValue::Int(1));
    assert_eq!(inner.value, StatValue::Int(10));
    assert_eq!(breakdown.traces.len(), 2);
}

#[test]
fn multi_level_formulas_chain() {
    let fx = fixture();
    let pipeline = stack_pipeline(vec![
        int_def(fx.stats[0]),
        def(
            fx.stats[1],
            StatKind::Int,
            Some(Expr::Multiply(
                Box::new(Expr::Stat(fx.stats[0])),
                Box::new(lit(2)),
            )),
        ),
        def(
            fx.stats[2],
            StatKind::Int,
            Some(Expr::Add(
                Box::new(Expr::Stat(fx.stats[1])),
                Box::new(lit(1)),
            )),
        ),
    ]);
    let stored = block(&[(fx.stats[0], StatValue::Int(5))]);
    let (value, breakdown) = query_stat(&pipeline, &fx, &stored, fx.stats[2]).unwrap();
    assert_eq!(value, StatValue::Int(11));
    assert_eq!(breakdown.traces.len(), 3);
    assert_eq!(
        breakdown.traces[&fx.stats[2]].dependencies,
        vec![fx.stats[1]]
    );
}

#[test]
fn diamonds_evaluate_once_with_sorted_dependencies() {
    let fx = fixture();
    // delta = beta + gamma; beta = alpha + 1; gamma = alpha + 2.
    let pipeline = stack_pipeline(vec![
        int_def(fx.stats[0]),
        def(
            fx.stats[1],
            StatKind::Int,
            Some(Expr::Add(
                Box::new(Expr::Stat(fx.stats[0])),
                Box::new(lit(1)),
            )),
        ),
        def(
            fx.stats[2],
            StatKind::Int,
            Some(Expr::Add(
                Box::new(Expr::Stat(fx.stats[0])),
                Box::new(lit(2)),
            )),
        ),
        def(
            fx.stats[3],
            StatKind::Int,
            Some(Expr::Add(
                Box::new(Expr::Stat(fx.stats[2])),
                Box::new(Expr::Stat(fx.stats[1])),
            )),
        ),
    ]);
    let stored = block(&[(fx.stats[0], StatValue::Int(10))]);
    let (value, breakdown) = query_stat(&pipeline, &fx, &stored, fx.stats[3]).unwrap();
    assert_eq!(value, StatValue::Int(23));
    // Four distinct traces: the shared alpha subtree is not duplicated.
    assert_eq!(breakdown.traces.len(), 4);
    assert_eq!(
        breakdown.traces[&fx.stats[3]].dependencies,
        vec![fx.stats[1], fx.stats[2]]
    );
}

#[test]
fn no_memo_cache_survives_a_query() {
    let fx = fixture();
    let pipeline = stack_pipeline(vec![
        int_def(fx.stats[0]),
        def(
            fx.stats[1],
            StatKind::Int,
            Some(Expr::Add(
                Box::new(Expr::Stat(fx.stats[0])),
                Box::new(lit(1)),
            )),
        ),
    ]);
    let first = block(&[(fx.stats[0], StatValue::Int(1))]);
    let (before, _) = query_stat(&pipeline, &fx, &first, fx.stats[1]).unwrap();
    assert_eq!(before, StatValue::Int(2));
    let second = block(&[(fx.stats[0], StatValue::Int(50))]);
    let (after, _) = query_stat(&pipeline, &fx, &second, fx.stats[1]).unwrap();
    assert_eq!(after, StatValue::Int(51));
}

#[test]
fn missing_bases_fail_only_when_evaluated() {
    let fx = fixture();
    let pipeline = stack_pipeline(vec![
        int_def(fx.stats[0]),
        int_def(fx.stats[1]),
        def(
            fx.stats[2],
            StatKind::Int,
            Some(Expr::Add(
                Box::new(Expr::Stat(fx.stats[0])),
                Box::new(lit(1)),
            )),
        ),
    ]);
    // beta is absent but unrelated: querying gamma succeeds.
    let stored = block(&[(fx.stats[0], StatValue::Int(1))]);
    let (value, _) = query_stat(&pipeline, &fx, &stored, fx.stats[2]).unwrap();
    assert_eq!(value, StatValue::Int(2));
    // Querying beta itself reports the missing base.
    let tags = TagSet::new();
    let mods: Vec<crpg_rules::Modifier> = vec![];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let error = pipeline.query(fx.entity, fx.stats[1], &ctx).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::MissingBase);
    assert_eq!(error.location, format!("/stats/{}", fx.stats[1].index()));
    // A transitive missing base reports the leaf, not the root.
    let lone = block(&[(fx.stats[1], StatValue::Int(1))]);
    let error = query_stat(&pipeline, &fx, &lone, fx.stats[2]).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::MissingBase);
    assert_eq!(error.location, format!("/stats/{}", fx.stats[0].index()));
}

#[test]
fn stored_derived_values_are_rejected() {
    let fx = fixture();
    let pipeline = stack_pipeline(vec![def(fx.stats[0], StatKind::Int, Some(lit(4)))]);
    let stored = block(&[(fx.stats[0], StatValue::Int(4))]);
    let error = query_stat(&pipeline, &fx, &stored, fx.stats[0]).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::DerivedBase);
    assert_eq!(error.location, format!("/stats/{}", fx.stats[0].index()));
}

#[test]
fn unknown_expression_references_fail_at_new() {
    let fx = fixture();
    let error = ModifierPipeline::new(
        vec![def(
            fx.stats[0],
            StatKind::Int,
            Some(Expr::Stat(fx.stats[9])),
        )],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::UnknownStat);
    assert_eq!(
        error.location,
        format!("/definitions/{}/derived", fx.stats[0].index())
    );
}

#[test]
fn self_cycles_report_a_repeated_closing_vertex() {
    let fx = fixture();
    let error = ModifierPipeline::new(
        vec![def(
            fx.stats[0],
            StatKind::Int,
            Some(Expr::Stat(fx.stats[0])),
        )],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::Cycle);
    assert_eq!(error.cycle, vec![fx.stats[0], fx.stats[0]]);
    assert_eq!(
        error.location,
        format!("/definitions/{}/derived", fx.stats[0].index())
    );
}

#[test]
fn multi_node_cycles_rotate_to_the_smallest_stat() {
    let fx = fixture();
    // gamma -> alpha -> beta -> gamma; DFS from the smallest reports in
    // cycle order starting and ending there.
    let error = ModifierPipeline::new(
        vec![
            def(fx.stats[0], StatKind::Int, Some(Expr::Stat(fx.stats[1]))),
            def(fx.stats[1], StatKind::Int, Some(Expr::Stat(fx.stats[2]))),
            def(fx.stats[2], StatKind::Int, Some(Expr::Stat(fx.stats[0]))),
        ],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::Cycle);
    assert_eq!(
        error.cycle,
        vec![fx.stats[0], fx.stats[1], fx.stats[2], fx.stats[0]]
    );
    assert_eq!(
        error.location,
        format!("/definitions/{}/derived", fx.stats[0].index())
    );
}

#[test]
fn disconnected_cycles_are_still_rejected() {
    let fx = fixture();
    // alpha is a clean base; the beta/gamma pair cycles without touching it.
    let error = ModifierPipeline::new(
        vec![
            int_def(fx.stats[0]),
            def(fx.stats[1], StatKind::Int, Some(Expr::Stat(fx.stats[2]))),
            def(fx.stats[2], StatKind::Int, Some(Expr::Stat(fx.stats[1]))),
        ],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::Cycle);
    assert_eq!(error.cycle, vec![fx.stats[1], fx.stats[2], fx.stats[1]]);
    assert_eq!(
        error.location,
        format!("/definitions/{}/derived", fx.stats[1].index())
    );
}

#[test]
fn all_binary_operators_evaluate_on_int() {
    type IntCase = (fn(Box<Expr>, Box<Expr>) -> Expr, i32, i32, i32);
    let fx = fixture();
    let cases: Vec<IntCase> = vec![
        (Expr::Add, 20, 6, 26),
        (Expr::Subtract, 20, 6, 14),
        (Expr::Multiply, 20, 6, 120),
        (Expr::Divide, 20, 6, 3),
        (Expr::Min, 20, 6, 6),
        (Expr::Max, 20, 6, 20),
    ];
    for (make, left, right, expected) in cases {
        let pipeline = stack_pipeline(vec![def(
            fx.stats[0],
            StatKind::Int,
            Some(make(
                Box::new(Expr::Literal(StatValue::Int(left))),
                Box::new(Expr::Literal(StatValue::Int(right))),
            )),
        )]);
        let stored = StatBlock::new();
        let (value, _) = query_stat(&pipeline, &fx, &stored, fx.stats[0]).unwrap();
        assert_eq!(value, StatValue::Int(expected));
    }
}

#[test]
fn all_binary_operators_evaluate_on_fixed() {
    type FixedCase = (fn(Box<Expr>, Box<Expr>) -> Expr, i32);
    let fx = fixture();
    // Raw hand-computed expectations: 3.0 and 1.5 in raw units.
    let cases: Vec<FixedCase> = vec![
        (Expr::Add, 294912),
        (Expr::Subtract, 98304),
        (Expr::Multiply, 294912),
        (Expr::Divide, 131072),
        (Expr::Min, 98304),
        (Expr::Max, 196608),
    ];
    for (make, expected) in cases {
        let pipeline = stack_pipeline(vec![def(
            fx.stats[0],
            StatKind::Fixed,
            Some(make(
                Box::new(Expr::Literal(StatValue::Fixed(raw_fx(196608)))),
                Box::new(Expr::Literal(StatValue::Fixed(raw_fx(98304)))),
            )),
        )]);
        let stored = StatBlock::new();
        let (value, _) = query_stat(&pipeline, &fx, &stored, fx.stats[0]).unwrap();
        assert_eq!(value, StatValue::Fixed(raw_fx(expected)));
    }
}

#[test]
fn non_numeric_derived_stats_pass_literals_and_references_through() {
    let fx = fixture();
    let pipeline = stack_pipeline(vec![
        def(fx.stats[0], StatKind::Bool, None),
        def(fx.stats[1], StatKind::Bool, Some(Expr::Stat(fx.stats[0]))),
        def(fx.stats[2], enum_kind("ed", &["ex", "ey"]), None),
        def(
            fx.stats[3],
            enum_kind("ed", &["ex", "ey"]),
            Some(Expr::Literal(enum_value("ed", "ey"))),
        ),
        def(fx.stats[4], StatKind::Tags, None),
        def(fx.stats[5], StatKind::Tags, Some(Expr::Stat(fx.stats[4]))),
    ]);
    let stored = block(&[
        (fx.stats[0], StatValue::Bool(true)),
        (fx.stats[2], enum_value("ed", "ex")),
        (fx.stats[4], StatValue::Tags(tag_set(&fx.tags[..2]))),
    ]);
    let (first, _) = query_stat(&pipeline, &fx, &stored, fx.stats[1]).unwrap();
    assert_eq!(first, StatValue::Bool(true));
    let (second, _) = query_stat(&pipeline, &fx, &stored, fx.stats[3]).unwrap();
    assert_eq!(second, enum_value("ed", "ey"));
    let (third, breakdown) = query_stat(&pipeline, &fx, &stored, fx.stats[5]).unwrap();
    assert_eq!(third, StatValue::Tags(tag_set(&fx.tags[..2])));
    assert_eq!(
        breakdown.traces[&fx.stats[5]].dependencies,
        vec![fx.stats[4]]
    );
}

#[test]
fn mixed_kind_operands_are_rejected() {
    let fx = fixture();
    let error = ModifierPipeline::new(
        vec![
            def(fx.stats[0], StatKind::Fixed, None),
            def(
                fx.stats[1],
                StatKind::Int,
                Some(Expr::Add(
                    Box::new(lit(1)),
                    Box::new(Expr::Stat(fx.stats[0])),
                )),
            ),
        ],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::TypeMismatch);
    assert_eq!(
        error.location,
        format!("/definitions/{}/derived", fx.stats[1].index())
    );
}

#[test]
fn final_kind_must_match_the_declaration() {
    let fx = fixture();
    let error = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Fixed, Some(lit(1)))],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::TypeMismatch);
    assert_eq!(
        error.location,
        format!("/definitions/{}/derived", fx.stats[0].index())
    );
}

#[test]
fn unknown_enum_literal_domains_and_members_are_rejected() {
    let fx = fixture();
    let unknown_domain = ModifierPipeline::new(
        vec![def(
            fx.stats[0],
            enum_kind("ed", &["ex"]),
            Some(Expr::Literal(enum_value("nope", "ex"))),
        )],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(unknown_domain.code, RulesErrorCode::InvalidEnum);
    let unknown_member = ModifierPipeline::new(
        vec![def(
            fx.stats[0],
            enum_kind("ed", &["ex"]),
            Some(Expr::Literal(enum_value("ed", "ez"))),
        )],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(unknown_member.code, RulesErrorCode::InvalidEnum);
}

#[test]
fn inconsistent_enum_domains_are_rejected() {
    let fx = fixture();
    let error = ModifierPipeline::new(
        vec![
            def(fx.stats[0], enum_kind("ed", &["ex"]), None),
            def(fx.stats[1], enum_kind("ed", &["ex", "ey"]), None),
        ],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::InvalidEnum);
    assert_eq!(
        error.location,
        format!("/definitions/{}/kind", fx.stats[1].index())
    );
}

#[test]
fn runtime_division_by_zero_fails_for_int_and_fixed() {
    let fx = fixture();
    let pipeline = stack_pipeline(vec![
        def(
            fx.stats[0],
            StatKind::Int,
            Some(Expr::Divide(Box::new(lit(7)), Box::new(lit(0)))),
        ),
        def(
            fx.stats[1],
            StatKind::Fixed,
            Some(Expr::Divide(
                Box::new(Expr::Literal(StatValue::Fixed(raw_fx(7)))),
                Box::new(Expr::Literal(StatValue::Fixed(raw_fx(0)))),
            )),
        ),
    ]);
    let stored = StatBlock::new();
    let error = query_stat(&pipeline, &fx, &stored, fx.stats[0]).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::DivisionByZero);
    assert_eq!(error.location, format!("/stats/{}", fx.stats[0].index()));
    let error = query_stat(&pipeline, &fx, &stored, fx.stats[1]).unwrap_err();
    assert_eq!(error.code, RulesErrorCode::DivisionByZero);
    assert_eq!(error.location, format!("/stats/{}", fx.stats[1].index()));
}

#[test]
fn int_division_floors_and_min_over_neg_one_saturates() {
    let fx = fixture();
    // -7 / 2 floors to -4; 7 / -2 floors to -4; MIN / -1 saturates to MAX.
    let cases: Vec<(i32, i32, i32)> = vec![
        (-7, 2, -4),
        (7, -2, -4),
        (7, 2, 3),
        (-7, -2, 3),
        (i32::MIN, -1, i32::MAX),
    ];
    for (left, right, expected) in cases {
        let pipeline = stack_pipeline(vec![def(
            fx.stats[0],
            StatKind::Int,
            Some(Expr::Divide(Box::new(lit(left)), Box::new(lit(right)))),
        )]);
        let stored = StatBlock::new();
        let (value, _) = query_stat(&pipeline, &fx, &stored, fx.stats[0]).unwrap();
        assert_eq!(value, StatValue::Int(expected));
    }
}

#[test]
fn int_expression_arithmetic_saturates_per_operation() {
    let fx = fixture();
    // (MAX + 1) - 1 saturates the addition first, then subtracts: MAX - 1.
    // Regrouped as MAX + (1 - 1) the answer would be MAX, which is wrong.
    let pipeline = stack_pipeline(vec![def(
        fx.stats[0],
        StatKind::Int,
        Some(Expr::Subtract(
            Box::new(Expr::Add(
                Box::new(Expr::Literal(StatValue::Int(i32::MAX))),
                Box::new(lit(1)),
            )),
            Box::new(lit(1)),
        )),
    )]);
    let stored = StatBlock::new();
    let (value, _) = query_stat(&pipeline, &fx, &stored, fx.stats[0]).unwrap();
    assert_eq!(value, StatValue::Int(i32::MAX - 1));
}

/// Builds a complete binary tree of `Add` nodes over one-valued leaves.
fn wide_expr(leaves: usize) -> Expr {
    let mut level: Vec<Expr> = (0..leaves).map(|_| lit(1)).collect();
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        let mut iter = level.into_iter();
        while let Some(first) = iter.next() {
            match iter.next() {
                Some(second) => next.push(Expr::Add(Box::new(first), Box::new(second))),
                None => next.push(first),
            }
        }
        level = next;
    }
    level.into_iter().next().unwrap_or_else(|| lit(1))
}

/// Builds an `Add` chain of `adds` nested additions over one-valued leaves.
fn chain_expr(adds: usize) -> Expr {
    let mut expr = lit(1);
    for _ in 0..adds {
        expr = Expr::Add(Box::new(lit(1)), Box::new(expr));
    }
    expr
}

#[test]
fn expression_node_boundaries_respect_binary_parity() {
    let fx = fixture();
    // A 2048-leaf complete tree holds 4095 nodes: the largest valid size.
    let valid = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Int, Some(wide_expr(2048)))],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    );
    assert!(valid.is_ok());
    // A 2049-leaf complete tree holds 4097 nodes: the first over-limit size.
    let error = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Int, Some(wide_expr(2049)))],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::LimitExceeded);
    assert_eq!(
        error.location,
        format!("/definitions/{}/derived", fx.stats[0].index())
    );
}

#[test]
fn expression_depth_boundaries() {
    let fx = fixture();
    // 31 nested additions read as depth 32: the largest valid depth.
    let valid = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Int, Some(chain_expr(31)))],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    );
    assert!(valid.is_ok());
    // 32 nested additions read as depth 33: the first over-limit depth.
    let error = ModifierPipeline::new(
        vec![def(fx.stats[0], StatKind::Int, Some(chain_expr(32)))],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::LimitExceeded);
    assert_eq!(
        error.location,
        format!("/definitions/{}/derived", fx.stats[0].index())
    );
}

#[test]
fn total_expression_node_boundaries() {
    let mut fx = fixture();
    // Eighteen definitions totalling exactly 65536 nodes: valid.
    let mut definitions = Vec::new();
    for index in 0..18 {
        let stat = fx.interners.intern_stat(&format!("bulk-{index:02}"));
        let leaves = if index < 17 { 1821 } else { 1820 };
        definitions.push(def(stat, StatKind::Int, Some(wide_expr(leaves))));
    }
    let valid = ModifierPipeline::new(
        definitions,
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    );
    assert!(valid.is_ok());
    // One more literal node tips the set to 65537: over the limit.
    let mut fx = fixture();
    let mut definitions = Vec::new();
    for index in 0..18 {
        let stat = fx.interners.intern_stat(&format!("bulk-{index:02}"));
        let leaves = if index < 17 { 1821 } else { 1820 };
        definitions.push(def(stat, StatKind::Int, Some(wide_expr(leaves))));
    }
    let extra = fx.interners.intern_stat("bulk-extra");
    definitions.push(def(extra, StatKind::Int, Some(lit(0))));
    let error = ModifierPipeline::new(
        definitions,
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::LimitExceeded);
    assert_eq!(error.location, "/definitions");
}

#[test]
fn derived_depth_boundaries_count_endpoints() {
    let mut fx = fixture();
    // 127 derived links over one base read as a 128-endpoint path: valid.
    let mut definitions = Vec::new();
    let base = fx.interners.intern_stat("depth-000");
    definitions.push(int_def(base));
    let mut previous = base;
    for index in 1..128 {
        let stat = fx.interners.intern_stat(&format!("depth-{index:03}"));
        definitions.push(def(
            stat,
            StatKind::Int,
            Some(Expr::Add(Box::new(Expr::Stat(previous)), Box::new(lit(0)))),
        ));
        previous = stat;
    }
    let valid = ModifierPipeline::new(
        definitions,
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    );
    assert!(valid.is_ok());
    // One more link reads as 129 endpoints: over the limit.
    let mut fx = fixture();
    let mut definitions = Vec::new();
    let base = fx.interners.intern_stat("depth-000");
    definitions.push(int_def(base));
    let mut previous = base;
    for index in 1..129 {
        let stat = fx.interners.intern_stat(&format!("depth-{index:03}"));
        definitions.push(def(
            stat,
            StatKind::Int,
            Some(Expr::Add(Box::new(Expr::Stat(previous)), Box::new(lit(0)))),
        ));
        previous = stat;
    }
    let error = ModifierPipeline::new(
        definitions,
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::LimitExceeded);
}

#[test]
fn definition_count_boundaries() {
    let mut fx = fixture();
    let mut definitions = Vec::new();
    for index in 0..1024 {
        definitions.push(int_def(
            fx.interners.intern_stat(&format!("slot-{index:04}")),
        ));
    }
    let valid = ModifierPipeline::new(
        definitions,
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    );
    assert!(valid.is_ok());
    let mut fx = fixture();
    let mut definitions = Vec::new();
    for index in 0..1025 {
        definitions.push(int_def(
            fx.interners.intern_stat(&format!("slot-{index:04}")),
        ));
    }
    let error = ModifierPipeline::new(
        definitions,
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::LimitExceeded);
    assert_eq!(error.location, "/definitions");
}

#[test]
fn contributions_cover_dependency_touched_modifiers() {
    let fx = fixture();
    // Modifiers on the dependency appear in its trace, not the root's.
    let pipeline = stack_pipeline(vec![
        int_def(fx.stats[0]),
        def(
            fx.stats[1],
            StatKind::Int,
            Some(Expr::Add(
                Box::new(Expr::Stat(fx.stats[0])),
                Box::new(lit(0)),
            )),
        ),
    ]);
    let stored = block(&[(fx.stats[0], StatValue::Int(2))]);
    let tags = TagSet::new();
    let mods = vec![modifier(
        1,
        "sk",
        1,
        fx.stats[0],
        ModOp::Add(StatValue::Int(3)),
        common::TYPE_A,
        None,
        None,
        0,
    )];
    let ctx = context(fx.entity, &stored, &tags, &mods);
    let (value, breakdown) = pipeline.query(fx.entity, fx.stats[1], &ctx).unwrap();
    assert_eq!(value, StatValue::Int(5));
    assert!(breakdown.traces[&fx.stats[1]].contributions.is_empty());
    assert_eq!(breakdown.traces[&fx.stats[0]].contributions.len(), 1);
    assert_eq!(
        breakdown.traces[&fx.stats[0]].contributions[0].status,
        ContributionStatus::Applied {
            before: StatValue::Int(2),
            after: StatValue::Int(5),
        }
    );
}

#[test]
fn enum_variant_count_boundaries() {
    let fx = fixture();
    // 1024 variants in one domain: the largest valid size.
    let variants: BTreeSet<String> = (0..1024).map(|index| format!("v-{index:04}")).collect();
    assert_eq!(variants.len(), 1024);
    let valid = ModifierPipeline::new(
        vec![def(
            fx.stats[0],
            StatKind::Enum {
                enum_id: String::from("ed"),
                variants,
            },
            None,
        )],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    );
    assert!(valid.is_ok());
    // 1025 variants: the first over-limit size.
    let fx = fixture();
    let variants: BTreeSet<String> = (0..1025).map(|index| format!("v-{index:04}")).collect();
    let error = ModifierPipeline::new(
        vec![def(
            fx.stats[0],
            StatKind::Enum {
                enum_id: String::from("ed"),
                variants,
            },
            None,
        )],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::LimitExceeded);
    assert_eq!(
        error.location,
        format!("/definitions/{}/kind", fx.stats[0].index())
    );
    // An empty variant set is below the 1..=MAX bound, not a valid domain.
    let fx = fixture();
    let error = ModifierPipeline::new(
        vec![def(
            fx.stats[0],
            StatKind::Enum {
                enum_id: String::from("ed"),
                variants: BTreeSet::new(),
            },
            None,
        )],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::LimitExceeded);
    assert_eq!(
        error.location,
        format!("/definitions/{}/kind", fx.stats[0].index())
    );
}

#[test]
fn tag_literal_count_boundaries() {
    let mut fx = fixture();
    // A 1024-tag literal in a derived Tags formula: the largest valid size.
    let mut tags: TagSet = fx.tags.iter().copied().collect();
    for index in 0..1020 {
        tags.insert(fx.interners.intern_tag(&format!("lt-{index:04}")));
    }
    assert_eq!(tags.len(), 1024);
    let pipeline = ModifierPipeline::new(
        vec![def(
            fx.stats[0],
            StatKind::Tags,
            Some(Expr::Literal(StatValue::Tags(tags))),
        )],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    let stored = StatBlock::new();
    let (value, breakdown) = query_stat(&pipeline, &fx, &stored, fx.stats[0]).unwrap();
    assert_eq!(breakdown.traces[&fx.stats[0]].contributions.len(), 0);
    let StatValue::Tags(resolved) = value else {
        panic!("expected a Tags value");
    };
    assert_eq!(resolved.len(), 1024);
    // A 1025-tag literal: the first over-limit size.
    let mut fx = fixture();
    let mut tags: TagSet = fx.tags.iter().copied().collect();
    for index in 0..1021 {
        tags.insert(fx.interners.intern_tag(&format!("lt-{index:04}")));
    }
    assert_eq!(tags.len(), 1025);
    let error = ModifierPipeline::new(
        vec![def(
            fx.stats[0],
            StatKind::Tags,
            Some(Expr::Literal(StatValue::Tags(tags))),
        )],
        policies(&[(common::TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap_err();
    assert_eq!(error.code, RulesErrorCode::LimitExceeded);
    assert_eq!(
        error.location,
        format!("/definitions/{}/derived", fx.stats[0].index())
    );
}
