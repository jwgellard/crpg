//! Modifier properties: permutation invariance, source-removal reversal,
//! and integer/fixed scaling oracles.

#[allow(dead_code)]
#[path = "common/mod.rs"]
mod common;

use crpg_rules::{ModOp, Modifier, ModifierPipeline, StackingPolicy, StatValue, TagSet};
use proptest::prelude::*;

use common::{block, context, fixture, int_def, modifier, policies, raw_fx, uid, Fixture, TYPE_A};

fn pipeline_three_types() -> (Fixture, ModifierPipeline) {
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![int_def(fx.stats[0])],
        policies(&[
            (TYPE_A, StackingPolicy::StackAll),
            (common::TYPE_B, StackingPolicy::StackAll),
            (common::TYPE_C, StackingPolicy::HighestPriorityPerName),
        ]),
    )
    .unwrap();
    (fx, pipeline)
}

/// Integers biased toward saturation and rounding edges.
fn edge_int() -> impl Strategy<Value = i32> {
    prop_oneof![
        Just(i32::MAX),
        Just(i32::MIN),
        Just(0),
        Just(1),
        Just(-1),
        any::<i32>(),
    ]
}

fn param_strategy() -> impl Strategy<Value = (u8, i32, i32, i16, u8, u8)> {
    (0..4u8, edge_int(), edge_int(), any::<i16>(), 0..3u8, 0..3u8)
}

/// Builds one valid modifier from generated params with a position identity.
fn build_param(
    params: &(u8, i32, i32, i16, u8, u8),
    id: u128,
    fx: &Fixture,
    source: (&str, u128),
) -> Modifier {
    let (op_tag, first, second, priority, type_idx, name_idx) = *params;
    let target = fx.stats[0];
    let op = match op_tag {
        0 => ModOp::Set(StatValue::Int(first)),
        1 => ModOp::Add(StatValue::Int(first)),
        2 => ModOp::Multiply(raw_fx(first)),
        _ => ModOp::Clamp {
            min: StatValue::Int(first.min(second)),
            max: StatValue::Int(first.max(second)),
        },
    };
    let (mod_type, name) = match type_idx {
        0 => (TYPE_A, None),
        1 => (common::TYPE_B, None),
        _ => (common::TYPE_C, Some(format!("nm-{}", name_idx % 3))),
    };
    Modifier {
        id: uid(id),
        source: crpg_rules::SourceRef {
            kind: source.0.to_owned(),
            id: uid(source.1),
        },
        target: crpg_rules::ModifierTarget::Stat(target),
        op,
        mod_type: crpg_rules::ModTypeId(mod_type.to_owned()),
        name,
        condition: None,
        priority,
    }
}

