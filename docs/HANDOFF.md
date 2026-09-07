# Handoff — how to continue without this conversation

## 1. Commit the documents (do this first)

```
docs/
  CRPG_ENGINE_SPEC.md          the architecture and roadmap
  AGENTIC_WORKFLOW_PLAN.md     tooling, cost tiers, agent process
  DAY_ONE.md                   the setup log (keep it, it dates your decisions)
  PROJECT_STATE.md             the living file — see below
  adr/                         numbered decision records
tasks/
  BACKLOG.md                   the task list
  T0NN.md                      individual task files
AGENTS.md                      the rules agents load every session
```

Once these are committed, no conversation needs to be preserved. Everything a future session needs is in the repo, and a future session can read the two or three files that matter instead of a 40-message history.

## 2. `docs/PROJECT_STATE.md` — the only file you update constantly

Keep it short. Thirty lines, not three hundred. Its job is to answer "where am I?" in ten seconds.

```markdown
# Project state

Updated: 2026-09-03

## Phase
Phase 1 — core skeleton and test harness.

## Done
- T004 workspace, 15 stub crates, CI green on Linux + Windows
- T005 dependency-direction lint

## In progress
- T005b determinism lint (bans HashMap iteration + floats in rules crates)

## Next three
- T006 crpg-core: EntityId, Fx16_16, DeterministicRng
- T007 crpg-sim: World and ComponentStore
- T008 state_hash + fixed-step tick loop

## Decisions made
- ADR-0001 Godot consumed as pinned dependency, not forked
- ADR-0002 Rust for everything below the presentation layer
- Godot pinned at 4.6.x  (record your exact version; live pin is now 4.7.2 — see `docs/PROJECT_STATE.md`)
- Toolchain pinned at rustc 1.XX.0 (live pin is now 1.98.0)

## Open questions
- Whether to buy a subscription (decide end of week 1)

## Known problems
- (nothing yet)
```

Update it when you finish a task. It takes one minute and it is what makes every future session cheap.

## 3. The cold-start prompt

When you open a new chat with any model, paste this. It is the whole handoff.

> I'm building a purpose-built CRPG engine and campaign editor — a Rust simulation core with no engine dependency, a headless authoritative server, and Godot used only as a presentation host for the client and editor. I'm a solo developer using AI agents, working on Windows.
>
> Per [ADR-0012](adr/0012-windows-primary-platform.md), `x86_64-pc-windows-msvc` is primary for development, product, release gates, and behavioural baselines: client, editor, embedded single-player server, dedicated server, and CLI. `x86_64-unknown-linux-gnu` is fully supported for dedicated servers, headless CLI/tooling, server-side extensibility, and testing; target-specific failures are defects. Linux GUI builds are not promised and Linux headless support does not depend on Godot.
>
> Windows single-player embeds the same platform-neutral authoritative server that Windows and Linux dedicated processes host. In-memory transport preserves the client/server authority boundary; clients never mutate authoritative state directly. Replay determinism is exact-build, not cross-platform lockstep: each target must reproduce its own independently generated scoped golden, never the other target's hashes.
>
> Attached: `PROJECT_STATE.md` and `docs/CRPG_ENGINE_SPEC.md`.
>
> Read the state file first. I want help with: [one specific thing].

Attach the state file always. Attach the spec only when the question is architectural. For anything routine, the state file plus the relevant crate's `AGENTS.md` is enough, and it is a fraction of the context.

## 4. Which conversations to have where

**In the terminal, with a coding agent.** Anything with a task file. This is most of your work and it should never touch a chat window.

**In a fresh chat, short session.** Design questions the spec doesn't answer, debugging something you can't explain, reviewing a subsystem you've lost confidence in. Bring the failing test and the relevant `AGENTS.md`, not the repository.

**Nowhere.** "How's the project going", "what should I do next" — the state file already answers that. If it doesn't, the state file is the thing to fix.

## 5. Session hygiene

- **One session per task.** Close it when the task is done. Never resume yesterday's.
- **Start a new chat when the topic changes.** Setup questions and rules-design questions do not belong in the same thread.
- **When a session gets long, write down what you learned and start again.** The output of a good long session is an ADR or a task file, not the session itself.
- **If you find yourself re-explaining the project, your state file is out of date.** Fix the file rather than the explanation.

## 6. When to come back to a long conversation

Rarely, and deliberately: reviewing a whole phase before starting the next one, or reworking the roadmap after something significant changes. Those are worth a fresh, focused session with the spec attached — not a continuation of an old one.

## 7. Right now

**Current handoff (2026-09-07):** T009a and T009c are merged on `master`
(commit `bb9a702`); all required gates passed on native Windows/MSVC and
genuine Linux/GNU in WSL Ubuntu 24.04 and the final audit is complete.
T009b's thin `crpgc replay` wrapper is next. Follow `PROJECT_STATE.md`
and the [T009c completion record](../tasks/T009c.md).
the bootstrap checklist below is historical, not a
direction to commit the current working tree or repeat completed setup.

1. `git add docs/ tasks/ AGENTS.md` and commit.
2. Write `PROJECT_STATE.md` with your real Godot and rustc versions in it.
3. Write ADR-0001 and ADR-0002 — one page each, why you're not forking Godot and why Rust.
4. Close this conversation.
5. Open a terminal, write `tasks/T005b.md` for the determinism lint, and run the agent.

Step 5 is the actual project. Everything above is bookkeeping that makes step 5 repeatable a thousand times.

---

## Agent log

- 2026-09-05 (UTC) · opencode/muse-spark + stale-pin hygiene · Annotated the template pin lines with the live Godot 4.7.2 / rustc 1.98.0 values; template history otherwise untouched.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c documentation alignment · Made the cold-start prompt carry ADR-0012's Windows-primary/Linux-supported policy and unchanged embedded authority boundary. Distinguished the historical setup checklist from uncommitted T009a and T009c's pending native verification.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c verification status · Updated the current handoff to the reported passing native Windows and genuine WSL Ubuntu gates, linking the completion record. Kept T009c ahead of T009b awaiting review/merge, T009a uncommitted, and final audit distinct from native verification.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c final audit · Updated the handoff to the reported completed final audit and working-tree implementation/verification completion. Review/merge remains outstanding, T009a is also uncommitted, and T009c stays ahead of T009b.
- 2026-09-07 (UTC) · opencode/big-pickle + T009a/T009c merged · Updated the current handoff to the merged state; T009b is next.
