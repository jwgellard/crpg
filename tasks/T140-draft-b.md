# T140 — next `crpg-net` protocol version: ADR-0029 N1–N4 facts, events, intents, snapshot allowlist, clock heartbeat (draft, blind)

## Task

One new main protocol version after v2 that carries the ADR-0029 §4 N1 facts,
the events `RoundStarted`/`ModeChanged`/`OrderEnded`, the intents
`DeclareAction`/`EndTurn`/`Stop`/`Pause`/`Unpause`/`RequestMode`, the rejection
codes other contracts left to T140, the snapshot-allowlist extension of
ADR-0028, per-field grant filtering for every new fact (T027b H-filter, N2),
and the `Clock` heartbeat message (N4). v1, v2, transfer (`0x81`) and resume
(`0x91`) stay frozen byte for byte.

## Crate(s)

Sole crate: `crpg-net`.

## Split (T140a, T140b, T140c, T140d)

The work is more than one implementer run (each pool below is about 400–500
source lines plus its suite), so it splits in dependency order. Each part ends
with its own gate command; the work is serialized, never parallel.

- **T140a — vocabulary.** Owns:
  - `crates/crpg-net/src/protocol_v3.rs` (constants, tag values, bounds,
    compile-time eq asserts, wire types: `IntentBody` v3, `DeltaOp` v3,
    `RejectionCode` v3 superset 1..=30).
  - `crates/crpg-net/src/codec_v3.rs` (intent codec: `encode_intent`/
    `decode_intent`, six tags, precedence below).
  - `crates/crpg-net/tests/codec_v3.rs` (freeze tests: version gates, tag
    oracle, bounds, frame caps).
  - `crates/crpg-net/src/lib.rs` (two `pub mod` lines in alphabetical order).
  - goldens: existing wire goldens and hashes must not move — v1/v2/0x81/0x91
    bytes, tags, maxima and policy pins stay frozen (`tests/codec.rs` sets all
    nine v1 maxima and tags 0..=7; `tests/events_v2.rs` pins tags 8..=10;
    `tests/interest.rs:2503` pins literal page bytes; `tests/snapshot_transfer.rs`
    and `tests/reconnect_wire.rs` pin the subprotocol tags) because v3 only
    adds a third version byte; the new tests add literal byte oracles inside
    `tests/codec_v3.rs`, no golden files.
- **T140b — deltas and projection.** Owns:
  - `crates/crpg-net/src/codec_v3.rs` (delta codec: `encode_delta`/
    `decode_delta`, tags 0..=26 with byte oracles, bounded-before-allocation
    strings/vecs).
  - `crates/crpg-net/src/projection_v3.rs` (`EventCandidate` v3 variants for
    the five wired events, `project_events` ordered-disclosure append).
  - `crates/crpg-net/tests/events_v3.rs` (byte oracles, suppression,
    ordered projection of real T138/T139 histories through the dev-only sim
    edge).
  - Update `crates/crpg-net/tests/codec.rs`:183 — the
    `transfer_and_lane_zero_refuse_each_other`-style cross-refusals add byte
    3: v1/v2 refuse a v3 delta with `UnsupportedVersion`; v3 refuses 1 and 2
    the same way. Old pins unchanged.
- **T140c — interest and the `0x82` snapshot body.** Owns:
  - `crates/crpg-net/src/interest.rs` (extended `FieldMask`,
    `ViewerContext`, `MAX_PAGE_OPS_V3`, new state-op production, snapshot
    body 3 projection, per-field grant filtering).
  - `crates/crpg-net/src/protocol_transfer.rs` (transfer version `0x82`,
    extended `SnapshotBody` types, new bounds and eq asserts).
  - `crates/crpg-net/src/codec_transfer.rs` (`encode_body8x`/`decode_body8x`
    for `0x82`).
  - `crates/crpg-net/tests/interest_v3.rs`, tests/transfer_v3.rs (grants,
    page bounds, `0x82` byte assembly, full body).
  - goldens: `tests/interest.rs:1398` fresh-page layout parity, and
    `tests/interest.rs:1982` hidden-input-invariance, extend to the new
    fields with new cases, never changed pins; `tests/snapshot_transfer.rs`
    and `tests/snapshot_transfer_contract.rs` stay untouched (they pin `0x81`
    only).
- **T140d — heartbeat, replica, conformance.** Owns:
  - `crates/crpg-net/src/replica.rs` (extended op table: state ops replace
    facts, notices are shaping-checked; no new state stored beyond a bounded
    `ReplicaEncounter` summary).
  - `crates/crpg-net/tests/events_v3.rs` heartbeat cases plus
    `tests/conformance.rs` additions driving the heartbeat and the new intents
    through the simulated transport with positive controls.
  - `crates/crpg-net/tests/replica.rs` (op-table cases for every new tag).
  - goldens: none move; `tests/conformance.rs:1466-1483` keeps its v1
    constants and hashes; the new suite records only in-process hashes.

## Readings of the ADR

1. **"One protocol version" (N3, `docs/adr/0029-kernel-range-and-time-model.md:338-340`) means main protocol v3 on lane 0, not a fourth subprotocol.**
   The facts ride the combat delta stream with `Legacy`-style wrapping exactly
   as T021 did; alternative rejected: a separate facts subprotocol would
   split one journal-ordered stream across two delivery lanes and lose the
   cutover/fencing guarantees T024/T025a built on one order per direction.
2. **The clock heartbeat (N4, `:342-345`) is exactly one delta frame holding one `Clock` op, not a new frame kind.**
   v1/v2/v3 delta frames have no discriminator byte after the version/lane
   pair (`crates/crpg-net/src/codec_v2.rs:600-613`), so a standalone
   `Clock { server_tick, paused }` frame would be indistinguishable from a
   delta whose `server_tick` varint shares its first byte. Alternative
   rejected: adding a kind byte to the v3 delta header would break the
   T021 byte law that old vocabularies are byte-identical except the version
   byte (`tests/events_v2.rs` `v1_and_v2_are_explicit`). The header's
   `server_tick` field is already the heartbeat's `server_tick`, so the op
   reduces to `(paused, reason)`.
