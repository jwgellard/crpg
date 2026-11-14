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
//! The encoding is compact JSON in this exact field order. Version 1 (written
//! whenever no host record is retained, so every turn-based save is exactly
//! what it always was):
//! `{"version":1,"incarnation":u64,"protocol":1|2,"server_tick":u64,`
//! `"history":{..},"captured":[..],"capture_acknowledged":u64}`.
//! Version 2 (T141s, written when at least one [`HostRecord`] is retained)
//! adds `"host_records":[..]` after `"captured"`: the retained command
//! records and host records, each list ascending by `capture_seq`. The loader
//! reads both: a version-1 checkpoint is a version-2 checkpoint with no host
//! record, so nothing is migrated.
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
    host_record_len, record_len, valid_outcome, CaptureEntry, CapturedRecord, HostRecord,
    MAX_CAPTURE_BYTES, MAX_CAPTURE_RECORDS, MAX_CAPTURE_RECORD_BYTES, MAX_HOST_RECORD_BYTES,
    MAX_HOST_RECORD_EVENTS, MAX_NEW_EVENTS_PER_COMMAND_RT, MAX_OUTCOME_BYTES, MAX_RECORD_VIEWS,
    MAX_VIEW_HEALTH,
};
use crate::host::{Host, HostConfig, ProtocolSelection};

/// The checkpoint version written when no host record is retained (the
/// original shape, still read).
pub const CHECKPOINT_VERSION: u32 = 1;
/// The checkpoint version written when at least one host record is retained.
pub const CHECKPOINT_VERSION_HOST_RECORDS: u32 = 2;
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
    /// A version other than [`CHECKPOINT_VERSION`] or
    /// [`CHECKPOINT_VERSION_HOST_RECORDS`].
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

/// The canonical version-1 output shape, in field order.
#[derive(Serialize)]
struct CheckpointOut<'a> {
    version: u32,
    incarnation: u64,
    protocol: u8,
    server_tick: u64,
    history: &'a HistoryWorld,
    captured: Vec<&'a CapturedRecord>,
    capture_acknowledged: u64,
}

/// The canonical version-2 output shape, in field order.
#[derive(Serialize)]
struct CheckpointOutHostRecords<'a> {
    version: u32,
    incarnation: u64,
    protocol: u8,
    server_tick: u64,
    history: &'a HistoryWorld,
    captured: Vec<&'a CapturedRecord>,
    host_records: Vec<&'a HostRecord>,
    capture_acknowledged: u64,
}

