//! In-memory host checkpoint bytes (T022 §7; ADR-0022).
//!
//! Fresh sessions on restart (D10): a checkpoint carries the full
//! authoritative wrapper (world, sim journal and its sequence/ack state),
//! the retained capture journal plus its watermark, the protocol selection,
//! and the incarnation identity — never live bindings, credentials, lane
//! caches, delivery logs, entity maps, grants, failure budgets, or delivery
//! watermarks. A loaded host continues the authority and the capture
//! journal unchanged while every peer rebinds under a new, strictly greater
//! incarnation, so pre-restart command bytes fail as `SessionExpired`.
//!
//! The encoding is compact JSON in this exact field order:
//! `{"version":1,"incarnation":u64,"protocol":1|2,"server_tick":u64,`
//! `"history":{..},"captured":[..],"capture_acknowledged":u64}`.
//!
//! Input is capped at [`MAX_CHECKPOINT_BYTES`] *before* any JSON parsing
//! (the reader entry point reads at most cap + 1 bytes), so parser scratch is
//! bounded by this host budget; T020's retained-history caps bound only what
//! the wrapper keeps. Decoding rejects duplicate, unknown, and missing keys at
//! every level, then validates the capture journal against the wrapped
//! authority. Loading builds a temporary host and returns it only when every
//! check passes: there is no partial state on failure.
//!
//! Production persistence (filesystem, crash atomicity, compression) is a
//! later `crpg-persist` task plus a server adapter, not part of this module.

use std::cell::Cell;
use std::fmt;
use std::io::Read;

use serde::de::{DeserializeSeed, Error as DeError, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crpg_net::protocol::{LANE_COMBAT, SEQ_FIRST, SEQ_LAST};
use crpg_sim::{HistoryEnvelope, HistoryEvent, HistoryWorld};

use crate::capture::{
    record_len, valid_outcome, CapturedRecord, MAX_CAPTURE_BYTES, MAX_CAPTURE_RECORDS,
    MAX_CAPTURE_RECORD_BYTES, MAX_NEW_EVENTS_PER_COMMAND, MAX_OUTCOME_BYTES, MAX_RECORD_VIEWS,
    MAX_VIEW_HEALTH,
};
use crate::host::{Host, HostConfig, ProtocolSelection};

/// The only checkpoint version this implementation reads and writes.
pub const CHECKPOINT_VERSION: u32 = 1;
/// Maximum checkpoint bytes, checked before parsing.
pub const MAX_CHECKPOINT_BYTES: usize = 16 * 1024 * 1024;

/// Every checkpoint failure. `Display` is `<VariantName> at
/// host/checkpoint`; there is no source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointError {
    /// Input (or encoded output) exceeds [`MAX_CHECKPOINT_BYTES`].
    TooLarge {
        /// The observed length (cap + 1 for the reader entry point).
        len: usize,
    },
    /// Not a well-formed checkpoint, or the capture journal disagrees with
    /// the wrapped authority.
    Malformed,
    /// A version other than [`CHECKPOINT_VERSION`].
    UnsupportedVersion {
        /// The rejected version.
        version: u32,
    },
    /// The new incarnation is not strictly greater than the saved one.
    EpochReused,
    /// Save was attempted with staged ingress.
    PendingIngress,
    /// The wrapped `HistoryWorld` failed its own validation.
    AuthorityInvalid,
    /// The reader failed.
    Io,
}

impl fmt::Display for CheckpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::TooLarge { .. } => "TooLarge",
            Self::Malformed => "Malformed",
            Self::UnsupportedVersion { .. } => "UnsupportedVersion",
            Self::EpochReused => "EpochReused",
            Self::PendingIngress => "PendingIngress",
            Self::AuthorityInvalid => "AuthorityInvalid",
            Self::Io => "Io",
        };
        write!(f, "{name} at host/checkpoint")
    }
}

impl std::error::Error for CheckpointError {}

/// The canonical output shape, in field order.
#[derive(Serialize)]
struct CheckpointOut<'a> {
    version: u32,
    incarnation: u64,
    protocol: u8,
    server_tick: u64,
    history: &'a HistoryWorld,
    captured: &'a std::collections::VecDeque<CapturedRecord>,
    capture_acknowledged: u64,
}

