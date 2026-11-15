# T037-draft-a — blind draft of the T037 host movement adapter contract

## Task

Draft the exact contract for T037, the `crpg-server` host movement adapter. A
v4 session (HM-1) runs protocol v4 on lane 0 and movement subprotocol `0xA1`
on lane 1; the host bakes and binds navigation at area load (H1), decides
`MoveTo` and `DeclareMove` against nav and sim with receipts (H2, H3, HM-4,
HM-9), samples positions by datagram at a pinned refresh interval (H4, HM-5,
HM-6), and asserts the shared constants at compile time (H5, S3). `crpg-net`
supplies bytes, filters and views; `crpg-sim` owns movers, routes and
`HistoryEvent::Moved`; `crpg-nav` computes paths. The host is the only caller
that can supply a route (A2, A3). All movement change to a `World` is a sim
operation or tick stage 4 (A4); pause, rate limits and nav queries stay here.

## Parts

Serialized. Part 1 lands the nav/sim adapter and can be tested against an
in-memory baked mesh; Part 2 adds lane-1 admission and receipts on top of it;
Part 3 adds the v4 delivery surface (pages, `Moved`, samples). Each part is
one implementer run. Real-history and real-navigation integration tests live
in `crpg-testkit` and are owned by the part whose behaviour they exercise.

- **Part 1 — area load and navigation binding (H1, H5, S3).** Crate
  `crpg-server`.
  - Owns: `crates/crpg-server/Cargo.toml` (add `crpg-nav`; see Open
    question 1), `crates/crpg-server/src/host.rs` (area load, `NavRevision`
    accessor, compile-time constant asserts, `PositionRow` builder helper
    skeleton), `crates/crpg-server/tests/host_nav.rs` (new),
    `crates/crpg-testkit/tests/nav_host.rs` (new, real mesh).
  - Why the cut is here: H1 is the only part that touches `crpg-nav` and
    `Area.bounds`; it has no protocol dependency, so it can land and be
    golden-checked before any lane-1 code.
  - goldens: none in `crates/crpg-server/tests/goldens/` (the directory does
    not exist); `crates/crpg-sim/tests/goldens/movement_v1.*.txt` and
    `history_v3.*.txt` must not move, because a mover-free world stays
    byte-identical (M1, W1).

- **Part 2 — `MoveTo`/`DeclareMove` decisions (H2, H3, HM-4, HM-9).**
  Crate `crpg-server`.
  - Owns: `crates/crpg-server/src/host.rs` (decision routine, per-actor
    per-tick nav-query gate, `Superseded` handling, `set_route`/
    `perform_move` calls, receipt construction), `crates/crpg-server/tests/host_move.rs` (new).
  - Why the cut is here: the path computation and sim mutation are the
    smallest unit that produces a receipt; lane-1 admission (Part 3) can
    call it without knowing nav.
  - goldens: same sim goldens as Part 1 must not move; v3 sessions gain a
    verb but no existing `authority_hash` changes.

- **Part 3 — lane-1 admission, receipts, aggregates and v4 delivery (HM-1..
  HM-10, H4, H5 net).** Crate `crpg-server`, with `crpg-testkit` for the
  real-history integration case.
  - Owns: `crates/crpg-server/src/host.rs` (lane-1 FIFO and cache, admission
    pipeline, aggregates, position rows, `Moved` candidates, sample datagrams,
    refresh tick), `crates/crpg-server/src/host_v3.rs` (page `Moved` op,
    position anchor, sample clock bookkeeping), `crates/crpg-server/tests/host_move_lane1.rs` (new),
    `crates/crpg-testkit/tests/move_host.rs` (new).
  - Why the cut is here: it is the only part that depends on `crpg-net` v4
    types and on the v3 publication path; keeping it last avoids touching
    `host_v3.rs` until Parts 1–2 are stable.
  - goldens: the six `crates/crpg-testkit/goldens/*.golden` must not move;
    all `crates/crpg-sim/tests/goldens/*.txt` must not move.

