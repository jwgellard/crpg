# T140-draft-a — blind draft: the next `crpg-net` protocol version (v3)

Status: **draft for adjudication, 2026-11-10.** Not approved. A Claude architect
adjudicates it against the other blind draft; the user approves the result.
Nothing here is a claim about code that exists.

## Task

`crpg-net` gains protocol version 3: ADR-0029 N1's world and host facts, the
N5 events, the six intents, the codes T138/T139/T137 deferred, a v3 snapshot
body, per-field grants for every new fact, and the N4 heartbeat. v1, v2, the
`0x81` transfer and the `0x91` resume subprotocols stay frozen byte for byte,
and `crpg-net` still reads no `World`. T141 owns every `crpg-server` value.

## Crate(s)

Sole crate: `crpg-net`; T141 owns the `crpg-server` projection. Three
implementer runs, in dependency order.

- **T140a — protocol and codec.** Owns `crates/crpg-net/src/protocol_v3.rs`,
  `crates/crpg-net/src/codec_v3.rs`, `crates/crpg-net/tests/codec_v3.rs`.
  `goldens:` no existing golden moves; `codec_v3` adds v3 intent/delta literal
  bytes. v1/v2 pinned bytes must not move because no v1/v2 module is edited.
- **T140b — projection and interest.** Owns
  `crates/crpg-net/src/projection_v3.rs`, `crates/crpg-net/src/interest_v3.rs`,
  `crates/crpg-net/tests/events_v3.rs`, `crates/crpg-net/tests/interest_v3.rs`.
  `goldens:` `tests/events_v2.rs` and `tests/interest.rs` literal pages must
  not move; v3 adds literal pages in its own suites.
- **T140c — v3 snapshot and docs.** Owns
  `crates/crpg-net/src/protocol_transfer_v3.rs`,
  `crates/crpg-net/src/codec_transfer_v3.rs`,
  `crates/crpg-net/tests/snapshot_transfer_v3.rs`,
  `docs/architecture/crpg-net.md`, `crates/crpg-net/AGENTS.md`.
  `goldens:` `tests/snapshot_transfer.rs` and
  `tests/snapshot_transfer_contract.rs` (`0x81`) and `tests/reconnect_wire.rs`
  (`0x91`) must not move; v3 adds literal `0x82` frames and a body-size golden.

## Readings of the ADR

N3 requires one version to add all of N1 after T024's, with v1, v2 and T024
frozen (`docs/adr/0029-kernel-range-and-time-model.md:338-340`).

1. **Byte 3, new modules.** Versions today are 1
   (`crates/crpg-net/src/protocol.rs:25`) and 2
   (`crates/crpg-net/src/protocol_v2.rs:34`); the next is 3 in
   `protocol_v3`/`codec_v3`. *Rejected:* extend v2 — its bytes and fixtures are
   frozen (`docs/architecture/crpg-net.md:213-217`). *Rejected:* negotiate —
   the codecs refuse each other and there is no autodetect
   (`crates/crpg-net/src/protocol_v2.rs:11-15`).
2. **`Legacy` wraps v2; new tags start at 11.** v3
   `DeltaOp::Legacy(v2::DeltaOp)` re-encodes tags 0..10 unchanged, as v2's
   `Legacy(v1::DeltaOp)` did (`crates/crpg-net/src/protocol_v2.rs:88-90`).
   *Rejected:* re-declare the old ops — every tag would need a mapping table
   and "v2 bytes live inside v3" would be untestable.
3. **One `Encounter` op replaces `Turn`.** It carries mode, pending mode,
   round, clock and active with one presence bit each. *Rejected:* extend v1
   `Turn { active, round }` (`crates/crpg-net/src/protocol.rs:347-352`) — its
   meaning is frozen and two round facts could disagree. *Rejected:* one op
   per field — five ops per round boundary would eat `MAX_PAGE_OPS`
   (`crates/crpg-net/src/interest.rs:21`).
4. **The timeline is its own op**, ascending `(key, entity)`, key = raw `i32`
   of `InitiativeKey` (`crates/crpg-sim/src/timeline.rs:25`, `:127-130`).
   *Rejected:* fold it into `Encounter` — one hidden participant would
   suppress the whole order, and presence is a per-field grant
   (`crates/crpg-net/src/interest.rs:39-46`). *Rejected:* send hidden keys —
   that discloses presence.
5. **One `EntityFacts` op per visible entity**, with optional pools, effects,
   order, stats and tags. *Rejected:* one op per family — five per entity
   exceeds the 3 × `MAX_VISIBLE_ENTITIES` + 1 page envelope
   (`crates/crpg-net/src/interest.rs:21`). *Rejected:* an open bag — forbidden
   by `crates/crpg-net/AGENTS.md:52-54` and ADR-0011.
6. **Small element bounds keep the maximal body under 1 MiB.**
   `MAX_V3_POOLS = 4`, `MAX_V3_EFFECTS = 4`, `MAX_V3_STATS = 8`,
   `MAX_V3_TAGS = 4`, names/ids ≤ 32 B; the worst-case body from the v3 layout
   is 843,821 B, under `MAX_SNAPSHOT_BYTES`
   (`crates/crpg-net/src/protocol.rs:89`) and 32 × 32 KiB
   (`crates/crpg-net/src/protocol_transfer.rs:56-57`). *Rejected:* 16/64/64/32
   — ≈ 1.57 MB, which would force raising the frozen transfer envelope.
7. **Codes are versioned.** v3 owns `protocol_v3::RejectionCode` 1..=31 and
   its own `Receipt` op (tag 21); the legacy receipt (tag 7) still decodes
   1..=20. *Rejected:* widen v1's enum
   (`crates/crpg-net/src/protocol.rs:391-435`) — v1/v2 `from_u8` would accept
   21+, and `frozen_wire_values_pinned` (`crates/crpg-net/tests/codec.rs:178-207`)
   pins 1..=20.
