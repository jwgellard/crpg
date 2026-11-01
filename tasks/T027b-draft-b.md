# T027b-draft-b — blind T027b exact contract draft (`crpg-net` interest projection)

## Task / Crate(s) / Status

Sole crate: `crpg-net`, Phase 4 (tasks/T027b.md:5). Status: **proposed draft**.
D14 selected whole-area membership with explicit private-field grants
(tasks/POST-T018-DECISIONS.md:324); T027a's approved facts or sufficiency proof,
T021/T024 and this exact projection API remain (tasks/T027b.md:5). This draft
is input to adjudication; nothing here is approved, and no code is written.

## Decision

- **D1. New `interest` module in `crpg-net`, no new crate, no new dependency.**
  The module lives at `crates/crpg-net/src/interest.rs`, registered as
  `pub mod interest;` in `crates/crpg-net/src/lib.rs` (crates/crpg-net/src/lib.rs:30)
  between `codec_v2` and `projection_v2` (alphabetical). It names only
  `crpg_core::{EntityId, Ulid}` (crates/crpg-core/src/entity.rs:60,
  crates/crpg-core/src/ulid.rs:17) plus existing net vocabulary, covered by the
  crate's `crpg-core` edge. Rejected: a new crate or a `crpg-sim`/`crpg-server`
  edge, because standing rule E004 allows one implementation crate per child
  (tasks/T027b.md:36 incorporates it) and net must not execute gameplay.
- **D2. One `PeerView` equals one viewer in one area; no within-area AOI.**
  `InterestSet` carries exactly one area plus a grant generation. Phase 4
  whole-area interest does not authorize within-area AOI indexing
  (tasks/T027b.md:30). Rejected: a multi-area set or distance/cell index.
- **D3. Presence from same-area authorization; health/turn/action-result by
  explicit grant.** This mirrors D14 exactly: presence is visible to an
  authorized viewer in the same area, health/turn/action-result fields require
  separate explicit grants, unknown fields default to absent
  (tasks/POST-T018-DECISIONS.md:331). Rejected: granting health with presence,
  because the H-filter demo needs positions without hit points.
- **D4. Host-fed fact rows; net derives nothing.** `InterestFact` rows are the
  area-membership analogue of `EventCandidate`
  (crates/crpg-net/src/projection_v2.rs:53): every `Some` is an independently
  authorized disclosure for this viewer/generation, and absence is omission.
  Rejected: deriving area from `NetId` or health, per D14's no-derivation rule
  (tasks/POST-T018-DECISIONS.md:337).
- **D5. Mapping lifecycle with epoch-scoped graves; exhaustion forces resync
  or a new epoch, never id reuse.** `NetId` zero is never valid and ids are
  never reissued in one epoch (crates/crpg-net/src/protocol.rs:212). Retired
  (area, entity) pairs move to a bounded grave set; re-entry allocates a fresh
  `NetId`. Rejected: reissuing the retired id (it aliases two visible lifetimes
  in client logs) and unbounded graves (breaks tasks/T027b.md:28).
- **D6. Emit only frozen v1 `DeltaOp`s and the T024 `SnapshotBody`; no new
  wire fact, no version bump.** Ops are the eight filtered-replica variants
  (crates/crpg-net/src/protocol.rs:309); snapshots reuse `SnapshotBody` /
  `SnapshotEntity` (crates/crpg-net/src/protocol_transfer.rs:278,
  crates/crpg-net/src/protocol_transfer.rs:287). New facts or fields require
  their own reviewed protocol version before implementation (tasks/T027b.md:16).
  Rejected: a v3 tag or snapshot-field addition here, because v1 bytes must
  stay intact and the demo needs no new payload.
- **D7. One internal `InterestError`; public wire denials stay
  `NotAuthorized`.** Internal variants separate host-config bugs
  (`AreaMismatch`, `StaleContext`) from capacity states (`TooManyFacts`,
  `MappingFull`, `IdExhausted`); anything naming an entity to an
  unauthorized viewer is the existing public code for missing, hidden, and
  foreign ids alike (crates/crpg-net/src/protocol.rs:426). Display renders
  `<Variant> at interest/projection`, mirroring `ProjectionError`
  (crates/crpg-net/src/projection_v2.rs:117). Rejected: distinct wire codes
  per cause, because T027c denial uniformity forbids leaking the cause.
