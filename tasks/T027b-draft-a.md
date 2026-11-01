# T027b-draft-a — N-INTEREST projection machinery (`crpg-net`)

## Task / Crate(s) / Status

Sole crate: `crpg-net`, Phase 4. Status: **proposed draft** (blind, attempt 1); nothing here
is approved. D14 selects whole-area interest (`tasks/POST-T018-DECISIONS.md:326`) and
`tasks/T027b.md:11` is the interface gate this draft fills.

What exists: frozen lane-0 v1 wire and codec (`crates/crpg-net/src/protocol.rs:25`,
`crates/crpg-net/src/codec.rs:52`), the v2 host-fed `EventCandidate`/`project_events`
(`crates/crpg-net/src/projection_v2.rs:53`, `:141`), and the `0x81` snapshot wire with
`SnapshotBody` (`crates/crpg-net/src/protocol_transfer.rs:278`). There is **no** production
filter: the only projection is a test driver whose tables are fixtures, not perception
(`crates/crpg-net/AGENTS.md:86`), and no host call site exists
(`crates/crpg-net/tests/support/driver.rs:723`;
`crates/crpg-server/src/host.rs:2200` plans its own map). T027b adds the
machinery additively; no v1 byte moves.

## Decision

- **D1. New Rust-only module `interest`.** One public module
  (`crates/crpg-net/src/interest.rs`); no codec change; it emits only existing
  `protocol::DeltaOp` and `protocol_transfer` values. Rejected: a protocol version or new tags —
  there is no new wire fact (`tasks/T027b.md:15`) and a bump would change bytes for nothing.
  Surface growth still needs the architecture-doc update and the ADR required by
  `docs/architecture/crpg-net.md:39-40` (open question 1).
- **D2. `InterestSet` is exactly one area ULID (Tier 1).** Whole-area membership is selected
  (`tasks/POST-T018-DECISIONS.md:326`, `docs/CRPG_ENGINE_SPEC.md:815`) and the type keeps Tier 2
  a one-file change (`docs/CRPG_ENGINE_SPEC.md:816`). Rejected: a net `visible_to(world, viewer,
  entity) -> ComponentMask` oracle (`docs/CRPG_ENGINE_SPEC.md:817`): T027b forbids a net
  ownership/LOS oracle (`tasks/T027b.md:13`), net may not read `World`
  (`docs/architecture/crpg-net.md:311-314`), and the spec itself makes the mask host-held.
- **D3. The host supplies each entity's area; net only compares it.** `EntityFact::area` is
  `Some(area)` when presence is authorized, `None` otherwise; a fact is visible iff `area ==
  Some(interest.area())`. Rejected: a precomputed visible-id list (the D14 rule would live
  nowhere in net and the two-viewer test would be vacuous); deriving area from `NetId` or health
  (`tasks/POST-T018-DECISIONS.md:337`); per-entity area sets (Tier 2, forbidden by
  `tasks/T027b.md:30`).
- **D4. `FieldMask` is closed: `HEALTH`, `TURN`.** It mirrors `EntityDisclosure::health`
  (`crates/crpg-server/src/host.rs:145`) and `DisclosureGrants::turn_state` (`:178`); `NONE` is
  deny-by-default (`tasks/POST-T018-DECISIONS.md:332`). Rejected: an `ACTION_RESULT` bit —
  action results are already per-field on `EventCandidate`
  (`crates/crpg-net/src/projection_v2.rs:77-87`), and a second gate duplicates authorization and
  turns a hidden bad outcome into an error instead of suppression
  (`docs/architecture/crpg-net.md:217-224`).
- **D5. `CauseMask` is closed: `SPAWN`, `DEATH`, `DESPAWN`.** It mirrors `EntityDisclosure::spawn`/`died`/`despawn`
  (`crates/crpg-server/src/host.rs:147-151`) and allows a same-tick spawn-and-die notice.
  Rejected: one `Cause` enum — it makes that real combination lossy or an error, while the host
  models the three independently.
- **D6. Facts key on the whole `EntityId`.** Ids are ordered
  (`crates/crpg-core/src/entity.rs:58-63`), an index is reused (`:88-97`), and two areas may
  mint equal ids (`tasks/T027a.md:141`). Rejected: arena-index keys; a composite `(area,
  EntityId)` key (one viewer has one area, and net must not derive the area).
