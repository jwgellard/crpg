## Task
Align the spec's scripting budgets and sandbox-strip list with ADR-0005, and
settle the graph-vs-Lua double budget. Human-decision task (spec edit).

## Why this is deferred

Three related gaps, one silent-hole class:

- E008 fixed §5.2 to instruction/bytecode budgets, but §12.1 still promises T1
  content running "with instruction/memory/**time** budgets"
  (`docs/CRPG_ENGINE_SPEC.md:918`) — an implementer copying §12.1 reintroduces
  the wall-clock abort E008 removed.
- The spec bans `loadstring` (`docs/CRPG_ENGINE_SPEC.md:580`) — the Lua 5.1
  name. ADR-0005 proved `mlua`'s `StdLib` gating does not cover the base
  library and `load`/`loadfile`/`dofile` must be nilled by hand
  (`docs/adr/0005-lua-sandbox-spike.md:52-74`); spec T3 (`:1587`) and
  `tasks/T003.md:29-30` repeat the omission. Following the spec literally
  leaves a sandbox hole.
- Lua is called synchronously from IR (`CallScript -> next`,
  `docs/CRPG_ENGINE_SPEC.md:505,521`) under two budgets at once: the graph's
  node-count + instruction budget (`:529`, per-trigger) and Lua's
  instruction hook + memory ceiling (`:583`). Which budget aborts a looping
  `CallScript`, and how nested accounting works, is undefined. ADR-0005
  deferred tuning (`:111-114`) but the spec holds no placeholder values.

## Decision to make
- Replace "time budgets" with the E008 vocabulary; list the exact globals to
  strip (`load`, `loadfile`, `dofile`, plus `loadstring` as belt-and-braces);
  define nested-budget semantics (inner abort propagates as outer node
  failure? counted twice?) with placeholder numbers marked TBD.

## Deliverable
- Edits to `docs/CRPG_ENGINE_SPEC.md` §§5.2, 5.4, 12.1 and spec T3, each with
  a dated inline note. No source changes.

## Constraints
- Doc-only; no `crpg-*` sources. Human sign-off; this text becomes the
  `crpg-script` sandbox acceptance basis.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: groups the E008 near-miss, the ADR-0005 strip gap, and the undefined nested budget.
