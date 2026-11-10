//! Bounded `postcard` codec for the explicit v3 wire (T140a, ADR-0029).
//!
//! v1 ([`crate::codec`]) and v2 ([`crate::codec_v2`]) are frozen and
//! untouched: this module duplicates their small staged-decode skeleton with
//! version byte 3 rather than sharing a version-parameterized path.
//! Calling [`decode_intent`]/[`decode_delta`] here is explicit version
//! selection: any other first byte (1, 2, `0x81`, `0x91`) is
//! [`CodecError::UnsupportedVersion`], never reinterpreted, and the v1 and v2
//! codecs refuse byte 3 the same way.
//!
//! Intent frame: `version=3, epoch[16], lane, seq, observed_tick, actor,
//! body tag`, then the body (tag 0: ability ULID text and target; tags 1..=4
//! nothing; tag 5: one mode byte). Delta frame: `version=3, lane,
//! server_tick, event_seq, op_count`, then each op as its tag and fields.
//! Tags 0..=10 are byte-identical to v2 except that a receipt's code byte is
//! a v3 [`RejectionCode`]. Tags 11..=13 are reserved for T140b and are
//! [`CodecError::UnknownMessage`] here.
//!
//! Decode order is contract: whole-frame cap, version, header fields (lane,
//! `seq` range, ...), op-count cap before any op is read (the vector grows by
//! push), each tag then its fields (each checked when read), trailing-byte
//! reject. Encode checks lane, count, then each op in order, then the frame
//! cap, and never returns partial bytes. Strings are bounded before
//! allocation. Wire discriminants are matched as raw bytes, never inferred
//! from serde failures.
//!
//! The codec checks shape, not entitlement, and never touches `World`.

use crpg_core::Ulid;

use crate::protocol::{
    DELTA_TAG_DESPAWNED, DELTA_TAG_DIED, DELTA_TAG_ENTITY_ENTER, DELTA_TAG_ENTITY_LEAVE,
    DELTA_TAG_HEALTH, DELTA_TAG_RECEIPT, DELTA_TAG_SPAWNED, DELTA_TAG_TURN, LANE_COMBAT,
    MAX_DELTA_FRAME_BYTES, MAX_DELTA_OPS, MAX_INTENT_FRAME_BYTES, MAX_WIRE_STRING_BYTES,
    RECEIPT_TAG_APPLIED, RECEIPT_TAG_REJECTED, SEQ_FIRST, SEQ_LAST,
};
use crate::protocol_v2::{
    DELTA_TAG_ACTION_RESOLVED, DELTA_TAG_ENCOUNTER_ENDED, DELTA_TAG_TURN_STARTED,
};
use crate::protocol_v3::{
    AutoPauseToggles, AutoPauseTrigger, DeltaFrame, DeltaOp, EncounterMode, HostFacts, IntentBody,
    IntentFrame, NetId, PauseReason, PauseRights, ReceiptStatus, RejectionCode, DELTA_TAG_CLOCK,
    DELTA_TAG_HOST_FACTS, DELTA_TAG_JOINED, DELTA_TAG_LEFT, DELTA_TAG_MODE_CHANGED,
    DELTA_TAG_ORDER_ENDED, DELTA_TAG_ROUND_STARTED, INTENT_TAG_DECLARE_ACTION, INTENT_TAG_END_TURN,
    INTENT_TAG_PAUSE, INTENT_TAG_REQUEST_MODE, INTENT_TAG_STOP, INTENT_TAG_UNPAUSE,
    PROTOCOL_VERSION,
};

/// Shape failure of the explicit v3 wire: the same six variants as v1.
pub use crate::codec::CodecError;

// ---------------------------------------------------------------------------
// postcard plumbing (v2-frozen shapes duplicated, never shared mutably).
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
/// [`CodecError::Malformed`]: discriminant dispatch happens on raw tag bytes
/// matched explicitly by the caller.
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
    check_seq(seq)?;
    Ok(seq)
}