- **D7. `InterestView` owns the mapping and mints `NetId`s.** Ids start at 1, are minted in
  ascending visible-`EntityId` order, and never repeat in the view's epoch. Rejected: the host
  keeping the map (already duplicated at `crates/crpg-server/src/host.rs:2200-2226`; T027b asks
  net for "mapping lifecycle, per-peer view/cursor", `tasks/T027b.md:11`; two maps drift).
- **D8. Retirement is derived, not stored.** A `NetId` is retired iff it is below `next_net` and
  not live — the host's own monotone proof (`crates/crpg-server/src/host.rs:2214-2218`).
  Rejected: a grave table (unbounded, or needs an invented cap, and proves nothing new).
- **D9. Two projection functions, one context.** `project_state` → delta page + cursor;
  `project_snapshot` → body + cursor; both take `ViewerContext`. Rejected: a mode flag
  (different outputs and bounds); a paged/iterator API (breaks per-tick coherence); per-field
  `observe_change` calls (order and atomicity would become host behaviour).
- **D10. Membership notices belong to `project_state`.** It emits `EntityEnter`, `EntityLeave`,
  `Spawned`, `Despawned` and state ops; `project_events` keeps resolved-action, turn-start and
  encounter-end history (`crates/crpg-net/src/projection_v2.rs:141`). Rejected: both paths
  emitting a despawn (a disclosable despawn would yield `EntityLeave` + `Despawned`,
  contradicting `crates/crpg-net/tests/support/driver.rs:778-781`); always `EntityLeave` (loses
  the approved notice, `tasks/T027b.md:23`).
- **D11. Turn disclosure has four gates.** `TURN` granted, a fact supplied, at least one live
  mapping, and (when `active` is `Some`) a mapped active entity; otherwise the whole `Turn` op
  is omitted. Rejected: a per-entity turn grant (turn is encounter-global,
  `crates/crpg-server/src/host.rs:178`); disclosing the round with no visible entity (suppressed
  at `crates/crpg-net/tests/support/driver.rs:941-947`).
- **D12. `ViewCursor { tick, version }` is view state.** `version` advances by one per committed
  page or snapshot; `tick` is that commit's context tick. Rejected: the host's delivery
  `event_seq` (`crates/crpg-server/src/host.rs:470`,
  `crates/crpg-net/src/protocol_transfer.rs:96-98`) — numbering, suppression and chunking are
  host-owned (`docs/architecture/crpg-net.md:221-223`); a tick-only cursor cannot order two
  projections in one tick.
- **D13. Projection is atomic.** Epoch, input length, inspected duplicates and id capacity are
  checked before mutation; any `Err` leaves ops, mapping, remembered facts and cursor unchanged.
  Rejected: mutate-then-rollback — rolling back minted ids would reuse ids.
- **D14. No limits type, no knobs.** The only cap is `MAX_VISIBLE_ENTITIES`
  (`crates/crpg-net/src/protocol.rs:92`). Rejected: a host-supplied `ViewLimits`: B5 is
  tightening-only (`tasks/T027b.md:29`) and a parameter is a loosening path.
- **D15. `InterestError` is local.** Rejected: `CodecError` (wire decode,
  `crates/crpg-net/src/codec.rs:52`) and `ProjectionError` (history vocabulary,
  `crates/crpg-net/src/projection_v2.rs:107`); neither names "this host context cannot serve
  this view".

## Interface (`crates/crpg-net/src/interest.rs`)

