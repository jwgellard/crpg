//! Bounded capture journal shapes (T022 §1, §4, §6; ADR-0022).
//!
//! One [`CapturedRecord`] is appended for every accepted command, after
//! `HistoryWorld::perform_action` returns and before the host acknowledges
//! the sim journal or starts the next staged command. A record keeps the
//! symbolic fields of the returned `ActionOutcome` (never the roll, margin,
//! or kill flag), the exact history range the command produced together with
//! a clone of those envelopes, and the permitted post-state of every bound
//! peer that received content for the command. Records are trusted capture
//! evidence: they never enter wire bytes.
//!
//! Records are numbered by a monotonic `capture_seq` from 1 and retire only
//! through the trusted consumer's `Host::acknowledge_captures`; the bounds
//! below are enforced by reservation before execution (§6) and again when a
//! checkpoint is loaded (§7).
//!
//! Serialization is compact `serde_json` in declaration order; that byte
//! length is the unit of the capture ledger's byte accounting. Decoding
//! rejects duplicate, unknown, and missing fields, including `Option`
//! fields, which are required-but-nullable. `NetId` fields use a
//! server-local checked nonzero `u64` adapter rather than a net serde API.

use serde::{Deserialize, Deserializer, Serialize};

use crpg_core::{EntityId, Ulid};
use crpg_net::protocol::NetId;
use crpg_sim::HistoryEnvelope;

/// Maximum retained capture records.
pub const MAX_CAPTURE_RECORDS: usize = 4096;
/// Maximum retained capture bytes (compact `serde_json` record lengths).
pub const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;
/// Maximum compact `serde_json` length of one record, retained history and
/// per-peer post-state included; the worst-case shape is measured by test.
pub const MAX_CAPTURE_RECORD_BYTES: usize = 16 * 1024;
/// Maximum records returned by one `Host::read_captures` page.
pub const MAX_CAPTURE_PAGE: usize = 256;
/// Maximum history envelopes one command produces:
/// `ActionResolved` + `Died` + `TurnStarted` | `EncounterEnded`.
pub const MAX_NEW_EVENTS_PER_COMMAND: usize = 3;
/// Maximum delivery ops per command per peer: receipt + events + state ops.
pub const MAX_DELIVERY_OPS_PER_COMMAND: usize = 8;
/// Maximum encoded delivery frame bytes per command per peer; the
/// worst-case frame is measured by test.
pub const MAX_DELIVERY_BYTES_PER_COMMAND_PEER: usize = 2048;

/// Maximum health entries in one [`CapturedView`]: actor, target, and the
/// incoming active head (T022 §4/§7).
pub(crate) const MAX_VIEW_HEALTH: usize = 3;
/// Maximum views in one record: one per bound peer.
pub(crate) const MAX_RECORD_VIEWS: usize = 8;
/// Maximum outcome text bytes (the T020/T021 string bound).
pub(crate) const MAX_OUTCOME_BYTES: usize = 256;

/// The symbolic fields of one returned `ActionOutcome`.
///
/// `roll`, `margin`, and `target_died` are never stored anywhere; `outcome`
/// is T020's symbolic text for the same operation and `damage` is the
/// reported, possibly overkill, flat amount.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedOutcome {
    /// The acting combatant.
    pub actor: EntityId,
    /// The targeted combatant.
    pub target: EntityId,
    /// The ability that was used.
    pub ability: Ulid,
    /// `critical_success`, `success`, `failure`, `critical_failure`, or
    /// `custom:<0..255>` without leading zeros.
    pub outcome: String,
    /// The reported flat damage.
    pub damage: u32,
}

/// One involved entity's permitted health state for one peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedHealth {
    /// The peer's replica id for the entity.
    #[serde(with = "net_id")]
    pub entity: NetId,
    /// Current health.
    pub health: u32,
    /// Maximum health.
    pub max_health: u32,
    /// Terminal flag.
    pub dead: bool,
}

/// Permitted turn state for one peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedTurn {
    /// The active head when disclosed to the peer, else `None`.
    #[serde(with = "opt_net_id")]
    pub active: Option<NetId>,
    /// The combat round widened from the persisted counter.
    pub round: u64,
}

/// One peer's permitted post-state for one accepted command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedView {
    /// The peer's session epoch.
    pub epoch: [u8; 16],
    /// At most three entries, ascending `NetId`.
    pub health: Vec<CapturedHealth>,
    /// Present only when the peer holds the `turn_state` grant.
    #[serde(deserialize_with = "required_option")]
    pub turn: Option<CapturedTurn>,
}

/// One accepted command's captured evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedRecord {
    /// Monotonic capture number from 1.
    pub capture_seq: u64,
    /// The originating session epoch.
    pub epoch: [u8; 16],
    /// Always `LANE_COMBAT`.
    pub lane: u8,
    /// The originating command sequence.
    pub seq: u64,
    /// Exclusive start of the history range: events are
    /// `(history_start, history_end]`.
    pub history_start: u64,
    /// Inclusive end of the history range.
    pub history_end: u64,
    /// The returned outcome; `None` for an accepted `EndTurn`.
    #[serde(deserialize_with = "required_option")]
    pub outcome: Option<CapturedOutcome>,
    /// The authoritative tick the command finalized at.
    pub processed_tick: u64,
    /// The exact history range, cloned before the sim acknowledgement.
    pub events: Vec<HistoryEnvelope>,
    /// At most eight views, ascending epoch bytes; trusted capture only.
    pub views: Vec<CapturedView>,
}

