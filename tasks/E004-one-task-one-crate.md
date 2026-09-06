## Task
Resolve the one-task-one-crate rule vs multi-crate tasks T008/T009/T011/T016/T018.
Human-decision task.

> **Decided 2026-09-06 as Option A** (split into per-crate sub-tasks). T008
> is split concretely below; T009/T011/T016/T018 split at specification time
> (Stage 2), following the T008 precedent — splitting them now, with no task
> files and only guessed scopes, would be fiction. The `crpgc run`
> thin-wrapper moves to T013 (`crpg-cli`).

## Why this is deferred

AGENTS.md mandates "one task = one crate" (reviewer/editor constraint). But the
roadmap in `docs/CRPG_ENGINE_SPEC.md` / `tasks/BACKLOG.md` lists tasks that span
multiple crates:

- T008 (sim) — likely sim + tests + possibly dev tools.
- T009 (script) — script + testkit? + tests.
- T011 — possibly script+sim coordination.
- T016, T018 — TBD (persist/net adjacent).

If a task touches multiple crates it violates the rule and would fail
`tools/lint/deps.py` or peer review.

## Decision to make
- Option A: Split each multi-crate task into per-crate sub-tasks (e.g. T009a
  script core, T009b testkit integration). Preferred, keeps the rule intact.
- Option B: Relax the rule to allow a single task to touch multiple crates when
  they are tightly coupled, with an explicit carve-out.
- Option C: Keep the rule strict; re-scope each listed task so it is single-crate.

## Deliverable
- Update `tasks/BACKLOG.md` (and the spec task list) to split or re-scope.
- If the rule changes, edit `AGENTS.md` accordingly and keep in sync.

## Constraints
- Human decision; do not guess. Only the maintainer can relax a non-negotiable.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + maintainer sign-off · Decided Option A: T008 splits into T008a (`crpg-sim`: `state_hash`, tick loop, `Timeline` advance) and T008b (`crpg-testkit`: hash-sequence harness + golden convention), with the `crpgc run` wrapper transferred to T013. Later multi-crate tasks split at Stage 2 on this precedent; the rule itself is unchanged, so `AGENTS.md` needs no edit.
