//! Bounded `postcard` codec for the explicit v2 wire (T021, ADR-0019).
//!
//! v1 ([`crate::codec`]) is frozen and untouched: this module duplicates the
//! small staged-decode skeleton with version byte 2 and the three new event
//! tags rather than sharing a version-parameterized path that could blur v1
//! precedence. Calling [`decode_intent`]/[`decode_delta`] here is explicit
//! version selection: byte 1 is
//! [`CodecError::UnsupportedVersion`], never reinterpreted.
//!
//! v2 intent is identical to v1 except its first byte is 2. v2 delta header
//! is `version:u8=2, lane:u8=0, server_tick:u64, event_seq:u64,
//! op_count:u64` with the existing `postcard` primitive conventions. Tags
//! 0..7 and every following payload byte are identical to v1; new tag
//! payloads follow this exact order:
//!
//! - 8 (`ActionResolved`): `actor:u64`, `target:u64`, `ability:ULID-text`,
//!   `outcome:UTF8-text`, `damage:u32`.
//! - 9 (`TurnStarted`): `actor:u64`, `round:u64`.
//! - 10 (`EncounterEnded`): `encounter:ULID-text`, `round:u64`.
//!
//! ULID-text is a `postcard` length-prefixed 26-character uppercase string;
//! decoding accepts core's case aliases and re-encodes canonically. Outcome
//! is T020's exact five symbolic forms. No raw `EntityId`, sim sequence,
//! internal operation id, roll, DC, margin, or grant is transmitted. Receipt
//! payload remains exactly epoch/lane/seq/tick/status.
//!
//! Decode order is contract: whole-frame cap → version → lane → header
//! scalars → op-count cap before vector growth → each tag then its fields →
//! trailing-byte reject. A failure prevents examining later fields.
//! Nonzero ids and intent/receipt sequence ranges are checked when read,
//! matching actual v1 behavior; delta `event_seq` accepts the full `u64`
//! representation (the host issues `1..=u64::MAX-1` and refuses exhaustion
//! without wrapping). Outcome byte-length errors are
//! [`CodecError::LimitExceeded`]; bad UTF-8, truncation, short bodies, and
//! ULID/outcome semantic failures are [`CodecError::Malformed`]; unknown op
//! or receipt tags/codes are [`CodecError::UnknownMessage`]; a receipt
//! echoing a foreign lane is [`CodecError::WrongDirection`]. Empty op lists
//! are allowed. Encode checks lane, count, then op semantics in order, then
//! the final frame length, and never returns partial bytes. The op vector
//! grows by push, never `with_capacity` from the untrusted count.
//!
//! Bounded-before-allocation (the T020 lesson): string lengths are read as
//! bounded integers and checked against
//! [`MAX_WIRE_STRING_BYTES`](crate::protocol::MAX_WIRE_STRING_BYTES)
//! **before** allocating, copying, or checking available body bytes; then
//! body availability and UTF-8 are checked, then ULID/outcome semantics.
//! This module therefore never copies v1's owned-string-before-check shape.
//!
//! The codec checks shape, not entitlement, and never touches `World`, RNG,
//! or the event queue.

use crpg_core::Ulid;

use crate::protocol::{
    DeltaOp as DeltaOpV1, IntentBody, IntentFrame, NetId, ReceiptStatus, RejectionCode,
    DELTA_TAG_DESPAWNED, DELTA_TAG_DIED, DELTA_TAG_ENTITY_ENTER, DELTA_TAG_ENTITY_LEAVE,
    DELTA_TAG_HEALTH, DELTA_TAG_RECEIPT, DELTA_TAG_SPAWNED, DELTA_TAG_TURN,
    INTENT_TAG_DECLARE_ACTION, INTENT_TAG_END_TURN, LANE_COMBAT, MAX_DELTA_FRAME_BYTES,
    MAX_DELTA_OPS, MAX_INTENT_FRAME_BYTES, MAX_WIRE_STRING_BYTES, RECEIPT_TAG_APPLIED,
    RECEIPT_TAG_REJECTED, SEQ_FIRST, SEQ_LAST,
};
use crate::protocol_v2::{
    DeltaFrame, DeltaOp, DELTA_TAG_ACTION_RESOLVED, DELTA_TAG_ENCOUNTER_ENDED,
    DELTA_TAG_TURN_STARTED, PROTOCOL_VERSION,
};

