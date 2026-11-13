//! Explicit v3 wire vocabulary: encounter facts, events, intents and the
//! host-fact ops (T140a, ADR-0029 N1–N4).
//!
//! v1 ([`crate::protocol`]) and v2 ([`crate::protocol_v2`]) are frozen: their
//! version bytes, tags, codes and fixtures are unchanged by this module. v3
//! is a new, explicitly selected version (first byte 3). Its codec
//! ([`crate::codec_v3`]) refuses every other version byte with
//! [`CodecError::UnsupportedVersion`](crate::codec::CodecError), and the v1
//! and v2 codecs refuse byte 3 the same way. There is no negotiation,
//! autodetection or downgrade.
//!
//! Tags 0..=10 are re-declared natively here, never wrapped in the v2
//! `Legacy` variant: the receipt's code byte must reach the new codes
//! 21..=30, and the v1 [`crate::protocol::RejectionCode`] is frozen at
//! 1..=20. Payload bytes of tags 0..=10 are identical to v2. Tags 11..=13
//! (T140b) are the fact ops: [`DeltaOp::Encounter`], [`DeltaOp::Timeline`]
//! and [`DeltaOp::EntityFacts`], each carrying the viewer's complete image of
//! its subject, with the wire bounds and shape checks defined below.
//!
//! A withheld value is `None` on the wire (a `0` flag with no further bytes),
//! never a zero. A disclosed nullable value is a nested `Option`. `Clock` is
//! the only carrier of pause state; it carries no tick, because every frame
//! header already holds the server tick.
//!
//! Nothing here reads `World`, mints replica ids, or authorises anything.

use std::collections::BTreeSet;

use crpg_core::Ulid;

use crate::codec::CodecError;

pub use crate::protocol::{NetId, SessionEpoch};

// ---------------------------------------------------------------------------
// Version, tags and policy.
// ---------------------------------------------------------------------------

/// Wire protocol version for explicitly selected v3 frames.
pub const PROTOCOL_VERSION: u8 = 3;

/// [`IntentBody::DeclareAction`] tag (same meaning and payload as v1).
pub const INTENT_TAG_DECLARE_ACTION: u8 = 0;
/// [`IntentBody::EndTurn`] tag (same meaning and payload as v1).
pub const INTENT_TAG_END_TURN: u8 = 1;
/// [`IntentBody::Stop`] tag.
pub const INTENT_TAG_STOP: u8 = 2;
/// [`IntentBody::Pause`] tag.
pub const INTENT_TAG_PAUSE: u8 = 3;
/// [`IntentBody::Unpause`] tag.
pub const INTENT_TAG_UNPAUSE: u8 = 4;
/// [`IntentBody::RequestMode`] tag.
pub const INTENT_TAG_REQUEST_MODE: u8 = 5;

// Delta tags 0..=7 are `crate::protocol::DELTA_TAG_*` and 8..=10 are
// `crate::protocol_v2::DELTA_TAG_*`; they are used by path, never copied.

/// [`DeltaOp::Encounter`] tag.
pub const DELTA_TAG_ENCOUNTER: u8 = 11;
/// [`DeltaOp::Timeline`] tag.
pub const DELTA_TAG_TIMELINE: u8 = 12;
/// [`DeltaOp::EntityFacts`] tag.
pub const DELTA_TAG_ENTITY_FACTS: u8 = 13;
/// [`DeltaOp::RoundStarted`] tag.
pub const DELTA_TAG_ROUND_STARTED: u8 = 14;
/// [`DeltaOp::ModeChanged`] tag.
pub const DELTA_TAG_MODE_CHANGED: u8 = 15;
/// [`DeltaOp::OrderEnded`] tag.
pub const DELTA_TAG_ORDER_ENDED: u8 = 16;
/// [`DeltaOp::Joined`] tag.
pub const DELTA_TAG_JOINED: u8 = 17;
/// [`DeltaOp::Left`] tag.
pub const DELTA_TAG_LEFT: u8 = 18;
/// [`DeltaOp::Clock`] tag.
pub const DELTA_TAG_CLOCK: u8 = 19;
/// [`DeltaOp::HostFacts`] tag.
pub const DELTA_TAG_HOST_FACTS: u8 = 20;

