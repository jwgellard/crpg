# T046-draft-a — client replica: storage, delta application, read queries (`crpg-net`)

Sole crate: `crpg-net`, Phase 5 (M5). Status: **blind draft, attempt 1; nothing
here is approved.** Contract, not code. Prerequisites are T024 (the `0x81`
snapshot wire) and T027b (per-viewer projection); the plan row is
`docs/IMPLEMENTATION_PLAN.md:248`.

Facts, read on this branch:

- The v2 delta frame is `lane`/`server_tick`/`event_seq`/`ops`
  (`crates/crpg-net/src/protocol_v2.rs:67-77`) and its ops are the frozen v1
  eight wrapped in `Legacy` plus three event variants
  (`crates/crpg-net/src/protocol_v2.rs:88-120`). No epoch is on a delta frame.
- The v2 decoder checks the frame cap, version, lane and op count, and
  **deliberately does not range-check `event_seq`**
  (`crates/crpg-net/src/codec_v2.rs:564-566`, `:604-613`). The reassembly pool
  is the only current guard (`0` or above `SEQ_LAST` is
  `TransferError::DeltaSequence`, `crates/crpg-net/src/reassembly.rs:914-917`),
  and T047's lane-0 slice bypasses that pool.
- A bound session's first delivery is a fresh state page from `event_seq` 1
  (`crates/crpg-server/src/host.rs:689-690`). A lane-0 grant narrowing clears
  the log, bumps the generation, sets `next_event_seq = 1` and stages
  `full_state_ops` again (`crates/crpg-server/src/host.rs:840-857`, `:1659-1671`,
  layout `:2436-2457`).
- A snapshot publication is `{ transfer, base_tick, cursor, body }`, `cursor`
  being the last included delivery `event_seq`
  (`crates/crpg-net/src/reassembly.rs:276-286`); the body is entities plus an
  optional turn (`crates/crpg-net/src/protocol_transfer.rs:277-312`) and the
  codec requires ascending ids with `turn.active` inside the body
  (`crates/crpg-net/src/codec_transfer.rs:32-57`).
- The body's health facts are `Option`-shaped: absent means undisclosed, never
  zero (`crates/crpg-net/src/protocol_transfer.rs:285-312`).
- The only replica today is a test double that buffers out-of-order frames and
  drops duplicates (`crates/crpg-net/tests/conformance.rs:282-341`); it is an
  oracle for T018c, not a client. No `ReplicaWorld` exists.
- T046's obligations to T047 are exactly: apply a v2 `DeltaFrame` in order,
  error on a gap rather than skip, list entities ascending by `NetId`, keep
  absent health distinct from zero, expose the turn
  (`docs/adr/0033-godot-bridge.md:525-529`); a refused frame is fatal code 53
  (`:342`) and is never skipped (`:346-349`). The rules this draft consumes are
  N1–N9 (`tasks/T027b.md:568-593`); T046 reads the wire, not `InterestView`.

**Dependency: none.** The module imports `core::fmt` and crate items only
(`protocol`, `protocol_v2`, `protocol_transfer`). No `Cargo.toml`, lock, dev-dep
or `ALLOWED` change. No serde: a deserialisable replica could be restored with
stale authorization (`tasks/T027c.md:23`).

## Decision

In each decision (a) is the recommendation; (b) and (c) are rejected with their
costs.

- **D1. One new module `replica`, one type `ReplicaWorld`.** Path
  `crates/crpg-net/src/replica.rs`, registered in `src/lib.rs` between
  `reassembly` and `resume`. E012 and the spec name this owner and type
  (`tasks/E012-binary-crate-naming.md:72`, `docs/CRPG_ENGINE_SPEC.md:874`).
  Rejected: (b) putting storage in `crpg-godot` — the engine-free half must be
  testable on Linux with no Godot; (c) reviving the conformance `Replica`
  (`crates/crpg-net/tests/conformance.rs:285`) — its buffering and
  drop-on-duplicate rules are the opposite of this contract, and it must stay an
  independent oracle.

- **D2. Storage is one `Vec<ReplicaEntity>` sorted by `NetId`.** Lookup is a
  binary search; the listing is a slice. Rejected: (b) `BTreeMap<NetId, _>` —
  T047 needs one bulk ascending pass (`docs/adr/0033-godot-bridge.md:271`), and a
  map hands it an iterator or a second allocation; (c) parallel `Vec<NetId>` +
  `Vec<Option<HealthFact>>` arrays shaped like the FFI — two lengths that can
  desync and the same order invariant asserted twice. `HashMap`/`HashSet` appear
  nowhere, in the module or its tests. At 1,024 rows an insert or remove is
  `O(n)` with `n ≤ 1024`.

- **D3. A row holds presence and health only:**
  `ReplicaEntity { net_id: NetId, health: Option<HealthFact> }`. Presence is the
  row; health absent is `None`, never a zeroed fact. Rejected: (b) an
  `enum { Unknown, Known(HealthFact) }` — `Option` is already this crate's
  absent idiom and exactly what `SnapshotEntity.health` uses
  (`crates/crpg-net/src/protocol_transfer.rs:290-291`); (c) a separate `dead`
  field — `dead` already travels inside `HealthFact`
  (`tasks/T027b.md:366-369`). T047 takes presentation from its own table
  (`docs/adr/0033-godot-bridge.md:239-249`).