/// Shape failure of the explicit v2 wire: the same six variants as v1.
pub use crate::codec::CodecError;

// ---------------------------------------------------------------------------
// postcard plumbing (v1-frozen shapes duplicated, never shared mutably).
// ---------------------------------------------------------------------------

/// Extends `out` with one `postcard`-encoded primitive.
///
/// Primitive extension into a `Vec` is infallible; a failure here would be
/// an internal encoding bug, reported as [`CodecError::Malformed`] so
/// encode stays total without panicking.
fn extend<T: serde::Serialize + ?Sized>(out: Vec<u8>, value: &T) -> Result<Vec<u8>, CodecError> {
    postcard::to_extend(value, out).map_err(|_| CodecError::Malformed)
}

/// Decodes one `postcard` primitive from the head of `rest`, advancing it.
///
/// Every `postcard` decode failure (truncation, bad varint/bool) is a
/// [`CodecError::Malformed`]: discriminant dispatch happens on raw tag
/// bytes matched explicitly by the caller, never inferred from this error.
fn take<'a, T: serde::Deserialize<'a>>(rest: &mut &'a [u8]) -> Result<T, CodecError> {
    let (value, remaining) =
        postcard::take_from_bytes::<T>(rest).map_err(|_| CodecError::Malformed)?;
    *rest = remaining;
    Ok(value)
}

/// Decodes a nonzero replica id.
fn take_net_id(rest: &mut &[u8]) -> Result<NetId, CodecError> {
    let raw: u64 = take(rest)?;
    NetId::new(raw).ok_or(CodecError::Malformed)
}

/// Decodes a lane-0 command sequence number (`SEQ_FIRST..=SEQ_LAST`).
fn take_seq(rest: &mut &[u8]) -> Result<u64, CodecError> {
    let seq: u64 = take(rest)?;
    if !(SEQ_FIRST..=SEQ_LAST).contains(&seq) {
        return Err(CodecError::Malformed);
    }
    Ok(seq)
}

/// Validates an encode-side sequence number.
fn check_seq(seq: u64) -> Result<(), CodecError> {
    if !(SEQ_FIRST..=SEQ_LAST).contains(&seq) {
        return Err(CodecError::Malformed);
    }
    Ok(())
}

/// Decodes one unsigned LEB128 `postcard` varint from the head of `rest`.
///
/// Used only to read string length prefixes before any body inspection, so
/// a hostile length reports [`CodecError::LimitExceeded`] even when its
/// body is absent. A truncated or over-long varint cannot yield a length
/// and is [`CodecError::Malformed`].
fn take_varint_len(rest: &mut &[u8]) -> Result<u64, CodecError> {
    let mut value: u64 = 0;
    for shift in (0..64).step_by(7) {
        if rest.is_empty() {
            return Err(CodecError::Malformed);
        }
        let byte = rest[0];
        *rest = &rest[1..];
        // Only bit 63 remains in the tenth byte. Reject overflow before
        // shifting, otherwise a hostile length can wrap below the bound.
        if shift == 63 && byte > 1 {
            return Err(CodecError::Malformed);
        }
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(CodecError::Malformed)
}

/// Borrows one length-prefixed string after its bounded length check.
///
/// The varint length is checked against
/// [`MAX_WIRE_STRING_BYTES`](crate::protocol::MAX_WIRE_STRING_BYTES)
/// before body availability or UTF-8 is examined, so no allocation or copy
/// is driven by an untrusted length. Nothing here allocates: the returned
/// slice borrows the frame.
fn take_bounded_str<'a>(rest: &mut &'a [u8]) -> Result<&'a str, CodecError> {
    let len = take_varint_len(rest)?;
    if len > MAX_WIRE_STRING_BYTES as u64 {
        return Err(CodecError::LimitExceeded);
    }
    let len = len as usize;
    if rest.len() < len {
        return Err(CodecError::Malformed);
    }
    let (body, remaining) = rest.split_at(len);
    let text = core::str::from_utf8(body).map_err(|_| CodecError::Malformed)?;
    *rest = remaining;
    Ok(text)
}

