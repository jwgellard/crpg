# T046 — client replica: storage, delta application, read queries (`crpg-net`): blind contract draft (b)

Status: **proposed, not approved.** Sole crate: `crpg-net`. Prerequisites on
this branch: T027b exact contract (`tasks/T027b.md:568`: "What T046 may rely on",
rules N1–N9), ADR-0033 T046 obligations
(`docs/adr/0033-godot-bridge.md:525`), T021 v2 wire, T024 snapshot body.
Nothing here needs a host, sim, core, protocol-version, golden, or fixture
change to compile or test.

## Decision

| # | Question | Options | Recommendation + consequence |
|---|---|---|---|
| 1 | Replica shape | (a) One `ReplicaWorld` struct owning all state. (b) Split store + applier + query view. | **(a).** Spec names one type (`docs/CRPG_ENGINE_SPEC.md` via `tasks/T027b-facts.md:49`: "behind a `ReplicaWorld` type with no mutating methods except `apply_delta`") and E012 agrees (`tasks/T027b-facts.md:42,50`). One type = one owner, no cross-type invariant. |
| 2 | Delta application strictness | (a) Gap/duplicate/reorder = error, never skip. (b) Skip-and-continue. | **(a).** Required by ADR-0033 §6 step 7 + §11 code 53 (`docs/adr/0033-godot-bridge.md:346,352`): "A bad frame is never skipped: the replica would silently diverge." T046 contract restates it; bridge maps any failure to fatal `REPLICA_REJECTED`. |
| 3 | Snapshot semantics | (a) Whole replacement incl. turn. (b) Merge into live state. | **(a).** Required by N5 (`tasks/T027b.md:582`). Missing id = gone with no op; turn replaced (absent turn clears). |
| 4 | N1–N9 enforcement | (a) Each rule = named error variant. (b) One generic `InvalidFrame`. | **(a).** Closed enum with one variant per violated rule (Behaviour B4 table) so the bridge log names the cause and tests pin each rule. |
| 5 | Atomicity | (a) Failed apply leaves replica unchanged. (b) Partial mutation allowed. | **(a).** Validate-then-commit: check the whole frame/body against current state, mutate only on success. Every error test asserts byte-equality with a pre-call clone. |
| 6 | Read queries | (a) Engine-neutral value queries only. (b) Godot-flavoured objects. | **(a).** `crpg-net` owns "filtered replica storage, delta application and engine-neutral read queries" (`tasks/T027b-facts.md:42,50`); `crpg-godot` owns engine-facing objects. No `godot` dependency, no presentation types. |
| 7 | V1 vs V2 | (a) v2 only. (b) Accept v1 and v2. | **(a) v2 only.** The embedded authority binds V2 (`docs/adr/0033-godot-bridge.md:90`: "`V2` is required: the combat log needs the event ops"). v1 vocabulary still arrives through `Legacy` (`crates/crpg-net/src/protocol_v2.rs:88`), so no state shape is lost. A v1-versioned frame is a version error, not a silent reinterpret. |
| 8 | Test oracle | (a) Hand-written literals + independent mirror. (b) `InterestView` as oracle. | **(a).** T027b forbids it (`tasks/T027b.md:571`): "`InterestView` to generate test input, never as its oracle." Mirror is test-local and counts its own ids. |
| 9 | Sessions | (a) One session, `crpg-net`. (b) Split storage/queries across tasks. | **(a).** Standing rule E004 one implementation crate per child (`tasks/T027b-facts.md:39`); the surface below is ~250 lines + tests in one session. |

## Interface (`crates/crpg-net/src/replica.rs`)