- **D4. The turn is one `Option<TurnFact>`, replica-wide.** Outer `None` = no
  turn fact disclosed yet in this epoch; inner `active: None` = "no disclosed
  active actor" (N7). Rejected: (b) a struct with `round: 0` meaning "none" — it
  makes N7 and "no fact" indistinguishable; (c) two root fields (`active`,
  `round`) — the same fact split, and `TurnFact` is already the T024 type used
  by the op and the body (`crates/crpg-net/src/protocol_transfer.rs:306-312`).

- **D5. `apply_delta` takes a decoded `&protocol_v2::DeltaFrame`.** The bridge
  decodes (`docs/adr/0033-godot-bridge.md:151-153`); the replica must not
  duplicate the codec, and it must not accept ops and a sequence separately
  (the caller could pair one page's ops with another page's number). Rejected:
  (b) `&[u8]` and decode here — a second decode path, and `CodecError` (code 52)
  would blur into `ReplicaError` (code 53); (c) `&[DeltaOp]` + `event_seq` —
  invites the mismatch above.

- **D6. Sequence law: contiguous, and `event_seq == 1` is a fresh-state page.**
  In order: `event_seq` must be `1..=SEQ_LAST`; `1` always begins a fresh
  replica (item 5); otherwise it must be exactly `last + 1`; equal to
  `last` is `DuplicateEventSeq`, any other regression is `EventSeqRewind`, and
  anything above `last + 1` is `EventSeqGap`. Rejected: (b) treating every
  backward sequence as an error and leaving
  the T022 lane-0 narrowing reset to the caller — the reset is announced only
  by `event_seq` restarting at 1 (`crates/crpg-server/src/host.rs:852`), so
  without this rule the N9 path desynchronises the client forever, and a public
  `reset()` would be a mutating method with no wire fact behind it; (c) the
  conformance oracle's buffer-and-drain (`crates/crpg-net/tests/conformance.rs:282-297`)
  — it hides exactly the faults the brief says must be errors. **Consequence,
  stated not absorbed:** a re-delivered frame 1 after progress is
  indistinguishable from a reset and clears the view. Lane 0 is reliably ordered
  and `LocalLink` is FIFO (`docs/adr/0033-godot-bridge.md:95-99`), and the
  reassembly pool already drops superseded and byte-identical frames
  (`crates/crpg-net/src/reassembly.rs:918-936`); a caller that may re-deliver
  must filter before `apply_delta`.

- **D7. The replica validates every op, including notices and events.** It is
  the only component that knows the row set, so N1 is enforced once here and
  T047 merely renders. Rejected: (a) ignoring `Spawned`/`Died`/events — an
  unknown id would silently pass and N1 would have no enforcing behaviour;
  (b) returning events to the caller — the bridge already holds the frame it
  decoded (`docs/adr/0033-godot-bridge.md:152`).

- **D8. Atomicity by two passes, not by cloning.** Pass 1 reads only and builds
  a staging list of at most `MAX_DELTA_OPS` changes; pass 2 commits in place and
  cannot fail. `Err` therefore changes nothing, and an `Ok` commits rows, turn,
  counters and sequence together. Rejected: (a) clone and swap — a row vector
  and health copy per frame; (b) mutate and roll back — re-inserting a removal
  would restore a retired id and rolling back `max_seen` would forget a mint.

- **D9. `apply_snapshot(cursor, &SnapshotBody)` is the only whole replacement.**
  It replaces rows, health and the turn, and sets `last_event_seq` from the
  publication cursor. Entity order in the body never matters (it is a set);
  duplicates are an error. Rejected: (a) taking `&Publication` — couples the
  replica to the reassembly module and to `TransferId`; (b) body only — the next
  delta cannot be ranged and a whole transfer leg is misread as a gap; (c)
  requiring ascending input — `codec_transfer` already guarantees it on the wire
  (`crates/crpg-net/src/codec_transfer.rs:36-49`), so the check would be dead
  there and only reject hand-built test input.

- **D10. Reuse is detected from one monotone counter plus one fresh-page flag.**
  `max_seen` is the highest `NetId` ever observed (enter or snapshot body).
  A delta `EntityEnter` for a live id is `DuplicateEntity`; for an id at or
  below `max_seen` it is `ReusedEntityId` — except inside a fresh page's enter
  section, where the ids are that page's retained keys. The section is a
  `fresh: bool`: armed by a seq-1 frame, disarmed by the first op that is not an
  `EntityEnter` (or by a snapshot), because the host's full state is all enters
  first (`crates/crpg-server/src/host.rs:2436-2457`) and a page over 256 ops is
  split (`:1536`). A snapshot body may name any low id (ids are retained).
  Rejected: (a) a retired-id set — unbounded, and T027b rejected the same table
  on the minting side (`tasks/T027b.md:129-132`); (b) no reuse check — N3 would
  be unenforced and a re-issued id would silently alias another entity; (c) no
  fresh-page exemption — any reset larger than one frame (256 ops) would be
  rejected as reuse, so N9 would fail at the stated 1,024 bound.