/// Decodes one ULID from its bounded length-prefixed canonical text.
///
/// The byte-length bound fires before parsing, so a hostile length prefix
/// inside the frame cap reports [`CodecError::LimitExceeded`], while text
/// that fits but is not a ULID reports [`CodecError::Malformed`].
fn take_ulid(rest: &mut &[u8]) -> Result<Ulid, CodecError> {
    let text = take_bounded_str(rest)?;
    text.parse::<Ulid>().map_err(|_| CodecError::Malformed)
}

/// Reports whether `text` names T020's exact symbolic outcome vocabulary.
pub(crate) fn valid_outcome(text: &str) -> bool {
    match text {
        "critical_success" | "success" | "failure" | "critical_failure" => true,
        _ => {
            let Some(rest) = text.strip_prefix("custom:") else {
                return false;
            };
            if rest.is_empty() || rest.len() > 3 {
                return false;
            }
            if !rest.bytes().all(|byte| byte.is_ascii_digit()) {
                return false;
            }
            if rest.len() > 1 && rest.starts_with('0') {
                return false;
            }
            match rest.parse::<u32>() {
                Ok(number) => number <= 255,
                Err(_) => false,
            }
        }
    }
}

/// Validates an encode-side outcome: byte bound, then symbolic validity.
fn check_outcome(text: &str) -> Result<(), CodecError> {
    if text.len() > MAX_WIRE_STRING_BYTES {
        return Err(CodecError::LimitExceeded);
    }
    if !valid_outcome(text) {
        return Err(CodecError::Malformed);
    }
    Ok(())
}

/// Decodes one outcome string: bounded length, body, UTF-8, then symbolic
/// validity. Length errors are [`CodecError::LimitExceeded`]; a fitting
/// but non-vocabulary outcome is [`CodecError::Malformed`].
fn take_outcome(rest: &mut &[u8]) -> Result<String, CodecError> {
    let text = take_bounded_str(rest)?;
    if !valid_outcome(text) {
        return Err(CodecError::Malformed);
    }
    Ok(text.to_owned())
}

