//! Lane-1 movement subprotocol (T026p2, ADR-0036 L1–L6, D13).
//!
//! Lane 1 is its own separately versioned subprotocol: every message starts
//! with [`MOVEMENT_VERSION`] (`0xA1`), as the snapshot (`0x81`/`0x82`) and
//! resume (`0x91`) subprotocols have their own first bytes. Lane-0 v4 frames
//! start with byte 4, so every lane-0 codec refuses lane-1 bytes with
//! `UnsupportedVersion` and this codec refuses every other first byte the
//! same way. It reads no `World` and checks shape, not entitlement.
//!
//! Upstream (client to server) a [`MoveIntent`] is `0xA1, epoch[16], lane (1),
//! seq, actor, tag`: tag 0 `MoveTo` is one point, tag 1 `Halt` nothing. There
//! is no `observed_tick`. Downstream (server to client) a message is `0xA1,
//! lane (1), kind`: kind 0 is a [`MoveReceipt`], kind 1 a [`MoveSample`].
//! A sample is one self-contained datagram of at most
//! [`MAX_MOVE_DOWN_BYTES`] bytes and [`MAX_SAMPLE_ENTRIES`] entries, with no
//! `event_seq` and no epoch; [`sample_datagrams`] cuts one tick's entries into
//! such datagrams.
//!
//! Decode order is contract: whole-message cap, empty input, version, header
//! fields, body (each field checked when read), trailing-byte reject. Encode
//! checks, then writes, and never returns partial bytes.

use crate::codec_v4::{extend_point, take_point};
use crate::protocol::{MAX_INTENT_FRAME_BYTES, SEQ_FIRST, SEQ_LAST};
use crate::protocol_v4::{NetId, Point, RejectionCode};

/// Shape failure of the lane-1 wire: the same six variants as v1.
pub use crate::codec::CodecError;

/// Movement subprotocol version 1 (Reading 1): the first byte of every lane-1
/// message.
pub const MOVEMENT_VERSION: u8 = 0xA1;
/// The lane byte of every lane-1 message (0 combat, 2 snapshot, 3 session).
pub const LANE_MOVEMENT: u8 = 1;
/// [`MoveBody::MoveTo`] tag.
pub const MOVE_TAG_MOVE_TO: u8 = 0;
/// [`MoveBody::Halt`] tag.
pub const MOVE_TAG_HALT: u8 = 1;
/// Downstream kind byte of a [`MoveReceipt`].
pub const DOWN_KIND_RECEIPT: u8 = 0;
/// Downstream kind byte of a [`MoveSample`].
pub const DOWN_KIND_SAMPLE: u8 = 1;
/// [`MoveOutcome::Accepted`] tag.
pub const OUTCOME_TAG_ACCEPTED: u8 = 0;
/// [`MoveOutcome::Superseded`] tag.
pub const OUTCOME_TAG_SUPERSEDED: u8 = 1;
/// [`MoveOutcome::Rejected`] tag.
pub const OUTCOME_TAG_REJECTED: u8 = 2;
/// Largest lane-1 downstream message, receipt or sample (Reading 6).
pub const MAX_MOVE_DOWN_BYTES: usize = 1_024;
/// Largest encoded sample header: version 1 + lane 1 + kind 1 + server_tick 10
/// + processed_seq 10 + count 1.
pub const MAX_SAMPLE_HEADER_BYTES: usize = 24;
/// Largest encoded sample entry: entity 10 + three coordinates of 5 + moving 1.
pub const MAX_SAMPLE_ENTRY_BYTES: usize = 26;
/// Most entries in one sample: the largest `n` with 24 + 26 `n` <= 1,024.
pub const MAX_SAMPLE_ENTRIES: usize = 38;
const _: () = assert!(
    MAX_SAMPLE_HEADER_BYTES + MAX_SAMPLE_ENTRIES * MAX_SAMPLE_ENTRY_BYTES <= MAX_MOVE_DOWN_BYTES
);
const _: () = assert!(
    MAX_SAMPLE_HEADER_BYTES + (MAX_SAMPLE_ENTRIES + 1) * MAX_SAMPLE_ENTRY_BYTES
        > MAX_MOVE_DOWN_BYTES
);
/// Policy, never wire (ADR-0036 U7; Reading 14): sustained lane-1 intents per
/// second per peer, enforced by the host.
pub const POLICY_MOVE_INTENTS_PER_SEC: u64 = 20;
/// Policy, never wire: lane-1 intent burst per peer.
pub const POLICY_MOVE_INTENT_BURST: u64 = 40;
/// Policy, never wire: unreceipted lane-1 intents in flight per peer.
pub const POLICY_MOVE_IN_FLIGHT: usize = 8;
const _: () = assert!(POLICY_MOVE_IN_FLIGHT <= crate::protocol::POLICY_RETRY_CACHE_PER_PEER_LANE);

