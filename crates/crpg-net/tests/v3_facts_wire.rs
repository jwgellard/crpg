//! T140b protocol v3 fact ops: `Encounter` (11), `Timeline` (12) and
//! `EntityFacts` (13), the frame splitter `frame_spans`, and `projection_v3`
//! (ADR-0029 N1, `tasks/T140.md` B-1..B-6).
//!
//! Public API only. Every v3 expectation is a hand-written literal byte
//! oracle built from the contract's layout (postcard primitives: raw `u8` and
//! `bool`, LEB128 varints, zigzag `i32`, length-prefixed UTF-8, ULID text),
//! never from the codec under test. Every rejection has a passing control
//! beside it.

use crpg_core::Ulid;
use crpg_net::codec::CodecError;
use crpg_net::codec_v2;
use crpg_net::codec_v3::{decode_delta, encode_delta, frame_spans};
use crpg_net::projection_v2::EventCandidate as V2Candidate;
use crpg_net::projection_v3::{project_events, EventCandidate, ProjectionError};
use crpg_net::protocol::{LANE_COMBAT, MAX_DELTA_FRAME_BYTES, MAX_DELTA_OPS};
use crpg_net::protocol_v3::{
    check_effects, check_pools, check_stats, check_tags, check_timeline, ClockFact, DeltaFrame,
    DeltaOp, EffectFact, EncounterFacts, EncounterMode, FactSet, NetId, OrderFact, PoolFact,
    ReceiptStatus, StatFact, StatValue, TimelineEntry, MAX_CLOCK_ELAPSED,
    MAX_DELTA_HEADER_BYTES_V3, MAX_FACT_EFFECTS, MAX_FACT_NAME_BYTES, MAX_FACT_POOLS,
    MAX_FACT_STATS, MAX_FACT_TAGS, MAX_FRAME_OP_BYTES_V3, MAX_TIMELINE_ENTRIES, STAT_KIND_FIXED,
    STAT_KIND_INT,
};

// ---------------------------------------------------------------------------
// Fixtures and byte helpers.
// ---------------------------------------------------------------------------

fn net(raw: u64) -> NetId {
    NetId::new(raw).expect("fixture id is nonzero")
}

