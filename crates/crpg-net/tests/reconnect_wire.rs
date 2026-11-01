//! T025a1 resume wire suite (`tasks/T025a.md` S§11, cases 1–10), public API
//! only, pure functions (no pool, no clock), plus the T025a2 mechanisms
//! (cases 11–21) with their test-local pool, replica, oracle and host
//! stand-in.
//!
//! Wire bytes for the codec cases are hand-written from S§6 and are never
//! produced by the codec under test. Requests and tickets are compared
//! through `as_bytes()` and by re-encoding, because they have no `PartialEq`.
//! The stand-in performs no authentication: its decision is test input, and
//! no case here claims to prove authentication or host resume.

#[allow(dead_code)]
#[path = "support/driver.rs"]
mod driver;
#[allow(dead_code)]
#[path = "support/fixture.rs"]
mod fixture;

use core::marker::PhantomData;
use std::collections::BTreeMap;

use crpg_core::EntityId;
use crpg_net::codec;
use crpg_net::codec_resume::{
    decode_down, decode_request, encode_down, encode_request, CodecError,
};
use crpg_net::codec_transfer;
use crpg_net::codec_v2;
use crpg_net::protocol::{
    self, DeltaFrame, DeltaOp, IntentBody, IntentFrame, NetId, ReceiptStatus, RejectionCode,
    LANE_COMBAT, MAX_INTENT_FRAME_BYTES, SEQ_LAST,
};
use crpg_net::protocol_resume::*;
use crpg_net::protocol_transfer::{
    self, CancelReason, HealthFact, ResyncReason, SnapshotBody, SnapshotEntity, TransferChunk,
    TransferDown, TransferId, TransferUp, TurnFact,
};
use crpg_net::protocol_v2;
use crpg_net::reassembly::{
    DeltaOutcome, DeltaWire, GateMode, GatedDelta, ReassemblyPool, Rebound, ReceiverId,
    ReplicaUpdate, TransferError, TransferLimits, TransferOutcome,
};
use crpg_net::resume::{check_result, ResumeDecision, ResumeError, SessionFence};
use crpg_net::sim::{InMemoryTransport, PeerId, QueueCaps, RateCaps, SimClock};
use driver::{NetDriver, OwnershipTable, Visibility, VisibilityTable};
use fixture::Fixture;

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

const ZERO_EPOCH: [u8; 16] = [0x00; 16];
const FF_EPOCH: [u8; 16] = [0xFF; 16];
/// `P`: 16 × `AA`.
const PROOF: [u8; 16] = [0xAA; 16];

fn fence(raw: u64) -> ConnectionFence {
    ConnectionFence::new(raw).expect("fixture fence is nonzero")
}

fn proof(bytes: &[u8]) -> ResumeProof {
    ResumeProof::new(bytes.to_vec()).expect("fixture proof is 16..=128 bytes")
}

fn claim(lane: u8, through: u64) -> LaneClaim {
    LaneClaim {
        lane,
        receipted_through: through,
    }
}

fn progress(lane: u8, through: u64) -> LaneProgress {
    LaneProgress {
        lane,
        finalized_through: through,
    }
}

/// LEB128, written here so the codec is never its own oracle.
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

/// A ten-byte varint whose last byte overflows `u64`.
fn overflow_varint() -> Vec<u8> {
    let mut v = vec![0xFF; 9];
    v.push(0x02);
    v
}

/// The error half of `decode_request`, so a refusal can be compared without
/// `ResumeRequest: PartialEq`.
fn request_err(bytes: &[u8]) -> Option<CodecError> {
    decode_request(bytes).err()
}

/// The error half of `decode_down`, so a refusal can be compared without
/// `ResumeDown: PartialEq`.
fn down_err(bytes: &[u8]) -> Option<CodecError> {
    decode_down(bytes).err()
}

/// The S§6.4 literal frames.
fn l1_bytes() -> Vec<u8> {
    let mut b = vec![0x91, 0x03, 0x40];
    b.extend_from_slice(&ZERO_EPOCH);
    b.push(0x01); // fence 1
    b.push(0x10); // proof_len 16
    b.extend_from_slice(&PROOF);
    b.push(0x01); // lane_count 1
    b.push(0x00); // lane 0
    b.push(0x00); // through 0
    b
}

fn l2_bytes() -> Vec<u8> {
    let mut b = vec![0x91, 0x03, 0x40];
    b.extend_from_slice(&ZERO_EPOCH);
    b.extend_from_slice(&[0xAC, 0x02]); // fence 300
    b.push(0x10);
    b.extend_from_slice(&PROOF);
    b.push(0x01);
    b.push(0x00);
    b.extend_from_slice(&[0x84, 0x07]); // through 900
    b
}

fn l3_bytes() -> Vec<u8> {
    let mut b = vec![0x91, 0x03, 0x30];
    b.extend_from_slice(&ZERO_EPOCH);
    b.push(0x10);
    b.extend_from_slice(&PROOF);
    b
}

fn l4_bytes() -> Vec<u8> {
    let mut b = vec![0x91, 0x03, 0x31];
    b.extend_from_slice(&ZERO_EPOCH);
    b.push(0x02); // granted 2
    b.push(0x01); // lane_count 1
    b.push(0x00);
    b.push(0x00);
    b
}

fn l5_bytes() -> Vec<u8> {
    let mut b = vec![0x91, 0x03, 0x31];
    b.extend_from_slice(&ZERO_EPOCH);
    b.extend_from_slice(&[0xAD, 0x02]); // granted 301
    b.push(0x01);
    b.push(0x00);
    b.extend_from_slice(&[0x84, 0x07]); // through 900
    b
}

fn l6_bytes() -> Vec<u8> {
    let mut b = vec![0x91, 0x03, 0x32];
    b.extend_from_slice(&ZERO_EPOCH);
    b.push(0x01); // Denied
    b
}

fn l7_bytes() -> Vec<u8> {
    let mut b = vec![0x91, 0x03, 0x32];
    b.extend_from_slice(&ZERO_EPOCH);
    b.push(0x02); // Busy
    b
}

/// The S§6.4 literal values.
fn request_l1() -> ResumeRequest {
    ResumeRequest {
        epoch: ZERO_EPOCH,
        fence: fence(1),
        proof: proof(&PROOF),
        claims: vec![claim(0, 0)],
    }
}

fn request_l2() -> ResumeRequest {
    ResumeRequest {
        epoch: ZERO_EPOCH,
        fence: fence(300),
        proof: proof(&PROOF),
        claims: vec![claim(0, 900)],
    }
}

fn ticket_l3() -> ResumeDown {
    ResumeDown::Ticket(ResumeTicket {
        epoch: ZERO_EPOCH,
        proof: proof(&PROOF),
    })
}

fn accepted_l4() -> ResumeDown {
    ResumeDown::Answer(ResumeResult::Accepted(ResumeAccepted {
        epoch: ZERO_EPOCH,
        granted: fence(2),
        progress: vec![progress(0, 0)],
    }))
}

fn accepted_l5() -> ResumeDown {
    ResumeDown::Answer(ResumeResult::Accepted(ResumeAccepted {
        epoch: ZERO_EPOCH,
        granted: fence(301),
        progress: vec![progress(0, 900)],
    }))
}

fn rejected_l6() -> ResumeDown {
    ResumeDown::Answer(ResumeResult::Rejected(ResumeRejected {
        epoch: ZERO_EPOCH,
        reason: ResumeRejection::Denied,
    }))
}

fn rejected_l7() -> ResumeDown {
    ResumeDown::Answer(ResumeResult::Rejected(ResumeRejected {
        epoch: ZERO_EPOCH,
        reason: ResumeRejection::Busy,
    }))
}

fn assert_request_eq(
    actual: &ResumeRequest,
    epoch: [u8; 16],
    fence_raw: u64,
    proof_bytes: &[u8],
    claims: &[LaneClaim],
) {
    assert_eq!(actual.epoch, epoch);
    assert_eq!(actual.fence.get(), fence_raw);
    assert_eq!(actual.proof.as_bytes(), proof_bytes);
    assert_eq!(actual.claims, claims);
}

fn assert_answer(actual: &ResumeDown, expected: &ResumeResult) {
    match actual {
        ResumeDown::Answer(result) => assert_eq!(result, expected),
        other => panic!("expected Answer, got {other:?}"),
    }
}

