//! Opt-in authoritative history (T020, ADR-0017).
//!
//! [`HistoryWorld`] owns one [`World`](crate::World) privately and journals
//! every authoritative transition in a bounded, single-consumer journal of
//! [`HistoryEvent`] envelopes. Callers get immutable world queries plus the
//! approved typed transactional operations — never `&mut World`, stores,
//! queues, or RNG handles on the owned authority. Each operation stages a
//! clone of the world, reuses the existing combat controllers on the staged
//! state, drains the staged legacy queue exactly once, translates each
//! legacy event exactly once, collects the branch-observed transition facts,
//! validates the journal additions against the live wrapper, and only then
//! publishes. A rejected operation commits nothing: authoritative state,
//! RNG draws, entity allocation, legacy queue counters, and history
//! counters are all preserved.
//!
//! The legacy path is untouched: [`SimEvent`](crate::SimEvent) stays closed,
//! every free function and [`World`](crate::World) method keeps its
//! emissions, serialization bytes, and hashes. [`history_hash`] covers the
//! complete canonical wrapper representation — world, version, pending
//! payloads, and sequence/acknowledgement state — with no exclusions.
//!
//! ## Bounded parsing honesty
//!
//! The journal caps ([`MAX_HISTORY_EVENTS`], [`MAX_HISTORY_BYTES`], and
//! [`MAX_HISTORY_STRING_BYTES`]) bound *retained* history, not all transient
//! allocation inside an arbitrary JSON parser: `serde_json` may allocate
//! scratch storage for an escaped string before any string visitor runs, and
//! generic `Deserialize` cannot impose a parser-wide memory bound. Load
//! therefore checks decoded string lengths before retaining them, rejects an
//! excess entry before decoding its envelope, and checks canonical byte
//! totals incrementally before retaining each envelope — and T022 must cap
//! checkpoint input bytes *before* invoking JSON parsing (including
//! reader-based input). Parser scratch is bounded by that host input
//! budget, not by the sim journal string cap. Adjacent-tag payloads require
//! `type` before `value` on input and reject the reverse order before
//! decoding the value, avoiding whole-payload buffering.

use std::cell::Cell;
use std::fmt;

use serde::de::{DeserializeSeed, Error as DeError, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crpg_core::{EntityId, Tick, Ulid};
use crpg_rules::Outcome;

use crate::combat::{
    perform_action_tracked, start_encounter_in_mode_tracked, ActionOutcome, CombatAction,
    CombatError, EncounterMode, EncounterSpec, EncounterSummary, TickEvent, TransitionFacts,
};
use crate::creature::{CreatureSpec, CreatureState};
use crate::event::SimEvent;
use crate::transform::Transform;
use crate::world::{EntityMeta, World};

/// Maximum envelopes retained in one history journal.
pub const MAX_HISTORY_EVENTS: usize = 4096;
/// Maximum canonical serialized envelope bytes retained in one journal.
pub const MAX_HISTORY_BYTES: usize = 1_048_576;
/// Maximum envelopes returned by one [`HistoryWorld::read_after`] page.
pub const MAX_HISTORY_PAGE: usize = 256;
/// Maximum UTF-8 bytes in one retained history string payload.
pub const MAX_HISTORY_STRING_BYTES: usize = 256;
/// The newest history wrapper version this build writes; 1 is still read.
pub const HISTORY_VERSION: u32 = 2;
/// The version a new wrapper starts at (T138 D16): it upgrades lazily.
const HISTORY_VERSION_V1: u32 = 1;

/// One authoritative history fact, in the wrapper's own terms.
///
/// A separate sim-owned, core-closed vocabulary (T020, ADR-0017): structural
/// and death facts mirror the drained legacy queue one-for-one while the
/// richer variants carry only symbolic outcome strings, reported damage,
/// widened rounds, modes and entity references — never rolls, margins, DCs,
/// or interned handles. The last three variants are version 2 (T138b,
/// ADR-0029 N5). Serializes with adjacent `type`/`value` tags in snake
/// case; deserialization requires `type` before `value` and rejects the
/// reverse order before decoding the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum HistoryEvent {
    /// The wrapper spawned this entity.
    Spawned {
        /// The minted entity.
        entity: EntityId,
    },
    /// The wrapper despawned this entity.
    Despawned {
        /// The removed entity.
        entity: EntityId,
    },
    /// Combat killed this entity, translated from the drained legacy queue.
    Died {
        /// The combatant whose health reached zero.
        entity: EntityId,
    },
    /// One accepted ability use resolved, including legal failure.
    ActionResolved {
        /// The acting combatant.
        actor: EntityId,
        /// The targeted combatant.
        target: EntityId,
        /// The ability that was used.
        ability: Ulid,
        /// The symbolic outcome: `critical_success`, `success`, `failure`,
        /// `critical_failure`, or `custom:<n>` with decimal `n` and no
        /// leading zeros.
        outcome: String,
        /// The reported flat damage applied (zero on failure or unmapped
        /// outcomes; possibly overkill, never an inferred health delta).
        damage: u32,
    },
    /// One logical turn actually started, as observed by the controller
    /// branch that refreshed it.
    TurnStarted {
        /// The incoming turn head.
        actor: EntityId,
        /// The combat round widened from the persisted `u32` counter.
        round: u64,
    },
    /// The first transition to terminal, never an explicit release.
    EncounterEnded {
        /// The naturally completed encounter.
        encounter: Ulid,
        /// The closing combat round widened from the persisted counter.
        round: u64,
    },
    /// A round started in RT (ADR-0029 T6, D10).
    RoundStarted {
        /// The new round, widened from the persisted `u32`.
        round: u64,
    },
    /// A pending mode applied at a round boundary (ADR-0029 T10).
    ModeChanged {
        /// The mode now in force.
        mode: EncounterMode,
        /// The round the switch started, widened.
        round: u64,
    },
    /// A due order failed validation and was cleared (ADR-0029 T7, D11).
    OrderEnded {
        /// The combatant whose order ended.
        actor: EntityId,
    },
}

impl HistoryEvent {
    /// Whether this is one of the three version 2 variants (T138b).
    fn is_v2(&self) -> bool {
        matches!(
            self,
            Self::RoundStarted { .. } | Self::ModeChanged { .. } | Self::OrderEnded { .. }
        )
    }
}

/// One journaled event: its payload plus the coordinates that order it.
///
/// Field order is the canonical serialization order: tick, then sequence,
/// then payload. Deserialization is manual so duplicate keys fail instead
/// of silently keeping the last value; field order stays free on input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryEnvelope {
    /// The authoritative tick the operation ran under.
    pub tick: Tick,
    /// The journal sequence, assigned on commit from `1` upward.
    pub seq: u64,
    /// The event itself.
    pub payload: HistoryEvent,
}

impl<'de> Deserialize<'de> for HistoryEnvelope {
    /// Decodes one envelope, rejecting duplicate, unknown, or missing keys.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(EnvelopeVisitor)
    }
}

/// The [`HistoryEnvelope`] map visitor: duplicate-rejecting, order-free.
struct EnvelopeVisitor;

impl<'de> Visitor<'de> for EnvelopeVisitor {
    type Value = HistoryEnvelope;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a history envelope with `tick`, `seq`, and `payload`")
    }

    fn visit_map<A>(self, mut map: A) -> Result<HistoryEnvelope, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut tick: Option<Tick> = None;
        let mut seq: Option<u64> = None;
        let mut payload: Option<HistoryEvent> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "tick" => {
                    if tick.is_some() {
                        return Err(A::Error::custom("duplicate history envelope field `tick`"));
                    }
                    tick = Some(map.next_value()?);
                }
                "seq" => {
                    if seq.is_some() {
                        return Err(A::Error::custom("duplicate history envelope field `seq`"));
                    }
                    seq = Some(map.next_value()?);
                }
                "payload" => {
                    if payload.is_some() {
                        return Err(A::Error::custom(
                            "duplicate history envelope field `payload`",
                        ));
                    }
                    payload = Some(map.next_value()?);
                }
                _ => {
                    return Err(A::Error::custom("unknown history envelope field"));
                }
            }
        }
        let tick = tick.ok_or_else(|| A::Error::custom("history envelope misses `tick`"))?;
        let seq = seq.ok_or_else(|| A::Error::custom("history envelope misses `seq`"))?;
        let payload =
            payload.ok_or_else(|| A::Error::custom("history envelope misses `payload`"))?;
        Ok(HistoryEnvelope { tick, seq, payload })
    }
}

