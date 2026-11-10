//! T140a two-keys contract tests: protocol v3 version gate, intents, scalar
//! ops. Written from `tasks/T140.md` alone; every byte oracle is hand-derived
//! (postcard: LEB128 varints, zigzag signed, one-byte bools, length-prefixed
//! strings) and never produced by running the codec under test.

use crpg_core::Ulid;
use crpg_net::codec::{self as v1, CodecError};
use crpg_net::codec_v2 as v2;
use crpg_net::codec_v3 as c3;
use crpg_net::protocol as p1;
use crpg_net::protocol_v2 as p2;
use crpg_net::protocol_v3::{
    self as p3, AutoPauseToggles, AutoPauseTrigger, DeltaFrame, DeltaOp, EncounterMode, HostFacts,
    IntentBody, IntentFrame, NetId, PauseReason, PauseRights, ReceiptStatus, RejectionCode,
};

const Z16: [u8; 16] = [0; 16];
const TRIGGERS: [AutoPauseTrigger; 5] = [
    AutoPauseTrigger::EncounterStarted,
    AutoPauseTrigger::RoundStarted,
    AutoPauseTrigger::ModeChanged,
    AutoPauseTrigger::OrderEnded,
    AutoPauseTrigger::PartyMemberDied,
];
/// Varint of `u64::MAX`.
const MAX64: [u8; 10] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];
/// Varint of `u64::MAX - 1` (`SEQ_LAST`).
const LAST64: [u8; 10] = [0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];

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

/// `03 Z16 lane seq 07 actor` plus `tail`.
fn iraw(lane: u8, seq: &[u8], actor: &[u8], tail: &[u8]) -> Vec<u8> {
    [&[3][..], &Z16, &[lane], seq, &[7], actor, tail].concat()
}

