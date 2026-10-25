# Decision D37

Status: **Answered 2026-10-25 (the user, session 22)**.

## D37a — keep the T137-gaps self-target Opposed fold pinned by the crate-private plan test?

**Answered 2026-10-25: (a).**

Keep the T137-gaps self-target Opposed fold pinned by the crate-private plan
test; no new public surface.

## D37b — narrow `opencode-go/beam-free` to doc-only work, fact sheets and first-pass review?

**Answered 2026-10-25: (a).**

Narrow `opencode-go/beam-free` to doc-only work, fact sheets and first-pass
review; not core implementation, not tests.

Evidence: T024 (attempt 1 rejected, attempt 2 hit its step limit, a Sonnet fix
rewrote cases 23–31 and the oracle) and lesson L7 (Beam's ready-for-pr review
missed six untested behaviours the Claude core review found).

## D37c — routing while Go is over quota?

**Answered 2026-10-25.**

For implementation and tests, try the free OpenCode models first
(`opencode/deepseek-v4.1-flash-free`, then other free Zen models), then the
Claude `implementer` (Sonnet 5.5); never Beam.

## Agent log

- 2026-10-25 (UTC) · opencode-go/beam-free + D37-record · Recorded the user's 2026-10-25 answers: D37a keeps the T137-gaps self-target Opposed fold crate-private, D37b narrows Beam to doc-only work, fact sheets and first-pass review, and D37c routes implementation and tests to free Zen models first then the Claude implementer, never Beam.