/// Every history failure.
///
/// Gameplay errors arrive first as [`HistoryError::Combat`], which delegates
/// `Display` and `source`; every other display is `<VariantName> at
/// history`, without payloads or source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryError {
    /// The staged gameplay operation failed; nothing was committed.
    Combat(CombatError),
    /// A read page asked for zero or more than 256 envelopes.
    InvalidPageLimit {
        /// The rejected page size.
        limit: usize,
    },
    /// A read named a sequence at or below the acknowledgement watermark.
    StaleCursor {
        /// The requested sequence.
        requested: u64,
        /// The acknowledgement watermark.
        acknowledged: u64,
    },
    /// A read or acknowledgement named a sequence past the last issued one.
    FutureCursor {
        /// The requested sequence.
        requested: u64,
        /// The last issued sequence.
        last: u64,
    },
    /// An emitting operation found no issuable sequence left; zero-event
    /// operations still succeed at exhaustion.
    SequenceExhausted,
    /// An emitting operation would retain more than 4096 envelopes.
    EventLimit,
    /// An emitting operation would retain more than one MiB of canonical
    /// envelope bytes.
    ByteLimit,
    /// A history string payload exceeds 256 UTF-8 bytes.
    StringLimit,
}

impl fmt::Display for HistoryError {
    /// Renders the combat delegate or `<VariantName> at history`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Combat(error) => write!(f, "{error}"),
            Self::InvalidPageLimit { .. } => write!(f, "InvalidPageLimit at history"),
            Self::StaleCursor { .. } => write!(f, "StaleCursor at history"),
            Self::FutureCursor { .. } => write!(f, "FutureCursor at history"),
            Self::SequenceExhausted => write!(f, "SequenceExhausted at history"),
            Self::EventLimit => write!(f, "EventLimit at history"),
            Self::ByteLimit => write!(f, "ByteLimit at history"),
            Self::StringLimit => write!(f, "StringLimit at history"),
        }
    }
}

impl std::error::Error for HistoryError {
    /// The staged combat error, when this failure wraps one.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Combat(error) => Some(error),
            _ => None,
        }
    }
}

/// The retained journal plus its derived canonical byte total (T097).
///
/// The total is the checked sum of [`envelope_len`] over [`envelopes`], cached
/// because every mutation used to re-derive it by re-encoding the whole
/// retained journal. It is a derived value, never persisted: [`Serialize`]
/// writes exactly the envelope sequence, so wrapper bytes are unchanged. J1
/// holds it to the envelopes at every public boundary.
///
/// [`envelopes`]: Journal::envelopes
#[derive(Clone, PartialEq, Eq)]
struct Journal {
    /// The retained envelopes, oldest first.
    envelopes: Vec<HistoryEnvelope>,
    /// The derived canonical byte total; never serialized.
    bytes: usize,
}

impl Journal {
    /// An empty journal with a zero byte total.
    fn new() -> Self {
        Self {
            envelopes: Vec::new(),
            bytes: 0,
        }
    }

    /// A loaded journal carrying the canonical byte total the loader computed.
    fn from_loaded(envelopes: Vec<HistoryEnvelope>, bytes: usize) -> Self {
        Self { envelopes, bytes }
    }

    /// The retained envelopes, oldest first.
    fn as_slice(&self) -> &[HistoryEnvelope] {
        &self.envelopes
    }

    /// The number of retained envelopes.
    fn len(&self) -> usize {
        self.envelopes.len()
    }

    /// The cached canonical byte total.
    fn bytes(&self) -> usize {
        self.bytes
    }

    /// Appends `envelopes`, adopting `total` as the new FULL byte total.
    ///
    /// `total` is the post-append total, not a delta: the caller has already
    /// added the new envelopes' canonical lengths to [`bytes`](Self::bytes).
    fn append(&mut self, envelopes: Vec<HistoryEnvelope>, total: usize) {
        debug_assert!(
            total >= self.bytes,
            "a full byte total only grows on append"
        );
        self.envelopes.extend(envelopes);
        self.bytes = total;
    }

    /// Drops the first `count` envelopes, subtracting their canonical bytes.
    ///
    /// Emptying the journal resets the total to zero without encoding
    /// anything: every retained envelope is already in the cache.
    fn drain_prefix(&mut self, count: usize) {
        debug_assert!(
            count <= self.envelopes.len(),
            "a drained prefix stays inside the journal"
        );
        if count == self.envelopes.len() {
            self.envelopes.clear();
            self.bytes = 0;
            return;
        }
        let mut removed: usize = 0;
        for envelope in &self.envelopes[..count] {
            removed = removed.saturating_add(envelope_len(envelope));
        }
        self.bytes = self.bytes.saturating_sub(removed);
        self.envelopes.drain(..count);
    }
}

impl fmt::Debug for Journal {
    /// Delegates to the envelope sequence, so `Debug` output is unchanged.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.envelopes, f)
    }
}

impl Serialize for Journal {
    /// Writes exactly the retained envelope sequence; the byte cache is not
    /// persisted.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.envelopes.serialize(serializer)
    }
}

/// The opt-in authoritative history wrapper: one privately owned [`World`](crate::World)
/// plus its bounded journal.
///
/// New state journals nothing with `acknowledged = 0` and `next_seq = 1`.
/// Issuable sequences are `1..=u64::MAX - 1`; `u64::MAX` is the exhausted
/// next-sequence sentinel. Pending entries are exactly the contiguous suffix
/// `(acknowledged, next_seq)`; the last sequence is `next_seq - 1` even
/// after acknowledgement.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryWorld {
    /// The wrapper version, 1 or 2 (T138 D16); first for canonical order.
    version: u32,
    /// The privately owned authoritative world.
    world: World,
    /// The acknowledgement watermark: every retained sequence exceeds it.
    acknowledged: u64,
    /// The next issuable sequence, or `u64::MAX` once exhausted.
    next_seq: u64,
    /// The retained contiguous suffix `(acknowledged, next_seq)`.
    pending: Journal,
}

impl HistoryWorld {
    /// A new, empty history world: tick zero, RNG seeded from `seed`, an
    /// empty journal with `acknowledged = 0` and `next_seq = 1`.
    pub fn new(seed: u64) -> Self {
        Self {
            version: HISTORY_VERSION_V1,
            world: World::new(seed),
            acknowledged: 0,
            next_seq: 1,
            pending: Journal::new(),
        }
    }

    /// A new, empty history world whose inner world is bound to the authored
    /// `area` (T027a, ADR-0020); otherwise identical to [`new`](Self::new).
    pub fn new_in_area(seed: u64, area: Ulid) -> Self {
        Self {
            world: World::new_in_area(seed, area),
            ..Self::new(seed)
        }
    }

    /// Attaches an already validated transform to a live entity of the staged
    /// wrapper, journaling nothing. Crate-visible only: the area transfer's
    /// pair transaction uses it after its typed spawn; no public mutable-inner
    /// path exists.
    pub(crate) fn attach_transform(&mut self, entity: EntityId, transform: Transform) {
        debug_assert!(self.world.contains(entity), "attach to a live entity");
        self.world.transforms_mut().insert(entity, transform);
    }

