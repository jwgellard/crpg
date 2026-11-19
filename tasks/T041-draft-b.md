# T041 — scripted headless clients and desync suite (`crpg-testkit`)

**Status: draft, 2026-11-19 (UTC). Not approved.**

## Task
Give `crpg-testkit` the Phase 4 headless-client proof the M4 exit criteria
demand (`docs/IMPLEMENTATION_PLAN.md:168-174`): scripted clients that connect,
move and fight; a 5,000-tick loss/jitter desync with semantic comparison; two
clients ending with identical visible state; and the gate 13 dedicated smoke
(`docs/IMPLEMENTATION_PLAN.md:181-183`). It also takes the host-level half of
the malicious-client suite: T042 is the wire level in `crpg-net`, and
"host-level cases live in T041" (`docs/IMPLEMENTATION_PLAN.md:247`;
`tasks/T042.md:39`), indexed by T042's D-1 "Host level" column
(`tasks/T042.md:431-446`) plus the non-canonical-retry case T042 hands over
(`tasks/T042.md:694-696`) and the D67a server-log-line backlog row
(`tasks/BACKLOG.md:341-348` region; `tasks/DECISIONS-D67.md:5-12`).

One part, **T041**, in `crpg-testkit`, tests only: no `src/` change, no public
export, no golden. Not S-set (`docs/IMPLEMENTATION_PLAN.md:86-98`): the T041
row carries no S marker (`docs/IMPLEMENTATION_PLAN.md:246`), and nothing below
touches `World`, tick order, wire bytes, checkpoints, history versions,
schemas or manifests. Prerequisites: T025b, T027c, T037, T039 (the plan row).
T040 is not built on this base (the binary still refuses to run,
`crates/crpg-server/src/main.rs:10`), so the smoke drives the library
adapters, not the binary; T153's `Host::drive_ai` is specified
(`tasks/T153.md:60-66`) but not built here (only the run-loop hook
`Hosting::drive_ai(now_ms)` exists, `crates/crpg-server/src/run.rs:45`), so
the fight is scripted intents, not AI decisions (Q4).

## Parts

## Readings

## Interface

## Behaviour

## Errors

## Tests

## What must not change

## Dependency edges

## Out of scope

## Open questions

## Agent log
- 2026-11-19 (UTC) · opencode-go/muse-spark-1.3-contributor + T041 draft b · Skeleton with all section headings.