/// Encodes a quiescent host (see [`Host::save_checkpoint`]).
pub(crate) fn encode(host: &Host) -> Result<Vec<u8>, CheckpointError> {
    if host.has_staged_ingress() {
        return Err(CheckpointError::PendingIngress);
    }
    let out = CheckpointOut {
        version: CHECKPOINT_VERSION,
        incarnation: host.incarnation(),
        protocol: host.protocol().as_u8(),
        server_tick: host.server_tick(),
        history: host.history(),
        captured: host.retained_captures(),
        capture_acknowledged: host.capture_acknowledged(),
    };
    let bytes = serde_json::to_vec(&out).map_err(|_| CheckpointError::Malformed)?;
    if bytes.len() > MAX_CHECKPOINT_BYTES {
        return Err(CheckpointError::TooLarge { len: bytes.len() });
    }
    Ok(bytes)
}

/// Loads a checkpoint into a new host under `new_incarnation`, which must be
/// strictly greater than the saved incarnation. All sessions start fresh.
pub fn load_checkpoint(
    bytes: &[u8],
    new_incarnation: u64,
    now_ms: u64,
) -> Result<Host, CheckpointError> {
    if bytes.len() > MAX_CHECKPOINT_BYTES {
        return Err(CheckpointError::TooLarge { len: bytes.len() });
    }
    let flags = Flags {
        version: Cell::new(None),
        history: Cell::new(false),
    };
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let parsed = CheckpointSeed(&flags)
        .deserialize(&mut deserializer)
        .and_then(|parsed| deserializer.end().map(|()| parsed));
    let parsed = match parsed {
        Ok(parsed) => parsed,
        Err(_) => {
            return Err(if let Some(version) = flags.version.get() {
                CheckpointError::UnsupportedVersion { version }
            } else if flags.history.get() {
                CheckpointError::AuthorityInvalid
            } else {
                CheckpointError::Malformed
            });
        }
    };
    let protocol = ProtocolSelection::from_u8(parsed.protocol).ok_or(CheckpointError::Malformed)?;
    if parsed.history.pending_len() != 0 {
        return Err(CheckpointError::AuthorityInvalid);
    }
    if parsed.server_tick != parsed.history.world().tick().get() {
        return Err(CheckpointError::Malformed);
    }
    validate_captures(&parsed)?;
    if new_incarnation <= parsed.incarnation {
        return Err(CheckpointError::EpochReused);
    }
    Ok(Host::restore(
        HostConfig {
            protocol,
            incarnation: new_incarnation,
        },
        parsed.history,
        parsed.captured,
        parsed.capture_acknowledged,
        now_ms,
    ))
}

/// [`load_checkpoint`] from a reader, enforcing the cap while reading: at
/// most `MAX_CHECKPOINT_BYTES + 1` bytes are read; excess is `TooLarge`,
/// a read failure is `Io`.
pub fn load_checkpoint_from_reader<R: Read>(
    reader: R,
    new_incarnation: u64,
    now_ms: u64,
) -> Result<Host, CheckpointError> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_CHECKPOINT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CheckpointError::Io)?;
    if bytes.len() > MAX_CHECKPOINT_BYTES {
        return Err(CheckpointError::TooLarge { len: bytes.len() });
    }
    load_checkpoint(&bytes, new_incarnation, now_ms)
}

// ---------------------------------------------------------------------------
// Validation against the wrapped authority.
// ---------------------------------------------------------------------------