/// `U(n)` of the contract.
fn u(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

/// Wire text of `U(n)` for `n` in 1..=9: `1A`, then 25 x `30`, then the digit.
fn ulid_wire(digit: u8) -> Vec<u8> {
    let mut out = vec![0x1A];
    out.extend(std::iter::repeat_n(0x30, 25));
    out.push(0x30 + digit);
    out
}

/// Wire text of `U(n)` written independently: Crockford base32, 26 digits.
fn ulid_text(n: u128) -> Vec<u8> {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut digits = [b'0'; 26];
    let mut rest = n;
    for slot in digits.iter_mut().rev() {
        *slot = ALPHABET[(rest & 31) as usize];
        rest >>= 5;
    }
    let mut out = vec![26];
    out.extend_from_slice(&digits);
    out
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

/// LEB128 varint, written independently of postcard.
fn varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

/// Zigzag then LEB128, written independently of postcard.
fn zigzag(value: i32) -> Vec<u8> {
    varint(((i64::from(value) << 1) ^ (i64::from(value) >> 31)) as u32 as u64)
}

/// Length-prefixed text.
fn text(s: &str) -> Vec<u8> {
    cat(&[&varint(s.len() as u64), s.as_bytes()])
}

fn delta(ops: Vec<DeltaOp>) -> DeltaFrame {
    DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 100,
        event_seq: 1,
        ops,
    }
}

/// `03 00 64 01 <count>` followed by `ops`.
fn delta_bytes(count: u8, ops: &[u8]) -> Vec<u8> {
    cat(&[&[3, 0, 0x64, 1, count], ops])
}

/// The op bytes (tag first) of a one-op frame, header stripped and checked.
fn op_bytes(op: DeltaOp) -> Vec<u8> {
    let bytes = encode_delta(&delta(vec![op])).expect("fixture op encodes");
    assert_eq!(&bytes[..5], &[3, 0, 0x64, 1, 1], "one-op frame header");
    bytes[5..].to_vec()
}

/// Encodes one op to its op bytes or the encode error.
fn try_op_bytes(op: DeltaOp) -> Result<Vec<u8>, CodecError> {
    encode_delta(&delta(vec![op])).map(|bytes| bytes[5..].to_vec())
}

/// Decodes a one-op frame holding exactly `bytes`.
fn dec(bytes: &[u8]) -> Result<DeltaOp, CodecError> {
    decode_delta(&delta_bytes(1, bytes)).map(|frame| {
        assert_eq!(frame.ops.len(), 1);
        frame.ops.into_iter().next().unwrap()
    })
}

fn dec_err(bytes: &[u8]) -> CodecError {
    dec(bytes).expect_err("these bytes must be refused")
}

/// Asserts `op` encodes to exactly `bytes` and decodes back to itself.
fn assert_op(op: DeltaOp, bytes: &[u8]) {
    assert_eq!(op_bytes(op.clone()), bytes);
    assert_eq!(dec(bytes), Ok(op));
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

fn facts(entity: u64, set: FactSet) -> DeltaOp {
    DeltaOp::EntityFacts {
        entity: net(entity),
        facts: set,
    }
}

fn pools_only(pools: Vec<PoolFact>) -> DeltaOp {
    facts(
        2,
        FactSet {
            pools: Some(pools),
            ..FactSet::default()
        },
    )
}

fn effects_only(effects: Vec<EffectFact>) -> DeltaOp {
    facts(
        2,
        FactSet {
            effects: Some(effects),
            ..FactSet::default()
        },
    )
}

fn stats_only(stats: Vec<StatFact>) -> DeltaOp {
    facts(
        2,
        FactSet {
            stats: Some(stats),
            ..FactSet::default()
        },
    )
}

fn tags_only(tags: Vec<String>) -> DeltaOp {
    facts(
        2,
        FactSet {
            tags: Some(tags),
            ..FactSet::default()
        },
    )
}

fn pool(n: u128, current: u32, max: u32) -> PoolFact {
    PoolFact {
        pool: u(n),
        current,
        max,
    }
}

fn effect(n: u128, expires_round: u64) -> EffectFact {
    EffectFact {
        effect: u(n),
        expires_round,
    }
}

fn stat(name: &str, value: StatValue) -> StatFact {
    StatFact {
        name: name.to_owned(),
        value,
    }
}

fn tags(list: &[&str]) -> Vec<String> {
    list.iter().map(|t| (*t).to_owned()).collect()
}

/// A withheld family: just its `00` flag.
const W: &[u8] = &[0];

/// `0D 02` and the five family encodings (each includes its own flag).
fn entity_facts_bytes(parts: [&[u8]; 5]) -> Vec<u8> {
    cat(&[
        &[0x0D, 0x02],
        parts[0],
        parts[1],
        parts[2],
        parts[3],
        parts[4],
    ])
}

/// A disclosed list family: flag `01`, the count varint, then `elements`.
fn family(count: usize, elements: &[u8]) -> Vec<u8> {
    cat(&[&[1], &varint(count as u64), elements])
}

fn n_pools(count: usize) -> Vec<PoolFact> {
    (0..count).map(|i| pool(i as u128 + 1, 1, 2)).collect()
}

fn n_effects(count: usize) -> Vec<EffectFact> {
    (0..count)
        .map(|i| effect(i as u128 + 1, i as u64))
        .collect()
}

fn n_stats(count: usize) -> Vec<StatFact> {
    (0..count)
        .map(|i| stat(&format!("s{i:02}"), StatValue::Int(i as i32)))
        .collect()
}

fn n_tags(count: usize) -> Vec<String> {
    (0..count).map(|i| format!("t{i}")).collect()
}

const FLAG_BAD: u8 = 2;

// ---------------------------------------------------------------------------
// Constants and small types.
// ---------------------------------------------------------------------------

#[test]
fn constants_and_withheld_predicates_are_pinned() {
    assert_eq!(MAX_FACT_POOLS, 16);
    assert_eq!(MAX_FACT_EFFECTS, 16);
    assert_eq!(MAX_FACT_STATS, 32);
    assert_eq!(MAX_FACT_TAGS, 8);
    assert_eq!(MAX_FACT_NAME_BYTES, 32);
    assert_eq!(MAX_TIMELINE_ENTRIES, 1024);
    assert_eq!(MAX_CLOCK_ELAPSED, 2_147_483_647);
    assert_eq!(MAX_CLOCK_ELAPSED, i32::MAX as u64);
    assert_eq!(STAT_KIND_INT, 0);
    assert_eq!(STAT_KIND_FIXED, 1);
    assert_eq!(MAX_DELTA_HEADER_BYTES_V3, 24);
    assert_eq!(MAX_FRAME_OP_BYTES_V3, 65_512);
    assert_eq!(
        MAX_FRAME_OP_BYTES_V3 + MAX_DELTA_HEADER_BYTES_V3,
        MAX_DELTA_FRAME_BYTES
    );

    // is_withheld: true only when every field is None; each field alone flips it.
    assert!(EncounterFacts::default().is_withheld());
    assert!(!EncounterFacts {
        mode: Some(EncounterMode::TurnBased),
        ..EncounterFacts::default()
    }
    .is_withheld());
    assert!(!EncounterFacts {
        pending_mode: Some(None),
        ..EncounterFacts::default()
    }
    .is_withheld());
    assert!(!EncounterFacts {
        clock: Some(None),
        ..EncounterFacts::default()
    }
    .is_withheld());

    assert!(FactSet::default().is_withheld());
    let one_each = [
        FactSet {
            pools: Some(vec![]),
            ..FactSet::default()
        },
        FactSet {
            effects: Some(vec![]),
            ..FactSet::default()
        },
        FactSet {
            order: Some(None),
            ..FactSet::default()
        },
        FactSet {
            stats: Some(vec![]),
            ..FactSet::default()
        },
        FactSet {
            tags: Some(vec![]),
            ..FactSet::default()
        },
    ];
    for set in one_each {
        assert!(!set.is_withheld(), "{set:?}");
    }
}

// ---------------------------------------------------------------------------
// B1 encounter_literals
// ---------------------------------------------------------------------------

/// Independent oracle for the `Encounter` payload.
fn encounter_oracle(facts: &EncounterFacts) -> Vec<u8> {
    let mut out = vec![0x0B];
    match facts.mode {
        None => out.push(0),
        Some(m) => out.extend([1, m.as_u8()]),
    }
    match facts.pending_mode {
        None => out.push(0),
        Some(None) => out.extend([1, 0]),
        Some(Some(m)) => out.extend([1, 1, m.as_u8()]),
    }
    match facts.clock {
        None => out.push(0),
        Some(None) => out.extend([1, 0]),
        Some(Some(c)) => {
            out.extend([1, 1]);
            out.extend(varint(u64::from(c.round_ticks)));
            out.extend(varint(c.elapsed));
        }
    }
    out
}

#[test]
fn b1_encounter_literals() {
    assert_op(
        DeltaOp::Encounter(EncounterFacts::default()),
        &[0x0B, 0, 0, 0],
    );
    assert_op(
        DeltaOp::Encounter(EncounterFacts {
            mode: Some(EncounterMode::TurnBased),
            ..EncounterFacts::default()
        }),
        &[0x0B, 1, 0, 0, 0],
    );
    assert_op(
        DeltaOp::Encounter(EncounterFacts {
            pending_mode: Some(None),
            ..EncounterFacts::default()
        }),
        &[0x0B, 0, 1, 0, 0],
    );
    assert_op(
        DeltaOp::Encounter(EncounterFacts {
            clock: Some(None),
            ..EncounterFacts::default()
        }),
        &[0x0B, 0, 0, 1, 0],
    );
    assert_op(
        DeltaOp::Encounter(EncounterFacts {
            mode: Some(EncounterMode::TurnBased),
            pending_mode: Some(Some(EncounterMode::RealTime)),
            clock: Some(Some(ClockFact {
                round_ticks: 120,
                elapsed: 240,
            })),
        }),
        &[0x0B, 1, 0, 1, 1, 1, 1, 1, 0x78, 0xF0, 0x01],
    );

    // All 36 combinations match the independent oracle and round-trip.
    let modes = [
        None,
        Some(EncounterMode::TurnBased),
        Some(EncounterMode::RealTime),
    ];
    let pendings = [
        None,
        Some(None),
        Some(Some(EncounterMode::TurnBased)),
        Some(Some(EncounterMode::RealTime)),
    ];
    let clocks = [
        None,
        Some(None),
        Some(Some(ClockFact {
            round_ticks: 120,
            elapsed: 240,
        })),
    ];
    let mut count = 0;
    for mode in modes {
        for pending_mode in pendings {
            for clock in clocks {
                let facts = EncounterFacts {
                    mode,
                    pending_mode,
                    clock,
                };
                assert_op(DeltaOp::Encounter(facts), &encounter_oracle(&facts));
                count += 1;
            }
        }
    }
    assert_eq!(count, 36);
}

#[test]
fn b1_encounter_refusals() {
    // Mode byte 02 and FF are unknown, with a passing control (mode 01).
    assert!(dec(&[0x0B, 1, 1, 0, 0]).is_ok());
    assert_eq!(dec_err(&[0x0B, 1, 2, 0, 0]), CodecError::UnknownMessage);
    assert_eq!(dec_err(&[0x0B, 1, 0xFF, 0, 0]), CodecError::UnknownMessage);
    // Same for the pending byte.
    assert!(dec(&[0x0B, 0, 1, 1, 1, 0]).is_ok());
    assert_eq!(dec_err(&[0x0B, 0, 1, 1, 2, 0]), CodecError::UnknownMessage);
    assert_eq!(
        dec_err(&[0x0B, 0, 1, 1, 0xFF, 0]),
        CodecError::UnknownMessage
    );
    // round_ticks 0 is Malformed; control 1.
    assert!(dec(&[0x0B, 0, 0, 1, 1, 1, 0]).is_ok());
    assert_eq!(dec_err(&[0x0B, 0, 0, 1, 1, 0, 5]), CodecError::Malformed);

    // elapsed: i32::MAX accepted, i32::MAX + 1 Malformed, on decode ...
    let at = ClockFact {
        round_ticks: 1,
        elapsed: MAX_CLOCK_ELAPSED,
    };
    let over = ClockFact {
        round_ticks: 1,
        elapsed: MAX_CLOCK_ELAPSED + 1,
    };
    let clock_op = |c: ClockFact| {
        DeltaOp::Encounter(EncounterFacts {
            clock: Some(Some(c)),
            ..EncounterFacts::default()
        })
    };
    assert_eq!(
        dec(&[0x0B, 0, 0, 1, 1, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0x07]),
        Ok(clock_op(at))
    );
    assert_eq!(
        dec_err(&[0x0B, 0, 0, 1, 1, 1, 0x80, 0x80, 0x80, 0x80, 0x08]),
        CodecError::Malformed
    );
    // ... and on encode.
    assert_eq!(
        op_bytes(clock_op(at)),
        [0x0B, 0, 0, 1, 1, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0x07]
    );
    assert_eq!(try_op_bytes(clock_op(over)), Err(CodecError::Malformed));
    assert_eq!(
        try_op_bytes(clock_op(ClockFact {
            round_ticks: 0,
            elapsed: 0
        })),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        try_op_bytes(clock_op(ClockFact {
            round_ticks: u32::MAX,
            elapsed: 0
        }))
        .map(|b| b.len()),
        Ok(1 + 2 + 2 + 5 + 1)
    );

    // Flag bytes other than 0/1 are Malformed in each position; the clock
    // `some` flag is last. Controls are the all-withheld literal above.
    assert_eq!(dec_err(&[0x0B, FLAG_BAD, 0, 0]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x0B, 0, FLAG_BAD, 0]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x0B, 0, 1, FLAG_BAD, 0]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x0B, 0, 0, FLAG_BAD]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x0B, 0, 0, 1, FLAG_BAD]), CodecError::Malformed);
    // Truncation and trailing bytes.
    assert_eq!(dec_err(&[0x0B]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x0B, 0, 0]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x0B, 0, 0, 0, 0]), CodecError::Malformed);
}

// ---------------------------------------------------------------------------
// B2 timeline_literals_and_bounds
// ---------------------------------------------------------------------------

#[test]
fn b2_timeline_literals() {
    assert_op(DeltaOp::Timeline { entries: None }, &[0x0C, 0]);
    assert_op(
        DeltaOp::Timeline {
            entries: Some(vec![]),
        },
        &[0x0C, 1, 0],
    );
    // Equal keys may be in descending NetId order.
    assert_op(
        timeline(&[(-1, 2), (-1, 1), (5, 3)]),
        &[0x0C, 1, 3, 1, 2, 1, 1, 0x0A, 3],
    );
    // Key extremes.
    assert_op(
        timeline(&[(i32::MIN, 1)]),
        &[0x0C, 1, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 1],
    );
    assert_op(
        timeline(&[(i32::MAX, 1)]),
        &[0x0C, 1, 1, 0xFE, 0xFF, 0xFF, 0xFF, 0x0F, 1],
    );
    assert_op(
        timeline(&[(i32::MIN, 1), (i32::MAX, 2)]),
        &[
            0x0C, 1, 2, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 1, 0xFE, 0xFF, 0xFF, 0xFF, 0x0F, 2,
        ],
    );
}

#[test]
fn b2_timeline_never_checks_net_id_order() {
    // Descending NetIds under one key, ascending under another, all legal.
    let desc: Vec<(i32, u64)> = (1..=8).rev().map(|n| (7, n)).collect();
    let op = timeline(&desc);
    let bytes = op_bytes(op.clone());
    assert_eq!(dec(&bytes), Ok(op));
    assert_eq!(check_timeline(&tl(&desc)), Ok(()));
    // Keys across a tie group stay non-decreasing; here a lower NetId after a
    // higher key is fine because the key is what orders.
    assert_eq!(
        check_timeline(&tl(&[(1, 9), (2, 1), (2, 5), (3, 2)])),
        Ok(())
    );
}

