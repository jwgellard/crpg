//! T140b two-keys contract tests: protocol v3 fact ops (tags 11..=13), the
//! `check_*` functions, `frame_spans`, the reachable frame cap and
//! `projection_v3::project_events`. Written from `tasks/T140.md` alone; every
//! byte oracle is hand-derived (postcard: LEB128 varints, zigzag signed,
//! one-byte bools, length-prefixed strings) and never produced by running the
//! codec under test.

// `frame_spans` returns a `Vec` of ranges; one-range expectations are the point.
#![allow(clippy::single_range_in_vec_init)]

use crpg_core::Ulid;
use crpg_net::codec::CodecError;
use crpg_net::codec_v3 as c3;
use crpg_net::projection_v2 as pv2;
use crpg_net::projection_v3::{self as pr3, EventCandidate, ProjectionError};
use crpg_net::protocol_v3::{
    self as p3, ClockFact, DeltaFrame, DeltaOp, EffectFact, EncounterFacts, EncounterMode, FactSet,
    NetId, OrderFact, PoolFact, StatFact, StatValue, TimelineEntry,
};

type Verdict = Result<(), CodecError>;

const MAX64: [u8; 10] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];
const NO: &[u8] = &[0];
const TB: EncounterMode = EncounterMode::TurnBased;
const RT: EncounterMode = EncounterMode::RealTime;
const MALFORMED: CodecError = CodecError::Malformed;
const LIMIT: CodecError = CodecError::LimitExceeded;
const UNKNOWN: CodecError = CodecError::UnknownMessage;

fn net(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero fixture id")
}