3. **`SnapshotBody` extension rides transfer version `0x82` (T024's own promise).**
   `tasks/T024.md` S§5.4 says new facts "need a new transfer version
   (`0x82`) and their own review" (`tasks/T024.md:334`), and ADR-0029 N3
   says the same protocol version extends ADR-0028's allowlist — both hold:
   v3 is the protocol version that introduces `0x82`; `0x81` stays frozen
   and callable. Alternative rejected: keeping `0x81` and resyncing new
   facts only through deltas contradicts N3's "resync and late join see
   what deltas show" (`docs/adr/0029-kernel-range-and-time-model.md:340`)
   and would leave a party member joining mid-encounter blind to pool/effect
   state until the next event.
4. **`Joined` and `Left` get wire forms in v3; `Moved` does not.**
   T139 explicitly hands the `Joined`/`Left` wire forms and codes to T140
   (`tasks/T139.md:1914-1917`) and the host withholds them today
   (`crates/crpg-server/src/host.rs:2484-2489`). `Moved` is withheld because
   ADR-0036 says the `DeclareMove` verb and `Moved` projection ride T026p,
   which defines the *next* version only if T140's contract is closed
   (`docs/adr/0036-movement-and-navigation.md:568-571`); that is Open
   question 3 below. Alternative rejected: freeing `Moved` now would put
   `Vec<[Fx16_16; 3]>` on the wire with no grant vocabulary and no
   consuming verb, splitting T026p's decision across two tasks.
5. **`RejectionCode` v3 is a re-declared superset `1..=30`, not a parallel enum.**
   v1 codes `1..=20` are frozen (`crates/crpg-net/src/protocol.rs:391-435`).
   The new codes have to reach a receipt, and the receipt op is a hard-coded
   v1 tag inside `DeltaOp::Legacy` — a v2-style `Legacy` wrap cannot widen
   the code byte without re-declaring the op. Alternative rejected: a
   name-space extension validated only in the host would make `RejectionCode`
   codes version-dependent, breaking the `as_u8`/`from_u8` contract
   (`crates/crpg-net/src/protocol.rs:437-448`).
6. **Round and active combatant already have wire forms; they come under grants, not new tags.**
   N1 lists "round, active combatant (TB)" as facts, and both already travel
   in `DeltaOp::Turn { active, round }` (`crates/crpg-net/src/protocol.rs:346-352`)
   under the `turn_state` grant (`crates/crpg-net/src/interest.rs:89`).
   Alternative rejected: a second `Active` op would let the two disagree
   when only one is granted, which ADR-0019's per-field-suppression rule
   (`docs/adr/0019-event-protocol-v2.md:22-26`) resolves by construction
   today.
7. **Standing orders default to the controlling peer only, enforced host-side.**
   N2's default (`docs/adr/0029-kernel-range-and-time-model.md:334-336`) is a
   grant decision about *who* gets the `order` field grant; `crpg-net` only
   implements the filter. Knowing a peer is the controller is host policy
   (T141): the grant vocabulary here is per-entity, deny-by-default
   (`crates/crpg-net/src/interest.rs:39-46`). Alternative rejected: putting
   a controller flag on the wire would disclose control relationships the
   N2 rules never authorize.

## Interface

New module `crates/crpg-net/src/protocol_v3.rs` (+ `codec_v3.rs`,
`projection_v3.rs`), in the T021 shape: every new struct and enum derives
`Debug, Clone, PartialEq, Eq`; discriminants are explicit `u8` matches, not
derived serde tags (`crates/crpg-net/AGENTS.md` known traps). No new
dependency; wire primitives are the existing `postcard` ones
(`docs/architecture/crpg-net.md:143-147` documents the byte shapes).

### Constants and tags

```rust
pub const PROTOCOL_VERSION: u8 = 3;
pub use crate::protocol::{LANE_COMBAT, SEQ_FIRST, SEQ_LAST, SessionEpoch, NetId,
    MAX_INTENT_FRAME_BYTES, MAX_DELTA_FRAME_BYTES, MAX_DELTA_OPS,
    MAX_WIRE_STRING_BYTES, MAX_VISIBLE_ENTITIES};
// v3 declares its own IntentFrame (header identical to v1's, v3 body type).

Delta tags (tags 0..=10 all freeze here; declaration order):

```rust
pub const DELTA_TAG_MODE: u8 = 11;      // Mode { mode, pending }
pub const DELTA_TAG_ENCOUNTER_CLOCK: u8 = 12;  // EncounterClock { round, elapsed, round_ticks }
pub const DELTA_TAG_ENTRY: u8 = 13;     // Entry { entity, key }
pub const DELTA_TAG_POOL: u8 = 14;      // Pool { entity, pool, current, max }
pub const DELTA_TAG_EFFECT: u8 = 15;    // Effect { entity, effect, expires_round }
pub const DELTA_TAG_ORDER: u8 = 16;     // Order { entity, order }
pub const DELTA_TAG_STAT: u8 = 17;      // Stat { entity, stat, value }
pub const DELTA_TAG_ROUND_STARTED: u8 = 18;
pub const DELTA_TAG_MODE_CHANGED: u8 = 19;
pub const DELTA_TAG_ORDER_ENDED: u8 = 20;
pub const DELTA_TAG_JOINED: u8 = 21;
pub const DELTA_TAG_LEFT: u8 = 22;
pub const DELTA_TAG_CLOCK: u8 = 23;     // Clock { paused, reason } (the N4 heartbeat op)
pub const DELTA_TAG_PAUSE_RIGHTS: u8 = 24;
pub const DELTA_TAG_AUTO_PAUSE: u8 = 25;
pub const DELTA_TAG_TURN_TIMER: u8 = 26;
pub const MODE_TURN_BASED: u8 = 0;
pub const MODE_REAL_TIME: u8 = 1;
pub const PAUSE_REASON_PLAYER: u8 = 1;  // ADR-0029 P5; the owning peer is an open question (Q4)
pub const PAUSE_REASON_AUTO: u8 = 2;
pub const PAUSE_REASON_OPERATOR: u8 = 3;
pub const PAUSE_REASON_RESTORE: u8 = 4;
pub const AUTO_PAUSE_ENCOUNTER_STARTED: u8 = 1; // ADR-0029 P6 closed list
pub const AUTO_PAUSE_ROUND_STARTED: u8 = 2;
pub const AUTO_PAUSE_MODE_CHANGED: u8 = 3;
pub const AUTO_PAUSE_ORDER_ENDED: u8 = 4;
pub const AUTO_PAUSE_PARTY_DIED: u8 = 5;
```

New hard maxima (wire, versioned; each is a reviewed bound, not policy):

```rust
pub const MAX_WIRE_POOLS: usize = 8;         // per combatant, pool ids distinct
pub const MAX_WIRE_EFFECTS: usize = 16;      // per combatant, effect ids distinct
pub const MAX_WIRE_STATS: usize = 16;        // per combatant, stat names distinct
pub const MAX_WIRE_STAT_NAME_BYTES: usize = 64;
pub const MAX_WIRE_AUTO_PAUSES: usize = 32;
pub const MAX_PAGE_OPS_V3: usize =
    MAX_VISIBLE_ENTITIES * (9 + MAX_WIRE_POOLS + MAX_WIRE_EFFECTS + MAX_WIRE_STATS) + 7;
const _: () = assert!(MAX_PAGE_OPS_V3 <= usize::MAX);
```

Intent tags (0/1 keep their v1 meaning; `crates/crpg-net/src/protocol.rs:172-174`):

```rust
pub const INTENT_TAG_DECLARE_ACTION: u8 = 0;
pub const INTENT_TAG_END_TURN: u8 = 1;
pub const INTENT_TAG_STOP: u8 = 2;
pub const INTENT_TAG_PAUSE: u8 = 3;
pub const INTENT_TAG_UNPAUSE: u8 = 4;
pub const INTENT_TAG_REQUEST_MODE: u8 = 5;
```

Frame shape (header byte-for-byte v1's, `crates/crpg-net/src/protocol.rs:245-258`):

```rust
pub struct IntentFrame {
    pub epoch: [u8; 16],
    pub lane: u8,             // = LANE_COMBAT
    pub seq: u64,             // SEQ_FIRST..=SEQ_LAST
    pub observed_tick: u64,   // advisory; POLICY_OBSERVED_TICK_WINDOW host-side
    pub actor: NetId,
    pub body: IntentBody,     // v3's six-variant body
}
```

### Intents

Semantics map to sim verbs: `DeclareAction` (TB: `perform_action`; RT: sets
`Combatant.order` via `StandingOrder { ability, target, repeat }`,
`crates/crpg-sim/src/combat.rs:95-104`), `EndTurn` (TB only), `Stop` (RT only,
`crates/crpg-sim/src/combat.rs`), `Pause`/`Unpause` (host fact, ADR-0029 P4
`docs/adr/0029-kernel-range-and-time-model.md:290-297`), `RequestMode`
(`crates/crpg-sim/src/combat.rs:4305`; host authorization is T141).

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentBody {
    DeclareAction { ability: Ulid, target: NetId }, // tag 0
    EndTurn,                                        // tag 1
    Stop,                                           // tag 2
    Pause,                                          // tag 3
    Unpause,                                        // tag 4
    RequestMode { mode: u8 },                       // tag 5; 0 or 1, else UnknownMessage
}
```

`ulid` in the intent body must be a canonical 26-char ULID; the formatting
rule follows the v1 codec (`crates/crpg-net/src/protocol.rs:143-147`) and
`tests/codec.rs:248-263` pins both codecs. `RequestMode` uses raw `0`/`1`
with the named constants.

### Deltas and events

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaFrame {           // same field order and wire layout as v2
    pub lane: u8,                 // = LANE_COMBAT
    pub server_tick: u64,
    pub event_seq: u64,
    pub ops: Vec<DeltaOp>,        // push-built, at most MAX_DELTA_OPS
}

/// v3 delta vocabulary, re-declared natively: no `Legacy` chain. Tags 0..=10
/// keep v1/v2 payload formats byte for byte; the re-declaration exists so the
/// receipt op's code byte can widen past the frozen 1..=20 (Reading 5) and so
/// one enum owns every tag. The v1 declaration stays in `protocol.rs`:
/// `crates/crpg-net/src/protocol.rs:308-366`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaOp {
    // --- v1 ops: identical payload bytes (tags 0..=7) ---
    EntityEnter { entity: NetId },          // tag 0
    EntityLeave { entity: NetId },          // tag 1
    Spawned { entity: NetId },              // tag 2
    Despawned { entity: NetId },            // tag 3
    Died { entity: NetId },                 // tag 4
    Health { entity: NetId, health: u32, max_health: u32, dead: bool }, // tag 5
    Receipt { epoch: [u8; 16], lane: u8, seq: u64, processed_tick: u64,
              status: ReceiptStatus },      // tag 7; seq in SEQ_FIRST..=SEQ_LAST
    // --- v2 events: identical payload bytes (tags 8..=10) ---
    ActionResolved { actor: NetId, target: NetId, ability: Ulid,
                     outcome: String, damage: u32 },   // tag 8; outcome forms
                         // are T020's five symbolic strings only
    TurnStarted { actor: NetId, round: u64 },          // tag 9
    EncounterEnded { encounter: Ulid, round: u64 },    // tag 10
    // --- v3 state ops (producer: InterestView) ---
    Mode { mode: u8, pending: Option<u8> },                    // tag 11
    EncounterClock { round: u64, elapsed: u64, round_ticks: u32 }, // tag 12
    Entry { entity: NetId, key: i32 },                         // tag 13
    Pool { entity: NetId, pool: Ulid, current: u32, max: u32 },// tag 14
    Effect { entity: NetId, effect: Ulid, expires_round: u32 },// tag 15
    Order { entity: NetId, order: Option<OrderFact> },         // tag 16
    Stat { entity: NetId, stat: String, value: i32 },          // tag 17
    // --- v3 events (producer: project_events) ---
    RoundStarted { round: u64 },          // tag 18
    ModeChanged { mode: u8, round: u64 }, // tag 19
    OrderEnded { actor: NetId },          // tag 20
    Joined { entity: NetId },             // tag 21
    Left { entity: NetId },               // tag 22
    // --- v3 host facts (producer: host-encoded candidates) ---
    Clock { paused: bool, reason: Option<u8> },       // tag 23 (N4 heartbeat)
    PauseRights { pause: bool, unpause: bool },       // tag 24
    AutoPause { triggers: Vec<u8> },                  // tag 25, len ≤ MAX_WIRE_AUTO_PAUSES
    TurnTimer { deadline_ms: u64 },                   // tag 26
}