/// A lane-1 intent verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveBody {
    /// Walk the header's actor to `destination` (tag 0).
    MoveTo {
        /// Where to walk, within U3 by its type.
        destination: Point,
    },
    /// Stop the header's actor where it stands (tag 1).
    Halt,
}

/// One lane-1 intent. The lane is implied by the type (wire byte
/// [`LANE_MOVEMENT`]); there is no `observed_tick`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveIntent {
    /// Session generation this intent belongs to.
    pub epoch: [u8; 16],
    /// Per-lane intent identity, `SEQ_FIRST..=SEQ_LAST`.
    pub seq: u64,
    /// Acting replica entity.
    pub actor: NetId,
    /// The verb.
    pub body: MoveBody,
}

/// The host's verdict on one lane-1 intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveOutcome {
    /// The route was set; walking starts at `start_tick` (tag 0).
    Accepted {
        /// Server tick at which the actor starts moving.
        start_tick: u64,
    },
    /// A later intent for the same actor replaced this one (tag 1).
    Superseded,
    /// The intent was refused (tag 2), with a v3 code.
    Rejected(RejectionCode),
}

/// Exactly one receipt per admitted lane-1 intent, in `seq` order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveReceipt {
    /// Session generation of the intent.
    pub epoch: [u8; 16],
    /// The receipted intent's `seq`, `SEQ_FIRST..=SEQ_LAST`.
    pub seq: u64,
    /// The intent's actor.
    pub actor: NetId,
    /// Server tick at which the host processed the intent.
    pub processed_tick: u64,
    /// The verdict.
    pub outcome: MoveOutcome,
}

/// One entity's sampled position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleEntry {
    /// The sampled entity.
    pub entity: NetId,
    /// Where it is, within U3 by its type.
    pub point: Point,
    /// Whether it currently has a route.
    pub moving: bool,
}

/// One position-sample datagram: at most [`MAX_SAMPLE_ENTRIES`] entries in
/// strictly ascending `NetId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveSample {
    /// Server tick the positions were taken at.
    pub server_tick: u64,
    /// Highest lane-1 `seq` receipted to the receiving peer (0 before any).
    pub processed_seq: u64,
    /// The sampled entities.
    pub entries: Vec<SampleEntry>,
}

/// A lane-1 downstream message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveDown {
    /// Kind 0.
    Receipt(MoveReceipt),
    /// Kind 1.
    Sample(MoveSample),
}

// ---------------------------------------------------------------------------
// postcard plumbing (codec_v3's private helpers duplicated, only those used).
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

/// Decodes a nonzero replica id.
fn take_net_id(rest: &mut &[u8]) -> Result<NetId, CodecError> {
    let raw: u64 = take(rest)?;
    NetId::new(raw).ok_or(CodecError::Malformed)
}

/// Decodes a lane-1 sequence number (`SEQ_FIRST..=SEQ_LAST`).
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

// ---------------------------------------------------------------------------
// Intent codec.
// ---------------------------------------------------------------------------

/// Encodes one lane-1 intent to its canonical bytes (first byte `0xA1`).
///
/// Checks `seq` (`Malformed`), then writes the layout; the intent cap cannot
/// be reached (54 bytes at most).
pub fn encode_intent(intent: &MoveIntent) -> Result<Vec<u8>, CodecError> {
    check_seq(intent.seq)?;
    let mut out = extend(Vec::new(), &MOVEMENT_VERSION)?;
    out = extend(out, &intent.epoch)?;
    out = extend(out, &LANE_MOVEMENT)?;
    out = extend(out, &intent.seq)?;
    out = extend(out, &intent.actor.get())?;
    match intent.body {
        MoveBody::MoveTo { destination } => {
            out = extend(out, &MOVE_TAG_MOVE_TO)?;
            out = extend_point(out, destination)?;
        }
        MoveBody::Halt => out = extend(out, &MOVE_TAG_HALT)?,
    }
    Ok(out)
}

