# Lessons

This file keeps one line per lesson with ID, evidence, and promotion.

| ID | Lesson | Evidence | Promotion |
|---|---|---|---|
| L1 | When batch PRs are stacked, retarget the upper PR's base to `master` as soon as the lower PR merges, and before the upper PR merges; GitHub merges a stacked PR into its old base branch, which is no longer headed for `master`. | PR #65 (session 15) was merged into `mgr/session-14-batch` nine seconds after #64 merged into `master`, which stranded session 15's commits; PR #66 re-landed them on 2026-10-23. | `not yet promoted` |

## Agent log

- 2026-10-23 (UTC) · opencode-go/muse-spark-1.3-contributor + A0-11 · Started this file with the L1 stacked-PR retarget lesson, so the session-15 stranding is recorded where briefs can recall it.
