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

## Interface

## Behaviour

## Errors

## Tests

## What must not change

## Out of scope

## Open questions (each with a recommendation)

## Agent log
