#![allow(missing_docs)]
//! Bounded codec for the transfer subprotocol (T024).

use crate::protocol::{MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_CHUNKS, MAX_VISIBLE_ENTITIES, SEQ_LAST};
use crate::protocol_transfer::MAX_SEGMENT_BYTES;
use crate::protocol_transfer::*;

/// Re-export.
pub use crate::codec::CodecError;

// plumbing
fn extend<T: serde::Serialize + ?Sized>(out: Vec<u8>, value: &T) -> Result<Vec<u8>, CodecError> {
    postcard::to_extend(value, out).map_err(|_| CodecError::Malformed)
}

fn take<'a, T: serde::Deserialize<'a>>(rest: &mut &'a [u8]) -> Result<T, CodecError> {
    let (value, remaining) = postcard::take_from_bytes::<T>(rest).map_err(|_| CodecError::Malformed)?;
    *rest = remaining;
    Ok(value)
}

fn take_transfer_id(rest: &mut &[u8]) -> Result<TransferId, CodecError> {
    let raw: u64 = take(rest)?;
    TransferId::new(raw).ok_or(CodecError::Malformed)
}

// ---------------------------------------------------------------------------
// Body codec.
// ---------------------------------------------------------------------------

pub fn encode_body(body: &SnapshotBody) -> Result<Vec<u8>, CodecError> {
    if body.entities.len() > MAX_VISIBLE_ENTITIES {
        return Err(CodecError::LimitExceeded);
    }
    // ascending check
    let mut last: Option<u64> = None;
    for e in &body.entities {
        let raw = e.entity.get();
        if raw == 0 {
            return Err(CodecError::Malformed);
        }
        if let Some(l) = last {
            if raw <= l {
                return Err(CodecError::Malformed);
            }
        }
        last = Some(raw);
    }
    if let Some(t) = &body.turn {
        if let Some(a) = t.active {
            let raw = a.get();
            if !body.entities.iter().any(|e| e.entity.get() == raw) {
                return Err(CodecError::Malformed);
            }
        }
    }
    let mut out = extend(Vec::new(), &(body.entities.len() as u64))?;
    for e in &body.entities {
        out = extend(out, &e.entity.get())?;
        match &e.health {
            None => {
                out = extend(out, &false)?;
            }
            Some(h) => {
                out = extend(out, &true)?;
                out = extend(out, &h.health)?;
                out = extend(out, &h.max_health)?;
                out = extend(out, &h.dead)?;
            }
        }
    }
    match &body.turn {
        None => {
            out = extend(out, &false)?;
        }
        Some(t) => {
            out = extend(out, &true)?;
            match &t.active {
                None => {
                    out = extend(out, &false)?;
                }
                Some(a) => {
                    out = extend(out, &true)?;
                    out = extend(out, &a.get())?;
                }
            }
            out = extend(out, &t.round)?;
        }
    }
    if out.len() > MAX_SNAPSHOT_BYTES {
        return Err(CodecError::LimitExceeded);
    }
    Ok(out)
}

pub fn decode_body(bytes: &[u8]) -> Result<SnapshotBody, CodecError> {
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(CodecError::LimitExceeded);
    }
    let mut rest = bytes;
    let count: u64 = take(&mut rest)?;
    if count > MAX_VISIBLE_ENTITIES as u64 {
        return Err(CodecError::LimitExceeded);
    }
    let mut entities = Vec::new();
    let mut last: Option<u64> = None;
    for _ in 0..count {
        let raw: u64 = take(&mut rest)?;
        let entity = NetId::new(raw).ok_or(CodecError::Malformed)?;
        if let Some(l) = last {
            if raw <= l {
                return Err(CodecError::Malformed);
            }
        }
        last = Some(raw);
        let present: bool = take(&mut rest)?;
        let health = if present {
            let health: u32 = take(&mut rest)?;
            let max_health: u32 = take(&mut rest)?;
            let dead: bool = take(&mut rest)?;
            Some(HealthFact { health, max_health, dead })
        } else {
            None
        };
        entities.push(SnapshotEntity { entity, health });
    }
    let turn_present: bool = take(&mut rest)?;
    let turn = if turn_present {
        let active_present: bool = take(&mut rest)?;
        let active = if active_present {
            let raw: u64 = take(&mut rest)?;
            let id = NetId::new(raw).ok_or(CodecError::Malformed)?;
            if !entities.iter().any(|e| e.entity.get() == raw) {
                return Err(CodecError::Malformed);
            }
            Some(id)
        } else {
            None
        };
        let round: u64 = take(&mut rest)?;
        Some(TurnFact { active, round })
    } else {
        None
    };
    if !rest.is_empty() {
        return Err(CodecError::Malformed);
    }
    Ok(SnapshotBody { entities, turn })
}