/// Decodes one lane-1 intent.
///
/// Order: cap (4,096), empty, version, `epoch`, lane, `seq` range, nonzero
/// `actor`, tag, the `MoveTo` point, trailing bytes.
pub fn decode_intent(bytes: &[u8]) -> Result<MoveIntent, CodecError> {
    if bytes.len() > MAX_INTENT_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    let mut rest = bytes;
    let version: u8 = take(&mut rest)?;
    if version != MOVEMENT_VERSION {
        return Err(CodecError::UnsupportedVersion);
    }
    let epoch: [u8; 16] = take(&mut rest)?;
    let lane: u8 = take(&mut rest)?;
    if lane != LANE_MOVEMENT {
        return Err(CodecError::WrongDirection);
    }
    let seq = take_seq(&mut rest)?;
    let actor = take_net_id(&mut rest)?;
    let tag: u8 = take(&mut rest)?;
    let body = match tag {
        MOVE_TAG_MOVE_TO => MoveBody::MoveTo {
            destination: take_point(&mut rest)?,
        },
        MOVE_TAG_HALT => MoveBody::Halt,
        _ => return Err(CodecError::UnknownMessage),
    };
    if !rest.is_empty() {
        return Err(CodecError::Malformed);
    }
    Ok(MoveIntent {
        epoch,
        seq,
        actor,
        body,
    })
}

// ---------------------------------------------------------------------------
// Downstream codec.
// ---------------------------------------------------------------------------

/// Encodes one lane-1 downstream message to its canonical bytes (first byte
/// `0xA1`).
///
/// A receipt checks `seq` (`Malformed`); a sample runs [`check_sample`] and
/// returns exactly its error. Then the layout is written; the downstream cap
/// cannot be reached (60 bytes for a receipt, 1,012 for a sample).
pub fn encode_down(msg: &MoveDown) -> Result<Vec<u8>, CodecError> {
    match msg {
        MoveDown::Receipt(receipt) => {
            check_seq(receipt.seq)?;
            let mut out = extend(Vec::new(), &MOVEMENT_VERSION)?;
            out = extend(out, &LANE_MOVEMENT)?;
            out = extend(out, &DOWN_KIND_RECEIPT)?;
            out = extend(out, &receipt.epoch)?;
            out = extend(out, &receipt.seq)?;
            out = extend(out, &receipt.actor.get())?;
            out = extend(out, &receipt.processed_tick)?;
            match receipt.outcome {
                MoveOutcome::Accepted { start_tick } => {
                    out = extend(out, &OUTCOME_TAG_ACCEPTED)?;
                    extend(out, &start_tick)
                }
                MoveOutcome::Superseded => extend(out, &OUTCOME_TAG_SUPERSEDED),
                MoveOutcome::Rejected(code) => {
                    out = extend(out, &OUTCOME_TAG_REJECTED)?;
                    extend(out, &code.as_u8())
                }
            }
        }
        MoveDown::Sample(sample) => {
            check_sample(sample)?;
            let mut out = extend(Vec::new(), &MOVEMENT_VERSION)?;
            out = extend(out, &LANE_MOVEMENT)?;
            out = extend(out, &DOWN_KIND_SAMPLE)?;
            out = extend(out, &sample.server_tick)?;
            out = extend(out, &sample.processed_seq)?;
            out = extend(out, &(sample.entries.len() as u64))?;
            for entry in &sample.entries {
                out = extend(out, &entry.entity.get())?;
                out = extend_point(out, entry.point)?;
                out = extend(out, &entry.moving)?;
            }
            Ok(out)
        }
    }
}

/// Decodes one lane-1 downstream message.
///
/// Order: cap ([`MAX_MOVE_DOWN_BYTES`]), empty, version, lane, kind, the body,
/// trailing bytes.
pub fn decode_down(bytes: &[u8]) -> Result<MoveDown, CodecError> {
    if bytes.len() > MAX_MOVE_DOWN_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    let mut rest = bytes;
    let version: u8 = take(&mut rest)?;
    if version != MOVEMENT_VERSION {
        return Err(CodecError::UnsupportedVersion);
    }
    let lane: u8 = take(&mut rest)?;
    if lane != LANE_MOVEMENT {
        return Err(CodecError::WrongDirection);
    }
    let kind: u8 = take(&mut rest)?;
    let msg = match kind {
        DOWN_KIND_RECEIPT => MoveDown::Receipt(take_receipt(&mut rest)?),
        DOWN_KIND_SAMPLE => MoveDown::Sample(take_sample(&mut rest)?),
        _ => return Err(CodecError::UnknownMessage),
    };
    if !rest.is_empty() {
        return Err(CodecError::Malformed);
    }
    Ok(msg)
}