```rust
use crpg_core::{EntityId, Ulid};

use crate::protocol::{DeltaOp, NetId, SessionEpoch, MAX_VISIBLE_ENTITIES};
use crate::protocol_transfer::{HealthFact, SnapshotBody, SnapshotEntity, TurnFact};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InterestSet { area: Ulid }
impl InterestSet {
    pub const fn new(area: Ulid) -> Self;
    pub const fn area(self) -> Ulid;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FieldMask(u8);                    // D4; NONE is deny-by-default
impl FieldMask {
    pub const NONE: Self;
    pub const HEALTH: Self;
    pub const TURN: Self;
    pub const ALL: Self;
    pub const fn bits(self) -> u8;
    pub const fn from_bits(bits: u8) -> Option<Self>;   // None outside ALL
    pub const fn contains(self, other: Self) -> bool;
    pub const fn union(self, other: Self) -> Self;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CauseMask(u8);                    // D5
impl CauseMask {
    pub const NONE: Self;
    pub const SPAWN: Self;
    pub const DEATH: Self;
    pub const DESPAWN: Self;
    pub const ALL: Self;
    pub const fn bits(self) -> u8;
    pub const fn from_bits(bits: u8) -> Option<Self>;
    pub const fn contains(self, other: Self) -> bool;
    pub const fn union(self, other: Self) -> Self;
}

/// One host fact for one viewer at one tick. Field order is contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityFact {
    pub entity: EntityId,            // whole id, generation included (D6)
    pub area: Option<Ulid>,          // Some iff presence is authorized (D3)
    pub health: Option<HealthFact>,  // value only; disclosure still needs HEALTH
    pub fields: FieldMask,           // per-entity field grants
    pub causes: CauseMask,           // lifecycle notices authorized this tick
}

/// One coherent viewer context for one tick. Field order is contract.
#[derive(Debug)]
pub struct ViewerContext<'a> {
    pub epoch: SessionEpoch,
    pub tick: u64,
    pub interest: InterestSet,
    pub entities: &'a [EntityFact],
    pub turn: Option<TurnFact>,
    pub grants: FieldMask,           // context grants; currently TURN only
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct ViewCursor { pub tick: u64, pub version: u64 }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateProjection { pub ops: Vec<DeltaOp>, pub cursor: ViewCursor }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotProjection {
    pub body: SnapshotBody,
    pub entity_count: u16,           // == body.entities.len() as u16
    pub cursor: ViewCursor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterestError {
    WrongEpoch,          // context epoch is not this view's epoch
    TooManyEntities,     // context.entities.len() > MAX_VISIBLE_ENTITIES
    DuplicateEntity,     // two inspected facts share one EntityId
    NetIdExhausted,      // the epoch's NetId space is spent (B12)
}
impl core::fmt::Display for InterestError { /* "<Variant> at projection/interest" */ }
impl std::error::Error for InterestError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterestView {
    epoch: SessionEpoch,
    next_net: u64,                     // 1 at new (D7)
    live: BTreeMap<EntityId, Live>,     // ascending EntityId
    index: BTreeMap<NetId, EntityId>,   // ascending NetId
    last_turn: Option<TurnFact>,
    cursor: ViewCursor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Live {
    net: NetId,
    last_health: Option<HealthFact>,    // last disclosed value; None = undisclosed
}

impl InterestView {
    pub fn new(epoch: SessionEpoch) -> Self;
    pub fn epoch(&self) -> SessionEpoch;
    pub fn cursor(&self) -> ViewCursor;
    pub fn next_net(&self) -> u64;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn net_id(&self, entity: EntityId) -> Option<NetId>;
    pub fn entity(&self, net: NetId) -> Option<EntityId>;
    pub fn is_retired(&self, net: NetId) -> bool;
    pub fn project_state(&mut self, context: &ViewerContext<'_>)
        -> Result<StateProjection, InterestError>;
    pub fn project_snapshot(&mut self, context: &ViewerContext<'_>)
        -> Result<SnapshotProjection, InterestError>;
}
```

`DeltaOp` is `crate::protocol::DeltaOp` (`crates/crpg-net/src/protocol.rs:309`);
`HealthFact`/`TurnFact`/`SnapshotBody`/`SnapshotEntity` are the T024 facts
(`crates/crpg-net/src/protocol_transfer.rs:296`, `:307`, `:278`, `:287`) reused unchanged.
`ops` has no per-page type cap; B11 gives the structural bound. Every public item is
documented (`docs/architecture/crpg-net.md:246`). Caps: `MAX_VISIBLE_ENTITIES` inputs and live
mappings, `5 * MAX_VISIBLE_ENTITIES` ops per page, `MAX_ENCODED_BODY_BYTES` per snapshot, and
`u64::MAX - 1` as the last mintable `NetId`.

## Behaviour

Precedence for both functions, before any mutation (D13):