```rust
use std::collections::BTreeMap;

use crate::protocol::{NetId, SessionEpoch, MAX_VISIBLE_ENTITIES};
use crate::protocol_transfer::{HealthFact, SnapshotBody, TurnFact};
use crate::protocol_v2::{DeltaFrame, DeltaOp};

/// Engine-neutral client replica. Not `Serialize`: restoring a saved view
/// would restore stale authorization (`tasks/T027c.md:23`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaWorld {
    epoch: SessionEpoch,       // fixed at `new`
    next_seq: u64,             // next expected `event_seq`, starts at 1
    server_tick: u64,          // last applied frame's tick, 0 when empty
    entities: BTreeMap<NetId, EntitySlot>, // ascending-NetId order
    turn: Option<TurnFact>,    // last disclosed turn, else None
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EntitySlot {
    health: Option<HealthFact>, // None = present, undisclosed (N4/N8)
    dead: bool,                 // sticky terminal flag from Health/Died
}

/// Closed failure set. Display: `"<Variant> at replica/apply"`, no payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicaError {
    WrongEpoch,       // frame epoch != `epoch` (N3)
    SequenceGap,      // `event_seq` != `next_seq` (gap, dup, or reorder)
    UnknownEntity,    // op names an id never entered or retired (N1/N2)
    DuplicateEnter,   // `EntityEnter` for a live id
    TooManyEntities,  // live count would exceed `MAX_VISIBLE_ENTITIES`
    DuplicateId,      // snapshot body repeats one `NetId`
    TurnDangling,     // snapshot `turn.active` names no body id
    NotSnapshotPeer,  // snapshot when constructed for delta-only use (none here; reserved, never constructed)
}

impl ReplicaWorld {
    pub fn new(epoch: SessionEpoch) -> Self;
    pub fn epoch(&self) -> SessionEpoch;
    /// Next expected per-client `event_seq` (1 when empty).
    pub fn next_event_seq(&self) -> u64;
    pub fn server_tick(&self) -> u64;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn contains(&self, net: NetId) -> bool;
    pub fn apply_delta(&mut self, frame: &DeltaFrame) -> Result<AppliedDelta, ReplicaError>;
    pub fn apply_snapshot(&mut self, body: &SnapshotBody, base_seq: u64, base_tick: u64)
        -> Result<(), ReplicaError>;
    // --- read queries (T047 surface; bridge decodes frames itself) ---
    /// Visible ids, ascending. Dead-but-retained ids are included.
    pub fn net_ids(&self) -> Vec<NetId>;
    /// `None` = unknown id; `Some(None)` = known, health undisclosed; `Some(Some(f))` = disclosed.
    pub fn health(&self, net: NetId) -> Option<Option<HealthFact>>;
    pub fn is_dead(&self, net: NetId) -> Option<bool>;
    pub fn turn(&self) -> Option<TurnFact>;
}

/// What `apply_delta` changed, for view rebuilds. No entity data: query it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedDelta {
    pub event_seq: u64,
    pub server_tick: u64,
    pub entered: Vec<NetId>,   // ascending
    pub removed: Vec<NetId>,   // ascending
    pub turn_changed: bool,
}
```

`src/lib.rs` gains `pub mod replica;` after `protocol_v2`. Every public item
documented (`#![warn(missing_docs)]`, `crates/crpg-net/src/lib.rs:2`). No other
existing item changes. No new dependency (reuse `protocol`, `protocol_v2`,
`protocol_transfer`; `BTreeMap` per the `HashMap` ban).

## Behaviour

- **B1. Shape and bounds.** `new(epoch)` is empty: `next_seq == 1`,
  `server_tick == 0`, no entities, `turn == None`. Live count never exceeds
  `MAX_VISIBLE_ENTITIES` (`crates/crpg-net/src/protocol.rs:92`: 1024). Storage
  is one `BTreeMap` entry per live id plus one `TurnFact`, two counters: no
  graves, no history, no receipts. Ordering is ascending `NetId` everywhere
  (`NetId` is `Ord`, `crates/crpg-net/src/protocol.rs:216`; iteration
  `World::ids` precedent `crates/crpg-sim/src/world.rs:1022`).
- **B2. Delta admission (order).** `apply_delta` checks, before any mutation:
  (a) version/lane already enforced by `codec_v2` (byte 2,
  `crates/crpg-net/src/protocol_v2.rs:34`; lane 0,
  `crates/crpg-net/src/protocol.rs:34`); (b) frame epoch bytes equal
  `self.epoch` else `WrongEpoch`; (c) `frame.event_seq == next_seq` else
  `SequenceGap` — a gap, a duplicate and a reorder are the same error, never
  a skip (ADR-0033 §11 code 53, `docs/adr/0033-godot-bridge.md:352`); (d) every
  op validates against current state (B3) else its variant. `Err` changes
  nothing (B6). `Ok` commits ops in frame order, then
  `next_seq += 1` (saturating is wrong: `SEQ_LAST` is `u64::MAX - 1`,
  `crates/crpg-net/src/protocol.rs:48`; exhausting `u64` is unreachable and
  untestable, so plain `+ 1`), `server_tick = frame.server_tick`, and returns
  `AppliedDelta` with ascending `entered`/`removed` and `turn_changed`.
