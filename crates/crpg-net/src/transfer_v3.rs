//! Protocol v3 snapshot transfer, version byte `0x82` (T140d, ADR-0029 N3).
//!
//! `0x82` is the `0x81` transfer subprotocol ([`crate::codec_transfer`], T024)
//! with a new first byte, a wider body and a larger envelope. Every frame
//! layout is `0x81`'s, and every check is T024's, in T024's order, with
//! exactly three bounds replaced: a Begin's `total_bytes` cap becomes
//! [`MAX_SNAPSHOT_BYTES_V3`] (4,194,304), and a Begin's `chunk_count` cap and
//! a Chunk's `index` bound become [`MAX_SNAPSHOT_CHUNKS_V3`] (128). The v1,
//! v2 and `0x81` caps in [`crate::protocol`] stay 1 MiB and 32 chunks.
//!
//! Because the `0x81` codec and [`BeginTransfer::validate`] refuse the wider
//! values, this module encodes and decodes Begin and Chunk itself, through
//! [`check_begin`] and [`chunk_len`], and never calls either original.
//!
//! The body is the `0x81` body with a fact set (the T140b fact-set layout)
//! after each entity's health, then an encounter block and a timeline block.
//! Host facts are never in the body: they are per viewer and not state at
//! the cursor. The module reads no `World` and mints no id.

use crate::codec_v3::{
    extend_encounter, extend_fact_set, extend_timeline, take_encounter, take_fact_set,
    take_timeline,
};
use crate::protocol::{MAX_VISIBLE_ENTITIES, SEQ_LAST};
use crate::protocol_transfer::{
    CancelReason, ResyncReason, MAX_CHUNK_BYTES, MAX_DOWN_FRAME_BYTES, MAX_SEGMENTS_PER_CHUNK,
    MAX_SEGMENT_BYTES, MAX_UP_FRAME_BYTES, TRANSFER_TAG_BEGIN, TRANSFER_TAG_CANCEL,
    TRANSFER_TAG_CHUNK, TRANSFER_TAG_PUBLISHED, TRANSFER_TAG_RESYNC,
};
use crate::protocol_v3::{EncounterFacts, FactSet, NetId, TimelineEntry};

pub use crate::codec::CodecError;
pub use crate::protocol_transfer::{
    BeginTransfer, HealthFact, TransferChunk, TransferDown, TransferId, TransferUp, TurnFact,
    LANE_SNAPSHOT,
};

/// First byte of every v3 transfer frame.
pub const TRANSFER_VERSION_V3: u8 = 0x82;

/// The v3 snapshot envelope (Reading 15): 4 MiB. The v1/`0x81` cap,
/// [`crate::protocol::MAX_SNAPSHOT_BYTES`], stays 1 MiB.
pub const MAX_SNAPSHOT_BYTES_V3: usize = 4 * 1_048_576;

/// The v3 chunk cap: 128 chunks of [`MAX_CHUNK_BYTES`]. The v1/`0x81` cap,
/// [`crate::protocol::MAX_SNAPSHOT_CHUNKS`], stays 32.
pub const MAX_SNAPSHOT_CHUNKS_V3: usize = MAX_SNAPSHOT_BYTES_V3 / MAX_CHUNK_BYTES;

/// The largest valid v3 body (Behaviour D-4): 2 (count) + 1,024 x 2,766
/// (a fully disclosed entity) + 22 (turn) + 17 (encounter) + 15,363
/// (timeline). It fits [`MAX_SNAPSHOT_BYTES_V3`] in 87 chunks of 8 segments.
pub const MAX_ENCODED_BODY_BYTES_V3: usize = 2_847_788;