fn run_query(
    pipeline: &ModifierPipeline,
    fx: &Fixture,
    base: i32,
    mods: &[Modifier],
) -> Result<(StatValue, crpg_rules::ModifierBreakdown), crpg_rules::RulesError> {
    let stored = block(&[(fx.stats[0], StatValue::Int(base))]);
    let tags = TagSet::new();
    let ctx = context(fx.entity, &stored, &tags, mods);
    pipeline.query(fx.entity, fx.stats[0], &ctx)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, ..ProptestConfig::default() })]

    #[test]
    fn permuting_modifiers_preserves_result_and_breakdown(
        rows in prop::collection::vec((param_strategy(), any::<u64>()), 0..24),
    ) {
        let (fx, pipeline) = pipeline_three_types();
        let mods: Vec<Modifier> = rows
            .iter()
            .enumerate()
            .map(|(index, (params, _))| {
                build_param(params, index as u128 + 1, &fx, ("sk", index as u128 + 1))
            })
            .collect();
        // A random permutation via generated sort keys, ties by old index.
        let mut keyed: Vec<(u64, usize)> = rows
            .iter()
            .enumerate()
            .map(|(index, (_, key))| (*key, index))
            .collect();
        keyed.sort();
        let permuted: Vec<Modifier> = keyed.iter().map(|(_, index)| mods[*index].clone()).collect();
        let first = run_query(&pipeline, &fx, 7, &mods).unwrap();
        let second = run_query(&pipeline, &fx, 7, &permuted).unwrap();
        prop_assert_eq!(first, second);
    }

    #[test]
    fn removing_a_source_reverses_exactly(
        base_rows in prop::collection::vec(param_strategy(), 0..12),
        extra_rows in prop::collection::vec(param_strategy(), 1..6),
    ) {
        let (fx, pipeline) = pipeline_three_types();
        let base_mods: Vec<Modifier> = base_rows
            .iter()
            .enumerate()
            .map(|(index, params)| build_param(params, index as u128 + 1, &fx, ("sk", index as u128 + 1)))
            .collect();
        let mut augmented = base_mods.clone();
        for (offset, params) in extra_rows.iter().enumerate() {
            let id = 5000 + offset as u128;
            augmented.push(build_param(params, id, &fx, ("extra-kind", 777777)));
        }
        let original = run_query(&pipeline, &fx, -12345, &base_mods).unwrap();
        let grown = run_query(&pipeline, &fx, -12345, &augmented).unwrap();
        // The extra source always contributes, so the pair always changes.
        prop_assert!(grown != original);
        // Removing exactly that source restores the original pair exactly,
        // with survivors byte-identical to the base slice.
        let surviving: Vec<Modifier> = augmented
            .iter()
            .filter(|modifier| !(modifier.source.kind == "extra-kind"
                && modifier.source.id == uid(777777)))
            .cloned()
            .collect();
        prop_assert_eq!(surviving.clone(), base_mods);
        let restored = run_query(&pipeline, &fx, -12345, &surviving).unwrap();
        prop_assert_eq!(restored, original);
    }

    #[test]
    fn int_scaling_matches_the_floor_oracle((value, raw) in (edge_int(), edge_int())) {
        let (fx, pipeline) = pipeline_three_types();
        let mods = vec![build_param(&(2u8, raw, 0, 0, 0u8, 0u8), 1, &fx, ("sk", 1))];
        let (result, _) = run_query(&pipeline, &fx, value, &mods).unwrap();
        // Independent oracle: Euclidean division by a positive divisor is
        // floor division, unlike the truncating-adjust path under test.
        let product = i64::from(value) * i64::from(raw);
        let expected = product.div_euclid(65536).clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        prop_assert_eq!(result, StatValue::Int(expected));
    }

    #[test]
    fn fixed_multiply_matches_the_raw_oracle((first, second) in (edge_int(), edge_int())) {
        let fx = fixture();
        let pipeline = ModifierPipeline::new(
            vec![common::def(fx.stats[0], crpg_rules::StatKind::Fixed, None)],
            policies(&[(TYPE_A, StackingPolicy::StackAll)]),
        )
        .unwrap();
        let stored = block(&[(fx.stats[0], StatValue::Fixed(raw_fx(first)))]);
        let tags = TagSet::new();
        let mods = vec![common::modifier(
            1,
            "sk",
            1,
            fx.stats[0],
            ModOp::Multiply(raw_fx(second)),
            TYPE_A,
            None,
            None,
            0,
        )];
        let ctx = context(fx.entity, &stored, &tags, &mods);
        let (result, _) = pipeline.query(fx.entity, fx.stats[0], &ctx).unwrap();
        // Independent raw-integer expectation: arithmetic shift floors.
        let expected =
            ((i64::from(first) * i64::from(second)) >> 16).clamp(i32::MIN as i64, i32::MAX as i64)
                as i32;
        prop_assert_eq!(result, StatValue::Fixed(raw_fx(expected)));
    }
}

fn without_extra(mods: &[Modifier]) -> Vec<Modifier> {
    mods.iter()
        .filter(|modifier| {
            !(modifier.source.kind == "extra-kind" && modifier.source.id == uid(777777))
        })
        .cloned()
        .collect()
}