/// Checks the retained capture journal: contiguous `(acknowledged, last]`
/// numbering, per-record bounds and vocabulary, and history ranges inside
/// the wrapper's issued sequences that exactly match the retained events.
fn validate_captures(parsed: &Parsed) -> Result<(), CheckpointError> {
    let malformed = Err(CheckpointError::Malformed);
    let last_history = parsed.history.last_sequence();
    let mut previous_end: Option<u64> = None;
    for (index, record) in parsed.captured.iter().enumerate() {
        let expected = parsed
            .capture_acknowledged
            .checked_add(index as u64 + 1)
            .ok_or(CheckpointError::Malformed)?;
        if record.capture_seq != expected || record.capture_seq == u64::MAX {
            return malformed;
        }
        if record.lane != LANE_COMBAT || !(SEQ_FIRST..=SEQ_LAST).contains(&record.seq) {
            return malformed;
        }
        if record.events.len() > MAX_NEW_EVENTS_PER_COMMAND
            || record.views.len() > MAX_RECORD_VIEWS
            || record
                .views
                .iter()
                .any(|view| view.health.len() > MAX_VIEW_HEALTH)
        {
            return malformed;
        }
        if record.history_start > record.history_end || record.history_end > last_history {
            return malformed;
        }
        if record.history_end - record.history_start != record.events.len() as u64 {
            return malformed;
        }
        let contiguous = record
            .events
            .iter()
            .enumerate()
            .all(|(offset, envelope)| envelope.seq == record.history_start + 1 + offset as u64);
        if !contiguous {
            return malformed;
        }
        if previous_end.is_some_and(|end| record.history_start < end) {
            return malformed;
        }
        previous_end = Some(record.history_end);
        if record.processed_tick > parsed.server_tick {
            return malformed;
        }
        if !outcome_matches(record) {
            return malformed;
        }
        let views_ascending = record
            .views
            .windows(2)
            .all(|pair| pair[0].epoch < pair[1].epoch);
        let health_ascending = record.views.iter().all(|view| {
            view.health
                .windows(2)
                .all(|pair| pair[0].entity < pair[1].entity)
        });
        if !views_ascending || !health_ascending {
            return malformed;
        }
    }
    Ok(())
}

/// The captured outcome agrees with the retained resolution: `Some` needs a
/// matching `ActionResolved` in the range with a valid bounded outcome,
/// `None` needs none.
fn outcome_matches(record: &CapturedRecord) -> bool {
    let resolved =
        record
            .events
            .iter()
            .find_map(|envelope: &HistoryEnvelope| match &envelope.payload {
                HistoryEvent::ActionResolved {
                    actor,
                    target,
                    ability,
                    outcome,
                    damage,
                } => Some((actor, target, ability, outcome, damage)),
                _ => None,
            });
    match (&record.outcome, resolved) {
        (None, None) => true,
        (Some(captured), Some((actor, target, ability, outcome, damage))) => {
            captured.outcome.len() <= MAX_OUTCOME_BYTES
                && valid_outcome(&captured.outcome)
                && captured.actor == *actor
                && captured.target == *target
                && captured.ability == *ability
                && captured.outcome == *outcome
                && captured.damage == *damage
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Streaming decode with failure attribution.
// ---------------------------------------------------------------------------

/// Which parse failure the opaque `serde_json` error came from.
struct Flags {
    version: Cell<Option<u32>>,
    history: Cell<bool>,
}

/// The decoded checkpoint, before validation.
struct Parsed {
    incarnation: u64,
    protocol: u8,
    server_tick: u64,
    history: HistoryWorld,
    captured: Vec<CapturedRecord>,
    capture_acknowledged: u64,
}

struct CheckpointSeed<'f>(&'f Flags);

impl<'de> DeserializeSeed<'de> for CheckpointSeed<'_> {
    type Value = Parsed;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Parsed, D::Error> {
        deserializer.deserialize_map(CheckpointVisitor(self.0))
    }
}

struct CheckpointVisitor<'f>(&'f Flags);

/// Stores `value` into `slot`, failing on a duplicate key.
fn once<T, E: DeError>(slot: &mut Option<T>, value: T, key: &'static str) -> Result<(), E> {
    if slot.is_some() {
        return Err(E::custom(format_args!(
            "duplicate checkpoint field `{key}`"
        )));
    }
    *slot = Some(value);
    Ok(())
}

