# Agentic Development Workflow & Cost Plan

**Companion to:** `CRPG_ENGINE_SPEC.md`
**Constraint:** solo developer, one paid AI subscription maximum, free GitHub Copilot via the Student Developer Pack, two consumer machines.
**Hardware:** Lenovo LOQ (RTX 4060 Laptop, 8 GB VRAM, 24 GB RAM) and a desktop (GTX 1080, 8 GB VRAM, 16 GB RAM).

**Platform policy:** [ADR-0012](adr/0012-windows-primary-platform.md) makes
`x86_64-pc-windows-msvc` primary for development, product, release gates, and
behavioural baselines, including client/editor, embedded single-player server,
dedicated server, and CLI. `x86_64-unknown-linux-gnu` is fully supported for
dedicated server, headless CLI/tooling, server-side extensibility, and tests;
target-specific failures are defects, not best effort. Linux GUI builds are
not promised and Linux headless support must not depend on Godot.

> **Pricing volatility warning.** The AI tooling market moved violently through 2026, and the free agentic tier took most of the damage. Alibaba cut Qwen Code's free OAuth quota tenfold and then closed it entirely on 15 April. GitHub paused Copilot signups in April and switched to usage-based AI Credits on 1 June; the free student tier came back thinner. Google discontinued free Gemini CLI serving on 18 June and replaced it with Antigravity CLI on a far smaller quota. Every number in this document was checked in early September 2026 and some of it will be wrong within a quarter. **Verify against vendor pricing pages before you commit money.** The *workflow* design below is deliberately built so that swapping a provider is a one-line config change, because it will happen more than once.

---

## 1. Reality check

Three uncomfortable things, stated first, because the plan only makes sense once they are accepted.

**Your GPUs cannot run a frontier coding model, and no amount of configuration changes that.** Both cards have 8 GB of VRAM. That puts you in the 7–9B dense class at Q4, or a 30B MoE model with heavy CPU offload on the 24 GB laptop. Those models are genuinely useful for constrained, verifiable tasks. They will not architect a deterministic Rust simulation core, debug a QUIC reconciliation bug, or design a modifier stacking pipeline. Planning around "I'll just run it locally" is the most common way this kind of project stalls.

**The GTX 1080 is a bad inference card and a good server.** Pascal has no BF16, no FP8, and no modern attention kernels. It runs Q4 GGUF through llama.cpp acceptably and nothing else well. Its real value in this project is as a **self-hosted CI runner and a dedicated-server test box**, which is a role it is genuinely good at and which saves you real money and real wall-clock time.

**The expensive resource is not tokens; it is your attention.** At $20/month you have enough model capacity to outpace your own ability to review code. The failure mode is not "I ran out of quota", it is "I merged 4,000 lines I do not understand and now the golden tests fail and I cannot tell why". Every guardrail below exists to keep generated code reviewable.

### 1.1 What each budget level actually buys

| Level | Monthly | Realistic output for this project |
|---|---|---|
| $0 | $0 | 1–2 well-specified tasks per day, on whichever free models still exist this month. Slower, more manual review, more friction, and periodic disruption when a provider closes its free tier. Achievable, at roughly 1.5–2× the calendar time. |
| $20 | one subscription | 3–6 tasks per day, plus a capable architect model for design work. **This is the recommended tier and the plan is written for it.** |
| $30 | + Copilot Pro | Same, with removed friction on completions and IDE agent mode. Worth it once the student credits become annoying. |
| $100 | Max 5x | Only worth it when you are working on this near full time and hitting session limits most days. Do not start here. |
| metered | burst | For specific bulk jobs (generating 150 PF2e spell documents). Use with a hard spend cap. |

---

## 2. Where the tokens actually go

Before choosing tools, understand the shape of the work. In a project like this, LLM consumption divides into five very different categories, and they have wildly different quality requirements. **The whole cost strategy is: route each category to the cheapest model that can do it, and make the boundaries between categories explicit.**

| Category | Share of tokens | Quality needed | Verifiable by machine? |
|---|---|---|---|
| **A. Architecture & design** | ~5% | Highest | No — only by you |
| **B. Hard implementation** (netcode, rules pipeline, determinism, unsafe FFI) | ~20% | High | Partly (tests) |
| **C. Routine implementation** (a struct, a system, a CLI subcommand, a test suite from a written spec) | ~35% | Medium | **Yes** (`cargo test`) |
| **D. Bulk content** (campaign JSON, ruleset data, doc comments, fixtures) | ~30% | Low | **Yes** (`crpgc validate`) |
| **E. Mechanical** (commit messages, changelogs, renames, formatting fixes, compile-error repair) | ~10% | Low | **Yes** (compiler) |