pub struct OrderFact {
    pub ability: Ulid,
    pub target: NetId,
    pub repeat: bool,
}

/// v3 `ReceiptStatus`, carrying the widened code space (Reading 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptStatus {
    Applied,                    // tag 0
    Rejected(RejectionCode),    // tag 1
}

/// v3 rejection codes: codes 1..=20 are the frozen v1 meanings verbatim
/// (bytes and semantics); 21..=30 are new. `as_u8`/`from_u8` as in v1; values
/// 0 and 31..=255 decode as `UnknownMessage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RejectionCode {
    FrameTooLarge = 1, QueueFull = 2, UnsupportedVersion = 3, WrongDirection = 4,
    UnknownMessage = 5, Malformed = 6, LimitExceeded = 7, RateLimited = 8,
    Unauthenticated = 9, SessionExpired = 10, SeqConflict = 11, SeqGap = 12,
    StaleSeq = 13, SeqExhausted = 14, StaleTick = 15, FutureTick = 16,
    NotAuthorized = 17, IllegalAction = 18, ServerBusy = 19, SnapshotExpired = 20,
    // T138 hands these two to T140 (`tasks/T138.md:859-866`):
    ModeUnsupported = 21, WrongMode = 22,
    // T137 names this one (`tasks/T137.md:823-824`):
    StatEvaluation = 23,
    // T139 names these six (`tasks/T139.md:1914-1917`):
    EncounterTerminal = 24, RulesetMismatch = 25, PlacementInUse = 26,
    NotCreature = 27, AlreadyParticipant = 28, NoEntrants = 29,
    // The P4 pause-right rejection (ADR-0029
    // `docs/adr/0029-kernel-range-and-time-model.md:296-297`):
    NoPauseRight = 30,
}
```

### Projection and per-field grants

`crates/crpg-net/src/projection_v3.rs` receives host-authorized candidates,
following `projection_v2` (`docs/architecture/crpg-net.md:301-306`):

```rust
pub enum EventCandidate {
    RoundStarted { round: Option<u64> },          // Some = `round` granted
    ModeChanged { mode: Option<u8>, round: Option<u64> },
    OrderEnded { actor: Option<NetId> },
    Joined { entity: Option<NetId> },
    Left { entity: Option<NetId> },
}
pub enum ProjectionError { TooManyEvents, StringLimit, Error }
pub fn project_events(candidates: &[EventCandidate]) -> Result<Vec<DeltaOp>, ProjectionError>;
```

Order preserved in, one op appended per fully-disclosed event, partially
hidden events omitted without inspecting hidden fields further
(ADR-0019 `docs/adr/0019-event-protocol-v2.md:22-26`); `ModeChanged` with a
granted `round` but an ungranted `mode` omits the whole event
(consumer gates are whole-event, Reading 6's twin rule from
`docs/architecture/crpg-net.md:231-234`).

State-op and host-fact filtering lives in `crates/crpg-net/src/interest.rs`.
`FieldMask` gains five fields after `health`; `Default` still denies
everything (`crates/crpg-net/src/interest.rs:39-46`):

```rust
pub struct FieldMask {
    pub presence: bool,     // unchanged meaning
    pub health: bool,       // unchanged meaning
    pub entry: bool,        // Timeline Entry key (tag 13)
    pub pools: bool,        // Pool ops (tag 14)
    pub effects: bool,      // Effect ops (tag 15)
    pub order: bool,        // Order op (tag 16); N2's controlling-peer
                            // default is the host's job (Reading 7)
    pub stats: bool,        // Stat ops (tag 17)
}
```

`ViewerContext` gains, after its existing fields (field order is contract,
`crates/crpg-net/src/interest.rs:74-92`): per-row `EntityFact` carries
optional `entry`, `pools: Vec<PoolFact>`, `effects: Vec<EffectFact>`,
`order: Option<OrderFact>`, `stats: Vec<StatFact>` inputs (values only ever
read when the matching `FieldMask` flag is set); plus encounter-level inputs
`mode: Option<(u8, Option<u8>)>` + `mode_granted: bool`, `clock:
Option<EncounterClockInput>` + `clock_granted: bool`, and the viewer-bound
host facts `pause: Option<(bool, Option<u8>)>` + `pause_granted`,
`pause_rights: Option<(bool, bool)>` + `pause_rights_granted`,
`auto_pause: Option<Vec<u8>>` + `auto_pause_granted`,
`turn_timer: Option<u64>` + `turn_timer_granted`.

Op ownership (extends `docs/architecture/crpg-net.md:451-457`):

| Op (tags) | Owner |
|---|---|
| `EntityEnter`/`Health`/`Turn`/`EntityLeave`/`Despawned` | `InterestView` (unchanged) |
| `Mode`, `EncounterClock`, `Entry`, `Pool`, `Effect`, `Order`, `Stat`, `PauseRights`, `AutoPause`, `TurnTimer` | `InterestView` |
| `Spawned`/`Died`/`ActionResolved`/`TurnStarted`/`EncounterEnded` (unchanged) and `RoundStarted`/`ModeChanged`/`OrderEnded`/`Joined`/`Left` | `project_events` |
| `Clock` (tag 23) and `Receipt` | host |

### Snapshot transfer `0x82`

`crates/crpg-net/src/protocol_transfer.rs` adds, beside the frozen `0x81`
types (`crates/crpg-net/src/protocol_transfer.rs:277-312`):

```rust
pub const TRANSFER_VERSION_82: u8 = 0x82;
pub const MAX_SNAPSHOT_CHUNKS_82: u8 = 64;     // 64 × 32,768 = 2,097,152
pub const MAX_SNAPSHOT_BYTES_82: usize = MAX_CHUNK_BYTES
    * MAX_SNAPSHOT_CHUNKS_82 as usize;         // compile-time assert equal
