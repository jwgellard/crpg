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

## Resolution — 2026-09-30 (T032, POST-T018 D07)

- Spec §15.1's embedded `crpg-rules` contract is now labelled an
  **illustrative, non-authoritative sketch**, pointing to the root
  `AGENTS.md`, each crate's `AGENTS.md`, `docs/architecture/`, `tasks/`, and
  the enforced `tools/lint/deps.py` / `tools/lint/determinism.py`. The example
  itself is not rewritten, so it cannot silently broaden or restate the live
  rules.
- `tasks/T004.md` is backfilled as a one-page retrospective from the original
  bootstrap and CI commits (`06646f2`, `751c937`, `621769c`), marking
  2026-09-03 test/CI results as unknown.

- 2026-09-30 (UTC) · claude-code + T032 E021 resolution · Recorded the illustrative-only label and the evidence-based T004 backfill that close this hygiene item.