/// Decodes a receipt body (after the kind byte).
fn take_receipt(rest: &mut &[u8]) -> Result<MoveReceipt, CodecError> {
    let epoch: [u8; 16] = take(rest)?;
    let seq = take_seq(rest)?;
    let actor = take_net_id(rest)?;
    let processed_tick: u64 = take(rest)?;
    let tag: u8 = take(rest)?;
    let outcome = match tag {
        OUTCOME_TAG_ACCEPTED => MoveOutcome::Accepted {
            start_tick: take(rest)?,
        },
        OUTCOME_TAG_SUPERSEDED => MoveOutcome::Superseded,
        OUTCOME_TAG_REJECTED => {
            let code: u8 = take(rest)?;
            MoveOutcome::Rejected(RejectionCode::from_u8(code).ok_or(CodecError::UnknownMessage)?)
        }
        _ => return Err(CodecError::UnknownMessage),
    };
    Ok(MoveReceipt {
        epoch,
        seq,
        actor,
        processed_tick,
        outcome,
    })
}

/// Decodes a sample body (after the kind byte).
fn take_sample(rest: &mut &[u8]) -> Result<MoveSample, CodecError> {
    let server_tick: u64 = take(rest)?;
    let processed_seq: u64 = take(rest)?;
    if processed_seq > SEQ_LAST {
        return Err(CodecError::Malformed);
    }
    // A postcard `u64` is an LEB128 varint; one that overflows is `Malformed`.
    let count: u64 = take(rest)?;
    if count > MAX_SAMPLE_ENTRIES as u64 {
        return Err(CodecError::LimitExceeded);
    }
    let mut entries = Vec::new();
    let mut previous = 0u64;
    for _ in 0..count {
        let entity = take_net_id(rest)?;
        if entity.get() <= previous {
            return Err(CodecError::Malformed);
        }
        previous = entity.get();
        let point = take_point(rest)?;
        let moving: bool = take(rest)?;
        entries.push(SampleEntry {
            entity,
            point,
            moving,
        });
    }
    Ok(MoveSample {
        server_tick,
        processed_seq,
        entries,
    })
}

// ---------------------------------------------------------------------------
// Sample checks and splitter.
// ---------------------------------------------------------------------------

/// Checks `processed_seq` (above `SEQ_LAST`: `Malformed`).
fn check_processed_seq(processed_seq: u64) -> Result<(), CodecError> {
    if processed_seq > SEQ_LAST {
        return Err(CodecError::Malformed);
    }
    Ok(())
}

/// Checks that entities are strictly ascending (`Malformed` otherwise).
fn check_ascending(entries: &[SampleEntry]) -> Result<(), CodecError> {
    if entries
        .windows(2)
        .any(|pair| pair[1].entity.get() <= pair[0].entity.get())
    {
        return Err(CodecError::Malformed);
    }
    Ok(())
}

/// M-6's encode checks for one sample, in wire order: `processed_seq` above
/// `SEQ_LAST` (`Malformed`), more than [`MAX_SAMPLE_ENTRIES`] entries
/// (`LimitExceeded`), an entity not strictly above the previous one
/// (`Malformed`).
pub fn check_sample(sample: &MoveSample) -> Result<(), CodecError> {
    check_processed_seq(sample.processed_seq)?;
    if sample.entries.len() > MAX_SAMPLE_ENTRIES {
        return Err(CodecError::LimitExceeded);
    }
    check_ascending(&sample.entries)
}

/// M-7: cuts one tick's entries into encoded sample datagrams.
///
/// Empty `entries` gives no datagrams. Otherwise `processed_seq` and strict
/// ascent are checked over the whole input (first failure, no output), then
/// the entries are cut in order into chunks of [`MAX_SAMPLE_ENTRIES`] (the
/// last may be shorter), each encoded as a [`MoveSample`] with the given
/// `server_tick` and `processed_seq`. Every datagram is at most 1,012 bytes.
pub fn sample_datagrams(
    server_tick: u64,
    processed_seq: u64,
    entries: &[SampleEntry],
) -> Result<Vec<Vec<u8>>, CodecError> {
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    check_processed_seq(processed_seq)?;
    check_ascending(entries)?;
    entries
        .chunks(MAX_SAMPLE_ENTRIES)
        .map(|chunk| {
            encode_down(&MoveDown::Sample(MoveSample {
                server_tick,
                processed_seq,
                entries: chunk.to_vec(),
            }))
        })
        .collect()
}