/// Mask of the five defined [`AutoPauseToggles`] bits (`0..=4`).
pub const AUTO_PAUSE_TOGGLE_MASK: u8 = 0x1F;

/// Policy, never wire: the host sends a `Clock` at least this often (in
/// ticks) while the session is ticking.
///
/// Half the observed-tick window, so one lost heartbeat still leaves a fresh
/// tick.
pub const POLICY_CLOCK_HEARTBEAT_TICKS: u64 = 100;

const _: () = assert!(POLICY_CLOCK_HEARTBEAT_TICKS < crate::protocol::POLICY_OBSERVED_TICK_WINDOW);

// ---------------------------------------------------------------------------
// Fact bounds (T140b).
// ---------------------------------------------------------------------------

/// Most pools one [`FactSet`] may carry.
pub const MAX_FACT_POOLS: usize = 16;
/// Most attached effects one [`FactSet`] may carry.
pub const MAX_FACT_EFFECTS: usize = 16;
/// Most stat values one [`FactSet`] may carry.
pub const MAX_FACT_STATS: usize = 32;
/// Most record tags one [`FactSet`] may carry.
pub const MAX_FACT_TAGS: usize = 8;
/// Longest stat name or tag, in bytes (the shortest is 1).
pub const MAX_FACT_NAME_BYTES: usize = 32;
/// Most timeline entries one [`DeltaOp::Timeline`] may carry.
pub const MAX_TIMELINE_ENTRIES: usize = crate::protocol::MAX_VISIBLE_ENTITIES;
/// Largest legal clock `elapsed`: `i32::MAX`, the sim's `RT_CLOCK_MAX`.
pub const MAX_CLOCK_ELAPSED: u64 = 2_147_483_647;
/// Wire `kind` byte of [`StatValue::Int`].
pub const STAT_KIND_INT: u8 = 0;
/// Wire `kind` byte of [`StatValue::Fixed`].
pub const STAT_KIND_FIXED: u8 = 1;
/// Largest delta-frame header: version 1 + lane 1 + `server_tick` 10 +
/// `event_seq` 10 + op count 2 (at most 256).
pub const MAX_DELTA_HEADER_BYTES_V3: usize = 24;
/// Op bytes one frame may carry so that it fits
/// [`MAX_DELTA_FRAME_BYTES`](crate::protocol::MAX_DELTA_FRAME_BYTES) under any
/// header.
pub const MAX_FRAME_OP_BYTES_V3: usize =
    crate::protocol::MAX_DELTA_FRAME_BYTES - MAX_DELTA_HEADER_BYTES_V3;

// ---------------------------------------------------------------------------
// Modes, triggers, pause reasons, host facts.
// ---------------------------------------------------------------------------

/// Encounter time mode (wire byte 0 or 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EncounterMode {
    /// Turn-based (wire 0).
    TurnBased,
    /// Real-time (wire 1).
    RealTime,
}

impl EncounterMode {
    /// Returns the wire byte.
    pub fn as_u8(self) -> u8 {
        match self {
            Self::TurnBased => 0,
            Self::RealTime => 1,
        }
    }

    /// Maps a wire byte to its mode, or [`None`] for 2..=255.
    pub fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::TurnBased),
            1 => Some(Self::RealTime),
            _ => None,
        }
    }
}

/// An event that may pause the host automatically (wire bit and byte 0..=4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoPauseTrigger {
    /// An encounter started (wire 0).
    EncounterStarted,
    /// A round started (wire 1).
    RoundStarted,
    /// The encounter mode changed (wire 2).
    ModeChanged,
    /// A standing order ended (wire 3).
    OrderEnded,
    /// A party member died (wire 4).
    PartyMemberDied,
}

impl AutoPauseTrigger {
    /// Returns the wire value (`0..=4`).
    pub fn as_u8(self) -> u8 {
        match self {
            Self::EncounterStarted => 0,
            Self::RoundStarted => 1,
            Self::ModeChanged => 2,
            Self::OrderEnded => 3,
            Self::PartyMemberDied => 4,
        }
    }

    /// Maps a wire value to its trigger, or [`None`] for 5..=255.
    pub fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::EncounterStarted),
            1 => Some(Self::RoundStarted),
            2 => Some(Self::ModeChanged),
            3 => Some(Self::OrderEnded),
            4 => Some(Self::PartyMemberDied),
            _ => None,
        }
    }
}