- **B3. Op application (frame order, split pages allowed).** Ops apply in the
  order received; a page may split an entity's block at any op boundary (N8),
  so `Health` before any `Health` for an entered id in the same frame is
  legal. Per op:
  | Op | Rule |
  |---|---|
  | `Legacy(EntityEnter{e})` | live `e` → `DuplicateEnter`; count+1 > 1024 → `TooManyEntities`; else insert with `health: None`. |
  | `Legacy(EntityLeave{e})` / `Legacy(Despawned{e})` | not live → `UnknownEntity`; else remove. Cause-neutral: identical effect (N2). |
  | `Legacy(Spawned{e})` / `Legacy(Died{e})` | notices, not state (N8): not live → `UnknownEntity`; else `Spawned` changes nothing, `Died` sets `dead = true` and keeps the mapping (N4: "stays after death", `tasks/T027b.md:581`). |
  | `Legacy(Health{entity, ..})` | not live → `UnknownEntity`; else replace the whole fact (`health = Some(fact)`), `dead = fact.dead` (N4). |
  | `Legacy(Turn{active, round})` | `active == Some(id)` with id not live **in the post-op map of this frame** → `UnknownEntity` (enters earlier in the same frame count); else `turn = Some` copy. `active: None` stored as-is (N7). |
  | `Legacy(Receipt{..})` | ignored by state (bridge takes receipts itself, `docs/adr/0033-godot-bridge.md:529`); never an error. |
  | `ActionResolved/TurnStarted/EncounterEnded` | ignored by state (bridge collects events itself, same line); actor/target ids are **not** validated (a hidden actor is nameable in history while absent from state). |
  | v1 `DeltaFrame` passed as such | not accepted: the function takes only `protocol_v2::DeltaFrame` (`crates/crpg-net/src/protocol_v2.rs:68`); there is no conversion path. |
- **B4. N1–N9 map (each rule → enforcing behaviour → wire-break error).**
  | Rule | Enforced by | Wire break → error |
  |---|---|---|
  | N1 no op before enter/presence (`tasks/T027b.md:573`) | B3 liveness check | `UnknownEntity` |
  | N2 one removal op, no later naming (`tasks/T027b.md:576`) | B3 remove + liveness check | `UnknownEntity` on any later naming |
  | N3 no id reuse; new epoch = empty (`tasks/T027b.md:578`) | `WrongEpoch` unless epoch matches; never reissued ids within epoch | `WrongEpoch` |
  | N4 absent = unknown, replace-whole, death retains (`tasks/T027b.md:581`) | `health: None` default; whole-fact replace; `Died` keeps mapping | n/a (value rule; `UnknownEntity` if id unknown) |
  | N5 snapshot replaces whole + turn (`tasks/T027b.md:583`) | B5 | `DuplicateId` / `TurnDangling` |
  | N6 delta never clears; queries return last disclosed (`tasks/T027b.md:585`) | no op clears `health` or `turn`; `EncounterEnded` ignored by state | n/a |
  | N7 `None` vs hidden indistinguishable (`tasks/T027b.md:587`) | store `None` as-is, no side channel | n/a |
  | N8 split pages; notices not state (`tasks/T027b.md:589`) | B3 order-insensitivity within a frame | n/a (framing rule) |
  | N9 no `Spawned`/`Died` in V1; lane-0 narrowing reset empties first (`tasks/T027b.md:592`) | v2-only input; reset path = fresh `ReplicaWorld::new` on new epoch (host clears log and restarts `event_seq` at 1, `crates/crpg-server/src/host.rs:819`) | `WrongEpoch` for pre-reset frames |
- **B5. Snapshot application.** `apply_snapshot(body, base_seq, base_tick)`
  replaces everything: checks `body.entities.len() <= MAX_VISIBLE_ENTITIES`
  else `TooManyEntities`; duplicate `entity` in `body.entities` (order
  irrelevant) else `DuplicateId`; `body.turn.active == Some(id)` with id not
  among `body.entities` else `TurnDangling`. Commits: entities become exactly
  the body in ascending `NetId`, each `health` copied verbatim (`None` stays
  undisclosed), `dead` copied from the fact (`false` when `None`); `turn`
  becomes `body.turn` verbatim including `None`; `server_tick = base_tick`,
  `next_seq = base_seq + 1`. `resend`/cutover semantics are host-side
  (`tasks/T027b.md:379`); the replica only replaces. Every accepted body
  passes `codec_transfer::encode_body` bounds: ≤1024 entities
  (`crates/crpg-net/src/protocol.rs:92`), `MAX_ENCODED_BODY_BYTES`
  (`crates/crpg-net/src/protocol_transfer.rs:50`).
- **B6. Atomicity.** Both `apply_*` are validate-then-commit: `Err` leaves the
  replica bit-identical (same `entities`, `turn`, `next_seq`, `server_tick`).
  Neither function panics on any input. `ReplicaWorld: Clone` is the complete
  rollback witness in tests.