## Readings of the ADR and T026p

1. **Protocol version is chosen per session, never negotiated (HM-1,
   T026p Reading 1).** A v4 session speaks lane-0 v4 and lane-1 `0xA1`;
   `bind_peer`/`bind_snapshot_peer` keep v1/v2 and `bind_v3_peer` keeps v3
   (T141 Reading 7). *Rejected:* a session that upgrades mid-life; it would
   split the retry cache and the `InterestView` minting, and T026p forbids
   negotiation.

2. **Lane-1 admission mirrors lane 0's nine steps, with its own sequence
   domain and retry cache (HM-2).** `crpg-net` keeps window 1 on lane 0.
   *Rejected:* one merged FIFO across lanes; a lane-1 flood would head-of-line
   block a combat retry (HM-10).

3. **Lane-1 frames count toward every aggregate (HM-3).** `POLICY_INTENT_*`,
   ingress and egress caps, failure budgets. *Rejected:* lane-1-only buckets;
   they would let a peer double its intent rate by splitting lanes.

4. **Receipt decision order is `Superseded`, `WrongMode`, `IllegalAction`,
   then `Accepted` (HM-4).** `Superseded` wins even when the later intent is
   illegal, so a peer learns its earlier click was dropped. *Rejected:* running
   `validate_action` first; it spends a nav query on an intent that will be
   dropped and hides the supersession.

5. **`MoveTo` is real-time, `DeclareMove` is turn-based (H2, H3, L6).**
   `MoveTo` calls `set_route`; `DeclareMove` runs the same path computation
   then `perform_move`. *Rejected:* one verb with a mode flag; the wire tags,
   rights and receipt codes differ (`WrongMode` 22).

6. **At most one navigation query per actor per tick (H2).** Earlier queued
   intents for that actor get `Superseded` with no `find_path` call.
   *Rejected:* one query per intent; A* plus the funnel per click would
   dominate a tick and the later route would win anyway (R5).

7. **Destination snap happens before `nearest_on_mesh` (S3).** Host and
   client preview call `snap(measure, point)` (identity in v1). *Rejected:*
   snapping inside `find_path`; nav has no `Measure` and sim's `snap` is the
   seam (T036 Out of scope).

8. **`HistoryEvent::Moved` becomes `EventCandidate::Moved` at the history slot
   (HM-8).** It replaces the `Moved => None` arm at
   `crates/crpg-server/src/host.rs:2489`. Disclosure follows the viewer's
   position grant; points are validated by `check_points` and a failing
   candidate is omitted, not refused (T026p Reading 9). *Rejected:* liveness-
   checking the mapped entity in the replica; T140r Reading 10 leaves
   notices unchecked.

9. **Samples are datagrams only, never lane-0 `event_seq` (HM-5).**
   `project_samples` with the refresh interval, `sample_datagrams`, and
   `processed_seq` is the highest lane-1 `seq` receipted. *Rejected:* samples
   on the stream; they would cost one reliable op per mover per tick, which
   L4 moved to datagrams.

10. **Position goes with visibility (T026p Reading 7).** One `PositionRow` per
    visible entity with a mover; `granted` true exactly when the viewer is
    granted presence; `moving` true exactly with a route. *Rejected:* a
    separate position grant; it would add an authoring setting and a second
    disclosure path, and fog of war later changes only host policy.

11. **Every v4 page is built by `project_page_v4`, in the same tick as
    `project_snapshot_v4`, with `Clock` and `HostFacts` (HM-7).** *Rejected:*
    a `0x83` snapshot body carrying positions; Reading 8 chose the first page
    after the snapshot as the anchor.

12. **Lane-1 traffic beside a lane-0 retry spends nothing twice and reorders
    no lane-0 receipt (HM-10).** Lane-0 runs to completion first, then
    lane-1; aggregates are shared but the two FIFOs are not merged.
    *Rejected:* a single merged pump; it would let a lane-1 intent consume a
    reservation slot ahead of a queued combat retry.

