//! T026p1 two-keys contract tests: protocol v4, lane 0. Written from
//! `tasks/T026p.md` (and the T140 literals it cites) alone, through the public
//! `crpg-net` API. Every byte oracle is hand-derived (postcard: raw `u8` and
//! `bool`, LEB128 varints, zigzag `i32`, raw `[u8; 16]`); none was produced by
//! running the codec under test. The two small encoders below (`varint`, `zz`)
//! model the layout independently and are used only to build long literals.

// `frame_spans` returns a `Vec` of ranges; one-range expectations are the point.
#![allow(clippy::single_range_in_vec_init)]

use crpg_core::Ulid;
use crpg_net::codec::{self as v1, CodecError};
use crpg_net::codec_v2 as v2;
use crpg_net::codec_v3 as c3;
use crpg_net::codec_v4 as c4;
use crpg_net::protocol as p1;
use crpg_net::protocol_v3::{
    self as p3, AutoPauseToggles, AutoPauseTrigger, EffectFact, EncounterFacts, FactSet, HostFacts,
    OrderFact, PauseReason, PauseRights, PoolFact, StatFact, StatValue, TimelineEntry,
};
use crpg_net::protocol_v4::{
    self as p4, DeltaFrame, DeltaOp, EncounterMode, IntentBody, IntentFrame, NetId, Point,
    PositionFact, ReceiptStatus, RejectionCode,
};

const Z16: [u8; 16] = [0; 16];
/// `2^29`, the coordinate limit.
const L: i32 = 1 << 29;
/// Varint of `u64::MAX`.
const MAX64: [u8; 10] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];
/// Varint of `u64::MAX - 1` (`SEQ_LAST`).
const LAST64: [u8; 10] = [0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];
/// `L` as a zigzag varint.
const BL: [u8; 5] = [0x80, 0x80, 0x80, 0x80, 0x04];
/// `-L` as a zigzag varint.
const BNL: [u8; 5] = [0xFF, 0xFF, 0xFF, 0xFF, 0x03];
/// `L + 1`.
const BL1: [u8; 5] = [0x82, 0x80, 0x80, 0x80, 0x04];
/// `-L - 1`.
const BNL1: [u8; 5] = [0x81, 0x80, 0x80, 0x80, 0x04];
/// `i32::MIN`.
const BMIN: [u8; 5] = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
/// A varint that overflows `i32`.
const BOVER: [u8; 5] = [0xFF, 0xFF, 0xFF, 0xFF, 0x1F];

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

fn net(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero fixture id")
}

fn ulid(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

fn pt(x: i32, y: i32, z: i32) -> Point {
    Point::new([x, y, z]).expect("point within bounds")
}

fn a() -> Point {
    pt(0, 0, 0)
}

fn b() -> Point {
    pt(65536, 0, -131072)
}

fn varint(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let low = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 {
            out.push(low);
            return out;
        }
        out.push(low | 0x80);
    }
}

fn zz(v: i32) -> Vec<u8> {
    varint(u64::from(((v << 1) ^ (v >> 31)) as u32))
}

fn point_bytes(p: Point) -> Vec<u8> {
    let [x, y, z] = p.raw();
    [zz(x), zz(y), zz(z)].concat()
}

/// Length-prefixed ULID text of `ulid(n)` (26 bytes, prefix `1A`).
fn utext(n: u128) -> Vec<u8> {
    let text = ulid(n).to_string();
    assert_eq!(text.len(), 26);
    [&[0x1A][..], text.as_bytes()].concat()
}

/// `first Z16 lane seq 07 actor` plus `tail`.
fn iraw(first: u8, lane: u8, seq: &[u8], actor: &[u8], tail: &[u8]) -> Vec<u8> {
    [&[first][..], &Z16, &[lane], seq, &[7], actor, tail].concat()
}

/// Standard v4 intent (lane 0, seq 1, observed_tick 7, actor 1) plus `tail`.
fn ib(tail: &[u8]) -> Vec<u8> {
    iraw(4, 0, &[1], &[1], tail)
}

fn ifr(body: IntentBody) -> IntentFrame {
    IntentFrame {
        epoch: Z16,
        lane: 0,
        seq: 1,
        observed_tick: 7,
        actor: net(1),
        body,
    }
}

fn ifr3(body: p3::IntentBody) -> p3::IntentFrame {
    p3::IntentFrame {
        epoch: Z16,
        lane: 0,
        seq: 1,
        observed_tick: 7,
        actor: net(1),
        body,
    }
}

fn conv_intent(f: p3::IntentFrame) -> IntentFrame {
    IntentFrame {
        epoch: f.epoch,
        lane: f.lane,
        seq: f.seq,
        observed_tick: f.observed_tick,
        actor: f.actor,
        body: IntentBody::from(f.body),
    }
}

fn dfr(ops: Vec<DeltaOp>) -> DeltaFrame {
    DeltaFrame {
        lane: 0,
        server_tick: 100,
        event_seq: 1,
        ops,
    }
}

fn dfr3(ops: Vec<p3::DeltaOp>) -> p3::DeltaFrame {
    p3::DeltaFrame {
        lane: 0,
        server_tick: 100,
        event_seq: 1,
        ops,
    }
}

fn wrap_frame(f: p3::DeltaFrame) -> DeltaFrame {
    DeltaFrame {
        lane: f.lane,
        server_tick: f.server_tick,
        event_seq: f.event_seq,
        ops: f.ops.into_iter().map(DeltaOp::V3).collect(),
    }
}

/// One-op v4 delta frame bytes: `04 00 64 01 01` plus the op bytes.
fn d1(op: &[u8]) -> Vec<u8> {
    [&[4, 0, 0x64, 1, 1][..], op].concat()
}

/// Two-op v4 delta frame bytes: `04 00 64 01 02` plus the op bytes.
fn d2(ops: &[u8]) -> Vec<u8> {
    [&[4, 0, 0x64, 1, 2][..], ops].concat()
}

fn r(round: u64) -> DeltaOp {
    DeltaOp::V3(p3::DeltaOp::RoundStarted { round })
}

fn moved(entity: u64, points: Vec<Point>) -> DeltaOp {
    DeltaOp::Moved {
        entity: net(entity),
        points,
    }
}

fn position(entity: u64, position: Option<PositionFact>) -> DeltaOp {
    DeltaOp::Position {
        entity: net(entity),
        position,
    }
}

fn p64() -> Vec<Point> {
    (0..64).map(|i| pt(L - i, L, L)).collect()
}

fn p65() -> Vec<Point> {
    (0..65).map(|i| pt(L - i, L, L)).collect()
}

fn moved_bytes(entity: u64, points: &[Point]) -> Vec<u8> {
    let mut out = vec![0x15];
    out.extend(varint(entity));
    out.extend(varint(points.len() as u64));
    for p in points {
        out.extend(point_bytes(*p));
    }
    out
}

/// Size of one op as encoded, tag included.
fn op_len(op: &DeltaOp) -> usize {
    c4::encode_delta(&dfr(vec![op.clone()])).unwrap().len() - 5
}

/// Intent oracle: encodes to `ib(tail)`, decodes back, re-encodes equal.
fn check_intent(body: IntentBody, tail: &[u8]) {
    let frame = ifr(body);
    let bytes = ib(tail);
    assert_eq!(c4::encode_intent(&frame), Ok(bytes.clone()));
    assert_eq!(c4::decode_intent(&bytes), Ok(frame));
    let again = c4::encode_intent(&c4::decode_intent(&bytes).unwrap()).unwrap();
    assert_eq!(again, bytes);
}

/// Delta oracle for a one-op frame, both directions and re-encode.
fn check_op(op: DeltaOp, op_bytes: &[u8]) {
    let frame = dfr(vec![op]);
    let bytes = d1(op_bytes);
    assert_eq!(
        c4::encode_delta(&frame),
        Ok(bytes.clone()),
        "{op_bytes:02X?}"
    );
    assert_eq!(c4::decode_delta(&bytes), Ok(frame), "{op_bytes:02X?}");
    let again = c4::encode_delta(&c4::decode_delta(&bytes).unwrap()).unwrap();
    assert_eq!(again, bytes);
}

fn op_err(op_bytes: &[u8], err: CodecError) {
    assert_eq!(c4::decode_delta(&d1(op_bytes)), Err(err), "{op_bytes:02X?}");
}

fn intent_err(tail: &[u8], err: CodecError) {
    assert_eq!(c4::decode_intent(&ib(tail)), Err(err), "{tail:02X?}");
}

fn v1_end_turn() -> p1::IntentFrame {
    p1::IntentFrame {
        epoch: Z16,
        lane: 0,
        seq: 1,
        observed_tick: 7,
        actor: net(1),
        body: p1::IntentBody::EndTurn,
    }
}

fn receipt3(lane: u8, status: ReceiptStatus) -> p3::DeltaOp {
    p3::DeltaOp::Receipt {
        epoch: Z16,
        lane,
        seq: 1,
        processed_tick: 100,
        status,
    }
}