const _: () = assert!(MAX_ENCODED_BODY_BYTES_V3 <= MAX_SNAPSHOT_BYTES_V3);
// BeginTransfer.total_bytes is a u32.
const _: () = assert!(MAX_SNAPSHOT_BYTES_V3 <= u32::MAX as usize);
// chunk_count and a chunk index stay one raw byte.
const _: () = assert!(MAX_SNAPSHOT_CHUNKS_V3 <= u8::MAX as usize);
const _: () = assert!(MAX_SNAPSHOT_CHUNKS_V3 * MAX_CHUNK_BYTES == MAX_SNAPSHOT_BYTES_V3);

// ---------------------------------------------------------------------------
// postcard plumbing (T024's shapes duplicated, never shared).
// ---------------------------------------------------------------------------

/// Extends `out` with one `postcard`-encoded primitive.
fn extend<T: serde::Serialize + ?Sized>(out: Vec<u8>, value: &T) -> Result<Vec<u8>, CodecError> {
    postcard::to_extend(value, out).map_err(|_| CodecError::Malformed)
}

/// Decodes one `postcard` primitive from the head of `rest`, advancing it.
fn take<'a, T: serde::Deserialize<'a>>(rest: &mut &'a [u8]) -> Result<T, CodecError> {
    let (value, remaining) =
        postcard::take_from_bytes::<T>(rest).map_err(|_| CodecError::Malformed)?;
    *rest = remaining;
    Ok(value)
}

/// Decodes a nonzero transfer id.
fn take_transfer_id(rest: &mut &[u8]) -> Result<TransferId, CodecError> {
    let raw: u64 = take(rest)?;
    TransferId::new(raw).ok_or(CodecError::Malformed)
}

// ---------------------------------------------------------------------------
// Begin checks.
// ---------------------------------------------------------------------------

/// `BeginTransfer::validate`'s checks, in its order, with the v3 caps (D-1):
/// transfer id, `cursor` (`Malformed` above `SEQ_LAST`), `total_bytes` (0
/// `Malformed`, over 4,194,304 `LimitExceeded`), `entity_count` (over 1,024
/// `LimitExceeded`), `segments_per_chunk` (0 `Malformed`, over 8
/// `LimitExceeded`), `chunk_count` (0 `Malformed`, over 128 `LimitExceeded`),
/// then `chunk_count` must be the canonical cut (`Malformed`).
pub fn check_begin(begin: &BeginTransfer) -> Result<(), CodecError> {
    if begin.transfer.get() == 0 {
        return Err(CodecError::Malformed);
    }
    if begin.cursor > SEQ_LAST {
        return Err(CodecError::Malformed);
    }
    if begin.total_bytes == 0 {
        return Err(CodecError::Malformed);
    }
    if begin.total_bytes as usize > MAX_SNAPSHOT_BYTES_V3 {
        return Err(CodecError::LimitExceeded);
    }
    if begin.entity_count as usize > MAX_VISIBLE_ENTITIES {
        return Err(CodecError::LimitExceeded);
    }
    if begin.segments_per_chunk == 0 {
        return Err(CodecError::Malformed);
    }
    if begin.segments_per_chunk > MAX_SEGMENTS_PER_CHUNK {
        return Err(CodecError::LimitExceeded);
    }
    if begin.chunk_count == 0 {
        return Err(CodecError::Malformed);
    }
    if begin.chunk_count as usize > MAX_SNAPSHOT_CHUNKS_V3 {
        return Err(CodecError::LimitExceeded);
    }
    let chunk_size = begin.segments_per_chunk as usize * MAX_SEGMENT_BYTES;
    let expected = (begin.total_bytes as usize).div_ceil(chunk_size);
    if expected != begin.chunk_count as usize {
        return Err(CodecError::Malformed);
    }
    Ok(())
}