    /// The wrapper version: 1 until the first RT-capable commit, then 2 (D16).
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Immutable view of the privately owned authoritative world.
    ///
    /// Cloning this reference cannot mutate or replace the wrapper's
    /// authority: every store, queue, and RNG accessor reachable through it
    /// needs a mutable borrow the shared reference cannot provide, and so do
    /// the free mutating controllers.
    ///
    /// ```compile_fail
    /// use crpg_sim::HistoryWorld;
    /// let mut history = HistoryWorld::new(0);
    /// history.world().transforms_mut();
    /// ```
    ///
    /// ```compile_fail
    /// use crpg_sim::HistoryWorld;
    /// let mut history = HistoryWorld::new(0);
    /// history.world().events_mut();
    /// ```
    ///
    /// ```compile_fail
    /// use crpg_sim::HistoryWorld;
    /// let mut history = HistoryWorld::new(0);
    /// history.world().rng_mut();
    /// ```
    ///
    /// ```compile_fail
    /// use crpg_sim::{CombatAction, HistoryWorld, perform_action};
    /// let mut history = HistoryWorld::new(0);
    /// let actor = history.world().ids().next().expect("illustrative only");
    /// perform_action(history.world(), &CombatAction::EndTurn { actor });
    /// ```
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Mints an entity and journals one `Spawned` transactionally.
    ///
    /// Capacity rejection preserves entity allocation: the staged id is
    /// discarded with the staged clone.
    pub fn spawn(&mut self, meta: EntityMeta) -> Result<EntityId, HistoryError> {
        let mut world = self.world.clone();
        let id = world.spawn(meta);
        let drained = world.events_mut().drain();
        let tick = world.tick();
        debug_assert_eq!(drained.len(), 1, "spawn enqueues exactly one event");
        let mut events = Vec::with_capacity(drained.len());
        for envelope in &drained {
            events.push(translate_legacy(&envelope.payload));
        }
        self.commit(world, events, tick)?;
        Ok(id)
    }

    /// Journaled registration (T139, ADR-0029 T16): stages
    /// [`register_creature`](crate::register_creature) and journals one
    /// `Spawned`. Gameplay errors win and commit nothing.
    pub fn register_creature(&mut self, spec: &CreatureSpec<'_>) -> Result<EntityId, HistoryError> {
        let mut world = self.world.clone();
        let id =
            crate::creature::register_creature(&mut world, spec).map_err(HistoryError::Combat)?;
        let drained = world.events_mut().drain();
        let tick = world.tick();
        debug_assert_eq!(drained.len(), 1, "registration enqueues exactly one event");
        let mut events = Vec::with_capacity(drained.len());
        for envelope in &drained {
            events.push(translate_legacy(&envelope.payload));
        }
        self.commit(world, events, tick)?;
        Ok(id)
    }

    /// [`spawn`](Self::spawn) that also attaches the moved record, journaling
    /// one `Spawned`. Crate-visible only: the area transfer's destination
    /// spawn carries the record through `commit`, so a rejected commit
    /// discards it with the staged clone.
    pub(crate) fn spawn_with_creature(
        &mut self,
        meta: EntityMeta,
        creature: Option<CreatureState>,
    ) -> Result<EntityId, HistoryError> {
        let mut world = self.world.clone();
        let id = world.spawn(meta);
        if let Some(creature) = creature {
            world.creatures_mut().insert(id, creature);
        }
        let drained = world.events_mut().drain();
        let tick = world.tick();
        debug_assert_eq!(drained.len(), 1, "spawn enqueues exactly one event");
        let mut events = Vec::with_capacity(drained.len());
        for envelope in &drained {
            events.push(translate_legacy(&envelope.payload));
        }
        self.commit(world, events, tick)?;
        Ok(id)
    }

    /// Removes `entity` and journals `Despawned` plus any resulting turn or
    /// terminal transition the removal branch actually took.
    ///
    /// An absent id returns `Ok(false)` with no event, even when the journal
    /// is full. Removing the last participant of a previously nonterminal
    /// encounter journals one `EncounterEnded`; removing the last of an
    /// already terminal encounter journals no second end.
    pub fn despawn(&mut self, entity: EntityId) -> Result<bool, HistoryError> {
        let mut world = self.world.clone();
        let (removed, facts) = world.despawn_tracked(entity);
        let drained = world.events_mut().drain();
        let tick = world.tick();
        debug_assert!(drained.len() <= 1, "despawn enqueues at most one event");
        let mut events = Vec::with_capacity(drained.len() + 1);
        for envelope in &drained {
            events.push(translate_legacy(&envelope.payload));
        }
        push_transition(&mut events, &facts);
        self.commit(world, events, tick)?;
        Ok(removed)
    }

    /// One fixed simulation step on the staged world, journaling what the
    /// real-time encounter stages observed (T138b, ADR-0029 N5).
    ///
    /// Order: `ModeChanged`, `RoundStarted`, `TurnStarted` (whichever
    /// occurred); then, in execution order, each `ActionResolved` followed
    /// at once by `Died` if that action killed, and each `OrderEnded`; last,
    /// one `EncounterEnded` if the pass made the encounter terminal. A
    /// turn-based, turn-based-only or no-encounter tick journals nothing, and
    /// zero-event operations need no free journal slot or sequence. A tick
    /// whose events exceed a cap commits nothing, not even the clock.
    pub fn tick(&mut self) -> Result<(), HistoryError> {
        let mut world = self.world.clone();
        let facts = crate::tick::tick_tracked(&mut world);
        let drained = world.events_mut().drain();
        let tick = world.tick();
        let mut events = Vec::with_capacity(drained.len() + facts.events.len() + 3);
        push_lead(&mut events, &facts.transitions);
        if let Some((actor, round)) = facts.transitions.turn_started {
            events.push(HistoryEvent::TurnStarted {
                actor,
                round: u64::from(round),
            });
        }
        let mut deaths = drained.iter();
        for fact in &facts.events {
            match fact {
                TickEvent::Resolved(produced) => {
                    events.push(resolved_event(produced));
                    if produced.target_died {
                        let died = deaths
                            .next()
                            .expect("each killing resolution enqueued exactly one `Died`");
                        events.push(translate_legacy(&died.payload));
                    }
                }
                TickEvent::OrderEnded { actor } => {
                    events.push(HistoryEvent::OrderEnded { actor: *actor });
                }
            }
        }
        for rest in deaths {
            debug_assert!(false, "every drained `Died` follows a killing resolution");
            events.push(translate_legacy(&rest.payload));
        }
        if let Some((encounter, round)) = facts.transitions.encounter_ended {
            events.push(HistoryEvent::EncounterEnded {
                encounter,
                round: u64::from(round),
            });
        }
        self.commit(world, events, tick)?;
        Ok(())
    }

    /// Validates and publishes one encounter, journaling one `Spawned` per
    /// participant in authored order plus the initial `TurnStarted`.
    ///
    /// Gameplay errors win before history errors and commit nothing.
    pub fn start_encounter(&mut self, spec: &EncounterSpec<'_>) -> Result<(), HistoryError> {
        self.start_encounter_in_mode(spec, EncounterMode::TurnBased)
    }

    /// [`start_encounter`](Self::start_encounter) in the host-chosen `mode`
    /// (ADR-0029 T11). A real-time start journals only its `Spawned` events:
    /// it schedules no initial turn.
    pub fn start_encounter_in_mode(
        &mut self,
        spec: &EncounterSpec<'_>,
        mode: EncounterMode,
    ) -> Result<(), HistoryError> {
        let mut world = self.world.clone();
        let facts = match start_encounter_in_mode_tracked(&mut world, spec, mode) {
            Ok(facts) => facts,
            Err(error) => return Err(HistoryError::Combat(error)),
        };
        let drained = world.events_mut().drain();
        let tick = world.tick();
        let mut events = Vec::with_capacity(drained.len() + 1);
        for envelope in &drained {
            events.push(translate_legacy(&envelope.payload));
        }
        if let Some((actor, round)) = facts.initial_turn {
            events.push(HistoryEvent::TurnStarted {
                actor,
                round: u64::from(round),
            });
        }
        self.commit(world, events, tick)?;
        Ok(())
    }

