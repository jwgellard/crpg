## Task
Finish the ADR-0008 alignment that the E001 batch left incomplete: three spec
lines and two E-file headers still show the pre-split picture. Human-decision
task (spec edit).

## Why this is deferred

The E001 batch updated BACKLOG, README's role table, and spec §16.2, but left
residue an agent reading the spec first will hit before BACKLOG:

- The `World` sketch still shows bare `events: EventQueue`, unparameterized
  (`docs/CRPG_ENGINE_SPEC.md:233`), not `EventQueue<SimEvent>` per
  `docs/adr/0008-event-ownership.md:37`.
- The layer diagram (`docs/CRPG_ENGINE_SPEC.md:165`) and repo tree
  (`docs/CRPG_ENGINE_SPEC.md:985`) say "event queue" in core without the
  "generic substrate" qualifier — a T007 reader sees core owning the queue.
- The README ASCII layer diagram (`README.md:37`) still says "events" in
  `crpg-core` while the role table below it (`README.md:56-59`) was fixed —
  the same file disagrees with itself.
- Spec §24 T7/T10/T14 (`docs/CRPG_ENGINE_SPEC.md:1617-1623,1641-1647,1673-1679`)
  omit the ownership BACKLOG already assigns (`tasks/BACKLOG.md:47,68,77`).
- `tasks/E001-events.md:23-27` still poses the pre-split A-vs-B binary with no
  "superseded by A′" header; `tasks/E008-wallclock-event-budget.md:7` cites
  "Spec §4" for what is §5.2.

## Decision to make
- Narrow the three spec lines to "generic substrate" wording and parameterize
  the sketch; fix the README diagram line; mirror the BACKLOG assignments into
  spec §24 T7/T10/T14; add superseded/citation headers to E001/E008.

## Deliverable
- Edits to `docs/CRPG_ENGINE_SPEC.md` (§2.2 sketch, §2.4 diagram, §14 tree,
  §24 T7/T10/T14), `README.md:37`, `tasks/E001-events.md`,
  `tasks/E008-wallclock-event-budget.md`, each with a dated inline note.
- No source changes.

## Constraints
- Doc-only; no `crpg-*` sources. Human sign-off so the spec stays the
  authority agents read first.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: collects the five leftover lines the ADR-0008 batch knowingly deferred.
- 2026-09-06 (UTC) · opencode/muse-spark + maintainer sign-off · Applied all five: spec sketch parameterized to `EventQueue<SimEvent>`, diagram/tree lines say generic substrate, §24 T7/T10/T14 mirror the BACKLOG assignments, E001/E008 headers fixed; see spec and README Agent logs.
