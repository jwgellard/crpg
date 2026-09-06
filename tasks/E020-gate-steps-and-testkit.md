## Task
Close the integration-gate gap and give the testkit cycle an owner.
Human-decision task (CI planning + task assignment).

## Why this is deferred

- The §15.4 gate lists 13 steps (`docs/CRPG_ENGINE_SPEC.md:1090-1105`); CI
  implements 6 (fmt, clippy, test, deny, two lints, self-tests —
  `.github/workflows/ci.yml:32-97`). Steps 7–13 (schema drift, `crpgc
  validate`, golden replay, save/load equivalence, perf gate, client+editor+
  server build, smoke test) have no jobs, and T09's done-when requires step 9
  (`:1639`) — it can never pass. The workflow plan's two-layer CI, merge
  queue, and self-hosted runner (`docs/AGENTIC_WORKFLOW_PLAN.md:264-272`)
  are likewise absent.
- E005's sim→testkit→sim dev-dependency cycle is lint-green (the cycle check
  covers runtime edges only, `tools/lint/deps.py:148-162`) but cargo-rejected,
  with no owning task (`tasks/E005-testkit-cycle.md:6-13`); T008/T009 are the
  first tasks that will trip it. Related: T08/T09/T16/T18 touch 2–3 crates
  each under a one-task-one-crate rule (see E004), and `deny.toml:79` still
  says `yanked="warn"` against S001's `deny`.

## Decision to make
- Sequence gate steps 7–13 into tasked CI work (what blocks T009 vs what
  rides the slow layer); assign the testkit-ownership/cycle fix; confirm the
  multi-crate splits (E004) and the yanked policy (S001) so T008 unblocks.

## Deliverable
- BACKLOG rows for the gate steps + testkit task + E004 splits; CI
  follow-up scope (no workflow edits in this task). No source changes.

## Constraints
- Planning-only. Human sign-off; T009's acceptance criterion depends on it.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: a 13-step gate with 6 steps built, plus an ownerless dependency cycle.