fn assert_ticket(actual: &ResumeDown, epoch: [u8; 16], proof_bytes: &[u8]) {
    match actual {
        ResumeDown::Ticket(ticket) => {
            assert_eq!(ticket.epoch, epoch);
            assert_eq!(ticket.proof.as_bytes(), proof_bytes);
        }
        other => panic!("expected Ticket, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Case 1.
// ---------------------------------------------------------------------------

#[test]
fn resume_identifiers_and_caps_are_pinned() {
    // S§9.1 constants.
    assert_eq!(RESUME_VERSION, 0x91);
    assert_eq!(LANE_SESSION, 3);
    assert_eq!(RESUME_TAG_TICKET, 0x30);
    assert_eq!(RESUME_TAG_ACCEPTED, 0x31);
    assert_eq!(RESUME_TAG_REJECTED, 0x32);
    assert_eq!(RESUME_TAG_REQUEST, 0x40);
    assert_eq!(MIN_RESUME_PROOF_BYTES, 16);
    assert_eq!(MAX_RESUME_PROOF_BYTES, 128);
    assert_eq!(MAX_RESUME_LANES, 4);

    assert_eq!(MAX_RESUME_FRAME_BYTES, MAX_INTENT_FRAME_BYTES);
    assert_eq!(MAX_RESUME_FRAME_BYTES, 4_096);

    assert_eq!(ConnectionFence::FIRST.get(), 1);
    assert_eq!(ConnectionFence::new(0), None);
    let max_fence = ConnectionFence::new(u64::MAX).expect("u64::MAX is a valid fence");
    assert_eq!(max_fence.get(), u64::MAX);

    // First byte outside {1, 2, 0x81, 0x82}; family nibbles disjoint.
    assert_ne!(RESUME_VERSION, protocol::PROTOCOL_VERSION);
    assert_ne!(RESUME_VERSION, protocol_v2::PROTOCOL_VERSION);
    assert_ne!(RESUME_VERSION, protocol_transfer::TRANSFER_VERSION);
    assert_ne!(RESUME_VERSION, 0x82);
    assert_eq!(RESUME_VERSION & 0xF0, 0x90);
    assert_eq!(protocol_transfer::TRANSFER_VERSION & 0xF0, 0x80);

    // Lane outside {0, 1, 2}.
    assert_ne!(LANE_SESSION, 0);
    assert_ne!(LANE_SESSION, 1);
    assert_ne!(LANE_SESSION, 2);

    // Resume tags share nothing with the intent, delta or transfer tags.
    let resume_tags = [
        RESUME_TAG_TICKET,
        RESUME_TAG_ACCEPTED,
        RESUME_TAG_REJECTED,
        RESUME_TAG_REQUEST,
    ];
    let intent_tags = [
        protocol::INTENT_TAG_DECLARE_ACTION,
        protocol::INTENT_TAG_END_TURN,
    ];
    let delta_tags: Vec<u8> = (0..=10).collect();
    let transfer_tags = [
        protocol_transfer::TRANSFER_TAG_BEGIN,
        protocol_transfer::TRANSFER_TAG_CHUNK,
        protocol_transfer::TRANSFER_TAG_CANCEL,
        protocol_transfer::TRANSFER_TAG_PUBLISHED,
        protocol_transfer::TRANSFER_TAG_RESYNC,
    ];
    for resume_tag in resume_tags {
        for other in intent_tags
            .iter()
            .chain(delta_tags.iter())
            .chain(transfer_tags.iter())
        {
            assert_ne!(resume_tag, *other, "resume tag {resume_tag:#04x} collides");
        }
    }

    // Rejection mapping: Some exactly for 1 and 2, and as_u8 round-trips.
    for raw in 0..=255u8 {
        match raw {
            1 => {
                assert_eq!(ResumeRejection::from_u8(raw), Some(ResumeRejection::Denied));
                assert_eq!(ResumeRejection::Denied.as_u8(), 1);
            }
            2 => {
                assert_eq!(ResumeRejection::from_u8(raw), Some(ResumeRejection::Busy));
                assert_eq!(ResumeRejection::Busy.as_u8(), 2);
            }
            _ => assert_eq!(ResumeRejection::from_u8(raw), None),
        }
    }

    // Existing values unchanged.
    assert_eq!(protocol::PROTOCOL_VERSION, 1);
    assert_eq!(protocol_v2::PROTOCOL_VERSION, 2);
    assert_eq!(protocol_transfer::TRANSFER_VERSION, 0x81);
    assert_eq!(LANE_COMBAT, 0);
    assert_eq!(protocol_transfer::LANE_SNAPSHOT, 2);
}

// ---------------------------------------------------------------------------
// Case 2.
// ---------------------------------------------------------------------------

#[test]
fn resume_literal_byte_oracles() {
    // L1.
    let l1 = l1_bytes();
    assert_eq!(l1.len(), 40);
    assert_eq!(encode_request(&request_l1()).unwrap(), l1);
    let d1 = decode_request(&l1).unwrap();
    assert_request_eq(&d1, ZERO_EPOCH, 1, &PROOF, &[claim(0, 0)]);
    assert_eq!(encode_request(&d1).unwrap(), l1);

    // L2 (fence 300 -> AC 02, claim 900 -> 84 07).
    assert_eq!(varint(300), [0xAC, 0x02]);
    assert_eq!(varint(900), [0x84, 0x07]);
    let l2 = l2_bytes();
    assert_eq!(l2.len(), 42);
    assert_eq!(encode_request(&request_l2()).unwrap(), l2);
    let d2 = decode_request(&l2).unwrap();
    assert_request_eq(&d2, ZERO_EPOCH, 300, &PROOF, &[claim(0, 900)]);
    assert_eq!(encode_request(&d2).unwrap(), l2);

    // L3 Ticket.
    let l3 = l3_bytes();
    assert_eq!(l3.len(), 36);
    assert_eq!(encode_down(&ticket_l3()).unwrap(), l3);
    assert_ticket(&decode_down(&l3).unwrap(), ZERO_EPOCH, &PROOF);
    assert_eq!(encode_down(&decode_down(&l3).unwrap()).unwrap(), l3);

    // L4 Accepted.
    let l4 = l4_bytes();
    assert_eq!(l4.len(), 23);
    assert_eq!(encode_down(&accepted_l4()).unwrap(), l4);
    assert_answer(
        &decode_down(&l4).unwrap(),
        &ResumeResult::Accepted(ResumeAccepted {
            epoch: ZERO_EPOCH,
            granted: fence(2),
            progress: vec![progress(0, 0)],
        }),
    );
    assert_eq!(encode_down(&decode_down(&l4).unwrap()).unwrap(), l4);

    // L5 Accepted (granted 301 -> AD 02).
    assert_eq!(varint(301), [0xAD, 0x02]);
    let l5 = l5_bytes();
    assert_eq!(l5.len(), 25);
    assert_eq!(encode_down(&accepted_l5()).unwrap(), l5);
    assert_answer(
        &decode_down(&l5).unwrap(),
        &ResumeResult::Accepted(ResumeAccepted {
            epoch: ZERO_EPOCH,
            granted: fence(301),
            progress: vec![progress(0, 900)],
        }),
    );
    assert_eq!(encode_down(&decode_down(&l5).unwrap()).unwrap(), l5);

    // L6 Rejected Denied.
    let l6 = l6_bytes();
    assert_eq!(l6.len(), 20);
    assert_eq!(encode_down(&rejected_l6()).unwrap(), l6);
    assert_answer(
        &decode_down(&l6).unwrap(),
        &ResumeResult::Rejected(ResumeRejected {
            epoch: ZERO_EPOCH,
            reason: ResumeRejection::Denied,
        }),
    );
    assert_eq!(encode_down(&decode_down(&l6).unwrap()).unwrap(), l6);

    // L7 Rejected Busy.
    let l7 = l7_bytes();
    assert_eq!(l7.len(), 20);
    assert_eq!(encode_down(&rejected_l7()).unwrap(), l7);
    assert_answer(
        &decode_down(&l7).unwrap(),
        &ResumeResult::Rejected(ResumeRejected {
            epoch: ZERO_EPOCH,
            reason: ResumeRejection::Busy,
        }),
    );
    assert_eq!(encode_down(&decode_down(&l7).unwrap()).unwrap(), l7);
}

// ---------------------------------------------------------------------------
// Case 3.
// ---------------------------------------------------------------------------

#[test]
fn resume_frame_sizes_at_the_bounds() {
    let proof128 = vec![0xAB; 128];

    // Largest request: 170 bytes.
    let largest_request = ResumeRequest {
        epoch: FF_EPOCH,
        fence: fence(u64::MAX),
        proof: proof(&proof128),
        claims: vec![claim(0, SEQ_LAST)],
    };
    let bytes = encode_request(&largest_request).unwrap();
    assert_eq!(bytes.len(), 170);
    let decoded = decode_request(&bytes).unwrap();
    assert_request_eq(
        &decoded,
        FF_EPOCH,
        u64::MAX,
        &proof128,
        &[claim(0, SEQ_LAST)],
    );

    // Smallest request: L1, 40 bytes.
    let smallest_request = request_l1();
    let bytes = encode_request(&smallest_request).unwrap();
    assert_eq!(bytes.len(), 40);
    assert_request_eq(
        &decode_request(&bytes).unwrap(),
        ZERO_EPOCH,
        1,
        &PROOF,
        &[claim(0, 0)],
    );

    // Largest ticket: 148 bytes.
    let largest_ticket = ResumeDown::Ticket(ResumeTicket {
        epoch: FF_EPOCH,
        proof: proof(&proof128),
    });
    let bytes = encode_down(&largest_ticket).unwrap();
    assert_eq!(bytes.len(), 148);
    assert_ticket(&decode_down(&bytes).unwrap(), FF_EPOCH, &proof128);

    // Smallest ticket: L3, 36 bytes.
    let bytes = encode_down(&ticket_l3()).unwrap();
    assert_eq!(bytes.len(), 36);
    assert_ticket(&decode_down(&bytes).unwrap(), ZERO_EPOCH, &PROOF);

    // Largest accepted: 41 bytes.
    let largest_accepted = ResumeDown::Answer(ResumeResult::Accepted(ResumeAccepted {
        epoch: FF_EPOCH,
        granted: fence(u64::MAX),
        progress: vec![progress(0, SEQ_LAST)],
    }));
    let bytes = encode_down(&largest_accepted).unwrap();
    assert_eq!(bytes.len(), 41);
    assert_answer(
        &decode_down(&bytes).unwrap(),
        &ResumeResult::Accepted(ResumeAccepted {
            epoch: FF_EPOCH,
            granted: fence(u64::MAX),
            progress: vec![progress(0, SEQ_LAST)],
        }),
    );

    // Smallest accepted: L4, 23 bytes.
    let bytes = encode_down(&accepted_l4()).unwrap();
    assert_eq!(bytes.len(), 23);
    assert_answer(
        &decode_down(&bytes).unwrap(),
        &ResumeResult::Accepted(ResumeAccepted {
            epoch: ZERO_EPOCH,
            granted: fence(2),
            progress: vec![progress(0, 0)],
        }),
    );

    // Rejected: always 20 bytes.
    assert_eq!(encode_down(&rejected_l6()).unwrap().len(), 20);
    assert_eq!(encode_down(&rejected_l7()).unwrap().len(), 20);
    assert_answer(
        &decode_down(&l6_bytes()).unwrap(),
        &ResumeResult::Rejected(ResumeRejected {
            epoch: ZERO_EPOCH,
            reason: ResumeRejection::Denied,
        }),
    );
}

// ---------------------------------------------------------------------------
// Case 4.
// ---------------------------------------------------------------------------

#[test]
fn request_decode_precedence() {
    // The cap comes before the version.
    let too_large = vec![0x01u8; 4_097];
    assert_eq!(request_err(&too_large), Some(CodecError::FrameTooLarge));

    // Exactly 4,096 bytes made of L1 plus padding: Malformed, not FrameTooLarge.
    let mut padded = l1_bytes();
    padded.resize(4_096, 0x00);
    assert_eq!(request_err(&padded), Some(CodecError::Malformed));

    // Empty and truncated headers.
    assert_eq!(request_err(&[]), Some(CodecError::Malformed));
    assert_eq!(request_err(&[0x91]), Some(CodecError::Malformed));
    assert_eq!(request_err(&[0x91, 0x03]), Some(CodecError::Malformed));

    // Version bytes.
    for bad in [0x00u8, 0x01, 0x02, 0x81, 0x82, 0x90, 0x92, 0xFF] {
        let mut b = l1_bytes();
        b[0] = bad;
        assert_eq!(
            request_err(&b),
            Some(CodecError::UnsupportedVersion),
            "version {bad:#04x}"
        );
    }

    // Lane bytes.
    for bad in [0x00u8, 0x01, 0x02, 0x04, 0xFF] {
        let mut b = l1_bytes();
        b[1] = bad;
        assert_eq!(
            request_err(&b),
            Some(CodecError::WrongDirection),
            "lane {bad:#04x}"
        );
    }

    // Tags: the other direction's tags, then unknown tags.
    for bad in [0x30u8, 0x31, 0x32] {
        let mut b = l1_bytes();
        b[2] = bad;
        assert_eq!(
            request_err(&b),
            Some(CodecError::WrongDirection),
            "tag {bad:#04x}"
        );
    }
    for bad in [0x00u8, 0x10, 0x20, 0x21, 0x3F, 0x41, 0xFF] {
        let mut b = l1_bytes();
        b[2] = bad;
        assert_eq!(
            request_err(&b),
            Some(CodecError::UnknownMessage),
            "tag {bad:#04x}"
        );
    }

    // Combinations report the earliest stage.
    let mut bad_version_and_lane = l1_bytes();
    bad_version_and_lane[0] = 0x00;
    bad_version_and_lane[1] = 0x00;
    assert_eq!(
        request_err(&bad_version_and_lane),
        Some(CodecError::UnsupportedVersion)
    );
    let mut bad_lane_and_tag = l1_bytes();
    bad_lane_and_tag[1] = 0x00;
    bad_lane_and_tag[2] = 0x00;
    assert_eq!(
        request_err(&bad_lane_and_tag),
        Some(CodecError::WrongDirection)
    );

    // Controls.
    assert!(decode_request(&l1_bytes()).is_ok());
    assert!(decode_request(&l2_bytes()).is_ok());
}

// ---------------------------------------------------------------------------
// Case 5.
// ---------------------------------------------------------------------------

#[test]
fn down_decode_precedence() {
    // Cap before version.
    let too_large = vec![0x01u8; 4_097];
    assert_eq!(down_err(&too_large), Some(CodecError::FrameTooLarge));

    // Exactly 4,096 bytes made of L6 plus padding: Malformed.
    let mut padded = l6_bytes();
    padded.resize(4_096, 0x00);
    assert_eq!(down_err(&padded), Some(CodecError::Malformed));

    // Empty and truncated headers.
    assert_eq!(down_err(&[]), Some(CodecError::Malformed));
    assert_eq!(down_err(&[0x91]), Some(CodecError::Malformed));
    assert_eq!(down_err(&[0x91, 0x03]), Some(CodecError::Malformed));

    // Version bytes.
    for bad in [0x00u8, 0x01, 0x02, 0x81, 0x82, 0x90, 0x92, 0xFF] {
        let mut b = l6_bytes();
        b[0] = bad;
        assert_eq!(
            down_err(&b),
            Some(CodecError::UnsupportedVersion),
            "version {bad:#04x}"
        );
    }

    // Lane bytes.
    for bad in [0x00u8, 0x01, 0x02, 0x04, 0xFF] {
        let mut b = l6_bytes();
        b[1] = bad;
        assert_eq!(
            down_err(&b),
            Some(CodecError::WrongDirection),
            "lane {bad:#04x}"
        );
    }

    // Tags: upstream tag is WrongDirection; unknown tags are UnknownMessage.
    let mut upstream_tag = l6_bytes();
    upstream_tag[2] = 0x40;
    assert_eq!(down_err(&upstream_tag), Some(CodecError::WrongDirection));
    for bad in [0x00u8, 0x10, 0x2F, 0x33, 0x41, 0xFF] {
        let mut b = l6_bytes();
        b[2] = bad;
        assert_eq!(
            down_err(&b),
            Some(CodecError::UnknownMessage),
            "tag {bad:#04x}"
        );
    }

    // Rejected reason bytes.
    for bad in [0x00u8, 0x03, 0xFF] {
        let mut b = l6_bytes();
        b[19] = bad;
        assert_eq!(
            down_err(&b),
            Some(CodecError::UnknownMessage),
            "reason {bad:#04x}"
        );
    }

    // Controls.
    assert!(decode_down(&l3_bytes()).is_ok());
    assert!(decode_down(&l4_bytes()).is_ok());
    assert!(decode_down(&l5_bytes()).is_ok());
    assert!(decode_down(&l6_bytes()).is_ok());
    assert!(decode_down(&l7_bytes()).is_ok());
}

// ---------------------------------------------------------------------------
// Case 6.
// ---------------------------------------------------------------------------

/// Header (version, lane, tag) + zero epoch: the stable prefix for
/// request-field mutations.
fn request_header() -> Vec<u8> {
    let mut b = vec![0x91, 0x03, 0x40];
    b.extend_from_slice(&ZERO_EPOCH);
    b
}

/// `request_header` + fence 1 + proof_len 16 + proof: the prefix for lane-list
/// mutations.
fn request_with_proof() -> Vec<u8> {
    let mut b = request_header();
    b.push(0x01); // fence 1
    b.push(0x10); // proof_len 16
    b.extend_from_slice(&PROOF);
    b
}

#[test]
fn request_field_bounds() {
    // --- fence ---
    let mut fence_zero = request_header();
    fence_zero.push(0x00);
    fence_zero.push(0x10);
    fence_zero.extend_from_slice(&PROOF);
    fence_zero.extend_from_slice(&[0x01, 0x00, 0x00]);
    assert_eq!(request_err(&fence_zero), Some(CodecError::Malformed));

    let mut fence_max = request_header();
    fence_max.extend_from_slice(&[0xFF; 9]);
    fence_max.push(0x01);
    fence_max.push(0x10);
    fence_max.extend_from_slice(&PROOF);
    fence_max.extend_from_slice(&[0x01, 0x00, 0x00]);
    assert!(decode_request(&fence_max).is_ok());

    let mut fence_overflow = request_header();
    fence_overflow.extend_from_slice(&overflow_varint());
    fence_overflow.push(0x10);
    fence_overflow.extend_from_slice(&PROOF);
    fence_overflow.extend_from_slice(&[0x01, 0x00, 0x00]);
    assert_eq!(request_err(&fence_overflow), Some(CodecError::Malformed));

    // 0x80 followed by end of input: truncated varint.
    let mut fence_truncated = request_header();
    fence_truncated.push(0x80);
    assert_eq!(request_err(&fence_truncated), Some(CodecError::Malformed));

    // --- proof_len ---
    // 0x80 with 128 bytes (0x10 with 16 bytes is L1 itself).
    let mut proof128_ok = request_header();
    proof128_ok.push(0x01); // fence 1
    proof128_ok.push(0x80);
    proof128_ok.extend_from_slice(&[0xAB; 128]);
    proof128_ok.extend_from_slice(&[0x01, 0x00, 0x00]);
    assert!(decode_request(&proof128_ok).is_ok());

    for bad_len in [0x0Fu8, 0x00] {
        let mut b = request_header();
        b.push(0x01); // fence 1
        b.push(bad_len);
        assert_eq!(
            request_err(&b),
            Some(CodecError::Malformed),
            "proof_len {bad_len:#04x}"
        );
    }
    for bad_len in [0x81u8, 0xFF] {
        let mut b = request_header();
        b.push(0x01); // fence 1
        b.push(bad_len);
        assert_eq!(
            request_err(&b),
            Some(CodecError::LimitExceeded),
            "proof_len {bad_len:#04x}"
        );
    }

    // 0x10 followed by 15 bytes and end of input.
    let mut short_proof = request_header();
    short_proof.push(0x01); // fence 1
    short_proof.push(0x10);
    short_proof.extend_from_slice(&[0xAA; 15]);
    assert_eq!(request_err(&short_proof), Some(CodecError::Malformed));

    // --- lane_count and entries ---
    let mut count_zero = request_with_proof();
    count_zero.push(0x00);
    assert_eq!(request_err(&count_zero), Some(CodecError::Malformed));

    for bad_count in [0x05u8, 0xFF] {
        let mut b = request_with_proof();
        b.push(bad_count);
        assert_eq!(
            request_err(&b),
            Some(CodecError::LimitExceeded),
            "lane_count {bad_count:#04x}"
        );
    }

    let mut two_dupes = request_with_proof();
    two_dupes.extend_from_slice(&[0x02, 0x00, 0x00, 0x00, 0x00]);
    assert_eq!(request_err(&two_dupes), Some(CodecError::Malformed));

    let mut two_foreign = request_with_proof();
    two_foreign.extend_from_slice(&[0x02, 0x00, 0x00, 0x01, 0x00]);
    assert_eq!(request_err(&two_foreign), Some(CodecError::UnknownMessage));

    for foreign_lane in [0x01u8, 0x03] {
        let mut b = request_with_proof();
        b.extend_from_slice(&[0x01, foreign_lane, 0x00]);
        assert_eq!(
            request_err(&b),
            Some(CodecError::UnknownMessage),
            "lane {foreign_lane:#04x}"
        );
    }

    let mut truncated_second = request_with_proof();
    truncated_second.extend_from_slice(&[0x02, 0x00, 0x00]);
    assert_eq!(request_err(&truncated_second), Some(CodecError::Malformed));

    // --- receipted_through ---
    let mut through_max = request_with_proof();
    through_max.extend_from_slice(&[0x01, 0x00]);
    through_max.extend_from_slice(&varint(SEQ_LAST));
    assert!(decode_request(&through_max).is_ok());

    let mut through_overflow = request_with_proof();
    through_overflow.extend_from_slice(&[0x01, 0x00]);
    through_overflow.extend_from_slice(&varint(u64::MAX));
    assert_eq!(request_err(&through_overflow), Some(CodecError::Malformed));

    // --- prefixes and trailing bytes ---
    for lit in [l1_bytes(), l2_bytes()] {
        for len in 0..lit.len() {
            assert_eq!(request_err(&lit[..len]), Some(CodecError::Malformed));
        }
        let mut with_trailing = lit.clone();
        with_trailing.push(0x00);
        assert_eq!(request_err(&with_trailing), Some(CodecError::Malformed));
    }
}

// ---------------------------------------------------------------------------
// Case 7.
// ---------------------------------------------------------------------------

/// Header (version, lane, tag) + zero epoch: the stable prefix for ticket
/// (0x30) mutations.
fn ticket_header() -> Vec<u8> {
    let mut b = vec![0x91, 0x03, 0x30];
    b.extend_from_slice(&ZERO_EPOCH);
    b
}

/// Header + zero epoch: the stable prefix for accepted (0x31) granted and
/// progress-list mutations.
fn accepted_header() -> Vec<u8> {
    let mut b = vec![0x91, 0x03, 0x31];
    b.extend_from_slice(&ZERO_EPOCH);
    b
}

/// `accepted_header` + granted 2: the stable prefix for progress-list
/// mutations.
fn accepted_with_granted() -> Vec<u8> {
    let mut b = accepted_header();
    b.push(0x02); // granted 2
    b
}

#[test]
fn down_field_bounds() {
    // --- proof_len rows of case 6, applied to L3 ---
    let mut proof128_ok = ticket_header();
    proof128_ok.push(0x80);
    proof128_ok.extend_from_slice(&[0xAB; 128]);
    assert!(decode_down(&proof128_ok).is_ok());

    for bad_len in [0x0Fu8, 0x00] {
        let mut b = ticket_header();
        b.push(bad_len);
        assert_eq!(
            down_err(&b),
            Some(CodecError::Malformed),
            "proof_len {bad_len:#04x}"
        );
    }
    for bad_len in [0x81u8, 0xFF] {
        let mut b = ticket_header();
        b.push(bad_len);
        assert_eq!(
            down_err(&b),
            Some(CodecError::LimitExceeded),
            "proof_len {bad_len:#04x}"
        );
    }
    let mut short_proof = ticket_header();
    short_proof.push(0x10);
    short_proof.extend_from_slice(&[0xAA; 15]);
    assert_eq!(down_err(&short_proof), Some(CodecError::Malformed));

    // --- granted rows, applied to L4 ---
    let mut granted_zero = accepted_header();
    granted_zero.extend_from_slice(&[0x00, 0x01, 0x00, 0x00]);
    assert_eq!(down_err(&granted_zero), Some(CodecError::Malformed));

    let mut granted_one = accepted_header();
    granted_one.extend_from_slice(&[0x01, 0x01, 0x00, 0x00]);
    assert_eq!(down_err(&granted_one), Some(CodecError::Malformed));

    let mut granted_two = accepted_header();
    granted_two.extend_from_slice(&[0x02, 0x01, 0x00, 0x00]);
    assert!(decode_down(&granted_two).is_ok());

    let mut granted_max = accepted_header();
    granted_max.extend_from_slice(&[0xFF; 9]);
    granted_max.push(0x01);
    granted_max.extend_from_slice(&[0x01, 0x00, 0x00]);
    assert!(decode_down(&granted_max).is_ok());

    let mut granted_overflow = accepted_header();
    granted_overflow.extend_from_slice(&overflow_varint());
    granted_overflow.extend_from_slice(&[0x01, 0x00, 0x00]);
    assert_eq!(down_err(&granted_overflow), Some(CodecError::Malformed));

    // --- lane rows of case 6, applied to L4's progress list ---
    let mut count_zero = accepted_with_granted();
    count_zero.push(0x00);
    assert_eq!(down_err(&count_zero), Some(CodecError::Malformed));

    for bad_count in [0x05u8, 0xFF] {
        let mut b = accepted_with_granted();
        b.push(bad_count);
        assert_eq!(
            down_err(&b),
            Some(CodecError::LimitExceeded),
            "lane_count {bad_count:#04x}"
        );
    }

    let mut two_dupes = accepted_with_granted();
    two_dupes.extend_from_slice(&[0x02, 0x00, 0x00, 0x00, 0x00]);
    assert_eq!(down_err(&two_dupes), Some(CodecError::Malformed));

    let mut two_foreign = accepted_with_granted();
    two_foreign.extend_from_slice(&[0x02, 0x00, 0x00, 0x01, 0x00]);
    assert_eq!(down_err(&two_foreign), Some(CodecError::UnknownMessage));

    for foreign_lane in [0x01u8, 0x03] {
        let mut b = accepted_with_granted();
        b.extend_from_slice(&[0x01, foreign_lane, 0x00]);
        assert_eq!(
            down_err(&b),
            Some(CodecError::UnknownMessage),
            "lane {foreign_lane:#04x}"
        );
    }

    let mut truncated_second = accepted_with_granted();
    truncated_second.extend_from_slice(&[0x02, 0x00, 0x00]);
    assert_eq!(down_err(&truncated_second), Some(CodecError::Malformed));

    let mut through_overflow = accepted_with_granted();
    through_overflow.extend_from_slice(&[0x01, 0x00]);
    through_overflow.extend_from_slice(&varint(u64::MAX));
    assert_eq!(down_err(&through_overflow), Some(CodecError::Malformed));

    // --- prefixes and trailing bytes ---
    for lit in [l3_bytes(), l4_bytes(), l5_bytes(), l6_bytes(), l7_bytes()] {
        for len in 0..lit.len() {
            assert_eq!(down_err(&lit[..len]), Some(CodecError::Malformed));
        }
        let mut with_trailing = lit.clone();
        with_trailing.push(0x00);
        assert_eq!(down_err(&with_trailing), Some(CodecError::Malformed));
    }
}

// ---------------------------------------------------------------------------
// Case 8.
// ---------------------------------------------------------------------------

#[test]
fn encode_mirrors_decode() {
    // encode_request and ResumeRequest::validate agree on every bad claim list.
    let bad_claims: Vec<Vec<LaneClaim>> = vec![
        vec![],
        vec![
            claim(0, 0),
            claim(0, 0),
            claim(0, 0),
            claim(0, 0),
            claim(0, 0),
        ],
        vec![claim(1, 0)],
        vec![claim(0, 0), claim(0, 0)],
        vec![claim(0, u64::MAX)],
    ];
    let expected = [
        CodecError::Malformed,
        CodecError::LimitExceeded,
        CodecError::UnknownMessage,
        CodecError::Malformed,
        CodecError::Malformed,
    ];
    for (claims, want) in bad_claims.iter().zip(expected) {
        let req = ResumeRequest {
            epoch: ZERO_EPOCH,
            fence: fence(1),
            proof: proof(&PROOF),
            claims: claims.clone(),
        };
        assert_eq!(req.validate(), Err(want));
        assert_eq!(encode_request(&req), Err(want));
    }

    // Accepted with granted FIRST is Malformed; the granted check comes first.
    let bad_accepted = ResumeResult::Accepted(ResumeAccepted {
        epoch: ZERO_EPOCH,
        granted: ConnectionFence::FIRST,
        progress: vec![progress(0, 0)],
    });
    assert_eq!(bad_accepted.validate(), Err(CodecError::Malformed));
    assert_eq!(
        encode_down(&ResumeDown::Answer(bad_accepted)),
        Err(CodecError::Malformed)
    );

    let bad_accepted_many = ResumeResult::Accepted(ResumeAccepted {
        epoch: ZERO_EPOCH,
        granted: ConnectionFence::FIRST,
        progress: vec![
            progress(0, 0),
            progress(0, 0),
            progress(0, 0),
            progress(0, 0),
            progress(0, 0),
        ],
    });
    assert_eq!(bad_accepted_many.validate(), Err(CodecError::Malformed));
    assert_eq!(
        encode_down(&ResumeDown::Answer(bad_accepted_many)),
        Err(CodecError::Malformed)
    );

    // granted 2 with an empty or foreign progress list.
    let empty_progress = ResumeResult::Accepted(ResumeAccepted {
        epoch: ZERO_EPOCH,
        granted: fence(2),
        progress: vec![],
    });
    let foreign_progress = ResumeResult::Accepted(ResumeAccepted {
        epoch: ZERO_EPOCH,
        granted: fence(2),
        progress: vec![progress(1, 0)],
    });
    assert_eq!(empty_progress.validate(), Err(CodecError::Malformed));
    assert_eq!(
        encode_down(&ResumeDown::Answer(empty_progress)),
        Err(CodecError::Malformed)
    );
    assert_eq!(foreign_progress.validate(), Err(CodecError::UnknownMessage));
    assert_eq!(
        encode_down(&ResumeDown::Answer(foreign_progress)),
        Err(CodecError::UnknownMessage)
    );

    // Valid values encode and decode back equal (control).
    let bytes = encode_request(&request_l1()).unwrap();
    assert_request_eq(
        &decode_request(&bytes).unwrap(),
        ZERO_EPOCH,
        1,
        &PROOF,
        &[claim(0, 0)],
    );
    let bytes = encode_down(&accepted_l4()).unwrap();
    assert_answer(
        &decode_down(&bytes).unwrap(),
        &ResumeResult::Accepted(ResumeAccepted {
            epoch: ZERO_EPOCH,
            granted: fence(2),
            progress: vec![progress(0, 0)],
        }),
    );
}

// ---------------------------------------------------------------------------
// Case 9.
// ---------------------------------------------------------------------------

struct Probe<T>(PhantomData<T>);
macro_rules! probe {
    ($has:ident, $lacks:ident, $method:ident, $($bound:tt)+) => {
        trait $has { fn $method(&self) -> bool { true } }
        impl<T: $($bound)+> $has for Probe<T> {}
        trait $lacks { fn $method(&self) -> bool { false } }
        impl<T> $lacks for &Probe<T> {}
    };
}
probe!(HasPartialEq, LacksPartialEq, partial_eq, PartialEq);
probe!(HasEq, LacksEq, total_eq, Eq);
probe!(HasHash, LacksHash, hash_impl, core::hash::Hash);
probe!(HasOrd, LacksOrd, total_ord, Ord);
probe!(HasDisplay, LacksDisplay, display_impl, core::fmt::Display);

#[test]
#[allow(clippy::needless_borrow)] // the autoref probe below relies on the explicit `&`
fn proof_is_opaque_bounded_and_redacted() {
    // Bounds.
    assert_eq!(ResumeProof::new(vec![]).err(), Some(CodecError::Malformed));
    assert_eq!(
        ResumeProof::new(vec![0x00; 15]).err(),
        Some(CodecError::Malformed)
    );
    assert!(ResumeProof::new(vec![0x00; 16]).is_ok());
    assert!(ResumeProof::new(vec![0x00; 128]).is_ok());
    assert_eq!(
        ResumeProof::new(vec![0x00; 129]).err(),
        Some(CodecError::LimitExceeded)
    );

    // Opaque: 00 FF 80 7F then 12 x 01 round-trips byte-identically.
    let mut opaque = vec![0x00, 0xFF, 0x80, 0x7F];
    opaque.extend_from_slice(&[0x01; 12]);
    assert_eq!(opaque.len(), 16);

    let request = ResumeRequest {
        epoch: ZERO_EPOCH,
        fence: fence(1),
        proof: proof(&opaque),
        claims: vec![claim(0, 0)],
    };
    let req_bytes = encode_request(&request).unwrap();
    assert_eq!(
        decode_request(&req_bytes).unwrap().proof.as_bytes(),
        &opaque[..]
    );

    let ticket = ResumeDown::Ticket(ResumeTicket {
        epoch: ZERO_EPOCH,
        proof: proof(&opaque),
    });
    let ticket_bytes = encode_down(&ticket).unwrap();
    assert_ticket(&decode_down(&ticket_bytes).unwrap(), ZERO_EPOCH, &opaque);

    // Redaction.
    let p = proof(&opaque);
    assert_eq!(format!("{p:?}"), "ResumeProof(<redacted>)");

    let q_bytes: Vec<u8> = (0u8..=0x7F).collect();
    let aa16 = [0xAA; 16];

    let req_p = ResumeRequest {
        epoch: ZERO_EPOCH,
        fence: fence(1),
        proof: proof(&opaque),
        claims: vec![claim(0, 0)],
    };
    let req_aa = ResumeRequest {
        epoch: ZERO_EPOCH,
        fence: fence(1),
        proof: proof(&aa16),
        claims: vec![claim(0, 0)],
    };
    let req_q = ResumeRequest {
        epoch: ZERO_EPOCH,
        fence: fence(1),
        proof: proof(&q_bytes),
        claims: vec![claim(0, 0)],
    };
    let req_p_dbg = format!("{req_p:?}");
    let req_aa_dbg = format!("{req_aa:?}");
    let req_q_dbg = format!("{req_q:?}");
    assert_eq!(req_p_dbg, req_aa_dbg);
    assert_eq!(req_p_dbg, req_q_dbg);
    assert!(req_p_dbg.contains("ResumeProof(<redacted>)"));

    let ticket_p = ResumeDown::Ticket(ResumeTicket {
        epoch: ZERO_EPOCH,
        proof: proof(&opaque),
    });
    let ticket_aa = ResumeDown::Ticket(ResumeTicket {
        epoch: ZERO_EPOCH,
        proof: proof(&aa16),
    });
    let ticket_q = ResumeDown::Ticket(ResumeTicket {
        epoch: ZERO_EPOCH,
        proof: proof(&q_bytes),
    });
    let ticket_p_dbg = format!("{ticket_p:?}");
    let ticket_aa_dbg = format!("{ticket_aa:?}");
    let ticket_q_dbg = format!("{ticket_q:?}");
    assert_eq!(ticket_p_dbg, ticket_aa_dbg);
    assert_eq!(ticket_p_dbg, ticket_q_dbg);
    assert!(ticket_p_dbg.contains("ResumeProof(<redacted>)"));

    let raw_ticket_p = ResumeTicket {
        epoch: ZERO_EPOCH,
        proof: proof(&opaque),
    };
    let raw_ticket_aa = ResumeTicket {
        epoch: ZERO_EPOCH,
        proof: proof(&aa16),
    };
    let raw_ticket_q = ResumeTicket {
        epoch: ZERO_EPOCH,
        proof: proof(&q_bytes),
    };
    let raw_ticket_p_dbg = format!("{raw_ticket_p:?}");
    let raw_ticket_aa_dbg = format!("{raw_ticket_aa:?}");
    let raw_ticket_q_dbg = format!("{raw_ticket_q:?}");
    assert_eq!(raw_ticket_p_dbg, raw_ticket_aa_dbg);
    assert_eq!(raw_ticket_p_dbg, raw_ticket_q_dbg);
    assert!(raw_ticket_p_dbg.contains("ResumeProof(<redacted>)"));

    // ct_eq.
    let p = proof(&opaque);
    let q = proof(&q_bytes);
    assert!(p.ct_eq(p.as_bytes()));
    assert!(q.ct_eq(q.as_bytes()));

    let mut p0 = opaque.clone();
    p0[0] ^= 0xFF;
    let mut p8 = opaque.clone();
    p8[8] ^= 0xFF;
    let mut p15 = opaque.clone();
    p15[15] ^= 0xFF;
    let mut p17 = opaque.clone();
    p17.push(0x01);
    let p_prefix = opaque[..15].to_vec();
    let empty: &[u8] = &[];

    let mut q64 = q_bytes.clone();
    q64[64] ^= 0xFF;
    let q_prefix = q_bytes[..127].to_vec();
    let mut q129 = q_bytes.clone();
    q129.push(0x00);

    assert!(!p.ct_eq(&p0));
    assert!(!p.ct_eq(&p8));
    assert!(!p.ct_eq(&p15));
    assert!(!p.ct_eq(&p17));
    assert!(!p.ct_eq(&p_prefix));
    assert!(!p.ct_eq(empty));
    assert!(!q.ct_eq(&q64));
    assert!(!q.ct_eq(&q_prefix));
    assert!(!q.ct_eq(&q129));

    // Missing traits, detected at compile time via the autoref probe.
    assert!(!(&Probe::<ResumeProof>(PhantomData)).partial_eq());
    assert!(!(&Probe::<ResumeProof>(PhantomData)).total_eq());
    assert!(!(&Probe::<ResumeProof>(PhantomData)).hash_impl());
    assert!(!(&Probe::<ResumeProof>(PhantomData)).total_ord());
    assert!(!(&Probe::<ResumeProof>(PhantomData)).display_impl());

    assert!(!(&Probe::<ResumeRequest>(PhantomData)).partial_eq());
    assert!(!(&Probe::<ResumeRequest>(PhantomData)).display_impl());
    assert!(!(&Probe::<ResumeTicket>(PhantomData)).partial_eq());
    assert!(!(&Probe::<ResumeTicket>(PhantomData)).display_impl());
    assert!(!(&Probe::<ResumeDown>(PhantomData)).partial_eq());
    assert!(!(&Probe::<ResumeDown>(PhantomData)).display_impl());

    assert!((&Probe::<ConnectionFence>(PhantomData)).partial_eq());
    assert!((&Probe::<ConnectionFence>(PhantomData)).total_eq());
    assert!((&Probe::<ConnectionFence>(PhantomData)).hash_impl());
    assert!((&Probe::<ConnectionFence>(PhantomData)).total_ord());
    assert!((&Probe::<CodecError>(PhantomData)).display_impl());
}

// ---------------------------------------------------------------------------
// Case 10.
// ---------------------------------------------------------------------------

#[test]
fn resume_and_existing_codecs_refuse_each_other() {
    let literals = [
        l1_bytes(),
        l2_bytes(),
        l3_bytes(),
        l4_bytes(),
        l5_bytes(),
        l6_bytes(),
        l7_bytes(),
    ];
    for lit in &literals {
        assert_eq!(
            codec::decode_intent(lit),
            Err(CodecError::UnsupportedVersion)
        );
        assert_eq!(
            codec::decode_delta(lit),
            Err(CodecError::UnsupportedVersion)
        );
        assert_eq!(
            codec_v2::decode_intent(lit),
            Err(CodecError::UnsupportedVersion)
        );
        assert_eq!(
            codec_v2::decode_delta(lit),
            Err(CodecError::UnsupportedVersion)
        );
        assert_eq!(
            codec_transfer::decode_down(lit),
            Err(CodecError::UnsupportedVersion)
        );
        assert_eq!(
            codec_transfer::decode_up(lit),
            Err(CodecError::UnsupportedVersion)
        );
    }

    // Valid v1 end-turn intent: version 1, zero epoch, lane 0, seq 1,
    // observed_tick 0, actor 1, EndTurn tag.
    let mut v1_intent = vec![0x01];
    v1_intent.extend_from_slice(&ZERO_EPOCH);
    v1_intent.extend_from_slice(&[0x00, 0x01, 0x00, 0x01, 0x01]);

    // Valid v2 end-turn intent: the v1 shape with version byte 2.
    let mut v2_intent = vec![0x02];
    v2_intent.extend_from_slice(&ZERO_EPOCH);
    v2_intent.extend_from_slice(&[0x00, 0x01, 0x00, 0x01, 0x01]);

    // Valid v1 delta: version 1, lane 0, server_tick 0, event_seq 1, one
    // EntityEnter op (tag 0, entity 1).
    let v1_delta = [0x01, 0x00, 0x00, 0x01, 0x01, 0x00, 0x01];

    // Valid v2 delta: version 2, same shape.
    let v2_delta = [0x02, 0x00, 0x00, 0x01, 0x01, 0x00, 0x01];

    // The five T024 S§5.5 transfer literals (all-zero epoch).
    let mut begin = vec![0x81, 0x02, 0x10];
    begin.extend_from_slice(&ZERO_EPOCH);
    begin.extend_from_slice(&[0x01, 0x00, 0x00, 0x02, 0x00, 0x01, 0x01]);

    let mut chunk = vec![0x81, 0x02, 0x11];
    chunk.extend_from_slice(&ZERO_EPOCH);
    chunk.extend_from_slice(&[0x01, 0x00, 0x01, 0x02, 0x00, 0x00]);

    let mut cancel = vec![0x81, 0x02, 0x12];
    cancel.extend_from_slice(&ZERO_EPOCH);
    cancel.extend_from_slice(&[0x01, 0x01]);

    let mut published = vec![0x81, 0x02, 0x20];
    published.extend_from_slice(&ZERO_EPOCH);
    published.push(0x01);

    let mut resync = vec![0x81, 0x02, 0x21];
    resync.extend_from_slice(&ZERO_EPOCH);
    resync.extend_from_slice(&[0x00, 0x01]);

    let v1_intent_vec = v1_intent;
    let v1_delta_vec = v1_delta.to_vec();
    let v2_delta_vec = v2_delta.to_vec();
    for frame in [
        &v1_intent_vec,
        &v1_delta_vec,
        &v2_delta_vec,
        &begin,
        &chunk,
        &cancel,
        &published,
        &resync,
    ] {
        assert_eq!(request_err(frame), Some(CodecError::UnsupportedVersion));
        assert_eq!(down_err(frame), Some(CodecError::UnsupportedVersion));
    }

    // A 32,815-byte transfer chunk: the resume 4,096 cap comes first.
    let big_chunk = TransferDown::Chunk(TransferChunk {
        epoch: ZERO_EPOCH,
        transfer: TransferId::new(u64::MAX).expect("nonzero transfer id"),
        index: 0,
        bytes: vec![0x00; 32_768],
    });
    let big_chunk_bytes = codec_transfer::encode_down(&big_chunk).unwrap();
    assert_eq!(big_chunk_bytes.len(), 32_815);
    assert_eq!(
        request_err(&big_chunk_bytes),
        Some(CodecError::FrameTooLarge)
    );
    assert_eq!(down_err(&big_chunk_bytes), Some(CodecError::FrameTooLarge));

    // Control: each codec decodes its own frames.
    assert!(codec::decode_intent(&v1_intent_vec).is_ok());
    assert!(codec_v2::decode_intent(&v2_intent).is_ok());
    assert!(codec::decode_delta(&v1_delta).is_ok());
    assert!(codec_v2::decode_delta(&v2_delta).is_ok());
    assert!(codec_transfer::decode_down(&begin).is_ok());
    assert!(codec_transfer::decode_down(&chunk).is_ok());
    assert!(codec_transfer::decode_down(&cancel).is_ok());
    assert!(codec_transfer::decode_up(&published).is_ok());
    assert!(codec_transfer::decode_up(&resync).is_ok());
    assert!(decode_request(&l1_bytes()).is_ok());
    assert!(decode_down(&l3_bytes()).is_ok());
}

// ---------------------------------------------------------------------------
// T025a2 harness (cases 11–21).
// ---------------------------------------------------------------------------

/// Session epoch `E1`.
const E1: [u8; 16] = [0xE1; 16];
/// Session epoch `E2`.
const E2: [u8; 16] = [0xE2; 16];
/// The resume proof `P`.
const P: [u8; 16] = [0xAA; 16];
/// The injected clock base, in milliseconds.
const T0: u64 = 10_000;

fn net(raw: u64) -> NetId {
    NetId::new(raw).expect("fixture net id is nonzero")
}

fn tid(raw: u64) -> TransferId {
    TransferId::new(raw).expect("fixture transfer id is nonzero")
}

fn health(h: u32, m: u32, dead: bool) -> HealthFact {
    HealthFact {
        health: h,
        max_health: m,
        dead,
    }
}

/// `codec::encode_delta` for one fixture delta on lane 0.
fn delta(seq: u64, ops: Vec<DeltaOp>) -> Vec<u8> {
    codec::encode_delta(&DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 100 + seq,
        event_seq: seq,
        ops,
    })
    .expect("fixture delta encodes")
}

fn hp(entity: u64, h: u32) -> DeltaOp {
    DeltaOp::Health {
        entity: net(entity),
        health: h,
        max_health: 10,
        dead: false,
    }
}

/// `n` entities with `NetId` 1..=n, each at 10/10, and no turn.
fn body_n(n: u64) -> SnapshotBody {
    SnapshotBody {
        entities: (1..=n)
            .map(|i| SnapshotEntity {
                entity: net(i),
                health: Some(health(10, 10, false)),
            })
            .collect(),
        turn: None,
    }
}

fn plan(
    epoch: [u8; 16],
    id: u64,
    base: u64,
    cursor: u64,
    spc: u8,
    body: &SnapshotBody,
) -> Vec<Vec<u8>> {
    codec_transfer::plan_transfer(epoch, tid(id), base, cursor, spc, body)
        .expect("fixture transfer plans")
}

fn cancel_frame(epoch: [u8; 16], id: u64, reason: CancelReason) -> Vec<u8> {
    codec_transfer::encode_down(&TransferDown::Cancel {
        epoch,
        transfer: tid(id),
        reason,
    })
    .expect("fixture cancel encodes")
}

fn chunk_frame(epoch: [u8; 16], id: u64, index: u8, bytes: &[u8]) -> Vec<u8> {
    codec_transfer::encode_down(&TransferDown::Chunk(TransferChunk {
        epoch,
        transfer: tid(id),
        index,
        bytes: bytes.to_vec(),
    }))
    .expect("fixture chunk encodes")
}

fn chunk_payload(frame: &[u8]) -> Vec<u8> {
    match codec_transfer::decode_down(frame).expect("fixture chunk decodes") {
        TransferDown::Chunk(chunk) => chunk.bytes,
        other => panic!("expected Chunk, got {other:?}"),
    }
}

fn pool() -> ReassemblyPool {
    ReassemblyPool::new(TransferLimits::v1()).expect("v1 limits are valid")
}

fn pop_all(pool: &mut ReassemblyPool, rid: ReceiverId) -> Vec<ReplicaUpdate> {
    let mut out = Vec::new();
    while let Some(update) = pool.pop_update(rid) {
        out.push(update);
    }
    out
}

/// The `event_seq` of a queued delta update, if it is one.
fn delta_seq(update: &ReplicaUpdate) -> Option<u64> {
    match update {
        ReplicaUpdate::Delta(GatedDelta::V1(frame)) => Some(frame.event_seq),
        ReplicaUpdate::Delta(GatedDelta::V2(frame)) => Some(frame.event_seq),
        ReplicaUpdate::Snapshot(_) => None,
    }
}

/// The published cursor of a queued snapshot update, if it is one.
fn snapshot_cursor(update: &ReplicaUpdate) -> Option<u64> {
    match update {
        ReplicaUpdate::Snapshot(publication) => Some(publication.cursor),
        ReplicaUpdate::Delta(_) => None,
    }
}

/// A request presenting `fence_raw` with claims `claims`.
fn request_with(epoch: [u8; 16], fence_raw: u64, claims: Vec<LaneClaim>) -> ResumeRequest {
    ResumeRequest {
        epoch,
        fence: fence(fence_raw),
        proof: proof(&P),
        claims,
    }
}

// ---------------------------------------------------------------------------
// The test-local replica and its independent oracle.
// ---------------------------------------------------------------------------

type HealthTuple = (u32, u32, bool);
type TurnTuple = (Option<u64>, u64);

/// A replica view as plain facts: entity -> optional health, plus the turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Facts {
    entities: BTreeMap<u64, Option<HealthTuple>>,
    turn: Option<TurnTuple>,
}

/// Hand-authored expected facts. Never derived from a transfer, a projection
/// or a World hash.
fn facts(entities: &[(u64, Option<HealthTuple>)], turn: Option<TurnTuple>) -> Facts {
    Facts {
        entities: entities.iter().copied().collect(),
        turn,
    }
}

/// Applies popped updates, in pop order, to a `BTreeMap` replica.
///
/// A snapshot replaces everything; `EntityEnter` starts a fresh record with no
/// health; `Health` sets it; `EntityLeave` removes it; `Turn` sets the turn.
#[derive(Debug, Clone, Default)]
struct Replica {
    facts: Facts,
    snapshots: usize,
}

impl Replica {
    fn apply_op(&mut self, op: &DeltaOp) {
        match op {
            DeltaOp::EntityEnter { entity } => {
                self.facts.entities.insert(entity.get(), None);
            }
            DeltaOp::EntityLeave { entity } => {
                self.facts.entities.remove(&entity.get());
            }
            DeltaOp::Health {
                entity,
                health,
                max_health,
                dead,
            } => {
                self.facts
                    .entities
                    .insert(entity.get(), Some((*health, *max_health, *dead)));
            }
            DeltaOp::Turn { active, round } => {
                self.facts.turn = Some((active.map(|a| a.get()), *round));
            }
            DeltaOp::Spawned { .. }
            | DeltaOp::Despawned { .. }
            | DeltaOp::Died { .. }
            | DeltaOp::Receipt { .. } => {}
        }
    }

    fn apply(&mut self, update: &ReplicaUpdate) {
        match update {
            ReplicaUpdate::Snapshot(publication) => {
                self.facts = Facts::default();
                for entity in &publication.body.entities {
                    self.facts.entities.insert(
                        entity.entity.get(),
                        entity
                            .health
                            .as_ref()
                            .map(|h| (h.health, h.max_health, h.dead)),
                    );
                }
                self.facts.turn = publication
                    .body
                    .turn
                    .as_ref()
                    .map(|t| (t.active.map(|a| a.get()), t.round));
                self.snapshots += 1;
            }
            ReplicaUpdate::Delta(GatedDelta::V1(frame)) => {
                for op in &frame.ops {
                    self.apply_op(op);
                }
            }
            ReplicaUpdate::Delta(GatedDelta::V2(frame)) => {
                for op in &frame.ops {
                    if let protocol_v2::DeltaOp::Legacy(op) = op {
                        self.apply_op(op);
                    }
                }
            }
        }
    }
}

fn drain_into(replica: &mut Replica, pool: &mut ReassemblyPool, rid: ReceiverId) -> usize {
    let mut n = 0;
    while let Some(update) = pool.pop_update(rid) {
        replica.apply(&update);
        n += 1;
    }
    n
}

/// Fact-by-fact comparison; empty means the replica equals `want`.
fn diff(got: &Facts, want: &Facts) -> Vec<String> {
    let mut out = Vec::new();
    for (id, expected) in &want.entities {
        match got.entities.get(id) {
            None => out.push(format!("missing entity {id}")),
            Some(actual) if actual != expected => out.push(match (actual, expected) {
                (Some(_), None) => format!("leak: health of entity {id} is not permitted"),
                (None, Some(_)) => format!("missing health of entity {id}"),
                _ => format!("mismatch health of entity {id}: got {actual:?} want {expected:?}"),
            }),
            Some(_) => {}
        }
    }
    for id in got.entities.keys() {
        if !want.entities.contains_key(id) {
            out.push(format!("leak: entity {id} is not permitted"));
        }
    }
    match (&got.turn, &want.turn) {
        (None, Some(_)) => out.push("missing turn".to_string()),
        (Some(_), None) => out.push("leak: turn is not permitted".to_string()),
        (Some(actual), Some(expected)) if actual != expected => {
            out.push(format!("mismatch turn: got {actual:?} want {expected:?}"))
        }
        _ => {}
    }
    out
}

fn assert_matches(replica: &Replica, want: &Facts) {
    let d = diff(&replica.facts, want);
    assert!(
        d.is_empty(),
        "oracle mismatch ({} items), first: {:?}",
        d.len(),
        d.iter().take(4).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// The host stand-in and the client-side resume state.
// ---------------------------------------------------------------------------

/// A client's retained resume state: the epoch and the fence it holds.
struct Client {
    epoch: [u8; 16],
    fence: ConnectionFence,
}

impl Client {
    fn new(epoch: [u8; 16]) -> Client {
        Client {
            epoch,
            fence: ConnectionFence::FIRST,
        }
    }
}

/// Host stand-in. It performs **no authentication**: `decision` is test input
/// that each test sets explicitly before every request, and no case here
/// claims to prove authentication or host resume.
struct Stand {
    fence: SessionFence,
    decision: ResumeDecision,
}

impl Stand {
    fn new(epoch: [u8; 16]) -> Stand {
        Stand {
            fence: SessionFence::new(epoch),
            decision: ResumeDecision::Reject(ResumeRejection::Denied),
        }
    }
}

/// One request/answer round trip over the resume codec, as S§18 describes it.
fn resume_once(stand: &mut Stand, client: &mut Client) -> Result<ResumeResult, ResumeError> {
    let request = request_with(client.epoch, client.fence.get(), vec![claim(0, 0)]);
    let bytes = encode_request(&request).expect("request encodes");
    let decoded = decode_request(&bytes).expect("request decodes");
    let result = stand.fence.resolve(&decoded, &stand.decision)?;
    let down = encode_down(&ResumeDown::Answer(result)).expect("result encodes");
    let result = match decode_down(&down).expect("result decodes") {
        ResumeDown::Answer(result) => result,
        other => panic!("expected Answer, got {other:?}"),
    };
    check_result(client.epoch, client.fence, &result)?;
    if let ResumeResult::Accepted(accepted) = &result {
        client.fence = accepted.granted;
    }
    Ok(result)
}

fn granted_of(result: &ResumeResult) -> u64 {
    match result {
        ResumeResult::Accepted(accepted) => accepted.granted.get(),
        other => panic!("expected Accepted, got {other:?}"),
    }
}

fn state(fence: &SessionFence) -> (u64, u64) {
    (fence.current().get(), fence.confirmed().get())
}

/// Among tags `1..=current`, exactly `current` passes `check_frame`. Probes
/// are clones, so the probes change nothing.
fn only_current_passes(fence_state: &SessionFence) {
    let current = fence_state.current().get();
    for raw in 1..=current {
        let mut probe = fence_state.clone();
        let outcome = probe.check_frame(fence(raw));
        if raw == current {
            assert_eq!(outcome, Ok(()), "tag {raw} must pass");
            assert_eq!(probe.current(), fence_state.current());
        } else {
            assert_eq!(
                outcome,
                Err(ResumeError::Fenced),
                "tag {raw} must be fenced"
            );
            assert_eq!(probe, *fence_state);
        }
    }
}

/// Every refusal must be the same from `check_request` and `resolve`, under
/// all three decisions, with the fence unchanged.
fn assert_refusal(fence_state: &mut SessionFence, request: &ResumeRequest, want: ResumeError) {
    let before = fence_state.clone();
    assert_eq!(fence_state.check_request(request), Err(want));
    assert_eq!(*fence_state, before);
    for decision in [
        ResumeDecision::Accept {
            progress: vec![progress(0, 0)],
        },
        ResumeDecision::Reject(ResumeRejection::Denied),
        ResumeDecision::Accept { progress: vec![] },
    ] {
        assert_eq!(fence_state.resolve(request, &decision), Err(want));
        assert_eq!(*fence_state, before);
    }
}

// ---------------------------------------------------------------------------
// Case 11.
// ---------------------------------------------------------------------------

#[test]
fn fence_starts_at_first_and_checks_frames() {
    let mut fence_state = SessionFence::new(E1);
    assert_eq!(fence_state.epoch(), E1);
    assert_eq!(fence_state.current(), ConnectionFence::FIRST);
    assert_eq!(fence_state.confirmed(), ConnectionFence::FIRST);
    assert_eq!(fence_state.max_fence(), u64::MAX);

    assert_eq!(fence_state.check_frame(ConnectionFence::FIRST), Ok(()));
    let before = fence_state.clone();
    assert_eq!(
        fence_state.check_frame(fence(2)),
        Err(ResumeError::UnknownFence)
    );
    assert_eq!(fence_state, before);

    for bad in [0u64, 1] {
        assert_eq!(
            SessionFence::with_max(E1, bad),
            Err(ResumeError::InvalidLimit),
            "max_fence {bad}"
        );
    }
    let limited = SessionFence::with_max(E1, 2).expect("2 is a valid limit");
    assert_eq!(limited.max_fence(), 2);

    for (error, name) in [
        (ResumeError::WrongEpoch, "WrongEpoch"),
        (ResumeError::UnknownFence, "UnknownFence"),
        (ResumeError::StaleFence, "StaleFence"),
        (ResumeError::Fenced, "Fenced"),
        (ResumeError::FenceExhausted, "FenceExhausted"),
        (ResumeError::InvalidProgress, "InvalidProgress"),
        (ResumeError::InvalidLimit, "InvalidLimit"),
    ] {
        assert_eq!(error.to_string(), format!("{name} at resume/fence"));
    }
}

// ---------------------------------------------------------------------------
// Case 12.
// ---------------------------------------------------------------------------

#[test]
fn resolve_reflects_an_explicit_decision() {
    let q = request_with(E1, 1, vec![claim(0, 0)]);

    for reason in [ResumeRejection::Denied, ResumeRejection::Busy] {
        let mut fence_state = SessionFence::new(E1);
        let before = fence_state.clone();
        let got = fence_state.resolve(&q, &ResumeDecision::Reject(reason));
        assert_eq!(
            got,
            Ok(ResumeResult::Rejected(ResumeRejected { epoch: E1, reason }))
        );
        assert_eq!(fence_state, before);
    }

    let mut fence_state = SessionFence::new(E1);
    let got = fence_state.resolve(
        &q,
        &ResumeDecision::Accept {
            progress: vec![progress(0, 7)],
        },
    );
    assert_eq!(
        got,
        Ok(ResumeResult::Accepted(ResumeAccepted {
            epoch: E1,
            granted: fence(2),
            progress: vec![progress(0, 7)],
        }))
    );
    assert_eq!(fence_state.current(), fence(2));
    assert_eq!(fence_state.confirmed(), fence(1));

    let invalid: Vec<Vec<LaneProgress>> = vec![
        vec![],
        vec![progress(1, 0)],
        vec![progress(0, 0), progress(0, 0)],
        vec![progress(0, 0); 5],
        vec![progress(0, u64::MAX)],
    ];
    for progress_list in invalid {
        let mut fence_state = SessionFence::new(E1);
        let before = fence_state.clone();
        assert_eq!(
            fence_state.resolve(
                &q,
                &ResumeDecision::Accept {
                    progress: progress_list,
                }
            ),
            Err(ResumeError::InvalidProgress)
        );
        assert_eq!(fence_state, before);
    }

    // Control: the valid progress list then succeeds.
    let mut fence_state = SessionFence::new(E1);
    assert!(fence_state
        .resolve(
            &q,
            &ResumeDecision::Accept {
                progress: vec![progress(0, 7)],
            }
        )
        .is_ok());

    // Claims are advisory: the same accept from different claims agrees.
    let q_high = request_with(E1, 1, vec![claim(0, 900)]);
    let decision = ResumeDecision::Accept {
        progress: vec![progress(0, 7)],
    };
    let mut left = SessionFence::new(E1);
    let mut right = SessionFence::new(E1);
    assert_eq!(
        left.resolve(&q, &decision),
        right.resolve(&q_high, &decision)
    );
    assert_eq!(left, right);
}

// ---------------------------------------------------------------------------
// Case 13.
// ---------------------------------------------------------------------------

#[test]
fn resolve_refusal_precedence() {
    let valid = request_with(E1, 1, vec![claim(0, 0)]);
    let accept = ResumeDecision::Accept {
        progress: vec![progress(0, 0)],
    };

    let mut fresh = SessionFence::new(E1);
    for (request, want) in [
        (
            request_with(E2, 1, vec![claim(0, 0)]),
            ResumeError::WrongEpoch,
        ),
        (
            request_with(E2, 9, vec![claim(0, 0)]),
            ResumeError::WrongEpoch,
        ),
        (
            request_with(E1, 2, vec![claim(0, 0)]),
            ResumeError::UnknownFence,
        ),
    ] {
        assert_refusal(&mut fresh, &request, want);
    }

    let mut confirmed = SessionFence::new(E1);
    assert!(confirmed.resolve(&valid, &accept).is_ok());
    assert_eq!(confirmed.check_frame(fence(2)), Ok(()));
    assert_refusal(
        &mut confirmed,
        &request_with(E1, 1, vec![claim(0, 0)]),
        ResumeError::StaleFence,
    );

    let mut limited = SessionFence::with_max(E1, 2).expect("2 is a valid limit");
    assert!(limited.resolve(&valid, &accept).is_ok());
    assert_refusal(
        &mut limited,
        &request_with(E1, 2, vec![claim(0, 0)]),
        ResumeError::FenceExhausted,
    );
    assert_refusal(
        &mut limited,
        &request_with(E1, 1, vec![claim(0, 0)]),
        ResumeError::FenceExhausted,
    );
    assert_refusal(
        &mut limited,
        &request_with(E1, 3, vec![claim(0, 0)]),
        ResumeError::UnknownFence,
    );

    // Controls.
    let check_only = SessionFence::new(E1);
    let before = check_only.clone();
    assert_eq!(check_only.check_request(&valid), Ok(fence(2)));
    assert_eq!(check_only, before);

    let mut recovered = SessionFence::new(E1);
    assert_eq!(
        recovered.check_request(&request_with(E1, 2, vec![claim(0, 0)])),
        Err(ResumeError::UnknownFence)
    );
    assert!(matches!(
        recovered.resolve(&valid, &accept),
        Ok(ResumeResult::Accepted(_))
    ));

    let mut live = SessionFence::new(E1);
    assert_eq!(
        live.resolve(&valid, &ResumeDecision::Accept { progress: vec![] }),
        Err(ResumeError::InvalidProgress)
    );
    assert_eq!(
        live.resolve(&valid, &ResumeDecision::Reject(ResumeRejection::Denied)),
        Ok(ResumeResult::Rejected(ResumeRejected {
            epoch: E1,
            reason: ResumeRejection::Denied,
        }))
    );

    let mut bounded = SessionFence::with_max(E1, 3).expect("3 is a valid limit");
    for _ in 0..2 {
        assert!(matches!(
            bounded.resolve(&valid, &accept),
            Ok(ResumeResult::Accepted(_))
        ));
    }
    assert_eq!(
        bounded.resolve(&valid, &accept),
        Err(ResumeError::FenceExhausted)
    );
}

// ---------------------------------------------------------------------------
// Case 14.
// ---------------------------------------------------------------------------

#[test]
fn fence_window_retries_lost_grants_without_reissuing() {
    let mut fence_state = SessionFence::new(E1);
    let accept = ResumeDecision::Accept {
        progress: vec![progress(0, 0)],
    };
    let present = |raw: u64| request_with(E1, raw, vec![claim(0, 0)]);
    let mut grants = Vec::new();

    // Step 1.
    let got = fence_state.resolve(&present(1), &accept);
    grants.push(granted_of(&got.expect("step 1 resolves")));
    assert_eq!(state(&fence_state), (2, 1));
    only_current_passes(&fence_state);

    // Step 2: the result was lost, so the client retries fence 1.
    let got = fence_state.resolve(&present(1), &accept);
    grants.push(granted_of(&got.expect("step 2 resolves")));
    assert_eq!(state(&fence_state), (3, 1));
    only_current_passes(&fence_state);

    // Step 3.
    assert_eq!(fence_state.check_frame(fence(2)), Err(ResumeError::Fenced));
    assert_eq!(state(&fence_state), (3, 1));
    only_current_passes(&fence_state);

    // Step 4.
    assert_eq!(fence_state.check_frame(fence(3)), Ok(()));
    assert_eq!(state(&fence_state), (3, 3));
    only_current_passes(&fence_state);

    // Step 5.
    assert_eq!(
        fence_state.check_request(&present(1)),
        Err(ResumeError::StaleFence)
    );
    assert_eq!(
        fence_state.check_request(&present(2)),
        Err(ResumeError::StaleFence)
    );
    assert_eq!(
        fence_state.check_request(&present(4)),
        Err(ResumeError::UnknownFence)
    );
    assert_eq!(state(&fence_state), (3, 3));
    only_current_passes(&fence_state);

    // Step 6.
    let got = fence_state.resolve(&present(3), &accept);
    grants.push(granted_of(&got.expect("step 6 resolves")));
    assert_eq!(state(&fence_state), (4, 3));
    only_current_passes(&fence_state);

    // Step 7: a racing attempt on the same fence.
    let got = fence_state.resolve(&present(3), &accept);
    grants.push(granted_of(&got.expect("step 7 resolves")));
    assert_eq!(state(&fence_state), (5, 3));
    only_current_passes(&fence_state);

    // Step 8.
    assert_eq!(fence_state.check_frame(fence(4)), Err(ResumeError::Fenced));
    assert_eq!(fence_state.check_frame(fence(5)), Ok(()));
    assert_eq!(state(&fence_state), (5, 5));
    only_current_passes(&fence_state);

    // Step 9.
    assert_eq!(
        fence_state.check_request(&present(4)),
        Err(ResumeError::StaleFence)
    );
    let got = fence_state.resolve(&present(5), &accept);
    grants.push(granted_of(&got.expect("step 9 resolves")));
    assert_eq!(state(&fence_state), (6, 5));
    only_current_passes(&fence_state);

    assert_eq!(grants, [2, 3, 4, 5, 6]);
}

// ---------------------------------------------------------------------------
// Case 15.
// ---------------------------------------------------------------------------

#[test]
fn client_check_result_and_duplicates() {
    let accepted = ResumeResult::Accepted(ResumeAccepted {
        epoch: E1,
        granted: fence(2),
        progress: vec![progress(0, 0)],
    });
    assert_eq!(check_result(E1, ConnectionFence::FIRST, &accepted), Ok(()));
    assert_eq!(
        check_result(E1, fence(2), &accepted),
        Err(ResumeError::StaleFence)
    );

    let other_epoch = ResumeResult::Accepted(ResumeAccepted {
        epoch: E2,
        granted: fence(2),
        progress: vec![progress(0, 0)],
    });
    assert_eq!(
        check_result(E1, ConnectionFence::FIRST, &other_epoch),
        Err(ResumeError::WrongEpoch)
    );
    let rejected_other_epoch = ResumeResult::Rejected(ResumeRejected {
        epoch: E2,
        reason: ResumeRejection::Denied,
    });
    assert_eq!(
        check_result(E1, ConnectionFence::FIRST, &rejected_other_epoch),
        Err(ResumeError::WrongEpoch)
    );
    let busy = ResumeResult::Rejected(ResumeRejected {
        epoch: E1,
        reason: ResumeRejection::Busy,
    });
    assert_eq!(check_result(E1, ConnectionFence::FIRST, &busy), Ok(()));

    let first_grant = ResumeResult::Accepted(ResumeAccepted {
        epoch: E1,
        granted: ConnectionFence::FIRST,
        progress: vec![progress(0, 0)],
    });
    assert_eq!(
        check_result(E1, ConnectionFence::FIRST, &first_grant),
        Err(ResumeError::StaleFence)
    );
    assert_eq!(
        encode_down(&ResumeDown::Answer(first_grant)),
        Err(CodecError::Malformed)
    );

    // L4 decoded twice gives equal values and the same client answer.
    let decode_l4 = || match decode_down(&l4_bytes()).expect("L4 decodes") {
        ResumeDown::Answer(result) => result,
        other => panic!("expected Answer, got {other:?}"),
    };
    let first = decode_l4();
    let second = decode_l4();
    assert_eq!(first, second);
    let first_answer = check_result(ZERO_EPOCH, ConnectionFence::FIRST, &first);
    let second_answer = check_result(ZERO_EPOCH, ConnectionFence::FIRST, &second);
    assert_eq!(first_answer, Ok(()));
    assert_eq!(first_answer, second_answer);
}

// ---------------------------------------------------------------------------
// Case 16.
// ---------------------------------------------------------------------------

#[test]
fn rebind_keeps_floors_and_fails_closed() {
    let mut pool_state = pool();
    let r = pool_state
        .add_receiver(E1, DeltaWire::V1, GateMode::Live)
        .expect("receiver admitted");

    // (a) Three queued deltas.
    let d1 = delta(1, vec![hp(1, 9)]);
    let d2 = delta(2, vec![hp(1, 8)]);
    let d3 = delta(3, vec![hp(1, 7)]);
    for bytes in [&d1, &d2, &d3] {
        assert_eq!(
            pool_state.offer_delta(r, bytes, T0),
            Ok(DeltaOutcome::Released { count: 1 })
        );
    }

    // (b) A transfer in flight with two held deltas.
    let t4 = plan(E1, 4, 200, 5, 1, &body_n(1_000));
    assert_eq!(
        pool_state.offer_transfer(r, &t4[0], T0),
        Ok(TransferOutcome::Begun)
    );
    let begin = match codec_transfer::decode_down(&t4[0]).expect("begin decodes") {
        TransferDown::Begin(begin) => begin,
        other => panic!("expected Begin, got {other:?}"),
    };
    assert_eq!(begin.chunk_count, 2);
    assert_eq!(begin.total_bytes, 5_876);
    assert_eq!(
        pool_state.offer_transfer(r, &t4[1], T0),
        Ok(TransferOutcome::Stored {
            received: 1,
            chunk_count: 2,
        })
    );
    assert_eq!(
        pool_state.offer_delta(r, &delta(5, vec![hp(1, 6)]), T0),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(
        pool_state.offer_delta(r, &delta(7, vec![hp(1, 4)]), T0),
        Ok(DeltaOutcome::Buffered)
    );
    let status = pool_state.status(r).expect("receiver exists");
    assert_eq!(
        status
            .in_flight
            .as_ref()
            .map(|i| (i.transfer, i.received, i.chunk_count)),
        Some((tid(4), 1, 2))
    );
    assert_eq!(status.held_frames, 5);
    assert_eq!(status.next_event_seq, 4);
    assert_eq!(status.retired_through, 0);
    assert_eq!(status.mode, GateMode::Live);
    assert_eq!(pool_state.staging_bytes(), 5_876);

    // (c) Rebind.
    assert_eq!(
        pool_state.rebind_receiver(r, T0),
        Ok(Rebound {
            discarded: Some(tid(4)),
            dropped_deltas: 2,
        })
    );
    let status = pool_state.status(r).expect("receiver exists");
    assert!(status.in_flight.is_none());
    assert_eq!(status.retired_through, 4);
    assert_eq!(status.mode, GateMode::AwaitSnapshot);
    assert_eq!(status.resync, None);
    assert_eq!(status.next_event_seq, 4);
    assert_eq!(status.published, None);
    assert_eq!(status.queued_updates, 3);
    assert_eq!(status.held_frames, 3);
    assert_eq!(pool_state.staging_bytes(), 0);
    assert_eq!(
        pool_state.held_delta_bytes(),
        d1.len() + d2.len() + d3.len()
    );

    // (d) Late frames.
    let settled = pool_state.clone();
    assert_eq!(
        pool_state.offer_transfer(r, &t4[2], T0),
        Err(TransferError::StaleTransfer)
    );
    assert_eq!(pool_state, settled);
    assert_eq!(
        pool_state.offer_transfer(r, &t4[0], T0),
        Err(TransferError::StaleTransfer)
    );
    assert_eq!(pool_state, settled);
    let behind = plan(E1, 6, 300, 1, 8, &body_n(2));
    assert_eq!(
        pool_state.offer_transfer(r, &behind[0], T0),
        Err(TransferError::BehindReplica)
    );
    assert_eq!(pool_state, settled);
    assert_eq!(
        pool_state.offer_delta(r, &d3, T0),
        Ok(DeltaOutcome::Superseded)
    );
    assert_eq!(
        pool_state.offer_delta(r, &delta(4, vec![hp(1, 6)]), T0),
        Ok(DeltaOutcome::Buffered)
    );

    // (e) The resume snapshot installs before post-cutover deltas.
    let t5 = plan(E1, 5, 300, 6, 8, &body_n(2));
    assert_eq!(
        pool_state.offer_transfer(r, &t5[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool_state.offer_transfer(r, &t5[1], T0),
        Ok(TransferOutcome::Published {
            transfer: tid(5),
            released: 0,
        })
    );
    let updates = pop_all(&mut pool_state, r);
    assert_eq!(updates.len(), 4);
    assert_eq!(delta_seq(&updates[0]), Some(1));
    assert_eq!(delta_seq(&updates[1]), Some(2));
    assert_eq!(delta_seq(&updates[2]), Some(3));
    assert_eq!(snapshot_cursor(&updates[3]), Some(6));
    assert_eq!(pool_state.pop_update(r), None);

    assert_eq!(
        pool_state.offer_delta(r, &delta(7, vec![hp(1, 4)]), T0),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    let popped = pool_state.pop_update(r).expect("delta 7 pops");
    assert_eq!(delta_seq(&popped), Some(7));
    assert_eq!(
        pool_state.offer_delta(r, &delta(6, vec![hp(1, 5)]), T0),
        Ok(DeltaOutcome::Superseded)
    );

    // (f) Idempotence and unknown ids.
    let status_before = pool_state.status(r).expect("receiver exists");
    assert_eq!(
        pool_state.rebind_receiver(r, T0),
        Ok(Rebound {
            discarded: None,
            dropped_deltas: 0,
        })
    );
    let status_after = pool_state.status(r).expect("receiver exists");
    let mut expected = status_before;
    expected.mode = GateMode::AwaitSnapshot;
    assert_eq!(status_after, expected);

    let mut unknown = pool();
    let gone = unknown
        .add_receiver(E1, DeltaWire::V1, GateMode::Live)
        .expect("receiver admitted");
    unknown.remove_receiver(gone);
    // The step-1 sweep clamps the pool clock first; after that the unknown id
    // changes nothing.
    assert!(unknown.poll(T0).is_empty());
    let before = unknown.clone();
    assert_eq!(
        unknown.rebind_receiver(gone, T0),
        Err(TransferError::UnknownReceiver)
    );
    assert_eq!(unknown, before);

    // (g) The expiry sweep runs first.
    let mut expiring = pool();
    let g = expiring
        .add_receiver(E1, DeltaWire::V1, GateMode::Live)
        .expect("receiver admitted");
    let t1 = plan(E1, 1, 100, 0, 1, &body_n(1_000));
    assert_eq!(
        expiring.offer_transfer(g, &t1[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        expiring.offer_transfer(g, &t1[1], T0),
        Ok(TransferOutcome::Stored {
            received: 1,
            chunk_count: 2,
        })
    );
    assert_eq!(
        expiring.rebind_receiver(g, T0 + 5_000),
        Ok(Rebound {
            discarded: None,
            dropped_deltas: 0,
        })
    );
    let status = expiring.status(g).expect("receiver exists");
    assert_eq!(status.retired_through, 1);
    assert_eq!(status.resync, None);
    assert_eq!(status.mode, GateMode::AwaitSnapshot);
    assert_eq!(expiring.staging_bytes(), 0);

    // (g2) The sweep sees the clamped clock, not the raw one. The reachable
    // state where the two differ is the saturating top of the clock: a
    // transfer begun there has `deadline_ms == now_ms == u64::MAX`, so the
    // clamped sweep expires it first and `discarded` is `None`, while the
    // raw `0` would find nothing to sweep and discard `T9` in step 3.
    let mut saturated = pool();
    assert!(saturated.poll(u64::MAX).is_empty());
    let s = saturated
        .add_receiver(E1, DeltaWire::V1, GateMode::Live)
        .expect("receiver admitted");
    let t9 = plan(E1, 9, 100, 0, 1, &body_n(1_000));
    assert_eq!(
        saturated.offer_transfer(s, &t9[0], 0),
        Ok(TransferOutcome::Begun)
    );
    let status = saturated.status(s).expect("receiver exists");
    assert_eq!(
        status.in_flight.as_ref().map(|i| i.deadline_ms),
        Some(u64::MAX)
    );
    assert_eq!(
        saturated.rebind_receiver(s, 0),
        Ok(Rebound {
            discarded: None,
            dropped_deltas: 0,
        })
    );
    let status = saturated.status(s).expect("receiver exists");
    assert!(status.in_flight.is_none());
    assert_eq!(status.retired_through, 9);
    assert_eq!(status.mode, GateMode::AwaitSnapshot);
    assert_eq!(status.resync, None);
    assert_eq!(saturated.staging_bytes(), 0);

    // (h) Negative control: remove-then-add loses both floors.
    let mut readded = pool();
    let first = readded
        .add_receiver(E1, DeltaWire::V1, GateMode::Live)
        .expect("receiver admitted");
    for bytes in [&d1, &d2, &d3] {
        assert_eq!(
            readded.offer_delta(first, bytes, T0),
            Ok(DeltaOutcome::Released { count: 1 })
        );
    }
    assert_eq!(
        readded.offer_transfer(first, &t4[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        readded.offer_transfer(first, &t4[1], T0),
        Ok(TransferOutcome::Stored {
            received: 1,
            chunk_count: 2,
        })
    );
    readded.remove_receiver(first);
    let second = readded
        .add_receiver(E1, DeltaWire::V1, GateMode::AwaitSnapshot)
        .expect("receiver admitted");
    assert_eq!(
        readded.offer_transfer(second, &t4[0], T0),
        Ok(TransferOutcome::Begun)
    );
    let third = readded
        .add_receiver(E1, DeltaWire::V1, GateMode::Live)
        .expect("receiver admitted");
    let t6 = plan(E1, 6, 100, 1, 8, &body_n(2));
    assert_eq!(
        readded.offer_transfer(third, &t6[0], T0),
        Ok(TransferOutcome::Begun)
    );
}

// ---------------------------------------------------------------------------
// Case 17.
// ---------------------------------------------------------------------------

/// A live client that has popped deltas 1–4 and holds `{1: 7/10}`.
fn resumed_history() -> (ReassemblyPool, ReceiverId, Replica) {
    let mut pool_state = pool();
    let r = pool_state
        .add_receiver(E1, DeltaWire::V1, GateMode::Live)
        .expect("receiver admitted");
    let mut replica = Replica::default();
    let d1 = delta(1, vec![DeltaOp::EntityEnter { entity: net(1) }, hp(1, 10)]);
    for bytes in [
        d1,
        delta(2, vec![hp(1, 9)]),
        delta(3, vec![hp(1, 8)]),
        delta(4, vec![hp(1, 7)]),
    ] {
        assert_eq!(
            pool_state.offer_delta(r, &bytes, T0),
            Ok(DeltaOutcome::Released { count: 1 })
        );
        for update in pop_all(&mut pool_state, r) {
            replica.apply(&update);
        }
    }
    (pool_state, r, replica)
}

#[test]
fn resume_cutover_installs_snapshot_before_post_cutover_deltas() {
    let (mut pool_state, r, history) = resumed_history();
    assert_matches(&history, &facts(&[(1, Some((7, 10, false)))], None));

    // Resume.
    assert_eq!(
        pool_state.rebind_receiver(r, T0),
        Ok(Rebound {
            discarded: None,
            dropped_deltas: 0,
        })
    );
    let mut stand = Stand::new(E1);
    stand.decision = ResumeDecision::Accept {
        progress: vec![progress(0, 0)],
    };
    let mut client = Client::new(E1);
    let got = resume_once(&mut stand, &mut client).expect("resume answers");
    assert_eq!(
        got,
        ResumeResult::Accepted(ResumeAccepted {
            epoch: E1,
            granted: fence(2),
            progress: vec![progress(0, 0)],
        })
    );

    // Run A: the clearing host.
    let body = SnapshotBody {
        entities: vec![SnapshotEntity {
            entity: net(1),
            health: Some(health(6, 10, false)),
        }],
        turn: None,
    };
    let t1 = plan(E1, 1, 60, 5, 8, &body);
    assert_eq!(
        chunk_payload(&t1[1]),
        [0x01, 0x01, 0x01, 0x06, 0x0A, 0x00, 0x00]
    );
    assert_eq!(
        pool_state.offer_delta(r, &delta(6, vec![hp(1, 5)]), T0),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(
        pool_state.offer_transfer(r, &t1[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool_state.offer_transfer(r, &t1[1], T0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 1,
        })
    );
    assert_eq!(
        pool_state.offer_delta(r, &delta(7, vec![hp(1, 4)]), T0),
        Ok(DeltaOutcome::Released { count: 1 })
    );

    let updates = pop_all(&mut pool_state, r);
    assert_eq!(updates.len(), 3);
    assert_eq!(snapshot_cursor(&updates[0]), Some(5));
    assert_eq!(delta_seq(&updates[1]), Some(6));
    assert_eq!(delta_seq(&updates[2]), Some(7));
    let mut view = Replica::default();
    for (update, want) in updates.iter().zip([
        facts(&[(1, Some((6, 10, false)))], None),
        facts(&[(1, Some((5, 10, false)))], None),
        facts(&[(1, Some((4, 10, false)))], None),
    ]) {
        view.apply(update);
        assert_matches(&view, &want);
    }

    // Duplicates and late frames.
    let settled = pool_state.clone();
    assert_eq!(
        pool_state.offer_transfer(r, &t1[1], T0),
        Err(TransferError::StaleTransfer)
    );
    assert_eq!(pool_state, settled);
    assert_eq!(
        pool_state.offer_transfer(r, &t1[0], T0),
        Err(TransferError::StaleTransfer)
    );
    assert_eq!(pool_state, settled);
    for bytes in [
        delta(5, vec![hp(1, 6)]),
        delta(6, vec![hp(1, 5)]),
        delta(7, vec![hp(1, 4)]),
    ] {
        assert_eq!(
            pool_state.offer_delta(r, &bytes, T0),
            Ok(DeltaOutcome::Superseded)
        );
        assert_eq!(pool_state, settled);
    }

    // Run B: a host that re-sends delta 5 before the Begin.
    let (mut run_b, rb, _) = resumed_history();
    assert_eq!(
        run_b.rebind_receiver(rb, T0),
        Ok(Rebound {
            discarded: None,
            dropped_deltas: 0,
        })
    );
    assert_eq!(
        run_b.offer_delta(rb, &delta(5, vec![hp(1, 6)]), T0),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(
        run_b.offer_transfer(rb, &t1[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        run_b.offer_transfer(rb, &t1[1], T0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    assert_eq!(
        run_b.offer_delta(rb, &delta(6, vec![hp(1, 5)]), T0),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    let run_b_updates = pop_all(&mut run_b, rb);
    assert_eq!(run_b_updates.len(), 2);
    assert_eq!(snapshot_cursor(&run_b_updates[0]), Some(5));
    assert_eq!(delta_seq(&run_b_updates[1]), Some(6));
    assert!(run_b_updates
        .iter()
        .filter_map(delta_seq)
        .all(|seq| seq > 5));

    // Negative control: the same frames on a receiver that was not rebound.
    let (mut live, rl, _) = resumed_history();
    assert_eq!(
        live.offer_delta(rl, &delta(5, vec![hp(1, 6)]), T0),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    assert_eq!(
        live.offer_transfer(rl, &t1[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        live.offer_transfer(rl, &t1[1], T0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    assert_eq!(
        live.offer_delta(rl, &delta(6, vec![hp(1, 5)]), T0),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    let live_updates = pop_all(&mut live, rl);
    assert_eq!(live_updates.len(), 3);
    assert_eq!(delta_seq(&live_updates[0]), Some(5));
    assert_eq!(snapshot_cursor(&live_updates[1]), Some(5));
    assert_eq!(delta_seq(&live_updates[2]), Some(6));
    assert!(live_updates
        .iter()
        .filter_map(delta_seq)
        .any(|seq| seq <= 5));
}

// ---------------------------------------------------------------------------
// Driver rig for cases 18–19 (copied from `tests/conformance.rs`).
// ---------------------------------------------------------------------------

fn generous_rates() -> RateCaps {
    RateCaps {
        frames_per_sec: 1_000_000,
        burst_frames: 1_000_000,
        bytes_per_sec: 1_000_000_000,
        burst_bytes: 1_000_000_000,
        fail_per_sec: 1_000_000,
        burst_fail: 1_000_000,
        fail_bytes_per_sec: 1_000_000_000,
        burst_fail_bytes: 1_000_000_000,
    }
}

/// One fabric peer bound to `E1` that owns both combatants, fully visible, at
/// `server_tick = 1_000`.
fn rig() -> (NetDriver, PeerId, EntityId, EntityId) {
    let fixture = Fixture::standard();
    let (world, ids) = fixture.start(7);
    assert_eq!(ids.len(), 2, "standard fixture fields two combatants");
    let (first, second) = (ids[0], ids[1]);
    let mut transport = InMemoryTransport::new(
        QueueCaps {
            per_peer_frames: 10_000,
            per_peer_bytes: 64 * 1024 * 1024,
            host_frames: 100_000,
            host_bytes: 512 * 1024 * 1024,
        },
        generous_rates(),
        SimClock(0),
    );
    let peer = transport.add_peer(E1).expect("peer admitted");
    let mut visibility = VisibilityTable::new();
    for entity in [first, second] {
        visibility.set(peer, entity, Visibility::full());
    }
    let mut ownership = OwnershipTable::new();
    ownership.set(peer, vec![first, second]);
    let mut driver = NetDriver::new(world, visibility, ownership);
    driver.server_tick = 1_000;
    driver
        .bind_peer(peer, E1, &generous_rates())
        .expect("peer binds");
    driver.assign(peer);
    (driver, peer, first, second)
}

fn end_turn_bytes(epoch: [u8; 16], seq: u64, observed: u64, actor: NetId) -> Vec<u8> {
    codec::encode_intent(&IntentFrame {
        epoch,
        lane: LANE_COMBAT,
        seq,
        observed_tick: observed,
        actor,
        body: IntentBody::EndTurn,
    })
    .expect("fixture intent encodes")
}

/// A fresh rig that has admitted `B1` and lost its receipt, then resumed.
fn rig_after_lost_receipt() -> (NetDriver, PeerId, EntityId, EntityId, Vec<u8>, NetId) {
    let (mut driver, peer, first, second) = rig();
    let x = driver
        .world
        .combat()
        .expect("encounter started")
        .active
        .expect("a turn is active");
    let net_x = driver.net_of(peer, x).expect("X mapped");
    let b1 = end_turn_bytes(E1, 1, 1_000, net_x);
    let _ = driver.admit(peer, &b1);
    assert_eq!(driver.next_seq(peer), Some(2));
    let mut stand = Stand::new(E1);
    stand.decision = ResumeDecision::Accept {
        progress: vec![progress(
            0,
            driver.next_seq(peer).expect("peer is bound") - 1,
        )],
    };
    let mut client = Client::new(E1);
    let _ = resume_once(&mut stand, &mut client).expect("resume answers");
    (driver, peer, first, second, b1, net_x)
}

// ---------------------------------------------------------------------------
// Case 18.
// ---------------------------------------------------------------------------

#[test]
fn retry_identity_across_resume() {
    let (mut driver, peer, first, second) = rig();
    let x = driver
        .world
        .combat()
        .expect("encounter started")
        .active
        .expect("a turn is active");
    let y = if x == first { second } else { first };
    let net_x = driver.net_of(peer, x).expect("X mapped");
    let b1 = end_turn_bytes(E1, 1, 1_000, net_x);

    // Step 1: the receipt is lost.
    let lost = driver.admit(peer, &b1);
    assert!(!lost.is_empty(), "the lost receipt is a real receipt");
    let executions = driver.executions();
    let hash = driver.authoritative_hash();
    assert_eq!(executions, 1);
    assert_eq!(driver.next_seq(peer), Some(2));

    // Step 2: resume, reporting the retained ledger.
    let mut stand = Stand::new(E1);
    stand.decision = ResumeDecision::Accept {
        progress: vec![progress(
            0,
            driver.next_seq(peer).expect("peer is bound") - 1,
        )],
    };
    let mut client = Client::new(E1);
    let got = resume_once(&mut stand, &mut client).expect("resume answers");
    assert_eq!(
        got,
        ResumeResult::Accepted(ResumeAccepted {
            epoch: E1,
            granted: fence(2),
            progress: vec![progress(0, 1)],
        })
    );
    assert_eq!(client.fence, fence(2));

    // Step 3: the stored bytes retry as the cached receipt.
    assert_eq!(driver.admit_status(peer, &b1), ReceiptStatus::Applied);
    assert_eq!(driver.executions(), executions);
    assert_eq!(driver.authoritative_hash(), hash);
    assert_eq!(driver.next_seq(peer), Some(2));
    let receipt = driver.admit(peer, &b1);
    let frame = codec::decode_delta(&receipt).expect("receipt decodes");
    assert_eq!(
        frame.ops,
        vec![DeltaOp::Receipt {
            epoch: E1,
            lane: LANE_COMBAT,
            seq: 1,
            processed_tick: 1_000,
            status: ReceiptStatus::Applied,
        }]
    );

    // Step 4: Y now holds the turn; the ledger continues at seq 2.
    let net_y = driver.net_of(peer, y).expect("Y mapped");
    let b2 = end_turn_bytes(E1, 2, 1_000, net_y);
    assert_eq!(driver.admit_status(peer, &b2), ReceiptStatus::Applied);
    assert_eq!(driver.next_seq(peer), Some(3));
    assert_eq!(driver.executions(), executions + 1);

    // Negative control (a): re-encoding seq 1 changes the canonical bytes.
    let (mut driver, peer, _, _, _b1, net_x) = rig_after_lost_receipt();
    let reencoded = end_turn_bytes(E1, 1, 999, net_x);
    assert_eq!(
        driver.admit_status(peer, &reencoded),
        ReceiptStatus::Rejected(RejectionCode::SeqConflict)
    );

    // Negative control (b): a reset ledger judges seq 1 as new.
    let (mut driver, peer, _, _, b1, _) = rig_after_lost_receipt();
    driver
        .bind_peer(peer, E1, &generous_rates())
        .expect("peer rebinds");
    assert_eq!(
        driver.admit_status(peer, &b1),
        ReceiptStatus::Rejected(RejectionCode::IllegalAction)
    );
}

// ---------------------------------------------------------------------------
// Case 19.
// ---------------------------------------------------------------------------

/// Routes one inbound frame through the connection's fence before admission.
fn route(
    fence_state: &mut SessionFence,
    conns: &BTreeMap<u32, ConnectionFence>,
    conn: u32,
    driver: &mut NetDriver,
    peer: PeerId,
    bytes: &[u8],
) -> Result<ReceiptStatus, ResumeError> {
    let tag = *conns.get(&conn).expect("connection is known");
    fence_state.check_frame(tag)?;
    Ok(driver.admit_status(peer, bytes))
}

/// Routes one resume request through the connection's fence before `resolve`.
fn route_resume(
    fence_state: &mut SessionFence,
    conns: &BTreeMap<u32, ConnectionFence>,
    conn: u32,
    bytes: &[u8],
    decision: &ResumeDecision,
) -> Result<ResumeResult, ResumeError> {
    let tag = *conns.get(&conn).expect("connection is known");
    fence_state.check_frame(tag)?;
    let request = decode_request(bytes).expect("request decodes");
    fence_state.resolve(&request, decision)
}

#[test]
fn fenced_connection_frames_never_reach_admission() {
    let (mut driver, peer, first, second) = rig();
    let x = driver
        .world
        .combat()
        .expect("encounter started")
        .active
        .expect("a turn is active");
    let y = if x == first { second } else { first };
    let net_x = driver.net_of(peer, x).expect("X mapped");
    let net_y = driver.net_of(peer, y).expect("Y mapped");
    let b1 = end_turn_bytes(E1, 1, 1_000, net_x);
    let b2 = end_turn_bytes(E1, 2, 1_000, net_y);

    let mut stand = Stand::new(E1);
    let mut conns: BTreeMap<u32, ConnectionFence> = BTreeMap::new();
    conns.insert(1, ConnectionFence::FIRST);

    // Control: conn 1 is the current fence.
    assert_eq!(
        route(&mut stand.fence, &conns, 1, &mut driver, peer, &b1),
        Ok(ReceiptStatus::Applied)
    );
    let executions = driver.executions();
    let hash = driver.authoritative_hash();
    assert_eq!(driver.next_seq(peer), Some(2));

    // Resume on conn 2.
    stand.decision = ResumeDecision::Accept {
        progress: vec![progress(0, 1)],
    };
    let mut client = Client::new(E1);
    let got = resume_once(&mut stand, &mut client).expect("resume answers");
    let granted = granted_of(&got);
    assert_eq!(granted, 2);
    conns.insert(2, fence(granted));

    // Conn 1 is fenced: no admission, no state change.
    assert_eq!(
        route(&mut stand.fence, &conns, 1, &mut driver, peer, &b1),
        Err(ResumeError::Fenced)
    );
    assert_eq!(
        route(&mut stand.fence, &conns, 1, &mut driver, peer, &b2),
        Err(ResumeError::Fenced)
    );
    assert_eq!(driver.executions(), executions);
    assert_eq!(driver.authoritative_hash(), hash);
    assert_eq!(driver.next_seq(peer), Some(2));

    // A resume request on conn 1 is fenced before `resolve` is reached.
    let request = request_with(E1, 2, vec![claim(0, 0)]);
    let request_bytes = encode_request(&request).expect("request encodes");
    assert_eq!(
        route_resume(
            &mut stand.fence,
            &conns,
            1,
            &request_bytes,
            &ResumeDecision::Accept {
                progress: vec![progress(0, 1)],
            }
        ),
        Err(ResumeError::Fenced)
    );

    // Conn 2 works: the cached retry and the new command.
    assert_eq!(
        route(&mut stand.fence, &conns, 2, &mut driver, peer, &b1),
        Ok(ReceiptStatus::Applied)
    );
    assert_eq!(driver.executions(), executions);
    assert_eq!(
        route(&mut stand.fence, &conns, 2, &mut driver, peer, &b2),
        Ok(ReceiptStatus::Applied)
    );
    assert_eq!(driver.next_seq(peer), Some(3));

    // A second resume on conn 3, presenting the now-confirmed fence 2.
    stand.decision = ResumeDecision::Accept {
        progress: vec![progress(0, 2)],
    };
    let mut client = Client::new(E1);
    client.fence = fence(2);
    let got = resume_once(&mut stand, &mut client).expect("resume answers");
    let granted = granted_of(&got);
    assert_eq!(granted, 3);
    conns.insert(3, fence(granted));

    assert_eq!(
        route(&mut stand.fence, &conns, 2, &mut driver, peer, &b2),
        Err(ResumeError::Fenced)
    );
    assert_eq!(
        route(&mut stand.fence, &conns, 3, &mut driver, peer, &b2),
        Ok(ReceiptStatus::Applied)
    );
}

// ---------------------------------------------------------------------------
// Case 20.
// ---------------------------------------------------------------------------

#[test]
fn obsolete_transfer_never_overwrites_the_resume_snapshot() {
    let mut pool_state = pool();
    let r = pool_state
        .add_receiver(E1, DeltaWire::V1, GateMode::Live)
        .expect("receiver admitted");
    let mut replica = Replica::default();

    // Step 1: T1 publishes and pops; deltas 1–2 release and pop.
    let t1 = plan(E1, 1, 10, 0, 8, &body_n(2));
    assert_eq!(
        pool_state.offer_transfer(r, &t1[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool_state.offer_transfer(r, &t1[1], T0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    drain_into(&mut replica, &mut pool_state, r);
    for (seq, entity) in [(1, 1), (2, 2)] {
        assert_eq!(
            pool_state.offer_delta(r, &delta(seq, vec![hp(entity, 9)]), T0),
            Ok(DeltaOutcome::Released { count: 1 })
        );
        drain_into(&mut replica, &mut pool_state, r);
    }

    // Step 2: T2 is in flight when the connection drops.
    let t2 = plan(E1, 2, 20, 2, 1, &body_n(1_000));
    assert_eq!(
        pool_state.offer_transfer(r, &t2[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool_state.offer_transfer(r, &t2[1], T0),
        Ok(TransferOutcome::Stored {
            received: 1,
            chunk_count: 2,
        })
    );
    assert_eq!(
        pool_state.rebind_receiver(r, T0),
        Ok(Rebound {
            discarded: Some(tid(2)),
            dropped_deltas: 0,
        })
    );

    // Step 3: resume #1, then a cancel and T3 in flight.
    let mut stand = Stand::new(E1);
    stand.decision = ResumeDecision::Accept {
        progress: vec![progress(0, 1)],
    };
    let mut client = Client::new(E1);
    let got = resume_once(&mut stand, &mut client).expect("resume answers");
    assert_eq!(granted_of(&got), 2);
    assert_eq!(
        pool_state.offer_transfer(r, &cancel_frame(E1, 2, CancelReason::Superseded), T0),
        Ok(TransferOutcome::Cancelled { discarded: false })
    );
    let t3 = plan(E1, 3, 30, 2, 1, &body_n(1_000));
    assert_eq!(
        pool_state.offer_transfer(r, &t3[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool_state.offer_transfer(r, &t3[1], T0),
        Ok(TransferOutcome::Stored {
            received: 1,
            chunk_count: 2,
        })
    );

    // Step 4: a second disconnect.
    assert_eq!(
        pool_state.rebind_receiver(r, T0),
        Ok(Rebound {
            discarded: Some(tid(3)),
            dropped_deltas: 0,
        })
    );

    // Step 5: resume #2, cancel T3, then T4 publishes.
    stand.decision = ResumeDecision::Accept {
        progress: vec![progress(0, 2)],
    };
    let got = resume_once(&mut stand, &mut client).expect("resume answers");
    assert_eq!(granted_of(&got), 3);
    assert_eq!(
        pool_state.offer_transfer(r, &cancel_frame(E1, 3, CancelReason::Superseded), T0),
        Ok(TransferOutcome::Cancelled { discarded: false })
    );
    let t4 = plan(E1, 4, 40, 2, 8, &body_n(3));
    assert_eq!(
        pool_state.offer_transfer(r, &t4[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool_state.offer_transfer(r, &t4[1], T0),
        Ok(TransferOutcome::Published {
            transfer: tid(4),
            released: 0,
        })
    );
    drain_into(&mut replica, &mut pool_state, r);

    // Step 6: late frames of both retired transfers are stale.
    let settled = pool_state.clone();
    assert_eq!(
        pool_state.offer_transfer(r, &t2[2], T0),
        Err(TransferError::StaleTransfer)
    );
    assert_eq!(pool_state, settled);
    assert_eq!(
        pool_state.offer_transfer(r, &t3[2], T0),
        Err(TransferError::StaleTransfer)
    );
    assert_eq!(pool_state, settled);
    assert_eq!(
        pool_state.offer_transfer(r, &t2[0], T0),
        Err(TransferError::StaleTransfer)
    );
    assert_eq!(pool_state, settled);
    assert_eq!(
        pool_state.offer_transfer(r, &t3[0], T0),
        Err(TransferError::StaleTransfer)
    );
    assert_eq!(pool_state, settled);

    // Step 7: cross-epoch frames are refused, with a same-epoch control.
    let other = plan(E2, 6, 40, 2, 8, &body_n(2));
    assert_eq!(
        pool_state.offer_transfer(r, &other[0], T0),
        Err(TransferError::WrongEpoch)
    );
    assert_eq!(pool_state, settled);
    assert_eq!(
        pool_state.offer_transfer(r, &other[1], T0),
        Err(TransferError::WrongEpoch)
    );
    assert_eq!(pool_state, settled);
    assert_eq!(
        pool_state.offer_transfer(r, &cancel_frame(E2, 6, CancelReason::Superseded), T0),
        Err(TransferError::WrongEpoch)
    );
    assert_eq!(pool_state, settled);

    let t5 = plan(E1, 5, 40, 2, 8, &body_n(2));
    assert_eq!(
        pool_state.offer_transfer(r, &t5[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool_state.offer_transfer(r, &cancel_frame(E1, 5, CancelReason::Superseded), T0),
        Ok(TransferOutcome::Cancelled { discarded: true })
    );

    // Step 8: behind the replica.
    let behind_cursor = plan(E1, 6, 40, 1, 8, &body_n(2));
    assert_eq!(
        pool_state.offer_transfer(r, &behind_cursor[0], T0),
        Err(TransferError::BehindReplica)
    );
    let behind_tick = plan(E1, 6, 39, 2, 8, &body_n(2));
    assert_eq!(
        pool_state.offer_transfer(r, &behind_tick[0], T0),
        Err(TransferError::BehindReplica)
    );

    // Only T1's and T4's snapshots ever reached the replica.
    assert_eq!(replica.snapshots, 2);
    assert_eq!(
        replica.facts,
        facts(
            &[
                (1, Some((10, 10, false))),
                (2, Some((10, 10, false))),
                (3, Some((10, 10, false))),
            ],
            None,
        )
    );
    assert!(replica.facts.entities.keys().all(|id| *id <= 3));
}

// ---------------------------------------------------------------------------
// Case 21.
// ---------------------------------------------------------------------------

/// The pre-gap body, hand-written from S§18 case 21.
const PRE_GAP_BODY: [u8; 20] = [
    0x03, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x02, 0x01, 0x07, 0x0A, 0x00, 0x03, 0x01, 0x03, 0x03, 0x00,
    0x01, 0x01, 0x01, 0x02,
];
/// The post-gap body: A 8/10, B present without health, C absent.
const POST_GAP_BODY: [u8; 12] = [
    0x02, 0x01, 0x01, 0x08, 0x0A, 0x00, 0x02, 0x00, 0x01, 0x01, 0x01, 0x02,
];

#[derive(Debug, Clone)]
struct Row {
    entity: u64,
    present: bool,
    health: HealthTuple,
    health_granted: bool,
}

/// The host stand-in's facts and grants for case 21. The body is built from
/// granted facts only.
#[derive(Debug, Clone)]
struct Truth {
    rows: Vec<Row>,
    turn: Option<u64>,
    round: u64,
    turn_granted: bool,
}

impl Truth {
    fn body(&self) -> SnapshotBody {
        let entities: Vec<SnapshotEntity> = self
            .rows
            .iter()
            .filter(|row| row.present)
            .map(|row| SnapshotEntity {
                entity: net(row.entity),
                health: row
                    .health_granted
                    .then(|| health(row.health.0, row.health.1, row.health.2)),
            })
            .collect();
        let turn = self.turn_granted.then(|| TurnFact {
            active: self
                .turn
                .filter(|active| entities.iter().any(|e| e.entity.get() == *active))
                .map(net),
            round: self.round,
        });
        SnapshotBody { entities, turn }
    }

    fn pre_gap() -> Truth {
        Truth {
            rows: vec![
                Row {
                    entity: 1,
                    present: true,
                    health: (10, 10, false),
                    health_granted: true,
                },
                Row {
                    entity: 2,
                    present: true,
                    health: (7, 10, false),
                    health_granted: true,
                },
                Row {
                    entity: 3,
                    present: true,
                    health: (3, 3, false),
                    health_granted: true,
                },
            ],
            turn: Some(1),
            round: 2,
            turn_granted: true,
        }
    }

    fn post_gap() -> Truth {
        Truth {
            rows: vec![
                Row {
                    entity: 1,
                    present: true,
                    health: (8, 10, false),
                    health_granted: true,
                },
                Row {
                    entity: 2,
                    present: true,
                    health: (7, 10, false),
                    health_granted: false,
                },
                Row {
                    entity: 3,
                    present: false,
                    health: (3, 3, false),
                    health_granted: true,
                },
            ],
            turn: Some(1),
            round: 2,
            turn_granted: true,
        }
    }

    /// The stale-grant stand-in: the post-gap health, but the pre-gap grants.
    fn stale_grants() -> Truth {
        Truth {
            rows: vec![
                Row {
                    entity: 1,
                    present: true,
                    health: (8, 10, false),
                    health_granted: true,
                },
                Row {
                    entity: 2,
                    present: true,
                    health: (7, 10, false),
                    health_granted: true,
                },
                Row {
                    entity: 3,
                    present: true,
                    health: (3, 3, false),
                    health_granted: true,
                },
            ],
            turn: Some(1),
            round: 2,
            turn_granted: true,
        }
    }
}

fn pre_gap_facts() -> Facts {
    facts(
        &[
            (1, Some((10, 10, false))),
            (2, Some((7, 10, false))),
            (3, Some((3, 3, false))),
        ],
        Some((Some(1), 2)),
    )
}

fn post_gap_facts() -> Facts {
    facts(&[(1, Some((8, 10, false))), (2, None)], Some((Some(1), 2)))
}

/// A client that has published and popped the pre-gap `T1`.
fn pre_gap_receiver(pre: &Truth) -> (ReassemblyPool, ReceiverId, Replica) {
    let mut pool_state = pool();
    let r = pool_state
        .add_receiver(E1, DeltaWire::V1, GateMode::Live)
        .expect("receiver admitted");
    let mut replica = Replica::default();
    let t1 = plan(E1, 1, 10, 0, 1, &pre.body());
    assert_eq!(chunk_payload(&t1[1]), PRE_GAP_BODY);
    assert_eq!(
        pool_state.offer_transfer(r, &t1[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool_state.offer_transfer(r, &t1[1], T0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    drain_into(&mut replica, &mut pool_state, r);
    assert_matches(&replica, &pre_gap_facts());
    (pool_state, r, replica)
}

#[test]
fn resume_snapshot_oracle_and_corruption() {
    let pre = Truth::pre_gap();
    let post = Truth::post_gap();

    // The unmodified resume transfer matches the hand-authored view.
    let (mut pool_state, r, mut replica) = pre_gap_receiver(&pre);
    assert_eq!(
        pool_state.rebind_receiver(r, T0),
        Ok(Rebound {
            discarded: None,
            dropped_deltas: 0,
        })
    );
    let mut stand = Stand::new(E1);
    stand.decision = ResumeDecision::Accept {
        progress: vec![progress(0, 0)],
    };
    let mut client = Client::new(E1);
    let _ = resume_once(&mut stand, &mut client).expect("resume answers");
    let t2 = plan(E1, 2, 50, 0, 1, &post.body());
    assert_eq!(chunk_payload(&t2[1]), POST_GAP_BODY);
    assert_eq!(
        pool_state.offer_transfer(r, &t2[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool_state.offer_transfer(r, &t2[1], T0),
        Ok(TransferOutcome::Published {
            transfer: tid(2),
            released: 0,
        })
    );
    drain_into(&mut replica, &mut pool_state, r);
    assert_matches(&replica, &post_gap_facts());

    // (a) Corruption: the chunk payload's byte 3 changes 08 -> 09.
    let mut corrupted = POST_GAP_BODY.to_vec();
    assert_eq!(corrupted[3], 0x08);
    corrupted[3] = 0x09;
    let (mut corrupt_pool, corrupt_r, mut corrupt_replica) = pre_gap_receiver(&pre);
    assert_eq!(
        corrupt_pool.rebind_receiver(corrupt_r, T0),
        Ok(Rebound {
            discarded: None,
            dropped_deltas: 0,
        })
    );
    let begin = plan(E1, 2, 50, 0, 1, &post.body())[0].clone();
    let chunk = chunk_frame(E1, 2, 0, &corrupted);
    assert_eq!(
        corrupt_pool.offer_transfer(corrupt_r, &begin, T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        corrupt_pool.offer_transfer(corrupt_r, &chunk, T0),
        Ok(TransferOutcome::Published {
            transfer: tid(2),
            released: 0,
        })
    );
    drain_into(&mut corrupt_replica, &mut corrupt_pool, corrupt_r);
    let mismatch = diff(&corrupt_replica.facts, &post_gap_facts());
    assert!(
        mismatch.iter().any(|m| m.contains("entity 1")),
        "corruption must be visible: {mismatch:?}"
    );

    // (b) Dropped chunk: the transfer expires; the replica keeps the pre-gap
    // view until a resync replaces it.
    let (mut drop_pool, drop_r, mut drop_replica) = pre_gap_receiver(&pre);
    assert_eq!(
        drop_pool.rebind_receiver(drop_r, T0),
        Ok(Rebound {
            discarded: None,
            dropped_deltas: 0,
        })
    );
    assert_eq!(
        drop_pool.offer_transfer(drop_r, &t2[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(drop_pool.poll(T0 + 4_999), Vec::<ReceiverId>::new());
    assert_eq!(drop_pool.poll(T0 + 5_000), vec![drop_r]);
    assert_eq!(drop_pool.pop_update(drop_r), None);
    assert_matches(&drop_replica, &pre_gap_facts());
    let stale_view = diff(&drop_replica.facts, &post_gap_facts());
    assert!(stale_view.iter().any(|m| m.contains("entity 2")));
    assert!(stale_view.iter().any(|m| m.contains("entity 3")));

    // Recovery: the client's Resync makes the stand-in send T3.
    let resync = codec_transfer::encode_up(&TransferUp::Resync {
        epoch: E1,
        after: Some(tid(2)),
        reason: ResyncReason::Expired,
    })
    .expect("resync encodes");
    assert!(codec_transfer::decode_up(&resync).is_ok());
    let t3 = plan(E1, 3, 50, 0, 1, &post.body());
    assert_eq!(
        drop_pool.offer_transfer(drop_r, &t3[0], T0 + 5_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        drop_pool.offer_transfer(drop_r, &t3[1], T0 + 5_000),
        Ok(TransferOutcome::Published {
            transfer: tid(3),
            released: 0,
        })
    );
    drain_into(&mut drop_replica, &mut drop_pool, drop_r);
    assert_matches(&drop_replica, &post_gap_facts());

    // (c) Stale grants: the old grants leak B's health and C's presence.
    let stale = Truth::stale_grants();
    let (mut stale_pool, stale_r, mut stale_replica) = pre_gap_receiver(&pre);
    assert_eq!(
        stale_pool.rebind_receiver(stale_r, T0),
        Ok(Rebound {
            discarded: None,
            dropped_deltas: 0,
        })
    );
    let t2_stale = plan(E1, 2, 50, 0, 1, &stale.body());
    assert_eq!(
        chunk_payload(&t2_stale[1]),
        [
            0x03, 0x01, 0x01, 0x08, 0x0A, 0x00, 0x02, 0x01, 0x07, 0x0A, 0x00, 0x03, 0x01, 0x03,
            0x03, 0x00, 0x01, 0x01, 0x01, 0x02,
        ]
    );
    assert_eq!(
        stale_pool.offer_transfer(stale_r, &t2_stale[0], T0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        stale_pool.offer_transfer(stale_r, &t2_stale[1], T0),
        Ok(TransferOutcome::Published {
            transfer: tid(2),
            released: 0,
        })
    );
    drain_into(&mut stale_replica, &mut stale_pool, stale_r);
    let leaks = diff(&stale_replica.facts, &post_gap_facts());
    assert!(
        leaks.iter().any(|m| m.contains("entity 2")),
        "B's health must leak: {leaks:?}"
    );
    assert!(
        leaks.iter().any(|m| m.contains("entity 3")),
        "C must leak: {leaks:?}"
    );
}