// ---------------------------------------------------------------------------
// Frame codecs.
// ---------------------------------------------------------------------------

pub fn encode_down(msg: &TransferDown) -> Result<Vec<u8>, CodecError> {
    match msg {
        TransferDown::Begin(b) => {
            b.validate()?;
            let mut out = extend(Vec::new(), &TRANSFER_VERSION)?;
            out = extend(out, &LANE_SNAPSHOT)?;
            out = extend(out, &TRANSFER_TAG_BEGIN)?;
            out = extend(out, &b.epoch)?;
            out = extend(out, &b.transfer.get())?;
            out = extend(out, &b.base_tick)?;
            out = extend(out, &b.cursor)?;
            out = extend(out, &(b.total_bytes as u64))?;
            out = extend(out, &(b.entity_count as u64))?;
            out = extend(out, &b.segments_per_chunk)?;
            out = extend(out, &b.chunk_count)?;
            if out.len() > MAX_DOWN_FRAME_BYTES {
                return Err(CodecError::FrameTooLarge);
            }
            Ok(out)
        }
        TransferDown::Chunk(c) => {
            if c.index >= MAX_SNAPSHOT_CHUNKS as u8 {
                return Err(CodecError::LimitExceeded);
            }
            if c.bytes.is_empty() {
                return Err(CodecError::Malformed);
            }
            if c.bytes.len() > MAX_CHUNK_BYTES {
                return Err(CodecError::LimitExceeded);
            }
            // segment
            let mut segs: Vec<&[u8]> = Vec::new();
            let mut off = 0;
            while off < c.bytes.len() {
                let end = (off + MAX_SEGMENT_BYTES).min(c.bytes.len());
                segs.push(&c.bytes[off..end]);
                off = end;
            }
            if segs.len() > MAX_SEGMENTS_PER_CHUNK as usize {
                return Err(CodecError::LimitExceeded);
            }
            let mut out = extend(Vec::new(), &TRANSFER_VERSION)?;
            out = extend(out, &LANE_SNAPSHOT)?;
            out = extend(out, &TRANSFER_TAG_CHUNK)?;
            out = extend(out, &c.epoch)?;
            out = extend(out, &c.transfer.get())?;
            out = extend(out, &c.index)?;
            out = extend(out, &(segs.len() as u8))?;
            for s in segs {
                out = extend(out, &(s.len() as u64))?;
                // write raw bytes: postcard serializes [u8] as length-prefixed? Need raw bytes.
                // Use extend with slice? postcard to_extend for &[u8] writes length prefix. We need raw bytes no length prefix? 
                // S§5 says len then len bytes. The len is separate varint we already wrote. So body bytes raw.
                out.extend_from_slice(s);
            }
            if out.len() > MAX_DOWN_FRAME_BYTES {
                return Err(CodecError::FrameTooLarge);
            }
            Ok(out)
        }
        TransferDown::Cancel { epoch, transfer, reason } => {
            if transfer.get() == 0 {
                return Err(CodecError::Malformed);
            }
            // reason validity
            if CancelReason::from_u8(reason.as_u8()).is_none() {
                return Err(CodecError::UnknownMessage);
            }
            let mut out = extend(Vec::new(), &TRANSFER_VERSION)?;
            out = extend(out, &LANE_SNAPSHOT)?;
            out = extend(out, &TRANSFER_TAG_CANCEL)?;
            out = extend(out, epoch)?;
            out = extend(out, &transfer.get())?;
            out = extend(out, &reason.as_u8())?;
            if out.len() > MAX_DOWN_FRAME_BYTES {
                return Err(CodecError::FrameTooLarge);
            }
            Ok(out)
        }
    }
}

