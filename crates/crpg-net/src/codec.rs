//! Bounded `postcard` codec for the lane-0 v1 wire (T018a).
//!
//! Encoding is canonical: fields are written in the frozen order documented
//! below with `postcard` primitive encodings (single-byte `u8`/`bool`,
//! varint integers, length-prefixed strings, raw fixed arrays). Decoding is
//! staged so the first failure wins with its exact [`CodecError`]:
//!
//! 1. Length cap against the frame maximum, before any allocation.
//! 2. Version byte.
//! 3. Lane/direction bytes (intent lane, delta lane, receipt lane).
//! 4. Discriminant tags (intent body, delta op, receipt status, code).
//! 5. Length and collection bounds (ability string bytes, op count).
//! 6. Complete decode of the remaining fields.
//! 7. Empty-remainder check: trailing bytes are [`CodecError::Malformed`].
//! 8. Semantic value checks: nonzero [`NetId`](crate::protocol::NetId)s,
//!    `seq` within `SEQ_FIRST..=SEQ_LAST`, valid ULID text.
//!
//! The codec never touches `World`, RNG, or the event queue, never reserves
//! from untrusted lengths (the op vector grows by push; string bodies are
//! bounded by the already-checked frame cap), and never serializes the
//! evolving sim enum: every tag is matched explicitly here.
//!
//! Frozen v1 field order, intent frame (after the version byte):
//! `epoch [u8; 16]`, `lane u8`, `seq u64`, `observed_tick u64`, `actor u64`,
//! body tag `u8`, then per [`crate::protocol::IntentBody`]: `DeclareAction`
//! carries `ability` (length-prefixed string) and `target u64`; `EndTurn`
//! carries nothing. Frozen v1 field order, delta frame (after the version
//! byte): `lane u8`, `server_tick u64`, `event_seq u64`, op count `u64`, then
//! per [`crate::protocol::DeltaOp`]: entity ops carry `entity u64`; `Health`
//! carries `entity u64`, `health u32`, `max_health u32`, `dead bool`; `Turn`
//! carries an `active`-present `bool`, the `active u64` when present, and
//! `round u64`; `Receipt` carries `epoch [u8; 16]`, `lane u8`, `seq u64`,
//! `processed_tick u64`, status tag `u8`, and the code `u8` when rejected.

use core::fmt;

use crate::protocol::{
    DeltaFrame, DeltaOp, IntentBody, IntentFrame, NetId, ReceiptStatus, RejectionCode,
    DELTA_TAG_DESPAWNED, DELTA_TAG_DIED, DELTA_TAG_ENTITY_ENTER, DELTA_TAG_ENTITY_LEAVE,
    DELTA_TAG_HEALTH, DELTA_TAG_RECEIPT, DELTA_TAG_SPAWNED, DELTA_TAG_TURN,
    INTENT_TAG_DECLARE_ACTION, INTENT_TAG_END_TURN, LANE_COMBAT, MAX_DELTA_FRAME_BYTES,
    MAX_DELTA_OPS, MAX_INTENT_FRAME_BYTES, MAX_WIRE_STRING_BYTES, PROTOCOL_VERSION,
    RECEIPT_TAG_APPLIED, RECEIPT_TAG_REJECTED, SEQ_FIRST, SEQ_LAST,
};

/// Shape failure of the lane-0 v1 wire.
///
/// Host/transport admission and gameplay outcomes use the frozen
/// [`RejectionCode`] inside receipts instead; this enum covers exactly the
/// codec boundary, with first-failure-wins precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecError {
    /// Input (decode) or produced bytes (encode) exceed the frame cap.
    FrameTooLarge,
    /// Version byte differs from [`PROTOCOL_VERSION`].
    UnsupportedVersion,
    /// A lane byte differs from [`LANE_COMBAT`].
    WrongDirection,
    /// A discriminant tag or rejection-code byte is outside the v1 set.
    UnknownMessage,
    /// Truncated or ill-formed data, trailing bytes, zero [`NetId`],
    /// out-of-range `seq`, or unparsable ULID text.
    Malformed,
    /// A string, collection, or op-count bound was exceeded.
    LimitExceeded,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameTooLarge => write!(f, "frame exceeds its byte cap"),
            Self::UnsupportedVersion => write!(f, "unsupported protocol version"),
            Self::WrongDirection => write!(f, "wrong lane or direction"),
            Self::UnknownMessage => write!(f, "unknown message discriminant"),
            Self::Malformed => write!(f, "malformed frame"),
            Self::LimitExceeded => write!(f, "wire bound exceeded"),
        }
    }
}

impl std::error::Error for CodecError {}

// ---------------------------------------------------------------------------
// postcard plumbing.
// ---------------------------------------------------------------------------

/// Extends `out` with one `postcard`-encoded primitive.
///
/// Primitive extension into a `Vec` is infallible; a failure here would be an
/// internal encoding bug, reported as [`CodecError::Malformed`] so encode
/// stays total without panicking.
fn extend<T: serde::Serialize + ?Sized>(out: Vec<u8>, value: &T) -> Result<Vec<u8>, CodecError> {
    postcard::to_extend(value, out).map_err(|_| CodecError::Malformed)
}

/// Decodes one `postcard` primitive from the head of `rest`, advancing it.
///
/// Every `postcard` decode failure (truncation, bad varint/bool/UTF-8) is a
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

/// Decodes a lane-0 command sequence number.
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

/// Decodes the ability ULID from its length-prefixed canonical text.
///
/// The byte-length bound fires before parsing, so a hostile length prefix
/// inside the frame cap reports [`CodecError::LimitExceeded`], while text
/// that fits but is not a ULID reports [`CodecError::Malformed`].
fn take_ability(rest: &mut &[u8]) -> Result<crpg_core::Ulid, CodecError> {
    let text: String = take(rest)?;
    if text.len() > MAX_WIRE_STRING_BYTES {
        return Err(CodecError::LimitExceeded);
    }
    text.parse::<crpg_core::Ulid>()
        .map_err(|_| CodecError::Malformed)
}

// ---------------------------------------------------------------------------
// Intent codec.
// ---------------------------------------------------------------------------

/// Encodes one intent frame to its canonical v1 bytes.
///
/// Enforces the encode-side bounds (lane, `seq` range, op-free shape) and
/// rejects the result with [`CodecError::FrameTooLarge`] if it exceeds
/// [`MAX_INTENT_FRAME_BYTES`].
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
            if text.len() > MAX_WIRE_STRING_BYTES {
                return Err(CodecError::LimitExceeded);
            }
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

/// Decodes one intent frame from its v1 bytes.
///
/// Applies the staged checks in order: frame cap, version, lane, body tag,
/// string bound, complete decode with no trailing bytes, then the nonzero-id
/// and `seq`-range value checks.
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
            let ability = take_ability(&mut rest)?;
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

/// Encodes one delta operation, including its frozen tag byte.
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
    }
}

/// Decodes one delta operation after its frozen tag byte.
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
        _ => Err(CodecError::UnknownMessage),
    }
}

/// Encodes one delta frame to its canonical v1 bytes.
///
/// Enforces the encode-side bounds (lane, op count, receipt lane/`seq`) and
/// rejects the result with [`CodecError::FrameTooLarge`] if it exceeds
/// [`MAX_DELTA_FRAME_BYTES`].
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

/// Decodes one delta frame from its v1 bytes.
///
/// Applies the staged checks in order: frame cap, version, lane, op-count
/// bound (before any per-op decode or reservation; the vector grows by push),
/// per-op tags and value checks, then the empty-remainder check.
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
