# Lessons

This file keeps one line per lesson with ID, evidence, and promotion.

| ID | Lesson | Evidence | Promotion |
|---|---|---|---|
| L1 | When batch PRs are stacked, retarget the upper PR's base to `master` as soon as the lower PR merges, and before the upper PR merges; GitHub merges a stacked PR into its old base branch, which is no longer headed for `master`. | PR #65 (session 15) was merged into `mgr/session-14-batch` nine seconds after #64 merged into `master`, which stranded session 15's commits; PR #66 re-landed them on 2026-10-23. | `not yet promoted` |
| L2 | `oc.py wave` picked Rust slot 1 while a Claude worker held it (no lock), so T137-draft-a saw T136b's uncommitted shapes as code (session 17). | A subagent takes no `oc.py` slot lock, so lock-only slot choice saw a slot full of another task's work as free. | `promoted: A0-12 guard in oc.py` |

## Agent log

- 2026-10-23 (UTC) · opencode-go/muse-spark-1.3-contributor + A0-11 · Started this file with the L1 stacked-PR retarget lesson, so the session-15 stranding is recorded where briefs can recall it.
- 2026-10-24 (UTC) · opencode-go/deepseek-v4.1-flash + A0-12 · Recorded L2, the unlocked slot that let T137-draft-a read T136b's uncommitted work, so slot choice is checked against the work a slot holds.