/// Every v3 op the contract names, with its v3 bytes where a literal is
/// pinned (T140 A4, A6, A7, B1, B2, B3). `None` means "differential only".
fn v3_cases() -> Vec<(p3::DeltaOp, Option<Vec<u8>>)> {
    let mut v: Vec<(p3::DeltaOp, Option<Vec<u8>>)> = Vec::new();
    let e = |tag: u8| Some(vec![tag, 1]);
    v.push((p3::DeltaOp::EntityEnter { entity: net(1) }, e(0)));
    v.push((p3::DeltaOp::EntityLeave { entity: net(1) }, e(1)));
    v.push((p3::DeltaOp::Spawned { entity: net(1) }, e(2)));
    v.push((p3::DeltaOp::Despawned { entity: net(1) }, e(3)));
    v.push((p3::DeltaOp::Died { entity: net(1) }, e(4)));
    v.push((
        p3::DeltaOp::Health {
            entity: net(1),
            health: 10,
            max_health: 10,
            dead: false,
        },
        None,
    ));
    v.push((
        p3::DeltaOp::Turn {
            active: None,
            round: 5,
        },
        Some(vec![6, 0, 5]),
    ));
    v.push((
        p3::DeltaOp::Turn {
            active: Some(net(1)),
            round: 5,
        },
        Some(vec![6, 1, 1, 5]),
    ));
    v.push((
        receipt3(0, ReceiptStatus::Applied),
        Some([&[7][..], &Z16, &[0, 1, 0x64, 0]].concat()),
    ));
    for code in 1u8..=30 {
        v.push((
            receipt3(
                0,
                ReceiptStatus::Rejected(RejectionCode::from_u8(code).unwrap()),
            ),
            Some([&[7][..], &Z16, &[0, 1, 0x64, 1, code]].concat()),
        ));
    }
    v.push((
        p3::DeltaOp::ActionResolved {
            actor: net(1),
            target: net(2),
            ability: ulid(1),
            outcome: "success".to_string(),
            damage: 7,
        },
        None,
    ));
    v.push((
        p3::DeltaOp::TurnStarted {
            actor: net(1),
            round: 3,
        },
        None,
    ));
    v.push((
        p3::DeltaOp::EncounterEnded {
            encounter: ulid(1),
            round: 3,
        },
        None,
    ));
    // A6
    v.push((p3::DeltaOp::RoundStarted { round: 3 }, Some(vec![0x0E, 3])));
    v.push((
        p3::DeltaOp::RoundStarted { round: u64::MAX },
        Some([&[0x0E][..], &MAX64].concat()),
    ));
    v.push((
        p3::DeltaOp::ModeChanged {
            mode: EncounterMode::RealTime,
            round: 2,
        },
        Some(vec![0x0F, 1, 2]),
    ));
    v.push((
        p3::DeltaOp::ModeChanged {
            mode: EncounterMode::TurnBased,
            round: 2,
        },
        Some(vec![0x0F, 0, 2]),
    ));
    v.push((
        p3::DeltaOp::OrderEnded { actor: net(2) },
        Some(vec![0x10, 2]),
    ));
    v.push((p3::DeltaOp::Joined { entity: net(2) }, Some(vec![0x11, 2])));
    v.push((p3::DeltaOp::Left { entity: net(2) }, Some(vec![0x12, 2])));
    // A7
    v.push((p3::DeltaOp::Clock { paused: None }, Some(vec![0x13, 0])));
    v.push((
        p3::DeltaOp::Clock { paused: Some(None) },
        Some(vec![0x13, 1, 0]),
    ));
    v.push((
        p3::DeltaOp::Clock {
            paused: Some(Some(PauseReason::Player)),
        },
        Some(vec![0x13, 1, 1, 0]),
    ));
    v.push((
        p3::DeltaOp::Clock {
            paused: Some(Some(PauseReason::Auto(AutoPauseTrigger::OrderEnded))),
        },
        Some(vec![0x13, 1, 1, 1, 3]),
    ));
    v.push((
        p3::DeltaOp::Clock {
            paused: Some(Some(PauseReason::Operator)),
        },
        Some(vec![0x13, 1, 1, 2]),
    ));
    v.push((
        p3::DeltaOp::Clock {
            paused: Some(Some(PauseReason::Restore)),
        },
        Some(vec![0x13, 1, 1, 3]),
    ));
    let rights = PauseRights {
        pause: true,
        unpause: true,
    };
    v.push((
        p3::DeltaOp::HostFacts(HostFacts {
            rights,
            auto_pause: None,
            turn_timer_ms: None,
        }),
        Some(vec![0x14, 1, 1, 0, 0]),
    ));
    v.push((
        p3::DeltaOp::HostFacts(HostFacts {
            rights,
            auto_pause: Some(AutoPauseToggles::from_bits(0x1F).unwrap()),
            turn_timer_ms: Some(Some(30000)),
        }),
        Some(vec![0x14, 1, 1, 1, 0x1F, 1, 1, 0xB0, 0xEA, 0x01]),
    ));
    v.push((
        p3::DeltaOp::HostFacts(HostFacts {
            rights,
            auto_pause: None,
            turn_timer_ms: Some(None),
        }),
        Some(vec![0x14, 1, 1, 0, 1, 0]),
    ));
    v.push((
        p3::DeltaOp::HostFacts(HostFacts {
            rights,
            auto_pause: None,
            turn_timer_ms: Some(Some(u32::MAX)),
        }),
        Some(vec![0x14, 1, 1, 0, 1, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F]),
    ));
    // B1
    v.push((
        p3::DeltaOp::Encounter(EncounterFacts::default()),
        Some(vec![0x0B, 0, 0, 0]),
    ));
    v.push((
        p3::DeltaOp::Encounter(EncounterFacts {
            mode: Some(EncounterMode::TurnBased),
            pending_mode: Some(Some(EncounterMode::RealTime)),
            clock: Some(Some(p3::ClockFact {
                round_ticks: 120,
                elapsed: 240,
            })),
        }),
        Some(vec![0x0B, 1, 0, 1, 1, 1, 1, 1, 0x78, 0xF0, 0x01]),
    ));
    // B2
    v.push((p3::DeltaOp::Timeline { entries: None }, Some(vec![0x0C, 0])));
    v.push((
        p3::DeltaOp::Timeline {
            entries: Some(vec![]),
        },
        Some(vec![0x0C, 1, 0]),
    ));
    v.push((
        p3::DeltaOp::Timeline {
            entries: Some(vec![
                TimelineEntry {
                    key: -1,
                    entity: net(2),
                },
                TimelineEntry {
                    key: -1,
                    entity: net(1),
                },
                TimelineEntry {
                    key: 5,
                    entity: net(3),
                },
            ]),
        },
        Some(vec![0x0C, 1, 3, 1, 2, 1, 1, 0x0A, 3]),
    ));
    // B3
    v.push((
        p3::DeltaOp::EntityFacts {
            entity: net(2),
            facts: FactSet::default(),
        },
        Some(vec![0x0D, 2, 0, 0, 0, 0, 0]),
    ));
    let mut full = vec![0x0D, 2];
    full.extend([1, 1]);
    full.extend(utext(1));
    full.extend([2, 3]);
    full.extend([1, 1]);
    full.extend(utext(2));
    full.push(4);
    full.extend([1, 1]);
    full.extend(utext(3));
    full.extend([1, 1]);
    full.extend([1, 2, 5, 0x6D, 0x69, 0x67, 0x68, 0x74, 0, 8]);
    full.extend([5, 0x72, 0x65, 0x61, 0x63, 0x68, 1, 0x80, 0x80, 0x0C]);
    full.extend([1, 1, 6, 0x75, 0x6E, 0x64, 0x65, 0x61, 0x64]);
    assert_eq!(full.len(), 123);
    v.push((
        p3::DeltaOp::EntityFacts {
            entity: net(2),
            facts: FactSet {
                pools: Some(vec![PoolFact {
                    pool: ulid(1),
                    current: 2,
                    max: 3,
                }]),
                effects: Some(vec![EffectFact {
                    effect: ulid(2),
                    expires_round: 4,
                }]),
                order: Some(Some(OrderFact {
                    ability: ulid(3),
                    target: net(1),
                    repeat: true,
                })),
                stats: Some(vec![
                    StatFact {
                        name: "might".to_string(),
                        value: StatValue::Int(4),
                    },
                    StatFact {
                        name: "reach".to_string(),
                        value: StatValue::Fixed(98304),
                    },
                ]),
                tags: Some(vec!["undead".to_string()]),
            },
        },
        Some(full),
    ));
    v
}

/// (name, lane, seq bytes, actor bytes, body tail, expected error).
type IntentRefusal = (&'static str, u8, Vec<u8>, Vec<u8>, Vec<u8>, CodecError);

/// `v3` bytes with byte 0 set to 4.
fn as_v4(mut bytes: Vec<u8>) -> Vec<u8> {
    assert_eq!(bytes[0], 3);
    bytes[0] = 4;
    bytes
}

fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state >> 33
}

// ---------------------------------------------------------------------------
// W1
// ---------------------------------------------------------------------------

