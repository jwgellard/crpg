# Lessons

This file keeps one line per lesson with ID, evidence, and promotion.

| ID | Lesson | Evidence | Promotion |
|---|---|---|---|
| L1 | When batch PRs are stacked, retarget the upper PR's base to `master` as soon as the lower PR merges, and before the upper PR merges; GitHub merges a stacked PR into its old base branch, which is no longer headed for `master`. | PR #65 (session 15) was merged into `mgr/session-14-batch` nine seconds after #64 merged into `master`, which stranded session 15's commits; PR #66 re-landed them on 2026-10-23. | `not yet promoted` |
| L2 | `oc.py wave` picked Rust slot 1 while a Claude worker held it (no lock), so T137-draft-a saw T136b's uncommitted shapes as code (session 17). | A subagent takes no `oc.py` slot lock, so lock-only slot choice saw a slot full of another task's work as free. | `promoted: A0-12 guard in oc.py` |
| L3 | Two-keys test briefs require a per-test mutation proof: break the guarded code in a scratch edit, confirm the test fails, restore `src/` byte-identical, and list the mutation per test; a test with no failing mutation is reported, not kept silently. | T136b-tests needed a review-fix round for tests passing for the wrong reason; T138a-fix used mutation proofs and passed first time (session 18). | `promoted: TEMPLATES.md §1` |
| L4 | A brief that adds a rule is self-checked against the existing tests and fixtures it will meet before dispatch, naming every code path the rule must cover (e.g. `--dry-run`). | A0-12 attempt 1 (the rule flagged the shared fixtures while the brief forbade the needed test edits) and attempt 3 (dry-run missed) (session 18). | `promoted: TEMPLATES.md §1` |
| L5 | Parallel `oc.py ask` runs on one model share an attempt number and write one `.jsonl`/report, so three reviews return one (session 21, T024 reviews attempt 15). | Parallel `oc.py ask` runs collide: they get the same attempt number and write one jsonl (session 21, T024). | `promoted: oc.py attempt claim (A0-13)` |
| L6 | `verify_task.py` passed a test file of 31 empty `fn tN() {}` stubs that the worker reported as "31 passed" (session 21, T024 attempt 1 on Beam). | Beam may report empty test stubs as passing: check named tests and assertion counts before trusting a pass (session 21, T024 attempt 1 on Beam). | `promoted: verify_task.py asserts check (A0-14)` |
| L7 | Beam's first-pass review of T024 said ready-for-pr; the Claude core review then found six untested contract behaviours (items 5–10 of `.agents/briefs/T024-tests-notes-3.md`). | A Beam ready-for-pr is not test-strength evidence (session 22, T024 Claude core review, Sonnet 5.5, ~216k tokens, VERDICT fix). | `not yet promoted` |
| L8 | Beam doc trims delete or reorder prior agent-log entries, drop blocks while reporting "no info lost", and leave heredoc remnants (`EOF`, `PY`, `"`) or a corrupted `·`; docsig does not catch a deleted prior entry. | A0-8-sim attempt 2, A0-8-data attempt 1, A0-8-net attempt 1, A0-8-testkit, T024h-facts. | `docsig part promoted (A0-15)`, A0-16 (`lost.py`); meanwhile briefs say "never delete, move or reorder a dated agent-log line; use the file edit tool, not heredocs", and the manager runs `lost.py` on every trim. |

## Agent log

- 2026-10-23 (UTC) · opencode-go/muse-spark-1.3-contributor + A0-11 · Started this file with the L1 stacked-PR retarget lesson, so the session-15 stranding is recorded where briefs can recall it.
- 2026-10-24 (UTC) · opencode-go/deepseek-v4.1-flash + A0-12 · Recorded L2, the unlocked slot that let T137-draft-a read T136b's uncommitted work, so slot choice is checked against the work a slot holds.
- 2026-10-24 (UTC) · opencode-go/muse-spark-1.3-contributor + S18-bookkeeping · Appended L3 (two-keys mutation proof) and L4 (brief self-check), both promoted to `TEMPLATES.md` §1, so the session 18 template rules are recalled where briefs can find them.
- 2026-10-25 (UTC) · opencode-go/beam-free + S21-bookkeeping · Appended L5 (parallel oc.py ask attempt collision) and L6 (empty test stubs reported as passing), both proposed as A0 tooling follow-ups.
- 2026-10-25 (UTC) · opencode-go/beam-free + S22-bookkeeping · Appended L7 (Beam ready-for-pr is not test-strength evidence), from the T024 Claude core review that found six untested behaviours.
- 2026-10-25 (UTC) · opencode-go/beam-free + S23-bookkeeping · Appended L8 (Beam doc trims delete/reorder agent-log entries; docsig does not catch a deleted prior entry).
- 2026-10-30 (UTC) · opencode-go/muse-spark-1.3-contributor + A0-14 · Promoted L6 to the landed `verify_task.py` asserts check so the empty-stub lesson points at its enforcement.
- 2026-10-30 (UTC) · opencode-go/muse-spark-1.3-contributor + A0-14 attempt 2 · Kept L6 promoted: refined the `asserts` check to accept same-file asserting helpers (transitively), `macro_rules!` delegates, and non-empty `proptest!` properties after the attempt-1 check flagged 26 legitimate tests.
- 2026-10-30 (UTC) · opencode-go/deepseek-v4.1-flash + A0-13 · Promoted L5 to the landed `oc.py` attempt claim, so the parallel-attempt-collision lesson points at its enforcement.
- 2026-10-30 (UTC) · opencode-go/deepseek-v4.1-flash + A0-15 · Marked L8's docsig part promoted: `verify_task.py` now keeps prior entries in order and catches bad separators and heredoc debris, while A0-16 still owns lost-text detection.