/// Why the host is paused. Carries no peer identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    /// A player paused (tag 0).
    Player,
    /// An auto-pause trigger fired (tag 1, then the trigger byte).
    Auto(AutoPauseTrigger),
    /// The operator paused (tag 2).
    Operator,
    /// The session paused itself after a restore (tag 3).
    Restore,
}

impl PauseReason {
    /// Returns the wire reason tag (`0..=3`, declaration order).
    pub fn tag(self) -> u8 {
        match self {
            Self::Player => 0,
            Self::Auto(_) => 1,
            Self::Operator => 2,
            Self::Restore => 3,
        }
    }
}

/// The auto-pause toggles as a bit set: bit `n` set means the trigger with
/// wire value `n` is on. Bits 5..=7 are always clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AutoPauseToggles(u8);

impl AutoPauseToggles {
    /// Wraps raw bits, or returns [`None`] if any bit outside
    /// [`AUTO_PAUSE_TOGGLE_MASK`] is set.
    pub fn from_bits(bits: u8) -> Option<Self> {
        if bits & !AUTO_PAUSE_TOGGLE_MASK != 0 {
            None
        } else {
            Some(Self(bits))
        }
    }

    /// Returns the raw bits (`0..=0x1F`).
    pub fn bits(self) -> u8 {
        self.0
    }

    /// Reports whether `trigger` is on.
    pub fn get(self, trigger: AutoPauseTrigger) -> bool {
        self.0 & (1 << trigger.as_u8()) != 0
    }

    /// Returns a copy with `trigger` switched to `on`.
    pub fn with(self, trigger: AutoPauseTrigger, on: bool) -> Self {
        let bit = 1u8 << trigger.as_u8();
        if on {
            Self(self.0 | bit)
        } else {
            Self(self.0 & !bit)
        }
    }
}

/// The viewer's own pause rights; never withheld.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PauseRights {
    /// The viewer may pause.
    pub pause: bool,
    /// The viewer may unpause.
    pub unpause: bool,
}

/// Host facts for one viewer (tag [`DELTA_TAG_HOST_FACTS`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostFacts {
    /// The viewer's own rights; never withheld.
    pub rights: PauseRights,
    /// The auto-pause toggles; [`None`] is withheld.
    pub auto_pause: Option<AutoPauseToggles>,
    /// Remaining turn-timer milliseconds: [`None`] is withheld,
    /// `Some(None)` is disclosed with no timer running, `Some(Some(ms))` is
    /// the remaining time.
    pub turn_timer_ms: Option<Option<u32>>,
}

// ---------------------------------------------------------------------------
// Fact types (T140b).
// ---------------------------------------------------------------------------

/// The encounter's round clock: ticks per round and ticks elapsed in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockFact {
    /// Ticks in one round; never 0.
    pub round_ticks: u32,
    /// Ticks elapsed in the current round, at most [`MAX_CLOCK_ELAPSED`].
    pub elapsed: u64,
}

impl ClockFact {
    /// Checks the wire shape: `round_ticks` nonzero and `elapsed` within
    /// [`MAX_CLOCK_ELAPSED`], both [`CodecError::Malformed`].
    pub fn check(&self) -> Result<(), CodecError> {
        if self.round_ticks == 0 || self.elapsed > MAX_CLOCK_ELAPSED {
            return Err(CodecError::Malformed);
        }
        Ok(())
    }
}

/// Encounter facts for one viewer (tag [`DELTA_TAG_ENCOUNTER`]). A [`None`]
/// field is withheld, never zero; a disclosed nullable value is a nested
/// `Option`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EncounterFacts {
    /// The time mode; [`None`] is withheld.
    pub mode: Option<EncounterMode>,
    /// The pending mode: [`None`] is withheld, `Some(None)` is disclosed with
    /// no pending mode.
    pub pending_mode: Option<Option<EncounterMode>>,
    /// The round clock: [`None`] is withheld, `Some(None)` is disclosed with
    /// no clock.
    pub clock: Option<Option<ClockFact>>,
}