1. `context.epoch != self.epoch` → `WrongEpoch`.
2. `context.entities.len() > MAX_VISIBLE_ENTITIES` → `TooManyEntities`
   (`crates/crpg-net/src/protocol.rs:92`).
3. Mark a fact **inspected** iff it is visible or its `EntityId` is live. Two
   inspected facts with one `EntityId` → `DuplicateEntity`. Uninspected facts
   are ignored whole: no id, health read, cause read or counter (B4).
4. If the visible count needs more ids than `u64::MAX - next_net` →
   `NetIdExhausted`.

Only then is the page built and committed.

1. **Enter before any reference.** For each visible fact not live, in ascending
   `EntityId`: `DeltaOp::EntityEnter { entity: net }` first
   (`crates/crpg-net/src/protocol.rs:177`), then `DeltaOp::Health` when granted
   and supplied (`:336`), then `DeltaOp::Spawned` when `SPAWN` is granted
   (`:181`), then `DeltaOp::Died` when `DEATH` is granted (`:185`). No op
   references a `NetId` that this view has not disclosed by an earlier
   `EntityEnter` or snapshot.
2. **Leave on revocation.** For each live entity not visible, in ascending
   `EntityId`, exactly one membership op is emitted and the entity is unmapped
   before the call returns: `DeltaOp::Despawned` (`:324`) when the inspected
   fact grants `DESPAWN`, otherwise `DeltaOp::EntityLeave` (`:315`). An entity
   simply absent from `context.entities` has no cause and leaves
   cause-neutrally.
3. **Retirement and actionable mapping.** Every unmapped `NetId` is retired for
   the epoch: it is never issued again, `entity(net)`/`net_id(entity)` report
   `None` afterwards, and `is_retired(net)` is `true`. An id at or above
   `next_net` was never issued by this view, so `is_retired` is `false`. The
   mapping a host resolves intents through is gone before a later admission in
   the same tick (`tasks/T027c.md:12`). `new` restarts at 1 only because a new
   view belongs to a new epoch (`crates/crpg-net/src/protocol.rs:212-214`).
4. **Hidden spawn consumes nothing.** A fact with `area: None` or another area
   for an entity not live produces no op, allocates no `NetId`, leaves
   `next_net`/`cursor` unchanged, and its `health`/`causes` are not read even if
   invalid for a visible fact (`tasks/POST-T018-DECISIONS.md:332`).
5. **Notice versus cause-neutral leave.** `Despawned` only for a live entity whose inspected fact
   grants `DESPAWN`; every other removal is `EntityLeave`. An invisible but live fact contributes
   only that bit; a removal cause on a still-visible entity is ignored (no op).
6. **Death is not despawn.** `HealthFact.dead` never removes or hides an
   entity: it stays live, mapped and un-left. `Died` is a cause notice, emitted
   only with `DEATH` for a live entity, and never implies despawn
   (`crates/crpg-net/src/protocol.rs:330`). A hidden death (no `HEALTH`, no
   `DEATH`) emits neither and keeps the entity mapped.
7. **Omit, never fill.** `Health` only when `fact.fields.contains(HEALTH)` and
   `fact.health.is_some()`; `Turn` only under D11. Absent fields are omitted,
   never zero- or false-filled. No op, `next_net`, `len` or cursor depends on a
   hidden value: `next_net` advances only for visible first appearances and
   `cursor.version` only for a page with at least one op.
8. **Per-field change and re-grant.** `Health` only when the granted supplied
   value differs from `last_health`; the remembered value becomes the disclosed
   one, and `None` when the grant is absent or no value is supplied, so a later
   grant emits current state, not a cached secret (`tasks/T027c.md:20`).
   `last_turn` follows the same rule. An already-live entity with `SPAWN` is not
   re-entered and emits no `Spawned`; `DEATH` may still emit once.
