# T041 — scripted headless clients and the desync suite (`crpg-testkit`)

**Status: draft, 2026-11-19 (UTC). Not approved.** Blind draft A, written by
DeepSeek V4 Pro, session 79, slot 4, attempt 1. A second model is drafting the
same contract blind; a Claude architect writes the final contract from the two
drafts and the user approves it. Nothing here is approved.

Written from the T041 row (`docs/IMPLEMENTATION_PLAN.md:246`), ADR-0012
(`docs/adr/0012-windows-primary-platform.md`), D65a, D67a, D70 (approved; the
dedicated server program is specified in `tasks/T040.md`, not built yet) and
the root `AGENTS.md`. Testkit rules: root `AGENTS.md` (testkit is a one-way
integration consumer) and the `ALLOWED` table in `tools/lint/deps.py`. The
fact sheet `tasks/T041-facts.md` is the map; every fact relied on was
re-checked in code. Citation convention (T140's): a bare `:N` is a line of
the file cited earlier in the same item.

## Task

Give `crpg-testkit` the headless-client machinery the Phase 4 exit criteria
need (`docs/IMPLEMENTATION_PLAN.md:171–172`, spec
`docs/CRPG_ENGINE_SPEC.md:1488`, `:1587–1589`), all tests-only, no `src/`
change, no public export:

- **scripted headless clients**: one tests-only harness assembling the
  net-local modules a Godot-free client needs (`crates/crpg-net/src/lib.rs:66`)
  around the in-memory host link — `LocalHost`/`LocalClient`
  (`crates/crpg-server/src/run.rs:332`, `:818`), `ReplicaWorldV4`
  (`crates/crpg-net/src/replica_v4.rs:55`), `MoveOutbox`
  (`crates/crpg-net/src/outbox_v4.rs:54`) — so a test scripts two clients
  through attack/move/end-turn intents;
- **a 5,000-tick loss/jitter desync test**: run the authoritative host and two
  client replicas over the simulated network's seeded `FaultSchedule`
  (`crates/crpg-net/src/sim.rs:321`) for 5,000 ticks and assert, at fixed
  checkpoints, that each replica's visible state equals the host's filtered
  projection (semantic comparison, never hash equality — ADR-0009,
  `docs/adr/0009-determinism-scope.md:46`);
- **two clients with identical visible state**: both clients bound with equal
  disclosure grants converge to equal replicas after the scripted run
  (`docs/CRPG_ENGINE_SPEC.md:1527`);
- **the gate 13 dedicated smoke**: two clients over the real dedicated
  `crpg-server` binary move, fight and end with identical visible state
  (`docs/CRPG_ENGINE_SPEC.md:1589`);
- **the host-level malicious-client cases** the T042 row hands here
  (`docs/IMPLEMENTATION_PLAN.md:247`): the "Host level" column of
  `tasks/T042.md` D-1 — ownership, timing, interest, flooding, replay, and a
  same-`seq` retry with a non-canonical encoding treated as a cached retry,
  never a `SeqConflict` (`tasks/T042.md:693–696`).

Testkit is a one-way integration consumer (root `AGENTS.md:36–42`): everything
T041 builds lives under `crates/crpg-testkit/tests/`, never ships, and reaches
the host through the dev-only `crpg-server` edge T037e already opened
(`crates/crpg-testkit/AGENTS.md:137–140`).

## Parts

_(filled next edit)_

## Readings

_(filled next edit)_

## Interface

_(filled next edit)_

## Behaviour

_(filled next edit)_

## Errors

_(filled next edit)_

## Tests

_(filled next edit)_

## What must not change

_(filled next edit)_

## Dependency edges

_(filled next edit)_

## Out of scope

_(filled next edit)_

## Open questions

_(filled next edit)_

## Agent log

_(filled next edit)_