- **D11. One closed, payload-carrying `ReplicaError`; every variant is fatal.**
  No variant is recoverable: a bad frame is never skipped
  (`docs/adr/0033-godot-bridge.md:346-349`), so the caller maps any `Err` to
  code 53. Payloads (`expected`/`got`, `entity`, `cursor`) exist because this is
  the failure class debugged from `last_error`, and `Debug` is then usable as-is.
  Rejected: (a) payload-free like `CodecError`
  (`crates/crpg-net/src/codec.rs:52`) — the bridge cannot say what broke;
  (b) reusing `CodecError` — that is the byte layer (code 52), and `WrongLane`
  after a successful decode is not a decode error; (c) a recoverable
  "skip frame" variant — the divergence the ADR forbids.

- **D12. v2 only.** `apply_delta` accepts `protocol_v2::DeltaFrame`; there is no
  v1 entry point. Rejected: (a) both versions — T047 binds a V2 session
  (`docs/adr/0033-godot-bridge.md:90`) and the slice has no V1 client; (b) v1
  only — it has no event ops, so no `Spawned`/`Died` and no forward path. A v1
  session's ops are the same eight tags, so a future task adds a two-line
  `Legacy` adapter without changing this contract. From a V2 session the replica
  accepts legacy-only pages, event-bearing pages and snapshot publications; from
  a V1 session it accepts nothing.

- **D13. Read queries are a slice plus point lookups; no tick, no allocation.**
  `entities()` is the sorted slice T047 walks once; `entity`/`contains`/`health`
  are binary searches; `turn`/`active`/`round`/`epoch`/`last_event_seq` are
  copies. Rejected: (a) a `view()` that clones the state; (b) iterator queries —
  the bridge would `collect` anyway; (c) exposing `server_tick` — the bridge has
  the frame it decoded, and a tick in the replica would collide with
  `Publication.base_tick`.

## Interface (`crates/crpg-net/src/replica.rs`)

```rust
use core::fmt;

use crate::protocol::{
    NetId, SessionEpoch, MAX_DELTA_OPS, MAX_VISIBLE_ENTITIES, SEQ_LAST,
};
use crate::protocol_transfer::{HealthFact, SnapshotBody, TurnFact};
use crate::protocol_v2::DeltaFrame;

/// One entity of the filtered replica. Rows are ordered by `net_id`, ascending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaEntity {
    /// The client-scoped replica id; never zero, never reused in an epoch.
    pub net_id: NetId,
    /// The last disclosed health fact, or `None` while undisclosed.
    pub health: Option<HealthFact>,
}

/// Why one frame or publication was refused. Every variant is fatal to the
/// session; see ADR-0033 code 53.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicaError {
    /// `frame.lane` is not `LANE_COMBAT`.
    WrongLane,
    /// `frame.ops.len() > MAX_DELTA_OPS`.
    TooManyOps,
    /// `event_seq` is 0 or above `SEQ_LAST`.
    EventSeqOutOfRange,
    /// `event_seq` is beyond the next expected one; frames were lost.
    EventSeqGap { expected: u64, got: u64 },
    /// `event_seq` is behind the last applied one.
    EventSeqRewind { expected: u64, got: u64 },
    /// `event_seq` is the last applied one again.
    DuplicateEventSeq { got: u64 },
    /// An op names an entity that is not live at that point (N1, N2).
    UnknownEntity { entity: NetId },
    /// An `EntityEnter` names a live id, or a snapshot body lists an id twice.
    DuplicateEntity { entity: NetId },
    /// An `EntityEnter` names an id already issued in this epoch (N3).
    ReusedEntityId { entity: NetId },
    /// More than `MAX_VISIBLE_ENTITIES` rows would be live.
    TooManyEntities,
    /// The snapshot cursor is above `SEQ_LAST` or behind the applied sequence.
    InvalidSnapshotCursor { cursor: u64 },
}
impl fmt::Display for ReplicaError { /* "<Variant> at replica" */ }
impl std::error::Error for ReplicaError {}

/// The engine-neutral client replica of one filtered v2 session.
///
/// One instance per session epoch. Clone/`PartialEq` exist for tests and
/// rollback; the type is not `Serialize` and has no restore constructor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaWorld {
    epoch: SessionEpoch,
    entities: Vec<ReplicaEntity>,   // ascending net_id
    turn: Option<TurnFact>,
    last_event_seq: Option<u64>,    // 1..=SEQ_LAST
    max_seen: Option<NetId>,
    fresh: bool,                    // inside a seq-1 page's enter section (D10)
}

impl ReplicaWorld {
    /// An empty replica for `epoch`.
    pub fn new(epoch: SessionEpoch) -> Self;
    /// The session epoch this replica belongs to.
    pub fn epoch(&self) -> SessionEpoch;
    /// Every live entity, ascending `NetId`. Never longer than
    /// `MAX_VISIBLE_ENTITIES`.
    pub fn entities(&self) -> &[ReplicaEntity];
    /// One row, by binary search.
    pub fn entity(&self, entity: NetId) -> Option<&ReplicaEntity>;
    /// Whether `entity` is live.
    pub fn contains(&self, entity: NetId) -> bool;
    /// Live row count.
    pub fn len(&self) -> usize;
    /// Whether no entity is live.
    pub fn is_empty(&self) -> bool;
    /// The row's health, or `None` for an undisclosed fact **or an absent
    /// entity**; see `contains`.
    pub fn health(&self, entity: NetId) -> Option<&HealthFact>;
    /// The last rendered turn fact; `None` before any turn op in this epoch.
    pub fn turn(&self) -> Option<&TurnFact>;
    /// The last disclosed active id, verbatim (`None` also when none is
    /// disclosed, N7). It need not be in `entities()`: N6 forbids a delta from
    /// clearing the turn, so pair it with `contains` before using it as a row.
    pub fn active(&self) -> Option<NetId>;
    /// The last disclosed round; `None` before any turn fact.
    pub fn round(&self) -> Option<u64>;
    /// The last applied delivery sequence; `None` before the first frame.
    pub fn last_event_seq(&self) -> Option<u64>;
    /// Applies one decoded v2 delta frame, or leaves the replica unchanged.
    pub fn apply_delta(&mut self, frame: &DeltaFrame) -> Result<(), ReplicaError>;
    /// Replaces the whole replica from one published snapshot at `cursor`, or
    /// leaves it unchanged.
    pub fn apply_snapshot(
        &mut self,
        cursor: u64,
        body: &SnapshotBody,
    ) -> Result<(), ReplicaError>;
}
```

