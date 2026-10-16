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

## D29c — who comes first: primary authors and players

**Answered 2026-10-16: module authors first, agents equal. Recorded as D28l
in `tasks/DECISIONS-2026-10-16.md`; the question stays here for the record.**

M6-R's personas, and every usability trade-off after them, need an order of
priority among: you or a small team making your own game (experts; speed and
power first), a community of module authors (hobbyists; onboarding,
templates and forgiving errors first), and AI agents (command shape and
error messages first). Players split the same way: people playing your
campaign, or people finding and installing community modules.

- **Needed by:** the T101 ADR, before it fixes the personas.
- **Recommend:** no recommendation; this is a product choice. Whatever the
  order, agents stay a first-class audience (D28j).

## D29d — a test-only encoding counter in `crpg-sim` (T097)

May T097's tests use a `thread_local!` counter that exists only under `#[cfg(test)]`, against `crates/crpg-sim/AGENTS.md` rule 8 ("no clock, no threads")? It starts no thread, and `tools/lint/determinism.py` matches only `std::thread`. Source: the T097 architect and the Claude reviewer.

- **Recommend:** (a) yes, and the I package records the exception in rule 8.
- Alternative: (b) no, count encodings another way.

## D29e — cross-area transfer staging (T097)

Should `crates/crpg-sim/src/area.rs` `transfer_history_entity` also move to world-only staging?

- **Recommend:** (a) no: it is rare and no longer re-encodes once D1 lands.
- Alternative: (b) yes, in the same package.

## D29f — OpenCode 2.x

OpenCode was upgraded from 1.18.35 to 2.0.31 on this machine on 2026-10-16 (MODELS.md §1 item 12). Keep 2.0.31 as the pinned version?

- **Recommend:** (a) yes, and the plan and MODELS.md record 2.0.31: `oc.py` supports both, and 2.x's `--standalone` isolates runs.
- Alternative: (b) go back to 1.18.35.

## Agent log

- 2026-10-16 (UTC) · claude-code + manager session · Started the D29 draft with two non-blocking questions found during setup wave A0, stacked PRs and the reviewer's write access, so that they wait for a batch instead of interrupting the wave.
- 2026-10-16 (UTC) · claude-code + manager session · Added D29c, the order of priority among authors and players, which the T101 ADR needs before it fixes personas; it is a product choice, so it carries no recommendation.
- 2026-10-16 (UTC) · claude-code + manager session · Marked D29c answered and pointed to D28l.
- 2026-10-16 (UTC) · claude-code + manager handover · Added D29d, D29e and D29f from the T097 review and the OpenCode 2.x upgrade, so the user can answer them with the T097 contract.