Categories C, D, and E are 75% of the volume and all three have a machine oracle. That is the single most important fact in this plan, and it is a direct consequence of the architecture in the spec: schema-first campaign data with a validating CLI, per-crate test suites, and deterministic golden tests. **The architecture was designed to make cheap models useful. Use that.**

Category D is the one people underestimate. Phase 11 (PF2e) is hundreds of JSON documents. That is by far the largest single token expense in the project, and it is exactly the work a 7B local model can do when the schema is tight and `crpgc validate --json` gives it a repair signal. Budget zero dollars for it.

---

## 3. The four roles

Do not think in terms of "a rendering agent" and "a networking agent". Think in terms of **four roles at four price points**, any of which can be pointed at any subsystem.

### Role 1 — Architect
**Runs:** the best model you have access to, in a chat interface, usually with no repository access at all.
**Does:** ADRs, interface design, `crpg-contracts` changes, tick-order decisions, protocol design, debugging hard failures you cannot explain, reviewing a subsystem you have lost confidence in.
**Output:** an ADR, or a task file, or a short design note. **Never code directly into the repo.**
**Frequency:** a few sessions per week.
**Why no repo access:** design conversations do not need 40 files in context. Paste the two relevant `AGENTS.md` files and the failing test. This keeps the most expensive tokens on the smallest context.

### Role 2 — Implementer
**Runs:** a CLI coding agent inside the repository, on a mid-tier model.
**Does:** executes one task file, in one crate, on one branch, until the task's stated command passes.
**Output:** a branch and a PR.
**Frequency:** several times a day.
**Rule:** the Implementer never decides *what* to build. If the task file is ambiguous, it stops and asks, and you go back to the Architect.

### Role 3 — Grunt
**Runs:** a free-tier hosted model or a local model.
**Does:** category D and E work. Campaign JSON from a schema. Doc comments. Test fixtures. Commit messages. Compile-error repair loops. Mechanical refactors. Summarising a crate before an Architect session.
**Frequency:** constantly, often in batch, often unattended.

### Role 4 — Gate
**Runs:** CI. Deterministic scripts, not a model.
**Does:** everything in the integration pipeline from the spec, Section 15.4.
**Why it is a "role":** because it is the only thing standing between agent output and your codebase, and treating it as a first-class participant rather than as infrastructure changes how much you invest in it.

There is deliberately **no "reviewer agent"**. An LLM reviewing another LLM's code catches style issues and misses exactly the failures you care about (determinism violations, authority leaks, subtly wrong stacking rules). The Gate catches those mechanically. You catch the rest. Spending tokens on AI code review is the lowest-return use of your budget at this scale.

---

## 4. Model routing

The table below is the operational core of the plan. Fill in the right-hand column once, put it in `docs/agents/routing.md`, and change it only deliberately.

| Work | Role | Tier 0 ($0) | Tier 1 ($20) | Tier 3 ($100) |
|---|---|---|---|---|
| ADRs, interface design | Architect | free frontier chat tiers (rotate) | **Claude Pro (Opus-class) in chat** | Max 5x |
| Protocol, determinism, FFI, netcode | Implementer | OpenCode + best free Zen model | **Claude Code (Sonnet-class)** | Claude Code (Opus for the hard parts) |
| Routine crate implementation from a task file | Implementer | OpenCode + Zen/OpenRouter free | Claude Code, or OpenCode when quota is tight | Claude Code |
| Test suites from a written spec | Implementer | OpenCode free models | OpenCode free models (save subscription quota) | Claude Code |
| Campaign JSON, ruleset data | Grunt | **local 7B + `crpgc validate` loop** | local 7B, hosted for the tricky ones | same |
| Doc comments, commit messages, changelog | Grunt | local 7B | local 7B | local 7B |
| Compile-error repair | Grunt | local 7B, then OpenCode free | OpenCode free models | Claude Code |
| Inline completion while you type | You | Copilot (student, unlimited completions) | Copilot | Copilot |
| Integration, merge, validation | Gate | GitHub Actions (free on public repos) | same | same |

Two rules that matter more than the specific model names:

1. **The subscription is for the hard 25%, not the easy 75%.** If you find yourself using your paid quota for commit messages, the routing has failed.
2. **Every tool in the table is swappable.** OpenCode, Cline, Aider, and Kilo Code are all free, open source, and model-agnostic; they will happily point at a subscription, a free tier, or Ollama. Keep at least one of them configured as a fallback so a provider outage or a pricing change costs you an afternoon, not a week.

---

## 5. Pricing tiers in detail

### Tier 0 — $0/month