pub const MAX_ENCODED_BODY_BYTES_82: usize = 2_087_958; // pictorial math below

pub struct SnapshotBody82 {
    pub entities: Vec<SnapshotEntity82>,   // ≤ MAX_VISIBLE_ENTITIES, ascending NetId
    pub encounter: Option<EncounterFact82>,
}
pub struct SnapshotEntity82 {
    pub entity: NetId,
    pub health: Option<HealthFact>,        // unchanged 0x81 fact
    pub entry: Option<i32>,                // Entry key; Some only if the
                                           // entity holds a timeline entry
    pub combatant: Option<CombatantFacts82>,
}
pub struct CombatantFacts82 {
    pub pools: Vec<PoolFact82>,            // ≤ MAX_WIRE_POOLS, distinct ids
    pub effects: Vec<EffectFact82>,        // ≤ MAX_WIRE_EFFECTS, distinct Ulids
    pub order: Option<OrderFact82>,
    pub stats: Vec<StatFact82>,            // ≤ MAX_WIRE_STATS, distinct names
}
pub struct PoolFact82     { pub pool: Ulid, pub current: u32, pub max: u32 }
pub struct EffectFact82   { pub effect: Ulid, pub expires_round: u32 }
pub struct OrderFact82    { pub ability: Ulid, pub target: EntityRef82,
                            pub repeat: bool }