impl EncounterFacts {
    /// Reports whether all three fields are withheld.
    pub fn is_withheld(&self) -> bool {
        self.mode.is_none() && self.pending_mode.is_none() && self.clock.is_none()
    }
}

/// One timeline entry: the ordering key and the entity it orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineEntry {
    /// Ordering key; keys never decrease along a timeline.
    pub key: i32,
    /// The replica entity. Order among equal keys is the authority's, and
    /// the codec never checks it.
    pub entity: NetId,
}

/// One resource pool of an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolFact {
    /// Authored pool identity.
    pub pool: Ulid,
    /// Current amount, at most `max`.
    pub current: u32,
    /// Maximum amount.
    pub max: u32,
}

/// One effect attached to an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectFact {
    /// Authored effect identity.
    pub effect: Ulid,
    /// The round the effect expires at.
    pub expires_round: u64,
}

/// An entity's standing order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderFact {
    /// Authored ability identity.
    pub ability: Ulid,
    /// The replica entity the order targets.
    pub target: NetId,
    /// Whether the order repeats.
    pub repeat: bool,
}

/// A stat value: the `Int` attribute or `Fixed` stat kinds only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatValue {
    /// An integer attribute (wire kind [`STAT_KIND_INT`]).
    Int(i32),
    /// A `Fixed` stat as raw `Fx16_16` bits (wire kind [`STAT_KIND_FIXED`]).
    Fixed(i32),
}

/// One named stat of an entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatFact {
    /// Stat name, 1..=[`MAX_FACT_NAME_BYTES`] bytes.
    pub name: String,
    /// The value.
    pub value: StatValue,
}

/// An entity's fact set (tag [`DELTA_TAG_ENTITY_FACTS`] after the entity).
/// Each family is withheld when [`None`]; an empty disclosed list is distinct
/// from withheld.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FactSet {
    /// Pools, at most [`MAX_FACT_POOLS`], pairwise distinct.
    pub pools: Option<Vec<PoolFact>>,
    /// Effects, at most [`MAX_FACT_EFFECTS`], strictly ascending by id.
    pub effects: Option<Vec<EffectFact>>,
    /// The standing order: [`None`] is withheld, `Some(None)` is disclosed
    /// with no order.
    pub order: Option<Option<OrderFact>>,
    /// Stats, at most [`MAX_FACT_STATS`], names pairwise distinct.
    pub stats: Option<Vec<StatFact>>,
    /// Record tags, at most [`MAX_FACT_TAGS`], strictly ascending bytewise.
    pub tags: Option<Vec<String>>,
}

impl FactSet {
    /// Reports whether all five families are withheld.
    pub fn is_withheld(&self) -> bool {
        self.pools.is_none()
            && self.effects.is_none()
            && self.order.is_none()
            && self.stats.is_none()
            && self.tags.is_none()
    }
}

/// Checks a pool list: count over [`MAX_FACT_POOLS`] is
/// [`CodecError::LimitExceeded`]; then per pool in order, `current > max` and
/// a repeated pool id are [`CodecError::Malformed`].
pub fn check_pools(pools: &[PoolFact]) -> Result<(), CodecError> {
    if pools.len() > MAX_FACT_POOLS {
        return Err(CodecError::LimitExceeded);
    }
    for (index, pool) in pools.iter().enumerate() {
        if pool.current > pool.max {
            return Err(CodecError::Malformed);
        }
        if pools[..index]
            .iter()
            .any(|earlier| earlier.pool == pool.pool)
        {
            return Err(CodecError::Malformed);
        }
    }
    Ok(())
}

/// Checks an effect list: count over [`MAX_FACT_EFFECTS`] is
/// [`CodecError::LimitExceeded`]; then each effect id must be strictly
/// greater than the previous one ([`CodecError::Malformed`]).
pub fn check_effects(effects: &[EffectFact]) -> Result<(), CodecError> {
    if effects.len() > MAX_FACT_EFFECTS {
        return Err(CodecError::LimitExceeded);
    }
    for pair in effects.windows(2) {
        if pair[1].effect <= pair[0].effect {
            return Err(CodecError::Malformed);
        }
    }
    Ok(())
}