8. **Exactly the deferred codes:** `ModeUnsupported`, `WrongMode`
   (`tasks/T138.md:866`); `EncounterTerminal`, `RulesetMismatch`,
   `PlacementInUse`, `NotCreature`, `AlreadyParticipant`, `NoEntrants`
   (`tasks/T139.md:1914-1916`); `StatEvaluation` (`tasks/T137.md:823`); and
   `PauseDenied`/`UnpauseDenied` for P4's independent rights
   (`docs/adr/0029-kernel-range-and-time-model.md:290-297`). *Rejected:* one
   `NoPauseRight` — the rights differ. *Rejected:* a `Paused` code — commands
   run while paused (`:284-286`).
9. **The v3 snapshot is transfer `0x82` on lane 2**, reusing the `0x81`
   framing, caps and `ReassemblyPool`; only the body type changes
   (`crates/crpg-net/src/protocol_transfer.rs:19`,
   `docs/architecture/crpg-net.md:362-368`). *Rejected:* extend the `0x81`
   body — T024 froze its fields and literal bytes (`tasks/T024.md:321-335`).
   *Rejected:* a new lane — lane 2 selects snapshots and the first byte selects
   their version.
10. **Snapshot and delta facts are one set; host facts are not snapshot
    facts.** Pause is host state, not hashed
    (`docs/adr/0029-kernel-range-and-time-model.md:281-282`, `:327-329`).
    A new viewer gets `HostFacts` and `Clock` as ordinary ops after
    publication. *Rejected:* pause in the body — stale, and per-viewer rights
    in a shared body.
11. **`Joined`/`Left` get notice tags 17/18; `Moved` does not.** T139 left
    the first two to T140 (`tasks/T139.md:1914-1916`). *Rejected:* fold
    movement in although ADR-0036 L6 allows it
    (`docs/adr/0036-movement-and-navigation.md:413-417`) — it is lane 1,
    T026p depends on it (`:540`), and this draft owns N1 only.
12. **Heartbeat is a delta op every 100 ticks and on pause change.**
    `Clock { server_tick, paused }` (tag 19), `POLICY_CLOCK_HEARTBEAT_TICKS =
    100`, half the 200-tick window (`crates/crpg-net/src/protocol.rs:159-165`),
    so one loss leaves a fresh observation. *Rejected:* 200 — one loss empties
    the window. *Rejected:* pause changes only — N4 names that failure
    (`docs/adr/0029-kernel-range-and-time-model.md:342-345`).
13. **Host facts are one `HostFacts` op (tag 20):** paused and reason, the
    viewer's two rights, five auto-pause toggles, turn-timer deadline
    (`docs/adr/0029-kernel-range-and-time-model.md:327-329`, `:299-316`).
    *Rejected:* leave the shape to T141 — T140 owns the wire; T141 owns values
    and timing.
14. **v3 re-declares intents; tags 0/1 keep v1 payloads** (`crates/crpg-net/src/protocol.rs:268-279`);
    `Stop`, `Pause`, `Unpause`, `RequestMode` are tags 2..5. *Rejected:* a
    parallel intent message — two decoders on one lane. *Rejected:* a rights
    payload — the host owns rights (`:290-297`).
15. **`crpg-net` encodes host-supplied values only; T141 reads `World`.**
    Sim is a dev-dependency and ADR-0019 refused a runtime codec import
    (`crates/crpg-net/Cargo.toml:15-22`,
    `docs/adr/0019-event-protocol-v2.md:10-14`). *Rejected:* read
    `World::combat()` (`crates/crpg-sim/src/world.rs:1239-1241`) — it moves
    disclosure below the H-filter N2 puts in T027b (`:333-336`).
16. **Roll transparency stays closed.** v3 has no roll, DC or margin field;
    ADR-0019 keeps them off the wire (`docs/adr/0019-event-protocol-v2.md:22-26`)
    and Open question 8 belongs to the user (`:515-519`). *Rejected:* an
    optional `roll` now — an ungranted fact with no H-filter rule.

## Interface

All new public types derive `Debug, Clone, PartialEq, Eq`; no serde, no
restore constructor. Errors are `crate::codec::CodecError` re-exported.

### `protocol_v3` — constants

```rust
pub const PROTOCOL_VERSION: u8 = 3;
pub const DELTA_TAG_ENCOUNTER: u8 = 11;
pub const DELTA_TAG_TIMELINE: u8 = 12;
pub const DELTA_TAG_ENTITY_FACTS: u8 = 13;
pub const DELTA_TAG_ROUND_STARTED: u8 = 14;
pub const DELTA_TAG_MODE_CHANGED: u8 = 15;
pub const DELTA_TAG_ORDER_ENDED: u8 = 16;
pub const DELTA_TAG_JOINED: u8 = 17;
pub const DELTA_TAG_LEFT: u8 = 18;
pub const DELTA_TAG_CLOCK: u8 = 19;
pub const DELTA_TAG_HOST_FACTS: u8 = 20;
pub const DELTA_TAG_RECEIPT: u8 = 21;
pub const INTENT_TAG_DECLARE_ACTION: u8 = 0;
pub const INTENT_TAG_END_TURN: u8 = 1;
pub const INTENT_TAG_STOP: u8 = 2;
pub const INTENT_TAG_PAUSE: u8 = 3;
pub const INTENT_TAG_UNPAUSE: u8 = 4;
pub const INTENT_TAG_REQUEST_MODE: u8 = 5;
pub const MODE_TAG_TURN_BASED: u8 = 0;
pub const MODE_TAG_REAL_TIME: u8 = 1;
pub const MAX_V3_PAGE_OPS: usize = MAX_VISIBLE_ENTITIES + 2;
pub const MAX_V3_TIMELINE_ENTRIES: usize = MAX_VISIBLE_ENTITIES;
pub const MAX_V3_POOLS: usize = 4;
pub const MAX_V3_EFFECTS: usize = 4;
pub const MAX_V3_STATS: usize = 8;
pub const MAX_V3_TAGS: usize = 4;
pub const MAX_V3_STRING_BYTES: usize = 32;   // pool id, stat name, tag
pub const MAX_ENCODED_BODY_BYTES_V3: usize = 1_048_576;
pub const POLICY_CLOCK_HEARTBEAT_TICKS: u64 = 100;
pub use crate::protocol::{NetId, SessionEpoch, SEQ_FIRST, SEQ_LAST};
```