**Stack:**
- **Primary harness:** **OpenCode** (MIT, model-agnostic, ~195k stars). The harness is free and permanent; the *models* are what churn. This is the whole point of choosing it.
- **Primary free models:** **OpenCode Zen's rotating free tier.** Zen periodically hosts models at no cost while their teams collect feedback (recent examples include MiMo-V2.5 Free, Nemotron 3 Ultra Free, Ling 3.0 Flash Fin Free, and Big Pickle). Which models are free changes month to month, so check `opencode.ai/docs/zen` rather than trusting any list, including this one. Note that free models may train on your prompts. For this project that is a non-issue: the repository is public by design.
- **Secondary free models:** **OpenRouter's `:free` model variants** (roughly 50 requests/day by default, raised to about 1,000/day if you ever put $10 of lifetime credit on the account). Configure as a second provider in OpenCode so a Zen model disappearing costs you one config line.
- **Completions:** GitHub Copilot via the Student Developer Pack. The important detail: **code completions and next-edit suggestions are unlimited and do not consume credits** on any plan. The student tier's ~200 monthly AI Credits are for chat and agent mode, which you should treat as a small emergency reserve rather than a workhorse. Model selection is automatic on the student plan, so do not build a workflow that depends on picking a specific model there.
- **Local:** Ollama or llama.cpp on both machines for category D and E work (Section 6).
- **Other free tiers worth registering for, all volatile:** Rovo Dev CLI (Atlassian, generous daily token allowance in beta, Claude-backed), Jules (a small number of async tasks/day), Antigravity CLI (`agy`, Google's Gemini CLI replacement, roughly a couple of dozen requests/day). Treat each as a bonus that may vanish, never as a dependency.
- **CI:** GitHub Actions, free and unmetered on **public** repositories, plus a self-hosted runner on the desktop for the heavy jobs.
- **IDE:** VS Code, or JetBrains RustRover free through the Student Pack. RustRover's analyser is materially better than rust-analyzer in VS Code for a large workspace, and it is free for you.

**What this tier costs you:** friction, not capability. You will spend more time writing task files precisely, more time reviewing, and more time on the compile-fix loop. Expect roughly half the throughput of Tier 1.

**Honest limitation:** free hosted models are good at Rust boilerplate and noticeably weaker than a frontier model at exactly the things this project is hard at — deterministic system design, subtle ownership issues across crate boundaries, and reasoning about a protocol's failure modes. On Tier 0 you must do more of the thinking yourself.

**Honest limitation, second and more serious: the free agentic tier is structurally unstable.** In 2026 alone, Google discontinued free Gemini CLI serving (18 June), Alibaba cut Qwen Code's free OAuth quota from 1,000/day to 100/day and then closed it entirely (15 April), MiniMax changed its licensing days earlier, and GitHub paused Copilot signups and re-based the product on usage credits. Every one of those broke somebody's workflow overnight. **Do not build a process that depends on any single free provider.** The mitigation is architectural, not commercial: use a model-agnostic harness, keep at least three providers configured including a local one, and treat your task-file format — not your model — as the thing that is stable.

### Tier 1 — ~$20/month (recommended)

Tier 0, plus **one frontier subscription** used strictly for the Architect and hard-Implementer roles.

Claude Pro is $20/month ($17 annually) and includes Claude Code; usage is metered as a rolling five-hour session limit plus a weekly cap, shared with the chat apps. That sharing matters for planning: an afternoon of long design chats eats the evening's coding quota. Anthropic publishes multipliers rather than token counts, so plan by observation — run `/usage` and learn your own burn rate in the first fortnight.

The equivalent alternatives at the same price point (a ChatGPT/Codex subscription, Cursor Pro, Copilot Pro+ at $39) all work with this plan. Choose on model preference and on whether you want a terminal agent or an editor. For a Rust workspace with a lot of terminal-driven testing, a terminal agent fits better.

**Budget discipline at this tier:**
- OpenCode on a free model remains the default Implementer for routine work. The subscription is reserved.
- Do design work in the chat interface with pasted context, not in the CLI agent with repo access. Repo access on a design question can quietly read thirty files.
- Track it. `ccusage` and similar local dashboards break usage down per session at no cost.

### Tier 2 — ~$30/month

Tier 1 plus **Copilot Pro at $10**, if and only if the student tier's credit cap and forced automatic model selection start costing you time. Pro restores manual model choice and a larger credit allowance ($10 base plus flex credits under the June 2026 AI Credits billing). This is a convenience purchase, not a capability one. Defer it until you have felt the pain.

### Tier 3 — ~$100/month

Max 5x. The honest trigger for this is specific and measurable: **you are stopped mid-task by session limits on most working days**. Not "I'd go faster with more". If you are working on this ten hours a week, you will not hit that trigger. If you go full time during a summer, you might. Reassess monthly and downgrade without sentiment.

### Tier 4 — metered burst

Keep an API console account with a **hard spend cap** for one specific purpose: bulk generation jobs that are too large for a subscription's session limits and too quality-sensitive for a local model. The canonical example is Phase 11: generating and validating several hundred PF2e ability and spell documents in one weekend.

Do the arithmetic before you start. At Sonnet-class rates of roughly $2 per million input and $10 per million output tokens, a spell document at ~800 output tokens with ~4,000 input tokens of schema and examples costs about a cent. Three hundred of them is around $5, and batch discounts and cache reads (input cached at a tenth of the rate) cut it further. Bulk content is cheap; it is interactive back-and-forth that is expensive. Structure the job as a batch, not a conversation.

**Set the cap in the console before the first request, not after the first bill.** A subscription's limits are a safety feature; a metered key has none unless you add them.

---

## 6. Local models: concrete setup

### 6.1 What each machine should run

**Laptop — RTX 4060 8 GB / 24 GB RAM. Role: development machine.**

- `qwen2.5-coder:7b` or a current 7–9B coder model at Q4_K_M, ~5 GB VRAM. Fast, fits alongside your IDE, 32K context. This is your everyday Grunt.
- Optionally a 30B-class **MoE** model (the A3B architectures with ~3B active parameters) at Q4 with CPU offload across your 24 GB of system RAM. It will not be fast — expect something in the region of 10–20 tokens/second with offload — but it is meaningfully smarter than a 7B and it is fine for unattended batch work you start and walk away from.
- Do **not** try to run a dense 30B. It will swap and you will hate it.

**Desktop — GTX 1080 8 GB / 16 GB RAM. Role: server and CI.**

Its primary jobs are not inference:
1. **Windows/MSVC self-hosted GitHub Actions runner** for primary product work: Windows Godot client/editor and server builds, the primary perf gate, and expanded Windows golden replay coverage as those capabilities exist. This removes the slowest jobs from your laptop and from Actions queues; it does not replace the hosted Windows/Linux replay gates.
2. **Dedicated-server test box.** From Phase 4 onward you need a `crpg-server` running on a different machine from the client to test networking honestly. Exercise Windows and Linux dedicated adapters of the same authoritative host, with genuine environments for each; do not assume one desktop provides both. Loopback multiplayer hides an enormous number of bugs.
3. **Overnight batch inference.** A 7B Q4 model here can run generate-validate-repair loops on campaign content while you sleep, at zero marginal cost.

Set the runner to only execute jobs from branches in your own repository. On a public repo, a self-hosted runner that accepts fork pull requests will run arbitrary attacker code on your desktop. Restrict `pull_request` jobs to GitHub-hosted runners and reserve the self-hosted runner for `push` on your own branches and for manually dispatched workflows.

Runner labels must identify the actual execution target, not just hardware.
Keep Linux hosted tests/server checks on genuine Linux/GNU; add a separate
Linux runner or verified Linux environment for heavy server work when needed.
All required T009c gates passed on native Windows/MSVC and genuine Linux/GNU
in WSL Ubuntu 24.04; see the [completion record](../tasks/T009c.md).
Actual native execution, not WSL availability alone or cross-compilation,
provides generation and verification evidence.

### 6.2 The generate-validate-repair loop

This is the pattern that makes an 8 GB card genuinely productive, and it depends entirely on the CLI tooling specified in `CRPG_ENGINE_SPEC.md` §4.8.

```
for each item in the work list:
    prompt = schema (from `crpgc schema creature`)
           + 2 known-good examples
           + the one-line description of the item
    for attempt in 1..3:
        output = local_model(prompt)
        write to a scratch file
        diagnostics = `crpgc validate --json scratch/`
        if empty: move to campaign/, break
        prompt += "These diagnostics were produced. Fix them:" + diagnostics
    else:
        add to a review queue for a better model
```

Run it against `campaigns/fixtures` first to calibrate the failure rate. If a 7B model gets 70% of documents valid within three attempts, you have just moved 70% of your largest token expense to $0. If it gets 20%, tighten the schema and the examples before concluding the model is inadequate — the usual cause is a schema that permits too much.

**This loop is why the spec insists on machine-readable diagnostics with JSON pointers.** A validator that prints prose to stderr is useless here; one that emits structured, positioned errors turns a weak model into a competent content author.

### 6.3 What local models must not be used for

`crpg-contracts`. `crpg-sim` tick order. The Lua sandbox. Packet validation. Anything touching `unsafe`. Anything that re-blesses a golden test. These are listed again in Section 12 because they are the same list, and the reason is the same: a plausible-looking wrong answer is worse than no answer.

---

## 7. The task pipeline

This is the actual day-to-day workflow. It has five stages and the handoff between each is a file, so any stage can be done by any model at any tier.

```
  YOU + ARCHITECT              you             IMPLEMENTER          GATE          you
 ┌───────────────┐    ┌──────────────────┐   ┌─────────────┐   ┌─────────┐   ┌────────┐
 │ design session│ →  │ tasks/T042.md    │ → │ branch      │ → │   CI    │ → │ review │
 │ → ADR or note │    │ (the contract)   │   │ + PR        │   │ + queue │   │ + merge│
 └───────────────┘    └──────────────────┘   └─────────────┘   └─────────┘   └────────┘
      expensive            cheap (you)          mid-tier        free          your time
```

### Stage 1 — Design (Architect, expensive, rare)

Only when the answer is not already in the spec or an ADR. Bring: the relevant `AGENTS.md`, the relevant contract trait, and a precise statement of the problem. Do not bring the repository.

Output is an ADR in `docs/adr/NNNN-*.md` or a paragraph you paste into a task file. If the session produces code, that code is a sketch to be re-derived by the Implementer, not something to paste in.

### Stage 2 — Task specification (you, cheap, frequent)

The template is in `CRPG_ENGINE_SPEC.md` §15.5. Writing these well is the highest-leverage habit in the whole workflow, and it is where you should be spending your own time rather than reviewing code.

A good task file takes you 10 minutes and saves an hour of agent flailing. A bad one produces 600 lines that touch four crates and fail the dependency lint.

**Three fields do most of the work:**
- `Interface` — exact signatures. If you cannot write them, the design is not finished and you should go back to Stage 1.
- `Test` — the exact command that must pass. This is the agent's stopping condition. Without it, agents stop when they feel finished, which is the wrong time.
- `Out of scope` — the explicit list. Agents are relentlessly helpful and will refactor adjacent code, add a convenience method, and "fix" a test they misunderstand. This field is the only reliable prevention.

Keep tasks in `tasks/` in the repo, numbered, with a `tasks/BACKLOG.md` index derived from `CRPG_ENGINE_SPEC.md` §19. Committing them means every task file is diffable, greppable, and available as context for later tasks.

### Stage 3 — Implementation (Implementer)

```bash
git worktree add ../crpg-t042 -b task/T042
cd ../crpg-t042
opencode    # or claude, or whichever harness is configured
> Read AGENTS.md, crates/crpg-ai/AGENTS.md, and tasks/T042.md. Implement exactly
> that task. Do not modify any crate other than crpg-ai. Stop when
> `cargo test -p crpg-ai` passes. If the task is ambiguous, stop and tell me why.
```

**Use git worktrees, not branches on one checkout.** Worktrees give each agent session its own directory and its own `target/` build state. Two agents in one directory will overwrite each other's files and produce a mess that takes longer to untangle than doing the work yourself. This is the single most important mechanical detail in the whole workflow.

Run the local pre-flight **before** the agent finishes, not after:

```bash
# tools/preflight.sh — cheap, local, no tokens
cargo fmt --all
cargo clippy -p "$CRATE" --all-targets -- -D warnings
cargo test -p "$CRATE"
python tools/lint/deps.py
python tools/lint/determinism.py
```

Tell the agent to run this itself. Every compile error the agent fixes locally is one you did not pay a round trip for, and every clippy warning caught here is one that does not fail CI ten minutes later.

### Stage 4 — Gate (CI, free)

The pipeline from `CRPG_ENGINE_SPEC.md` §15.4, in two layers:

- **GitHub-hosted, on every push:** retain Windows/MSVC and Linux/GNU workspace tests, including their real target-scoped replay comparisons required by T009c, plus fmt, clippy, deny, architecture/determinism lints and their self-tests. Schema drift and campaign validation activate when implemented. Target: under 8 minutes with `Swatinem/rust-cache` or `sccache`.
- **Windows self-hosted, on integration and nightly:** expanded Windows golden coverage, save/load equivalence, primary perf gate, Windows client/editor/server product builds, and Windows embedded-server and dedicated-server smoke tests when implemented.
- **Linux hosted or separately provisioned Linux runner:** retain Linux headless tests and its independent server regression golden; activate real Linux headless server artifacts and dedicated-server smoke checks when implemented. Linux headless support must not pull in Godot.

These are allocation requirements, not claims that future jobs exist. E020's
capability gating forbids unavailable or skipped-green placeholders. Windows
single-player must host the same platform-neutral server as Windows/Linux
dedicated processes, behind the unchanged client/server transport boundary.
Each target must generate and verify its own golden under the pinned toolchain,
normal test profile, and default features. Select comparisons at compile time,
never by runtime OS; missing baselines fail, and neither tolerance nor comparison
to the other target is permitted. Re-baselines require review and native
provenance (commands, `rustc -vV`, results, counts, SHA-256), not copied hashes.

Splitting it this way keeps the feedback loop fast and keeps the slow jobs off the critical path.

Enable GitHub's **merge queue** even as a solo developer. When you have three worktrees with three agent branches, the queue rebases each onto main and runs the full pipeline before merging. Two branches that each pass alone and fail together is the characteristic parallel-agent failure, and the queue is the only thing that catches it automatically.

### Stage 5 — Review (you, unavoidable)

Non-negotiable review rules:

- **Read the diff of every test file.** Agents fix failing tests by weakening them. This is the most common serious failure and it is invisible in a summary. Consider a CI check that flags any PR modifying both an implementation file and its existing test assertions.
- **Read every new `unsafe`, every new dependency, and every change to a public API.** Everything else you can skim.
- **If a golden hash changed, stop and understand why.** Re-blessing goldens because CI is red is how a determinism bug ships. The replay harness tells you which tick diverged; use it.
- **If you do not understand a merged change, that is technical debt with interest.** Either get it explained until you do, or revert it. A codebase you cannot reason about cannot be debugged at 2am in year three.

---

## 8. Context discipline is cost discipline

Token spend is dominated by input, and input is dominated by how much of the repository ends up in context. The architecture already helps: small crates, explicit contracts, one-file-per-object campaign data. Reinforce it deliberately.

**Practices, in rough order of value:**

1. **One crate per task.** A task spanning three crates costs roughly three times the context and produces roughly ten times the review burden.
2. **`AGENTS.md` is the context, not the source.** Each crate's file states purpose, public API, invariants, allowed dependencies, and the test command. A well-written 60-line `AGENTS.md` replaces several thousand tokens of exploratory file reading, every single session.
3. **Keep the root instruction file small and stable.** Agent CLIs cache the system prompt and project instructions; caching bills at roughly a tenth of the input rate. Churning that file on every commit throws the cache away. Write it once, change it rarely.
4. **Give the agent tools instead of files.** `crpgc explain <id>`, `cargo tree -p <crate>`, `rg` — one command's output beats twenty file reads. This is a genuine reason to invest in the CLI early.
5. **Start a new session per task.** Long sessions accumulate irrelevant context that you pay for on every subsequent turn. When a task is done, close it.
6. **Never point an agent at `schemas/` or `rulesets/pf2e/`.** They are large, generated or bulk, and almost never what the agent needs. Add them to the agent's ignore file.
7. **Say "no" to exploration.** "Read the codebase and suggest improvements" is the most expensive prompt you can write and the least useful output you can get.

---

## 9. Parallelism: how many agents at once?

**Two, and rarely three.** Not because of quota, but because you are the reviewer and the integrator, and a queue of unreviewed PRs is worse than no PRs.

A workable pattern:

| Slot | Where | Typical work |
|---|---|---|
| Foreground | laptop, worktree A | the task you are actually thinking about |
| Background | laptop, worktree B | a mechanical task (tests from a spec, doc comments, a rename) |
| Overnight | desktop, local model | batch content generation, validated by CLI |

The third slot is the interesting one, because it costs nothing and runs while you sleep. Queue content generation and fixture authoring for it.

**Serialise anything touching:** `crpg-contracts`, `crpg-sim` tick order or the `World` struct, schema versions, the GDExtension boundary. One at a time, human-reviewed. These are the same items as §15.2 of the spec.

---

## 10. Quota-aware weekly rhythm

Different providers meter differently, and matching your working pattern to the meter is worth a surprising amount of free capacity.

- **Daily-reset quotas** (OpenRouter free models, Rovo, Jules) reward steady daily use. Unused capacity evaporates at midnight. Keep a standing queue of small tasks so a quiet day still consumes some of it.
- **Rolling five-hour windows** (Claude Pro/Max) reward planning a session, not grazing. Decide what the session is for before you open it. Chat and Claude Code share the pool, so a long design conversation costs you coding capacity later the same day.
- **Monthly credits** (Copilot AI Credits) reward saving them. Treat the student tier's allowance as an emergency reserve.

**A rhythm that fits a student's week:**

| | Work | Role | Meter |
|---|---|---|---|
| Sun evening | Plan the week. Write 5–10 task files from the backlog. | You + Architect | one design session |
| Mon–Thu | 1–2 tasks/day. Review and merge same day. | Implementer | daily free quota first, subscription for the hard one |
| Any evening | Queue overnight content generation | Grunt (local) | free |
| Fri | Integration day: merge queue, nightly suite, fix what broke, update ADRs | You | minimal |
| Sat | Play the game. Profile. Author fixtures by hand. | You | zero tokens |

That last row is not filler. Playing your own build and hand-authoring content is how you discover that the editor is unusable, and no agent will tell you that.

---

## 11. Guardrails against specific agent failure modes

| Failure | How it shows up | Guardrail |
|---|---|---|
| **Weakening tests to pass** | `assert_eq!` becomes `assert!(x.is_ok())`; a case is deleted | Read every test diff. CI flag on PRs that change existing assertions. |
| **Quota burn in a loop** | 40 turns of the same compile error | Set a max-turns limit. Require `preflight.sh` before the agent's final turn. Kill and re-scope rather than letting it grind. |
| **Scope expansion** | "I also refactored the error types" | `Out of scope` field. Dependency lint. Reject the PR rather than salvaging it. |
| **Silent architectural violation** | `crpg-rules` grows a `f64`; `crpg-sim` imports `godot` | The two custom lints in CI. They exist for exactly this. |
| **Plausible nonsense in rules code** | Stacking rule looks right, is subtly wrong | Table-driven tests written *before* the implementation, from the ruleset text, by you or the Architect. |
| **Dependency sprawl** | `Cargo.toml` grows six crates | `cargo deny` with an allow-list. Adding a dependency is its own task with its own justification. |
| **Golden re-blessing** | `insta accept` run to make CI green | Golden updates require a separate commit with a written reason, reviewed alone. |
| **Context rot** | Agent confidently references a function deleted last week | New session per task. Never resume yesterday's session. |
| **Provider outage or pricing change** | Your workflow stops | Keep a second agent CLI configured and a local endpoint working. Test the fallback once a month. |
| **You stop understanding your codebase** | You cannot debug your own project | The review rules in §7.5. This is the one that actually kills projects. |

---

## 12. Never delegated

Repeating the spec's list, because it is also the cost plan's list:

- `crpg-contracts` — every trait in it.
- `crpg-sim` tick order and the `World` struct definition.
- Schema versions and migrations.
- The Lua sandbox deny-list and budget enforcement.
- Client intent validation and per-client component filtering.
- Anything `unsafe`, which in practice means `crpg-godot`.
- Re-blessing golden test outputs.
- The decision that a task is finished.

An agent may *draft* any of these. You decide, and you read every line.

---

## 13. Measurement

Cheap, local, and worth ten minutes to set up:

- **Per-session token and cost tracking.** `ccusage` and equivalents read the agent's local logs and break usage down by session and project. No account, no telemetry, no cost.
- **Tasks merged per week.** The number that actually matters. Track it in `tasks/BACKLOG.md`.
- **Cost per merged task.** Divide. If it climbs, your task files are getting vaguer, not your models worse. That is almost always the cause.
- **CI wall-clock.** When the GitHub-hosted job passes 10 minutes, move something to the self-hosted runner or improve caching. Slow CI makes you skip it, and skipped CI is the whole guardrail system gone.
- **Patch-queue size** (`third_party/godot/patches/*.patch`, total lines). From the spec: a growing number means the fork decision is drifting and needs an ADR.

Review these monthly, alongside the question "should I move up or down a tier?"

---

## 14. Model allocation by project phase

Mapping the routing table onto the roadmap, so the budget matches the work.

| Phase | Dominant category | Where the money goes | Tier 0 viability |
|---|---|---|---|
| 0 — Spikes | A, B | Architect-heavy. Three go/no-go decisions worth getting right. | Hard. If you buy one month of subscription, buy it here. |
| 1 — Core + harness | B, C | Determinism and the world store need care. Tests are C. | Workable with careful task files. |
| 2 — Campaign format | C, D | Mostly schema types and migrations. Highly verifiable. | **Good.** Free tiers handle most of it. |
| 3 — Rules kernel | A, B | The highest-risk design in the project. | Weak. Spend here if you spend anywhere. |
| 4 — Server + net | B | Protocol design and failure modes. Frontier-model work. | Weak. |
| 5 — Client | C | Godot glue, UI, scene sync. Verifiable by running it. | Good. |
| 6 — Editor v1 | C, D | Generated forms mean lots of routine code. | **Very good.** |
| 8 — Graphs, quests | C | IR interpreter is the only hard part. | Good. |
| 9 — AI | B, C | Utility scoring design is B; the rest is C. | Mixed. |
| 10 — Multiplayer hardening | B | Debugging distributed failures. | Weak. |
| 11 — PF2e | **D** | Enormous volume, trivially verifiable. | **Excellent.** Local models and a metered batch job. |
| 12 — Modding, packaging | C | Routine. | Good. |

The pattern: **the cheap tiers are strongest exactly where the volume is, and weakest exactly where the risk is.** If your budget is intermittent, subscribe during Phases 0, 3, 4, and 10, and run free during 2, 5, 6, and 11. That is a legitimate strategy and it may halve your total spend across the project.

---

## 15. Setup checklist

Roughly one day of work. Do it before writing any project code.

**Accounts and access**
1. Verify GitHub Education at `education.github.com`. Confirm what the student Copilot tier currently includes before planning around it.
2. Create the repository **public** from day one. Unlimited Actions minutes, and the spec's licence recommendation is permissive anyway.
3. Install OpenCode (`npm install -g opencode-ai`). Register at `opencode.ai/auth` for Zen and at `openrouter.ai` for a free key. Configure both as providers, plus your local Ollama endpoint as a third. Confirm which Zen models are currently free.
4. If buying a subscription, buy one and install its CLI. Run its usage command and learn what a session costs you.
5. Install one model-agnostic fallback agent (OpenCode, Aider, Cline, or Kilo Code) and point it at your local endpoint. Confirm it works before you need it.

**Local inference**
6. Install Ollama or llama.cpp on the laptop. Pull a current 7–9B coder model at Q4. Measure tokens/second.
7. Do the same on the desktop. Confirm the Pascal card's throughput is acceptable for batch work.
8. Write `tools/gen_loop.py` implementing §6.2 against a stub schema. Prove the loop before you need it in Phase 11.

**Repository scaffolding**
9. Cargo workspace with every crate as a stub (spec task T4).
10. Root `AGENTS.md` and one per crate. Keep them short.
11. `tasks/` with the template and `BACKLOG.md` seeded from spec §19.1.
12. `docs/adr/0000-template.md` and your first real ADR.
13. `tools/preflight.sh`.
14. The dependency-direction and determinism lints (spec task T5). Build them before the code they police.

**CI**
15. GitHub Actions workflow with the fast layer, plus Rust caching.
16. Windows/MSVC self-hosted runner on the desktop, restricted to your own branches, for primary product/golden work; retain Linux hosted coverage and provision genuine Linux execution separately for heavy headless/server gates as needed.
17. Enable the merge queue.

**Working practice**
18. Create three git worktrees. Get used to them immediately.
19. Install a local usage dashboard.
20. Put the routing table in `docs/agents/routing.md` and note the date you last verified prices.

---

## 16. The plan in one paragraph

Buy one $20 subscription and use it only for architecture and the hard 25% of implementation. Run OpenCode against whatever free models exist this month as your everyday implementer, with at least three providers configured so no single shutdown stops you. Run a 7B local model on both machines for campaign content, doc comments, and compile repair, verified by `crpgc validate` and the compiler rather than by judgement. Use Copilot's unlimited completions while you type and hoard its credits. Make the repository public so CI is free, and put the GTX 1080 desktop to work as a self-hosted runner and a real remote server for network testing rather than as an inference box. Move work between tiers by writing precise task files, because the task file is the interface that lets a cheap model do expensive-looking work. Review every test diff, never re-bless a golden without understanding it, and keep two agents running at most, because your reviewing capacity is the real bottleneck. Re-verify every price in this document quarterly, because in this market they will have changed.

Assign primary product and behavioural-baseline work to a Windows/MSVC runner;
retain genuine Linux/GNU hosted tests and server checks against its independent
baseline. This is exact-build replay verification, not cross-platform lockstep.
T009a, T009c, and T009b are merged on `master` as of 2026-09-07 (T009a/T009c
in commit `bb9a702`); the next task is T010 campaign data.

## Agent log

- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c documentation alignment · Assigned primary product/golden work to Windows while retaining genuine Linux headless/server gates and independent replay provenance. Kept future CI capability-gated and distinguished available native Windows/WSL Ubuntu environments from verified T009c results.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c verification status · Linked the completion record for the reported passing native Windows/MSVC and genuine WSL Ubuntu Linux/GNU gates while preserving runner and provenance requirements. T009c remains the priority awaiting review/merge, T009a is also uncommitted, and final audit is still running.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c final audit · Recorded the reported final audit completion and completed working-tree implementation/verification while preserving runner and provenance requirements. Review/merge remains outstanding, T009a is also uncommitted, and T009c stays ahead of T009b.
- 2026-09-07 (UTC) · opencode/big-pickle + T009a/T009c merged · Updated the runner paragraph to the merged state; T009b is next.
- 2026-09-07 (UTC) · opencode/big-pickle + T009b merged · Updated the runner paragraph to the merged `crpgc replay` wrapper; T010 campaign data is next.