/// Checks one stat name or tag length: over [`MAX_FACT_NAME_BYTES`] is
/// [`CodecError::LimitExceeded`], empty is [`CodecError::Malformed`].
pub(crate) fn check_name_len(len: usize) -> Result<(), CodecError> {
    if len > MAX_FACT_NAME_BYTES {
        return Err(CodecError::LimitExceeded);
    }
    if len == 0 {
        return Err(CodecError::Malformed);
    }
    Ok(())
}

/// Checks a stat list: count over [`MAX_FACT_STATS`] is
/// [`CodecError::LimitExceeded`]; then per stat in order, a name over
/// [`MAX_FACT_NAME_BYTES`] bytes is [`CodecError::LimitExceeded`], an empty
/// name or a name equal to an earlier stat's is [`CodecError::Malformed`].
pub fn check_stats(stats: &[StatFact]) -> Result<(), CodecError> {
    if stats.len() > MAX_FACT_STATS {
        return Err(CodecError::LimitExceeded);
    }
    for (index, stat) in stats.iter().enumerate() {
        check_name_len(stat.name.len())?;
        if stats[..index]
            .iter()
            .any(|earlier| earlier.name == stat.name)
        {
            return Err(CodecError::Malformed);
        }
    }
    Ok(())
}

/// Checks a tag list: count over [`MAX_FACT_TAGS`] is
/// [`CodecError::LimitExceeded`]; then per tag in order, a tag over
/// [`MAX_FACT_NAME_BYTES`] bytes is [`CodecError::LimitExceeded`], an empty
/// tag or one not strictly greater (byte order) than the previous is
/// [`CodecError::Malformed`].
pub fn check_tags(tags: &[String]) -> Result<(), CodecError> {
    if tags.len() > MAX_FACT_TAGS {
        return Err(CodecError::LimitExceeded);
    }
    for (index, tag) in tags.iter().enumerate() {
        check_name_len(tag.len())?;
        if index > 0 && tag.as_bytes() <= tags[index - 1].as_bytes() {
            return Err(CodecError::Malformed);
        }
    }
    Ok(())
}

