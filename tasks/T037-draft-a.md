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

## Readings of the ADR and T026p

## Interface

## Behaviour

## Errors

## Tests

## What must not change

## Out of scope

## Open questions (each with a recommendation)

## Agent log