/// Validates a sequence number.
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
/// body is absent. A truncated or over-long varint is [`CodecError::Malformed`].
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
/// [`MAX_WIRE_STRING_BYTES`] before body availability or UTF-8 is examined,
/// so no allocation or copy is driven by an untrusted length.
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
fn take_ulid(rest: &mut &[u8]) -> Result<Ulid, CodecError> {
    let text = take_bounded_str(rest)?;
    text.parse::<Ulid>().map_err(|_| CodecError::Malformed)
}

/// Validates an encode-side outcome: byte bound, then symbolic validity.
fn check_outcome(text: &str) -> Result<(), CodecError> {
    if text.len() > MAX_WIRE_STRING_BYTES {
        return Err(CodecError::LimitExceeded);
    }
    if !crate::codec_v2::valid_outcome(text) {
        return Err(CodecError::Malformed);
    }
    Ok(())
}

/// Decodes one outcome string: bounded length, body, UTF-8, then symbolic
/// validity.
fn take_outcome(rest: &mut &[u8]) -> Result<String, CodecError> {
    let text = take_bounded_str(rest)?;
    if !crate::codec_v2::valid_outcome(text) {
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

/// Decodes one mode byte; 2..=255 is `UnknownMessage`.
fn take_mode(rest: &mut &[u8]) -> Result<EncounterMode, CodecError> {
    let raw: u8 = take(rest)?;
    EncounterMode::from_u8(raw).ok_or(CodecError::UnknownMessage)
}

// ---------------------------------------------------------------------------
// Intent codec.
// ---------------------------------------------------------------------------

/// Encodes one intent frame to its canonical v3 bytes (first byte 3).
///
/// Checks lane (`WrongDirection`), then `seq` (`Malformed`), then writes the
/// layout, then enforces [`MAX_INTENT_FRAME_BYTES`] (`FrameTooLarge`).
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
        IntentBody::EndTurn => out = extend(out, &INTENT_TAG_END_TURN)?,
        IntentBody::Stop => out = extend(out, &INTENT_TAG_STOP)?,
        IntentBody::Pause => out = extend(out, &INTENT_TAG_PAUSE)?,
        IntentBody::Unpause => out = extend(out, &INTENT_TAG_UNPAUSE)?,
        IntentBody::RequestMode { mode } => {
            out = extend(out, &INTENT_TAG_REQUEST_MODE)?;
            out = extend(out, &mode.as_u8())?;
        }
    }
    if out.len() > MAX_INTENT_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    Ok(out)
}

/// Decodes one intent frame from its v3 bytes.
///
/// Order: frame cap, version, epoch, lane, `seq` range, `observed_tick`,
/// nonzero `actor`, body tag, body fields, trailing bytes.
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
        INTENT_TAG_STOP => IntentBody::Stop,
        INTENT_TAG_PAUSE => IntentBody::Pause,
        INTENT_TAG_UNPAUSE => IntentBody::Unpause,
        INTENT_TAG_REQUEST_MODE => IntentBody::RequestMode {
            mode: take_mode(&mut rest)?,
        },
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

/// Encodes the receipt status tag and, when rejected, its v3 code byte.
fn extend_status(mut out: Vec<u8>, status: &ReceiptStatus) -> Result<Vec<u8>, CodecError> {
    match status {
        ReceiptStatus::Applied => extend(out, &RECEIPT_TAG_APPLIED),
        ReceiptStatus::Rejected(code) => {
            out = extend(out, &RECEIPT_TAG_REJECTED)?;
            extend(out, &code.as_u8())
        }
    }
}

/// Decodes a receipt status tag and optional v3 code byte.
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

/// Encodes the `Clock` pause state after its tag.
fn extend_clock(
    mut out: Vec<u8>,
    paused: Option<Option<PauseReason>>,
) -> Result<Vec<u8>, CodecError> {
    match paused {
        None => extend(out, &false),
        Some(None) => {
            out = extend(out, &true)?;
            extend(out, &false)
        }
        Some(Some(reason)) => {
            out = extend(out, &true)?;
            out = extend(out, &true)?;
            out = extend(out, &reason.tag())?;
            if let PauseReason::Auto(trigger) = reason {
                out = extend(out, &trigger.as_u8())?;
            }
            Ok(out)
        }
    }
}