- **D8. Net fills per-client `event_seq` from the view cursor, starting at 1;
  a suppressed-only tick emits zero frames and consumes no sequence.** This
  matches the frozen `DeltaFrame` shape (crates/crpg-net/src/protocol.rs:292)
  and the standing rule that a suppressed-only page emits no frame. Rejected:
  host-assigned seq for v1 interest deltas, because the cursor must advance
  atomically with the mapping it numbers.
- **D9. Revocations land before admissions inside one call; explicit
  `revoke`/`reseed` cover the host's pre-removal capture and atomic cutover.**
  Retire-then-enter in a single `project_delta` call implements the
  revoke-before-next-request order; `reseed` mirrors grant narrowing, which
  clears the delivery log and restarts `event_seq` at 1
  (crates/crpg-server/src/host.rs:819). Rejected: lazy revocation on next
  admission, because a revoked mapping must read as gone before any subsequent
  request executes.
- **D10. `FieldMask.action_result` gates v2 `ActionResolved` candidates, not
  v1 ops.** No v1 op carries action results (crates/crpg-net/src/protocol.rs:309);
  the bit is consumed when the host filters `EventCandidate::ActionResolved`
  rows through `project_events` (crates/crpg-net/src/projection_v2.rs:141).
  Rejected: dropping the bit (D14 names the field, tasks/POST-T018-DECISIONS.md:331)
  and inventing a v1 carrier for it (D6).

## Interface

New module `crates/crpg-net/src/interest.rs` (D1). Field order below is
declaration order.

```rust
use crpg_core::{EntityId, Ulid}; // crates/crpg-core/src/entity.rs:60, crates/crpg-core/src/ulid.rs:17
use crate::protocol::{DeltaFrame, NetId, SessionEpoch, LANE_COMBAT, MAX_DELTA_OPS, MAX_VISIBLE_ENTITIES};
use crate::protocol_transfer::SnapshotBody; // crates/crpg-net/src/protocol_transfer.rs:278

/// Whole-area membership claim for one viewer (D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterestSet {
    pub area: Ulid,
    pub generation: u64,
}

/// Per-field grants; presence needs no bit (D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldMask {
    pub health: bool,
    pub turn: bool,
    pub action_result: bool,
}

impl FieldMask {
    pub const fn none() -> Self;
    pub const fn all() -> Self;
}

/// Coherent viewer context for one projection call page (D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewerContext {
    pub epoch: SessionEpoch, // crates/crpg-net/src/protocol.rs:207
    pub interests: InterestSet,
    pub mask: FieldMask,
}

/// Permitted health/dead replica state (v1 Health payload shape).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthFact {
    pub health: u32,
    pub max_health: u32,
    pub dead: bool,
}

/// Host-asserted transition for one entity in this tick, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleNotice {
    Spawned,
    Despawned,
    Died,
}

/// One host-authorized fact row: `None` is omission, never a zero fill (D4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterestFact {
    pub area: Ulid,
    pub entity: EntityId,
    pub notice: Option<LifecycleNotice>,
    pub disclose_notice: bool,
    pub health: Option<HealthFact>,
}

/// Encounter-level turn grant resolved through the caller's map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisclosedTurn {
    pub active: Option<(Ulid, EntityId)>,
    pub round: u64,
}

/// Projection failure: input bound, coherence, or capacity (D7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterestError {
    /// `facts.len() > MAX_VISIBLE_ENTITIES` (crates/crpg-net/src/protocol.rs:92).
    TooManyFacts,
    /// A row's area differs from the view's area (single-area viewers, D2).
    AreaMismatch,
    /// A call would map more than `MAX_VISIBLE_ENTITIES` live entities.
    MappingFull,
    /// No `NetId` may be issued: allocator or grave set exhausted; the host
    /// must resync or open a new epoch (D5).
    IdExhausted,
    /// A row or turn was authorized under an older grant generation.
    StaleContext,
}

impl core::fmt::Display for InterestError {
    /// Renders `<Variant> at interest/projection`, without payloads.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result;
}
impl std::error::Error for InterestError {}

/// Per-peer interest cursor: mapping, graves, and both cursors.
pub struct PeerView {
    pub context: ViewerContext,
    // Private: live map (area, entity) -> NetId (<= MAX_VISIBLE_ENTITIES);
    // graves of retired triples (same cap); last-projected HealthFact per
    // live entry; next raw id from 1 (0 never valid,
    // crates/crpg-net/src/protocol.rs:221); next event_seq from 1.
}

impl PeerView {
    pub fn new(context: ViewerContext) -> Self;
    /// Grant narrowing: drops live mappings to graves, clears last-projected
    /// state, restarts `event_seq` at 1.
    pub fn reseed(&mut self, context: ViewerContext);
    /// Retires one mapping to graves now; false when unmapped. `entity_of`
    /// returns `None` for it immediately (D9).
    pub fn revoke(&mut self, area: Ulid, entity: EntityId) -> bool;
    /// Retires every live mapping; grave overflow keeps existing graves and
    /// reports full.
    pub fn revoke_all(&mut self);
    pub fn context(&self) -> &ViewerContext;
    pub fn generation(&self) -> u64;
    pub fn map_of(&self, area: Ulid, entity: EntityId) -> Option<NetId>;
    pub fn entity_of(&self, net: NetId) -> Option<(Ulid, EntityId)>;
    pub fn is_retired(&self, area: Ulid, entity: EntityId) -> bool;
    pub fn mapped_len(&self) -> usize;
    pub fn retired_len(&self) -> usize;
    pub fn next_event_seq(&self) -> u64;
}

/// Projects one tick of authorized facts to v1 delta frames (D6, D8), chunked
/// at `MAX_DELTA_OPS` (crates/crpg-net/src/protocol.rs:70) with `lane =
/// LANE_COMBAT` (crates/crpg-net/src/protocol.rs:34), the passed
/// `server_tick`, and successive `event_seq` from the view cursor.
pub fn project_delta(
    view: &mut PeerView,
    facts: &[InterestFact],
    turn: Option<DisclosedTurn>,
    server_tick: u64,
) -> Result<Vec<DeltaFrame>, InterestError>;

