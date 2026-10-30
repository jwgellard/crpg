//! T025a1 resume wire suite (`tasks/T025a.md` S§11, cases 1–10), public API
//! only, pure functions (no pool, no clock).
//!
//! Wire bytes for the codec cases are hand-written from S§6 and are never
//! produced by the codec under test. Requests and tickets are compared
//! through `as_bytes()` and by re-encoding, because they have no `PartialEq`.

use core::marker::PhantomData;

use crpg_net::codec;
use crpg_net::codec_resume::{
    decode_down, decode_request, encode_down, encode_request, CodecError,
};
use crpg_net::codec_transfer;
use crpg_net::codec_v2;
use crpg_net::protocol::{self, LANE_COMBAT, MAX_INTENT_FRAME_BYTES, SEQ_LAST};
use crpg_net::protocol_resume::*;
use crpg_net::protocol_transfer::{self, TransferChunk, TransferDown, TransferId};
use crpg_net::protocol_v2;

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
    let ticket_p_dbg = format!("{ticket_p:?}");
    let ticket_aa_dbg = format!("{ticket_aa:?}");
    assert_eq!(ticket_p_dbg, ticket_aa_dbg);
    assert!(ticket_p_dbg.contains("ResumeProof(<redacted>)"));

    let raw_ticket_p = ResumeTicket {
        epoch: ZERO_EPOCH,
        proof: proof(&opaque),
    };
    let raw_ticket_aa = ResumeTicket {
        epoch: ZERO_EPOCH,
        proof: proof(&aa16),
    };
    let raw_ticket_p_dbg = format!("{raw_ticket_p:?}");
    let raw_ticket_aa_dbg = format!("{raw_ticket_aa:?}");
    assert_eq!(raw_ticket_p_dbg, raw_ticket_aa_dbg);
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
