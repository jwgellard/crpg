## Task
Contain the in-spec `crpg-rules` contract example and backfill the missing
T004 file. Human-decision task (small hygiene).

## Why this is deferred

- Spec §15.1 embeds a 19-line `crpg-rules` agent contract
  (`docs/CRPG_ENGINE_SPEC.md:1031-1050`) whose done-gate (`test -p` three
  crates incl. nonexistent `rules_golden`) matches neither the real gate
  (`cargo fmt --all`, `clippy --all-targets -D warnings`, both lints,
  self-tests, arch doc, log entry — root `AGENTS.md:42-49`,
  `crates/crpg-core/AGENTS.md:162-171`) nor the enforced dependency table
  (example conflates workspace and external deps; misses rename/target
  rules). An agent copying the example builds the wrong contract; the real
  234-line core contract shows what these files actually cost.
- `tasks/BACKLOG.md:36` records T004 done with no `tasks/T004.md` on disk —
  the only done task without a file, against §15.5 and the workflow plan's
  "keep tasks in `tasks/`".

## Decision to make
- Mark the embedded example illustrative-only with a pointer to the
  authoritative sources (root `AGENTS.md`, `deps.py`, the finished core
  contract); decide whether T004 gets a retroactive one-page file or a
  BACKLOG annotation.

## Deliverable
- Small edits to `docs/CRPG_ENGINE_SPEC.md` (§15.1) + T004 file or BACKLOG
  note, each with a dated inline note. No source changes.

## Constraints
- Doc-only. Human sign-off; trivial content, precedent value (examples in
  the spec must not outrank the lints).

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: a stale example gate plus the one fileless done task.