13. **Shared constants are asserted, not imported (H5, T026p Reading 13).**
    `crpg-server` (and `crpg-net`) cannot use `crpg-sim`/`crpg-nav` outside
    tests except by the approved edges, so the host asserts equality at
    compile time. *Rejected:* importing `crpg-nav`'s constants into sim; nav
    depends on `crpg-core` only (V1).

## Interface

Additions only. `HostError` gains `NavBake(crpg_nav::BakeError)` after
`Invariant`. Every public item has a doc comment. No `HashMap`/`HashSet`,
`f32`/`f64`, `unsafe`, clock, thread or filesystem call in `host.rs`.

### Constants and H5 asserts (`crates/crpg-server/src/host.rs`)
```rust
/// Stationary-position refresh interval for `MoveSample` datagrams.
pub const MOVE_SAMPLE_REFRESH_TICKS: u64 = 4;

const _: () = assert!(crpg_nav::COORD_LIMIT_RAW == crpg_sim::movement::COORD_LIMIT_RAW);
const _: () = assert!(crpg_nav::MAX_PATH_POINTS == crpg_sim::movement::MAX_ROUTE_POINTS);
const _: () = assert!(crpg_nav::MAX_PATH_LENGTH_RAW == crpg_sim::movement::MAX_ROUTE_LENGTH_RAW);
const _: () = assert!(crpg_net::protocol_v4::COORD_LIMIT_RAW == crpg_sim::movement::COORD_LIMIT_RAW);
const _: () = assert!(crpg_net::protocol_v4::MAX_MOVED_POINTS == crpg_sim::movement::MAX_ROUTE_POINTS);
```

### Area load (Part 1)
```rust
impl Host {
    /// Bakes nav from `area.bounds` (v1 `bake_box`), binds the revision,
    /// places off-mesh movers at `nearest_on_mesh`, returns the revision.
    pub fn load_area(&mut self, area: &crpg_data::Area)
        -> Result<crpg_sim::movement::NavRevision, HostError>;
    pub fn nav_revision(&self) -> Option<crpg_sim::movement::NavRevision>;
}
```

### Movement decisions (Part 2)
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDecision {
    Accepted { start_tick: u64 },
    Superseded,
    Rejected(crpg_net::protocol_v3::RejectionCode),
}

impl Host {
    /// Real-time: path compute + `set_route`. Turn-based callers use
    /// `declare_move`. `peer` must control `actor`; control is checked first.
    pub fn decide_move_to(
        &mut self, peer: PeerHandle, actor: crpg_core::EntityId,
        destination: [crpg_core::Fx16_16; 3], now_tick: u64,
    ) -> Result<MoveDecision, HostError>;