/// Encodes a quiescent host (see [`Host::save_checkpoint`]).
pub(crate) fn encode(host: &Host) -> Result<Vec<u8>, CheckpointError> {
    if host.has_staged_ingress() {
        return Err(CheckpointError::PendingIngress);
    }
    let mut captured: Vec<&CapturedRecord> = Vec::new();
    let mut host_records: Vec<&HostRecord> = Vec::new();
    for entry in host.retained_captures() {
        match entry {
            CaptureEntry::Command(record) => captured.push(record),
            CaptureEntry::Host(record) => host_records.push(record),
        }
    }
    // Version 1 unless a host record is retained: a host that never made one
    // writes today's bytes.
    let bytes = if host_records.is_empty() {
        serde_json::to_vec(&CheckpointOut {
            version: CHECKPOINT_VERSION,
            incarnation: host.incarnation(),
            protocol: host.protocol().as_u8(),
            server_tick: host.server_tick(),
            history: host.history(),
            captured,
            capture_acknowledged: host.capture_acknowledged(),
        })
    } else {
        serde_json::to_vec(&CheckpointOutHostRecords {
            version: CHECKPOINT_VERSION_HOST_RECORDS,
            incarnation: host.incarnation(),
            protocol: host.protocol().as_u8(),
            server_tick: host.server_tick(),
            history: host.history(),
            captured,
            host_records,
            capture_acknowledged: host.capture_acknowledged(),
        })
    }
    .map_err(|_| CheckpointError::Malformed)?;
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
    let entries = validate_captures(&parsed)?;
    if new_incarnation <= parsed.incarnation {
        return Err(CheckpointError::EpochReused);
    }
    Ok(Host::restore(
        HostConfig {
            protocol,
            incarnation: new_incarnation,
        },
        parsed.history,
        entries,
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

/// Checks the retained capture journal and returns it merged by
/// `capture_seq`: the two lists together are exactly `(acknowledged, last]`
/// with no duplicate and no gap, each list ascending; per-record bounds and
/// vocabulary; history ranges inside the wrapper's issued sequences that
/// exactly match the retained events; and the entry count and byte total.
/// Adjacent entries' history ranges are contiguous: each starts where the
/// previous one ended, because after the wrap every history envelope comes
/// from a captured command or a host record and a gap would mean omitted
/// evidence.
fn validate_captures(parsed: &Parsed) -> Result<Vec<CaptureEntry>, CheckpointError> {
    let malformed = Err(CheckpointError::Malformed);
    let last_history = parsed.history.last_sequence();
    let total = parsed.captured.len() + parsed.host_records.len();
    let mut commands = parsed.captured.iter().peekable();
    let mut hosts = parsed.host_records.iter().peekable();
    let mut entries: Vec<CaptureEntry> = Vec::with_capacity(total);
    let mut bytes: usize = 0;
    let mut previous_end: Option<u64> = None;
    for index in 0..total {
        let expected = parsed
            .capture_acknowledged
            .checked_add(index as u64 + 1)
            .ok_or(CheckpointError::Malformed)?;
        if expected == u64::MAX {
            return malformed;
        }
        let command_next = commands.peek().is_some_and(|r| r.capture_seq == expected);
        let host_next = hosts.peek().is_some_and(|r| r.capture_seq == expected);
        // Neither list holds the expected number (a gap, or a list out of
        // order), or both do (a duplicate).
        if command_next == host_next {
            return malformed;
        }
        let (start, end) = if command_next {
            let record = commands.next().expect("peeked");
            check_command_record(record, parsed.server_tick, last_history)?;
            bytes = bytes.saturating_add(record_len(record));
            entries.push(CaptureEntry::Command(record.clone()));
            (record.history_start, record.history_end)
        } else {
            let record = hosts.next().expect("peeked");
            check_host_record(
                record,
                parsed.server_tick,
                last_history,
                MAX_HOST_RECORD_EVENTS,
                MAX_HOST_RECORD_BYTES,
            )?;
            bytes = bytes.saturating_add(host_record_len(record));
            entries.push(CaptureEntry::Host(record.clone()));
            (record.history_start, record.history_end)
        };
        if previous_end.is_some_and(|previous| start != previous) {
            return malformed;
        }
        previous_end = Some(end);
    }
    check_journal_totals(total, bytes, MAX_CAPTURE_RECORDS, MAX_CAPTURE_BYTES)?;
    // The restored host's `last_capture` is `capture_acknowledged +
    // entries.len()`; bind it to the largest value a running host can reach
    // (`u64::MAX - 1`, see `accepted_room`), so a checkpoint can never
    // install the reserved `u64::MAX` counter.
    let watermark = parsed
        .capture_acknowledged
        .checked_add(total as u64)
        .ok_or(CheckpointError::Malformed)?;
    if watermark > u64::MAX - 1 {
        return Err(CheckpointError::Malformed);
    }
    Ok(entries)
}

/// One command record's own bounds, vocabulary and range (numbering and
/// adjacency are the caller's).
fn check_command_record(
    record: &CapturedRecord,
    server_tick: u64,
    last_history: u64,
) -> Result<(), CheckpointError> {
    let malformed = Err(CheckpointError::Malformed);
    if record.lane != LANE_COMBAT || !(SEQ_FIRST..=SEQ_LAST).contains(&record.seq) {
        return malformed;
    }
    if record.events.len() > MAX_NEW_EVENTS_PER_COMMAND_RT
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
    if !contiguous(record.history_start, &record.events) {
        return malformed;
    }
    if record.processed_tick > server_tick {
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
    Ok(())
}

/// Whether `events` carry sequences `start + 1`, `start + 2`, ….
fn contiguous(start: u64, events: &[HistoryEnvelope]) -> bool {
    events
        .iter()
        .enumerate()
        .all(|(offset, envelope)| envelope.seq == start + 1 + offset as u64)
}

/// One host record's own bounds (T141s S-6(b)): a non-empty range inside the
/// issued sequences whose width is exactly the event count, at most
/// `max_events`; envelopes numbered contiguously from `history_start + 1`;
/// `processed_tick` no later than `server_tick`; compact JSON length at most
/// `max_bytes`. The limits are arguments so a unit test can reach each one.
pub(crate) fn check_host_record(
    record: &HostRecord,
    server_tick: u64,
    last_history: u64,
    max_events: usize,
    max_bytes: usize,
) -> Result<(), CheckpointError> {
    let malformed = Err(CheckpointError::Malformed);
    if record.history_start >= record.history_end || record.history_end > last_history {
        return malformed;
    }
    if record.events.len() > max_events
        || record.history_end - record.history_start != record.events.len() as u64
    {
        return malformed;
    }
    if !contiguous(record.history_start, &record.events) {
        return malformed;
    }
    if record.processed_tick > server_tick {
        return malformed;
    }
    if host_record_len(record) > max_bytes {
        return malformed;
    }
    Ok(())
}

/// The journal's entry count and byte total against the limits (T141s
/// S-6(e)); the limits are arguments so a unit test can reach them.
pub(crate) fn check_journal_totals(
    entries: usize,
    bytes: usize,
    max_entries: usize,
    max_bytes: usize,
) -> Result<(), CheckpointError> {
    if entries > max_entries || bytes > max_bytes {
        Err(CheckpointError::Malformed)
    } else {
        Ok(())
    }
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
    /// Empty for version 1.
    host_records: Vec<HostRecord>,
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
        let mut host_records: Option<Vec<HostRecord>> = None;
        let mut capture_acknowledged: Option<u64> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "version" => {
                    let value: u32 = map.next_value()?;
                    once(&mut version, value, "version")?;
                    if value != CHECKPOINT_VERSION && value != CHECKPOINT_VERSION_HOST_RECORDS {
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
                "host_records" => {
                    if host_records.is_some() {
                        return Err(A::Error::custom(
                            "duplicate checkpoint field `host_records`",
                        ));
                    }
                    host_records = Some(map.next_value_seed(HostRecordsSeed)?);
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
        let version = version.ok_or_else(|| missing("version"))?;
        // Version 1 has no `host_records` key and version 2 requires one (an
        // empty list is fine): the existing unknown/missing-key rule.
        let host_records = match (version, host_records) {
            (CHECKPOINT_VERSION, None) => Vec::new(),
            (CHECKPOINT_VERSION_HOST_RECORDS, Some(records)) => records,
            (CHECKPOINT_VERSION, Some(_)) => {
                return Err(A::Error::custom("unknown checkpoint field `host_records`"))
            }
            _ => return Err(missing("host_records")),
        };
        Ok(Parsed {
            incarnation: incarnation.ok_or_else(|| missing("incarnation"))?,
            protocol: protocol.ok_or_else(|| missing("protocol"))?,
            server_tick: server_tick.ok_or_else(|| missing("server_tick"))?,
            history: history.ok_or_else(|| missing("history"))?,
            captured: captured.ok_or_else(|| missing("captured"))?,
            host_records,
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

/// Decodes the host-record list with the count bound checked before
/// decoding an excess element and the per-record byte bound checked as each
/// record is retained; the journal's combined bounds are
/// [`check_journal_totals`]'s.
struct HostRecordsSeed;

impl<'de> DeserializeSeed<'de> for HostRecordsSeed {
    type Value = Vec<HostRecord>;

    fn deserialize<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Vec<HostRecord>, D::Error> {
        deserializer.deserialize_seq(HostRecordsVisitor)
    }
}

struct HostRecordsVisitor;

impl<'de> Visitor<'de> for HostRecordsVisitor {
    type Value = Vec<HostRecord>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded host record list")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<HostRecord>, A::Error> {
        let mut records = Vec::new();
        loop {
            if records.len() == MAX_CAPTURE_RECORDS {
                if seq.next_element::<IgnoredAny>()?.is_some() {
                    return Err(A::Error::custom("too many host records"));
                }
                break;
            }
            let Some(record) = seq.next_element::<HostRecord>()? else {
                break;
            };
            if host_record_len(&record) > MAX_HOST_RECORD_BYTES {
                return Err(A::Error::custom("host record exceeds its byte bound"));
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

    // -- T141s loader bounds (S8) ---------------------------------------------

    fn envelope(seq: u64) -> HistoryEnvelope {
        serde_json::from_value(serde_json::json!({
            "tick": 0,
            "seq": seq,
            "payload": {"type": "round_started", "value": {"round": 1}}
        }))
        .expect("envelope parses")
    }

    /// A host record over `(10, 10 + count]` at processed tick 5.
    fn record(count: u64) -> HostRecord {
        HostRecord {
            capture_seq: 2,
            op: crate::capture::HostOp::Tick,
            history_start: 10,
            history_end: 10 + count,
            processed_tick: 5,
            events: (11..=10 + count).map(envelope).collect(),
        }
    }

    #[test]
    fn loader_bounds() {
        // Events: the limit is inclusive.
        assert_eq!(check_host_record(&record(3), 5, 100, 3, usize::MAX), Ok(()));
        assert_eq!(
            check_host_record(&record(4), 5, 100, 3, usize::MAX),
            Err(CheckpointError::Malformed)
        );
        // Bytes: the limit is inclusive.
        let len = host_record_len(&record(3));
        assert_eq!(check_host_record(&record(3), 5, 100, 3, len), Ok(()));
        assert_eq!(
            check_host_record(&record(3), 5, 100, 3, len - 1),
            Err(CheckpointError::Malformed)
        );
        // Journal totals: both limits are inclusive.
        let bytes = 123_456;
        assert_eq!(check_journal_totals(4096, bytes, 4096, bytes), Ok(()));
        assert_eq!(
            check_journal_totals(4097, bytes, 4096, bytes),
            Err(CheckpointError::Malformed)
        );
        assert_eq!(
            check_journal_totals(4096, bytes + 1, 4096, bytes),
            Err(CheckpointError::Malformed)
        );
    }

    #[test]
    fn host_record_shape_rules() {
        let ok = record(2);
        assert_eq!(
            check_host_record(&ok, 5, 12, MAX_HOST_RECORD_EVENTS, MAX_HOST_RECORD_BYTES),
            Ok(())
        );
        let check = |r: &HostRecord, tick: u64, last: u64| {
            check_host_record(r, tick, last, MAX_HOST_RECORD_EVENTS, MAX_HOST_RECORD_BYTES)
        };
        // processed_tick: equal to the server tick is fine, past it is not.
        assert_eq!(check(&ok, 4, 12), Err(CheckpointError::Malformed));
        // history_end past the issued sequences.
        assert_eq!(check(&ok, 5, 11), Err(CheckpointError::Malformed));
        // An empty range, and a reversed one.
        let mut empty = record(0);
        assert_eq!(check(&empty, 5, 12), Err(CheckpointError::Malformed));
        empty.history_start = 11;
        empty.history_end = 10;
        assert_eq!(check(&empty, 5, 12), Err(CheckpointError::Malformed));
        // A width that disagrees with the event count, either way.
        let mut wide = record(2);
        wide.history_end = 13;
        assert_eq!(check(&wide, 5, 13), Err(CheckpointError::Malformed));
        let mut narrow = record(2);
        narrow.history_end = 11;
        assert_eq!(check(&narrow, 5, 13), Err(CheckpointError::Malformed));
        // Envelope sequences not starting at history_start + 1, or gapped.
        let mut shifted = record(2);
        shifted.events = vec![envelope(10), envelope(11)];
        assert_eq!(check(&shifted, 5, 12), Err(CheckpointError::Malformed));
        let mut gapped = record(2);
        gapped.events = vec![envelope(11), envelope(13)];
        assert_eq!(check(&gapped, 5, 13), Err(CheckpointError::Malformed));
    }

    #[test]
    fn host_record_list_count_is_bounded_before_decoding_the_excess() {
        let element = serde_json::to_string(&record(1)).expect("encodes");
        let list = |count: usize| format!("[{}]", vec![element.as_str(); count].join(","));
        let decode =
            |text: &str| HostRecordsSeed.deserialize(&mut serde_json::Deserializer::from_str(text));
        assert_eq!(
            decode(&list(MAX_CAPTURE_RECORDS))
                .expect("at the bound")
                .len(),
            MAX_CAPTURE_RECORDS
        );
        assert!(decode(&list(MAX_CAPTURE_RECORDS + 1)).is_err());
        // An empty list is fine.
        assert_eq!(decode("[]").expect("empty").len(), 0);
    }
}