impl<'de> Visitor<'de> for CheckpointVisitor<'_> {
    type Value = Parsed;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a host checkpoint object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Parsed, A::Error> {
        let flags = self.0;
        let mut version: Option<u32> = None;
        let mut incarnation: Option<u64> = None;
        let mut protocol: Option<u8> = None;
        let mut server_tick: Option<u64> = None;
        let mut history: Option<HistoryWorld> = None;
        let mut captured: Option<Vec<CapturedRecord>> = None;
        let mut capture_acknowledged: Option<u64> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "version" => {
                    let value: u32 = map.next_value()?;
                    once(&mut version, value, "version")?;
                    if value != CHECKPOINT_VERSION {
                        flags.version.set(Some(value));
                        return Err(A::Error::custom("unsupported checkpoint version"));
                    }
                }
                "incarnation" => once(&mut incarnation, map.next_value()?, "incarnation")?,
                "protocol" => once(&mut protocol, map.next_value()?, "protocol")?,
                "server_tick" => once(&mut server_tick, map.next_value()?, "server_tick")?,
                "history" => {
                    if history.is_some() {
                        return Err(A::Error::custom("duplicate checkpoint field `history`"));
                    }
                    history = Some(map.next_value_seed(HistorySeed(flags))?);
                }
                "captured" => {
                    if captured.is_some() {
                        return Err(A::Error::custom("duplicate checkpoint field `captured`"));
                    }
                    captured = Some(map.next_value_seed(CapturesSeed)?);
                }
                "capture_acknowledged" => once(
                    &mut capture_acknowledged,
                    map.next_value()?,
                    "capture_acknowledged",
                )?,
                _ => return Err(A::Error::custom("unknown checkpoint field")),
            }
        }
        let missing = |key: &'static str| A::Error::custom(format_args!("missing `{key}`"));
        version.ok_or_else(|| missing("version"))?;
        Ok(Parsed {
            incarnation: incarnation.ok_or_else(|| missing("incarnation"))?,
            protocol: protocol.ok_or_else(|| missing("protocol"))?,
            server_tick: server_tick.ok_or_else(|| missing("server_tick"))?,
            history: history.ok_or_else(|| missing("history"))?,
            captured: captured.ok_or_else(|| missing("captured"))?,
            capture_acknowledged: capture_acknowledged
                .ok_or_else(|| missing("capture_acknowledged"))?,
        })
    }
}

/// Decodes the wrapped authority through its own validated `Deserialize`,
/// attributing any failure to it.
struct HistorySeed<'f>(&'f Flags);

impl<'de> DeserializeSeed<'de> for HistorySeed<'_> {
    type Value = HistoryWorld;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<HistoryWorld, D::Error> {
        HistoryWorld::deserialize(deserializer).inspect_err(|_| self.0.history.set(true))
    }
}

/// Decodes the capture list with the count bound checked before decoding an
/// excess element and the per-record and total byte bounds checked as each
/// record is retained.
struct CapturesSeed;

impl<'de> DeserializeSeed<'de> for CapturesSeed {
    type Value = Vec<CapturedRecord>;

    fn deserialize<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Vec<CapturedRecord>, D::Error> {
        deserializer.deserialize_seq(CapturesVisitor)
    }
}

struct CapturesVisitor;

impl<'de> Visitor<'de> for CapturesVisitor {
    type Value = Vec<CapturedRecord>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded capture record list")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<CapturedRecord>, A::Error> {
        let mut records = Vec::new();
        let mut total: usize = 0;
        loop {
            if records.len() == MAX_CAPTURE_RECORDS {
                if seq.next_element::<IgnoredAny>()?.is_some() {
                    return Err(A::Error::custom("too many capture records"));
                }
                break;
            }
            let Some(record) = seq.next_element::<CapturedRecord>()? else {
                break;
            };
            let len = record_len(&record);
            if len > MAX_CAPTURE_RECORD_BYTES {
                return Err(A::Error::custom("capture record exceeds its byte bound"));
            }
            total += len;
            if total > MAX_CAPTURE_BYTES {
                return Err(A::Error::custom("capture journal exceeds its byte bound"));
            }
            records.push(record);
        }
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_display_variant_and_location_only() {
        assert_eq!(
            CheckpointError::TooLarge { len: 1 }.to_string(),
            "TooLarge at host/checkpoint"
        );
        assert_eq!(
            CheckpointError::UnsupportedVersion { version: 2 }.to_string(),
            "UnsupportedVersion at host/checkpoint"
        );
        assert_eq!(CheckpointError::Io.to_string(), "Io at host/checkpoint");
        assert!(std::error::Error::source(&CheckpointError::Malformed).is_none());
    }
}
