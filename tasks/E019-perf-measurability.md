## Task
Make the performance targets falsifiable and task the bench harness.
Human-decision task (spec edit + task scoping).

## Why this is deferred

Every §13.1 row (`docs/CRPG_ENGINE_SPEC.md:939-950`) lacks load definition,
fixture, or harness: "active" (200+2000) undefined; AI budget numbers absent;
"designed-for 32" untestable by construction; ≤8 ms gate needs "target load"
plus a flaky-absolute-ms policy on shared runners (contradicting the 20%
relative gate in `:960` — which baseline? stored where?); bandwidth rows
don't state the interest tier assumed; pathfinding average has no corpus;
60 fps names a 2019 GPU the project never measured on. Worse, the
1,000-entity / 60 fps / no-LOD triple is already falsified by the project's
own spike (43.7 fps @1000 on an RTX 4060 Laptop,
`docs/adr/0003-gdextension-rendering-spike.md:33-39`), while §13.3 defers LOD
and §9.1 specifies only the per-entity polling the ADR warns against
(`:48-56`: bulk snapshot + animation LOD never landed). `crpgc bench`
(`:960`) has no BACKLOG task and the self-hosted runner is still missing
(`tasks/BACKLOG.md:115-118`).

## Decision to make
- Downgrade the ceiling or scope LOD (resolve the falsified triple); define
  load/fixture/harness per row or demote rows to aspirations; task `crpgc
  bench` + baseline policy + bulk-snapshot bridge API.

## Deliverable
- Edits to `docs/CRPG_ENGINE_SPEC.md` (§§9.1, 13) with dated notes + BACKLOG
  bench-harness row. No source changes.

## Constraints
- Doc-only. Human sign-off; perf rows become CI gates only through this task.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: unfalsifiable targets plus one already-falsified triple.