    /// Applies one authoritative action, journaling `ActionResolved`, each
    /// actual `Died` in legacy drain order, then the actual `TurnStarted`
    /// or first `EncounterEnded`, if either occurred.
    ///
    /// The reported damage is the returned outcome's flat amount — possibly
    /// overkill on a killing blow, zero on failure — never an inferred
    /// health difference. A legal failed attack journals its resolution too
    /// and consumes its action; a rejected action commits nothing, including
    /// RNG draws and event sequence. An accepted `EndTurn` journals only its
    /// actual turn or terminal transition.
    pub fn perform_action(
        &mut self,
        action: &CombatAction,
    ) -> Result<Option<ActionOutcome>, HistoryError> {
        let mut world = self.world.clone();
        let (outcome, facts) = match perform_action_tracked(&mut world, action) {
            Ok(done) => done,
            Err(error) => return Err(HistoryError::Combat(error)),
        };
        let drained = world.events_mut().drain();
        let tick = world.tick();
        let mut events = Vec::with_capacity(drained.len() + 2);
        if let Some(ref produced) = outcome {
            events.push(resolved_event(produced));
        }
        for envelope in &drained {
            events.push(translate_legacy(&envelope.payload));
        }
        push_transition(&mut events, &facts);
        self.commit(world, events, tick)?;
        Ok(outcome)
    }

    /// Releases the active encounter with its retained summary, journaling
    /// nothing: explicit release is not natural completion.
    ///
    /// Zero-event operations need no free journal slot or sequence.
    pub fn end_encounter(&mut self) -> Result<EncounterSummary, HistoryError> {
        let mut world = self.world.clone();
        let summary = match crate::combat::end_encounter(&mut world) {
            Ok(summary) => summary,
            Err(error) => return Err(HistoryError::Combat(error)),
        };
        let drained = world.events_mut().drain();
        let tick = world.tick();
        let mut events = Vec::with_capacity(drained.len());
        for envelope in &drained {
            events.push(translate_legacy(&envelope.payload));
        }
        debug_assert!(events.is_empty(), "release emits no legacy events");
        self.commit(world, events, tick)?;
        Ok(summary)
    }

    /// A journaled mode request (ADR-0029 T9, T16); emits no event.
    ///
    /// Stages the request, runs [`request_mode`](crate::request_mode) and
    /// commits zero events, so it needs no free journal slot or sequence.
    /// Gameplay errors win and commit nothing.
    pub fn request_mode(&mut self, mode: EncounterMode) -> Result<(), HistoryError> {
        let mut world = self.world.clone();
        if let Err(error) = crate::combat::request_mode(&mut world, mode) {
            return Err(HistoryError::Combat(error));
        }
        let drained = world.events_mut().drain();
        let tick = world.tick();
        debug_assert!(drained.is_empty(), "a mode request emits no legacy events");
        self.commit(world, Vec::new(), tick)?;
        Ok(())
    }

    /// The acknowledgement watermark: every retained sequence exceeds it.
    pub fn acknowledged(&self) -> u64 {
        self.acknowledged
    }

    /// The last issued sequence, `next_seq - 1` even after acknowledgement.
    pub fn last_sequence(&self) -> u64 {
        debug_assert!(self.next_seq >= 1, "the next sequence is never zero");
        self.next_seq - 1
    }

    /// The number of retained envelopes.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// The checked sum of each retained envelope's compact canonical byte
    /// length: struct declaration order with the existing core serde, no
    /// `Vec` brackets or commas. Derived, never persisted, and O(1): it
    /// reads the journal's cached total instead of re-encoding anything.
    pub fn pending_bytes(&self) -> usize {
        self.pending.bytes()
    }

    /// Reads up to `limit` cloned envelopes with sequences above `after`, in
    /// order.
    ///
    /// The limit must lie in `1..=256`; then `after` below the watermark is
    /// [`HistoryError::StaleCursor`] and `after` past the last sequence is
    /// [`HistoryError::FutureCursor`]. `after` equal to the last sequence
    /// returns empty. Reading is inert: it advances no cursor and grants no
    /// automatic acknowledgement.
    pub fn read_after(
        &self,
        after: u64,
        limit: usize,
    ) -> Result<Vec<HistoryEnvelope>, HistoryError> {
        if limit == 0 || limit > MAX_HISTORY_PAGE {
            return Err(HistoryError::InvalidPageLimit { limit });
        }
        if after < self.acknowledged {
            return Err(HistoryError::StaleCursor {
                requested: after,
                acknowledged: self.acknowledged,
            });
        }
        let last = self.last_sequence();
        if after > last {
            return Err(HistoryError::FutureCursor {
                requested: after,
                last,
            });
        }
        let skip = (after - self.acknowledged) as usize;
        Ok(self
            .pending
            .as_slice()
            .iter()
            .skip(skip)
            .take(limit)
            .cloned()
            .collect())
    }

    /// Advances the watermark past every sequence through `through`,
    /// removing the acknowledged prefix.
    ///
    /// Acknowledgements past the last sequence fail; at or below the
    /// watermark they are an idempotent no-op, never a stale error.
    /// Unread events may be acknowledged because capture is the host's
    /// obligation — T022 captures before acknowledging — not an extra
    /// persisted "last read" cursor.
    pub fn acknowledge(&mut self, through: u64) -> Result<(), HistoryError> {
        let last = self.last_sequence();
        if through > last {
            return Err(HistoryError::FutureCursor {
                requested: through,
                last,
            });
        }
        if through <= self.acknowledged {
            return Ok(());
        }
        let drop = (through - self.acknowledged) as usize;
        self.pending.drain_prefix(drop);
        self.acknowledged = through;
        Ok(())
    }

    /// Validates staged journal additions in pinned order — event strings,
    /// then sequence range, then event count, then canonical byte total —
    /// assigns sequences, and commits the staged world plus the appended
    /// envelopes. Failure commits nothing.
    ///
    /// The byte stage starts from the live journal's cached total and encodes
    /// only the new envelopes, so it costs O(new envelopes) and never
    /// re-encodes a retained one; a zero-event operation encodes nothing.
    fn commit(
        &mut self,
        world: World,
        events: Vec<HistoryEvent>,
        tick: Tick,
    ) -> Result<(), HistoryError> {
        for event in &events {
            if let HistoryEvent::ActionResolved { outcome, .. } = event {
                if outcome.len() > MAX_HISTORY_STRING_BYTES {
                    return Err(HistoryError::StringLimit);
                }
                debug_assert!(
                    is_valid_outcome(outcome),
                    "the controller maps only the symbolic outcome vocabulary"
                );
            }
        }
        let count = events.len() as u64;
        let end = self
            .next_seq
            .checked_add(count)
            .ok_or(HistoryError::SequenceExhausted)?;
        let retained = self
            .pending
            .len()
            .checked_add(events.len())
            .ok_or(HistoryError::EventLimit)?;
        if retained > MAX_HISTORY_EVENTS {
            return Err(HistoryError::EventLimit);
        }
        let mut bytes = self.pending.bytes();
        let mut envelopes = Vec::with_capacity(events.len());
        let mut seq = self.next_seq;
        for event in events {
            let envelope = HistoryEnvelope {
                tick,
                seq,
                payload: event,
            };
            let encoded = envelope_len(&envelope);
            bytes = bytes.checked_add(encoded).ok_or(HistoryError::ByteLimit)?;
            if bytes > MAX_HISTORY_BYTES {
                return Err(HistoryError::ByteLimit);
            }
            envelopes.push(envelope);
            seq = seq.checked_add(1).ok_or(HistoryError::SequenceExhausted)?;
        }
        // D16: the first commit that stages an RT-capable encounter or
        // journals a v2 event upgrades the wrapper, for good. Rejected
        // commits returned above, so a failed operation never upgrades.
        if self.version < HISTORY_VERSION
            && (world.combat().is_some_and(|state| state.clock.is_some())
                || envelopes.iter().any(|envelope| envelope.payload.is_v2()))
        {
            self.version = HISTORY_VERSION;
        }
        self.world = world;
        self.pending.append(envelopes, bytes);
        self.next_seq = end;
        Ok(())
    }
}