/// Checks a timeline: count over [`MAX_TIMELINE_ENTRIES`] is
/// [`CodecError::LimitExceeded`]; then per entry in order, a key below the
/// previous key and an entity seen earlier are [`CodecError::Malformed`].
/// `NetId` order among equal keys is never checked.
pub fn check_timeline(entries: &[TimelineEntry]) -> Result<(), CodecError> {
    if entries.len() > MAX_TIMELINE_ENTRIES {
        return Err(CodecError::LimitExceeded);
    }
    let mut seen = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 && entry.key < entries[index - 1].key {
            return Err(CodecError::Malformed);
        }
        if !seen.insert(entry.entity) {
            return Err(CodecError::Malformed);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rejection codes and receipts.
// ---------------------------------------------------------------------------

/// v3 rejection codes: the frozen v1 codes `1..=20` plus `21..=30`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RejectionCode {
    /// Frame exceeded its byte cap before decode.
    FrameTooLarge = 1,
    /// Bounded ingress queue was full.
    QueueFull = 2,
    /// Unknown protocol version.
    UnsupportedVersion = 3,
    /// Wrong lane or direction.
    WrongDirection = 4,
    /// Unknown message tag.
    UnknownMessage = 5,
    /// Truncated or ill-formed frame.
    Malformed = 6,
    /// A wire collection, string, or op bound was exceeded.
    LimitExceeded = 7,
    /// Per-peer rate budget exhausted.
    RateLimited = 8,
    /// Connection not authenticated.
    Unauthenticated = 9,
    /// Session epoch expired or unknown.
    SessionExpired = 10,
    /// Same seq retried with different canonical bytes; closes the epoch.
    SeqConflict = 11,
    /// New seq skipped ahead of the next expected seq.
    SeqGap = 12,
    /// Seq older than the retained cache.
    StaleSeq = 13,
    /// Sequence space exhausted; a new epoch is required.
    SeqExhausted = 14,
    /// Observed tick older than the freshness window.
    StaleTick = 15,
    /// Observed tick ahead of the server tick.
    FutureTick = 16,
    /// Actor/target not resolvable for this client.
    NotAuthorized = 17,
    /// Gameplay-illegal at execution time.
    IllegalAction = 18,
    /// Server refuses a new peer for the proof population bound.
    ServerBusy = 19,
    /// Snapshot reassembly expired.
    SnapshotExpired = 20,
    /// The ruleset does not support the requested mode.
    ModeUnsupported = 21,
    /// The command does not apply in the encounter's current mode.
    WrongMode = 22,
    /// A stat could not be evaluated.
    StatEvaluation = 23,
    /// The encounter already ended.
    EncounterTerminal = 24,
    /// The ruleset of the request does not match the encounter's.
    RulesetMismatch = 25,
    /// The placement is already in use.
    PlacementInUse = 26,
    /// The entity is not a creature.
    NotCreature = 27,
    /// The entity already participates in the encounter.
    AlreadyParticipant = 28,
    /// The encounter has no entrants.
    NoEntrants = 29,
    /// The peer has no right to pause or unpause.
    NoPauseRight = 30,
}

impl RejectionCode {
    /// Returns the wire value (`1..=30`).
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Maps a wire byte to its code, or [`None`] for 0 and 31..=255.
    pub fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::FrameTooLarge),
            2 => Some(Self::QueueFull),
            3 => Some(Self::UnsupportedVersion),
            4 => Some(Self::WrongDirection),
            5 => Some(Self::UnknownMessage),
            6 => Some(Self::Malformed),
            7 => Some(Self::LimitExceeded),
            8 => Some(Self::RateLimited),
            9 => Some(Self::Unauthenticated),
            10 => Some(Self::SessionExpired),
            11 => Some(Self::SeqConflict),
            12 => Some(Self::SeqGap),
            13 => Some(Self::StaleSeq),
            14 => Some(Self::SeqExhausted),
            15 => Some(Self::StaleTick),
            16 => Some(Self::FutureTick),
            17 => Some(Self::NotAuthorized),
            18 => Some(Self::IllegalAction),
            19 => Some(Self::ServerBusy),
            20 => Some(Self::SnapshotExpired),
            21 => Some(Self::ModeUnsupported),
            22 => Some(Self::WrongMode),
            23 => Some(Self::StatEvaluation),
            24 => Some(Self::EncounterTerminal),
            25 => Some(Self::RulesetMismatch),
            26 => Some(Self::PlacementInUse),
            27 => Some(Self::NotCreature),
            28 => Some(Self::AlreadyParticipant),
            29 => Some(Self::NoEntrants),
            30 => Some(Self::NoPauseRight),
            _ => None,
        }
    }
}

impl From<crate::protocol::RejectionCode> for RejectionCode {
    fn from(code: crate::protocol::RejectionCode) -> Self {
        use crate::protocol::RejectionCode as V1;
        match code {
            V1::FrameTooLarge => Self::FrameTooLarge,
            V1::QueueFull => Self::QueueFull,
            V1::UnsupportedVersion => Self::UnsupportedVersion,
            V1::WrongDirection => Self::WrongDirection,
            V1::UnknownMessage => Self::UnknownMessage,
            V1::Malformed => Self::Malformed,
            V1::LimitExceeded => Self::LimitExceeded,
            V1::RateLimited => Self::RateLimited,
            V1::Unauthenticated => Self::Unauthenticated,
            V1::SessionExpired => Self::SessionExpired,
            V1::SeqConflict => Self::SeqConflict,
            V1::SeqGap => Self::SeqGap,
            V1::StaleSeq => Self::StaleSeq,
            V1::SeqExhausted => Self::SeqExhausted,
            V1::StaleTick => Self::StaleTick,
            V1::FutureTick => Self::FutureTick,
            V1::NotAuthorized => Self::NotAuthorized,
            V1::IllegalAction => Self::IllegalAction,
            V1::ServerBusy => Self::ServerBusy,
            V1::SnapshotExpired => Self::SnapshotExpired,
        }
    }
}

/// Terminal outcome of one finalized command (wire 0, or 1 then the code).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptStatus {
    /// The command finalized successfully.
    Applied,
    /// The command finalized as rejected with a v3 code.
    Rejected(RejectionCode),
}