/// `BeginTransfer::chunk_len` with [`check_begin`] in place of `validate`:
/// the exact payload length of chunk `index`, or `None`.
pub fn chunk_len(begin: &BeginTransfer, index: u8) -> Option<usize> {
    if index >= begin.chunk_count {
        return None;
    }
    if check_begin(begin).is_err() {
        return None;
    }
    let chunk_size = begin.segments_per_chunk as usize * MAX_SEGMENT_BYTES;
    let total = begin.total_bytes as usize;
    let start = index as usize * chunk_size;
    if start >= total {
        return None;
    }
    let end = ((index as usize + 1) * chunk_size).min(total);
    Some(end - start)
}

// ---------------------------------------------------------------------------
// Body.
// ---------------------------------------------------------------------------

/// One entity of a v3 snapshot: its health and its fact set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotEntityV3 {
    /// Replica id.
    pub entity: NetId,
    /// Health, if disclosed.
    pub health: Option<HealthFact>,
    /// The viewer's image of the entity's facts; a withheld family is `None`.
    pub facts: FactSet,
}

/// The v3 snapshot body: the `0x81` body plus facts, encounter and timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotBodyV3 {
    /// Strictly ascending [`NetId`], at most `MAX_VISIBLE_ENTITIES`.
    pub entities: Vec<SnapshotEntityV3>,
    /// Round and active combatant, if disclosed.
    pub turn: Option<TurnFact>,
    /// The encounter image.
    pub encounter: EncounterFacts,
    /// The timeline image, in authority order; `None` is withheld.
    pub timeline: Option<Vec<TimelineEntry>>,
}

/// Whether `net` is one of `entities` (strictly ascending, so searchable).
fn is_member(entities: &[SnapshotEntityV3], net: NetId) -> bool {
    entities
        .binary_search_by_key(&net.get(), |entity| entity.entity.get())
        .is_ok()
}

/// Encodes a v3 body (D-2), checking in the decode order (D-3/D-4) so that it
/// refuses exactly what [`decode_body`] refuses, with the same error. It
/// never returns partial bytes.
pub fn encode_body(body: &SnapshotBodyV3) -> Result<Vec<u8>, CodecError> {
    if body.entities.len() > MAX_VISIBLE_ENTITIES {
        return Err(CodecError::LimitExceeded);
    }
    let mut out = extend(Vec::new(), &(body.entities.len() as u64))?;
    let mut last: Option<u64> = None;
    for entity in &body.entities {
        let raw = entity.entity.get();
        if last.is_some_and(|previous| raw <= previous) {
            return Err(CodecError::Malformed);
        }
        last = Some(raw);
        out = extend(out, &raw)?;
        match &entity.health {
            None => out = extend(out, &false)?,
            Some(health) => {
                out = extend(out, &true)?;
                out = extend(out, &health.health)?;
                out = extend(out, &health.max_health)?;
                out = extend(out, &health.dead)?;
            }
        }
        out = extend_fact_set(out, &entity.facts)?;
    }
    for entity in &body.entities {
        if let Some(Some(order)) = &entity.facts.order {
            if !is_member(&body.entities, order.target) {
                return Err(CodecError::Malformed);
            }
        }
    }
    if let Some(turn) = &body.turn {
        if let Some(active) = turn.active {
            if !is_member(&body.entities, active) {
                return Err(CodecError::Malformed);
            }
        }
    }
    match &body.turn {
        None => out = extend(out, &false)?,
        Some(turn) => {
            out = extend(out, &true)?;
            match turn.active {
                None => out = extend(out, &false)?,
                Some(active) => {
                    out = extend(out, &true)?;
                    out = extend(out, &active.get())?;
                }
            }
            out = extend(out, &turn.round)?;
        }
    }
    out = extend_encounter(out, &body.encounter)?;
    out = extend_timeline(out, &body.timeline)?;
    if let Some(entries) = &body.timeline {
        for entry in entries {
            if !is_member(&body.entities, entry.entity) {
                return Err(CodecError::Malformed);
            }
        }
    }
    if out.len() > MAX_SNAPSHOT_BYTES_V3 {
        return Err(CodecError::LimitExceeded);
    }
    Ok(out)
}