/// Projects the same authorized facts to a snapshot body for the transfer
/// channel (`0x81`, lane 2; crates/crpg-net/src/protocol_transfer.rs:19,
/// crates/crpg-net/src/protocol_transfer.rs:22), using the caller's map and
/// allocating ids for newly disclosed entities under the same bounds.
pub fn project_snapshot(
    view: &mut PeerView,
    facts: &[InterestFact],
    base_tick: u64,
) -> Result<SnapshotBody, InterestError>;
```

Caps applied: `MAX_VISIBLE_ENTITIES` (crates/crpg-net/src/protocol.rs:92) bounds
each `facts` slice, the live map, and the graves; `MAX_DELTA_OPS`
(crates/crpg-net/src/protocol.rs:70) chunks frames; the frame/collection/string
caps ride the existing `encode_delta` path
(crates/crpg-net/src/codec.rs:391); raw ids run `1..=u64::MAX - 1`, mirroring
`SEQ_FIRST..=SEQ_LAST` (crates/crpg-net/src/protocol.rs:41,
crates/crpg-net/src/protocol.rs:48). Host tightening-only `POLICY_*` defaults
(e.g. crates/crpg-net/src/protocol.rs:130, crates/crpg-net/src/protocol.rs:152)
are unchanged and unwired.

## Behaviour

1. **Enter plus permitted initial state before any reference.** An unmapped,
   ungraved row allocates the next raw id and emits `EntityEnter`
   (crates/crpg-net/src/protocol.rs:311) first; disclosed `Spawned` adds a
   `Spawned` op (crates/crpg-net/src/protocol.rs:321); granted `health: Some`
   adds one `Health` op (crates/crpg-net/src/protocol.rs:336). `map_of`
   resolves the entity only after the call returns `Ok`.
2. **Leave on revoke; retire for the epoch; mapping gone before next request.**
   A live-mapped entity absent from `facts` emits `EntityLeave`
   (crates/crpg-net/src/protocol.rs:316), moves to graves with its `NetId`,
   and `entity_of`/`map_of` return `None` for it from that call onward.
   `revoke`/`revoke_all`/`reseed` apply the same transition without emitting.
   Re-entry allocates a fresh id; the retired id is never reissued.
3. **Hidden spawns consume no visible id and no sequence.** A row with
   `disclose_notice == false`, or an entity undisclosed to this viewer
   (absent, or present with `notice: None` and `health: None`), allocates
   nothing, emits nothing, and advances neither cursor.
4. **Despawn notice only when disclosable, else cause-neutral leave.** Disclosed
   `Despawned` emits `Despawned` (crates/crpg-net/src/protocol.rs:325) then
   retires; undisclosed emits `EntityLeave` and retires identically.
5. **Death stays distinct from despawn; the terminal entity is retained.**
   Disclosed `Died` emits `Died` (crates/crpg-net/src/protocol.rs:331); the
   mapping stays live and no `Despawned` follows. Undisclosed death emits no
   `Died`; granted health still projects `dead: true`.
6. **Omit, never zero-fill; never leak through metadata.** `health: None` or
   `mask.health == false` emits no `Health` op and no placeholders; unmapped
   `DisclosedTurn.active` renders as `Turn { active: None, .. }`
   (crates/crpg-net/src/protocol.rs:347). Counts, `event_seq`, graves, and
   cached state expose nothing hidden: any two calls differing only in
   undisclosed payloads give byte-identical output.
7. **Per-field updates against the last-projected image.** Granted `health:
   Some` emits `Health` only on change from the stored image; `turn: Some`
   with `mask.turn` emits one `Turn` per page. `mask.action_result` is
   ignored by v1 projection and gates v2 candidates instead (D10).
8. **Snapshot uses the coherent context at the approved cutover.**
   `project_snapshot` rejects rows authorized under another generation with
   `StaleContext`, builds `SnapshotBody` from the same map/cursors as deltas,
   and allocates ids for newly disclosed rows under B9 bounds. Turn state is
   delta-only and never appears in the snapshot. Result: `SnapshotBody` whose
   ids match the delta stream; error: `StaleContext`, `TooManyFacts`,
   `AreaMismatch`, `MappingFull`, `IdExhausted`.
9. **Bounded state; exhaustion means resync or a new epoch.** `facts` longer
   than `MAX_VISIBLE_ENTITIES` fails `TooManyFacts`; mapping beyond the live
   cap fails `MappingFull`; grave overflow or raw-id exhaustion fails
   `IdExhausted`. On `IdExhausted` the host must resnapshot/rebind or open a
   new `SessionEpoch` (crates/crpg-net/src/protocol.rs:207) with a fresh
   `PeerView`; no id is ever reused within the epoch. Result: errors leave the
   view fully unmutated (B11).
10. **Tightening-only policy, B5 maxima, no AOI.** B5 maxima ride the existing
    codec path and no limit is loosened; operators may only tighten `POLICY_*`.
    One view carries one area; foreign rows fail `AreaMismatch`.
11. **Atomic pages.** `project_delta` and `project_snapshot` run all
    bound/coherence/capacity pre-checks (`TooManyFacts`, `AreaMismatch`,
    `StaleContext` for snapshots, `MappingFull`, `IdExhausted` including grave
    room for every pending retirement) before emitting or mutating; any error
    returns with the view, map, graves, and both cursors byte-identical to
    entry, mirroring the no-partial-output rule
    (crates/crpg-net/src/projection_v2.rs:141). Result: `Ok` pages apply
    fully; `Err` applies nothing.
12. **Gapless per-client sequencing and chunking.** The first frame of a view's
    life carries `event_seq == 1`; each emitted frame advances the cursor by
    exactly one; ops chunk at `MAX_DELTA_OPS` in input-row order (same-tick
    order is input order). A fully suppressed tick returns `Ok(vec![])` and
    consumes no `event_seq`. Projection never emits `Receipt`
    (crates/crpg-net/src/protocol.rs:354): command outcomes stay on the ingest
    path (crates/crpg-server/src/host.rs:904). Result: contiguity checkable by
    the existing `Replica::contiguous` oracle shape
    (crates/crpg-net/tests/conformance.rs:338).

## Wire impact

No new fact and no protocol version: v1 bytes are intact, v2 tags
(crates/crpg-net/src/protocol_v2.rs:37) and the transfer first byte
(crates/crpg-net/src/protocol_transfer.rs:19) are untouched, and decode
precedence (`CodecError`, crates/crpg-net/src/codec.rs:52) is unchanged.
Projection output reuses the frozen v1 ops plus the T024 snapshot body, so
existing `codec` round-trips (crates/crpg-net/src/codec.rs:154,
crates/crpg-net/src/codec.rs:391) and the explicit-version gate
(crates/crpg-net/tests/events_v2.rs:170) keep passing unmodified.

## Tests

New suite `crates/crpg-net/tests/interest.rs` (locked), each case naming its
Behaviour items and carrying a positive disclosure control for every
suppression, plus drop/corrupt negative controls. Oracles project permitted
facts independently of delivery bytes and compare per-fact with events as
multisets, following the `ExpectedView` / `Replica::matches` shape
(crates/crpg-net/tests/support/driver.rs:176,
crates/crpg-net/tests/conformance.rs:354).

- **T1 (B1, B10, two viewers/areas).** Two views over two areas: rows in area
  A enter with `EntityEnter` + initial `Health`, absent from B's page; a
  second-area row in a single-area view fails `AreaMismatch` without mutation.
  Positive control: granting B the area adds the enter page (tasks/T027b.md:41).
- **T2 (B1, B7 field-by-field).** Masks toggled per bit: granted bits
  project, revoked omit byte-identically to never-granted; unmapped active
  renders `active: None`. Positive control per suppression: same granted tick.
- **T3 (B2 leave/reenter).** Revoking authorization leaves with `EntityLeave`,
  retires the id, `map_of` returns `None`; re-granting allocates a fresh id
  the retired id never reappears as.
- **T4 (B3 hidden spawn).** `disclose_notice == false` emits nothing and pins
  both cursors; the positive control (`true`) emits
  `EntityEnter` + `Spawned` + initial `Health` and advances both.
- **T5 (B4 despawn both grants).** Disclosed despawn emits `Despawned` and
  retires; undisclosed emits only `EntityLeave` and retires identically; both
  read back via `is_retired`.
- **T6 (B5 death vs despawn).** Disclosed death emits `Died`, retains the live
  mapping, later health still projects, no `Despawned` appears; undisclosed
  death emits no `Died` while granted health shows `dead: true`
  (cf. crates/crpg-net/tests/conformance.rs:1152).
- **T7 (B6 omission).** Redacted vs granted pages for the same entity: equal
  after granting; redacted bytes, counts, and `event_seq` reveal nothing.
  Drop/corrupt negative control on the oracle
  (cf. crates/crpg-net/tests/conformance.rs:1095).
- **T8 (B2, B11 same-tick revoke-and-request).** One call revokes X while
  admitting Y: X leaves and retires before Y's enter is numbered, so Y never
  inherits X's id and no reference precedes Y's `EntityEnter`.
- **T9 (B8, B12 snapshot/delta cutover).** Deltas then `project_snapshot`:
  snapshot/delta id-identity, `base_tick` coherence, gapless `event_seq`
  across the boundary; `StaleContext` on older-generation rows. Contiguity
  via crates/crpg-net/tests/conformance.rs:338, byte-identical redelivery via
  crates/crpg-net/tests/events_v2.rs:2005.
- **T10 (B9 bounded state).** `TooManyFacts`, `MappingFull`, and forced
  `IdExhausted` each leave map lengths, graves, and both cursors pinned;
  recovery is a fresh epoch and `PeerView` with no id reused.
- **T11 (B12 sequencing/chunking).** A 300-update tick chunks into two frames
  with successive `event_seq` from 1 in input order; a fully suppressed tick
  returns empty with the cursor pinned; a 5,000-tick loss/jitter run converges
  per peer (cf. crates/crpg-net/tests/conformance.rs:1366,
  crates/crpg-net/tests/conformance.rs:1039).
- **T12 (v1 bytes intact, no new Behaviour).** The existing `codec`,
  `conformance`, and `events_v2` suites pass unmodified, including explicit
  version selection (crates/crpg-net/tests/events_v2.rs:170), ordered
  real-history projection (crates/crpg-net/tests/events_v2.rs:1739), and
  field-by-field suppression (crates/crpg-net/tests/events_v2.rs:1055).

## Inputs from T027c and consumers

The host (T027c, sole crate `crpg-server`) supplies, per bound peer, one
`ViewerContext` and, once per pump (crates/crpg-server/src/host.rs:954)
post-state, one `facts` slice plus `Option<DisclosedTurn>`:

- `epoch`: the peer's session generation, matching the epoch the host bound
  at `bind_peer` (crates/crpg-server/src/host.rs:691).
- `interests.area`: the viewer-area binding from the host's authoritative
  viewer-to-area table; `generation`: the host's grant generation, bumped on
  every narrowing `set_grants` (crates/crpg-server/src/host.rs:819) before the
  matching `reseed`. Rows carry the generation they were authorized under;
  older rows fail `StaleContext` instead of projecting stale grants.
- `mask`: derived from the peer's `DisclosureGrants`
  (crates/crpg-server/src/host.rs:167) / `EntityDisclosure`
  (crates/crpg-server/src/host.rs:140); presence needs no bit (D3). Control
  authority stays in `ControlGrant` (crates/crpg-server/src/host.rs:133);
  net never checks who may act.
- `facts` rows: iterated from the bound `World` via `ids`
  (crates/crpg-sim/src/world.rs:1022) filtered by `area_of`
  (crates/crpg-sim/src/world.rs:896) against the bound area
  (crates/crpg-sim/src/world.rs:878, crates/crpg-sim/src/world.rs:886), in
  ascending `(area, entity)` order; `notice` values from sim-observed
  transitions including cross-area moves (crates/crpg-sim/src/area.rs:158,
  crates/crpg-sim/src/area.rs:193); `health` from the per-peer captured state
  (`CapturedView`, crates/crpg-server/src/capture.rs:105). Pre-removal capture
  plus atomic revocation stay host-ordered (tasks/T027b.md:74); `revoke` /
  `revoke_all` are the mechanism the host calls inside that order.
- New-epoch admission, `MAX_PEERS` binding (crates/crpg-server/src/host.rs:72),
  and delivery through `take_delivery` (crates/crpg-server/src/host.rs:1682)
  are unchanged; projected frames enter the host's existing per-peer byte log.
- What T046 (replica) needs from this API, without designing T046: stable
  per-viewer mapping with enter-before-reference and epoch-scoped graves;
  `EntityEnter`/`EntityLeave`/`Spawned`/`Despawned`/`Died`/`Health`/`Turn`
  pages with gapless `event_seq` from 1; snapshots sharing the same ids;
  omission guarantees strong enough to persist bytes verbatim; and
  `StaleContext`/`IdExhausted` as the only resync triggers. Storage,
  application, and read queries remain T046's design.

## Out of scope

Authoritative perception generation (tasks/POST-T018-DECISIONS.md:327),
authentication, within-area AOI, unrestricted save snapshots, the legality
engine (`validate_action` stays the only admission path,
crates/crpg-sim/src/combat.rs:2949; enumeration stays read-only and bounded,
crates/crpg-sim/src/combat.rs:3177), client interpolation, QUIC carriage,
reconnect policy beyond `SessionFence` (crates/crpg-net/src/resume.rs:74),
T027c host wiring, and T046 replica storage (see above).

## Size budget

`src/interest.rs` about 220–300 lines with docs; `tests/interest.rs` about
500–700 lines (T1–T12 plus oracle); no other file touched. One session.

## Open questions (each with a recommendation)

1. **Unmapped `Turn.active`: `active: None` or omit the op?** Recommend
   `active: None`: absence is already encodable
   (crates/crpg-net/src/protocol.rs:347), so the round survives and the hidden
   actor leaks nothing.
2. **Grave cap shared with the live map, or separate?** Recommend one shared
   `MAX_VISIBLE_ENTITIES` (crates/crpg-net/src/protocol.rs:92) cap per set,
   overflow reporting `IdExhausted`: overflow already means resync.
3. **Multi-area viewers?** Recommend one `PeerView` per area, composed
   host-side; `AreaMismatch` stays a hard error and no cross-area indexing
   enters net.
4. **Snapshots carrying turn?** Recommend no (delta-only): the snapshot stays
   a pure entity image and turn arrives on the first post-cutover delta.
   Revisit if the demo shows a turnless first paint.
5. **A v1 `action_result` accumulation query?** Recommend no: the host already
   filters v2 candidates per viewer (crates/crpg-net/src/projection_v2.rs:141).
   Revisit only with a consumer that cannot use v2.

## Agent log

- 2026-11-01 (UTC) · opencode-go/muse-spark-1.3-contributor + T027b draft · Wrote the blind exact contract for per-viewer interest projection so adjudication can compare it without reading the rival draft.