impl From<crate::protocol::ReceiptStatus> for ReceiptStatus {
    fn from(status: crate::protocol::ReceiptStatus) -> Self {
        match status {
            crate::protocol::ReceiptStatus::Applied => Self::Applied,
            crate::protocol::ReceiptStatus::Rejected(code) => Self::Rejected(code.into()),
        }
    }
}

// ---------------------------------------------------------------------------
// Client intents (upstream).
// ---------------------------------------------------------------------------

/// One versioned, lane-tagged v3 client command frame.
///
/// The header is the v1 header. `actor` is nonzero; for `Pause`, `Unpause`
/// and `RequestMode` it names the requesting peer's bound character, and the
/// host decides what that authorizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntentFrame {
    /// Session generation this command belongs to.
    pub epoch: [u8; 16],
    /// Must equal [`LANE_COMBAT`](crate::protocol::LANE_COMBAT).
    pub lane: u8,
    /// Per-lane command identity, `SEQ_FIRST..=SEQ_LAST`.
    pub seq: u64,
    /// Last authoritative tick this client acknowledges; advisory only.
    pub observed_tick: u64,
    /// Acting replica entity.
    pub actor: NetId,
    /// The closed verb.
    pub body: IntentBody,
}

/// Closed v3 intent vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentBody {
    /// Declare one single-target ability use (tag 0): in turn-based mode the
    /// actor acts now, in real time it sets the standing order.
    DeclareAction {
        /// Authored ability identity.
        ability: Ulid,
        /// Single target replica entity.
        target: NetId,
    },
    /// End the actor's turn (tag 1).
    EndTurn,
    /// Stop the actor's standing order (tag 2).
    Stop,
    /// Ask the host to pause (tag 3).
    Pause,
    /// Ask the host to unpause (tag 4).
    Unpause,
    /// Ask the host to change the encounter mode (tag 5, then the mode byte).
    RequestMode {
        /// The requested mode.
        mode: EncounterMode,
    },
}

// ---------------------------------------------------------------------------
// Server deltas (downstream).
// ---------------------------------------------------------------------------

/// One versioned v3 server delta frame.
///
/// `lane` must equal [`LANE_COMBAT`](crate::protocol::LANE_COMBAT).
/// `server_tick` is the authoritative tick the frame was produced at and is
/// the only tick on the wire; `event_seq` is the per-client delivery
/// sequence. A heartbeat is an ordinary frame holding a [`DeltaOp::Clock`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaFrame {
    /// Must equal [`LANE_COMBAT`](crate::protocol::LANE_COMBAT).
    pub lane: u8,
    /// Authoritative tick this delta was produced at.
    pub server_tick: u64,
    /// Per-client delivery sequence, gapless per peer.
    pub event_seq: u64,
    /// Filtered replica operations for this frame, in order.
    pub ops: Vec<DeltaOp>,
}

