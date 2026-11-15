//! Bounded `postcard` codec for the explicit v4 lane-0 wire (T026p1,
//! ADR-0036 L1–L6).
//!
//! v1, v2 and v3 are frozen and untouched. Calling [`decode_intent`] or
//! [`decode_delta`] here is explicit version selection: any other first byte
//! (1, 2, 3, `0x81`, `0x82`, `0x91`, `0xA1`) is
//! [`CodecError::UnsupportedVersion`], and the v1, v2 and v3 codecs refuse
//! byte 4 the same way.
//!
//! Intent frame: `version=4, epoch[16], lane, seq, observed_tick, actor, body
//! tag`, then the body: tags 0..=5 exactly v3's, tag 6 `DeclareMove` one
//! point. Delta frame: `version=4, lane, server_tick, event_seq, op_count`,
//! then each op: tags 0..=20 are v3's bytes and are decoded only by
//! [`crate::codec_v3`]'s `take_op`; tag 21 `Moved` is `entity`, a count
//! varint (2..=64) and that many points; tag 22 `Position` is `entity`, a
//! disclosed flag and, when set, a point and a `moving` flag.
//!
//! A point is `x`, `y`, `z`, each a zigzag `i32` checked against the U3
//! bound as soon as it is read.
//!
//! Decode order is contract: whole-frame cap, empty input, version, header
//! fields, op-count cap before any op is read (the vector grows by push),
//! each tag then its fields (each checked when read), trailing-byte reject.
//! Encode checks lane, count, then each op in order, then the frame cap, and
//! never returns partial bytes. The codec checks shape, not entitlement, and
//! never touches `World`.

use core::ops::Range;

use crate::protocol::{
    LANE_COMBAT, MAX_DELTA_FRAME_BYTES, MAX_DELTA_OPS, MAX_INTENT_FRAME_BYTES,
    MAX_WIRE_STRING_BYTES, SEQ_FIRST, SEQ_LAST,
};
use crate::protocol_v3::{
    EncounterMode, INTENT_TAG_DECLARE_ACTION, INTENT_TAG_END_TURN, INTENT_TAG_PAUSE,
    INTENT_TAG_REQUEST_MODE, INTENT_TAG_STOP, INTENT_TAG_UNPAUSE, MAX_FRAME_OP_BYTES_V3,
};
use crate::protocol_v4::{
    check_points, is_zero_segment, DeltaFrame, DeltaOp, IntentBody, IntentFrame, NetId, Point,
    PositionFact, DELTA_TAG_MOVED, DELTA_TAG_POSITION, INTENT_TAG_DECLARE_MOVE, MAX_MOVED_POINTS,
    PROTOCOL_VERSION,
};
use crpg_core::Ulid;

/// Shape failure of the explicit v4 wire: the same six variants as v1.
pub use crate::codec::CodecError;

// ---------------------------------------------------------------------------
// postcard plumbing (v3's private helpers duplicated, never shared).
// ---------------------------------------------------------------------------

/// Extends `out` with one `postcard`-encoded primitive.
fn extend<T: serde::Serialize + ?Sized>(out: Vec<u8>, value: &T) -> Result<Vec<u8>, CodecError> {
    postcard::to_extend(value, out).map_err(|_| CodecError::Malformed)
}

/// Decodes one `postcard` primitive from the head of `rest`, advancing it.
///
/// Every `postcard` decode failure (truncation, bad varint/bool) is a
/// [`CodecError::Malformed`].
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
/// A truncated or over-long varint is [`CodecError::Malformed`]; the tenth
/// byte may only carry bit 63.
fn take_varint_len(rest: &mut &[u8]) -> Result<u64, CodecError> {
    let mut value: u64 = 0;
    for shift in (0..64).step_by(7) {
        if rest.is_empty() {
            return Err(CodecError::Malformed);
        }
        let byte = rest[0];
        *rest = &rest[1..];
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

/// Validates an encode-side ULID text length.
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
// Points.
// ---------------------------------------------------------------------------

/// Encodes one point: `x`, `y`, `z` as zigzag `i32`s.
pub(crate) fn extend_point(mut out: Vec<u8>, point: Point) -> Result<Vec<u8>, CodecError> {
    for component in point.raw() {
        out = extend(out, &component)?;
    }
    Ok(out)
}

/// Decodes one point; each coordinate is checked against U3 when read, before
/// the next one is read.
pub(crate) fn take_point(rest: &mut &[u8]) -> Result<Point, CodecError> {
    let mut raw = [0i32; 3];
    for slot in &mut raw {
        let component: i32 = take(rest)?;
        // A one-component point is the U3 check for this component alone.
        Point::new([component, 0, 0]).ok_or(CodecError::Malformed)?;
        *slot = component;
    }
    Point::new(raw).ok_or(CodecError::Malformed)
}

// ---------------------------------------------------------------------------
// Intent codec.
// ---------------------------------------------------------------------------

/// Encodes one intent frame to its canonical v4 bytes (first byte 4).
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
        IntentBody::DeclareMove { destination } => {
            out = extend(out, &INTENT_TAG_DECLARE_MOVE)?;
            out = extend_point(out, destination)?;
        }
    }
    if out.len() > MAX_INTENT_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    Ok(out)
}

