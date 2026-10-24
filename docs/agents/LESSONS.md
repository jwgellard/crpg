# Lessons

This file keeps one line per lesson with ID, evidence, and promotion.

| ID | Lesson | Evidence | Promotion |
|---|---|---|---|
| L1 | When batch PRs are stacked, retarget the upper PR's base to `master` as soon as the lower PR merges, and before the upper PR merges; GitHub merges a stacked PR into its old base branch, which is no longer headed for `master`. | PR #65 (session 15) was merged into `mgr/session-14-batch` nine seconds after #64 merged into `master`, which stranded session 15's commits; PR #66 re-landed them on 2026-10-23. | `not yet promoted` |
| L2 | `oc.py wave` picked Rust slot 1 while a Claude worker held it (no lock), so T137-draft-a saw T136b's uncommitted shapes as code (session 17). | A subagent takes no `oc.py` slot lock, so lock-only slot choice saw a slot full of another task's work as free. | `promoted: A0-12 guard in oc.py` |
| L3 | Two-keys test briefs require a per-test mutation proof: break the guarded code in a scratch edit, confirm the test fails, restore `src/` byte-identical, and list the mutation per test; a test with no failing mutation is reported, not kept silently. | T136b-tests needed a review-fix round for tests passing for the wrong reason; T138a-fix used mutation proofs and passed first time (session 18). | `promoted: TEMPLATES.md §1` |
| L4 | A brief that adds a rule is self-checked against the existing tests and fixtures it will meet before dispatch, naming every code path the rule must cover (e.g. `--dry-run`). | A0-12 attempt 1 (the rule flagged the shared fixtures while the brief forbade the needed test edits) and attempt 3 (dry-run missed) (session 18). | `promoted: TEMPLATES.md §1` |

## Agent log

- 2026-10-23 (UTC) · opencode-go/muse-spark-1.3-contributor + A0-11 · Started this file with the L1 stacked-PR retarget lesson, so the session-15 stranding is recorded where briefs can recall it.
- 2026-10-24 (UTC) · opencode-go/deepseek-v4.1-flash + A0-12 · Recorded L2, the unlocked slot that let T137-draft-a read T136b's uncommitted work, so slot choice is checked against the work a slot holds.
- 2026-10-24 (UTC) · opencode-go/muse-spark-1.3-contributor + S18-bookkeeping · Appended L3 (two-keys mutation proof) and L4 (brief self-check), both promoted to `TEMPLATES.md` §1, so the session 18 template rules are recalled where briefs can find them.
