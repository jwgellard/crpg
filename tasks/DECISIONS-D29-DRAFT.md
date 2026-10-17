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

**Answered 2026-10-16: (a), with the T097 contract ("approve all").**

May T097's tests use a `thread_local!` counter that exists only under `#[cfg(test)]`, against `crates/crpg-sim/AGENTS.md` rule 8 ("no clock, no threads")? It starts no thread, and `tools/lint/determinism.py` matches only `std::thread`. Source: the T097 architect and the Claude reviewer.

- **Recommend:** (a) yes, and the I package records the exception in rule 8.
- Alternative: (b) no, count encodings another way.

## D29e — cross-area transfer staging (T097)

**Answered 2026-10-16: (a), with the T097 contract ("approve all").**

Should `crates/crpg-sim/src/area.rs` `transfer_history_entity` also move to world-only staging?

- **Recommend:** (a) no: it is rare and no longer re-encodes once D1 lands.
- Alternative: (b) yes, in the same package.

## D29f — OpenCode 2.x

**Answered 2026-10-16: (a), keep 2.0.31 ("approve all").**

OpenCode was upgraded from 1.18.35 to 2.0.31 on this machine on 2026-10-16 (MODELS.md §1 item 12). Keep 2.0.31 as the pinned version?

- **Recommend:** (a) yes, and the plan and MODELS.md record 2.0.31: `oc.py` supports both, and 2.x's `--standalone` isolates runs.
- Alternative: (b) go back to 1.18.35.

## D29g — T022 promises `IllegalAction` where the code answers `NotAuthorized` (audit #66)

`tasks/T022.md:435-437` says a `Host::new` host "correctly rejects gameplay as `IllegalAction`". But admission step 5 (mapping, `:378-387`) runs before step 7 (execution, where `IllegalAction` comes from). A `new` host has no entities, so no actor maps and step 5 always answers `NotAuthorized`. The code follows the numbered steps (found by T125, PR #53).

- **Recommend:** (a) amend the §4 sentence to `NotAuthorized` and add the test T125 wrote, which pins it. The pipeline order is the deliberate part: missing, hidden and foreign must stay indistinguishable.
- Alternative: (b) special-case empty authorities to answer `IllegalAction`. That would reorder the pipeline for one case.

## D29h — should checkpoint loading reject history gaps between capture records? (audit #63)

`load_checkpoint` requires `capture_seq` to be contiguous and each record's history range to match its own events (`T022.md:614-616`). It does not require record N+1's range to start where record N's ended, and the code (`crates/crpg-server/src/checkpoint.rs:243`) rejects only overlaps. A live host never produces a gap (every history envelope after the wrap comes from a captured command, and `tick()` journals nothing), so a gap in a loaded checkpoint means omitted evidence.

- **Recommend:** (a) tighten it: add "retained records' history ranges are contiguous" to T022's validation list, and have a crpg-server fix task enforce it with a regression test.
- Alternative: (b) leave it, and say in T022 that a checkpoint is trusted input.

## D29i — two v1 host-wide caps no input can reach (audit #65, #67)

- The ingress host totals (1024 frames, 2 MiB) are exactly 8 × the per-peer caps (128 frames, 256 KiB), and at most 8 peers bind, so the per-peer cap always refuses first.
- The grant-narrowing `QueueFull` (`T022.md:549-551`) needs the resync state to miss the 16 MiB egress host total after the narrowed peer's own log is discarded. The other seven peers hold at most 14 MiB, so it needs a resync state over about 2 MiB.

Neither path has a test, because none can be written.

- **Recommend:** (a) keep the caps as defence in depth, and state in `docs/architecture/crpg-server.md` that under v1 they are not reachable, so no one later counts them as tested.
- Alternative: (b) lower the host totals below the sum of the per-peer caps so that they bind, with tests. That changes the T022 capacity table.

## D29j — `RefreshTrigger` decodes a zero `OnTick` period (audit #52)

`T015.md:378` says "OnTick period must be nonzero" in the `ResourcePool` validation paragraph. `ResourcePool` construction rejects zero, but a standalone `RefreshTrigger` decodes `{"type":"on_tick","value":0}` as `OnTick(0)` (found by T126, PR #51).

- **Recommend:** (a) reject zero in `RefreshTrigger`'s own decoder (a small crpg-rules fix task with a regression test), so no value of the type can carry the invalid period.
- Alternative: (b) state in T015 that the rule binds the pool only.

## D29k — "byte-identical" CLI apply versus its own error wording (audit #12)

`crates/crpg-cli/AGENTS.md` invariant 4 says the CLI's reference apply "must stay byte-identical\*" to testkit's `test_apply`. The CLI's unknown-payload diagnostic says `unknown intent payload`, testkit's says `unknown test payload`. T016d pins the CLI wording as black-box behaviour, and two existing CLI tests assert it. The invariant's stated reason is that both must verify the fixture to the same result, which the wording does not affect. The stray `*` points to an unfinished qualifier.

- **Recommend:** (a) clarify invariant 4: "byte-identical" covers apply semantics, the CLI's diagnostic vocabulary is its own, and the asterisk goes. Doc-only.
- Alternative: (b) make the CLI text match testkit, and change T016d and the two tests that pin the current wording.

## Agent log

- 2026-10-16 (UTC) · claude-code + manager session · Started the D29 draft with two non-blocking questions found during setup wave A0, stacked PRs and the reviewer's write access, so that they wait for a batch instead of interrupting the wave.
- 2026-10-16 (UTC) · claude-code + manager session · Added D29c, the order of priority among authors and players, which the T101 ADR needs before it fixes personas; it is a product choice, so it carries no recommendation.
- 2026-10-16 (UTC) · claude-code + manager session · Marked D29c answered and pointed to D28l.
- 2026-10-16 (UTC) · claude-code + manager handover · Added D29d, D29e and D29f from the T097 review and the OpenCode 2.x upgrade, so the user can answer them with the T097 contract.
- 2026-10-16 (UTC) · claude-code + user answers · Marked D29d, D29e and D29f answered (a) each, from the user's "approve all" on the T097 contract and its questions.
- 2026-10-17 (UTC) · claude-code (manager, Opus 5.5) + session 7 test-writer pass · Added D29g–D29k, five contract questions the per-crate test writers turned up that are not code defects, each with a recommendation, so the user can answer them in one batch.
