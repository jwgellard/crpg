# Decision D38

Status: **Answered 2026-10-25 (the user, session 23)**.

## D38 — narrow `opencode-go/beam-free` further: new files only, and its reports are not evidence?

**Answered 2026-10-25: agreed.**

Tightens D37b. Beam keeps work that writes **new files**: fact sheets and
first blind drafts, each checked afterwards by scripts and a stronger
reviewer (the `architect` for contracts). Beam no longer takes work that
edits, moves or condenses **existing** text, even doc-only work such as the
A0-8 `AGENTS.md` trims. The remaining A0-8 crates go to an O1 model when
quota returns, and the manager runs `crpg-slots/manager/lost.py` on every
trim whatever the model (A0-16 makes it a tool). A Beam report or review
("no information lost", "ready-for-pr") is never evidence; only scripts and
Claude review count.

Evidence (session 23, lesson L8): five new-file runs (two fact sheets,
three drafts) passed on attempt 1; all four A0-8 trims needed fixes. Two
deleted prior agent-log entries, two dropped text (6 and 14 blocks) while
reporting "no information lost", and others left heredoc remnants or a
corrupted `·`. Earlier: T024 implementation (D37b) and L7 (review).

## Agent log

- 2026-10-25 (UTC) · claude-code/opus-5.5 (manager) + D38 · Recorded the user's agreement to narrow Beam to new-file work with its self-reports never counted as evidence, from the session 23 A0-8 trim failures.