    pub fn declare_move(
        &mut self, peer: PeerHandle, actor: crpg_core::EntityId,
        destination: [crpg_core::Fx16_16; 3], now_tick: u64,
    ) -> Result<MoveDecision, HostError>;
}
```

### Lane-1 admission (Part 3)
```rust
impl Host {
    pub fn ingest_move(&mut self, peer: PeerHandle, bytes: &[u8])
        -> Result<Step, HostError>;
    pub fn pump_moves(&mut self);
}
```
`Step` is the existing `crates/crpg-server/src/host.rs` enum (`Step::Accepted`,
`Step::Blocked`, …); lane-1 uses the same nine-step order (HM-2) with a
separate `cache_move: VecDeque<CacheEntry>` and `next_move_seq` per session.

### Position rows and samples (Part 3)
```rust
impl Host {
    pub fn position_rows(&self, peer: PeerHandle)
        -> Result<Vec<crpg_net::positions_v4::PositionRow>, HostError>;
    pub fn take_datagrams(&mut self, peer: PeerHandle)
        -> Result<Vec<Vec<u8>>, HostError>;
}
```

### Session state additions
```rust
// In Session (host.rs:605-637):
pub v4: Option<Box<V4Session>>,   // movement bookkeeping for a v4 session
// V4Session (new, in host.rs):
pub struct V4Session {
    pub next_move_seq: u64,
    pub cache_move: VecDeque<CacheEntry>,
    pub cache_move_bytes: usize,
    pub last_sample_tick: u64,
    pub move_refresh_ticks: u64,   // == MOVE_SAMPLE_REFRESH_TICKS
}
```

### Host-error mapping for nav
`load_area` maps `BakeError` to `HostError::NavBake` and performs no sim
mutation on failure. `decide_move_to`/`declare_move` never return a
`HostError` for a nav refusal; the refusal is a `MoveDecision::Rejected`.

## Behaviour

Normative. First failure wins in every list. "Refused" means the stated
error and no state change.

### Area load (H1)
B-1. **Bake.** `load_area` calls `crpg_nav::bake_box` with
    `area.bounds` as the outer rectangle and no holes (v1). `BakeError` is
    returned as `HostError::NavBake` with no mutation.
B-2. **Revision.** `NavRevision::of_bytes(mesh.to_canonical_bytes())`
    (`crates/crpg-sim/src/movement.rs:53-58` for `of_bytes`,
    `crates/crpg-nav/src/mesh.rs:310` for canonical bytes).
B-3. **Bind.** `history.world_mut().bind_nav(revision)`; the returned cleared
    `(EntityId, RouteTag)` pairs are logged at debug only. A changed
    revision clears every route (M4).
B-4. **Place.** For every mover whose `position()` in `x`/`z` is off the mesh
    (`crpg_nav::locate(...) == None`), call `place(entity, nearest_on_mesh(position))`.
    `place` failures are `Invariant` (a mover that cannot be placed is a
    load-time defect).
B-5. **Announce.** The revision is stored in the world and returned; the host
    does not push a frame here. Publication happens on the next page
    (`Clock`/`HostFacts` carry the revision only if the host chooses to
    publish it; v1 has no nav-revision op, so it is held for T049).

### `MoveTo` (H2, HM-4)
B-6. **Control check first.** `peer` controls `actor` (D14), else
    `MoveDecision::Rejected(NotAuthorized)`. No nav query.
B-7. **Per-actor per-tick gate.** If a navigation query has already run for
    `actor` this tick, the intent is `MoveDecision::Superseded` and no
    `find_path` runs (H2). The gate is keyed by `(tick, actor)` in a
    `BTreeSet` cleared at the start of `Host::tick`.
B-8. **Path.** `find_path(mover.position(),
    nearest_on_mesh(snap(measure, destination)))`. `measure` is the
    encounter's `Measure` (Euclidean in v1) or `Measure::Euclidean` outside
    an encounter. `snap` is `crpg_sim::movement::snap`
    (`crates/crpg-sim/src/movement.rs:384`).
B-9. **Set route.** On `Ok(path)`, copy `path.into_points()` into a
    `MoveRoute { revision: world.nav().unwrap(), tag: RouteTag(next), points }`
    and call `set_route(actor, route)`. Tag is a host counter, never
    interpreted by sim (R3).
B-10. **Outcome.** `Accepted { start_tick: now_tick + 1 }` on success;
    `Rejected(IllegalAction)` on any `NavError` or `MoveError`, with the
    specific code recorded in the host log. `set_route`'s `TurnBased` becomes
    `WrongMode` when the actor is a turn-based participant (HM-4's
    `WrongMode`).

### `DeclareMove` (H3, HM-9)
B-11. Same control check, gate, path computation and `set_route` as B-6..B-10,
    then `perform_move(world, actor, &route)` (sim spends the pool and
    journals `HistoryEvent::Moved`). `perform_move`'s `WrongMode` stays
    `WrongMode`; `CannotAfford` stays `IllegalAction`. On success the host
    builds `EventCandidate::Moved` at the history slot (HM-8).

### Lane-1 admission (HM-2)
B-12. **Nine steps**, in this order, mirroring
    `crates/crpg-server/src/host.rs:1588` `admit`:
    (1) decode with `movement_v4::decode_intent`;
    (2) session epoch (no session or different epoch → `SessionExpired`);
    (3) cache first — identical bytes re-send the retained receipt, different
    bytes → `SeqConflict` and fence;
    seq bounds `SeqExhausted`/`SeqGap`/`StaleSeq`;
    (4) freshness against `POLICY_OBSERVED_TICK_WINDOW` (`FutureTick`/
    `StaleTick`); lane-1 has no `observed_tick`, so freshness is checked
    against the receipt's `processed_tick` window;
    (5) mapping/control/disclosure resolution;
    (6) `validate_action` only for `Halt` (a `MoveTo` is a movement verb, not
    a combat action);
    (7) reservation of `POLICY_MOVE_IN_FLIGHT` in-flight intents;
    (8) execution/commit split;
    (9) receipt construction.
    First failure wins.

### Aggregates (HM-3)
B-13. Every admitted lane-1 intent spends `intent_frames`/`intent_bytes`,
    counts toward `ingress_frames`/`ingress_bytes`, and on refusal spends a
    failure bucket. The caps are the existing ones (`QueueCaps::v1`,
    `RateCaps::v1()`); no new aggregate.

### Receipts (HM-4)
B-14. Exactly one `MoveReceipt` per admitted intent, in `seq` order, on the
    lane-1 stream. `processed_tick` is the tick at which the decision runs.
    `Accepted { start_tick: t + 1 }` matches B-10.
B-15. `Superseded` is decided before `WrongMode`/`IllegalAction` (HM-4).

### Samples (H4, HM-5, HM-6)
B-16. **Each tick**, for every v4 session:
    (a) collect granted entities that moved this tick (mover position changed
    in stage 4, or `HistoryEvent::Moved` committed) into `SampleEntry`s;
    (b) if `server_tick - last_sample_tick >= MOVE_SAMPLE_REFRESH_TICKS`,
    collect every granted position (one `PositionRow` with `granted == true`)
    into the same list, deduplicated by `NetId` (moved entries first, then
    the refresh entries not already present);
    (c) encode with `movement_v4::sample_datagrams(server_tick, processed_seq,
    entries)` and send as datagrams via `take_datagrams`.
B-17. `processed_seq` is the highest lane-1 `seq` receipted to that peer (0
    before any).
B-18. Samples never enter lane-0 `event_seq` and never use a stream.

### `Moved` candidate (HM-8)
B-19. At the history slot, for each `HistoryEvent::Moved { entity, points }`,
    build `EventCandidate::Moved { entity: view.net_id(key_of(world, entity)),
    points: points mapped to `Point` (raw `Fx16_16`), disclose: the viewer's
    position grant for that entity }`. Points are converted with
    `Fx16_16::to_raw` and validated by `check_points` before projection; a
    failing candidate is omitted (T026p Behaviour Q-8).

### Position rows (HM-6)
B-20. One `PositionRow { key, granted, point, moving }` per visible entity
    with a mover. `granted` true iff the viewer is granted presence for that
    entity; `moving` true iff the mover has `route() == Some`. `point` is
    `mover.position()` converted to `Point`. Rows for non-visible entities are
    not built.

### Pages (HM-7)
B-21. Every v4 page is built by `project_page_v4` with the rows from B-20;
    in the same tick as `project_snapshot_v4` for a staging, with `Clock` and
    `HostFacts`. Position anchors follow T026p Q-4 (forget on revoke, resend
    on `context.resend`).

### v3 byte-identical
B-22. A host that never calls `load_area`, `decide_move_to`,
    `declare_move`, `ingest_move` or `pump_moves` writes byte-identical
    checkpoints and delivers byte-identical frames to v1/v2/v3 sessions
    (B-22 is the "What must not change" contract).

## Errors

## Tests

## What must not change

## Out of scope

## Open questions (each with a recommendation)

## Agent log