9. **Snapshot replacement.** `project_snapshot` applies the same precedence and
   visibility, keeps live ids, mints new ones in ascending `EntityId`, retires
   every non-visible live entity, and returns `SnapshotBody` with entities in
   strictly ascending `NetId` (the rule `encode_body` enforces,
   `crates/crpg-net/src/codec_transfer.rs:36-49`), health only when granted.
   Turn follows D11 and is absent when its active entity is not in the body
   (`crates/crpg-net/src/codec_transfer.rs:50-57`). Size is bounded by
   construction: `2 + 1024 * 22 + 22` equals `MAX_ENCODED_BODY_BYTES`
   (`crates/crpg-net/src/protocol_transfer.rs:50`), and the body is directly
   usable with `plan_transfer` (`crates/crpg-net/src/codec_transfer.rs:488`).
10. **Coherence and cutover.** One successful call applies exactly one context atomically
    (mapping and remembered facts). `cursor.version` advances by one only when the call emits a
    snapshot or at least one op, with `cursor.tick = context.tick`; an empty page leaves the
    cursor unchanged. Any `Err` emits nothing and changes nothing.
11. **Bounds.** `live.len() <= MAX_VISIBLE_ENTITIES`; metadata is one `Live` per live entity, one
    `last_turn`, one `next_net`, one cursor — no retired-id table (D8). A page is at most
    `5 * MAX_VISIBLE_ENTITIES` ops (1024 removals + Enter/Health/Spawned/Died per appearance) and
    a snapshot at most `MAX_ENCODED_BODY_BYTES` (item 9). Nothing is configurable (D14).
12. **Exhaustion.** Allocation is refused with `NetIdExhausted` when
    `next_net == u64::MAX`, before mutation. D14's remedy is a new
    `SessionEpoch` with a fresh `InterestView` (resync/new epoch); ids below
    `next_net` are never reissued in the epoch (`tasks/T027b.md:28`,
    `crates/crpg-server/src/host.rs:2214`).
13. **Order.** Removals in ascending `EntityId`; then one block per visible
    fact in ascending `EntityId` (`EntityEnter`, `Health`, `Spawned`, `Died`);
    then `Turn` last. Input order never matters.
14. **No derived authority.** Net compares supplied area ULIDs only; it never
    derives area, LOS, perception, AOI, legality or credentials, adds no
    dependency and reads no `World`, RNG or event queue (`tasks/T027b.md:13`,
    `:30`, `:36`; `docs/architecture/crpg-net.md:311-314`, `:246-249`).

## Wire impact

None. No new fact, field, tag, numeric code or version: the module emits existing v1 `DeltaOp`
values and the existing T024 snapshot body, and the v2 host wraps state ops in `DeltaOp::Legacy`,
which adds no tag (`docs/architecture/crpg-net.md:273-280`). v1 bytes, fixtures and precedence are
intact (`docs/architecture/crpg-net.md:137-139`), the nine B5 maxima are untouched
(`crates/crpg-net/src/protocol.rs:58-95`), and a future within-area AOI needs its own version
(`tasks/T027b.md:15-16`).

## Tests (new `crates/crpg-net/tests/interest.rs`)

The oracle is a test-local replica replay plus a hand-written per-viewer expectation, never
production projection (`tasks/POST-T018-RULES.md:54`); each suppression has a positive control.

- **I1 (B1, B2, B4, B14). Two viewers, two areas.** One list holds area-X and area-Y facts;
  viewer A sees only X, viewer B only Y; `next_net`, `len`, ops and cursor stay per view and the
  other area allocates nothing. Positive control: flip `area` and exactly one `EntityEnter`
  appears, advancing `next_net` by one.
- **I2 (B7, B8). Field-by-field entitlements.** Four viewers over the same facts: `HEALTH`,
  `TURN`, neither, both. Health appears only in health views, `Turn` only in turn views with a
  live entity, and neither leaks through `next_net`, `len` or `cursor.version`. Positive
  control: grant each field and assert the exact op at its fixed index.
- **I3 (B1, B3, B13). Enter/update/leave/reenter without id reuse.** A shuffled input yields
  ascending-`EntityId` blocks with `Enter` before `Health`; a change emits one `Health`; leaving
  retires the id, re-entry mints a **new** id, `entity(old)` is `None`, `is_retired(old)` is
  true, and `next_net` never goes back.
- **I4 (B4). Hidden spawn negative control.** A fact with `area: None` and one with another area
  carrying bogus health produce nothing and allocate nothing. Positive control: make it visible
  and assert one `EntityEnter` with its granted initial health.