impl Default for HistoryWorld {
    /// `new(0)`: for tests and tooling that own no seed.
    fn default() -> Self {
        Self::new(0)
    }
}

/// BLAKE3 over the complete canonical wrapper JSON bytes: `[u8; 32]`.
///
/// Canonical means exactly what the wrapper serde impls produce — `version`,
/// `world`, `acknowledged`, `next_seq`, `pending` in declaration order, with
/// the legacy world bytes inline rather than an opaque world hash — so every
/// serialized authoritative field, including pending history and
/// acknowledgement state, is covered with no exclusions. The consumer ack
/// schedule is therefore part of the deterministic input. Non-finite floats
/// are rejected up front for the same `null`-collision reason as
/// [`state_hash`](crate::state_hash); fix the producer instead of scrubbing.
pub fn history_hash(world: &HistoryWorld) -> [u8; 32] {
    for (_, transform) in world.world.transforms().iter() {
        for value in transform.position.iter().chain(transform.velocity.iter()) {
            assert!(
                value.is_finite(),
                "world state holds only finite floats; see doc comment"
            );
        }
    }
    let bytes =
        serde_json::to_vec(world).expect("history serialization must not fail after finite check");
    blake3::hash(&bytes).into()
}

/// Translates one drained legacy event into its history counterpart.
fn translate_legacy(event: &SimEvent) -> HistoryEvent {
    match event {
        SimEvent::Spawned { entity } => HistoryEvent::Spawned { entity: *entity },
        SimEvent::Despawned { entity } => HistoryEvent::Despawned { entity: *entity },
        SimEvent::Died { entity } => HistoryEvent::Died { entity: *entity },
    }
}

/// The `ActionResolved` event for one resolved ability use.
fn resolved_event(produced: &ActionOutcome) -> HistoryEvent {
    HistoryEvent::ActionResolved {
        actor: produced.actor,
        target: produced.target,
        ability: produced.ability,
        outcome: outcome_text(produced.outcome),
        damage: produced.damage,
    }
}

/// Appends the mode switch and the real-time round start, in that order, when
/// the operation took them (T138b, H2).
fn push_lead(events: &mut Vec<HistoryEvent>, facts: &TransitionFacts) {
    if let Some((mode, round)) = facts.mode_changed {
        events.push(HistoryEvent::ModeChanged {
            mode,
            round: u64::from(round),
        });
    }
    if let Some(round) = facts.round_started {
        events.push(HistoryEvent::RoundStarted {
            round: u64::from(round),
        });
    }
}

/// Appends the branch-observed trailing transition: `ModeChanged`,
/// `RoundStarted`, then an actual `TurnStarted`, else the first
/// `EncounterEnded`, if any occurred.
fn push_transition(events: &mut Vec<HistoryEvent>, facts: &TransitionFacts) {
    push_lead(events, facts);
    if let Some((actor, round)) = facts.turn_started {
        debug_assert!(
            facts.encounter_ended.is_none(),
            "one operation takes one trailing transition"
        );
        events.push(HistoryEvent::TurnStarted {
            actor,
            round: u64::from(round),
        });
    } else if let Some((encounter, round)) = facts.encounter_ended {
        events.push(HistoryEvent::EncounterEnded {
            encounter,
            round: u64::from(round),
        });
    }
}

/// Maps one kernel outcome to its exact symbolic history text.
fn outcome_text(outcome: Outcome) -> String {
    match outcome {
        Outcome::CriticalSuccess => "critical_success".to_owned(),
        Outcome::Success => "success".to_owned(),
        Outcome::Failure => "failure".to_owned(),
        Outcome::CriticalFailure => "critical_failure".to_owned(),
        Outcome::Custom(byte) => format!("custom:{byte}"),
    }
}

/// Reports whether `text` names the exact symbolic outcome vocabulary:
/// the four fixed words, or `custom:<n>` with decimal `n` in `0..=255` and
/// no leading zeros.
fn is_valid_outcome(text: &str) -> bool {
    match text {
        "critical_success" | "success" | "failure" | "critical_failure" => true,
        _ => {
            let Some(rest) = text.strip_prefix("custom:") else {
                return false;
            };
            if rest.is_empty() || rest.len() > 3 {
                return false;
            }
            if !rest.bytes().all(|byte| byte.is_ascii_digit()) {
                return false;
            }
            if rest.len() > 1 && rest.starts_with('0') {
                return false;
            }
            match rest.parse::<u32>() {
                Ok(number) => number <= 255,
                Err(_) => false,
            }
        }
    }
}

// Test-only count of [`envelope_len`] calls on the current test thread.
//
// The CI regression guard for T097 is a count of canonical encodings, never a
// wall clock: it proves a mutation's byte cost is proportional to its new
// envelopes rather than to the retained journal. It is a per-test-thread
// counter (`thread_local!`), not a thread the simulation owns, and it does not
// exist in any non-test build.
#[cfg(test)]
thread_local! {
    // Encodings this thread has performed since it last reset the counter.
    static ENCODINGS: Cell<u64> = const { Cell::new(0) };
}

/// One envelope's compact canonical byte length, the unit of journal byte
/// accounting.
///
/// Every call is an encoding ([`ENCODINGS`] counts them under `#[cfg(test)]`),
/// so callers must encode an envelope exactly once: the loader's inline encode
/// is deliberately not routed through here, and the retained journal's total
/// is cached rather than re-derived.
fn envelope_len(envelope: &HistoryEnvelope) -> usize {
    #[cfg(test)]
    ENCODINGS.with(|count| count.set(count.get() + 1));
    serde_json::to_vec(envelope)
        .expect("history envelopes serialize")
        .len()
}

/// One core-closed entity reference inside a history payload.
///
/// Deserialization is manual so a duplicate `entity` key fails instead of
/// silently keeping the last value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EntityRef {
    /// The referenced entity; historical references may name despawned
    /// generations and are never liveness-checked.
    entity: EntityId,
}

impl<'de> Deserialize<'de> for EntityRef {
    /// Decodes one entity reference, rejecting duplicate/unknown/missing keys.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(EntityRefVisitor)
    }
}

/// The [`EntityRef`] map visitor.
struct EntityRefVisitor;

impl<'de> Visitor<'de> for EntityRefVisitor {
    type Value = EntityRef;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a history entity reference with `entity`")
    }

    fn visit_map<A>(self, mut map: A) -> Result<EntityRef, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entity: Option<EntityId> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "entity" => {
                    if entity.is_some() {
                        return Err(A::Error::custom("duplicate history payload field `entity`"));
                    }
                    entity = Some(map.next_value()?);
                }
                _ => {
                    return Err(A::Error::custom("unknown history payload field"));
                }
            }
        }
        let entity = entity.ok_or_else(|| A::Error::custom("history payload misses `entity`"))?;
        Ok(EntityRef { entity })
    }
}