/// The compact `serde_json` length of one record: the capture ledger unit.
pub(crate) fn record_len(record: &CapturedRecord) -> usize {
    serde_json::to_vec(record)
        .expect("capture records serialize")
        .len()
}

/// Reports whether `text` is T020's exact symbolic outcome vocabulary: the
/// four fixed words, or `custom:<n>` with decimal `n` in `0..=255` and no
/// leading zeros.
pub(crate) fn valid_outcome(text: &str) -> bool {
    match text {
        "critical_success" | "success" | "failure" | "critical_failure" => true,
        _ => {
            let Some(rest) = text.strip_prefix("custom:") else {
                return false;
            };
            if rest.is_empty() || rest.len() > 3 || !rest.bytes().all(|b| b.is_ascii_digit()) {
                return false;
            }
            if rest.len() > 1 && rest.starts_with('0') {
                return false;
            }
            rest.parse::<u32>().is_ok_and(|n| n <= 255)
        }
    }
}

/// Deserializes an `Option` field that must be present (possibly `null`).
///
/// Using `deserialize_with` makes serde treat a missing key as an error
/// instead of silently defaulting to `None`.
fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

/// Checked nonzero `u64` serde adapter for [`NetId`].
mod net_id {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    use crpg_net::protocol::NetId;

    /// Writes the raw nonzero id.
    pub fn serialize<S: Serializer>(id: &NetId, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(id.get())
    }

    /// Reads a raw id, refusing the never-valid zero.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<NetId, D::Error> {
        let raw = u64::deserialize(deserializer)?;
        NetId::new(raw).ok_or_else(|| D::Error::custom("zero NetId"))
    }
}

/// Checked nonzero `u64` serde adapter for `Option<NetId>`; the key is
/// required (a missing key is an error, `null` is `None`).
mod opt_net_id {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    use crpg_net::protocol::NetId;

    /// Writes the raw nonzero id or `null`.
    pub fn serialize<S: Serializer>(id: &Option<NetId>, serializer: S) -> Result<S::Ok, S::Error> {
        match id {
            Some(id) => serializer.serialize_some(&id.get()),
            None => serializer.serialize_none(),
        }
    }

    /// Reads a raw id or `null`, refusing the never-valid zero.
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<NetId>, D::Error> {
        match Option::<u64>::deserialize(deserializer)? {
            None => Ok(None),
            Some(raw) => NetId::new(raw)
                .map(Some)
                .ok_or_else(|| D::Error::custom("zero NetId")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(raw: u64) -> NetId {
        NetId::new(raw).expect("nonzero")
    }

    fn sample() -> CapturedRecord {
        CapturedRecord {
            capture_seq: 1,
            epoch: [1; 16],
            lane: 0,
            seq: 1,
            history_start: 0,
            history_end: 0,
            outcome: None,
            processed_tick: 0,
            events: Vec::new(),
            views: vec![CapturedView {
                epoch: [1; 16],
                health: vec![CapturedHealth {
                    entity: net(1),
                    health: 1,
                    max_health: 2,
                    dead: false,
                }],
                turn: Some(CapturedTurn {
                    active: None,
                    round: 0,
                }),
            }],
        }
    }

    #[test]
    fn record_round_trips_in_declaration_order() {
        let record = sample();
        let text = serde_json::to_string(&record).expect("encodes");
        assert!(text.starts_with("{\"capture_seq\":1,\"epoch\":["));
        let back: CapturedRecord = serde_json::from_str(&text).expect("decodes");
        assert_eq!(back, record);
        assert_eq!(record_len(&record), text.len());
    }

    #[test]
    fn decode_rejects_missing_unknown_duplicate_and_zero() {
        let text = serde_json::to_string(&sample()).expect("encodes");
        // Missing required-but-nullable `outcome`.
        let missing = text.replace("\"outcome\":null,", "");
        assert!(serde_json::from_str::<CapturedRecord>(&missing).is_err());
        // Missing required-but-nullable `turn` / `active`.
        let missing_turn = text.replace(",\"turn\":{\"active\":null,\"round\":0}", "");
        assert_ne!(missing_turn, text);
        assert!(serde_json::from_str::<CapturedRecord>(&missing_turn).is_err());
        let missing_active = text.replace("\"active\":null,", "");
        assert_ne!(missing_active, text);
        assert!(serde_json::from_str::<CapturedRecord>(&missing_active).is_err());
        // Unknown field.
        let unknown = text.replacen("{\"capture_seq\":1,", "{\"capture_seq\":1,\"x\":0,", 1);
        assert!(serde_json::from_str::<CapturedRecord>(&unknown).is_err());
        // Duplicate field.
        let duplicate = text.replacen(
            "{\"capture_seq\":1,",
            "{\"capture_seq\":1,\"capture_seq\":1,",
            1,
        );
        assert!(serde_json::from_str::<CapturedRecord>(&duplicate).is_err());
        // Zero NetId.
        let zero = text.replace("\"entity\":1,", "\"entity\":0,");
        assert_ne!(zero, text);
        assert!(serde_json::from_str::<CapturedRecord>(&zero).is_err());
    }

    #[test]
    fn outcome_vocabulary_is_exact() {
        for good in [
            "critical_success",
            "success",
            "failure",
            "critical_failure",
            "custom:0",
            "custom:9",
            "custom:255",
        ] {
            assert!(valid_outcome(good), "{good}");
        }
        for bad in [
            "",
            "Success",
            "custom:",
            "custom:00",
            "custom:01",
            "custom:256",
            "custom:1000",
            "custom:-1",
            "custom:+1",
        ] {
            assert!(!valid_outcome(bad), "{bad}");
        }
    }
}