- **I5 (B5, B6). Death, despawn, hidden death, hidden despawn.** A dead but present entity stays
  mapped and emits `Died` only with `DEATH`; it never leaves. A removal with `DESPAWN` emits
  `Despawned`, one without emits `EntityLeave` — exactly one membership op each. Each
  suppression's positive control grants the bit (or moves the entity out of the area) and
  asserts the other op appears, never both.
- **I6 (B2, B3; §7 order). Same-tick revoke-and-request mapping.** Project a context without a
  previously visible entity, then before any further call resolve its `NetId`: `entity()` is
  `None`, `is_retired()` is true, and a forged target fails. Positive control: a still-visible
  entity resolves.
- **I7 (B9, B10, B13). Snapshot and delta cutover.** The body has ascending `NetId`, health only
  where granted, a turn only with a visible active entity, and advances `version` by one at
  `tick`. A following delta page re-emits no unchanged field. A snapshot that drops an entity
  retires its id; a later page re-enters it with a new id. A wrong-epoch or duplicate snapshot
  leaves the view equal to a pre-call clone.
- **I8 (B11). Bounded state.** 1,024 visible facts pass; 1,025 → `TooManyEntities`, view
  unchanged. A maximal page (1,024 removals plus 1,024 appearances) is at most
  `5 * MAX_VISIBLE_ENTITIES` ops. A 1,024-entity all-health snapshot passes
  `codec_transfer::encode_body` and `plan_transfer`, proving the `MAX_ENCODED_BODY_BYTES` bound
  (`crates/crpg-net/tests/snapshot_transfer.rs:797-798`).