- **B7. Errors fatal at the bridge.** Every `ReplicaError` maps to fatal code
  53 `REPLICA_REJECTED` (`docs/adr/0033-godot-bridge.md:353`): "the core calls
  `Host::shutdown`, drops the host, and every later `poll` returns the same
  code until `close`." Rationale: any single skipped or invented fact silently
  diverges the client. The replica itself stays usable after `Err` (tests
  continue); fencing is bridge policy, not replica state.
- **B8. Read queries (what T047 gets).** `net_ids` ascending `NetId`;
  `health` triple-state (unknown id / known-undisclosed / disclosed fact —
  absent is never zero, per `docs/adr/0033-godot-bridge.md:274`);
  `is_dead` (`None` on unknown id); `turn` (last disclosed, `None` when never
  set or cleared by snapshot); `epoch`, `next_event_seq`, `server_tick`,
  `len`/`is_empty`/`contains`. The replica does **not**: decode bytes (bridge
  uses `codec_v2::decode_delta`,
  `docs/adr/0033-godot-bridge.md:529`); collect event ops or receipts
  (bridge takes them "itself"); validate gameplay, ownership, grants or
  ability text; expose `EntityId`, names, sides, placements, positions or
  options (wire carries none: `crates/crpg-net/src/protocol.rs:305` note "no
  transform op", ADR-0033 context item 4).
- **B9. V1 vs V2 sessions.** v2 only. A v1-versioned byte is refused at the
  codec (`UnsupportedVersion` either direction,
  `tasks/T027b-facts.md:110`); the replica function cannot even receive one
  (distinct `DeltaFrame` types, `crates/crpg-net/src/protocol.rs:292` vs
  `crates/crpg-net/src/protocol_v2.rs:68`). N9's "V1 sessions receive no
  `Spawned` or `Died`" (`tasks/T027b.md:592`) is therefore vacuous for the
  replica: it never sees a V1 session. The T022 narrowing-reset path
  (`tasks/T027c.md`-era host behaviour, `crates/crpg-server/src/host.rs:819`)
  restarts `event_seq` at 1 on retained ids, so the bridge must construct a
  fresh `ReplicaWorld` (same or new epoch per host) and re-enter — it must
  not feed post-reset frames to the pre-reset replica (`WrongEpoch` or
  `SequenceGap` fires instead of silent divergence).
- **B10. No derived authority.** Reads decoded frames only. No `World`, RNG,
  event journal, transport, clock, thread, I/O, or new dependency.

## Wire impact

**None.** No new protocol version, tag, field, fact, code, bound or fixture.
Input is existing decoded v2 `DeltaFrame` values (tags 0–10,
`crates/crpg-net/src/protocol.rs:177`,
`crates/crpg-net/src/protocol_v2.rs:37`) and existing `SnapshotBody` values
(`crates/crpg-net/src/protocol_transfer.rs:278`). v1 bytes, fixtures, decode
precedence and B5 maxima untouched. `Despawned` (tag 3) already valid v1 op.

## Tests (new `crates/crpg-net/tests/replica.rs`)

No new dependency. Inputs are hand-built frames plus `InterestView`
(`tasks/T027b.md:298`) used **only as a frame generator** (drive
`project_delta`/`project_snapshot`, feed the produced ops through
`codec::encode_delta`-equivalent assembly into v2 frames); the oracle is a
test-local `Mirror` (its own `BTreeMap` + counters) plus hand-written
literals, never production projection (`tasks/POST-T018-RULES.md:54`). Every
error case asserts the replica equals its pre-call `clone()` (atomicity). The
conformance `Replica` oracle (`crates/crpg-net/tests/conformance.rs:285`)
stays untouched.

| # | Case (covers) | Valid control |
|---|---|---|
| R1 | Enter + initial health + turn; listing ascending (B1,B3,B8) | Hand literal: shuffled enters still list ascending; control reorders frames and lists identically. |
| R2 | Gap / duplicate / reorder are `SequenceGap`, state unchanged (B2,B6) | Valid next frame applies; control: in-order redelivery applies exactly once. |
| R3 | Unknown id on `Health`/`Leave`/`Died` → `UnknownEntity`, unchanged (B3,B4-N1) | Enter then the same op succeeds; control. |
| R4 | Double enter → `DuplicateEnter`; leave twice → `UnknownEntity` (B3) | Leave once then re-enter takes effect; control. |
| R5 | Removal retires: later `Health` for it → `UnknownEntity` (B4-N2) | Fresh enter of another id still works; control. |
| R6 | Wrong epoch → `WrongEpoch`, unchanged; fresh replica on new epoch starts at seq 1 (B2,B4-N3,B9) | Same frame against matching replica applies; control. |
| R7 | Snapshot replaces whole incl. turn; missing id gone with no op; `DuplicateId`/`TurnDangling`/`TooManyEntities` unchanged (B5,B4-N5) | Literal body equals queries after apply; control: next delta emits only differences. |
| R8 | Absent health is `Some(None)`, never zero; `Health` replaces whole; `Died` keeps mapping with `dead=true` (B4-N4) | Grant-then-value sequence shows the value; control. |
| R9 | `Turn{active: None}` stored and returned; `Turn` naming a removed id → `UnknownEntity`; `Turn` naming a same-frame enter succeeds (B3,B4-N7) | Active enter + turn in one frame; control. |
| R10 | Split page: enter in frame k, `Health` in frame k+1 and `Health`-before-second-`Health` orders converge (B3,B4-N8) | Single-frame combined page converges equally; control. |
| R11 | Event ops and `Receipt` ignored by state but consume the seq (B3) | State queries equal with/without them; `next_event_seq` advanced; control. |
| R12 | `InterestView`-generated 2-viewer/2-area schedule (~500 steps): feed ops as v2 frames, `Mirror` matches after every step; hidden rows never appear (B4-N1, generator-only) | Same schedule with one visible grant flipped changes state; control. |
| R13 | Determinism: same frame sequence twice (fresh replicas) → equal replicas and equal query vectors (item 8) | Byte-compare `net_ids`/`health`/`turn`; control mutates one op and diverges. |
| R14 | Bounds: 1024 enters pass, 1025th → `TooManyEntities` unchanged; `MAX_DELTA_OPS` chunking passes `codec_v2::encode_delta` (B1) | 1024 all-health snapshot body passes `encode_body`; control. |

## Definition of done

Run `cargo test -p crpg-net --test replica --locked`,
`cargo clippy -p crpg-net --all-targets --locked -- -D warnings`, then the
common native gates from `crates/crpg-net/AGENTS.md` on both native targets.
`git diff --exit-code <base> -- crates/crpg-server crates/crpg-sim
crates/crpg-core` must exit 0. Docs: `docs/architecture/crpg-net.md`
"Client replica (T046)" section (owner table B3, B4 map, query list) and
`crates/crpg-net/AGENTS.md` traps (never skip a gap, never restore a saved
replica, never validate event actor ids, never add a `godot` edge), each
signed. No ADR needed (no wire/host change); file under existing ADR-0033.

## Out of scope

Host capture/projection (T027c), event-op collection and receipts (bridge,
`docs/adr/0033-godot-bridge.md:529`), option lists and `legal_intents`
(T047a), presentation names/sides/placements (decision 9 table), movement and
positions (no transform op, `crates/crpg-net/src/protocol.rs:305`), interpola
tion/prediction (T049/T026), multi-area viewers, resume/reconnect policy
(T025a), save/restore of the replica, any wire, golden, sim or core change.

## What must not change

Existing `crpg-net` public items, signatures, tags, codecs, fixtures and
goldens. No dependency, `unsafe`, float, `HashMap`/`HashSet` or manifest edit.
A needed change outside `crpg-net` and its two docs is a stop, not a fix.

## Routing and size budget

`crpg-net`, H-filter (per-client filtering change). One O1 test writer writes
R1–R14 from this contract before reading the implementation. `src/replica.rs`
about 250–330 lines with docs; `tests/replica.rs` about 600–800;
`src/lib.rs` +1; docs about +30 each. One session.

## Open questions for the user

1. **Should `NotSnapshotPeer`-style reservation stay?** The draft keeps the
   variant as never-constructed to reserve the discriminant. *Recommendation:
   drop it before implementation* — dead variants invite matching bugs; the
   closed set is complete without it.
2. **Should `server_tick` regression (older tick) be an error?** Area clocks
   need not match (`tasks/T027a.md`, "Source/destination clocks need not
   match"), and T027b D10 rejected a tick-regression error for the same
   reason. *Recommendation: no error* — store the latest frame's tick as-is.
3. **Does the demo need names/sides in the replica?** No: the bridge builds the
   presentation table from the host at `open`
   (`docs/adr/0033-godot-bridge.md:240`) and T027b decision 2 reserves any wire
   form to a later versioned task. *Recommendation: confirm out of scope.*

## Agent log

- 2026-11-02 (UTC) · opencode-go/muse-spark-1.3-contributor + T046 draft · Drafted the blind v2-only replica contract from T027b N1–N9, ADR-0033 §6/§11 obligations and the frozen v1/v2/transfer wire shapes, with no wire, dependency, golden, sim or core change.