/// Decodes a v3 body in the D-3 order: size cap, entity count, per entity
/// (id, ascending, health, fact set), order-target membership, turn,
/// encounter, timeline (each entry's entity must be an entity), trailing
/// bytes.
pub fn decode_body(bytes: &[u8]) -> Result<SnapshotBodyV3, CodecError> {
    if bytes.len() > MAX_SNAPSHOT_BYTES_V3 {
        return Err(CodecError::LimitExceeded);
    }
    let mut rest = bytes;
    let count: u64 = take(&mut rest)?;
    if count > MAX_VISIBLE_ENTITIES as u64 {
        return Err(CodecError::LimitExceeded);
    }
    // Grows by push: no reservation from the untrusted count.
    let mut entities: Vec<SnapshotEntityV3> = Vec::new();
    let mut last: Option<u64> = None;
    for _ in 0..count {
        let raw: u64 = take(&mut rest)?;
        let entity = NetId::new(raw).ok_or(CodecError::Malformed)?;
        if last.is_some_and(|previous| raw <= previous) {
            return Err(CodecError::Malformed);
        }
        last = Some(raw);
        let present: bool = take(&mut rest)?;
        let health = if present {
            let health: u32 = take(&mut rest)?;
            let max_health: u32 = take(&mut rest)?;
            let dead: bool = take(&mut rest)?;
            Some(HealthFact {
                health,
                max_health,
                dead,
            })
        } else {
            None
        };
        let facts = take_fact_set(&mut rest)?;
        entities.push(SnapshotEntityV3 {
            entity,
            health,
            facts,
        });
    }
    for entity in &entities {
        if let Some(Some(order)) = &entity.facts.order {
            if !is_member(&entities, order.target) {
                return Err(CodecError::Malformed);
            }
        }
    }
    let turn_present: bool = take(&mut rest)?;
    let turn = if turn_present {
        let active_present: bool = take(&mut rest)?;
        let active = if active_present {
            let raw: u64 = take(&mut rest)?;
            let id = NetId::new(raw).ok_or(CodecError::Malformed)?;
            if !is_member(&entities, id) {
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
    let encounter = take_encounter(&mut rest)?;
    let timeline = take_timeline(&mut rest)?;
    if let Some(entries) = &timeline {
        for entry in entries {
            if !is_member(&entities, entry.entity) {
                return Err(CodecError::Malformed);
            }
        }
    }
    if !rest.is_empty() {
        return Err(CodecError::Malformed);
    }
    Ok(SnapshotBodyV3 {
        entities,
        turn,
        encounter,
        timeline,
    })
}

// ---------------------------------------------------------------------------
// Frame codecs.
// ---------------------------------------------------------------------------

/// Encodes one downstream `0x82` frame. A Begin is checked by
/// [`check_begin`], a Chunk's `index` by [`MAX_SNAPSHOT_CHUNKS_V3`]; every
/// other check is T024's. It never returns partial bytes.
pub fn encode_down(msg: &TransferDown) -> Result<Vec<u8>, CodecError> {
    match msg {
        TransferDown::Begin(begin) => {
            check_begin(begin)?;
            let mut out = extend(Vec::new(), &TRANSFER_VERSION_V3)?;
            out = extend(out, &LANE_SNAPSHOT)?;
            out = extend(out, &TRANSFER_TAG_BEGIN)?;
            out = extend(out, &begin.epoch)?;
            out = extend(out, &begin.transfer.get())?;
            out = extend(out, &begin.base_tick)?;
            out = extend(out, &begin.cursor)?;
            out = extend(out, &(begin.total_bytes as u64))?;
            out = extend(out, &(begin.entity_count as u64))?;
            out = extend(out, &begin.segments_per_chunk)?;
            out = extend(out, &begin.chunk_count)?;
            if out.len() > MAX_DOWN_FRAME_BYTES {
                return Err(CodecError::FrameTooLarge);
            }
            Ok(out)
        }
        TransferDown::Chunk(chunk) => {
            if chunk.index as usize >= MAX_SNAPSHOT_CHUNKS_V3 {
                return Err(CodecError::LimitExceeded);
            }
            if chunk.bytes.is_empty() {
                return Err(CodecError::Malformed);
            }
            if chunk.bytes.len() > MAX_CHUNK_BYTES {
                return Err(CodecError::LimitExceeded);
            }
            let segments: Vec<&[u8]> = chunk.bytes.chunks(MAX_SEGMENT_BYTES).collect();
            if segments.len() > MAX_SEGMENTS_PER_CHUNK as usize {
                return Err(CodecError::LimitExceeded);
            }
            let mut out = extend(Vec::new(), &TRANSFER_VERSION_V3)?;
            out = extend(out, &LANE_SNAPSHOT)?;
            out = extend(out, &TRANSFER_TAG_CHUNK)?;
            out = extend(out, &chunk.epoch)?;
            out = extend(out, &chunk.transfer.get())?;
            out = extend(out, &chunk.index)?;
            out = extend(out, &(segments.len() as u8))?;
            for segment in segments {
                out = extend(out, &(segment.len() as u64))?;
                out.extend_from_slice(segment);
            }
            if out.len() > MAX_DOWN_FRAME_BYTES {
                return Err(CodecError::FrameTooLarge);
            }
            Ok(out)
        }
        TransferDown::Cancel {
            epoch,
            transfer,
            reason,
        } => {
            if transfer.get() == 0 {
                return Err(CodecError::Malformed);
            }
            if CancelReason::from_u8(reason.as_u8()).is_none() {
                return Err(CodecError::UnknownMessage);
            }
            let mut out = extend(Vec::new(), &TRANSFER_VERSION_V3)?;
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

/// Decodes one downstream `0x82` frame: frame cap, empty input, version,
/// lane, tag, then T024's field order with the v3 bounds (D-1).
pub fn decode_down(bytes: &[u8]) -> Result<TransferDown, CodecError> {
    if bytes.len() > MAX_DOWN_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    if bytes.is_empty() {
        return Err(CodecError::Malformed);
    }
    let mut rest = bytes;
    let version: u8 = take(&mut rest)?;
    if version != TRANSFER_VERSION_V3 {
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
            if total_bytes_raw > MAX_SNAPSHOT_BYTES_V3 as u64 {
                return Err(CodecError::LimitExceeded);
            }
            if total_bytes_raw == 0 {
                return Err(CodecError::Malformed);
            }
            let total_bytes = total_bytes_raw as u32;
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
            if chunk_count as usize > MAX_SNAPSHOT_CHUNKS_V3 {
                return Err(CodecError::LimitExceeded);
            }
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
            check_begin(&begin)?;
            Ok(TransferDown::Begin(begin))
        }
        TRANSFER_TAG_CHUNK => {
            let epoch: [u8; 16] = take(&mut rest)?;
            let transfer = take_transfer_id(&mut rest)?;
            let index: u8 = take(&mut rest)?;
            if index as usize >= MAX_SNAPSHOT_CHUNKS_V3 {
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
            for position in 0..segment_count {
                let len_raw: u64 = take(&mut rest)?;
                if len_raw > MAX_SEGMENT_BYTES as u64 {
                    return Err(CodecError::LimitExceeded);
                }
                if len_raw == 0 {
                    return Err(CodecError::Malformed);
                }
                let len = len_raw as usize;
                if position < segment_count - 1 && len != MAX_SEGMENT_BYTES {
                    return Err(CodecError::Malformed);
                }
                if rest.len() < len {
                    return Err(CodecError::Malformed);
                }
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
            Ok(TransferDown::Cancel {
                epoch,
                transfer,
                reason,
            })
        }
        TRANSFER_TAG_PUBLISHED | TRANSFER_TAG_RESYNC => Err(CodecError::WrongDirection),
        _ => Err(CodecError::UnknownMessage),
    }
}

/// Encodes one upstream `0x82` frame (`Published` or `Resync`).
pub fn encode_up(msg: &TransferUp) -> Result<Vec<u8>, CodecError> {
    match msg {
        TransferUp::Published { epoch, transfer } => {
            if transfer.get() == 0 {
                return Err(CodecError::Malformed);
            }
            let mut out = extend(Vec::new(), &TRANSFER_VERSION_V3)?;
            out = extend(out, &LANE_SNAPSHOT)?;
            out = extend(out, &TRANSFER_TAG_PUBLISHED)?;
            out = extend(out, epoch)?;
            out = extend(out, &transfer.get())?;
            if out.len() > MAX_UP_FRAME_BYTES {
                return Err(CodecError::FrameTooLarge);
            }
            Ok(out)
        }
        TransferUp::Resync {
            epoch,
            after,
            reason,
        } => {
            if ResyncReason::from_u8(reason.as_u8()).is_none() {
                return Err(CodecError::UnknownMessage);
            }
            let after_raw = after.map(|transfer| transfer.get()).unwrap_or(0);
            let mut out = extend(Vec::new(), &TRANSFER_VERSION_V3)?;
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

/// Decodes one upstream `0x82` frame: frame cap, empty input, version, lane,
/// tag, then T024's fields.
pub fn decode_up(bytes: &[u8]) -> Result<TransferUp, CodecError> {
    if bytes.len() > MAX_UP_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    if bytes.is_empty() {
        return Err(CodecError::Malformed);
    }
    let mut rest = bytes;
    let version: u8 = take(&mut rest)?;
    if version != TRANSFER_VERSION_V3 {
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
            Ok(TransferUp::Resync {
                epoch,
                after,
                reason,
            })
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

/// Plans one `0x82` transfer: the Begin frame, then each Chunk frame, cut
/// canonically from the encoded body.
///
/// Checks, in order: `cursor` above `SEQ_LAST` (`Malformed`),
/// `segments_per_chunk` 0 (`Malformed`) or above 8 (`LimitExceeded`), the
/// body ([`encode_body`]'s error), then a chunk count above
/// [`MAX_SNAPSHOT_CHUNKS_V3`] (`LimitExceeded`, computed in `usize` before
/// any narrowing). The concatenated Chunk payloads are exactly the body bytes.
pub fn plan_transfer(
    epoch: [u8; 16],
    transfer: TransferId,
    base_tick: u64,
    cursor: u64,
    segments_per_chunk: u8,
    body: &SnapshotBodyV3,
) -> Result<Vec<Vec<u8>>, CodecError> {
    if cursor > SEQ_LAST {
        return Err(CodecError::Malformed);
    }
    if segments_per_chunk == 0 {
        return Err(CodecError::Malformed);
    }
    if segments_per_chunk > MAX_SEGMENTS_PER_CHUNK {
        return Err(CodecError::LimitExceeded);
    }
    let body_bytes = encode_body(body)?;
    let total_bytes = body_bytes.len();
    let chunk_size = segments_per_chunk as usize * MAX_SEGMENT_BYTES;
    let chunk_count = total_bytes.div_ceil(chunk_size);
    if chunk_count > MAX_SNAPSHOT_CHUNKS_V3 {
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
    check_begin(&begin)?;
    let mut frames = Vec::with_capacity(chunk_count + 1);
    frames.push(encode_down(&TransferDown::Begin(begin))?);
    for (index, payload) in body_bytes.chunks(chunk_size).enumerate() {
        frames.push(encode_down(&TransferDown::Chunk(TransferChunk {
            epoch,
            transfer,
            index: index as u8,
            bytes: payload.to_vec(),
        }))?);
    }
    Ok(frames)
}