`src/lib.rs` gains `pub mod replica;` in alphabetical position and one
crate-doc sentence. Every public item is documented (`src/lib.rs:2` warns
`missing_docs`). No other public item in the crate changes. The replica does not
decode bytes, check an epoch, assemble or chunk snapshots, buffer or dedupe
frames, keep events or receipts, expose `ended`, names or options, encode
intents, read a tick, or persist.

## Behaviour

Terms: **live** = present in `entities`; **known** = live, or entered earlier in
this same frame; **next** = `last_event_seq + 1`, or 1 when `None`.

1. **Construction and ownership.** `new(epoch)` is empty: no rows, no turn, no
   sequence, `max_seen == None`, `fresh == false`, `len() == 0`, `is_empty()`.
   The epoch never changes; a new epoch is a new `ReplicaWorld` with an empty
   state (N3). No
   other mutating method exists: storage changes only through `apply_delta` and
   `apply_snapshot`. The spec's "no mutating methods except `apply_delta`"
   (`docs/CRPG_ENGINE_SPEC.md:175`) is read as the apply family: T024 added the
   second wire shape after that sentence was written.

2. **Precedence, before any mutation.** `apply_delta` checks, in order:
   (a) `lane` (`WrongLane`); (b) op count (`TooManyOps`); (c) `event_seq` range
   (`EventSeqOutOfRange`); (d) the sequence law of item 4; (e) ops in order,
   item 7; (f) the final row count (`TooManyEntities`). `apply_snapshot` checks,
   in order: (a) cursor, item 15; (b) body length (`TooManyEntities`);
   (c) duplicate ids (`DuplicateEntity`); (d) `turn.active` membership
   (`UnknownEntity`). Combined faults report the first in this order.

3. **Atomicity.** `Err` changes nothing: rows, health, turn, `last_event_seq`,
   `max_seen` and `fresh` are equal to a pre-call clone. `Ok` commits all of them
   together. No input makes either function panic, index out of bounds, or
   unwrap; the commit pass performs no fallible operation.

4. **Sequence law.** `event_seq` is `1..=SEQ_LAST` (`SEQ_LAST = u64::MAX - 1`,
   `crates/crpg-net/src/protocol.rs:48`). `1` is a fresh-state page (item 5).
   Otherwise `event_seq == next` continues; `event_seq == last` is
   `DuplicateEventSeq`; any other value below `next` is `EventSeqRewind`; any
   value above `next` is `EventSeqGap`. A fresh replica's first frame must be 1.
   Every accepted frame sets `last_event_seq`, including an empty `ops` and a
   notices-only frame. Nothing is buffered and nothing is skipped.

5. **`event_seq == 1` begins a fresh replica.** The page is applied to an empty
   baseline: the previous rows and health are discarded, whether they were empty
   or not, and the page's own ops build the new state. This is the T022 lane-0
   narrowing reset (`crates/crpg-server/src/host.rs:840-857`), whose page is
   `full_state_ops` (enters, healths, turn, `:2436-2457`) chunked from 1
   (`:1535`). `max_seen` is **not** lowered. Validation and commit are one
   atomic step (item 3): a rejected fresh page leaves the old rows intact. The
   page arms `fresh` (D10), so ids at or below `max_seen` may be re-announced
   while its enter section runs (item 12).

6. **One frame at a time.** A page may be split across frames at any op boundary
   (N8): an entity can be live with `health: None`, and a `Turn` may be applied
   before the health ops of the same page arrive in a later frame. The replica
   never waits for the rest of a page and never holds a partly applied frame.