- **I9 (B12, unit test in `src/interest.rs`). Exhaustion.** No real stream reaches `u64::MAX`,
  so a `#[cfg(test)]` helper forces `next_net` (precedent: core's forced-generation unit test).
  One new visible fact → `NetIdExhausted`, view unchanged; existing live entities still update.
- **I10 (B1, B5, B10; corrupt/drop controls). Oracle fails loudly.** Replay the pages into a
  test replica and compare with the hand-written view. Negative controls: drop an `EntityEnter`,
  swap `Health` before `Enter`, duplicate a membership op, drop a frame between pages — each
  must fail. Positive control: untouched pages converge.
- **I11 (Wire impact). v1 bytes intact.** A fixed projected page encodes via
  `codec::encode_delta` (`crates/crpg-net/src/codec.rs:391`) to the same bytes as a hand-built
  frame; the same ops in `DeltaOp::Legacy` encoded by `codec_v2` differ only in byte 0.
  `--test codec --test conformance` unchanged.
- **I12 (B10, B13). Precedence and atomicity.** Each of `WrongEpoch`, `TooManyEntities`,
  `DuplicateEntity` is provoked and the view equals its pre-call clone; after each, a valid
  context projects the expected page.

## Inputs from T027c and consumers

| Input | Value and source | When |
|---|---|---|
| `epoch` | session epoch (`crates/crpg-net/src/protocol.rs:207`) | bind and every context |
| `InterestSet` | viewer's authorized area, validated against the campaign (`tasks/T027a.md:194-196`) | bind and viewer-area change |
| `entity` | full `EntityId` from T027a's area query (`tasks/T027a.md:135-139`) | every phase |
| `area` | `Some(world.area())` iff presence is authorized, else `None`; never derived by net (`tasks/POST-T018-DECISIONS.md:327`, `:337`) | every phase |
| `health` | authoritative health fact (`crates/crpg-net/src/protocol_transfer.rs:296`) | every phase |
| `fields` | `HEALTH` from `EntityDisclosure::health` (`crates/crpg-server/src/host.rs:145`) | grant change and every phase |
| `causes` | `SPAWN`/`DEATH`/`DESPAWN` from `EntityDisclosure::spawn`/`died`/`despawn` (`crates/crpg-server/src/host.rs:147-151`) | the lifecycle tick |
| `turn`, `grants` | turn fact (`crates/crpg-net/src/protocol_transfer.rs:307`) and `TURN` from `turn_state` (`crates/crpg-server/src/host.rs:178`) | every phase |
| `tick` | authoritative `server_tick` the context is pinned to | every phase |

Host duties: run the projection phase after the authoritative mutation commits and **before**
the next admission, so a revocation is visible to `entity()` first (`tasks/T027c.md:12`,
`:20`). Chunk a page into frames of at most
`MAX_DELTA_OPS` ops (`crates/crpg-net/src/protocol.rs:66-70`, `:289-290`)
without reordering or splitting an entity's block; assign frame `event_seq` and map `cursor`
to the delivery gate yourself (D12). A new epoch gets a new
`InterestView`; never reuse one across epochs or restore one from a save
(`tasks/T027c.md:23`). Cross-area moves and narrowing grants must cancel or supersede an
in-flight snapshot before publication (`tasks/POST-T018-DECISIONS.md:335-336`,
`docs/adr/0031-host-snapshot-sender-and-snapshot-sessions.md:11`, `:26`). Actor
control and the action grants stay host-side (`ControlGrant`,
`crates/crpg-server/src/host.rs:133`; `:153-156`); net's mapping is only the
target-side id resolution. T027c consumes this API rather than keeping
`plan_map` (`crates/crpg-server/src/host.rs:2200`) beside it.

**What T046 (replica) needs, without designing T046:** the delivered contract — page order and
chunking, `EntityEnter` before any state op, one membership op per removal, `Died` never implying
despawn, a snapshot body replacing the whole replica at its cursor, and no `NetId` ever aliasing a
later entity. T046 owns `ReplicaWorld`, `apply_delta`, read queries
(`tasks/E012-binary-crate-naming.md:72`, `docs/CRPG_ENGINE_SPEC.md:874-881`) and its own treatment
of an op for an unknown id (a protocol error, not a new net API).

## Out of scope

Any code or any file except this draft; T027c, T046, T027a and T021/T024 changes. Within-area
AOI, LOS, perception/stealth generation, GM bypass (`tasks/T027b.md:30`, `:63-66`).
Authentication, credentials, resume (T025a) and QUIC. Movement, prediction/interpolation, saved
views. Gameplay legality, `legal_actions` filtering, action-result history (T021), receipt/cache
policy. New dependencies, `unsafe`, floats, hash maps, any wire or golden change.

## Size budget

`src/interest.rs` ≈ 420–520 lines with docs and the unit test; `tests/interest.rs` ≈ 700–900;
`src/lib.rs` +1; the two crate docs ≈ +12 each. One implementation session after approval, plus
the ADR session if open question 1 requires it.

## Open questions (each with a recommendation)

1. **Does the public module need its own ADR first?** Interface changes require
   one (`docs/architecture/crpg-net.md:39-40`), as T021/T024/T025a each had.
   **Recommend yes: a short ADR recording D14 and this projection surface; it
   is outside this draft's files.**
2. **Who owns `Spawned`/`Despawned`/`Died`?** **Recommend D10**: the
   one-op-per-removal rule is already enforced at
   `crates/crpg-net/tests/support/driver.rs:778-781`, and a second owner
   double-emits.
3. **View cursor or host delivery ordinal?** **Recommend the view cursor**:
   the host owns frame numbering and `BeginTransfer.cursor`
   (`crates/crpg-net/src/protocol_transfer.rs:96-98`) is on that numbering; the
   two can be mapped one-to-one only when a page is never chunked.
4. **Is `project_snapshot` legal mid-epoch?** **Recommend yes**, idempotent in
   mapping and epoch-bound; narrowing resync needs it
   (`docs/adr/0031-host-snapshot-sender-and-snapshot-sessions.md:11`) and T024
   is reused unchanged.
5. **Is `TooManyEntities` on input length too strict?** **Recommend yes**: the
   host must scope facts; the cap is the B5 visible bound; otherwise a buggy
   host buys unbounded work before suppression.
6. **Should `is_retired` be public?** **Recommend yes**: T027c's same-tick
   revocation check and T046's late-op detection need it, and it is derived
   (D8), not stored.

## Agent log

- 2026-11-01 (UTC) · opencode-go/deepseek-v4.1-flash + T027b draft · Wrote the blind exact contract for per-viewer whole-area interest projection: host-supplied area/field/cause facts, a net-owned monotone `NetId` view, ordered enter/leave/death and snapshot projection, derived retirement, and exact errors, bounds and tests, with no wire change.