fn tl(entries: &[(i32, u64)]) -> Vec<TimelineEntry> {
    entries
        .iter()
        .map(|&(key, entity)| TimelineEntry {
            key,
            entity: net(entity),
        })
        .collect()
}

#[test]
fn b2_timeline_refusals_decode_and_encode() {
    // Descending key, repeated entity, and entity 0 are Malformed.
    // Controls: the same shapes in legal order decode.
    assert!(dec(&[0x0C, 1, 2, 8, 2, 10, 1]).is_ok()); // (4,2) then (5,1)
    assert_eq!(
        dec_err(&[0x0C, 1, 2, 10, 1, 8, 2]), // (5,1) then (4,2)
        CodecError::Malformed
    );
    assert_eq!(
        dec_err(&[0x0C, 1, 2, 2, 1, 4, 1]), // (1,1) then (2,1)
        CodecError::Malformed
    );
    assert_eq!(dec_err(&[0x0C, 1, 1, 0, 0]), CodecError::Malformed); // entity 0
    assert!(dec(&[0x0C, 1, 1, 0, 1]).is_ok());
    // Same key, same entity twice.
    assert_eq!(dec_err(&[0x0C, 1, 2, 2, 1, 2, 1]), CodecError::Malformed);

    // Encode agrees.
    assert_eq!(
        try_op_bytes(timeline(&[(5, 1), (4, 2)])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        try_op_bytes(timeline(&[(1, 1), (2, 1)])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        try_op_bytes(timeline(&[(1, 1), (1, 1)])),
        Err(CodecError::Malformed)
    );
    assert!(try_op_bytes(timeline(&[(4, 2), (5, 1)])).is_ok());

    // Flag byte 02 and truncation.
    assert_eq!(dec_err(&[0x0C, FLAG_BAD]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x0C]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x0C, 1]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x0C, 1, 1]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x0C, 0, 0]), CodecError::Malformed); // trailing byte
}

#[test]
fn b2_timeline_count_bound() {
    // 1,024 entries are accepted and match an independent oracle.
    let entries: Vec<(i32, u64)> = (1..=1024u64).map(|n| (0, n)).collect();
    let mut expected = vec![0x0C, 1];
    expected.extend(varint(1024));
    for n in 1..=1024u64 {
        expected.push(0); // key 0
        expected.extend(varint(n));
    }
    assert_op(timeline(&entries), &expected);

    // A count of 1,025 (81 08) is LimitExceeded with no entries following.
    assert_eq!(dec_err(&[0x0C, 1, 0x81, 0x08]), CodecError::LimitExceeded);
    // So is a ten-byte hostile count; the same count with the high bit set
    // beyond u64 is Malformed.
    let mut hostile = vec![0x0C, 1];
    hostile.extend([0xFF; 9]);
    hostile.push(0x01);
    assert_eq!(dec_err(&hostile), CodecError::LimitExceeded);
    let mut overflow = vec![0x0C, 1];
    overflow.extend([0xFF; 9]);
    overflow.push(0x02);
    assert_eq!(dec_err(&overflow), CodecError::Malformed);
    // Control: a count of 1,024 with no entries is a truncation, not a limit.
    assert_eq!(dec_err(&[0x0C, 1, 0x80, 0x08]), CodecError::Malformed);

    // Encode: 1,025 entries are LimitExceeded.
    let too_many: Vec<(i32, u64)> = (1..=1025u64).map(|n| (0, n)).collect();
    assert_eq!(
        try_op_bytes(timeline(&too_many)),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        check_timeline(&tl(&too_many)),
        Err(CodecError::LimitExceeded)
    );
    // The count wins over a shape error in the first entry.
    let mut bad_and_many = too_many.clone();
    bad_and_many[0] = (5, 1);
    bad_and_many[1] = (4, 1);
    assert_eq!(
        check_timeline(&tl(&bad_and_many)),
        Err(CodecError::LimitExceeded)
    );
}

// ---------------------------------------------------------------------------
// B3 entity_facts_literals
// ---------------------------------------------------------------------------

#[test]
fn b3_entity_facts_literals() {
    assert_op(facts(2, FactSet::default()), &[0x0D, 2, 0, 0, 0, 0, 0]);
    assert_op(pools_only(vec![]), &[0x0D, 2, 1, 0, 0, 0, 0, 0]);
    assert_op(
        pools_only(vec![pool(1, 2, 3)]),
        &cat(&[&[0x0D, 2, 1, 1], &ulid_wire(1), &[2, 3, 0, 0, 0, 0]]),
    );
    // The disclosed "no order" is distinct from withheld.
    assert_op(
        facts(
            2,
            FactSet {
                order: Some(None),
                ..FactSet::default()
            },
        ),
        &[0x0D, 2, 0, 0, 1, 0, 0, 0],
    );
    assert_op(
        effects_only(vec![effect(2, 4)]),
        &cat(&[&[0x0D, 2, 0, 1, 1], &ulid_wire(2), &[4, 0, 0, 0]]),
    );
    assert_op(
        tags_only(tags(&["undead"])),
        &cat(&[&[0x0D, 2, 0, 0, 0, 0, 1, 1, 6], b"undead"]),
    );

    let full = FactSet {
        pools: Some(vec![pool(1, 2, 3)]),
        effects: Some(vec![effect(2, 4)]),
        order: Some(Some(OrderFact {
            ability: u(3),
            target: net(1),
            repeat: true,
        })),
        stats: Some(vec![
            stat("might", StatValue::Int(4)),
            stat("reach", StatValue::Fixed(98304)),
        ]),
        tags: Some(tags(&["undead"])),
    };
    let expected = cat(&[
        &[0x0D, 2],
        &[1, 1],
        &ulid_wire(1),
        &[2, 3],
        &[1, 1],
        &ulid_wire(2),
        &[4],
        &[1, 1],
        &ulid_wire(3),
        &[1, 1],
        &[1, 2, 5],
        b"might",
        &[0, 8],
        &[5],
        b"reach",
        &[1, 0x80, 0x80, 0x0C],
        &[1, 1, 6],
        b"undead",
    ]);
    assert_eq!(expected.len(), 123);
    assert_op(facts(2, full), &expected);
}

#[test]
fn b3_independent_oracle_matches_ulid_text() {
    // The independent base32 oracle agrees with the contract's fixed texts.
    assert_eq!(ulid_text(1), ulid_wire(1));
    assert_eq!(ulid_text(3), ulid_wire(3));
    assert_eq!(ulid_text(0).len(), 27);
}

#[test]
fn b3_ulid_case_alias_decodes_canonical() {
    // `U(10)` is `...A`; the lowercase alias decodes and re-encodes uppercase.
    let upper = cat(&[&[0x0D, 2, 1, 1], &ulid_text(10), &[0, 0, 0, 0, 0, 0]]);
    let mut lower = upper.clone();
    let last = 4 + 26;
    assert_eq!(lower[last], b'A');
    lower[last] = b'a';
    let decoded = dec(&lower).expect("lowercase alias decodes");
    assert_eq!(decoded, pools_only(vec![pool(10, 0, 0)]));
    assert_eq!(op_bytes(decoded), upper);
}

// ---------------------------------------------------------------------------
// B4 fact_set_bounds
// ---------------------------------------------------------------------------