7. **Op effects.** Ops are applied in frame order. Validation uses the state
   after the ops already accepted in this frame.

   | Op (v2 `DeltaOp`) | Effect on the replica | Error |
   |---|---|---|
   | `Legacy(EntityEnter { entity })` | insert `{ entity, health: None }`; raise `max_seen` | `DuplicateEntity` if live; `ReusedEntityId` if `≤ max_seen` while not `fresh` (item 12) |
   | `Legacy(EntityLeave { entity })` | remove the row, and its health with it | `UnknownEntity` |
   | `Legacy(Despawned { entity })` | exactly as `EntityLeave` | `UnknownEntity` |
   | `Legacy(Spawned { entity })` | none (notice, item 8) | `UnknownEntity` |
   | `Legacy(Died { entity })` | none (notice; does not set `dead`) | `UnknownEntity` |
   | `Legacy(Health { entity, health, max_health, dead })` | replace the whole fact, verbatim | `UnknownEntity` |
   | `Legacy(Turn { active, round })` | `turn = Some(TurnFact { active, round })` | `UnknownEntity` when `active` is `Some` and not known |
   | `Legacy(Receipt { .. })` | none; not inspected | — |
   | `ActionResolved { actor, target, .. }` | none (event) | `UnknownEntity` for `actor`, then `target` |
   | `TurnStarted { actor, .. }` | none (event) | `UnknownEntity` |
   | `EncounterEnded { .. }` | none (event) | — |

   Health values are stored verbatim: the replica validates no relation between
   `health`, `max_health` and `dead` (that is gameplay, not shape).

8. **Notices and events are not state.** `Spawned`, `Died`, `ActionResolved`,
   `TurnStarted` and `EncounterEnded` change no row, health, turn or counter.
   `Died` does not set `dead`: `dead` travels only with the `health` grant
   (`tasks/T027b.md:366-369`). The bridge renders these events from the frame it
   decoded; the replica only proves their ids are legal at that point (N1).

9. **Health (N4).** Absent is unknown, never zero: a row starts with
   `health: None`, a query returns `None`, and a snapshot row with
   `health: None` still lists the entity (unknown, not absent). `Health`
   replaces all three fields at once. A dead entity stays live (N4, N6): no op
   removes a row because `dead: true`.