/// Standard intent (lane 0, seq 1, observed_tick 7, actor 1) plus `tail`.
fn ib(tail: &[u8]) -> Vec<u8> {
    iraw(0, &[1], &[1], tail)
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

/// Intent oracle: encodes to `ib(tail)`, decodes back, re-encodes equal.
fn check_intent(body: IntentBody, tail: &[u8]) {
    let frame = ifr(body);
    let bytes = ib(tail);
    assert_eq!(c3::encode_intent(&frame), Ok(bytes.clone()));
    assert_eq!(c3::decode_intent(&bytes), Ok(frame));
    let again = c3::encode_intent(&c3::decode_intent(&bytes).unwrap()).unwrap();
    assert_eq!(again, bytes);
}

/// Delta oracle for a one-op frame, both directions.
fn check_op(op: DeltaOp, op_bytes: &[u8]) {
    let frame = dfr(vec![op]);
    let bytes = d1(op_bytes);
    assert_eq!(c3::encode_delta(&frame), Ok(bytes.clone()));
    assert_eq!(c3::decode_delta(&bytes), Ok(frame));
    let again = c3::encode_delta(&c3::decode_delta(&bytes).unwrap()).unwrap();
    assert_eq!(again, bytes);
}

fn op_err(op_bytes: &[u8], err: CodecError) {
    assert_eq!(c3::decode_delta(&d1(op_bytes)), Err(err), "{op_bytes:02X?}");
}

fn intent_err(tail: &[u8], err: CodecError) {
    assert_eq!(c3::decode_intent(&ib(tail)), Err(err), "{tail:02X?}");
}

fn action(outcome: &str) -> DeltaOp {
    DeltaOp::ActionResolved {
        actor: net(1),
        target: net(2),
        ability: ulid(1),
        outcome: outcome.to_string(),
        damage: 7,
    }
}

fn receipt(lane: u8, seq: u64, status: ReceiptStatus) -> DeltaOp {
    DeltaOp::Receipt {
        epoch: Z16,
        lane,
        seq,
        processed_tick: 100,
        status,
    }
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

fn host_facts(pause: bool, unpause: bool, auto: Option<u8>, timer: Option<Option<u32>>) -> DeltaOp {
    DeltaOp::HostFacts(HostFacts {
        rights: PauseRights { pause, unpause },
        auto_pause: auto.map(|b| AutoPauseToggles::from_bits(b).expect("valid bits")),
        turn_timer_ms: timer,
    })
}

// ---------------------------------------------------------------------------
// A1
// ---------------------------------------------------------------------------

#[test]
fn v3_version_gate() {
    let frame = ifr(IntentBody::EndTurn);
    let mut want = vec![3];
    want.extend_from_slice(&Z16);
    want.extend_from_slice(&[0, 1, 7, 1, 1]);
    assert_eq!(want.len(), 22);
    let b3 = c3::encode_intent(&frame).unwrap();
    assert_eq!(b3, want);
    assert_eq!(p3::PROTOCOL_VERSION, 3);

    let b1 = v1::encode_intent(&v1_end_turn()).unwrap();
    let b2 = v2::encode_intent(&v1_end_turn()).unwrap();
    assert_eq!((b1[0], b2[0]), (1, 2));
    let mut b1_as_3 = b1.clone();
    b1_as_3[0] = 3;
    assert_eq!(b1_as_3, b3);

    for bad in [
        &b1[..],
        &b2[..],
        &[0x81][..],
        &[0x91][..],
        &[0][..],
        &[4][..],
    ] {
        assert_eq!(c3::decode_intent(bad), Err(CodecError::UnsupportedVersion));
        assert_eq!(c3::decode_delta(bad), Err(CodecError::UnsupportedVersion));
    }
    assert_eq!(c3::decode_intent(&[]), Err(CodecError::Malformed));
    assert_eq!(c3::decode_delta(&[]), Err(CodecError::Malformed));
    // The old codecs refuse v3 bytes; the controls still decode.
    assert_eq!(v1::decode_intent(&b3), Err(CodecError::UnsupportedVersion));
    assert_eq!(v2::decode_intent(&b3), Err(CodecError::UnsupportedVersion));
    assert_eq!(v1::decode_intent(&b1), Ok(v1_end_turn()));
    let empty3 = [3, 0, 0x64, 1, 0];
    assert_eq!(
        v1::decode_delta(&empty3),
        Err(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        v2::decode_delta(&empty3),
        Err(CodecError::UnsupportedVersion)
    );
    assert_eq!(c3::decode_delta(&empty3), Ok(dfr(vec![])));
    for first in [1u8, 2] {
        let old = [first, 0, 0x64, 1, 0];
        assert_eq!(c3::decode_delta(&old), Err(CodecError::UnsupportedVersion));
    }

    // Frame cap before version: first byte 2 is wrong, yet the cap wins.
    let mut big_i = vec![2u8; 4097];
    assert_eq!(c3::decode_intent(&big_i), Err(CodecError::FrameTooLarge));
    big_i.truncate(4096);
    assert_eq!(
        c3::decode_intent(&big_i),
        Err(CodecError::UnsupportedVersion)
    );
    let mut big_d = vec![2u8; 65_537];
    assert_eq!(c3::decode_delta(&big_d), Err(CodecError::FrameTooLarge));
    big_d.truncate(65_536);
    assert_eq!(
        c3::decode_delta(&big_d),
        Err(CodecError::UnsupportedVersion)
    );
    // At the cap with version 3: parsed, then trailing bytes; one more is too big.
    let mut pad_i = b3.clone();
    pad_i.resize(4096, 0);
    assert_eq!(c3::decode_intent(&pad_i), Err(CodecError::Malformed));
    pad_i.push(0);
    assert_eq!(c3::decode_intent(&pad_i), Err(CodecError::FrameTooLarge));
    let mut pad_d = empty3.to_vec();
    pad_d.resize(65_536, 0);
    assert_eq!(c3::decode_delta(&pad_d), Err(CodecError::Malformed));
    pad_d.push(0);
    assert_eq!(c3::decode_delta(&pad_d), Err(CodecError::FrameTooLarge));
}

// ---------------------------------------------------------------------------
// A2
// ---------------------------------------------------------------------------

#[test]
fn v3_intent_literals() {
    check_intent(IntentBody::EndTurn, &[1]);
    check_intent(IntentBody::Stop, &[2]);
    check_intent(IntentBody::Pause, &[3]);
    check_intent(IntentBody::Unpause, &[4]);
    for body in [IntentBody::Stop, IntentBody::Pause, IntentBody::Unpause] {
        assert_eq!(c3::encode_intent(&ifr(body)).unwrap().len(), 22);
    }
    let real = IntentBody::RequestMode {
        mode: EncounterMode::RealTime,
    };
    let turn = IntentBody::RequestMode {
        mode: EncounterMode::TurnBased,
    };
    check_intent(real, &[5, 1]);
    check_intent(turn, &[5, 0]);
    assert_eq!(c3::encode_intent(&ifr(real)).unwrap().len(), 23);

    // The Ulid oracle itself: 25 zeros then `1`.
    assert_eq!(utext(1), [&[0x1A][..], &[0x30; 25], &[0x31]].concat());
    let decl = IntentBody::DeclareAction {
        ability: ulid(1),
        target: net(2),
    };
    let tail = [&[0][..], &utext(1), &[2]].concat();
    check_intent(decl, &tail);
    assert_eq!(c3::encode_intent(&ifr(decl)).unwrap().len(), 50);
    // DeclareAction matches v1's payload; only byte 0 differs.
    let v1_decl = p1::IntentFrame {
        body: p1::IntentBody::DeclareAction {
            ability: ulid(1),
            target: net(2),
        },
        ..v1_end_turn()
    };
    let mut b = v1::encode_intent(&v1_decl).unwrap();
    b[0] = 3;
    assert_eq!(b, ib(&tail));
    // Header fields at their extremes.
    let wide = IntentFrame {
        epoch: [0xAB; 16],
        lane: 0,
        seq: p1::SEQ_LAST,
        observed_tick: u64::MAX,
        actor: net(u64::MAX),
        body: IntentBody::Stop,
    };
    let want = [&[3][..], &[0xAB; 16], &[0], &LAST64, &MAX64, &MAX64, &[2]].concat();
    assert_eq!(c3::encode_intent(&wide), Ok(want.clone()));
    assert_eq!(c3::decode_intent(&want), Ok(wide));
}

// ---------------------------------------------------------------------------
// A3
// ---------------------------------------------------------------------------

#[test]
fn v3_intent_refusals() {
    use CodecError::*;
    intent_err(&[6], UnknownMessage);
    intent_err(&[0xFF], UnknownMessage);
    intent_err(&[5, 2], UnknownMessage);
    intent_err(&[5, 0xFF], UnknownMessage);
    intent_err(&[5], Malformed);
    intent_err(&[], Malformed);
    intent_err(&[1, 0], Malformed);
    intent_err(&[5, 1, 0], Malformed);
    for tag in 1..=4u8 {
        assert!(c3::decode_intent(&ib(&[tag])).is_ok());
    }
    assert!(c3::decode_intent(&ib(&[5, 1])).is_ok());
    // Header: lane, seq, actor.
    assert_eq!(
        c3::decode_intent(&iraw(1, &[1], &[1], &[1])),
        Err(WrongDirection)
    );
    assert_eq!(
        c3::decode_intent(&iraw(0xFF, &[1], &[1], &[1])),
        Err(WrongDirection)
    );
    assert_eq!(
        c3::decode_intent(&iraw(0, &[0], &[1], &[1])),
        Err(Malformed)
    );
    assert_eq!(
        c3::decode_intent(&iraw(0, &MAX64, &[1], &[1])),
        Err(Malformed)
    );
    let ok = c3::decode_intent(&iraw(0, &LAST64, &[1], &[1])).unwrap();
    assert_eq!(ok.seq, p1::SEQ_LAST);
    assert_eq!(
        c3::decode_intent(&iraw(0, &[1], &[0], &[1])),
        Err(Malformed)
    );
    // Order: lane beats seq; seq and actor beat the body tag; the tag beats
    // trailing bytes and later fields.
    assert_eq!(
        c3::decode_intent(&iraw(1, &[0], &[0], &[6])),
        Err(WrongDirection)
    );
    assert_eq!(
        c3::decode_intent(&iraw(0, &[0], &[1], &[6])),
        Err(Malformed)
    );
    assert_eq!(
        c3::decode_intent(&iraw(0, &[1], &[0], &[6])),
        Err(Malformed)
    );
    intent_err(&[6, 0, 0], UnknownMessage);
    intent_err(&[5, 2, 0], UnknownMessage);
    // Version first, then epoch truncation.
    let mut v = ib(&[1]);
    v[0] = 4;
    assert_eq!(c3::decode_intent(&v), Err(UnsupportedVersion));
    v[0] = 3;
    v.truncate(10);
    assert_eq!(c3::decode_intent(&v), Err(Malformed));
    // DeclareAction body.
    intent_err(&[0, 0x81, 0x02], LimitExceeded);
    let a256 = [&[0][..], &[0x80, 0x02], &[0x41; 256], &[2]].concat();
    intent_err(&a256, Malformed);
    let bang = [&[0, 0x1A][..], &[b'!'; 26], &[2]].concat();
    intent_err(&bang, Malformed);
    let short = [&[0, 0x19][..], &[b'0'; 25], &[2]].concat();
    intent_err(&short, Malformed);
    intent_err(&[&[0][..], &utext(1), &[0]].concat(), Malformed);
    intent_err(&[&[0][..], &utext(1)].concat(), Malformed);
    intent_err(&[&[0][..], &utext(1), &[2, 0]].concat(), Malformed);
    assert!(c3::decode_intent(&ib(&[&[0][..], &utext(1), &[2]].concat())).is_ok());
    // Every strict prefix of a valid frame is truncation.
    for body in [
        IntentBody::EndTurn,
        IntentBody::RequestMode {
            mode: EncounterMode::RealTime,
        },
        IntentBody::DeclareAction {
            ability: ulid(1),
            target: net(2),
        },
    ] {
        let full = c3::encode_intent(&ifr(body)).unwrap();
        for n in 1..full.len() {
            assert_eq!(c3::decode_intent(&full[..n]), Err(Malformed), "prefix {n}");
        }
    }
    // Encode: lane before seq.
    let mut f = ifr(IntentBody::EndTurn);
    f.lane = 1;
    assert_eq!(c3::encode_intent(&f), Err(WrongDirection));
    f.seq = 0;
    assert_eq!(c3::encode_intent(&f), Err(WrongDirection));
    f.lane = 0;
    assert_eq!(c3::encode_intent(&f), Err(Malformed));
    f.seq = u64::MAX;
    assert_eq!(c3::encode_intent(&f), Err(Malformed));
    f.seq = p1::SEQ_LAST;
    assert!(c3::encode_intent(&f).is_ok());
    f.seq = p1::SEQ_FIRST;
    assert!(c3::encode_intent(&f).is_ok());
}

// ---------------------------------------------------------------------------
// A4
// ---------------------------------------------------------------------------

/// Every v1 and v2 op shape, receipts with codes 1..=20.
fn legacy_ops() -> Vec<p2::DeltaOp> {
    let l = p2::DeltaOp::Legacy;
    let mut ops = vec![
        l(p1::DeltaOp::EntityEnter { entity: net(1) }),
        l(p1::DeltaOp::EntityLeave { entity: net(1) }),
        l(p1::DeltaOp::Spawned { entity: net(1) }),
        l(p1::DeltaOp::Despawned { entity: net(1) }),
        l(p1::DeltaOp::Died { entity: net(1) }),
        l(p1::DeltaOp::Health {
            entity: net(1),
            health: 10,
            max_health: 10,
            dead: false,
        }),
        l(p1::DeltaOp::Turn {
            active: None,
            round: 5,
        }),
        l(p1::DeltaOp::Turn {
            active: Some(net(1)),
            round: 5,
        }),
        p2::DeltaOp::ActionResolved {
            actor: net(1),
            target: net(2),
            ability: ulid(1),
            outcome: "success".to_string(),
            damage: 7,
        },
        p2::DeltaOp::TurnStarted {
            actor: net(1),
            round: 3,
        },
        p2::DeltaOp::EncounterEnded {
            encounter: ulid(1),
            round: 3,
        },
    ];
    let rejected = (1..=20u8)
        .map(|c| p1::ReceiptStatus::Rejected(p1::RejectionCode::from_u8(c).expect("v1 code")));
    for status in std::iter::once(p1::ReceiptStatus::Applied).chain(rejected) {
        ops.push(l(p1::DeltaOp::Receipt {
            epoch: Z16,
            lane: 0,
            seq: 1,
            processed_tick: 100,
            status,
        }));
    }
    ops
}

#[test]
fn v3_legacy_tags_identical() {
    let ops = legacy_ops();
    assert_eq!(ops.len(), 11 + 21);
    for o in ops {
        let v2_frame = p2::DeltaFrame {
            lane: 0,
            server_tick: 100,
            event_seq: 1,
            ops: vec![o.clone()],
        };
        let mut want = v2::encode_delta(&v2_frame).unwrap();
        assert_eq!(want[0], 2);
        want[0] = 3;
        let frame = dfr(vec![DeltaOp::from(o.clone())]);
        assert_eq!(c3::encode_delta(&frame), Ok(want.clone()), "{o:?}");
        assert_eq!(c3::decode_delta(&want), Ok(frame));
        assert_eq!(v2::decode_delta(&want), Err(CodecError::UnsupportedVersion));
    }
    // Independent literals.
    check_op(DeltaOp::EntityEnter { entity: net(1) }, &[0, 1]);
    check_op(DeltaOp::EntityLeave { entity: net(1) }, &[1, 1]);
    check_op(DeltaOp::Spawned { entity: net(1) }, &[2, 1]);
    check_op(DeltaOp::Despawned { entity: net(1) }, &[3, 1]);
    check_op(DeltaOp::Died { entity: net(1) }, &[4, 1]);
    let health = DeltaOp::Health {
        entity: net(1),
        health: 10,
        max_health: 10,
        dead: false,
    };
    check_op(health, &[5, 1, 10, 10, 0]);
    let dead = DeltaOp::Health {
        entity: net(1),
        health: 0,
        max_health: 128,
        dead: true,
    };
    check_op(dead, &[5, 1, 0, 0x80, 1, 1]);
    let turn_none = DeltaOp::Turn {
        active: None,
        round: 5,
    };
    assert_eq!(
        c3::encode_delta(&dfr(vec![turn_none.clone()])),
        Ok(vec![0x03, 0x00, 0x64, 0x01, 0x01, 0x06, 0x00, 0x05])
    );
    check_op(turn_none, &[6, 0, 5]);
    let turn_some = DeltaOp::Turn {
        active: Some(net(1)),
        round: 5,
    };
    check_op(turn_some, &[6, 1, 1, 5]);
    let tail = [&[8, 1, 2][..], &utext(1), &[7], b"success", &[7]].concat();
    check_op(action("success"), &tail);
    let started = DeltaOp::TurnStarted {
        actor: net(1),
        round: 3,
    };
    check_op(started, &[9, 1, 3]);
    let ended = DeltaOp::EncounterEnded {
        encounter: ulid(1),
        round: 3,
    };
    check_op(ended, &[&[10][..], &utext(1), &[3]].concat());
    // Zero ids in legacy payloads are malformed.
    for tag in 0..=4u8 {
        op_err(&[tag, 0], CodecError::Malformed);
    }
    op_err(&[9, 0, 3], CodecError::Malformed);
    op_err(&[5, 0, 1, 1, 0], CodecError::Malformed);
}

// ---------------------------------------------------------------------------
// A5 and A6 of Behaviour (codes)
// ---------------------------------------------------------------------------

#[test]
fn v3_receipt_codes() {
    use CodecError::*;
    let applied = receipt(0, 1, ReceiptStatus::Applied);
    let head = [&[7][..], &Z16, &[0, 1, 0x64]].concat();
    let applied_bytes = [&head[..], &[0]].concat();
    check_op(applied.clone(), &applied_bytes);
    assert_eq!(c3::encode_delta(&dfr(vec![applied])).unwrap().len(), 26);

    let modeunsup = receipt(
        0,
        1,
        ReceiptStatus::Rejected(RejectionCode::ModeUnsupported),
    );
    check_op(modeunsup, &[&head[..], &[1, 0x15]].concat());
    let nopause = receipt(0, 1, ReceiptStatus::Rejected(RejectionCode::NoPauseRight));
    check_op(nopause, &[&head[..], &[1, 0x1E]].concat());

    for n in 1..=30u8 {
        let code = RejectionCode::from_u8(n).expect("1..=30");
        assert_eq!(code.as_u8(), n);
        check_op(
            receipt(0, 1, ReceiptStatus::Rejected(code)),
            &[&head[..], &[1, n]].concat(),
        );
    }
    for n in 0..=255u8 {
        assert_eq!(
            RejectionCode::from_u8(n).is_some(),
            (1..=30).contains(&n),
            "{n}"
        );
    }
    for bad in [0u8, 0x1F, 0xFF, 31, 100] {
        op_err(&[&head[..], &[1, bad]].concat(), UnknownMessage);
    }
    op_err(&[&head[..], &[2]].concat(), UnknownMessage);
    op_err(&[&head[..], &[0xFF]].concat(), UnknownMessage);
    op_err(&[&head[..], &[1]].concat(), Malformed);
    op_err(&head, Malformed);
    // Receipt lane and seq.
    let r = |lane: u8, seq: &[u8], status: &[u8]| {
        [&[7][..], &Z16, &[lane], seq, &[0x64], status].concat()
    };
    op_err(&r(1, &[1], &[0]), WrongDirection);
    op_err(&r(0xFF, &[1], &[0]), WrongDirection);
    op_err(&r(0, &[0], &[0]), Malformed);
    op_err(&r(0, &MAX64, &[0]), Malformed);
    let last = c3::decode_delta(&d1(&r(0, &LAST64, &[0]))).unwrap();
    assert_eq!(
        last.ops,
        vec![receipt(0, p1::SEQ_LAST, ReceiptStatus::Applied)]
    );
    // Fields are checked left to right: lane, then seq, then status.
    op_err(&r(1, &[0], &[2]), WrongDirection);
    op_err(&r(0, &[0], &[2]), Malformed);
    op_err(&r(0, &[1], &[2]), UnknownMessage);
    // Encode checks the receipt lane, then seq.
    let bad_lane = dfr(vec![receipt(1, 1, ReceiptStatus::Applied)]);
    assert_eq!(c3::encode_delta(&bad_lane), Err(WrongDirection));
    let bad_seq = dfr(vec![receipt(0, 0, ReceiptStatus::Applied)]);
    assert_eq!(c3::encode_delta(&bad_seq), Err(Malformed));
    let bad_seq = dfr(vec![receipt(0, u64::MAX, ReceiptStatus::Applied)]);
    assert_eq!(c3::encode_delta(&bad_seq), Err(Malformed));
    let both = dfr(vec![receipt(1, 0, ReceiptStatus::Applied)]);
    assert_eq!(c3::encode_delta(&both), Err(WrongDirection));
}

#[test]
fn v3_rejection_code_mapping() {
    for n in 0..=255u8 {
        let old = p1::RejectionCode::from_u8(n);
        assert_eq!(old.is_some(), (1..=20).contains(&n), "{n}");
        if let Some(code) = old {
            let new = RejectionCode::from(code);
            assert_eq!(new.as_u8(), n);
            assert_eq!(RejectionCode::from_u8(n), Some(new));
            assert_eq!(
                ReceiptStatus::from(p1::ReceiptStatus::Rejected(code)),
                ReceiptStatus::Rejected(new)
            );
        }
    }
    assert_eq!(
        ReceiptStatus::from(p1::ReceiptStatus::Applied),
        ReceiptStatus::Applied
    );
    assert_eq!(RejectionCode::from_u8(0), None);
    assert_eq!(RejectionCode::from_u8(31), None);
    assert_eq!(RejectionCode::from_u8(255), None);
    for n in 21..=30u8 {
        assert_eq!(p1::RejectionCode::from_u8(n), None);
    }
    assert_eq!(RejectionCode::ModeUnsupported as u8, 21);
    assert_eq!(RejectionCode::NoPauseRight as u8, 30);
    assert_eq!(RejectionCode::SnapshotExpired as u8, 20);
    assert_eq!(RejectionCode::FrameTooLarge as u8, 1);
}

// ---------------------------------------------------------------------------
// A6
// ---------------------------------------------------------------------------

#[test]
fn v3_event_literals() {
    use CodecError::*;
    check_op(DeltaOp::RoundStarted { round: 3 }, &[0x0E, 3]);
    let real = DeltaOp::ModeChanged {
        mode: EncounterMode::RealTime,
        round: 2,
    };
    check_op(real, &[0x0F, 1, 2]);
    let turn = DeltaOp::ModeChanged {
        mode: EncounterMode::TurnBased,
        round: 2,
    };
    check_op(turn, &[0x0F, 0, 2]);
    check_op(DeltaOp::OrderEnded { actor: net(2) }, &[0x10, 2]);
    check_op(DeltaOp::Joined { entity: net(2) }, &[0x11, 2]);
    check_op(DeltaOp::Left { entity: net(2) }, &[0x12, 2]);
    // Round varints at their edges.
    check_op(DeltaOp::RoundStarted { round: 0 }, &[0x0E, 0]);
    check_op(DeltaOp::RoundStarted { round: 127 }, &[0x0E, 0x7F]);
    check_op(DeltaOp::RoundStarted { round: 128 }, &[0x0E, 0x80, 1]);
    check_op(
        DeltaOp::RoundStarted { round: u64::MAX },
        &[&[0x0E][..], &MAX64].concat(),
    );
    let wide = DeltaOp::ModeChanged {
        mode: EncounterMode::TurnBased,
        round: u64::MAX,
    };
    check_op(wide, &[&[0x0F, 0][..], &MAX64].concat());
    check_op(
        DeltaOp::OrderEnded {
            actor: net(u64::MAX),
        },
        &[&[0x10][..], &MAX64].concat(),
    );
    // Refusals.
    op_err(&[0x0F, 2, 2], UnknownMessage);
    op_err(&[0x0F, 0xFF, 2], UnknownMessage);
    op_err(&[0x0F, 2], UnknownMessage);
    op_err(&[0x0F, 1], Malformed);
    op_err(&[0x0F], Malformed);
    op_err(&[0x10, 0], Malformed);
    op_err(&[0x11, 0], Malformed);
    op_err(&[0x12, 0], Malformed);
    op_err(&[0x0E], Malformed);
    op_err(&[0x10], Malformed);
    // Varint overflow: ten bytes whose last carries more than one bit.
    let over = [&[0x0E][..], &[0xFF; 9], &[0x02]].concat();
    op_err(&over, Malformed);
    let endless = [&[0x0E][..], &[0xFF; 12]].concat();
    op_err(&endless, Malformed);
}

// ---------------------------------------------------------------------------
// A7 and A9 of Behaviour (toggles)
// ---------------------------------------------------------------------------

#[test]
fn v3_clock_and_host_facts() {
    use CodecError::*;
    check_op(DeltaOp::Clock { paused: None }, &[0x13, 0]);
    check_op(DeltaOp::Clock { paused: Some(None) }, &[0x13, 1, 0]);
    let paused = |r| DeltaOp::Clock {
        paused: Some(Some(r)),
    };
    check_op(paused(PauseReason::Player), &[0x13, 1, 1, 0]);
    check_op(paused(PauseReason::Operator), &[0x13, 1, 1, 2]);
    check_op(paused(PauseReason::Restore), &[0x13, 1, 1, 3]);
    for (n, t) in TRIGGERS.into_iter().enumerate() {
        check_op(paused(PauseReason::Auto(t)), &[0x13, 1, 1, 1, n as u8]);
    }
    check_op(
        paused(PauseReason::Auto(AutoPauseTrigger::OrderEnded)),
        &[0x13, 1, 1, 1, 3],
    );
    // A withheld reason has no bytes after its flag.
    op_err(&[0x13, 1, 0, 0], Malformed);
    op_err(&[0x13, 0, 0], Malformed);
    op_err(&[0x13, 2], Malformed);
    op_err(&[0x13, 2, 1, 0], Malformed);
    op_err(&[0x13, 1, 2], Malformed);
    op_err(&[0x13, 1, 2, 0], Malformed);
    op_err(&[0x13], Malformed);
    op_err(&[0x13, 1], Malformed);
    op_err(&[0x13, 1, 1], Malformed);
    op_err(&[0x13, 1, 1, 1], Malformed);
    op_err(&[0x13, 1, 1, 4], UnknownMessage);
    op_err(&[0x13, 1, 1, 0xFF], UnknownMessage);
    op_err(&[0x13, 1, 1, 1, 5], UnknownMessage);
    op_err(&[0x13, 1, 1, 1, 0xFF], UnknownMessage);
    op_err(&[0x13, 1, 1, 0, 0], Malformed);
    // HostFacts.
    check_op(host_facts(true, true, None, None), &[0x14, 1, 1, 0, 0]);
    check_op(
        host_facts(true, true, Some(0x1F), Some(Some(30000))),
        &[0x14, 1, 1, 1, 0x1F, 1, 1, 0xB0, 0xEA, 1],
    );
    check_op(
        host_facts(true, true, None, Some(None)),
        &[0x14, 1, 1, 0, 1, 0],
    );
    check_op(
        host_facts(false, false, Some(0), Some(Some(0))),
        &[0x14, 0, 0, 1, 0, 1, 1, 0],
    );
    check_op(
        host_facts(true, false, Some(0x08), Some(Some(u32::MAX))),
        &[0x14, 1, 0, 1, 8, 1, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
    );
    check_op(
        host_facts(false, true, None, Some(Some(127))),
        &[0x14, 0, 1, 0, 1, 1, 0x7F],
    );
    check_op(
        host_facts(false, true, None, Some(Some(128))),
        &[0x14, 0, 1, 0, 1, 1, 0x80, 1],
    );
    // Refusals.
    op_err(&[0x14, 1, 1, 1, 0x20, 0], Malformed);
    op_err(&[0x14, 1, 1, 1, 0x80, 0], Malformed);
    op_err(&[0x14, 1, 1, 1, 0xE0, 0], Malformed);
    op_err(&[0x14, 1, 1, 1, 0xFF, 0], Malformed);
    assert!(c3::decode_delta(&d1(&[0x14, 1, 1, 1, 0x1F, 0])).is_ok());
    op_err(&[0x14, 1, 1, 0, 1, 2], Malformed);
    op_err(&[0x14, 1, 1, 0, 2], Malformed);
    op_err(&[0x14, 1, 1, 2, 0], Malformed);
    op_err(&[0x14, 2, 1, 0, 0], Malformed);
    op_err(&[0x14, 1, 2, 0, 0], Malformed);
    op_err(&[0x14, 1, 1, 0, 1, 1], Malformed);
    op_err(&[0x14, 1, 1, 0, 1, 1, 0xFF, 0xFF, 0xFF, 0xFF], Malformed);
    op_err(&[0x14, 1, 1, 0], Malformed);
    op_err(&[0x14, 1, 1, 1], Malformed);
    op_err(&[0x14], Malformed);
    // The withheld timer and the disclosed "no timer" differ in bytes.
    let withheld = c3::encode_delta(&dfr(vec![host_facts(true, true, None, None)])).unwrap();
    let none = c3::encode_delta(&dfr(vec![host_facts(true, true, None, Some(None))])).unwrap();
    assert_ne!(withheld, none);
    let w = c3::encode_delta(&dfr(vec![DeltaOp::Clock { paused: Some(None) }])).unwrap();
    let r = c3::encode_delta(&dfr(vec![DeltaOp::Clock { paused: None }])).unwrap();
    assert_ne!(w, r);
    // Toggles.
    assert_eq!(p3::AUTO_PAUSE_TOGGLE_MASK, 0x1F);
    for b in 0..=255u8 {
        let t = AutoPauseToggles::from_bits(b);
        assert_eq!(t.is_some(), b <= 0x1F, "{b:#x}");
        if let Some(t) = t {
            assert_eq!(t.bits(), b);
            for (n, trig) in TRIGGERS.into_iter().enumerate() {
                assert_eq!(t.get(trig), b & (1 << n) != 0);
            }
        }
    }
    let base = AutoPauseToggles::default();
    assert_eq!(base.bits(), 0);
    assert_eq!(base.with(AutoPauseTrigger::OrderEnded, true).bits(), 0x08);
    for (n, trig) in TRIGGERS.into_iter().enumerate() {
        let on = base.with(trig, true);
        assert_eq!(on.bits(), 1 << n);
        assert!(on.get(trig));
        assert_eq!(on.with(trig, false), base);
        assert_eq!(on.with(trig, true), on);
        for other in TRIGGERS.into_iter().filter(|o| *o != trig) {
            assert!(!on.get(other));
        }
    }
    let all = TRIGGERS.into_iter().fold(base, |t, g| t.with(g, true));
    assert_eq!(all.bits(), 0x1F);
    // Reason tags, enum helpers.
    assert_eq!(PauseReason::Player.tag(), 0);
    assert_eq!(
        PauseReason::Auto(AutoPauseTrigger::PartyMemberDied).tag(),
        1
    );
    assert_eq!(PauseReason::Operator.tag(), 2);
    assert_eq!(PauseReason::Restore.tag(), 3);
    for n in 0..=255u8 {
        assert_eq!(AutoPauseTrigger::from_u8(n).is_some(), n <= 4);
        assert_eq!(EncounterMode::from_u8(n).is_some(), n <= 1);
    }
    for (n, t) in TRIGGERS.into_iter().enumerate() {
        assert_eq!(t.as_u8(), n as u8);
        assert_eq!(AutoPauseTrigger::from_u8(n as u8), Some(t));
    }
    assert_eq!(EncounterMode::TurnBased.as_u8(), 0);
    assert_eq!(EncounterMode::RealTime.as_u8(), 1);
    assert_eq!(EncounterMode::from_u8(0), Some(EncounterMode::TurnBased));
    assert_eq!(EncounterMode::from_u8(1), Some(EncounterMode::RealTime));
    assert!(EncounterMode::TurnBased < EncounterMode::RealTime);
}

// ---------------------------------------------------------------------------
// A8
// ---------------------------------------------------------------------------

#[test]
fn v3_tag_table_closed() {
    assert_eq!(p3::PROTOCOL_VERSION, 3);
    let intent_tags = [
        p3::INTENT_TAG_DECLARE_ACTION,
        p3::INTENT_TAG_END_TURN,
        p3::INTENT_TAG_STOP,
        p3::INTENT_TAG_PAUSE,
        p3::INTENT_TAG_UNPAUSE,
        p3::INTENT_TAG_REQUEST_MODE,
    ];
    assert_eq!(intent_tags, [0, 1, 2, 3, 4, 5]);
    let delta_tags = [
        p3::DELTA_TAG_ENCOUNTER,
        p3::DELTA_TAG_TIMELINE,
        p3::DELTA_TAG_ENTITY_FACTS,
        p3::DELTA_TAG_ROUND_STARTED,
        p3::DELTA_TAG_MODE_CHANGED,
        p3::DELTA_TAG_ORDER_ENDED,
        p3::DELTA_TAG_JOINED,
        p3::DELTA_TAG_LEFT,
        p3::DELTA_TAG_CLOCK,
        p3::DELTA_TAG_HOST_FACTS,
    ];
    assert_eq!(delta_tags, [11, 12, 13, 14, 15, 16, 17, 18, 19, 20]);
    assert_eq!(p3::AUTO_PAUSE_TOGGLE_MASK, 0x1F);
    assert_eq!(p3::POLICY_CLOCK_HEARTBEAT_TICKS, 100);
    const { assert!(p3::POLICY_CLOCK_HEARTBEAT_TICKS < p1::POLICY_OBSERVED_TICK_WINDOW) };
    // Everything outside 0..=10 and 14..=20 is unknown in T140a, 11..=13 too.
    for tag in (11..=13u8).chain(21..=255) {
        op_err(&[tag], CodecError::UnknownMessage);
        op_err(&[tag, 1, 1, 1, 1, 1, 1], CodecError::UnknownMessage);
    }
    // The tag is checked before the count of later ops is considered.
    let two = [3, 0, 0x64, 1, 2, 0x0E, 0, 0xFF];
    assert_eq!(c3::decode_delta(&two), Err(CodecError::UnknownMessage));
    let good = [3, 0, 0x64, 1, 2, 0x0E, 0, 0x0E, 1];
    assert_eq!(c3::decode_delta(&good).unwrap().ops.len(), 2);
    // v2 has no tag 0E.
    let v2_frame = [2, 0, 0x64, 1, 1, 0x0E, 3];
    assert_eq!(v2::decode_delta(&v2_frame), Err(CodecError::UnknownMessage));
    let v1_frame = [1, 0, 0x64, 1, 1, 0x0E, 3];
    assert_eq!(v1::decode_delta(&v1_frame), Err(CodecError::UnknownMessage));
}

// ---------------------------------------------------------------------------
// A9
// ---------------------------------------------------------------------------

#[test]
fn v3_delta_bounds() {
    use CodecError::*;
    assert_eq!(c3::encode_delta(&dfr(vec![])), Ok(vec![3, 0, 0x64, 1, 0]));
    assert_eq!(c3::decode_delta(&[3, 0, 0x64, 1, 0]), Ok(dfr(vec![])));
    // 256 ops accepted: count varint `80 02`.
    let rs = DeltaOp::RoundStarted { round: 0 };
    let full = dfr(vec![rs.clone(); 256]);
    let bytes = [&[3, 0, 0x64, 1, 0x80, 0x02][..], &[0x0E, 0].repeat(256)].concat();
    assert_eq!(c3::encode_delta(&full), Ok(bytes.clone()));
    assert_eq!(c3::decode_delta(&bytes), Ok(full));
    // 257: refused before any op is read, and on encode.
    assert_eq!(
        c3::decode_delta(&[3, 0, 0x64, 1, 0x81, 0x02]),
        Err(LimitExceeded)
    );
    let with_ops = [&[3, 0, 0x64, 1, 0x81, 0x02][..], &[0x0E, 0].repeat(257)].concat();
    assert_eq!(c3::decode_delta(&with_ops), Err(LimitExceeded));
    let junk = [3, 0, 0x64, 1, 0x81, 0x02, 0xFF];
    assert_eq!(c3::decode_delta(&junk), Err(LimitExceeded));
    assert_eq!(c3::encode_delta(&dfr(vec![rs; 257])), Err(LimitExceeded));
    // Much larger counts, and a count whose ops are missing.
    assert_eq!(
        c3::decode_delta(&[3, 0, 0x64, 1, 0xFF, 0x7F]),
        Err(LimitExceeded)
    );
    assert_eq!(c3::decode_delta(&[3, 0, 0x64, 1, 1]), Err(Malformed));
    assert_eq!(
        c3::decode_delta(&[3, 0, 0x64, 1, 2, 0x0E, 0]),
        Err(Malformed)
    );
    // Lane, trailing bytes, extremes.
    assert_eq!(c3::decode_delta(&[3, 1, 0x64, 1, 0]), Err(WrongDirection));
    assert_eq!(
        c3::decode_delta(&[3, 0xFF, 0x64, 1, 0]),
        Err(WrongDirection)
    );
    assert_eq!(
        c3::decode_delta(&[3, 1, 0x64, 1, 0x81, 0x02]),
        Err(WrongDirection)
    );
    assert_eq!(c3::decode_delta(&[3, 0, 0x64, 1, 0, 0]), Err(Malformed));
    assert_eq!(
        c3::decode_delta(&[3, 0, 0x64, 1, 1, 0x0E, 0, 0]),
        Err(Malformed)
    );
    assert_eq!(c3::decode_delta(&[3, 0, 0x64, 1]), Err(Malformed));
    assert_eq!(c3::decode_delta(&[3, 0, 0x64]), Err(Malformed));
    assert_eq!(c3::decode_delta(&[3, 0]), Err(Malformed));
    assert_eq!(c3::decode_delta(&[3]), Err(Malformed));
    let mut f = dfr(vec![]);
    f.event_seq = u64::MAX;
    f.server_tick = u64::MAX;
    let want = [&[3, 0][..], &MAX64, &MAX64, &[0]].concat();
    assert_eq!(c3::encode_delta(&f), Ok(want.clone()));
    assert_eq!(c3::decode_delta(&want), Ok(f));
    f = dfr(vec![]);
    f.server_tick = 0;
    f.event_seq = 0;
    assert_eq!(c3::encode_delta(&f), Ok(vec![3, 0, 0, 0, 0]));
    assert_eq!(c3::decode_delta(&[3, 0, 0, 0, 0]), Ok(f.clone()));
    f.lane = 1;
    assert_eq!(c3::encode_delta(&f), Err(WrongDirection));
}

// ---------------------------------------------------------------------------
// A10
// ---------------------------------------------------------------------------

fn all_variants() -> Vec<DeltaOp> {
    let mut ops: Vec<DeltaOp> = legacy_ops().into_iter().map(DeltaOp::from).collect();
    ops.push(receipt(
        0,
        p1::SEQ_LAST,
        ReceiptStatus::Rejected(RejectionCode::NoEntrants),
    ));
    ops.push(DeltaOp::RoundStarted { round: 9 });
    ops.push(DeltaOp::ModeChanged {
        mode: EncounterMode::RealTime,
        round: 9,
    });
    ops.push(DeltaOp::OrderEnded { actor: net(3) });
    ops.push(DeltaOp::Joined { entity: net(4) });
    ops.push(DeltaOp::Left { entity: net(5) });
    ops.push(DeltaOp::Clock { paused: None });
    ops.push(DeltaOp::Clock { paused: Some(None) });
    ops.push(DeltaOp::Clock {
        paused: Some(Some(PauseReason::Auto(AutoPauseTrigger::ModeChanged))),
    });
    ops.push(host_facts(true, false, Some(5), Some(None)));
    ops.push(host_facts(false, true, None, None));
    ops
}

#[test]
fn v3_round_trip_identity() {
    for body in [
        IntentBody::DeclareAction {
            ability: ulid(0xABCDEF),
            target: net(9),
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
    ] {
        let f = ifr(body);
        let a = c3::encode_intent(&f).unwrap();
        assert_eq!(a, c3::encode_intent(&f).unwrap());
        assert_eq!(c3::decode_intent(&a), Ok(f));
    }
    for op in all_variants() {
        let f = dfr(vec![op.clone()]);
        let a = c3::encode_delta(&f).unwrap();
        assert_eq!(a, c3::encode_delta(&f).unwrap());
        assert_eq!(c3::decode_delta(&a), Ok(f), "{op:?}");
        // Every strict prefix of a valid frame is truncation.
        for n in 1..a.len() {
            assert_eq!(
                c3::decode_delta(&a[..n]),
                Err(CodecError::Malformed),
                "{op:?} {n}"
            );
        }
    }
    let combined = dfr(all_variants());
    let bytes = c3::encode_delta(&combined).unwrap();
    assert_eq!(c3::decode_delta(&bytes), Ok(combined));

    // Case aliases decode and re-encode canonically, in intents and deltas.
    let id = ulid(0xABCD_EF01_2345_6789_ABCD);
    let text = id.to_string();
    assert!(text.bytes().any(|b| b.is_ascii_alphabetic()));
    let lower = text.to_ascii_lowercase();
    assert_ne!(text, lower);
    let body = |t: &str| [&[0][..], &[0x1A], t.as_bytes(), &[2]].concat();
    let canon = ib(&body(&text));
    let alias = ib(&body(&lower));
    let decoded = c3::decode_intent(&alias).unwrap();
    assert_eq!(
        decoded.body,
        IntentBody::DeclareAction {
            ability: id,
            target: net(2)
        }
    );
    assert_eq!(c3::encode_intent(&decoded), Ok(canon.clone()));
    assert_eq!(c3::decode_intent(&canon), Ok(decoded));
    let op = |t: &str| [&[8, 1, 2, 0x1A][..], t.as_bytes(), &[7], b"success", &[7]].concat();
    let d = c3::decode_delta(&d1(&op(&lower))).unwrap();
    assert_eq!(c3::encode_delta(&d), Ok(d1(&op(&text))));
}

// ---------------------------------------------------------------------------
// A11
// ---------------------------------------------------------------------------

#[test]
fn v3_delta_encode_checks() {
    use CodecError::*;
    let enc = |outcome: &str| c3::encode_delta(&dfr(vec![action(outcome)]));
    for ok in [
        "custom:255",
        "custom:0",
        "success",
        "failure",
        "critical_success",
        "critical_failure",
    ] {
        assert!(enc(ok).is_ok(), "{ok}");
    }
    assert_eq!(enc("bogus"), Err(Malformed));
    assert_eq!(enc("custom:256"), Err(Malformed));
    assert_eq!(enc("custom:01"), Err(Malformed));
    assert_eq!(enc("custom:"), Err(Malformed));
    assert_eq!(enc(""), Err(Malformed));
    assert_eq!(enc(&"a".repeat(256)), Err(Malformed));
    assert_eq!(enc(&"a".repeat(257)), Err(LimitExceeded));
    assert_eq!(
        enc(&format!("custom:{}", "1".repeat(250))),
        Err(LimitExceeded)
    );
    assert_eq!(enc(&format!("custom:{}", "1".repeat(249))), Err(Malformed));
    // Decode applies the same vocabulary and bound.
    let raw = |outcome: &[u8], len: &[u8]| [&[8, 1, 2][..], &utext(1), len, outcome, &[7]].concat();
    op_err(&raw(b"bogus", &[5]), Malformed);
    op_err(&raw(b"custom:256", &[10]), Malformed);
    assert!(c3::decode_delta(&d1(&raw(b"custom:255", &[10]))).is_ok());
    op_err(&raw(&[], &[0x81, 0x02]), LimitExceeded);
    // Order of checks.
    let bogus = action("bogus");
    let bad_receipt = receipt(1, 1, ReceiptStatus::Applied);
    let mut f = dfr(vec![bogus.clone(); 257]);
    f.lane = 1;
    assert_eq!(c3::encode_delta(&f), Err(WrongDirection));
    assert_eq!(
        c3::encode_delta(&dfr(vec![bogus.clone(); 257])),
        Err(LimitExceeded)
    );
    assert_eq!(
        c3::encode_delta(&dfr(vec![bogus.clone(), bad_receipt.clone()])),
        Err(Malformed)
    );
    assert_eq!(
        c3::encode_delta(&dfr(vec![bad_receipt, bogus])),
        Err(WrongDirection)
    );
    let over = action(&"a".repeat(257));
    let seq0 = receipt(0, 0, ReceiptStatus::Applied);
    assert_eq!(
        c3::encode_delta(&dfr(vec![seq0.clone(), over.clone()])),
        Err(Malformed)
    );
    assert_eq!(c3::encode_delta(&dfr(vec![over, seq0])), Err(LimitExceeded));
}

// ---------------------------------------------------------------------------
// Review round: flag bytes must be exactly 00/01 even with a full remainder.
// ---------------------------------------------------------------------------

#[test]
fn v3_flag_bytes_strict_with_full_remainder() {
    use CodecError::*;
    for bad in [2u8, 0xFF] {
        // HostFacts auto_disclosed: bad flag, then bits and a complete timer.
        op_err(&[0x14, 1, 1, bad, 0x1F, 1, 1, 5], Malformed);
        op_err(&[0x14, 1, 1, bad, 0x1F, 0], Malformed);
        // HostFacts timer_disclosed: bad flag, then a complete running timer.
        op_err(&[0x14, 1, 1, 0, bad, 1, 5], Malformed);
        op_err(&[0x14, 1, 1, 0, bad, 0], Malformed);
        // HostFacts timer_running: bad flag, then a full u32.
        op_err(&[0x14, 1, 1, 0, 1, bad, 5], Malformed);
        // Turn present flag: bad flag, then entity and round.
        op_err(&[0x06, bad, 1, 5], Malformed);
        op_err(&[0x06, bad, 5], Malformed);
    }
    // Passing controls: the identical frames with 01.
    check_op(
        host_facts(true, true, Some(0x1F), Some(Some(5))),
        &[0x14, 1, 1, 1, 0x1F, 1, 1, 5],
    );
    check_op(
        host_facts(true, true, None, Some(Some(5))),
        &[0x14, 1, 1, 0, 1, 1, 5],
    );
    check_op(
        host_facts(true, true, None, Some(Some(5))),
        &[0x14, 1, 1, 0, 1, 1, 5],
    );
    check_op(
        DeltaOp::Turn {
            active: Some(net(1)),
            round: 5,
        },
        &[0x06, 1, 1, 5],
    );
    // A HostFacts timer varint above u32::MAX is Malformed; u32::MAX passes.
    op_err(
        &[0x14, 1, 1, 0, 1, 1, 0x80, 0x80, 0x80, 0x80, 0x10],
        Malformed,
    );
    op_err(
        &[0x14, 1, 1, 0, 1, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0x1F],
        Malformed,
    );
    check_op(
        host_facts(true, true, None, Some(Some(u32::MAX))),
        &[0x14, 1, 1, 0, 1, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
    );
}

#[test]
fn v3_toggles_with_false_on_clear_bit() {
    let base = AutoPauseToggles::default();
    for t in TRIGGERS {
        // Clearing a bit that is already clear changes nothing.
        assert_eq!(base.with(t, false).bits(), 0);
        let others = TRIGGERS
            .into_iter()
            .filter(|o| *o != t)
            .fold(base, |acc, o| acc.with(o, true));
        let expected = 0x1F & !(1u8 << t.as_u8());
        assert_eq!(others.bits(), expected);
        assert_eq!(others.with(t, false).bits(), expected);
        assert!(!others.with(t, false).get(t));
        // Setting then clearing, and clearing twice, are stable.
        assert_eq!(others.with(t, true).bits(), 0x1F);
        assert_eq!(others.with(t, true).with(t, false).bits(), expected);
        assert_eq!(others.with(t, false).with(t, false).bits(), expected);
    }
}