/// Decodes the `Clock` pause state after its tag.
fn take_clock(rest: &mut &[u8]) -> Result<Option<Option<PauseReason>>, CodecError> {
    let paused: bool = take(rest)?;
    if !paused {
        return Ok(None);
    }
    let disclosed: bool = take(rest)?;
    if !disclosed {
        return Ok(Some(None));
    }
    let tag: u8 = take(rest)?;
    let reason = match tag {
        0 => PauseReason::Player,
        1 => {
            let raw: u8 = take(rest)?;
            PauseReason::Auto(AutoPauseTrigger::from_u8(raw).ok_or(CodecError::UnknownMessage)?)
        }
        2 => PauseReason::Operator,
        3 => PauseReason::Restore,
        _ => return Err(CodecError::UnknownMessage),
    };
    Ok(Some(Some(reason)))
}

/// Encodes the `HostFacts` fields after the tag.
fn extend_host_facts(mut out: Vec<u8>, facts: &HostFacts) -> Result<Vec<u8>, CodecError> {
    out = extend(out, &facts.rights.pause)?;
    out = extend(out, &facts.rights.unpause)?;
    match facts.auto_pause {
        None => out = extend(out, &false)?,
        Some(toggles) => {
            out = extend(out, &true)?;
            out = extend(out, &toggles.bits())?;
        }
    }
    match facts.turn_timer_ms {
        None => out = extend(out, &false)?,
        Some(None) => {
            out = extend(out, &true)?;
            out = extend(out, &false)?;
        }
        Some(Some(ms)) => {
            out = extend(out, &true)?;
            out = extend(out, &true)?;
            out = extend(out, &ms)?;
        }
    }
    Ok(out)
}

/// Decodes the `HostFacts` fields after the tag.
fn take_host_facts(rest: &mut &[u8]) -> Result<HostFacts, CodecError> {
    let pause: bool = take(rest)?;
    let unpause: bool = take(rest)?;
    let auto_disclosed: bool = take(rest)?;
    let auto_pause = if auto_disclosed {
        let bits: u8 = take(rest)?;
        Some(AutoPauseToggles::from_bits(bits).ok_or(CodecError::Malformed)?)
    } else {
        None
    };
    let timer_disclosed: bool = take(rest)?;
    let turn_timer_ms = if timer_disclosed {
        let running: bool = take(rest)?;
        if running {
            Some(Some(take::<u32>(rest)?))
        } else {
            Some(None)
        }
    } else {
        None
    };
    Ok(HostFacts {
        rights: PauseRights { pause, unpause },
        auto_pause,
        turn_timer_ms,
    })
}