#[test]
fn v4_version_gate() {
    let frame = ifr(IntentBody::EndTurn);
    let want = [&[4][..], &Z16, &[0, 1, 7, 1, 1]].concat();
    assert_eq!(want.len(), 22);
    let b4 = c4::encode_intent(&frame).unwrap();
    assert_eq!(b4, want);
    assert_eq!(p4::PROTOCOL_VERSION, 4);
    assert_eq!(c4::decode_intent(&b4), Ok(frame));

    let b1 = v1::encode_intent(&v1_end_turn()).unwrap();
    let b2 = v2::encode_intent(&v1_end_turn()).unwrap();
    let b3 = c3::encode_intent(&ifr3(p3::IntentBody::EndTurn)).unwrap();
    assert_eq!((b1[0], b2[0], b3[0]), (1, 2, 3));
    // The v3 bytes are the v4 bytes with byte 0 changed from 3 to 4.
    assert_eq!(as_v4(b3.clone()), b4);

    // v4 decoders refuse every other first byte; the control decodes.
    for bad in [&b1[..], &b2[..], &b3[..]] {
        assert_eq!(c4::decode_intent(bad), Err(CodecError::UnsupportedVersion));
    }
    for first in 0u8..=255 {
        if first == 4 {
            continue;
        }
        let one = [first];
        assert_eq!(
            c4::decode_intent(&one),
            Err(CodecError::UnsupportedVersion),
            "intent first byte {first:#04X}"
        );
        assert_eq!(
            c4::decode_delta(&one),
            Err(CodecError::UnsupportedVersion),
            "delta first byte {first:#04X}"
        );
        let mut full = b4.clone();
        full[0] = first;
        assert_eq!(
            c4::decode_intent(&full),
            Err(CodecError::UnsupportedVersion)
        );
        let empty = [first, 0, 0x64, 1, 0];
        assert_eq!(
            c4::decode_delta(&empty),
            Err(CodecError::UnsupportedVersion)
        );
    }
    for named in [0x81u8, 0x82, 0x91, 0xA1, 1, 2, 3] {
        assert_eq!(
            c4::decode_intent(&[named]),
            Err(CodecError::UnsupportedVersion)
        );
        assert_eq!(
            c4::decode_delta(&[named]),
            Err(CodecError::UnsupportedVersion)
        );
    }
    assert_eq!(c4::decode_intent(&[]), Err(CodecError::Malformed));
    assert_eq!(c4::decode_delta(&[]), Err(CodecError::Malformed));
    // A bare version byte 4 is a truncation, not a version error.
    assert_eq!(c4::decode_intent(&[4]), Err(CodecError::Malformed));
    assert_eq!(c4::decode_delta(&[4]), Err(CodecError::Malformed));

    // The old codecs refuse v4 bytes; their controls still decode.
    assert_eq!(v1::decode_intent(&b4), Err(CodecError::UnsupportedVersion));
    assert_eq!(v2::decode_intent(&b4), Err(CodecError::UnsupportedVersion));
    assert_eq!(c3::decode_intent(&b4), Err(CodecError::UnsupportedVersion));
    assert_eq!(v1::decode_intent(&b1), Ok(v1_end_turn()));
    assert!(c3::decode_intent(&b3).is_ok());
    let empty4 = [4, 0, 0x64, 1, 0];
    assert_eq!(
        v1::decode_delta(&empty4),
        Err(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        v2::decode_delta(&empty4),
        Err(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        c3::decode_delta(&empty4),
        Err(CodecError::UnsupportedVersion)
    );
    assert_eq!(c4::decode_delta(&empty4), Ok(dfr(vec![])));
    // v4 decoders refuse the old empty frames.
    for first in [1u8, 2, 3] {
        let old = [first, 0, 0x64, 1, 0];
        assert_eq!(c4::decode_delta(&old), Err(CodecError::UnsupportedVersion));
    }

    // Caps run before the version check.
    for first in [3u8, 4, 0x81, 0xA1] {
        let mut big = vec![0u8; 4097];
        big[0] = first;
        assert_eq!(c4::decode_intent(&big), Err(CodecError::FrameTooLarge));
        let mut big = vec![0u8; 65537];
        big[0] = first;
        assert_eq!(c4::decode_delta(&big), Err(CodecError::FrameTooLarge));
    }
    // A 4,096-byte v4 intent (EndTurn padded with 00) is a trailing-byte
    // failure, not a cap failure.
    let mut padded = b4.clone();
    padded.resize(4096, 0);
    assert_eq!(c4::decode_intent(&padded), Err(CodecError::Malformed));
    // The same for a 65,536-byte delta.
    let mut padded = empty4.to_vec();
    padded.resize(65536, 0);
    assert_eq!(c4::decode_delta(&padded), Err(CodecError::Malformed));
    // The intent cap is 4,096, not the delta's: a 4,097-byte delta with a
    // valid-prefix is not FrameTooLarge.
    let mut mid = empty4.to_vec();
    mid.resize(4097, 0);
    assert_eq!(c4::decode_delta(&mid), Err(CodecError::Malformed));
}

// ---------------------------------------------------------------------------
// W2 (and W-3)
// ---------------------------------------------------------------------------

#[test]
fn v4_intent_v3_bodies() {
    let bodies = [
        p3::IntentBody::EndTurn,
        p3::IntentBody::Stop,
        p3::IntentBody::Pause,
        p3::IntentBody::Unpause,
        p3::IntentBody::RequestMode {
            mode: EncounterMode::TurnBased,
        },
        p3::IntentBody::RequestMode {
            mode: EncounterMode::RealTime,
        },
        p3::IntentBody::DeclareAction {
            ability: ulid(1),
            target: net(2),
        },
    ];
    for body in bodies {
        for seq in [1u64, 2, 300, u64::MAX - 1] {
            let mut f3 = ifr3(body);
            f3.seq = seq;
            let b3 = c3::encode_intent(&f3).unwrap();
            let f4 = conv_intent(f3);
            let b4 = c4::encode_intent(&f4).unwrap();
            assert_eq!(b4, as_v4(b3), "{body:?} seq {seq}");
            assert_eq!(c4::decode_intent(&b4), Ok(f4), "{body:?} seq {seq}");
        }
    }

    // Literals (T140 A2 with byte 0 = 4).
    check_intent(IntentBody::Stop, &[2]);
    check_intent(IntentBody::Pause, &[3]);
    check_intent(IntentBody::Unpause, &[4]);
    check_intent(
        IntentBody::RequestMode {
            mode: EncounterMode::RealTime,
        },
        &[5, 1],
    );
    check_intent(
        IntentBody::RequestMode {
            mode: EncounterMode::TurnBased,
        },
        &[5, 0],
    );
    let mut declare_tail = vec![0, 0x1A];
    declare_tail.extend([0x30u8; 25]);
    declare_tail.extend([0x31, 2]);
    let declare = ib(&declare_tail);
    assert_eq!(declare.len(), 50);
    check_intent(
        IntentBody::DeclareAction {
            ability: ulid(1),
            target: net(2),
        },
        &declare_tail[..],
    );
    // `From` keeps the variant and fields.
    assert_eq!(
        IntentBody::from(p3::IntentBody::DeclareAction {
            ability: ulid(1),
            target: net(2)
        }),
        IntentBody::DeclareAction {
            ability: ulid(1),
            target: net(2)
        }
    );
    assert_eq!(
        IntentBody::from(p3::IntentBody::RequestMode {
            mode: EncounterMode::RealTime
        }),
        IntentBody::RequestMode {
            mode: EncounterMode::RealTime
        }
    );
    assert_eq!(
        IntentBody::from(p3::IntentBody::EndTurn),
        IntentBody::EndTurn
    );

    // v3 refusals: the v3 bytes with byte 0 = 4 give v4 the error v3 gives
    // the 3 form. (name, lane, seq, actor, tail, v3 error, v4 error)
    let id1 = [1u8];
    let seq1 = [1u8];
    let target0_tail = [&[0u8, 0x1A][..], &utext(1)[1..], &[0]].concat();
    let cases: Vec<IntentRefusal> = vec![
        (
            "body tag 07",
            0,
            seq1.to_vec(),
            id1.to_vec(),
            vec![7],
            CodecError::UnknownMessage,
        ),
        (
            "body tag FF",
            0,
            seq1.to_vec(),
            id1.to_vec(),
            vec![0xFF],
            CodecError::UnknownMessage,
        ),
        (
            "mode 02",
            0,
            seq1.to_vec(),
            id1.to_vec(),
            vec![5, 2],
            CodecError::UnknownMessage,
        ),
        (
            "mode FF",
            0,
            seq1.to_vec(),
            id1.to_vec(),
            vec![5, 0xFF],
            CodecError::UnknownMessage,
        ),
        (
            "lane 1",
            1,
            seq1.to_vec(),
            id1.to_vec(),
            vec![1],
            CodecError::WrongDirection,
        ),
        (
            "seq 0",
            0,
            vec![0],
            id1.to_vec(),
            vec![1],
            CodecError::Malformed,
        ),
        (
            "seq u64::MAX",
            0,
            MAX64.to_vec(),
            id1.to_vec(),
            vec![1],
            CodecError::Malformed,
        ),
        (
            "actor 0",
            0,
            seq1.to_vec(),
            vec![0],
            vec![1],
            CodecError::Malformed,
        ),
        (
            "DeclareAction target 0",
            0,
            seq1.to_vec(),
            id1.to_vec(),
            target0_tail,
            CodecError::Malformed,
        ),
        (
            "ULID length 257, no body",
            0,
            seq1.to_vec(),
            id1.to_vec(),
            vec![0, 0x81, 0x02],
            CodecError::LimitExceeded,
        ),
        (
            "one trailing byte",
            0,
            seq1.to_vec(),
            id1.to_vec(),
            vec![1, 0],
            CodecError::Malformed,
        ),
        (
            "RequestMode missing mode",
            0,
            seq1.to_vec(),
            id1.to_vec(),
            vec![5],
            CodecError::Malformed,
        ),
    ];
    for (name, lane, seq, actor, tail, err) in cases {
        let s3 = iraw(3, lane, &seq, &actor, &tail);
        let s4 = iraw(4, lane, &seq, &actor, &tail);
        assert_eq!(c3::decode_intent(&s3), Err(err), "v3 oracle {name}");
        assert_eq!(c4::decode_intent(&s4), Err(err), "v4 {name}");
    }
    // Controls beside them: SEQ_LAST, mode 01, and a good body decode Ok.
    assert!(c4::decode_intent(&iraw(4, 0, &LAST64, &[1], &[1])).is_ok());
    assert!(c4::decode_intent(&iraw(4, 0, &[1], &[1], &[5, 1])).is_ok());
    assert!(c4::decode_intent(&iraw(4, 0, &[1], &[1], &[0, 0x1A])).is_err());
    let mut declare_ok = vec![0, 0x1A];
    declare_ok.extend([0x30u8; 25]);
    declare_ok.extend([0x31, 2]);
    assert!(c4::decode_intent(&iraw(4, 0, &[1], &[1], &declare_ok)).is_ok());
    // ULID text of 256 bytes of 41 is Malformed (not LimitExceeded).
    let mut long = vec![0, 0x80, 0x02];
    long.extend([0x41u8; 256]);
    long.push(1);
    assert_eq!(
        c4::decode_intent(&iraw(4, 0, &[1], &[1], &long)),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        c3::decode_intent(&iraw(3, 0, &[1], &[1], &long)),
        Err(CodecError::Malformed)
    );

    // Two faults, first wins (v4 bytes).
    assert_eq!(
        c4::decode_intent(&iraw(4, 1, &[0], &[1], &[1])),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        c4::decode_intent(&iraw(4, 1, &[1], &[0], &[1])),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        c4::decode_intent(&iraw(4, 0, &[0], &[1], &[0xFF])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        c4::decode_intent(&iraw(4, 0, &[1], &[0], &[0xFF])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        c4::decode_intent(&iraw(4, 0, &[1], &[1], &[0xFF, 0])),
        Err(CodecError::UnknownMessage)
    );
    assert_eq!(
        c4::decode_intent(&iraw(4, 0, &[1], &[1], &[5, 2, 0])),
        Err(CodecError::UnknownMessage)
    );
    // seq beats actor, and actor beats body tag (both Malformed vs Unknown).
    assert_eq!(
        c4::decode_intent(&iraw(4, 0, &[0], &[0], &[1])),
        Err(CodecError::Malformed)
    );
    // Wrong lane beats an unknown body tag.
    assert_eq!(
        c4::decode_intent(&iraw(4, 1, &[1], &[1], &[0xFF])),
        Err(CodecError::WrongDirection)
    );
    // Encode of lane 1 with seq 0 is WrongDirection; each alone differs.
    let mut bad = ifr(IntentBody::EndTurn);
    bad.lane = 1;
    bad.seq = 0;
    assert_eq!(c4::encode_intent(&bad), Err(CodecError::WrongDirection));
    bad.lane = 0;
    assert_eq!(c4::encode_intent(&bad), Err(CodecError::Malformed));
    bad.seq = u64::MAX;
    assert_eq!(c4::encode_intent(&bad), Err(CodecError::Malformed));
    bad.seq = u64::MAX - 1;
    assert!(c4::encode_intent(&bad).is_ok());

    // observed_tick is any u64.
    let mut tick_max = ib(&[1]);
    tick_max.splice(19..20, MAX64);
    assert_eq!(
        c4::decode_intent(&tick_max).map(|f| f.observed_tick),
        Ok(u64::MAX)
    );

    // W-3 decode half: mutation differential against v3. Every one-byte
    // mutation, truncation and extension of each v3 intent encoding, with
    // byte 0 set to 4, decodes as v3 does (converted), except where v3
    // reports an unknown tag and the string holds a 6 that v4 may read as
    // `DeclareMove`.
    let mut checked = 0u32;
    for body in bodies {
        let base = c3::encode_intent(&ifr3(body)).unwrap();
        let mut variants: Vec<Vec<u8>> = vec![base.clone()];
        for pos in 1..base.len() {
            for value in 0..=255u8 {
                let mut s = base.clone();
                s[pos] = value;
                variants.push(s);
            }
            variants.push(base[..pos].to_vec());
        }
        let mut ext = base.clone();
        ext.push(0);
        variants.push(ext);
        for s3 in variants {
            let r3 = c3::decode_intent(&s3);
            if r3 == Err(CodecError::UnknownMessage) && s3[1..].contains(&6) {
                continue;
            }
            let mut s4 = s3.clone();
            s4[0] = 4;
            assert_eq!(
                c4::decode_intent(&s4),
                r3.map(conv_intent),
                "{body:?} {s4:02X?}"
            );
            checked += 1;
        }
    }
    assert!(checked > 10_000);
}

// ---------------------------------------------------------------------------
// W3
// ---------------------------------------------------------------------------

#[test]
fn declare_move_literals_and_bounds() {
    assert_eq!(p4::INTENT_TAG_DECLARE_MOVE, 6);
    let frame = ifr(IntentBody::DeclareMove { destination: b() });
    let want = [
        &[4][..],
        &Z16,
        &[0, 1, 7, 1, 6, 0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F],
    ]
    .concat();
    assert_eq!(want.len(), 29);
    assert_eq!(c4::encode_intent(&frame), Ok(want.clone()));
    assert_eq!(c4::decode_intent(&want), Ok(frame));

    // Coordinates at the limit, in each of x, y and z.
    for pos in 0..3 {
        for (bytes, raw) in [(BL, L), (BNL, -L)] {
            let mut tail = vec![6];
            let mut raws = [0i32; 3];
            raws[pos] = raw;
            for slot in 0..3 {
                if slot == pos {
                    tail.extend(bytes);
                } else {
                    tail.push(0);
                }
            }
            let want = IntentBody::DeclareMove {
                destination: Point::new(raws).unwrap(),
            };
            check_intent(want, &tail);
        }
        // Refusals, each with the other two components valid (00).
        for bad in [BL1, BNL1, BMIN, BOVER] {
            let mut tail = vec![6];
            for slot in 0..3 {
                if slot == pos {
                    tail.extend(bad);
                } else {
                    tail.push(0);
                }
            }
            intent_err(&tail, CodecError::Malformed);
        }
    }
    // A bad x is refused even though y and z follow validly, and a bad
    // component is refused when nothing else follows.
    intent_err(&[6, 0x82, 0x80, 0x80, 0x80, 0x04], CodecError::Malformed);
    // Missing z, and one trailing byte.
    intent_err(&[6, 0, 0], CodecError::Malformed);
    intent_err(&[6, 0, 0, 0, 0], CodecError::Malformed);
    intent_err(&[6], CodecError::Malformed);
    // Components in x, y, z order: raw [1, 2, 3] -> 02 04 06.
    check_intent(
        IntentBody::DeclareMove {
            destination: pt(1, 2, 3),
        },
        &[6, 2, 4, 6],
    );
    // Truncation anywhere is Malformed (the 29-byte frame, every prefix).
    for cut in 1..want.len() {
        assert_eq!(
            c4::decode_intent(&want[..cut]),
            Err(CodecError::Malformed),
            "prefix {cut}"
        );
    }
    assert_eq!(c4::decode_intent(&[]), Err(CodecError::Malformed));

    // Point::new / raw.
    assert!(Point::new([L, -L, 0]).is_some());
    assert_eq!(Point::new([L, -L, 0]).unwrap().raw(), [L, -L, 0]);
    assert!(Point::new([L + 1, 0, 0]).is_none());
    assert!(Point::new([0, -L - 1, 0]).is_none());
    assert!(Point::new([0, 0, i32::MIN]).is_none());
    assert!(Point::new([i32::MAX, 0, 0]).is_none());
    assert!(Point::new([0, 0, 0]).is_some());
    assert!(Point::new([-L, -L, -L]).is_some());
    assert!(Point::new([L, L, L]).is_some());
    assert_eq!(b().raw(), [65536, 0, -131072]);
    assert_eq!(p4::COORD_LIMIT_RAW, 536_870_912);

    // Encode refuses lane 1 with WrongDirection and seq 0 with Malformed.
    let mut bad = ifr(IntentBody::DeclareMove { destination: b() });
    bad.lane = 1;
    assert_eq!(c4::encode_intent(&bad), Err(CodecError::WrongDirection));
    bad.lane = 0;
    bad.seq = 0;
    assert_eq!(c4::encode_intent(&bad), Err(CodecError::Malformed));
    bad.lane = 1;
    assert_eq!(c4::encode_intent(&bad), Err(CodecError::WrongDirection));
    bad.lane = 0;
    bad.seq = 1;
    assert!(c4::encode_intent(&bad).is_ok());
    // Decode-order for the new body: lane and seq before the body.
    assert_eq!(
        c4::decode_intent(&iraw(4, 1, &[0], &[1], &[6, 0, 0, 0])),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        c4::decode_intent(&iraw(4, 0, &[0], &[1], &[6, 0x82, 0x80, 0x80, 0x80, 0x04])),
        Err(CodecError::Malformed)
    );
    // The v3 decoders do not know tag 6 in a v3 frame.
    let v3_tag6 = iraw(3, 0, &[1], &[1], &[6, 0, 0, 0]);
    assert_eq!(c3::decode_intent(&v3_tag6), Err(CodecError::UnknownMessage));
    assert!(c4::decode_intent(&iraw(4, 0, &[1], &[1], &[6, 0, 0, 0])).is_ok());
}

// ---------------------------------------------------------------------------
// W4 (and W-8)
// ---------------------------------------------------------------------------

#[test]
fn v4_v3_ops_identical() {
    let cases = v3_cases();
    for (op, literal) in &cases {
        let f3 = dfr3(vec![op.clone()]);
        let b3 = c3::encode_delta(&f3).unwrap();
        let f4 = dfr(vec![DeltaOp::V3(op.clone())]);
        let b4 = c4::encode_delta(&f4).unwrap();
        assert_eq!(b4, as_v4(b3.clone()), "{op:?}");
        assert_eq!(c4::decode_delta(&b4), Ok(f4.clone()), "{op:?}");
        assert_eq!(
            c4::encode_delta(&c4::decode_delta(&b4).unwrap()).unwrap(),
            b4
        );
        assert_eq!(DeltaOp::from(op.clone()), DeltaOp::V3(op.clone()));
        if let Some(lit) = literal {
            assert_eq!(b4, d1(lit), "literal {op:?}");
        }
    }
    // Multi-op frames too, with a Moved and a Position between v3 ops.
    let all: Vec<p3::DeltaOp> = cases.iter().map(|(op, _)| op.clone()).collect();
    let f3 = dfr3(all.clone());
    let b3 = c3::encode_delta(&f3).unwrap();
    assert_eq!(
        c4::encode_delta(&wrap_frame(f3.clone())).unwrap(),
        as_v4(b3.clone())
    );
    assert_eq!(c4::decode_delta(&as_v4(b3)), Ok(wrap_frame(f3)));
    let mixed: Vec<DeltaOp> = all
        .iter()
        .cloned()
        .flat_map(|op| {
            [
                DeltaOp::V3(op),
                moved(2, vec![a(), pt(1, 0, 1)]),
                position(3, None),
            ]
        })
        .take(256)
        .collect();
    let bytes = c4::encode_delta(&dfr(mixed.clone())).unwrap();
    assert_eq!(c4::decode_delta(&bytes), Ok(dfr(mixed)));

    // Literal: Turn { None, 5 }.
    assert_eq!(
        c4::encode_delta(&dfr(vec![DeltaOp::V3(p3::DeltaOp::Turn {
            active: None,
            round: 5
        })])),
        Ok(vec![4, 0, 0x64, 1, 1, 6, 0, 5])
    );

    // The named v3 refusals with byte 0 = 4 give the v3 error (v3 oracle
    // beside it), each with its accepted control.
    let mut receipt_code_1f = vec![0x07];
    receipt_code_1f.extend(Z16);
    receipt_code_1f.extend([0, 1, 0x64, 1, 0x1F]);
    let mut receipt_lane1 = vec![0x07];
    receipt_lane1.extend(Z16);
    receipt_lane1.extend([1, 1, 0x64, 0]);
    let mut receipt_ok = vec![0x07];
    receipt_ok.extend(Z16);
    receipt_ok.extend([0, 1, 0x64, 1, 0x1E]);
    let refusals: Vec<(&str, Vec<u8>, CodecError, Vec<u8>)> = vec![
        (
            "receipt code 1F",
            receipt_code_1f,
            CodecError::UnknownMessage,
            receipt_ok,
        ),
        (
            "receipt lane 1",
            receipt_lane1,
            CodecError::WrongDirection,
            {
                let mut ok = vec![0x07];
                ok.extend(Z16);
                ok.extend([0, 1, 0x64, 0]);
                ok
            },
        ),
        (
            "encounter mode 02",
            vec![0x0B, 1, 2, 0, 0],
            CodecError::UnknownMessage,
            vec![0x0B, 1, 0, 0, 0],
        ),
        (
            "encounter pending 02",
            vec![0x0B, 0, 1, 1, 2, 0],
            CodecError::UnknownMessage,
            vec![0x0B, 0, 1, 1, 1, 0],
        ),
        (
            "timeline count 1025",
            vec![0x0C, 1, 0x81, 0x08],
            CodecError::LimitExceeded,
            vec![0x0C, 1, 0],
        ),
        (
            "stats count 33",
            vec![0x0D, 2, 0, 0, 0, 1, 0x21],
            CodecError::LimitExceeded,
            vec![0x0D, 2, 0, 0, 0, 1, 0, 0],
        ),
        (
            "clock reason 04",
            vec![0x13, 1, 1, 4],
            CodecError::UnknownMessage,
            vec![0x13, 1, 1, 3],
        ),
        (
            "host facts bits 20",
            vec![0x14, 1, 1, 1, 0x20, 0],
            CodecError::Malformed,
            vec![0x14, 1, 1, 1, 0x1F, 0],
        ),
        (
            "clock paused 02",
            vec![0x13, 2],
            CodecError::Malformed,
            vec![0x13, 1, 0],
        ),
        (
            "round started truncated",
            vec![0x0E],
            CodecError::Malformed,
            vec![0x0E, 1],
        ),
    ];
    for (name, bad, err, good) in refusals {
        let s3 = [&[3, 0, 0x64, 1, 1][..], &bad].concat();
        assert_eq!(c3::decode_delta(&s3), Err(err), "v3 oracle {name}");
        op_err(&bad, err);
        assert!(c4::decode_delta(&d1(&good)).is_ok(), "control {name}");
    }

    // Mutation differential (W-8 decode half): every byte mutation,
    // truncation and extension of each one-op v3 frame, byte 0 set to 4,
    // decodes as v3 does with ops wrapped, except where v3 reports an
    // unknown tag and the string holds a 21 or 22 that v4 reads as an op.
    let mut checked = 0u32;
    let mut seeds: Vec<Vec<u8>> = cases
        .iter()
        .filter(|(op, _)| {
            !matches!(
                op,
                p3::DeltaOp::Receipt {
                    status: ReceiptStatus::Rejected(c),
                    ..
                } if !matches!(c.as_u8(), 1 | 17 | 22 | 30)
            )
        })
        .map(|(op, _)| c3::encode_delta(&dfr3(vec![op.clone()])).unwrap())
        .collect();
    seeds.push(c3::encode_delta(&dfr3(all)).unwrap());
    seeds.push(c3::encode_delta(&dfr3(vec![])).unwrap());
    for base in seeds {
        let mut variants: Vec<Vec<u8>> = vec![base.clone()];
        for pos in 1..base.len() {
            for value in 0..=255u8 {
                let mut s = base.clone();
                s[pos] = value;
                variants.push(s);
            }
            variants.push(base[..pos].to_vec());
        }
        let mut ext = base.clone();
        ext.push(0);
        variants.push(ext);
        // Large frames: only a deterministic sample of positions.
        if base.len() > 400 {
            let mut state = 7u64;
            variants.truncate(1);
            for _ in 0..20_000 {
                let mut s = base.clone();
                let pos = 1 + (lcg(&mut state) as usize) % (s.len() - 1);
                s[pos] = (lcg(&mut state) & 0xFF) as u8;
                variants.push(s);
            }
        }
        for s3 in variants {
            let r3 = c3::decode_delta(&s3);
            if r3 == Err(CodecError::UnknownMessage) && s3[1..].iter().any(|b| matches!(b, 21 | 22))
            {
                continue;
            }
            let mut s4 = s3.clone();
            s4[0] = 4;
            assert_eq!(c4::decode_delta(&s4), r3.map(wrap_frame), "{s4:02X?}");
            checked += 1;
        }
    }
    assert!(checked > 20_000);

    // W-8 encode half: errors are the same too.
    let mut lane1 = dfr3(vec![p3::DeltaOp::RoundStarted { round: 1 }]);
    lane1.lane = 1;
    assert_eq!(c3::encode_delta(&lane1), Err(CodecError::WrongDirection));
    assert_eq!(
        c4::encode_delta(&wrap_frame(lane1)),
        Err(CodecError::WrongDirection)
    );
    let bogus = p3::DeltaOp::ActionResolved {
        actor: net(1),
        target: net(2),
        ability: ulid(1),
        outcome: "bogus".to_string(),
        damage: 7,
    };
    let f3 = dfr3(vec![bogus]);
    assert_eq!(c3::encode_delta(&f3), Err(CodecError::Malformed));
    assert_eq!(
        c4::encode_delta(&wrap_frame(f3)),
        Err(CodecError::Malformed)
    );
    // A v3 outcome check inside v4: "custom:255" accepted, "custom:256" and
    // a 257-byte outcome refused as v3 does.
    for (outcome, want) in [
        ("custom:255".to_string(), Ok(())),
        ("custom:256".to_string(), Err(CodecError::Malformed)),
        ("a".repeat(257), Err(CodecError::LimitExceeded)),
        (
            format!("custom:{}", "9".repeat(250)),
            Err(CodecError::LimitExceeded),
        ),
    ] {
        let op = p3::DeltaOp::ActionResolved {
            actor: net(1),
            target: net(2),
            ability: ulid(1),
            outcome,
            damage: 7,
        };
        let got3 = c3::encode_delta(&dfr3(vec![op.clone()])).map(|_| ());
        let got4 = c4::encode_delta(&dfr(vec![DeltaOp::V3(op)])).map(|_| ());
        assert_eq!(got3, got4);
        if want == Ok(()) || want == Err(CodecError::Malformed) {
            assert_eq!(got4, want);
        }
    }
}

// ---------------------------------------------------------------------------
// W5
// ---------------------------------------------------------------------------

#[test]
fn moved_literals_and_bounds() {
    assert_eq!(p4::DELTA_TAG_MOVED, 21);
    assert_eq!(p4::MAX_MOVED_POINTS, 64);
    // The 11-byte literal.
    let lit = [0x15, 0x02, 0x02, 0, 0, 0, 0x80, 0x80, 0x08, 0, 0];
    check_op(moved(2, vec![a(), pt(65536, 0, 0)]), &lit);
    assert_eq!(moved_bytes(2, &[a(), pt(65536, 0, 0)]), lit);
    assert_eq!(op_len(&moved(2, vec![a(), pt(65536, 0, 0)])), 11);

    // P64 is accepted; with entity u64::MAX the op is 972 bytes.
    let p = p64();
    assert_eq!(p.len(), 64);
    check_op(moved(2, p.clone()), &moved_bytes(2, &p));
    let big = moved(u64::MAX, p.clone());
    assert_eq!(op_len(&big), 972);
    check_op(big, &moved_bytes(u64::MAX, &p));
    // Each P64 point is 15 bytes.
    assert!(p.iter().all(|q| point_bytes(*q).len() == 15));
    // A point at the limit in x, y and z inside a Moved.
    check_op(
        moved(2, vec![a(), pt(L, -L, L)]),
        &[&[0x15, 2, 2, 0, 0, 0][..], &BL, &BNL, &BL].concat(),
    );

    // Count 65 with no point following is LimitExceeded; its controls: count
    // 64 with 64 points is accepted (above).
    op_err(&[0x15, 2, 0x41], CodecError::LimitExceeded);
    // Count 0 and 1 are Malformed (before any point is read, and with one).
    op_err(&[0x15, 2, 0x00], CodecError::Malformed);
    op_err(&[0x15, 2, 0x01], CodecError::Malformed);
    op_err(&[0x15, 2, 0x01, 0, 0, 0], CodecError::Malformed);
    // The two-op frame with a count-1 Moved then RoundStarted { 3 }.
    assert_eq!(
        c4::decode_delta(&[4, 0, 0x64, 1, 2, 0x15, 2, 1, 0, 0, 0, 0x0E, 3]),
        Err(CodecError::Malformed)
    );
    // Its control with count 2 decodes.
    assert_eq!(
        c4::decode_delta(&[4, 0, 0x64, 1, 2, 0x15, 2, 2, 0, 0, 0, 1, 0, 0, 0x0E, 3]),
        Ok(dfr(vec![moved(2, vec![a(), pt(-1, 0, 0)]), r(3)]))
    );
    // Count varint: overflow is Malformed, u64::MAX is LimitExceeded.
    let mut ovf = vec![0x15, 2];
    ovf.extend([0xFF; 9]);
    ovf.push(0x02);
    op_err(&ovf, CodecError::Malformed);
    let mut max = vec![0x15, 2];
    max.extend(MAX64);
    op_err(&max, CodecError::LimitExceeded);
    // Count 65 with 65 valid points is still LimitExceeded.
    op_err(&moved_bytes(2, &p65()), CodecError::LimitExceeded);

    // Zero horizontal segment: [A, P(0, 65536, 0)] is Malformed, its control
    // [A, P(0, 0, 1)] is accepted, and [A, P(1,0,0), P(1,5,0)] is Malformed.
    op_err(
        &[0x15, 2, 2, 0, 0, 0, 0, 0x80, 0x80, 0x08, 0],
        CodecError::Malformed,
    );
    check_op(
        moved(2, vec![a(), pt(0, 0, 1)]),
        &[0x15, 2, 2, 0, 0, 0, 0, 0, 2],
    );
    assert_eq!(
        c4::encode_delta(&dfr(vec![moved(2, vec![a(), pt(0, 65536, 0)])])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        c4::encode_delta(&dfr(vec![moved(2, vec![a(), pt(1, 0, 0), pt(1, 5, 0)])])),
        Err(CodecError::Malformed)
    );
    op_err(
        &[0x15, 2, 3, 0, 0, 0, 2, 0, 0, 2, 0x0A, 0],
        CodecError::Malformed,
    );
    // Control: only consecutive points matter.
    check_op(
        moved(2, vec![a(), pt(1, 0, 0), a()]),
        &[0x15, 2, 3, 0, 0, 0, 2, 0, 0, 0, 0, 0],
    );
    // Equal points, first two and later.
    op_err(&[0x15, 2, 2, 2, 2, 2, 2, 2, 2], CodecError::Malformed);
    op_err(
        &[0x15, 2, 3, 0, 0, 0, 2, 0, 0, 2, 0, 0],
        CodecError::Malformed,
    );
    // Entity 0 is Malformed; entity is checked before the count.
    op_err(&[0x15, 0, 2, 0, 0, 0, 2, 0, 0], CodecError::Malformed);
    op_err(&[0x15, 0, 0x41], CodecError::Malformed);
    // A second point with x = L + 1 is Malformed, control x = L accepted; so
    // is a bad first point.
    let mut bad_x = vec![0x15, 2, 2, 0, 0, 0];
    bad_x.extend(BL1);
    bad_x.extend([0, 0]);
    op_err(&bad_x, CodecError::Malformed);
    let mut ok_x = vec![0x15, 2, 2, 0, 0, 0];
    ok_x.extend(BL);
    ok_x.extend([0, 0]);
    assert!(c4::decode_delta(&d1(&ok_x)).is_ok());
    let mut bad_first = vec![0x15, 2, 2];
    bad_first.extend(BNL1);
    bad_first.extend([0, 0, 0, 0, 1]);
    op_err(&bad_first, CodecError::Malformed);
    // Point y and z bounds inside a Moved.
    for bad in [BL1, BNL1, BMIN, BOVER] {
        let mut y = vec![0x15, 2, 2, 0, 0, 0, 1];
        y.extend(bad);
        y.push(0);
        op_err(&y, CodecError::Malformed);
        let mut z = vec![0x15, 2, 2, 0, 0, 0, 1, 0];
        z.extend(bad);
        op_err(&z, CodecError::Malformed);
    }
    // y may differ with equal x and z only when... it may not (zero segment),
    // but a differing x or z is fine.
    check_op(
        moved(2, vec![a(), pt(0, 0, -1)]),
        &[0x15, 2, 2, 0, 0, 0, 0, 0, 1],
    );

    // check_points: [] and [A] are Malformed, P65 LimitExceeded,
    // [A, P(0, 65536, 0)] Malformed, P64 Ok; each equals encode_delta's error.
    let table: Vec<(Vec<Point>, Result<(), CodecError>)> = vec![
        (vec![], Err(CodecError::Malformed)),
        (vec![a()], Err(CodecError::Malformed)),
        (p65(), Err(CodecError::LimitExceeded)),
        (vec![a(), pt(0, 65536, 0)], Err(CodecError::Malformed)),
        (vec![a(), a()], Err(CodecError::Malformed)),
        (p64(), Ok(())),
        (vec![a(), pt(1, 0, 0)], Ok(())),
        // Count beats the zero-segment rule.
        (vec![a(); 65], Err(CodecError::LimitExceeded)),
        // Count under 2 beats nothing else; a zero segment is Malformed.
        (vec![a(); 64], Err(CodecError::Malformed)),
    ];
    for (points, want) in table {
        assert_eq!(p4::check_points(&points), want, "{} points", points.len());
        let got = c4::encode_delta(&dfr(vec![moved(2, points.clone())])).map(|_| ());
        assert_eq!(got, want, "encode {} points", points.len());
    }

    // Encode order inside a frame: lane first, then count, then ops.
    let mut f = dfr(vec![moved(2, vec![a()])]);
    f.lane = 1;
    assert_eq!(c4::encode_delta(&f), Err(CodecError::WrongDirection));
}

// ---------------------------------------------------------------------------
// W6
// ---------------------------------------------------------------------------

#[test]
fn position_literals() {
    assert_eq!(p4::DELTA_TAG_POSITION, 22);
    let fact = |moving| Some(PositionFact { point: b(), moving });
    check_op(
        position(2, fact(true)),
        &[0x16, 2, 1, 0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F, 1],
    );
    check_op(
        position(2, fact(false)),
        &[0x16, 2, 1, 0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F, 0],
    );
    check_op(position(2, None), &[0x16, 2, 0]);
    // B is 7 bytes of point: 1 + 10 + 1 + 7 + 1 = 20; the largest point
    // (15 bytes) gives the contract's 28.
    assert_eq!(op_len(&position(u64::MAX, fact(true))), 20);
    let widest = Some(PositionFact {
        point: pt(L, -L, L),
        moving: true,
    });
    assert_eq!(op_len(&position(u64::MAX, widest)), 28);

    // Disclosed flag 02 before a full point and 01 is Malformed; the 01
    // control is accepted.
    op_err(
        &[0x16, 2, 2, 0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F, 1],
        CodecError::Malformed,
    );
    op_err(
        &[0x16, 2, 0xFF, 0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F, 1],
        CodecError::Malformed,
    );
    assert!(c4::decode_delta(&d1(&[
        0x16, 2, 1, 0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F, 1
    ]))
    .is_ok());
    // A moving byte of 02 is Malformed.
    op_err(
        &[0x16, 2, 1, 0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F, 2],
        CodecError::Malformed,
    );
    op_err(
        &[0x16, 2, 1, 0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F, 0xFF],
        CodecError::Malformed,
    );
    // The two-op frame.
    assert_eq!(
        c4::decode_delta(&[4, 0, 0x64, 1, 2, 0x16, 2, 0, 0x0E, 3]),
        Ok(dfr(vec![position(2, None), r(3)]))
    );
    assert_eq!(
        c4::encode_delta(&dfr(vec![position(2, None), r(3)])),
        Ok(vec![4, 0, 0x64, 1, 2, 0x16, 2, 0, 0x0E, 3])
    );
    // Entity 0 is Malformed (withheld and disclosed forms), and a missing
    // moving byte is Malformed.
    op_err(&[0x16, 0, 0], CodecError::Malformed);
    op_err(&[0x16, 0, 1, 0, 0, 0, 0], CodecError::Malformed);
    op_err(
        &[0x16, 2, 1, 0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F],
        CodecError::Malformed,
    );
    // A withheld flag takes no bytes: one trailing byte after it is Malformed
    // (a lone frame) and the next op is read from the next byte.
    op_err(&[0x16, 2, 0, 0], CodecError::Malformed);
    // Point bounds inside a Position, in x, y and z, with accepted controls.
    for pos in 0..3 {
        for good in [BL, BNL] {
            let mut tail = vec![0x16, 2, 1];
            for slot in 0..3 {
                if slot == pos {
                    tail.extend(good);
                } else {
                    tail.push(0);
                }
            }
            tail.push(1);
            assert!(c4::decode_delta(&d1(&tail)).is_ok(), "pos {pos}");
        }
        for bad in [BL1, BNL1, BMIN, BOVER] {
            let mut tail = vec![0x16, 2, 1];
            for slot in 0..3 {
                if slot == pos {
                    tail.extend(bad);
                } else {
                    tail.push(0);
                }
            }
            tail.push(1);
            op_err(&tail, CodecError::Malformed);
        }
    }
    // Truncation anywhere in a disclosed Position frame is Malformed.
    let full = d1(&[0x16, 2, 1, 0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F, 1]);
    for cut in 1..full.len() {
        assert_eq!(
            c4::decode_delta(&full[..cut]),
            Err(CodecError::Malformed),
            "prefix {cut}"
        );
    }
    // Position round trips for assorted values, in either moving state.
    for moving in [true, false] {
        for p in [a(), pt(L, -L, L), pt(-1, 1, -1), pt(1, -1, 1)] {
            let op = position(7, Some(PositionFact { point: p, moving }));
            let mut want = vec![0x16, 7, 1];
            want.extend(point_bytes(p));
            want.push(u8::from(moving));
            check_op(op, &want);
        }
    }
}

// ---------------------------------------------------------------------------
// W7
// ---------------------------------------------------------------------------

#[test]
fn v4_tag_table_closed() {
    assert_eq!(p4::PROTOCOL_VERSION, 4);
    assert_eq!(p4::INTENT_TAG_DECLARE_MOVE, 6);
    assert_eq!(p4::DELTA_TAG_MOVED, 21);
    assert_eq!(p4::DELTA_TAG_POSITION, 22);
    assert_eq!(p4::COORD_LIMIT_RAW, 536_870_912);
    assert_eq!(p4::MAX_MOVED_POINTS, 64);

    // Bare one-op frame, tag alone: 0..=22 are truncations, 23..=255 are
    // unknown tags.
    for tag in 0u8..=255 {
        let want = if tag <= 22 {
            CodecError::Malformed
        } else {
            CodecError::UnknownMessage
        };
        assert_eq!(c4::decode_delta(&d1(&[tag])), Err(want), "tag {tag}");
    }
    // The same tags with a full valid remainder after them (the Moved body),
    // so truncation cannot mask the refusal: 23..=255 stay UnknownMessage.
    let body = [2u8, 2, 0, 0, 0, 0x80, 0x80, 0x08, 0, 0];
    for tag in 23u8..=255 {
        let bytes = d1(&[&[tag][..], &body].concat());
        assert_eq!(
            c4::decode_delta(&bytes),
            Err(CodecError::UnknownMessage),
            "tag {tag}"
        );
        // And in a second op position after a valid op.
        let two = d2(&[&[0x0E, 3, tag][..], &body].concat());
        assert_eq!(
            c4::decode_delta(&two),
            Err(CodecError::UnknownMessage),
            "second-op tag {tag}"
        );
    }
    // Controls: tag 21 with that body and tag 22 with a Position body decode.
    assert!(c4::decode_delta(&d1(&[&[0x15][..], &body].concat())).is_ok());
    assert!(c4::decode_delta(&d1(&[0x16, 2, 0])).is_ok());
    assert!(c4::decode_delta(&d1(&[0x0E, 3])).is_ok());
    // Unknown tag beats trailing bytes; and the 22-position tag 23 boundary.
    assert_eq!(
        c4::decode_delta(&d1(&[23, 0, 0, 0, 0])),
        Err(CodecError::UnknownMessage)
    );

    // Intent body tags 07..=FF are unknown, with a full valid remainder (a
    // point's three bytes and more); the control tag 06 decodes.
    for tag in 7u8..=255 {
        intent_err(&[tag, 0, 0, 0], CodecError::UnknownMessage);
        intent_err(&[tag], CodecError::UnknownMessage);
    }
    assert!(c4::decode_intent(&ib(&[6, 0, 0, 0])).is_ok());
    assert!(c4::decode_intent(&ib(&[1])).is_ok());
    // Body tags 0..=5 truncated are Malformed (not unknown); tag 6 too.
    for tag in 0u8..=6 {
        if tag == 1 || (2..=4).contains(&tag) {
            continue;
        }
        intent_err(&[tag], CodecError::Malformed);
    }

    // v3 does not learn tags 21 or 22.
    let tag21 = [
        3u8, 0, 0x64, 1, 1, 0x15, 2, 2, 0, 0, 0, 0x80, 0x80, 0x08, 0, 0,
    ];
    assert_eq!(c3::decode_delta(&tag21), Err(CodecError::UnknownMessage));
    let tag22 = [3u8, 0, 0x64, 1, 1, 0x16, 2, 0];
    assert_eq!(c3::decode_delta(&tag22), Err(CodecError::UnknownMessage));
    // v1 and v2 refuse by version.
    assert_eq!(
        v1::decode_delta(&tag21),
        Err(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        v2::decode_delta(&tag21),
        Err(CodecError::UnsupportedVersion)
    );
    // The v4 form of the same bytes is accepted.
    let mut v4form = tag21;
    v4form[0] = 4;
    assert!(c4::decode_delta(&v4form).is_ok());
    // v3 tag 6 in an intent is unknown to v3 and DeclareMove to v4.
    assert_eq!(
        c3::decode_intent(&iraw(3, 0, &[1], &[1], &[6, 0, 0, 0])),
        Err(CodecError::UnknownMessage)
    );
}

// ---------------------------------------------------------------------------
// W8
// ---------------------------------------------------------------------------

#[test]
fn v4_delta_bounds() {
    // The empty frame.
    assert_eq!(c4::encode_delta(&dfr(vec![])), Ok(vec![4, 0, 0x64, 1, 0]));
    assert_eq!(c4::decode_delta(&[4, 0, 0x64, 1, 0]), Ok(dfr(vec![])));
    // 256 ops accepted; count 256 is `80 02`.
    let ops = vec![r(0); 256];
    let bytes = c4::encode_delta(&dfr(ops.clone())).unwrap();
    assert_eq!(&bytes[..6], &[4, 0, 0x64, 1, 0x80, 0x02]);
    assert_eq!(bytes.len(), 6 + 512);
    assert_eq!(c4::decode_delta(&bytes), Ok(dfr(ops)));
    // Count 257 (`81 02`) with no op following is LimitExceeded.
    assert_eq!(
        c4::decode_delta(&[4, 0, 0x64, 1, 0x81, 0x02]),
        Err(CodecError::LimitExceeded)
    );
    // Encoding 257 ops is LimitExceeded.
    assert_eq!(
        c4::encode_delta(&dfr(vec![r(0); 257])),
        Err(CodecError::LimitExceeded)
    );
    // Op count varint: u64::MAX is LimitExceeded, an overflow is Malformed.
    let mut s = vec![4, 0, 0x64, 1];
    s.extend(MAX64);
    assert_eq!(c4::decode_delta(&s), Err(CodecError::LimitExceeded));
    let mut s = vec![4, 0, 0x64, 1];
    s.extend([0xFF; 9]);
    s.push(2);
    assert_eq!(c4::decode_delta(&s), Err(CodecError::Malformed));
    // Frame lane 01 is WrongDirection (even with an oversized count).
    assert_eq!(
        c4::decode_delta(&[4, 1, 0x64, 1, 0]),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        c4::decode_delta(&[4, 1, 0x64, 1, 0x81, 0x02]),
        Err(CodecError::WrongDirection)
    );
    let mut lane1 = dfr(vec![]);
    lane1.lane = 1;
    assert_eq!(c4::encode_delta(&lane1), Err(CodecError::WrongDirection));
    // One trailing byte is Malformed.
    assert_eq!(
        c4::decode_delta(&[4, 0, 0x64, 1, 0, 0]),
        Err(CodecError::Malformed)
    );
    // Truncation anywhere in the header is Malformed.
    for cut in 1..5 {
        assert_eq!(
            c4::decode_delta(&[4, 0, 0x64, 1, 0][..cut]),
            Err(CodecError::Malformed)
        );
    }
    // A count of 1 with no op is Malformed ("no byte left").
    assert_eq!(
        c4::decode_delta(&[4, 0, 0x64, 1, 1]),
        Err(CodecError::Malformed)
    );
    // event_seq and server_tick may be any u64 (u64::MAX round-trips).
    for (tick, seq) in [(u64::MAX, u64::MAX), (0, 0), (0, u64::MAX), (u64::MAX, 0)] {
        let f = DeltaFrame {
            lane: 0,
            server_tick: tick,
            event_seq: seq,
            ops: vec![r(1), moved(2, vec![a(), pt(1, 0, 0)])],
        };
        let b = c4::encode_delta(&f).unwrap();
        assert_eq!(c4::decode_delta(&b), Ok(f));
    }
    let mut max_seq = vec![4, 0, 0x64];
    max_seq.extend(MAX64);
    max_seq.push(0);
    assert_eq!(
        c4::decode_delta(&max_seq),
        Ok(DeltaFrame {
            lane: 0,
            server_tick: 100,
            event_seq: u64::MAX,
            ops: vec![]
        })
    );

    // Encode order: lane, then count, then each op in order.
    let a_only = moved(2, vec![a()]);
    let rc_lane1 = DeltaOp::V3(receipt3(1, ReceiptStatus::Applied));
    let mut f = dfr(vec![r(0); 257]);
    f.lane = 1;
    assert_eq!(c4::encode_delta(&f), Err(CodecError::WrongDirection));
    let mut many = vec![a_only.clone()];
    many.extend(vec![r(0); 256]);
    assert_eq!(many.len(), 257);
    assert_eq!(c4::encode_delta(&dfr(many)), Err(CodecError::LimitExceeded));
    assert_eq!(
        c4::encode_delta(&dfr(vec![a_only.clone(), rc_lane1.clone()])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        c4::encode_delta(&dfr(vec![rc_lane1.clone(), a_only.clone()])),
        Err(CodecError::WrongDirection)
    );
    // Controls: each alone-valid sibling encodes.
    assert!(c4::encode_delta(&dfr(vec![moved(2, vec![a(), pt(1, 0, 0)]), r(1)])).is_ok());
    assert!(c4::encode_delta(&dfr(vec![DeltaOp::V3(receipt3(0, ReceiptStatus::Applied))])).is_ok());
}

// ---------------------------------------------------------------------------
// W9 (and W-10)
// ---------------------------------------------------------------------------

fn oa() -> Vec<DeltaOp> {
    let mut ops = Vec::new();
    ops.extend((0..67).map(|_| moved(u64::MAX, p64())));
    ops.extend((0..28).map(|_| r(u64::MAX)));
    ops.extend((0..40).map(|_| r(0)));
    ops
}

fn timeline(k: u64) -> p3::DeltaOp {
    p3::DeltaOp::Timeline {
        entries: Some(
            (0..k)
                .map(|i| TimelineEntry {
                    key: 0,
                    entity: net((1u64 << 63) + i),
                })
                .collect(),
        ),
    }
}

fn v3_b9_a() -> Vec<p3::DeltaOp> {
    let mut ops: Vec<p3::DeltaOp> = (0..5).map(|_| timeline(1024)).collect();
    ops.push(timeline(711));
    ops.extend((0..122).map(|_| p3::DeltaOp::RoundStarted { round: u64::MAX }));
    ops.push(p3::DeltaOp::RoundStarted { round: 1 << 21 });
    ops
}

fn v3_b9_b() -> Vec<p3::DeltaOp> {
    let mut ops = v3_b9_a();
    *ops.last_mut().unwrap() = p3::DeltaOp::RoundStarted { round: 1 << 28 };
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

#[test]
fn v4_frame_spans_and_cap() {
    // Op sizes the contract names.
    assert_eq!(op_len(&moved(u64::MAX, p64())), 972);
    assert_eq!(op_len(&r(u64::MAX)), 11);
    assert_eq!(op_len(&r(0)), 2);
    assert_eq!(op_len(&r(128)), 3);
    assert_eq!(
        op_len(&position(
            u64::MAX,
            Some(PositionFact {
                point: pt(L, L, L),
                moving: true
            })
        )),
        28
    );

    // At the cap.
    let ops = oa();
    assert_eq!(ops.len(), 135);
    let total: usize = ops.iter().map(op_len).sum();
    assert_eq!(total, 65_512);
    assert_eq!(c4::frame_spans(&ops), Ok(vec![0..135]));
    let frame = max_frame(ops.clone());
    let bytes = c4::encode_delta(&frame).unwrap();
    assert_eq!(bytes.len(), 65_536);
    let mut header = vec![4, 0];
    header.extend(MAX64);
    header.extend(MAX64);
    header.extend([0x87, 0x01]);
    assert_eq!(header.len(), 24);
    assert_eq!(&bytes[..24], &header[..]);
    assert_eq!(c4::decode_delta(&bytes), Ok(frame));

    // One past: the last op becomes R(128), 65,513 op bytes.
    let mut ob = oa();
    *ob.last_mut().unwrap() = r(128);
    let total: usize = ob.iter().map(op_len).sum();
    assert_eq!(total, 65_513);
    assert_eq!(
        c4::encode_delta(&max_frame(ob.clone())),
        Err(CodecError::FrameTooLarge)
    );
    assert_eq!(c4::frame_spans(&ob), Ok(vec![0..134, 134..135]));
    // Every span encodes within the cap under the largest header.
    for span in c4::frame_spans(&ob).unwrap() {
        let bytes = c4::encode_delta(&max_frame(ob[span.clone()].to_vec())).unwrap();
        assert!(bytes.len() <= 65_536);
    }

    // Op cap: 256 x R(0), 257, and empty.
    assert_eq!(c4::frame_spans(&vec![r(0); 256]), Ok(vec![0..256]));
    assert_eq!(
        c4::frame_spans(&vec![r(0); 257]),
        Ok(vec![0..256, 256..257])
    );
    assert_eq!(
        c4::frame_spans(&vec![r(0); 512]),
        Ok(vec![0..256, 256..512])
    );
    assert_eq!(
        c4::frame_spans(&vec![r(0); 513]),
        Ok(vec![0..256, 256..512, 512..513])
    );
    assert_eq!(c4::frame_spans(&[]), Ok(vec![]));
    assert_eq!(c4::frame_spans(&[r(0)]), Ok(vec![0..1]));
    // Position and Moved op mixes follow the same rule.
    let positions = vec![position(2, None); 300];
    assert_eq!(c4::frame_spans(&positions), Ok(vec![0..256, 256..300]));
    let movers = vec![moved(u64::MAX, p64()); 68];
    assert_eq!(c4::frame_spans(&movers), Ok(vec![0..67, 67..68]));
    let movers = vec![moved(u64::MAX, p64()); 67];
    assert_eq!(c4::frame_spans(&movers), Ok(vec![0..67]));
    // Spans cover 0..len contiguously for a larger mixed list.
    let mut mixed = Vec::new();
    for i in 0..700u64 {
        mixed.push(match i % 3 {
            0 => moved(i + 1, p64()),
            1 => r(u64::MAX),
            _ => position(i + 1, None),
        });
    }
    let spans = c4::frame_spans(&mixed).unwrap();
    let mut next = 0;
    for span in &spans {
        assert_eq!(span.start, next);
        assert!(span.end > span.start);
        assert!(span.len() <= 256);
        let bytes: usize = mixed[span.clone()].iter().map(op_len).sum();
        assert!(bytes <= 65_512);
        // Maximal: adding the next op would break a limit.
        if span.end < mixed.len() {
            let with_next = bytes + op_len(&mixed[span.end]);
            assert!(span.len() + 1 > 256 || with_next > 65_512);
        }
        next = span.end;
    }
    assert_eq!(next, mixed.len());

    // Error: the first failing op wins and no spans are returned.
    assert_eq!(
        c4::frame_spans(&[r(0), moved(2, vec![a()])]),
        Err(CodecError::Malformed)
    );
    let mut late = vec![r(0); 300];
    late.push(moved(2, vec![a()]));
    assert_eq!(c4::frame_spans(&late), Err(CodecError::Malformed));
    assert_eq!(
        c4::frame_spans(&[
            moved(2, vec![a()]),
            DeltaOp::V3(receipt3(1, ReceiptStatus::Applied))
        ]),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        c4::frame_spans(&[
            DeltaOp::V3(receipt3(1, ReceiptStatus::Applied)),
            moved(2, vec![a()])
        ]),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        c4::frame_spans(&[moved(2, p65())]),
        Err(CodecError::LimitExceeded)
    );
    // Control: the same ops without the bad one split.
    assert!(c4::frame_spans(&vec![r(0); 300]).is_ok());

    // Same as v3 for the B9 `A` and `B` op lists wrapped in V3.
    for (name, ops3) in [("A", v3_b9_a()), ("B", v3_b9_b())] {
        let wrapped: Vec<DeltaOp> = ops3.iter().cloned().map(DeltaOp::V3).collect();
        assert_eq!(c4::frame_spans(&wrapped), c3::frame_spans(&ops3), "{name}");
    }
    assert_eq!(c3::frame_spans(&v3_b9_a()), Ok(vec![0..129]));
    assert_eq!(c3::frame_spans(&v3_b9_b()), Ok(vec![0..128, 128..129]));
    // And through the other v3 op shapes in general.
    let ops3: Vec<p3::DeltaOp> = v3_cases().into_iter().map(|(op, _)| op).collect();
    let wrapped: Vec<DeltaOp> = ops3.iter().cloned().map(DeltaOp::V3).collect();
    assert_eq!(c4::frame_spans(&wrapped), c3::frame_spans(&ops3));
}

// ---------------------------------------------------------------------------
// W10
// ---------------------------------------------------------------------------

#[test]
fn v4_round_trip_identity() {
    // Every intent body.
    let bodies = [
        IntentBody::DeclareAction {
            ability: ulid(1),
            target: net(2),
        },
        IntentBody::EndTurn,
        IntentBody::Stop,
        IntentBody::Pause,
        IntentBody::Unpause,
        IntentBody::RequestMode {
            mode: EncounterMode::TurnBased,
        },
        IntentBody::RequestMode {
            mode: EncounterMode::RealTime,
        },
        IntentBody::DeclareMove { destination: b() },
        IntentBody::DeclareMove {
            destination: pt(-L, L, -L),
        },
        IntentBody::DeclareMove { destination: a() },
    ];
    for body in bodies {
        let f = ifr(body);
        let b1 = c4::encode_intent(&f).unwrap();
        assert_eq!(c4::encode_intent(&f).unwrap(), b1);
        assert_eq!(c4::decode_intent(&b1), Ok(f));
    }
    // A V3 op of every v3 tag, Moved, and Position with Some and None.
    let mut ops: Vec<DeltaOp> = v3_cases()
        .into_iter()
        .map(|(op, _)| DeltaOp::V3(op))
        .collect();
    ops.push(moved(2, vec![a(), pt(65536, 0, 0)]));
    ops.push(moved(u64::MAX, p64()));
    ops.push(position(
        2,
        Some(PositionFact {
            point: b(),
            moving: true,
        }),
    ));
    ops.push(position(
        2,
        Some(PositionFact {
            point: pt(-L, 0, L),
            moving: false,
        }),
    ));
    ops.push(position(3, None));
    // All 21 v3 tags are present.
    let mut tags = std::collections::BTreeSet::new();
    for op in &ops {
        let bytes = c4::encode_delta(&dfr(vec![op.clone()])).unwrap();
        tags.insert(bytes[5]);
    }
    assert_eq!(tags, (0u8..=22).collect());
    for op in &ops {
        let f = dfr(vec![op.clone()]);
        let b1 = c4::encode_delta(&f).unwrap();
        assert_eq!(c4::encode_delta(&f).unwrap(), b1);
        assert_eq!(c4::decode_delta(&b1), Ok(f));
    }
    // Lowercase ULID alias in DeclareAction: decodes, re-encodes uppercase.
    let mut lower_tail = vec![0, 0x1A];
    lower_tail.extend([0x30u8; 25]);
    lower_tail.push(0x61);
    lower_tail.extend([2]);
    let lower = ib(&lower_tail);
    let decoded = c4::decode_intent(&lower).unwrap();
    assert_eq!(
        decoded.body,
        IntentBody::DeclareAction {
            ability: ulid(10),
            target: net(2)
        }
    );
    let again = c4::encode_intent(&decoded).unwrap();
    let mut upper_tail = lower_tail.clone();
    upper_tail[2 + 25] = 0x41;
    assert_eq!(again, ib(&upper_tail));
    assert_ne!(again, lower);

    // Point::new is Some exactly when every |component| <= 2^29.
    let probes = [
        0,
        1,
        -1,
        L - 1,
        L,
        L + 1,
        -L + 1,
        -L,
        -L - 1,
        i32::MAX,
        i32::MIN,
        i32::MIN + 1,
    ];
    for &x in &probes {
        for &y in &probes {
            for &z in &probes {
                let ok = [x, y, z].iter().all(|c| c.unsigned_abs() <= L as u32);
                let got = Point::new([x, y, z]);
                assert_eq!(got.is_some(), ok, "{x} {y} {z}");
                if let Some(p) = got {
                    assert_eq!(p.raw(), [x, y, z]);
                }
            }
        }
    }
    // Point derives equality and order, and is Copy.
    let p1 = pt(1, 2, 3);
    let p2 = p1;
    assert_eq!(p1, p2);
    assert!(pt(1, 2, 3) < pt(1, 2, 4));
}

// ---------------------------------------------------------------------------
// W11
// ---------------------------------------------------------------------------

#[test]
fn v4_constants_match_sim() {
    assert_eq!(p4::COORD_LIMIT_RAW, crpg_sim::COORD_LIMIT_RAW);
    assert_eq!(p4::MAX_MOVED_POINTS, crpg_sim::MAX_ROUTE_POINTS);
    assert_eq!(p4::COORD_LIMIT_RAW, L);
}