`LANE_COMBAT`, `MAX_INTENT_FRAME_BYTES`, `MAX_DELTA_FRAME_BYTES`,
`MAX_DELTA_OPS`, `MAX_VISIBLE_ENTITIES` and `MAX_WIRE_STRING_BYTES` are used
by qualified reference, never copied. `POLICY_CLOCK_HEARTBEAT_TICKS` is policy,
never wire.

### `protocol_v3` — modes, codes, facts

```rust
#[repr(u8)]
pub enum EncounterMode { TurnBased = 0, RealTime = 1 }
impl EncounterMode { pub fn as_u8(self) -> u8; pub fn from_u8(raw: u8) -> Option<Self>; }

#[repr(u8)]
pub enum StatValueKind { Int = 0, Fixed = 1 }
pub enum StatValue { Int(i32), Fixed(i32) }   // Fixed = Fx16_16::to_raw
impl StatValue { pub fn kind(self) -> StatValueKind; pub fn raw(self) -> i32; }

pub struct StatFact { pub name: String, pub value: StatValue }
pub struct PoolFact { pub id: String, pub current: u32, pub max: u32 }
pub struct EffectFact { pub effect: Ulid, pub expires_round: u32 }
pub struct OrderFact { pub ability: Ulid, pub target: NetId, pub repeat: bool }
pub struct TimelineEntry { pub key: i32, pub entity: NetId }
pub struct ClockFact { pub round_ticks: u32, pub elapsed: u64 }

#[repr(u8)]
pub enum PauseReason { Player = 0, Auto = 1, Operator = 2, Restore = 3 }
#[repr(u8)]
pub enum AutoPauseTrigger {
    EncounterStarted = 0, RoundStarted = 1, ModeChanged = 2,
    OrderEnded = 3, PartyMemberDied = 4,
}
pub struct AutoPauseToggles { pub bits: u8 }   // bit n = trigger n; 5..7 = 0
impl AutoPauseToggles {
    pub fn get(self, trigger: AutoPauseTrigger) -> bool;
    pub fn set(&mut self, trigger: AutoPauseTrigger, on: bool);
}
pub struct PauseFact { pub reason: PauseReason, pub peer: Option<u64> } // peer iff Player
pub struct HostFacts {
    pub paused: Option<PauseFact>,   // None = running
    pub can_pause: bool,
    pub can_unpause: bool,
    pub auto_pause: AutoPauseToggles,
    pub turn_timer_deadline_ms: Option<u64>,
}

#[repr(u8)]
pub enum RejectionCode {
    FrameTooLarge = 1, QueueFull = 2, UnsupportedVersion = 3, WrongDirection = 4,
    UnknownMessage = 5, Malformed = 6, LimitExceeded = 7, RateLimited = 8,
    Unauthenticated = 9, SessionExpired = 10, SeqConflict = 11, SeqGap = 12,
    StaleSeq = 13, SeqExhausted = 14, StaleTick = 15, FutureTick = 16,
    NotAuthorized = 17, IllegalAction = 18, ServerBusy = 19, SnapshotExpired = 20,
    ModeUnsupported = 21, WrongMode = 22, EncounterTerminal = 23,
    RulesetMismatch = 24, PlacementInUse = 25, NotCreature = 26,
    AlreadyParticipant = 27, NoEntrants = 28, StatEvaluation = 29,
    PauseDenied = 30, UnpauseDenied = 31,
}
impl RejectionCode {
    pub fn as_u8(self) -> u8;                 // 1..=31
    pub fn from_u8(raw: u8) -> Option<Self>;  // 0 and 32..=255 are None
}
pub enum ReceiptStatus { Applied, Rejected(RejectionCode) }
```

`RejectionCode`/`ReceiptStatus` are distinct types from
`crate::protocol`'s; a legacy receipt inside `Legacy` keeps 1..=20.

### `protocol_v3` — frames and ops

```rust
pub struct DeltaFrame {
    pub lane: u8, pub server_tick: u64, pub event_seq: u64,
    pub ops: Vec<DeltaOp>,   // ≤ MAX_DELTA_OPS
}
pub enum DeltaOp {
    Legacy(crate::protocol_v2::DeltaOp),   // tags 0..=10, bytes unchanged
    Encounter {
        mode: Option<EncounterMode>,
        pending_granted: bool,
        pending_mode: Option<EncounterMode>,
        round: Option<u64>,
        clock: Option<ClockFact>,
        active: Option<Option<NetId>>,   // None = not granted
    },
    Timeline { entries: Vec<TimelineEntry> },
    EntityFacts {
        entity: NetId,
        health: Option<crate::protocol_transfer::HealthFact>,
        pools: Option<Vec<PoolFact>>,
        effects: Option<Vec<EffectFact>>,
        order: Option<OrderFact>,
        stats: Option<Vec<StatFact>>,
        tags: Option<Vec<String>>,
    },
    RoundStarted { round: u64 },
    ModeChanged { mode: EncounterMode, round: u64 },
    OrderEnded { actor: NetId },
    Joined { entity: NetId },
    Left { entity: NetId },
    Clock { server_tick: u64, paused: bool },
    HostFacts(HostFacts),
    Receipt {
        epoch: [u8; 16], lane: u8, seq: u64, processed_tick: u64,
        status: ReceiptStatus,
    },
}

pub struct IntentFrame {
    pub epoch: [u8; 16], pub lane: u8, pub seq: u64,
    pub observed_tick: u64, pub actor: NetId, pub body: IntentBody,
}
pub enum IntentBody {
    DeclareAction { ability: Ulid, target: NetId },  // tag 0
    EndTurn,                                          // tag 1
    Stop,                                             // tag 2
    Pause,                                            // tag 3
    Unpause,                                          // tag 4
    RequestMode { mode: EncounterMode },              // tag 5
}
```

Every optional field is a presence bit: omitted means no payload bytes.
A granted empty list is count `0`, never an absent block.

### `codec_v3`