10. **Turn (N6, N7).** `Turn` replaces the fact; `active: None` is stored as
    such, so "none disclosed" and "hidden" are the same answer. No delta ever
    clears the turn, including `EncounterEnded` and a removal of the actor
    (see `active`'s doc). A snapshot replaces it, including with `None`.

11. **Removal (N2).** `EntityLeave` and `Despawned` are the same state effect;
    the distinction is a notice for the log, not a stronger removal. Exactly one
    row disappears with its health. A second removal, or any later op naming the
    id, is `UnknownEntity`.

12. **Reuse (N3).** `max_seen` is the highest id ever observed. A non-fresh
    `EntityEnter` at or below `max_seen` is `ReusedEntityId` (unless the id is
    live, which is `DuplicateEntity`). While `fresh` is armed the ceiling test
    is skipped, which is what lets a fresh page span frames; the flag clears at
    the first non-enter op (or a snapshot), so an id the page did not
    re-announce is rejected afterwards. A snapshot accepts any id in the body
    (ids are retained) and raises `max_seen`. Ids are never reissued by the
    replica; a re-entering entity arrives as a new id from the host, and the
    replica cannot link the two.

13. **Bounds (N8, B5).** After any successful call `len() <= MAX_VISIBLE_ENTITIES`
    (1,024, `crates/crpg-net/src/protocol.rs:92`) and a frame carries at most
    `MAX_DELTA_OPS` (256, `:70`) ops. Stored per instance: one row per live
    entity (8-byte id + a fixed-size optional health fact), one optional turn
    fact, one optional sequence, one optional id. No retired-id table, no event
    or receipt
    buffer, no history, no per-op scratch that outlives a call. Transient work is
    one staging list of at most 256 entries per `apply_delta`; `apply_snapshot`
    builds its sorted row vector before swapping it in, so a failure never
    touches the live rows. Nothing here grows with time.

14. **Snapshot replacement (N5).** `apply_snapshot` replaces rows and turn
    wholesale and clears `fresh`: a row absent from the body is gone with no
    removal op, a listed row keeps its `NetId`, health is exactly the body's
    `Option<HealthFact>`, and the turn is exactly `body.turn`. `Spawned`/`Died`
    notices do not exist in a body. Body order never matters; the stored order is
    ascending `NetId`. Duplicate ids are `DuplicateEntity`, more than 1,024 rows
    is `TooManyEntities`, and a `turn.active` outside the body is
    `UnknownEntity`.

15. **Snapshot cursor.** `cursor` is the last delivery `event_seq` the body
    already contains (`crates/crpg-net/src/reassembly.rs:279-281`). A cursor of 0
    on a replica that has applied nothing leaves `last_event_seq` as `None`; any
    other valid cursor sets `last_event_seq = Some(cursor)`. `cursor > SEQ_LAST`,
    or a cursor below the applied sequence, is `InvalidSnapshotCursor`. After a
    successful publication at `cursor`, the next delta must be `cursor + 1`; a
    snapshot at the applied sequence is legal.

16. **Read queries.** `entities()` is ascending by `NetId` and has no gaps;
    `entity`, `contains` and `health` agree with it; `turn`/`active`/`round`
    return the last disclosed turn fact until replaced (N6); `epoch` is the
    constructor's value; `last_event_seq` is the last applied frame or
    publication. Queries allocate nothing and are `O(log n)` or `O(1)`.

17. **No derived authority, no persistence, and determinism.** The replica
    reads only the frame/publication values: no `World`, clock, RNG, event,
    config, file or OS. It derives no health, turn, presence, area, ownership or
    legality. The same constructor epoch and the same accepted sequence of
    frames and publications give an equal replica, independent of input order
    within a snapshot body and of how a page was chunked. It cannot be saved,
    loaded or rehydrated, and it exposes no mutator besides the two apply calls.

## Rules N1–N9, and the error when the wire breaks them

| Rule (`tasks/T027b.md:568-593`) | Enforced by | Error |
|---|---|---|
| **N1** no op names an id before its enter or a published body | item 7, ids checked against the live/known set in op order; snapshot `turn.active` membership, item 14 | `UnknownEntity` |
| **N2** exactly one removal, nothing names the id afterwards | item 11; items 7 and 12 for a second removal, a reference, or a re-enter | `UnknownEntity` |
| **N3** ids never reused; a new epoch is empty; no linking | `new(epoch)`, item 12 (`max_seen`) | `ReusedEntityId` |
| **N4** absent health is unknown, not zero; `Health` replaces all; the entity stays | items 3, 9 | `UnknownEntity` for a `Health` on an absent id; absence itself is legal |
| **N5** a snapshot replaces everything including the turn; a missing id is gone | items 14, 15 | `TooManyEntities`, `DuplicateEntity`, `UnknownEntity`, `InvalidSnapshotCursor` |
| **N6** delta mode never clears health or turn; `EncounterEnded` is the end signal | items 9, 10; no op clears, and events are inert (item 8) | none — the break is a replica that invents a clear, which this contract forbids |
| **N7** `Turn { active: None }` means no disclosed actor | item 10, D4 | `UnknownEntity` only for a non-`None` unknown active |
| **N8** pages split at any op boundary; notices are not state | item 6 (per frame, no buffering), item 8 | sequence faults from a broken split: `EventSeqGap`, `DuplicateEventSeq`, `EventSeqRewind` |
| **N9** V1 gets no notices; the lane-0 reset restarts at 1 and empties first | items 4, 5; D12 (no v1 entry point) | `EventSeqRewind` for any other regression |

## Wire impact

**None.** No version, tag, field, fact, code, constant or fixture changes. The
module consumes existing `protocol_v2::DeltaFrame`, `protocol::DeltaOp` and
`protocol_transfer` values; it encodes, decodes and mints nothing. v1 bytes,
v2 bytes, decode precedence and every B5 maximum are untouched, and no golden
or hash moves.

What this crosses:

- A new public surface needs an ADR (`docs/architecture/crpg-net.md:39-40`),
  under the next free number; 0030 is reserved and 0033 is the bridge, so 0034.
- T047 is the only consumer (`docs/adr/0033-godot-bridge.md:525-529`). Nothing
  in `crpg-server` changes, and no host call site appears: the host already
  produces exactly these frames and bodies.
- `apply_snapshot` has no caller in T047's slice, where a `Transfer` entry is
  fatal (`docs/adr/0033-godot-bridge.md:230-232`). It is the T024 publication
  path for the first client that narrows, tested now so that path is not written
  blind.

## Tests (new `crates/crpg-net/tests/replica.rs`)

No new dependency. Frames and bodies are built literally or from
`codec_v2`/`codec_transfer` encoders; expected values are hand-written literals
or produced by a test-local `Model` (`BTreeMap<u64, Option<HealthFact>>` plus
turn and sequence, with its own applier). `InterestView` (T027b) is used only to
generate input pages in R18, never as the oracle; if `interest` has not landed,
R18 uses literal frames instead. The production replica is never its own
oracle. Every rejection compares the replica with a pre-call `clone`.

- **R1 (1, 16). Fresh and ordered.** A new world is empty, `epoch` is the
  constructor's, `turn`/`active`/`round`/`last_event_seq` are `None`. One page
  with three enters gives rows ascending `NetId`, each `health: None`.
  *Control:* a `Health` with `health: 0` stores `Some(0, ..)` while an untouched
  row still answers `None`, so unknown and zero are distinguishable.
- **R2 (7). Enter, health, leave, despawn.** Literal pages: health replaces all
  three fields; a leave removes row and health; a `Despawned` removes
  identically; `entity`/`contains`/`len` follow each step. *Control per fact:*
  each op alone produces exactly its one change.
- **R3 (7, 8). Notices.** `Spawned`, `Died`, `ActionResolved`, `TurnStarted` and
  `EncounterEnded` over live ids change nothing, and `Died` leaves `dead`
  unchanged. *Controls:* each of `Died`, `Spawned`, `ActionResolved` (actor and
  target) and `TurnStarted` with an unknown id errors; `Health` with
  `dead: true` does set it.
- **R4 (10). Turn.** No fact; `Some(active)`, round 3; `active: None`, round 4;
  a delta in between; `EncounterEnded`; snapshot to `None`. Each replaces
  exactly one fact. *Control:* an unchanged turn is not fabricated by any other
  op.
- **R5 (4). Acceptance.** Frames 1..n in order, including an empty-`ops` frame
  and a notices-only frame, are accepted and advance `last_event_seq`. *Control:*
  a fresh replica rejects a first frame at 2 with `EventSeqGap`.
- **R6 (4). Gap.** 1 then 3 is `EventSeqGap`, unchanged. *Control:* 1, 2, 3
  applies.
- **R7 (4). Duplicate.** 1, 2, 2 is `DuplicateEventSeq`. *Control:* 1, 2, 3.
- **R8 (4). Rewind.** 1, 2, 3 then 2 is `EventSeqRewind`; then 4 applies.
  *Control:* 1, 2, 3, 4.
- **R9 (5, N9). Fresh-state page.** 1..k live with A and B; a page at seq 1 with
  new health for A and B and no C: C is gone with no op, A and B keep their ids,
  health is the page's, and a later new id applies. A 300-entity reset split
  after every op still applies, because the enter section stays fresh.
  *Control:* the same page with its `EntityEnter` ops removed, applied at `k+1`,
  updates A and B and leaves C present (clearing happens only at 1). A rejected
  fresh page (unknown id) leaves the pre-call rows intact.
- **R10 (12, N3). Reuse.** Seq 1 enters ids 1 and 2; seq 2 leaves 2; a later
  enter of 2 is `ReusedEntityId`. Across a snapshot: a body keeps 1 and drops 2;
  a later enter of 2 is `ReusedEntityId`. *Controls:* after a fresh page ends its
  enter section (a `Health` follows), a re-enter of a narrowed-away id is
  `ReusedEntityId`, while the page's own retained enters were accepted; a
  brand-new id above `max_seen` enters; a snapshot may list a low retained id.
- **R11 (13). Bounds.** 1,024 rows apply; the 1,025th enter is
  `TooManyEntities` and changes nothing; a 1,025-row body is
  `TooManyEntities`; a 1,024-row body applies. A 257-op frame is `TooManyOps`;
  a 256-op frame applies. *Control:* at the limit, leave-then-enter succeeds.
- **R12 (14). Snapshot replacement.** A mid-epoch body with shuffled rows:
  ascending storage, ids kept, health from the body only, dropped ids gone with
  no op, turn replaced, a duplicate row is `DuplicateEntity`, and a
  `turn.active` outside the body is `UnknownEntity`. *Control:* the same set in
  different input orders gives equal replicas.
- **R13 (15). Snapshot cursor.** `(0, body)` on a fresh replica; after frames
  1..3, cursor 3 and 4 are accepted, cursor 2 and `SEQ_LAST + 1` are
  `InvalidSnapshotCursor`, and the next delta must be `cursor + 1`. *Control:*
  the next frame at `cursor + 1` applies and at `cursor + 2` gaps.
- **R14 (2). Shape.** Lane 2 is `WrongLane`; `event_seq` 0 and `SEQ_LAST + 1`
  are `EventSeqOutOfRange`; the same frame with lane 0 and seq 1 applies.
  *Control:* every accepted shape in R5 is lane 0 with `1..=SEQ_LAST`.
- **R15 (3). Atomicity on every error.** Each of the eleven variants is
  provoked on a non-empty replica (a crafted frame, body or cursor), the replica
  equals its clone, and a valid frame then applies. Includes a panic probe:
  zero-op, 256-op, all-notice and random-byte-built frames never panic.
  *Control:* the same valid frame on the same pre-state succeeds.
- **R16 (17). Determinism.** A 100-frame script applied to two replicas gives
  equal worlds (rows, health, turn, sequence); re-running it on a fresh replica
  gives the same result. *Control:* swapping two ops inside one frame changes the
  outcome, so the equality is not vacuous.
- **R17 (6, N8). Split pages.** A fresh page of enters + healths + turn, split
  after every op into 1-op frames numbered 1.., gives the same final replica as
  the single frame, and after the enter frame the rows are live with
  `health: None`. *Control:* the unsplit page.
- **R18 (N1–N6, 6). Scripted convergence.** 500 steps from
  `DeterministicRng::from_seed(46).stream("replica")` over a T027b-generated
  schedule: enters, healths, removals, notices, turn changes, a mid-stream
  snapshot, a fresh page. `Model` matches after every apply. *Control:* dropping
  one frame, duplicating one, or moving one op across its enter makes the run
  fail loudly.
- **R19 (Wire impact, D12). Real bytes.** A literal v2 page encodes with
  `codec_v2::encode_delta`, decodes, and applies to the same replica as the
  struct does; a snapshot body round-trips through `encode_body`/`decode_body`
  and `apply_snapshot`. *Control:* `--test codec`, `--test conformance` and
  `--test events_v2` pass unmodified.

## Definition of done

Run, on both native targets, from `crates/crpg-net/AGENTS.md:22-33`:

```
cargo test -p crpg-net --test replica --locked
cargo test -p crpg-net --locked
cargo clippy -p crpg-net --all-targets -- -D warnings
cargo fmt --all
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
cargo deny check
git diff --check
```

and `git diff --exit-code <base>` must show changes only in
`crates/crpg-net/src/replica.rs`, `crates/crpg-net/src/lib.rs`,
`crates/crpg-net/tests/replica.rs`, `docs/architecture/crpg-net.md`,
`crates/crpg-net/AGENTS.md`, the new ADR and this task's completion record; any
other path, `Cargo.toml` and `Cargo.lock` included, is a failure.

Docs, each signed: an ADR (D1–D13); a `docs/architecture/crpg-net.md` public
surface entry and a "Client replica (T046)" section (storage, sequence law,
fresh-page rule, reuse counter, snapshot cursor); and `crates/crpg-net/AGENTS.md`
traps: never buffer, never skip a frame, never add a restore/serde path, never
add a retired-id table, never decode bytes or take an epoch from anywhere but
`new`, keep the conformance `Replica` an oracle. Completion record with the case
count.

## Out of scope

Movement, positions, transforms and lane 1; interpolation and prediction (T049,
T026); within-area AOI and multi-area viewers (spec Tier 2); event and receipt
retention, the combat log and `ended()` (T047/T051); option lists (T047a); intent
encoding, seq counters and epoch choice; transfer assembly, chunking, deadlines
and the cutover gate (T024); resume and rebind (T025a); QUIC and `LocalLink`;
grant/perception policy (T027c); v1 sessions; save, load and any persisted
replica; names, kinds, sides and control; any wire, golden, sim, core, server or
manifest change.

## What must not change

Every existing `crpg-net` public item, signature, tag, constant and `Display`
string, and every byte of the existing test suites and fixtures
(`tests/codec.rs`, `conformance.rs`, `events_v2.rs`, `snapshot_transfer*.rs`,
`reconnect_wire.rs`). The conformance `Replica` stays a test double and must not
be replaced by `ReplicaWorld`. No dependency, no `unsafe`, no float, no
`HashMap`/`HashSet`, no `Serialize`/`Deserialize` on replica types. A change
needed outside `crpg-net` and its two docs is a stop, not a fix.

## Routing and size budget

`crpg-net`, one crate, one session after contract approval. Not an S-set change,
not an H-filter (it decides no disclosure; it consumes what T027b/T027c already
decided) and not H-unsafe. An ADR is still required by the crate's own rule, and
the contract should get an independent test writer (an O1-style case author
writing R1–R15 and R17–R19 from this contract before reading the
implementation); the keys are clone-comparison on every error and a `Model`
oracle, never the replica. Sizes: `src/replica.rs` about
260–340 lines with docs; `tests/replica.rs` about 650–850; `src/lib.rs` +2; the
two crate docs about +30 each; ADR about 60 lines.

**Split proposal.** One session is enough. If the estimator insists on two, both
stay in `crpg-net` and land in this order: **T046a** storage, sequence law, delta
application and queries (R1–R11, R14–R17), without which nothing else is usable;
**T046b** snapshot application, cursor continuity and the fresh-page rule (R9,
R12, R13, R18). Both share D8's commit path, so a split duplicates either the
staging helper (ship it in a) or R15 (it needs every variant, so it lands in b).
That is why one session is recommended.

## Open questions for the user

Each states what changes if overruled.

1. **Is `event_seq == 1` allowed to clear the replica (D6)?** *Options:* (a) yes,
   as written — the reset is only observable this way; (b) no — require a
   caller-invoked fresh-state entry, which makes the strict sequence law
   absolute but leaves a remote lane-0 client no reset signal and needs one more
   public mutator. **Recommend (a)**, with the D6 residual recorded; the change
   is one call site and one test if overruled.
2. **Does the replica validate event ops, or leave them to the bridge (D7)?**
   *Options:* (a) validate, as written (N1 enforced once); (b) ignore them, and
   T047 checks ids before rendering. **Recommend (a).**
3. **Payloads on `ReplicaError` (D11)?** *Options:* (a) carry
   `expected`/`got`/`entity`/`cursor`, as written, with `Debug` for diagnosis;
   (b) payload-free like `CodecError`, for a uniform `Display` shape.
   **Recommend (a).**
4. **Snapshot body order (D9):** accept any order and sort, or reject
   non-ascending input. **Recommend accept**, because the codec already enforces
   ascending on the wire and a set-shaped contract makes the determinism case
   meaningful.
5. **A narrowing to an empty view sends no frame**: an empty `full_state_ops`
   yields no deltas (`crates/crpg-server/src/host.rs:1551-1553`), so no client
   can clear from the wire. **Recommend T027c/T022 own a reset page or a
   transfer; T046 adds no mutator.**

## Agent log

- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T046 draft · Wrote the blind exact contract for the headless client replica: sorted-`NetId` storage, strict per-frame sequence law with the T022 fresh-state reset, a derived reuse counter, atomic two-pass application, whole-body snapshot cutover by cursor, a closed fatal error enum and the engine-neutral query surface T047 needs, with no wire, golden or dependency change.