/// The decoded `action_resolved` payload before retention checks.
///
/// Deserialization is manual so duplicate keys fail instead of silently
/// keeping the last value. The outcome still decodes through
/// [`deserialize_bounded_outcome`], so an oversized outcome fails while its
/// field decodes, before the trailing `damage` field and before any
/// retained copy.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ActionResolvedFields {
    /// The acting combatant.
    actor: EntityId,
    /// The targeted combatant.
    target: EntityId,
    /// The ability that was used.
    ability: Ulid,
    /// The symbolic outcome text, length-checked during decoding before
    /// any retained copy is made.
    outcome: String,
    /// The reported flat damage.
    damage: u32,
}

/// One bounded history outcome string: length-checked during field decoding.
struct BoundedOutcome(String);

impl<'de> Deserialize<'de> for BoundedOutcome {
    /// Decodes through [`deserialize_bounded_outcome`].
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_bounded_outcome(deserializer).map(BoundedOutcome)
    }
}

impl<'de> Deserialize<'de> for ActionResolvedFields {
    /// Decodes one payload, rejecting duplicate/unknown/missing keys.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(ActionResolvedVisitor)
    }
}

/// The [`ActionResolvedFields`] map visitor: duplicate-rejecting, order-free.
struct ActionResolvedVisitor;

impl<'de> Visitor<'de> for ActionResolvedVisitor {
    type Value = ActionResolvedFields;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an `action_resolved` payload with its five fields")
    }

    fn visit_map<A>(self, mut map: A) -> Result<ActionResolvedFields, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut actor: Option<EntityId> = None;
        let mut target: Option<EntityId> = None;
        let mut ability: Option<Ulid> = None;
        let mut outcome: Option<String> = None;
        let mut damage: Option<u32> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "actor" => {
                    if actor.is_some() {
                        return Err(A::Error::custom("duplicate history payload field `actor`"));
                    }
                    actor = Some(map.next_value()?);
                }
                "target" => {
                    if target.is_some() {
                        return Err(A::Error::custom("duplicate history payload field `target`"));
                    }
                    target = Some(map.next_value()?);
                }
                "ability" => {
                    if ability.is_some() {
                        return Err(A::Error::custom(
                            "duplicate history payload field `ability`",
                        ));
                    }
                    ability = Some(map.next_value()?);
                }
                "outcome" => {
                    if outcome.is_some() {
                        return Err(A::Error::custom(
                            "duplicate history payload field `outcome`",
                        ));
                    }
                    outcome = Some(map.next_value::<BoundedOutcome>()?.0);
                }
                "damage" => {
                    if damage.is_some() {
                        return Err(A::Error::custom("duplicate history payload field `damage`"));
                    }
                    damage = Some(map.next_value()?);
                }
                _ => {
                    return Err(A::Error::custom("unknown history payload field"));
                }
            }
        }
        let actor = actor.ok_or_else(|| A::Error::custom("history payload misses `actor`"))?;
        let target = target.ok_or_else(|| A::Error::custom("history payload misses `target`"))?;
        let ability =
            ability.ok_or_else(|| A::Error::custom("history payload misses `ability`"))?;
        let outcome =
            outcome.ok_or_else(|| A::Error::custom("history payload misses `outcome`"))?;
        let damage = damage.ok_or_else(|| A::Error::custom("history payload misses `damage`"))?;
        Ok(ActionResolvedFields {
            actor,
            target,
            ability,
            outcome,
            damage,
        })
    }
}

/// Deserializes one history outcome string with its byte length checked
/// before the retained copy is made.
///
/// `visit_str`/`visit_borrowed_str` check the decoded text length before
/// calling `to_owned`, so an oversized unescaped outcome borrowed from the
/// input is rejected without allocating a second retained copy; escaped
/// input may still hold transient parser scratch (bounded by T022's
/// pre-parse host input cap), but no oversized string is ever retained.
/// `visit_string` covers deserializers that hand over ownership (for
/// example `serde_json::Value` in tests) by checking before accepting.
/// Vocabulary validation stays in [`PayloadVisitor`] so both layers report
/// their own failure before the envelope is retained.
fn deserialize_bounded_outcome<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    struct BoundedOutcomeVisitor;

    impl Visitor<'_> for BoundedOutcomeVisitor {
        type Value = String;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a history outcome string of at most 256 bytes")
        }

        fn visit_str<E>(self, value: &str) -> Result<String, E>
        where
            E: DeError,
        {
            if value.len() > MAX_HISTORY_STRING_BYTES {
                return Err(E::custom("history outcome exceeds 256 bytes"));
            }
            Ok(value.to_owned())
        }

        fn visit_borrowed_str<E>(self, value: &str) -> Result<String, E>
        where
            E: DeError,
        {
            if value.len() > MAX_HISTORY_STRING_BYTES {
                return Err(E::custom("history outcome exceeds 256 bytes"));
            }
            Ok(value.to_owned())
        }

        fn visit_string<E>(self, value: String) -> Result<String, E>
        where
            E: DeError,
        {
            if value.len() > MAX_HISTORY_STRING_BYTES {
                return Err(E::custom("history outcome exceeds 256 bytes"));
            }
            Ok(value)
        }
    }

    deserializer.deserialize_str(BoundedOutcomeVisitor)
}

/// The decoded `turn_started` payload.
///
/// Deserialization is manual so a duplicate key fails instead of silently
/// keeping the last value.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TurnStartedFields {
    /// The incoming turn head.
    actor: EntityId,
    /// The widened combat round.
    round: u64,
}

impl<'de> Deserialize<'de> for TurnStartedFields {
    /// Decodes one payload, rejecting duplicate/unknown/missing keys.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(TurnStartedVisitor)
    }
}

/// The [`TurnStartedFields`] map visitor.
struct TurnStartedVisitor;

impl<'de> Visitor<'de> for TurnStartedVisitor {
    type Value = TurnStartedFields;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a `turn_started` payload with `actor` and `round`")
    }

    fn visit_map<A>(self, mut map: A) -> Result<TurnStartedFields, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut actor: Option<EntityId> = None;
        let mut round: Option<u64> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "actor" => {
                    if actor.is_some() {
                        return Err(A::Error::custom("duplicate history payload field `actor`"));
                    }
                    actor = Some(map.next_value()?);
                }
                "round" => {
                    if round.is_some() {
                        return Err(A::Error::custom("duplicate history payload field `round`"));
                    }
                    round = Some(map.next_value()?);
                }
                _ => {
                    return Err(A::Error::custom("unknown history payload field"));
                }
            }
        }
        let actor = actor.ok_or_else(|| A::Error::custom("history payload misses `actor`"))?;
        let round = round.ok_or_else(|| A::Error::custom("history payload misses `round`"))?;
        Ok(TurnStartedFields { actor, round })
    }
}

/// The decoded `encounter_ended` payload.
///
/// Deserialization is manual so a duplicate key fails instead of silently
/// keeping the last value.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EncounterEndedFields {
    /// The naturally completed encounter.
    encounter: Ulid,
    /// The widened closing round.
    round: u64,
}

impl<'de> Deserialize<'de> for EncounterEndedFields {
    /// Decodes one payload, rejecting duplicate/unknown/missing keys.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(EncounterEndedVisitor)
    }
}

/// The [`EncounterEndedFields`] map visitor.
struct EncounterEndedVisitor;

impl<'de> Visitor<'de> for EncounterEndedVisitor {
    type Value = EncounterEndedFields;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an `encounter_ended` payload with `encounter` and `round`")
    }

    fn visit_map<A>(self, mut map: A) -> Result<EncounterEndedFields, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut encounter: Option<Ulid> = None;
        let mut round: Option<u64> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "encounter" => {
                    if encounter.is_some() {
                        return Err(A::Error::custom(
                            "duplicate history payload field `encounter`",
                        ));
                    }
                    encounter = Some(map.next_value()?);
                }
                "round" => {
                    if round.is_some() {
                        return Err(A::Error::custom("duplicate history payload field `round`"));
                    }
                    round = Some(map.next_value()?);
                }
                _ => {
                    return Err(A::Error::custom("unknown history payload field"));
                }
            }
        }
        let encounter =
            encounter.ok_or_else(|| A::Error::custom("history payload misses `encounter`"))?;
        let round = round.ok_or_else(|| A::Error::custom("history payload misses `round`"))?;
        Ok(EncounterEndedFields { encounter, round })
    }
}