/// Decodes one intent frame from its v4 bytes.
///
/// Order: frame cap, empty input, version, epoch, lane, `seq` range,
/// `observed_tick`, nonzero `actor`, body tag, body fields, trailing bytes.
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
        INTENT_TAG_DECLARE_MOVE => IntentBody::DeclareMove {
            destination: take_point(&mut rest)?,
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

/// Encodes one v4 delta operation, including its tag byte. Tags 0..=20 are
/// written by `codec_v3::extend_op`.
fn extend_op(mut out: Vec<u8>, op: &DeltaOp) -> Result<Vec<u8>, CodecError> {
    match op {
        DeltaOp::V3(inner) => crate::codec_v3::extend_op(out, inner),
        DeltaOp::Moved { entity, points } => {
            check_points(points)?;
            out = extend(out, &DELTA_TAG_MOVED)?;
            out = extend(out, &entity.get())?;
            out = extend(out, &(points.len() as u64))?;
            for point in points {
                out = extend_point(out, *point)?;
            }
            Ok(out)
        }
        DeltaOp::Position { entity, position } => {
            out = extend(out, &DELTA_TAG_POSITION)?;
            out = extend(out, &entity.get())?;
            match position {
                None => extend(out, &false),
                Some(fact) => {
                    out = extend(out, &true)?;
                    out = extend_point(out, fact.point)?;
                    extend(out, &fact.moving)
                }
            }
        }
    }
}

/// Decodes the fields of a `Moved` after its tag: entity, count bound before
/// any point, then each point with the zero-segment check against the
/// previous one.
fn take_moved(rest: &mut &[u8]) -> Result<DeltaOp, CodecError> {
    let entity = take_net_id(rest)?;
    let count = take_varint_len(rest)?;
    if count > MAX_MOVED_POINTS as u64 {
        return Err(CodecError::LimitExceeded);
    }
    if count < 2 {
        return Err(CodecError::Malformed);
    }
    let mut points: Vec<Point> = Vec::new();
    for _ in 0..count {
        let point = take_point(rest)?;
        if let Some(previous) = points.last() {
            if is_zero_segment(*previous, point) {
                return Err(CodecError::Malformed);
            }
        }
        points.push(point);
    }
    Ok(DeltaOp::Moved { entity, points })
}

/// Decodes the fields of a `Position` after its tag.
fn take_position(rest: &mut &[u8]) -> Result<DeltaOp, CodecError> {
    let entity = take_net_id(rest)?;
    let disclosed: bool = take(rest)?;
    let position = if disclosed {
        let point = take_point(rest)?;
        let moving: bool = take(rest)?;
        Some(PositionFact { point, moving })
    } else {
        None
    };
    Ok(DeltaOp::Position { entity, position })
}

/// Decodes one v4 delta operation, tag byte first. Tags 0..=20 are read only
/// by `codec_v3::take_op`; 23..=255 are [`CodecError::UnknownMessage`].
fn take_op(rest: &mut &[u8]) -> Result<DeltaOp, CodecError> {
    let Some(&tag) = rest.first() else {
        return Err(CodecError::Malformed);
    };
    match tag {
        0..=20 => crate::codec_v3::take_op(rest).map(DeltaOp::V3),
        DELTA_TAG_MOVED => {
            *rest = &rest[1..];
            take_moved(rest)
        }
        DELTA_TAG_POSITION => {
            *rest = &rest[1..];
            take_position(rest)
        }
        _ => Err(CodecError::UnknownMessage),
    }
}

/// Greedy split of `ops`, in order, into frame-sized index ranges.
///
/// First runs the per-op encode checks of [`encode_delta`] over every op in
/// order; the first failure is returned and no spans are. Otherwise the
/// returned ranges are contiguous, cover `0..ops.len()` in order, and a frame
/// closes exactly when adding the next op would make it hold more than
/// [`MAX_DELTA_OPS`] ops or more than [`MAX_FRAME_OP_BYTES_V3`] bytes of
/// encoded ops (tags included). Empty input gives no spans. The v4 header is
/// at most 24 bytes, so a lane-0 frame holding one span encodes within
/// [`MAX_DELTA_FRAME_BYTES`] for every `server_tick` and `event_seq`.
pub fn frame_spans(ops: &[DeltaOp]) -> Result<Vec<Range<usize>>, CodecError> {
    let mut sizes = Vec::with_capacity(ops.len());
    for op in ops {
        sizes.push(extend_op(Vec::new(), op)?.len());
    }
    let mut spans = Vec::new();
    let mut start = 0usize;
    let mut bytes = 0usize;
    for (index, size) in sizes.iter().enumerate() {
        let held = index - start;
        if held > 0 && (held + 1 > MAX_DELTA_OPS || bytes + size > MAX_FRAME_OP_BYTES_V3) {
            spans.push(start..index);
            start = index;
            bytes = 0;
        }
        bytes += size;
    }
    if start < ops.len() {
        spans.push(start..ops.len());
    }
    Ok(spans)
}

/// Encodes one v4 delta frame to its canonical bytes.
///
/// Checks lane, op count, then each op in order, then
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

/// Decodes one v4 delta frame from its bytes.
///
/// Order: frame cap, empty input, version, lane, `server_tick`, `event_seq`,
/// op-count bound before any op is read (the vector grows by push), per-op
/// tag then fields, trailing bytes.
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
