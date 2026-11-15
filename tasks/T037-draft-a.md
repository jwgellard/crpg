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

## Behaviour

## Errors

## Tests

## What must not change

## Out of scope

## Open questions (each with a recommendation)

## Agent log
