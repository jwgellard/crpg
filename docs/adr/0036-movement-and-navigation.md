# ADR-0036: Movement and navigation in both time modes

Date: 2026-11-08 (UTC)
Status: **Proposed** — filed by T034 (architect). Nothing here is approved.
The six open questions at the end need the user's answers before the T035
and T036 contracts are written.

## Context

T034 (`docs/IMPLEMENTATION_PLAN.md` §3 M4 4C) is the specification D13 asks
for (`tasks/POST-T018-DECISIONS.md` D13): the path format, the movement rules
and the numeric limits, in both time modes, with `crpg-sim` consuming
navigation results it may not compute itself. It gates T035 (`crpg-nav` v1),
T036 (authoritative movement, S-set), T026p (lane-1 movement, S-set), T037
(host adapter) and the client tasks T049, T026 and T050. Every engine fact
below was checked against this branch.

What is true today:

- **Positions.** `crpg-sim` `Transform` is `position: [f32; 3]`,
  `velocity: [f32; 3]`, no rotation (`transform.rs:19-25`). Nothing in the
  sim writes one for a creature: `register_creature` "adds no transform"
  (`creature.rs:135`); only area transfer and the CLI's `spawn_at` do
  (`area.rs:175`, `crpg-cli/src/apply.rs:40`). `state_hash` panics on a
  non-finite float (`hash.rs:37-49`). Authored positions are already
  fixed-point: `crpg-data` `Transform.position: [Fx16_16; 3]` and
  `Area.bounds: Bounds` (`types.rs:235-256`, `document.rs:78-101`).
  `Fx16_16` is an `i32` with 16 fraction bits and no square root
  (`crpg-core/src/fixed.rs:36`).
- **Rules.** No float in a rules path (spec §2.4 rule 3, `AGENTS.md`);
  `f64` is banned in sim. Movement costs are pool spends with no kernel
  change (ADR-0029 K6). ADR-0029's Consequences ask T034 to "pin the
  quantization in sim" between `f32` positions and integer costs.
- **Time.** Encounters are TB or RT (ADR-0029 T1); T12 says non-participants
  and RT participants move every tick, TB participants only through their
  own turn's actions. The tick runs stage 2 (encounter clock) and stage 3
  (RT orders) (`tick.rs:54-59`); spec §10 puts movement at stage 4. In TB,
  `tick` knows nothing of combat (`crates/crpg-sim/AGENTS.md`, "never
  invents actions"). `CombatAction` is `Copy` (`combat.rs:911`).
- **Graph.** `crpg-sim` may import only core, data and rules; `crpg-nav`
  only core; `crpg-server`, `crpg-godot`, `crpg-cli` and `crpg-testkit` may
  import any crate except their listed exclusions (`tools/lint/deps.py`
  `ALLOWED`). `crpg-nav` is a four-line stub with no dependencies.
- **Navigation.** D27c: custom representation and bake, integer or
  `Fx16_16`, in the workspace crate; the `pathfinding` crate only with the
  user's approval. Spec §6.3: server and client must use the identical
  navmesh, and local avoidance is skipped at first.
- **Wire.** Lane 0 is the only lane; lanes 1+ are reserved for movement
  (`crpg-net/src/protocol.rs:27-34`); there is no transform op
  (`protocol.rs:300-305`). D13: movement gets its own versioned lane with
  processed-seq/server-tick acks; lane 0 stays window 1; prediction is
  outside the replica and sim, own movement only. ADR-0004's spike sent
  per-tick `MoveTo { seq, target }` datagrams with latest-seq-wins.

## Decision

Items are normative and each is written so a test can prove it. Prefixes:
A authority, U units and numbers, R route format, M sim state and
operations, W time modes, C turn-based cost, V navigation, L wire, H host,
G client, X range (only if Open question 4 adopts it).

### A. Authority and data flow

A1. **Clients send destinations, never paths or positions.** A movement
intent names an actor and a destination point. There is no client-supplied
path, speed or position (spec §7.3).

A2. **Nav computes, the host supplies, sim steps (D13).** `crpg-sim` never
imports `crpg-nav`. Sim owns its input type `MoveRoute` (R1); nav owns its
output type `NavPath` (V4); the host converts one to the other by copying
points. Neither crate names the other's types, and nothing is added to
`crpg-core`.

A3. **Sim revalidates everything it can check without geometry**: identity,
navmesh revision, shape, limits, start point, mode, turn and cost (R4, W4).
Walkability is navigation's guarantee, proven by T035's tests (V5), and the
host is the only caller that can supply a route.

A4. **Every movement change to a `World` is a sim operation** (M4, W4) or
tick stage 4 (W1). Pause, coalescing, rate limits and navigation queries
stay in the host (ADR-0029 T16, P1).

A5. **The arithmetic is shared, not copied.** `crpg-sim` exports the length,
step and cost functions (U4, U5, C4) as pure functions with no `World`
argument. The client's prediction and previews call them (`crpg-godot` may
import `crpg-sim`), so client and server compute the same numbers.