/// The decoded `round_started` payload (version 2).
///
/// Deserialization is manual so a duplicate key fails instead of silently
/// keeping the last value.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RoundStartedFields {
    /// The widened real-time round.
    round: u64,
}

impl<'de> Deserialize<'de> for RoundStartedFields {
    /// Decodes one payload, rejecting duplicate/unknown/missing keys.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(RoundStartedVisitor)
    }
}

/// The [`RoundStartedFields`] map visitor.
struct RoundStartedVisitor;

impl<'de> Visitor<'de> for RoundStartedVisitor {
    type Value = RoundStartedFields;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a `round_started` payload with `round`")
    }

    fn visit_map<A>(self, mut map: A) -> Result<RoundStartedFields, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut round: Option<u64> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "round" => {
                    if round.is_some() {
                        return Err(A::Error::custom("duplicate history payload field `round`"));
                    }
                    round = Some(map.next_value()?);
                }
                _ => {
                    return Err(A::Error::custom("unknown history payload field"));
                }
            }
        }
        let round = round.ok_or_else(|| A::Error::custom("history payload misses `round`"))?;
        Ok(RoundStartedFields { round })
    }
}

/// The decoded `mode_changed` payload (version 2).
///
/// Deserialization is manual so a duplicate key fails instead of silently
/// keeping the last value.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ModeChangedFields {
    /// The mode now in force.
    mode: EncounterMode,
    /// The widened round the switch started.
    round: u64,
}

impl<'de> Deserialize<'de> for ModeChangedFields {
    /// Decodes one payload, rejecting duplicate/unknown/missing keys.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(ModeChangedVisitor)
    }
}

/// The [`ModeChangedFields`] map visitor.
struct ModeChangedVisitor;

impl<'de> Visitor<'de> for ModeChangedVisitor {
    type Value = ModeChangedFields;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a `mode_changed` payload with `mode` and `round`")
    }

    fn visit_map<A>(self, mut map: A) -> Result<ModeChangedFields, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut mode: Option<EncounterMode> = None;
        let mut round: Option<u64> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "mode" => {
                    if mode.is_some() {
                        return Err(A::Error::custom("duplicate history payload field `mode`"));
                    }
                    mode = Some(map.next_value()?);
                }
                "round" => {
                    if round.is_some() {
                        return Err(A::Error::custom("duplicate history payload field `round`"));
                    }
                    round = Some(map.next_value()?);
                }
                _ => {
                    return Err(A::Error::custom("unknown history payload field"));
                }
            }
        }
        let mode = mode.ok_or_else(|| A::Error::custom("history payload misses `mode`"))?;
        let round = round.ok_or_else(|| A::Error::custom("history payload misses `round`"))?;
        Ok(ModeChangedFields { mode, round })
    }
}

/// The decoded `order_ended` payload (version 2).
///
/// Deserialization is manual so a duplicate key fails instead of silently
/// keeping the last value. It has its own `actor` visitor: [`EntityRef`]
/// names its key `entity`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OrderEndedFields {
    /// The combatant whose order ended.
    actor: EntityId,
}

impl<'de> Deserialize<'de> for OrderEndedFields {
    /// Decodes one payload, rejecting duplicate/unknown/missing keys.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(OrderEndedVisitor)
    }
}

/// The [`OrderEndedFields`] map visitor.
struct OrderEndedVisitor;

impl<'de> Visitor<'de> for OrderEndedVisitor {
    type Value = OrderEndedFields;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an `order_ended` payload with `actor`")
    }

    fn visit_map<A>(self, mut map: A) -> Result<OrderEndedFields, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut actor: Option<EntityId> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "actor" => {
                    if actor.is_some() {
                        return Err(A::Error::custom("duplicate history payload field `actor`"));
                    }
                    actor = Some(map.next_value()?);
                }
                _ => {
                    return Err(A::Error::custom("unknown history payload field"));
                }
            }
        }
        let actor = actor.ok_or_else(|| A::Error::custom("history payload misses `actor`"))?;
        Ok(OrderEndedFields { actor })
    }
}

impl<'de> Deserialize<'de> for HistoryEvent {
    /// Decodes one adjacent-tagged payload, requiring `type` before `value`.
    ///
    /// The reverse order is rejected before its value is decoded or
    /// buffered; the selected payload then decodes directly with no
    /// intermediate `serde_json::Value` or Serde content buffer. Outcome
    /// byte length is bounded during field decoding (before any retained
    /// copy) and re-checked here, and the outcome vocabulary is validated.
    /// Other payload field order is unrestricted; duplicate, unknown, or
    /// missing payload keys fail.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(PayloadVisitor)
    }
}

/// The [`HistoryEvent`] payload visitor: streaming tag order, direct decode.
struct PayloadVisitor;

impl<'de> Visitor<'de> for PayloadVisitor {
    type Value = HistoryEvent;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an adjacent-tagged history payload with `type` before `value`")
    }

    fn visit_map<A>(self, mut map: A) -> Result<HistoryEvent, A::Error>
    where
        A: MapAccess<'de>,
    {
        let first: Option<String> = map.next_key()?;
        match first.as_deref() {
            Some("type") => {}
            _ => {
                return Err(A::Error::custom(
                    "history payload requires `type` before `value`",
                ));
            }
        }
        let tag: String = map.next_value()?;
        let second: Option<String> = map.next_key()?;
        match second.as_deref() {
            Some("value") => {}
            _ => {
                return Err(A::Error::custom(
                    "history payload requires `value` after `type`",
                ));
            }
        }
        let event = match tag.as_str() {
            "spawned" => {
                let fields: EntityRef = map.next_value()?;
                HistoryEvent::Spawned {
                    entity: fields.entity,
                }
            }
            "despawned" => {
                let fields: EntityRef = map.next_value()?;
                HistoryEvent::Despawned {
                    entity: fields.entity,
                }
            }
            "died" => {
                let fields: EntityRef = map.next_value()?;
                HistoryEvent::Died {
                    entity: fields.entity,
                }
            }
            "action_resolved" => {
                let fields: ActionResolvedFields = map.next_value()?;
                if fields.outcome.len() > MAX_HISTORY_STRING_BYTES {
                    return Err(A::Error::custom("history outcome exceeds 256 bytes"));
                }
                if !is_valid_outcome(&fields.outcome) {
                    return Err(A::Error::custom("history outcome names no known outcome"));
                }
                HistoryEvent::ActionResolved {
                    actor: fields.actor,
                    target: fields.target,
                    ability: fields.ability,
                    outcome: fields.outcome,
                    damage: fields.damage,
                }
            }
            "turn_started" => {
                let fields: TurnStartedFields = map.next_value()?;
                HistoryEvent::TurnStarted {
                    actor: fields.actor,
                    round: fields.round,
                }
            }
            "encounter_ended" => {
                let fields: EncounterEndedFields = map.next_value()?;
                HistoryEvent::EncounterEnded {
                    encounter: fields.encounter,
                    round: fields.round,
                }
            }
            "round_started" => {
                let fields: RoundStartedFields = map.next_value()?;
                HistoryEvent::RoundStarted {
                    round: fields.round,
                }
            }
            "mode_changed" => {
                let fields: ModeChangedFields = map.next_value()?;
                HistoryEvent::ModeChanged {
                    mode: fields.mode,
                    round: fields.round,
                }
            }
            "order_ended" => {
                let fields: OrderEndedFields = map.next_value()?;
                HistoryEvent::OrderEnded {
                    actor: fields.actor,
                }
            }
            _ => return Err(A::Error::custom("unknown history event type")),
        };
        if map.next_key::<String>()?.is_some() {
            return Err(A::Error::custom("unknown history payload field"));
        }
        Ok(event)
    }
}