#[test]
fn removal_reversal_covers_value_change_saturation_and_competition() {
    // Deterministic companion to the reversal property: hand-built cases that
    // provably change the value (rounding scale), saturate without changing
    // the value, and steal a stacking slot, each exactly reversed by removing
    // one fresh source.
    let (fx, pipeline) = pipeline_three_types();
    // Rounding scale: -3 * 0.5 floors to -2; removal restores -3 exactly.
    let extra_a = vec![modifier(
        50,
        "extra-kind",
        777777,
        fx.stats[0],
        ModOp::Multiply(raw_fx(32768)),
        TYPE_A,
        None,
        None,
        0,
    )];
    let original_a = run_query(&pipeline, &fx, -3, &[]).unwrap();
    let grown_a = run_query(&pipeline, &fx, -3, &extra_a).unwrap();
    assert_eq!(grown_a.0, StatValue::Int(-2));
    assert_ne!(grown_a, original_a);
    assert_eq!(without_extra(&extra_a), Vec::<Modifier>::new());
    assert_eq!(
        run_query(&pipeline, &fx, -3, &without_extra(&extra_a)).unwrap(),
        original_a
    );
    // Saturation: MAX + 1 stays MAX but the breakdown gains a contribution;
    // removal restores the exact original pair.
    let extra_b = vec![modifier(
        51,
        "extra-kind",
        777777,
        fx.stats[0],
        ModOp::Add(StatValue::Int(1)),
        TYPE_A,
        None,
        None,
        0,
    )];
    let original_b = run_query(&pipeline, &fx, i32::MAX, &[]).unwrap();
    let grown_b = run_query(&pipeline, &fx, i32::MAX, &extra_b).unwrap();
    assert_eq!(grown_b.0, StatValue::Int(i32::MAX));
    assert_ne!(grown_b.1, original_b.1);
    assert_eq!(
        run_query(&pipeline, &fx, i32::MAX, &without_extra(&extra_b)).unwrap(),
        original_b
    );
    // Stacking competition: the extra source steals the per-name slot with a
    // higher priority, then removal hands it back to the original winner.
    let base_c = vec![modifier(
        60,
        "sk",
        60,
        fx.stats[0],
        ModOp::Add(StatValue::Int(100)),
        common::TYPE_C,
        Some("nm-0"),
        None,
        0,
    )];
    let mut grown_c_mods = base_c.clone();
    grown_c_mods.push(modifier(
        61,
        "extra-kind",
        777777,
        fx.stats[0],
        ModOp::Add(StatValue::Int(1)),
        common::TYPE_C,
        Some("nm-0"),
        None,
        5,
    ));
    let original_c = run_query(&pipeline, &fx, 0, &base_c).unwrap();
    assert_eq!(original_c.0, StatValue::Int(100));
    let grown_c = run_query(&pipeline, &fx, 0, &grown_c_mods).unwrap();
    assert_eq!(grown_c.0, StatValue::Int(1));
    let surviving_c = without_extra(&grown_c_mods);
    assert_eq!(surviving_c, base_c);
    assert_eq!(
        run_query(&pipeline, &fx, 0, &surviving_c).unwrap(),
        original_c
    );
}

#[test]
fn fixed_edge_vectors_use_hand_computed_raws() {
    // (base raw, factor raw, expected raw), each verified by hand.
    let vectors: Vec<(i32, i32, i32)> = vec![
        (65536, 65536, 65536),
        (i32::MAX, 65536, i32::MAX),
        (i32::MAX, 131072, i32::MAX),
        (i32::MIN, 65536, i32::MIN),
        (i32::MIN, i32::MIN, i32::MAX),
        (1, 1, 0),
        (-65536, 65536, -65536),
        (3, 3, 0),
        (-3, 65536, -3),
        (100000, 65536, 100000),
        (i32::MIN, -65536, i32::MAX),
        (0, 12345, 0),
    ];
    let fx = fixture();
    let pipeline = ModifierPipeline::new(
        vec![common::def(fx.stats[0], crpg_rules::StatKind::Fixed, None)],
        policies(&[(TYPE_A, StackingPolicy::StackAll)]),
    )
    .unwrap();
    for (base, factor, expected) in vectors {
        let stored = block(&[(fx.stats[0], StatValue::Fixed(raw_fx(base)))]);
        let tags = TagSet::new();
        let mods = vec![common::modifier(
            1,
            "sk",
            1,
            fx.stats[0],
            ModOp::Multiply(raw_fx(factor)),
            TYPE_A,
            None,
            None,
            0,
        )];
        let ctx = context(fx.entity, &stored, &tags, &mods);
        let (result, _) = pipeline.query(fx.entity, fx.stats[0], &ctx).unwrap();
        assert_eq!(result, StatValue::Fixed(raw_fx(expected)));
    }
}
