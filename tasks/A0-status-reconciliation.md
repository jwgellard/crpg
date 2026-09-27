# A0 — Status and review-record reconciliation (documentation only)

## Status and purpose

**Specified; implementation pending.** Work package A0 of the
[project gap remediation plan](../docs/reviews/REMEDIATION_PLAN_2026-09-27.md)
(§4, gap G8). Reconciles status and review records so the same next
milestone and merge state appear in every active status summary, outstanding
findings have an owner, and no "all done" ambiguity remains.

This file is the implementation contract. Documentation only: no source,
schema, content, replay, golden, manifest, workflow, or lint change is
authorized here. Do not replace prior agent logs, do not turn reported
verification into a new claim, and do not relocate code.

## Baseline (verified against git, 2026-09-27)

- `master` is `76a48da` (2026-09-26, PR #13, T015 dice/resolution/resource).
  T015 is merged; every status file still calling it next/future is stale.
- Uncommitted working tree holds T016a (content), T016b (sim + ADR-0013),
  T016c (replay + native goldens), T016d (CLI), and T016e (A1 invariant
  repair, implemented and dual-native verified 2026-09-27, unmerged).
- T016a/c/d headers say "Specified; implementation and verification
  pending" while their own completion records document implementation and
  verification. T016b's header already matches its record.
- `crpg-testkit/AGENTS.md` says the crate "never ships in a binary" while
  `crpgc` (T009b, T016d) consumes its harness/replay library at runtime.
- The remediation plan has no progress ledger section yet (§12 covers
  verification rules, not per-package status).

## Scope

Documentation-only edits in exactly these files (append-only attribution on
every file; never rewrite or delete a prior entry):

- `tasks/BACKLOG.md` — T015 `open` → `done` (merged 2026-09-26, PR #13,
  `76a48da`); T016 `open` → `in progress` with a one-line child-state
  pointer (T016a–d + T016e implemented+verified uncommitted; A2 lifecycle +
  merge outstanding); throughput `2026-09-26` → 28 for T015 (unmerged work
  is never counted); agent-log append.
- `docs/PROJECT_STATE.md` — `Updated` 2026-09-27; Phase 3 wording to
  "T015 landed; T016 in progress"; branch state through `76a48da` plus the
  uncommitted T016a–e tree sketch; Done gains the T015 bullet (merged work
  only — T016a–e stay out); Next becomes T016 (A2 + merge) then T017;
  platform-history paragraph extended past `18145d4`; agent-log append.
- `README.md` — status block: Phase 2 → Phase 3, T015-next → T015-merged /
  T016-in-progress with the uncommitted-tree and outstanding-work qualifiers;
  agent-log append. Nothing else in the file.
- `crates/crpg-testkit/AGENTS.md` — reconcile the two "never ships" spots
  (Purpose line and invariant 5) to the as-built split: tests-only adapters
  never ship; the harness/replay library surface is shared with `crpgc` at
  runtime since T009b. No code, dependency, or ownership move; agent-log
  append.
- `docs/reviews/REMEDIATION_PLAN_2026-09-27.md` — add `## 14. Progress
  ledger` before the agent log: per-package table (package, task file,
  owning crate, specified/implemented/verified/merged states, evidence,
  blockers) with merge status kept separate from implementation status;
  current rows A1 (T016e, implemented+verified dual-native, unmerged),
  A0 (this task), A2/B/C/D/E/F/P (specified, pending, dependencies noted);
  agent-log append.
- `tasks/T016.md` — Completion record `Pending` → in-progress pointer
  (child states with links, T016e repair, A2 + merge remaining); agent-log
  append. The milestone contract above it is untouched.
- `tasks/T016a.md`, `tasks/T016c.md`, `tasks/T016d.md` — Status bold line
  only: "Specified; implementation and verification pending" → "Implemented
  and verified in the working tree per the completion record below; no
  commit, push, or PR created" (T016c keeps its dependency-approval clause
  with exercised-tense); agent-log append each. Completion records verbatim.
  `tasks/T016b.md` already matches; do not touch it.

Out of scope: any `.rs`/`.json`/`.replay`/`.golden`/`.toml`/`.lock`/`.yml`/
`.py` change; any rebaseline, rewording of a completion record's evidence,
or merge/PR action.

## Acceptance

- T015 reads `done`/landed/merged in BACKLOG, PROJECT_STATE, and README
  with the same revision (PR #13, `76a48da`, 2026-09-26).
- T016 reads `in progress` with the same child-state pointer and the same
  outstanding work (A2 lifecycle decision, milestone merge) everywhere.
- No status file claims T016a–e merged, complete, or verified beyond its
  task record; "specified / implemented / verified / merged" stay distinct.
- Testkit wording no longer contradicts the CLI's runtime consumption, and
  no code moved.
- The plan carries a dated ledger row for A1 and A0 with evidence links.

## Verification (documentation task)

```text
git diff --check
git diff --name-only
```

Every hunk is in the eleven listed files; every edited file carries its
append-only attribution; `git status` shows no source change. No cargo gates
are required for `.md`-only edits. Record the command results honestly.

## Completion record

Implemented 2026-09-27 in the working tree; no commit, push, or PR.
`git diff --check` clean; `git diff --name-only` shows only the eleven
listed `.md`/task files (all other tree changes are the pre-existing
T016a–e implementation work). BACKLOG (T015 done PR #13, T016 in progress,
throughput 28), PROJECT_STATE (Updated 2026-09-27, Phase 3 T015-landed,
branch through `76a48da` plus the uncommitted tree, T015 Done bullet, T016
Next), README (Phase 3, T015-merged/T016-in-progress), testkit AGENTS
(library/adapters split), plan §14 ledger, and T016-family headers all
agree; no file claims a merge and no completion record was rewritten.

## Agent log

- 2026-09-27 (UTC) · opencode/muse-spark + A0 executable task · Scoped the documentation-only reconciliation against git truth (T015 merged PR #13; T016a–e uncommitted; testkit wording split) with exact per-file edits, ledger shape, and md-only verification; no source, evidence, or merge claim is authorized.
- 2026-09-27 (UTC) · opencode/muse-spark + A0 implementation · Applied the eleven-file reconciliation with diff-check verification; all status summaries agree on T015-merged/T016-in-progress with the A2 decision and milestone merge outstanding.

(End of file - total 110 lines)