/// Encodes one v3 delta operation, including its tag byte.
fn extend_op(mut out: Vec<u8>, op: &DeltaOp) -> Result<Vec<u8>, CodecError> {
    match op {
        DeltaOp::EntityEnter { entity } => {
            out = extend(out, &DELTA_TAG_ENTITY_ENTER)?;
            extend(out, &entity.get())
        }
        DeltaOp::EntityLeave { entity } => {
            out = extend(out, &DELTA_TAG_ENTITY_LEAVE)?;
            extend(out, &entity.get())
        }
        DeltaOp::Spawned { entity } => {
            out = extend(out, &DELTA_TAG_SPAWNED)?;
            extend(out, &entity.get())
        }
        DeltaOp::Despawned { entity } => {
            out = extend(out, &DELTA_TAG_DESPAWNED)?;
            extend(out, &entity.get())
        }
        DeltaOp::Died { entity } => {
            out = extend(out, &DELTA_TAG_DIED)?;
            extend(out, &entity.get())
        }
        DeltaOp::Health {
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
        DeltaOp::Turn { active, round } => {
            out = extend(out, &DELTA_TAG_TURN)?;
            match active {
                None => out = extend(out, &false)?,
                Some(entity) => {
                    out = extend(out, &true)?;
                    out = extend(out, &entity.get())?;
                }
            }
            extend(out, round)
        }
        DeltaOp::Receipt {
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
        DeltaOp::RoundStarted { round } => {
            out = extend(out, &DELTA_TAG_ROUND_STARTED)?;
            extend(out, round)
        }
        DeltaOp::ModeChanged { mode, round } => {
            out = extend(out, &DELTA_TAG_MODE_CHANGED)?;
            out = extend(out, &mode.as_u8())?;
            extend(out, round)
        }
        DeltaOp::OrderEnded { actor } => {
            out = extend(out, &DELTA_TAG_ORDER_ENDED)?;
            extend(out, &actor.get())
        }
        DeltaOp::Joined { entity } => {
            out = extend(out, &DELTA_TAG_JOINED)?;
            extend(out, &entity.get())
        }
        DeltaOp::Left { entity } => {
            out = extend(out, &DELTA_TAG_LEFT)?;
            extend(out, &entity.get())
        }
        DeltaOp::Clock { paused } => {
            out = extend(out, &DELTA_TAG_CLOCK)?;
            extend_clock(out, *paused)
        }
        DeltaOp::HostFacts(facts) => {
            out = extend(out, &DELTA_TAG_HOST_FACTS)?;
            extend_host_facts(out, facts)
        }
    }
}

/// Decodes one v3 delta operation, tag byte first.
///
/// Any tag outside the implemented set (including the reserved 11..=13) is
/// [`CodecError::UnknownMessage`].
fn take_op(rest: &mut &[u8]) -> Result<DeltaOp, CodecError> {
    let tag: u8 = take(rest)?;
    match tag {
        DELTA_TAG_ENTITY_ENTER => Ok(DeltaOp::EntityEnter {
            entity: take_net_id(rest)?,
        }),
        DELTA_TAG_ENTITY_LEAVE => Ok(DeltaOp::EntityLeave {
            entity: take_net_id(rest)?,
        }),
        DELTA_TAG_SPAWNED => Ok(DeltaOp::Spawned {
            entity: take_net_id(rest)?,
        }),
        DELTA_TAG_DESPAWNED => Ok(DeltaOp::Despawned {
            entity: take_net_id(rest)?,
        }),
        DELTA_TAG_DIED => Ok(DeltaOp::Died {
            entity: take_net_id(rest)?,
        }),
        DELTA_TAG_HEALTH => Ok(DeltaOp::Health {
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
            Ok(DeltaOp::Turn {
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
            Ok(DeltaOp::Receipt {
                epoch,
                lane,
                seq,
                processed_tick,
                status: take_status(rest)?,
            })
        }
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
        DELTA_TAG_ROUND_STARTED => Ok(DeltaOp::RoundStarted { round: take(rest)? }),
        DELTA_TAG_MODE_CHANGED => {
            let mode = take_mode(rest)?;
            let round: u64 = take(rest)?;
            Ok(DeltaOp::ModeChanged { mode, round })
        }
        DELTA_TAG_ORDER_ENDED => Ok(DeltaOp::OrderEnded {
            actor: take_net_id(rest)?,
        }),
        DELTA_TAG_JOINED => Ok(DeltaOp::Joined {
            entity: take_net_id(rest)?,
        }),
        DELTA_TAG_LEFT => Ok(DeltaOp::Left {
            entity: take_net_id(rest)?,
        }),
        DELTA_TAG_CLOCK => Ok(DeltaOp::Clock {
            paused: take_clock(rest)?,
        }),
        DELTA_TAG_HOST_FACTS => Ok(DeltaOp::HostFacts(take_host_facts(rest)?)),
        _ => Err(CodecError::UnknownMessage),
    }
}

/// Encodes one v3 delta frame to its canonical bytes.
///
/// Checks lane, op count, then each op in order (receipt lane and `seq`,
/// ULID text bound, outcome bound then vocabulary), then
/// [`MAX_DELTA_FRAME_BYTES`]. No partial bytes are returned on failure.
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

/// Decodes one v3 delta frame from its bytes.
///
/// Order: frame cap, version, lane, `server_tick`, `event_seq`, op-count
/// bound before any op is read (the vector grows by push), per-op tag then
/// fields, trailing bytes.
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
