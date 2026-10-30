//! Bounded codec for the resume subprotocol (T025a1).
//!
//! Four functions: [`encode_request`]/[`decode_request`] (upstream `0x40`)
//! and [`encode_down`]/[`decode_down`] (downstream `0x30`/`0x31`/`0x32`).
//! The layout, bounds and decode precedence are pinned by `tasks/T025a.md`
//! S§6–S§7: whole-frame cap before any read, version `0x91`, lane `3`, tag,
//! then fields left to right, then a trailing-byte reject. Tags are matched
//! as explicit `u8` values, never derived serde tags.

use crate::protocol::{LANE_COMBAT, SEQ_LAST};
use crate::protocol_resume::*;

/// Re-export.
pub use crate::codec::CodecError;

// plumbing (duplicated for this module, as `codec_transfer` did).
fn extend<T: serde::Serialize + ?Sized>(out: Vec<u8>, value: &T) -> Result<Vec<u8>, CodecError> {
    postcard::to_extend(value, out).map_err(|_| CodecError::Malformed)
}

fn take<'a, T: serde::Deserialize<'a>>(rest: &mut &'a [u8]) -> Result<T, CodecError> {
    let (value, remaining) =
        postcard::take_from_bytes::<T>(rest).map_err(|_| CodecError::Malformed)?;
    *rest = remaining;
    Ok(value)
}

/// Reads a raw `u8` proof length, checks it against the bounds before any
/// body availability, then copies exactly that many bytes.
fn take_proof(rest: &mut &[u8]) -> Result<ResumeProof, CodecError> {
    let len: u8 = take(rest)?;
    let len = len as usize;
    if len < MIN_RESUME_PROOF_BYTES {
        return Err(CodecError::Malformed);
    }
    if len > MAX_RESUME_PROOF_BYTES {
        return Err(CodecError::LimitExceeded);
    }
    if rest.len() < len {
        return Err(CodecError::Malformed);
    }
    let bytes = rest[..len].to_vec();
    *rest = &rest[len..];
    ResumeProof::new(bytes)
}

/// Reads a raw `u8` lane count, then applies the L-rules inline as each entry
/// is read, so a truncated list fails at the first bad or missing entry.
fn take_lanes(rest: &mut &[u8]) -> Result<Vec<(u8, u64)>, CodecError> {
    let count: u8 = take(rest)?;
    let count = count as usize;
    if count == 0 {
        return Err(CodecError::Malformed);
    }
    if count > MAX_RESUME_LANES {
        return Err(CodecError::LimitExceeded);
    }
    let mut entries = Vec::new();
    let mut previous: Option<u8> = None;
    for _ in 0..count {
        let lane: u8 = take(rest)?;
        if lane != LANE_COMBAT {
            return Err(CodecError::UnknownMessage);
        }
        if let Some(prev) = previous {
            if lane <= prev {
                return Err(CodecError::Malformed);
            }
        }
        previous = Some(lane);
        let through: u64 = take(rest)?;
        if through > SEQ_LAST {
            return Err(CodecError::Malformed);
        }
        entries.push((lane, through));
    }
    Ok(entries)
}

/// Encodes one resume request to its canonical bytes.
pub fn encode_request(msg: &ResumeRequest) -> Result<Vec<u8>, CodecError> {
    msg.validate()?;
    let mut out = extend(Vec::new(), &RESUME_VERSION)?;
    out = extend(out, &LANE_SESSION)?;
    out = extend(out, &RESUME_TAG_REQUEST)?;
    out = extend(out, &msg.epoch)?;
    out = extend(out, &msg.fence.get())?;
    let proof = msg.proof.as_bytes();
    out = extend(out, &(proof.len() as u8))?;
    out.extend_from_slice(proof);
    out = extend(out, &(msg.claims.len() as u8))?;
    for claim in &msg.claims {
        out = extend(out, &claim.lane)?;
        out = extend(out, &claim.receipted_through)?;
    }
    if out.len() > MAX_RESUME_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    Ok(out)
}

/// Decodes one resume request from its bytes.
pub fn decode_request(bytes: &[u8]) -> Result<ResumeRequest, CodecError> {
    if bytes.len() > MAX_RESUME_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    if bytes.is_empty() {
        return Err(CodecError::Malformed);
    }
    let mut rest = bytes;
    let version: u8 = take(&mut rest)?;
    if version != RESUME_VERSION {
        return Err(CodecError::UnsupportedVersion);
    }
    let lane: u8 = take(&mut rest)?;
    if lane != LANE_SESSION {
        return Err(CodecError::WrongDirection);
    }
    let tag: u8 = take(&mut rest)?;
    match tag {
        RESUME_TAG_REQUEST => {
            let epoch: [u8; 16] = take(&mut rest)?;
            let fence_raw: u64 = take(&mut rest)?;
            let fence = ConnectionFence::new(fence_raw).ok_or(CodecError::Malformed)?;
            let proof = take_proof(&mut rest)?;
            let entries = take_lanes(&mut rest)?;
            let claims = entries
                .into_iter()
                .map(|(lane, receipted_through)| LaneClaim {
                    lane,
                    receipted_through,
                })
                .collect();
            if !rest.is_empty() {
                return Err(CodecError::Malformed);
            }
            Ok(ResumeRequest {
                epoch,
                fence,
                proof,
                claims,
            })
        }
        RESUME_TAG_TICKET | RESUME_TAG_ACCEPTED | RESUME_TAG_REJECTED => {
            Err(CodecError::WrongDirection)
        }
        _ => Err(CodecError::UnknownMessage),
    }
}