pub struct EncounterFact82 {
    pub mode: u8,                          // 0 or 1
    pub pending: Option<u8>,               // 0 or 1 when Some
    pub clock: Option<ClockFact82>,        // Some in RT, None in TB at all
}
pub struct ClockFact82    { pub round: u64, pub elapsed: u64, pub round_ticks: u32 }
```

`target` in `OrderFact82` must be a `SnapshotEntity82::entity` present in
the body when the order is `Some`, just as `active` must be present in the
`0x81` body (`tasks/T024.md` S§6 body rule). Missing: `u8` proves presence,
so `EntityRef82 = NetId` + a per-variant rule — `Some` orders whose target
is absent decode `Malformed`.

The maximal-body arithmetic (re-computable; reviewers should redo it):
per entity worst case = id ≤10, health ≤12, entry ≤6, combatant flag 1,
pools 1+8×38 = 305, effects 1+16×33 = 529, order 1+38 = 39, stats 1+16×71 =
1,137 → 2,039; ×1,024 = 2,087,936; header 2 + encounter ≤20 → **2,087,958**
= `MAX_ENCODED_BODY_BYTES_82`. It fits exactly under
`MAX_SNAPSHOT_BYTES_82 = 2,097,152` (9,194 bytes slack). `64 × 32,768 =
2,097,152` keeps the compile-time equality; chunk frame size (≤ 32,815
bytes) stays inside `MAX_DELTA_FRAME_BYTES` (`crates/crpg-net/src/protocol.rs:58-95`).

### Who builds each fact, and what only gets encoded

- `crpg-sim` builds every hashed fact — mode/pending/clock from `World::combat()`
  (`crates/crpg-sim/src/world.rs:1239-1241`, fields at
  `crates/crpg-sim/src/combat.rs:831-856`), timeline order with keys from
  `World::timeline()` (`crates/crpg-sim/src/world.rs:1165-1167`;
  `crates/crpg-sim/src/timeline.rs:128` ascending `(key, id)`), pools
  (`crates/crpg-rules/src/resource.rs:142-158`), effects
  (`crates/crpg-sim/src/combat.rs:190-198`), orders
  (`crates/crpg-sim/src/combat.rs:282-285`), and stat values
  (`crates/crpg-sim/src/combat.rs:246-250`) — and journals the three events
  plus `Joined`/`Left` (`crates/crpg-sim/src/history.rs:89-170`).
- `crpg-net` only encodes bytes and filters by grants: it reads no `World`
  (`docs/architecture/crpg-net.md:437-445`); the interest projection
  consumes host-supplied values.
- `crpg-server` (T141, out of scope) owns the projection wiring: it maps
  sim facts into `ViewerContext`/`EventCandidate` inputs and decides the
  `order` default to the controlling peer (N2), which peers hold pause
  rights (ADR-0029 P4, `docs/adr/0029-kernel-range-and-time-model.md:290-297`),
  the auto-pause toggles (P6, `:303-309`), and the turn-timer deadline
  (P8, `:314-316`). T140 leaves host.rs's `candidate()` withholding of the
  five events in place (`crates/crpg-server/src/host.rs:2484-2489`) and its
  heartbeat emission policy (interval in N4, "at a policy interval",
  `:342-345`) entirely to T141.

## Behaviour

1. **Version gate.** v3's `decode_intent`/`decode_delta` compare the first
   byte to `PROTOCOL_VERSION` = 3, refuse anything else with
   `UnsupportedVersion`, refuse empty input with `Malformed`, check the
   whole-frame cap before any read (4,096 / 65,536), refuse lane ≠
   `LANE_COMBAT` with `WrongDirection` and trailing bytes with `Malformed`
   (`crates/crpg-net/src/codec_v2.rs:278-285`, `:595-628` shape, byte 3 in
   place of 2). v1 and v2 codecs are untouched and keep refusing byte 3.
2. **First failure wins, fields left to right.** Tag byte first
   (unassigned tag → `UnknownMessage`), then each field checked as read:
   an overflowing varint → `Malformed`; a value above its cap →
   `LimitExceeded`; semantic failures (`NetId` 0, `mode` not 0/1, reason not
   1..=4, trigger not 1..=5, `AutoPause` not ascending, duplicate pool id /
   effect Ulid / stat name) → `Malformed`. Collections grow by push,
   lengths compared before allocation
   (`docs/architecture/crpg-net.md:220-228` rule).
3. **Intent encoding/decoding.** All six `IntentBody` variants round-trip;
   `RequestMode` accepts only 0/1; `seq` outside `SEQ_FIRST..=SEQ_LAST` and
   `NetId` 0 are `Malformed` on both sides (`crates/crpg-net/src/protocol.rs:250-253`).
4. **Delta round-trip.** Tags 0..=10 encode to exactly the v1/v2 payload
   bytes (`crates/crpg-net/src/protocol_v2.rs:82-86` law); delta `event_seq`
   accepts full `u64` per `docs/architecture/crpg-net.md:80-81`.
5. **State-op production and grants.** `InterestView::project_delta` in a
   v3 session emits: a `Mode` op when `mode_granted` and the inputs are set
   (none when there is no encounter); an `EncounterClock` op when
   `clock_granted` and the clock input is `Some`; per visible key, in
   ascending `NetId`: `Entry` if `fields.entry` and the row has a key, then
   `Pool` per pool if `fields.pools`, `Effect` per effect if
   `fields.effects`, `Order` (Some or cleared) if `fields.order`, `Stat`
   per stat if `fields.stats`; then `PauseRights`/`AutoPause`/`TurnTimer`
   when granted and changed; at most `MAX_PAGE_OPS_V3` ops. A viewer
   without a grant sees no new op at all, never zeros standing in for
   hidden facts (the crate's no-open-bag trap, `crates/crpg-net/AGENTS.md`).
6. **Suppression is whole-event.** `ModeChanged` emits only when both
   `mode` and `round` are `Some`; the others are single-fact. A withheld
   event consumes no per-client sequence (`docs/architecture/crpg-net.md:231-234`).
7. **Ordering.** `Entry` ops appear in authoritative `(key, NetId)`
   ascending order so the client rebuilds the shared visible order
   (N1 quotes R14 rec 5 / R16 rec 5,
   `docs/adr/0029-kernel-range-and-time-model.md:324-331`); pools follow
   authored pool order; effects the ascending effect-Ulid order
   (`crates/crpg-sim/src/combat.rs:216-218`).
8. **Heartbeat.** The host emits a frame whose only op is `Clock` (tag 23)
   when pause changes and at a policy interval while ticking (N4, `:342-345`);
   the interval is a tightening-only documented default
   (`POLICY_CLOCK_HEARTBEAT_TICKS: u64`, proposed 100, inside the 200-tick
   freshness window of `crates/crpg-net/src/protocol.rs:159-165`). A
   heartbeat frame never carries other ops. Its `server_tick` header field
   is the authoritative tick at send; the replica takes `server_tick` from
   it, so an eventless ticking TB fight keeps every client fresh.
9. **Receipts.** `Stop` in TB → `Rejected(WrongMode)`; `Pause`/`Unpause`
   without the right → `Rejected(NoPauseRight)` (ADR-0029 P4's typed-code
   rule, `:296-297`); `RequestMode` for an RT-incapable encounter →
   `Rejected(ModeUnsupported)` (T2 wording, `:189-191`); sim gameplay
   failures reach the receipt too: `StatEvaluation`,
   `EncounterTerminal`, `RulesetMismatch`, `PlacementInUse`,
   `NotCreature`, `AlreadyParticipant`, `NoEntrants`
   (`tasks/T137.md:823-824`, `tasks/T139.md:1914-1917`). Codes 1..=20 keep
   their v1 bytes; 0 and 31..=255 are `UnknownMessage`.
10. **Snapshot body.** `project_snapshot` emits the extended body for a v3
    view: presence still makes an entity, the health grant adds `health`
    (`docs/adr/0028-snapshot-transfer-subprotocol.md:32`), the new grants
    add `entry`/`combatant` facts, `mode_granted` adds `encounter`; a fact
    that does not exist in a TB-only world (`clock`) is absent by
    construction, the ADR-0029 absence-compatibility pattern
    (`:191-193`). A v3 body transfer begins `0x82`; `0x81` bodies stay
    valid and callable; each body decoder refuses the other's version with
    `UnsupportedVersion`.
11. **Snapshot order and bounds.** `0x82` entities stay strictly ascending
    `NetId`; `pool`/`effect`/`stat` vectors are bounded
    (`MAX_WIRE_POOLS`/`MAX_WIRE_EFFECTS`/`MAX_WIRE_STATS`) and duplicate
    ids/names are `Malformed`; the body holds only facts granted at
    capture (ADR-0028/T024 S§11 rule 6,
    `docs/adr/0028-snapshot-transfer-subprotocol.md:32`).

## Errors

All v3 codec failures reuse the six-variant `CodecError`
(`docs/architecture/crpg-net.md:52-55`): `FrameTooLarge`,
`UnsupportedVersion` (wrong first byte in either direction, including
`0x81`/`0x91` reflexes), `WrongDirection` (lane or mirrored tag),
`UnknownMessage` (unknown tag, unassigned reason/trigger/mode bytes).
Bounds failures are `LimitExceeded`; shape failures (`Malformed`) cover cold
varints, non-ascending collections, `NetId` 0, absent
`OrderFact82` targets, `mode` ∉ {0,1}, an `AutoPause` trigger outside
1..=5 (an empty trigger list is allowed: all off);
the field order check follows the v1
field-order discipline (`tasks/T024.md` S§6); Display strings are
`<Variant> at codec/v3: <area>`, the `<VariantName> at <module>/<area>`
pattern (`docs/architecture/crpg-net.md:83-85`). The
interest extension reuses `InterestError` unchanged
(`crates/crpg-net/src/interest.rs:123-127`), since checking before mutation
is already its rule (`docs/architecture/crpg-net.md:489-493`); a v3 page
over `MAX_PAGE_OPS_V3` is `TooManyFacts`, and the v1/v2 bounds apply
unchanged to old-vocabulary views.

## Tests

Named cases, with literal bytes written as byte-oracle tests inside the
named suites (no golden files; the v3 oracles are recorded next to the v2
pattern in `crates/crpg-net/tests/events_v2.rs:276-501`):

1. `crates/crpg-net/tests/codec_v3.rs::version_gates_refuse_each_other` —
   v3 `decode_intent` on the byte `01` (v1 EndTurn intent) is
   `UnsupportedVersion` and the same for `02`; a v1 `decode_intent` on a v3
   intent whose frame is otherwise valid is `UnsupportedVersion`; same for
   deltas all around; transfer/resume decoders on v3 bytes are
   `UnsupportedVersion`.
2. `...::intent_tag_oracles` — literal bytes: minimal `Pause` intent for
   epoch all-zero, lane 0, seq 1, observed_tick 0, actor 1 encodes to
   `03 00` … and round-trips; `EndTurn` (tag 1) reproduces the frozen v1
   bytes from `crates/crpg-net/tests/codec.rs:277-291` with the version
   byte changed 1 → 3 and nothing else; `RequestMode { mode: 1 }`, `Stop`
   and `Unpause` each pin their byte string.
3. `...::request_mode_bounds` — `mode` byte 2 and 255 decode
   `UnknownMessage`; the intent frame with 4,097 bytes is `FrameTooLarge`;
   `seq` 0 and `SEQ_LAST + 1` are `Malformed`; `NetId` 0 is `Malformed`.
4. `crates/crpg-net/tests/events_v3.rs::delta_tag_oracles` — literal
   bytes for each of tags 11..=26, written max/min pairs, including:
   `Mode { mode: 1, pending: Some(1) }` = `0B 01 01 01 01` (5 bytes);
   `Entry { entity: 1, key: -1 }` pins the signed-varint encoding of `key`,
   including `i32::MIN` and `i32::MAX`; `Stat` with a 64-byte name at
   `MAX_WIRE_STRING_BYTES` + 1 is `LimitExceeded` without reading the body.
5. `...::new_tags_refused_by_old_versions` — every tags 11..=26 op inside a
   valid v2 envelope is `UnknownMessage` in v2, and every tag 0..=10 op
   round-trips in both v2 and v3 with identical bytes except the version
   byte.
6. `...::receipt_code_space` — `ReceiptStatus::Rejected(ModeUnsupported)`
   encodes the code byte `21`; codes 0 and 31 decode
   `UnknownMessage`; codes 1..=20 round-trip through the v2 codec byte for
   byte and through v3; the v1 superset test re-pins the frozen values
   (`crates/crpg-net/tests/codec.rs:178-207`).
7. `...::auto_pause_bounds` — `AutoPause` with 32 ascending triggers
   round-trips; 33 triggers is `LimitExceeded`; a non-ascending list,
   a 0 trigger, and a 6 trigger are `Malformed`; an empty list is allowed
   (all triggers off).
8. `crates/crpg-net/tests/interest_v3.rs::per_field_grants` — for each new
   fact (mode+pending, clock, entry, pools, effects, order, stats):
   grant on → op emitted once; grant off → no op, no byte changed in the
   rest of the page compared to the `fields.presence`-only baseline
   (`crates/crpg-net/tests/interest.rs:1982-...` hidden-input-invariance
   pattern extended).
9. `...::viewer_without_grants_sees_nothing` — a fully denied
   `FieldMask@Default` plus no encounter/host grants produces exactly the
   v1 page a T027b view would produce, byte for byte (the old vocabulary is
   unchanged).
10. `...::page_layout_and_bound` — a fresh page emits enter section, health
    section, then per-entity new-fact sections, then one removal op per
    removed key, in the owner-table order (`docs/architecture/crpg-net.md:451-457`);
    a page at exactly `MAX_PAGE_OPS_V3` succeeds and one more `fact` row is
    `TooManyFacts` with the view unchanged.
11. `crates/crpg-net/tests/transfer_v3.rs::body82_literal_examples` — the
    minimal `0x82` body (no entities, no encounter) is `00 00` exactly;
    one entity with health and one pool pin their bytes and round-trip
    byte-identically on re-encode (the T024 body rule, S§6).
12. `...::body82_maximal_assembly` — 1,024 entities, every fact at maximum
    values, is exactly `MAX_ENCODED_BODY_BYTES_82` bytes, assembles as 64
    chunks of the 8-segment canonical split, publishes atomically, and
    re-encodes identically; 1,025 entities is `LimitExceeded` on count
    before allocation.
13. `...::order_target_must_be_present` — a `Some` order whose target is
    not among the body entities decodes `Malformed`; removing the ordered
    entity from the body sanitizes or forbids the pair on encode.
14. `crates/crpg-net/tests/replica.rs::v3_op_table` — each state op
    replaces its stored fact; `Mode`/`Entry` on an unknown entity is
    `UnknownEntity`; the `Clock` op updates `server_tick` (via the frame
    header) and stores pause state; every failure leaves the replica equal
    to its pre-call clone (`crates/crpg-net/tests/replica.rs` pattern).
15. `crates/crpg-net/tests/events_v3.rs::heartbeat_rules` — a frame mixing
    `Clock` with any other op is `Malformed` at encode; two heartbeats in
    one frame is `Malformed`; the simulated-transport run proves the
    observed tick stays within `POLICY_OBSERVED_TICK_WINDOW` over a
    1,000-tick eventless TB fight; pausing stops the `server_tick`
    progression but still emits a `Clock { paused: true, .. }` at the
    change instant only.
16. `crates/crpg-net/tests/conformance.rs::<new case>` — end-to-end:
    `Stop` in TB → receipt `WrongMode`; `Pause` without right →
    `NoPauseRight`; `RequestMode` on a TB-only ruleset → `ModeUnsupported`;
    every rejected input leaves the authoritative hash unchanged
    (`:243-251` pattern).

## Out of scope

- Any code change; this slot is doc-only.
- Everything in `crpg-server` (T141): projection wiring of the new facts
  (`PeerView::candidate()` stays withholding five events,
  `crates/crpg-server/src/host.rs:2484-2489`), pause state and rights
  (ADR-0029 P1–P7), auto-pause evaluation (P6), the turn timer (P8),
  heartbeat emission policy (N4's "at a policy interval"), placement and
  join/leave admission codes' *host-side* meaning, the creature-record
  projection `World::creatures()` (`tasks/T139.md:1914-1917` host-side half).
- T026p lane 1 and the lane-0 `DeclareMove` verb + `Moved` projection
  (`docs/adr/0036-movement-and-navigation.md:413-417`, `:540`) — see Q2.
- `crpg-sim` changes: N5 history facts are already built and journaled;
  nothing in sim moves.
- Changing v1, v2, the `0x81` transfer or the `0x91` resume bytes, tags,
  maxima or policy defaults.
- Any new dependency; `crpg-net`'s allowed-dependencies list is unchanged
  (`docs/architecture/crpg-net.md:349-360`).
- Roll transparency beyond the grant plumbing (Q1).

## Definition of done

- [ ] The adjudicated contract (post blind-draft review) is approved by the
      user before any code starts; this file records the blind draft only.
- [ ] T140a–T140d each pass: `cargo fmt --all`, `cargo clippy -p crpg-net
      --all-targets --locked -- -D warnings`, `cargo test -p crpg-net
      --locked`, plus the workspace lints:
      `python tools/lint/deps.py`, `python tools/lint/determinism.py`
      and the unittest discover run (`crates/crpg-net/AGENTS.md` DoD list).
- [ ] Existing wire goldens, hashes and pins all stay byte-identical and a
      test proves it (the frozen suites above still pass unchanged).
- [ ] Every numbered Behaviour item has a named test; every new rejection
      code has a positive and a first-invalid case.
- [ ] `docs/architecture/crpg-net.md`, `crates/crpg-net/AGENTS.md` and a new
      ADR (protocol v3) file are written by the implementation, in the
      existing docs pattern; agent-log entries signed.
- [ ] v1, v2, `0x81`, `0x91`: nothing moves; verified by the untouched
      freeze suites.

## Questions for the user

1. **Roll transparency (Open question 8; required by the brief).**
   ADR-0029 recommends letting a ruleset opt in to disclosing an actor's own
   raw roll and modifier to its controller, DCs hidden unless public
   (`docs/adr/0029-kernel-range-and-time-model.md:515-519`, D30 answer 8 at
   `tasks/DECISIONS-D30-DRAFT.md:46`). Options:
   - (a) Fold it into v3 now: two new `Roll`/`RollDetail` event ops with
     per-field grants; +2 tags, +1 suite.
   - (b) Defer to T141/T143 (the server preview path), v3 ships without.
   - Impact in play: (a) lets a ruleset show "you rolled 14 + 3" to the
     acting player as it happens; (b) keeps the wire strictly symbolic
     (ADR-0019's no-rolls-on-the-wire ban stays fully intact,
     `docs/adr/0021-authoritative-host-capture.md:72`).
   - Recommendation: (b), deferring. ADR-0019's ban has no exception text in
     the code or the ADR yet, and the sim-side `ActionOutcome` capture is
     contract-bound to *not store* the roll
     (`docs/adr/0021-authoritative-host-capture.md:72`), so the fact v3
     would reveal does not exist anywhere yet. Opening it changes sim
     capture, history and v3 together — its own task, not a rider on this
     one.
2. **Fold `DeclareMove`/`Moved` into v3?** (ADR-0036 L6: "This needs a
   protocol version after T140's, or it folds into T140 if that contract is
   still open" — it is still open, `docs/adr/0036-movement-and-navigation.md:568-571`.)
   Options: (a) fold (v3 grows one intent verb + one event op + position
   grants); (b) don't (T026p defines v4). Impact: (a) saves a version bump
   and lets TB moves be visible the day T036c's history lands; (b) keeps
   this version's review surface smaller. Recommendation: (a), because the
   contract is open and the arbiter can pick either; folding avoids a v4
   churn for one verb.
3. **`0x82` bounds.** This draft raises the transfer chunk budget to 64
   chunks / 2 MiB to fit 1,024 maximally-granted combatants with stat names
   at 64 bytes (Interface section). Options: (a) as drafted;
   (b) tighten `MAX_WIRE_STATS` to 4 and keep the 1 MiB / 32-chunk
   envelope. Impact: (a) a big ruleset shows every pool/effect/stat at
   resync; (b) more-granted rulesets get a partially-filled sheet until the
   next delta heals it. Recommendation: (a); the arithmetic holds, the
   frame-size invariant stays untouched, and segment layout is canonical.
4. **Pause reason `Player(peer)` on the wire.** ADR-0029 P5's reason carries
   the pausing peer (`docs/adr/0029-kernel-range-and-time-model.md:299-301`),
   but no peer identity concept exists on the wire today. Options:
   (a) v3 carries a peer index (host-assigned seat number, granted with
   `pause` rights); (b) v3 carries `Player` without identity, UIs say
   "a player paused". Impact: (a) is exact; (b) avoids introducing a peer
   number that T026p/QA must live with. Recommendation: (b) for v3; if
   T141 identifies a need, it lands with T141's own ADR.
5. **Stat value grant granularity.** The draft bundles all stat values
   under one `stats` flag. Options: (a) one flag; (b) per-stat-name grants.
   Impact: (a) is small; (b) is finer but adds a string-keyed grant list on
   the wire needs interning that doesn't exist. Recommendation: (a), with
   (b) revisited when a ruleset asks.

## Agent log

- 2026-11-10 (UTC) · opencode/glm-5.3-flash-free + T140 blind draft b · Wrote
  the blind contract draft (version 3 vocabulary, tag map, grants, `0x82`
  snapshot body, heartbeat-as-op) from `tasks/T140-facts.md`, ADR-0029 §4,
  ADR-0019 and ADR-0028, verified every cited `path:line` against the branch
  tip; nothing here is approved.