```rust
pub use crate::codec::CodecError;
pub fn encode_intent(frame: &IntentFrame) -> Result<Vec<u8>, CodecError>;
pub fn decode_intent(bytes: &[u8]) -> Result<IntentFrame, CodecError>;
pub fn encode_delta(frame: &DeltaFrame) -> Result<Vec<u8>, CodecError>;
pub fn decode_delta(bytes: &[u8]) -> Result<DeltaFrame, CodecError>;
```

v3 delta: version `3`, lane, `server_tick`, `event_seq`, `op_count`, then per
op a tag and its fields. v3 intent: version `3`, `epoch [u8;16]`, lane, `seq`,
`observed_tick`, `actor`, body tag, body. Field order below is the wire.

| Tag | Fields after the tag |
|---|---|
| 0..=10 | exactly the v2 op bytes (`Legacy` adds no byte) |
| 11 Encounter | `mask` (bit 0 mode, 1 pending, 2 round, 3 clock, 4 active); then set bits in order: `mode u8`; `pending_present bool` + `pending u8`; `round u64`; `round_ticks u32` + `elapsed u64`; `active_present bool` + `active u64` |
| 12 Timeline | `count` (≤ 1,024); per entry `key i32` (zigzag), `entity u64` |
| 13 EntityFacts | `entity u64`, `mask` (bits 0..5 health, pools, effects, order, stats, tags); per set bit: health `u32,u32,bool`; pools `count` + (`id` string ≤32, `current u32`, `max u32`); effects `count` + (`effect` ULID text, `expires_round u32`); order `ability` ULID text, `target u64`, `repeat bool`; stats `count` + (`name` ≤32, `kind u8`, `value i32`); tags `count` + string ≤32 |
| 14 RoundStarted | `round u64` |
| 15 ModeChanged | `mode u8`, `round u64` |
| 16 OrderEnded | `actor u64` |
| 17 Joined | `entity u64` |
| 18 Left | `entity u64` |
| 19 Clock | `server_tick u64`, `paused bool` |
| 20 HostFacts | `paused_present bool`; if true `reason u8`, and for `Player` `peer u64`; `can_pause bool`, `can_unpause bool`, `toggles u8` (5..7 zero), `timer_present bool` + `deadline_ms u64` |
| 21 Receipt | `epoch [u8;16]`, `lane u8`, `seq u64`, `processed_tick u64`, `status u8`; for `Rejected` then `code u8` (1..=31) |

Intent bodies: tag 0 `ability` ULID text + `target u64`; tags 1..4 no fields;
tag 5 `mode u8`. `actor` is the acting replica entity for every verb; rights
are host policy.

### `projection_v3`

```rust
pub struct EventGrants {
    pub round_started: bool, pub mode_changed: bool, pub order_ended: bool,
    pub joined: bool, pub left: bool,
}
pub enum EventCandidate {
    RoundStarted { round: Option<u64> },
    ModeChanged { mode: Option<EncounterMode>, round: Option<u64> },
    OrderEnded { actor: Option<NetId> },
    Joined { entity: Option<NetId> },
    Left { entity: Option<NetId> },
}
pub enum ProjectionError { TooManyEvents }
pub fn project_events(
    candidates: &[EventCandidate], grants: &EventGrants,
) -> Result<Vec<DeltaOp>, ProjectionError>;
```

At most `MAX_DELTA_OPS` inputs. A candidate is emitted only when every field
is `Some` and its grant is true; otherwise it is omitted without inspecting it
further, as `projection_v2` does (`docs/architecture/crpg-net.md:301-307`).
`Joined`/`Left` require the row's `presence`; the host encodes that by setting
`EventGrants::joined`/`left` only for disclosed rows (no second grant).

### `interest_v3` — per-field filtering and the v3 snapshot

```rust
pub struct FactMask {
    pub presence: bool, pub health: bool, pub pools: bool,
    pub effects: bool, pub order: bool, pub stats: bool, pub tags: bool,
}
pub struct FactGrants {
    pub mode: bool, pub pending_mode: bool, pub round: bool,
    pub clock: bool, pub active: bool, pub timeline: bool,
}
pub struct EncounterState {
    pub area: Ulid, pub mode: EncounterMode,
    pub pending_mode: Option<EncounterMode>, pub round: u64,
    pub clock: Option<ClockFact>, pub active: Option<EntityId>,
}
pub struct FactRow<'a> {
    pub key: EntityKey, pub fields: FactMask,
    pub health: Option<HealthFact>,
    pub pools: &'a [PoolFact], pub effects: &'a [EffectFact],
    pub order: Option<OrderFact>, pub stats: &'a [StatFact],
    pub tags: &'a [String],
}
pub struct FactContext<'a> {
    pub epoch: SessionEpoch, pub tick: u64, pub interest: InterestSet,
    pub facts: &'a [FactRow<'a>], pub grants: FactGrants,
    pub encounter: Option<EncounterState>, pub resend: bool,
}
pub struct SnapshotProjectionV3 { pub body: SnapshotBodyV3, pub cursor: ViewCursor }
pub fn facts_page(
    view: &InterestView, context: &FactContext<'_>,
) -> Result<Vec<DeltaOp>, InterestError>;
impl InterestView {
    pub fn project_snapshot_v3(
        &mut self, context: &FactContext<'_>,
    ) -> Result<SnapshotProjectionV3, InterestError>;
}
```

`facts_page` is read-only: it translates authority rows through
`InterestView::net_id`/`entity`, so there is still one view per epoch
(`crates/crpg-net/AGENTS.md:107-113`). `project_snapshot_v3` is the only v3
replacement primitive: it mints and retires exactly as `project_snapshot`
(`crates/crpg-net/src/interest.rs:328-395`) but returns the v3 body; a session
uses one body version for its lifetime and never mixes the two calls.