/// Validates an encode-side ULID text length (canonical text is always 26
/// bytes; the bound still applies so the check is explicit).
fn check_ulid_text(text: &str) -> Result<(), CodecError> {
    if text.len() > MAX_WIRE_STRING_BYTES {
        return Err(CodecError::LimitExceeded);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Intent codec (v1 shape, version byte 2).
// ---------------------------------------------------------------------------

/// Encodes one intent frame to its canonical v2 bytes (first byte 2).
///
/// Enforces the encode-side bounds (lane, `seq` range) and rejects the
/// result with [`CodecError::FrameTooLarge`] if it exceeds
/// [`MAX_INTENT_FRAME_BYTES`](crate::protocol::MAX_INTENT_FRAME_BYTES).
pub fn encode_intent(frame: &IntentFrame) -> Result<Vec<u8>, CodecError> {
    if frame.lane != LANE_COMBAT {
        return Err(CodecError::WrongDirection);
    }
    check_seq(frame.seq)?;
    let mut out = extend(Vec::new(), &PROTOCOL_VERSION)?;
    out = extend(out, &frame.epoch)?;
    out = extend(out, &frame.lane)?;
    out = extend(out, &frame.seq)?;
    out = extend(out, &frame.observed_tick)?;
    out = extend(out, &frame.actor.get())?;
    match frame.body {
        IntentBody::DeclareAction { ability, target } => {
            out = extend(out, &INTENT_TAG_DECLARE_ACTION)?;
            let text = ability.to_string();
            check_ulid_text(&text)?;
            out = extend(out, &text)?;
            out = extend(out, &target.get())?;
        }
        IntentBody::EndTurn => {
            out = extend(out, &INTENT_TAG_END_TURN)?;
        }
    }
    if out.len() > MAX_INTENT_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    Ok(out)
}

/// Decodes one intent frame from its v2 bytes.
///
/// Applies the staged checks in order: frame cap, version (byte 1 is
/// [`CodecError::UnsupportedVersion`]), lane, body tag, string bound,
/// complete decode with no trailing bytes, then the nonzero-id and
/// `seq`-range value checks.
pub fn decode_intent(bytes: &[u8]) -> Result<IntentFrame, CodecError> {
    if bytes.len() > MAX_INTENT_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    let mut rest = bytes;
    let version: u8 = take(&mut rest)?;
    if version != PROTOCOL_VERSION {
        return Err(CodecError::UnsupportedVersion);
    }
    let epoch: [u8; 16] = take(&mut rest)?;
    let lane: u8 = take(&mut rest)?;
    if lane != LANE_COMBAT {
        return Err(CodecError::WrongDirection);
    }
    let seq = take_seq(&mut rest)?;
    let observed_tick: u64 = take(&mut rest)?;
    let actor = take_net_id(&mut rest)?;
    let tag: u8 = take(&mut rest)?;
    let body = match tag {
        INTENT_TAG_DECLARE_ACTION => {
            let ability = take_ulid(&mut rest)?;
            let target = take_net_id(&mut rest)?;
            IntentBody::DeclareAction { ability, target }
        }
        INTENT_TAG_END_TURN => IntentBody::EndTurn,
        _ => return Err(CodecError::UnknownMessage),
    };
    if !rest.is_empty() {
        return Err(CodecError::Malformed);
    }
    Ok(IntentFrame {
        epoch,
        lane,
        seq,
        observed_tick,
        actor,
        body,
    })
}

// ---------------------------------------------------------------------------
// Delta codec.
// ---------------------------------------------------------------------------

/// Encodes the receipt status tag and, when rejected, its frozen code byte.
fn extend_status(mut out: Vec<u8>, status: &ReceiptStatus) -> Result<Vec<u8>, CodecError> {
    match status {
        ReceiptStatus::Applied => extend(out, &RECEIPT_TAG_APPLIED),
        ReceiptStatus::Rejected(code) => {
            out = extend(out, &RECEIPT_TAG_REJECTED)?;
            extend(out, &code.as_u8())
        }
    }
}

/// Decodes a receipt status tag and optional frozen code byte.
fn take_status(rest: &mut &[u8]) -> Result<ReceiptStatus, CodecError> {
    let tag: u8 = take(rest)?;
    match tag {
        RECEIPT_TAG_APPLIED => Ok(ReceiptStatus::Applied),
        RECEIPT_TAG_REJECTED => {
            let code: u8 = take(rest)?;
            RejectionCode::from_u8(code)
                .map(ReceiptStatus::Rejected)
                .ok_or(CodecError::UnknownMessage)
        }
        _ => Err(CodecError::UnknownMessage),
    }
}

/// Encodes one frozen v1 op, including its tag byte, for [`DeltaOp::Legacy`].
fn extend_legacy_op(mut out: Vec<u8>, op: &DeltaOpV1) -> Result<Vec<u8>, CodecError> {
    match op {
        DeltaOpV1::EntityEnter { entity } => {
            out = extend(out, &DELTA_TAG_ENTITY_ENTER)?;
            extend(out, &entity.get())
        }
        DeltaOpV1::EntityLeave { entity } => {
            out = extend(out, &DELTA_TAG_ENTITY_LEAVE)?;
            extend(out, &entity.get())
        }
        DeltaOpV1::Spawned { entity } => {
            out = extend(out, &DELTA_TAG_SPAWNED)?;
            extend(out, &entity.get())
        }
        DeltaOpV1::Despawned { entity } => {
            out = extend(out, &DELTA_TAG_DESPAWNED)?;
            extend(out, &entity.get())
        }
        DeltaOpV1::Died { entity } => {
            out = extend(out, &DELTA_TAG_DIED)?;
            extend(out, &entity.get())
        }
        DeltaOpV1::Health {
            entity,
            health,
            max_health,
            dead,
        } => {
            out = extend(out, &DELTA_TAG_HEALTH)?;
            out = extend(out, &entity.get())?;
            out = extend(out, health)?;
            out = extend(out, max_health)?;
            extend(out, dead)
        }
        DeltaOpV1::Turn { active, round } => {
            out = extend(out, &DELTA_TAG_TURN)?;
            match active {
                None => {
                    out = extend(out, &false)?;
                }
                Some(entity) => {
                    out = extend(out, &true)?;
                    out = extend(out, &entity.get())?;
                }
            }
            extend(out, round)
        }
        DeltaOpV1::Receipt {
            epoch,
            lane,
            seq,
            processed_tick,
            status,
        } => {
            if *lane != LANE_COMBAT {
                return Err(CodecError::WrongDirection);
            }
            check_seq(*seq)?;
            out = extend(out, &DELTA_TAG_RECEIPT)?;
            out = extend(out, epoch)?;
            out = extend(out, lane)?;
            out = extend(out, seq)?;
            out = extend(out, processed_tick)?;
            extend_status(out, status)
        }
    }
}

/// Decodes one frozen v1 op for [`DeltaOp::Legacy`] after its tag byte.
///
/// The tag is passed in because [`take_op`] already consumed it from the
/// cursor; the payload fields decode in the exact v1 order.
fn take_legacy_op_after(tag: u8, rest: &mut &[u8]) -> Result<DeltaOpV1, CodecError> {
    match tag {
        DELTA_TAG_ENTITY_ENTER => Ok(DeltaOpV1::EntityEnter {
            entity: take_net_id(rest)?,
        }),
        DELTA_TAG_ENTITY_LEAVE => Ok(DeltaOpV1::EntityLeave {
            entity: take_net_id(rest)?,
        }),
        DELTA_TAG_SPAWNED => Ok(DeltaOpV1::Spawned {
            entity: take_net_id(rest)?,
        }),
        DELTA_TAG_DESPAWNED => Ok(DeltaOpV1::Despawned {
            entity: take_net_id(rest)?,
        }),
        DELTA_TAG_DIED => Ok(DeltaOpV1::Died {
            entity: take_net_id(rest)?,
        }),
        DELTA_TAG_HEALTH => Ok(DeltaOpV1::Health {
            entity: take_net_id(rest)?,
            health: take(rest)?,
            max_health: take(rest)?,
            dead: take(rest)?,
        }),
        DELTA_TAG_TURN => {
            let present: bool = take(rest)?;
            let active = if present {
                Some(take_net_id(rest)?)
            } else {
                None
            };
            Ok(DeltaOpV1::Turn {
                active,
                round: take(rest)?,
            })
        }
        DELTA_TAG_RECEIPT => {
            let epoch: [u8; 16] = take(rest)?;
            let lane: u8 = take(rest)?;
            if lane != LANE_COMBAT {
                return Err(CodecError::WrongDirection);
            }
            let seq = take_seq(rest)?;
            let processed_tick: u64 = take(rest)?;
            Ok(DeltaOpV1::Receipt {
                epoch,
                lane,
                seq,
                processed_tick,
                status: take_status(rest)?,
            })
        }
        _ => Err(CodecError::UnknownMessage),
    }
}

/// Encodes one v2 delta operation, including its tag byte.
///
/// [`DeltaOp::Legacy`] contributes no extra tag: the wrapped v1 op encodes
/// to exactly its v1 tag and payload bytes.
fn extend_op(mut out: Vec<u8>, op: &DeltaOp) -> Result<Vec<u8>, CodecError> {
    match op {
        DeltaOp::Legacy(inner) => extend_legacy_op(out, inner),
        DeltaOp::ActionResolved {
            actor,
            target,
            ability,
            outcome,
            damage,
        } => {
            out = extend(out, &DELTA_TAG_ACTION_RESOLVED)?;
            out = extend(out, &actor.get())?;
            out = extend(out, &target.get())?;
            let text = ability.to_string();
            check_ulid_text(&text)?;
            out = extend(out, &text)?;
            check_outcome(outcome)?;
            out = extend(out, outcome.as_str())?;
            extend(out, damage)
        }
        DeltaOp::TurnStarted { actor, round } => {
            out = extend(out, &DELTA_TAG_TURN_STARTED)?;
            out = extend(out, &actor.get())?;
            extend(out, round)
        }
        DeltaOp::EncounterEnded { encounter, round } => {
            out = extend(out, &DELTA_TAG_ENCOUNTER_ENDED)?;
            let text = encounter.to_string();
            check_ulid_text(&text)?;
            out = extend(out, &text)?;
            extend(out, round)
        }
    }
}

/// Decodes one v2 delta operation after its tag byte.
///
/// Tags 0..7 decode to [`DeltaOp::Legacy`] with payload bytes identical to
/// v1; tags 8..=10 decode the new event payloads in exact field order; any
/// other tag (including 11) is [`CodecError::UnknownMessage`].
fn take_op(rest: &mut &[u8]) -> Result<DeltaOp, CodecError> {
    let tag: u8 = take(rest)?;
    match tag {
        DELTA_TAG_ENTITY_ENTER
        | DELTA_TAG_ENTITY_LEAVE
        | DELTA_TAG_SPAWNED
        | DELTA_TAG_DESPAWNED
        | DELTA_TAG_DIED
        | DELTA_TAG_HEALTH
        | DELTA_TAG_TURN
        | DELTA_TAG_RECEIPT => Ok(DeltaOp::Legacy(take_legacy_op_after(tag, rest)?)),
        DELTA_TAG_ACTION_RESOLVED => {
            let actor = take_net_id(rest)?;
            let target = take_net_id(rest)?;
            let ability = take_ulid(rest)?;
            let outcome = take_outcome(rest)?;
            let damage: u32 = take(rest)?;
            Ok(DeltaOp::ActionResolved {
                actor,
                target,
                ability,
                outcome,
                damage,
            })
        }
        DELTA_TAG_TURN_STARTED => {
            let actor = take_net_id(rest)?;
            let round: u64 = take(rest)?;
            Ok(DeltaOp::TurnStarted { actor, round })
        }
        DELTA_TAG_ENCOUNTER_ENDED => {
            let encounter = take_ulid(rest)?;
            let round: u64 = take(rest)?;
            Ok(DeltaOp::EncounterEnded { encounter, round })
        }
        _ => Err(CodecError::UnknownMessage),
    }
}

/// Encodes one v2 delta frame to its canonical bytes.
///
/// Enforces the encode-side bounds (lane, op count, receipt lane/`seq`,
/// ULID/outcome shape in payload order) and rejects the result with
/// [`CodecError::FrameTooLarge`] if it exceeds
/// [`MAX_DELTA_FRAME_BYTES`](crate::protocol::MAX_DELTA_FRAME_BYTES). No
/// partial bytes are returned on failure. Delta `event_seq` accepts the
/// full `u64` representation: the host issues `1..=u64::MAX-1` and refuses
/// exhaustion without wrapping, but the codec imposes no range of its own.
pub fn encode_delta(frame: &DeltaFrame) -> Result<Vec<u8>, CodecError> {
    if frame.lane != LANE_COMBAT {
        return Err(CodecError::WrongDirection);
    }
    if frame.ops.len() > MAX_DELTA_OPS {
        return Err(CodecError::LimitExceeded);
    }
    let mut out = extend(Vec::new(), &PROTOCOL_VERSION)?;
    out = extend(out, &frame.lane)?;
    out = extend(out, &frame.server_tick)?;
    out = extend(out, &frame.event_seq)?;
    out = extend(out, &(frame.ops.len() as u64))?;
    for op in &frame.ops {
        out = extend_op(out, op)?;
    }
    if out.len() > MAX_DELTA_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    Ok(out)
}

/// Decodes one v2 delta frame from its bytes.
///
/// Applies the staged checks in order: frame cap, version (byte 1 is
/// [`CodecError::UnsupportedVersion`]), lane, header scalars, op-count
/// bound before any per-op decode or reservation (the vector grows by
/// push), per-tag fields with bounded-before-allocation strings, then the
/// empty-remainder check.
pub fn decode_delta(bytes: &[u8]) -> Result<DeltaFrame, CodecError> {
    if bytes.len() > MAX_DELTA_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    let mut rest = bytes;
    let version: u8 = take(&mut rest)?;
    if version != PROTOCOL_VERSION {
        return Err(CodecError::UnsupportedVersion);
    }
    let lane: u8 = take(&mut rest)?;
    if lane != LANE_COMBAT {
        return Err(CodecError::WrongDirection);
    }
    let server_tick: u64 = take(&mut rest)?;
    let event_seq: u64 = take(&mut rest)?;
    let count: u64 = take(&mut rest)?;
    if count > MAX_DELTA_OPS as u64 {
        return Err(CodecError::LimitExceeded);
    }
    // Grows by push: no reservation from the untrusted count.
    let mut ops = Vec::new();
    for _ in 0..count {
        ops.push(take_op(&mut rest)?);
    }
    if !rest.is_empty() {
        return Err(CodecError::Malformed);
    }
    Ok(DeltaFrame {
        lane,
        server_tick,
        event_seq,
        ops,
    })
}