### U. Space, units and numbers

U1. **One world unit is one metre.** `y` is up; the ground plane is `x`/`z`
(Godot's convention; the client already places proxies at `y = 0`).

U2. **Authoritative movement coordinates are `[Fx16_16; 3]`.** Lengths,
costs and speeds are integers or `Fx16_16`. Conversion goes one way only:
`Fx16_16` to `f32` for the `Transform` mirror (M3) and presentation. No
`f32` value is converted back or read by movement or rules code. This is
the quantization ADR-0029 asks for: `f32` is never a source, so nothing is
quantized.

U3. **Coordinate limit.** Every component of every position and route
point satisfies `|raw| <= COORD_LIMIT_RAW` (2^29, 8,192 m). Differences
then fit in 2^30 and every product below fits in `i64`/`u64`.

U4. **Length is horizontal.**
`seg_len(a, b) = ceil_sqrt((b.x-a.x)^2 + (b.z-a.z)^2)` over raw values in
`u64`, where `ceil_sqrt(n) = s + (s*s < n) as u64` and `s = n.isqrt()`.
`y` does not count. A route's length is the sum of its segment lengths.

U5. **Interpolation.** For `0 <= d <= len`, `point_at(a, b, len, d)` is,
per component, `a + ((b - a) * d) / len` in `i64` with Rust's truncating
`/`; `d == len` returns `b` exactly. `y` is interpolated like `x` and `z`.

U6. **`f32` projection.** `to_f32(v) = (v.to_raw() as f32) / 65536.0`.
Deterministic on one build; it may lose the last bit beyond 256 m, which
only presentation sees.

U7. **Limits.**

| Constant | Value | Owner |
|---|---|---|
| `COORD_LIMIT_RAW` | `1 << 29` (8,192 m) | sim; nav's equal constant is asserted by the host (H5) |
| `MAX_ROUTE_POINTS` | 64, start included | sim; nav `MAX_PATH_POINTS` equal (H5) |
| `MAX_ROUTE_LENGTH_RAW` | `1 << 28` (4,096 m) | sim; nav equal (H5) |
| `MAX_SPEED_PER_TICK_RAW` | `1 << 18` (4 m per tick) | sim and data validation |
| `MAX_NAV_POLYGONS` | 16,384 | nav |
| `MAX_NAV_VERTICES` | 65,536 | nav |
| `MAX_POLYGON_VERTICES` | 8 | nav |
| `POLICY_MOVE_INTENTS_PER_SEC` / burst | 20 / 40 | net policy (may be lowered) |
| `POLICY_MOVE_IN_FLIGHT` | 8 unreceipted intents per peer | net policy |
| Interpolation delay | 2 ticks (100 ms, spec §2.5) | client policy, measured under D06 |

### R. The path format (D13)

R1. **Types, in `crpg-sim`.**

```rust
pub struct NavRevision(pub [u8; 32]);
impl NavRevision { pub fn of_bytes(bytes: &[u8]) -> Self } // BLAKE3
pub struct RouteTag(pub u64);
pub struct MoveRoute {
    pub revision: NavRevision,
    pub tag: RouteTag,
    pub points: Vec<[Fx16_16; 3]>,
}
```

R2. **Revision is content identity.** It is BLAKE3 over the navmesh's
canonical bytes (V6), through sim's existing `blake3` dependency. A save
restored against a rebaked mesh, or a client holding a different mesh, is
detected by inequality.

R3. **The tag is the host's request identity.** Sim stores and echoes it
and never orders or interprets it.

R4. **Validation order** (`validate_route`, shared by M4 and W4): point
count in `2..=MAX_ROUTE_POINTS` (`RouteSize`); every component within U3
(`OutOfBounds { index }`); every segment's U4 length nonzero
(`ZeroSegment { index }`); total length `<= MAX_ROUTE_LENGTH_RAW`
(`TooLong`); a bound revision (`NavUnbound`) equal to the route's
(`StaleRevision`); `points[0]` equal to the mover's position in all three
components (`StartMismatch`).

R5. **Routes are replaced whole**, never edited.

### M. Sim state and operations (T036)

M1. **State.** `World` gains `movers: ComponentStore<Mover>` (skipped when
empty) and `nav: Option<NavRevision>` (skipped when `None`). Worlds without
movers serialize and hash byte-identically.

```rust
pub struct Mover {          // fields private, read through getters
    position: [Fx16_16; 3],
    speed: Fx16_16,         // per tick, 0 < raw <= MAX_SPEED_PER_TICK_RAW
    route: Option<ActiveRoute>,
}
pub struct ActiveRoute { route: MoveRoute, segment: u16, along: u64 }
```

M2. **Creation.** A creature gets a `Mover` exactly when its ruleset
declares `movement` (C1): at `register_creature`, and when an encounter
start or join spawns a participant. Position is the placement's
`transform.position`; speed is the ruleset's speed resolved at creation (a
constant, or the creature's `Fixed` stat). A speed or position outside the
limits is a typed error and nothing changes. Rulesets without `movement`
produce no movers, so every existing fixture and golden is unchanged.

M3. **`Transform` is a mirror.** Every mover has a `Transform` whose
position is `to_f32(mover.position)` and whose velocity is `to_f32` of its
last stage-4 displacement (zero after any operation). Only movement code
writes it. Stage 4 rewrites every mover's mirror every tick.

M4. **Operations**, each a `World` function with a `HistoryWorld` wrapper of
the same name that journals no history event. All are all-or-nothing.

- `bind_nav(revision) -> Vec<(EntityId, RouteTag)>`: the same revision is a
  no-op; a different one clears every active route (movers stay where they
  are) and returns the cleared routes in ascending `EntityId`.
- `place(entity, position) -> Result<(), MoveError>`: checks
  `AbsentEntity`, `NotMover`, `OutOfBounds`; sets the position and clears
  any route. A trusted host operation, never a client intent.
- `set_route(entity, route) -> Result<Option<RouteTag>, MoveError>`: checks
  `AbsentEntity`, `NotMover`, `DeadCombatant`, `TurnBased` (a participant
  of a TB encounter), then R4. It replaces any active route, returns the
  replaced tag, and starts at segment 0, `along` 0. Movement begins in the
  next tick's stage 4.
- `clear_route(entity) -> Result<Option<RouteTag>, MoveError>`: checks
  `AbsentEntity`, `NotMover`; returns the cleared tag.

M5. **`MoveError` is one closed enum** with the variants named in R4, M4,
W4 and C2, plus `Combat(CombatError)` for the turn checks W4 reuses.
`Display` follows `CombatError`'s `<Code> at <location>`.

M6. **Lifecycle.** `despawn` removes the mover. `transfer_entity` copies
the mover without its route; the host then calls `place` with the arrival
point, because coordinates belong to an area.

M7. **Load coherence.** `World::deserialize` rejects: a route failing R4's
shape checks; `segment >= points.len() - 1` or `along >= seg_len` of that
segment; a position not equal to `point_at` of the route state; a route
while `nav` is `None` or with a different revision; a mover with no
`Transform`, or whose `Transform` position is not `to_f32` of its position.

### W. The two time modes

W1. **Stage 4.** `run_systems` appends `movement_system` after stage 3,
commented `§10 stage 4`. It runs every tick, in and out of encounters, in
both modes. For each mover with a route, in ascending `EntityId`:

```text
if the mover is a dead combatant: clear the route; fact Cancelled{Dead}
budget = speed.raw
loop:
  left = seg_len(p[segment], p[segment+1]) - along
  if budget < left { along += budget; break }
  budget -= left; segment += 1; along = 0
  if segment == p.len() - 1 { position = p[last]; clear route; fact Arrived; break }
position = point_at(p[segment], p[segment+1], seg_len, along)
```

Leftover budget at arrival is discarded. Then stage 4 writes every mover's
mirror (M3). The facts (`Arrived { entity, tag }`, `Cancelled { entity,
tag, reason }`) join `TickFacts` for the host and are not history events.

W2. **Who moves when (ADR-0029 T12).** Non-participants and participants
of an RT encounter move only by `set_route` plus stage 4. Participants of a
TB encounter move only by `perform_move` on their own turn (W4).

W3. **Transitions stop movers.** A TB encounter start, a TB join and the
RT-to-TB switch (ADR-0029 T10) clear the route of every participant
involved. TB-to-RT, leaving and encounter end change nothing. No existing
function gains a return value; the host sees the cleared route.

W4. **A TB move is one atomic action.**
`perform_move(world, actor, &MoveRoute) -> Result<MoveOutcome, MoveError>`
with `validate_move` as its read-only twin. Checks: the `UseAbility` turn
checks (`NoEncounter`, `AbsentActor`, `DeadActor`, `OutOfTurn`) as
`Combat(..)`; mode TB, else `WrongMode` (RT uses `set_route`); `NotMover`;
R4; the C2 cost, else `CannotAfford { cost }`. On success it spends the
cost, sets the position to the last point, writes the mirror with zero
velocity, keeps the turn and draws no RNG. `MoveOutcome` holds actor, cost,
from and to. The wrapper journals `HistoryEvent::Moved { entity, points }`;
the first committed `Moved` makes the wrapper `HISTORY_VERSION` 3 (the D16
pattern), and earlier versions stay readable and unchanged.

W5. **Pause.** A paused host does not tick, so no one moves. A `set_route`
accepted while paused starts on the first tick after unpause (ADR-0029 P2).

W6. **Orders and routes are independent in v1.** Moving does not clear an
RT standing order, and an order does not stop a route. Only X3 changes this.

W7. **AI moves through the same operations**, submitted by the host
(ADR-0029 T15).

### C. Turn-based movement cost

C1. **Data.** A ruleset may declare
`movement { speed: Length, turn: { pool, step: Length, step_cost } }`.
`Length` is a constant `Fx16_16 > 0` or the name of a declared `Fixed`
stat. `pool` names a declared pool; `step_cost >= 1`; a constant speed is
within U7. Absent means "nothing moves". This is a new `crpg-data` schema
version with a migration to absence.

C2. **Cost.** `cost = ceil(L / step) * step_cost`, in `u64`, where `L` is
the route length (U4) and `step` is resolved at action time from the
actor's stats, the same value `validate_action` would read. A resolved
`step <= 0` is `InvalidStep`; a cost above `u32::MAX` is `CannotAfford`.

C3. **Spending.** The pool must hold at least `cost`; exactly `cost` is
spent (`ResourcePool::spend`). A move never ends the turn, and an actor may
move any number of times while it can pay.

C4. **Pure exports:** `route_length(&[[Fx16_16; 3]]) -> Option<u64>` and
`route_cost(length: u64, step: Fx16_16, step_cost: u32) -> Option<u32>`.

C5. **RT and exploration movement spends nothing.** Speed limits it.

### V. Navigation v1 (T035, `crpg-nav`)

V1. **Purity.** Depends on `crpg-core` only; integers and `Fx16_16` only; no
`HashMap`/`HashSet`. T035 adds `crpg-nav` to `tools/lint/determinism.py`'s
`no-hashmap` and `no-float` scope, with a lint self-test. D27c assumes this
lint already covers the crate, but it does not (`CRATES` lists core, rules
and sim).

V2. **Representation.** Convex polygons in `x`/`z`, counter-clockwise, no
reflex vertex (collinear vertices allowed so that splitting at a T-junction
keeps adjacency exact), at most `MAX_POLYGON_VERTICES`. Each v1 polygon is
horizontal (one `y`). Two polygons are adjacent when they share an edge with
the same two vertex indices. Construction validates all of this and fails
with a typed error.

V3. **Bake v1.** `bake_box(min, max, holes)` builds an axis-aligned
rectangle minus axis-aligned rectangular holes, decomposed by one pinned
deterministic sweep (spec §19.1 #21). If T035 leaves holes out, its query
tests use hand-built non-convex meshes instead.

V4. **Queries.** `locate(p) -> Option<PolyId>` (inside or on the boundary;
ties go to the lowest id). `nearest_on_mesh(p) -> [Fx16_16; 3]` (nearest in
`x`/`z`, with a pinned tie-break, and `y` from the polygon).
`find_path(from, to) -> Result<NavPath, NavError>`: A* over polygons with
integer costs and a pinned tie-break, then an integer funnel. The output
starts with `from` exactly, carries polygon heights after that, ends at
`to`'s `x`/`z`, has no zero-length segment, and fails closed with
`StartOffMesh`, `GoalOffMesh`, `AlreadyThere`, `PathTooComplex` (over
`MAX_PATH_POINTS`) or `TooLong`.

V5. **Guarantees.** Results are deterministic, and every returned segment
lies inside the mesh. That second property is the test oracle. Shortest
Euclidean paths are not required in v1.

V6. **Canonical bytes.** A pinned little-endian layout with a version byte
and a `from_canonical_bytes` round trip. Shipping it as a build artifact
(spec §6.3) is a later storage step, not a format change.

V7. **No new dependency.** Search is written in-house. The `pathfinding`
crate stays available under D27c if the user approves it.

### L. Wire (T026p, `crpg-net`)

L1. **Lane 1 is movement**, with its own sequence domain per epoch
(`SEQ_FIRST..=SEQ_LAST`). Lane 0 keeps window 1 (D13).

L2. **Upstream is a reliable ordered stream**: `MoveTo { seq, actor,
destination: [i32; 3] }` (raw `Fx16_16`) and `Halt { seq, actor }`. There
are no other fields (A1).

L3. **Reliable receipts**: `MoveReceipt { seq, actor, outcome }`, where the
outcome is `Accepted { start_tick }`, `Superseded`, or `Rejected { code }`
from a closed code list. Every admitted intent gets exactly one receipt, in
`seq` order. `processed_seq` is the highest receipted `seq`; ticks are
time, and `seq` is identity.

L4. **Samples are unreliable datagrams**:
`MoveSample { server_tick, processed_seq, entries: [{ actor, position:
[i32; 3], moving: bool }] }`. Each datagram is self-contained and within
the transport's datagram limit. A client ignores any sample not newer than
the newest it holds for that entity. Positions are integers, never floats.
This is the first datagram lane, so the T030 dedup patch applies (D23).

L5. **Disclosure.** Position is a per-field grant (T027b, H-filter).
Samples and resync snapshots carry only granted positions, and revocation
stops them at once.

L6. **TB moves ride lane 0** as a combat verb `DeclareMove { actor,
destination }` (window 1, retry and dedup as lane 0). The result is a
`Moved { net_id, points }` projection of `HistoryEvent::Moved`. This needs
a protocol version after T140's, or it folds into T140 if that contract is
still open.

### H. Host adapter (T037, `crpg-server`)

H1. **Area load.** Bake from the area data (v1: `bake_box` from
`Area.bounds`), compute `NavRevision::of_bytes(canonical)`, call
`bind_nav`, and `place` any mover that is off the mesh in `x`/`z` at
`nearest_on_mesh`. Announce the revision to clients with the area.

H2. **`MoveTo`.** Check that the peer controls the actor (D14). Run at most
one navigation query per actor per tick; earlier queued intents for that
actor get `Superseded`. Then `find_path(mover.position,
nearest_on_mesh(destination))` and `set_route`. Success is
`Accepted { start_tick: t + 1 }` when submitted between ticks `t` and
`t + 1`; failure is `Rejected`.

H3. **`DeclareMove`.** The same path computation, then `perform_move`.

H4. **Samples.** Each tick, send the granted entities that moved, plus every
granted position at a refresh interval that T037 pins.

H5. **Shared constants are asserted at compile time.**
`COORD_LIMIT_RAW`, the point limit and the length limit must be equal in
nav and sim. The `crpg-server` to `crpg-nav` path edge needs approval at
contract review.

### G. Client (`crpg-godot`)

G1. **T049, interpolation.** Each entity has a sample buffer keyed by
server tick. Render at the newest tick minus the U7 delay, interpolating
linearly between the two samples around it, after `to_f32`. With no newer
sample, hold the last position; never extrapolate. A TB `Moved` event
animates along its points and overrides samples for that entity until the
animation ends. Revocation removes the entity and its buffer at once.

G2. **T026, prediction.** Only for actors the peer controls, and never in
TB (TB moves spend resources: D13 forbids combat prediction). Prediction
runs only when the client's own baked mesh revision equals the announced
one. It uses local `find_path` and sim's pure step functions (A5), re-timed
to `start_tick` on `Accepted` and dropped on `Rejected` or `Superseded`.
It corrects beyond a measured threshold (D06). T026 pins the buffer bounds.

G3. **T050, click-to-move.** Pick a ground point; preview the path and TB
cost with local navigation and `route_cost` (display hints, spec §9.2);
send `MoveTo` or `DeclareMove` by mode; never send a path. The
`crpg-godot` to `crpg-nav` path edge needs approval.

### X. Range (only if Open question 4 adopts it)

X1. An ability may declare `range: Fx16_16`. Absent means unlimited, so
existing data is unchanged.

X2. `validate_action` checks, after target liveness, that the actor-target
`seg_len` is at most `range`. If either side has no mover, the result is
`OutOfRange`. `legal_actions` omits out-of-range targets.

X3. In RT, a due order whose target is out of range neither executes nor
ends; it stays due. This amends ADR-0029 T7 for this case only.

## Authority and affected surface

- Decided by: pending the user (Proposed).
- Authoritative contracts: the follow-on contracts below; this ADR fixes
  decisions, item numbers and limits, not final signatures.
- Owning crates: `crpg-nav`, `crpg-data`, `crpg-sim`, `crpg-net`,
  `crpg-server`, `crpg-godot`, plus `tools/lint/determinism.py` (V1).

## Compatibility, hash and dependency impact

| Serialized set (plan §1.4) | Touched? | By |
|---|---|---|
| `World` struct | **Yes** | M1 (`movers`, `nav`, both absent when empty) |
| Tick order | **Yes** | W1 appends stage 4; mover-free worlds stay byte-identical |
| `crpg-data` schema | **Yes** | C1 (new ruleset version, migration to absence) |
| `crpg-contracts`, GDExtension boundary, workflows | No | — |

- History: version 3 only on the first `Moved` (W4), with v1 and v2 still
  readable. Protocol: lane 1 and a lane-0 verb in a new version (L1-L6);
  v1, v2 and T024 frozen.
- Goldens: every existing golden stays byte-identical, and a test proves
  it. Movement goldens are new, per target (ADR-0012), H-golden. Nav results
  are integer, but goldens stay target-scoped.
- Dependencies: no external crate. Path edges `crpg-server` and `crpg-godot`
  to `crpg-nav` are allowed by `ALLOWED` but need approval; `ALLOWED` is
  unchanged. `blake3` is reused inside sim.
- Platform: pure Rust, no OS branch, and no Godot below `crpg-godot`.

## Alternatives considered

- **`f32` authoritative positions, with quantized costs** (the literal
  reading of ADR-0029's Consequences): rejected. A cost would be derived
  from a float, client prediction would depend on float steps, and D27c
  already makes navigation fixed-point.
- **A fixed-point `Transform`**: rejected. It changes an existing hashed
  shape and every golden that holds a transform.
- **A shared path type in `crpg-core`**: rejected. It grows core for one
  producer and one consumer; a host copy is enough.
- **Client-supplied paths**: rejected. They could go through walls.
- **TB moves that walk over ticks**: rejected. They put combat in the TB
  tick, create half-moved states, and stall TB play when the host does not
  tick. With an atomic move, traps and interruptions can later cut the path
  at command time.
- **Latest-seq-wins datagram intents (the ADR-0004 spike)**: rejected for
  click-to-move. A lost click stays lost, and a peer that controls several
  actors cannot tell which actor's intent was dropped. Intents are sparse,
  so a reliable stream costs little; positions still travel as datagrams.
- **3D length**: rejected. Tabletop rules measure on the map, and it would
  tie cost to how heights are represented.
- **A grid representation**: spec §20 row 10 rejected a custom grid for
  navigation. A grid for rule measurement is Open question 1.
- **Third-party navigation crates**: rejected by D27c.

## Consequences and follow-on contracts

| Contract | Crate | Must pin | Depends on |
|---|---|---|---|
| T035 | `crpg-nav` (+ lint scope) | V1-V7, U3, U4 | Accepted |
| new: movement schema | `crpg-data` | C1 | Accepted; Q2, Q3 |
| T036 (may split: a = state, ops, stage 4; b = TB move, history v3) | `crpg-sim` | U2-U7, R1-R5, M1-M7, W1-W7, C2-C5 | the schema task |
| T026p | `crpg-net` | L1-L6, U7 policies | T036, T140 |
| T037 | `crpg-server` | H1-H5 | T026p, T035 |
| T049, T026, T050 | `crpg-godot` | G1, G2, G3 | as the plan lists, plus T037 |
| new, only if Q4 says so: range | `crpg-data`, then `crpg-sim` | X1-X3 | T036 |

- The plan lists T036 as needing only T034. It also needs the schema task.
- Spec §6.3 (Recast bindings, Detour) and §20 row 10 need a note pointing to
  D27c and this ADR after acceptance (the manager's edit).

## Open questions for the user

1. **Free movement or a grid?** Characters can either walk anywhere on the
   walkable ground, measured in metres (Baldur's Gate 3, Pillars), or step
   between squares or hexes (XCOM, classic tabletop maps). The choice
   decides how distances read to players and how authors write speeds and
   ranges. The spec already picked a navigation mesh, which is free
   movement. *Recommendation:* free movement in metres for v1. A ruleset
   could later add a grid as its way of measuring distance.
2. **How does a turn-based move get paid for?** In D&D 5e you have a
   separate movement allowance per turn. In Fallout and Divinity, moving
   uses the same action points as attacking. In Pathfinder 2e, one action
   moves you up to your speed. *Recommendation:* one rule covers all three
   (C2): the ruleset names the pool, how far one step is, and what a step
   costs. Each ruleset picks its style in data.
3. **How fast do characters walk in real time?** Either each creature has a
   walking speed used when exploring and in real-time fights (Baldur's
   Gate), or the speed comes from the turn-based allowance (for example
   9 m per 6-second round, which is slow). *Recommendation:* a separate
   walking speed, a constant or a creature stat. Turn-based distance stays
   governed by the pool.
4. **Do attacks need the attacker to be close enough?** Today any attack
   reaches anyone, so movement changes nothing in a fight. *Recommendation:*
   approve X1-X3 now, as a small follow-on (data, then sim) right after
   T036, so movement matters before the Godot click-to-move demo.
5. **Can characters walk through each other and stop on the same spot?**
   Blocking needs body sizes and makes paths depend on who stands where,
   which also breaks client prediction. *Recommendation:* yes, they pass
   through in v1 (spec §6.3 skips avoidance). Spacing rules come later.
6. **What waits until after v1?** *Recommendation:* defer difficult terrain,
   slopes and multiple floors, doors that change the walkable area at
   runtime (the revision mechanism R2 is ready for them), trigger volumes,
   facing and rotation (the client turns models toward their motion),
   opportunity attacks (ADR-0029 K5), formations, and follow-the-leader.

## Acceptance and evidence

Not yet run. Each follow-on contract turns its items into tests. Required
across them: every existing golden and a mover-free world stay
byte-identical (M1, W1); per-target goldens for an RT walk, a TB move with
cost and a split-party tick (W1-W4); nav path literal tests and the
segment-inside-mesh oracle (V4, V5); and load-rejection tests for every M7
rule.

## Supersession and corrections

## Agent log

- 2026-11-08 (UTC) · claude-code/opus-5.5 (architect) + T034 · Filed the proposed movement and navigation ADR. Authoritative positions are fixed-point with an `f32` mirror, the host supplies nav routes to sim by revision and tag, TB moves are atomic pool spends, and lane 1 carries reliable intents with datagram samples. Six user questions are left open.