fn encode_result(result: &ResumeResult) -> Result<Vec<u8>, CodecError> {
    result.validate()?;
    match result {
        ResumeResult::Accepted(accepted) => {
            let mut out = extend(Vec::new(), &RESUME_VERSION)?;
            out = extend(out, &LANE_SESSION)?;
            out = extend(out, &RESUME_TAG_ACCEPTED)?;
            out = extend(out, &accepted.epoch)?;
            out = extend(out, &accepted.granted.get())?;
            out = extend(out, &(accepted.progress.len() as u8))?;
            for progress in &accepted.progress {
                out = extend(out, &progress.lane)?;
                out = extend(out, &progress.finalized_through)?;
            }
            if out.len() > MAX_RESUME_FRAME_BYTES {
                return Err(CodecError::FrameTooLarge);
            }
            Ok(out)
        }
        ResumeResult::Rejected(rejected) => {
            let mut out = extend(Vec::new(), &RESUME_VERSION)?;
            out = extend(out, &LANE_SESSION)?;
            out = extend(out, &RESUME_TAG_REJECTED)?;
            out = extend(out, &rejected.epoch)?;
            out = extend(out, &rejected.reason.as_u8())?;
            if out.len() > MAX_RESUME_FRAME_BYTES {
                return Err(CodecError::FrameTooLarge);
            }
            Ok(out)
        }
    }
}

/// Encodes one downstream resume frame to its canonical bytes.
pub fn encode_down(msg: &ResumeDown) -> Result<Vec<u8>, CodecError> {
    match msg {
        ResumeDown::Ticket(ticket) => {
            let mut out = extend(Vec::new(), &RESUME_VERSION)?;
            out = extend(out, &LANE_SESSION)?;
            out = extend(out, &RESUME_TAG_TICKET)?;
            out = extend(out, &ticket.epoch)?;
            let proof = ticket.proof.as_bytes();
            out = extend(out, &(proof.len() as u8))?;
            out.extend_from_slice(proof);
            if out.len() > MAX_RESUME_FRAME_BYTES {
                return Err(CodecError::FrameTooLarge);
            }
            Ok(out)
        }
        ResumeDown::Answer(result) => encode_result(result),
    }
}

/// Decodes one downstream resume frame from its bytes.
pub fn decode_down(bytes: &[u8]) -> Result<ResumeDown, CodecError> {
    if bytes.len() > MAX_RESUME_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    if bytes.is_empty() {
        return Err(CodecError::Malformed);
    }
    let mut rest = bytes;
    let version: u8 = take(&mut rest)?;
    if version != RESUME_VERSION {
        return Err(CodecError::UnsupportedVersion);
    }
    let lane: u8 = take(&mut rest)?;
    if lane != LANE_SESSION {
        return Err(CodecError::WrongDirection);
    }
    let tag: u8 = take(&mut rest)?;
    match tag {
        RESUME_TAG_TICKET => {
            let epoch: [u8; 16] = take(&mut rest)?;
            let proof = take_proof(&mut rest)?;
            if !rest.is_empty() {
                return Err(CodecError::Malformed);
            }
            Ok(ResumeDown::Ticket(ResumeTicket { epoch, proof }))
        }
        RESUME_TAG_ACCEPTED => {
            let epoch: [u8; 16] = take(&mut rest)?;
            let granted_raw: u64 = take(&mut rest)?;
            let granted = ConnectionFence::new(granted_raw).ok_or(CodecError::Malformed)?;
            if granted.get() < 2 {
                return Err(CodecError::Malformed);
            }
            let entries = take_lanes(&mut rest)?;
            let progress = entries
                .into_iter()
                .map(|(lane, finalized_through)| LaneProgress {
                    lane,
                    finalized_through,
                })
                .collect();
            if !rest.is_empty() {
                return Err(CodecError::Malformed);
            }
            Ok(ResumeDown::Answer(ResumeResult::Accepted(ResumeAccepted {
                epoch,
                granted,
                progress,
            })))
        }
        RESUME_TAG_REJECTED => {
            let epoch: [u8; 16] = take(&mut rest)?;
            let reason_raw: u8 = take(&mut rest)?;
            let reason = ResumeRejection::from_u8(reason_raw).ok_or(CodecError::UnknownMessage)?;
            if !rest.is_empty() {
                return Err(CodecError::Malformed);
            }
            Ok(ResumeDown::Answer(ResumeResult::Rejected(ResumeRejected {
                epoch,
                reason,
            })))
        }
        RESUME_TAG_REQUEST => Err(CodecError::WrongDirection),
        _ => Err(CodecError::UnknownMessage),
    }
}