/// Closed v3 delta vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaOp {
    /// A permitted entity becomes visible (tag 0).
    EntityEnter {
        /// The replica entity now visible.
        entity: NetId,
    },
    /// A still-known entity stops being visible (tag 1).
    EntityLeave {
        /// The replica entity no longer visible.
        entity: NetId,
    },
    /// A visible spawn notice (tag 2).
    Spawned {
        /// The spawned replica entity.
        entity: NetId,
    },
    /// A typed despawn notice (tag 3).
    Despawned {
        /// The removed replica entity.
        entity: NetId,
    },
    /// A disclosable death notice (tag 4).
    Died {
        /// The replica entity that died.
        entity: NetId,
    },
    /// Permitted health/dead replica state (tag 5).
    Health {
        /// The replica entity described.
        entity: NetId,
        /// Current health.
        health: u32,
        /// Maximum health.
        max_health: u32,
        /// Terminal flag.
        dead: bool,
    },
    /// Permitted active-actor/round state (tag 6); round and the active
    /// combatant travel only here.
    Turn {
        /// Active actor, if the encounter has one.
        active: Option<NetId>,
        /// Current round.
        round: u64,
    },
    /// Terminal per-command outcome for one intent (tag 7).
    Receipt {
        /// Session generation of the command.
        epoch: [u8; 16],
        /// Lane of the command.
        lane: u8,
        /// Sequence of the command.
        seq: u64,
        /// Authoritative tick the command finalized at.
        processed_tick: u64,
        /// Outcome: applied or rejected with a v3 code.
        status: ReceiptStatus,
    },
    /// One disclosable ability resolution (tag 8).
    ActionResolved {
        /// The disclosable acting replica entity.
        actor: NetId,
        /// The disclosable targeted replica entity.
        target: NetId,
        /// The authored ability identity.
        ability: Ulid,
        /// The symbolic outcome (T020's five forms).
        outcome: String,
        /// The reported flat damage.
        damage: u32,
    },
    /// One disclosable logical turn start (tag 9).
    TurnStarted {
        /// The disclosable incoming turn head.
        actor: NetId,
        /// The combat round.
        round: u64,
    },
    /// The first transition to terminal (tag 10).
    EncounterEnded {
        /// The naturally completed encounter identity.
        encounter: Ulid,
        /// The closing combat round.
        round: u64,
    },
    /// The viewer's encounter facts (tag 11).
    Encounter(EncounterFacts),
    /// The viewer's timeline image (tag 12): [`None`] is withheld, which
    /// tells the client to forget the timeline.
    Timeline {
        /// Entries in authority order, keys non-decreasing, entities
        /// distinct; hidden entities have no entry.
        entries: Option<Vec<TimelineEntry>>,
    },
    /// The viewer's fact image of one entity (tag 13); an all-withheld set
    /// tells the client to forget the facts.
    EntityFacts {
        /// The replica entity described.
        entity: NetId,
        /// The viewer's complete current image.
        facts: FactSet,
    },
    /// A round started (tag 14).
    RoundStarted {
        /// The round that started.
        round: u64,
    },
    /// The encounter mode changed (tag 15).
    ModeChanged {
        /// The new mode.
        mode: EncounterMode,
        /// The round at the change.
        round: u64,
    },
    /// A standing order ended (tag 16).
    OrderEnded {
        /// The disclosable actor whose order ended.
        actor: NetId,
    },
    /// An entity joined the encounter; a notice that mints no id (tag 17).
    Joined {
        /// The replica entity that joined.
        entity: NetId,
    },
    /// An entity left the encounter; a notice that retires no id (tag 18).
    Left {
        /// The replica entity that left.
        entity: NetId,
    },
    /// The pause state heartbeat (tag 19): [`None`] is running,
    /// `Some(None)` is paused with the reason withheld, `Some(Some(r))` is
    /// paused for `r`. Carries no tick; the frame header does.
    Clock {
        /// Pause state and, when granted, its reason.
        paused: Option<Option<PauseReason>>,
    },
    /// The viewer's host facts (tag 20).
    HostFacts(HostFacts),
}

impl From<crate::protocol::DeltaOp> for DeltaOp {
    fn from(op: crate::protocol::DeltaOp) -> Self {
        use crate::protocol::DeltaOp as V1;
        match op {
            V1::EntityEnter { entity } => Self::EntityEnter { entity },
            V1::EntityLeave { entity } => Self::EntityLeave { entity },
            V1::Spawned { entity } => Self::Spawned { entity },
            V1::Despawned { entity } => Self::Despawned { entity },
            V1::Died { entity } => Self::Died { entity },
            V1::Health {
                entity,
                health,
                max_health,
                dead,
            } => Self::Health {
                entity,
                health,
                max_health,
                dead,
            },
            V1::Turn { active, round } => Self::Turn { active, round },
            V1::Receipt {
                epoch,
                lane,
                seq,
                processed_tick,
                status,
            } => Self::Receipt {
                epoch,
                lane,
                seq,
                processed_tick,
                status: status.into(),
            },
        }
    }
}

impl From<crate::protocol_v2::DeltaOp> for DeltaOp {
    fn from(op: crate::protocol_v2::DeltaOp) -> Self {
        use crate::protocol_v2::DeltaOp as V2;
        match op {
            V2::Legacy(inner) => inner.into(),
            V2::ActionResolved {
                actor,
                target,
                ability,
                outcome,
                damage,
            } => Self::ActionResolved {
                actor,
                target,
                ability,
                outcome,
                damage,
            },
            V2::TurnStarted { actor, round } => Self::TurnStarted { actor, round },
            V2::EncounterEnded { encounter, round } => Self::EncounterEnded { encounter, round },
        }
    }
}