impl<'de> Deserialize<'de> for HistoryWorld {
    /// Decodes and validates one wrapper: exact key set, known version,
    /// ordered journal bounds, contiguous retained sequences, monotone ticks
    /// within the world tick, and an empty inner legacy queue.
    ///
    /// Retained journal storage and collection growth stay bounded: the
    /// pending visitor trusts no collection size hint, rejects an excess
    /// entry before decoding its envelope, and checks canonical byte totals
    /// incrementally before retaining each envelope. See the module
    /// documentation for what these checks do not bound.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(WrapperVisitor)
    }
}

/// The [`HistoryWorld`] wrapper visitor.
struct WrapperVisitor;

impl<'de> Visitor<'de> for WrapperVisitor {
    type Value = HistoryWorld;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a versioned authoritative history wrapper")
    }

    fn visit_map<A>(self, mut map: A) -> Result<HistoryWorld, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut version: Option<u32> = None;
        let mut world: Option<World> = None;
        let mut acknowledged: Option<u64> = None;
        let mut next_seq: Option<u64> = None;
        let mut pending: Option<Journal> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "version" => {
                    if version.is_some() {
                        return Err(A::Error::custom(
                            "duplicate history wrapper field `version`",
                        ));
                    }
                    version = Some(map.next_value()?);
                }
                "world" => {
                    if world.is_some() {
                        return Err(A::Error::custom("duplicate history wrapper field `world`"));
                    }
                    world = Some(map.next_value()?);
                }
                "acknowledged" => {
                    if acknowledged.is_some() {
                        return Err(A::Error::custom(
                            "duplicate history wrapper field `acknowledged`",
                        ));
                    }
                    acknowledged = Some(map.next_value()?);
                }
                "next_seq" => {
                    if next_seq.is_some() {
                        return Err(A::Error::custom(
                            "duplicate history wrapper field `next_seq`",
                        ));
                    }
                    next_seq = Some(map.next_value()?);
                }
                "pending" => {
                    if pending.is_some() {
                        return Err(A::Error::custom(
                            "duplicate history wrapper field `pending`",
                        ));
                    }
                    pending = Some(map.next_value_seed(PendingSeed)?);
                }
                _ => {
                    return Err(A::Error::custom("unknown history wrapper field"));
                }
            }
        }
        let version =
            version.ok_or_else(|| A::Error::custom("history wrapper misses `version`"))?;
        let world = world.ok_or_else(|| A::Error::custom("history wrapper misses `world`"))?;
        let acknowledged = acknowledged
            .ok_or_else(|| A::Error::custom("history wrapper misses `acknowledged`"))?;
        let next_seq =
            next_seq.ok_or_else(|| A::Error::custom("history wrapper misses `next_seq`"))?;
        let pending =
            pending.ok_or_else(|| A::Error::custom("history wrapper misses `pending`"))?;
        if version != HISTORY_VERSION_V1 && version != HISTORY_VERSION {
            return Err(A::Error::custom("unknown history version"));
        }
        if version == HISTORY_VERSION_V1 {
            // Payloads decode with both vocabularies whatever the version
            // (`version` may follow `pending`), so a v1 wrapper is judged
            // here, after every key is read.
            if pending
                .as_slice()
                .iter()
                .any(|envelope| envelope.payload.is_v2())
            {
                return Err(A::Error::custom("history v1 journal holds a v2 event"));
            }
            if world.combat().is_some_and(|state| state.clock.is_some()) {
                return Err(A::Error::custom(
                    "history v1 wrapper holds a real-time-capable encounter",
                ));
            }
        }
        if next_seq == 0 {
            return Err(A::Error::custom("history next sequence must not be zero"));
        }
        if acknowledged >= next_seq {
            return Err(A::Error::custom(
                "history acknowledgement must precede its next sequence",
            ));
        }
        if !world.events().is_empty() {
            return Err(A::Error::custom(
                "history wrapper holds a nonempty inner event queue",
            ));
        }
        validate_journal(pending.as_slice(), acknowledged, next_seq, world.tick())?;
        Ok(HistoryWorld {
            version,
            world,
            acknowledged,
            next_seq,
            pending,
        })
    }
}

/// Validates retained sequences against the acknowledgement watermark and
/// the next sequence, and ticks against their monotone world bound.
fn validate_journal<E: DeError>(
    pending: &[HistoryEnvelope],
    acknowledged: u64,
    next_seq: u64,
    tick: Tick,
) -> Result<(), E> {
    if pending.is_empty() {
        if acknowledged.checked_add(1) != Some(next_seq) {
            return Err(E::custom("history journal is not a contiguous suffix"));
        }
        return Ok(());
    }
    let mut expected = acknowledged
        .checked_add(1)
        .ok_or_else(|| E::custom("history sequence overflows"))?;
    let mut previous: Option<Tick> = None;
    for envelope in pending {
        if envelope.seq != expected {
            return Err(E::custom("history journal is not a contiguous suffix"));
        }
        if envelope.seq == 0 || envelope.seq == u64::MAX {
            return Err(E::custom("history sequence out of range"));
        }
        if let Some(before) = previous {
            if envelope.tick < before {
                return Err(E::custom("history ticks decrease"));
            }
        }
        if envelope.tick > tick {
            return Err(E::custom("history event ticks after its world"));
        }
        previous = Some(envelope.tick);
        expected = expected
            .checked_add(1)
            .ok_or_else(|| E::custom("history sequence overflows"))?;
    }
    if expected != next_seq {
        return Err(E::custom("history journal is not a contiguous suffix"));
    }
    Ok(())
}

/// The pending-journal seed: bounded element decoding without size hints.
struct PendingSeed;

impl<'de> DeserializeSeed<'de> for PendingSeed {
    type Value = Journal;

    fn deserialize<D>(self, deserializer: D) -> Result<Journal, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(PendingSeqVisitor)
    }
}

/// The pending-journal sequence visitor.
struct PendingSeqVisitor;

impl<'de> Visitor<'de> for PendingSeqVisitor {
    type Value = Journal;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded history journal")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Journal, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut out: Vec<HistoryEnvelope> = Vec::new();
        let mut bytes: usize = 0;
        let count = Cell::new(0usize);
        while let Some(envelope) = seq.next_element_seed(CappedSeed { count: &count })? {
            let encoded = serde_json::to_vec(&envelope).map_err(A::Error::custom)?;
            bytes = bytes
                .checked_add(encoded.len())
                .ok_or_else(|| A::Error::custom("history journal exceeds its byte budget"))?;
            if bytes > MAX_HISTORY_BYTES {
                return Err(A::Error::custom("history journal exceeds its byte budget"));
            }
            out.push(envelope);
        }
        Ok(Journal::from_loaded(out, bytes))
    }
}

/// One pending-envelope seed: rejects the excess entry before decoding it.
///
/// The seed runs only when an element is present, so the count guard fires
/// before any over-limit envelope is decoded or retained.
struct CappedSeed<'a> {
    /// Envelopes decoded so far; shared by borrow across seed invocations.
    count: &'a Cell<usize>,
}

impl<'de> DeserializeSeed<'de> for CappedSeed<'_> {
    type Value = HistoryEnvelope;

    fn deserialize<D>(self, deserializer: D) -> Result<HistoryEnvelope, D::Error>
    where
        D: Deserializer<'de>,
    {
        if self.count.get() >= MAX_HISTORY_EVENTS {
            return Err(D::Error::custom("history journal exceeds 4096 events"));
        }
        self.count.set(self.count.get() + 1);
        HistoryEnvelope::deserialize(deserializer)
    }
}

#[cfg(test)]
mod tests;