#[test]
fn b4_pool_bound() {
    // 16 accepted against an independent oracle, 17 refused.
    let mut elements = Vec::new();
    for i in 0..16u128 {
        elements.extend(ulid_text(i + 1));
        elements.extend([1, 2]);
    }
    let expected = entity_facts_bytes([&family(16, &elements), W, W, W, W]);
    assert_op(pools_only(n_pools(16)), &expected);
    assert_eq!(
        try_op_bytes(pools_only(n_pools(17))),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(check_pools(&n_pools(16)), Ok(()));
    assert_eq!(check_pools(&n_pools(17)), Err(CodecError::LimitExceeded));
    // Count 17 (11) is refused before any element is read.
    let bytes = entity_facts_bytes([&[1, 0x11], W, W, W, W]);
    assert_eq!(dec_err(&bytes), CodecError::LimitExceeded);
    // Control: count 16 with no elements is a truncation.
    let bytes = entity_facts_bytes([&[1, 0x10], W, W, W, W]);
    assert_eq!(dec_err(&bytes), CodecError::Malformed);
}

#[test]
fn b4_effect_bound() {
    let mut elements = Vec::new();
    for i in 0..16u128 {
        elements.extend(ulid_text(i + 1));
        elements.extend(varint(i as u64));
    }
    let expected = entity_facts_bytes([W, &family(16, &elements), W, W, W]);
    assert_op(effects_only(n_effects(16)), &expected);
    assert_eq!(
        try_op_bytes(effects_only(n_effects(17))),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(check_effects(&n_effects(16)), Ok(()));
    assert_eq!(
        check_effects(&n_effects(17)),
        Err(CodecError::LimitExceeded)
    );
    let bytes = entity_facts_bytes([W, &[1, 0x11], W, W, W]);
    assert_eq!(dec_err(&bytes), CodecError::LimitExceeded);
    let bytes = entity_facts_bytes([W, &[1, 0x10], W, W, W]);
    assert_eq!(dec_err(&bytes), CodecError::Malformed);
}

#[test]
fn b4_stat_bound() {
    let mut elements = Vec::new();
    for i in 0..32 {
        elements.extend(text(&format!("s{i:02}")));
        elements.push(0); // Int
        elements.extend(zigzag(i));
    }
    let expected = entity_facts_bytes([W, W, W, &family(32, &elements), W]);
    assert_op(stats_only(n_stats(32)), &expected);
    assert_eq!(
        try_op_bytes(stats_only(n_stats(33))),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(check_stats(&n_stats(32)), Ok(()));
    assert_eq!(check_stats(&n_stats(33)), Err(CodecError::LimitExceeded));
    // Count 33 is 21.
    let bytes = entity_facts_bytes([W, W, W, &[1, 0x21], W]);
    assert_eq!(dec_err(&bytes), CodecError::LimitExceeded);
    let bytes = entity_facts_bytes([W, W, W, &[1, 0x20], W]);
    assert_eq!(dec_err(&bytes), CodecError::Malformed);
}

#[test]
fn b4_tag_bound() {
    let mut elements = Vec::new();
    for i in 0..8 {
        elements.extend(text(&format!("t{i}")));
    }
    let expected = entity_facts_bytes([W, W, W, W, &family(8, &elements)]);
    assert_op(tags_only(n_tags(8)), &expected);
    assert_eq!(
        try_op_bytes(tags_only(n_tags(9))),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(check_tags(&n_tags(8)), Ok(()));
    assert_eq!(check_tags(&n_tags(9)), Err(CodecError::LimitExceeded));
    // Count 9 is 09.
    let bytes = entity_facts_bytes([W, W, W, W, &[1, 0x09]]);
    assert_eq!(dec_err(&bytes), CodecError::LimitExceeded);
    let bytes = entity_facts_bytes([W, W, W, W, &[1, 0x08]]);
    assert_eq!(dec_err(&bytes), CodecError::Malformed);
}

#[test]
fn b4_name_bounds() {
    let name32 = "n".repeat(32);
    let name33 = "n".repeat(33);
    // A stat name and a tag of 32 bytes are accepted.
    let stat_bytes = entity_facts_bytes([
        W,
        W,
        W,
        &cat(&[&[1, 1, 0x20], name32.as_bytes(), &[0, 0]]),
        W,
    ]);
    assert_eq!(
        dec(&stat_bytes),
        Ok(stats_only(vec![stat(&name32, StatValue::Int(0))]))
    );
    assert_op(
        stats_only(vec![stat(&name32, StatValue::Int(0))]),
        &stat_bytes,
    );
    let tag_bytes = entity_facts_bytes([W, W, W, W, &cat(&[&[1, 1, 0x20], name32.as_bytes()])]);
    assert_op(tags_only(vec![name32.clone()]), &tag_bytes);

    // A length of 33 (21) is LimitExceeded even with no body bytes.
    assert_eq!(
        dec_err(&entity_facts_bytes([W, W, W, &[1, 1, 0x21], W])),
        CodecError::LimitExceeded
    );
    assert_eq!(
        dec_err(&entity_facts_bytes([W, W, W, W, &[1, 1, 0x21]])),
        CodecError::LimitExceeded
    );
    // A length of 0 is Malformed (stat and tag).
    assert_eq!(
        dec_err(&entity_facts_bytes([W, W, W, &[1, 1, 0, 0, 0], W])),
        CodecError::Malformed
    );
    assert_eq!(
        dec_err(&entity_facts_bytes([W, W, W, W, &[1, 1, 0]])),
        CodecError::Malformed
    );
    // Encode agrees.
    assert_eq!(
        try_op_bytes(stats_only(vec![stat(&name33, StatValue::Int(0))])),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        try_op_bytes(tags_only(vec![name33.clone()])),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        try_op_bytes(stats_only(vec![stat("", StatValue::Int(0))])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        try_op_bytes(tags_only(vec![String::new()])),
        Err(CodecError::Malformed)
    );
    // Multi-byte text counts bytes, not characters: 17 two-byte characters.
    let wide = "\u{e9}".repeat(17);
    assert_eq!(wide.len(), 34);
    assert_eq!(
        try_op_bytes(tags_only(vec![wide])),
        Err(CodecError::LimitExceeded)
    );
    let wide16 = "\u{e9}".repeat(16);
    assert_op(
        tags_only(vec![wide16.clone()]),
        &entity_facts_bytes([W, W, W, W, &cat(&[&[1, 1, 32], wide16.as_bytes()])]),
    );
}

// ---------------------------------------------------------------------------
// B5 fact_set_shape
// ---------------------------------------------------------------------------

#[test]
fn b5_pool_shape() {
    let one = |current: u8, max: u8| {
        entity_facts_bytes([&cat(&[&[1, 1], &ulid_wire(1), &[current, max]]), W, W, W, W])
    };
    assert!(dec(&one(3, 3)).is_ok());
    assert_eq!(dec_err(&one(4, 3)), CodecError::Malformed);
    assert_eq!(
        try_op_bytes(pools_only(vec![pool(1, 3, 3)])).map(|b| b.len()),
        Ok(one(3, 3).len())
    );
    assert_eq!(
        try_op_bytes(pools_only(vec![pool(1, 4, 3)])),
        Err(CodecError::Malformed)
    );
    assert_eq!(check_pools(&[pool(1, 3, 3)]), Ok(()));
    assert_eq!(check_pools(&[pool(1, 4, 3)]), Err(CodecError::Malformed));

    // A repeated pool is Malformed; two different pools are fine.
    let two = |a: u8, b: u8| {
        entity_facts_bytes([
            &cat(&[&[1, 2], &ulid_wire(a), &[1, 1], &ulid_wire(b), &[1, 1]]),
            W,
            W,
            W,
            W,
        ])
    };
    assert!(dec(&two(1, 2)).is_ok());
    assert!(dec(&two(2, 1)).is_ok(), "pool order is free");
    assert_eq!(dec_err(&two(1, 1)), CodecError::Malformed);
    assert_eq!(
        try_op_bytes(pools_only(vec![pool(1, 1, 1), pool(1, 2, 2)])),
        Err(CodecError::Malformed)
    );
    assert_eq!(check_pools(&[pool(1, 1, 1), pool(2, 1, 1)]), Ok(()));
    assert_eq!(check_pools(&[pool(2, 1, 1), pool(1, 1, 1)]), Ok(()));
    assert_eq!(
        check_pools(&[pool(1, 1, 1), pool(1, 2, 2)]),
        Err(CodecError::Malformed)
    );
}

#[test]
fn b5_effect_shape() {
    let two = |a: u8, b: u8| {
        entity_facts_bytes([
            W,
            &cat(&[&[1, 2], &ulid_wire(a), &[1], &ulid_wire(b), &[1]]),
            W,
            W,
            W,
        ])
    };
    assert!(dec(&two(1, 2)).is_ok());
    assert_eq!(dec_err(&two(2, 1)), CodecError::Malformed);
    assert_eq!(dec_err(&two(1, 1)), CodecError::Malformed);
    assert_eq!(check_effects(&[effect(1, 1), effect(2, 1)]), Ok(()));
    assert_eq!(
        check_effects(&[effect(2, 1), effect(1, 1)]),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        check_effects(&[effect(1, 1), effect(1, 2)]),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        try_op_bytes(effects_only(vec![effect(2, 1), effect(1, 1)])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        try_op_bytes(effects_only(vec![effect(1, 1), effect(1, 1)])),
        Err(CodecError::Malformed)
    );
    assert!(try_op_bytes(effects_only(vec![effect(1, 1), effect(2, 1)])).is_ok());
    // Three effects: the middle one out of order is caught.
    assert_eq!(
        check_effects(&[effect(1, 0), effect(3, 0), effect(2, 0)]),
        Err(CodecError::Malformed)
    );
}

#[test]
fn b5_stat_shape() {
    let stat_bytes = |parts: &[&[u8]]| entity_facts_bytes([W, W, W, &cat(parts), W]);
    // Control: two distinct names.
    assert!(dec(&stat_bytes(&[
        &[1, 2],
        &text("a"),
        &[0, 2],
        &text("b"),
        &[0, 2]
    ]))
    .is_ok());
    // Repeated name (even with a different kind).
    assert_eq!(
        dec_err(&stat_bytes(&[
            &[1, 2],
            &text("a"),
            &[0, 2],
            &text("a"),
            &[1, 2]
        ])),
        CodecError::Malformed
    );
    assert_eq!(
        check_stats(&[stat("a", StatValue::Int(1)), stat("a", StatValue::Fixed(1))]),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        try_op_bytes(stats_only(vec![
            stat("a", StatValue::Int(1)),
            stat("a", StatValue::Int(1)),
        ])),
        Err(CodecError::Malformed)
    );
    // Kind 02 and FF are unknown; 00 and 01 are the control.
    assert!(dec(&stat_bytes(&[&[1, 1], &text("a"), &[1, 2]])).is_ok());
    assert_eq!(
        dec_err(&stat_bytes(&[&[1, 1], &text("a"), &[2, 2]])),
        CodecError::UnknownMessage
    );
    assert_eq!(
        dec_err(&stat_bytes(&[&[1, 1], &text("a"), &[0xFF, 2]])),
        CodecError::UnknownMessage
    );
    // Invalid UTF-8 name `02 C3 28` is Malformed; valid two-byte name control.
    assert!(dec(&stat_bytes(&[&[1, 1], &[2, 0xC3, 0xA9], &[0, 2]])).is_ok());
    assert_eq!(
        dec_err(&stat_bytes(&[&[1, 1], &[2, 0xC3, 0x28], &[0, 2]])),
        CodecError::Malformed
    );
    // Name body shorter than its length is Malformed.
    assert_eq!(
        dec_err(&stat_bytes(&[&[1, 1], &[5, b'a']])),
        CodecError::Malformed
    );
    // Value bytes.
    assert_eq!(
        dec(&stat_bytes(&[&[1, 1], &text("a"), &[1, 0x80, 0x80, 0x0C]])),
        Ok(stats_only(vec![stat("a", StatValue::Fixed(98304))]))
    );
    assert_eq!(
        op_bytes(stats_only(vec![stat("a", StatValue::Fixed(98304))])),
        entity_facts_bytes([
            W,
            W,
            W,
            &cat(&[&[1, 1], &text("a"), &[1, 0x80, 0x80, 0x0C]]),
            W
        ])
    );
    assert_eq!(
        op_bytes(stats_only(vec![stat("a", StatValue::Int(-3))])),
        entity_facts_bytes([W, W, W, &cat(&[&[1, 1], &text("a"), &[0, 5]]), W])
    );
    assert_eq!(
        op_bytes(stats_only(vec![stat("a", StatValue::Int(i32::MIN))])),
        entity_facts_bytes([
            W,
            W,
            W,
            &cat(&[&[1, 1], &text("a"), &[0, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F]]),
            W
        ])
    );
    for value in [i32::MIN, -3, -1, 0, 1, 98304, i32::MAX] {
        for make in [StatValue::Int, StatValue::Fixed] {
            let op = stats_only(vec![stat("a", make(value))]);
            let bytes = op_bytes(op.clone());
            assert_eq!(dec(&bytes), Ok(op), "value {value}");
            // The independent zigzag oracle agrees with the codec.
            let z = zigzag(value);
            assert_eq!(
                &bytes[bytes.len() - 1 - z.len()..bytes.len() - 1],
                z.as_slice(),
                "value {value}"
            );
        }
    }
    // A value varint beyond i32 (zigzag 0x1_FFFF_FFFF) is Malformed.
    assert_eq!(
        dec_err(&stat_bytes(&[
            &[1, 1],
            &text("a"),
            &[0, 0xFF, 0xFF, 0xFF, 0xFF, 0x1F]
        ])),
        CodecError::Malformed
    );
}

#[test]
fn b5_tag_shape() {
    let tag_bytes = |parts: &[&[u8]]| entity_facts_bytes([W, W, W, W, &cat(parts)]);
    assert!(dec(&tag_bytes(&[&[1, 2], &text("a"), &text("b")])).is_ok());
    assert_eq!(
        dec_err(&tag_bytes(&[&[1, 2], &text("b"), &text("a")])),
        CodecError::Malformed
    );
    assert_eq!(
        dec_err(&tag_bytes(&[&[1, 2], &text("a"), &text("a")])),
        CodecError::Malformed
    );
    assert_eq!(check_tags(&tags(&["a", "b"])), Ok(()));
    assert_eq!(check_tags(&tags(&["b", "a"])), Err(CodecError::Malformed));
    assert_eq!(check_tags(&tags(&["a", "a"])), Err(CodecError::Malformed));
    assert_eq!(
        try_op_bytes(tags_only(tags(&["b", "a"]))),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        try_op_bytes(tags_only(tags(&["a", "a"]))),
        Err(CodecError::Malformed)
    );
    // Byte order, not locale: uppercase sorts before lowercase, and a prefix
    // before its extension.
    assert_eq!(check_tags(&tags(&["B", "a"])), Ok(()));
    assert_eq!(check_tags(&tags(&["a", "B"])), Err(CodecError::Malformed));
    assert_eq!(check_tags(&tags(&["a", "ab"])), Ok(()));
    assert_eq!(check_tags(&tags(&["ab", "a"])), Err(CodecError::Malformed));
    // Invalid UTF-8 tag.
    assert_eq!(
        dec_err(&tag_bytes(&[&[1, 1], &[2, 0xC3, 0x28]])),
        CodecError::Malformed
    );
}

#[test]
fn b5_order_shape() {
    let order = |target: u8, repeat: u8| {
        entity_facts_bytes([
            W,
            W,
            &cat(&[&[1, 1], &ulid_wire(3), &[target, repeat]]),
            W,
            W,
        ])
    };
    assert_eq!(
        dec(&order(1, 1)),
        Ok(facts(
            2,
            FactSet {
                order: Some(Some(OrderFact {
                    ability: u(3),
                    target: net(1),
                    repeat: true
                })),
                ..FactSet::default()
            }
        ))
    );
    assert_eq!(
        dec(&order(1, 0)),
        Ok(facts(
            2,
            FactSet {
                order: Some(Some(OrderFact {
                    ability: u(3),
                    target: net(1),
                    repeat: false
                })),
                ..FactSet::default()
            }
        ))
    );
    assert_eq!(dec_err(&order(0, 1)), CodecError::Malformed); // target 0
    assert_eq!(dec_err(&order(1, FLAG_BAD)), CodecError::Malformed); // repeat flag
}

#[test]
fn b5_flag_bytes_other_than_0_or_1_are_malformed() {
    // Each of the five disclosed flags, the order `some` flag.
    let bad = [FLAG_BAD];
    assert_eq!(
        dec_err(&entity_facts_bytes([&bad, W, W, W, W])),
        CodecError::Malformed
    );
    assert_eq!(
        dec_err(&entity_facts_bytes([W, &bad, W, W, W])),
        CodecError::Malformed
    );
    assert_eq!(
        dec_err(&entity_facts_bytes([W, W, &bad, W, W])),
        CodecError::Malformed
    );
    assert_eq!(
        dec_err(&entity_facts_bytes([W, W, &[1, FLAG_BAD], W, W])),
        CodecError::Malformed
    );
    assert_eq!(
        dec_err(&entity_facts_bytes([W, W, W, &bad, W])),
        CodecError::Malformed
    );
    assert_eq!(
        dec_err(&entity_facts_bytes([W, W, W, W, &bad])),
        CodecError::Malformed
    );
    // Control: all-withheld decodes. Entity 0 is Malformed; truncation and
    // trailing bytes too.
    assert!(dec(&entity_facts_bytes([W, W, W, W, W])).is_ok());
    assert_eq!(dec_err(&[0x0D, 0, 0, 0, 0, 0, 0]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x0D, 2, 0, 0, 0, 0]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x0D, 2, 0, 0, 0, 0, 0, 0]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x0D]), CodecError::Malformed);
    assert_eq!(dec_err(&[0x0D, 2]), CodecError::Malformed);
}

// ---------------------------------------------------------------------------
// B6 fact_precedence
// ---------------------------------------------------------------------------

#[test]
fn b6_fact_precedence() {
    let bad_pool = cat(&[&[1, 1], &ulid_wire(1), &[4, 3]]);
    let good_pool = cat(&[&[1, 1], &ulid_wire(1), &[3, 3]]);
    let stats33 = [1u8, 0x21];
    // Pools current > max comes first: Malformed.
    assert_eq!(
        dec_err(&entity_facts_bytes([&bad_pool, W, W, &stats33, W])),
        CodecError::Malformed
    );
    // Control: with a valid pool the stats count is the first failure.
    assert_eq!(
        dec_err(&entity_facts_bytes([&good_pool, W, W, &stats33, W])),
        CodecError::LimitExceeded
    );
    // Left to right on encode too.
    let both = facts(
        2,
        FactSet {
            pools: Some(vec![pool(1, 4, 3)]),
            stats: Some(n_stats(33)),
            ..FactSet::default()
        },
    );
    assert_eq!(try_op_bytes(both), Err(CodecError::Malformed));
    let stats_first = facts(
        2,
        FactSet {
            pools: Some(vec![pool(1, 3, 3)]),
            stats: Some(n_stats(33)),
            ..FactSet::default()
        },
    );
    assert_eq!(try_op_bytes(stats_first), Err(CodecError::LimitExceeded));

    // A stat name length of 33 followed by invalid UTF-8 is LimitExceeded: the
    // length is checked before the body. Control: length 2 and the same bytes.
    let mut long_name = vec![1u8, 1, 0x21];
    long_name.extend([0xC3, 0x28]);
    assert_eq!(
        dec_err(&entity_facts_bytes([W, W, W, &long_name, W])),
        CodecError::LimitExceeded
    );
    assert_eq!(
        dec_err(&entity_facts_bytes([
            W,
            W,
            W,
            &[1, 1, 2, 0xC3, 0x28, 0, 0],
            W
        ])),
        CodecError::Malformed
    );
    // Name length 0 before a bad kind: the name error wins (left to right).
    assert_eq!(
        dec_err(&entity_facts_bytes([W, W, W, &[1, 1, 0, 9, 0], W])),
        CodecError::Malformed
    );
    // A bad kind before a duplicate name: the kind error wins.
    assert_eq!(
        dec_err(&entity_facts_bytes([
            W,
            W,
            W,
            &cat(&[&[1, 2], &text("a"), &[0, 2], &text("a"), &[7, 2]]),
            W
        ])),
        CodecError::UnknownMessage
    );
    // A bad family earlier in the set beats a missing later one: pools with a
    // repeated id, then truncation.
    let dup = cat(&[&[1, 2], &ulid_wire(1), &[1, 1], &ulid_wire(1), &[1, 1]]);
    let mut truncated = vec![0x0D, 2];
    truncated.extend(&dup);
    assert_eq!(dec_err(&truncated), CodecError::Malformed);
}

// ---------------------------------------------------------------------------
// B7 check_functions_match_codec
// ---------------------------------------------------------------------------

fn assert_encode_matches(op: DeltaOp, expected: Result<(), CodecError>) {
    assert_eq!(try_op_bytes(op).map(|_| ()), expected);
}

#[test]
fn b7_clock_check_matches_codec() {
    let clock_op = |c: ClockFact| {
        DeltaOp::Encounter(EncounterFacts {
            clock: Some(Some(c)),
            ..EncounterFacts::default()
        })
    };
    let cases = [
        (
            ClockFact {
                round_ticks: 1,
                elapsed: 0,
            },
            Ok(()),
        ),
        (
            ClockFact {
                round_ticks: 120,
                elapsed: MAX_CLOCK_ELAPSED,
            },
            Ok(()),
        ),
        (
            ClockFact {
                round_ticks: 0,
                elapsed: 0,
            },
            Err(CodecError::Malformed),
        ),
        (
            ClockFact {
                round_ticks: 1,
                elapsed: MAX_CLOCK_ELAPSED + 1,
            },
            Err(CodecError::Malformed),
        ),
        (
            ClockFact {
                round_ticks: 0,
                elapsed: u64::MAX,
            },
            Err(CodecError::Malformed),
        ),
    ];
    for (clock, expected) in cases {
        assert_eq!(clock.check(), expected, "{clock:?}");
        assert_encode_matches(clock_op(clock), expected);
    }
}

#[test]
fn b7_list_checks_match_codec() {
    // Pools.
    let pool_cases: Vec<(Vec<PoolFact>, Result<(), CodecError>)> = vec![
        (vec![], Ok(())),
        (n_pools(16), Ok(())),
        (n_pools(17), Err(CodecError::LimitExceeded)),
        (vec![pool(1, 3, 3)], Ok(())),
        (vec![pool(1, 4, 3)], Err(CodecError::Malformed)),
        (
            vec![pool(1, 1, 1), pool(1, 1, 1)],
            Err(CodecError::Malformed),
        ),
        // Count beats shape, element order beats later elements.
        (
            {
                let mut v = n_pools(17);
                v[0] = pool(1, 9, 1);
                v
            },
            Err(CodecError::LimitExceeded),
        ),
    ];
    for (list, expected) in pool_cases {
        assert_eq!(check_pools(&list), expected);
        assert_encode_matches(pools_only(list), expected);
    }
    // Effects.
    let effect_cases: Vec<(Vec<EffectFact>, Result<(), CodecError>)> = vec![
        (vec![], Ok(())),
        (n_effects(16), Ok(())),
        (n_effects(17), Err(CodecError::LimitExceeded)),
        (vec![effect(1, 0), effect(2, 0)], Ok(())),
        (vec![effect(2, 0), effect(1, 0)], Err(CodecError::Malformed)),
        (vec![effect(1, 0), effect(1, 9)], Err(CodecError::Malformed)),
    ];
    for (list, expected) in effect_cases {
        assert_eq!(check_effects(&list), expected);
        assert_encode_matches(effects_only(list), expected);
    }
    // Stats.
    let long = "n".repeat(33);
    let stat_cases: Vec<(Vec<StatFact>, Result<(), CodecError>)> = vec![
        (vec![], Ok(())),
        (n_stats(32), Ok(())),
        (n_stats(33), Err(CodecError::LimitExceeded)),
        (vec![stat(&"n".repeat(32), StatValue::Int(0))], Ok(())),
        (
            vec![stat(&long, StatValue::Int(0))],
            Err(CodecError::LimitExceeded),
        ),
        (
            vec![stat("", StatValue::Int(0))],
            Err(CodecError::Malformed),
        ),
        (
            vec![stat("a", StatValue::Int(0)), stat("a", StatValue::Fixed(0))],
            Err(CodecError::Malformed),
        ),
        // List order: the first failing element decides.
        (
            vec![stat("", StatValue::Int(0)), stat(&long, StatValue::Int(0))],
            Err(CodecError::Malformed),
        ),
        (
            vec![stat(&long, StatValue::Int(0)), stat("", StatValue::Int(0))],
            Err(CodecError::LimitExceeded),
        ),
    ];
    for (list, expected) in stat_cases {
        assert_eq!(check_stats(&list), expected);
        assert_encode_matches(stats_only(list), expected);
    }
    // Tags.
    let tag_cases: Vec<(Vec<String>, Result<(), CodecError>)> = vec![
        (vec![], Ok(())),
        (n_tags(8), Ok(())),
        (n_tags(9), Err(CodecError::LimitExceeded)),
        (vec!["n".repeat(32)], Ok(())),
        (vec![long.clone()], Err(CodecError::LimitExceeded)),
        (vec![String::new()], Err(CodecError::Malformed)),
        (tags(&["a", "b"]), Ok(())),
        (tags(&["b", "a"]), Err(CodecError::Malformed)),
        (tags(&["a", "a"]), Err(CodecError::Malformed)),
    ];
    for (list, expected) in tag_cases {
        assert_eq!(check_tags(&list), expected);
        assert_encode_matches(tags_only(list), expected);
    }
    // Timeline.
    let tl_cases: Vec<(TimelineInput, Result<(), CodecError>)> = vec![
        (vec![], Ok(())),
        (vec![(-1, 2), (-1, 1), (5, 3)], Ok(())),
        (vec![(5, 1), (4, 2)], Err(CodecError::Malformed)),
        (vec![(1, 1), (2, 1)], Err(CodecError::Malformed)),
        ((1..=1024u64).map(|n| (0, n)).collect(), Ok(())),
        (
            (1..=1025u64).map(|n| (0, n)).collect(),
            Err(CodecError::LimitExceeded),
        ),
    ];
    for (list, expected) in tl_cases {
        assert_eq!(check_timeline(&tl(&list)), expected);
        assert_encode_matches(timeline(&list), expected);
    }
}

#[test]
fn b7_encode_checks_run_per_op_in_order() {
    let bad_timeline = timeline(&[(5, 1), (4, 2)]);
    let bad_pools = pools_only(vec![pool(1, 4, 3)]);
    let bad_receipt = DeltaOp::Receipt {
        epoch: [0; 16],
        lane: 1,
        seq: 1,
        processed_tick: 1,
        status: ReceiptStatus::Applied,
    };
    // First op wins.
    assert_eq!(
        encode_delta(&delta(vec![bad_receipt.clone(), bad_timeline.clone()])),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        encode_delta(&delta(vec![bad_timeline.clone(), bad_receipt.clone()])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        encode_delta(&delta(vec![pools_only(n_pools(17)), bad_pools.clone()])),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        encode_delta(&delta(vec![bad_pools, pools_only(n_pools(17))])),
        Err(CodecError::Malformed)
    );
    // Frame lane and op count come before any op check.
    let mut frame = delta(vec![bad_timeline.clone()]);
    frame.lane = 1;
    assert_eq!(encode_delta(&frame), Err(CodecError::WrongDirection));
    let many = vec![bad_timeline; MAX_DELTA_OPS + 1];
    assert_eq!(encode_delta(&delta(many)), Err(CodecError::LimitExceeded));
}

// ---------------------------------------------------------------------------
// Frame-level behaviour of the fact ops.
// ---------------------------------------------------------------------------

#[test]
fn fact_ops_mix_in_one_frame_and_round_trip() {
    let ops = vec![
        DeltaOp::Encounter(EncounterFacts {
            mode: Some(EncounterMode::RealTime),
            ..EncounterFacts::default()
        }),
        timeline(&[(0, 1), (0, 2)]),
        pools_only(vec![pool(1, 1, 1)]),
        DeltaOp::RoundStarted { round: 3 },
        facts(2, FactSet::default()),
    ];
    let frame = delta(ops);
    let bytes = encode_delta(&frame).unwrap();
    assert_eq!(&bytes[..5], &[3, 0, 0x64, 1, 5]);
    assert_eq!(decode_delta(&bytes), Ok(frame.clone()));
    // Canonical: encoding twice gives the same bytes.
    assert_eq!(encode_delta(&frame).unwrap(), bytes);
}

#[test]
fn v2_codec_still_does_not_know_the_fact_tags() {
    for tag in [0x0Bu8, 0x0C, 0x0D] {
        assert_eq!(
            codec_v2::decode_delta(&[2, 0, 0x64, 1, 1, tag, 0, 0, 0]),
            Err(CodecError::UnknownMessage)
        );
    }
}

// ---------------------------------------------------------------------------
// B8 project_events_order_and_suppression
// ---------------------------------------------------------------------------

#[test]
fn b8_project_events_order_and_suppression() {
    let input = vec![
        EventCandidate::V2(V2Candidate::TurnStarted {
            actor: Some(net(1)),
            round: Some(1),
        }),
        EventCandidate::RoundStarted { round: Some(2) },
        EventCandidate::ModeChanged {
            mode: Some(EncounterMode::RealTime),
            round: None,
        },
        EventCandidate::ModeChanged {
            mode: Some(EncounterMode::RealTime),
            round: Some(2),
        },
        EventCandidate::OrderEnded { actor: None },
        EventCandidate::OrderEnded {
            actor: Some(net(3)),
        },
        EventCandidate::Joined {
            entity: Some(net(4)),
            disclose: false,
        },
        EventCandidate::Joined {
            entity: Some(net(4)),
            disclose: true,
        },
        EventCandidate::Left {
            entity: Some(net(4)),
            disclose: true,
        },
    ];
    assert_eq!(
        project_events(&input),
        Ok(vec![
            DeltaOp::TurnStarted {
                actor: net(1),
                round: 1
            },
            DeltaOp::RoundStarted { round: 2 },
            DeltaOp::ModeChanged {
                mode: EncounterMode::RealTime,
                round: 2
            },
            DeltaOp::OrderEnded { actor: net(3) },
            DeltaOp::Joined { entity: net(4) },
            DeltaOp::Left { entity: net(4) },
        ])
    );
}

#[test]
fn b8_each_candidate_needs_every_field() {
    // RoundStarted.
    assert_eq!(
        project_events(&[EventCandidate::RoundStarted { round: None }]),
        Ok(vec![])
    );
    assert_eq!(
        project_events(&[EventCandidate::RoundStarted { round: Some(0) }]),
        Ok(vec![DeltaOp::RoundStarted { round: 0 }])
    );
    // ModeChanged: both fields, in both directions.
    for (mode, round) in [
        (None, None),
        (Some(EncounterMode::TurnBased), None),
        (None, Some(5)),
    ] {
        assert_eq!(
            project_events(&[EventCandidate::ModeChanged { mode, round }]),
            Ok(vec![]),
            "{mode:?} {round:?}"
        );
    }
    assert_eq!(
        project_events(&[EventCandidate::ModeChanged {
            mode: Some(EncounterMode::TurnBased),
            round: Some(5)
        }]),
        Ok(vec![DeltaOp::ModeChanged {
            mode: EncounterMode::TurnBased,
            round: 5
        }])
    );
    // OrderEnded.
    assert_eq!(
        project_events(&[EventCandidate::OrderEnded { actor: None }]),
        Ok(vec![])
    );
    // Joined / Left: entity and grant.
    for (entity, disclose) in [(None, false), (None, true), (Some(net(9)), false)] {
        assert_eq!(
            project_events(&[EventCandidate::Joined { entity, disclose }]),
            Ok(vec![]),
            "joined {entity:?} {disclose}"
        );
        assert_eq!(
            project_events(&[EventCandidate::Left { entity, disclose }]),
            Ok(vec![]),
            "left {entity:?} {disclose}"
        );
    }
    assert_eq!(
        project_events(&[EventCandidate::Joined {
            entity: Some(net(9)),
            disclose: true
        }]),
        Ok(vec![DeltaOp::Joined { entity: net(9) }])
    );
    assert_eq!(
        project_events(&[EventCandidate::Left {
            entity: Some(net(9)),
            disclose: true
        }]),
        Ok(vec![DeltaOp::Left { entity: net(9) }])
    );
}

#[test]
fn b8_v2_candidates_map_like_projection_v2() {
    let ability = u(7);
    let enc = u(8);
    let cases: Vec<(V2Candidate, Vec<DeltaOp>)> = vec![
        (
            V2Candidate::Spawned {
                entity: Some(net(1)),
                disclose: true,
            },
            vec![DeltaOp::Spawned { entity: net(1) }],
        ),
        (
            V2Candidate::Despawned {
                entity: Some(net(1)),
                disclose: true,
            },
            vec![DeltaOp::Despawned { entity: net(1) }],
        ),
        (
            V2Candidate::Died {
                entity: Some(net(1)),
                disclose: true,
            },
            vec![DeltaOp::Died { entity: net(1) }],
        ),
        (
            V2Candidate::Died {
                entity: Some(net(1)),
                disclose: false,
            },
            vec![],
        ),
        (
            V2Candidate::ActionResolved {
                actor: Some(net(1)),
                target: Some(net(2)),
                ability: Some(ability),
                outcome: Some("success".to_owned()),
                damage: Some(7),
            },
            vec![DeltaOp::ActionResolved {
                actor: net(1),
                target: net(2),
                ability,
                outcome: "success".to_owned(),
                damage: 7,
            }],
        ),
        (
            V2Candidate::TurnStarted {
                actor: Some(net(1)),
                round: None,
            },
            vec![],
        ),
        (
            V2Candidate::EncounterEnded {
                encounter: Some(enc),
                round: Some(3),
            },
            vec![DeltaOp::EncounterEnded {
                encounter: enc,
                round: 3,
            }],
        ),
    ];
    for (candidate, expected) in cases {
        assert_eq!(
            project_events(&[EventCandidate::V2(candidate.clone())]),
            Ok(expected),
            "{candidate:?}"
        );
    }
}

#[test]
fn b8_limits_errors_and_no_dedup() {
    // 256 candidates are accepted, 257 are TooManyEvents, even if all omitted.
    let omitted = EventCandidate::RoundStarted { round: None };
    assert_eq!(
        project_events(&vec![omitted.clone(); MAX_DELTA_OPS]),
        Ok(vec![])
    );
    assert_eq!(
        project_events(&vec![omitted; MAX_DELTA_OPS + 1]),
        Err(ProjectionError::TooManyEvents)
    );
    let shown = EventCandidate::RoundStarted { round: Some(1) };
    assert_eq!(
        project_events(&vec![shown.clone(); MAX_DELTA_OPS])
            .unwrap()
            .len(),
        MAX_DELTA_OPS
    );
    assert_eq!(
        project_events(&vec![shown; MAX_DELTA_OPS + 1]),
        Err(ProjectionError::TooManyEvents)
    );
    assert_eq!(project_events(&[]), Ok(vec![]));

    // A fully disclosed bogus outcome fails the whole call, even after good
    // events; an omitted one (actor None) does not.
    let resolved = |actor: Option<NetId>, outcome: &str| {
        EventCandidate::V2(V2Candidate::ActionResolved {
            actor,
            target: Some(net(2)),
            ability: Some(u(7)),
            outcome: Some(outcome.to_owned()),
            damage: Some(1),
        })
    };
    assert_eq!(
        project_events(&[resolved(Some(net(1)), "bogus")]),
        Err(ProjectionError::InvalidOutcome)
    );
    assert_eq!(
        project_events(&[
            EventCandidate::RoundStarted { round: Some(1) },
            resolved(Some(net(1)), "bogus")
        ]),
        Err(ProjectionError::InvalidOutcome)
    );
    assert_eq!(project_events(&[resolved(None, "bogus")]), Ok(vec![]));
    assert!(project_events(&[resolved(Some(net(1)), "success")]).is_ok());
    assert_eq!(
        project_events(&[resolved(Some(net(1)), &"a".repeat(257))]),
        Err(ProjectionError::StringLimit)
    );

    // Repeats are kept and the order is the input order.
    let repeated = [
        EventCandidate::OrderEnded {
            actor: Some(net(5)),
        },
        EventCandidate::OrderEnded {
            actor: Some(net(2)),
        },
        EventCandidate::OrderEnded {
            actor: Some(net(5)),
        },
    ];
    assert_eq!(
        project_events(&repeated),
        Ok(vec![
            DeltaOp::OrderEnded { actor: net(5) },
            DeltaOp::OrderEnded { actor: net(2) },
            DeltaOp::OrderEnded { actor: net(5) },
        ])
    );
}

// ---------------------------------------------------------------------------
// B9 frame_spans_and_frame_cap
// ---------------------------------------------------------------------------

/// `T(k)`: a timeline of `k` entries `(0, 2^63 + i)`.
fn t(k: u64) -> DeltaOp {
    let entries: Vec<(i32, u64)> = (0..k).map(|i| (0, (1u64 << 63) + i)).collect();
    timeline(&entries)
}

/// `R(r)`: `RoundStarted { r }`.
fn r(round: u64) -> DeltaOp {
    DeltaOp::RoundStarted { round }
}

fn max_frame(ops: Vec<DeltaOp>) -> DeltaFrame {
    DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: u64::MAX,
        event_seq: u64::MAX,
        ops,
    }
}

fn pre_cap_ops() -> Vec<DeltaOp> {
    let mut ops = Vec::new();
    for _ in 0..5 {
        ops.push(t(1024));
    }
    ops.push(t(711));
    for _ in 0..122 {
        ops.push(r(u64::MAX));
    }
    ops
}

#[test]
fn b9_op_sizes_match_the_contract() {
    assert_eq!(op_bytes(t(1024)).len(), 11_268);
    assert_eq!(op_bytes(t(711)).len(), 7_825);
    assert_eq!(op_bytes(t(128)).len(), 4 + 11 * 128);
    assert_eq!(op_bytes(r(u64::MAX)).len(), 11);
    assert_eq!(op_bytes(r(1 << 21)).len(), 5);
    assert_eq!(op_bytes(r(1 << 28)).len(), 6);
}

#[test]
fn b9_frame_at_the_cap_is_one_span_and_encodes_exactly() {
    let mut a = pre_cap_ops();
    a.push(r(1 << 21));
    assert_eq!(a.len(), 129);
    let total: usize = a.iter().map(|op| op_bytes(op.clone()).len()).sum();
    assert_eq!(total, MAX_FRAME_OP_BYTES_V3);
    assert_eq!(total, 65_512);
    assert_eq!(frame_spans(&a), Ok(spans(&[(0, 129)])));

    let frame = max_frame(a);
    let bytes = encode_delta(&frame).expect("65,536 bytes is within the cap");
    assert_eq!(bytes.len(), 65_536);
    let mut header = vec![3, 0];
    header.extend([0xFF; 9]);
    header.push(0x01);
    header.extend([0xFF; 9]);
    header.push(0x01);
    header.extend([0x81, 0x01]);
    assert_eq!(header.len(), 24);
    assert_eq!(&bytes[..24], header.as_slice());
    assert_eq!(decode_delta(&bytes), Ok(frame));
}

#[test]
fn b9_frame_one_byte_over_the_cap_splits_and_does_not_encode() {
    let mut b = pre_cap_ops();
    b.push(r(1 << 28));
    assert_eq!(b.len(), 129);
    let total: usize = b.iter().map(|op| op_bytes(op.clone()).len()).sum();
    assert_eq!(total, 65_513);
    assert_eq!(frame_spans(&b), Ok(spans(&[(0, 128), (128, 129)])));
    assert_eq!(
        encode_delta(&max_frame(b.clone())),
        Err(CodecError::FrameTooLarge)
    );
    // Each span encodes within the cap with the largest header.
    for span in frame_spans(&b).unwrap() {
        let bytes = encode_delta(&max_frame(b[span].to_vec())).unwrap();
        assert!(bytes.len() <= MAX_DELTA_FRAME_BYTES);
    }
}

#[test]
fn b9_op_count_cap_splits_at_256() {
    let n256 = vec![r(0); 256];
    assert_eq!(frame_spans(&n256), Ok(spans(&[(0, 256)])));
    let n257 = vec![r(0); 257];
    assert_eq!(frame_spans(&n257), Ok(spans(&[(0, 256), (256, 257)])));
    let n512 = vec![r(0); 512];
    assert_eq!(frame_spans(&n512), Ok(spans(&[(0, 256), (256, 512)])));
    let n513 = vec![r(0); 513];
    assert_eq!(
        frame_spans(&n513),
        Ok(spans(&[(0, 256), (256, 512), (512, 513)]))
    );
    assert_eq!(frame_spans(&[]), Ok(vec![]));
    assert_eq!(frame_spans(&[r(0)]), Ok(spans(&[(0, 1)])));
}

#[test]
fn b9_a_large_op_closes_the_frame_before_it() {
    // Six T(1024) are 67,608 bytes: five fit (56,340), the sixth opens frame 2.
    let ops: Vec<DeltaOp> = (0..6).map(|_| t(1024)).collect();
    assert_eq!(frame_spans(&ops), Ok(spans(&[(0, 5), (5, 6)])));
    // Spans are contiguous and in order for a mixed list.
    let mut mixed = vec![r(0); 3];
    mixed.extend((0..6).map(|_| t(1024)));
    mixed.extend(vec![r(1); 3]);
    let spans = frame_spans(&mixed).unwrap();
    assert_eq!(spans.first().unwrap().start, 0);
    assert_eq!(spans.last().unwrap().end, mixed.len());
    for pair in spans.windows(2) {
        assert_eq!(pair[0].end, pair[1].start);
    }
    for span in &spans {
        assert!(!span.is_empty());
        let bytes = encode_delta(&max_frame(mixed[span.clone()].to_vec())).unwrap();
        assert!(bytes.len() <= MAX_DELTA_FRAME_BYTES);
    }
}

#[test]
fn b9_frame_spans_checks_every_op_first() {
    let bogus = DeltaOp::ActionResolved {
        actor: net(1),
        target: net(2),
        ability: u(1),
        outcome: "bogus".to_owned(),
        damage: 0,
    };
    // Control: the same list without the bad op splits.
    assert_eq!(frame_spans(&[r(0)]), Ok(spans(&[(0, 1)])));
    assert_eq!(
        frame_spans(&[r(0), bogus.clone()]),
        Err(CodecError::Malformed)
    );
    // The failure wins even where a split would have happened earlier.
    let mut many = vec![r(0); 300];
    many.push(bogus);
    assert_eq!(frame_spans(&many), Err(CodecError::Malformed));
    // Fact-op checks are part of it.
    assert_eq!(
        frame_spans(&[r(0), timeline(&[(5, 1), (4, 2)])]),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        frame_spans(&[pools_only(n_pools(17))]),
        Err(CodecError::LimitExceeded)
    );
    // First failure wins, in op order.
    assert_eq!(
        frame_spans(&[
            pools_only(n_pools(17)),
            DeltaOp::Receipt {
                epoch: [0; 16],
                lane: 1,
                seq: 1,
                processed_tick: 0,
                status: ReceiptStatus::Applied
            }
        ]),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        frame_spans(&[
            DeltaOp::Receipt {
                epoch: [0; 16],
                lane: 1,
                seq: 1,
                processed_tick: 0,
                status: ReceiptStatus::Applied
            },
            pools_only(n_pools(17)),
        ]),
        Err(CodecError::WrongDirection)
    );
}

/// The maximal per-entity fact set of D-4 (2,744 bytes).
fn maximal_set() -> FactSet {
    FactSet {
        pools: Some(
            (0..16u128)
                .map(|i| pool(i + 1, u32::MAX, u32::MAX))
                .collect(),
        ),
        effects: Some((0..16u128).map(|i| effect(i + 1, u64::MAX)).collect()),
        order: Some(Some(OrderFact {
            ability: u(1),
            target: net(u64::MAX),
            repeat: true,
        })),
        stats: Some(
            (0..32)
                .map(|i| stat(&format!("{i:0>32}"), StatValue::Int(i32::MIN)))
                .collect(),
        ),
        tags: Some((0..8).map(|i| format!("{i:a>32}")).collect()),
    }
}

#[test]
fn b9_page_of_maximal_entity_facts_splits_into_fitting_frames() {
    let ops: Vec<DeltaOp> = (0..1024u64)
        .map(|i| facts((1u64 << 63) + i, maximal_set()))
        .collect();
    // Each op is exactly 2,755 bytes: tag 1, entity 10, fact set 2,744.
    assert_eq!(op_bytes(ops[0].clone()).len(), 2_755);
    let spans = frame_spans(&ops).unwrap();
    // floor(65,512 / 2,755) = 23 ops per frame: 44 full frames and a final 12.
    assert_eq!(spans.len(), 45);
    for (index, span) in spans.iter().enumerate() {
        let expected = if index < 44 { 23 } else { 12 };
        assert_eq!(span.len(), expected, "span {index}");
        let bytes = encode_delta(&max_frame(ops[span.clone()].to_vec())).unwrap();
        assert!(bytes.len() <= MAX_DELTA_FRAME_BYTES, "span {index}");
        assert_eq!(decode_delta(&bytes).unwrap().ops.len(), expected);
    }
    // Counting ops alone would not fit: 256 of them are refused.
    assert_eq!(
        encode_delta(&max_frame(ops[..256].to_vec())),
        Err(CodecError::FrameTooLarge)
    );
}

/// Index ranges from `(start, end)` pairs.
fn spans(pairs: &[(usize, usize)]) -> Vec<std::ops::Range<usize>> {
    pairs.iter().map(|&(start, end)| start..end).collect()
}

/// `(key, entity)` pairs of a timeline fixture.
type TimelineInput = Vec<(i32, u64)>;