pub fn decode_down(bytes: &[u8]) -> Result<TransferDown, CodecError> {
    use crate::protocol_transfer::*;
    if bytes.len() > MAX_DOWN_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    if bytes.is_empty() {
        return Err(CodecError::Malformed);
    }
    let mut rest = bytes;
    let version: u8 = take(&mut rest)?;
    if version != TRANSFER_VERSION {
        return Err(CodecError::UnsupportedVersion);
    }
    let lane: u8 = take(&mut rest)?;
    if lane != LANE_SNAPSHOT {
        return Err(CodecError::WrongDirection);
    }
    let tag: u8 = take(&mut rest)?;
    match tag {
        TRANSFER_TAG_BEGIN => {
            let epoch: [u8; 16] = take(&mut rest)?;
            let transfer = take_transfer_id(&mut rest)?;
            let base_tick: u64 = take(&mut rest)?;
            let cursor: u64 = take(&mut rest)?;
            if cursor > SEQ_LAST {
                return Err(CodecError::Malformed);
            }
            let total_bytes_raw: u64 = take(&mut rest)?;
            let total_bytes = if total_bytes_raw > MAX_SNAPSHOT_BYTES as u64 {
                return Err(CodecError::LimitExceeded);
            } else if total_bytes_raw == 0 {
                return Err(CodecError::Malformed);
            } else {
                total_bytes_raw as u32
            };
            let entity_count_raw: u64 = take(&mut rest)?;
            if entity_count_raw > MAX_VISIBLE_ENTITIES as u64 {
                return Err(CodecError::LimitExceeded);
            }
            let entity_count = entity_count_raw as u16;
            let segments_per_chunk: u8 = take(&mut rest)?;
            if segments_per_chunk == 0 {
                return Err(CodecError::Malformed);
            }
            if segments_per_chunk > MAX_SEGMENTS_PER_CHUNK {
                return Err(CodecError::LimitExceeded);
            }
            let chunk_count: u8 = take(&mut rest)?;
            if chunk_count == 0 {
                return Err(CodecError::Malformed);
            }
            if chunk_count as usize > MAX_SNAPSHOT_CHUNKS {
                return Err(CodecError::LimitExceeded);
            }
            // consistency
            let chunk_size = segments_per_chunk as usize * MAX_SEGMENT_BYTES;
            let expected = (total_bytes as usize).div_ceil(chunk_size);
            if expected != chunk_count as usize {
                return Err(CodecError::Malformed);
            }
            if !rest.is_empty() {
                return Err(CodecError::Malformed);
            }
            let begin = BeginTransfer {
                epoch,
                transfer,
                base_tick,
                cursor,
                total_bytes,
                entity_count,
                segments_per_chunk,
                chunk_count,
            };
            // validate
            begin.validate()?;
            Ok(TransferDown::Begin(begin))
        }
        TRANSFER_TAG_CHUNK => {
            let epoch: [u8; 16] = take(&mut rest)?;
            let transfer = take_transfer_id(&mut rest)?;
            let index: u8 = take(&mut rest)?;
            if index as usize >= MAX_SNAPSHOT_CHUNKS {
                return Err(CodecError::LimitExceeded);
            }
            let segment_count: u8 = take(&mut rest)?;
            if segment_count == 0 {
                return Err(CodecError::Malformed);
            }
            if segment_count > MAX_SEGMENTS_PER_CHUNK {
                return Err(CodecError::LimitExceeded);
            }
            let mut bytes = Vec::new();
            for si in 0..segment_count {
                let len_raw: u64 = take(&mut rest)?;
                if len_raw > MAX_SEGMENT_BYTES as u64 {
                    return Err(CodecError::LimitExceeded);
                }
                if len_raw == 0 {
                    return Err(CodecError::Malformed);
                }
                let len = len_raw as usize;
                // non-final segment must be 4096
                if si < segment_count - 1 && len != MAX_SEGMENT_BYTES {
                    return Err(CodecError::Malformed);
                }
                if rest.len() < len {
                    return Err(CodecError::Malformed);
                }
                // segment length checked before body availability; copy bounded
                bytes.extend_from_slice(&rest[..len]);
                rest = &rest[len..];
            }
            if bytes.is_empty() {
                return Err(CodecError::Malformed);
            }
            if bytes.len() > MAX_CHUNK_BYTES {
                return Err(CodecError::LimitExceeded);
            }
            if !rest.is_empty() {
                return Err(CodecError::Malformed);
            }
            Ok(TransferDown::Chunk(TransferChunk {
                epoch,
                transfer,
                index,
                bytes,
            }))
        }
        TRANSFER_TAG_CANCEL => {
            let epoch: [u8; 16] = take(&mut rest)?;
            let transfer = take_transfer_id(&mut rest)?;
            let reason_raw: u8 = take(&mut rest)?;
            let reason = CancelReason::from_u8(reason_raw).ok_or(CodecError::UnknownMessage)?;
            if !rest.is_empty() {
                return Err(CodecError::Malformed);
            }
            Ok(TransferDown::Cancel { epoch, transfer, reason })
        }
        TRANSFER_TAG_PUBLISHED | TRANSFER_TAG_RESYNC => {
            Err(CodecError::WrongDirection)
        }
        _ => Err(CodecError::UnknownMessage),
    }
}