Page order: one `Encounter` (when any field is granted and the encounter is in
the viewer's area), one `Timeline` (when granted), then one `EntityFacts` per
visible row in ascending `NetId`; at most `MAX_V3_PAGE_OPS`. It never emits
`Health`, `Turn`, `EntityEnter`, `EntityLeave` or a notice: those stay with
`InterestView`/`project_events` (`docs/architecture/crpg-net.md:451-457`).

### `protocol_transfer_v3` and `codec_transfer_v3`

```rust
pub const TRANSFER_VERSION_V3: u8 = 0x82;
// LANE_SNAPSHOT, BeginTransfer, TransferChunk, CancelReason, ResyncReason,
// TransferDown, TransferUp, TransferId and the T024 caps are re-used by path.
pub struct SnapshotBodyV3 {
    pub entities: Vec<SnapshotEntityV3>,
    pub encounter: Option<EncounterSnapshot>,
    pub timeline: Option<Vec<TimelineEntry>>,
}
pub struct SnapshotEntityV3 {
    pub entity: NetId, pub health: Option<HealthFact>,
    pub pools: Option<Vec<PoolFact>>, pub effects: Option<Vec<EffectFact>>,
    pub order: Option<OrderFact>, pub stats: Option<Vec<StatFact>>,
    pub tags: Option<Vec<String>>,
}
pub struct EncounterSnapshot {
    pub mode: EncounterMode, pub pending_mode: Option<EncounterMode>,
    pub round: u64, pub clock: Option<ClockFact>, pub active: Option<NetId>,
}
pub fn encode_down(msg: &TransferDown) -> Result<Vec<u8>, CodecError>;
pub fn decode_down(bytes: &[u8]) -> Result<TransferDown, CodecError>;
pub fn encode_up(msg: &TransferUp) -> Result<Vec<u8>, CodecError>;
pub fn decode_up(bytes: &[u8]) -> Result<TransferUp, CodecError>;
pub fn encode_body(body: &SnapshotBodyV3) -> Result<Vec<u8>, CodecError>;
pub fn decode_body(bytes: &[u8]) -> Result<SnapshotBodyV3, CodecError>;
pub fn plan_transfer(
    epoch: [u8; 16], transfer: TransferId, base_tick: u64, cursor: u64,
    segments_per_chunk: u8, body: &SnapshotBodyV3,
) -> Result<Vec<Vec<u8>>, CodecError>;
```

The `0x82` frame layout is byte-for-byte `0x81`'s with the first byte changed
(`tasks/T024.md:298-319`). Body order: `entity_count`; per entity `entity`,
`mask` (bits 0..5 as tag 13), each set block; `encounter_present bool` and its
fields as tag 11; `timeline_present bool` and its entries. Every present fact
is granted at capture; absent is a zero flag, never a zero value.

## Behaviour

Normative; each names its test in `## Tests`. "Refused" means the stated error
with no state change.

1. **Version selection is explicit and closed.** Encoders start with `3`;
   decoders accept only `3` and return `UnsupportedVersion` for `1`, `2` and
   `0x81`; empty is `Malformed`; v1/v2 refuse `3`. No negotiation.
   *Test:* `v3_is_explicit_and_closed`.
2. **Old vocabulary is byte-identical inside v3.** `Legacy(op)` encodes to the
   v2 frame bytes with only the version byte changed; `Legacy` adds no tag.
   *Test:* `legacy_adds_no_bytes_in_v3`.
3. **The v3 intent header is v1's with byte 3.** Tags 0/1 keep v1 payloads;
   tags 2..4 have no body; tag 5 carries one mode byte. Unknown body tag or
   mode is `UnknownMessage`; trailing bytes are `Malformed`.
   *Test:* `intent_v3_literal_bytes_and_refusals`.
4. **Intent identity keeps v1 rules.** `lane != 0` is `WrongDirection`; `seq`
   outside `SEQ_FIRST..=SEQ_LAST` and `actor` 0 are `Malformed`.
   *Test:* `intent_identity_rules_are_v1_rules`.
5. **Decode precedence is frozen.** Frame cap → version → lane → tag → fields
   left to right → trailing bytes → semantics; first failure wins. An
   over-cap value is `LimitExceeded` even without a payload; a hostile varint
   is `Malformed`; encode checks the same and returns no partial bytes.
   *Test:* `v3_decode_precedence`.
6. **Tags are closed.** v3 owns 11..=21; every other byte is `UnknownMessage`.
   `17` is `Joined`, not `Moved`; 22 and 255 are unknown.
   *Test:* `tag_table_is_closed`.
7. **Every `Encounter` field has its own grant.** Only `mode` granted carries
   one mask bit and one byte; only `active` granted with no actor carries
   presence `false` and no id; all 32 masks round-trip; bits 5..7 set are
   `Malformed`. *Test:* `encounter_field_grants`.
8. **The timeline is ordered and bounded.** Ascending `(key, entity)`; equal
   keys for different entities are legal; a repeated entity, a descending pair
   or a zero id is `Malformed`; count > 1,024 is `LimitExceeded` before
   allocation. Hidden entities have no entry, so no hidden key leaks.
   *Test:* `timeline_order_and_bounds`.
9. **Per-entity facts are filtered per family.** A visible row with no family
   grant emits `EntityFacts { entity, mask: 0 }`; a family appears exactly when
   its bit is set; a granted empty list is count `0`; an ungranted family has
   no bytes; the element and 32-byte string caps fire before allocation.
   *Test:* `entity_facts_field_by_field`.
10. **Health is the frozen `HealthFact`**, emitted only when `presence` and
    `health` are granted; `facts_page` never emits the legacy `Health` op.
    *Test:* `v3_health_uses_the_frozen_fact`.
11. **The three N5 events.** `RoundStarted { round }`, `ModeChanged { mode,
    round }`, `OrderEnded { actor }`; a candidate needs every field `Some` and
    its grant, else it is omitted whole; input order survives (no sort/dedup);
    > `MAX_DELTA_OPS` candidates is `TooManyEvents` with no output.
    *Test:* `round_mode_order_events`.
12. **`Joined`/`Left` are notices, not identity.** Tags 17/18 carry one id and
    never mint, retire or re-map (that stays `EntityEnter`/`EntityLeave`/
    `Despawned`, `docs/architecture/crpg-net.md:451-457`). The host grants them
    only for a present row and omits a `Left` whose id is already retired.
    *Test:* `joined_left_are_notices`.
13. **`Moved` has no v3 tag or field.** Tag 22 is `UnknownMessage`; movement
    waits for T026p (`docs/adr/0036-movement-and-navigation.md:413-417`).
    *Test:* `moved_waits_for_t026p`.
14. **Codes are frozen 1..=31.** Every value maps both ways; 0 and 32..=255 are
    `None` and a receipt carrying one is `UnknownMessage`; every receipt
    round-trips; a foreign lane is `WrongDirection`; a bad `seq` is
    `Malformed`; v1/v2 still reject 21..=31.
    *Test:* `rejection_codes_1_to_31`.
15. **The heartbeat is a delta op with a self-consistent tick.** `Clock`
    round-trips; its `server_tick` must equal the frame's or the frame is
    `Malformed`. Sending every `POLICY_CLOCK_HEARTBEAT_TICKS = 100` while
    ticking and immediately on pause change is a T141 host requirement.
    *Test:* `clock_heartbeat_shape`.
16. **Host facts are one op.** Every reason, both rights, 32 toggle bitsets
    with bits 5..7 zero, and absent/present deadline round-trip. `Player`
    alone carries a peer (`Malformed` otherwise); an unknown reason is
    `UnknownMessage`; high toggle bits are `Malformed`. A missing right yields
    a `PauseDenied`/`UnpauseDenied` receipt; commands still run while paused
    (P2, `docs/adr/0029-kernel-range-and-time-model.md:284-286`).
    *Test:* `host_facts_round_trip`.
17. **A viewer with no grant sees none of the new facts.** With all grants and
    masks false, `facts_page` is `[]`, every event grant false is `[]`, and the
    snapshot body has entity ids with no families and no encounter/timeline.
    Nothing is zero-filled or empty-filled.
    *Test:* `no_grant_sees_nothing_new`.
18. **Orders default to the controller only.** `FactMask.order` starts false;
    the host sets it for the controlling peer alone (N2). `facts_page` has no
    ownership input, so a row with an order and `order: false` emits no block.
    *Test:* `order_is_controller_only`.
19. **The snapshot matches the delta page.** Entities ascend by `NetId`;
    presence gates an entity; the same grants gate the same families; `active`
    is `Some` only inside the body; encounter and timeline follow their
    grants; host facts are absent. Decode rejects non-ascending or duplicate
    entities, an outside `active`, an over-cap family and a string > 32 B.
    *Test:* `snapshot_v3_matches_delta_facts`.
20. **The snapshot is a new transfer version, not a new lane.** `0x82` uses
    lane 2 and the `0x81` framing; each version refuses the other; the body
    cap is `MAX_ENCODED_BODY_BYTES_V3`; a maximal body fits 32 chunks and
    publishes atomically, leaving the prior replica intact on failure.
    *Test:* `transfer_v3_is_a_new_version`.
21. **Redelivery is byte-identical.** Every frame and body re-encodes to the
    same bytes; a fresh view replaying the same contexts produces equal pages;
    no call mutates its context. *Test:* `v3_redelivery_is_identical`.
22. **Nothing frozen moved.** The existing suites prove v1, v2, `0x81`, `0x91`
    and codes 1..=20 unchanged; no sim code is touched, so no
    `authoritative_hash` changes. *Test:* the existing `crpg-net` suite.

## Errors

T140 adds `ProjectionError` and reuses `CodecError`/`InterestError`. No error
carries a payload, id or input string; `Display` is `<VariantName> at <area>`
and `std::error::Error` has no source (`tasks/T024.md:437-442`).

| Call | Refusal | Error |
|---|---|---|
| encode/decode intent | over 4,096 B | `FrameTooLarge` |
| | version not 3 | `UnsupportedVersion`; empty `Malformed` |
| | lane not 0 | `WrongDirection` |
| | unknown body tag or mode byte | `UnknownMessage` |
| | bad `seq`/`actor`, ULID text, truncation, trailing | `Malformed` |
| | ULID text > 256 B | `LimitExceeded` |
| encode/decode delta | over 65,536 B | `FrameTooLarge` |
| | version not 3 | `UnsupportedVersion`; empty `Malformed` |
| | lane not 0 (receipt lane too) | `WrongDirection` |
| | tag outside 0..=21 | `UnknownMessage` |
| | unknown mode/reason/code byte | `UnknownMessage` |
| | > `MAX_DELTA_OPS` ops | `LimitExceeded` |
| | mask high bits, ordering, duplicates, zero id, tick mismatch | `Malformed` |
| | count/string/element over cap | `LimitExceeded` |
| body | count over cap, body over 1 MiB | `LimitExceeded` |
| | non-ascending/duplicate entity, outside `active`, kind > 1 | `Malformed` |
| `project_events` | > `MAX_DELTA_OPS` candidates | `TooManyEvents` |
| `facts_page` | wrong epoch / over 1,024 / duplicate / over cap | `WrongEpoch` / `TooManyFacts` / `DuplicateFact` |
| `project_snapshot_v3` | the same, then `IdExhausted` last | `InterestError` |

Every refusal leaves the view equal to its pre-call clone. `IdExhausted` stays
last (`docs/architecture/crpg-net.md:489-493`). `ReassemblyPool` keeps its
errors for `0x82`; only the body decoder differs. Gameplay refusals are
receipts, not codec errors: the ten codes plus `PauseDenied`/`UnpauseDenied`.

## Tests

Public API only; literal byte oracles; hostile input per bound
(`tasks/T024.md:911-916`). Epochs are 16 zero bytes unless stated.

### `tests/codec_v3.rs`

1. `v3_is_explicit_and_closed`: v3 `EndTurn { seq 1, observed_tick 7, actor 1 }`
   is `03` + 16 × `00` + `00 01 07 01 01` (21 B). v1/v2 frames are
   `UnsupportedVersion`; `03` alone is `Malformed`; `[3, 1, …]` is
   `WrongDirection`.
2. `legacy_adds_no_bytes_in_v3`: v2 `Turn { active: None, round: 5 }` is
   `02 00 00 01 01 06 00 05`; its v3 `Legacy` frame is
   `03 00 00 01 01 06 00 05` and decodes to the same op.
3. `intent_v3_literal_bytes_and_refusals`: `RequestMode RealTime` is
   `03` + 16 × `00` + `00 01 07 01 05 01`; `Stop`/`Pause`/`Unpause` end at
   their tag (20 B); body tag `06` and mode `02` are `UnknownMessage`; one
   trailing byte is `Malformed`.
4. `intent_identity_rules_are_v1_rules`: `seq 0`, `seq u64::MAX` and `actor 0`
   are `Malformed`; `lane 1` is `WrongDirection`.
5. `v3_decode_precedence`: 4,097 B intent and 65,537 B delta are
   `FrameTooLarge` even with a bad version; `[3, 0]` is `Malformed`;
   `[2, 0, …]` is `UnsupportedVersion`; `[3, 1, …]` is `WrongDirection`;
   `[3, 0, 0, 1, 1, 0x16]` is `UnknownMessage`; trailing `00` is `Malformed`.
6. `tag_table_is_closed`: tags are exactly 11..=21 and codes 1..=31 in order;
   `0x16` and `0xFF` are `UnknownMessage`; `0x11` is `Joined`.
7. `encounter_field_grants`: mask `0` is `… 0B 00`; mask `0x10` with
   `Some(None)` is `… 0B 10 00`; with `Some(Some(1))` is `… 0B 10 01 01`;
   masks 0..=31 round-trip; `0x20` is `Malformed`.
8. `timeline_order_and_bounds`: one `(0, 1)` entry is `… 0C 01 00 01`;
   `[(1,1),(1,2)]` is accepted; `[(1,1),(0,2)]` and a repeated entity are
   `Malformed`; count `1025` (`81 08`) is `LimitExceeded`.
9. `entity_facts_field_by_field`: presence only is `… 0D 01 00`; all six
   families give mask `0x3F` and round-trip health `3/5/false`, pool
   `("ap",2,3)`, effect `(ulid,4)`, order `(ulid,2,true)`, stat
   `("str",Int(4))` and tag `"undead"`; `pools: false` removes the block
   (same bytes as presence only). Pool/effect/tag count `5` and stat count `9`
   are `LimitExceeded`; a 33-byte id is `LimitExceeded`.
10. `v3_health_uses_the_frozen_fact`: `HealthFact { 3, 5, false }` is the
    existing three fields; `fields.health = false` removes it; no legacy
    `Health` op is emitted.
11. `round_mode_order_events`: `RoundStarted 3` is `… 0E 03`;
    `ModeChanged { RealTime, 2 }` is `… 0F 01 02`; `OrderEnded 2` is
    `… 10 02`; a `None` field omits the whole event; 256 inputs are kept in
    order, 257 is `TooManyEvents` with no output.
12. `joined_left_are_notices`: `Joined 2` is `… 11 02`, `Left 2` is
    `… 12 02`; neither changes a view mapping; `joined: false` omits it.
13. `moved_waits_for_t026p`: tag `0x16` is `UnknownMessage`; no `Moved`
    variant and no `DELTA_TAG_MOVED`.
14. `rejection_codes_1_to_31`: values 1..=31 map both ways; 0/32/255 are
    `None`; v1/v2 still reject 21..=31. `Receipt Applied` is
    `03 00 00 01 01 15` + 16 × `00` + `00 01 00 00`; `Rejected 22` ends
    `… 01 16`; code `32` is `UnknownMessage`; a foreign lane is
    `WrongDirection`.

### `tests/events_v3.rs`

15. `clock_heartbeat_shape`: `Clock { 0, false }` is `03 00 00 01 01 13 00 00`;
    `Clock { 7, true }` in a `server_tick: 8` frame is `Malformed`;
    `POLICY_CLOCK_HEARTBEAT_TICKS == 100`; a replica follows a `Clock` frame
    like any delta.
16. `host_facts_round_trip`: running, no rights, toggles 0, no timer is
    `… 14 00 00 00 00 00`; `Player(peer 9)` begins `01 00 01 09`;
    `Auto(RoundStarted)` begins `01 01 00`; toggles `0x1F` round-trip and
    `0x20` is `Malformed`; reason `4` is `UnknownMessage`; `Player` without a
    peer is `Malformed`; missing rights give `PauseDenied` 30 / `UnpauseDenied`
    31.
17. `round_mode_order_events` half two: a real five-event journal
    `[RoundStarted, ModeChanged, OrderEnded, Joined, Left]` survives projection
    in order; suppressing one leaves four.

### `tests/interest_v3.rs`

18. `no_grant_sees_nothing_new`: one all-false row plus a populated encounter
    and two timeline entries give `[]` from `facts_page`, `[]` from all-false
    event grants, and a snapshot `{ entities: [], encounter: None, timeline:
    None }` encoding as `00 00 00`.
19. `order_is_controller_only`: `order: Some(...)` with `fields.order = false`
    encodes as absent; `true` adds only the order block; no ownership input
    exists.
20. `entity_facts_field_by_field` page half: input `[net 2, net 1]` emits
    ascending 1 then 2 and no `EntityEnter`; a foreign-area row is dropped;
    1,025 rows is `TooManyFacts` and leaves the view unchanged.
21. `snapshot_v3_matches_delta_facts`: the same context gives equal values in
    both calls; one entity body is `01 01 00 00 00`; `active` is `None` when
    hidden; non-ascending/duplicate/outside-active/over-cap decode as
    `Malformed`/`LimitExceeded`.
22. `no_grant_sees_nothing_new` snapshot half: two rows contain only `entity`
    and a zero mask; encounter and timeline are `None`.

### `tests/snapshot_transfer_v3.rs`

23. `transfer_v3_is_a_new_version`: empty-body `Begin` is `82 02 10` + 16 ×
    `00` + `01 00 00 03 00 01 01` (26 B); its `Chunk` is `82 02 11` + 16 ×
    `00` + `01 00 01 03 00 00 00` (26 B); each version refuses the other;
    lane 2 and the direction tags keep their rules.
24. `maximal_v3_body_fits_the_envelope`: 1,024 entities each with health,
    4 × 32-byte pools, 4 effects, an order, 8 × 32-byte stats, 4 × 32-byte
    tags, plus a clocked encounter and 1,024 timeline entries; encode succeeds
    at ≤ 1 MiB and ≤ 32 × 32,768; `plan_transfer` uses ≤ 32 chunks; decode
    returns the same body.
25. `v3_publication_is_atomic`: a full body publishes atomically; a missing,
    conflicting or truncated chunk leaves the prior replica unchanged; a
    `0x82` body never publishes through `0x81`.
26. `v3_redelivery_is_identical`: every op/body encodes twice equal; a fresh
    view on the same context sequence produces equal pages; the context is not
    mutated.
27. `nothing_frozen_moved`: `codec`, `events_v2`, `snapshot_transfer`,
    `snapshot_transfer_contract`, `reconnect_wire`, `interest` and
    `conformance` run unchanged and still pin versions 1/2, `0x81`, `0x91` and
    codes 1..=20; no other test file is edited.

## Out of scope

- Any source change; this is a draft.
- T141: reading `World`, building values, mode authorization, pause rights,
  auto-pause, turn timers, heartbeat scheduling, the server projection.
- T026p and lane 1: `DeclareMove`, `Moved`, movement samples, position grants.
  ADR-0036 L6 decides later (`docs/adr/0036-movement-and-navigation.md:413-417`,
  `:568-571`).
- Roll/DC/margin disclosure; v3 has no field for them.
- Edits to v1, v2, `0x81`, `0x91`, their goldens or codecs; any new dependency;
  any other crate, including `crpg-server` and `crpg-godot`.
- Negotiation, downgrade, compression, save/replay formats, `crpg.ui/1`.

## Definition of done

- [ ] The adjudicated contract is approved by the user; this draft is not one.
- [ ] T140a: `protocol_v3`/`codec_v3` match the constants, tags, fields, bounds
      and precedence above; cases 1–14 pass.
- [ ] T140b: `projection_v3`/`interest_v3` exist; `facts_page` is read-only and
      `project_snapshot_v3` the only v3 replacement; cases 15–22 pass.
- [ ] T140c: `protocol_transfer_v3`/`codec_transfer_v3` use `0x82`, the `0x81`
      framing and `ReassemblyPool`, and the v3 body; cases 23–27 pass.
- [ ] The maximal body fits `MAX_ENCODED_BODY_BYTES_V3` and 32 chunks; no
      frozen cap is raised.
- [ ] Every existing `crpg-net` test passes unchanged; v1/v2/`0x81`/`0x91` and
      codes 1..=20 stay byte-identical.
- [ ] `cargo fmt --all`, `cargo clippy -p crpg-net --all-targets -- -D
      warnings`, `cargo test -p crpg-net --locked`, the two Python lints and
      their self-tests pass.
- [ ] `docs/architecture/crpg-net.md` and `crates/crpg-net/AGENTS.md` record
      v3, the tables, the `0x82` body and "no `World` here", each signed.
- [ ] No `HashMap`/`HashSet`, `f32`/`f64`, `unsafe`, I/O, threads or clocks.

## Questions for the user

1. **Roll transparency (ADR-0029 Open question 8).** (a) Closed (this draft):
   no roll, modifier, DC or margin. (b) A ruleset may show a controller its own
   raw roll and modifiers, DCs hidden unless public. (c) Everyone sees every
   number. *Player effect:* (a) they never know why an attack failed; (b) they
   see their own dice, not the target number; (c) hidden DCs leak.
   *Recommendation:* (b), as ADR-0029 recommends, in a follow-on; stay at (a)
   until answered.
2. **Do `Joined`/`Left` ship in v3?** (a) Yes, tags 17/18. (b) No, leave them
   to T026p. *Player effect:* (a) a client can say "X joined" rather than
   infer it from `EntityEnter`; (b) authors cannot distinguish join from
   enter. *Recommendation:* (a); T139 deferred the wire form here
   (`tasks/T139.md:1914-1916`).
3. **Heartbeat interval.** (a) 100 ticks. (b) 200, the window itself. (c) host
   policy only. *Player effect:* (b) one lost heartbeat makes later commands
   stale; (a) survives one loss; (c) clients cannot test a fixed rule.
   *Recommendation:* (a).
4. **Element caps.** (a) 4/4/8/4 (under 1 MiB). (b) 16/64/64/32 (needs a new
   transfer envelope). *Author effect:* (a) more than four pools or effects on
   one creature cannot replicate; (b) can, at a frozen-cap review.
   *Recommendation:* (a) now; revisit with T109.
5. **Where do host facts first reach a late joiner?** (a) As `HostFacts` and
   `Clock` ops after publication. (b) In the snapshot body. *Player effect:*
   (b) one message, but per-viewer rights in a shared, stale body; (a) costs
   two small ops. *Recommendation:* (a).
6. **Pause rights codes.** (a) `PauseDenied` and `UnpauseDenied`. (b) One
   `NoPauseRight`. *Player effect:* (a) the client says which right is
   missing; (b) it cannot. *Recommendation:* (a); P4 grants them separately
   (`docs/adr/0029-kernel-range-and-time-model.md:290-297`).
7. **Does movement fold into v3?** (a) No; T026p takes the next version.
   (b) Add `DeclareMove`/`Moved` now. *Player effect:* (b) movement one
   version earlier, but a new H-filter rule and a second review of every v3
   fact. *Recommendation:* (a); ADR-0036 L6 permits it
   (`docs/adr/0036-movement-and-navigation.md:568-571`).
8. **Is `pending_mode` separately granted?** (a) Yes, own bit. (b) Covered by
   `mode`. *Player effect:* (a) a client shows "switching" only when told;
   (b) it always sees the pending switch. *Recommendation:* (a); N2 makes every
   fact its own field grant.

## Agent log

- 2026-11-10 (UTC) · deepseek-v4.1-flash-free + T140 blind draft a · Drafted the v3 wire, the T140a/b/c split, the readings, the per-field grant filter, the `0x82` snapshot body and eight user questions from the T140 fact sheet and ADR-0029, without reading the other blind draft.
