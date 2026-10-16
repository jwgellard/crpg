# Decision D29 — draft

Status: **Draft; not yet put to the user.** Non-blocking questions collected
by manager sessions under `docs/AGENT_TEAM_PLAN.md` §6, each with a
recommendation. Nothing here is decided until the user answers.

## D29a — stacked PRs for contract-then-implementation chains

GitHub stacked pull requests became generally available on 2026-10-06
(R6.R1, checked). Branch protection applies to every PR in a stack. Today a
C package's PR must merge before its I package's PR can open against
`master`, and the strict ruleset makes every merge stale the rest.

- **Recommend:** trial a stack once, on T024: the contract PR (#34) at the
  bottom and the T024 implementation PR on top. Setup and documentation work
  stays one PR per wave. Judge it at the next gate by merges per day and by
  rebases you had to do.
- Alternative: keep waiting for each merge.

## D29b — the `reviewer` agent can write

`reviewer` is meant never to edit, and its `tools:` line lists only `Read,
Grep, Glob, Bash`. `memory: project` adds `Write` and `Edit` anyway, as the
A0-1 spawn check showed on 2026-10-16, so that the agent can keep its memory
file.

- **Recommend:** keep the memory and add a `PreToolUse` rule in A0-6 that
  refuses `reviewer` any write outside `.claude/agent-memory/`.
- Alternative: drop `memory: project`, so the reviewer starts every review
  without notes from earlier ones.

## Agent log

- 2026-10-16 (UTC) · claude-code + manager session · Started the D29 draft with two non-blocking questions found during setup wave A0, stacked PRs and the reviewer's write access, so that they wait for a batch instead of interrupting the wave.
