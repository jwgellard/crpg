## Task
Resolve event ownership (core vs sim) so T007/T008/netcode do not invent
incompatible event types. Human-decision task — NOT for autonomous execution.

> **Decided 2026-09-05 as A′** (generic substrate in core, `SimEvent` in sim,
> IR types in data, hooks in rules) — recorded in ADR-0008. The A-vs-B choice
> below is retained as history.

## Why this is deferred

The 2026-09-06 full-project review found that the status docs and roadmap are
mutually inconsistent about where events live:

- `README.md:54` assigns "events" to `crpg-core` (role column).
- Spec §24 role description for core says "…time, events, errors".
- Spec §16.2 core test list promises "event ordering" property tests.
- But T006 (spec §24, split into T006a–e) is complete and its work list omits
  events entirely. The current `docs/architecture/crpg-core.md` declares the
  core primitives complete without them.
- No remaining numbered task (T007, T008, …) explicitly owns the event payload
  types or the event queue.

## Decision to make

Choose one, and record it (an ADR or an architecture-doc statement):

- **Option A — payload types in core, queue in sim.** Event *payload* types live
  in `crpg-core` (so net/script/persist share them), the event queue/tick
  ordering lives in `crpg-sim`. This matches `README.md`/spec loosely.
- **Option B — both in sim.** Sim owns events start to finish; core stays
  primitives-only (simpler, but every consumer of event types must reach up).

Note AGENTS.md's dependency rule: `crpg-net`, `crpg-script`, `crpg-persist` may
depend on `crpg-sim`, so if event types live in sim those consumers can use
them; `crpg-edit` and `crpg-cli` cannot reach sim, which matters if they need
event types.

## Deliverable

- A short ADR (or an architecture-doc update) recording the choice.
- Assign the payload/queue types explicitly to one upcoming task (T007 or a new
  event task) in `tasks/BACKLOG.md`.
- Make `README.md`, the spec role table, and the core test list agree.

## Constraints
- Do not add a new dependency.
- Respect AGENTS.md dependency direction.
- This is a human decision; do not guess.

## Agent log

- 2026-09-05 (UTC) · opencode/muse-spark · Decided with maintainer as A′ and recorded in ADR-0008: generic substrate in core, SimEvent in sim, IR types in data, hooks in rules. BACKLOG, README, and spec tables aligned in the same batch.
- 2026-09-06 (UTC) · opencode/muse-spark + E009 · Added the decided-as-A′ header so readers stop at the outcome instead of re-litigating A-vs-B; no decision change.