pub fn encode_up(msg: &TransferUp) -> Result<Vec<u8>, CodecError> {
    match msg {
        TransferUp::Published { epoch, transfer } => {
            if transfer.get() == 0 {
                return Err(CodecError::Malformed);
            }
            let mut out = extend(Vec::new(), &TRANSFER_VERSION)?;
            out = extend(out, &LANE_SNAPSHOT)?;
            out = extend(out, &TRANSFER_TAG_PUBLISHED)?;
            out = extend(out, epoch)?;
            out = extend(out, &transfer.get())?;
            if out.len() > MAX_UP_FRAME_BYTES {
                return Err(CodecError::FrameTooLarge);
            }
            Ok(out)
        }
        TransferUp::Resync { epoch, after, reason } => {
            if ResyncReason::from_u8(reason.as_u8()).is_none() {
                return Err(CodecError::UnknownMessage);
            }
            let after_raw = after.map(|t| t.get()).unwrap_or(0);
            let mut out = extend(Vec::new(), &TRANSFER_VERSION)?;
            out = extend(out, &LANE_SNAPSHOT)?;
            out = extend(out, &TRANSFER_TAG_RESYNC)?;
            out = extend(out, epoch)?;
            out = extend(out, &after_raw)?;
            out = extend(out, &reason.as_u8())?;
            if out.len() > MAX_UP_FRAME_BYTES {
                return Err(CodecError::FrameTooLarge);
            }
            Ok(out)
        }
    }
}

pub fn decode_up(bytes: &[u8]) -> Result<TransferUp, CodecError> {
    if bytes.len() > MAX_UP_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    if bytes.is_empty() {
        return Err(CodecError::Malformed);
    }
    let mut rest = bytes;
    let version: u8 = take(&mut rest)?;
    if version != TRANSFER_VERSION {
        return Err(CodecError::UnsupportedVersion);
    }
    let lane: u8 = take(&mut rest)?;
    if lane != LANE_SNAPSHOT {
        return Err(CodecError::WrongDirection);
    }
    let tag: u8 = take(&mut rest)?;
    match tag {
        TRANSFER_TAG_PUBLISHED => {
            let epoch: [u8; 16] = take(&mut rest)?;
            let transfer = take_transfer_id(&mut rest)?;
            if !rest.is_empty() {
                return Err(CodecError::Malformed);
            }
            Ok(TransferUp::Published { epoch, transfer })
        }
        TRANSFER_TAG_RESYNC => {
            let epoch: [u8; 16] = take(&mut rest)?;
            let after_raw: u64 = take(&mut rest)?;
            let after = if after_raw == 0 {
                None
            } else {
                TransferId::new(after_raw)
            };
            let reason_raw: u8 = take(&mut rest)?;
            let reason = ResyncReason::from_u8(reason_raw).ok_or(CodecError::UnknownMessage)?;
            if !rest.is_empty() {
                return Err(CodecError::Malformed);
            }
            Ok(TransferUp::Resync { epoch, after, reason })
        }
        TRANSFER_TAG_BEGIN | TRANSFER_TAG_CHUNK | TRANSFER_TAG_CANCEL => {
            Err(CodecError::WrongDirection)
        }
        _ => Err(CodecError::UnknownMessage),
    }
}

// ---------------------------------------------------------------------------
// plan_transfer
// ---------------------------------------------------------------------------

pub fn plan_transfer(
    epoch: [u8; 16],
    transfer: TransferId,
    base_tick: u64,
    cursor: u64,
    segments_per_chunk: u8,
    body: &SnapshotBody,
) -> Result<Vec<Vec<u8>>, CodecError> {
    // cursor check
    if cursor > SEQ_LAST {
        return Err(CodecError::Malformed);
    }
    // segments_per_chunk
    if segments_per_chunk == 0 {
        return Err(CodecError::Malformed);
    }
    if segments_per_chunk > MAX_SEGMENTS_PER_CHUNK {
        return Err(CodecError::LimitExceeded);
    }
    // body
    let body_bytes = encode_body(body)?;
    // chunk count
    let total_bytes = body_bytes.len();
    let chunk_size = segments_per_chunk as usize * MAX_SEGMENT_BYTES;
    let chunk_count = total_bytes.div_ceil(chunk_size);
    if chunk_count == 0 {
        // shouldn't happen; body at least 2 bytes
    }
    if chunk_count > MAX_SNAPSHOT_CHUNKS {
        return Err(CodecError::LimitExceeded);
    }
    let begin = BeginTransfer {
        epoch,
        transfer,
        base_tick,
        cursor,
        total_bytes: total_bytes as u32,
        entity_count: body.entities.len() as u16,
        segments_per_chunk,
        chunk_count: chunk_count as u8,
    };
    begin.validate()?;
    // frames
    let mut frames = Vec::new();
    let begin_frame = encode_down(&TransferDown::Begin(begin.clone()))?;
    frames.push(begin_frame);
    for i in 0..chunk_count {
        let start = i * chunk_size;
        let end = ((i + 1) * chunk_size).min(total_bytes);
        let payload = body_bytes[start..end].to_vec();
        let chunk = TransferChunk {
            epoch,
            transfer,
            index: i as u8,
            bytes: payload,
        };
        let chunk_frame = encode_down(&TransferDown::Chunk(chunk))?;
        frames.push(chunk_frame);
    }
    Ok(frames)
}