fn ulid(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

/// Length-prefixed ULID text of `ulid(n)` (26 bytes, prefix `1A`).
fn utext(n: u128) -> Vec<u8> {
    let text = ulid(n).to_string();
    assert_eq!(text.len(), 26);
    [&[0x1A][..], text.as_bytes()].concat()
}

fn varint(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

fn zigzag(v: i32) -> Vec<u8> {
    varint(u64::from(((v << 1) ^ (v >> 31)) as u32))
}

fn dfr(ops: Vec<DeltaOp>) -> DeltaFrame {
    DeltaFrame {
        lane: 0,
        server_tick: 100,
        event_seq: 1,
        ops,
    }
}

/// One-op delta frame bytes: `03 00 64 01 01` plus the op bytes.
fn d1(op: &[u8]) -> Vec<u8> {
    [&[3, 0, 0x64, 1, 1][..], op].concat()
}

/// Oracle for a one-op frame: encodes to `d1(bytes)`, decodes back, and
/// `encode(decode(bytes)) == bytes`.
fn check_op(op: DeltaOp, bytes: &[u8]) {
    let frame = dfr(vec![op]);
    let wire = d1(bytes);
    assert_eq!(c3::encode_delta(&frame), Ok(wire.clone()), "{bytes:02X?}");
    assert_eq!(c3::decode_delta(&wire), Ok(frame));
    assert_eq!(
        c3::encode_delta(&c3::decode_delta(&wire).unwrap()),
        Ok(wire)
    );
}

fn dec_err(bytes: &[u8], err: CodecError) {
    assert_eq!(c3::decode_delta(&d1(bytes)), Err(err), "{bytes:02X?}");
}

fn enc_err(op: DeltaOp, err: CodecError) {
    assert_eq!(c3::encode_delta(&dfr(vec![op])), Err(err));
}

fn encounter(
    mode: Option<EncounterMode>,
    pending_mode: Option<Option<EncounterMode>>,
    clock: Option<Option<ClockFact>>,
) -> DeltaOp {
    DeltaOp::Encounter(EncounterFacts {
        mode,
        pending_mode,
        clock,
    })
}

fn clock(round_ticks: u32, elapsed: u64) -> Option<Option<ClockFact>> {
    Some(Some(ClockFact {
        round_ticks,
        elapsed,
    }))
}

fn timeline(entries: &[(i32, u64)]) -> DeltaOp {
    DeltaOp::Timeline {
        entries: Some(
            entries
                .iter()
                .map(|&(key, entity)| TimelineEntry {
                    key,
                    entity: net(entity),
                })
                .collect(),
        ),
    }
}

fn facts_op(facts: FactSet) -> DeltaOp {
    DeltaOp::EntityFacts {
        entity: net(2),
        facts,
    }
}

/// `0D 02` plus the five family encodings, each including its own flag.
fn efb(pools: &[u8], effects: &[u8], order: &[u8], stats: &[u8], tags: &[u8]) -> Vec<u8> {
    [&[0x0D, 2][..], pools, effects, order, stats, tags].concat()
}

/// A disclosed list: flag `01`, varint count, then the items.
fn list(items: &[Vec<u8>]) -> Vec<u8> {
    [&[1][..], &varint(items.len() as u64), &items.concat()].concat()
}

fn pool_b(n: u128, current: u32, max: u32) -> Vec<u8> {
    [utext(n), varint(u64::from(current)), varint(u64::from(max))].concat()
}

fn effect_b(n: u128, expires: u64) -> Vec<u8> {
    [utext(n), varint(expires)].concat()
}

fn str_b(s: &str) -> Vec<u8> {
    [&varint(s.len() as u64)[..], s.as_bytes()].concat()
}

fn stat_b(name: &str, kind: u8, value: i32) -> Vec<u8> {
    [&str_b(name)[..], &[kind], &zigzag(value)].concat()
}

fn tags_b(tags: &[&str]) -> Vec<u8> {
    list(&tags.iter().map(|t| str_b(t)).collect::<Vec<_>>())
}

fn pool(n: u128, current: u32, max: u32) -> PoolFact {
    PoolFact {
        pool: ulid(n),
        current,
        max,
    }
}

fn effect(n: u128, expires_round: u64) -> EffectFact {
    EffectFact {
        effect: ulid(n),
        expires_round,
    }
}

fn int_stat(name: &str, v: i32) -> StatFact {
    StatFact {
        name: name.to_string(),
        value: StatValue::Int(v),
    }
}

fn fixed_stat(name: &str, v: i32) -> StatFact {
    StatFact {
        name: name.to_string(),
        value: StatValue::Fixed(v),
    }
}

fn pools_only(pools: Vec<PoolFact>) -> DeltaOp {
    facts_op(FactSet {
        pools: Some(pools),
        ..FactSet::default()
    })
}

fn effects_only(effects: Vec<EffectFact>) -> DeltaOp {
    facts_op(FactSet {
        effects: Some(effects),
        ..FactSet::default()
    })
}

fn stats_only(stats: Vec<StatFact>) -> DeltaOp {
    facts_op(FactSet {
        stats: Some(stats),
        ..FactSet::default()
    })
}

fn tags_only(tags: &[&str]) -> DeltaOp {
    facts_op(FactSet {
        tags: Some(tags.iter().map(|t| (*t).to_string()).collect()),
        ..FactSet::default()
    })
}

fn pools_n(n: u128) -> Vec<PoolFact> {
    (1..=n).map(|i| pool(i, 0, 0)).collect()
}

fn effects_n(n: u128) -> Vec<EffectFact> {
    (1..=n).map(|i| effect(i, 0)).collect()
}

fn stats_n(n: usize) -> Vec<StatFact> {
    (0..n).map(|i| int_stat(&format!("s{i:02}"), 0)).collect()
}

fn tag_names(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("t{i:02}")).collect()
}

fn tags_n(n: usize) -> DeltaOp {
    facts_op(FactSet {
        tags: Some(tag_names(n)),
        ..FactSet::default()
    })
}

#[test]
fn b1_encounter_literals() {
    assert_eq!(p3::DELTA_TAG_ENCOUNTER, 11);
    assert_eq!(p3::MAX_CLOCK_ELAPSED, 2_147_483_647);
    check_op(encounter(None, None, None), &[0x0B, 0, 0, 0]);
    check_op(encounter(Some(TB), None, None), &[0x0B, 1, 0, 0, 0]);
    check_op(encounter(None, Some(None), None), &[0x0B, 0, 1, 0, 0]);
    check_op(encounter(None, None, Some(None)), &[0x0B, 0, 0, 1, 0]);
    check_op(
        encounter(Some(TB), Some(Some(RT)), clock(120, 240)),
        &[0x0B, 1, 0, 1, 1, 1, 1, 1, 0x78, 0xF0, 0x01],
    );
    // Extremes of every field.
    check_op(
        encounter(None, None, clock(1, 0)),
        &[0x0B, 0, 0, 1, 1, 1, 0],
    );
    check_op(
        encounter(None, None, clock(u32::MAX, 2_147_483_647)),
        &[
            0x0B, 0, 0, 1, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0xFF, 0xFF, 0xFF, 0xFF, 0x07,
        ],
    );
    // All 36 combinations, with bytes derived from the layout.
    let modes = [
        (None, vec![0]),
        (Some(TB), vec![1, 0]),
        (Some(RT), vec![1, 1]),
    ];
    let pendings = [
        (None, vec![0]),
        (Some(None), vec![1, 0]),
        (Some(Some(TB)), vec![1, 1, 0]),
        (Some(Some(RT)), vec![1, 1, 1]),
    ];
    let clocks = [
        (None, vec![0]),
        (Some(None), vec![1, 0]),
        (clock(120, 240), vec![1, 1, 0x78, 0xF0, 0x01]),
    ];
    let mut combos = 0;
    for (mode, mb) in &modes {
        for (pending, pb) in &pendings {
            for (clk, cb) in &clocks {
                let bytes = [&[0x0B][..], mb, pb, cb].concat();
                check_op(encounter(*mode, *pending, *clk), &bytes);
                combos += 1;
            }
        }
    }
    assert_eq!(combos, 36);
    // Refusals (left to right, each checked when read).
    for bad in [2, 0xFF] {
        dec_err(&[0x0B, 1, bad, 0, 0], UNKNOWN);
        dec_err(&[0x0B, 0, 1, 1, bad, 0], UNKNOWN);
    }
    dec_err(&[0x0B, 1, 2, 2], UNKNOWN); // mode byte before the later bad flag
    dec_err(&[0x0B, 0, 0, 1, 1, 0, 0], MALFORMED); // round_ticks 0
    dec_err(
        &[0x0B, 0, 0, 1, 1, 1, 0x80, 0x80, 0x80, 0x80, 0x08],
        MALFORMED,
    ); // elapsed 2^31
    for flags in [&[2][..], &[0, 2], &[0, 1, 2], &[0, 0, 2], &[0, 0, 1, 2]] {
        dec_err(&[&[0x0B][..], flags].concat(), MALFORMED);
    }
    dec_err(&[0x0B, 0, 0], MALFORMED); // truncated
    dec_err(&[0x0B, 0, 0, 0, 0], MALFORMED); // trailing byte
                                             // Encode refuses the same values.
    enc_err(encounter(None, None, clock(0, 5)), MALFORMED);
    enc_err(encounter(None, None, clock(1, 2_147_483_648)), MALFORMED);
    enc_err(encounter(None, None, clock(1, u64::MAX)), MALFORMED);
}

#[test]
fn b1_is_withheld() {
    assert!(EncounterFacts::default().is_withheld());
    let shown = |f: EncounterFacts| assert!(!f.is_withheld());
    shown(EncounterFacts {
        mode: Some(TB),
        ..EncounterFacts::default()
    });
    shown(EncounterFacts {
        pending_mode: Some(None),
        ..EncounterFacts::default()
    });
    shown(EncounterFacts {
        clock: Some(None),
        ..EncounterFacts::default()
    });
    assert!(FactSet::default().is_withheld());
    let shown = |f: FactSet| assert!(!f.is_withheld());
    shown(FactSet {
        pools: Some(vec![]),
        ..FactSet::default()
    });
    shown(FactSet {
        effects: Some(vec![]),
        ..FactSet::default()
    });
    shown(FactSet {
        order: Some(None),
        ..FactSet::default()
    });
    shown(FactSet {
        stats: Some(vec![]),
        ..FactSet::default()
    });
    shown(FactSet {
        tags: Some(vec![]),
        ..FactSet::default()
    });
}

#[test]
fn b2_timeline_literals_and_bounds() {
    assert_eq!(p3::DELTA_TAG_TIMELINE, 12);
    check_op(DeltaOp::Timeline { entries: None }, &[0x0C, 0]);
    check_op(
        DeltaOp::Timeline {
            entries: Some(vec![]),
        },
        &[0x0C, 1, 0],
    );
    // Equal keys, descending NetId: accepted and order preserved.
    check_op(
        timeline(&[(-1, 2), (-1, 1), (5, 3)]),
        &[0x0C, 1, 3, 1, 2, 1, 1, 0x0A, 3],
    );
    check_op(
        timeline(&[(i32::MIN, 1)]),
        &[0x0C, 1, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 1],
    );
    check_op(
        timeline(&[(i32::MAX, 1)]),
        &[0x0C, 1, 1, 0xFE, 0xFF, 0xFF, 0xFF, 0x0F, 1],
    );
    // Key order is by value, not by zigzag bits.
    check_op(timeline(&[(-1, 1), (0, 2)]), &[0x0C, 1, 2, 1, 1, 0, 2]);
    check_op(
        timeline(&[(i32::MIN, 1), (i32::MAX, 2)]),
        &[
            0x0C, 1, 2, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 1, 0xFE, 0xFF, 0xFF, 0xFF, 0x0F, 2,
        ],
    );
    // 1,024 entries are accepted; count bytes `80 08`.
    assert_eq!(p3::MAX_TIMELINE_ENTRIES, 1024);
    let entries: Vec<(i32, u64)> = (0..1024).map(|i| (i, i as u64 + 1)).collect();
    let mut bytes = vec![0x0C, 1, 0x80, 0x08];
    for &(k, e) in &entries {
        bytes.extend(zigzag(k));
        bytes.extend(varint(e));
    }
    check_op(timeline(&entries), &bytes);
    // Refusals.
    dec_err(&[0x0C, 1, 2, 0x0A, 1, 0x08, 2], MALFORMED); // (5,1),(4,2)
    dec_err(&[0x0C, 1, 2, 2, 1, 4, 1], MALFORMED); // (1,1),(2,1)
    dec_err(&[0x0C, 1, 2, 2, 5, 2, 5], MALFORMED); // same entity, equal key
    dec_err(&[0x0C, 1, 2, 0, 1, 1, 2], MALFORMED); // (0,1),(-1,2)
    dec_err(&[0x0C, 1, 1, 2, 0], MALFORMED); // entity 0
    dec_err(&[0x0C, 2], MALFORMED); // flag 02
    dec_err(&[0x0C, 1, 0x81, 0x08], LIMIT); // 1,025, no entries follow
    dec_err(&[0x0C, 1, 0x80, 0x08], MALFORMED); // control: 1,024, truncated
    let mut hostile = vec![0x0C, 1];
    hostile.extend(MAX64);
    dec_err(&hostile, LIMIT);
    let mut overflow = vec![0x0C, 1];
    overflow.extend([0xFF; 9]);
    overflow.push(0x02);
    dec_err(&overflow, MALFORMED);
    // Encode agrees.
    enc_err(timeline(&[(5, 1), (4, 2)]), MALFORMED);
    enc_err(timeline(&[(0, 1), (-1, 2)]), MALFORMED);
    enc_err(timeline(&[(1, 1), (2, 1)]), MALFORMED);
    let too_many: Vec<(i32, u64)> = (0..1025).map(|i| (0, i + 1)).collect();
    enc_err(timeline(&too_many), LIMIT);
}

#[test]
fn b3_entity_facts_literals() {
    assert_eq!(p3::DELTA_TAG_ENTITY_FACTS, 13);
    check_op(facts_op(FactSet::default()), &[0x0D, 2, 0, 0, 0, 0, 0]);
    let empty = FactSet::default;
    check_op(
        facts_op(FactSet {
            pools: Some(vec![]),
            ..empty()
        }),
        &[0x0D, 2, 1, 0, 0, 0, 0, 0],
    );
    check_op(
        facts_op(FactSet {
            effects: Some(vec![]),
            ..empty()
        }),
        &[0x0D, 2, 0, 1, 0, 0, 0, 0],
    );
    check_op(
        facts_op(FactSet {
            order: Some(None),
            ..empty()
        }),
        &[0x0D, 2, 0, 0, 1, 0, 0, 0],
    );
    check_op(
        facts_op(FactSet {
            stats: Some(vec![]),
            ..empty()
        }),
        &[0x0D, 2, 0, 0, 0, 1, 0, 0],
    );
    check_op(
        facts_op(FactSet {
            tags: Some(vec![]),
            ..empty()
        }),
        &[0x0D, 2, 0, 0, 0, 0, 1, 0],
    );
    check_op(
        pools_only(vec![pool(1, 2, 3)]),
        &[&[0x0D, 2, 1, 1][..], &utext(1), &[2, 3, 0, 0, 0, 0]].concat(),
    );
    // The full set is the contract's 123 bytes.
    let full = FactSet {
        pools: Some(vec![pool(1, 2, 3)]),
        effects: Some(vec![effect(2, 4)]),
        order: Some(Some(OrderFact {
            ability: ulid(3),
            target: net(1),
            repeat: true,
        })),
        stats: Some(vec![int_stat("might", 4), fixed_stat("reach", 98304)]),
        tags: Some(vec!["undead".to_string()]),
    };
    let bytes = [
        &[0x0D, 2, 1, 1][..],
        &utext(1),
        &[2, 3, 1, 1],
        &utext(2),
        &[4, 1, 1],
        &utext(3),
        &[1, 1],
        &[1, 2, 5, 0x6D, 0x69, 0x67, 0x68, 0x74, 0, 8],
        &[5, 0x72, 0x65, 0x61, 0x63, 0x68, 1, 0x80, 0x80, 0x0C],
        &[1, 1, 6, 0x75, 0x6E, 0x64, 0x65, 0x61, 0x64],
    ]
    .concat();
    assert_eq!(bytes.len(), 123);
    check_op(facts_op(full), &bytes);
    // Order with repeat false and the largest target.
    check_op(
        facts_op(FactSet {
            order: Some(Some(OrderFact {
                ability: ulid(3),
                target: net(u64::MAX),
                repeat: false,
            })),
            ..empty()
        }),
        &[&[0x0D, 2, 0, 0, 1, 1][..], &utext(3), &MAX64, &[0], &[0, 0]].concat(),
    );
    // Extremes of the numeric fields.
    check_op(
        pools_only(vec![pool(1, 0, 0), pool(2, u32::MAX, u32::MAX)]),
        &efb(
            &list(&[pool_b(1, 0, 0), pool_b(2, u32::MAX, u32::MAX)]),
            NO,
            NO,
            NO,
            NO,
        ),
    );
    check_op(
        effects_only(vec![effect(1, 0), effect(2, u64::MAX)]),
        &efb(
            NO,
            &list(&[effect_b(1, 0), effect_b(2, u64::MAX)]),
            NO,
            NO,
            NO,
        ),
    );
    check_op(
        stats_only(vec![
            int_stat("a", 0),
            int_stat("b", i32::MAX),
            fixed_stat("c", -1),
            fixed_stat("d", i32::MIN),
        ]),
        &efb(
            NO,
            NO,
            NO,
            &list(&[
                stat_b("a", 0, 0),
                stat_b("b", 0, i32::MAX),
                stat_b("c", 1, -1),
                stat_b("d", 1, i32::MIN),
            ]),
            NO,
        ),
    );
    // Withheld and disclosed-empty differ.
    assert_ne!(
        c3::encode_delta(&dfr(vec![facts_op(FactSet::default())])),
        c3::encode_delta(&dfr(vec![tags_only(&[])]))
    );
}

#[test]
fn b4_fact_set_bounds() {
    assert_eq!(p3::MAX_FACT_POOLS, 16);
    assert_eq!(p3::MAX_FACT_EFFECTS, 16);
    assert_eq!(p3::MAX_FACT_STATS, 32);
    assert_eq!(p3::MAX_FACT_TAGS, 8);
    assert_eq!(p3::MAX_FACT_NAME_BYTES, 32);
    // At the cap: hand-built bytes decode and re-encode.
    let at_pools: Vec<Vec<u8>> = (1..=16).map(|i| pool_b(i, 0, 0)).collect();
    check_op(
        pools_only(pools_n(16)),
        &efb(&list(&at_pools), NO, NO, NO, NO),
    );
    let at_effects: Vec<Vec<u8>> = (1..=16).map(|i| effect_b(i, 0)).collect();
    check_op(
        effects_only(effects_n(16)),
        &efb(NO, &list(&at_effects), NO, NO, NO),
    );
    let at_stats: Vec<Vec<u8>> = (0..32).map(|i| stat_b(&format!("s{i:02}"), 0, 0)).collect();
    check_op(
        stats_only(stats_n(32)),
        &efb(NO, NO, NO, &list(&at_stats), NO),
    );
    let at_tags: Vec<Vec<u8>> = (0..8).map(|i| str_b(&format!("t{i:02}"))).collect();
    check_op(tags_n(8), &efb(NO, NO, NO, NO, &list(&at_tags)));
    // One past: LimitExceeded on encode ...
    enc_err(pools_only(pools_n(17)), LIMIT);
    enc_err(effects_only(effects_n(17)), LIMIT);
    enc_err(stats_only(stats_n(33)), LIMIT);
    enc_err(tags_n(9), LIMIT);
    // ... and on decode, from the count alone (counts 17, 33, 9).
    dec_err(&efb(&[1, 0x11], &[], &[], &[], &[]), LIMIT);
    dec_err(&efb(NO, &[1, 0x11], &[], &[], &[]), LIMIT);
    dec_err(&efb(NO, NO, NO, &[1, 0x21], &[]), LIMIT);
    dec_err(&efb(NO, NO, NO, NO, &[1, 0x09]), LIMIT);
    // Controls: the cap count with no elements is truncation, not a limit.
    dec_err(&efb(&[1, 0x10], &[], &[], &[], &[]), MALFORMED);
    dec_err(&efb(NO, &[1, 0x10], &[], &[], &[]), MALFORMED);
    dec_err(&efb(NO, NO, NO, &[1, 0x20], &[]), MALFORMED);
    dec_err(&efb(NO, NO, NO, NO, &[1, 0x08]), MALFORMED);
    // Names and tags: 32 bytes accepted, 33 bytes not, 0 bytes Malformed.
    let n32 = "n".repeat(32);
    let n33 = "n".repeat(33);
    check_op(
        stats_only(vec![int_stat(&n32, 0)]),
        &efb(NO, NO, NO, &list(&[stat_b(&n32, 0, 0)]), NO),
    );
    check_op(
        tags_only(&[&n32]),
        &efb(NO, NO, NO, NO, &list(&[str_b(&n32)])),
    );
    enc_err(stats_only(vec![int_stat(&n33, 0)]), LIMIT);
    enc_err(tags_only(&[&n33]), LIMIT);
    dec_err(&efb(NO, NO, NO, &[1, 1, 0x21], &[]), LIMIT); // no body bytes follow
    dec_err(&efb(NO, NO, NO, NO, &[1, 1, 0x21]), LIMIT);
    dec_err(&efb(NO, NO, NO, &[1, 1, 0x20], &[]), MALFORMED); // control: 32, short body
    dec_err(&efb(NO, NO, NO, &[1, 1, 0], &[]), MALFORMED);
    dec_err(&efb(NO, NO, NO, NO, &[1, 1, 0]), MALFORMED);
    enc_err(stats_only(vec![int_stat("", 0)]), MALFORMED);
    enc_err(tags_only(&[""]), MALFORMED);
    // The bound is in bytes: 16 two-byte characters fit, plus one byte do not.
    let wide = "\u{e9}".repeat(16);
    check_op(
        tags_only(&[&wide]),
        &efb(NO, NO, NO, NO, &list(&[str_b(&wide)])),
    );
    enc_err(tags_only(&[&format!("{wide}a")]), LIMIT);
    enc_err(stats_only(vec![int_stat(&format!("{wide}a"), 0)]), LIMIT);
    // The shortest name is one byte.
    check_op(
        tags_only(&["a"]),
        &efb(NO, NO, NO, NO, &list(&[str_b("a")])),
    );
}

#[test]
fn b5_fact_set_shape() {
    // current <= max: 3/3 accepted, 4/3 refused.
    check_op(
        pools_only(vec![pool(1, 3, 3)]),
        &efb(&list(&[pool_b(1, 3, 3)]), NO, NO, NO, NO),
    );
    dec_err(&efb(&list(&[pool_b(1, 4, 3)]), NO, NO, NO, NO), MALFORMED);
    enc_err(pools_only(vec![pool(1, 4, 3)]), MALFORMED);
    // Pools: duplicates refused (adjacent or not), any other order accepted.
    let dup = [pool_b(1, 1, 1), pool_b(1, 2, 2)];
    dec_err(&efb(&list(&dup), NO, NO, NO, NO), MALFORMED);
    let gap = [pool_b(1, 0, 0), pool_b(2, 0, 0), pool_b(1, 0, 0)];
    dec_err(&efb(&list(&gap), NO, NO, NO, NO), MALFORMED);
    enc_err(pools_only(vec![pool(1, 0, 0), pool(1, 1, 1)]), MALFORMED);
    enc_err(
        pools_only(vec![pool(1, 0, 0), pool(2, 0, 0), pool(1, 0, 0)]),
        MALFORMED,
    );
    check_op(
        pools_only(vec![pool(2, 0, 0), pool(1, 0, 0)]),
        &efb(&list(&[pool_b(2, 0, 0), pool_b(1, 0, 0)]), NO, NO, NO, NO),
    );
    // Effects: strictly ascending by id.
    dec_err(
        &efb(NO, &list(&[effect_b(2, 0), effect_b(1, 0)]), NO, NO, NO),
        MALFORMED,
    );
    dec_err(
        &efb(NO, &list(&[effect_b(1, 0), effect_b(1, 0)]), NO, NO, NO),
        MALFORMED,
    );
    enc_err(effects_only(vec![effect(2, 0), effect(1, 0)]), MALFORMED);
    enc_err(effects_only(vec![effect(1, 0), effect(1, 0)]), MALFORMED);
    // Stats: names unique (not sorted); kind 02 is unknown.
    let dup_stat = list(&[stat_b("a", 0, 0), stat_b("b", 0, 0), stat_b("a", 1, 0)]);
    dec_err(&efb(NO, NO, NO, &dup_stat, NO), MALFORMED);
    enc_err(
        stats_only(vec![int_stat("a", 0), int_stat("b", 0), int_stat("a", 1)]),
        MALFORMED,
    );
    check_op(
        stats_only(vec![int_stat("b", 0), int_stat("a", 0)]),
        &efb(
            NO,
            NO,
            NO,
            &list(&[stat_b("b", 0, 0), stat_b("a", 0, 0)]),
            NO,
        ),
    );
    dec_err(&efb(NO, NO, NO, &[1, 1, 1, 0x61, 2, 0], NO), UNKNOWN);
    dec_err(&efb(NO, NO, NO, &[1, 1, 1, 0x61, 0xFF, 0], NO), UNKNOWN);
    // The kind byte is read before the duplicate check.
    let dup_kind = [1, 2, 1, 0x61, 0, 0, 1, 0x61, 2, 0];
    dec_err(&efb(NO, NO, NO, &dup_kind, NO), UNKNOWN);
    // Zigzag values: Fixed(98304) `80 80 0C`, Int(-3) `05`, Int(MIN).
    check_op(
        stats_only(vec![fixed_stat("a", 98304)]),
        &efb(NO, NO, NO, &[1, 1, 1, 0x61, 1, 0x80, 0x80, 0x0C], NO),
    );
    check_op(
        stats_only(vec![int_stat("a", -3)]),
        &efb(NO, NO, NO, &[1, 1, 1, 0x61, 0, 0x05], NO),
    );
    check_op(
        stats_only(vec![int_stat("a", i32::MIN)]),
        &efb(
            NO,
            NO,
            NO,
            &[1, 1, 1, 0x61, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
            NO,
        ),
    );
    // A value outside i32 (2^32 as a varint) is Malformed.
    dec_err(
        &efb(
            NO,
            NO,
            NO,
            &[1, 1, 1, 0x61, 0, 0x80, 0x80, 0x80, 0x80, 0x10],
            NO,
        ),
        MALFORMED,
    );
    // Tags: strictly ascending in byte order.
    dec_err(&efb(NO, NO, NO, NO, &tags_b(&["b", "a"])), MALFORMED);
    dec_err(&efb(NO, NO, NO, NO, &tags_b(&["a", "a"])), MALFORMED);
    dec_err(&efb(NO, NO, NO, NO, &tags_b(&["a", "B"])), MALFORMED);
    enc_err(tags_only(&["b", "a"]), MALFORMED);
    enc_err(tags_only(&["a", "a"]), MALFORMED);
    check_op(
        tags_only(&["B", "a"]),
        &efb(NO, NO, NO, NO, &tags_b(&["B", "a"])),
    );
    // Order target 0.
    let order0 = [&[1, 1][..], &utext(3), &[0, 1]].concat();
    dec_err(&efb(NO, NO, &order0, NO, NO), MALFORMED);
    // Invalid UTF-8 in a name.
    dec_err(
        &efb(NO, NO, NO, &[1, 1, 2, 0xC3, 0x28, 0, 0], NO),
        MALFORMED,
    );
    dec_err(&efb(NO, NO, NO, NO, &[1, 1, 2, 0xC3, 0x28]), MALFORMED);
    // A flag byte of 02 in any disclosed flag is Malformed.
    let bad: &[u8] = &[2];
    dec_err(&efb(bad, NO, NO, NO, NO), MALFORMED);
    dec_err(&efb(NO, bad, NO, NO, NO), MALFORMED);
    dec_err(&efb(NO, NO, bad, NO, NO), MALFORMED);
    dec_err(&efb(NO, NO, NO, bad, NO), MALFORMED);
    dec_err(&efb(NO, NO, NO, NO, bad), MALFORMED);
    dec_err(&efb(NO, NO, &[1, 2], NO, NO), MALFORMED); // order_some flag
                                                       // An unparsable ULID text and an over-long one.
    let bad_text = [&[1, 1, 0x1A][..], &[b'U'; 26], &[0, 0]].concat();
    dec_err(&efb(&bad_text, NO, NO, NO, NO), MALFORMED);
    dec_err(&efb(&[1, 1, 0x81, 0x02], NO, NO, NO, NO), LIMIT);
    // Truncation and trailing bytes.
    dec_err(&[0x0D, 2, 0, 0, 0, 0], MALFORMED);
    dec_err(&efb(NO, NO, NO, NO, &[0, 0]), MALFORMED);
}

/// A flag byte of `02` is `Malformed` (A-7), and the frame must reach that
/// verdict through the flag, not through running out of bytes. Each frame
/// below is a full valid one (the control, decoded `Ok`) with one flag byte
/// replaced by `02`; a reader taking `u8 != 0` would still decode it.
#[test]
fn b1_encounter_flag_02_with_a_full_remainder() {
    // tag, mode_disclosed, mode, pending_disclosed, pending_some, pending,
    // clock_disclosed, clock_some, round_ticks, elapsed.
    let full = [0x0B, 1, 0, 1, 1, 0, 1, 1, 1, 0];
    check_op(encounter(Some(TB), Some(Some(TB)), clock(1, 0)), &full);
    // mode_disclosed, pending_disclosed, pending-some, clock_disclosed, clock_some.
    for at in [1, 3, 4, 6, 7] {
        assert_eq!(full[at], 1);
        let mut bad = full;
        bad[at] = 2;
        dec_err(&bad, MALFORMED);
    }
}

#[test]
fn b2_timeline_flag_02_with_a_full_remainder() {
    check_op(timeline(&[(0, 1)]), &[0x0C, 1, 1, 0, 1]); // control
    dec_err(&[0x0C, 2, 1, 0, 1], MALFORMED);
}

#[test]
fn b3_entity_facts_flag_02_with_a_full_remainder() {
    let pools = [1, 0];
    let effects = [1, 0];
    let order = [&[1, 1][..], &utext(3), &[1, 1]].concat(); // some, ability, target 1, repeat
    let stats = [1, 1, 1, 0x61, 0, 0];
    let tags = [1, 1, 1, 0x61];
    let full = efb(&pools, &effects, &order, &stats, &tags);
    let set = FactSet {
        pools: Some(vec![]),
        effects: Some(vec![]),
        order: Some(Some(OrderFact {
            ability: ulid(3),
            target: net(1),
            repeat: true,
        })),
        stats: Some(vec![int_stat("a", 0)]),
        tags: Some(vec!["a".to_string()]),
    };
    check_op(facts_op(set), &full); // control: every flag is 01
    let pools_at = 2;
    let effects_at = pools_at + pools.len();
    let order_at = effects_at + effects.len();
    let stats_at = order_at + order.len();
    let tags_at = stats_at + stats.len();
    let at_list = [
        pools_at,     // pools disclosed
        effects_at,   // effects disclosed
        order_at,     // order disclosed
        order_at + 1, // order some
        stats_at - 1, // order repeat (the last order byte)
        stats_at,     // stats disclosed
        tags_at,      // tags disclosed
    ];
    for at in at_list {
        assert_eq!(full[at], 1, "flag at {at}");
        let mut bad = full.clone();
        bad[at] = 2;
        dec_err(&bad, MALFORMED);
    }
}

#[test]
fn b2_timeline_refuses_a_duplicate_entity_that_is_not_adjacent() {
    // Equal keys (zigzag 1 is `02`), so only the entity repeat is wrong.
    check_op(
        timeline(&[(1, 1), (1, 2), (1, 3)]),
        &[0x0C, 1, 3, 2, 1, 2, 2, 2, 3],
    ); // control
    dec_err(&[0x0C, 1, 3, 2, 1, 2, 2, 2, 1], MALFORMED);
    enc_err(timeline(&[(1, 1), (1, 2), (1, 1)]), MALFORMED);
    // The repeat is of the middle entry, three entries after the start.
    dec_err(&[0x0C, 1, 4, 2, 1, 2, 2, 2, 3, 2, 2], MALFORMED);
    enc_err(timeline(&[(1, 1), (1, 2), (1, 3), (1, 2)]), MALFORMED);
    // Different keys do not excuse the repeat.
    dec_err(&[0x0C, 1, 3, 2, 1, 4, 2, 6, 1], MALFORMED);
    enc_err(timeline(&[(1, 1), (2, 2), (3, 1)]), MALFORMED);
    assert_eq!(
        p3::check_timeline(&[
            TimelineEntry {
                key: 1,
                entity: net(1)
            },
            TimelineEntry {
                key: 1,
                entity: net(2)
            },
            TimelineEntry {
                key: 1,
                entity: net(1)
            },
        ]),
        Err(MALFORMED)
    );
}

#[test]
fn b3_stat_kind_is_refused_before_its_value_is_read() {
    // The frame ends right after the kind byte: no value byte at all.
    dec_err(&[0x0D, 2, 0, 0, 0, 1, 1, 1, 0x61, 2], UNKNOWN);
    dec_err(&[0x0D, 2, 0, 0, 0, 1, 1, 1, 0x61, 0xFF], UNKNOWN);
    // A kind of 02 with an out-of-range value: the kind still comes first.
    dec_err(
        &[
            0x0D, 2, 0, 0, 0, 1, 1, 1, 0x61, 2, 0x80, 0x80, 0x80, 0x80, 0x10, 0,
        ],
        UNKNOWN,
    );
    // Controls: kinds 00 and 01 with a value decode; with no value, Malformed.
    assert!(c3::decode_delta(&d1(&[0x0D, 2, 0, 0, 0, 1, 1, 1, 0x61, 0, 0, 0])).is_ok());
    assert!(c3::decode_delta(&d1(&[0x0D, 2, 0, 0, 0, 1, 1, 1, 0x61, 1, 0, 0])).is_ok());
    dec_err(&[0x0D, 2, 0, 0, 0, 1, 1, 1, 0x61, 1], MALFORMED);
}

#[test]
fn b3_stat_names_that_differ_only_in_case_are_distinct() {
    for (first, second) in [("might", "MIGHT"), ("MIGHT", "might")] {
        check_op(
            stats_only(vec![int_stat(first, 1), int_stat(second, 2)]),
            &efb(
                NO,
                NO,
                NO,
                &list(&[stat_b(first, 0, 1), stat_b(second, 0, 2)]),
                NO,
            ),
        );
    }
    // Control: the same name twice is still refused.
    dec_err(
        &efb(
            NO,
            NO,
            NO,
            &list(&[stat_b("might", 0, 1), stat_b("might", 0, 2)]),
            NO,
        ),
        MALFORMED,
    );
}

#[test]
fn b6_fact_precedence() {
    // current > max, then a stats count of 33: the pool fails first.
    let stats33 = [1, 0x21];
    dec_err(
        &efb(&list(&[pool_b(1, 4, 3)]), NO, NO, &stats33, NO),
        MALFORMED,
    );
    // Control: a valid pool lets the stats count fail.
    dec_err(&efb(&list(&[pool_b(1, 3, 3)]), NO, NO, &stats33, NO), LIMIT);
    // A name length of 33 followed by invalid UTF-8: length first.
    let long_bad = [&[1, 1, 0x21][..], &[0xFF; 33], &[0, 0]].concat();
    dec_err(&efb(NO, NO, NO, &long_bad, NO), LIMIT);
    let tag_bad = [&[1, 1, 0x21][..], &[0xFF; 33]].concat();
    dec_err(&efb(NO, NO, NO, NO, &tag_bad), LIMIT);
    // Control: length 32 followed by invalid UTF-8 is Malformed.
    let ok_len_bad = [&[1, 1, 0x20][..], &[0xFF; 32], &[0, 0]].concat();
    dec_err(&efb(NO, NO, NO, &ok_len_bad, NO), MALFORMED);
    // Left to right: effects count 17 before a bad stat kind.
    dec_err(&efb(NO, &[1, 0x11], NO, &[1, 1, 1, 0x61, 2, 0], NO), LIMIT);
    // On encode the first failure in wire order wins: pools before stats.
    let both = FactSet {
        pools: Some(vec![pool(1, 4, 3)]),
        stats: Some(stats_n(33)),
        ..FactSet::default()
    };
    enc_err(facts_op(both), MALFORMED);
    let both = FactSet {
        pools: Some(pools_n(16)),
        stats: Some(stats_n(33)),
        ..FactSet::default()
    };
    enc_err(facts_op(both), LIMIT);
    // Within a list the count is checked before any element, then elements
    // in list order.
    let mut stats = stats_n(33);
    stats[0].name = String::new();
    assert_eq!(p3::check_stats(&stats), Err(LIMIT));
    let stats = vec![int_stat("", 0), int_stat(&"x".repeat(33), 0)];
    assert_eq!(p3::check_stats(&stats), Err(MALFORMED));
    let stats = vec![int_stat(&"x".repeat(33), 0), int_stat("", 0)];
    assert_eq!(p3::check_stats(&stats), Err(LIMIT));
}

/// `check` must give exactly what `encode_delta` gives for an op with the value.
fn same(op: DeltaOp, got: Result<(), CodecError>, want: Result<(), CodecError>) {
    assert_eq!(got, want);
    assert_eq!(c3::encode_delta(&dfr(vec![op])).map(|_| ()), want);
}

#[test]
fn b7_check_functions_match_codec() {
    let ok = Ok(());
    // Clock.
    for (ticks, elapsed, want) in [
        (1, 0, ok),
        (u32::MAX, 2_147_483_647, ok),
        (0, 0, Err(MALFORMED)),
        (1, 2_147_483_648, Err(MALFORMED)),
    ] {
        let fact = ClockFact {
            round_ticks: ticks,
            elapsed,
        };
        same(
            encounter(None, None, clock(ticks, elapsed)),
            fact.check(),
            want,
        );
    }
    // Timeline.
    let entries = |v: &[(i32, u64)]| -> Vec<TimelineEntry> {
        v.iter()
            .map(|&(key, e)| TimelineEntry {
                key,
                entity: net(e),
            })
            .collect()
    };
    let big: Vec<(i32, u64)> = (0..1025).map(|i| (0, i + 1)).collect();
    let cases: Vec<(Vec<(i32, u64)>, Verdict)> = vec![
        (vec![], ok),
        (vec![(-1, 2), (-1, 1), (5, 3)], ok),
        (vec![(i32::MIN, 1), (i32::MAX, 2)], ok),
        (big[..1024].to_vec(), ok),
        (big, Err(LIMIT)),
        (vec![(5, 1), (4, 2)], Err(MALFORMED)),
        (vec![(0, 1), (-1, 2)], Err(MALFORMED)),
        (vec![(1, 1), (2, 1)], Err(MALFORMED)),
    ];
    for (v, want) in cases {
        same(timeline(&v), p3::check_timeline(&entries(&v)), want);
    }
    // Pools.
    let pool_cases = vec![
        (pools_n(16), ok),
        (pools_n(17), Err(LIMIT)),
        (vec![pool(1, 3, 3)], ok),
        (vec![pool(1, 4, 3)], Err(MALFORMED)),
        (vec![pool(2, 0, 0), pool(1, 0, 0)], ok),
        (
            vec![pool(1, 0, 0), pool(2, 0, 0), pool(1, 0, 0)],
            Err(MALFORMED),
        ),
    ];
    for (v, want) in pool_cases {
        same(pools_only(v.clone()), p3::check_pools(&v), want);
    }
    // Effects.
    let effect_cases = vec![
        (effects_n(16), ok),
        (effects_n(17), Err(LIMIT)),
        (vec![effect(2, 0), effect(1, 0)], Err(MALFORMED)),
        (vec![effect(1, 0), effect(1, 0)], Err(MALFORMED)),
    ];
    for (v, want) in effect_cases {
        same(effects_only(v.clone()), p3::check_effects(&v), want);
    }
    // Stats.
    let stat_cases = vec![
        (stats_n(32), ok),
        (stats_n(33), Err(LIMIT)),
        (vec![int_stat(&"n".repeat(32), 0)], ok),
        (vec![int_stat(&"n".repeat(33), 0)], Err(LIMIT)),
        (vec![int_stat("", 0)], Err(MALFORMED)),
        (vec![int_stat("a", 0), int_stat("a", 1)], Err(MALFORMED)),
        (vec![int_stat("b", 0), int_stat("a", 0)], ok),
    ];
    for (v, want) in stat_cases {
        same(stats_only(v.clone()), p3::check_stats(&v), want);
    }
    // Tags.
    let owned = |v: &[&str]| -> Vec<String> { v.iter().map(|s| (*s).to_string()).collect() };
    let tag_cases: Vec<(Vec<String>, Verdict)> = vec![
        (tag_names(8), ok),
        (tag_names(9), Err(LIMIT)),
        (owned(&[&"n".repeat(32)]), ok),
        (owned(&[&"n".repeat(33)]), Err(LIMIT)),
        (owned(&[""]), Err(MALFORMED)),
        (owned(&["b", "a"]), Err(MALFORMED)),
        (owned(&["a", "a"]), Err(MALFORMED)),
        (owned(&["B", "a"]), ok),
    ];
    for (v, want) in tag_cases {
        let op = facts_op(FactSet {
            tags: Some(v.clone()),
            ..FactSet::default()
        });
        same(op, p3::check_tags(&v), want);
    }
}

fn pe(candidates: &[EventCandidate]) -> Result<Vec<DeltaOp>, ProjectionError> {
    pr3::project_events(candidates)
}

fn turn_started() -> pv2::EventCandidate {
    pv2::EventCandidate::TurnStarted {
        actor: Some(net(1)),
        round: Some(1),
    }
}

fn action(outcome: &str, actor: Option<NetId>) -> pv2::EventCandidate {
    pv2::EventCandidate::ActionResolved {
        actor,
        target: Some(net(2)),
        ability: Some(ulid(1)),
        outcome: Some(outcome.to_string()),
        damage: Some(7),
    }
}

fn joined(entity: Option<u64>, disclose: bool) -> EventCandidate {
    EventCandidate::Joined {
        entity: entity.map(net),
        disclose,
    }
}

fn left(entity: Option<u64>, disclose: bool) -> EventCandidate {
    EventCandidate::Left {
        entity: entity.map(net),
        disclose,
    }
}

#[test]
fn b8_project_events_order_and_suppression() {
    let input = vec![
        EventCandidate::V2(turn_started()),
        EventCandidate::RoundStarted { round: Some(2) },
        EventCandidate::ModeChanged {
            mode: Some(RT),
            round: None,
        },
        EventCandidate::ModeChanged {
            mode: Some(RT),
            round: Some(2),
        },
        EventCandidate::OrderEnded { actor: None },
        EventCandidate::OrderEnded {
            actor: Some(net(3)),
        },
        joined(Some(4), false),
        joined(Some(4), true),
        left(Some(4), true),
    ];
    assert_eq!(
        pe(&input),
        Ok(vec![
            DeltaOp::TurnStarted {
                actor: net(1),
                round: 1
            },
            DeltaOp::RoundStarted { round: 2 },
            DeltaOp::ModeChanged { mode: RT, round: 2 },
            DeltaOp::OrderEnded { actor: net(3) },
            DeltaOp::Joined { entity: net(4) },
            DeltaOp::Left { entity: net(4) },
        ])
    );
    // Each missing field omits the candidate, whatever else it carries.
    let omitted = [
        EventCandidate::RoundStarted { round: None },
        EventCandidate::ModeChanged {
            mode: None,
            round: Some(1),
        },
        EventCandidate::ModeChanged {
            mode: Some(TB),
            round: None,
        },
        EventCandidate::OrderEnded { actor: None },
        joined(None, true),
        joined(Some(1), false),
        left(None, true),
        left(Some(1), false),
    ];
    for candidate in omitted {
        assert_eq!(pe(&[candidate]), Ok(vec![]));
    }
    // No sorting and no deduplication.
    let twice = [
        left(Some(9), true),
        EventCandidate::RoundStarted { round: Some(5) },
        EventCandidate::RoundStarted { round: Some(5) },
        joined(Some(1), true),
    ];
    assert_eq!(
        pe(&twice),
        Ok(vec![
            DeltaOp::Left { entity: net(9) },
            DeltaOp::RoundStarted { round: 5 },
            DeltaOp::RoundStarted { round: 5 },
            DeltaOp::Joined { entity: net(1) },
        ])
    );
    // V2 candidates give exactly what projection_v2 gives, converted.
    let v2s = [
        pv2::EventCandidate::Spawned {
            entity: Some(net(1)),
            disclose: true,
        },
        pv2::EventCandidate::Despawned {
            entity: Some(net(2)),
            disclose: true,
        },
        pv2::EventCandidate::Died {
            entity: Some(net(3)),
            disclose: true,
        },
        pv2::EventCandidate::Died {
            entity: Some(net(3)),
            disclose: false,
        },
        action("success", Some(net(1))),
        turn_started(),
        pv2::EventCandidate::EncounterEnded {
            encounter: Some(ulid(1)),
            round: Some(3),
        },
    ];
    for c in v2s {
        let want: Vec<DeltaOp> = pv2::project_events(std::slice::from_ref(&c))
            .unwrap()
            .into_iter()
            .map(DeltaOp::from)
            .collect();
        assert_eq!(pe(&[EventCandidate::V2(c)]), Ok(want));
    }
    assert_eq!(
        pe(&[EventCandidate::V2(pv2::EventCandidate::Despawned {
            entity: Some(net(2)),
            disclose: true
        })]),
        Ok(vec![DeltaOp::Despawned { entity: net(2) }])
    );
    assert_eq!(
        pe(&[EventCandidate::V2(action("success", Some(net(1))))]),
        Ok(vec![DeltaOp::ActionResolved {
            actor: net(1),
            target: net(2),
            ability: ulid(1),
            outcome: "success".to_string(),
            damage: 7,
        }])
    );
    // Bounds: 256 accepted, 257 refused even if every candidate is omitted.
    let hidden = EventCandidate::RoundStarted { round: None };
    assert_eq!(pe(&vec![hidden.clone(); 256]), Ok(vec![]));
    assert_eq!(pe(&vec![hidden; 257]), Err(ProjectionError::TooManyEvents));
    let shown = EventCandidate::RoundStarted { round: Some(1) };
    assert_eq!(pe(&vec![shown.clone(); 256]).unwrap().len(), 256);
    assert_eq!(pe(&vec![shown; 257]), Err(ProjectionError::TooManyEvents));
    // V2 errors fail the whole call; an omitted candidate is not inspected.
    let bogus = EventCandidate::V2(action("bogus", Some(net(1))));
    assert_eq!(
        pe(&[EventCandidate::RoundStarted { round: Some(1) }, bogus]),
        Err(ProjectionError::InvalidOutcome)
    );
    assert_eq!(pe(&[EventCandidate::V2(action("bogus", None))]), Ok(vec![]));
    let long = EventCandidate::V2(action(&"a".repeat(257), Some(net(1))));
    assert_eq!(pe(&[long]), Err(ProjectionError::StringLimit));
    let edge = EventCandidate::V2(action(&"a".repeat(256), Some(net(1))));
    assert_eq!(pe(&[edge]), Err(ProjectionError::InvalidOutcome));
}

fn t(k: u64) -> DeltaOp {
    DeltaOp::Timeline {
        entries: Some(
            (0..k)
                .map(|i| TimelineEntry {
                    key: 0,
                    entity: net((1 << 63) + i),
                })
                .collect(),
        ),
    }
}

fn r(round: u64) -> DeltaOp {
    DeltaOp::RoundStarted { round }
}

fn at_cap_ops() -> Vec<DeltaOp> {
    let mut ops = vec![t(1024); 5];
    ops.push(t(711));
    ops.extend(vec![r(u64::MAX); 122]);
    ops.push(r(1 << 21));
    ops
}

fn max_frame(ops: Vec<DeltaOp>) -> DeltaFrame {
    DeltaFrame {
        lane: 0,
        server_tick: u64::MAX,
        event_seq: u64::MAX,
        ops,
    }
}

/// Encoded op bytes (tag included) of a lone op: the frame minus its 5-byte header.
fn sized(op: DeltaOp) -> usize {
    c3::encode_delta(&dfr(vec![op])).unwrap().len() - 5
}

#[test]
fn b9_frame_spans_and_frame_cap() {
    assert_eq!(p3::MAX_DELTA_HEADER_BYTES_V3, 24);
    assert_eq!(p3::MAX_FRAME_OP_BYTES_V3, 65_512);
    // The op sizes the contract states.
    assert_eq!(sized(t(1024)), 11_268);
    assert_eq!(sized(t(711)), 7_825);
    assert_eq!(sized(t(128)), 4 + 11 * 128);
    assert_eq!(sized(r(u64::MAX)), 11);
    assert_eq!(sized(r(1 << 21)), 5);
    assert_eq!(sized(r(1 << 28)), 6);
    // At the cap: 129 ops of exactly 65,512 bytes form one frame of 65,536.
    let a = at_cap_ops();
    assert_eq!(a.len(), 129);
    assert_eq!(c3::frame_spans(&a), Ok(vec![0..129]));
    let bytes = c3::encode_delta(&max_frame(a.clone())).unwrap();
    assert_eq!(bytes.len(), 65_536);
    let header = [&[3, 0][..], &MAX64, &MAX64, &[0x81, 0x01]].concat();
    assert_eq!(header.len(), 24);
    assert_eq!(&bytes[..24], &header[..]);
    assert_eq!(c3::decode_delta(&bytes), Ok(max_frame(a.clone())));
    // One byte past: the frame cap is reachable on encode, and spans split.
    let mut b = a;
    *b.last_mut().unwrap() = r(1 << 28);
    assert_eq!(
        c3::encode_delta(&max_frame(b.clone())),
        Err(CodecError::FrameTooLarge)
    );
    assert_eq!(c3::frame_spans(&b), Ok(vec![0..128, 128..129]));
    // Decode: 65,537 bytes are FrameTooLarge before anything else.
    let mut over = bytes;
    over.push(0);
    assert_eq!(c3::decode_delta(&over), Err(CodecError::FrameTooLarge));
    // Op cap: 256 ops fit, 257 split.
    assert_eq!(c3::frame_spans(&vec![r(0); 256]), Ok(vec![0..256]));
    assert_eq!(
        c3::frame_spans(&vec![r(0); 257]),
        Ok(vec![0..256, 256..257])
    );
    assert_eq!(c3::frame_spans(&[]), Ok(vec![]));
    assert_eq!(c3::frame_spans(&[r(0)]), Ok(vec![0..1]));
    // Greedy: six full timelines close after five, and each frame restarts.
    assert_eq!(c3::frame_spans(&vec![t(1024); 6]), Ok(vec![0..5, 5..6]));
    assert_eq!(
        c3::frame_spans(&vec![t(1024); 11]),
        Ok(vec![0..5, 5..10, 10..11])
    );
    // Errors: the first failing op wins, with no spans.
    let bogus = DeltaOp::ActionResolved {
        actor: net(1),
        target: net(2),
        ability: ulid(1),
        outcome: "bogus".to_string(),
        damage: 7,
    };
    assert_eq!(c3::frame_spans(&[r(0), bogus.clone()]), Err(MALFORMED));
    assert_eq!(
        c3::frame_spans(&[bogus, pools_only(pools_n(17))]),
        Err(MALFORMED)
    );
    assert_eq!(
        c3::frame_spans(&[r(0), pools_only(pools_n(17))]),
        Err(LIMIT)
    );
    assert_eq!(
        c3::frame_spans(&[timeline(&[(5, 1), (4, 2)])]),
        Err(MALFORMED)
    );
    assert_eq!(
        c3::frame_spans(&[encounter(None, None, clock(0, 0))]),
        Err(MALFORMED)
    );
}

#[test]
fn b9_page_of_maximal_entity_facts() {
    let facts = FactSet {
        pools: Some((1..=16).map(|i| pool(i, u32::MAX, u32::MAX)).collect()),
        effects: Some((1..=16).map(|i| effect(i, u64::MAX)).collect()),
        order: Some(Some(OrderFact {
            ability: ulid(1),
            target: net(1 << 63),
            repeat: true,
        })),
        stats: Some(
            (0..32)
                .map(|i| {
                    let name = format!("{i:02}{}", "n".repeat(30));
                    if i % 2 == 0 {
                        fixed_stat(&name, i32::MAX)
                    } else {
                        int_stat(&name, i32::MIN)
                    }
                })
                .collect(),
        ),
        tags: Some((0..8).map(|i| format!("{i}{}", "z".repeat(31))).collect()),
    };
    let page: Vec<DeltaOp> = (0..1024u64)
        .map(|i| DeltaOp::EntityFacts {
            entity: net((1 << 63) + i),
            facts: facts.clone(),
        })
        .collect();
    // Tag 1 + entity 10 + a 2,744-byte fact set.
    assert_eq!(sized(page[0].clone()), 2_755);
    let spans = c3::frame_spans(&page).unwrap();
    // 23 ops are 63,365 bytes; a 24th would make 66,120 > 65,512.
    assert_eq!(spans.len(), 45);
    let mut next = 0;
    for span in &spans {
        assert_eq!(span.start, next);
        assert!(span.len() == 23 || span.end == 1024);
        next = span.end;
        let frame = max_frame(page[span.clone()].to_vec());
        let bytes = c3::encode_delta(&frame).unwrap();
        assert!(bytes.len() <= 65_536);
        assert_eq!(c3::decode_delta(&bytes), Ok(frame));
    }
    assert_eq!(next, 1024);
    assert_eq!(spans[44], 1012..1024);
}
